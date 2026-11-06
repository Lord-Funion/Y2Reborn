//! A Bluetooth stream never runs dry (F69, Y2Linux #96, Y2Reborn #99): skips, seeks, the end of a
//! song and a pause leave the sink open and fed, on a sink that plays at real time as BlueALSA's
//! does and counts every underrun (the stop and restart inside BlueALSA after which AirPods stay
//! silent).
use super::*;
use reborn_audio::Parameters;
use reborn_core::{AudioOutput, PcmFormat};

const RATE: u32 = 48_000;
const RING: u64 = 8192;
const START_THRESHOLD: u64 = 2048;
/// S32LE stereo.
const FRAME: usize = 8;

#[derive(Default)]
struct Clock {
    queued: f64,
    since: Option<Instant>,
    running: bool,
    underruns: u64,
    played: f64,
    opens: u64,
    open_now: u64,
    discards: u64,
    device_pauses: u64,
}
impl Clock {
    /// Play what the time since the last look took out of the ring.
    fn advance(&mut self) {
        let now = Instant::now();
        if let (true, Some(since)) = (self.running, self.since) {
            let due = now.duration_since(since).as_secs_f64() * f64::from(RATE);
            if due > self.queued {
                self.played += self.queued;
                self.queued = 0.0;
                self.running = false;
                self.underruns += 1;
            } else {
                self.queued -= due;
                self.played += due;
            }
        }
        self.since = Some(now);
    }
}
struct RealTimeSink(Arc<Mutex<Clock>>);
impl AudioSink for RealTimeSink {
    fn parameters(&self) -> Parameters {
        Parameters {
            rate: RATE,
            period: 1024,
            buffer: RING as u32,
            format: PcmFormat::S32LE,
            channels: 2,
            device: "bluealsa:test".into(),
            ..Default::default()
        }
    }
    fn write(&mut self, pcm: &[u8]) -> Result<usize, String> {
        let mut clock = self.0.lock().unwrap();
        clock.advance();
        let room = RING.saturating_sub(clock.queued as u64);
        let n = (pcm.len() / FRAME).min(room as usize);
        if n == 0 {
            drop(clock);
            thread::sleep(Duration::from_millis(5));
            return Ok(0);
        }
        clock.queued += n as f64;
        if !clock.running && clock.queued as u64 >= START_THRESHOLD {
            clock.running = true;
        }
        Ok(n)
    }
    fn discard(&mut self) -> Result<(), String> {
        let mut clock = self.0.lock().unwrap();
        clock.discards += 1;
        clock.queued = 0.0;
        clock.running = false;
        Ok(())
    }
    fn pause(&mut self, on: bool) -> Result<(), String> {
        if on {
            self.0.lock().unwrap().device_pauses += 1;
        }
        Ok(())
    }
    fn delay(&self) -> u64 {
        let mut clock = self.0.lock().unwrap();
        clock.advance();
        clock.queued as u64
    }
    fn queued(&self) -> Option<u64> {
        let mut clock = self.0.lock().unwrap();
        clock.advance();
        Some(clock.queued as u64)
    }
}
impl Drop for RealTimeSink {
    fn drop(&mut self) {
        self.0.lock().unwrap().open_now -= 1;
    }
}

