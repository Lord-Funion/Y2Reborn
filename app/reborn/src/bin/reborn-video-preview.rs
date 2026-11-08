//! Deterministic production UI previews with a picture decoded from the synthetic video fixture.
#[path = "../skins.rs"]
mod skins;
use reborn_core::{AppModel, PlaybackState, Screen, Track};
use reborn_media::{Cancel, VideoDecoder};
use reborn_ui::{Item, Ui};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::Instant,
};

fn tracks() -> Vec<Track> {
    [
        "Blue Water",
        "Night Drive",
        "Paper Moons",
        "Quiet Harbour",
        "Slow Light",
        "Sunrise",
    ]
    .iter()
    .enumerate()
    .map(|(i, album)| Track {
        id: i as i64 + 1,
        source_id: "internal".into(),
        path: format!("/media/internal/music/{album}/01.flac").into(),
        title: format!("Song {}", i + 1),
        album: (*album).into(),
        artist: "Reborn Sample Library".into(),
        album_artist: "Reborn Sample Library".into(),
        online: true,
        artwork: true,
        duration_ms: 180_000,
        ..Track::default()
    })
    .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let output = PathBuf::from(
        args.first()
            .ok_or("usage: reborn-video-preview OUT [--skin DIR]")?,
    );
    fs::create_dir_all(&output)?;
    let mut ui = Ui::default();
    ui.motion_fixed = true;
    ui.spin_fixed = Some(0.45);
    ui.clock_fixed = Some(1_793_980_800);
    ui.themes = skins::scan(&[Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skins")]);
    if let Some(dir) = args
        .iter()
        .position(|s| s == "--skin")
        .and_then(|i| args.get(i + 1))
    {
        let dir = Path::new(dir);
        let skin = skins::load(dir, "preview", &|p, w, h| {
            reborn_media::decode_image(p, w, h)
        })?;
        fs::write(output.join("skin-atlas.rgba"), &skin.atlas)?;
        fs::write(
            output.join("skin-atlas.json"),
            serde_json::to_vec(&json!({
            "width": skin.atlas_width, "height": skin.atlas_height }))?,
        )?;
        ui.skin = Some(Rc::new(skin));
    }
    fs::write(
        output.join("cover-style.json"),
        serde_json::to_vec(&json!({
        "radius": ui.cover_radius() }))?,
    )?;
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fixtures/video-h264-aac.mp4");
    let mut decoder = VideoDecoder::open(&fixture, Cancel::new()?)?;
    let frame = loop {
        let frame = decoder
            .read()?
            .ok_or("synthetic video has no picture at 500 ms")?;
        if frame.position_ms >= 500 {
            break frame;
        }
    };
    fs::write(output.join("video-frame.rgba"), &frame.rgba)?;
    fs::write(
        output.join("video-frame.json"),
        serde_json::to_vec(&json!({
        "width": frame.width, "height": frame.height, "position_ms": frame.position_ms }))?,
    )?;
    ui.video.title = "Reborn motion test.mp4".into();
    ui.video.detail = "H.264 · 320 × 180".into();
    ui.video.picture = Some((frame.width, frame.height));
    ui.video.controls_at = Some(Instant::now());
    ui.video.rows = [
        ("Reborn motion test.mp4", "0:02 · 320 × 180 · h264"),
        ("Train journey.mkv", "0:02 · 320 × 180 · h264"),
        ("Morning light.webm", "0:01 · 160 × 90 · vp8"),
    ]
    .iter()
    .enumerate()
    .map(|(i, (name, detail))| Item::new(*name, format!("video:{i}")).with_secondary(*detail))
    .collect();
    let tracks = tracks();
    ui.set_resident_thumbs(tracks.iter().map(reborn_ui::art_key));
    let mut m = AppModel {
        screen: Screen::Videos,
        position_ms: 500,
        playback: PlaybackState::Playing,
        ..AppModel::default()
    };
    m.library.tracks = tracks;
    m.queue = vec![Track {
        id: -1,
        title: ui.video.title.clone(),
        duration_ms: 2000,
        online: true,
        ..Track::default()
    }];
    for (name, screen, state) in [
        ("home", Screen::Home, PlaybackState::Playing),
        ("video-library", Screen::Videos, PlaybackState::Playing),
        ("cover-flow", Screen::Albums, PlaybackState::Playing),
        (
            "video-playing",
            Screen::VideoPlaying,
            PlaybackState::Playing,
        ),
        ("video-paused", Screen::VideoPlaying, PlaybackState::Paused),
    ] {
        m.screen = screen;
        m.playback = state;
        m.navigation.focus = if screen == Screen::Albums { 2 } else { 0 };
        let tracks = std::mem::take(&mut m.library.tracks);
        ui.normalize(&mut m, &tracks);
        m.library.tracks = tracks;
        let quads = ui.draw(&m, &m.library.tracks, false);
        fs::write(
            output.join(format!("{name}.json")),
            serde_json::to_vec(&json!({
            "width": 480, "height": 360, "fixture": true, "quads": quads }))?,
        )?;
    }
    ui.keyboard = Some(reborn_ui::keyboard::Keyboard::new(
        reborn_ui::keyboard::Purpose::RadioSearch,
        "Blue water",
    ));
    m.screen = Screen::TextEntry;
    fs::write(
        output.join("keyboard.json"),
        serde_json::to_vec(&json!({
            "width": 480, "height": 360, "fixture": true,
            "quads": ui.draw(&m, &m.library.tracks, false)
        }))?,
    )?;
    Ok(())
}
