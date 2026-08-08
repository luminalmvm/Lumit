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
| `lumit-render` | Making the picture | The whole path from "here is the project" to "here are the pixels" — decoding, compositing, caching, export |
| `lumit-media` | Decoding video | Turning an .mp4 into frames |
| `lumit-gpu` | The GPU pipeline | Drawing and processing frames on the graphics card |
| `lumit-audio` | Sound | Playback and the clock everything syncs to |
| `lumit-eval` | The render engine | Working out what each frame looks like |
| `lumit-cache` | Caching | Remembering rendered frames so they're never rendered twice |
| `lumit-flow` | Optical flow | Motion vectors for smooth-retime and flow motion blur |
| `lumit-text` | Text | Rasterising text layers |
| `lumit-keymap` | Keyboard shortcuts | What each key combination means, and what clashes with what |
| `lumit-bridge` | The Flutter seam | How the Flutter frontend talks to the engine (see [17-BRIDGE-CONTRACT.md](17-BRIDGE-CONTRACT.md)) |

Everything you actually *see* lives outside `crates/`, in `flutter_ui/` — the
Flutter application (panels, menus, the theme). The original egui shell
(`lumit-ui` + `lumit-app`) was deleted in K-182; if you ever need to look at how
the old frontend did something, it is one `git log` away. (`lumit-keymap` went
with it, as unused at the time, and came back unchanged in K-199 when the
shortcut editor was actually built.)

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

That rule is also why `lumit-render` exists. The picture-making code once lived inside the
frontend, which meant anything wanting a frame had to reach *through* a user interface to
get one — a dashboard wired into another dashboard. Pulling it into its own crate (decision
K-178) put it back where it belongs: the Viewer and the exporter now ask the same engine for
frames, so what you preview cannot differ from what you export.

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
  (Line period), optionally rolling them over time and alternating which half of
  each band darkens every other cycle for an interlaced-video feel — it has no hash and no
  Seed of its own, since it just reads straight down from each row rather than jumping
  around like Block glitch does. Each effect has its own Intensity dial that turns its own
  look up or down, and at 0 each is a guaranteed no-op — checked by a test — whatever Mix is
  set to. (Scanlines used to have *two* darken dials — Intensity and a separate Darkness —
  that multiplied together to do one job; they were merged into the single Intensity, which
  now simply means "how dark the dark lines get", 0 nothing and 1 fully black. An old project
  that still has the separate Darkness folds it into the one dial on load, so it looks the
  same.) The interesting engineering wrinkle, in Block glitch: which block "moves" and by
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
  so — like the flow fix earlier — you never get a stale, frozen trail. Echo now reaches back
  up to sixteen frames one frame apart (it was eight), fades each by a Decay you set, and
  offers a Mode menu that starts with two echo-only choices — Behind (each echo tucked behind
  the trail, ghosting) and In front (over it) — then a divider, then the everyday light-combine
  blends: Add, Screen, Multiply, Overlay, Soft/Hard light, Lighten, Darken, Difference,
  Exclusion, Subtract, Divide (Screen is the default for bright glowing trails). The old "Max"
  is just Lighten now, and the old "Normal" is the clearer "In front". One nuance worth knowing:
  these blends run in
  the same "linear light" the compositor adds light in, on the see-through-aware (premultiplied)
  trail — the right space for stacking glowing copies, and it keeps the CPU and graphics-card
  versions matching to the last bit. Old projects keep whichever mode they had. Wider/looser
  trails (a Spacing control) are still a follow-up, and the other effects that want
  neighbouring frames — motion blur that follows real motion, and the datamosh look — build on
  this same machinery (both explained further down).
- **Fast motion blur — blur that follows real motion** — a temporal effect (called **Fast
  motion blur** in the menus, to set it apart from the whole-scene *Motion blur* of the
  accumulation kind) that turns game capture (which has no natural blur — every frame is
  pin-sharp) into footage that streaks the way a real camera would. It builds on two things
  already in the box: Echo's "fetch a neighbouring frame" plumbing, and the optical-flow engine
  that powers slow-motion. The trick is to look at the current frame and the *next* one, work
  out how far every pixel moved between them (that's the flow — a little arrow for each pixel
  saying where it went), and then smear each pixel along its own arrow. Fast-moving areas get
  long streaks; still areas stay crisp — exactly what real motion blur does, and what plugins
  like RSMB sell. The flow is worked out during decoding, where both frames are sitting in
  memory anyway (the same place slow-motion computes it), and passed to the blur as a little
  motion-map image; the preview and the export do it the identical way, so what you see is what
  you get. **The tricky bit — no more blocky cut-outs (the FX-19 fix).** Guessing motion is
  unreliable where things appear, disappear, or cross an edge, and the old version simply didn't
  blur those spots — leaving hard, obviously-wrong seams between blurred and un-blurred patches.
  The fix hands the blur a second little map alongside the arrows: a *confidence* from 0 to 1,
  worked out by checking the forward arrows against the backward ones (they should cancel out;
  where they don't, trust is low) and then softened so it fades rather than jumps. The streak
  length is simply multiplied by that confidence, so an unreliable area *eases* toward no blur
  instead of cutting. Three knobs plus a viewer: **Shutter angle** (how long the "shutter" stays
  open — 180° is the film-standard half-frame smear; higher blurs more, up to a full 720°),
  **Samples** (how many steps to take along each streak — more is smoother but slower), and a
  **View** picker — leave it on *Rendered* for the blurred picture, or switch to *Motion vectors*
  (the arrows, colour-coded) or *Confidence* (the trust map in grey) to see exactly what the
  effect is doing. A still frame, a shutter of zero, or zero confidence leaves the picture
  untouched. For now it follows the footage's own motion only (not, yet, motion you add with
  keyframes) and works on footage layers, the same starting scope Echo has.
- **Datamosh** — the corrupted-video "melting picture" look, rebuilt (T19) to follow motion
  properly. Real video codecs sometimes drop a frame's actual picture and just reuse the last
  one nudged by that frame's motion arrows; when this keeps happening, the old picture is
  dragged further and further along the motion and everything that's moving smears and *blooms*
  while the still parts stay put. This effect fakes that on purpose. For every pixel it takes a
  short **walk** along the motion arrows, starting from the previous frame: each step follows
  the arrow at the spot it's currently standing on (re-reading the arrow as it goes, so the
  smear *curves* with the motion instead of running dead straight), nudges along by about one
  frame's worth of movement, and picks up the previous frame's colour there. Those picked-up
  colours are blended together into a melting streak, which is then laid over the ordinary
  frame. Four dials shape it:
  - **Intensity** — how strongly the melt is laid over the true frame. It goes *above* full,
    which over-shoots past the moshed picture for a harder tear; at zero the effect does
    nothing at all.
  - **Displacement** — how far the walk reaches, measured in frames of motion. Higher reaches
    further along the arrows, so a longer smear piles up — the way a long run of "reused"
    frames drifts further from the last clean one. (This replaces the old "Streak length" dial;
    an older project's setting is read straight into it, so nothing changes on load.)
  - **Bloom** — how much of that reach actually accumulates. Turned down, only the nearest bit
    of the walk counts, so the trail is short and keeps resetting; turned up, the whole walk
    averages together into a long, drawn-out melt. It is the "does the smear pile up, or keep
    starting fresh" control.
  - **Reset interval** — an optional clock, in seconds, for the "clean frame" that a real codec
    inserts now and then. Leave it at zero and the melt just runs continuously. Set it, and the
    whole melt fades back to a clean picture at each tick and then builds up again until the
    next — the classic datamosh rhythm of clean, melt, melt, melt, clean. (It's in seconds
    rather than a frame count because, at the point in the pipeline where this is worked out,
    the effect doesn't know the project's frame rate; a frame-count version is a later job.) On
    top of that clock, a clean frame *also* happens by itself wherever there's no motion to
    follow — a still, or a hard cut — which is exactly where a codec would put one.

  It started life as a toggle inside Glitch, off by default, because turning it on means
  fetching an extra frame and running the motion-arrow calculation; when Glitch split into
  three separate effects it became its own, and T19 rebuilt its insides into the walk described
  above. One wrinkle worth knowing: the app can only carry one motion-arrow map per layer per
  frame right now, so if a layer somehow had both Motion blur and Datamosh turned on together,
  only whichever one is listed first in the effect stack gets its arrows this frame — the other
  quietly sits out, the same "missing data, do nothing" safety rule every temporal effect
  already follows.
- **Posterize time — the stop-motion "on twos" look, and a new kind of effect entirely.**
  Every effect so far takes a finished picture and paints on it. **Posterize time** does
  something different: it changes *what moment in time* the layers render at. Drop it on a
  full-frame adjustment layer, set a frame rate like 12, and the whole scene beneath updates
  only 12 times a second — the animation goes choppy and hand-made, the classic stop-motion
  look. The trick is simple arithmetic: the current time is rounded *down* to the nearest step
  on that coarser grid (so any moment between two steps shows the earlier one), and the scene
  below is re-rendered at that held moment. Because it re-renders rather than repaints, it
  cannot live where the other effects live (they only ever see a finished picture, not the
  layers or the clock). Instead it plugs in at the one place that holds the layers and the
  time — the render loop itself — and that place is the same in the preview and in an export,
  so they always agree (the whole point of the shared `render_below_at` helper: both the live
  viewer and the file are literally the same re-render code). Two honest details: the *video
  frame itself* also steps to the coarse grid (that was the FX-1 fix — a scene that is only
  footage playing back would otherwise look untouched, because only the animation was being held;
  now the app also picks the held moment's source frame, so playback visibly chunks along at, say,
  12 a second). Smoothing footage *between* those held frames — real motion blur on the streaks —
  is a different effect (the flow Motion blur); Posterize just quantises the playback grid. And a
  couple of exotic combinations (an echo *inside* the held part, or Posterize buried in a
  collapsed precomp) quietly do nothing rather than risk a wrong picture. There used to be a
  Scope switch choosing between "everything below" and "just this layer" — it is gone (K-166),
  because the layer you drop the effect on already answers the question: an *adjustment layer's*
  whole job is to affect everything beneath it, so Posterize there steps the whole scene; drop it
  on a *normal* layer and only that layer goes choppy — its effects and its footage playback
  step while the layer keeps *moving* smoothly. The per-layer form needs no re-render of the
  rest of the scene at all: the layer simply reads a "held" clock for its own effect stack and
  source frame while its position reads the live one. That is why it is the cheap, simple
  cousin of the whole-scene version.
- **"Don't re-sample this effect" — a per-effect opt-out for the choppy passes.** When
  Posterize time (and, soon, accumulation motion blur) re-renders the scene at a *different*
  moment, it normally re-runs everything at that moment. But some effects are expensive or
  random — a particle system, say — and you would not want them re-computed for every sample;
  it would look wrong and cost a fortune. So every effect now carries a quiet switch, **on** by
  default: leave it on and the effect moves in time with the rest of the scene; turn it **off**
  and that one effect stays frozen at the real playhead while everything around it is held or
  sampled. Behind the scenes this is just "which clock do I read?" per effect — with the switch
  on, both clocks read the same time, so an ordinary render (no posterise, no accumulation
  blur) is completely unaffected.
- **"Motion blur" — the expensive, correct motion blur (accumulation).** There are three kinds
  of motion blur in Lumit, and this is the heavyweight — the one simply called **Motion blur**
  in the menus. (The others: the per-layer transform *switch*, which smears one layer along its
  own movement, and **Fast motion blur**, which invents blur for game footage that never had
  any.) This kind does the honest, brute-force thing: it renders the *whole scene beneath it*
  several times at instants spread across a single frame — a few moments just before the frame,
  a few just after — and averages those finished pictures together. Because it re-renders the
  real scene each time, everything comes out right: moving footage, animated effects, a depth
  pass, the camera drifting — all correctly placed at each instant, then blended. The averaging
  is a neat trick with light: each of the N pictures is added in at one-Nth strength, so a part
  of the scene that didn't move averages back to exactly itself (nothing changes when nothing
  moves — a promise the tests check to the last bit), while anything that *did* move leaves a
  smear proportional to how far it travelled. You drop it on a full-frame adjustment layer to
  blur the whole scene; the Shutter angle sets how much of the frame the "camera" was open
  (180° is the film-standard half-frame), Samples sets how many in-between renders (more is
  smoother and slower — it is genuinely N times the work), and Mix fades the blur back toward
  the sharp original. There is also a **Force on all layers** switch: turn it on and every layer
  also smears along its *own* transform inside each of those in-between renders (the per-layer
  motion blur, forced on for the whole scene at once, using this effect's shutter — your project
  is never actually changed, only the temporary render is). It is a convenience — one switch
  instead of ticking motion blur on every layer — and it smooths the result at lower sample
  counts. It shares the very same re-render machinery as Posterize, so the preview and the
  exported file are, again, literally the same code.
- **Depth of field becomes a real effect — and effects can now read another layer.** Until
  now every effect took numbers, colours, a file. Depth of field needs a *second picture*: a
  "depth map" that says how far away each pixel is. The natural place to get one is **another
  layer** in your composition — a depth pass that matches your footage. So effects gained a
  new kind of control: a **layer reference**, "use *that* layer as my input." It works just
  like a **matte** (which already lets one layer point at another and borrow its shape): the
  app renders the pointed-at layer on its own and hands its picture to the effect. Depth of
  field reads the **red channel** of that picture as depth (dark = near, bright = far, though
  since you choose the focus distance it works either way), and blurs the footage more the
  farther a pixel's depth sits from focus. Two things are worth knowing. First, the depth
  layer is rendered *plainly* — its own effects are not applied — which, as a happy side
  effect, means a depth reference can never chase its own tail into an endless loop. Second,
  the picture you see while scrubbing and the picture you export go through the **one and the
  same** "render that layer on its own" helper, so the preview can never quietly disagree with
  the file (the house rule every effect follows). For now the depth pass should share your
  footage's framing (it is stretched to fit) and should be a *visible* layer; a depth built
  from effects, or hidden away, is a later refinement. The blur disc itself is the foundation
  kernel below, unchanged and still proven against its plain-Rust twin. One more piece the
  owner will add: the little dropdown in the effect controls that actually *picks* the depth
  layer — until that lands the effect is wired and correct but has no layer to point at yet.
