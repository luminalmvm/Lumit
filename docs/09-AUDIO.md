# Audio

**Status: implementation-ready for v1; the Composer is design intent.** Implements K-050:
v1 audio is a **sync toolkit** — everything a montage editor needs to cut to music — and
the **Composer** workspace comes later. Terminology per [01-GLOSSARY.md](01-GLOSSARY.md);
playback architecture per [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) and K-013/K-017;
panel layout per [07-UI-SPEC.md](07-UI-SPEC.md).

---

## 1. Scope of v1

In: import, sample-accurate playback, timeline waveforms, manual and automatic beat
markers, beat snapping, volume keyframes, mute/solo, multiple audio layers per comp, audio
from video footage, audio scrubbing, audio in export.

Out (explicitly, §7): audio effects, a mixing console, and audio retiming.

## 2. Import and decode

- Lumit MUST import any ffmpeg-decodable audio (via rsmpeg, K-013): mp3, AAC/m4a, wav,
  flac, ogg/opus, and the audio streams of any importable video container.
- Audio items decode to **fp32 interleaved PCM at the engine's session sample rate**
  (default 48 kHz; resampled on decode via soxr-quality resampling). Source sample rate,
  channel count, and duration are stored as interpretation metadata on the asset.
- Decoded audio for layers near the playhead is held in a RAM ring; full decode is lazy.
  A whole-file decode pass runs once at import to build peak files (§4) and beat analysis
  (§5), in the background, cancellable, per K-017 (never on the UI thread).

## 3. Playback and sync

### 3.1 Audio clock is the playback master

Output runs through **cpal** into the OS device (WASAPI on Windows). During preview the
**audio clock is the master**: the video system schedules frames against the audio
device's sample position, not a wall-clock timer. The audio callback MUST be real-time
safe: no locks shared with the UI or render threads, no allocation; it reads from a
pre-mixed ring buffer filled by a dedicated audio thread (K-017).

Mixing model in v1: per-layer gain (volume keyframes, §6) → sum of all audible layers →
master limiter (a hard safety clip at −0.3 dBFS; not user-adjustable in v1 — v1 clamps
sample peaks to that ceiling, `lumit-audio::mix::MASTER_CEILING`; true inter-sample-peak
limiting per ITU-R BS.1770 is future) →
device. Sample-accurate means: layer in points, edit points, and volume keyframes are
resolved to exact sample positions, not frame-quantised.

### 3.2 Drift correction

Consumer audio devices do not run at exactly their nominal rate. Strategy:

1. Video chases audio: each displayed frame is chosen from the audio clock's current
   comp time. Video can never drift — it has no independent clock.
2. If preview rendering falls behind, frames are skipped to stay on the audio clock
   (latest-wins, K-017); audio MUST NOT pause or stutter to wait for video.
3. On device sample-rate mismatch or device change (headphones unplugged), the engine
   rebuilds the stream and resumes from the playhead; a device glitch MUST NOT desync — the
   audio clock restarts as master and video re-chases.

### 3.3 Frame-rate mismatches

Audio is not framed, so comp frame rate does not quantise audio. Rules:

- A comp's audio timeline is continuous seconds; audio layer in/out points MAY sit
  between video frames (they snap to frames by default; Alt-drag for free placement).
- Footage with a frame rate override (interpretation) keeps its audio at real-time rate —
  the override changes which video frame shows at a time, never audio pitch. If the
  override desyncs video from its own audio, the Timeline shows a desync badge on the
  layer and offers "restore native rate".
- Nested comps contribute their mixed audio at the parent's sample timeline directly;
  there is no per-comp resample.

### 3.4 Audio scrubbing

Dragging the playhead plays the audio under it (a short windowed grain at each new
position, pitch-native). Toggleable in the Timeline; on by default. Scrub audio uses the
same decoded ring, so it is warm wherever the cache bar is warm.

## 4. Waveforms

- At import, the background pass writes a **peak file**: min/max/RMS per block at multiple
  zoom tiers (samples-per-block 256 / 4 096 / 65 536), stored in the project's sidecar
  folder (K-040), keyed by content hash, rebuilt silently if missing or stale.
