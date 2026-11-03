//! Choosing and keeping the audio output: automatic output, a lost Bluetooth peer. Codec choices are
//! the radio daemon's (it applies the owner's choice when a headset connects or its stream stops).
use crate::*;

/// Bluetooth headphones win over the headphone jack. The first time a
/// connected peer's playback PCM is ready, audio moves to it (playback
/// continues there). Once per connection: choosing the jack afterwards sticks
/// until the peer reconnects, and a codec change that recreates the PCM is not
/// a new connection. A working Bluetooth output is never displaced. Losing the
/// peer returns to the jack and pauses (`Event::BluetoothDisconnected`).
#[derive(Default)]
pub(crate) struct AutoOutput {
    pub(crate) routed: HashSet<String>,
}

impl AutoOutput {
    pub(crate) fn next(
        &mut self,
        status: &bluetooth::Status,
        output: &AudioOutput,
    ) -> Option<String> {
        if !status.available {
            return None; // A failed observation is not a disconnection.
        }
        self.routed.retain(|address| {
            status.powered
                && status
                    .devices
                    .iter()
                    .any(|d| d.connected && d.address.eq_ignore_ascii_case(address))
        });
        if !status.powered {
            return None;
        }
        let ready = |address: &str| status.playback_pcm(address).is_ok();
        let peer = status.devices.iter().find(|d| {
            d.connected && d.audio && !self.routed.contains(&d.address) && ready(&d.address)
        })?;
        match output {
            AudioOutput::Bluetooth(current) if current.eq_ignore_ascii_case(&peer.address) => {
                // Already there (restored or chosen by hand): this connection is done.
                self.routed.insert(peer.address.clone());
                None
            }
            AudioOutput::Bluetooth(current) if ready(current) => None,
            _ => Some(peer.address.clone()),
        }
    }
    pub(crate) fn routed(&mut self, address: String) {
        self.routed.insert(address);
    }
}

/// Bluetooth headphones that were the output until they dropped (F51).
pub(crate) struct LostPeer {
    pub(crate) address: String,
    /// The loss, or the wake after it.
    pub(crate) since: Instant,
    /// A Play is waiting for them since then.
    pub(crate) play: Option<Instant>,
}

impl LostPeer {
    pub(crate) fn new(address: String, since: Instant) -> Self {
        Self {
            address,
            since,
            play: None,
        }
    }
    pub(crate) fn is(&self, output: &AudioOutput) -> bool {
        matches!(output, AudioOutput::Bluetooth(a) if a.eq_ignore_ascii_case(&self.address))
    }
    /// Play waits: the jack took over from them a short while ago and holds
    /// no headphones (a plug there is a choice of its own).
    pub(crate) fn applies(
        &self,
        output: &AudioOutput,
        headphones: Option<bool>,
        now: Instant,
    ) -> bool {
        matches!(output, AudioOutput::Wired)
            && headphones != Some(true)
            && !self.gave_up(now)
            && (self.play.is_some() || !self.expired(now))
    }
    pub(crate) fn gave_up(&self, now: Instant) -> bool {
        self.play
            .is_some_and(|at| now.saturating_duration_since(at) >= LOST_PEER_PLAY_WAIT)
    }
    pub(crate) fn expired(&self, now: Instant) -> bool {
        self.play.is_none() && now.saturating_duration_since(self.since) >= LOST_PEER_WINDOW
    }
}

/// A start failed because the selected Bluetooth peer is not connected, and
/// the last observation agrees: the disconnect was missed, not just late.
pub(crate) fn selected_peer_gone(
    output: &AudioOutput,
    state: &bluetooth::Status,
    error: &str,
) -> bool {
    let AudioOutput::Bluetooth(address) = output else {
        return false;
    };
    error.contains("A2DP peer required")
        && state.available
        && !state
            .devices
            .iter()
            .any(|d| d.address.eq_ignore_ascii_case(address) && d.connected)
}
