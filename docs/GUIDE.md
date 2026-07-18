# The plain-English guide to Lumit's code

**Who this is for:** the project owner — someone who knows editing software inside-out but
has never written Rust and hasn't worked with threads or GPUs directly. Read this once and
you'll be able to navigate the codebase, understand what each part does and why, and make
changes without fear. Everything here is explained with editing analogies where they help.
No prior programming knowledge in Rust is assumed; general "I've seen code before" level is.

This guide is **kept current by rule** (CLAUDE.md): whenever a new concept enters the
codebase, a plain-English section for it is added here in the same commit.

---

## 1. The 30-second map

Lumit is split into **crates** (Rust's word for a module/library — think of them as the
app's departments). They live in `crates/`:

| Crate | Job | Plain English |
|---|---|---|
| `lumit-core` | Time, the document, undo | The project file's brain: what a comp/layer *is*, and every edit that can happen to it |
| `lumit-project` | `.lum` files, autosave, recovery | Saving and loading, and the "never lose work" machinery |
| `lumit-ui` | Everything you see | Panels, menus, the theme — the shell around the engine |
| `lumit-app` | The `main()` entry | Ten lines that open the window and start the UI |
| `lumit-media` | (coming) decoding video | Turning an .mp4 into frames |
| `lumit-gpu` | (coming) the GPU pipeline | Drawing and processing frames on the graphics card |
| `lumit-audio` | (coming) sound | Playback and the clock everything syncs to |
| `lumit-eval` | (coming) the render engine | Working out what each frame looks like |
| `lumit-cache` | (coming) caching | Remembering rendered frames so they're never rendered twice |

Three of these have proper names you'll see in the app and docs (decision K-083),
drawn from the same astral register as the app itself: **Nova** (a burst of new light) is
the render pipeline — `lumit-eval` + `lumit-gpu` working together to turn the project's
edits into the picture; **Nebula** (the cloud where material gathers) is the cache;
**Pulsar** (the cosmic clock) is the audio engine whose clock everything syncs to. Crate
names stay plain `lumit-*` — the names are for people, the identifiers are for code.

**One rule ties them together:** the engine crates never depend on the UI. The UI asks the
engine for things; the engine doesn't know the UI exists. That's why the UI could be
replaced entirely without touching the engine — like swapping a car's dashboard without
opening the engine bay.

## 2. Rust in ten minutes, Lumit edition

You don't need to write Rust to read it. The handful of ideas that appear everywhere:

- **Ownership.** Every piece of data in Rust has exactly one owner, and the compiler
  enforces it. When you see code "cloning" a document, that's making an independent copy so
  two parts of the app can't fight over one. This is the language feature that makes the
  "never crashes" goal realistic — whole categories of crash (two threads corrupting the
  same memory) simply don't compile.
- **`Result` — errors are values, not explosions.** A function that can fail returns
  `Result<Thing, Error>`: either `Ok(thing)` or `Err(why)`. The caller *must* deal with
  both. You'll see `?` a lot — it means "if this failed, pass the error up to my caller".
  Lumit bans the shortcuts (`unwrap`/`panic`) that turn errors into crashes; the build
  literally fails if someone uses them in engine code.
- **`Option`** is the same idea for "might not exist": `Some(comp)` or `None`. No
  null-pointer crashes, ever.
- **Structs and enums.** A `struct` is a record (a Layer has a name, an in point, an out
  point…). An `enum` is a choice between shapes — `LayerKind` is Footage *or* Sequence *or*
  Text… and the compiler forces every `match` to handle every case, so adding a new layer
  kind makes the compiler point at every place that needs updating. That's why the strict
  glossary maps so well to code.
- **Traits** are capability contracts, like "anything that can decode frames". Code can say
  "give me anything that satisfies this trait" — that's how the engine will stay swappable
  (a CPU decoder and a GPU decoder behind the same trait).
- **`Arc<T>`** means "shared, read-only handle to T" (Atomically Reference Counted). Several
  parts of the app can hold the same document snapshot at once; it's freed automatically
  when the last holder lets go.
- **Crates and Cargo.** `Cargo.toml` files list dependencies (like a plugins list).
  `cargo build` compiles, `cargo run` launches, `cargo test` runs every test. Those three
  commands are 95% of what you'll ever type.

## 3. Threads, in editing terms

A thread is an independent worker inside the program. Lumit's design gives each worker a
fixed job (the full table is in [05-ARCHITECTURE.md](05-ARCHITECTURE.md)):

- **The UI thread** is front-of-house: it draws the interface and responds to your mouse.
  The golden rule — it **never** does heavy work. Every stutter you've ever felt in AE is
  some engineer breaking this rule. In Lumit it's structural: the UI thread hands work to
  others and carries on drawing.
- **Worker threads** are the render farm: they evaluate frames, run effects, do maths.
  There are roughly as many as your CPU has cores.
- **Dedicated threads** exist for decoding video, disk IO, and audio — because those jobs
  must never wait behind anything else (audio especially: if its thread is ever late, you
  *hear* it).

Two mechanisms make this safe, and you'll see them by name in the code:

- **Snapshots (`ArcSwap`).** When you edit, the UI thread produces a complete new immutable
  copy of the document and atomically swaps a pointer to it. Workers that were mid-render
  keep the old copy; new work uses the new one. Nobody ever sees a half-finished edit —
  like workers each getting their own printed copy of the script, and edits producing a
  fresh printing rather than scribbling on someone's pages.
- **Epochs (cancellation).** Every piece of work carries a ticket number. When you scrub,
  the global ticket number increments; workers check their ticket often ("is my work still
  wanted?") and quietly stop if it's stale. Nothing is force-killed — force-killing is how
  you corrupt state — everything checks and steps aside. Details in
  [impl/playback-scheduler.md](impl/playback-scheduler.md).
- **Channels** are how threads hand each other work: a conveyor belt with a fixed length.
  A full belt makes the sender wait — that's **back-pressure**, and it's deliberate: it's
  the mechanism that stops the app drowning itself under load (rule K-018, degrade never
  crash).

## 4. What exists today, file by file

- `crates/lumit-core/src/time.rs` — **Rational time.** Times are stored as exact fractions
  (`num/den`), never decimals, so frame maths is exact forever (a 3-hour NTSC timeline
  never drifts by a frame). The four "timebases" (source/clip/layer/comp time — glossary §4)
  are separate types, so mixing them up is a compile error, not a subtle bug.
- `crates/lumit-core/src/model.rs` — **What a project is.** Structs for the document,
  comps, layers, footage items. Each has an `extra` field that preserves anything a future
  Lumit version adds — so old and new versions can share project files.
- **Block glitch and Scanlines, the corrupted-video look, as two separate effects** (a third,
  Datamosh, is explained further down, once the flow-field machinery it needs has been
  introduced — the three used to be one "Glitch" effect with on/off sections, but each does
  one thing so each is now its own effect you drop on separately; stack Block glitch then
  Scanlines to get the old combined look back). **Block glitch**
  carves the frame into a grid (Block size) and, per block, reads its picture from a
  slightly different spot — a random-looking but fully repeatable jump, plus an optional
  colour-channel split and a "slice repeat" look where a thin strip of the block tiles
  instead of showing a plain shifted read. **Scanlines** darkens alternating bands of rows
  (Line period, Darkness), optionally rolling them over time and alternating which half of
  each band darkens every other cycle for an interlaced-video feel — it has no hash and no
  Seed of its own, since it just reads straight down from each row rather than jumping
  around like Block glitch does. Each effect has its own Intensity dial that turns its own
  look up or down, and at 0 each is a guaranteed no-op — checked by a test — whatever Mix is
  set to. The interesting engineering wrinkle, in Block glitch: which block "moves" and by
  how much has to be decided freshly for every pixel, on the GPU, from nothing but (seed,
  that block's row/column, a coarse time-step) — there's no way to precompute a lookup table
  for it up front, because a busy frame can have thousands of blocks. That means the effect
  needs its own hash function running *inside* the graphics-card program, not just on the CPU
  side like Shake's wobble does. Shake's existing hash is built on 64-bit numbers, which
  graphics-card programs (written in a language called WGSL) cannot represent — so Block
  glitch gets a sibling hash built entirely from 32-bit numbers instead, same design, both
  the CPU and the GPU version running the identical recipe so they always agree. Every
  "which block, how much, which look" answer comes from that one shared hash fed different
  small numbers, which is also why the same project glitches exactly the same way on every
  machine, every time. It reuses the same frame-cache lesson Shake taught the codebase:
  because Block glitch is seeded, the cache automatically knows a frozen frame still needs
  the *current* moment's local time to look right, with no Block-glitch-specific code needed
  for that part at all.
- **Echo / trails, and "temporal effects"** — the montage speed-line staple, and the first
  effect that needs *more than the current frame*. Until now every effect looked only at the
  single frame it was drawn on. Echo lays several earlier frames of the layer behind (or over)
  the current one, each fainter than the last, so a fast move smears into a trail. That means
  the app has to fetch those earlier frames and hand them to the effect — a new bit of
  plumbing. Each effect now declares, up front, which frames it needs (a little list of
  offsets like "this frame, one back, two back…"); the decode step reads the layer's footage
  at exactly those moments (following the retiming, same as the frame you're on), and both the
  live preview and the export do it the identical way so they still match. The picture cache
  learned about it too: an echo frame's identity now includes the neighbours it's built from,
  so — like the flow fix earlier — you never get a stale, frozen trail. In this first version
  Echo reaches back up to eight frames one frame apart, fades each by a Decay you set, and
  offers three ways to stack the trail (Add for bright streaks, Behind for ghosting, Max for a
  lighten-only look); wider/looser trails are a follow-up, and the other effects that want
  neighbouring frames — motion blur that follows real motion, and the datamosh look — build on
  this same machinery (both explained further down).
- **Motion blur that follows real motion** — the second temporal effect, and the one that
  turns game capture (which has no natural blur — every frame is pin-sharp) into footage that
  streaks the way a real camera would. It builds on two things already in the box: Echo's
  "fetch a neighbouring frame" plumbing, and the optical-flow engine that powers slow-motion.
  The trick is to look at the current frame and the *next* one, work out how far every pixel
  moved between them (that's the flow — a little arrow for each pixel saying where it went),
  and then smear each pixel along its own arrow. Fast-moving areas get long streaks; still
  areas stay crisp — exactly what real motion blur does, and what plugins like RSMB sell. The
  flow is worked out during decoding, where both frames are sitting in memory anyway (the same
  place slow-motion computes it), and passed to the blur as a little motion-map image; the
  preview and the export do it the identical way, so what you see is what you get. Two knobs:
  **Shutter angle** (how long the "shutter" stays open — 180° is the film-standard half-frame
  smear; higher blurs more, up to a full 720°) and **Samples** (how many steps to take along
  each streak — more is smoother but slower). A still frame, or a shutter of zero, leaves the
  picture untouched. For now it follows the footage's own motion only (not, yet, motion you
  add with keyframes) and works on footage layers, the same starting scope Echo has.
- **Datamosh** — the corrupted-video "reused an old frame's motion" look, and the third
  effect to use the flow-field machinery Motion blur introduced. Real video codecs sometimes
  drop a frame's actual picture data and just reuse the previous frame's content nudged by
  that frame's motion vectors — which looks like melting, trailing smears where things
  moved. This effect fakes that on purpose: it works out how far every pixel moved between
  the frame *before* this one and this one (the same kind of arrow-map Motion blur reads,
  just measured one frame earlier), then paints each pixel of *this* frame by looking up
  where that arrow says its content used to be, back in the previous frame — a single lookup
  per pixel, not Motion blur's multi-step smear along the arrow. Intensity fades between the
  ordinary frame and the moshed one. It started life as a toggle inside Glitch, off by
  default, because turning it on meant fetching an extra frame and running the motion-arrow
  calculation, unlike Glitch's other two sections which were always on; when Glitch split
  into three separate effects, Datamosh kept that same shape as its own effect — you simply
  do not add it to a layer unless you want the look, rather than flicking a switch inside a
  bigger effect. One wrinkle worth knowing: the app can only carry one motion-arrow map per
  layer per frame right now, so if a layer somehow had both Motion blur and Datamosh turned
  on together, only whichever one is listed first in the effect stack gets its arrows this
  frame — the other quietly sits out, the same "missing data, do nothing" safety rule every
  temporal effect already follows.
- **Blur gains a Radial mode** — the third and final mode of the §3.8 trio, alongside
  Gaussian and Directional. Drop a Centre point anywhere on the frame (as two percentages,
  Centre X and Centre Y, of the frame's width and height) and pick a Type: **Spin** streaks
  every pixel along the arc it would trace if the frame span rotating about that point;
  **Zoom** streaks it along the straight line from the centre through it instead, like a
  camera punching in. Either way the streak grows the farther a pixel sits from Centre —
  right at Centre nothing moves at all, and the effect gets stronger toward the edges,
  reaching its full length (set by Amount, in the same "% of frame diagonal" units Radius
  and Length already use) at the frame's farthest corner. The clever bit is *how* those two
  streak directions get computed: rather than actually rotating anything (which needs
  trigonometry, and GPU trigonometry is allowed to be slightly imprecise — the same reason
  Transform's matrix arrives pre-computed from the CPU), both Spin and Zoom turn out to be
  nothing more than stretching the vector from Centre to the pixel by a plain number — along
  that vector for Zoom, sideways from it for Spin. No division, no sine or cosine anywhere,
  and — as a free bonus — every stretch is exactly zero at Centre itself, so there is no
  special case to write for "what happens exactly at the middle". Sideways-instead-of-rotated
  is a deliberate simplification (a straight sideways nudge closely matches a true curved arc
  for the modest sweep this effect targets) and is written down as a pinned choice in docs/08
  §3.8, alongside the other numbers the spec didn't pin down itself (the exact ranges and
  defaults for Centre and Amount). Old projects saved before Radial existed still read as
  Gaussian, byte for byte, and Amount 0 is an exact passthrough — both pinned by tests.
- **Flash fires on the beat.** The Flash effect's Mode switch now has three positions.
  *Manual* is exactly the old behaviour — keyframed hits with an exponential fade — and
  stays the default, so nothing saved earlier changes by a single byte. *Trigger* lights
  the flash from the comp's beat markers themselves: on each beat the envelope jumps to
  full, then either cuts off after Duration frames (Shape: Hard) or ramps linearly to
  zero across them (Shape: Fade); Phase offset slides every hit earlier or later by
  whole frames. *Strobe* is Trigger that counts: only every Nth beat fires, which is how
  "flash on the kick, not the hi-hat" works when the detector marked both. All of this
  is worked out on the CPU while parameters resolve — the GPU kernel still receives one
  strength number, untouched, so the existing Flash oracle passes as it was. The frame
  cache learned the matching lesson in the same commit: a beat-driven flash's cache key
  now includes the frame's local time and the small window of triggers its envelope
  actually reads, so nudging a distant marker never re-renders frames it cannot affect,
  while a Manual-mode flash keeps its time-free keys.
- **Beat markers reach the effects engine** (the docs/08 §1.4 plumbing). When a layer's
  effect stack is resolved for a frame, it now receives a small *marker context*: the
  comp's beat-marker times, each translated into the layer's own clock (a layer that
  starts three seconds into the comp sees a beat at comp second five as “two seconds
  in”), plus the comp's frame rate so parameters authored in frames can become seconds.
  Nothing draws differently yet — this is the wiring the beat-driven effect modes
  (Flash first) plug into. Two details matter: the context is built by one shared
  constructor that preview and export both call, so the two can never disagree about
  where a beat falls (the K-031 promise); and a caller with no markers passes an
  obvious empty context, because a marker-driven effect must always degrade to doing
  nothing rather than misbehaving — a project with no music still renders.
- **Shake.** The beatshake workhorse: a virtual camera wobble. The layer is resampled
  once through the same kernel the Transform effect uses — never pixel noise — so the
  whole frame sways as one. The wobble comes from *seeded value noise*: a deterministic
  recipe that turns (seed, time) into a smooth wander between −1 and 1, so the same
  project shakes identically on every machine and every run — there is no real
  randomness anywhere, only maths that looks random (the engine's seeded-and-stateless
  rule). Amplitude sets how far it roams (as % of the comp diagonal), Frequency how
  fast, Rotation amount how much twist; Auto-scale (on by default) zooms in just enough
  that the wobble never drags the frame's edge into view. Seed is a new parameter type:
  an integer picking *which* wander you get — each new instance rolls its own so two
  shaken layers never move in sync, and the Reseed button rolls a fresh one. Shake also
  taught the frame cache a lesson: its parameters can sit constant while the picture
  moves every frame, so for effects that declare seeded randomness the cache key now
  includes the layer's local time — without that, a shaken solid would replay its first
  cached frame forever.
- **Glow.** The montage bloom: anything brighter than Threshold spills light. The
  pipeline is three steps — keep only the light *above* the threshold (with Knee
  easing the cut so it doesn't snap on), blur that leftover wide (Radius, measured
  like Blur's), then add it back on top, scaled by Intensity and coloured by Tint.
  Because Lumit works in scene-linear light, an HDR value of 4 has four times the
  energy of white and blooms accordingly — which is why Threshold is the first
  parameter with a *one-sided* hard range (design rule K-090): it clamps at zero
  below but you can type any value above the slider's 4, because HDR pixels really
  do sit up there. The halo carries alpha too: glow blooming past a layer's edge
  raises coverage there, so the spill reads as light over transparency instead of
  stopping dead at the matte. At Intensity 0 the effect passes pixels through
  bit-exactly — a test pins that promise.
- **RGB split gains a Wavelength mode** (K-090's quality-tier pattern: where physical
  accuracy is optional, it hides behind a Bool next to the fast look). Off — the
  default, and exactly the effect as it was, byte for byte — the split is three
  samples: red pulled one way, blue the other, green in place. On, the kernel instead
  takes *nine* samples spread along the same line, one per slice of the visible
  spectrum from 650 nm red to 450 nm blue-violet, and weights each by that
  wavelength's actual colour in linear RGB before summing — how real lens dispersion
  works, so the fringe is a graded rainbow rather than a hard red/blue rim. The
  wavelength→colour table lives in `lumit-core` next to the CPU reference and is
  handed to the GPU kernel through its parameter block, so both paths read literally
  the same numbers (the same trick as the host-computed sines). The table's columns
  are normalised so a flat image passes through unchanged, and alpha still refuses to
  move — mattes never grow coloured rims in either mode.
- **The Transform effect** (K-090, replacing the dropped smooth-zoom idea) is the layer
  transform group — Anchor, Position, Scale, Rotation, Opacity, same names and units —
  packaged as a stack effect. Why would you want a second transform? *Adjustment
  layers.* An adjustment layer's effects apply to the composite of everything below
  it, so a Transform effect on one is the montage punch-in or whip-pan gesture over
  the whole frame at once, without touching any individual layer's own transform.
  Under the hood it works backwards: for each output pixel the kernel asks "which
  input point would the forward transform have moved *here*?" (the inverse affine),
  takes one bilinear sample there, and shows transparent for anything that maps
  outside the frame. The matrix arrives pre-computed from the CPU (GPU trigonometry
  is allowed to be sloppy; ours must match the reference bit-for-bit), and at default
  parameters the effect is a *bit-exact* passthrough — a test pins that promise. A
  zero scale collapses the image to fully transparent rather than dividing by zero —
  engine code never faults. Its Anchor and Position are measured in comp pixels, so
  the resolver now carries the preview-resolution factor as well as the diagonal:
  half-resolution preview frames exactly like full, only softer (design rule §2.3).
- **Blur grows a Directional mode.** The Blur effect now has a Mode switch: *Gaussian*
  (the soft circular blur it has always been) or *Directional* — a streak along an
  angle, the speed-line look. Under the hood directional blur is a *line integral*:
  for each pixel, the kernel walks a short line through it (Length long, pointing
  along Angle), samples the image at evenly spaced points on that line, and averages
  them — as if the image slid past an open shutter in that direction. The two modes
  are separate GPU programs, so adding Directional changed nothing about Gaussian:
  the original blur maths, and the test that pins them to the CPU reference, are
  byte-for-byte what they were. Old projects saved before the switch existed simply
  read as Gaussian. (The third §3.8 mode, Radial spin/zoom, is still to come.)
- **Grade splits into Colour balance and Saturation** (K-090's one-thing rule: an
  effect does one job, so the young all-in-one Grade became two Colour-category
  effects; a deliberate all-in-one grading suite may return much later, but
  single-purpose is the default shape). **Colour balance** is lift / gamma / gain per
  channel — the trackball grammar every colourist tool shares. *Gain* multiplies
  (brightens everything proportionally), *lift* adds (raises the blacks — or crushes
  them, negative values are allowed), *gamma* bends the mid-tones without moving black
  or white. Each is a colour parameter, so warming the shadows while cooling the
  highlights is just different numbers per channel. **Saturation** does exactly one
  thing: it pivots colourfulness around proper Rec. 709 luma, so desaturating gives
  true greyscale, not the grey-green mush of naive averaging. The same two design
  rules shape both: they grade *unpremultiplied* colour (same reason as Sharpen —
  grading premultiplied pixels shifts matte edges), and they never clip highlights — a
  gain of 2 on an HDR value of 4 gives 8, and whatever glow comes later gets all of
  it. Neutral settings now short-circuit the *whole effect*: at defaults each passes
  pixels through bit-for-bit untouched (and there's a test holding it to that) rather
  than rounding them through power curves. The rest of §3.10 — exposure, white
  balance, curves, vignette, and the Looks-style preset browser — arrives as further
  single-purpose colour effects.
- **Flash.** The beat-strobe, in its manual form until beat markers exist. Its Trigger
  parameter reads unusually on purpose: *each keyframe is a hit*. Drop a keyframe with
  value 1 on a kick drum and the frame flashes to the flash colour, then fades out
  exponentially over Decay milliseconds — you author one keyframe per beat, not a
  spike-and-fall pair. (When the audio engine starts producing beat markers, they'll
  drive the same envelope automatically — that's why the effect declares "marker input:
  beat" in its traits already.) The flash respects the layer's own transparency: pixels
  outside the footprint never light up, so flashing a masked layer flashes the masked
  shape, not the whole rectangle. Flash also introduced the **colour parameter**: an
  effect can now declare a scene-linear RGBA colour (the Flash tint defaults to white),
  which the Effects group shows as R/G/B number fields plus a live swatch. Linear values
  above 1 are legal — a "4.0 white" flash carries real HDR energy into any glow that
  follows it in the stack.
- **RGB split.** The impact-frame staple: the red and blue channels slide apart while
  green stays put, like a lens fringing under stress. Keyframe a spike on Amount at a
  hit and you have the genre's signature punch. Two modes: *linear* shifts everything
  one way (set by Angle), *radial* grows the shift from the centre outward, like real
  lens aberration. Two details matter in the code: alpha stays glued to the green
  channel (if it moved with red or blue, every matte edge would grow a coloured rim —
  design rule §3.6), and the sines behind the shift direction are computed once on the
  CPU and handed to the GPU, because GPU trigonometry is allowed to be slightly
  imprecise and the CPU-vs-GPU agreement test demands better.
- **Sharpen.** The second effect in the catalogue, following Blur's four-part template.
  It's an *unsharp mask* — the counter-intuitive classic: blur a copy of the image,
  subtract it from the original (what's left is the fine detail), then add that detail
  back on top, scaled by Amount. Two subtleties earn comments in the code. First, it
  works on **unpremultiplied** colour (design rule §2.2): footage with transparency
  stores its colours pre-multiplied by alpha, and sharpening those values directly would
  draw halos around every matte edge — so the kernel divides alpha out, sharpens, and
  multiplies it back in. Second, **Threshold** is a *soft* gate: detail weaker than the
  threshold (compression noise, mostly) is ignored, but rather than a hard on/off — which
  would leave visible contours where detail crosses the line — the gate shaves the
  threshold off everything, so the transition is seamless. "Luminance only" (the default)
  sharpens the brightness signal and leaves colour alone, because sharpening the colour
  channels of compressed game capture produces rainbow fringes.
- **Flow is a layer option** (K-088) — the wind toggle in a footage layer's switch
  cluster. On, it synthesises in-between frames with optical flow wherever the footage's
  rate (through any retime) undershoots the comp's — the moment a source frame would sit
  across two comp frames, flow takes over; footage already at comp rate costs nothing. A
  **Flow** group appears beside Transform and Effects with the engine's knobs (Quality:
  half-resolution fields, the fast default, or full). Under the hood it's the retime's
  frame-interpolation policy — an un-retimed layer quietly gains an identity retime to
  carry it, and loses it again when you switch off.
- **Effects are usable end to end.** Twirl a layer open, open its **Effects** group,
  and "Add effect" lists the catalogue. Each effect shows a bypass
  tick, a remove button, and one row per parameter — a Blur radius has a stopwatch
  and lane diamonds exactly like Position does, so effect animation and layer
  animation are one skill. The same stack renders in preview and in export through
  the same GPU passes, and cached frames re-render themselves when a parameter
  moves (the cache key already understood effects).
- **Dragging an effect on works too (K-101).** You don't have to open the "Add
  effect" menu: drag an entry straight out of the Effects & Presets browser and drop
  it on a footage or adjustment layer's row in the Timeline — the row outlines while
  you hover, and letting go appends the effect exactly as if you'd picked it from
  that layer's own menu, one undo step either way.
- **Effects, the pixel side.** The first real effect exists end to end: **Blur**
  (gaussian). Its life is the template every effect will follow (design rule §1.1's four
  parts): a catalogue entry in `lumit-core/src/fx.rs` declaring parameters and behaviour
  traits; a plain-Rust reference implementation there too (the *oracle* — slow but
  unarguably correct); a GPU program (`lumit-gpu/src/fx_blur.wgsl`) that does the same
  maths fast; and a test that renders a nasty little corpus (gradients, hard alpha edges,
  a brighter-than-white spike) through both and fails if they ever disagree. The radius is
  measured as a percentage of the comp's diagonal, so half-resolution preview looks the
  same as full — just smaller.
- **Effects, the data side (Phase 3 begins here).** Every layer now carries an ordered
  **effect stack** in the project model: each entry says *which* effect (a stable name +
  a version, so cached frames from older maths retire themselves), whether it's bypassed,
  and its parameters — which are real animatable properties like Position or Opacity, so
  keyframes and the graph editor work on a Glow radius exactly as they do on a scale. A
  layer-level **fx switch** mutes the whole stack. Edits go through ops (one op replaces the
  stack — add, remove, reorder and parameter changes are all undoable in one step), and the
  cache knows a live effect changes pixels while a bypassed one doesn't. The registry (a
  growing built-in catalogue — blur, sharpen, RGB split, glow, shake, colour balance and
  more, grouped by category), the GPU passes, and adjustment-layer staging (K-091) all run
  for real now, and the dedicated **Effect Controls** dock panel shows the selected layer's
  effect stack in a roomier home than the Timeline row — the same rows, the same undo, just
  reusing the Timeline's stack editor rather than being a second, divergent one. You can
  still edit the stack inline on the layer's own row in the Timeline; the panel is the same
  editor given more room. Saving a stack as a **preset** and loading one back (a small
  `.lumfx` JSON file, K-065) lives on that same add-effect row. **Dragging an effect's value
  updates the Viewer live** — as you drag a Glow radius or a Blur amount, the picture re-runs
  the effect with the value under your cursor every frame, committing once when you let go (so
  a whole drag is one undo step). It reuses the same trick a transform-value drag already uses:
  the retained frame is re-composited with the provisional value patched in, no re-decode.
- **Two more single-frame effects (K-099).** **Vignette** darkens the frame toward black
  away from the centre (Amount/Radius/Softness/Roundness); **Chromatic aberration** fringes
  red and blue outward/inward from the centre by a set number of pixels — a simpler,
  always-on-the-corner sibling of RGB split's own Radial mode, for the common one-click case.
- **Exposure (K-106).** The one-knob brightness lever, measured in photographic *stops* —
  each +1 doubles the light, −1 halves it. It is a straight multiply on the colour (done in
  the scene-linear light the compositor works in, so it behaves like a real camera exposure,
  not a washed-out lift), with 0 stops leaving the picture exactly untouched. Distinct from
  Colour balance's three-channel gain: a single animatable control for the whole image.
- **Hue shift (K-108).** Turn every colour's hue by an angle — reds toward orange, blues
  toward purple, and so on — while keeping how *bright* each looks unchanged (a
  "constant-luminance" rotation, the same maths web browsers use for their hue-rotate filter).
  0° leaves the picture exactly as it was. Under the hood it is a small fixed colour-mixing
  matrix worked out once for the angle, so the preview and the export apply the identical
  numbers.
- **Contrast (K-110).** The familiar contrast slider: push everything further from a middle
  grey (brights brighter, darks darker) or pull it toward that grey to flatten the image.
  100 % leaves the picture exactly as it was; below 100 % flattens, above 100 % punches. The
  middle grey it pivots around is a plain 50 %, like a photo editor's contrast control. One
  subtlety worth knowing: because it *shifts* colours toward or away from a fixed point rather
  than simply scaling them, it has to be done on the "straight" colour of a semi-transparent
  pixel — Lumit briefly divides the alpha back out, applies the contrast, then multiplies it
  back in, so soft edges keep their shape instead of fringing. Exposure does not need that
  step because a plain multiply already behaves the same with or without the alpha folded in.
- **Gamma (K-112).** A brightness curve for the mid-tones: it leaves pure black and
  pure white where they are but bends everything in between. A Gamma above 1 lifts the middle
  (a brighter, flatter look); below 1 pushes it down (darker, punchier). It is the classic
  "gamma" slider, where the number behaves like a monitor's gamma. Like Contrast it works on the
  "straight" colour of a semi-transparent pixel (Lumit divides the alpha out, curves, then
  multiplies it back in), so soft edges keep their shape. One safety detail: colours in the
  compositor's light space can dip a hair below zero, and raising a negative number to a power is
  meaningless, so Lumit nudges any such value up to zero before curving — done identically on the
  preview and the export, so the two never disagree. A Gamma of 1 leaves the picture exactly as
  it was.
- **Temperature (K-113).** The warm/cool slider: drag it positive to warm the picture (more
  red, less blue) or negative to cool it (more blue, less red), with green left alone. It is
  a plain per-channel multiply — red and blue each get their own gain worked out once from the
  slider (at +100, red is boosted by half and blue cut by half; 0 leaves the picture exactly
  as it was) — so, like Exposure, it needs no alpha round trip and semi-transparent edges stay
  clean. This is the quick one-knob warmth move, not a full colour-science white balance (that
  fuller version, which shifts the picture along real colour-temperature lines and adds a
  green/magenta Tint axis, is a later Tier-2 job); it is the everyday "make it feel warmer"
  control, and it animates like every other grade.
- **LUT (K-114).** Drop this on a layer and press its **Select Cube LUT…** button to pick a
  `.cube` file — a colour recipe a colourist baked elsewhere (the loader below reads it) — and
  the whole picture is regraded through it; the **Mix** slider dials the look back toward the
  original. Until you pick a file it simply passes the picture through unchanged (so does a file
  that is missing, unreadable, or the older one-dimensional kind — it never errors, just shows
  as doing nothing). Because a colour look is a whole file, you cannot smoothly *blend* from one
  LUT to another; you *step* between them with hold keyframes (the picture snaps to the new look
  at each key). One honest limitation to know: the file is applied to the picture in Lumit's own
  internal light space exactly as written, without first translating it into whatever space the
  LUT was authored for — a proper "input space" control is a later job — so a LUT built for a
  very different encoding may look off. This grade runs **only on the graphics card**: unlike
  Contrast or Gamma there is no slow CPU stand-in, so if Lumit ever has to fall back to
  CPU-only drawing a LUT layer shows through ungraded. Under the hood the cube of sample points
  is handed to the card as a **3D texture** — an ordinary image has width and height, a 3D
  texture adds a third dimension (depth), so the card can look a colour up by its red, green and
  blue coordinates in one fetch — the first effect in Lumit to need one. The preview and the
  export load and apply the LUT the same way, so an exported file matches what you saw.
- `crates/lumit-core/src/lut.rs` — **reading a colour LUT (`.cube` file).** A LUT
  (look-up table) is a colour recipe a colourist bakes elsewhere: feed it a red/green/blue
  and it hands back a graded red/green/blue. The common `.cube` text format stores that as a
  cube of sample points — a 3D LUT is a grid (say 33×33×33) of "this colour in, that colour
  out", a 1D LUT is three separate curves, one per channel. This file reads such a file into
  memory and answers the one question the coming LUT effect (docs/08 §3.11) will ask millions
  of times a frame — "what does this LUT turn *this* pixel into?" — by **trilinear
  interpolation**: it finds the eight grid points around the input colour and blends them by
  how close the input sits to each, so colours between the baked samples come out smooth
  rather than blocky (a 1D LUT just blends along each channel's own curve). That blending is
  deliberately the simplest continuous maths there is, because the identical recipe has to run
  again on the graphics card later and the two must agree to the last decimal — the
  CPU-reference-as-oracle rule (docs/08 §1.6). The reader is strict about broken files (a
  missing or repeated size, the wrong number of rows, non-numbers, a size of 0 or 1) and
  returns a plain typed error rather than ever crashing, and it refuses an absurd cube (over
  256 points per axis) instead of trying to allocate gigabytes for it. Nothing is wired to an
  effect yet — this is just the load-and-sample building block.
- `crates/lumit-core/src/ops.rs` — **Every possible edit, as data.** An edit is an `Op`
  (AddLayer, SetLayerSpan…). Applying an op returns its exact inverse — that pair is what
  makes undo *provably* correct instead of hopefully correct.
- **Layer parenting** (K-103) — a layer can name another layer as its **parent**, so moving,
  rotating or scaling the parent carries the child with it (the After Effects null-object
  rig). Pick a parent from the **Parent** dropdown at the top of the Effect Controls panel;
  the list hides any choice that would make a loop, and "None" clears it. Under the hood the
  child's picture is placed inside the parent's coordinate space by multiplying the parent's
  transform in front of the child's — reusing the very same machinery a collapsed precomp
  already uses — computed identically for the preview and the export so they always match.
  A layer with no parent (every layer, until you set one) renders exactly as before. For now
  it inherits the flat 2D move/rotate/scale; inheriting the 2.5D depth/tilt is a later touch.
- **Solo (isolate)** (K-105) — tick **Solo** on a layer (top of the Effect Controls panel,
  next to Parent) and the composition shows only that layer; solo a few and it shows just
  those, hiding everything else, so you can look at one thing without deleting or hiding the
  rest. It is a view aid, not a permanent change — untick to bring everything back. The rule
  ("if anything is soloed, only soloed layers draw") is applied the same way in the preview
  and the export, so what you isolate is what you'd get. Nothing is soloed by default, so
  existing projects look identical.
- `crates/lumit-core/src/anim.rs` — **the keyframe engine.** Between two keyframes the
  value follows a bezier curve shaped by AE-style *speed* (units per second) and
  *influence* (how far each handle reaches). The subtle part: the curve is parametric, so
  "value at time t" first requires solving "where on the curve is x = t?" — done with a
  solver that combines Newton's speed with a bracket it mathematically cannot escape.
  That solver quality is exactly what makes handles feel right in a graph editor at the
  extremes (AE's 100% influence "spike" case is a test here). Property tests fire
  thousands of random curves at it per CI run.
- `crates/lumit-core/src/retime.rs` — **the Retime maths.** One store per clip answers
  "when the clip's clock reads t, which moment of the source shows?". Speed ramps,
  freezes and slow motion are all segments of that one curve, and the editor's speed
  graph and value graph are two views of the same store — never two systems. Every
  segment boundary keeps its source position as an exact fraction, so cutting and
  re-editing a ramp never drifts: a frame synced to a beat stays on the beat. The map
  only chooses *which* source moment shows; how in-between moments become pixels
  (nearest, blend, optical flow) is a separate per-clip policy. **All three are wired up now**:
  a retimed footage layer's twirl-down has a Frames toggle — Nearest shows the closest real
  frame (crisp, a touch stuttery in deep slow-mo), Blend crossfades the two neighbouring frames
  by how far between them the moment falls (smoother, slightly ghosted), and **Flow** invents a
  genuine in-between frame by working out how everything *moved* between the two and dragging
  each halfway (the real slow-mo trick). Flow lives in its own crate (`lumit-flow`) and uses
  **DIS — Dense Inverse Search** — the algorithm the specs pin for it (same family OpenCV
  ships). In plain terms: the frames are stacked into a pyramid of ever-smaller copies;
  starting from the smallest, thousands of little 8×8 tiles each hunt for where their bit of
  picture went (a few quick "am I getting warmer?" refinement steps each); every pixel then
  takes a vote among the tiles covering it, trusting only tiles whose answer actually *looks
  right* at that pixel — that mistrust is what keeps the motion crisp at object edges instead
  of rubber-sheeting. Pixels visible in only one frame (things being covered or revealed —
  where slow-mo artefacts live) are found by checking the two directions of motion against
  each other, and the synthesis quietly falls back to a plain crossfade wherever both frames
  lost sight of something. It ships as **two backends behind one door**: a pure, deterministic
  CPU implementation — the "oracle", also the export path on machines with no usable GPU
  (K-019) — and a GPU compute version (`gpu.rs` + `dis.wgsl`) that runs the identical
  algorithm as shader code, thousands of patches at once instead of one after another. The
  shader mirrors the CPU maths operation for operation, and a test holds the two to agreeing
  within a thousandth of a pixel; another proves the GPU gives bit-identical answers run to
  run. Callers hold a `FlowEngine`, which picks the GPU when one is available and quietly
  drops to the CPU if anything about the GPU ever fails — flow never crashes a preview, it
  just slows down. On the dev machine the GPU solves a 1080p flow pair in about 4 ms where
  the CPU takes about 400 ms — the difference between slow-motion preview being usable and
  not. Both are tested against scenes with mathematically known motion (translations,
  rotations, checkerboards, a sliding square's occlusion) and against a plain crossfade
  (sharper on textured motion). The
  frame-pick and each interpolation are shared functions used by *both* preview and export, so a
  slow-mo frame is identical in each — the preview-equals-export promise holds for interpolation
  too. The same Frames toggle appears per-clip on Sequence layers (next to Clip speed %), so a
  single slowed clip can flow-interpolate while its neighbours stay crisp.
  One knob worth knowing about lives in the Flow group: **Input rate**. High-speed footage —
  say a 600fps phone clip — is a trap for flow, because its frames are so close together in
  time (under two thousandths of a second apart) that there's essentially no motion between
  neighbours to interpolate; flow slow-mo of it looks frozen. Input rate fixes that: tell
  flow to *treat* the clip as, say, 24fps, and it interpolates between frames a real
  twenty-fourth of a second apart instead — actual motion, actual slow-motion. Native (the
  default) uses the clip's own rate. It's the same "conform to N fps" idea editors know from
  interpreting footage in other tools, and because it changes which frames get blended, it's
  folded into the picture cache's identity so you never see a frame flowed at the wrong rate.
  **This is wired up for
  Footage layers now**: a Speed % box in a footage layer's twirl-down retimes it (50% =
  half speed, and so on), and the same Retime map feeds preview, export, and the cache
  key — so a retimed clip previews, exports, and caches consistently. The Speed box is a
  ramp: a start speed → an end speed with an ease (Linear/Slow/Fast/Smooth/Sharp), so a
  clip can rush in and settle — the core montage gesture — not just play at one flat rate.
  When a retime speeds a clip up so much that it runs out of footage, `overrun_local_time`
  reports the exact moment it runs dry — the point where the last frame gets held rather
  than inventing more footage. The Timeline draws that held tail on the layer's bar: a
  faint kraft wash with diagonal hatching over the span, a thin kraft line at the exact
  frame the source runs out, a small `HOLD` tag when there's room, and a tooltip when you
  hover it ("Source ends here — holding the last frame"). Kraft, never a red alarm — house
  rule: a held frame is legal and well-defined, you just need to see it. Right-clicking the
  clip offers **Trim to source end** to cut it there. It never trims for you (boundaries must
  stay put so cuts keep landing on the beat). Sequence layers, the graph-editor lenses, and
  per-beat cutting come next.
- `crates/lumit-core/src/sequence.rs` — **Sequence layers (the model).** A Sequence layer
  is one timeline row holding clips laid end to end — Lumit's Vegas-style editing surface.
  Each clip points at a source, carries its own trim and its own Retime ramp, and sits at
  an exact place on the row; clips never overlap and a gap shows through transparent. This
  file answers the one question the renderer asks — "which clip is under the playhead, and
  which moment of its source does that map to?" — and checks the no-overlap rule. Drawing
  those clips is now wired: a Sequence layer (Composition → Add sequence layer — it starts
  from the selected footage as one clip) renders whichever clip is under the playhead
  through the same footage decode path as a plain footage layer, so its clips preview,
  export, and cache like any other source. You can **cut** a clip at the playhead
  (Composition → Cut clip, or ⌘⇧D / Ctrl+Shift+D) — it splits into two clips whose
  speed ramps exactly partition the original, and neither clip moves (the beat-sync
  covenant). Crucially, a clip's first frame is always its own trim-in whatever its
  speed, so splitting and re-speeding the second half never shifts where it starts.
  Cutting through a *curved* (eased) ramp works too: behind the scenes each half is converted
  to the exact After Effects-style bezier curve form (docs/04-RETIMING.md §5.1/§5.3), so the
  motion is preserved to the frame — only a constant-speed or straight linear ramp stays a
  plain speed ramp after the cut.
  You can also **delete the clip under the playhead** (Composition → Delete clip at
  playhead), which leaves a gap — the Vegas surface allows gaps, and a gap simply renders
  transparent. **Click a clip to select it** (it highlights in clay) and set its **Clip speed
  %** in the layer's twirl-down: the clip keeps its exact place on the layer — its edit points
  don't budge, honouring the beat-sync covenant — and only the stretch of source it consumes
  changes (that maths is `Clip::with_speed`, unit-tested). A non-100% clip shows its speed on
  its bar. Dragging more clips in and per-clip trimming are the next steps.
  You can also **right-click a footage layer → Convert to sequenced layer** (K-071): it
  becomes a single-source layer bound to that one clip — a "fancy precomp" you'll soon
  open in its own editing tab to cut and retime, where a camera track (run once on the
  full footage) can follow the edits. For now it converts in place, keeping the layer's
  id, transform, masks and any speed you'd set.
- `crates/lumit-core/src/store.rs` — **The document store**: applies ops, publishes
  snapshots, keeps the undo/redo stacks.
- `crates/lumit-project/src/lib.rs` — **`.lum` files.** A `.lum` is a zip containing
  readable JSON (rename one to `.zip` and look inside — genuinely). Saves are atomic:
  written to a temp file, flushed to disk, then renamed over the old file, so a crash
  mid-save can never destroy the previous save. The **journal** logs every edit to a side
  file the instant it happens; after a crash, replaying it restores your work.
- `crates/lumit-media/` — **reading media files** (via FFmpeg, the industry-standard
  media library). Two jobs so far: the *probe* (a file's vital statistics — resolution,
  frame rate, duration — shown under each item in the Project panel) and the *frame
  index* — a scan of the whole file that records where every frame and keyframe sits, so
  scrubbing can land on exactly the right frame. Indexing runs on a background thread
  (the UI never waits) and the result is cached on disk, keyed by a *fingerprint* of the
  file's content — change the file and the stale index is ignored automatically.
- `crates/lumit-gpu/` — **the colour foundation.** All engine maths happens on
  "light-linear" values (where adding two lights behaves like real light); files and
  screens use sRGB encoding. This crate owns the only two crossings between those worlds
  — decode-side linearise and display-side encode — and a "golden" test proves every
  possible 8-bit value survives the round trip within one step. That test is what makes
  the washed-out/too-dark "double gamma" class of bug impossible to reintroduce, and it's
  the bedrock of the preview-equals-export promise (K-031). The clever part: the shader
  contains no gamma maths at all — the GPU's texture formats do the conversions in
  hardware, so decode and encode can never drift apart.
- `crates/lumit-gpu/src/composite.rs` — **the compositor seed.** Each layer is a picture
  on glass; the compositor stacks the glass on the GPU. Position/scale/rotation move each
  sheet (already as full 4×4 matrices, so 3D later needs no rewrite), opacity fades it,
  and stacking happens in linear light where combining images behaves like combining real
  light — a test proves the result differs from the naive approach by exactly the amount
  physics predicts. This is the beginning of the evaluator: the thing that will one day
  render whole comps with effects.
- `crates/lumit-gpu/src/oklab.rs` — **perceptual colour.** Two colour worlds, two jobs:
  linear RGB is where *light* combines correctly (layering, glow, exposure), and Oklab is
  where *perception* behaves — a gradient interpolated in Oklab stays vivid where an RGB
  gradient sags into grey, and rotating a hue in Oklab keeps its brightness. Lumit
  converts on the fly (a handful of multiplications per pixel), users never see anything
  but normal RGB values, and tests pin both promises: round-trips are exact and hue
  rotation provably never changes lightness.
- `crates/lumit-cache/` — **the cupboard with a size limit.** Rendered and decoded
  frames get remembered so they're never computed twice; when the cupboard is full,
  whatever was used longest ago gets thrown out first. The limit is in bytes, not item
  counts — one 4K frame costs what sixty thumbnails cost, and budgeting any other way is
  how apps balloon.
  As of the disk tier (`disk.rs`), frames also get **parked on disk**: once a project is
  saved, a `yourproject.lum-cache` folder appears beside it and rendered frames are quietly
  written there (compressed) by a background thread — so closing and reopening a project
  doesn't start the cache from zero, and frames squeezed out of RAM can come back without
  re-rendering. Each frame is one small file named by its content fingerprint; anything
  unreadable is silently deleted and re-rendered, so the folder is **always safe to delete**
  — it can make things faster, never wrong. The idle background fill now checks the disk
  before rendering: promoting a parked frame beats recomputing it. The timeline's cache bar
  grew a second colour for this: **mint** = in memory, plays right now; **blue** = parked on
  disk, ready to promote.
  The third tier is **VRAM**: the last few hundred megabytes of frames you actually looked
  at stay resident on the graphics card, so scrubbing back over them re-shows the exact
  texture with zero work — no upload, no colour maths. All three tiers answer to the same
  content fingerprint, so a frame is a frame wherever it lives.
- **Timeline guide lines** — the faint vertical lines through the lanes have a mode picker
  in the bottom bar ("Grid"): **beats** (the default — detected beats shine through every
  layer so cuts land on the music), **time** (a neutral second grid that subdivides as you
  zoom in, down to 10 ms), or **off**. The bright ruler ticks up top stay regardless.
- `crates/lumit-ui/src/export.rs` — **writing video files.** Every frame of a comp is
  rendered through the *exact same* colour engine and compositor the Viewer uses, then
  compressed to an .mp4. Using one shared path isn't laziness — it's the design's central
  promise (what you preview IS what you export), and it runs on its own worker so the app
  stays responsive while exporting, with live progress and a real cancel. The **export
  dialogue** offers presets — *YouTube 1080p60*, *YouTube 4K60*, *Vertical 1080×1920p60* —
  which are just rows of numbers (frame size, codec, bitrates) stamped into fields you can
  still edit, so the custom path is always open. Presets are pinned by a unit test, so a
  stray edit can't quietly change what "YouTube 1080p60" means. When the comp's shape
  differs from the preset's, Lumit fits the picture keeping its proportions and adds black
  bars (a wide comp gets bars top and bottom in a vertical export); the fitting maths
  (`fit_contain` / `letterbox_resize` in `pixels.rs`) is unit-tested. **Sound comes too**:
  the comp's audio is mixed by the very same code that plays it back (one shared `mixdown`
  — playback, beat detection, and export literally cannot hear different things), then
  written as an AAC track fed to the file in step with the picture, a video frame's worth
  of samples at a time, so players never see sound and image drift. Exports now **queue**:
  ask for another while one runs and it waits its turn, each item frozen exactly as the
  project stood when you queued it — later edits never sneak into a queued export. The
  status bar shows which file is exporting, how far along it is, which encoder is doing the
  work, and how many items wait; one failed item never stalls the rest.
- `crates/lumit-media/src/encode.rs` — **compressing the file, and how export picks an
  encoder.** Compressing video is heavy work, and every GPU vendor ships a dedicated chip
  for it: NVIDIA calls theirs NVENC, AMD has AMF, Intel has Quick Sync. They are far faster
  than doing it on the CPU, but temperamental — a machine can have the NVIDIA *software*
  installed with no NVIDIA card present, or the card can refuse because too many programs
  are already encoding. So Lumit works down a ladder: try NVENC, then AMF, then Quick Sync,
  then plain software (x264/x265, always works). And it doesn't just ask "are you there?" —
  it *proves* each rung by encoding sixteen blank frames at the export's exact size before
  trusting it, because these chips are notorious for saying yes and failing a moment later.
  Whichever rung passes first does the export, and the finished dialogue tells you which
  ("Encoded with NVENC"). The ladder order and the fallback rule are plain data plus a tiny
  pure function, so the "hardware exists but won't open" cases are ordinary unit tests, and
  one integration test runs the real ladder on whatever machine the tests run on. The same
  module now also writes **HEVC** (H.265 — newer, smaller files than H.264 at the same
  quality) and an **AAC audio track**, interleaved with the video the way streaming players
  expect, with a `+faststart` flag so the file's table of contents sits at the front and
  playback can begin before the download ends.
- `crates/lumit-audio/` — **playback and the clock.** The sound card asks for samples on
  its own strict schedule through a "realtime callback" — a tiny function that must never
  wait for anything (if it's ever late, you hear a glitch). The count of samples it has
  played *is* the playback clock: video asks "what time is it?" every frame and shows
  whatever frame matches. One clock, owned by the audio hardware — that's why picture and
  sound can't drift apart, and it's the same design the full engine keeps forever.
- **Composition audio and playback** (`lumit-audio::mix`) — pressing Space on a comp now
  plays it. A comp can have many layers that make sound, each starting at its own moment;
  to play it, Lumit decodes each one and lays them on a single strip at the right offset
  and trim, then adds them together (a mixing desk summing channels — `mix_stereo`). That
  one mixed track goes to the sound card, and its clock drives the picture, so a comp's
  video and audio stay locked exactly like a single clip's. The mixing happens on a
  background thread so pressing Space never stalls; a silent comp just plays on a plain
  timer instead. This retires the old stopgap where comp playback guessed the time from a
  wall clock.
- **Beat detection** (`lumit-audio::beat`) — the groundwork for cutting to the music. It
  slides a short window along the track and, at each step, measures how much *new* energy
  appeared since the last step (the "spectral flux"); a kick or snare makes that number
  spike, and the spikes are the onsets. Autocorrelating the spikes recovers the tempo (BPM),
  preferring the sensible 70–180 range so a fast track doesn't report double-time. A
  sensitivity dial trades more markers for fewer. It's the standard, well-understood
  approach done carefully — no AI guesswork — and it's tested against synthetic clicks at a
  known tempo (every beat found, tempo within 2 BPM). A **grid assist** (`snap_to_grid`) then
  nudges any beat that's within ~45 ms of the tempo grid exactly onto it — the grid's phase
  is worked out from the beats themselves — which tidies away the small, unavoidable delay in
  raw onset detection so markers land where a musician would tap. Onsets that fall well off
  the grid (syncopation, fills) are left where they are.
- **Markers** (`lumit-core::markers`) — a marker is a labelled flag at a moment on a
  composition's timeline. Three kinds: ones you place (User), chapter divisions, and the
  Beat markers Lumit detects from the music (each with a confidence). Re-running beat
  detection replaces only the Beat markers, so cues you dropped by hand are never disturbed.
  `snap_time` returns the nearest marker within a threshold (else the original time) — the
  basis for cuts landing exactly on the beat. All of this is exact-rational and unit-tested.
  In the app, **Composition → Detect beats** mixes the comp's audio on a background thread,
  runs the detector, and drops a Beat marker on every onset (re-running replaces only those,
  never your hand-placed cues). The markers show as clay ticks on the timeline ruler — faint
  or bright by confidence — and scrubbing the playhead snaps to a nearby marker, so you land
  on the beat.
- **The timeline waveform** — a strip under the ruler draws the composition's mixed audio as
  a min/max envelope on the same time axis, so the beats sit right above the transients that
  made them. It's built by `waveform_peaks` (in `lumit-audio::mix`), which buckets the mono
  mixdown into (min, max) pairs — a pure, tested down-sample — computed once when the comp's
  audio is mixed for playback.
- The **graph editor** — a toggle in the Timeline's bottom-right corner, like After
  Effects' graph button. Switching it on does not replace the timeline: the layer outline
  on the left, the ruler, the scrollbars and the bottom bar all stay exactly where they
  were, and only the lane area on the right (where the layer bars normally sit) swaps to
  the selected property's live curve. Twirl a layer open and click a property's name to
  choose what the curve shows (a retimed footage layer's "Speed %" row graphs its Retime
  channel); on the curve you drag the keyframes (value and time together, one
  undo per drag), double-click the background to add a key, right-click a key for a menu
  (Easy ease / Linear / Hold, or Delete). Each key's shape tells you its interpolation at a
  glance — a diamond is linear, a circle is eased (bezier), a square is a hold.
  You can also edit **several keyframes at once**: drag a box over the curve's empty
  background (a *marquee*) and every key inside it is selected, shown with a ring around it.
  Dragging any selected key then moves the whole selection up or down by the same amount —
  values only, times stay put — as one undo step; dragging a key outside the selection
  drops the selection and moves just that key, as before. With two or more keys selected,
  *typing* a number into that property's value field in the layer outline sets every
  selected key to exactly that value (dragging the field keeps its usual one-value
  behaviour). A plain click on the graph's background, or switching to another curve,
  clears the selection. Under the bonnet the selection remembers each key by its position
  *and* its time, so if anything else edits the curve underneath, the selection simply
  clears rather than ever grabbing the wrong keys.
  The curve you see is sampled from the same evaluator that renders the comp, so what the
  graph shows is exactly what plays. There are two ways to look at any property: the **value**
  view (the raw number over time) and the **speed** view (its rate of change — the
  derivative). Both are editable, and they are the *same* data seen two ways (K-070): in the
  speed view you drag a key up or down to set how fast the value is moving at that moment,
  which is often the easier way to make motion feel right. Editing one view updates the other.
  The speed curve is the *exact* derivative of the value curve (K-080), so any bezier shaping you
  give a key in the value view carries straight across: an eased key that starts and ends slow
  shows in the speed view as a smooth hump, a straight run shows as a flat line, a hold as zero.
  You can shape a key from *either* view (K-081): click a key in the speed view to select it and
  the same **gold tangent handles** appear, here drawn as horizontal ease bars — drag a handle up
  or down to set that side's speed, and left or right to set its influence (how long the ease
  holds). It edits the very same curve, so the value view updates in step.
  The value/speed switch lives in the timeline's **bottom bar** (its own little group next to
  the zoom buttons, shown only in graph mode) rather than in each curve's header, because it
  is one setting shared by every curve. The plot also carries a small **y-axis**: a few faint
  gridlines down the left edge labelled with the value at that height — degrees or per cent
  for the transform properties, `HH:MM:SS:FF` timecode or per cent for a Retime channel, and
  units-per-second in a property's speed view. And the graph **follows your edits**: adding
  or changing a keyframe anywhere in the timeline — a stopwatch click, scrubbing a value in a
  property row, dragging a key — selects that layer and points the graph at that channel, so
  the curve you see is always the one you just touched.
  **Panning and zooming the graph (K-079).** The graph now shares the timeline's time axis, so
  **Alt + wheel** zooms and **Shift + wheel** (or a horizontal wheel) scrolls the curve left
  and right in step with the layer bars. Up and down, the value view **auto-fits** by default —
  and the fit covers the whole editable picture: a bezier that overshoots its keys stays fully
  on screen, and so do the tips of every key's gold tangent handles, so a steep handle never
  pokes out of view (and because the fit reads every key's handles, not just the selected
  ones, selecting a key never makes the view jump). The **Fit** button in the bottom bar is a
  toggle: while it is lit the graph keeps re-fitting as the curve changes; click it while lit
  to freeze the view exactly where it is. A plain wheel over the graph scrolls it vertically
  and **Ctrl + wheel** zooms the value range toward the cursor; either one switches the toggle
  off and takes over, and clicking **Fit** back on drops that manual framing and resumes
  fitting. While you hold a manual framing, resizing the timeline panel keeps the graph's
  *scale* rather than its range: making the panel taller reveals **more** of the value range
  about the same centre instead of stretching the curve (auto-fit simply re-fits to the new
  height, as you'd expect). Because the graph fills the lane area and the layer
  outline sits to its left, a wheel over the graph moves the *graph* while a wheel over the
  outline still scrolls the *layer list* — the two scroll independently.
  **Shaping a key (bezier handles).** New keys are **linear** — straight lines in, straight
  lines out. Select a key (click it, or marquee several) and press **F9**, or the **Bezier**
  button in the bottom bar, to *easy-ease* it — After Effects' smooth default. A bezier key
  grows two short **gold handles** in the value view, one reaching back toward the previous
  key and one forward toward the next. Drag a handle to shape the curve: how steeply it leaves
  the handle sets the **speed** there, how far the handle reaches sets the **influence** (how
  long that ease holds sway). By default the two handles are **unified** — they stay in a
  straight line through the key, so the motion glides through smoothly. When they are unified,
  moving one handle rotates the other to stay opposite it, but the *other* handle keeps the
  length you last gave it — it only pivots, it never grows or shrinks as you swing the one you
  are holding — and "length" here means what you see: the partner keeps its on-screen pixel
  length whatever the axes' units or zoom, however steep the drag. The handle you are dragging
  simply follows the cursor: no snapping to vertical, no sudden lengthening near the top or
  bottom. While a handle is being dragged the graph's y-axis **holds still** — the view only
  re-fits once you let go, so the curve isn't sliding under your cursor mid-shape. And however
  wild the curve gets, it stays inside the graph: it never paints over the ruler, the layer
  outline, or the bottom bar.
  **Alt-drag** a handle to *break* it and shape the two sides independently (a corner). The
  break is decided per drag and it *sticks*: once you've started moving with Alt held you can
  let go of Alt and the handles stay broken. The same gesture reverses it — **Alt-drag a broken
  handle** and the pair re-unifies, snapping collinear again. (Right-click → **Unify handles**
  still works too.) The **Linear** button (bottom bar) straightens
  the selected keys again, and its neighbour the **Hold** button *steps* them — the value
  freezes at the key and jumps to the next one only when the playhead reaches it, never
  blending in between (a square key; the discrete choice a File param uses). Right-clicking a
  key still offers the same Easy ease / Linear / Hold / Delete. Whatever the handles, the
  curve always passes exactly through the keys.
  **A file parameter** (K-111) — some effects need a *file* rather than a number, a colour LUT
  being the first. Its row in Effect Controls shows the chosen file's name and a **Select…**
  button that opens the usual file picker, filtered to the kind the effect wants (a LUT shows
  only `.cube` files). Until you pick one the effect does nothing — a LUT with no file loaded
  simply passes the picture through. A file can even be *animated*, but only as a **hold** step:
  you keyframe which of a few files is showing when, and it switches at each key rather than
  trying to cross-fade between two files (which would be meaningless) — it reuses the very same
  hold keyframe described just above, so a file animates with the same tools as everything else.
  The **marquee works in both views**: drag a box over the speed view's background and the
  speed points inside it are selected, just like value keys.
  The **Retime channel's Velocity lens** can now edit *eased* ramps too: a ramp shaped with
  the Slow/Fast/Smooth/Sharp presets shows a small **square handle** where two ramps join —
  drag it up or down to set the speed at that join, and both neighbouring ramps re-aim to
  meet it while keeping their easing shapes. (Round handles remain the plain keyframes of
  un-eased ramps, as before.)
  A **footage layer** also carries a **Retime channel** here, named for the lens you are in
  (K-076): **Time** in the value view, **Velocity** in the speed view. In the **Time** lens it
  is now *exactly* an ordinary property graph (K-078): the curve is the source position (in
  seconds of footage) over the clip's own time — "which moment of the footage is on screen
  here", After Effects' *Time Remap* — and it edits with the same tools as Position or Scale.
  Keys drag, double-click adds one, and you can shape each with the same **gold bezier
  handles** and **F9** easy-ease as any property; the view auto-fits to the curve. A straight
  line is a constant speed, a curve is a speeding-up or slowing-down. A stopwatch turns
  keyframing on (adding a key that holds the source frame showing at the playhead); enabling it
  always yields at least the start and end keys — press the stopwatch with the playhead at the
  layer's very start or end and those endpoint keys simply appear (the stopwatch still lights;
  nothing is silently skipped). In the **Velocity** lens the same channel reads playback speed
  per cent, and dragging a point authors a ramp — the Vegas gesture, still its own bespoke
  editor with the ramp presets. They are two views of one store: shaping the Time curve with
  handles re-expresses the whole channel in After Effects terms, so any eased speed ramp you
  built in the Velocity lens is replaced by explicit value tangents once you drag a Time
  handle. The channel opens to the Time view by default; a "Vegas" tick makes it open to
  Velocity. (Time values show as plain seconds for now, like any property's axis — a proper
  `HH:MM:SS:FF` timecode readout is still to come. A *held* Time key — freeze then jump — also
  isn't distinct yet; a Hold there reads as a straight line.)
  (Frame interpolation — how in-between frames are synthesised, Nearest / Blend / Flow — is a
  per-layer retime setting in the data model, but is not surfaced in the timeline for now; it
  will return in a dedicated place.)
- **Property rows in the Timeline** (K-072) — twirl a layer open and each of its animatable
  properties (Position, Scale, Rotation, Opacity, and the 3D ones) gets its own row: on the
  left a stopwatch to turn animation on or off, the property's name, and its current value;
  on the right, along the same time ruler as the layer bars, a little diamond at each of that
  property's keyframes — so you can see *which* property is keyed *when*, not just that the
  layer has keys somewhere. Click a property's name to open its curve in the graph view.
  Once a property is animated its row also carries a **keyframe navigator** — `◄ ◆ ►` — where
  the middle button adds a key at the playhead (or removes the one already there) and the
  arrows jump the playhead to the previous or next key, so you can walk a property's keys
  without hunting for them by eye.
  (When the layer is twirled shut, the layer bar still shows a summary of all its keys.)
  Scale is special: by default x and y are locked together on a single "Scale %" row that
  keeps their ratio as you drag; the 🔓 button unlocks them into two separate rows for
  independent editing, and 🔗 re-locks. (Re-locking keeps whatever ratio the two currently
  have and loses nothing — a small, friendlier deviation from the original "relinking may
  discard one axis" idea.)
  **Position and Anchor come linked by default too, but in a different sense**: one
  "Position" row (and one "Anchor" row) carries *two* value boxes, x then y, exactly as
  After Effects shows a 2D position. Unlike Scale there is no ratio lock — dragging x never
  moves y; the link only merges the row's furniture. The shared stopwatch animates or
  freezes both axes together as a single undo step, the shared keyframe navigator walks the
  union of both axes' keys (its diamond adds a key to *both* axes at the playhead, or clears
  whatever keys sit there on either axis), clicking the name opens the x channel in the
  graph, and the lane shows both axes' diamonds. The chain button splits them into the old
  separate "Position x" / "Position y" rows when you want to walk one axis's keys on its
  own, and a "Link position" row underneath joins them back up. The choice is remembered
  per layer for the session, and nothing about the project file changes either way — it is
  purely how the rows are drawn. A selected sequence clip's **Speed %** is a full ramp — a start
  and end speed with an ease (Linear/Slow/Fast/Smooth/Sharp), equal ends being a plain
  constant — so a single clip can rush in and settle; cut a clip into pieces and ramp each to
  build the classic ramp-freeze-ramp velocity edit, edit points staying on the beat
  (`Clip::with_ramp`, tested). Footage layers also get a **Speed %** row with the same stopwatch:
  turn it on and speed becomes keyframable, so you can slow-mo one moment and speed through
  another. Under the bonnet each speed keyframe becomes a segment of the retiming curve (a
  straight speed ramp between keys); the frame-accurate maths that keeps cuts on the beat is
  the same engine described above. Curved (eased) speed ramps are still the graph editor's job.
- **Getting around the Timeline** — the panel is split into the **layer outline** on the left
  (the stack of names, stopwatches and toggles) and the **lane area** on the right (the time
  ruler with each layer's bar on its own *lane*). Each bar wears its layer's identity colour:
  a 3px tab on its left edge plus a very faint tint across the fill, so a tall stack reads at
  a glance — footage is steel, sequences indigo, precomps plum, solids neutral grey, text
  parchment, cameras dry gold. The colours are deliberately muted siblings, so the clay
  selection colour still beats all of them. Drag a layer's bar body to slide it earlier
  or later in time (one undo per drag). Every drag in the lane area — moving a bar, trimming
  an edge, scrubbing the ruler — follows the cursor one-for-one at any zoom, and the small
  "magnetic" pull towards nearby markers stays the same ~6 px on screen however far in you
  are (both used to speed up with zoom, which felt like the timeline slipping out from under
  the mouse); a twirled-open layer's keyframe diamonds line up under its bar at any zoom
  too. The twirl — the little triangle at the far left of each layer row that opens
  its property rows — is drawn at a readable size and brightens under the cursor, so it is
  findable rather than a four-pixel smudge. Zoom the time ruler with **Alt + wheel** — it zooms
  toward the cursor so the frame under the pointer stays put — and scroll it with **Shift +
  wheel** (or a trackpad's horizontal wheel); a plain wheel scrolls the rows up and down. Along
  the bottom of the lanes sits a small contained bar: `−`, `+` and **Fit** with the current
  zoom per cent on the left, the Layers/Graph view toggle on the right, and a draggable
  horizontal scrollbar just above it (the vertical scrollbar stops above the bar so the two
  never fight). Layers/Graph is only a change of what the lanes *draw* — the outline stays
  identical between the two, so twirling a layer open shows the same rows either way.
- The **2.5D camera** — the parallax tool. Every layer has a z position and x/y
  rotations alongside the flat transform; they sleep until you switch the layer to 3D
  (the "3D" toggle in its twirl-down) *and* the comp has a Camera layer
  (Composition → Add camera layer). The camera follows the After Effects model: its
  *zoom* is a focal distance in comp pixels, and a layer sitting at z = 0 draws
  pixel-for-pixel exactly as it did flat — so turning the system on changes nothing
  until you actually move something in depth. Push a layer back (positive z) and it
  shrinks by zoom ÷ (z + zoom); move the camera and near layers slide faster than far
  ones — that's parallax, the flow style's second-most-used trick after speed ramps.
  The topmost visible Camera layer wins when there are several (AE's rule), everything
  on it keyframes like any other property, and the maths lives in one place
  (`camera_matrix` in the GPU crate) shared by preview and export, so a camera move
  can't look different in the exported file. A regression test proves both promises:
  z = 0 maps 1:1, and depth scales exactly as the formula says.
- **Adjustment layers** (Composition → Add adjustment layer) — a comp-sized layer with no
  picture of its own: its effects apply to *everything beneath it* on the stack, so one
  colour balance or blur can treat a whole composite at once (K-091). How it works, in
  kitchen terms: when the render reaches an adjustment layer, it takes a snapshot of
  everything cooked so far, runs the layer's effect stack on that snapshot, and then blends
  the treated and untreated versions back together. What controls the blend is *coverage* —
  draw masks on the adjustment layer and only the masked region gets the effects; lower the
  layer's opacity and the effects fade partway; move or scale the layer and the affected
  *region* moves, never the picture itself. Add the Transform effect to one and you can pan,
  rotate or zoom the whole composite below — the punch-in trick the effect was built for.
  Both the preview and the export walk the exact same staging code (and every effect runs
  through one shared "run the stack" routine, `fxops`, so a new effect wired up once works
  in the preview, in exports, and on adjustment layers with no extra plumbing). One honest
  limit: a live adjustment layer inside a collapsed precomp quietly turns the collapse off
  for that precomp (the switch dims) — its effects must see only its own comp's contents,
  which splicing into the parent cannot honour. It still reuses the solid's glyph for the
  moment; a distinct icon is a small later touch.
- **The window layout** (K-074, refined by K-086) — the picture (the Viewer) fills the middle
  with nothing above it: no tab, no strip, just the image. Around it sit the other panels:
  Project and the effect panels stacked as tabs on the left, scopes on the right, the
  Timeline along the bottom. A panel only shows a little title tab when it shares its spot
  with other panels; a panel sitting alone — the Timeline, scopes — is as bare as the Viewer,
  so there is no needless "Timeline" label above the timeline any more (K-086). Stack two
  panels together and the tab bar appears by itself; drag a tab to move a panel somewhere
  else — beside another panel, stacked as tabs, above or below — and drag the edge between
  two panels to resize. Tabbed panels keep the small pop-out button that lifts them into
  their own separate window, and dragging a tab does the moving; a bare panel has no tab bar
  to carry either, so it gets its own pair of affordances (owner request): **right-click
  anywhere empty in it** for a "Pop out into its own window" menu (the Timeline's existing
  right-click-the-comp-strip pop-out still works exactly as before — it is the same
  mechanism, just no longer a special case), and **a small grip in its top-right corner** to
  drag it to a new spot, the same as dragging a tab would. The grip sits in its own tiny
  corner rather than spreading the drag gesture across the whole empty top strip, because of
  an egui quirk worth knowing if you touch this code: a region that senses dragging does not
  automatically step aside for an ordinary button drawn on top of it the way a plain click
  does — dragging is tracked per-widget from the moment the mouse is pressed, not by "whoever
  is visually on top" at release, so a wide drag-sensing strip sitting *underneath* a panel's
  own buttons could reach in and steal an ordinary click-and-slightly-move as a pane-drag
  instead. Keeping the grip small, and adding it *after* (visually on top of) the panel's own
  content, keeps it out of that trap. Closing any popped-out window drops the panel back
  where it was. A workspace saved before this change tidies itself the first time it loads.
  Under the bonnet this uses a "tiling" layout engine that, unlike the docking library we
  tried first, is happy to leave any lone pane without a tab bar.
- **The Scopes panel** (`shell/scopes.rs`, K-096) — the colourist's instruments. Instead of
  showing the picture, a scope plots its numbers: the **waveform** shows how bright each
  column of the image is (bright at the top, dark at the bottom), the **histogram** counts
  how many pixels sit at each brightness, and the **vectorscope** plots colour on a circle
  (hue as the direction, how vivid as the distance from the middle — a grey picture is a dot
  in the centre). Each Scopes panel shows one of these, picked from the little row of buttons
  at its top, so you can open a few side by side. It reads the frame you are looking at in
  the Viewer — the last one Lumit kept in memory. There is one honest limit for now (K-096):
  Lumit only keeps that frame in memory while you are paused or scrubbing, because during
  playback it skips saving frames to stay fast, so during playback the scope shows the last
  frame you stopped on and catches up the moment you pause. Reading the picture live while it
  plays needs the graphics card to do the counting, which is a later addition. The scope's
  own colours (the near-black background, the green trace, the red/green/blue channel
  colours) are fixed and the same in light or dark mode, for the same reason the Viewer's
  surround is a fixed neutral grey — you cannot judge an image against a background that
  keeps changing brightness.
- **The command palette** (`shell/command_palette.rs`, K-102) — press **Ctrl/Cmd+Shift+P**
  (or Window → Command palette…) and a search box appears with a list of commands under it:
  save, undo, new composition, add a layer, switch the colour scheme or panel shape, open
  Settings, export. Start typing and the list narrows to what matches — you don't have to
  type the words in full or in order, just the letters in sequence ("nc" finds "New
  composition"). Arrow keys move the highlight, Enter or a click runs the highlighted one,
  Escape closes. It is the fast way to reach anything without hunting through menus. It is
  not the effects radial menu (that is a separate, still-to-come tool for dropping an effect
  onto whatever is under the cursor) — this is the plain app-wide command list.
- **The Hierarchy panel** (`shell/hierarchy.rs`, K-102) — a foldable outline of the
  composition you are working on: its layers, and where a layer is itself another
  composition (a precomp), a little triangle folds it open to show that composition's own
  layers, and so on down. It is the map of a nested project — which composition is built
  from which — and clicking any row jumps you to that layer. It only shows the structure, it
  never changes it. It is the simple tree version of the fuller node-graph flowchart that
  comes later.
- The **Project panel** — AE-shaped (K-068): the selected item's details up top, the
  folder tree below, and drag-and-drop everywhere. Drag footage onto the Timeline or
  Viewer to make a layer; with no comp open yet, the composition dialogue appears
  already filled in from that footage. Solids are proper assets now — one "White solid"
  in the project can back fifty layers, and the first one you make creates a Solids
  folder that future solids follow even if you rename it or tuck it inside another
  folder (Lumit remembers the folder itself, not its name). Compositions do the same
  with a Compositions folder. Multi-step creations like that land as a single undo
  step — a batch operation whose inverse is just the reversed inverses of its members.
- **The evaluation graph (`lumit-eval::graph`)** — before rendering, Lumit lowers a
  composition into a wiring diagram: for each layer a short chain of typed steps — fetch the
  source, retime it, mask it, place it (transform), then blend it over everything beneath —
  ending in a single "comp output". It is built bottom layer first, exactly the order the
  picture is stacked up. The neat part is *folding*: a layer with no masks gets no mask step, a
  footage layer with no retime gets no retime step, so the renderer never spends a moment on a
  no-op. It also shares work: two layers on the same footage compile to a *single* decode step
  (keyed by the source, never the layer), so a duplicated clip is fetched once, not twice. The
  diagram is rebuilt whenever you edit, and every render already in flight keeps the
  diagram it started with, so an edit can never half-apply to a frame mid-render. Today this
  builds the render's *shape* (tests prove the folding and the bottom-first order); turning each
  step into pixels on the GPU is the next slice. This is the front half of **Nova**.
- **Epochs (`lumit-eval::epoch`)** — the cancellation mechanism the whole scheduler
  will stand on. Every scheduled job carries a ticket stamped with the number that was
  on the wall when it started; scrubbing or stopping turns the wall number over, and
  workers glance at the wall between small steps and quietly stop if their ticket is
  stale. Nothing is ever force-killed. A test proves a deliberately slow job stops
  within 15 milliseconds of the number changing.
- **The frame scheduler's brain (`lumit-eval::schedule`)** — the decision rules for
  smooth playback, written as plain arithmetic so tests can prove them. During playback
  Lumit renders frames ahead of the playhead onto a small shelf; each screen refresh
  takes the newest shelf frame whose time has come, quietly binning ones the clock has
  passed, and simply holds the last picture if rendering falls behind (sound never
  waits). How far ahead to render adapts to how slow frames have actually been, between
  8 and 16 frames. And in realtime mode, frames too slow for the frame budget drop to a
  coarser preview resolution within a frame or two, earning it back only after a
  sustained cheap stretch — quick to worsen, slow to improve, so the picture never
  flickers between qualities. None of the real machinery (threads, the audio clock, the
  GPU) lives here yet; this is the referee, and the players arrive later.
- **Preview resolution never changes where things are.** To keep the picture responsive,
  Lumit can decode footage smaller than its true size — and "Auto" resolution decodes at
  exactly the size the layer is shown on screen, so it gets sharper as you zoom in. That is
  purely a quality choice: a layer's *position and size in the composition* are always
  worked out from the footage's real pixel dimensions, not the shrunk-down preview copy. If
  they were ever worked out from the preview copy, a layer would appear to grow as you
  zoomed in — which is exactly the bug this rule exists to prevent.
- **Scrubbing shows a draft instantly, then sharpens.** While you drag the playhead (on the
  timeline ruler or the footage scrub bar), Lumit decodes a small, quick version of each
  frame so the picture keeps up with your cursor — the same "keep moving, drop quality" idea
  the playback engine uses. The instant you let go, it reloads that one frame at whatever
  resolution you've chosen (Full, Half, Auto…). The quick draft frames are shown but never
  saved into the frame cache, so the cache only ever holds full-quality frames, and the
  background pre-rendering pauses while you scrub so it doesn't compete for the disc and CPU.
- **Dragging a value — or a keyframe — updates the picture live.** When you drag a value like
  Position or Scale, the viewport follows your drag immediately, before the edit is written
  down. Dragging a keyframe in the graph editor does the same: the picture shows what the curve
  now gives *at the current frame* as you move the key. It can do this cheaply because moving or
  scaling a layer doesn't change *which* frame of the footage is shown — only where it sits — so
  Lumit keeps the last decoded frame and simply re-arranges it with your in-progress value each
  tick, no re-decoding. The moment you let go, the edit is committed as a single undo step and
  the frame re-renders normally.
- **Idle time is spent pre-caching nearby frames.** When you stop on a frame and aren't
  playing or dragging, Lumit quietly renders the frames around the playhead into the cache
  at your chosen resolution, so stepping or scrubbing to them is instant instead of waiting
  each time. It works outwards from the playhead but favours the frames *ahead* — roughly
  three ahead for every one behind — because that's usually where you're going next. It fills
  one frame at a time and any real request (a scrub, an edit) immediately takes priority.
  During playback it keeps warming *ahead of itself* too: the audio card's clock decides which
  frame to show and never waits, so whenever the frame under the playhead is already cached
  Lumit spends the spare moment decoding the next uncached frame a short way in front of the
  clock (about a dozen frames' lookahead). That's why the first pass over a cold section can
  stutter but the work-area loop settles into perfectly smooth playback once round.
- **Mask editing in the Viewer** — select a layer with masks and its outlines draw
  over the picture in clay, with a square handle on every vertex. Drag a handle and
  the outline follows your cursor live; let go and the pixels update — one undo step
  per drag, like every other edit. The maths mirrors the layer's transform both ways
  (screen position → layer space and back), so handles stay glued to the picture at
  any zoom, pan, scale or rotation. The Pen button in the Viewer bar arms
  click-to-place drawing: each click drops a vertex, clicking the first one (it grows a
  ring once closable) closes the shape into a mask, Escape cancels, right-click on any
  handle removes a vertex. Curved tangent handles are the remaining slice.
- **Origin (anchor point)** — every layer's transform now starts with Anchor x / Anchor y:
  the point the layer scales and rotates *about*, and the point Position places in the
  comp. New layers default it to the centre of their content and sit centred in the comp
  (the After Effects default), so a fresh clip spins about its middle rather than its
  top-left corner. The selected layer shows its origin as a small clay crosshair in the
  Viewer, and you can **drag that crosshair to move the origin** — the layer stays put
  while its pivot shifts (After Effects' "pan behind", position compensates automatically),
  committed as one undo step.
- **The tool strip** — the row of buttons under the menu sets what a Viewer drag does,
  the way every editor's toolbar does. Select (V) and Hand (H) both pan the view for
  now (object selection comes with the object tools); Shape (Q) rubber-bands a new mask
  — right-click the Shape button to choose rectangle, ellipse or star; Pen (G) is the
  click-to-place mask drawing above. The mode is one value (`ToolMode`) the Viewer reads
  each frame, so the whole app agrees on what the mouse is doing.
- **Masks on Precomp layers** — a masked transition can now wipe a whole nested comp,
  the flow staple. Pixel layers (footage, solids, text) get their masks applied on the
  CPU before upload; a Precomp's pixels only ever exist on the GPU, so its mask stack
  is rasterised into a little coverage texture instead and the compositor multiplies
  it in per-fragment. Same maths, two routes — a GPU test pins the texture route to
  the CPU one.
- **Collapse transformations (Precomp layers)** — normally a nested comp renders to its
  own little picture first, and the parent then moves/scales that picture: two rounds of
  resampling, and anything poking outside the nested comp's edges gets cut off. The
  **collapse switch** (the sunburst on a Precomp layer's row) removes the middle step:
  the inner layers composite straight into the parent, their transforms multiplied into
  one matrix, so content is resampled once and nothing clips at the nested bounds — the
  quality move AE users expect for scaled-up precomps. Some things genuinely need the
  middle picture (a mask on the Precomp layer, a blend mode, opacity below 100%, using
  it as a matte) — then the switch dims to say "set, but overridden". The undoable
  switch lives in ops like every edit; the cache knows collapse changes pixels, so
  toggling it re-renders.
- **Blend modes** — the full everyday set: Normal, Add, Multiply, Screen, Overlay,
  Soft light, Hard light, Lighten, Darken. Two families under the hood: Add and
  Multiply are physical light maths and run in linear; Screen, Overlay and the lights
  are the Photoshop-era formulas people know by eye, so Lumit runs them on encoded
  values (running them in linear is tidier maths and the wrong look). Lighten and
  Darken are a simple per-channel max/min where the distinction doesn't matter. Every
  mode is pinned to its textbook formula by a GPU test.
- **Colour depth, in one paragraph.** Lumit's frames are "half float" (fp16) in linear
  light. Unlike AE's 16bpc — which is integer maths that clips at 1.0 — half float
  keeps brightness above 1.0 (a glow can genuinely overshoot) and negatives, which is
  what people switch AE to 32bpc for. Depth is one project-wide switch (8 / 16 float /
  32 float — K-069): flip it and every comp and effect in the project renders at that
  depth, AE-style, via a small button at the foot of the Project panel. Full float
  doubles every frame's memory and roughly halves compositing throughput, so 16-float
  stays the default; the heavy maths inside effects can run wider internally either way.
- `crates/lumit-ui/src/theme.rs` — **the design tokens.** The only file allowed to contain
  colour values. Change a colour here, it changes everywhere. As of K-084 the look follows
  the *structure* of rerun.io's viewer (a data-tools app whose interface the owner likes):
  the app's background is nearly black, panels sit just above it, and menus float a clear
  step higher on a soft shadow; buttons have no borders — you can tell idle from hovered
  from pressed purely by how light their fill is; scrollbars are thin and solid; panel
  edges are single crisp 1px lines. The colours themselves (the clay accent, the cool grey
  family) are still Lumit's own — we borrowed the skeleton, not the skin.
  Five appearance controls live in the **Settings window** (K-098) — open it from
  **Window → Settings…** or **Ctrl/Cmd+comma**. That window is Lumit's application-settings
  surface, shaped like macOS's System Settings: a list of pages down the left (General,
  Appearance, Interface, Performance, Export), and on the right the chosen page's settings in grouped
  cards, a label on the left of each row and its control on the right. It follows the
  Sharp/Round look like everything else — rounded filled cards under Round, hairline-framed
  under Sharp.
  The **Appearance** page carries the theme controls (they used to sit in the Window menu):
  **Mode** switches the whole app between Dark and a new Light theme — one
  plain white for every panel on a soft neutral canvas, not a tinted panel per section (that
  idea is wanted, but saved for a future setting rather than built now); **Background**
  (only shown under Dark, since there's nothing to pick under Light) switches between the
  near-black ramp and the previous bluer one; **Accent** lets you pick any colour for the
  app's single accent — selection, the playhead, active states all follow it, since they are
  one token; **Shape** switches between the existing sharp, edge-to-edge look and a new
  Round shape — panels float as rounded cards with real gaps between them and the window
  edge, Figma-inspired, no blur or bevel, just a soft shadow standing in for the border; and
  **Animation** picks how much motion the UI's own chrome shows (All / Minimal / None) —
  this reaches things like a collapsing section's arrow or a dialog's fade-in, not (yet) the
  app's own dropdown menus, which don't animate at all today regardless of this setting. All
  five persist with your workspace; Reset returns the clay default for Accent.
  The **Performance** page of the same window is where you tell Lumit how hard to work your
  machine: how much memory its frame cache may hold, how much disk the on-disk cache may use,
  and how much video memory (VRAM, the graphics card's own memory) the cache of
  already-drawn frames on the GPU may hold. All three apply the moment you change them —
  nudge a budget down and the matching cache trims itself to fit at once. The defaults match
  what Lumit used before the page existed, so nothing changes until you move a slider. A
  **Clear cache** button underneath empties the memory and video-memory caches straight away
  (handy after a big edit, or if you just want a clean start) — the on-disk cache is left
  alone since clearing it would mean re-decoding footage from scratch. Beside it, a
  **Background fill** switch controls whether Lumit spends its idle moments quietly decoding
  the frames around wherever the playhead sits, so scrubbing nearby feels instant — switch it
  off and Lumit does nothing until you actually ask for a frame, trading that warm cache for a
  quieter machine when you're doing something else at the same time. On by default, matching
  what Lumit always did. Underneath that, a **Cache root folder** row shows where the on-disk
  frame cache currently lives — "Default (next to the project file)" until you change it — with
  a **Choose…** button that opens a folder picker and a **Use default** button that puts it
  back. This is for moving the cache off a slow or crowded drive: point it at a fast NVMe (or
  any other drive with room) and every project's on-disk cache is parked there instead of
  beside the project file, which also keeps a slow network or removable drive holding your
  project files from also taking the brunt of cache writes. Each project still gets its own
  cache folder under whatever root you choose — two differently-named projects, or even two
  projects that happen to share a file name in different folders, never collide. Changing this
  takes effect straight away, the next time Lumit notices the setting changed (well under a
  second): it does not require a restart or a re-open of the project. (More performance
  controls — CUDA acceleration, worker counts — arrive on this page as those systems gain their
  knobs.)
  The **Interface** page holds two controls that don't belong to a theme. **UI scale**
  is a slider from 75% to 200% that makes the whole app — panels, text, icons, everything —
  draw larger or smaller than your display's native scale, for a hi-DPI screen that reads too
  small or a projector that needs everything bigger; it applies the moment you move it, using
  egui's own zoom mechanism (the same one behind its built-in Ctrl+= / Ctrl+- zoom shortcut,
  here exposed as a persistent, saved preference instead of a one-off per-session nudge).
  **Show tooltips** is a single switch for every hover tooltip in the app at once — the icon
  names and shortcuts that pop up when you rest the pointer on a button. Both default to
  today's behaviour (native scale, tooltips on), so nothing changes for anyone until they visit
  this page.
  The **Export** page (K-119) holds two defaults for the export dialogue. **Default preset**
  is the preset that a plain "Export comp…" action starts from — pick a specific preset from
  the File menu's "Export preset" submenu instead and that always wins, regardless of what's
  set here. **Filename template** lets you write the suggested file name yourself instead of
  taking whatever the preset would otherwise call it, using three tokens: `{comp}` for the
  composition's own name, `{preset}` for the preset's usual file name, and `{date}` for
  today's date. Leave it blank (the default) and nothing changes — you get exactly the file
  name each preset always suggested. Whatever comes out is checked for characters Windows
  won't allow in a file name (like `:` or `/`, which a composition name could easily contain)
  and those get swapped out automatically, and the name always ends in `.mp4` even if you
  forgot to type it. Two rows from the fuller Export plan aren't here yet — export priority
  and which encoder to prefer — because nothing in Lumit today has a concept of either one to
  control; they'll appear once that machinery exists.
  The **General** page holds an **Autosave** group: how often Lumit quietly saves a spare copy
  of a saved project (in minutes) and how many timestamped copies it keeps, so a crash or a
  mistake never costs more than the interval. The defaults are the same 5 minutes / 5 copies
  Lumit always used; they are just adjustable now.
  The **focused panel** also wears a thin accent edge: whichever panel you last clicked is
  where keyboard shortcuts land, and the edge keeps that visible at a glance (the After
  Effects convention) — it follows the Round shape's card rounding too, when that's picked.
  Four more complete colour schemes live in `theme.rs` alongside Dark, Dark blue and
  Light (K-097): Gruvbox dark, Gruvbox light, Catppuccin Mocha and Catppuccin Latte, each a
  well-known palette from outside Lumit re-mapped onto its existing surfaces, text, accent and
  so on, rather than a new set of rules. All seven are picked from a single **Colour scheme**
  dropdown on the Settings window's Appearance page — the old separate light/dark and
  background-ramp rows folded into it. An older save that used the two-row picker migrates its
  choice into the new one automatically, so nobody's theme resets on upgrade.
- `crates/lumit-ui/src/icons.rs` — **the icons: Iconoir, shipped as a font** (K-085).
  Little pictures like the play triangle or the padlock come from Iconoir, a free
  professionally drawn icon family, baked into the program as a small font file — each icon
  is a character in that font, so it stays crisp at any size and always takes the theme
  colour (dimming on hover, turning accent when active) exactly like text does. Emoji are
  still banned: a glyph is either from this set or deliberately drawn, never a character we
  hope the user's fonts carry — that's how the invisible stopwatch/arrow bugs happened. To
  add one, add a name to the `Icon` list and its Iconoir name in the lookup; a test fails
  if the name doesn't exist in the set, so a typo can't ship.
- `crates/lumit-ui/src/shell.rs` + `app_state.rs` — **the window**: panels, menus,
  shortcuts, and the state glue (current project, dirty flag, autosave timer, recovery
  prompt).

## 5. Making a change safely (the recipe)

1. **Find the doc first.** Specs (`docs/00–16`) say what the behaviour should be; impl
   notes (`docs/impl/`) say how the hard parts work. If your change disagrees with a doc,
   the doc gets updated in the same commit — docs are canonical.
2. **Make the change.** The compiler is your ally: in Rust, most mistakes fail to compile
   rather than fail at runtime. Read its messages — they're unusually helpful and usually
   tell you exactly what to fix.
3. **Run `cargo test`.** Everything green? Your change didn't break any promise that's
   been made so far.
4. **Add a test for what you changed.** New behaviour = new test proving it. Fixed a bug =
   a regression test that fails without your fix (that bug can now never return unnoticed).
5. **Commit with a message saying what and why.** CI re-runs everything on every push.

Even if you never write the change yourself, this recipe is how you *direct* a model to do
it and check it did it right: point at the doc, ask for the change plus its test, look at
the test.

## 6. The testing philosophy (and your regression-coverage rule)

Standing policy, enforced in CI ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md) §tests):

- **Every feature lands with tests.** Not after — with. A feature without tests is not done.
- **Every bug fix lands with a regression test** that reproduces the bug first. The suite
  is a museum of every bug ever fixed, and none of them can come back silently.
- **Property tests** generate thousands of random inputs looking for edge cases humans
  don't think of (the time maths runs under these).
- **Golden tests** compare output against a known-correct reference — later, whole rendered
  frames get compared pixel-by-pixel, which is how "preview equals export" stays true.
- **Coverage is measured in CI** and the engine crates must stay above the threshold —
  it can only be raised, never lowered.

One budget deserves its own mention because it's the project's founding grievance: **the
interface must stay responsive with thousands of layers and hundreds of thousands of
keyframes** (the "stress document" budgets in
[13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md) §2.1). Two design rules deliver it: the
UI only ever draws what's visible on screen (so a 5,000-layer timeline costs the same as a
20-layer one), and the UI thread never does engine work. One known shortcut exists today —
saving a snapshot currently copies the whole document per edit, which is fine now and will
be replaced with "copy only what changed" before Phase 1 ends; it's recorded in the
performance rules so it can't be forgotten.

What the suite guards *today*: time maths exactness (6 property suites), undo/redo
symmetry, journal replay, the crash-recovery drill both ways, file-format round-trips,
unknown-field survival, autosave rotation, version refusal.

## 7. Words you'll meet in the code

| Term | Meaning |
|---|---|
| `fn` | A function |
| `pub` | Public — usable from other files/crates |
| `let` | Create a variable |
| `&thing` / `&mut thing` | Borrow it read-only / borrow it with permission to change |
| `impl X` | "Here are X's functions" |
| `#[derive(...)]` | Auto-generate boilerplate (comparisons, serialisation) |
| `#[serde(...)]` | Instructions for JSON conversion |
| `mod` / `use` | Declare / import a module |
| `Vec<T>` | A growable list of T |
| `HashMap<K, V>` | A dictionary/lookup table |
| `match` | A switch that must handle every case |
| `async` | Not used in Lumit's engine — we use threads and channels instead, deliberately |

When you hit something not covered here, ask any session "explain X in GUIDE.md terms and
add it to the guide" — that's the standing arrangement.

## 8. Building and running it on your machine

To turn the source into a running app you need the Rust toolchain and one outside
dependency: **FFmpeg**, the library that actually decodes and encodes video and audio.
Lumit doesn't reinvent that wheel; `lumit-media` talks to FFmpeg. So the build needs
FFmpeg present, and everyday `cargo` commands need to know where it is.

There are two moving parts, and it helps to know why each exists:

- **FFmpeg itself** — the video/audio engine. We use version 7.1. On Windows it comes as a
  folder with three important sub-folders: `lib` (the "how to call in" stubs the build links
  against), `include` (the description of what's callable), and `bin` (the actual `.dll`
  files the finished app loads while it runs, plus the `ffmpeg` command-line tool the tests
  use to make sample clips).
- **libclang** — a translator. FFmpeg is written in C, and something has to read FFmpeg's
  C descriptions and generate the matching Rust ones automatically. That translator is a
  piece of the LLVM toolchain called libclang. One gotcha, learned the hard way: use
  **LLVM 18**. A much newer LLVM makes the translator quietly produce nonsense (it turns
  whole data structures into blanks), and the build fails with confusing errors. Pinning 18
  avoids it.

### On Windows (the shipping platform)

1. Download `ffmpeg-n7.1-latest-win64-gpl-shared-7.1.zip` from the
   [BtbN FFmpeg builds](https://github.com/BtbN/FFmpeg-Builds/releases) page and unzip it
   under your user folder, e.g. `C:\Users\you\ffmpeg\`. (GPL because Lumit is GPL; "shared"
   because we want the `.dll` files.)
2. Install LLVM 18 and the Rust toolchain: `winget install LLVM.LLVM --version 18.1.8` and
   `winget install Rustlang.Rustup`. Rust's default Windows setup links with Visual Studio's
   C++ build tools, so having Visual Studio (or the standalone Build Tools) installed matters.
3. From the repo root, run `. .\scripts\win-dev-env.ps1 -Persist`. That one script finds the
   FFmpeg folder and LLVM, points the build at them, and (`-Persist`) remembers the settings
   so every future terminal already knows. The leading dot is required — it means "apply
   these to my current shell", not "run and forget".
4. Now the normal commands work: `cargo run -p lumit-app` to launch, `cargo test --workspace`
   to run the whole test suite.

### On macOS

FFmpeg comes from Homebrew: `brew install ffmpeg@7`. The repo's `.cargo/config.toml` already
points the build at it, and macOS ships the translator (libclang) as part of its developer
tools, so there's nothing else to set up — `cargo test --workspace` just works.

### On Linux (K-082)

Linux finds FFmpeg the same way macOS does — by asking the system's package registry
(`pkg-config`) where the libraries live — so the setup is: install the FFmpeg 7
*development* packages (the ones ending `-dev`, which carry the headers the binding
generator reads), plus `pkg-config` and `clang`. On Debian 13 or Ubuntu 24.10 and newer
that is one line: `sudo apt install pkg-config clang libavcodec-dev libavformat-dev
libavutil-dev libswscale-dev libswresample-dev libavfilter-dev libavdevice-dev`. On Arch:
`sudo pacman -S ffmpeg clang pkgconf`. Then `cargo run -p lumit-app` as usual.

One honest caveat: the build needs FFmpeg **7**, and some distributions still ship
FFmpeg 6 — Ubuntu 24.04 LTS is the big one. On those, `cargo build` will complain about
"ffmpeg stuff" (a version the binding doesn't accept, or missing headers). The fix is a
newer distribution release, or building FFmpeg 7.1 from source and letting `pkg-config`
find it. There is no Linux machine in CI yet, so treat Linux as "documented and expected
to work" rather than "verified on every push".

### What the robots check

Every push, CI rebuilds and retests everything on both macOS and Windows, media included, so
"it builds on my machine" can never quietly drift from "it builds for real". The Windows
recipe above is exactly what CI does, written out by hand in `.github/workflows/ci.yml`.
