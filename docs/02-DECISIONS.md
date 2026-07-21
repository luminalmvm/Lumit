# Kiriko decision log

**Status: canonical.** Numbered, append-only. Every entry is either **DECIDED** (locked by
the project owner) or **PROPOSED** (a strong default chosen during the July 2026 design sessions; veto by
editing the entry and noting why). Reversing a DECIDED entry requires a new entry that
supersedes it — never edit history.

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
the command palette, every setting individually changeable. Added 2026-07-12 at Mack's
request; post-v1 polish. Spec: [07-UI-SPEC.md](07-UI-SPEC.md) §13.1.

## Core model

**K-007 · DECIDED · Docs stay owner-readable; regression coverage is near-full.** All
documentation must remain understandable to the project owner (expert editor, new to Rust
and systems concepts): [GUIDE.md](GUIDE.md) is the plain-English companion, updated in the
same commit as any new concept. Testing policy: every feature ships with tests, every bug
fix ships with a regression test, CI enforces fmt/clippy/tests on macOS + Windows plus an
engine-crate coverage gate whose threshold may rise but never fall, and a design-token
lint. Added 2026-07-13 at Mack's request. Spec: [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md).

**K-008 · DECIDED · Brand mark and boot splash.** The mark is an Edo-kiriko faceted glass
hexagon whose clay facets form a K (assets/brand/; construction and colour constants in
[15-DESIGN.md](15-DESIGN.md) §brand). Boot shows a small centred splash listing each module
and effect as it initialises (the boot log — real registry plumbing that grows with the
effect suite and OFX scanning), minimum ~1 s dwell, failure lines in kraft. Added
2026-07-13 at Mack's request.

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
engine may assume DX12-only. Added 2026-07-13 at Mack's request.

**K-035 · DECIDED · Every effect gets a built-in strength matte.** Any effect instance can
select a per-pixel strength source — the layer's own masks or any other layer (same
dropdown model as layer mattes) — scaling the effect's influence at each pixel. The host
implements it once, uniformly: for colour-type effects as a per-pixel mix between input
and effected image; for warp/distort-type effects by scaling the displacement field where
the effect declares vector output (falling back to output-mix otherwise). No effect
author writes masking code; it composes with everything. AE needs per-effect "composite
on original"/precomp workarounds for this. Lands with the effect suite (phase 3). Added
2026-07-13 at Mack's request. Spec: [08-EFFECTS.md](08-EFFECTS.md) §effect model.

**K-036 · DECIDED · A node view is a planned lens over the evaluation graph.** Kiriko's
layer stack already compiles to a DAG (K-015), so a Nuke-style node editor is a *view*,
not a second engine: post-parity (phase 6 alongside the 3D ambitions), Kiriko exposes the
graph for node-based compositing, starting where nodes earn their keep first — a
Resolve-style grading node chain in the Colour workspace. Layers and nodes stay two lenses
on one document; neither is a mode you convert into. Added 2026-07-13 at Mack's request.

**K-037 · DECIDED · Share export: size-targeted clips for the community workflow.**
Editors share previews (usually Discord, 50 MB free-tier cap): a one-click export mode
takes the active playback area (work area; whole comp until it exists), computes the
bitrate from the size budget ((target bytes × 8 ÷ duration) less audio/container
overhead), optionally caps resolution, and writes a compressed H.264 clip. Presets:
Discord 50 MB (default), 10 MB, custom size, plus a quality-first slider for people who
prefer choosing compression over size. Added 2026-07-13 at Mack's request. Spec:
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
runs in ([08-EFFECTS.md](08-EFFECTS.md)). Added 2026-07-13 at Mack's request. Spec:
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §3.

