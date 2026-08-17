# Media, audio and text

Three crates that touch the outside world: files, sound cards, fonts.

## lumit-media: decode and encode

Wraps FFmpeg through rsmpeg 0.18. Note that the architecture doc's `MediaSource`
trait does not exist yet in code. The seam is the concrete `VideoDecoder` struct plus
two injected closures. `load_or_build_with` makes the index-cache decision, and tests
can drive it without FFmpeg. `pick_first_working` is the encoder ladder, and tests
can drive it without hardware.

| File | Owns |
|---|---|
| `src/probe.rs` | Read-only probe: duration, container, video/audio info |
| `src/index.rs` | `FrameIndex`: packet scan mapping frame ↔ pts ↔ keyframe. Bincode sidecar cache |
| `src/decode.rs` | `VideoDecoder`: exact-frame seek, D3D11VA hardware decode, RGBA conversion |
| `src/audio.rs` | Whole-file audio decode to interleaved stereo f32 |
| `src/encode.rs` | mp4 encoder ladder (NVENC → AMF → QSV → software) and image sequences |
| `src/slate.rs` | Generated colour bars for missing footage |

**The frame index** is a packet scan — no decoding. It records pts and keyframe
flags, and detects variable frame rate. It caches to a `.kidx` sidecar keyed by a
blake3 fingerprint (size + mtime + head/tail hash). Stale, corrupt or missing all
mean "rebuild", never an error.

**Seeking** is the performance-critical decision (inside `VideoDecoder::frame_rgba`): seek to the
nearest keyframe ≤ N **only when it saves work**. Playing forward with drops never
seeks. A seek per request collapses playback about 20×. A regression test pins at
most one seek per keyframe crossed.

**Hardware decode** (Windows, D3D11VA) transfers frames back and repacks NV12 →
yuv420p. Hardware and software then share one RGBA conversion path. Skipping the
repack makes preview differ from export by up to 161/255 per byte.

Every rsmpeg pointer touch is a small named helper with a SAFETY comment. The crate
spawns no threads: callers own decoders on their own worker threads.

## lumit-audio: Pulsar, and why it is the clock

The number of frames the cpal callback consumed **is** the playback clock. Video
polls `clock_seconds()` and chases it. Audio never waits for pixels, so A/V cannot
drift.

The callback (`fill` in `src/lib.rs`) obeys real-time rules: no allocation, no waiting.
It calls `plan.try_read()`. If an edit holds the lock, it plays one silent buffer
rather than block.

What crosses to the callback is an `Arc<MixPlan>` — placed clips over shared audio
buffers plus an optional gain envelope. `swap_plan` replaces it mid-play without
touching the clock or play state. Solo, mute and move are therefore audible on the
next callback (~10 ms) with no re-bake. Export uses the *baked* mixer (`mix_stereo`).
Playback uses the live plan. The two match sample for sample.

Two more pieces:

- `peaks.rs` — a three-tier min/max/rms pyramid × 4 frequency bands, built in one
  pass, for waveform drawing at any zoom.
- `beat.rs` — spectral-flux onset detection: Hann STFT → log-magnitude positive flux
  → adaptive peak picking → autocorrelation BPM preferring 70–180. Fully
  deterministic, so results cache by fingerprint.

## lumit-project: the .lum file

A `.lum` is a deflated zip: `manifest.json` **first**, then `project.json`. Saves are
byte-deterministic (sorted maps). This matters because the frame cache keys on those
bytes.

- **Save** is atomic: temp file in the same directory, `sync_all`, rename.
- **Open** gates on `min_reader` (a newer file returns `TooNew`), then walks
  `MIGRATIONS` over raw JSON *before* typing. One migration exists today: 0.1.0 →
  0.2.0, segment Retime → property Retime (K-249).
- **Journal**: every committed `Op` appends one JSONL line with `sync_data`.
  Recovery replays last save + journal. It tolerates a torn final line (crash
  mid-append), but it stops at a malformed line mid-file.
- **Autosaves** rotate `<stem>.autosave-1..N.lum`, 1 = newest.
- **Relink**: saved files carry relative paths plus fingerprints (K-173). Resolution
  order is relative → legacy absolute → fingerprint search → `Missing`. Missing
  never blocks opening.

## lumit-keymap

A plain `Vec<Binding { context, chord, action }>` plus a list of deliberate unbinds
(K-302). `lookup` tries the exact context, then `Global`.

Clash rules: two bindings on one chord in the **same** context are a `Conflict`. A
panel binding over a Global one is a `Shadow`. Precedence resolves a `Shadow`, and it
is reported rather than flagged (K-281). Every `ActionId` needs a `description()`.
Each description needs both an `engine_labels.dart` entry and an `app_en.arb` key.

## lumit-text

v1 only: `rasterise_line` over embedded Inter Regular via fontdue. Measure pass, blit
pass, advance-based layout. No shaping, no kerning, no styles.

## Traps

- Never allocate, lock or block in the audio callback. `AudioEngine` is not `Send`
  (the cpal stream). Only `ClockHandle` crosses threads.
- A `VideoDecoder` is stateful and belongs to exactly one owner. Never share one
  across threads.
- Obtain a `FrameIndex` only through `load_or_build_index`. Fingerprint fields are
  public and deserialised — treat them as untrusted.
- `manifest.json` must stay the first zip entry. Unknown fields survive via `extra`
  maps. `absolute_path` never serialises.
- A schema bump appends a `Migration` and a superseding decision entry. Never edit
  migration history.
