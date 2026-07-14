# The plain-English guide to Kiriko's code

**Who this is for:** the project owner — someone who knows editing software inside-out but
has never written Rust and hasn't worked with threads or GPUs directly. Read this once and
you'll be able to navigate the codebase, understand what each part does and why, and make
changes without fear. Everything here is explained with editing analogies where they help.
No prior programming knowledge in Rust is assumed; general "I've seen code before" level is.

This guide is **kept current by rule** (CLAUDE.md): whenever a new concept enters the
codebase, a plain-English section for it is added here in the same commit.

---

## 1. The 30-second map

Kiriko is split into **crates** (Rust's word for a module/library — think of them as the
app's departments). They live in `crates/`:

| Crate | Job | Plain English |
|---|---|---|
| `kiriko-core` | Time, the document, undo | The project file's brain: what a comp/layer *is*, and every edit that can happen to it |
| `kiriko-project` | `.kir` files, autosave, recovery | Saving and loading, and the "never lose work" machinery |
| `kiriko-ui` | Everything you see | Panels, menus, the theme — the shell around the engine |
| `kiriko-app` | The `main()` entry | Ten lines that open the window and start the UI |
| `kiriko-media` | (coming) decoding video | Turning an .mp4 into frames |
| `kiriko-gpu` | (coming) the GPU pipeline | Drawing and processing frames on the graphics card |
| `kiriko-audio` | (coming) sound | Playback and the clock everything syncs to |
| `kiriko-eval` | (coming) the render engine | Working out what each frame looks like |
| `kiriko-cache` | (coming) caching | Remembering rendered frames so they're never rendered twice |

Three of these have proper names you'll see in the app and docs (decision K-067),
drawn from the Edo-kiriko craft the project is named for: **Togi** (polishing) is the
render pipeline — `kiriko-eval` + `kiriko-gpu` working together to turn the project's
cuts into the picture; **Kura** (storehouse) is the cache; **Hibiki** (resonance) is the
audio engine whose clock everything syncs to. Crate names stay plain `kiriko-*` — the
names are for people, the identifiers are for code.

**One rule ties them together:** the engine crates never depend on the UI. The UI asks the
engine for things; the engine doesn't know the UI exists. That's why the UI could be
replaced entirely without touching the engine — like swapping a car's dashboard without
opening the engine bay.

## 2. Rust in ten minutes, Kiriko edition

You don't need to write Rust to read it. The handful of ideas that appear everywhere:

- **Ownership.** Every piece of data in Rust has exactly one owner, and the compiler
  enforces it. When you see code "cloning" a document, that's making an independent copy so
  two parts of the app can't fight over one. This is the language feature that makes the
  "never crashes" goal realistic — whole categories of crash (two threads corrupting the
  same memory) simply don't compile.
- **`Result` — errors are values, not explosions.** A function that can fail returns
  `Result<Thing, Error>`: either `Ok(thing)` or `Err(why)`. The caller *must* deal with
  both. You'll see `?` a lot — it means "if this failed, pass the error up to my caller".
  Kiriko bans the shortcuts (`unwrap`/`panic`) that turn errors into crashes; the build
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

A thread is an independent worker inside the program. Kiriko's design gives each worker a
fixed job (the full table is in [05-ARCHITECTURE.md](05-ARCHITECTURE.md)):

- **The UI thread** is front-of-house: it draws the interface and responds to your mouse.
  The golden rule — it **never** does heavy work. Every stutter you've ever felt in AE is
  some engineer breaking this rule. In Kiriko it's structural: the UI thread hands work to
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

- `crates/kiriko-core/src/time.rs` — **Rational time.** Times are stored as exact fractions
  (`num/den`), never decimals, so frame maths is exact forever (a 3-hour NTSC timeline
  never drifts by a frame). The four "timebases" (source/clip/layer/comp time — glossary §4)
  are separate types, so mixing them up is a compile error, not a subtle bug.
- `crates/kiriko-core/src/model.rs` — **What a project is.** Structs for the document,
  comps, layers, footage items. Each has an `extra` field that preserves anything a future
  Kiriko version adds — so old and new versions can share project files.
- `crates/kiriko-core/src/ops.rs` — **Every possible edit, as data.** An edit is an `Op`
  (AddLayer, SetLayerSpan…). Applying an op returns its exact inverse — that pair is what
  makes undo *provably* correct instead of hopefully correct.
- `crates/kiriko-core/src/anim.rs` — **the keyframe engine.** Between two keyframes the
  value follows a bezier curve shaped by AE-style *speed* (units per second) and
  *influence* (how far each handle reaches). The subtle part: the curve is parametric, so
  "value at time t" first requires solving "where on the curve is x = t?" — done with a
  solver that combines Newton's speed with a bracket it mathematically cannot escape.
  That solver quality is exactly what makes handles feel right in a graph editor at the
  extremes (AE's 100% influence "spike" case is a test here). Property tests fire
  thousands of random curves at it per CI run.
- `crates/kiriko-core/src/retime.rs` — **the Retime maths.** One store per clip answers
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
  each halfway (the real slow-mo trick). Flow lives in its own crate (`kiriko-flow`) — a pure,
  deterministic CPU reference (pyramidal Lucas–Kanade motion estimation, then motion-compensated
  synthesis with a graceful crossfade fallback where the motion is unreliable). It's the "oracle"
  the later fast GPU version must match, and it's tested against known translations (motion
  recovered to sub-pixel) and against a plain crossfade (sharper on textured motion). The
  frame-pick and each interpolation are shared functions used by *both* preview and export, so a
  slow-mo frame is identical in each — the preview-equals-export promise holds for interpolation
  too. The same Frames toggle appears per-clip on Sequence layers (next to Clip speed %), so a
  single slowed clip can flow-interpolate while its neighbours stay crisp.
  **This is wired up for
  Footage layers now**: a Speed % box in a footage layer's twirl-down retimes it (50% =
  half speed, and so on), and the same Retime map feeds preview, export, and the cache
  key — so a retimed clip previews, exports, and caches consistently. The Speed box is a
  ramp: a start speed → an end speed with an ease (Linear/Slow/Fast/Smooth/Sharp), so a
  clip can rush in and settle — the core montage gesture — not just play at one flat rate.
  When a retime speeds a clip up so much that it runs out of footage, `overrun_local_time`
  reports the exact moment it runs dry — the point where the last frame gets held rather
  than inventing more footage. That point is now drawn on the clip as a kraft warning line
  with the held tail hatched (kraft, never a red alarm — house rule), and right-clicking the
  clip offers **Trim to source end** to cut it there. It never trims for you (boundaries must
  stay put so cuts keep landing on the beat). Sequence layers, the graph-editor lenses, and
  per-beat cutting come next.
