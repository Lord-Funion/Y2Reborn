use super::*;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fixtures/video-h264-aac.mp4")
}

#[test]
fn h264_decodes_moving_pictures_with_ordered_timestamps_and_matching_audio() {
    let mut video = VideoDecoder::open(&fixture(), Cancel::new().unwrap()).unwrap();
    assert_eq!((video.info.width, video.info.height), (320, 180));
    assert_eq!(
        (video.info.display_width, video.info.display_height),
        (480, 270)
    );
    assert_eq!(video.info.codec, "h264");
    assert!((1_900..=2_100).contains(&video.info.duration_ms));
    let mut previous = 0;
    let mut count = 0;
    let mut first = Vec::new();
    let mut changed = false;
    while let Some(frame) = video.read().unwrap() {
        assert!(frame.position_ms >= previous);
        assert!(frame.rgba.len() <= reborn_core::video::FRAME_BYTES);
        if count == 0 {
            first = frame.rgba.clone();
        } else {
            changed |= first != frame.rgba;
        }
        previous = frame.position_ms;
        count += 1;
    }
    assert_eq!(count, 40);
    assert!(
        changed,
        "actual moving pictures, not a repeated still image"
    );
    let mut audio = Decoder::open_video_audio(
        &fixture(),
        OutputSpec {
            rate: 44_100,
            format: PcmFormat::S16LE,
        },
        DspConfig::with_volume(100),
        Cancel::new().unwrap(),
    )
    .unwrap();
    let mut end = 0;
    while let Some(pcm) = audio.read().unwrap() {
        end = pcm.position_ms + pcm.frames * 1000 / u64::from(pcm.rate);
    }
    assert!(
        end.abs_diff(video.info.duration_ms) < 100,
        "audio ends at {end}"
    );
    assert!(
        end.abs_diff(previous) < 100,
        "last picture is {previous}, audio ends at {end}"
    );
}

#[test]
fn video_seeks_after_eof_cancels_and_rejects_non_video_sources() {
    let cancel = Cancel::new().unwrap();
    let mut video = VideoDecoder::open(&fixture(), cancel.clone()).unwrap();
    while video.read().unwrap().is_some() {}
    video.seek(1000).unwrap();
    let mut stamp = 0;
    while let Some(frame) = video.read().unwrap() {
        stamp = frame.position_ms;
        if stamp >= 1000 {
            break;
        }
    }
    assert!((1000..1100).contains(&stamp));
    cancel.cancel();
    assert!(video.read().is_err());
    assert!(video.seek(u64::MAX).is_err());
    assert!(VideoDecoder::open(Path::new("/missing.mp4"), Cancel::new().unwrap()).is_err());
    let audio = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fixtures/m4a-aac.m4a");
    assert!(VideoDecoder::open(&audio, Cancel::new().unwrap()).is_err());
}

#[test]
fn truncated_video_fails_without_leaking_descriptors() {
    let path = std::env::temp_dir().join(format!("reborn-truncated-{}.mp4", std::process::id()));
    let bytes = std::fs::read(fixture()).unwrap();
    std::fs::write(&path, &bytes[..40]).unwrap();
    let descriptors = || {
        std::fs::read_dir("/proc/self/fd")
            .unwrap()
            .flatten()
            .filter(|e| std::fs::read_link(e.path()).ok().as_ref() == Some(&path))
            .count()
    };
    for _ in 0..20 {
        assert!(VideoDecoder::open(&path, Cancel::new().unwrap()).is_err());
    }
    assert_eq!(
        descriptors(),
        0,
        "failed opens must close their file descriptors"
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn matroska_h264_aac_and_webm_vp8_opus_use_the_same_membrane() {
    for name in ["video-h264-aac.mkv", "video-vp8-opus.webm"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fixtures")
            .join(name);
        let mut video = VideoDecoder::open(&path, Cancel::new().unwrap()).unwrap();
        assert!(
            video.read().unwrap().is_some(),
            "{name}: moving picture stream"
        );
        let mut audio = Decoder::open_video_audio(
            &path,
            OutputSpec {
                rate: 48_000,
                format: PcmFormat::S16LE,
            },
            DspConfig::with_volume(100),
            Cancel::new().unwrap(),
        )
        .unwrap();
        assert!(
            audio.read().unwrap().is_some(),
            "{name}: decoded audio stream"
        );
    }
}
