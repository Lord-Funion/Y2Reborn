//! Built-in video feature boundary. No plugin ABI exists upstream yet; decoding and discovery
//! live behind this controller, reusing the music transport and its audible ALSA clock.
use crate::*;
use reborn_core::video::{Request, Resume, Timing};
use reborn_media::{Cancel, VideoFrame};
use std::sync::mpsc::TryRecvError;

#[cfg(test)]
mod controller_tests;
mod runtime;
mod worker;
use worker::{Entry, Reply, Worker};

const LOG_SUBSYSTEM: &str = "media";

pub struct Controller {
    entries: Vec<Entry>,
    scan: Option<(Cancel, Receiver<Vec<Entry>>)>,
    worker: Option<Worker>,
    replies: Option<Receiver<Reply>>,
    pending: Option<VideoFrame>,
    pub(super) music: Option<Box<AppModel>>,
    current: Option<Track>,
    generation: Option<u64>,
    suspended: bool,
    resume_path: PathBuf,
    resumes: Vec<Resume>,
    last_save: Instant,
    last_position: u64,
}

impl Controller {
    pub fn new(resume_path: PathBuf) -> Self {
        let resumes = fs::metadata(&resume_path)
            .ok()
            .filter(|m| m.len() <= 256 * 1024)
            .and_then(|_| fs::read(&resume_path).ok())
            .and_then(|b| serde_json::from_slice::<Vec<Resume>>(&b).ok())
            .unwrap_or_default()
            .into_iter()
            .take(128)
            .collect();
        Self {
            entries: Vec::new(),
            scan: None,
            worker: None,
            replies: None,
            pending: None,
            music: None,
            current: None,
            generation: None,
            suspended: false,
            resume_path,
            resumes,
            last_save: Instant::now(),
            last_position: 0,
        }
    }

    pub fn active(&self) -> bool {
        self.current.is_some()
    }

    /// A movie ends once; music's Repeat All must not pre-open a second audio pass.
    pub(super) fn upcoming(&self, model: &AppModel) -> (Vec<Track>, Vec<QueueEntryId>) {
        if self.active() {
            (Vec::new(), Vec::new())
        } else {
            model.upcoming(UPCOMING)
        }
    }

    pub(super) fn gapless(&self, model: &AppModel) -> bool {
        !self.active() && model.settings.gapless_enabled
    }

    fn checkpoint(&mut self, model: &AppModel, session: &Path) -> Option<std::io::Result<()>> {
        let music = self.music.as_mut()?;
        music.settings = model.settings.clone();
        music.output = model.output.clone();
        let session = music.checkpoint(session);
        if let Err(error) = self.save(model.position_ms) {
            return Some(Err(std::io::Error::other(error)));
        }
        Some(session)
    }

    fn save(&mut self, position_ms: u64) -> Result<(), String> {
        let Some(track) = &self.current else {
            return Ok(());
        };
        let path = track.path.to_string_lossy().into_owned();
        if path.len() > 1024 {
            return Err("video filename is too long to save a resume point".into());
        }
        self.resumes.retain(|r| r.path != path);
        if self.resumes.len() >= 128 {
            self.resumes.remove(0);
        }
        self.resumes.push(Resume {
            path,
            bytes: track.size,
            modified: track.mtime as u64,
            position_ms,
        });
        let bytes = serde_json::to_vec(&self.resumes).map_err(|e| e.to_string())?;
        reborn_core::atomic_write(&self.resume_path, &bytes).map_err(|e| e.to_string())?;
        self.last_save = Instant::now();
        Ok(())
    }

    fn resume(&self, track: &Track) -> u64 {
        self.resumes
            .iter()
            .find(|r| Path::new(&r.path) == track.path)
            .map_or(0, |r| {
                r.position(track.size, track.mtime as u64, track.duration_ms)
            })
    }

    fn start_decode(&mut self, music: &Path, position: u64) -> Result<(), String> {
        let track = self.current.clone().ok_or("no video selected")?;
        if self.worker.is_none() {
            self.worker = Some(Worker::new()?);
        }
        if let Some(worker) = &mut self.worker {
            self.replies = Some(worker.start(track, music, position)?);
        }
        self.pending = None;
        Ok(())
    }

    fn stop_decode(&mut self) {
        if let Some(worker) = &mut self.worker {
            worker.stop();
        }
        self.replies = None;
        self.pending = None;
        self.generation = None;
    }

    fn scan(&mut self, sources: Vec<Source>, music: PathBuf) -> Result<(), String> {
        if self.scan.is_some() {
            return Ok(());
        }
        let cancel = Cancel::new()?;
        let rx = worker::scan(sources, music, cancel.clone());
        self.scan = Some((cancel, rx));
        Ok(())
    }

    fn scan_result(&mut self) -> Option<Result<(), String>> {
        let (_, rx) = self.scan.as_ref()?;
        match rx.try_recv() {
            Ok(entries) => {
                self.entries = entries;
                self.scan = None;
                Some(Ok(()))
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.scan = None;
                Some(Err("video scan worker stopped".into()))
            }
        }
    }

    fn quiesce(&mut self) -> bool {
        self.stop_decode();
        let scan_ready = self.scan.as_ref().is_none_or(|(cancel, rx)| {
            cancel.cancel();
            !matches!(rx.try_recv(), Err(TryRecvError::Empty))
        });
        if scan_ready {
            self.scan = None;
        }
        let worker_ready = self.worker.as_mut().is_none_or(Worker::quiesce);
        scan_ready && worker_ready
    }

    fn frame(&mut self, clock: u64) -> Result<Option<VideoFrame>, String> {
        let mut shown = None;
        // Drain at most the bounded channel and one pending picture per UI pass.
        for _ in 0..reborn_core::video::FRAME_QUEUE + 1 {
            if let Some(frame) = self.pending.take() {
                match reborn_core::video::timing(frame.position_ms, clock) {
                    Timing::Wait => {
                        self.pending = Some(frame);
                        break;
                    }
                    Timing::Show => shown = Some(frame),
                    Timing::Drop => {}
                }
            }
            let Some(rx) = &self.replies else {
                break;
            };
            match rx.try_recv() {
                Ok(Reply::Frame(frame)) => self.pending = Some(frame),
                Ok(Reply::Opened(info)) => {
                    if let Some(current) = &mut self.current {
                        current.duration_ms = info.duration_ms;
                    }
                }
                Ok(Reply::Failed(error)) => return Err(error),
                Ok(Reply::End) | Err(TryRecvError::Disconnected) => {
                    self.replies = None;
                    break;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        Ok(shown)
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        if let Some((cancel, _)) = &self.scan {
            cancel.cancel();
        }
        self.stop_decode();
    }
}
