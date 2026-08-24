# Kiriko decision log

**Status: canonical.** Numbered, append-only. Every entry is either **DECIDED** (locked by
the project owner) or **PROPOSED** (a strong default chosen during the July 2026 design sessions; veto by
editing the entry and noting why). Reversing a DECIDED entry requires a new entry that
supersedes it — never edit history.

**How to use this log:** it is a long reference, not a start-of-task read. Don't read it end to end - search it for the entries relevant to your task (by topic keyword, or by the `k-###` numbers the relevant spec cites) and read those. Where two entries conflict, the later one that says it supersedes the earlier wins.

Format: ID · status · decision · rationale · consequences.

---

## Product

**K-001 · DECIDED · Kiriko is a native Windows application, developed cross-platform.**
Ships and is optimised for Windows; the Rust/wgpu stack (K-010) means the app also runs on
macOS during development so the window can be watched while building. macOS/Linux releases are
a possibility, never a priority.

**K-002 · DECIDED · Primary audience: flow / MVM-style gaming editors first; full AE
replacement over time.** Clarified 2026-07-12: the target lane is the smooth, cinematic
style (the CoD movie-making "MVM" lineage and today's flow style — the project owner's own
lane, per editors like stooh and starkerr), not classic kill-montage editing. This style is
compositing and animation as much as cutting. v1 milestone: a flow-style edit can be
completed start-to-finish in Kiriko (import high-fps captures, cut against the music with
beat markers, speed ramping with optical-flow slow motion, a smooth 2.5D camera move, a
masked transition, shake/glow/motion-blur/grade, export for YouTube). Long-term: Kiriko's
own version of everything After Effects has. Consequence: graph-editor ergonomics, masking,
and a basic camera join the v1 path ([16-ROADMAP.md](16-ROADMAP.md)); the effect staples of
K-064 are unchanged. Roadmap gates are phrased as "can a flow-style editor do X yet".

**K-003 · DECIDED · Licence: GPLv3.** Community contributions welcome; forks must stay open;
official binaries may still be sold later. LICENSE file at repo root.

**K-004 · DECIDED · Dark-first Aizome design.** Kiriko uses a dark-native variant of the
household Aizome design language: near-neutral dark panels (colour-grading accuracy), clay as
the single accent, hairline borders, household type stack. Recorded as a deliberate deviation
from the paper-light household default. Light mode is documented as a later option.
Spec: [15-DESIGN.md](15-DESIGN.md).

**K-005 · PROPOSED · Voice: en-GB, sentence case, calm, no exclamation marks** — in docs and
UI copy, per the household mandate. UI strings go through an i18n table from day one so this
is cheap to revisit.

**K-006 · DECIDED · Migration-aware first run.** On first launch, one skippable screen asks
which tools the user comes from (Vegas for ramps+effects / Vegas ramps + AE effects / AE for
both / neither) and tunes defaults accordingly — chiefly the Retime graph lens (speed vs
value), keymap preset offer, and which mapping tips show. One screen only, re-runnable from
the command palette, every setting individually changeable. Added 2026-07-12 at the owner's
request; post-v1 polish. Spec: [07-UI-SPEC.md](07-UI-SPEC.md) §13.1.

## Core model

**K-007 · DECIDED · Docs stay owner-readable; regression coverage is near-full.** All
documentation must remain understandable to the project owner (expert editor, new to Rust
and systems concepts): [GUIDE.md](GUIDE.md) is the plain-English companion, updated in the
same commit as any new concept. Testing policy: every feature ships with tests, every bug
fix ships with a regression test, CI enforces fmt/clippy/tests on macOS + Windows plus an
engine-crate coverage gate whose threshold may rise but never fall, and a design-token
lint. Added 2026-07-13 at the owner's request. Spec: [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md).

**K-008 · DECIDED · Brand mark and boot splash.** The mark is an Edo-kiriko faceted glass
hexagon whose clay facets form a K (assets/brand/; construction and colour constants in
[15-DESIGN.md](15-DESIGN.md) §brand). Boot shows a small centred splash listing each module
and effect as it initialises (the boot log — real registry plumbing that grows with the
effect suite and OFX scanning), minimum ~1 s dwell, failure lines in kraft. Added
2026-07-13 at the owner's request.

**K-020 · DECIDED · Layer-based model with a Sequence layer type.** Ordinary layers stay 1:1
with a source, as in AE. A dedicated **Sequence layer** holds clips cut back-to-back on one
row — the Vegas-style surface. This was chosen over (a) making every layer multi-clip and
(b) a Resolve-style dual-mode timeline.

**K-021 · DECIDED · One retiming system ("Retime") with two graph views.** Stored as retime
segments per clip (Sequence layers) or per layer (Footage layers); edited through the value
graph (AE-style) or the speed graph (Vegas-style semantics, drawn in the graph editor below
the value view — never overlaid on the clip like Vegas). Spec: [04-RETIMING.md](04-RETIMING.md).

**K-022 · DECIDED · Retime edits never move clip boundaries ("the beat-sync covenant").**
When a retime runs out of source media, Kiriko holds the boundary frame and draws an explicit
overrun indicator; an explicit "trim to source end" command exists. No auto-ripple, ever.

**K-023 · DECIDED · 2.5D now, deeper 3D later.** v1 core: 3D layer transforms, cameras,
depth-of-field, basic lights (AE-style 2.5D). All transform maths is 4×4 from day one. The
long-term ambition (working "directly in 3D", importing Blender scenes) is tracked in the
roadmap as a post-parity phase; nothing in the core data model may preclude it.

**K-024 · DECIDED · Non-destructive always.** Nothing the user does modifies source media or
bakes irreversibly into the project. Baking/flattening exists only inside the export pipeline
(and internal caches), invisible to the project document.

**K-025 · PROPOSED · Keyframe maths is AE-compatible.** Bezier keyframes carry per-side speed
(units/sec) and influence (0.1–100%), hold and linear modes, spatial beziers with roving
keyframes. Rationale: lossless AE import (K-060) and zero relearning for the target audience.

**K-026 · PROPOSED · Per-comp colour bit depth (fp16 default, fp32 opt-in)** rather than AE's
project-global bit depth. Working space is scene-linear, premultiplied alpha.

## Architecture

**K-010 · DECIDED · Language: Rust.** Memory/thread safety is the best structural defence for
the never-crash requirement; ecosystem proven by Rerun, Gyroflow, Cap. C ABI interop covers
ffmpeg, OFX, CUDA.

**K-011 · DECIDED · GPU: wgpu** (DX12 backend on Windows, Metal on macOS). First-party
effects written in WGSL compute so NVIDIA and AMD both get acceleration without vendor lock.

**K-012 · DECIDED · UI: egui** (+ egui_dock/egui_tiles, winit, AccessKit), Rerun-style: a
custom wgpu renderer for the Viewer inside an egui panel shell. Known risk: text polish and
timeline-scale widget performance; the crate split must keep the UI layer swappable
(escape hatches: GPUI, Qt shell).

**K-013 · PROPOSED · Media I/O: ffmpeg via rsmpeg**; hardware decode via D3D11/12VA (and
VideoToolbox on the dev Mac) with one GPU→GPU copy into wgpu at v1; NVENC/AMF/QSV encode via
ffmpeg. Audio: cpal, audio-clock-master sync.

**K-014 · PROPOSED · CUDA is an optional per-node accelerator, not a pipeline.** The one
portable compute path is WGSL/DX12. CUDA (via cudarc + Vulkan interop) may accelerate specific
heavy nodes (optical flow) where measured wins justify it. Never a hard requirement.

**K-015 · PROPOSED · Layers in the UI, DAG underneath.** The layer stack compiles to an
immutable, content-hashed evaluation graph; Nuke-style split of a cheap metadata pass from a
cancellable pixel pass. Spec: [05-ARCHITECTURE.md](05-ARCHITECTURE.md),
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md).

**K-016 · PROPOSED · Three-tier content-hash cache** (VRAM → RAM → disk), keyed by
hash(node id+version, params, time, quality, input hashes) — never by timeline position.
Idle-time background rendering fills the timeline cache bar.

**K-017 · PROPOSED · The UI thread never evaluates anything.** Work-stealing job pool,
dedicated decode/IO/audio/GPU-submit threads, epoch-based cancellation on scrub,
latest-wins progressive previews.

**K-018 · PROPOSED · Degrade, never crash.** A central resource governor with an explicit
degradation ladder (pause background render → evict cache → drop preview res → tile → CPU
fallback); GPU device-loss is treated as routine and recovered; operation-journal autosave.
Spec: [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md).

**K-019 · PROPOSED · Minimum spec: Windows 10 20H2+, any DX12-capable GPU, 16 GB RAM
recommended.** CPU-only operation must work (slowly) for every built-in effect: each WGSL
effect ships a CPU reference implementation, which doubles as its test oracle.

**K-033 · DECIDED · Metal/macOS is a supported future target, already carried by the
architecture.** The wgpu pipeline (K-011) compiles WGSL to Metal today — macOS builds run
the full compositing path natively on Apple GPUs with no separate render backend. A proper
Mac *release* (post-v1, demand-driven; refines K-001's "possibility, never a priority")
additionally needs: VideoToolbox hardware decode/encode promoted from dev-convenience to
first-class (zero-copy via IOSurface, [impl/media-io.md](impl/media-io.md) §4), ProRes
workflows (Mac editors' mezzanine norm), the Metal branch of the OFX 1.5 GPU render suite
([12-PLUGINS.md](12-PLUGINS.md) §2.4), and a notarised universal binary. Nothing in the
engine may assume DX12-only. Added 2026-07-13 at the owner's request.

**K-035 · DECIDED · Every effect gets a built-in strength matte.** Any effect instance can
select a per-pixel strength source — the layer's own masks or any other layer (same
dropdown model as layer mattes) — scaling the effect's influence at each pixel. The host
implements it once, uniformly: for colour-type effects as a per-pixel mix between input
and effected image; for warp/distort-type effects by scaling the displacement field where
the effect declares vector output (falling back to output-mix otherwise). No effect
author writes masking code; it composes with everything. AE needs per-effect "composite
on original"/precomp workarounds for this. Lands with the effect suite (phase 3). Added
2026-07-13 at the owner's request. Spec: [08-EFFECTS.md](08-EFFECTS.md) §effect model.

**K-036 · DECIDED · A node view is a planned lens over the evaluation graph.** Kiriko's
layer stack already compiles to a DAG (K-015), so a Nuke-style node editor is a *view*,
not a second engine: post-parity (phase 6 alongside the 3D ambitions), Kiriko exposes the
graph for node-based compositing, starting where nodes earn their keep first — a
Resolve-style grading node chain in the Colour workspace. Layers and nodes stay two lenses
on one document; neither is a mode you convert into. Added 2026-07-13 at the owner's request.

**K-037 · DECIDED · Share export: size-targeted clips for the community workflow.**
Editors share previews (usually Discord, 50 MB free-tier cap): a one-click export mode
takes the active playback area (work area; whole comp until it exists), computes the
bitrate from the size budget ((target bytes × 8 ÷ duration) less audio/container
overhead), optionally caps resolution, and writes a compressed H.264 clip. Presets:
Discord 50 MB (default), 10 MB, custom size, plus a quality-first slider for people who
prefer choosing compression over size. Added 2026-07-13 at the owner's request. Spec:
export sections of [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md)/[07-UI-SPEC.md](07-UI-SPEC.md).

**K-034 · DECIDED · Perceptual colour operations happen in Oklab.** Two colour domains,
each doing the job it is correct for: **linear RGB** remains the compositing/working space
(light adds physically there — blending, exposure, glow are correct and stay put), while
**interpolation and hue-type operations** — gradient ramps, colour-property keyframe
interpolation, hue rotation, saturation adjustments — convert through **Oklab/OkLCh** so
gradients between two colours stay colourful instead of collapsing to grey, and altering
hue genuinely preserves perceived lightness. Users interact in ordinary RGB throughout;
conversion is engine-internal and cheap (two 3×3 matrices + three cube roots per
direction, identical constants in the Rust CPU reference and the WGSL snippet, guarded by
round-trip and hue-invariance tests). Effects declare which domain each parameter's maths
runs in ([08-EFFECTS.md](08-EFFECTS.md)). Added 2026-07-13 at the owner's request. Spec:
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §3.

**K-031 · DECIDED · Colour spaces are selectable; preview always matches export.** Working
colour space is selectable per comp (with app-level defaults, and OCIO joining post-v1 per
06), like AE — but with a hard parity guarantee: **what the Viewer shows at Full resolution
and full quality is bit-identical to what export produces** through the same transforms.
Export-only settings (encoder, bitrate, container, subsampling to 8/10-bit) sit strictly
after the parity point. Adaptive degradation and Realtime mode affect interaction only and
are always visibly indicated. Added 2026-07-12 at the owner's request. Spec:
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §3.

**K-032 · DECIDED · Resource and export controls are explicit settings.** RAM/VRAM budgets,
CUDA on/off, decoder pool, worker caps, cache root/size in Settings → Performance/Cache;
export dialogue exposes full custom controls (resolution, frame rate, format, codec,
encoder choice, rate control, audio, thread count and a background/balanced/fast priority)
alongside presets — and exporting never blocks editing (06 §7.1). Added 2026-07-12 at
the owner's request. Spec: [07-UI-SPEC.md](07-UI-SPEC.md) §Settings inventory.

**K-030 · DECIDED · Two preview modes: Cached (default) and Realtime-adaptive.** Cached
plays at full chosen quality from the render-ahead ring and cache. Realtime never waits:
every frame renders live at whatever resolution tier sustains the comp frame rate, adjusted
continuously with hysteresis — judge motion now at reduced resolution rather than full
quality after a wait. Clarified same day: the mode toggle is a **separate control** from
the Viewer bar's resolution picker (Full/Half/Third/Quarter/Auto) — it lives in the
transport and Settings → Preview, never in the resolution dropdown, and Cached always
honours the picked resolution. Added 2026-07-12 at the owner's request. Spec:
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §6.5.

## Persistence

**K-040 · DECIDED · Project file: hybrid container.** A single `.kir` file — a zip holding
a human-readable, versioned `project.json` plus small embedded assets (thumbnails, curve
data). Footage referenced by path with relink logic. Caches, proxies, and exports live in a
sidecar folder, deletable at any time. Autosave is journalled. Spec:
[10-FILE-FORMAT.md](10-FILE-FORMAT.md).

## Audio

**K-050 · DECIDED · v1 audio is a sync toolkit; the Composer comes later.** v1: import,
sample-accurate playback, timeline waveforms, manual + automatic beat markers, volume
keyframes, mute/solo, multiple audio layers per comp. Later: the **Composer** workspace —
sound design against the edit inside Kiriko (multiple sounds per layer, so editors stop
round-tripping to Vegas for audio). Spec: [09-AUDIO.md](09-AUDIO.md).

## Extensibility and interop

**K-060 · DECIDED · AE project import via an exporter panel, parser as best-effort backup.**
Primary: a free ExtendScript/CEP panel running inside After Effects that walks the scripting
DOM and emits Kiriko-schema JSON (comps, layers, transforms, keyframes with bezier params,
masks, mattes, retime, expression text, effect match-names). Secondary: best-effort direct
`.aep` (RIFX) parsing, structure only, no fidelity promises. Third-party AE effect internals
never map; they import as inert placeholders. Spec: [11-AE-IMPORT.md](11-AE-IMPORT.md).

**K-061 · PROPOSED · Kiriko is an OFX host.** OpenFX is BSD-3/open; Twixtor, RSMB, Sapphire
ship OFX builds already proven in Vegas/Resolve. This is the legal, practical route to the
gaming-edit plugin staples. Native `.aex` AE plugins will never load (technically and legally
infeasible — see research).

**K-062 · PROPOSED · Native plugin API "KFX": CLAP-shaped.** Stable C ABI core + versioned
typed extensions, host-owned animated parameters, out-of-process sandboxed execution with
shared-memory/shared-texture frames, MIT-licensed headers + a validator tool. Plugins ship
after the main application, but every engine interface is designed against KFX from day one.

**K-063 · PROPOSED · Expressions: JavaScript on QuickJS-ng**, exposing the AE expression
surface (`wiggle`, `loopOut`, `valueAtTime`, `time`, `seedRandom`, …) at ES2018 level, fully
deterministic (seeded random, no Date/IO/JIT variance) so distributed/export renders agree.

**K-064 · PROPOSED · Built-in effect suite covers the montage staples in-box** — optical-flow
retiming (Twixtor-class), optical-flow motion blur (RSMB-class), exposure-aware glow
(Deep Glow-class), parameterised camera shake, smooth-zoom presets, RGB split, flash/strobe,
colour grading with preset browser — so a new editor needs zero third-party plugins for the
core genre look. Spec: [08-EFFECTS.md](08-EFFECTS.md).

**K-065 · PROPOSED · Preset and project sharing is a first-class feature** (import/export of
presets and template projects), because shared project files and CC packs are how the montage
scene onboards. Nothing in the file format may make shared projects machine-specific.

**K-066 · DECIDED · Every plugin supports every colour depth and multi-frame rendering.**
KFX plugins MUST process fp16 and fp32 correctly (validator-enforced at both depths) and
MUST tolerate frames rendering in parallel, out of order, on any thread — the host renders
frame-parallel by default through instance pooling, and `kfx.thread-unsafe` is the sole,
discouraged opt-out. **The host owns the optimisation strategy**: instance counts and frame
scheduling are decided from declared traits plus measured cost under the governor's
budgets, exactly as for built-in nodes. OFX plugins are scheduled per their declared
render-thread-safety, with the host converting depth at the boundary. Added 2026-07-12 at
the owner's request. Spec: [12-PLUGINS.md](12-PLUGINS.md) §2.3, §3.3–3.4.

**K-067 · DECIDED · The engine's pillars carry Edo-kiriko craft names.** The render
pipeline as a whole — evaluation graph, GPU compositor, colour engine — is **Togi**
(研ぎ, the polishing stage that turns cut glass brilliant: it turns the project's cuts
into the picture). The three-tier cache is **Kura** (蔵, the storehouse). The audio
engine and master clock is **Hibiki** (響, resonance — everything syncs to it). The
names appear in user-facing surfaces (boot splash, settings, docs, marketing); crate
names stay `kiriko-*` and code identifiers stay plain English per the glossary. Future
subsystem names come from the same craft vocabulary and are logged here. Added
2026-07-13 at the owner's request.

**K-068 · DECIDED · AE-style Project panel with auto-filing and the composition
dialogue.** The Project panel is info-header-plus-tree: the selected item's details at
the top, the folder tree below, and everything moves by drag and drop — rows drag onto
folders to file them, onto the Timeline or Viewer to become layers (the "Add to comp"
buttons are gone). Solids are assets (`SolidDef`, per 03-DATA-MODEL §2): the first solid
creates a "Solids" folder and later ones follow it *by id* — renaming or nesting the
folder keeps the habit; deleting it just recreates it on next use. Compositions auto-file
the same way into "Compositions". Manual comp creation always shows the settings dialogue
(name, size, frame rate, duration); dropping footage with no comp open shows it
pre-filled from that footage; comps created implicitly inside an active comp (future
precompose) inherit the parent's settings silently; settings stay editable later
(Composition settings…, one invertible op). Multi-step creations commit as one
`Op::Batch` — one undo step. Added 2026-07-13 at the owner's request.

**K-069 · DECIDED · Working depth is one project-wide switch.** Supersedes the
per-comp fp32 opt-in in K-026. The project renders everything — comps, effects,
inter-node buffers — at a single depth: 8 bpc integer, 16 bpc float (default), or
32 bpc float. No per-comp override; switching the project switches everything (the AE
project-bit-depth model, which editors already understand). The control is a small
depth button at the foot of the Project panel; Application Settings holds only the
default for new projects. Kernel-internal accumulators may exceed the project depth
where the algorithm needs it, but node inputs/outputs never do. Depth remains part of
the cache key's quality field. Implementation lands with the depth-aware pipeline work
in the effects phase; until then 16 bpc float is the only rendering depth. Decided
2026-07-13 at the owner's request. Spec: [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §3.1.

**K-070 · DECIDED · The graph editor is a general derivative-lens editor, in the
Timeline.** Three points from the owner (2026-07-13):

1. **Derivative lenses for every animatable property.** The value/speed views of §5.1
   generalise: any property (transform, effect parameter, mask, retime) can be viewed and
   edited as its **value**, its **speed** (first derivative), or its **acceleration**
   (second derivative) — the distance/velocity/acceleration analogy. Acceleration joins
   value and speed as a first-class lens (extends [07-UI-SPEC.md](07-UI-SPEC.md) §5.1). All
   three are views of the one keyframe/segment store; editing any of them round-trips
   losslessly. The lens-switch controls are **glyphs in the bottom-right of the graph
   editor** (alongside the ease-preset footer of §5.3). Retime's value/speed lenses (§5.2,
   [04-RETIMING.md](04-RETIMING.md) §9) are the retime-specific instance of this system.

2. **The graph editor lives in the Timeline area, not a separate panel** — a mode of the
   Timeline lane area with a header toggle, exactly as [07-UI-SPEC.md](07-UI-SPEC.md) §5
   already specifies. Kiriko's current implementation as a standalone dock tab
   (`Panel::GraphEditor`) is a temporary divergence to be corrected when the lens work
   lands.

3. **Frame-pinning invariant for Vegas-style speed edits (binding).** Changing a segment's
   speed pins the source position at the segment's **start** and ripples the change
   **downstream only** (the §4.1 boundary-consistency recompute already encodes this: sᵢ is
   fixed, sᵢ₊₁… are recomputed). Consequently a clip's first frame is always its own
   trim-in whatever its speed, so splitting a clip and re-speeding the second half never
   moves where it starts — and this holds after the layer's start/in-point is later
   adjusted, because `place` is layer-time and the retime domain is unchanged. Locked by
   `kiriko-core::sequence` tests (`re_speeding_a_cut_clip_keeps_its_start_frame`).

**K-071 · DECIDED · The sequenced layer is single-source, order-preserving, edited in its
own timeline tab.** Refines the Sequence layer (K-020) per the owner (2026-07-13):

- You **convert an imported-footage layer** into a *sequenced layer* (name pending — only
  footage sources qualify). It opens in its **own, visually distinct timeline tab** showing
  a **single row: that one source**. In the parent comp it reads as one layer — **a fancy
  precomp**: comp-level transform/effects/masks apply to its assembled output, and the
  layer's length **tracks the end of the assembled sequence** (the last piece's end).
  Opening it swaps the Timeline into a distinct single-source editing view (a new window/
  tab with a slightly different UI).
- **Single source only, for now.** Every clip in a sequenced layer references the same
  footage item. The general multi-source Vegas assembly (K-020's broader reading) is
  **deferred** and may return.
- **Operations**: cut, delete (with **gaps allowed** — a gap renders transparent), and
  **retime per piece**. **No reordering / "no mixing footage time":** reading the pieces
  left to right, source time never jumps backwards (`source_in` is non-decreasing by
  timeline position). You remove and space pieces; you do not shuffle them.
- **Why the order constraint**: it keeps comp-time -> source-time a clean forward mapping, so
  a **camera tracker** (its own tool, not an effect) can run once on the **full, unaltered,
  un-retimed** footage, and its track then **replays through the cuts and retimes** in the
  comp, linked to the layer. The clip-resolution model (`kiriko-core::sequence`) is exactly
  that mapping. If a track is linked, the ordering restriction may later be relaxed.
- **Invariants (binding for now)**: single source (`sequence::single_source`), source-ordered
  (`sequence::is_source_ordered`), gaps allowed, and the K-070 frame-pinning rule per clip.
- Note: the inline razor shipped this session operates on the general model in the main
  timeline; the dedicated-tab editing surface is the intended home and supersedes it.

**K-072 · DECIDED · Transform property rows: keyframable speed and linked scale.** Detail
for the property-row timeline restructure (07-UI-SPEC §5, K-070), from the owner (2026-07-13):

- **Speed is a keyframable property like any other**, in the regular (layer) timeline view
  as well as the graph view. The Speed row gets a stopwatch; keyframing it builds the
  retime's speed lens (Rate segments between speed keyframes), and its keyframes show as
  glyphs on its own row. A single un-keyframed value stays the constant-speed case.
- **Scale x / y share one row by default, with a ratio lock (default on)** — like the
  composition Size field. Linked: one Scale control edits both, preserving the x:y ratio.
  Unlocking lets you edit x and y separately and **splits them onto two rows**; a relink
  button stays available. **Relinking collapses to a single row and keeps one axis, losing
  the other's independent changes — unless one axis was never changed**, in which case the
  two merge losslessly and keep the ratio.
- Both land with the property-row restructure (each animatable property as its own timeline
  row: left column stopwatch + name + value, track shows its keyframes; clicking a row
  graphs that property). Keyframe-interpolation glyphs (bezier/linear/hold) on each key are
  a later refinement; the near-term requirement is that keyframes are *shown where set*, on
  the property's row rather than the layer bar.
- **Implemented 2026-07-14.** Per-property timeline rows, the Scale ratio lock, and
  keyframable speed (via `Retime::from_speed_keyframes`/`speed_keyframes`) all shipped. Two
  deliberate deviations from the above, both easy to revisit: (a) relinking scale keeps the
  current ratio and loses nothing, rather than discarding an axis — the combined row can
  represent any ratio, so the lossy rule was unnecessary; (b) keyframe-interpolation glyphs
  and live-preview while dragging a *speed* key are still outstanding (speed edits re-decode
  on commit). Clicking a transform property's name graphs it.
- **Speed-lens editing, increment 1 (2026-07-14):** the graph editor's Speed view is now
  editable for transform properties (K-070) — dragging a key's handle sets its bezier tangent
  (both sides), the derivative curve updates live, and the release writes back to the
  keyframes; the derivative you set is the derivative you read back (round-trip test in
  `kiriko-ui`). Still to come (increment 2): Retime wired as its own graph channel whose value
  lens reads the resolved source position as timecode and whose derivative lens reads speed %,
  with a Vegas-editor setting choosing the default lens (K-021).

**K-073 · DECIDED · v1 shell is a fixed native-panel layout, not a dock.** The Viewer is a
bare, full-bleed central area with **no tab bar**; the Project/effects panel (left), Scopes
(right) and Timeline (bottom) are resizable native panels around it. Chosen 2026-07-13 at
the owner's insistence that the viewport carry no "top bit": egui_dock (0.16) draws a tab bar on
every leaf and offers no per-leaf toggle, so the only way to give the Viewer a bare frame was
to leave the docking system. Consequences: egui_dock is dropped as a dependency; drag-to-dock,
tab rearrangement across regions, and floating panels are gone for now; the left panel keeps a
small Project / Effect controls / Effects & presets tab switcher so nothing is lost. Pop-out
returns later as real OS windows (egui viewports), a cleaner pop-out than dock floats. This
supersedes the docking mandate in [07-UI-SPEC.md](07-UI-SPEC.md) §1 for v1, which now documents
the eventual target. The `kiriko-ui` crate must keep the UI layer swappable regardless (K-012).

**K-074 · DECIDED · Dockable tiling shell with a bare Viewer (supersedes K-073).** The
window is a single tiling layout (egui_tiles): every panel except the Viewer carries a
title tab and can be dragged to re-arrange the workspace; the **Viewer alone is a bare pane
with no tab bar** (the owner, 2026-07-14: the viewport must have no top bit). This reverses
K-073's "fixed native panels, no docking" — that was a stopgap taken because egui_dock draws
a tab bar on every leaf; egui_tiles doesn't force a tab on a lone pane, so the Viewer can be
bare *and* the other panels fully dockable. Mechanism: the Viewer is inserted as a direct
child of a linear container (never a tab group) with `all_panes_must_have_tabs = false`;
`prune_single_child_tabs = false` keeps single panels (Timeline, Scopes) showing their tab.
Default layout: an upper band — Project/effect-controls/effects-&-presets tab group (left),
the Viewer (centre), Scopes (right) — above a **full-width Timeline** tab group along the
bottom (the Edit workspace of [07-UI-SPEC.md](07-UI-SPEC.md) §3; the Timeline is a direct
child of the vertical root so it spans the whole window). Pop-out into a panel's **own OS
window** is
implemented: a tab's ⇱ button hides its tile in the dock (`Tiles::set_visible`) and renders
it in an egui immediate viewport; closing that window docks it back. Supersedes the v1-status
note in [07-UI-SPEC.md](07-UI-SPEC.md) §1; keeps the UI layer swappable (K-012).

**K-075 · DECIDED · Retime is a graph-editor channel (footage layers): frame-timecode value
lens, speed-% derivative lens, Vegas default-lens setting; sequence-layer retiming lives in
the sequence view.** Confirmed by the owner (2026-07-14), building on K-021, K-070, K-071, K-072:

- **Footage layers — Retime graphs like any other channel.** A retimed footage layer exposes
  its Retime in the graph editor's left column beside the transform properties, using the same
  two-lens machinery (K-070). The value and derivative lenses are two views of the **one**
  retime store — the segment model of [04-RETIMING.md](04-RETIMING.md) stands; nothing is
  re-stored as keyframes.
  - **Value lens = source position as frame timecode** (`HH:MM:SS:FF` in the footage's own
    timebase) — "which source frame is showing here" — not seconds or a percentage.
    **Derivative lens = speed per cent** (Vegas-style). Editing either writes retime segments
    ([04-RETIMING.md](04-RETIMING.md) §9); switching lenses never converts data.
  - **A Vegas-editor preference picks the default lens.** On → the Speed channel opens to the
    per-cent (derivative) lens; off → the frame-timecode (value) lens. This generalises
    K-021's "opens the speed graph by default" into a user preference.
- **Sequence layers do NOT get an editable Speed channel.** Their retiming is done *inside*
  the sequenced-layer view (K-071): the view shows the single source as a layer you
  cut/splice/move, with an **optional graph pane below it** — the layer stays visible on top,
  so cutting/splicing continues while retiming, and the graph (the regular graph view)
  reflects the sequence's retime, respecting the gaps between pieces. Documented here;
  **implemented later** (a good candidate for a focused `fable` session, per the owner).
- **Increments:** *2a* (now) — footage Retime graphable, both lenses + the setting + the
  correct default lens; *2b* — the full [04-RETIMING.md](04-RETIMING.md) §9.2 in-graph segment
  editing (RateSegment endpoint drags, compensating edits, Rate↔Map conversions); *2c* (later)
  — the sequence-view graph pane.

**K-076 · DECIDED · The Retime graph channel is named by its lens: Time (value) and Velocity
(speed).** Confirmed by the owner (2026-07-14), refining K-075. The Retime channel — its outline
row and its graph — reads **Time** in the value lens (source position, "which frame is
showing") and **Velocity** in the derivative lens (the Vegas velocity-envelope heritage the
speed graph already invokes). This **reverses the glossary §9 "velocity → speed" ban for this
one UI label**: "speed" remains the term for the quantity everywhere else (percentages,
RateSegment speeds, prose, identifiers); "velocity" is permitted solely as this channel's
derivative-lens label. The channel also behaves like any other property — it carries a
stopwatch/keyframe control in the outline — and its **default lens is the value (Time) lens**
(the Vegas-preference of K-075 defaults **off**), so the channel opens to Time.

**K-078 · DECIDED · The Time (value) lens is a fully bezier-keyframed property, identical to
any transform channel.** From the owner (2026-07-14), extending K-025/K-070/K-075/K-076. The Retime
**Time** lens is not a special read-only view: it is the ordinary graph editor — draggable
keys, gold tangent handles, F9 easy-ease, marquee, auto-fit — operating on source position over
local time, exactly like Position or Scale. This is realised by mapping each pair of value
keyframes to a **`MapSegment`** (the AE cubic already specified in K-025): a segment's control
handles are the left key's out-tangent and the right key's in-tangent, using the *same*
control-point construction as `anim::CubicSpan::from_ae`, so a Time curve renders **bit-for-bit**
like the same keys on a transform property (regression-tested). The bridge is
`Retime::from_source_keyframes` (keys → store) and `Retime::source_keyframes` (store → keys).
Consequences and limits, for now:
- A **Linear** side lies on the chord (influence ⅓), matching `anim::side_params`.
- A **Hold** side is treated as Linear — a stepped Time Remap (freeze-then-jump) is future work,
  since a single monotone `MapSegment` cannot express a step while keeping boundary C0 exact.
- A **`RateSegment`** (an eased speed-lens ramp, or the identity store) displays as a straight
  Linear side in the Time lens; dragging any handle there recommits the whole channel as
  `MapSegment`s, so the eased *speed* shaping is replaced by explicit *value* tangents. The two
  native vocabularies (Rate/Vegas vs Map/AE) still don't losslessly interconvert — editing in a
  lens commits in that lens's vocabulary, which is the K-070 model working as intended.
- Source positions round onto the flick grid on commit; local-time boundaries stay exact
  (keyframe times are rational), so the beat-sync covenant (§4/§7) is unaffected.
The "which lens a channel opens to" preference (K-076) stays; per-project lens customisation is
still deferred.

**K-079 · DECIDED · The graph editor pans and zooms; it shares the timeline's time axis and
auto-fits vertically by default.** From the owner (2026-07-15). The curve editor previously mapped x
over the whole comp duration and framed y purely by auto-fit, so neither axis scrolled. Now:
- **Horizontal** follows the shared lane axis (07-UI-SPEC §4): the same pixels-per-second and
  scrolled left edge as the layer bars, so **Alt-wheel** zooms and **Shift/horizontal-wheel**
  scrolls the curve in step with the lanes. (This resolves the standing "share the lanes' zoomed
  time axis" increment.) The value lens draws across the visible window for full resolution when
  zoomed; the Velocity lens keeps a whole-duration axis for now.
- **Vertical** auto-fits the whole curve by default (a bezier overshoot stays on screen). A plain
  wheel over the graph pans the value range and **Ctrl-wheel** zooms it about the cursor, taking
  over with a manual range (`graph_view_y`); a **Fit** button in the bottom bar restores auto-fit.
  The manual range resets when the lens or graphed channel changes. Applies to the value lens
  only.
- **Independent scrolling:** the graph fills the lane area with the layer outline to its left, so
  a wheel over the graph moves the graph while a wheel over the outline scrolls the layer list —
  achieved by zeroing only `smooth_scroll_delta` (which the outline's ScrollArea reads) over the
  graph, leaving `raw_scroll_delta` for the graph. The graph also gets its own vertical scrollbar
  on its right edge when a manual range doesn't cover the whole curve.
Not yet done: relocating the layer list's own built-in scrollbar onto the outline's edge (it
still sits at the far right); that needs a custom outline scrollbar and is deferred.

**K-080 · DECIDED · The speed lens draws the exact derivative of the value bezier.** From the owner
(2026-07-15). The speed (derivative) view sampled its curve by central finite difference at
half-frame steps — a display stopgap that could smear the shape near steep handles. It now uses
`anim::evaluate_speed`, the closed-form `dv/dt = y′(u)/x′(u)` of the value-lens cubic (with the
`x′` floor at a 100%-influence handle), so the speed curve is precisely the slope of what the
value lens draws: bezier easing in the value view shows as the matching smooth speed curve, a
straight span as a flat speed, a Hold as zero. This is the value/speed "two views of one data"
promise (K-070) made exact.

**K-081 · DECIDED · Tangent handles are draggable in the speed lens too.** From the owner
(2026-07-15). The speed (derivative) lens showed one draggable speed point per key; it now also
carries the same gold tangent handles as the value lens for a selected key, so a curve can be
eased from either view. In the speed graph a handle's **height is that side's speed** and its
**horizontal reach is its influence** (After Effects' speed-graph ease bars); dragging writes the
same `SideInterp::Bezier` store through `apply_tangent`, so the value and speed lenses stay in
lock-step. Clicking a speed key selects it (as in the value lens) to reveal its handles. The
value lens keeps the unified partner-length behaviour (K-072 refinement); the speed lens mirrors
a unified drag but keeps the partner's own reach (no screen-length preservation — the speed lens
is about the speeds themselves).

**K-082 · DECIDED · Linux is a supported build target.** From the owner (2026-07-16), after outside
requests to run Kiriko on Linux. Kiriko remains **Windows-first** (that ordering is unchanged);
Linux joins macOS as a supported desktop target: the build must work from a plain
`cargo build` given the platform's usual dependencies, and the README documents them. On Linux
FFmpeg resolves through pkg-config (the same `link_system_ffmpeg` path as macOS), which needs
the **FFmpeg 7.x development packages**, `pkg-config`, and `clang` (for the binding generator).
Known constraint: distributions still shipping FFmpeg 6 (e.g. Ubuntu 24.04 LTS) cannot build
without a newer FFmpeg; that is documented, not worked around. A Linux CI job joins the matrix
when a maintainer can verify it; until then Linux support is best-effort docs + upstream-standard
code (no platform-specific code paths exist today).

**K-083 · DECIDED · The application is named Luminal; subsystems are Nova, Nebula and Pulsar.**
From the owner (2026-07-16). Kiriko is renamed **Luminal** (the owner's handle; of light and of
thresholds) across the entire application: UI strings, all living docs, crate names
(`kiriko-*` → `luminal-*`), the project file extension (`.kir` → `.lum`, safe pre-release with
no files in the wild), the brand asset filenames, and the GitHub repository
(`luminalmvm/Kiriko` → `luminalmvm/Luminal`; old URLs redirect). The K-067 subsystem names are
reversed in the same stroke — the Edo-kiriko craft register no longer fits — and replaced with
an astral register: **Nova** (render pipeline, was Togi), **Nebula** (cache, was Kura),
**Pulsar** (audio engine and its clock, was Hibiki). Historical records (this log's earlier
entries, `docs/research/`) keep the old names verbatim; the hexagon cut-glass mark stays as an
approved placeholder pending a Luminal redesign (noted in 15-DESIGN). The design-language
overhaul that accompanies the rename (rerun-io-style look, colour scheme kept) is its own
follow-up decision.

**K-084 · DECIDED · The visual system adopts rerun.io's structure, keeping Luminal's colours.**
From the owner (2026-07-16), with the K-083 rename. The look moves from the Aizome dark
adaptation's mid-dark ramp to the structure of rerun.io's viewer (`re_ui`, studied at source):
a near-black canvas (`surface_0` `#0b0c0e`), panels one small step above it, floating surfaces
(menus, inputs, tab bars) a clear step up, **borderless widgets** whose idle/hover/pressed
states are fill steps rather than stroke changes, crisp 1 px hairline separations as the only
panel elevation, floats on a real soft shadow (offset 0/15, blur 50), 4 px control / 6 px
float radii, thin solid 6 px scrollbars, 14 px indents and a 16 px interact height. Deliberate
deviations from rerun: the item-spacing grid stays Luminal-dense (6×4, not 8×8 — the timeline's
row pitch is part of the app's feel), and every hue is Luminal's own (clay accent, the cool
grey ramp, the K-004 strictly-neutral Viewer surround, now `#121212`). The accent carries
selection, punchier than before (50% fill). Embedding Inter (rerun's UI face) is a pending
follow-up awaiting the owner's decision on shipping the font file. The owner also wants a
sleeker "liquid glass" alternative theme later; that is not this decision. The hexagon mark
redesign (noted at K-083) remains open.

**K-085 · DECIDED · Icons are the Iconoir set, embedded as an icon font via `iconflow`.**
From the owner (2026-07-16). Reverses 15-DESIGN §5's hand-drawn-only iconography (and its "no
icon font" clause): the hand-drawn glyphs are replaced wholesale by **Iconoir** (MIT), embedded
through the `iconflow` crate (MIT, `pack-iconoir` feature only) as a font whose glyphs render
like text — theme-coloured, resolution-independent. The change also retires every raw Unicode
symbol the UI hoped the fonts carried and didn't (the pop-out `⇱`, the keyframe navigators'
`◄ ◆ ►` — all rendered blank): those are proper icons now (`open-new-window`, `nav-arrow-*`,
`keyframe`/`keyframe-plus`). What stands from §5: monochrome only, theme-coloured, and the
emoji ban — a glyph is from the set or deliberately painter-drawn (track keyframe diamonds),
never a hoped-for font character. A CI test resolves every mapped name against the embedded
pack, so a typo'd icon name cannot ship.

**K-086 · DECIDED · Solo panels render bare; the Timeline pops out from its comp strip.**
From the owner (2026-07-16): the default workspace showed a needless "Timeline" dock tab above
the Timeline's own comp-tab strip, and the only way to lose it was popping the panel out and
back. Now a panel that sits alone in its tile renders with **no tab bar at all** — the bare
look K-074 reserved for the Viewer, extended to every solo pane — and a tab bar appears only
where panels are stacked into a tab group. This partially supersedes K-074's mechanism note:
the dock's simplification sets `prune_single_child_tabs = true`, and because that pass runs on
every draw, a workspace saved under the old rule is tidied the first time it is shown
(single-child tab wrappers are pruned; layouts keep loading and panes keep their sizes).
Consequences: a bare pane has no tab to drag, so it is re-arranged by dropping tabbed panels
onto it (the Viewer's existing behaviour), and it loses the tab's pop-out button. The Timeline
gets a replacement — right-click an empty spot on its comp-tab strip for **Pop out timeline**
(the request travels through `AppState::pop_out_timeline`, consumed by the shell after the
dock draws); other panels pop out via the tab they grow when stacked. The default layout is
unchanged in substance, minus the two single-child tab wrappers (Scopes, Timeline).

**K-087 · DECIDED · The application is named Lumit (was Luminal); the astral register stays.**
From the owner (2026-07-16), same day as K-083. Luminal becomes **Lumit** (from *lumen*)
everywhere living: UI strings, docs, crate names (`luminal-*` → `lumit-*`, binary `lumit.exe`),
brand asset filenames, and the GitHub repository (`luminalmvm/Luminal` → `luminalmvm/Lumit`,
old URLs redirect). Explicitly retained from K-083: the subsystem names **Nova** / **Nebula** /
**Pulsar**, and the `.lum` project extension (it reads even better for Lumit). Historical
records (this log's earlier entries, `docs/research/`) keep their era's names verbatim.

**K-088 · DECIDED · Flow is a per-layer option, not an effect.** From the owner (2026-07-18).
docs/08 §3.1 placed the flow engine (retime interpolation) in the effect tier list; the owner
reverses that: flow is a property of how a footage layer *samples its source*, so it becomes a
**layer option** — a toggle in the layer's switch cluster, and when enabled, a **Flow** group
beside Transform and Effects in the expanded layer carrying its parameters (quality, and the
knobs 08 §3.1 already specifies). It engages only when it can help: when the footage's frame
rate (through any retime) is lower than the composition's, i.e. when the same source frame
would otherwise repeat across two or more comp frames. The frame-interpolation *policy*
storage (Retime.interpolation) remains the underlying model; the option surfaces it. The
"Flow" name stays pending a better one the owner may pick.

**K-089 · DECIDED · The native plugin API is LFX (was KFX).** From the owner (2026-07-18),
following K-087: Kiriko's initial is gone from the app, so it goes from the plugin API too.
`KFX` → `LFX` in every living doc, `EffectNamespace::Kfx` → `Lfx`, the future host crate
`lumit-kfx` → `lumit-lfx`. Historical entries keep the old name.

**K-090 · DECIDED · Effects do one thing; the menu is categorised; ranges may be one-sided.**
From the owner (2026-07-18), amending docs/08:
- **One effect, one job.** Multi-purpose effects split (the v1 Grade becomes separate colour
  effects); an all-in-one Lumetri-style grading suite MAY exist later as a deliberate
  exception, but singleness is the default shape.
- **The Add-effect menu groups by category** (Blur & sharpen, Colour, Distortion, Stylise,
  Temporal, Utility) — schemas carry a category.
- **Hard ranges may be one-sided** (§1.2 amendment): a parameter like a glow threshold clamps
  at zero below and is unbounded above.
- **Quality tiers where physical accuracy is optional**: chromatic aberration gains a
  wavelength-based mode behind a Bool beside its simple RGB-split mode (§3.6); the same
  pattern is welcome elsewhere.
- **Smooth zoom (§3.5) is dropped**; in its place a **Transform effect** — the transform
  properties as an effect — so an adjustment layer can transform everything below it.
- Per-effect bypass next to the name in the effects UI is confirmed as required (§1.5 already
  specifies it; the implementation carries it).

**K-091 · DECIDED · Adjustment layers stage the composite; collapse never bleeds them into
the parent.** The docs/06 §1.5 model is now the running behaviour: everything below a live
adjustment layer composites into an intermediate, the layer's effect stack runs on that, and
the result mixes back over the unprocessed composite by coverage — the mask raster times the
layer opacity, placed by the layer's transform (the coverage map moves; the picture never
does). Two render-semantics points are pinned:
- The mix is a straight per-channel lerp, alpha included, between the unprocessed and
  processed composites. Routing it through the compositor's premultiplied-over would inflate
  alpha wherever the composite is semi-transparent.
- A live adjustment layer inside a *collapsed* Precomp forces the intermediate (§1.4 force
  list). After Effects lets a collapsed precomp's adjustment layers process the parent's
  stack below them; Lumit deliberately diverges — the stack applies within the adjustment
  layer's own comp, always, so precomposing never changes what an adjustment layer sees.

**K-092 · DECIDED · Theme shape, mode and animation level ship as three independent settings.**
From the owner (2026-07-19): alongside the existing dark-ramp picker (`ThemeVariant`), Lumit
gains a light ramp and a second panel geometry, plus a UI-animation-level control — each its
own setting, not one combined picker, all in the Window menu for now (07-UI-SPEC.md §15's
future Settings window is their eventual home).
- **`ThemeMode` (Dark/Light)**: one light ramp (`Theme::light()`), not a light equivalent of
  every dark variant. `ThemeVariant` (Dark/DarkBlue) narrows to "which dark ramp" and is
  meaningless — hidden in the Window menu — under Light. Light mode ships with **one uniform
  panel colour** (white) on a soft neutral canvas; per-panel colour tinting is a wanted, but
  explicitly deferred, future customisation setting.
- **`ThemeShape` (Sharp/Round)**: Sharp is the existing edge-to-edge, hairline-elevated system,
  byte-identical to before. Round is a Figma-UI3-inspired floating-card system — visible gaps
  between panels and from the window edge, rounded corners, a soft shadow standing in for the
  hairline — carried as data (`ShapeTokens`) on `Theme` rather than hardcoded in `apply()`.
  This reverses two prior binding statements *for Round only*, Sharp keeping them as written:
  §7.3's "there are no gaps between docked panels", and §2.3's shadow_float being "permitted
  solely on" floating chrome — Round's ordinary docked cards join that list. Every panel,
  Viewer included, cards identically under Round; no exemption. A stated, permanent v1 limit:
  stacked tab-bar containers stay square-cornered under Round — `egui_tiles` 0.12.0's
  `Behavior` trait has no hook to round a tab bar's own container.
- **`AnimationLevel` (All/Minimal/None)**: a three-tier refinement of the existing
  motion/reduced-motion binary (15-DESIGN.md §8) — `None` is that same reduced-motion behaviour,
  `Minimal` is the new middle tier. Backed by one global lever over egui's own
  `Style::animation_time`, covering what egui's internals already animate (collapsing
  headers, resizable-panel expand/collapse, scrollbar fade, dialog fade-in). It does not reach
  Lumit's own menus/dropdowns, which have no animation today regardless of this setting.

Spec: [15-DESIGN.md](15-DESIGN.md) §2, §7.3, §8, §11; [07-UI-SPEC.md](07-UI-SPEC.md) §15.

**K-093 · DECIDED · The sub-frame position is content in the frame-cache key under a
synthesising interpolation policy.** Fixing a real bug (owner-reported "flow only changes
once in the middle"): `feed_source` keyed a retimed footage layer on the stamped *integer*
source frame plus the interpolation tag, but not the sub-frame fraction. Under Blend/Flow a
ramp from source frame N to N+1 crosses every fraction in between, each a different
synthesised morph, yet all collapsed onto the nearest integer frame's key — so the three-tier
cache computed one frame per integer span and held it. The key now also hashes the exact
retimed `source_time` whenever the policy is non-Nearest (both the Footage and Sequence
paths). Nearest still hashes nothing beyond the stamped frame, so the "Nearest keys like
no-retime" law is untouched and pre-existing Nearest keys stay shared. No `ALGO_VERSION`
bump: the new keys are strictly longer byte strings, so they cannot collide with the old
buggy keys — stale entries simply stop being addressed, per the Global-Performance-Cache
lesson.

**K-094 · DECIDED · Temporal effects read neighbour source frames; those frames are cache-key
content.** The machinery behind Echo (docs/08 §3.13) and the coming flow motion blur and
datamosh: an effect declares a frame-offset window (`EffectTraits.temporal`), and
`fx::stack_temporal_window` unions a layer's live stack into the offsets the render must
supply. For a footage layer with a temporal stack, the decode path (preview and export
alike, K-031) decodes the layer's source at each offset — mapped through the same retime and
comp frame step as the primary frame, nearest and unmasked — and hands them to the effect.
The frame-cache key hashes those stamped neighbour frames (a `temporal/` block in
`feed_source`'s caller), because the synthesised output depends on them: two comp times that
share a held leading frame can differ in their neighbours. Only footage layers with a live
temporal stack pay this; every other key is byte-for-byte unchanged, so no `ALGO_VERSION`
bump. v1 scope limits (echo's fixed 8-frame window and one-frame spacing, source-not-stack
input, footage-only) are recorded in docs/08 §3.13's status note.

**K-095 · DECIDED · Flow gains an input-rate (conform) override.** From the owner
(2026-07-19), after the K-093 flow fix: interpolating between adjacent frames of
high-framerate footage (e.g. 600fps, whose neighbours are ~1.7ms apart) produces almost no
motion, so flow slow-motion looks frozen. `FlowParams` gains `input_fps: Option<f64>` — the
rate the clip is *interpreted* at for flow. `None` = the source's native rate (adjacent
frames, unchanged behaviour). `Some(r)` with `r` below native conforms the clip to `r` fps:
`frame_pick` brackets the source frames spaced `1/r` apart and blends between *those*, giving
real motion to interpolate — the standard "interpret footage as N fps" trick. Applied
identically in preview and export (K-031); the frame-cache key hashes the conform rate
because the same source time synthesises from different frames under it (no `ALGO_VERSION`
bump — Native keys are byte-for-byte unchanged, and a conformed key gains a `conform` tag).
The Flow group's "Input rate" dropdown offers Native and common rates. (Manual on/off already
exists — the wind toggle forces Flow unconditionally.) Separate near/far-blur-style controls
belong to the future depth-of-field effects, not here.

**K-096 · DECIDED · Scopes v1 read the banked composited frame on the CPU; GPU-live scopes
deferred.** The Scopes panel (docs/07 §8) ships: `Panel::Scopes(ScopeKind)` carries the
scope each instance shows (waveform luma, RGB waveform, vectorscope, histogram), chosen in
its header, persisted with the workspace, so two Scopes panels can show different scopes.
§8 specifies scopes "GPU-computed from the Viewer's displayed frame … live during playback";
v1 narrows that: scopes are computed on the CPU from the composited frame Lumit already
banks in RAM (`comp_frame_cache`, the RAM tier of docs/06), which *is* the Viewer's displayed
frame. That frame is banked only while paused or scrubbing — during playback the readback is
skipped to protect the frame budget (docs/13) — so a v1 scope updates on every paused frame
and holds the last shown frame during playback, rather than tracing live. Live-during-playback
scopes wait on a GPU-side scope pass (a compute shader over the presented texture); recorded
as a v1 limit, not a reversal of §8's intent. Banked frames are always specified-resolution
(draft frames are never banked), so §8's "computed at Half" note never fires in v1. Scope
colours are one fixed `ScopeColours` set on the theme — a near-black graticule and bright
trace whatever the light/dark chrome, the same grading-accuracy reasoning that keeps
`viewer_surround` neutral (docs/15 §2.1). The frame cache gains a recency-neutral `peek`
(alongside `contains_key`) so a scope reading the current frame every paint does not distort
LRU eviction. The §8 tap-point open question (pre- vs post-display-transform) is untouched —
v1 has no display transform, so the banked sRGB frame is both.

**K-097 · DECIDED · Four community colour schemes join the theme as named, first-class
options.** From the owner: alongside Dark, Dark blue and Light, `Theme` gains `gruvbox_dark`,
`gruvbox_light`, `catppuccin_mocha` and `catppuccin_latte` — full constructors populating
every token, built the same way as the existing three (`dark()`/`light()`/`dark_blue()`).
A new `ColorScheme` enum (`Dark`/`DarkBlue`/`Light`/`GruvboxDark`/`GruvboxLight`/
`CatppuccinMocha`/`CatppuccinLatte`) supersedes the old `ThemeMode` × `ThemeVariant` split as
the thing a full theme picker selects from, with `ColorScheme::mode()` still reporting the
light/dark half for callers (e.g. `with_accent`'s hover-shift direction) that only need that.
`Theme::for_scheme(scheme, shape)` is the shape-inclusive composition entry point, sitting
alongside the pre-existing `Theme::for_settings(mode, variant, shape)` rather than replacing
it — both remain callable; wiring the Settings window's Appearance page onto `ColorScheme`
instead of the old two-axis picker is a follow-up change (K-098's window), not part of this
entry. Each new scheme maps its source palette onto Lumit's existing roles rather than
introducing new ones: surfaces follow that palette's own background ramp (monotonic
light→dark for the dark schemes; mirroring `light()`'s "elevation reads as a darker wash"
structure, `surface_4` below `surface_0`, for the two light schemes), text takes that
palette's foreground/muted ramp, `accent` is the scheme's usual signature hue (Gruvbox
orange, Catppuccin mauve), and `viewer_surround` and `scope` stay exactly as every other
theme's — strictly neutral and the one fixed `ScopeColours::STANDARD` respectively, never
palette-tinted, per the grading-accuracy rule in docs/15 §2.1/§11. Gruvbox's error role takes
the palette's *neutral* red rather than its bolder "bright red", a curation choice keeping it
a notch short of alarming in the spirit of docs/15 §3.1's no-punishment-red rule while
remaining an authentic Gruvbox hue. Spec: [15-DESIGN.md](15-DESIGN.md) §2, §11.

**K-098 · DECIDED · A Settings window replaces the Window-menu theme cluster; app-wide
params migrate onto it.** From the owner (2026-07-18): a proper application-settings surface,
macOS-System-Settings-shaped — a left sidebar of pages, each page a column of grouped
"cards" of label-plus-control rows — honouring the Sharp/Round shape like every panel (Round
gives cards a fill and rounded corners, Sharp a hairline frame). It opens from Window →
Settings… or Ctrl/Cmd+comma (`settings.rs`). This supersedes the plan note in docs/07 §15
that the K-092 theme toggles "live in the Window menu for now": Theme Mode, Background ramp,
Accent, Shape and Interface motion now live on the **Appearance** page, and the Window menu
keeps only Reset workspace and a Settings… opener. v1 also ships a **Performance** page
(RAM frame-cache budget and disk-cache cap, both applied live — `ByteLru::set_budget` and a
new `diskio::Cmd::SetCap` the disk worker remembers across project switches) and a **General**
page (reset workspace, version). Performance settings persist on `Shell` as
`PerformanceSettings`; defaults reproduce the previous hardcoded budgets (512 MiB RAM, 50 GiB
disk) exactly, so an existing install is unchanged until a slider moves. The Appearance page's
Mode-plus-Background pair is the old two-axis picker; folding it into a single K-097
`ColorScheme` dropdown (so Gruvbox and Catppuccin are selectable) is the immediate follow-up.
The fuller §15 inventory (VRAM/CUDA, decoder pool, worker cap, cache root/proxy, Preview,
Colour, Export, Keymap, Autosave, Plugins) fills in on this same surface as those systems gain
controls; a GPU-acceleration toggle was deliberately deferred rather than shipped half-wired
(the flow engine lives in the decode worker and needs its own control message). The window is
the `docs/07 §15` "Interface/Preferences" surface, not a second one.

**K-099 · DECIDED · Vignette and Chromatic aberration ship as two new single-frame effects
(docs/08 §3.14, §3.15).** Both are cheap, pointwise, `{0}` temporal, wired at the usual four
sites (schema in `lumit-core`, WGSL kernel + `FxEngine` method in `lumit-gpu`, `run_ops` arm
in `lumit-ui`). **Vignette** — Amount/Radius/Softness/Roundness (each a plain 0–1 fraction)
plus the host Mix — darkens toward black away from the frame centre; Category **Colour**,
matching where docs/08 §3.10's text already listed it as planned scope, not Stylise. Its
distance metric blends between a circle and a frame-aspect ellipse by Roundness, computed from
the raster's own width/height at kernel time, so Radius/Softness need no %-diag conversion
despite governing a spatial falloff — the metric is already resolution-relative by
construction. Amount 0 is the neutral point (bit-exact passthrough, pinned by test, mirroring
Glow's own Intensity-0 short-circuit); a Colour param to tint the vignette away from black was
scoped but deferred, v1 always darkening toward black. **Chromatic aberration** — Amount
(px@comp) plus Mix — is a dedicated, always-radial, single-purpose sibling of RGB split's own
Radial mode (docs/08 §3.6): same R-outward/B-inward shape, but with nothing else to configure,
the same one-thing shape rule that split the old Grade into Colour balance/Saturation (K-090).
Deliberate overlap, not a functional gap: RGB split's Radial mode already covers this exact
maths as one of its three modes, sharing an Amount currency (% diag) with Linear mode's
Angle-driven offset; this effect exists purely for the common one-click case. Because it has no
Angle to share a currency with, its Amount is authored in raw px@comp instead — scaled by the
preview factor like Glitch's Block size — and its ROI trait is `full-frame` rather than a
%-diag padding, since a fixed pixel offset cannot be statically bounded as a percentage of the
diagonal across every comp resolution; Category is **Distortion**, matching RGB split. Neither
the CPU reference nor the WGSL kernel needs an explicit Amount-0 short circuit — the radial
scale factor is an exact `0.0` at Amount 0, so every tap already collapses onto its own pixel,
the same un-guarded style RGB split's own kernel uses — asserted bit-exact by test rather than
built as a branch. Both oracles measured worst 1 fp16 ULP on the dev RTX (0 ULP at their
passthrough cases), within the cheap-class ≤ 2 ULP bound (§1.6).

**K-100 · DECIDED · The Performance page gains a video-memory (VRAM) budget and a
Clear cache action.** Extends K-098: `PerformanceSettings` gains `vram_cache_mb` (default
512, matching `GpuViewer`'s existing `VRAM_TIER_CAP`), applied live through a new
`GpuViewer::set_vram_cap` alongside the RAM and disk lines already wired in
`apply_cache_budgets`. `set_vram_cap` re-evicts the VRAM tier's oldest entries against the
new cap immediately, reusing the same `vram_evict_count` policy `present_keyed` already
applies on insert — no separate eviction logic. A **Cache** group joins the Performance page
with a single **Clear cache** button: it empties the RAM `comp_frame_cache` and the VRAM
tier (`GpuViewer::clear_vram`, which releases each texture's egui registration so nothing
leaks) and bumps `AppState::cache_epoch` so the cache bar and any live views notice the
tiers are now empty. This is the first row of the docs/07 §15 "Performance" inventory's VRAM
budget to ship; CUDA on/off, decoder pool size, worker thread cap and background cache fill
remain open.

**K-101 · DECIDED · Effects browser drag-to-apply lands on Timeline layer rows in v1, scoped
to footage and adjustment layers.** Implements the docs/07 §7 apply path "drag onto a layer
row in the Timeline": each built-in-effect entry in the Effects & Presets browser
(`effects_panel`) is a drag source carrying an `EffectDragPayload(&'static str)` — the
effect's stable `match_name` — kept distinct from the Project panel's `uuid::Uuid` item
payload so a drop target can tell them apart by type alone. In the Timeline, a layer row
accepts the drop only when `accepts_effect_drop` says its kind is Footage or Adjustment — the
effect stack's two ordinary homes; every other kind (Sequence, Precomp, Solid, Text, Camera)
still gains effects only through its own row's existing "Add effect" menu, unchanged. A
hovered drop paints an accent outline over the row's lane area; a release instantiates the
effect (`fx::instantiate`) and appends it to the layer's `effects` through the same
`Op::SetLayerEffects` the "Add effect" row commits, so applying by drag is one ordinary undo
step, then the preview refreshes the way other Timeline commits do. Double-click apply, drag
onto the Viewer, and presets/favourites — the rest of §7's inventory — remain later steps.

**K-102 · DECIDED · Command palette and a composition hierarchy panel ship as the first two
command/navigation surfaces.** Two self-contained UI surfaces, both `egui::Modal`/panel work
touching no engine code. (1) The **command palette** (docs/07 §12, `command_palette.rs`):
Ctrl/Cmd+Shift+P or Window → Command palette… opens a top-anchored modal with a focused
search box over a fuzzy-ranked command list (subsequence match; a label hit outranks a
keyword-only hit; earlier/contiguous matches rank higher — unit-tested). v1 covers the
commands category (save, undo/redo, new composition, add layers, reset workspace, open
Settings, colour-scheme and shape switches, export); the effects/comps/panels categories,
recent-first ranking and taught shortcuts are later. It is explicitly **not** the deferred
effects radial menu (Ctrl+Space, apply-to-clip) — that remains blocked on a from-scratch
build (no egui 0.31-compatible `egui_pie_menu`/`egui_node_graph`). (2) The **Hierarchy
panel** (`hierarchy.rs`, a new `Panel::Hierarchy` tabbed into the left group of the default
layout): a read-only, recursion-guarded tree of the active composition — its layers, with
precomp layers folding open to their nested composition's layers; clicking a row selects that
layer and switches to its composition. It is the simple tree form of the AE composition
flowchart; the full node-graph flowchart (the same deferred `egui_node_graph`-style view the
radial menu wants) grows from it. Both count as modals/panels that suppress the active-panel
focus edge while a modal is open, reusing the K-098 modal-gating.

**K-103 · DECIDED · Layer parenting (AE-style transform inheritance) — foundation first.**
`Layer` gains `parent: Option<Uuid>` (serde default `None`, so every existing project and
layer is byte-for-byte unchanged; a missing/deleted/cyclic parent degrades to "no parent" at
render time, the same invariant as `matte`). `Op::SetLayerParent { comp, layer, parent }`
sets or clears it, rejecting a self-parent, a parent not in the comp, or one that would form
a cycle (`OpError::InvalidParent`), with cycle-safety in two pure, tested helpers
(`model::layer_parent_chain`, `model::parenting_would_cycle`). This entry lands the **model +
op + validation** only; the transform is not yet inherited at render time. The render wiring
is planned to reuse the existing, proven primitives — `lumit_gpu::place_matrix` +
`concat_place` + the `CompLayerDraw.pre` field that precomp-collapse already uses — via a
shared parent-chain world-placement helper called by BOTH `draws.rs` (`build_comp_draws`,
preview) and `export.rs` (`render_comp_linear`, export) so preview/export parity holds
(K-031), gated on `parent.is_some()` so unparented layers keep their exact current path.
v1 scope composes the 2D affine (position/anchor/scale/rotation); inheriting the 2.5D axes
(`position_z`, `rotation_x/y`) is a follow-up. UI: a Parent picker in the layer's inspector
rows. Staged deliberately so the safe, fully-tested foundation ships before the render-path
change, which is best verified visually with the owner present.

**K-104 · DECIDED · Datamosh (Glitch's third section) ships, reusing Motion blur's flow
machinery rather than adding new plumbing.** Datamosh (docs/08 §3.12) was deferred at K-094
pending "machinery no effect has yet"; Motion blur (§3.2) built that machinery in the
meantime, and Datamosh turned out to need only a second frame pair through it, not new
infrastructure. `fx::stack_temporal_window`/`stack_is_temporal` gain the one case in the
registry where an effect's temporal reach depends on a param value, not just its static
schema trait: a live `glitch` instance's `datamosh_enabled` bool (new, off by default) adds
offset `-1` to the window. `stack_wants_flow_field` (bool) is replaced by
`stack_flow_neighbour` (`Option<i32>`): Motion blur wants neighbour `1`, Datamosh wants
`-1`. A layer carries only one flow field per frame in v1 (`CompLayerPixels::flow_field`
stays a single slot) — if a stack somehow has both a live Motion blur and a Datamosh-on
Glitch, the first one encountered in stack order wins the slot and the other's flow-
dependent behaviour degrades to its existing missing-field passthrough (pinned by test).
Datamosh itself is one GPU pass sharing Motion blur's `mb_layout`/`mb_pl` (three sampled
inputs — current frame, `-1` neighbour, flow field — plus storage-out and uniform): a single
bilinear tap per pixel (motion-compensated prediction), not a streak integral, blended
against the already block/scanline'd frame by the shared Intensity dial. Off by default
(unlike Block displacement/Scanlines, which have been on since Glitch first shipped) because
it is footage-only and adds a flow computation the moment it is live — existing Glitch
instances render byte-identically until an editor opts in. Operates on the layer's *source*
frames, the same v1 simplification Echo and Motion blur already made. Oracle: GPU matches
`lumit_core::fx::cpu::datamosh` at ≤ 2 fp16 ULP (measured 0–1).

**K-105 · DECIDED · Solo / isolate switch on layers.** `Switches` gains `solo: bool` (serde
default false, so every existing project is byte-identical). While *any* layer in a
composition is soloed, only soloed layers render — the standard After Effects isolate. The
gate is one shared helper, `model::any_solo(comp)`, applied identically in the preview
(`build_comp_draws`) and export (`render_comp_linear`) visibility checks so the two agree
(K-031): a layer renders iff `visible && in_span && (!any_solo || solo)`. `Op::SetLayerSolo`
toggles it as one undo step (mirroring `SetLayerVisible`). The control is a Solo checkbox at
the top of the Effect Controls panel, beside the Parent picker; a Timeline solo column is a
later refinement. Known v1 edge: a non-soloed layer used as a *matte* source for a soloed
layer is hidden like any other non-soloed layer (solo takes precedence over the matte-source
exemption) — acceptable until the Timeline surface makes solo state obvious per row.

**K-106 · DECIDED · Exposure ships as a new single-frame grade effect (docs/08 §3.16).**
A single scene-linear gain on RGB, `factor = 2^Stops`, computed host-side so the CPU
reference and the WGSL kernel multiply by the identical number (no per-pixel `exp2`, no path
divergence). Params Stops (default 0, slider −5..+5, unbounded) plus the host Mix; Category
**Colour**, alongside Colour balance and Saturation. Premultiplied — a scalar scales
premultiplied colour consistently, so no unpremultiply round trip and alpha is untouched.
Continuous (unlike a posterise/quantise, which would blow the ULP oracle at every
quantisation edge), so the §1.6 oracle holds to ≤ 2 fp16 ULP (measured 0–1 on the dev RTX).
`factor` 1.0 (0 stops) short-circuits to the input on both paths — the bit-exact neutral
point, pinned by test — and Mix 0 is likewise the identity. Distinct from Colour balance's
three-channel Gain: the single, animatable, photographic-stops brightness lever the montage
grade reaches for first. Wired at the usual four sites (schema in `lumit-core`, WGSL +
`FxEngine::exposure` in `lumit-gpu`, `run_ops` arm in `lumit-ui`).

**K-107 · DECIDED · Glitch splits into Block glitch, Scanlines and Datamosh; the combined
effect is removed (docs/08 §3.12).** Per the one-effect-one-job rule (K-090 — the same rule
that split the v1 Grade into Colour balance and Saturation, and split Chromatic aberration
off RGB split's own Radial mode): the old `glitch` effect did three things behind enableable
section toggles (Block displacement, Scanlines, Datamosh — the last added by K-104), so it
splits into three standalone schemas — `block_glitch`, `scanlines`, `datamosh` — and `glitch`
is deleted outright. Pre-v1, single user, no saved-project migration: existing `glitch`
instances simply stop resolving; no alias or upgrade path is built. `block_glitch` and
`scanlines` carry over their section's parameters unchanged (ids, labels, ranges, defaults),
minus the now-redundant `block_enabled`/`scanline_enabled` toggles — each is always on in its
own effect now. Stacking Block glitch → Scanlines, each at Mix 100%, reproduces the old
combined Glitch's look bit-for-bit, since the two sections never interacted beyond sharing one
kernel pass. `block_glitch` keeps `seeded: true` and `full-frame` ROI (the block hash can
displace a read from anywhere in the grid); `scanlines` drops Seed entirely and declares
`seeded: false` and `exact` ROI — it reads the input pixel directly, no hash, no neighbour tap.
Datamosh keeps its existing GPU pass and CPU oracle (`FxEngine::datamosh`, `cpu::datamosh`,
`fx_datamosh.wgsl`) byte-for-byte unchanged; only its schema, `Resolved` variant and stack
wiring are new. Its temporal reach is now **static** — schema `temporal: {0, -1}`, the same
shape Motion blur's own `{0, +1}` already has — which retires the one dynamic special case
`stack_temporal_window`/`stack_flow_neighbour` carried since K-104 (a live `glitch` instance's
`datamosh_enabled` param toggling whether the stack's temporal window and flow-field gate
reached back to -1); `stack_flow_neighbour` now recognises a live `datamosh` instance the same
static way it recognises `motion_blur`. Datamosh's Mix folds into its existing single-blend-
fraction `intensity` argument by multiplication at the call site (`run_ops`) rather than adding
a second uniform to the unchanged kernel — mixing the same two inputs (current frame, warped
neighbour) twice collapses algebraically to one mix by the product, so Intensity-0 and Mix-0
are both the identical bit-exact passthrough. All three new schemas declare Category
**Distortion**, matching Shake and RGB split (their closest siblings: a seeded positional
wobble; a channel split), not the additive-light Stylise pair (Glow, Flash) — unchanged from
the old combined Glitch. Landed as three green commits: Datamosh split out first (retiring the
dynamic special case on its own), then Block glitch/Scanlines split out and `glitch` deleted,
then docs.

**K-108 · DECIDED · Hue shift ships as a new single-frame grade effect (docs/08 §3.17).**
A constant-luminance hue rotation (the standard SVG `feColorMatrix` hue-rotate, Rec.709 luma
weights), a linear 3×3 colour matrix computed host-side (`fx::hue_matrix`) so the CPU
reference and the WGSL kernel multiply by identical `f32` coefficients — the nine travel as
individual uniform fields so their tight packing matches the Rust `[f32; 9]` (a uniform array
strides at 16). Params Angle (degrees, default 0) plus the host Mix; Category **Colour**,
beside Exposure and Saturation. Premultiplied — a linear matrix scales through alpha, so no
unpremultiply round trip and alpha is untouched. Continuous, so the §1.6 oracle holds to ≤ 2
fp16 ULP (0–1 on the dev RTX). 0° resolves to the exact identity matrix (the bit-exact neutral
point, pinned by test); Mix 0 is likewise the identity. The rotation runs in scene-linear
working space, consistent with the other grades. Wired at the usual four sites.

**K-110 · DECIDED · Contrast ships as a new single-frame grade effect (docs/08 §3.18).**
The fourth one-knob colour grade beside Exposure, Hue shift and Saturation: it expands or
compresses each RGB channel about a fixed pivot, `out = (in − pivot) × k + pivot`, with
`k = Contrast ÷ 100` (default 100 % = identity, slider 0–200, hard min 0 and unbounded above,
matching Exposure/Saturation's one-sided bound) and `pivot = 0.5`. The pivot is a plain
mid-grey 0.5, not the 0.18 scene-linear mid-grey, so the control behaves like a photo-editor
contrast slider (symmetric about 50 %) rather than a light-meter grey card — the one
substantive design call, flagged for the owner to review. Because the `− pivot` offset makes
this an affine grade, not a pure scale, it does not commute with premultiplied alpha: it
declares `premultiplied: false` and the host unpremultiplies → grades → re-premultiplies (like
Colour balance and Saturation), so matte edges do not shift — unlike Exposure, whose pure
multiply is alpha-safe. Alpha is untouched and the maths runs in the scene-linear working
space. Continuous everywhere (no round/clamp/quantize), so the §1.6 oracle holds (worst 1 fp16
ULP on the dev RTX, partial-alpha pixels tested); Contrast 100 % and Mix 0 are bit-exact
passthroughs. Resolve clamps `k` at `max(0.0)` to honour the schema's hard min; the kernel
itself clamps nothing, staying continuous. Wired at the usual sites, built in an isolated
worktree and merged.

**K-111 · DECIDED · File-reference parameter kind, animated by stepping (K-109 skipped).**
Effects can declare a `File` parameter (`ParamKind::File { filter, filter_name }`) whose value
is a `FileParam { paths: Vec<String>, index: Property }` — a set of referenced file paths plus
an f64 `index` selecting which is live at a given time. The inspector shows the file's basename
and a "Select …" button opening a native dialog filtered by the effect's declared extensions;
picking a file sets a single static path. It is animatable, but only by *stepping*: two paths
cannot be blended, so the index carries **Hold keyframes only** (the discrete keyframe that
landed just before this) and is rounded and clamped at evaluation, never landing between paths.
This deliberately reuses the whole existing keyframe / graph / expression machinery for the
index rather than adding a string-valued keyframe type; the common case is one path with a
static index. An empty `paths` is the unset state and resolves to identity, so a File-param
effect is a no-op until a file is chosen — a sanctioned exception to the no-no-op-default rule
(§1.2), since a file the user must supply has no tasteful default. The path string joins the
frame cache key (length-prefixed, the live path at the time), the same policy a footage source
path follows; file *contents* are re-read by the consumer's own path+mtime cache, not this
hash. First consumer is the coming LUT effect (§3.11). K-109 was reserved for this during
parallel work but Contrast took K-110 first, so K-109 is intentionally skipped to keep this log
ascending.

**K-112 · DECIDED · Gamma ships as a new single-frame grade effect (docs/08 §3.19).**
The fifth one-knob Colour grade: a per-channel power curve `out = pow(max(in, 0), 1 ÷ gamma)` in
scene-linear working space, alpha untouched. Float Gamma (default 1.0, slider 0.1–4.0, hard floor
0.01 to keep `1 ÷ gamma` finite, no ceiling — Contrast's open-topped shape). The input is clamped
to ≥ 0 before the power (scene-linear can dip negative and a power of a negative base is
undefined); the clamp is byte-identical on CPU and GPU so the §1.6 oracle holds (≤ 1 fp16 ULP on
the dev RTX). The exponent is `1 ÷ gamma`, so Gamma above 1 brightens mid-tones (the display-gamma
reading), the opposite direction from Colour balance's per-channel Gamma — noted in §3.19 to avoid
confusion. A power curve is non-linear, so it does not commute with premultiplied alpha:
`premultiplied: false`, host-wrapped unpremultiply → curve → re-premultiply like Contrast and
Saturation. Gamma 1.0 short-circuits to a bit-exact passthrough (not a reliance on `pow(x, 1)`
being `x`) and Mix 0 likewise, both pinned by test. Built in an isolated worktree and merged.

**K-113 · DECIDED · Temperature ships as a new single-frame grade effect (docs/08 §3.20).**
The sixth one-knob Colour grade: a warm/cool white balance as a per-channel gain in scene-linear
space, `gain_r = 1 + 0.5·k` and `gain_b = 1 − 0.5·k` for `k = Temperature ÷ 100` (green and alpha
held). Float Temperature (default 0, slider −100..+100, hard ±100). The two gains are host-computed
at resolve and passed as uniforms, so the CPU reference and the WGSL kernel multiply by
byte-identical f32 factors. A per-channel multiply commutes with premultiplied alpha (scaling a
premultiplied channel by a constant is exact, alpha untouched), so it declares `premultiplied:
true` and applies straight through like Exposure — unlike the affine Contrast and Saturation
grades, no unpremultiply round trip. Continuous everywhere (a linear scale, no round/clamp/quantize),
so the §1.6 oracle holds (worst 1 fp16 ULP, partial-alpha tested); Temperature 0 gives gains
exactly `(1.0, 1.0)` for a bit-exact identity, Mix 0 likewise, both pinned by test. REVIEW: the
±0.5 R/B strength (so ±100 → red/blue gains 1.5/0.5, green held) is a taste choice for the montage
warmth range, not a physical calibration; the fuller Bradford-adapted CCT white balance with a
Tint axis remains a Tier-2 job (§3.10). Built in an isolated worktree and merged.

**K-114 · DECIDED · The LUT effect ships (docs/08 §3.11), the File param's first consumer.**
A `lut` built-in in the Colour category, v1 subset: a File parameter (`.cube`, animatable by
hold-stepping between paths — K-111) plus the host Mix, applied 3D-trilinear in the compositor's
scene-linear working space **as-is** (no Input-space transfer), unpremultiplied. `Resolved::Lut
{ mix }` carries only Mix; because a file path is not `Copy`, the parsed-and-uploaded cube
travels **beside** the resolved op as a parallel `luts` slot on `fxops::run_ops`, exactly as the
flow field and neighbour frames do for the temporal effects. `CompLayerDraw.lut_files` carries a
layer's ordered enabled-builtin-`lut` paths; since a `lut` effect always resolves to exactly one
`Resolved::Lut`, that list is 1:1 and in order with the ops (the threading linchpin). Preview
(GpuViewer) and export (Renderer) both build the list with the identical filter and load it
through a path-keyed upload cache into the one shared `run_ops`, so they are pixel-identical
(K-031, reviewed by hand rather than by test since the wiring has no end-to-end oracle). An
unset, missing, 1D, or unreadable file is a labelled no-op, never a fault. `cpu::apply` is a
passthrough — a LUT is a GPU colour map, so the CPU degradation rung renders it as identity, and
the §1.6 oracle reference is `lut::Lut3d::sample` used directly in the lumit-gpu kernel test
(worst 1 fp16 ULP), the one effect whose reference lives outside `cpu::apply` because its
parameter is a file, not a number. The GPU uses the first 3D texture in the FxEngine
(`Rgba32Float` cube, manual `textureLoad` trilinear — not the hardware sampler — so the oracle
stays exact). Follow-ups (flagged): Input-space control, Tetrahedral interpolation, mtime cache
invalidation, a content-hash cache key, and embedding small LUTs in the project (K-040). Built
across three isolated worktrees (parser, GPU sampler, wiring) and merged.

**K-115 · DECIDED · The Performance page gains a Background fill toggle (K-109, K-114
skipped/reserved).** Closes the last named row of K-100's remaining list. `PerformanceSettings`
gains `background_fill: bool` (default `true`, matching today's unconditional behaviour) with a
struct-level `#[serde(default)]` so an older saved workspace missing the field falls back to the
default rather than failing to deserialize (the existing three fields relied only on the
field-level default on `Shell::settings`, which only covers a wholly-absent `settings` key, not
a `PerformanceSettings` missing one new field — this closes that latent gap for future fields
too). The Cache group's idle-fill loop (`shell/mod.rs`, the "Idle: fill the work area around the
playhead" block) is gated on the new flag alongside its existing playing/interacting/in-flight
checks; off means zero background decode/render work while idle, trading a colder cache for a
quieter machine. K-114 is reserved for the in-flight LUT effect and intentionally skipped here to
keep the log ascending without colliding with that session's work.

**K-116 · DECIDED · Hit-target compensation promoted from KD-2 (docs/15-DESIGN.md §1.2/§7.2).**
The household accessibility gate demands ≥44px touch targets everywhere; a Timeline showing
twenty layers at once cannot meet that on every row, so Lumit records a deliberate, scoped
exception rather than silently missing the gate. Toolbar, transport, dialog, and Viewer-toolbar
controls keep the full household ≥44px hit extent. Dense-surface controls — Timeline rows,
clips, keyframes, curve handles, property lanes, the cache bar — drop to ≥24px **visual** extent
on their smaller axis, but MUST still carry ≥32px of **interactive** hit-slop (e.g. a keyframe
renders at 9px but hit-tests at 32px, nearest-wins, with adjacent slop regions split at their
midpoint). Timeline rows default to 28px, 24px minimum at the densest zoom; nothing interactive
ever hit-tests below 32px in either axis. This was recorded as PROPOSED deviation KD-2 pending
promotion to the decision log (docs/15-DESIGN.md §Open questions); that question is now
resolved — KD-2 is promoted here as DECIDED, and docs/15-DESIGN.md is updated in the same commit
to point at K-116 instead of the stale "promote as K-006" note (K-006 was independently taken by
Migration-aware first run before this promotion happened).

**K-117 · DECIDED · Settings → Performance → Cache gains a cache root folder override
(docs/07-UI-SPEC.md §15).** Closes the last named row of the Cache group.
`PerformanceSettings::cache_root: Option<PathBuf>` (default `None`) keeps today's
`<project>-cache` sidecar-beside-the-project-file behaviour byte-for-byte, so existing projects
and saved workspaces are unaffected until the user picks a folder. When set, each project's disk
cache moves under the chosen root as `<stem>-<hash8hex>-cache`, the hash taken from the
canonicalized project path so same-named projects in different folders never collide while the
stem keeps folders eyeball-recognisable. `lumit_cache::disk::cache_root_for` carries the
override-aware lookup; the existing `sidecar_root` is untouched and still backs the `None` case.
The picker uses `rfd::FileDialog::pick_folder`, matching every other file/folder chooser in the
app. Applied live: `AppState::disk_sync_root` already polls once per frame and diffs the
computed root against the one in use, so a Settings change repoints the disk-cache worker on the
next frame with no restart. Trade-off, flagged for follow-up: old cache folders at a previous
root are not migrated or deleted when the root changes — orphaned, not corrupting, consistent
with the cache's "always safe to delete, never authoritative" design; worth a cleanup pass if
orphaned caches become a nuisance. Built in an isolated worktree and merged.

**K-118 · DECIDED · The Settings window gains an Interface page: UI scale and a tooltips
on/off switch (docs/07-UI-SPEC.md §15).** Closes two of the three named controls in the
Interface group; reduced motion already shipped separately as Interface motion on the
Appearance page (K-092) and is untouched here. UI scale is a 75–200% slider applied live
through egui's own `Context::set_pixels_per_point` — the same zoom primitive behind egui's
built-in Ctrl+=/Ctrl+- shortcut, here surfaced as a persisted preference applied at start-up as
well as on change, rather than a per-session nudge. Tooltips are suppressed globally by pushing
`egui::Style::interaction.tooltip_delay` to infinity rather than gating each `.on_hover_text()`
call site individually — confirmed against `Response::should_show_hover_ui` that this genuinely
prevents a tooltip ever showing, and confirmed the resulting infinite duration cannot panic the
repaint-scheduling path. "On" restores egui's own stock default delay rather than a hardcoded
guess. Both default to today's implicit behaviour (native scale, tooltips on), so no existing
install changes until the user visits the page. Trade-off, flagged for follow-up: tooltip
suppression rides on `tooltip_delay`'s current meaning in egui's style struct, which is worth
re-checking on any future egui upgrade. Built in an isolated worktree and merged.

**K-119 · DECIDED · The Settings window gains an Export page: a default preset and a filename
template (docs/07-UI-SPEC.md §15).** Closes two of the four named rows in the Export group;
export priority and encoder preference order stay unbuilt — no priority or encoder-order
concept exists anywhere in the export pipeline yet, so a control for either would be dead.
`ExportSettings::default_preset` (default `Custom`, matching `ExportPreset`'s own new `Default`)
is stamped by every generic "Export…" action — the File-menu entry and its native-menu twin —
while an explicit pick from the "Export preset" submenu always keeps its own preset regardless.
`ExportSettings::filename_template: Option<String>` (default `None`) substitutes `{comp}`,
`{preset}`, and `{date}` into the export dialogue's suggested name when set, sanitised against
characters Windows forbids in file names (a composition name is free text and can carry one)
and guaranteed to end in `.mp4`; `None`, or a template blank once trimmed, reproduces
`preset.default_file_name()` byte-for-byte, so no existing install's suggested name shifts until
the user visits the page. Today's date comes from a small hand-rolled UTC civil-date conversion
(Howard Hinnant's `civil_from_days` over `SystemTime`) rather than a new `chrono`/`time`
dependency. Built in an isolated worktree and merged.

**K-121 · DECIDED · Matte key ships as a soft chroma-key effect (docs/08 §3.21).**
A greenscreen keyer in the Utility category: alpha is driven down where a pixel's chroma is
close to a chosen key colour. The metric is Euclidean distance in the chroma plane — a
colour's chroma is `rgb − Rec.709-luma`, so distance ignores brightness and a green of any
exposure keys alike. The keep-factor is `smoothstep(tolerance, tolerance + softness, d)` —
fully keyed (alpha ×0) at/below tolerance, fully kept at/above tolerance+softness, smooth
between — so it is continuous everywhere (no hard step, which would blow the cheap-class ULP
oracle). It runs on straight colour (`premultiplied: false`, §2.2): unpremultiply → key +
despill → re-premultiply, like Saturation, so edges are judged by true colour not coverage.
Spill suppression removes a fraction of the pixel's projection onto the key-hue direction,
desaturating kept pixels toward their own luma along the key hue so green fringes fade (a grey
key has no hue, so spill is a no-op). The key colour is a `ParamKind::Colour` resolved to a
scene-linear array at frame time; CPU reference and WGSL kernel derive the chroma/hue from that
identical resolved colour, holding the §1.6 oracle to ≤ 2 fp16 ULP (measured 1). Default green
+ Tolerance 20 % key a typical screen out of the box (the tasteful-default rule, §1.2, so no
neutral no-op); Mix 0 is the bit-exact identity. Chroma-distance was chosen over a hue-angle
metric to avoid per-pixel trig and keep CPU/GPU byte-identical (trade-off: saturation-sensitive,
which Tolerance widens for). A viewer eyedropper to pick the key off the image, and a
matte-choker / luma-key companion, are noted follow-ups. Built in an isolated worktree and
merged. (Numbered after K-120 per-layer motion blur, which lands from a parallel worktree; the
two are independent, so the log briefly carries K-121 before K-120.)

**K-120 · DECIDED · Per-layer motion blur is transform-sampled multi-draw (docs/06 §4).**
With a composition's motion-blur master on (`Composition.motion_blur.enabled`), a layer whose
own `Switches.motion_blur` is set is drawn at N sub-frame placements across the open shutter —
offsets `phase/360 + (k + 0.5)/N · angle/360` frames, centred by the −90°/180° AE defaults
(`MotionBlur::sample_offsets`) — and averaged into one comp-space smear; the layer's blend,
opacity, matte and mask apply once to that average, not per sub-copy. The average is a **true
premultiplied mean** via a dedicated additive-on-both-channels accumulation pipeline (not
`Blend::Add`, whose `alpha: over` would leave a static opaque layer at ~63 % alpha), so a still
layer is unchanged and a moving one thins along its path. Preview (`realise_segment`) and export
(`render_comp_linear`) derive the sample times through one shared `motion_blur_samples` and
build the average through one shared `Compositor::motion_blur_average`, so a blurred preview
equals a blurred export (K-031, reviewed by hand — both call the one helper). Comp motion-blur
settings and the per-layer switch join the frame cache key. Only the layer's own transform is
sampled; **parent-motion blur** (a still layer under a moving parent) and per-layer blur on the
inner layers of a **collapsed Precomp** are deferred follow-ups. Numbered K-120 though it lands
just after K-121 (matte key), the two being independent parallel-worktree work. Distinct from
the flow `motion_blur` effect (footage-internal motion) and the coming accumulation MB (full
sub-frame re-render).

**K-122 · DECIDED · Timeline and effects-panel interaction pass (docs/07 §4/§6).**
A batch of timeline/effects-panel UX with two decision-sized parts. **Reorder by
drag:** a layer is restacked by dragging its name in the outline, committing one
`ReorderLayer { comp, layer, new_index }` (lift-and-reinsert, clamped, 0 = top,
its own inverse); an effect is restacked by dragging its name, committing the
existing whole-stack `SetLayerEffects` (its doc already designates it the
add/remove/reorder commit, so no dedicated `ReorderEffect`). Each move is one
undo step with an accent insertion line. **A single layer context menu:**
right-clicking a layer's name opens one menu — rename, add effect (BUILTINS
submenu), add mask, duplicate, delete, solo, enable, convert-to-sequenced,
trim-to-source — **replacing** the old lane-bar right-click menu, so a layer's
actions live in one place (right-clicking the bar no longer opens a menu).
Non-decision polish landing with it: double-click a name to rename inline
(`RenameLayer`); names are a frameless button so dragging never selects text;
opening a layer twirl no longer auto-opens the Transform sub-twirl; the Effect
Controls panel and layer area get themed separator bars per effect/section title;
a column-header icon row sits over the outline switches level with the ruler; and
the effect drag-drop onto a layer (outline or lane) and into the Effect Controls
panel is fixed — the old drop tested a lane-clipped rect, so the visible half
never registered; it now uses occlusion-proof `contains_pointer` full-row drop
zones. Layer-area width is session state, not persisted (like every timeline
preference). Built in an isolated worktree and merged.

**K-123 · DECIDED · Layer-reference effect parameter kind (docs/03 §8, docs/08 §1.2).**
Effects gain a parameter referencing **another layer** in the same composition as an auxiliary
picture — `ParamKind::Layer {}` / `EffectValue::Layer(Option<Uuid>)`, the shape a track matte's
`MatteRef` uses minus channel/invert (static in v1). The host renders that layer **alone,
source-only** (its own effect stack skipped) and threads its texture to the effect beside the
resolved ops via the one shared `fxops::render_layer_input`, exactly as the matte stage renders
a matte layer alone; preview and export call that one helper so they match (K-031). Source-only
rendering makes reference **cycles structurally impossible** (the depth render never re-enters an
effect stack). An unset or dangling reference resolves to **identity** — the sanctioned no-op
exception the File parameter also takes, since a layer the user must supply has no tasteful
default. The frame cache key hashes the referenced layer's source + transform (the matte block's
shape). The inspector **Layer picker** and an undoable set-param op are a follow-up; until then
an unpicked Layer renders as nothing via the inspector's existing wildcard. First consumer is
the DoF effect (K-124). Built in an isolated worktree and merged.

**K-124 · DECIDED · Depth of field ships as a depth-driven lens blur (docs/08 §3.22).**
A variable-radius disc blur whose per-pixel circle-of-confusion comes from a **depth pass**
supplied by a Layer-reference parameter (K-123) — the first effect to take a whole layer as
input. Params: Depth layer, Focus distance (0.5), Focus range (0.1), Aperture (px@comp, 8,
slider 0–40), Mix; premultiplied, Moderate cost, padded ROI, `{0}` temporal, Blur & sharpen
category. It drives the pre-existing `lumit_gpu::fx::dof` kernel and its §1.6 oracle (depth read
from the referenced layer's red channel, 0 near / 1 far, symmetric about Focus). v1: the depth
layer is rendered source-only and **resampled to the effect's working raster** `(w, h)` — not
comp size, since the kernel reads depth at the consuming layer's own grid, which shrinks under
reduced-resolution preview; a framing-matched depth pass is expected, and the depth layer must be
visible + in-span in preview (the decode-planner gate, a recorded follow-up to lift). Placement/
effects-aware depth and the shaped-bokeh "DOF PRO" second effect are post-v1. Preview == export
via the one shared render helper. Built in an isolated worktree and merged.

**K-125 · DECIDED · Matte "after effects" toggle (docs/03 §6 matte, docs/impl/layer-input.md).**
A matte reads the source layer's **source pixels** by default (its own effect stack irrelevant),
but a new `MatteRef::after_effects` bool (serde-default false, so old projects are unchanged) has
the source's **own effect stack run into the matte texture** before it gates the consumer — a
keyed greenscreen, a blurred or levels-adjusted edge. The matte source is uploaded, linearised,
`run_ops` applies its resolved stack, then it composites alone exactly as a source-only matte
does; preview (`shell::gpu`) and export both do this from the same resolve + `run_ops`, so they
match (K-031). This also **fixed a latent K-031 bug**: export had been feeding the matte source's
*post-fx* `prepared` texture while preview fed source-only, so a matte source with effects
diverged between the two; both are now source-only by default and post-fx only when the toggle is
set. The frame key folds the source's stack (via the shared `feed_effect_stack`) only when the
toggle is on, so a source-only matte keeps its keys and a keyed matte invalidates when its key
colour moves. **v1 boundary:** temporal inputs (echo neighbours, flow motion-blur field, a nested
depth reference) are **not** fed through an after-effects matte — the source's spatial and colour
stack applies, but an echo/flow effect on the matte source degrades to a still; the common cases
(colour key, blur, levels) are exact. The same toggle for a Layer-reference depth input (K-123)
rides as a `depth_after_effects` schema bool on each consuming effect, not a model field. Built on
the main branch alongside the effects sprint. *Follow-up landed same sprint:* the DoF depth input
gained `depth_after_effects` (default false); `render_dof_inputs`/`build_dof_inputs` run the depth
layer's stack before resampling, and the key folds it via `feed_effect_stack`'s Layer arm guarded
by a one-level `allow_after_effects_refs` (a referenced layer's own layer-inputs stay source-only,
matching the render where they render as passthrough).

**K-126 · DECIDED · Invert ships as a single-frame colour effect (docs/08 §3.23).**
A simple colour inverse — `out.rgb = 1 − in.rgb` per channel, alpha kept — with only the host
Mix. Because `1 − c` is affine (not a pure scale) it does not commute with premultiplied alpha,
so it declares `premultiplied: false` and the host wraps unpremultiply → invert → re-premultiply,
exactly like Contrast and Gamma (§2.2), so matte edges do not fringe. The inverse is taken in the
compositor's scene-linear working space as-is (the owner's "simple inverse"): values above 1.0
invert to honest negatives, never clipped, and there is no display-referred round trip — a
perceptual inversion is a possible later variant. Cheap cost, Exact ROI, `{0}` temporal, Colour
category (beside the other grades). Continuous everywhere, so the §1.6 oracle holds to ≤ 2 fp16
ULP (measured worst 1); there is no neutral no-op value (invert always inverts), and Mix 0 is the
bit-exact identity, both pinned by test. Built in an isolated worktree; not pushed.

**K-127 · DECIDED · Tint ships as a luminance-duotone colour effect (docs/08 §3.24).**
A gradient map: two colour params, Map black to (default black) and Map white to (default white),
and `out.rgb = black + (white − black)·luma(in)` with Rec.709 luma on the unpremultiplied colour,
alpha kept — every pixel's brightness picks a colour on the two-colour gradient, recolouring the
image while keeping its luminosity structure (the owner's "map all colours between two colours").
A luma-driven remap does not commute with premultiplied alpha, so it declares `premultiplied:
false` and the host wraps unpremultiply → map → re-premultiply, like Contrast and Gamma (§2.2).
The lerp is written `black + (white − black)·luma` (not the `mix()` form) so the CPU reference and
the WGSL kernel reduce in the same order. The default black→black / white→white maps every pixel
to its own luma — a greyscale, a visible tasteful default (§1.2), not a no-op. Cheap cost, Exact
ROI, `{0}` temporal, Colour category. Continuous everywhere, so the §1.6 oracle holds to ≤ 2 fp16
ULP (measured worst 1); Mix 0 is the bit-exact identity, pinned by test. The two colours render
through the inspector's existing `ParamKind::Colour` arm — no inspector change needed. The fuller
shadows/mids/highlights Tritone is a Tier 2 follow-up (§4). Built in an isolated worktree; not
pushed.
**K-128 · DECIDED · Depth of field gains depth invert, separate near/far blur, and Display views
(docs/08 §3.22).** Three owner-requested additions modelled on Frischluft / DOF PRO. (1) **Depth
invert** (bool, default off): inverts the depth (`d' = 1 − d`) before the circle-of-confusion,
swapping near and far. (2) **Near/Far blur** (px@comp, default 8, slider 0–40): per-side maximum
circle-of-confusion — depths in front of focus (`d < focus`) use Near, the far side Far. The
existing **Aperture** is retained as a **master** that scales both about its default 8 (unity:
`radius · Aperture / 8`), so the near/far select flips only where the smoothstep `s` is zero (at
`d = focus`) and the radius stays continuous. (3) **Display** (choice, default Rendered):
diagnostic views — Rendered (the blur), Depth map (post-invert greyscale), Focus map (the smooth
`1 − s` in-focus mask); Depth/Focus map short-circuit before the gather and ignore Mix. All three
are threaded through `Resolved::Dof` (still `Copy`), the resolve arm, the CPU oracle, `DofParams`,
`FxEngine::dof` and `fx_dof.wgsl`; the UI renders the new Bool/Float/Choice params automatically
and the frame key hashes them via the effect-stack feed with no change. **Back-compat:** old
`dof` instances lack the new params, so Depth invert reads off, Display reads Rendered, and
Near/Far fall back to Aperture (both sides `8 · Aperture/8 = Aperture`), rendering identically.
Every shipped mode is continuous, so the §1.6 ULP oracle covers invert on/off, asymmetric near/far,
and each Display mode with no exclusion (worst 1 fp16 ULP on the RTX). Built in an isolated
worktree.
**K-129 · DECIDED · User-preset library and browser (docs/07 §7).** Effect presets (K-065)
gain a browsable home: a **Presets** group at the top of the Effects & Presets panel lists the
`.lumfx` files in a single preset library — `directories::ProjectDirs::from("dev","Lumit","Lumit")
.data_dir().join("presets")`, i.e. the platform roaming app-data folder, shared across projects
(alongside the existing `media_index_dir`/`journal_path` helpers in `lumit-project`). The folder is
created lazily and scanned live each paint (cheap for a small library), so a just-saved preset
appears at once; a missing or unreadable folder yields a hint, never a panic. Each entry's label is
the preset's own `name`, falling back to the file stem when the file can't be parsed, and the list
sorts case-insensitively by that label for stability between paints. A **click** applies the
preset, appending its saved stack with fresh instance ids to the selected layer as one undoable
`SetLayerEffects` — the same append the inspector's "Load preset…" already commits (K-065); with no
layer selected the click surfaces a status hint. "Save stack as preset…" defaults its rfd dialogue
to this folder so saving and browsing share one home, while still allowing the user to navigate
elsewhere. The scan/label/sort and load-with-fresh-ids logic are pure helpers (`preset::list_presets`,
`preset::load_instantiated`) with unit tests. Drag-a-preset-onto-a-layer, favourites, and preset
thumbnails (§7) remain later steps. Built in an isolated worktree; not pushed.
**K-130 · DECIDED · Scopes trace the live frame during playback from the CPU cache (docs/07
§8, extends K-096).** K-096 shipped scopes that updated only while paused/scrubbing and held the
last frame during playback, deferring live tracing to a GPU-side scope pass. This lifts that for
the common case without a new readback or any change to the render loop: the Scopes panel reads the
composited frame **under the playhead** (`comp_frame_cache.peek(frame_key_for(preview_frame))`, the
same frame the eyedropper reads) **every paint**, and while `app.is_playing()` requests
`request_repaint_after(16ms)` so it re-samples at the playback cadence. Because playback already
banks frames ahead (prefetch) and warms the work area when idle, the frame under the playhead is
normally cached, so the scope tracks live end to end. When it is not yet banked — a frame the budget
readback skipped, or one still rendering — the pane **holds the last frame it showed** (its key kept
in egui temp memory, re-validated against the cache so an evicted key never dangles) instead of
blanking, matching §8's "degrade the update rate under load". `request_repaint_after` (not a bare
`request_repaint`) is used deliberately so the panel never shortens the frame delay to zero and
never busies an idle-paused UI (the `is_playing` guard) nor spins faster than playback. The frame
choice is a pure `shown_frame_key` helper with a unit test. Guaranteed every-frame tracing under all
conditions (a cold, unwarmed comp) still waits on the GPU-side scope pass K-096 named; this is a
strict improvement over "holds during playback", not that pass. No change to the playback loop,
banking, or GPU code. Built in an isolated worktree; not pushed.

**K-131 · DECIDED · Temporal re-render effects share one `render_below_at`; Posterize time
(everything-below) ships first (docs/08 §3.25, docs/impl/temporal-rerender.md).**
Posterize time and (next) accumulation motion blur are not per-pixel effects — they change
*what time the layers below them render at*, so they live at the frame-orchestration layer, not
`run_ops`. Both re-render the below-stack at a changed time through **one** shared helper,
`render_below_at` = `build_comp_draws` at the held/sample time (reusing the SAME held decoded
pixels — footage is held, only transforms/effects/camera re-resolve) → a shared `Realiser`
(the GpuViewer compositor factored behind a borrowed handle so export can drive it too). Both
the preview comp-render entry and export's `render_comp_linear` call it, so preview equals
export pixel-for-pixel (K-031). Proved by a still-scene identity test (a re-render at the same
time is bit-identical to no re-render) and a moving-scene test (a full-coverage posterised
frame equals a plain render at the held time). Posterize time is an **adjustment** effect
(Everything below scope) detected on the adjustment layer; a Posterize effect resolves to no
op, so the detection — not the resolved stack — keeps such an adjustment live, and its held
below composites in place of the plain below-composite before the coverage blend. **Held-time
maths** `floor((t − phase)·rate)/rate + phase` (rate ≤ 0 holds nothing, never divides by
zero). **Boundaries (v1):** temporal effects inside the held below-stack (echo, flow motion
blur, datamosh) degrade to stills (the held re-render carries no neighbour decode, matching the
after-effects matte, K-125); a Posterize adjustment inside a collapsed Precomp is a no-op (its
held draws are sized for the nested comp); *This layer's effects* scope and the held-time cache
dedup are tracked follow-ups (the schema and maths are already in place). Built in an isolated
worktree; not pushed.

**K-132 · DECIDED · A held/sub-frame temporal re-render honours the per-effect
`sample_temporally` flag (docs/08 §3.25, docs/impl/temporal-rerender.md §5).** In a Posterize
time (and, next, accumulation motion blur) re-render, an effect on a below-layer flagged
`sample_temporally == false` resolves at the true frame time `t`, not the held/sample time `τ`,
so a particle system or other costly/stochastic effect is not re-run per held sample while the
rest of the scene (transforms, camera and the sampling effects) moves to `τ`. Implementation:
`lumit_core::fx::resolve_stack_temporal(effects, sample_lt, frame_lt, …)` shares `resolve_one`
with `resolve_stack`, handing each effect `frame_lt` when its flag is false and `sample_lt`
otherwise — so `sample_lt == frame_lt` is byte-identical to `resolve_stack` and the ordinary
(non-temporal) render is unchanged. `build_comp_draws` is now a thin wrapper over
`build_comp_draws_at(doc, comp, t_comp, frame_t, …)`, which threads the playhead `frame_t`
through nested Precomps and into `posterize_below`/`below_draws_at`/`render_below_at`; each
layer's own stack resolves through `resolve_stack_temporal`. Preview and export drive the one
threaded path, so they stay identical (K-031). The after-effects matte/depth sources keep their
own K-125 temporal boundary. Concurrent-worktree risk: another agent may also claim K-132 —
renumber on merge if so. Built in an isolated worktree; not pushed.

**K-133 · DECIDED · Posterize time *This layer's effects* scope ships: a per-layer effect-time
hold (docs/08 §3.25, docs/impl/temporal-rerender.md §4).** The second Posterize scope holds only
the layer's **own effect stack** on the coarse grid — its transform and source stay live, so
the layer moves smoothly while its effect animation steps. No re-render of other layers, no
orchestration re-entry (the simple cousin of *Everything below*). The held effect time is
`lumit_core::fx::this_layer_effect_time(effects, fx_on, lt, start_offset)` — the grid computed
on comp time `lt + start_offset` (matching *Everything below*'s comp-time hold), mapped back
into the layer's own base, and `lt` unchanged when the stack has no live *This layer* Posterize.
Both `build_comp_draws_at` (preview) and export's `apply_fx` compute it and feed it to
`resolve_stack_temporal` as the sample time (with `lt` as the frame time, so a
`sample_temporally == false` effect still resolves at the live playhead, K-132), so preview
equals export (K-031). With no this-layer Posterize this is byte-identical to the previous
`resolve_stack`, so ordinary layers are unchanged. Concurrent-worktree risk: another agent may
also claim K-133 — renumber on merge if so. Built in an isolated worktree; not pushed.

**K-134 · DECIDED · Accumulation motion blur ships: the second temporal re-render effect
(docs/08 §3.26, docs/impl/temporal-rerender.md §3).** The expensive, correct motion blur — it
re-renders the whole scene below at N sub-frame times and averages the finished frames, so
footage motion, animated effects, depth passes and the camera are all correct per sample (no
blurred-depth artefact). An **adjustment** effect detected exactly as Posterize is; it resolves
to no per-pixel op, so the detection keeps the adjustment live. The sub-frame times reuse the
per-layer motion-blur shutter maths (`MotionBlur::sample_offsets`, so `τ_k = t + off_k·dt`) via
`lumit_core::fx::stack_accumulation_mb` → `AccumulationMbParams`. The combine is a **new** GPU
pass, `Compositor::accumulate(&[(&Texture, weight)])` over a premultiplied-passthrough fragment
`fs_accumulate` (the inputs are already-premultiplied comp composites, so — unlike per-layer
`motion_blur_average`, which premultiplies a straight-alpha source — it must NOT re-premultiply);
colour AND alpha add, so a static scene is unchanged. Preview (`Realiser::accumulate_below`) and
export both render the N sub-frames through the one shared `render_below_at`, average at `1/N`,
then blend the average against the frame-time below by Mix (a second weighted `accumulate`, a
pure linear interpolation), so preview equals export (K-031). Proved by a still-scene bit-identity
test (`1/N` is exact in fp16 for a power-of-two N, the N copies sum back exactly) and a
moving-scene coverage-widening test. Params: Samples N, Shutter angle, Shutter phase, Mix; cost
Heavy (≈ N× a full comp render). Honours the per-effect `sample_temporally` flag (K-132) via the
shared `below_draws_at` threading. **Boundaries (v1):** temporal effects inside the sampled
below-stack hold to stills (K-125); an accumulation adjustment inside a collapsed Precomp is a
no-op (its sampled draws are sized for the nested comp); it takes precedence over Posterize when
an adjustment somehow carries both; sub-frame sample-count reduction under draft/scrub is a
tracked follow-up (full N always on export). Concurrent-worktree risk: another agent may also
claim K-134 — renumber on merge if so. Built in an isolated worktree; not pushed.

**K-135 · DECIDED · Effect parameter ranges prefer real/pixel units with open ceilings over
0–1 or percentage caps.** From the owner (2026-07-19). Unless a parameter's name carries a `%`
or a 0–1 ratio is genuinely its natural unit (a "roundness" that is literally how-circular, an
opacity, a mix), a built-in effect parameter should read in real or pixel units with a
one-sided `0..∞` (or wider signed) hard range rather than a 0–1 or fixed-percentage cap — the
maths almost always extrapolates cleanly past the old cap, and an editor should not hit a wall
wanting more. This continues the K-090 one-sided-range amendment, applied as a sweep across the
shipped grade/stylise effects:
- **Saturation** (§3.10) — the hard ceiling is lifted (`hard: (Some(0.0), None)`, slider to
  400 %). The luma/colour mix already extrapolates past 200 %; the CPU reference and WGSL
  kernel never clamped it, only the resolver did.
- **Vignette Softness** (§3.14) — lifted to `hard: (Some(0.0), None)`, slider to 2, kept in the
  normalised distance metric (not converted to pixels). The metric itself is not capped at 1
  (a corner reaches ~√2 under circular roundness), so a Softness beyond 1 is a legitimately
  wider feather; Amount/Radius/Roundness keep their 0–1 caps.
- **Temperature** (§3.20) — slider widened to ±150, hard to ±200, and the per-unit gain
  strengthened from `0.5·k` to `0.75·k` (`k = Temperature ÷ 100`, clamped to ±2) so full
  deflection is a decisive orange/blue; the gains floor at 0 (`max(0, …)`) so an extreme never
  drives a channel negative. 0 stays the bit-exact neutral point; CPU/GPU parity is preserved
  (gains computed host-side, as before).
- **Glow** (§3.3) — default Threshold lowered to 0.8; the **Knee** parameter's UI label renamed
  to **Softness** (the stable id stays `knee`, so saved projects and expressions are
  unaffected); **Radius** converted from % diag to **px@comp** with `hard: (Some(0.0), None)`
  (slider to 200, default 24 px), scaled by the preview factor like every px@comp parameter;
  the effect's ROI becomes `full-frame` since an unbounded px radius cannot be bounded as a
  %-diag padding (mirroring Chromatic aberration's own px@comp choice).

The changes touch schema ranges/labels and the resolve step (clamps and the glow radius unit +
the temperature gain formula) only; the CPU oracles and WGSL kernels are unchanged (they never
clamped), so K-031 preview/export parity holds automatically. Regression tests widen to exercise
the un-capped values and the temperature floor. Concurrent-worktree risk: another agent may also
claim K-135 — renumber on merge if so. Built in an isolated worktree; not pushed.

**K-136 · DECIDED · Hue shift gains a Preserve-luminance toggle (default on).** From the owner
(2026-07-19). The Hue shift effect (§3.17) adds a `preserve_luminance` bool, defaulting **on**,
which keeps today's behaviour: a constant-luminance rotation weighted by Rec.709 luma, so
perceived brightness stays put as the hue turns (a project saved before the toggle reads it as
on). **Off** switches to a plain-RGB spin about the neutral grey axis with equal weights, which
preserves the raw R+G+B sum rather than perceived luminance, letting brightness ride with the
hue. Both modes are the same SVG-`feColorMatrix` construction differing only in the luma
weights, so the resolve step simply picks which host-computed matrix
(`lumit_core::fx::hue_matrix` vs `hue_matrix_rgb`) to carry; the matrix-general CPU reference
and WGSL kernel are unchanged and stay in lock-step (K-031). 0° is the bit-exact identity in
both modes. Note for the record: the preserve-on mode is a Rec.709-weighted **linear-RGB**
rotation — the *spirit* of K-034's "hue-type operations convert through Oklab" (hold lightness,
turn hue) reached cheaply, not a literal Oklab/OkLCh rotation; a true-Oklab hue mode remains
possible future work. Concurrent-worktree risk: another agent may also claim K-136 — renumber
on merge if so. Built in an isolated worktree; not pushed.

**K-137 · DECIDED · The Blur effect splits into three: Gaussian, Directional, Radial.** Applies
K-090's "one effect, one job" to the blur family: the single mode-driven "Blur" effect (a Mode
dropdown selecting Gaussian / Directional / Radial, with every mode's parameters present at
once) becomes three separate effects in the **Blur & sharpen** category — **Gaussian blur**,
**Directional blur** and **Radial blur**. The maths, WGSL kernels and CPU oracles are untouched
(the `Resolved::Blur` / `DirBlur` / `RadialBlur` variants and their `blur` / `dir_blur` /
`radial_blur` kernels stand); only the schema and the resolve arms that read it changed.
Consequences: **Gaussian keeps match_name `blur`**, so a project saved with the old combined
effect loads as Gaussian at its stored Radius (whatever Mode it saved — the now-unread
mode/length/centre params are ignored); Directional (`directional_blur`) and Radial
(`radial_blur`) are new match names. The Mode parameter is gone. **Length** (Directional) and
**Amount** (Radial) become **hard-unbounded above** (sliders to 200 and 100 respectively) now
each is its own effect rather than sharing the family's reach — cost stays bounded because the
tap counts clamp (`cpu::dir_blur_taps` / `radial_blur_taps`). The shared **Edges** control
(Transparent / Repeat / Mirror) is kept **only on Radial**; Gaussian and Directional resolve at
the old default, Repeat, so their look is byte-unchanged. Add-effect menu, command palette and
preset paths are all BUILTINS-driven, so the three appear automatically. Spec:
[08-EFFECTS.md](08-EFFECTS.md) §3.8. Built in an isolated worktree; not pushed.

**K-138 · DECIDED · The Sharpen effect is really an unsharp mask; a plain Sharpen joins it.**
The v1 "Sharpen" effect (§3.9) was an unsharp mask (gaussian-based detail lift with Radius /
Threshold / luminance-only). K-138 renames its **label** to **Unsharp mask** — match_name stays
`sharpen`, so saved projects are unchanged — and adds a separate, single-purpose **Sharpen**
(match_name `sharpen_simple`): a fixed 3×3 high-pass convolution scaled by one **Amount**
(`out = u + amount·(4·u − up − down − left − right)` per RGB channel, clamp-addressed
neighbours), on unpremultiplied colour (§2.2), alpha kept. Amount 0 (whatever the Mix) and Mix 0
are the bit-exact passthrough (the kernel and CPU reference both short-circuit). Full 4-site
build: schema (`builtins.rs`), `Resolved::SharpenSimple` + resolve arm (`resolved.rs`), CPU
reference `cpu::sharpen_simple` (the oracle), the `fx_sharpen_simple.wgsl` kernel dispatched
from `run_ops`, and the `wgsl_sharpen_simple_matches_the_cpu_oracle` parity test (cheap class,
≤ 2 fp16 ULP). Both effects sit in **Blur & sharpen**. Spec: [08-EFFECTS.md](08-EFFECTS.md)
§3.9. Built in an isolated worktree; not pushed.

**K-139 · DECIDED · The accumulation temporal effect is *the* "Motion blur"; it gains "Force on
all layers" (docs/08 §3.26).** The accumulation re-render effect (K-134) is renamed from
"Accumulation motion blur" to plain **Motion blur** — the correct, whole-scene kind takes the
user-facing name — and the optical-flow effect (§3.2) is renamed to **Fast motion blur** so the
two never collide (the per-layer transform motion-blur *switch*, K-120, is untouched — it is a
layer switch, not an effect). New bool parameter **Force on all layers** (default off): during
each sub-frame sample render every layer's own per-layer motion blur (K-120) is forced on, the
effect's own Shutter angle/phase/Samples standing in for the comp master and each layer's switch,
so one effect blurs every moving layer without toggling each one and each accumulation sample is
itself transform-smeared (smoother at low sample counts). Implemented WITHOUT mutating the comp:
`AccumulationMbParams::forced_layer_mb()` hands a `MotionBlur` to `below_draws_at`, which drops
it onto the sample render's cloned comp master and every layer switch — the document and the
live-below composite are untouched, and preview and export drive the identical forced sample
render (K-031). Boundary: the force reaches the top-level below layers; nested-Precomp inner
layers keep their own switches (a v1 follow-up). Renaming is label-only — the `accumulation_mb`
/ `motion_blur` match names and saved projects are unchanged. Concurrent-worktree risk: another
agent may also claim K-139 — renumber on merge if so. Built in an isolated worktree; not pushed.

**K-140 · DECIDED · Fast motion blur scales the streak by a smooth confidence, not a hard gate,
and gains a View enum (docs/08 §3.2, docs/impl/optical-flow.md §4).** The optical-flow motion
blur (§3.2, renamed to **Fast motion blur** in K-139) left hard un-blurred cut regions wherever
the patch-based flow was unreliable (occlusions, motion boundaries). Fix: the decode worker now
computes a per-pixel **confidence** in 0..1 alongside the flow — `lumit_flow::confidence(fwd,
bwd)`, the raw forward–backward consistency mapped 1 (agree) … 0 (disagree, at the same rel/abs
scale the binary occlusion cut uses; an invalid patch fully suspect), 3×3 box-blurred so the
taper has no seam — and the kernel scales each pixel's **streak length** by it (`sv = flow ·
shutter_frac · conf`). Suspect regions fade toward unblurred smoothly instead of cutting;
confidence 0 is a bit-exact passthrough for that pixel, composing with the existing zero-motion
and zero-shutter passthroughs. The confidence rides in a new `.z` channel of the flow texture
(now `rgba32float`, not `rg32float`; Datamosh shares it and reads only `.xy`, so it is
unaffected). New **View** enum parameter (*Rendered* | *Motion vectors* | *Confidence*, default
Rendered): the diagnostic views output the flow colour-coded or the confidence as greyscale.
Full CPU/GPU parity is kept — `lumit_core::fx::cpu::motion_blur` gains matching `conf`/`view`
arguments and stays op-for-op with `fx_motionblur.wgsl` at the cheap-class ≤ 2 fp16 ULP oracle
bound; preview and export compute confidence with the identical deterministic function (K-031).
Concurrent-worktree risk: another agent may also claim K-140 — renumber on merge if so. Built in
an isolated worktree; not pushed.

**K-141 · DECIDED · Comp playback audio is kept in step with the document by a per-frame
signature, not baked once (GEN-4 audio fixes).** The comp mix (`export::mixdown` of the
audible footage layers, laid on the strip by `lumit_audio::mix::place_on_timeline`) was baked
into one flat buffer when playback started and never revisited, so muting, moving, trimming or
deleting an audio layer had no effect on what played — the four owner-reported GEN-4 bugs.
Fix: beside the loaded mix Lumit stores a **signature** (`audio_jobs_signature`: the ordered
contributing layers with their in/out/offset, plus the comp length). Each UI frame
`sync_comp_audio` derives the current jobs from the live snapshot and, via the pure
`comp_audio_sync`, either leaves a matching mix alone, re-bakes a stale one, or **unloads** a
mix whose comp has fallen silent (every audio layer muted or deleted) so it stops sounding at
once. `toggle_play` replays the loaded mix only when its signature still matches; otherwise it
re-bakes. Deliveries from the background bake carry their signature and are dropped by
`poll_comp_audio` if a newer edit has superseded them, so a stale mix never lands. Muting stays
a decode-skipping filter in `comp_audio_jobs` (a muted layer is never decoded); the signature
machinery makes that filter, and the placement, take effect live. Cost: one cheap hash of a
handful of layers per frame while a comp's audio is managed (loaded, in flight, or playing);
idle comps are untouched. A full per-audio-block re-mix from cached decoded sources (so edits
apply with zero re-decode latency) is the natural next step but was deferred as a larger
refactor of the single-baked-buffer engine. Built in an isolated worktree; not pushed —
another agent may also claim K-141, renumber on merge if so.

**K-142 · DECIDED · Layer-input source is a three-way combobox, not a before/after bool
(revises K-125).** A track matte's source and an effect's Layer-reference input (the Depth of
field depth layer) each replace K-125's "after effects" bool with a **source** combobox beside
the layer picker offering **None** (the referenced layer's raw footage/solid — no masks, no
effects), **Masks** (its source plus its own masks, no effects) and **Effects and masks** (its
finished picture — the source's effects and masks run in first; K-125's `after_effects = true`).
A shared `LayerInputSource { None, Masks, EffectsAndMasks }` (lumit-core) carries the semantics:
`applies_masks()` gates the source's masks, `folds_effects()` runs its stack. `None` samples the
source with its masks **cleared** (a masks-stripped clone through the same `pixels_for`/`prepare`
the preview and export already share, so preview == export, K-031); `Masks` and `Effects and
masks` reuse the existing source-only and after-effects paths. Storage: the matte carries
`MatteRef::source` (replacing `after_effects`), migrated on load by a serde shim
(`after_effects: true` → `EffectsAndMasks`, `false` → `Masks`, absent → the default); a layer-input effect
carries a sibling `<id>_source` Choice, read by `EffectInstance::layer_source`, which falls back
to the legacy `<id>_after_effects` bool so old DoF projects still key and render correctly
(the removed `depth_after_effects` schema param). The frame key hashes the mode discriminant in
place of the old bool byte (0/1/2), so switching modes retires stale frames, and still folds the
source stack only for `EffectsAndMasks`. **Default and migration (owner-decided):** a new
matte/depth input defaults to **Effects and masks** — the most complete source is the sensible
default. Because the historical source-only path (`after_effects = false`) already applied the
referenced layer's *masks* (via the shared `pixels_for`), the faithful migration of the old bool
is `true → EffectsAndMasks`, `false → Masks` (so no masks are dropped); a matte predating both
fields takes the default. The v1 temporal boundary is unchanged (echo/flow on the source still
degrade to a still).

**K-143 · DECIDED · A reusable three-colour channel picker, and RGB split gains per-channel
amounts.** From the owner (2026-07-19), the P2 + FX-9 channel-split work.
- **The three-colour channel picker (P2).** A small reusable inspector widget shows three
  colour swatches (defaults red / green / blue), each opening the colour picker, for effects
  that split a picture into three tinted channels. It is convention-driven: any effect whose
  schema declares three `ParamKind::Colour` parameters named `channel_colour_1`,
  `channel_colour_2`, `channel_colour_3` renders them as one compact swatch row instead of
  three separate colour rows — the widget (`shell::inspector::channel_picker`) finds the group
  by those ids, so a future three-tinted-channel effect adopts it with no new UI code. The three
  colours are ordinary scene-linear Colour parameters, so they serialise and animate through the
  existing model unchanged. First adopter: Chromatic aberration (K-144).
- **RGB split per-channel amounts (FX-9).** RGB split (§3.6) gains three per-cent scales —
  **Red** / **Green** / **Blue** (defaults 100 / 0 / 100, hard-open both sides per K-135) — that
  multiply the overall Amount per channel: R and G displace along −offset, B along +offset, so
  the defaults reproduce the classic split bit-for-bit while letting R and B fringe by different
  amounts (or G leave its anchor). They apply to the classic (non-Wavelength) mode only.
- Build: `Resolved::RgbSplit` gains a `scale: [f32; 3]`; the CPU reference
  (`cpu::rgb_split`), the `fx_rgbsplit.wgsl` kernel and the `RgbSplitOp` carry it, and green
  is now read through the same `bilinear` sampler as R and B (at scale 0 it lands exactly on
  its own pixel, so the classic look is byte-identical). CPU/GPU parity and the
  `wgsl_rgb_split_matches_the_cpu_oracle` test hold (K-031). Built in an isolated worktree; not
  pushed — another agent may also claim K-143, renumber on merge if so.

**K-144 · DECIDED · Chromatic aberration adopts the channel picker and RGB split's Wavelength
machinery; the spectral dispersion becomes a user-controlled variable-sample count.** From the
owner (2026-07-19), the FX-10 + FX-9 spectral work.
- **Chromatic aberration (§3.15)** becomes three tinted radial taps at offset fractions −1 / 0 /
  +1, each sampled and multiplied component-wise by one of the K-143 channel colours and summed.
  Defaults red / green / blue keep only their own channel, reproducing the historical
  R-outward / B-inward / G-anchor split bit-for-bit; the three colours are edited through the
  reusable picker (K-143). It also gains a **Wavelength** Bool + **Samples** control that reuse
  §3.6 RGB split's own spectral machinery — turning Wavelength on resolves the effect to a radial
  `SpectralSplit`, so no second dispersion kernel exists. The channel colours apply to the
  non-Wavelength mode only.
- **Variable-sample spectral dispersion (FX-9/FX-10).** The Wavelength mode of both RGB split and
  Chromatic aberration carries a **Samples** count (`3..=64`, default 16, replacing the fixed nine
  taps). More taps fill the same `±offset` span more densely, so a large offset disperses as a
  smooth rainbow instead of a few discrete stacked copies. The taps — each a column-normalised RGB
  weight plus its offset fraction — are resampled from the nine `SPECTRAL_BASIS` anchors host-side
  (`fx::spectral_taps` / `spectral_basis_uniform`) and shared by the CPU reference and the WGSL
  kernel (which reads each tap's offset fraction from the vec4 `w` lane), so a uniform image still
  passes through unchanged and preview equals export (K-031). The floor is 3, not 2, because two
  taps (the red and blue ends alone) carry no green weight. Legacy Wavelength instances saved
  before the control existed read the default 16, a denser look than the old nine.
- Build: `Resolved::SpectralSplit` gains a `samples: i32` (staying `Copy`; the taps are rebuilt
  from it on both paths); `Resolved::ChromaticAberration` gains `tints: [[f32; 3]; 3]`. The
  `SpectralSplitOp`/`fx_spectral.wgsl` uniform carries a fixed 64-entry tap array plus a `count`;
  `ChromaticAberrationOp`/`fx_chromatic.wgsl` carries the three tints. Full 4-site + oracle
  (`wgsl_spectral_split_matches_the_cpu_oracle`, `wgsl_chromatic_aberration_matches_the_cpu_oracle`,
  cheap class, ≤ 2 fp16 ULP). Built in an isolated worktree; not pushed — another agent may also
  claim K-144, renumber on merge if so.

**K-145 · DECIDED · Two reusable effect-UI primitives: a shared Edges mode enum (P3) and
schema-driven collapsible parameter groups (P4).** Factored out so effects stop re-deciding
two recurring shapes:
- **`EdgesMode { Transparent, Repeat, Mirror }`** (`lumit-core::fx`) names the one edge
  policy a transform- or displacement-domain effect applies to the border its warp reveals.
  The blur family and Shake already spoke it as loose 0/1/2 `u32` codes plus an
  `EDGE_OPTIONS` string slice; the enum makes that vocabulary a type — `code()` /
  `from_code()` are the only bridge to the wire form the resolved ops and WGSL kernels read
  (the numbers are unchanged, so nothing re-serialises), and `EDGE_OPTIONS` is now
  `EdgesMode::OPTIONS`. Radial blur's resolve flows through it unchanged; new effects reuse
  it rather than inventing an edge meaning. The Transform *effect* itself stays
  transparent-only (it passes code 0), but its shared kernel — CPU `cpu::transform` and
  `fx_transform.wgsl` — gained an `edge` parameter so Shake can dispatch through it with any
  policy; `edge = 0` is bit-identical to the old transparent-only kernel (pinned by the
  transform oracle, which now sweeps all three modes).
- **`ParamGroup`** (a `label` + a contiguous run of member param ids + a `collapsed`
  default) is declared on `EffectSchema::groups`, and the Effect Controls panel renders each
  group under a disclosure "twirl" (reusing `group_header_row`, the same header a layer's
  Transform/Effects sections use), hiding its members when closed. Driven entirely from
  schema metadata, so any effect adopts a twirl by declaring a group — no per-effect UI
  code. Every existing schema declares `groups: &[]`. Spec: [08-EFFECTS.md](08-EFFECTS.md)
  §3.4/§3.8. Built in an isolated worktree; not pushed — another agent may also claim K-145,
  renumber on merge if so.

**K-146 · DECIDED · Shake reshaped: a per-axis wobble twirl, and Edges replaces Auto-scale
(FX-11).** The Shake effect (§3.4) keeps its master Amplitude / Frequency / Rotation amount
and gains a **Per-axis wobble** twirl (the K-145 P4 group) holding per-axis **x / y / z**
amount and frequency: x and y amount/frequency are dimensionless multipliers on the master
values (default ×1 reproduces the old uniform x/y shake bit-for-bit), and **z** is the
depth/scale shake — z amount is a scale-pump per cent that **replaces the old "Zoom pump"**
(same range and meaning), z frequency a rate multiplier. The **Auto-scale** bool is
**removed** and replaced by an **Edges** control (the K-145 P3 enum, default Repeat): the
resample's revealed border is now handled by the edge policy rather than by an automatic
cover-scale that zoomed in to hide it. Shake stays seeded and deterministic (§1.3/§2.4): the
generator (two octaves of value noise per axis) and the host-side affine → Transform-kernel
dispatch are unchanged, so with default per-axis values the resolved wobble is identical to
before; only the border treatment and the new z/frequency biasing differ. **Migration:** a
project saved before FX-11 has its `zoom_pump` read as the z amount and its `auto_scale`
read as the Edges control (on → Repeat, off → Transparent) via resolve-time fallbacks, so
saved shakes keep their pump and never sprout a transparent border unexpectedly; the
Auto-scale cover behaviour itself is gone (an intentional change — the wobble no longer
zooms to hide edges). CPU/GPU parity and the §1.6 oracle hold across all three edge modes.
Spec: [08-EFFECTS.md](08-EFFECTS.md) §3.4. Built in an isolated worktree; not pushed —
another agent may also claim K-146, renumber on merge if so.

**K-147 · DECIDED · Scanlines collapses to a single Intensity (FX-13).** The Scanlines
effect (§3.12) previously carried two darken controls — **Intensity** (0–1) and **Darkness**
(%) — that multiplied into one darken amount (`eff_mult = 1 − Intensity × Darkness` on the
dark half), so two dials did one job. They collapse into a **single Intensity** (0–1 = *how
dark the dark lines get*: 0 the bit-exact passthrough, 1 takes the dark half to black); the
bright half is untouched and Line period / Roll speed / Interlace / Mix are unchanged. The
schema drops `scanline_darkness` and bumps the effect version 1 → 2. **Migration:** a project
saved with the old pair still carries its `scanline_darkness` param; the resolve arm folds it
in — the single Intensity resolves to the old `Intensity × Darkness` product — so the loaded
look is unchanged (pinned by `scanlines_migrates_old_darkness_into_intensity`). The kernel is
simplified (the dark half's base is black, band 0, so `eff_mult = 1 − Intensity`), keeping
CPU/GPU parity and the §1.6 oracle; Intensity 0 stays a bit-exact passthrough via the
early-return. Spec: [08-EFFECTS.md](08-EFFECTS.md) §3.12. Built in an isolated worktree; not
pushed — another agent may also claim K-147, renumber on merge if so.

**K-148 · DECIDED · Datamosh gains Streak length and an open Intensity ceiling (FX-14).**
The Datamosh effect (§3.12) was too subtle at its one-frame reach. Two changes: (1) the
**Intensity** hard cap lifts (K-135 value-range policy) — clamped at zero below, open above,
so a value over 1 extrapolates past the moshed frame for a punchier tear (`mix()` does not
clamp in either the CPU oracle or the WGSL kernel; 0 stays a bit-exact passthrough). (2) a new
**Streak length** (frames, default 4, hard min 1, open above) scales the flow displacement the
single warp reaches, so it predicts that many frames of motion from the -1 reference — the
accumulated smear of a long P-frame run before a clean reference frame (longer = more
smearing). The shared optical-flow texture stays `rgba32float`; only its `.xy` is read (the
`.z` confidence lane is untouched). The clean I-frame "reset" is content-driven — where the
flow is zero/unmeasurable (a still, a cut) the warp lands on the pixel itself; a
**fixed-interval** I-frame reset was considered but deferred, as it needs the comp frame index
threaded through `resolve_stack` (a broad signature change for one parameter) and Streak length
already delivers the "how much accumulated smear" control without it. The schema bumps version
1 → 2. **Migration:** an old project (no `streak_length` param) folds to the default 4-frame
reach — a deliberate look change (the effect was too subtle), the sanctioned kind K-146 also
took. CPU/GPU parity and the §1.6 oracle hold (the oracle sweeps streaks 1–4 and an over-unity
intensity). The `match_name` and label stay "datamosh" for now; a rename is wanted but
unchosen (candidate names proposed to the owner). Spec: [08-EFFECTS.md](08-EFFECTS.md) §3.12.
Built in an isolated worktree; not pushed — another agent may also claim K-148, renumber on
merge if so.

**K-149 · DECIDED · Echo gains the standard blend modes (default Screen) and a 16-echo cap
(FX-17).** The Echo effect (§3.13) previously offered three combine modes (Add / Behind / Max)
and reached at most 8 frames back. Two changes: (1) **Mode** now mirrors the comp blend set —
Normal, Add, Multiply, Screen, Overlay, Soft light, Hard light, Lighten (the legacy Max),
Darken — plus the echo-specific **Behind** (ghosting), with the **default changed to Screen**.
Each mode folds the weighted echo tap into the trail **per channel in the working linear
premultiplied space** — not the compositor's perceptual sRGB domain, because Echo composites
light trails (linear is correct there) and a single arithmetic domain keeps the CPU oracle
(`cpu::echo_blend`) and the WGSL `echo_accumulate` bit-for-bit identical. The legacy Choice
indices 0/1/2 (Add/Behind/Max) are held and the new modes appended, so a project saved before
FX-17 loads unchanged; only new instances default to Screen. (2) The **echo-count cap rises
8 → 16**: the static `temporal` window and the resolved/kernel weight arrays grow to 16
(`[f32; 16]`), so up to 16 neighbour frames are decoded when Echo is live — a Spacing control
and a dynamic window (the eventual 1–32 of the spec's parameter line) remain later
refinements. The schema bumps version 1 → 2. CPU/GPU parity and the §1.6 oracle hold: the
oracle sweeps every mode (the additive trio two-tap at ≤4 fp16 ULP, the
multiplicative/perceptual modes single-tap at ≤8, the looser bound justified by their local
slope amplifying the fp16 quantisation of the current frame against HDR neighbours — still
orders of magnitude tighter than any formula error). Spec: [08-EFFECTS.md](08-EFFECTS.md)
§3.13. Built in an isolated worktree; not pushed — another agent may also claim K-149,
renumber on merge if so.

**K-150 · DECIDED · A new layer's transform centres its anchor on its own content (FX-20).**
A freshly added layer defaults its **anchor** (origin) to the centre of its *own* pixel
content and its **position** to the composition centre, so it appears centred and pivots
about its middle under scale and rotation — the After Effects default the glossary already
describes ([01-GLOSSARY.md](01-GLOSSARY.md) §2, "New layers default their anchor to the
centre of their content"). Sized per layer kind: **footage** by the footage's natural pixel
size (comp size until the probe lands), **precomp** by the nested comp's size, **solid** by
the `SolidDef`'s own size, **sequenced layer** by the comp (a "fancy precomp", K-071), and
comp-sized kinds (**adjustment**) by the comp. One private helper,
`AppState::centred_transform(nat_w, nat_h, comp_w, comp_h)`, is the single wiring point every
add-layer path routes through, so the rule cannot drift between kinds. Two deliberate
exceptions: a **camera** is a viewpoint, not a picture, so it keeps position at the comp
centre with no content anchor; a **text** layer keeps its origin at the text insertion point
(anchor 0,0) because its content size is only known after glyph layout, matching AE's
point-text convention. Only *new* layers default this way — saved projects load their stored
transforms unchanged (the transform is serialised in full). Added 2026-07-19 at the owner's
request. Built in an isolated worktree; not pushed — another agent may also claim K-150,
renumber on merge if so.

**K-151 · DECIDED · Blend modes gain Darken and Subtract (GEN-1).** The layer blend-mode set
adds **Darken** (`min(dst, src)` per channel) and **Subtract** (`dst − src` per channel,
clamped at black). Darken is domain-invariant (per-channel min commutes with the monotone
transfer function) and runs in linear alongside Lighten. Subtract runs in **linear light** —
it is Add's darkening twin, the physical removal of light — not in the encoded/perceptual
domain, and clamps at zero so it never produces negative light. Both take the compositor's
snapshot path (like Screen and the per-channel min/max modes), so layer opacity and mattes
mix by coverage correctly; the premultiplied-alpha maths is the shared
`rgb = mix(dst, blended, a)`, `a_out = a + dst_a·(1−a)` every snapshot blend uses. Darken was
already present in the enum, the UI dropdown and both GPU mappings when this work began; GEN-1
adds Subtract to match. CPU/GPU parity holds (the compositor's inline oracle tests pin each
mode's formula). Spec: [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §3.5. Added 2026-07-19 at
the owner's request. Built in an isolated worktree; not pushed — another agent may also claim
K-151, renumber on merge if so.

**K-152 · DECIDED · Vibrancy, a saturation-aware colour effect (GEN-2).** A new **Colour**
effect complementing Saturation (§3.10): where Saturation scales colourfulness uniformly,
**Vibrancy** raises it *more* for less-saturated pixels and *less* for already-vivid ones, so
near-neutrals and skin tones lift while saturated regions are protected from clipping. One
**Amount** dial (per cent): 0 is the neutral, bit-exact identity; the slider reaches a heavy
200 and typing higher pushes further (value-range policy K-135, open above, floored at 0). The
maths, in linear light on unpremultiplied colour (§2.2) exactly like Saturation: measure each
pixel's HSV-style saturation `sat = (max−min)/max` (clamped to 0..1, scale-invariant), form a
per-pixel factor `1 + amount·(1−sat)`, and scale colour about Rec. 709 luma by it, clamped at
zero and re-premultiplied. Built to the four-site pattern (schema → Resolved + resolve → CPU
reference oracle → WGSL kernel → the `wgsl_vibrancy_matches_the_cpu_oracle` parity test), so
preview equals export (K-031). Spec: [08-EFFECTS.md](08-EFFECTS.md) §3.10. Added 2026-07-19 at
the owner's request. Built in an isolated worktree; not pushed — another agent may also claim
K-152, renumber on merge if so.

**K-153 · DECIDED · Layers sit freely across the comp boundaries (GEN-3).** From the owner
(2026-07-19). A layer in the lane area may start **before comp time 0** (`in_point` and
`start_offset` may be negative) and end **past the comp duration** (`out_point` may exceed
it); only `out > in` is still enforced. The engine renders and plays a layer solely where its
span `[in_point, out_point)` **intersects the comp window `[0, comp_end)`** — out-of-window
frames are never sampled — so an over-hanging head or tail is carried without data loss and is
recoverable by sliding the layer. This is already how presence is gated (`t ∈ [in, out)` for a
`t` that only ranges over the comp window) in the evaluator, the preview job collector and the
exporter, and how audio places (`place_on_timeline` + `mix_stereo` clip a negative-offset head
and a past-the-end tail to `[0, comp_end)`); GEN-3 removes the *authoring* clamps that stopped
the model reaching those states. Consequences:
- The lane **move drag** no longer clamps a layer's start to 0 (`moved_span` converts through a
  sign-preserving `rational_at_signed`, not the ≥ 0 `rational_at`); frame/marker snapping is
  unchanged. Trim-edge and keyframe times stay ≥ 0 (layer-local times never precede 0).
- **Import never trims a long clip to fit.** A footage layer keeps its full media duration and
  a Precomp layer its full nested duration, positioned from the comp start
  (`add_footage_to_comp` / `add_precomp_to_comp`), instead of clamping the out point to the
  comp — matching the "layers extend beyond bounds without data loss" invariant in
  [03-DATA-MODEL.md](03-DATA-MODEL.md) §5.1. Preview == export and determinism are unaffected
  (the render/decode/audio paths already only sample the intersection). Known v1 limit: the
  timeline view does not scroll to negative time, so a bar that starts before 0 is drawn
  clipped at the lane's left edge (its in-window body stays grabbable). Built in an isolated
  worktree; not pushed — another agent may also claim K-153, renumber on merge if so.

**K-154 · DECIDED · Matte key becomes a Keylight-style colour-difference keyer (docs/08
§3.21, FX-21).** The K-121 chroma-distance key is expanded (same `matte_key` effect, version
1 → 2) into a proper greenscreen keyer with the strength/balance/clip/despill controls a
colourist expects from Foundry's Keylight. The screen matte is a **colour difference**, not a
distance: the **Screen colour**'s largest channel is the *primary* axis (green for a green
screen, blue for a blue one — a general improvement over the hue-agnostic distance metric),
the two others are *secondaries* blended by **Screen balance** into a reference, and a
pixel's `primary − reference`, normalised by the screen colour's own difference, gives `raw`
(1 on the exact screen, 0 on a neutral); `matte = clamp(1 − gain·raw, 0, 1)` with **Screen
gain** scaling the fall-off. **Alpha bias** subtracts a bias-colour neutral (grey ⇒ no-op) so
a tinted bias re-defines neutral; **Clip black/white** remap the matte's ends and **Clip
rollback** eases them back toward the un-clipped matte to recover fine detail. **Despill**
pulls the primary channel down toward the (**Despill bias**-shifted) secondary reference by
the **Despill amount**, draining screen tint; **Replace method** (Source / Hard / Soft /
None, default Soft) then recolours where spill was removed, Soft scaling the **Replace
colour** by the pixel's brightness. A **View** enum (Final result / Screen matte / Status)
lets the user see the matte they are pulling; the Status view is a *continuous* heat
(`4·m·(1−m)` tint) so it stays oracle-safe. It still runs on straight colour (§2.2,
`premultiplied: false`) and stays `cheap`/`exact`/`{0}`. Every step is `clamp`/`min`/`max`/
`mix` — **continuous everywhere** — and the screen's primary axis and reference are derived
from the resolved Screen colour identically on the CPU reference (`cpu::matte_key`, the
oracle) and the WGSL kernel (`fx_matte_key.wgsl`), so preview == export (K-031) and the §1.6
oracle holds to ≤ 2 fp16 ULP (test `wgsl_matte_key_matches_the_cpu_oracle`, sweeping gain /
balance / clips / despill / replace / bias colours and all three views over a near-screen /
far-from-screen / partial-alpha / HDR corpus). Colour, bias and replace swatches render
through the existing `ParamKind::Colour` inspector arm (each with the eyedropper); the Screen
matte controls sit in a K-145 `ParamGroup` twirl. There is **no neutral no-op default** (the
default green + 100 % gain keys out of the box, §1.2); **Mix 0 is the bit-exact identity**,
pinned by test. **Migration:** a project saved before K-154 keeps its stored `key` (Screen
colour) and `spill` (now Despill amount); its now-unread `tolerance`/`softness` are superseded
by gain/balance/clip, and the new controls take their Keylight defaults — resolve reads every
new parameter with an `unwrap_or(default)`, so no old project faults and none re-serialises
until edited. Distinct from the Tier 2 §4 keying suite (luma/screen key) still tracked
separately. Builds on K-121 (which it supersedes without editing). Built in an isolated
worktree; not pushed — another agent may also claim K-154, renumber on merge if so.

**K-155 · DECIDED · The spatial and layer-input Keylight controls are a deferred follow-up
(docs/08 §3.21).** The pointwise K-154 landing deliberately leaves out the Keylight features
that are *not* a single pointwise pass, so each can arrive with its own oracle rather than
being half-implemented: the **spatial screen-matte controls** — Screen pre-blur, Screen
shrink/grow (morphological erode/dilate), Screen softness (blur), Screen despot black/white
(speck removal) — which need a multi-pass morphology/blur pipeline and a costlier oracle
class; the **Inside/Outside garbage masks**, a layer-input holdout reusing the DoF
layer-reference plumbing (`ParamKind::Layer`, docs/impl/layer-input.md) with per-mask softness
and invert; the **Colour correction** twirls (Foreground and Edge: enable + saturation /
contrast / brightness / colour balance, Edge adding hardness / grow); and the **Source crops**
(per-axis edge method — Colour / Repeat / Reflect / Wrap — an edge colour, and Left / Right /
Top / Bottom crop amounts). None is required for "properly key footage" — the K-154 core
(screen matte + clips + despill + views) is — so they are ordered after it and tracked here.
When they land, each keeps the K-031 preview==export and §1.6 oracle guarantees. Numbered
K-155, alongside K-154; renumber on merge if another agent also claims it.

**K-156 · DECIDED · "Save stack as preset" saves the current selection, not always the whole
stack (docs/07-UI-SPEC.md §7, UI-10).** The Effect Controls → Presets "Save stack as preset…"
now writes exactly what the user has highlighted, decided by the existing selection model — the
effect-row selection (`selected_prop`/`selected_props`) and the lane keyframe selection
(`lane_selection`), both restricted to the layer being saved. The rule (pure, tested in
`preset::selection_subset`): with **nothing highlighted** it saves the whole stack, so the old
behaviour is the fallback; otherwise it saves **every effect the selection touches** — a
highlighted parameter row, or a highlighted key — in stack order, and within each of those
effects any Float parameter that has highlighted keys is **trimmed to just those keys**. A
parameter with no highlighted keys keeps its value exactly as set, including any full animation
the user did not single a key out of; a stale selection (a key edited away, an effect removed)
simply matches nothing and is skipped rather than emptying a parameter. Key times match exactly
on their stored rational, which is what the lane selection carries. The `.lumfx` format is
unchanged (a preset is still a list of `EffectInstance`s); pre-release, no migration is needed.

**K-157 · DECIDED · The Project panel's selected-item info box is fixed-height and shows a
footage thumbnail reused from the Viewer (docs/07-UI-SPEC.md §3.1, UI-4).** The info box keeps a
constant height (`PROJECT_HEADER_HEIGHT`) whatever is selected — drawn into a reserved,
clipped rect — so choosing different items never shifts the tree beneath it. For footage it
shows a small thumbnail on the left: the **Viewer's own decoded frame**, passed through to the
panel and drawn aspect-fitted, guarded so it is used only when that texture really is the
selected item's picture (`preview_comp` unset and `preview_item` equal to the item). No new
decode path is added (a dedicated proxy/thumbnail cache and hover-scrub, spec §3.1, stay a later
step); when no frame is to hand — still probing, a pop-out with no texture, or a non-media build
— a neutral placeholder carrying the footage glyph stands in. Paired with the panel-wide search
field (UI-3), which filters the tree live by name (case-insensitive substring, subtree-aware so
the path to a hit stays visible) per the existing spec §3.1 and needs no separate decision.

**K-158 · DECIDED · Every property row in the layer area — transform, effect and Retime —
shares one selection, keying and navigator model (owner parity rule: transform and effect
properties look and behave the same unless specified otherwise).** Four threads land together:
(1) **UI-1 — linked pair rows no longer clip their value boxes.** The Anchor/Position/Scale
rows carry a chain-link control plus one or two value boxes in the narrow outline column; the
boxes were shaved at the column's right edge. The fix caps each pair value box at a fixed
width (`PAIR_VALUE_W`) and tightens the row's inter-widget gap and button padding
(`pair_row_tighten`), so `[x][link][y]` fits without clipping; single-axis rows keep their
full-width box. Pixel-layout only, no model change. (2) **UI-6 — effect parameter rows and the
footage Retime "Time"/"Velocity" row join the transform rows' multi-select model.** All three
route their name/row click through one shared gesture (`prop_click_select`): a plain click
single-selects (and, for transform/Retime, opens the curve), Ctrl/Cmd toggles, Shift ranges
over the frame's draw order — which now records transform, Retime and effect rows alike, so a
range or a mixed set can span all three. A new `PropRow::Retime` variant names the single
per-layer Retime channel. The Effect Controls panel builds and resolves its own draw order each
frame, mirroring the Timeline, so the two panels never tread on each other's range resolution.
(3) A command-palette action **"Key selected properties"** (`AppState::key_selected_props`)
keys every selected row at the playhead in one undo step, each holding its current value —
transforms as `SetTransformProperty`, effects folded per layer into one `SetLayerEffects`, and
the Retime channel as a velocity-lens speed key (lens-independent and media-free, so a mixed
keying is deterministic). (4) **One shared `◄ ◆ ►` keyframe navigator** (`keyframe_navigator`
returning a `KeyNavAction` the caller commits) replaces the four drifted copies used by
transform single props, transform linked pairs, the Retime row and effects — the
Position/Anchor and Retime rows had kept the older `Keyframe`/`KeyframeAdd` glyphs instead of
the effect navigator's `KeyframeFilled`/`Keyframe` look. The only per-row deviation the shared
navigator supports is the Retime lens's structural endpoint keys (removal disabled there).
(5) The **Retime "Time" value drag now drives the live preview** like transform (`prop_edit`)
and effect (`fx_edit`) drags: a new `AppState::retime_edit` field carries the provisional
retime store, and — because a retime change alters *which source frame* is on screen rather
than how an already-decoded frame composites — the decode job builder overrides the layer's
retime with it and re-decodes, rather than re-compositing. Backwards compatibility is not
required (pre-release). Built in an isolated worktree; not pushed — renumber on merge if
another agent also claims K-158.

**K-159 · DECIDED · The Timeline outline and lane/graph areas scroll together in the layers
view but independently in the graph view (UI-8).** In the ordinary **layers view** the layer
outline (the left column of property/layer rows) and the lane area to its right share **one**
vertical scroll: a single wheel or scrollbar moves both, synced, so a row's outline controls
and its bar never drift apart. In the **graph view** the lane area becomes the curve editor,
which pans and zooms its own value axis on the wheel (K-079); the outline is therefore given
its **own** vertical scroll, its scrollbar pinned to the **right edge of the outline column**
(not at the far right, over the curve). The two are then fully decoupled — a wheel over the
curve never scrolls the layer list, and the list scrolls on its own bar or on a wheel over the
outline column. Mechanically this is the one lane `ScrollArea` capped to the outline's width in
graph mode and spanning the whole panel in the layers view: an egui scroll area only reacts to
the wheel over its own rectangle, so once it stops at the outline's right edge the curve's wheel
never reaches it, and the earlier stop-gap (freeing the curve's wheel by zeroing the shared
scroll's `smooth_scroll_delta`) is removed. The wheel's destination is decided by a small pure
router, `timeline_wheel_route`, unit-tested per mode. In the speed lens — which has no vertical
pan — a plain wheel over the curve simply does nothing, consistent with the decoupling (the list
still has its own bar). Refines K-079 (which established that the curve and the layer list scroll
on separate wheels) without reversing it; no other decision changes. Built in an isolated
worktree; not pushed.

**K-160 · DECIDED · The Flow input rate is a keyframeable value field, not a preset
dropdown.** From the owner (UI-11): the Flow group's **Input rate** (the conform fps of
K-095) becomes a numeric field the user types any rate into — with the usual stopwatch and
◄ ◆ ► keyframe navigator — replacing the Native + common-rates dropdown. It is **keyframeable
like any other property**, so the conform rate can ramp over the clip. Storage changes cleanly
(pre-release, no migration): `FlowParams.input_fps` moves from `Option<f64>` to an
`anim::Property`, read at frame time through the new `FlowParams::input_fps_at(lt)`; `0` (the
default, and any value that rounds to it) means **Native** — the source's own rate — so a
keyframe ramp from Native to a real rate resolves without a discontinuity. A plain Native rate
stays out of the serialised file (`skip_serializing_if`), so an un-animated Native flow clip
writes exactly as before. The frame-cache key hashes the value the property reads at each local
time (superseding the K-095 single hashed fps), so an animated rate keys each frame distinctly
and preview still equals export (K-031). This supersedes the "dropdown offers Native and common
rates" detail of K-095 (which stays otherwise intact — the conform semantics are unchanged).
Built in an isolated worktree; not pushed — renumber on merge if another agent also claims
K-160.

**K-161 · DECIDED · RGB split becomes a linear tinted-tap fringe; Radial mode is dropped;
it gains the shared three-colour picker (T17).** From the owner (testing T17): §3.6 RGB split
loses its **Radial** mode entirely — the always-radial shape is already owned by §3.15
Chromatic aberration (K-143/K-144), so the mode was redundant. In its place RGB split gains the
same reusable three-colour picker chromatic aberration carries (`channel_colour_1/2/3`, default
red / green / blue), tinting its three offset taps. The classic behaviour is preserved
bit-for-bit: each tap is now sampled in **full colour** and multiplied by its tint before the
three are summed, and with the default primary tints (`[1,0,0]`/`[0,1,0]`/`[0,0,1]`) that
reduces exactly to the historical channel-separated split (`split.r = tap0.r`, `split.g =
tap1.g`, `split.b = tap2.b`). The per-tap **Red / Green / Blue** displacement scales (FX-9,
K-143) stay, now labelled as scaling their like-numbered tint. `Resolved::RgbSplit` drops
`radial` and gains `tints: [[f32;3];3]`; the GPU `RgbSplitOp`/kernel lose the radial branch and
`amount_px`, gaining the three vec4 tints. Wavelength mode still resolves to `SpectralSplit`,
now always `radial: false`. Pre-release, no migration: instances saved with a `radial` param
simply ignore it, and instances without the tint params fall back to the primaries. This
supersedes the "Mode (Linear / Radial)" and radial Centre/Falloff detail of K-090's §3.6 (the
Wavelength quality tier and per-tap amounts are otherwise unchanged). The A1 report — that the
picker colours do nothing in **Wavelength** mode — is not addressed here for the spectral path:
`SpectralSplit` still uses the physically-based `SPECTRAL_BASIS`, so the picker governs the
classic mode only; whether Wavelength should also be driven by the picker colours is left open
(see §3.6 Open questions). Built on `main`.

**K-162 · DECIDED · The full After Effects colour-blend set ships in v1 (T24).** From the
owner (testing T24, "add ALL After Effects blend modes"): `BlendMode` grows from the ten-mode
v1 subset to the complete AE colour set — adding Colour burn, Linear burn, Darker colour,
Colour dodge, Lighter colour, Linear light, Vivid light, Pin light, Hard mix, Difference,
Exclusion, Divide, Hue, Saturation, Colour, and Luminosity (16 new, 26 total). All run on the
existing snapshot path in the encoded (display-referred) domain — matching AE's 8/16-bit look
and the docs/06 §3.5 rationale — except the domain-invariant Darken/Lighten/Subtract, which stay
linear. The formulas are the W3C/PDF compositing set; the four HSL modes and Darker/Lighter
colour are non-separable (whole-pixel), the rest per-channel. `BlendMode::ALL` and
`BlendMode::name()` on the core enum are the single source of truth the layer dropdown and the
effect Mode param (T21) both consume, so the two never drift, and the AE group dividers come
from `blend_group_break`. `lumit_eval::blend_tag` gains stable cache-key bytes 10–25 (never
reused). A new GPU test (`perceptual_blend_modes_match_the_reference_formula`) verifies every
encoded-domain mode against a Rust reference of its formula — the compositor blends had no
oracle before. Deliberately deferred to post-v1: Dissolve / Dancing dissolve (need a dither
seed), the legacy "Classic" variants, and the alpha operators (Stencil / Silhouette / Alpha add
/ Luminescent premul, which modify alpha compositing, not colour). Extends docs/06 §3.5's own
list without reversing it. Built on `main`.

**K-163 · DECIDED · The Wavelength dispersion is driven by the three-colour picker, not a fixed
physical basis (A1).** From the owner (testing A1, resolving the §3.6 open question in favour of
"replace the basis"): the RGB split / chromatic aberration Wavelength mode no longer disperses
through the fixed physical `SPECTRAL_BASIS` (the 9-anchor CIE-derived table). Instead each
spectral tap is tinted by the effect's own three-colour picker sampled as a gradient — Colour 1
at the −offset end, Colour 2 at centre, Colour 3 at the +offset end (`tint_gradient`) — so the
picker now controls the fringe hues in Wavelength mode exactly as it does the three discrete taps
in the classic mode. The default red / green / blue reproduces the same red-at-−1 / blue-at-+1
direction the physical basis had, so the default dispersion still runs red→green→blue; other
colours re-tint it. Colour columns are normalised across the taps (guarded against a zero column)
so a uniform image passes through unchanged — the dispersion tints the fringe, never the
exposure. `Resolved::SpectralSplit` gains `tints: [[f32;3];3]`; `spectral_taps` /
`spectral_basis_uniform` take the tints; the basis is still built host-side and shared by the CPU
oracle and WGSL kernel (the kernel is unchanged, so preview == export holds, K-031). The physical
`SPECTRAL_BASIS` const and its column-sum test are retired. Pre-release, no migration. This
resolves the §3.6 open question and supersedes the "physically-based dispersion" detail of
K-090/K-144 (the smooth-many-tap machinery is otherwise unchanged). Built on `main`.

**K-164 · DECIDED · Datamosh is reimplemented as a flow-driven streamline melt with Bloom and
a periodic Reset (T19).** From the owner's test note T19 ("reimplement referencing the
well-known datamoshing technique; adjust params as needed"). The K-104/K-148 Datamosh (§3.12)
was a single motion-compensated tap — it warped the -1 source neighbour by that pixel's own
flow vector once and blended it over the current frame. T19 rebuilds it toward the genuine
datamosh look (removing I-frames so a frame's motion vectors keep being applied to the *wrong*
picture, dragging and blooming the moving regions). The new per-pixel kernel is a **streamline
walk**: starting at the pixel centre it follows the current→previous flow field out of the -1
neighbour, **re-sampling the flow at each step** (so the smear curves with the motion) and
advancing ~one frame of motion per step, then sampling the neighbour there; the samples
accumulate with a geometric weight into a melting prediction blended over the current frame.
Four params (schema version 2 → 3):
- **Intensity** (open ceiling, K-135) — blend strength; 0 the bit-exact passthrough.
- **Displacement** (frames, ≥ 1, open) — the walk's reach; the tap count is derived from it
  (~one tap per frame of motion, clamped 2–64). Supersedes K-148's `streak_length`, still read
  as a fallback so an existing instance keeps its reach (pre-release, no migration required).
- **Bloom** (0–1) — how much of the reach accumulates: 0 keeps the nearest step (a short,
  quickly-resetting trail ≈ the old single tap), 1 averages the whole walk (a long melting
  bloom). The "accumulates vs resets" dial.
- **Reset interval** (seconds, 0 = off) — the simulated I-frame period. When set, the melt
  ramps from a clean frame just after each reset up to full by the next (a sawtooth in layer
  time, computed in resolve and folded into the effective Intensity and Displacement), so the
  kernel stays time-agnostic and the frame-cache key already covers it (a param+time function —
  the K-093/K-094 reasoning; no `ALGO_VERSION` bump). It is in **seconds, not frames**, because
  the resolve step is frame-rate-agnostic — a frame-count interval needs the comp frame index
  threaded through `resolve_stack`, the broad signature change K-148 deferred, and this delivers
  the periodic-reset look without it. A **content-driven reset** still fires regardless (zero/
  unmeasurable flow at a still or cut holds the picture, where a real codec inserts its I-frame).

No new host plumbing: it keeps Datamosh's existing threaded inputs (current frame, -1 source
neighbour, one shared flow field) and its `temporal: {-1, 0}` static reach, so
`stack_flow_neighbour`/`stack_temporal_window` and the one-flow-field-per-layer rule (K-104) are
unchanged. Cost rises **cheap → moderate** (a multi-tap streamline like Motion blur's streak,
plus a flow re-sample each step); ROI stays `full-frame`, `seeded: false`. The GPU kernel
mirrors the CPU oracle (`lumit_core::fx::cpu::datamosh`) op-for-op — the same walk, tap order,
bloom weights and edge-clamp — measured worst **1 fp16 ULP** across a bloom/step sweep, within
the ≤ 2 bound. Sites: schema (`fx/builtins.rs`), `Resolved::Datamosh` variant + resolve arm
(`fx/resolved.rs`), CPU reference (`fx/cpu.rs`), WGSL kernel (`fx_datamosh.wgsl`) + `DatamoshOp`
(`lumit-gpu/src/fx/temporal.rs`) + UI dispatch (`lumit-ui/src/fxops.rs`); docs (§3.12, GUIDE).
Built in an isolated worktree; renumbered from K-161 to K-164 on merge (K-161-163 were taken by
the main session's T17 / T24 / A1).

**K-165 · DECIDED · The Shake effect's own motion blur is host-side sub-frame averaging over
a phase-domain shutter.** From the owner (T18): "Shake: add its own motion-blur twirl (toggle
+ amount), computed from inter-frame movement, applying only to this effect." Decisions:
- **Approach (a), true sub-frame averaging.** The shake wobble is a pure function of time
  (`shake_noise` at `local time × frequency`), so its motion blur samples the wobble at a
  fixed, odd count of sub-frame placements across the shutter (`SHAKE_MB_SAMPLES = 9`, the
  centre sample being the frame itself), resamples the input through each as a full
  transform-domain affine, and averages the premultiplied results — the same
  premultiplied-linear mean the accumulation motion blur uses (docs/06 §4). Translation,
  rotation and zoom all smear. This applies to **this effect's output only** — independent of
  the per-layer and comp motion blur. A dedicated one-pass kernel (`fx_shake_mb.wgsl`, up to
  9 bilinear taps) mirrors the new CPU reference `cpu::transform_average` op-for-op; the
  toggle off (or Shutter 0) is the bit-exact single resample, pinned by test.
- **The sub-frames are computed host-side.** The noise lattice uses `splitmix64`, and WGSL
  has no 64-bit integer (docs/08 §3.12), so the GPU cannot sample the noise. The resolver
  computes the 9 sub-frame `(offset, rotation, zoom)` states and the dispatch is handed ready
  affines — the same split the plain Shake already uses.
- **The shutter window is measured in the shake's own phase, not seconds.** The window spans
  `± SHAKE_MB_SPAN_BASE · amount / 2` in the noise base domain (`local time × frequency`),
  with `SHAKE_MB_SPAN_BASE = 1.0` and the Shutter amount a 0–1 fraction (default 0.5). This
  was chosen over threading a frame rate into the effect resolver: `resolve_stack` is
  deliberately frame-rate-agnostic (it carries only local time, the diagonal in pixels and
  the preview factor), and rewiring an fps through it and its many call sites for a cosmetic
  smear was not worth it. The consequence — a virtue — is that the smear is **frame-rate
  independent** (a shake motion-blurs identically at 30 or 60 fps) and still a genuine
  function of the shake's own inter-frame movement: a faster axis (higher frequency
  multiplier) advances further through its noise over the same window, so it smears more,
  exactly as real inter-frame movement would. If a seconds-anchored shutter is ever wanted,
  it is an additive change (thread fps, convert to base units at resolve).
- Two schema params in a **Motion blur** twirl (P4): `motion_blur` (Bool, default off) and
  `mb_amount` (the Shutter, 0–1, default 0.5). Off by default so existing shakes and the
  established look are unchanged; the old spec-table default of "on" (docs/08 §3.4) is
  superseded. Built in an isolated worktree against a base predating K-161–K-163; renumbered
  from K-164 to K-165 on merge (T19 Datamosh had already taken K-164).

**K-166 · DECIDED · Posterize Time loses its Scope parameter; reach is implied by the carrier
layer's kind (pass 5, T12).** The *Everything below* / *This layer's effects* choice duplicated
information the layer stack already expresses: an **adjustment layer's** effect input *is* the
composite of everything beneath it, and any **other layer's** effect input is its own source and
stack. So the parameter is gone and the hold simply covers whatever the carrier would feed its
effects anyway — Posterize on an adjustment layer steps the whole scene below (laid back by the
adjustment's coverage), Posterize on a plain layer steps that layer's own effects and source
sampling while its transform stays live. Both K-133 behaviours survive unchanged; only the
selector is removed. Orchestration sites (`posterize_below`, `posterize_sample_times`, export's
below-filter) key on `LayerKind::Adjustment` instead of the stored choice. Projects saved with a
Scope value still load (unknown params are ignored on read); the stored value is simply unread.
Pre-release, so no migration is owed (the standing backwards-compat policy).

**K-167 · DECIDED · Three-tap tint columns are normalised per output channel in the classic
split modes (pass 5, T17).** Owner report: changing the tap tints on RGB split / Chromatic
aberration shifted the whole image's exposure, not just the fringe. Root cause: the three taps
sum, so tints whose per-channel weights do not sum to 1 rescale even perfectly aligned regions.
Fix: `lumit_core::fx::normalise_tint_columns` rescales each output channel's column of tap
weights to sum to 1 (guarded below 1e-6) before resolve hands the tints to CPU or GPU — the
same rule the Wavelength gradient already applied host-side (K-163). Consequence: custom tints
only affect the parts of the picture where the taps disagree (the misaligned fringe); uniform
regions pass through at original exposure, and the default red / green / blue columns already
sum to 1, so the classic split stays bit-exact. Applied in both classic resolve arms; Wavelength
mode was already normalised.

**K-168 · DECIDED · The Timeline outline adopts After Effects' five column groups; lock and
label-colour switches enter the model (pass 5, TL2).** Left to right: **1** visibility · audio ·
solo · lock, **2** label chip · stack number · name, **3** flow-or-collapse · fx bypass · motion
blur · 3D, **4** matte · blend, **5** parent. New model surface: `Op::SetLayerLocked` and
`Op::SetLayerLabel`; `Layer.label: u8` (serde default 0, so old projects load). A locked layer's
bar, trims and stack order refuse edits (its property values stay editable — v1 lock protects
timing/order, the thing a stray drag breaks); the label chip cycles eight colours drawn from the
theme's existing roles via `Theme::label_colour` (no new hex, docs/15 §4). Neither `label` nor
`locked` feeds the frame cache key — both are organisational, never pixels. Deliberately not
built yet, each blocked on machinery it would misrepresent without: **shy** (needs an outline
filter row), **quality** (needs a bicubic sampler choice), **preserve underlying transparency**
(needs compositor support), and the **pick-whip** parent drag (the dropdown stands in, K-103).

**K-169 · DECIDED · The optical-flow engine is dense inverse search (DIS); resolves 08 Open
Question 1.** The flow field that feeds Retime's flow interpolation and Fast motion blur is
computed by **Dense Inverse Search** (Kroeger et al., ECCV 2016), not the "variational /
patch-match hybrid" the 08 §3.1 sketch first floated. DIS is the studied sweet spot: fast,
GPLv3-clean (no trained model to redistribute), and cheap enough to run per preview frame. The
exact structure — 8×8 patches on a stride-4 grid, a few Newton steps per patch, forward-backward
occlusion, box-blurred confidence — is pinned in `docs/impl/optical-flow.md` and implemented in
`lumit-flow` as a CPU oracle plus WGSL twin (K-019). A learned RAFT-class backend stays a
possible future FlowField producer behind the unchanged API (dense vectors + occlusion +
confidence); motion blur would keep using DIS vectors. This records a choice the impl note and
shipped code already made but the spec's open question still listed as pending.

**K-170 · DECIDED · The UI's worker-result channels are unbounded `std::sync::mpsc` by
deliberate choice; 14-ENGINEERING-RULES §5's "no unbounded queue without a decision entry" is
satisfied here.** The `lumit-ui` shell talks to its background threads over plain unbounded
`mpsc` channels — pre-mixed audio and comp-audio buffers, beat-detection results
(`app_state/mod.rs`), disk-cache load commands and their loaded frames (`app_state/diskio.rs`),
preview-render results (`app_state/preview.rs`), export-progress events (`export.rs`), and media
decode results (`app_state/media.rs`). None of these grows without bound in practice, for two
distinct reasons, and that — not oversight — is why they are unbounded:

- **Latest-wins mailboxes** (audio / comp-audio / beats / preview results): the UI drains the
  whole channel every frame and keeps only the newest message, so the standing depth is at most
  the handful of items a producer can emit inside one ~16 ms frame. A bounded channel would add
  `try_send`-and-drop plumbing to achieve the same effect the drain already gives for free.
- **Self-throttling work queues** (disk IO commands, media decode, export events): the UI issues
  at most one outstanding request per cache slot / per active job, so the number of in-flight
  messages is capped by the caller's own concurrency, not by the channel.

v1 therefore keeps the simpler unbounded type. The escape hatch: if profiling ever shows a
channel accumulating (a producer outrunning a stalled UI thread), the fix is a bounded
`sync_channel` with explicit latest-wins drop on the latest-wins ones, logged as a follow-up
decision — not a silent swap. The realtime audio callback stays lock-free ring-buffer reads only
and is unaffected by this entry.

**K-171 · DECIDED · Cached preview playback renders every frame and never skips; skipping is
Realtime mode's job alone.** The intended behaviour, stated by the owner (it predates this log
but was never written down): in the default **Cached** mode, playback advances to the next frame
only when that frame has rendered. When rendering is slower than realtime the playhead slows
down with it — audio pauses (v1) or timestretches to match (later) — and every frame lands in
the cache; once the span is cached, playback replays it at full speed from cache. The shipped
behaviour to date — a realtime clock that drops any frame not ready in time — is *not* Cached
mode; that clock-chasing, frame-dropping discipline belongs exclusively to **Realtime** mode
(K-030), where responsiveness is the point and resolution degrades instead. Consequences: the
playback tick gains a render-gated stepping path as the default; the audio clock is master only
while playback is actually realtime (cached replay, or Realtime mode); during slower-than-
realtime cached rendering the *frame counter* leads and audio follows or waits. 06 §6 and the
playback-scheduler impl note describe the ring/pre-roll machinery this stepping feeds.

**K-172 · DECIDED · Per-layer audio: the Volume property ships (−∞..+50 dB) and per-layer
waveform lanes replace the comp-wide strip (owner, 2026-07-21).** Three linked calls from
the owner's desk testing. (1) `Layer.volume_db` lands as the docs/09 §6 animatable dB
property — `Op::SetLayerVolume` (coarse-grained like SetTransformProperty), default 0 dB,
ceiling raised from the spec's +12 to +50, and −100 dB is the −∞ knee (gain exactly 0 at or
below; the value box reads "−inf"). A static volume is a constant gain on the placed clip;
a keyframed one bakes to a ~10 ms control-rate `GainEnvelope` read identically by the live
`MixPlan` callback and the baked export mixdown — playback == export, pinned by test.
(2) The timeline outline gains an **Audio** group (footage with an audio stream only):
the Volume row with the standard stopwatch / ◄ ◆ ► furniture, and a **Waveform** twirl
whose lane draws the layer's own decoded peaks mapped through its live in/out/offset every
paint — so dragging the layer carries its transients in realtime, the owner's report
against the comp strip (which only refreshed when the mix re-planned). (3) The comp-wide
waveform strip under the ruler is removed outright, along with its T25 toggles and the
background peaks bake (its `CompAudioMsg::Peaks` delivery). Lane keyframe diamonds for
Volume await the shared PropRow widening (the UI-11 note); fade commands and detach-audio
remain future §6 work.

**K-173 · DECIDED · A saved project never contains an absolute path; the absolute location is
session-state (TF-36, tester privacy report).** docs/10 §2 contradicted itself: it said every
reference stores "the last absolute path" AND that nothing machine-specific — "no local
usernames" — is ever written. The tester sharing a project found their username inside
(`/home/<name>/...` in every reference), which settles which half wins. `MediaRef.absolute_path`
is now `#[serde(skip_serializing)]`: it lives for the running session (probing, the resolver's
step-2 fallback) and never reaches the file; projects saved before K-173 still *load* theirs.
What the file carries instead: a **relative path rebased against the project's folder on every
save** (forward slashes, so a Windows save resolves on Linux; a cross-drive reference falls back
to the bare file name) and a **fingerprint stamped at save time** where one is missing. On open,
the previously built-but-unwired docs/10 §2 resolver now actually runs: relative path → legacy
absolute → fingerprint search across the project tree; found files repoint the session path
(this is what makes a moved project folder open intact — the tester's other half of the report),
and missing ones are named in a notice, the relink dialogue remaining future work.

**K-174 · DECIDED · A Flutter frontend alternative is built on its own branch, docs-first,
one-for-one before any redesign.** The owner wants to evaluate replacing the egui frontend
with Flutter (text rendering, motion, platform polish, widget ecosystem). The experiment
lives on `flutter-frontend-alternative`: a Dart application in `flutter_ui/` over the
unchanged Rust engine crates, specified by `docs/archive/flutter-port/` (strategy, full UI
inventory, bridge architecture, widget map, living parity checklist). Ground rules: the
first pass reproduces the shipped egui behaviour exactly — known rough edges are logged,
not fixed — so there is a truthful baseline; the glossary, no-hex-outside-theme and
tests-with-features rules bind the Dart tree as they bind Rust; engine crates never depend
on either frontend; `main` keeps shipping the egui frontend until the Flutter one reaches
parity and wins the side-by-side. The Viewer's frame path is the one piece of new systems
work (wgpu → shared D3D11 texture → Flutter texture registrar, docs/archive/flutter-port/03).

**K-175 · DECIDED · The bridge borrows lumit-ui's renderer through the headless seam until
the pixel pass moves into an engine crate.** The composited comp frame the Flutter Viewer
needs (every layer, transform, blend and effect — the pixels the egui Viewer and the
exporter show, K-031) is produced by the compositor that currently lives in `lumit-ui`
(`crate::export`'s window-free `Renderer`). To reach it without duplicating the compositor,
`lumit-ui` gains a small `headless` module (`HeadlessRenderer`, the export path made
reusable behind a GPU context it owns), and `lumit-bridge` gains a default-on `render`
feature that depends on `lumit-ui` and drives that seam through
`lumit_bridge_render_comp_frame`. This is a **deliberate, temporary** arrangement: the
bridge (a leaf, not an engine crate) depends on the UI crate here and nowhere else. The
docs/05 rule — *engine crates never depend on a frontend* — is unbroken; the bridge is not
an engine crate. When the pixel pass is extracted into an engine crate (the shared-compositor
work docs/archive/flutter-port/03 anticipates), the bridge will depend on that crate instead and the
`lumit-ui` dependency is dropped. Recorded so the dependency edge is understood as scaffolding,
not the destination.

**K-177 · DECIDED · The Viewer's zero-copy path is a D3D12 shared NT handle Flutter samples
directly, with the read-back path kept as the airtight fallback.** The recorded top
performance gap (K-176) was the Viewer's per-frame round trip: render on the GPU → read the
pixels down to the CPU → copy across FFI → upload back to the GPU. This closes it on Windows.
wgpu runs over D3D12; behind an **opt-in `shared-texture` feature** (off by default, so every
existing build and CI gate is byte-for-byte unchanged) the headless renderer reaches through
wgpu to its D3D12 device (`Device::as_hal`), creates a texture in a **shared heap**
(`D3D12_HEAP_FLAG_SHARED`, `DXGI_FORMAT_R8G8B8A8_UNORM`, `ALLOW_SIMULTANEOUS_ACCESS`), exports
an **NT handle** (`ID3D12Device::CreateSharedHandle`), and wraps the same resource back as a
`wgpu::Texture` (`create_texture_from_hal`). The finished, display-encoded frame is copied
GPU-to-GPU into it (a valid srgb-differing `copy_texture_to_texture`, no re-encode) and the
handle is handed across the bridge; the Windows runner registers it with Flutter as a
`kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle` external texture (the embedder opens the handle
on its own ANGLE/D3D11 device), and the Viewer shows a `Texture` widget. The pixels never
leave the graphics card. **Choice made — D3D12-direct, not a separate D3D11 device:** the
direct route is self-contained (no second device, no D3D11-on-12) and was verified to work end
to end on the dev machine (the `solid_comp_renders_to_a_stable_shared_handle` test creates the
shared resource, exports a non-zero handle, and re-uses it across frames). Under the feature
the headless renderer pins the **D3D12 backend** (the interop needs it); every non-feature
build keeps the all-backends instance. **Synchronisation:** after the copy we `poll(Wait)` so
Flutter never samples a half-written frame; a keyed-mutex / shared-fence handshake is the
recorded follow-up, worth adding only if tearing shows in practice (D3D12 uses fences, not
keyed mutexes, so a cross-API handshake is non-trivial — deferred until observed). **No new
runtime dependency:** the plumbing pattern (descriptor shape, the DXGI-shared-handle surface
type, the register / mark-frame-available dance) follows the MIT-licensed
`flutter_wgpu_texture` package as a *reference* — pattern borrowed with a code-comment credit,
not added as a dependency (it owns its own renderer/scene architecture and is very young). The
`windows` crate is pinned to **0.58** so its D3D12 types unify with the ones wgpu-hal already
uses. **Fallback is airtight and tested:** `lumit_bridge_shared_supported()` is false for an
old `.dll`, a non-Windows build, or a feature-less build; `render_to_shared` returns false (Dart
falls back for that frame) on no D3D12 adapter or any interop error; the platform channel
missing (an unwired runner) latches the controller off for the session — every seam falls back
to the read-back path, each covered by a fake in the Dart suite. **Scopes** still need CPU
pixels (the texture path moves none): a throttled read-back render (~10 Hz) feeds them while
the texture drives the Viewer. **Remaining after this:** the read-back path stays for scopes
and for every fallback; engine-side render cancellation and a rendered-frame cache (K-176)
are still open; the keyed-mutex handshake is the named follow-up.

**K-178 · DECIDED · The pixel pass moves into `lumit-render`, an engine crate both
frontends drive; the bridge's dependency on `lumit-ui` (K-175) is retired.** K-175 recorded,
as deliberate scaffolding, that `lumit-bridge` would depend on `lumit-ui` to reach the
compositor "until the pixel pass moves into an engine crate". This is that move. A new engine
crate `lumit-render` holds the whole pass: probing abstraction (`source`), decode planning
(`plan`), the decode worker and its decoded-frame cache (`decode`), draw-list building
(`build`) and its types (`draw`), the GPU compositor (`realise`), effect dispatch (`fxops`),
frame naming and the cache tiers (`cache`, `diskio`), export, and the headless seam. It
depends on no frontend and names neither egui nor Flutter; `lumit-ui` and `lumit-bridge` both
drive it. The docs/05 rule is not merely unbroken but strengthened — the bridge is no longer a
leaf hanging off a frontend, and the shipped Flutter `.dll` no longer links egui, `egui_tiles`,
`iconflow`, `rfd` or `muda`. Two pieces moved further down: `pixels` and `preset` are pure
data/maths with no media or GPU dependency and must survive a `--no-default-features` build, so
they live in `lumit-core`. **Why now, and what it bought:** the reason was performance, not
tidiness. The Flutter Viewer drove `export::Renderer`, which decodes every frame afresh at full
resolution and retains nothing, so *dragging a value re-decoded the whole composition on every
tick* — while the egui Viewer had long re-composited from the frame's retained per-layer pixels
and never re-decoded during a drag. Sharing one crate made it possible to give the Flutter path
that behaviour instead of building it a second time: `HeadlessRenderer::render_preview` plans
the decode, reuses the pixels it holds when the plan is unchanged (`plan::same_decode`), and
decodes at the preview resolution. `DecodePool::comp_decodes` counts real decodes so the drag
contract is a *test*, not a claim. The zero-copy shared-texture paths (K-177) were moved onto
the same walk, so the shipped build gets the fast path and cannot disagree with the read-back
path about a frame. **Frame naming:** the bridge's rendered-frame cache (K-176) keyed on
`(comp, frame, scale)` plus the identity of the document snapshot, so *any* commit — a rename,
a work-area nudge, a solo toggle — emptied it. It now keys on the content hash
(`lumit_eval::comp_frame_key`, already an engine crate), so picture-free edits discard nothing
and an edit to one layer retires only the frames that layer appears in. **Cost, recorded
honestly:** two comp walks still exist — `build_comp_draws` (interactive) and
`render_comp_linear` (export) — kept in step by hand and by tests, exactly as they were inside
`lumit-ui`. Unifying them by having export decode into a pixels map and share the draw walk is
the recorded next step (docs/TODO.md, Now), gated on a bit-identity matrix across precomps,
mattes, adjustments, collapse and motion blur; a solid-comp identity test is in place already.
This entry supersedes K-175's temporary arrangement; K-175 stays as the record of why the edge
existed.

**K-179 · DECIDED · flutter_rust_bridge is the only front/back seam; the hand-written
`extern "C"` bridge is deleted.** The interim transport ("bridge v0" in
[17-BRIDGE-CONTRACT.md](17-BRIDGE-CONTRACT.md)) passed whole documents as JSON text over 107
hand-written `extern "C"` functions. It was always described as a deliberate interim choice
with flutter_rust_bridge as the intended target once the command surface stabilised; this
entry records that arriving. Everything the frontend does now goes through
`crates/lumit-bridge/src/api/`, which hands Dart opaque reference handles
(`ProjectReference`, `CompositionReference`, `LayerReference`, `ItemReference`) with methods on
them, plus a scoped-change stream naming which reference an edit touched. **The reference types
are the identity**: there is no snapshot to diff, no mirror class to keep in step, and no id
lookup, which is what removes the whole-document JSON round trip per edit. Two shapes follow
from that and are binding: an op takes a **whole value** rather than a granular delta (a
keyframe drag that moves time *and* value is one write and therefore one undo step), and a
*staged* edit — a drag — renders through a patched clone engine-side and commits once on
release. The two bridges ran side by side while each panel moved across, then v0 was removed in
one sweep so the two never had to be kept in step; the migration's running order and the
capability gaps that remain are in [TODO.md](TODO.md). What survived the sweep is shared
infrastructure, not transport: the layer and asset defaults both frontends build from
(`edits.rs`), the scale-to-decode-size policy (`render::quality_for`), the rendered-frame cache,
the realtime controller, media probing/decoding, and the exporter — whose entry point now takes
the document as an argument rather than reading a process-wide bridge, which is what let one
exporter serve both frontends. This supersedes 17-BRIDGE-CONTRACT.md's "JSON over a C ABI"
transport section; the four binding rules in it (no panic crosses the boundary, no lock held
across the boundary, rational time crosses as integers, the engine never depends on the
frontend) are unchanged and still bind.

**K-180 · DECIDED · A composition's duration is a length of time, and the frame rate is only a
frame rate.** The Composition settings dialogue used to edit the duration as a *frame count* and
the rate as a visible numerator over a denominator, and `BridgeCompSettings` carried the count
across the bridge. That was a bug, not a presentation choice: a frame count means nothing
without the rate it was counted at, so pressing Save after changing 60 fps to 30 wrote
yesterday's 1800 frames back at the new rate and *doubled* the comp's real length, while every
layer kept the seconds it already occupied. On screen that looked exactly like the layers
speeding up or slowing down — reported by the owner, and the reason for this entry. **Binding
now:** the duration crosses the bridge as exact rational **seconds** (`BridgeCompSettings
.duration`), which is what the document has always stored, so changing the rate changes only how
finely the comp is counted — never how long it is, never where a layer sits, never how fast
anything plays. Frame counts are derived on demand from `CompositionReference::duration_frames`.
**The dialogue's shape follows from that:** the rate is one number in one field (`600`,
`23.976`) with the awkward rates on a Presets list, and the duration is `HH:MM:SS.mmm`. The
exact `num`/`den` pair still crosses the boundary — docs/14 §2's rational-time rule is
untouched, and 23.976 still reaches the engine as 24000/1001 — but the pair is worked out from
what was typed rather than typed by hand, because a denominator is an implementation detail of
NTSC and not a question to ask someone making a comp. **One dialogue, three doors:** the same
window is New composition (with a Create button) and Composition settings (with Save), reached
from the menu bar, the Project panel's footer button, and a right-click on a comp; creating is
one call and therefore one undo step, never "create then apply". **Footage dropped on the New
composition button** opens it prefilled from the media's own size, rate and length, and every
dropped item lands in the finished comp as a layer, which is what docs/07 §3.1 has always asked
for; that is also why the Project panel now multi-selects (`Ctrl` adds, `Shift` takes the run)
and why a drag carries the whole selection.

**K-181 · DECIDED · The frontend holds no logic — it displays values and forwards calls.**
K-017 says the UI thread never *evaluates*; 17-BRIDGE-CONTRACT says the engine owns the
document and the frontend never mutates it directly. Both were narrower than the rule actually
wanted, and the gap let a whole scheduler grow in Dart without breaking either: the Viewer's
playback loop mutated nothing and evaluated nothing, it merely *decided* — which frame to ask
for next, how many renders to keep in flight, whether the picture was stale, what frame the
audio clock implied. Reported by the owner as "we need to move the logic for handling playback
to the Rust side". **Binding now:** the frontend may own *interaction state* — where the
playhead is, the zoom, the selection, the pan — and must act on it immediately, without a round
trip; what it may not own is *policy*. It states facts to the engine ("the playhead is at 40",
"play from here", "the document changed") and paints what comes back. Anything that has to be
decided — scheduling, timing, invalidation, degradation, when work is worth doing — is the
engine's, because the engine is the half that holds the inputs to those decisions. **The test
of the rule:** if a Dart change would need a clock, a queue, a retry, a staleness flag, or a
count of work in flight, it is on the wrong side of the boundary. **First application:**
playback moved into `lumit-bridge`'s render worker, which now paces itself and publishes each
frame with its own frame number, so the frontend no longer has to track what it asked for. The
`Ticker`, the every-frame pump, the in-flight counter and the stale flag are all deleted.
The worker is not yet the full scheduler `docs/impl/playback-scheduler.md` §5 specifies — no
epoch tokens, no ring, no adaptive lookahead — and that gap is recorded in docs/TODO.md rather
than pretended away.

**K-182 · DECIDED · The egui frontend is deleted; git history is the parity reference.**
Supersedes the working stance (recorded in TODO.md after K-174) that the egui code stays in the
tree as the parity reference. Reported by the owner: after egui → Flutter → frb, the project
had "become bloated, over-engineered and far too complex", and a full over-engineering review
confirmed the bloat was almost entirely migration corpses, not the live code. **Deleted in one
sweep:** `crates/lumit-ui` (~30,600 lines) and `crates/lumit-app` — nothing else depended on
them; `crates/lumit-keymap` — zero dependents, existed only for an unbuilt settings page;
`packaging/flatpak` and its CI job — it shipped the egui binary; the never-wired pop-out
subsystem (`flutter_ui/lib/popout/`, the `desktop_multi_window` plugin and the dock's pop-out
chrome — `canPopOut` was hard-coded false, so all of it was unreachable); and the dead Dart the
port left behind (`scope_maths.dart`, `AutosaveScheme`, the settings structs nothing read, the
PowerShell RAM probe, the per-call bridge tracer). **Why deletion rather than keeping the
reference:** a parity reference you can `git show` is exactly as available as one you compile,
and the in-tree copy cost every CI run, every workspace build, and a standing invitation to
"fix it in the old frontend too". **The rule going forward:** when a feature is parked (as
pop-out is), it is *removed* and rebuilt from history when wanted — half-shipped code that
ships its dependencies but not its entry point is the worst of both. K-174's decision itself
is unchanged; this only deletes the superseded implementation.

**K-183 · DECIDED · Frames cross the bridge as GPU handles only, and reads cross grouped.**
Reported by the owner's collaborator: the Viewer should lose the ability to send pixel data
back to Flutter entirely ("forced to use shared texture everywhere"), and the panels should
stop paying a bridge call per field ("we don't have calls to things like .name() .id(),
grouping things into .get_info() that can be called once per widget rebuild"). **Transport:**
the CPU read-back frame path is deleted — `WorkerResponse::RenderedPixels`, the `zero_copy`
opt-out flag, the Dart `viewerImage` fallback machinery and the `useSharedTexture` setting are
gone, and `shared-texture` + `shared-texture-linux` are default cargo features (each inert off
its platform), so every build and every test exercises the shipped path. A failed zero-copy
render drops the frame rather than falling back; a platform with neither path (macOS, K-033)
has no Viewer picture until it grows its own. Thumbnails and the 256×256 scope traces still
cross as pixels, deliberately — bounded and rare. The rendered-frame cache is now filled only
by the scope path. **Grouped reads:** `LayerReference::get_info` returns name, kind, switches,
blend, the span already mapped to comp frames, clip split frames, and the parent id *and
name* in one crossing; `BridgeEffectInstance::get_info` returns id, name, enabled and every
parameter value. Panels read one info per widget rebuild, the parent picker builds its menu
lazily on click (it was O(layers) calls per row, O(layers²) per outline), and the transform
rows share one `get_transform`. Selecting a layer measured ~75 → 31 calls; the budget test
caps it at 64. The per-field getters remain for one-shot call sites — grouping is for what
rebuilds, not a ban.

**K-184 · DECIDED · The panels draw from a Rust-built read model; a rebuild costs one call.**
Follows K-183's grouping to its end, prompted by the owner: "why does selecting a layer take
31 calls?? Surely one or two is all it needs?" The answer was that selection changed nothing
in the document — the 31 were two panels repainting and re-asking for what they already knew.
**Binding now:** `CompositionReference::get_model` returns the whole fronted comp as the
panels draw it — every layer's handle plus name, kind, switches, blend, span as frames, clip
frames, parent and its name, the full transform, and every effect's every value — in ONE
crossing. Dart holds it in `CompModel` (state/comp_model.dart), and the panels (Timeline,
Hierarchy, Effect controls, the parent picker, the comp tabs) draw from it with no bridge
calls in build. **Freshness is a revision number, not faith:** `DocumentStore` counts every
published snapshot (commit, undo, redo, recovery — regression-tested in lumit-core), and the
model compares that one number per read, re-reading the world only when it moved. So any
rebuild for any reason shows the current document — the exact contract the old
read-everything-in-build code had — for one call instead of dozens, and the model needs no
trust in the async change stream to be correct (the stream just triggers repaints; panels
also nudge `refresh()` after their own ops so an edit is on screen without a round trip).
The model is plain data, never handles: edits still go through the references, and effect
ops fetch a fresh instance handle at click time (frb consumes handles passed by value).
`LayerBuilder` is deleted — per-row change scoping existed because rebuilds were expensive,
and rebuilds that cost one revision check need no scoping. Measured: selecting a layer is
now 11 calls (was ~75 pre-K-183, 31 after it); the budget test caps it at 24.

**K-185 · DECIDED · There is one comp walk; export drives the preview path.**
K-031 ("preview == export") was held together by hand: `build_comp_draws` + `Realiser` drew the
Viewer while `render_comp_linear` — a parallel ~1,400-line implementation of the same rules —
drew the file, kept identical by discipline and comments. The TODO's gate ran first: a
bit-identity matrix (blends/opacity, nested and collapsed precomps, all three matte source
modes, adjustment stacks, per-layer motion blur, posterize time, camera over 3D, plain footage,
Retime blend and Retime flow) proved the two walks byte-identical on every row **before**
anything moved. Then the export encode loop and `render_rgba` switched onto the preview path —
`HeadlessRenderer::render_preview` at full decode quality, the exporter on its own renderer and
device so it never contends with the Viewer — and `render_comp_linear`, its `Renderer` and every
private helper were deleted (export.rs: 2131 → 683 lines). K-031 is now true by construction:
there is no second walk to disagree. The matrix stays as the determinism gate for the one walk.

**K-186 · DECIDED · The composite runs at the preview scale; geometry stays logical.**
The realtime tier and Auto resolution used to shrink only the decode: the composite itself
always ran on a full comp-sized target (measured 59.7 ms/frame for a one-solid 1080p comp
shown at 0.42), so a coarser tier barely made frames cheaper. **Binding now:** the one walk
carries a render scale (`Realiser::render_scale`, a field so the nested/below/adjustment
recursions inherit it with no signature ripple). The split is logical-steers,
target-allocates: every placement matrix and the camera keep the LOGICAL comp dims —
geometry is in comp pixels — while `composite_seeded` / `motion_blur_average` allocate their
targets, dst snapshots and fp32 accumulators at the ACTUAL `lumit_gpu::scaled_size` dims and
feed those to the fragment's `target_size` uniform (which normalises the frag position to
comp UV for matte and snapshot sampling); NDC lands the same geometry on the smaller raster.
`scaled_size` is the ONE rounding both the target and the preview's final blit use. The
matte render-alone pass deliberately stays full-res (sampled by normalised comp UV, so any
size is correct); the adjustment stack, coverage and `adjust_blend` run at the actual raster
(texel-matched reads). The shared-texture registration sizes off the texture's actual dims,
so a tier change re-registers a genuinely smaller texture. Export builds the walk with scale
1.0 always, and the K-031 matrix pins that path bit-unchanged — the preview scale can never
leak into the file. Regression tests: `a_render_scale_shrinks_the_target_but_not_the_geometry`
(lumit-gpu) and `auto_resolution_composites_at_the_scaled_size` (lumit-render).

**K-187 · DECIDED · The VRAM final-frame cache and the idle fill: revisited frames are free.**
Docs/06 §5's top tier, built for the zero-copy transport that made the RAM frame cache
irrelevant to the Viewer (K-183): the renderer keeps finished display textures on the card,
keyed `(comp, frame, preview scale in thousandths, channel order)` under a byte-budgeted LRU
(default 512 MiB, Settings → Performance sets it). Playback, scrubbing and the ring all pass
through it — a warm span composites nothing. **Position keys carry two duties:** every
committed edit drops the whole tier (the same generation signal that drops the RAM bytes,
watched by the worker each loop turn), and a live drag's provisional renders pass
`cacheable: false` — they must neither be served stale nor bank half-committed pixels.
**Idle fill (§5.5, forward-biased):** after a 200 ms request lull the worker renders
uncached frames outward from the last-shown frame — two ahead for every one behind, bounded
by the work area and by the budget (it stops before the LRU would churn) — one frame per
wake so any request pre-empts it within one render. **The cache bar merges the tier**: the
worker publishes its holdings (packed exactly like `framecache`'s keys) and `cached_frames`
reports card-held frames as green — the bar means something on the zero-copy transport
again. The textures never leave the worker's thread; settings speak to it through three
atomics and a published mirror, so no lock ever spans GPU work. The disk tier (§5.4) and
content keying (K-178) remain open. Regression tests:
`a_cacheable_frame_is_served_from_vram_and_a_drag_never_is` (lumit-render),
`the_fill_order_is_forward_biased_and_complete` and `cached_tiers_merges_the_vram_mirror`
(lumit-bridge).

**K-188 · DECIDED · The Timeline header rework: four draggable column groups, open comp tabs, shy.**
Supersedes K-168's shipped five-cluster arrangement. The outline's columns sit in FOUR
groups, each draggable in the header to reorder as a unit — 1 visibility · audio · solo ·
lock · shy; 2 twirl · label chip · layer number · name; 3 flow-or-collapse · fx · motion
blur · 3D; 4 matte · blend · parent. The header icons are indicators only; the switches
live on the rows, and visibility/audio swap glyph when off (closed eye, muted speaker)
rather than only dimming. **Shy is a real engine switch** (`Switches::shy`, `SetLayerShy`):
it hides the layer from the Timeline's list while the toolbar's shy filter is on and never
changes what renders. **Comp tabs are open tabs now**, not the whole project: fronting a
comp opens its tab, the tab's × closes only the tab, and closing the fronted tab fronts
its nearest neighbour. **The toolbar lives inside the outline** (timecode `HH:MM:SS:FF`
plus a zero-based frame readout, the layer search, a master motion-blur button writing
`Composition::motion_blur.enabled` through the new `set_motion_blur_enabled`, the shy
filter, Lane/Graph view buttons, and a ⋯ menu holding the layer/work-area/marker
commands); the lane side gives that whole height to a taller labelled time ruler. The
fold-out's value cells span exactly the render group's width, so values line up under it
wherever the groups are dragged. The flow column is reserved: optical flow has no
per-layer engine backing yet (docs/TODO.md), so a Precomp shows collapse there and other
kinds leave the cell empty. Lock is enforced UI-side where the gestures live (bar
move/trim, razor, rename, reorder/delete); property-row edits on a locked layer are a
recorded gap. Master motion blur does NOT cascade into nested comps — each comp's own
master gates its own layers (lumit-eval reads `comp.motion_blur.enabled` per comp).
Regression tests: `timeline_panel_frb_test.dart` (group drag, switches, readouts, shy,
lock, master toggle), `timeline_extras_frb_test.dart` (tab open/close), and
`the_master_motion_blur_toggle_flips_only_the_enable` (lumit-bridge).

**K-189 · DECIDED · Timeline round two: label colours drive the bars, animated values stay editable, drags never scroll.**
Follows K-188 in the same rework. **Labels colour the lanes:** a layer's label chip and its
bar in the lane area are the same colour, from a dedicated bright eight-chip palette in the
theme (replacing TL2's role-colour chips, which were built to be quiet rather than tellable
apart), and each layer kind starts on its own chip (`base_layer` assigns it; the user's
pick simply overwrites). One palette for both themes. **Animated values stay editable
everywhere they show:** an outline value field on a keyframed property shows the value
under the playhead and an edit writes the key sitting there — or plants a linear one — via
`scalarWithValueAt`; a static write over a curve is no longer possible from a value field.
The keyframe controls read the *live* playhead (the ◆ diamond fills exactly while the
playhead sits on a key). **Keyframes show in lane view:** keyed rows draw their diamonds
on their lanes, and dragging empty lane space boxes them up with the shared `MarqueeSelect`
(the same widget the graph editor's lanes use). **Dragging never scrolls the timeline** —
the wheel and the scrollbars do: the outline and lanes share a linked vertical scroll (one
thumb on the lane side; two independent ones in graph view), and the lane bottom bar holds
− / + / Fit time zoom with a horizontal scrollbar. Lane painters decline hit-tests (a
`CustomPaint` background painter otherwise absorbs the marquee's drag). The graph editor's
command bar moved to the bottom and its lanes label their value axis. Regression tests:
`timeline_panel_frb_test.dart` (key-at-playhead edits, live diamond, lane marquee, zoom,
bar colour, tall-stack scroll), `effect_controls_frb_test.dart` (animated field edits the
key), `theme_test.dart` (distinct chips).

**K-190 · DECIDED · Timeline round three: row seams, key dragging, and the scroll gutters.**
Continues K-188/K-189. **Column metrics:** every gap *inside* a group is now the same
`cellGap` — the render switches pack left in ordinary switch cells (the rest of that
group's span is the fold-out's value column, not spare icon room) and matte · blend ·
parent sit a cell-gap apart. The compose group's header titles carry the dropdown's own
`dropdownTextInset`, so each title sits over the text in the cell below it. The group seam
is a hairline **in the header only** — the rows keep the same width as plain space,
because a rule down every row of a tall stack is noise. **Row seams** run the full width of
the lane area, drawn as ONE `IgnorePointer` overlay per lane column rather than as a border
per row: `RenderDecoratedBox.hitTestSelf` delegates to the decoration, so a `Container`
with a `decoration` **absorbs pointers** — a per-row border silently ate the keyframe
marquee under it (the same trap as a `CustomPaint` background painter, which needs
`hitTest => false`). Bars fill their whole row height and the seam draws over them.
**Lane keyframes drag in time**: each diamond is a handle, the gesture is held in Dart and
committed once (`moveLaneKey`), and a move onto a neighbour is refused rather than clamped.
The **magnet** (lane bottom bar, on by default) decides whether a dragged key lands on a
whole frame or between two; off, the time is quantised to a thousandth of a frame and built
from the comp's exact rate, so it stays rational (docs/14 §2). **Scroll gutters:** the
vertical thumb lives in a fixed-width gutter *outside* the horizontal scroller, pinned to
the viewport's right edge — it used to ride the scrolled content and drift off screen. The
outline reserves the same gutter, with a fixed undraggable block level with its toolbar and
column header (After Effects' reserved corner), so the columns do not shift when graph view
gives the outline its own thumb. **Wheel:** plain scrolls the rows (both halves, linked),
`Shift` scrolls sideways, `Ctrl` zooms time about the pointer — handled by a `Listener`
placed *inside* the scrollables so the pointer-signal resolver offers it the wheel first,
and left alone otherwise so a plain wheel still reaches the scrollable. Effect parameter
rows take the fold-out's zero row padding, matching the transform rows they sit beside;
the card keeps its own. Regression tests: `timeline_panel_frb_test.dart` (key drag with
magnet on and off, marquee, dividers via the passing marquee, zoom, bar colour).

**K-191 · DECIDED · A composition double-clicks open; an empty Timeline takes a drop.**
Two dead ends closed. **Double-clicking a comp row in the Project panel opens it in the
Timeline** rather than renaming it — what a double-click means in every editor. The panel's
click model is otherwise unchanged (a second click on the lone selected row still renames
footage, solids and folders in place, resolved on the raw pointer-up so there is no arena
delay); only compositions divert. Renaming a comp therefore moved to the row menu's new
**Rename** entry, which is offered for every item kind so nothing lost a rename path, and
its settings dialogue still carries the name field. **Dropping footage on a Timeline with
no composition open** raises the New composition dialogue seeded from the media — the same
gesture the Project panel's New composition button already took — and fronts the finished
comp; dropping a comp there simply opens it. The panel used to show a placeholder with no
drop target at all, so the drag lifted, showed its feedback and dropped into nothing.
Regression tests: `double-clicking a composition opens it in the Timeline`
(project_panel_frb_test.dart) and `footage dropped on an empty Timeline offers a new comp`
(timeline_panel_frb_test.dart).

**K-192 · DECIDED · Resizable column groups, property selection, and a keyframed drag as one undo step.**
**The undo bug first, because it was a real one:** [`DragValueField`] falls back to
`onChanged` on *every drag tick* when no `onChangeLive` is given, so a drag on a keyframed
value (K-189's editable animated cells) committed one op per pixel — the undo stack filled
with a step per tick and a single undo moved the value back by a hair instead of undoing the
gesture; a drag that planted a new key planted one per tick. `KeyedValueField`
(keyframe_controls_frb.dart) now stages the drag in Dart and commits exactly once on
release, and the transform rows, effect parameter rows and the Volume row all use it.
**Column groups resize:** each group carries its own width, the header seam between two
groups is a drag handle for the one on its left, and every other group keeps its width — so
the outline grows by exactly what the drag moved. The fold-out's value cells span the render
group *as it currently is*, and the compose group's three pickers share theirs
proportionally, so widening a group widens what sits in it. The identity group is a plain
width now rather than flexing. **Properties select:** every fold row has a hierarchical
path (`<layer>/effects/<fx>/<param>`, sharing its prefixes with the group paths), clicking a
property row selects it, and every row *containing* it — the effect's heading, the layer's
own row — marks itself a shade dimmer, which is what will tell the graph editor which curve
is meant. Boxing keyframes on a lane selects their property too. **Two Flutter traps, both
found the hard way and both now guarded:** `ScrollController.offset`/`.position` assert
when a rebuild momentarily leaves two views attached (a drop target lighting up was
enough) — read through `_positionOf`, which returns null unless exactly one is attached;
and `RawScrollbar` learns where its scrollable is from `ScrollNotification`s rising through
its *own* subtree, so one sat in a gutter beside the scroll view never repaints and its
thumb is simply invisible — replaced by `_GutterScrollbar`, which listens to the controller
and drags it directly. The outline's row seams are now one scroll-phased overlay across the
columns *and* the gutter, so they meet the lane area's. Regression tests: `a drag on a
keyframed value is one undo step`, `clicking a property selects it and marks its parents`,
`boxing keyframes on a lane selects their property`, `dragging a header seam resizes just
that group` (timeline_panel_frb_test.dart).

**K-193 · DECIDED · Layers reorder by drag, the Transform card is a choice, and Settings has pages.**
**Reordering:** a layer's name is its stack handle — drag it onto another row and it takes
that row's place, one op and one undo step. Layers were otherwise stuck in the order they
were added, movable only from the row menu one place at a time. A locked layer neither
drags nor accepts a drop. **The Transform card in Effect controls is off by default**
(Settings → Interface turns it on): the Timeline's fold-out already carries Transform, and
repeating it pushed the effect stack — what the panel is *for* — a screen down on a 3D
layer. It stays available because it is a habit After Effects users bring with them.
**Settings is paged** (General · Appearance · Interface · Performance), each page a stack
of named sections and each section a card of rows that read the same way: what it is, a
line saying what it does, its control on the right. That is the egui shell's arrangement,
restored; it replaces one scrolling column of five groups that had outgrown a window. The
rebuild also surfaces settings that existed but were never exposed — UI scale, tooltips,
the animation level, and the playback mode, all of which were being persisted while
unreachable. `Workspace.settingsChanged()` is the one call that makes an in-place edit to
`interface`/`performance` stick, since those are plain structs rather than a setter per
field. **Pages with nothing behind them are not listed:** Export defaults, the keymap
editor and colour management are unbuilt (docs/TODO.md), and an empty page is a promise
the window cannot keep. Regression tests: `dragging a layer by its name reorders the
stack` (timeline_panel_frb_test.dart) and `the pages divide the settings, and a choice
persists` (shell_frb_test.dart).

**K-194 · DECIDED · A test may not touch the real settings; budgets are typed; menus have submenus.**
**The settings-reset bug first.** `Workspace.save()` wrote to
`%APPDATA%\lumit\flutter-workspace.json` unconditionally, and every test that builds a
`Workspace` and touches a setter calls it — so a `flutter test` run wrote *defaults* over
the developer's own settings, every run. `Workspace.storeOverride` redirects the store, and
the frb test harness points it at a temp file. Machine state is not something a test run may
reach. **Cache budgets are typed numbers** (drag or type, in MB) rather than a pick from a
fixed list, capped at what the machine actually has: `system_memory_bytes()` via
`GlobalMemoryStatusEx` and `video_memory_bytes()` via the first DXGI adapter's dedicated
memory (`crates/lumit-bridge/src/api/system.rs`; both answer 0 off Windows and the frontend
falls back to a documented 16 GB ceiling rather than pretending). The old dropdown could not
express "3 GB on a 32 GB machine" and its options were a guess at what hardware would turn
up. The **Frame transport** row is deleted — it named an implementation detail the user
cannot act on. **Menus nest** (`SubmenuRow`, widgets/controls.dart): Window → Workspaces
holds the four presets and Reset, and Add effect → *category* → effect replaces one 380 px
scrolling list. The submenu opens *over* its parent rather than replacing it: closing the
parent first would take the row's `BuildContext` with it, and the overlay the submenu needs
is reached through that context. The Add-effect menu now drops from the **button** (a
`Builder` gives it its own context) instead of the panel's left edge. **Source and Retime
join Transform** behind the Settings → Interface toggle: all three describe the *layer*, and
this panel is about the effects on it. **Matte and layer-valued effect parameters offer only
layers with a picture** — `LayerReference::has_picture()`, the mirror of `has_audio`, false
for a camera and for an audio-only clip — and never the layer they sit on. Both pickers are
lazy, so the probe happens when a menu opens and never while drawing a row (K-184).
Regression tests: the settings/menu/effect tests in `shell_frb_test.dart`,
`menu_bar_frb_test.dart` and `effect_controls_frb_test.dart`.

**K-195 · DECIDED · macOS gets a Viewer picture: Metal/IOSurface is the third zero-copy
transport.** K-183 deleted the CPU read-back path and left macOS with no way to show a
frame at all — every render was composited and then dropped with "No zero-copy transport in
this build", so the Viewer was blank for a whole session while every other panel worked.
The macOS primitive for two parts of a program pointing at one piece of graphics memory is
the **IOSurface**: `lumit-gpu`'s `shared_metal` creates one (`IOSurfaceCreate`), asks Metal
for a texture backed by it (`newTextureWithDescriptor:iosurface:plane:`), and wraps that
`MTLTexture` back up as a `wgpu::Texture` the ordinary render path copies the finished frame
into. The runner (`macos/Runner/ViewerTextureBridge.swift`) looks the surface up by id,
wraps it in a `CVPixelBuffer` — a wrapper, not a copy — and registers it as a Flutter
external texture on the same `lumit/viewer_texture` channel with the same
`register`/`frameReady`/`unregister` methods the Windows and Linux runners implement.
**The payload is the Windows one, deliberately:** macOS reports `RenderedSharedTexture`,
because both platforms hand across one opaque integer naming a surface plus its size (an NT
handle there, an `IOSurfaceID` here) and neither side does anything with it but pass it on.
So there is no third bridge variant, no codegen change and no Dart change — only Linux, which
needs stride, offset and a DRM format, has its own. The surface is `'BGRA'`
(`kCVPixelFormatType_32BGRA`, the one format Flutter's macOS texture path accepts), so the
renderer is asked for BGRA display bytes there exactly as it is on Windows. Feature
`shared-texture-macos`, default-on and inert off macOS, matching its two siblings.
Regression test: `the_surface_yields_the_pixels_in_bgra_order` in `shared_metal.rs` writes
through the wgpu texture and reads back off the locked IOSurface, which is the channel-order
mistake that would otherwise cost a silent blank session (the Windows sibling's test exists
for the same reason). Extends K-177; supersedes K-183's "macOS has no Viewer picture until
it grows its own" — it has grown one. The rest of K-033's Mac release list (VideoToolbox,
ProRes, notarisation, the native menu bar) is untouched and still outstanding.

**K-196 · DECIDED · The graph editor is the AE graph, and the keyframe clipboard speaks
AE's format.** From the owner (2026-07-28), replacing the per-channel mini-lanes the frb port
shipped with the behaviour docs/07 §5 always specified. The graph is **one full-height
pane** sharing the Timeline's ruler, zoom and horizontal scroll; the curves it draws are
evaluated by a Dart port of the engine's own cubic (`flutter_ui/lib/panels/graph_maths.dart`,
pinned to `crates/lumit-core/src/anim.rs` by docs/impl/keyframe-eval.md §1–2 and held
together by golden tests), because a paint may not cross the bridge (K-184). Decisions
folded in: **(a)** property selection rides on the property's *name* in the outline —
`Ctrl` toggles, `Shift` ranges, across layers — and editing a value or keying a property
selects it too; a click elsewhere on the row selects nothing. Every selected property is a
coloured curve (the theme's `curve` palette, per axis — Position is AE's red/green pair)
and the outline label takes its curve's colour. **(b)** Wheel bindings match the lane view:
`Ctrl`+wheel zooms time about the pointer, `Shift`+wheel scrolls sideways; the value axis
auto-fits until the Auto fit toggle is off, and then a plain wheel pans it and `Alt`+wheel
zooms it. **(c)** Tangent handles are per side and joined by default: a drag swings the partner
**live and in screen space**, keeping the pixel length it had when the gesture began, and
`Alt` held at drag start flips broken/joined. Screen space, not value space, because the
two axes carry different units at independent zooms — mirroring in value space bends the
line the pair is supposed to draw and appears to stretch the partner as the tangent swings
toward vertical, which is the exact complaint that killed this in the egui frontend. For
the same reason the handles' hit targets never grow past their own reach: a handle sits a
few pixels from its key on a long composition, and a fixed target made which one you
grabbed a coin toss. The pixel length holds at *every* angle, with two supports rather
than a compromise: a tangent may never stand exactly upright (its reach is floored at a
thousandth of its span — sub-pixel at any sane zoom), because a vertical tangent covers no
time and so has no speed that describes it, which is the one state the geometry cannot
come back from; and each handle's drawn length is **remembered** per keyframe and side,
against the scales it was measured under, so swinging a pair out to near-vertical and back
returns both handles exactly as long as they went in. Reach in time is therefore allowed
to become very small at the extreme — that is what a near-upright tangent *is* — without
the length on screen following it down. One consequence is worth stating rather than
patching around: a joined partner moves when the pair **rotates**, so dragging a handle
straight out from an already-steep tangent lengthens it without turning it and the other
side barely stirs. That is the see-saw behaving, not sticking.
**(d)** The speed lens draws the exact derivative (K-080): each key is an independent
in-speed and out-speed dot, dragged vertically for that side's speed and **sideways to move
the keyframe in time**, with one influence handle each; editing either lens writes the same
speed/influence store losslessly (K-025). **(e)** The keyframe clipboard: in-app it keeps
full fidelity; the system clipboard simultaneously receives a tab-separated table headed
**`Lumit <version> Keyframe Data`** (the rate, the source size, then a property group per
copied property with a column per value) — extended with **two easing columns per value**,
`linear` / `hold` / `bezier(speed,influence)`, so shaping survives the round trip instead
of flattening. The easing columns come last, after every value, so a reader that does not
know them stops at the values it does; a foreign keyframe table with no easing columns
parses back as linear keys. Copy and paste are bound to the keyframe *selection*, not to
the graph, so they work from the lane view too. **(f)** The F9 family
(F9 / `Shift+F9` / `Ctrl+Shift+F9`) and the footer's Linear / Bezier / Hold act on the key
selection in *either* view — the lane marquee's catch included. Retime stays an ordinary
property here (per the standing TODO): no Retime channel, no §5.2 lenses yet; the
acceleration lens (K-070), numeric entry, transform-box scaling, beat-marker snapping and
waveform ghosting remain open in docs/07 §5.

**K-197 · DECIDED · Retime starts again as an ordinary keyframable property.** The segment
model (docs/04-RETIMING.md: Rate/Map segments, eases, boundaries with exact rational source
positions) is a fine *destination* and a poor starting point — it has cost more than it has
paid, and none of its editing affordances ever reached the frontend (docs/TODO.md lists a
dozen). So Retime restarts as the simplest thing that is honestly a retime: a
`lumit_core::anim::Property` on the **layer** (`Layer::retime`, `Option<Property>`) whose
value is the source time, in seconds, the layer shows at its own local time — the After
Effects Time Remap shape. It is a graph-editor channel like any other, which supersedes
K-196's "no Retime channel" — that clause meant no *segment* channel and no lenses, and
neither is what this is. Being an ordinary `Property` is the whole point: the stopwatch,
the ◄ ◆ ► navigator, the lane diamonds, the graph editor's lane, its handles and its interp
menu all work on it already, with no Retime-specific code anywhere. **No extras at all** —
no speed lens, no ease presets, no ramp editing, no freeze, no overrun band, no
interpolation policy on this path. Those return, if they return, on top of a property that
already works. `Option` rather than an always-present property because "not retimed" and
"retimed to exactly 1×" are different states in the file, and only the first skips the map:
a layer with no Retime shows **no row**, and Alt+Shift+T installs the identity map (two
linear keys, source running alongside local time) so switching it on changes nothing
visible. The row sits **above** Transform, outside every group, because it decides which
frame of the source the rest of the fold-out then transforms. `Layer::source_time_at` is the
single place the mapping is decided, so the render plan (`plan.rs`) and the frame-cache key
(`lumit-eval`) can never disagree about which source frame a layer shows; it prefers the
property and falls back to the old `LayerKind::Footage::retime` store for documents that
carry one. Supersedes K-194's "build the fold-out group and move the Source card's retime
rows into it" — the rows being moved would have been the segment card's, and this is a
different property with a different model; the Source card's speed/reverse/interpolation
rows stay where they are until the new path replaces them outright. Regression tests:
`retime_property_round_trips_and_maps_source_time` (lumit-core),
`the_retime_property_toggles_and_reads_back` (lumit-bridge), `Retime shows above Transform
only once the layer has one` (timeline_panel_frb_test.dart) and `Alt+Shift+T toggles the
selected layer's Retime` (shortcuts_frb_test.dart). The shortcut is **Alt+Shift+T**, the
owner's choice, replacing docs/07 §15's never-built `Ctrl+Alt+T`; that table is updated in
the same commit.

**K-198 · DECIDED · Retime keeps its chord and gains one the operating system cannot
take.** From the owner (2026-07-28), extending K-197 rather than reversing it. K-197's
**Alt+Shift+T** is unchanged and stays the shortcut the specs name. It also, on Windows,
collides with the system's **input-language switch**: left Alt with Shift is how Windows
cycles keyboard layouts, so on any machine with a second layout installed the OS consumes
the chord and the application never receives the T — the command appears simply not to
work, which is how this was found. Two additions, no removals: **Ctrl+Alt+T** does the same
thing (After Effects' own Time Remap chord, and the one K-197 had replaced — nothing
intercepts it), and **Composition ▸ Enable Retime / Disable Retime** carries the command in
the menus, naming what it will do to the selected layer and greyed out when there is none.
Both routes go through one `LumitState.toggleRetime`, so they cannot drift apart, and it
swallows a failed call rather than letting a menu click take the interface down. Covered by
`Ctrl+Alt+T toggles Retime as well` beside K-197's own shortcut test. The general lesson
outlives this shortcut: a chord the OS claims is not a chord the application has, so a
command whose only route is the keyboard has no route at all — every keyboard command
wants a menu or palette entry beside it.

**K-199 · DECIDED · The keymap is the engine's, the keyboard is the frontend's, and the
reveal cycle is three commands on one key.** From the owner (2026-07-29), restoring what K-182
removed and finishing what docs/07 §15 has promised since it was written. `lumit-keymap`
came back from git history unchanged — chords, contexts, conflict detection, the shipped
default and the After Effects preset, with its eight tests — because it was deleted as
unused rather than as wrong, and rewriting it would have been retyping.

**The split, and why it falls here.** Everything that has to be *decided* about a keyboard
lives in Rust: what a chord means, whether the focused panel outranks the app-wide binding,
whether two bindings clash, what the shareable file says. The frontend turns a real
`KeyEvent` into chord text (`Mod+Alt+Shift+Key`, with `Mod` resolving to Cmd on macOS and
Ctrl elsewhere), draws the table, and forwards the edits — it holds no opinion about any of
it, per K-181. The one thing it *does* decide is what counts as a gesture: the 500 ms
multi-tap window for `U` is a gesture like a double-click, and gestures are the platform's.

**Where the keymap is kept.** In the engine for the session, behind its own lock. The file
is the frontend's: `keymap_to_json`/`keymap_from_json` hand the whole map across as text and
the workspace file stores that blob verbatim, never looking inside it. One format serves
both the restore-on-launch path and the "Export keymap…" a user mails to a friend, so a
keymap that survives a restart is the same keymap that travels.

**A row shows every chord, not the first one.** An action can hold two — K-198 gives Retime
both `Alt+Shift+T` and `Ctrl+Alt+T` deliberately, and neither is removable — so
`BridgeKeyBinding` carries a list. A table that showed one of them would be lying about the
keyboard, which is the exact failure the page exists to prevent. Rebinding a row replaces
all of its chords with the one pressed; resetting restores all of them.

**Taking a chord someone else holds is never refused**, because refusing makes swapping two
actions' keys impossible — the swap needs a moment where one chord is claimed twice. Inside
one context the previous owner simply loses it and its row goes blank, which is visible;
across overlapping contexts both survive and the clash is reported for the user to resolve.

**Retime's chords moved from the Timeline context to Global**, with no change to the chords
themselves. The shell runs that command wherever focus is and the Composition menu carries
it too, so scoping it to one panel described something that was not true.

**`U` / `UU` / `UUU`** (docs/07 §4.3, and the third tap is After Effects' own behaviour
rather than a Lumit invention): animated properties, then everything modified, then shut.
Which groups qualify is answered by `LayerReference::reveal_groups` rather than worked out
in the panel — "does this hold a keyframe" and "is this changed from a fresh layer" are
facts about the document, and the second needs the layer-seeding rule that decides what
unchanged *means* for Position. The panel is told which groups to open and decides nothing
about why.

**What this does not do.** The Tools, Project, Panels and Effects contexts have bindings in
the table and no dispatch behind them yet — those commands do not exist on this frontend, so
the rows are honest about the keymap and silent in use. `docs/TODO.md` carries that.

**K-200 · DECIDED · Retime has one chord, like everything else.** From the owner (2026-07-29),
superseding the two-chord half of K-198. The owner's recollection behind K-197's
**Alt+Shift+T** was simply wrong — the After Effects chord being reached for was
**Ctrl+Alt+T** all along — so the collision K-198 worked around (Windows takes Alt+Shift
for its input-language switch) was a collision with a chord nobody should have shipped.
The remedy is now the removal: **Ctrl+Alt+T** (`Mod+Alt+T`) is Retime's one binding,
Alt+Shift+T is unbound, and no shipped action carries two chords. Retime is not special,
and with K-199's Settings → Keymap in, anyone who wants a second chord can bind one — a
per-user preference no longer needs to ship as a default. What K-198 *keeps*: the menu
route (Composition ▸ Enable/Disable Retime) and its general lesson, that every keyboard
command wants a menu or palette entry beside it. The bridge simplifies with the decision:
a keymap row carries one chord, not a list whose only customer was this pair.

**K-201 · DECIDED · The export dialogue grows the fields an export actually has, and image
sequences join the formats.** From the owner (2026-07-29). File ▸ Export… (the glossary bans
"render" for user-facing output, so the name was never a choice) now carries: a **format**
box — H.264/HEVC into `.mp4`, or a **PNG/TIFF image sequence**, one lossless RGBA still per
frame written through the same ffmpeg seam and the same frame walk as video, named
`shot.00001.png` beside the chosen path; a **frame rate** defaulting to the comp's own,
where a different rate resamples by nearest comp frame over the same wall-clock span and is
stamped exactly (`fps_rational` — 29.97 stays 2997/100, fixing the old path that rounded
every comp rate to a whole number); a **range** in comp frames defaulting to the work area
(K-037's rule stands as the default; the dialogue's explicit range wins over it, and always
sends what it shows so setting the range to the whole comp over a work area means the whole
comp); and the **AAC bitrate** when audio joins. Sequences carry no audio and no bitrate —
resolution strips both so the exporter never sees a contradiction — and a cancelled or
failed sequence deletes the frames it wrote. The dialogue's preset and codec lists now offer
only what the engine ships (the old list named `prores` and two presets that stamped
nothing). The preview-equals-export identity (K-031) is untouched: the range and rate choose
*which* comp frames render and how the file is stamped, never how a frame renders.

**K-202 · DECIDED · Themes are yours to make, and the Timeline gets a second ground.**
From the owner (2026-07-29). Four Appearance changes, one of which is a spec correction.

**Custom themes.** Settings → Appearance → **Customise…** opens every colour the theme
carries, one row each — name and a line saying what it does on the left, a swatch that
opens the picker on the right — seeded from the theme currently in use, previewing live as
you change it, because a colour you cannot see against the rest of the interface is a
colour you cannot judge. **Save** names it the first time and updates it in place after;
closing with unsaved edits asks rather than assuming, and discarding puts back exactly what
was there. A custom theme is stored as **a name, a light-or-dark base, and a bag of
colours** — not a copy of the struct — so a theme saved today still opens when Lumit grows
a token tomorrow, taking the new one from its base. Colours are written to the workspace
file as readable `#rrggbb`, so a theme can be hand-edited or pasted between machines.

The colours are declared once in `theme_tokens.dart`, each with the reader and writer that
reach its field; the editor and the stored theme both walk that list, and a test counts the
struct's colours against it so a token added and not listed fails rather than going
missing. **One colour is deliberately not offered**: the Viewer's surround, which is
strictly neutral by spec (15-DESIGN §2.1/§11) because a grade cannot be judged against a
tinted surround.

**The picker is grouped** — Dark, Light, then Custom. Seven built-ins plus a growing list of
user themes is a long flat menu, and light-or-dark is the first thing anyone chooses by.

**Scopes stop taking the theme's colours by default**, which is what 15-DESIGN §8 and §551
have said all along: a waveform is a measuring instrument, read on a near-black graticule
with a bright trace whatever the chrome, the same reasoning that keeps the Viewer surround
neutral. `ScopeColours.standard` was already in the Dart theme, correct and unused — the
panel simply never asked for it. Themed scopes remain available as an Appearance toggle,
off by default: off-spec, opt-in, and squarely a matter of taste.

**The Timeline gets two grounds, and selection its own colour.** The lane, layer and graph
areas were one long strip at a single value, which left a selected row almost nothing to
stand out against and left the span being delivered invisible below the ruler. Now the work
area keeps `surface1` and everything outside it is washed a step darker
(`timeline_out_of_range`), with a bigger step on light schemes because the same difference
reads as less on a bright ground. Selection moves off `surface2` onto its own
`selection_fill`, which lifts on a dark scheme and *drops* on a light one — a rule the
surface ramp cannot express, because it is a ramp. Both default from the mode rather than
being restated by seven schemes, and both are editable like any other token. The work
area's edges are **draggable on the ruler** for the first time on this frontend: it was
settable only from the menu, and a span you can see is one you expect to take hold of.

**K-203 · DECIDED · Selection you can get out of, a work area that exists, and a surround
that is grey.** From the owner (2026-07-29). Six defects reported against the K-199…K-202 work,
fixed together because four of them are one theme: the interface holding state the user
could no longer see or reach.

**Selection lets go.** A selected property survived its layer being twirled shut — invisible
but still the selection, so it came back lit when the layer reopened and went on colouring
that layer's row while the user worked on a different layer entirely. Closing a fold now
drops the selection inside it; clicking a layer clears the property selection, because "this
layer" means this layer and not also whatever was picked on the last one. And there is a way
out: **a click on empty ground in either half of the table deselects everything** — no
layer, no properties, no keyframes. Until now the only way to change the selection was to
pick something else, which left every command that reads it (Delete, the Retime chord, `U`)
stuck with whatever was picked last.

**`U` with nothing selected is the whole composition's.** "Show me what is animated" is a
question about the comp at least as often as about one layer; refusing to answer it unless
something was selected made the commonest use of the key the one it did not serve. The
`U`/`UU`/`UUU` cycle is unchanged — it simply runs over every layer instead of one.

**The work area is the whole comp until it is narrowed.** The engine stores "not narrowed"
as null, which is right. The *interface* has no such state: a comp that has not been
narrowed has a work area of the whole thing, which is what every editor shows and what
leaves the ends there to grab. Without it the K-202 drag handles had nothing to hang on,
the wash had nothing to shade, and `B`/`N` — bound since K-199 and dispatched by nobody —
did nothing at all, so the whole feature read as unimplemented. The two-shade ground now
runs the full height of the lane view **and the graph view**, and the ruler's ends are
draggable from the first frame. Clearing the work area no longer removes it; it widens it
back to the comp.

The read is in **frames, once per panel build**, handed down to the ruler, the lanes and the
curves rather than asked again in each — the first cut of this cost eighteen extra bridge
calls per twirl and broke the call-budget gate (docs/13).

**Ctrl+S saves.** `file.save` was in the keymap from the day the keymap came back and had no
case in the shell's dispatch, so the chord resolved to an action nobody ran and the status
line went on saying "Unsaved changes". The menu's save is now a free function both call, so
there is one path to disk rather than two to keep honest.

**The Viewer's surround is neutral again.** It was painting `surface0` — the theme's own
panel surface — where the theme has carried a neutral `viewer_surround` all along, for the
reason 15-DESIGN §2.1/§11 gives: a grade cannot be judged against a tinted surround. Neutral
is the default; taking the theme is an Appearance toggle, off by default, the same shape of
answer K-202 gave the scopes. This does not reopen K-202's decision to keep the surround out
of the theme editor: it is still not a token, it is a switch between the theme's neutral and
the theme's surface.

**The Scopes toolbar drops its frame readout.** The playhead's position is the Timeline's
and the Viewer's to state; a third copy above the trace only competed with it.

**K-204 · DECIDED · Installed memory is answerable on all three targets, and no tracked
file carries one platform's absolute path.** From two outside contributors (2026-07-29),
whose pull request is where both halves of this came from.

**The build fix first, because it was the real breakage.** `.cargo/config.toml` carried an
`[env]` block setting `FFMPEG_PKG_CONFIG_PATH` to the macOS Homebrew keg
(`/opt/homebrew/opt/ffmpeg@7/lib/pkgconfig`), because ffmpeg@7 is keg-only and pkg-config
cannot otherwise find it. Cargo's `[env]` has no per-target form, so every platform got it.
On Linux that directory does not exist, and rusty_ffmpeg's build script does not shrug and
fall back — it panics outright ("FFMPEG_PKG_CONFIG_PATH is set to `…`, which does not
exist"), so a fresh clone could not build at all until the line was deleted or overridden.
Two contributors independently deleted it. The `[env]` block is therefore **gone**, and each
platform is pointed at FFmpeg from outside the repo: macOS exports
`FFMPEG_PKG_CONFIG_PATH="$(brew --prefix ffmpeg@7)/lib/pkgconfig"` (per CI job, per
developer shell), Linux exports nothing because the distro's FFmpeg 7 development packages
already sit on pkg-config's default search path, and Windows keeps `FFMPEG_LIBS_DIR` /
`FFMPEG_INCLUDE_DIR` (rusty_ffmpeg's pkg-config branch is `cfg(not(windows))`, so it never
read the variable there). Moving the discovery into a build script of our own was
considered and rejected: the discovery lives in rusty_ffmpeg's build script, which is a
dependency of ours and therefore runs *before* anything we could write, and
`cargo::rustc-env` reaches our own compilation rather than a dependency's build script.
There is no seam without forking, so the fix is the honest one — the platform with the
unusual requirement states it, instead of every other platform undoing it.

**CI was masking the defect, which is the part that must not recur.** Both Linux jobs
exported `FFMPEG_PKG_CONFIG_PATH` themselves before building, and Cargo's `[env]` without
`force = true` does not override an already-set variable — so CI took rusty_ffmpeg's
explicit-override branch and stayed green while every real Linux clone failed. Contributors
were doing CI's job. The Linux jobs now export **`PKG_CONFIG_PATH`** instead, which is what
a distro install produces implicitly, so the branch a contributor actually takes is the
branch that gets tested, and re-adding the `[env]` line would now turn the Linux jobs red.
The pinned FFmpeg 7.1 tarball stays: the runner's own distribution still ships FFmpeg 6.
The standing lesson is the general one — a CI job that pre-sets what a contributor would
not have set is not testing the contributor's build.

**`system_memory_bytes()` now answers on Linux and macOS too**, extending the Windows-only
implementation K-194 recorded (that entry's "both answer 0 off Windows" is superseded for
this function only; `video_memory_bytes()` stays Windows-only and still answers 0
elsewhere). K-082 already makes Linux and macOS supported build targets, so this fills in a
target that was supported rather than adding one. `MemTotal:` from `/proc/meminfo` on
Linux, the `hw.memsize` sysctl on macOS, both falling through to 0 if the file, the field,
or the call does not yield a number. One honesty note recorded rather than corrected:
Linux's `MemTotal` is *usable* RAM, excluding what firmware and an integrated GPU reserved
before the kernel booted — about 15.5 GB on a 16 GB machine. It errs low, and low is the
safe direction for a cache-budget ceiling, which is the same reasoning K-194 already
applied to reporting the first adapter's video memory. Regression test:
`system_memory_bytes_reports_non_zero_on_supported_platforms` in
`crates/lumit-bridge/src/api/tests.rs`.

**K-205 · DECIDED · The renderer's backend is pinned on every platform, in every build.**
From the owner (2026-07-29), out of the Linux hybrid-GPU report. K-177 pinned the D3D12 backend
only under the opt-in `shared-texture` feature and said in as many words that "every
non-feature build keeps the all-backends instance"; the Linux and macOS siblings copied that
shape. This supersedes K-177 on that point. `GpuContext::headless` now selects **DX12 on
Windows, Vulkan on Linux and Metal on macOS unconditionally**, whatever the shared-texture
features are set to.

The reason is that the alternative no longer has a user. Zero-copy requires a pinned backend
— the hand-off reaches through wgpu to *that* backend's device — and K-183 deleted the CPU
read-back transport, so no build is left that shows frames without it. A mixed-backend
instance therefore buys nothing, and it costs something real: letting wgpu enumerate GL
alongside Vulkan on a hybrid iGPU+dGPU machine makes `PowerPreference::HighPerformance`
choose unreliably, and the reported case picked the integrated part driving the display and
then exhausted its memory during submission. Pinning is also simply honest about the
requirement, rather than leaving it implied by a feature flag that gates something else.

The pin is **not overridable from the environment**. The instance descriptor is built from
`from_env_or_default`, so `WGPU_*` still tunes the flags, the DX12 shader compiler and the
GLES version, but `backends` is set explicitly afterwards and wins: an environment variable
must not be able to put the Viewer on a backend the texture hand-off cannot use.

The three `shared-texture*` features keep their old scope — they gate the interop code, and
nothing else.

---

**K-206 · DECIDED · The Null layer ships, and the bridge enum spells it `NullLayer`.**
From the owner (2026-07-29). The Null layer (01-GLOSSARY §2, reserved in 03-DATA-MODEL §5.2 since
the model was written) is now a shipped kind: an invisible, source-less, size-less layer that
carries only a transform, so layers parent to it and move as a rig.

**The variant is `LayerKind::Null`.** `LayerKind` is serde-serialised by variant name, so the
spelling is what lands in every `.lum` file on disk; it had to be the name the docs already
reserve rather than a working name, because picking it later would mean a migration for
nobody's benefit.

**The bridge enum deviates, and only there.** `BridgeLayerKind` is code-generated into Dart,
where enum members are lower-camel-cased — `Null` would become a member called `null`, which
is a Dart reserved word and will not compile. The bridge enum therefore names the variant
`NullLayer` (Dart: `BridgeLayerKind.nullLayer`). This is a spelling forced by the target
language at the outermost edge of the system: `lumit-core` and every engine crate keep
`Null`, nothing serialised changes, and no user-facing string is affected. In the interface
the kind is named the way an Adjustment layer is named — **Null** in the Timeline's add-layer
menu, **Add null layer** in the Composition menu.

**The label palette grows to nine chips.** K-189 gave each layer kind its own starting label
colour from an eight-chip palette, and the eight were exactly taken by the seven earlier
kinds plus the neutral default at index 0. Rather than give the Null a chip that already
means something else — index 0 would have made every new Null read as "no colour chosen" —
the palette gains a ninth (coral). The chip picker now draws `LumitTheme.labelCount` chips
rather than a hand-kept literal, so the next kind costs one line in the theme.

**What a Null deliberately does not do.** It emits no node in the evaluation graph and draws
no pixels, and `has_picture` answers false for it, so it is not offered as a matte source or
as a layer-valued effect parameter — before this the catch-all arm handed it a picture it
does not have, and picking it silently produced nothing. Its transform still feeds the frame
key, so moving a Null retires the cached frames of the rig hanging off it. Two gaps are
recorded rather than closed (docs/TODO.md): a Null cannot be selected in the Viewer, because
unlike AE's 100×100 box it has no size to click; and effects added to a Null are accepted and
never run, harmless as on a Camera but neither refused nor labelled.

**CI re-runs codegen and diffs.** The checked-in Dart for this feature was stale on arrival —
the generated doc comment described wording the Rust source no longer had — because nothing
in CI ran `flutter_rust_bridge_codegen generate` and checked the tree came back unchanged. It
does now, on Linux, at the version the workspace pins. A generated file is an output, and an
output is checked by CI, not by a reviewer's eye.

**K-207 · DECIDED · The lane area is rows all the way down, the work area is a band you can
drag, and the playhead has a head.** From the owner (2026-07-29). Four defects reported against
K-202/K-203 while testing them.

**The lane area has no bottom.** The rows were laid out to their own height, so with one
layer in the comp everything below 22px was blank: no ground, no seams, and — since K-203
put deselect on the ground — nothing to click on to let go of a selection. The scrolled
content is now given at least the viewport's height, and the two-shade ground, the row
seams and the marquee run to the bottom of the panel whatever the comp holds.

**The work-area wash is drawn over the bars as well as under them.** K-202 put it under, so
it showed only in the gaps between layers — which is to say it disappeared exactly where
there was something to look at. The same wash is now painted again over the rows at reduced
strength: out of range reads as dimmed, not hidden.

**On the ruler the work area is a band in the lower half**, as 07-UI-SPEC §4.1's
top-to-bottom order always said it was, rather than a tint over the whole ruler competing
with the ticks and labels. Its handles keep the full height to grab.

**Dragging an edge no longer lags the pointer.** The handle was drawn from the work area the
*engine* returned, so every frame of the drag went out to the document and back before the
mark moved. The ruler now holds the dragged edge itself and draws from that, and commits
only when the drag crosses a frame — a pointer emits many moves per frame of travel, and
each commit costs a document write and a panel rebuild.

**Playback loops the work area** (07-UI-SPEC §10: loop work area is the default mode).
Reaching the end starts again from the start, restarted through `play` rather than by moving
the playhead, because the sound and the scheduler's clock both take their baseline from the
frame play was asked for. A comp that has not been narrowed plays to its end and stops, as
before.

**The playhead has a head** (15-DESIGN §6.5): an 11×8px accent triangle at the top of the
ruler with the line carried into it as a notch in `surface_0` — black on a dark scheme,
white on a light one. A 1px line alone reads as a row seam at a glance.

**K-208 · DECIDED · A layer drag moves both halves of the Timeline, and the two halves
measure the table once.** From the owner, reporting Airizz (2026-07-29). The Timeline's outline
and lane area are built as two columns of rows side by side, which is what makes their
horizontal scrolls independent and their layout easy to follow. Two things came out of that
which needed answering: an animation that cannot cross the seam, and a table that can be
misaligned by getting one height wrong.

**The drag state belongs to the panel, not to the outline.** The gesture is made in the
outline — the name is the stack handle — so only the outline knew a drag was in flight, and
only the outline could move: the names slid out of the way while the bars beside them sat
still. The lifted index and the index it would land on now live on the panel and are read by
both halves, which slide their blocks by one shared, tested function. In graph view there
are no lanes to move, so the outline animates alone. Transform only, ≤150ms, at the user's
animation level including zero (15-DESIGN §8).

**The row heights are worked out once per panel build and handed to both halves.** Two
measurements that must agree are two chances to disagree, which is exactly the failure
Airizz warned about: get one height wrong and the two sides of the table stop lining up.
The same walk now feeds the outline, the lanes and the drag maths.

**The outline's per-row seam is gone.** It drew a second hairline a fraction of a pixel from
the one K-192's overlay already draws, and the overlay is phased by the scroll offset — which
a trackpad leaves fractional — so the two lines pulled apart as the table scrolled and the
outline's rows read taller than the lanes.

**Zoom no longer slides before it settles.** Ctrl+wheel held the frame under the cursor by
correcting the scroll offset in a post-frame callback, which painted one whole frame at the
new width with the old offset. The jump is made in the same turn as the zoom: `jumpTo` does
not clamp, and the layout that follows already has the wider content, so the viewport clamps
it correctly on the way through.

**Not done: merging the two halves into one row widget.** Airizz's suggestion — one row
spanning outline and lanes — would make misalignment structurally impossible and any future
animation free. It also means the lane side's horizontal scrolling stops being a scroll view
and becomes an offset the rows apply themselves, taking the ruler, the cache bar, the
playhead, the work-area ground, the marquee and the graph view's scroll plumbing with it.
The requirement attached to this round was that both views behave exactly as they do now, so
the seam stays for the moment; this entry does not close the door on it.

**K-209 · DECIDED · Icons draw at 16px and land on the pixel grid.** From the owner, reporting
Airizz (2026-07-29): the icons read as crunchy, and the guess was that anti-aliasing was
missing. It was not — it was the mechanism. Iconoir's line art carries a 1.5-unit stroke on
a 24-unit grid, so an icon drawn at 12px has a 0.75px stroke, which the renderer can only
show as two part-lit pixels either side of where the line belongs. Panels had drifted to
10–13px against 15-DESIGN §5's stated 16 for panels and 20 for the transport; both are now
named constants, and 16 is recorded as a **floor** with the arithmetic that makes it one.

Icons are additionally offset half a device pixel when their stroke is an **odd** number of
device pixels wide, so a one-pixel stroke lands on a pixel centre rather than on the
boundary between two — the difference between one lit pixel and two half-lit ones, and it
applies to most of the geometry in an interface icon set. Not applied at even widths, where
the stroke already covers whole pixels and the nudge is what would blur it. At fractional
display scalings (150%) no offset makes a stroke whole; that is inherent and is stated in
the note rather than papered over.


**K-210 · DECIDED · The dropper reads a value at a pixel — not only a colour — and the
picker applies live.** From the owner (2026-07-30), asking for the egui build's two tools back in
Flutter, in the shape they had there.

**The dropper is a pixel tool, not a colour tool.** It is armed from whatever wants a value
at a point, and what it lifts is that thing's business: a colour for a Colour parameter, a
*depth* for the depth-of-field focal point. The armed state therefore carries what is being
read and a closure to write it, rather than naming a layer, an effect and a parameter index
to be re-resolved on the far side of the picture — which is what the egui build did, and the
source of its silent "the target has since moved" no-ops.

**The magnifier is fixed at 9×9 with the region inside it.** Nine pixels a side, dashed rules
between every pair, and a solid border round the pixels actually taken — the centre pixel
alone by default, grown by Shift+scroll through 3×3, 5×5, 7×7, 9×9. The ladder is odd
throughout so there is always one centre pixel, and never exceeds the grid, so the tool can
never average over pixels it is not showing. The border's corners take the theme's control
radius: rounded under the round shape, square under the sharp one, with no shape flag in the
widget. Shift+scroll no longer also zooms the Viewer — sizing the sample while the picture
moves out from under the pointer is not two features, it is one broken one.

**The magnifier belongs to the pointer being over the picture, and to nothing else.** It is
shown only while the pointer is over the drawn image — arming shows nothing until then, and a
fresh arm forgets where the last pick left the pointer, which it did not: the magnifier
appeared the instant the tool was armed, sitting where the previous pick had happened. And it
keeps one fixed offset from the pointer everywhere. It used to be clamped inside the Viewer,
so approaching the bottom-right corner it crept over the very pixels being aimed at and then
stopped following the pointer at all — a pick there is as ordinary as a pick anywhere else.
It is drawn in the application's overlay rather than in the panel's own stack, which is what
lets it hang over whatever is beside the Viewer and so need no clamp. (Both reported by the owner
on testing.)

The **window's** edge is the one exception, and it is answered by flipping rather than sliding
(the owner, asked for explicitly): the viewfinder goes to the other side of the pointer on whichever
axis would run off — above instead of below, left instead of right, each axis independently —
at the same distance, so it still never creeps over the pixel being read. Only a window with
room for neither side clamps, because half a magnifier beats none. The bound is the **window's
content area, not the display's**: an application cannot paint outside its own window, so a
magnifier past the screen edge is one the window would have clipped anyway — and where the
window sits on the display is not something Flutter reports without a windowing plugin, which
would buy no extra room.

**Living in the overlay means the panel's rebuilds are not the magnifier's.** Where it goes is
worked out when the **pointer moves** — the one moment both trees are settled — and used
afterwards as plain numbers. Asking render objects where they are from inside the overlay's own
build, and marking that overlay dirty from inside the panel's build, are both wrong for the
same reason, and an ordinary scroll over the Viewer did both: the wheel zooms the picture, the
panel relays out, and the magnifier tried to place itself against a tree mid-rebuild — a red
window and `'attached': is not true` (the owner, on testing). Nothing that places it touches a render
object now, and a redraw asked for during a build is deferred to after the frame.

**The strip under the grid says what is about to be taken, in the terms of the thing being
picked.** A colour pick shows the colour and its numbers. A pick reading something else shows
**the layer the numbers come from and the value found there** — a swatch of the composite
would be a colour nobody is choosing.

**A layer pick samples that layer rendered alone.** The egui build read the depth from the
composite's luma, or from a stashed copy of the layer's decoded pixels; the composite is
wrong (a depth pass is nearly always hidden, so it contributes nothing to it) and the stash
was a second path to keep in step. The worker instead renders the composition with that layer
soloed and visible, on a *patched copy* of the snapshot that never goes near `commit`, and
holds one such render against `(comp, frame, layer, cache generation)` so dragging the pointer
renders once rather than once per move. `depth_invert` is applied at the pick, so the caption
and the committed number cannot disagree.

**The pixels cross as a window, not a frame — and not a pixel.** A `sample_pixels` request
answers with a **129×129 square** of the picture (66 KiB) on the same stream the frames and
traces ride, and the frontend cuts the magnifier's own 9×9 out of it. Moving the pointer and
changing the sample size then cost nothing at all; a read happens only when the pointer nears
the window's edge, the playhead moves, an edit lands, or a different layer is being read. The
first cut of this asked per mouse move — a request, a cache lookup and a stream message at
pointer rate, each one cloning the whole eight-megabyte frame to copy 81 pixels out of the
middle (the owner, on testing: "a crazy number of calls"). Two fixes, both kept: the window, and
`framecache::with_best_frame`, which hands a reader the pixels **in place** under the cache
lock instead of cloning them — bounded, pure-CPU, nowhere near the GPU or the FFI boundary.

**A read is asked for as a fraction of the picture, never as a pixel.** The picture the engine
reads is a reduced-resolution preview whenever the Viewer is showing one, so its pixel grid is
neither the composition's nor anything the frontend can know in advance. The first cut of the
window asked in composition pixels and then indexed the reply with them: with a fitted Viewer
the two grids differ by the preview scale, every index fell outside the window, each one
clamped to the nearest edge — and the magnifier showed nine by nine of the *same* pixel, which
reads as a flat average of the area (the owner, on testing: "just an average of all the values in
it… and it might not be aligned"). The request now carries `(u, v)` in 0–1, the reply says
which raster it cut from, and every pixel either side names is in that raster; asking in the
wrong grid is no longer expressible. The clamp went too: a position outside the window answers
**nothing** rather than its nearest edge, so the next such mistake shows as blank cells instead
of a plausible colour. (Beyond the *picture's* border nothing changes — the window carries the
picture's own edge repeats, so those positions are inside it and answer normally.)

The size is chosen to sit between the two failure modes. Whole-picture-once — the obvious
answer — is 8 MiB at 1080p and 33 MiB at 4K, an 8.8 ms codec stall (35 ms at 4K) on arming and
that much held while armed, which is the very transport K-183 deleted, reintroduced through a
tool. A window is two orders of magnitude smaller, and one still lasts sixty pixels of pointer
travel in any direction. The cap is enforced engine-side rather than trusted from the caller,
so no request can turn this into a frame transport by the back door. It is a worker request
rather than a synchronous call because the pixels only exist where the renderer does, and the
renderer is owned outright by the worker thread — a sync call would have to render on Dart's
UI isolate or take a lock across GPU work, both of which docs/14 forbids.

**The picker's numbers are in the scale of the thing being edited, and a channel may exceed
1.** A display colour — a theme colour, a solid's swatch — is eight bits, so it reads 0–255 and
its hex is the same value said another way. A scene-linear colour in a float working depth
(fp16 today, docs/06 §3.1) reads 0–1 for black to white, as decimals, and may go **above 1** or
below 0 as far as the parameter's own declared range allows: several built-ins declare 0–4
precisely because HDR tints are legal in linear light, and one declares −1 for a lift. A 0–255
dial cannot express those at all, so the picker was silently clamping values the engine carries
happily (the owner, on testing). The square and the strip stay 0–1 — they are a chromaticity
picture — and an over-range colour is carried through them as a gain on its brightest channel,
so dragging about on the square does not quietly throw the overshoot away.

**The hex box stays, clipped, and says so.** Hex is an eight-bit display notation and cannot
say 1.8. Hiding the box on the float scale loses the one notation people actually exchange
colours in; showing a clipped hex silently would let it read as the truth. So it shows the
colour clipped into 0–1, typing one sets exactly those values, and a line under the swatches
appears whenever a channel is outside the range the swatch and the box can show. When the
project depth switch lands (docs/06 §3.1, not built), an 8 bpc project is what puts an effect
colour back on the 0–255 scale; nothing else needs to change.

**The picker applies to the document as it changes.** R, G and B sit above the graph, each
drag-scrubbable and typeable, and every edit — a number, the square, the strip, the hex box —
previews live and settles into one undoable edit, the same staged-drag shape the effect rows
use. Because the live value *is* the document's, closing the picker needs no decision:
clicking away from it keeps what is applied, and so does Apply. **Cancel** is the one button
that changes anything on the way out — it writes back the colour the picker opened with. A
plain "Close" was considered and rejected: with live application it would be indistinguishable
from Apply, so it would be a button that promises a choice it does not have.

**Not done here:** the x/y position pick for coordinate-valued parameter pairs. The magnifier
carries the mode, but no Flutter row pairs x and y into one control yet, so there is nothing
to arm it from; recorded in docs/TODO.md rather than half-wired.
**K-211 · DECIDED · A layer's ends are handles, and its source is the limit.** From the
owner (2026-07-30; numbered K-210 while it was in review, and renumbered on the merge that
gave that number to the dropper): the start and end of a layer must be draggable to change its length,
for every layer kind — and a Footage, audio or Precomp layer must not be draggable to show
what its source does not hold, unless it is retimed.

**Trimming for every kind.** Dragging either end of a bar trims that end; dragging its
middle moves it, as before. The grab zone at each end is 8px but never more than a third of
the bar, which is what makes a short bar draggable at all: at a flat 6px a two-frame bar was
entirely edge, so it could be trimmed but never moved. The pointer takes the horizontal
resize arrow over each zone, because an affordance nobody can see is one nobody uses — the
gesture existed before this entry and the report was that it did not.

**The source is the limit.** A layer whose source has a length of its own trims within it:
the in point stops at the source's first frame (the layer's own time zero, which is where
`start_offset` puts it on the comp timeline) and the out point stops at its last. That is
Footage — picture and sound alike — and Precomp, whose length is the nested comp's duration.
Every generated kind (Solid, Text, Adjustment, Null, Camera, Sequence) has nothing to run
out of and trims freely. **Retime takes both limits off** (docs/04-RETIMING.md): a retimed
layer maps its own local time onto source time, so its length stops being the source's
business and it stretches as far as it is dragged. Both routes to a retime count — the
Retime property (K-197) and the Source card's older speed map — because both make the same
promise. Moving a bar is never limited: `start_offset` travels with it.

**A bound never drags an edge that is already outside it.** A layer stretched while retimed
and then un-retimed keeps the length it has; the limit holds its end still rather than
snapping it back, and pulling back towards the source is always allowed. Anything else would
silently destroy work on a switch toggle.

**Media that will not read leaves the ends free.** A missing file, or a build with no media
feature, answers "no length" — and no length means no limit, never a limit guessed at. A
layer must never be cropped by the absence of an answer.

**The marks: a small triangle in the top corner of the bar** on the side that is at its
limit, drawn in the same ink as the clip splits so the bar keeps one vocabulary. Present
only on the kinds that have a source, absent the moment Retime is on — the mark and the
rule are the same fact, so they can never disagree on screen.

**Where the rule lives.** In the panel, not the engine. `SetLayerSpan` still accepts any
span that is not inverted: AE import, project load and `trim_to_source_end` all legitimately
write spans the drag would refuse, and an op that second-guesses its caller would break
them. The gesture is what is bounded, and the bound is a pure function with its own tests.

**What it costs per rebuild: nothing (K-184).** A footage length comes from probing the file,
so it is asked once per layer off the build and kept for the session, like the waveform
peaks. The rest — a precomp's duration, whether Retime is on, where the start offset sits —
is worked out once per *document revision* and cached; `CompModel` now exposes that revision
so a panel can cache anything derived from the model honestly. Frames come from exact
integer arithmetic on the comp's rate rather than a `frame_at` call per layer per frame.

**K-212 · DECIDED · Letting go of Retime re-hangs the layer on its source, and a
trimmed layer shows how far its media reaches.** From the owner (2026-07-30), refining
K-211. Two halves, both about the same thing: a layer's relationship with the material
behind it should be visible, and should survive being switched about.

**Switching Retime off re-anchors the layer.** While it is retimed a layer can be any
length, because it chooses which source moment each of its own frames shows; when the map
goes away it plays at source rate again and has to be given a length. Holding the stretched
length (K-211's first answer) was wrong: it left the layer showing material the source does
not have. The rule now is the frame already on screen. The layer keeps its **in point** and
shows the **same frame** there — so if that was the source's first frame it simply starts
from the beginning, and if it was some way in it carries on from there. From that anchor it
runs at source rate until either the source runs out or its own out point arrives,
**whichever comes first**. It never grows: a layer trimmed short stays short. One undo step
covers the removal and the span together.

The anchor is snapped to the **comp's** frame grid rather than kept at full precision. The
start offset it produces is what every later trim measures from, and an offset sitting
between two frames puts the layer's own zero between two frames for good; the timeline edits
in whole frames, so the anchor does too. Both routes to a retime behave identically — the
Retime property (K-197) and the Source card's speed map — because both make the same promise.

`unretimed_span` is a pure function in `lumit-core::ops`, next to `edit_layer_span`: this is
span arithmetic, and it is the kind of rule that must be provable rather than observed. The
bridge supplies the two facts it cannot derive — the source moment showing at the in point,
read through the map that is about to go, and the source's own length. No readable length
(missing media, a build without the media feature) re-anchors and leaves the out point
alone, the same "no length is never a guessed length" rule K-211 set.

**A trimmed layer shows its source's reach.** A Footage, audio or Precomp layer that is not
retimed and does not fill its source draws a **faint outlined rectangle spanning the whole
source**, behind the bar, in the layer's own label colour. What shows past each end is
exactly the material trimmed away, and because it is drawn behind, the layer reads as one
clip with the middle solid rather than as three objects. It is absent when the bar already
fills the source, absent on the kinds with no source, and absent under Retime, where "the
source's reach" is not a fact about the layer at all. It sits with K-211's corner triangles
in one vocabulary: a triangle says *this end can go no further*, the ghost says *this end
could go further, and this is how far*.

**Both marks travel with a move.** They are drawn from the source's reach, which moves with
the layer: sliding a bar along the timeline carries its start offset, so the bounds slide
with it. Drawn from the document's bounds alone, a bar being moved appeared to leave its
limit behind — the fix is one shift applied to both marks while a move is in flight.

**The trap the ghost set, recorded because it cost a working gesture.** The outline is a
second child of the bar's `Stack`, and it appears the moment a trim starts. Unkeyed,
Flutter matches a `Stack`'s children by position, so the ghost arriving took the bar's slot
and the bar's element — with it the recogniser holding the drag in the gesture arena — was
rebuilt from scratch mid-gesture. The bar moved by the first pointer event's worth of
frames and then went dead: "dragging a footage edge only moves one frame". Both children
carry keys now. It only ever bit the source-backed kinds, because they are the only ones
with a ghost to appear, and only when the pointer moved in more than one event — which is
why the first round of tests, each dragging in a single synthetic step, all passed.

**K-213 · DECIDED · Keyframes live in the layer's time and cross on the composition's
clock.** From the owner (2026-07-30): switching Retime on put its two keys "where the start
and end points would be if the layer's position was still at the start of the comp". They
were, and so was every other keyframe on any layer that had been moved — the report caught a
seam fault whose only unmissable face is the two keys Retime creates for you.

**The engine keys properties in layer-local time, and that is right.** Every evaluation
path — the render plan, the transform sampler, the cache-key hasher — reads a property at
`comp time − start_offset`, so a layer's animation travels with the layer when it is
dragged along the timeline. That is the behaviour an editor must have; nothing about it
changes.

**The frontend thinks in comp frames, and that is also right.** The ruler counts comp
frames, a lane is drawn against the comp's axis, and a key drag commits where the pointer
is. Asking the interface to hold two clocks would put the conversion in every row, lane,
curve and field that touches a key.

**So the bridge converts, and it is the only place that does.** `BridgeScalar::read_at`
carries each key out by the owning layer's `start_offset`; `animation_at` carries it back.
Both take the offset as an argument with no default, so a new reader cannot quietly forget
one — the compiler asked for it at all forty-odd call sites when the signatures changed.
Everything that crosses carrying keys goes through them: the transform group, the Retime
property, effect parameters, a camera's zoom, a volume curve, and the staged
`BridgeEffectInstance` — which now carries its layer's offset, because a handle read out of
a layer is the only place that fact is known. `BridgeEffectInstance::new` stopped being
exposed to Dart in the same move: it was never called from there, and a constructor with no
layer would have no honest offset to take.

**Retime's two keys span the layer's own in and out.** `Layer::identity_retime` took a
duration and keyed zero and that duration; it now takes the layer's local in and out points.
Two faults in one: on screen the keys sat at the start of the composition, and in the model
a trimmed layer's map stopped short of its tail — past the last key a property holds, so
everything beyond `duration` played one frame over and over. Spanning the real range fixes
both, and keeps the promise that switching Retime on changes nothing visible.

**Not done: the pre-K-197 speed map.** The Source card's segment store has the same
"identity across `0..duration`" shape and the same tail problem on a trimmed layer. It is
not keyframed, so nothing draws it in the wrong place, and it is the arm the ponytail
comment in `Layer::source_time_at` marks for deletion; it is left alone rather than being
half-migrated. Recorded here so the next person meets it deliberately.

**K-214 · DECIDED · The frame cache is named by content, and its three tiers are one ladder.**
Requested by the owner (2026-07-30), from two complaints that turned out to be the same one: "a lot
of things are resetting the cache when they shouldn't — moving the work area, adding audio to a
layer, changing the opacity of a hidden layer", and "when I undo, we shouldn't have to cache
from scratch again". Both are the cost of positional keying, which K-178 recorded as an interim
and this entry closes.

**Every tier is keyed by the content hash the specification always asked for** (docs/06 §5.2),
not by `(comp, frame, scale)`. A positional name does not change when the picture does, so the
only safe answer to a committed edit was to drop every held frame of every composition — and
the price was paid on exactly the edits that cannot change a pixel. There is now no
invalidation step anywhere: an edit renames the frames it changed and leaves the rest
addressable, so a rename keeps the bar green and an undo finds its frames still filed under the
names the restored document asks for. It also makes the disk tier honest, which is why the
TODO listed content keying as its blocker: a frame parked under a positional name would serve
the picture from before an edit, or from another day's document entirely.

**The key gained a layer's inherited parent chain, and `ALGO_VERSION` went to 2.** A hidden
layer contributes nothing — it draws nothing — but its children still follow it, so moving a
hidden Null changed the picture while leaving every name alone. Harmless while everything was
discarded on every edit; a stale-frame bug the moment names started surviving. K-206 makes it
the common case rather than a corner: a Null is the layer a user will most readily hide.

**The demotion ladder runs both ways, and the read-back is asynchronous.** A frame evicted from
the card is read back into memory and written behind to disk; a frame held below is uploaded
straight back into a texture instead of being composited again. The upward half is what makes
the lower tiers worth having at all — before it, nothing could turn held bytes into a picture
the Viewer shows.

**Deviation from docs/06 §5.3, recorded rather than hidden: there is no cost threshold on
demoting.** The specification says to read a frame back only when its recompute cost exceeds
the read-back's, which is the right idea; the number to compare is not available. A composite
is *submitted* to the graphics card and the call returns, so the wall-clock a renderer can
measure around it is the submit, not the work — a frame that costs the card 8 ms can measure
under one, and a threshold on that gates the ladder on noise. What bounds the traffic instead
is a ceiling of four read-backs in flight, which bounds it directly; the measured cost is still
used for eviction *ordering*, where a comparative number is good enough. Two derived rules: a
frame promoted up is never demoted again (it is already below), and a frame goes to disk on the
way down rather than when memory later forgets it.

**The disk cache lives in Lumit's own cache folder by default**, keyed by the document id,
rather than in the `<project>.lum-cache/` sidecar docs/06 §5.4 describes. The sidecar only works
once a project *has* a file, and a project should cache from the moment it is created; the
document id is in the `.lum` and survives every save. Both other options are offered in
Settings → Performance — beside the project (the per-project choice) and a folder the user picks
— and changing the setting moves nothing, since a cache folder is deletable at any time with no
correctness effect. A per-project override stored inside the `.lum` is left in the backlog.

**Clearing the disk tier asks first**; the other two do not. RAM and VRAM cost a re-render each,
while this one deletes files that may represent a night's work and there is nothing to undo.
With nothing parked it does not ask.

**The cache bar became a published strip rather than a query** (docs/06 §5.6's "lock-free bitmap
snapshot", which was always the design). Naming a frame needs the renderer's probe results and a
hash per frame, so the interface cannot answer its own question: the bar records what it is
drawing and the worker publishes the strip for it. Consequences stated rather than papered over —
up to ~150 ms stale, blank for a beat after a composition switch, and sampled on a composition
longer than about a thousand frames, because the stripe is a thousand pixels wide at most. Its
values grew from three to five: nothing, held-coarser, held, on-disk-coarser, on-disk, with
playable outranking promotable.

**The card's tier and memory's share one colour on the bar** (mint), because they answer the same
question — does this frame play now — and a frame in memory is one upload from the screen. Which
of the two holds it is the status line meter's business, where each tier has its own bar.

**K-215 · DECIDED · The three follow-ups K-214 left in the backlog are closed.** Requested by
the owner (2026-07-30): implement what the TODO named rather than leaving it. Each was a stated gap,
and each is a different kind of gap.

**The disk tier has an index, so it evicts by the same rule as the tiers above it.** It held
nothing beside its files, so the only thing it could sort by was the modification time a
filesystem happens to remember — it deleted the oldest frame even when that frame had cost half a
second to render and its neighbour two milliseconds. It now records size, recompute cost, last use
and quality per entry, so presence and the byte total cost nothing at run time and eviction is the
spec's stale × large ÷ cheap-to-remake (docs/06 §5.3) from the top of the ladder to the bottom.

Two files rather than one, which is the interesting part: a snapshot (`index.bin`) rewritten now
and then, and a log (`index.log`) with one fixed-size record appended per change. A snapshot
rewritten per change rewrites megabytes to record one frame; a snapshot rewritten only
occasionally loses what happened since, and those losses are *worse than forgotten* — the files
remain on disk taking up room nothing knows to reclaim. Opening replays the log over the
snapshot; a record torn by a crash is a partial trailing record and is discarded by length, which
is what fixed-size records buy. Either file missing or unreadable falls back to walking the
folder, which is §5.4's "rebuilt by scan if missing or corrupt".

**A deviation from §5.4, recorded rather than silent: not SQLite.** The spec says `index.db`. This
is a flat map of fixed-size rows read once at start-up and otherwise held in memory; SQLite would
put a C dependency into an engine crate to store it, and the media frame index (docs/10 §3)
already sets the house precedent of a plain binary sidecar.

**The cache bar converges on per-frame truth instead of staying sampled.** Naming a frame means
hashing the composition at that time, so a ten-minute composition is tens of thousands of hashes
and the first version sampled one frame in forty. Two passes now: the sampled pass still runs
first, because the bar owes an answer for the whole composition at once — a stripe filling in from
one end reads as the *cache* filling in from one end — and a refinement pass then walks the strip
in bounded chunks replacing each sample with the frames it stood for, starting at the frame last
shown and wrapping so the region under the playhead firms up first. A composition short enough to
name in one go has a stride of one and is exact on the first pass. Only a **held** sample paints
the frames it skipped: painting a stride green off one held frame and correcting it a moment later
would flash cache the user does not have.

**Where a project parks its frames can now be the project's own answer.** Application-wide is the
right default and was the wrong only-option: a project living on a scratch drive wants its frames
on that drive, and a project handed to someone else should carry the choice with it. So
`Document` gains `cache_location: Option<CacheLocation>` — `None` meaning "follow the
application", which is the ordinary case and stores nothing in the file — set through an ordinary
op, so it is undoable, journalled and saved like any other edit, and it travels with a copy of the
project in a way a settings-file entry could not. Settings → Performance gained an **Applies to**
row (*Everything* / *This project*); switching back to Everything clears the project's answer
rather than copying the application's into it, so the project follows along afterwards. A
project's answer overrides the application's, and changing either moves nothing — the frames in
the old folder simply stop being addressed, and that folder is deletable at any time.

**Not done, and deliberately: nothing about this is in the pull request for K-214.** the owner asked
for these on a branch of their own so the reviewable change stays the one that was reviewed.
**K-216 · DECIDED · The toolbar is one strip under the menu bar, and it ships with the whole
tool set whether or not each tool works yet.** From the owner (2026-07-31): the toolbar had
no counterpart at all on the Flutter frontend — the egui shell's tool row did not survive the
port, so the only tools that existed were the ones a panel happened to offer.

**Where it lives.** A single strip spanning the window below the menu bar and above the dock
(docs/07 §1.7). It is chrome: it cannot be closed, docked, tabbed or floated, and it is the
same strip in every workspace. Its right-hand end carries the two switches that belong to no
panel — snapping, and the workspace strip §1.4 has always required in the window chrome and
which until now existed only inside the Window menu.

**The tool set is After Effects', grouped as After Effects groups it.** Selection, Hand,
Zoom, Rotation, Anchor point, Razor, the five shape tools, the five pen tools, two type
tools, brush/clone stamp/eraser, roto brush/refine edge, four puppet pins, three camera
tools. A group is one button showing the member last used with a corner triangle; hold or
right-click for the rest; the shortcut arms the remembered member and, pressed again while
that group is armed, steps to the next and wraps. That last rule is why a tool chord is not
simply "select this tool": `Q` walking the shapes without opening a flyout is the gesture
the audience arrives with.

**Unbuilt tools are drawn, not hidden.** Only Selection and Hand do anything today; the rest
change the Viewer's pointer and nothing else. Shipping the strip with only those two would
have taught the wrong shape of the application and left no agreed place to put the rest as
they land — so the specified set is on screen, and each unbuilt tool's tooltip says plainly
that arming it changes nothing yet. The alternative — a tooltip that lies by omission — is
the one thing a toolbar must not do, because a toolbar is how a user learns what an editor
can do at all.

**The chords go through the keymap, and the Tools context is asked last.** The engine already
shipped `tool.select` … `tool.pen` in a `Tools` key context with nothing behind them (K-199);
this branch adds `tool.rotate`, `tool.type`, `tool.paint`, `tool.roto`, `tool.puppet` and
`tool.camera` beside them and gives all thirteen a frontend. AE's own chords wherever Lumit
has not already spent the key — `W` rotates, `Alt+W` is the roto brush — and `Shift+C` for
the camera group, because `C` was given to the razor in docs/07 §15 long before there was a
camera tool and moving either would break a keyboard people already have in their hands.
`Tools` is not a panel, so no focused pane ever *is* that context: a chord resolves against
the focused panel and the global table first, and reaches the `Tools` table only if both
decline. That ordering is what lets `C` cut a clip in the Timeline and arm the razor
everywhere else without either binding knowing about the other.

**The armed tool is session state.** Not project state — arming a tool changes no document —
and not workspace state either, so it is not written to the layout file; every session opens
on Selection, as AE does. It lives on the shell's UI state beside the dropper's arm, for the
same reason: it is set in one place and read in another, and no panel should have to be
mounted for either.

**K-217 · DECIDED · A layer is something you can see the edges of, point at, and take hold
of.** From the owner (2026-07-31), specifying the first two toolbar tools: the Selection tool
should drag the layer under the pointer, hovering one should say so before the click lands,
several should be selectable at once, and the Hand tool should move the picture and never
the layer. Everything here follows from that.

**The wireframe is the layer's own rectangle, put through its transform.** Not the comp's:
the box the Viewer drew before this was the comp rectangle for every kind, which is only
right when the layer happens to fill the frame. A layer's rectangle comes from what it is
made of — a clip's frame size, a solid's dimensions, a nested comp's size — and is
comp-sized for the kinds with no content of their own. A Null gets a 100×100 box of its own,
because "no pixels" must not mean "cannot be selected": rigging is exactly the job that
needs to grab one.

**Content sizes are the frontend's to cache, not the read model's to carry.** A clip's size
is a question about a *file*, and the honest answer needs FFmpeg — which is disk work and
asynchronous, so it cannot sit in `get_model`, which runs on every document change. So the
Viewer holds a small cache: cheap kinds are read from the document and dropped whenever the
revision moves; a clip is probed once per session, and the layer falls back to the comp's
size until the answer lands. This is the same fallback the engine itself uses when it places
a clip it cannot probe, so a missing file is a full-frame box rather than a box of nothing.

**Selection is a list, and `selectedLayer` is its first entry.** Almost everything in the
application acts on one layer and reads that notifier directly; teaching forty call sites to
take the head of a list would have been a large change with nothing to show for it. So the
list lives beside it, and setting the single one — which the Timeline and every test do —
makes that layer the whole selection. Delete now takes the whole selection: with several
boxes on the picture, deleting one of them would be a surprise every time.

**What a press means is decided by where it went down, not where the drag was recognised.**
Flutter reports a drag's start position *after* the pointer has travelled its slop, which is
already further than a 9px handle is wide — every handle drag would have been read as a drag
of the layer's body, and the first version of this was. The press point is recorded
separately, and the slop's travel is folded into the drag so a move does not lag the pointer
by it for the rest of the gesture.

**The marquee takes only what it wholly contains.** After Effects' rule, and the one that
makes a sloppy sweep predictable: a rectangle that merely clips a corner of a layer is far
more likely to be an accident than an intention.

**The layer-controls switch hides the marks, not the mouse.** The Viewer bar's new switch
governs drawing only — clicks and drags still select and move with it off. It exists because
a grade cannot be judged with a box and eight handles over the picture, which is the same
reason the surround is neutral (K-203); it is not a way of putting the tools down, and After
Effects' Show Layer Controls behaves the same.

**A preview may fail; a gesture may not.** The provisional picture during a drag is a
courtesy, and asking for it crosses the bridge, where anything can throw (a stopped worker,
a machine with no adapter). It threw out of the pointer handler and killed the drag
mid-stroke, commit and all. Every preview and every commit in the gizmo is guarded now: the
boxes follow the pointer and the edit lands whatever the renderer is doing.

**What is deliberately not built yet.** A multiple selection moves but grows no handles —
scaling a set about one shared box is different maths, not a smaller version of this one. A
keyframed position draws no box at all, because the read model carries the curve rather than
its value at the playhead, and a box in the wrong place is worse than none. The anchor
handle, snapping, parent-aware and 3D gizmos, and motion paths are unchanged from before
this entry: still specified, still absent, now listed in docs/TODO.md against §2.3.

**K-218 · DECIDED · Every zoom is anchored, and — except the wheel — every zoom is a
flight.** From the owner (2026-07-31), specifying the Zoom tool and asking for zooming across
the interface to stop jumping.

**One piece of arithmetic, three gestures.** The wheel, a click of the Zoom tool and a
dragged box are the same question — what magnification and pan put *this* where I want it —
so they go through the same two pure functions rather than each growing its own version. The
click doubles about the point clicked (`Alt` halves), which is After Effects' step and the
one the magnification menu's own list walks; the box fits its rectangle to the panel and
centres it, and `Alt`+box is the exact inverse, shrinking the whole view into the box's
footprint. Being an exact inverse is the point: `Alt` undoes the sweep you have just made
rather than being a differently-sized guess at it.

**Anchoring is the property worth testing.** "The comp point you aimed at does not move" is
what makes zooming feel like leaning in rather than teleporting, and it is a property a unit
test can state in one line for all three gestures. The tests assert exactly that, not a
table of expected numbers.

**Zooming animates, and the interpolation is geometric.** A magnification change is a *place*
changing, not a value being nudged: cutting from one magnification to another loses the
reader's place, which is the very thing anchoring exists to keep. So it travels, over the
shell's own motion duration (15-DESIGN §7), and cuts instantly under *No animation* — the
setting means what it says. The interpolation is on the logarithm of the magnification,
because magnification is a ratio: lerping the number itself makes the first half of a 1× → 8×
flight bolt and the second half crawl. The magnification and the pan are animated together
from one controller, because animating them separately lets the anchor point drift mid-flight
— which is the whole promise, broken in the middle where it is most visible.

**The wheel stays instant.** It already arrives as a stream of small steps; animating each of
them puts the picture behind the fingers. A gesture that is itself continuous does not want a
second continuity laid over it.

**The frame is re-asked for at the end of the flight, not during it.** The engine renders at
the size the picture is *shown* at, so the frame in hand is the wrong resolution once the
magnification has changed — but a render per frame of a 120 ms animation is a render per
frame for nothing, since the intermediate ones are stretched by less than the eye can hold.

**Not done: the same treatment everywhere else.** The Timeline's time zoom, the graph
editor's, and the Project panel's thumbnail scaling all still jump. They want the same
pattern — a target, a controller, a geometric interpolation, and the animation level deciding
whether it runs — and it should be lifted into one shared piece rather than written three
more times. Recorded in docs/TODO.md.

**K-219 · DECIDED · The Rotation tool turns the selection about each layer's own anchor, and
its pointer is drawn rather than picked.** From the owner (2026-07-31), specifying the fourth
toolbar tool: the cursor should be a curved arrow like After Effects', sharper at a corner
than along an edge, leaning the way the layer would turn — and the drag should turn only what
is selected, about the anchor, with `Shift` locking to 45°.

**The pointer is painted, and the system one is hidden under it.** No desktop platform ships
a rotate cursor, and Flutter can only ask for cursors the platform has — so this draws its
own over the picture. Hiding the real pointer is not a thing to do lightly, and it is worth
it here for one reason: a drawn pointer can *turn*. The arc faces away from the layer's
anchor, so it always curves round the pin the layer spins on, and it closes up towards a
corner and opens out along an edge — so the pointer says where you are on the layer without
your looking away from the picture. A cursor that could only sit there in one orientation
would not have been worth hiding anything for.

**Corner-ness is measured in the layer's own space.** The arc's width comes from how equally
far out the two axes are from the middle: equal is the diagonal, which is a corner; one alone
is square out from an edge. Measured through the layer's inverse map, so "the top-right
corner" stays the top-right corner of the *layer* when the layer is upside down — the same
reasoning as the wireframe's hit-testing (K-217), and the same one line of maths.

**A set turns as one gesture.** The angle is swept about the *first* selected layer's anchor
and applied to every selected layer, each of which still turns about its own anchor. The
alternative — each layer measuring its own angle from the same pointer — makes a multiple
selection fly apart the moment the anchors differ, which is not a rotation of anything.

**Clicking selects, as it does with the Selection tool.** After Effects' behaviour, and
without it every turn would mean a trip back to the toolbar to choose the next layer.

**The anchor is marked while the tool is armed.** A rotation about a point you cannot see is
a rotation you cannot predict, and the anchor is exactly the thing this tool is *about*. It
is drawn as the ring-and-cross the anchor-point tool's icon carries, so the two read as one
idea.

**The same slop trap as K-217, in a new place.** The turn is measured from where the pointer
went *down*, not from where the framework recognised the drag: recognition happens after ~18
pixels, and measuring from there took a fixed bite out of every turn — a quarter-turn drag
committed 45° instead of 90°. Recorded twice now because it will keep coming: any gesture
whose *meaning* depends on its start point has to record that start point itself.

**K-220 · DECIDED · The Anchor point tool pans behind, and the Razor cuts under the blade.**
From the owner (2026-07-31), asking what After Effects does with these two and for the same
behaviour here. The answers differ, because only one of them is an After Effects tool at all.

**Anchor point is AE's Pan Behind, and the name is the behaviour.** Dragging a layer's anchor
naively moves the layer, because Position places the anchor: change the anchor and the same
Position means somewhere else. The tool moves the anchor *and* compensates Position by
exactly the amount that cancels the jump, so the pivot slides while the picture stays still.
The maths is `pan_behind_position`, ported from the egui frontend's anchor overlay and
already unit-tested — this branch supplies the gesture round it, not new geometry.

Two modifiers, both AE's. `Shift` locks the drag to one **screen** axis (the lock is about
the hand's gesture, not the layer's axes, so it stays straight across the screen on a turned
layer). `Ctrl`/`Cmd` snaps the anchor to the layer's nine key points — corners, edge
midpoints, centre — measured in **screen** pixels, because a layer at 10% would otherwise
snap from half a screen away and one at 1000% would never snap at all. That is the same rule
docs/07 §4.5 sets for the Timeline: the distance a user can see is the distance that counts.

**One op for four properties.** The anchor pair and the position pair are only meaningful
together here — committing half of this edit *moves the picture*, which is the one thing the
tool promises not to do — so it goes through `set_transforms`, which exists for exactly this
and makes the drag one undo step.

**The Razor is not an After Effects tool, and copying Premiere is right.** AE has no razor:
it splits layers with `Ctrl+Shift+D` at the playhead, and the toolbar key `C` is its camera
cycle. Lumit has a razor because it has Sequence layers — the Vegas-style cutting surface
(K-071) — so the tool is Premiere's, and Premiere's razor cuts **where you click**, not where
the playhead is. The old behaviour (click a bar, cut at the playhead) made the tool a slower
way of pressing the shortcut; docs/07 §4.4 has always said "click a clip to cut it at that
time", and now it does. `Shift`-click cuts every layer that spans that moment, which is
Premiere's cut-all-tracks.

**Two kinds of cut, because there are two kinds of layer.** A Sequence layer holds clips, so
a cut makes an **edit point** inside it. Everything else **splits into two layers**, which is
what AE's `Ctrl+Shift+D` does — a new bridge op (`split_at`), one `Batch`, one undo step. The
halves keep everything (source, effects, masks, parent, label, keyframes) and — this is the
part that matters — the **same `start_offset`**, so each half shows exactly the frames it
showed before and every keyframe stays on the same comp frame (K-213). A cut at either end is
refused rather than making a layer of no length.

**One razor, two doors.** The Timeline's own "Arm razor" menu item now arms the *toolbar's*
tool rather than a flag of its own. Two pieces of state that both mean "the razor is armed"
is one too many: they would disagree the first time someone used the other door.

**Both pointers are drawn, for K-219's reason.** No platform ships a pan-behind cursor or a
razor. The anchor's pointer is the anchor's own ring-and-cross with a small arrow at its tail
(AE's pairing, and it says what the tool moves); the razor's is a blade with a full-height
line down the lanes marking where the cut will land — the line is the useful half, because a
cut you cannot aim is a cut you undo.

**K-221 · DECIDED · A cut through a retimed layer leaves a keyframe behind, and the gizmo's
centre handle pans behind.** From the owner (2026-07-31), two refinements to the tools that
landed with K-220.

**Why a cut needs a key.** Splitting a layer gives both halves the whole document — the same
source, the same effects, and the same Retime map. That is what makes the cut invisible, and
it is also what makes the two halves' speed ramps *one curve*: bend the first half's speed
afterwards and the second half bends with it, which is not what anyone means by cutting. So
the razor puts a keyframe at the cut, on both halves, giving each an end of its own to hold.
Premiere does the same to a speed ramp it cuts, for the same reason.

**And why the key must not change anything.** A cut that altered the speed ramp it was
cutting would be worse than no cut at all, so the insertion preserves the curve exactly. A
span is one cubic bezier (docs/impl/keyframe-eval.md §1); de Casteljau splits a cubic into
two cubics whose union *is* the original — not an approximation of it — so all that is left
is converting the control points back into the AE speed/influence pair each keyframe side
stores, which is the exact inverse of `CubicSpan::from_ae`. The test samples the span two
hundred times before and after and demands agreement to 1e-6. A held span is inserted flat,
because a hold has no shape to keep; a key outside the keyed range takes the held end value.

`Property::insert_key_preserving_shape` lives in `lumit-core::anim`, not in the bridge: it is
curve arithmetic, and the kind of rule that has to be provable rather than observed. The
razor calls it before cloning the layer, which is what puts the key on both halves.

**The centre handle.** The Selection tool's gizmo now has a ninth handle at the layer's
anchor, and dragging it pans behind exactly as the Anchor point tool does (K-220) — same
`Shift` axis lock, same `Ctrl`/`Cmd` snapping, same one-op commit. Its grab radius is 16px
against the other handles' 32, and that asymmetry is the whole design: the anchor usually
sits in the middle of the box, which is also the easiest place to grab a layer to move it, so
a generous slop there would turn every body drag into a pan-behind — the pivot sliding while
the layer stayed put, which reads as the drag being broken. The pivot has to be aimed at.

**K-222 · DECIDED · The shape tools draw masks, and a mask crosses the bridge as its path.**
From the owner (2026-07-31), choosing to build masks now and shape layers on a branch of
their own.

**The seam that did not exist.** `lumit-core` has had masks all along — bezier paths,
rectangle/ellipse/star constructors, and a renderer that applies them — and *no bridge API
exposed any of it*. The Flutter frontend could not see a mask, let alone make one. So the
first half of this is the seam: `get_masks`, `add_mask`, `set_mask`, `delete_mask`, and the
masks carried in the read model (K-184) so the Timeline's twirl-down draws a row per mask
without asking per frame.

**A mask crosses as its path, in layer space.** `BridgeVertex` is the engine's vertex — a
position and two tangent handles, each an offset from it — carried across unchanged, so a
path never changes meaning by crossing. Layer space, not comp space, because that is what
makes a mask travel with its layer's transform for free; the tool takes the pointer's screen
position and runs it backwards through the layer's map, the same inverse the wireframe
hit-tests with.

**Every edit is the whole mask.** The engine's op is `SetLayerMasks` — the whole list,
exactly invertible — so an add, a delete, a rename and an invert are all one shape of edit
and each is one undo step. The bridge refuses a path of fewer than two vertices: that is not
a shape, and a mask that gates nothing would be a Timeline row with nothing behind it.

**Two gestures, because there are two kinds of shape.** Rectangle, rounded rectangle, ellipse
and star are *boxes*: drag two opposite corners, `Shift` for square, and the drag reads the
same in all four directions because the box is normalised before the path is built. The
polygon is a *path*: click for a corner, click-drag for a vertex whose bezier handles mirror
each other as they grow, `Alt` to break that mirror, and a click on the first vertex closes
it — closing being what applies it. Escape abandons, Backspace takes back a point. (After
Effects calls this its Pen tool and gives its polygon tool a regular n-gon; the owner asked
for it on the polygon, and one path-building tool is better than two.)

**Masks sit above Effects in the fold-out**, because that is the order they are applied in: a
mask gates the layer's alpha *before* its effects run (docs/06 render order), so the
twirl-down reads top to bottom the way the picture is built.

**Nothing selected says so.** After Effects would make a shape layer; Lumit's `LayerKind` has
no Shape variant, so there is nothing honest to make. The tool posts a notice naming what to
do instead. Silence would read as a broken tool, and a solid-with-a-mask dressed as a shape
layer would be a lie in the layer list — one that would have to be untold when the real kind
lands.

**K-223 · DECIDED · The path-building gesture is the Pen's, and the polygon tool draws a
polygon. Supersedes K-222's placement of it.** From the owner (2026-07-31): "I think I
might've misunderstood the polygon tool — everything I said there applies to the pen tool."
They are right, and K-222 built it in the wrong place.

**What moved.** Click for a corner, click-drag for a vertex whose bezier handles mirror as
they grow, `Alt` to break that mirror, click the first vertex to close and apply, `Escape` to
abandon, `Backspace` to take a point back — all of that is now the **Pen** (`G`), which is
where After Effects puts it and where anyone arriving from AE will look for it.

**What the polygon is instead.** A shape you drag out like the others: the regular five-sided
figure inscribed in the drag's box, first point at the top, `Shift` for a regular pentagon in
a square box. It is the star without its notches, and the two now read as the pair they are.

**Why this was worth correcting rather than living with.** A tool that does something other
than its name is a tool that has to be explained every time; and the Pen sitting there doing
nothing while the polygon did the Pen's job would have been two wrong tools rather than one.
The code moved with the name: `PolygonDraft` is `PathDraft`, and it is documented as the
Pen's.

**And the five shape tools are marked built.** K-222 shipped them working while their
`ToolMode.ready` flags still said otherwise, so every tooltip claimed "not built yet" over a
tool that drew masks. `ready` is a promise about what a tooltip says (K-216) and it was
lying; a test now pins the set.

**K-224 · DECIDED · A mask's points are things you can aim at, sweep up and drag.** From the
owner (2026-07-31): "if you have the selection tool and wireframes enabled, if you have a
layer that's a shape or has a mask, you should be able to see the individual points that make
it up. And if you do the drag selection I mentioned it should select any point inside it so
you can alter and drag them about."

**What a press means, in one order.** Over the picture with the Selection tool the pointer
has more and more things under it, so the order they are tried in is the whole design: a
**scale or rotation handle** first (it sits on the box's edge, where the body also is), then a
**mask point** of a *selected* layer, then the **layer** under the pointer, then empty space.
Points come before the body because they are drawn on top of it and are much smaller; they
come after the handles because a handle is the coarser target and losing it would be worse.
Only *selected* layers' points are tried: a stray vertex of some layer underneath must never
steal a press meant for the picture.

**The marquee gathers points when there are points to gather.** A sweep from empty space that
catches any of the selected layers' vertices selects **those**, and the layer selection is
left alone; a sweep that catches none is the layer sweep it has always been. That is one
gesture doing two jobs, decided by what is actually under it — which is what After Effects
does, and what makes "select some points and move them" a single fluid thing rather than a
mode.

**Which forced the selection to be decided on release, not on press.** The old marquee cleared
the selection the moment it began. That is invisible when it is layers being swept, and fatal
when it is points: the press would drop the very layer whose points the sweep was about to
gather. So the band now leaves the selection alone while it is drawn and settles it on
release — which also keeps the boxes on screen while the user is aiming, and is the better
behaviour on its own terms.

**A drag moves each point in its own layer's space.** The pointer's travel is a *screen*
delta; each mask is written in its layer's coordinates. The delta is therefore mapped through
each layer's own inverse (two points on the picture, subtracted, so scale and rotation are
undone exactly) before it is added — so a selection spanning two layers with different
transforms still moves together on screen. One `set_mask` per mask, which is one undo step per
mask, the same rule the razor follows for a multi-layer cut.

**No live preview while points are dragged.** The dragged points follow the pointer as drawn
marks, and the picture catches up on release. The preview path (K-183) patches one layer's
*transform* into a clone of the document; a mask path has no room in it, and inventing one for
a gesture this short is not worth a second preview shape. The marks moving is enough feedback
to aim with.

**Handles are not points.** A vertex's two bezier handles cannot be dragged yet, and neither
can a mask path be keyframed. Both are in TODO.md. This decision is about the *positions* of
the points, which is the half that makes a drawn mask correctable.

**K-225 · DECIDED · The Type tool makes and edits text layers on the picture, and the
toolbar grows the options the drawing tools use.** From the owner (2026-07-31): "Now add
typing. These should also be their own layer type… along with this there should be the
options for fills/border colour pickers and pixel widths etc just like AE."

**Text is already its own layer kind**, so this is the interface catching up with the
engine: `LayerKind::Text` holds a document (one styled run, docs/03 §9.1), the renderer
rasterises it, and the only way to make one was a menu item that dropped "Text" in the middle
of the composition. The tool puts it where the user points.

**One click, two meanings.** On empty picture the tool makes a text layer *where the pointer
is* and starts typing into it; on an existing text layer it edits that one. Clicking
somewhere else ends the edit and begins the next, which is After Effects' behaviour and the
only one that lets a caption be typed without a trip to the Timeline.

**A stray click leaves nothing behind.** A layer this tool made that ends its edit with no
text is deleted. An empty line renders nothing, so what a stray click would otherwise leave
is an invisible row in the Timeline — the same reasoning as the bridge refusing a mask of one
vertex.

**The document is written once, and the picture keeps up through a preview.** Every document
edit is an undo step, so a `set_text` per keystroke would make undo walk back through a
sentence one letter at a time. So typing sends `render_frame_with_text_preview` — a third
member of K-183's preview family, beside the effects and transform ones, patching a document
into a *clone* the same way — and the layer is written when the edit ends. One typing
session, one undo step.

**The caret is drawn; the text is the engine's.** The keyboard is a real Flutter text field,
so arrows, selection, backspace, paste and IME all work — but it is invisible, because the
text the user should see is the engine's own rendering. What is drawn is the caret, placed by
the same rough estimate of a line's width the bridge uses to anchor a new text layer (half
the point size per character). Being wrong the same way on both sides is what keeps the caret
and the picture agreeing about where a line ends; the true advance widths live in the
rasteriser and are not on the bridge. When they are, both sums change together.

**A new layer's anchor is recentred when the edit ends, pan-behind.** It starts on the left
end of an empty line, because an empty line has no middle; once there is a line, the anchor
moves to its middle and Position compensates by exactly the amount that keeps the words where
they were (K-220's sum). So a typed layer scales and turns about itself without ever having
appeared to move.

**Vertical type is not built.** `lumit-text` lays out one horizontal line; the member stays
on the strip, marked unbuilt like every other, and says so if it is clicked.

**The toolbar grows a tool options area**, where After Effects has one: the fill swatch and
the point size while a type tool is armed, the fill and stroke swatches and the stroke width
while a drawing tool is. Fill and size are live — they set what the next text layer is made
with. **Stroke and stroke width are drawn disabled**, because nothing in the engine strokes
anything: a shape layer's outline and a paint stroke are both engine features that do not
exist. They are shown rather than hidden for the same reason unbuilt tools are shown (K-216):
the tool set is the specified one, and a control that is visibly not working yet says more
than a gap does.

**And a bug the Type tool found: arming a tool did not rebuild the Viewer's overlays.** The
panel listened to the tool only to change the *pointer*, handing the whole stage in as a
cached child — so every tool layer under it stayed armed for whichever tool was in hand when
the panel last rebuilt, and only happened to work because a tool is usually picked before
anything else redraws. The stage is now built inside the listener.

**K-226 · DECIDED · The tools that draw wear a drawn pointer: the eyedropper's crosshair
badged with the tool's own icon, a brush ring for the painting tools, and an I-beam for
type.** From the owner (2026-07-31): "for the shape and pen tools, they should use the same
cursor as the dropper has? And maybe have the icon for the shape or pen in use just slightly
offset to the bottom right of the cursor… For text I think it should use a text select type
cursor and rotate this depending on if the text is horizontal or vertical… make sure the
different brush options all have correct cursor icons for their function."

**Shape and Pen: crosshair plus badge.** The crosshair is the eyedropper's — the pointer that
means *this exact pixel* — because that is exactly what the first corner of a shape or the
first point of a path is. The tool's own icon sits down and to the right of it, out of the
way of the shape being dragged out, drawn twice: a halo copy a pixel across, then the ink one,
so it is legible on a white picture and a black one alike. This is After Effects' own badging
and it is what makes five shape tools that share a gesture tell each other apart.

**The painting tools get a ring, not a crosshair.** A brush is not a point, it is a *width*,
so its pointer is a circle the size of the stroke it would leave, with a dot at the centre for
where that stroke starts. The ring is drawn from the toolbar's stroke width through the
current magnification — a picture-pixel width shown at picture scale — clamped so a hairline
still has a visible pointer and a very wide brush does not fill the window. The badge under it
says which of brush, clone stamp and eraser is in hand. **Nothing is painted**: the layer
exists to wear the pointer and to say what is missing when clicked, since the engine has no
paint at all (docs/TODO.md).

**Type: the I-beam, turned when the type is.** Horizontal type takes the system's own I-beam
— every platform ships one and everybody already reads it as "you can type here". No platform
ships a *sideways* one, so vertical type has one drawn: the same beam a quarter turn round, so
the pointer says which way the line will run before a single letter is typed.

**And the click is where the words start.** A new text layer's anchor now begins at the left
end of its line's baseline rather than in the middle of an empty box, so what is typed runs to
the right of the pointer and sits on it instead of straddling it — the same relationship the
caret is drawn with. The anchor is still recentred on the finished line pan-behind (K-225), so
nothing appears to move.

**Why drawn rather than chosen.** A system cursor is a small fixed picture from the list the
platform ships, and none of these are on it. The three tools that already needed this —
Rotation, Anchor point and Razor — hide the system pointer over the picture and paint their
own; these do the same, through one shared pointer widget rather than a fourth private
painter.

**K-227 · DECIDED · Painting is a list of strokes on a layer, stamped into its pixels before
its masks gate them.** From the owner (2026-07-31): "please implement the brush stuff". K-226
gave the three painting tools their pointers and an honest notice; this gives them something
to do.

**What is stored is the gesture, never the pixels.** A stroke is the path the pointer took in
the layer's own coordinates, plus its colour, width, hardness, opacity and mode — and it is
re-stamped at whatever resolution the frame is being rendered at. So painting at a quarter
preview and exporting at full size gives a full-size stroke rather than a blurry quarter-size
one, and every setting stays changeable afterwards. Storing a painted bitmap would have been
less code and a dead end: it fixes the resolution, fixes the colour, and cannot be undone
stroke by stroke.

**A polyline, not a bezier.** Masks and shape layers are the bezier things; a stroke is a
record of a gesture nobody edits vertex by vertex. It is thinned on the way out — samples
closer than two screen pixels are dropped, first and last always kept — because a slow drag
raises hundreds of events a second and a thousand-point path costs the renderer for nothing
anyone can see.

**Three modes, one shape of thing.** Paint lays the colour down, Erase takes alpha away, and
Clone copies from elsewhere on the *same* layer by a fixed offset. Clone reads a copy of the
raster taken **before any stroke in the pass is stamped**, so it never picks up paint laid
down beside it — the alternative smears its own output across the picture. The clone stamp
refuses to stamp until `Alt`-click has set its source, and says so, rather than stamping
nothing.

**Where it lands in the picture.** Strokes are stamped into the layer's own raster, before
its masks gate it and before its effects run — so "mask off the part I painted" and "blur what
I painted" both mean the obvious thing. Two consequences: a flat solid is no longer rasterised
as an 8×8 tile once it has paint on it (a brush needs pixels to mark), and paint on a collapsed
Precomp layer forces the nested intermediate, exactly as a mask does.

**One drag is one stroke is one undo step.** `SetLayerPaint` replaces the whole list and is
exactly invertible, the same shape as `SetLayerMasks`: an add, a delete, a rename and a
recolour are one kind of edit. `Escape` abandons a stroke in flight; `Backspace` takes the last
one back.

**The brush gets its own three settings, and they are live.** Size, hardness and opacity sit
on the toolbar beside the fill swatch whenever a painting tool is armed. They are *not* the
shape tools' stroke pair — a brush is a different thing that happens to have a width, and
K-225's stroke controls stay disabled because nothing still strokes a path. The brush colour
**is** the fill: a fill is what a tool lays down.

**Strokes list in the Timeline** under a Paint heading between Masks and Effects — the order
the picture is built in — with each row named for the tool that made it, carrying its opacity
and a menu that deletes it. The heading appears only once the layer has a stroke, exactly as
Masks and Effects do.

**CPU, and deliberately so for now.** The stamping is a scanline loop beside the mask
rasteriser, which is where the layer's pixels already are. A GPU path (a stroke as a
tessellated ribbon, or a compute stamp) is the right long-term answer for a stroke being
painted live at 4K, and it changes nothing about what is stored — which is the point of
deciding the storage first. Paint on a Precomp layer's *nested* pixels is not built for the
same reason: those pixels never come back to the CPU.

**Not built, and named so nobody assumes otherwise:** pressure and tilt, brush shapes other
than round, spacing and scatter, write-on (a stroke's own start and end times), painting in
Layer view rather than on the composite, and per-stroke blending modes.
**K-228 · DECIDED · A tool that is not built cannot be armed. It stays on the strip, drawn
disabled.** From the owner (2026-07-31): "I think we'd be better off just disabling that in the
toolbar for now… it'd be better to see what we still need to add rather than removing it and
forgetting about the code."

**Which is a correction to K-216's honesty rule, not a reversal of it.** K-216 put every
specified tool on the strip and had the unbuilt ones say so in a tooltip. That was right about
*showing* them and wrong about *arming* them: a tool you can pick that then does nothing reads
as a broken application, and the tooltip is only read by someone who already suspected. Shown,
disabled and labelled is the honest version of the same idea — the strip still teaches the
shape of the application, and nothing in it lies.

**Disabled means disabled everywhere.** `ToolsState.select` refuses an unbuilt tool, so the
button, the flyout row and the keyboard chord all decline together; a group's chord cycles only
its built members; and a group with nothing built in it takes no click at all. The state is the
gate rather than the widget, because there are three ways in and only one of them is a button.

**The flag is `ToolMode.ready`, which already existed.** No second list to keep in step: a tool
becomes armable the moment its behaviour lands, in the same commit, and the test that pins the
built set (K-223) now pins what is *arming-able* too. This is also what keeps the two branches
straight — paint is built on the engine branch and not on this one, and each branch's toolbar
follows its own flags without either being edited to suit the other.

**What is disabled today:** the Roto tools and the Puppet pins (both engine features of their
own size, at the owner's direction), the Pen's four editing siblings, and vertical type.

**K-229 · DECIDED · The camera tools move the composition's active camera by dragging on the
picture.** From the owner (2026-07-31): "Camera, make this like after effects implementation
along with any gizmo's etc and custom cursors if necessary."

**What Lumit's camera is, which decides what the tools can be.** A camera layer holds a
position, three rotations and a *zoom* — the focal distance in composition pixels. The plane at
the camera's own position renders 1:1 and centred, so **the position is the point the camera is
looking at**, and the eye sits `zoom` behind it along the camera's forward axis. Everything
follows:

* **Orbit** changes the rotations and leaves the position alone. Because the eye is derived
  from both, it swings round the point being looked at — a true orbit, with no separate pivot
  to store.
* **Track** slides the position along the camera's own right and up axes, so the eye travels
  with it and the picture slides under the pointer, the same sense the Hand tool has.
* **Dolly** slides the position along the forward axis, moving the eye and what it looks at
  together, in or out of the scene, by a fraction of the distance already in hand — so a dolly
  across a wide shot covers ground and one in a close-up creeps.

**The axes are built the compositor's way** (`Ry · Rx · Rz`, lumit-gpu's `camera_matrix`). This
is the one piece of frontend arithmetic that has to agree with the renderer exactly: a tool
that moved the camera along a different set of axes would send it sideways when asked for
forward. It is unit-tested against hand-computed cases rather than by dragging.

**Dragging up lifts the camera over the top** — which means tilting it to look *down*, a
negative x rotation in a frame where +y is down the screen. Getting that backwards is the
classic inverted orbit, so it has its own test. The pitch is clamped just short of the poles
rather than wrapped: past straight down the next pixel flips the picture over.

**The gizmo is the pivot.** While a camera tool is armed the point the camera is looking at is
marked, and while orbiting the circle it swings round is drawn faintly. That point projects to
the middle of the frame by construction, which is worth saying plainly: what the camera looks
at is the middle of the picture.

**They act on the active camera, not the selection.** The topmost visible camera layer whose
span covers the playhead — the one the renderer looks through — because the camera is what you
are looking *through* rather than a thing you have picked. With no camera at all the tool says
so. A camera whose placement is keyframed is left alone, the same rule the layer gizmo follows:
there is no single value for a drag to add to.

**No point of interest, and no unified camera tool.** After Effects' two-node camera has a
separate point of interest, and its Unified Camera tool switches between the three by mouse
button. Both are in TODO.md; neither changes the three tools above.

**K-230 · DECIDED · What the toolbar's tools were getting wrong, in one pass.** From the owner
(2026-07-31), on using the tools for the first time. Every item here is a correction to K-216 →
K-229 rather than a new capability, so they are recorded together.

**One gesture is one undo step.** Dragging a layer on the picture wrote Position x and Position
y as two ops, so `Ctrl+Z` put the layer back along one axis and left it half moved; scaling did
the same. Both write through `set_transforms` now — the batch op the Anchor point tool already
used — so a drag is one step. The Type tool was worse: making a layer was three ops (a layer
saying "Text" in the middle of the composition, an empty line written into it, a move to the
click) and finishing the edit was two more, so undo walked back through states nobody had ever
seen. Making a text layer is now one op (`add_text_layer_at`) and finishing a typing session is
one more (`set_text_placed`), so the first undo takes back the words and the very next removes
the layer. **The rule, stated once: an op is what the user would call an action, and a gesture
that writes several properties writes them in one `Op::Batch`.**

**A drag takes what is selected, whatever is on top of it.** A press inside an already-selected
layer moves *that* layer, even where a higher one overlaps the same spot; only a plain click
still takes the topmost, because that is how a layer underneath gets chosen with the mouse at
all. Without the rule, a layer chosen in the Timeline could not be dragged wherever anything
covered it — the press silently swapped the selection and moved the wrong thing.

**Windows ships neither a grab nor a magnifier, so the Hand and the Zoom draw their own.**
Flutter accepts `grab`, `grabbing`, `zoomIn` and `zoomOut`; the Windows embedder's table has
none of them and quietly answers with the ordinary arrow, which is why arming those two tools
looked like arming nothing. They join the Rotation, Anchor point and Razor tools in hiding the
system pointer and painting their own (K-219, K-226): an open hand that closes while it pans,
and a magnifier whose sign follows the Alt key. The Razor gives up its crosshair over the
*picture* — it cuts in the Timeline, and a precise pointer promised a gesture the Viewer does
not have — and its Timeline blade gains a marked hot spot, because the point where a leaning
blade actually bites is otherwise an unmarked corner of a drawing.

**The Rotation pointer settles on eight positions** — the four edges and four corners of the
layer's own box — rather than leaning by a continuously varying angle. The continuum was true
to the geometry and worse to read: a mark that is never twice the same shape is one the eye
re-reads every time. Eight shapes are eight things to recognise.

**A preview in flight is drawn in flight.** The wireframe is built from the document, so while
a turn was being dragged the picture rotated under a box that sat still until the button came
up. The angle in flight is published on the interface state and the layer that draws the boxes
reads it. The same rule covers the drawing tools' own pointers: a `MouseRegion` stops reporting
a hovering pointer the moment a button goes down, so every drawn pointer follows the *drag* as
well, and the Pen previews its next edge as the curve the placed vertex's handles make it,
not as a straight line that changes shape once the point lands.

**Zooming in must not cost the window.** The transparency board behind the picture was a widget
the size of the *picture*: at 800 % on an HD composition that is 15360 pixels across, and an
8-pixel grid over it is half a million rectangles a paint for the few thousand on screen. It is
bounded by the panel now, clipped to the picture and pinned to the picture's own grid, so it
costs the same at every magnification.

**Magnification is not resolution.** The scale reported to the engine follows the *panel* — a
Viewer docked small is cheap, which is the point of reporting anything — and not the zoom
inside it. Zooming out used to lower the preview resolution, which threw away every cached
frame and made the picture coarser for a gesture that only meant "let me see more of it".
Zooming in cannot raise it either: above composition resolution there is nothing to render.

**Panning, and hovering with a camera tool, must ask the engine nothing.** Both rebuilt their
panel on every movement of the pointer, and both re-read the document to do it — the Viewer
asked for the composition's settings, its size and every layer's source item; the camera layer
re-found the active camera, which reads a focal distance and a frame rate across the bridge.
Both answers are held until an edit lands. Budgets in `bridge_call_budget_test.dart`.

**The camera tools hold the pointer still while they drag.** Moving a camera is a gesture with
no place — nothing on the picture is being aimed at — so a pointer that wanders out of the
Viewer and finally into the corner of the screen is a drag that ends before the user does. The
pointer is pinned where it was pressed and only its movement is read (`freeze_cursor` /
`restore_frozen_cursor`, Windows-only; elsewhere the drag reads movement between events exactly
as before). Putting the pointer back is itself a movement, so the drag measures against the
anchor rather than against the last event, and the put-back reads as no movement at all.

**A text layer is as big as its line.** Text had no measured bounds on the frontend and fell
back to the composition's size, so a click with the Type tool drew a box the size of the frame
round twelve-pixel text. It measures the point size tall and the engine's own width estimate
wide; an empty line keeps one character's worth so a layer waiting to be typed into is still
visible and still says what size it will be set at.

**Escape ends a typing session, and so does `Ctrl+Z`.** The text field swallowed the undo
chord, so undo appeared to have stopped working while typing. The edit is written first and the
chord handed on, so there is one undo path in the application rather than two.

**The toolbar is 30px tall, and keeps its 44px buttons across.** 15-DESIGN §7.2's hit extent is
kept along the row — which is what the strip is read by — and given up down the page: the strip
runs the full width of the window, so a 44px band of mostly empty chrome is height taken from
the panels underneath for nothing.

**The snapping switch is removed.** Nothing in the application read it (docs/07 §1.7 said so
outright). A toggle that governs nothing is worse than a missing one: it makes the reader doubt
what snapping *is* here rather than what it is set to. It returns with the snapping it governs.

**K-231 · DECIDED · The second pass over the tools, from using them (2026-07-31).** Follows
K-230 in the same shape: corrections found by the owner in a working build, recorded together.

**A layer switched off is not on the picture at all.** It gets no wireframe and takes no click,
and a click over it falls through to whatever is underneath. Switching a layer's eye off is how
you get it out of the way; a box round something invisible, and a click that selected it, put
it straight back in the way.

**A scale in flight is drawn in flight, and a scale may be negative.** The wireframe follows a
scale drag exactly as K-230 made it follow a turn — one shared "the box as the gesture in
flight would have it" in the gizmo, which the rotation knob, the scale handles and the Rotation
tool all pass through. And the layer↔screen map no longer floors the factor just above zero: a
handle dragged past the anchor turns the layer over, which is how every editor mirrors a layer.
Only *zero* is barred, because the inverse map divides by it; the sign is kept.

**Drawing reads the copy in hand; only editing checks.** The read model (K-184) asked the
engine whether the document had moved before answering — once per frame while a frame was being
built, and *every time* outside one, which is where every pointer handler runs. So a tool that
redraws as the mouse moves asked that question at the rate a mouse reports, and the answer was
always no: moving a mouse changes no document. The paint path now reads `heldLayers` /
`heldRevision`, which ask nothing. That is safe precisely because a change refreshes the model
and notifies, and everything that draws is listening — but it means **a panel that commits an
edit must refresh the model itself**, which the Timeline and Effect controls already did and
the Viewer now does too. Checking-as-you-draw was covering for that, invisibly.

**The pointer a tool draws follows the mouse whichever button is held.** Recorded under K-230's
pointer rules in docs/07 §2.3.3, and worth naming here for the shape of the bug: `MouseRegion`
reports *hover*, which stops the moment any button goes down — including buttons the tool does
not answer to at all. So right-clicking froze the drawn pointer where it was pressed until the
button came up. The position comes from pointer *move* events now, through one shared
`DrawnPointerRegion` rather than seven copies of the tracking.

**A drawn pointer is one frame behind, and that is inherent.** The system pointer is composited
by the operating system; ours is painted by the application, so it arrives with the frame. The
cost is kept to a repaint rather than a rebuild, and a tool that can wear a system pointer
still does — which is why only the tools with no platform cursor draw their own.

**K-233 · DECIDED · The third pass over the tools, from using them (2026-07-31).** Follows
K-230 and K-231. (K-232 is the cache bar's own entry.)

**The Anchor point tool puts the pivot where you point.** It was a *nudge*: the drag was
measured from the press and added to the anchor the layer already had, so grabbing anywhere and
pushing moved the pivot by that much. That makes placing a pivot a matter of aim-then-correct —
you can push it towards somewhere, never put it anywhere. A **click** now places it, and a drag
keeps it under the pointer the whole way. Shift still locks to one screen axis, measured from
the press; Ctrl (Cmd) still snaps to the layer's own key points. Shift+click stays a *selection*
gesture and moves nothing: a click that both changed the selection and moved that layer's pivot
would be two edits nobody asked for at once.

**The Pen tells you when a click would close the path.** The closing tolerance is a fixed number
of screen pixels and nothing said how near "near enough" was — you clicked, and either the path
closed or it grew a point you did not want. The first vertex grows a ring and the pointer wears
a smaller one, so the question's two halves are both answered: *which* point closes it, and
whether the click about to be made is that one.

**The Pen previews the edge it is actually aiming at.** While the next vertex's handles are
being pulled out, that vertex is already placed — it is where the press landed — so the
preview runs to *there* and bends into it by the handle facing back along the path, which is
the mirror of the one under the pointer (or the vertex itself, when Alt has broken the pair).

**`Ctrl+Z` takes back one point while a path is being built.** The one place in the application
where undo means something narrower than "undo the last edit", and it has to be: the points are
not in the document — the path is applied in one op when it closes — so an undo pressed
mid-path sailed straight past every point placed and undid whatever the user had done *before*
picking up the Pen. It goes back to the document's own undo the moment the path is empty.

**A text box grows with the words.** The document holds the old line until the edit ends
(K-230's one-op rule), so a box measured from the document did not grow as the words did. What
is being typed is published for the boxes to measure, the same way a turn in flight is (K-230).

**The camera tools' chatter, finally.** K-230 cached the active camera and K-231 gave the paint
path an *unchecked* read of the model, but the camera layer's own cache key was still the
checking one — so it asked the engine for the document's revision on every frame of every mouse
movement, which is what it had been reported doing twice. It reads the held revision now.

**And the test that should have caught that could not see it.** `bridge_call_budget_test.dart`
pumped frames with `tester.pump()`, which does not advance the clock — so every frame carried
the same timestamp, the read model's own "once per frame" grouping saw *one* frame for a whole
gesture, and the budget measured zero while the running application was making a call per
frame. The budgets pump with time on the clock now. **A performance test that cannot reproduce
the conditions it is guarding is worse than no test: it certifies the bug.**

**K-234 · DECIDED · Mask rows in the Timeline: selectable, undoable, deletable (2026-07-31).**
Three faults reported by the owner against the mask rows K-222 added, fixed together because
they are one story: you pick a mask, you change it, you take the change back, you delete it.

**A mask's opacity is one undo step for the whole drag.** The row's opacity field wrote
straight through on every drag tick, so a drag across twenty ticks left twenty
`SetLayerMasks` ops on the undo stack and one `Ctrl+Z` took back one percent — which reads as
undo doing nothing. The op was invertible all along; the fault was the *rate* of committing.
The field stages its value in the row and commits once on release, exactly as the Volume,
transform and effect-parameter rows already do. **Any control that writes while a gesture is
running must stage: an undo step is a gesture, not a tick.**

**A mask row is a property row.** It carries a fold path already, so it joins
`_selectedProperties` through the same click that selects Position — plain, Ctrl and Shift all
behave as they do everywhere else, the row lights up, its Masks heading marks itself, and
closing the fold drops the selection inside it (K-203). No parallel mask-selection state:
a second selection model is a second thing to keep honest.

**A panel with a finer selection is asked before Delete deletes a layer.** Every keyboard
shortcut in the shell is a `HardwareKeyboard` handler, and Flutter runs *all* of them on every
key — so a panel cannot claim a chord merely by handling it and returning true. With a mask row
selected, the Timeline's Delete and the shell's Delete would both have fired, and the layer the
mask sits on would have gone with it. The shell asks `LumitUiState.deleteClaim` first and stands
down when the answer is yes; the Timeline sets that claim while it is mounted and answers with
"I deleted the selected masks" or "not mine". Deleting a mask is the same call the row's
right-click menu makes, so there is one path a mask is deleted by.

**K-235 · DECIDED · Two pointers that were pointing at the wrong thing (2026-07-31).**

**The Anchor point tool's pointer is a reticle.** It carried a small arrow off its tail, down
and to the right, so the mark would read as a pointer rather than as an overlay that happened
to sit under the mouse. That was a lie about the one thing a pointer must be honest about: the
arrow's tip is not where the pivot lands — the middle of the ring is. A ring with gapped
crosshair arms says "this exact point" and has no tip to mislead with. The gap is not
decoration: it leaves the point itself visible instead of covering it with the mark that is
supposed to be aiming at it.

**The Razor's pointer is the application's own scissors, and the line does the aiming.** The
tool drew a bespoke blade leaning up and away from the point it cuts at, which needed a second
mark (K-230's hot spot) to say where the edge actually bit. Once the cut line says *where*, the
pointer only has to say *which tool is in hand* — and the icon already on the toolbar says that
better than a hand-drawn one, at no cost in code. The blade and its hot spot are deleted; the
full-height line at the pointer's frame stays, because that is the mark that answers the
question the razor asks.

**The general rule these two share.** A drawn pointer must have exactly one point it claims,
and everything else it draws must be recognisable rather than aiming. Where a mark cannot be
both, the aiming belongs to a separate mark that can be put exactly where the action lands.

**Alt brings the system pointer back, and it has to be sent away again.** Alt is the key
Windows reserves for the window menu, and pressing it takes the pointer's own state with it —
so the arrow reappeared beside the Zoom tool's drawn magnifier, which is two pointers, exactly
what hiding the system one is for. Flutter will not re-apply a cursor by itself: it only does
so when the answer *changes*, and hidden-to-hidden is no change. The request is made directly
for the device the pointer events arrive from. **Not** by giving the region a new identity to
force the question — that was tried, and it rebuilt the gesture detector underneath and dropped
any drag in flight, which the Alt-box-zoom test caught.

**A pivot dragged on the gizmo moves as it is dragged.** The last of the in-flight previews
(K-230's turn, K-231's scale): the anchor handle wrote on release only, so the mark sat still
while the picture behind it was already being previewed. The box deliberately does not move —
that is what panning behind means — but the pivot on it does.


**K-236 · DECIDED · A cut keys only what has been retimed, and the Zoom tool opens on the plus
(2026-07-31).** From the owner, on the last pass before merging.

**The razor's keyframe belongs to a retimed layer, not to a layer with a Retime property.**
K-221 gave both halves of a cut a keyframe at the cut, so two speed ramps that were one curve
each get an end of their own to hold. Right — for a layer somebody has shaped. But switching
Retime on installs the *identity* map, so "has a Retime property" and "has been retimed" are
different questions, and the first one was being asked. Cutting an untouched layer left
keyframes on both halves that the user then had to notice and remove, for a cut they had asked
nothing else of. The rule now tests the map itself: keys whose values read back their own
times, joined linearly, are the map nobody has touched. A frozen frame is a retime however it
is written, and an eased pair that happens to start and end where an identity would is a ramp,
not an identity — both have their own tests.

**The Zoom tool believes what it has seen, not what the platform remembers.** Arming it could
open on the minus, so a plain click zoomed *out*. Windows eats the Alt key-up when Alt reaches
for the window menu or Alt+Tab leaves the application, so the platform's own "is Alt down?" can
answer yes long after the key came up. The tool tracks Alt from the events it sees, starts
false every time it is armed, and the *click* reads that same flag rather than asking the
platform again — so what the pointer promises and what the click does cannot disagree, which
was the other half of the same fault.


**A workspace name you cannot read is a button you cannot use.** The strip lost 14px of height
(K-230) and the workspace names kept the 24px of vertical padding they had in a 44px band. In a
30px one that left the words three pixels tall: four pressable blanks on the right of the bar,
which is exactly the report — "hovering that area still shows a button that can be pressed".
The padding fits the strip now, and the test measures the *label*, not the button: a control
that is laid out and hit-tests correctly can still be unreadable, and only the text's own height
says so.
**K-237 · DECIDED · A shape layer is a list of paths with a fill and a stroke, and its size is
the box its art fills.** The other half of K-222's gesture: with nothing selected the shape
tools now make a layer instead of saying they cannot, which is what the owner asked for when
the shape tools were specified.

**The path type is the mask's, unchanged.** One `BezierPath` in the document, one rasteriser,
one vertex type across the bridge. A shape's path and a mask's path differ in what they *do*,
not in what they are — which is why the shape tools and the Pen could draw a shape layer's art
with the same geometry they had been drawing masks with since the day they landed. Nothing in
`viewer_shapes.dart` changed.

**Flat contents, not nested groups.** `LayerKind::Shape { contents: Vec<ShapeItem> }`, each
item a path plus a fill colour, a stroke colour and a stroke width. After Effects' nested
groups exist to carry its shape *modifiers* — repeater, trim paths, wiggle — and none of those
are built; a group hierarchy with nothing to put in it is a data model designed around a
feature nobody has written.

**A fill is the mask rasteriser; a stroke is the paint rasteriser.** The fill's coverage comes
from `mask::rasterise`, the same scanline walk with the same subsamples that decides which
pixels a mask gates. The outline is a `PaintStroke` run along the flattened path — the widened
path K-227 already had. Two rasterisers already in the engine, each doing what it was written
to do, instead of a third that could disagree with both.

**The layer's size is its art's bounding box, and it moves.** Every kind built so far has a
size fixed by its source: a clip's frame size, a solid's dimensions, a comp's. A shape layer's
changes as the art is edited, which was the trap named in the plan note — `LayerBoundsCache`
keys on the document revision so it follows, and both sides now bound the curve by its
**control points** (a cubic never leaves its own control hull) so the box on screen is the box
the picture was drawn into.

**A new shape layer lands where the art was drawn**: the anchor sits on the art's own top-left
corner and Position carries it there, so a rectangle dragged across the picture appears exactly
under the drag. It is added at the top of the stack and selected, so the next drag masks it —
which is the behaviour that makes "draw a shape, then draw another" work without a trip to the
Timeline.

**The stroke controls are live at last.** K-225 put a stroke swatch and a width on the toolbar
and drew them disabled, because nothing in the engine stroked anything. A shape layer's art
does, so they now paint: a width of zero draws no outline, which is how a fill-only shape is
made. The Type tool's options are unchanged.

**Contents list in the Timeline** under a Contents heading, *above* Masks and Effects — a shape
layer's art is its picture, the masks gate that picture, and the effects process it, which is
the order docs/06 builds a layer in. The heading appears only once there is art, exactly as
Masks and Paint do.

**One op, `SetShapeContents`, replacing the whole list** — the third of the same shape after
`SetLayerMasks` and `SetLayerPaint`. An add, a delete, a recolour and a path edit are one kind
of edit and each is one undo step.

**Not built, and named so nobody assumes otherwise:** nested groups and the shape modifiers
(repeater, trim paths, wiggle, offset paths), gradient fills, dashed strokes, line joins and
caps other than round, fill rules other than the mask rasteriser's, animated paths, and editing
a shape layer's points on the picture (K-224 edits *mask* points; the same gesture over shape
contents is the next piece).

**K-238 · DECIDED · What the shape and paint tools were getting wrong, in one pass.** From the
owner (2026-08-01), on using them for the first time. Every item is a correction to K-227 and
K-237 rather than a new capability, so they are recorded together.

**A stroke was invisible, and the cache was why.** A frame is named by a hash of everything
that went into it, and two frames with the same name are the same picture (K-214). The name
hashed a layer's masks and never its **paint** — so a brush drag changed no name, every cached
frame stayed valid, and the mark never appeared. Nothing could make it appear either, short of
moving something else in the composition: the report was "after letting go the line you drew
disappears and nothing makes it reappear", and that is exactly right. Paint is stamped into the
layer's own pixels, so it is content in precisely the way masks are. Hashed only when a layer
*has* paint, so an unpainted layer keeps the name it already had and no frame banked by an
earlier version is thrown away. A shape layer's `contents` were already hashed; that now has a
test of its own so it cannot quietly stop being.

**This is what made the clone stamp and the eraser look unbuilt.** Neither had a fault of its
own. All three painting tools wrote strokes the renderer drew and the cache then hid.

**A tool must not act on a layer that has gone.** Making a shape layer selects it, so the next
drag masks it — the point of the gesture. Undo removed the layer and left its id in the
selection, so the next drag still believed a layer was selected, tried to mask one that no
longer existed, and did nothing at all: the tool had stopped working with nothing on screen to
say why. The selection now drops layers the model no longer has, answered once from the model
rather than at each of the several places a layer can vanish. Undo is only the easiest way to
see it — deleting a layer reaches the same state.

**A tool draws what it is about to make, translucently.** The shape preview asked the *selected
layer* to place its points, so the half of the gesture that makes a shape layer — nothing
selected — previewed nothing at all: you dragged blind and the shape appeared on release. The
preview takes a coordinate space now rather than a layer, and there is always one: the
composition's when there is no layer. It is drawn **filled**, in the toolbar's own fill and
stroke at half opacity, so the swatches finally answer "what colour is this going to be?"
before the shape exists. Half opacity rather than full, because a solid preview is
indistinguishable from a shape that is already there. A drag on a *selected* layer still
previews in the accent: a mask has no colour of its own — it cuts — and promising a fill that
will never appear is worse than an outline. The Pen's closing ring had the same layer-shaped
hole and is answered the same way.

**One drag of a stroke's opacity is one undo step.** The row was written from the mask row as it
stood *before* K-234 fixed exactly this, so it committed on every tick and `Ctrl+Z` walked back
a single percent at a time — which reads as undo not working. Staged and committed once on
release, like every other dragged value in the Timeline.

**Hiding the system pointer is the shared region's job, not each tool's.** Alt is the key
Windows reserves for the window menu, and pressing it brings the arrow back beside a drawn
pointer (K-235). The Zoom tool had a fix for this; the painting tools did not, so the clone
stamp's `Alt`-click — the gesture that *sets its source* — put a second pointer inside the
brush ring. The fix moved into `DrawnPointerRegion`, so every tool that draws its own pointer
has it, and on **any** key rather than a list of the ones a platform might reserve.

**The workspace strip is held against the right-hand end** (docs/07 §1.4). It drifted in beside
the last tool when the options strip arrived: the tools took a *loose* Flexible, which claims
only the width it needs, so the free space was stranded past the workspace buttons rather than
in front of them.

**Still owed, and not part of this:** editing a shape's or a stroke's points on the picture the
way K-224 edits a mask's, and dragging a vertex's bezier **handles** — which no path in Lumit
can do yet, mask included. Both are named in TODO.md.

**K-239 · DECIDED · A value you drag has to show what it is doing.** K-238 made a stroke's
opacity one op per drag by staging it and committing on release. That fixed undo and broke the
other half: the picture did not move until the button came up, which the owner reported at once
and is the wrong bargain. Both halves are the requirement — **the tick previews and the release
commits** — which is the division the Type tool (K-225) and the transform rows already use.

**Through the same door, not a new one.** The render request already carries provisional
effects, a provisional transform and a provisional text document, all patched onto a *clone* of
the document so nothing is committed. Paint and a shape layer's contents join them. Anything
that can be dragged and cannot be committed per tick needs this, so the list will keep growing;
what matters is that there is one path and one place where a drag's provisional value meets the
renderer.

**The whole list rides along, not one item's opacity.** Paint and shape contents are stored and
committed as whole lists (`SetLayerPaint`, `SetShapeContents`). A preview shaped differently
from the op would be a second description of the same thing, and the two would drift.

**The shape row had the fault K-238 fixed on the stroke row.** It committed on every tick, so a
shape item's opacity was never one undo step. It is staged and previewed now, and the two rows
match — they were always meant to be the same row with a different noun.

**A cancelled drag asks for the picture back.** Releasing a drag that never ticked, or a gesture
the framework cancels, leaves the screen showing a value nobody committed. The row re-previews
the document's own value rather than waiting for the next thing to redraw it.

**Not done here, and named in TODO.md:** the *mask* row stages without previewing, so its
opacity drag has the fault this fixes, and a mask preview wants the same clone-and-patch path
these three now share.

**K-240 · DECIDED · The mask row previews too, and the pattern is closed.** The last of the
three whole-list rows to get what K-239 gave paint and shape art: the tick previews, the release
commits. K-234 had staged it, so the drag was already one undo step; what was missing was the
picture moving while the drag was in flight.

**Nothing new was designed for it.** A mask list joins paint and contents on the same
clone-and-patch render request, converted by the same `write` the commit uses. That is the point
of recording it: the three rows are one row with a different noun, and the third one arriving
without inventing anything is the evidence that the shape was right.

**What this closes.** Every whole-list property the Timeline can drag — a mask's opacity, a
stroke's, a shape item's — is now one op, one undo step, and live on the picture. A property
added to this family later has one path to join rather than a choice to make.

**K-241 · DECIDED · A nested comp's intermediate is transparent.** A Precomp layer's
intermediate is cleared to nothing before its inner layers draw, whatever background colour the
nested comp carries. Until now it was cleared to that colour, and since a new comp's background
is opaque black, every Precomp arrived as a solid black rectangle: content that was see-through
inside the nested comp painted black over the parent's stack.

**A background colour is a viewing backdrop, not a layer.** It belongs to the comp you are
looking at — the Viewer, and the export of that comp. A comp used *as a source* contributes its
layers and their alpha, nothing else. That is also what After Effects does, and it is why
collapse was never affected: a collapsed Precomp splices its inner draws straight into the
parent and never had a background to paint.

**Nothing else reads it differently.** The below-stack re-render (Posterize Time, accumulation
motion blur) still clears to the comp's own background, because that stack *is* the comp being
viewed, held at another time. The regression test is in `build_tests.rs`, on the same case that
already guarded collapse on and off.


---

**K-242 · DECIDED · A floating window can be moved, the Settings window can be resized, and
both are remembered.** The Settings window was 700×460 whatever the monitor, which is a size
chosen for the smallest laptop and read as cramped on anything larger — the Keymap table
scrolled a few rows at a time on a 1440p screen. And every window that floats over the shell
was pinned to the centre, so anything it covered had to be closed for rather than moved off.

**What it is.** One change in one place: `showLumitModal` (the funnel every floating window
already went through) now puts the window in a frame that can be dragged, optionally resized,
and optionally remembers where it was left.

* **Moving** is by dragging any part of the window no control has claimed. A slider, a
  scrolling list or a text selection wins the gesture over the window, so dragging a control
  still does what the control does. No separate title bar to grab: the windows draw their own
  titles in their own way, and a shared bar would have been a bigger change than the feature.
* **Resizing** is from a grip in the bottom-right corner, for windows given a size — for now
  the Settings window alone, which opens at 880×640 and clamps between 560×380 and the app
  window. The corner grip is a *sibling* of the window rather than a child of it: two nested
  drag detectors both join the gesture arena and neither one ends up moving anything.
* **Remembering** is per window id in the machine-local workspace store, beside the dock
  layout and the other working preferences — never in the project file. What is stored is an
  **offset from the centre**, not a corner position: a place saved on a large monitor then
  restored on a small one still lands on screen, and the window needs to know nothing about
  its own size to open centred the first time. The offset is clamped so the middle of a window
  can never leave the app window, whatever was stored and whatever the monitor.

**What it is not.** These are not native OS windows: they do not leave the app window, do not
appear in the taskbar, and cannot be dragged to a second monitor — that needs a second Flutter
engine per window and every bridge stream reachable from it, which is a different job. They
also stay **modal**: the dimmed backdrop and click-outside dismissal are unchanged. Moving one
is for seeing what is behind it, not for working while it is open.

The regression test is `flutter_ui/test/modal_window_test.dart`: dragging moves the window by
exactly what was dragged, the grip resizes from the corner with the opposite edge staying put,
the minimum and the app window bound the size, and a placement survives a store round trip.

---

**K-243 · DECIDED · A double-click opens; `Enter` renames.** Double-clicking a layer in the
Timeline opened an inline rename, and a second click on a footage row in the Project panel did
the same. That is the wrong verb on both: a double-click means *open this* in every editor,
and K-191 had already made it mean that for a composition — which left the application saying
two different things with one gesture depending on what was under the pointer.

**One rule, and the item answers it.** A double-click opens what it lands on:

* a **composition** fronts in the Timeline (K-191, unchanged);
* a **Precomp layer** fronts the comp it draws — the same answer the Project panel and the
  Hierarchy already gave for the comp itself;
* **footage** raises New composition on the selection, already the media's size, rate and
  length, with the selected items landing in the comp as layers. Footage has no window of its
  own, and the thing wanted from a clip just double-clicked is a comp to put it in. The
  dialogue was already able to do this — it is what dropping footage on the New composition
  button does — so this is a second door onto one funnel, not a second implementation;
* a **folder** shows or hides what is in it — opening a folder *is* seeing inside it. It
  keeps a caret so a shut one does not read as an empty one, every row reserves the caret's
  width so a child still lines up one step right of the folder holding it, and a search
  looks inside a shut folder because otherwise searching would depend on where the twirls
  were left. Which folders are shut is session state, like the search text, not the
  document's business;
* **every other layer kind** does nothing yet. It should open that layer in a Viewer of its
  own, and there is no such Viewer to open; a double-click that half-worked would be worse
  than one that waits.

**Renaming is `Enter`**, which docs/07 §15 has bound to `layer.rename` in the Timeline since
the keymap was written and nothing had ever handled. The row that the selected layer is on
opens its own editor, driven by a notifier the panel sets — a rename must not rebuild the
whole table (K-208's reasoning, K-231's budget). A locked layer still refuses, as it did when
a double-click was the way in. Renaming a comp or a footage item stays on the row menu, and a
comp also on its settings dialogue.

**Three things had to be true for `Enter` to work at all**, and each was a bug of its own:

1. **A dialogue's keys are its own.** The panels register their commands on the hardware
   keyboard rather than by holding focus, so nothing about an open window stopped the
   Timeline hearing a keypress meant for it — the Pre-compose dialogue's `Enter` renamed the
   layer behind the window. `showLumitModal` now says whether a modal is up
   (`lumitModalOpen`) and the Timeline's handler stands down while one is. The count is kept
   by the windows themselves as they mount and unmount, not by the open and close calls: a
   window can also leave by having its tree taken down under it, and a count only the close
   path decremented would stick above zero and leave the keyboard dead for the session.
2. **Pre-compose is the dialogue's default action.** It takes focus when the window opens and
   `Enter` presses it wherever that focus sits, drawn with the accent edge docs/15 §2 keeps
   for exactly this. The name field no longer takes focus on open, which is the cost: typing
   a name is now one click into the field.
3. **Clicking away finishes an edit and keeps it.** The Timeline's rename only committed on
   `Enter`; a click anywhere else left the field open and the typing was lost. It commits on
   the tap outside now, the way the Project panel's rename already did — the same
   `onTapOutside` hook, added to the shared `HouseTextField` so every inline editor gets it.
   The test harness gained the `TapRegionSurface` the application has from its `MaterialApp`,
   without which that hook never fires and no test could have seen this.

The regression tests are in `timeline_panel_frb_test.dart` (Enter renames the selected layer,
a locked layer refuses, clicking elsewhere commits, a double-clicked Precomp fronts its comp,
a double-clicked anything else does nothing), `project_panel_frb_test.dart` (a second click on
footage makes a comp of it, on a folder opens and shuts it, a search still looks inside a shut
one, the row menu renames, and none of it falls through to the empty-area import),
`precompose_dialog_frb_test.dart` (Enter presses Pre-compose) and `modal_window_test.dart`
(a window says it is open, and stops saying so however it leaves).

---
**K-244 · DECIDED · The menu bar is the whole application, listed — and on macOS it is the
system's.** The bar was three menus and a Window heading, which is what the port had time for.
It is now the nine After Effects menus (File, Edit, Composition, Layer, Effect, Animation, View,
Window, Help) with the commands each of them will carry, whether or not those commands exist
yet. A command that is specified and unbuilt is *listed*, marked "(Not implemented)" and drawn
disabled.

**Why list what does not work.** Two reasons, both about testing. The shape of the finished
application is visible while it is being built, so a tester knows whether a command is missing
or broken rather than guessing; and every unbuilt command has a place waiting for it, so
building one is filling a row in rather than deciding where it goes. The alternative — a short
menu that grows — hides the plan and re-opens the placement argument once per command.

**One tree, two renderers.** `lumitMenus` returns the bar as data: labels, keymap action ids,
enablement, ticks. Windows and Linux draw it in the window; macOS hands the same tree to
`PlatformMenuBar`, so the menus appear in the Mac menu bar like every other Mac application's,
with About and Settings lifted into the application menu as Apple's guidelines ask. Neither
renderer holds a list of its own, which is the only arrangement in which the two cannot drift
apart. iOS and Android are not targets (K-174 is a desktop decision), so there is no third case.

**Shortcuts come from the keymap, never from the menu.** A row names an action id and shows
whatever chord `lumit-keymap` currently binds to it, so a rebind in Settings ▸ Keymap changes
the menus too and a menu can never teach a chord that does nothing. Nine new actions joined the
shipped table for commands that already worked and had no chord — `file.new` (`Ctrl+Alt+N`),
`file.open`, `file.save.as`, `file.import`, `file.export` (`Ctrl+Alt+M`), `comp.new` (`Ctrl+N`),
`edit.select.all`, `edit.deselect.all`, and `app.settings` (`Ctrl+Alt+;`). After Effects' own
chords where it has one: `Ctrl+N` makes a composition there and `Ctrl+Alt+N` a project, which is
the pair anyone arriving from AE has in their fingers. Each is dispatched in the shell through
the very function its menu row calls, so a shortcut and its menu item cannot become two
implementations (the K-203 rule, applied to the rest of the bar).

**Settings is under Edit, and About is under Help.** Preferences-under-Edit is what every
Windows and Linux application those users know does; macOS moves both rows into the application
menu, which is what every application *those* users know does. About left Settings ▸ General
altogether: Settings is for what you change, and a version number is not that.

**Window lists every panel with a tick.** Toggling one adds or drops its pane in the dock, which
is also what makes it persist — what is stored is the arrangement, so a panel closed today is
closed after a restart with no new state to keep. The last panel standing cannot be hidden: an
empty dock has no way back.

**Composition ▸ Add to export queue, not "render queue"** (glossary §9 — *export*, never
*render*, for anything the user sees).


---

**K-245 · DECIDED · A project remembers the sitting it was left in, and carries it to whoever
opens it next.** Reopening a project landed on an empty Viewer: no comp fronted, no tabs, the
playhead at zero, whatever panel arrangement the application happened to be in. The frontend
already modelled the answer — `SavedSession` in `flutter_ui/lib/state/workspace.dart`, written
during the port — and nothing ever called it. This wires it, and extends it in the direction
the owner asked for: the arrangement is **per project**, and it **travels with the file**.

**What is remembered.** Which comps are on the Timeline tab strip, which of them is fronted,
what frame the playhead sits on, which layer is selected, and how the panels are arranged —
the arrangement itself (the tree, the tab groups, which tab each group fronts, the split
shares), never the name of a preset. Sizes and positions a user drags to *are* the
arrangement; a name could not carry them.

**Two homes, and which one answers.** The same record is written to two places:

* the **machine-local workspace store**, keyed by the project's file path, kept up to date as
  the user works (fronting a comp, closing a tab, changing the selection, dragging a panel,
  and on the window losing focus or closing — the playhead moves far too often to write per
  move);
* **inside the `.lum`**, in a new `ui_state` field on the document, written at save time only.

On open the **local record wins**, and the file's is the fallback. That ordering is what makes
both jobs work at once: your own projects open the way *you* last had them even between saves,
and a project arriving from somebody else — which has no local record — opens arranged the way
its author left it. A user with several projects open in turn therefore gets each one's own
layout as they switch, with no layout names to manage.

**Why the arrangement is in the document at all.** This supersedes the clause in
[10-FILE-FORMAT.md](10-FILE-FORMAT.md) §2 that forbade window layout in `project.json`. That
rule exists to keep a project file portable and free of one machine's private business — cache
paths, local usernames, absolute paths (K-173). A panel arrangement is none of those: it is
panel names, tab indices and fractional shares, which mean the same thing on any machine and
any monitor. It is written as an **absent-by-default** field, so a project nobody has arranged
gains no line for it and an older build reads the file unchanged, and it is **opaque to the
engine** — carried, stored and handed back, never read into — because its shape belongs to
whichever frontend wrote it. A reader that cannot make sense of it ignores it and opens the
way it already was; a layout naming a panel this build has never heard of is dropped whole
rather than half-applied. Window *placement* (K-242) stays machine-local: pixel offsets on one
person's monitor are exactly the private business the rule is about.

**Recording it is not an edit.** `DocumentStore::set_ui_state` is deliberately not an op: not
undoable (Ctrl-Z must never rearrange the window), not journalled (crash recovery replays work,
not furniture), and it does not move the revision — so resizing a panel does not make a project
read as having unsaved changes. The frontend writes it immediately before saving, which is the
moment the arrangement it describes is the one on screen.

Regression tests: `crates/lumit-core/src/store.rs` (recording is neither undoable nor a
change), `crates/lumit-project/src/lib.rs` (the arrangement survives a save, and an unarranged
project saves no field), and `flutter_ui/test/frb/session_restore_frb_test.dart` — the round
trip through a real save and open, a stale session naming a since-deleted comp falling back
quietly, and a shared project opening arranged on a machine that has never seen it.

The round-trip test deliberately reads the comp list **before** opening, because adopting a
project has to invalidate that cache (K-184) and the first cut of this did not: every panel
reads `comps()`, so in a running application the cache is always warm, the restore looked the
reopened project's comps up in the previous project's list, and a reopened project came back
with no tabs and nothing fronted. A test that opens with a cold cache is testing the one state
the application is never in.

---

**K-246 · DECIDED · Two Vegas preferences, and a first-run screen that sets them.** From the
owner (2026-08-02). Lumit gains two application-level settings, independent of each other and
each an ordinary visible row in the Settings window:

- **Retime opens to speed** — the graph editor opens a Retime channel in the **speed lens**
  (K-075's Vegas default-lens preference, carried into the property era), with the envelope
  semantics of K-247. Off, a Retime channel opens to the value lens and the speed lens keeps
  its ordinary two-sided shape; ordinary properties are untouched either way.
- **Video arrives as a Sequence layer** — adding **video footage or an image sequence**
  (never a still image) to a comp creates a one-clip Sequence layer over that source instead
  of a Footage layer. One funnel decides it (the `add_footage_layer` path), so drag-drop,
  double-click insertion and the menus cannot disagree. Single-source stands: each import
  wraps into its own Sequence layer; splicing different sources into one layer stays deferred
  (K-071's single-source clause survives K-248).

And the **first-run screen is un-gated** (supersedes K-006's post-v1 stance, which existed
because the screen had no settings to write — these are those settings): on the very first
launch, one calm screen asks how the user edits — **AE-style** or **Vegas-style** — where
Vegas ticks both settings and AE ticks neither. Skippable, and everything it writes remains
an ordinary setting afterwards (07-UI-SPEC §13.1's rules stand). v1 is deliberately plain;
the four-card version with per-choice imagery (the owner: "looking a little like Apple's
settings menus") is the destination and its polish is in TODO.

**K-247 · DECIDED · The Vegas speed lens is an envelope: one point per key, speed as the
value.** From the owner (2026-08-02), building on K-076 and K-197. With the Vegas preference
on, the Retime channel's speed view stops being a derivative plot with two one-sided dots per
key and becomes a **speed envelope**: each keyframe is **one draggable point** whose height
*is* the playback speed (per cent) at that moment — the owner: "think of the speed percent as
the value instead of it being a derivative of the actual frame values". Between points the
speed runs straight for now (eased shapes return with the preset-shelf rework, in TODO). The
store stays the one Retime property (K-197, K-249): an envelope point is a source-time
keyframe whose two side speeds are equal, and dragging a point **re-integrates the keyframe
values downstream** — source positions after the drag shift, while every keyframe *time* and
the layer's box stay put (K-022's covenant; K-070's start-pinning). Details:

- The **default vertical range is 100% down to −25%**, Vegas mode only, growing whenever the
  curve exceeds it. The visible negative floor is deliberate: it teaches that dragging below
  zero reverses the clip.
- **Reverse is ungated everywhere**, both modes, both lenses: a Retime curve may descend.
  (The old segment engine's `allow_reverse` gate does not carry over to the property path.)
- The AE-style speed graph — ordinary properties, and Retime with the preference off — is
  untouched.

**K-248 · DECIDED · The sequence view is the layer's own row grown tall, and clips reorder.**
From the owner (2026-08-02), superseding two clauses of K-071 and refining K-020.

- **Inline, not a tab.** Double-clicking a Sequence layer — its name in the outline (where a
  Precomp opens its comp) or its bar in the lanes — expands the row *in place*: the row grows
  tall, each clip draws a start and an end **thumbnail** (a thumbnail-at-source-time read;
  the existing bridge call only decodes a first frame), the razor and `Ctrl+Shift+D` cut the
  clip under the pointer/playhead in two, and beneath the clips sits a **small speed-envelope
  strip** (K-247 semantics, speed lens only) whose points belong to their clip and travel
  with it. Double-clicking again collapses. K-071's dedicated timeline tab is superseded:
  the Timeline's tabs hold comps, and the owner prefers cutting without leaving the lane; the
  full graph editor stays the home of the value view.
- **Clips reorder.** K-071's source-ordered invariant ("source time never jumps backwards")
  is dropped: clips on a Sequence layer may be reordered and repeated freely, the Vegas
  expectation. The camera-tracker rationale that motivated the ordering is re-answered rather
  than lost: a tracker will run on the **full, unaltered footage** and its result be mapped
  through the sequence's clip/retime resolution onto the layer — that mapping is the owner's
  design, expected with the tracker work.
- **The layer's span is its clips'.** A Sequence layer's bar always runs from its first
  clip's start to its last clip's end; interior gaps render transparent (unchanged); deleting
  or trimming an outermost clip moves the corresponding end of the bar. Retime edits still
  move nothing (K-022).

**K-249 · DECIDED · One retime system: the property.** From the owner (2026-08-02), resolving
[04-RETIMING.md](04-RETIMING.md)'s first open question — the K-197 duality — in the direction
K-213 already marked: **the property survives, and everything else converges on it.**

- The pre-K-197 **layer** speed map (`LayerKind::Footage::retime`, the fallback arm in
  `Layer::source_time_at`, and the Source card's speed/reverse/frames rows that edit it) is
  **deleted**. A document that carries one converts to property keyframes at load, through
  the store's own exact keyframe reader (`Retime::source_keyframes`), one way, with a notice.
- **Sequence clips migrate too**: `Clip::retime` moves from the segment store to the same
  `Property` shape, so the layer channel's Vegas lens and the sequence view's envelope strip
  are one editor over one representation, and cutting a clip splits keyframes rather than
  segments. The exact-split and integration maths of 04-RETIMING §§4–5 remain the reference
  for how the property is split and integrated; the segment store survives only as the
  load-time conversion for older documents.
- 04-RETIMING §0's framing — the property as the current surface, the segment engine as the
  destination — inverts: the property is both. Segment-era affordances (freeze, presets,
  overrun indication…) return by being rebuilt on the property, per K-197's own rule.

---

**K-250 · DECIDED · The speed envelope opens with headroom: 125% to −25%.** From the owner
(2026-08-02), refining K-247's figure after using it. K-247 set the Vegas envelope's default
range at 100% down to −25%; the top figure is now **125%**.

The room above normal playback is the whole point of the change. At exactly 100 the flat
line every un-retimed clip draws sat on the very top edge of the graph with nowhere to go
but down, which reads as a ceiling rather than as the ordinary speed it is — and speeding a
clip *up* is the commonest thing anyone does here. The floor stays where it was: −25% is
enough negative space to show that dragging below zero runs the clip backwards, without
giving up half the graph to a state most clips never reach. The range still only ever grows:
a curve reaching past either end reframes the axis, and while a drag is in flight the axis
is *frozen* and the drawing clipped to the graph's own bounds, so a point taken past an edge
never draws over the rows beside it and the framing catches up when the pointer is let go.

Both readings of the envelope take it — the layer's Retime channel in the graph editor, and
the sequence view's strip — because they are one editor over one representation (K-249).

---

**K-251 · DECIDED · The mark is the twin keyframes; the Kiriko facet mark retires.**
From the owner (2026-08-02). The brand mark becomes **two rounded keyframe diamonds
side by side — blue and violet-magenta — overlapping in an additive white core**: keyframes
for motion, the overlap for compositing, the white for luminance. This supersedes the mark
half of K-008 (the Edo-kiriko faceted hexagon, "approved placeholder" since 2026-07-13);
K-008's splash structure and the Persona-5 broken-glass splash-art ambition stand.

The shape was not taste alone: a corpus study of ~1,270 icons (Apple top charts US/GB/JP/DE
free+paid, plus every editing/creative search) found the winning grammar — at most two hue
families (78% of top apps), one large glyph, dark tile in the pro-editing tier — and the
category's burned imagery to avoid (play button, film strip, clapperboard, lens ring, colour
wheel; the four-point sparkle now reads as an AI badge). Letterform candidates were rejected
by the owner as boring; composited-frames and lit-gem candidates were culled as
gallery-generic and AI-generic respectively.

Deployment: the bare mark (no tile) is the Windows/Linux icon; the dark rounded tile is
macOS-only. File-type icons for `.lum` (twin keys, `LUM` kicker) and `.lumfx` (single key,
`LUMFX` kicker) wait in `assets/brand/` for an installer to register the associations.
Sources are the four SVGs in `assets/brand/`; `scripts/gen-icons.py` renders every raster
(each size rendered from the SVG directly, never downscaled). docs/15-DESIGN.md brand
section rewritten in the same commit.

**K-252 · PROPOSED · Per-platform installers: Inno Setup, a user-level install script, a DMG.**
Follows K-251 (2026-08-02): the document icons stop waiting. `packaging/` gains the three
platform stories, deliberately boring ones — **Windows**: an Inno Setup script
(`lumit.iss`, built by `build-installer.ps1`) that installs the release bundle and registers
`.lum`/`.lumfx` in the registry with their icons and an open command; chosen over MSIX for
zero signing prerequisites on a GPLv3 hobby-scale project. **Linux**: a POSIX `install.sh`
that places the bundle under `~/.local`, plus a desktop entry and shared-mime-info XML; the
brand SVGs install as scalable icons directly. Proper distro packages (deb/rpm/Flatpak) can
grow from these files later. **macOS**: `make-dmg.sh` (hdiutil) and the Info.plist document
types; unsigned, un-notarised, and the document `.icns` files join the bundle with the macOS
pass (K-033), which also owes `application:openFile:` handling.

The app now honours a `.lum` on its command line (`projectPathFromArgs`, regression-tested),
so the Windows association genuinely opens the document, not just the application. Still
open, in TODO: a CI release pipeline that builds all three, signing/notarisation, and
double-click opening on macOS.

**K-253 · PROPOSED · The Linux release also ships a Flatpak, repacked from the CI bundle.**
The "grow later" from K-252 (2026-08-03). Unlike the buried egui-era manifest (K-182),
which compiled the whole workspace inside the sandbox from a generated `cargo-sources.json`,
the new `packaging/flatpak/` manifest builds nothing: the release job already produces a
staged bundle with the FFmpeg 7.1 libraries inside it, so the Flatpak is that bundle plus
the desktop entry, mime XML and icon renamed to the app id (Flatpak only exports
app-id-named files to the host — which is also why the `.lum`/`.lumfx` document icons stay
a native-install feature). Runtime is `org.gnome.Platform//49`, not freedesktop: the
Flutter Linux embedder needs GTK 3, which only the GNOME runtime ships. Sandbox holes
(`--filesystem=host`, dri, pulseaudio) carry over from the old manifest's reasoning
verbatim. Published as a single-file `.flatpak` on the GitHub Release; Flathub submission,
if ever, is a separate decision.

**K-254 · PROPOSED · The playhead returns when playback stops, a scrub takes it off the
transport, and markers are flags you can drag, name and jump to by number.**
Three things the owner asked for on 2026-08-03, all about the playhead.

*Scrubbing during playback.* The engine chooses frames and hands each one back, and
`_arrived` moves the playhead to follow the picture (K-181). A drag on the ruler was
therefore unwinnable — every tick put the playhead back where the transport wanted it.
Taking hold of the playhead now **takes it off the transport**: every ruler and lane seek
goes through one new `LumitUiState.scrubTo`, which stops playback first. One funnel rather
than a guard per call site, because the three `onSeek` closures were three copies of
`playheadFrame.value = …` and a fourth would have been written without the guard.

*Returning on stop.* Stopping now puts the playhead back where `play` was asked for —
including when the composition simply runs out, since where you are when the transport
stops should not depend on why it stopped. This realises §9's long-standing
"stop-to-start toggle behaviour as a setting" line. The default is the returning one:
playback is a preview of the moment being worked on. Settings ▸ Interface ▸ Editing ▸
*Playhead stays where playback stopped* restores the older After Effects behaviour, and
unlike the K-246 pair it does **not** defer to a settings file's silence — an install
written before the field adopts the new default rather than being pinned to the old
behaviour by omission. The first-run screen does not touch it: both answers want the
returning playhead (real Vegas returns its cursor too), so there is nothing for the
question to decide, and K-246's pair stays a pair.

*Markers.* The engine has had markers since docs/03 §11 and the ⋯ menu has had a dialogue
for them; what was missing was the ruler. Comp markers are now small flags with the
**point at the top**, **centred on the frame** so the point sits on the playhead — the
point is what carries the meaning, *this* frame and not the one next door, so a shape hung
off to one side was marking the wrong thing. They hang into the ruler's lower row beside
the work-area band, drawn last so a flag wins the pointer over a work-area handle it sits
on. What a marker says rides in a box of the same colour flying from the flag's **centre
point** — the pole is the marker's centre line and the cloth hangs off it — rather than as
loose text over the ticks, where it crossed the ruler and the work-area wash and read as
neither. Both carry a hairline outline in the darkest surface and sit on the floor of the
ruler: a pale flag on the pale work-area band lost its silhouette, and the point was the
first part to go. Colour is a `marker` token of its own — a plain grey, light on dark and dark on
light, editable like any other role — because a marker says *here*, not *good* or
*careful*, and the accent is already spent on the work area.

**Markers do not stack.** Adding one where a marker already sits replaces it, and so does
dragging one on top of another; both go through one `markersWithFrb`, so the shortcut and
the drag cannot drift into different ideas of what placing a marker means. Two flags on one
frame are two things to click and one place, and the second hides the first exactly.

A drag **writes once, on release**. Committing per frame crossed — what the work-area edges
do — cost a document write, a cache flush and a panel rebuild for every frame of travel,
which is what made the drag feel heavy; the work-area edge can afford it because the
Viewer's preview range changes as it moves, and a marker has nothing to show until it
lands. In the same vein the marker list is now remembered in Dart (`markersOf`, beside the
frame/time memo, cleared by the same committed-change hook) rather than fetched across the
bridge on every ruler rebuild — sixty times a second while playback runs, for a list only
an edit can change. `bridge_call_budget_test` pins both: nothing per rebuild, exactly one
`set_markers` per drag.

**Right-click** offers *Edit marker…* and *Delete marker*.

The keyboard is the numbered pair every NLE has: **`Shift+0…9`** sets marker *N* at the
playhead and the **bare digit** returns to it — `Shift`+digit rather than `Ctrl`+digit,
which is the chord After Effects itself uses. Setting a numbered marker that already
exists *moves* it rather than adding a second — a digit has to name one place. A digit
with no marker behind it is left unhandled, not swallowed. The plain marker key is
**`Shift+M`** alongside AE's numpad `*`: Premiere and Vegas both use `M`, but `M` reveals
Masks in the Timeline and that reflex is older, so the letter stays put and the marker
takes Shift. Twenty new action ids, described by prefix (`marker.add.N`, `marker.goto.N`)
the way `workspace.switch.N` already is, and the shipped keymap stays conflict-free.

**Markers travel with the material (2026-08-03).** A layer carries its own marker list
(`Layer::markers`, docs/03 §11, drawn on its bar and moving with it), and two moments fill
it. Dropping a composition into another **copies** that comp's markers onto the layer: a
comp placed in a comp is a piece of material and its beats are part of what you are
placing. Copies with fresh ids, not a live view of the source list — the alternative makes
deleting one flag on one row change a different composition, and every other place that
composition is used, which is spooky action at a distance for a right-click menu.
**Pre-composing** copies the comp's markers into the new composition (shifted by the same
amount `adjust_duration` shifts the layers, and dropped if they fall outside the new span
rather than parked where nothing can reach them) and gives the Precomp layer **none**: the
cues are on the ruler above it already, and drawing them on the layer too would say it
twice. `BridgeLayerInfo` carries the list with each marker's comp frame precomputed —
layer-local time plus the layer's start offset, exactly as a Sequence clip's is — so the
bar draws them with no bridge call, and dragging the layer moves them.

Two things this leaves: §10's *`8` during playback = beat tap* now wants a key of its own
or a modal reading, since the bare digits are spoken for; and `set_markers` still flattens
every marker to `MarkerKind::User`, so dragging a marker turns detected beats into ordinary
cues and *Clear beat markers* stops finding them. That was already true of the dialogue's
remove button and is not made worse here, but it is now much easier to hit — it wants a
kind carried across the bridge.

**K-255 · DECIDED · Menus navigate by hover, and the barrier stops blocking the pointer.**
K-194 gave the menus submenus and left every one of them behind a click: with File open,
reaching Window meant dismissing File and clicking Window, and a submenu — Open recent,
Layer ▸ New, an Effect category — only appeared when its row was clicked. Every desktop menu
bar these sit beside hands over on hover once a menu is open, and flies a submenu out under
the resting pointer. The obstacle was `showLumitPopup`'s full-window barrier: `HitTestBehavior
.opaque` swallowed hover as well as clicks, so neither the bar nor the menu underneath ever
felt the pointer move. Menus now pass `hoverThrough: true`, which makes that barrier
*translucent* — it still wins the click, being above what it covers, but the pointer reaches
through. Only menus opt in: a dropdown that let the panel behind it light up under the pointer
would be answering to a click it will never get. Two pieces of state follow. The bar keeps one
`_openHeading`/`_closeHeading` pair (only one menu is ever open) and clears it on the open
menu's *disposal* rather than on its close call, so a menu that goes with its window leaves the
bar out of menus too. Each `FloatSurface` carries the hovered row of its own surface, because
the row that flies a submenu out is never the row that has to take it back — the sibling the
pointer moves to is — and scoping it to the surface keeps a flyout's own rows from disturbing
the menu it came from. Regression tests: the two hover tests in `menu_bar_frb_test.dart`.

The same pass made the Keymap page's clash test wait for the engine rather than for luck: the
rebind it makes before opening the page is an frb call onto the worker thread, not done when
it returns, and the Linux runner built the banner before the clash existed. It now settles on
`keymapConflicts()` first. A test that passes because the machine is quick is not passing.

**K-256 · DECIDED · Lens flare is simulated, not sprited — and its oracle is staged.**
The Tier 1 suite gains a Lens flare effect (docs/08 §3.27, docs/impl/lens-flare.md):
ghosts ray-traced per frame through bundled real lens prescriptions (static data in
`lumit-core`, sourced from published patents), coloured by quarter-wave coating
interference, rasterised as warped ray grids through a hardware render pass; the
starburst is the aperture's Fourier diffraction pattern, baked on the CPU at
parameter-change time along with the aperture image and the FRFT ghost disc. The
reference implementation studied end-to-end is the GPLv3 realflare renderer; the port's
four deviations are pinned in the impl note (Cauchy-from-Abbe dispersion instead of a
glass catalogue; deterministic per-element coating wavelengths instead of a per-element
editor; wgpu hardware rasterisation instead of realflare's software binner, which existed
only because OpenCL has no raster pipeline; Rec.709 working primaries instead of AP1).
Three consequences are decisions rather than details. **(1) The §1.6 oracle is staged**:
the CPU twin matches the GPU trace ray-for-ray at tight absolute bounds (positions to
microns, reflectance to 1% — GPU transcendental builtins are not correctly rounded, so
an ULP promise through sin/asin/acos would be unimplementable) and the baked textures are
shared bit-for-bit, but the rasterised frame is held to a perceptual bound (mean error +
total energy), because hardware fill rules pin no per-pixel contract a CPU scanline twin
could meet at ULP tolerance — the flow-field precedent (§1.6) extended to raster fill.
**(2) The CPU degradation rung renders the effect as a labelled no-op**, the K-114 LUT
precedent: a software rasterisation of a heavy flare is not a usable fallback rung.
**(3) Full-image convolution is a non-goal**: flaring every pixel of an HDR frame (the
offline batch-tool technique) costs seconds per frame and cannot preview; the recorded
path for image-driven flares is the top-K highlight detection spec'd in the impl note §6.


**K-257 · DECIDED · The Lens flare panel is reshaped by owner design, and its sources
become modes.** The first owner pass over K-256's effect. **(1) The panel**: the light
becomes one x/y point row with a pick-on-Viewer dropper — the docs/07 §6.1 pair, built
generically: any adjacent `_x`/`_y` Float pair folds into one row, and a declared
%-of-frame allowlist (the flare's light, Radial blur's centre) gets the position dropper.
Parameter groups finally CROSS the bridge (`list_parameter_groups`) and render as
sub-twirls in the Effect controls card — the schema's K-145 groups had never reached
Flutter — with an added `visible_when` (a group shown only while a named sibling Choice
holds a value) and an empty-label headerless form, which is how per-mode controls appear.
**(2) An Int parameter kind**: Blades and Max ghosts are whole numbers; the value stays a
Float scalar (animation and serialisation unchanged) and only the schema kind, bridge and
row change — replacing the "rounded float row" convention. **(3) Scale scales the whole
flare** about the optical centre, not just the starburst; the starburst's own rotation
and softness parameters are deleted (pre-release, no migration; the stochastic bake
jitter speckled and the spectral integration is the honest smear). **(4) Coating type**
presets pick the per-surface coating-tuning pattern (modern multicoat / vintage single
coat / warm / cool) — the λc cycle is a bake input. **(5) The wavelength ladder rises to
3/5/7/9** per quality: the owner read the 3-band fringe as an RGB split, correctly — bands
are integrals, and five is where dispersion reads smooth. **(6) Source modes**: Manual
light (the tracked point), **Matte** (shipped — impl note §6: deterministic on-GPU top-8
bright-source detection over a referenced layer, threaded exactly as the DoF depth pass,
each source spawning a full flare tinted by its pixel through a soft threshold gate), and
**Lights** (prepared: the Choice exists and resolves as Manual until light layers land as
flare sources). **(7) The lens library doubles** to six real prescriptions and drops the
codenames for the lenses' actual names. The **lens designer** — a window where the user
builds a prescription element by element with a live diagram, as the reference apps do —
is recorded as the intended custom-lens path (TODO, Later), ahead of flat custom-file
loading.

**K-258 · DECIDED · The flare's exposure closes the loop, its quality ladder goes
photo-real, and stacks forward-migrate on load.** The owner's second pass over the Lens
flare. **(1) Loaded stacks backfill**: a built-in instance saved before its schema grew a
parameter lacked it — the panel drew a dash and `set_value` refused the id. `open()` now
appends every missing declared parameter at its default (`backfill_builtin_params`);
values present are never touched. This is the general forward-migration for every effect,
not a flare fix. **(2) Closed-loop auto-exposure**: the probe-median and probe-flux
exposure proxies both mispredicted real lenses by orders of magnitude (ghost energy
depends on where a design's caustics land at render framing — the Petzval read 30× dim),
so the bake now renders the actual CPU reference at thumbnail size with gain 1 and
normalises the measured mean to a target. Deterministic, milliseconds, and every bundled
lens lands in one exposure band by construction. **(3) The quality ladder rises** to
4/8/16/32 traced bands with grids 16/48/80/128 and a 512² ghost disc — at Ultra the
spectral rims are effectively continuous (the owner's "photo-real to the extreme" tier,
~250 ms a frame at 960×540 on the dev RTX; Normal stays real-time at ~24 ms). **(4) The
launch square covers the whole front element** (2.3× its half-height): the previous
undersized square was visible in the picture as a rectangular ghost boundary — the
bundle's own clip instead of the housing's feathered vignette. **(5) Background**
(Transparent / Black): Black makes the live output opaque — the flare-element-over-black
export for Screen/Add workflows; the neutral passthroughs stay bit-exact. **(6) Four
classic public-domain designs join the library** (Cooke triplet 50/3.5, Tessar 50/2.8,
Petzval 85/2.2, Double Gauss 58/2 — the design names, era-authentic) for ten lenses
total, and three coating presets join (Amber single coat, Two-tone vintage, Broad
multicoat) for seven.

**K-259 · DECIDED · The flare's light carries a tint, and a source may keep its own
colour or not.** Two controls the owner asked for, plus the schema generalisation they
needed. **Light tint** (a Colour parameter — so it gets the inspector's picker *and* its
eyedropper for free) multiplies every light's colour in **every** source mode: in Manual
it simply *is* the flare's colour, in Matte it tints what the detected sources
contribute. It is a frame-time value, deliberately outside the bake key, so animating it
never rebakes. **Use source colour** (Bool, default on) chooses whether a detected
source's own colour rides with it: on, a warm practical flares warm and a cool one cool;
off, every source flares white through the tint alone — which is what a matte used purely
as a *position* mask wants. Both paths compute the light as `(use_source ? source rgb :
white) × gate × tint` in one shared expression, mirrored by the CPU reference and the
WGSL detection. The toggle is shown for **Matte and Lights alike**, which the K-257
`visible_when` could not express — it named one Choice value — so it now names a **set**
of them (`Option<(&str, &[u32])>`, `visible_when_values` across the bridge). Version
bumped 2 → 3; pre-release, and loaded stacks backfill the new parameters at their
defaults through K-258's migration anyway.
**K-260 · DECIDED · The flare's optics are calibrated against the lens itself, focus
distance is a parameter, and point parameters are authored in comp pixels.** An accuracy
pass taken directly from the reference renderer's author. **(1) Calibrated sensor**: the
sensor plane sits at the prescription's *measured* infinity focus — the bake traces one
paraxial marginal ray through the main path and places the sensor at its axis crossing —
not at the patent table's trailing gap (off by up to 10 mm on bundled lenses). The same
trace measures the true EFL, which replaces the label focal length in the light direction
and focus maths; measuring vindicated his warning that patent tables are not normalised
to the label (the Zeiss "50mm" measures 64.8 mm). The four classic reconstructions are
rescaled to land within 0.03% of their labels; real patent tables stay as published.
**(2) Focus (m)**: the thin-lens focusing extension `f²/(1000·d − f)` mm shifts the
sensor at trace time — frame-time, outside the bake key, so pulling focus never rebakes —
and rearranges the whole ghost train, the "same lens, different focus, completely
different flare" behaviour. **(3) Padded FRFT**: the aperture embeds centred in a 2×
zero field before the ghost-disc transform (centre-cropped after); unpadded, the circular
transform wrapped its own ringing into banded arcs across every ghost. **(4) Wide-open
iris**: effective roundness is `max(user, 1 − clamp(fstop/native − 1, 0, 2)/2)` — at the
native stop the iris retracts behind the circular bore and ghosts go round regardless of
blade count. Roundness itself became an SDF lerp toward the true circle; the additive
sine bulge it replaces pinched into a flower near 1 (caught against the reference's round
wide-open ghosts). **(5) Point parameters are comp pixels** — standing convention: any
x/y point parameter (the flare's Light, and every future one) is authored in px@comp,
never % of frame; the Viewer dropper converts its fraction through the comp size at click
time. Radial blur's legacy % centre is grandfathered until migrated.

**K-261 · DECIDED · The flare adopts the FlareSim optical model and its 1299-lens
library, rendered through the energy-conserving quad grid.** The owner pointed at
FlareSim (github.com/SeanBRVFX/FlareSim_Nuke_builded, building on space55's renderer) as
the reference and asked for its lens files in Lumit; its model is reimplemented here from
understanding — not translated (the repo carries no licence; the lens files are
transcribed patent data from PhotonsToPhotos, each citing its patent). **(1) The
library**: 1299 .lens prescriptions embedded in lumit-core as text, parsed on selection
(no IO, no panics); the Lens dropdown lists them all; native f-numbers come from the
collection filenames. The K-256 ten-lens hand-built table and the K-257 Coating-type
presets are gone — every prescription carries its own per-surface AR coating layer
counts, and the Coating dial blends bare glass toward them. **(2) The optics**:
FlareSim's three-phase walk (forward to the far bounce, backward to the near, forward
out), all-pairs ghost enumeration with an interface filter and on-axis brightness probe,
per-surface MgF₂ quarter-wave/multicoat reflectance, an iris mask (blades / roundness /
softness) weighting a regular pupil grid sized to the entrance pupil, and the f-stop
scaling the stop and pupil together. The K-260 focus shift and wide-open roundness
carry over; the K-260 paraxial sensor calibration is superseded — these files carry
measured back-focal chains. **(3) The renderer**: FlareSim's own Monte-Carlo point
splatting was built first and measured — at photographic ghost sizes it needs orders of
magnitude more rays than the K-256 quad-grid area method for the same smoothness, so the
quad grid (with the K-261 flux-exact sub-pixel caustic inflation, and a housing feather
with a 10% clip skirt so bundle edges fade instead of stepping) renders the FlareSim
optics noise-free. The FRFT ghost-disc texture is gone — the ghost's shape IS the warped
grid × mask. **(4) Ghost softness** (new parameter, FlareSim's Ghost Blur): a 3-pass box
blur in % of the frame diagonal, frame-time. **(5) The exposure gain cap drops to 64**:
a lens whose every ghost is extreme defocused wash has almost no probe energy, and the
unbounded K-258 loop amplified the residue into artefact fields; capped, such glass
renders honestly dim. Schema v3 → v4 (coating_preset removed, ghost_softness added;
pre-release, old lens indices land on different-but-valid library entries). Known limit,
pinned in TODO: extreme-defocus prescriptions show grid-aligned steps at low quality —
adaptive refinement is the follow-up.

**K-262 · DECIDED · The flare's fold artefacts are fixed at the guard, the ray budget
follows ghost size, and a 1299-option dropdown gets a searchable picker.** The owner's
first pass over the shipped FlareSim model found three faults. **(1) The streaks.**
K-261's sub-pixel inflation scaled ANY quad under 4 px² up about its centroid — correct
for a small compact cell, wrong for a fold-straddling *sliver*, whose near-zero area made
the scale factor up to 100× and stretched a 20 px sliver into a 2000 px line: the "random
lines across the flare". Inflation is now restricted to compact quads; long-and-thin ones
(past 4% of the frame diagonal with `longest² > 8 × area`) are dropped at any size, and
the caustic density cap tightens from 10 000× to 333×, which removes the hard chromatic
spikes while keeping the bright rims. Note the failure mode: **both oracles agreed while
drawing the artefact**, because the CPU and WGSL mirrored the same wrong formula — so the
regression pin is a unit test of the guard itself, verified to fail on the K-261 code.
**(2) Normal quality.** The pupil grid is now allocated **per pair** by its measured
image spread (½× to 2.5× the ladder base, which itself rises to 32/64/96/144): a
frame-filling defocused ghost gets the cells it needs and a tight blob stops wasting
them. Ghost softness accordingly defaults to 0.05 instead of 0.3, and **0 is a clean
setting** — the blur was hiding artefacts, not adding character. **(3) The crash.** A
1299-row dropdown built every row eagerly inside an `IntrinsicWidth`, which walks them
all twice; the app died in a Flutter layout pass with a thread-allocation failure. Long
option lists (≥ 40) now get `BareSearchDropdown`: a search field over a lazily-built
`ListView.builder` with maker headings, and the library's labels are regenerated as
`Maker · Model` (53 normalised makers, series names like "AI Nikkor" folded onto Nikon)
so grouping and typing both work. Also capped the Ghost-softness blur radius at 80 px —
an uncapped 2% radius on a 4K frame is ~1000 taps per pixel across six passes, a GPU
timeout waiting to happen. The default lens index moves with the re-sort (pre-release; a
saved index lands on a different valid lens).

**K-263 · DECIDED · The flare's frame is bounded, batched by its own grid, and pooled —
no submission the watchdog can kill, no per-frame megabyte churn.** The owner tried the
shipped K-262 flare on a Mac: after a few passes through the lens picker the Viewer
stopped updating and nothing brought it back, not even opening a different project. That
last detail is the diagnosis — a project is a fresh worker but the *process* keeps the
graphics device, so a frozen picture that survives a new project means the device itself
is gone. Three things fed it, and all three are fixed at the cause rather than by
lowering quality; the picture is unchanged (the CPU-oracle parity, determinism and
bit-exact-neutral tests all hold). **(1) One unbounded submission.** The whole frame —
every ghost, wavelength and light — went to the card as a single command buffer whose
size the user sets through Quality, Max ghosts and the source mode. macOS and Windows
both kill a submission that runs too long, and that kill takes the device, which is
exactly the symptom. The frame is now cut into submissions at a fixed ray–surface-step
budget; the batches queue in the same order and blend in the same order, they are merely
handed over in pieces. **(2) The scratch was allocated per frame and sized by hope.**
K-262's budget bottomed out at one combo and then let eight lights at an Ultra grid ask
for ~100 MB anyway — allocated and dropped every frame, which on a unified-memory Mac is
how a flare fills the memory it shares with everything else. The budget is now a hard
bound (the light dimension splits too) and the buffers are pooled and reused. **(3) One
stride for the whole frame.** The scratch was strided by the widest grid in the frame and
the vertex pass ran over that stride to park what a narrower batch did not fill — so a
single frame-filling ghost made every compact ghost dispatch and draw at *its* cell
count. A batch is a run of combos at one grid, so each now strides by its own and draws
exactly its own cells. Alongside those: the energy pass folded into the vertex pass (it
re-read the same four rays to recompute the same area); a drawn corner is 20 bytes stored
once per cell instead of six 32-byte vertices; the Ghost blur sums through a workgroup
line cache (~3.5 texture fetches a pixel where 161 was the worst case); the aperture
feather tracks its radius squared and takes one square root a ray instead of one per
surface; the bake stops interpolating the CIE table 6.5 million times for a hundred
answers, hoists the wavelength-independent iris mask out of the wavelength loop, and
measures ghost spreads only for the 200 pairs a frame can reach. The bake cache stops
emptying itself at the cap and evicts oldest-first at 24 — clearing the lot made trying
lenses quadratic. Measured on a software rasteriser: a 960×540 Normal/60-ghost frame
3.03 s → 2.30 s, and a bake 0.80 s → 0.66 s. Also new: `tests/wgsl_validates.rs` parses
and validates every shipped WGSL kernel with naga, so a broken shader fails on any
machine instead of only on one with a graphics card. **Not** fixed here and recorded in
TODO: the bake still runs on the render thread, so picking a lens blocks the picture for
about half a second — the fix is the progress indicator and an off-thread bake; and the
raster still draws culled cells, which a deterministic prefix-sum compaction would skip.

**K-264 · DECIDED · The flare's density lives at the grid corners, its raster is
multisampled, a ray never dies at an aperture, the library is a curated twenty, and a
.lens file is a parameter.** The owner's Ultra pass after K-263 showed triangles in the
ghost rims, blocky faceting, and jagged edges — and asked for the library to shrink from
1299 to at most twenty distinctive lenses plus a user-loadable prescription, with the
Ghost softness default at 0.02. Nothing here lowers quality to hide anything; every fix
is at the reconstruction. **(1) Vertex-smoothed density** ([Hullin et al. 2011]'s
per-vertex rule, confirmed against the published implementations): a corner's density is
the launch cell area over the mean landed area of the live cells touching it,
interpolated by the raster — the K-256..K-263 per-CELL density was constant inside a
cell and jumped at its edge, and that repeated discontinuity WAS the faceting and the
moiré. The caustic cap now floors the mean, so fold clusters still top out at 333×.
**(2) 4× multisampling** on the ghost raster (resolved into the flare buffer), with the
CPU reference modelling the same four standard sample positions — coverage times
centre-interpolated colour — so the oracle stays tight. Sub-sample inflation's floor
drops 4 px² → 1 px² accordingly; at 4 it also caught the merely small and tiled
small-rendered wash ghosts with overlapping diamonds. **(3) Geometry never dies at an
aperture.** Every binary kill in the walk — the iris mask, the housing skirt, a missed
sphere, total internal reflection — sampled its boundary at pupil-cell granularity and
drew it as a staircase. All four now continue the ray with weight forced continuously to
zero (mask multiplies; the feather clamps its denominator to the glass's own extent so a
transcription error cannot outrun it; a miss continues virtually through the vertex
plane; TIR continues straight — its transmitted Fresnel already fades to zero). Cells
spanning from lit geometry to a distant virtual landing pull their unlit corners to
within one local cell-width of the lit centroid, so the fade lands where the boundary
is: drawn they fanned lines, dropped they notched rims — K-262's streak drop is
superseded, long thin fold cells draw, and the trace oracle pins positions only for rays
carrying weight. **(4) The adaptive budget probes off-axis too** and takes the larger
spread — on-axis-only handed frame-filling off-centre ghosts the half grid. **(5) The
starburst sprite fades radially to zero** inside its border; its pedestal used to end at
the quad edge as a hard grey square. **(6) The library is twenty curated lenses**
(owner-decided; verified distinct by rendering all twenty through the pipeline into a
montage and looking): cine multicoat, 1930s uncoated exotics, Tessar, f0.95, fisheye,
process glass, superzoom, long telephoto. Saved pre-K-264 indices land on a valid
curated lens (pre-release). **(7) `lens_file`** (the LUT File pattern): a user's .lens
in the same Optical Bench format overrides the pick entirely, content-hashed into the
bake key so edits take effect next frame; unset/missing/unparsable degrades to the pick.
The K-262 searchable picker stays as the guard for any future long Choice list, pinned
by a synthetic-options widget test. **(8) Ghost softness defaults to 0.02**
(owner-set): taste, not cover. Verified by eye through a new `#[ignore]`d GPU
frame-dump harness (before/after PNGs at Ultra on the artefact lenses) — the faceting,
moiré, rim notches and staircase edges are gone at Ultra with a mild ripple left on
hard vignetted edges at Normal (known limit, TODO'd with adaptive refinement). Cost, measured
on the software rasteriser: a 960×540 Normal/60-ghost frame 2.30 s → 3.32 s — the
multisample fill and the always-completing walks are real work, mitigated by skipping
pupil corners so far outside the iris that no cell they touch can hold light (a fifth
of the square, bit-identical output); on a hardware GPU the multisample share of that
is near-free. Accepted: the owner's bound was quality at fixed settings, and the K-263
submission splitting keeps any of it from becoming a watchdog kill. One driver find
for CI: dynamically-indexed `let` arrays in WGSL crash lavapipe's shader compiler —
use `var`.

**K-265 · DECIDED · The flare's budget is the user's dial, every bundled lens must
flare everywhere, and the big allocations are pooled — found live by the owner within
the hour of K-264.** Five reports, five causes. **(1) The application died after
minutes of lens-switching.** The K-264 multisample target — the effect's largest
allocation, ~66 MB at a 1080p flare buffer — was created and dropped EVERY FRAME,
exactly the rolling-backlog disease K-263 diagnosed for buffers, felt as renders
slowing over minutes and then the process dying. Pooled now, keyed by size, beside the
ray scratch. **(2) Several curated lenses rendered nothing.** The K-264 montage was
judged with a near-centred light, and lenses that only work there slipped through: a
three-position probe (centre, off-centre, far corner) now stands behind the curation,
and it found the Kinoptik, both fisheyes, every wide-angle retrofocus design, both
superzoom compacts and the 50mm Tessar file either dead off-centre or baking ZERO
ghost pairs. The library is re-cut to twenty verified-alive, still maximally distinct
lenses (in: FD 300/2.8, Canon 50/1.2 LTM, both Noktons, Elmarit 90, Summilux-C 100
cine, Orestor 135, Ultra Prime 135, Tessar 100/4.5, DEM 180 APO; out: everything the
probe failed, plus the Projection Optics wash — an 8-diagonal ghost no bounded grid
can sample, the recorded §4 limit). **Wide-angle and fisheye prescriptions are a
recorded model limit**: the trace's angular acceptance collapses off-axis for
retrofocus designs, so none are bundled — a lens that only flares with a centred
light is a bug report, not a look. **(3) The Lens file row could not pick a file.**
The File parameter row was display-only — true for the LUT too since the Flutter
port. It is now the picker: click opens the dialogue through the schema's own
extension filter, set rows grow a clear button, and the fix covers every File
parameter. **(4) "Let me choose the rays myself."** A new **Detail** dial (0.25–4,
default 1) multiplies the Quality tier's pupil grid AND its wavelength count through
shared helpers, because the owner's toothed EF 70-200 corona proved more rays alone
cannot dissolve spectral banding. The K-262 half-grid rung for small-spread pairs is
gone — a small ghost is not a cheap ghost, its caustic rim carries structure the
blob-size probe cannot see — and the grid clamp rises to 512 for the dial's headroom.
Software-rasteriser cost for the default Normal frame: 3.32 s → 4.41 s (the halved
pairs quadrupled), accepted for the same reason as K-264. **(5) The corona that
remains on the EF 70-200 at f1.5 is pinned as a known limit** after ablating, one by
one: grid 72→288, wavelengths 32→64, the pull-in reach (mean → smallest neighbour —
kept, it is the right scale), sub-sample inflation off, a local branch-jump cull
(reverted: it nicked interior caustics without touching the corona), and a 3× wider
housing feather. The corona is invariant to all of them: it is the fold structure of
that ghost in an extrapolated regime (shooting an f2.8 zoom at f1.5), and the recorded
fix is adaptive refinement at folds, not another guard. The ablation list lives here
so nobody re-chases it.

**K-266 · DECIDED · Px parameters survive preview scaling, the flare stops at its own
buffer, weight cliffs smooth, and a precomp is a legal layer input.** Four owner
reports from live use. **(1) The light landed past where it was put** — at 1500 of a
1920 comp, both axes off by exactly the preview factor. An ADJUSTMENT layer's stack
resolves px@comp parameters with factor 1 ("runs on the comp-sized intermediate"),
but under reduced-resolution preview that intermediate is the preview raster: every
px-dimensioned parameter on an adjustment layer — the flare's light, DoF apertures,
blur radii — ran 1.28× too big/far, in preview only. New `fx::rescale_px` scales
every px field of resolved ops (exhaustive match, so a new op must declare its px
fields), and the realise walk applies `render_width / comp_width` to adjustment
stacks before running them. The Precomp-layer variant of the same disease is TODO'd
with the diagnosis. **(2) Anamorphic squeeze below 1 smeared the frame edge**: the
combine's clamp-addressed tap repeated the flare buffer's border row outward.
Outside the buffer there is no flare — the tap returns zero, both twins, pinned by
test. **(3) The chunky wash edges** (the owner's Ultra Prime screenshot): a weight
cliff — the housing feather compressed into less than a cell, a vignette cut —
lands inside one cell and draws as facets. A corner's COLOUR weight is now the mean
over its 3×3 ray neighbourhood (dead rays as zero), turning any cliff into a
two-cell ramp; geometry decisions stay on raw weights so virtual continuations
cannot smear light into fan lines. Verified on the reported lens: the staircase is
gone. **(4) A precomp as a Matte source detected nothing** — `pixels_for` has no
pixels for a comp, so the reference silently became "no matte". Layer inputs
(flare mattes AND DoF depth, same mechanism) now carry an optional nested draw
list, built with the ancestor-path cycle guard and realised recursively exactly as
a Precomp layer's picture is. Boundary: footage inside a matte-only precomp still
needs the decode planner taught (solids, text, shapes, nested renders all work) —
TODO'd. On area sources generally: Matte mode approximates an area by its
brightest eight points with non-max suppression; raising that count and sampling
the area proper is recorded as the follow-up rather than pretended.

**K-267 · DECIDED · The grid budget re-measures at the frame's light, the flare buffer
pads for squeeze/scale, and an area source weighs as its whole lit area.** Third live
round on the same branch. **(1) "Still choppy at the corner" (7Artisans).** The bake's
image-spread probe is a bounding box, and the corner measurements showed the box does
not grow at corner lights — what grows is the worst LOCAL stretch: a pair the same
overall size stretches ~6× near a fold, and those cells are the polyline edges. New
frame-time probe (`frame_grid_needs`, the Hullin patent's "grid resolution adapted at
runtime, guided by bounding shape estimations"): a 12×12 weight-gated trace per
renderable pair at the actual light direction measures the worst adjacent-landing
distance and derives the grid side that would keep the largest cell under 0.5% of the
sensor diagonal. Uncapped this septupled a frame (24 s on the software rasteriser), so
the raise is BUDGETED (`plan_frame_grids`): half again over the frame's rung-grid ray
baseline, spent worst-stretch-first with partial grants, per-pair cap 3× the rung,
hard clamp 512. Manual mode only (Matte lights exist GPU-side; both twins gate the
same way). The probe runs in lumit-render through the same seam callback as the bake
and returns the FINAL per-pair grids, so the CPU reference and the GPU dispatch cannot
disagree about a single ray. Cost at the default Normal frame: 3.35 s → 5.76 s
software-rasterised, the bounded price of the last three rounds' artefact class;
Detail stays the user's dial in both directions. **(2) Anamorphic below 1 "cuts to
black at the edges"** — K-266 honestly stopped the edge-repeat smear and honestly
showed there was no flare past the buffer. Now there is: the ghost buffer renders
PADDED (`flare_pad_dims`, up to 2× per axis, geometry centred, screen transform and
blur radius still derived from the base dims), and the combine's tap only gains a
constant border offset — zero when unpadded, bit-compatible. Past even the 2× cap
(squeeze below 0.5 at Scale 1) the zero-outside rule still holds, pinned by the same
test. **(3) "Eight brightest points, do better — now."** MAX_LIGHTS rises 8 → 16, and
detection stops pretending an area is a pixel: after the top-K anchor pick, EVERY
gated detection tile's flux — its brightest pixel's colour × its threshold gate —
accumulates onto its nearest anchor (Chebyshev, ties to the lowest index, fixed tile
order, CPU and GPU op-for-op). A one-tile point source is its own anchor's only
contributor and reads exactly as before; the owner's white-circle precomp now carries
the flux of every tile it lights. Position stays the anchor pixel; sub-tile centroid
positioning would be the next refinement if ever needed.

**K-268 · DECIDED · A precomp gates as a track matte, and an effect on a Precomp layer
keeps its pixels under a reduced preview.** Two holes on the same seam — what a Precomp
layer *is* to the code that consumes it — found by reading K-266's own recorded
boundary. **(1) A precomp set as a TRACK matte gated nothing at all.** K-266 fixed the
layer-input mattes (a flare's Matte source, a DoF depth pass) by giving them an optional
nested draw list; the track matte — `Layer::matte`, the row everyone actually reaches for
— still ran through `pixels_for`, which has no pixels for a comp, so the matte silently
became "no matte" and the consumer drew everywhere. `MatteDraw` now carries the same
`nested` draw list, built by the shared `nested_comp_draw` helper (one ancestor-path
cycle guard for both kinds of reference) and realised recursively exactly as a Precomp
layer's picture is. The source-mode toggles (None / Masks / Effects and masks) do not
apply to a comp reference — a comp already carries its layers' own masks and effects —
which is the K-266 boundary, unchanged and now shared. **(2) K-266's recorded boundary
was on the wrong side.** "Footage inside a matte-only precomp needs the decode planner
taught" — the planner was already teaching it: `collect_comp_jobs` puts a matte source
and a layer-input reference into `wanted` whether or not the layer is visible, and a
Precomp among them recurses. Pinned now by a test over both shapes of reference, so the
next reader gets a passing test rather than a note to re-derive. **(3) The Precomp twin
of K-266's px@comp drift.** Effects ON a Precomp layer resolved px@comp parameters at
factor 1 against the nested comp's width while running on the nested comp's *preview*
raster, so a Transform's offset, a flare's light or a blur radius drifted by exactly the
preview factor — preview only, full resolution always correct. The Nested arm now carries
`fx_ref_width` (the nested comp's own width) and the realise walk applies
`raster_width / ref_width` through `fx::rescale_px` before running the stack, which is
the identical correction the adjustment path has taken since K-266. Both fixes land with
end-to-end GPU regression tests (a matte that must gate, a shift that must stay a quarter
of the frame at Full and at Half); both fail on the code as it stood.

**K-269 · DECIDED · A skipped GPU test is a failure where an adapter was installed, and
the no-hex rule follows the widgets into Dart.** Two CI gates that read as coverage and
were not. **(1) `LUMIT_REQUIRE_GPU`.** Every kernel test skips itself without a graphics
adapter — the friendly behaviour on a developer's machine, and the reason a Linux job that
*installs* Mesa's lavapipe could lose its Vulkan driver, run none of about ninety shader
oracles, and still report green. `lumit_gpu::no_adapter()` is now the one skip site
(89 call sites converted from a bare `eprintln!`), and with `LUMIT_REQUIRE_GPU` set to
anything but `0` it panics instead. The Linux job sets it; macOS and Windows deliberately
do not, because nobody has confirmed those runners enumerate an adapter and a gate is only
worth having where it has been verified — flipping them on is a TODO with a one-run test.
The rule itself (unset/empty/`0` skip, anything else demand) is unit-tested rather than
living only in a workflow file. **(2) The design-token lint greps Rust, where no widget has
lived since K-182.** All the colours are in Dart now, so the same rule runs over
`flutter_ui/lib` outside `theme/`: hex `Color(0x…)` literals, Material's `Colors.*` palette,
and `Color.fromARGB`/`fromRGBO` calls built entirely from number literals. It found three:
a modal scrim spelled out as `0x99000000`, and `Colors.red`/`Colors.amber` standing in for
the theme's error and warning roles. The scrim becomes a **token** (`LumitTheme.scrim`),
defaulting from the mode in the K-202 manner rather than being restated by all seven
schemes — translucent black in both families, lighter on a light scheme where the same
opacity reads as a blackout. Two shapes stay legal and are documented in the job: fully
transparent `0x00000000` (the absence of a colour, not a choice of one) and a colour
rebuilt from stored numbers (data, not a design decision).

**K-270 · DECIDED · A marker write-back merges onto the marker that is already there.**
The panel writes its whole marker list back through `set_markers`, and a `BridgeMarker`
carries the three fields a panel can edit: id, time, label. Each one was then rebuilt from
those three alone, which silently reset the three the engine owns — the **kind** (a
detected beat's provenance and its confidence), a spanning marker's **duration**, and the
**`extra`** map that keeps fields a newer Lumit wrote (docs/10 §1.1). So dragging a beat
marker one frame turned it into an ordinary cue, and *Clear beat markers* then walked past
it; K-254's ruler markers put that one drag away. Fixed by merging rather than converting:
each incoming marker is matched by id against the list the document holds and keeps that
marker's kind, duration and extra; an id the document has never seen is a plain user
marker, which is exactly what a marker the panel just made is. **Deliberately not** by
adding the kind to the frb struct (the TODO's own suggestion): the panel has no control for
a kind, no use for one it cannot edit, and inventing a UI to fix a data-loss bug is the
wrong order — while the merge also saves the duration and the forward-compatibility fields,
which no widening of `BridgeMarker` was going to cover. Both the composition's list and
every layer's own (K-254) go through the one helper. Also recorded: the TODO entry claiming
installed RAM is read only on Windows was stale — K-204 answers it on all three desktops;
only `video_memory_bytes` is still Windows-only, and the entry now says that instead.

**K-271 · DECIDED · The LUT kernel remaps through the cube's own domain, and the cube
cache notices the file changing.** Both halves of [impl/lut.md](impl/lut.md)'s recorded
K-114 gaps, closed together because they are the same effect's two ways of showing the
wrong grade. **(1) The domain.** `fx_lut.wgsl` assumed the default `0..1` input domain and
skipped the `(c - lo) / (hi - lo)` remap `Lut3d::sample` applies, so a `.cube` declaring a
`DOMAIN_MIN`/`DOMAIN_MAX` — the log and display-referred cubes a grading tool exports —
rendered silently wrong on the GPU while the CPU oracle was right. `LutParams` now carries
the six floats (two padded `vec4`s; a uniform `vec3` is 16-byte aligned regardless) and the
shader remaps operation for operation, including the zero-span guard: a `DOMAIN_MIN` equal
to its `DOMAIN_MAX` reads as 0 on both paths rather than dividing. Chosen over the recorded
alternative (refusing such cubes as a labelled no-op) because the maths was already written
down in §2 and the file is not wrong — Lumit was. The oracle test gains an asymmetric
non-default-domain cube and a degenerate zero-span one; the old shader misses the first by
23684 fp16 ULP. **(2) The cache.** One `LutCache` keyed by `(path, mtime)` and bounded to
eight entries, most recently used first, replacing the unbounded path-only map. Grading is
iterative — export the cube, look, adjust, export again over the same path — and keyed by
path alone the second export never appeared until the application restarted, with nothing
on screen to say the file and the picture had parted company. A stale entry for a path is
replaced rather than kept beside the new one; a path that cannot be stat'd keys as `None`,
which still matches itself, so it is cached by path exactly as before instead of being
re-read every frame.

**K-272 · DECIDED · The toolchain is pinned and dependency hygiene is a CI job.** Two of
[14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md) §9's owed tools, which are the same
promise from two directions: what this repository is built *with*, and what it is built
*from*. **(1) `rust-toolchain.toml` pins 1.97.1** (with rustfmt and clippy) — the version the
repository was already building on. **A pin must name the version the repository is already
on**: the first attempt here named an older one, which is a downgrade in disguise, and CI
said so twice — an `objc` macro tripped `unexpected_cfgs` in the macOS-only Metal path, and
the bridge's generated code came out naming a different derive-expansion helper
(`assert_receiver_is_total_eq` for `assert_fields_are_eq`), so two jobs failed for reasons
that had nothing to do with what was being pinned. Without the pin at all,
"stable" means whatever each machine happens to have, and `-D warnings` turns a compiler
released mid-week into a red build on a commit that changed nothing. Every CI job installs
stable and then lets the file decide, so there is one place to raise it — deliberately,
with the suite run and the changelog written, never incidentally. **(2) `cargo deny check`
runs on every push** over licences, advisories, wildcards and sources, through the upstream
action (it ships the binary and caches the advisory database, so the job is seconds rather
than a three-minute build of a tool that reads a lock file). The allowed-licence list is the
GPLv3-compatible permissive set plus Lumit's own GPL-3.0-only, so a dependency with any
other licence stops the build until someone decides deliberately — which is exactly the
conversation §9 already asks for in the pull request. Three unmaintained-crate advisories
are ignored **by id, each with what it would take to leave it** (`ttf-parser` via fontdue
wants the `skrifa` migration; `bincode` 1.x and `paste` leave when their parents update):
failing every build over a transitive dependency nobody here can update trains people to
ignore the job, and an unmaintained crate is a different question from a vulnerable one —
`yanked` and real advisories still deny. Duplicate versions warn rather than fail (wgpu and
rsmpeg each bring their own stack). Every workspace crate gains `publish = false`, which is
both true — they are the application, not libraries — and what lets the wildcard rule see a
`path` dependency between our own crates as the non-problem it is.

**K-273 · DECIDED · The feature-less build builds, and an observer attaches to a store that
is already shared.** Two of the bridge's recorded rough edges. **(1)
`cargo build -p lumit_bridge --no-default-features` builds and tests again.** The rule was
already written down (docs/17 §Feature gates: the API surface is one shape whatever the
features, so the generated Dart is one shape everywhere) and the code had drifted from it in
three places: `api::beats` was gated *away* — so the checked-in generated code called a
function that did not exist — while `prefetch` and the thumbnail cache leaked
`lumit_media` types past their gates. Beats is now always compiled and its detection asks
the audio pipeline, answering `NoAudioPipeline` where a build has none, which is the shape
every other feature-sensitive call already uses. The decode-ahead thread still exists and
still drains its queue without the decoder; a build with the feature off is one that does not
*decode*, not one with a different scheduler. **What it is not**, and what the first version
of this entry wrongly claimed: a build without FFmpeg. `lumit-render` and `lumit-audio`
depend on `lumit-media` unconditionally and the bridge depends on both, so FFmpeg is still
linked — the feature governs the bridge's own decode paths, not the dependency tree. CI
caught the overstatement immediately, on a runner deliberately given no FFmpeg. Making the
tree genuinely media-optional is its own piece of work, TODO'd rather than implied. Chosen over the recorded alternative — dropping
the pretence that the features are independent — because the contract's promise is the
valuable half and only the code had lapsed. One behaviour change fell out of it: a footage
item whose file is not on disk now reports **Missing** in a media-less build too. Whether a
file exists is a question for the filesystem, not the decoder; it used to answer "ready".
**(2) `DocumentStore::set_callback` takes `&self`.** The observer had to be registered
before the store went into its `Arc`, which no type enforced and which fights the natural
shape of the thing: an observer usually wants to refer back to the object that owns the
store, and that object does not exist yet at that point. The callback lives behind a
`parking_lot::Mutex`, locked only to clone the `Arc` out or swap it and never across the
call itself — the existing no-locks-across-FFI and re-entrancy rules (docs/14 §3) are
unchanged, and their tests still pass.

**K-274 · DECIDED · An effect on a Null is labelled inert, never refused — and
anti-aliasing is a project property, on by default.** Two owner decisions on open
questions (2026-08-05). **(1) Effects on a Null layer.** The recorded choice was "either
refuse the drop or say plainly that the stack is inert"; the owner chose *inert*, with the
reason that decides the shape of it: **a Null is where a control lives when it is meant to
drive something else.** A Slider (or any parameter) on a Null is how a value gets published
for another layer's expression to read, so refusing the drop would remove a feature rather
than prevent a mistake — and this holds for every effect, not just the expression-control
family. So: the drop is accepted, the stack is stored, keyframed and sampled exactly as on
any other layer (pinned by `an_effect_on_a_null_layer_keeps_its_animated_value`), nothing
strips it, and the Effect controls panel says once, calmly, that a Null draws nothing so an
effect here changes no picture while its parameters stay live. When expressions land
(Phase 4, [12-PLUGINS.md](12-PLUGINS.md)) they read those parameters like any other; nothing
about this decision is deferred to them. **(2) Anti-aliasing** is a **project property, on
by default, with one value shared by preview and export** — it changes what a comp looks
like, so it must travel with the file and match on another machine, and a preview that
anti-aliases differently from the export would break the K-031 identity. That answers both
of the recorded open questions; the work itself (MSAA targets and resolve in the composite
pass, the setting through the bridge, and an adapter capability check — a sample count is
asked for, never assumed) stays in [TODO.md](TODO.md).

**K-275 · DECIDED · Selecting a layer means the same thing wherever it happens, and
layers and effects copy as documents.** Two owner requests (2026-08-05). **(1) The
selection is the shell's, and the Timeline must follow it.** Picking a layer on the
picture already replaced the whole selection, but the Timeline kept its *own* per-layer
state — the property selection, the graph's key selection, the row highlight — and cleared
it only in its own click path. So a pick in the Viewer left the previous layer's rows lit:
two layers appearing chosen at once, the exact ambiguity K-203 set out to remove. The panel
now listens to `selectedLayer` and drops that state wherever the choosing happened, through
the one helper its own click path uses. The Effect controls panel already followed the
shell; a test now pins that a Viewer-shaped selection change switches it, so it cannot
quietly stop. **(2) Copy and paste carry the document, not a summary.** A copied layer is
the model's own `Layer`, serialised — transform and keyframes, masks, paint, effects,
switches, markers, retime, and any field a newer Lumit added riding in `extra` — because a
paste that dropped a property would be found much later, on a shot that looked almost
right. The paste gives fresh layer and effect ids, and keeps a parent or track matte
reference **only when it still names a layer in the target comp**: pasting back where it
came from keeps both, pasting elsewhere keeps neither rather than leaving a dangling
pointer. Time: `at_frame` lands the in point there and moves in point, out point and
`start_offset` together (`edit_layer_span`'s `MoveIn`, the `[` key's own rule), so
keyframes and source frames travel with the layer; `None` keeps the copied time, which is
the owner's setting for rebuilding a moment in a second composition — **default at the
playhead**, the other behind a preference. Effects copy as the **same `.lumfx` document a
preset is**, so a copied effect can be saved as a preset and a preset pasted as an effect,
and they always paste with their **first keyframe at the playhead** whatever the layer
setting says: what is being placed is an animation, not a position. An effect with no
keyframes pastes unmoved — there is no timing to place. The panel wiring landed with it: a session clipboard on
`LumitUiState` (written through methods that notify, because Paste greys out while it is
empty and a menu that never hears about the copy stays greyed — which is how it behaved
before those methods existed), **Edit → Cut / Copy / Paste**, and the *Paste layers at
their original time* row in Settings → Interface. Still owed, and TODO'd: **Copy effect**
on an effect's heading in the Effect controls panel and on its Timeline row, which is
where an effect is picked rather than a layer.

**K-276 · DECIDED · The Viewer says how far a slow frame has got, and the Timeline can
be asked what each layer and effect cost.** Two halves of the same instrumentation
(docs/13 §7.1's first visible piece), both driven from one recorder in `lumit-render`
(`profile.rs`) that the headless renderer builds per frame and hands to the realise
walk. **(1) The preview progress bar** (docs/07 §2.5): the engine reports a stage and a
0..1 fraction as a frame passes through planning, decoding (per source job), building,
compositing (per top-level layer) and presenting; the bridge forwards each as
`WorkerResponse::RenderProgress` and always closes with `done`, so a frame that faults
or is served from the cache still ends its own bar. Reporting is turned on **per
request** — only for the frame a user is waiting on (a scrub, a playhead move, a
value drag), never for playback, the idle fill or a scope trace — and the frontend
shows nothing until a render has been outstanding for 150 ms, so ordinary frames stay
silent and only a genuine wait speaks. The fractions are fixed stage weights, not
measurements: a bar's job is "how much longer, roughly", and claiming more would be a
lie with a decimal point. **(2) The render-time indicators**: per-layer and per-effect
milliseconds, published as `WorkerResponse::FrameProfile`, shown in a new Timeline
column (`TimelineGroup.timings`) on each layer row and on each effect's heading in the
fold-out, and on the effect's title row in the Effect controls panel. Attribution is
carried, not inferred: `CompLayerDraw` gains the layer id and `fx_ids`, and
`fx::resolve_stack_temporal_named` returns each op beside the effect instance that
wrote it — a `Resolved` op has forgotten where it came from, and re-deriving it by
filtering the effect list misaligns the moment a stack holds a placeholder or an
orchestration-only effect. Two boundaries, both deliberate: only the top-level layers
of the composition being rendered are timed (a Precomp's number therefore includes
everything inside it — the layers inside are rows of another comp), and a layer's
number is its own picture (source, effects), because the final composite is one pass
over the whole stack rather than a per-layer act and so lands in the frame total.
**(3) Measuring is opt-in and it fences.** GPU work is submitted, not performed, so a
wall-clock span around a kernel call would time the paperwork; a measured node
therefore waits for the card before the clock is read. That is a true measurement and
it costs the processor/card overlap for the frame measured — so the Timeline column
carries a stopwatch switch, nothing is measured until it is pressed, playback is never
measured whatever it says, and turning it off drops the numbers rather than leaving
stale ones on screen. §7.1's "continuously, at negligible cost" wants GPU timestamp
queries and stays the recorded follow-up in TODO; what ships is honest about which of
the two it is. **(4) A measured frame is a composited frame.** Found on macOS the day
this landed: the column drew nothing, ever. Numbers exist only for a frame the engine
actually composites, and a frame the cache already holds is served without one — so on
any composition warm enough to be worth profiling (which is every composition a moment
after it opens, the idle fill seeing to that) every render was a cache hit and reported
nothing. While measuring, the whole ladder is therefore stepped over — the renderer's
own held textures, the RAM tier, the disk tier — and switching the column on asks for
the frame under the playhead again, so the numbers appear where the user is looking
rather than at the next place they happen to scrub to. Re-rendering held frames is the
cost of asking, which is what the switch is for. **(5) The effect heading's two reorder
arrows give up their place to the render time.** Moving an effect is a handful of acts
in a session and read-what-it-costs is continuous while a comp is being made faster, so
the arrows become a right-click menu on the heading — which can also send an effect
straight to the top or the bottom, and lists only the moves that effect can make — and
the heading itself **drags to reorder** (docs/07 §6's owed gesture, and the one every
other list in the application already uses: the name is what you take hold of, the
heading under the pointer lights up to say which place is being taken). **(6) A column
that is idle says so.** Reported the same day as (4), and the more instructive half of
it: the column drew *nothing* until measuring was switched on, so a header called Time
over a row per layer and nothing in any of them read exactly like a feature that did not
work — and the switch was a glyph in a header nobody had reason to press. An idle cell
now shows a dimmed dash, and **a click on any of them starts measuring**: the column is
its own switch, wherever the user reaches for it, and the header keeps its stopwatch for
switching back off. A discoverability bug is a bug; a feature nobody can find is not
shipped. **(7) The column reports its own state, and so does the engine.** The follow-up
report — "I see a dash but no values" — could have meant three different faults, and the
interface showed the same dash for all of them. So: the header carries the **whole frame's
cost** while measuring (an ellipsis until one has been measured), which separates "nothing
is coming back" from "something came back but not about this row"; a refusal from the
engine posts a notice instead of leaving a lit switch over an empty column; and the engine
prints **one line per switching on** and one more on the first frame it measures, so a
session's console answers "did the engine measure anything at all" without a debugger.
Diagnosing a report should not need the reporter to be a developer. **(8) The switch moves
to the bottom strip and starts ON.** The whole thread of reports above has one root: the
switch was a glyph inside a column header, and the owner — who had read the design and
asked for the feature — did not know the header was a button. That is the design being
wrong, not the reporting. So the clock moves to the **bottom strip, after the cache
meters**, where a session-wide, costs-something switch belongs and where it is seen without
being looked for; the column header becomes a plain readout; and both sides now *default to
measuring* (the engine's flag starts true, so no startup call is needed to agree). The cost
— a fence per node, and a measured frame composited rather than served from a cache — is
now paid by default, which is the owner's call, made knowing it: the toggle is one obvious
click away — and turning it off takes the **whole column** away (header, cells and width)
along with the figures on the Effect controls headings, rather than leaving a row of dashes:
a column of blanks is not a column, and the outline's width is worth more than an indicator
nobody has asked for. **(9) An effect's number shares its layer's column.** A `Flexible` label beside
a `Spacer` splits the free space between them rather than queueing, so the effect heading's
figure landed halfway across the row instead of in the column. One `Expanded` label and no
Spacer puts it exactly where the layer rows' numbers are, pinned by a test that compares
the two rectangles.

**K-278 · DECIDED · A trackpad scrolls the Timeline, and the two halves scroll exactly as
far as each other.** Two reports from the same Mac session, neither visible to anyone using
a mouse. **(1) The Timeline could not be scrolled by trackpad at all.** A two-finger scroll
on a Mac arrives as a pan *gesture* (`PointerPanZoom`), not as the wheel's pointer signal —
and the panel deliberately sets `dragDevices: const {}` so that a drag draws a keyframe
marquee instead of scrolling. That setting, correct for a mouse, also switched off the only
route a trackpad has. It now allows exactly `PointerDeviceKind.trackpad`: two fingers
scroll, a click-drag still draws the box (a click-drag is a pointer drag, not a pan-zoom).
The editing recognisers laid over those surfaces — the marquee, the bars, the graph's
handles — exclude the trackpad in turn (`dragDevices` in `widgets/controls.dart`), so they
cannot take the gesture back in the arena. **(2) The lane side could scroll further than
the outline.** The lanes carry a bottom bar (zoom, magnet, the horizontal scrollbar) that
the outline did not, so the lane rows had a shorter viewport, a larger `maxScrollExtent`,
and the two halves came apart at the bottom of a long stack — the halves are one table, and
a table whose rows disagree about where they are is not one. The outline reserves the same
height below its rows, which makes both viewports equal by construction. Both are pinned by
tests that drive a real trackpad gesture and compare both halves' scroll extents. **(7) The column reports its own state, and so does the engine.** The follow-up
report — "I see a dash but no values" — could have meant three different faults, and the
interface showed the same dash for all of them. So: the header carries the **whole frame's
cost** while measuring (`…` until one has been measured), which separates "nothing is
coming back" from "something came back but not about this row"; a refusal from the engine
posts a notice instead of leaving a lit switch over an empty column; and the engine prints
**one line per switching on** and one more on the first frame it measures, so a session's
console answers "did the engine measure anything at all" without a debugger. Diagnosing a
report should not need the reporter to be a developer.

**K-277 · DECIDED · The disk tier's write queue is bounded and de-duplicated, because a
write-behind queue nobody counts is a memory leak.** Reported from a Mac: the system ran
out of memory with Lumit holding 81 GB, while the editor sat idle. The idle backup
(docs/06 §5.5, K-215) copies held frames down to disk, and it decides what to copy by
asking the disk mirror "is this frame parked?" — a question that only turns true once the
*write has finished*. Parking is write-behind by design, so between handing a frame over
and the write landing, that frame looked to the backup exactly like one that had never
been offered. The loop wakes every couple of milliseconds: it read the same frames off the
card and handed them over again, and again, each copy a whole frame (8 MB at 1080p)
queued behind an IO thread that had to swizzle, compress and write — so the queue grew as
fast as the graphics card could read back, for as long as the editor was left alone. The
fix is two rules, in one place (`diskio::ParkQueue`, the only route to `Cmd::Store`): a
frame already on its way down is not offered again (`DiskIo::is_pending`, asked beside
`contains`), and at most **eight** frames may be waiting at once — past that a park is
refused, which costs that frame its place on disk and nothing else, since it is still on
the card and in memory and will be offered again. This is the docs/14 §5 decision the
unbounded channel needed and never had: `Cmd::Store` carries whole frames, so its depth
*is* a memory budget, and 64 MB is the ceiling now written down.

**K-279 · DECIDED · The website lives in this repository, not a separate one.**
`web/` is lumitlab.com and `web-docs/` is docs.lumitlab.com: two small Astro projects,
static output, deployed as two Cloudflare Pages projects pointed at those two root
directories (a real subdomain needs its own deployment target, hence two rather than
one). They sit outside the Cargo workspace and nothing depends on them.

Kept in-tree because the alternative has to synchronise three things across a repo
boundary for no present gain. **(1)** The download page reads the GitHub releases API of
*this* repository and links straight at the assets `release.yml` publishes on a `v*` tag,
so tagging updates the site with no deploy; split, the site would trail the pipeline that
feeds it. **(2)** The site uses the brand assets from `assets/brand/`, so a split means
copying the mark and letting it drift from the app icon — the exact failure this project
avoids elsewhere by keeping one source of truth. **(3)** This repo's standing rule is that
a doc changes in the same commit as the thing it describes; that is impossible across two
repositories, and release notes, install instructions and the roadmap all straddle the
seam.

Nothing is hosted by us: GitHub serves the release binaries from its own CDN with no
bandwidth cap, and Cloudflare Pages serves the two static sites. There is no server and
no scaling story to own. Revisit only if the site grows contributors who should not have
to clone a Rust + Flutter tree — at which point Cloudflare's per-path build filters
already prevent the two from triggering each other's builds.

**K-280 · DECIDED · Waveforms are mip-mapped, window-fetched, and stacked by frequency.**
Three things, one seam, because they are the same seam. **(1) The resolution follows the
zoom.** K-172's lane asked once for 2 048 buckets across the whole source and kept them for
the session, then stretched that one summary however far the Timeline was zoomed — so past
about ten seconds on screen the wave became a staircase of blocks, which is the opposite of
what zooming in is for. Now `lumit-audio::peaks::PeakPyramid` summarises a source at three
levels of detail in one pass (256 / 4 096 / 65 536 samples per block, the tiers docs/09 §4
always named), the bridge keeps one pyramid per **file path** for the session (bounded: four
entries, 64 MB, least-recently-asked evicted — two layers cut from one song decode it once),
and a lane asks for *the stretch it is showing* at one bucket per pixel column, again
whenever a zoom or a scroll moves that window. The request rounds itself off and pads half a
view either side, so scrolling a few pixels sends nothing.

**(2) Clips draw their own waveform.** A Sequence layer's clips were coloured boxes; docs/09
§4 has always said the clip waveform is "the primary visual for beat-checking an edit". Clip
peaks are bucketed in the clip's own **placed** time rather than in source time, because a
clip is the one thing on the timeline whose source clock is not a straight line — a ramp
plays its middle slowly and its end fast, and buckets taken evenly in source time would put
the transients in the wrong columns. The mapping is done in the engine, where the map lives.
Sliding a clip moves box and picture together with nothing refetched; trimming an edge
changes the mapping, so the peaks are asked for again when the trim commits (during the drag
the picture holds still, which reads as the content staying put while the window moves over
it — what a trim is).

**(3) Multiwave.** One wave says how *loud* a moment is and nothing about what is in it: a
mastered track is a solid block whether it is a kick, a snare or a vocal, and cutting to a
block means cutting by ear alone. So the same pass also splits the signal into bass (below
200 Hz), middle, and treble (above 2 kHz) with 24 dB/octave filters and summarises each; the
lane stacks the three, bass at the bottom. The kick shows in the bottom band, the hats in the
top, and a cut can be aimed at either. **On by default**, with Settings ▸ Interface ▸ Editing
▸ *Waveforms show the frequency stack* returning the single wave unchanged — the plain
picture stays a first-class choice, it is just no longer the only one. Prior art: BLICK's
multiwave, which is where the idea came from.

The waveform colours become their own theme grouping (`WaveformColours`: `rest` plus the
three bands) rather than the roles the lane was borrowing — docs/15 §6.4 has a standing
direction that each grouping splits out as its area is next touched, and §6.4 also says
waveforms are *content, not state*, which the old lane broke by drawing in `accent`.

Not done here, and still the design intent: writing the pyramid to the project sidecar keyed
by content hash, so a reopened project does not decode again ([TODO.md](TODO.md)).

**K-281 · DECIDED · `L` reveals a layer's Audio in the Timeline, and a panel shadowing an
app-wide chord is not a conflict.** `L` on the selected layers opens their **Audio** group,
`LL` opens the waveform lane inside it, `LLL` shuts them again — the same three-tap shape
`U` already has, and the reason is the same: the thing you want is usually one of three
depths, and a modifier for each is three chords to remember. A layer with no sound is left
alone rather than opened onto a group it does not have. `Shift+L` (K-172's *Reveal Volume*)
now reaches the same cycle, so the older habit still works.

`L` is also J/K/L shuttle transport (docs/07 §15), which was bound app-wide, and the keymap's
conflict detector treated *any* app-wide binding sharing a chord with a panel-scoped one as a
clash — so the shipped default could never give a panel a plain letter transport already
used. That rule is superseded: `Keymap::lookup` has always resolved the pair by a stated
precedence (the focused panel gets first refusal, app-wide is the fallback), so the chord runs
exactly one action and which one is never in doubt. `Keymap::shadows` reports those pairs
instead, because the app-wide meaning genuinely stops working in that one panel and somebody
reading their keymap should be able to see that. Two bindings in the **same** context remain a
conflict — nothing can tell those apart.

The cost is real and accepted: inside the Timeline, `L` no longer steps the playhead forward.
The Timeline is the panel where you reach for a layer's sound and the least likely place to be
shuttling; the arrows, `PageUp`/`PageDown` and `J`/`K` all still move time there, and `L`
keeps its transport meaning in every other panel.

**K-282 · DECIDED · Stepping a frame is `Mod`+arrow; the bare arrows belong to whatever has
focus.** `ArrowRight`/`ArrowLeft` were bound app-wide to next/previous frame. That is one key
each for the commonest transport move, which is why it was done — but the arrows are the two
keys *every* focused thing wants for moving within itself: a list moving its highlight, a
field moving its cursor, a canvas nudging a selection. An app-wide binding on them means none
of those can ever be given the key without taking the transport away, and a panel-scoped
binding that shadows it (K-281) would have to be added one panel at a time for ever. So the
step moves to `Mod+ArrowRight` / `Mod+ArrowLeft` — Ctrl on Windows and Linux, Cmd on macOS,
like every other `Mod` chord — and the bare arrows are unbound.

Nothing is lost: `Page Down` / `Page Up` still step a frame with nothing held, `Shift` with
them still steps ten, and `J`/`K`/`L` still shuttle (outside the Timeline, per K-281). This
also supersedes K-281's aside that "the arrows … still move time" in the Timeline: they no
longer move time anywhere without `Mod`.

**K-283 · DECIDED · Settings → Keymap says a shadow out loud, quietly.** K-281 stopped
reporting a panel-scoped binding that takes an app-wide chord as a conflict, which was right
— nothing is ambiguous — but reporting *nothing* would have been wrong: the app-wide meaning
really does stop working in that one panel, and finding that out by pressing the key is the
worst way to learn it. So `Keymap::shadows` is surfaced (`keymap_shadows` on the bridge) as a
plain muted line above the table — "`L` — Reveal Audio in the Timeline, shuttle forward
elsewhere" — with no border and no warning colour, because it is a fact about the keymap and
not something to go and fix. The bordered banner stays for real conflicts.

One consequence worth writing down: a **rebind can no longer make a conflict at all**. Within
one context the previous owner is evicted (K-200's one row, one chord), and across contexts
the pair is a shadow — so the banner is now only ever tripped by an imported keymap file
carrying a duplicate, which is where its regression test now goes.

**K-284 · DECIDED · Past the finest tier the samples answer, and the multiwave is drawn
through the wave rather than beside it.** Two corrections to K-280 from looking at it.

**(1) Fully zoomed in, a waveform should be a line.** K-280 fixed the stretched-summary
staircase but left a second one behind it: the finest tier is 256 samples a block, and the
Timeline zooms to 64×, so on a short comp a pixel column ends up covering about seven
samples — thirty-odd columns reading the same block, drawn as thirty-odd identical slabs. A
mip-map cannot fix this, because there is nothing finer in it. So a short source now keeps
its **mono mixdown** beside the pyramid (16-bit at the peak rate: 96 KB a second, half the
memory of float and three ten-thousandths of a pixel of difference), and any query finer than
one block per bucket is taken off it in one streaming pass — full band straight, the three
split bands filtered on the way with a `SAMPLE_PREROLL` run-up so the filters are settled by
the time the window starts. Below one sample per column, min and max meet and consecutive
columns join into a continuous trace, which is the picture every editor shows at full zoom.
**Short** is `SAMPLE_KEEP_SECONDS` (ten minutes): past that the 64× ceiling can never get a
column under one block, so a sample copy would be tens of megabytes held to answer a question
nobody can ask. The peak cache's budget rises to 96 MB to hold the copies, and it is a byte
budget rather than a count precisely because the count no longer says anything about the cost.

**(2) The stack goes through the wave, not beside it.** K-280 put the three bands in a third
of the lane each. In a 22 px row that is six pixels a band, which is not a waveform, it is a
smear — and it asks the reader to add three small pictures up in their head to get back the
one they were already reading. Drawn instead **over one another around one centre line**, dim
to bright as the frequency climbs, the bass fills a soft broad body and the treble lands as
bright thin spikes on it: one silhouette with its inside showing, which is what the reference
this came from actually looks like. The band colours become a brightness ramp rather than
three hues for the same reason — hue-coded, they read as three unrelated waveforms — and band
strokes are opaque, since three softened envelopes over one another blend into a wash and lose
the ranking. The **single wave is untouched**: same softened envelope, same solid RMS core.

**K-285 · DECIDED · Where a waveform sits is its own setting: centred, or standing on the
floor.** A waveform is symmetrical about silence, so a centred one spends half its row
drawing a mirror of the other half. In the Timeline's 22 px lane that is eleven pixels of
information and eleven pixels of restating it. Settings ▸ Interface ▸ Editing ▸ *Waveforms
rise from the bottom* folds it onto the baseline instead: each column reaches up by how far
the signal swung either way, whichever was further, over the whole row's height.

Kept as a **second, independent** switch rather than folded into the multiwave one, because
the two answer different questions — *what is in the sound* and *how the row is spent* — and
all four combinations are sensible. It is also purely a drawing decision: the peaks fetched
are identical either way, so `WaveformStyle.needsBands` is what reaches the engine and
flipping the baseline repaints without asking for anything.

Centred stays the default. It is what Lumit has always drawn, it is what the eye expects of a
*wave*, and defaults do not change under people for a preference.

**K-286 · DECIDED · Anti-aliasing defaults to eight samples, what the card can do is reported
rather than saved, and the project's own settings leave the Settings window.** K-274 settled
that anti-aliasing is a project property, on by default, with one value shared by preview and
export. Building it ([impl/anti-aliasing.md](impl/anti-aliasing.md)) left four smaller choices,
taken here.

**(1) The default is eight coverage samples.** K-274 said "on" without saying how much. Eight
smooths the shallow diagonals four still steps on, which is where a slow rotation's crawl is
most visible, and the cost is one multisample attachment beside the comp frame rather than more
shading — a memory cost, paid once per comp frame, not a per-pixel one. A card that will not
give eight falls back to four by the rule in (2), so the weaker machine lands on what would
have been the conservative default anyway. Off / 2 / 4 / 8 are the choices, because those are
the counts hardware actually implements — a free number would offer precision that does not
exist.

**(2) What the machine can draw is reported, never written back.** The count is asked of the
adapter and never assumed; a card that will not multisample the working format at the count
asked for gets the highest it will, down to off. The project keeps the value its author chose
and the Settings row states what is being used instead, beside it, in the calm voice
([15-DESIGN.md](15-DESIGN.md)) — a statement, never a warning. The alternative, quietly
lowering the stored setting, would mean opening a file on a weaker machine silently changed
the project for everyone who opened it afterwards. A machine's limit is not a project's error.

**(3) The count is part of a frame's name, and `ALGO_VERSION` goes to 3.** The setting changes
every pixel, so it joins the content hash a cached frame is filed under (docs/06 §5.2) — a
frame banked at one count must never be served at another. And because the default is *on*,
every frame banked before this was made without anti-aliasing, so the version bump retires all
of them by construction. Both reasons stand alone; either would have been enough.

**(4) A project's settings get their own window, and Settings stays machine-local.** The count
first landed as a **Rendering** page inside Settings, marked as the project's with a section
heading — which put a value that travels in the `.lum`, and undoes like an edit, in the window
whose every other value belongs to this machine and to no document. A caption was doing a
window's job. So **File ▸ Project settings…** (`Mod+Alt+Shift+K`, After Effects' own chord)
holds the project's answers, and [07-UI-SPEC.md](07-UI-SPEC.md) §15's "every value here is
machine-local" needs no narrowing for it after all. The disk cache's *Applies to* row (K-215)
stays in Settings → Performance: its whole purpose is choosing between the two scopes, so it is
the one control that has to stand with a foot in each. Colour management and export defaults
land in the new window when they are built, rather than back in Settings.

**K-287 · DECIDED · The bars that carry time hold still: fixed slots, the progress bar on
the transport, typed timecode, and a Retime that reads as a clock.** From the owner
(2026-08-06). Five changes, all of them the same complaint — the parts of the interface
that report *time* were moving while time passed, which is distracting exactly when the
picture is being watched.

- **The Viewer's preview progress bar moves onto the right-hand end of the transport**
  (docs/07 §2.5), instead of floating over the bottom of the picture. Over the picture it
  covered the composition while a frame was being waited for, which is when the
  composition is being looked at hardest. On the bar it has a place of its own: the
  controls take the space that is left over, so the bar arriving and leaving moves none of
  them.
- **Every part of the Viewer's bar whose text varies gets a fixed slot** (docs/07 §2.2),
  sized for the longest thing it can ever say, and a part that comes and goes — the
  degradation badge — keeps its slot while it is away.
- **The playback-mode button says the mode and nothing else**: "Adaptive res" or "Every
  frame". It used to carry the settled tier beside the name ("Adaptive · Half"), so it
  re-lettered itself as the engine felt its way up and down the ladder. Which tier a frame
  was made at is the degradation badge's job (docs/13 §4 still stands: silent degradation
  is a bug — the badge is what says it, and it says it only while there is something to
  say).
- **The Timeline's timecode and frame readouts get the same fixed slots, and both become
  click-to-type** (docs/07 §4.1). They sit left of the layer search, and a readout that
  resized itself as it counted shoved the search field sideways through every second of
  playback. Typing a time in either moves the playhead — the timecode in the format it
  already shows, the frame readout as a plain number with or without its `f` — and a time
  outside the composition is **clamped to the nearest end** rather than refused. The
  Viewer's own clock gains the same typing, which docs/07 §2.2 item 11 had always asked
  for.
- **The Retime row reads as `HH:MM:SS:FF`**, not as a number of seconds, realising K-075's
  value lens for the outline row (docs/04 §9.3). It is dragged and typed in whole source
  frames, at the composition's rate — the read model does not carry the footage's own rate
  yet, and every other time in the panel is counted in comp frames; when it does, this
  readout moves to the footage's timebase as K-075 asks, with no change to what is stored.
  Settings ▸ Interface ▸ Editing ▸ *Retime values in seconds* puts the decimal seconds
  field back, and is the only way to state a source position between two frames.

The shared widget is `TimeReadout` (`flutter_ui/lib/widgets/time_readout.dart`): a slot
measured in characters of the face it draws in, a click that turns it into a field holding
exactly what was shown, and an optional drag for the places that were a drag field before
they were a clock.

**K-288 · DECIDED · A layer-input parameter may name the layer the effect is on, and
that means "this effect's own input" — which on an adjustment layer is everything
below.** A layer reference (K-123, K-142) used to name only *another* layer: the picker
excluded the owner outright, on the reasonable-sounding ground that sampling yourself is
not defined. For a depth pass that is true enough. For the Lens flare's Matte source it
was simply wrong, in two ways at once. On an ordinary layer, "flare the lights in this
picture" is what asking for a matte source nearly always means, and the effect made you
go and find the layer you were already standing on. On an **adjustment layer** — which
has no picture of its own, and whose whole job is to act on the composite beneath it —
there was nothing correct to point at at all: whichever layer you picked, you were
detecting lights in the wrong image, and the effect that most wants to sit on an
adjustment layer was the one that could not.

So a reference to the owning layer resolves, everywhere, to **the effect's own input at
its point in the stack**. No second render happens — `run_ops` binds the texture it is
already carrying — which makes it cheaper than any other answer as well as the right
one, and makes it exactly aligned with the raster the effect writes (a separately
rendered layer is resampled to get there). On an adjustment layer that texture is the
composite of everything below, so the flare finds the lights in the footage beneath it
with no setup. The K-142 source combobox (None / Masks / Effects and masks) does not
apply to a this-layer reference: nothing is re-rendered, so there is nothing for it to
choose between.

A schema declares `ParamKind::Layer { self_default }`, and a `true` there means a fresh
instance **added to a layer** starts pointed at that layer. The Lens flare's Matte layer
takes it; DoF's Depth layer does not, because a depth pass is never the picture itself —
though it may still be pointed at this layer by hand, and reads the same input if it is.
Plain `instantiate` (presets, tests) leaves every reference unset, so the labelled no-op
stays the value a preset carries. The frame key feeds a distinct marker and stops
recursing: this layer's own content is already hashed by the walk the parameter is
inside, and an adjustment layer's below-composite by the other layers' entries, since
draw order is content.

**K-289 · DECIDED · The Lens flare's Background pair becomes a Blend menu, defaulting
to Add; Normal is the flare on black.** K-258 gave the flare a two-option Background
choice — Transparent (the layer's own alpha carries the flare) or Black (the output
forced opaque, so the flare could be exported as an element over black and Screened or
Added back in a compositor). That is a blend mode question wearing a disguise: both
options are answers to "how does this light combine with the picture", and only two of
the answers were available.

Everything the effect renders is a black-backed light **element** — a frame that is pure
black where there is no flare — so the honest control is the same menu a layer's Mode
dropdown offers, applied to that element over the layer beneath. It offers the curated
light-combine set **Echo** offers (K-149, T21) and omits the same modes for the same
reason: the HSL, burn and dodge modes are ill-defined on a premultiplied light overlay.
In code order: Normal, a divider, then Add (the default), Screen, Multiply, Overlay,
Soft light, Hard light, Lighten, Darken, Difference, Exclusion, Subtract, Divide. Every
mode runs per channel on all four channels in premultiplied linear light — this is light
being added to light, not a perceptual re-encode of a finished picture, which is also
what keeps the CPU reference and the WGSL kernel bit-exact (§1.6) without an sRGB round
trip.

Two modes carry the old behaviour. **Add** is `out = in + flare` with alpha saturating at
1 — bit-identical to every flare rendered before this menu existed, which is why it is
the default and why a project that never touched Background renders the same pixels.
**Normal** ignores the layer and returns the element on its opaque black background:
that is precisely the flare-over-black that Background = Black existed to export, so a
project saved with Black migrates to Normal. The migration runs in
`backfill_builtin_params` and drops the dead `background` parameter, because the schema
no longer declares it and the panel cannot draw a row `set_value` refuses. The neutral
passthroughs (Intensity 0, Mix 0) return before any of this, so they stay bit-exact
whatever the menu holds.

**K-290 · DECIDED · A frame is one command buffer, and a measured frame is not.** Every pass
in `lumit-gpu` used to build its own command buffer and submit it, so a frame cost the graphics
driver one round trip per layer and per effect — measured 2026-07-31 at `layers + 2`
submissions, 34 at thirty-two layers. All of a frame's passes are already in order on one
queue, so they are now encoded once and handed over once: 3 submissions, and flat in the layer
count. This takes [impl/playback-scheduler.md](impl/playback-scheduler.md) §2's
one-GPU-submit-thread rule further rather than conflicting with it — that rule says *who* may
submit, this says *how often*.

**Batching is a property of the context, not of a threaded parameter.** `GpuContext` holds the
frame's encoder between `begin_frame` and `end_frame`, and `encoder()` hands it out; outside a
batch it hands out a fresh, self-submitting one, so every pass called on its own behaves
exactly as before and no test changed. The alternative — threading `&mut CommandEncoder`
through the realise walk — would have rewritten every signature in the crate and every call
site in the walk, for a walk that recurses through nested comps, adjustment staging and one
whole render per motion-blur sample. `begin_frame` nests instead, so the recursive entry point
opens the batch and the outermost caller closes it.

**Anything that observes the GPU flushes first.** A command that has not been submitted has not
run, so the read-backs, the scope trace and the three shared-texture present paths hand the
batch over before their own submission and wait. These keep their own command buffers
deliberately: each is followed by a fence, and a fence is the one thing batching cannot defer.

**A measured frame gives the batching up, and that is the right trade.** The render-time
column fences on the device at each layer and each effect; under batching that fence would
wait on a queue nothing had been handed to, and every number would silently become the time
Lumit takes to *describe* a layer rather than the time the card takes to draw it. So measuring
flushes as it goes. K-276 already established that measuring costs the overlap between
processor and card, which is why it is opt-in and never runs during playback; this is that same
cost, not a new one.

**The gate is a count, not a stopwatch.** `GpuContext::submits_so_far` counts every submission
through the one choke point, and the regression tests assert the shape: an unmeasured frame's
count does not grow with its layers, and a measured frame's does. A submit is a round trip
whose cost does not depend on the card, so the count is the honest measure — and unlike a
timing it means something on CI's software rasteriser. A fixed budget was deliberately not
pinned; "adding thirty-one layers adds no submissions" is the property that was lost, and it is
the one worth holding. **The wall-clock win is still unmeasured on real hardware**: the number
that motivated this was a submission count, and what it buys in milliseconds wants a run on a
real card either side.

**K-291 · DECIDED · The lock is enforced in the engine, and it protects the work rather than
the housekeeping.** The Timeline guarded the *gestures* a locked layer offers — its bar, the
razor, rename, reorder, delete — while the fold-out's transform, effect and volume rows went on
editing it. So the switch did not mean what its own tooltip says ("Locked — no edits until
unlocked"), and the backlog carried the open question: guard the rows, or enforce in the engine?

**Enforce in the engine.** One guard at the top of `apply` covers every op, every caller, and
every op yet to be written. A guard per row has to be remembered each time a row is added, and
forgetting one is precisely how this hole opened — the rows that leaked are the three *newest*
families of row. The refusal is `OpError::LayerLocked`, which crosses the bridge as an ordinary
op error.

**And guard the rows anyway, for the interface's sake.** A locked layer's property rows are now
shown but not touchable: the numbers are still the document's and the curves still draw, but
nothing on the row takes a pointer. That is not belt-and-braces for its own sake — without it
the interface would go on offering a gesture the engine would only refuse, which is a worse
answer than not offering it. *Group* rows stay live: twirling one open is navigation, not
editing, and a locked layer you could not look inside would be worse than one you can.

**Lock protects the work, not the housekeeping.** A locked layer refuses every edit to what it
*is* — transform, effects, masks, paint, art, text, clips, markers, blend, matte, parent,
retime, volume, its switches, its span, its place in the stack, its existence. It still accepts
three: the **lock itself** (or it could never be undone), **shy** (a filter on the Timeline's
list, changing no pixel and no timing) and the **label** colour. That line is drawn where it is
because "locked means the composition does not change" is a sentence a user can hold, and
neither of the other two changes the composition. If it turns out to be the wrong line, it is
the reversible half of this decision — the guard's shape does not depend on it.

**Undo still crosses a lock, which is what makes the guard safe to put in the applier.** An
edit can only have been made while the layer was unlocked, so the journal always holds the
unlock *after* the edit, and walking backwards meets the unlock first. A `Batch` is guarded by
its members — each passes through `apply` on its way in, and a refusal rolls the whole batch
back, so a batch stays all or nothing. Both are pinned by tests.

**K-292 · DECIDED · Snapping reaches in pixels, reports what caught it, and lets `Ctrl`
past.** [07-UI-SPEC.md](07-UI-SPEC.md) §4.5 has always asked for snapping across edit points,
layer in/out points, keyframes, markers, beat markers, the playhead and the work area edges.
K-190 shipped the magnet covering exactly one of them — a whole frame — and the rest waited.
They are built now, for the lane key drag.

**The reach is measured in screen pixels, not in time**, which is the spec's rule and the one
that makes a single slop feel right everywhere. Zoomed out, a hundred frames may be ten pixels
apart and snapping should be eager; zoomed in, one frame may be fifty pixels and it must not
reach across three of them. Eight pixels is the distance: a little under half a row, close
enough that landing on a marker takes no aim and far enough that the frame either side stays
reachable at any useful zoom. Magnification is therefore the precision control, and there is no
second setting for it.

**What caught the drag is part of the answer, not a side effect.** `snapFrame` returns the
target as well as the frame, because the spec requires the capture to be *indicated* — a key
that jumps to a place the pointer was not reads as a fault unless something says why. The lane
draws a line at what took it, for as long as it holds it.

**A whole frame is the fallback, not a target.** With nothing in reach the drag rounds, exactly
as K-190 made it. That keeps the magnet's original meaning intact for the common case — an
empty comp has nothing to snap to and behaves precisely as before — and it is why the
whole-frame landing reports *no* caught target and so draws no indicator: it is not news.

**A lane's own keys are excluded.** A key that could snap to itself would be pinned where it
started, which reads as a broken drag rather than as a snap. A neighbour already on the same
frame goes with it, since being taken to where you already are is not a service either.

**`Ctrl` held suspends it, rather than a second toggle.** It is wanted for a moment inside a
gesture, not for a session; the magnet in the bottom bar remains the session-length switch.

**Beat markers are markers.** Beat detection writes ordinary markers, so beat snapping — the
beat-sync covenant's daily face — arrives by being marker snapping rather than by being a
separate kind with a separate list.

**The razor reads the same function, and that fixed a disagreement nobody had written down**
(owner, 2026-08-06). A cut was always quantised — `TimelineAxis.frameAt` rounds — but the line
drawn under the blade followed the pointer continuously, so the mark stood up to half a frame
from where the edge actually bit. One function now answers for both, so they cannot part. A cut
is a clip boundary and therefore a whole frame, so the razor rounds *after* snapping: a target
that sits between frames still takes the cut, and the cut still lands on a frame.

**The layer bar drag, the work-area handles and marker drags still land where the pointer puts
them.** That is a deliberate stopping point rather
than an oversight: the arithmetic is pure and shared (`panels/timeline_snap.dart`, tested on
its own), so each remaining gesture is a wiring job with no design left in it, and doing them
one at a time keeps each one's regression test honest. TODO carries the list.

**K-293 · DECIDED · Zoom flies, and the Timeline's zoom is a slider whose ends mean
something.** From the owner (2026-08-06), in three parts: the zoom should move rather than
cut, faster input should zoom further and settle when the hand stops, and the bottom bar's
− / + / Fit buttons should be a slider.

**The wheel still never zooms without a modifier.** This was briefly built the other way, on a
reading of the owner's first message that they corrected the same day: docs/07 §4.6's "no
scroll hijack" MUST stands, plain wheel scrolls, `Ctrl+wheel` zooms. Recorded because the
supersede was written and then withdrawn, and a reader finding half of it in the history should
know it never applied.

**The motion is the Viewer's, lifted out.** `widgets/smooth_zoom.dart` is K-218's shape shared:
the Viewer has flown since then while the Timeline, the graph editor and the Project panel all
cut. It interpolates **geometrically**, because magnification is a *ratio* — lerp 1 → 16
linearly and half the flight is spent between 8 and 16, which reads as a lurch then a crawl.
The Timeline reads it now; the graph editor and Project panel are named in TODO and are a
matter of holding one and reading its value.

**A fast roll goes further, with a ceiling.** A notch is worth more the sooner it follows the
last — linear in the gap, which is the thing the hand controls directly — up to 4×. The ceiling
is not a detail: without one a flick crosses the whole zoom range in a single gesture and there
is no way back to where you were. A notch arriving mid-flight extends the *target* rather than
restarting from wherever the flight had reached, which is what makes a rolled wheel one
continuous motion instead of hops that never arrive. When the hand stops, the flight finishes
and settles.

**The anchor is held for the whole flight, not just its ends.** The frame under the pointer
stays under it on every tick, because the lanes grow all the way through — hold the scroll
offset still instead and the anchor slides out from under the cursor, which is the drift the
Viewer's own note warns about. The correction runs in the same turn as the rebuild: deferring
it to a post-frame callback paints one whole frame at the new width with the old offset, a
visible sideways slide.

**The slider's ends are a promise, and one of them is a count of frames.** Left is the whole
composition. Right is **twenty frames across the lanes** — not a magnification like "6400%",
because a magnification means nothing without knowing the comp's length, while "twenty frames"
means the same thing on a five-second comp and a ten-minute one. The visible span is
`frames / zoom` whatever the panel's width, so the ceiling is simply `frames / 20`, and it
moves with the composition. The slider runs on the **logarithm** of the zoom for the same
reason the flight does: linear, nine tenths of its length would sit inside the last handful of
frames of a long comp and every useful zoom would be crushed into the first centimetre.

**Two zooms, two anchors, and that is deliberate.** `Ctrl+wheel` holds the frame **under the
pointer**, because there the pointer is the whole gesture. The slider has no pointer, so it
holds the **playhead** — corrected by the owner the same day from the middle of the visible
lanes, which was their own first suggestion and which they withdrew: the middle of the
scrollbar is a place nobody is looking at, while the playhead is where the work is happening,
and it is what After Effects zooms its own timeline about. In view, the playhead keeps exactly
the screen position it has, so nothing under the eye moves at all; out of view, it is brought
to the middle of the lanes, because magnifying about something you cannot see leaves you
nowhere.

**A dragged slider does not fly.** The flight is for input that arrives in *steps* — a wheel
notch, a tap on the track — where the gap between two zooms has to be filled. A drag is
already the motion, and animating towards a target the finger moves every few milliseconds
meant the lanes trailed the handle by a whole flight, restarting before they ever arrived:
reported by the owner as the slider being "super super laggy". So a drag sets the zoom at
once, and the handle is drawn from where the zoom is *going* rather than from where a flight
has reached, which is what keeps it under the finger. `HouseSlider` gained `onChangeLive` for
this; a tap on its track still flies.

**Zoom rebuilds the lanes, not the panel.** The other half of that lag: the zoom was a plain
field and every tick called `setState`, so a flight rebuilt the outline's every row, its
toolbar and its column header sixty times a second — along with the work-area read, the fold
tables and the cache-bar read that come with a full rebuild. Nothing left of the seam depends
on the zoom. The zoom is a `Listenable` and only the lane side listens to it, which is a
standing shape for this panel rather than a patch: the playhead is already handled this way,
and for the same reason. docs/13's S1 budgets a Timeline scroll/zoom frame at 8 ms, and the
bridge-call budget suite is what holds this: a zoom drag now has its own entry there, and
what it asserts is that the count does not scale with the number of steps dragged. Three things found in the same pass and fixed with it — the cache bar
was asking the engine for the whole composition's cache map on every rebuild despite its own
note saying it never polls (it now holds one read until a frame arrives), the merged
"something changed" listenable was allocated fresh per build so every cache bar
resubscribed, and the row-divider painter compared its blanks by identity against a list
rebuilt each time, so it always repainted.

**The scroll correction belongs inside layout.** Holding the anchor meant moving the scroll
offset the moment the zoom changed — and that offset is only valid for the *new* lane width,
which has not been laid out yet. For the rest of that frame the position sat past its own end,
so Flutter began springing it back, and the bottom bar's thumb was drawn from a position and a
length that disagreed: a thumb that twitched all the way through a drag (owner, 2026-08-06,
"jumps around a bit"). A scroll position is told its new content size during layout, in
`applyContentDimensions`, which is the one moment the width and the offset are known together —
and that method is *documented* to return false when it has moved the offset, so layout runs
again with the corrected one. `widgets/zoom_anchored_scroll.dart` is a `ScrollController` that
does exactly that, and the anchor it holds is **one-shot**: an anchor that outlived its zoom
would be applied by the next unrelated layout — a window resize — and drag the view back to a
zoom the reader had since scrolled away from. A zoom still in flight simply asks again on its
next tick, which it does anyway, because every tick is a new width.

**The slider's ends are landscapes, drawn rather than looked up.** A small one and a large one,
the pair After Effects flanks its own zoom slider with, replacing two sizes of magnifying
glass. Two reasons, both mattering: at the sizes that make the pair read as "less / more" the
small end is well under 16px, and K-209's floor exists because an Iconoir glyph's 1.5-unit
stroke lands on less than a pixel there and crunches — which is exactly what the 13px
magnifier did. A filled silhouette has no stroke to lose. docs/15 §5 already allows a
deliberately painter-drawn glyph, and this is one.

The − / + / Fit buttons are gone: the slider's two ends *are* Fit and full zoom, and a slider
also says where you are between them, which three buttons never did. `HouseSlider` gained a
width and a value-hiding option rather than a second slider being written for a toolbar.

**K-294 · DECIDED · Memory is reported, not guessed at: every tier's bytes beside the
process's own, and the difference named.** From the owner (2026-08-06), after a second
report of Lumit holding tens of gigabytes on a Mac — 85 GB, following the 81 GB that
K-277 bounded the write-behind queue for.

The first question either time was the same, and neither time could it be answered from
outside the process: **is a cache doing exactly what it was told, or is something holding
memory nobody is counting?** Every tier already knew its own bytes and every one of them
is byte-budgeted; what was missing was the total to weigh them against. So Settings ▸
Performance opens with a **Memory** section: what the operating system says the process
holds, what the frame cache and the decoded-frame cache hold, how many decoders are open,
how deep the write-behind queue is, and **what is left over**.

The left-over figure is the point. If the tiers sit at their budgets and the process is a
hundred times larger, the search is not in this list at all — it is memory held below us
(graphics allocations the driver has not reclaimed, a decoder's own buffers) and that is a
different hunt with different tools. Turning a week of guessing into one screenshot is
worth a syscall.

- `resident_memory_bytes` asks each platform for its nearest equivalent of what the task
  manager shows: `WorkingSetSize` on Windows, `VmRSS` on Linux, and **`phys_footprint`
  from `TASK_VM_INFO`** on macOS. Resident size is the obvious macOS answer and the wrong
  one — it leaves out the compressed pages and the IOSurface and Metal allocations a
  graphics application lives on, which is most of what would be hunted. `phys_footprint`
  is what Activity Monitor prints under *Memory*, and so the number a user reads back.
- **The graphics driver's own accounting rides beside the tiers.** The first reading in
  anger made the case: 12 GB held, 11 GB of it unaccounted, with ~405 frames decoded —
  which cleared every byte-budgeted tier at a glance and left the layer underneath, where
  the tiers' own numbers cannot reach.
  - It was first written as bytes alone (`Device::generate_allocator_report`), and on the
    Mac it was written for it read **"not reported by this driver"**: that report is
    Vulkan and D3D12 only, and Metal does its own allocation. An instrument that works
    only where there is no problem is not an instrument, so the report now leads with
    **live object counts** — how many textures and buffers the driver is holding — which
    every backend keeps. A handful at rest against thousands is exactly the difference
    between a cache doing its job and frames the engine dropped never being destroyed.
  - The byte figures stay for the platforms that have them, and that row is **not drawn**
    where they are zero: a zero nobody can distinguish from a real answer is worse than a
    missing row (docs/15-DESIGN.md — the honest gap).
  - The counts are pinned by a test that makes a texture, drops it, and checks the tally
    follows: compiled without wgpu's `counters` feature they would read zero for ever,
    which is the failure this row could not afford.
- **VRAM is reported apart, never subtracted**: on unified memory it is inside the process
  and on a discrete card it is not, so folding it in is wrong on half the machines Lumit
  runs on. **Nothing is counted twice** — a frame in the write-behind queue shares its
  allocation with the frame cache, so the queue reports a count. **What cannot be weighed
  is counted** — an open decoder's buffers belong to FFmpeg and the driver, so the report
  says how many are open rather than inventing bytes.
- A platform that cannot answer returns 0 and the interface says "not known here". The
  honest gap, per docs/15-DESIGN.md, beats a plausible number.

This is a diagnostic, and it is deliberately not a fix: it does not reclaim a byte. It is
the instrument the next report is read with, written down in docs/13 §7.0.1 as a standing
rule — a tier that holds memory and does not report it is not finished.

**K-295 · DECIDED · What the engine drops, the driver hands back on the next turn: the
worker reclaims once a loop.** From the owner's readings on 2026-08-06, which caught the
fault in the act: 6 GB held with **around 5 500 live graphics buffers**, then 2.9 GB and
**8 buffers** moments later — because switching back to a settings page happened to make
the device do a maintain. Memory that comes back only when the user does something
unrelated is a leak in every sense that matters to the person whose machine it is.

**The mechanism.** Dropping a texture or a buffer does not free it. wgpu marks it
destroyed and hands the memory back on the device's next *maintain*, which a renderer
drawing into a window gets for free from presenting. This engine renders into caches, on a
worker thread, and spends most of its time idle: the frame cache evicts, read-backs
finish, a composite's intermediates go out of scope — and none of that asked the device
for anything, so the memory sat marked-and-not-returned until something else polled for
its own reasons. The idle fill and the idle backup make that worse rather than better,
because they are what produces the dropped objects while nothing presents.

**The fix is one line and a rule.** `GpuContext::reclaim` — a non-blocking
`Maintain::Poll` — on every turn of the worker's loop. It drains what has already
finished, costs nothing when there is nothing to drain, and makes reclamation a property
of time passing rather than of the user opening a panel.

**The rule this writes down** (docs/13 §7.0.2): an engine that renders without presenting
MUST maintain its device on a schedule of its own. Anything that only frees memory as a
side effect of an unrelated call is not freeing memory.

Two things fell out of the same readings and are fixed with it:

- **Frames on the card are counted against the process where the card's memory *is* the
  process's memory.** K-294 reported VRAM apart from every tier on the grounds that a
  discrete card's frames are not in the process — true, but on the Apple Silicon Mac doing
  the reporting they are, so a cache doing exactly its job showed up inside the
  unaccounted figure and looked like the fault. The adapter now says which kind of memory
  it has (`unified_memory`, integrated or software), and the report counts accordingly.
  The rule generalises: a report that can mislead in the direction of "this is the bug" is
  worse than one that says less.
- **The memory section is a debug-build instrument** (owner, 2026-08-06). It is for
  hunting a fault, not a setting anybody should be asked to interpret; `kDebugMode` gates
  both the section and the call behind it, so a release build neither draws it nor asks.

**Verified** by `what_the_engine_drops_the_driver_gets_back`, which renders far more
frames than the cache can hold and then asks the driver what it still has: tens, not one
per frame. It runs on every platform the suite runs on, and the one that matters is
**macOS** — where the reclamation went wrong, and where no allocator report exists to see
it, which is why the gate is a count of live objects rather than a number of bytes.

**K-296 · DECIDED · Updates are checked from the Help row itself, fetched whole from the
GitHub Release, and installed on a restart the user chooses.** Help ▸ Check for updates was
the last row of the bar that was listed and dead. It is now the whole update sequence in one
row, and nothing else is added to the interface for it.

**The row is the state machine.** Press it and it greys and reads "Checking for updates…";
a moment later it is either "Click to update - v0.2.0" — the version in the row, where
somebody deciding whether to update is already looking — or back to "Check for updates",
with "Lumit is up to date" in the status line. Press it in the offered state and the update
is fetched; while it comes the row reads "Downloading update… 42%"; once it is on disk and
verified it reads "Restart to finish updating" until the restart happens. A row that said
"You are up to date" and stayed that way would be a stale claim by the next morning, which
is why success goes back to the resting wording rather than boasting about it. This needed
one small thing of the menu bar: `MenuEntry.live` is a row that rebuilds in place while its
menu is open and does *not* close the menu when pressed, because the point of pressing it is
to watch what it does. It is the only live row there is, and rows should have to earn it.

**Full installers, never patches.** Releases already publish the finished installer per
platform (`release.yml`, a `v*` tag): `setup.exe`, `.dmg`, `.tar.gz`, `.flatpak`. The updater
downloads whichever suits the machine, entire. A delta scheme means publishing a patch per
pair of versions, a tool to apply them and a fallback for the pairs that are missing — three
new failure modes to save bandwidth GitHub serves for nothing (K-279: no bandwidth cap, no
server of ours). The saving is real and the cost is a few hundred megabytes on a deliberate
click, a few times a year.

**Nothing is downloaded without being asked, and nothing is run without being checked.**
Automatic updates are on by default and are offered twice — on the setup screen (K-246) and
in Settings ▸ General ▸ Updates — but what "on" means is *looking*, once a day at launch, and
saying so in the menu. The download always waits for a press: this is a video application and
someone editing on a hotel connection should not find Lumit spending their data. Before the
downloaded file is executed it must match the length the release published and, where GitHub
publishes a `digest`, its SHA-256; a file that fails either is deleted rather than run. An
installer is the most dangerous file Lumit ever touches, so verification is a gate and not a
diagnostic.

**Finishing means restarting, and the work comes first.** An installer cannot replace files
that are running, so the last window says so: *Restart to finish updating*. With unsaved work
open the buttons are **Save and restart** / **Restart without saving** / **Later**; with a
clean project, **Restart now** / **Later**. Later keeps the verified installer and the row
that offers it, so the update is not lost by declining it once. Windows starts Inno Setup
silently (`/SILENT /CLOSEAPPLICATIONS /NORESTART` — the install questions were answered the
first time and asking them again is ceremony) and quits; macOS opens the disk image and
quits; **Linux only reveals the download** and does not quit, because a tarball is unpacked
wherever its owner keeps it and a Flatpak is installed by Flatpak, neither of which Lumit
should do on somebody's behalf.

**It lives in Dart, not in the engine.** `state/updates.dart` and
`shell/update_dialog_frb.dart`, with `dart:io`'s own HTTP client and `crypto` for the digest
— no new Rust dependency, no TLS stack pulled into a crate that renders frames. An updater is
shell business by every test docs/05 applies: it touches no document, no timeline and no GPU,
and the engine crates stay free of the network. The version it compares against is the one
the boot log already reports (K-008), so there is no second source of truth to keep in step.

**K-297 · DECIDED · Lumit installs per user and replaces itself, the way Chrome and VS
Code do — except inside a Flatpak, where that is Flatpak's job.** K-296 shipped updating by
running the installer again. That works and it is a poor experience: a wizard, a UAC prompt,
and questions the user answered the first time. The reason it had to be that way was the
install location, not the updater.

**The install moves to the user's own folder.** `packaging/windows/lumit.iss` gains
`PrivilegesRequired=lowest` and installs to `{localappdata}\Programs\Lumit`
(`UsePreviousAppDir=yes`, so an existing `Program Files` copy stays where it is and keeps
being updated by installer). This is what Chrome, VS Code and Discord all do, and it is the
whole trick: a folder the user owns can be rewritten by a program the user is running, with
no elevation and nothing to approve. macOS bundles and the Linux tarball already live
somewhere their owner can write.

**Releases carry the application, not only its installer.** `release.yml` now publishes
`lumit-<v>-windows-x64.zip` beside the setup, and `lumit-<v>-macos-<arch>.zip` beside the
disk image; the Linux tarball already was one. macOS uses `ditto` rather than `zip` because
an `.app` carries symlinks, executable bits and a signature that a naive archiver drops,
producing a bundle the system will not open. Unpacking uses the platform's own tool for the
same reason — `ditto` on macOS, `tar` elsewhere, including Windows, which has carried bsdtar
since Windows 10 1803. No Dart zip library, no new dependency.

**The swap is two renames, not a few hundred file copies.** The new version is unpacked to
`<install>.new`, verified, and marked complete; then `<install>` becomes `<install>.old` and
`<install>.new` becomes `<install>`. Renaming is one filesystem operation — it happened or
it did not — where copying files over a running application is hundreds of chances to be
interrupted into a Lumit that is neither version and may not start. If the second rename
fails the first is undone immediately, from code already in memory. The old folder is left
behind on purpose: Windows will not delete a loaded DLL, so `main()` sweeps it on the next
launch, and puts it back if it ever finds the install folder missing.

**Three deliveries, chosen by where Lumit actually lives** (`InstallSite.detect`):
*in place* for a folder or bundle Lumit can write beside — proven by writing a probe file,
not assumed from the path; *installer* for anywhere it cannot, which covers every existing
`Program Files` install and the macOS disk image; *Flatpak bundle* inside a Flatpak. The
release attachment follows the delivery, so a Flatpak is never offered a tarball it cannot
use, and a per-user install is never offered a setup it does not need.

**A Flatpak is not updated from inside, and pretending otherwise would be a lie.** The
sandbox is read-only by design and reaching the host to run `flatpak install` would need
permissions no editor should hold. So Lumit fetches the bundle, says the one command that
installs it, and stays open. Making that a proper `flatpak update` needs Lumit published to
an OSTree remote rather than as a single-file bundle — tracked in TODO, and the reason the
Flatpak wording names a command rather than a button.

**What this costs.** The window between the two renames is not recoverable by Lumit if the
machine loses power inside it: the install folder would be `Lumit.old` and nothing would
start. It is two rename calls wide, the start-up sweep puts it back for every failure short
of that, and the fallback is the installer, which is still published. Judged worth it
against a UAC prompt on every update for ever.

**K-298 · DECIDED · A theme is a file you can send somebody, and a theme you like is one
you can copy.** From the owner (2026-08-07). K-202 made every colour editable and left the
result trapped: a custom theme lived in the workspace file, which is machine-local, so a
theme could not be posted, put in a repo, or carried to a second machine — and the only way
to try a change without losing what you had was to save over it and undo by hand. The keymap
has had a shareable file since K-199; a theme is the other thing in Lumit worth sharing.

**The file.** `.lumtheme`, a small indented JSON document: a `format` marker, a `version`,
and then exactly `CustomTheme.toJson` — name, light-or-dark base, and the colours as
`#rrggbb`. The same shape the workspace file already stores, so the two forms cannot drift,
and readable for the same reason the workspace one is: a theme is a thing people tinker
with. **Export** writes the theme on screen, offered from a built-in scheme as well as from
one of the user's own, because "the stock dark with my accent" is a perfectly good thing to
send somebody. **Import** reads one and selects it.

**Reading is forgiving one way and strict the other.** A file from a newer Lumit opens, with
the colours this build knows and the rest taken from its base — that forward tolerance is
the whole reason K-202 stored a theme *over* a base rather than as a copy of the struct, and
this is where it earns its keep. A theme with no marker opens too, since a theme lifted
straight out of a workspace file has the same fields and refusing it would be pedantry. What
is refused is refused with a sentence under the buttons rather than an exception: picking
the wrong file is a normal thing to do.

**An import never overwrites one of the user's own.** A name is the identity of a theme —
the picker shows it and the workspace stores the selection by it — so every route that adds
one (import, duplicate, save a copy, rename) goes through `Workspace.availableThemeName`,
which numbers a clash rather than silently replacing somebody's work. The settings page says
so when it happens.

**Duplicate, rename, delete, import, export** sit together under the theme rows as a wrapped
row of buttons rather than one settings row each: they are five verbs about the same thing,
and five rows saying *Rename* would be a list of buttons pretending to be settings.
**Duplicate works from a built-in scheme too** — it is how a built-in becomes editable
without the editor having to ask for a name first — while **Rename and Delete are offered
only for the user's own**, because a built-in's name is Lumit's and two people describing
different Darks helps nobody. The editor gains **Save a copy…** beside Save, which is the
same branch made from inside the colours.

**The picker shows what it is offering.** Eight swatches beside the dropdown — the three
grounds, the text on them, the accent, and success/warning/error — so a theme can be
recognised before it is applied. Not every token: thirty-odd swatches is a colour chart, not
a preview.

**A new file type gets a file type's furniture.** `.lumtheme` joins `.lum` and `.lumfx`
everywhere K-251 and K-252 put those two: a fifth brand SVG (`assets/brand/lumit-theme.svg`)
rendered to `.ico` and `.icns` by `scripts/gen-icons.py`, a Windows registry association with
its document icon, a freedesktop MIME type with a scalable icon installed by
`packaging/linux/install.sh`, and a document type plus exported UTI in the macOS Info.plist.
The artwork keeps the family's page and folded corner and swaps the keyframe mark for three
overlapping swatches in the two key gradients and the core white, because what this file
carries is colours — legible at 16 pixels, where the kicker is a smudge. Like `.lumfx` it
registers **no open verb**: a theme is taken in from Settings, not opened as a document, and
an icon that promises double-click would be a lie. Documented as §6 of
`docs/10-FILE-FORMAT.md`.

**K-299 · DECIDED · An effect is copied from its heading, in both places it has one.**
K-275 built copy and paste and named what it left: "the two places an effect is *picked*:
**Copy effect** on an effect's heading in the Effect controls panel and on its row in the
Timeline, both calling `copy_effects(Some(id))`". Both are wired now.

Nothing new crosses the bridge. `copy_effects(Some(id))` has taken one effect since K-275, and
the in-app clipboard has held `.lumfx` text since then too — what was missing was any way to
*name* a single effect from the interface, so the call had no caller and the Edit menu's Copy
took the whole layer.

**One effect and a whole stack land on the same clipboard**, because both are the same
`.lumfx` document. Paste therefore needs no idea which it holds, and pasting one effect onto a
bare layer adds exactly one — which is the test.

**Only an effect's heading offers it.** The Timeline's fold-out draws Transform, Effects,
Masks, Contents, Paint and Audio as headings too, and none of them is a thing that can be
copied. `effectIdOfPath` already told the render-time indicator which rows are effects
(docs/13 §7.1); it now tells the menu the same thing, so a grouping opens no menu at all
rather than opening one with a dead row in it (docs/15: no punishment UI).

This entry was written as K-287 on its branch; that number went to the Viewer and Timeline
bar layout on main first, so it is K-299 here.

**K-300 · DECIDED · An effect is a thing you select, and Copy takes whatever is selected.**
K-299 put **Copy effect** on an effect's heading in both places it has one, and the first
person to use it found the hole around it: the heading could be right-clicked but not
*clicked*, `Ctrl+C` on a selected layer did nothing at all, and an effect name in the Effect
controls panel answered no click, with or without Shift. Copy worked from a menu row and
nowhere else.

**Three faults, one shape.** There was no selection an effect could be part of; there was no
`edit.copy` chord in the keymap and no case for it in the shell's handler; and the two places
an effect is drawn each had their own idea of what was picked, which was nothing.

**One effect selection, held by the shell.** `LumitUiState.selectedEffects` holds instance
ids in **stack order** with the layer they are on, and both places write to it and read from
it: an effect picked in the Timeline is lit in the Effect controls panel and the other way
round. It follows the same three click rules as every other list here — plain replaces, Ctrl
toggles, Shift extends the run — and picking a different layer clears it, because an effect
belongs to a layer and Copy must never act on something no longer on screen.

**A heading picks; the twirl folds.** In the **Effect controls panel** the name only picks —
a click that also collapsed the card took the parameters away at the moment you said which
effect you meant, which is the opposite of what selecting one is for, and the twirl mark is
right there. In the **Timeline** a plain click still opens the heading as well, because the
fold is how that outline is navigated and it has always worked that way; a *modified* click
there only picks, so Shift-clicking a run of effects does not flap every one of them open on
the way past. The twirl mark always folds and never picks, in both places.

**Copy takes the finest selection.** Keyframes when a panel has claimed them, else the picked
effects, else the layer — the ladder Delete has followed since K-234, and through the same
kind of claim (`copyClaim`, `pasteClaim`), because every hardware-key handler runs on every
key and a panel cannot claim a chord by handling it. The Timeline's hand-written `Ctrl+C` and
`Ctrl+V` comparisons are gone with it: they were fine while the shell had no copy of its own,
and would have been a double paste the moment it had one.

**`Mod+X` / `Mod+C` / `Mod+V` are in the keymap now**, where every other chord lives (K-199),
so they are rebindable and show beside their menu rows.

**`copy_effects` takes a list.** `Option<Uuid>` became `Vec<Uuid>` — empty is still the whole
stack — and the effects come back in stack order, not click order, so a copied group pastes
back in the order it was drawn in. Ids naming nothing on the layer are ignored; naming none
of them is a refusal rather than a silent whole-stack copy, which would be the worst possible
guess.

**What is deliberately not here.** Transform, Effects, Masks and Audio headings select like
any other row but are not copyable — Copy falls through to the layer, which is what a
transform copy would have to mean anyway. Cutting an effect removes it from the stack; cutting
with nothing but a layer selected still deletes the layer.

**K-301 · DECIDED · A row that is not animated still copies — its value is the thing being
copied.** K-300 made Copy take the finest selection there is, and left one hole in the
ladder: at the property level it took *keyframes*, so a row with none copied nothing, gave
up, and fell through to copying the whole layer. The one thing the user was pointing at was
the one thing that did not travel.

**Copy with rows selected and no individual key picked copies the rows whole**: every key of
an animated one, the plain value of one with no keyframes at all. Picking individual
keyframes still copies exactly those, which is K-196 unchanged.

**A copied value pastes as a value.** Onto a target that is not animated it replaces the
number; onto one that is, it sets a key at the playhead — which is what "put this value
here" can only mean on a row that already moves. A value has no time, so this paste is the
one that does not shift anything onto the playhead.

**The system clipboard gets the plain numbers** when nothing copied was animated,
tab-joined — the same text a value field's own right-click Copy writes, so a value copied
out of a row and a value copied out of a field are the same thing to everything else on the
machine. With anything animated in the set, the keyframe table is written as before.

**The other levels already carried their values** and are unchanged: `copy_layer`
serialises the whole layer and `copy_effects` the whole `.lumfx`, both including every
parameter that is a plain number. This entry is only about the property row.

**K-302 · DECIDED · A copy leaves a trace the machine can see, and a stored keymap can
never take a key away.** Two faults found the same afternoon by the owner, one hiding the
other.

**The keymap fault, which is the serious one.** A stored keymap replaced the session's map
whole (`keymap_from_json`: `*km = parsed`). A keymap file only knows the actions that
existed when it was written, so **every action added to Lumit afterwards had no chord at
all** for anybody who had ever saved one — and the workspace saves one on the first rebind.
K-300 bound `Mod+C`, every test passed, and the owner's `Ctrl+C` did nothing, because the
tests start from the shipped defaults and only a real session has a file.

A stored keymap is now **laid over the defaults**: the file's chord wins for every action it
names, and an action it never heard of keeps its default. That needed a way to tell "I took
that key away" apart from "that action did not exist yet", so `Keymap` gained `unbound` — a
list of deliberately silent actions — and unbinding records itself there. Absent from an
older file, which is right: nothing in one was ever a deliberate unbind that survived a
restart, because the whole map was being replaced anyway.

The general rule this is a case of: **restored state is laid over the current defaults, never
swapped for them.** Anything the user did not choose must come from the running build.

**The clipboard fault.** Layer and effect copies went into Lumit's own tray and nowhere else
(K-275's deliberate choice). Paste into a text editor and nothing arrives — which is
indistinguishable from Copy having done nothing, and is the first thing anybody checks. So
every copy is now **mirrored to the system clipboard as its document text**, and a paste that
finds the tray empty reads the system clipboard and takes a Lumit document back off it
(sniffed: a layer document says `kind`, an effect document is the `.lumfx` shape, and
anything else is somebody's shopping list and is left alone). The window also picks up a
document when it comes back to the front, so Paste is live rather than greyed over something
that is genuinely there.

The tray still comes first, because it holds the exact text this session copied — no round
trip, and nothing else on the machine can have overwritten it. K-275's cost line, "copying
between two running Lumit windows does not work yet", is paid off by this: the second window
takes the document off the system clipboard.
**K-303 · DECIDED · Lumit's words leave the code: one British-English `.arb` is the
source, Crowdin is where translation happens, and the engine's own labels come along
by lookup.** K-005 said UI strings go through an i18n table "from day one" and
[14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md) §7 made it binding — *no string
literal shown to a user lives in code*. Neither was ever true: the words lived inline
across ninety Dart files, and the rule had no test behind it. This makes it true, and
puts a gate under it.

**One file, in en-GB, written by hand.** `flutter_ui/lib/l10n/app_en.arb` holds every
phrase Lumit shows, each with an `@key` note saying where it appears and what
constrains it — the only context a translator gets. The other `app_*.arb` files beside
it come back from Crowdin and are never edited in this repo; a wrong translation is
fixed on Crowdin or the next sync overwrites the fix. `crowdin.yml` at the repo root is
the whole configuration: one source file, four target languages, credentials from the
environment because this repo is public. British English is the source and stays the
source (K-005); there is no en-US.

**Reached through a plain global, not through `BuildContext`.** Code writes
`l10n.importFootage`. A good third of Lumit's text is decided outside a widget — the
keymap, the tool table, the settings model — where the usual `Strings.of(context)` has
nothing to ask, and threading a context through all of it would be a large change for
no gain: Lumit has one window's worth of language at a time, and no case where two
halves of the screen want different ones. The cost is that changing the language does
not by itself repaint anything; it does not need to, because the only caller is the
settings model, whose `notifyListeners` already rebuilds the shell on the same frame.

**The engine's labels are translated by lookup on the English text.** Effect and
parameter names live in `lumit-core`'s schema, shortcut descriptions in `lumit-keymap`,
and both reach the interface over the bridge as plain English —
`lib/l10n/engine_labels.dart` maps each to a key. Rust is untouched: it learns nothing
about languages, no second set of identifiers has to be kept in step with the schema,
and a word with no entry comes back as it arrived rather than blanking a panel. The
limit is that the lookup is by word rather than by place, so two controls both called
"Scale" in English take one translation; the fix, when it bites, is to give the
affected label distinct English text in the schema, which is better practice anyway.
`test/l10n/engine_labels_test.dart` reads both Rust sources and fails if either can
send a word the table has no entry for, so a new effect or shortcut cannot quietly
ship untranslated.

**Tooltips shrank, because the spec always said they should.**
[07-UI-SPEC.md](07-UI-SPEC.md) §13.2 has always asked for the control's name and its
shortcut, with the sentence-length "rich" tooltip reserved for Lumit-specific
behaviour. What had grown instead was explanation — *"Put every parameter back to its
default, removing its keyframes"* on a button already labelled Reset. Every tooltip is
now under five words and most are two, and `test/l10n/arb_test.dart` keeps them there:
six exceptions are named in that test with their reasons — the three cache meters,
whose tooltips carry live numbers and warn that clicking throws work away, and the two
playback modes, which are the adaptive degradation §13.2 names outright. The same test
holds every string to the glossary, which is how *"Retime opens to Velocity"* and
*"Keyframe velocity…"* were caught and corrected to **speed** (§9 is binding for copy,
not only for identifiers).

**Settings ▸ Interface ▸ Language**, defaulting to the machine's own language and
storing nothing until the user chooses. Following the operating system for ever — and
after they change it — is the better default than freezing whichever language they
happened to launch in. The picker lists each language under its own name (Deutsch,
Қазақша, Українська, 简体中文) rather than under an English one, so somebody who has
set Lumit to a language they cannot read can still find their way back.

**What this costs.** Roughly a thousand keys to keep in order, and a `const` widget
constructor wherever a literal used to sit — a real but negligible allocation cost in
a shell that rebuilds on a notifier anyway. Simplified Chinese lands as plain `zh`
rather than `zh_Hans`, because Flutter requires a script-less base file to fall back
to and there is only one Chinese script to translate into; adding Traditional later
means adding `zh-TW: zh_Hant` to `crowdin.yml` and leaving `zh` as the base. The two
shortcut labels the engine builds with a number in them ("Add marker 3 at the
playhead") are not literals in Rust and so are not in the lookup table; they stay
English until the engine hands their number over separately, and that is in
[TODO.md](TODO.md).

**K-304 · PROPOSED · A release is exactly three files, and every one of them gates it.**
Supersedes the artefact list in K-252 (2026-08-03) and K-253 (2026-08-03): the Linux
release tarball is withdrawn. A tagged release now publishes a Windows setup `.exe`, a
macOS `.dmg` and a Linux `.flatpak` — one artefact per platform, nothing else. The
tarball asked the user to clone the repository and run `install.sh` to get a menu entry
and file associations; the Flatpak gives them both by installing, so the tarball was the
worse of two Linux stories and its `INSTALL.txt` existed only to apologise for that. The
staged bundle it was built from stays — the Flatpak is repacked from it.

The `continue-on-error` flags come off the macOS job and the Flatpak step at the same
time, and for the same reason: a release that quietly ships two files when three were
promised is worse than one that fails loudly. This means a Homebrew or Flathub hiccup can
now redden a tag, which is the intended trade. It also means the Flatpak added in K-253
gets proved: it has never once run, having landed the day after v0.1.0 shipped, and CI
builds no packaging at all — the first tag after this is its first execution. A tag
carrying a suffix (`v0.2.0-rc1`) publishes as a pre-release, which is the rehearsal.

Neither the installer nor the DMG is signed, and this entry does not change that. The DMG
is ad-hoc signed because macOS will not run a bundle with vendored dylibs otherwise
(`make-dmg.sh`), not because anyone has a certificate; Gatekeeper still warns, and
SmartScreen still warns on Windows. Developer ID signing and notarisation stay where
K-033 left them, waiting on an Apple Developer Program membership; Windows signing waits
on a code-signing certificate. Both are purchases, not code, and neither blocks a release.

**Reconciled with K-297 on merge.** This entry was written before in-place updating landed,
and "nothing else" was written against a release that had nothing else in it. K-297 attaches
a plain application archive per platform — a Windows `.zip` and a macOS `.zip` — for the
updater to fetch. Those stay: they are not installers and are not offered as a way to
install, so the count that matters is unchanged, and *three artefacts are installed, one per
platform*. The Linux side needs no archive of its own, because a Flatpak is updated from the
`.flatpak` bundle this entry already makes compulsory. `updates.dart` prefers
`linux-x64.tar.gz` and falls back to `.flatpak`, so an installation made from the withdrawn
tarball still finds something to offer.

**K-305 · DECIDED · Expressions run on Rhai, and "deterministic" means reproducible in
practice rather than bit-identical across platforms.** Supersedes **K-063**, which chose
JavaScript on QuickJS-ng and gave one reason for it: QuickJS is pure-software IEEE754, so
the same project gives bit-identical numbers on every machine, which a JIT engine cannot
promise.

The shipping implementation is [Rhai](https://rhai.rs) — a small Rust scripting language
that embeds directly, with no C dependency, no separate runtime to sandbox, and native Rust
types across the boundary. It went in first because it was the cheapest thing that worked,
and it stayed because the argument against it turned out to be weaker than it looked.

**Why the determinism objection does not block it.** Rhai's `sin`, `cos` and friends go to
the platform's libm, which is not required to give identical last bits everywhere, so two
operating systems can disagree in the final ulp. That is real. It is also, as Airyzz put it
reviewing this, *not new*: plenty of the existing engine already has the property, and the
GPU — where most of Lumit's arithmetic happens — offers no cross-vendor bit-identity at all
and never will. Buying exactness in the expression evaluator alone would not buy exactness
in the picture; it would buy one determinstic component inside a pipeline that is not.

**So the standard is stated rather than implied.** Lumit aims to be *reproducible*: the
same project on the same machine gives the same frames, every run, and the frame cache can
rely on that — which is what the cache key actually needs, and it is what the tests assert.
Across operating systems and GPU vendors, Lumit aims to be *as close as the hardware
allows* and does not promise the last bit. A user moving a project between platforms should
expect the same picture, not a byte-identical file. Getting as close as possible remains
the goal; pretending the floor is exact would be the lie.

This is a deliberate narrowing of a promise, not an abandonment of it. If a future feature
genuinely needs bit-identity across platforms — a distributed render farm splitting one
frame range across mixed machines, say — that feature brings the argument back with its own
evidence, and this entry is the thing it supersedes.

**K-306 · DECIDED · A text layer's words can come from an expression, and one resolver
serves both the picture and the cache.** From Airizz (2026-08-02), debugging expressions:
"im really just trying to print values to the screen at render time". Every property on a
layer could already be driven by an expression; the one thing that could not was the one
that would have shown the answer. `TextDocument` gains an optional `expression`
([03-DATA-MODEL.md](03-DATA-MODEL.md) §9.1): when set, the layer's line at layer time *t*
is that expression evaluated at *t* and printed — the same language the numeric properties
use, except the answer is shown rather than measured, so **any** result type is accepted
(refusing one would only mean wrapping it in a conversion).

**The typed `text` is kept, not overwritten**, and is what the layer says again once the
expression is cleared. An empty or whitespace-only expression *is* "cleared", never "an
expression that says nothing" — the alternative leaves a blank layer with no way back to
its words. **A broken expression prints nothing rather than failing the frame**: these are
typed against a live preview, where half a written expression is invalid for most of the
time it takes to write it, and an empty line is what the editor already shows for empty
text.

**The rasteriser and the frame cache key read the line through one function**
(`TextDocument::resolved_text`), which is the load-bearing part. Hashing the *stored* text
for an expression-driven layer keys every frame identically, so the number on screen would
freeze on whatever it read first — the feature shipping with the bug it exists to solve.
Routing both through one resolver makes that disagreement unrepresentable rather than
merely fixed, and gives the right cache behaviour for free at both ends: a frame-varying
line keys per frame, a constant one keys once, with nothing to configure. The cost is that
`feed_source` now takes the comp the layer sits in, since that is what an expression
context is built from.

Per-character animation of a driven line is **not** in scope and was not asked for; it
belongs with the styled-runs model ([03-DATA-MODEL.md](03-DATA-MODEL.md) §9.1). The engine itself is settled separately, in K-305.

**K-307 · DECIDED · A shape layer's own art is correctable on the picture, by the gesture that
already corrects a mask.** K-237 shipped shape layers and named the gap outright — "editing a
shape layer's points on the picture (K-224 edits *mask* points; the same gesture over shape
contents is the next piece)". Until now art could be drawn and then only *re*drawn.

**One piece of code, because it is one kind of thing.** A mask and a shape item hold the same
path type (`BezierPath`, in `lumit_core::mask`), so the points of both are found, drawn, swept
up and dragged by the same walk. What differs is only where the edit is written back: a mask
one at a time by id (`set_mask`), a shape layer's items as a whole list
(`set_shape_contents` / `SetShapeContents`). That is still **one op per layer**, which is the
rule K-224 set for a multi-layer point drag and the razor set for a multi-layer cut.

**A shape point and a mask point are told apart by their key, and by nothing else.** A layer
can carry both; their ids are both UUIDs; the selection is a set of strings. So a shape item's
key carries a `shape#` prefix. Without it a selection could not tell the two apart and a shape
point would be committed as a mask — which is why there is a test for exactly that case.

**Everything else K-224 decided carries over unchanged**, deliberately: the press order (handle,
then point of a *selected* layer, then layer, then empty space), the marquee that gathers points
when there are points to gather and settles on release, and the screen delta mapped through each
layer's own inverse so a selection spanning two layers still moves together. None of that needed
restating for shapes; it needed only to stop being mask-shaped.

**Still not built, and each for its own reason.** A **paint stroke's** points: a stroke is a
stored *gesture*, not a path, so it is a different piece of work rather than the same one
extended. **Bezier handles**, on any path, mask included — K-224 deferred them and the reason
has grown teeth since: `Vertex` has no linked/broken flag, corners being merely both tangents at
zero, so an `Alt`-drag that re-links a pair needs a [03-DATA-MODEL.md](03-DATA-MODEL.md) change
and a decision about what a file written before that flag means. It is not a gesture waiting to
be wired.

**K-308 · DECIDED · A shape point is drawn where its art is, the picture follows the drag,
and the art nobody dragged does not move.** Three faults of one gesture, all of them the
same misunderstanding about *whose coordinates a shape item's vertices are in*, found the
first time K-307's editing was used on a real drawing.

**A shape layer's picture is its art's bounding box** (docs/06 §1.2, `build.rs`): the
raster is exactly that box, and the layer's own pixel (0, 0) is therefore the box's
**top-left corner**, not the origin of the coordinates the vertices are stored in. A mask's
vertex is already in layer pixels, so it goes straight through the layer's map; a shape
item's is not, and the Viewer was pushing it through the same map unchanged — so every
drawn point sat a whole bounding box away from the art it belonged to, while the wireframe
box and the picture, which both read the *size*, agreed with each other. One subtraction,
in one place (`LayerBox.shapePoint`), and the points land on the art.

**Their outermost points sit exactly where the scale handles do**, which follows from the
box being the art. K-224's press order put a handle first, so on a drawn square every
corner was a scale and never a point edit — the gesture could not be performed at all on
the shape most likely to be drawn. A press within a *point's* own reach now means the
point; a handle's reach is twice as far, so nothing else about the order changes.

**Position follows the corner, in the same op.** Dragging the left-most point left grows
the box leftwards, which moves the corner the layer's pixels start at — so every *other*
point would slide right by the same amount, an edit nobody asked for. `set_shape_contents`
now commits the art and the position adjustment as one `Op::Batch`: one drag, one undo
step, and the art that was not dragged stays where it was. A keyframed position is left
alone — it has no single value to add to, and moving one key of a curve would be a worse
surprise than the drift.

**And the drag previews.** A point drag showed its wireframe and left the picture until the
release, because the preview call had no room for a path — it has since (K-239, K-240), so
it does now, throttled like every other live drag. Art and the compensating position ride
in one request, because a preview of the art alone would show the untouched half sliding
and the commit would put it back. One layer at a time, as with a move: the engine patches
one layer into its clone. A layer whose mask *and* art are dragged together previews the
art; the mask catches up on release.

**K-309 · DECIDED · The macOS application icon is a layer stack, not a rendered picture:
`lumit-icon.icon`, compiled by Xcode.** Extends K-251's brand set (the artwork is
unchanged) and takes the icon half of K-033's macOS pass. macOS 26 composites app icons
itself — the squircle mask, the bevel, the shadow, the specular highlight that tracks the
pointer, and the dark, tinted and clear appearances a user can put the whole Dock into.
None of that is available to a flat image: the system needs the pieces separately, so the
question was never whether to render better PNGs.

- **The source is `assets/brand/lumit-icon.icon`**, an Icon Composer document holding the
  mark's six pieces as SVG layers (tile, bloom, blue key, magenta key, core glow, core
  diamond) and an `icon.json` recording the stack — which layers are glass, per-appearance
  opacity, shadow depth. The flat `lumit-icon.svg` stays in the brand set as the reference
  drawing and the single-image hand-out, but nothing ships from it any more.
- **The layers omit their own lighting**: no corner radius on the tile, no drop shadow
  under the keys, and no dark rim stroke around them — that stroke exists in the flat
  icon to imply a lit edge, and Liquid Glass bevels and lights each layer for real. All
  three are supplied by the system per appearance, and a painted-in copy doubles up in
  all of them (docs/15-DESIGN.md, brand).
- **Xcode compiles it, so there is nothing to regenerate.** The `.icon` is a resource of
  the Runner target — referenced in place at `../../assets/brand/`, not copied into
  `flutter_ui/`, so the brand folder stays the one home for artwork — and
  `ASSETCATALOG_COMPILER_APPICON_NAME` names it. `actool` also emits a flat `.icns` from
  the same layers for Macs before 26, verified against the project's 10.15 deployment
  target, so one source covers every supported macOS.
- **`Runner/Assets.xcassets` and its `AppIcon.appiconset` are deleted**, and
  `scripts/gen-icons.py` no longer writes macOS PNGs. The catalog held nothing else, and
  the appiconset was the same artwork by a second route: two sources of one drawing is how
  they come to disagree. The script keeps the Windows `.ico` files and the document
  `.icns` files, which are unaffected.

This is the icon only. Signing and notarisation stay open under K-033: the disk image is
still ad-hoc signed, which Gatekeeper reads as unsigned.

**K-310 · DECIDED · The macOS artefacts are Developer ID signed and notarised in CI;
the Windows installer stays unsigned.** Supersedes the fourth paragraph of **K-304**
(2026-08-07), which recorded that neither artefact was signed and parked both behind a
purchase. The Apple Developer Program membership has been bought, so half of that
paragraph has expired; the Windows half has not.

A tagged release now produces a `.app` and a `.dmg` that are signed with a Developer ID
Application certificate, built with the hardened runtime and a trusted timestamp,
notarised by Apple and stapled. Gatekeeper opens them without the right-click ceremony,
including on a machine that has never been online — that is what stapling buys, and it is
the reason to staple rather than to rely on Apple being reachable at first launch.

Signing is *opt-in through the environment*, not compulsory. `make-dmg.sh` signs ad hoc
when `MACOS_SIGN_IDENTITY` is unset and skips notarisation when `APPLE_API_KEY_PATH` is,
which is what a laptop build and a fork both get. This keeps one script for both worlds:
the alternative — a signed path only CI exercises — is a path that breaks silently and is
discovered by a tag. The six secrets live in the repository; the identity string is one of
them rather than a literal in the workflow, because it carries a legal name and this is a
public repository (the name is embedded in every signed binary regardless, which is
unavoidable and normal, but there is no reason to commit it as well).

Two details are load-bearing and easy to lose. **`codesign --deep` is banned here.** It
walks nested bundles but is unreliable for the loose dylibs `dylibbundler` copies into
`Contents/Frameworks`, and notarisation answers a missed binary with a rejection twenty
minutes after the tag; the contents are signed explicitly instead, innermost first, since
a bundle signature seals a hash that signing its frameworks afterwards would change.
**Notarisation happens twice**, because a ticket covers exactly what was submitted: once
for the `.app` that K-297's in-place updater downloads as a bare `.zip`, once for the
`.dmg` a first-time user double-clicks. One submission would leave whichever artefact was
skipped quarantined, and the updater's payload is the easier of the two to forget.

Signing the Windows installer still waits on a code-signing certificate, so SmartScreen
still warns. That remains a purchase rather than code, and it does not block a release.

**K-311 · DECIDED · Traditional Chinese is the fifth language, and the locale a
translation file names is settled on Crowdin rather than repaired here.** K-303 named four
target languages and said adding Traditional later meant adding `zh-TW: zh_Hant` to
`crowdin.yml` and leaving `zh` as the fallback. The first real Crowdin pull landed it, so
that is now done: `app_zh_Hant.arb` sits beside `app_zh.arb`, and Settings ▸ Interface ▸
Language lists 繁體中文 under its own name like the rest.

**A script, not a country.** The file is `zh_Hant` and not `zh_TW` because `localeTag` in
`lib/l10n/strings.dart` writes a locale's *script* into the settings file. A country name
comes back from Flutter's generator as a `countryCode`, which `localeTag` does not write —
so `zh_TW` and `zh` would both save as `"zh"` and the user's choice would not survive a
restart.

**The `@@locale` trap, which cost a red main.** Crowdin writes its own code into the
`@@locale` key inside every file it sends back — `zh-CN` into the file `crowdin.yml` asked
it to call `app_zh.arb`. Flutter's generator refuses to run when that key and the file name
disagree, and it runs on `flutter pub get`, so the first pull took down all three Flutter
jobs before a single test was reached. The fix belongs on Crowdin, in the per-language
custom ARB code, because a hand-edit in this repo is overwritten by the next sync (K-303).
What this repo owes is a loud failure: `test/l10n/arb_test.dart` now checks every `.arb`
against its own file name, so the next bad sync fails one named test with the remedy in its
message instead of an opaque `pub get` error.

**There is still no en-US.** The same pull brought an `app_en_US.arb` — the British source
copied under another name, from a target language enabled by mistake. K-303 said British
English is the source and stays it; the file is deleted and the test above keeps it deleted,
pointing at the Crowdin setting that produced it.

**K-312 · DECIDED · Two of Icon Composer's settings are unusable, and a one-second
Linux check keeps them out.** K-309 made the macOS icon a layer stack authored in Icon
Composer and compiled by `actool`. Two things Icon Composer 26 writes into `icon.json`
cannot then be compiled: a non-empty top-level `features` array, and a `specular` whose
value is the string `"inside"` rather than `true` or `false`. Both arrived with the icon
revision that landed alongside the signing work, and both were invisible until the
localisation fix (K-311) unblocked the macOS build job they had been hiding behind.

**The failure names the wrong thing.** `actool` does not report an unsupported setting;
it dies part-way through with `attempt to insert nil object from objects[0]` and a
twenty-frame backtrace through `AssetCatalogFoundation`, under the heading *Could not
open "lumit-icon.icon"*. That reads as a corrupt file, and sends you looking at the SVGs
— which are fine. Each key was found by bisecting `icon.json` against the last revision
that compiled, one property at a time.

**What the icon loses is a highlight's address, not the effect.** The `refractivity`
blocks are untouched and still compile; `features` only *declares* which of them the
document uses, and the icon renders the same without it. `specular: "inside"` becomes
`specular: true`, which keeps the specular highlight on that group and gives up only the
choice of where inside it sits. The rendered `.icns` was checked by eye after the change.

**`scripts/check-icon.py` runs in the design-token lint job**, on Linux, in about a
second. The macOS build already catches this, but it catches it five minutes in and only
on a runner with Xcode 26 — and reopening the icon in Icon Composer and saving is enough
to put both keys back, so this is a mistake with a standing invitation to recur. The
script is the regression test K-007 asks for: it fails on the `icon.json` as it was, and
passes on the one that compiles.

**K-313 · DECIDED · Bokeh folds into Depth of field rather than shipping beside it; the
"advanced" slot is reserved for a genuinely physical model.** The contributed Bokeh effect
(PR #38) brought an aperture polygon (blades, roundness reaching below zero into stars, an
anamorphic squeeze, a radial weighting), a split-at-threshold power mean so a small bright
thing blooms into a ball instead of dissolving into its surroundings, and a fuller depth
model (channel pick, click-to-focus point, a Profile that scales the depth distance before
the ramp, edge-leak suppression). It shipped as a second effect standing next to `dof`.

**Why it folds in.** §3.22 has recorded since K-124 that the flat disc was the *base* and
that "shaped, bright-rimmed highlights are the planned DOF PRO second effect" — but none of
the above is a second effect's worth of physics. There is no scene-referred aperture, no
f-stop, no per-pixel scatter, no occlusion or inpainting behind foreground edges, no
spectral response. It is the base lens blur finished properly. Shipped beside `dof` it
would have left the *default* depth-of-field permanently the worse of the two, and left two
90 %-identical gathers to maintain. Folded, the shipped effect gains the aperture and
DOF PRO stays free for the physically-accurate, deliberately intensive model the owner
wants it to be.

**Every added control is neutral at its default, and neutral is reached by BRANCHING.**
Roundness 1 takes the plain `r² ≤ coc²` circle test, Concentration 0 and Remove edge leak 0
take the unweighted accumulation, Exposure 0 takes the unsplit sum. None of those is an
IEEE 754 identity — `Σ(c·w)/Σw` is not `Σc/n` when every `w` is 1, `min(c,t) + max(c−t,0)`
is not reliably `c`, and scaling both sides of a comparison by `apothem2` can flip a
boundary tap — so multiplying by one would have re-rendered every saved project by an ULP
or two. The branch is what makes the fold safe, and
`the_default_aperture_is_the_historical_disc_bit_for_bit` pins it on the arithmetic rather
than asserting it in a comment. Profile is the exception that proves the rule: its neutral
is a multiply by exactly 1, which *is* exact, so it needs no branch.

**Three of the contributed controls are dropped.** Placement and Resolution were declared
but inert; Custom blur shape declared a layer reference the kernel never bound. Resolution
was worse than inert — read as a band count quantising the defocus ramp, it put a real
depth pass's whole content in one band and its near object in another, making focus
all-or-nothing. A declared-but-dead control is a promise the panel cannot keep, so they go
until someone can say what they do.

**No `ParamKind::Point` was added.** The contributed branch introduced one; the panel had
meanwhile learned to fold two adjacent `_x`/`_y` Float parameters into a single row with a
crosshair pick (docs/07 §6.1), which the Lens flare's Light and Radial blur's Centre both
ride. Focus point uses that instead — the naming convention is the whole mechanism, and a
schema kind, a bridge kind, generated code and a Dart row were deleted rather than written.
`ParamKind::Angle` *is* added: there is no arrangement of existing rows that draws a dial.

**Consequence:** `EffectSchema` gains `enabled_when` (the greyed-row rule, evaluated by
`lumit_core::fx::param_enabled` and mirrored on the panel), the schema gains `Angle`, and
`dof` gains three collapsed twirls — Iris, Highlights, Depth map. The stops-to-power
constant (`EXPOSURE_STOPS_PER_DOUBLING`, 12) is fitted from screenshots rather than measured
against a reference plugin, and §3.22 records it as open.

**K-315 · DECIDED · Depth of field's control surface, after the owner's first pass on
it.** Five changes, each from testing K-313 in the app rather than from a spec.

**Composite mode is gone.** Five blend modes on one effect is a menu nobody has a reason to
open: an effect whose result wants adding over a sharp plate is an adjustment layer with a
blend mode, which already exists, does it in one obvious place, and does it for *every*
effect rather than for whichever ones happened to grow a dropdown.

**The depth channel list is five entries, and every one can explain itself.** Luminance (the
default — right for the grey map a depth pass usually is, whatever channels it was written
to), Alpha (some renderers put depth there), Red/Green/Blue (a packed pass, several AOVs
flattened into one image). Hue, saturation, lightness and the plain channel mean are gone:
nothing encodes a depth or a density as a hue, and offering the option only invites someone
to find out. **Depth invert moves into that group** — it is part of how the pass is *read*,
not part of where focus is. Changing the default from Red to Luminance is a look change on a
non-grey depth pass; it is recorded here rather than hidden, and K-313 has not shipped.

**Focus distance, Use focus point and Focus point are now adjacent.** A switch that hands one
control's job to another is an affordance only if both are visible at once; with the toggle
three twirls below the number it governed, neither row explained the other.

**Three renames, because the names were the problem.** *Profile* → **Gamma**: it is a
gamma on the depth axis, deciding how hard the blur answers to a small change in depth.
*Concentration* → **Rim brightness**: it decides where the light sits inside each ball, which
is spherical aberration. *Deform* → **Aspect ratio**. None of the three could be guessed from
its old label, and a control nobody can name is a control nobody uses.

**The angle dial sits beside its number, not under it**, and the same control now serves every
*unbounded* rotation in the catalogue: the Transform effect's Rotation, Hue shift's Angle (a
hue shift is a rotation about the colour wheel — the most dial-shaped control there is) and
the Lens flare's aperture Rotation. The two blur-direction angles keep their `±3600` hard
bound and stay plain numbers, because `Angle` is deliberately unbounded and swapping them
would quietly drop a clamp.

**K-316 · DECIDED · Body text renders at regular weight; Medium is rationed to emphasis.**
From the owner (2026-08-09): "less text should be bold — a lot of it is too thick and can
actually reduce readability." The cause was packaging, not the type scale: only
`Inter-Medium.otf` was bundled, so every weight the code asked for drew as Medium and the
whole interface sat a step bolder than docs/15-DESIGN §7.1 specifies (12px *plain* Inter
for panel copy, menus and buttons; Medium reserved for dialog emphasis and tab labels).
`Inter-Regular.otf` (same v3.019 build as the bundled Medium, so metrics match) is now
bundled at weight 400; the theme's `body`/`small`/`caption` styles request w400 and
`heading` keeps w500, joined by a `bodyStrong` getter (w500) that the dock tab pills and
drag ghost titles use — the two "panel tab label" emphasis sites §7.1 names. The two spots
that had hand-forced `FontWeight.w400` over the Medium default (timeline marker flags)
drop their overrides. No spec change: this aligns the build with what 15-DESIGN already
said. Regression test: `body text is regular weight; emphasis is medium and rationed`
(theme_test.dart).

**K-317 · DECIDED · The type scale drops a step, property rows tighten, and a selected
bar brightens instead of growing an outline.** Three owner calls from testing K-316 in
the app beside After Effects (2026-08-09).

**Type drops one step.** Body Inter goes 12px → 11px (and with it every value field,
menu and button, since they all read the theme's `body`/`bodyPrimary`); `small` 11 → 10;
`caption` 10 → 9. docs/15-DESIGN §7.1's table moves with it in this commit. Beyond size,
the owner's "words feel soft" reads as Flutter-on-Windows greyscale antialiasing (no
ClearType subpixel rendering), which no theme value reaches; the smaller regular-weight
face is the lever the theme has.

**Property rows tighten.** The vertical breathing space on effect, transform and source
rows goes 3px → 2px a side, bringing the Effect controls' row rhythm to AE's. One value
in four files (`effect_param_row_frb`, `transform_rows_frb`, `source_rows_frb`, and the
point row) — the Timeline's fold-out already passes zero and is untouched.

**A selected bar brightens.** The lane bar used to mark selection with a 1px accent
outline; on a 22px bar that is a whisper. It now lerps its label colour 35 % toward
`textPrimary` — the hue still says which layer it is (K-188's rule survives), and the
lit bar is what AE does and reads at any zoom. No spec pinned the outline, so nothing
else moves.

**K-318 · DECIDED · Submenus survive the diagonal: the safe hover triangle.**
From the owner (2026-08-09): "when going through menus of any kind, I think we need to add
safe hover triangles — like how JavaScript has intent plugins." A flyout opens beside the
row that owns it, so the natural path to its first entry crosses the rows *below* that row;
the menu switched on whichever row the pointer merely passed over, and the flyout vanished
before it could be reached. The fix is the classic one: while a flyout is open, a hover
report from another row of the same surface is **held** while the pointer is inside the
triangle from where it left the owning row to the flyout's near edge. The held switch lands
when the pointer leaves the triangle, or after a 300ms grace if the pointer simply stops
there — resting on a row still means that row, which is the property a plain delay would
lose. Reaching the flyout voids anything pending; a move that is not travel at all (straight
down the menu) switches with no delay. The geometry lives in
`flutter_ui/lib/widgets/hover_intent.dart` as pure arithmetic (`SafeTriangle`, tested as
such), with the timers and hover state in `FloatSurface` — so every popup on the shared menu
surface gets it at once: the menu bar, the Add effect browser's category flyouts, and every
right-click menu. No animation, no toolkit dependency. Regression tests:
`hover_intent_test.dart` (the geometry, and three submenu journeys — crossing, settling,
leaving).

**K-318a · NOTE · What Flutter already gives, and what it does not (the wheel check).**
Asked by the owner before merging K-318/K-319: are we reinventing things the toolkit ships?
Checked against Flutter 3.44.7's own source, and worth recording so it is not re-argued.

**Already Flutter's, and used as such:** `ReadingOrderTraversalPolicy` is the traversal K-319
installs — not a hand-written comparator. `TextSelectionGestureDetectorBuilder` is the
press-to-caret/drag-to-highlight the value and timecode editors gained; the earlier code's
fault was a bare `EditableText` with no gesture builder around it, not a missing feature.
`DismissIntent` is now what Escape means in a modal (see below).

**Flutter has a weaker answer, so ours stands.** `SubmenuButton` offers `hoverOpenDelay`
(`material/menu_anchor.dart`) — a plain delay before a flyout *opens*. That is the naive fix
K-318 rejected: it makes every submenu feel sluggish, and it does not address the actual
complaint, which is that crossing a sibling row **closes** the flyout you are travelling to.
There is no safe triangle anywhere in the framework (no hit for `safeTriangle`/`hoverIntent`
in `packages/flutter`). The K-318 geometry is therefore not a reimplementation.

**Flutter has nothing:** a radial/pie menu (no decision number yet). Correct to build.

**We did duplicate one thing, mildly.** `WidgetsApp` already binds Enter/numpadEnter/Space to
`ActivateIntent`, and `FocusableActionDetector` bundles focus + hover + shortcuts + actions —
so the per-control `Focus(onKeyEvent:)` in `HouseButton`/`HouseCheckbox`/`HouseRadio` is about
eight lines each that the Actions system could carry. It is left as it is *for now*, on
purpose: the house controls are deliberately not Material (K-084), the hand-rolled version is
tested, and — the part that matters — it does **not** take focus on a mouse click, so a
clicked button shows no focus ring. Moving to `ActivateIntent` means opting into the standard
focus-highlight behaviour and re-deciding that. Worth doing as its own change with its own
look, not folded into this one.

**The check found a real bug, which is why it was worth doing.** `showLumitModal`'s comment
claimed dismissal happened on "Escape, via the route" — but a Lumit modal is an
`OverlayEntry`, not a route, so **nothing listened and Escape did nothing in every dialogue
in the application**. Fixed the framework's way rather than with a tenth key handler: the
window contributes an `Actions` entry for `DismissIntent`, which `WidgetsApp` has already
bound Escape to, and dismissing means completing with null exactly as a click on the scrim
does. Regression test: `Escape closes a modal, the same as clicking the scrim`
(dialog_keys_test.dart), which fails without the `Actions` entry.

**K-319 · DECIDED · Every window has a default action; every control answers the keyboard;
Tab reads left-to-right, top-to-bottom.**
From the owner (2026-08-09), three complaints in one shape — "opening any confirmation
window should have the okay button selected by default, and pressing enter presses whatever
is currently the selected button", "tabbing through menus needs to be improved… left to
right then top to bottom", and "when a user clicks a text/value box but immediately starts
dragging without lifting up, it should still just be like they've selected the box". All
three were the same gap: house controls were painted, not focusable. `HouseButton`,
`HouseCheckbox`, `HouseRadio` and the idle `DragValueField` now hold a `ControlFocusNode`,
draw the accent focus ring (docs/15 §6.5) and answer `Enter`/`Space`; the global shortcut
handler stands down while one holds focus, exactly as it already did for a focused text
field, so a dialogue's `Enter` can never also fire a panel command. Each confirmation window
names one **default action** — affirmative, or safe where the affirmative is destructive —
which is `primary: true` *and* `autofocus: true`; K-243 had established that shape for the
Pre-compose dialogue alone, and it is now all of them (disk-cache clear, composition
settings, export, project settings, theme name, theme-editor save, marker label, update
offer, restart). Modals wrap their body in a `FocusScope` + `ReadingOrderTraversalPolicy`, so
Tab cycles inside the window in *visual* reading order rather than widget-tree order — the
two disagree wherever a layout nests columns inside rows. For the value boxes: a drag that
never crosses one increment now cancels as a drag and then opens the editor (a click that
wobbled is a click); the editor opens with the value **selected**, since a value is retyped
far more often than amended; the numeric and timecode editors gained the desktop selection
gestures they never had, so press-and-drag highlights; and `HouseTextField` takes focus on
the pointer's *down* stroke so a press that slides into a drag selects text from the first
pixel. Regression tests: `dialog_keys_test.dart`.

**K-320 · DECIDED · A dragged zoom slider anchors once.**
Same report ("zooming in the timeline with the slider can still ping around a lot"), and
K-293's anchoring was right but measured at the wrong moment. `_setZoom` re-measured the
anchor on every drag update, reading `_hLane.offset` *before* layout had corrected it for
the zoom just applied — a fresh zoom against a stale offset — so each update re-anchored
somewhere slightly wrong and the lanes lurched; near the viewport edges the in-view/recentre
branch flip-flopped as well. The slider's drag now brackets the gesture (`onChangeStart`/
`onChangeEnd` on `HouseSlider`): the anchor is chosen once, at the start, and held to the
end, which is the invariant the flight already assumed. The anchor's per-frame width is also
taken from the scroll position's own content extent — the same numbers `zoomAnchorOffset`
applies it with — rather than from the build-time viewport cache, which disagreed by a
little at every zoom and by more the further in you were. Landed with K-319, from the same
report; the zoom rule it amends is docs/07 §4.6.

**K-321 · DECIDED · `Enter` renames the selection; nothing renames on a double-click; effects
can carry their own name.**
From the owner (2026-08-09): "if there's anywhere still allowing double click or click a
selected item to rename, drop that behaviour and instead enable pressing enter to edit the
name of the selected item (this also needs to work for effect names in the effect control,
but not property rows, just effect name)." K-191 had already moved compositions off the
second-click rename and K-243 had given the Timeline `Enter`; the Project panel still
renamed footage, solids and folders on a second click, which is the same gesture as a slow
double-click and opened editors under people's pointers. That is gone: a second click
*opens* (K-191's rule, now without exception), and `Enter` renames whatever the focused panel
has selected. Two new actions join the keymap — `item.rename` (Project) and `effect.rename`
(Effect controls) — bound to `Enter` in their own contexts, so the binding is live in the
focused panel alone and one press can never open two editors (the Timeline's handler gained
the same guard). **An effect instance gains `custom_name: Option<String>`**
(`serde(default, skip_serializing_if = "Option::is_none")`, so a project without one is
byte-for-byte unchanged and an older file reads as `None`). It is a display name only:
`match_name`, the schema, the parameters and every lookup are untouched, and it shows in
place of the effect's label in both the Effect controls heading and the Timeline's fold-out.
`BridgeEffectInstance::set_custom_name` stages it and `set_effects` commits, so a rename is
one op and one undo step; an empty or whitespace name clears back to the label. Parameter
rows are not renameable — a parameter's name is the schema's. Regression tests:
`custom_name_roundtrips_and_defaults_to_none` (lumit-core),
`enter_renames_the_selection_in_each_panel_that_has_one` (lumit-keymap), `Enter renames the
selected item` (project_panel_frb_test.dart), `Enter renames the selected effect, and the
name persists` (effect_controls_frb_test.dart).

**K-322 · DECIDED · The default workspace puts Effects & presets in the right-hand column.**
From the owner (2026-08-09): "the default workspace layout should move the effect and preset
panel to the right side panel." It also settles a disagreement between code and spec that
had stood since the port: docs/07 §1.6 always described the Edit workspace as having
"right column Effects & Presets", while `defaultLayout()` made it the *third tab of the left
group* — behind Project and Effect controls, so it was never visible on a fresh install —
and fronted the **Debug** view in the right column instead, which is a developer panel. The
left group is now Project (fronted), Effect controls, Hierarchy; the right group is Effects
& presets (fronted), Scopes, Debug. Shares are unchanged (0.68/0.32; 0.22/0.58/0.20), and
the other three presets are untouched. A saved workspace is unaffected — this is the factory
layout, which Reset workspace restores. Regression test: the amended `default layout matches
default_layout() structure and shares` (dock_test.dart), which now also pins which tab each
group opens on.

**K-323 · DECIDED · `Escape` is the way out of an inline editor, and it writes nothing.**
From the owner (2026-08-09), testing K-321: "escape still doesn't exit the rename dialogue".
It never did, and the reason is worth recording because K-319 looked like it had covered
this. K-319 gave *modals* an Escape by contributing an `Actions` entry for Flutter's own
`DismissIntent`, which `WidgetsApp` already binds the key to. An inline rename is not a
modal — it is a text field that replaced a label in place — so there was no `DismissIntent`
handler anywhere above it and the key reached nothing.

**The gap was the shape of the contract, not one missing handler.** K-243 established that
every way out of an inline rename *commits*: Enter commits, clicking away commits (that was
the point of K-243), losing focus commits. That is right — a rename typed and then abandoned
by clicking elsewhere should not be thrown away. But it left no way to change your mind at
all, on any of the three inline renames (an effect's name, a layer's name, a Project item's
name) or in the value boxes, which have the same all-roads-commit shape.

**So `Escape` cancels: the editor shuts and nothing is written.** `HouseTextField` gains an
`onCancelled` callback and the two renames that use it pass one; the Project row's editor is
a bare `EditableText`, so it wires the same key on its own focus node; `DragValueField`'s
open editor does the same for typed numbers. In every case the key is handled on the field's
**own focus node**, which sees it before the `Shortcuts`/`Actions` system — deliberately,
because `EditableText` has its own `DismissIntent` handling and a handler placed above it
could be swallowed. Clearing the editing flag *before* the editor closes is load-bearing in
the value box: closing it is what loses focus, and the focus listener commits on focus loss.

Regression tests, one per surface, each failing without the fix: `Enter renames the selected
effect, and the name persists` and `Enter renames the selected item` (extended with an
Escape leg), `Enter renames the selected layer` (timeline_panel_frb_test.dart), and `a value
box opens its editor with the text selected` (dialog_keys_test.dart).

**K-324 · DECIDED · The Ctrl+Space console: a search bar over the effects, and a Blender-style
radial menu under it. Supersedes K-102's deferral.**
From the owner (2026-08-09): a Ctrl+Space window with "at the top a search bar the user can
type in… effect options then a little divider for comp names", modelled on Video Copilot's
**FX Console** ("with the little camera/snapshot button too"), and "below this bar a radial
menu just like Blender's" whose entries follow the selection. K-102 deferred exactly this
("the effects radial menu (Ctrl+Space, apply-to-clip) — that remains blocked on a from-scratch
build (no egui 0.31-compatible `egui_pie_menu`)"). That blocker is gone with egui: the port to
Flutter (K-174) means a ring is a `Stack` of positioned labels over a gesture detector, and
the only real content is the arithmetic of which slice a direction means. This entry
supersedes that half of K-102; the command palette (Ctrl+Shift+P) stays exactly as it is,
because the two answer different questions — the palette is every command by name, the
console is *effects*, fast, plus the thing you were about to do.

**The search half.** Effects first, then a divider, then compositions — ranked within each
kind and never across it, because the reason to open this window is nearly always an effect
and a comp that happened to score better would be in the way. Matching is the palette's
subsequence ranking (earlier and tighter wins), so "gau" finds Gaussian blur. Enter applies
the top match to **every** selected layer, as the Effect menu does (K-217); a comp fronts.
The **snapshot** button beside the field writes the frame on screen to a PNG — a one-frame
image-sequence export (`codec: 'png'`, K-201) rather than a second still-writer beside the
exporter, so it is the same tested path to a file and the status line already reports it. It
lands in a `Snapshots` folder beside the saved project, or the user's pictures folder when
the project has never been saved — never the working directory.

**The radial half.** A slice is chosen by **angle alone**, not by hit-testing a drawn wedge:
flick in a direction and the choice is made however far the pointer travelled, which is what
makes a ring faster than a list once the hand has learned it. A dead zone in the middle picks
nothing, so opening the menu and releasing without moving cancels rather than committing to
whatever was nearest. The first slice is straight up and they run clockwise. The entries are
chosen from the selection in four contexts — a picked effect offers what you do to an effect;
a selected layer what you do to a layer; a composition with nothing selected the new-layer
menu; nothing open at all the two ways to get somewhere — each capped at six, because a ring
of twelve is a ring nobody learns and the long tail is the search bar directly above it. A
slice that cannot run right now is drawn dimmed rather than dropped, so a direction a hand
has learned keeps its meaning.

**Where the lists come from.** `menu_bar_frb.dart`, beside the menu items, for the same
reason the palette's commands are declared there (K-102): the effects this applies and the
comps it fronts must be the ones the menus mean, and a second list would drift.
`fx_console_frb.dart` is the widget and knows nothing about the document;
`fx_console_context.dart` holds the selection knowledge; `widgets/radial_maths.dart` is the
geometry, widget-free so it is tested as arithmetic. Regression tests:
`radial_maths_test.dart` (slice centres, direction-picks-slice at any distance, the dead
zone, every wedge boundary, an empty ring), `fx_console_test.dart` (subsequence ranking,
effects-before-comps, Enter applies the top match, the snapshot button's two states, a flick
runs a slice, a dead-zone release cancels, a disabled slice keeps its place),
`the_fx_console_has_its_own_chord_and_does_not_clash` (lumit-keymap — and the bare space bar
still plays).

**K-325 · DECIDED · The console opens around the pointer, the search waits to be asked, and
rings nest. Reshapes K-324's presentation; the chord, ranking and snapshot stand.**
From the owner (2026-08-09), after working with K-324's console, four faults with how it
presented: it opened as a centred window rather than at the mouse; the search half listed
every effect before anything was typed; the box was opaque over the very frame it acts on;
and the ring for a *selected layer* offered "Solid" and "Text" — new-layer commands that
have nothing to do with the thing selected.

**It opens where the mouse is.** The ring is centred on the pointer, because the whole point
of a ring is that the flick can start the instant the chord lands — travel to a window first
and a list would have done. The key event carries no position, so the shell records the last
pointer position — through a **global pointer route**, not a widget `Listener`: the owner's
first build showed a `Listener` misses everywhere no widget claims the hit (the Viewer's
texture, exactly where this menu is most wanted), so the console kept opening at wherever
the pointer had last crossed a panel. The route sees every pointer event regardless of hit
testing, and is still one plain field write per event — no `setState`, no bridge call, so
the no-bridge-in-rebuild-paths budget is untouched. The **search bar floats above the
ring**, or below it when the pointer is near the top of the window; centre and bar placement
(edge clamping included) is `fxConsoleLayout` in `radial_maths.dart`, pure arithmetic with
its own tests. No boxed window; the console's surfaces are the standard menu float let
through a little (`surface3` at 0.88 — derived from the theme, no new colour), over the
modal scrim at **half strength** (from the owner, same day: a slight darkening keeps every
slice legible over any frame, while a full scrim would shut out the very work the console
acts on).

**The search waits to be asked.** An empty bar lists nothing — the ring is the offer. Typing
opens a dropdown *below the bar* with the matches (K-324's ranking unchanged: effects first,
comps after the divider, never across), and the ring steps aside while the query is
non-empty, both because the dropdown needs the room and because starting to type *is*
choosing the other way in. Escape retreats one step at a time — clear the text, then pop a
sub-ring, then close — and Enter on an empty bar closes rather than sitting inert. Escape is
handled at the **keyboard itself** for the console's lifetime, the way the shell's own
shortcuts are, and nowhere else: the owner found a handler on the search field's focus node
answers only while the field has focus, which a pointer resting on the ring need not have —
and one handler means one press is always exactly one step back.

**Rings nest, so context stays honest.** `RadialEntry` gains `children`: choosing such a
slice expands the menu in place (Blender's nested pies), the centre of the ring names where
you are and steps back out, and a caret on the slice says it expands. The layer-selected
ring is now only what you do to *this* layer — Duplicate, Add effect, Pre-compose (wired to
the real pre-compose dialogue now, not a jump to the Timeline panel), Delete — plus a
**New ▸** slice whose sub-ring is Layer ▸ New's six items in the menu's order. The
comp-with-nothing-selected ring keeps creation at the top level (that context *is* "make me
a layer") reordered to match the menu, and the picked-effect and nothing-open rings stand.

Regression tests: `fxConsoleLayout` placement (centres on the anchor, pulls in at edges, bar
flips below near the top, tiny-window fallback — radial_maths_test.dart); the empty bar
lists nothing; typing opens the dropdown and hides the ring, clearing restores it; Escape's
one-step retreat; the ring centres on the anchor; a child slice expands in place, the centre
backs out, a flick expands rather than closes, Escape pops before it closes; Enter on an
empty bar closes (fx_console_test.dart).

**K-326 · DECIDED · The Keyframe ring: the console keys a transform row where the playhead
stands, and the Timeline shows the key it made.**
From the owner (2026-08-09): "maybe on the radial menu having a keyframe option, which opens
up all properties on that layer you could add a keyframe to in that position, and clicking
adds one and opens that property row in the timeline if it's not already". So the
layer-selected ring gains a sixth slice, **Keyframe ▸** (the ring is now at K-325's cap of
six), whose sub-ring is one slice per transform row: Anchor point, Position, Scale, Rotation,
Opacity — the five everyday rows, not the 3D extras, both for the cap and because Rotation
X/Y remain the fold-out's business. A row driven by an expression is dimmed rather than
dropped: writing keys over an expression would delete it.

**Choosing a slice plants a key at the playhead holding the value already there** — nothing
moves, the same invariant the stopwatch keeps — with every axis of the row keyed together
and the key inserted in time order. A row already keyed at the playhead skips the write; in
both cases the Timeline is fronted with **that row open**, so the key just made (or found)
is on screen. The reveal is a new `revealPropertyRequest` on the shell state, speaking the
same `reveal.*` words the P/S/R/T/A keys use so one mapping serves both — but it
**ensures open** rather than toggling, because asking to see a row twice must never hide it.

Regression tests (fx_console_context_frb_test.dart, against the real engine): the ring is
exactly the five rows; a slice plants one key at the playhead and fires the reveal; the same
frame never duplicates a key while a new frame inserts in order; an expressed row is dimmed;
the Timeline opens exactly the asked row, consumes the request, and a second ask never
closes it.

**K-327 · DECIDED · A Project panel item's ring is "Add to comp" — one slice, dimmed when it
cannot run, never the new-layer grab-bag.**
From the owner (2026-08-09): "when you select an item in the project panel, why does it
display the layer types…?? We don't want that, remove those… if it can be added to the
current comp then have that as an option (otherwise have it there so people can get muscle
memory but disable it)". The console had no project-item context at all, so a picked item
fell through to the comp's new-layer ring — six slices with nothing to do with the
selection. Now, **while the Project panel is the active panel** (the console follows where
the user stands, as the keymap's contexts do) and an item is picked there, the ring is a
single slice: **Add to comp**, doing exactly what dropping the item on the Timeline does —
footage becomes a footage layer (honouring K-246's Vegas preference), a composition nests
as a precomp. Per the owner's muscle-memory rule (and K-325's), the slice is **dimmed, never
dropped**, when it cannot run: no comp open, a folder, a solid (no engine path from the
panel yet), or a comp offered to itself, which the engine would refuse — said up front
rather than after the flick.

**The plumbing.** The Project panel's selection stays its own; it now publishes the anchor
item to a `selectedProjectItem` notifier on the shell state on every click, which is also
what puts the item's name in the middle of the ring. A stale handle (the item deleted, the
project switched since publishing) dims the slice and falls through the title rather than
throwing. Regression tests: the panel publishes on click and follows it
(project_panel_frb_test.dart); footage places a layer, a comp nests but never into itself,
the slice dims with no comp open, and the item counts only while the Project panel is the
active one (fx_console_context_frb_test.dart).

**K-328 · DECIDED · While the console is open, the keyboard is the console's: the search box
holds focus for the console's whole life, and every command handler stands down.**
From the owner (2026-08-09), running K-325's build: "the search bar has stopped being
selected by default… if any text is typed when the console is on screen, it is what keys are
put into, so users don't accidentally start opening/editing layers etc". Two faults, one
root: the boxed K-324 console was a movable window, which counted into `lumitModalOpen` —
the flag every panel's hardware-keyboard handler checks (K-243) — and its field won focus as
dialogs do. The K-325 overlay was neither, so the field's `autofocus` lost the race against
the shell's own scope and, with focus astray, keystrokes fell through to the panels' and
shell's command handlers: typing a search renamed and added layers underneath.

So the console now does both things a dialogue does, explicitly. **It counts into
`lumitModalOpen`** (via `markModalMounted`/`markModalUnmounted`, mount-counted for K-243's
stuck-counter reason), and the *shell's* global key handler now honours that flag too —
which it never had, an older gap the console exposed. **And the search field holds focus
deterministically**: focused post-frame on open (`autofocus` races are what failed), then
re-taken the moment anything steals it, for as long as the console is open. There is no
keyboard route out of the console except Escape; the pointer route is a click outside.
`HouseTextField` gains an optional caller-owned `focusNode` to make that steering possible.

**And the console's `Stack` children are keyed, which is load-bearing rather than tidiness.**
Owner-found immediately after the above: typing worked for exactly one letter and then
stopped. The ring is hidden while the query is non-empty, so the first keystroke *removes a
child from the middle of the stack* — and Flutter matches unkeyed children by index and
runtime type, both of these being `Positioned`. The bar's element was recycled onto the
ring's old slot and the field beneath it rebuilt from nothing; a fresh `EditableText` whose
focus node is **already** focused never opens a text-input connection, so every later
keystroke had nowhere to land. Keying each child matches by identity instead, and the field
survives the ring coming and going untouched. The general rule this is an instance of: any
conditional child in a `Stack` whose siblings hold state needs a key.

Regression tests: the field has focus on open and takes it back when unfocused; **typing
keeps going after the ring steps aside** — the field's state object must be the same
instance across the change, and the second letter is delivered through the connection
already open (`updateEditingValue`) rather than `enterText`, which re-attaches one and would
hide exactly this fault (fx_console_test.dart). With the console open the space bar types
instead of playing, and plays again once Escape closes it (shortcuts_frb_test.dart — the
existing Ctrl+Space test now closes the console before asserting the bare space bar).


**K-329 · DECIDED · Curves preview while they are dragged, and a Retime flattened to one
constant is a Retime removed.** Two reports from the 0.2.0 release, one week apart, that turn
out to be the same complaint: the Retime path had no live feedback, and the one gesture that
looked like "take it away" quietly froze the layer instead.

**A graph drag previews.** Every other live drag in the editor already renders its provisional
value through the engine's patched clone (K-192, K-225, K-239, K-240, K-247); the graph editor
— where curves are actually shaped — was the one place that committed on release and showed
nothing before it. It now previews on every tick, throttled and coalescing like the rest
(`previewChannelEdits`, beside the `commitChannelEdits` it mirrors, so the picture during the
drag is made of exactly the scalars the release will write). One layer and one kind of patch
per gesture, because a preview request patches one layer's one state: a selection spanning
several layers, or a transform *and* an effect at once, shows the rest on release as before.
The layer's own Retime map gets a preview door of its own
(`CompositionReference::render_frame_with_retime`), for K-247's reason applied to K-197's
property: a retime decides *which source frame is decoded*, so it cannot be previewed by
re-compositing pixels already in hand. The Retime row's value drag uses the same door, which
retires the "no preview path for that yet" note that sat in it.

**A constant map removes the Retime.** `set_retime_property` given a static value takes the
property away and re-hangs the layer on its source (K-212) rather than writing it. The two
gestures that produce one — the row's stopwatch turned off, and the last key deleted, which
the graph editor answers with a static value — both mean "no more retime"; written as they
arrived they left the layer showing a single source frame for its whole length, with the row
gone quiet and nothing on screen to say why. That is not a state K-197 has ("no freeze"), and
it is the exact bug reported. This narrows K-197's "an ordinary property, the same stopwatch,
nothing Retime-specific": the stopwatch is still the same control, but on this one property
turning it off means what Ctrl+Alt+T off means. A freeze is still reachable and still says so
— a map with one key holds that moment, as After Effects does. Regression tests:
`a_flattened_retime_is_removed_rather_than_freezing_the_layer` (lumit-bridge), which also pins
the one undo step covering removal and re-hang together.

**K-330 · DECIDED · A positional frame lookup must prove the frame is still that position's.**
Reported on 0.2.0: retime a footage layer and the Scopes jump, flicker and match nothing in the
Viewer.

The frame cache names a frame by its **content**, and keeps its **provenance** — the position
and quality it was made for — beside it, because a hash cannot answer "is there any picture of
frame 12?" (K-096, K-183). Two consumers ask exactly that positional question: the Scopes, which
want the numbers in a frame at any resolution, and the dropper. Provenance records where a frame
*came from*, and that never stops being true — but what a position *shows* does change. An edit
renames every frame it touches, so frame 12 renders to a new name while the entry made before it
sits in the map still claiming frame 12. `best_frame` took the finest of the candidates, and
which one that was flipped as the tiers churned under playback: the flicker, and a scope
disagreeing with the picture beside it. A retime made it obvious because a retime changes every
frame of the layer at once.

So both positional lookups now take a predicate and ask each candidate whether its name is still
what that position renders to **at the quality that candidate was made at** — which is why
`FrameProvenance` carries the `Quality` and not only the preview scale it derived from. A stale
entry is passed over, never evicted: its name is still valid content, so an undo that brings the
old picture back finds it in the cache. Nothing current held means the consumer renders its own,
which is the fallback it always had. The predicate runs under the cache lock and is therefore
held to the dropper's rule — bounded, pure CPU, nowhere near the GPU or the FFI boundary
(docs/14). Regression test: `a_frame_the_edit_orphaned_is_not_served_positionally` (lumit-bridge).

**Renumbered on merge, twice.** These two were written as K-256 and K-257 on a branch; the lens-flare work claimed those first, so they became K-268 and K-269 — and main claimed *those* while the branch waited. They are K-331 and K-332 here, and this is the last time: the renumber-on-merge rule K-160 records.

**K-331 · DECIDED · Flow is rebuilt on the render device: GPU synthesis, a cache tier of its
own, a resolution independent of preview quality, and the §3.1 parameters it was always
specified to have.** From the owner (2026-08-04), reopening the flow engine that landed in the
egui era and has not been touched since. The DIS algorithm itself stands (K-169, and
`docs/impl/optical-flow.md` remains the authoritative *how*); everything around it is replaced.

- **One device, one walk.** `FlowEngine::new_auto` built its *own headless wgpu device* inside
  the decode worker, measured flow there, read the field back, and synthesised the in-between
  frame per-pixel on the CPU in sRGB bytes. Flow now runs in `realise`, on the compositor's
  device, where both source textures already exist, and synthesis is a WGSL pass in linear
  premultiplied fp16 as `docs/impl/optical-flow.md` §3 always required. The decode worker goes
  back to decoding: `DrawSource` carries the two bracketing frames and the phase, not
  pre-synthesised pixels. Because preview, the headless renderer and export all drive that one
  walk, K-031 holds by construction rather than by a second implementation agreeing.
- **Flow resolution is its own setting, not a side effect of preview quality.** Flow was
  measured on whatever the preview scale had shrunk the decode to, so a draft scrub and an
  export were different *measurements*, not the same measurement at two sizes. Flow resolution
  moves into `FlowParams` and defaults to native. **The accepted cost:** a layer with Flow live
  decodes at native width even in draft preview, because full-resolution flow cannot be
  measured on a shrunk decode — draft stops being cheap on flow layers, and that is the price
  of a preview that does not lie about what the export will look like. The quality knob remains
  for anyone who wants the speed back.
- **A `flow/` cache tier** beside `frames/` (docs/06 §5.4 reserved it and nothing was ever
  written there), keyed by `(item, frame A, frame B, flow params, algorithm version)` and
  **not** by the preview quality tier, since flow no longer varies with it — so a draft scrub
  warms the cache for the full-quality pass. Fields store as `rg16float` plus an `r8`
  confidence rather than the f32 buffers the CPU parity contract needed (≈18 MB per 1080p
  pair). Retime flow and Fast motion blur hit the *same* entry when they want the same frame
  pair: they are one measurement with two consumers (retime uses the vectors to invent a frame
  between two, motion blur uses them to streak pixels within one), and a layer running both
  paid for DIS twice.
- **Flow is a switch, not a dropdown entry.** Completing K-088: the Source rows' interpolation
  dropdown drops to Nearest / Blend, and Flow becomes a toggle in the footage layer's switch
  cluster which reveals the **Flow** group beside Transform and Effects. `Interpolation::Flow`
  remains the storage (K-088's "the option surfaces the policy"); only the control moves. The
  gate is the K-246 duration rule — media that runs qualifies, so image sequences qualify for
  free the day they become a footage kind, with no flow-specific work.
- **The engagement gate ships, with an override.** K-088's "engages only when it can help" was
  never built; Flow ran whenever selected, paying full cost on clips where it changed nothing.
  Flow now passes through to Nearest unless the source rate through the retime undershoots the
  comp rate, and the Flow group carries a manual override that forces it regardless (the "wind
  toggle" K-095 refers to).
- **The §3.1 parameters ship**: Vector detail, Smoothness, Occlusion handling and Fallback join
  the resolution and the already-built-but-unreachable keyframeable Input rate (K-095, K-160 —
  `set_interpolation` wrote `Interpolation::Flow(Default::default())` and discarded every one of
  them, so two decisions' worth of working engine had no control surface at all). **The
  HUD/overlay guard of §3.1 step 5 ships with them**: static regions with high texture bias
  toward pure blending, which is what stops a game HUD smearing across the frame — the
  single most valuable behaviour for this project's primary footage (K-002).

Superseded in passing: the "flow fields are f32 storage buffers because fp16 rounding would eat
the CPU-parity budget" note of `docs/impl/optical-flow.md` §1 applies to the *search*, which
keeps its f32 working buffers and its CPU oracle; only the *stored* field narrows to fp16.

**K-332 · DECIDED · DIS ships its variational refinement; "skip it in v1" is reversed.** From
the owner (2026-08-04), reporting that the motion vectors are artefact-heavy and the flow and
Fast motion blur that ride on them look poor. `docs/impl/optical-flow.md` §1 step 4 said: *skip
the paper's full variational refinement in v1 — measure first; it is the difference between 2 ms
and 10 ms and mostly helps large untextured regions, rare in game footage.* The measurement has
now happened, and both halves of that sentence were wrong.

DIS is **three** parts — inverse search, densification, variational refinement (Kroeger et al.,
ECCV 2016) — and Lumit shipped two. The paper's own parameter analysis reports that refinement
"always significantly reduced the error for a moderate increase in run-time"; OpenCV's
`DISOpticalFlow`, the implementation everyone benchmarks against, enables it by default. The
quality bar the impl note sets — "≈ Twixtor's easy-80% on game footage" — was set for the whole
algorithm and judged against two thirds of it.

The dismissal of untextured regions was the deeper mistake. Smoke, sky, muzzle flash, water,
darkness and motion-blurred backgrounds are not *rare* in game capture, they are most of a
frame during exactly the fast moments a montage slows down. And the current code fails hard
there rather than softly: densification weights patch votes by a narrow Gaussian photometric
term (σ = 0.08), so where nothing matches, the pixel keeps the coarse initialisation and is
marked invalid; `occlusion` counts invalid as occluded; synthesis then crossfades it. Untextured
regions collapse to patches of ghosted crossfade — the reported artefact, arrived at by three
correct-looking local decisions. The single 3×3 bilateral pass was standing in for the
regularisation the paper leaves to the refinement, and it cannot.

Shipping, per the paper: intensity constancy **and gradient constancy** (the latter is what
survives illumination change — a muzzle flash or explosion is a brightness step across a moving
frame, the case plain intensity constancy has no answer for), a smoothness term, the robust
penaliser `Ψ(a²) = √(a² + ε²)`, solved by successive over-relaxation once per pyramid level.
Validity stops meaning "no patch covered me" and starts meaning "the refined flow does not
explain these pixels", measured from the residual after refinement — a dense field has an
answer everywhere, and the honest question is whether that answer is right.

**Not adopted, and why.** Learned flow is the state of the art — WAFT (2025) leads Spring,
Sintel and KITTI by replacing cost volumes with high-resolution warping, and RIFE-class models
are what the community already pre-processes with. All of them are trained networks, which
collides with three standing commitments: engine determinism (docs/14), preview equals export
(K-031), and no model-file download in v1 (K-169's reasoning, unchanged). One architectural
point decides the shape regardless: **RIFE synthesises frames directly and emits no flow
field**, so Fast motion blur (§3.2) and Datamosh (§3.12) need DIS-class vectors whatever
happens to retime synthesis — a learned model could one day replace the *synthesis* half and
never the *measurement* half. The follow-up the owner accepted is a measurement harness on real
gameplay, so the learned ceiling is judged later against numbers rather than impressions.

**K-333 · DECIDED · Four graph-editor faults from the owner's pass, and a keyed value drag
that showed nothing.** All reported against the 0.2.0 build; each is a fix rather than a change
of intent, so they are recorded together.

**Auto-fit frames what is on screen.** It was fitting over every key of every selected channel
whatever the time zoom, so zooming into a quiet stretch of a curve that spikes somewhere
off-screen still left room for the spike and the part under the pointer stayed a flat line. The
fit now takes the keys inside the visible time window, plus what each curve reads at the two
edges of it — the edges being what stops a span *between* two keys from framing on nothing.

**"No other scroll works until I press Alt again" was never the Alt key.** The first reading was
a stale modifier — Alt is Windows' menu-activation chord, so the key-up that ends an Alt+wheel
zoom is easily lost — and the handler does now ask the platform what is really held. But that
was not the fault. Alt+wheel multiplies the vertical span by 1.2 a tick and nothing bounded it,
so half a second of scrolling gave a range hundreds of times the curve and a few seconds gave
millions. Nothing about the pane then looks broken; it looks *dead*. The curve is far outside
the window, a pan of one wheel notch moves it by a fraction of a span nobody can see, and only
another Alt+wheel — being multiplicative — can climb back, which is exactly what "press Alt
again and it works" was describing. The vertical range is now held finite, the right way up, and
within a thousandfold of what auto-fit would choose; the zoom's anchor is clamped to the pane,
because the pointer signal is reported against a listener taller than the graph and an anchor
from outside it zooms about a value nowhere near the curve.

**The magnet snaps the picture, not only the write.** A key drag rounded to whole frames on
release and drew unrounded until then, so a key bound for frame 12 sat between 11 and 12 for
the whole gesture and jumped on the way out.

**`Shift` lays a tangent handle flat**, holding the value at the key's own so the curve leaves
it horizontally; a joined partner is mirrored from the dragged side and comes flat with it.

**A value drag in the layer area previews, and the graph follows it.** The picture is rendered
through the same patched clone a static drag uses (K-192), carrying the whole animation. The
curve is a second problem with the same cause — the pane draws from the read model, and the
provisional value lives in the row's own state until the release — so the row **publishes** it
(`rowValueDrag`), exactly as a bar drag publishes its travel for the waveform lane
(`BarDragPreview`, K-172), and the pane draws through it. Matched by layer and *axis*, so
dragging Position x leaves y where it is.

Two rules about keys go with it. An **animated** property with no key under the playhead gains
one the moment the drag starts, holding the value already showing — nothing moves, and the drag
then has a key to carry, which is what makes it visible in the graph as it goes. An **unkeyed**
property is drawn at its new value and gains no diamond: the drag is not planting a key, and a
glyph would say it was.

**K-334 · DECIDED · A press on a row's controls selects the row, and Alt is asked of the
operating system.** Two more findings from the owner testing K-333, both of which turn earlier
fixes from nearly-right to right.

**Selection.** K-196 put selection on the property's *name*; every other press on a row — the
stopwatch, the ◄ ◆ ► navigator, the value field — acted without choosing. That is why the graph
still did not follow a value drag after K-333 wired the preview: the pane draws **selected**
channels, the drag was on an unselected row, and the channel it should have moved was not on
screen at all. Any unmodified pointer-down on a property row now selects it (replace, not
toggle — `_selectOnEdit`'s behaviour), on pointer-DOWN so the channel exists before the drag's
first tick. Modified presses keep the label's Ctrl/Shift semantics; group headings keep their
pick-and-twirl click (K-300). Extends K-196.

**Alt.** K-333's second reading (the unbounded zoom) was real and stays, but the first reading
was righter than its fix: Alt genuinely sticks, and `syncKeyboardState` cannot unstick it
because it re-asks the same embedding that missed the key-up. `altActuallyHeld()` asks the OS
(`GetKeyState`) — only ever to clear a false positive, trusting the framework when it says Alt
is up, and trusting simulated modifiers under `flutter test`. Used everywhere the graph gates
behaviour on Alt: the wheel zoom, Alt-click key removal, and handle break/join — a stuck Alt was
also silently deleting keys on plain clicks and flipping every handle drag to broken.

**K-335 · DECIDED · The Alt witness is `GetAsyncKeyState`, and every value row publishes its
drag.** The owner's third report of both K-333 bugs, and this time the mechanisms rather than
more wiring.

**Alt.** K-334's `GetKeyState` was the right idea asked of the wrong thread: it reads the
keyboard state of the *calling thread's message queue*, and Dart's UI thread is not the Win32
thread that receives keyboard messages, so its answer was as stale as the framework belief it
was meant to correct. `GetAsyncKeyState` reads the physical key state whoever asks. Same
guardrails: only ever clears a false positive, and trusts simulated modifiers under
`flutter test`.

**The graph follow.** The transform rows were wired and the effect parameter rows were not —
and a value being dragged in the layer area is as often a blur radius as a Position. The
published drag (`rowValueDrag`) grows selectors for all three channel kinds — a transform axis,
an effect parameter, the Retime — and every keyed row publishes: transform axes (K-334), effect
parameters (this entry, with the drag-start key plant and the staged-stack picture preview the
transforms already had), and the Retime row. Three end-to-end regression tests drive the real
outline field with a held-down gesture and watch the graph: a drag on a key, a drag *between*
keys (the key plants at drag start and is carried), and a drag on an effect parameter.

**K-336 · DECIDED · The dead scrolling was the Windows menu loop, and the drag preview matches
keys by half a frame.** The owner's fourth report of the Alt bug, and the one that ends it —
because this time the mechanism was reproduced in a clean-room probe app before the fix was
written, and the fix was proven against the same probe.

**It was never a modifier.** Releasing a *lone* Alt makes DefWindowProc enter the modal menu
loop (`WM_SYSCOMMAND`/`SC_KEYMENU`): a loop inside Windows itself that swallows every wheel —
plain, Ctrl and Shift alike — and keyboard input, until Alt is pressed again, Escape is hit, or
the window is clicked. A key press between Alt going down and up cancels the request, but a
wheel tick does not — which is why exactly Alt+wheel (the graph's vertical zoom) left scrolling
dead while every ordinary Alt shortcut was fine, and why "press Alt again" fixed it. The probe:
a bare Flutter app whose posted probe-key vanishes after `SC_KEYMENU` and returns after an Alt
press — and stops vanishing entirely with the fix in. The fix is in the runner
(`win32_window.cpp`): `SC_KEYMENU` returns 0, because Lumit's menu bar is Flutter-drawn and
there is no native menu for the chord to open. K-334/K-335's stale-modifier readings were
wrong about the cause; `altActuallyHeld()` stays, as a harmless guard that only ever clears a
true false-positive.

**The drag preview replaces keys within half a frame.** The published row drag swapped its
value into the curve by *exact float* frame equality, and a key's frame comes back through
rational-to-double maths that does not always land on the integer — so the drag's key could be
inserted beside the document's instead of replacing it. One extra key shifts every later glyph
index: the dragged key drew at the next key's place and everything after it sat one key off
until the release rebuilt from the document. The preview now replaces the nearest key within
half a frame, keeping list length and order stable, and the between-keys regression test pins
the glyph count and the immobility of the keys after the playhead.

**K-337 · DECIDED · A glyph reads both its coordinates from one list.** The screenshot that
closed the drag-preview saga: drag the Retime readout on a frame with no key and the diamonds
floated off the curve. K-336's half-frame match fixed replacement, but a keyless frame takes the
*insertion* path — the preview list is one key longer than the document's — and `_keyPoint` read
x from the document's keys while `_keyY` read y from the preview's, so every glyph past the
insertion drew with one key's x and another's y. Both now read the same `_shownKeys` list in
every lens, with an index guard. The Retime row also plants its key on the drag's first tick,
as the transform and effect rows already did (K-333's rule), so the ordinary gesture takes the
replacement path anyway and a diamond stands at the playhead from the first tick. Regression:
`a Retime drag on a keyless frame keeps the diamonds on the curve`, which fails on the mixed
lists and on the missing plant alike.

**K-338 · DECIDED · Masks gain modes, feather and expansion, and the first mask in the
list decides what the fold starts from.** 03-DATA-MODEL §7 always described a v1 mask as
"static, Add-mode" with the rest listed as future. The future is now partly here:
`MaskMode` is `None | Add | Subtract | Intersect | Difference`, and every mask carries a
`feather` and an `expansion` in layer pixels. Lighten and Darken are deliberately not
built — they are max and min over overlapping opacities, and nobody has asked.

**Combination is now sequential, and it had to become so.** `combined_coverage` summed
every mask's coverage and clamped, which is order-independent and correct precisely
because everything was Add. Subtract and Intersect are not commutative, so masks now fold
top to bottom in list order, which is what 06-RENDER-PIPELINE §3 always said they should.
Add's expression is unchanged, so an all-Add project is bit-identical to before.

**The first mask needs a starting value, and the honest one depends on its mode.** With
the accumulator starting at zero, a lone Subtract subtracts from nothing and a lone
Intersect intersects with nothing — both give an empty frame, which is not what anyone
drawing a single subtract mask means. So the fold starts from zero when the topmost
non-`None` mask is Add, and from full coverage otherwise: a lone Subtract cuts a hole, a
lone Intersect shows just its own shape, a lone Difference shows its inverse. This is
After Effects' behaviour and it is the only reading under which the first mask does
something rather than nothing.

**Feather and expansion are one mechanism, not two.** The obvious build is a blur for
feather and a morphological grow/shrink for expansion. Instead the rasterised coverage
becomes a signed distance field once — an exact Euclidean transform, Felzenszwalb and
Huttenlocher, seeded from the antialiased edge so it keeps sub-pixel placement — and both
controls read off it: expansion shifts the zero crossing, feather sets the width of the
ramp across it. One pass, and it is what "feather in pixels" actually means, measured
along the surface normal rather than approximated by a blur radius. The cost is that an
expanded or eroded shape rounds its corners, because distance is measured to the nearest
point on the path; that is also what After Effects does.

**With both at zero the rasteriser's bytes are returned untouched** — no distance field,
no allocation. That is what keeps every existing project bit-identical, and it is why the
new fields also serialise only when they differ from their defaults: the frame cache key
hashes the serialised masks, and always emitting them would have retired every frame
every existing project has banked.

**Variable-width (per-point) feather is not built.** It is a second point set on the path
with its own tool, and `ToolMode.penMaskFeather` already exists in the toolbar as a stub
with nothing behind it. It stays in TODO.

**K-339 · DECIDED · A mask path animates through its own keyframe list, and mismatched point
counts resample upward.** From the owner (2026-08-08): the deferral K-224 recorded ("neither
can a mask path be keyframed") is closed for the engine half.

**A separate carrier, not a generic `Property`.** Every animatable value in Lumit is a scalar:
`Animation::{Static, Keyframed, Expression}` behind a `Property` whose `value_at` returns one
`f64` at roughly two hundred call sites. A shape is not a scalar, and making `Property` generic
to hold one would churn all two hundred to buy nothing. So `Mask` carries `path_keys:
Vec<PathKeyframe>` beside its `path`, in `lumit-core::mask` where the path type already lives.
Empty means unanimated, and empty is omitted from the file — an untouched mask writes the exact
bytes it always did, which is what keeps every frame every existing project has banked
(`lumit-eval` hashes the serialised masks into the frame key).

**Timing eases, no value graph.** Path keys carry the same `SideInterp` pair a scalar keyframe
does, and it shapes the **interpolation parameter** — 0 at this key, 1 at the next — evaluated
by the scalar evaluator itself rather than a second copy of the same maths. A shape has no
value to plot, so the lane shows diamonds only; the graph editor's speed lens is the TODO item
that pairs with this.

**Mismatched vertex counts resample to the higher count, never refuse.** Adding a point to a
mask halfway through an animation is an ordinary act, so interpolation between a four-point key
and a seven-point key must simply work. The sparser path is redrawn at the higher count by
**splitting its own segments** — de Casteljau at a parameter, the same exact split K-221 relies
on, so the two halves *are* the original cubic and the reconciled path is geometrically the
path it was. Distribution is fixed arithmetic (evenly, remainder to the earliest segments), so
the reconciliation is deterministic and playback repeats frame for frame. Then the two run
vertex for vertex, position and both handles blended straight. This is what After Effects does,
so an imported comp and a hand-built one behave alike.

**Open against closed is held, not blended.** Whether a path is joined up is not a quantity and
has no halfway. Across a span it takes the outgoing key's flag and flips at the next key — a
Hold in all but name. The geometry still interpolates; only the closing segment appears or
disappears, on a frame boundary rather than smearing.

**The frame cache needs the evaluated path, not the stored keys.** The key carries no timeline
position by design (K-214), and a keyframed mask serialises identically at every frame — so the
stored keys alone would name every frame of a moving mask the same, and playback would hand
back the first frame drawn while the mask sat still. The evaluated shape at the layer's local
time therefore joins the hash, **and only for masks that are actually animated**, so no
existing key moves and `ALGO_VERSION` does not need bumping. Masks evaluate at the layer's own
clock, the one every other property on the layer reads (K-213).

**Still whole-list ops.** `SetLayerMasks` carries the entire mask list, so a keyframe drag
rewrites all of it as one undo entry. That is correct but coarse; a per-key op is noted in
TODO.md for when the interface can make one.

**K-314 · DECIDED · The Viewer gets an exposure and a tone map that the export can never
see, and the tone map is a fixed highlight rolloff.** Two controls in the Viewer bar
(07-UI-SPEC §2.2), both preview only, both inside the display transform where
06-RENDER-PIPELINE §3.3 already reserved room for exactly this. Exposure reads signed
stops to one decimal and means what the Exposure effect means — the same `2^stops`
scene-linear gain, computed host-side, so the two agree by construction (K-106). The tone
map is an icon toggle with a tooltip and no menu.

**The curve is a knee at 0.8 and an exponential shoulder above it**, applied to luminance
so hue and saturation stay where the author put them:
`knee + room · (1 − exp(−(L − knee) / room))`, `room = 1 − knee`. Its slope at the knee is
exactly 1, so the join is smooth, and it approaches 1 without reaching it, so no highlight
clips flat however bright. Below the knee it is the identity, exactly.

**That identity is the reason for the choice.** Reinhard darkens mid grey by about 15%,
the ACES fits impose filmic contrast across the whole range and carry the familiar hue
skews, and AgX — the best-looking of them on genuinely blown content, and Blender's
default — is a *look*: it moves mids and saturation on a composite that never exceeds 1.
Any of those makes turning the toggle on change a picture that had nothing wrong with it,
which reads as the Viewer lying about the export. The rolloff cannot: on an ordinary
composite it does nothing at all, and on one running past 1 it shows what is up there.
AgX remains the right answer later as a *selectable* transform when OCIO lands, and the
tone-mapping **effect** (08-EFFECTS, still unbuilt) must share this curve, so this entry
binds twice.

**"Auto" was asked for and is not what shipped.** A measured, time-smoothed exposure makes
the displayed frame depend on which frames preceded it: scrubbing back to frame one shows
different pixels than frame one showed, which breaks the cache tier outright and puts a
clock in the pixel path that 14-ENGINEERING-RULES §7 forbids. Unsmoothed measurement is
worse — the picture pumps on every cut. No compositor's viewer adapts per frame; After
Effects, Resolve and Blender all apply a fixed transform. So the control is named "tone
mapping", because nothing about a constant is automatic, and the word "auto" is left free
for the day a measured white point is genuinely built.

**Export is neutral by construction, not by discipline.** `DisplayParams` defaults to
neutral on `HeadlessRenderer`, the shader short-circuits on it so a neutral pass is
bit-identical to the plain copy it replaced, and an export builds its own renderer that
nobody ever calls the setter on. The K-031 promise — the Viewer at full resolution is the
export — survives as "the Viewer at full resolution and neutral view".

**A non-neutral view makes a frame unnameable**, so nothing rendered while a control is
engaged enters any cache tier and the neutral frames already banked stay banked, returning
as hits the moment it goes back to neutral. Widening the frame key through three tiers
would have been the alternative; this costs a cache miss while the control is engaged and
cannot mis-serve an exposed frame to something expecting the composite.

**The settings persist per composition, in the project, through `ui_state`** — the blob
K-245 already writes into the `.lum`, which is not undoable and does not mark the project
dirty. A way of looking is not an edit to the work, so Ctrl+Z must never undo an exposure
nudge, and a comp reopens looking how it was left.

**K-340 · DECIDED · Every one of a mask's values animates, and a still mask still writes
bare numbers.** From the owner, testing K-338 in the app (2026-08-10): "currently no mask
property has the clock icon to enable keyframing. This should be the same as any
transform/effect etc. and all the properties should be able to be keyframed." K-339 had
given the *path* its keys and left the three numbers static, and had exposed neither to the
frontend — so the branch claimed keyframing that nothing in the interface could reach.

**Opacity, feather and expansion become ordinary `Property`s**, not a second key-list
carrier beside each value. K-339 argued the other way for the *path*, and that argument
still holds there: a shape is not a scalar, and making `Property` generic to hold one would
churn two hundred call sites to buy nothing. A number is a scalar. Making these three what
every other animatable number already is means the Timeline row reuses the stopwatch, the
◄ ◆ ► navigator, the keyed-value field and the lane diamonds exactly as they stand —
"the same as any transform/effect" is then true by construction rather than by a parallel
implementation that would drift.

**The file keeps its old shape while the mask is still.** A `Property` normally serialises
as an object, and switching to that would have migrated every `.lum` ever written and —
worse — retired every frame every project has banked, because the frame key names a mask by
the bytes its list serialises to. So the three fields carry their own encoding
(`still_or_keyed`): a static value writes as the bare number it always wrote, and only a
mask somebody has actually keyed grows the object. Reading takes either. An unkeyed mask is
therefore byte-identical to what it was, which is the same promise K-338 and K-339 each
made and is why the cache survives all three.

**The frame key learns the evaluated numbers, for the same reason it learned the evaluated
path.** A keyed opacity serialises identically at every frame while the key deliberately
carries no timeline position of its own (K-214), so without the value at the layer's local
time a moving mask would name every frame alike and playback would hand back the first one
drawn. Fed only for properties that actually hold keys.

**Clamping an animation clamps its keys.** The bridge has always held a mask's opacity into
0..100 and its feather and expansion into a sane span rather than trusting the frontend. A
key three seconds away at −40 % is exactly as wrong as one now, so the clamp walks the
keyframes rather than the value under the playhead.

**Opacity moves off the mask's header onto a row of its own**, joining Path, Feather and
Expansion. A property with no row has nowhere to put a stopwatch, and the header now carries
only what the mask *is*: its name, its invert switch and its mode.

**The shape's row keys through its own ops.** `toggle_mask_path_key` and
`clear_mask_path_keys` plant, remove and stop — a key holds the shape the mask is *already*
showing at that moment, so pressing ◆ never moves anything, and switching animation off
keeps the shape under the playhead rather than snapping to the first key. That is what the
stopwatch does everywhere else. `MaskPathKeyframesFrb` sits in the same file as the scalar
controls so the two cannot drift into different ideas of what a diamond means.

**Two switches mean a mask does nothing, and neither hides the layer.** Mode `None` and
opacity zero both used to blank the layer outright when the mask was the only one on it —
the fold started from an empty stack and then skipped the very mask it had started from.
`Mask::does_something_at` is now the single question every caller routes through, and it
takes a time because opacity animates: a mask keyed up from zero is off for the first half
of the shot and on for the second.


**K-341 · DECIDED · A picked property row is a picked layer everywhere else, and a mask's
rows behave like every other property row.** From the owner, testing K-340 (2026-08-10):
"why tf if I click a mask row it doesn't just select it. Please can you treat all property
rows the same in this regard, between transform/effects… whenever I add a keyframe it
doesn't display it in the lane area… i can't view any of the mask properties in the graph
view."

**The selection was never the problem; everything downstream of it was.** Clicking a mask
row did pick it — the row's fill said so — but nothing else in the program knew what to do
with the pick. `laneKeysOf` had no mask arm, so a key planted by the stopwatch drew no
diamond; `graphChannels` had no mask arm, so the graph editor had no curve to show and read
as "the row cannot be selected"; and the property selection never left the Timeline at all,
so the Viewer outlined nothing. Three separate silences that added up to one apparently
dead row. Mask rows now answer all three, through the same functions the transform and
effect rows already go through rather than through a mask-shaped path of their own.

**The shape's lane shows diamonds without a curve.** A path key holds no number, so it
cannot be a graph channel (K-339 already said so) — but it *can* be a position on a lane,
and a key the author just planted must be visible or it reads as not having landed. The
diamonds are built from the key times alone, and dragging one goes through a dedicated
`move_mask_path_key` rather than the scalar path, because what is moving is a whole shape.

**Picking a property says which layer is being worked on, and the picture says so too.**
`selectedProperties` is published from the Timeline to the shell, and the Viewer outlines
the layers those rows belong to — wireframe and masks — exactly as it does for a layer
picked on its own row. Drawing only: what can be *dragged* stays the layer selection
proper, so an outline never turns into a handle nobody asked for.

**A mask's Path row is the shape being edited.** Picking it offers that mask's points for
dragging without the layer having to be clicked first, and the reverse holds too: dragging
a keyed mask path selects that Path row, so the key the drag just wrote is on a row the
author can see. The two directions are one idea — the row and the shape are the same thing
seen from two panels.

**Both of the mask's own switches move into the value column.** The invert mark and the
mode picker sat beside the name, in no column at all; K-340 had put the mode under the
blend header on the grounds that it is the same kind of choice, and the owner's answer was
that consistency down the *fold-out* matters more than consistency across to the layer row.
The mode picker takes the rest of the cell so a long name ellipsises rather than pushing
the row wider than its column, which is the rule the blend picker already followed.


**K-342 · DECIDED · The wireframe of an animated mask is drawn from the shape it is
showing, asked of the engine.** From the owner, testing K-340/K-341 (2026-08-10): "after
you move the path in the viewer, visually it snaps back to its original position, but it
adds the keyframe correctly and when you preview it animates correctly."

**The picture was right and the outline was stale.** Once a path is keyed, `Mask::path` is
no longer what the mask draws — `path_at` reads the keys — but the Viewer's wireframe was
drawn from the vertices the mask carries, which are exactly that stale `path`. So a drag
wrote its key, the render animated, and the outline sprang back to where the shape began.

**Evaluated engine-side, not in Dart.** Dart samples ordinary scalars itself and could have
been given the keyed *shapes* to interpolate — but interpolating two paths means
reconciling their vertex counts by splitting cubics (K-339), and a second implementation of
that here would drift from the one that draws the pixels. A wireframe that stops matching
the mask it describes is worse than no wireframe. So `animated_mask_paths_at(frame)` asks
the engine, which answers with the same `path_at` the renderer uses.

**Only animated masks are listed, and the answer is held.** A still mask's own vertices
already say where it is, so the ordinary composition answers with an empty list and pays
nothing. The Viewer rebuilds on every movement of the pointer, so the answer is cached
against the document revision and the playhead frame — the two things that can change it —
and a hover asks nothing. That keeps K-184's budget intact: the hover test still measures
zero.

**K-343 · DECIDED · A property row takes the press across its whole width, not only where a
widget happens to sit.** From the owner (2026-08-10): "when I click the path row, it
deselects and can't be re-selected by clicking like it should."

The fold-out's rows select on pointer-down through a `Listener`, and a `Listener` defers to
its children by default — so a press only counted where it landed *on* something. A
property row is mostly empty: the label stops where its text stops, and the value column is
one narrow field in a wide cell. A press in the space between reached the outline behind
instead, which is the surface that **clears** the selection — so clicking a row could
unpick it, and clicking again did the same thing rather than picking it back.

Worst on a mask's **Path** row, which by design has no value field at all (a shape has no
number to put in one, K-339), leaving almost the whole row dead to the pointer.

The rows that select are now opaque to hit testing, so the press lands on the row wherever
it falls. Group headings keep defer-to-child: their own detector owns the click, and a
heading both picks and twirls (K-300).

**The graph editor showing nothing for a Path row is not this bug.** A path has no value
axis, so it is deliberately not a graph channel — its keys live on the lane as diamonds.
Easing them wants the speed lens K-339 already recorded as outstanding. A mask's opacity,
feather and expansion *do* resolve into channels and draw curves.

**K-344 · DECIDED · A keyed mask shape draws its rate of change, in both lenses.** The
deferral K-339 recorded — "a keyframed mask path shows a speed graph and no value graph" —
is closed. From the owner (2026-08-10): opening the graph on a Path row showed an empty
pane, which reads as a property that is plainly animating having nothing to say.

**What a path can honestly plot.** A shape has no number, so there is no value curve. What
there *is* is the crossing from one keyed shape to the next, shaped by the ordinary eases
K-339 gave those keys — and the rate of that crossing is a real, meaningful curve. So the
shape's keys now carry a **counted-up interpolation parameter**: key *i* holds *i*, and
every span therefore rises by exactly one. The number is not worth reading; its slope is
the whole point, and it is what After Effects draws for a mask path.

**Both lenses draw the slope.** The value lens would otherwise show a meaningless staircase
and the speed lens the useful curve — one of the two views blank or misleading for no
reason. A shape channel is therefore drawn in the speed reading whichever lens is on, and a
pane holding only shapes fits its axis to speeds. Every other channel still follows the
view's lens exactly as before.

**The keys cross with their eases, and edits go back the same way.** `BridgeMask` carries
`path_keys` (time, counted value, both `SideInterp`s) rather than bare times, which is what
lets the lane draw its diamonds *and* the graph draw the curve from one read.
`set_mask_path_keys` writes a whole re-timed, re-eased list back — refused outright if the
times are not strictly ascending, because the evaluator walks them assuming so and a
half-applied reorder is not a mask. The shapes themselves never cross: a key holds a path,
which the drawing tools edit (K-339).


**K-345 · DECIDED · "Clip" is restricted as a noun, not as the keying/colour verb.** From
the owner (2026-08-11), resolving a question the 2026-08-10 audit raised: the Matte key
ships Keylight's control names — Clip black, Clip white, Clip rollback — and glossary §9's
"clip only inside Sequence layers" reads as if it forbids them. It does not. The
restriction is about the noun (a clip is an entry inside a Sequence layer, never a general
word for a layer or footage); *to clip* in its keying and colour sense — clipping a matte,
clipped highlights — is ordinary trade language and keeps its names. §9 now says so.

**K-346 · DECIDED · A Viewer look names its frames apart; it no longer switches the caches
off.** From the owner (2026-08-11), superseding the naming half of K-314. K-314 made a
non-neutral view leave frames **unnameable**, so nothing entered any cache tier while an
exposure or the tone map was engaged — the reasoning being that it was cheaper than widening
the key through three tiers and could not mis-serve an exposed frame. In use that reads as a
fault: the owner worked with the tone map on, found the VRAM and RAM meters sat at zero all
session, and nothing on screen said why (the §2.2 colour-management badge is still unbuilt).
A way of looking that silently disables the whole cache ladder is not a preview convenience.
So the look is now folded into the frame's name instead: `HeadlessRenderer::named_under_view`
hashes the exposure gain and the tone-map flag into the content name, under its own tag, with
the same blake3 the name was built with — deterministic across runs and toolchains, which the
disk tier needs since it keeps names between sessions. **Neutral is untouched**, byte-for-byte
the name it always had, so every frame already banked stays a hit. Each look therefore banks
its own frames and changing a control retires nothing but takes a fresh set of names, which is
honest: those are different pictures. The mis-serving worry K-314 raised is answered by
construction rather than by refusing to cache — an export is neutral by construction (see the
export test), so a graded preview frame can never collide with one. The cost is that dialling
an exposure through several values leaves several sets of banked frames competing for the same
budget; the tiers evict by the usual rule and nobody has to think about it.

**K-347 · DECIDED · The tone map button is asked for, not given.** From the owner
(2026-08-11), refining K-314's presentation without touching what the feature does: "it just
doesn't seem like a feature most people need or want, but I think it's neat and nice to
have." The Viewer bar's tone-map switch is therefore **hidden by default** and revealed by
**Settings → Interface → Show the tone map button**. The **exposure field is not hidden** —
stops are an ordinary photographic control that people reach for; tone mapping is the
specialist one.

**Hidden means off, not merely invisible.** The setting gates `LumitUiState.viewerLook`, the
one place the per-composition store becomes the look in use, so the Viewer bar, the engine
push and the button cannot disagree, and a session saved while the tone map was engaged
cannot come back stranded — an engaged look with no button to turn it off would change what
the Viewer shows with nothing to explain it. Only the *reading* is gated: the per-comp store
keeps its value, so turning the setting back on finds each composition as it was. (Moving the
exposure while the button is away writes the pair back as it reads, which clears the stored
tone map — a state you cannot see does not persist behind your back.) Recorded in
[07-UI-SPEC.md](07-UI-SPEC.md) §2.2, which is where the Viewer bar is specified.

**K-348 · DECIDED · A shaped ease is drawn once in a unit box and stamped span by span,
from the value lens only.** K-196's graph editor shapes one span at a time, by dragging the
tangent handles of two particular keyframes; the footer's Linear / Bezier / Hold buttons go
the other way and set a *constant* on every selected key. Neither covers the common job:
one hand-drawn ease put on a great many keys at once. The **Easing…** button opens a
normalised cubic — the four numbers CSS writes as `cubic-bezier(x1, y1, x2, y2)` — with a
row of preset shapes, and Apply stamps it.

**(1) The unit of work is a span, not a key.** A shape describes the travel *between* two
keys, so a span takes the curve when both of its ends are selected; a lone key has named no
travel and is left alone. This is deliberately unlike the one-click three, which are key-wise,
and it is what makes selecting a run of keys ease the whole run.

**(2) Each span converts against its own chord slope.** A keyframe side stores AE-style
*speed* (value-units per second) and *influence* (a fraction of the gap) — `anim.rs`. Speed
is an absolute rate, so the identical drawn shape must become a **different** stored speed
on a span covering 400 pixels than on one covering 40, or only one of the two would look
like the curve that was drawn. Influence is already a fraction and carries across untouched.
`EasingCurve.sidesFor` is that conversion, derived from the control-point placement in
docs/impl/keyframe-eval.md §1, and it is the whole reason the shape is held apart from any
one span. A flat span has chord slope 0 and stays flat whatever the shape.

**(3) Value lens only, locked twice.** The button is absent while the speed lens is up, and
`_applyEasing` refuses the call as well. The box draws a shape against **value** travel, so
a curve stamped from the speed lens would edit a graph the user is not looking at. The
one-click three stay in both lenses: a side's interp means the same thing either way.

**(4) The presets are named for which end is slow.** *Slow start* / *Slow finish*, not
"ease in" / "ease out": in Lumit "in" and "out" already name the two **sides** of a key
(F9's family), while the web's `ease-in` means a slow *start* — the opposite reading. Two
presets leave the box on purpose (Overshoot, Anticipate), which is what sizes the editor's
vertical margin: a handle drawn past the edge of the view is one the pointer cannot reach
to drag back.

**K-349 · DECIDED · The easing editor is a panel, and the popup is the setting.**
From the owner (2026-08-12), revising K-348's shipping form before it reached anyone.
K-348 put the shape editor in a popup opened from the graph footer. Every popup here
closes on a click outside it — and **choosing different keyframes is a click outside**, so
a shape could only ever be tried on the selection that happened to be live when the box
opened. That is the opposite of what a reusable ease is for: the whole value of drawing one
shape is putting it on this run of keys, then that one. The editor is now the **Easing
panel**; `EasingEditor` is one widget and the popup is the same widget in an overlay.

**(1) The panel is the default; the popup is a preference.** Settings ▸ Interface ▸ Editing
▸ *Shape eases in a popup* (`easingInPopup`, off) restores the K-348 behaviour for a small
screen, or for anyone who would rather not spend a column on it. Phrased as a deviation
from the default, like K-254's playhead and K-285's waveforms, so a settings file written
before the field existed adopts the panel by its own silence.

**(2) It is not in the default arrangement, but it has an arrangement of its own.** Adding
a pane to `defaultLayout` would rearrange the first-run screen for a panel most projects
never open, so the four shipped presets are untouched. A fifth preset, **Retiming**, gives
Easing the right-hand column outright — not tabbed behind Scopes, because a panel behind a
tab is a panel you keep fetching, which is the popup's problem again in slower form — over
a Timeline as tall as Audio's. Everywhere else the graph footer's **Easing…** button docks
it on first press and fronts it thereafter (`setPanelVisible` is a no-op when it is already
there), and Window ▸ Easing ticks it like every other panel. This is the first panel not
present in every arrangement; `dock_test.dart` names it as the single exemption rather than
loosening its invariant to "some panels are missing".

**(3) The panel never learns what is selected.** It publishes nothing and asks nothing: the
Timeline hands the shell a callback (`LumitUiState.easingApply`) while it can take a shape,
and the panel presses it. That is K-234's and K-300's claim idiom for Delete, Copy and
Paste, and it keeps the keyframe selection the Timeline's alone. The one difference is that
this claim is a `ValueNotifier` rather than a bare field, because it is *read to draw
with*: null — no Timeline on screen, or a graph in the speed lens (K-348) — greys the
panel's Apply and shows the reason. A popup that simply vanished could stay silent about
this; a panel sitting in the corner with a live-looking button that does nothing cannot.

**K-350 · DECIDED · The lens flare's bake runs beside the frame, and a frame that drew a
lens it does not name is never banked.** K-263 measured the flare's one blocking CPU step
at about 0.66 s and recorded the fix as owed: choosing a lens stopped the picture for half
a second, because the bake was a closure the render thread ran inside the frame. It now
runs on a **bake thread** of its own (`lumit-gpu`'s `LensFlareFx`), and a frame that asks
for a lens the engine does not hold draws the lens the previous frame drew — or, with none
yet, no flare at all — so the freeze becomes a wait you can watch. The new optics are
picked up by the next frame after they land, and the worker makes that frame itself
(`republish_after_bake`), so the picture catches up without the user touching anything.

Three properties make it safe, and each is the reason for a piece of the design.

**(1) An export never draws a provisional picture.** Deferring is *off* by default and only
the Viewer's renderer turns it on; the exporter builds its own renderer on its own device
(`export::run`) and nobody calls it there, so the safe behaviour is what a path gets by
forgetting to choose rather than by remembering to. K-031's preview-equals-export identity
is untouched, and an export is bit-for-bit what it was.

**(2) A provisional frame is unnameable.** The three cache tiers are keyed by a hash of
what is *in* a frame (K-178), so a frame drawn with the previous lens and filed under the
new lens's name is an entry that lies about itself and outlives every edit and undo that
might have fixed it. `frame_key` therefore answers `None` while a bake is in flight — the
same mechanism unprobed footage and a non-neutral display view already use — and, because
a bake can also be queued *during* a render, the naming is checked either side of it
(`flare_bake_generation`). The idle cache fill stands down while a bake is in flight for
the same reason, and starts again when it lands.

**(3) The bake is still pure, and cancellation is exact.** The bake is the same function of
the same key wherever it runs, so the frame that finally shows the new lens is the frame
the old code would have drawn. Superseded keys are dropped **before** they start: a bake is
named by a hash of the parameters that made it, so dragging the f-stop queues a key a tick
and only the last is worth half a second of optics — the rest are answered with nothing and
taken off the in-flight list, which is what stops an abandoned lens leaving every frame
permanently unnameable.

Not fixed here, and unchanged: the flare's raster still draws the cells it culled, and the
`wgsl_lens_flare_matches_the_cpu_frame_reference_and_neutrals` bit-stability question is
still open (both remain in TODO).

**K-351 · DECIDED · Opening a project shows one loading card and nothing else, and no
shader a project does not use is compiled before its first frame.** Two halves of the same
complaint — the first preview took the better part of ten seconds, and the interface spent
that time looking loaded but empty.

**The measurement first**, because the cause was not where it looked. On this machine a
render worker took **7.7 s to start**, and 6.5 s of that was `LensFlareFx::new`: the flare's
ray tracer is the largest shader in the program and its three pipelines dominate every other
kernel put together (`crates/lumit-render/examples/first_frame_probe.rs` is the probe that
says so). Every project paid it, including one with an empty composition and no flare
anywhere in it, because the worker built every pipeline before answering its first request.
The flare's pipelines are now built on a **thread of their own** (`LazyFlare`) and joined by
the first frame that actually draws a flare — which, by the time anyone applies one, has long
since finished. Worker start is 1.1 s. Nothing about what the effect draws changes: this is
only about *when* the compiling happens, and the bake-deferral machinery of K-350 is
untouched — the flag set before the pipelines exist is applied when they do.

**And the reading of the document is off the drawing thread.** `open_project` is no longer
`#[frb(sync)]`: parsing a `.lum` and stating every media file it names froze the window for
as long as it took. It now runs on a worker thread, which is what makes the second half
possible.

**One swap, not a fill.** While a document is being read *and* until the Viewer has
something to show of it, the shell stands behind a single card with a progress bar
(`OpeningOverlay`) and the panels behind it still hold the previous project. Panels that
filled one by one while the picture was still coming read as a slow editor; everything
appearing at once reads as an application loading, which is what it is. The card lifts on
the first reply from the new project's worker — any reply, not the frame alone, so a first
render that faults cannot leave the interface covered — or, for a project that fronts no
composition, as soon as the session restore says there is no picture to wait for.

Not fixed here: the 1.1 s that remains is the graphics device (0.6 s) and the other engines
(0.5 s), and a project's own footage still probes on first use.

**K-352 · DECIDED · The transparency grid actually sees transparency: while it is up, the
Viewer's renderer leaves the comp's background colour out of the composite.** docs/07 §2.2
item 4 always said the grid is a checkerboard "instead of the comp background colour", and
it never was: the comp you are looking at cleared to its backdrop at full alpha (nested
comps clear transparent, K-241, but the top of the walk did not), so every uncovered pixel
reached the Viewer opaque and the board behind the picture could never show — even with
every layer hidden. The grid button was a control that did nothing visible.

The grid state now lives in `LumitUiState` (not the panel) and rides the Viewer's one
look message: `set_viewer_look` carries exposure, tone map (K-314) and this flag together,
so the renderer can never hold half a look and each control costs no channel of its own.
Renderer-owned, deduplicated against the last look actually sent (the record clears on
project adoption, so a worker just born cannot disagree with the button), and never sent
to an export's own renderer — an export always draws the backdrop. The two backdrops are two different pictures, so the flag is folded into the
frame's cache name beside the view (K-346's mechanism); the one-time cost is that frames
banked under a non-neutral view before this entry take new names.

Also fixed here, the board's edge: the picture and its board share one rectangle, but the
board is painted anti-aliased while the platform texture is not, so a fractional rectangle
bled a soft row of board out under the picture at some zooms. The shared rectangle is now
snapped to whole device pixels (`snapToDevicePixels`), where the two rasterise identically.

**K-353 · DECIDED · The lens flare antialiases itself, and hardware multisampling is gone
from it — which is what made the flare bit-stable again.** The flare had been failing its
own "GPU lens flare must be bit-stable" assertion on a clean main since before 2026-08-08
(docs/TODO.md carried it as the standing blocker). It is fixed, and the cause was not
where any of the guesses pointed.

**What was measured, in order.** The ray trace is bit-identical run to run — the trace
oracle hook reads the ray landings back and they never move. The ghost blur and the
starburst are innocent: switching either off leaves the variance untouched. The variance
survives all the way down to a single ghost at one wavelength and minimum detail, so it is
not an accumulation-depth effect either. Pooling is not the cause; allocating the scratch
and the multisample target fresh every frame changes nothing. What does change everything
is the number of samples: rendering the identical frame through a single-sampled pipeline
is bit-identical across every configuration tried, four runs each. **Additively blending
fp16 into a 4x multisample target is not reproducible on this hardware** — a few hundred
of 36864 floats came back one fp16 ULP different each run, in different places each time.

**The fix keeps the antialiasing and drops the hardware.** K-264 added multisampling
because jagged ghost silhouettes were one of the three Ultra artefacts; that reason still
stands, so the coverage is now computed rather than sampled. Barycentric coordinates are
affine in screen position, so `dpdx`/`dpdy` of them are exact, and a fragment can evaluate
its own barycentric at each of the four standard sample positions and take
`colour x covered/4`. That is exactly the model `lumit_core`'s `raster_triangle` already
spelled out as the CPU twin, so the oracle now agrees with the GPU **by construction**
rather than by resembling a hardware resolve.

Two things that fix demanded and are worth knowing. A single-sampled rasteriser only makes
a fragment where the pixel CENTRE is covered, so cells that cover sample positions but no
centre would simply vanish — a third of the flare's energy went missing at the first
attempt. Every triangle is therefore widened by a pixel before rasterising, and the
fragment's own coverage test throws the padding away again. The widening displaces the two
edge lines and re-intersects them rather than pushing corners away from the centroid:
a caustic-folded cell is a **sliver** whose corners are nearly collinear, so "away from the
centroid" points along it and leaves its thin axis exactly as thin as it was — which is the
3% of energy the anamorphic test caught still missing after the first widening.

Also gone with the multisample target: the largest allocation the effect made, ~66 MB at a
1080p flare buffer (K-265's pool), and the resolve.

Not fixed here, and still open: the flare's raster still draws the cells it culled, which
remains in TODO.

**K-354 · DECIDED · A detected flare source sits at the centre of its light, not at one
arbitrary pixel of it.** Matte mode pinned each light to the brightest pixel of its
brightest tile. For a point source that is exactly right and still is. For a *practical* —
a softbox, a window, a lamp with a visible bulb — it put the light wherever the tile scan
happened to reach a maximum first, so a flare fired from a large soft source came out of
its edge rather than its middle. Each anchor now takes the flux-weighted centroid of the
tiles that feed it, in the same fixed tile order both twins already used, so the change
costs nothing and stays deterministic. A one-tile source has only itself to average and is
untouched.

**What this deliberately does NOT fix, so it is not assumed.** The weight is flux and each
tile is still represented by its single brightest pixel, so one very hot pixel — a sensor
sparkle, a specular glint — still pulls the centroid toward itself and still flickers frame
to frame on real footage. Suppressing that needs each tile to carry its whole flux and its
own centroid rather than one pixel's, which is a change to the tile reduction in both twins
and is folded into the source-region rework
([NEXT-FEATURES.md](NEXT-FEATURES.md) entry 1 Phase D), where real segmented source regions
with recovered flux replace tile-max detection altogether. The earlier plan's "firefly
suppression by 1/(1+luma) weighting" assumed a per-pixel sum that this detector does not
have; it is not applicable as written, and Phase D is where it actually belongs.

**K-355 · DECIDED · A flare no longer jumps, because a light is no longer one pixel — and a
light with an area flares like one.** Two complaints with one root. Every detection tile was
represented by its single brightest pixel, for the light's position AND its colour, and a
light was always a point however large the thing emitting it.

**Why flares jumped.** Inside a practical — a lamp with a visible bulb, a window, a
softbox — *which* pixel is brightest changes frame to frame with sensor noise and specular
sparkle. The reported position hopped between them, so the whole flare jittered across a
source that had not moved at all. K-354 centroided the tiles and helped, but the weight was
still one pixel per tile, so one hot pixel could still drag it. Each tile now carries the
statistics of its **whole lit area** — Σ gate, Σ colour·gate, and Σ luma·gate with its first
moments — so a source's position is the flux centroid of every lit pixel in it and its
colour is their mean. A 40× sparkle moves a 64 px source by under a pixel, which the
regression test asserts by adding one. Point sources are untouched: a single lit pixel is
its own centroid and its own mean.

**Why area lights now look right.** The flare of an extended source is the integral of the
point flares over the emitting area, and at the budget K-353's plan sets out (~2 s a frame)
that integral is evaluated **directly** rather than approximated. Detection measures each
source's half-extent as the standard deviation of its flux about its centre; a source wider
than a fraction of the frame is split into a small centred grid of samples, each carrying an
equal share of the flux. The sum carries the source's shape: a tube's ghosts come out as
bars, a window's as rectangles, a bulb's as discs — which a point source structurally cannot
do. Energy is conserved by construction (the shares sum to one light), so a source only ever
gets *smoother* as it is split, never brighter, and the GPU test pins exactly that.

The grid is regular, centred and unjittered, so determinism holds (docs/14) and K-353's
bit-stability still passes. Manual mode gets the same thing as a pair of dials — **Source
width** and **Source height**, px@comp half-extents (K-260), defaulting to 0, which is the
point source the effect has always had.

`MAX_LIGHTS` rises 16 → 64 and splits into `MAX_SOURCES` (16, distinct sources detection may
find) and `MAX_LIGHTS` (64, slots the trace carries), because a source now spends several
slots when it has extent. Sixteen point sources still cost sixteen slots; a source that
cannot be split faithfully within the budget is carried as the single point it started as
rather than as half an area source, which would lose the rest of its flux.

Two consequences worth stating. A source's colour is now the mean of its lit pixels rather
than its brightest pixel's, so a source with a hot core reads very slightly dimmer than it
did — that is the firefly suppression working, and it is the more correct answer. And the
per-tile sums are order-dependent where the old maximum was not: the CPU scans a tile in row
order and the GPU merges 64 threads' partials in thread order, so the twins agree to the
matte oracle's perceptual bound rather than op-for-op. Both are internally deterministic,
which is the property that actually matters.

Not fixed here: ghosts are still *summed* point flares rather than the source's image warped
by each ghost's own Jacobian, which is the exact treatment
([NEXT-FEATURES.md](NEXT-FEATURES.md) entry 1 Phase D). Sampling converges to it and is what
the literature uses as the reference; the convolution is the optimisation, not the truth.

**K-356 · DECIDED · Lens coatings are real multi-layer stacks solved by transfer matrix,
not one layer times a quarter.** The flare's per-surface reflectance was a single-layer MgF₂
quarter wave, with every additional layer of the prescription's coating column approximated
as "quarter the residual again". That is the FlareSim shortcut, and it is the reason ghost
*colour* was the least convincing thing about the effect: a single-layer coating has one
reflectance minimum, so it can only ever tint ghosts one way, while a real multicoated lens
has two or more and runs magenta, then green, then amber as a light crosses the frame.

Each surface's coating is now a stack, and its reflectance the standard **characteristic
transfer matrix**: per layer a phase thickness `δ = 2π n d cos θ / λ` and an optical
admittance `η = n cos θ` (s) or `n / cos θ` (p), matrices `[[cos δ, i sin δ/η], [i η sin δ,
cos δ]]` chained and closed on the substrate to give `Y = C/B` and `r = (η₀ − Y)/(η₀ + Y)`,
with both polarisations averaged for unpolarised light.

**The angle term is the one that earns this.** `δ` carries `cos θ`, so the whole reflectance
band shifts blue as the angle of incidence rises — and flare rays strike interfaces at large,
varied angles. That is precisely the observed behaviour a scalar coating strength cannot
express: a ghost changing hue as its source moves off axis. The test pins it (53° reflects
over 1.5× what normal incidence does at the design wavelength) alongside wavelength
selectivity (a broadband stack's reflectance varies more than 3× across the visible).

**What the stacks are, and the honest limit on them.** A `.lens` file publishes a layer
*count*, never a recipe — real designs are manufacturer secrets, and every serious attempt in
the literature concludes coatings can only be *measured*, not predicted. So each order takes
its textbook design: one layer is the MgF₂ quarter wave, two a V-coat, three the classic
broadband quarter/half/quarter W, and more extends it with alternating quarter-wave pairs.
The *shape* is what matters here, not the exact recipe. Per-lens calibration against
photographed flares is what would make the model invertible, and it stays out of scope
([NEXT-FEATURES.md](NEXT-FEATURES.md) entry 1 Phase E).

One test had to change, and its old assertion was a fossil of the old model: it compared a
single layer against a "multicoat" on **n = 1.9** glass, where MgF₂ is very nearly the ideal
single layer (1.38² = 1.904) and any stack is *worse* at exactly 550 nm. That is a
coincidence of that glass, not a property of coatings; the comparison now runs on ordinary
n = 1.5 crown, where the broadband stack beats the single layer as it should.

**K-357 · DECIDED · Auto is a real preview tier, the resolution is per composition, and the
comp's backdrop has a swatch.** The three halves docs/07 §2.2 owed after the bar dropdown
landed.

**Auto is not Full under another name, and the old "Full" was neither.** The tier labelled
Full silently multiplied the panel's own scale, so it never meant composition resolution —
there was no way to ask for that at all, which is exactly what you want when judging detail
at 100 %. Auto now means "render only the pixels the current magnification can display" and
is the default because it is what the Viewer has always in fact done; Full means composition
resolution whatever the panel is showing; Half, Third and Quarter are the fractions they
say, taken as asked rather than multiplied by the panel, so the tier you chose is the tier
you get. Third and Auto have no chord of their own (§15 names three), so `action` is now
optional on the enum.

**The tier is per composition**, in the session blob beside the viewer looks (K-314's
pattern, K-245's blob) rather than in the document: a heavy shot wants Quarter while the
title card beside it does not, and choosing how coarsely to preview is a way of *working on*
a comp, not an edit to it — so it makes no op, no undo step, and can never reach an export
(glossary §5). A tier name a build does not recognise reads as Auto rather than refusing the
project.

**The background swatch is the opposite kind of thing, and sits next to the grid button for
that reason.** Everything else on that half of the bar — exposure, tone map, the
transparency grid (K-352) — is a way of looking. The background colour is what the composite
is actually drawn onto and what an export writes there, so it goes through a new
`SetCompBackground` op and Ctrl+Z undoes it. The two controls answer the same question from
opposite sides — *what is behind the picture* — and having one without the other is what
made a black comp confusing. Black and white are offered as presets because that is nine
uses in ten.

**K-359 · DECIDED · The sprite flare is its own effect, not a mode of the physical one.**
docs/08 §3.29. The owner asked for an Optical-Flares-style flare alongside the simulated
one (2026-08-12), and *alongside* is the whole design: §3.27 asks "what would this lens
actually do", §3.29 asks "draw me a flare here". They answer different questions, and the
first draft of NEXT-FEATURES muddled itself by trying to make one serve both.

Everything is placed from the light's **position** — a glow on it, a train of iris ghosts
along the line from the light through the frame's centre, and an anamorphic streak. The
ghosts march through the centre because that is where a real lens puts its reflections
(mirrored about the optical axis), and their spacing is a *fraction* of the light→centre
distance, so the train stretches and gathers as the light moves rather than sliding rigidly.

**No bright-pass, which is the point.** Nothing is read from the picture's brightness, so
there is no threshold for a source to cross and nothing to pop as grain moves — the exact
complaint that made §3.27's Matte mode unpleasant on footage. The oracle asserts it as a
property rather than trusting it: moving the light one pixel may not change any pixel by
more than a small bound, which a threshold-driven flare cannot pass.

One procedural compute pass, no inputs but the layer, so it is Cheap where §3.27 is Heavy.
Both flares stay; neither is the other's fallback.

**K-360 · DECIDED · Light layers, and the flare's Lights source mode finally wired.**
`LayerKind::Light` (docs/03-DATA-MODEL.md §5.5) — the first new layer kind since Shape, and
what K-257 reserved the flare's third source mode for.

**A light is a Camera-shaped thing, not a Solid-shaped one.** It draws no pixels of its own;
it is something other layers *see*. So it carries its placement in the ordinary layer
transform rather than inventing a second one — a light animates, parents, and is dragged
with everything already built for that — and `Composition::lights_at` resolves the visible,
in-span ones at a time, top of the stack first, which is the order the effects that read
them take their slots in. A light switched off is not a light, exactly as a layer switched
off is not on the picture (K-230).

**The area kind is the one that earns the layer.** Point and spot are there for completeness;
a rectangle with a real width and height is what a compositor actually reaches for, and what
the flare can do something with that a point cannot. An area light arrives at the flare with
a real extent and flares as its own shape through exactly the machinery K-355 built for
detected sources — so a strip light throws bar-shaped ghosts, with no new rendering code at
all. That is why K-355 came first.

**Lights mode needed no new plumbing**, which is worth recording because the obvious design
does. The lights are resolved in `resolved.rs` from the expression context, which already
carries the document, the comp and the time — everything needed — and they ride to the GPU
in a fixed array on `LensFlareParams` rather than a `Vec`, because those params must stay
`Copy` for the bake cache and the frame-key hash. An empty list in Lights mode is the
labelled no-op: a comp with no lights flares with nothing, rather than falling back to the
Manual point and putting a flare somewhere nobody asked for.

The frame key hashes the light's own properties, not just the fact of it — unlike a Camera,
where the pose is hashed at comp level. A light that changed colour without renaming its
frames would serve a stale one.

Not built here: **LTC shading of layers by these lights** (NEXT-FEATURES entry 4c). The model
is now in place for it, which was the dependency.

---

## K-361 — Layers are shaded by lights with a closed-form area-light integral, and light adds rather than replaces

**DECIDED** (2026-08-12). Supersedes nothing; completes K-360, which built the Light layer
and said this was the dependency it was there to satisfy.

A layer with the new **Accepts lights** switch on is shaded by the composition's Light
layers in a pass that runs after its effect stack and before it is placed —
`lumit_core::lighting` (the reference), `fx_lighting.wgsl` (its twin), called from
`Realiser::realise_segment`. It is **not an effect**: it has no entry in docs/08, no
`Resolved` variant and no place in a stack, because it is not something you add to a layer,
it is something the composition does to it. docs/06 gains the pass.

**The maths is closed-form, and that is the whole point.** How brightly a flat surface is lit
by a flat glowing rectangle has an exact answer — the cosine-weighted fraction of the
surface's sky that the rectangle covers, which is a sum of one term per edge. Four edges,
four terms, no sampling and no noise. This is the diffuse form factor, and it is also the
identity-matrix case of **Linearly Transformed Cosines** (Heitz et al. 2016), which is what
NEXT-FEATURES entry 4c named.

**LTC's fitted matrix tables are deliberately not shipped.** They are what buys roughness and
specular highlights, they are 64×64×2 textures of published-but-third-party fitted data, and
a 2.5D compositor shading flat layer planes has nothing to be glossy with — there are no
per-pixel normals, and inventing them from luminance is a content-dependent quality cliff
that Nuke's Relight is the ceiling for, not a first landing. Diffuse over a quad is the
honest 2.5D answer and it needs no tables at all. The code is shaped so a matrix fetch drops
in ahead of the same integral if that changes.

**Light adds; it does not replace.** The pass multiplies the picture by `1 + light`. Physical
shading multiplies by the light alone, which means dropping one light into a composition
plunges everything it does not reach into black — the correct answer to a question no
compositor is asking. Adding also makes the no-op exact rather than approximate: a
composition with no Light layers produces an empty light list, the pass never runs, and the
frame is byte-for-byte what it was before any of this existed. That is the compatibility
promise, and it is a test.

**A point light has no inverse square.** Its brightness is the cosine law and the artistic
falloff dial, nothing else. An inverse square measured in comp pixels is a number with no
physical meaning that a compositor would only have to fight; `falloff_px` already says, in
pixels, where the light stops, which is the control someone actually wants.

**The rectangle is clipped to the horizon**, not merely clamped at the end. The part of a
light that has sunk behind the surface must be removed before the sum, or the answer is
nonsense rather than merely too big — and a light straddling the plane is a real case, not a
corner one. The form factor is then taken as a **magnitude**: the sign only records which way
round the corners were listed, and a light here has no back face to hide behind, so no
caller has to wind its corners a particular way to get light.

**A 2D layer is shaded where it is drawn, not where its transform says.** A layer without the
3D switch is composited flat at z = 0 whatever its z and out-of-plane rotations hold, so the
shading forces those to zero too. Shading a layer at a depth the compositor ignores would
light something that is not there.

`ResolvedLight` gained `z`, `rotation_x_deg` and `rotation_y_deg` for this. The Lens flare
ignores them — it works in the projected picture, where a light is simply wherever it lands —
but shading cannot: a rectangle in the same plane as the surface it lights is edge-on and
throws nothing, so a softbox that does anything at all is a softbox in front.

**Budget:** eight lights per layer, the nearest kept, chosen by a total order so two runs pick
the same eight. Running out of uniform slots must never make a frame fail (docs/13).

**Frame key:** the comp's lights are hashed once at comp level (a Light layer draws no pixels,
so nothing else in that walk would notice one moving), and only when the comp has one.
`accepts_lights` is hashed only when it is *off* and a light exists — the default is on, so
off is the state that departs from what a pre-lighting key described. Every key made before
this stays valid.

---

## K-362 — The region of interest is a window on the composite, and a frame rendered through one takes its own name

**DECIDED** (2026-08-12). Closes docs/07 §2.2 item 7 and NEXT-FEATURES entry 7.

Drag a rectangle on the picture and the Viewer composites only that window of the
composition. Preview only: the export renderer is never sent one, which is the same
construction that keeps the preview scale out of files (K-186) and the Viewer's exposure out
of them (K-346).

**It is a window, not a crop of a finished frame.** `Realiser::realise_region` shifts the
comp-pixels-to-NDC mapping and sizes the target to the region, so the composite writes only
the pixels asked for. The camera matrix is untouched — it projects comp space, and the region
maps a window onto the result, which is why a region cannot change perspective. Pixel density
is unchanged: a region half as wide renders half as many pixels of the same size, so every
px-dimensioned parameter in the frame still means what it meant.

**Two stagings refuse the window, and the frame is composited whole and cropped instead.** An
**adjustment layer** runs its effects on the composite of everything below it, and a
**motion-blurring layer** averages sub-frame copies into a comp-sized texture first. Both are
written against the comp raster, and windowing either halfway would give a wrong picture
rather than a fast one. Refusing is silent by design *because it cannot be noticed*: the
returned texture is the region's size either way, so the picture is identical and only the
work differs. A regression test asserts a windowed composite is the same pixels the full
frame has there — that equivalence is the whole promise, and without it working inside a
region is working on a lie.

That is the honest form of the warning NEXT-FEATURES recorded against this entry. A region
saves the composite, the display encode and the publish; it does **not** save the effect
stack, which runs per layer at the layer's own size and is untouched. Culling layers whose
placement misses the region would save that too, and is the upgrade path — it is not here
because a layer that appears not to touch the region can still reach it through an adjustment
layer's blur, and getting that wrong is a missing element rather than a slow frame.

**The frame name folds the region in** (`named_under_view`, joining K-346's view and K-352's
transparent background). The alternative the plan floated — refusing to name frames while a
region is set — would make the cache useless exactly when it is wanted, since scrubbing
inside a region is the use case. A region covering the whole comp is refused as no region at
all, so the common case keeps sharing the full frame's names and nothing already banked is
orphaned.

**The rectangle crosses every boundary as fractions**, never pixels: which pixel a point is
depends on the raster the engine settles on, and it settles on different ones at different
preview resolutions. Fractions mean the same thing at all of them. Degenerate input —
inverted, empty, out of range, not finite — clears the region rather than faulting, on both
sides: a drag that ends where it began is a gesture, not an error.

**One button, both states.** Set a region and the same control clears it; "look at a corner"
and "stop looking at a corner" are one decision, not two. The region is outlined whenever it
is in force, so it is never possible to be looking at part of a shot without being told —
the failure mode this feature would otherwise introduce.

Per comp, in the session beside the preview resolution (K-357), for the same reason: choosing
where to look is a way of working, not an edit.

---

## K-363 — The flare threshold is one-sided: the luma a pixel must exceed, softened only upward

**DECIDED** (2026-08-12, owner). Supersedes the symmetric gate K-259 shipped.

The Matte-mode Threshold now means what it says: **the absolute scene-linear luma a pixel
must exceed to flare at all.** At 1.0 only over-range highlights flare; at 0.0 anything
brighter than black does — and black itself never flares. Threshold softness widens the
onset *above* the line (fully open at `threshold + softness`), never below it.

The gate it replaces opened at `threshold − softness` and reached half strength at the
threshold itself. Two consequences the owner hit: sources dimmer than the stated threshold
still flared, and — the degenerate case that made the old shape indefensible — at threshold
0 with any softness, **pure black passed the gate at half strength**. A threshold whose zero
does not mean "everything brighter than black" is a dial that cannot be reasoned about.

Both twins changed together (`lens_flare::threshold_gate` and `fx_lens_flare_detect.wgsl`'s
`gate`), the detection tests pin the two named cases, and the hard-edge comparison is
strictly greater-than — "at the line" is not "over it".

This changes pictures for projects that relied on at-or-below-threshold sources flaring,
so the effect's version bumps 5 → 6 — the K-016 mechanism that retires every cached frame
the old gate drew. The old behaviour is recoverable by lowering the threshold by the
softness — which is exactly the sentence that shows the new semantics are the primitive ones.

---

## K-364 — Ghost radiometry is spectral: 8 sub-samples per traced band against a baked reflectance table

**DECIDED** (2026-08-12). Entry A2 of the flare programme; builds directly on K-356's
transfer-matrix coatings, which created the problem this solves.

A ghost's colour is its path's reflectance, and since K-356 that reflectance is a real
multi-layer stack whose R(λ) oscillates several times across the visible. The trace sampled
it once per traced band — at the band centre — and a curve with three minima sampled at
three points is systematically wrong in exactly the quantity K-356 made accurate.

Geometry and radiometry now split, because they vary at different rates. The ray path is
still traced once per band (dispersion is smooth; the geometry ladder is unchanged). The
ray's **energy** is carried per band as eight sub-sample throughputs, each reading a
**baked reflectance table** — `FlareBaked::reflectance`, R on a (surface, direction, λ at
5 nm, cos θ) grid, computed once per lens change by the same `stack_reflectance` the trace
used to chain per ray — and folded against the band's CIE weights at the sensor. Even the
lowest quality tier now samples the spectrum 24 times where it sampled 3; Ultra samples it
256 times. The per-frame cost went *down*: eight table reads replaced a 2×2 complex matrix
chain per surface event, and the WGSL's inline thin-film maths is deleted.

Choices worth recording:

- **Both crossing directions are tabulated** (a ghost's phase-2 walk crosses surfaces
  backwards) rather than Snell-conjugating angles at trace time. Twice the table, none of
  the per-ray trigonometry, no reciprocity argument to get subtly wrong.
- **Sub-wavelengths snap to the table's 5 nm grid**, so the lookup is exact in λ and only
  cos θ interpolates. The CPU twin and the WGSL mirror the same arithmetic op for op.
- **Exposure is preserved by construction**: a band's sub-weights are its CIE integral
  split eight ways, so a spectrally flat throughput renders at exactly the old exposure.
  The out-of-gamut clamp moves from per-band to per-sub-sample, which throws strictly less
  away (violet bands used to zero their whole negative channel).
- **The Coating dial stays frame-time**: the table stores the fully coated stack, plain
  Fresnel stays analytic at the band wavelength, and the dial blends between them per
  event — so animating it never rebakes, exactly as before.
- The trace's corner output grew from a scalar weight to (geometric weight, rgb), and the
  K-266 3×3 cliff-smoothing now carries weight×rgb through the same mean — with a constant
  rgb it is exactly the old smooth, so nothing about its shape changed.

The ranking and spread probes inside the bake keep the scalar single-λ walk: they only
rank, and 8× radiometry there would be spent on answers nothing reads.

---

## K-365 — The starburst is field-dependent: eight cat's-eye slices, one azimuth, rotated at draw time

**DECIDED** (2026-08-12). Entry B2 of the flare programme.

The starburst is Fraunhofer diffraction at the iris, and the bake treated the iris as the
whole story: one sprite per lens, stamped identically wherever the light sat. Off-axis
that is wrong. The hole light actually diffracts through is the iris clipped by the
front and rear mechanical stops from opposite sides — the **cat's-eye** every
photographer knows from the lemon-shaped bokeh at a picture's edges — so a real starburst
squashes and leans as the light moves out towards a corner, and ours did not.

The aperture is now baked at `STARBURST_FIELDS` = 8 field angles, slice 0 on-axis and
slice 7 at the sensor-corner angle `atan(half sensor diagonal / focal)`. Each slice is
the same `pupil_mask` polygon multiplied by the imaging path's vignette, measured by a
new straight refract-only walk (`trace_transmit`) that accumulates the housing feather
`trace_splat` already uses. The slices are concatenated slice-major and uploaded as one
atlas texture; the combine reads the two slices bracketing each light's field fraction
and lerps.

Choices worth recording:

- **One azimuth, rotated at draw time.** The cat's-eye is symmetric about the meridional
  plane, so the whole family is one baked azimuth (the lean along +x) turned by −azimuth
  when the sprite is stamped. Baking the azimuths instead would multiply the bake by the
  number of directions and buy nothing.
- **Slice 0 is the old picture, bit-for-bit.** On-axis the imaging vignette is 1 across
  the whole entrance pupil for every bundled prescription, so a centred light renders
  exactly what it rendered before. Nothing about this change is a look; it is a shape
  that was missing off-centre.
- **The vignette trace skips the stop and samples the entrance pupil.** The iris is
  already the polygon mask, so counting the stop again would shrink every image by its
  own edge; and the disc traced is `focal / 2N`, not the wider ghost-spray radius, which
  when used clipped the polygon into a circle at two thirds of its edge and made every
  aperture round. Both are the kind of double-count that reads as "the feature does
  nothing" rather than as a bug.
- **A dead slice holds the last live one.** A lens that does not cover the full frame —
  the bundled 7Artisans is an APS-C design, and a user `.lens` file may be anything —
  passes nothing at the outer field angles, and its corner slice bakes black. A starburst
  that vanishes as the light nears the corner is a worse picture than one that stopped
  changing there.
- **The bake stays under half a second.** The eight slices are independent FFTs and go
  wide across the pool; a 24-surface prescription measured 170 ms before and 225 ms after
  on the development machine. The bake already runs beside the frame (K-350), so this
  never touches the picture's latency.

The blade parity the starburst has always had — an even blade count gives N spikes, an
odd one 2N — is now pinned by a test on slice 0, because the vignette multiply is exactly
the kind of change that can quietly round the iris off and leave a plausible-looking glow.

---

## K-366 — The ghost grid is splatted per ray, not rasterised as quads

**DECIDED** (2026-08-12). Entry B1 of the flare programme.

The trace fires a grid of rays through the pupil and every one of them lands somewhere on
the sensor. Since K-261 the renderer joined neighbouring landings into quads and drew
those quads, brightness being the launch cell's area over the landed one. That is a fair
model of a smooth map and a wrong one at a **caustic fold**, which is precisely where a
flare's rims and arcs live: at a fold the map folds back on itself, so a quad joining four
rays across it is a sliver spanning geometry that is not one patch at all. Every rescue
K-261..K-264 added — sub-pixel inflation about the centroid, sliver parking, the unlit
corner pull-in, the vertex-smoothed density — and K-353's pixel widening with its analytic
four-sample coverage exist to survive that one wrong join. Each fixed the artefact in front
of it and moved the next one somewhere else: notched rims, fan lines out of the bore,
Ultra faceting, dropped fold flux.

A ray now deposits on its own. Its footprint is the image of its pupil cell under the
ghost's map, read as a 2×2 Jacobian by central differences over the neighbouring rays'
landings (`ray_axes`; one-sided at the grid edge or beside a dead ray). It spreads its
flux over that parallelogram as a separable tent `(1−|u|)(1−|v|)`, which integrates to the
parallelogram's area — so flux is conserved exactly, and a fold is simply several splats
landing on top of one another, which is the integral the effect wanted all along. Nothing
is joined to anything, so there is no sliver to park, nothing to inflate, no corner to
pull in and no coverage to compute; all of that machinery is deleted rather than patched
again.

Choices worth recording:

- **The density cap is kept, unchanged.** At a fold the density `cell ÷ landed` genuinely
  diverges; the integral over a pixel does not, but a *discrete* ray concentrates the
  divergence into a few pixels. `MIN_AREA_FRAC` = 3e-3 caps it at ≈333×, which is K-262's
  number and K-262's reason.
- **`MIN_SPLAT_AXIS_PX` = 0.75 replaces `MIN_QUAD_PX`'s inflation.** It is an anti-alias
  floor, not a rescue: a footprint that collapses below a pixel still deposits over
  roughly one, so a caustic line is a line rather than a row of dropped sub-pixel points.
  The fold case — both axes long but nearly parallel — pushes the second axis across the
  first up to the same floor, which is what stops an edge-on fold's flux vanishing into a
  zero-area parallelogram.
- **The GPU does the division, the raster does the tent.** `build_splats` (one thread per
  ray, replacing `quad_area` and `build_verts`) writes centre, half-axes and the peak;
  the draw is one instanced six-vertex quad per ray whose fragment evaluates the tent and
  adds. A tent is continuous, so the single-sampled target of K-353 needs no coverage
  logic at all and the pipeline keeps its bit-stability by construction — fixed instance
  order, additive blend, one sample.
- **Scratch is now per ray, not per quad.** 48 bytes a splat against 84 a cell (20-byte
  corners plus the area word), and `side²` of them where there were `(side−1)²` — a wash
  in memory, one compute pass fewer, and the `SCRATCH_BYTE_BUDGET` bound is unchanged.
- **Pictures move, so the effect's version goes 6 → 7.** Folds and rims are where the
  difference is; smooth ghost bodies render as they did.

The CPU reference and the WGSL are twins op for op, as they have been since K-261, and the
two constants above are spelled in both and pinned by test.

## K-367 — An area source is integrated per ray, not replicated into point lights

**DECIDED** (2026-08-12). Entry D of the flare programme, and the answer to the owner's
report that the effect was rendering "bright areas of a matte using multiple points
instead of an area".

K-355 gave a source a size and rendered it by **replication**: `expand_area_lights` split
one light into up to 5×5 point lights spanning ±extent, each carrying a share of the flux,
and the whole ray pipeline ran once per sample (the Matte path expanded the same way
inside detection, which is the only reason `MAX_LIGHTS` was 64 rather than 16). The flare
of an extended source really is the integral of the point flares over it, so the model was
right; evaluating it by replication was not. It cost up to 25× the rays, and wherever a
ghost was smaller than the spacing between samples it showed exactly what the owner saw —
N overlapping copies of the aperture strung out in a line, rather than one smeared shape.
Sampling more finely could only ever push the artefact below the current ghost size, never
remove it.

**Each ray integrates the source itself instead.** A ray at pupil-grid (i, j) offsets its
light position within the source's ±extent rectangle by a smooth deterministic
stratification and computes its own direction from the jittered position; every ray
carries the light's full colour, because the pupil grid already averages. The source
integral is absorbed into the pupil quadrature the trace was already performing, so an
area source costs exactly what a point source costs, whatever its size — the plan's
"per-ghost warped convolution", arrived at through the splatting pipeline rather than
beside it. The sampling *is* the warped convolution, evaluated per ray.

What makes replicas impossible rather than merely rare is K-366. A splat's footprint is
the image of its pupil cell read by central differences over its neighbours' landings, and
those neighbours now sit at different points of the source — so each footprint inflates by
the local source-to-sensor stretch, which is precisely the gap a replica would have sat
in. No two rays share a source position, and every ray's deposit already covers the
spacing between them.

Choices worth recording:

- **A triangle wave, not `fract`.** `offset = tri((i + ½)·Φ)·extent`, with
  `tri(x) = 2·|2·(fract(x) − ½)| − 1`. The usual low-discrepancy trick is a bare `fract`
  of an irrational rotation, and it is wrong here for one specific reason: `fract` jumps
  the whole range at each wrap, and the footprints are central differences over exactly
  the neighbours a jump would separate by the width of the source — one splat inflated to
  the whole source stamps a bright bar across the ghost. A triangle wave is continuous at
  every wrap, still uniform on [−1, 1], and just as deterministic.
- **A different irrational per axis** (`PHI_U` = 1/ρ, `PHI_V` = 1/ρ², the plastic
  constant's pair): one constant for both would put every offset on a diagonal of the
  rectangle, sampling a line rather than an area.
- **Extent 0 is bit-identical to before.** Every ray offsets by exactly zero, so the point
  source every existing project has does not move — pinned by test over a grid wider than
  any quality tier launches, alongside the render being independent of how the light was
  built.
- **`MAX_LIGHTS` is deleted; `MAX_SOURCES` (16) is again the whole story.** One source is
  one light slot however large it is. Matte detection stores each source's measured extent
  instead of looping samples into slots, and the trace dispatch shrinks with it. The op
  seam carries the extent: `LensFlareOp::manual_lights` is `[x, y, r, g, b, ext_x, ext_y]`
  and the WGSL `Light` spends two of its three pads on it.
- **The starburst smears by a fixed 3×3 stamp over the source.** The ghosts integrate
  their source per ray; the starburst cannot, because it is a baked sprite rather than a
  traced path. It is *shift-invariant* though — a hole's diffraction pattern does not
  change shape as the source moves, only where it is centred — so an extended source's
  starburst is exactly the point sprite convolved with the source, and the combine
  evaluates that convolution in quadrature: three stamps per axis whose extent passes
  `SB_MIN_EXTENT` (0.004 of the raster, the old area threshold), spanning ±extent, each
  carrying `1/(nx·ny)` of the light. The K-355 replication used to give this for free as a
  side effect of stamping per sample, and per-ray integration would otherwise have thrown
  it away, leaving a softbox with a star's pinpoint spike. The K-365 field slice and
  azimuth are computed **per stamp**, so a smeared starburst near the frame edge leans a
  little differently at each end of itself. A point source is one stamp at full strength
  on its own position, bit-identical to what it always drew.
- **Area pictures change, so the effect's version goes 7 → 8.** A point source's does not.

The CPU twin (`source_jitter`) and the shader's copy are op for op, and the two constants
are pinned by test — compared as bits, since Rust and WGSL print floats differently.

## K-368 — Four-bounce ghosts are enumerated under a reflectance-product prefilter and ranked in one list with the pairs

**DECIDED** (2026-08-12). Entry C1 of the flare accuracy programme.

A ghost is light that reflected off **two** lens surfaces on its way to the sensor, and
the bake has always traced exactly those: `N(N−1)/2` paths for `N` interfaces, ranked by
an on-axis ray probe. Light that bounces **four** times lands on the sensor too. With
modern coatings each such path carries on the order of 10⁻⁵ of a two-bounce one, which
sounds like nothing — but the sun is ~10⁵ times a normal highlight, a few four-bounce
paths happen to focus tightly rather than wash out, and with vintage uncoated glass
(R ≈ 4% a surface) they are plainly visible as the chains of doubled ghosts old lenses
are known for. Not tracing them made every bundled prescription slightly cleaner than the
glass it describes.

**The path model.** `FlareBaked::pairs` becomes `Vec<[u32; 4]>`. Slots 0 and 1 keep
exactly the meaning K-261 gave them — the ray runs forward to `b`, reflects, back to `a`,
reflects, then out — so `a < b` still holds and a two-bounce path is
`[a, b, NO_BOUNCE, NO_BOUNCE]` with `NO_BOUNCE = u32::MAX`. A four-bounce path adds the
same figure once more: forward from `a` to `c` reflecting there, back to `d` reflecting
there, then forward to the sensor. Hence `a < c` and `d < c`, while `c` may sit either
side of `b` and `d` may be `a` itself. The sentinel is what keeps the two-bounce case
honest: with `c` = `u32::MAX` no surface index can equal it, so phase 3 runs to the end
of the stack with its reflect flag always false and the ghosts every existing project has
execute the statements they always did — in all three walks (`trace_splat`,
`trace_splat_spectral`, and the WGSL trace kernel, which spends two of `Combo`'s padding
slots on `bounce_c`/`bounce_d` and so keeps its layout).

**Why they are prefiltered rather than probed.** There are ~N⁴/4 four-bounce paths — over
a hundred thousand on a normal prescription, millions on a zoom — and the ray probe that
ranks pairs costs three traced rays each. They are therefore pre-ranked by an upper bound
on the energy a path can carry: the product of the four surfaces' reflectances at normal
incidence and 550 nm, precomputed once per surface. It is an upper bound in the coating's
own terms (an AR stack is designed to be at its worst on-axis) and it is a product of
numbers under one, so a partial product bounds the whole — which is what lets the search
prune an entire `(a, b)` sub-tree the moment its pair reflectance cannot beat the worst
candidate already kept. The best `FOUR_BOUNCE_PROBE_CAP` = 1500 of them are kept, in a
bounded top-K heap keyed by (bound bits, tuple) so the kept set is one deterministic set
rather than whichever equal-bound path arrived first.

**The bound decides only what is probed.** What renders is decided by the same on-axis
three-wavelength ray probe every pair faces, against the same `PAIR_MIN_INTENSITY` floor,
and the survivors merge into **one** ranked list — descending probe brightness, ties by
tuple order. Everything downstream (`MAX_RENDERED_PAIRS`, the spread probes, the frame
grid plan, the GPU combo table) consumes that list unchanged and cannot tell the two kinds
apart. A crude bound can therefore only cost a path its chance to be measured; it can
never make one render brighter than it is.

Measured on the bundled library: the whole two-bounce family outranks the whole
four-bounce one on every lens — four Fresnel factors are worth more than any geometry — so
what decides whether the extra ghosts appear is whether the pairs run out first. The
11-surface Biotar has 45 pairs, so its four-bounce paths start at rank 45 and over a
hundred fall inside the rendered 200; the 24-surface Master Prime has 252 pairs and its
four-bounce paths never get a look in, which is the right answer for modern multicoated
glass. Bakes stay inside the budget (118–208 ms across the library, the cap's 1500 probes
included).

**Existing projects gain ghosts, so the effect's version goes 8 → 9.**

## K-369 — Ghost edges are Fresnel: a baked near-field ring mask per defocus rung

**DECIDED** (2026-08-12). Entry C2 of the flare accuracy programme.

The starburst is **Fraunhofer** diffraction — the far field of the aperture — and it has
been right since K-256. The ghosts are **Fresnel**. Each ghost image is defocused by its
own amount, so its edge is not the hard iris polygon the ray trace draws: it carries
near-field diffraction ringing, a set of fringes just inside the rim brighter than the
middle of the ghost, at a scale set by that ghost's own defocus. Every real-time flare
drops this, which is exactly why real-time ghosts have stencil edges and photographed ones
shimmer.

**The propagator was already in the file.** `bake_starburst` multiplies the aperture by
the quadratic phase `e^{iπ(x²+y²)/(λd)}` and takes one FFT — that IS Fresnel propagation to
distance `d` (the single-FFT propagator). Joo et al. (2016, CGF 35(4)) parameterise the
family of ghost images by the fractional Fourier transform; the computable member of that
family, at each order, is this same propagator at a different distance. C2 is therefore the
machinery the file already contains, run at a ladder of distances, and applied as the
**iris mask** of the brightest ghosts rather than as a sprite.

**The ladder.** `RING_SLICES` = 6 slices of `RING_RES`² = 128², at Fresnel numbers
`F = 64, 32, 16, 8, 4, 2`. Folding `F = a²/(λd)` into coordinates normalised so the
aperture frame is `x, y ∈ [−1, 1]` makes the chirp phase exactly `π·F·(x² + y²)`, which is
the whole parameterisation: high `F` is a nearly-focused edge with fine ripples, low `F`
spreads. The slices are baked from the **same on-axis aperture image the starburst's slice
0 uses** — computed once and read twice — so the ring masks rebake with blades, rotation,
roundness and f-stop exactly when the starburst does, under the same `bake_key`.

Two details are load-bearing and were derived rather than guessed. First, the propagator's
output lands in its own frame: DFT bin `m` sits at `x' = m/(N·Δx·F)` in the aperture's
units, so the output window is `±1/(2·Δx·F)` wide and **narrows as `F` rises**. Reaching
`F = 64` at all needs `N ≥ 256`, which is why the propagation runs at `APERTURE_RES` and
only the resampled mask is `RING_RES`; the same bound is the chirp's Nyquist limit, so one
condition covers both. Second, the intensity is resampled back onto **pupil** coordinates
(`u = 1` is aperture ndc `APERTURE_SIZE`), so a mask lookup at a traced ray's `(u, v)` is a
direct bilinear tap — `ring_mask_sample`, mirrored op-for-op in the trace WGSL with
`RING_RES`/`RING_SLICES` pinned by the constants test.

**Energy is preserved, over the pupil DISC.** Each slice is scaled so its energy equals the
analytic iris mask's — a ringed ghost must be neither brighter nor dimmer than the
hard-edged one it replaces — and nothing is clamped: the overshoot above 1 at the rim
(measured peak ≈ 1.50 against a plateau of ≈ 0.98 at `F` 64) is the Gibbs ringing and is the
entire point. The disc rather than the square matters: Fresnel diffraction genuinely throws
light past the geometric aperture, and the trace does not spray rays out there, so
normalising over the square would quietly halve the widest ghosts.

**Which ghost gets which rung, and the honest limits of that.** The top `RING_GHOSTS` = 32
ranked paths carry a slice; everything below keeps the analytic mask, because a ghost that
faint contributes no visible edge. The slice comes from the path's already-measured image
spread: `F ≈ RING_SPREAD_REF / spread`, clamped to the ladder. Joo et al. derive each
ghost's order from the ABCD matrix chain of its own path — the exact answer, and **the
recorded upgrade path**; the spread proxy buys the same monotone relationship (tight ghost
→ near its focus → crisp edge with fine ringing; frame-filling wash → broad ringing)
without the matrix plumbing. `RING_SPREAD_REF` = 3.2 is a **calibration, not a derivation**:
the bundled library's top-32 spreads run 0.05 … 8 of the sensor diagonal, and 3.2 lays the
six rungs across 0.05 … 1.6 of that. It is the knob to turn, and it earns its own
regression test — at the plan's first value of 0.5 every ranked path on every bundled lens
landed on the bottom rung and the ladder had one step.

The bake cost is the six FFTs alone: the aperture image they propagate is one the starburst
was already tracing, so the added time is single-digit milliseconds against a bake of
hundreds.

**Existing projects' ghost edges change, so the effect's version goes 9 → 10.**

## K-370 — Ghost-edge diffraction is the knife-edge asymptotic at the real Fresnel numbers, not a propagated mask ladder

**DECIDED** (2026-08-13). **Supersedes the implementation of [K-369](#k-369--ghost-edges-are-fresnel-a-baked-near-field-ring-mask-per-defocus-rung)**; K-369's intent — a ghost's rim carries near-field diffraction, and stencil edges are wrong — stands unchanged.

**What the owner saw.** "The output from the lens flare effect feels like it has a Fresnel
interference pattern which can be distracting across the whole screen where the effect is
applied." That is exactly what it was, and the measurement is unambiguous. Dumping the
K-369 ring masks' radial profiles on the bundled default:

| slice | F | interior ripple | interior mean vs the flat mask |
|---|---|---|---|
| 0 | 64 | 6.3% | 0.99 |
| 3 | 8 | 23.6% | 1.20 |
| 4 | 4 | 36.4% | 1.26 |
| 5 | 2 | **44.0%** | **2.39** (4.7× at the centre, 0.3 at the rim) |

and every one of the top-32 paths landed on slices 3, 4 or 5. So each visible ghost carried
a broad concentric brightness gradient across its **whole area**, not a rim effect — and the
ghosts that fill the frame painted that over the whole picture.

**Why the ladder could only ever land there.** The Fresnel number of a ghost is derivable,
and K-369 never derived it. The ghost patch *is* the defocused aperture, so its radius on
the sensor is `a`; the cone forming it leaves the pupil at the marginal-ray angle, which the
working f-number fixes at `1/(2N)`, so the defocus is `z ≈ 2Na`. One power of `a` cancels:

```text
F = a²/(λz) = a/(2Nλ)
```

Put real numbers in. A 5%-of-frame ghost at f/2.8 is `F ≈ 350`; a frame-filling one `F ≈
7000`; the widest washes on the bundled lenses reach `F ≈ 50 000`. K-369 baked at `F` of 2
to 64 — **two to three orders of magnitude low** — because that is the ceiling a 256²
single-FFT propagator can reach: its output window is `±(N−1)/(4F)` aperture units and has
to cover the aperture, so `F ≤ (N−1)/3`. Reaching `F = 1000` would need a 4096² transform.
The calibration `RING_SPREAD_REF` was then tuned to spread real ghosts across the rungs the
propagator *could* reach, which guaranteed they all sat in the regime where the near field
is a whole-aperture pattern rather than an edge one. K-369's own entry called that constant
"a calibration, not a derivation, and the knob to turn"; the knob had no setting that worked,
because the mechanism was wrong for the regime.

**The replacement.** At `F` in the hundreds and up, the fringes are a rim effect a few
percent of the pupil wide and the blade is locally straight, so the correct model is the
**knife-edge asymptotic** — a closed form of one variable, the perpendicular distance to the
blade:

```text
I(v) = ½[(C(v) + ½)² + (S(v) + ½)²],   v = s·√(2F)
```

with `C`, `S` the Fresnel integrals (`π/2` convention, by the standard auxiliary-function
rational approximation, error < 2e-3). It is 1 deep inside, exactly ¼ on the geometric edge,
peaks at ≈ 1.37 at `v ≈ 1.22`, and decays to nothing outside — the light real diffraction
throws past the blade, which the old bake had to normalise away. `s` is `(bound − r)` from
the same polygon bound `pupil_mask` already computes, times the cosine that turns a radial
gap into a perpendicular one (exact for the polygon, 1 for a round iris).

**The interior of a ghost is now flat by construction.** Whatever else this profile does, it
cannot shade or tint the middle of a ghost — that is the property the regression test pins,
and the one K-369 could not have passed.

**Fringes nobody can sample are averaged, not drawn.** A fringe train finer than the pupil
ray grid does not appear; it **aliases**, and an aliased fringe train is a beat pattern
spread across the whole ghost — the other half of what was on screen. The honest answer when
they cannot be resolved is their average, and a diffraction profile averages to the geometric
edge it surrounds. So the mask crosses from the ringed profile to the plain one over
`blur_v` of 0.5 … 2, where `blur_v` is the wider of the ray-grid step and the Softness
feather, in `v` units. A soft blade smears its own fringes exactly as a coarse grid does, so
the two enter the same way.

The practical consequence is worth stating plainly: **the big frame-filling ghosts now show
essentially the plain iris edge**, because their fringes are far finer than any grid the
effect traces, and the tight bright ones — where a real photograph shows ringed rims — keep
theirs. That is the opposite of what K-369 did, and it is the right way round.

**What this deletes.** `RING_SLICES`, `RING_RES`, `RING_GHOSTS`, `RING_SPREAD_REF`,
`bake_ring_masks`, `analytic_ring_mask`, `ring_disc_energy`, `ring_slice_for`,
`ring_mask_sample`, the six FFTs and the `RING_SLICES × RING_RES²` float table, the
`FlareBaked::ring_masks`/`ring_slice` fields, the GPU-side buffer and its trace-kernel
binding 9. A `f32` Fresnel number per combo replaces the `i32` slice index in the same
padding slot, so the struct layout is again unchanged. There is no per-path budget any more:
the closed form costs the same as the polygon it replaces, so every ranked path rings.

**The Fresnel number is computed per frame, not baked.** It moves with the working stop —
stopping down shrinks the ghost and the pupil together, so `F ∝ stop_scale²` — and both twins
derive it from the bake's measured spread by `ghost_fresnel_number`, mirrored in lumit-gpu as
`ghost_fresnel_of` under the usual twin rule.

**The phase argument is reduced by hand, and it has to be.** `v` reaches the low hundreds
deep inside a ghost, so `v²` reaches five figures and the phase `πv²/2` runs to tens of
thousands of radians. A CPU `sin` reduces that properly; a GPU one is not required to, and on
CI's real hardware does not — the twins disagreed by **1.25% of the frame's total energy**,
spread over every ghost interior, which is the shape of a range-reduction failure rather than
of a maths error. Both twins now take `v²` mod 4 first: one f32 multiply and a floor,
identical on both sides by IEEE, leaving each to ask for a sine of something under 2π.

**Recorded limits.** The fringes are computed at one wavelength (`RING_LAMBDA_UM` = 0.55):
their spacing goes as `√λ`, so across the visible band it varies by ±15%, far under the blur
they are already averaged by. They are uniform round the rim, which is right along a blade
and wrong at a corner, where two edges' diffraction would add. Neither is worth six times the
per-ray arithmetic; both are the recorded upgrade path if a corner ever shows.

**Existing projects' ghost edges change, so the effect's version goes 10 → 11.**

## K-371 — A coating is chosen per glass element, from a measured palette, and the panel offers one row per element the lens has

**DECIDED** (2026-08-13). Extends [K-356](#k-356) (multi-layer coatings by transfer matrix) rather than replacing it: the maths is untouched, what changes is who chooses the design.

**Why.** Owner: "Irl lens flares can have very different colours for different
ghosts. For instance it feels quite common seeing blue, purple, green and orange ghosts all
in the same flare. So it'd be nice to be able to change the coating per glass lens in the
lens, changing the number of them depending on lens used." That is right, and it is the
lens, not stylisation: a coated surface reflects whatever its coating fails to suppress, and
a manufacturer cuts different elements for different parts of the spectrum. Until now the
whole prescription took its coating from the `.lens` file's own column and a single global
Coating dial, so every ghost in a train was tinted by the same design.

**The element model.** A `.lens` prescription is a list of *surfaces*, but what a person
points at is a piece of glass. A row whose medium is glass opens an element; the row after
closes it; elements number front to back, as every patent diagram does. A cemented pair's
shared surface has cement on it rather than air, so it carries no AR coating in reality —
it goes to the **earlier** element, and nothing about a cemented interface deserves a
control of its own. `surface_elements` is that walk and `element_count` its total; the
bundled library runs 4 elements (Tessar) to 18 (Canon 70-200).

**The palette is measured, not asserted.** Each entry is a textbook design of its order —
real recipes are manufacturer secrets (K-356) — and the residual was measured across
420–680 nm at normal incidence, keeping only designs that are both distinctly coloured and,
as a real coating is, dimmer than bare glass everywhere. Band split of the reflected energy,
against bare crown glass at a flat 0.04:

| # | design | peak R | r / g / b | reads as |
|---|---|---|---|---|
| 1 | uncoated | 0.040 | flat | bright neutral |
| 2 | MgF₂ quarter, 520 nm | 0.018 | 0.38 / 0.35 / 0.27 | straw |
| 3 | MgF₂ + Al₂O₃ quarters, 520 nm | 0.019 | 0.55 / 0.12 / 0.32 | magenta |
| 4 | broadband W, 520 nm | 0.004 | 0.37 / 0.38 / 0.25 | green, faintest |
| 5 | broadband W, 480 nm | 0.013 | 0.81 / 0.09 / 0.09 | amber |
| 6 | broadband W, 560 nm | 0.025 | 0.07 / 0.14 / 0.79 | blue |

Entry 0 is "As the lens file" and is the default, so an untouched panel bakes byte-for-byte
the picture it always did.

**A finding worth recording.** The palette writes its stacks out rather than reading
K-356's `coating_stack` layer ladder, because that ladder's 2-, 4- and 6-layer rungs —
"the same W with extra quarter-wave pairs beneath, broadening it further and lowering the
mean" — measure **0.06 to 0.31 peak reflectance, brighter than bare glass**. They are a
plausible-looking extension, not a real design. Nothing exercises them today: every bundled
prescription's coating column is 0 or 1, and the palette avoids them. Correcting the ladder
itself is left as its own change with its own measurements.

**It is a bake input.** The per-element choices resolve into the surface table before the
reflectance table is built from it, and `bake_key` folds them in, so changing one rebakes
exactly as changing the lens does — 0.2 s release. The resolved design rides in the surface
row's former padding slot, so the WGSL mirror's stride is unchanged and the shader, which
only ever reads the baked table, needs no change at all.

**How the row count follows the lens, without a new frontend mechanism.** The schema
declares twenty rows — the ceiling — each in its own single-member group carrying
`visible_when_lens_elements`. That field never crosses the bridge as itself: group
visibility is *already* resolved in the panel from a live sibling value, so
`list_parameter_groups` turns each threshold into exactly that shape — the sibling is the
Lens dropdown, the values are the lenses whose prescription has enough elements. The panel
implements one visibility rule, not two, and learns nothing about optics. A four-element
Tessar draws four rows; the Canon 70-200 draws eighteen.

**Recorded limit.** A user's own `.lens` file overrides the Lens dropdown, and only the file
knows its element count, so the rows offered then follow the *picked* lens. An element with
no row keeps the file's own coating, which is what an untouched row does anyway. Lifting it
needs a per-instance schema query, which the effect-schema seam does not have and which is
not worth inventing for one control.

**No version bump.** Every row defaults to "As the lens file", so an existing project's
picture is unchanged, and the effect's stored values gain rows that read as their default
when absent.

**New strings — a Crowdin upload is owed**: the twenty `fxElement1…fxElement20` row labels
and the seven `fxCoating*` palette options.

## K-372 — The adaptive tier is playback's alone: a still frame is rendered, and named, at the scale it was asked for

**DECIDED** (2026-08-13). Fixes an owner-reported slowness; narrows [K-186](#k-186) (the adaptive resolution tier) to the case it was designed for.

**What the owner saw.** "After pre-rendering a bit of the comp, scrubbing the playhead to a
different point that was pre-rendered still took time to try and render the frame again for
some reason, which really shouldn't be happening and it makes the program feel very slow and
sluggish." Correct on every count, including that it should not be happening: the frames
really were there.

**The cause.** `publish_zero_copy` — the display path for a **scrub**, a drag preview, and
the republish after a lens bake — scaled the requested preview scale by the adaptive
playback tier:

```rust
let effective = if matches!(mode, Adaptive) { scale * tier_scale(tier()) } else { scale };
```

Three facts make that a cache miss on every scrub:

1. The frontend sends `Adaptive` for a **still** frame whenever the user's Playback
   preference is Adaptive, which is the default (`requestFrame` in `main.dart`).
2. The tier is a *playback* verdict and **survives the run that set it** — it is reset at
   the start of the next `play`, not at its end. So a single heavy pass leaves the tier at
   Half or Quarter for as long as the editor sits still.
3. The idle cache fill names its frames from the raw `last_shown` scale, with no tier.

So after any heavy playback pass, the fill banked frames at `scale` while every scrub asked
for `scale × tier`. Under content keying those are **different frames** (the quality tag is
part of the name, K-178), so the fill's copy was invisible to the scrub that wanted it and
the picture was composited from scratch — while the cache bar showed green over it, because
the fill's copy really was there. Both halves of the machine worked correctly and neither
could help the other.

**Why the line existed.** It predates [K-181](#k-181), which moved playback into the worker.
Playback now has its own ring and applies the tier itself, in `play_one_frame`, read beside
the cost it is about to explain. Nothing that reaches `publish_zero_copy` is playback any
more, so the tier there had stopped doing its job and was only doing damage. Its comment
still argued the old case — "the only display path" — which it had not been for some time.

**The rule.** The tier buys a cheaper composite so a *run keeps time*. Nothing still is being
paced, so a still frame is rendered at the scale it was asked for. Two named functions say
which is which — `still_quality(scale)` and `playback_quality(scale, mode, tier)` — and the
fill and the display path both call the first, so they cannot drift apart again. The tier is
**passed** to the second rather than read inside it, so the distinction is visible where the
trade is made and testable without touching the process-wide controller.

A still frame also now reports `FINEST_TIER` to the frontend rather than whatever playback
last settled on: it was made at Full, and the resolution badge should not claim otherwise.

**A separate slowness is left standing, and named here so it is not rediscovered as this
one.** The idle fill composites one frame per turn and is not interruptible, so a scrub
arriving mid-frame waits for that composite to finish — up to a couple of seconds on a comp
with a Lens flare. It is gated behind a 200 ms lull, so a continuous drag never meets it, but
a pause-then-scrub does. Fixing it means cancelling work already handed to the GPU, which
`docs/14-ENGINEERING-RULES.md` asks for in general and the flare's render pass does not yet
offer; it is its own change with its own measurements.

## K-373 — The splat tent reaches a full grid step, because a half-step tent reconstructs its own sampling grid

**DECIDED** (2026-08-13). Fixes the reconstruction half of [K-366](#k-366--the-ghost-grid-is-splatted-per-ray-not-rasterised-as-quads); its per-ray splatting model is unchanged and correct.

**What the owner saw**, and what the earlier fix did not touch. After K-370 removed the
ring-mask ladder, a frame still showed "the interference pattern ... clearly visible at all
options": a fine woven cross-hatch over the whole picture, a bright cross through the big
ghost's centre along the two pupil axes, and stepped edges on the ghost rims. That is not
diffraction. It is the sampling grid, printed on the image by the reconstruction filter.

**The bug, in one line.** `ray_axes` returns **half**-axes — a full step between neighbouring
rays is `2·a1` — and K-366 gave the tent a support of `±a1`. Two neighbouring tents therefore
met exactly at the point where both had fallen to zero.

A linear B-spline partitions unity only when its support is **twice** the sample spacing:
tents at spacing `h` must reach `±h`. At `±h/2` they do not overlap at all, and the sum is a
lattice of separate pyramids with a seam of zero along every cell boundary. Measured on a
uniform sheet of identical rays — the case whose answer must be flat — the reconstruction ran
from 0.0029 to 0.1436 about an expected 0.0469: a **49× peak-to-trough ripple**, 206% worst
deviation. The regression test asserts flatness to 2% and fails at that figure without the
fix.

**Energy was conserved throughout, which is why nothing caught it.** The tent integrates to
the parallelogram's area either way, so every flux test, every oracle mean and every
bit-stability check passed while the artefact was plainly on screen. A conservation law is
not a smoothness law, and the flare's test suite had the first and not the second.

**The fix** is to give the tent the reach a partition of unity needs: `±2·a1`, i.e. one full
grid step each way. The integral grows by four with it, so the peak is divided by four and
the deposited flux is unchanged; `area`, `MIN_AREA_FRAC` and the density cap keep their K-366
meaning in half-axis units, untouched. On the GPU the splat quad doubles and the fragment
tent is unchanged — its `uv` still runs `±1` at the quad's corners, but that corner now sits
on the *next ray along*, where its own tent is at full height.

**It costs four times the fragments per splat**, on the effect's hottest raster. That is the
price of a reconstruction that does not print its own sampling grid on the picture, and it is
the sort of cost `docs/13-PERFORMANCE-RULES.md` exists to measure rather than guess at — the
per-frame budget should be re-timed on a real card.

**A test was recalibrated, and honestly.** `lens_flare_an_area_source_does_not_replicate_its_ghosts`
asserted that an area source's ghost profile shows no more peaks than a *point* source's. That
held only while the grid seams were flattening the profile. A bar-shaped source's ghost is a
bar, and a cut across a bar shows both of its rims where a point's small ghost shows one
summit — so the comparison was never of the same object. It now measures against the
replication it was built to rule out, which is the like-for-like control the test already
constructs.

## K-374 — A group's lens-element visibility names `lens_model`, and an unreachable threshold says "never" rather than "always"

**DECIDED** (2026-08-13). A defect in [K-371](#k-371) as shipped, fixed with the test that would have caught it.

**What the owner saw**: "trying different lens coating options it didn't seem to do anything,
but ... it also looked like only coating's 19/20 were available." Both halves are one bug, and
the second explains the first.

The bridge resolves each element row's threshold into the visibility rule the panel already
has — sibling parameter plus the values it may hold — and it named that sibling **`"lens"`**.
The Lens dropdown's id is **`lens_model`**. A sibling that does not exist fails *silently*:
the panel looks it up, finds nothing, and hides every row that names it. The rows that
survived were elements 19 and 20, whose threshold no bundled lens reaches (the deepest is 18)
and whose value set was therefore **empty** — and an empty set means "always visible". So the
only rows on screen were the two that govern elements no lens has, which is why changing them
did nothing.

Two fixes, and a test each:

- the id is spelled once, as `LENS_PICK_PARAM`, and a test asserts the schema declares it —
  along with every element row, its palette, and that each threshold governs the row it names;
- an unreachable threshold now emits a set holding one impossible index rather than an empty
  one, so it reads as "never" in the vocabulary the panel already has. A test pins that the
  number of never-drawing rows is exactly the schema's ceiling less the deepest bundled lens.

**The general lesson, recorded because the seam invites it**: every id crossing to the
frontend by *name* wants a test that the name resolves. Nothing in the type system connects a
string in `lumit-bridge` to a `ParamSchema` in `lumit-core`, and the failure mode is a control
that quietly is not there.

## K-375 — The flare's splats accumulate in f32 through a compute pass, not in fp16 through the blender

**DECIDED** (2026-08-13, owner's choice between three costed options). Replaces the raster deposit half of [K-366](#k-366--the-ghost-grid-is-splatted-per-ray-not-rasterised-as-quads); the per-ray splatting model and [K-373](#k-373)'s tent are unchanged.

**The defect.** Every ray's flux is deposited over a footprint, and a bright pixel takes
contributions from thousands of them. That accumulation was done by the raster blender,
additively, into the flare buffer — which is `WORKING_FORMAT`, `Rgba16Float`. **Adding a small
increment to a large fp16 running sum loses everything below half an ULP of the sum**, and
that is a systematic loss, not jitter that cancels: the brighter the pixel, the more of each
further contribution disappears.

Measured against the f32 CPU reference on the padded-anamorphic oracle: the middle of the
frame 4.5% dim, the border ring 0.7%, the outer fifths 0.1% — tracking local brightness
exactly as the mechanism predicts, and growing with the number of contributions per pixel
(K-373 quadrupled that count and the deficit grew ~3.5×, which is what identified it). It had
been there since K-366 and read as a mysterious 1.2% oracle gap.

**Why this option and not the other two.** `Rgba32Float` blending needs the
`FLOAT32_BLENDABLE` device feature, which this build does not request and which is not
universally available — it would either raise the hardware floor or make the picture differ by
machine, which is the determinism [K-353](#k-353) fought for. Restating the oracle's bound
would have recorded the loss rather than fixed it, and the loss is real: the GPU was
systematically darkening its own highlights.

**The mechanism.** The sum is accumulated in **f32** in a pooled storage buffer, three channels
a pixel, and written into the fp16 texture **once**, by a resolve pass, at the end. One
rounding instead of thousands. The texture stays fp16 — a single stored value has precision to
spare; it was only ever the accumulation that was short, so nothing downstream (blur, combine)
changes at all.

**The sum is fixed point, and the first attempt at this was wrong.** WGSL has no float
atomics, and the obvious substitute — a compare-and-swap loop over the f32's bit pattern — is
exact per add but leaves the *order* of the adds to whichever thread wins the slot. Float
addition is not associative, so the same document rendered two different pictures. CI caught
it immediately (`an area source must be bit-stable too`), which is [K-353](#k-353) doing its
job.

Integer addition **is** associative and commutative, so `atomicAdd` on a u32 gives a sum
independent of thread order. The accumulator is therefore fixed point at 2^18 steps per unit
of radiance: every deposit is rounded to the nearest 3.8e-6 and added exactly. That is
*unbiased* rounding against the blender's systematic truncation — better precision where it
mattered, and reproducible, which the float version was not. Radiance is never negative, so
the sign bit is spare range rather than a missing case.

The cost is a ceiling: above 16383.99 in a channel the u32 **wraps** rather than saturating,
because detecting the overflow would need the compare-and-swap whose order dependence this
design exists to avoid. The margin is the safety, and it is measured rather than asserted — a
test renders the CPU reference at four times intensity with no ghost blur and requires the
brightest pixel to sit at least 100× under the ceiling, so the scale is revisited before a
user ever sees a wrapped highlight.

**A twin, not merely an analogue.** `deposit` mirrors `lumit_core`'s `splat_ray` op for op —
same bounding box, same inverse 2×2, same tent, same order — which the raster could not, since
its pixel selection was the rasteriser's fill rule rather than the reference's `|u| < 2` test.
The oracle should therefore agree more closely than it ever has, and the exact figure is
CI's to report: this machine has no adapter, and `tests/wgsl_validates.rs` (naga, no card)
is what checked the new kernel here.

**What this deletes**: the whole render pipeline, its bind group layout, its
`fx_lens_flare_draw.wgsl`, and the additive `BlendState` that was the actual bug.
`Scratch` gains the accumulator, pooled and size-bounded on the same rule as the rays and
splats, and cleared per frame with `clear_buffer`.

## K-376 — The splat kernel is a quadratic B-spline, because a tent's creases are visible too

**DECIDED** (2026-08-14). Continues [K-373](#k-373); the partition-of-unity requirement it established is unchanged, and this is about what *kind* of partition.

**The owner, on a build carrying K-373**: "genuinely feels like the flares are getting worse.
That grid pattern is still absurdly visible." K-373 was right and not sufficient, and the gap
between those two is worth recording, because the test that passed is the reason it was missed.

**Why the flat-sheet test passed while the artefact stayed.** K-373's regression test lays
down a *uniform* lattice of identical rays and requires the reconstruction to be flat. A tent
handles that case exactly — it is the case a tent is designed for. A real ghost's rays are
neither uniform nor axis-aligned, and there the tent's weakness shows: a linear B-spline is
only **C⁰**, so the reconstructed surface has a crease along every cell boundary. The eye finds
creases; it is the same Mach-band sensitivity that makes a polygon silhouette visible in
otherwise smooth shading. A test on the easy case proved a property that did not transfer.

**Measured on a real frame** — each pixel's departure from its own 3×3 mean, relative to that
mean, in two brightness bands:

| kernel | bright | dark |
|---|---|---|
| K-366, tent at half a step (what the owner photographed) | 15.8% | — |
| K-373, tent at a full step | 2.42% | 4.59% |
| **K-376, quadratic B-spline** | **1.91%** | **3.81%** |
| cubic B-spline, for comparison | 1.87% | 3.72% |

The quadratic — `3/4 − t²` inside a half step, `(3/2 − |t|)²/2` out to one and a half — is the
standard answer to this exact symptom in the simulation literature, where it is called grid
imprinting or cell-crossing noise. It partitions unity like the tent and is C¹. The cubic buys
almost nothing further for sixteen cells a splat against nine, so it is recorded here and not
taken.

**What is left is not the kernel.** The dark band settles at about 3% and **stops falling when
the rays are multiplied eightfold**, which sampling noise would not do — so it is the flare's
own fine detail (iris rims, overlapping faint ghost tails) being read by a metric that cannot
tell detail from artefact, not a residual grid. The new test's thresholds sit just above the
measured figures for that reason, and say so.

**The measurement is now a test.** `lens_flare_reconstruction_does_not_imprint_its_own_grid`
measures a real frame rather than a synthetic sheet, which is the thing K-373 failed to do.

## K-377 — The accumulator's fixed-point scale is sized from what the buffer holds

**DECIDED** (2026-08-14). Corrects a number in [K-375](#k-375).

K-375 picked 2^18 steps per unit of radiance and justified the ceiling — 16384 — as "orders
above anything a frame produces". It is: the flare buffer measures a **peak of 0.042** and a
**median lit pixel of 0.0028** on the bundled default, because the auto-exposure normalises it
there (K-258). That makes the ceiling four hundred thousand times the peak — range spent on
headroom no frame will ever reach, and paid for in resolution exactly where the picture is
dark and banding is most visible.

2^24 keeps a ceiling of 256 — still six thousand times the measured peak, and a thousand times
it at fourfold intensity — and drops the quantum to 6e-8, a fifty-thousandth of the median lit
pixel. The margin is measured by the same test as before rather than argued.

The lesson is the one K-375 should have taken: the range was chosen before anything was
measured, and a measurement was a two-line print away.

## K-378 — Area-source sampling the splat reconstruction can actually follow

**DECIDED** (2026-08-16). Amends [K-367](#k-367) (per-ray source integration — the
mechanism stands) and [K-366](#k-366)/[K-373](#k-373) (the footprint rule changes).

**The owner, with matte sources on footage**: "until this is fixed (the grid looking
stuff) the lens flare is unusable — Normal still needs to be usable and currently it is
completely unusable even at ultra." Reproduced exactly on the CPU reference: give the
default lens any source extent and every ghost wears a woven mesh; a point source is
clean. K-367's integration was right about the integral and wrong about what the
reconstruction could survive, in three separable ways, each fixed at its own site:

1. **Footprints averaged where they should cover.** The source offsets hop by more than
   the whole source between pupil neighbours — that is what equidistributes them — so a
   ray's two neighbours regularly land on the *same side* of it, and K-366's central
   difference (their average) cancelled toward zero: a collapsed splat sitting between
   two wide gaps, quasi-periodically across the ghost. `ray_axes`/`build_splats` now
   take the **longer one-sided difference** per axis. On a smooth map the sides agree
   and this is the central difference it was; under jitter, under-coverage becomes
   impossible and the cost is overlap, which is only blur. (A smooth low-frequency
   jitter that central differences could follow was tried first and rejected: it
   couples source position to pupil position, and a big softbox rendered as folded
   zigzag sheets.)

2. **1/ρ² is not a rotation.** K-367 took the plastic constant's 2D low-discrepancy
   pair (1/ρ, 1/ρ²), but each constant drives its own axis by its own index, so each
   must be a good **1D** rotation alone — and 1/ρ² = 0.5698 sits within 0.002 of 4/7,
   so its samples fall into seven combs that precess too slowly to wash out across a
   pupil grid: stripes down every ghost, on either axis it drives. `PHI_V` is now
   **1/ψ = 0.6823278** (supergolden; ψ³ = ψ² + 1) — the same family of cubic Pisot
   units as 1/ρ, rationally independent of it, and tied-best of a scanned battery on
   the stripe metric. (The golden ratio, the textbook best rotation, was measured and
   is *worse* here — the triangle wave's reflection symmetry doubles its coincidence
   structure into a mesh. The battery beat the theory; the test pins the measurement.)

3. **Every band re-traced the same source points.** Bands splat independently and sum,
   so each now samples the source at its own phase (`PHI_BAND` = 0.618 per band):
   band-count × the effective source sampling for free, and each band's residual
   ripple averages toward the mean instead of reinforcing.

Measured on the new `lens_flare_an_area_source_renders_without_stripes` (a 9×9
grid-imprint metric — K-376's 3×3 provably cannot see this artefact: the mesh's period
is the ray spacing, several pixels, and the K-367 mesh *passes* K-376's bound while
plainly visible): **7.3% → 1.1%**; the visual mesh is gone at every quality including
Draft. A point source is bit-identical throughout (zero extent, zero offset, every
band). The area-flux test's floor widens 0.98 → 0.94: the wider footprints spread a
little further, and on a 192-px test raster a few percent of the smear honestly crosses
the frame edge — measured 1.007 on a padded buffer that catches it.

## K-379 — The submission pacing counts the deposit, not just the trace

**DECIDED** (2026-08-16). Extends [K-263](#k-263)'s submission split and [K-375](#k-375)'s
deposit.

**The owner**: changing a flare setting "starts lagging my entire pc… it ended up
freezing the whole pc until it crashed lumit and closed vscode with it, after the pc was
frozen for like a good 2 minutes." That is the K-263 device-loss failure back in a new
coat: the frame IS split into submissions, but the split is paced by
`Batch::steps` — the trace's ray–surface count — and since K-375 the deposit is a
compute scatter whose cost the trace count cannot see. A splat deposits over its own
footprint, so a ghost's deposit costs about **nine times its image area in atomic
adds, per combo per light, however many rays sample it** (kernel support of 1.5 grid
steps each way = ~9 cells of overlap). For a frame-filling defocused ghost on a 1080p
buffer that is ~40 M atomics per combo-light — and a Normal frame of sixty ghosts,
eight bands and a handful of matte sources packs *tens of seconds* of scatter into
submissions the step model priced as milliseconds. The watchdog kills one, the device
dies, the session is over; until it dies, the saturated queue starves the desktop
compositor, which is the whole-PC freeze.

Two changes, both in the plan (testable without a card, like the rest of it):

- `combo_deposit_cost(spread × stop_scale, flare diagonal px)` estimates each combo's
  deposit pixels from its bake spread — over-counting elongated ghosts, which errs the
  safe way — and `plan_flushes` paces on trace steps + deposit pixels, treating one
  deposited pixel as one step (both are a few dozen operations).
- `plan_batches` caps a batch's (combo × light) slot count so one batch's deposit
  stays about one `STEPS_PER_SUBMIT`, because a batch is the atomic unit of encoding
  and a flush between batches cannot split the inside of one. A batch of one combo and
  one light is always allowed — it is itself about that size for the biggest ghost a
  padded 4K buffer holds, which is the floor the bound rests on.

This bounds every submission; it does not make the work smaller. The deposit's total
cost — 9 × ghost area × combos × lights, independent of ray count — is the recorded
follow-up: deposit large splats into coarser accumulator levels and upsample at
resolve, so a splat's cost is capped whatever its size.

## K-380 — Big splats deposit into a pyramid, so a splat's cost is capped

**DECIDED** (2026-08-16). Delivers [K-379](#k-379)'s recorded follow-up; continues
[K-375](#k-375)/[K-376](#k-376), whose accumulator and kernel are unchanged in kind.

K-379 bounded the *submissions*; the *work* was still nine times each ghost's image area in
atomic adds per combo per light, independent of ray count — the deposit kernel spans three
grid steps, so a frame of defocused ghosts pays its own area tens of times over. That is
what "Normal still needs to be usable and currently it is completely unusable" costs, and
the owner named the remedy in the same breath: "use the grid to speed things up then smooth
it."

The accumulator becomes a level pyramid — level 0 the flare buffer, each level ceil-halving
both axes, ~1.33× the pixels in total. A splat whose kernel span exceeds
`DEPOSIT_SPAN_PX` (48 px) deposits at the shallowest level that brings it under, everything
scaled into level pixels; the resolve bilinearly upsamples and sums the levels. The kernel,
the floors, the fold guard, the density cap and the fixed-point accumulation are all
untouched — the peak is a density, and a density survives resampling — and level 0's
read-back is the identity, so small-splat frames render exactly as before. The smoothing a
coarse level costs is about a twenty-fourth of the splat's own size, on splats whose own
softness is far coarser. A splat now costs at most about `DEPOSIT_SPAN_PX`² pixels.

**Measured**: the frame-cost harness (960×540, Normal, 60 ghosts) went **1.15 s → 87 ms**
per frame on the development machine — thirteenfold, and the difference between a flare
dial that saturates the card and one that scrubs.

Two implementation notes that cost an afternoon and are pinned in the impl note so they are
not re-learned: FXC refuses dynamic indexing of uniform-buffer arrays (the level dims are
derived in-shader as `ceil(raster / 2^level)` — provably what iterated ceil-halving
produces — instead of being passed as a table), and refuses l-value indexing of local
vectors (the resolve taps whole `vec3`s). Both failures presented as a silently black or
garbage flare with the validation error only visible on stderr. The accumulator's old
clamp to the scratch budget is also gone: level 0 must hold every pixel of the flare
buffer, and the clamp was silently truncating it past 2K rasters.

## K-381 — An effect is declared once, and the catalogue is a list of values rather than a variant of an enum

**DECIDED** (2026-08-13). Adopts the substance of issue #16 (Airyzz, "Proposed change to how
effects are stored") and supersedes the build pattern [K-099](#k-099) names — "the usual four
sites (schema in `lumit-core`, WGSL kernel + `FxEngine` method in `lumit-gpu`, `run_ops`
arm)". The four sites become two: the effect's own file, and one line of a list. Nothing
about an effect's *maths* changes; K-099's parity requirement (§1.6's CPU oracle) is
untouched and is what proves each migrated effect still renders the same picture.

**The complaint, and it is fair.** An effect was written down five times: a schema literal in
`builtins.rs`, a variant of `fx::Resolved`, an arm of `resolve_one`, an arm of `cpu::apply`,
an arm of `run_ops` — six with `rescale_px`, for anything holding a length. Three of those
matches were exhaustive, so `Resolved` was a compile-time chokepoint: every effect in
existence had to be known to `lumit-core` when it was built, which is a strange property for
a program that intends to host OFX plugins (docs/12) and, in time, effects the user writes.
The sets had already drifted — `spectral_split` had a variant and no schema entry;
`posterize_time` and `accumulation_mb` had entries and no variant — and nothing caught it.

**What was disagreed, and how it resolved.** The owner's objection to the original sketch was
performance and determinism: `Resolved` is `Copy` plain-old-data, built once per effect per
frame, and replacing it with `serde_json` would allocate on every frame and look up every
parameter by string. That objection stands and is met, not overruled. The resolved form is a
**bag of key/value pairs** as the issue asked, but the key is a `ParamId` — a `u64` FNV-1a
hash of the stable id, computed in a `const fn`, so a built-in's ids are compile-time
constants and a lookup compares two integers. The pairs live in a **per-stack arena**: one
allocation for a whole layer's stack, which is one *fewer* than the `Vec<Resolved>` it
replaces, and `ResolvedFx` borrows a run of it and stays `Copy`.

The fixed-size stack array the thread landed on was tried and rejected on the numbers: the
Lens flare declares 50 parameters, so the array would be sized for it, and a one-parameter
Blur would carry 1.2 kB of unused slots — while a user-added spare parameter could still
overflow it. The arena has no cap, which is the property the dynamic half of the issue needs.

**Determinism.** `Resolved` fed the frame key byte-wise (K-143). `Value` has padding, so the
arena is hashed **field by field** instead — tag, then live bytes, in stack order. That is
stronger than what it replaces: a byte-wise hash silently changes meaning when a variant
grows a field, and this one cannot.

**Registration is a written list, not a `ctor`.** The issue proposed self-registration before
`main`; the thread withdrew it and this confirms the withdrawal, for a reason that is not
start-up time: the Add-effect menu, the command palette and the preset browser are all
catalogue-order-driven (K-137), and a `ctor` makes that order depend on link order. One file
lists the effects, one line each, in menu order. Effects that are *not* known at compile time
arrive through the same `EffectDef` trait object at run time — which is the whole point, and
is the seam OFX will use.

**Units are declared.** Every numeric parameter says what its number means (`Raw`, `PctDiag`,
`Px`, `Degrees`, `Seconds`), which turns `rescale_px` from a match that had to know which
field of which variant held a length into one generic pass. An effect can no longer forget to
be rescaled — a preview raster and a full-size export disagreeing was previously one
forgotten match arm away. docs/08 §2.3 is unchanged: `Px` means px@comp, and "pixels of
whatever buffer I was handed" is still forbidden.

**Dynamic and spare parameters** (the issue's second and third parts) are what the bag is
*for*, and their rules are recorded in docs/impl/effect-registry.md §4: nothing is added or
removed automatically, keyframes outlive their parameter (K-065's "keep what you do not
understand"), and the cache key feeds the derived *shape* — ids and kinds, in order — as well
as the values it already fed, because a shader edit that changes which uniforms exist is a
different effect at the same version.

**Migration.** Batch by batch, simplest family first, each batch a commit that leaves CI
green, the awkward ones (`dof`, `shake`, `lens_flare`, `matte_key`, `motion_blur`,
`datamosh`) each on their own. While it runs, a test holds every generated declaration to be
field-for-field identical to the hand-written literal it replaces, which is what makes the
port mechanical and checkable rather than a rewrite; it is deleted with the last batch. A
commit that migrates an effect and changes its output is two commits, and the second one
needs an entry here.

## K-382 — Three overlaid waves need a paint order and a fan

**DECIDED** (2026-08-06, landed 2026-08-17 — the commit sat unpushed). K-284 put the
multiwave's three bands in one lane, which is right, but drawing them concentric and
brightest-last was two mistakes at once.

**Paint order: treble first, bass last.** The bands were drawn dim to bright, so the palest
one landed on top — and a pale shape over a darker one does not read as two shapes, it
swallows what is under it. Reversed, every darker band sits in *front* of a paler one and the
layering is legible. Fixed by band index rather than worked out from the colours: a theme may
put the ramp anywhere, and a picture that reorders itself depending on the palette is a
picture nobody can learn to read.

**A fan, not a stack of concentric rings.** Three bands of one sound agree with each other
most of the time — that is what makes them one sound — so drawn about a common baseline they
hide inside one another wherever they agree, which is exactly where the reader is trying to
tell them apart. Each band is therefore lifted slightly above the one behind it: 8% of the
row's height, clamped to 1–4 px, so a 22 px lane fans by under two pixels and a tall clip by
four. The lift is taken out of the wave's own reach rather than off the top of the row, so a
full-scale signal still fits. It applies to both baselines (K-285) — centred, the three
centre lines fan up; standing on the floor, the three floors do.

Both are drawing decisions only: nothing about the fetched peaks changes, and the single wave
is untouched.

**And the easter-egg plate is uploaded once.** It was decoded, linearised and uploaded on
*every dispatch* — with the full-resolution plate that is ~3 MB of decode and ~16 MB of
upload per frame, which starves the device until unrelated bind groups fail to create. The
symptom is a preview that stops rendering and repeats a validation error naming some other
pass entirely, which is a miserable thing to debug from. It is held in a `Mutex` on the
`FxEngine`, the same shape as the Lens flare's bake cache and for the same reason.

## K-383 — A live drag renders the Viewer small

**DECIDED** (2026-08-08, landed 2026-08-17 — the commit sat unpushed; renumbered from
K-316, which was already taken). The drag preview is capped at a 640×360 raster, and
the full-resolution frame comes back on release. Dragging a Depth of field or Lens dirt parameter left the picture one
to five seconds behind the pointer, which makes those effects untunable — you
cannot judge a blur by a frame that arrives after you have already let go. Every
other editor solves this the same way, and so does Lumit now: while a value is
being dragged the frame is rendered at a coarser resolution, and the moment the
drag commits the ordinary render path puts a full-resolution frame back.

**The mechanism already existed; only the decision to use it was missing.** The
§2.3 preview factor threads through the whole resolve step — every px@comp
parameter is scaled by it, K-266 and K-268 repaired the two places it leaked —
so a smaller drag raster frames identically to the export, only softer. Nothing
new was built: `render_comp_with_preview` simply divides the scale it was handed
before publishing.

**It is a decision taken before the first tick, not the adaptive tier.** The
realtime controller (K-030/K-171) learns from a dozen measured frames and is
deliberately bypassed on the drag path; a drag is usually over before it has
finished learning, so the first drag on a heavy comp would still stall — which
is the entire complaint. So the drag rule is a pixel budget instead: the finest
divisor of 1/2/3/4 whose raster fits 640×360, floored at Quarter because below
that the picture stops being judgeable. A 1080p comp at full scale drags at a
third; a comp already under the budget, or one shown small in its panel, is not
degraded at all. The budget is anchored on B3 of `docs/13-PERFORMANCE-RULES.md`
(50 ms from input to a possibly degraded frame) and the fact that a gather
kernel's cost falls with roughly the *fourth* power of the scale — its radius is
px@comp too — so a third of the scale is nearer a hundredth of the work.

**It lives in the worker, not in Dart, because every call to
`render_frame_with_preview` is by definition a live drag.** A release commits and
returns through `render_frame` at the Viewer's own scale. So the reduction needs
no flag from the frontend, cannot be forgotten by a new call site, and covers
every drag the frontend has at once: effect parameters, transform rows, masks,
shape contents, text, paint, and the Viewer gizmos.

**What is not done:** progressive refinement *during* a held drag — pausing
mid-drag with the button still down keeps the coarse frame until release. That
needs a per-gesture idle timer at every drag call site, which is a new mechanism
for a case the release already covers a moment later.

## K-384 — Radial blur's Amount rescales with the preview raster, because it was always pixels

**DECIDED** (2026-08-17). The one output change in the registry migration's blur batch,
recorded on its own because **no effect's maths changes in this refactor** is the batch
rule (docs/impl/effect-registry.md §6) and this is the exception that proves it.

The old `rescale_px` listed Radial blur as "nothing in pixels" and left `amount_px`
alone — but `amount_px` is raster pixels, and the kernel divides it by the actual
raster's half-diagonal. So a stack resolved at comp size and re-run on a reduced
preview blurred too far by the inverse of the preview factor: a K-266 miss, invisible
at full size and on every oracle (which render one raster), visible as a preview that
blurs harder than the export.

Declaring the parameter's true unit (`PctDiag`) fixes it structurally: the resolve
step converts, the generic rescale moves every spatial value, and the old behaviour
is no longer expressible — which is also why this rode the migration commit rather
than landing as the two commits §6 asks for. The unit system cannot state the bug.
`the_stylise_family_rescales_once_in_each_unit` and
`a_migrated_spatial_parameter_rescales_as_the_old_op_did` pin the corrected rule.

## K-385 — An effect that needs the clock asks for it through one resolve-time hook

**DECIDED** (2026-08-17). Unblocks the registry migration for the effects the blur and
stylise batches held back — Flash, Scanlines, Block glitch, and the temporal family —
which derive values from layer time, the marker context, or a whole keyframe track:
things the parameter bag cannot carry, because they are not parameters.

`EffectDef` grows one optional method, `resolve_derived(cx, push)`, called by the
generic resolve after the declared parameters. It sees exactly what the old
`resolve_one` arms saw (the instance, layer time, the raster diagonal, the §2.3 preview
factor, and the marker and expression contexts) and its only output is values pushed
into the bag under derived
`ParamId` constants that live beside the effect's schema ids. Derived values are never
panel rows, never keyframed, never serialised — they are recomputed every resolve, as
the old arms recomputed them. The frame key is untouched: time is the frame's identity
and markers are document state it already covers.

The alternative — leaving a bespoke match for "the time-dependent ones" — would have
made resolve_one immortal, and the migration's whole point is that it dies.
docs/impl/effect-registry.md §2.4a carries the shape.

## K-386 — A wrong Unit::Raw found during migration is corrected where it is found

**DECIDED** (2026-08-17). Generalises [K-384](#k-384): the registry migration keeps
finding parameters whose old `rescale_px` treatment was wrong — raster-pixel values
listed as "nothing in pixels" that therefore never rescaled with the preview factor
(K-266's class of fault). Scanlines' period and Block glitch's sizes joined Radial
blur's amount in this batch.

The rule, so each instance does not need its own entry: when migration shows a
parameter's true unit and the old dispatch mis-declared it, the declared unit wins,
the module documents the change, and `every_parameter_declares_a_unit`'s golden list
pins it. The output change is the *correction* of a preview≠export disagreement, never
a new look at full resolution — an oracle that moves at full raster still means a bug.

## K-387 — A side-table effect declares which list it consumes

**DECIDED** (2026-08-17). The last seam of the registry migration. The render prepares
parallel input lists beside a stack (LUTs, layer inputs, neighbour frames, the flow
field, the flare's mattes and lenses), bound to ops by shared counters in stack order —
one predicate in build.rs, one order in run_ops, drift impossible. Under registry
dispatch that binding becomes a declaration: `GpuEffect::aux()` names the list, the
Registry arm advances the matching counter and hands the borrowed slot into `run`.
The build-side predicate keys on instance match_names and the consumption on def
match_names — the same names — so the one-predicate, one-order rule is untouched, and
a missing slot stays a passthrough. docs/impl/effect-registry.md §2.5a carries the
shape.

## K-388 — The bag gains a four-float value, and Shake's noise is derived unit-free

**DECIDED** (2026-08-17). The last migration blocker. Shake resolves nine sub-frame
wobble samples — four floats each — and `Value` had no shape for them: forty flat
`derived.*` ids would drown the bag, and abusing `Colour` would lie about what the
floats are. Two changes, taken together:

- **`Value::Vec4([f32; 4])`**: one variant, one `tag()` byte, one `feed_hash` arm
  (tag, then the four floats — the field-by-field rule of K-381 unchanged), one
  `Params::vec4` accessor. General, not Shake-shaped: the next effect with a small
  fixed vector uses it too.
- **Shake derives its noise unit-free and keeps its amplitude in the schema.** The
  old arm baked `amp_px` into every sample, which would have made the derived values
  spatial — and derived values do not rescale (`rescale_spatial` walks schema units).
  Instead `amplitude` declares its true unit (`PctDiag`, under K-386's rule: the arm
  hand-multiplied by the diagonal), so the arena holds a rescalable schema value, and
  `resolve_derived` pushes only the unit-free noise vectors. `packed()` reassembles
  `ShakeWobble::at`'s arithmetic — same association, same cast point — so frame-time
  output is bit-identical; on the rescale path the factor associates one multiply
  earlier, an ulp of the accepted narrowing class.

The alternative — teaching derived values to rescale — would have given `derived.*`
ids a second contract (a unit table living nowhere) for exactly one consumer.
Deriving the value that has no unit is smaller and cannot drift.

## K-389 — The harness measures in absolute numbers and CI gates on the ratio to a baseline

**DECIDED** (2026-08-18). Implements [13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md)
§7.3 on the hardware that exists. It supersedes nothing: §2's budgets remain the truth,
and this entry is about who is allowed to assert them.

§7.3 asks CI to run the harness "on a reference-desktop-class runner" and fail a build
that breaks a budget. There is no such runner. A GitHub runner is a shared virtual
machine whose graphics card is Mesa's lavapipe — a software rasteriser — and asserting
"a scrub shows something within 50 ms" there would fail for reasons that have nothing to
do with Lumit, on a machine that is not the one §1 names. §14's open question ("perf
gates need pinned hardware") is the same observation. Waiting for pinned hardware would
have meant no measurement at all in the meantime, which is how a budget quietly becomes
a slogan.

So the harness and the gate are separated:

- **The harness measures.** It emits one absolute number per budget as JSON
  (`{"budget":"B3","value_ms":…,"frames":…}`) and judges nothing. Absolute numbers stay
  absolute; they are simply not compared with the reference desktop's on a machine that
  is not it.
- **CI gates on the ratio to a checked-in baseline, per runner operating system.** A
  baseline is a previous run's own output, committed on purpose and regenerated the way
  `crates/lumit-core/fx-labels.txt` is: run the harness, replace the file, commit the
  change. A budget fails at **1.6x worse than its baseline**. That catches a real
  regression — a lost cache tier, an accidental full-resolution decode, a pipeline
  recompiled every frame are all factors, not percentages — while a noisy afternoon on a
  shared machine is not a factor. §7.3's "more than 10%" is the number for a quiet
  machine and stays the aim; 10% on a runner would be a coin toss, and a gate that fails
  at random is one everyone learns to re-run. A measurement under **1 ms** is never
  failed on ratio: serving a warm frame costs about ten microseconds, where a factor of
  two is the scheduler blinking and nothing a user could see hides underneath.
  Comparison across operating systems is refused outright rather than attempted.
- **The absolute budgets are asserted only under `LUMIT_REFERENCE_HW=1`**, which is set
  on §1's reference desktop and on nothing else — no runner sets it, and neither does
  the owner's development box, which is not that machine either. The day a self-hosted
  reference runner exists, setting the variable there is the whole change: §7.3's
  original design switches on, and the ratio gate keeps running beside it.

**The reference comp is built in code**, not committed. §1 describes it in a paragraph —
1080p60, twenty seconds, two H.264 footage layers with one retimed to 40% through flow, a
text layer, a Sequence layer of four clips, an adjustment layer carrying a 3D LUT and
curves, a glow, motion blur on two layers, a luma matte, an audio layer with volume
keyframes — and that paragraph is assembled through `lumit-core` and `lumit-render`
directly, with no bridge and no Flutter, over synthetic clips the harness asks ffmpeg for
at startup (the pattern the media tests already use). Committing forty megabytes of video
to a public repository to measure a cache was the alternative.

**Which budgets a headless harness can reach.** B3 (scrub latency), B4 (refine to full),
B5 (warm playback), B6 (cold adaptive playback), B7 (cold full-resolution playback) and
B11 (idle fill of the work area) are measured here. **B1 and B2** are UI-thread budgets
and need the real window; **B8** needs the encoder; **B9** is device loss; **B10** is A/V
drift. Those five remain manual or real-window checks and [TODO.md](TODO.md) keeps them —
naming them here so that "the harness is green" is never mistaken for "every budget is
enforced".

**Home**: `crates/lumit-bench`, a development crate. It is a workspace member so `fmt`,
`clippy` and `test` cover it, and nothing depends on it — the shipped library is
`lumit-bridge`, which has never heard of it — so it cannot reach the application's
dependency tree. Its binary is what CI runs (job `performance gates (ratio vs
baseline)`); each scenario is also an `#[ignore]`d test, for measuring one budget while
working on it.

## K-390 — Flow learns census matching and edge-held boundaries; the blur learns to scatter

**DECIDED** (2026-08-18). docs/impl/optical-flow.md §4.5 carries the full shape. The
measured brief (§5.5): flow beats a crossfade decisively on game capture and loses on
line art by worst-block SSIM, with the causes diagnosed — no evidence in flats, and
smoothing that diffuses across boundaries. The fix is method, not knobs: a ternary
census matching cost in the inverse search (illumination-invariant, edge-concentrated
evidence), a fast-bilateral-solver-shaped edge-aware densification of the field guided
by the picture with confidence as the data weight, and a Guertin-class feature-aware
reconstruction for Fast motion blur (tile-max/neighbour-max dominant directions,
confidence-weighted taps, adaptive counts, ±1 central difference, destination
fixed point, curved trails on High).

Model-free deliberately: learned flow stays a plugin-era question (§0's backend seam
already reserves the slot). Acceptance is the existing harness — worst-5% block SSIM
up on animation, strictly not down on gameplay — and the CPU/WGSL oracles stay
op-for-op through every change. Normal/High tiers ship regardless of cost; the owner's
one-second-a-frame ceiling decides whether High is ever the default.

**Postscript (2026-08-19): what shipped, and what was measured out.** The programme
ran to the end and **one of its three items ships**. Item 3, the Guertin-class blur
reconstruction, landed **in full** (K-392) — tile-max and neighbour-max dominant
directions, McGuire-weighted taps, adaptive counts, curved trails on High, and the
confidence reversal — with only the ±1 central difference deferred for a seam reason
K-392 names. Item 1 (census matching) and item 2 (edge-aware densification) are both
**reverted; the flow engine ships this round exactly as it entered**, SSD inverse
search and §1 step 3 densification, byte-for-byte in both backends. Item 1 missed the
bar by 0.0073 on game capture (impl §5.5.1); K-393's per-patch refinement of it moved
the miss to 0.0002 on the cinematic and still did not clear it (§5.5.2); item 2 lost
on four of the five clips and its code, its `FlowSettings::dense_iters` field, its
`VectorDetail` mapping and its WGSL kernel are deleted (K-391, §4.6). No shipped
`.lum` ever carried `dense_iters`, so removing it is a no-op for project
compatibility.

That is a **no-ship on the flow half, recorded as a result rather than a failure**.
The bar this entry set — worst-5% up on animation, strictly not down on game capture
— is the reason the tree is not now carrying a slower, three-branch inverse search
that trades the footage the project exists for (K-002) against line art. What the
campaign leaves is the blur, the harness (`flow_quality.rs`, `clip_cadence.rs`,
`blur_proof.rs`) and four sections of measurements: line art's problem is missing
evidence, field-space smoothing cannot manufacture it, census can but costs precision
where evidence is plentiful, and the two costs cannot be hard-switched between without
paying at the boundary. A later attempt starts from that instead of from a prediction.

## K-391 — Edge-aware densification is measured, and does not ship

**DECIDED** (2026-08-19). K-390's item 2 — a fast-bilateral-solver-shaped solve of the
finished flow field against the picture's luma edges, confidence as the data weight — was
built (CPU reference, WGSL twin, oracle parity green at both ping-pong parities), measured,
and **deleted**. docs/impl/optical-flow.md §4.6 carries the tables.

It fails K-390's own acceptance bar on four of the five owner clips, at every λ and ζ
tried. Worst-5% block SSIM moves +0.005 at best on the anime clip and −0.011, −0.013,
−0.023 and −0.016 on cartoon, gameplay, cinematic and synthetic; the gentlest setting that
keeps anything of the anime gain still costs gameplay −0.006, past the −0.005 no-regression
limit. Anime is the only clip whose best column is the one where the pass does *most*;
on the other four the optimum is the setting nearest to switched off.

The reason is structural rather than a tuning miss, which is why no further sweep is
owed: by this point the field has already been regularised by variational refinement
using real photometric evidence, and this pass regularises it again using none — only luma
similarity and the field's own values. It cannot add information, only exchange one
smoothing for another, so it can win only where refinement had nothing to go on and must
lose wherever refinement was right. This is the same trade §5.5 recorded and removed once
before (0.036 of gameplay for 0.012 of animation); the numbers are smaller and the verdict
is the same.

**Deleted rather than parked disabled**, which was this entry's first ruling and the wrong
one. A pass no setting reaches, a kernel nothing dispatches and a `FlowSettings` field no
shipped `.lum` can carry is a dead knob that every later reader has to re-derive the verdict
on; the tables in §4.6 are the reproducible record, and a future item 2 must be
**evidence-bearing** rather than field-space — a different pass, not this one re-enabled, so
it loses nothing by starting from the numbers instead of the skeleton. `dense_iters`, its
`VectorDetail` mapping, `densify_edge` and the ping-pong buffers are all out of the tree.
The line-art problem is missing evidence, which is item 1's department: census matching
moved the same anime measure 0.697 → 0.7025 — and did not clear its own bar either
(K-393). Item 3 (feature-aware blur reconstruction) is untouched by this and shipped.
Arrangement decided along the way: densification composed with §1 step 4's bilateral blur
rather than replacing it (measured better on both animation clips), so that blur stays
unconditional.

## K-392 — The blur scatters, and low confidence borrows instead of freezing

**DECIDED** (2026-08-19). K-390's item 3, shipped. docs/impl/optical-flow.md §4.7 carries
the shape and the measurements. Fast motion blur becomes a two-pass Guertin-class
reconstruction: a 16 px tile reduction of the flow field, then a gather that alternates
between the neighbourhood's dominant direction and the pixel's own, McGuire-weighted
(cone + cone + cylinder) so a fast object smears over the still background it passes.
Tap counts are adaptive (§4's `S = ceil(‖v‖/2)`), which demotes the schema's `Samples`
from a count to a cap. `MbQuality` (Normal/High) buys half the tap spacing and curved
trails; it is the **only** method choice a user sees, per K-390 — one method adapts
internally and there is no picker. The `View` enum gains Dominant motion, one new
user-facing string (`fxDominantMotion`), K-303 treatment complete; Quality/Normal/High
were already in the table.

**Confidence changes job, and this is the reversal.** v1 multiplied the streak by
confidence, so an uncertain pixel collapsed to no blur and read as a frozen speck amid
motion. It now *blends*: an uncertain pixel borrows its neighbourhood's motion at 0.6
strength. Zero blur survives only where the tile itself is still, which is the owner's
stated rule. `cpu_motion_blur_still_and_zero_shutter_are_passthrough` was rewritten
accordingly — "zero confidence does not blur" was v1's contract and is now false by
design.

**Borrowing and scattering are not the same summary, and treating them as one was a real
defect** — caught on footage, not in review. An extremum is right for "what is the fastest
thing near me" and wrong for "what is my neighbourhood doing"; used for both, neighbouring
tiles won unrelated wild vectors and cartoon.mp4 f200 (fast zoom, 70% of pixels below half
confidence) came out in rectangular patches of differently-angled blur. The borrow now
samples the tile field bilinearly between tile centres: continuous, so no tile edge shows,
and self-cancelling under disagreement, so the blur backs off where there is no consensus
to borrow.

**Two things did not land, deliberately.** The **±1 central difference** cannot be built at
the current seam: `CompJob::flow_neighbour` is one `Option<i32>`, `CompLayerPixels::flow_field`
one `Option<(Vec<f32>, Vec<f32>, Vec<f32>)>`, and `AuxSlot::FlowField` one texture — the −1
source frame is not even decoded for this effect (`temporal = &[0, 1]`). Adding it means a
second flow measurement, a second decoded neighbour and a second field threaded through
decode → build → draw → realise → fxops → the bind group; it is a render-crate change, not a
kernel one, and it is not free (2× flow on a scrub, ~1× amortised on linear playback, since
the pair measured at frame N is the pair frame N+1 wants). Left to its own change. **Jitter**
was dropped: adaptive taps already hold the ≤ 2 px spacing banding needs, and a hash required
to agree bit-for-bit across Rust and WGSL is a parity hazard bought for nothing.

**Known limitation, recorded rather than tuned away.** Without a depth buffer the
foreground/background weighting is symmetric, so a *small* static object entirely surrounded
by fast motion receives its neighbours' smear (cartoon f200's burnt-in logo, 0.37 → 8.95).
Large static regions are unaffected — the gameplay clip's desktop moves 0.014 of 255 against
3.99 inside the moving viewport. The fix, if ever wanted, is a depth input, not a constant.

## K-393 - The matching cost is chosen per patch, the bar is still not met, and the cost change is reverted

**DECIDED** (2026-08-19). §5.5.1's named fix, built and measured:
docs/impl/optical-flow.md §5.5.2 carries the tables. The inverse search no longer
picks census or SSD per build; each 8x8 patch picks its own from the Hessian trace
`h11 + h22`, which the step already sums and which *is* SSD's discrimination (the
second-order term of the SSD cost about its own minimum). Per pixel and rooted, it
is compared against one named constant, `CENSUS_GRAD_RMS`; below it the patch is
census-scored, at or above it SSD-scored, and the chosen cost carries that patch's
ballot, its keep-or-revert semantics, its residual and its validity test. No
setting reaches the constant; both backends compute it identically; the sweep's two
ends reproduce §5.5.1's before and after columns, which is the implementation's own
proof that an SSD-mode patch is bit-for-bit the old engine.

**It does not clear K-390's acceptance bar.** One content-blind sweep - octaves
0.01 to 0.16 closed to half-octaves, all five clips at once - and the best row,
tau = 0.0566, meets three of four conditions: anime +0.0012, cartoon +0.0045,
gameplay -0.0043, cinematic **-0.0052** against a -0.005 allowance. No row meets
all four, and the frontier's shape says none can: game capture falls monotonically
with tau while the cinematic is U-shaped with its minimum mid-grid, so the two
clips' admissible windows (tau <= 0.063 and tau <= 0.019 or >= 0.070) do not
intersect, and the low branch is closed anyway because anime is still down at
tau <= 0.04.

**Why it was worth putting up anyway.** Against K-390 as measured the miss moves
from 0.0023 on game capture - the footage the project exists for (K-002), which
the bar was written strict to protect - to 0.0002 on the cinematic, a tenth the
size and inside the harness's own scatter. Both animation clips still rise, game
capture recovers 0.0030 of the 0.0073 census cost it, and the synthetic clip goes
from -0.0059 to +0.0026. It is the same trade three times smaller and aimed at a
less important clip.

**Ruling: reverted to global SSD, and the flow ships unchanged this round.** A
smaller miss is not a met bar, and 0.0002 is exactly the size of miss that a bar
exists to refuse without argument - accept it once and the next campaign's
0.0002s compound into the 0.036-for-0.012 trade §5.5 already recorded and removed.
Census, the per-patch selector, `CENSUS_GRAD_RMS`, `HUBER_DELTA` and the census
constants are all out of the tree in both backends; `inverse_search` is
byte-for-byte the function it was before K-390. Nothing about the reasoning is
lost - impl §5.5.1 and §5.5.2 keep both tables, both frontiers and the reading,
which is the whole return on the stage.

**The follow-up the numbers name, not taken here.** A hard switch costs most where
patches sit near the threshold, because the field becomes a patchwork of two
estimators with different biases voting together in densification - which is
exactly where the grainy cinematic sits and exactly where its minimum is. Blending
the two costs across a band, or hysteresis so a patch's mode agrees with its
neighbours', is the indicated move; both need the two costs on a common scale,
which is a design question and a second measurement, so neither is smuggled in.

**Test coverage moved with it, and left with it.** Perlin's finest octave is a 10 px
period, so every patch of the two analytic parity scenes was census-scored at this
threshold and the SSD branch would have gone unproven; `gpu_matches_the_cpu_oracle`
gained a translated 16 px checkerboard, 92% SSD by patch. That scene went out with
the revert and any second attempt at a two-cost inverse search needs it back. A flat
cel with a hard outline is the tempting scene and the wrong one - its interior
constrains nothing, so it fails CPU/GPU parity at every threshold including
tau = infinity, where the code is exactly K-390's.

## K-394 — Round commits to the bubble: stadium controls, bigger cards, filled-pill actives

**DECIDED** (2026-08-19, owner-directed from a reference the owner likes: OUTLOUD's
Lyrica editor). The Round shape (K-092) floats its cards but stops at "slightly
rounded" — 8px controls on 14px cards read as Sharp with the corners sanded, and the
owner judges it under-differentiated. Round v2 takes cues, not a copy:

- **Controls become stadiums.** Buttons, chips, tabs, dropdowns and the transport
  cluster under Round are full capsules (radius = half the control's height), and the
  transport's buttons sit together inside one pill container. Sharp is untouched.
- **Cards grow.** `card_radius` 14 → 18, `float_radius` 12 → 16; the gap/inset system
  stands as K-092 built it.
- **The active state is a filled pill.** An active tab or mode chip fills with the
  accent and flips its label to `surface0` — the far end of the ramp from the text, so
  it is the dark label on a dark scheme and the light one on a light scheme (Round on
  Light exists) — instead of tinting its text; the state contrast is most of what makes
  the reference read as designed.
- **Slider dot thumbs were already ours.** `HouseSlider` has always drawn a round thumb
  on a thin track, under both shapes. The cue is recorded because the reference shares
  it, but nothing changes for it: making the dot Round-only would take it away from
  Sharp, which this decision does not touch.
- **Timeline bars round their ends** under Round: layer bars and Sequence clips draw
  as capsules (stadium ends at the bar's own height), Sharp keeps its rectangles.
- **Panel headers gain a small accent dot** under Round — the reference's quiet
  live-mark, adopted because it costs nothing and reads as identity.

Two cues were considered and rejected, recorded so they are not re-litigated: the
reference's uppercase panel titles (sentence case is the voice, K-005) and its light
window shell around dark cards (the dark-first surround rules and the neutral Viewer
pasteboard are binding, §2 and K-203; inverting the shell is a different theme, not a
shape). Everything lands as `ShapeTokens`/theme reads — a hex outside the theme module
is still a defect — and the strings change not at all.

## K-395 — Every effect can be driven by a matte, through one uniform row

**DECIDED** (2026-08-19, an original project goal of the owner's). Every built-in effect
gains a **Matte** input: a layer whose luma drives the effect per pixel — distinct from
masking an effect's *result*, because it feeds the effect itself, and what "drives"
means may differ per effect. The full shape lives in docs/08-EFFECTS.md §2.6 (new).

- **The row is uniform everywhere**: one row holding the layer picker and an **Invert**
  checkbox beside it, labelled "Matte" / "Invert". Effects that already had the idea
  under other names — Depth of field's depth layer, the Lens flare's matte source —
  adopt the same row treatment and labels; their stored parameter ids do not change
  (a save is a save, K-065), only their presentation and prose.
- **The default semantic is strength**: unless an effect declares better, the matte
  scales the effect's per-pixel mix — matte 1 applies fully, 0 leaves the source, in
  one generic post-lerp implemented once beside the registry dispatch. That makes the
  row *meaningful on all 35 effects from day one*.
- **Effects may override with a deeper meaning** where the matte belongs inside the
  maths: displacement-class effects scale their vectors before sampling; Depth of
  field keeps its focus-depth meaning; the flare keeps source detection. An override
  documents its meaning in the schema prose (and so in the manual, which generates
  parameter tables from the schema).
- **Mechanically**: the derive injects the pair (an `#[effect]`-level default, so no
  declaration repeats it and none can forget it); the K-387 aux seam carries the matte
  as a per-op optional layer input, rendered alone at the effect's raster exactly as
  DoF's input is; absent matte = today's behaviour byte-for-byte (K-258 backfill:
  defaults are unset + false).

**Postscript (2026-08-20): shipped, in full.** Every built-in carries the row. The derive
injects the `matte` / `matte_invert` pair at `#[effect]` level, so no declaration repeats
it and none can forget it — `every_effect_carries_a_matte_row` walks the catalogue and
fails on the first effect that declares none. The default semantic is one generic
post-lerp written once beside the registry dispatch (`cpu::matte_mix` and its WGSL twin
`fx_matte_mix.wgsl`), op-for-op as §1.6 requires.

**Four effects claim the matte inside their own maths**, and only four: Gaussian blur
scales its radius per pixel (grey blurs narrowly, which a dissolve cannot produce), Glow
gates which pixels may seed the halo before the bright pass (so light still spills outward
across dark matte), Depth of field keeps its depth meaning, and the Lens flare keeps source
detection. Each documents that meaning in its schema, which is what the manual's parameter
tables print — the test refuses an override that does not.

**The consolidation is the part worth recording.** Depth of field's depth pass and the
flare's matte source were two private lists; both now ride the one matte carriage, their
stored parameter ids unchanged (`depth`, `matte` — a save is a save, K-065), so what
changed for those two effects is presentation and prose, not their files. The K-387 aux
carriage is left holding exactly one thing, Light wrap's Background. The list is 1:1 with
the resolved ops that declare a matte, in order, which is the invariant that keeps a matte
from driving the wrong effect.

**K-258 was proven three ways, not asserted.** On the resolve side an instance stripped of
both parameters — which is precisely what every saved project is — resolves to the same
bag as a fresh one. On the picture side a real three-effect stack (a gather, a pointwise
op and a multi-pass) renders the same *bits* on the GPU with the pair stripped and with the
row unset, which is what rules out the tempting near-identity of running the dissolve
unconditionally at k = 1: that costs a full-frame pass per effect and requantises through
another fp16 store. And end to end, a document whose Exposure is driven by a half-frame
matte lifts the lit half in full and leaves the dark half byte-identical to the untouched
source. Absent matte is not a cheap lerp; it is no pass at all.

**No new strings.** The row reuses the `fxMatte` and `fxInvert` entries the flare and Depth
of field already had, which is what "the row is uniform everywhere" buys — thirty-five
effects gained a control and Crowdin is owed nothing for it. The campaign left the flow
engine untouched (`lumit-flow` has no diff), so K-390's recorded figures stand unmoved.

## K-396 — Curves stores five knots a channel, not a point list

**DECIDED** (2026-08-20, with the colour batch of the AE-parity wave, docs/08 §3.30).
After Effects stores `ADBE CurvesCustom` as **arbitrary data**: a per-channel list of
control points, as many as the user dragged, in a private serialisation. Arbitrary data
is not interpolable, so AE itself only ever *holds* a curve between keyframes — a curve
does not animate there, it steps.

Lumit's parameter kinds (docs/08 §1.1) are float, int, bool, enum, angle, colour, point,
seed, file and layer. None of them is "a list of points that grows", and adding one would
need a value type, a keyframe interpolation, a bridge shape and a panel widget before a
single pixel changed — and would still only step.

So **Curves v1 is twenty ordinary Floats**: five knots on each of Master, Red, Green and
Blue, at the fixed inputs 0, 0.25, 0.5, 0.75 and 1, each holding that channel's *output*
there. Every one keyframes and takes an expression, which is more than AE's own blob can
say. The shape between knots is a **monotone cubic** (Fritsch–Carlson), fitted host-side
in `Curves::packed` so the CPU reference and the WGSL kernel evaluate the same tangents
and neither fits a spline per pixel; the limiter is what stops a lifted highlight knot
ringing a dark halo into a roll-off, which an unlimited Catmull–Rom would.

Three consequences, all accepted:

- **A knot cannot slide sideways.** The curve is shaped by moving outputs, not by placing
  points. Five knots at quarter spacing is enough for the S, the crush, the lift and the
  per-channel split, which is what the effect is for.
- **The import samples rather than carries** (docs/11 §5). A fixed-kind parameter could
  not have carried an arbitrary point list under any design; the conversion evaluates
  AE's spline at the five inputs and is reported as *mapped*, not lossless.
- **The panel drawing twenty rows is not the stored form.** A drawn curve editor lands
  through `customEffectRows` (the per-effect hook `effect_controls_panel_frb.dart` asks
  first) and changes what the panel *draws*, not what a project stores — so a curve
  authored today survives it.

Reversing this later means adding a genuine curve parameter kind, and that is a decision
about the data model and the bridge, not about this effect.

## K-397 — Brightness & Contrast is a sibling effect, not a mode of Contrast

**DECIDED** (2026-08-20), resolving the open question in
docs/impl/ae-effect-parity.md. AE's `ADBE Brightness & Contrast 2` maps to **Brightness**,
a new effect in the Colour category carrying both of AE's sliders under AE's names and
AE's neutral point of zero (docs/08 §3.32). The existing one-knob **Contrast** (§3.18) is
untouched.

The alternative — folding both into Contrast behind a mode switch — was rejected on three
counts, in increasing weight:

1. **The import needs one effect.** AE's is a single effect with two properties that
   animate together; splitting it across two Lumit effects would split one keyframed pair
   across two stack entries and make the import report explain a shape nobody authored.
2. **The two knobs are not the same knob.** Lumit's Contrast is a per cent where 100 is
   neutral; AE's is signed, where 0 is. One control cannot be both without one of the two
   spellings changing meaning, and a save is a save (K-065).
3. **Menu hygiene loses to honesty.** A mode switch that silently re-scales an existing
   slider reads fine in a menu and wrong in a project file. Two small effects that each do
   one thing is what K-090's shape rule asks for anyway.

Brightness is an affine grade about the same mid-grey pivot Contrast uses —
`(u + Brightness÷100 − 0.5)·(1 + Contrast÷100) + 0.5` — so it declares
`alpha mode: unpremultiplied` for the same reason Contrast does, and highlights are never
clipped. It is deliberately *not* Exposure: an addition lifts the shadows as much as the
highlights, where a multiply leaves black where it was, and that difference is the reason
both exist.

## K-398 — Generate is the seventh effect category

**DECIDED** (2026-08-20), extending K-090's category list. The Add-effect menu gains
**Generate**, between Distortion and Stylise, for the effects that *make* pixels rather
than change them: **Fill** (§3.34), **Gradient** (§3.35), **Noise** (§3.36) and **Fractal
noise** (§3.37). The six categories K-090 named are otherwise untouched, and no existing
effect moves.

The alternative was to file the four under Stylise, which is where a generator ends up in
a catalogue that has nowhere else to put it. It was rejected because a category is a
promise about *what a thing does to your picture*, and these four do not answer that
question — three of them ignore the incoming picture entirely. A menu whose Stylise
flyout contains both Glow and Fractal noise has stopped grouping and started listing, and
the cost of the honest answer is one enum variant, one label and one translated string.

Two consequences worth recording:

- **The category set is not closed, but it is not free either.** A seventh category is a
  seventh flyout in every menu that groups by category, a seventh section in the manual,
  and a seventh word to translate. The bar for an eighth is the one this cleared: a
  family of effects that already exists and that no current category describes without
  lying.
- **"Generate" is the word, not "Generators".** Sentence case, singular, matching the
  voice of the other six (docs/01 §9, docs/15).

## K-399 — The distort batch: a fifth matte override, and how a resampling kernel is judged

**DECIDED** (2026-08-20), with the distort batch (docs/08 §3.38–§3.42: Turbulent displace,
Tile, Offset, Mirror, Lens distort). Two things in it are decision-sized; the rest is spec
and lives in docs/08.

**Turbulent displace is the fifth effect to claim the matte inside its own maths** (K-395).
Its matte scales the *displacement vector* before the sample, so a grey matte warps the
picture less rather than showing a fully-warped copy blended over an unwarped one — which
is two of every edge, and is what the generic dissolve gives. K-395's DECIDED text names
this case in advance ("displacement-class effects scale their vectors before sampling"), so
the rule is unchanged; what this supersedes is only the **count** in that entry's
2026-08-20 postscript, which said four. It is five, and the enumeration in
`every_effect_carries_a_matte_row` is where a sixth would have to be argued for. The claim
is held by picture as well as by tolerance: under a flat quarter matte a lit pixel must
travel about a quarter as far as it does at full matte, which a dissolve cannot do at all
(it leaves the pixel where it was, only fainter).

**A kernel whose real output is a sample position is judged on absolute difference over a
smooth corpus, not in fp16 ULPs over the hard-edged one** (docs/08 §1.6). Every effect in
this batch decides *where to read from* and then takes one bilinear tap. Where the two
paths contract a multiply-add differently — a dot product fused on the GPU and not on the
CPU, which is legal on both — the position moves in its last bits, and a hard edge
multiplies that into a whole pixel of colour while a smooth picture does not. Measured: the
same Mirror kernel reads 31 fp16 ULP on the alpha-edge corpus and under 2 × 10⁻³ absolute
on the smooth one, and the arithmetic is identical in both. The ULP metric was measuring
the size of the edge. Offset alone kept it, because its arithmetic has no expression to
fuse. This is a **testing** decision and not a licence: the kernels are still written
op-for-op against the CPU reference, and the effects that grade colour rather than move it
are still held to ULPs.

A corollary is recorded here because it looks like a violation of §1.6 and is not: **Lens
distort runs one transcendental per pixel** (`tan` forward, `atan` reversed). The
catalogue's rotations arrive as host-computed cosine/sine pairs precisely because WGSL's
trigonometry is not correctly rounded — but here the angle *is* a function of the pixel and
cannot be lifted out of the loop. The one call that can be (`tan(fov ÷ 2)`) is. The rest is
admitted in docs/08 §3.42 rather than hidden, and is exactly what the paragraph above
exists to measure.

## K-400 — Transition is an eighth category, and Set matte's matte is its output

**DECIDED** (2026-08-20), with the utility and transition batch (docs/08 §3.43–§3.47: Drop
shadow, Set matte, Channel blur, Linear wipe, Radial wipe). Two things in it are
decision-sized; the rest is spec and lives in docs/08.

**An eighth category, Transition**, between Temporal and Utility, for the effects that
remove the picture progressively so a cut can be made out of one: **Linear wipe** (§3.46)
and **Radial wipe** (§3.47) today, and AE's Iris wipe, Venetian blinds and Card wipe when
Tier B lands. K-398's postscript set the bar for an eighth — "a family of effects that
already exists and that no current category describes without lying" — and this clears it
the same way Generate did. A wipe is not a stylisation (it adds nothing) and not a utility
(a utility is a tool, and this is a *shot transition*, the thing an editor reaches for by
that name); filing it under either would be listing rather than grouping. The cost is
again one enum variant, one label and one translated string. **"Transition" is the word,
not "Transitions"** — sentence case, singular, matching the other seven (docs/01 §9).

**Set matte is the sixth effect to claim the matte inside its own maths** (K-395), and
that resolves the open question docs/impl/ae-effect-parity.md carried: *whether Set matte
belongs in Utility or is a documented pattern of K-395's row.* It is **both, and those
were never two answers.** The effect lives in Utility and its source is the universal
Matte row, under the `Own` role — because what its matte supplies is the alpha itself, not
an amount of some other effect. It is the first override for which the matte is the
*output* rather than a modifier of one, and the picture proves it: under a disc matte, Set
matte gives a disc-shaped picture, where the generic strength dissolve gives the whole
frame with a faint ring at the matte's own soft edge. That is not a difference of degree.

Two smaller things are recorded here because they look like violations and are not.

**A kernel whose real output is a *threshold on a position* is judged like one whose
output is a position** (K-399, docs/08 §1.6). Both wipes reduce to a signed distance — a
dot product for the linear one, an `atan2` and an angular wrap for the radial — divided by
a feather that may be narrower than a pixel. Where the two paths contract that dot product
differently, a difference of 10⁻⁶ in the distance becomes a visible difference at the edge,
exactly as it does for a sample position; so the wipes take K-399's smooth corpus and its
absolute-difference tolerance. Measured worst: 9.5 × 10⁻⁴ across both effects' whole
sweeps, against a 2 × 10⁻³ bound. Set matte, which computes no position at all, stays on
fp16 ULPs and measures 1.

**The angular wrap uses `floor(x + ½)` and never `round`.** Rust rounds halves away from
zero and WGSL rounds them to even; the two disagree on exactly the pixels that sit on the
wedge's boundary, which is the set of pixels a wipe is *about*. Written once in
`cpu::radial_wipe_keep` and mirrored op-for-op, it is a one-line trap that would otherwise
have shown up as a single flickering pixel and been blamed on the driver.

## K-401 — Parity means importable, never confined: AE's parts are the floor, not the ceiling

**DECIDED** (2026-08-20, owner's standing rule). The AE parity programme exists so the
import can convert real projects, and that requires every effect to carry AE's
parameters with AE-mappable semantics — the floor. It does not require looking
identical to After Effects or stopping at AE's parameter set — there is no ceiling.
Where Lumit can render an effect better, or a parameter beyond AE's would genuinely
improve it, take it: the owner's example is grain, which can look nicer than AE's
while still carrying every part AE's has so an imported instance maps cleanly.

The one discipline this adds: an effect's AE-mappable subset stays identifiable —
docs/11's table maps AE parameters onto ours, and an import must be able to set ONLY
that subset and get a faithful conversion, with Lumit's extras at their neutral
defaults. An extra parameter whose default is not neutral would silently change every
imported project, which is the failure this rule's discipline exists to prevent.
Applies retroactively: any shipped effect that was confined to AE's shape out of
caution may grow past it whenever an improvement is real.

## K-402 — Displacement map's matte is its map, and a guarded texture fetch is not guarded

**DECIDED** (2026-08-20), with Wave 2's Distort I batch (docs/08 §3.48–§3.52: Corner pin,
Displacement map, Polar coordinates, Twirl, Spherize). Four things in it are decision-sized;
the rest is spec and lives in docs/08.

**Displacement map is the seventh effect to claim the matte inside its own maths** (K-395),
and the second — after Set matte (K-400) — for which the matte is the effect's *subject*
rather than a modifier of one: the layer on the Matte row **is** the displacement field. AE
has a picker of its own for it; adding a second layer reference beside a row that already
names a layer and renders it at this raster would have been a seam bought for nothing. The
choice pays a second dividend that was not the reason for it: AE's *Displacement Map
Behaviour* (Centre Map / Stretch Map to Fit / Tile Map) exists only because AE hands the
effect the map layer at the map's own size. The matte carriage renders it at **this**
effect's raster, so "stretch to fit" is the only behaviour there has ever been, and the
other two are reported by the import rather than approximated.

**A guarded out-of-range texture fetch is not guarded.** Every kernel in the distort family
carries a copy of the same helper — "fetch the texel at (x, y), or transparent if that is
outside the frame" — written as a bounds check with an early return before `textureLoad`.
A texture fetch has no side effects, so the compiler may hoist it above the branch, and on
this machine's Windows backend the hoisted out-of-range fetch returns a **live alpha lane**
with a zero colour. The symptom is a pixel that should be empty arriving opaque with the
right colour, and it appears only where *all four* bilinear taps are outside, which is why
no shipped kernel's oracle had caught it: Polar coordinates is the first whose samples leave
the frame in bulk. The rule from here: **clamp the coordinate into the frame, fetch that,
and choose between it and transparent afterwards** (`select`) — no fetch is ever out of
range, so nothing is undefined, and it costs one instruction. The five new kernels do this.
**The older copies of the pattern are unchanged and are a known defect** (Mirror, Tile, Lens
distort, Drop shadow, Transform, Shake and the blur family): their oracles pass today
because their corpora do not drive every tap outside at once, which is a statement about the
tests rather than about the kernels.

**Spherize splits AE's one signed Radius into a length and a signed Bulge.** AE's control is
a radius in raster pixels that passes through zero to mean "inside out". One control cannot
be both a resolution-independent length (docs/08 §2.3, so it survives a resize) and a signed
quantity whose zero is the neutral — the same shape of argument K-397 settled for
Brightness, and it lands the same way: two controls, each meaning one thing. Radius is
`% diag`, Bulge is a per cent from −100 to 100, and the import converts AE's magnitude to
one and its sign to the other, losing nothing.

**Two more exact-inverse pairs, and both are tested as such.** Polar coordinates' two
conversions and Spherize's bulge and pinch each compose to the identity (to resampling
error), because each is written as a genuine inverse function — `sin` against `asin` for the
sphere — rather than as a coefficient with a minus sign in front of it. Lens distort's
Reverse (§3.42) was the first; the property is worth stating as a family rule, because a
negated coefficient *looks* like an undo in a still frame and is not one, and the difference
only shows up when somebody stacks the pair.

A corollary, recorded because it looks like an unnecessary branch and is not: **Spherize
short-circuits at Bulge 0**, as Lens distort short-circuits at Field of view 0. Without it
the sample scale falls out of the blend as `ρ ÷ ρ`, which this backend compiles as a
reciprocal-multiply and answers a hair under 1 — a whole picture of resampling softness for
an effect the user has turned off.

## K-403 — A speed control cannot be deterministic, and a solver must be allowed to fail and made to prove itself

**DECIDED** (2026-08-20), with Wave 2's Distort II batch (docs/08 §3.53–§3.57: Ripple, Wave
warp, Bezier warp, Warp, Roughen edges). Four things in it are decision-sized; the rest is
spec and lives in docs/08.

**AE's Wave Speed has no Lumit equivalent, and its absence is the faithful conversion.**
Ripple and Wave Warp both animate themselves in After Effects from a control measured in
cycles per second, which is read off the composition clock. Lumit cannot have one: docs/08
§2.4 requires that the same frame render the same picture every time, and a clock-driven
effect makes the preview and the export disagree, makes two machines exporting the same
project disagree, and makes a cached frame a lie about its own contents. Both effects
instead carry an **angle the timeline animates** — Evolution on §3.53, Phase on §3.54 — where
one full turn is one whole wave, and the import writes AE's speed as two linear keyframes of
`360 × speed` degrees a second. This is the first time a *missing* control has been the
correct import, and it generalises: any AE parameter whose unit contains "per second" is a
keyframe here, not a slider.

**Bezier warp is the first kernel in the catalogue that solves rather than computes**, and a
solver carries two obligations a closed-form map does not. It must be **allowed to give up**:
a Coons patch dragged until it folds over itself has no single answer for the pixels in the
fold, so the Newton iteration stops on a singular Jacobian rather than dividing by zero —
K-402's degenerate-quad rule (docs/08 §3.48), which is itself the house rule of
14-ENGINEERING-RULES §4. And its answer must be **checked**. Outside the patch there is no
answer at all, and an unchecked iteration wanders until it happens to land in `0..1`, which
draws a scatter of stray opaque pixels across the empty part of the frame — a defect that
reads exactly like a driver bug and is not one. The fix is one more patch evaluation asking
"does this answer solve the problem?", and discarding it when the residual exceeds a pixel.
**The rule from here: an iterative kernel verifies its result before it uses it.** The cost
is one evaluation; the alternative is confetti.

Two corollaries of the same effect, both in §3.55. AE's **Quality** on Bezier Warp buys
smaller triangles because AE draws the patch as a mesh; Lumit inverts the patch per pixel, so
the same slider buys Newton steps instead — the number converts unchanged and means "more
accurate" on both, which is the most that can honestly be claimed. And **a sample landing
within a thousandth of a pixel of its own centre is snapped to it**, so an unbent region of a
bent frame is bit-exact rather than resampled. §3.42's Field of view 0 and §3.52's Bulge 0
short-circuit the same complaint at one setting each (K-402's last note); this answers it
everywhere, for one comparison, four orders of magnitude below anything a resampler could
show.

**A blurred alpha is a distance field, and reusing the shipped blur for one is the second
time a kernel has paid for another.** Roughen edges has to know, per pixel, how far it is
from the shape's outline in order to chew a Border-deep bite out of it. A real distance
transform is a separate algorithm with its own passes and its own tie-breaking; blurring the
picture by Border gives the same information for nothing, because the half-way contour of a
blurred alpha sits exactly where the edge was and the ramp either side of it is Border wide.
The effect therefore blurs (the §3.8 gaussian, unchanged — the same reuse §3.43's softening
makes, K-400) and re-cuts the result at a threshold the §3.37 fractal field wobbles. The
companion decision is that **the wobble is weighted by the band itself**: deep inside a solid
layer the blurred alpha is 1, and one low octave would otherwise punch a hole in the middle
of the shape. Weighted, the chewing is confined to the band, which is both correct and what
Border's name promises.

**A dropdown that multiplies two independent choices together is one control too few.** AE's
Roughen Edges ships seven Edge Types, which are three shapes (Roughen, Cut, Spiky) times a
colour flag, plus Photocopy. Lumit ships the three shapes as **Edge type** and the flag as
**Colour edge**, with an Edge colour beside it (§3.57). The conversion is lossless in both
directions, and the split buys what the multiplied form cannot give: either half can be
animated, read or expressed on its own. AE's Photocopy converts to Cut with Colour edge on
and is reported as an approximation. This is K-397's shape of argument a second time — one
control, one meaning — arriving from the opposite direction.

A note that is not a decision but has cost time twice now and is written down for the third
occasion: **every one of these kernels is a gather**, so a map that reads *further out*
makes the picture *smaller*. Warp's thirteen styles are named for what the picture does, so
the five swelling ones (Arch, Bulge, Fish, Fisheye, Inflate) subtract their coefficient where
the bending ones add theirs. Written the natural-looking way round, a style called Bulge
pinches at a positive Bend, which is the one thing a named preset may not do.

## K-404 — A tone control belongs where the eye is, and a quantiser needs an exactly-rounded curve

**DECIDED, 2026-08-20.** Wave 2's Stylise I batch — Posterize (docs/08 §3.58), Threshold
(§3.59), Tritone (§3.60), Photo filter (§3.61), Black and white (§3.62) and Shadow highlight
(§3.63), all six in the **Colour** category, where After Effects also files them. The
catalogue stands at **69**. Five decisions came out of the batch, and the first two are one
decision seen from two sides.

**Every control that names a *place on the tone range* is placed perceptually, not in
light.** Posterize's rungs, Threshold's Level, Tritone's three stops and Shadow highlight's
midtone pivot are all statements about where a person sees the middle of the picture, and in
scene-linear light (docs/08 §2.1) the middle is 0.25 rather than 0.5 — the working space is a
count of photons and the eye is not. Spacing eight posterize rungs evenly *in light* puts six
of them above mid-grey, which bands the highlights to pieces and leaves the shadows a smooth
ramp: not what a posterize is for, and not the picture After Effects gives, AE quantising an
8-bit display value. So all four controls run through one shared curve
(`lumit_core::fx::cpu::perceptual`), and the import converts through it rather than passing
AE's number straight in. The general rule: **a grade's *operation* is done in light; a
grade's *control* is placed where the eye is**, and §3.18's linear pivot is not a
counter-example — it is the middle of an operation, where this is the middle of a judgement.

**That curve is a square root, and the reason is the oracle rather than the picture.** A
quantiser's output is a *step*, so the two render paths disagreeing by one bit about which
side of a rung a value falls on is a whole rung of colour, not a last-bit difference — K-399's
rule about a threshold, arriving on a colour effect where nothing moves at all. `sqrt` is a
single correctly-rounded instruction on both the CPU and the GPU; `pow(u, 1/2.2)` is a
polynomial each vendor writes differently, and a difference of one ULP in it is a visible band
edge that moves between the preview and the export. Between a gamma of 2.0 and sRGB's 2.2
there is no visible difference in where eight bands land, and between an exactly agreed answer
and an approximately agreed one there is a flickering test. **Choose the transfer function
your oracle can prove, not the one the textbook prefers.** The same reasoning fixes the
rounding: `floor(x + ½)` is written out in both paths, because WGSL's `round` breaks a tie to
even and Rust's breaks it away from zero.

**A blurred picture can be a question rather than an answer, and that is what "local" means.**
Shadow highlight is the first effect in the Colour family that reads a pixel's neighbours. It
blurs the picture at Radius — the shipped §3.8 gaussian for the **third** time, after §3.43's
softening and §3.57's distance field — and uses only the *luma* of the result, and only to
decide **whether this pixel is being treated as a shadow**. No colour is ever taken from the
blur, so nothing is softened and no detail is borrowed. That is the whole of local adaptation:
a white shirt button inside a dark jacket is lifted with the jacket instead of being singled
out, because the question asked about the button was about the jacket. Two consequences worth
recording. The lift is a **multiply, not a per-pixel gamma** — monotone, no clamp, no inverse,
no `pow` a pixel, and the mask is what makes the effect adaptive rather than the exponent. And
Lumit ships **one Radius where AE ships two**: a second full-frame gaussian is a great deal of
work for the softness of a mask, and the import averages AE's pair and reports it.

**An effect that decides its own settings from the frame is not an effect, and the omission is
the decision.** AE's Shadow/Highlight defaults to Auto Amounts, which reads the frame's
histogram, and smooths that reading across neighbouring frames with Temporal Smoothing and
Scene Detect. The first half is a whole-frame reduction, the second reads frames the effect is
not given, and together they produce a grade whose answer at a frame depends on the shot
around it — which cannot be scrubbed backwards, cannot be judged on a still, and is a
different programme from a tone control. Lumit ships neither; an imported instance arrives with
AE's default manual pair written in and the report says so. This is §3.53's missing speed
control in another costume (K-403): **the faithful conversion of a control that cannot be
deterministic is sometimes no control at all.**

**A weight set has to be provably harmless on a neutral.** Black and white's six sliders work
on an *exact* decomposition — every colour is a grey, plus one secondary, plus one primary,
and the three parts sum back to the colour they came from — rather than on a nearest-primary
weighting. Two things fall out that a weighted-sum scheme cannot have: on a grey pixel every
difference is zero, so the six sliders do nothing at all and a neutral-heavy shot does not
drift while one colour is tuned; and the six branches agree wherever two channels are equal,
because the term that distinguishes them is zero there, so a gradient has no seam. The same
shape of argument as §3.33's hat functions summing to one, arrived at from the other end.

One note that is not a decision. **Photo filter's twenty named filters are Lumit's own
chromaticities under Adobe's names** — Adobe's values are not published — so the import is a
look-for-look conversion reported as mapped, exactly as §3.56's thirteen Warp styles are. The
Wratten designations in the names (85, LBA, 81, 80, LBB, 82) are photographic terms and are
not translated, which is the second entry in the label table to say so after the Lens flare's
lens library (K-303).

## K-405 — A selection has to be data-oblivious, an honest cap is a hard one, and a coordinate is a threshold too

**DECIDED, 2026-08-20.** Wave 2's Stylise II batch — Median (docs/08 §3.64), Mosaic (§3.65),
Find edges (§3.66), Emboss (§3.67) and Texturize (§3.68) in **Stylise**, and Broadcast safe
(§3.69) in **Utility**. The catalogue stands at **75**. Five decisions came out of it.

**A kernel may not branch on a pixel's value, so a median is a compare-exchange network.**
Median is the first effect in the catalogue whose answer is *chosen* rather than computed:
the middle value of the window, which every textbook finds with a quickselect. A quickselect
cannot be one of Lumit's kernels. It diverges every lane in a warp, and — the part that
actually decides it — it executes a *different sequence of comparisons* on the CPU and on the
GPU, which §1.6 has no way to hold to agreement: two paths that compare in different orders
can pick different samples, and one sample apart is a visibly different pixel. What both
paths run instead is a network: sweep the window once, carry the `⌈N ÷ 2⌉` smallest values
seen so far in a sorted array, and insert each new sample by bubbling it down with `min`/`max`
pairs. Nothing branches, the two paths execute identical comparisons, and because `min` and
`max` are exact and a sorted set does not depend on insertion order, the answers are
bit-identical *even though the two paths sweep different windows* — the GPU always sweeps the
widest one and pads with a value above every pixel, because a padded insertion is provably a
no-op. Two things fall out worth keeping. `min`/`max` on a vector are componentwise, so the
three colour channels and the alpha are selected **simultaneously**, four medians for one
network. And the general rule: **where an effect has to choose rather than compute, write the
choice as a fixed sequence of exact operations, not as a search.**

**A cap you can type past is not a cap.** Median's sweep costs `(2r+1)⁴ ÷ 2` compare-exchanges
a pixel — 45 at radius 1, 325 at 2, 1 225 at 3, 17 000 at 6. AE's Radius goes to 50. Lumit's
stops at 3, and stops **hard**: the slider's `hard_max` is 3, not a soft limit that a typed
value could exceed (docs/08 §1.2 lets a slider be exceeded by typing, and this is the case
where that must not be true). The alternative — accept any number and clamp it silently —
was rejected because a control that answers a different question from the one it was asked is
worse than a control that refuses. This is the catalogue's only `heavy` single-pass kernel and
its cost class says so; the AE import writes 3 and **reports the instance as approximated**,
which makes it the first conversion in docs/11's table limited by a *budget* rather than by a
semantic.

**K-399's rule about a threshold applies to a coordinate, and the answer is integer
arithmetic.** Mosaic decides which block a pixel is in. Written the obvious way — `floor(x ÷
block_width)` — a pixel whose division comes out exact lands in different blocks on the two
paths, and a whole block of colour changes. Every boundary and every sample position in Mosaic
is therefore computed in **integers**: `(x · blocks) ÷ len` and back, with the stratified
sample at `lo + (2k·span + span) ÷ 2n`. Integer division has no tie to break. The companion
decision is that **the averaged mode samples the block rather than reading it**: a true mean of
a block of a 1080p frame at the default grid is 3 500 taps redone by all 3 500 pixels, so at
most an 8×8 stratified grid is read — which is the same flat colour on any block worth
mosaicking, and is an *exact* mean on any block under eight pixels across, where the grid
covers everything. Bounding a reduction is not an approximation when the bound is above the
point where the answer stops changing.

**A gradient belongs on the perceptual value too, which is K-404's rule reaching an effect
that is not a grade.** Find edges and Emboss both difference their neighbours, and both do it
in `√` light rather than in light. In scene-linear the step from 3.0 to 4.0 in a sunlit sky is
a larger number than the step from 0.01 to 0.05 in a shadow, though the eye sees the second
and not the first: a Sobel taken in light draws the specular highlights and nothing else, and
a relief taken in light is all highlight and no shadow. K-404 stated the rule for a control's
*placement* on the tone range; this is the same rule for a *difference* taken across it, and
it is what makes Find edges read as a pencil drawing and Emboss as AE's grey relief rather
than as maps of where the picture is brightest.

**A second layer is not always the matte, and the fitting question has one answer.** Texturize
takes a layer, as §3.49's Displacement map does, and does **not** take it on the Matte row. The
distinction is worth stating because it will come up again: a displacement map has nothing
else it could be, so the row that names a layer is the whole of that effect's input; a texture
is not, because an editor wants to press a canvas into a layer *and* limit the pressing to a
region, and one row cannot say both. So Texturize declares its own Texture row (Light wrap's
Background, §3.28, is the precedent) and keeps the generic §2.6 strength matte. **The test is
whether the effect has a second thing to say about *where*.** The related decision is
Placement. The layer carriage renders a referenced layer alone at this raster, so "stretch to
fit" is what it does and always did — §3.49 disposed of AE's Displacement Map Behaviour on
exactly that ground. Texturize keeps AE's three names by splitting the question: **Scale** (a
Lumit control) says how big one copy of the texture is, and **Placement** says only what
happens *outside* that copy — hold the edge, repeat, or leave it untextured. At Scale 100 all
three coincide, and that one case is AE's Stretch Texture to Fit exactly, which is why Scale
defaults to 100 and why the import converts the choice and reports only the size.

Two notes that are not decisions. **Broadcast safe is named for what it does**, docs/01 §9,
rather than for AE's Broadcast Colors; it ships all four of AE's treatments, including the two
diagnostic views, because seeing *which* pixels are illegal is half of why anyone reaches for
it. Its kernel is the only one in the family that writes its luma out longhand instead of
using `dot` — two of its four modes turn that number into a **threshold on the alpha**, so a
fused multiply-add taken by one path and not the other is a pixel keyed out on one and kept on
the other. And **`target` is a WGSL reserved keyword**: a module that uses one does not
compile, silently, into a texture of zeros — the pipeline is still created and the dispatch
still runs. The §1.6 oracle caught it at 15 584 fp16 ULP, which is worth knowing as the shape
that failure takes.

## K-406 — A gather can hold a camera, a shape can be one sector, and a control the projection hides is a control that must go

**DECIDED, 2026-08-20.** Wave 2's Transitions batch — Venetian blinds (docs/08 §3.70), Iris
wipe (§3.71) and Card wipe (§3.72), all three in **Transition**, which completes the category
K-400 opened. The catalogue stands at **78**. Four decisions came out of it.

**A projective flip belongs in a gather, because its inverse is one division.** Card wipe is
the first effect in the catalogue to put a camera in front of a pixel, and the obvious way to
build one is the wrong way here: transform the rectangle, rasterise it, composite the result.
Lumit's effects gather (docs/08 §1.1) — a pixel asks where it should read from — so the kernel
solves the projection **backwards** instead. A one-point projection of a rotating card,
`f = s·cos θ·D ÷ (D − s·sin θ)`, is a Möbius map in `s`; it inverts to
`s = f·D ÷ (D·cos θ + f·sin θ)`, one divide, exactly. That single fact is what makes a card
wipe a cheap one-pass kernel with no geometry pipeline behind it, and it generalises: **before
building a scatter, check whether the map inverts in closed form.** §3.55's Bezier warp is the
case where it does not and Newton has to run; this is the case where it does. The
foreshortening the solved `s` produces then divides the cross-axis coordinate, which is what
makes a card *turn* rather than merely squash.

**The camera is fixed and has no controls, and the omission is the conversion.** AE's Card
Wipe carries Camera Position, Corner Pins, Composite Camera, a Lighting group, a Material
group and two jitters — all of it there to place one *shared* 3D camera in front of the grid.
Lumit keeps cameras on the composition (docs/06) and has never had one on an effect, so every
card is projected in its own local frame from a fixed viewing distance of three card
half-widths. This is the same shape of ruling as K-403's missing Wave Speed and §3.63's absent
Auto Amounts: **the faithful conversion of a control Lumit cannot honour is to not have it and
to say so**, rather than to approximate it into something that will disagree with the original
in a way nobody can predict. AE's Back Layer and Card Scale go the same way; a card that turns
to nothing is AE's own picture with an empty back layer. What survives the cut is chosen by one
test — **is it still visible?** Flip Direction only means something because the perspective is
there; in a flat squash it would be a control that does nothing, which is a defect wearing a
name (K-405's Broadcast safe naming, from the other end).

**A rotationally symmetric shape is one sector, and the fold makes the feather a width.** Iris
wipe never rasterises its polygon. A regular polygon and a star are both a single wedge
repeated round a circle, so the pixel's angle folds into one wedge and mirrors about that
wedge's bisector, and the entire boundary — six sides or sixty-four — becomes **one straight
edge** whose two vertices the host solved once. The distance to it is a dot product. Two things
follow that are worth carrying to the next shape effect. The plain polygon and the star are the
*same expression*, differing only in where the host puts the second vertex, so the toggle costs
nothing per pixel and cannot drift between the two cases. And the number that comes out is a
**true perpendicular distance in pixels**, so Feather is a width — which is precisely the
problem §3.47's radial feather had to clamp its way around, avoided rather than solved.

**Both ends of an animated range must be tested for, not arrived at.** Card wipe's flip runs
`θ = t·½π`, and `cos(½π)` in `f32` is 6·10⁻⁸ rather than zero — enough that at Completion 100
every card would leave a hairline of quarter-strength pixels down its spine. The progress `t`
is clamped, so both paths test `t ≤ 0` (pass the pixel through) and `t ≥ 1` (clear it) instead
of trusting the trigonometry to land. This is the general form of §3.42's and §3.52's
short-circuits — which were about an effect the user had turned *off* — extended to the far
end of the range, and it is what lets the effect claim an exactly empty frame rather than
nearly one.

Three deliberate divergences from AE, all recorded in docs/08. **Venetian blinds' Width is a
length in px@comp** (§2.3, §3.38 decision 5's reasoning again) where AE's is raster pixels, and
its Completion defaults to 50 for §3.46's reason. **Iris wipe's two radii are per cents of the
comp diagonal** while its centre is px@comp — §3.51's split of a *size* (which must survive a
reframe) from a *place* (which the user clicks), stated once more because the mixture inside
one effect looks like an inconsistency and is not. And **Card wipe's Flip Order loses AE's
Gradient entry**, which reads its order from a gradient layer: §3.68's test asks whether the
effect has a second thing to say about *where*, a card wipe plainly does, and so the one layer
row it has stays the universal Matte. Randomness and Seed cover what is left of the intent, and
a gradient order can arrive later on a row of its own without moving anything.

One note that is not a decision: **`active` is a WGSL reserved keyword**, and like K-405's
`target` it fails at module creation rather than at runtime. The batch's own kernel names its
short-circuit flag `has_shape`.

## K-407 — Randomness that does not vary per pixel does not belong in the kernel, and a clock a control replaces is a clock that never lies

**DECIDED, 2026-08-20.** Wave 2's Draw and grain batch — Beam (docs/08 §3.73), Lightning
(§3.74), Radio waves (§3.75), Vegas (§3.76) and Add grain (§3.77), all five in **Generate**,
which is where §3.36 Noise already sits for the same reason: what they do to a frame is put
something *on* it rather than change the colour of what is there. The catalogue stands at
**83**. Two of AE's seven Tier B draw effects — **Scribble** and **Stroke** — are not built,
and the reason is a missing seam rather than a decision; it is recorded below and in
docs/impl/ae-effect-parity.md. Five decisions came out of the five that landed.

**If the randomness does not vary per pixel, it does not belong in the kernel.** A lightning
bolt is a recursive displacement of a straight line, and the obvious build re-derives it for
every pixel so that each can measure its distance to the result — about two hundred hashes a
pixel, two million times a frame, for a shape that is *the same shape for all of them*.
Lightning instead builds the bolt once in Rust, in `packed()`, into a list of at most 192
straight segments carried in the uniform; the kernel is then a plain minimum over capsules with
no hashing at all. Three things follow. It is a few hundred multiplications a frame instead of
a few hundred million. **It disposes of §1.6 for free** — both paths are handed the identical
numbers, so there is no second implementation of the generator that could disagree with the
first, and the oracle only has to hold the *drawing* to agreement. And the parameter struct
that crosses the boundary stops being a bag of scalars and becomes a small piece of geometry,
which is a shape the registry (docs/impl/effect-registry.md §2.4) always allowed and nothing
had yet needed. The rule to carry: **ask whether the random thing varies per pixel before
writing a hash into a kernel.** Card wipe's per-card shuffle (K-406) genuinely does — every
pixel is in a different card — and stayed. A bolt does not.

**A clock becomes a control, and the control is better than the clock.** Radio waves is an
emitter: AE's version knows what second it is and throws out a wave accordingly. §2.4 forbids
that, so Lumit's **Time** is an ordinary parameter in seconds that the timeline animates, with
Frequency, Expansion, Lifespan and Spin all keeping their per-second units and meaning what
they say against *that* Time. Keyframed linearly it is AE's effect exactly; held, it freezes
the whole set of waves mid-flight; scrubbed backwards, every wave returns to where it was. This
is the third time a missing rate has been the faithful conversion (K-403's Wave Speed on both
Ripple and Wave warp, §3.63's Auto Amounts) and the first where **the replacement is richer
than the original**: a rate cannot be varied and a keyframed second can, so Lumit's version can
do slow motion and a freeze that AE's cannot. Beam's Time and Length are the same idea one step
simpler, and Add grain's Animate switch is the degenerate case — a rate with only two useful
values, which is a switch.

**A stroke needs a contour's direction, and a level set gives one where an edge detector does
not.** Vegas has to answer two questions per pixel: how far across the contour am I, and how
far along it. The obvious build thresholds the gradient's magnitude, which answers neither — the
band's thickness is then decided by how steep the picture happens to be, so Width would do
nothing on a soft edge and everything on a hard one. Dividing the value's distance from the
threshold *by* the gradient turns it into a distance in **pixels**, which is Width's unit, and
costs one division; the same expression switches the effect off where the picture is flat,
because a vanishing gradient sends the distance to infinity rather than to zero. Two companions
to that. The gradient is a separable **5×5** Sobel rather than a 3×3, and the two extra taps
each way are not a refinement: on compressed footage a 3×3 gradient points a different way in
almost every pixel, and the dashes come out as speckle instead of as a line. And the dash's
phase is measured **from the middle of the frame**, because an error of ε in the contour's
direction moves the phase by `|p|·ε` — halving the arm halves the wobble for nothing.

**Softness can be a crossfade between two readings of one field, which is cheaper than a
blur and correct at both ends.** Add grain's hard reading takes one value per cell — a flat
square, which is what a grain particle is — and its soft reading interpolates the same lattice
smoothly. Blending them costs one extra hash and gives a control whose 0 is a sharp scan-grain
and whose 100 is a soft organic mottle, both of which are looks somebody wants. A real blur
would have been a second full-frame pass for a control nobody keyframes. The companion is that
**Monochrome is a lane, not an average**: the three channels read the noise core's `channel`
argument — the same decorrelation the fractal sum uses for its octaves — so colour grain is
three independent fields and mono grain is one field read three times, and neither is the other
filtered.

**A colour a control names must be a colour somebody can see.** Beam's Softness is the share of
the half-width the rim occupies, and the obvious build ramps the inside colour to the outside
one across exactly that band — which reaches the outside colour at the beam's own edge, where
the pixel is half-covered and about to disappear. The Outside colour control would then be
a control that names a colour nobody ever sees at full strength. The crossover takes the rim's
**inner half** instead, so the outer half is solid rim and the control means what it says. The
general form, worth carrying: **when a gradient's endpoint coincides with a coverage's
endpoint, the endpoint is not reachable** — put the gradient inside the coverage.

Five deliberate divergences from AE, all recorded in docs/08. **Beam's two thicknesses and Add
grain's Size are lengths in px@comp** (§2.3, §3.38 decision 5's reasoning again), as is Radio
waves' Expansion, which is such a length per second. **Beam's 3D Perspective is not carried** —
it foreshortens from a camera Lumit keeps on the composition, which is K-406's ruling on Card
Wipe's camera in a smaller costume. **Four of AE's eight Lightning Types are built**, the other
four mapping to the nearest and being reported; Alpha Obstacle is not carried at all, being a
search rather than a formula. **Radio waves ships one Stroke width where AE tapers from a start
to an end**, the Fade pair carrying what that taper was mostly for, and only the Polygon wave
type is built. And **Vegas' Segments count becomes a Segment length in px@comp**, because an
effect that never traces a path has no arc length to count segments around — on a straight
contour the two are the same picture, and on a tightly curved one Lumit's dashes drift in phase
where AE's stay evenly spaced. All five took the generic strength matte (K-395): none wants the
matte as its *subject*, and what a matte says on a draw effect is *where the drawing is*, which
is what a dissolve says.

**Scribble, Stroke and Vegas' Mask/Path half are blocked on a seam that does not exist, and it
is named here so the next attempt does not rediscover it.** All three read their layer's **mask
paths** — not the coverage a mask produces, the *geometry*: Scribble fills a path with strokes
at an angle, Stroke walks a path with a brush between a start and an end per cent, and AE's
Vegas can march segments round a path it has traced. Nothing in Lumit's effect boundary carries
that. An effect is handed resolved parameters (`lumit_core::fx::Params` — floats, integers,
choices, colours, seeds, file slots and layer references) and pictures (`AuxSlot` in
`lumit-render/src/gpufx.rs` — the K-395 matte, layer inputs, temporal neighbours, a flow field,
a lens prescription), and `ParamKind` has no vector-geometry variant. The masks themselves live
on the layer (`lumit_core::mask::Mask`, `Layer::masks`) and are consumed by
`lumit_core::mask::apply_masks` in `lumit-render/src/build.rs` *after* the effect stack has run,
as a coverage buffer. Building either effect would therefore mean **a new kind of effect input**
— a resolved path list, sampled at the frame's time, arriving beside the matte — and that is a
docs/08 §1.1 and docs/17 change with a serialisation and an animation story of its own, not
something to force through a float parameter. Until it exists: Scribble's intent is Fill (§3.34)
inside the mask, and Stroke's nearest honest answer is **Vegas on the layer's alpha**, which
strokes the *shape* a mask cuts rather than the path itself. Both import as placeholders with
those suggestions (docs/11).

Two notes that are not decisions. **`active` is a WGSL reserved keyword**, exactly as K-406's
`target` is, and it fails the same silent way — Beam's short-circuit flag is spelled
`is_active`. And **a lightning bolt's core and glow are taken as a maximum over the segments,
never a sum**: every joint is shared by two segments and every fork meets its parent, so a sum
puts a bright bead at each of them and the bolt reads as a string of pearls.

## K-408 — An effect can be handed a mask's geometry, through one path input kind

**DECIDED** (2026-08-21). The seam Wave 2 stopped on, built deliberately rather than
around: Scribble, Stroke and Vegas's path half read a mask's *curve*, not its
coverage, and nothing carried one. Three pieces, mirroring the K-387/K-395 shapes:

- **`ParamKind::MaskPath`** — a parameter naming one of this layer's masks (by mask
  id, with "First mask" as the self-default the way `#[layer]` has `self_default`).
  The panel draws it as the layer's masks in a dropdown on the ordinary row system.
- **The carriage**: at resolve time the named mask's `path_at(t)` is flattened to an
  arc-length-parameterised polyline (a fixed, documented tolerance in px@comp) and
  rides beside the op the way K-387's aux slots do — CPU as a slice, GPU as a storage
  buffer. Absent or empty mask = the effect's documented no-op, degrade never fault.
- **The frame key** covers it for free where the mask is the layer's own (mask edits
  rename frames already); the flattening tolerance is a constant, so it cannot vary a
  frame's identity.

The consumers land with the seam: Scribble (§3.77+1), Stroke, and Vegas's Mask/Path
source, completing Wave 2's stopped rows. docs/08 §1.1 gains the input kind; the
bridge surface changes ride docs/17's ordinary process.

## K-409 — The three path effects share one kernel, and coarsen rather than stop

**DECIDED** (2026-08-21). K-408's consumers, landed: **Scribble** (docs/08 §3.78),
**Stroke** (§3.79) and **Vegas's Mask/Path source** (§3.76), catalogue at 85. Three
things about how, because each would otherwise be re-derived or undone:

- **One kernel, not three.** They differ entirely in *where the line goes* and hardly
  at all in *how it is drawn*. Where it goes is decided host-side — a hatch clipped to
  the mask, a brush trail along it, the mask itself — and what reaches the GPU is
  identical in all three cases: straight pieces in raster pixels, each carrying its
  distance along the drawing. §3.76's "a lit share of 2 is a continuous line" already
  switches the dash off without a branch, so Scribble and Stroke ride Vegas's dash
  machinery for nothing. One WGSL kernel, one CPU reference, one §1.6 oracle covering
  all three. This is K-399/§3.74's rule generalised: if it does not vary per pixel it
  does not belong in the kernel — and once it is out, effects can share the kernel.
- **Uniform, not storage buffer, and the budget coarsens.** The parity note left the
  buffer layout to the first consumer; the answer is 512 pieces in a uniform, which is
  Lightning's array twice over. Past it every consumer **coarsens and never truncates**:
  the hatch widens its spacing, the dots space out, a long chain straightens. Drawing
  part of a shape is the failure somebody sees; a slightly coarser whole one is not
  (docs/14 §4). A storage buffer becomes right the day something wants tens of
  thousands of pieces, and nothing does.
- **A dense brush stroke is drawn as the path it sweeps.** The gather-form question the
  parity note raised — Stroke's brush is a scatter — dissolves: a chain of round stamps
  spaced under half a brush width apart *is* the swept capsule, to within an eighth of a
  radius. Below that threshold Stroke draws the trimmed path; above it, separate dots,
  which is what Spacing is for. No scatter pass, no distance field, no second buffer.

**What is still owed to K-408's seam** is only what a row naming *one* mask cannot say:
AE's All Masks and Stroke Sequentially, and Scribble's two multi-mask Fill Types. Those
want a row naming a *set*, and the import reports them rather than guessing. Everything
else the two effects and the Vegas half needed is carried, and docs/11's Scribble and
Stroke substitutions are retired.

## K-410 — The Bridge captures; Rust converts

**DECIDED 2026-08-21.** The Lumit Bridge's bundle carries a **faithful, AE-shaped
capture** (`capture.json`) rather than a document in the Lumit project schema, and
every conversion — rational time, ids, keyframe carriage, effect mapping, retime,
mattes — lives in Rust, in a new engine crate **`lumit-import`**. This supersedes
the shape docs/11 §2.3 originally specified (a Lumit-schema `project.json` with an
`ae` namespace), which K-060 decided before any of the importer existed.

The reasons are the project's own rules. The Bridge is ExtendScript: CI cannot run
it, so every line of conversion logic written there is logic the regression suite
(K-007) can never cover — and the conversions are exactly the part that must not
drift. A dumb walker that records what the DOM says, one try/catch per property, is
also the piece most likely to survive Adobe's version drift untouched. docs/11 §2.2
item 9 already ruled this way for effects ("the Bridge does not know which effects
Lumit can map — it captures everything and lets the importer decide"); K-410 extends
that principle to the whole walk. The importer's output is an ordinary
`lumit_core::Document` handed to `lumit-project` to save, so there is still no
second dialect of the *Lumit* format to maintain — the capture schema is versioned
in the bundle's `manifest.json` and owned by `docs/impl/ae-import.md`.

One honesty note recorded at the same time: AE's own scripting DOM cannot read
`CUSTOM_VALUE` property data (Curves' point list, Levels' histogram, Hue/
Saturation's channel ranges). The plain sibling properties carry Levels and Hue/
Saturation fine; **Curves imports as a placeholder via the Bridge** until a blob
decoder exists (the direct-parse route's problem, §7), and docs/11's Curves row now
says so.

## K-411 — The Viewer bar is arranged in instruments, not a queue

**DECIDED 2026-08-22** (owner-directed, from an After Effects 2026 reference frame).
docs/07 §2.2 lists what the bar holds; this decides how it reads. The bar is a row of
**instruments** — small groups with one job each, separated by breathing room — rather
than an evenly-spaced queue of controls:

1. **The picture's scale**: magnification and preview resolution, two content-hugging
   dropdowns (value + chevron), never fixed-width boxes.
2. **The view toggles**, one tight icon cluster at icon spacing: region of interest,
   transparency grid (an icon now, not the word), layer controls, the background
   swatch. Toggles read in the accent while engaged, as they already do.
3. **How the pixels read**: the channel picker as a compact icon dropdown (its glyph
   tinted by the chosen channel) beside the exposure aperture + stops box and the
   tone-map icon — the three things that change what the numbers on screen mean.
4. **The clock**, its own field, click-to-type, beside the toggles rather than lost
   between transport and badge — the After Effects cue worth taking whole.
5. **The transport** with its playback-mode dropdown (Round keeps its K-394 pill).
6. **The right edge**: colour-management badge, degradation badge, preview progress —
   readouts, not controls, which is why they live apart.

Within a group the gap is small (4 px); between groups it is wide (12 px). No control
gains or loses a feature by this decision — item 8's badge, K-287's clock, K-314's
preview-only pair, K-362's region all keep their exact behaviour — and the Sharp and
Round shapes both follow it, Round on its detached tile. This is presentation, so the
regression tests assert grouping and order by key, not pixels.

## K-412 — Curves becomes a real curve

**DECIDED 2026-08-22** (owner-directed). The Curves effect's five fixed inputs (K-396)
are replaced by a real curve: a new `ParamKind::Curve` whose value is an ordered list
of 2..16 control points in the unit square (default the identity diagonal), one such
row per channel — Master, Red, Green, Blue, Alpha, After Effects' own five. K-396's
parameterisation was the honest floor while no editor existed; the owner has asked for
the editor, and a curve stored as its points is what an editor edits. The effect is
days old and unreleased, so the schema is replaced outright rather than migrated.

The evaluation discipline is Lightning's (K-407): the spline — a clamped cubic through
the points, Photoshop's family, since that is the curve every editor's hand already
knows — is baked once per resolve into a 257-entry table in f64, and both render paths
are handed the identical table, so the §1.6 oracle only has to check the *lookup*.
docs/08 §3.30 keeps its domain semantics (what the axes mean does not change, only how
many points may bend the line). Curve values do not keyframe in v1, joining File,
Layer and MaskPath on the static side of the seam; the editor is a custom panel row —
channel tabs, draggable points, click to add, drag out to remove — in both shapes.
The AE import row is unchanged: the blob stays unreadable via the Bridge (K-410), so
Curves still imports as a placeholder; what changes is that the day a blob decoder
lands, the target can carry the whole curve instead of a five-point sample.

## K-413 — Levels draws its histogram

**DECIDED 2026-08-22** (owner-directed). The Levels effect's panel presentation
becomes the editor everyone expects: the frame's histogram drawn behind the input
black, gamma and white handles, with the output range as a bar beneath. Parameters,
ids and semantics are untouched — this is presentation, so no import or oracle
changes. The histogram reads from the same data path the Scopes panel reads, fetched
once per displayed frame and only while the row is on screen — never in a rebuild
path (the bridge-call budget gates it, as everywhere).

## K-414 — A slider is a kind, and the Expression Controls exist

**DECIDED 2026-08-22** (owner-directed). Two related additions:

**`ParamKind::Slider`** — a bounded number drawn as a track and thumb with its value
beside it, for parameters whose whole meaning lives inside a closed range. The value
side stays `EffectValue::Float`, exactly as Int and Angle ride (the kind is the
control, not the storage). Angle set the precedent: a control with no arrangement of
existing rows that draws it earns a kind. Existing parameters may adopt it where a
closed range is the parameter's nature (temperature is the first candidate); adoption
changes no stored value.

**The Expression Controls family** — Slider Control, Angle Control, Checkbox Control,
Colour Control and Point Control, as parameter-only identity effects (they render
nothing; their one row exists to be read by expressions and driven by keyframes),
which is exactly what they are in After Effects and why half the CC-pack rigs in the
world are wired through them. They sit in the catalogue under their own **Controls**
category. Their AE match names are the famous ones (`ADBE Slider Control` et al.) but
are **not yet in the audited set**: per docs/11's rule they enter the import table
marked pending until the next audit sitting confirms them — the claimed-matchnames
list gains the five so that sitting is already prepared.

## K-415 — Tracking is classical, global, and zoom-aware; learned trackers are a plugin road

**DECIDED 2026-08-22** (owner-directed: object tracking and, more importantly, camera
tracking; robust to moving objects, maskable, and able to survive a zoom that jumps
between two frames). Four rulings:

**No bundled models**, the K-390 reasoning again: the current learned trackers
(MegaSaM, DATAP-SfM, MonST3R and kin) are hundreds of megabytes each and version
fast; they are the plugin road, and the seam stays open for them. What ships is the
strongest classical pipeline — the SynthEyes/libmv class, built on the 2024 global-SfM
results rather than the 2010 incremental ones.

**The pipeline is global, not incremental** (GLOMAP's revision of the field): 2D
tracks → pairwise geometry on selected keyframes → rotation averaging → global
positions → triangulation → one sparse bundle adjustment. Incremental chains drift
and die on the middle of a shot; global solves stand or fall honestly. Tracks are
pyramidal affine KLT (affine, because a zoom changes a patch's scale) seeded by the
flow engine we already ship, forward-backward verified. Moving objects are handled
the way the epipolar geometry says: RANSAC's dominant model wins, tracks that
disagree with it over their lifetime are segmented out and downweighted, and the user
can mask regions out entirely — the K-408 mask seam already carries mask geometry to
the engine, and the tracker reads the same carriage.

**Zoom is a first-class unknown.** Focal length is solved per segment, and a zoom
*cut* — the owner's scope-in, where focal leaps between two adjacent frames — is
detected from the tracks themselves (a burst in the median log scale change) and
treated as a segment boundary: pose continuity is kept across the cut while focal is
freed, which is what a scope-in physically is. A smooth zoom solves as a
spline-regularised focal within its segment. Principal point stays at centre; radial
distortion k1/k2 is per segment and optional.

**One substrate, two products.** Camera solve and object track share the track store:
an object track is a track group solved against the solved camera (rigid pose,
later), or exported directly as 2D keyframed transforms and corner-pin data (docs/08
§7's Tracker row, first). K-248's ruling stands: the tracker runs once on the full,
unaltered footage, and results map through retimes.

Lives in a new engine crate `lumit-track`; the how is docs/impl/tracking.md, which
pins the algorithms and the test plan. UI follows the engine (a Tracking workspace is
its own later piece).

## K-416 — The Viewer gains overlays and snapshots, in the display

**DECIDED 2026-08-21** (owner-directed, completing K-411's functionality audit against
After Effects' Composition panel). Two additions to docs/07 §2.2, both preview-only:

**The grid-and-guides menu** (§2.2 items 5–6's first real slice): one icon menu on the
bar's toggle cluster with checkable entries — Grid, and Title/action safe — drawn as
overlays on the picture in theme colours, session state per comp. Rulers and draggable
guides remain their own owed feature (docs/TODO.md); the menu is built so they land as
entries, not as new chrome.

**Snapshots**: Take snapshot stores what the Viewer is showing; Show snapshot swaps
the picture to the stored one while held, for the before/after read every grade leans
on. It is a *display* affordance, so it lives in the display: the stage captures its
own picture (the RepaintBoundary route the screenshot harness proved) and overlays it
while held — no engine copy, no cache entry, no export path anywhere near it. One
slot in v1; AE's four-slot Shift-F5 family can follow on the same mechanism if asked
for. Preview-only in the K-314 sense, and item 8's badge does not engage for it — a
held comparison is not a lying picture, it is a second picture, and releasing the
button is its whole lifecycle.

## K-417 — The tracker is an effect, and the camera it makes is a link

**DECIDED 2026-08-21** (owner-directed; where in doubt, After Effects is the
reference). Phase 4's surface, in five rulings:

**Camera track is an effect** — applied to a footage or precomp layer, rendering
identity, owning the analysis controls, the status readout and the Viewer overlay on
its layer, exactly the working shape the owner likes in AE: you keep editing while it
tracks. The effect is the *handle*; the work runs elsewhere.

**The analysis is keyed to the source, not the clip** (K-248 made this decision years
before the feature): the background job tracks and solves the **entire unaltered
source clip**, keyed by (media, analysis settings), cached in the project's sidecar
(`track/`, rebuildable like every sidecar tier, deterministic so a rebuild is
byte-identical). Every clip of that footage — trimmed, reordered, speed-ramped,
retimed, in a Sequence layer or not — reads the same solve through its own time
mapping. Reordering clips or changing a speed never re-tracks anything.

**The dynamic camera is a link, not a copy.** A Camera layer gains a *solve link*
naming a tracked layer; its transform and focal are derived per frame by walking the
full comp → clip → source time chain and reading the solve. While linked, the
camera's transform is read-only and wears a calm badge. The owner's precomp workflow
holds: a linked camera in the *parent* comp points at the precomp layer, and the
chain resolves through it to the tracked layer inside. A link that stops resolving
(the layer deleted, the media offline) holds its last derived motion and says so —
never a silent freeze, never a crash. **Convert to keyframes** bakes one key per
frame at the comp rate and severs the link; from then on it is an ordinary camera the
user edits (the bake is honest about being many keyframes — they are real, editable,
and the graph editor shows them like any others).

**The point cloud is on by default after a solve**: solved points draw over the
picture on the tracked layer (depth-cued, theme colours), selectable singly or
marquee'd; the creation gesture is AE's — selected points make a **Null** or a
**Solid** at their mean solved position, oriented to face the camera.

**`ParamKind::Action` enters the schema** for the Analyse button (and Cancel while
running): a parameter row that is a button, generic because the tracker will not be
the last effect that needs one (beat detection is already waiting). An Action carries
no value, never keyframes, and crosses the bridge as an event.

Analysis runs on its own thread — never a pool worker (docs/05's decode rule for the
same reason), cancellable between frames (the seam phase 3 recorded as owed), with
progress reported to the effect's status row and the overlay.

## K-418 — The importer reads the .aep itself, and the Bridge becomes the backstop

**DECIDED 2026-08-21** (owner-directed: "we need this to be seamless — users shouldn't
have to think about running scripts and enabling writing in After Effects"). This
supersedes the *priority* half of K-060: **direct `.aep` parsing is the primary
route** — the user picks an After Effects project file and it imports — and the Lumit
Bridge becomes the fidelity backstop and the verification harness rather than the
front door. K-060's reasoning (RIFX is undocumented and version-drifting) was true
and remains true; what changed is the evidence available: the community
reverse-engineering has matured into maintained, MIT-licensed chunk parsers
(forticheprod's, licence-checked — closing docs/11's open question), and Lumit now
owns something better than any of it: `tools/ae-bridge/fixtures/` holds a real
`.aep` beside AE's own byte-exact account of its contents. Every claim the parser
makes is checked against what After Effects itself said about the same file.

**The architecture is one funnel.** The parser emits the same `Capture` the bundle
reader produces — a second front end to `lumit-import`, not a second importer. The
whole mapping layer, the sixty-row effect table, the placeholders, the report and
the golden tests are reused unchanged, and the differential test (parse
`fixture.aep`, compare field by field against `capture.json`) measures recovery per
category instead of asserting it. Where the parser cannot recover something the
Bridge captures (a field, a property class, a whole feature), that is a report row
and a measured number, never a guess — and the report's suggestion for a
low-recovery project is the Bridge route, whose teaching string already exists.

Honesty that stands from K-060: a new AE version MAY break the parser at any time;
the UI copy says so calmly, the Bridge remains the answer that cannot drift, and the
bundle import stays a first-class citizen forever (studios export where Lumit is not
installed). The Kaitai/licence open question in docs/11 closes: nothing is vendored;
the MIT parsers are read as documentation and reimplemented under Lumit's own rules,
with attribution in the impl note.

## K-419 — Distances are pixels at composition size, never a percentage of the diagonal

**DECIDED 2026-08-23** (owner-directed). Every effect parameter that was declared
`% diag` — the blur family's radii and lengths, RGB split's and Block glitch's offsets,
Shake's amplitude, Twirl's, Spherize's and Ripple's reaches, Shadow highlight's radius,
Iris wipe's two radii — is now **px@comp**, with pixel-sized ranges and defaults. The
resolve step already scales px@comp by the preview factor and again for a different
export size, so a Half or Quarter preview frames exactly like the export, only softer;
nothing in any effect's maths changed. `Unit::PctDiag` stays only for ROI padding and the
reference format, and a test fails the build on any parameter that declares it. Saved
projects are **not** converted and the format version does not move: none exist in the
wild yet. This supersedes docs/08 §2.3's old "% diag" default wherever an earlier entry
(K-135, K-388, K-398, K-400 and the Wave 2 batches) relied on it.

## K-425 — Every matte row picks its channel and every Mix row picks its blend, once at the seam

**DECIDED** (2026-08-23, backlog 6.50 and 6.51; numbered K-425 because K-420..K-424 are
reserved by another branch). Two controls on the rows every effect already carries,
implemented once at the dispatch seam so no kernel learns about either:

- **Channel** beside Matte / Invert: which channel of the matte layer drives the effect
  (the shared `CHANNEL_OPTIONS` list — Luminance by default, the premultiplied Rec. 709
  luma every kernel has always read — then Alpha, Red, Green, Blue). The seam rewrites the
  matte once, before the kernel or the dissolve sees it, into a grey whose R = G = B = the
  channel, clamped, inverted if asked, alpha 1 (`cpu::matte_prepare`,
  `fx_matte_prepare.wgsl`). Invert therefore happens in exactly one place: Gaussian blur,
  Glow and Turbulent displace no longer invert inside their own maths. Effects that own a
  channel choice — Depth of field, Displacement map, Set matte, the Lens flare — opt out
  (`matte_channel = false`) and keep reading the raw RGBA matte, Invert included.
- **Blend** beside Mix: the layer blend modes verbatim (`BlendMode::ALL`), Normal by
  default, on every effect with a Mix slider except the Lens flare, which owns one. The
  Mix lives inside every kernel, so blending an already-mixed output would mix twice:
  when Blend is not Normal the seam runs the kernel at Mix 100 and applies
  `input·(1 − mix) + blend(input, unmixed)·mix` itself (`cpu::blend_seam`,
  `cpu::blend_mix`, `fx_blend_mix.wgsl`), in the compositor's domains (docs/06). Blend
  runs before the generic matte dissolve, so the matte still holds the whole result off.
- **The owner's rule for mattes**: the matte multiplies the effect's amount per pixel; it
  is not a mask. Where scaling the amount is mathematically the generic dissolve the
  effect keeps `MatteRole::Strength`. The **Matte key carries no matte row at all**
  (`matte = false`): a strength matte over a keyer is a garbage matte, a mask's job. Set
  matte keeps its `matte` — it is the effect's subject, not a strength — and gains no
  Channel row because it owns one.
- **Defaults are yesterday's bytes** (K-258). Luminance without Invert and Normal both
  run **no pass** — not a pass at identity, because a trip through an fp16 texture would
  requantise what the kernels read. An instance stripped of both rows resolves to the
  same numbers as a fresh one, and a matted document renders the same bytes with the
  rows stripped and with them at their defaults (`matte_end_to_end.rs`).

No new strings: "Channel", "Blend" and every option label already had `app_en.arb` and
`engine_labels.dart` entries from Set matte, the Lens flare and the layer Mode dropdown.

## K-426 — The matte scales the amount of every blur, sharpen and colour effect

**Status: DECIDED (2026-08-23).** Numbered K-426 because K-425 is already taken on main
(the channel and blend seam) and K-420..K-424 are reserved by another branch.

**Decision.** The owner's rule for mattes, applied to the Blur & sharpen and Colour
categories: **the matte multiplies the effect's amount per pixel, toward its neutral
value, before the maths runs.** It is not a mask. Seventeen effects claim their matte on
that basis (docs/08 §2.6 lists them and the control each scales); Gaussian blur and Depth
of field keep their earlier claims. Where scaling the amount is mathematically the generic
dissolve — the output is a straight lerp of the input — the effect keeps `MatteRole::
Strength` and nothing changes: **Contrast and Vignette** are exactly that (so the rulings
that named them are not carried out, by the rule's own test), as are Tritone, Black and
white, Tint, Curves, Levels, Invert, LUT and Broadcast safe. **Threshold** stays on
`Strength` because a cut has no honest per-pixel form that returns the colour picture
where the matte is black; the only formula is the lerp the row already applies.

**Mechanics.** One helper per path — `cpu::matte_toward` and its WGSL twin, `neutral·(1 −
k) + value·k`, spelled so k = 1 is the value to the bit — so an empty matte reproduces
the pre-claim function byte for byte (K-258) without a second code path. Three controls
carry the raw number into the kernel because a lerp of the host's derived numbers would
not be "the control scaled": Exposure's gain is `exp2(stops·k)`, Temperature's gains are
rebuilt from `t·k` (the blue gain floors at 0), Hue shift builds the matrix for `angle·k`
in the kernel. Beside a non-Normal Blend the order is the one K-425 gives for every
override: the kernel (with the matte inside it) first, then the blend, then the Mix — so a
black matte under Multiply squares the source, exactly as an effect at amount 0 does.

**Held by** `check_matte_claim` (`lumit-gpu/src/fx/tests.rs`): per effect, parity under a
ramp matte and bit-stability, the empty matte equal to the old function to the byte, a
flat half matte *not* equal to the generic dissolve, and parity there too.

## K-427 — The matte scales the displacement of every distortion, at the destination pixel

**Status: DECIDED (2026-08-23).** Numbered K-427 because K-425 and K-426 are already taken
on main and K-420..K-424 are reserved by another branch.

**Decision.** The owner's rule for mattes (K-426), applied to the Distortion category: a
distortion's amount is a **distance**, so the matte multiplies *the displacement* per pixel
before the maths runs. Fourteen effects claim their matte on that basis — RGB split's and
Chromatic aberration's Amount (both tiers), the Shake's Amplitude and Rotation amount,
Block glitch's Intensity, Offset's shift, Lens distort's distortion, Corner pin's and
Bezier warp's pull from the frame's own corners, Twirl's Angle, Spherize's Bulge, Ripple's
and Wave warp's Wave height, Warp's Bend with both distortions. Turbulent displace (K-395)
and Displacement map (K-402) had already claimed theirs on this rule before it was written
down. Each names what it scales in its declaration, which is what the manual prints.

**k is read at the destination pixel, everywhere.** A distortion runs backwards — it asks,
per output pixel, which input pixel belongs there — so the matte could be read at either
end. The destination is the only choice under which the matte's own picture is the picture
that comes out: paint a black region and *that* region of the finished frame is the one
that did not move. It is also what makes the claim visible on a whole-frame move: a soft
matte on the Shake turns a shove into a **warp**, one picture that moved by different
amounts, where the dissolve gives two of every edge.

**Two exceptions, both by the rule's own test.** **Scanlines** cannot scale Intensity —
that is the generic dissolve to the bit — so its matte **divides Line period** instead:
the lines spread apart as the matte darkens and are too far apart to see at black, the
divide floored at `cpu::SCANLINES_MIN_K` (`1e-4`, the identical literal in the WGSL), with
Intensity untouched. **Datamosh keeps `MatteRole::Strength`**: its output is
`current·(1 − Intensity) + melted·Intensity`, so scaling Intensity per pixel and dissolving
per pixel are the same arithmetic — the rulings that named it are therefore not carried
out, for K-426's reason. **Tile, Mirror and Polar coordinates** keep `Strength` too: a
repeat, a reflection and a change of coordinates have no amount to scale.

**Mechanics are K-426's, unchanged.** `cpu::matte_toward` and its WGSL twin for the pulls,
`_matted` CPU twins with the old names kept as the `&[]` wrappers (an empty matte is the
pre-claim function byte for byte, K-258), a `matte_on` in each uniform reading binding 4,
`dispatch_matted`, `aux.matte()`. Two kernels are shared and only one side claims:
`fx_transform.wgsl` serves the Shake (which binds a matte) and the Transform effect (which
passes `None` and keeps the dissolve), and `fx_spectral.wgsl` serves the Wavelength tier of
both splits.

**Held by** `check_matte_claim` per effect, plus three pictures a dissolve cannot draw: a
half matte on a 6,4 shove is the 3,2 shove, a half matte on a 200° twirl is the 100° twirl,
and a half matte on Scanlines is the lines at twice the period. No new strings.

## K-428 — The matte scales the amount of every generator and stylise effect

**Status: DECIDED (2026-08-23).** Numbered K-428 because K-425, K-426 and K-427 are already
taken on main and K-420..K-424 are reserved by another branch.

**Decision.** The owner's rule for mattes (K-426), applied to the Generate and Stylise
categories. Two shapes of amount, one rule:

- **A thing drawn over the picture has an opacity**, and the matte multiplies that, so the
  drawing fades along its own length and what lies underneath is untouched. Lightning's
  bolt (its core's coverage and its Glow opacity together), Radio waves' Opacity, Vegas'
  Opacity, Scribble's and Stroke's Opacity, and Drop shadow's shadow Opacity.
- **A thing with a size has that size scaled.** Roughen edges' Border, Median's Radius,
  Emboss's and Texturize's Relief — and Add grain's Intensity, which belongs here rather
  than with the additive grains because its wobble is added on the perceptual value and
  squared back.

Eleven claims. Each names what it scales in its declaration, which is what the manual
prints.

**Four rulings are not carried out, by the rule's own test.** **Noise**, **Flash**,
**Sprite flare** and **Light wrap** were each named for their Amount or Intensity, and each
adds a *linear* amount of something to the picture — grain onto unpremultiplied colour, a
colour lerped toward, additive light, a screened spill. For all four, `out(amount·k)` is
`lerp(input, out(amount), k)` identically, so scaling the amount and running the generic
dissolve are the same arithmetic and there is nothing to change: they keep
`MatteRole::Strength`, for K-426's reason and Datamosh's (K-427). This is not a judgement
call — `check_matte_claim` refuses a claim that is the dissolve, so the gate would fail.
**Fill, Gradient, Fractal noise, Beam, Mosaic and Find edges** keep Strength as the task
ruled: they replace the picture rather than adding an amount to it. Glow keeps its seed
gate (K-395) and the Lens flare its source detection.

**Two claims draw a picture the dissolve provably cannot.** A half matte on a **Median** set
to Radius 2 is *exactly* the Radius 1 median, to the bit — a genuinely smaller window, which
is what lets one painted matte despeckle a sky and leave a face alone. A black matte on
**Emboss** is the flat mid-grey sheet, to the bit, and not the picture back: Relief 0 is
that sheet and not the identity (§3.67), so this is the one place in the batch where a black
matte does not mean "leave the pixel alone" — honestly, because the matte turns Relief down
rather than turning Emboss off. The drawn effects make their own case in the composite
modes that replace rather than overlay: with Composite on original off, or on Paint on
transparent or Reveal original, a black matte leaves transparency where the dissolve hands
the whole picture back.

**k is read at the destination pixel**, as K-427 fixed for the distortions. It matters once,
on **Drop shadow**: the shadow's Opacity is scaled where the shadow *falls*, not where the
shape stands, so the matte's own picture is the picture of where the shadow goes. Its blur
takes no matte at all — that blur is taken where the shape is, and a per-pixel softness there
would soften the wrong picture.

**Mechanics are K-426's, unchanged.** `matte_toward` on each path, `_matted` CPU twins with
the old names kept as the `&[]` wrappers (an empty matte is the pre-claim function byte for
byte, K-258), a `matte_on` in each uniform reading binding 4, `dispatch_matted`,
`aux.matte()`. Two reuses are worth naming. **Roughen edges' whole claim is one argument**:
Border *is* the radius of the §3.8 gaussian it already runs, and that blur has scaled a
radius per pixel since K-395, so the claim is `None` becoming `matte` at one call site on
each path. And **`path_draw` is shared by three effects** — Scribble, Stroke and Vegas'
Mask/Path half — all of which claim, so unlike K-427's `fx_transform.wgsl` there is no side
that passes `None`. It scales the sample's result rather than the Opacity field, because
that result *is* coverage × Opacity and Opacity enters nowhere else. **Median** is the only
kernel whose loop bound the matte moves: `keep` is now derived from the pixel's own radius
instead of read from the uniform, and a pixel whose radius comes to 0 takes the short-circuit
the whole effect takes at Radius 0.

**Held by** `check_matte_claim` per effect (parity under a ramp, bit-stability, the empty
matte equal to the old function to the byte, a flat half matte *not* equal to the generic
dissolve, and parity there too), plus the three equalities above. No new strings.

## K-429 — The matte scales shutter, decay and completion, and the keyers carry no matte row

**Status: DECIDED (2026-08-23).** The owner's rule for mattes (K-426, K-427, K-428) applied
to the last two families, Temporal and Transition, and the last two opt-outs settled in
Utility. Numbered K-429 because K-425..K-428 are taken on main and K-420..K-424 are reserved
by another branch.

**Temporal: the amount is a duration.** **Echo** scales its **Decay**, so the ghosts are
genuinely fewer and shorter where the matte is dark rather than a long trail faded back.
Because `(decay·k)^(i+1)` factorises as `decay^(i+1) · k^(i+1)`, the per-pixel weight is the
whole-frame weight multiplied by `k` exactly `i + 1` times, which makes a white matte the
unmatted picture *to the bit* and a half matte *exactly* the half-decay trail. A tap the
matte has taken to nothing is **skipped**, the same `continue` a dead tap already takes,
because a zero-weight tap is not a no-op under every combine mode — Multiply by nothing is
black. The kernel can read the matte at all because the host already runs one dispatch per
tap, so the pass knows which tap it is; the tap index joins the uniform.

**Fast motion blur** (§3.2) scales its **Shutter angle**, read at the destination pixel and
spent everywhere the shutter is spent — this pixel's own vector, the neighbourhood's dominant
sweep, and each tap's reach — so both sides of the McGuire weighting still speak the same
language about how far a thing moves, and a half matte is exactly the half-shutter streak.
It reads the matte on its own bind layout rather than the shared one, because it has three
sampled inputs before it gets to a matte; Depth of field and Datamosh share that layout and
bind their own source in the new slot, unread.

**Motion blur (accumulation, §3.26) scales its Shutter angle in the combine**, because it
has no kernel to claim it inside: it orchestrates a re-render, resolves to no op, and is
skipped by the matte carriage `run_ops` walks on both sides. The matte therefore rides on the
sub-frame plan (`AccumulationBelow`), rendered by the same helper every other matte goes
through, with its Channel and Invert applied once before the combine reads it — the seam's
job, done by hand where there is no seam. The combine treats the samples as cells (sample *k*
owns `[k/N, (k+1)/N]` of the open shutter) and averages over the whole span scaled toward the
**shutter anchor**, `−phase ÷ shutter` clamped to 0..1, which is where the frame's own time
falls. That anchor is the whole of the design: black has to mean *the frame*, and with the
standard −90° phase on a 180° shutter that is the middle of the span, not the instant the
shutter opened. A cell's weight is its overlap with that window over the window's own width,
clamped, so the weights sum to one at every strength; the window is floored at a thousandth
of the span, because `hi − lo` at 1e-6 loses more than a per cent of the answer to
cancellation. With no matte bound the old equal-weight hardware pass runs unchanged, byte for
byte (K-258).

**Posterize time keeps the strength dissolve**, by the rule's own test: it holds a time
rather than drawing an amount, and its own output is what a dissolve blends.

**Transition: the amount is Completion, and scaling it makes a gradient wipe.** Linear wipe,
Radial wipe, Venetian blinds and Card wipe each scale **Completion** toward 0 — the edge is
further along where the matte is bright, so a painted ramp sweeps the frame in the ramp's own
shape. The Card wipe asks it per *pixel* and not per card, so one half of a card can stand
while the other half has flipped away; that also settles §3.72's fourth decision, which
declined AE's **gradient flip order** for want of a row to put it on. Painting a ramp on the
matte *is* that order now, and "only over the sky" is what a mask is for.

**The Iris wipe has no Completion** — the radius is the transition (§3.71) — so it scales
**that**, which is the same sentence about the same thing. It costs one multiply: the solved
sector's vertex is the only place a radius survives into the expression, so scaling it scales
the outer and inner radii together and leaves the edge's direction, and so the normal, alone.
A half matte draws exactly the half-radius iris; a black matte takes the same exact identity
short-circuit Outer radius 0 already takes. **Transform** and **Broadcast safe** keep the
dissolve, because scaling their amount *is* it, and the **Camera track** carries no row at
all (K-417).

**A Motion vectors layer may stand in for the measured flow** (backlog 7.48). Fast motion
blur gains a **Motion vectors** Layer row whose red and green channels are the per-pixel
motion in the encoding every velocity and vector pass already uses — red sideways, green
up-and-down, **mid-grey standing still** — so the motion in pixels is `(r − ½) · Vector
scale` and `(g − ½) · Vector scale`. Blue and alpha are not read, and confidence comes out at
1 everywhere: a supplied vector is not a measurement that can have failed to match. **Vector
scale** (px@comp, default 32, greyed until a layer is picked) reconciles one engine's
normalisation with the frame it came from. The layer is converted to the same field the flow
engine measures, in one pass on each path, *before* the tile reduction — so everything
downstream reads one kind of field and knows nothing about where it came from, and the effect
works for the first time on a layer that has no measured flow at all.

**Utility: two effects carry no matte row.** The Matte key already did (K-425). **Set matte**
joins it. Every Matte row answers "how much of me happens here", and Set matte has no answer
to give: what it takes from another layer is the coverage itself, which is the whole effect
rather than an amount of one. So it stops claiming the universal row and declares its **own**
source picker instead, under the same ids and the same labels, riding the ordinary
auxiliary-layer carriage beside Light wrap's Background. No dissolve stands beside the kernel,
no injected Channel row duplicates the one it already had, and Invert is applied once, inside
the kernel. A project saved before this loads exactly as it did (K-065): the
forward-migration walk only ever *appends* what a schema has grown, so a row nobody declares
any more — the Matte key's three, or a Channel this effect no longer injects — is carried
along untouched rather than rewritten. That is the deliberate half of K-258, and the same
courtesy Gaussian blur's unread `mode` and Posterize time's unread `scope` already get;
`migrate_lens_flare_background` is the other shape, for a value that had somewhere new to
*go*, and none of these has.

**Which list a layer arrives on is the schema's answer, not a table.** The layer-input
carriage was a `match` on match names in `build.rs` and an `AuxKind::LayerInput` in
`run_ops`; both are gone. `EffectSchema::layer_input()` — the first `ParamKind::Layer` row
that is not the matte — is the one predicate both sides walk, exactly as `matte.param()` and
`mask_path()` already are, and the slot is a **field** on the aux slot beside them rather
than a kind. That is what lets Fast motion blur read a whole flow field *and* a Motion
vectors layer *and* a matte at once: a variant per pair is the combinatorial seam the matte
was kept out of in the first place.

**Mechanics are K-426's, unchanged.** `matte_toward` on each path, `_matted` CPU twins with
the old names kept as the `&[]` wrappers, a `matte_on` in each uniform reading binding 4,
`dispatch_matted`, `aux.matte()`. The five wipes' `*_keep` helpers take their completion as
an argument now instead of reading it off the params struct, since a uniform cannot be
rewritten per pixel.

**Held by** `check_matte_claim` per effect (parity under a ramp, bit-stability, the empty
matte equal to the old function to the byte, a flat half matte *not* equal to the generic
dissolve, and parity there too), plus four equalities a dissolve cannot produce: a half matte
on a radius-8 iris is the radius-4 iris to the bit, a half matte on decay 0.6 is the
decay-0.3 trail to the bit, a half matte on a 360° shutter is the 180° streak to the bit, and
a black matte on Echo under Multiply leaves the frame alone rather than turning it black. The
accumulation combine is held to three of its own (white is the equal-weight average, black is
the sample at the frame's own moment, half is the middle-half average and *not* the dissolve
between them), the Motion vectors layer to a round trip through the measured-flow blur, and
the two keyers to a load of a save that still holds their dropped rows. One new string:
`fxVectorScale`.

## K-420 — A cached frame is shown at once and measured afterwards

**DECIDED 2026-08-23.** An
amendment to K-276 (4) and (8) together. (4) made a measured frame a composited one and
stepped over the whole cache ladder while the clock was on; (8) then turned the clock on
by default. Put together, every scrub onto a frame the cache bar showed green composited
it again — fenced at every layer and every effect — on arrival, and the bar's green was
a promise the Viewer did not keep. The owner's ruling: keep (8), and **serve the hit,
measure afterwards.** A held frame on the card, in memory or on disk is served to a
measured request exactly as to any other; the worker notes that it was, and on its
next idle lull composites that one frame again with measuring on and discards the
picture, so the column fills one idle turn after the picture. Idle fill and playback
stay unmeasured, so nothing is measured that nobody is looking at. Two fixes landed
beside it: the 1% cache key is taken from the scale rounded to a thousandth first, so
the bar and the scrub name a frame alike (about one scale in twenty landed on
different 1% steps before); and the Scopes read a frame held on the card back rather
than compositing it again, and no longer re-request the frame they already show. This
number was allocated on the lane3 branch.

## K-421 — Each effect's output is kept, so editing the last one re-runs only that one

**DECIDED 2026-08-23.** The only cache the render path had was the finished comp frame (K-178,
K-214), so an Exposure after a Lens flare re-ran the flare on every nudge. A per-effect
intermediate cache now sits in the realiser — VRAM only, 256 MB by default, beside the LUT
cache — and `run_ops` names each op's output by content (docs/06 §5.2: the input's
identity, the raster, the flare bake generation, and every op up to that one with its
resolved values and side inputs), looks for the longest held prefix from the last op
backwards, and runs only what follows. Only committed, non-playback renders add to it;
drags and playback read it. v1 boundaries, all recorded in §5.2: only sources made from
bytes (footage, sequences, solids) are named; an op binding another layer's texture, the
neighbour frames or the flow field breaks the chain. This is the effect-stack slice of the
node-output cache the K-178 evaluator will own, built ahead of it where the stacks run,
and it does not change the evaluator plan. Preview and export remain byte-identical
(K-031): a held prefix is the same texture a cold walk makes. This number was allocated
on the lane3 branch.

## K-422 — A precomp's frames are cached as one unit

**DECIDED 2026-08-23.** The manual promised that a
precomposed section caches as one unit, and docs/06 §5.2 that the same nested comp used in
five places renders once; neither was true — a non-collapsed Precomp layer walked its comp,
decoded its footage and realised every inner layer on every parent frame, and the frame
key folded the nested comp into the parent's hasher inline so the nested frame had no name.
Now `lumit-eval` names a nested comp with a fresh hasher (its own `comp_frame_key` at the
layer time on the flick grid, same quality tier) and folds that name into the parent; the
draw builder carries it on the nested draw; the realiser serves the nested comp's linear
texture from the K-421 store by that name mixed with the exact render scale and sample
count, and files it on committed renders; and the decode planner asks the store (through
a caller-supplied answerer, the one place planning knows a cache exists) and plans no
decodes for a held comp, pinning what it relies on until the frame is realised. Collapsed
Precomps are never cached; held re-renders (Posterize, accumulation blur) and draft renders
carry no name; a measured frame realises the comp in full so its inner rows get numbers.
Preview and export stay byte-identical (K-031). `ALGO_VERSION` bumped to 4. This number
was allocated on the lane3 branch.

## K-423 — Layers under a full-frame opaque layer are not rendered

**DECIDED 2026-08-23.** The only gates
on the comp walk were hidden, out of span and solo, so a full-frame solid over a stack of
footage still decoded, uploaded, effected and composited everything under it. A single
predicate in `lumit-core` (`occlusion::occluder_index`, docs/06 §1.1) now names the topmost
layer that provably covers the frame opaquely, and both the draw builder and the decode
planner skip every layer below it. The v1 predicate is deliberately narrow — a Solid with
alpha 1, 2D, unrotated, Normal blend at full opacity, no masks, paint, effects or motion blur,
its axis-aligned placement (parent chain included, nothing driven by an expression) covering
the comp rectangle; no active camera; no visible Adjustment above it; nothing visible above
it referencing a layer below as a matte or a layer input; never inside a collapsed Precomp's
splice — because a wrong cull costs pixels while a refused one costs only speed. Footage
whose probe reports no alpha is left for a later extension: the predicate sits below the
probe and would need the answer threaded down the way K-422's `held` question is. The frame
key keeps hashing culled layers; preview and export stay byte-identical (K-031). This number
was allocated on the lane3 branch.

## K-424 — The idle fill wraps the work area and keeps going into RAM

**DECIDED 2026-08-23.** Supersedes
the bound in K-187 ("bounded by the budget — it stops before the LRU would churn"). The
fill used to keep a window of VRAM-budget frames around the playhead and stop: it never
wrapped to the start of the work area, and the far side of a loop longer than the window
was evicted as the walk went forward, so playback re-rendered it every pass while the
worker sat idle. Now `fill_order` wraps in both directions (playback loops the work area,
so the frame after the last is the first) and ends when every frame has been visited once,
2:1 throughout; and the walk's reach is VRAM plus RAM rather than VRAM alone — once the
card is full each render evicts its stalest frame into memory, which is the existing
demotion path, so the whole work area ends up held in one tier or the other. The LRU stays
the eviction authority. One rule keeps it from churning: a frame held in memory is climbed
only while the card has room. Regression tests:
`the_fill_order_is_forward_biased_and_complete` (playback.rs) and
`the_fill_keeps_going_into_memory_once_the_card_is_full` (worker_thread.rs). This number
was allocated on the lane3 branch.

## K-430 — The Viewer asks again when it changes size, and the point cloud follows the effect and the solve

**DECIDED 2026-08-23.** Three frontend staleness bugs with one shape: something changed
what should be on screen, and nothing told the thing that draws it. (1) On Auto the render
scale is measured by the Viewer's layout, and `LumitUiState.reportViewerScale` only stored
it — so the first frame of a session was made at the size the window opened at and stayed
there, because growing a panel is neither an edit nor a move of the playhead. It now
compares the new scale with the old at 1 % granularity (the engine's own key resolution) and
schedules one `requestFrame` after the frame; a fixed tier is exempt, and the Viewer passes
`settled: false` while its zoom animation runs, so a flight asks for the frame it lands on
rather than one per tick. (2) The point cloud's presence was read outside any listener, so
disabling the Camera track left the dots on the picture; the block now sits under a
`ListenableBuilder` on the read model. (3) A landed solve changes what the engine would
answer while moving neither the playhead nor the document's revision — the cloud's whole
read memo — so the dots did not appear until the frame changed. A `solveLanded` counter on
`LumitUiState`, bumped by the Camera track card on the Done transition beside a
`requestFrame`, is the third key. Regression tests: `a Viewer that grows asks for the frame
again` (bridge_call_budget_test.dart, where it also holds the budget: one request for the
change, not one per layout), `disabling the effect removes the cloud, with no frame change`
and `a landed solve is read without the playhead moving` (camera_track_frb_test.dart). No
new user-facing strings. This number was allocated on the lane3 branch.

## K-431 — An animated aperture no longer stops the flare's frames from caching

**DECIDED 2026-08-23.**
Supersedes the second invariant of K-350 ("a provisional frame is unnameable") and the
K-421 op name that carried the bake generation. K-350 refused to name *any* frame while
*any* lens flare bake was in flight, which was right for the case it was written for — a
user picking a lens, one bake, half a second — and wrong for the case that followed. A
keyframed f-stop (or blades, aperture rotation, softness, roundness, a per-element coating)
asks for a slightly different iris on every frame, so a bake was in flight for as long as
it played: every frame of every comp went unnameable, flare or no flare, nothing entered
any tier, and the idle fill stood down and stayed down.

Three changes, none of which weakens the rule K-350 was protecting — that a frame drawn
with the previous lens must never be filed under the new lens's name (K-178).

**(1) The question is counted, not guessed at.** `LensFlareFx` bumps a `substitutions`
counter at the one place a frame draws optics its parameters do not name: the deferred
fallback to the last drawn bake (or to no flare at all, with none drawn yet). Read either
side of a render — `FxEngine::flare_substitutions`, `HeadlessRenderer::flare_substitutions`
— it answers *may this frame be kept?* exactly, where `flare_bake_generation` could only
answer *is something being baked somewhere?*. `frame_key` no longer refuses to name a
frame, the frame caches and the effect cache bank on the count instead of the generation,
and the op names of K-421 carry the count rather than the generation. The generation keeps
its other job: telling the worker a bake has landed and the picture is worth making again.

**(2) The bake sees a snapped aperture.** The bake precomputes two things from the iris —
the starburst sprite (a Fourier transform of the hole, eight field slices) and the
auto-exposure gain — and costs about two thirds of a second. Neither is separable into the
per-frame pass at any acceptable cost: the sprite is an FFT, and the gain is a thumbnail
render. So `bake_params` snaps the continuous iris dials before both `bake_key_with` and
`bake_with` read them — the working f-number to a twentieth of a stop, aperture rotation to
half a degree, Roundness and Iris softness to 1/256 — and a run of frames shares one bake.
Nothing the *frame* computes is snapped: the ghost trace's own stop scale, the iris mask it
draws each rim with and the K-260 wide-open blend all read the raw dial, so the ghosts
still shrink and turn smoothly. What steps is the starburst's shape and the exposure, by
about 1.7% a step. The trade is deliberate and stated: a slow f-stop ramp shows the
exposure in steps rather than a continuous slide, which is the price of the bake being
shareable at all.

**(3) A file parameter's key covers the file, not just its path.** `lumit-eval` fed the
path string alone into the frame key, while the flare's bake keys on the file's *content*
(`lens_text_hash`) — so editing a `.lens` file on disk rebaked, drew different optics, and
filed the result under the old file's name: a cached frame no edit or undo could clear. The
key now folds the file's size and last-modified time too (a stat, not the bytes: a LUT is
megabytes and this runs for every frame key). `ALGO_VERSION` goes to 5, so no frame made
under the old rules is addressed again. Recorded limit: a file rewritten inside one
filesystem tick at exactly the same length is not seen.

Regression tests: `a_baking_flare_does_not_unname_other_frames`,
`a_keyframed_aperture_names_every_frame`, `an_edited_lens_file_renames_frames` (headless.rs),
`a_bake_in_flight_does_not_rename_every_op` (fxops.rs),
`lens_flare_bakes_are_shared_across_one_step_of_aperture` (lumit-core fx/tests.rs — both
halves of the cache's promise: two f-stops inside a step key the same *and* bake
bit-identically), and the substitution counts added to
`lens_flare_deferred_bakes_answer_with_the_previous_lens_then_the_new_one` (lumit-gpu).
No new user-facing strings. This number was allocated on the lane3 branch.

## K-432 — The flare's auto-exposure reads the native stop, so stopping down dims it honestly

**DECIDED 2026-08-23.** Amends K-431 (2), which recorded the exposure gain stepping with the snapped
working f-number as a deliberate trade, and settles the residue that entry left: on a slow
f-stop ramp the whole flare's brightness stepped about 1.7% at every twentieth-of-a-stop
boundary, because the auto-exposure probe (K-258) rendered its thumbnail at the working
aperture and its gain therefore tracked roughly `(f/native)²`.

The fix is the physical one rather than a finer step. **The probe is shot at the lens's
native (maximum-aperture) f-number.** The gain is a property of the glass — how much light
this prescription's ghosts put on the sensor with the iris open — and measuring it through
a closed iris made it cancel the stop-down exactly: a lens rendered the same flare
brightness at f/16 as wide open, which no lens does. Stopping down now dims the flare by
the square of `fstop_scale`, as the light the iris passes dims; **Intensity** is the user's
knob for putting it back, and no shipped parameter moves. The default frame is
correspondingly dimmer at the default f/2.8 on a fast prime, which is the visible change.

Two consequences worth stating. The exposure no longer steps at all under an animated
aperture — one gain per lens, whatever the iris is doing. And the working f-number stays a
bake-key input, but for **one** reason only: `effective_roundness`'s K-260 wide-open blend
decides how round the diffracting hole is, so it shapes the starburst sprite. The K-431
snap therefore stays exactly as it is (the sprite is an FFT and cannot move per-frame),
now documented as covering the sprite alone — impl/lens-flare.md §5c, §5d.

Regression test: `lens_flare_auto_exposure_reads_the_native_stop` (lumit-core fx/tests.rs)
— two working f-stops on one lens bake bit-identically the same gain, and the stopped-down
frame renders measurably dimmer. No new user-facing strings. This number was allocated on
the lane3 branch.

## K-433 — The ROI tile paddings are pixels at composition size too

**DECIDED 2026-08-23** (owner-directed follow-up to K-419, which converted every
*parameter* to px@comp and left the ROI paddings behind). `Roi::PaddedPctDiag(pct)` is
**removed** and replaced by `Roi::PaddedPx(px)`: the margin of spare input an effect needs
around a tile is quoted in px@comp, and `Roi::padding_raster_px(px_scale)` resolves it by
the same multiplication a `Unit::Px` parameter gets in the resolve step — so a padding and
the radius it exists to cover move together under preview resolution. It rounds up to whole
pixels and never resolves below one, so a 3×3 kernel still has its neighbours at Quarter.

This supersedes the padding half of K-419 (`Unit::PctDiag` now stays only for the reference
format) and every `PaddedPctDiag` figure the Wave 2 batches recorded in docs/08.

**Why it was wrong.** Since K-419 every radius is a pixel count, so a percentage of the
diagonal no longer tracks anything the user can type: 25 % of a 1080p diagonal is 551
pixels, and Gaussian blur's hard maximum is 2 000 — a typed radius clipped at the tile edge
on any comp that size or smaller. Each of the seventeen declarations is now sized from the
effect's **own hard maximum** (Gaussian blur and Channel blur 2 000, Shadow highlight 2 000,
RGB split 500, Unsharp mask 100, Median 3, the two one-pixel kernels 1). Where the hard
maximum is open — a value may be typed past the slider — the padding is the slider's
maximum **doubled**, said so in a comment at the declaration: Displacement map, Turbulent
displace, Wave warp and Roughen edges 1 000, Light wrap 400, Depth of field 80, Emboss and
Texturize 40, Sharpen 16.

Nothing consumes the padding yet — no tiling scheduler exists — so no rendered pixel moves;
this is the declaration and its resolver being made honest before something reads them.
The OFX/plugin surface (docs/12) never named `PaddedPctDiag`, so nothing is kept deprecated,
and `fx-reference.json` does not export ROI, so it is not regenerated.

Regression tests: `the_roi_padding_follows_the_raster_like_a_px_radius` and
`the_roi_padding_covers_the_hard_max_radius_at_1080p` (lumit-core `fx/tests.rs`) — the
second fails against the old 25 % figure. No new user-facing strings.

## K-434 — Render workers take turns building their renderers, and skip it for a project that has gone

**DECIDED 2026-08-23.** A render worker builds its `HeadlessRenderer` — a GPU device plus
every pipeline the compositor needs — before it serves its first request. That build is now
**serialised process-wide** by one mutex held across `HeadlessRenderer::new()` and nothing
else, and once a worker's turn comes it **checks its project is still open** and stops
without building if it is not (`worth_building_for`, `crates/lumit-bridge/src/api/
worker_thread.rs`).

**Why.** Building one takes seconds where the driver has no warm shader cache — measured on
a Windows development machine at 3.3–5.0 s against a first render of about 30 ms — and it
cannot be interrupted once begun. A process that opens projects faster than that had every
build in flight at once: the `viewer_panel_frb` suite makes a project per test and draws in
most of them, so ninety devices were requested inside a few seconds and the card ran out
part way through. From there `HeadlessRenderer::new()` failed with "Not enough memory left"
for projects that were perfectly healthy, their workers stopped, and their Viewers showed
nothing — which read as a broken transport rather than an exhausted card.

Both halves are needed and neither works alone. Serialising bounds the peak at one device
under construction; the liveness check is what keeps the queue from then building a device
for each of the projects that opened and closed while it waited. Twenty at once was never
faster than twenty in turn — the driver serialises device creation regardless — so nothing
is given up.

**The editor never notices.** It has one project open and therefore one worker, so the lock
is never contended and the check always passes. This is entirely about processes that hold
many project sessions in a row.

**A recorded deviation from docs/14.** The engineering rules say a lock must not be held
across an FFI call, and this one is held across exactly that. The rule exists so a slow call
cannot stall a thread that is waiting for *data*; `BUILDING_RENDERER` guards no data, and
the slow call is the thing it exists to order — it is a queue, written as a mutex because a
queue of one is a mutex. The cost accepted with it: a build that hung would now hold every
other worker behind it rather than failing on its own. `HeadlessRenderer::new()` either
returns a device or an error today, and a worker that cannot build one stops; if that ever
becomes a hang, this wants a timeout rather than a wider lock.

Regression test: `a_closed_project_is_not_worth_a_renderer` (lumit-bridge
`api/worker_thread.rs`). The suite-level evidence is `viewer_panel_frb_test.dart`, which
went from seven failures and ten failed device requests to green with none. No new
user-facing strings.

## K-435 — The Audio layer is a flag on the layer, not a layer kind of its own

**DECIDED 2026-08-23.** An **Audio layer** ([01-GLOSSARY.md](01-GLOSSARY.md): "a layer whose
source is an audio item, or the audio channel of footage") is a Footage layer carrying
`Layer::audio_only = true`. There is no `LayerKind::Audio`. The flag says one thing — *this
layer contributes sound and no picture* — and everything else about the layer is unchanged.

**Why a flag.** Audio-only media already worked end to end before this entry: a placed music
file became `LayerKind::Footage`, `AudioJobsBuilder::audio_jobs` found it by that kind and
probed it for an audio stream, and it mixed and played. The only things missing were a way
to take a *video* file's sound without its picture, and the picture paths knowing not to ask
for pixels. A new layer kind would have thrown away the working half: `audio_jobs`, the
waveform peaks, Retime, the matte and layer-parameter menus, the project file and the AE
import all resolve a footage source by matching `LayerKind::Footage`, and each would have
needed a second, footage-shaped arm to learn. The flag keeps every one of them working
untouched, and the cost is one `bool` that a non-footage layer ignores.

It sits on `Layer` rather than inside the `Footage` variant for the same reason `volume_db`
does: it is how the layer *uses* its source, it defaults cleanly for every project written
before it existed (`#[serde(default)]`, and it is not written out when false), and putting it
in the variant would have rewritten sixty construction and match sites for one bit.

**What it changes.** Three picture paths skip a layer with the flag — the frame key
(`feed_comp`, lumit-eval), the decode plan (`collect_comp_jobs`, lumit-render) and the draw
builder (`build.rs`) — and `comp_footage_items` leaves its file out of the renderer's item
map, so a video placed for its sound alone is never probed or frame-indexed for a picture it
will not show. The audio path looks at the flag nowhere at all, which is the point.

**Solo splits in two.** `any_solo` (every soloed layer) is what the mixer asks;
`any_picture_solo` (soloed layers that draw) is what the compositor, the occlusion cull and
the frame key ask. Soloing a music track means "just this sound"; it cannot sensibly mean an
empty picture, because the track has no picture to show.

**The frame key skips before the switch gate, not after.** This is the ordering that makes
docs/TODO 2.5 true: a layer tested for `visible` first would change the picture's name by
being hidden or shown, so muting a music track would retire every rendered frame of the
comp. Skipped ahead of the gate, none of an Audio layer's switches can reach the key at all.

**The switches column shows only what a layer can do.** An Audio layer is offered no
visibility switch and a layer with no sound — a solid, a title, a shape, image-only footage —
is offered no audible switch. A control that cannot do anything is worse than no control, and
it is the same reasoning that already decides whether the Audio group appears under a layer.

**Placement.** Media with no picture becomes an Audio layer by whichever route placed it, so
the caller does not have to know what the file turned out to hold; media that will not probe
is assumed to have a picture, because a wrongly-flagged Audio layer would silently drop a
picture the user placed. *Add audio only* on a footage item's Project-panel menu is the
deliberate case: the sound of a clip that does have a picture. One `AddLayer` op, one undo.

**A known limit.** A project saved before this entry keeps `audio_only = false` on its
audio-only footage layers, so they still enter the frame key as they always did (with the
stable `audio#` stamp — correct, just not skipped) until the layer is placed again. Nothing
can migrate it: deciding it means probing the media, and the project loader has no decoder.

Regression tests: `an_audio_layers_switches_never_change_the_picture` and
`soloing_an_audio_layer_does_not_blank_the_picture` (lumit-eval),
`audio_only_media_is_omitted_not_slated` (lumit-render `headless.rs`),
`the_sound_of_a_clip_can_be_its_own_layer` (lumit-bridge `api/tests.rs`), "an audio row has
no visibility switch and an image row has no audio switch"
(`timeline_panel_frb_test.dart`) and "Add audio only puts a clip's sound in the open comp"
(`project_panel_frb_test.dart`). New user-facing string: `addAudioOnly`.

## K-436 — A retimed layer's waveform is bucketed through its Retime map

**DECIDED 2026-08-23.** `LayerReference::audio_peaks` takes its window in the **layer's own
clock**, not the source's, and maps each bucket's edges through
`Layer::source_time_at` before asking the pyramid. A layer that has been retimed therefore
draws a wave that stretches, slows and reverses with its map, exactly as a Sequence clip's
already did through `Clip::source_time` (K-280).

**What was wrong.** The lane's mapping was a straight line in x, and so was the fetch: the
Timeline asked for `[view − start_offset, …)` and drew bucket *i* at column *i*. That is
right for every layer playing at speed 1 and wrong for every layer that is not. A layer
slowed to half filled the left half of its bar with the whole of its sound and drew silence
across the right half; a reversed layer drew its wave the wrong way round. The picture and
the sound disagreed about which moment a column stood for, which is the one thing a waveform
in a Timeline exists to tell you.

**Why the layer's clock and not the source's.** Layer time is what a column of the lane *is*
— comp time less the layer's start offset — so the caller already had it and the change
costs the bridge nothing: an un-retimed layer's map is the identity, and its window means
exactly what it meant before. Source time cannot be the unit of the window at all once a map
is in the way, because the window would then have to be non-contiguous.

**The straight path is kept.** Only a layer that *has* a Retime property pays for the
per-bucket walk; an un-retimed one still hands the whole window to `PeakPyramid::range` in
one pass, which is the common case and the fast one.

**Refetching.** A reshaped map changes the answer without moving the window, so the lane's
fetch key carries the document revision **for retimed layers only**. An ordinary layer keys
on the window alone and an edit elsewhere in the document asks the engine for nothing.

Regression tests: "a retimed layer's waveform stretches with the map"
(`timeline_panel_frb_test.dart`), which also pins that the identity map — switching Retime on
without shaping it — changes nothing.

## K-437 — The waveform lane is drawn on the divider, across both of its rows

**DECIDED 2026-08-23.** The Timeline's waveform lane paints at **twice** the row height,
anchored to the bottom of its own row and reaching up through the row above it. A centred
wave then sits on the **divider** between the two; one standing on the floor rises through
both. The row's height is unchanged: only the painting reaches up, so the outline and the
lanes stay level (K-208's rule that the two halves agree on every height is untouched).

**Why there is a row to borrow.** A waveform lane only ever exists under its own **Waveform**
twirl (K-281), and that twirl's row is empty lane space — a label on the outline side and
nothing at all on the lane side. So the pair is already there, and one of them was being
spent on nothing while the wave was squeezed into 22 pixels beside it.

**Why the divider is the right line.** A centred waveform is symmetrical about silence, and
the divider is the strongest horizontal line the lane area draws. Putting silence on it means
the drawing is read against a line that is really there rather than against an invented one
inside a row, and it doubles the amplitude the wave has to spend without moving a single row.
The from-the-bottom mode (K-285) keeps its floor and simply gains the room above it.

Regression tests: "a centred wave given both rows sits on the divider between them", "a wave
from the floor given both rows rises through both" and "with no height given, the wave stays
inside its own box" (`waveform_test.dart` — the last is what keeps a Sequence clip's wave
inside its clip box, which has no empty row to borrow), and "the waveform lane is drawn across
both rows" (`timeline_panel_frb_test.dart`).

## K-438 — Lumit's type and palette stand on their own: Hanken Grotesk and Geist Mono

**DECIDED 2026-08-23.** Two decisions taken together, because the second is what makes the
first possible.

**Lumit's design language is decoupled from the household Aizome system.** K-004 bound Lumit
to a dark-first adaptation of the household design; the dark-first half of that decision
stands — it was made for the Viewer's sake, not the household's — and so do the values worth
keeping on their own merits (token-only colour, hairline elevation, no punishment UI,
sentence case, the voice rules, viewer-surround neutrality). What ends is the upstream: no
household file constrains Lumit's fonts or colours any more, household repaints imply no
change wave here, and 15-DESIGN §4.2's mapping table becomes lineage rather than contract.
This partially supersedes **K-004** (the household binding; dark-first survives) and retires
the household type stack recorded in 15-DESIGN §1.1.

**The type is Hanken Grotesk for UI text and Geist Mono for every number, timecode and
container label.** Both are SIL OFL and both are bundled. There is no third face in chrome —
no display face, no serif accent line. The mono-for-numbers rule is unchanged in substance
and changes only its face: JetBrains Mono's jobs pass to Geist Mono wholesale. Kickers —
9–11px mono caps, +0.08–0.12em tracking, muted — become the style of **every container
label**: panel titles, properties section headers, column headers, tab labels, dialog
titles. Nothing in chrome sits above 13px except dialog body emphasis. 15-DESIGN §7.1
carries the new scale.

Rationale: the redesign study found that what makes the reference tools look disciplined is
not a different palette but fewer, harder rules — and a four-face stack with a serif in it
was the wrong shape for a dense dark tool. Hanken Grotesk was picked from a visual
comparison board; it has no mono of its own, and Geist Mono pairs with it.

## K-439 — Three greys at rest, and the accent's jobs split with a second colour: `animated`

**DECIDED 2026-08-23.** Three rules, one discipline.

**Three greys at rest.** A panel in its resting state shows at most three surface values:
`surface_0` (canvas), `surface_1` (body), `surface_2` (header strip). `surface_3` and
`surface_4` are hover and floating only. **Input wells are inset on `surface_0`** — darker
than the panel, never a raised fill. The five-level ramp of K-084 keeps all five values;
what changes is how many a resting panel is allowed to spend.

**The accent's five jobs become two short lists.** `accent` (clay) keeps **the single
filled button per surface, the playhead, and the active tab tick** (the selection tokens
keep deriving from it). A new token **`animated`** — a desaturated warm amber, placeholder
value `#d8a24a`, tunable — takes "this is animated or in hand": **keyframe diamonds,
stopwatch-on, selected keyframes, selected gizmo handles, the focused value field, and the
work-area band**. That list is closed by construction: **if a third kind of use appears, it
is wrong.**

**Editable values** are mono text in an inset well at rest, `animated` when keyframed,
`accent` while being dragged — the well says "editable", the colour says "keyed", the
accent says "in your hand right now". This resolves the long-standing editable-value-colour
question: colour alone (the After Effects way) would break the Viewer neutrality zone; the
well works everywhere.

Feedback follows the same discipline: transient and local — the drag-scrub modifier ladder
shows only while dragging, drop highlights only over the target, attached things move
during the drag — and **nothing changes the resting state**. 15-DESIGN §2.1, §3.1 and §12A
carry the rules.

## K-440 — Chrome speaks Words by default, Icons by setting, and the icon set is Lumit's own

**DECIDED 2026-08-23.** Supersedes **K-085** (Iconoir): Lumit draws its own icon set, one
glyph per chrome word, on a fixed grammar — **16px grid, 1.5px stroke, round caps, one
weight, monochrome via `currentColor`**. The one glyph with real colour is the Viewer's
Channels indicator, whose circles fill per viewed channel (all three plus a white centre
for RGB; a single circle for R, G or B). A bypassed effect draws as a **dashed outline**
rather than a dimmed row. Iconoir remains in place until the set lands; K-085's standing
rules (monochrome, the emoji ban, CI resolving every icon name) carry over unchanged.

**A three-way Chrome labels setting** decides what chrome speaks: **Words** (default);
**Icons** (buttons, tabs and toggles become glyphs, panel titles stay text); **Icons
everywhere** (panel titles too). **Tooltips always carry the word**, in every mode — and a
tooltip is now **one or two words, never more**, tightening docs/07 §13.2's "under five".
Content the user typed is never iconified. 15-DESIGN §5 and §5.1 carry the grammar.

## K-441 — The Timeline's resting shape: Layers and Graph, the Animated filter, the double-height ruler

**DECIDED 2026-08-23.** The redesign's timeline rules, and the record that **Sharp is the
redesign's reference shape** — everything is designed and judged under Sharp first, and
Round (K-092) is revisited against the finished shell afterwards.

- **Modes are Layers and Graph. There is no Keys mode.** A dope-sheet mode was designed and
  then withdrawn: Layers mode already carries the twirl-down editing, so Keys collapsed
  into an **Animated filter** (`U`) on Layers — show only keyframed properties across all
  layers; All restores the full lists. Block selection, end-handle stretch and the Ease
  popover are Layers behaviours.
- **Composition tabs run the full panel header.** The row above the outline carries the
  timecode and frame count at its far left and the mode tabs at its far right.
- **The ruler is double height**: times and the playhead head above; markers and the work
  area below; the **cache bar beneath, coloured by resolution tier** (which also reframes
  K-214's bar: the storage split folds into the status line's cache meter). A marker is the
  upward triangle sitting on the cache bar, half inside its backdrop pill and half outside
  to its left; minor ticks subdivide with zoom until one tick is one frame.
- **A few pixels of padding either side of the ruler** in every mode, so a first- or
  last-frame handle stays visible and draggable.
- **The work area is one band** in `animated`, from the ruler handles down through the
  lanes, drawn behind the cache bar.
- **Trimmed layers keep a faint outline of the full source extent** (carried over from
  today's behaviour); Sequence clips get the same per clip. **Layer bars, and the clips
  inside a Sequence layer, fill desaturated with a solid leading edge.** **Keyframe
  diamonds on layer rows draw at half row scale.**

15-DESIGN §12A.1 is the spec; the approved mockups govern the exact layout.

## K-442 — Graph mode is the filtered animated list beside curves that run edge to edge

**DECIDED 2026-08-23.** Graph mode keeps the same double-height ruler, work area, cache bar
and playhead as Layers mode, so both modes scroll the same range. Its outline is the
filtered animated list — Animated (default) or All, no Selected — with a **tick in each
curve's colour** per row; a setting restores an outline identical to Layers mode. **Curves
run flat to both edges of the visible area; value labels live in a fixed right-hand
gutter.** The thin horizontal scrollbar and the Value / Speed tool strip (tangent mode,
ease presets, Fit, zoom) exist only along the bottom of the graph side, never in the
outline. Bezier, linear and hold are drawn distinctly. 15-DESIGN §12A.2 carries it.

## K-443 — Properties rows lay out on fixed column edges, behind a square stopwatch

**DECIDED 2026-08-23.** Every properties and effect-controls row shares the same x
positions: **stopwatch** (square under Sharp), then a **reserved keyframe-navigation slot
that stays empty until the property is animated** — so the label never moves when animation
begins — then the label, then the control column at a fixed x. Rows that cannot be keyed
omit the stopwatch: a ragged stopwatch column and a dead-straight label edge. **Vector
pairs are two equal wells with a link glyph between them**; unlinking splits them into two
rows. **Units are plain mono, never caps.** **Position-type parameters get a crosshair
point picker** (pick the point on the Viewer) exactly as colour parameters have an
eyedropper. The per-effect **Mix row gains blend mode and matte channel**, and the **Matte
row carries an invert**. 15-DESIGN §12A.3 carries it.

## K-444 — Every dialog is one pattern, and (post multi-window) a real OS window

**DECIDED 2026-08-23.** Two halves:

**The pattern.** Every dialog shares one shape: a kicker title strip; an optional page-tab
row (Export: General / Encoder / Colour / Metadata); label-left / control-right rows with
labels in a fixed 110px column; hairline-separated, kicker-titled groups; and a footer
strip carrying a factual mono summary line and **the single filled action**. 15-DESIGN
§12A.4 carries it.

**The ordering.** The Flutter multi-window upgrade lands **first, as its own phase**,
before the redesigned shell — so dialogs are rebuilt once, as real OS windows on the new
foundation, and upgrade regressions stay separable from redesign regressions. Until that
phase lands the pattern applies inside the window; after it, a popup is a window.

## K-445 — The node graph is a second view of the effect stack that can also wire effects together

**DECIDED 2026-08-23.** Lumit gets a node graph — as **both** a second view of a layer's
effect stack **and** a way to wire effects into each other, not as a replacement document
model. The linear stack stays the default and serialises as today; graph extras (extra
wires, helper nodes) are additive. **Whether the wiring ultimately needs a new document
model is deliberately open** — the graph design step decides, and a lot of work is expected
to make graph mode good; it is budgeted as its own redesign phase, after the panels. The
surface: a Graph panel following the selected layer, and a **Nodes workspace** that makes
the graph the main surface with a small viewer alongside and a short timeline beneath.
Auto-wire (connect a new node as it lands) and Heal (reconnect neighbours on delete) are
toggles. Wire and socket colour is the data type, from `viz_*`-family tokens under the
no-hex rule — colour as the legend.

## K-446 — Particulate, renamed from Particle, emits a points stream

**DECIDED 2026-08-23.** The particle system is named **Particulate** and stays a separate
effect — but its output is a **points stream**, a first-class data type rather than pixels
straight to the composite. The same type is what a later grid / scatter / clone-to-points /
connect-points family will share, so Particulate is the first consumer of a points pipeline
rather than a dead end: separate effect, shared type. Nothing about its feature set is
decided here beyond the output's shape.

## K-447 — There is no auto-key: the stopwatch is the whole model

**DECIDED 2026-08-23.** Lumit ships no auto-key. The stopwatch model already carries the
meaning an auto-key mode exists to patch: a property animates exactly while its stopwatch
is lit — from then on every edit at the playhead is a keyframe — and an edit to an unlit
property sets its constant value. This settles the open auto-key semantic question by
removing its subject: there is no record mode to leave on by accident, no "temporary value
that reverts when the playhead moves" state to explain, and whether an edit keys is always
visible on the row that takes it. docs/07 §4.3 carries it.

## K-448 — The remaining approved shell rules from the mockup rounds

**DECIDED 2026-08-23.** The redesign's approval rounds settled a set of smaller shell
rules, recorded together here; each lives in the spec noted.

- **The transport lives in the Viewer bar.** The bar's items may split between a top and a
  bottom bar by default; a setting gathers everything into one bar, at the top or the
  bottom (15-DESIGN §12A.6, docs/07 §2.2).
- **A setting hides shortcut hints throughout the UI**, the main menu excluded (docs/07
  settings inventory).
- **The Timeline's bottom bar carries a toggle for the switches / modes / parent columns**
  (15-DESIGN §12A.1).
- **No toolbar swatches**: project colours appear inside the colour picker, never as a
  swatch strip in the toolbar (docs/07 §6.1).
- **The Node preview is its own panel**, openable in a sidebar of the Effects workspace —
  beside, not replacing, the Nodes workspace's small viewer (K-445).
- **First click selects, always**: selection lands immediately on the first click, and a
  double-click's action stacks on top of that selection (docs/07 §14).
- **macOS keeps its native menu bar**, so the wordmark and the workspace tabs move into
  the toolbar row (docs/07 §1.8).
- **The Settings section over the Viewer's own toggles is titled "Viewer"**, not "Viewer
  chrome".
- **The welcome screen** carries the New, Blank and Open project cards, with Manual and
  What's new as outlined buttons, and no "free and open source" line; with nothing open,
  the same three cards repeat in the empty Viewer until a composition is viewed (docs/07
  §13.2).
- **The Project panel gains a bottom bar with Folder and Composition** (dropping an asset
  on Composition makes a comp to match); colour tags tint the item icon's strokes rather
  than adding a dot; the path column sits at the right of the list; the preview card
  carries name, size, rate, duration and codec (docs/07 §3.1).
- **Paired dialog buttons share one width** (15-DESIGN §12A.4).

## K-449 — Multi-window waits for Flutter; the redesign does not

**DECIDED 2026-08-24.** Supersedes the **ordering** half of K-444; the dialog pattern
itself stands unchanged. Flutter's windowing support is not shippable: it exists only on
the main channel behind `--enable-windowing`, its API is annotated internal with breaking
changes promised even in patch versions, and no stable release carries it
(docs/impl/multi-window.md §1). Lumit pins stable Flutter, so the redesign cannot wait on
it and must not take a production dependency on it.

The redesign therefore runs in these phases: **1** the theme tokens, the bundled fonts and
the icon set; **2** panels and windows, with dialogs built **in-window** in the K-444
pattern using ordinary framework dialogs (`showDialog`) — not a stopgap but the documented
migration prep, since with windowing enabled `showDialog` becomes a real child dialog
window with no per-dialog rewrite; **3** the node graph and the Nodes workspace; **4** the
website.

**Multi-window becomes its own later phase**, gated on Flutter stabilising the API on the
stable channel un-flagged, and opened by the Viewer-texture spike in
docs/impl/multi-window.md §6 — whether a second window can composite the engine's shared
texture is the one Lumit-specific risk no release note answers, and it decides whether
multi-monitor Viewers can ever be promised.

## K-450 — The filled action's label is a kicker; the secondary button is outlined

**DECIDED 2026-08-24.** Amends the phrasing of K-439 and 15-DESIGN §2.3 where the approved
mockups already differ; the three-greys rule and the borderless grammar otherwise stand.

- **The single filled action's label is a kicker** — 10px Geist Mono caps, +0.08–0.12em,
  in `surface_0` on the `accent` fill (15-DESIGN §7.1's new row). It is the one place a
  kicker's colour is decided by the fill under it rather than being `text_muted`.
- **The secondary button rests as a `hairline_strong` outline**, a deliberate exception to
  the borderless-widget rule (§2.3) for buttons only: a dialog's non-default action has to
  be findable beside the filled one without spending a fourth grey on it.

## K-451 — Mockup heights are canonical, and width gives way in a fixed order

**DECIDED 2026-08-24.** The approved mockups' vertical metrics are binding: chrome is
built to the table in `docs/15-DESIGN.md` §12A.6 (22px header strips, 18px secondary
rows, 22px outline rows, 26px property rows, 36px ruler, and the rest), and vertical
metrics never compress — short panels scroll. When width runs out, the degradation
ladder in §12A.6 applies in order: flexible text ellipsises, secondary control runs
wrap, optional metadata columns hide, toolbars overflow into a menu, and below a
panel's declared minimum width the panel scrolls horizontally. Values and units never
truncate, and the ladder never flips the user's own column toggles.

## K-452 — The hit-target floor bends inside 18px secondary rows

**DECIDED 2026-08-24.** KD-2/K-116's ≥32px floor cannot hold for buttons seated in the
18px secondary rows (timecode/search/mode row, column headers, bottom bars): expanding
their hit areas vertically would swallow the neighbouring strip's gestures (tab drag
above, header resize and reorder below). Those buttons keep their full room along the
axis the row is aimed on, and the floor continues to govern everything outside such
rows. The reasoning also lives where `_toolbarHeight` is declared.

## K-453 — Both switch columns are pinned; the cache bar rides the ruler's floor

**DECIDED 2026-08-24.** Three resting-state corrections, taken against the approved
mockup at matched zoom, extending K-448 and K-451 rather than reversing them.

**Modes joins Switches as a fixed-width column** (owner): it is five icon cells and
nothing else, so it is exactly `5 × switchCellWidth`, its seam is not a drag handle, and
it can no longer be widened. The Timeline fold-out's value cells still align to that
span (docs/07 §4.3) — they are as wide as the column they sit under, which is the point
of aligning them to it at all. The old 150 existed only to give those fields room, and
paid for it with a blank third of a column in every row.

**The cache bar is drawn inside the ruler's 36**, on the work-area band's row at the
ruler's floor, with the band painting behind it and the marker flags standing on it —
not as a 3px strip of its own beneath the ruler, which cut the band short of the lanes
at exactly the place the eye follows it across.

**The workspace tabs are mono-caps kickers with an accent underline** under the one in
force, which is what §3.1's "active tab tick" has always meant. They had drifted to
sentence-case words tinted in the accent: that spends the accent on text and leaves the
strip with no tick at all. Round keeps its filled pill (K-394) and needs no tick under a
fill. The Timeline's snap magnet loses its accent for the same reason — a tool toggle is
not on §3.1's closed list, and it now reads as foreground on the button's own face.

## K-454 — Two row densities: the mockups' room by default, Compact as a setting

**DECIDED 2026-08-24.** The owner's ruling on density: the redesign's default row and
bar heights follow the approved mockups' *effective* rendered heights — content plus
the borders and gaps the mockup actually paints, rounded toward the roomier reading —
and a **Compact** toggle in the settings houses the slimmer variants for users who want
more visible at once. Where a K-451 metric reads tighter than the mockup renders, the
mockup's value becomes the Regular default and the tighter value moves into Compact;
§12A.6's table gains the second column as the measurements land. Supersedes K-451 only
in that sense — the table's authority and its degradation ladder stand.

## K-455 — Keys returns as a third Timeline mode

**DECIDED 2026-08-24.** Supersedes K-441's first bullet only. The owner's ruling: the
dope sheet earns its place after all — the closer look at the approved Keys drawing
showed a genuinely different reading of a comp's keyframes, not a subset of Layers.
The Timeline's modes are **Layers, Keys and Graph**. Layers keeps everything it
absorbed when Keys was withdrawn — the Animated filter (`U`), twirl-down lane editing,
block selection, end-handle stretch, the Ease popover — because that is where
After Effects hands land. Keys is an additional way of seeing, built to the approved
Keys drawing; it adds no editing behaviour Layers lacks. K-441's other bullets stand.

## K-456 — Icons display at the mockups' sizes; the 16 grid is the drawing grid, not the law

**DECIDED 2026-08-24.** Supersedes K-209's fixed 16px rendering where a mockup renders
smaller. The owner's ruling, with the general note that any earlier design decision may
be overruled by the redesign's drawings. Glyphs stay drawn on the 16 grid with the
1.5-unit stroke; each panel renders them at the size its mockup's computed-style
manifest records (the Project panel: 13px in rows, 14px in the footer). Slight stroke
softening at non-native sizes is accepted as the mockups' own look.

## K-457 — A keyframe's shape says its interpolation, half by half

**DECIDED 2026-08-24.** In the Timeline's lanes (and the Keys mode), a keyframe's mark
depends on interpolation: **linear = the diamond, bezier = an hourglass** (two triangles
tip to tip), **held = a square** — all at one height. The mark is split at its vertical
centre: the left half draws the incoming interpolation's shape, the right half the
outgoing, so a key that eases in and holds out reads as half hourglass, half square.
Colours unchanged (animated; selected brightens as today).

## K-458 — The drawing is authoritative; Keys mode carries everything its drawing shows

**DECIDED 2026-08-24.** Two rulings from the owner. First, the general one, sharpening
K-450: **an approved mockup is authoritative over every earlier decision and every
scope argument — unless the owner personally says otherwise.** "Adds no new behaviour"
is never a reason to leave out something a drawing shows; the drawings are the plan.
Second, the application: K-455's restriction ("Keys adds no editing behaviour Layers
lacks") is superseded. Keys mode carries the whole of its drawing: the block-selection
box with stretch handles and its `n keys · n f` badge, the Ease popover, and the bottom
bar's Interpolation / Reverse / Copy / Paste-at-playhead strip. Where those tools also
belong in Layers per K-441's own bullet, the shared machinery serves both; where the
engine lacks an op (reverse, paste at playhead), the op is built with it, per the
standing build-the-bridge-parts rule.

## K-459 — A lane key is the same size in Layers mode as in Keys

**DECIDED 2026-08-24.** The owner's ruling, superseding K-451's "the 8px one a property's
own lane draws" in §12A.6. A keyframe on a property's lane measures **11px point to
point in both modes** — the size the approved drawings give it, and the size Keys mode
already drew. Layers mode drew it at 8, which made the same key change size as the panel
switched view, and made the mark harder to aim at in the mode where it is most often
dragged. **A shut layer's summary diamonds keep their smaller scale**: they are a
statement that something is keyed inside, not a thing to take hold of.

The shapes go with the size. K-457's marks are drawn by the one painter both modes share,
so Layers mode now says a key's interpolation too, and `keyShapeOf` answers with the
**pair** of shapes a split mark needs rather than folding a key's two sides into one
verdict. The bezier side is the **hourglass**, which supersedes the rounded shape the
Keys mode landed with. Nothing new crosses the bridge: `BridgeKeyframe` has carried
`interp_in` and `interp_out` since the graph editor could edit them, so this is exposure
of a seam that was already there.

## K-460 — The Timeline's two clock readouts sit in wells, and the frame count edits bare

**DECIDED 2026-08-24.** The owner's ruling, extending §2.1's value well to the timecode
row. Both readouts above the outline — the timecode and the frame count — draw as **value
wells**: the inset `surface_0` face inside a hairline that every editable number in the
editor wears at rest. They have been typeable since K-287, but they looked like text, and
a number that looks like text is a number nobody clicks; the recess is the whole of the
invitation and it costs no colour to make. Hover firms the edge and the open editor takes
`animated`, exactly as a property's well does — neither ever lifts out of its recess.

**The frame count drops its `f` while it is being edited.** At rest it reads `f48`; the
field it opens holds `48`, and the letter is worn again the moment the edit commits. The
`f` names which clock this is rather than counting in it, so an edit that began by
stepping over a letter began wrong. `/ 250` stays outside the well: the comp's length is
not edited there, and a recess round it would say it was. Commit on Enter, Escape reverts
(§12A.3).

## K-461 — The outline's identity cluster: the mockup's order, its room, and columns cut to their content

**DECIDED 2026-08-24.** The owner's ruling on the Timeline outline, superseding §12A.1's
"a layer number column sits between the label dot and the name" (K-441/K-451) and the
compose column defaults that shipped with it.

- **The label dot and the layer number change places**: the cluster reads **twirl ·
  number · dot · name**, as the approved Main drawing has it. The number is the row's
  address; the dot belongs to the name it colours.
- **The cluster's gaps are the drawing's 8**, between the twirl and the number and
  between the number and the dot. They had been 0 and 4 — the three smallest marks in
  the panel packed into the one place that most needs air. The dot keeps its 16px cell
  as a hit target (K-452), whose inset is also the drawing's dot-to-name gap.
- **Matte, blend and parent start at their content's width**: the drawing's 84 / 84 / 64
  dropdown faces plus the group's normal gaps, with the matte cell carrying its two mode
  toggles on top of its 84. They were 118 / 112 / 96, which bought each picker slack it
  never used. Only the defaults move: every seam still drags, and a project with long
  layer names widens the group once and keeps it.
- **The Timeline's type is the drawing's throughout**, re-audited row by row: 11px mono
  for the timecode, 10 for the frame count, the layer number and the picker faces, 11 for
  every name, 9 for the kickers over the columns and in the bottom bar. Two numbers had
  drifted and are corrected — the Retime clock, at `mono`'s bare 12 rather than the value
  well's 11, and the render-time readout at a kicker's 9 rather than a value's 10.

## K-462 — One gap in an outline row, bare switches, and three measurements taken off the drawing

**DECIDED 2026-08-24.** The owner's polish pass on the Timeline outline, against the
approved Main drawing. It supersedes parts of §12A.1 and §12A.6 recorded under K-451,
K-454 and K-461, and settles a colour ruling the owner has now given more than once.

- **One gap, everywhere in an outline row: 8.** The drawing lays a layer's row out as a
  single line with one even space between every item on it and 8 of padding at either
  end. The outline had used three different numbers — 8 inside the identity cluster
  (K-461), 4 between the compose pickers, 7 for the seam between two clusters — plus a
  stray 4 tucked behind the layer name. They are now one number, which supersedes K-461's
  wording that made the 8 a property of the identity cluster alone. Inside a switch
  cluster the drawing is tighter, at 6, and so is the outline: its cells stand the glyphs
  6 apart while the whole cell stays the click target (§7.2).
- **The dropdown faces are 84 / 84 / 64, the matte's included.** K-461 cut the compose
  *columns* to the drawing's widths but let the matte's face swell to the whole column
  whenever no matte was set, on the reasoning that empty room beside a picker reads as a
  gap. It reads worse as a third dropdown width in a row that draws two: the room beyond
  84 in that column belongs to the two mode toggles whether they are drawn or not.
- **A row switch is a bare glyph.** No boxed or outlined face on the eye, solo, lock, shy,
  or on any mode switch, anywhere in the outline — the drawing puts one on none of them,
  and ten boxed marks at the head of a row turned two quiet columns into a grid of
  buttons. This supersedes the "small outlined box, so the click targets read as buttons"
  that shipped with K-441. The cell keeps its width, so nothing about the aiming changes.
- **On is `text_primary`, off is `text_muted`, and the Modes column takes no accent.**
  Two strengths, the drawing's own, replacing the third (`text_disabled`) that switches
  whose glyph does not flip used to rest at: with the faces gone the colour is the whole
  of the state, and two readings a person can tell apart beat three that shade into one
  another. Never the accent (§3.1's list is closed) and never `animated`, which means
  "this is keyed" and has no business on a motion-blur switch.
- **A shut layer's summary keyframe mark is 8px point to point**, not the 5 the outline
  drew. §12A.6 had read the drawing's mark as a 4px square, ≈5.7 across its diagonal; the
  square carries a 1px border and stands on its corner, so it renders 8×8. The measured
  rendering wins over the arithmetic done on the declared side.
- **The layer-search well is 16 tall**, in a secondary row of 19. It had sized itself to
  its own 16px glyph plus its hairline, come out at 18, and filled the row edge to edge;
  the drawing keeps ground above and below, which is what makes it read as a well sitting
  in a row rather than as the row itself. Measured in the same pass: the in-row pickers
  already stand at the drawing's 18 under Regular and 16 under Compact, and did not move.
- **A fold-out row keeps the same trailing space as a layer row** (8 of padding plus the 2
  the outline spends on the column header's wider left inset). It had kept a bare 4 — the
  layer rows' own padding before K-441 widened it and before K-454 added the 2 — so every
  value cell and every effect heading's millisecond reading stood 6px right of the column
  it belongs to. This is a bug the pinned test had been failing on, not a new rule.

## K-463 — The matte column carries its toggles' room only while a matte is set

**DECIDED 2026-08-24.** The owner, on the outline as it stands: "there's now a big gap
between matte and blend, can we reduce that to the normal padding width again."

K-462 fixed the matte dropdown's face at the drawing's 84 while leaving the column 28
wider, on the reasoning that the two mode toggles' room is theirs whether or not they are
drawn — so the blend column never shifts as mattes come and go. That rationale is
superseded here. Most comps have no matte anywhere in them, and on every one of their rows
the reserved room read as a hole between the matte face and the blend: a 28px gap in a row
whose every other gap is 8 (K-462).

- **The compose group is 84 + 8 + 84 + 8 + 64 at rest**, and the gap between the matte face
  and the blend column is the row's own [outlineGap] of 8, like every other gap in it.
- **The toggles' room appears with the first matte in the comp** and goes when the last one
  goes: while any *visible* row has a matte set, the compose group is that much wider and
  every row of the outline reserves the slot — including the rows with no matte, so the
  blend column stays a column down the whole stack. The blend column shifting once, when
  that first matte is set, is the accepted price of the eight-pixel rhythm the rest of the
  time.
- **The face is still 84 either way.** K-462's other half stands: a dropdown never swells
  into room it has not been given, which would put a third dropdown width in a row that
  draws two.
- The width is derived in Dart from the layer list the outline already holds — no engine
  call in a rebuild path — and the header and the rows are handed the same answer, so they
  cannot disagree about where a column starts. The seam still drags, and a dragged width
  keeps the same behaviour: the toggles' room is added on top of whatever the group is at.

## K-464 — The welcome screen, built to its drawing, with a per-row forget

**DECIDED 2026-08-24.** Fills in K-448's one-line welcome rule from the approved drawing,
and records the owner's one addition on top of it. Nothing here reverses an earlier entry.

- **The welcome is the window after the boot splash**, not a card over a half-built
  shell — the same handover the splash already makes (K-008). It stays in-window: a
  welcome *window* is the multi-window phase's (K-449), and nothing waits on that.
- **It is not put to somebody who has already answered it.** A `.lum` on the command
  line — a double-clicked project — goes straight to the shell.
- **One centred 560px column, four blocks 28 apart**: the wordmark in mono at 22, three
  180×63 start cards 10 apart (New project / Blank project / Open), the recents list, and
  a footer carrying the version and two outlined links (Manual, What's new).
- **No filled action anywhere on it.** §3.1's accent rule is a ceiling of one, not a
  floor, and the drawing spends none of it here.
- **A recent row is name, path, format, date and a forget button**, in a 40px row inside a
  hairline well. The format column (`1920×1080 · 25`) is drawn but **empty**: the size and
  rate live inside the `.lum`, and the engine has no way to read them without opening the
  file. Dart does not guess one — the seam is listed in docs/TODO.md and the column keeps
  its room until the engine can answer.
- **Recents carry a last-opened stamp** in the workspace store, written when a project is
  adopted. It is this machine's record of the user's own work, not the file's timestamp:
  a `stat` per row would make opening the screen wait on a network drive.
- **The list is emptied by Clear and thinned by a per-row ×** (owner, beyond the drawing).
  The × is the composition tabs' close mark — muted, no box, brightening under the
  pointer — and it is the innermost hit on the row, so pressing it never also opens the
  project. Its room comes out of the flexible name column, which is step 1 of §12A.6's
  ladder.
- **Neither asks first.** The one destructive control that asks is the disk cache
  (`shell/cache_confirm_frb.dart`), because that one throws away a night's rendering with
  nothing to undo. Forgetting a path deletes nothing and File ▸ Open brings it back.

## K-465 — Settings is rebuilt to its drawing: nine pages named, six built, rows of 30

**DECIDED 2026-08-24.** The approved Settings drawing is authoritative over K-193's five
pages and over §12A.4's dialog row as it was written (K-458's standing rule). Nothing else
here reverses an earlier entry.

- **The window is the drawing's frame**: 760×520 inside a hairline, a 30px kicker title
  strip carrying a search well and a close mark, a 160px sidebar of pages, the page beside
  it, and a 43px footer saying *Changes apply immediately* with **Reset page** and
  **Close** at its far end. No filled action: §3.1's one-filled-button rule is a ceiling,
  and the drawing spends none of it here. The accent is spent once, on the 2px tick beside
  the page in force.
- **A page is sections, and a section is a kicker over rows.** No cards: the drawing
  separates one section from the next with a rule and 6px of air, which is what
  `settingsSection` now draws. Project settings (K-286) shares the shape, as it always has.
- **A row is 30, its label in a fixed 190px column, and its control **beside** the label
  rather than at the right edge.** This corrects §12A.4's "labels in a fixed 110px column"
  and its right-aligned control: 190 and left is what the drawing computes, twice over.
- **The help sentence under each setting is gone.** The drawing has no room for one, and a
  setting whose name does not say what it does is a naming problem. A row may still carry a
  line *under* it, at full width, when it has something live to report — where the parked
  frames are, what the last update check found. The `settingsHelp*` strings those rows used
  are left in the arb rather than deleted, so nothing that still wants one has to be
  re-translated to get it back.
- **The pages are General, Appearance, Timeline, Viewer, Preview and cache, Shortcuts.**
  The drawing lists three more — **Audio, Autosave, Export** — and they are *not built*:
  there is no audio device setting, no autosave interval and no export default in the
  engine to put on them, and an empty page is a promise the window cannot keep (K-193's
  rule, which the drawing does not contradict — it draws the destination, not the stock).
  Each arrives with the settings it would hold.
- **A boolean in this window is the drawing's pill switch**, `HouseToggle`: 22×12, on in
  `animated`, off in `hairline_strong`. Not the accent — a page of switches would spend
  §3.1's accent a dozen times over — and not the 9px checkbox (K-450), which stays the
  mark for a checkbox in a panel. Where the drawing puts a dropdown on what was a
  checkbox, the setting keeps its two values and gains the two words the drawing gives
  them (Surround: Neutral / Theme colour; waveform Style: Frequency / Plain; Anchor:
  Centre / Bottom; Tooltips: Short / Off).
- **The title strip's search is the whole window's.** On every page but Shortcuts it hides
  the rows whose names do not match; on Shortcuts it is the keymap query the engine
  filters on, which is where that page's own search box went.
- **Reset page** puts the page being shown back to what Lumit ships. Preview and cache is
  the exception it can only half keep: the cache budgets are the engine's own defaults and
  it offers no way to ask for them back, so that page resets the playback mode and the
  quality tier and leaves the budgets alone. Nothing asks first — nothing here is
  destructive, and the disk cache's own confirmation (K-193) still is.
- **Five one-click accents** (`LumitTheme.accentPresets`), the same five in every scheme,
  with the hex of whatever the accent actually is beside them. The theme editor still holds
  the whole wheel; these are the quick answers.
- **Three drawn rows have nothing behind them and are not faked**: *Viewer bar* (K-448's
  split/top/bottom arrangement, which no setting stores yet), *Show shortcut hints* (no
  menu or tooltip reads such a flag), and *Editable values* (the well's own look, which
  §3.1 fixes rather than offers). They land with the behaviour they would switch.
- **Chrome labels (K-440) is confirmed as this window's**, on the Appearance page's
  Interface section, and is not drawn there yet because the three-way setting has nothing
  to change: the icon set is embedded but no surface reads a labels mode. It arrives with
  Phase 1's own work, in the place this entry names.

## K-466 — The Viewer is rebuilt to its drawing: two strips, a reading, and a chip on the picture

**DECIDED 2026-08-24.** The Viewer and its bars, rebuilt from the approved Main drawing
under K-458's rule that a drawing is authoritative over an earlier decision. It supersedes
**K-411** (the single bar "arranged in instruments") outright and amends K-448, K-416 and
K-314 in the places named below; docs/07 §2.2 and 15-DESIGN §12A.6 are corrected in the
same commit.

- **The Viewer wears a header strip of its own, 22 tall.** It docks as a pane rather than
  as a tab, so the dock drew no header above it and the one panel whose drawing shows a
  title had none. The strip carries the panel's kicker and, at its right-hand end, the
  three pickers the drawing puts there: the **magnification**, the **preview quality** and
  the **colour pipeline**, each a `.dd` of 18 with a 10px label, 6 apart. This is K-448's
  "items may split between a top and a bottom bar" settled: the split is the drawing's.
- **The bottom bar is the drawing's, in the icon set's own order.** Left to right: the
  transparency board, the view menu, the channel, the exposure; a hairline seam; the
  snapshot. Then the transport's five marks and the clock, and at the right-hand end the
  composition's reading. Every glyph is **14** (K-456's rule, the drawing's measurement),
  the gaps are **8** in the left cluster and **10** in the transport, and the strip is
  padded 10 either end.
- **The transport's steps get glyphs.** They had been the characters `|◀ ◀ ▶ ▶|` in the
  body face; the icon set has carried To start, Previous frame, Next frame and To end
  since the icon pass, and the drawing draws them. They are 14 like every other mark on
  the bar, which supersedes the "the transport is the one place the spec asks for 20".
- **One reading replaces two badges.** `Opening titles · 00:00:01:23 · 1920×1080 →
  960×540 · 47%` states the composition, the time, the pixels the engine actually made and
  the magnification. It absorbs the degradation badge (§2.2 item 9), which used to appear
  and disappear mid-playback and drag the bar about: the tier is the second pixel count,
  stated always and in the place a person already looks.
- **The colour-management badge becomes the colour-pipeline picker**, and the tone map
  moves into its menu. The badge still says `Linear → sRGB`, and still says `· preview` in
  the accent while the exposure or the tone map is engaged (§2.2 item 8). K-314's rule that
  the tone map is off the bar unless Settings asks for it is kept, and now gates a menu row
  rather than a button.
- **The quality picker carries the playback mode.** Preview resolution and adaptive-vs-
  every-frame are one question — how much quality this preview may spend — and the drawing
  has one seat for it. The resolution rows are no longer disabled while playback is
  adaptive: the two answers now sit in the same menu, where the relationship is visible
  instead of being a control that refuses.
- **The view menu absorbs everything the drawing gives no seat.** The drawing has one
  overlay glyph, and §2.2 item 5 has always owed a wireframe/overlay menu; it now carries
  the grid, the safe areas, the **layer controls** (K-217), the **region of interest**
  (K-362) and the composition's **background** (K-357, with the colour beside its row).
- **Snapshot and compare are one mark, two gestures** (amending K-416's pair of buttons):
  a click photographs the picture, a press and hold puts the photograph back over it. The
  drawing draws one glyph, and each gesture does exactly one thing — a press released
  before the hold delay never flashes a comparison, and one held past it never takes a
  second photograph.
- **The exposure is bare.** The drawing sets it as the number alone, 10px mono in
  `text_secondary`, with no aperture beside it and no well under it — the one editable
  value in the application that does not rest in an inset, because a 20px well in a 22px
  bar reads as the bar's own edge. `DragValueField` gains a `bare` face for it; the scrub,
  the modifier ladder, click-to-type and the context menu are untouched.
- **The selection's name is drawn on the picture**, 16 in from the stage's left and 8 down
  from its top: 9px mono, tracked 0.08em, in `animated` inside an `animated` hairline. It
  is the drawing's TITLE chip and the "viewer tag" the graph study asked for — selection is
  agreed in four places and the Viewer was the one that only showed it as a box.
- **The selection box, its handles and its anchor mark are `animated`, not `accent`.**
  §3.1's closed list already names "selected gizmo handles" for that colour, §3.2 bans the
  accent inside the Viewer's neutrality zone, and the drawing draws both in the amber. This
  is a correction, not a new rule.