- The Timeline draws waveforms from peak files only — never from raw decode — so waveform
  rendering is O(pixels) at any zoom. Rendering follows
  [15-DESIGN.md](15-DESIGN.md): filled min/max body with RMS core, no per-sample spikes.
- Waveforms appear: on Audio layers (always), on Footage layers with audio (expandable
  lane), and **inside Sequence layer clips** — each clip draws the waveform of its own
  source range, so a cut's audio content is visible exactly where the clip sits. Clip
  waveforms account for the clip's trim; they are the primary visual for beat-checking an
  edit.

## 5. Markers and beat detection

- **Manual markers**: comp and layer markers per [01-GLOSSARY.md](01-GLOSSARY.md) §3,
  placed at the playhead (keyboard: `M`), draggable, labelled.
- **Beat markers**: generated by onset analysis of a chosen audio layer (or asset). v1
  algorithm: **spectral flux** onset detection — STFT (2048/512 hop at 48 kHz), per-band
  positive flux, adaptive median threshold, peak-pick — which is robust on the scene's
  material (EDM/phonk/trap with hard transients). Controls:
  - **Sensitivity** (0–100): scales the adaptive threshold; live re-run is near-instant
    because the STFT is cached from the import pass.
  - **BPM-grid assist**: estimates tempo (autocorrelation of the onset envelope), lets the
    user confirm or type a BPM and phase, then snaps detected onsets to the grid and fills
    grid beats where detection missed one. The grid is assistive; markers remain
    individually editable.
  - **Tap tempo**: tap a key in time with playback to seed the BPM estimate.
- Beat markers are ordinary markers with a `beat` label: deletable, draggable, and stored
  in the project file. Re-running detection offers replace or merge.
- **Snapping**: when snap is enabled, edit points, layer in/out points, keyframes, the
  work area, and marker-trigger effects ([08-EFFECTS.md](08-EFFECTS.md) §1.4) snap to beat
  markers during drags, with the standard snap affordance from
  [07-UI-SPEC.md](07-UI-SPEC.md). Beat markers are how "cut on the kick, flash on the
  snare" becomes drag-and-release.

## 6. Layers, volume, and control

- **Multiple audio layers per comp**; audio layers mix per §3.1. There is no layer-count
  audio limit beyond CPU.
- **Volume** is an animatable property per audio-capable layer (dB scale, −∞..+50 dB,
  default 0 dB; the owner raised the ceiling from the original +12 — K-172), keyframable
  and expression-visible like any property. −100 dB is the −∞ knee: at or below it the
  gain is exactly zero (the UI reads "−inf"), never a denormal whisper. Fades are volume
  keyframes; the fade-in/fade-out commands that write eased keyframe pairs are still to
  come. **Shipped (K-172):** `Layer.volume_db` + `Op::SetLayerVolume`; an animated volume
  bakes to a ~10 ms control-rate gain envelope applied identically by the live mix plan
  and the baked mixdown (playback == export, pinned by test); it lives in the layer's
  **Audio** group in the timeline outline, beside a **Waveform** twirl that draws that
  layer's own peaks in its lane (replacing the comp-wide strip — the per-layer lane
  follows a dragged bar in realtime, where the strip only refreshed on re-mix).
- **Mute / solo** via the audible and solo switches ([01-GLOSSARY.md](01-GLOSSARY.md) §2).
  Solo on any layer silences non-soloed audio, matching video solo semantics.
- **Audio from video footage**: a Footage layer with audio exposes its audio as part of
  the same layer (audible switch, volume property, waveform lane). "Detach audio" creates
  a linked Audio layer sharing the source so music-video workflows can keyframe them
  independently; the link is a grouping convenience, not a constraint — either side can be
  trimmed or moved alone after detaching.
- Stereo is the v1 channel model; mono sources upmix centred. Pan is not in v1 (see §7).

## 7. Out of scope for v1

- **Audio effects** (EQ, reverb, compression) — none in v1. The effect stack accepts no
  audio effects until the Composer phase; the [12-PLUGINS.md](12-PLUGINS.md) KFX surface
  reserves an audio-effect extension so the ABI does not need breaking later.
