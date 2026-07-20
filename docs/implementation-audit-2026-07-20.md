# Implementation audit — docs vs code (2026-07-20)

Every numbered spec (00–16) checked claim-by-claim against the code at `f2bc3ed`
(branch point of `claude/doc-implementation-audit-2fmbs8`). Nothing was taken on the
docs' word: each claim was traced to source with `file:line` evidence, CI history was
pulled from GitHub Actions, and the test suite was run where the container allows.

**Purpose.** The specs are written docs-first in confident present tense, so they read
as if everything is built. This audit separates what is actually in the code from what
is still to be implemented, so a pre-release worklist can be drawn from it. Already
implemented claims are deliberately omitted from the tables below (each section opens
with a short note on what *is* solid); the tables list only the gaps.

## Fixes applied (living log)

Work resolving the findings below, newest first. The finding tables are left as the
original snapshot; this log is the running record of what has since changed.

- **Cross-cutting ① — CI red → fixed.** `Compositor::accumulate` now sums the
  accumulation-motion-blur combine in fp32 (ping-ponged `Rgba32Float` targets, resolved to
  the working format once), so the still-scene identity is bit-exact and the failing
  `accumulation_still_scene_is_identity_and_moving_scene_smears` test passes. Verified
  locally against a software Vulkan adapter (lavapipe) with a new fractional-coverage
  regression test; end-to-end CI still pending a PR. Retires the accumulation-path half of
  **06 §4** (per-layer `motion_blur_average` remains fp16 — the other half).
- **Cross-cutting ⑥ / 00 §6 / 05 naming — fixed.** Code codenames reconciled to the decided
  astral register (K-083): `Togi → Nova`, `Kura → Nebula`, `Hibiki → Pulsar` (comments and
  boot-log strings only).
- **08 §3.9 (Sharpen "radius-free") — doc-synced.** Doc updated: plain Sharpen carries an
  adjustable Radius (T15), not a fixed 3×3.
- **08 §5 / 10 §5 (preset `.kpreset`/`.kfxpreset`) — doc-synced.** Corrected to `.lumfx` (the
  name already used in code, K-065/K-129, 07 and GUIDE); the "zipped with embedded assets"
  claim reworded as a future extension (v1 is plain JSON).
- **08 §3.1 (flow "variational/patch-match hybrid") — resolved.** Doc synced to the shipped
  DIS engine; 08 Open Question 1 resolved and logged as **K-169**.
- **15 §6.3 (cache-bar colours) — doc-synced.** Rewritten to the shipped two-tier bar (RAM
  `success` mint, disk `cache_disk` steel-blue per 06 §5.6, 2px bands); the all-mint tonal
  ramp and VRAM tier noted as future. No code change.
- **10 §1.1 (enum serialisation "lower-kebab") — doc-synced.** Corrected to serde's default
  PascalCase / external tagging, matching the shipped `.lum` format and round-trip tests.
- **06 §3.5a (luma mattes in linear) — fixed.** Luma/silhouette mattes now gate by Rec.709
  luma of the sRGB-*encoded* signal (perceptual, matching AE), not linear light. lumit-gpu,
  verified locally; new test pins ~0.735 for a linear-0.5 grey.
- **04 §7.2 (overrun clamp) — fixed.** `sequence::resolve` clamps the mapped source position
  to the clip's `[source_in, source_out]`, so overrun holds the trimmed boundary frame
  instead of running into media past the trim; one seam covers preview, export, and the eval
  cache key. lumit-core, verified locally with a new overrun test. (Footage-layer retimes have
  no source trim, so holding at media end is already correct there — this is clip-specific.)
- **09 §6 (audio solo) — fixed (CI-pending).** `comp_audio_jobs` now honours solo, so a soloed
  layer silences other audio the way it hides other video. lumit-ui — verified by reading; CI
  on the PR is its check.
- **15 §10 (error banner) — fixed (CI-pending).** Completion notices split into a neutral
  `notice` field; genuine errors now carry the fig error tint. lumit-ui — CI-verified on the PR.

## Status legend

| Status | Meaning |
|---|---|
| **Not implemented** | No code exists for the claim (searches listed where useful) |
| **Partial** | Some of it exists; the missing part is named |
| **Contradicted** | Code exists but does something different from the doc |
| **Violated** | A binding rule the code breaks |
| **Future-by-design** | The doc itself marks it later/post-v1 (still listed, since it is to-be-implemented) |
| **Unwired** | Built and tested, but nothing in the app calls it |
| **Unverifiable** | Needs hardware/runtime measurement to confirm |

The **Status** column (above) classifies what *kind* of gap a finding is. The **Resolution /
next step** column (last column of every findings table) tracks what has since been *done* about
it — so the tables double as a live worklist. Its markers:

