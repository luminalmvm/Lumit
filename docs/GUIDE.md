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
- **Why that drop once died silently (the one-slot drag rule).** egui carries exactly
  one "thing being dragged" for the whole app, like a single hand that can hold one
  object. The catch: when any drop zone asks "was that released on me?", egui hands
  the object over *before* checking whether it is the kind that zone wanted — and if
  it is the wrong kind, the object is simply gone. The Timeline's whole-body zone
  (the one that accepts footage dropped from the Project panel) sits underneath every
  layer row, so it asked first, was handed the dragged *effect*, shrugged, and
  discarded it — the row you actually dropped on found the hand empty. The fix is a
  small shared reader (`dnd_release_of` in `panels.rs`) that peeks at the kind first
  and only takes a drop that matches; every drop zone in the app now reads through
  it, so a footage drag and an effect drag can never eat each other again.
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
- **Colour picker and eyedropper.** Every effect **Colour** parameter — a Flash tint, a Colour
  balance wheel, the Matte key's Key colour, and so on — now shows a **clickable swatch**. Click
  it and Lumit's colour wheel and sliders open, so you can pick a colour by eye instead of typing
  three numbers. Beside the swatch sits a small **eyedropper**: click it and the tool arms, then
  move the pointer over the Viewer and a **magnifier** follows the cursor. The magnifier shows a
  zoomed 9×9 grid of the pixels under the pointer, dotted lines between them and the centre pixel
  ringed; click to lift that colour into the parameter, or press **Escape** (or click off the
  Viewer) to cancel. **Shift+scroll** while it is up grows the sampled patch — 1×1, 2×2, 3×3, … —
  so you can average over a grainy area instead of grabbing one noisy pixel; the current size
  shows under the grid, and the committed colour is the average over that patch. Depth of field's
  **Focus** carries the same eyedropper, except it lifts *depth* rather than colour: click the
  part of the picture you want sharp and Focus jumps to it. The pixels are read straight from the
  frame shown in the Viewer — the very frame the Scopes read — and a picked colour is converted
  back into Lumit's internal light space so it matches what you sampled. Two honest notes: the
  wheel edits ordinary 0–1 colours, so a rare "brighter than white" tint is clamped by the picker
  (the number boxes still reach it); and the Focus pick uses the brightness of the clicked pixel
  as a stand-in for depth, since the depth layer's own picture is not separately available to the
  panel.
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
  outline sits to its left, the two scroll **completely apart** in graph mode (UI-8): the layer
  list gets its own vertical scrollbar tucked against the **right edge of the outline**, a wheel
  over the graph pans the *curve* only, and a wheel over the outline (or a drag of that
  scrollbar) moves the *layer list* only — neither ever nudges the other. This is the one place
  they differ from the ordinary lane view, where the outline and the lanes ride a **single**
  shared scroll so a row's controls and its bar always move together.
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
  without hunting for them by eye. (Effect parameters get this same navigator now too — an
  animated Glow radius or blur amount steps and adds/removes keys from its row exactly as a
  transform property does.)
  **The diamonds on the lane are live, not just a picture (notes 2.1/2.6).** Click a keyframe
  diamond to select it — it wears a ring — and **drag it left or right to change its time**;
  while the **magnet** (the bottom-bar toggle, on by default) is lit it snaps to the nearest
  whole frame, exactly like a key drag in the graph editor. On the lane only the *time* moves
  (a key's value and easing are shaped in the graph editor). Select several at once and they
  slide together as one undo step: **Shift-click** adds a key to the selection, **Ctrl-click**
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
  zoom per cent on the left, a **Grid** picker and a **magnet** toggle (on by default) that
  governs whether a dragged keyframe snaps its time to the nearest whole frame — the magnet
  shows in both the Layers and Graph views — the Layers/Graph view toggle on the right, and a
  draggable horizontal scrollbar just above it (the vertical scrollbar stops above the bar so
  the two never fight). Layers/Graph is only a change of what the lanes *draw* — the outline stays
  identical between the two, so twirling a layer open shows the same rows either way.
- **Working the layer outline** — a few habits from other editors now work the way you
  would expect. The outline's switches sit in After Effects' five familiar clusters
  (K-168), left to right: first the **eye, speaker, solo dot and padlock**; then a small
  **label-colour chip**, the layer's **stack number** and its **name**; then the
  **flow-or-collapse glyph, an fx bypass switch, motion blur and 3D**; then the **Matte
  and Blend** dropdowns; and at the far right a **Parent** dropdown (the same
  parent-and-inherit link the Effect Controls tab offers — pick another layer and this one
  rides its transform). The padlock freezes a layer's *timing*: while locked, its bar
  will not slide, its ends will not trim and it will not reorder in the stack — though its
  values stay editable, since the lock exists to stop stray drags, not work. The label
  chip cycles through eight theme colours with a click, purely for telling layers apart
  in a tall stack; it changes nothing about the picture. A row of tiny icons sits over
  the outline columns, level with the time ruler, naming each cluster at a glance. The
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
  the Viewer — the one under the playhead — and re-reads it every time it redraws, so the
  scope now **follows the picture while it plays** (K-130): each time you press play, Lumit
  keeps a little run of frames ready in memory (it warms them ahead of the playhead and while
  you sit paused), and the scope traces whichever one is on screen. If a frame hasn't been
  kept in memory yet — Lumit skips saving some frames during playback to stay fast — the scope
  simply holds the last frame it had rather than going blank, and snaps back to live the
  instant the current frame is ready. The one honest limit that remains (from K-096): tracing
  *every* frame no matter what, even on a brand-new composition nothing has warmed yet, needs
  the graphics card to do the counting, which is still a later addition. The scope's
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
  one, with a plain placeholder shown until a frame is to hand (UI-4, K-157). Drag footage
  onto the Timeline or Viewer to make a layer; with no comp open yet, the composition dialogue
  appears already filled in from that footage. Solids are proper assets now — one "White solid"
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
  switching preview and export over. Until then the shipped renderer in `lumit-ui` keeps
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
- **The stress project and speed benchmarks (`lumit-project::fixtures`, docs 13)** — the
  promise that Lumit stays responsive on huge projects needs something huge to test against.
  There's now a builder that makes a deliberately enormous project on demand — hundreds of
  compositions, thousands of layers, a quarter of a million keyframes — always *identical*
  down to the last byte, so a speed measurement means the same thing every time. Alongside it,
  a set of **benchmarks** time the everyday operations on that project (open it, save it, make
  one edit, undo). They run when a developer asks (`cargo bench`), and they'll later become
  pass/fail speed budgets in the automated checks.
- **Remappable keyboard shortcuts (`lumit-keymap`)** — the rules behind "every shortcut can be
  changed" live in their own small, self-contained piece with no screen or window in sight, so
  they can be proven correct on their own. A **chord** is a key plus its held modifiers
  (`Shift+F3`, `Ctrl/Cmd+D`); a **context** is where you are (the whole app, the timeline, the
  viewer…); a **binding** ties a chord in a context to an action. The interesting part is
  spotting **clashes** — the same chord that would trigger two different things at once — with
  the twist that an app-wide (Global) shortcut is live everywhere, so it clashes with a
  same-chord shortcut in *any* panel, while two different panels may reuse a key harmlessly.
  The default set (the whole documented table) and an "After Effects" preset both ship
  clash-free, the map can be saved to a shareable file, and Ctrl/Cmd is stored as one
  neutral "primary" key so a keymap works on both Windows and Mac. Still to come: wiring the
  live key presses and the Settings → Keymap screen to this core.

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
find it.

### Not building it at all: the Flatpak

If you just want to *run* Lumit on Linux, there is now a **Flatpak** — the one artifact
that sidesteps the whole FFmpeg-version problem. A Flatpak is an application packaged
together with the exact libraries it was built against, run in a light sandbox. Because
Lumit's bundle carries its own FFmpeg 7.1, it does not care what the distribution ships:
the same file installs on Ubuntu, Fedora, Arch or anything else.

Every CI run builds one and attaches it to the run as `lumit-x86_64.flatpak` (about 15 MB —
the app plus its own FFmpeg). Download it, then:

```
flatpak install --user lumit-x86_64.flatpak
flatpak run io.github.luminalmvm.Lumit
```

The recipe lives in `packaging/flatpak/`. Two parts of it are worth understanding, because
they look strange otherwise. First, the manifest **builds FFmpeg 7.1 itself** rather than
using the one in the Flatpak runtime — the runtime's is 6.x, the same version problem as
above, just moved indoors. Second, a Flatpak build has **no network access** on purpose (so
a build is reproducible and can't fetch surprises), which means every Rust crate Lumit
depends on has to be listed in advance; CI generates that list mechanically from
`Cargo.lock` before building, which is why you won't find it committed.

The sandbox is granted the GPU (the whole compositor is GPU work), audio out, and access to
your files — a video editor has to read footage from wherever you keep it, external drives
included.

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
a machine with no graphics card in it. And a sixth job builds the Flatpak, which is how we
know the packaging works and not just the code.