- `crates/kiriko-core/src/sequence.rs` — **Sequence layers (the model).** A Sequence layer
  is one timeline row holding clips laid end to end — Kiriko's Vegas-style editing surface.
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
- `crates/kiriko-core/src/store.rs` — **The document store**: applies ops, publishes
  snapshots, keeps the undo/redo stacks.
- `crates/kiriko-project/src/lib.rs` — **`.kir` files.** A `.kir` is a zip containing
  readable JSON (rename one to `.zip` and look inside — genuinely). Saves are atomic:
  written to a temp file, flushed to disk, then renamed over the old file, so a crash
  mid-save can never destroy the previous save. The **journal** logs every edit to a side
  file the instant it happens; after a crash, replaying it restores your work.
- `crates/kiriko-media/` — **reading media files** (via FFmpeg, the industry-standard
  media library). Two jobs so far: the *probe* (a file's vital statistics — resolution,
  frame rate, duration — shown under each item in the Project panel) and the *frame
  index* — a scan of the whole file that records where every frame and keyframe sits, so
  scrubbing can land on exactly the right frame. Indexing runs on a background thread
  (the UI never waits) and the result is cached on disk, keyed by a *fingerprint* of the
  file's content — change the file and the stale index is ignored automatically.
- `crates/kiriko-gpu/` — **the colour foundation.** All engine maths happens on
  "light-linear" values (where adding two lights behaves like real light); files and
  screens use sRGB encoding. This crate owns the only two crossings between those worlds
  — decode-side linearise and display-side encode — and a "golden" test proves every
  possible 8-bit value survives the round trip within one step. That test is what makes
  the washed-out/too-dark "double gamma" class of bug impossible to reintroduce, and it's
  the bedrock of the preview-equals-export promise (K-031). The clever part: the shader
  contains no gamma maths at all — the GPU's texture formats do the conversions in
  hardware, so decode and encode can never drift apart.
