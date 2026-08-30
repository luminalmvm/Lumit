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

The engine layer (`lumit-audio`, `lumit-bridge`) is built; the Flutter Audio panel is not.
The sections below describe the intended design, and [TODO.md](TODO.md) is the one document
that says what exists.

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

**Which device.** The system default, unless the user names one in Settings → Audio. The
choice is stored **by device id** (the name the sound system reports — cpal exposes no
other handle that survives a restart) and it is **application settings, never project
data**: a sound card is a property of the machine, so it lives in the settings file and the
frontend hands it to the engine on boot and on every change (`set_audio_device`;
`list_audio_devices` names what the machine offers and which one is in use). A named device
that is not present when the stream opens falls back to the system default, then to the
first output there is — the choice is *kept*, not rewritten, so plugging the device back in
uses it again, and the frontend is told a fallback happened so it can say so. A machine
with no output at all is the calm terminal no-device state of §3.1: no sound, no error,
the picture on its own clock. Changing the device closes the open stream (a cpal stream
cannot be moved), so sound stops until the next play.

**What the mix is doing** is read off that same callback (K-690). Once per buffer — about
ten milliseconds — it publishes, for every **mixer strip** and for the master, the loudest
sample it just wrote (peak) and the root-mean-square of it (RMS), plus a sticky flag saying
something reached the ceiling and the limiter had to hold it. A strip is **a row of the
composition being mixed**: a footage layer is its own, and everything arriving through one
Precomp layer folds onto that layer's row, because that is the row the mixer draws and the
fader that would move it. A strip reads **pre-master** (how loud that layer is), the master
reads what the device is handed. The numbers are a fixed bank of plain atomics the callback
overwrites — never a queue that can fill up — so the panel loads the newest reading whenever
it repaints and a missed buffer is ten milliseconds of a bar that is about to move again.
Peak *hold* — the line resting above the bar for a few seconds — is the panel's, not the
engine's; the clip flag is cleared by hand, because a light that cleared itself would report
an overload only to somebody who happened to be looking.

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

- Waveform peaks (min/max/RMS per block) are **computed on demand** from the decoded audio
  and held as a multi-zoom **peak pyramid** (`lumit-audio::peaks::PeakPyramid`, K-280): one
  pass over the samples fills the finest tier and the coarser ones fold down from it, at the
  samples-per-block sizes 256 / 4 096 / 65 536. The pyramid is built once per file and kept
  for the session by the bridge's own bounded cache (`lumit-bridge::peaks`, keyed by path so
  two layers cut from one song decode it once). Writing it to the project sidecar as a
  persistent **peak file** keyed by content hash is still the design intent and **not yet
  built** ([TODO.md](TODO.md)) — so it is rebuilt the next time the project opens.
- Waveforms render 0(pixels) from the peak buckets - never from raw decode. Rendering follows
  [15-DESIGN.md](15-DESIGN.md): filled min/max body with RMS core, no per-sample spikes.
- **The resolution follows the zoom** (K-280). A lane asks for the stretch of source it is
  currently showing, at one bucket per pixel column, and asks again when a zoom or a scroll
  moves that window far enough to matter; the pyramid answers from whichever tier is coarse
  enough that a bucket costs a handful of block merges. Zooming in therefore *gains* detail
  rather than stretching a summary taken at import — the failure the original fixed
  2 048-bucket strip had.
- **Past the finest tier, the samples answer** (K-284). A summary runs out somewhere: below
  one block per column, neighbouring columns share a block and the wave becomes a staircase
  of flat slabs. So a short source keeps its **mono mixdown** beside the pyramid (16-bit, at
  the peak rate — `SAMPLE_KEEP_SECONDS`, about ten minutes, past which the 64× zoom ceiling
  can never out-resolve the finest tier anyway), and a query finer than one block per bucket
  is taken straight off it in one streaming pass. Fully zoomed in, the lane then draws the
  signal itself — a continuous trace, which is what a waveform is supposed to become. The
  three bands are filtered on the fly over the same pass, run up from
  `SAMPLE_PREROLL` samples before the window so the filters are settled by the time it
  starts.
