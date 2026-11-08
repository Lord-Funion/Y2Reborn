//! Isolated video discovery and a single decoder worker with replaceable jobs and bounded output.
use reborn_core::{
    video::{self, FRAME_QUEUE, MAX_DEPTH, MAX_FILES},
    Source, Track,
};
use reborn_media::{Cancel, VideoDecoder, VideoFrame, VideoInfo};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, UNIX_EPOCH},
};

pub struct Entry {
    pub track: Track,
    pub info: Result<VideoInfo, String>,
}

pub fn discover(sources: &[Source], cancel: &Cancel) -> Vec<Track> {
    let mut out = Vec::new();
    let mut examined = 0;
    for source in sources.iter().filter(|s| s.online) {
        let mut dirs = vec![(source.root.clone(), 0)];
        while let Some((dir, depth)) = dirs.pop() {
            if cancel.is_cancelled() || out.len() >= MAX_FILES || examined >= MAX_FILES * 20 {
                return out;
            }
            let Ok(files) = fs::read_dir(&dir) else {
                continue;
            };
            for file in files.flatten() {
                examined += 1;
                if cancel.is_cancelled() || out.len() >= MAX_FILES || examined >= MAX_FILES * 20 {
                    return out;
                }
                let Ok(kind) = file.file_type() else {
                    continue;
                };
                if kind.is_dir() && depth < MAX_DEPTH && dirs.len() < MAX_FILES {
                    dirs.push((file.path(), depth + 1));
                } else if kind.is_file() && video::candidate(&file.path()) {
                    let Ok(meta) = file.metadata() else {
                        continue;
                    };
                    let filename = file.file_name().to_string_lossy().into_owned();
                    let modified = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |t| t.as_nanos().min(i64::MAX as u128) as i64);
                    out.push(Track {
                        id: -(out.len() as i64 + 1),
                        source_id: source.id.clone(),
                        path: file.path(),
                        filename: filename.clone(),
                        title: filename,
                        size: meta.len(),
                        mtime: modified,
                        online: true,
                        ..Track::default()
                    });
                }
            }
        }
    }
    out.sort_by_cached_key(|t| t.filename.to_lowercase());
    out
}

pub fn scan(sources: Vec<Source>, music: PathBuf, cancel: Cancel) -> Receiver<Vec<Entry>> {
    let (tx, rx) = sync_channel(1);
    let _ = thread::Builder::new()
        .name("video-scan".into())
        .spawn(move || {
            let mut entries = Vec::new();
            for mut track in discover(&sources, &cancel) {
                if cancel.is_cancelled() {
                    break;
                }
                let info = reborn_platform::storage::pin_track(&track, &music).and_then(|source| {
                    VideoDecoder::open(source.path(), cancel.clone()).map(|d| d.info.clone())
                });
                if let Ok(info) = &info {
                    track.duration_ms = info.duration_ms;
                    track.codec = info.codec.clone();
                }
                entries.push(Entry { track, info });
            }
            let _ = tx.send(entries);
        });
    rx
}

#[derive(Debug)]
pub enum Reply {
    Opened(VideoInfo),
    Frame(VideoFrame),
    Failed(String),
    End,
}

struct Job {
    track: Track,
    music: PathBuf,
    position_ms: u64,
    cancel: Cancel,
    tx: SyncSender<Reply>,
}

#[derive(Default)]
struct Inbox {
    job: Option<Job>,
    closed: bool,
    running: bool,
}

pub struct Worker {
    inbox: Arc<(Mutex<Inbox>, Condvar)>,
    cancel: Option<Cancel>,
}

impl Worker {
    pub fn new() -> Result<Self, String> {
        let inbox = Arc::new((Mutex::new(Inbox::default()), Condvar::new()));
        let thread_inbox = inbox.clone();
        thread::Builder::new()
            .name("video-decode".into())
            .spawn(move || {
                let worker = DecodeWorker {
                    inbox: thread_inbox,
                };
                worker.run();
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            inbox,
            cancel: None,
        })
    }

    pub fn start(
        &mut self,
        track: Track,
        music: &Path,
        position_ms: u64,
    ) -> Result<Receiver<Reply>, String> {
        self.stop();
        let cancel = Cancel::new()?;
        let (tx, rx) = sync_channel(FRAME_QUEUE);
        let job = Job {
            track,
            music: music.into(),
            position_ms,
            cancel: cancel.clone(),
            tx,
        };
        self.cancel = Some(cancel);
        self.inbox
            .0
            .lock()
            .map_err(|_| "video worker unavailable")?
            .job = Some(job);
        self.inbox.1.notify_one();
        Ok(rx)
    }

    pub fn stop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        if let Ok(mut inbox) = self.inbox.0.lock() {
            inbox.job = None;
        }
    }

    pub fn quiesce(&mut self) -> bool {
        self.stop();
        self.inbox.0.lock().is_ok_and(|state| !state.running)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
        if let Ok(mut inbox) = self.inbox.0.lock() {
            inbox.closed = true;
        }
        self.inbox.1.notify_one();
    }
}

struct DecodeWorker {
    inbox: Arc<(Mutex<Inbox>, Condvar)>,
}

impl DecodeWorker {
    fn run(self) {
        loop {
            let job = {
                let Ok(mut state) = self.inbox.0.lock() else {
                    return;
                };
                while state.job.is_none() && !state.closed {
                    let Ok(next) = self.inbox.1.wait(state) else {
                        return;
                    };
                    state = next;
                }
                if state.closed {
                    return;
                }
                let job = state.job.take();
                state.running = job.is_some();
                job
            };
            if let Some(job) = job {
                if let Err(error) = Self::play(&job) {
                    Self::send(&job, Reply::Failed(error));
                }
            }
            if let Ok(mut state) = self.inbox.0.lock() {
                state.running = false;
            }
        }
    }

    fn send(job: &Job, mut reply: Reply) -> bool {
        while !job.cancel.is_cancelled() {
            match job.tx.try_send(reply) {
                Ok(()) => return true,
                Err(TrySendError::Full(returned)) => {
                    reply = returned;
                    thread::sleep(Duration::from_millis(5));
                }
                Err(TrySendError::Disconnected(_)) => return false,
            }
        }
        false
    }

    fn play(job: &Job) -> Result<(), String> {
        let source = reborn_platform::storage::pin_track(&job.track, &job.music)?;
        if !source.matches_revision(job.track.size, job.track.mtime) {
            return Err("video changed; refresh the library".into());
        }
        let mut decoder = VideoDecoder::open(source.path(), job.cancel.clone())?;
        if !Self::send(job, Reply::Opened(decoder.info.clone())) {
            return Ok(());
        }
        if job.position_ms > 0 {
            decoder.seek(job.position_ms)?;
        }
        while !job.cancel.is_cancelled() {
            if !source.is_current() {
                return Err("video source was removed or changed".into());
            }
            match decoder.read()? {
                Some(frame) if frame.position_ms.saturating_add(150) < job.position_ms => {}
                Some(frame) => {
                    if !Self::send(job, Reply::Frame(frame)) {
                        return Ok(());
                    }
                }
                None => {
                    Self::send(job, Reply::End);
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
