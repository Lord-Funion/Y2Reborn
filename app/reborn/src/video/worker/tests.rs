use super::*;
use reborn_core::MediaSource;

fn temporary(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("reborn-video-{name}-{}", std::process::id()))
}

#[test]
fn discovery_finds_both_sources_skips_symlinks_and_keeps_music_out() {
    let root = temporary("discover");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("internal/nested")).unwrap();
    fs::create_dir_all(root.join("sd")).unwrap();
    for path in [
        "internal/nested/clip.MP4",
        "sd/clip.webm",
        "sd/music.mp3",
        "sd/image.jpg",
    ] {
        fs::write(root.join(path), b"candidate").unwrap();
    }
    std::os::unix::fs::symlink(root.join("sd"), root.join("internal/loop")).unwrap();
    let source = |id: &str, name: &str, kind| Source {
        id: id.into(),
        kind,
        root: root.join(name),
        online: true,
        mount: "host".into(),
        mount_id: None,
    };
    let sources = [
        source("internal", "internal", MediaSource::Internal),
        source("sd", "sd", MediaSource::SdCard("test".into())),
    ];
    let cancel = Cancel::new().unwrap();
    let found = discover(&sources, &cancel);
    assert_eq!(found.len(), 2);
    assert!(found.iter().any(|t| t.source_id == "internal"));
    assert!(found.iter().any(|t| t.source_id == "sd"));
    cancel.cancel();
    assert!(discover(&sources, &cancel).is_empty());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn replacing_a_decoder_job_cancels_its_old_output() {
    let root = temporary("worker");
    fs::create_dir_all(&root).unwrap();
    let path = root.join("clip.mp4");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fixtures/video-h264-aac.mp4"),
        &path,
    )
    .unwrap();
    let cancel = Cancel::new().unwrap();
    let source = Source {
        id: "internal".into(),
        kind: MediaSource::Internal,
        root: root.clone(),
        online: true,
        mount: "host".into(),
        mount_id: None,
    };
    let track = discover(&[source], &cancel).pop().unwrap();
    let mut worker = Worker::new().unwrap();
    let first = worker.start(track.clone(), &root, 0).unwrap();
    assert!(matches!(
        first.recv_timeout(Duration::from_secs(10)).unwrap(),
        Reply::Opened(_)
    ));
    // Replace the job without consuming the rest of its bounded output.
    let next = worker.start(track, &root, 500).unwrap();
    assert!(matches!(
        next.recv_timeout(Duration::from_secs(10)).unwrap(),
        Reply::Opened(_)
    ));
    drop(first);
    let until = std::time::Instant::now() + Duration::from_secs(10);
    while !worker.quiesce() {
        assert!(
            std::time::Instant::now() < until,
            "video storage must quiesce for sleep"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert!(next.try_iter().count() <= FRAME_QUEUE);
    assert!(
        matches!(
            next.recv_timeout(Duration::from_secs(2)),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        ),
        "quiescence drops the pinned decoder and job"
    );
    drop(next);
    drop(worker);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancellation_releases_a_sender_with_a_provably_full_frame_queue() {
    let cancel = Cancel::new().unwrap();
    let (tx, _rx) = sync_channel(FRAME_QUEUE);
    for _ in 0..FRAME_QUEUE {
        tx.send(Reply::End).unwrap();
    }
    let job = Job {
        track: Track::default(),
        music: PathBuf::new(),
        position_ms: 0,
        cancel: cancel.clone(),
        tx,
    };
    let (done, result) = sync_channel(1);
    let worker = thread::spawn(move || {
        done.send(DecodeWorker::send(&job, Reply::End)).unwrap();
    });
    cancel.cancel();
    assert!(!result.recv_timeout(Duration::from_secs(2)).unwrap());
    worker.join().unwrap();
}
