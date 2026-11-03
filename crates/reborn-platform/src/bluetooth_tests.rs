//! The player's view of the radio daemon: playable only when ready, one status from each state, and
//! requests that name devices by address.
use super::*;

fn connected() -> Status {
    Status {
        bluealsa: true,
        devices: vec![Device {
            path: "/org/bluez/hci0/dev_12_34_56_78_90_AB".into(),
            address: "12:34:56:78:90:AB".into(),
            connected: true,
            audio: true,
            ..Default::default()
        }],
        pcms: vec![BluetoothPcm {
            device: "/org/bluez/hci0/dev_12_34_56_78_90_AB".into(),
            transport: "A2DP-source".into(),
            mode: "sink".into(),
            format: Some(0x8210),
            rate: Some(44100),
            channels: Some(2),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn selects_negotiated_rate_for_exact_peer_and_direction() {
    let mut s = connected();
    assert_eq!(s.playback_rate("12:34:56:78:90:ab").unwrap(), 44100);
    s.pcms[0].rate = Some(48000);
    assert_eq!(s.playback_rate("12:34:56:78:90:AB").unwrap(), 48000);
    s.pcms[0].device = "/org/bluez/hci0/dev_other".into();
    assert!(s.playback_rate("12:34:56:78:90:AB").is_err());
    s = connected();
    s.pcms[0].mode = "source".into();
    assert!(s.playback_rate("12:34:56:78:90:AB").is_err());
    s = connected();
    s.pcms[0].transport = "HFP-AG".into();
    assert!(s.playback_rate("12:34:56:78:90:AB").is_err());
}

#[test]
fn unavailable_settling_and_unsupported_pcm_never_play() {
    for (rate, channels) in [(Some(96000), Some(2)), (Some(44100), Some(1))] {
        let mut s = connected();
        s.pcms[0].rate = rate;
        s.pcms[0].channels = channels;
        assert!(s.playback_rate("12:34:56:78:90:AB").is_err());
    }
    let mut s = connected();
    s.pcms.clear();
    assert!(s.playback_rate("12:34:56:78:90:AB").is_err());
    s = connected();
    s.devices[0].connected = false;
    assert!(s.playback_rate("12:34:56:78:90:AB").is_err());
    s = connected();
    s.devices[0].settling = true;
    assert_eq!(
        s.playback_pcm("12:34:56:78:90:AB").err().as_deref(),
        Some("Bluetooth headphones are still connecting")
    );
}

#[test]
fn bluealsa_pcm_format_is_typed_and_unknown_is_not_sbc() {
    let mut s = connected();
    assert_eq!(
        s.pcms[0].negotiated_format().unwrap(),
        reborn_core::PcmFormat::S16LE
    );
    s.pcms[0].format = Some(0x8418);
    assert_eq!(
        s.pcms[0].negotiated_format().unwrap(),
        reborn_core::PcmFormat::S24LE
    );
    s.pcms[0].format = None;
    assert!(s.pcms[0].negotiated_format().is_err());
}

fn observer() -> Observer {
    let dir = std::env::temp_dir().join(format!(
        "reborn-bt-client-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    Observer::new(&dir).unwrap()
}

fn daemon_state() -> radiod::Bluetooth {
    radiod::Bluetooth {
        service: radiod::Service::Ready,
        available: true,
        powered: true,
        bluealsa: true,
        devices: vec![radiod::Device {
            path: "/org/bluez/hci0/dev_12_34_56_78_90_AB".into(),
            address: "12:34:56:78:90:AB".into(),
            name: "Buds".into(),
            paired: true,
            bonded: true,
            trusted: true,
            connected: true,
            audio: true,
            link: Link::Settling,
            auto_connect: true,
            volume: Some(64),
            transport: Some("/org/bluez/hci0/dev_12_34_56_78_90_AB/fd0".into()),
            ended: None,
        }],
        pcms: vec![radiod::Pcm {
            object: "/org/bluealsa/hci0/dev_12_34_56_78_90_AB/a2dpsrc/sink".into(),
            device: "/org/bluez/hci0/dev_12_34_56_78_90_AB".into(),
            transport: "A2DP-source".into(),
            mode: "sink".into(),
            codec: Some("AAC".into()),
            format: Some(0x8210),
            rate: Some(44_100),
            channels: Some(2),
            running: Some(false),
            generation: 7,
        }],
        failure: Some(radiod::Failure {
            reason: "connect_failed".into(),
            message: "Couldn't connect to Buds. Make sure they're on and nearby.".into(),
            address: None,
            sequence: 3,
        }),
        ..Default::default()
    }
}

#[test]
fn the_daemons_state_becomes_one_status_and_settling_headphones_wait() {
    let mut state = daemon_state();
    let status = status_from(&state);
    assert!(status.devices[0].settling);
    assert_eq!(
        status.waiting.as_deref(),
        Some(state.devices[0].path.as_str())
    );
    assert!(status.playback_pcm("12:34:56:78:90:AB").is_err());
    assert_eq!(status.pcms[0].transport_generation, 7);
    assert_eq!(status.devices[0].volume, Some(64));
    assert_eq!(
        status.error.as_deref(),
        Some("Couldn't connect to Buds. Make sure they're on and nearby.")
    );
    state.devices[0].link = Link::Ready;
    let ready = status_from(&state);
    assert!(!ready.devices[0].settling && ready.waiting.is_none());
    assert_eq!(ready.playback_rate("12:34:56:78:90:AB").unwrap(), 44_100);
    assert_eq!(ready.service, "ready");
}

#[test]
fn commands_name_devices_by_address_and_codecs_by_label() {
    let (_tx, commands) = sync_channel(1);
    let (states, _rx) = sync_channel(1);
    let (_a, wake_receiver) = UnixStream::pair().unwrap();
    let mut client = Client {
        log: observer(),
        socket: "/nonexistent".into(),
        commands,
        wake_receiver,
        states,
        ui_wake: None,
        last: Status::default(),
        waiting: Vec::new(),
        unsent: None,
    };
    client.last = status_from(&daemon_state());
    let path = "/org/bluez/hci0/dev_12_34_56_78_90_AB".to_string();
    assert_eq!(
        client.request(Command::Connect(path.clone())),
        Some(Request::Connect {
            address: "12:34:56:78:90:AB".into()
        })
    );
    assert_eq!(
        client.request(Command::Volume {
            device: path,
            volume: 90
        }),
        Some(Request::Volume {
            address: "12:34:56:78:90:AB".into(),
            volume: 90
        })
    );
    assert_eq!(client.request(Command::Connect("/unknown".into())), None);
    assert_eq!(
        client.request(Command::Codec {
            address: String::new(),
            preference: CodecPreference::AptxHd
        }),
        Some(Request::CodecPreference {
            preference: "aptX-HD".into()
        })
    );
    assert_eq!(
        client.request(Command::Power(false)),
        Some(Request::BluetoothPower {
            on: false,
            persist: true
        })
    );
}

#[test]
fn a_daemon_that_is_away_keeps_the_owners_requests_for_later() {
    let (tx, commands) = sync_channel(4);
    let (states, _rx) = sync_channel(1);
    let (_a, wake_receiver) = UnixStream::pair().unwrap();
    let mut client = Client {
        log: observer(),
        socket: "/nonexistent/radiod.sock".into(),
        commands,
        wake_receiver,
        states,
        ui_wake: None,
        last: Status::default(),
        waiting: Vec::new(),
        unsent: None,
    };
    tx.send(Command::Power(true)).unwrap();
    assert!(!client.idle(Duration::from_millis(50)));
    assert_eq!(
        client.waiting,
        [Request::BluetoothPower {
            on: true,
            persist: true
        }]
    );
    tx.send(Command::Stop).unwrap();
    assert!(client.idle(Duration::from_millis(50)));
}

#[test]
fn codec_rows_only_offer_policy_enabled_codecs_the_peer_supports() {
    use crate::codecs::{Capability, Inventory, Session};
    let mut inventory = Inventory {
        schema: 1,
        ..Default::default()
    };
    for (name, approved) in [("SBC", true), ("AAC", false), ("LDAC", false)] {
        inventory.codecs.insert(
            name.into(),
            Capability {
                compiled_locally: true,
                distribution_approved: approved,
                platform_qualified: approved,
                owner_private_experiment: !approved,
            },
        );
    }
    let sbc = vec!["SBC".to_string()];
    let all = vec!["SBC".into(), "AAC".into(), "LDAC".into()];
    let mut status = Status {
        codec_policy: Some(Session::new(
            1,
            CodecPreference::Sbc,
            &inventory,
            &sbc,
            &sbc,
        )),
        ..Default::default()
    };
    assert!(status.codec_choices().is_empty(), "SBC only: no dead row");
    status.codec_policy = Some(Session::new(
        1,
        CodecPreference::Sbc,
        &inventory,
        &sbc,
        &all,
    ));
    assert!(status.codec_choices().is_empty());
    status.codec_policy = Some(Session::new_experimental(
        1,
        CodecPreference::Sbc,
        &inventory,
        &all,
        &["SBC".into(), "AAC".into()],
        true,
    ));
    assert_eq!(
        status.codec_choices(),
        vec![
            CodecPreference::Auto,
            CodecPreference::Sbc,
            CodecPreference::Aac
        ]
    );
}
