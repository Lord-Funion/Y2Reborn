# Videos

Open **Home → Videos**. Refresh Videos searches the same available Internal Storage and SD-card
sources as music, on a background worker. Put videos anywhere under the internal music folder
(`/media/internal/music`, or `/data/music` on the platform's fallback storage) or under `/media/sd`.
A `Videos` subfolder keeps them together. The music database and its scanner are unchanged.

A refresh examines at most 200,000 directory entries, descends at most 32 levels and keeps at
most 10,000 video candidates. Files beyond these discovery limits are omitted from that refresh.

This contribution is a source candidate, with no physical Y2 video qualification yet. The target
needs the paired Y2Linux FFmpeg component change described in
[Y2Linux's video dependencies](https://github.com/SchulzCode/Y2Linux/blob/main/docs/architecture/reborn-video.md).
An older image's audio-only FFmpeg package cannot supply H.264 video decoding. Rebuild the player
and `libreborn_media.so` together using the supported Y2Linux Buildroot integration.

## Files and controls

The initial target formats are MP4/M4V with H.264 and AAC, Matroska with H.264 and an available audio
codec, and WebM with VP8/Opus. The browser uses extensions to find candidates, then FFmpeg validates
their containers and streams. Unsupported or invalid files stay disabled with the probe's reason.
HEVC, VP9 and AV1 are not added to the target package. Silent video, subtitles, alternate audio-track
selection, playlists of videos and rotation metadata are outside this first implementation.

Source pictures with more than 414,720 pixels (the area of 720×576) are refused before opening
the video decoder. This is a memory
admission ceiling, **not** a resolution or frame-rate performance promise. Lower-resolution files
reduce CPU conversion and decoding work. No decode speed or battery claim is made for the Y2.
Display scaling respects the sample aspect ratio and fits within 480×360, with letterboxing.

| Control while watching | Action |
| --- | --- |
| Centre or Play/Pause | Play/pause |
| Wheel | Seek backward/forward |
| Hold Previous/Next | Repeated seek steps through the existing transport |
| Previous or hold Centre | Restart from the beginning |
| Next | Seek forward |
| Volume buttons | Change volume through the existing output |
| Back | Stop and return to Videos |
| Power short / screen sleep | Pause video; wake leaves it paused |

Seeking while paused chooses where playback resumes; its still picture changes when playback starts
again. Playing controls disappear after three seconds without input. Paused, buffering, stopped and error
states keep them visible. Videos play once, without inheriting music's repeat or gapless handoffs;
the music preferences stay unchanged. Playback reuses the selected wired or Bluetooth output, its delay-aware
position reports, output-loss handling and volume controls. A video does not scrobble. Leaving it
restores the previous music queue, paused at its saved position, including audiobook state.

Resume positions live separately in `state/video-resume.json`, bounded to the 128 most recently
saved files. File size and modification time must still match. A position within five seconds of
the end restarts the file. Writes use the existing atomic, flushed state-file helper. Checkpoints, including deep-sleep preparation,
while watching preserve the music session rather than putting the video into the music queue on
disk. Resume errors are visible and do not silently discard the older session.

## Implementation boundary

Upstream `main` has no plugin host or official video extension interface. In issue #41 the
maintainer deferred video until that interface exists. This candidate does not invent a plugin ABI:
`app/reborn/src/video` owns discovery, a replaceable decoder job, presentation and resume state.
The maintainer should review whether to admit this built-in boundary or move it behind the eventual
plugin host. The audio engine, platform contract and renderer remain the existing owners.

The video worker opens descriptor-pinned local files through the existing storage guard. The media
membrane shares local I/O cancellation and probe budgets, restricts video containers, uses two codec
threads and `max_pixels`, and holds one FFmpeg packet and decoded frame. Converted RGBA pictures are
at most 480×360. The output channel holds two messages; the UI may hold one future picture and one being uploaded,
and the worker may hold one picture in conversion or waiting to send. The UI waits for future timestamps and drops pictures more
than 150 ms late against the audible audio clock. Pausing or audio buffering cannot run pictures
ahead. Seeking replaces the pending job and its channel, so old pictures cannot cross a seek. Sleep
preparation cancels discovery and decoding and waits for their storage guards to be released.

Presentation retains the existing calm UI budget of 34 ms between render starts. That scheduling
budget is not a measured Y2 frame rate.

No whole video is read into memory. The GLES renderer reuses the same texture storage while picture
dimensions stay unchanged, updating it with `glTexSubImage2D`. Scaling uses existing `libswscale`;
no unqualified hardware decode or YUV shader is advertised. The memory caps cannot guarantee
allocation success on an exhausted device; recoverable allocation/decode failures stop playback
with a reason. Device-level out-of-memory termination still needs system qualification.

## Skins and validation

Videos uses the ordinary `page`/`list` fallback for browsing, and native controls through the same
skin palette and typography path as other fallback screens. A skin may supply `videos` and
`video_playing` layouts. All sixteen bundled skins specify optional native font roles and cover
shape; format-1 third-party packages without these fields retain their previous behavior. See
[skins](skins.md). Video pixels are never palette-remapped.

Run the normal offline gates with Rust 1.99.0 and host development libraries:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
../Y2Linux/tools/development/platform-gate.sh
```

The synthetic fixtures contain FFmpeg test patterns and generated tones. They exercise changing
H.264 pictures, ordered timestamps, matching audio duration, seek after end-of-file, cancellation,
corrupt input, descriptor cleanup, MP4/MKV/WebM admission and ordinary audio decoding. Discovery,
resume arithmetic, input behavior and skin switching have separate tests. These tests establish
host behavior; they do not establish real-time Y2 audio/video synchronization, supported frame
rates, Bluetooth latency, sleep/wake reliability or sustained device memory use.

The [host validation report](validation/video-host-validation.md) records the actual checks,
earlier failures, decode measurements, rendered previews and remaining qualification.
