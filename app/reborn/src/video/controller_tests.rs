use super::*;
use std::sync::mpsc::sync_channel;

#[test]
fn video_failures_use_a_registered_diagnostic_channel() {
    assert!(reborn_observability::SUBSYSTEMS.contains(&LOG_SUBSYSTEM));
}

#[test]
fn videos_do_not_preopen_a_second_audio_pass_when_music_repeats() {
    let mut model = AppModel::default();
    model.settings.repeat = reborn_core::RepeatMode::All;
    model.settings.gapless_enabled = true;
    let track = Track {
        path: "/music/clip.mp4".into(),
        ..Track::default()
    };
    model.replace_queue(vec![track.clone()], 0).unwrap();
    let mut controller = Controller::new(PathBuf::from("/missing-video-resume.json"));
    assert_eq!(
        controller.upcoming(&model).0.len(),
        1,
        "music still repeats"
    );
    assert!(controller.gapless(&model));
    controller.current = Some(track);
    let (tracks, ids) = controller.upcoming(&model);
    assert!(
        tracks.is_empty() && ids.is_empty(),
        "no second video audio pass"
    );
    assert!(!controller.gapless(&model));
    assert_eq!(model.settings.repeat, reborn_core::RepeatMode::All);
    assert!(
        model.settings.gapless_enabled,
        "music preferences stay intact"
    );
}

fn frame(position_ms: u64) -> Reply {
    Reply::Frame(VideoFrame {
        rgba: vec![0, 0, 0, 255],
        width: 1,
        height: 1,
        position_ms,
    })
}

#[test]
fn presentation_follows_the_audio_clock_and_pause_cannot_advance_pictures() {
    let mut controller = Controller::new(PathBuf::from("/missing-video-resume.json"));
    let (tx, rx) = sync_channel(2);
    tx.send(frame(40)).unwrap();
    tx.send(frame(80)).unwrap();
    controller.replies = Some(rx);
    assert!(controller.frame(0).unwrap().is_none());
    assert_eq!(controller.frame(40).unwrap().unwrap().position_ms, 40);
    assert!(controller.frame(40).unwrap().is_none());
    assert_eq!(controller.frame(80).unwrap().unwrap().position_ms, 80);
    tx.send(frame(120)).unwrap();
    assert!(
        controller.frame(400).unwrap().is_none(),
        "late pictures are discarded"
    );
}

#[test]
fn a_seek_clears_the_pending_picture_and_old_channel() {
    let mut controller = Controller::new(PathBuf::from("/missing-video-resume.json"));
    let (tx, rx) = sync_channel(2);
    tx.send(frame(1000)).unwrap();
    controller.replies = Some(rx);
    assert!(controller.frame(0).unwrap().is_none());
    assert!(controller.pending.is_some());
    controller.stop_decode();
    assert!(controller.pending.is_none());
    assert!(controller.replies.is_none());
    assert!(tx.send(frame(1100)).is_err());
}

#[test]
fn checkpoint_and_sleep_save_the_music_queue_and_video_position_separately() {
    let root = std::env::temp_dir().join(format!("reborn-video-checkpoint-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut music = AppModel::default();
    music
        .replace_queue(
            vec![Track {
                path: "/music/song.flac".into(),
                ..Track::default()
            }],
            0,
        )
        .unwrap();
    music.position_ms = 12_000;
    music.playback = PlaybackState::Paused;
    let mut video = AppModel::default();
    let track = Track {
        path: "/music/clip.mp4".into(),
        duration_ms: 60_000,
        ..Track::default()
    };
    video.replace_queue(vec![track.clone()], 0).unwrap();
    video.position_ms = 25_000;
    video.settings.volume = 42;
    let path = root.join("session.json");
    let mut controller = Controller::new(root.join("video-resume.json"));
    controller.music = Some(Box::new(music));
    controller.current = Some(track.clone());
    controller.checkpoint(&video, &path).unwrap().unwrap();
    let saved = AppModel::restore(&path).unwrap();
    assert_eq!(saved.current().unwrap().path, Path::new("/music/song.flac"));
    assert_eq!(saved.position_ms, 12_000);
    assert_eq!(saved.settings.volume, 42);
    assert_eq!(
        Controller::new(root.join("video-resume.json")).resume(&track),
        25_000
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resume_is_atomic_bounded_and_survives_restart_without_touching_the_music_session() {
    let root = std::env::temp_dir().join(format!("reborn-video-resume-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("video-resume.json");
    fs::write(root.join("session.json"), b"music session").unwrap();
    let track = Track {
        path: "/media/internal/music/clip.mp4".into(),
        size: 10,
        mtime: 20,
        duration_ms: 60_000,
        ..Track::default()
    };
    let mut controller = Controller::new(path.clone());
    controller.current = Some(track.clone());
    controller.save(12_000).unwrap();
    let restored = Controller::new(path.clone());
    assert_eq!(restored.resume(&track), 12_000);
    controller.resumes = (0..128)
        .map(|i| Resume {
            path: format!("/clip-{i}.mp4"),
            ..Resume::default()
        })
        .collect();
    controller.save(20_000).unwrap();
    assert_eq!(Controller::new(path.clone()).resumes.len(), 128);
    assert_eq!(
        fs::read(root.join("session.json")).unwrap(),
        b"music session"
    );
    fs::write(&path, b"broken json").unwrap();
    assert_eq!(Controller::new(path).resume(&track), 0);
    fs::remove_dir_all(root).unwrap();
}
