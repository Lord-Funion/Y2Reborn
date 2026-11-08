//! The video controller's connection to navigation, the existing music transport and persistence.
use super::*;

impl Runtime {
    pub(crate) fn video_prepare_sleep(&mut self) -> bool {
        self.ui.video.loading = false;
        self.video.suspended = self.video.active();
        self.video.quiesce()
    }

    pub(crate) fn video_recreate_picture(&mut self) {
        self.ui.video.picture = None;
        if self.video.active() {
            if let Err(error) = self
                .video
                .start_decode(&self.model.settings.music_directory, self.model.position_ms)
            {
                self.video_failed(error);
            }
            self.video.generation = Some(self.model.generation);
        }
    }

    pub(crate) fn video_request(&mut self, request: Request) -> Result<(), String> {
        match request {
            Request::Scan => self.video.scan(
                self.model.sources.clone(),
                self.model.settings.music_directory.clone(),
            )?,
            Request::Play(index) => self.play_video(index)?,
            Request::Stop => self.stop_video()?,
            Request::Restart => {
                self.pause();
                self.release_hold();
                self.model.position_ms = 0;
                self.video.suspended = false;
                self.load()?;
            }
        }
        self.dirty.mark_render();
        Ok(())
    }

    fn play_video(&mut self, index: usize) -> Result<(), String> {
        let entry = self
            .video
            .entries
            .get(index)
            .ok_or("video is no longer in the library")?;
        let info = entry.info.as_ref().map_err(Clone::clone)?.clone();
        let track = entry.track.clone();
        self.pause();
        self.release_hold();
        self.save_book();
        // The saved music session excludes the resident catalog and platform snapshot.
        let library = std::mem::take(&mut self.model.library);
        let platform = std::mem::take(&mut self.model.platform);
        self.video.music = Some(Box::new(self.model.clone()));
        self.model.library = library;
        self.model.platform = platform;
        let position = self.video.resume(&track);
        self.model.replace_queue(vec![track.clone()], 0)?;
        self.model.position_ms = position;
        self.video.last_position = position;
        self.video.current = Some(track.clone());
        self.video.suspended = false;
        self.ui.video.title = track.filename;
        self.ui.video.detail = format!("{} · {} × {}", info.codec, info.width, info.height);
        self.ui.video.error = None;
        self.ui.video.picture = None;
        self.ui.video.controls_at = Some(Instant::now());
        if let Err(error) = self
            .video
            .start_decode(&self.model.settings.music_directory, position)
        {
            self.stop_video()?;
            return Err(error);
        }
        if let Err(error) = self.load() {
            self.ui.video.error = Some(error.clone());
            self.stop_video()?;
            return Err(error);
        }
        self.video.generation = Some(self.model.generation);
        Ok(())
    }

    pub(crate) fn stop_video(&mut self) -> Result<(), String> {
        if !self.video.active() {
            return Ok(());
        }
        let save = self.video.save(self.model.position_ms);
        self.pause();
        self.release_hold();
        self.video.stop_decode();
        self.video.current = None;
        self.ui.close_video(&mut self.model);
        if let Some(music) = self.video.music.take() {
            self.model.queue = music.queue;
            self.model.queue_entry_ids = music.queue_entry_ids;
            self.model.queue_position = music.queue_position;
            self.model.queue_queued = music.queue_queued;
            self.model.queue_order = music.queue_order;
            self.model.queue_source = music.queue_source;
            self.model.queue_origin = music.queue_origin;
            self.model.history = music.history;
            self.model.position_ms = music.position_ms;
            self.model.book = music.book;
            self.model.playback = if self.model.current().is_some() {
                PlaybackState::Paused
            } else {
                PlaybackState::Stopped
            };
            self.model.invalidate();
        }
        self.checkpoint();
        self.dirty.mark_both();
        save
    }