| Marker | Meaning |
|---|---|
| **—** | Not yet addressed — still to do (this is most rows) |
| **✅ Done** | Fixed and landed (commit noted); `tested` = a regression test covers it, `CI-pending` = verified only once PR CI is green (lumit-ui can't build locally) |
| **◑ Partial** | Partly resolved; the remainder is named |
| **👁 review …** | **Needs your eye** — a visual or behavioural change CI/tests can't fully judge; look at the real result and confirm it reads right |

Every time a finding is resolved, its row's last column is updated here in the same change.

---

## Cross-cutting findings (read these first)

1. **CI on `main` is red, and has been for at least the last 30 runs** — including
   HEAD `f2bc3ed`. One test fails on both macOS and Windows:
   `shell::draws::render_below_at_tests::accumulation_still_scene_is_identity_and_moving_scene_smears`
   (`crates/lumit-ui/src/shell/draws.rs:1643`, "a still scene averaged over N must
   equal the plain composite bit-for-bit"). fmt, clippy, the coverage gate and the
   hex lint all pass. This directly contradicts CLAUDE.md / K-007's "a red CI blocks
   everything else" and the audit-2026-07-19 claim of a fully green workspace.
   (Local verification: the six non-FFmpeg crates pass 301/301 tests in this
   container; the failing test needs the media build.)
2. **Three docs describe subsystems that do not exist at all**: 11-AE-IMPORT (no
   importer code of any kind), 12-PLUGINS (no OFX host, no LFX ABI, no expression
   engine — only an `EffectNamespace` enum with unused `Ofx`/`Lfx` variants), and most
   of 13-PERFORMANCE-RULES (no benchmark harness, no CI perf gate, no resource
   governor, no degradation ladder).
3. **"Enforced" claims with no enforcement tooling**: glossary banned terms (01 §9),
   performance budgets "gate merges" (13 §7.3, 16 standing rules, 00 §3.1), contrast
   floors "CI-checked" (15 §9), i18n "from day one" (K-005, 14 §7), `cargo deny`,
   fuzzing, golden-frame tests, pinned toolchain (14 §6/§9). CI actually runs: fmt,
   clippy `-D warnings`, tests (macOS + Windows), coverage ≥75% on `lumit-core` +
   `lumit-project` only, and the no-hex-outside-theme lint. That is the full list.
4. **A real playback scheduler exists but is unwired.** `FrameRing`, `Lookahead` and
   the adaptive `RealtimeController` (Full→Half→Quarter with hysteresis) are
   implemented and unit-tested in `lumit-eval/src/schedule.rs` but never referenced
   by the app; playback uses a simpler single-frame prefetch, and "adaptive
   degradation" in practice is a manual resolution picker plus a draft cap.
5. **The evaluation graph does not render.** `lumit-eval/graph.rs` compiles the DAG
   (with folding and dedup) but the pixel pass is deferred; actual rendering is the
   imperative draw-list builder in `lumit-ui` (`export.rs` / the preview shell).
   ROI/DoD, macro-tiles, the texture pool and per-node CPU fallback do not exist.
6. **Subsystem names in the docs don't match the code.** The evaluator calls itself
   **Togi** (`lumit-eval/src/lib.rs:1`), the RAM cache tier **Kura**
   (`app_state/previewing.rs:173`); "Nebula" matches the cache crate, but "Nova" and
   "Pulsar" appear nowhere in the code.
7. **Four documented crates don't exist** (`lumit-time`, `lumit-expr`, `lumit-ofx`,
   `lumit-lfx`); two exist that 05 doesn't document (`lumit-flow`, `lumit-text`).
8. **Typed-time rule is broken at the evaluation layer**: `lumit-eval` and property
   evaluation thread raw `f64` time through every API including the content-hash
   cache key, against 14 §2's "authoritative time MUST NOT be `f64`".

---

## 00-VISION

**Findings.** The engineering pillars are mostly real: GPU compositing (30+ WGSL
shaders), blake3 content-hash frame keys, journalled autosave + crash recovery,
first-class Retime with two graph lenses, DIS optical-flow slow motion wired into
preview and export, the built-in effect suite, a 2.5D camera, and an
NVENC→AMF→QSV→software export ladder with YouTube presets. The overstated parts are
the plugin story, the AE importer, the performance-enforcement claims, and GPU-reset
recovery.

| § | Claim | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| §2, §3.2 | "OFX plugin support"; "plugins in separate processes" | Not implemented | Only `EffectNamespace::Ofx` enum variant (`model.rs:407`) used as a hash byte. No host, no subprocess/IPC anywhere | — |
| §2 | "an AE project importer" | Not implemented | No `.aep`/Bridge/Lottie code (see 11 below) | — |
| §3.1 | "adaptive degradation" | Partial (unwired) | `RealtimeController` exists + tested (`lumit-eval/src/schedule.rs:256`) but never constructed by the UI; live preview hardcodes `Quality { divisor: 1 }` (`app_state/previewing.rs:102`); "Auto" res keys off display size, not load | ◑ Slice built ec2abe2 · ⚠ UNTESTED — can't build lumit-ui here, CI compiles only; needs your run under a heavy comp. Cost = CPU composite time (partial proxy); render-ahead ring (06 §6.4) still separate · 👁 verify adaptation |
| §3.1 | Performance "budgeted and CI-enforced" | Contradicted | `.github/workflows/ci.yml` has no perf/benchmark job; no criterion benches in the workspace | — |
| §3.1 | "the UI thread never renders a frame"; interactive at thousands of layers | Unverifiable / partial | Decode/export/mix are off-thread, but preview GPU submission happens on the UI thread via eframe's shared queue (`shell/gpu.rs:865-909`); no scaling benchmark exists | — |
| §3.2 | "treat GPU resets as routine" | Not implemented | No device-lost handling anywhere; `main.rs:48-64` only pins a backend to avoid first-frame loss | — |
| §3.5 | "One glossary, enforced everywhere" | Partial | No glossary lint in CI or scripts; discipline is manual (see 01) | — |
| §4 | v1 milestone "build a masked transition" | Partial | Masks are static, Add-mode, no feather, not animatable (`mask.rs:8-9,30`) — an animated masked transition isn't possible yet | — |
| §6 | "**Nova** render pipeline, **Nebula** cache, **Pulsar** audio" | Contradicted (minor) | Code names: Togi (eval), Kura (RAM tier); Nebula matches; Nova/Pulsar absent from code | ✅ Done · d87bcf4 (Togi→Nova, Hibiki→Pulsar) |

---

## 01-GLOSSARY

**Findings.** The core vocabulary (Composition, Layer, Clip, Sequence layer, Precomp,
Retime + lenses, markers, matte, blend modes, panels) is faithfully implemented. The
gaps are terms defined in present tense for features that don't exist, and the §9
enforcement claim: **no tooling enforces the banned-terms table** — CI's only text
lint is the hex/design-token check. Despite that, the code is very clean; only
borderline internal uses were found.

Terms naming unimplemented features:

| Term | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|
| Shape layer | Not implemented | `LayerKind` has no Shape variant (`model.rs:724-759`); no path/fill/stroke types | — |
| Null layer | Not implemented | No `Null` kind; parenting exists so rigs work via any layer, but the type is absent | — |
| Light layer | Not implemented | No `Light` kind (Camera exists) | — |
| Audio layer / Audio item | Partial | No `AudioItem` asset, no `Audio` layer kind; audio exists only as the audio channel of footage layers | — |
| Expression | Not implemented | No JS engine of any kind; only a doc-comment "expression slot" (`anim.rs:274`) | — |
| Proxy (footage) | Not implemented | No proxy generation or toggle; "proxy" hits are the unrelated DoF depth proxy | — |
| Stretch (time-stretch) | Not implemented | No layer rate multiplier; all "stretch" hits are UI pixel stretching | — |
| Composer | Future-by-design | Glossary itself marks it "planned"; absent | — |
| OFX / LFX (as APIs) | Not implemented | Enum variants only, hashed into the frame key; no loader/host | — |
| Workspace presets | Partial | Docking + "Reset workspace" exist; named presets don't ("presets arrive with the panel set", `app_update.rs:760`) | — |
| Switches: shy, quality | Partial | `Switches` lacks both (`model.rs:615-643`) | — |
| Cache VRAM tier | Partial | RAM + disk tiers only; VRAM tier explicitly deferred (`lumit-cache/src/lib.rs:4`) | — |

Banned-term reality (§9):

| Item | Finding |
|---|---|
| Enforcement | **Manual only.** No lint/CI/script greps for banned terms; the one custom CI lint is the hex check (`ci.yml:101-114`) |
| "render" for export | Borderline violation: hidden command-palette search alias `"render output video mp4"` surfaces the Export command (`command_palette.rs:158,429`); visible labels are compliant |
| "velocity" as a quantity | Minor: internal comments/test names use it (`layers.rs:876`, `sequence.rs:112,363`, `retime.rs:349`); the UI "Velocity" lens label is explicitly permitted (K-076) |
| "track matte" | Borderline: comments use it as the feature's working name (`eval/graph.rs:47`, `model.rs:268`) rather than as an AE citation; public types are correctly `MatteRef`/`matte` |
| track/bin/event/CTI/pre-render | No real violations found (remaining hits are widget/container senses) |

---

## 02-DECISIONS

**Findings.** The decision log tracks the code unusually closely — of ~150 numbered
entries, nearly all DECIDED-and-built entries verify against named symbols and tests,
and no decision was found that the code silently violates (superseded entries are
correctly followed by their successors). The gaps are the forward-looking subsystems
(most also covered by docs 11/12/13) and a handful of present-tense claims without
code. K-103 parenting is implemented *further* than its own text claims (render wiring
is live in preview and export).

| K-# | Decision (short) | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| K-005 | i18n table for UI strings "from day one" | Partial | en-GB voice is followed, but no i18n infrastructure exists; all strings hardcoded | — |
| K-006 | Migration-aware first-run screen | Not implemented | No first-run screen (doc marks post-v1 polish) | — |
| K-014 | Optional CUDA per-node accelerator | Not implemented | No cudarc/`as_hal`; flow is WGSL-only — consistent with "never a requirement" | — |
| K-016 | Three-tier content-hash cache | Partial | RAM (`ByteLru`) + disk exist; VRAM tier, `index.db` and the governor deferred (`lumit-cache/src/lib.rs:4`). The K-100 "VRAM budget" setting caps the UI display-texture cache, not a Nebula tier | — |
| K-017 | UI thread never evaluates; work-stealing pool | Partial | Epoch cancellation + off-thread decode exist; no work-stealing pool — ad-hoc `thread::spawn` per job; preview GPU submit is on the UI thread | — |
| K-018 | Degrade-never-crash governor + ladder | Partial | Autosave/recovery + VRAM eviction exist; no central governor, no ladder, no device-loss recovery | — |
| K-025 | AE-compatible keyframe maths incl. spatial + roving | Partial | Temporal side complete and tested (`anim.rs`); spatial beziers + roving keyframes deferred (`anim.rs:276`) | — |
| K-030 | Cached + Realtime-adaptive preview modes | Partial (unwired) | No `PreviewMode` toggle; `RealtimeController` unwired; resolution picker is manual | ◑ Slice built ec2abe2 · ⚠ UNTESTED — can't build lumit-ui here, CI compiles only; needs your run under a heavy comp. Cost = CPU composite time (partial proxy); render-ahead ring (06 §6.4) still separate · 👁 verify adaptation |
| K-034 | Perceptual ops in Oklab; params declare domain | Partial | Oklab CPU+WGSL twin exists and is used; no per-parameter domain declaration; colour keyframe interpolation does not route through Oklab | — |
| K-035 | Universal per-effect strength matte | **Not implemented** | No strength-matte slot on `EffectInstance` (`model.rs:511`); no host mixing plumbing anywhere (also flagged under 08) | — |
| K-050 | v1 audio sync toolkit | Partial | Beats/markers/waveform/mute real; volume keyframes and audio layers absent (see 09) | — |
| K-060 | AE import | Not implemented | See 11 | — |
| K-061 / K-062 / K-066 | OFX host / LFX API / plugin depth rules | Not implemented | See 12; enum variants only | — |
| K-063 | Expressions on QuickJS-ng | Not implemented | No JS engine dependency; no expression field on `Property` | — |
| K-069 | Project-wide 8/16/32 bpc switch | Not implemented | `WORKING_FORMAT` is hard-const `Rgba16Float` (`lumit-gpu/src/lib.rs:62`) — matches the entry's own deferral | — |
| K-071 | Sequence layer's dedicated timeline tab as editing home | Partial | Model invariants + inline razor real; the dedicated editing surface is not yet the home (entry itself notes this) | — |
| K-116 | Dense controls ≥24px visual but ≥32px hit-slop; "nothing hit-tests below 32px" | Partial | No hit-slop enforcement found; resize slots are 16px and trim handles 8px on the time axis (`timeline/panel.rs:660,1187`) | — |
| K-036 | Node-view lens | Future-by-design | Post-parity phase; no node editor | — |
| K-155 | Keylight spatial/garbage/CC extras | Future-by-design | The entry itself records the deferral; correctly absent | — |
| K-168 | Timeline columns — deferred set | Future-by-design | shy/quality/preserve-transparency/pick-whip "deliberately not built yet", and indeed absent | — |

Everything else in the log (≈120 entries: the whole effect-suite run K-090–K-115 and
K-117–K-168, theme/settings/parenting/solo/cache-key decisions, supersessions
K-026→K-069, K-067/K-083→K-087, K-073→K-074) verified as implemented.

---

## 03-DATA-MODEL

**Findings.** The core model is substantially implemented at a Phase-0/1 slice:
rational time with four timebases, UUIDv7 identity, AE keyframe maths, the op
journal/undo store, sequence clips, masks (static), effects, markers, motion blur,
parenting, camera. But the top-level container diverges from the doc, several
documented structs don't exist, `Property` is scalar-only, and a few semantics are
outright contradicted.

| § | Claim | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| 2 | `Project { id, settings, assets, compositions }` | Contradicted | Actual: `Document { id, items, auto_folders, extra }` (`model.rs:1053`); no `ProjectSettings`, no comp-ordering vec | ✅ Doc-synced (03 structural pass) — code verified; unbuilt bits now marked future |
| 2 | `ProjectSettings` | Not implemented | No such struct | ✅ Doc-synced (03 structural pass) — code verified; unbuilt bits now marked future |
| 2 | Asset kinds `AudioItem`, `StillItem`, `SequenceItem` | Not implemented | `ProjectItem` = Footage/Folder/Composition/Solid only (`model.rs:1004`) | ✅ Doc-synced (03 structural pass) — code verified; unbuilt bits now marked future |
| 2/3 | `FootageItem` carries interpretation + proxy state | Partial | `{ id, name, media, extra }` only (`model.rs:31`) | ✅ Doc-synced (03 §3/§5.3/§9/§11/§12 pass) — code verified; unbuilt bits marked future |
| 3 | `MediaRef.fingerprint` | Partial/contradicted | No fingerprint field ("lands in slice 4", `model.rs:20`); `Fingerprint` exists only in `lumit-media` | ✅ Doc-synced (03 §3/§5.3/§9/§11/§12 pass) — code verified; unbuilt bits marked future |
| 3 | `FootageInterpretation` (fps override, alpha, colour space, loop, TC policy) | Not implemented | None of the types exist | ✅ Doc-synced (03 §3/§5.3/§9/§11/§12 pass) — code verified; unbuilt bits marked future |
| 4 | `Composition.pixel_aspect`; `Composition.depth: CompDepth` | Not implemented | Neither field exists (depth superseded by K-069 anyway) | ✅ Doc-synced (03 §4/§6/§7 pass) — code verified; unbuilt bits marked future |
| 4 | 16384² hard cap | Not implemented | No enforcement (doc's own open question) | ✅ Doc-synced (03 §4/§6/§7 pass) — code verified; unbuilt bits marked future |
| 4 | `work_area` mandatory | Partial | Code: `Option<(CompTime, CompTime)>`, `None` = full comp | ✅ Doc-synced (03 §4/§6/§7 pass) — code verified; unbuilt bits marked future |
| 5.1 | `Layer.stretch` | Not implemented | No field (doc's own open question) | ✅ Doc-synced (03 structural pass) — code verified; unbuilt bits now marked future |
| 5.1 | `Layer.audio: AudioProps` (animatable level) | Not implemented | Mute is `Switches.audible`; no volume anywhere | ✅ Doc-synced (03 structural pass) — code verified; unbuilt bits now marked future |
| 5.1 | Per-layer `markers` | Not implemented | Markers only on `Composition` (`model.rs:86`) | ✅ Doc-synced (03 structural pass) — code verified; unbuilt bits now marked future |
| 5.1 | `Switches { shy, quality, adjustment }` | Contradicted | Missing shy/quality/adjustment (adjustment is a `LayerKind`); code adds an undocumented `fx` switch (`model.rs:615`) | ✅ Doc-synced (03 structural pass) — code verified; unbuilt bits now marked future |
| 5.1 | `source: LayerInputSource` defaults **None**; K-125 migration `false→None` | Contradicted | Default is `EffectsAndMasks` (`model.rs:289`); migration maps `false→Masks`, absent→`EffectsAndMasks` (`model.rs:299`, tests `:1220`) | ✅ Doc-synced (03 structural pass) — code verified; unbuilt bits now marked future |
| 5.2 | `Precomp { comp, retime }` | Contradicted | No retime on Precomp (`model.rs:740`); only Footage carries retime | ✅ Doc-synced (03 structural pass) — code verified; unbuilt bits now marked future |
| 5.2 | Layer kinds Shape/Null/Audio/Light | Not implemented | See 01 | ✅ Doc-synced (03 structural pass) — code verified; unbuilt bits now marked future |
| 5.3 | `Clip.label: LabelColour` | Partial | Absent from `Clip` (`place` split into start+duration is fine) | ✅ Doc-synced (03 §3/§5.3/§9/§11/§12 pass) — code verified; unbuilt bits marked future |
| 6.1 | `Property<T: PropValue>` generic, with `id` + expression slot | Contradicted/partial | `Property` is `f64`-scalar only (`anim.rs:285`); Vec2/Vec3 modelled as separate scalar dims; no id, no expression | ✅ Doc-synced (03 §4/§6/§7 pass) — code verified; unbuilt bits marked future |
| 6.2 | `Keyframe` spatial tangents, roving, label | Not implemented | `{ time, value: f64, interp_in, interp_out }` only (`anim.rs:29`) | ✅ Doc-synced (03 §4/§6/§7 pass) — code verified; unbuilt bits marked future |
| 6.2 | Bezier influence 0.1..=100 (%) | Contradicted (units) | Code influence is a fraction in (0,1] (`anim.rs:181`); curve maths itself matches AE | ✅ Doc-synced (03 §4/§6/§7 pass) — code verified; unbuilt bits marked future |
| 6.3/6.4 | Expression stage in evaluation | Not implemented | No expression struct/stage | ✅ Doc-synced (03 §4/§6/§7 pass) — code verified; unbuilt bits marked future |
| 7 | `Mask` animatable path/opacity, mode, feather, expansion | Partial/contradicted | Static path + scalar opacity, no mode (Add hardcoded), no feather/expansion (`mask.rs:29,203`; full set noted future `mask.rs:8`) | ✅ Doc-synced (03 §4/§6/§7 pass) — code verified; unbuilt bits marked future |
| 9.1 | `TextDocument` styled runs, fonts, stroke, tracking, alignment | Partial | Single run `{ text, size, fill }` (`model.rs:804`) | ✅ Doc-synced (03 §3/§5.3/§9/§11/§12 pass) — code verified; unbuilt bits marked future |
| 9.2 | `ShapeElement` tree | Not implemented | No shape types (mask shape helpers unrelated) | ✅ Doc-synced (03 §3/§5.3/§9/§11/§12 pass) — code verified; unbuilt bits marked future |
| 9.3 | `CameraProps` / `LightProps` | Partial / not implemented | Camera inline `{ zoom }` only; no lights | ✅ Doc-synced (03 §3/§5.3/§9/§11/§12 pass) — code verified; unbuilt bits marked future |
| 11 | `Marker.colour` | Not implemented | No colour field (`markers.rs:29`) | ✅ Doc-synced (03 §3/§5.3/§9/§11/§12 pass) — code verified; unbuilt bits marked future |
| 12 | Migration framework | Partial | `schema_version`/`min_reader` gating + ad-hoc serde shims; no migration registry | ✅ Doc-synced (03 §3/§5.3/§9/§11/§12 pass) — code verified; unbuilt bits marked future |

---

## 04-RETIMING

**Findings.** The retime maths engine (`retime.rs`, 2 522 lines) is exceptionally
faithful: all five eases with exact integrals, Rate/Map segments, the Newton-in-bracket
cubic solver, exact Rate→Map, least-squares Map→Rate, splitting/merging, freeze, slip,
the reverse gate, overrun detection, i128/flick overflow fallback — all implemented and
heavily tested, including the §12.4 worked example bit-for-bit. The gaps are the UI/
command surface (several core methods have **no caller**), graph-lens chrome, and one
genuine render bug (overrun clamp).

| § | Claim | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| 7.2 | Overrun holds the boundary frame of the **trimmed** extent (`clamp(f(t), src_in, src_out)`) | **Contradicted (bug)** | Render clamp uses whole-media length, not the clip's `src_out` (`pixels.rs:135`): a clip trimmed to 2s of a 10s file shows media past 2s on overrun instead of holding | ✅ Done · 0f3eae8 · tested (lumit-core) |
| 6.1 | Kink badge at boundaries when one-sided speeds differ | Not implemented | Nothing draws it; `smooth` flag stored but unused by UI | — |
| 9.4 | RATE/MAP type chips per segment | Not implemented | No chip rendering | — |
| 9.1/9.2 | Graph lens draws source-extent band, media-end line, overrun band | Partial | Overrun hatch only on the timeline clip bar (`timeline/panel.rs:994-1076`); graph draws curve + reference lines only | — |
| 6.2 | Reverse toggle UI + lock glyph + negative region | Partial | Core clamp works; `allow_reverse` only set implicitly; no toggle/glyph/negative editing | — |
| 12.1 | Command table keys (R/E/F/1–6/[ ]/Tab/quantise) | Not implemented | No retime commands in `shortcuts.rs`/palette; actual affordances differ (header buttons, bottom-bar labels) | — |
| 7.3 | `freeze_at_playhead` | Partial (unwired) | `insert_freeze` implemented + tested (`retime.rs:998`); zero UI callers | — |
| 5.4 | Delete boundary → merge via graph editor | Partial (unwired) | `merge_boundary` implemented + tested (`retime.rs:926`); no UI caller | — |
| 5.2/9.2 | "Convert to rate" menu + drift warning badge | Partial (unwired) | Core fit implemented (`retime.rs:1041,1462`); no menu, no badge | ◑ Partial · ⚠ CI-compiled (can't build lumit-ui here) · wired a **→Rate** button in the speed-lens graph header that calls the tested `with_segment_as_rate` on the segment under the playhead and commits `SetLayerRetime`; the fit **drift** is surfaced as a status notice (e.g. "fitted, 3 ms drift") rather than a persistent badge. The always-visible drift badge is still to build · 👁 try →Rate on a mapped segment |
| 12.2 | Preset row includes **Hold** | Partial/contradicted | Buttons are Lin/Slow/Fast/Smth/Shrp only (`graph.rs:167-173`) | — |
| 12.3 | Bulk quantise-boundaries-to-beats | Not implemented | Only per-drag beat snapping exists | — |
| 9.2 | Speed-lens numeric % entry; Alt-compensated edit | Partial | Only boundary/speed-key drags | — |
| 11.2 | Stretch rewrites the Retime store | Not implemented | No stretch op exists at all | — |
| 11.6 | `sourceTime()`/`retimeSpeed()` expression reads | Not implemented | No expression engine | — |
| 7.3/8.2 | Outward trim extends map with constant-speed segment | Partial | Only inward trims implemented; outward-extend documented in code as "a separate op" not present (`sequence.rs:221-259`) | — |
| 8.2 | Copy/paste retime, paste-attributes | Not implemented | No paste path for Retime | — |
| 10 | Blend selectable in UI; source-rate advisory badge | Partial | Blend honoured in render/cache but UI toggle is Nearest↔Flow only (`controls.rs:575,586`); no advisory badge | — |
| 13.1/13.2 | AE Time Remap import / Vegas mapping | Not implemented / future | Conversion maths exists (`from_source_keyframes`) but no importer drives it | — |
| impl notes | AE golden-value fixtures; 10⁶-edit denominator property test | Not found | Only self-consistency tests + a single overflow case | — |

---

## 05-ARCHITECTURE

**Findings.** The document-model foundations are genuinely built (immutable snapshots +
arc-swap, command journal, content-hash keys, DAG lowering, epoch cancellation), and
the single most load-bearing rule — engine crates never depend on the UI — holds in
every `Cargo.toml`. The process/thread architecture, plugin isolation, and half the
crate graph are not built as specified.

| § | Claim | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| §1 | Crates `lumit-time`, `lumit-expr`, `lumit-ofx`, `lumit-lfx` | Not implemented | None exist; undocumented `lumit-flow`, `lumit-text` do | ✅ Doc-synced · 05 §1: split into "exist today (v1)" vs "reserved for later" tables; `lumit-time` folded into `lumit-core`; `lumit-flow`/`lumit-text` now listed |
| §1.1 | `FrameSource`/`KernelExecutor`/`CacheStore` trait seams in eval | Not implemented | Only `SourceStamper` exists; the pixel pass those traits serve is unbuilt (`graph.rs:3-4`) | ✅ Doc-synced · 05 §1.1: eval now stated to depend only on `lumit-core` via `SourceStamper`; pixel pass + those seams marked reserved-not-built |
| §1.1/§9 | wgpu isolated to one owning crate | Violated | Direct dep of `lumit-gpu`, `lumit-flow`, plus `egui-wgpu` in `lumit-ui` | ✅ Doc-synced · 05 §1.1: recorded as a known deviation (wgpu direct in `lumit-gpu`+`lumit-flow`, surface use in ui/app) |
| §2 | Work-stealing worker pool (cores−1) | Not implemented | No pool; ad-hoc `thread::spawn` per job in `lumit-ui` | — |
| §2 | Dedicated GPU-submit thread owns the queue | Not implemented | Submission via eframe's shared device/queue on the UI thread (`shell/gpu.rs:207,281-294,865-909`) | — |
| §2 | Persistent per-stream decode threads + bounded queues | Not implemented | One-shot spawn per request (`previewing.rs:686`, `media.rs:53`) | — |
| §2 | Audio-render thread evaluating ahead; lock-free cpal ring | Partial | Audio pre-mixed on a spawned thread; callback reads via `RwLock::try_read` (`lumit-audio/src/lib.rs:133`) — non-blocking but not lock-free, no ring | — |
| §5 | Texture pool + resource governor; "nothing allocates ad hoc" | Not implemented | No pool/governor; 512 MB display budget hardcoded ("governor makes this adaptive later", `shell/gpu.rs:238`) | — |
| §5 | Device-lost recovery (epochs, recreate, replay) | Not implemented | No `DeviceLost` handling anywhere | — |
| §5 | CUDA accelerator path | Not implemented (future) | WGSL-only flow | — |
| §5 | Scheduler-inserted CPU bridge nodes | Not implemented | CPU references exist as oracles only | — |
| §7 | Out-of-process OFX/LFX; sandboxed expressions | Not implemented | See 12 | — |
| §1/§8 | Crash-handler process | Not implemented | None installed | — |
| naming | Nova / Pulsar | Contradicted | Code: Togi / (unnamed); Nebula matches | ✅ Done · d87bcf4 |

---

## 06-RENDER-PIPELINE

**Findings.** The pixel-producing core is real and impressively complete — linear fp16
premultiplied compositing, the full AE blend set in correct colour domains, the matte
model, cameras/2.5D, collapse, adjustment staging, motion blur (per-layer +
accumulation), a thorough content-hash cache key, and export reusing the exact preview
engine (K-031 holds structurally). But it lives in `lumit-ui`, not the specified
eval-graph executor, and several headline claims are contradicted.

| § | Claim | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| 1.1 | Comp lowered to an eval-graph that is demand-pulled to pixels | Partial | DAG compiles but the pixel pass is deferred (`graph.rs:3`); real renderer is the imperative draw-list in `export.rs:1120` / preview shell | — |
| 2.1–2.2 | ROI/DoD protocol, texture pool with refcounts, macro-tiles, per-node CPU fallback (K-019) | Not implemented | None of it exists; nodes render whole comp-sized textures | — |
| 3.1 | Project-wide 8/16/32 bpc (K-069) | Not implemented | Hard-const `Rgba16Float`; depth not in the cache key | — |
| 3.2 | Hardware decode lands NV12/P010 on GPU; one compute pass does colour matrix + chroma upsample + linearise; "no CPU round trip" | **Contradicted** | Decode is CPU ffmpeg → swscale to sRGB RGBA8 (`decode.rs:147-167`) → CPU Vec → upload → sRGB-format linearise (`colour.wgsl`). Full CPU round trip; no HW decode; no CM compute pass | — |
| 3.2 | Per-footage colour-space tag; video defaults Rec.709/BT.1886 | Not implemented | Everything treated as sRGB; no tag, no BT.1886 | — |
| 3.3 | `ColourTransform` display interface (sRGB/Rec.709/linear, exposure, channel isolation) | Partial | Single fixed sRGB encode via target format; no abstraction, no options | — |
| 3.3 | K-031 enforced by CI golden viewer-vs-export comparison | Partial | Structural parity real (shared engine); no such golden test exists (only the sRGB round-trip) | — |
| 3.4 | Host-managed `wants_straight_alpha` unpremult/premult fusing | Contradicted (mechanism) | Each effect shader hand-rolls the wrap (`fx_contrast.wgsl:28` etc.); no host flag | — |
| 3.5a | Luma mattes use Rec.709 Y of the **sRGB-encoded** signal | **Contradicted** | Luma dot-product runs on linear premultiplied values (`composite.wgsl:214,252`) — luma mattes will read differently than specified | ✅ Done · 22a9ddd · tested · 👁 eyeball luma mattes on real footage |
| 3.5 | Stencil/Silhouette/Alpha-add modes "ship in v1" (table) | Future-by-design | Absent; the doc's own prose reclassifies them post-v1, contradicting its table | — |
| 4 | Adaptive motion-blur sample count, fp32 accumulator, sub-frame re-render | Partial (self-flagged) | Fixed comp samples; working-format mean; source drawn once at N placements | ◑ Partial · fp32 accumulator DONE both paths (93c5e90, 6fcf695, tested); the "adaptive sample count" part of this row is still fixed-N |
| 5.1 | Three cache tiers; playback promotes disk→RAM→VRAM | Partial | RAM + disk only; no VRAM tier (`lumit-cache/src/lib.rs:5`) | — |
| 5.1/5.4 | fp16 planes in RAM/disk; LZ4 fp16 + colourspace marker on disk | Contradicted | Disk stores 8-bit sRGB RGBA LZ4 only (`disk.rs:27,171`); fp16 noted future | — |
| 5.3 | Cost-aware GreedyDual eviction, pinning, demotion | Not implemented | Plain LRU (RAM) and oldest-mtime (disk) | — |
| 5.4 | `index.db` SQLite index | Not implemented | Filesystem `is_file` check ("later speed-up", `disk.rs:8`) | — |
| 6.2 | Degradation ladder + status readout | Partial | Only a draft-width cap (`mod.rs:59`); no ladder/readout | — |
| 6.4 | Render-ahead ring 8–16 + ~150 ms pre-roll | Partial (unwired) | Simple lookahead window; `FrameRing` never wired; no pre-roll | — |
| 6.5 | Cached/Realtime preview toggle (K-030) | Not implemented (unwired) | `RealtimeController` tested, never referenced by UI | ◑ Slice built ec2abe2 · ⚠ UNTESTED — can't build lumit-ui here, CI compiles only; needs your run under a heavy comp. Cost = CPU composite time (partial proxy); render-ahead ring (06 §6.4) still separate · 👁 verify adaptation |
| 7.2 | Export compiler with baking (K-024) | Not implemented | Export renders live per frame; no flatten/bake stage | — |
| 7.4 | Output colour transform per preset; ProRes/DNxHR; straight/premult option | Partial | sRGB display readback → YUV only; no ProRes/DNxHR, no alpha option | — |
| 7.5 | Presets incl. 1440p60 + Master | Partial | 1080p60 / 4K60 / Vertical / Custom only (`export.rs:72-97`) | ◑ Partial · ✅ CI-green (macOS + Windows) · added the **YouTube 1440p60** preset (2560×1440 HEVC, VBR 25/35 — matches doc §7.5) across the enum/`ALL`/label/params/filename + the export menu. **Master** left out: it needs DNxHR/ProRes, which the encoder doesn't have (see 7.4) — a codec feature, not a data add |
| 7.5 | Vertical one-click reframe (draggable crop / pillar-fit) | Not implemented | Plain letterbox resize (`export.rs:1750`) | — |
| 8 | Scopes are GPU compute; "never computed on the CPU"; <0.5 ms at 4K | **Contradicted** | Scopes are CPU byte-buffer traces (`shell/scopes.rs:13-24`), self-noting the K-096 deferral | — |
| 5.1 | Device-loss drops VRAM, recovery from lower tiers | Not implemented | No loss handling; no VRAM tier to lose | — |

Also worth knowing: the GPU tests skip when no adapter is present, so CI (GitHub
runners) may not exercise the compositor kernels at all.

---

## 07-UI-SPEC

**Findings.** A substantial subset is delivered: the tiling dock with pop-out, the
timeline outline with the five AE column clusters, keyframe-lane editing with marquee,
the graph editor with Retime lenses, all four scopes, command palette, settings window,
Effects & Presets with drag-to-apply, and the hierarchy panel. The **keymap is the
largest gap** (~15 of ~45 documented bindings exist, several wired differently), and
whole documented subsystems are absent.

| § | Claim | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| 1.1 | Backtick maximise/restore panel under pointer ("MUST") | Not implemented | No handler | — |
| 1.1 | Per-panel menu (Undock/Close/Maximise/Help) | Partial | Pop-out button/right-click only (`dock.rs:311,223`) | — |
| 1.2 | Five highlighted drop zones ("MUST") | Future-by-design | Doc's own v1 note: approximated by egui_tiles | — |
| 1.3 | Ctrl-drop floating windows; float hosts own splits | Partial | Single-panel pop-out only; no Ctrl gesture, no floating frame tree | — |
| 1.4 | Workspaces: 4 presets, switcher, Alt+Shift+1…9, user CRUD | Not implemented | Only default layout + Reset ("presets arrive with the panel set", `app_update.rs:754-761`) | — |
| 1.5 | Persisted per-comp viewer/timeline state, viewer locks, column state | Partial | Dock tree/theme/divider persist; the rest doesn't | — |
| 2.2 | Viewer bar: magnification dropdown, Ctrl+scroll zoom, Shift+/ fit | Partial/contradicted | Static "Full · Fit" text (`panels.rs:80-109`); zoom is plain scroll (`overlays.rs:494-525`); no dropdown/steps | — |
| 2.2 | Preview-res dropdown stored per comp | Partial | Dropdown exists (`overlays.rs:603-629`) but is app-state, not per-comp | — |
| 2.2 | Channel view; transparency grid; wireframe/guides menus; rulers Ctrl+R; ROI; CM badge; degradation badge; bg swatch; click-to-type time | Not implemented | None present (CM is static text) | — |
| 2.3 | Transform gizmo (move/scale/rotate, Shift uniform, snapping) | Partial/contradicted | Only anchor crosshair + mask/pen overlays (`overlays.rs:144-242`); transform edited numerically | — |
| 2.4 | Motion paths in Viewer | Not implemented | No overlay | — |
| 2.6 | Viewer locks | Not implemented | Padlock is only the aspect lock | — |
| 3.1 | Project panel columns, sortable; hover-scrub thumbnails; Ctrl+F | Contradicted / not implemented | Name-only tree (`panels.rs:694-799`); static thumbnail; no Ctrl+F | — |
| 3.2 | Interpret footage dialogue | Not implemented | Nothing anywhere | — |
| 3.3 | Proxy badges/toggle; missing-footage badge + relink flow | Not implemented | Only an "unreadable: {e}" line (`panels.rs:483-488`) | — |
| 4.1 | Markers ribbon interactions (create/drag/labels/layer rows) | Partial/contradicted | Markers drawn only (`panel.rs:254-273`); add via menu | — |
| 4.1 | Work-area drag ends + double-click reset | Partial | B/N keys work; band not draggable | — |
| 4.1 | Cache bar three tiers | Partial | RAM + disk drawn only (`panel.rs:190-215`) | — |
| 4.2 | Full column set (shy, quality, adjustment, source toggle, Alt-click) | Partial | Matches K-168 shipped set; deferred set absent (doc self-flagged) | — |
| 4.3 | Ctrl+click adds keyframe; Alt+drag scales timing; U/UU reveal | Partial | None of those three; lane select/drag/marquee do exist | — |
| 4.4 | Clip thumbnails, roll edits, per-clip source menu | Partial | Sub-bars + speed % + edit ticks only | — |
| 4.4 | Razor tool on `C` | Contradicted | No razor tool; cut is Ctrl+Shift+D at playhead (`app_update.rs:880`) | — |
| 4.5 | Snapping to edit points/in-out/keyframes/playhead/work area; Ctrl suspend; indication | Partial | Markers/beats snapping + whole-frame magnet only | — |
| 4.6 | `=`/`-`/`Shift+=`/`\` zoom keys; Ctrl-scrub audio; edge-follow scroll | Partial | Wheel routing exists; keys unbound; no audio scrub | ◑ Partial · ✅ CI-green · bound the **zoom keys** — `=`/`Shift+=` zoom in, `-` out, `\` fits — in `global_shortcuts`, same 1.4×/1–400% math as the bottom-bar buttons. Ctrl-scrub audio and edge-follow scroll remain |
| 4.7 | `[`/`]`, Alt+[/], Ctrl+D | Not implemented | Unbound; duplicate via menu only | ✅ Done · CI-green (macOS + Windows compiled the keys; coverage gate ran the span-maths tests) · all bound in `global_shortcuts`: **Cmd/Ctrl+D** duplicate; **`[`/`]`** move the selected layer's in/out to the playhead; **Alt+`[`/`]`** trim that edge. The AE span maths live in `lumit_core::ops::edit_layer_span` (4 unit tests); the UI resolves selection + playhead and commits `SetLayerSpan` · 👁 try the layer-edit keys |
| 5 | Graph toggled by Shift+F3; per-property include toggle | Contradicted | Button in timeline row (`bottom_bar.rs:70-84`); selected-property only | ◑ Partial · ✅ CI-green · **Shift+F3** now toggles the graph editor via a new cross-platform `global_shortcuts` handler (the bottom-bar button stays too). The per-property include toggle (graph still shows the selected property only) is unchanged |
| 5.1 | Acceleration graph; auto/stacked/ghosted views | Not implemented | Value + Speed lenses only | — |
| 5.3 | Ease-in/out `Shift+F9`/`Ctrl+Shift+F9`; auto-bezier; numeric keyframe entry; `F` fit; box scale | Partial | `F9` easy-ease + Linear/Bezier/Hold buttons only | — |
| 6 | Effect Controls solo/reset/rename/copy-to-layer | Partial | Panel + enable/reorder/eyedropper exist; those extras not evident | — |
| 7 | Double-click apply; drag-onto-Viewer; favourites; hover descriptions | Partial | Drag-to-row/EC + click-apply presets exist; rest deferred (K-101) or absent | — |
| 9 | Preview panel/transport: loop modes, fill-cache, mute, quality toggle, Cached/Realtime | Not implemented | No `Preview` panel type (`mod.rs:55-63`); bare play/pause only | — |
| 10 | Audio panel; beat controls UI; meters; per-layer waveform; `8` tap | Partial | Menu-only beat detect with 2 fixed sensitivities; comp waveform strip only; no meters/tap | — |
| 11 | Export queue list UI (reorder/retry/cancel per item), full custom controls | Partial | Background queue + dialog + tokens real; no queue-list UI; some documented controls dead/not built (doc self-notes) | — |
| 12 | Palette: comps/panels categories, badges, recent-first | Partial | Commands + effects only; unwired on macOS menu (`command_palette.rs:76-77`) | — |
| 13.1 | First-run screen | Future-by-design | K-006 post-v1 | — |
| 13.2 | Tooltips with shortcut text at 500 ms | Partial | Tooltips exist, no shortcut text, default delay | — |
| 14 | Focus cycling (Ctrl+F6), Tab traversal, arrow nav, Move-panel-to | Not implemented | None bound | — |
| 15 | Remappable keymap + conflict detection + AE preset; keymap settings page | Not implemented | Hardcoded bindings; no keymap page | — |
| 15 | J/K/L shuttle ×2/×4/×8; Home/End; the ~30 other listed keys | Contradicted / not implemented | J = single frame-step back; L = toggle play; K = pause; End unbound; Page/I/O/,/./*/Ctrl+M/P-S-R-T-A/E/M/U/X/Z/Y etc. all unbound | ◑ Partial · ✅ CI-green · bound **End** → last frame of the current preview via a new `AppState::preview_frame_count` (comp or footage); Home already sought frame 0. The shuttle multipliers and the ~30 other keys remain — a full remappable keymap is the real fix (see the row above) |
| 15 | macOS native menu matches keymap | Contradicted (minor) | Own accelerator set (`native_menu.rs:96`) | — |

---

## 08-EFFECTS

**Findings.** The strongest doc-to-code match in the set: all 26 Tier-1 effect
sections exist as 32 schema entries, each with resolve arm, CPU reference and WGSL
kernel, and the load-bearing shader algorithms (glow, flow MB, datamosh, radial blur,
matte key, DoF, spectral split) match the doc op-for-op. No code effect is missing
from the doc. The gaps are mostly doc-flagged deferrals, plus four genuine findings.

Genuine (not doc-flagged) findings:

| § | Claim | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| K-035 | Universal per-effect strength matte (none / own masks / any layer) | **Not implemented** | No slot on `EffectInstance`, no host plumbing anywhere in `fx/` or `lumit-gpu` | — |
| 3.10/§5 | Ships ≥40 grade presets, live engine-rendered thumbnails, `.kpreset` zip format | Not implemented / contradicted | `preset.rs` is a `.lumfx` plain-JSON save/load browser; no shipped presets, no thumbnails, no zip/embedded assets | ◑ Partial · ext→`.lumfx` 28a3752; shipped library + thumbnails still to build |
| 3.9 | Sharpen is "radius-free… fixed 3×3" | Contradicted (doc stale) | Code added a Radius param (T15; `builtins.rs:327`, `fx_sharpen_simple.wgsl:54`) — code is ahead of the doc | ✅ Done · 28a3752 (doc) |
| 3.1 | Flow engine is a "variational/patch-match hybrid" | Contradicted (wording) | Engine is DIS (Kroeger 2016) per `docs/impl/optical-flow.md`; §3.1's sketch wording is stale | ✅ Done · 28a3752 (doc) · K-169 |

Doc-flagged deferrals confirmed absent (to-be-implemented list): glow mip-chain +
Falloff/CA/Screen-recombine; shake Style presets/Triggered mode/Decay; fast-MB
Amount/Vector-source/Quality; flash Blend sub-param; LUT input-space/tetrahedral/
content-hash cache; echo Spacing + per-echo transform; matte-key spatial controls,
garbage masks, CC twirls, crops (K-155); vignette colour tint; curves/full white
balance (Tier 2); all 18 Tier-2 effects (§4, post-v1).

---

## 09-AUDIO

**Findings.** The sync toolkit is real: cpal output with the audio clock as master, a
non-allocating RT callback, multi-source mixing, spectral-flux beat detection faithful
to the impl note, beat markers + snapping, live waveform, AAC export. Much of §5–§7 is
missing or hardcoded.

| § | Claim | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| 3.1 | Master limiter: hard safety clip at −0.3 dBFS true peak | Contradicted | Plain ±1.0 sample clamp (`mix.rs:49-51`); no true-peak logic | ◑ Partial · ✅ CI-green (macOS + Windows built lumit-audio and passed the test) · `mix_stereo` now clamps to ±`MASTER_CEILING` (−0.3 dBFS = 0.96605) with a both-polarity regression test; doc 09 §3.1 updated to say sample-peak now, true inter-sample-peak (BS.1770) future · 👁 review by ear |
| 3.1/6 | Per-layer gain (volume keyframes) | Not implemented | Mix gain hardwired `1.0` (`export.rs:238`); no volume property in the model | — |
| 6 | Fade-in/out commands | Not implemented | No fade code | — |
| 6 | Audio solo silences non-soloed audio | Partial | Mute works; audio path never consults `solo` — soloing does not silence other audio | ✅ Done · afc7136 · CI-green · 👁 review by ear |
| 6 | Audio layer kind; detach-audio | Not implemented | Audio only via footage layers; no detach command | — |
| 3.4 | Audio scrubbing (windowed grain, on by default) | Not implemented | Scrubbing pauses audio (`playback.rs:268`) | — |
| 3.2 | Device-change stream rebuild ("MUST NOT desync") | Not implemented | One stream built once; error callback is a no-op | — |
| 3.1 | Output latency compensation | Partial (deferred in code) | `clock_seconds` raw; comment defers to ring-buffer work (`lib.rs:119-124`) | — |
| 2/3.1 | 48 kHz session rate, RAM ring, lazy decode, background import pass | Partial/deviates | Playback resamples to device rate; whole-file synchronous decode into RAM (`audio.rs:34`); export does use 48 kHz | — |
| 4 | Multi-tier sidecar peak files (min/max/RMS at 3 zooms, content-hash keyed) | Not implemented | Live-computed single 2048-bucket strip | — |
| 4 | Waveforms on layers and inside clips | Not implemented | One comp-wide strip only (`timeline/panel.rs:305-357`) | — |
| 5 | Sensitivity slider 0–100 | Partial | Two hardcoded presets (1.5 / 1.1) via menu | ◑ Partial · ✅ CI-green (macOS + Windows + coverage gate ran the mapping test) · added `lumit_audio::beat::delta_from_sensitivity(0–100→δ)` (anchored 50→1.5, 70→1.1; unit-tested) and replaced the two-item Detect-beats menu with a 0–100 slider + Detect button (`beat_sensitivity` app-state field, default 50). Keyboard quick-detect shortcuts still use the two fixed densities · 👁 try the slider |
| 5 | BPM confirm/type + phase; grid-fill missed beats | Partial | Auto grid-snap + BPM display only; no UI, no fill | — |
| 5 | Tap tempo | Not implemented | Nothing | — |
| 7 | Retime mutes audio with a badge | Not implemented | Retimed footage audio plays un-retimed, un-muted, un-badged | — |
| 8 | Loudness normalisation (−14 LUFS) | Future-by-design | Doc marks post-v1; absent | — |
| 9 | The Composer | Future-by-design | Doc marks future; absent | — |

---

## 10-FILE-FORMAT

**Findings.** The `.lum` container, manifest-first ordering, atomic save
(temp+fsync+rename), rotating autosaves, torn-tail-tolerant crash journal, and
pervasive unknown-field preservation are genuinely implemented and tested. Deltas:

| § | Claim | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| 1 | `thumbs/` in the container | Not implemented | Only manifest + project.json ("Phase 0 scope", `lumit-project/src/lib.rs:2`) | — |
| 1 | Two saves byte-identical (stable key order) | Partial/unverifiable | Pretty-printed, but no determinism guarantee or test | ✅ Done · verified deterministic by construction (no `HashMap` in serialised structs; `extra` is a sorted `serde_json::Map`) and locked in with a regression test asserting two saves + open→save reproduce identical `project.json` bytes (7 project tests green locally) |
| 1 | Newer reader **migrates** older files | Not implemented | Only refuse-too-new gating (`lib.rs:114`); no migration step | — |
| 1.1 | Enums serialise lower-kebab | Contradicted | Default PascalCase/externally-tagged everywhere (e.g. `"channel":"Alpha"`) | ✅ Done · 5bb73ff (doc) |
| 2 | Fingerprint + 4-step relink + sibling auto-relink | Partial / not implemented | No fingerprint on `MediaRef`; no relink resolver | — |
| 2 | Collect-for-sharing command | Not implemented | No such function | — |
| 3 | Full sidecar tree (disk-cache/proxies/peaks/flow/index) | Partial | Journal, media-index and presets dirs only | — |
| 4 | Autosave every N minutes + before risky ops | Partial | Rotation/writing real; timer and risky-op triggers live outside the crate (cadence exists in UI settings; risky-op trigger not found) | — |
| 4 | Journal truncated on successful save; recovery dialogue | Partial | ~~`clear` exists but nothing wires clear-on-save~~; recovery replay exists in UI, the three-way offer dialogue does not | ✅ Done (was half-wrong in audit) · clear-on-save **is** wired (`AppState::save` truncates on success, as do `new_project`/discard-recovery). And the missing **third** recovery option is now built: `lumit_project::latest_autosave` (unit-tested locally) + an "Open autosave" button in the recovery modal → `recover_from_autosave` (clears the interrupted journal, opens the autosave, keeps the project path, marks dirty). ✅ CI-green (macOS + Windows compiled the modal/flow; `latest_autosave` test green) · 👁 verify the recovery flow end-to-end |
| 5 | Preset extension `.kfxpreset` (zip) | Contradicted | `.lumfx` plain JSON (`preset.rs:25`) | ✅ Done · 28a3752 (doc) |
| 5 | "New from template" mode | Not implemented | No template-open mode | — |
| 6 | AE Bridge / Lottie / OTIO interchange | Not implemented / future | Doc marks Lottie/OTIO future; Bridge import absent (see 11) | — |

---

## 11-AE-IMPORT

**Findings.** **The After Effects importer does not exist in any form** — not a stub,
not a mapping table, not a menu entry. Searches for `aep`, `bodymovin`, `lottie`,
`zxp`, `extendscript`, `lum-bundle`, `ae-effect-map`, `RIFX`, "Lumit Bridge" across
`crates/` return zero hits. `lumit-project` (named by §7 as the parser's home) contains
only the `.lum` container. The code even self-documents the absence
(`retime.rs:719`: AE import "is not present yet"). What *does* exist is the Lumit-side
AE-compatible semantics the doc leans on: AE keyframe maths (tested for internal
consistency, not against AE golden fixtures), the Time-Remap→Retime conversion maths,
the full AE blend-mode set, the matte model, and shutter/camera defaults.

| § | Claim | Status |
|---|---|---|
| §1–2 | All three import routes (Bridge `.zxp` panel / direct `.aep` / Lottie) | Not implemented |
| §2.3 | `.lum-bundle` format with `ae` namespace | Not implemented |
| §2.5 | Footage relink chain on import | Not implemented |
| §4 | Fidelity matrix (lossless/mapped/placeholder/unsupported) | Not implemented |
| §5 | `ae-effect-map.toml` + golden-frame tests | Not implemented |
| §6 | Placeholder badge "not rendered — imported from AE" | Not implemented (the inert `Placeholder` effect namespace exists, nothing populates it) |
| §7 | Rust RIFX `.aep` parser in `lumit-project` | Not implemented |
| §8 | Lottie/bodymovin importer | Not implemented |
| §9 | Import report panel | Not implemented |

---

## 12-PLUGINS

**Findings.** **Essentially none of this document is implemented.** No `lumit-ofx`,
`lumit-lfx` or `lumit-expr` crate; no FFI, plugin discovery, out-of-process broker,
parameter bridge, quirks table or validator; no JS engine of any kind, no expression
field on properties, no `wiggle`/`loopOut`/`valueAtTime`. The only trace is the
four-variant `EffectNamespace` enum (`model.rs:401-413`) used as a cache-key
discriminant. This matches the doc's own §1 ("plugins ship after the main
application") — but every §2–§5 mechanism is to-be-implemented:

| § | Area | Status |
|---|---|---|
| §2 | OFX host (suites, actions, contexts) | Not implemented |
| §2.2 | Parameter bridging | Not implemented |
| §2.3 | Out-of-process plugin server, watchdog, shared memory (K-066) | Not implemented |
| §2.4–2.6 | GPU render suites; quirks table; discovery | Not implemented |
| §3 | LFX C ABI + validator + template repo | Not implemented |
| §4 | Expression engine (QuickJS-ng), API subset, determinism, perf model | Not implemented |
| §5 | Security model | Not implemented (nothing to isolate) |

---

## 13-PERFORMANCE-RULES

**Findings.** Almost entirely aspirational. The two central enforcement claims — that
budgets gate merges in CI and that a resource governor owns memory with an ordered
degradation ladder — have no implementation. There is no benchmark harness, no
criterion benches, no reference/stress comp fixtures, and no perf job in CI, so no
budget can fail a build. What is real: epoch cancellation, per-effect graceful no-op
fallbacks, the content-hash quality axis, the two-tier cache with a dedicated disk-IO
thread, audio-clock-master playback, and per-effect CPU references. The pure playback
decision core (ring/lookahead/adaptive controller) is built and tested but unwired.

| § | Claim | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| §7.3 | Budgets gate merges; >10% regression fails the build | Contradicted | `ci.yml` has no perf job, no baselines | — |
| §7.3 | Headless benchmark harness emitting B3–B8/B11 | Not implemented | No harness, no `benches/`, no criterion | — |
| §1/§2.1/§7.3 | Reference comp + deterministic stress fixture in repo for CI | Not implemented | No fixtures or builder | — |
| §3 | Resource governor (VRAM/RAM budgets, DXGI subscription, ledger, pools) | Not implemented | "Governor" exists only in comments (`shell/gpu.rs:238`, `lumit-cache/src/lib.rs:4`) | — |
| §4 | 7-step degradation ladder, hysteresis, status chip | Not implemented | No ladder/chip; per-effect no-ops are unrelated | — |
| §4 step 3 / K-030 | Adaptive resolution during playback | Partial (unwired) | `RealtimeController` tested, unused by `comp_playback_tick` | ◑ Slice built ec2abe2 · ⚠ UNTESTED — can't build lumit-ui here, CI compiles only; needs your run under a heavy comp. Cost = CPU composite time (partial proxy); render-ahead ring (06 §6.4) still separate · 👁 verify adaptation |
| §2 B5–B7 | Frame pacing via render-ahead ring | Partial (unwired) | `FrameRing`/`Lookahead` unused; single-frame prefetch instead | — |
| §5 | GPU device-loss recovery + DRED + repeated-loss CPU fallback (B9) | Not implemented | No handling anywhere | — |
| §6 | Effect contract: cost class used by scheduler, scratch ceilings, per-effect benchmarks | Partial | Cost class declared but unused; no scheduler; no memory benchmarks | — |
| §7.1 | Per-node profiler, render-time column, recording mode | Not implemented | No GPU timestamps or spans | — |
| §7.2 | Diagnostics ring log + export action | Not implemented | Nothing | — |
| §2 | All numeric budgets B1–B11 / S1–S6 | Unverifiable | Nothing measures them | — |

---

## 14-ENGINEERING-RULES

**Findings.** The strongest rules hold: panic lints configured and CI-enforced (all
`#[allow]`s sit on test modules), `unsafe_code` denied with sanctioned exceptions only
in `lumit-media`, exact rational arithmetic with typed overflow errors, typed error
enums, determinism hygiene (no clocks in evaluation, canonicalised -0.0), snapshot
isolation, no `unsafe impl Send/Sync`, no async (so no locks-across-await). But
several binding rules are violated or their promised enforcement doesn't exist.

| § | Rule | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| §2 | Authoritative time MUST NOT be `f64`; timebase mixing must not compile | **Violated** | The whole eval layer is `f64`: `comp_frame_key(…, t: f64)` (`lumit-eval/src/lib.rs:61,67`), bare-f64 local-time arithmetic (`lib.rs:144`), `Property::value_at(f64)`, `Retime::evaluate(f64)`; the cache key is hashed from f64s | — |
| §2 | `RationalTime{num:i64,den:i32}` in crate `lumit-time` | Partial | Sound newtypes exist, but as `Rational{i64,i64}` in `lumit-core/src/time.rs`; no `lumit-time` crate | — |
| §2 | `FrameIndex(i64)` distinct type | Not implemented | Frames are bare `i64`/`usize` | — |
| §4 | Deny `todo!`/`unimplemented!`/`indexing_slicing`/`arithmetic_side_effects` | Not implemented | Workspace lints deny only unwrap/expect/panic (`Cargo.toml:10-13`) | ◑ Partial · added `todo`/`unimplemented` denies to workspace lints (zero existing violations tree-wide; buildable engine crates clippy-clean locally). `indexing_slicing`/`arithmetic_side_effects` still deferred — they need a hot-path sweep first |
| §4 | No panicking indexing in hot paths | Violated | `&self.nodes[id]` (`graph.rs:87`); realtime audio callback indexes `buffer.samples[…]` (`lumit-audio/src/lib.rs:146-147`) | ◑ Partial · `graph.rs` node accessor now returns `Option<&Node>` via `.get()` (eval builds + 46 tests green locally) — panic path removed. Audio-callback index still open (can't build lumit-audio here) |
| §4 | `DeviceLost` recoverable, never a crash | Not implemented | No handling | — |
| §5 | Pooled frame allocations accounted to the governor | Not implemented | Ad-hoc `Vec`s; no pool/governor | — |
| §5 | No unbounded queues without a decision entry | Violated | `mpsc::channel()` (unbounded) for beats/audio/comp-audio (`playback.rs:151`, `previewing.rs:709,795`) — drained latest-wins, but unbounded and unlogged | ✅ Done · logged as K-170: unbounded is deliberate (latest-wins mailboxes drained per frame + self-throttling work queues), with a documented bounded-`sync_channel` escape hatch if profiling shows growth. Rule now satisfied (decision entry exists) |
| §5 | Journal compaction story | Partial | Undo journal is an ever-growing `Vec` (`store.rs:23-24`) | — |
| §6 | Golden-frame EXR tests per platform | Not implemented | No EXR fixtures, no CI step | — |
| §6 | cargo-fuzz on `.lum`/journal/OFX on a schedule | Not implemented | No fuzz dir or job | — |
| §6 | Perf regression gates | Not implemented | See 13 | — |
| §6 | Engine-crate coverage gate | Partial | Only `lumit-core` + `lumit-project` gated at 75%; other engine crates excluded | ✅ Done · gate extended to all six FFmpeg-free engine crates (`+eval/cache/flow/text`) and threshold ratcheted 75→80 (measured 93.7% combined locally via `cargo llvm-cov`; lowest crate 87%). `lumit-gpu` held out while its kernel tests skip on adapterless runners |
| §7 | `rust_2024_idioms`, `clippy::pedantic` | Not implemented | Absent from workspace lints | — |
| §7 | i18n from day one | Not implemented | Raw string literals everywhere | — |
| §7 | CI greps glossary banned terms | Not implemented | No such step | ◑ Investigated · codebase is compliant in practice — every banned-term hit is a glossary-sanctioned context (the K-076 "Velocity" lens label, "track matte", audio "track" in media, AE-context "Time Remap"); no CTI. A real CI gate needs a curated allowlist + user-facing-string scoping (most terms — line/render/event/bin/clip — are too context-sensitive to grep raw), so it's a scoped design task, not a blind add. **Owner decision needed on term list + scope** 👁 |
| §7 | FFI `catch_unwind` + layout tests | Partial/unverifiable | No `catch_unwind`; likely moot (no registered callbacks) | — |
| §8 | `tracing` spans throughout; per-node timings; crash capture (Crashpad); opt-in telemetry | Not implemented | `tracing` not a dependency of any crate; no crash handler | — |
| §9 | `cargo deny` in CI | Not implemented | No `deny.toml`, no job | — |
| §9 | Pinned toolchain; edition 2024 | Violated | No `rust-toolchain.toml` (CI uses unpinned stable); every crate is edition 2021 | — |

---

## 15-DESIGN

**Findings.** The best-implemented doc in the set. The `Theme` struct is a real single
colour source; the dark ramp, text, hairline, accent and layer hex values match the doc
exactly; the no-hex CI lint genuinely passes (theme.rs confirmed the only `.rs` with
colour literals); seven schemes, the Sharp/Round shape axis, animation levels, overrun
hatching, Iconoir icons and the voice rules (no exclamation marks, no American
spellings) all check out. Gaps:

| § | Claim | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|---|
| §6.3 | Cache bar: 3 tiers, all olive/mint family (vram `#5fcfae` / ram `#3f9077` / disk `#2c6353`) | Contradicted | Two tiers; RAM drawn `theme.success` (`#5fcfae` — the doc's *VRAM* value) and disk steel-blue `#5f93b8` (`theme.rs:253`, deliberately following doc 06 §5.6 instead) | ✅ Done · 5bb73ff (doc) |
| §4.1 | Theme fields `disabled`, `fill_tonal`, `keyframe`, `cache`, `marker`, `overrun_hatch`, `waveform`, `selection`, `shadow_float` | Partial | None exist as tokens (`theme.rs:19-71`); semantics derived ad-hoc from existing roles; `disabled`(cloud) and `fill_tonal`(oat) simply absent | ✅ Doc-synced · 15 §4.1: added a "v1 status of this struct" note listing shipped roles (+ the code's own `scope`/`cache_disk`) vs the ad-hoc/reserved ones, framed as the target shape with token-splitting as standing direction |
| §6.1 | 11 layer-type colours | Partial | 6 implemented (`theme.rs:196-203`) — matches the 6 existing layer kinds, but the doc's table over-claims | ✅ Doc-synced · 15 §6.1: table split into the six shipped tokens + a note that Adjustment (7th kind) borrows `layer.solid` in v1, and a reserved table for the unmodelled Shape/Null/Audio/Light kinds |
| §1.1/§7.1 | Type stack: Schibsted Grotesk, Source Serif 4, Inter, JetBrains Mono for all numbers ("no exceptions") | Partial/violated | Only Inter is embedded (`assets/fonts/`, `theme.rs:583-599`); mono-for-numbers uses egui's default monospace; headings are Inter | — |
| §7.1 | Documented type scale (20px transport, 13px values, 24px+ display) | Partial | Applied styles are Heading 16 / Body 12 / Button 12 / Small 11 / Mono 12 (`theme.rs:685-704`) | — |
| §9 | Contrast floors CI-checked from theme values | Not implemented | No contrast test or job | — |
| §9 | AccessKit wired from day one (roles, landmarks, timeline tree) | Not implemented | Zero accesskit references | — |
| §10 | Errors as fig-tinted banners | Partial | Single status line tinted `warning` (kraft), reused for success text (`app_update.rs:216,922-927`) | ✅ Done · afc7136 · CI-green · 👁 review look |

---

## 16-ROADMAP

**Findings.** Phases 0–3 are substantially built (skeleton, compositing core, Retime
flagship, effect suite). Phase 4 is essentially unstarted; 5–6 are future by design.
The standing rules over-claim.

| Item | Status | Evidence / what's missing | Resolution / next step |
|---|---|---|---|
| Phase 1 "three-tier cache" | Partial | Two tiers (see 06/13) | — |
| Phase 1/2 masks usable for transitions | Partial | Static-only masks block the Gate-3 animated masked transition | — |
| Phase 3 "smooth zoom" effect | Not found | No such effect in `fx/builtins.rs` (only Radial blur's Zoom mode) | — |
| Phase 3 Scopes | Partial | CPU implementation; GPU pass deferred (K-096 note in `scopes.rs:20`) | — |
| Phase 4: expressions, OFX host, LFX, AE Bridge/importer, migration first-run | Not implemented | See 11/12; no QuickJS, no host, no importer, no first-run screen | — |
| Phases 5–6 | Future-by-design | Doc marks them Ongoing / "specified when we get there" | — |
| Standing rule: "performance gates run in CI on every merge" | **Contradicted** | No perf job exists in `ci.yml` | — |
| Standing rule: every feature lands with tests + glossary compliance | Partial | Tests are widespread and a coverage gate exists (2 of 11 crates); glossary compliance not CI-checked; **and CI is currently red on `main`** (see cross-cutting findings) | ◑ Partial · coverage gate now covers 6 of 11 crates (all FFmpeg-free engine crates, ≥80%); the `main`-red compositor test is fixed on the PR branch (fp32 accumulator); glossary CI grep still open (needs careful scoping of context-sensitive terms) |
| Gates 0–3 runtime criteria (4K60 scrub, pixel parity, Twixtor-comparable flow, six-hour soak) | Unverifiable | Need hardware/runtime; no harness measures them | — |

---

## Method

- One focused auditor per doc (grouped for the small ones), each instructed to verify
  every claim against `crates/`, `.github/`, `scripts/` with file:line evidence and to
  distrust the docs' present tense. Findings above preserve their evidence citations.
- CI state pulled from GitHub Actions for `luminalmvm/Lumit` (workflow `ci.yml`, last
  30 runs of `main`); the failing test's panic message extracted from the job log.
- Local test verification in this (Linux, no-FFmpeg, no-GPU) container:
  `cargo fmt --check` clean; `cargo test` green for `lumit-core`, `lumit-project`,
  `lumit-eval`, `lumit-cache`, `lumit-flow`, `lumit-text` (301 tests). The
  FFmpeg-dependent crates (`lumit-media`, `lumit-audio`, `lumit-ui`, `lumit-app`)
  could not be built here (macOS-pinned FFmpeg path; container lacks FFmpeg 7.1) —
  their state is taken from CI, which runs them on macOS and Windows.
- GPU-dependent tests skip without an adapter, on CI runners as well as here — the
  compositor kernels' correctness claims rest on runs on GPU-equipped machines.