**K-031 · DECIDED · Colour spaces are selectable; preview always matches export.** Working
colour space is selectable per comp (with app-level defaults, and OCIO joining post-v1 per
06), like AE — but with a hard parity guarantee: **what the Viewer shows at Full resolution
and full quality is bit-identical to what export produces** through the same transforms.
Export-only settings (encoder, bitrate, container, subsampling to 8/10-bit) sit strictly
after the parity point. Adaptive degradation and Realtime mode affect interaction only and
are always visibly indicated. Added 2026-07-12 at Mack's request. Spec:
[06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §3.

**K-032 · DECIDED · Resource and export controls are explicit settings.** RAM/VRAM budgets,
CUDA on/off, decoder pool, worker caps, cache root/size in Settings → Performance/Cache;
export dialogue exposes full custom controls (resolution, frame rate, format, codec,
encoder choice, rate control, audio, thread count and a background/balanced/fast priority)
alongside presets — and exporting never blocks editing (06 §7.1). Added 2026-07-12 at
Mack's request. Spec: [07-UI-SPEC.md](07-UI-SPEC.md) §Settings inventory.

**K-030 · DECIDED · Two preview modes: Cached (default) and Realtime-adaptive.** Cached
plays at full chosen quality from the render-ahead ring and cache. Realtime never waits:
every frame renders live at whatever resolution tier sustains the comp frame rate, adjusted
continuously with hysteresis — judge motion now at reduced resolution rather than full
quality after a wait. Clarified same day: the mode toggle is a **separate control** from
the Viewer bar's resolution picker (Full/Half/Third/Quarter/Auto) — it lives in the
transport and Settings → Preview, never in the resolution dropdown, and Cached always
honours the picked resolution. Added 2026-07-12 at Mack's request. Spec:
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
Mack's request. Spec: [12-PLUGINS.md](12-PLUGINS.md) §2.3, §3.3–3.4.

**K-067 · DECIDED · The engine's pillars carry Edo-kiriko craft names.** The render
pipeline as a whole — evaluation graph, GPU compositor, colour engine — is **Togi**
(研ぎ, the polishing stage that turns cut glass brilliant: it turns the project's cuts
into the picture). The three-tier cache is **Kura** (蔵, the storehouse). The audio
engine and master clock is **Hibiki** (響, resonance — everything syncs to it). The
names appear in user-facing surfaces (boot splash, settings, docs, marketing); crate
names stay `kiriko-*` and code identifiers stay plain English per the glossary. Future
subsystem names come from the same craft vocabulary and are logged here. Added
2026-07-13 at Mack's request.

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
`Op::Batch` — one undo step. Added 2026-07-13 at Mack's request.

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
2026-07-13 at Mack's request. Spec: [06-RENDER-PIPELINE.md](06-RENDER-PIPELINE.md) §3.1.

**K-070 · DECIDED · The graph editor is a general derivative-lens editor, in the
Timeline.** Three points from Mack (2026-07-13):

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
own timeline tab.** Refines the Sequence layer (K-020) per Mack (2026-07-13):

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
for the property-row timeline restructure (07-UI-SPEC §5, K-070), from Mack (2026-07-13):

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
Mack's insistence that the viewport carry no "top bit": egui_dock (0.16) draws a tab bar on
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
with no tab bar** (Mack, 2026-07-14: the viewport must have no top bit). This reverses
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
the sequence view.** Confirmed by Mack (2026-07-14), building on K-021, K-070, K-071, K-072:

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
  **implemented later** (a good candidate for a focused `fable` session, per Mack).
- **Increments:** *2a* (now) — footage Retime graphable, both lenses + the setting + the
  correct default lens; *2b* — the full [04-RETIMING.md](04-RETIMING.md) §9.2 in-graph segment
  editing (RateSegment endpoint drags, compensating edits, Rate↔Map conversions); *2c* (later)
  — the sequence-view graph pane.

**K-076 · DECIDED · The Retime graph channel is named by its lens: Time (value) and Velocity
(speed).** Confirmed by Mack (2026-07-14), refining K-075. The Retime channel — its outline
row and its graph — reads **Time** in the value lens (source position, "which frame is
showing") and **Velocity** in the derivative lens (the Vegas velocity-envelope heritage the
speed graph already invokes). This **reverses the glossary §9 "velocity → speed" ban for this
one UI label**: "speed" remains the term for the quantity everywhere else (percentages,
RateSegment speeds, prose, identifiers); "velocity" is permitted solely as this channel's
derivative-lens label. The channel also behaves like any other property — it carries a
stopwatch/keyframe control in the outline — and its **default lens is the value (Time) lens**
(the Vegas-preference of K-075 defaults **off**), so the channel opens to Time.

**K-078 · DECIDED · The Time (value) lens is a fully bezier-keyframed property, identical to
any transform channel.** From Mack (2026-07-14), extending K-025/K-070/K-075/K-076. The Retime
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
auto-fits vertically by default.** From Mack (2026-07-15). The curve editor previously mapped x
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

**K-080 · DECIDED · The speed lens draws the exact derivative of the value bezier.** From Mack
(2026-07-15). The speed (derivative) view sampled its curve by central finite difference at
half-frame steps — a display stopgap that could smear the shape near steep handles. It now uses
`anim::evaluate_speed`, the closed-form `dv/dt = y′(u)/x′(u)` of the value-lens cubic (with the
`x′` floor at a 100%-influence handle), so the speed curve is precisely the slope of what the
value lens draws: bezier easing in the value view shows as the matching smooth speed curve, a
straight span as a flat speed, a Hold as zero. This is the value/speed "two views of one data"
promise (K-070) made exact.

**K-081 · DECIDED · Tangent handles are draggable in the speed lens too.** From Mack
(2026-07-15). The speed (derivative) lens showed one draggable speed point per key; it now also
carries the same gold tangent handles as the value lens for a selected key, so a curve can be
eased from either view. In the speed graph a handle's **height is that side's speed** and its
**horizontal reach is its influence** (After Effects' speed-graph ease bars); dragging writes the
same `SideInterp::Bezier` store through `apply_tangent`, so the value and speed lenses stay in
lock-step. Clicking a speed key selects it (as in the value lens) to reveal its handles. The
value lens keeps the unified partner-length behaviour (K-072 refinement); the speed lens mirrors
a unified drag but keeps the partner's own reach (no screen-length preservation — the speed lens
is about the speeds themselves).

**K-082 · DECIDED · Linux is a supported build target.** From Mack (2026-07-16), after outside
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
transforms unchanged (the transform is serialised in full). Added 2026-07-19 at Mack's
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
Mack's request. Built in an isolated worktree; not pushed — another agent may also claim
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
Mack's request. Built in an isolated worktree; not pushed — another agent may also claim
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