- **Mixing console** — no mixer panel; per-layer volume plus master limiter only.
- **Audio retiming.** Retime is video-only in v1: a retimed Footage layer's own audio is
  muted with a badge whenever its retime map differs from identity ("Retime mutes audio in
  this version"), because unpitched audio warping sounds bad and pitch-preserving
  stretching is real work. Roadmap: a later release adds pitch-preserving audio retime
  (phase-vocoder or WSOLA class) as a per-layer opt-in following the same retime map
  ([04-RETIMING.md](04-RETIMING.md)); nothing in the retime model assumes audio ignores
  it. Montage practice today (music is the master; gameplay audio is muted) makes this a
  low-cost cut.

## 8. Export

- Export mixes audio with the same engine as preview (same code path, §3.1, minus the
  device) at the export sample rate, and encodes **AAC via ffmpeg** (default 48 kHz stereo
  256 kbps; wav/PCM available for archival exports). Encoder settings live in the export
  queue's per-item settings ([06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) export
  section).
- Export MUST be sample-accurate and deterministic: two exports of the same project
  produce identical PCM before encoding.
- **Loudness normalisation** (EBU R128 / −14 LUFS targets for platform delivery) is a
  planned post-v1 export option, not in v1; the export path reserves a final-mix analysis
  hook for it.

## 9. The Composer (future — design intent)

The product owner's brief: *see the edit and add sounds, with more than one sound per
layer.* The Composer is a workspace ([01-GLOSSARY.md](01-GLOSSARY.md) §7) for sound
design against a finished or in-progress edit — the reason montage editors currently
round-trip to Vegas.

**Core idea — audio attachments.** Any layer MAY host multiple **audio attachments**: an
audio source reference plus offset (relative to layer time), gain, fade-in/out, and an
optional anchor (layer in point, a marker, or a keyframe time on a named property — so a
whoosh rides a Smooth zoom trigger and an impact rides a cut). Attachments are ordinary
properties in the data model: serialised like properties, expression-visible, undoable,
and mixed by the §3.1 engine as additional sources. **No new timeline type is introduced**
— this is the load-bearing design constraint, and it is why nothing in v1 forecloses the
Composer: the v1 mixing model (sum of per-source gains) and the property model already
accommodate attachments; v1 simply ships no UI for them.

**The workspace.** A Composer layout: a video program view (the Viewer, playing the comp)
above an audio-focused timeline in which each layer row shows its attachments as compact
pills with waveforms, plus the comp's Audio layers. Alongside: a **sounds library panel**
— tagged SFX folders (whoosh, impact, riser, ambience) with hover-audition (hover plays
the sound), drag-out to attach at a beat marker or playhead. Per-attachment controls:
gain, fades, offset nudge (with beat snapping, §5).

**Later still**: per-attachment send levels into a master chain (the first legitimate home
for audio effects), ducking presets (music dips under SFX), and library packs shipping
with Lumit under a clear licence.

**Sequencing.** The Composer ships after v1 ([16-ROADMAP.md](16-ROADMAP.md)); the only v1
obligations it imposes are the ones already met above: property-shaped audio model, mixing
engine that sums arbitrary sources, and a file format that tolerates new property groups
(K-040 versioned schema).

---

## Open questions

1. **Session sample rate.** Fixed 48 kHz engine rate is specced; following the output
   device's native rate avoids one resample but complicates determinism (export MUST stay
   device-independent). Recommend fixed 48 kHz; confirm.
2. **Onset algorithm ceiling.** Spectral flux is fine for percussive genre music; melodic
   onsets (piano edits, some phonk) may need a complex-domain or ML detector later. Is
   detector pluggability worth designing in now, or is replace-when-needed acceptable?
3. **Detached audio linking.** Should detached audio keep a persistent sync-lock badge
   with "resync" (Premiere-style), or is the v1 grouping-only link enough for the
   audience? Needs a quick user test with a montage editor.
4. **Scrub feel.** Grain length and windowing for scrub audio (§3.4) need tuning against
   Vegas, which this audience considers the scrub benchmark; parameters live in one place
   so tuning is cheap.
5. **Composer library licensing.** Shipping SFX packs requires cleared-licence audio;
   source and licence for a ship-with library are unresolved (CC0 curation vs
   commissioned pack).
