//! Video library and click-wheel playback controls, using the ordinary skin fallback path.
use crate::{
    canvas::{Canvas, Icon},
    components,
    pages::{Hero, Page},
    text::Face,
    theme::{color, HEIGHT, WIDTH},
    Item, Ui,
};
use reborn_core::{video::Request, Action, AppModel, Effect, PlaybackState, Screen, Track};
use reborn_graphics::{Quad, Tex};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct View {
    pub rows: Vec<Item>,
    pub loading: bool,
    pub error: Option<String>,
    pub title: String,
    pub detail: String,
    pub picture: Option<(u32, u32)>,
    pub controls_at: Option<Instant>,
}

pub(crate) fn library(ui: &Ui) -> Page {
    let v = &ui.video;
    Page {
        hero: if v.rows.is_empty() {
            Some(Hero {
                title: if v.loading {
                    "Finding Videos…"
                } else {
                    "No Videos"
                }
                .into(),
                body: "Copy videos to Internal Storage's music folder or the SD card.".into(),
            })
        } else {
            None
        },
        rows: v
            .rows
            .iter()
            .cloned()
            .chain(std::iter::once(Item::new("Refresh Videos", "video_scan")))
            .collect(),
        status: v.error.clone(),
        ..Default::default()
    }
}

impl Ui {
    pub fn close_video(&mut self, m: &mut AppModel) {
        if m.screen == Screen::VideoPlaying {
            Self::back(m);
        }
        self.video.picture = None;
    }
    pub(crate) fn screen_action(
        &mut self,
        m: &mut AppModel,
        tracks: &[Track],
        action: Action,
    ) -> Option<Effect> {
        if m.screen_off {
            return None;
        }
        if let Some(effect) = self.video_action(m, action) {
            return Some(effect);
        }
        self.normalize(m, tracks);
        None
    }

    pub(crate) fn video_select(&mut self, m: &mut AppModel, key: &str) -> Option<Effect> {
        if key == "videos" {
            Self::go(m, Screen::Videos, "");
            self.video.loading = true;
            return Some(Effect::Video(Request::Scan));
        }
        if key == "video_scan" {
            self.video.loading = true;
            return Some(Effect::Video(Request::Scan));
        }
        let index = key.strip_prefix("video:")?.parse().ok()?;
        Self::go(m, Screen::VideoPlaying, "");
        Some(Effect::Video(Request::Play(index)))
    }

    pub(crate) fn video_action(&mut self, m: &mut AppModel, action: Action) -> Option<Effect> {
        if m.screen != Screen::VideoPlaying || m.navigation.modal.is_some() {
            return None;
        }
        self.video.controls_at = Some(Instant::now());
        Some(match action {
            Action::Back => {
                Self::back(m);
                Effect::Video(Request::Stop)
            }
            Action::Home => {
                Self::home(m);
                Effect::Video(Request::Stop)
            }
            Action::Select | Action::PlayPause if m.playback == PlaybackState::Stopped => {
                Effect::Video(Request::Restart)
            }
            Action::Select | Action::PlayPause => Effect::TogglePlayback,
            Action::PreviousTrack | Action::NavigateLeft => Effect::Video(Request::Restart),
            Action::NextTrack | Action::NavigateRight | Action::SeekForward => Effect::Seek(1),
            Action::SeekBackward => Effect::Seek(-1),
            Action::WheelClockwise(n) => Effect::Seek(i64::from(n.max(1))),
            Action::WheelCounterClockwise(n) => Effect::Seek(-i64::from(n.max(1))),
            Action::NavigateUp | Action::VolumeUp => Effect::AdjustVolume(2),
            Action::NavigateDown | Action::VolumeDown => Effect::AdjustVolume(-2),
            Action::ContextMenu => Effect::Video(Request::Restart),
            Action::ShowNowPlaying => Effect::None,
            Action::ScreenSleep | Action::PowerShort => Effect::ScreenSleep,
            Action::ScreenWake => Effect::ScreenWake,
            Action::PowerHold => {
                self.power_held(m);
                Effect::None
            }
            Action::PowerHoldCancel => {
                self.power_hold = None;
                Effect::None
            }
        })
    }
}

pub(crate) fn playing(c: &mut Canvas, ui: &Ui, m: &AppModel) {
    // The skin supplies the letterbox background; decoded pixels keep their original colours.
    if let Some((w, h)) = ui.video.picture {
        let (w, h) = (w as f32, h as f32);
        c.rect(0., 0., WIDTH, HEIGHT, color::BLACK);
        c.draw.push(Quad {
            tex: Tex::Video,
            ..Quad::rect((WIDTH - w) / 2., (HEIGHT - h) / 2., w, h, 0xFFFFFFFF)
        });
    }
    let playing = m.playback == PlaybackState::Playing;
    let controls = !playing
        || ui
            .video
            .controls_at
            .is_none_or(|t| t.elapsed() < Duration::from_secs(3));
    if !controls {
        return;
    }
    c.rect(0., 0., WIDTH, 55., color::BG);
    c.text_mid(18., 19., 444., &ui.video.title, Face::S17, color::TEXT);
    c.text_mid(18., 41., 444., &ui.video.detail, Face::R11, color::MUTED);
    c.rect(0., 270., WIDTH, 90., color::BG);
    let duration = m.current().map_or(0, |t| t.duration_ms);
    let progress = components::progress(m.position_ms, duration);
    c.rrect(18., 283., 444., 4., 2., color::TRACK);
    c.rrect(18., 283., 444. * progress, 4., 2., color::ACCENT);
    c.icon(
        if playing { Icon::Pause } else { Icon::Play },
        18.,
        305.,
        16.,
        color::TEXT,
    );
    c.text(
        44.,
        318.,
        &components::time(m.position_ms),
        Face::R13,
        color::TEXT,
    );
    c.text_right(
        462.,
        318.,
        &components::time(duration),
        Face::R13,
        color::MUTED,
    );
    let hint = ui
        .video
        .error
        .as_deref()
        .unwrap_or("Centre: play/pause · Wheel: seek · Back: stop");
    c.text_mid_center(
        240.,
        342.,
        &c.fit_text(Face::R11, hint, 444.),
        Face::R11,
        if ui.video.error.is_some() {
            color::DANGER
        } else {
            color::DIM
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_controls_use_the_existing_transport_and_back_stops_video() {
        let mut ui = Ui::default();
        let mut m = AppModel {
            playback: PlaybackState::Paused,
            ..AppModel::default()
        };
        Ui::go(&mut m, Screen::Videos, "");
        Ui::go(&mut m, Screen::VideoPlaying, "");
        assert!(matches!(
            ui.video_action(&mut m, Action::Select),
            Some(Effect::TogglePlayback)
        ));
        assert!(matches!(
            ui.video_action(&mut m, Action::WheelClockwise(2)),
            Some(Effect::Seek(2))
        ));
        assert!(matches!(
            ui.video_action(&mut m, Action::Back),
            Some(Effect::Video(Request::Stop))
        ));
        assert_eq!(m.screen, Screen::Videos);
        Ui::go(&mut m, Screen::VideoPlaying, "");
        assert!(matches!(
            ui.video_action(&mut m, Action::Home),
            Some(Effect::Video(Request::Stop))
        ));
        assert_eq!(m.screen, Screen::Home);
    }
}