- `crates/kiriko-gpu/src/composite.rs` — **the compositor seed.** Each layer is a picture
  on glass; the compositor stacks the glass on the GPU. Position/scale/rotation move each
  sheet (already as full 4×4 matrices, so 3D later needs no rewrite), opacity fades it,
  and stacking happens in linear light where combining images behaves like combining real
  light — a test proves the result differs from the naive approach by exactly the amount
  physics predicts. This is the beginning of the evaluator: the thing that will one day
  render whole comps with effects.
- `crates/kiriko-gpu/src/oklab.rs` — **perceptual colour.** Two colour worlds, two jobs:
  linear RGB is where *light* combines correctly (layering, glow, exposure), and Oklab is
  where *perception* behaves — a gradient interpolated in Oklab stays vivid where an RGB
  gradient sags into grey, and rotating a hue in Oklab keeps its brightness. Kiriko
  converts on the fly (a handful of multiplications per pixel), users never see anything
  but normal RGB values, and tests pin both promises: round-trips are exact and hue
  rotation provably never changes lightness.
- `crates/kiriko-cache/` — **the cupboard with a size limit.** Rendered and decoded
  frames get remembered so they're never computed twice; when the cupboard is full,
  whatever was used longest ago gets thrown out first. The limit is in bytes, not item
  counts — one 4K frame costs what sixty thumbnails cost, and budgeting any other way is
  how apps balloon. This is the seed of the three-tier cache the whole engine design
  revolves around.
- `crates/kiriko-ui/src/export.rs` — **writing video files.** Every frame of a comp is
  rendered through the *exact same* colour engine and compositor the Viewer uses, then
  compressed to an .mp4. Using one shared path isn't laziness — it's the design's central
  promise (what you preview IS what you export), and it runs on its own worker so the app
  stays responsive while exporting, with live progress and a real cancel. Besides the comp's
  own size you can pick an **export preset** — YouTube 1080p/4K, or vertical 1080×1920 — and
  Kiriko fits the picture into that frame keeping its shape, adding black bars where the
  aspect ratios differ (a wide comp gets bars top and bottom in a vertical export). The
  fitting maths (`fit_contain` / `letterbox_resize` in `pixels.rs`) is unit-tested.
