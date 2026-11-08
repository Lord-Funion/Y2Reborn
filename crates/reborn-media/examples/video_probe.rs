//! Decode a local video for host or target qualification; reports measurements, never a device claim.
use reborn_media::{Cancel, VideoDecoder};
use serde_json::json;
use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(std::env::args().nth(1).ok_or("usage: video_probe FILE")?);
    let started = Instant::now();
    let mut decoder = VideoDecoder::open(&path, Cancel::new()?)?;
    let opened = started.elapsed();
    let mut count = 0u64;
    let mut first = None;
    let mut last = 0;
    let mut maximum = 0;
    let decode = Instant::now();
    while let Some(frame) = decoder.read()? {
        first.get_or_insert(frame.position_ms);
        last = frame.position_ms;
        maximum = maximum.max(frame.rgba.len());
        count += 1;
    }
    let elapsed = decode.elapsed().as_secs_f64();
    let rss = std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("VmHWM:"))
                .and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse::<u64>().ok())
        });
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "architecture": std::env::consts::ARCH, "ffmpeg": reborn_media::version(),
            "codec": decoder.info.codec, "source_width": decoder.info.width,
            "source_height": decoder.info.height, "frames": count,
            "first_ms": first, "last_ms": last, "duration_ms": decoder.info.duration_ms,
            "open_ms": opened.as_secs_f64() * 1000., "decode_seconds": elapsed,
            "decode_fps": if elapsed > 0. { count as f64 / elapsed } else { 0. },
            "largest_rgba_bytes": maximum, "peak_rss_kib": rss,
            "real_time_playback_test": false
        }))?
    );
    Ok(())
}
