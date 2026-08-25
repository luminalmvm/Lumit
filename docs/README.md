# Lumit documentation

This folder is the specification and reference set for Lumit. Start here.

## Which doc do I want?

**If you are new to the codebase**, read [GUIDE.md](GUIDE.md) - the plain-English
tour of what each crate does, with Rust and threading explained in editing terms.
It is large; use its section index rather than reading straight through.

**If you are new to the codebase *and* intend to change it**, read
[learn/](learn/) - the onboarding set: one doc per area of the repo, plus
[learn/RUST.md](learn/RUST.md), [learn/FLUTTER.md](learn/FLUTTER.md) and
[learn/WGSL.md](learn/WGSL.md), each teaching the language from Lumit's own code.
GUIDE.md explains what the code does; these explain how to write it.

**If you want to change how the frontend and engine talk**, read
[17-BRIDGE-CONTRACT.md](17-BRIDGE-CONTRACT.md) - the single source of truth for
the front/back boundary.

**If you want to know what is left to build**, read [TODO.md](TODO.md) - the backlog. 
[16-ROADMAP.md](16-ROADMAP.md) is the aspirational phase plan above it.

## The three kinds of document here

Keeping these separate is what stops the set from rotting:

1. **Reference / spec** - durable intent and invariants. Changes slowly; kept
    canonical. The numbered docs below, plus the bridge contract.
2. **Living / state** - must mirror current reality. [TODO.md](TODO.md) and the
    bridge contract's version table. Update these in the same commit as the change
    they describe.
3. **Historical / point-in-time** - dated snapshots that are never updated, only
    read. They live in [archive/](archive/) and are frozen.

## Specification set

|Doc | What it specifies |
|---|---|
| [00-VISION](00-VISION.md) | Why Lumit exists, pillars, non-goals, the v1 milestone |
| [01-GLOSSARY](01-GLOSSARY.md) | Canonical terminology - binding on docs, UI, and code. §9 bans terms outright |
| [02-DECISIONS](02-DECISIONS.md) | Numbered decision log (K-###) with rationale. **Search it; never read it end to end** |
| [03-DATA-MODEL](03-DATA-MODEL.md) | Project/comp/layer/clip/property/keyframe object model |
| [04-RETIMING](04-RETIMING.md) | The Retime system: segments, the two graph lenses |
| [05-ARCHITECTURE](05-ARCHITECTURE.md) | Crates, threads, snapshots, the evaluation graph, GPU |
| [06-RENDER-PIPELINE](06-RENDER-PIPELINE.md) | Render order, colour, caching, preview, export |
| [07-UI-SPEC](07-UI-SPEC.md) | Panels, workspaces, Viewer, Timeline, graph editor, keymap |
| [08-EFFECTS](08-EFFECTS.md) | The built-in effect suite |
| [09-AUDIO](09-AUDIO.md) | The sync toolkit and the future Composer |
| [10-FILE-FORMAT](10-FILE-FORMAT.md) | The `.lum` container, sidecar caches, autosave |
| [11-AE-IMPORT](11-AE-IMPORT.md) | After Effects import and the fidelity matrix |
| [12-PLUGINS](12-PLUGINS.md) | OFX hosting, the LFX native API, expressions |
| [13-PERFORMANCE-RULES](13-PERFORMANCE-RULES.md) | Budgets, resource governor, degradation ladder |
| [14-ENGINEERING-RULES](14-ENGINEERING-RULES.md) | Binding rules for all code |
| [15-DESIGN](15-DESIGN.md) | The dark-first Aizome design language |
| [16-ROADMAP](16-ROADMAP.md) | Phases and their gates |
| [17-BRIDGE-CONTRACT](17-BRIDGE-CONTRACT.md) | The Flutter/Rust front/back boundary |

## Living documents

- [TODO.md](TODO.md) - the work backlog (Now / Next / Later).
- [GUIDE.md](GUIDE.md) - the plain-English guide to the codebase.

## Subfolders

- [learn/](learn/) - the onboarding set: an area-by-area tour of the codebase with
diagrams, and one teaching doc each for Rust, Flutter/Dart and WGSL, written from
real excerpts of this repository. Start at [learn/README.md](learn/README.md).
- [impl/](impl/) - implementation notes for the genuinely hard, low-level parts
(rational time, cubic solving, wgpu patterns, hardware decode, the scheduler,
optical flow, OFX hosting, beat detection, expressions, LUTs, layer inputs,
temporal re-render, paint, shape layers, lens flare, anti-aliasing): the authoritative *how* for each
topic — [impl/README.md](impl/README.md) holds the full table. Read the
matching note before implementing its feature.
- [research/](research/) - the background research that informed the specs.
Compiled 2026-07-12, under the project's former name Kiriko (K-087). Not
canonical: where it disagrees with a spec, the spec wins.
- [archive/](archive/) - frozen, dated material: audits, the egui-to-Flutter port
notes, and superseded ledgers. Read-only history; never updated.

Every numbered doc is canonical, follows [01-GLOSSARY](01-GLOSSARY.md), and uses
RFC-2119 keywords (MUST / SHOULD / MAY) in the binding sense.
