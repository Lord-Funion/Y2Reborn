use super::*;
fn status() -> bluetooth::Status {
    let mut status = bluetooth::Status::default();
    status.available = true;
    status.powered = true;
    status.bluealsa = true;
    status.devices.push(bluetooth::Device {
        address: "AA".into(),
        path: "/peer".into(),
        connected: true,
        audio: true,
        paired: true,
        bonded: true,
        ..Default::default()
    });
    status.pcms.push(reborn_core::BluetoothPcm {
        device: "/peer".into(),
        transport: "A2DP-source".into(),
        mode: "sink".into(),
        running: Some(false),
        transport_generation: 1,
        ..Default::default()
    });
    status
}
#[test]
fn saved_codec_retries_only_on_real_reconnect_and_never_during_playback() {
    let mut guard = CodecReconnect::default();
    let mut s = status();
    assert!(guard.next(&s, PlaybackState::Playing, false).is_none());
    assert!(guard.next(&s, PlaybackState::Paused, true).is_none());
    s.pcms[0].running = None;
    assert!(guard.next(&s, PlaybackState::Paused, false).is_none());
    s.pcms[0].running = Some(false);
    assert_eq!(
        guard.next(&s, PlaybackState::Paused, false).as_deref(),
        Some("AA")
    );
    // Queue-full does not consume the sole attempt; caller confirms send.
    assert_eq!(
        guard.next(&s, PlaybackState::Paused, false).as_deref(),
        Some("AA")
    );
    guard.sent("AA".into());
    s.pcms[0].transport_generation = 2; // Codec selection may recreate PCM.
    assert!(guard.next(&s, PlaybackState::Paused, false).is_none());
    s.available = false;
    s.devices[0].connected = false;
    assert!(guard.next(&s, PlaybackState::Paused, false).is_none());
    s.available = true;
    s.devices[0].connected = true; // Failed snapshot did not prove a disconnect.
    assert!(guard.next(&s, PlaybackState::Paused, false).is_none());
    s.devices[0].connected = false;
    assert!(guard.next(&s, PlaybackState::Paused, false).is_none());
    s.devices[0].connected = true;
    assert_eq!(
        guard.next(&s, PlaybackState::Stopped, false).as_deref(),
        Some("AA")
    );
}
#[test]
fn a_choice_refused_by_playback_is_asked_again_at_the_next_pause() {
    // Y2Reborn #87: asked while connecting, refused once music started on
    // the headphones; the saved LDAC never applied (link stayed AAC 320k).
    let mut guard = CodecReconnect::default();
    let mut s = status();
    assert_eq!(
        guard.next(&s, PlaybackState::Paused, false).as_deref(),
        Some("AA")
    );
    guard.sent("AA".into());
    let refused = |seq| {
        Some(bluetooth::CodecDeferred {
            address: "AA".into(),
            seq,
        })
    };
    s.codec_deferred = refused(1);
    s.pcms[0].running = Some(true);
    assert!(guard.next(&s, PlaybackState::Playing, false).is_none());
    s.pcms[0].running = Some(false);
    assert_eq!(
        guard.next(&s, PlaybackState::Paused, false).as_deref(),
        Some("AA")
    );
    guard.sent("AA".into());
    // The same refusal in a later snapshot does not re-arm it.
    assert!(guard.next(&s, PlaybackState::Paused, false).is_none());
    for seq in 2..=3 {
        s.codec_deferred = refused(seq);
        assert_eq!(
            guard.next(&s, PlaybackState::Paused, false).as_deref(),
            Some("AA")
        );
        guard.sent("AA".into());
    }
    // Three refusals per connection, then it waits for a reconnect.
    s.codec_deferred = refused(4);
    assert!(guard.next(&s, PlaybackState::Paused, false).is_none());
    s.devices[0].connected = false;
    assert!(guard.next(&s, PlaybackState::Paused, false).is_none());
    s.devices[0].connected = true;
    assert_eq!(
        guard.next(&s, PlaybackState::Paused, false).as_deref(),
        Some("AA")
    );
    guard.sent("AA".into());
    s.codec_deferred = refused(5);
    assert_eq!(
        guard.next(&s, PlaybackState::Paused, false).as_deref(),
        Some("AA")
    );
    guard.sent("AA".into());
    // A restarted worker counts its refusals from 1 again.
    s.codec_deferred = None;
    assert!(guard.next(&s, PlaybackState::Paused, false).is_none());
    s.codec_deferred = refused(1);
    assert_eq!(
        guard.next(&s, PlaybackState::Paused, false).as_deref(),
        Some("AA")
    );
}
#[test]
fn prepared_sleep_allows_restore_and_observation_and_refuses_mutations() {
    for command in [
        Command::Status,
        Command::Audio,
        Command::Health,
        Command::Metrics,
        Command::Snapshot,
        Command::SleepRestore { dark: false },
        Command::SleepRestore { dark: true },
    ] {
        assert!(sleep_control_allowed(&command));
    }
    for command in [
        Command::SleepPrepare,
        Command::Scan,
        Command::Diagnose,
        Command::Tests,
    ] {
        assert!(!sleep_control_allowed(&command));
    }
}