- **Multiwave** (K-280, redrawn by K-284): alongside the plain wave, the sound is split into
  three bands — bass (below 200 Hz), middle, treble (above 2 kHz) — with 24 dB/octave
  filters, and each is summarised the same way. The lane draws all three **over one another
  in one lane**, ranked dim to bright as the frequency climbs. So what is in a loud passage is
  visible where one wave would be a solid block, and a cut can be aimed at the kick or at the
  hats. Overlaid rather than in three separate lanes, because the point is to see inside the
  wave you are already reading, and because three lanes in a 22 px row are six pixels each and
  say nothing. Two rules keep three overlaid waves legible (K-382): they are drawn **treble
  first and bass last**, so the pale end of the ramp sits behind and each darker band lands in
  front of a paler one (a dark shape on a pale one reads as two shapes; the reverse swallows
  what is under it), and **each is lifted slightly above the one behind it** — proportional to
  the row, 1–4 px — because three concentric waves of one sound agree most of the time and
  hide each other where they do. The lift comes out of the wave's own height, never off the
  top of the row. On by default;
  Settings ▸ Interface ▸ Editing ▸ *Waveforms show the frequency stack* returns the single
  wave.
- **Where the wave sits** is a second, independent choice (K-285). Centred about silence by
  default; Settings ▸ Interface ▸ Editing ▸ *Waveforms rise from the bottom* stands it on the
  floor of its row instead, rectified — each column reaching up by how far the signal swung
  either way, whichever was further. Half of a centred wave is a mirror of the other half, so
  folding it spends the whole row's height on the half that carries information, which reads
  better in a short row. It applies to the single wave and the stack alike, and it changes
  nothing about what is fetched: the peaks are the same either way, so switching it repaints
  and asks the engine for nothing.
- **A retimed layer's wave stretches with its map** (K-436). A lane's window is in the
  **layer's own clock**, and each bucket's edges are mapped through the layer's Retime
  (`Layer::source_time_at`) before the pyramid is asked — the same shape a clip's buckets
  already had through `Clip::source_time`. Bucketed evenly in source time instead, a layer
  slowed to half filled the left half of its bar with the whole of its sound and drew
  silence across the right half: the picture and the wave disagreed about which moment a
  column stood for. An un-retimed layer maps through the identity and still hands its whole
  window to `PeakPyramid::range` in one pass, so only a layer that has actually been
  retimed pays for the per-bucket walk. A reshaped map changes the answer without moving
  the window, so a retimed layer's fetch key carries the document revision as well.
- **The lane is drawn across both of its rows** (K-437). A waveform lane only ever exists
  under its own **Waveform** twirl, whose row is empty lane space — so the lane paints at
  twice the row height, standing on its own floor and reaching up through the row above. A
  centred wave then sits on the **divider** between the two, which is a line that is really
  there rather than an invented one inside a row; one rising from the floor gets the pair to
  rise through. Only the painting reaches up: the row keeps its height, so the outline and
  the lanes stay level. A Sequence clip's wave is unchanged — it is drawn inside the clip's
  own box, which has no empty row beside it to borrow.
- Waveforms appear: on Audio layers (always), on Footage layers with audio (expandable
  lane), and **inside Sequence layer clips** — each clip draws the waveform of its own
  source range, so a cut's audio content is visible exactly where the clip sits. Clip
  waveforms account for the clip's trim and its speed map (they are bucketed in the clip's
  own placed time, so a ramp's transients land where they are heard) and travel with the
  clip when it is slid; they are the primary visual for beat-checking an edit.

## 5. Markers and beat detection

- **Manual markers**: comp and layer markers per [01-GLOSSARY.md](01-GLOSSARY.md) §3,
  placed at the playhead (keyboard: `*`, per the [07-UI-SPEC.md](07-UI-SPEC.md) §15 keymap),
  draggable, labelled.
