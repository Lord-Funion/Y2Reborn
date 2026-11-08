//! Video admission, presentation timing and rollback-safe resume records.
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FRAME_WIDTH: u32 = 480;
pub const FRAME_HEIGHT: u32 = 360;
pub const FRAME_BYTES: usize = FRAME_WIDTH as usize * FRAME_HEIGHT as usize * 4;
pub const FRAME_QUEUE: usize = 2;
pub const MAX_FILES: usize = 10_000;
pub const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Scan,
    Play(usize),
    Stop,
    Restart,
}

/// Extensions nominate candidates only; FFmpeg must still validate the container and streams.
pub fn candidate(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "mp4" | "m4v" | "mkv" | "webm"
        )
    })
}

/// A decoded picture is shown against the audible source clock, never the decode clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Timing {
    Wait,
    Show,
    Drop,
}

pub fn timing(picture_ms: u64, heard_ms: u64) -> Timing {
    if picture_ms > heard_ms.saturating_add(15) {
        Timing::Wait
    } else if picture_ms.saturating_add(150) < heard_ms {
        Timing::Drop
    } else {
        Timing::Show
    }
}

/// Size in square screen pixels, accounting for the encoded sample aspect ratio.
pub fn fit(width: u32, height: u32, sar_num: u32, sar_den: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    let aspect = f64::from(width) * f64::from(sar_num.max(1))
        / (f64::from(height) * f64::from(sar_den.max(1)));
    let w = f64::from(FRAME_WIDTH).min(f64::from(FRAME_HEIGHT) * aspect);
    (
        w.round().max(1.) as u32,
        (w / aspect).round().max(1.) as u32,
    )
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Resume {
    pub path: String,
    pub bytes: u64,
    pub modified: u64,
    pub position_ms: u64,
}

impl Resume {
    pub fn position(&self, bytes: u64, modified: u64, duration_ms: u64) -> u64 {
        if self.bytes != bytes
            || self.modified != modified
            || self.position_ms.saturating_add(5_000) >= duration_ms
        {
            0
        } else {
            self.position_ms
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_are_case_insensitive_and_do_not_admit_music_or_images() {
        for path in ["clip.MP4", "a.m4v", "a.mkv", "a.WebM"] {
            assert!(candidate(Path::new(path)));
        }
        for path in ["a.mp3", "a.m4a", "a.jpg", "mp4", "a.mp4.exe"] {
            assert!(!candidate(Path::new(path)));
        }
    }

    #[test]
    fn clock_holds_future_pictures_and_discards_late_ones_without_drift() {
        assert_eq!(timing(1000, 900), Timing::Wait);
        assert_eq!(timing(1000, 1000), Timing::Show);
        assert_eq!(timing(1000, 1200), Timing::Drop);
        assert_eq!(timing(u64::MAX, u64::MAX), Timing::Show);
    }

    #[test]
    fn aspect_ratio_includes_anamorphic_and_portrait_content() {
        assert_eq!(fit(1920, 1080, 1, 1), (480, 270));
        assert_eq!(fit(720, 576, 16, 15), (480, 360));
        assert_eq!(fit(360, 640, 1, 1), (203, 360));
        assert_eq!(fit(0, 640, 1, 1), (0, 0));
    }

    #[test]
    fn resume_restarts_finished_or_replaced_files() {
        let r = Resume {
            bytes: 10,
            modified: 20,
            position_ms: 15_000,
            ..Resume::default()
        };
        assert_eq!(r.position(10, 20, 60_000), 15_000);
        assert_eq!(r.position(11, 20, 60_000), 0);
        assert_eq!(r.position(10, 21, 60_000), 0);
        assert_eq!(r.position(10, 20, 19_000), 0);
    }
}