fn bluetooth() -> SinkSpec {
    SinkSpec {
        output: AudioOutput::Bluetooth("12:34:56:78:90:AB".into()),
        rate: RATE,
        format: PcmFormat::S32LE,
        physical_bits: 32,
        valid_bits: 32,
        channels: 2,
        layout: "stereo".into(),
        device: "bluealsa:test".into(),
        codec: None,
        transport_object: None,
        transport_device: None,
        transport: None,
        mode: None,
        transport_generation: 0,
        fallback: false,
        fallback_reason: String::new(),
        speaker: false,
    }
}
fn fixture(name: &str) -> Track {
    Track {
        source_id: "internal".into(),
        path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fixtures")
            .join(name),
        ..Default::default()
    }
}
fn playback(clock: &Arc<Mutex<Clock>>) -> Playback {
    let root = std::env::temp_dir().join(format!(
        "reborn-continuity-{}-{:?}",
        std::process::id(),
        thread::current().id()
    ));
    let clock = clock.clone();
    Playback::with_sink(
        Observer::new(&root).unwrap(),
        root,
        Box::new(move |_, _, _| {
            let mut c = clock.lock().unwrap();
            c.opens += 1;
            c.open_now += 1;
            Ok(Box::new(RealTimeSink(clock.clone())) as Box<dyn AudioSink>)
        }),
    )
    .unwrap()
}
/// A start as the runtime makes it: the playing stream switched out, the next one opened.
fn start(p: &mut Playback, generation: u64, track: Track, position: u64) {
    p.switch_and_wait(generation).unwrap();
    p.start_with_gapless(LoadRequest {
        music_root: track.path.parent().unwrap().into(),
        track,
        entry_id: QueueEntryId(generation),
        queue: vec![],
        queue_entry_ids: vec![],
        position,
        spec: bluetooth(),
        generation,
        dsp: DspConfig::with_volume(100),
        gapless_enabled: true,
        id: generation,
        fade_in: false,
    })
    .unwrap();
    wait_for(p, generation, |e| matches!(e, Event::TrackStarted { .. }));
}
fn wait_for(p: &Playback, generation: u64, wanted: impl Fn(&Event) -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(PlaybackEvent::Core(event)) = p.events.recv_timeout(Duration::from_millis(50)) {
            let g = match &event {
                Event::TrackStarted { generation } | Event::TrackEnded { generation } => {
                    *generation
                }
                _ => 0,
            };
            if g == generation && wanted(&event) {
                return;
            }
        }
        assert!(
            Instant::now() < until,
            "stream {generation} never got there"
        );
    }
}
fn checked(clock: &Arc<Mutex<Clock>>, what: &str) {
    let c = clock.lock().unwrap();
    assert_eq!(c.underruns, 0, "{what}: the Bluetooth stream ran dry");
    assert_eq!(c.opens, 1, "{what}: one open sink for the whole session");
    assert_eq!(c.discards, 0, "{what}: queued audio is never dropped");
    assert_eq!(c.device_pauses, 0, "{what}: BlueALSA is never paused");
    assert!(c.played > 0.0, "{what}: nothing played");
}

#[test]
fn quick_skips_and_slow_song_starts_never_run_the_stream_dry() {
    let clock = Arc::new(Mutex::new(Clock::default()));
    let mut p = playback(&clock);
    start(&mut p, 1, fixture("book-chapters.mp3"), 0);
    for generation in 2..12 {
        thread::sleep(Duration::from_millis(if generation % 2 == 0 {
            60
        } else {
            400
        }));
        // The next song's decoder takes its time to open (a slow card, a big file).
        p.switch_and_wait(generation).unwrap();
        thread::sleep(Duration::from_millis(250));
        start(&mut p, generation, fixture("book-chapters.mp3"), 0);
    }
    thread::sleep(Duration::from_millis(300));
    checked(&clock, "skips");
}

#[test]
fn a_seek_holds_with_silence_and_restarts_in_the_open_stream() {
    let clock = Arc::new(Mutex::new(Clock::default()));
    let mut p = playback(&clock);
    start(&mut p, 1, fixture("book-chapters.mp3"), 0);
    thread::sleep(Duration::from_millis(300));
    for (generation, position) in [(2, 3_000), (3, 1_000), (4, 0), (5, 4_500)] {
        // `seek_by` holds while the wheel turns, then the stream restarts at the target.
        p.hold(true);
        thread::sleep(Duration::from_millis(500));
        start(&mut p, generation, fixture("book-chapters.mp3"), position);
        thread::sleep(Duration::from_millis(200));
    }
    checked(&clock, "seeks");
}

#[test]
fn the_end_of_a_song_leaves_the_stream_running_for_the_next_one() {
    let clock = Arc::new(Mutex::new(Clock::default()));
    let mut p = playback(&clock);
    for generation in 1..4 {
        start(&mut p, generation, fixture("tone.flac"), 0);
        wait_for(&p, generation, |e| matches!(e, Event::TrackEnded { .. }));
        // The runtime takes a moment to start the next song (gapless off, Repeat One).
        thread::sleep(Duration::from_millis(300));
    }
    checked(&clock, "song ends");
    // The end of the queue: the kept sink closes on its own, the stream stops then.
    p.switch_and_wait(10).unwrap();
    let until = Instant::now() + KEEP_SINK_FOR + Duration::from_secs(1);
    while clock.lock().unwrap().open_now != 0 {
        assert!(Instant::now() < until, "the kept sink stayed open");
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(clock.lock().unwrap().underruns, 0);
}

#[test]
fn a_pause_keeps_the_stream_started_with_silence_and_play_continues_in_it() {
    let clock = Arc::new(Mutex::new(Clock::default()));
    let mut p = playback(&clock);
    start(&mut p, 1, fixture("book-chapters.mp3"), 0);
    thread::sleep(Duration::from_millis(300));
    p.hold(true);
    thread::sleep(Duration::from_millis(1500));
    let played = clock.lock().unwrap().played;
    p.hold(false);
    thread::sleep(Duration::from_millis(500));
    assert!(clock.lock().unwrap().played > played, "the stream went on");
    checked(&clock, "pause");
}
