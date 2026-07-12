# Phase 0 kickoff: the cold-start build order

For the first implementation session. Everything here follows [16-ROADMAP.md](../16-ROADMAP.md)
Phase 0 and [05-ARCHITECTURE.md](../05-ARCHITECTURE.md); this note just sequences the work
into slices that each end runnable, so progress is visible from hour one and nothing is
built floating in the air. Do the slices in order; do not start a slice until the previous
one runs.

## 0. Workspace scaffold

```
kiriko/
├── Cargo.toml                 # [workspace], resolver = "2"
└── crates/
    ├── kiriko-core/           # Rational, timebases, document model, ops/journal
    ├── kiriko-media/          # rsmpeg wrapper, frame index, decode pool
    ├── kiriko-gpu/            # device, pool, blit, NV12 shader
    ├── kiriko-audio/          # cpal stream, clock, mixer
    ├── kiriko-cache/          # governor + RAM tier (VRAM/disk tiers arrive Phase 1)
    ├── kiriko-eval/           # (Phase 1 — create the crate, leave it nearly empty)
    ├── kiriko-project/        # .kir container, autosave, journal persistence
    ├── kiriko-ui/             # egui shell, theme, docking, panels
    └── kiriko-app/            # bin: wires everything
```

Dependencies (take the latest compatible releases; majors as of writing): `wgpu` 24+,
`egui`/`eframe`/`egui_dock` matching set, `winit` via eframe, `rsmpeg` (+ FFmpeg 7 shared
libs per [media-io.md](media-io.md) §1), `cpal`, `crossbeam-channel`, `arc-swap`, `rayon`,
`serde`/`serde_json`, `zip`, `uuid` (v7 feature), `proptest` + `insta` (dev). Workspace
lints from day one: `unwrap_used = "deny"` and `expect_used = "deny"` for the engine crates
([14-ENGINEERING-RULES.md](../14-ENGINEERING-RULES.md)) — adding lints later means a
cleanup commit nobody enjoys.

CI on the first day, before features: `cargo clippy --all-targets -- -D warnings`,
`cargo test`, `cargo fmt --check`, on Windows + macOS runners. The perf gates join in
Phase 1; the culture of red-blocks-merge starts now.

## Slice 1 — window, theme, docking (runs: empty shell)

eframe app; implement the theme struct from [15-DESIGN.md](../15-DESIGN.md) §tokens
(hex literals only inside `kiriko_ui::theme` — add the lint/CI grep immediately);
egui_dock layout with placeholder Project / Viewer / Timeline / Effect Controls panels in
the default Edit workspace arrangement ([07-UI-SPEC.md](../07-UI-SPEC.md)); workspace
save/restore to app settings. **Exit test**: panels drag/dock/float/restore; dark theme
audits against the token table; runs on the MacBook.

## Slice 2 — rational time + document model (runs: same shell, tested core)

`Rational` and the four timebases exactly per [rational-time.md](rational-time.md)
(property tests first — they are quick to write and catch the overflow bugs while the
code is a page long). Document model structs for Phase 0 scope only: Project, assets,
folders, one Composition, Footage layers with in/out (no properties/keyframes yet —
Phase 1). Operations + journal + undo/redo on the arc-swap snapshot pattern
([playback-scheduler.md](playback-scheduler.md) §3). **Exit test**: scripted edit
sequences undo/redo to identical snapshots; journal replays to the same hash.

## Slice 3 — .kir container (runs: New/Open/Save works)

[10-FILE-FORMAT.md](../10-FILE-FORMAT.md) §1–2 and §4: manifest-first zip, stable-order
project.json, unknown-field preservation (test it now — retrofitting is misery), atomic
save, autosave rotation, journal-based crash recovery. Wire File menu + recent projects +
the empty-project card. **Exit test**: the Gate-0 kill-and-recover drill passes
(`kill -9` mid-edit → relaunch → offered journal replay → nothing lost).

## Slice 4 — import + frame index (runs: Project panel fills)

rsmpeg wiring, the packet-scan frame index with VFR conform policy
([media-io.md](media-io.md) §2), fingerprints, sidecar layout, missing-footage state,
thumbnails. Import via dialogue and drag-drop. **Exit test**: index for an hour-long
ShadowPlay VFR capture builds in seconds, survives sidecar deletion, badge shows on VFR.

## Slice 5 — decode → Viewer (runs: you can SEE footage)

`kiriko-gpu` device + pool + NV12→linear shader + display-transform blit
([gpu-foundation.md](gpu-foundation.md) §1–3, §6; [media-io.md](media-io.md) §3–5,
baseline decode path only — no D3D11 interop yet). Viewer panel shows a selected footage
item; resolution picker (Full/Half/Third/Quarter) as true raster downsampling; zoom/fit;
scrub bar with exact seeking. **Exit tests**: the colour round-trip golden; 1000-random-
frame seek exactness on the test corpus; scrub feels immediate at Half on 4K.

## Slice 6 — playback + audio (runs: Gate 0 demo)

`kiriko-audio` cpal stream + clock; decode-ahead ring; the frame scheduler loop with
epochs ([playback-scheduler.md](playback-scheduler.md) §1, §4–5 — Cached mode only;
Realtime mode needs the evaluator, Phase 1); Timeline panel showing the footage layer
strip with in/out trim; J/K/L + Space transport. **Exit test**: Gate 0 in full
([16-ROADMAP.md](../16-ROADMAP.md)): 4K60 capture scrubs smoothly, plays with sync
(measured ≤ ±½ frame), UI ≤ 8 ms, kill/recover clean — verified on both machines, then
tag `phase-0`.

## Standing instructions for the implementing session

- Read the matching impl note **before** each slice; implement its test plan **with** the
  slice. Specs are canonical; when reality disagrees, change the doc in the same commit.
- Keep the glossary: the words in code identifiers come from
  [01-GLOSSARY.md](../01-GLOSSARY.md), including in this scaffold (no `Track`, no `Velocity`).
- Commit per slice at minimum; the exit test's evidence (test name, numbers) goes in the
  commit message.
- Anything discovered that is decision-sized goes to [02-DECISIONS.md](../02-DECISIONS.md),
  not into code comments.