- **Depth of field grows three lens controls.** Three tick-and-slide additions, all borrowed
  from the reference plugins. **Depth invert** is a tickbox that flips the depth map's reading
  (`near` becomes `far` and back), so if your depth pass is the wrong way round you fix it with
  one click instead of re-rendering it. **Near blur** and **Far blur** let you set *how much*
  blur the close side and the far side get *separately* — a shallow foreground and a soft
  distance, or the reverse — where before both sides shared one Aperture. Aperture now acts as a
  **master**: it scales both sides together (its normal value, 8, means "leave Near and Far as
  they are"), and turning it up or down blurs the whole picture more or less without touching the
  balance between the two. Old projects saved before this — which only had the one Aperture —
  open and look exactly the same, because Near and Far quietly start out matching it. **Display**
  is a small dropdown of *what you're looking at*: normally **Rendered** (the finished blur), but
  switch to **Depth map** to see the depth pass itself as a greyscale picture (handy for checking
  it is the right way round), or **Focus map** to see a white-where-sharp mask that shows exactly
  which parts of the frame are in focus. The two diagnostic views ignore the blur so you get a
  clean look. As always, the graphics-card program and its plain-Rust twin were checked to agree
  to the last bit across every one of these — invert on and off, lopsided near/far, and each
  display mode.
- **Depth-of-field, the foundation** — the first piece of a "lens blur" that keeps one
  distance sharp and softens everything nearer and farther, the way a real camera lens does.
  A photographic lens can only focus at one distance at a time; things off that plane spread
  each point of light into a little disc — the bigger the disc, the blurrier it looks — and
  the disc's size is called the *circle of confusion*. This kernel does exactly that: for
  every pixel it looks up how *deep* that pixel is (a plain 0-to-1 "depth map", near to far),
  works out how far that depth sits from the chosen focus distance, and from that picks a
  blur-disc size — nothing at all inside a sharp band around focus (set by Focus distance and
  Focus range), then easing up to a maximum (Aperture, the biggest disc in pixels) for the
  most out-of-focus depths. It then averages a disc of the source image that size around the
  pixel, so near-focus areas stay crisp and distant ones melt. Two honest limitations for
  now, and they are the whole reason this landed as a *foundation* rather than a finished
  effect: first, nothing in Lumit yet produces a real depth map — a proper version needs to
  read depth from another layer, which is a much larger plumbing change (the same kind Motion
  blur's motion-map needed), so for the moment the depth is something a test or a future
  source hands in; second, the bokeh is a plain flat disc, not the shaped, bright-rimmed
  highlights the eventual "DOF PRO" effect will add. What *is* finished and locked by a test
  is the maths: the graphics-card program and a plain-Rust copy of it compute byte-for-byte
  the same disc, tap for tap, so — exactly like every other effect — what the card draws
  provably matches the reference, and a zero Aperture (or a subject sitting right on the
  focus plane) leaves the picture untouched to the last bit.
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
- **Blur becomes three separate effects** (the house rule: one effect, one job). Until now
  "Blur" was a single effect with a Mode dropdown — Gaussian, Directional or Radial — and all
  three modes' controls sat on it at once, most of them greyed-out and irrelevant depending on
  the mode. Now there are three effects you pick from the Add-effect menu directly — **Gaussian
  blur**, **Directional blur** and **Radial blur** — each showing only its own controls. Nothing
  about *how* each blur looks changed: the actual blur programs and their reference twins are
  the exact same code, only the menu and the little bit of glue that reads the controls moved.
  A few knock-on tidyings came with the split. The old effect had one **Edges** control
  (Transparent / Repeat / Mirror — what to pretend is beyond the frame's edge) shared by all
  three modes; it now lives **only on Radial blur**, where a spin or zoom most often sweeps past
  the border and you might want it to mirror or fade. Gaussian and Directional just use the old
  default (Repeat, which keeps full-frame footage from darkening at the edges), so they look
  identical. Directional's **Length** and Radial's **Amount** can now go past their old ceilings
  (bigger sliders, and you can type further still) since each is its own effect and no longer
  has to share one budget — the programs already cap how much work a huge value can ask for, so
  there's no runaway cost. And projects saved with the old combined Blur still open fine:
  whatever mode they were on, they come back as a Gaussian blur at the same radius (the effect
  kept its internal name, `blur`), which is the sensible common case.
- **Sharpen splits into "Unsharp mask" and a plain "Sharpen".** The effect that was called
  Sharpen was, under the hood, an *unsharp mask* — the photographer's technique of blurring a
  copy, subtracting it to find the fine detail, and adding that detail back, with knobs for how
  wide the detail is (Radius), how strong (Amount), a Threshold to leave flat areas alone, and a
  luminance-only option. That is still here, just honestly relabelled **Unsharp mask** (its
  internal name is unchanged, so nothing saved breaks). Sitting beside it is now a brand-new,
  much simpler **Sharpen**: a plain 3×3 sharpen — the classic one every image editor has — that
  looks at each pixel and its four immediate neighbours and pushes the pixel away from their
  average, with a single **Amount** dial for how hard (1 is the textbook strength, 0 does
  nothing). No radius, no threshold — just "sharpen it a bit". It works on the true colour
  (dividing out transparency first, like the other colour effects, so edges of a cut-out don't
  fringe), and turning Amount or Mix to zero leaves the picture untouched to the last bit. As
  always, the graphics-card version and a plain-Rust copy were checked to agree pixel-for-pixel.
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
  fast, Rotation amount how much twist. A **Per-axis wobble** twirl (a collapsible
  sub-section, see below) tucks the finer controls away: X and Y amount/frequency let you
  bias each axis (they multiply the master values, so leaving them at 1 gives the plain
  even shake), and Z is a depth shake — the frame pumps a little bigger and smaller, the
  old "zoom pump" renamed. When the wobble drags the frame's edge into view, the **Edges**
  control decides what shows there: Transparent (a clear border), Repeat (the edge pixel
  held outward) or Mirror (the picture reflected) — the same three choices the blur effects
  offer (see "Edges control" below). This replaced an older Auto-scale toggle that quietly
  zoomed in to hide the border; a project saved before the change carries its old zoom-pump
  and auto-scale settings across automatically (auto-scale on becomes Repeat, off becomes
  Transparent). Seed is a new parameter type: an integer picking *which* wander you get —
  each new instance rolls its own so two shaken layers never move in sync, and the Reseed
  button rolls a fresh one. Shake also taught the frame cache a lesson: its parameters can
  sit constant while the picture moves every frame, so for effects that declare seeded
  randomness the cache key now includes the layer's local time — without that, a shaken
  solid would replay its first cached frame forever. A second twirl, **Motion blur**, gives
  the shake *its own* motion blur (separate from the layer and comp motion blur, and touching
  only this effect). Because the wobble is pure maths of time, the engine can ask "where was
  the shake a moment before, and a moment after this frame" and draw the picture at several of
  those in-between positions, then average them — so a fast shake smears along its own path
  instead of snapping frame to frame, the way a real camera blurs when it jolts. It is off by
  default; the **Shutter** dial (0 to 1) sets how long that smear is, and 0 (or the toggle
  off) is exactly the plain, un-blurred shake. The in-between positions are worked out on the
  CPU because the noise recipe needs 64-bit whole numbers the graphics card cannot do, then a
  small dedicated GPU program does the averaging. The smear's length is measured in the
  shake's own rhythm rather than in seconds, so it looks the same whether your project runs at
  30 or 60 frames a second (K-165).
- **Edges control (a shared effect building block).** Several effects move pixels around —
  a blur that smears sideways, a shake that slides the whole frame — and wherever the
  picture shifts, it can pull in area from *outside* the layer that has no pixels of its
  own. The Edges control names what to put there, with three settings shared by every
  effect that needs them: **Transparent** (leave it clear), **Repeat** (stretch the very
  edge pixel outward, so full-screen footage never grows a dark border) and **Mirror**
  (reflect the picture back on itself). It is one small reusable piece rather than each
  effect inventing its own, so it behaves identically everywhere it appears (in code it is
  a shared `EdgesMode` with three fixed options).
- **Collapsible "twirl" sub-sections in effect controls.** An effect's parameter list can
  hide its advanced controls behind a disclosure triangle — a little header you click to
  fold a group open or shut, exactly like twirling a layer open in the timeline. Shake's
  "Per-axis wobble" is the first: the everyday knobs (Amplitude, Frequency, Rotation) stay
  in plain view, and the per-axis fine-tuning tucks away until you want it. Any effect can
  ask for one just by declaring the group in its parameter schema — it is a reusable piece
  of the effect-controls panel, not something written afresh each time.
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
- **Lens flare** (K-256) is the first effect that *simulates* something instead of
  filtering pixels. A camera lens is a stack of curved glass discs with an iris in the
  middle; when a bright light is in shot, a whisper of it reflects off the inside of one
  glass surface, bounces backward, reflects off another, and still reaches the sensor —
  that faint double-bounce image is one **ghost**, and a lens with fifteen surfaces has
  dozens of such paths, which is the train of coloured blobs sliding across real footage.
  Lumit doesn't draw those blobs; it ships the actual measurements of real lenses (the
  curvature, spacing and glass of each element, from published patents) and every frame
  shoots a grid of imaginary light rays through that geometry on the graphics card, letting
  each ghost land where the physics puts it. That is why the ghosts stretch, flip through
  the centre as the light crosses frame, grow when you open the F-stop, and wear
  blue-green-magenta tints — the tint is the anti-reflective coating on each surface
  interfering with itself, computed per wavelength, not a colour swatch. The **starburst**
  (the spiky star on the light itself) is different physics: light bending around the iris
  blades, which Lumit gets by taking a Fourier transform of the iris shape — an
  established bit of optics maths — so an 8-blade iris genuinely makes an 8-spike star,
  and the spikes fringe into rainbows because each wavelength bends by a different amount.
  The expensive maths (the Fourier transforms, the iris image) runs once on the CPU when
  you change a parameter and is remembered; only the ray shooting and drawing happen per
  frame, which is what keeps it scrubbable. When it's doing too much, the dials to reach
  for are Intensity, the separate Ghost/Starburst intensities, Max ghosts (thins the
  train), and Quality (Draft renders the flare at half resolution). One honest caveat
  lives in the decision log: the trace has an exact CPU twin the tests hold to, but the
  final drawn frame is checked against a *close* reference rather than bit-for-bit,
  because two rasterisers never fill triangle edges identically — the full story is in
  docs/impl/lens-flare.md. A second pass (K-257) reshaped the panel to the owner's
  design — the light is one x/y row with a pick-on-the-picture dropper, the detail
  dials fold behind Lens options / Flare options twirls (the first effect panel whose
  schema groups actually render as twirls, a mechanism every effect now shares), and a
  **Source** choice appeared: Manual light is the classic tracked point, **Matte** finds
  up to eight of the brightest points in another layer's picture on the graphics card
  and gives each its own full flare tinted by the source's colour, and Lights waits,
  ready, for light layers. It also taught the effect system whole-number parameters
  (Blades really is 8, not 8.00) and conditional rows that appear only for the mode
  that uses them. A **Light tint** colour (with the usual swatch and eyedropper) then
  colours the flare in any mode, and beside it — when the source is a matte or, later,
  light layers — a **Use source colour** switch decides whether each detected light
  keeps its own colour (a warm practical flaring warm, a cool one cool) or flares white
  through the tint alone, which is what you want when the matte is only there to say
  *where* the lights are. A calibration pass (K-260, on advice from the author of a
  reference flare renderer) made the optics honest: instead of trusting the patent
  paperwork about where the sensor sits, the bake now shoots one thin test ray through
  the lens and puts the sensor exactly where that ray comes to a point — the lens's own
  measured focus. That mattered because patent tables lie a little (the bundled "Zeiss
  50mm" actually measures 64.8 mm). It also added a **Focus (m)** dial: refocusing a
  real lens slides the sensor a fraction of a millimetre, and that tiny slide visibly
  rearranges the whole ghost train — the same lens at 1 m and at infinity throws
  completely different flares, and now Lumit's does too. The third pass (K-261)
  swapped the whole optical engine for the one in FlareSim, an open Nuke plugin the
  owner pointed at: instead of ten hand-built lenses, Lumit now embeds **1,299 real
  lens prescriptions** — text files transcribed from patents, each one the actual
  glass recipe of a Nikon, Canon, Zeiss and so on — and the Lens dropdown picks
  among them. Each file says how each glass surface is coated, so the old
  "coating preset" menu became unnecessary: the lens itself knows. The tracing
  follows FlareSim's method faithfully, but the drawing keeps Lumit's own
  smooth-grid approach — FlareSim splats millions of ray dots and blurs the noise
  away, which needs far more rays than drawing the ray grid as connected little
  panels that brighten where rays bunch up. A new **Ghost softness** dial (borrowed
  from FlareSim's Ghost Blur) adds a touch of out-of-focus softness on top.
  A follow-up pass (K-262) fixed what the owner found when actually using it.
  The faint lines shooting across the flare turned out to be a bug in Lumit's
  own drawing: when a cell of the ray grid lands across a "fold" in the light,
  it arrives as a hair-thin sliver, and the code that rescues tiny cells from
  being dropped was stretching those slivers into long streaks. Slivers are now
  thrown away instead — the light they carry is spread to nothing anyway, and
  their neighbours draw the real shape. The grid is also spent more cleverly:
  each ghost gets rays in proportion to how big it lands, so a huge soft ghost
  is no longer drawn with the same handful of cells as a tiny sharp one. That
  is what makes the **Normal** quality usable rather than the tier where you
  see the grid. And the Lens list, at 1299 entries, became a **searchable
  picker** grouped by manufacturer — the plain dropdown was building all 1299
  rows at once, which crashed the app.
  The next pass (K-263) chased the worst report yet: after a few passes through
  the lens picker on a Mac, the picture simply stopped updating, and opening a
  different project did not bring it back. That last part is the clue. Every
  project gets its own worker, but they all share one *graphics device* — the
  connection to the card that the whole program holds — so a frozen picture that
  survives a new project means the device itself has died. It had. Work is given
  to a graphics card in parcels called submissions, and both macOS and Windows
  kill a submission that takes too long, on the assumption that anything running
  that long has hung; the kill takes the device with it, and everything after
  fails silently. The flare was sending an entire frame as **one** parcel whose
  size you set with Quality, Max ghosts and the source mode — so winding those up
  did not make a slow frame, it made a dead device. The frame is now cut into
  parcels small enough that no combination of settings can reach the limit. That
  changes nothing about the picture: the pieces are handed over in the same order
  and add up the same way, they just go in several loads instead of one.
  Two other things were feeding it. The flare was asking the card for tens of
  megabytes of scratch memory **every frame** and throwing it away — and a
  graphics driver only really reclaims that when the work it belonged to has
  finished, so a Viewer redrawing continuously built up a backlog of abandoned
  memory; on a Mac, where the card shares memory with everything else, that is a
  slow squeeze. It now keeps one set of scratch buffers and reuses them. And the
  work itself was being done at the wrong size: the biggest ghost in the frame set
  the working grid for *all* of them, so fifty compact ghosts each did the work of
  the one enormous one. Each group of ghosts now works at its own size. Add a
  handful of smaller economies — the two passes that read the same rays merged
  into one, a drawn cell shrunk from 192 bytes to 80, the softness blur reading
  each texel once for a whole row of pixels instead of once per pixel, a square
  root per ray instead of one per glass surface — and a heavy frame measured about
  a quarter faster on a slow test machine, with nothing about the result changed:
  the same tests that check the picture against the reference maths still pass.
  Choosing a lens is still a pause of about half a second, because the one-off
  maths for a new lens runs on the same thread that draws — the fix for that is
  the progress indicator on the TODO list, not a shortcut in the optics. What did
  improve is that Lumit now remembers the last two dozen lenses you tried instead
  of eight, and forgets them one at a time instead of all at once; before, every
  ninth lens threw away the eight before it, so going back to one you had just
  looked at made you wait all over again.
  The pass after that (K-264) went after what the owner saw at Ultra: triangles
  in the ghost rims, blocky faceting, jagged edges. All three came from the same
  root. The flare is drawn as a fine net of little four-cornered panels, and each
  panel used to be given ONE brightness — so neighbouring panels disagreed at
  their shared edge, and a smooth ghost became a mosaic of tiles. The fix is the
  one the original research paper used: give the brightness to the CORNERS
  (each corner averages the panels around it) and let the graphics card blend
  smoothly across every panel — the seams simply cease to exist. The jagged
  outlines got the standard cure, multisampling: the card checks four points
  per pixel instead of one at the edges of shapes, which is how every game
  smooths its edges. And the triangular notches in the bright rims turned out to
  be panels the code was *throwing away* out of caution — an earlier bug made
  long thin panels dangerous to draw, so they were dropped, and every dropped
  one left a bite mark in a rim. They are safe to draw now that brightness is
  smooth, so the rims are whole. A subtler family of the same disease: whenever
  a ray of light "died" at some edge inside the lens — the iris, a lens
  barrel, missing a glass surface — every panel touching it vanished, and the
  ghost's edge was quantised into stair-steps. Rays never die now: they carry
  on with their light set to zero, fading over a distance instead of stopping
  at a panel boundary. Same maths, same energy, smooth reconstruction — the
  before/after images that drove the work were rendered through the real
  pipeline on a software graphics driver and compared by eye, and that little
  harness stayed behind as a test anyone can run.
  The same pass rebuilt the lens list around a decision of the owner's: twenty
  lenses instead of 1299. A thousand entries is a search problem, not a choice;
  the twenty were picked for maximally different characters — modern cinema
  glass, uncoated lenses from the 1930s that flare bright and neutral, a tiny
  four-element design that throws a sparse clean train, an f0.95 with huge
  discs, a fisheye, a projection lens, a superzoom with a ghost for every one
  of its many elements — and each was rendered through the pipeline into a
  contact sheet and checked by eye for distinctness. For everything else there
  is a new **Lens file** parameter: point it at a `.lens` prescription file
  (the same public format the bundled ones use, and the parser already read)
  and it replaces the picked lens entirely. The file's contents are hashed
  into the effect's cache key, so editing the file shows up on the next frame;
  a missing or broken file quietly falls back to the picked lens rather than
  erroring. Ghost softness now defaults to 0.02 — with the smooth shading
  there is nothing left for blur to hide, so the default is a taste, not a
  bandage.
  The owner then lived with it for an hour and found five more truths (K-265).
  The app died after minutes of switching lenses: the new multisampled canvas —
  the biggest piece of memory the effect owns — was being created and thrown
  away every frame, the exact disease the K-263 pass diagnosed for buffers,
  and it is now kept and reused like the rest. Several of the twenty lenses
  turned out to flare only when the light sat near the centre — the contact
  sheet that chose them had been rendered with a near-centred light, so
  lenses that die off-centre slipped through. The curation now stands on a
  three-position probe (centre, off-centre, far corner) and the list was
  re-cut: every wide-angle and fisheye design failed it, because the ray
  model's acceptance genuinely collapses off-axis for those constructions —
  so none are bundled, and that limit is written down rather than papered
  over. The Lens file row, which shipped un-clickable (true of the LUT's file
  row too, an old gap from the Flutter port), is now the picker itself: click
  it, choose a file, and a small × clears it. And a new **Detail** dial
  (0.25–4) hands the ray budget to the user — it multiplies both the number
  of rays AND the number of traced colours, because the owner's zoom-lens
  test proved a corona of colour-banding that rays alone cannot dissolve.
  One artefact survived every fix thrown at it — a toothed fringe on one
  ghost of a zoom shot two stops past its widest — and rather than another
  guard, the decision log records the six things that were tried and ruled
  out, so the real cure (subdividing the grid exactly at the folds) stays an
  honest TODO instead of a mystery.
  A fourth pass (K-266) closed the day's remaining reports. The flare's
  light was landing past where the owner put it — but only in preview, and
  only on adjustment layers, because an adjustment's effects are resolved
  as if they will run at full comp size and preview quietly runs them
  smaller; a new one-place repair (`rescale_px`) scales every
  pixel-flavoured parameter by the true factor before the stack runs, which
  also silently fixes preview-vs-export drift for blur radii and DoF
  apertures on adjustment layers. Anamorphic squeeze below 1 used to smear
  the frame edge — the flare was being asked for pixels beyond its own
  canvas and the card answered with the nearest edge pixel, repeated;
  outside the canvas is now simply dark. The chunky tiled edges on the big
  soft ghosts fell to one more smoothing rule: a ray's brightness is now
  averaged with its eight neighbours before drawing, so a hard lighting
  cliff becomes a gentle two-panel ramp (the geometry still uses the raw
  values, so nothing bleeds where it shouldn't). And picking a precomp as
  the flare's matte — the natural way to author "a white circle on black
  as my light source" — finally works: a comp has no pixels until it is
  rendered, so the matte machinery quietly gave up on them; it now renders
  the nested comp exactly the way a precomp layer is rendered, loops
  guarded, and hands the picture to the light detector. One honest edge
  remains written down: footage inside a matte-only precomp needs the
  decode planner taught before it appears there.
  (K-268 closed that edge, and it turned out to be smaller than feared —
  see *Precomps, in the two places they used to fall through* below.)
  A fifth pass (K-267) closed the next round. The choppy ghost edges that
  survived at corner lights turned out to be a measuring problem, not a
  drawing one: the effect sizes each ghost's ray budget from a once-per-lens
  measurement of how BIG the ghost gets, but a ghost near the frame corner
  does not get bigger — it gets locally *stretched*, like an image printed
  on rubber, and the stretched patch is where the facets show. So every
  frame now runs a tiny probe (a handful of test rays per ghost, microseconds)
  at the light's actual position, finds the worst-stretched ghosts, and gives
  extra rays exactly there — under a strict allowance, worst first, so the
  frame's cost is bounded and predictable rather than exploding on a bad
  lens. Anamorphic squeeze below 1 stopped cutting to black at the edges:
  the ghost picture is now simply rendered onto a wider canvas (up to twice
  as wide) before the squeeze samples it, so there is real image where it
  reaches. And an area light finally weighs as an area: the detector still
  anchors each flare on the brightest spot (now up to sixteen of them), but
  every lit detection tile pours its brightness into the nearest anchor, so
  the owner's white-circle precomp flares with the strength of the whole
  circle where it used to count as a single pixel — while a true pinpoint
  light reads exactly as before.
  A sixth pass fixed two things that had been quietly making the effect
  harder to use than it needed to be.
  **The matte now starts on the layer you put the effect on** (K-288).
  Switching Source to Matte used to leave the Matte layer empty, so the
  effect did nothing until you went and found a layer to point it at — and
  the layer you nearly always wanted was the one you were already standing
  on. Worse: on an **adjustment layer** there was no right answer at all.
  An adjustment layer has no picture of its own; its job is to act on
  everything below it. But the picker refused to offer the layer itself
  ("you can't sample yourself"), so you had to point at some other layer and
  detect lights in the wrong image. Now the picker does offer it, labelled
  *(this layer)*, and it means something precise: **read whatever picture is
  arriving at this effect**. On an ordinary layer that is the layer's own
  image; on an adjustment layer it is the composite of everything beneath
  it — which is exactly the picture you wanted flared. Nothing is rendered
  twice to do it (the effect already has that picture in hand), so it is
  cheaper as well as more useful, and it lines up pixel-for-pixel with what
  the flare draws instead of being stretched to fit. The rule applies to any
  effect that reads another layer, not just this one — the depth-of-field
  depth pass can be pointed at its own layer too, though it doesn't start
  there, because a depth map is never the picture itself.
  **And the Background choice became a Blend menu** (K-289). The flare used
  to offer Transparent or Black, which was really a blend-mode question in
  disguise: everything the effect renders is a picture of light on a black
  background, and those two options were two ways of combining that picture
  with your layer. So it is now the same menu a layer's own Mode dropdown
  offers — Normal, then Add, Screen, Multiply, Overlay, Soft light, Hard
  light, Lighten, Darken, Difference, Exclusion, Subtract, Divide (the same
  list Echo offers, and short of the layer list for the same reason: hue and
  colour-burn style modes don't mean anything applied to a glow). **Add** is
  the default and is exactly what the flare always did, pixel for pixel, so
  nothing you have already built moves. **Normal** shows the flare on its
  own black background and hides the layer, which is what "Black" was for —
  the flare as a separate element you Screen back on in another comp — so a
  project saved with Black opens on Normal.
- **RGB split gains a Wavelength mode** (K-090's quality-tier pattern: where the smooth
  look is optional, it hides behind a Bool next to the fast one). Off — the default —
  the split is three tinted samples: the first colour pulled one way, the third the
  other, the second in place. On, the kernel instead takes many samples (up to 64)
  spread along the same line and tints each by your three-colour picker blended into a
  smooth gradient — the first colour at one end, the second in the middle, the third at
  the other end (A1/K-163). So the fringe is a smooth graded band you control by colour,
  and the default red / green / blue gives the familiar red→green→blue dispersion.
  (Earlier this used a fixed physical spectrum table; the owner chose to let the picker
  drive it instead, so changing the colours changes the fringe.) The gradient is worked
  out once in `lumit-core` next to the CPU reference and handed to the GPU kernel through
  its parameter block, so both paths read literally the same numbers (the same trick as
  the host-computed sines). Its columns are normalised so a flat image passes through
  unchanged — the fringe is tinted, not the exposure — and alpha still refuses to move,
  so mattes never grow coloured rims in either mode. The classic three-tap mode now gets
  the *same* normalisation (K-167): because the three taps are simply added together,
  custom tints used to brighten or darken the whole picture, not just the fringe — each
  output channel's three weights are now rescaled to add up to one before the kernel sees
  them, so recolouring the split only recolours the parts where the taps disagree (the
  misaligned edges), and the default red / green / blue is untouched to the bit.
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
- **Vibrancy** (K-152) is Saturation's smarter cousin. Saturation scales *every* pixel's
  colourfulness by the same amount, so pushing it hard blows out the colours that were
  already strong (and turns skin an unnatural orange). Vibrancy looks at how colourful
  each pixel already is and lifts the dull ones more than the vivid ones — near-greys and
  skin tones come alive while the saturated bits are left roughly alone, so nothing
  clips. It has one **Amount** dial (0 does nothing; turn it up to taste, and it happily
  goes past 100). Same careful plumbing as Saturation — it works on unpremultiplied
  colour in linear light, pivots about proper luma, and never goes negative — with a GPU
  test holding it exactly to the CPU reference.
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
  *Two later additions (FX-9):* **per-channel amounts** — three sliders (Red / Green /
  Blue, defaults 100 / 0 / 100 per cent) that scale each channel's own shift, so you can
  fringe red harder than blue, or nudge green too; the defaults are exactly the classic
  split. And in **Wavelength** mode there is now a **Samples** knob: that mode makes a smooth
  graded fringe by taking many samples along the shift and tinting each from your three-colour
  picker's gradient (A1/K-163), and at big shifts too few samples showed a handful of separate
  copies — Samples (default 16, up to 64) fills the gap so it reads as a smooth band. The samples
  are worked out once on the CPU and handed to the GPU, the same trick as the sines, so preview
  and export agree to the last bit.
- **The reusable three-colour channel picker.** Some effects split a picture into three
  tinted channels; **Chromatic aberration** (below) is the first. Rather than three separate
  colour rows, those effects show one tidy row of three swatches (defaults red / green /
  blue) — click a swatch to open the colour picker. It is one small shared widget: any
  effect whose parameter list names three colours `channel_colour_1/2/3` gets the picker
  automatically, so the next such effect needs no new interface code. Chromatic aberration's
  three swatches tint its three taps, and leaving them red / green / blue gives the ordinary
  R-outward / B-inward / green-anchored fringe; recolour them for a stylised split.
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
- **The one-slot drag rule**, worth knowing wherever drag-and-drop is written. If the
  toolkit carries exactly one "thing being dragged" for the whole app — a single hand
  holding one object — then a drop zone that asks "was that released on me?" may be
  handed the object *before* anything checks it is the kind that zone wanted, and a
  zone that shrugs at the wrong kind has already consumed it. Overlapping zones then
  eat each other's drops silently: the wide zone underneath asks first and discards
  what the row on top was waiting for. **Every drop zone must peek at the kind before
  taking the drop**, through one shared reader rather than each zone deciding for
  itself.
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
  `.lumfx` JSON file, K-065) lives on that same add-effect row. **A preset library (K-129)**
  gives those saved looks a browsable home: the **Effects & Presets** panel now opens with a
  **Presets** group listing every `.lumfx` file in one shared folder (tucked away in Windows'
  roaming app-data area, next to Lumit's other saved data). Click a preset and its whole
  saved stack is added to the layer you have selected — one undo step, exactly as loading a
  preset by hand does. "Save stack as preset…" now points its save box at that same folder to
  begin with, so anything you save shows up in the list straight away; you can still save it
  elsewhere if you want. An empty folder just shows a gentle hint rather than an error.
  **A preset now saves whatever you have highlighted, not always the whole stack (UI-10,
  K-156).** Highlight one or more effects and it saves just those, with their settings as they
  stand; pick out specific keyframes on the lanes and it saves only those keys (the rest of the
  animation, and any effect you did not touch, is left out). Highlight nothing and it still saves
  the whole stack, as before — so the old behaviour is one click away when you want it.
  **Dragging an effect's value
  updates the Viewer live** — as you drag a Glow radius or a Blur amount, the picture re-runs
  the effect with the value under your cursor every frame, committing once when you let go (so
  a whole drag is one undo step). It reuses the same trick a transform-value drag already uses:
  the retained frame is re-composited with the provisional value patched in, no re-decode.
- **Two more single-frame effects (K-099).** **Vignette** darkens the frame toward black
  away from the centre (Amount/Radius/Softness/Roundness); **Chromatic aberration** fringes
  red and blue outward/inward from the centre by a set number of pixels — a simpler,
  always-on-the-corner sibling of RGB split's own Radial mode, for the common one-click case.
  It later grew two matching extras (K-143/K-144): the **three-colour channel picker** (recolour
  the three tinted taps; leaving them red / green / blue is the ordinary fringe) and RGB split's
  own **Wavelength/Samples** rainbow mode, reusing the very same spectral machinery.
- **Exposure (K-106).** The one-knob brightness lever, measured in photographic *stops* —
  each +1 doubles the light, −1 halves it. It is a straight multiply on the colour (done in
  the scene-linear light the compositor works in, so it behaves like a real camera exposure,
  not a washed-out lift), with 0 stops leaving the picture exactly untouched. Distinct from
  Colour balance's three-channel gain: a single animatable control for the whole image.
- **Hue shift (K-108, K-136).** Turn every colour's hue by an angle — reds toward orange,
  blues toward purple, and so on. 0° leaves the picture exactly as it was. Under the hood it is
  a small fixed colour-mixing matrix worked out once for the angle, so the preview and the
  export apply the identical numbers. A **Preserve luminance** tick (on by default) chooses how
  it turns:
  - **On** keeps how *bright* each colour looks unchanged as its hue moves — a
    "constant-luminance" rotation, the same maths web browsers use for their hue-rotate filter.
    This weights the calculation by how bright the eye finds each channel (green counts far more
    than blue).
  - **Off** does the plainer thing: it spins the red/green/blue values around like a colour
    wheel with every channel weighted equally. That can *change* how bright a colour looks as
    its hue turns (a green may go duller or brighter), which is sometimes exactly the punchy,
    less-careful look you want.

  A word on this and Oklab. Lumit's rule of thumb (K-034) is that hue-type work belongs in
  Oklab, the perceptual colour space where "keep the brightness, change the hue" is natural.
  Hue shift's preserve-luminance mode is that *idea* — hold brightness, turn the hue — but it
  reaches it with a cheaper Rec.709-weighted spin in ordinary linear RGB rather than a full
  Oklab conversion, which is plenty for a hue wheel and keeps the CPU and GPU trivially
  matched. The preserve-luminance-**off** mode is the honest "just spin the RGB numbers"
  version, weights and brightness-shifts and all.
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
- **Matte key — greenscreen removal (K-154).** Drop this on green-screen footage and it makes
  the green vanish, leaving whatever was shot in front of it on a clean transparent background.
  It is modelled on the professional keyer *Keylight*: you tell it the **Screen colour** (a
  green by default, so it works the moment you add it — but its brightest channel decides the
  screen, so a blue screen keys just as well), and it measures each pixel's screen colour
  against the two *other* colours to decide how much is screen and how much is subject. The
  top-level dials are the ones you reach for first. **Screen gain** is the overall strength —
  turn it up if patches of green survive, down if the foreground starts thinning. **Screen
  balance** decides how the two non-screen channels are combined into the reference the screen
  is measured against; the middle setting suits most shots, and nudging it either way rescues
  awkward tints. **Despill amount** tackles the green *spill* a bright screen throws onto the
  subject's edges — it drains that green back out so shoulders and hair don't glow green
  against the new background. Two colour swatches, **Despill bias** and **Alpha bias**, let you
  tell the keyer what should count as "neutral" for the spill and for the matte respectively;
  left grey they do nothing, which is the usual starting point.
  - The **View** menu at the top is how you *see* what you are keying: **Final result** is the
    finished cut-out, **Screen matte** shows the transparency itself as a black-and-white image
    (white stays, black goes) so you can spot holes and grey patches, and **Status** tints the
    uncertain in-between areas so problem edges jump out.
  - The **Screen matte** twirl holds the clean-up controls. **Clip black** forces the nearly
    transparent parts fully transparent (killing background haze), **Clip white** forces the
    nearly solid parts fully solid (filling pinholes in the subject), and **Clip rollback**
    eases those two back off a touch to win back fine detail like stray hairs. **Replace
    method** (with its **Replace colour**) decides what colour fills the de-spilled edges —
    *Soft colour*, the default, tints them with the replace colour scaled to the edge's own
    brightness so it settles in naturally; *Hard colour* uses it flat; *Source* keeps the
    original edge colour; *None* leaves the plainly de-spilled colour.
  - Two design points worth knowing: every step is a *gradual blend* rather than a hard on/off
    switch (a hard switch would make the CPU and graphics-card versions disagree by a hair,
    which the agreement test forbids — same rule as everywhere else), and like the other colour
    tools it works on the picture's *straight* colours, undoing the alpha pre-multiply first, so
    it judges edge pixels by their true colour and doesn't leave a fringe. Any of the colour
    swatches can be set with the **eyedropper** beside it, sampling straight from the Viewer
    (see the colour picker and eyedropper note below). A project made before this expansion
    keeps its old screen colour and spill amount and simply re-keys with the new controls at
    their defaults. Some further Keylight refinements — blurring and shrinking the matte,
    garbage masks, per-region colour correction and edge crops — are noted for a later pass.
- **Invert (K-126).** The classic negative: every colour flips to its opposite — black becomes
  white, blue becomes orange, and so on (each channel is replaced by "one minus itself"). There
  are no dials except the shared **Mix**, so it always inverts; turn Mix down to blend the
  negative part-way back toward the original. Like Contrast and Gamma it works on the picture's
  *straight* colours (Lumit divides the alpha out, inverts, folds it back in) so soft edges don't
  fringe. It flips in the compositor's own light space, which keeps it simple and truthful — very
  bright (above-white) values honestly flip to negatives rather than being clipped, exactly as the
  owner asked for a "simple inverse".
- **Tint (K-127).** A two-colour recolour that keeps the *brightness* of the picture but swaps
  its *palette*. You pick two colours — **Map black to** and **Map white to** — and Lumit reads
  each pixel's brightness and places it on the gradient between those two: the darkest parts take
  the first colour, the brightest take the second, everything in between blends across. Left at its
  defaults (black→black, white→white) it turns the image black-and-white; set the two colours to,
  say, deep teal and warm cream and you get a duotone poster look while the shading of the original
  is preserved. Like the other colour tools it works on the straight colour under the alpha so
  edges stay clean, and **Mix** dials the whole effect in or out.
- **Layer-input source: None / Masks / Effects and masks (K-142, was K-125).** Some tools read
  a **second layer** for their shape or data: a **track matte** borrows another layer's brightness
  or transparency to decide where the layer below shows through, and **Depth of field** reads a
  **depth pass** layer to know how far each pixel is. For both, a little **Source** combobox sits
  beside the layer picker and decides *how much* of that other layer to read:
  - **None** — its **raw picture** only: no masks, no effects. The plainest input.
  - **Masks** — its picture **with its own masks** applied, but not its effects.
  - **Effects and masks** — its **finished picture**: the layer's effects and masks run first.
    This is the one you want when the *point* is the effect — a **keyed** greenscreen matte, an
    edge you **softened** with a blur, or a depth pass you **graded** before the lens blur reads it.

  This replaces the old two-way **After effects** on/off switch. A project saved with that switch
  loads correctly: on becomes **Effects and masks**, off becomes **None**. One limitation worth
  knowing (unchanged): "Effects and masks" applies the layer's *look* effects (keys, blurs, colour)
  but not its *time-based* ones — an Echo or motion-blur-from-movement on the referenced layer is
  treated as a still frame; the everyday cases are exact.
- **Precomps, in the two places they used to fall through (K-268).** Every other kind of layer
  has *pixels*: a solid is a colour, footage is a decoded frame, text is rasterised type. A
  **precomp** has none — its picture only exists once it has been rendered, which is a job in
  itself. Anywhere the code reached for "this layer's pixels" and got a precomp, it had to be
  taught to render one instead, and two such places had been missed.
  - **A precomp used as a track matte gated nothing.** Set a layer's matte to a precomp and
    the matte quietly did not exist: the layer drew everywhere, exactly as if the row had been
    left unset. (The sibling case — a precomp as a lens flare's Matte source or a depth-of-field
    depth pass — was fixed in K-266; the track matte, which is the row you actually reach for,
    was still asking for pixels that a comp does not have.) It now renders the nested comp the
    same way a precomp *layer* is rendered, loops guarded, and gates with the result. Because
    a comp already contains its own layers' masks and effects, the None / Masks / Effects and
    masks combobox above has nothing left to decide for one, and is ignored there.
  - **An effect ON a precomp layer drifted in reduced-resolution preview.** Parameters measured
    in comp pixels — a Transform's offset, a flare's light position, a blur radius — are worked
    out for a full-size frame and then have to be scaled down when preview renders smaller. That
    correction was being applied to adjustment layers (K-266) and to ordinary layers, but not to
    a precomp layer, so an effect on one landed further across the picture the coarser the
    preview got, and snapped back at Full. Preview and export were never wrong at full
    resolution; now they agree at every resolution, which is the whole point of a preview.

  While closing the first of those, a note K-266 left behind — "footage inside a matte-only
  precomp won't decode" — turned out to be about a bridge that was already built: the part of
  the engine that decides which video frames to read (the *decode plan*) follows matte and
  layer-input references whether or not the layer is visible. It is now pinned by a test rather
  than by a warning in a document.
- **Colour picker and dropper (K-210).** Every effect **Colour** parameter — a Flash tint, a
  Colour balance wheel, the Matte key's Key colour, and so on — shows a **clickable swatch**.
  Click it and Lumit's own picker opens: the **red, green and blue numbers across the top**,
  each of which you can drag sideways or type into, then the big square (how vivid, how bright),
  the rainbow strip (which hue), and a hex box. Change any one of them and the rest follow.

  The picker **applies as you go**: whatever it is showing is what the composition shows, so
  there is no button standing between choosing a colour and seeing it. Dragging inside it
  previews continuously and settles into one undo step when you let go, exactly like dragging a
  number in Effect controls. **Clicking anywhere outside the picker closes it and keeps the
  colour**; **Apply** does the same from a button; **Cancel** puts back the colour you started
  with and closes.

  Beside the swatch sits the **dropper** — a small pipette. Click it and the tool arms (the
  pipette lights up so you can see it is armed), then move the pointer over the Viewer and a
  **magnifier** follows it. It appears only once the pointer is actually over the picture, and
  it sits the same distance from the pointer wherever you go — including the corners, where it
  simply hangs over whatever is next to the Viewer rather than shuffling out of your way and
  covering the pixel you were aiming at. At the edge of the *window* it hops to the other side
  of the pointer instead — above rather than below, or left rather than right — the way a
  tooltip does, at the same distance, so it stays out of your way there too. The magnifier shows a **9×9 grid** of the pixels under the pointer,
  each blown up to a square you can aim at, with **dashed lines between every pair** so you can
  tell one pixel from the next. A **solid border** rings the pixels that will actually be taken:
  **just the centre one** to begin with, and **Shift+scroll** grows it to 3×3, 5×5, 7×7 and 9×9
  so a grainy area averages out instead of grabbing one noisy pixel. Click to lift the value;
  press **Escape**, click the pipette again, or click away from the picture to put the tool away.
  Under the grid a strip says what you are about to take — the colour and its numbers, and the
  size of the patch.

  **The dropper is not only for colour.** It means "the value at this pixel", whatever value the
  thing you armed it from is after. Depth of field's **Focus** carries one: click the part of the
  picture you want sharp and Focus jumps to it. There the strip does not show a colour swatch —
  a colour would be meaningless — but **the name of the depth layer it is reading and the number
  it found there**, so you can see where the value is coming from. That pick reads the depth
  layer **rendered on its own**, not the composite: a depth pass is nearly always hidden, so what
  the composite shows at that pixel is not the number the effect uses. If the effect's **Depth
  invert** is ticked, the picked number is inverted to match, so what the strip says and what
  lands in Focus are the same thing.

  **The numbers are in the scale of what you are editing.** A theme colour or a solid's swatch
  is an ordinary eight-bit display colour, so it reads **0–255** and its hex is exactly the same
  value written another way. An effect's colour is not: Lumit works in **linear light at float
  precision**, where **0–1 is black to white** and a channel is free to go *above* 1 — a tint
  brighter than white, which is a real thing in this kind of maths and something several effects
  explicitly allow (up to 4, and one goes down to −1 for a lift). So those read as decimals, and
  you can drag or type a channel past 1 as far as that parameter allows. A hex is an eight-bit
  notation and cannot say "1.8", so on that scale the box shows the colour **clipped** into
  0–1, and a line under the swatches tells you when that is happening — rather than the box
  quietly claiming to be the whole truth.

  **How the pixels get there.** The Viewer's picture normally never leaves the graphics card
  (that is what makes playback cheap), so the dropper cannot simply look at what is on screen.
  Instead it asks the engine for a **window** of the picture — a 129-pixel square around where
  you are pointing, about 66 KB — and then cuts the nine-by-nine it shows out of that window
  itself. It asks by *where in the picture* you are pointing rather than by pixel number,
  because when the Viewer is showing the picture smaller than full size the engine is working
  on a smaller grid than the composition's, and only the engine knows which; its answer says
  which grid it used. That is why moving the pointer feels free: it is reading pixels it already has, and
  it only asks the engine again when you get near the edge of that window (or move the playhead,
  or edit something). Sending a whole 1080p frame across that boundary would cost about eight
  milliseconds every time, which is why nothing does it. The averaging is done in **light**, not
  in screen values, which is why one white pixel among nine averages to a ninth of the light
  rather than to mid-grey — and it means a picked colour matches what you sampled.

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

  Two things about LUTs were quietly wrong until K-271. A `.cube` file may declare the *range
  of input colours it was built for* — most say "nought to one", but a cube meant for log
  footage might say "−0.25 to 1.5". Lumit's plain-Rust reference honoured that; the version
  running on the graphics card ignored it and assumed nought-to-one, so such a cube came out
  with the wrong colours and nothing said so. The card now does the same conversion, and the
  test that compares the two paths includes a cube with an odd range, which the old shader
  missed by a mile. Separately, Lumit remembered a `.cube` **by its filename only** — so the
  loop everyone actually works in (export a grade, look at it, adjust, export again over the
  same file) showed you the *first* version until you restarted the application. It now
  remembers the file's last-changed time as well, so a re-exported grade appears on the next
  frame, and it keeps only the eight most recently used cubes rather than every one the
  session ever touched.
- `crates/lumit-core/src/lut.rs` — **reading a colour LUT (`.cube` file).** A LUT
  (look-up table) is a colour recipe a colourist bakes elsewhere: feed it a red/green/blue
  and it hands back a graded red/green/blue. The common `.cube` text format stores that as a
  cube of sample points — a 3D LUT is a grid (say 33×33×33) of "this colour in, that colour
  out", a 1D LUT is three separate curves, one per channel. This file reads such a file into
  memory and answers the one question the LUT effect (docs/08 §3.11) will ask millions
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
- **Retime, restarted as an ordinary property (K-197).** There are now *two* answers to
  "which moment of the source does this layer show?", and the new one is the simple one.
  A layer carries a `retime` field that is just an animatable number — the same kind of
  number Position and Opacity are — and its value *is* the source time, in seconds. Press
  Ctrl+Alt+T on a layer and it gains one; press again and it loses it (K-200 — it briefly
  had a second chord, Alt+Shift+T, which turned out to be a misremembering and which
  Windows steals for its keyboard-layout switch anyway; the command is in the Composition
  menu too, which nothing can intercept). While it has one, a
  **Retime** row appears in the Timeline's twirl-down above Transform, with the same
  stopwatch, the same diamonds and the same graph-editor lane as every other property,
  because it genuinely *is* every other property — there is no Retime-specific code in any
  of those places. Switching it on installs two keys running source time alongside layer
  time, so the picture does not move; drag the second key later and the clip plays slower,
  drag it earlier and it plays faster. **The two keys land on the layer's own start and
  end** — where it currently sits on the timeline, and where its ends currently are if you
  have trimmed it (K-213). That sounds obvious and was not: keyframes are stored in the
  layer's *own* clock, which is what makes a layer's animation travel with it when you slide
  the layer along the timeline, and the Timeline draws in the composition's clock. The two
  are converted for each other in exactly one place, at the engine boundary; before that,
  every keyframe on a layer that had been moved was drawn as though the layer still began at
  the start of the composition, and Retime's own two keys made it impossible to miss. That is deliberately *all* it does for now: no speed
  ramps, no ease presets, no freeze command. `Layer::source_time_at` is the one function
  that answers the question, so what the renderer decodes and what the frame cache files it
  under can never drift apart. The older, much larger machinery below still answers for
  documents that carry it.
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
  twenty-fourth of a second apart instead — actual motion, actual slow-motion. You type the
  rate straight into the box (0 means Native — the clip's own rate), and it's keyframeable
  like any other property: it has a stopwatch, so the conform rate can ramp over the clip if
  you want the slow-motion to ease in. It's the same "conform to N fps" idea editors know from
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
  **Per-layer motion blur** lives here too (`motion_blur_average`). Turn the composition's
  motion-blur master on and flip a layer's motion-blur switch — the **MB** toggle in the
  layer's switch cluster on the right of its Timeline row (or the "Motion blur" line in its
  right-click menu) — and that layer is drawn not
  once but many times — its *same* picture, nudged to where the layer sat at a spread of
  instants across the "shutter" (a slice of the frame, 180° = half a frame by default) —
  and those copies are averaged. A still layer averages back to itself exactly; a
  fast-moving one turns into a translucent smear along its path, thinning out where it only
  passed briefly, which is what real motion blur looks like. The averaging adds the copies
  up (each at 1/N strength) including their transparency, so a covered patch stays solid
  and a half-covered one goes half-transparent — a plain "Add" blend would wrongly keep
  transparency high, so there's a dedicated add-everything blend just for this. The layer's
  real blend mode, opacity, matte and mask are applied *once*, to the finished smear, not to
  each copy. Crucially the Viewer and the file export call this one shared routine with the
  same sub-frame instants, so a blurred preview and a blurred export match (K-031). Two
  follow-ups are noted in the code: a layer that blurs because its *parent* moves isn't
  covered yet (only the layer's own motion is sampled), and an inner layer of a
  *collapsed* precomp doesn't blur (so the Viewer and export can't disagree about it).
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
  As of the disk tier (`disk.rs`), frames also get **parked on disk**: a cache folder — in
  Lumit's own app-data area by default, or beside the project file or somewhere you choose
  (Settings → Performance) — collects rendered frames, written there compressed by a
  background thread, so closing and reopening a project doesn't start the cache from zero,
  and frames squeezed out of memory can come back without re-rendering. Each frame is one small file named by its content fingerprint; anything
  unreadable is silently deleted and re-rendered, so the folder is **always safe to delete**
  — it can make things faster, never wrong. The idle background fill now checks the disk
  before rendering: promoting a parked frame beats recomputing it. The timeline's cache bar
  grew a second colour for this: **mint** = in memory, plays right now; **blue** = parked on
  disk, ready to promote.
  The third tier is **VRAM**: the last few hundred megabytes of frames you actually looked
  at stay resident on the graphics card, so scrubbing back over them re-shows the exact
  texture with zero work — no upload, no colour maths. All three tiers answer to the same
  content fingerprint, so a frame is a frame wherever it lives — and a frame pushed out of
  one tier falls into the next rather than being lost (K-214; §9 walks the whole
  ladder).
- **Timeline guide lines** — the faint vertical lines through the lanes have a mode picker
  in the bottom bar ("Grid"): **beats** (the default — detected beats shine through every
  layer so cuts land on the music), **time** (a neutral second grid that subdivides as you
  zoom in, down to 10 ms), or **off**. The bright ruler ticks up top stay regardless.
- `crates/lumit-render/src/export.rs` — **writing video files.** Every frame of a comp is
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
  (`fit_contain` / `letterbox_resize` in `lumit-core`'s `pixels.rs`) is unit-tested. **Sound comes too**:
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
  The mixed track is kept **in step with the comp**: each frame Lumit works out a small
  fingerprint of what the comp should sound like (which layers make sound, and where each
  sits on the timeline). If you mute a layer, slide it, trim it, or delete it, the
  fingerprint changes and the track is re-mixed from the new state — and if muting or
  deleting leaves nothing audible, the track is dropped so it stops sounding at once. Before
  this, the track was mixed once when you pressed Space and never revisited, so those edits
  had no effect on what you heard (the GEN-4 audio fixes). The fingerprint is a plain,
  tested function, so "a muted layer is silent" and "a moved layer's sound moves with it"
  are checked without needing a sound card.
- **The live mix plan (`lumit-audio::mix::MixPlan`)** — how audio edits became instant, and
  how a feature film stopped eating all the memory. Originally, playing a comp *baked* one
  giant pre-mixed track the length of the whole comp — for a two-hour film that single
  track is gigabytes, and every solo/mute/move re-decoded and re-baked the lot (minutes of
  waiting, and the out-of-memory the owner hit). Now each footage file is decoded **once**
  into a shared, byte-budgeted store (it stays within the one Memory budget in Settings →
  Performance, half your machine's RAM by default), and the comp's audio is just a *plan*:
  "this file's samples play here, that file's there". The sound card's callback adds up the
  few numbers it needs for each moment as it goes — a handful of multiplications, nothing a
  sound card notices. Soloing, muting, moving or trimming a layer swaps in a new plan and is
  heard on the very next callback, about ten milliseconds later, with the clock untouched.
  A test proves the plan sounds *sample-for-sample identical* to the old baked mix, another
  proves a mid-play swap keeps the clock running.
- **Per-layer Volume and the waveform in the layer's own row (K-172)** — every audio-carrying
  layer now has an **Audio** group in its timeline twirl, next to Transform and Effects. Inside:
  a **Volume** value in dB — 0 is the file's own loudness, positive boosts (up to +50), and
  −100 or below reads "−inf", true silence. It keyframes like any other property (stopwatch,
  the ◄ ◆ ► arrows), which is how fades work: two keyframes, loud to silent. Under the volume
  sits a **Waveform** twirl that draws *that layer's* sound in its own lane — and because the
  drawing reads the layer's position fresh every screen refresh, dragging the layer slides its
  transients along with it, live. The old single waveform strip under the ruler is gone: it
  showed the whole comp's mixed sound in one place, went stale mid-drag, and told you nothing
  about *which* layer a spike belonged to. When a volume is keyframed, the fade is baked into a
  little list of loudness levels every ten milliseconds (a "gain envelope") that both the live
  player and the export mixer read — the same numbers, so what you hear is what you export;
  changing a volume re-plans the mix instantly, like every other audio edit above. Precomps
  carry their sound out with them: a nested comp's audio layers are walked recursively into
  the same mix (spans mapped onto the outer timeline, mutes and solos respected per comp),
  and a precomp layer's own Volume scales everything inside it — the gains multiply down the
  chain, so it has the Volume row too. And a purely-audio layer (a music file) shows no eye
  in the outline at all: there is no picture to hide.
- **Waveforms that sharpen as you zoom, and show what the sound *is* (K-280)** — a waveform
  is not the sound itself, it is a summary of it: for each column of pixels, how far the
  speaker cone swung up and down while that sliver of time went by. That means a summary is
  only ever as detailed as the stretch it was taken over. The first version took one summary
  of the whole file when you opened the lane and kept it, so zooming in stretched the same
  coarse picture until the wave was a staircase of blocks — you could see roughly where the
  loud bits were and never where the *hit* was, which is the one thing you zoom in for.
  Now the sound is summarised once at three levels of detail at the same time — think of the
  smaller pictures a phone keeps beside a photo so it can show a thumbnail without loading
  the whole thing — and the lane asks for whichever level suits the stretch it is currently
  showing, one bucket per pixel column. Zoom in and it asks again over a shorter stretch, so
  the wave *gains* detail instead of stretching. A summary runs out somewhere, though: once a
  pixel column is narrower than the smallest block, neighbouring columns start sharing one and
  the wave goes blocky again — so a short file also keeps the *actual samples* beside its
  summary, and once you are zoomed in that far the lane draws those instead. Fully zoomed, the
  waveform becomes a single continuous line tracing the sound, which is what it should be. A
  long file skips the sample copy: at Lumit's zoom ceiling you can never get close enough to
  an hour-long podcast for the summary to run out, so keeping tens of megabytes to answer a
  question nobody can ask would be waste. The summary is built once per file (a whole
  track takes a moment to read) and kept for as long as the app is open, shared between every
  layer cut from that file, with a firm ceiling on how much memory the lot may use.
  Two other things came with it. **Clips on a Sequence layer now draw their own waveform**
  inside their box — so a cut, which is a box on a row, finally shows the sound you are
  cutting — and it travels with the clip when you drag it, because it is drawn from the clip's
  own clock rather than pinned to the timeline. And the **multiwave**: instead of one wave, the
  lane can draw three at once, splitting the sound into bass, middle and treble. This matters
  because a modern mastered track is loud all the way through, so a single wave is a solid
  block whatever is playing — the phrase for it is "a sausage". The three are drawn *on top of
  each other* around the same centre line rather than side by side, getting brighter as the
  pitch goes up: the bass fills a soft wide body, and the hats and other sharp sounds land as
  bright thin spikes over it. The result is one waveform with its insides showing, so you can
  cut to the kick or to the hat and see which is which.
  There is a second switch beside it for **where the wave sits**. Normally it is centred, with
  the sound drawn going up and down from a middle line — but the two halves are mirror images,
  so half the row is saying the same thing twice. Turn *Waveforms rise from the bottom* on and
  the wave is folded onto the floor of its row: every column starts at the bottom and reaches
  up by however far the sound swung, which uses the whole row's height and is what a lot of
  editors draw. It applies to the plain wave and the frequency stack alike, and it is only a
  matter of drawing — nothing is re-read from the file when you flip it. It is on by default; Settings ▸ Interface ▸ Editing has a switch that puts the single
  plain wave back. (The idea is BLICK's, an editor that does the same thing.)
- **`L` opens a layer's sound (K-281)** — press `L` with layers selected and their **Audio**
  group opens; press it again and the waveform lane opens under it; a third time shuts the
  layer. The same three-tap shape `U` has for animated properties, and for the same reason:
  what you want is usually one of three depths, and inventing a modifier for each would be
  three shortcuts to remember instead of one key pressed once, twice or three times.
  `L` is also "play forward" in the NLE keyboard Lumit borrows (`J` back, `K` stop, `L`
  forward) — so inside the Timeline it now means the audio reveal, and everywhere else it
  still moves time. (Stepping a single frame is `Ctrl`+arrow — see the note below.) That kind
  of takeover used to be reported as a *clash* the user had to go
  and fix; it is now reported as a *shadow* and left alone, because there was never any
  ambiguity about which one runs — the panel you are in gets first refusal, and the app-wide
  meaning is the fallback. Settings ▸ Keymap says so in a quiet line above the table
  ("`L` — Reveal Audio in the Timeline, shuttle forward elsewhere") rather than a warning
  box, so you can see it without being asked to fix it (K-283). Two shortcuts fighting inside the *same* panel is still a clash,
  since nothing can tell those apart.
- **Stepping a frame is `Ctrl`+arrow (K-282)** — it used to be the bare left and right arrow
  keys. The trouble with that is that the arrows are *everybody's* keys: a list wants them to
  move the highlight, a text field to move the cursor, a canvas to nudge what is selected. As
  long as the app-wide transport owned them, nothing else could ever be given them without a
  fight. So the frame step took a modifier and the bare arrows went back to being available.
  `Page Down`/`Page Up` still step a frame with nothing held, so there is still a
  one-key way to do it.
- **Your project remembers where you were** — reopening a saved project no longer lands on a
  blank Viewer waiting for a playhead nudge. Which comp tabs were open, which one was in
  front, where the playhead sat, which layer was selected, and which twirls were unfurled all
  come back, and the first frame renders immediately. The mechanism is the same one that
  remembers the timeline column width: small notes in the app's own settings store, keyed by
  the project's file path — nothing is written into the project file itself, so sharing a
  `.lum` never leaks your window arrangement.
- **Project files carry no absolute paths (K-173)** — a tester about to share a project
  noticed their username sitting inside it: every media reference stored a full path like
  `/home/Their Name/projects/clip.mp4`. No longer. A saved project stores each file's
  location *relative to the project folder* (recomputed every save, with forward slashes so
  a Windows save opens on Linux) plus a small **content fingerprint** — the file's size and
  a hash of its first and last chunks. Where the file sits on *your* machine lives only in
  memory while the app runs. Opening a project finds each file by walking: is it where the
  relative path says? (This is why moving the whole project folder now just works.) If not,
  does an old save's absolute path still point somewhere real? If not, the fingerprint
  search combs the project's folder tree for a file with the same content — so footage that
  was reorganised into a subfolder is found by what it *is*, not where it was. Anything
  still missing is named in a notice and its reference kept intact.
- **When footage goes missing, you see colour bars** — the broadcast test pattern, the same
  one a television shows with no signal. The reasoning is that the alternative is worse: a
  missing layer that renders *black* looks exactly like a deliberate edit, so the mistake
  can survive all the way into an exported file. Bars cannot be mistaken for anything but
  "there is nothing here". They appear in the Viewer and in exports alike, for the same
  reason. In the Project panel the item wears a crossed-link icon and a **Relink…** button;
  pointing it at the file's new home also relinks every *other* missing file sitting in that
  same folder, in one undo step — losing a folder of footage is then one dialogue rather
  than twenty. The pattern itself is drawn by arithmetic at whatever size is needed, not
  loaded from a bundled image, so it is crisp at any resolution and adds nothing to the
  download. When something *is* missing, a toggle appears beside the Project panel's search
  box (and on any footage row's right-click menu) that filters the panel down to just the
  broken files and the folders leading to them — the "what else is broken?" view. It works
  alongside the search box rather than replacing it, so you can hunt for one missing clip by
  name; and when nothing is missing it tells you so plainly instead of showing an empty
  panel that looks like a fault.
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

  One thing worth knowing about how the panel and the engine share a marker (K-270). The
  panel can see and change three things about one: where it is, what it is called, and which
  marker it is. The engine knows three more: whether it was detected or placed by hand (and
  how confident the detector was), how long it lasts if it spans time, and any fields a
  *newer* version of Lumit wrote that this one has never heard of. When you drag a marker,
  the panel sends the whole list back — and it used to send back only the three things it
  knows, which quietly reset the other three. Move a detected beat by one frame and it
  stopped being a beat: **Clear beat markers** would then leave it behind, because nothing
  was left to say where it came from. Now the write-back is a *merge* — each marker is
  matched up with the one already there by its id, and everything the panel cannot see is
  carried straight over. A marker you have just made has nothing to carry over, so it is a
  plain user marker, which is what it is.
- **The timeline waveform** — a strip under the ruler draws the composition's mixed audio as
  a min/max envelope on the same time axis, so the beats sit right above the transients that
  made them. It's built by `waveform_peaks` (in `lumit-audio::mix`), which buckets the mono
  mixdown into (min, max) pairs — a pure, tested down-sample — computed once when the comp's
  audio is mixed for playback.
- The **graph editor** — the curve view of the Timeline, like After Effects' graph button
  (the Graph toggle in the Timeline toolbar, or `Shift+F3`). Switching it on keeps the layer
  outline on the left and swaps the lane area for **one full-height pane of curves**, under
  the same time ruler, zoom and horizontal scroll as the lanes — a frame sits at the same
  x whichever view you are in, and the playhead line runs through both.
  **Choosing what to graph.** Click a property's *name* in the outline (twirl the layer
  open first) and its value-over-time appears as a line — even a property with no keyframes
  shows as a flat line of its value. **Ctrl+click** more names to add them, **Shift+click**
  to take a whole run of rows, across layers; each curve gets its own colour from the
  theme's curve palette, and the property's name in the outline is tinted to match, so you
  always know which line is which. A property with more than one axis shows every axis —
  Position is an x curve and a y curve, like AE's red/green pair, with a coloured dot per
  axis beside the label. Selection rides on the *name* on purpose: clicking a value field
  or a stopwatch never re-aims the graph, but *editing* a value or adding a keyframe does
  select that property, so the curve you see is the one you just touched.
  **Reading the curve.** Each key's glyph tells you its interpolation at a glance — a
  diamond is linear, a circle is eased (bezier), a square is a hold. The curve between keys
  is drawn by a Dart copy of the *engine's own* evaluator (`graph_maths.dart`, pinned to
  `anim.rs` by docs/impl/keyframe-eval.md and golden tests), so the shape on screen is
  exactly the motion that renders — and drawing it costs no bridge calls at all.
  **Editing keys.** Drag a key to move it in time *and* value at once — one undo step per
  property, even when a drag moves a whole selection. Drag a box over empty pane (the
  *marquee*) to select many keys; Shift or Ctrl adds to the selection; a plain click on
  the background clears it; **Ctrl+click** on a curve plants a new key right on it;
  Delete removes the selected keys (the last key of a curve leaves a static value holding
  what it held). Keys may pass each other in a drag — the curve just re-sorts — but two
  keys can never share a frame: a drag that would collide simply stops, nothing is lost.
  The magnet in the bottom bar decides whether dragged keys land on whole frames.
  **Shaping a key (bezier handles).** New keys are linear. Select some and press **F9**
  (or the **Bezier** button in the bottom bar) to *easy-ease* them — AE's smooth default:
  the curve arrives and leaves flat, and the key grows two **tangent handles**, one
  reaching toward each neighbour. Drag a handle to shape the curve: its steepness is the
  **speed** there (units per second) and its reach is the **influence** (how much of the
  gap the ease covers). The two handles are **in sync** by default — they behave as one
  straight line through the key, so dragging one swings the other round to stay opposite it
  and motion glides *through* the key. The partner keeps the length it *looks* on screen
  rather than its length in values, at every angle — so it never appears to shoot out as
  the pair steepens, and swinging one handle out to near-upright and back brings the other
  home exactly as long as it started. Two small things make that hold. A tangent can never
  be made *perfectly* vertical, only a hair off it: an upright tangent spans no time at
  all, and there is no speed that describes such a thing, so it is the one shape the editor
  could not undo (the difference is well under a pixel — no ease you shape can tell). And
  each handle's length is remembered as you leave it, rather than worked back out of the
  ease, which at that extreme is where the arithmetic gets thin. (One thing worth knowing
  about the see-saw: the partner moves when the line *rotates*. Dragging a handle straight
  out from an already-steep tangent lengthens it without turning it much, so the other side
  barely stirs — that is the geometry, not a stuck handle.) Hold **Alt** as you start a
  drag to break the two apart and shape a corner; Alt-drag again re-joins them. `Shift+F9`
  eases only the way *in*, `Ctrl+Shift+F9` only the way *out*, and the **Linear** and
  **Hold** buttons put selected keys back to straight lines or steps.
  **Value and speed.** The bottom bar's **Value / Speed** buttons switch what the pane
  plots (docs/07 §5.1). The speed graph is the value curve's *exact derivative* (K-080) —
  an eased key reads as a smooth dip to zero, a straight run as a flat line, a hold as
  zero. Here each key is really **two dots** — the speed coming *in* and the speed going
  *out* — that drag up and down independently, each with a single horizontal **influence
  handle**; this is AE's speed graph, and both views edit the same store, so shaping one
  always updates the other losslessly.
  **Framing and the wheel.** Vertically the pane **auto-fits** by default: the curves,
  every handle tip and any bezier overshoot stay in view, and the framing holds still
  during a drag so the curve isn't sliding under your cursor. Toggle **Auto fit** off in
  the bottom bar to take the vertical axis yourself: a plain wheel pans it, **Alt+wheel**
  zooms it about the cursor, and **F** re-frames whenever you want. **Ctrl+wheel** zooms
  time about the pointer and **Shift+wheel** scrolls sideways — the same bindings as the
  lane view, because it is the same axis. A y-axis of faint gridlines down the left edge
  labels the values.
  **Copy and paste.** `Ctrl+C` copies the selected keys and `Ctrl+V` pastes them into the
  selected properties, the earliest key landing on the playhead. It is not a graph-only
  gesture: keys boxed up on a *lane* copy and paste exactly the same way. The in-app
  clipboard keeps everything — times, values, both sides' easing. The *system* clipboard
  gets the same keys as a **tab-separated table** headed `Lumit <version> Keyframe Data`:
  the rate and source size, then a row per frame with a column per value — and, after
  those, two more columns per value carrying that key's easing (`linear`, `hold`, or
  `bezier(speed,influence)`). So a copied ramp can be read by a script, dropped into a
  spreadsheet, or carried into another tool *with its shaping intact*, which is the part
  a plain values table always loses. Reading is deliberately forgiving: a keyframe table
  from another editor — same shape, no easing columns — pastes in as linear keys rather
  than being refused.
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
- **Any footage layer can become a Sequence layer, and come back.** Layer ▸ Convert to
  sequence layer, or right-click the layer in the Timeline; the same rows offer the way back
  once it is one. The Vegas preference decides what an *import* becomes, never what a layer
  is allowed to be — somebody without that preference ticked may still want one row cut into
  pieces, and somebody who tried it has to be able to change their mind. Coming back keeps
  the clip's source, its trim and its ramp: a single clip spans its whole layer, so the two
  are the same map on the same clock and nothing is converted. A row of **several** clips
  refuses rather than silently keeping one and throwing the rest away — which clip the layer
  should become is a decision only you can make, so delete the others first.
- **A Sequence layer has no Retime of its own.** Ctrl+Alt+T and the Composition menu's
  Enable Retime are greyed out on one, and the menu says why. Its *clips* each carry a
  retime, ramped in the sequence view — a second map over the whole row would be a rival to
  those, which is exactly the situation the one-retime-system work existed to end.
- **The sequence view: a Sequence layer's row, grown tall (K-248).** Double-click a
  Sequence layer — its name in the outline, or its bar in the lanes — and its row **opens in
  place** rather than swapping the Timeline for another tab. That is deliberate: you cut
  against a beat you can see, so the music, the other layers and the ruler all have to stay
  on screen while you do it. (An earlier decision put this in a tab of its own; K-248
  changed it.)
  It opens **six rows tall, three and three**, and the layer's own bar row is the **top of
  the three**: while the view is open the bar itself stands down and the clip region takes
  that row, so the three read as one block with no seam through them — the row seams are
  suppressed across the whole view, and a hairline between the first and second row was
  exactly the thing that made it look like a bar with strips stuck underneath. Opening
  therefore adds the two rows below the bar rather than pushing a new strip under an
  unchanged one, and a collapsed Sequence layer looks precisely as it always did. An open
  view carries a faint accent outline around the whole of it, and a line across the bottom
  of the clips says where the graph begins. The top three hold the **clips**: each drawn
  where it sits with its playback speed on it, draggable along the row by its body and
  trimmable by either edge, and cuttable with the razor or `Ctrl+Shift+D` exactly as the bar
  above is — they are the same commands on the same layer. The bottom three hold the
  **speed envelope**. Everything below the layer moves down by those six rows, so the view
  is part of the table rather than something floating over it, and the row seams stop at its
  edges: an open view is one cell, and ruling it into six would draw lines straight through
  the graph.
  **The envelope is the same editor as the graph's Vegas lens**, over the same keyframes: a
  point per key, its height the playback speed in per cent, straight lines between.
  `Ctrl`-click or double-click the line plants a point, `Alt`-click lifts one, and dragging
  a point moves it **both ways**: up and down for the speed, left and right for *when* the
  ramp reaches it — which matters as much as the speed does when you are cutting to a beat.
  Moving one sideways keeps the speed it had and **re-works the frames around it**, so every
  stretch of the line stays straight. That is not a detail: a keyframe stores a *speed*,
  while the stretch between two of them has an *average* — move a key in time and the
  average changes while the stored speed does not, so a line that was straight quietly bows
  and the graph starts describing playback the points do not say. Re-running the sums
  through the same speeds puts it back on its line.
  A point carries a little readout of the per cent it is setting, because reading a speed off
  the height of a dot against an axis that reframes as you drag is not aiming. The two end
  points stay put: they are the clip's own edges, and a clip's length is trimmed on the clip,
  never on its speed curve.
  The 100% and 0 reference lines are **dotted**, so the graph's own furniture never reads as
  the row seams that rule the rest of the table — solid, they were the same mark meaning two
  different things. A clip nobody has retimed draws the flat 100% it is actually
  playing, and dragging that line moves it **as one level** — the obvious reading of dragging
  a flat line is "this clip plays at that speed", which is what Vegas's first envelope point
  does too. Plant a point and the line has a shape worth keeping, so from then on a drag
  moves only the point it has hold of.
  The vertical axis **grows as you drag**. It opens on 100% down to −25%, with air either
  side so a flat 100% line is not sitting on the strip's own top edge, and if you drag a
  point past either end the axis reframes to hold it — so a fast ramp or a hard reverse
  stays inside its own three rows instead of running out over the layers below.
  **The layer's bar stays a plain bar.** Cutting a clip used to draw split lines up there;
  the clips and their edit points belong to the view now, and the lines only said the same
  thing twice. What the bar does show is where its clips are **not** — the gaps wash out
  faintly, the way a trimmed footage layer shows the source it is not using — so a collapsed
  Sequence layer still tells you it has holes in it.
  **Dropping a clip on another overwrites it.** Drag a clip over its
  neighbours and, when you let go, the one you dragged wins its whole span:
  anything buried under it is gone, anything covered at one end is trimmed back
  to its edge, and anything it lands in the middle of becomes two clips with
  the dropped one between them. It is destructive exactly where it lands and
  nowhere else — no edit point beyond the drop moves, and nothing ripples. The
  pieces left either side keep playing the frames they played, because the
  trims and the split go through the same arithmetic the razor uses.
  **A clip dragged back past the start takes the layer with it.** A clip's place is measured
  in the layer's own time, which cannot go negative — so dragging one before the start of the
  row carries the whole layer earlier instead, exactly as dragging any other layer's bar
  before the start of the composition does. Every *other* clip is pushed the same amount
  later in layer time, so it stays precisely where it was on the composition's clock and only
  the clip you dragged actually moves.
  **The layer's bar is its clips' extent**: first clip's start to last clip's end. Delete the
  last clip and the end of the bar comes in with it; a gap in the middle stays a gap, renders
  transparent, and is never closed for you.
  **Clips can be reordered.** An earlier rule required a Sequence layer's clips to run
  forward through their source — you could remove and space them, not shuffle them — which
  existed so a camera track could replay through the cuts. K-248 drops it: a tracker will run
  on the whole unaltered footage and be mapped through the sequence instead, and reordering
  is what anyone coming from Vegas expects.
  **A selection box** works in the envelope: press anywhere that is not on a clip's own line
  and drag, and every point inside is caught (`Shift` adds to what was there). Dragging any
  caught point moves the whole set by the same amount, keeping its own spread — a selection
  shifts a shape rather than flattening it — and a box drawn across two clips moves the
  points in both.
  **A row's shape copies onto another layer.** Right-click a clip: *copy this clip's shape*,
  *copy the whole row's shape*, *paste shape onto this layer*. A shape is where the cuts
  fall, where the gaps are and how each piece is ramped — and deliberately **no media**,
  which is the whole point. Cutting a depth pass to the same beats as the footage it belongs
  to is work nobody should do twice by hand, and doing it by eye guarantees the two drift
  apart. Pasting keeps the target layer's own source and applies the shape as far as that
  row reaches: the piece straddling the end is trimmed to it and anything wholly past it is
  dropped, so a shape taken from long footage lands sensibly on short footage.
  Per-clip **thumbnails** are still owed ([TODO.md](TODO.md)): they need a decode at a given
  source moment, which no engine path offers yet.
- **Video arriving as a Sequence layer (K-246).** With that switch on, dropping a video into
  a composition gives you a **Sequence layer** holding one clip, rather than a plain Footage
  layer — a row you can cut into pieces and ramp piece by piece, instead of a single
  indivisible block. A **still image never is**, and the rule for telling them apart is
  worth knowing because it is not the file extension: a still probes as a video stream too,
  one frame long, so the question the engine asks is *does this run* — is it longer than a
  single frame at its own rate. Image sequences will answer yes by that same rule the day
  they exist as a footage kind, with nothing here to change.
  Media that will not open at all answers **no** and arrives as a plain Footage layer.
  Guessing towards the simpler shape when there is nothing to go on is the cheaper mistake:
  a layer that should have been sequenced is one command away from being one, and a wrongly
  sequenced layer is more to undo.
  It is **one call into the engine**, not "make a layer, then convert it" — so it is one
  undo step, and every route into a composition (a drop on the timeline, footage dropped on
  the New composition button, the menus) goes through the same funnel and cannot disagree
  about what a video import becomes.
- **Planting and lifting keys on a curve, and holding a drag to one axis.** Four small
  gestures the graph editor was missing, all of them working in either lens and whether or
  not Vegas mode is on.
  **Double-click the curve to plant a key** where you clicked. The key takes the value the
  curve already has there, so nothing moves — a planted point is somewhere to grab, not an
  edit. On the Vegas envelope it also takes the speed the line already reads there, so the
  straight line it lands on stays straight. `Ctrl`-click does the same thing, and so does a
  plain click with the **Pen** armed.
  **`Alt`-click a key to lift it**, or click one with the Pen armed. Everything else on the
  channel stays exactly as it was, and the last key of a channel refuses to go — an animated
  property with no keyframes at all is not a state worth being able to reach by accident.
  (Double-clicking a *key* deliberately does nothing. Flutter holds every single tap back
  until the double-tap timer expires once a double-tap is registered on the same widget, and
  a single click on a key — selecting it — is the commonest gesture in the pane. It is not
  worth a visible delay on every selection to save one modifier.)
  **Hold `Shift` while dragging a key** and it moves in one direction only: along time, or
  along value, whichever way the pointer has travelled further **in pixels**. Pixels rather
  than the numbers themselves, because the two axes carry different units — seconds against
  source-seconds, or per cent — so comparing values would make the constraint depend on how
  far the graph happened to be zoomed rather than on the gesture your hand made. It follows
  the pointer live: sweep the other way mid-drag and the constraint switches with you, and
  letting `Shift` go gives back the full travel rather than losing whatever it suppressed.
- **The Vegas speed envelope (K-247).** With *Retime opens to Velocity* ticked, the Retime
  channel's speed view stops being the After Effects speed graph and becomes the thing Vegas
  editors already know: a **velocity envelope**. The difference is what a keyframe is on it.
  In the After Effects speed graph a key has *two* dots — the speed coming in and the speed
  going out, dragged separately — because a key is a corner where two spans meet. On the
  envelope a key has **one** dot, and its height simply *is* the playback speed at that
  moment: 100% is normal, 0% is a freeze, 300% is three times as fast, and dragging below
  zero runs the footage backwards. Between two dots the speed runs in a straight line, which
  is what a Vegas ramp is.
  **Dragging a point changes the frames after it, and moves nothing.** Raise a point and the
  footage from there on plays faster, so by the end of the layer you are further into the
  clip than you were — but the layer does not get longer, no keyframe moves in time, and
  nothing ripples. That is the whole promise the editor is built around: a beat you already
  synced stays synced (the "beat-sync covenant"). The **first** point is pinned, so a clip
  always starts on the frame it started on however you re-speed it.
  **It is the same curve, not a simplified picture of it.** This is worth spelling out
  because it is easy to assume otherwise. The Time view underneath is made of ordinary
  bezier keyframes, and one might expect the straight lines of an envelope to be an
  approximation drawn over them. They are not: if you set a span's value change to exactly
  the area under the envelope's straight line — the average of the two speeds times the
  span — the bezier's own speed comes out *exactly* that straight line, with the curvature
  term cancelling to nothing. So the envelope and the Time curve are two readings of one
  set of keyframes, and a ramp built in either is read back honestly in the other. (There
  is a test that samples along the curve and checks it sits on the envelope's line; if that
  ever fails, the two views have started disagreeing.)
  The axis opens at **125% down to −25%** (K-250), and only ever grows. The headroom above
  normal playback is the point of the top figure: at exactly 100 the flat line an un-retimed
  clip draws sat on the graph's very top edge with nowhere to go but up and out of sight,
  which reads as a ceiling rather than as the ordinary speed it is — and speeding a clip up
  is the commonest thing anyone does here. The negative strip is deliberate too, so it is
  visible that dragging below zero is allowed and what it does. Push a ramp to 850% and the
  axis reframes to fit it — though **not while you are dragging**: the axis is frozen for
  the length of a gesture (or the range would grow as you drag, which stretches what the
  next pixel is worth and sends the value off exponentially), and the drawing is clipped to
  the graph's own bounds so a point taken past an edge never draws over the rows beside it.
  The framing catches up when you let go.
  **The picture follows the point as you drag it.** A retime decides *which frame* of the
  source is on screen, so unlike dragging a position — where the pixels are already in hand
  and only get moved about — nothing can update until the provisional map reaches the render
  plan. It rides along with the frame request and is patched onto a throwaway copy of the
  document, so the picture keeps up without a single edit being written or an undo step
  being made.
  Two smaller things came with it. The Retime row's **name is now clickable**, like every
  other property row's — it never was, which meant the Retime curve could be built but not
  opened. And with the preference off, everything above is unchanged: the speed view is the
  ordinary two-dot graph, for Retime and for every other property, in both modes.
- **One way to retime a layer, not two (K-249).** For a while Lumit could retime a footage
  layer by either of two entirely separate mechanisms, and this is the note about the day
  that stopped. The **Retime property** is the one described just above: a keyframable
  value, in the graph editor, saying which moment of the source is on screen at each moment
  of the layer. The other was older — a *segment store* living inside the layer's Footage
  source, built from spans of eased speed, and edited by three rows on the Source card (an
  on switch, a speed percentage, a reverse tick). Two systems, one job, and only the
  property could draw a ramp, so the segment rows could not even show what the graph had
  made — they said "varies" and refused to be edited.
  The property survives and the segment rows are gone. Retiming a layer is **Ctrl+Alt+T**
  (or Composition ▸ Enable Retime) and then the graph, everywhere, for every layer.
  Three consequences worth knowing:
  **Old projects convert themselves.** A project saved by the older build has its segment
  store turned into the identical keyframes when it opens — the same shape, the same source
  moment at every point of the layer — and the file's schema version moves from `0.1.0` to
  `0.2.0` to record that this has happened. Nothing is lost: the conversion goes through the
  segment store's own reader, which was written to describe a curve exactly.
  **"In-between frames" moved rather than died.** How a fractional moment becomes pixels —
  Nearest, Blend, or optical Flow — was stored *inside* the old segment store, which meant
  the setting existed only if you used that particular retiming system. It was never part
  of the map (the specification has always said the two are independent), so it now sits on
  the layer in its own right, beside the Retime property, still on the Source card and still
  saying what it always said. A layer that is not retimed has one too, which is right: a
  25 fps source in a 60 fps composition is already being asked for frames that do not exist.
  **A drag override nobody built came out with it.** The renderer carried a parameter for
  overriding a layer's retime mid-drag, threaded through every layer of the preview and
  export call chain — and constructed by no code anywhere. It went; a Retime drag arrives as
  a provisionally-edited document, exactly like a Position drag, and the plan reads the map
  from that.
- **Property rows in the Timeline** (K-072) — twirl a layer open and each of its animatable
  properties (Position, Scale, Rotation, Opacity, and the 3D ones) gets its own row: on the
  left a stopwatch to turn animation on or off, the property's name, and its current value;
  on the right, along the same time ruler as the layer bars, a little diamond at each of that
  property's keyframes — so you can see *which* property is keyed *when*, not just that the
  layer has keys somewhere. Click a property's name to open its curve in the graph view.
  Once a property is animated its row also carries a **keyframe navigator** — `◄ ◆ ►` — where
  the middle button adds a key at the playhead (or removes the one already there) and the
  arrows jump the playhead to the previous or next key, so you can walk a property's keys
  without hunting for them by eye. (Effect parameters get this same navigator now too — an
  animated Glow radius or blur amount steps and adds/removes keys from its row exactly as a
  transform property does.)
  **The diamonds on the lane are live, not just a picture (notes 2.1/2.6).** Click a keyframe
  diamond to select it — it wears a ring — and **drag it left or right to change its time**;
  while the **magnet** (the bottom-bar toggle, on by default) is lit it snaps to the nearest
  whole frame, exactly like a key drag in the graph editor. On the lane only the *time* moves
  by dragging — a key's *value* is shaped in the graph editor — but its **easing is not graph
  work**: **F9** and its family (and the bottom bar's Linear / Bezier / Hold buttons) act on
  whatever keys are selected here, so easing a key never means opening the graph. The easing
  chords are bound in the graph context in docs/07 §15; the Timeline honours them because the
  two views are one panel with one key selection between them. (Touching a diamond is what
  selects it — the same gesture that drags it, which is why a click with no movement still
  counts: the lane's drag recogniser is alone in its arena, so it wins on release either way.
  That matters more than it sounds: a lane at fit zoom is a third of a pixel per frame, and a
  competing tap recogniser would swallow every drag short of twenty frames.) **Shift-click** adds a key to
  the selection, **Ctrl-click**
  toggles one, and dragging over empty timeline space draws a **marquee** box that selects
  every key it covers — *across different property rows*, so you can grab, say, a Position key
  and a Rotation key together and nudge them in step. Hold **Shift** while you drag the
  marquee to add to the current selection instead of replacing it. A drag that begins on a
  layer bar still moves the bar, and one that begins on a key drags the key — the marquee only
  opens on genuinely empty space. Every key you move commits through the normal document
  edit, so it is one undo step and the preview re-renders exactly as the export will. (A
  linked Position/Anchor/Scale row shows the union of both axes' keys as one diamond per time;
  dragging it moves *both* axes' keys at that time, keeping the pair in step.)
  You can also **highlight several property rows at once** by their names, the usual list way
  (note 2.6b): **Ctrl-click** a name to add or remove that one row, **Shift-click** to select
  the whole run of rows between it and the last one you clicked. A plain click still picks a
  single row and opens its curve; a Ctrl/Shift-click only changes the highlight and leaves the
  graphed channel alone. This works the **same for every kind of row** (UI-6): transform
  properties, effect parameters and a footage layer's Retime "Time"/"Velocity" row all select
  and multi-select alike, and one selection can mix all three (a plain click on an effect or
  Retime row single-selects it, exactly like a transform row). Once you have a set highlighted,
  the command palette's **Key selected properties** adds a keyframe to every one of them at the
  playhead in a single undo step — so you can key several channels at the same point at once,
  each holding its current value.
  **Copy and paste keyframes (note 2.2).** With keys selected, **Ctrl/Cmd+C** copies them —
  bezier handles and all — remembering each key's time relative to the earliest one in the
  set. Move the playhead and **Ctrl/Cmd+V** drops them back down at the playhead, keeping their
  spacing and their easing, and **overwriting** any key that already sits at the same time. A
  paste is one undo step. (Copying a key on a linked Position/Scale/Anchor row carries both
  axes, so the pair pastes back together.) These only fire when no text box is focused, so
  typing still copies and pastes text as normal.
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
  In its **Time** lens the row shows a source timecode you can scrub, and the viewer now
  **updates live as you drag it** — because changing the retiming changes *which frame of the
  footage* is on screen, the preview re-fetches that frame while you drag rather than waiting
  for release (the same instant feedback a transform or effect value already gives). Every
  keyframe row across the whole layer area — transform properties, the Retime Time/Velocity
  row and effect parameters — also shares **one** `◄ ◆ ►` add/step navigator now, so they look
  and behave identically wherever you meet them.
- **Getting around the Timeline** — the panel is split into the **layer outline** on the left
  (the stack of names, stopwatches and toggles) and the **lane area** on the right (the time
  ruler with each layer's bar on its own *lane*). Each bar wears its layer's **label
  colour** — the same chip its outline swatch shows (K-189) — so a tall stack reads at a
  glance and picking a new label recolours the bar. Drag a layer's bar body to slide it
  earlier or later in time (one undo per drag); drag its ends to trim — the pointer turns
  into the horizontal resize arrow over the last few pixels of each end, which is where
  the trim grab lives. **The ends know what the source holds** (K-211): a Footage, audio
  or Precomp layer stops where its media does — you cannot drag its head earlier than the
  clip's first frame or its tail past the last, and when an end is sitting on that limit a
  small triangle appears in that top corner of the bar to say so. Every generated kind —
  Solid, Text, Adjustment, Null, Camera — has no source to run out of, so both its ends go
  wherever you drag them and neither wears a triangle. Switch **Retime** on and the limits
  come off (and the triangles go): a retimed layer chooses which source moment each of its
  own frames shows, so it can be stretched to any length you like. Sliding a bar along the
  timeline is never limited — moving carries the content with it, so a clip that fits its
  source still fits it wherever it lands.
  **Trim one back and you can see what you cut off**: a faint outline runs behind the bar
  as far as the media reaches, so the trimmed-away head or tail shows as an empty extension
  of the clip — drag the end back out and the bar fills it again. It appears only when there
  is something to show, and never while Retime is on.
  **Turning Retime off puts the layer back on its source.** A retimed layer can be any
  length, so when you switch the retime off Lumit has to give it one again, and it does that
  from the frame you are already looking at: the layer keeps its start, still shows that
  same frame there, and plays at normal speed from there until the footage runs out (or
  until where the layer already ended, if that came first — it never gets longer than it
  was). So a clip that started on its first frame simply plays from the beginning again,
  and one parked half-way in carries on from half-way in. It is one undo either way.
  A layer twirled
  open shows its **keyframes as diamonds on the lanes**: drag a diamond to move that
  keyframe in time, or drag a box on empty lane space to select the diamonds inside it.
  Dragging never scrolls the timeline — the wheel and the scrollbars do: a plain wheel
  moves the rows, **Shift + wheel** scrolls sideways, and **Ctrl + wheel** zooms time
  around wherever the pointer is. The two halves scroll vertically **as one table**, with
  the shared scrollbar on the lane side's far right (in Graph view each side gets its
  own, and the outline keeps that strip reserved either way so the columns never jump).
  Along the bottom of the lanes sits a small bar: `−`, `+` and **Fit** with the current
  zoom per cent, the **magnet**, and the horizontal scrollbar that moves the view once
  you are zoomed in. The magnet — on by default — is what makes a dragged keyframe land
  on a whole frame; switch it off and a keyframe can sit between two frames, which is
  occasionally what a fast move needs.
  The Lane/Graph view buttons live in the Timeline's toolbar; Graph is only a change of
  what the right side *draws* — the outline stays identical between the two, so twirling
  a layer open shows the same rows either way.
- **Working the layer outline** — a few habits from other editors now work the way you
  would expect. The outline's columns sit in **four groups** (K-188), left to right:
  first the **eye, speaker, solo star, padlock and shy** switches; then the **twirl, a
  small label-colour chip, the layer's stack number and its name**; then the
  **flow-or-collapse glyph, an fx bypass switch, motion blur and 3D**; then the **Matte,
  Blend and Parent** dropdowns (Parent is the same parent-and-inherit link the Effect
  Controls tab offers — pick another layer and this one rides its transform). The row of
  tiny icons over the columns names each group — and it is also a handle: **drag a
  group's header to move the whole group**, which is how you reorder the columns, and
  **drag the little line after a group to make it wider or narrower**. Only that group
  changes; the others keep the width you gave them, so the whole layer area grows or
  shrinks to suit. Whatever lives in a group grows with it — widen the switches group and
  the value boxes under it widen to match, so a long number always has room.
  **Drag a layer by its name** to move it up or down the stack — drop it on another
  row and it takes that row's place, in one undo step. (Dragging its *bar*, over in the
  lane area, moves it in time instead.)
  **Clicking a property** (a Position, an effect's Radius, a Volume) selects it, and
  everything it belongs to — its effect, its layer — lights up faintly behind it, so you
  can see at a glance whose property you are looking at. That is also what the graph view
  will use to know which curve you meant. The eye
  and speaker swap to a closed eye and a muted speaker when off, so a hidden or silent
  layer reads at a glance. **Shy** is list housekeeping borrowed from After Effects: mark
  the layers you are done fiddling with as shy, press the shy filter in the Timeline's
  toolbar, and they vanish from the *list* — never from the picture — until you press it
  again. The padlock freezes a layer: while locked, its bar will not slide, its ends will
  not trim, it will not rename, reorder or delete. The label chip opens a small
  eight-colour picker, and the colour you pick is also the colour of the layer's **bar in
  the lane area** — each kind of layer starts on its own bright chip (footage azure,
  solids amber, precomps violet, text mint, cameras teal, sequences indigo, adjustments
  magenta), so a fresh stack is tellable apart before you name anything. It changes
  nothing about the picture itself. The toolbar above the columns shows the playhead twice —
  as `HH:MM:SS:FF` timecode and as a plain frame count like `f72` (both start at zero,
  so frame 0 is 00:00:00:00) — plus the layer search, and a **master motion blur**
  button: the comp-wide shutter switch that decides whether the layers whose own motion
  blur switch is on actually blur. The master is per comp — a nested comp inside a
  Precomp layer has its own master and follows that one, not the parent's. The
  thin line between the
  outline and the lanes is a handle — drag it to widen or narrow the outline; if you drag
  it hard against a limit and keep pushing, it now waits for the cursor to travel back to
  where the handle actually is before it starts moving again, rather than lurching the
  instant you reverse. **Double-click
  a layer's name** to rename it in place (Enter or clicking away keeps the change, Escape
  throws it away); **drag a name up or down** to reorder the stack (top = renders last, one
  undo per move, with an accent line showing where it will land); and **right-click a name**
  for the layer menu — rename, add an effect (by category) or a mask, duplicate, delete, and
  the solo and enable toggles, all in one place. Names are plain labels now, so dragging over
  one never smears a text selection across it. Opening a layer's twirl no longer also unfurls
  its Transform group — Transform starts closed, so you see a tidy list of section headings
  (Transform, Effects…) each sitting in its own faint bar, and open only the one you want.
- **Reordering effects** — in the Effect Controls panel (or a layer's Effects group in the
  Timeline) each effect's name is a drag handle: drag it up or down to restack the effects,
  one undo step. Each effect's title sits in its own subtle bar so it is obvious where one
  ends and the next begins. Dragging an effect out of the **Effects & Presets** browser now
  drops onto the *whole* layer row — the name side as readily as the lane — and onto the
  Effect Controls panel too, not just the sliver of lane past the bar.
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
- **Null layers** (Composition → Add null layer) — an invisible layer that is nothing but a
  transform: no source, no size, no pixels, and it never appears in the picture. Its whole
  job is to be something to parent *to*. Park a null in a comp, parent five layers to it,
  and moving, rotating or scaling the null moves all five together while each keeps its own
  animation on top — the standard rig for a camera-style push or a group that has to travel
  as one. Change your mind about the move and you re-animate one layer, not five.
  Mechanically it is the emptiest kind in the model: the evaluation graph skips it entirely
  (it emits no node, so it costs no drawing pass), the renderer returns no pixels for it,
  and it answers "no" when asked whether it has a picture — so it is never offered as a
  matte source or as a layer-valued effect parameter, where picking it would have quietly
  produced nothing.

  You *can* still drop an effect on a null, and Lumit deliberately lets you (K-274). Nothing
  is drawn — there is no picture for an effect to change — so the Effect controls panel says
  so once, quietly, rather than refusing the drop. The reason is that an effect is not only
  a picture operation: its parameters are values, animatable like any other, and a null is
  the natural home for a value that is meant to drive *other* layers. Put a slider on a null,
  animate it, and point another layer's expression at it, and the fact that the null itself
  renders nothing is exactly the point. So the stack on a null is stored, keyframed and
  sampled like any other; only the drawing is absent. It is *not* invisible to the frame cache, though, and that distinction
  matters: the transform still feeds the key that decides which cached frames are still
  good, so nudging a null correctly throws away the cached frames of everything hanging off
  it. A null draws a grabbable 100×100 wireframe box in the Viewer (K-230 — After Effects'
  own convention; the box is a drawing convention, not pixels), so it can be clicked and
  dragged like any layer. One honest limit for now: effects added to a null are accepted and
  then never run, since there are no pixels for them to touch; harmless, the same as on a
  camera, but not yet either refused or labelled.
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
  a trap worth knowing wherever drag regions are written: a region that senses dragging does
  not automatically step aside for an ordinary button drawn on top of it the way a plain
  click does — dragging is tracked from the moment the mouse is pressed, not by "whoever
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
  the Viewer — the one under the playhead — and re-reads it every time it redraws, so the
  scope now **follows the picture while it plays** (K-130): each time you press play, Lumit
  keeps a little run of frames ready in memory (it warms them ahead of the playhead and while
  you sit paused), and the scope traces whichever one is on screen. If a frame hasn't been
  kept in memory yet — Lumit skips saving some frames during playback to stay fast — the scope
  simply holds the last frame it had rather than going blank, and snaps back to live the
  instant the current frame is ready. The counting itself now runs on the graphics card (the
  GPU scope pass, K-096 v1 — `crates/lumit-gpu/src/scope.rs`), so tracing every frame costs
  almost nothing; the CPU counting in `shell/scopes.rs` remains as the fallback for a machine
  with no adapter. The scope's
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
- The **Project panel** — AE-shaped (K-068): a **search box** across the top, the selected
  item's details just under it, the folder tree below, and drag-and-drop everywhere. The
  search box filters the tree live by name as you type (case-insensitive; a folder stays
  visible when anything inside it matches, so you always see the path down to a hit), and
  clearing it shows everything again (UI-3). The details box now keeps a **fixed height**
  whatever you select, so the tree beneath it no longer jumps around as you click between
  items; and when the selected item is footage it shows a small **thumbnail** of the frame on
  the left — reusing the very frame the Viewer already decoded rather than decoding a fresh
  one, with a plain placeholder shown until a frame is to hand (UI-4, K-157). **Double-click
  a composition to open it** in the Timeline; double-clicking anything else renames it where
  it sits, and a comp is renamed from its right-click menu or its settings dialogue instead.
  Drag footage onto the Timeline or Viewer to make a layer; with no comp open yet, dropping
  it on the empty Timeline raises the composition dialogue already filled in from that
  footage, and the clips land in the comp it makes. Solids are proper assets now — one "White solid"
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
- **The worker pool (`lumit-eval::pool`)** — the crew of threads that will do the
  rendering, so the interface thread never has to. Picture a small workshop with two
  in-trays: an *urgent* tray (the frame under your cursor, a scrub) and an *everything
  else* tray (warming the cache, thumbnails). Whenever a worker finishes a job it always
  takes from the urgent tray first, so scrubbing never queues behind housekeeping. Both
  trays have a fixed size on purpose: if one fills up, new work is refused on the spot
  and the caller decides what to drop — work can never silently pile up behind a stall.
  The pool never kills a running job; jobs stop *themselves* by glancing at the epoch
  wall (previous bullet). The crew size is your machine's core count minus three — one
  core each left free for the interface, the GPU feeder, and the operating system.
  Tests prove the urgent-first rule, the fixed tray sizes, and that a misbehaving job
  can't take a worker down with it.
- **The pixel-pass walker and its plug sockets (`lumit-eval::exec`)** — the piece that
  walks the wiring diagram (two bullets up) and turns it into an ordered list of actual
  work. It starts at the final "comp output" box and works backwards: to blend a layer
  you first need its placed pixels, to place them you first need the source frame. Each
  box is done exactly once — two layers sharing a clip share the one fetched frame — and
  the real pixel work is done through three *sockets* it doesn't look inside: "fetch me
  this source's frame", "run this one step", and "have we rendered this exact frame
  before?" (the cache, checked before doing anything and filled afterwards). Because the
  sockets are plug-shaped, the tests plug in cardboard fakes — no GPU, no codecs — and
  prove the order, the sharing, the cache behaviour, and that a scrub landing mid-walk
  abandons it cleanly. A second proof goes further: a *walking skeleton* test in
  `lumit-gpu` plugs the **real GPU compositor** into the sockets, renders solid-colour
  layers through the walker, reads the pixels back, and checks the colours are exactly
  right — including that two layers blend in linear light and that a cache hit does zero
  GPU work. So the sockets are proven to fit the real machinery; what remains is teaching
  the adapters the full layer vocabulary (transforms, masks, retimes, effects) and then
  switching preview and export over. Until then the shipped renderer in `lumit-render` keeps
  drawing the picture.
- **Two ways to play back (`lumit-eval::schedule::cached_step`, K-171)** — the important
  distinction between the two preview modes. In **Cached** mode (the default), Lumit shows you
  *every* frame and never skips: the playhead only moves on to the next frame once that frame
  has finished rendering, and no faster than real time. So if a comp is heavy and rendering is
  slower than real time, playback simply slows down to match — you see every frame, just not at
  full speed — and once a stretch is rendered it plays back at true speed from the cache. Sound
  pauses while a frame is being waited for (so it never runs ahead of a frozen picture) and
  plays during smooth realtime replay. One subtlety a tester caught: the app only gets to move
  the playhead when the screen refreshes, and refreshes never land exactly on a frame boundary —
  if the pace timer restarted "from now" at each step, the few spare milliseconds were thrown
  away every frame, the picture crept along slower than true speed, and the sound (which runs on
  the audio hardware's own clock) drifted ahead and kept getting yanked back. The fix is the
  metronome trick (`cached_pace_carry`): the leftover is *carried into the next frame's window*,
  so over any stretch the picture holds exactly true speed and stays with the sound. A genuine
  freeze (dragging the window, say) is not "repaid" — the timer re-anchors rather than
  fast-forwarding. And the rule for *when sound runs* is readiness, not history (the owner's
  second report — audio used to sit out a quarter-second "warm-up" even on a fully cached run):
  sound plays exactly when the coming quarter-second of frames is already cached, so a ready
  run has audio from its very first frame, a still-rendering stretch stays silent rather than
  flapping on and off at the render's crawling edge, and after a stall it rejoins the moment
  the road ahead is paved. In **Realtime** mode, the opposite trade: the clock never
  waits, and when frames can't keep up Lumit drops the preview *resolution* to stay in time
  rather than slowing down. The stepping decision — advance, or hold and render, and whether
  sound should be playing — is a plain tested function; the messy wiring (the audio clock, the
  render requests) lives in the UI and just asks it what to do each screen refresh.
  The way realtime keeps from freezing is a small but important rule: it renders **one frame
  at a time and never throws that render away just because the clock moved on**. It asks for a
  frame, lets it finish however long it takes, shows it, times it — and only *then* asks for
  the next one, at wherever the clock has reached by that point (skipping the frames in
  between). The timing of each finished frame is what tells the resolution controller to drop a
  notch when things are slow. The earlier version re-asked for a new frame every screen refresh,
  so under load each render was abandoned before it finished: nothing ever completed, the
  controller was never told how slow things were, and the picture sat frozen. Rendering one
  un-abandoned frame at a time fixes both — the picture always moves forward, and the
  resolution actually adapts. (A cached frame still shows instantly and for free, without
  waiting on any render.) The "how slow was that frame" measurement is taken on the worker
  thread as the actual decode time, *not* as the time from asking to seeing — the latter
  would fold in how often the screen happens to refresh (~16 ms), making even a cheap comp
  look exactly one refresh slow and walking the resolution down for no reason. One honest
  limit worth knowing: dropping the preview resolution makes the *compositing and effects*
  cheaper, but video *decoding* costs about the same whatever size you view it at (the whole
  frame is decoded, then shrunk). So on a comp whose cost is mostly raw footage decoding,
  realtime can still look a little choppy even at a low resolution — the smooth path there is
  Cached mode, which renders ahead and then replays from memory. Truly smoothing realtime for
  decode-heavy comps needs *rendering ahead* (a shelf of frames prepared before their time
  comes), which is the `FrameRing` machinery that is built and tested but not yet wired in.
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
- **The cache has to know when a file's *identity* changed, not just the project.** Every
  cached frame is filed under a "frame key" — a short fingerprint of everything that
  decides what the picture looks like, worked out from the project. Ask for the same frame
  again, get the same key, and Lumit can hand back the picture it already has instead of
  re-rendering. That works because the project is the whole story… almost. Checking a file
  is really on disk happens on a background thread, so for a moment after opening a project
  Lumit genuinely does not know whether a clip exists, and draws that layer as nothing. The
  project hasn't changed when the answer arrives — but the picture has: the layer now shows
  colour bars. Same key, different picture, which is exactly the thing a cache must never
  allow.
  Throwing away the cached frames when the answer lands is half the fix, and the half that
  isn't enough: the pre-cacher above has *already sent off* renders of the neighbouring
  frames, and those come back a moment later, drawn without the colour bars, and get filed
  under keys that now promise colour bars. That was a real bug — the missing-footage bars
  showed on the frame you were sitting on and every other frame in the composition went
  black. So Lumit keeps a counter, the *media epoch*, which ticks whenever an answer changes
  what a file is. Every render request is stamped with the counter's value, the finished
  frame carries the stamp home, and anything stamped with an old value is thrown away rather
  than shown or filed. It is the render-queue equivalent of binning work that was started
  from an out-of-date brief.
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
- **Pre-composing (`Ctrl+Shift+C`)** — the opposite move to collapse: take layers you
  already have and wrap them in a comp of their own, which then sits in their place as
  a single Precomp layer. Useful when a group has grown into one thing you want to
  treat as one thing — blur it, mask it, move it, all at once.

  A dialogue asks first, because two of the choices genuinely change what you get. The
  first is where the **attributes** go: a layer's transform, effects and masks can
  travel into the new comp with it, or stay behind on the Precomp layer standing in its
  place. Staying behind is the one you want when you are wrapping a layer so that
  something can act on it *from the outside* — but it only makes sense for a single
  layer, since a group of layers has no one layer for the attributes to stay on, so with
  several selected the option greys out. The second is whether the new comp is as long
  as the one it came out of, or trimmed to just the stretch the selected layers cover.
  Either way nothing moves in time: trimming shifts the packed layers back by exactly as
  much as it moves the Precomp layer forward. Your answers are remembered for next time.
- **Blend modes** — the full After Effects colour set (T24): Normal; the darken group
  (Darken, Multiply, Colour burn, Linear burn, Darker colour); the lighten group (Add,
  Lighten, Screen, Colour dodge, Lighter colour); the contrast group (Overlay, Soft light,
  Hard light, Linear light, Vivid light, Pin light, Hard mix); the comparative group
  (Difference, Exclusion, Subtract, Divide); and the component group (Hue, Saturation,
  Colour, Luminosity). The dropdown groups them with dividers exactly as AE does. Two
  families under the hood: Add, Subtract and Multiply are physical light maths and run in
  linear; the rest are the Photoshop-era formulas people know by eye, so Lumit runs them on
  encoded values (running them in linear is tidier maths and the wrong look). Add pours
  light in; **Subtract** is its mirror — it takes the top layer's light away and stops at
  black, never going negative (K-151). Lighten and Darken are a simple per-channel max/min
  where the distinction doesn't matter; **Darker/Lighter colour** compare the whole pixel by
  brightness instead of each channel. The four component modes borrow one property (the hue,
  the saturation, the colour, or the brightness) from the top layer and keep the rest from
  below. Every mode is pinned to its textbook formula by a GPU test. (Dissolve and the
  stencil/silhouette alpha modes are still to come.)
- **Colour depth, in one paragraph.** Lumit's frames are "half float" (fp16) in linear
  light. Unlike AE's 16bpc — which is integer maths that clips at 1.0 — half float
  keeps brightness above 1.0 (a glow can genuinely overshoot) and negatives, which is
  what people switch AE to 32bpc for. Depth is one project-wide switch (8 / 16 float /
  32 float — K-069): flip it and every comp and effect in the project renders at that
  depth, AE-style, via a small button at the foot of the Project panel. Full float
  doubles every frame's memory and roughly halves compositing throughput, so 16-float
  stays the default; the heavy maths inside effects can run wider internally either way.
- `flutter_ui/lib/theme/theme.dart` — **the design tokens.** The only file allowed to contain
  colour values. Change a colour here, it changes everywhere. As of K-084 the look follows
  the *structure* of rerun.io's viewer (a data-tools app whose interface the owner likes):
  the app's background is nearly black, panels sit just above it, and menus float a clear
  step higher on a soft shadow; buttons have no borders — you can tell idle from hovered
  from pressed purely by how light their fill is; scrollbars are thin and solid; panel
  edges are single crisp 1px lines. The colours themselves (the clay accent, the cool grey
  family) are still Lumit's own — we borrowed the skeleton, not the skin.
  *(A note on the Settings window paragraphs below: they record the full design as the
  egui shell shipped it. The Flutter Settings window is now the same shape — a sidebar of
  pages, grouped cards, a setting's name and a line about it on the left of each row and
  its control on the right — but carries four pages rather than five: **General**,
  **Appearance**, **Interface** and **Performance**. Export and Autosave have nothing
  behind them on this frontend yet, and a page with no working controls would be a promise
  the window cannot keep; they are tracked in [TODO.md](TODO.md).)*
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
- **Editing style: the two Vegas settings, and being asked once (K-246).** The Interface page
  has a third group, **Editing**, holding two switches that decide whether Lumit behaves the
  way After Effects does or the way Vegas does. They are genuinely independent — plenty of
  people want Vegas ramps and After Effects imports, which is why they are two switches and
  not one mode — and **both are off by default**, off being the After Effects behaviour Lumit
  has always had. A new switch must never change how somebody's editor works without them
  asking, so a settings file written before these existed reads as "off, off".
  **Retime opens to Velocity** decides which way round the Retime graph opens. Off, it opens
  showing *which moment of the footage is on screen* — a line climbing steadily means normal
  playback. On, it opens showing *playback speed* as a percentage, which is the Vegas
  velocity envelope: one point per keyframe, dragged up to speed the footage up, down towards
  zero to slow it, and below zero to run it backwards. Both are views of the same underlying
  retime — switching between them converts nothing — and this setting only picks which one you
  land in. Ordinary properties like Position are untouched by it.
  **Video arrives as a Sequence layer** decides what dropping a video file into a composition
  makes. Off, you get a plain Footage layer, one layer per file, as in After Effects. On, you
  get a **Sequence layer**: a layer that holds a run of clips on its one row, so you can cut
  the footage into pieces and ramp each piece separately without stacking up layers. Still
  images are never wrapped this way — there is nothing to cut in a single frame — but image
  sequences are, because they are footage that runs.
  Because those two settings now exist, Lumit finally has something to ask on a **first
  launch**. On a machine with no settings file at all, one small screen appears before
  anything else and asks *how do you edit?*, with two answers: **After Effects**, which
  leaves both switches off, and **Vegas**, which turns both on. There is a **Skip**, which
  keeps the defaults, and clicking outside the screen does the same thing. Whatever you pick,
  the answer is written down so the question is asked exactly once — and everything it set is
  an ordinary switch on the Interface page afterwards, so no answer is one you are stuck with.
  What counts as a first launch is precisely "there is no settings file on this machine":
  a file that predates the question, or one that has been corrupted, both belong to somebody
  who has used Lumit already, and being asked to introduce yourself after months of work
  would be absurd. (The screen is deliberately plain for now. The fuller version in
  [07-UI-SPEC.md](07-UI-SPEC.md) §13.1 — four choices, each with a small picture of what it
  does — is in [TODO.md](TODO.md).)
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
- `flutter_ui/lib/icons/icons.dart` — **the icons: Iconoir** (K-085).
  Little pictures like the play triangle or the padlock come from Iconoir, a free
  professionally drawn icon family, so every glyph stays crisp at any size and always
  takes the theme colour (dimming on hover, turning accent when active) exactly like text
  does. Emoji are banned: a glyph is either from this set or deliberately drawn, never a
  character we hope the user's fonts carry — that's how the invisible stopwatch/arrow bugs
  happened. To add one, add a name to the `LumitIcon` list and its Iconoir widget in the
  lookup.
- `flutter_ui/lib/main.dart` + `lib/shell/` — **the window**: panels, menus, shortcuts,
  and the state glue (current project, selection, the render worker's reply stream).
- **Layers can hang over the edges of the composition** (K-153, GEN-3). Think of a
  composition as a fixed-length window of time — say ten seconds. A layer used to be forced to
  live entirely inside that window: you could not slide it so it *started before* the comp's
  zero mark, and importing a clip longer than the comp chopped it down to fit. Now a layer sits
  wherever you drag it. Its start may be a negative time (it begins "off to the left", before
  the comp starts) and its end may run past the comp's end. The program only ever *shows and
  plays the part that overlaps the ten-second window* — the bit hanging off either edge is
  simply never asked for — but nothing is thrown away, so sliding the layer back brings the
  hidden footage straight back. Two everyday wins: a long clip keeps its whole length on
  import (you position it, the window trims the view, not the clip), and you can push a layer
  left so an earlier moment of it lands on the very first frame. Under the bonnet this needed
  almost nothing in the engine — the picture and the sound were already built to render only
  the overlapping slice — so the change was really just *removing* the old "snap it back inside
  the comp" rules from the drag and the import. One rough edge for now: the timeline can't
  scroll to show negative time, so a layer that starts before zero is drawn tucked under the
  left edge (you can still grab the part that's on screen).
- **Finding footage that moved (`lumit-project` fingerprint + relink)** — a project doesn't
  hold the video and audio files inside it; it *points* at them on disc. Move or rename a
  file and the pointer goes stale. Lumit now records, next to each pointer, a small
  **fingerprint** of the file: its size and a quick hash of the first and last chunk (never
  the whole thing, so it stays instant even on a feature-length movie). When a project opens,
  each pointer is resolved in order — first the path relative to the project, then the last
  full path it was seen at, then, if both miss, a **search by fingerprint** through folders
  you've told Lumit to look in — so a clip that was simply moved is found again by its
  *content*, not its name. Relink one file and its neighbours that moved the same way are
  offered automatically (the "it all went into a new folder" case). Nothing is a blocking
  error: a file that can't be found shows a placeholder and waits for you to relink it.
- **Collect for sharing (`lumit-project::collect_for_sharing`)** — one command copies the
  project and every file it uses into a single folder, rewriting the pointers to sit next to
  the copies. Nothing machine-specific is written (no "C:\Users\me\…" paths), so the folder
  opens cleanly on someone else's computer — the mechanism behind sharing a project with the
  community. Two clips that happen to share a name are copied under distinct names so neither
  overwrites the other, and anything that can't be found is listed rather than silently
  dropped.
- **Opening older projects (`lumit-project` schema migrations)** — the file format will
  change over time. So a saved project carries a version number, and when a newer Lumit opens
  an older file it walks it up through a chain of small **migration** steps — each one nudging
  the raw saved data from one version to the next — before the program ever tries to
  understand it as a real project. Today the chain is empty (this is the first format), but
  the machinery is in place, so future changes have a home and old files keep opening. A
  current-version file skips all of it and loads directly, so ordinary saves are untouched.
- **The frame cupboard decides what to drop (`lumit-cache`, docs 06 §5.3)** — the store of
  rendered frames has a strict size limit (a budget in megabytes, not a count — one big frame
  costs as much as many small ones). When it's full and a new frame arrives, it throws out the
  frame that's the *best bargain to lose*: one you haven't looked at in a while, that's large
  (frees the most room), and that's cheap to recreate — the "stale × big × cheap" rule. Two
  frames it will **never** throw out are ones that have been **pinned**: the picture on screen
  and the handful of frames either side of the playhead, so playback can't accidentally bin
  the very frame it's about to show. If the whole cupboard is pinned it simply runs a touch
  over budget for a moment rather than dropping something you need — the pins clear on their
  own as the playhead moves on.
- **Undo doesn't remember forever (`lumit-core::store`)** — every edit is remembered so you
  can undo it, but that memory can't be allowed to grow without end over a long session. So
  the undo history keeps at most a few hundred steps; once it's full, the *oldest* step falls
  off the back. You can't undo past that point any more, but nothing about your current
  project changes — dropping old history only limits how far back you can rewind. (Crash
  recovery is separate and unaffected: every edit is also written to a journal on disc as it
  happens, independently of this in-memory limit.)
- **Playback keeps time on a grid, not a stopwatch (`lumit-bridge::playback`)** — in
  every-frame mode each picture used to be allowed out "one frame period after the last one
  actually went out". That sounds right and is quietly wrong: every present is a little late
  (the thread wakes a moment after its alarm, the loop has bookkeeping), and measuring from
  the *actual* last present meant each frame's lateness was added to the next frame's
  schedule, forever. A 60 fps comp could never truly play at 60 — it settled around 55,
  cached or not, and the faster the comp the worse the shortfall. Now the due times sit on a
  fixed grid, one period apart: a present that goes out a millisecond late leaves the next
  one due at the *grid* time, so the small latenesses are absorbed instead of accumulating.
  Only a genuine stall (more than a whole period late) moves the grid, and then playback
  carries on at rate from where it is — every-frame still never skips a picture and never
  fast-forwards to catch up. The last two milliseconds before each due time are also waited
  out precisely (a busy wait) rather than slept, because an operating-system sleep is only
  as accurate as its timer, and oversleeping by one timer tick is a whole frame at 100 fps.
  On Windows that timer matters even before the busy wait: its default tick is about
  16 milliseconds — *twice* a 120 fps frame — so every paced sleep overshot its due time and
  a 120 fps comp could only manage ~85. The playback thread now asks Windows for
  1-millisecond timing when it starts (`timeBeginPeriod`, the request every media
  application makes), which is also what stops presents jittering by several milliseconds
  at 60 fps. That jitter had a second victim: the sound. The audio minder stops the sound
  when a picture arrives "late", and its allowance was a quarter of the frame period —
  2 ms at 120 fps, inside ordinary scheduler noise, so the sound kept stopping over
  pictures that looked perfectly smooth. The allowance now never shrinks below a few
  milliseconds, because the ear judges slip in milliseconds, not in frames.
- **Frames get their names from a memo (`lumit-bridge::names`)** — every cached frame is
  filed under a fingerprint of everything that goes into it, and computing that fingerprint
  means walking the whole composition at that frame's time. Cheap once; not cheap when the
  cache bar names hundreds of frames per redraw and playback names every frame it looks
  ahead to. The memo remembers each answer, keyed by composition, frame and preview size,
  and forgets everything the instant an edit lands (an edit renames an unknown set of
  frames, so remembering across one would be guessing). The result: during playback of an
  unchanged project, naming a frame costs a lookup instead of a walk — which is most of what
  made the timeline's cache stripe expensive to keep fresh while playing.
- **The disk cache is asked early, and given a moment (`lumit-bridge` worker)** — a frame
  parked on disk takes a few loop turns to come back (read the file, decompress it), so a
  frame asked for at the instant it's needed *always* arrives too late, and used to be
  re-rendered from scratch even though a perfect copy sat in a file. Two fixes. First,
  pressing play now asks the disk for the first stretch of frames *before* the first render
  starts, so the reads run while playback is still warming up. Second, in every-frame mode,
  if the next frame's copy has been asked for and is on its way, the renderer waits up to a
  twentieth of a second for it rather than re-rendering — every-frame promises every frame,
  not any particular arrival time, and the copy is far cheaper than the render. (Adaptive
  mode never waits; it keeps chasing the clock.) And a frame that comes back off disk now
  keeps a copy in the memory tier too, so the *next* pass over the same span climbs from
  memory instead of reading the same files again — before this, a comp bigger than the
  graphics card's cache re-read its files on every single pass, and the disk's speed became
  the playback speed.
- **The cache stripe redraws without stealing playback's deadline (`lumit-bridge` worker)** —
  the coloured stripe over the timeline is computed on the same thread that renders playback
  frames. It used to restart its full sweep every half-second during playback (because every
  promoted frame changes what the caches hold, and any change restarted it), naming up to a
  thousand frames in one go — a visible hitch, rhythmically, all through playback of cached
  material. Now a sweep in progress finishes before any restart, the per-turn chunk is
  smaller while playback runs, and most names come from the memo above anyway. While
  playing, a frame already known to be parked on disk at the right size also skips the
  three extra "is a coarser version held?" checks — the stripe may briefly show blue where
  dimmed green was strictly truer, and it firms up the moment playback stops. The stripe
  also greens **live** now: the sweep walks forward from the playhead, so the frames
  playback had just banked — always just behind it — were the last thing it reached, and
  the stripe sat frozen until you paused. Banking a frame now paints its own slot in the
  strip directly (the bank knows exactly which frame it filed), and each publish of the
  strip nudges the interface to redraw — the old wiring only nudged it when the *idle*
  cache fill banked something, which is precisely the thing that never happens during
  playback.
- **The frame-rate readout in the Debug View (`flutter_ui/lib/panels/performance_view.dart`)** —
  a small counter showing how fast the interface itself is drawing, which is how you tell "the
  engine is presenting at rate" from "the window is keeping up with it". It **watches** frames
  rather than asking for them: the engine reports what each finished frame cost, after the
  fact, and the counter reads those reports. The first version instead asked to be woken after
  every frame and redrew itself each time, which quietly pinned the whole interface at full
  drawing rate whatever the editor was doing — the meter became a large part of what it was
  measuring. It also hung every automated test that waits for the interface to go still,
  because "still" was the one state it made impossible. Watching costs nothing when nothing is
  moving, and the numbers on screen refresh five times a second rather than per frame, because
  a readout redrawn per frame is one more thing drawing per frame.
- **The stress project and speed benchmarks (`lumit-project::fixtures`, docs 13)** — the
  promise that Lumit stays responsive on huge projects needs something huge to test against.
  There's now a builder that makes a deliberately enormous project on demand — hundreds of
  compositions, thousands of layers, a quarter of a million keyframes — always *identical*
  down to the last byte, so a speed measurement means the same thing every time. Alongside it,
  a set of **benchmarks** time the everyday operations on that project (open it, save it, make
  one edit, undo). They run when a developer asks (`cargo bench`), and they'll later become
  pass/fail speed budgets in the automated checks.

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

**Shaders get checked without a graphics card (K-263).** The little programs that run on
the card — the `.wgsl` files — used to be checked by exactly one thing: the card itself,
at the moment Lumit built the pipeline. That is a bad place to find a typo. It means a
broken shader builds fine, ships fine, and turns up as a black picture on somebody's
machine, while any test runner without a graphics card sails past it. So there is now a
test that runs the *same* shader compiler wgpu uses (naga) over every kernel in the crate
and fails on anything it would reject. It needs no card, so it runs everywhere and
finishes in milliseconds. It checks that a shader is *valid*, not that it is *right* —
being right is what the CPU-reference comparisons are for, and those do need a card.

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
4. Now the normal commands work: `cargo test --workspace` runs the engine's test suite,
   and `flutter run` from `flutter_ui/` launches the app.

### On macOS

FFmpeg comes from Homebrew: `brew install ffmpeg@7`. That formula is *keg-only* — Homebrew
deliberately does not put it where the system looks by default, because it is an older
version than the plain `ffmpeg` formula and linking it would shadow that. So the build has
to be told where it went, once, in the terminal you build from:

```sh
export FFMPEG_PKG_CONFIG_PATH="$(brew --prefix ffmpeg@7)/lib/pkgconfig"
```

Put that in your shell profile and every future terminal has it. macOS ships the translator
(libclang) as part of its developer tools, so there is nothing else to set up — then
`cargo test --workspace` works.

This line used to live in the repo's `.cargo/config.toml` instead, so nobody had to type
it. It was removed (K-204) because Cargo offers no way to make such a setting apply to one
platform only: the macOS folder was being handed to Linux builds as well, where it does not
exist, and the FFmpeg binding generator stops with an error rather than falling back. One
platform's convenience was the other's broken build, and macOS is the one with the unusual
requirement, so macOS is the one that says it out loud.

### On Linux (K-082)

Linux finds FFmpeg the same way macOS does — by asking the system's package registry
(`pkg-config`) where the libraries live — so the setup is: install the FFmpeg 7
*development* packages (the ones ending `-dev`, which carry the headers the binding
generator reads), plus `pkg-config` and `clang`. On Debian 13 or Ubuntu 24.10 and newer
that is one line: `sudo apt install pkg-config clang libavcodec-dev libavformat-dev
libavutil-dev libswscale-dev libswresample-dev libavfilter-dev libavdevice-dev`. On Arch
or Artix: `sudo pacman -S ffmpeg pkgconf clang18 llvm18` — note the **18**: those
distributions' plain `clang` package is a much newer LLVM, and as explained above a newer
LLVM makes the translator produce nonsense, so the versioned packages are the ones to
install.

Nothing about FFmpeg then has to be handed to the build: the packages put their `.pc`
description files where `pkg-config` already looks, which is where the build asks. One
setting may still be needed, and only on the distributions whose default translator is
newer than 18:

```sh
export LIBCLANG_PATH=/usr/lib/llvm18/lib          # Debian/Ubuntu: /usr/lib/llvm-18/lib
```

That says "use the *18* translator, not whichever one is the default" — on Arch and Artix
the default always is newer. Put it in your shell profile and every future terminal has it.
Then `cargo test --workspace`, and `flutter run` from `flutter_ui/` to launch the app.

Linux used to need a second export here, to undo a macOS folder the repo's
`.cargo/config.toml` handed to every platform. That setting is gone (K-204) and Linux is
plain again; if you are looking at an older checkout, deleting the `[env]` block from
`.cargo/config.toml` is what the fix amounted to.

One honest caveat: the build needs FFmpeg **7**, and some distributions still ship
FFmpeg 6 — Ubuntu 24.04 LTS is the big one. On those, `cargo build` will complain about
"ffmpeg stuff" (a version the binding doesn't accept, or missing headers). The fix is a
newer distribution release, or building FFmpeg 7.1 from source and letting `pkg-config`
find it.

(A **Flatpak** — a ready-to-install Linux bundle — is how releases ship for Linux, and
since K-290 it is the *only* way they do. An earlier one packaged the old egui application
and was retired with it in K-182; the current recipe, described under "Installers" below,
is a different animal that compiles nothing.)

One Linux-only difference worth knowing, because it looks like a bug otherwise: on Windows
and macOS Lumit *starts as* the little splash card — that small frameless window you see
during boot is the real window, and it grows into the editor when loading finishes. On Linux
it can't. Under Wayland an application isn't allowed to resize its own window (the desktop
decides), so the "now grow to full size" instruction was simply ignored and the editor stayed
trapped at splash size, unable to be dragged bigger. So on Linux the window opens at working
size straight away and the splash card is drawn in the middle of it.

### What the robots check

Every push, CI rebuilds and retests everything on **macOS, Windows and Linux**, media
included, so "it builds on my machine" can never quietly drift from "it builds for real".
The platform recipes above are exactly what CI does, written out by hand in
`.github/workflows/ci.yml`. The Linux job goes a little further than the others: it installs
Mesa's *lavapipe*, a Vulkan driver that renders on the CPU, so the GPU tests actually run on
a machine with no graphics card in it.

One gap worth naming, because it is the kind of thing that surprises you at the worst
moment: CI checks the *code* on every push, but it does not check the *packaging*. Nothing
in `ci.yml` builds an installer, a disk image or a Flatpak — those live only in
`release.yml`, which runs when you push a tag. So the first time a packaging change is
proved is the first tag after it. That is what pre-release tags are for (below).

A rule the FFmpeg episode above taught, worth stating on its own: **a CI job that sets up
something a contributor would not have set up is not testing the contributor's build.** The
Linux jobs used to hand the build an explicit FFmpeg location before compiling, which meant
they never took the route a person with FFmpeg installed normally takes — and so they
stayed green for weeks while nobody could actually clone the repository on Linux and build
it. The jobs now leave that route alone and let the build find FFmpeg the ordinary way. If
a step exists purely to make CI work, ask who else has to run it.

Two of those checks were quietly weaker than they looked, and K-269 fixed both.

**A skipped test looks exactly like a passing one.** Every test that needs a graphics card
is written to *skip itself* when there isn't one — that is what lets the suite run on a
laptop with nothing installed. But it also means a machine whose graphics driver went
missing runs none of them and still reports a green tick: about ninety checks of the actual
shader maths, silently not run. So there is now an environment variable,
`LUMIT_REQUIRE_GPU`, that turns "no adapter" from a polite skip into a failure, and the
Linux job — the one that deliberately installs lavapipe — sets it. Your own machine leaves
it unset and keeps the friendly skip. The macOS and Windows jobs deliberately do not set it
yet: nobody has confirmed those runners offer an adapter at all, and a gate is only worth
having where it has been checked.

**A build with no FFmpeg is a real build again (K-273).** Lumit can be compiled without the
video decoder — useful for anyone who wants to work on the editing model without installing
FFmpeg first, and it is why the Windows job used to be quick. Nobody had built it that way
for a while, and it had stopped compiling. The rule it broke is worth knowing, because it is
easy to break again: **the list of functions Dart can call is the same in every build.** The
Dart side of the bridge is generated from that list, so a function that *vanishes* when a
feature is off leaves generated code calling something that is not there. What may change is
what a function *does*: beat detection is always present and simply answers "this build has
no audio pipeline". A media-less build loses decoding — no probing, no thumbnails, no
waveform peaks, and the decode-ahead thread quietly does nothing — never a call that is
missing. CI now builds and tests it on every push, so it cannot rot again.

One honest correction, because the first version of this said otherwise: turning the
feature off does **not** give you a build with no FFmpeg in it. Two other parts of the
engine — the renderer and the audio mixer — depend on the decoder unconditionally, and the
bridge depends on both, so FFmpeg is still linked either way. What the feature governs is
the bridge's *own* decode paths. Making the whole dependency tree media-optional is a
separate job, and it is written down as one rather than implied by a feature flag.

**Two more robots joined in K-272, both about things nobody here writes.** The first is a
*pinned compiler*: "stable Rust" means whatever version your machine last downloaded, and
because Lumit treats every compiler warning as an error, a new Rust released on a Tuesday
could turn a build red on a commit that changed nothing. A small file at the top of the
repository (`rust-toolchain.toml`) names the one version everything is built with, and Rust
fetches exactly that on every machine including CI. Raising it is then a deliberate act
rather than a surprise. The second is a *dependency check* (`cargo deny`): Lumit is GPLv3,
which means it may only carry libraries whose licences the GPL can absorb, and it should
not quietly pick up one with a published security hole or one whose author has stopped
maintaining it. Four hundred-odd libraries arrive indirectly, so `deny.toml` writes the
rules down and CI checks them — including three abandoned libraries we knowingly live with
for now, each recorded with what it would take to leave it, because pretending they are not
there would be worse than saying so.

**The no-hex rule was being enforced on the wrong language.** Every colour is supposed to
come from the theme, so the schemes and any custom theme actually reach every pixel — and
CI was grepping for stray colour values in the *Rust* code, which is where the old frontend
lived. All the widgets are Dart now. The same grep now runs over `flutter_ui/lib` outside
`theme/`, and it found three real ones: a modal window's dimming wash spelled out in hex,
and Material's own red and amber standing in for the theme's error and warning colours. The
wash became a proper token (`scrim`), so it follows the scheme like everything else. Two
things deliberately still pass: fully transparent (`0x00000000`), which is the *absence* of
a colour rather than a choice of one, and rebuilding a colour from numbers that came out of
a saved file, which is data rather than a design decision.

### How Lumit knows how much memory your machine has (K-194, K-204)

Settings → Performance lets you type a cache size in megabytes, which means the engine
needs a real ceiling to check it against — offering to cache 64 GB on a 16 GB machine is
just a way to make everything swap to disk and crawl. So the bridge has two small
questions it can ask the operating system: how much RAM is installed, and how much memory
the graphics card has.

There is no one way to ask, because each operating system answers differently. Windows has
a single call for it. macOS has a general-purpose "ask the kernel a named question"
mechanism, and the question is called `hw.memsize`. Linux does not have a call at all: it
exposes a plain text file, `/proc/meminfo`, whose first line reads something like
`MemTotal: 16264532 kB`, and you read the number out of it. Three implementations, one
answer, and — this is the important habit — **every one of them returns 0 rather than
guessing** if the answer does not come back. The interface treats 0 as "not known here" and
falls back to a documented ceiling of its own, which is honest, where a made-up number
would quietly be wrong.

One oddity you will notice: on a 16 GB Linux machine the number comes back as roughly
15.5 GB, not 16. That is not a bug and it is not worth correcting. Linux reports the memory
*the kernel can actually use*, and some was already taken before the kernel started — by
the firmware, and by an integrated graphics chip carving out its share. The 16 GB is what
you bought; the 15.5 GB is what is there to spend. For deciding how big a cache may be, the
smaller of the two is the one you want, so reporting slightly low errs in the safe
direction. The video-memory answer leans the same way for the same reason: on a machine
with both an integrated and a discrete graphics chip it reports whichever the system lists
first, which may be the smaller one — again, a ceiling that is too low costs you some
speed, while one that is too high costs you the session.

## 9. The Flutter frontend, in plain terms

Flutter is Google's toolkit for building interfaces; Dart is the language it uses,
about as readable as TypeScript. Flutter keeps a *widget tree* — a description of
the interface — and redraws only the parts whose description changed. It became
Lumit's frontend in K-174, replacing egui.

**The frontend is a view, not a second brain.** Everything that opens files,
decodes video, composites, caches, mixes audio and exports stays in the Rust
crates. Dart displays values and forwards calls; when something has to be
*decided*, the engine decides it. Anything that looks like editing logic in
`flutter_ui/` is a defect waiting to disagree with the engine.

### Where things live

| Path | What it is |
|---|---|
| `flutter_ui/` | The Dart app. Builds and runs without touching the Rust build |
| `flutter_ui/lib/panels/` | One file per panel (Project, Viewer, Timeline, Effect controls, Scopes…) |
| `flutter_ui/lib/shell/` | Menu bar, dock, popped-out windows, keyboard entry |
| `flutter_ui/lib/state/` | The Dart-side caches: `comp_model.dart` (the read model), `comp_time.dart`, `timecode.dart`, `preview_throttle.dart` |
| `flutter_ui/lib/widgets/` | Shared controls, including `showLumitModal` |
| `flutter_ui/lib/theme/` | Every colour in the interface. A hex literal anywhere else is a defect, and CI says so |
| `flutter_ui/lib/src/rust/` | Generated Dart — never hand-edited |
| `crates/lumit-bridge/` | The Rust half of the seam; `src/api/` is the hand-written part |
| `flutter_ui/rust_builder/` | cargokit: compiles the Rust library as part of `flutter run` |
| `docs/17-BRIDGE-CONTRACT.md` | The normative front/back boundary. Read it before touching the seam |
| `docs/archive/flutter-port/` | Frozen notes from the port itself |

### The bridge

`crates/lumit-bridge` builds to one shared library the app loads at startup. It
depends on the engine crates and nothing depends on it, so it stays a leaf.

**The glue is generated, not written.** `flutter_rust_bridge` ("frb") reads
`crates/lumit-bridge/src/api/` and writes both halves: `src/frb_generated.rs` on
the Rust side and `flutter_ui/lib/src/rust/` on the Dart side. Those files are
checked in but are *output* — change `api/`, then run
`flutter_rust_bridge_codegen generate` from `flutter_ui/`. A hand edit is lost at
the next run, and CI's `codegen-fresh` job fails if the committed files differ
from what the generator produces.

**Dart holds handles, not copies.** A `LayerReference` is an opaque token
standing for one layer in the engine; the handle *is* the identity, so renaming
is `layer.rename(name: 'hero shot')` with no document to re-read and no Dart-side
mirror to keep in step. Rust pushes a "something changed, and here is what"
message down a stream, worked out by `op_scope` in
`crates/lumit-bridge/src/api/state.rs`, so only the affected part of the
interface redraws.

Two promises hold at the seam: a panic inside Rust is caught and returned as an
ordinary error rather than crashing Dart (CI's `no-panics-in-frb-api` job greps
for the shortcuts that would break this), and a call that takes a handle *by
value* empties the Dart side — never keep one you have handed over.

### How the picture reaches the screen

Video frames are far too large to pass through function calls sixty times a
second, so the engine draws each frame into a piece of GPU memory and hands
Flutter only its *name*. Flutter shows it with a `Texture` widget; no pixels are
copied (K-177, K-183, K-195). Each platform has its own name for the same idea:

- **Windows** — a shared texture named by an NT handle, in BGRA order (what
  ANGLE accepts). Feature `shared-texture`.
- **Linux** — a Vulkan exportable image named by a **DMA-BUF** file descriptor,
  plus stride and format (`DRM_FORMAT_ABGR8888`, linear), imported as an
  EGLImage. Feature `shared-texture-linux`. Each side owns its own descriptor:
  Rust closes what it exported, the runner `dup()`s and closes only its copy, and
  a failed `dup()` refuses the registration rather than showing black.
- **macOS** — an **IOSurface** named by a plain integer, wrapped in a
  `CVPixelBuffer`. The payload is the same shape as Windows', so it reuses the
  Windows message and Dart code. Default-on.

Fallbacks are deliberate and visible. Every frame request says whether the
frontend can display a texture; if it cannot, or the runner reports that
announced frames are never actually fetched, the engine copies pixels instead and
Settings says which transport is live. The Scopes always need real numbers, so a
slow read-back runs a few times a second alongside the fast path.

CI only proves the Linux and macOS halves *compile* — the authoring machine is
Windows. Verifying them is a collaborator's job: build (`cargo build -p
lumit_bridge --features shared-texture-linux --release`, or plain
`cargo build -p lumit_bridge` on macOS), run `flutter run -d linux|macos`, open a
comp, and check the picture matches the CPU path and that Settings reads **GPU**.
On Linux, toggle the kill-switch to prove the fallback is reachable without a
rebuild. If the Viewer is blank, capture console lines mentioning
`eglCreateImageKHR` / `vkGetMemoryFdKHR` (Linux) or `IOSurfaceCreate` /
`newTextureWithDescriptor` (macOS), and report the GPU and driver version — that
distinguishes a format mismatch from a missing extension.

### The three-tier cache (K-214, K-215)

A frame's name is a 128-bit hash of *everything that went into the picture*:
evaluated transforms, effects and values, masks, paint, blend mode, switches,
footage file and frame, inherited parent transforms, preview resolution. Where a
frame sits in time or space is not part of the name. Two consequences fall out
and are the whole point: an edit that cannot change a pixel keeps every cached
frame, and undo lands warm — there is no invalidation code to get wrong.

Frames live in three places, cheapest first: a texture on the graphics card,
bytes in memory, a file on disk. Only the disk tier survives the session.
Eviction reads a frame down a tier; a hit promotes it back up. The normative spec
is [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §5; sizes, locations and a
per-tier meter with click-to-empty are in Settings → Performance.

When you stop interacting, an idle pass fills the tiers around the playhead
rather than sitting still — rendering neighbours it does not have, and copying
held frames down to disk. It never re-renders a frame it already holds somewhere.

**Writing to disk happens behind your back, and that had a sting in it (K-277).**
A frame is handed to the disk thread and forgotten, and that thread has to
convert, compress and write it — slower than the graphics card can hand frames
over. A frame only counts as *on disk* once its write has finished, so the idle
pass, which asks that question every few milliseconds, saw every frame still in
the queue as one it had never copied, and handed it over again. And again. Each
copy is a whole frame of memory (8 MB at 1080p) sitting in a queue nobody
counted, and on a Mac left running the application reached 81 GB. Two rules fix
it and are now the only way a frame reaches the disk thread: a frame already on
its way is not sent again, and at most eight may be waiting at once. Past that
the copy is simply skipped — the frame is still on the card and in memory, and it
will be offered again later.

**The cache bar** under the time ruler shows what is held: mint at the current
preview resolution, dimmed mint only at a coarser one, steel-blue on disk,
nothing for absent. The render worker computes the strip and publishes it; the
interface draws the last thing published, rather than asking per frame.

### Playback

The sound card owns the clock — playback time is samples consumed, and the
picture asks the engine what time it is on each refresh. The audio mix plan is
built in the background and hot-swapped when the comp's fingerprint changes,
without touching the clock. Playback starts after a short pre-roll (three banked
frames or 150 ms, whichever comes first) so it does not begin by stuttering.

A background isolate does the rendering, one frame at a time, always coalescing
to the newest request; a separate thread decodes ahead of it, and a ring of
pre-rendered frames is presented at each frame's due moment. Every-frame mode
shows every frame in order and stops the sound rather than drift; adaptive mode
shows the newest frame the clock has reached, stops the sound after one late
picture and restarts it after eight on-time ones. Stopping returns the playhead
to where playback began (K-254); a scrub is the exception.

### Telling how long a frame is taking, and where the time went (K-276)

Two readouts, one mechanism. Both come from a small recorder the engine builds
for a frame — `crates/lumit-render/src/profile.rs`. The progress bar reports only
for frames somebody is waiting on; the render times are measured unless the clock
in the bottom strip is switched off.

**The preview progress bar.** Most frames arrive too quickly to mention. Some do
not: a heavy composition under a dragged value, or a scrub onto a frame nothing
has made before. The engine now reports how far such a frame has got — planning,
reading media (per file), reading the composition, compositing (per layer),
showing — and the Viewer draws a slim bar across the bottom of the picture with
the stage in words. It never appears during playback (a frame due in sixteen
milliseconds has no use for one) and never for a frame that arrives within about
150 ms, so ordinary work stays silent. The percentage is an estimate from fixed
stage weights — "roughly how much longer" — not a measurement.

**The render-time column.** The Timeline's last column shows what each layer's
own picture cost in the frame at the playhead, and twirling a layer open puts the
same kind of number on each effect's heading; the Effect controls panel shows it
on the effect's title row too. So "why is this comp slow" is answered with names
and numbers rather than guesses.

**Where the switch is, and what the column is telling you.** Measuring is on by
default, and the clock in the **bottom strip** — just after the cache meters —
turns it off for the session. (It began life as a glyph in the column's own
header, which is where nobody found it: the column looked broken instead.) While
measuring, the header stops saying "Time" and shows what the *whole frame* cost:
an ellipsis until a measured frame has come back, then the number. So the three
things that can be true read differently — not measuring, measuring and waiting,
measured — instead of one dash meaning all three. Switch it off and the column
goes altogether — header, cells and width — as do the figures on the effect
headings in the Effect controls panel; a dash while it *is* measuring means that
row was not in the last measured frame. If the engine refuses the switch, the status line says so, and
the console carries one line per switching on and one more on the first frame
actually measured.

**A measured frame is a re-made frame.** Lumit keeps finished frames — on the
graphics card, in memory, on disk — so returning to one costs a copy rather
than a render. A copy has nothing to say about what the layers cost, so while
the stopwatch is on the engine steps over all of that and composites the frame
properly. That is why switching it on also re-asks for the frame you are looking
at: otherwise the column would sit empty until you happened to scrub somewhere
nothing had been made yet.

**Why it has a switch.** Work for the graphics card is *handed over*, not
performed: the call that blurs a layer returns long before the card has blurred
anything. Timing that call would therefore time the paperwork. So a measured
frame *waits* for the card at each layer and each effect before reading the
clock, which makes the millisecond true and costs the overlap between the
processor and the card for that frame. That is a fine price to pay while you are
reading the numbers and a silly one to pay when you are not — hence the stopwatch
in the column's header, off by default, never applied to playback, and the
numbers dropped when it goes off so nothing stale is left on screen. Doing this
continuously and for free needs *GPU timestamp queries*, which is written down as
the follow-up in TODO.

### Smoothing the edges of a rotated layer (K-274)

Rotate a layer a few degrees and its edge crosses each pixel diagonally. A pixel
is a small square that is either painted or not, so the edge comes out as a
staircase — and on a slow rotation the steps crawl along it, which is the thing
the eye actually catches.

The cure is to stop asking one question per pixel. **Multisampling** keeps four
(or two, or eight) coverage points inside each pixel, works the colour out
*once*, and mixes it into that pixel in proportion to how many of those points
the layer covered. A pixel the edge cuts in half comes back half-covered instead
of guessing. It costs some memory on the graphics card and one extra step per
frame; it does not cost four times the work, which is why it is the standard
answer for edges rather than a luxury.

**It is a setting on the project, not on your copy of Lumit** (File ▸ Project
settings…). That is deliberate, and it is why that window exists at all:
everything in **Settings** belongs to this machine — your theme, your cache
sizes, your shortcuts — and nothing there travels or undoes. A project setting
is the opposite on both counts, so it has a window of its own. It changes what a composition looks like, so it
has to be saved inside the `.lum` and be the same when somebody else opens the
file — and the *same* value is used for the preview and for the export, because
the whole render path is built on the promise that what you are watching is what
you will get. Eight samples is the default: on.

Two things it deliberately does not do. It does not soften the *inside* of a
layer's picture — that is the scaling filter's job, and a shape's own curves, a
mask's edge and the outline of a letter are already smooth where they are drawn.
And it does not change with the preview resolution: a half-size preview is a
smaller picture with the same treatment of its edges.

**If your graphics card cannot manage the number asked for**, Lumit uses the
highest it will and says which — in that window, plainly, beside the one you
chose. Your project keeps the value you picked; nothing is rewritten behind your
back, and nothing fails.

One consequence worth knowing about: because the setting changes every pixel, it
is part of how a finished frame is *named* in the cache (see "the three-tier
cache" above). Changing it means the frames already made no longer answer, and
the ones you look at next are made afresh. That is the same rule every other
picture-changing edit follows; it is not the setting misbehaving.

### Asking the graphics card once instead of thirty-two times

Work does not go to the graphics card one instruction at a time. It is written
into a **command buffer** — a list of things to do — and the whole list is then
*submitted*. Handing a list over is a round trip through the graphics driver,
and that round trip costs roughly the same whether the list has one item on it
or a thousand.

Lumit used to build a separate list for every step of a frame: one for each
layer, one for each effect on it, one for the final combine. A composition with
thirty-two layers handed over thirty-four lists to draw a single frame. All of
that work was going to the same card in the same order anyway, so there was
never a reason for it to travel separately.

Now a frame writes one list and hands it over once. Measured on the same
composition: thirty-four submissions became three, and — the part that matters —
the number no longer grows when you add layers. Adding thirty-one more layers to
a comp now adds *no* extra round trips.

**Two places still have to hand work over early**, and both for the same reason:
they need to *look* at what the card produced. You cannot read a picture the
card has not drawn yet, and a list that has not been submitted has not been
drawn. So reading a finished frame back, measuring a scope, and handing the
picture to the Viewer each push the list through first.

**The render-time column is the interesting exception.** It measures a layer by
waiting for the card to finish that layer before reading the clock — but under
one-list-per-frame there is nothing to wait for yet, so the wait would return
instantly and every number would be wrong. So a *measured* frame deliberately
goes back to handing work over layer by layer. That is not a bug: it is the same
trade the stopwatch already makes, which is why measuring is a switch you turn
on rather than something running all the time.

The thing that keeps this honest is a **count**, not a stopwatch. Lumit counts
every submission, and a test asserts that adding layers adds none. A timing test
would prove nothing on the machines that check the code (they have no real
graphics card), but a count is a count anywhere.

### The panels

`state/comp_model.dart` is the read model: **one** bridge call returns the whole
fronted composition — names, switches, bar positions, transforms, effect values —
and panels draw from that copy. It re-reads when the engine's committed-edit
counter moves, and every panel that commits refreshes it directly. `comp_time.dart`
memoises frame↔time conversions the same way, and `timecode.dart` is the single
`HH:MM:SS:FF` formatter everything displays through.

- **Project** — items in folders, multi-select, drag onto the Timeline or onto
  New composition (which prefills size, rate and duration from what you dropped).
- **Viewer** — the picture, the transport, the resolution picker, the on-picture
  tool overlay. It watches the playhead and re-renders whenever it moves,
  whoever moved it.
- **Timeline** — two linked columns (outline left, lanes right, shared vertical
  scroll, independent horizontal — and the outline reserves the height of the
  lane side's bottom bar so neither half can scroll further than the other),
  the ruler with markers and the work-area band,
  layer bars with snapping, and per-layer fold-outs: Contents, Masks, Paint,
  Effects, Transform, Audio. The graph editor lives here as a lens over the
  lanes, with value, speed and Time views.
- **Effect controls** — Source / Transform / one heading per effect, two-column
  rows, Reset at the top of the value column, reorder and remove pinned right.
- **Scopes** — waveform, RGB waveform, vectorscope, histogram, computed on the
  graphics card (`crates/lumit-gpu/src/scope.rs`) and drawn on fixed near-black
  whatever the theme.
- **Bottom strip** — unsaved indicator, cache meter, one line of notices.

Panels can be popped out into their own desktop window (`desktop_multi_window`);
each gets its own Flutter engine but opens a handle to the *same* engine state,
so edits share one undo history.

**What "the selection" means when Copy is pressed (K-300).** Three different things
can be selected at once: some keyframes, an effect, and the layer they all sit on.
Copy has to pick one, and it picks the *finest* — keyframes if any are selected,
otherwise the effects picked out of the stack, otherwise the whole layer. Delete has
worked this way since K-234, and it works through the same trick: Flutter runs every
keyboard handler on every key, so a panel cannot claim a key simply by handling it
first. Instead the Timeline leaves a small function with the shell — a *claim* — and
the shell calls it before doing anything itself. If the claim says "I took that", the
shell stands down.

An **effect is selected by clicking its name**, in the Effect controls panel or on its
row in the Timeline's fold-out; `Ctrl` adds one, `Shift` takes the run between. There
is only one such selection, held by the shell rather than by either panel, which is why
an effect picked in one place lights up in the other. In the Effect controls panel picking
an effect leaves it open — the twirl mark is the only thing that folds a card there. In the
Timeline a plain click also twirls, the way it always has, and a modified click only
selects, so `Shift`-clicking down a stack does not flap all of them open. Copying several
effects produces a single `.lumfx` document — the same kind of document a preset is —
holding them in stack order rather than click order, so pasting puts them back the way
they were drawn.

**A row with no keyframes copies too (K-301).** Copy at the property level used to mean
"the selected keyframes", so a row that was never animated had nothing to give and the
chord quietly copied the whole layer instead. Now selecting rows and pressing Copy takes
those rows whole: every key of an animated one, the plain number of one that has none. A
copied number pastes as a number onto a row that is not animated, and as a key at the
playhead onto one that is. The other levels always carried their values — a copied layer
or a copied effect is the document itself, animated parts and plain numbers alike.

**Where a copy actually goes (K-302).** Lumit keeps its own tray, because what is being
copied is a piece of a Lumit document and the system clipboard is shared with every other
program on the machine. But a copy that leaves *nothing* on the system clipboard is
indistinguishable from a copy that failed — paste into a text editor and you get an empty
line — so every copy is mirrored there as its own text, and a paste that finds the tray
empty reads the system clipboard and takes a Lumit document back off it. That is also what
lets two Lumit windows copy between each other. Ordinary text is left alone: only the two
document shapes the engine's paste calls accept are recognised.

**And a lesson worth more than the feature.** The reason `Ctrl+C` did nothing in the real
app while every test passed: a saved keymap was restored by *replacing* the whole keymap,
and a saved file only knows the actions that existed when it was written — so every
shortcut added in a later version was silently missing for anyone who had ever changed a
key. Restored state is now **laid over** the current defaults rather than swapped for them,
which is the shape any "remember what the user had" code should have: the user's choices
win, and everything they never had an opinion about comes from the running build. Telling
"they turned this off" apart from "this did not exist yet" is the part that needs storing
on purpose.

**Scrolling it, and why a trackpad needed its own answer (K-278).** Dragging in
the lanes draws a selection box round keyframes, so the panel switches off
drag-to-scroll — which on a Mac also switched off the trackpad, because a
two-finger scroll there is a *drag gesture* and not the wheel's signal. The panel
now admits exactly the trackpad as a scrolling device and every drag handler over
those surfaces refuses it, so two fingers scroll and a click-drag still draws the
box. Clicking and dragging with a mouse is unaffected either way.

### Tools and the on-picture overlay

One tool is armed at a time, application-wide, and a tool is the answer to one
question: what does dragging do (K-216, K-228)? The toolbar holds thirteen
buttons covering about thirty tools, grouped behind flyouts, showing the
last-used one of each group. It stores action names only — the chords come from
the engine's keymap, in a context named `Tools`. Unbuilt tools are drawn greyed
and unpickable rather than hidden.

The overlay draws each layer as a wireframe: its own rectangle pushed through its
transform (K-217). Hit-testing runs the pointer *backwards* through that
transform and compares against the layer's bounds, so there is no polygon
geometry anywhere. The same inverse serves mask points. Zoom (K-218) routes wheel,
click and box drag through two shared functions, anchored so the comp point under
the cursor stays under the cursor; magnification is never resolution. Where the
system has no suitable cursor the tool hides it and paints its own (K-219, K-226).

Built on top of that: masks and shape layers sharing one path type across the
bridge (K-222, K-237), paint strokes stored as the *drag* rather than the pixels
and re-stamped at render resolution (K-227), the razor (K-221), pan behind
(K-220), the type tool (K-225) and camera tools (K-229).

**Correcting a path after it is drawn.** With the Selection tool and the
wireframes on, every point of a selected layer's masks — and of a shape layer's
own art — is drawn as a small square you can aim at. Click one to pick it,
sweep a box over several to pick those, and drag to move them; the marks follow
the pointer and the picture catches up when you let go. A sweep that catches no
points is still the ordinary layer sweep it always was, so it is one gesture
doing two jobs depending on what is actually under it (K-224, K-307).

The reason masks and shape art behave identically is that they are the same
thing underneath: both are a list of points with two curve handles each, so one
piece of code finds them, draws them and moves them. The only difference is
which call writes the change back — and that difference is carried in the name
each point is filed under, so a shape point can never be saved as a mask.

**Where a shape's points are, and why that took a second go.** A shape layer is
sized by the art it holds: the layer *is* the box the drawing fits into, and it
grows and shrinks as the drawing is edited. That means the numbers stored for a
point — where it sits in the drawing — are not the same as where it sits *on the
layer*, which is measured from the box's top-left corner. The Viewer drew the
points as though the two were the same, so they appeared a whole box away from
the art, while the wireframe rectangle and the picture (which only need the box's
*size*) looked right. Subtracting the corner puts them back on the art (K-308).

Two things follow from the box being the drawing. The first: the outermost points
of a shape sit exactly where the box's resize handles do, so on a drawn square
every corner used to start a resize instead of an edit — a press close enough to a
point now means the point, and the handles keep the rest of the reach. The second:
dragging an outermost point *moves the box*, so everything else in the drawing
would slide the other way. The engine now moves the layer by the same amount in
the same edit, which is why the rest of the art stays put and why undo still puts
everything back in one step.

The picture also keeps up as you drag now, rather than waiting for you to let go:
the drag asks the engine for a provisional frame, at most one every twenty
milliseconds, exactly as dragging a layer about does.

What you still cannot do is drag a point's **curve handles** — the two arms that
decide how the line bends through it. You can pull them out while *placing* a
point with the Pen, but not afterwards, on any path. That is not an oversight
waiting to be wired: the file format has no way to say "these two arms are
linked" versus "this is a corner", so adding the gesture means adding that to
the format first, and deciding what an older project means without it.

### What the lock switch actually does

Locking a layer used to stop you dragging its bar, cutting it, renaming it,
reordering it or deleting it — but you could still open its twirl-down and
change its position, its effects or its volume. The switch said "no edits until
unlocked" and meant something narrower.

Now the refusal lives in the **engine**, in the one place every edit passes
through on its way into the document. That matters more than it sounds: there
are twenty-nine different kinds of edit a layer can receive, and guarding them
one interface control at a time means remembering to do it again every time a
new control is added — which is exactly how the hole opened, since the three
kinds of row that leaked are the three newest.

The rows are also shown greyed and untouchable, so you are not offered a gesture
that would only be refused. Headings still open and close: looking inside a
locked layer is not editing it.

Three things a locked layer still accepts, because none of them changes the
composition: **unlocking it** (or you could never get back), the **shy** flag
(which only hides the row from the Timeline's list) and its **label colour**.
Everything else waits until you unlock it.

Undo still works across a lock, and the reason is worth knowing because it is
what makes the whole approach safe: an edit can only have been made while the
layer was *unlocked*, so walking backwards through your history always reaches
the unlock before it reaches the edit underneath.

### Why a keyframe jumps onto things

Drag a keyframe along its lane with the magnet on — the horseshoe in the bar
under the Timeline — and it now wants to land on the things already there: the
start or end of a layer, a cut inside a sequence, another keyframe, a marker,
the playhead, the edges of the work area. Before, the only thing it wanted was
a whole frame.

**The reach is measured in screen pixels, not in time**, and that is the part
worth understanding. Zoomed right out, a hundred frames might be ten pixels
apart, and a snap that reached "two frames" would be useless. Zoomed right in,
one frame might be fifty pixels, and a snap that reached two frames would drag
your key somewhere you never pointed. Measuring the reach on the screen instead
means how far you are zoomed *is* how precise you are being — which is the thing
your hand already understands, so there is no second setting to learn.

When something catches the drag, a line is drawn at it. Without that, a key
that leaps to a spot the pointer wasn't looks like a bug rather than a service.

Two escapes. The magnet switch turns the whole thing off for as long as you like
— and with it off a key may sit *between* frames, which is occasionally exactly
what you want. Hold **Ctrl** during a drag and snapping stops just for that
moment, for the one time in ten when the place you want is precisely where a
snap will not let you put it.

Beat markers need no special mention in any of this, and that is by design: beat
detection writes ordinary markers, so dragging near a beat lands on it because
it lands on markers.

One thing the indicator broke on its way in, now fixed. The line is a piece of
the lane that only exists while a snap is holding the drag, and it was drawn
*before* the diamonds rather than after them. Flutter keeps a widget's identity
by its position in a list unless you name it, so a line appearing at the front
of that list shunted every diamond one place along, and each of them was rebuilt
as though it were a different diamond — including the one your pointer was
holding. A control rebuilt mid-drag loses the pointer, and losing the pointer
ends the drag: the key committed the two or three pixels it had travelled by
then and ignored the rest of the gesture. That is what "a keyframe will only
move one frame, and dragging it again puts it back" was — the second drag being
caught by the same target and landing back on it. The diamonds and the line are
named now, so each is rebuilt as itself and a drag lasts until you let go.

Right now this covers dragging a keyframe on its lane. Dragging a layer's bar,
the razor, the work-area handles and markers themselves still land wherever you
point. The arithmetic is written once and shared, so each of those is wiring
rather than a fresh design.

### Zooming that flies, and a slider that means something

**The zoom moves rather than jumping.** Magnification is a *place* changing, not
a number being nudged: jump straight from one zoom to another and you lose where
you were. The Viewer has moved smoothly for a while; the Timeline used to cut.
It now uses the same piece of code.

Three details in that motion, each there for a reason:

- It moves **geometrically**. Going from 1× to 16×, halfway through is 4×, not
  8.5×. Zoom is a ratio, so equal time should buy equal ratio — interpolate it
  the other way and the start lurches and the end crawls.
- **Rolling Ctrl+wheel faster zooms further.** A notch counts for more the
  sooner it follows the last one, up to four times. There is a ceiling on
  purpose: without one a quick flick crosses the entire zoom range and you
  cannot find your way back.
- **The frame under your pointer stays under your pointer** for the whole
  flight, not just at the ends. The lanes are growing the entire time, so the
  scroll position has to be corrected on every single frame of the animation —
  hold it still and whatever you were aiming at slides away from the cursor.

**The bottom bar's zoom is a slider**, between a small landscape and a large
one — the same pair After Effects puts either side of its own. Those two marks
are drawn by hand rather than taken from the icon set, for a reason worth
knowing: the icon set's glyphs are line drawings, and below about 16 pixels the
line is thinner than a pixel, so it gets smeared across two at half strength.
That is what "crunchy" small icons are. A filled shape has no line to lose, so
it stays clean at nine pixels, which is what lets the small end be plainly
smaller than the large one.

The slider's two ends are promises: all the way left is the whole composition,
and all the way right is **twenty frames** across the lanes.

Twenty *frames*, not a percentage, and that is the point. "6400%" tells you
nothing unless you also know how long the comp is; "twenty frames" means the
same thing on a five-second clip and a ten-minute one. So the right-hand end
moves with the composition rather than being a fixed number.

The slider also runs on the logarithm of the zoom, for the same reason the
motion does. A plain linear slider on a ten-minute comp would spend nine tenths
of its length inside the last handful of frames, and every zoom you actually
wanted would be crushed into the first centimetre.

The two ways of zooming hold different things still, deliberately. Ctrl and the
wheel keeps the **frame under the cursor** where it is, because there the cursor
is the whole gesture. The slider has no cursor to work from, so it keeps the
**playhead** where it is — that is where the work is happening, and it is what
After Effects zooms its timeline about. If the playhead has been scrolled out of
sight, the zoom brings it to the middle instead, because magnifying about
something you cannot see leaves you nowhere.

**A dragged slider does not animate**, and that is not laziness. The flight
exists to fill the gap between two zooms that arrive as *steps* — a wheel notch,
a click on the track. A drag is already a continuous motion, so animating it
means the lanes are always chasing a target your finger has already moved,
starting a new 120-millisecond journey before the last one arrived. It feels
like the panel is stuck to treacle. Dragged, the zoom simply is where the finger
put it.

**The scrollbar stops twitching, and the reason is where the correction
happens.** Keeping something still while the lanes grow means moving the scroll
position to match the new width. Do that the instant the zoom changes and you
have moved it to a place that only makes sense for a width the panel has not
laid out yet — so for the rest of that frame the view is scrolled past its own
end, Flutter starts pulling it back, and the little thumb in the bottom bar is
drawn from two numbers that do not agree. That is the jitter. Flutter tells a
scroll how big its content is *during* layout, and offers a way to say "I have
moved the offset, lay out again" — so the correction now happens there, where
the width and the offset are known at the same time, and nothing outside that
moment ever sees a mismatch.

**And a zoom only rebuilds the lanes.** This is the other half of the same
problem. The Timeline is two halves of one table: the layer names on the left,
the bars on the right. Nothing on the left depends on the zoom — but the panel
used to redraw *all* of it every time the zoom moved a fraction, which during an
animation is sixty times a second, and each of those redraws asked the engine
again for the work area, the render cache and more. Now the right-hand half
listens for the zoom by itself and the left-hand half sits still. The Timeline
already did exactly this for the playhead, for exactly the same reason.

A plain wheel still scrolls, as it always did — it never zooms without a
modifier, which is a rule the specification is firm about and this did not
change.

The graph editor and the Project panel's thumbnails still cut rather than fly.
They are the same job, and the shared piece is written.

### What is remembered, and where

- **The workspace** — panel arrangement, colour scheme, interface scale, tooltips,
  keymap, modal window positions. One `Workspace` object, written to a
  machine-local settings file. Nothing personal reaches the project file.
- **The session** — open comp tabs, front tab, playhead, selection, and the panel
  tree. Written both to the local settings file (keyed by project path, updated
  as you work) and as an opaque blob inside the `.lum` at save (K-245). The local
  copy wins; the blob is what a machine seeing the project for the first time
  reads. The engine carries that blob without ever parsing it, and drops it if it
  cannot be understood — an unreadable layout must never stop a project opening.
- **The document** — every committed edit copies the document, pushes an undo
  entry, appends a line to the journal and fsyncs before the interface sees it.
  Autosave keeps three rotating copies beside the project; if they are newer than
  the file on open, Lumit offers to replay them.

Rearranging panels is deliberately *not* an edit: it goes through `set_ui_state`,
a side door that skips undo, the journal and the dirty flag.

### Themes you can pass around (K-298)

A theme you make is a name, a light-or-dark base, and a bag of colours (K-202).
Until now it lived only in the settings file, which is machine-local — so a theme
was stuck on the computer it was made on, and the only way to try a variation was
to save over the one you liked.

Three things changed, all in the Flutter frontend and none of them touching the
engine:

- **A theme is a file.** `flutter_ui/lib/theme/theme_file.dart` writes one out as
  `.lumtheme` — a short, indented JSON document you can read: what it is, a
  version number, the theme's name, whether it is a light or a dark theme, and
  every colour as a hex code like `#e05a72`. Settings → Appearance has **Export…**
  and **Import…** beside the other theme buttons. Export works from a built-in
  scheme too, because "the stock dark with my accent changed" is a perfectly good
  thing to send somebody.
- **Reading one is deliberately relaxed.** If the file was written by a newer
  Lumit that has colours this build has never heard of, those are simply ignored
  and everything else comes in — the theme still works, because any colour it does
  not carry is taken from the base underneath it. That tolerance is the whole
  reason a theme is stored as *changes over a base* rather than a copy of the
  colour struct. A file that is not a theme at all is refused with a sentence
  under the buttons, not an error box: picking the wrong file is a normal thing to
  do.
- **Nothing overwrites a theme you already have.** Import, Duplicate, Save a
  copy and Rename all ask `Workspace.availableThemeName` for a free name first, so
  importing a second "Ocean" gives you "Ocean 2" and says so. A theme's name is
  its identity — the picker lists it and the settings file records the selection
  by it — so two themes may never share one.

Beside that sits the everyday half: **Duplicate** (copy the theme you are looking
at, including a built-in, so you have something of your own to edit), **Rename…**
and **Delete** (your own themes only — a built-in's name is Lumit's), and **Save a
copy…** inside the colour editor, which branches a theme without first
overwriting it. The picker also draws eight swatches of the selected theme beside
its name, so you can recognise a theme without applying it.

### The rules that bite

These are the ones a plausible-looking change breaks. Each has tests standing
behind it.

- **No bridge calls in a rebuild path.** If mouse movement can trigger a rebuild,
  nothing in it may cross the bridge; the answer is computed once and held.
  Standing budget tests fail the build when crossings creep up (K-230, K-231).
- **A dragged value is a preview, not an edit.** The engine renders a copy of the
  project with one value replaced — no document write, no undo entry, no journal
  line, and the pixels are never cached. Release commits once (K-239, K-240).
- **One gesture, one undo step.** `Op::Batch` groups the pieces, so a two-axis
  position drag or a typing session is a single `Ctrl+Z`.
- **Never change a widget's shape to show state.** Flutter throws away and
  rebuilds anything whose shape changed, taking scroll position and in-flight
  drags with it. Focus outlines and hover borders are always present and merely
  transparent when unseen.
- **Throttle by holding the newest, never by dropping it**
  (`state/preview_throttle.dart`, mirrored by the engine's worker).
- **Register the new texture before releasing the old one** — never show less
  than you were showing a moment ago.
- **A gesture whose meaning depends on where it began must record where it began.**
  Flutter only reports a drag after about 18 px of travel.
- **The broader scope asks the narrower one first.** Flutter runs every key
  handler in registration order, so the shell's `Delete` asks the Timeline's claim
  before acting (K-234).
- **A readout that counts must not resize as it counts** (K-287). Numbers get a
  slot as wide as the longest thing they can ever say, and a badge that comes
  and goes keeps its slot while it is away. See below.
- **Tests must let real time pass.** `settleFrb` in
  `flutter_ui/test/frb/frb_test_support.dart` alternates real-time slices with
  fake-clock pumps until the expected state arrives; `await tester.pump()` alone
  advances no clock, and awaiting an engine call inside `runAsync` that was not
  started there deadlocks (K-233).

### The clock readouts (`widgets/time_readout.dart`)

**The problem, in plain terms.** Text is drawn as wide as it needs to be. `f9`
is narrower than `f10`, and in most typefaces the digit `1` is narrower than the
digit `8` — so a timecode counting up is a piece of text that changes width
several times a second. Everything laid out beside it slides to keep up. During
playback that means the Timeline's search field twitching sixty times a second,
in the corner of your eye, for no reason anybody can act on.

**The fix.** `TimeReadout` is one small widget every clock on a bar now uses. It
does three things:

- **It reserves its width.** You tell it how many characters the longest thing it
  could ever say is — `00:00:00:00` is eleven — and it measures that many
  characters of its own typeface *once*, caches the answer, and draws the number
  inside a box of exactly that size. The number changes; the box never does.
- **It can be typed into.** Clicking it swaps the text for a field already
  holding what was on screen, selected, so typing replaces it. `Enter` or
  clicking away takes what you typed; `Escape` throws it away. The widget knows
  nothing about timecode: whoever uses it hands over a *format* function (a frame
  number → the text) and a *parse* function (text → a frame number, or nothing if
  it does not read as a time). That is why the same widget serves the Viewer's
  clock, the Timeline's clock, the Timeline's `f72` frame count and the Retime
  row's source position.
- **It clamps rather than refuses.** A time past either end of the composition
  lands on that end. Asking for frame 100000 in a 300-frame comp obviously means
  "the end", and an error message would be a worse answer than the obvious one.

The same "reserve the space" rule applies to things that are not text: the
Viewer's degradation badge (the "Half" chip that appears when playback has had to
soften the picture) keeps its empty slot when it is not showing, so it does not
shove the bar sideways as it comes and goes.

### Where the memory went (K-294)

**The problem this solves.** Twice now Lumit has been found holding tens of
gigabytes of memory on a Mac. Both times the hard part was not fixing it — it
was working out *what* was holding it. Lumit keeps several stores of pictures:
finished frames in memory, finished frames on the graphics card, decoded frames
from video files, frames queued to be written to disk. Each has a budget and
each throws things away to stay inside it. So either one of them was misbehaving,
or something outside all of them was holding memory nobody was counting — and
from outside the program those two look identical.

**The fix is a subtraction.** Settings ▸ Performance now opens with a Memory
section: the total the operating system says Lumit is holding, then what each
store admits to, and then the difference. If the stores add up to half a gigabyte
and the total says eighty-five, the answer is "none of these" — which sounds like
nothing but is most of the investigation, because it rules out everything with a
budget and points at the layers underneath (the graphics driver, the video
decoders).

Three details that keep the arithmetic honest, all of which were tempting to get
wrong:

- **Frames on the graphics card are shown but not subtracted.** On an Apple
  Silicon Mac the graphics memory *is* the system memory, so those frames are
  already inside the total; on a PC with a separate graphics card they are not.
  Subtracting them would be right on one machine and wrong on the other, so the
  report shows the figure and lets you read it.
- **Nothing is counted twice.** A frame waiting to be written to disk is the
  same piece of memory as the copy in the frame cache — one picture, two lists —
  so the queue reports how many frames are waiting, not how many bytes.
- **What cannot be measured is counted instead.** Nobody outside FFmpeg knows how
  much memory an open video decoder holds, so the report says how many are open
  rather than inventing a number.

One more row asks the **graphics driver** how many pictures and buffers it is
still holding for Lumit. A handful is normal: the frames kept on the card, and
the working pictures of whatever frame is being made right now. Thousands would
mean pictures Lumit had finished with were never actually destroyed — which is a
different fault from any cache being too big, and on a Mac that memory is inside
the total at the top.

Counting them, rather than measuring them, is deliberate. The first version of
this row asked the driver for bytes, and on a Mac the answer was "not reported
by this driver" — that particular question only has an answer on Windows and
Linux. A count is a count on every machine, and it happens to be the sharper
question anyway: it distinguishes a big cache from a leak, which bytes alone
cannot.

The report is a **debug-build tool**: it is there while a fault is being hunted,
and a shipped Lumit does not show it. Asking somebody editing a video to
interpret a live texture count is handing them the engineering instead of the
tool.

### And the repair it found (K-295)

Here is what the instrument caught. Telling the graphics card "I have finished
with this picture" does not give the memory back. It marks it finished, and the
memory returns the next time the program asks the card to tidy up. A program
that is drawing to a window asks constantly, without meaning to, because showing
a frame *is* asking. Lumit spends much of its time drawing into its caches
instead — no window, no asking, and so a pile of finished pictures nobody had
collected.

That is why the memory came back when the owner switched panels: the switch
happened to ask. Now the engine asks once per turn of its own loop, whether
anything is on screen or not, which costs nothing when there is nothing to
collect and means the pile is never more than a moment old.

**Two ways to ask, and the test needed the other one.** Asking the card to tidy
up comes in two forms. "Collect anything you have finished with, and don't keep
me waiting" is the one the loop uses, because a loop that must produce a picture
cannot afford to stand still. But work handed to a graphics card does not happen
when you hand it over — it happens when the card gets to it, and a computer can
hand over frames far faster than a card draws them. So there is always a queue,
and everything still in that queue is memory the card cannot possibly release
yet. Ask the impatient way and the answer includes the queue.

The other form is "finish what you have, *then* collect", and it waits. That is
wrong inside a loop and exactly right for two other moments: an engine with
nothing left to draw, and a **measurement**. This matters because a test was
asking the impatient question and reading the queue as though it were a leak: on
a Mac it saw 113 abandoned pictures where the truth was a handful, and on Windows
577, while the same test on the build machine — which has no real graphics card,
so nothing ever queues — saw eighteen and looked perfectly healthy. The number
only means anything once the card has caught up.

## 10. The app icon and the brand files

The icon you see in the taskbar is not one picture — it is a small bag of
pictures. Windows keeps them all in a single `.ico` file (ours holds seven
sizes, 256 pixels down to 16), and shows whichever one fits the spot: big for
the desktop, tiny for a browser-style tab. macOS does the same thing with a
folder of loose PNGs instead of a bag.

Nobody draws seven pictures by hand. The artwork is drawn **once**, as an SVG —
a text file of drawing instructions ("a rounded square here, this gradient
there") that can be rendered at any size without going blurry. The five SVGs in
`assets/brand/` are the only files a human edits:

- `lumit-mark.svg` — the mark itself: two keyframe diamonds overlapping, white
  where they cross. This bare form is the Windows and Linux icon.
- `lumit-icon.svg` — the same mark sitting on a dark rounded tile. Only macOS
  uses this, because macOS expects every icon to bring its own tile.
- `lumit-project.svg`, `lumit-preset.svg` and `lumit-theme.svg` — document
  icons for `.lum` project files, `.lumfx` presets and `.lumtheme` colour
  themes: a dark page with a folded corner and the mark inside, like the little
  badge on any Photoshop or After Effects file. The theme one carries three
  overlapping colour swatches instead of the mark, since colours are what is in
  the file.

`scripts/gen-icons.py` turns those four drawings into every pixel file the
operating systems want (run `pip install resvg-py pillow` once, then
`python scripts/gen-icons.py`). It renders each size straight from the SVG
rather than shrinking one big picture — that is what keeps the 16-pixel
version crisp instead of mushy. You only run it after editing an SVG; the
generated files are committed, so a fresh checkout builds without it.

The document icons only appear next to your `.lum` files once something tells
the operating system "files ending in .lum belong to Lumit, use this icon".
Running the app never does that — it is an *installer's* job, and the
installers live in `packaging/` (decision K-252):

- **Windows** — `packaging/windows/build-installer.ps1` builds a normal
  setup.exe (it needs the free Inno Setup tool once:
  `winget install JRSoftware.InnoSetup`). Installing it copies the app into
  Program Files, writes the .lum/.lumfx/.lumtheme entries into the Windows
  registry with their icons, and puts Lumit in the Start menu. Double-clicking a `.lum` then
  genuinely opens it: the association hands Lumit the file's path as a command
  line argument, and the app checks its command line at boot
  (`projectPathFromArgs` in `main.dart`).
- **Linux** — `packaging/linux/install.sh` copies a built bundle into
  `~/.local`, and installs the desktop entry, the file-type declarations, and
  the icons where any desktop environment looks for them. No root needed.
  Releases themselves ship a Flatpak instead: a format that packs the app
  together with everything it needs, so one file installs the same way on
  Ubuntu, Fedora or Arch (`flatpak install lumit-….flatpak`). The recipe in
  `packaging/flatpak/` does no building of its own — it repacks the
  already-built bundle, FFmpeg included. The one thing it cannot do is the
  `.lum` icons: Flatpak only publishes icons named after the application, so
  document icons and double-click opening stay a job for `install.sh`.
- **macOS** — `packaging/macos/make-dmg.sh` produces the usual drag-to-
  Applications disk image (on a Mac): a white window with the app on the
  left, the Applications folder on the right, and a curved arrow showing
  the drag. The
  file-type declarations are in the app's Info.plist already, but their icons
  and double-click opening land with the larger macOS pass in the TODO.

None of this runs on `flutter run` — a dev run shows the app icon (it is baked
into the executable) but registers nothing.

Releases do not depend on your machines at all. Pushing a git tag that starts
with `v` (say `v0.1.0`) wakes `.github/workflows/release.yml`, and GitHub's
own computers do the work: a Windows machine builds the setup.exe, a Mac
builds the disk image, and a Linux machine builds a bundle and repacks it into
the Flatpak. All three land attached to a GitHub Release under that tag, and
those three files are the entire release (K-290) — one per platform, nothing
else.

This is the answer to "can I do a whole release from my Windows desktop": yes,
but only by letting the tag do it. Flutter cannot cross-build — a Windows
machine can only make the Windows app, and an Apple disk image can only be
made on a Mac — so no single computer you own can produce all three. GitHub's
runners are three computers pretending to be one button.

Every job gates the release, so if any platform fails, no release appears at
all. That is deliberate: a release quietly missing its Mac build looks exactly
like a release that never had one, and you would find out from a user rather
than from CI.

**Signing** is the paid certificate that tells Windows and macOS "a known
person made this". Without it, SmartScreen shows a blue warning panel and
Gatekeeper refuses the first double-click (right-click → Open gets past it,
once). The Windows installer is still unsigned, because that needs a
code-signing certificate nobody has bought yet.

The Mac side is signed, and it involves two separate things that are easy to
confuse. **Signing** puts your identity on the app: this came from Mackenzie
Reed, and here is Apple's certificate saying Apple agrees that is a real
person. **Notarisation** is a second step where you upload the finished app to
Apple, their automated scanner checks it for malware, and they hand back a
"ticket" — a note saying this exact build passed. **Stapling** attaches that
ticket to the file itself, so a Mac can see the app is approved without asking
Apple over the internet. Apple wants all three, and a downloaded app that is
signed but not notarised is treated almost as harshly as one that is neither.

Two things about that are worth knowing because they cause confusing failures.
The app is notarised *twice*, separately: once as the `.app` on its own and
once as the `.dmg` it rides inside. A ticket only covers the exact file you
sent, so approving the disk image says nothing about the bare app that the
updater downloads. And the signing has to happen from the inside out — the
FFmpeg libraries first, the app last — because signing the app records a
fingerprint of everything inside it, and touching anything afterwards makes
that fingerprint wrong. macOS then refuses to launch the app at all, which
looks like a mysterious crash rather than a signing problem.

None of this happens on your machine. `make-dmg.sh` only signs for real if it
finds the certificate details in its environment, which happens in CI, where
they are stored as GitHub repository secrets. Run the same script on a laptop
and it falls back to an "ad-hoc" signature — an unnamed one, worth nothing to
Gatekeeper, but which macOS insists on before it will run an app carrying its
own copies of FFmpeg at all — and skips notarisation entirely. That is
deliberate: one script for both cases means the signing path cannot quietly rot
between releases.

To rehearse all this without publishing anything real, tag a **pre-release**:
any tag with a suffix, like `v0.2.0-rc1`, runs the identical pipeline but
marks the result a pre-release, so the download page's "latest" ignores it.
Delete it afterwards and tag the real version.

## 11. Keeping Lumit up to date, in plain terms

Every release already ends up in the same place: a GitHub Release, tagged `v0.1.0`
or whatever the version is, with the finished installers hanging off it — a
`setup.exe` for Windows, a disk image for macOS, and a Flatpak for Linux
(K-304). That is the whole raw material the updater needs, and it means Lumit has
no update server to run and nothing to pay for.

**What "check for updates" actually does.** GitHub will answer a small,
public question — *what is the newest release of this repository, and what is
attached to it?* — over the same sort of web request a browser makes. Lumit asks
it, reads the tag (`v0.2.0`), strips the `v`, and compares that with the version
this build reports on start-up: the very first line of the boot log, the one the
About window shows. Comparing versions is fiddlier than it looks — `0.10.0` is
newer than `0.9.0` even though it sorts earlier as text, and `0.2.0-rc.1` is
*older* than `0.2.0` because a release candidate comes before the release. There
is a small function for exactly that, and a test for each of those traps.

**One menu row does everything.** Help ▸ Check for updates is not a button that
opens an update window; it is the update, in a row. Press it and it goes grey and
says *Checking for updates…*. A second later it either says *Click to update -
v0.2.0* or goes back to *Check for updates*, with "Lumit is up to date" in the
status line at the bottom. Press the offered version and Lumit asks whether to
fetch it, tells you how big it is, and shows a bar while it comes; the row
counts along too, in case you closed the window and went back to editing. When it
has arrived the row says *Restart to finish updating* until you do.

Making a menu row behave like that needed one new trick: menus in Lumit are
normally a list of labels decided the moment the menu opens, and pressing any row
closes the menu. This one row is a `MenuEntry.live` — it redraws itself while the
menu is open and it stays open when pressed, because the whole point of pressing
it is to watch what happens. It is the only row like that, deliberately.

**Automatic updates.** There is a tick on the setup screen, and the same tick in
Settings ▸ General ▸ Updates. It is on to begin with, and it means one specific
thing: when Lumit starts, and no more than once a day, it *looks*. It never
downloads anything on its own. If there is something newer, all that happens is
that the Help row is already saying so the next time you open the menu. Someone
editing on a hotel connection should never discover Lumit quietly spending their
data allowance.

**Why the whole installer, rather than a patch.** Patches are smaller, and that
is genuinely nice; the price is publishing a separate patch for every pair of
versions people might be coming from, writing the tool that applies them, and
then writing the fallback for when somebody's particular pair does not exist.
That is three new things that can be broken in order to save bandwidth GitHub
gives us for free. So: the whole installer, every time, on a click you made.

**What stops a bad download from being run.** The release says how many bytes the
installer is and — where GitHub provides one — its SHA-256, which is a
fingerprint of the file's contents: change one byte and the fingerprint changes
completely. Lumit checks both before it will run anything, and deletes the file
if either disagrees. An installer is the most dangerous file Lumit ever touches;
a truncated download or a swapped file is caught here or not at all.

**Why you have to restart.** An installer cannot overwrite a program that is
running, which is a rule of the operating system rather than a choice of ours. So
the last window says *Restart to finish updating*. If you have unsaved work open,
it offers **Save and restart** as well as **Restart without saving** — losing an
evening's work to a version number would be an absurd way to lose it — and
**Later**, which keeps the downloaded installer so you can finish whenever you
like. On Windows the installer runs itself quietly and Lumit closes; on macOS the
disk image opens and Lumit closes; on Linux Lumit only shows you the downloaded
file, because unpacking a bundle or installing a Flatpak is something you do
where you want it, not something an editor should do behind you.

**Where the code is (K-296).** `flutter_ui/lib/state/updates.dart` knows about versions,
downloads and files; `flutter_ui/lib/shell/update_dialog_frb.dart` is the windows
you see. None of it is in Rust: the engine has no business knowing about the
internet, and this way nothing that renders frames grows a network dependency.
Every point where the updater touches the outside world — asking GitHub,
downloading, running an installer, quitting — is a swappable seam, which is how
the tests exercise the entire sequence without ever going near a network or
actually running anything.

### Updating without an installer (K-297)

The section above describes updating by downloading the installer and running
it. That still happens in some cases, but it is no longer the normal one — and
the reason is not clever code, it is *where Lumit lives*.

**Why Chrome never shows you an installer.** Programs on Windows traditionally
go in `Program Files`, a folder only an administrator may write to. That is why
updating one always involves a prompt: the program cannot change its own files,
so it has to ask an installer, and the installer has to ask Windows for
permission. Chrome, VS Code and Discord sidestep the whole ceremony by
installing somewhere *you* own — a folder inside your own user profile. A
program running as you can rewrite a folder belonging to you without asking
anybody. Lumit now installs there too (`%LOCALAPPDATA%\Programs\Lumit`).

**What a release now carries.** As well as the installer for each platform,
every release publishes the application *by itself* as a plain archive: a zip of
the Windows files and a zip of the macOS `Lumit.app`. That archive is what Lumit
downloads to update itself. Nothing in it is an installer — it is simply the new
version of the same files. Linux needs no archive of its own: a Flatpak is
updated from the `.flatpak` bundle the release already carries, and the Linux
tarball this section used to name is withdrawn (K-304).

**How the swap works, and why it is done that way.** The obvious approach is to
copy the new files over the old ones. Do not: that is several hundred separate
operations, and if anything interrupts it half way — a crash, a flat battery —
you are left with a Lumit that is half one version and half another, which may
not start at all. Instead Lumit unpacks the whole new version *beside* the old
one, then does two renames:

1. `Lumit` becomes `Lumit.old`
2. `Lumit.new` becomes `Lumit`

A rename is a single filesystem operation: it either happened or it did not,
with nothing in between. So at every moment there is a complete, working Lumit
on disk — the old one, or the new one. If the second rename fails, the first is
undone on the spot, and the code doing the undoing is already loaded in memory,
so it does not need the files it is moving.

The old folder is deliberately left lying there. Windows will not let anyone
delete a `.dll` that is currently loaded, and at that moment Lumit is still
running from those very files. So it is swept up on the *next* launch, by the
new version, when nothing is holding it — that is the first thing `main()` does.
The same sweep also puts the old version back if it ever finds the install
folder missing, which is what a machine dying between those two renames would
leave behind.

**Three ways an update can be applied**, and Lumit works out which by looking at
where it is actually installed:

- **In place** — the swap above. Requires a folder Lumit can write beside, which
  it checks by genuinely writing a small file there rather than guessing from
  the path.
- **By installer** — for an older installation still in `Program Files`, or a
  macOS disk image. Exactly the K-296 behaviour, kept as the fallback so nobody
  is stranded.
- **Handed to Flatpak** — see below.

**Flatpak is the exception, and honestly so.** A Flatpak runs in a sandbox whose
files are read-only on purpose: that is the security model, not an oversight. An
application inside one genuinely cannot replace its own files, and the ways to
reach out to the host and do it anyway require permissions no video editor
should be asking for. So on Flatpak, Lumit downloads the new bundle, shows you
the one command that installs it, and stays open. Making this a real
`flatpak update` needs Lumit published to a Flatpak *remote* rather than as a
single downloadable bundle — that is written down in TODO as the next step.

## 12. Speaking other languages, in plain terms

Lumit used to have its words typed directly into the code. A button that said
*Import footage* was a line somewhere that literally read `Text('Import
footage')`. That is the natural way to write it, and it is fine right up until
somebody wants the button to say *Footage importieren* — at which point there is
no way to change it without editing ninety files, and no way for a translator to
help at all unless they are willing to learn Dart and be given commit access.

So the words moved out. There is now one file, `flutter_ui/lib/l10n/app_en.arb`,
which is a long list of every phrase Lumit can show, each with a short name. It
looks like this:

```json
"importFootage": "Import footage",
"@importFootage": {
  "description": "Button, menu item and tooltip: bring media files into the project."
},
```

The code now says `l10n.importFootage` instead of the phrase itself. `l10n` is
"localisation" abbreviated the way the industry abbreviates it — an *l*, ten
letters, an *n*. When Lumit is running in English it hands back "Import footage";
in German it hands back whatever the German file has under that name.

The `@importFootage` part underneath is a note **for the translator**, not for
the program. It is the only context they get: they see the phrase and that
sentence and nothing else, no screenshot, no surrounding page. Writing a good one
is the difference between *Fill* being translated as "to fill something up" and
as "the colour inside a shape". Every string has one, and a test fails if any
string does not.

### Where the translations come from

Your friends do the translating on **Crowdin**, which is a website built for
exactly this. They see the English phrase, its note, and a box to type theirs
into. Nobody clones the repository, nobody runs Flutter, nobody can break the
build by mistyping a bracket.

The traffic is one-way in each direction, and it matters which is which:

- **English goes up.** You change `app_en.arb`, run `crowdin push sources`, and
  Crowdin now offers the new phrase to translate.
- **Everything else comes down.** `crowdin pull translations` writes
  `app_de.arb`, `app_kk.arb`, `app_uk.arb` and `app_zh.arb` into the same folder.

Those four files are **never edited by hand in this repository.** If a German
phrase is wrong, it is fixed on Crowdin; fixing it here works until the next pull
overwrites it, which is worse than not working at all because it works for a
while first. `crowdin.yml` at the top of the repository is the whole
configuration — which file goes up, and what each language is called on disk.

The two passwords Crowdin needs are not written in that file, because this
repository is public. They are read from the environment instead:
`CROWDIN_PROJECT_ID` and `CROWDIN_PERSONAL_TOKEN`.

### What happens when a translation is missing

Nothing bad, which is the point. A phrase nobody has reached yet falls back to
the English one. That means the four language files can start out completely
empty — as they are today — and Lumit runs exactly as it did before, in English.
As your friends fill them in, more of the interface turns over. There is never a
moment where the application is half-broken waiting for a translation to arrive;
it is only ever more or less translated.

### The engine's words

Some of what you read on screen is not written in Dart at all. The effects name
themselves — *Gaussian blur*, *Radius*, *Blur & sharpen* — in the Rust engine's
schema, and so does every keyboard shortcut in Settings ▸ Keymap: *Play or pause*,
*Anywhere*. Those come up to the interface as plain English through the bridge.

Rather than teach the engine about languages, which would be a large change deep
in code that has no other reason to care, `lib/l10n/engine_labels.dart` holds a
table that looks each one up **by its English text**. The engine says "Gaussian
blur"; the table turns that into the German for it. Rust is untouched.

The obvious danger is that somebody adds an effect and forgets the table, so the
new effect ships in English inside an otherwise German application, and nobody
notices for months. `test/l10n/engine_labels_test.dart` prevents that: it reads
the Rust source files directly and fails if the engine can say a word the table
has no entry for.

### Why the tooltips got shorter

While the words were being moved, they were also read — all of them, in one place,
for the first time. A lot of the tooltips had quietly grown into paragraphs
explaining things the button already said. A Reset button whose tooltip read *"Put
every parameter back to its default, removing its keyframes"* is a sentence you
have to stop and read to learn something you already knew.

The specification always asked for the opposite: `docs/07-UI-SPEC.md` §13.2 says
a tooltip is the control's **name and its shortcut**, and reserves the
sentence-length kind for genuinely Lumit-specific ideas. So every tooltip is now
under five words and most are two — *Reset all parameters*, *Add keyframe*,
*Label colour*. Six are allowed to be longer, and they are listed by name in
`test/l10n/arb_test.dart` with the reason: the three cache meters, whose tooltips
carry live numbers and warn you that clicking throws work away, and the two
playback modes. Anything else that grows past five words fails that test.

### Choosing a language

Settings ▸ Interface ▸ Language. It defaults to whatever the machine itself is
set to, and stores nothing until you choose — so if you never open the picker,
Lumit follows your operating system for ever, including after you change it.

The list names each language in its own language: Deutsch, Қазақша, Українська,
简体中文. That is deliberate. Somebody who has set Lumit to a language they turn
out not to read needs to be able to find their way back, and they will not do it
by looking for the word "German".
### A text layer that says whatever the expression works out

Until now a text layer said one fixed thing. You typed some words, and those
words were what appeared, for the whole length of the layer. Everything else on
a layer can be animated — position, opacity, an effect's knob — and now, with
expressions in, animated by a little sum instead of by keyframes. The words were
the one thing that could not move.

A text layer now has a second box next to its words: **Expression**. Leave it
empty and nothing changes; the layer says what you typed. Put something in it —
`time`, or `time * 30`, or `"frame " + (time * comp_fps)` — and the layer says
whatever that works out to, freshly, at every frame. It is the same little
language the numeric knobs use. The only difference is what happens to the
answer: a knob *measures* it, and a text layer *prints* it.

That is deliberately the whole feature. It is there so a number can be put on
screen — a counter, a readout, and above all the value of an expression you are
in the middle of writing, which is much easier to debug when you can watch it
rather than infer it. Letters do not fly in one at a time; that is per-character
animation and it belongs with the styled-text work, later.

Three details are worth knowing, because each is a trap avoided rather than a
feature added:

**Your typed words are kept.** Setting an expression does not overwrite them.
They sit underneath, and clearing the Expression box hands the layer straight
back to them. An empty box means "no expression", never "an expression that
produces nothing" — otherwise emptying the field would leave you with a blank
layer and no way back.

**A broken expression prints nothing rather than stopping the render.** You will
be typing these while the preview is live, and half a typed expression is not
valid for most of the time it takes to write one. An unreadable caption for a
moment is a much smaller problem than a render that falls over, and an empty
line is the same thing the editor already shows you for empty text.

**The cache is told the truth.** Lumit keeps rendered frames and reuses them,
and it decides whether two frames are the same by hashing everything that went
into them. If it hashed the *typed* words for a layer whose words come from an
expression, every frame of that layer would hash identically — and the number on
screen would freeze on whatever it read the first time, which is the exact bug
this feature would otherwise ship with. So the rasteriser and the cache key ask
the same one function for the line, and by construction cannot disagree: a
counter keys a new frame each frame, and an expression that always says the same
thing keys once and is reused, with nothing to configure either way.

### Expressions, and why the engine is kept rather than built

An expression is a line of code sitting where a number would normally sit. A
property — position, rotation, opacity, an effect's knob — usually holds either
a fixed value or a row of keyframes. With an expression it holds a small sum
instead, and the answer is worked out afresh every time the property is read:
`time * 90` spins a layer at ninety degrees a second without a single keyframe.

The language is [Rhai](https://rhai.rs), a small scripting language that embeds
into Rust. An expression can read a handful of things about where it is —
`time`, `comp_width`, `comp_fps`, `cut_in` — and it can reach other layers:
`layer("Sun").x` is the horizontal position of the layer called Sun, at this
moment. That last one is what makes expressions worth having rather than a
novelty, and it is also what makes them awkward to implement, for two reasons
that are worth spelling out.

**Expressions nest.** Asking for `layer("Sun").x` means evaluating Sun's own x
property, and Sun's x may itself be an expression. So evaluating one expression
can start another, part-way through, before the first has finished. In
programming terms the evaluator is *re-entrant*: it has to cope with being
called again while it is already running.

**And they can chase their own tail.** If layer A's position reads layer B's,
and B's reads A's, that nesting never bottoms out. Lumit counts how deep it has
gone and gives up at a hundred, which is far more nesting than any real rig uses
and far less than it takes to exhaust the stack and crash.

Now the part that looks odd in the code. To run an expression you need an
*engine* — the Rhai interpreter, with all of Lumit's functions (`sin`, `noise`,
`layer`, `comp`) registered into it so expressions can call them. Building one
is not cheap: registering those functions takes roughly 370 microseconds, which
sounds like nothing until you notice how often expressions run.

They run *constantly*. Every driven property is re-evaluated for every frame,
and twice over: once by the renderer to draw the picture, and once by the frame
cache to work out whether this frame is one it has already got. At sixty frames
a second, a rig with a few dozen driven properties is tens of thousands of
evaluations a second. At 370µs each, forty-odd of them would use up the entire
budget for a frame before anything was drawn.

The obvious fix is to build one engine and keep it. That does not quite work
here, because the engine is where the *context* is parked — which comp, which
layer, what time — and a nested evaluation needs a different context from the
one that is running. One shared engine would have the inner expression trample
the outer one's context.

So Lumit keeps a small **pool**: a pile of ready-built engines. An evaluation
takes one off the pile, uses it, and puts it back. A nested evaluation takes a
second one, so the two never share. In practice the pile ends up as deep as the
deepest nesting, which is nearly always one. The cost per evaluation drops from
about 370 microseconds to about one — the engine is built once and then simply
handed round.

The same instinct applies a little further out. An expression needs to be able
to look at the project — that is how `layer("Sun")` finds Sun — so it holds a
reference to the document. Handing it a *copy* of the project would be tidy but
ruinous: copying a two-hundred-layer project takes about 150µs, and doing that
once per layer per frame is thirty milliseconds a frame, which is twice the
whole budget, spent entirely on copying. So the project is shared rather than
copied (an `Arc`, in Rust's terms — a single copy that many things can point at
safely), and the sharing is set up once per frame rather than once per layer.

The general lesson, which applies well beyond expressions: anything on the
per-frame path is run tens of thousands of times a second, so the question is
never "is this fast enough once", it is "what is this multiplied by sixty, by
the number of layers".
