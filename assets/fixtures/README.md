# Reborn audio fixtures

The generated fixtures are deterministic 440 Hz, ramp, artwork, metadata and
corruption inputs produced locally with FFmpeg 9.0.1 by
`tools/build/fixtures.py`. Their hashes are recorded in `manifest.json`.
`loud-24-96000-rg.flac` (2026-10-18, FFmpeg 9.0.2, `make_loud_flac`) is near full scale at 40 Hz and 8.8 kHz with
ReplayGain +6 dB and no peak tag: with a headphone profile only the limiter keeps it below full scale.

`ape-silence.ape` and `wavpack-silence.wv` are small decoder specimens because
the host FFmpeg build does not provide APE or WavPack encoders. They were
retrieved on 2026-09-18 from the Wavecor test-disc files
[`Track73.ape`](https://www.wavecor.co.uk/TestDisc/Track73.ape) and
[`Track73.wv`](https://www.wavecor.co.uk/TestDisc/Track73.wv). They remain
decoder test inputs; the generated-fixture CC0 statement does not apply to
these two files.

The 2026-10-04 [metadata fixtures](metadata/manifest.json) add Unicode/ID3/APE/
WAV/AIFF tags, malformed values and bounded-artwork cases. Regenerate their
metadata with `python3 tools/build/metadata_fixtures.py`; these fixture writers
are not production parsers. `tools/build/jpeg_fixture.c` uses direct host FFmpeg
libraries to generate the 4000×4000 JPEG. Generated metadata/images are CC0;
the underlying original media keeps the provenance above. The
[Platform 1.0 media receipt](../../docs/validation/REBORN-PLATFORM-V1-FFMPEG-METADATA.md)
records tests and reproducible benchmarks.
# Synthetic local-video fixtures

`video-h264-aac.mp4`, its Matroska remux `video-h264-aac.mkv`, and
`video-vp8-opus.webm` are generated FFmpeg test patterns with generated sine
tones. They contain no third-party video or music. Regenerate with:

```sh
ffmpeg -f lavfi -i testsrc2=size=320x180:rate=20:duration=2 \
  -f lavfi -i sine=frequency=440:sample_rate=44100:duration=2 \
  -c:v libx264 -preset veryfast -pix_fmt yuv420p -c:a aac -b:a 64k \
  -movflags +faststart -metadata title="Reborn synthetic video test" video-h264-aac.mp4
ffmpeg -i video-h264-aac.mp4 -c copy video-h264-aac.mkv
ffmpeg -f lavfi -i testsrc2=size=160x90:rate=10:duration=1 \
  -f lavfi -i sine=sample_rate=48000:duration=1 -c:v libvpx -b:v 100k \
  -c:a libopus -b:a 32k video-vp8-opus.webm
```

The fixtures test actual frame decoding, timestamps, seeking and cancellation,
and supply the picture in video UI previews. They are not device performance
evidence. Host CLI encoders used to generate fixtures are not target dependencies.
