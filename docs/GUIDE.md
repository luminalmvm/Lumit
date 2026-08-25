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
| `lumit-import` | After Effects import | Turning an AE project into a Lumit one, and saying honestly what changed |
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
  There are roughly as many as your CPU has cores. Each open project also has one
  **render worker** of its own, which owns a whole GPU connection — so a project that
  stops being shown is **closed**, not abandoned: closing it tells its worker to stop and
  give that connection back. Nothing enforced this for a long time, and every project a
  session ever made quietly kept a live worker; the test suite (one project per test)
  piled up enough of them to run the CI machine out of memory, which is how it was caught.
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
  effect is doing. For now it follows the footage's own motion only (not, yet, motion you add
  with keyframes) and works on footage layers, the same starting scope Echo has.

  **What the rebuild changed, and the two problems it fixes (K-392).** The description above
  is how the effect started, and it has two faults you can see once you know to look for them.

  The first is that *smearing each pixel along its own arrow* can never let a moving thing
  spill onto what it passes. A car crossing a still street gets a blurred car and a perfectly
  sharp street right up to its edge, which is not what a camera does — a real shutter has the
  car's light land on the street's pixels too. The reason the old way cannot do it is that it
  asks the wrong question: it asks each pixel "where am *I* going", when the question that
  produces a smear is "who might have flown *through here*". So the effect now also looks at
  what its **neighbourhood** is doing. The frame is divided into 16-pixel tiles, each tile
  notes the strongest motion inside it, and every pixel gathers along that direction as well
  as its own — keeping a sample only in proportion to whether the thing it found there was
  moving fast enough to have reached it. This is a standard technique with a slightly
  backwards name, *scatter as gather*: the honest way to smear paint outwards is to have every
  destination ask who could have thrown at it, because a graphics card can do that in
  parallel and cannot do the throwing.

  The second is the confidence taper from FX-19. Multiplying the streak by trust sounds right,
  but it means the *least* trustworthy pixels end up with **no blur at all** — so a patch the
  flow could not read comes out pin-sharp in the middle of a smeared frame, which is more
  obvious and worse-looking than a blur pointing slightly the wrong way. Confidence now
  *steers* instead of shortening: a pixel that trusts its own arrow uses it, and one that does
  not **borrows its neighbourhood's motion** at about 60% length. It is the honest answer to
  "I do not know how this moved, but I know everything around it is moving like *that*". The
  only place left that gets no blur is a patch where the neighbourhood itself is still — which
  is what you actually want. (A still frame and a shutter of zero still leave the picture
  exactly untouched.)

  One trap worth recording, because it was caught on real footage rather than by reading the
  code. "The strongest motion in the tile" and "what my neighbourhood is doing" sound like the
  same number, and they are not. The strongest is by definition the most *unusual* arrow out of
  the couple of hundred in a tile — and it gets picked exactly where the measurement is least
  reliable, so two tiles side by side can pick two unrelated wild arrows. Using it for the
  borrowing produced blur in **rectangular patches at different angles** across a cartoon's
  characters. The fix is to blend smoothly between tiles when borrowing: the borrowed direction
  becomes continuous so no tile edge can show, and — the nice part — tiles that *disagree*
  average each other out toward nothing, so where there is no consensus to borrow the blur
  quietly backs off instead of inventing one.

  Two more things came with the rebuild. **Samples** is now a *ceiling* rather than a count:
  the effect works out how many steps a streak actually needs (about one every two pixels) and
  spends no more, so a barely-moving pixel does not pay for 32 samples landing on top of each
  other. And a **Quality** choice — *Normal* or *High* — is the only method choice there is,
  deliberately: there is one blur that adapts internally, never a menu of algorithms to pick
  between. High halves the spacing between samples and re-reads the motion partway along each
  streak, so a streak can **bend** — which is what a spinning or swinging object's smear really
  does. The View picker gains *Dominant motion*, which paints the borrowed direction; flipping
  between it and *Confidence* shows you exactly where the effect stopped trusting a pixel and
  started trusting its surroundings.

  **One thing it gets wrong, which is worth knowing rather than hiding.** Deciding whether the
  fast thing is in *front* of you or *behind* you needs to know how far away things are, and a
  video has no such information. So the weighting is even-handed, and a **small** still object
  completely ringed by fast motion — a logo burnt into the corner of a cartoon, say — picks up
  some of its surroundings' smear. Large still areas are fine: a static desktop around a
  fast-moving game window measured 0.014 out of 255 of change, against 3.99 inside the window.
  Fixing it properly needs a depth input, not a cleverer constant, so it is recorded rather
  than tuned around.
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
  above.

  **Two effects, two sets of arrows (K-544).** There used to be a wrinkle here: a layer could
  carry only one motion-arrow map per frame, so a layer with both Fast motion blur and
  Datamosh on it served whichever came first in the stack and the other quietly sat out. That
  was never a shortage that could be shared away, because the two are not asking the same
  question. Fast motion blur asks "where is each pixel going *next* frame", so it needs the
  arrows measured forward. Datamosh asks "where did each pixel come *from* last frame", so it
  needs them measured backward. Different pairs of frames, different answers. Measuring is the
  expensive part, so the fix is not to measure more than necessary but to measure exactly what
  was asked for: the layer now works out which arrow maps its effects want — one, both, or
  none — and makes each one, and every effect is handed the map it asked for rather than
  whichever one happened to be lying about. A layer with only one of the two effects costs
  precisely what it always did.

  **Arrows for a picture that was never a video file (K-565).** Both these effects are
  usually described as going on an adjustment layer over the whole montage, and for a long
  time that was the one place they did nothing whatsoever. The reason is worth understanding,
  because it explains the fix. Measuring motion means comparing two pictures a frame apart,
  and the only part of Lumit that had two such pictures was the bit that opens video files
  and decodes frames out of them. An adjustment layer has no video file: its "picture" is
  simply everything painted beneath it, which exists as an image on the graphics card and
  nowhere else. A precomp layer has the same problem — its picture is a whole other
  composition, rendered on demand.

  What they *do* have is a recipe. Nothing stops Lumit following that recipe a second time
  with the clock moved on by one frame — it is exactly what Posterize time and the
  whole-scene Motion blur already do — and once it has, there are two pictures a frame apart
  again, and the motion arrows can be measured between them. That is the whole change: build
  the scene twice, once for now and once for the neighbouring moment, and compare.

  The comparison happens on the graphics card rather than by fetching the two images back
  into ordinary memory. That distinction sounds like plumbing and is actually the difference
  between "free" and "slow": copying two full-size images back off the card takes several
  times longer than measuring the motion does. So the motion engine learned to read a picture
  where it already sits, converting it to the plain grey version it works from with one small
  program that runs on the card itself.

  Two honest limits come with it. The second render is real work, so it only ever happens for
  a layer that actually carries one of these two effects — everything else builds nothing.
  And rebuilding the scene at another moment does *not* re-open the video files underneath;
  they hand over the frame they already decoded. So an adjustment layer measures the motion
  of things Lumit itself is animating — moving layers, animated effects, camera moves,
  precomps with animation inside — and not the motion happening inside footage that is simply
  playing. For that, the effect goes on the footage layer, where the decoder was measuring
  all along.
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
  the file (the house rule every effect follows). The depth pass should share your footage's
  framing, since it is stretched to fit; it does *not* need to be a visible layer, and
  usually should not be — a depth pass is something the effect reads, not something you want
  composited over your shot, so switching its eye off is the normal way to use one. (The
  part of the renderer that decodes footage walks matte and layer-input references whether
  or not the layer they name is visible, which is what makes that work.) The blur disc itself
  is the foundation kernel below, unchanged and still proven against its plain-Rust twin.
- **Depth of field grows three lens controls.** Three tick-and-slide additions, all borrowed
  from the reference plugins. **Invert** — the tickbox on Depth of field's Matte row — flips
  the depth map's reading
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
- **What an iris is, and why a blur is not a lens.** Blur a picture and bright points smear
  into soft grey nothing. Defocus a picture *through a lens* and every bright point becomes a
  little disc — and that disc is a **picture of the hole the light came through**. That hole
  is the iris: a ring of overlapping metal blades, so it is really a polygon, which is why
  out-of-focus street lights are hexagons on some lenses and circles on others. Depth of
  field's **Iris** twirl is that hole. **Blades** is how many, **Roundness** is how bowed they
  are (1 is a perfect circle, 0 a straight-edged polygon, and *below* zero the edges cave
  inward and you get a star), **Rotation** turns it (a number with a dial beside it — drag
  either), and **Aspect ratio** squashes it on one axis the way an anamorphic scope lens
  does. **Rim brightness** is the odd one worth knowing: a real lens does not throw a *flat*
  disc — some ring the edge bright (the "soap bubble" look), some pool the light in the
  middle (creamy and smooth). That is an optical defect called spherical aberration, and this
  is the dial for it.

  The other half is subtler and matters more. Averaging is what makes a blur, and averaging is
  also what *destroys* a highlight: one very bright pixel among a hundred dark ones comes out
  barely brighter than the dark ones. A real lens does not average — it spreads that bright
  point's energy over the whole disc, and the disc stays bright. The **Highlights** twirl fakes
  that honestly: **Highlight threshold** says which part of a pixel counts as a highlight
  (everything above scene white, by default), and **Highlight exposure** says how hard that
  part is allowed to survive the average. Turn exposure up and the bright points stop
  dissolving and start blooming into balls. That one control is the difference between "the
  background is blurry" and "the background is *bokeh*".

  Both are **off when you drop the effect on**, which is not shyness — it is a promise. Every
  project already made with Depth of field has to render the same pixels it always did, and
  the way the code keeps that promise is worth knowing because it looks like paranoia: at
  their neutral settings the kernel does not multiply by one, it takes a *different route
  through the code entirely*. The reason is that computer arithmetic is not school arithmetic.
  Adding a hundred numbers each multiplied by 1.0, then dividing by a hundred 1.0s, does not
  always give the same last digit as adding a hundred numbers and dividing by a hundred —
  each multiply rounds, and rounding accumulates. So "neutral" is a fork in the road, not a
  factor of one, and there is a test that renders the whole frame both ways and demands the
  results match to the last bit.

- **Picking a point by clicking it.** Depth of field's **Focus point** is a small thing that
  removes a genuinely annoying job. Focusing used to mean: switch the view to Depth map, look
  at the grey value of the thing you want sharp, guess it is about 0.34, type 0.34, look
  again, try 0.31. Now you arm the little crosshair beside the row and click the thing in the
  Viewer; the effect reads the depth under your click and focuses there. Underneath, a "point"
  in Lumit is not a special kind of value — it is just two ordinary number parameters named
  `something_x` and `something_y`, and the panel notices the pair and draws them as one row
  with a crosshair. That is why adding a point to an effect costs nothing: you name two
  numbers correctly and the row appears.

- **Greyed-out rows.** Some controls stop meaning anything once another control is set a
  certain way. Tick "Use focus point" and the Focus distance *number* no longer decides
  anything — the point does. Leaving the number live would invite you to drag something that
  does nothing, so the row goes dim and stops taking the mouse. The effect declares these
  rules in its own description ("this row is editable only while that switch is off"), the
  panel reads them and draws accordingly, and — importantly — the *renderer never consults
  them*. It works out for itself which control is in charge. That separation is deliberate:
  the worst case is a forgotten rule leaving a live control that does nothing, which is a
  panel bug you can see, rather than the panel and the renderer disagreeing about what your
  picture should look like.

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
  reaching its full length (set by Amount, in the same pixels-at-composition-size units
  Radius and Length already use) at the frame's farthest corner. The clever bit is *how* those two
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
  rule). Amplitude sets how far it roams (in pixels at composition size), Frequency how
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
  That star is not the same shape everywhere in the frame. Look through a lens from an
  angle, rather than straight down the barrel, and the round hole you see is not round —
  the metal rims at the front and the back of the barrel overlap it from opposite sides
  and pinch it into a pointed oval, the shape photographers call a **cat's eye** (it is
  the same effect that squashes out-of-focus background highlights into lemon slices near
  the edges of a picture). Light bends around *that* shape, not the iris, so a real
  starburst squashes and leans as the light moves out towards a corner. Lumit works out
  the shape of the hole at eight angles across the frame when you pick a lens, and each
  light in the shot reads the two that bracket where it actually sits, turned to point
  the right way. A light dead centre gets exactly the star it always got.
  The ghosts have an edge problem of their own, and it is the opposite kind of
  physics. If you cut a hole in card and shine a torch through it, the patch of
  light on the wall does not have a clean border: right at the rim there is a
  set of fine bright and dark rings, and the brightest of them is brighter than
  the middle of the patch. That is light bending round the edge of the hole,
  and a real ghost — which is just an out-of-focus picture of the iris — has
  exactly the same shimmer at its rim. Almost every flare in software draws
  ghosts with a stencil edge instead, which is a large part of why they read as
  fake. Lumit works the rings out properly, and how coarse they are is not a
  stylistic choice: it follows from how big that particular ghost is and how open
  the iris is, by one line of arithmetic.
  The first attempt at this got the *scale* badly wrong, and it is worth
  recording why, because the mistake was visible from across the room. The rings
  were produced by running the same Fourier maths the starburst uses, at a ladder
  of six settings. That machinery has a hard ceiling — reaching finer rings needs
  a transform sixteen times bigger — and real ghost rings are hundreds of times
  finer than the ceiling. So every ghost was drawn with rings far too coarse, and
  rings coarse enough are not a rim effect at all: they are broad bright and dark
  bands across the *whole* ghost. With big ghosts filling the frame, the picture
  picked up a faint concentric pattern over the entire shot. The fix was to stop
  computing the whole pattern and use the textbook formula for what happens at
  the *edge* alone, which is exact at the scales real ghosts actually have, costs
  nothing, and cannot touch the middle of a ghost at all — the interior is flat
  by construction now, not by luck.
  **The weave that was actually there.** Underneath all of the above sat a
  plainer fault, and it is worth explaining because it looked like optics and
  was not. The effect fires a grid of rays through the lens and each one lands
  somewhere on the picture, where it deposits its light spread over a small
  patch — brightest at the middle, fading to nothing at the patch's edge, so
  that neighbouring patches blend into a smooth sheet. Except they didn't
  blend: each patch faded to zero exactly where the next one *started*, so
  every patch was an island with a hairline of darkness around it. Add up a
  grid of those and you get a woven mesh of dark lines printed over the whole
  picture, a bright cross where the grid's own axes run through each ghost, and
  little stair-steps along every rim. The total amount of light was exactly
  right, which is why every test the effect had went on passing — they all
  measured how much light there was, and none measured whether it was smooth.
  The fix is that each patch now reaches further, out past where its neighbour
  sits, so they overlap and add up to an even sheet. That took two goes. The
  first used a simple triangular fade, which adds up to exactly the right total
  — but leaves a faint crease where one patch hands over to the next, and the
  eye is remarkably good at finding creases; it is the same reason a
  smooth-shaded model can still show its polygon edges. The second uses a
  gently curved fade that hands over without a crease at all. Measured on a
  real frame, the leftover texture went from about 16% of the local brightness,
  to 2.4%, to 1.9%. It costs nine times as much drawing per ray, which is the
  honest price of not printing the ray grid onto the picture.

  A caution worth passing on: the first fix had a test, and the test passed. It
  laid out an even grid of identical rays and checked the result came out flat
  — which is exactly the case the simple fade handles perfectly. Real ghosts
  are stretched and uneven, and that is where it fell down. The test now
  measures a real flare instead.

  **Adding up in the right precision.** One more fault sat underneath, and it
  is a nice illustration of how a thing can be wrong while every test says it
  is right. The light from each ray is *added* into the picture, and a bright
  pixel might have a few thousand rays landing on it. That adding-up was done
  in a 16-bit number format — fine for storing a colour, but when you add a
  tiny amount to a large running total in 16 bits, the tiny amount can be too
  small to change the total at all, and simply vanishes. It vanishes more the
  brighter the pixel already is, so the effect was quietly darkening its own
  highlights by about four percent in the middle of the frame. The sum is now
  kept off to one side in a whole-numbers format — counting in steps of about
  a millionth — and written into the picture once at the end. One rounding
  instead of thousands. Whole numbers matter for a second reason: the rays are
  added up by thousands of parallel threads in whatever order they finish, and
  ordinary decimals give slightly different totals depending on that order,
  which would mean the same project rendering two different pictures. Whole
  numbers add up the same however they are shuffled. The first attempt used
  decimals and did exactly that; the tests caught it the same day.

  **Big soft patches are painted coarse, then smoothed.** A ray that lands on a
  big defocused ghost spreads its light over a big patch — sometimes hundreds of
  pixels across — and painting every one of those pixels for every one of
  hundreds of thousands of rays is where the effect's time actually went: the
  cost of a ghost was its whole area, over and over, once per ray. So the
  canvas the rays paint on is now a stack: the full-resolution picture on top,
  and under it a half-size copy, a quarter-size one, and so on. A small sharp
  patch is painted on the top layer exactly as before; a patch too big to
  afford is painted on whichever smaller layer brings it down to a sensible
  number of pixels, and at the end the layers are enlarged and added together.
  Enlarging a coarse layer smooths it — which is fine, because the only
  patches sent down there were big soft ones whose own softness is far
  coarser than the smoothing. A patch is never blurred by more than about a
  twenty-fourth of its own size, which the eye cannot find on a defocused
  ghost. The payoff on a real frame was better than tenfold, and it is the
  reason turning a flare dial no longer saturates the graphics card.

  **Every element gets its own coating.** Look at a photograph of a real flare
  and the ghosts are not one colour: there is a blue one next to a purple one
  next to an amber one. That is the lens, not an effect. A coating works by
  cancelling reflections at some wavelength, so what the surface still reflects
  is the *opposite* colour — cancel the green and you see magenta come back — and
  a lens maker does not cut every element the same. Lumit therefore offers a
  coating choice per piece of glass in the lens, and it shows exactly as many
  rows as that lens has pieces: four for a Tessar, eighteen for a 70-200 zoom.
  Each choice is a real coating design of its kind, and the colours in the menu
  are not names somebody liked — they were measured off the maths, across the
  visible spectrum, and only designs that come out both distinctly coloured and
  dimmer than bare glass (as a real coating must be) made it in. Leaving a row
  at "As the lens file" changes nothing, which is where they all start.
  One honest consequence: a big soft ghost's rings are so fine that the effect
  cannot draw them without them turning into a moiré pattern, so it doesn't try —
  it draws their average, which is a plain soft edge. Tight bright ghosts, which
  is where a photograph actually shows a ringed rim, keep theirs. That is the
  right way round, and the previous behaviour was precisely the wrong way round.
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
  outlines got the standard cure, multisampling: four points checked per pixel
  instead of one at the edges of shapes, which is how every game smooths its
  edges. (Lumit later had to stop asking the *card* to do that counting and do
  it in its own arithmetic instead — same four points, same result, but
  repeatable. See "the flare that would not draw the same picture twice"
  below.) And the triangular notches in the bright rims turned out to
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

  **Driving *any* effect with a matte (K-395).** Every effect in Lumit now has
  a **Matte** row — a layer picker with an **Invert** tickbox beside it — and
  what it does is the obvious thing: the matte's brightness says *how much of
  the effect* each pixel gets. White, the effect applies in full; black, the
  pixel is left exactly as it arrived; grey, part way. Point a blur's Matte at
  a soft radial gradient and you have a blur that fades off towards the middle
  of the frame, with no second layer, no pre-comp and no track matte.

  It is worth being clear about how this differs from **masking** an effect,
  because they sound the same and are not. A mask hides what the effect *did*
  — the effect ran everywhere and you painted over the parts you did not want.
  A matte feeds the effect *itself*. For a plain colour correction the two look
  identical, and for anything with a radius or a direction they do not: a blur
  masked to a circle still smears pixels from outside that circle inwards,
  where a blur *driven* by a matte simply blurs less there.

  A few effects already had the idea under their own names — depth of field's
  depth pass, the lens flare's matte source — and they keep their own, deeper
  meanings (a depth pass decides *focus*, not strength). They now wear the same
  row and the same two words, so there is one thing to learn rather than three.
  Everything you already have is untouched: the row starts empty, and an empty
  row is not "a dissolve by one" — nothing extra runs at all, and the picture is
  the same pixels it was before the row existed.

  **Some effects do something cleverer with the matte than fading (K-395).**
  Fading is a good default and a poor answer for anything with a *size*. Four
  effects therefore take the matte into their own maths — depth of field and the
  lens flare, which always did, and two more worth knowing about because the
  result is a picture you could not get any other way.

  The **gaussian blur** treats the matte as a per-pixel *radius*. Where the
  matte is white you get the full Radius; where it is mid-grey you get half of
  it; where it is black, nothing. That is not the same as fading a finished
  blur, and the difference is easy to see once you know to look: fading leaves
  every pixel smeared from the full radius away and then dials the result back,
  so it looks like a soft veil laid over a picture that is still sharp
  underneath. Scaling the radius makes the blur genuinely *narrower* — the way a
  lens does when it comes into focus. Paint a gradient down one side of a frame
  and you have a lens racking focus across the shot, with no keyframes.

  **Glow** treats the matte as a gate on which pixels are allowed to *start* a
  glow. The bright parts of your matte bloom; the dark parts do not. But — and
  this is the whole point — the light that escapes from the bright parts still
  spreads outward, across the dark parts of the matte and past its edge, exactly
  as light does in a room. If glow merely faded to the matte, a glow "on the
  neon sign only" would stop dead at the sign's outline and refuse to light the
  wall beside it, which never looks right. Gating the seed lets the sign light
  the wall.

  **Which channel of the matte, and how the effect blends back in (K-425).**
  Two small controls on rows every effect already has. Beside the Matte picker
  and its Invert tickbox there is now a **Channel** choice: by default an
  effect is driven by the matte's brightness, but a depth pass or a packed
  render might keep the useful picture in its alpha or in one colour channel,
  and the choice says which. And beside every **Mix** slider there is a
  **Blend** choice — the same list a layer's Mode dropdown offers — saying how
  the effect's result combines with the untouched picture: Normal simply
  replaces it, as it always has; Add lays the result on as light; Multiply
  darkens; and so on.

  Both are done in one place rather than taught to forty kernels, which is the
  part worth understanding. The engine does not tell a blur "read the red
  channel"; it rewrites the matte *once* — before any effect sees it — into a
  grey picture whose brightness is the chosen channel, flipped if Invert is on.
  Every effect then reads brightness as it always did and gets the right
  answer, and Invert happens exactly once (three effects used to flip the matte
  themselves and no longer do, so it cannot be flipped twice). The Blend works
  the same way. An effect's Mix lives inside the effect, so blending an
  already-faded result would fade it twice; instead, when the Blend is anything
  but Normal, the engine runs the effect at full strength, blends that onto the
  untouched picture, and then applies the Mix itself, once. When you leave
  both at their defaults nothing extra runs at all, and a project saved before
  the two controls existed renders the same bytes it did.

  **The matte turns the effect's own knob (K-426).** The rule the owner set for
  mattes is short: the matte multiplies the *amount* of the effect, per pixel.
  It is not a mask laid over the result. Point a Gamma's Matte at a soft
  gradient and where the gradient is mid-grey the Gamma is genuinely half as
  strong — the curve the pixel goes through is gentler — rather than the full
  curve applied and then half hidden. For a blur that means a shorter streak;
  for Exposure, fewer stops; for Hue shift, a smaller turn of the colour wheel;
  for Posterize, finer steps (a black matte means steps too fine to see, which
  is the same as leaving the picture alone). Every blur, sharpen and colour
  effect now works this way, and each one's Matte row says in words which
  control it scales. Three do not, for an honest reason. Contrast and Vignette
  are left alone because for them turning the knob down and fading the result
  are the *same* picture — the maths is a straight line — so there was nothing
  to change. Threshold is left alone because a cut is either made or not: the
  only way to get the colour picture back where the matte is black is to fade
  to it, which is what the Matte row already did. None of this changes a saved
  project: an effect with no matte bound runs the very same code it always did.

  **And for a distortion, the knob is a distance (K-427).** Every effect in the
  Distortion group moves pixels, and how far it moves them is its amount — so
  the matte multiplies *that*. Point a Twirl's Matte at a soft gradient and the
  twirl is genuinely gentler where the gradient is grey; a half-grey matte on a
  200° twirl draws exactly the picture 100° would. The same idea covers the
  whole group: a shorter shift for Offset, a shallower ripple, a narrower
  colour fringe, a weaker lens bend, a smaller pull toward the pinned corners.

  This is where a matte does something a fade truly cannot. Fade a shaken frame
  back by half and you get the shaken picture ghosted over the still one — two
  of every edge, a double exposure. Halve the shake's *amplitude* and you get
  one picture that has simply moved less. So a soft matte on a Shake turns a
  frame-wide shove into a **warp**: part of the picture jolts, part of it stays
  put, and it is still one picture. That is the whole point of the rule.

  One detail decides how it looks, and it is worth naming: **the matte is read
  where the pixel lands, not where it came from.** A distortion works backwards
  — for each pixel of the output it asks "which part of the input belongs
  here?" — so it could read the matte at either end. Lumit reads it at the
  output, always, which means the shape you paint in the matte is the shape you
  see in the finished frame. Paint a black circle and that circle of the
  picture is the one that stays still.

  Two effects in the group take a different road for an honest reason. Scanlines
  could not scale its Intensity, because fading scanlines and making them
  fainter are the same picture; so its matte spreads the **lines further apart**
  instead, until at black they are too far apart to see at all. Datamosh is left
  alone entirely, for Contrast's reason — its Intensity dial *is* a fade, so
  there was nothing to change. Tile, Mirror and Polar coordinates have no "how
  far" at all: they repeat, reflect and re-map, and each keeps the plain fade.

  **For the things that draw and the things that stylise, the knob is what it
  always was (K-428).** The same rule, applied to the last two groups. Where an
  effect draws something on top of the picture — a bolt of lightning, expanding
  radio rings, a scribble, a brush stroke, marching dashes, a cast shadow — the
  matte scales *the drawn thing's opacity*, so it fades along its own length and
  leaves the picture underneath alone. Where an effect has a size, the matte
  scales the size: Median's Radius, Roughen edges' Border, Emboss's and
  Texturize's Relief. Add grain's Intensity is scaled too, and because grain is
  laid on in perceptual light and squared back, half the Intensity really is a
  finer grain rather than a half-faded coarse one.

  Two of these are worth seeing to believe. A half-grey matte on a Median set to
  Radius 2 gives you *exactly* the picture Radius 1 gives — a genuinely smaller
  window, so you can despeckle a noisy sky and leave a face untouched with one
  painted matte. And a black matte on Emboss gives the flat grey sheet rather
  than the picture back, because Emboss at Relief 0 is a flat sheet with no light
  on it; turning the relief down is not the same as turning the effect off, and
  the matte turns the relief down.

  Four in these groups are left on the plain fade for the honest reason Contrast
  was: **Noise**, **Flash**, **Sprite flare** and **Light wrap** each add a plain
  amount of something to the picture, so turning that amount down and fading the
  result back are the same arithmetic and there was nothing to change. Fill,
  Gradient, Fractal noise, Beam, Mosaic and Find edges have no amount of their
  own at all — they replace the picture — so they keep the fade too. Glow keeps
  its cleverer seed gate and the Lens flare its source detection.

  **For time, the knob is a duration; for a transition, it is how far along it
  is (K-429).** The last two groups. In the Temporal group the amount an effect
  has is a length of time. **Echo** has a Decay — how fast the trail of ghosts
  fades — and the matte scales that, so the trail is genuinely shorter where the
  matte is dark rather than a long trail half hidden; a half-grey matte draws
  exactly the trail a half Decay would. Both motion blurs have a **Shutter
  angle**, which is how long the shutter was open, and the matte scales that:
  the smear is genuinely shorter, gathering from nearer along the movement,
  instead of a long smear laid over a sharp frame at half strength.

  The accumulation Motion blur is worth a paragraph on its own, because it is the
  only effect that claims its matte somewhere other than in a shader. That effect
  does not draw anything — it asks the engine to render the whole scene several
  times at moments spread across the open shutter, and then averages the results.
  So the matte is spent in the **averaging**: where it is dark, the average is
  taken over a shorter slice of those moments, closed in toward the frame the
  viewer actually asked for. Fully white is the ordinary average, which is what
  the effect has always drawn; fully black is the one render at the frame's own
  moment, which is no blur at all; grey is a genuinely shorter exposure. Closing
  in on *the frame's own moment* rather than on the middle of the list is the
  detail that makes black mean "not blurred here", and it is why the plan carries
  a number saying where in the open shutter that moment falls.

  In the Transition group the amount is **Completion** — how far the wipe has
  got. Scaling it per pixel turns any wipe into a **gradient wipe**: paint a soft
  ramp on the matte and the edge follows the ramp's shape instead of the straight
  line the effect would otherwise draw. The Card wipe asks the question per
  *pixel* rather than per card, so half a card can still be standing while the
  other half has flipped away. The Iris wipe has no Completion at all — for it,
  growing the hole *is* the transition — so the matte scales the hole's radius,
  which is the same sentence about the same thing.

  **A motion field somebody else already knew (K-429).** Fast motion blur
  normally works out the movement itself, by comparing this frame of the footage
  with the next. Game engines and 3D renderers do not have to guess: they already
  know exactly how far every pixel moved, and most of them can hand that over as
  a picture. Point Fast motion blur's new **Motion vectors** row at such a layer
  and it uses that instead. The picture is read the way everyone writes it — red
  is sideways movement, green is up and down, and a flat mid-grey means standing
  still — with a **Vector scale** slider saying how many pixels a full channel
  stands for, because different engines scale their vector passes differently.
  Two things come with it. The blur is exact rather than measured, so it does not
  wobble where the guessing is hard; and the effect works for the first time on a
  layer with no footage to measure at all — a solid, a shape, a nested
  composition.

  **The two effects that carry no Matte row at all (K-429).** Every effect has
  one, with two exceptions, and both are about keying. The **Matte key** is a
  greenscreen keyer: what a strength matte over a key would be is a garbage
  matte, which is a mask's job, so it has none. **Set matte** takes another
  layer's brightness and makes it this layer's transparency — and that layer is
  not "how much of the effect happens here", it *is* the effect. So Set matte's
  picker is now its own row rather than the shared Matte one; it looks and reads
  exactly the same, holds the same thing in the same place in a saved project,
  and simply stops pretending to be a control it never was.

  **Handing an effect a mask's shape, not its cut-out (K-408).** A matte is a
  picture, and a picture is the wrong thing for some effects. Think of a brush
  that travels along a line you have drawn, from 20 % of the way along to 80 %,
  or of dashes marching round the outline of a shape. Neither can be told from a
  cut-out: a cut-out says which pixels are in and which are out, and says
  nothing at all about which way is *along*. What those effects need is the
  curve itself — the actual points, in order.

  So an effect can now carry a row that names one of **this layer's masks**. You
  draw the mask on the layer exactly as you always do, and the effect walks it.
  The row lists your masks by their own names, with **First mask** at the top,
  which simply means "whichever mask is first" — that is the default, because
  you usually drop the effect on before you draw the shape, and it keeps working
  when you draw one later. A mask set to mode **None** is still offered: that
  mode means "this shape is drawn but gates nothing", which is exactly what you
  want when the shape is there for an effect rather than for cutting the layer
  out.

  Two mechanical points, because they explain what you will see. First, the
  effect gets the curve as a long chain of very short straight pieces — a curve
  a computer can walk has to be straightened first — and the fineness of that
  chain is fixed at half a pixel, close enough that you cannot see the corners
  and identical every time. It has to be fixed rather than adjustable, because
  the frame cache names each finished frame by everything that went into it, and
  a fineness that could change would let one name mean two different pictures.
  Second, if the row names nothing, or names a mask you have since deleted, or
  the layer has no masks at all, the effect simply passes the picture through.
  Nothing errors; there is just nothing to walk.

  The seam was built ahead of the effects that use it, because building it into
  one of them would have buried a general mechanism inside a single effect.
  **Three effects use it now**, and they are the three that stopped waiting for
  it: **Scribble**, **Stroke** and **Vegas** on its new Mask/Path source.

  **Scribble** shades the mask in the way a hand shades a shape with a pencil:
  parallel strokes at whatever angle you pick, spaced how you like, running a
  little past the edges the way a hand does, and wavering as they go. Two
  behaviours of it are worth knowing about. The strokes are one *continuous*
  line — the pen crosses the shape, hops down the edge, and comes back — which
  is why Start and End trim it the way a pen drawing it would, so keyframing End
  from 0 makes the shading draw itself on. And where the shape has a notch in
  it, the pen lifts rather than flying across the gap. The waver is what makes it
  look drawn rather than ruled, and Wiggle type says how it moves: **Static**
  holds one arrangement, **Jagged** snaps to a new one several times a second
  (the pencil-test look), **Wiggly** drifts.

  **Stroke** runs a round brush along the mask's *line*. Start and End are per
  cents of the way round it, so the same "keyframe End from 0" trick draws the
  line on — that is what this effect is mostly for. Spacing is how far apart the
  brush stamps are, as a per cent of the brush's own width: leave it low and you
  get a solid line, push it past about half and the stamps come apart into
  dots, which is a deliberate look. Paint style decides what the stroke lands
  on: over the picture, on nothing but itself, or — the interesting one —
  **Reveal original**, where the picture survives *only* where the brush went,
  so a stroke drawing itself on wipes a title into view.

  **Vegas** already ran its dashes along a contour it found in the picture; it
  can now run them round a mask instead. That half is better than the contour
  half in one specific way, and it is worth saying why. Along a contour, Vegas
  has no idea how far round it has gone, so it spaces the dashes by position on
  screen — which drifts out of step wherever the contour curves hard. A mask is
  a real curve, and the distance round it is measured on the way over, so on
  this half the dashes stay evenly spaced all the way round however tight the
  bend. That measured distance is what After Effects' own "Segments" count
  means, so an imported project converts exactly rather than approximately.

  Under all three is **one piece of drawing code, not three**. It is worth
  knowing because it explains why they feel consistent. A scribble, a brush
  trail and a ring of dashes differ entirely in *where the line goes* and hardly
  at all in *how it is drawn* — a soft-edged band of a given width along a list
  of short straight pieces, optionally broken into dashes. So the "where"
  happens first, on the processor, in three quite different pieces of code, and
  what reaches the graphics card is the same short description in every case. It
  is the same principle as the lightning bolt: work out anything that does not
  change from pixel to pixel *before* the graphics card sees it, and the part
  that runs three million times a frame stays small.

  There is a ceiling on that description — 512 straight pieces — because it
  travels in a fixed-size parcel. You will not meet it in normal use; if you do,
  nothing breaks and nothing disappears. The hatch opens its spacing slightly,
  the dots space out slightly, a very long path straightens slightly. The whole
  shape is always drawn; it just gets a little coarser rather than stopping half
  way, which is the failure you would actually notice.

  You never have to remember which is which: every effect's page in the manual
  spells out, under its parameter table, what that effect's matte does — the
  plain fade for most of them, in the same words each time, and the deeper
  meaning where there is one. Those sentences are written on the effect itself,
  in the engine, so a page cannot describe a matte the effect stopped honouring
  (see "The effect pages write themselves", below).
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

  **The flare that would not draw the same picture twice (K-353).** Lumit
  holds itself to a rule: the same composition, the same frame, the same
  settings must produce *the same pixels* — that is what lets a frame be
  cached, and what makes the preview a promise about the export. The lens
  flare had quietly stopped keeping it. Render the same flare twice and a few
  hundred pixels came back a hair different, in different places each time,
  and the test that says so had been failing for months without anyone knowing
  which part was to blame.

  The answer came from crossing things off. The ray tracing was innocent —
  ask it for the same rays twice and every one lands in exactly the same
  place. The blur was innocent; so was the starburst. It even happened with a
  single ghost of a single colour, so it was not a matter of adding up too
  many things. What was left was the *drawing* — and specifically the four-
  points-per-pixel smoothing described further up. Asking the graphics card to
  keep four half-precision numbers per pixel and add light into them turns out
  not to be repeatable on this hardware: the card is free to do that
  arithmetic in a slightly different order each time, and half-precision
  numbers are small enough to notice.

  So Lumit stopped asking. The smoothing is still there and still uses the
  same four points — the flare does the counting itself, in its own
  arithmetic, which is repeatable by construction. It also happens to be the
  *exact* calculation the slow reference implementation already did, so the
  two now agree because they are the same sum rather than because one
  resembles the other. Two details had to come with it: shapes are grown by a
  pixel before drawing (otherwise a sliver of light that passes between pixel
  centres is never asked about at all, and a third of the flare's brightness
  went missing), and that growing pushes the *edges* outward rather than the
  corners — a folded sliver's corners lie almost in a line, so pushing them
  apart lengthens it without ever making it thicker. And the biggest single
  piece of memory the effect owned, about 66 MB on a large frame, was the
  card's four-points-per-pixel canvas; it is gone.

  **Why flares used to jump, and why lights now have a size (K-355).** Point
  the flare at footage — at a street lamp, a window, a practical in shot — and
  it used to twitch. The reason was that Lumit asked a very narrow question of
  the picture: *which single pixel here is brightest?* That pixel became the
  light. But inside a real lamp, the brightest pixel is a lottery: sensor
  noise and the odd sparkle move it from one frame to the next, so the light
  hopped about inside something that had not moved, and the whole flare
  hopped with it.

  Lumit now asks a better question: *where is the light, on average?* Every
  lit pixel is weighed, and the light is placed at the balance point of all of
  them — its centre of brightness. One pixel going forty times too bright
  moves that balance point by less than a pixel, so the jitter is simply gone.
  The same change gives a light its *colour* from the average of what is lit
  rather than from its hottest speck, so a sparkle can no longer tint a flare.

  And once you are weighing every lit pixel, you also know **how big the light
  is** — and that turns out to matter enormously. A real flare from a strip
  light does not look like a flare from a star: its ghosts are little images
  of the strip, stretched the way the strip is. So a big light has to flare
  as a shape, not as a dot.

  In Manual mode this is two dials, **Source width** and **Source height**,
  measured in pixels from the centre outward. Leave them at zero and you have
  the pinpoint star the effect always drew. When the flare is reading a layer
  instead, it measures the size itself from the picture.

  **Why a softbox flares as one smooth shape (K-367).** The obvious way to
  flare a big light is to treat it as a handful of small ones side by side:
  work out the flare for, say, twenty-five points spread across the softbox,
  each carrying its share of the brightness, and add them up. That is what
  Lumit did at first, and it has a giveaway that the owner spotted straight
  away — you can *see the twenty-five*. Wherever a ghost is smaller than the
  gap between those points, you get twenty-five little irises laid out in a
  grid instead of one soft rectangle. Using more points only shrinks the gap;
  it never closes it. It also costs twenty-five times the work.

  What Lumit does now is quietly better and, oddly, cheaper. The flare is
  already worked out by firing thousands of rays through the lens — a fine
  spray covering the front of the glass. Each of those rays is now told to
  come from *its own point* of the light: one ray from the top-left corner of
  the softbox, the next from somewhere near the middle, the next from the
  right-hand edge, and so on, spread evenly over the whole panel by a fixed,
  repeatable pattern. No two rays start from the same place, so there is
  nothing for copies to form out of — and because each ray already spreads
  its light over the little patch its neighbours bracket, and its neighbours
  now come from quite different parts of the softbox, that patch grows to
  cover exactly the gap a copy would have sat in. The result is one smooth
  bar for a tube, one smooth rectangle for a window, one smooth disc for a
  bulb.

  Two nice consequences. A big light now costs the same as a small one —
  nothing was added to the ray count, the size was folded into rays that were
  being fired anyway. And a light of zero size offsets every ray by exactly
  nothing, so every flare you have already built renders the same as before,
  right down to the last bit.

  The **starburst** — the spiky glint on the light itself — has to be handled
  separately, because it is not traced at all: it is a picture worked out once
  and stamped down. The saving grace is that its shape does not change as the
  light moves, only where it sits, so a starburst from a big soft source is
  just the small one smeared across that source. Lumit stamps it in a
  three-by-three grid over the panel, each at a third of a third of the
  brightness, which comes to the same thing. A softbox gets a broad soft
  glare where a bare bulb gets a pinpoint star — and a point of light still
  gets exactly one stamp, unchanged.

  **Why ghosts change colour as a light crosses the frame (K-356).** Every
  glass surface in a lens is coated to stop it reflecting — that is what the
  faint purple or green sheen on a lens front element is. Those coatings are
  the reason lens flare is coloured at all: what little light *does* bounce
  back off each surface has been filtered by them, so a ghost is tinted by
  the coating that made it.

  A coating works by interference: light bouncing off the front of a
  microscopically thin film and light bouncing off its back arrive slightly
  out of step, and cancel. How far out of step depends on the wavelength —
  which is why a coating kills some colours better than others — *and* on the
  angle the light arrives at, because a slanted path through the film is a
  longer path. So as a light moves toward the edge of frame and its rays hit
  the glass more steeply, the colour the coating suppresses shifts. That is
  the real-world effect of a flare going magenta on one side of the frame and
  green on the other.

  Lumit used to model this with a single film, which can only ever cancel one
  colour, so ghosts could only be tinted one way. It now solves real
  multi-layer stacks properly — the standard optics calculation, layer by
  layer, for both wavelength and angle. Modern lenses use several layers
  precisely to get two or more cancelled colours, which is where their
  characteristic look comes from, and that is now what Lumit draws.

  One honest limit: a lens prescription tells you *how many* layers a surface
  has, never the recipe — those are trade secrets, and the research is
  unanimous that real coatings can only be measured, not guessed. So Lumit
  uses the textbook design for each layer count. The behaviour is right; the
  exact tint of a specific real lens would need that lens photographed and
  fitted.

  **Tracing three colours and measuring twenty-four (K-364).** The catch with
  the coatings above is that their effect swings up and down several times
  across the visible spectrum — a stack cancels one colour, passes the next,
  cancels another. Lumit was tracing three wavelengths and reading the coating
  at each, which is three readings of a curve with five wiggles in it: the
  answer depended on where the three landed, not on the shape of the curve.

  The fix separates two things that used to travel together. *Where* a ray
  goes barely changes with colour — glass bends red and blue by very slightly
  different amounts, and that smooth difference is all the geometry cares
  about — so the path is still traced three times. *How much energy* the ray
  keeps is where the wiggles live, so each traced ray now carries eight
  separate energy readings, spread across its share of the spectrum, and each
  one reads the coating at its own wavelength. Even the lowest quality setting
  therefore samples colour twenty-four times where it sampled three, for
  almost no extra tracing.

  The coating maths itself no longer runs per ray at all. Once per lens, the
  reflectance of every surface is worked out ahead of time onto a small grid —
  every 5 nm across the visible, at sixteen angles — and the ray simply looks
  its answer up. Both halves of Lumit do this identically, the one on the
  processor and the one on the graphics card, so they still agree ray for ray;
  the card's copy got faster in the bargain, because a table lookup is cheaper
  than the layer-by-layer calculation it replaced.

  **Every ray now paints its own little patch (K-366).** Until this change the
  flare was drawn as a net: neighbouring rays were joined into four-cornered
  panels and the panels were painted. That works beautifully where the light
  spreads out smoothly, and it is wrong exactly where a flare is at its most
  beautiful — at a *fold*, the bright rim or arc where the lens crushes a whole
  band of rays onto one line. At a fold the light doubles back on itself, so a
  panel joining four rays across it stretches over two places at once and comes
  out as a hair-thin sliver. Every rescue the flare has ever grown — puffing up
  panels too small to see, throwing away the slivers, reining in stray corners,
  growing every shape by a pixel before drawing it — existed to survive that one
  bad join, and each one moved the problem somewhere else: notched rims one
  release, faint lines across the picture another, blocky facets a third.

  So the rays are no longer joined at all. Each ray asks its four neighbours
  where *they* landed, works out how big a patch of the picture its own share of
  the light covers, and dabs that share over the patch — brightest in the middle,
  fading to nothing at the edge, like a soft round brush stroke of exactly the
  right size. The total light in a dab is always the light the ray carried, so
  nothing is gained or lost however the patch is stretched. And a fold is now
  simply many dabs landing on top of one another, which is the honest answer:
  that *is* what a fold is. All the rescue machinery is deleted rather than
  patched again, including the growing-by-a-pixel from the fix above — a dab
  fades out on its own, so there is no hard edge left to smooth.

  Two knobs stay. A dab is never allowed to be thinner than about three quarters
  of a pixel, so a caustic line comes out as a line instead of a dotted row of
  misses; and there is still a ceiling on how bright a fold may get (about 333
  times), because a fold's brightness genuinely runs away in the maths while a
  real photograph's does not. Ghost bodies look as they did; rims and arcs are
  cleaner. Because the picture does move, the effect's version number steps up,
  which is how Lumit records that an old project renders a little differently.

  **The ghosts of the ghosts (K-368).** A ghost is light that bounced off two
  glass surfaces on its way through the lens. But light that has bounced twice
  can bounce twice more and still land on the sensor, and that is what produces
  the *chains* of doubled blobs old lenses are famous for. Lumit used to ignore
  those four-bounce paths, on the reasonable grounds that each one carries about
  a hundred-thousandth of what a two-bounce ghost does. The catch is the sun:
  it is about a hundred thousand times brighter than an ordinary highlight, so
  the two cancel out — and a few of these paths happen to focus their light into
  a tight spot rather than spreading it into a wash, which makes them visible
  even when they are faint. On old uncoated glass, where every surface reflects
  a full four per cent instead of a fraction of one, they are not faint at all.

  The difficulty is counting. A lens with twenty-seven surfaces has a few
  hundred two-bounce paths — easy to try them all — but around a hundred
  thousand four-bounce ones, and each trial costs real work. So Lumit sorts them
  first by a cheap estimate that can only ever be *too generous*: multiply
  together how reflective the four surfaces are at their most reflective. The
  best fifteen hundred by that estimate are then tested properly, exactly the
  way the two-bounce ghosts are tested, and whatever survives is ranked into the
  same single list with them. Because the cheap estimate only decides which
  paths get *measured* — never how bright anything renders — being generous with
  it is safe: the worst it can do is spend a measurement on a path that turns
  out to be dim.

  What falls out is pleasingly close to the real world. On the modern
  multicoated cine primes the extra paths exist but never rank high enough to
  draw, because the lens has so many ordinary ghosts ahead of them — which is
  precisely why modern lenses look clean. On a 1927 Biotar, which runs out of
  ordinary ghosts after forty-five of them, over a hundred of the four-bounce
  paths make the cut and the flare gains the doubled train that glass really
  has. Old projects gain those ghosts, so the effect's version number steps up
  again.
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
- **Flow is a layer option** (K-088). On, it synthesises in-between frames with optical flow
  wherever the footage's rate (through any retime) undershoots the comp's — the moment a
  source frame would sit across two comp frames, flow takes over; footage already at comp
  rate costs nothing, because the engine now checks and stands down (see the flow section
  further down for what that gate does and how to override it). Under the hood it's the
  retime's frame-interpolation policy — an un-retimed layer quietly gains an identity retime
  to carry it, and loses it again when you switch off.
  **How you reach it (K-331):** the **Flow** switch in a footage layer's switch cluster, in
  the same cell a Precomp uses for Collapse. Turning it on reveals a **Flow** group beside
  Transform and Effects, carrying flow resolution, vector detail, smoothness, occlusion
  handling, fallback, the HUD guard and an always-on override. Flow deliberately left the
  in-between-frames dropdown, which still offers Nearest and Blend: sitting there made the
  most expensive thing a layer can ask for look like a small setting you pick and forget.
  One rough edge worth knowing: turning the switch off and on again resets the group to
  defaults, because the parameters are stored inside the policy and have nowhere to live
  while the policy is Nearest. Fixing that is on the backlog.
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
  measured in pixels at composition size and scaled to whatever raster is being drawn, so
  half-resolution preview looks the same as full — just smaller.
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
- **Curves (K-412).** The grade everyone recognises: a graph of "what came in" against "what
  goes out", bent where you want it. You get a square with a straight diagonal across it, and
  you drag points on that line — click the line to add one, up to sixteen, drag one out of the
  square to throw it away. Leave it alone and it is a straight diagonal, which is no change at
  all; pull the shadows down and the highlights up and you have the classic S that adds punch
  without touching the middle. Five of these share the square behind a row of tabs: Master for
  all three colours at once, then Red, Green, Blue, and Alpha for the matte.

  Two things about it are choices rather than accidents, and both are explained properly
  further down — see "A control that is a shape" and "The editor that shapes the curve". In
  short: the smooth line is the same *clamped cubic* Photoshop draws, worked out once per
  frame on the processor so the preview and the export are handed identical numbers; and a
  curve **does not animate**, because there is no honest half-way between a four-point curve
  and a nine-point one. After Effects has exactly the same limit for exactly the same reason —
  its curves hold between keyframes rather than tweening. Where you want a grade that
  animates, Levels below is the one with keyframable numbers.
- **Levels (docs/08 §3.31).** The same job as Curves approached from the other end: instead of
  a shape, five numbers with names you can aim — and, since K-413, the frame's histogram drawn
  behind them so you can see what you are aiming at.
  **Input black** and **Input white** say which values in the picture should count as black
  and white — drag them inward to fill out a flat capture. **Gamma** bends the middle without
  moving either end. **Output black** and **Output white** say where those two ends land —
  lifting the output black is the film look where nothing in the frame is fully black. Each
  of Master, Red, Green and Blue gets its own set.

  One deliberate difference from After Effects: AE's Levels flattens everything above the
  input white, because it works in a range where there is nothing brighter than white. Lumit
  works in scene-linear light, where a highlight can be many times brighter than paper, so
  those values carry on through rather than stopping. Pull them back down with another effect
  and the detail is still there.
- **Brightness (K-397).** After Effects' *Brightness & Contrast*, as one effect: a slider that
  adds light to every pixel, and a slider that spreads the picture about middle grey. Both sit
  at zero when neutral, which is AE's spelling, and that is why it is a separate effect from
  Lumit's own **Contrast** (whose neutral is 100 %) rather than a mode of it — one control
  cannot honestly be both numbers, and a project file should not have to guess which one it
  holds.

  Brightness *adds*, where Exposure *multiplies*, and the difference is the reason both exist.
  Adding lifts the shadows exactly as much as the highlights, so it washes the picture out —
  sometimes precisely the look you want. Multiplying leaves black where black was, which is
  what a real camera's exposure does.
- **Hue and saturation (docs/08 §3.33).** Turn the colour wheel, change how colourful the
  picture is, and raise or lower how bright it is — either everywhere, or for one family of
  colours only. There is a Master group and six range groups: reds, yellows, greens, cyans,
  blues, magentas. Pulling the greens toward cyan and lifting them is the grade this effect
  exists for, and it leaves skin alone while it does it.

  How it decides what counts as "the greens" is the interesting part. Each range is widest at
  its own colour and fades to nothing by the time it reaches its neighbours, and the six fades
  are shaped so that they always add up to exactly one. A colour sitting between green and
  cyan therefore takes a share of each and never more than one range's worth in total — so a
  gradient sweeping from green to cyan changes smoothly instead of stepping as it crosses some
  invisible boundary. There is no range width to tune, because there is no boundary to place.

  There is one more guard, and it fixes a bug every naive version of this effect has. A grey
  pixel has no colour, and the arithmetic that works out "which hue is this?" answers zero for
  it — which is red. Left alone, that means lifting the reds quietly lightens every neutral in
  the frame. So each range's pull is scaled by how colourful the pixel already is: greys take
  the Master adjustment and nothing else.

  Finally, **Lightness** is a gain rather than a fade toward white, because in scene-linear
  light there is no white to fade toward. Full negative takes a colour to black, full positive
  doubles it, and nothing clips on the way. The one-knob **Hue shift** above is still the
  effect to reach for when all you want is a dial that holds brightness as the hue turns; this
  one does not try to.
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

  **The project's own colours live inside the picker** (K-448). Along the bottom of it is a
  strip of small squares: the colours this project has kept. Click one and it is applied, the
  same as picking it by hand. The **+** at the end of the strip keeps whatever colour the
  picker is currently showing, and a **right-click** (or a long press) on a kept colour offers
  to forget it. The strip is there only when a project is open, because that is what the
  colours belong to: they are stored in the `.lum` beside the compositions, so a copy of the
  project carries them and so does the machine you open it on next. Keeping a colour and
  forgetting one are ordinary edits — each is one step of undo, like anything else.

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
  so a grainy area averages out instead of grabbing one noisy pixel. Under the grid a strip
  says what you are about to take — the colour and its numbers, and the size of the patch.

  **Taking the value is a drag, not a click.** Press on the picture and *keep the button down*:
  the parameter takes whatever is under the pointer as you move, and the picture updates while
  you go, so you can sweep across a face until the skin tone looks right, or slide a light's
  position around until the flare sits where you want it, and watch the result the whole time.
  Nothing is written to the project until you **let go** — that release is the edit, and one
  press of undo takes the whole sweep back. Change your mind mid-sweep and press **Escape**:
  what you started with comes back and the tool puts itself away. (A press that lands before the
  first pixels have arrived from the engine simply does nothing and leaves the tool armed, so
  you can try again.) Click the pipette again, or press away from the picture, to put the tool
  away without picking anything.

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
  at each key). This grade runs **only on the graphics card**: unlike
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

  **Input space (K-543) — telling the LUT what kind of numbers it is being handed.** This is
  the limitation the paragraph above used to end on, and it is now a dropdown. Lumit does its
  compositing in *scene-linear* light: numbers that behave the way light behaves, where "mid
  grey" is 0.18. Almost every LUT a colourist hands you was baked in a grading application
  that works in *display* numbers, where mid grey is about 0.5. Feed a display-space table
  scene-linear numbers and every colour arrives in the wrong drawer of the filing cabinet —
  the grade is not subtly off, it is reading the wrong entries, which is why a perfectly good
  LUT could come out crushed and murky. **Input space** says which kind of numbers the table
  expects: leave it on **Linear** and nothing is translated (exactly what the effect did
  before, to the last bit); choose **sRGB** or **Rec. 709** and Lumit converts the picture
  into that encoding, looks the colour up, and converts the answer straight back to linear
  before the rest of the stack sees it. Those two curves are the ones Lumit already knows —
  sRGB is the same maths that puts pictures on your screen. Log-encoded LUTs (the ones camera
  manufacturers ship) have no option yet, because Lumit does not yet define any log curve and
  guessing one would be worse than saying so.
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
  drag it earlier and it plays faster. **Taking the retime away is the same thought however
  you say it.** Ctrl+Alt+T again, the stopwatch off, or deleting the last key all end with
  the layer hung back on its source, playing at its own rate from the frame that was showing.
  The alternative — writing down "show *this* one moment, for ever", which is literally what
  a curve with no keys left in it says — would leave the layer frozen on a single frame with
  the row gone quiet, and nobody means that by "remove the retime" (K-329). A freeze is asked
  for the way After Effects asks: leave one key, and that moment holds.
  **The two keys land on the layer's own start and
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
  **Animation has the mirror version of that problem**, and the same control fixes it. Anime
  and hand-drawn animation are usually drawn "on 2s" or "on 3s" — a new drawing every second
  or third frame, with the same picture held in between. So a 24fps cut on 2s really goes
  A A B B C C. Interpolate that at its native rate and half the frame pairs are a drawing and
  its own duplicate, where nothing moves at all, while the others carry the whole step: the
  result judders rather than flowing. Tell Input rate the clip is 12fps — the rate it was
  actually *drawn* at — and every pair spans two different drawings. The dropdown beside the
  field names the cadences rather than the numbers ("On 2s", "On 3s"), because an editor knows
  a cut is on 2s without wanting to work out that 24 ÷ 2 is 12. And because it is keyframeable,
  a cut that switches from 2s to 3s partway through — which happens constantly — can be
  followed rather than compromised on.
  Three more things about flow are worth understanding, because they each fix something that
  used to be quietly wrong.
  **Flow only switches itself on when it can actually help.** Inventing a frame *between* two
  real ones only means anything when there is a gap to fill. At 100% speed every frame of the
  composition lands squarely on a frame of the footage — there is no in-between moment — so
  measuring all that motion would cost a great deal and change nothing. Flow now checks: how
  far does the footage advance per composition frame? If it advances a whole frame or more,
  flow stands down and you get the plain nearest frame. If it advances less than one — meaning
  the same source frame would otherwise be shown twice or more in a row, which is exactly what
  makes slow motion look like a slideshow — flow takes over. A freeze stands down too, since
  there is nothing to move towards. If you want flow anyway, for a look rather than for the
  maths, there is an override in the Flow group that forces it on regardless.
  **Flow is measured at the footage's own size, not the preview's.** This one was a real trap.
  Previews are usually rendered smaller than full size for speed, and flow used to be measured
  on whatever shrunken copy the preview happened to be using. That is not the same measurement
  at a smaller size — it is a *different* measurement, because motion that is obvious at full
  resolution can vanish entirely in a quarter-size copy. The result was a preview that could
  look meaningfully different from the export, which is the one thing the pipeline promises
  never happens. Flow now always measures at the size *you* choose in the Flow group
  (Native by default), no matter how coarse the preview is. The price is honest and worth
  knowing: a layer with flow on decodes its footage at full size even in draft preview, so
  draft mode stops being cheap on that layer. If you need the speed back, drop the flow
  resolution to Half or Quarter — that is a decision you make once, and it then applies
  identically to the preview and the export.
  **Why the vectors used to look bad, and what fixed it.** DIS is a three-part algorithm, and
  Lumit shipped two of them. Parts one and two are *local*: little tiles hunt for where their
  bit of picture went, then every pixel takes a vote among the tiles covering it. That works
  wherever there is something to match — and has nothing whatsoever to say about a patch of
  sky, a cloud of smoke, water, or a dark corner, because every position there looks like
  every other. Those pixels came out with whatever the coarse guess was, flagged
  untrustworthy; untrustworthy was then treated as "hidden behind something", and hidden
  pixels get a plain crossfade. So large soft regions turned into patches of ghosted mush.
  Three reasonable-looking local decisions, one bad picture.
  Part three is **variational refinement**, and it was skipped with the note "mostly helps
  large untextured regions, rare in game footage". They are not rare — smoke, sky, muzzle
  flash and motion-blurred backgrounds are most of a frame during exactly the fast moments a
  montage slows down. Refinement stops treating pixels one at a time and solves the whole
  field at once, balancing three demands: each pixel should land on matching *brightness*, it
  should land on matching *edges* (this is the one that survives an explosion lighting up the
  frame — brightness changes everywhere, edges stay put), and neighbouring pixels should move
  alike unless there is strong evidence otherwise. That last demand is what fills the empty
  regions: a pixel with no evidence of its own inherits motion from neighbours that have
  some, seeping inward from the textured edges. It is the difference between "we don't know,
  so here's mush" and "we don't know, so here's what the surrounding motion implies", which
  is nearly always right.
  Two side effects worth knowing. "Untrustworthy" now means something better — it used to
  mean "nobody found an answer for me", and now means "the answer I was given doesn't actually
  explain my pixels", which is the question that was worth asking all along. And the solver
  sweeps the image in a **checkerboard** pattern rather than left-to-right, which sounds like
  an odd detail but is the whole reason it can run on the graphics card at all: on a
  checkerboard, every neighbour of a "red" pixel is "black", so an entire colour can be
  updated at once by a million threads without any two of them tripping over each other. The
  slow reference implementation is written the same way on purpose, so that the fast one is
  allowed to agree with it.
  **A fourth part that was built, measured, and switched off — and why that is a result, not
  a waste.** Cartoons and anime are the one kind of footage flow has never been good at. The
  reason is that a cel is a flat area of colour with a hard outline round it: over most of
  the frame there is simply nothing to match, because every pixel looks like its neighbours.
  The planned fix was to let the *outlines* decide the motion for the flat areas — solve the
  finished field against the picture's own edges, so a vector spreads freely inside one flat
  region and stops dead at the line round it, instead of a character's motion smearing onto
  the background. (This is a known technique, the "bilateral solver"; the flat-region-filling
  half of it demonstrably works, and there is a test that proves motion crosses a flat band
  and refuses to cross the outline.)
  It was built for both backends and measured on five clips, and it made four of them
  **worse**. The interesting part is why, because the reason is not "it needed more tuning".
  The pass runs *after* variational refinement, and refinement has already smoothed the field
  using real evidence from the two pictures. The new pass smooths it again using no evidence
  at all — only "these two pixels are a similar brightness, so they probably move together".
  It therefore cannot discover anything; it can only swap one kind of smoothing for another.
  Where refinement had nothing to go on it sometimes guesses a little better (the anime clip
  gained a bit), and everywhere refinement was already right it does damage. Every knob
  confirmed it: on four of the five clips, the best setting was the one that made the pass do
  as little as possible. When the best version of a feature is the one closest to switched
  off, the honest reading is that the feature is answering the wrong question.
  So it ships disabled, and the note it leaves behind is the useful bit: **line art is short
  of evidence, not short of smoothing.** Getting more evidence is a different part of the same
  programme — the matching step now compares each pixel with its neighbours rather than
  comparing brightness directly, which is exactly the kind of change that finds something new
  on a line drawing, and that one did help.

  **The HUD guard.** Game footage has a health bar, a killfeed, a minimap — things painted on
  top of the picture that stay perfectly still while the whole world slides underneath them
  during a fast turn. Flow sees a frame where everything moves except a few sharp rectangles,
  and the motion of the background inevitably bleeds into them: the classic artefact where the
  HUD smears across the screen, familiar to anyone who has used Twixtor on gameplay. The guard
  looks for the tell — a region that is **not moving** but **is full of fine detail** (a still
  patch of sky is smooth; a still patch of *text* is not) — and hands those pixels to a plain
  crossfade instead of warping them. For genuinely static content that is the correct picture
  anyway, since both frames agree there, so the guard costs nothing but the smear. It fades in
  and out gradually rather than switching, because a hard edge between "warped" and "not
  warped" is itself something you would see. It is on by default and can be turned off in the
  Flow group.

  **Turning flow off keeps your tuning.** Flipping the switch back to the plain picture is how
  you check whether the flow is actually helping, so the settings you arrived at are put aside
  rather than thrown away — the layer keeps them in a spare pocket (`parked_flow`) and hands
  them straight back when you turn flow on again. The pocket is part of the project, not a
  passing memory in the interface, so it survives saving, closing and reopening, and one undo
  reverses the whole switch-off: the policy and the settings come back together.
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
- `crates/lumit-import/` — **bringing an After Effects project across.** This happens in
  two halves, and the split is the whole design. The first half is a script that runs
  *inside* the user's own copy of After Effects (the **Lumit Bridge**): it walks the
  project — every comp, layer, keyframe, mask, effect and expression — and writes down
  what it finds, in AE's own words, changing nothing. It is a **courier, not a
  translator**: AE's clock times stay AE's floating-point seconds, AE's ids stay AE's
  integers, and every property keeps its AE "match name" (`ADBE Gaussian Blur 2` and
  friends — the stable internal name AE uses, which survives the user renaming things).
  What it writes is a **bundle**: a folder, or a zip of one, holding a `manifest.json`
  saying what version the format is, a `capture.json` holding the walk, and a
  `report.json` listing the handful of properties After Effects itself refused to hand
  over. The second half is this crate, and it does all the actual translating: AE's clock
  times become Lumit's exact times, AE's effects become Lumit's effects through a mapping
  table, and anything that cannot translate becomes a clearly-labelled placeholder rather
  than quietly vanishing.
  Why split it that way, rather than have the script produce a Lumit project directly?
  Because the script needs a real installation of After Effects to run, so no automated
  test of ours can ever check a line of it — and the conversions are exactly the part that
  must not drift. So the untestable half is kept too simple to be wrong, and every
  judgement lives on the half the test suite watches. It also ages better: Adobe changes
  the details between versions, and a script with no opinions has far less to break.
  The reader is deliberately forgiving in one direction and strict in the other. A bundle
  carrying fields this build has never heard of opens fine, with the unfamiliar parts
  ignored — that is how the format is allowed to grow. A bundle from a *newer* Lumit is
  refused outright, with a plain "please update Lumit", because reading an unfamiliar
  schema by guesswork is how an import goes silently wrong. And a bundle whose report is
  damaged still opens: the report is commentary, the capture is the work.
  From the outside all of that is one menu row: File ▸ Import After Effects bundle…, a
  folder chosen, and then a window listing what did not come across untouched. An import
  **replaces** whatever project was open, exactly as opening a `.lum` does — the same code
  performs the changeover, so the outgoing project's renderer is let go on either route
  rather than sitting in memory holding a graphics card. Footage is looked for on this
  machine at the same moment; a file that is not here imports as an **offline** item with a
  row saying so, because an import that waited for missing media would be an import nobody
  could finish.
- `crates/lumit-import/src/map/fx_*.rs` — **the effect mapping table**, the part of the
  import that turns an After Effects effect into the Lumit effect that does the same job.
  It is a list, one entry per effect, and each entry answers the same four questions.
  *Which dial becomes which dial* — AE's "Blurriness" is Lumit's Radius. *What arithmetic
  turns one number into the other* — AE's Twirl measures its radius as a per cent of the
  layer, and Lumit measures it in pixels at composition size, so the number changes even
  though the picture does not. That conversion is applied to the
  keyframes too, and to the *handles* on them: a keyframe handle is a speed, in "so many
  units a second", so if the units change the handle has to change with them or the curve
  between two keys is no longer the curve somebody drew. *Which of AE's dropdown entries
  is which of Lumit's* — After Effects stores a dropdown as a plain number, so getting the
  order wrong is a silently wrong picture rather than an error, and every list is anchored
  on the one entry a live After Effects has confirmed. And *what could not come across at
  all* — which is the question that gets the most careful answer, because the rule is that
  a control Lumit does not have is written down in the import report and never quietly
  approximated into something that looks similar. There are four kinds of note the report
  can carry about an effect: a control that was not carried, a control that arrived as the
  nearest thing Lumit has, a number that means the same thing in different units, and an
  effect that maps whole but evaluates differently by construction — Lumit works in real
  light where After Effects worked in eight-bit display values, and some differences are
  the point rather than a defect. A handful of effects are deliberately *not* in the
  table: Timewarp, because retiming is a whole feature of Lumit's rather than an effect,
  and Remove Grain, because removing grain is its own programme. Those import as
  placeholders, and the report says what to use instead.
- `crates/lumit-media/` — **reading media files** (via FFmpeg, the industry-standard
  media library). Two jobs so far: the *probe* (a file's vital statistics — resolution,
  frame rate, duration — shown under each item in the Project panel) and the *frame
  index* — a scan of the whole file that records where every frame and keyframe sits, so
  scrubbing can land on exactly the right frame. Indexing runs on a background thread
  (the UI never waits) and the result is cached on disk, keyed by a *fingerprint* of the
  file's content — change the file and the stale index is ignored automatically.
- `crates/lumit-render/src/media_index.rs` — **the one place that decides whether to scan.**
  That frame index costs seconds on a long clip, and it depends on nothing but the file's
  bytes, so it is worked out once and parked in a small sidecar file in Lumit's cache
  folder, named after the fingerprint. Everything that opens a decoder asks *this* helper
  for the index — the probe that fills the Project panel, the Viewer's decode, the
  decode-ahead thread, the thumbnails — so the sidecar one of them writes is the sidecar
  the others read, and the second time you open a project nothing is scanned at all. It
  had to be one shared helper: the probe used to warm the cache and the decoder used to
  ignore it, which is why the first preview frame of a session used to sit there thinking
  for a few seconds on every clip. If the file has changed since, the fingerprint no
  longer matches and the index is rebuilt rather than believed; if the sidecar is corrupt,
  or the cache folder is not writable, or the machine has nowhere to put it, everything
  still works — it simply scans, like it did before the cache existed. The folder is
  always safe to delete.
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
- **A cached frame has a *size*, not just a place.** When the machine cannot keep up, the
  preview quietly renders smaller — half, a third, a quarter of full size — and those
  smaller pictures get cached like any other. So "is frame 12 cached?" has never been quite
  the right question: the useful answer is "cached at what size", because a quarter-size
  frame is real cache that will still be re-rendered the moment you ask for a sharp one.
  Each frame of the cache bar is therefore **one byte holding two answers**: where the
  picture is kept (in memory, on disk, nowhere) and how big it is (full, half, third,
  quarter, measured against the size the Viewer is currently asking for). Two honest limits
  come with it. The engine can only look for the four sizes it actually renders at, so a
  frame cached at some other size is not found and shows as uncached — the same blind spot
  the bar always had. And on a very long comp the strip is first sketched from samples, one
  frame standing in for its neighbours, so an unrefined stretch wears its sample's size
  until the slow pass reaches it. The bar's colours grow out of that byte (K-441).
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
  each other* in one lane rather than side by side, getting brighter as the pitch goes up, so
  the result is one waveform with its insides showing and you can cut to the kick or to the
  hat and see which is which. Two small things make three overlapping waves readable instead
  of a pile (K-382): the palest one is painted first so the darker ones land in *front* of it
  (a dark shape on a pale one reads as two shapes; the other way round the pale one just
  swallows it), and each is nudged a couple of pixels higher than the one behind — like
  fanning a hand of cards, so no card is completely hidden by the one in front.
  There is a second switch beside it for **where the wave sits**. Normally it is centred, with
  the sound drawn going up and down from a middle line — but the two halves are mirror images,
  so half the row is saying the same thing twice. Turn *Waveforms rise from the bottom* on and
  the wave is folded onto the floor of its row: every column starts at the bottom and reaches
  up by however far the sound swung, which uses the whole row's height and is what a lot of
  editors draw. It applies to the plain wave and the frequency stack alike, and it is only a
  matter of drawing — nothing is re-read from the file when you flip it. It is on by default; Settings ▸ Interface ▸ Editing has a switch that puts the single
  plain wave back. (The idea is BLICK's, an editor that does the same thing.)
- **A slowed layer's waveform is slowed too (K-436)** — a waveform is drawn column by column,
  and each column has to stand for the moment of sound you would actually hear if the playhead
  were there. For a layer playing at normal speed that is easy: a second of the bar is a second
  of the file, so the drawing and the file run side by side. Retime a layer, though, and they
  stop running side by side — half speed means a second of the bar is half a second of the
  file, and reversing means the bar runs through the file backwards. The lane used to ignore
  that and lay the file out evenly along the bar anyway, which meant a layer slowed to half
  drew all of its sound crammed into the left half of its bar and silence across the right,
  where what you actually hear is spread across the whole thing. Now every column is looked up
  through the layer's Retime — the same map that decides which *frame* the column shows — so
  the wave stretches, squeezes and reverses exactly as the picture does, and a beat you can see
  is a beat you can hear at that spot. A layer nobody has retimed takes a shortcut and reads
  the file in one straight sweep, as it always did; only a retimed one pays for the per-column
  lookup. Clips on a Sequence layer worked this way from the start — this is the layer lane
  catching up with them.
- **The waveform sits on the line between its two rows (K-437)** — the waveform lane lives
  under a **Waveform** twirl, and that twirl has a row of its own with nothing in it: a label
  on the left and blank space across the whole width of the timeline. So there were two rows
  there and only one of them was carrying a picture. The wave is now drawn across both. A
  centred wave is symmetrical about silence, and the divider between two rows is the strongest
  horizontal line in the lane area — so silence is put *on* that line and the wave goes up into
  the empty row and down into its own, twice as tall as before and read against a line that is
  really there. The from-the-bottom mode keeps its floor and simply has twice the height to
  rise through. Nothing about the layout changed: the rows are the height they always were and
  every layer below stays exactly where it was, because only the *painting* reaches up past its
  row. (A clip's waveform is left alone — a clip is a box on a row, with no spare row beside it
  to borrow.)
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
  was reorganised into a subfolder is found by what it *is*, not where it was. And for
  anything still lost after all that, one last sweep looks for it **by file name** anywhere
  under the project's folder. That last one is deliberately the weakest, which is why it goes
  last, and it exists for a case none of the others can help with: a project imported from
  After Effects on a different computer. Such a project carries the paths of the machine it
  was made on and has no fingerprints at all — nothing has ever been saved — so if the clips
  are sitting in a subfolder beside it, their names are the only thing left to go on.
  Anything still missing after that is named in a notice and its reference kept intact.
- **When footage goes missing, you see colour bars** — the broadcast test pattern, the same
  one a television shows with no signal. The reasoning is that the alternative is worse: a
  missing layer that renders *black* looks exactly like a deliberate edit, so the mistake
  can survive all the way into an exported file. Bars cannot be mistaken for anything but
  "there is nothing here". They appear in the Viewer and in exports alike, for the same
  reason. In the Project panel the item wears a crossed-link icon and a **Relink…** button;
  pointing it at the file's new home also relinks every *other* missing file that moved the
  same way, in one undo step — losing a folder of footage is then one dialogue rather than
  twenty. "The same way" is worked out from the one file you did point at: whatever its old
  path and its new path have in common *at the end* is the part that did not move, and
  everything in front of that is the part that did. So if an edit kept forty-eight clips in
  forty-eight different subfolders and the whole tree was moved to another drive, relinking
  any one of them — however deep — tells Lumit where the root went, and it looks for each of
  the others in its own subfolder under the new root. Files it does not find that way it
  still looks for by name beside the one you picked, and it never repoints an item that is
  working or one whose predicted file is not actually there. The pattern itself is drawn by arithmetic at whatever size is needed, not
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
  **Reusing one ease (the Easing panel).** Shaping handles key by key is precise and slow,
  and montage work wants the *same* ease on a great many keys. The bottom bar's
  **Easing…** button opens the **Easing panel**, where you draw an ease once: travel runs
  left to right, from nothing done at the bottom-left corner to all done at the top-right,
  and two handles bend the line between them. Pick a starting shape from the row of
  presets, nudge it, then press **Apply** to put it on the selection.
  It is a *panel* rather than a floating box for one reason, and it is worth spelling out
  because the first version got it wrong. A floating box goes away the moment you click
  anywhere else — and clicking different keyframes *is* clicking somewhere else. So a shape
  you had just got right could only ever be used on the keys that happened to be selected
  when you opened it, which defeats the purpose: the value of drawing one ease is putting
  it on this run of keys, then on that one, then on a third. Docked, it simply stays there.
  Select, Apply, select something else, Apply again.
  The button docks the panel the first time you press it and brings it to the front after
  that; **Window ▸ Easing** does the same, and the **Retiming** workspace has it down the
  right already. If you would rather have the screen space back, Settings ▸ Interface ▸
  Editing ▸ *Shape eases in a popup* gives you the floating box instead, with the
  limitation above.
  When the panel cannot do anything with a shape — no Timeline on screen, or the graph is
  showing its Speed view rather than Value — **Apply** greys out and a line underneath says
  why, rather than the button quietly ignoring you.
  It works on **spans**, not keys: a stretch of travel takes the shape when the keys at
  *both* of its ends are selected, so selecting a run of keys eases the whole run, and
  selecting one lone key does nothing — it has named no travel to shape.
  The presets are named for which end of the travel is slow (*Slow start*, *Slow finish*)
  rather than "in" and "out", which here already mean the two *sides* of a key, and mean
  the opposite thing on the web. Two of them leave the box on purpose: **Overshoot** runs
  past its destination and settles back, and **Anticipate** pulls back a little before
  setting off. That is why the box has room above and below it.
  One thing worth understanding, because it is the whole trick: a keyframe does not store a
  shape, it stores a **speed** in real units per second. So the same drawn ease has to
  become a *different* stored speed on a move of four hundred pixels than on a move of
  forty — otherwise only one of the two would look like the curve you drew. Lumit works
  that out per span, from how far and how long that span travels
  (`flutter_ui/lib/panels/easing_curve.dart`). Influence, being already a fraction of the
  gap, carries across untouched.
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
  **The razor goes through a ramp.** A clip that has been sped up or slowed down over its
  length used to turn the blade away, which was the wrong way round: the reason to cut is
  usually a beat, and a ramped clip is the one most likely to be sitting on one. It cuts now,
  and the two pieces play exactly what the one piece played. The reason it can is worth a
  sentence, because the same trick is behind several things here: the speed line between two
  points is a *curve of a particular family* (a cubic), and cutting a curve of that family in
  half gives two curves of the same family that, laid end to end, are not merely close to the
  original — they are the original. So nothing is resampled, nothing is fitted, and no frame
  either side of the new edit point shows a different picture than it did. The one thing that
  needed care was a point that **aims itself**: such a point works its direction out from the
  points on either side of it, and a cut changes what is on either side, so the two nearest
  the blade have their directions written down first — the same direction, just stated rather
  than inferred (K-573).
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
  **A key's mark is two halves, and each half is a shape.** A keyframe has two sides — how
  the value *arrives* at it and how it *leaves* — and they need not agree: a move can ease
  in and then hold. So the mark on the lane is split down its middle, and each half is
  drawn from its own side's interpolation: a **diamond** point for linear, an **hourglass**
  for eased (bezier), a **square** block for a hold. A key that eases in and holds out is
  therefore half hourglass and half square, which is the truth about it and is not
  something one shape could ever have said. All three stand the same height, so a lane of
  mixed keys still reads as one even row of marks. The same marks at the same size on a
  property's own lane and on a shut layer's summary — one painter draws them all.
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
- **Any layer can *act* as an adjustment layer** — the fifth switch in the Timeline's Modes
  column, on every row that puts something on screen. Switch it on and that layer stops
  showing its own picture and starts treating everything below it instead: a shot you had
  already colour-corrected becomes the thing that colour-corrects the shots under it.
  Switch it off and the layer is simply itself again — the footage, the masks, the
  position, the effects, all exactly where they were. That "exactly where they were" is the
  whole reason it is a switch rather than a conversion. Turning a layer into another *kind*
  of layer would mean throwing its source away, and there would be nothing to give back
  when you changed your mind; a switch changes only what draws, so it costs nothing to try.
  Two small consequences worth knowing. It is drawn on hidden layers too, because "is this
  an adjustment layer" and "am I looking at it right now" are two different questions. And
  a layer *created* as an adjustment layer has no picture of its own to give back, so
  switching it off hands it a fresh white solid — the same one the *New solid* command
  makes — in one undoable step.
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

  **Every panel has a floor, and the seam stops there.** A panel narrows by giving things
  up in a fixed order — a word shortens, a column hides, a run of buttons folds into one
  `⋯` mark — but eventually there is nothing left to give up, and squeezing further would
  mean drawing over the panel's own edge. So each panel states the width below which it
  will not be squeezed, and dragging a seam past that simply stops: the boundary refuses to
  move rather than sliding on while the panel underneath breaks. The floors live in one
  list in `shell/dock_widget.dart`, next to the wrapper that enforces them.

  There is a second half to that, for the case a seam cannot cause: a window genuinely too
  small to hold the arrangement at all. Then the panel keeps its own width and **slides
  sideways** inside its pane, like a table wider than the page it is printed on. That is
  better than the alternative, which is what used to happen — Flutter draws a striped
  warning band over the part that did not fit and complains once per frame, and on a real
  build the panel simply looked broken. Because the sliding is done by one wrapper around
  every pane rather than by each panel separately, a panel nobody has ever tested at 40
  pixels wide still cannot paint outside its box; `panel_width_sweep_test` pumps all seven
  of the big panels across every width from 40 to 400 and fails on the first complaint.
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
  instant the current frame is ready. **It has to be sure the kept frame is still the frame**
  (K-330): Lumit files a kept frame under a name made from its *contents*, and notes which
  moment it was made for. Edit the comp — retime a layer, say — and that moment now looks
  different, so it renders under a new name, while the old picture is still sitting there
  labelled with the same moment. The scope used to take whichever of the two was the sharper
  copy, which flipped back and forth as frames came and went, so it flickered between the
  picture and the picture that moment used to be. It now checks that the name still matches
  what the moment renders to today, and traces its own frame rather than trusting an old one. The counting itself now runs on the graphics card (the
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
  footage, and the clips land in the comp it makes. **Filing is a drag too**: drop rows onto
  a folder row and they move inside it, or use **Move to folder** on the right-click menu,
  which lists the project's folders — either way the whole selection goes together as one
  undo step, and **Move to root** brings things back out. An item lives in one folder at a
  time, so filing it somewhere takes it out of wherever it was in the same step; a folder
  cannot be filed inside itself or inside anything it holds, which would take that whole
  branch off the panel with no way back to it, so Lumit simply declines those. Solids are proper assets now — one "White solid"
  in the project can back fifty layers, and the first one you make creates a Solids
  folder that future solids follow even if you rename it or tuck it inside another
  folder (Lumit remembers the folder itself, not its name). Compositions do the same
  with a Compositions folder. Multi-step creations like that land as a single undo
  step — a batch operation whose inverse is just the reversed inverses of its members.
- **A mask shape that moves (`lumit-core::mask`)** — everything else Lumit animates is a
  single number: a position, an opacity, a blur radius. A keyframe on one of those says "be
  40 here and 90 there", and asking for a moment in between is arithmetic. A mask path is not
  a number — it is a whole drawn shape, a ring of points each with two handles — so
  animating it needed its own small machine sitting beside the one that does numbers, rather
  than a rebuild of that one. A mask now carries a list of **shape keyframes**: at this
  moment the shape looks like *this*, at that moment like *that*. If the list is empty (which
  it is for every mask you have not keyed) nothing changes anywhere, including in the saved
  file, so old projects load and re-save byte for byte and their cached frames stay good.
  - **In between two keys, the shape is blended point by point** — each point walks from
    where it was towards where it is going, and its two handles walk with it, so the curve
    bends smoothly rather than snapping. The **timing** is the ordinary keyframe timing:
    hold, linear, or an eased handle, all borrowed from the number machine rather than
    written again. What the ease shapes here is *how far along the crossing you are* — 0 at
    one key, 1 at the next. There is no value graph for a shape, because there is no value
    to plot; the timeline shows shape keys as diamonds, exactly as After Effects does.
  - **The awkward bit: the two shapes need not have the same number of points.** Adding a
    point halfway through an animation is one of the most ordinary things anyone does, and
    refusing it is not an option. So before blending, the sparser shape is redrawn with as
    many points as the denser one — and this has to happen *without changing the shape*, or
    adding a point would visibly dent the mask. The trick is that a curve segment can be
    **cut in two** and the two halves, taken together, are the exact same curve; nothing is
    approximated. Cut a four-point ellipse in the right places and you have a seven-point
    ellipse that is still, pixel for pixel, the same ellipse — it just has more handles to
    grab. Which segments get the extra points is fixed arithmetic (spread evenly, earliest
    first) rather than anything clever, so the same two shapes always reconcile the same way
    and a playback is repeatable frame for frame.
  - **Open or closed is not something you can be halfway.** A shape is either joined up at
    the ends or it is not, so that setting *holds* across the crossing and flips at the next
    key, like a hold keyframe. The points still travel smoothly; only the closing segment
    appears or disappears, on a frame boundary, rather than smearing into existence.
  - One thing worth knowing about the cache: the name a rendered frame is filed under is
    made from *what is in the picture*, never from which frame it is (that is the whole
    reason a duplicated composition shares its original's cached frames). A keyframed mask
    is written to the file identically at every moment, so the list of keys alone would give
    every frame of a moving mask the same name — and playback would show the mask stuck at
    whichever frame drew first. The *worked-out* shape at that moment goes into the name as
    well, and only for masks that are actually animated, so nothing that already exists is
    disturbed.
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
- **Two ways to play back (K-171)** — the important distinction between the two preview
  modes, and both of them now run *inside the engine*. The frontend says "play from here"
  and paints what arrives; the worker thread does the deciding, because every decision
  (which frame is next, has the clock passed it, may this mode skip) needs the cost of the
  frame just finished, and the frontend has none of that. It used to guess — a Flutter
  ticker polled the audio clock each screen refresh and asked for a frame — which was a
  scheduler living on the far side of the FFI boundary from everything it schedules
  against (K-181).
  In **Cached** mode (labelled *Every frame*, the default) Lumit shows you *every* frame and
  never skips: frames go out in order, one per composition period, and never faster —
  however full the cache is, a 60 fps comp plays at 60. If the comp is heavy and rendering
  is slower than real time, playback simply falls behind — you see every frame, just not at
  full speed — and once a stretch is rendered it replays at true speed from the cache. Sound
  is stopped whenever the picture drops off the composition's rate (so it never runs ahead
  of a picture that has fallen out of time with it) and rejoins after eight pictures in a
  row have gone out on time.
  In **Realtime** mode (labelled *Adaptive*) the opposite trade: the clock never waits. The
  frame shown is the newest rendered frame the clock has reached, the ones it has already
  passed are binned rather than shown late, and when frames cannot keep up Lumit drops the
  preview *resolution* to stay in time rather than slowing down.
  Both modes ride the same machinery, described in the next bullet.
- **Rendering ahead: the ring, and the referee that is left (`lumit-bridge::playback` plus
  `lumit-eval::schedule`)** — how playback stays smooth. The worker no longer renders a
  frame and shows it in the same breath. It renders *ahead* of the clock into a **ring**: a
  small queue of finished frames still sitting on the graphics card, each shown ("presented"
  — one cheap copy) only when it is due. The slack is the whole point. A span of cheap or
  cached frames fills the ring, and when an expensive frame comes along it spends the banked
  time instead of stalling the picture. This is what makes decode-heavy comps watchable:
  dropping the preview resolution makes *compositing* cheaper but video *decoding* costs
  about the same whatever size you view it at, so the only real answer to heavy footage is
  to have decoded it already.
  Four rules do the work, and each is plain arithmetic with its own test:
  - **How far ahead to render** is not guessed. The worker keeps the last 32 render costs
    and takes the 95th percentile — sized by what the *slow* frames cost, since absorbing
    the occasional slow frame is the entire job — then renders `2 × that × fps` frames
    ahead, never fewer than 8 and never more than 16. The floor keeps a cushion even on
    cheap comps; the ceiling is the memory bill (at worst 16 display frames held at once).
  - **Which frame to show now.** Cached mode always takes the front of the ring, on a
    *grid* of due times: a frame that goes out a millisecond late leaves the next one due
    at its grid time, so loop overhead is absorbed instead of compounding. (Restarting the
    stopwatch at each frame instead added every scrap of lateness to every frame, and a
    60 fps comp could never actually reach 60.) Realtime mode takes the newest queued frame
    the clock has reached and discards the passed ones.
  - **Sound waits for a pre-roll.** Starting the audio the instant play is pressed means it
    runs while the first frame is still being composited — sound already tens of
    milliseconds in before there is anything to see, and then a jump as the picture catches
    up. The sound starts once three frames are banked, or after 150 ms whatever happens, so
    a heavy comp never sits in silence waiting.
  - **What resolution to render at**, in realtime mode — and this is the piece `lumit-eval`
    still owns, as a deliberately pure function of numbers with no clock, no threads and no
    GPU anywhere near it. It watches a smoothed average of recent render costs against the
    frame budget: above 0.9 of the budget it drops a preview tier immediately (Full → Half →
    Third → Quarter); below 0.4 of it, and only after twelve consecutive such frames, it
    earns one back. Quick to worsen, slow to improve, with a wide dead band between the two
    thresholds — that is what stops the picture flickering between qualities — and a rise
    that has to be reversed straight away doubles the patience required before trying again.
    `lumit-bridge::realtime` runs one of these for the session and feeds it the measured
    cost of each genuine GPU render. The cost measured is the worker's own render time, not
    the time from asking to seeing: the latter folds in how often the screen happens to
    refresh (~16 ms), which would make even a cheap comp look exactly one refresh slow and
    walk the resolution down for nothing.
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
  One bug here is worth recording because the symptom pointed away from the
  cause. Frames are filed under a *name* made from everything that went into
  them, and the preview resolution is part of that name — a half-size frame and
  a full-size one are two different pictures. Playback is allowed to quietly
  drop the resolution to keep time, and that decision stuck around after
  playback stopped. So the background filling went on making full-size frames
  while every scrub asked for half-size ones, and the two never met: the cache
  bar went green, and scrubbing onto a green frame still re-rendered it. The
  resolution drop is playback's business alone now, and a still frame is made at
  the size you asked for.

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
- **A moved layer keeps its cached frames, even a keyframed one.** A layer's own clock
  is the composition's clock minus where the layer starts, and computers do that sum in
  floating point, which is very slightly inexact: ten thirtieths minus three thirtieths
  comes out a hair's breadth off seven thirtieths. A hair's breadth is enough to nudge every
  animated value, and a nudged value means a different frame key, so dragging a keyframed
  layer three frames along the timeline used to throw away nearly all of its cached frames
  for no visible reason. Lumit now does that one subtraction in exact fractions and only
  converts the answer afterwards, so the same moment in a layer's life always produces the
  same numbers, wherever the layer sits.
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
- **Mask modes, feather and expansion — and the distance field underneath them.** A mask has
  a **mode** that says how it joins the masks above it: **Add** widens what shows, **Subtract**
  cuts a hole in it, **Intersect** keeps only where both agree, **Difference** keeps where
  exactly one of them covers, and **None** parks the shape as geometry that gates nothing (handy
  while you draw). They apply top to bottom in the list, so the order is part of the result:
  A with B subtracted from it is a different picture from B with A subtracted from it. The one
  question with no obvious answer is what the *first* mask combines with, since there is nothing
  above it. Lumit does what After Effects does: if the top mask is Add the stack starts empty, and
  otherwise it starts as the whole frame — which is what makes a single Subtract mask punch a hole
  in the layer instead of leaving you with nothing at all.

  **Feather** softens a mask's edge and **expansion** grows or shrinks the shape (in layer
  pixels, positive out, negative in). They sound like two jobs — a blur and a choke — but they
  are one, and building them as one is why they behave. The trick is a **signed distance
  field**: for every pixel, work out how far it is from the mask's outline, counting distances
  inside the shape as positive and outside as negative. Zero is exactly on the line. Once you
  have that map, both controls are just ways of reading it. Expansion adds a constant to every
  distance, which slides where "zero" falls and so moves the whole outline outward or inward,
  keeping its shape and rounding its corners the way an offset outline should. Feather says how
  many pixels the fade takes to cross that zero: a feather of 12 goes from fully on to fully
  off over twelve pixels, six either side of the line. A blur would have smeared corners and
  thin necks differently from long straight edges; distances don't care about any of that.

  Two details worth knowing. The distances are measured from the *antialiased* raster, not from
  a hard on/off version of it, so the softness starts out as smooth as the edge Lumit drew —
  a partly-covered pixel tells you, by how covered it is, roughly how far past its centre the
  edge ran. And feather and expansion are in **layer** pixels, so they are scaled along with
  everything else when the Viewer drops to half or quarter resolution: a soft edge looks the
  same width at every preview setting, and the same again on export. A mask with feather and
  expansion both at zero skips all of this entirely and uses the raster untouched — which is
  nearly every mask, and it costs nothing.
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
  edges are single crisp 1px lines. The colours themselves (the spruce accent, the cool grey
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
  edge, Figma-inspired, no blur or bevel, just a soft shadow standing in for the border.
  Round commits to that shape rather than merely softening the corners: every button, chip,
  tab and dropdown is a full capsule (a "stadium" — the ends are half-circles, so the corner
  is always half the control's own height, whatever height it turns out to be); the tab or
  mode you are currently on is filled solid with the accent colour instead of just having its
  text tinted, which is what makes state readable across the room; Timeline layer bars and
  Sequence clips get the same rounded ends; and each panel's tab carries a small accent dot
  beside its name — decoration, not a status light, so it never blinks or changes to tell you
  anything. Sharp is untouched by all of it. Finally,
  **Animation** picks how much motion the UI's own chrome shows (All / Minimal / None) —
  this reaches things like a collapsing section's arrow or a dialog's fade-in, not (yet) the
  app's own dropdown menus, which don't animate at all today regardless of this setting. All
  five persist with your workspace; Reset returns the spruce default for Accent.
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
  the frames around wherever the playhead sits, so scrubbing nearby feels instant — it
  works its way round the whole work area (off the end and back to the start, the way a
  loop plays), filling the graphics card first and then ordinary memory, so a loop that
  fits in the two together plays warm from end to end — switch it
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
- `flutter_ui/lib/icons/icons.dart` — **the icons, by name** (K-085, K-440).
  Little pictures like the play triangle or the padlock are asked for here by name: the
  `LumitIcon` list is the set of names the panels use, and this file decides what each one
  draws. Every glyph takes the theme colour (dimming on hover, turning accent when active)
  exactly like text does. Emoji are banned: a glyph is either from the icon set or
  deliberately drawn, never a character we hope the user's fonts carry — that's how the
  invisible stopwatch/arrow bugs happened.
  Most of those names now draw **Lumit's own glyph** from `lumit_icons.dart` (below). The
  handful the new set has no drawing for yet — the puppet, roto, vertex and camera-navigation
  tools, the star, the solid, the fx switch, the label tag, the snap magnet, tone map, the
  node panel's mark — still come from Iconoir, the free icon family Lumit started with, so
  nothing on screen is a glyph borrowed to mean something it doesn't. A few marks are drawn
  by hand rather than looked up at all, because they are Lumit's own artwork: the Null
  layer's crossed square, the rounded-rectangle tool, the Viewer's layer-controls box, and
  the zoom slider's two hills. As the new set gains those missing drawings, the names move
  across one line at a time and no panel changes.
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
- **When a control changes what its number means (`lumit-core::fx::migrate_percent_to_px`)** —
  a saved project stores a bare number for every control, and the number only means something
  because the effect says what unit it is in. So when a control's unit changes — a centre that
  used to be "half way across the frame" becoming "960 pixels" — every project saved before
  the change is holding a number in the old unit, and rendering it as though it were the new
  one would move the picture. Each effect therefore carries a small version number of its own,
  and on load an instance whose version is behind gets its old numbers converted and its
  version stamped forward. Done once, and never twice: the version is the record that it
  happened. The conversion needs to know how big the composition is — half a frame is only a
  pixel count once you know the frame — so it runs from inside the composition as the project
  is read, rather than from the general "fill in missing controls" pass, which is handed a bare
  list of effects and has no frame to measure against. Keyframed controls convert whole: every
  value scales and so does the steepness recorded at each key, which leaves the animation
  curve exactly the shape it was.
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

- **How an effect is written down (`crates/lumit-core/src/fx/effects/`, K-381)** — adding an
  effect used to mean writing the same effect out five times in five files, and keeping the
  five copies in step by hand: the list of its controls, a slot in one giant list of every
  effect there is, the code that fills that slot in, the code that turns it into work for the
  graphics card, and the plain version the tests check the graphics card against. Nothing
  noticed when the copies drifted apart, and adding one control to one effect meant touching
  all five. Now an effect is **one file**: a small block that names its controls, with each
  control's slider range and starting value written beside it, and the catalogue entry is
  generated from that block rather than typed out again. One line in `catalogue.rs` says the
  effect exists; that line's position is the order it appears in the Add-effect menu.

  Two knock-on changes are worth knowing about. First, when a frame is rendered the effect's
  controls become a small **list of (which control, what number)** rather than a slot in a
  fixed list of every effect — which is what will let an effect have controls nobody wrote
  down when Lumit was built: a slider you added yourself, or one read out of a shader you
  typed. Second, each control now says what its number *means* — a plain number, a
  length in pixels at composition size, an angle — so the step that shrinks
  everything for a draft-quality preview can do it for every effect at once, instead of
  needing a line of code per effect that somebody could forget to write. A preview that
  disagreed with the export used to be one forgotten line away.

  The move happened in batches, simplest effects first, and while it ran a test held each
  generated entry against the hand-written one it replaced and failed if they differed by so
  much as a default value — which is what made it a move rather than a rewrite.

  **All thirty-five effects have moved**, in nine batches: the ten colour ones first (Colour
  balance, Saturation, Vibrancy, Exposure, Hue shift, Contrast, Gamma, Temperature, Invert,
  Tint), then the five blur ones (Gaussian blur, Directional blur, Radial blur, Sharpen,
  Simple sharpen), then Vignette, RGB split and Chromatic aberration, then Flash, then
  seven more: Sprite flare, Transform, Glow, Block glitch and Scanlines, plus Posterize time
  and Motion blur, which draw nothing at all (see below); then LUT and Echo, the first two
  whose real input is not a number at all (see below); then the rest of those — Depth of
  field and Light wrap, which read another layer's picture, Fast motion blur and Datamosh,
  which read the movement measured off the footage, and Matte key, which turned out to read
  nothing extra at all and was simply large; then the Lens flare, which is the biggest
  effect Lumit has and needed one thing nothing before it did (see below); and last Shake,
  whose nine sub-frame wobbles do not fit a list of single numbers, so the list learned to
  hold a group of four at once and Shake's wobble was split into the part that scales with
  the picture and the part that does not.

  With the last one across, the old arrangement is **gone** rather than merely unused: the
  giant list of every effect, the slot each one had in it, the hand-written passage that
  filled that slot in, and the one that turned it into a graphics-card call have all been
  deleted, along with the test that was holding the two catalogues together — there is only
  one catalogue now, generated from the one list. A layer's effect stack is a single
  ordered run of controls, and the two places that walk it (the graphics-card path and the
  plain reference version) choose nothing: they ask each effect for its behaviour **by
  name**, which is the door third-party effects will one day come through. The step that
  works out an effect's numbers at a frame is likewise **one loop for all of them** — it
  reads the effect's own block of controls and asks the project for each value in turn —
  instead of a hand-written passage per effect.

  The sums each effect does before the graphics card sees the numbers — turning "Saturation
  250 %" into a factor of 2.5, or a temperature into two channel gains — now live in one
  place per effect, and both the graphics-card path and the plain reference version read
  that one place. That matters more than it sounds: the test that proves the two agree
  compares their *pictures*, so it would never have noticed the two doing the division
  differently. One copy cannot disagree with itself.

- **The effects that need the clock (K-385)** — the arrangement above assumes that reading
  an effect's controls tells you what it will draw, and a few effects break that: how bright
  a Flash is now depends on the moment, the composition's beat markers and a whole keyframed
  Trigger track, Scanlines needs to know how far its lines have scrolled by now, and Block
  glitch which coarse *tick* the moment falls in — none of them a control anybody could put
  a slider on. So an effect can answer one extra question at frame time — "given the clock
  and the markers, what do you work out?" — and its answers join its controls in the same
  list of numbers, under names marked *derived*. They are never saved and never shown in the
  panel, just recomputed every frame exactly as the old hand-written passage recomputed them.

- **The two effects that draw nothing** — Posterize time and Motion blur never change a
  pixel's colour; they change *what moment the layers underneath are drawn at*, which is
  decided a level above the effect stack, in the walk that works out which frames to render.
  So they declare their controls like everybody else and declare that they have no picture
  pass, and the render path skips them entirely.

- **The effects whose real input is not a number (K-387)** — a LUT's input is a *file* of
  colours somebody exported from a grading tool; Echo's input is the *pictures* of the
  frames just before this one; Depth of field's is another layer, rendered on its own. None
  of that fits in a list of numbers, so the render prepares those things in lists running
  alongside the effect stack, and which entry belongs to which effect is settled by
  **counting**: the second LUT on a layer takes the second cube. That counting is the whole
  contract, and it is easy to break in a way nothing notices — skip a cube because its file
  was missing, and every LUT below it silently grades with somebody else's look.

  So an effect now **says which list it consumes**, and one place — the loop that hands work
  to the graphics card — does the counting for all of them. An effect that names no list is
  handed nothing, which is nearly all of them. An effect whose entry is missing (the file
  never loaded, the layer was deleted) is handed an empty slot and passes the picture through
  untouched: the labelled do-nothing Lumit uses everywhere rather than an error. And the
  count still advances for that empty slot — which is the bug the regression test in
  `gpufx.rs` is written to catch, because it is the one that looks harmless.

- **The effect whose lights are other layers** — the Lens flare has three ways of being
  told where the light is. Two of them fit the arrangement above: you can drag a point
  around the frame, or you can point it at another layer and let it find the bright spots in
  the picture (that layer arrives on one of the lists just described, alongside the optional
  lens-prescription file you can supply instead of one of the twenty built in). The third is
  **Lights mode**, where the flares come from the composition's own Light layers — and those
  are neither a number you typed nor a picture the render prepared. They are whatever lights
  happen to exist in the composition at this moment, with their positions, colours and sizes
  read off their own animation.

  That is exactly the "given the clock, what do you work out?" question described above, so
  it is answered the same way: at the moment the flare's controls are read, the lights are
  looked up and written into the same list of numbers as extra *derived* entries — two per
  light, because a colour slot holds four numbers and a light needs seven. Only the lights
  that actually exist are written, so a flare in either of the other two modes carries none
  of it. A composition with no lights at all flares with nothing, rather than falling back
  to the draggable point and putting a flare somewhere nobody asked for.

### The benchmark harness — `crates/lumit-bench` (K-389)

[13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md) makes promises with numbers in them: a
scrub shows *something* within 50 ms, the twenty-second work area caches in under a minute,
a cached comp plays at sixty. A promise nobody measures is a slogan, so this crate is the
stopwatch.

It is a **development** crate — the app never loads it, nothing depends on it, and it does
not exist in the shipped `.dll`. It has three parts:

- **The media.** Rather than commit forty megabytes of video to a public repository, it asks
  ffmpeg for the clips: two twenty-second 1080p60 H.264 test patterns and a tone, generated
  once into a scratch directory and reused after that. The `.cube` grade is written out as
  text.
- **The composition.** §1 of the performance rules describes one specific comp in a
  paragraph — two footage layers, one of them retimed to 40% with flow; a title; a Sequence
  layer of four clips; an adjustment layer with a LUT and a grade; a glow; motion blur on
  two layers; a luma matte; an audio layer with volume keyframes. That paragraph is built in
  code, layer by layer, through the same document model the editor edits. Everything about
  it is fixed — even the internal identifiers are derived from the layer names rather than
  the clock, because those identifiers feed the name a finished frame is filed under, and a
  benchmark whose frames are named differently every run is measuring a cache that never
  hits.
- **The six scenarios.** Each does one thing the editor does, with a stopwatch around it:
  jump the playhead (B3), let the frame sharpen to full quality (B4), replay a stretch that
  is already cached (B5), play from cold at reduced and at full resolution (B6, B7), and
  fill the whole work area from cold (B11). Each prints one number.

Two details decide whether those numbers mean anything. The first: drawing a frame *hands
work to the graphics card* rather than doing it, so a stopwatch stopped straight afterwards
has timed the paperwork. The latency measurements therefore wait for the card before
stopping the clock; the throughput ones deliberately do not, because real playback works
ahead of its own clock and that overlap is part of the speed. The second: **cold** means a
brand-new renderer — no compiled shaders, no open files, nothing cached — built for that
scenario and thrown away after it.

**What the numbers are compared against.** Not the budgets, most of the time. The budgets
belong to a named machine (a desktop with an RTX 3060 in it), and the robots that check
every change run on shared rented machines with no real graphics card at all, where "50 ms"
would fail for reasons that have nothing to do with Lumit. So the harness measures, and the
check compares the run with **a set of numbers recorded earlier on that same kind of
machine**, failing only when something is 1.6 times worse than it was. That catches a real
regression — those are factors, not percentages — and ignores a noisy afternoon. The
recorded numbers are a file in the repository, regenerated deliberately and committed like
any other change. Only on the reference machine itself (a switch says so) are the absolute
budgets asserted. A measurement smaller than a millisecond a frame is never failed on ratio
at all: serving an already-cached frame costs about ten *millionths* of a second, where a
factor of two is the operating system blinking.

Five budgets are missing from that list and cannot be here: how smooth the interface itself
is (B1, B2) needs the real window; export speed (B8) needs the encoder; recovering from a
graphics driver reset (B9) and audio/picture sync (B10) need a running application. Those
stay manual checks, tracked in [TODO.md](TODO.md).

**Three of the numbers time one effect rather than a whole composition.** The particle
system has budgets of its own, because its cost is a number *you* set — the Max particles
dial — rather than one the engine has to guess, and burying that inside a comp average
would hide it. So three more scenarios run the particle pass on its own at 1080p and time
it, needing no video and no composition at all.

Measuring one effect turned up a small honesty problem worth knowing about. Every effect
pass copies the picture it was handed before it draws anything on top, and at 1080p that
copy costs about a tenth of a millisecond whatever the effect then does. For a big effect
that is nothing; for "three hundred particles", which is meant to cost about two tenths, it
is a third of the reading. The budget was being missed by an amount that was mostly the
copy. There were two ways to close the gap — raise the number, or stop charging the effect
for the frame's own paperwork — and the second is the honest one, so the harness times a
fourth run with **nothing to draw** and subtracts it from all three. What is reported is
the particles' own work.

**And some of what is checked is not a stopwatch at all.** Beside the timings sits a file
of expected numbers for the particle system: where a pinned set of particles is, how fast
each is moving, and what the three drawing styles put on a small canvas. Those are compared
exactly (to a part in a hundred thousand), and unlike a timing they mean the same thing on
every machine, so they run everywhere. The reason they exist is subtle: there were already
tests holding the graphics card's answer to the processor's, but if a change moved *both*
in the same direction — a rearranged formula, a different noise pattern — those tests would
have gone on passing while the picture quietly changed. A file of expected numbers is what
notices that. It is regenerated on purpose, and the change is read before it is committed.

### Making pictures out of nothing — `crates/lumit-core/src/fx/noise.rs` (K-398)

Every effect described so far takes a picture and changes it. Four new ones don't: **Fill**
floods a layer's shape with a colour, **Gradient** paints a ramp across the frame, **Noise**
sprinkles grain over what arrived, and **Fractal noise** generates clouds. They needed
somewhere to live in the Add-effect menu, and none of the six categories described what they
do, so there is now a seventh: **Generate**.

Three of the four are small. The interesting one is Fractal noise, and the interesting part
of *it* is the word "noise", which in this context does not mean random.

**Random would be useless here.** A genuinely random pattern would be a different picture
every time it was drawn — every scrub, every preview, every export. There would be nothing
to grade against and nothing to cache. What is wanted is a pattern that *looks* random and
is in fact completely fixed: ask for the value at a given point and you always get the same
answer, on any machine, in any run, for ever. That is what "seeded" means everywhere in
Lumit, and `noise.rs` is where it is built for pictures.

It is three layers stacked, and each is simple on its own.

**One: a hash.** Imagine a grid over the frame, a hundred pixels apart. Each crossing point
gets a number, and that number is *stirred* out of its own coordinates — its x, its y, and
the effect's seed — by a few multiplications and shifts. The stirring is chaotic enough that
neighbouring crossings get unrelated numbers, which is what makes it look random; but it is
also pure arithmetic on whole numbers, so it gives the same answer everywhere, always. Whole
numbers matter here for the same reason they mattered for the lens flare's ray counts: two
computers can disagree about the last bit of a fraction, but they cannot disagree about
`3 × 7`. The graphics card runs the identical few lines and gets the identical result, which
is what lets the test compare the two down to the last representable step.

**Two: smoothing between the crossings.** A grid of unrelated numbers is a chequerboard, not
a cloud. So the value anywhere in between is blended from the crossings around it, along a
curve that flattens out as it reaches each one — that is what stops the grid showing as
creases. There are two flavours: **Value** noise blends the corner *numbers*, which is cheap
and leaves a faintly woven look, and **Perlin** noise blends corner *slopes*, which costs
more and gives the rounded, organic shape everyone pictures when they say "clouds".

**Three: layers of detail.** One layer of that is a lava lamp. Real surfaces have big shapes
with finer shapes on them and finer shapes on *those*, so the effect adds several layers,
each roughly half the size and a bit fainter than the last. **Complexity** is how many,
**Sub influence** how much each counts for, **Sub scaling** how much smaller each one is.
Six layers is a cloud. That stack of shrinking, fading copies is what "fractal" means here,
and it is the whole trick.

The grid is three-dimensional, not two, and the third dimension is what **Evolution** moves
along. You are always looking at a flat slice of a solid block of noise; turning the dial
slides the slice deeper. That is why animating Evolution makes the pattern churn and drift
like smoke instead of flickering into an unrelated pattern each frame — the slices either
side of where you are are *nearly the same picture*, because they are neighbouring cuts
through one solid thing. **Cycle** takes that further: switch it on and the block is made to
repeat every so many steps, so an animation from 0 to the cycle length ends exactly where it
began and can be looped with no seam. That works because every detail layer shares the one
depth coordinate — a decision taken so that the loop closes precisely rather than
approximately, and one of the places Lumit deliberately parts company with After Effects.

**Why it is a module of its own rather than part of the effect.** The displacement effects
coming next — Turbulent displace above all — do not draw noise, they *steer* by it: each
pixel is pulled sideways by an amount the noise decides. That is the same field, read for a
different purpose. One copy of the maths, one copy of the graphics-card version beside it,
and one test holding the two together; the alternative is two implementations that agree
right up until the afternoon somebody fixes one of them.

### A control that is a shape — the curve parameter (K-412)

Every effect control so far has been a number, a switch, a colour or a name: things a row in
the Effect controls panel can show as one widget. Curves is the first that is a **shape**.
What the user edits is a handful of points in a square, with a smooth line drawn through
them; the horizontal axis is the brightness going in and the vertical axis is the brightness
coming out, so a point dragged upwards makes everything at that brightness lighter.

Curves used to fake this. Its first version had five sliders per channel at fixed positions
along the bottom — you could pull the line up and down at 0, a quarter, a half, three
quarters and 1, but you could not put a point anywhere else, and you could not add or remove
one. That was the honest small version while there was no editor to draw a real curve in.
Now there is, so the stored form became what an editor actually edits: an ordered list of
between two and sixteen points, defaulting to the straight diagonal that changes nothing.

**Why a curve cannot be animated, when everything else can.** A keyframe works by blending
two values: half-way between 10 and 20 is 15. Two curves may not even have the same number
of points, and there is no honest answer to "half-way between a four-point curve and a
nine-point one" — which point would pair with which? After Effects has exactly this problem
and solves it the same way: its curves jump from one shape to the next rather than gliding.
So a curve is stored as a plain value with no keyframes, sitting beside the other things
that cannot blend — a chosen file, a named layer, a named mask.

**Drawing the line through the points.** The line is a *clamped cubic spline*, which is the
same family Photoshop's curve comes from — worth matching, because it is the shape every
colourist's hand already expects. Between each pair of points it is a small cubic curve; the
pieces are chosen so that the join between two of them is not merely smooth but has the same
*rate of turn* on both sides, which is what stops the line looking hinged. At the two far
ends the line is made to leave along the straight line to its neighbour, and that is what
makes a two-point curve exactly a straight line — which the default diagonal depends on
being true to the last bit.

**Working it out once instead of two million times.** A frame has millions of pixels, and
solving the spline for each of them would be absurd, so the engine solves it **once per
frame** and writes down the answer as a list of 257 numbers per channel: what the line is at
input 0, at 1/256, at 2/256, and so on to 1. Both the graphics-card version and the CPU
reference version are handed that same list and do nothing but read from it, taking a
straight-line blend between the two nearest entries. Two useful things follow. The picture
cannot differ between the two paths because of the spline, since neither of them draws it —
that is the same trick the Lightning effect uses for its bolt. And the test that holds the
two paths together is only checking the *reading*, which is four lines of arithmetic instead
of a spline solver written twice.

Two rules keep the reading honest. The line is kept inside the square, because a cubic
through rising points can bulge above the highest of them, and a tone curve that climbed
above the white you placed would put a bright halo in a highlight roll-off. But *inputs*
outside 0 to 1 are not clipped: Lumit's pictures are scene-linear, so a value brighter than
white is a real value, and it carries on along the slope of the line's last stretch rather
than being flattened.

Finally, the list is **straightened on the way in** rather than policed on the way out. A
list that arrives out of order, with points outside the square, with two points stacked at
the same horizontal position, with more than sixteen of them, or with fewer than two is
sorted, clamped, thinned and — if nothing usable is left — replaced by the plain diagonal.
It arrives from a saved document, which somebody may have edited by hand or which an
importer may have written, and a document is something to render as best one can rather than
something to reject.

### The editor that shapes the curve, and why it draws its own line (K-412)

The panel half of the same feature. Curves' five channels — Master, red, green, blue and
alpha — are five separate parameters in the schema, but they are drawn as **one square with
a row of tabs above it**, because five stacked squares would be five times as tall and would
still leave you comparing shapes across a gap. The panel already does this kind of folding
twice: a pair of parameters named `something_x` and `something_y` becomes one point row, and
a layer picker swallows its Invert switch. A run of curve parameters is the third fold.
Inside the square: drag a point to move it, click the line to add one (up to sixteen), drag a
point well clear of the square to throw it away, and Reset puts *that channel* back to the
diagonal. The two end points move like any other — sliding the black point rightwards is how
you crush an end — but they cannot be thrown away, because a line needs somewhere to start
and finish.

**The line you see is drawn twice, on purpose.** The panel works the spline out again in
Dart, purely to draw it. That looks like duplication and would be, except for what the two
copies are *for*: the engine's copy is the one that grades the frame, and the panel's copy is
a picture of it. The alternative is to ask the engine for a fresh table every time the
pointer moves a pixel, which is a trip across the Rust/Flutter boundary per frame of a drag —
the exact traffic there is a test forbidding. So the panel draws its own line, using the same
algorithm, and if the last decimal place ever disagreed nobody could see it: the plot is a
hundred and fifty pixels wide. The file says so at the top, because an undocumented second
implementation is how two copies quietly drift apart.

**One gesture trap worth writing down.** Flutter calls a free two-dimensional drag a *pan*,
and a pan has to travel about twice as far as a single-axis drag before the framework decides
that is what you meant. The Effect controls panel is a scrolling list, and a scrolling list
watches for exactly that single-axis vertical drag — so it always made up its mind first, and
every attempt to pull a curve point upwards scrolled the panel instead. The fix is to ask for
a vertical drag and a horizontal drag rather than a pan: then the inner control and the list
are competing on equal terms, and the inner one wins. The Levels handles hit the same wall
and take the same answer.

### Levels shows you the picture it is grading (K-413)

Levels' five numbers per channel are exact but blind: you can type an input black of 0.06
without knowing whether anything in the shot is that dark. So the row now draws a
**histogram** — a bar chart of how much of the frame sits at each brightness — with the input
black, gamma and white handles sitting under it, and the output range as a small bar beneath
that. Drag the black handle to where the picture actually starts and the shadows go properly
black.

Nothing about the effect changed for this. The handles write the very same parameters through
the very same path the numbered rows write through, and those rows are still there underneath;
this is a second grip on values that already existed, in the way the dial beside an angle is a
second grip on a number.

The picture itself is borrowed rather than built. Lumit already has a Scopes panel that asks
the engine to measure a frame and send back a small 256×256 image of the measurement —
waveform, vectorscope, parade, or exactly the histogram this wants. So the Levels row asks the
same question, once per frame the playhead lands on and only while the row is on screen. One
small thing had to be added for it: the reply now says *which* measurement it is. Both panels
listen on the same channel, so without that a Scopes panel left open on a waveform would paint
its waveform behind the Levels handles.

### Effects that draw nothing — the Expression controls (K-414)

Every effect so far changes the picture. These five do not, and that is the whole idea. A
**Slider control** put on a layer adds one number to that layer's Effect controls panel, and
nothing else happens: no pixel moves. What it is for is to be *read*. Any property in the
composition can carry a little piece of JavaScript — an **expression** — and that script can
say "take whatever that slider is set to". So one number, in one visible place, drives six
things at once, and animating those six things means animating the slider.

That is how nearly every downloadable effects pack is built. A pack's "intensity" dial is a
Slider control on a null layer with two dozen expressions reading it. There are five of
them, because there are five kinds of thing worth holding: a number, an angle, a tick box, a
colour and a point on the screen. After Effects has exactly these five, under exactly these
names, which is deliberate — somebody arriving with a pack in their hands should find what
they are looking for.

Four consequences follow from "it draws nothing", and each is a small piece of code that is
*absent* rather than present. There is no graphics-card program for these effects and no CPU
version either, so there is nothing for the usual "do both paths agree?" test to compare —
the first effects in Lumit for which that is true. The engine's render walk skips them
entirely, so they cost nothing per frame. They have no **Mix** dial, because Mix means
"fade back towards the untouched picture" and there is no touched picture. And they refuse
the **Matte** input every other effect carries, because a matte is a picture that decides
where an effect applies, and an effect that applies nowhere cannot be told where.

### A number that cannot leave its range — the closed slider (K-414)

Most numbers in Lumit have a slider that is a *suggestion*: drag within the range, or type
past it if you know what you want. A blur radius works that way, and so does Temperature —
its slider stops at 150 but the number is allowed as far as 200, because there is a real
picture out there.

Some numbers have nothing outside their range at all. A wipe's **Completion** goes from not
begun to complete; 150% complete is not a picture, it is a typing mistake. Those parameters
now say so, in one word on the declaration, and what they get is a track and a thumb with no
way to leave the ends.

The part worth understanding is what does *not* change when a parameter adopts it. The
stored number is the same kind of number it always was, so old projects load unchanged, the
keyframes are the same keyframes, the graph editor still draws it, and an expression still
reads it. Only the control is different. That is the same trick the angle dial uses: the
value is a plain number of degrees, and "angle" only says which knob to draw for it.

### Moving pixels instead of recolouring them — the distort family

Most effects answer the question "what colour should this pixel be?" by doing arithmetic on
the colour that was already there. The distort effects — **Turbulent displace**,
**Tile**, **Offset**, **Mirror**, **Lens distort**, and the five in the section after this
one — answer a different question:
*where should this pixel fetch its colour from?* The colour itself is never touched. That
one difference is worth understanding, because it explains everything the family has in
common, including the things about it that look like quirks.

**They all work backwards.** It is tempting to picture a warp as pushing pixels around, like
smearing wet paint. That is not how it is done, and it could not be: pushing leaves gaps
where nothing landed and pile-ups where several things did. Instead the effect walks the
*output* — every pixel of the finished frame, one at a time — and for each one works out
which spot in the *input* it should read. Every output pixel gets exactly one answer, so
there are never gaps and never pile-ups. A Mirror does not fling the left half across the
frame; it says "if I am on the right, read from the mirrored spot on the left".

**The spot they land on is usually between pixels**, and that is where **bilinear sampling**
comes in. If the answer is "read from 12.4 pixels across and 7.8 down", there is no such
pixel — so the four surrounding pixels are mixed in proportion to how close each one is.
This is why a distortion very slightly softens the picture, and why moving by a whole number
of pixels does not: a whole-number move lands exactly on a pixel and the mix picks it alone.

**Sometimes the answer is outside the frame**, and each effect has to say what that means.
Offset wraps — what leaves one side arrives at the other, because it treats the frame as a
loop. Mirror gives transparency, because a reflection with nothing to reflect genuinely has
nothing there. Lens distort offers the choice (transparent, hold the border pixel outward,
or fold the picture back), because which one is right depends on the shot. Turbulent
displace avoids the question near any edge you have *pinned*: it fades its push to nothing
over a band exactly as wide as the push could reach, so the border cannot be pulled in from
outside in the first place.

**Turbulent displace is the one that reads the noise.** Its "where should I read from?" is
answered by the pattern described in the previous section: two fields of it, one deciding
how far to move sideways and one how far up or down. That is why the noise had to be a
module rather than part of the Fractal noise effect — the generator draws the field, the
displacer steers by it, and if the two ever disagreed about what the field *is*, using them
together would stop working.

**One thing that trips people up.** Every effect ends with a **Mix**, which blends the
finished result back towards the picture that arrived. On a colour effect that reads as
"less of the effect". On a warp it does not: blending a warped picture with an unwarped one
shows *both*, so every edge appears twice, like a double exposure. To warp less, turn the
Amount down. To warp less **in some places and not others**, use the Matte row — on
Turbulent displace, uniquely in this family, the matte scales the push itself rather than
fading the result, so a grey area is genuinely warped a little rather than being a full warp
shown faintly.

### Perspective, borrowed maps and circles — five more ways to move a pixel

Everything in the previous section applies unchanged to **Corner pin**, **Displacement
map**, **Polar coordinates**, **Twirl** and **Spherize**: they all walk the output and ask
where to read from, they all land between pixels and mix four of them, and they all have to
say what "outside the frame" means. What is new is the *shape of the answer*, and three of
those shapes are worth understanding.

**Corner pin does perspective, which is not the same as stretching.** Drag the four corners
of a layer onto the four corners of a phone screen in a shot and the picture has to narrow
where the screen recedes. Stretching cannot do that. What can is a **homography** — the
transform a real camera performs: straight lines stay straight, but parallel lines meet.
It needs eight numbers, and four points *are* eight numbers, which is why the effect stores
the points and works the matrix out at render time rather than the other way round.

Two consequences fall straight out of the maths. Because parallel lines meet, there is a
**horizon** — a line in the output beyond which the surface you described has folded away
behind the camera. Lumit draws nothing there. Some tools draw a mirrored copy of the picture
instead, which is the arithmetic quietly running past the point where it means anything.
And because a flat surface needs four corners to be a surface at all, putting three of them
in a line describes nothing: the effect renders the picture untouched rather than dividing
by zero, which is the house rule for every impossible setting (degrade, never fault).

**Displacement map borrows its warp from another layer.** Turbulent displace invents its
push out of a noise field it draws itself. Displacement map has no field of its own: you
point the **Matte** row at a layer, and *that layer is the map*. One channel of it says how
far to push sideways, another how far up and down.

The number that matters is the neutral: **mid-grey means "do not move"**. Full white pushes
the whole Amount one way, full black the whole Amount the other, and 50 % grey nothing at
all. That is what lets a single picture push in both directions, and it is why a map that
is mostly dark shoves everything one way — grade it around the middle and the warp moves
*about* its rest position instead of away from it. This is also the second effect in the
catalogue whose Matte row is the subject of the effect rather than a knob on it: Set matte's
matte is the alpha, this one's is the map.

**Polar coordinates bends the frame round its own middle.** In one direction the picture's
rows become rings: the top row shrinks to a point at the centre, the bottom row becomes the
outer ring, and the left and right edges meet in a seam pointing straight up. That is the
"tiny planet". The other direction is the exact opposite and unrolls a circle into a strip.
Two things follow. The centre is where detail dies — a whole row of the picture is squeezed
onto a handful of pixels there — so whatever you put along the top of the source is what the
middle of the planet will be made of. And its **Interpolation** control is a morph, not a
fade: at 50 the frame is genuinely half bent, every pixel drawn from half-way along its own
journey, which is quite different from laying a finished bend over the flat picture at half
opacity. (The ordinary Mix does the second thing, and it is occasionally what you want.)

**Twirl and Spherize are both bounded circles**, and that is the useful property. A twirl
turns the picture about a point, hardest in the middle, easing to exactly nothing at the
rim — so it can only ever rearrange what is already inside its own circle, and the rest of
the frame is untouched to the last bit. Spherize magnifies inside its circle the way a glass
marble does, which means the middle swells and everything it pushed aside is crowded into
the last few pixels before the edge. Its **Bulge** control turned negative pinches instead,
and the two are genuine opposites: a bulge and a pinch of the same strength cancel, which is
not true of most effects that offer a minus sign.

**One implementation trap, recorded because it cost an afternoon.** Every one of these
kernels contains the same small helper: "fetch the pixel at (x, y), or transparent if that
is outside the frame". The obvious way to write it is to check the coordinate and return
early. That reads correctly and is wrong: fetching a picture has no side effects, so the
compiler is free to move the fetch *above* the check and use the result afterwards — and an
out-of-range fetch on at least one Windows graphics driver comes back with a live value in
its alpha. The visible symptom is bizarre: a pixel that should be empty arrives opaque, with
its colour still correctly black. The fix is to clamp the coordinate into the frame first,
fetch that, and *then* choose between it and transparent — no fetch is ever out of range, so
there is nothing to be undefined.

### Waves, presets, and a warp that has to be solved

Five more effects move pixels — **Ripple**, **Wave warp**, **Bezier warp**, **Warp** and
**Roughen edges** — and between them they raise three ideas the catalogue had not needed
before.

**There is no speed control, anywhere, and that is a rule rather than an omission.** After
Effects' Ripple and Wave Warp both have a "Wave Speed", which means cycles per second and is
read off the clock. Lumit cannot have one. An effect that reads the clock renders a
different picture depending on *when* it was asked, so the preview and the export disagree,
two machines exporting the same project disagree, and a frame pulled out of the cache is
wrong. Both effects instead have an angle — Evolution, or Phase — that the timeline
animates, and one full turn is one whole wave. It is the same motion with the stopwatch in
the user's hand instead of the computer's, and an imported AE project converts the speed
into two keyframes automatically.

**Ripple's rings die at the middle as well as at the rim, and there is a reason.** Every
pixel in a ripple is pushed *along its own radius*, away from or toward the centre. The
pixel exactly at the centre has no radius and therefore no direction, so an effect that
pushed it anyway would have to pick one arbitrarily — and the result is a permanent pinched
blob at the epicentre. Shaping the strength so that it starts at nothing, peaks about a
third of the way out and falls back to nothing at the rim removes the problem exactly, and
is also what a real spreading wave looks like: the crests are strongest in a ring, not at
the point the stone went in.

**Warp is thirteen named bends behind one slider, and it hides a sign trap worth knowing.**
Every one of these kernels is a *gather*: it walks the output and asks where to read from.
So if a style wants the middle of the picture to swell outward, it has to read from
*further in*, not further out. Written the natural-looking way round, a style called Bulge
pinches at a positive Bend — which is the one thing a preset called Bulge may not do. The
five swelling styles therefore subtract their coefficient where the bending ones add theirs,
and the tests check that every style at Bend 0 returns the picture untouched to the last
bit.

**Bezier warp is the first effect in Lumit that has to solve an equation per pixel.** Its
twelve points — four corners and eight handles — bend the frame's four edges into curves,
and the inside is filled by a **Coons patch**: a surface defined by its own boundary, made
by blending the two horizontal edges vertically, blending the two vertical edges
horizontally, adding the results and subtracting the flat surface on the corners that the
two blends between them counted twice. That gives a formula for "where does source point
(u, v) end up?" — and rendering needs the opposite, "which source point ended up *here*?",
which the formula cannot be rearranged to answer.

So each pixel searches for it, by **Newton's method**: start from a guess, work out how far
wrong the guess is and which way the surface is sloping there, step, repeat. On a warp of
any ordinary size a couple of steps is enough, and the effect's **Quality** control is
simply how many steps it takes. (After Effects' Quality on the same effect means something
else — how finely it chops the patch into triangles — but "higher is more accurate" is true
of both, so a project converts without anyone noticing.)

Two things about a solver that a formula never needs. It can **fail to converge**, and it
must be told to stop: a patch dragged so far that it folds over itself has no single answer,
so the search gives up rather than dividing by zero. And its answer has to be **checked**.
Outside the bent shape there is no answer at all, and an unchecked search wanders about
until it happens to land in range — which draws a scatter of stray, wrong pixels across the
empty part of the frame. It looked like a driver bug and it was not; the fix is one more
evaluation at the end, asking "does this answer actually solve the problem?", and throwing
it away when it does not.

**Roughen edges reuses a blur as a distance field, which is the batch's best trick.** To
chew forty pixels off the outline of a shape you need to know, for every pixel, how far it
is from that outline. Computing that properly is a whole algorithm — a *distance transform*
— with its own passes and its own edge cases. But blurring the shape by forty pixels gives
the same information for nothing: the half-way contour of a blurred alpha sits exactly where
the original edge was, and the ramp either side of it is forty pixels wide. So the effect
blurs, then re-cuts the blurred coverage at a threshold that a noise field wobbles per
pixel. Drop shadow already reuses the same shipped blur for its softening; this is the
second time one kernel has paid for another.

The one thing the noise has to be told is *where it is allowed to act*. Deep inside a solid
layer the blurred coverage is 1, and a noise value low enough would otherwise punch a hole
in the middle of the shape. Weighting the wobble by how close the pixel is to the outline —
strongest on it, zero well inside or well outside — confines the chewing to a band, which is
both correct and exactly what the Border control ought to mean.

### Deciding how much of a pixel there is — coverage, and the wipes (K-400)

Every effect so far has answered "what colour is this pixel?" or "where does this pixel read
from?". Five more answer a third question: **how much of this pixel is there at all?** That
number is the alpha — the coverage — and it is what decides whether you can see through
something.

**Drop shadow** is the familiar one, and it is made entirely of coverage. The layer's alpha
already carries its shape: where it is solid, where its edge softens, every wisp of
antialiasing. Blur that shape, slide the blurred copy in the direction the light comes from,
paint it one colour, and draw it *behind* the layer. That "behind" is the whole reason it is
an effect rather than a duplicated layer, and it is why the effect can never make a shadow
on footage that fills the frame — an opaque rectangle has no shape to cast.

One small thing in it is worth knowing because it saves half the work. Blurring a picture
and then sliding it gives exactly the same result as sliding it and then blurring it: the
two operations do not care which order they happen in. So the effect softens the shape where
it stands and then simply *reads* the softened version at the offset position. One blur
instead of a blur and a resample, and not an approximation — the same picture.

**Set matte** is the one worth slowing down for, because it changes what the Matte row
means. Every effect has that row: pick a layer, and its brightness drives the effect. For
almost all of them "drives" means strength — the effect runs everywhere and is then faded
back towards the untouched picture where the matte is dark. Set matte's matte is not a
strength. It **is** the alpha: whichever channel you choose out of that layer becomes this
layer's transparency, so a title takes the shape of a cloud, or a fill takes the shape of a
ramp.

The difference is easy to see and impossible to fake. Point Set matte at a white disc on
black. The effect gives you a disc-shaped picture. A strength dissolve at the same matte
gives you the *whole frame*, with a faint ring where the disc's soft edge is — because
inside the disc it shows the effect (which is the picture) and outside it shows the input
(which is also the picture), and only the halfway ring differs at all. Six effects in the
catalogue claim their matte this way instead of taking the dissolve; this is the first for
which the matte is the answer rather than a knob on the answer.

**Channel blur** is the ordinary blur with one radius per channel. It earns its place
because a real camera does not resolve the three colours equally — blue is always the worst
— so softening blue alone reads as a lens rather than as a blur, which is why blue is the
one channel switched on by default. It is also how you feather a cut-out's edge without
touching the colour inside it: turn up Alpha and leave the rest at zero.

**Linear wipe and Radial wipe** are transitions, and they got a menu family of their own
because there was nowhere honest to put them. A wipe adds nothing to the picture, so it is
not a stylisation; it is not a tool either. It is the thing an editor means by the word
*transition*: keyframe **Completion** from 0 to 100 and the layer leaves the frame along a
straight edge or around a clock face, and a cut has been made out of a movement.

Two details in them are the kind of thing that only shows up on a real project. **The edge
travels slightly past each end of the frame**, by half the width of the feather. Without
that, a soft wipe at Completion 0 would already be nibbling at the far corner and at 100 it
would leave a faint ghost — so the wipe would neither start clean nor finish. And on the
radial one, **the feather is measured across the arc**, so the soft edge stays the same
thickness as the hand sweeps outwards instead of fanning open. Right at the centre that
becomes impossible (a fixed width covers the whole circle when the circle is a point), so
it is capped there, and a few pixels in the middle of a heavily feathered radial wipe are
mush. That is true of every radial wipe in every application; it is geometry, not a bug.

### Where a tone control should sit, and what "local" means (K-404)

Six more effects change colour rather than position — **Posterize**, **Threshold**,
**Tritone**, **Photo filter**, **Black and white** and **Shadow highlight** — and between
them they raise two ideas the catalogue had managed without until now.

**The picture is measured in photons, and a person is not, so a control has to be told
which of the two it lives in.** Lumit works in scene-linear light (see the colour section
above): a value of 1.0 is white, and 0.5 is *not* the grey halfway between white and black —
the grey a person points at is nearer 0.25. That is fine while an effect is doing
arithmetic, because light is the honest space to multiply in. It stops being fine the moment
a control asks a *question about where the middle is*.

Four controls in this batch do exactly that. Posterize is told how many shades to keep, and
has to decide where to put them; Threshold is told a brightness and has to cut there;
Tritone is told what colour the midtones should be; Shadow highlight has to know what a
midtone is before it can add contrast to one. Put those evenly in light, and eight
posterize bands land with six of them crowded into the highlights and the shadows left
almost smooth — the opposite of what the effect is for. So all four run their control
through one small conversion that stretches the dark end out, and the rule that came out of
it is worth remembering: **do the maths in light, but put the knob where the eye is.**

**The conversion is a square root, and the reason is the tests rather than the picture.**
Any reasonable curve would place the bands acceptably — the usual one in this trade is a
power of 1/2.2. The problem is that Posterize's answer is a *step*: a value either lands on
this band or the next one, with nothing in between. Every effect in Lumit is checked by
running the same maths on the processor and on the graphics card and demanding they agree
(see the testing section). Two implementations of a power function disagree in the last bit
or two, which for most effects is invisible — but for a step, "the last bit" is the
difference between one band and the next, and the test would fail at random on whichever
pixels happened to sit on a boundary. A square root is a single instruction that both the
processor and the card are required to get *exactly* right, so they cannot disagree at all.
Between a curve of 2.0 and one of 2.2 there is no visible difference in where eight bands
land; between an exact answer and an approximate one there is a flickering test. **Pick the
formula your test can prove.**

**Shadow highlight is the first effect here that asks about a pixel's neighbours, and the
way it does it is worth knowing.** The job is the backlit interview: a face against a
window, where the camera exposed for the window and the face went black. Brightening the
whole picture blows the window out. What is needed is to brighten the *dark regions* only —
and the word doing the work is *regions*.

The trick is that it blurs the picture, and then throws almost all of the blurred picture
away. It keeps one number per pixel: how bright the neighbourhood is. That number decides
*whether this pixel counts as a shadow*, and nothing else — the colour that comes out is
always the pixel's own, multiplied by a gain. The blur is a question, not an answer.

Two things follow. Nothing is softened, because no colour was ever borrowed from the blurred
copy; the picture comes out exactly as sharp as it went in. And a white shirt button inside
a dark jacket is brightened *along with the jacket*, instead of being singled out and left
behind, because the question asked about the button was really a question about the jacket.
That is the whole of what "local adaptive" means, and it is why the result looks like a
better exposure rather than a washed-out one. The one artefact to watch for is a bright halo
around a dark object on a bright background, which is what happens when the neighbourhood is
too small or the lift too strong; the shipped defaults are deliberately mild.

**Two smaller things from the same batch.** Black and white's six sliders — how bright should
reds be, and yellows, and so on — work by splitting every colour *exactly* into a grey plus a
bit of one colour plus a bit of another, rather than by guessing which slider a pixel most
belongs to. That sounds like a detail and is not: it means a grey pixel has nothing for the
sliders to act on, so a shot full of neutral does not drift while you tune one colour, and
two neighbouring colours on a gradient hand over without a seam. And After Effects'
Shadow/Highlight has an "Auto Amounts" that reads the frame's own histogram and then smooths
that reading over neighbouring frames; Lumit deliberately has no such thing, because it makes
a grade whose answer at one frame depends on the shot around it — which cannot be judged on a
still and cannot be scrubbed backwards. It is the same reason there is no Wave Speed
anywhere: a control that cannot give the same answer twice is one Lumit would rather not
have.

### Choosing a number instead of working one out (K-405)

Almost every effect Lumit has *computes* its answer: take the pixel, multiply,
add, done. Six more arrived together — **Median**, **Mosaic**, **Find edges**,
**Emboss**, **Texturize** and **Broadcast safe** — and the first of them does
something none of the others do. It **chooses**.

Median replaces each pixel with the *middle* value of the little square of
pixels around it: line the neighbours up in order and take the one in the
centre. That one idea does something no blur can. A stray white speck has no
neighbours agreeing with it, so it never gets near the middle of the queue and
vanishes completely — while a real edge has half its square on each side, so the
middle value is still on the correct side of it and the edge stays exactly where
it was. Blur softens everything, including the edges; median removes the specks
and leaves the edges alone. Turned up, it stops looking like a repair and starts
looking like paint.

**Why choosing is harder than computing, on a graphics card.** The obvious way to
find a middle value is a *sort*, or its clever cousin the quickselect: compare
two numbers, decide what to do next based on which was bigger, repeat. That is
fine on a processor and wrong here, for two reasons. A graphics card runs dozens
of pixels in lockstep, and a program that takes a different turning for each of
them makes every pixel wait for every other. Worse — and this is the reason that
actually settled it — Lumit checks each effect by running the same maths on the
processor and on the card and demanding the two agree (see the testing section).
A method that *decides what to compare next based on what it just saw* does not
have to make the same decisions on both, and two different orders of comparison
can land on two different pixels of the window. One pixel apart is a visibly
different answer.

So both paths run the same fixed dance instead. Walk the square once, carrying a
short sorted list of the smallest values seen so far; for each new neighbour,
push it along that list swapping wherever it is smaller than what is there. No
step of that depends on a value — it is the same sequence of comparisons for
every pixel in every frame — and the two halves of it, "the smaller of these
two" and "the larger of these two", are the two operations a computer is
required to get *exactly* right. So the processor and the card cannot disagree,
even though (as it happens) they do not even walk the same square: the card
always walks the largest one and pads the corners it does not want with a number
bigger than any pixel, because those can never reach the middle. The general
lesson is worth carrying: **when an effect has to pick rather than calculate,
write the picking as a fixed sequence of exact steps, never as a search.**

**The cost, and a slider that genuinely stops.** That dance costs roughly the
fourth power of the radius — forty-five comparisons a pixel at radius 1, three
hundred at 2, twelve hundred at 3, seventeen thousand at 6. After Effects lets
its Median radius go to 50, which here would be a quarter of a million
comparisons for every pixel of every frame. Lumit's stops at 3, and it is the one
control in the whole catalogue that *cannot be typed past*. Everywhere else a
slider is a suggestion and a number you type is obeyed; here the honest thing is
to refuse, because a control that quietly gives you radius 3 when you asked for
30 has answered a different question from the one you asked.

**Two smaller ideas from the same six.** Mosaic cuts the frame into blocks, and
which block a pixel belongs to has to be decided by *whole-number* arithmetic
rather than by dividing its position by the block width — because a division that
comes out exactly on a boundary can round the other way on the card than on the
processor, and a whole block of colour changes. And Find edges and Emboss both
work by comparing a pixel with its neighbours, which they do on the *perceptual*
brightness rather than the raw light, for the reason the section above gives: in
light, the step from a bright sky to a brighter one is a bigger number than the
step from a shadow to a slightly-lit shadow, though the eye sees the second and
not the first. Compare in light and you draw the highlights; compare
perceptually and you draw what a person would draw.

**And one that is about television.** Broadcast safe exists because analogue
television carried brightness and colour along a single wire, added together,
and a transmitter would distort a signal whose total swung too far. A saturated
yellow is the classic offender: its brightness is nearly white already, and its
colour adds a large amount on top, so the two together reach about a third again
past the legal peak — even though neither half is unusual on its own. The effect
measures that total for every pixel and either pulls the pixel down until it
fits, drains colour out of it until it fits, or simply shows you which pixels
the problem is in. It is a delivery tool rather than a look, which is why it sits
under Utility with Set matte rather than with the colour effects.

### Reading a picture instead of drawing one (K-406)

Three more transitions arrived together — **Venetian blinds**, **Iris wipe** and
**Card wipe** — and the last of them raises the biggest single idea in how
Lumit's effects are built. It is worth the detour, because once you have it, a
lot of the catalogue stops looking clever and starts looking obvious.

**Venetian blinds first, because it is the easy one.** It is Linear wipe — one
straight edge swept across the frame — with a single line added: before the edge
is applied, the distance across the frame is *folded* into one slat. Measure how
far a pixel is along the sweep, then throw away everything but the remainder when
you divide by the slat width. Now every slat sees the same little number, and one
edge becomes a rank of them. Everything Linear wipe already got right (the clean
start and finish, the direction convention, the feather) comes along untouched.
That is the cheapest kind of new effect there is: an old one, folded.

**Iris wipe is the same trick in a circle, and it saves an enormous amount of
work.** The effect cuts a polygon-shaped hole — six sides by default, up to
thirty-two, and with one switch every other corner pulls inwards and it becomes a
star. The obvious way to build that is to *draw* the polygon: work out its
corners, walk its edges, fill the inside. Nobody does that here. A polygon is the
same wedge repeated round a circle, so the effect takes the pixel's angle, folds
it into one wedge, and then mirrors it about that wedge's own centre line. What
is left of the entire boundary — six sides or sixty-four, points or no points —
is **one straight edge**, and how far a pixel is from a straight edge is a single
multiply-and-add. A star is the identical calculation with the second corner put
somewhere else, so the switch costs nothing at all while the effect is running.

There is a bonus in that, and it is the sort of thing worth noticing. Because the
answer is a real perpendicular distance measured in pixels, the Feather control
can be a *width* — the same thickness all the way round, including inside the
crooks of a star's points. Radial wipe, which measures in angles instead, has to
fight that battle and only half wins it (see the note about mush in its own
section). Choosing the right thing to measure made a control's problem disappear
rather than be solved.

**Card wipe: why Lumit reads pictures instead of drawing them.** The effect cuts
the frame into a grid of cards and turns each one edge-on until it vanishes, in a
wave crossing the frame. It genuinely turns: the near edge grows as it swings
towards you, the far edge shrinks, and the picture printed on the card slides
across it. That is perspective, and it is the first time anything in Lumit has
put a camera in front of a pixel.

Here is the part that matters. There are two ways to put a rotated rectangle on
screen. The way every 3D program does it is to **scatter**: take the rectangle's
four corners, work out where each one lands, and paint the pixels in between.
Lumit's effects never do that. They **gather** — every output pixel asks one
question, *where should I read from?*, and answers it by itself, knowing nothing
about any other pixel. Gathering is why effects run at full speed on the graphics
card (a thousand pixels can ask their question simultaneously without ever
tripping over each other), and it is why the processor and the card can be held to
producing bit-identical answers.

So Card wipe cannot draw a card. It has to run the perspective **backwards**:
given that I am standing here on screen, which point of the flat card is in front
of me? For a rotation about one axis with a camera in front of it, the forward
formula turns out to be a fraction — one thing divided by another — and fractions
of that shape can be turned inside out with school algebra. The inverse is one
division. That single fact is the whole reason a card wipe is a cheap one-pass
effect and not a rendering pipeline, and the general lesson has been written down:
**before you reach for drawing, check whether the formula can simply be turned
around.** (Bezier warp, elsewhere in the catalogue, is the case where it cannot be
— and that effect has to *solve* for its answer by repeated guessing, which is a
great deal more work.)

**What Card wipe deliberately does not have.** After Effects' version carries a
whole camera system: camera position, corner pins, lights, materials, jitters —
all of it there so you can look at the grid from an angle. Lumit keeps cameras on
the composition rather than inside an effect, so every card here is drawn in its
own little frame from one fixed viewing distance. The choice of what to keep was
made with one test: **is it still visible?** Perspective stayed in because without
it the Flip direction control would do nothing at all — a card that merely
squashed would look identical whichever way it turned — and a control that does
nothing is a defect with a name on it. The camera controls went because Lumit
cannot honour them, and the honest conversion of a control you cannot honour is
to not have it and say so. That is the same ruling as the missing Wave Speed and
the missing Auto Amounts elsewhere in these notes, for the third time.

**And one small thing that is easy to get wrong.** A card is gone when it has
turned a quarter turn, and the cosine of a quarter turn is zero — except that in a
computer's arithmetic it is 0.00000006, which is not zero. Trusting the
trigonometry would leave a hairline of faint pixels down the spine of every card
at the end of the transition. So both the processor and the card *test* for the
two ends of the range instead: not started, pass the pixel through untouched;
finished, clear it. The general form of that is worth carrying: **when an
animation has to land exactly on nothing, check for the end rather than
calculating your way to it.**

### Doing the random part once (K-407)

Five more effects landed together — **Beam**, **Lightning**, **Radio waves**,
**Vegas** and **Add grain** — and the batch contains an idea you will meet
again.

**Where work should happen: once a frame, or once a pixel.** A lightning bolt is
built by taking a straight line, pushing its middle sideways by a random amount,
then pushing the middle of each new half sideways, and so on — a few rounds of
that and you have the jagged shape everyone recognises. The question is *where*
that gets worked out.

The obvious answer is "in the graphics card's program", because that is where
effects live. But that program runs once for every pixel on the screen — two
million times for a 1080p frame — and every one of those two million runs would
be rebuilding the *same bolt* just so it could measure its own distance to it.
Two million identical calculations, thrown away 1,999,999 times.

So Lightning does it the other way round. The bolt is worked out **once a frame**,
in ordinary Rust, into a list of at most 192 straight segments; that list is
handed to the graphics card, which then only has to answer a very cheap question
per pixel: how far am I from the nearest of these segments? A few hundred
multiplications replace a few hundred million.

There is a second prize, and it is arguably the bigger one. Every effect in
Lumit is written twice — once in Rust as the reference, once as a graphics-card
program — and a test holds the two to agreement pixel by pixel. Normally that
test has to check the whole effect. Here it only has to check the *drawing*,
because both versions are handed the **identical list of segments**: there is no
second copy of the bolt-building code that could quietly disagree with the first.
The rule to carry, which is now written down as a decision: **if the random thing
does not change from pixel to pixel, it does not belong in the pixel program.**

**A clock is a bug; a control is not.** Radio waves throws out a ring several
times a second, each one expanding and fading. After Effects does that by asking
what time it is. Lumit is not allowed to: an effect that reads a clock gives a
different answer on a preview than on an export, and a cached frame becomes a
lie. So Lumit's version has a **Time** control — a plain number of seconds you
keyframe — and everything else is measured against it. Keyframe it in a straight
line and you get exactly After Effects' behaviour. But you can also slow the line
down, hold it to freeze every ring in mid-flight, or run it backwards; a rate
cannot do any of that. This is the third effect where a missing rate turned out
to be the faithful conversion, and the first where the replacement can do more
than the thing it replaced.

**Finding a line in a picture, and then running lights along it.** Vegas outlines
whatever is in the frame and marches dashes along the outline. The interesting
part is what "the outline" means. The obvious approach — measure how fast the
picture is changing and light up wherever it changes fast — gives a line whose
*thickness* depends on how sharp the edge happened to be, so a Width control
would do nothing on a soft edge and everything on a hard one. Instead Vegas asks
a different question: how far, **in pixels**, is this pixel from the place where
the brightness crosses the level you chose? That number comes out of one
division, it is a real distance, and Width is therefore a real width. It also
switches the effect off by itself where the picture is flat, because "how far to
a level that never arrives" is infinity.

Two small practicalities came out of building it, both the sort of thing that
only shows up on real footage. The direction of the outline is measured over a
5×5 neighbourhood rather than a 3×3 one, because on compressed video a small
neighbourhood points a different way in almost every pixel and the dashes come
out as speckle rather than as a line. And the dashes are counted from the middle
of the frame rather than from its corner, because a small error in the outline's
direction gets multiplied by how far away you are — so halving the distance
halves the wobble, for free.

### Turning an After Effects project into a Lumit one (K-410)

Elsewhere in this chapter there is the `crates/lumit-import/` entry, which explains
the two halves of an After Effects import: a script inside AE that writes down what
it sees, and a Rust crate that does the translating. `crates/lumit-import/src/map/`
is the second half — the one that reads the walk and builds an actual Lumit project
out of it.

Two rules shape everything in there.

**An import never fails.** Not for an item the script could not identify, not for a
composition whose settings never arrived, not for a layer pointing at footage that
has been deleted. Every one of those becomes a line in a **report** and the import
carries on, because a person who has just waited for a two-hundred-composition
project to convert is not helped by an error message. The only way to lose something
without anybody noticing is to not write it down, so everything that changed on the
way across is written down. The report has four grades, which are the ones docs/11
names: *imported* (came across whole), *adjusted* (came across with a documented
difference), *placeholder* (kept whole but rendering nothing), and *skipped* (could
not be represented, and named here rather than lost). The summary line at the top of
the panel is counted from the list beneath it, so the two can never disagree.

**Import makes a new project.** Merging an After Effects project into one that is
already open is a later piece of work; today the answer is always a fresh document.
That is why every After Effects id becomes a brand-new Lumit id rather than trying
to match anything that already exists.

Four translations in there are worth knowing about, because each is a place where a
mistake would be invisible rather than loud.

**Time.** After Effects hands times over as ordinary decimal seconds — `2.04` for
the moment a person calls "frame 51". Lumit stores exact fractions, so a walk of ten
thousand frames lands on the same moment however you got there. So every time is
read back as the fraction it was meant to be: within a millionth of a frame it *is*
that frame, and otherwise it is kept to the nearest thousandth of a frame. A key that
sits deliberately between two frames is never nudged onto one, because that would
quietly re-time somebody's animation. The same care recovers frame rates: `23.976`
is written back as 24000/1001, since a project storing the decimal drifts a whole
frame every twenty minutes.

**Which clock a keyframe is on.** After Effects reports a layer's keyframe times on
the *composition's* clock. Lumit stores them on the layer's own, which starts wherever
the layer was dragged to. Subtracting one from the other happens in exactly one place
(`Conv::layer_time`), because a layer moved two seconds down the timeline would
otherwise import with its animation two seconds into the future — and it would look
fine until you scrubbed.

**The two things AE calls time.** A layer "stretched" to 50% plays at double speed,
and a layer with "time remapping" has a graph saying which moment of its source to
show. Lumit has one system for both — **Retime** — so both become one. The stretch
becomes the straight line that plays the source at that rate, and a *negative*
stretch becomes the same line reflected: the layer opens on the last frame and walks
back to the first. Time remapping needs no translating at all, because AE's graph and
Lumit's Retime graph are literally the same mathematical object; a hold key on it *is*
a freeze, with nothing having to convert it.

**Effects that Lumit does not have.** Rather than guessing at the closest Lumit
effect — which is how an import quietly produces a picture nobody asked for — an
unrecognised effect becomes a **placeholder**: it keeps its name, its on/off state
and every one of its parameters as real Lumit properties that animate and appear in
the graph editor, and it renders nothing. It never disappears, and it survives being
saved and reopened, so the day Lumit learns that effect the data is still there. The
mapping table that will recognise the effects Lumit *does* have plugs into a single
function (`map_effect`); until it exists, everything takes the placeholder road, which
is the honest answer rather than a hopeful one.

Those effects come with a wrinkle in the report. After Effects' own scripting cannot
read a lot of what a third-party effect stores — the settings are kept in a private
blob it will not hand over — so the import keeps the blob whole and writes a line
saying it could not be read. On a project built with Particular or Sapphire that is
dozens of lines for one effect and thousands for the project, which drowns the lines
that actually need a person: the blend mode that had to change, the expression to look
at. So they are counted instead: one line per effect saying how many of its parameters
came across unread. Nothing is dropped, and one lone unreadable parameter still gets
its name said, because there is nothing to drown it out.

Anything After Effects knew that Lumit has no field for — its own item numbers, its
renderer's name, a footage item's interpretation settings, a layer's stretch
percentage — is parked in an **`ae` namespace** on whichever object it belonged to.
Lumit's own file format carries fields it does not understand through a save
untouched, so that data survives indefinitely and a later version can pick it up.

### Agreeing with other programs about what a colour means — `crates/lumit-colour` (K-489, K-490)

Two editors can both be told "this shot is ACEScct" and still show different pictures,
unless they agree on what that name *means*. **OpenColorIO** is the standard that
settles it: a studio publishes a **config** — one text file naming colour spaces and a
folder of look-up tables it points at — and every serious program reads the same file.
Load one into Lumit and the config's own names appear wherever Lumit already asks a
colour question: what a piece of footage arrived as, what the Viewer is showing it
through, what an export writes.

The official implementation is a C++ library, and Lumit deliberately does not use it.
The reason is the promise the whole application is built on: the preview *is* the
export, pixel for pixel. That library computes one way on the processor and another way
on the graphics card, by design — good enough for film work, but two answers cannot be
one answer. So Lumit reads the format itself, exactly as it hosts OpenFX plugins itself
and reads `.cube` files itself.

The trick that makes it affordable is **baking**. A config describes a transform as a
recipe: multiply by this matrix, take that logarithm, look this up in a table. Running
that recipe on every pixel of every frame would be hopeless, so it is run **once**, on
the processor, over a grid of sample colours, and the answers are kept in a small
table. That table is the *artefact*, and both the Viewer and the export sample the same
one — not two implementations that agree, one dispatch used twice.

There are two shapes of artefact, and which one a transform gets is decided
mechanically. If every step in the recipe treats red, green and blue separately, plus
matrices, the artefact is just a **sampled curve and a matrix** — exact, and still
correct on values outside the sampled range because the original steps are kept beside
the table and used out there. Camera input transforms are this shape by construction.
Everything else — a display's view, anything that mixes the channels — gets a
**shaper and a cube**: scene light has no top end, so a logarithmic squash (the
*shaper*) brings it into 0–1, and a 65×65×65 grid holds the answers. That form has an
honest cost, written down rather than hidden: what the shaper cannot reach, the cube
clamps, and a very bright, very saturated colour is read off a coarse part of the grid.
In-gamut material never notices; wide-gamut material under a config that does its own
gamut mapping can.

The part worth understanding as a *policy* rather than a mechanism is what happens when
a config asks for something Lumit has not implemented. It is **refused by name** —
"this config needs `FixedFunctionTransform`, which Lumit does not support yet" — and
never approximated. Writing a colour transform is easy; writing one that is subtly
wrong is easier, and a wrong picture that looks plausible is the failure nobody
catches. So the crate has a taxonomy of refusals, one per thing it cannot do, and a
suite of golden fixtures — inputs with expected outputs that Lumit did not produce —
that gates everything it claims it *can* do. A transform with no passing fixture is not
a supported transform.

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

### The first second: the boot splash

Lumit opens on a small centred card that lists what came up, then gives way to
the application. `BootGate` in `main.dart` is the switch, and the rule it keeps
is that **the splash is the window while it is up** — the shell is not put in
the tree behind it at all. That is not fussiness about appearances: if the shell
were built underneath, the first-run question would open on top of a screen you
cannot click through, and every panel would start asking the engine for pictures
nobody can see yet.

The lines it streams are not invented. They are the engine's own boot log — the
library version, the ABI it speaks, what this build was compiled with — read
once through `bootLog()`. That is genuinely everything the engine can say about
starting up: there is no stream of boot events to subscribe to, so the splash
cannot report a module that took its time or came up degraded. Noted in
`docs/TODO.md` as the thing that would need building first. A build with no
engine behind it at all falls back to a canned list, so the placeholder still
opens on something honest rather than a blank rectangle.

### The second second: the welcome screen

When the splash finishes it does not hand the window to the editor. It hands it
to one more page — `shell/welcome_frb.dart` — which asks the only question worth
asking before any work starts: what would you like to open? Three cards along the
top (make a project and choose where to keep it, start blank and save later, open
a `.lum` that already exists), the projects you had open recently under them, and
two links at the bottom to the manual and the release notes.

`BootGate` keeps the same rule here it keeps for the splash: **the welcome is the
window while it is up**. The shell is not built behind it, for exactly the reasons
above. Press any card and the welcome comes down, the shell goes up, and — if a
document is being read — the shell's own progress card is already over it.

Somebody who double-clicked a `.lum` in Explorer never sees the screen. They have
answered its question by opening the file, and asking again would be rude.
`main()` spots the path on the command line and turns the screen off for that
launch.

One thing is worth knowing about the recents list itself. **It is Lumit's own
record, not the disk's**: the date beside each project is when *you* last opened
it here, kept in the settings file (`Workspace.recentProjects` and its stamps),
because asking the file system for each row's timestamp would make the screen
wait on any project that lives on a network drive.

### The little picture on a recent row

Every recent row opens with a thumbnail — the project as it looked the last time
you saved it. Three questions are worth answering about it, because none of the
answers is the obvious one.

**Where does the picture live?** Not in the `.lum`. It goes in Lumit's own
settings folder (`%APPDATA%\lumit\thumbnails` on Windows), in a file named after
a scrambled version of the project's full path. Two reasons. The first is
manners: if you email somebody a project, they should not receive a still of your
screen inside it. The second is that the picture is about *your* copy of the file
on *your* machine, which is exactly what the rest of the settings file is about.
Naming the file after the path rather than after the project means two projects
both called `Untitled.lum` in different folders keep their own pictures — and
that moving a project loses its picture until the next save, which is the honest
answer rather than showing you a stale one.

**When is it taken?** Straight after a save finishes, and never before. The file
is written, the "saved to…" notice appears, and only then does Lumit go and
photograph the picture. That ordering is the whole point: taking a photograph is
a few milliseconds of work that has nothing to do with your document being safe
on disk, so it is not allowed to stand between you and the save. If it fails —
and there are several ordinary ways it can — nothing is said. A row with no
picture just shows a small grey placeholder, which is a perfectly normal thing
for a row to show.

**Where does the picture come from?** From the Viewer, by photographing what is
already on screen. This sounds like a shortcut and is not one: it is the only
option there is. A rendered composition frame **never crosses from Rust into
Dart as pixels**. The frame is drawn on the graphics card and handed over as a
*handle* to that card memory — the picture stays where it was made and Flutter
draws it from there, which is the whole reason the Viewer is fast. There is no
"give me the pixels" call to make, because giving Dart the pixels is precisely
what that design avoids. So the picture on screen is the only copy of the frame
that Dart can reach at all, and Lumit photographs it exactly where the Viewer's
own **Snapshot** button already does.

Two consequences follow, and both are fine. A project saved with no Viewer up —
from the welcome screen's own *New project* card, before the editor has even been
built — gets no picture until the next save from inside the editor. And if the
engine ever *does* grow a way to render a composition straight to bytes, there is
exactly one function to change (`captureViewerPicturePng`), and nothing else in
the chain knows or cares where the bytes came from.

The list is yours to prune: **Clear** empties it, and the **×** at the end of a row
forgets that one. Neither asks you first, because neither destroys anything — the
project files are untouched, and File ▸ Open brings any of them straight back. The
one control in Lumit that *does* stop and ask is the disk cache, and it asks
because emptying it can throw away a night's rendering with no way back.

### Opening a project, and why the old one stays on screen

Opening a `.lum` is not a small read. The engine parses the whole document and
then checks every media file it names is where the file says it is, and on a
large project that is long enough to notice. Two things follow from it, both in
`LumitState.openProject` in `main.dart`.

**The read happens away from the interface.** `open_project` is one of the few
bridge calls that is deliberately *not* marked `sync`: a sync call runs on the
same thread that draws, so a slow one freezes the window until it is done, to the
point where Windows offers to close the program for you. Unmarked, frb runs it on
a worker thread and hands Dart a promise, and the interface keeps drawing.

**Nothing changes over until all of it is ready — including the picture.** While
the read is running, the `opening` flag is up and the shell covers itself with a
card and a moving bar (`OpeningOverlay` in `shell/splash.dart`). Behind the card
the panels are still drawing the *previous* document — that is deliberate. The
alternative is watching an editor come apart and reassemble panel by panel as
the new document arrives.

The card does not lift when the document is in. It lifts when the Viewer has
something to show of it, or when the session restore says there is nothing to
show — a project that opens with no composition fronted. Panels filling while
the preview was still coming read as a slow editor; everything arriving at once
reads as an application loading (K-351). What ends it is `previewReady`, called
on the first reply from the new project's render worker: any reply, not the
frame alone, so a first render that fails cannot leave the shell covered for
good.

There is a subtlety worth knowing, because it caused a bug. Opening a project
clears the engine's registry of open projects, so **every handle Dart is holding
dies at that moment** — including any question already asked and not yet
answered. The missing-file badge in the Viewer asks each footage layer whether
its file is still there, one round trip each; if the document is replaced while
those answers are in flight, the rest of them come back as errors. They are
caught and dropped rather than reported: there is no missing media in a document
that is gone, and the panel is about to be rebuilt from the new one anyway.

**The same card covers other slow jobs.** Beat detection reads every audible
thing in a composition and can spend seconds doing it. Without a sign that
anything is happening, the command reads as one that did not land, so the shell
puts the opening card up for the duration with its own line — "Detecting beats"
— and takes it down when the markers arrive. The plumbing is one line of state:
`LumitState.busy` holds the words to show, or nothing when nothing is running,
and `BusyOverlay` (`shell/splash.dart`) is what watches it. Jobs set it through
`showBusyWhile`, which puts the words up and clears them when the job settles
**either way** — a comp with no audio finds no beats, and that is a finish, not
a reason to leave the interface covered.

The bar sweeps rather than filling: the analysis reports no fraction of itself,
and a bar that invented one would be describing work nobody is measuring.

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

**A structure at the seam only ever grows by fields with a default.** When the
engine learns a new export setting, that setting becomes another field on the
flat structure the dialogue hands over. If it were a *required* field, every
place in Dart that builds one would stop compiling — including screens nobody is
working on that day. So each new field says what it means when nobody sets it,
and the answer is always "what Lumit did before this field existed". The result
is that adding an option to the engine changes no interface code at all until
somebody chooses to draw it, and an old saved preset that has never heard of the
option still exports exactly the file it always did.

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

### Reading what a file is, without stopping the editor

Before Lumit can put a clip in a composition it has to know some plain facts about
it: how big the picture is, how fast it runs, how long it lasts, whether it has
sound. Finding out is called **probing**, and it is not a decode — it is opening
the file far enough to read the label. It is still a file being opened, though,
and off a slow drive or a network share that takes long enough to feel.

It used to be felt, because it happened *while you waited*. Dropping footage into
a composition asked the file its size and length on the very thread the interface
was calling on, so the editor stopped until the answer came back.

`crates/lumit-bridge/src/probe.rs` is the fix, and it has two halves:

- **A worker thread that reads ahead.** Importing a file, opening a project or
  relinking one says "this file will be asked about soon" and carries on
  immediately. The worker opens each file in the background and writes what it
  finds into a small shared notebook.
- **A fallback that never guesses.** When something genuinely needs the answer
  now — placing a layer, which has to know the media's real length — it looks in
  the notebook, and if the answer is not there yet it reads the file itself,
  there and then. That is the important half: the worker makes the answer *fast*,
  it never changes what the answer *is*. A file nobody warmed gives exactly the
  same layer as a file that was.

The notebook entry is filed under the file's own size and modification time, so
an answer can only ever be read back for the file it was taken from. Replace the
file, move it, delete it, relink to a different one, and the entry no longer
matches: it is read again. That is what lets the Project panel keep asking "is
this media still there" honestly while paying for the real question only once.
It is bounded (a few hundred files) and emptied when a project closes, which also
cancels anything the worker still had queued for the project that is going away —
the same "check whether your work is still wanted" habit the rest of the engine
has.

There is nothing to poll and nothing to drain. Every panel that shows a fact
about a footage file already asks for it when it draws; the worker only decides
whether that question costs a file open or a look-up.

### A folder of stills that behaves like a clip (K-539)

A 3D application does not hand you a movie. It hands you a folder — two thousand
files called `Depth000000_depth.exr`, `Depth000001_depth.exr`, and so on — which
between them are one shot. Lumit brings the whole run in as **one footage item**:
you pick any single file out of the picker, and the item that lands is the run.

Working out *which* run is `crates/lumit-media/src/sequence.rs`. It takes the file
name apart at its number — the longest run of digits before the extension, so
`shot_v2_0043.exr` is frame 43 of version 2 rather than frame 2 — and then looks
in the folder for every other file with the same name either side of the same-width
number. The run is the **unbroken block** around the one you picked. If frame 50 is
missing and you picked frame 7, you get 1 to 49; pick frame 60 and you get 51 to
100. That is a deliberate choice: refusing outright would let one deleted file
reject a whole shot, and quietly closing the hole would show you the wrong picture
at the wrong time without ever telling you.

**Lumit does not decode a single one of those files itself.** FFmpeg has been able
to read numbered runs for decades: hand it `Depth%06d_depth.exr`, a start number
and a frame rate and it gives back a video stream, exactly as if you had handed it
an `.mp4`. So the whole feature is a naming job. Once the pattern is worked out,
the probe, the frame table, the decode, the decoded-frame cache, the read-ahead
thread, the Project panel's thumbnail and the missing-media colour bars are all
the same code a video file goes through — there is no second path to keep honest.

Two things about a sequence are worth knowing.

**The project stores almost nothing about it.** Just the frame rate. Where the run
starts, how long it is and which files are in it are read off the folder every time
it is opened, because the files on disk are the truth — drop ten more frames in
overnight and the item is ten frames longer, with nothing to reconcile and nothing
that can go stale. The rate is the exception, because stills have no rate of their
own: a photograph does not know it is a twenty-fifth of a second. Somebody has to
say, and the only one who can is the project. It defaults to 25, and there is not
yet a control for changing it.

**The item still points at a real file** — the run's first frame. The pattern with
the `%06d` in it names nothing on disk, so it could not be fingerprinted, saved,
rebased or checked for existence. Everything that stats a path stats the first
frame; only the moment of opening the media uses the pattern. That is also what
makes relinking work: point at any frame of the run in its new home and Lumit
resolves it back to that run's first frame before doing anything else, which is
what lets the rest of your footage come back in the same sweep (K-538).

The frame table is arithmetic rather than a scan. For a video, Lumit walks every
packet in the file to learn which frame sits at which timestamp. For a sequence
that would mean *reading every file* — tens of gigabytes for a feature-length
OpenEXR render — to produce a table it can work out exactly: every file is one
frame, evenly spaced, and how many there are was already counted off the folder.
Two files are read to learn where the clock starts and how far it steps, and the
rest is multiplication.

After Effects projects bring their sequences with them. A `.aep` marks one by
having its file reference point at a **folder** rather than a file, so on import
Lumit looks inside that folder for the first numbered file and starts the run
there — skipping the `desktop.ini` a Windows folder collects, which would
otherwise sort ahead of frame zero.

### The renderer only opens the files the picture needs

The renderer does its own probing, and it is a heavier one than the Project
panel's: as well as the file's label it wants the file's **frame index** — the
table that says which byte in the file each frame starts at, which is what makes
scrubbing land on exactly the frame you asked for. Building that table for a
file it has never seen means reading the whole file through once. It is cached
on disk afterwards, so it is paid once per file, ever — but the first time is
seconds on a long clip, and it happens before any pixels appear.

It used to do this for **every footage item in the Project panel**, before the
first frame of any composition. A project with forty clips in it opened all
forty files before showing you the first frame of one of them, and — the part
that gave the game away — making a brand new, completely empty composition made
you wait for all forty too, in order to show you nothing.

The fix is to ask a smaller question. Before a frame is made, the renderer works
out which footage items *this* composition can actually put on screen:

- the footage its own layers name;
- the footage named by the clips on its Sequence layers;
- and then the same question again for every composition it nests — through a
  Precomp layer, or through a clip whose source is a comp — following the chain
  as deep as it goes.

Everything else in the project is left shut. An empty comp opens nothing at all.

Two details make this safe rather than merely quick. The walk ignores whether a
layer is switched visible and whether the playhead is inside it, because a hidden
layer is one click from being shown and the answer must not wobble as the
playhead moves. And what has been probed *stays* probed — the results live in one
notebook per session, keyed by item — so switching between two comps that share a
clip does not open it twice, and each composition's first frame only pays for
what it adds.

The interlock that keeps the cache honest is untouched. A frame can only be given
a name (and therefore banked) once every source it shows is known; a source the
renderer has not looked at yet leaves the frame unnameable, so it is drawn live
and filed nowhere rather than filed under a name that might be wrong. Since the
probing now covers exactly what the comp can show, the frame is nameable exactly
when it was before.

The walk itself lives in `lumit-core` (`comp_footage_items`), not in the
renderer, because "which files can this comp want" is a question about the
document rather than about pixels — the same question the background probe worker
above would need to ask if it ever wanted to warm the files for the comp you are
about to open, rather than every file you import.

### Only one renderer is built at a time

Every open project gets a **render worker**: a background thread that makes the
pictures the Viewer shows. The first thing a worker does is build its
*renderer*, and that is not a small thing — it asks the graphics card for a
device, then compiles every shader the compositor might need. On a machine whose
driver has not seen those shaders before it takes **three to five seconds**,
against a first picture of about thirty milliseconds once it stands. Nothing can
interrupt it half way; once it has started it runs to the end.

That is invisible while you are editing, because you have one project open and
therefore one worker, and the wait happens while the window is still arranging
itself. It is very visible in a *test process*, which opens a project, draws in
it, closes it, and does the whole thing again eighty-eight times in under a
minute. Each of those projects asked for a graphics device, and because none of
the builds could be interrupted, twenty of them were in flight at once. The card
ran out of memory part way through the file, and from then on projects that were
perfectly healthy got no device, no worker and no picture — which looks exactly
like a broken transport.

So the workers queue (K-434). One builds; the rest wait their turn. This costs
nothing, because the graphics driver was going to serialise them anyway — twenty
at once was never faster than twenty in order, only more expensive. And when a
worker's turn finally comes round it asks one question first: **is my project
still open?** In a test process it usually is not — the test finished seconds
ago — so the worker stops there and builds nothing, which is what lets a queue
of eighty drain in an instant rather than building eighty devices for projects
that have been and gone.

### Finding the beat, without stopping everything else

Beat detection is the same story one size up. Asking a composition where its
beats are means mixing all of its audio down — which decodes every sound file it
holds — and then analysing the result. On a long comp that is seconds.

It never ran on the thread that draws the interface, so it was not a freeze in
the obvious sense. But it did run on a small **pool** of threads that the bridge
keeps for anything slow, and that pool is shared: it is also how the Project
panel fetches thumbnails, how a layer finds out whether its source has sound,
how the footage panel reads a file's statistics. Two or three detections at once
could occupy the whole pool for seconds, and every panel waiting behind them
stopped. Nothing cancelled, either — closing the project left the analysis
running.

`crates/lumit-bridge/src/beats.rs` gives detection a thread of its own, built
the same way as the probe worker: requests queue and run one at a time, the
caller waits for its own answer (so the button still reports how many markers it
placed), each job remembers which project it was asked for, and closing that
project makes the worker drop it instead of analysing music nobody has open. If
the thread cannot be started at all, the caller simply does the work itself —
the worker chooses where the work happens, never what the answer is.

One detail worth knowing, because it is what keeps the promise testable: the
worker answers with *times and confidences*, not with finished markers. Markers
carry identifiers, and a fresh identifier is different every time by design, so
a marker list can never be compared with a marker list. Times and confidences
can — and "the same audio at the same sensitivity finds the same beats" is
exactly the promise that moving work between threads must not break.

### A layer that is only a sound (K-435)

Some layers make no picture. A camera is a viewpoint, a null is a handle to drag
things by, and now a music track is a sound. Lumit calls that last one an
**Audio layer**.

The surprising part is how little had to be built for it. Drop an MP3 into a
composition and it has played for a long time: it became an ordinary footage
layer, and the part of the engine that mixes sound found it by asking "is this a
footage layer, and does its file have an audio stream?" Both answers were yes,
so it played. Two things were missing. You could not take the *sound* of a video
file without its picture coming along. And the drawing half of the engine still
went through the motions for a layer that had nothing to draw.

So an Audio layer is not a new kind of layer at all — it is an ordinary footage
layer with one bit set on it, `audio_only`, meaning *sound, no picture*. That
choice is worth a moment, because the alternative looks tidier and is worse. Had
it been a new kind, every part of the engine that recognises a footage source by
its kind — the mixer, the waveform drawing, retiming, the project file, the
After Effects importer — would have needed teaching about a second kind that is
footage in all but name. Each of those is a place to forget. One bit on the
layer means they all carry on working and never learn anything, and the only
code that changed is the code that draws.

**What "the code that draws" means here** is three separate places, and they
have to agree. One plans which video files to decode. One builds the actual
drawing instructions. And one works out the *name* of each finished frame, so
that a frame already rendered can be recognised and reused instead of being
drawn again. All three now step over an Audio layer.

That third one is where the real prize is, and it is worth explaining, because
it is the difference between a smooth session and an infuriating one.

Every rendered frame is filed under a name made by hashing everything that could
change what the frame looks like: the layers, their positions, their effects,
which of them are switched on. Change any of it and the name changes, the old
frame no longer matches, and the picture is rendered afresh. That is exactly
what you want when you move a layer. It is exactly what you do not want when you
mute the music: the picture has not changed by a single pixel, but if the muted
layer was part of the name, every rendered frame in the composition has just
been thrown away, and the comp stops playing smoothly while it all comes back.

The fix is a matter of *order*. The frame-naming code walks the layer stack and,
for each layer, first asks whether it is switched on — a hidden layer is not in
the picture, so it is not in the name. An Audio layer is now skipped **before**
that question is reached rather than after it. Ask "is it visible?" first and
the answer itself becomes part of the name, so toggling it renames every frame.
Skip the layer before anything about it is examined and none of its switches can
reach the name at all. Mute it, hide it, solo it, drag its volume to silence —
the picture keeps every frame it had. There is a test that toggles each switch
in turn and insists the name never moves.

Solo needed splitting in half for the same reason. Solo means "just this one",
and it used to be a single question: is anything soloed? If so, show only what
is soloed. But soloing a music track cannot sensibly mean an empty picture — the
track has no picture to show. So there are now two questions. The mixer asks "is
any layer soloed?" and silences the rest. The compositor asks "is any layer that
*draws* soloed?" and, for a soloed music track, the answer is no — so the
picture carries on exactly as it was.

The last piece is the switches themselves. A layer's row shows an eye and a
speaker, and both used to appear on every row regardless of whether they could
do anything — a speaker on a solid colour that has never made a sound, an eye on
a music track that has never shown anything. Each row now shows only the
switches its layer can actually use. A control that does nothing when you click
it is worse than no control, because you have to click it to find out.

### The half-second the lens picker used to cost

The Lens flare effect simulates a real camera lens: the ghosts are traced through an actual
glass prescription, and the starburst is a Fourier transform of the iris. Most of that
happens on the graphics card every frame, and it is fast. One part does not: the **bake** —
working out the lens's ghost pairs, its starburst sprite and its exposure — is heavy
maths on the ordinary processor, about half a second for a complicated lens. It only has to
happen once per lens, and the result is kept, so trying lenses you have already seen is
instant.

The first time, though, it used to happen *inside the frame*, on the same thread that draws
the picture. So choosing a lens from the picker stopped the Viewer dead for half a second,
every time you tried one you had not tried before.

It now runs on a **thread of its own, beside the frame**. Pick a lens, and the frame you are
looking at carries on showing the lens you had — the picture keeps moving, keeps
scrubbing, keeps responding — while the optics are worked out next door. The moment they
are ready the frame is made again with the new lens and the Viewer catches up on its own.
A freeze became a wait you can watch.

Two things had to be true for that to be safe, and they are the interesting part.

**An export must never be provisional.** A frame with the wrong lens in it is a disaster in
a file you are delivering. So the "bake beside the frame" behaviour is *off* unless
something switches it on, and only the Viewer switches it on. The exporter builds its own
renderer, and nobody switches it on there — so the safe behaviour is what you get by
forgetting, rather than something you have to remember.

**A frame with the old lens in it must not be filed under the new lens's name.** Lumit
names every finished frame by a fingerprint of *what is in it* (that is what lets an undo
find its frames still waiting). A frame drawn with the previous lens but named for the new
one would be a permanent lie: nothing you did afterwards — no edit, no undo — would ever
clear it, because nothing would know it was wrong. So such a frame is drawn and shown and
then **thrown away** rather than kept. It costs a re-render; it cannot cost a wrong picture
that never goes away.

There is one more rule, and it is about not wasting the half-second. If you drag the
aperture slider, every position asks for a different bake. Only the last one is worth
computing, so the bake thread takes everything waiting, keeps the newest, and drops the
rest before they start — the same "is my work still wanted" habit the rest of the engine
has.

### When the aperture is animated, not dragged (K-431)

Dragging a slider stops. A **keyframe** does not. Put keyframes on the f-stop and the iris
is slightly different on every single frame — so every single frame asks for a bake of its
own, and one is always being made for as long as the comp plays.

That turned the rule above into something much larger than it was meant to be. The old
version of it was blunt: *while a lens is baking, do not name any frame at all*. Any frame,
in any composition, whether or not it had a flare anywhere near it. With an animated
aperture keeping a bake permanently in flight, that meant nothing in the whole project was
ever named, so nothing was ever kept, and the background job that quietly fills the cache
bar while you are not typing stood down and never started again. One keyframe on one dial
switched off caching for the entire project.

Two things fixed it.

**Ask the precise question.** The engine now simply *counts* the frames where a flare
actually drew the previous lens instead of the one it was asked for. A frame where that
never happened is a frame that shows exactly what its name says, so it is named and kept —
even if a bake for some later frame is being made at that very moment. Only the frames that
really did fall back are dropped. Comps without a flare in them are never affected at all.

**Let a run of frames share one bake.** The heavy part of the bake is genuinely a function
of the iris: the starburst is a Fourier transform of the hole's shape, and the exposure is
a small test render. Neither can be moved into the per-frame work — a Fourier transform
every frame would spend the whole effect's budget on the starburst alone. So instead the
bake **rounds** the iris dials before it looks at them: the f-number to a twentieth of a
stop, the iris rotation to half a degree, roundness and softness to a 256th. A half-stop
ramp then needs about ten bakes rather than one per frame, and the store of recent bakes
holds all ten.

Nothing you can see moving is rounded. The ghosts read the exact f-stop you set — they
shrink and turn smoothly as the iris closes, frame by frame. What steps, by about 1.7% at a
time, is the starburst's shape. That is a deliberate trade, and it is the honest one:
without it an animated aperture is not a lens that caches badly, it is a lens that cannot
be cached at all.

### Stopping down dims the flare (K-432)

The flare sets its own overall brightness. Every lens in the library throws a different
amount of light around inside itself, by factors of hundreds, so when the effect finishes
working out a lens it renders one tiny test picture and works out how much to multiply by
to bring that lens into a sensible range. That multiplier is the flare's exposure.

It used to be measured with the iris wherever you had left it — and that quietly undid the
aperture. Close the iris and the test picture came out dark, so the multiplier went up by
exactly as much as the iris had taken away, and the flare on screen was the same brightness
at f/16 as it was wide open. A real lens does not behave like that: a smaller hole passes
less light and its flare fades with it, which is why photographers stop down to kill one.

The test picture is now always shot with the iris **open** — at the lens's own widest
f-number — so the multiplier describes the glass and nothing else. Stopping down dims the
flare, as it should, and the amount is honest: the light falls off with the square of how
far you closed. If you want a small aperture *and* a bright flare, that is what
**Intensity** is for. As a side effect the flare's brightness no longer steps at all when
you animate the f-stop, since the exposure has stopped depending on it.

**And one related hole, closed at the same time.** You can point the flare at your own
`.lens` file instead of a bundled lens. The bake noticed when you edited that file, because
it identifies a lens by the file's *contents* — but the frame's name only mentioned the
file's *path*. So an edited prescription drew new optics under the old file's name: a
cached frame that was wrong and that nothing could ever clear. A frame's name now includes
the file's size and the time it was last changed, so editing it renames every frame that
reads it. The same goes for a colour LUT file.

### The six and a half seconds nobody was using (K-351)

A shader is a small program the graphics card runs, and the card's driver has to
translate ours into its own instructions before any of them can run. That
translation happens when Lumit starts a render worker, not when the shader was
written, and for most of Lumit's forty-odd kernels it takes a few milliseconds
each.

The lens flare is not most kernels. Its ray tracer is by a distance the largest
shader in the program, and on a real card its pipelines take about six and a half
seconds to compile — which the render worker paid *before it would answer its
first request*. Every project paid it: a project with one empty composition, a
project with no flare in it anywhere, every time one was opened. It was almost
the whole of the wait between opening a project and seeing a picture.

So the flare's pipelines are now built on a thread of their own (`LazyFlare` in
`lumit-gpu`) while the rest of the engine gets on with the first frame, and the
first frame that actually draws a flare waits for that thread — by which time,
in practice, it finished long ago. Nothing about what the effect draws changes;
only when the compiling happens. Worker start went from 7.7 seconds to 1.1.

If you want to see the numbers on your own machine,
`crates/lumit-render/examples/first_frame_probe.rs` is the stopwatch that found
this, and it prints each part of a renderer's start-up in turn:

```
cargo run --release -p lumit-render --features shared-texture --example first_frame_probe
```

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

**Looking at the picture another way, without lying about the export (K-314).**
Two controls sit on the Viewer bar: an **exposure** in stops (the photographic
unit — `+1.0` is twice the light, `-1.0` is half) and a **tone mapping** switch
that folds highlights brighter than the display can show back into range so you
can see what is actually up there. Both are *ways of looking*, not grades: they
happen at the very last step, where the finished scene-linear picture is turned
into the pixels your monitor understands, and the export path never goes near
them. That is not discipline — an export builds its own renderer, and nothing
ever sets a view on it.

Both are per composition and are remembered in the session (above), so a comp
reopens looking how you left it, and neither is an edit: Ctrl+Z will not undo an
exposure nudge and setting one does not make the project dirty.

**And the Viewer says so, in one place.** The **colour-management badge** on the
bar always names the display transform the picture is being shown through —
scene-linear to sRGB, the one built-in pair today — and while either of those
two controls is engaged it reads "· preview" instead, in the accent. The two
controls do light up while they are on, but that only says *this control is on*,
and only where you are already looking; the sentence that matters is "what you
are watching is not what the export will be", and it belongs somewhere you can
read without leaving the picture. Its tooltip is also, for the moment, the only
place on screen that says what tone mapping actually does — an icon's own
tooltip is its name and nothing more, which is a rule, and a readout is allowed
a sentence, which is the exception this fits through.

The cache had to be told about them, and the answer is that **a look is part of
the frame's name** (K-346). Remember that a banked frame is the finished
display-ready picture, so a frame seen through an exposure genuinely is a
different picture from the same frame seen plainly — and the cache names frames
by what is in them. So the exposure and the tone-map switch are mixed into the
name, and each way of looking banks its own frames. Neutral keeps exactly the
name it always had, which is why everything banked before you touched a control
is still there when you put it back.

It used to work the other way: while either control was engaged frames had *no*
name at all, and a frame with no name cannot be filed in any tier. That sounded
tidy and read as a fault — leave the tone map on and the whole cache ladder is
switched off for the session, with nothing on screen to say so. Naming each look
separately costs only that several looks compete for the same budget, which the
tiers already know how to sort out by eviction.

**The transparency grid is a way of looking too (K-352).** The checkerboard the
Viewer draws behind the picture can only show through pixels that are actually
transparent, and for a long time none were: the comp being looked at was always
composited onto its own background colour at full alpha, so even an empty comp
arrived as solid black. While the grid button is on, the engine now leaves that
backdrop out entirely — what nothing covers stays transparent, and the board
shows through it, which is the whole point of the button. It follows every rule
the exposure does — in fact it travels in the same message: the engine is told
the whole look (exposure, tone map, grid) in one call, so it can never hold
half of one. Folded into the frame's cache name like the rest (an opaque frame
and a see-through frame are two different pictures), and never sent to an
export, which always draws the backdrop.

### Light layers, and why an area light matters (K-360)

Lumit now has **Light layers** — Layer ▸ New ▸ Point light, Spot light or Area
light. A light draws nothing itself. Like a Camera, it is a thing the *other*
layers react to; you will not see it in the picture, only what it does.

It is placed with the ordinary transform, which means it animates, parents to a
Null and drags about exactly like any other layer. There was no need to invent
a special way to move a light, so there isn't one.

**The area light is the one worth knowing about.** A point light is a
mathematical dot — it has a position and nothing else. An area light is a
*rectangle* with a real width and height: a softbox, a window, a strip light,
the long tube over a kitchen counter. Real lights have size, and size is what
makes their reflections look the way they do.

Right now the thing that reads lights is the Lens flare, in its **Lights**
source mode: instead of you placing the flare by hand, or the effect hunting
for bright pixels, it flares from the actual lights in your composition. And
because an area light knows how big it is, the flare draws it as its own
shape — a strip light's ghosts come out as bars, not as circles. That fell out
of work already done: the machinery that measures and samples a source's extent
was built earlier for detected sources, so a light with a size simply walks
into it.

Move the light, and the flare follows. Animate it, and the flare animates. That
is the point of making a light a layer rather than a number inside an effect.

### What runs on the GPU, what runs on the CPU, and why lens changes felt slow

A fair question came up: "why aren't we doing all of this on the GPU?" The answer is that
the expensive every-frame work — tracing rays through the lens, drawing the ghosts,
detecting sources in a matte — already *is* on the GPU. What runs on the CPU is the
**bake**: the work done once per lens change, not per frame. Ranking which ghost pairs are
bright enough to draw, computing the starburst's diffraction pattern, measuring each
ghost's size. Those are functions of the lens, not of the frame, so doing them per frame
would be waste wherever they ran — and on the CPU they are deterministic by construction,
which the caches that name frames depend on.

Three things made that split *feel* broken, and all three are fixed:

- **The development build ran the engine unoptimised.** `flutter run` builds a debug app,
  and the Rust engine inherited that — every bake was about sixteen times slower than the
  released code would be. A one-second lens change became twenty-three. The engine's maths
  crates are now compiled fully optimised even in a debug app.
- **The bake used one core.** The ray tracing inside it now runs across all of them —
  producing bit-identical numbers, just sooner. A lens change bakes in roughly a sixth of
  the time it did, on top of the build fix.
- **A finished bake could go unnoticed.** The Viewer deliberately keeps showing the old
  lens while the new one bakes (a wait you can watch, not a freeze) — but the "is it done?"
  check could only be answered by rendering a frame, and nothing rendered a frame until it
  was done. So the picture sat one lens behind until you moved the playhead. The check no
  longer needs a frame, and the picture replaces itself the moment the optics are ready.

### Why changing the last effect is cheap now (K-421)

Put a Lens flare on a layer and an Exposure after it, then nudge the exposure. Until now
the engine ran the whole stack again — the flare included — because the only thing it ever
kept was the *finished* frame, and a finished frame is no use once any part of it changes.

Now every effect's output is kept for a while, on the graphics card, under a name made from
everything that went into it: which source, at what size, and every effect up to and
including this one with all its values. Before a stack runs, the engine asks, starting from
the *end*: "is the picture after effect N already here? After N−1?" It starts from the first
one it finds. Change the exposure and the picture after the flare is still there under its
old name, so only the exposure runs. Change the flare and every name after it changes too,
so everything from the flare on runs again — which is exactly right.

Three things to know. It is **not** a cache you manage: there is no invalidation, only
names, so nothing can be stale, and Clear cache empties it along with the frames. It only
**fills** from committed renders — a drag reads from it (that is the point) but does not
write, and neither does playback, so a long run cannot push your working stack out. And a
few things do not take part yet: an effect that reads another layer's picture (a matte, a
plate, a depth pass), a temporal effect reading neighbouring frames, and stacks on text,
shapes and adjustment layers. Those run as they always did, and are the follow-up. (Stacks
on precomps joined in with K-422, below, once a precomp's picture had a name.)

### A precomp's frames are kept as one picture (K-422)

Precompose a finished section and you expect to stop paying for it: the manual says so.
Until now the engine did not keep that promise. Every time the parent frame was drawn, the
Precomp layer walked into its comp and drew every layer inside it again — and decoded every
piece of footage inside it again — even when nothing inside had changed.

Now a nested comp's frame has a name of its own. The frame-naming described above already
folded the whole nested comp into the parent's name; it now names the nested comp first, on
its own, and folds that one name into the parent. The parent's name still changes when
anything inside changes, so nothing is ever stale — but the nested frame's name is the same
whichever parent asks for it, at whatever position on the parent's timeline, and that name
is what the finished picture is filed under. Draw a precomp once and the next parent frame
that wants it — a different frame of the parent, a parent edit, the same precomp used in
three places — takes the picture off the shelf. The decode planner asks the same question
before it reads any file, so a held precomp costs no decodes either.

It lives in the same store as the per-effect pictures (K-421), with the same rules: it
fills only from committed renders, Clear cache empties it, and it is gone with the card's
memory. Two things stay outside it. A **collapsed** Precomp is never kept, because its
layers are blended straight into the parent and there is no one picture to keep. And a
measured frame (the timing columns) draws the precomp in full so its inner rows get their
numbers, then files what it drew.

### What is hidden is not drawn (K-423)

Put a full-frame solid over a stack of footage and nothing under it can be seen — but until
now the engine drew it all anyway: every file decoded, every effect run, every layer
blended, and then the solid painted over the lot. Now both halves of the render (the
planner that decides which frames to read from disk, and the builder that decides what to
draw) ask one question first: "which is the topmost layer that provably covers the whole
frame with opaque pixels?" Everything under that layer is skipped.

The word that matters is *provably*. A wrong answer would change the picture, so the
question is asked as narrowly as possible and says "no" whenever it is unsure. Only a
plain solid qualifies: fully opaque, not rotated, not 3D, Normal blend at 100%, no masks,
paint, effects or motion blur, and placed (with any parent it follows) so that it reaches
every edge of the comp. And the comp itself must give no layer below a back door: no
camera, no adjustment layer above the solid, nothing above it using a layer below as a
matte or as an effect's input, and never inside a collapsed precomp (whose layers spill
past their own comp's edges). Anything else, and everything is drawn exactly as before.
Footage that has no transparency could qualify one day; it does not yet.

### The region of interest — working on one corner (K-362)

On a heavy shot every preview frame costs the whole frame, even when you are
fiddling with one corner of it. The **region of interest** lets you say: just
this rectangle, please.

Click the rectangle button on the Viewer bar, drag a box on the picture, and
that is all the engine composites until you clear it — click the same button
again. The region stays outlined the whole time it is in force, because the one
genuinely bad outcome here is forgetting you set one and wondering why the rest
of the shot has gone.

**What it saves, honestly.** The composite, the display encode and the handover
to the screen — the per-frame costs that scale with how many pixels there are.
It does **not** save the effect stack. Effects run on each layer at that
layer's own size, before the layers are combined, and none of them knows or
cares which part of the result you are looking at. So a region helps most where
the frame is large and the layers are many; it helps least where one layer has
an expensive blur on it.

Two situations quietly opt out: a composition with an **adjustment layer**, and
a layer with **motion blur** on. Both work by building a full-size intermediate
picture first, and cutting a window out of that halfway through would give a
wrong result rather than a fast one. In those cases the frame is made whole and
the region is cut from it afterwards — you see exactly the same picture, it
just did not save anything. Nothing tells you, because there is nothing to tell:
the picture is identical either way.

The region belongs to the composition, not the project. It rides the session
alongside the preview resolution, so it is where you left it when you come back
and it can never end up in an exported file.

### Lights that actually light things (K-361)

A Light layer used to be something the Lens flare read. Now it is also
something your **footage** reads: turn on a layer's **Accepts lights** setting —
it is on by default — and the composition's lights fall across it. You will
find it by right-clicking the layer in the Timeline; it is a ticked line in that
menu. It used to be a small icon in the Timeline's Modes column, which is a lot
of permanent screen furniture for something most compositions never touch, so
the owner had it moved (K-483).

Point a big area light at a piece of footage from slightly off to one side and
you get the thing this is for: a soft gradient raking across the shot, brighter
where the light faces it squarely, easing off toward the edges. That is what a
softbox does in a room, and it is what sells a composited element as belonging
in the same space as the plate behind it.

**How it knows.** There is a piece of geometry, several hundred years old, that
answers this exactly. Stand on the surface being lit and look up. The light is
a rectangle somewhere in your sky. How brightly you are lit is simply *how much
of your sky that rectangle covers* — with light arriving from near the horizon
counting for less, because it arrives at a slant and smears over more surface.
And that quantity has a closed-form answer: add up one number per edge of the
rectangle, four edges, done.

That last part is why this is fast. There is no ray tracing, no sampling, no
noise to clean up. Four small calculations per pixel and the answer is not an
estimate of the right number — it *is* the right number. It is the same
calculation the games industry uses for area lights, in the case where the
surface is matte rather than shiny.

**Two decisions you will notice.**

*Light adds; it does not take away.* Dropping a light into a composition can
only ever brighten things. A physically strict renderer would say that anything
the light does not reach receives nothing and should therefore be black — true
of a dark room, useless in a compositor, where the picture underneath is
already lit by whatever was in the shot. So the light is added on top of what
is there. A happy consequence: a composition with no lights in it is
untouched — not "almost identical", but the same file, byte for byte, as before
any of this existed. Every project you have ever saved still renders exactly as
it did.

*A layer is lit as the flat plane it is.* Lumit is 2.5D: a layer is a flat card
in space, and the whole card faces one direction. So the whole card is lit as
one surface. It does not know that the footage on it shows a face with a nose
on it — nothing tells it where the surface bulges, and guessing from brightness
goes wrong the moment someone wears a white shirt. A softbox raking across a
card is honest and it is what the geometry really is; the alternative is a
guess that looks impressive until it doesn't.

One thing that catches people: **a light in the same plane as the layer does
nothing at all.** That is correct, not a bug — a strip light lying flat on a
table throws no light *along* the table. Give the light some z, so it sits in
front of the layer, and it will do something. New lights come with a size but
no depth, so this is the first knob to reach for.

The switch is in the Timeline's render column, alongside the 3D and motion-blur
switches, marked with the same icon as a Light layer. Turn it off on a layer you
want the lights to leave alone.

### Two lens flares, and why there are two (K-359)

Lumit has a **Lens flare** and a **Sprite flare**, and they are not two
attempts at the same thing.

**Lens flare** is a simulation. It has a real lens inside it — the actual glass
elements of a real photographic lens, their curvatures and spacings — and it
traces light bouncing between those surfaces. The flare it draws is whatever
that lens would genuinely do. That is why it takes work to render, and why it
behaves in ways you did not design: ghosts change colour as the light crosses
frame, the train stretches at the edges, the starburst is the true diffraction
pattern of the aperture. You are photographing something.

**Sprite flare** is a drawing. You tell it where the light is, and it draws a
glow, a row of ghosts and a streak. Nothing is simulated and nothing is read
off the picture — which means it does exactly what you set, every frame,
forever. It is cheap, and it never surprises you.

Reach for the first when you want the shot to look photographed. Reach for the
second when you want a specific look and want it to stay put. Neither is a
fallback for the other, and both stay.

The ghosts in the sprite version march along the line from the light through
the *middle of the frame*, which is not an arbitrary choice — that is where a
real lens throws them, because they are reflections about the lens's own axis.
Move the light left and the ghosts swing right. Their spacing is a proportion
of that distance rather than a fixed number of pixels, so the train gathers as
the light nears the centre and stretches as it leaves — which is the thing that
makes it read as a lens rather than as a row of circles.

### Light wrap — why a cut-out looks pasted on, and the fix (K-358)

Key a person off a green screen, drop them on a new background, and something
is wrong even when the matte is perfect. The reason is light. In a real shot
the background is *behind* them, and its light spills round the edges of them —
catching the hair, grazing a shoulder, softening the outline. A matte cut in
software has none of that. The edge is a clean line where two pictures meet,
and the eye reads it instantly as two pictures.

**Light wrap** puts the spill back. You point it at the background layer and
give it a **Width** — how far, in pixels, the light should creep in from the
edge. It blurs the background over that distance and lays that glow into a band
just inside the subject's outline: strongest right at the edge, gone a few
pixels in, and nothing at all out where the subject isn't.

Two details make it behave rather than fight you. It finds the edge from the
subject's own matte, so there is no mask to draw. And it *screens* the light on
rather than adding it, which means a very bright background brightens the edge
towards itself and stops — it cannot blow out to white, which is what makes a
naive version look radioactive.

It is deliberately a small effect. Width and **Intensity** are the two dials
worth touching; leave Width at zero and nothing happens at all.

**Auto, Full, and the difference between them (K-357).** The Viewer bar's
resolution dropdown says how many pixels the engine is asked to *make* — which
is not the same as how big the picture is drawn. **Auto** makes only the pixels
the current magnification can actually show, so a Viewer in a small panel is
cheap; it is the default because it is what Lumit has always quietly done.
**Full** makes them all, whatever the panel is showing, which is what you want
when you are judging fine detail. **Half**, **Third** and **Quarter** are real
reductions of the composition — Half makes a quarter of the pixels — for when a
shot is too heavy to preview smoothly. None of them can reach an export.

The choice belongs to the **composition**, not to Lumit: a heavy effects shot
can sit at Quarter while the title card in the next tab stays at Auto, and each
remembers its own. It is remembered with the project but is not part of the
work, so it makes no undo step.

**Auto asks again when the panel changes size (K-430).** On Auto the number of
pixels a frame is made at is decided when the Viewer measures itself, and the
first measurement of a session happens at whatever size the window opened at.
Nothing used to notice afterwards: making a panel bigger is not an edit and does
not move the playhead, so the coarse first frame stayed on screen until
something else happened to change it — which, paused, could be a long time.
Lumit now compares the new measurement with the old one and asks for the frame
again when they differ, rounded to whole percent because that is how finely the
engine tells one frame's scale from another's. Two things are deliberately left
out: a fixed tier (Full, Half, Third, Quarter), which is a choice the panel has
no say in, and the middle of a zoom, which lays the picture out dozens of times
on its way to where it is going — only the size it lands on is asked for.

**The point cloud follows the switch and the solve (K-430).** When a shot has
been analysed by the **Camera track** effect, Lumit knows where a few hundred
features of the scene sit in space, and draws them over the picture as dots
whenever that effect's **Show points** is on. Which dots to draw is worked out
from two things: which frame is on screen, and where the document stands. That was enough
for everything the *user* does to the document — but not for two moments that
change what should be drawn without changing either. Switching the effect off is
one: the dots stayed until the frame next changed. An analysis finishing is the
other: a solve is the answer to minutes of work on the media file, not an edit,
so the document is where it was and the frame is where it was, and the dots did
not arrive. Both are now told. The Viewer watches the read model, so the switch
takes effect at once; and Lumit keeps a small counter that a landing solve bumps,
which the cloud treats as a third reason to ask the engine again.

**The background swatch** sits next to the transparency grid button, and the
two are opposite sides of one question — what is behind the picture. The grid
is a way of *looking* (it changes nothing about the composition). The swatch
changes the composition itself: it is the colour the picture is drawn onto, it
travels in the file, it comes out in the export, and Ctrl+Z undoes it. Black
and white are one click each, since that is what it is nearly always set to.

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

### When a camera track stops part-way (K-540)

Camera tracking works by following hundreds of small, distinctive specks of the
picture from frame to frame, and then asking what camera motion would explain
all of that sliding at once. Everything rests on the first half: a speck that is
followed from frame 40 into frame 41 is what *ties those two frames together*.
Take away every such speck at one boundary and the two halves of the shot become
two unrelated shots — there is nothing left that appears in both, so there is
nothing to work out how the camera moved between them.

That happens in real footage. The frame whites out; the lens racks so fast that
every patch smears; a cut lands in the middle of the clip; the shot goes through
a tunnel. Lumit used to carry on regardless — it decoded the rest of the file,
and placed a camera on frames it had followed nothing through, because the code
that fills in a frame with no measurement of its own simply copies its
neighbour. The result looked like a finished answer and was not one.

Now the analysis **stops** at that boundary and solves the part that worked.
Three things are worth knowing about how it decides.

**What it counts is what crossed, not what is alive.** After every frame, some
specks have died and the detector immediately goes looking for new ones in the
gaps they left — that is what keeps a long shot dense, and it means the number
of specks being followed is back to normal within a single frame however badly
the shot failed. So the number that matters is not "how many am I following" but
"how many of the ones I am following came here from the last frame", and it is
the second one Lumit watches.

**The number it stops at is not a preference.** Working out the geometry between
two frames needs seven matched specks at the absolute minimum, and eight before
there is one spare to check the answer with. Below eight the arithmetic does not
exist — not "is poor", does not exist — so that is the line. It is deliberately
a hard floor and nothing cleverer: a shot that degrades badly without ever
crossing it still solves badly, and the average error already reported is what
says so.

**A partial answer is a real answer.** The span that was followed is solved
properly, kept, and cached like any other, so re-opening the project does not
re-analyse it. The effect's card shows a thin bar of how much of the clip
carries a camera, with the line under it saying how far it got in words rather
than quoting an accuracy over frames it never saw. A Camera layer pointed at a
partial solve follows it inside that span and, past the end, holds the last
pose it worked out — the same holding it already did for a camera that runs on
past the end of its shot. What to do about it is a normal editing decision: mask
out whatever ruined the shot and analyse again, try a higher feature density, or
cut the shot where the track stops and treat the two halves as two shots, which
is what they are.

### Why the picture goes soft while you drag a value (K-383)

Some effects are expensive in a very particular way. Depth of field and Lens
dirt are *gather* effects: for every pixel of the output they read a whole disc
of pixels around it. Double the width of the picture and you have four times as
many pixels to fill — but each of those pixels is also gathering from a disc
that has itself doubled in radius, which is four times as many reads each. So
the work goes up with roughly the *fourth* power of the size. That is why a
Depth of field that renders comfortably in a small window can take seconds at
full resolution, and why dragging its aperture felt like arguing with a picture
that was always a few seconds behind your mouse.

The fix is the one every editor uses: while you are dragging, render the picture
small. Lumit already knew how to do that. Every "px@comp" parameter — an
aperture in pixels, a blur radius, a light's position — is multiplied by a
*preview factor* on the way in, so a frame rendered at a third of the size is
framed identically to the export, just softer. That machinery was built for
playback, where the engine drops resolution to keep the sound and picture
together.

So a drag now uses it too. The engine picks the finest of Full, Half, Third or
Quarter whose picture still fits inside a 640×360 budget, and never goes below
Quarter (below that you can no longer judge what you are looking at). A small
composition, or a big one shown in a small panel, is already inside the budget
and is not softened at all. A 1080p composition at full size drags at a third,
which is about a ninth of the pixels and — because of that fourth-power rule —
something nearer a hundredth of the work for a gather effect.

The moment you let go, the drag *commits*: the value is written to the document,
and the ordinary render path asks for the frame again at the Viewer's own
resolution. So the sharp picture comes back on release without anything special
being arranged for it.

The rule lives in the engine rather than the frontend, in one place, for a
reason worth knowing: there is a separate engine call for "render a frame with a
value the user has not committed yet", and *every* use of it is a live drag —
effect parameters, transform rows, masks, shapes, text, paint, the handles on
the picture itself. Putting the reduction there means no part of the frontend
has to remember to ask for it, and a drag added next year gets it for free.

What it does not do: refine while you hold still mid-drag. Pause with the button
down and the coarse picture stays until you release. That would need a timer per
gesture at every drag site, for a case that release answers a moment later
anyway.

### How far an effect reaches, and why the tile has a margin (K-433)

A blur does not read only the pixel it is writing. To put a 2 000-pixel radius on
one pixel it reads 2 000 pixels in every direction around it. So when the engine
works on a piece of a frame rather than the whole thing, it has to hand the effect
a piece with a **margin** of spare picture round the outside — otherwise the blur
runs out of neighbours at the edge of the piece and the seam shows.

Every effect therefore declares how far it reaches: nothing at all (a brightness
change needs only its own pixel), the whole frame (a directional blur whose length
can be typed to anything), or a margin of so many pixels. That margin is quoted in
the same unit as every distance in Lumit — pixels at composition size — and it is
sized from the effect's own *largest* setting, so no value you can type can ever
reach past it. Gaussian blur's radius stops at 2 000, so its margin is 2 000. Where
a setting has no ceiling, the margin is twice the end of the slider.

It used to be quoted as a percentage of the frame's diagonal, which was wrong once
every radius became a plain pixel count: a quarter of a 1080p diagonal is 551
pixels, and a 2 000-pixel blur would have been cut off at the edge of its own tile.
Being pixels, the margin now shrinks with the preview exactly as the radius does —
half the radius at Half resolution wants half the margin — and it never falls below
one pixel, so even a kernel that reads only its immediate neighbours still gets them
at Quarter.

### The one effect that makes the picture bigger (K-542)

Every effect in Lumit has, until now, been handed a picture and asked for a
picture the same size back. That is what makes a stack of them cheap to reason
about: the layer's own rectangle goes in at the top, comes out at the bottom, and
the compositor puts it where the layer's position and scale say it goes.

Tile breaks that, on purpose, because the After Effects effect it matches does.
Tile stamps a rectangle of the picture side by side across the frame, and it has a
control called **Output width and height** that says how much area gets stamped.
Set to 110 %, it means "keep going a little past the edges". That is not a
decoration; it is the standard way to give a shot some spare material. If you are
about to stabilise a shaky clip, the stabiliser has to push the picture around and
will run off the edge of it — so you first tile the frame outwards by ten per cent
with **Mirror edges** on, which folds the outer band back on itself and produces a
seamless border that did not exist in the footage. The stabiliser now has
something to eat into.

That only works if the extra material is *really there* for whatever comes next.
So when Output width or height goes above 100 %, Tile now hands back a **bigger
picture than it was given** — the original frame sitting in the middle of a wider
one, with the copies filling the margin. The effects below Tile in the stack run
on that wider picture, so a warp or a directional blur finds tiled material where
the layer's edge used to be nothing at all.

Three small things had to follow from that, and they are worth knowing because
they are where the surprises would be:

- **Nothing moves.** The compositor places a layer by a rectangle with a pin in it
  (the anchor point). When the picture grows, the rectangle grows by the same
  amount and the pin slides to stay over the same pixel, so every pixel of the
  original ends up exactly where it was before. Only the margin is new.
- **A few places cannot grow, and crop instead.** An adjustment layer's effects
  run against the composite underneath it, which is the size of the composition by
  definition — there is no "underneath" outside the frame to grow into. The same
  goes for a layer being used as a matte, or a layer another effect is reading as
  a second input. On those, a Tile above 100 % behaves the way it always did.
- **There is a ceiling.** The slider reaches 500 %, and five times a 4K frame in
  each direction would be a third of a gigabyte of working texture asked for by a
  drag of the mouse. The growth stops at 8 192 pixels a side, which is the largest
  texture every graphics card is guaranteed to allow.

While Tile was being fixed, its starting values changed too. A fresh Tile used to
arrive as a 2×2 grid, on the principle that an effect should look like something
the moment you drop it on. That principle is right for a blur, whose control means
something on its own, and wrong for Tile, whose controls mean nothing until you
have said where the picture is being repeated *to* — and a grid nobody asked for
is a change nobody can undo by eye. A fresh Tile now changes nothing at all, which
is also what After Effects does. "Nothing at all" is meant exactly: the code that
would resample the picture one-for-one is skipped entirely, because a divide
followed by the multiply that undoes it does not always come back to the same
number in a computer, and "not quite the same picture" would be worse than either
answer.

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

**A held frame is shown first and measured afterwards.** Lumit keeps finished
frames — on the graphics card, in memory, on disk — so returning to one costs a
copy rather than a render. A copy has nothing to say about what the layers cost.
The engine used to refuse the copy while the clock was on and composite the
frame again, which meant a frame the cache bar showed green still made you wait
on arrival — with measuring on by default, that was every scrub. Now the held
frame is shown at once, the worker makes a note of it, and on its next idle
moment (about a fifth of a second after you stop) it composites that one frame
again with the stopwatch running and throws the picture away: the numbers land
in the column a moment after the picture, and the picture never waits for them.
Switching the clock on still re-asks for the frame you are looking at, so the
column fills where you are rather than where you next scrub to.

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

**Where a shape's points are.** A shape layer is
sized by the art it holds: the layer *is* the box the drawing fits into, and it
grows and shrinks as the drawing is edited. That means the numbers stored for a
point — where it sits in the drawing — are not the same as where it sits *on the
layer*, which is measured from the box's top-left corner. Drawing a point
without subtracting that corner puts it a whole box away from the art — while
the wireframe rectangle and the picture, which only need the box's *size*,
still look right — so the Viewer subtracts it (K-308).

Two things follow from the box being the drawing. The first: the outermost points
of a shape sit exactly where the box's resize handles do, so on a drawn square
every corner is both — a press close enough to a
point means the point, and the handles keep the rest of the reach. The second:
dragging an outermost point *moves the box*, so everything else in the drawing
would slide the other way. The engine moves the layer by the same amount in
the same edit, which is why the rest of the art stays put and why undo still puts
everything back in one step.

The picture keeps up as you drag, rather than waiting for you to let go:
the drag asks the engine for a provisional frame, at most one every twenty
milliseconds, exactly as dragging a layer about does.

What you still cannot do is drag a point's **curve handles** — the two arms that
decide how the line bends through it. You can pull them out while *placing* a
point with the Pen, but not afterwards, on any path. That is not an oversight
waiting to be wired: the file format has no way to say "these two arms are
linked" versus "this is a corner", so adding the gesture means adding that to
the format first, and deciding what an older project means without it.

### Trimming a shape's path

A shape layer's art can be **cut back to a piece of itself**, and animated so the
piece grows: the drawing draws itself on. Three numbers do it, on rows of their
own under the item in the Timeline. **Trim start** and **Trim end** say where
along the path the art begins and ends, as a percentage; **Trim offset** slides
that piece along the path, in degrees, so 360 is a full lap of a closed shape.

The percentage is of the path's **length**, not of how many points it has. That
matters because points are not spread evenly: a circle drawn with four points and
long curve handles has most of its length between them, so counting points would
make the trim crawl along the straight bits and jump round the curved ones.
Measuring length is what makes the growth look even, and it is the same
measurement a paint stroke's write-on uses.

Two things behave the way After Effects behaves, and are worth knowing because
neither is obvious. The **fill** is trimmed too, not just the outline: the piece
that survives is joined end to end and filled, so half a trimmed circle is a
filled half-circle rather than a whole one with half an outline. And on a
**closed** shape the piece **wraps** — slide it far enough with the offset and it
runs through the point the shape starts at and carries on — while on an **open**
path there is nothing to wrap through, so sliding it far enough simply runs it
off the end and nothing is drawn.

Setting the end below the start draws nothing at all. That is not an error: it is
what the first frame of a write-on looks like.

### Dashing a shape's outline

An outlined shape carries **Dash**, **Gap** and **Dash offset** rows beneath it,
in pixels. Dash is how long each mark is, Gap is the space after it, and the two
repeat along the path for as long as the path lasts. Offset slides the whole
pattern along, so keying it makes marching ants.

The rows show up only on a shape that actually has an outline — a fill-only
shape has nothing to dash — and both start at zero, which means solid. Typing a
Dash is what turns the dashes on; there is no separate switch to find.

One deliberate limit: if the dash and gap are so small that the path would be cut
into thousands of pieces, the outline is simply drawn solid. At that size the
dashes would be invisible anyway, and cutting them would cost a frame's worth of
work to draw something you could not tell from a line.

### Filling a shape with a gradient

A shape item's first two rows are **Fill** — the colour inside its path, which you
can now change long after the shape was drawn — and **Gradient**, which is Flat,
Linear or Radial. Choose Linear or Radial and three more rows appear: the
**Gradient colour** the ramp ends at, and the two points that aim it.

Linear ramps along the line between the two points: everything before the first is
the fill colour, everything past the second is the gradient colour, and in between
the two mix. Radial ramps outwards from the first point, with the second sitting on
the outer edge — so the middle is the fill and every direction fades away from it.
Those are the same two readings the Gradient *effect* offers, and deliberately: one
idea should not have two meanings in one application.

Switching a ramp on aims it at the shape's own box for you — top to bottom for
linear, middle to edge for radial — so you see a ramp immediately and move it from
there. The two points are ordinary animatable numbers, so a gradient can sweep.

Two things this is not. It is not a stop list: there are two colours, the fill's and
the gradient's, and a ramp with five stops needs an editor of its own before it is
worth storing. And the colours themselves do not animate — the points do. Both are
places the feature can grow into without moving anything already saved.

### Growing and shrinking a shape's outline

**Offset path** is the first row under a shape item. It is one number in pixels:
positive pushes the outline outwards, so the shape gets fatter; negative pulls it
in, so it gets thinner. The shape keeps its character while it does — a rounded
rectangle grown by ten is still a rounded rectangle, with rounder corners.

That is the difference between an offset and a scale. Scaling a rounded rectangle
makes its corners bigger in proportion; offsetting it adds the same margin all the
way round, which is what you want for a border, a keyline or a thicker version of
a logo. Corners come out **round**, which is the only kind of corner Lumit's shape
outlines have.

One honest limit: pull the outline in by more than the shape is thick anywhere and
the outline crosses itself, leaving a small loop. Most of the time the fill rule
swallows it and you see nothing; where you do see it, the cure is a smaller number.
Cleaning that up properly means a whole polygon-clipping library, which is not
worth carrying for a case a slider drag walks straight back out of.

### Repeating a shape

A shape item has a **Copies** row. Leave it at one and nothing changes; turn it
up and the item is drawn that many times, each copy one more step along than the
last. Nine more rows appear once there is a step to describe: how far a copy is
moved (Repeater position x and y), turned (Repeater rotation) and scaled
(Repeater scale) from the one before it, the point it turns and scales about
(Repeater anchor x and y), and how opaque the first and the last copy are
(Start opacity and End opacity) — the ones between fade evenly from one to the
other.

That is a whole family of motion-graphics staples from one row: a row of ticks
is a position step, a clock face is a rotation step about an anchor, and a
spiral is both at once with a scale under a hundred. **Copy offset** says which
copy the original art is, so a negative one puts copies *behind* it — handy when
you want the drawing to stay where you left it and the copies to grow the other
way.

A copy is a scaled *drawing*, not a scaled path: halve the scale and the
outline halves with it, and so do the dashes, or a small copy would look like a
different shape with a heavy border. The copies are drawn back to front, so the
original stays on top of everything made from it.

Two things worth knowing. The layer's box **grows to hold the copies** — it has
to, or they would be drawn off the edge of the layer's own picture — and since
the layer's position is pinned to that box's corner, a repeater animated to grow
upwards or leftwards moves the art as it plays. Step down and to the right, or
key the layer's position to compensate. And the count stops at a hundred: every
copy is a full pass of the rasteriser, so an unbounded count would be an
unbounded frame.

### What the lock switch does

Locking a layer means **no edits until unlocked** — not just the obvious ones
like dragging its bar, cutting, renaming, reordering or deleting, but
everything reachable through its twirl-down too: position, effects, volume.

The refusal lives in the **engine**, in the one place every edit passes through
on its way into the document. That matters more than it sounds: there are
twenty-nine different kinds of edit a layer can receive, and guarding them one
interface control at a time means remembering to do it again every time a new
control is added — the newest rows are always the ones such a guard forgets.

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
under the Timeline — and it wants to land on the things already there: the
start or end of a layer, a cut inside a sequence, another keyframe, a marker,
the playhead, the edges of the work area — not merely on whole frames.

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

Right now this covers dragging a keyframe on its lane. Dragging a layer's bar,
the razor, the work-area handles and markers themselves still land wherever you
point. The arithmetic is written once and shared, so each of those is wiring
rather than a fresh design.

### Zooming that flies, and a slider that means something

**The zoom moves rather than jumping.** Magnification is a *place* changing, not
a number being nudged: jump straight from one zoom to another and you lose where
you were. The Viewer and the Timeline share one piece of code for that flight.

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

Two details of that anchoring are rules, not accidents (K-320). The slider's
anchor — where the playhead is on screen — is measured once, when the drag
begins, and held for the whole gesture: measuring afresh on every drag update
reads a fresh zoom against a scroll offset that has not been corrected yet, two
numbers describing different moments. And the width the correction works from is
the scroll position's own content extent — the same numbers the correction is
applied with — never a viewport size cached during build, which disagrees by a
little at every zoom and by more the further in you are.

**And a zoom only rebuilds the lanes.** This is the other half of the same
problem. The Timeline is two halves of one table: the layer names on the left,
the bars on the right. Nothing on the left depends on the zoom, so the
right-hand half listens for the zoom by itself and the left-hand half sits
still — redrawing the whole table on every fraction the zoom moves would mean,
during an animation, sixty redraws a second, each asking the engine again for
the work area, the render cache and more. The Timeline does exactly the same
for the playhead, for exactly the same reason.

A plain wheel still scrolls, as it always did — it never zooms without a
modifier, which is a rule the specification is firm about and this did not
change.

The graph editor and the Project panel's thumbnails still cut rather than fly.
They are the same job, and the shared piece is written.

### Magnification and preview resolution are two different things

Both sound like "zoom", and confusing them is how a viewer ends up lying to
you. `flutter_ui/lib/state/viewer_view.dart` is where the two words are kept
apart.

**Magnification** is how big the picture is *drawn*. Zooming to 400% does not
ask the engine for a single extra pixel — it takes the frame that arrived and
draws it four times the size. That is deliberate: if zooming out lowered the
resolution, every frame already banked in the cache would be worthless the
moment you leaned back to see more of the composition, and every frame would
have to be made again on the way in.

**Preview resolution** is the opposite: it changes what the engine is asked to
make. Half renders a quarter of the pixels (half the width *and* half the
height), so a heavy composition previews four times cheaper and looks
correspondingly softer. It is the honest trade you reach for while working on
something slow, and it can never reach the export — the export builds its own
renderer and is never told about it.

Two small pieces of plumbing make this work without the shell reaching into a
panel that may not even be on screen:

- **The magnification is asked for, not set.** *Fit* is a rule ("the whole
  picture in the panel"), not a number, and only the Viewer knows its own size —
  so View ▸ Fit, `Shift+/` and the command palette all bump a request that the
  Viewer answers if it is mounted, and nothing at all happens if it is not. The
  request carries a running number so pressing Zoom in twice is two events
  rather than one; a plain "the value is still zoomIn" would be no change at
  all and the second press would be swallowed.
- **The scale the engine is asked for is a multiplication.** The panel measures
  itself and reports the scale its size implies; the preview resolution is a
  fraction on top of that. `LumitUiState.viewerScale` multiplies the two on
  every read, which means a change to either is in force on the very next
  render request and there is no third number to keep in step with the other
  two.

### Moving between panels without the mouse

`Ctrl+F6` moves the focus ring on to the next panel, `Ctrl+Shift+F6` back, and
`Ctrl+F` puts the cursor in the focused panel's search box. Three small things,
and two of the details in them are worth knowing.

**The ring walks the arrangement, not a list of panels.** The order is the one
the dock tree is visited in — roughly left to right and top to bottom — so a
panel you have closed is simply not in the cycle, and rearranging the workspace
rearranges the ring with it. A panel sitting behind a tab is brought to the
front as the ring reaches it, because a focus ring on something nobody can see
is a keystroke that appears to have done nothing.

**The search chord asks rather than reaches.** The field belongs to whichever
panel is focused, and the shell has no business reaching into a panel — so
`Ctrl+F` bumps a request, and each panel that owns a search box listens and
answers only when it is the focused one. That is what makes it impossible for
one keystroke to focus two fields, and it means the chord is a quiet no-op in
the six panels that have no search box rather than doing something arbitrary.

There was a third thing to fix before either could work at all. Lumit asks the
engine what a chord means *in the focused panel's context* — and the keymap's
"Panels" context is one that no panel actually **is**, since its whole subject
is moving *between* panels. So the lookup never asked for it and the three
bindings were unreachable by construction. The keyboard now asks the Panels
context after the focused panel and the app-wide table have both declined,
which is exactly what it already did for the toolbar's "Tools" context, and for
the same reason.

### What is remembered, and where

- **The workspace** — panel arrangement, colour scheme, interface scale, tooltips,
  keymap, modal window positions. One `Workspace` object, written to a
  machine-local settings file. Nothing personal reaches the project file.
- **The session** — open comp tabs, front tab, playhead, selection, the Viewer's
  exposure and tone map per comp, and the panel
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
It lives in the settings file, which is machine-local — so without a way out, a
theme would be stuck on the computer it was made on. Three things give it one,
all in the Flutter frontend and none of them touching the engine:

- **A theme is a file.** `flutter_ui/lib/theme/theme_file.dart` writes one out as
  `.lumtheme` — a short, indented JSON document you can read: what it is, a
  version number, the theme's name, whether it is a light or a dark theme, and
  every colour as a hex code like `#35785e`. Settings → Appearance has **Export…**
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

### Workspaces you save yourself

Lumit ships six arrangements of panels — Edit, Effects, Nodes, Colour, Audio,
Retiming — and they are the six names along the right-hand end of the toolbar.
A **user workspace** is a seventh, or an eighth: the arrangement you happen to be
looking at, saved under a name of your own, from Window ▸ Workspace ▸ *Save as
new workspace…*. It then joins the strip after the shipped six and behaves
exactly like them — you click the name and the panels move, and nothing closes,
reloads or recalculates.

- **Each one is its own small file.** They live in `workspaces/` beside the
  settings file (`%APPDATA%\lumit\` on Windows), one readable JSON document each:
  what it is, a version, the name, and the panel tree. Never in the project —
  a project you send somebody carries the *edit*, not the shape of your screen.
  Because the stored file and the shared file are the same document, **Export
  workspace…** is a copy of it and **Import workspace…** reads one straight back.
- **The name is the identity**, exactly as a theme's is, so nothing overwrites
  something you already have: saving or importing a second "Grading" gives you
  "Grading 2" and says so in the status line.
- **A workspace you are in keeps what you drag it into.** While one of your own
  is ticked, moving a splitter or re-docking a panel is written back to its file.
  Under a shipped preset it is not — a preset's factory layout is not yours to
  overwrite, so your dragging changes the arrangement on screen and leaves Edit
  as Edit.
- **`Alt+Shift+1…9` picks by position on the strip**: the six presets first, then
  your own in name order, so `Alt+Shift+7` is the first workspace you saved and
  stays the same one on the next launch.

One deliberate gap, shared with the presets: after a restart **nothing is
ticked**. What Lumit remembers across launches is the arrangement itself, which
you are free to drag about, so a ticked name could claim a layout the panels no
longer match. Click a name and the tick — and the write-back — come straight
back.

### The safe triangle under a submenu (K-318)

Open a menu, hover a row with an arrow on it, and a second menu flies out to the
right. The natural path towards it is a diagonal — up-and-right, or
down-and-right — and that diagonal crosses the rows *underneath* the one you
started on. A menu watching only for "which row is the pointer over" would see
one of those rows, decide you had changed your mind, and take the flyout away
before you got there — the tax every such menu makes you pay is travelling the
corner instead: straight right first, then down.

The guard against that is old and has a name — the **safe triangle**. While a
flyout is open, an imaginary triangle runs from where your pointer is to the two
near corners of the flyout. Anything inside that triangle is *travel*: you are
on your way to the flyout, whatever row you happen to be passing over. So while
the pointer is in there, the menu holds the switch back rather than acting on
it.

Two details stop that from becoming its own annoyance:

- If the pointer *stops* inside the triangle — you were heading for the flyout,
  changed your mind, and settled on a row — the held switch lands anyway after
  about a third of a second. Resting on a row still means that row.
- If the pointer moves somewhere the triangle does not cover — straight down the
  menu, say — the switch happens immediately, with no delay to feel. Only travel
  towards the flyout is ever held.

The geometry is a separate little file (`widgets/hover_intent.dart`) with no
Flutter in it beyond the `Offset` and `Rect` types, which is what lets it be
tested as plain arithmetic: is this point inside this triangle? The timers and
the "which row is hovered" bookkeeping live with the floating menu surface every
popup already shares, so the menu bar, the Add effect browser and every
right-click menu get the same guard from the same code.

A guard like this is invisible when it works and invisible when it does not,
which makes it miserable to test by feel. So the Debug panel has a switch —
**Show safe hover triangles** — that draws the live triangle over the menus: the
shape, and a small ring at the apex where the pointer left the owning row. It
turns amber at the moment the guard is actually holding a row switch back, which
is the moment worth watching. It only draws; the guard cannot see it and decides
exactly what it would have decided with the switch off.

### One menu at a time (K-519)

Menus, dropdowns, pickers and right-click menus are all the same thing
underneath: a small floating panel painted into the window's *overlay*, with an
invisible full-window sheet behind it that catches the click that dismisses it.
Every one of them is raised by the same function, `showLumitPopup`.

For a long time each call was on its own. It pushed its own floating panel, put
its own invisible sheet behind it, and knew nothing about any other menu that
happened to be up. Nothing enforced the obvious rule — that only one menu should
be open at once — and with a quick enough pointer you could break it: skate
across the menu bar, an Add-effect list and a colour picker and end up with
three menus on screen, each needing its own separate click to make it go away.

The fix is a single list, held in one place: **the chain**. When a menu opens it
joins the end of the chain. Which end it joins tells you everything:

- Raised from *inside* an open menu — a submenu flying out of a row — it
  **extends** the chain. The parent stays; the two belong together.
- Raised from anywhere else — the menu bar, a panel, a toolbar button — it
  **replaces** the chain. Whatever was open closes first, automatically, with no
  opener needing to know what else exists.

Everything else follows from that. One click on the sheet dismisses the whole
chain rather than peeling one layer off it. Escape does the same. Hovering from
one menu-bar heading to the next hands over, because the second menu's opener
sits outside the first menu. And no individual menu had to be taught any of
this: the rule lives in the one function they all go through, which is why it
also covers the menus nobody has written yet.

A menu knows whether it is inside another because the popup wraps its contents
in a marker widget carrying its position on the chain. A widget can always ask
its ancestors what surrounds it, so an opener finds the marker if there is one
above it and finds nothing if there is not — which is exactly the question
"am I inside a menu?".

### Ctrl+A means "everything here" (K-522)

Select all used to mean one thing wherever you pressed it: every layer in the
composition. In the Project panel that was plainly wrong — it selected things
you could not see and left the list in front of you untouched.

What "everything" is now depends on which panel has the focus ring: items in the
Project panel, layers in the Timeline, effects in the Effect controls panel, and
— when its selection grows from one node to a set — nodes in the Node graph. The
shell does not decide any of that. It bumps a
counter that every panel can watch, and each panel answers only if it is the
focused one — the same arrangement `Ctrl+F` already used to put the cursor in
the right search box. Where no panel claims the chord, it still means every
layer, which is what the Timeline wants and what the Edit menu's own row does.

### How windows answer the keyboard (K-319)

**Every house control holds focus.** The buttons, the checkboxes, the radios,
and the resting state of a value box all carry a **focus node**, so Tab reaches
them. A focused control draws the accent ring the design spec asks for, and
answers Enter (and Space, where "press" is what it does).

**"Enter presses the OK button" is not a special case.** Rather than wiring one
button to the Enter key, each window says which of its buttons is the
*default* — the affirmative one, or the safe one where the affirmative deletes
something — and that button simply **takes focus when the window opens**. Enter
then presses whatever is focused, which is that button until you Tab somewhere
else. One rule instead of a special case, and it means a whole window is
operable from the keyboard rather than just its OK button.

There is a companion rule covering text fields and controls alike: while one
holds focus, the application's global shortcuts stand down. Otherwise Enter on
a dialogue's button would *also* rename a layer in the Timeline behind it
(K-243).

**Escape closes a window.** A Lumit dialogue is not a Flutter *route* — it is a
panel painted into the overlay — so it cannot lean on route behaviour. It takes
its place on the Escape ladder instead (below), on the rung for dialogues, and
closing means the same as clicking the dimmed background: the window goes and
answers "cancelled".

**Tab goes the way you read.** Left to right, then top to bottom. It sounds
like it should be the default, and it is not: the toolkit walks the *widget
tree*, and a layout that nests a column inside a row visits things in whatever
order the code composed them, which can be down-then-across, or worse. Flutter
ships a policy that sorts by actual screen position instead
(`ReadingOrderTraversalPolicy`); every modal is wrapped in one, plus a focus
scope so Tab cycles inside the window rather than wandering off into the panels
behind it.

### How a value box tells a scrub from a click (K-319)

A value box in Lumit does two jobs: drag it sideways to scrub the number, click
it to type one. Deciding which of those you meant happens in Flutter's *gesture
arena* — the pointer goes down, and whoever recognises a gesture first wins.
Move sideways more than a few pixels and the drag wins; stay still and the tap
wins. One case belongs to neither: a press that wanders a little but never
scrubs a whole step. Mice wobble constantly, so a drag that ends without ever
crossing one increment is understood for what it is — a click — and opens the
editor, rather than releasing into nothing.

Two more things about that editor. It opens with the whole value **selected**,
because a value is retyped far more often than it is amended, and a caret parked
at the end means every edit begins with a select-all of your own. And it has
real text selection: press to place the caret, drag to highlight. That is not
something you get for free — a bare `EditableText` takes keys but has no
selection gestures attached until you wrap it in the builder that provides
them.

The ordinary text fields take focus on the pointer's **down** stroke rather
than waiting for the tap to resolve. Somebody who presses in a field and
immediately drags is selecting text in one motion, and the field has to be
theirs before the highlight starts or the drag selects nothing.

### One rule for renaming (K-321)

**`Enter` renames whatever the focused panel has selected** — a layer in the
Timeline, an item in the Project panel, an effect in Effect controls.
Double-clicking, and clicking an already-selected row, mean *open* everywhere,
without exception. Nothing renames on a click, because clicking a selected row
is the same gesture as a slightly slow double-click — a rename that opens on it
keeps landing under pointers that meant "open".

Each panel's Enter is a separate keyboard binding in a separate context, which
is how one press cannot open two editors at once: a per-panel binding is only
live in the panel that has focus, and each panel's handler checks that it is
the focused one before acting.

**Effects can carry a name of their own.** A blur called "Gaussian blur" three
times down a stack tells you nothing; "Blur the sign" does. An effect instance
has one optional field, `custom_name`, shown in place of the effect's label
where it is set — in the Effect controls heading and in the Timeline's fold-out
alike. It is a *display* name only: `match_name` is the schema key everything
else looks the effect up by, so nothing about rendering or expressions changes,
and clearing the field puts the label back.

The saved-file rule is the one that makes a field like this safe: it is written
**only when it is set**. A project with no renamed effects is byte-for-byte the
file it was before, and an older project simply reads as "no name given".
Nothing needs migrating.

### Leaving an inline editor (K-243, K-323)

Every ordinary way of leaving an inline editor — an effect's name, a layer's
name, a Project item's name, a value box — *keeps* what you typed: pressing
Enter, clicking somewhere else, tabbing away (K-243). Abandoning a rename by
clicking elsewhere should not silently throw the work away. **Escape is the one
way out that writes nothing** (K-323): the editor shuts and the thing keeps the
name or the number it already had.

Escape is attached to each editor's own **focus node** rather than to the
general shortcut machinery, and the reason is specific. The "dismiss" request
Flutter binds Escape to travels up the widget tree looking for a handler — that
is how modals close — but an inline editor is not a dialog, it is a text field
that has replaced a label where it stood, so the request finds nobody. And
Flutter's text editor has its own idea of what dismissing means (hiding the
selection toolbar), so a handler placed above it could be quietly swallowed.
A focus node gets first refusal on keys, which is the one place the handling
cannot be intercepted.

One trap worth remembering, because it produces a bug that looks like the
opposite of what you wrote: a value box commits its number whenever it loses
focus, and closing the editor is *what loses focus*. So cancelling marks the
editor closed before it takes it down — otherwise Escape would commit the very
value it was asked to discard.

### The Ctrl+Space console, and why a ring beats a list (K-324, K-325)

Press Ctrl+Space and a ring of choices appears **around your mouse** —
anywhere, including over the picture in the Viewer — with a search box
floating just above it (or below, if your pointer is near the top of the
window). Nothing is boxed: the console floats translucent over your work,
because your work is the thing it is about to act on, and behind it the frame
dims just a little — enough that every slice stays readable over any picture,
never enough to hide what you are working on. While it is open, the keyboard
is the console's: anything you type goes into the search box, never into the
panels underneath, so a keystroke can never rename a layer you did not mean
to touch. Escape closes it from anywhere; so does a click outside. Two ways
into the same handful of things, because they suit different moments.

**The ring is a radial menu**, the kind Blender uses. The point of a ring is
not that it looks better than a list. It is that every choice is in a fixed
*direction*, and the ring opens where your hand already is — so after a few
uses your hand knows "duplicate is up" and stops reading the menu at all: you
press the chord, flick, and it is done. A list can never offer that, because a
list's third entry moves the moment the list grows.

Two rules follow from that, and they are most of the code:

- **A slice is chosen by angle, not by what you are hovering over.** Flick in a
  direction and the choice is made, however far the pointer actually travelled.
  If it were hit-testing a drawn wedge you would have to land *inside* the
  shape, and the gesture could only be as fast as your aim.
- **There is a dead zone in the middle.** Inside it nothing is chosen, so
  opening the menu and letting go without moving cancels — rather than
  committing you to whatever happened to be nearest the cursor.

What is *in* the ring depends on what you have selected. An item picked in
the Project panel (while you are standing in that panel) offers exactly one
thing: **Add to comp**, which places it the way dropping it on the Timeline
would — and when it cannot be placed (no comp open, a folder, a comp into
itself) the slice sits there dimmed rather than vanishing, so the direction
is learned before it is ever needed. An effect picked out
in the stack offers the things you do to an effect; a selected layer the
things you do to *that layer* — duplicate, add an effect, pre-compose, delete
— and never a stray "new solid" beside them, because creating a layer is not
something about your selection. Creation is still one flick away: the **New**
slice carries a small caret, and choosing it expands the ring in place into
the same six entries as Layer ▸ New, the way Blender nests its pies. The
centre of the ring always names where you are, and inside a sub-ring it is
also the way back out (so is Escape). A composition with nothing selected
offers the new-layer ring directly, because that is what an empty timeline is
asking for; with nothing open at all it offers the two ways to get somewhere.
Never more than six to a ring — a ring of twelve is a ring nobody learns, and
the long tail is the search box beside it. An entry that cannot run right now
is dimmed rather than removed, so a direction your hand has learned keeps
meaning the same thing tomorrow.

A selected layer's ring has one more trick: the **Keyframe** slice. It expands
into one slice per transform row — Anchor point, Position, Scale, Rotation,
Opacity — and choosing one plants a keyframe at the playhead holding whatever
value is already there, so nothing on screen moves. Then the Timeline comes to
the front with that row open, and the key you just made is sitting there under
the playhead. It is the flick-sized version of twirling the layer open,
finding the row, and pressing its diamond — the three steps it replaces. A row
already keyed at this frame is not keyed twice, and a row driven by an
expression is dimmed, because writing keys over an expression would erase it.

**The search box starts empty and shows nothing** — the ring is the offer.
Start typing and the ring steps aside for a dropdown of matches under the box:
type "gau", press Enter, and Gaussian blur is on every selected layer. The
dropdown puts **effects first and compositions after a divider**, and — this
is the deliberate part — a composition can never outrank an effect however
well it matches. The reason you hit this key is nearly always an effect; a
comp that happened to score better would just be in the way. The comps are
there so the same box can also be "take me to that comp". Escape backs out one
step at a time: it clears what you typed before it closes anything, so a
mistyped search never costs you the whole console.

This half is modelled on Video Copilot's **FX Console**, which is the plug-in
After Effects users install first and then cannot work without. That includes
its **snapshot button** beside the box: one press writes the frame you are
looking at to a PNG, so you can change a look and compare the two without
setting up an export.

Worth knowing how the snapshot is done, because the cheap way was also the
right way: it is a **one-frame export**. Lumit's exporter already writes PNG
sequences, and it is the tested path from a Lumit frame to a file — colour,
sizing, all of it. So a snapshot is that, with the range set to "this frame and
the next". The alternative would have been a second still-writer living beside
the exporter: a second thing to keep correct, for nothing gained. The file goes
into a `Snapshots` folder beside your project, or your pictures folder if the
project has never been saved — never into whatever directory the application
happened to be started from, which is where a bare file name would have put it.

One small mechanism makes "opens at the mouse" possible at all: a keyboard
event does not know where the mouse is. So the shell keeps a note of the last
place the pointer was seen — a single remembered position, updated as the
mouse moves, costing nothing — and the console reads it when the chord lands.
The note is taken at the door rather than in any one room: pointer events are
recorded globally, before the interface decides which widget they belong to,
because some regions (the Viewer's picture is drawn by the engine, not by a
widget) belong to no widget at all, and a note taken inside the widget tree
went stale exactly there.

**Why this is not the command palette.** Ctrl+Shift+P still opens that, and the
two are not competing: the palette is *every command by name*, the console is
effects plus the thing you were about to do. Both build their lists in the same
file as the menu items, so neither can drift into a different idea of what
"New composition" means.

No pie-menu library is involved: a ring is a stack of positioned
labels over a gesture detector, and the only real content is the arithmetic of
which slice a direction means — which is why that arithmetic, and the sums
that place the ring and bar on screen, live in their own file,
`widgets/radial_maths.dart`, with no Flutter in it and a test that treats it
as pure maths.

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
- **A child that comes and goes beside a child that holds state needs a key**
  (K-328). Flutter matches the children of a `Stack` or `Column` by *position in
  the list* unless they carry keys, so removing one shifts every later sibling
  onto its neighbour's element — and anything living in that element (a text
  field's editing session, a scroll position) is quietly rebuilt from nothing.
  In the FX console this cost the search box its keyboard connection the moment
  the ring was hidden, so typing worked for exactly one letter.
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
- **Ordinary text is regular weight; Medium is emphasis only** — dialog headings
  and the dock's tab pills (`bodyStrong`), per docs/15-DESIGN.md §7.1 (K-316).
  When everything is emphasised the emphasis stops meaning anything, so both
  font files are bundled and `theme_test.dart` pins which styles use which
  weight, so nothing drifts back to all-Medium without saying so.
- **Tests must let real time pass.** `settleFrb` in
  `flutter_ui/test/frb/frb_test_support.dart` alternates real-time slices with
  fake-clock pumps until the expected state arrives; `await tester.pump()` alone
  advances no clock, and awaiting an engine call inside `runAsync` that was not
  started there deadlocks (K-233).

### Two marks for one yes-or-no: the tick box and the switch

Lumit draws an on/off setting two ways, and which one you see is decided by the
drawing of the surface you are looking at rather than by what the setting does.

- **`HouseCheckbox`** is the little square you tick — a 9px outlined box with a
  block in it when it is on. It is the mark inside a *panel*, where a column of
  them reads as a list of things that are either true or not.
- **`HouseToggle`** is the switch you flick — a 22×12 pill with a knob that
  slides from one end to the other, amber when it is on. It is the mark in the
  *Settings window*, because that is what the approved drawing puts there
  (K-465).

Neither is the accent colour. The accent is Lumit's one loud colour and the
design rules ration it to the single filled button on a surface, to focus, and
to whatever is currently chosen; a page of a dozen switches would spend it a
dozen times over and it would stop meaning anything. The switch's "on" is the
same amber that marks a property with keyframes on it, which is the only other
stateful colour the chrome has.

Both answer the keyboard: Tab reaches them, Space flips them, and a focused one
draws a ring rather than moving by a pixel.

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

### Reading the memory report (K-294)

**The problem this solves.** Lumit keeps several stores of pictures: finished
frames in memory, finished frames on the graphics card, decoded frames from
video files, frames queued to be written to disk. Each has a budget and each
throws things away to stay inside it. When the whole process is holding more
memory than it should, either one of those stores is misbehaving, or something
outside all of them is holding memory nobody is counting — and from outside the
program those two look identical.

**The report is a subtraction.** Settings ▸ Performance opens with a Memory
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

Counting them, rather than measuring them, is deliberate. Asking the driver for
bytes only has an answer on Windows and Linux — a Mac's driver does not report
it — while a count is a count on every machine, and it happens to be the sharper
question anyway: it distinguishes a big cache from a leak, which bytes alone
cannot.

Two things behind that row are worth knowing (K-295). Telling the graphics card
"I have finished with this picture" does not give the memory back — it marks it
finished, and the memory returns the next time the program asks the card to
tidy up. A program drawing to a window asks constantly without meaning to,
because showing a frame *is* asking; Lumit spends much of its time drawing into
its caches instead, with no window involved. So the engine asks once per turn
of its own loop, whether anything is on screen or not — which costs nothing
when there is nothing to collect, and means finished pictures never pile up
waiting for something to ask.

And the asking comes in two forms. "Collect anything you have finished with,
and don't keep me waiting" is the one the loop uses, because a loop that must
produce a picture cannot afford to stand still. But work handed to a graphics
card happens when the card gets to it, not when it is handed over, so there is
always a queue of frames in flight — and everything still in that queue is
memory the card cannot possibly release yet. A **measurement**, this report or
a test reading it, must use the other form — "finish what you have, *then*
collect" — or the queue reads as a leak: the number only means anything once
the card has caught up.

The report is a **debug-build tool**: it is there while a fault is being hunted,
and a shipped Lumit does not show it. Asking somebody editing a video to
interpret a live texture count is handing them the engineering instead of the
tool.

## 10. The app icon and the brand files

The icon you see in the taskbar is not one picture — it is a small bag of
pictures. Windows keeps them all in a single `.ico` file (ours holds seven
sizes, 256 pixels down to 16), and shows whichever one fits the spot: big for
the desktop, tiny for a browser-style tab. macOS wants something different
again, and gets its own section below.

Nobody draws seven pictures by hand. The artwork is drawn **once**, as an SVG —
a text file of drawing instructions ("a rounded square here, this gradient
there") that can be rendered at any size without going blurry. The five SVGs in
`assets/brand/` are the only files a human edits:

- `lumit-mark.svg` — the mark itself: two keyframe diamonds overlapping, white
  where they cross. This bare form is the Windows and Linux icon.
- `lumit-icon.svg` — the same mark sitting on a dark rounded tile. Nothing
  ships from this file any more; it is kept as the flat reference drawing the
  macOS icon below was built from, and as the picture to hand anyone who asks
  for "the icon" as one image.
- `lumit-project.svg`, `lumit-preset.svg` and `lumit-theme.svg` — document
  icons for `.lum` project files, `.lumfx` presets and `.lumtheme` colour
  themes: a dark page with a folded corner and the mark inside, like the little
  badge on any Photoshop or After Effects file. The theme one carries three
  overlapping colour swatches instead of the mark, since colours are what is in
  the file.

`scripts/gen-icons.py` turns those drawings into every pixel file Windows and
Linux want (run `pip install resvg-py pillow` once, then
`python scripts/gen-icons.py`). It renders each size straight from the SVG
rather than shrinking one big picture — that is what keeps the 16-pixel
version crisp instead of mushy. You only run it after editing an SVG; the
generated files are committed, so a fresh checkout builds without it.

### The macOS icon is a stack of layers, not a picture (K-309)

macOS 26 stopped treating an app icon as a flat image. Its icons are made of
**layers**, and the system lights them itself: it puts the rounded tile behind
them, bevels the edges, adds the shadow, and slides a highlight across the
glass as you tilt the window — plus the dark, tinted and clear variants a user
can switch the whole Dock to. Handing it a finished picture gets none of that;
it has to be given the pieces.

So the macOS app icon is not a PNG we render. It is
`assets/brand/lumit-icon.icon` — a small folder Apple's free **Icon Composer**
app writes, holding the six pieces of the mark as separate SVGs (tile, bloom,
blue key, magenta key, core glow, core diamond) and an `icon.json` saying how
they stack: which ones are glass, how opaque each is in dark mode, how deep
the shadow goes. Open the folder in Icon Composer to change any of it.

One catch, if you do (K-312). Icon Composer can save two settings that Apple's
own compiler then refuses: a `features` list at the top of `icon.json`, and a
`specular` written as the word `"inside"` rather than simply on or off. Neither
is anything the icon needs — the first only lists features it uses elsewhere in
the file, and the second just says whereabouts on a layer the shine sits — but
either one stops the icon compiling, and the error you get says the file could
not be opened, which sends you looking at the artwork instead. `flutter build
macos` is where it bites. To save the wait, `scripts/check-icon.py` looks for
both in a second and runs on every push; if it complains after you have edited
the icon, delete the two settings it names and nothing about the picture
changes.

Those layers look slightly *unfinished* next to the flat icon, and that is the
point. The flat drawing paints in its own lighting — a rounded corner on the
tile, a shadow under the keys, a dark rim around each key standing in for an
edge catching light. macOS 26 draws all three for real, so the layers leave
them out; painted in, you would get each one twice, and a hand-drawn shadow
that does not move when the system's does looks worse than no shadow at all.

Xcode compiles that folder during the macOS build (the `.icon` is listed in
the Runner target, and `ASSETCATALOG_COMPILER_APPICON_NAME` names it), and
from the same layers it also generates the old-style flat `.icns` for Macs
before 26 — so one source covers every macOS version, and there is nothing to
regenerate by hand. The loose PNGs that used to live in
`Runner/Assets.xcassets` are gone; they were the old flat icon, and keeping
two sources of the same artwork is how they drift apart.

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
  the drag. macOS needs no registry writing and no install script: an app
  *declares* the types it owns in its own Info.plist, and the system reads
  that the first time it sees the app. The three document icons
  (`packaging/macos/lumit-project.icns` and friends) are resources of the app
  target, so they travel inside the bundle where those declarations point at
  them. What is still missing is double-click *opening* — the declarations
  tell macOS which app owns a `.lum`, but the app is handed the file through
  `application:openFile:`, which it does not yet answer; that lands with the
  larger macOS pass in the TODO.

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
confuse. **Signing** puts your identity on the app: this came from the project
owner's Apple Developer identity, and here is Apple's certificate saying
Apple agrees that is a real person. **Notarisation** is a second step where you upload the finished app to
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

Whole-word lookup works for a *label*, and not at all for a **sentence with a
fact in the middle of it**. "This config needs FixedFunctionTransform, which
Lumit does not support yet" is a different whole text for every transform there
could ever be, so no table could hold them all. For those the engine sends the
*pieces* instead: a short unchanging id — `unsupported_transform` — and the
facts by name — `name: "FixedFunctionTransform"`. The interface writes the
sentence in the reader's own language from those, which also lets a translator
put the blanks wherever their grammar wants them. The After Effects import
report's rows work this way, and so does the reason a colour config is not in
use.

One rule inside that, worth knowing because it looks like an inconsistency: the
facts are **not** translated. They are names out of somebody else's file — a
colour space, a look-up table, a path — and they must come back exactly as they
arrived. A config that happens to call one of its colour spaces "Alpha" means
that word as its own name, not as the name of an effect's Alpha parameter, and
translating it would be renaming the user's work.

### A tooltip is a name and a shortcut

`docs/07-UI-SPEC.md` §13.2 says a tooltip is the control's **name and its
shortcut**, and reserves the sentence-length kind for genuinely Lumit-specific
ideas. A Reset button whose tooltip reads *"Put every parameter back to its
default, removing its keyframes"* is a sentence you have to stop and read to
learn something you already knew. So every tooltip is under five words and most
are two — *Reset all parameters*, *Add keyframe*, *Label colour*. Six are
allowed to be longer, and they are listed by name in `test/l10n/arb_test.dart`
with the reason: the three cache meters, whose tooltips carry live numbers and
warn you that clicking throws work away, and the two playback modes. Anything
else that grows past five words fails that test.

### Choosing a language

Settings ▸ Interface ▸ Language. It defaults to whatever the machine itself is
set to, and stores nothing until you choose — so if you never open the picker,
Lumit follows your operating system for ever, including after you change it.

The list names each language in its own language: Deutsch, Қазақша, Українська,
简体中文, 繁體中文. That is deliberate. Somebody who has set Lumit to a language they
turn out not to read needs to be able to find their way back, and they will not
do it by looking for the word "German".

The words themselves are not written here. `lib/l10n/app_en.arb` is the one file
anybody types English into; every other `app_*.arb` beside it is sent back by
Crowdin, the site the translators work on, and editing one of those in this repo
achieves nothing — the next sync writes over it. So a wrong translation is fixed
on Crowdin, and so is anything about *which* languages exist.

One trap is worth knowing, because it stopped the build twice (K-311). Each of
those files names its own language twice: once in its file name, and once in a
key inside it called `@@locale`. Flutter refuses to build if the two disagree,
and Crowdin fills that key in with its own spelling of the language — "zh-CN"
where Flutter wants "zh". `test/l10n/arb_test.dart` compares the two on every
run, so the failure says which file and what to do about it rather than the
whole build stopping with an error about locales.

The cure was supposed to be a setting on Crowdin. It is not: Crowdin's language
mapping renames the *file* — which works, and is why the file is called
app_zh.arb at all — but the value written *inside* it comes from somewhere the
mapping does not reach, and every sync has written "zh-CN" regardless. So the
repair is automatic now. Crowdin pushes to a branch of its own, and a small job
(`.github/workflows/translation-locale.yml`) meets it there, puts each
`@@locale` back to whatever the file name says, and pushes the result on. By the
time anybody sees a translation pull request the names agree. Nothing else in
those files is touched — the words are the translators', and a locale name is
bookkeeping.
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

### A mask that moves, and the number in the file that stays a number

A mask is a drawn shape that decides which of a layer's pixels show. Until now the shape
could be animated but nothing in the interface could reach that, and the three numbers
beside it — how see-through the mask is, how soft its edge is, how much it is grown or
shrunk — could not be animated at all. Now all four animate, and they do it with the same
stopwatch, the same ◄ ◆ ► and the same diamonds as a layer's position (K-340).

For the three numbers the change was to make them the same *kind of thing* the rest of the
program already animates. Everywhere else, an animatable number is a "property": a little
box that holds either one value or a list of keyframes. A mask's opacity used to be a plain
number in a box of its own. Making it a property means every control that already knows how
to key a property works on it immediately, with nothing rewritten — which is why it now
behaves exactly like a transform rather than *almost* like one.

That change had a trap in it, and the trap is worth understanding because the same one
comes up whenever a stored value grows. A property normally writes itself into the saved
file as a small object — something like `{"animation": {"Static": 100}}` — where the plain
number wrote `100`. Two things break if that happens. Every project ever saved has the old
shape, so they would all need converting. And Lumit names every finished frame it has
stored by, among other things, the exact text a layer's masks turn into; change the text
and every name changes, so every frame anyone has banked is suddenly unrecognisable and has
to be drawn again. So the three fields keep their own private spelling: while the value is
still, it writes as the bare number it always wrote, and only a mask somebody has actually
keyed writes the longer form. Reading accepts both. An untouched mask is byte-for-byte what
it was.

The shape itself is the odd one out, and stays so. A keyframe on it holds a whole path
rather than a number, so there is nothing to plot on a value graph — its row shows diamonds
and no curve, and no number field, because there is no single number to put there. Its
stopwatch works through the engine's own path-key operations. Pressing the diamond stores
the shape the mask is *already* showing at that moment, so nothing jumps; switching
animation off keeps the shape under the playhead rather than snapping back to the first
key. That is the rule the stopwatch follows everywhere in Lumit.

One more thing changed in passing, and it was a plain bug. A mask can be switched off two
ways: set its mode to None, or take its opacity to zero. Both are meant to mean "this mask
has no say". Both did the opposite — a layer with one switched-off mask went completely
invisible — because the code started from "nothing is showing" and then skipped the very
mask that was supposed to say what *did* show. Now there is one question, asked in one
place, about whether a mask does anything at all; a layer whose masks are all off is simply
the layer it always was. The question takes a moment in time, because opacity animates: a
mask keyed up from zero is genuinely off early in a shot and on later.

Once a mask's shape is animated, one more thing has to follow it: the thin outline the
Viewer draws round the mask. That outline was drawn from the shape *stored* on the mask,
and the stored shape stops being the shape you see the moment the path is keyed — from then
on the picture works out an in-between shape for each frame, and the stored one is only
what the drawing tools last wrote. So dragging a point looked as though it snapped back the
instant you let go, even though the key had landed and the animation played correctly.

The outline now asks the engine where the shape actually is at the frame on screen. It
could have been worked out here instead — the keyed shapes could be sent over and blended
locally — but blending two shapes means matching up their points first, splitting curves
where one shape has fewer points than the other, and a second copy of that arithmetic would
slowly disagree with the one that draws the actual pixels. An outline that no longer traces
the mask it describes is worse than having no outline. Asking the one authority keeps them
identical by construction.

The asking is careful about cost. Only masks that are actually animated are ever sent, so
an ordinary project answers "nothing moved" and pays nothing; and because the Viewer redraws
every time the pointer moves, the answer is remembered against the only two things that can
change it — an edit to the document, and the playhead moving. Hovering asks nothing at all.

### The one curve a shape can draw

Open the graph editor on an animated position and you see its value rise and fall. Open it
on an animated *shape* and there is nothing to plot: a shape is not a number, so there is
no height to draw. The pane used to come up empty, which reads as though the property is
not animating at all — when plainly it is.

There is still one real curve in there. Between one keyed shape and the next, the drawing
crosses from the first to the second, and *how fast it crosses* is a genuine, editable
quantity — it is what the ease handles on those keys control. So each shape key now carries
a plain counting number: the first key holds 0, the second 1, the third 2. Nobody is meant
to read those numbers. What matters is that every span between two keys climbs by exactly
one, so the *steepness* of the line is precisely the rate the shape is changing at. Steep
means the shape is racing from one form to the next; flat means it has almost settled.

Both views show that steepness, rather than the value view showing the meaningless counting
staircase and only the speed view being useful. Everything else in the graph still behaves
as it did — this substitution applies to shapes alone, which are the only properties with
no value of their own.

This is also what After Effects shows for a mask path, and for the same reason.

### A soft edge that is soft in one place and sharp in another (K-545)

A mask's **feather** is the width of the soft band across its edge, and until now it was one
number for the whole shape. A sky replacement wants the opposite: crisp along the horizon,
blending away at the corner. So a mask can now be given a width **per point** of its drawn
shape, and the width runs smoothly from one point to the next along the curve between them.

The interesting part is what that costs, because the answer is "almost nothing", and the
reason is worth following.

Lumit does not feather a mask by blurring it. It first works out, for every pixel in the
frame, **how far that pixel is from the edge of the shape** — a positive number inside,
negative outside. That map of distances is built once, and everything about the edge is read
off it: growing or shrinking the shape (expansion) slides the point where the distance crosses
zero, and the feather is simply the number the distance is *divided by* before it is turned
into an opacity. Divide by 2 and the edge goes from solid to clear over two pixels; divide by
60 and it takes sixty.

Making the feather vary means that divisor stops being one number and becomes a second
picture: a width at every pixel. And the honest width for a pixel is the width of the piece of
edge it is nearest to — because the nearest piece of edge is exactly what its distance was
measured against in the first place.

Which sounds like a second, expensive search: for every pixel, find the closest point on the
outline, then look up what width was written there. It is not, because the algorithm that
computed the distances already knows the answer. The method Lumit uses (Felzenszwalb and
Huttenlocher's) works by sliding a parabola outwards from every starting pixel and keeping,
at each position, whichever parabola sits lowest — and *which* parabola won is the same thing
as *which starting pixel is nearest*. It was being computed and thrown away. Writing it down
costs one extra number per pixel, and it turns "how far to the edge" into "how far, and to
which bit of edge" at no real extra cost. From there the width is a lookup.

The starting pixels themselves come from walking the outline: the same walk that draws the
shape also stamps, into each pixel it crosses, the width interpolated between the two points
it is currently between.

Two things follow, and both matter more than they sound. A mask whose widths happen to be all
the same is spotted before any of this runs and takes the old single-number path, so an
ordinary mask is not made slower by a feature it does not use — and switching the feature on
without changing a width renders a frame identical to the one before. And the widths are
absent from the saved file until somebody actually varies them, so every project ever saved
still writes exactly the bytes it wrote yesterday, and every frame the cache has stored stays
valid.

The same change added the two mask combine modes that were missing, **Lighten** and
**Darken**: where **Add** sums two overlapping masks and can saturate, Lighten simply keeps
whichever of the two is more opaque, and Darken whichever is less.

### The keyer's second half: tidying the matte as a picture (K-546)

Everything the Matte key did until now decided one pixel at a time. It looked at a pixel's
colour, worked out how much like the green screen it was, and turned that into how transparent
the pixel should be — and then it was finished with that pixel and moved on. That is a fast,
simple shape, and it is why the effect was one pass over the frame.

The controls a colourist reaches for next cannot work that way, because every one of them is a
question about a pixel's **neighbours**:

- **Screen pre-blur** — "ignore the grain: judge this pixel by the average of the area around
  it." The catch is that only the *judgement* is softened. What comes out is still the sharp
  original picture; it has just been told its transparency by a blurry twin. Blurring the layer
  and keying the result would be a different, worse thing.
- **Screen shrink/grow** — "pull the whole edge of the cut-out in by a pixel and a half" (a
  green fringe disappears), or push it out. Not a blur: the edge moves and stays crisp.
- **Screen softness** — "blur the cut-out itself", so the join with the new background is a
  gradient rather than a line. The picture keeps its own sharpness; only the transparency is
  softened.
- **Despot black / white** — "there is a single stray dot in the middle of a clean area, get
  rid of it." A dark speck in the kept subject is a pinhole; a bright speck out in the
  background is a fleck.
- **Inside / Outside masks** — "never mind what the key thinks, *this* shape is always kept and
  *that* shape is always cut." The rig in the corner of frame, the hole in the screen behind
  the actor's shoulder.

**How they are made to fit.** The trick is to stop treating the transparency as a number inside
the colour loop and start treating it as a **picture of its own** — a greyscale image, white
where the subject is kept and black where the screen was, exactly what the effect's Screen
matte view already shows you. Once it is a picture, all five controls become ordinary picture
operations, and the effect runs as a short assembly line:

1. Soften the picture the key is judged from (if pre-blur asks).
2. Work out the matte from it and draw it into its own greyscale image.
3. March that image's edge in or out.
4. Blur it.
5. Remove its specks.
6. Paint the garbage masks into it — white inside the Inside mask, black inside the Outside one.
7. Only now, go back to the *original* sharp colour and spend the finished matte on it.

There is a real saving in step 2 being a picture. The blur Lumit already has works on pictures,
so steps 1 and 4 are that same blur, called twice on different things — no second blur was
written, and the two paths (the plain-code reference and the graphics-card version) were
already proven to agree. The matte is stored with the same grey in all four of its channels
precisely so that any existing picture operation can be pointed at it without noticing that it
is a matte rather than a photograph.

**Shrinking without softening.** Growing a shape by a pixel is "take the brightest value in the
little square around each pixel"; shrinking is "take the darkest". Do it once across and once
down and you have moved the whole edge, and — unlike a blur — a pixel's answer is always one of
its neighbours' actual values, so nothing in between gets invented and the edge stays hard.
Because the control is a slider and not a whole number of pixels, the outermost ring of the
square is faded in gradually, so dragging it is smooth rather than a series of jumps.

**What counts as a speck.** A speck is a pixel that **every single one** of its eight
neighbours disagrees with. That definition does the work: a dot alone in a clean area has no
ally, so it is pulled to whatever its neighbours agree on, while a pixel sitting on the real
edge of the subject always has neighbours on its own side and is left completely alone. It is
why the control can be turned all the way up without eating the edges — and why it is a
strength rather than a size. If a wider reach is ever wanted, it is built from the shrink/grow
step, not from a bigger despot.

**The garbage masks come from the mask you drew.** The two rows pick one of the layer's own
masks — the same shapes you draw with the pen tool. What travels to the effect is not a picture
of the shape but the shape itself: the list of straight pieces its outline is made of. The
kernel then asks, for each pixel, "how many times does a ray from here cross the outline?" —
odd means inside, even means outside — and how far the pixel is from the nearest piece, which
is what lets the edge be soft. The softness is the mask's **own** feather, and the mask's own
grow/shrink slides it, so a garbage mask fades exactly the way the shape it was drawn from
fades. There is nothing extra to set, and nothing that can drift out of step.

**And nothing changed for anybody who does not touch them.** Every one of these controls is off
by default, and when they are all off the effect takes the old single pass — not "the new path
with neutral settings", the *old path*, so an existing project keys the identical pixels it
keyed before. That is checked by a test rather than promised: the two paths are run side by
side at the defaults and compared for exact equality.

### Painting on a precomp (K-547)

Paint works by keeping the *gesture* rather than the pixels: the path your pointer took, in the
layer's own coordinates, stamped afresh into the layer's picture every time the frame is drawn.
That works because every kind of layer has a picture waiting on the processor before it goes to
the graphics card — a decoded video frame, a flat solid, a line of type, a drawn shape. The
brush stamps into those bytes and they are handed over painted.

A **precomp** layer is the exception, and the reason is worth a sentence: a composition has no
picture until it is rendered, and it is rendered on the graphics card. There were never any
bytes for the brush to mark, so a stroke painted on a precomp made a Timeline row and no
pixels at all.

The fix is deliberately the boring one. Once the nested composition has been rendered, its
picture is brought back from the graphics card, run through the **same** stamping code every
other layer uses, and sent back. Not a second rasteriser written in the graphics card's own
language — one rasteriser, one set of rules, and a stroke that lands identically wherever it is
painted. The price is that the nested picture makes a round trip through the eight-bit form
every video frame is painted in anyway, and only for a precomp somebody has actually painted
on; an unpainted precomp does not pay a thing. Doing the stamping on the card is the upgrade
if it ever shows up in a measurement, and nothing about what is *stored* would change when it
arrives.

### A square brush, and why that is one line of arithmetic (K-548)

The brush had one shape: a circle. Adding a square one sounds like adding a second rasteriser,
and it is not — it is a change to what the word *distance* means.

Here is how a dab is stamped. Lumit walks the pixels near where the dab landed and asks each
one "how far are you from the middle?". Inside a certain distance the pixel is fully painted;
beyond the brush's radius it is untouched; in between it fades, and how wide that fading band
is *is* the hardness control. Nothing else about a dab exists.

So the shape is not a separate drawing routine, it is the answer to that one question. Measure
the distance as the straight line to the middle and the fully-painted region is a circle.
Measure it as *the larger of the two directions* — how far across plus how far up, whichever is
bigger — and the same fully-painted region is a square. Every other part of the brush is
written in terms of that number and needs no case of its own: the hardness ramp softens a
square exactly as it softens a round, outwards from a flat-sided core; the spacing between dabs
along a stroke is unchanged; even the box the stroke reserves in the picture is the same, since
a square of a given width and a circle of the same width fit in the same box.

Two shapes is the whole of it. There is no brush-tip system here — no imported tip images, no
angle, no squashing a round tip into an ellipse — and deliberately no half-built room for one.
And a stroke that is round says nothing about its shape in the saved file, so every project
ever saved still writes exactly the bytes it wrote before and every frame the cache holds for
it stays valid.

### A stroke that draws itself on (K-549)

Every paint stroke now has two numbers under it in the Timeline, **Start** and **End**, each a
percentage of the stroke's own length. Leave them at 0 and 100 and the whole mark is drawn, as
before. Hold Start at 0 and animate End from 0 to 100 across a second, and the stroke appears as
if a hand were making it — a signature writing itself, an arrow growing across a diagram. That
is the effect After Effects calls write-on, and it is why paint animates at all.

The only subtlety worth explaining is what "half the stroke" means, because there are two
plausible answers and one of them is wrong.

A stroke is stored as the path the pointer took: a list of positions, sampled as the drag
happened. Those samples are **not** evenly spaced. Move the mouse slowly and a hundred of them
pile into one inch; whip it across and a single sample spans a foot. So "half the samples" is
not half the drawing at all — a stroke trimmed that way would crawl through the part you drew
carefully and leap through the part you drew fast, and the write-on would appear to speed up and
slow down for no visible reason.

The right answer is half the **distance**. Lumit walks the path adding up the length of each
straight piece, works out how far along the total the cut should fall, finds the piece the cut
lands in, and cuts that one piece at exactly the right fraction of the way across it. The result
travels at a steady pace regardless of how the stroke was drawn — and because the same walk
handles both ends, the mark can be trimmed from the front, from the back, or from both at once.

Two small honesties. An End that has not passed Start yet draws nothing, which is exactly what
the first frame of a write-on should look like rather than an error. And a stroke that is a
single dab has no length to divide, so it appears whole the moment any of it is asked for. Both
numbers are left out of the saved file until somebody moves them, so nothing changes for a
project that has never used them.

### A stroke that blends (K-550)

Every layer has a **blend mode** — the list with Multiply, Screen, Overlay and the rest in it —
which decides how its picture combines with everything underneath. A paint stroke now has one
too, on its own row in the Timeline, chosen from the same list.

What that means in practice: a Multiply stroke darkens what it is drawn over instead of covering
it, so it reads as ink soaking into the picture rather than paint sitting on top. Screen does the
opposite and only lightens, which is how a highlight is brushed on without flattening what was
there. Overlay keeps the darks dark and the lights light and pushes the colour through the
middle. They are the same words doing the same thing they do on a layer, which is the point:
nobody should have to learn a second vocabulary because the mark happens to be a brush stroke.

The part that mattered when building it is that a blend changes **what colour the mark is**, and
never **how much of the pixel it covers**. Those are two separate things and it is easy to
confuse them. The brush works out coverage first — how solidly this pixel is inside the mark,
given the brush's size, its hardness and the stroke's opacity — and that number is untouched by
the mode. Then the mode decides what colour to put there, by combining the brush's colour with
the colour already on the layer. Finally that colour is laid down at the coverage worked out in
step one, by exactly the same source-over the brush has always used. So a half-opacity Multiply
stroke is genuinely half of the way to the multiplied result, and a soft edge is still a soft
edge.

None of the arithmetic is new. Lumit already had every one of these formulas written once, for
the blend row on an effect, which itself matches the compositor's — so the stroke calls that
same code rather than growing a third copy that could drift. And the eraser ignores the setting
entirely, because an eraser has no colour to combine: it takes transparency away, and a mode
there would only be a second way of saying nothing.

## 13. The two public web sites

`web/` and `web-docs/` are the public face of the project: **lumitlab.com**,
the marketing and download page, and **docs.lumitlab.com**, the user
documentation (K-279). Both are built with **Astro**, a website builder that
takes pages written mostly as ordinary text and turns them into plain HTML
files at build time — there is no application running on a server afterwards,
just files handed to whoever asks, which is why each site builds in about a
second and costs nothing to host.

They sit outside the Cargo workspace and nothing depends on them: no engine
crate, no Flutter code and no test reaches into them, so a change to either
site can never break a build of Lumit itself, nor the other way round. The
practical details — running one locally, how deployment to Cloudflare works,
and the traps in it — are in `web/README.md`.

**Help ▸ Lumit help and Help ▸ Lumit online guides open the docs site**, and
that is the one thread between the application and either site.
`flutter_ui/lib/state/external_links.dart` holds the two addresses and one
launcher: Lumit has no browser of its own and hands the address to the desktop
— `rundll32 url.dll,FileProtocolHandler` on Windows, `open` on macOS,
`xdg-open` elsewhere — exactly as the updater already does when it reveals a
downloaded file. Nothing goes through a command shell (the URL is one argument
of a program, never part of a line to be parsed), the scheme is checked so only
`http` and `https` are ever handed over, and a machine that refuses gets a line
in the status strip rather than a menu row that silently does nothing.

Both rows point at pages, not sections. The docs site's sidebar headings are
generated from folders and have no page of their own, so a link to `/use/`
would be a 404 — "online guides" therefore goes to the first-composition
walkthrough, which is where somebody asking for guides wanted to end up.

### The effect pages write themselves

The manual has a page per effect, and each one carries a table of every
parameter with its range, its default and its unit. Nobody types those tables.
They are already written down once, on the effect's own declaration in the
engine — that is where a slider learns it runs 0 to 25 and starts at 1.5 — and
copying them into a web page by hand would mean every future tweak to a slider
silently made the manual wrong.

So the numbers travel. `cargo test -p lumit-core regenerate_fx_reference --
--ignored` asks the engine to write its whole catalogue out as one file,
`crates/lumit-core/fx-reference.json`; then, from `web-docs/`, `npm run
docs:effects` turns that file into the pages. (The odd `--ignored` is Rust's way
of saying "this test only runs when asked for by name" — it writes a file, so it
should not fire on every run.)

The trick that keeps it liveable is that the script does **not** own the pages.
Each page has a marked-off block, opened and closed by an invisible comment, and
the script rewrites what is between those two marks and nothing else. The
prose — what the effect is for, what each control actually does, the warnings —
is hand-written above and below, and survives every regeneration untouched. An
effect with no page yet gets a whole one scaffolded, headings and marks and all,
waiting for somebody to write the prose into it.

Two safety rails come with that. `npm run docs:effects -- --check` writes
nothing and simply fails if any page has fallen behind the engine, which is how
a build catches a slider that moved without the manual following. And a page
nobody claims any more — the leftover of a renamed or deleted effect — is
reported by name rather than quietly left to mislead readers.

This is also why one sentence can appear on all eighty-five pages at once. The
Matte row's plain-English meaning (above, under driving effects with mattes) is
written in the script for the effects that treat it as a simple strength, and on
the effect itself for the four that do something cleverer — so each page states
the truth about *that* effect without anybody maintaining eighty-five paragraphs
that say the same thing.

### The example pictures, and why nobody draws them

Every effect page carries a picture of that effect on one frame of footage. There
are eighty-five of those, and a folder of eighty-five screenshots taken by hand
would be wrong within a month: somebody changes a default, and the manual quietly
keeps showing what the effect used to do.

So the engine draws them. `npm run docs:effect-shots`, run from `web-docs/`, does
three things in a row. First it works out what the source footage is (below).
Then it runs a test in the Rust engine
(`crates/lumit-render/tests/effect_examples.rs`) which builds a tiny project in
memory, one composition with that clip on a single layer, puts one effect on the
layer, renders a frame, and does it again for the next effect. The walk it uses
is the Viewer's own walk, so a figure on the website is a frame the application
would genuinely produce. Last, **sharp** encodes the results as WebP into
`web-docs/src/assets/effects/`, where the pages look for them. They sit under
`src/assets/` rather than `public/` so that Astro's own image pipeline handles
them, which is what `Compare.astro` — the little component that lays out the
before-and-after wipe on every effect page — asks it for.

The source comes in two grades, and the difference reaches about four pictures.
The good grade is a real recording: two short clips of the actual footage and its
depth pass, plus a real `.cube` grade for the LUT page. Those three files sit in
`web-docs/src/assets/` and are **gitignored**, partly because megabytes of video
are not what a repository is for, and partly because the LUT is somebody else's
work to give away. The grade that ships is a pair of stills, which the script pans
sideways so that at least something moves. Every picture except the temporal
handful is identical either way; Fast motion blur, Echo and Datamosh all smear
motion the footage already contains, and a panned still gives them far less to
work with than a running firefight does.

Three details are worth knowing, because all three are deliberate.

The engine hands the pictures over as *raw* pixels, one uncompressed file each.
Nothing in the Rust workspace can write a PNG or a WebP, and writing an image
encoder so that a documentation script has something to read would be a real
piece of code carried for ever to save a script five lines. The node side already
has an encoder, so the node side encodes.

And the settings are not the defaults. A fresh Curves is a straight line and a
fresh Exposure is zero stops, which is exactly right in the application and
useless in a manual: the picture would be the untouched frame with a caption
claiming otherwise. So a table in the test names showcase settings for the
effects whose defaults change nothing, and the run compares every render against
the untouched frame and **fails** if any of them came back identical. A silently
neutral example is worse than a missing one, and this is the check that makes it
impossible.

A few effects get special handling for honest reasons. Drop shadow and light wrap
need an outline to work from, so their frame is masked to an oval first. Depth of
field, displacement map, set matte and texturize each read a second picture, so
the project gains the plate's own depth pass to point at, which is why the depth of
field picture blurs the scene rather than a ramp somebody invented.

One effect gets a fourth kind of help. Fast motion blur smears movement that is
already inside the footage, so its example plays the plate at triple speed through
the frame everything else is rendered at: two seconds of source crossing the
middle two thirds of a second of the composition. The retime is written as two
plain keyframes rather than a speed ramp, because a ramp has to be integrated to
know where it lands, and landing on a different frame from the rest of the manual
is the one thing that example must not do.

Two effects are skipped outright and say so. Posterize time holds one frame for
several, which only exists in motion, and a still of it would be a still of the
plate wearing a misleading caption. Matte key wants a screen to pull and the
example frame has none, so every setting of it either did nothing or keyed
something arbitrary. Neither gets a figure: the page generator leaves the picture
out whenever there is no file on disk, so nothing links to an image that is not
there.

Finally, the figures are not plain pictures. Each one lays the effect over the
untouched frame and clips it down the middle, with a handle the reader drags left
and right to wipe between the two. It is built from a range input stretched
invisibly across the whole picture, which is what makes it work with a finger, a
mouse and the arrow keys without any of that being written by hand; a few lines of
script in `astro.config.mjs` point a CSS clip at the input's value. With the script
switched off the figure is still an honest half-and-half split, because the
starting position is written into the markup rather than applied afterwards.

## 14. The redesign, in plain terms

In August 2026 Lumit's look got a proper overhaul — not new features, but a stricter set
of rules about how the existing ones are drawn. If you have used a well-finished editor
and wondered why it feels calmer than a busy one showing the same information, the answer
is usually discipline: fewer shades, fewer colours, fewer sizes, each with exactly one
meaning. That is what changed here.

**Three greys.** A panel that nobody is touching now uses at most three background shades:
the dark canvas behind everything, the panel's own body, and a slightly lighter strip for
its header. The two brighter shades still exist, but they are reserved for things that are
happening — a row under the mouse, a menu floating open. Boxes you can type a value into
go the other way: they are slightly *darker* than the panel, like a recess the number sits
in, which is what quietly tells you "this is editable" without any colour at all.

**Two colours, two meanings.** The pink-red accent used to mean five different things.
Now it means three: the one filled button on a surface, the playhead, and the tick under
the active tab. A second colour — a muted amber called "animated" — means exactly one
thing: this is keyframed, or this is the handle you have selected. Keyframe diamonds, the
lit stopwatch, the work-area band, the value field you are about to change — those are
amber, and nothing else is allowed to be. If a third use of amber ever appears, it is a
bug by definition. A value you can edit is plain text in its recess at rest, turns amber
once it has keyframes, and turns accent only while you are actually dragging it.

**Small capital labels.** Everything the *application* names — panel titles, section
headers in the effect controls, column headers, tab labels, dialog titles — is now set in
small, spaced-out capitals in the numbers font. Everything *you* name — layers, files,
values — stays in ordinary sentence case. Once you notice it, you can tell at a glance
which words are furniture and which words are your project. The fonts themselves changed
too: Hanken Grotesk for ordinary text and Geist Mono for every number, both free to
bundle, replacing the previous mix of four typefaces.

**Words or icons.** A new setting lets chrome speak in words (the default), in icons
(buttons and tabs become glyphs, panel titles stay text), or entirely in icons. Lumit is
getting its own icon set for this — one glyph per word, all drawn to the same grid and
stroke so they look like one family. Whatever the setting, hovering always shows the word,
in one or two words, never a paragraph.

**How it lands.** The overhaul runs in phases, in this order: first the theme groundwork —
the new colour tokens, the bundled fonts, the icon set; then the panels and windows one by
one (effect controls, timeline, project, viewer, settings, the welcome screen, and the
export dialog with its queue); then the node graph and its workspace; and finally the
website, restyled to match. Giving each dialog a window of its own — a floating export
dialog the taskbar knows about, a second viewer on a second monitor — comes later, as a
phase of its own, because the feature is not finished in Flutter, the toolkit Lumit's
interface is built on: it exists only in Flutter's development builds, behind a switch,
with the warning that it will change. The consolation is that waiting costs nothing.
Dialogs built the ordinary way today, as overlays inside the one window, are exactly the
ones that turn into real windows when the toolkit is ready — the same code, re-parented,
rather than rewritten.

**How much room a row gets.** Every row in Lumit has a fixed height, and those heights are
read off the approved mockups rather than chosen in code. There turned out to be two
reasonable answers to "how tall is a layer row", and the difference between them is a
pixel or two: the mockup draws 22 pixels of row with a hairline seam under it, so what the
eye actually receives is 23. Lumit now takes the roomier reading as its default and keeps
the tighter one behind a setting called **Compact**, in Settings → Interface → Display.
Turn it on and rows lose that pixel or two back, so four or five more layers fit on a
short screen. Nothing else moves: no colour changes, no word changes size, no control
changes what it does.

The way this is built is worth a sentence, because it is deliberately dull. The two sets of
numbers live side by side in one small object (`DensityTokens`, in
`flutter_ui/lib/theme/theme.dart`) — one instance holding the roomy values, one holding the
tight ones — and that object rides on the theme, the same parcel of colours and shapes every
part of the interface already has in hand when it draws. Switching the setting rebuilds the
theme, and everything repaints from the new numbers on the next frame. There is no
per-panel switch and no third setting, on purpose: a panel that could disagree with its
neighbour about row height is a panel whose halves stop lining up, and the Timeline in
particular is two lists side by side that have to agree row for row or the whole thing
shears. Only the handful of rows that genuinely differ are in that object at all — a
header strip, a clip bar and a value box measure the same either way, so they stay plain
fixed numbers where they are written. A dial with two identical settings is a dial that
does nothing.

One row of the Timeline broke out of that scheme after the editor was used on a real
desktop for a day: the strip carrying the clock, the layer search and the Layers / Keys /
Graph tabs is aimed at constantly, and at the mockup's height it was genuinely hard to hit
— a target a few pixels tall, clicked hundreds of times an hour. So the roomy setting now
gives that row **24** pixels and the column-header row under it **23**, where every other
thin row in the application still gets 19. The controls standing in those rows are told to
stand 20 tall rather than each measuring itself to its own word, so the tabs, the search
box and the two clocks all fill the row instead of floating in a band of empty ground.
Compact was left at exactly the numbers it had: it is the setting for someone who has
already said they want less air, and a ruling about the roomy default that quietly moved
the tight one would be a ruling about both.

That change has a knock-on the Timeline could not avoid, and it is the good kind. The
time ruler on the right-hand side of the panel is not given a height of its own — it is
told to be exactly as tall as the two rows facing it on the left, which is the whole
reason the panel's two halves line up row for row. Growing those two rows therefore grew
the ruler with them, from 38 to 47, and the extra nine pixels went to the clock. In the
same pass the line that used to be drawn across the ruler's middle came out: the ruler is
one band now, the labelled ticks reach down through where that line was, and the
work-area highlight and the two handles that drag its ends run the ruler's full height
instead of the bottom half. An edge you can see for the whole of the ruler and can only
take hold of for half of it is a handle that is half a lie.

**The mockups are the reference.** Each panel was designed as a full-size picture first
and approved by the owner before any code moves. When a question comes up about where
something sits or how big it is, the answer is read off the approved mockup, not
re-argued in code review. The written rules live in `docs/15-DESIGN.md`; the decisions
and their reasons are logged as K-438 to K-449 in `docs/02-DECISIONS.md`; what Flutter's
windowing support actually offers today, and what to test before believing it, is
`docs/impl/multi-window.md`.

## 15. Lumit's own icons, in plain terms

Until now the little pictures in the interface came from somebody else's set, fetched from
a package the way you might drop a stock logo into a title sequence. The redesign draws
its own instead, so that every glyph shares one grid, one line weight and one temperament
— the difference between a set and a collection.

A glyph is not a picture file. It is a short line of drawing instructions: move here, draw
a line to there, put a circle of this radius at that point, on a sixteen-by-sixteen grid.
All 116 of them live together in one data file, grouped by where they are used — tools,
layer switches, transport, the Viewer bar, and so on — with one line each. That file is
the set. If a glyph looks wrong, that one line is where it is wrong, and reading the file
top to bottom is a fair way to see the whole set at once.

Code cannot read that data file while the application is running without paying for it, so
a small tool converts it: it reads every glyph, wraps it in the same frame — same grid,
same 1.5-unit stroke, same round ends — and writes out a Dart file with one named entry
per glyph. Changing a glyph is therefore a one-line edit and one command to re-run the
tool. The written-out file is generated, never hand-edited; its header says so, and a test
compares it back against the data file, so a forgotten re-run fails the build rather than
quietly shipping the old drawing.

The naming matters more than it sounds. Because each glyph becomes a named entry rather
than a string of text, asking for a glyph that does not exist is caught when the code is
compiled, not when a button turns up blank in front of you.

Nothing in a glyph says what colour it is. Each one is drawn in "whatever colour the text
around me is", so a glyph in a dim row is dim, the same glyph under the mouse brightens
with the row, and an active one goes accent — with no second copy of the drawing anywhere.
The single exception is the Viewer's channels indicator, whose three overlapping circles
show which of red, green and blue you are looking at. Those colours are not a design
choice, they are the current state of the Viewer, so the stored glyph is plain like all
the others and the Viewer paints it when it draws it.

At this stage the set exists and is tested but is not yet wired into any panel; the old
icons stay on screen until the panels phase replaces them one panel at a time.

## 16. What a control's number means, in plain terms

Every control in Effect controls shows a number, and a number on its own is ambiguous.
Is a Radius of 30 thirty pixels, thirty per cent, thirty of something else? Lumit's
answer is that each control declares its **unit** once, in the effect's own declaration
in the engine, and everything else reads it from there.

There are six answers a control can give. **Pixels at composition size** — the unit of
every distance, radius and offset: a number authored against the composition's own size,
which the engine scales down for a half-resolution preview and up for a larger export, so
what you set once looks the same everywhere. **Per cent** — a share of something the
control names, where 100 is all of it: the Mix at the bottom of every effect, one
channel's share of a grain, a position given as a fraction of the frame. **Degrees** for
angles, **seconds** and **frames** for durations. And **none**, for a number that
genuinely has no unit — a gamma, a count of blades, a threshold in light values. "None"
is a deliberate answer, not a blank: a test fails the build if a control never says which
of the six it is, because a control that forgot would show a bare number and nobody
would spot it.

Declaring it once buys two things at opposite ends of the program. The engine knows which
numbers to rescale when a frame is rendered smaller than the composition — so no effect
can be forgotten when preview resolution changes, which used to be a list somebody had to
remember to add to. And the panel knows what to write in small type after the value, so
the same fact is not typed out a second time in the interface, where it would drift. It
had drifted: the interface was carrying its own short list of which controls write pixels
and which write percentages, keyed by the control's name — and because two different
effects can both call a control "Centre X" while meaning different things, that list could
only ever be right about one of them.

**Points, and the chain between them.** A point — a Centre, a Light, a Focus point — is
not one control in Lumit but two ordinary number controls sitting next to each other,
named `..._x` and `..._y`. The panel notices the pair and draws them as one row: two equal
boxes with a small chain between. Closed, the chain ties them together, so dragging one
moves the other in proportion; open, each is on its own. Which way you left the chain is
remembered with the project, per effect, exactly as a renamed effect is — and because
nothing before this had a chain, every project made until now opens with all of them open,
which is precisely how those projects behaved. Closing or opening one is a single undo
step, like any other change to an effect.

The engine owns the pairing and the remembering; the proportional dragging itself is the
panel's, because it only exists while your finger is on the mouse.

### How fast a dragged number moves: the modifier ladder

Every number in Lumit can be changed by dragging sideways across it, and one speed is
never enough: the same box that wants to travel a hundred units in a swipe also wants to
be nudged by a hundredth. So a drag has four speeds, chosen by what you are holding down
— hold **Shift** and each pixel is worth ten, hold nothing and it is worth one, hold
**Ctrl** and it is a tenth, hold **Alt** and it is a hundredth. You can press and release
these in the middle of a drag; the next pixel simply counts differently, and the
travel so far is kept.

The part worth explaining is what appears while you drag. A small chip floats above the
field showing all four speeds at once, with the one you are currently on drawn in a box.
It is there so the ladder is learned by using it rather than by reading about it — you
find the fine speeds because you saw them named while your hand was already on the number.
It is put up when the drag starts and taken down when you let go, and it changes nothing
about the panel underneath: like every hint in Lumit, it exists only while a gesture is
running, appears next to what the gesture is doing, and leaves no trace behind. That is
also why it floats *above* everything rather than sitting inside the row — a hint that
made room for itself would push the panel about every time you touched a number.

## 17. The Timeline's two views, in plain terms

The Timeline shows the same composition two ways, and a pair of words at the top right of
the panel — **Layers** and **Graph** — says which one is up.

**Layers** is the one you already know: one bar per layer, laid along time, and twirl a
layer open to see its properties with their keyframes on lanes beside them. It is where
you cut, trim, reorder and key.

**Graph** replaces the bars with the curves themselves — the same keyframes, drawn as the
shape of the movement between them, so you can see a value speed up and slow down instead
of only seeing when it changed. Its left-hand list is **exactly the Layers list**: the same
twirls, the same columns, the same rows. Whichever properties you have picked there are the
curves on the pane, and switching between the two views changes what is drawn against time
and nothing else about where things are.

There used to be a third, **Keys** — a dope sheet that listed every animated property of
every layer on one flat level. It was withdrawn after the owner worked in it: pressing `U`
or twirling a layer open was the same number of clicks, and the Layers view is already
dense enough to read a composition's timing off. The one part of it that was kept is its
strip of commands, which now lives along the bottom of the Layers view (below).

A key's *shape* is its interpolation, here and on a Layers lane alike. A diamond runs in a
straight line, an hourglass is a curve, a square is a hold — the value stays put until the
next key and then jumps. The mark is cut down the middle, and each half answers for its own
side, so a key that eases into itself and holds out of itself is half hourglass and half
square. Reading a sheet of them tells you what a composition's motion feels like before you
play a frame of it.

"Cut down the middle" is how the mark is *described*, not how it is drawn. Drawing it as
two halves is what a screen punishes: shapes are drawn with soft edges so their slopes do
not come out as staircases, and where two separately softened edges met on the centre line
neither quite covered it, so the lane's dark ground showed through as a hairline seam down
every key — even the plain diamonds, which have no seam to show. So the two halves are
joined into one outline *before* anything is painted: one shape, one soft edge round the
outside of it, nothing down the middle. An hourglass is the one shape allowed to keep two
pieces, because its two triangles meet at a single point and a point has no line to seam.

**Graph** is the curves: the same keyframes again, drawn as the value they carry rather
than as marks on a line, which is where you shape an ease by hand.

The three share everything above and below the middle — the time ruler, the work-area
band, the cache stripe, the markers, the playhead and the zoom slider are one set of
things the modes swap the *body* underneath, not three copies. That is why switching modes
never moves the ruler and never loses your place: the same range is on screen, described
differently. In the code this is one field saying which mode is up
(`TimelineMode` in `flutter_ui/lib/panels/timeline_panel_frb.dart`) rather than a switch
per mode, because two switches can say "both at once" and a state nobody can draw is a
state something will eventually reach.

### The graph's list is the Layers list

Graph mode used to have a left-hand list of its own — every animated property, flat, each
row with a tick box that put its curve on the pane. It has gone, and the graph now shows
**exactly** what Layers shows: the same twirls, the same columns, the same rows.

The reason is that there were two ways of choosing a curve and one of them was only
available in one view. Now there is one: **whichever property rows you have picked are the
curves on the pane**, in either view. Pick Opacity in Layers, switch to Graph, and its
curve is there. A row's small coloured tick on its lane says which curve is which, and a
property with two axes says both.

What went with the list was **Normalise**, a checkbox that drew each curve against its own
smallest and largest value so that a rotation in degrees and a position in pixels both
filled the height. It was withdrawn because it was hard to know what you were looking at
while it was on — the numbers down the side stopped being values and became percentages of
something different per curve. Without it the curves share one scale again, which means a
small-numbered curve beside a large-numbered one really is a flat line along the bottom.
That is the trade, taken deliberately: a picture that is harder to compare is better than
one that is easier to misread.

While exactly one keyframe is selected, a small row appears at the foot of the list reading
which frame it sits on, what it holds, and its two **influences** as editable percentages —
how far each side's ease reaches toward its neighbour. Typing into those is the same edit as
dragging the key's handle on the pane, and lands as one undo step. Two or more keys are a
block, and a block has its own badge, so the row steps aside.

Those numbers live in a narrow strip pinned to the **right** edge of what you can see —
never over the curves, and never scrolling away. The pane itself is as wide as the whole
composition and slides sideways inside a window onto it, so the strip is measured from the
window rather than from the pane: whatever the zoom and wherever you have scrolled to, the
scale is in the same place.

Finally, while exactly one keyframe is selected, a small row appears at the foot of the
list reading which frame it sits on, what it holds, and its two **influences** as editable
percentages — how far each side's ease reaches toward its neighbour. Typing into those is
the same edit as dragging the key's handle on the pane, and lands as one undo step. Two or
more keys are a block, and a block has its own badge, so the row steps aside.

### Selecting a run of keyframes as one thing

Drag a box across some keyframes and they stop being several marks and become a **block**.
A thin outline appears round everything you caught, with a small mark at each end and a
label beside it reading something like `4 keys · 36 f` — how many you have hold of, and
how many frames from the first to the last.

The marks at the ends are handles. Take hold of one and drag, and the whole run stretches:
the end you did **not** touch stays exactly where it is, the end you are dragging goes
where you put it, and every key in between keeps its share of the distance. A key that sat
a third of the way along still sits a third of the way along. That is how you take an
animation that runs over one second and make it run over two without re-timing every key
by hand. With the magnet on, every key lands on a whole frame, exactly as dragging a
single one does.

The label is also a button: press it and the **Ease popover** opens on the selection.

The box is drawn by the part of the panel that draws the lanes, so a block behaves the
same way wherever its keys were picked — one piece of code, not two that have to be kept
in step. (This is why nothing about blocks had to be unpicked when the dope sheet was
withdrawn: none of it was ever the sheet's.) The arithmetic behind
it (how far each key moves, what the label counts, what Reverse and Stagger do to a time)
lives on its own in `flutter_ui/lib/panels/key_block.dart`, with no picture and no engine
in it, so it can be checked directly rather than measured off a screen.

### The same block, in the graph: scaling by its edges

Graph mode draws the same box round two or more selected keyframes, and there it has a
second dimension to it. On a lane a block only has a length — the frames it covers. On the
graph a block also has a *height*, because up and down is the value the property holds. So
the box there is a real rectangle, and each of its four edges is something you can take
hold of.

Drag the **left or right** edge and the selection scales in time, exactly as the lane
handles do: the edge you did not touch stays put, the one in your hand goes where
you put it, and every key keeps its share of the distance. Drag the **top or bottom** edge
and the same thing happens to the values: pull the top down and the whole animation gets
tamer, push it up and it gets bigger, and whichever edge you left alone holds its keys
exactly where they were. It is the ordinary way to say "same movement, half as much" or
"same shape, twice as long" without touching a single key by hand.

Hold `Shift` while you drag an edge and the answer lands on whole numbers — whole frames
on the time axis, whole values on the other — and a small label under the box says live
what it now reaches, so you can drive it to a round number by eye. There is no grab at the
*corners*, and that is deliberate rather than missing: a box drawn round a selection has
its corners sitting on the selection's own outermost keyframes, and a corner target there
would take the clicks and drags those keys need for themselves. Scaling both directions is
two drags instead of one, which is a small price for keeping every keyframe reachable.

The sums are the lanes' sums. `scaledAbout` in
`flutter_ui/lib/panels/key_block.dart` — "here is the end that stays, here is how far the
other end used to reach, here is how far it reaches now, where does this one go?" — is the
whole of it, and both the lane handles and the graph's edges ask it. Time is measured in
frames; value is measured in **pixels on screen**, so that a scale meaning "half" means
half of what you can see whatever units the curve is in.

### Telling a Position's two numbers apart

A Position is two numbers, x and y, and they sit on one row sharing one stopwatch. Most of
the time that is what you want: you key "where the layer is", and both numbers get a key.

Sometimes it is not. A ball that bounces is falling and settling on the way down and
sliding evenly along sideways — two different shapes, on the same property. So
right-clicking the name **Position** (or **Anchor point**, or **Scale**) offers **Separate
axes**, and the one row becomes two: *Position x* and *Position y*, each with its own
stopwatch, its own diamonds on its own lane, and its own curve in the graph editor. Key one
without keying the other; ease one and leave the other straight.

Nothing in the saved file changes shape when you do this. Lumit has always kept the two
numbers as two separate properties under the bonnet — that is what makes a curve for x
alone possible in the first place — so all that is stored is your choice about how they are
shown. A project made before this existed opens with everything as it was, and a project
where you never touched the menu saves exactly the same bytes as before.

**Combine axes** puts them back on one row. Here Lumit has a small tidying job to do: back
on one row the two axes share a stopwatch, so a diamond on that row has to mean something
for both of them. If x has keys at frames 0 and 96 and y has one at frame 24, each gains
the other's times — x picks up a key at 24, y picks up keys at 0 and 96. The new keys are
not guesses. Each takes the value its curve already had at that moment, and the curve
either side of it is re-described so that it still passes through everything it passed
through before. The picture does not move. (A number that was never animated is left alone:
a constant does not need keys to stay constant.)

Scale has one extra state, and it starts there. A scale is normally meant to stay
proportional, so its row shows **one** box with the two axes **linked**: type 50 and both
halves become 50% of what they were, keeping the shape. **Unlink axes** gives you a box
each — a squash or a stretch — and **Separate axes** goes the whole way to a row each. All
of it is one undo step, whichever direction you go, including the keyframe tidying.

### Typing a keyframe's numbers

Double-click a keyframe in the graph and a small box opens holding four numbers: which
**frame** it sits on, the **value** it holds, and the reach of its two eases as **In** and
**Out** percentages. Type into any of them and the key moves; the box stays up, because
whoever is typing numbers usually has more than one to type.

Two details are worth knowing. The **frame** field will not take you past the keyframes on
either side of this one — it stops one frame short — because reordering the keys while the
box is open would leave it pointing at whichever key had taken this one's place. And the
double-click is spotted by looking at the *clock* rather than by asking Flutter for a
double-tap gesture: asking for one would make Flutter hold every ordinary single click on
a keyframe back until it was sure a second was not coming, and a visible delay on the
commonest gesture on the pane is a much worse thing than a slightly hand-rolled
double-click.

### Tangents that aim themselves — Auto, Clamp and Free

Every keyframe has two sides, and each side has a tangent: the little handle that says how
fast the movement is going as it arrives at the key, or as it leaves. Normally you put
that handle where you want it and it stays there. The three buttons on the graph's bottom
strip — **Auto**, **Clamp**, **Free** — say who is holding it.

**Free** is the familiar one: the handle is where you last dragged it, and it stays there
whatever happens to the keys around it.

**Auto** hands the handle to the neighbours. Instead of storing a direction, the side works
one out every time the curve is read, by aiming from the key *before* this one straight at
the key *after* it. That is the direction the movement is already travelling in, so the
curve runs through the key without a kink — and, because it is worked out fresh each time,
moving a neighbouring key re-aims it at once. Nothing has to be recalculated and saved: it
was never stored in the first place. The first and last keys of a curve have no pair to aim
between, so their automatic tangents lie flat, which reads as easing in and out of the ends.

**Clamp** is Auto with the overshoot taken out. A smooth aim can push the curve past the
value of a neighbouring key on its way — animate 0, 10, 9 and the curve will bulge above 10
before coming down — which is fine for a bouncing ball and wrong for a value that must not
exceed what you keyed. Clamp lies the tangent flat wherever the key is a peak or a trough
(any tilt would swing past one neighbour or the other) and otherwise limits how steep it
may be. The limit is a known one from the numerical-analysis literature, not a guess:
three times the gentler of the two slopes into and out of the key is exactly the point past
which a cubic can leave the box its keys make.

The mode belongs to a **side**, not to a key, and one thing about the switching is worth
knowing. If you shape a handle by hand and then press Auto, the ease you shaped is not
thrown away — it is filed inside the automatic side, unused, and pressing Free hands it
straight back. So Free → Auto → Free is a round trip, not a reset. The other direction is
simpler: dragging a handle at all takes that side back to Free, because you have just
overruled the neighbours, and there is no sense in the curve arguing with your hand.

### The magnet, and what a drag can land on

Every dragged thing on the Timeline wants to land on the things already there: the start
of a layer, a cut inside one, a keyframe, a marker, the playhead, the ends of the work
area. That wanting is the **magnet** — the button on the lane bottom bar — and it now
governs every drag on the panel rather than only a keyframe's. Drag a layer's bar and
either of its ends can be the one that lands; drag a work-area edge, a marker flag, or a
keyframe in the graph, and each reaches for the same list. While something has hold of a
drag, a thin line is drawn where it will land, so a drag that jumps looks like a service
rather than a fault. Holding `Ctrl` turns the magnet off for as long as you hold it, which
is the way out on the one occasion in ten when the place you want is exactly the place a
magnet will not let you put it.

Two rules keep it honest. **How near is measured in pixels on screen, never in time**, so
zooming in *is* the precision control: at a wide zoom a marker fifty frames away is worth
reaching for, and at a tight one a frame away is not. And **whatever you are dragging is
left out of its own list** — a work-area edge that could snap to itself, or a bar to its
own start, would simply refuse to move.

Two smaller things live on the ruler. Double-click the **work-area band** and the work
area goes back to being the whole composition; double-click the ruler's own ground and a
**marker** is made there, with its label editor open and waiting. And while the transport
is running the lanes **flip a page** whenever the playhead runs off the right-hand edge,
rather than scrolling continuously under it: what you are watching stays still, and the
view moves only at the moment it has to. Taking hold of the playhead stops playback, so
this can never happen under your hand.

### The Ease popover

A small box with four lines. **Curve** picks the shape by name — Easy ease, Slow start,
Overshoot and the rest, the same shapes the Easing panel draws. **Influence** is two
percentages: how far the ease reaches out of the first key and back into the last one, the
same "influence" a keyframe has always stored. **Stagger** is a number of frames and a
direction: set it to 3 and each row in the block starts three frames after the row above,
so a run of properties arrives one after another rather than all together — the cheapest
way to make a stack of things feel hand-animated. **Open graph** closes the box and shows
the same keys as curves, for when picking a shape by name is not enough; **Apply** puts it
on.

Nothing here works out a curve for itself. The shapes come from the same file the Easing
panel uses, and Apply goes through the same call, so an ease chosen here and an ease drawn
there land identically.

### The keyframe strip

Along the bottom of the Layers view: **Interpolation** — Linear, Hold, Ease, Bezier —
which sets both sides of every selected key at a press (Ease opens the popover above);
then **Reverse**, **Copy** and **Paste at playhead**.

Reverse turns the block back to front *where it stands*. The earliest key's time becomes
the latest's, and each value travels with its own key, so the movement plays backwards
without moving along the Timeline. Each key's two eases swap over as well, because the
side that was leaving is now the side arriving — without that the times would be reversed
while the shape of the motion still pointed the old way.

Paste at playhead puts the copied keys down with the first of them under the playhead, on
the properties they came off.

### Words or pictures on the buttons

Every button, tab and toggle in Lumit has a word and a small drawing that mean the same
thing. **Settings ▸ Appearance ▸ Interface ▸ Chrome labels** chooses which of the two you
see: *Words*, *Icons* (the default — buttons, tabs and toggles become drawings, while
panel titles stay as words), or *Icons everywhere* (the panel titles too).

Two things are true whichever you choose. Hovering anything always spells out the word, so
nothing is ever a picture you cannot name. And **anything you typed yourself** — a layer's
name, a composition's — is never turned into a drawing; the setting is about Lumit's own
chrome, not about your document.

The first place it shows is the Timeline's bottom bar, where the three column toggles
(Switches, Modes, Parent) draw as glyphs by default. The rest of the chrome still speaks
words and will be converted a surface at a time.

### One gesture, one undo

An edit in Lumit replaces a whole property's animation rather than nudging one keyframe,
which keeps undo simple: to undo, put the old animation back. The catch is that one
*gesture* can touch several properties on several layers — stretching a block that covers
three rows, pasting a clipboard that came off two — and each of those is a separate edit.
Ctrl-Z would then take three presses to put back one drag, and how many depended on what
happened to be selected.

So the engine can be told "everything between here and here is one step"
(`begin_undo_group` / `end_undo_group` in `crates/lumit-core/src/store.rs`). The edits
themselves still happen the moment they are made — nothing waits, so anything reading the
document part-way through a gesture sees it as it really is — but the history collects
them and files them as a single entry when the gesture ends. One drag, one Ctrl-Z, always.

Two details that matter if you ever read the code. A group that collected only one edit is
filed as that edit rather than as a bundle of one, because a bundle of one undoes the same
and reads worse. And the two calls have to be paired, so the Flutter side always closes
one in a `finally` (`asOneUndoStep` in `flutter_ui/lib/panels/layer_fold_frb.dart`): a
group left open would quietly stop recording history at all.

### Letting go of a drag half way: Escape

Everything in the Timeline that you drag — a layer's bar, a keyframe, the handle at the
end of a block, and in the graph a curve's tangent handle or an edge of the transform box
— works the same way underneath. Nothing is written to the document while
the button is down. The panel remembers how far the pointer has travelled, draws the thing
where that puts it, and makes exactly one edit when you let go. That is what makes a drag
one undo step, and it is also what gives a drag a way *out*: if nothing has been written
yet, then abandoning the gesture costs nothing at all — there is no edit to take back,
because there was never an edit.

So pressing `Escape` while the button is still down puts everything back where the drag
found it and writes nothing. The pointer carries on moving, and nothing follows it any
more; letting go afterwards does nothing either. This is the behaviour the study found in
Caddis and the reason it feels safe to try a drag there: a gesture you have started is
never a gesture you are committed to.

The mechanism is one small object, `DragEscape` (`flutter_ui/lib/widgets/drag_escape.dart`),
which each draggable thing keeps one of. It is told at the start of a gesture how to put
things back, it claims `Escape` only while the gesture runs, and at the release it
answers one question: is there anything to commit? A drag that was escaped answers no.
Sharing it is the point — a way out that only some drags had would be worse than none,
because the one time it is wanted is the time you cannot remember which drags have it.

### One Escape, one step back: the ladder

`Escape` is the busiest key in the application. At any moment several things could
plausibly answer it: a drag is running, a menu is open over the panel it was raised from,
a dialogue is up behind that, and a selection is sitting under all of it. Only *one* of
them should move when you press the key — the innermost one, the thing you are doing right
now — and the rest should be exactly as you left them.

Getting that wrong is easy, and Lumit did for a while. The way a Flutter application
listens for keys outside a focused text box is to add a handler to the toolkit's keyboard
object, and each surface added its own and returned "mine" when it took a press. That reads
like a queue and is not one: the toolkit calls **every** handler on **every** key press,
whatever the ones before it said, and then runs the focus machinery afterwards as well. So
one press of `Escape` mid-drag put the drag back *and* shut the menu *and* closed the
dialogue, in an order decided by which part of the screen happened to be built first.

The fix is an arbiter — one place that knows the order and asks. It is called the Escape
ladder (`flutter_ui/lib/widgets/escape_ladder.dart`), it adds a single handler to the
keyboard for the whole application, and it has four rungs, from innermost outwards:

1. a gesture in flight — a drag, a pick, a path, a stroke, a type edit, a shortcut being
   captured;
2. the open menu chain;
3. the frontmost dialogue or full-window surface;
4. the finest selection on screen.

A surface *registers a claim* on the rung it belongs to when it appears, and stands down
when it goes. On a press the ladder asks the rungs in order, and the first claim that says
"I took it" ends the matter — nothing below it is even asked. A claim can also say "not
just now" (a paint tool that is mounted but has no stroke in progress), and then the press
carries on down. Pressing `Escape` with nothing to take back is not an error and is not
swallowed: it travels on to whatever else wants it.

The one thing deliberately left off the ladder is a text box you are typing in. An inline
rename or an open value box handles `Escape` on its own focus node, which the toolkit runs
after every keyboard handler, so it is reached only when no rung claimed the press — and
that is the right place for it, because a field should not need to know the ladder exists.
The order itself is written down in the interface spec (`docs/07-UI-SPEC.md` §14.1), which
is the part a reader should be able to check the behaviour against.


## 18. The Viewer's two strips, in plain terms

The Viewer is the picture, and two thin strips of chrome round it — one above and one
below. Everything on either of them changes how the picture is *shown*, never what the
picture is; the two exceptions say so in their own way, and are named below.

**The strip above** answers three questions, in three small pickers at its right-hand end.
How big is it on screen (Fit, or a percentage)? How good a preview am I being made — full
resolution, or a half or a quarter of it to keep playback moving, with the two playback
behaviours in the same menu because they are the same question asked a second way? And
what am I looking at — the *colour pipeline*, which is the recipe the engine uses to turn
the numbers it works in into something a screen can show. With no colour configuration
loaded there is one recipe, scene-linear to sRGB, and the menu lists that one thing; load
one (§22) and the menu grows a section per display it declares, each of its views a row,
with the built-in recipe still at the top as the way back. If the project names a
configuration that is missing or unusable, the picker's face says so and the menu carries
the reason: the picture is still being made, through the built-in recipe, and saying which
is the whole job of this control. If the exposure or the tone map is turned on, that same picker adds the
word "preview" and turns the accent colour: it is the Viewer saying, calmly, that what you
are seeing is not what an export would write.

**The strip below** starts with the ways of looking. The chequerboard turns the
transparency board on and off — the grey squares that show through wherever the picture is
see-through. The next mark opens the **view menu**, which is where everything that is
*drawn over* the picture lives: the grid, the broadcast safe rectangles, the layer controls
(the box and handles round a selected layer) and the region of interest (sweep a rectangle
and the engine renders only that part while you work on it). The composition's own
background colour is the last row of that menu — it is the odd one out, because it is a
real edit to the document that Ctrl+Z undoes, and it sits there because "what is behind the
picture" is the same question the chequerboard asks from the other side.

Then the channel mark, which shows one colour channel at a time as grey — the way to judge
a key or a matte — and then the exposure, which is just a number reading `+0.0`. Drag it
and the whole picture brightens or darkens by that many stops. It changes nothing an export
will ever see; it is there so you can look into a shadow without grading one.

A hairline seam, and then the **two snapshot marks**. Click the camera and it photographs
whatever the Viewer is showing this instant. Press and *hold* the eye beside it and the
photograph comes back over the live picture for as long as you hold — let go and you are
back. That is the before/after every grade leans on. The eye is greyed out until you have
actually taken a photograph, and hovering it then says so. (It was briefly one mark doing
both jobs — click to take, hold to compare — which worked but hid the comparison: nothing
on screen told you a photograph existed or how to look at it.) Nothing about either goes
near the engine or an export: it is a picture of the screen, kept in the frontend, thrown
away when you take the next one.

In the middle, the transport: to the start, back a frame, play, on a frame, to the end, and
the clock. Click the clock and type a time to go there.

Two strips is the default because it is what the drawing draws, but it is a choice:
**Settings → Appearance → Viewer → Viewer bars** will put everything on one strip instead,
above the picture or below it. Nothing is added or taken away by that — the same controls
in the same order, on one row rather than two, for anyone who would rather spend those 22
pixels once.

At the far right, one line that says what is on screen: which composition, at what time, at
how many pixels the engine actually made it, and how big it is being drawn. The second
pixel count is the interesting one — `1920×1080 → 960×540` means you are being shown a
half-resolution preview, which is why it may look softer than it will export. That used to
be a badge that appeared and vanished during playback and shoved the rest of the bar about
as it came and went; a line that is always there and always true is easier to trust and
easier to ignore.

**As the Viewer narrows, things leave the bottom strip in a fixed order**, and the
transport is the last of them. Drag the Viewer's edge in and first the line at the far
right shortens — it drops the arrowed preview size, then the composition's name — then it
goes altogether, because everything it says is said again in the header, the tabs and the
clock. Narrower still and the ways of looking fold into a single `⋯` mark that opens all of
them in a little floating strip: the very same controls, gathered rather than removed.
Narrower again and the clock goes. What is left, on a Viewer squeezed into a sidebar, is
the five transport buttons — because someone who has made the Viewer small is still
watching something, and a strip that had kept the exposure field and lost Play would have
kept the wrong half.

**And one mark on the picture itself**: the name of the selected layer, in a small
gold-outlined chip at the top-left corner. Selection is agreed in four places — the
Timeline, the graph, the properties and the Viewer — and this is the Viewer's half of that
agreement. The box round the layer is the same gold, which is the colour Lumit reserves for
"this is selected, or animated, or in your hand" and for nothing else.

## 19. The export queue, in plain terms

Exporting is the one job in Lumit that takes minutes rather than milliseconds, and only one
of them can run at a time — two exports sharing a graphics card make each other slow and
neither of them predictable. So the second one waits, and the thing it waits in is the
**queue**.

**What one queued item is.** Not "export this composition later", which would be a promise
about a moving target: an item is a *photograph* of the project, taken the moment you add
it. Every layer, every keyframe, every effect setting is copied into the item as it stood
then. Keep working afterwards — move a layer, change a colour, delete the whole
composition — and the item still writes what you queued. That copy costs almost nothing,
because the document is stored as shared, unchanging pieces: the photograph is a note
saying "these pieces", not a second copy of the work (docs/06 §7.1).

**Adding does not start.** The export dialog's footer has two actions for a reason. *Add to
queue* puts the item on the list and leaves it there — the list is a plan for later, and
nothing about it moves. *Export* adds the item and lets the queue run, which starts it now
and then starts the next one after it, and the one after that, until the list runs dry.
The queue window has its own start for the list you built with the first button.

**Nothing turns a crank in the background.** There is no timer thread in the engine driving
this. Whenever the interface asks the queue how it is getting on — which it does a few
times a second while a window that shows it is open — the answer is worked out first: any
finished export is settled, and if nothing is running and something is waiting, that
something is started right there. So "asking" and "moving along" are the same act, which
means the queue cannot get stuck in a state nobody is looking at, and there is no lock held
by a thread that nothing is watching.

**Cancelling and removing.** Cancelling something that is *running* asks it to stop at the
end of the frame it is on, and the half-written file is deleted — a cancelled export never
leaves something that looks like a finished one. Cancelling something that has not started
just takes it off the list: it has nothing to stop and nothing to report. Removing a
finished row forgets it.

**One place says what an export is doing.** The progress numbers live with the export in
flight, not on the item — the row in the queue window and the line in the status strip are
two readings of the same thing, so they cannot disagree about which frame it is on.

**Open folder.** The tick in the export dialog is remembered by the item, not by the window
that set it, so a long export whose dialog you closed an hour ago still opens the folder it
landed in.

### 19.1 What an export actually carries

The queue is *when* an export happens. This is *what* it writes.

**Every setting travels together.** There is one settings record per export — the format,
the frame, the bitrate, the colour depth, the crop, the metadata, everything — and it is
copied into the queued item alongside the photograph of the project. That is why an export
started an hour ago still writes what you asked for then, even if you have changed every
field in the dialog since.

**A format is a set of promises, and not all formats make the same ones.** An `.mp4` cannot
hold transparency; a folder of PNGs cannot hold sound; a `.wav` has no picture to set a
depth on; nothing lossless has a bitrate, because a bitrate is a budget for throwing detail
away. Rather than each part of the program remembering this, there is one **capability
table**: one row per format saying what it can carry. The dialog reads it to decide which
controls mean anything, and the exporter reads it to refuse a setting the format cannot
honour. Refusing matters more than it sounds — a file that quietly came out without the
transparency you asked for is a file you find out about from somebody else.

**Sound with no picture.** An export can be nothing but the composition's mix: an `.m4a`,
which is compressed like the sound in a video file, or a `.wav`, which is not compressed at
all. That path needs no compositor and no graphics card, because there is nothing to draw —
the mix is already made, and it goes straight into the file.

**The pack stage.** The compositor's answer to every frame is four numbers per pixel — red,
green, blue and *coverage* (how much of the pixel the picture actually fills), each one byte.
Files want other shapes, and the small piece of code that converts is the **pack stage**:

- **RGB** writes the coverage as "completely covered" everywhere, so the file is the picture
  over its own background — what you want when the file is the finished thing.
- **RGB + alpha** keeps the coverage, so the file can be laid over something else later.
- **Premultiplied** and **straight** are two ways of storing a half-covered pixel. Lumit
  works premultiplied — a half-covered red pixel is stored as half-strength red — because
  that is the form in which blending and blurring are simply correct. Straight stores the
  red at full strength and the coverage beside it, which is what paint programs and some
  delivery specifications ask for, so the pack stage divides the colour back up on the way
  out.
- **8-bit or 16-bit** is how finely each of those numbers is written. Only the still formats
  can carry sixteen; every video codec Lumit writes today is eight.

**Why sixteen bits needed a second target.** The compositor works in floating-point numbers,
which are far finer than either file depth — but the last step before a frame leaves the
graphics card puts it into a *bucket*, and the ordinary bucket is the one the screen uses:
one byte a channel, 256 possible values. For a while a "16-bit" export took that bucket and
stretched it, writing each of those 256 values as one of 65,536 — arithmetically tidy, and
completely pointless, because the fineness had already been thrown away. So the export now
has a **bucket of its own**, twice as wide, and asks the graphics card for the frame in that
one instead; the file finally gets what the compositor actually computed. A long, shallow
sky gradient is where you see the difference: in eight bits it comes out as visible bands,
and no amount of stretching afterwards can put back the values between them.

One wrinkle worth knowing, because it explains an oddity in the code: the graphics card can
apply the screen's brightness curve for free when it writes into the ordinary byte bucket,
and there is no wide bucket it will do that for. So for the wide one the curve is written out
by hand in the shader — the one place in Lumit where that sum is spelled twice — and a test
puts the same frame through both buckets and insists they agree, so the two cannot drift.

**Crop, and the region of interest.** A crop takes pixels off each edge — top, left, bottom,
right — counted in the composition's own pixels, and it decides the size of the file unless
you named a different frame. The **region of interest** is the rectangle you can sweep on the
Viewer to make previews faster; ticking *use region of interest* at export means "crop to
that instead". It has to be converted, because a region is stored as fractions of the frame
rather than as pixels — a preview at half resolution has half as many pixels, and the same
region has to mean the same part of the picture at every one of them.

**Metadata** is the handful of facts written *about* the file rather than in it: title,
author, copyright, comment, when it was made. They are kept in a fixed order rather than a
lookup table, for a reason worth knowing: an export must be **deterministic** — the same
project must produce the same bytes twice — and the order these are written in lands in the
file's bytes. A lookup table gives them back in whatever order it feels like.

**Render settings** say how the composition is rendered on the way through: the quality tier
(the same Full/Half/Quarter machinery the Viewer uses — an export takes Full), whether the
disk cache is touched (it is not: an export is one pass through the timeline, and filling the
cache with frames nobody will ask for again would push out the ones you are actually working
with), whether effects run, and whether solo switches are obeyed. The last two work by making
a throwaway copy of the project photograph with those switches flipped — the export renders
the copy, and nothing about your actual project changes. Two things the export drawing shows
here do not exist yet and are not faked: **guide layers** (a "reference only" mark on a layer)
and **proxies** (a small stand-in file used while you work). Both are features of the project
itself before they can be export settings, and TODO.md says what each would take.

**Auto bitrate.** A bitrate is how many bits a second of video is allowed. Bigger pictures and
more frames need more, roughly in proportion, so *Auto* multiplies the pixels-per-second by a
constant per codec — the constant chosen so that 1920×1080 at 60 comes out at exactly the
16 Mbps the delivery preset table already specifies. Typing a number overrides it; leaving the
field blank is a third answer meaning "let the encoder decide", which is what it has always
meant.

**A guide layer is one you can see but never deliver.** Tick a layer's **guide** switch and
it stays exactly as it is in the Viewer — a reference photograph, a grid, a title-safe
rectangle, an animatic to match to — but no file Lumit writes ever contains it. That is not
a rule the export path applies to the top layer of the composition and forgets about
underneath: when an export begins, Lumit takes its private photograph of the project and, in
that copy, every guide layer at every depth loses its picture, its sound and its solo — so
the piece that decides what to draw, the piece that decides which footage to decode, and the
piece that names the finished frame all agree the layer is not there. Inside a precomp, too:
a comp used as a layer in another comp delivers without its own guide layers. The Viewer
never takes that path, so it goes on drawing them. Solo does not rescue one: solo says *which
of these layers am I looking at*, and guide says *this one is not in the file* — two
different questions, so soloing a guide layer changes what you see and nothing about what is
written. An export can ask for them anyway with one tick, for the rare frame you want the
reference in.

**Presets** are a name over that whole settings record. The built-in ones — *Master* and the
delivery presets — cannot be edited or deleted, because a preset name has to mean the same
thing on everybody's machine. Your own are saved into one small file in Lumit's data folder,
so they follow you between projects. If that file goes missing or gets damaged, Lumit reads it
as "no saved presets" and carries on: losing your presets should cost you a re-save, never an
export.

**When done** is what happens at the end: two independent ticks — play a short sound, and
open the folder the file landed in — so a long export left running can do both, one, or
neither. The sound is a file that ships with Lumit, and when it is not there the hook simply
does nothing, because a missing ding must never make a finished export look failed. Both are
honoured by the *queue* as the item finishes rather than by the dialog that set them, so an
export whose dialog you closed an hour ago still does what it was asked to.

### 19.2 The export dialog, in plain terms

The dialog is where all of the above is chosen. Three things about it are worth knowing,
because none of them is the obvious way to build such a window.

**It is one page, not six.** The strip of words under the title — Output, Time, Picture,
Colour, Audio, Metadata — looks like tabs and behaves like a table of contents: everything
is on one page that scrolls, and the strip tells you *where you are* on it. Scroll down past
the Time section and the strip moves to Time on its own; click a word and the page scrolls
that section into view (unless it is already fully in front of you, in which case moving the
page would only be disorienting) and outlines it for half a second so your eye can find it.
The reason for one page rather than six is that an export is one decision: a window that put
the picture on one page and the sound on another would hide half of what you are about to
write from the person deciding it.

**A control it cannot honour is dead, not missing.** Choose an `.mp4` and the alpha-channel
row does not disappear — it greys out. Hover it and it says why. This is deliberate and it
is the opposite of what most programs do: a row that vanishes leaves you wondering whether
you imagined it, or looking for the setting in a menu that has not got it either. A greyed
row with a reason answers the question where the question is asked. The same face carries
the handful of rows the *engine* cannot back yet — proxies, guide layers, motion blur at
export — so "not built" and "not possible in this format" look the same on purpose: in both
cases the honest answer is that this file will not do that.

**The dialog does not work anything out for itself.** What a format can carry, whether the
settings add up to a file that can be written, what the crop leaves, what bitrate *Auto*
comes to — every one of those answers comes from the engine, through a call, and is
remembered until a field changes. Two reasons. The first is that a second opinion is a
second thing to be wrong: if the dialog worked out its own bitrate estimate, the day the
engine's changed it would quietly disagree. The second is speed — asking the engine while
Flutter is drawing would put a call across the boundary into the middle of every frame, so
the asking happens when you *change* something, which is thousands of times less often.

Whatever the engine refuses is printed in the footer, where the summary line usually is, and
the two buttons go inert until it is answered. That is the whole point of asking early: the
refusal arrives while the fields that caused it are still in front of you, rather than
minutes later from a queue you have stopped watching.

## 20. The node graph, in plain terms

A layer's effects have always been a list: the picture goes in at the top, each effect
changes it in turn, and what comes out at the bottom is what you see. The Graph panel
(K-471) does not replace that list — it *draws* it, as boxes joined left to right by
wires. Same effects, same order, just laid out so you can see the picture's path. Reorder
the list anywhere — in Effect controls, in the Timeline — and the boxes move with it at
once, because there is only one list and these are two windows onto it. (Dragging a box
about the canvas moves the *box*, not its place in the list. Where a box sits is saved
with the project, so a graph you have arranged stays arranged; the order the picture
travels in is the list's business, and the list is the one place it is set.)

**What the graph adds is the driver.** A driver is a box that makes no picture at all —
it makes a *number* (or a colour). Wiggle makes a number that wobbles. Audio level
listens to a layer's sound and makes a number that follows the loudness. Colour cycle
makes a colour that slowly turns. Drag a wire from a driver into one of an effect's
sockets, and that parameter now follows the driver instead of its keyframes — "the glow
pulses with the music" becomes one visible wire instead of a line of code. The ordinary
Effect controls row does not pretend otherwise: while a parameter is driven, its row says
*driven* and names the driver.

**Why the saved project barely changes.** The file still holds the effect list exactly as
before, so every old project opens untouched and saves back byte for byte. A layer that
has drivers keeps them in a small extra section beside the list — the drivers, their
wires, and where the boxes sit on the canvas. A project that never opens the Graph panel
never gains that section. And because the picture's path is always the list — a wire can
drive a *value*, but the image itself always flows straight down the stack — there is no
arrangement of boxes the plain list view would have to lie about.

**The colours are the types.** Every socket and every wire is coloured by what flows
through it: one colour for pictures and mattes, one for numbers, one for colours, one for
shapes and points, one for sound. You can read a graph's plumbing without clicking
anything — a wire will only plug into a socket of its own colour.

**Two letters and two switches.** A box carries an **E** and a **B**. `E` is *expose*: an
effect has dozens of parameters and drawing a socket for every one of them would make the
canvas unreadable, so a box normally shows only the picture's own sockets and whatever is
already wired. Pressing `E` opens it up and shows the lot; pressing it again folds it
back. `B` is *bypass* — the same switch the effect's card in Effect controls has, and a
bypassed box draws its outline as a dashed line so you can see at a glance which one is
sitting the frame out. In the panel's own strip along the top are two switches. **Auto-wire**:
let a wire go over empty canvas, pick a driver from the list that appears, and the wire
is joined to the new box for you. **Heal**: when you delete a box, the wires that were on
it go with it. Turn Heal off and a box with wires still on it is left alone until you
unplug it yourself — for people who would rather nothing disappeared that they did not
name. The `Tab` key opens that same list of drivers anywhere on the canvas.

**Where Audio level's sound actually comes from.** The part of the engine that works out
effect values knows nothing about files or codecs — that is deliberate, and it is why the
same code can be tested against a tone nobody had to decode. So Audio level does not read
a file; it asks for "the sound of that layer, between these two moments", and the pixel
side of the engine answers by decoding the layer's own footage. Two details matter. The
sound is decoded at one fixed rate (48 kHz) rather than at whatever rate the sound card
happened to ask for, so the number is a fact about your project and not about the machine
— otherwise the same frame would come out differently on a laptop and on a render box.
And the answering happens in the one place both the Viewer and the exporter build their
picture from, so there is no second copy of the logic to drift: what pulses on screen is
what lands in the file. A layer with no sound, a file that has moved, a reference to a
layer somebody deleted — each reads as silence, which is a picture that simply does not
pulse rather than an error.

**Deleting a driven effect takes its wires with it.** The wires live beside the effect
list, so removing an effect could leave a wire pointing at a box that no longer exists —
and the next thing you did on the canvas, even just dragging a box, would be refused
because of it. Removing an effect now drops the wires, canvas positions and expanded
badges that named it, in the same single step, so one undo brings the effect *and* its
wiring back together. This is the same rule as **Heal** above, applied where the deletion
comes from somewhere else entirely — the Effect controls panel, or the Timeline.

**Dragging a driver's number moves the picture.** The same trick as dragging any other
value: the engine takes a throwaway copy of the project with your half-finished number in
it, renders that, and throws it away — nothing is written down until you let go, so a
drag is one entry in the undo history rather than a hundred. And when you let go of a
wire over empty canvas and pick a driver from the list that appears, two things are worth
knowing. The list only offers boxes the wire could actually plug into, so picking one
always connects. And the box and its wire arrive together as one change, so one undo
takes both away — which is only possible because the engine can say what sockets a driver
has before that driver exists.

**A wire cannot put a parameter somewhere you could not type it.** Every parameter has a
range it is not allowed past — a blur radius stops at 2000 pixels, and never goes below
nought — and typing has always been held to it. A *wire* was not, until now, and one
driver made that visible in an unpleasant way. The Points sample driver answers "how far
is the nearest particle" and, when nothing is wired into it yet, answers a deliberately
enormous number: it means "nothing is anywhere near", which is the honest answer for the
usual use (the closer a particle gets, the brighter the lamp). But wire that into a blur
before you have wired the particles in, and the blur sat at a billion pixels — past a
limit no typed number can reach, in code written for two thousand. Now the number is
clamped where it lands, so the parameter sits at its own maximum instead. The panel still
tells you *why*: a driver with nothing feeding it wears a small warning mark, and every
row it drives says **no stream** where it would otherwise say *driven*.

Two details, because both were deliberate choices. The clamp is on the **effect's** end of
the wire, not on a driver's own settings: Remap exists precisely to take a wide number and
squeeze it into a small range, so clamping *its* input would break the one box written for
the problem. And it happens in the single piece of code both the Viewer and the exporter
run, so a clamped picture on screen is the clamped picture in the file — there is no
second opinion to drift.

**Points** are the fifth kind of thing a wire carries: not a picture but a crowd of
positions — where every particle of a particle system is this frame, how fast each is
moving, how old each is. Particulate (K-446) is the first thing that makes them, and
§23 explains the whole idea in plain terms. The type was defined with the graph, before
anything emitted one, so that Particulate and its future relatives (scatter, clone to
points, connect points) plug into sockets that already exist rather than needing the
plumbing rebuilt around them.

**Where you meet all this.** The Graph panel follows the selected layer, like Effect
controls. The **Nodes workspace** makes the graph the main surface, with a small viewer
beside it and a short timeline underneath. The **Node panel** sits under that viewer and
lists the settings of whichever box you last clicked — the same rows Effect controls
draws, but for one box at a time, and for drivers too, which have no place in an effect
list. Click a driver on the canvas and its Amount and Frequency are right there; click an
effect and you get its own rows, with the driven ones saying so. Neither of these is the
engine's internal "evaluation graph" (§1's compiler still builds that in private); what
you are looking at is always your own document.

**Seeing the picture at one box: the "at effect" chip.** Select an effect — either by
clicking its box on the graph, or by clicking its heading in the Effect controls list,
which are the same click as far as Lumit is concerned — and a small chip appears over the
Viewer's picture saying *at* that effect's name. Click it and the Viewer shows the
composition as it looks with that layer's effects stopping there: the blur applied and
nothing after it. Click it again and the finished picture comes back. Nothing has been
soloed, bypassed or switched off, and nothing about your project has changed; you are
just looking at a different point in the chain, in the Viewer you were already using, at
its own size and quality, with the exposure and channel controls and the scopes all
reading the picture in front of you.

**How the chip knows what to show.** There is no special "render up to here" machinery
behind it, and that is the point. Because the chain *is* the list, the picture at the
third box is simply the picture your composition makes if that layer had only its first
three effects. So the engine takes a throwaway copy of the project, shortens that one
layer's list, and renders it the ordinary way — the same trick already used when you drag
a value and the picture keeps up, where a copy carries the value you are dragging before
it is written down. Your project is never touched; the copy is thrown away with the
frame. And because it is the ordinary way, it is the ordinary Viewer: the picture arrives
by the same route at the same quality, and playing, scrubbing and dragging all work as
they always do.

Two things fall out of that, and both are why it was worth doing this way rather than
building a second, smaller viewer beside the first. Lumit remembers finished frames under
a *name* worked out from everything that went into them, effects included — so a
shortened list is automatically a different name, and the cupboard can never hand you the
full picture by mistake. The other is subtler and is the kind of bug that looks like
nothing at all: Lumit also keeps a short-cut list of those names so it does not have to
work them out over and over, and that list is only thrown away when the *document*
changes. Turning the chip on does not change the document. So the engine throws the list
away itself whenever the chip moves — the same thing it already does when you change the
exposure, for exactly the same reason.

## 21. The wordmark and the empty shell, in plain terms

**The wordmark is a picture, not a word.** The "lumit" at the top of the welcome page used
to be the five letters typed out in Lumit's mono face. It is now the mark itself: the same
drawing the website uses, where the `l` and the `t` are the two coloured keys of the Lumit
logo — the blue one, and the violet one that is the blue one turned upside down — with
`umi` between them. The file is the site's own, copied into the application's assets rather
than redrawn, because a brand mark drawn twice is a brand mark that will one day disagree
with itself. Three small changes were made when it was copied, all of them noted at the top
of `flutter_ui/lib/shell/wordmark.dart`.

**Two of the colours never change, and one always does.** The keys are the brand, so they
are the same under every colour scheme and under a theme somebody invented this morning.
The three middle letters are only lettering, and lettering has to be readable, so they are
chosen against the surface they are standing on. The maths is the ordinary "how bright is
this colour" calculation every browser and toolkit has: over halfway to white counts as a
light surface and takes dark letters, under it takes light ones, and when there is nothing
to judge — a theme that cannot say — the letters stay light, because Lumit is dark-first.
Those six colours live in `flutter_ui/lib/theme/brand.dart`: beside the theme, not in it,
since a theme colour is one that changes and these do not.

**The empty shell.** The welcome page can now be closed with nothing open — Escape does it,
and Settings ▸ General can stop it appearing at launch at all. That would once have left
somebody staring at an editor with no document and no obvious way to start one, so the
Viewer now shows the same three cards the welcome shows — New project, Blank project, Open
— until there is something to display. They are not a copy: they are the same piece of
interface, running the same three functions, mounted in a second place. If a project *does*
have compositions and simply has none open, the Viewer says what it always said — "select a
composition" — because that is a different situation and deserves a different sentence.

## 22. Colour management and OCIO, in plain terms

**What problem this solves.** A pixel's numbers do not say what colour they are. `(1.0,
0.2, 0.0)` is one orange on an sRGB monitor, a noticeably different orange in a cinema
file, and something else again in footage straight off a log-profile camera. Every serious
editor therefore needs to know, for each piece of footage, *what its numbers mean* — and,
for every screen and every exported file, *what numbers to write so the right colour comes
out*. Lumit already does the simple version of this: it assumes footage is ordinary video
or screen material, works internally in "linear light" (numbers proportional to actual
light, which is what makes blurs and blends physically correct), and converts back to
ordinary sRGB on the way to your monitor and your export.

**What OCIO is.** OpenColorIO is the film and VFX industry's standard way of writing those
meanings down. An *OCIO config* is a folder: one text file, `config.ocio`, that lists
colour spaces by name — "ACEScct", "sRGB", "Rec.1886" — plus a set of look-up-table files
it points at, each describing how to convert one space to another. Studios publish
configs; the ACES ones are the well-known examples. Any program that can load a config
speaks the same colour language as any other program that loaded it, which is the whole
point: a Lumit project and a Nuke project can agree about what their pixels mean.

**Where you choose one, and what choosing does.** The config is a **project** setting, not
a machine one — it changes what the composition looks like, so it travels with the `.lum`
and matches on somebody else's machine. It is chosen in **File ▸ Project settings ▸
Colour**: one row holding the path, a *Choose…* button, and a line underneath saying what
was read ("Loaded: 42 colour spaces, 3 displays") or, if the file is missing or uses
something Lumit has not built, why it is not in force. That same *Choose…* is how a config
that moved is pointed at again. The path is stored the way footage paths are, so a config
living beside the project keeps working when the folder moves.

Choosing one makes three lists longer, and that is genuinely all. Each footage item's
colour-space tag — a **Colour space** submenu on its row in the Project panel — offers the
config's names, so log camera footage can be interpreted correctly on the way in. The
Viewer's colour picker, the third one in the strip above the picture, grows a section per
"display" the config declares, with its "views" as the rows: the config's named ways of
showing linear light on a screen. And the export's colour-space dropdown lists the
config's names under a heading of their own, so a delivered file can be written in a space
the config defines. A name in any of those lists is the config author's own word and is
shown exactly as it was written — translating somebody's colour space would rename their
work.

**How the maths runs — and why we wrote it ourselves.** The official OCIO software is a
big C++ library, and it deliberately computes slightly differently on the processor and
the graphics card. Lumit's foundational promise is that the preview *is* the export, bit
for bit, and you cannot build that on a library with two answers. So Lumit reads the
config format itself, in Rust, the same way it already hosts OpenFX plugins itself and
reads `.cube` LUT files itself. When a config loads, each conversion it describes is
worked out once and "baked" into a small table — think of a curve plus a 65×65×65 cube of
pre-computed answers — and that one table is what both the Viewer and the export use.
One implementation, one answer.

**How do we know our answers are right?** Only ever by comparing them against numbers
somebody else published — never against numbers Lumit produced, because a table made by
the code it is supposed to test proves nothing but that the code agrees with itself.
Three kinds of published number are checked in today. The first is *constants from the
standards*: the sRGB curve's own published values, the ACES constants the Academy prints,
white points that must come out white by the definition of a white point. The second is
the **CLF suite**. CLF is the film industry's file format for writing a colour transform
down as a list of steps — a matrix, then a curve, then a table — and Lumit reads it, so
the specification's own example documents are checked into the repository and run
through our reader, each one against an expectation the document itself publishes (some
of them literally print the formula that generated their table). Those found two real
faults the day they landed, which is what a test suite is for.

The third kind is a run of the **official OpenColorIO library** over the two real studio
configs everyone uses, producing nine hundred expected answers to check ours against.
That needs somebody at a machine with the library installed, and the recipe — which
library, which configs, which command — is written down in the fixtures folder so anyone
can repeat it. It has now been done, and the results are checked in beside the configs
that produced them.

It was expected to be a data drop and it was not: it found five separate faults, every
one of them the kind that only a real published config produces. One of them was that
"do nothing" was not doing nothing (below). Another was that a placeholder the newer ACES
configs write instead of a display's name — because one view description is shared by
eight displays — was not being understood at all, which meant none of those configs'
views worked. A third was that a *data* channel, a matte or a depth pass, was still being
put through a colour conversion. It also put a number on something the design had only
warned about: the newest ACES look, when stored as a table rather than as code, is
accurate to well under one part in five hundred for ordinary colours and about a tenth
off at a fully saturated blue. That number is written down and tested against, so the
day someone writes the code version, the improvement is measurable rather than claimed.

**Where those tables live.** The five ACES tables are big — 47 megabytes between them —
and for a while they were baked into the program file itself, which meant every copy of
the engine carried them whether it needed colour management or not. They now travel as
ordinary files in a small `colour` folder that ships beside the application (inside the
app bundle on a Mac), and the engine reads them the first time a config asks for one.
Nothing changes if the folder is missing except honesty: the styles that need those
tables refuse by name, the same refusal as any other feature that is not there, never a
wrong picture. On a development machine the engine simply reads them straight out of the
source tree.

And when a config uses some feature we have not built, Lumit refuses it *by name* — it
tells you what it cannot do — rather than quietly producing almost-right colours, because
a plausible wrong picture is the one failure this design refuses to ship.

### Why "do nothing" has to actually do nothing

A config describes each colour space one way round and lets the program work out the
other. So "convert from this space to that one" often ends up as a recipe like *apply this
table of nine numbers, then apply the table that undoes it*. On paper those two cancel and
the colour comes out exactly as it went in.

On a computer they do not quite cancel, and the reason is worth knowing because it comes up
everywhere. A number in the picture is stored to about seven decimal digits — very
precise, but not exact. When the first table runs, one channel can be sent a long way from
where it started; the second table brings it back. If the excursion was to a number ten
thousand times bigger than the answer, the seven digits of precision that were plenty at
the start are only three digits by the time it comes home. The colour is subtly wrong, and
nothing in the recipe looks wrong at all.

Lumit's answer is to never take the trip. When a recipe has two of these tables next to
each other, they are multiplied together into one table *before* any picture goes through —
and that multiplication is done at double precision, sixteen digits instead of seven, so
the two really do cancel. If the combined table turns out to be the do-nothing one, it is
dropped from the recipe altogether rather than kept as a formality.

That last step sounds pedantic and is not. A picture can legitimately contain infinity —
a value so bright it has run off the top of what the format holds. Multiplying infinity by
a very small number is still infinity, and adding infinity to minus infinity gives "not a
number", which shows up as a black or transparent hole. A step that does nothing must be
*absent*, not merely harmless, or it can turn one broken channel into three.

### Colours off the end of the scale

A baked table has a first entry and a last one, so there is always a question about what
happens to a colour past either end — and in a colour pipeline that question is not a
corner case, it is where the interesting colours live. Two kinds of value fall outside
the ordinary nought-to-one range. **Above one** are highlights: a practical light in shot,
a specular hit off chrome, a sun. **Below nought** are the colours a wide-gamut camera
saw that an ordinary monitor's three primaries cannot make; converting them into Lumit's
working space leaves them as slightly negative numbers, which is not a mistake but the
honest arithmetic of "redder than this red".

The tempting answer is to keep the original formulas beside the table and work those
values out properly when one turns up. That is exact — on the processor. The graphics card
cannot do it: it would have to re-implement every logarithm and power in the chain and
agree with the processor's version of them to the last decimal, and some steps in a chain
are themselves tables with no formula to re-implement. Preview and export would part
company precisely on the colours people care most about.

So Lumit does the opposite: it makes the *table* cover everything instead. The samples are
not spread evenly across nought to one. They are spread **logarithmically and
symmetrically about zero** — packed tightly around black, where the eye is most sensitive
and every transfer curve bends hardest, and thinning out towards the extremes, with the
same treatment mirrored on the negative side. Sixteen thousand samples arranged this way
reach from minus sixty-five thousand to plus sixty-five thousand, which is everything the
working format can hold, and the darkest part of the picture still gets more samples than
an evenly-spread table of the same size would give it. Looking a colour up is then the
same two steps everywhere: fold the value into the table's scale, then blend the two
nearest entries. The processor and the graphics card both do exactly that, so they cannot
disagree — which is the point.

**What happens when the config file goes missing.** Nothing dramatic. The project opens,
every assignment keeps its name, and the picture falls back to the built-in colour
handling with the Viewer's picker saying so calmly. The one thing that refuses is export:
writing a delivery file in a colour space Lumit cannot currently compute would produce a
wrong file, and a wrong file that looks finished is worse than an export that asks you to
relink the config first.

## 23. Particulate and the points stream, in plain terms

A particle system makes many small things — sparks, dust, snow — that are born, drift
about, and fade away. The usual way to build one is a **simulation**: every frame takes
the previous frame's particles and nudges them along. Lumit deliberately does not do
that, because a simulation can only answer "what does frame 500 look like?" by first
computing frames 0 to 499 — so scrubbing stutters, and two renders of the same project
can disagree. **Particulate** works the other way round: the moment a particle is born,
its whole life is settled — where it starts, which way it flies, how gravity and wind
will carry it — from a seeded random number that never changes. Asking for any frame is
then plain arithmetic: for each particle alive at that moment, work out where its
formula puts it, and draw it. Scrubbing anywhere is instant, the export matches the
preview exactly, and the same project makes the same pixels forever. The price is that
particles cannot react to each other — no collisions, no flocking — and for the montage
work this effect exists for, that is the right trade.

**The stream is the second thing it makes.** Besides drawing its particles over the
layer, Particulate hands them out as *data*: the **points stream** — every live
particle's position, speed, age, size, rotation and colour, this frame. On the Graph
panel that is a teal socket on Particulate's box, and a wire from it works exactly like
a driver's wire: it carries the data to whatever box you plug it into. The picture
still flows straight down the effect list, untouched — a points wire runs *beside* the
picture's path, never through it, so the plain Effect controls view never has to lie
about what is going on.

**The first thing that plugs in is a driver called Points sample.** It reads the stream
and turns it into numbers: how many particles are alive, and how far the nearest one is
from a point you choose on the picture. Those numbers can drive any parameter through
the wiring that already exists — so "the lamp glows brighter as a spark drifts past it"
is Particulate's Points socket wired into Points sample, and its Nearest distance wired
(through a Remap) into the glow's strength. No new machinery, just a new box that knows
how to read the crowd. Because the stream is pure arithmetic — never read off the
rendered picture — the driver can work it out on the processor, bit-for-bit the same on
every machine, and what it reports is always exactly the crowd the picture draws.

**Reading the crowd means working the crowd out first — including whatever is driving
it.** Particulate's own settings can be on wires too: an Audio level into Emit rate, a
Wiggle into Position. So when Points sample asks "where is everybody", the answer cannot
be worked out from the numbers typed into Particulate's panel — it has to be worked out
from the numbers Particulate is actually running on this frame, wires and all. That makes
the wiring walk *re-enter itself*: answering one wire sends it off to answer several
others first. It cannot go round for ever, because the one arrangement that would send it
in a circle — the particles depending on a number that depends on the particles — is the
loop the graph refuses when you draw it. And because two boxes reading one Particulate
would otherwise work the same crowd out twice, the walk remembers each producer's answer
for the rest of that frame: one Particulate, one crowd, however many things are reading
it.

**One thing a wire on Emit rate does *not* do: rewrite the past.** How many particles
exist is the running total of the emission rate since the layer began — so it is read off
the rate you *authored*, frame by frame, all the way back. A wire on Emit rate is a number
for *this* frame; it does not go back and re-decide how many particles were born a
thousand frames ago. That is deliberate twice over: re-walking every wire for every past
frame would make one picture cost a thousand wiring walks, and a history that rewrote
itself as a wire wobbled would make particles blink in and out rather than drift. The
picture and the Points sample driver both follow that same rule, which is why the number
the driver reports is always the crowd you are looking at.

**One wire the graph will politely refuse**: feeding Particulate's own particles back
into Particulate's own settings (say, wiring Count into Emit rate). That is a loop — the
particles would depend on a number that depends on the particles — and like every other
loop it is refused calmly when you try to connect it, rather than half-working.

**And one rule about *order*.** When a points wire joins two boxes that are both in the
effect list, the one making the particles has to sit **above** the one reading them —
because a later effect's stream can depend on the picture arriving at it, and that picture
is not made yet when something earlier asks for it. Drawn the wrong way round, the wire is
refused with the same "this would close a loop" message. But dragging the effects into a
new order is a different matter: rearranging the list is an edit to the *list*, and Lumit
will not refuse a perfectly ordinary drag because of a wire somewhere else. So if you drag
the producer below its reader, the wire is quietly dropped as part of that same drag —
one action, one undo. It is the same thing that happens when you delete the effect a wire
is plugged into: the wire goes with it, rather than being left pointing at nothing.

**A Points sample with nothing plugged into it says so.** The driver has to answer
*something* when there are no particles to look at, and what it answers is "nothing is
anywhere near" — deliberately an enormous distance, because the usual use of the wire is
"the nearer a particle gets, the more this happens", and answering *nought* would mean
"a particle is right here" and turn everything on at once. That is the honest answer, but
it is a startling one to meet by accident: wire the distance into a parameter before you
have wired the particles in, and the parameter shoots to the far end of its range and
stays there. So the box wears a small amber `!` while its Points socket is empty, and any
row that follows it says **no stream** where it would otherwise say *driven*. Neither
refuses anything — the wire is perfectly legal and is often drawn before the stream is —
they simply mean the number arriving is a placeholder rather than a measurement, and the
mark clears the moment a stream reaches the box.

**How the card actually draws a hundred thousand of them.** The awkward part of a particle
system on a graphics card is that nobody knows *how many* particles there are until they
have all been asked. A card is happiest drawing a fixed number of the same thing; "draw
however many of these turn out to be alive" is the one shape it finds difficult. Lumit
does it in four goes, and the order matters more than the speed:

1. **Count.** One tiny program per *candidate* — every particle that was born recently
   enough that it might still be around — answering one question: alive, yes or no. That
   costs one dice roll each, so a million of them is nothing.
2. **Add up.** The yes/no answers are added up running-total fashion, so that each living
   particle learns its own place in the finished list: "you are the four hundred and
   sixth". This is done as a *sum*, deliberately, and not by letting the particles grab
   numbered slots as they finish. Grabbing would be faster to write and would give a
   different answer every time, because which one finishes first is up to the card's mood
   — and then a particle's identity would change between two renders of the same frame,
   which is exactly what the whole design exists to prevent.
3. **Place.** Each living particle works out its position, speed, size, colour and turn in
   full, and writes itself into the slot the sum gave it. What comes out is the points
   stream: one tidy list, in birth order, with no gaps.
4. **Draw.** One small square per particle, all in a single instruction — the card is
   told "draw this square N times, and here is the list", and it reads the list itself.
   The number N comes off the card's own memory, so nothing has to be sent back to the
   computer and waited for in the middle of a frame.

The three looks — a soft round dot, a picture stamped from another layer, a streak — are
that same square with something different painted inside it. A streak is the dot smeared
from where the particle was a moment ago to where it is now, and *where it was a moment
ago* is worked out with the same formula run at an earlier age, not remembered. Which is
why a streak costs nothing extra and is exactly a dot when you set its length to zero.

**Two things are handed to the effect from outside**, because they are not numbers anybody
typed into a box. One is the layer's own clock. The other is the birth schedule — the
record of how many particles were born on every frame since the layer started, which
depends on the whole history of the Emit rate slider rather than on its value right now.
Both are worked out once by the part of Lumit that plans a frame, and travel alongside the
effect exactly as a mask's traced outline does.

**The same maths, twice, on purpose.** Every formula in this effect exists in two places:
plain Rust for the processor, and the card's own language for the card. That sounds like
the kind of duplication one should avoid, and normally it is — but the Rust one is the
*referee*. A test runs both and compares the answers, particle by particle. They cannot be
made identical to the last digit: a card's sine and its exponential are its own, accurate
to about a millionth rather than exactly, and a position is a speed multiplied by a time so
that millionth travels. So the test asks for agreement to one part in a hundred thousand of
each quantity's range, which on a 1920-wide composition is two hundredths of a pixel, and
notes where the answers came out closer than that anyway.

**What happens when you ask for too many.** Max particles is the dial, and it is honestly a
budget rather than a limit: it is the number Lumit reserves memory for. Ask for more
particles than the dial allows and the **newest** ones survive — old ones vanish a little
early, visibly, and in the same way whichever direction you scrubbed from. If the machine
falls behind while you are working, the same rule runs again at half the number, and half
again after that. That never happens on an export: a delivery is not allowed to be the
cheaper picture.

**What comes later** plugs into the same socket: Connect points (lines between nearby
particles), Clone to points (a layer stamped at every particle), Trail, Scatter, and
emitting particles from the image's own bright pixels. Each is a named future package;
none of them will need the plumbing rebuilt, because the socket, the wire and the
stream's shape are settled now. The full plan lives in docs/impl/points-stream.md, and
the effect's own design — every slider, formula and budget — in
docs/impl/particulate.md.

## 24. What the export writes, and how big: colour spaces and resampling, in plain terms

**What a colour space actually is — two things, not one.** Section 22 said a pixel's
numbers do not say what colour they are. Here is what a colour space adds to them. First,
**primaries**: which exact red, green and blue the three numbers are amounts of. Two files
can both say "full red" and mean visibly different reds, because their red lights are
different. Second, a **transfer function**: the curve that turns a stored number into an
amount of light. Numbers are not proportional to light in a delivery file — they are bent,
so that the codes are spent where the eye can see the difference, and the screen bends them
back. A colour space is one choice of each. Nothing else.

**Why five and not one.** Lumit's compositor works in linear light and hands the export a
frame already encoded the way an ordinary screen wants it — that is the Viewer's picture,
and it is what every Lumit export used to write, full stop. But delivery specs ask for other
things. A broadcast house asks for **Rec. 709** proper. A wide-gamut master wants **Rec.
2020**, whose red, green and blue are far more saturated, so it can hold colours 709 simply
cannot mix. Apple's world wants **Display P3**. And a file going straight back into another
compositor wants **Linear** — no curve at all, numbers proportional to light, because a
curve is only something the next program has to undo. So there are five, and the one you get
if you do not choose is the one you always got.

**How a conversion works.** Undo the incoming curve to get back to light. Multiply the three
light values by a small 3×3 table that says "this much of *our* red, green and blue makes the
same colour as that much of *theirs*". Apply the destination's curve. That table is worked
out in the code from the two spaces' published red/green/blue coordinates, rather than typed
in from a book — typed digits are how a colour bug gets in and stays for years — and the
tests check the worked-out table against the numbers the standards themselves print.

**The file has to say which one it is.** This is the part that matters more than the maths.
A player handed a Rec. 2020 file with no label assumes ordinary sRGB, plays it wrongly, and
the picture comes back looking washed out or lurid — and nobody can tell from the file that
anything is wrong. So an `.mp4` writes three small numbers into the container naming its
primaries, its curve and its matrix. A folder of PNGs has nowhere dependable to put them, so
Lumit does not offer the other four spaces for stills at all: the setting is greyed with a
reason, and an export that asks anyway is refused before a frame is drawn. Refusing is the
house rule (K-479) — a file that quietly is not what you asked for is a mistake you find out
about from somebody else, after you have sent it.

**Resampling: what "High" and "Fast" actually do.** When the exported frame is smaller than
the composition, several source pixels have to become one. **Fast** looks at the four pixels
nearest to where the new pixel's centre lands and mixes them by distance. That is quick, and
for a gentle shrink it is fine. But shrink by four and it is still only ever looking at four
of the sixteen pixels it is meant to be summarising — the other twelve are simply not
consulted, and fine detail turns into a shimmering interference pattern instead of an
average. **High** uses a wider, better-shaped window (Lanczos-3) *and* widens that window in
proportion to the shrink, so every source pixel that falls inside the new pixel's footprint
gets a say. A fine checkerboard shrunk 4:1 should be an even grey; High gives you that grey,
and the test in the engine checks it against exactly that arithmetic. Fast stays the default
because changing it would silently alter what every existing export writes, and an export
that quietly changed is the thing this whole part of the program is built to avoid.

## 25. Proxies, in plain terms

Editing 6K footage on any machine is slow for one boring reason: every frame you scrub past
has to be read off the disk and decoded, and a 6K frame is a lot of pixels to decode for a
picture you are looking at in a window a quarter that size. A **proxy** is the old,
obvious answer. You make a small copy of the clip once — half the width, so a quarter of the
pixels — and tell Lumit to look at *that* while you work. When you come to deliver, the big
file comes back and the small one is forgotten. Every editing application has this, and it is
the single biggest thing you can do to make a heavy project feel light.

**What Lumit stores.** A footage item keeps its own file reference: where the clip is, and a
fingerprint so it can be found again if it moves. A proxy is a *second* one of those, kept
beside it — same path handling, same fingerprint, same relinking — plus a tick saying whether
this item is currently using it. There is also one switch for the whole project, so you can
say "show me what I am really delivering" without going round every clip and back.

**Where the danger is, and it is not where you would guess.** The obvious risk is that the
wrong file gets opened. That one is harmless, because you can see it. The real risk is in the
frame cache. Lumit gives every finished frame a *name* made from everything that went into
it, so it never has to render the same picture twice (§4's cache notes go into this). If the
name did not mention *which file* the pixels came out of, then the small version of frame 300
and the big version of frame 300 would have the same name — and once one of them was banked,
the other would never be rendered again. You would turn proxies off, and get the soft picture
back anyway, with nothing on screen to explain it. Worse, you could export it.

So there is exactly one function in the engine that answers "which file does this item read?"
— and both the part that opens files and the part that names frames ask *it*, never each
other and never separately. The name already contained the file path, so the moment the
answer changes, every frame that reads that item is renamed by itself. Nothing had to be
added to the naming scheme; it just had to be asked in one place. That is also why switching
proxies back off is instant: those full-resolution frames are still sitting in the cache
under their own names, waiting.

**What a proxy is *not* allowed to change.** It has fewer pixels, and that is all. The layer
is still laid out at the original's size, still runs at the original's rate, and still has the
original's number of frames — so no position, no mask, no effect setting means anything
different when you switch a proxy on. If you have moved a layer to x = 1400, it stays at
1400; the picture inside it is simply decoded smaller and drawn into the same box. This is
the same trick the preview-resolution setting has always used, and it uses the same machinery.

**When a proxy is refused.** If the small file has a different number of frames, or a
different frame rate, it is not a smaller copy of this clip — it is a copy of *something
else*, and its frame 300 is not this clip's frame 300. Lumit will not use it: it quietly goes
back to the original rather than showing you the wrong moment. Same if the proxy is missing,
unreadable, or has not been looked at yet. And if the *original* goes missing, you get the
colour bars and the relink prompt even though a perfectly good proxy is sitting there, because
the clip in your timeline is the original and pretending otherwise would hide a lost file.

**Exporting.** The export ignores your proxies. Not "usually" — by default, always, whatever
the project is set to, because delivering the small version by accident is exactly the mistake
that is expensive to discover later. If you genuinely want a quick small file for someone to
review, you ask for it on the export itself. Mechanically this works the way the guide-layer
override does: the export makes its own throwaway copy of the document with the proxy switch
turned off, so everything downstream — what gets opened, what the frames are called, what
happens inside nested compositions — agrees without anybody having to pass an extra flag
around.

**Making one.** Lumit reads every frame of the original in order and writes it out half size
through the same encoder an export uses, into a file called `something_proxy.mov` sitting
next to the original. Beside the original rather than in a project folder, because the proxy
belongs to the footage: use the same clip in three projects and you want one proxy, not
three, and moving the folder of clips takes the proxies with it. It runs in the background and
reports progress exactly as an export does, because from where you are sitting it is the same
kind of wait. Sound is not copied — the audio always comes from the original, so there is
nothing to gain by re-encoding it and something to lose.