    /// Checkpoints during a movie retain the paused music queue in session.json.
    pub(crate) fn video_checkpoint(&mut self) -> Option<std::io::Result<()>> {
        self.video
            .checkpoint(&self.model, &self.root.join("state/session.json"))
    }

    pub(crate) fn video_tick(&mut self) {
        if let Some(result) = self.video.scan_result() {
            self.ui.video.loading = false;
            self.ui.video.error = result.err();
            self.ui.video.rows = self
                .video
                .entries
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let detail = match &e.info {
                        Ok(v) => format!(
                            "{} · {} × {} · {}",
                            components_time(v.duration_ms),
                            v.width,
                            v.height,
                            v.codec
                        ),
                        Err(error) => error.clone(),
                    };
                    let mut item = reborn_ui::Item::new(&e.track.filename, format!("video:{i}"))
                        .with_secondary(detail);
                    item.enabled = e.info.is_ok();
                    item
                })
                .collect();
            self.dirty.mark_render();
        }
        if !self.video.active() {
            return;
        }
        if self.model.current().map(|t| &t.path) != self.video.current.as_ref().map(|t| &t.path) {
            let _ = self.video.save(self.video.last_position);
            self.video.stop_decode();
            self.video.current = None;
            self.video.music = None;
            self.ui.close_video(&mut self.model);
            self.dirty.mark_render();
            return;
        }
        self.video.last_position = self.model.position_ms;
        if self.model.screen != Screen::VideoPlaying {
            if let Err(error) = self.stop_video() {
                self.fail(LOG_SUBSYSTEM, error);
            }
            return;
        }
        if self.model.screen_off
            && (!self.video.suspended
                || matches!(
                    self.model.playback,
                    PlaybackState::Playing | PlaybackState::Buffering
                ))
        {
            self.pause();
            self.release_hold();
            self.video.stop_decode();
            self.video.suspended = true;
        }
        if self.seek_until.is_some() {
            self.video.stop_decode();
            self.dirty.mark_render();
            return;
        }
        if matches!(
            self.model.playback,
            PlaybackState::Error | PlaybackState::Stopped
        ) {
            self.video.stop_decode();
            if self.model.playback == PlaybackState::Error {
                self.ui.video.error = self.model.last_error.clone();
            }
            return;
        }
        let playing = self.model.playback == PlaybackState::Playing;
        if !self.model.screen_off
            && (playing || self.model.playback == PlaybackState::Buffering)
            && (self.video.generation != Some(self.model.generation) || self.video.suspended)
        {
            let result = self
                .video
                .start_decode(&self.model.settings.music_directory, self.model.position_ms);
            self.video.generation = Some(self.model.generation);
            self.video.suspended = false;
            if let Err(error) = result {
                self.video_failed(error);
            } else {
                self.ui.video.error = None;
            }
        }
        if !self.model.screen_off && (playing || self.ui.video.picture.is_none()) {
            match self.video.frame(self.model.position_ms) {
                Ok(Some(frame)) => {
                    if let Some(graphics) = &mut self.graphics {
                        if let Err(error) = graphics.video(&frame.rgba, frame.width, frame.height) {
                            self.video_failed(error);
                            return;
                        }
                    }
                    self.ui.video.picture = Some((frame.width, frame.height));
                    self.dirty.mark_render();
                }
                Ok(None) => {}
                Err(error) => self.video_failed(error),
            }
        }
        if playing {
            self.last_activity = Instant::now();
            self.dirty.mark_render();
        }
        if self.video.last_save.elapsed() > CHECKPOINT_EVERY {
            if let Err(error) = self.video.save(self.model.position_ms) {
                self.ui.video.error = Some(error);
            }
        }
    }

    fn video_failed(&mut self, error: String) {
        self.pause();
        self.release_hold();
        self.video.stop_decode();
        self.ui.video.error = Some(error.clone());
        self.ui.video.controls_at = Some(Instant::now());
        self.model.playback = PlaybackState::Error;
        self.fail(LOG_SUBSYSTEM, error);
    }
}

fn components_time(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