- `crates/kiriko-audio/` — **playback and the clock.** The sound card asks for samples on
  its own strict schedule through a "realtime callback" — a tiny function that must never
  wait for anything (if it's ever late, you hear a glitch). The count of samples it has
  played *is* the playback clock: video asks "what time is it?" every frame and shows
  whatever frame matches. One clock, owned by the audio hardware — that's why picture and
  sound can't drift apart, and it's the same design the full engine keeps forever.
- **Composition audio and playback** (`kiriko-audio::mix`) — pressing Space on a comp now
  plays it. A comp can have many layers that make sound, each starting at its own moment;
  to play it, Kiriko decodes each one and lays them on a single strip at the right offset
  and trim, then adds them together (a mixing desk summing channels — `mix_stereo`). That
  one mixed track goes to the sound card, and its clock drives the picture, so a comp's
  video and audio stay locked exactly like a single clip's. The mixing happens on a
  background thread so pressing Space never stalls; a silent comp just plays on a plain
  timer instead. This retires the old stopgap where comp playback guessed the time from a
  wall clock.
- **Beat detection** (`kiriko-audio::beat`) — the groundwork for cutting to the music. It
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
- **Markers** (`kiriko-core::markers`) — a marker is a labelled flag at a moment on a
  composition's timeline. Three kinds: ones you place (User), chapter divisions, and the
  Beat markers Kiriko detects from the music (each with a confidence). Re-running beat
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
  made them. It's built by `waveform_peaks` (in `kiriko-audio::mix`), which buckets the mono
  mixdown into (min, max) pairs — a pure, tested down-sample — computed once when the comp's
  audio is mixed for playback.
- The **graph editor** (tabbed with the Timeline) — click a layer, and its animated
  properties draw as live curves: drag the keyframes (value and time together, one
  undo per drag), double-click the background to add a key, right-click a key for a menu
  (Easy ease / Linear / Hold, or Delete). Each key's shape tells you its interpolation at a
  glance — a diamond is linear, a circle is eased (bezier), a square is a hold.
  The curve you see is sampled from the same evaluator that renders the comp, so what the
  graph shows is exactly what plays. There are two ways to look at any property: the **value**
  view (the raw number over time) and the **speed** view (its rate of change — the
  derivative). Both are editable, and they are the *same* data seen two ways (K-070): in the
  speed view you drag a key up or down to set how fast the value is moving at that moment,
  which is often the easier way to make motion feel right. Editing one view updates the other.
  A **retimed footage layer** also shows a **"Speed (Retime)"** entry here (K-075): its value
  view reads the source frame showing at each point as `HH:MM:SS:FF` timecode, its speed view
  reads playback speed per cent, and dragging a speed point in that lens authors a ramp — the
  Vegas gesture. (A "Vegas" tick makes the channel open to the per-cent view by default.)
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
  discard one axis" idea.) A selected sequence clip's **Speed %** is a full ramp — a start
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
  ruler with each layer's bar on its own *lane*). Drag a layer's bar body to slide it earlier
  or later in time (one undo per drag). Every drag in the lane area — moving a bar, trimming
  an edge, scrubbing the ruler — follows the cursor one-for-one at any zoom, and the small
  "magnetic" pull towards nearby markers stays the same ~6 px on screen however far in you
  are (both used to speed up with zoom, which felt like the timeline slipping out from under
  the mouse); a twirled-open layer's keyframe diamonds line up under its bar at any zoom
  too. Zoom the time ruler with **Alt + wheel** — it zooms
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
- **The window layout** (K-074) — the picture (the Viewer) fills the middle with nothing
  above it: no tab, no strip, just the image. Around it sit the other panels, each with a
  little title tab you can grab: Project and the effect panels on the left, scopes on the
  right, the Timeline along the bottom. Drag a panel's tab to move it somewhere else — beside
  another panel, stacked as tabs, above or below — so you can build the layout that suits you;
  drag the edge between two panels to resize. Each tabbed panel also has a small pop-out
  button (⇱) that lifts it out into its own separate window; close that window and the panel
  drops back where it was. The Viewer is the one panel with no tab, so it always stays put as
  the bare picture. Under the bonnet this uses a "tiling" layout engine that, unlike the
  docking library we tried first, is happy to leave the Viewer without a tab bar.
- The **Project panel** — AE-shaped (K-068): the selected item's details up top, the
  folder tree below, and drag-and-drop everywhere. Drag footage onto the Timeline or
  Viewer to make a layer; with no comp open yet, the composition dialogue appears
  already filled in from that footage. Solids are proper assets now — one "White solid"
  in the project can back fifty layers, and the first one you make creates a Solids
  folder that future solids follow even if you rename it or tuck it inside another
  folder (Kiriko remembers the folder itself, not its name). Compositions do the same
  with a Compositions folder. Multi-step creations like that land as a single undo
  step — a batch operation whose inverse is just the reversed inverses of its members.
- **Epochs (`kiriko-eval::epoch`)** — the cancellation mechanism the whole scheduler
  will stand on. Every scheduled job carries a ticket stamped with the number that was
  on the wall when it started; scrubbing or stopping turns the wall number over, and
  workers glance at the wall between small steps and quietly stop if their ticket is
  stale. Nothing is ever force-killed. A test proves a deliberately slow job stops
  within 15 milliseconds of the number changing.
- **The frame scheduler's brain (`kiriko-eval::schedule`)** — the decision rules for
  smooth playback, written as plain arithmetic so tests can prove them. During playback
  Kiriko renders frames ahead of the playhead onto a small shelf; each screen refresh
  takes the newest shelf frame whose time has come, quietly binning ones the clock has
  passed, and simply holds the last picture if rendering falls behind (sound never
  waits). How far ahead to render adapts to how slow frames have actually been, between
  8 and 16 frames. And in realtime mode, frames too slow for the frame budget drop to a
  coarser preview resolution within a frame or two, earning it back only after a
  sustained cheap stretch — quick to worsen, slow to improve, so the picture never
  flickers between qualities. None of the real machinery (threads, the audio clock, the
  GPU) lives here yet; this is the referee, and the players arrive later.
- **Preview resolution never changes where things are.** To keep the picture responsive,
  Kiriko can decode footage smaller than its true size — and "Auto" resolution decodes at
  exactly the size the layer is shown on screen, so it gets sharper as you zoom in. That is
  purely a quality choice: a layer's *position and size in the composition* are always
  worked out from the footage's real pixel dimensions, not the shrunk-down preview copy. If
  they were ever worked out from the preview copy, a layer would appear to grow as you
  zoomed in — which is exactly the bug this rule exists to prevent.
- **Scrubbing shows a draft instantly, then sharpens.** While you drag the playhead (on the
  timeline ruler or the footage scrub bar), Kiriko decodes a small, quick version of each
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
  Kiriko keeps the last decoded frame and simply re-arranges it with your in-progress value each
  tick, no re-decoding. The moment you let go, the edit is committed as a single undo step and
  the frame re-renders normally.
- **Idle time is spent pre-caching nearby frames.** When you stop on a frame and aren't
  playing or dragging, Kiriko quietly renders the frames around the playhead into the cache
  at your chosen resolution, so stepping or scrubbing to them is instant instead of waiting
  each time. It works outwards from the playhead but favours the frames *ahead* — roughly
  three ahead for every one behind — because that's usually where you're going next. It fills
  one frame at a time and any real request (a scrub, an edit) immediately takes priority.
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
- **Blend modes** — the full everyday set: Normal, Add, Multiply, Screen, Overlay,
  Soft light, Hard light, Lighten, Darken. Two families under the hood: Add and
  Multiply are physical light maths and run in linear; Screen, Overlay and the lights
  are the Photoshop-era formulas people know by eye, so Kiriko runs them on encoded
  values (running them in linear is tidier maths and the wrong look). Lighten and
  Darken are a simple per-channel max/min where the distinction doesn't matter. Every
  mode is pinned to its textbook formula by a GPU test.
- **Colour depth, in one paragraph.** Kiriko's frames are "half float" (fp16) in linear
  light. Unlike AE's 16bpc — which is integer maths that clips at 1.0 — half float
  keeps brightness above 1.0 (a glow can genuinely overshoot) and negatives, which is
  what people switch AE to 32bpc for. Depth is one project-wide switch (8 / 16 float /
  32 float — K-069): flip it and every comp and effect in the project renders at that
  depth, AE-style, via a small button at the foot of the Project panel. Full float
  doubles every frame's memory and roughly halves compositing throughput, so 16-float
  stays the default; the heavy maths inside effects can run wider internally either way.
- `crates/kiriko-ui/src/theme.rs` — **the Aizome tokens.** The only file allowed to contain
  colour values. Change a colour here, it changes everywhere.
- `crates/kiriko-ui/src/icons.rs` — **the toolbar glyphs, drawn not downloaded.** Little
  pictures like the play triangle or the padlock aren't image files or a special font;
  Kiriko *draws* each one from a few lines and curves every frame (design rule §5: flat
  single-colour strokes, no emoji). The upside is they stay crisp at any size and always take
  the theme colour — so they dim on hover and go accent-orange when the tool is active, just
  like everything else. To add one, add a name to the `Icon` list and a small recipe of
  points in `paint`.
- `crates/kiriko-ui/src/shell.rs` + `app_state.rs` — **the window**: panels, menus,
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
| `async` | Not used in Kiriko's engine — we use threads and channels instead, deliberately |

When you hit something not covered here, ask any session "explain X in GUIDE.md terms and
add it to the guide" — that's the standing arrangement.

## 8. Building and running it on your machine

To turn the source into a running app you need the Rust toolchain and one outside
dependency: **FFmpeg**, the library that actually decodes and encodes video and audio.
Kiriko doesn't reinvent that wheel; `kiriko-media` talks to FFmpeg. So the build needs
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
   under your user folder, e.g. `C:\Users\you\ffmpeg\`. (GPL because Kiriko is GPL; "shared"
   because we want the `.dll` files.)
2. Install LLVM 18 and the Rust toolchain: `winget install LLVM.LLVM --version 18.1.8` and
   `winget install Rustlang.Rustup`. Rust's default Windows setup links with Visual Studio's
   C++ build tools, so having Visual Studio (or the standalone Build Tools) installed matters.
3. From the repo root, run `. .\scripts\win-dev-env.ps1 -Persist`. That one script finds the
   FFmpeg folder and LLVM, points the build at them, and (`-Persist`) remembers the settings
   so every future terminal already knows. The leading dot is required — it means "apply
   these to my current shell", not "run and forget".
4. Now the normal commands work: `cargo run -p kiriko-app` to launch, `cargo test --workspace`
   to run the whole test suite.

### On macOS

FFmpeg comes from Homebrew: `brew install ffmpeg@7`. The repo's `.cargo/config.toml` already
points the build at it, and macOS ships the translator (libclang) as part of its developer
tools, so there's nothing else to set up — `cargo test --workspace` just works.

### What the robots check

Every push, CI rebuilds and retests everything on both macOS and Windows, media included, so
"it builds on my machine" can never quietly drift from "it builds for real". The Windows
recipe above is exactly what CI does, written out by hand in `.github/workflows/ci.yml`.