- **Beat markers**: generated by onset analysis of a chosen audio layer (or asset). v1
  algorithm: **spectral-flux** onset detection, robust on the scene's material (EDM/phonk/trap
  with hard transients). The parameters and thresholds are
  [impl/beat-detection.md](impl/beat-detection.md), which is authoritative for them. Controls:
  - **Sensitivity** (0–100): scales the adaptive threshold; live re-run is near-instant
    because the STFT is cached from the import pass.
  - **BPM-grid assist**: estimates tempo (autocorrelation of the onset envelope), lets the
    user confirm or type a BPM and phase, then snaps detected onsets to the grid and fills
    grid beats where detection missed one. The grid is assistive; markers remain
    individually editable.
  - **Tap tempo**: tap a key in time with playback to seed the BPM estimate.
- Beat markers are ordinary markers with a `beat` label: deletable, draggable, and stored
  in the project file. Re-running detection offers replace or merge.
- **While it runs**: detection is seconds-long on a long composition, so the shell puts up
  the same card it shows while a project opens, reading "Detecting beats", and takes it
  down when the markers land — or when the analysis finds nothing, which a composition
  with no audio is entitled to.
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
  follows a dragged bar in realtime, where the strip only refreshed on re-mix). `L` opens
  that group on the selected layers, `LL` opens the waveform lane inside it, `LLL` shuts
  them again (K-281).
- **Mute / solo** via the audible and solo switches ([01-GLOSSARY.md](01-GLOSSARY.md) §2).
  Solo on any layer silences non-soloed audio, matching video solo semantics.
- **Audio from video footage**: a Footage layer with audio exposes its audio as part of
  the same layer (audible switch, volume property, waveform lane). **Shipped (K-435):**
  *Add audio only* on a footage item places that item's sound as its own **Audio layer** —
  a Footage layer with `audio_only` set (docs/03 §5.7), which never draws. Media with no
  picture becomes one on placement whichever route placed it. "Detach audio" — a *linked*
  Audio layer kept in step with an existing Footage layer's source — is **not yet built**
  ([TODO.md](TODO.md)); *Add audio only* makes an independent layer from the item.
- **Switches show only what a layer can do** (K-435): an Audio layer is offered no
  visibility switch, and a layer that can never be heard — a solid, a title, a shape,
  image-only footage — is offered no audible switch. The same reasoning that decides
  whether the Audio group appears under a layer at all (§4.3 of
  [07-UI-SPEC.md](07-UI-SPEC.md)).
- **An Audio layer is never in the picture's frame key**, so muting, hiding, soloing or
  shying one retires no rendered frame; soloing it silences other audio without blanking
  the picture (docs/03 §5.7).
- Stereo is the v1 channel model; mono sources upmix centred. Pan is not in v1 (see §7).

## 7. Out of scope for v1

- **Audio effects** (EQ, reverb, compression) — none in v1. The effect stack accepts no
  audio effects until the Composer phase; the [12-PLUGINS.md](12-PLUGINS.md) LFX surface
  reserves an audio-effect extension so the ABI does not need breaking later.
- **Mixing console** — no mixer panel; per-layer volume plus master limiter only.
- **Audio retiming.** Retime is video-only in v1: a retimed Footage layer's own audio is
  intended to mute with a badge whenever its retime map differs from identity ("Retime mutes
  audio in this version") - the mute-on-retime detection and badge are **not yet wired**
  ([TODO.md](TODO.md)). The reason it mutes rather than warps: unpitched audio warping sounds
  bad and pitch-preserving stretching is real work. Roadmap: a later release adds
  pitch-preserving audio retime
  (phase-vocoder or WSOLA class) as a per-layer opt-in following the same retime map
  ([04-RETIMING.md](04-RETIMING.md)); nothing in the retime model assumes audio ignores
  it. Montage practice today (music is the master; gameplay audio is muted) makes this a
  low-cost cut.

## 8. Export

- Export mixes audio with the same engine as preview (same code path, §3.1, minus the
  device) at the export sample rate, and encodes **AAC via ffmpeg** (default 48 kHz stereo
  320 kbps). Encoder settings live in the export
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
