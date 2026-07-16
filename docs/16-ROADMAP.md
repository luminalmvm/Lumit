# Luminal roadmap

**Status: canonical.** Build order per decision K-002: gaming-edit MVP first, then the march
to full AE replacement. Every phase ends at a **gate**: a user-visible capability plus
performance criteria on the reference hardware defined in
[13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md). A phase is not done until its gate
demo can be performed cold, from a fresh project, without a crash or a workaround.

Phases are strictly ordered for the vertical slice they build; work inside a phase can be
parallel. The anti-lesson from Olive ([05-ARCHITECTURE.md](05-ARCHITECTURE.md) appendix)
applies: ship the slice, never rewrite the world.

---

## Phase 0 — Skeleton

The app exists: window, docking shell with the default workspace layout
([07-UI-SPEC.md](07-UI-SPEC.md)), dark Aizome theme ([15-DESIGN.md](15-DESIGN.md)), project
new/open/save (`.lum` container, autosave, crash recovery), import of footage and audio,
Project panel, Viewer displaying footage with preview-resolution control, Timeline showing
Footage layers with in/out trimming, playback of a single clip with sample-accurate audio,
frame-indexed seeking.

**Gate 0**: open a 4K60 H.264 game capture, scrub it smoothly, play it back with audio in
sync, save, kill the process mid-edit, relaunch, recover. UI stays under the 8 ms
interaction budget throughout.

## Phase 1 — Compositing core

Layers stack and animate: transform pipeline (4×4 from day one), keyframes with
AE-compatible bezier maths, the graph editor (value + speed views), masks, blend modes,
mattes, solids, Precomp layers with collapse, adjustment layers, basic text, the `three_d`
switch with a basic one-node camera (parallax and smooth camera moves — the flow style's
second-most-used technique; depth of field and lights arrive in phase 5), the evaluation
graph with the three-tier content-hash cache and cache bars, background cache fill, export
queue with H.264/HEVC (NVENC/AMF/QSV + software fallback) and the YouTube/vertical presets.

**Gate 1**: build a six-layer animated composite (footage, text, solid, adjustment grade,
precomp, and a keyframed 2.5D camera move over parallaxed layers), see cache bars fill
while idle, play it back at 1080p60 in real time from warm cache, export it and verify the
export matches preview pixel-for-pixel at Full/fp16. Graph-editor ergonomics get judged
here too: easing a camera move must feel better than AE, not merely equivalent — this
style is named after interpolation quality.

## Phase 2 — Retime

The flagship: Sequence layers with clips, cutting, and gaps; the full Retime system of
[04-RETIMING.md](04-RETIMING.md) — both graph lenses, segment model, overrun hold with
indicator, trim-to-source, reverse gating; frame interpolation policies nearest/blend/flow
(first flow implementation); audio waveforms in the Timeline, manual and automatic beat
markers, snapping of edit points and retime features to beats
([09-AUDIO.md](09-AUDIO.md) v1 scope).

**Gate 2 — the velocity-edit test**: recreate a reference Vegas-style velocity edit — a
240 fps clip cut into four beat-synced clips on one Sequence layer, each with a
ramp-freeze-ramp retime, flow interpolation on — and confirm the beat-sync covenant: editing
any ramp moves no cut. Preview plays in real time; flow quality is comparable to Twixtor on
the easy 80% of shots and degrades gracefully on the rest.

## Phase 3 — The look

Tier-1 effect suite of [08-EFFECTS.md](08-EFFECTS.md): flow motion blur, glow, camera shake
(beat-triggerable), smooth zoom, RGB split, flash/strobe, blurs/sharpen, grade with preset
browser, LUT loader, echo/trails, glitch basics. Per-layer motion blur. Preset save/load and
import/export. Scopes panel.

**Gate 3 — the v1 milestone (K-002, [00-VISION.md](00-VISION.md) §4)**: a flow-style editor
completes an MVM-style edit start-to-finish in Luminal alone — beat-marked cuts, flow ramps,
a smooth camera move, a masked transition, the full stacked look — and exports for YouTube;
the look previews in real time at 1080p60 on the reference machine; a six-hour editing
session leaks no memory and never crashes. **This gate is v1.0.**

## Phase 4 — Extensibility

Expressions (QuickJS-ng, AE surface subset per [12-PLUGINS.md](12-PLUGINS.md)); the OFX host
(out-of-process, CPU rendering first) proven against Twixtor, RSMB, and Sapphire demos; the
KFX C ABI, validator, and template repo; the AE Bridge exporter panel and Luminal-side
importer with the fidelity report ([11-AE-IMPORT.md](11-AE-IMPORT.md)); best-effort `.aep`
structural import.

Also in this phase: the migration-aware first-run screen (K-006,
[07-UI-SPEC.md](07-UI-SPEC.md) §13.1) — it belongs alongside the AE Bridge, when
switchers start arriving.

**Gate 4**: a real community AE montage project imports via the Bridge with transforms,
keyframes, retimes, and mapped effects intact and an honest per-item report; Twixtor OFX
renders inside Luminal; a deliberately crashing test plugin takes down its process, not Luminal.

## Phase 5 — AE parity march

Ongoing, ordered by community demand: 2.5D cameras/lights/DOF in full, game camera-path
import (HLAE and friends — absorbing the flow scene's external tool chain), the tracker and
stabiliser, keying and matte tools, rotoscoping (the flow style leans on hand-roto and
multi-pass footage; depth-pass-aware compositing belongs here), a particle system, tier-2
effects, text animators, shape operators (repeater et al.), variable mask feather, the
**Composer** audio workspace ([09-AUDIO.md](09-AUDIO.md) §Composer), pitch-preserving audio
retime, OFX GPU render suite, OCIO colour management, app scripting.

**Gate 5 (rolling)**: each quarter, one published "made only in Luminal" piece that
previously required AE plus plugins.

## Phase 6 — Beyond parity

The long ambitions (K-023): working directly in 3D, Blender scene import, deeper 3D
compositing, and the node view over the evaluation graph (K-036 — grading node chain
first, full node compositing after). Also: Lottie export, OpenTimelineIO interchange, render-farm/CLI export, and
a first-class macOS/Metal release (K-033 — the engine already runs on Metal via wgpu;
this adds VideoToolbox zero-copy, ProRes, the OFX Metal suite, and notarisation).
Specified when we get there; the 4×4 transform core, rational time, and DAG engine were
chosen so none of this requires a rewrite.

---

## Standing rules

- Performance gates run in CI on every merge ([13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md));
  a regression blocks the merge, phase work notwithstanding.
- Every feature lands with its spec section, tests, and glossary compliance
  ([14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md) definition of done).
- Docs are canonical: if implementation must diverge, the doc changes first (or in the same
  change), with a decision-log entry when it reverses a K-number.

## Open questions

- Which phase first runs on the desktop (Windows) rather than the development MacBook —
  recommend Gate 0 is verified on both, gates 2+ are judged on Windows.
- Whether Phase 4's OFX host lands before or after the AE Bridge inside the phase — both are
  gate requirements; order by whichever unblocks community testers sooner.
