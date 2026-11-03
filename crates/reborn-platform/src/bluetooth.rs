//! Bluetooth as the platform's radio daemon (`y2-radiod`) runs it. The daemon alone talks to BlueZ and
//! BlueALSA: it powers the adapter, connects, reconnects, pairs, picks codecs and recovers. This module
//! sends it what the owner wants ([`Command`] → `y2p_contract::radiod::Request`) and turns every state
//! it publishes into one [`Status`]: the player's only copy of what Bluetooth is doing.
#![forbid(unsafe_code)]
use reborn_core::{platform::Fact, BluetoothPcm, CodecPreference, RadioScan};
use reborn_observability::{HealthState, Level, Observer};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::PathBuf,
    sync::{
        mpsc::{sync_channel, Receiver, SendError, SyncSender, TryRecvError, TrySendError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use y2p_contract::radiod::{self, Link, Message, Request};

use crate::radiod::{ask, send, SOCKET};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Device {
    pub path: String,
    pub name: String,
    pub address: String,
    pub paired: bool,
    #[serde(default)]
    pub bonded: bool,
    #[serde(default)]
    pub trusted: bool,
    pub connected: bool,
    pub audio: bool,
    /// The headphones' own volume (AVRCP absolute volume, 0..=127), only
    /// when they report one: BlueZ's A2DP transport `Volume` (Y2Reborn #22).
    #[serde(default)]
    pub volume: Option<u8>,
    /// That transport's object, where a new volume is set.
    #[serde(default)]
    pub transport: Option<String>,
    /// Connected, but the daemon is still setting up its stream (codec, A2DP): not playable yet.
    #[serde(default)]
    pub settling: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Status {
    pub available: bool,
    pub powered: bool,
    pub discovering: bool,
    pub bluealsa: bool,
    pub pcms: Vec<BluetoothPcm>,
    #[serde(default)]
    pub codec_policy: Option<crate::codecs::Session>,
    pub devices: Vec<Device>,
    pub pending: Option<Pairing>,
    /// The daemon's last failure, in the owner's words (cleared by the next request).
    pub error: Option<String>,
    #[serde(default)]
    pub scan: RadioScan,
    /// A device the daemon is connecting (device path).
    #[serde(default)]
    pub waiting: Option<String>,
    /// The daemon's service state ("ready", "starting", "recovering", "failed", "quiesced").
    #[serde(default)]
    pub service: String,
}

impl Status {
    pub fn playback_pcm(&self, address: &str) -> Result<BluetoothPcm, String> {
        let device = self
            .devices
            .iter()
            .find(|d| d.address.eq_ignore_ascii_case(address) && d.connected && d.audio)
            .ok_or("connected A2DP peer required")?;
        if !self.bluealsa {
            return Err("BlueALSA unavailable".into());
        }
        if device.settling {
            return Err("Bluetooth headphones are still connecting".into());
        }
        self.pcms
            .iter()
            .find(|pcm| pcm.is_a2dp_playback_for(&device.path))
            .cloned()
            .ok_or("BlueALSA playback PCM not ready; retry after connection completes".into())
    }

    /// The codec actually negotiated for `path`'s playback transport.
    pub fn active_codec(&self, path: &str) -> Option<String> {
        self.pcms
            .iter()
            .find(|pcm| pcm.is_a2dp_playback_for(path))
            .and_then(|pcm| pcm.codec.clone())
            .filter(|c| !c.is_empty())
    }

    /// Codec preferences worth offering for the connected peer: only codecs
    /// the current policy enables *and* the peer can use. A single usable
    /// codec means there is nothing to choose and the row is hidden.
    pub fn codec_choices(&self) -> Vec<CodecPreference> {
        let Some(policy) = &self.codec_policy else {
            return vec![];
        };
        let usable = |name: &str| {
            policy.eligibility.get(name).is_some_and(|e| {
                e.runtime_enabled
                    && e.mutually_usable
                    && (e.auto_eligible || e.experimental_eligible || name == "SBC")
            })
        };
        let mut choices: Vec<_> = [
            ("SBC", CodecPreference::Sbc),
            ("AAC", CodecPreference::Aac),
            ("aptX", CodecPreference::Aptx),
            ("aptX-HD", CodecPreference::AptxHd),
            ("LDAC", CodecPreference::Ldac),
        ]
        .into_iter()
        .filter(|(name, _)| usable(name))
        .map(|(_, preference)| preference)
        .collect();
        if crate::codecs::experimental_xq_enabled() && usable("SBC") {
            choices.insert(1, CodecPreference::SbcXq);
        }
        if choices.len() < 2 {
            return vec![];
        }
        if policy
            .eligibility
            .values()
            .any(|e| e.auto_eligible || e.experimental_eligible)
        {
            choices.insert(0, CodecPreference::Auto);
        }
        choices
    }

    /// Transport observations for Diagnostics → Bluetooth.
    pub fn diagnostic_facts(&self) -> Vec<Fact> {
        let yes = |b: bool| if b { "Yes" } else { "No" };
        let mut facts = vec![
            Fact::new("Adapter", yes(self.available)),
            Fact::new("Powered", yes(self.powered)),
            Fact::new("Audio service", yes(self.bluealsa)),
            Fact::new("Radio daemon", self.service.clone()),
        ];
        for d in self.devices.iter().filter(|d| d.paired || d.connected) {
            facts.push(Fact::new(
                d.name.clone(),
                format!(
                    "paired {} · bonded {} · trusted {} · connected {}",
                    yes(d.paired),
                    yes(d.bonded),
                    yes(d.trusted),
                    yes(d.connected)
                ),
            ));
        }
        for pcm in &self.pcms {
            facts.push(Fact::new(
                "Active codec",
                pcm.codec.clone().unwrap_or_else(|| "Not reported".into()),
            ));
            facts.push(Fact::new(
                "PCM",
                format!(
                    "{} · {} Hz · {} ch · {}",
                    pcm.negotiated_format()
                        .map(|f| f.as_str().to_owned())
                        .unwrap_or_else(|e| e),
                    pcm.rate
                        .map(|r| r.to_string())
                        .unwrap_or_else(|| "?".into()),
                    pcm.channels
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "?".into()),
                    if pcm.running == Some(true) {
                        "running"
                    } else {
                        "idle"
                    }
                ),
            ));
            facts.push(Fact::new(
                "Transport generation",
                pcm.transport_generation.to_string(),
            ));
        }
        if let Some(policy) = &self.codec_policy {
            facts.push(Fact::new("Codec preference", policy.preference.label()));
            facts.push(Fact::new("Codec negotiation", policy.state.clone()));
            if let Some(reason) = &policy.reason {
                facts.push(Fact::new("Codec reason", reason.clone()));
            }
        }
        facts
    }

    pub fn playback_rate(&self, address: &str) -> Result<u32, String> {
        let pcm = self.playback_pcm(address)?;
        if pcm.channels != Some(2) {
            return Err("Bluetooth PCM must negotiate stereo for Baseline 01".into());
        }
        pcm.negotiated_rate()
    }

    fn address_of(&self, path: &str) -> Option<String> {
        self.devices
            .iter()
            .find(|d| d.path == path)
            .map(|d| d.address.clone())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Pairing {
    pub device: String,
    pub display: String,
}

#[derive(Clone, Debug)]
pub enum Command {
    Power(bool),
    Scan(bool),
    Pair(String),
    Connect(String),
    Disconnect(String),
    Forget(String),
    Confirm(bool),
    Refresh,
    Codec {
        address: String,
        preference: reborn_core::CodecPreference,
    },
    /// The headphones' own volume (0..=127) for the device at this path.
    Volume {
        device: String,
        volume: u8,
    },
    Stop,
}

/// The bounded command queue, waking the client thread at once.
#[derive(Clone)]
pub struct CommandSender {
    queue: SyncSender<Command>,
    wake: Arc<UnixStream>,
}

impl CommandSender {
    pub fn send(&self, command: Command) -> Result<(), SendError<Command>> {
        self.queue.send(command)?;
        self.notify();
        Ok(())
    }
    pub fn try_send(&self, command: Command) -> Result<(), TrySendError<Command>> {
        self.queue.try_send(command)?;
        self.notify();
        Ok(())
    }
    fn notify(&self) {
        // At most sixteen queued commands; a nonblocking byte never delays UI.
        let _ = (&*self.wake).write(&[1]);
    }
}

pub struct Bluetooth {
    pub commands: CommandSender,
    pub events: Receiver<Status>,
}

impl Bluetooth {
    pub fn spawn(log: Observer) -> std::io::Result<Self> {
        Self::spawn_inner(log, None)
    }

    /// As `spawn`, and every published status writes a byte to `wake`: the UI waits seconds while
    /// the screen is dark, and a headset that connected must be known before its own Play button.
    pub fn spawn_notified(log: Observer, wake: UnixStream) -> std::io::Result<Self> {
        wake.set_nonblocking(true)?;
        Self::spawn_inner(log, Some(Arc::new(wake)))
    }

    fn spawn_inner(log: Observer, wake: Option<Arc<UnixStream>>) -> std::io::Result<Self> {
        let (tx, rx) = sync_channel(16);
        let (wake_sender, wake_receiver) = UnixStream::pair()?;
        wake_sender.set_nonblocking(true)?;
        wake_receiver.set_nonblocking(true)?;
        let (states, events) = sync_channel(4);
        let client = Client {
            log,
            socket: SOCKET.into(),
            commands: rx,
            wake_receiver,
            states,
            ui_wake: wake,
            last: Status::default(),
            waiting: Vec::new(),
            unsent: None,
        };
        thread::Builder::new()
            .name("bluetooth".into())
            .spawn(move || client.run())?;
        Ok(Self {
            commands: CommandSender {
                queue: tx,
                wake: Arc::new(wake_sender),
            },
            events,
        })
    }
}

/// The thread between the player and the daemon.
struct Client {
    log: Observer,
    socket: PathBuf,
    commands: Receiver<Command>,
    wake_receiver: UnixStream,
    states: SyncSender<Status>,
    ui_wake: Option<Arc<UnixStream>>,
    /// The last state, to find a device's address by its path.
    last: Status,
    /// Requests made while the daemon was away (sent when it is back; each is a wish, not a step).
    waiting: Vec<Request>,
    /// A state the main loop's queue had no room for (the newest replaces it).
    unsent: Option<Status>,
}

impl Client {
    fn run(mut self) {
        let mut announced = false;
        loop {
            match self.session(&mut announced) {
                Session::Stop => return,
                Session::Lost => {
                    if announced {
                        self.log.emit(
                            Level::Warn,
                            "bluetooth",
                            "radio_daemon_lost",
                            "The radio daemon went away; reconnecting",
                            None,
                            json!({}),
                        );
                        announced = false;
                    }
                    self.log.health_set(
                        "bluetooth",
                        HealthState::Unavailable,
                        false,
                        "radio daemon unavailable",
                    );
                    if self.idle(Duration::from_secs(1)) {
                        return;
                    }
                }
            }
        }
    }

    /// One connection to the daemon: subscribe, then pass states up and requests down.
    fn session(&mut self, announced: &mut bool) -> Session {
        let Ok(stream) = UnixStream::connect(&self.socket) else {
            return Session::Lost;
        };
        let Ok(reader) = stream.try_clone() else {
            return Session::Lost;
        };
        let _ = reader.set_nonblocking(true);
        let mut writer = stream;
        let _ = writer.set_write_timeout(Some(Duration::from_secs(1)));
        if send(&mut writer, &Request::Subscribe).is_err() {
            return Session::Lost;
        }
        for request in std::mem::take(&mut self.waiting) {
            let _ = send(&mut writer, &request);
        }
        if !*announced {
            self.log.emit(
                Level::Info,
                "bluetooth",
                "radio_daemon_connected",
                "Connected to the radio daemon",
                None,
                json!({}),
            );
            *announced = true;
        }
        let mut lines = BufReader::new(reader);
        loop {
            if self.read(&mut lines).is_err() {
                return Session::Lost;
            }
            self.flush_state();
            match self.drain_commands(&mut writer) {
                Drained::Stop => return Session::Stop,
                Drained::Lost => return Session::Lost,
                Drained::Done => {}
            }
            let wait = if self.unsent.is_some() {
                Duration::from_millis(50)
            } else {
                Duration::from_secs(60)
            };
            let fd = lines.get_ref().as_raw_fd();
            let wake = self.wake_receiver.as_raw_fd();
            let _ = crate::native::bluetooth_wait(fd, true, false, wake, wait);
            while (&self.wake_receiver)
                .read(&mut [0u8; 64])
                .is_ok_and(|n| n > 0)
            {}
        }
    }

    /// Every complete line the daemon sent.
    fn read(&mut self, lines: &mut BufReader<UnixStream>) -> Result<(), ()> {
        loop {
            let mut line = String::new();
            match lines.read_line(&mut line) {
                Ok(0) => return Err(()),
                Ok(_) if !line.ends_with('\n') => return Err(()),
                Ok(_) => self.message(&line),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(_) => return Err(()),
            }
        }
    }

    fn message(&mut self, line: &str) {
        match serde_json::from_str::<Message>(line) {
            Ok(Message::State(state)) => {
                let status = status_from(&state.bluetooth);
                let health = match state.bluetooth.service {
                    radiod::Service::Failed => HealthState::Failed,
                    radiod::Service::Recovering | radiod::Service::Starting => {
                        HealthState::Degraded
                    }
                    _ => HealthState::Ok,
                };
                self.log
                    .health_set("bluetooth", health, false, &status.service);
                self.last = status.clone();
                self.unsent = Some(status);
            }
            Ok(Message::Reply(reply)) if !reply.ok => self.log.emit(
                Level::Warn,
                "bluetooth",
                "request_refused",
                reply.reason.as_deref().unwrap_or("refused"),
                None,
                json!({}),
            ),
            Ok(Message::Reply(_)) => {}
            Err(_) => self.log.emit(
                Level::Warn,
                "bluetooth",
                "radio_daemon_message_unreadable",
                "A message from the radio daemon could not be read",
                None,
                json!({}),
            ),
        }
    }

    /// Hand the newest state to the main loop (kept for the next pass when its queue is full).
    fn flush_state(&mut self) {
        let Some(status) = self.unsent.take() else {
            return;
        };
        match self.states.try_send(status) {
            Ok(()) => {
                if let Some(wake) = &self.ui_wake {
                    let _ = (&**wake).write(&[1]);
                }
            }
            Err(TrySendError::Full(status)) => self.unsent = Some(status),
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    fn drain_commands(&mut self, writer: &mut UnixStream) -> Drained {
        loop {
            let command = match self.commands.try_recv() {
                Ok(command) => command,
                Err(TryRecvError::Empty) => return Drained::Done,
                Err(TryRecvError::Disconnected) => return Drained::Stop,
            };
            if matches!(command, Command::Stop) {
                return Drained::Stop;
            }
            let Some(request) = self.request(command) else {
                continue;
            };
            if send(writer, &request).is_err() {
                self.waiting.push(request);
                return Drained::Lost;
            }
        }
    }

    /// Wait without a daemon: commands are kept for the next connection. True when told to stop.
    fn idle(&mut self, wait: Duration) -> bool {
        let until = Instant::now() + wait;
        while Instant::now() < until {
            let left = until.saturating_duration_since(Instant::now());
            match self.commands.recv_timeout(left) {
                Ok(Command::Stop) => return true,
                Ok(command) => {
                    if let Some(request) = self.request(command) {
                        if self.waiting.len() < 16 {
                            self.waiting.push(request);
                        }
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return true,
            }
        }
        false
    }

    /// The daemon's request for a player command (devices by address, not BlueZ path).
    fn request(&self, command: Command) -> Option<Request> {
        let address = |path: &str| self.last.address_of(path);
        Some(match command {
            Command::Power(on) => Request::BluetoothPower { on, persist: true },
            Command::Scan(on) => Request::Scan { on },
            Command::Pair(path) => Request::Pair {
                address: address(&path)?,
            },
            Command::Connect(path) => Request::Connect {
                address: address(&path)?,
            },
            Command::Disconnect(path) => Request::Disconnect {
                address: address(&path)?,
            },
            Command::Forget(path) => Request::Forget {
                address: address(&path)?,
            },
            Command::Confirm(accept) => Request::Confirm { accept },
            Command::Codec { preference, .. } => Request::CodecPreference {
                preference: preference.label().into(),
            },
            Command::Volume { device, volume } => Request::Volume {
                address: address(&device)?,
                volume,
            },
            Command::Refresh | Command::Stop => return None,
        })
    }
}

enum Session {
    Lost,
    Stop,
}

enum Drained {
    Done,
    Lost,
    Stop,
}

/// The daemon's Bluetooth state as the player's [`Status`].
fn status_from(state: &radiod::Bluetooth) -> Status {
    let devices: Vec<Device> = state
        .devices
        .iter()
        .map(|d| Device {
            path: d.path.clone(),
            name: d.name.clone(),
            address: d.address.clone(),
            paired: d.paired,
            bonded: d.bonded,
            trusted: d.trusted,
            connected: d.connected,
            audio: d.audio,
            volume: d.volume,
            transport: d.transport.clone(),
            settling: d.connected && d.audio && d.link != Link::Ready,
        })
        .collect();
    let waiting = state
        .devices
        .iter()
        .find(|d| {
            matches!(d.link, Link::Connecting | Link::Pairing)
                || (d.link == Link::Settling && d.audio)
                || (d.link == Link::Waiting && !d.connected)
        })
        .map(|d| d.path.clone());
    Status {
        available: state.available,
        powered: state.powered,
        discovering: state.discovering,
        bluealsa: state.bluealsa,
        pcms: state.pcms.iter().map(pcm_from).collect(),
        codec_policy: state
            .codec
            .as_ref()
            .map(crate::codecs::Session::from_daemon),
        devices,
        pending: state.pairing.as_ref().map(|p| Pairing {
            device: p.device.clone(),
            display: p.display.clone(),
        }),
        error: state.failure.as_ref().map(|f| f.message.clone()),
        scan: match &state.scan {
            radiod::Scan::Idle => RadioScan::Idle,
            radiod::Scan::Scanning => RadioScan::Scanning,
            radiod::Scan::Complete { found } => RadioScan::Complete { found: *found },
            radiod::Scan::Failed { message } => RadioScan::Failed {
                message: message.clone(),
            },
        },
        waiting,
        service: format!("{:?}", state.service).to_ascii_lowercase(),
    }
}

fn pcm_from(pcm: &radiod::Pcm) -> BluetoothPcm {
    BluetoothPcm {
        object: pcm.object.clone(),
        device: pcm.device.clone(),
        transport: pcm.transport.clone(),
        mode: pcm.mode.clone(),
        codec: pcm.codec.clone(),
        format: pcm.format,
        rate: pcm.rate,
        channels: pcm.channels,
        running: pcm.running,
        transport_generation: pcm.generation,
    }
}

/// The daemon never changes a codec while the player's stream runs (it reads BlueALSA's `Running`),
/// so no lease is held any more; kept so the sink's open stays as it was.
pub fn playback_pcm_lease() -> Result<Option<std::fs::File>, String> {
    Ok(None)
}

fn current() -> Result<radiod::Bluetooth, String> {
    match ask(&Request::State)? {
        Message::State(state) => Ok(state.bluetooth),
        Message::Reply(reply) => Err(reply.reason.unwrap_or_else(|| "refused".into())),
    }
}

pub fn service_test() -> Result<serde_json::Value, String> {
    let s = current()?;
    Ok(
        json!({"passed":s.available,"adapter":s.available,"bluealsa":s.bluealsa,
        "connected":s.devices.iter().filter(|d|d.connected).count(),
        "service":format!("{:?}", s.service).to_ascii_lowercase()}),
    )
}

pub fn scan_test(seconds: u64) -> Result<serde_json::Value, String> {
    if !(1..=15).contains(&seconds) {
        return Err("scan duration 1..15 seconds".into());
    }
    ask(&Request::Scan { on: true })?;
    thread::sleep(Duration::from_secs(seconds));
    let state = current();
    let _ = ask(&Request::Scan { on: false });
    Ok(json!({"passed":true,"count":state?.devices.len()}))
}

#[cfg(test)]
#[path = "bluetooth_tests.rs"]
mod tests;
