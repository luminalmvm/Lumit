# Luminal

**A native motion-graphics and compositing editor that plays back what you built, at speed,
without crashing — built first for the editors After Effects forgot.**

Status: design phase. This document is the why; the rest of [docs/](.) is the what and how.

---

## 1. The gap

The gaming-edit scene runs on After Effects plus an ~$800 stack of third-party plugins
(Twixtor, RSMB, Deep Glow, Sapphire, FilmConvert), much of it pirated by a young audience
that cannot afford it. Luminal's home lane within that scene is the **flow / MVM style** —
the CoD movie-making ("MVM") lineage (codmvm.com), today curated by teams and taught across
the T3C community: one- to three-minute music-video-style edits built from game footage,
mood- and phrasing-driven rather than beat-hammering, defined by velocity ramps, synthetic
camera moves, masked and morphing transitions, and identity colour grades. This is compositing and
animation editing as much as cutting — a style literally named after interpolation quality.

Its editors' daily experience is preview lag, RAM exhaustion, render-time blackmail on blur
and retiming, and projects that grow until, in one editor's words, After Effects becomes
"almost unusable" — plus an external tool chain (HLAE camera paths, Flowframes/RIFE
interpolation pre-passes, re-encodes) bolted around AE's gaps, and a retiming workflow many
fight rather than use, preferring Vegas Pro's ramp-and-cut editing. CapCut proved this
audience will move instantly to anything that is fast and free; nothing fast and free is
also deep.

Luminal is the deep tool built for them: After Effects' compositing model, Vegas' retiming
soul, one application, GPU-first, open source (GPLv3).

## 2. Who it is for

1. **First: flow / MVM-style gaming editors** (the smooth, cinematic lane of the T3C
   community and its neighbours). Their editing is compositing and animation as much as
   cutting: the graph editor, the camera, and masks are daily tools, not occasional ones.
   The v1 milestone is theirs — see §4.
2. **Then: anyone leaving After Effects.** Luminal grows toward a full AE replacement, with
   its own version of everything AE has (decision K-002), an AE project importer, and OFX
   plugin support so existing tools come along.

## 3. Pillars

Everything in the specs traces to one of these; a feature that serves none of them is scope creep.

1. **Playback is the product — and so is responsiveness at scale.** Preview at speed,
   always: GPU-resident pipeline, content-hash caching, adaptive degradation. The app
   responds to input in every state — the UI thread never renders a frame — and stays fully
   interactive in intensive projects with thousands of layers and hundreds of thousands of
   keyframes, where AE becomes barely usable. Both are budgeted and CI-enforced
   ([13-PERFORMANCE-RULES.md](13-PERFORMANCE-RULES.md) §2, §2.1).
2. **Never lose work, never crash.** Degrade instead of dying; treat GPU resets as routine;
   journalled autosave; plugins in separate processes. Rust because the compiler enforces
   what a style guide cannot ([05-ARCHITECTURE.md](05-ARCHITECTURE.md)).
3. **Retiming as a first-class citizen.** One Retime system, two honest views (value graph,
   speed graph), cuttable clips on a Sequence layer, and the beat-sync covenant: editing a
   ramp never moves your cuts ([04-RETIMING.md](04-RETIMING.md)).
4. **The look is in the box.** The montage staples — flow retiming, flow motion blur, real
   glow, camera shake, grading — ship built in, so a new editor pirates nothing and installs
   nothing ([08-EFFECTS.md](08-EFFECTS.md)).
5. **Strict words, learnable tool.** One glossary, enforced everywhere
   ([01-GLOSSARY.md](01-GLOSSARY.md)). Precise language is what keeps a deep tool learnable
   and its codebase coherent.
6. **Open and shareable.** GPLv3; project files portable by design; presets and template
   projects as first-class shareable objects — because sharing is how this scene teaches
   itself (K-065).

## 4. The v1 milestone

> A flow-style editor records gameplay tonight and publishes tomorrow, using only Luminal:
> import 120/240 fps captures, cut against the music's phrasing with auto beat markers,
> speed-ramp with flow slow motion, animate a smooth 2.5D camera move, build a masked
> transition, apply shake/glow/motion-blur/grade, and export a YouTube-ready 1080p60 file —
> with real-time preview of the full look on an RTX 3060, and not one crash.

Phases and gates: [16-ROADMAP.md](16-ROADMAP.md).

## 5. Non-goals

- **Not an NLE for long-form.** No multi-hour documentary workflows, bins-and-logging, or
  multicam. The Sequence layer is for building shots and montages, not features films.
- **Not a DAW.** Audio serves the edit ([09-AUDIO.md](09-AUDIO.md)); mixing consoles and
  audio plugin hosting are out of scope until the Composer, and modest even then.
- **No accounts, no cloud, no telemetry.** Local software. Diagnostics stay on the machine.
- **Not a clone.** AE-compatible where compatibility is free learning (keyframe maths,
  layer semantics) or free imports; deliberately different where AE is wrong for our users
  (retiming, caching, matte model, per-comp depth).
- **No dark patterns**: no punishment UI, no upsells, no artificial limits (household
  design mandate, [15-DESIGN.md](15-DESIGN.md)).

## 6. Name

Luminal — of light, and of thresholds (K-083; the project began life as *Kiriko*, renamed
by the owner 2026-07-16). The app's whole work happens at a threshold: between an editor's
intent and the picture, between one frame and the next. The name is used bare ("Luminal",
never "the Luminal app"), and features are named per the glossary. The named subsystems
share the astral register: **Nova** is the render pipeline, **Nebula** the cache, and
**Pulsar** the audio engine whose clock everything syncs to.

## Open questions

- Distribution: GitHub releases at first; winget/Microsoft Store later?
- Whether official binaries are ever paid (GPLv3 permits it; undecided, no dependency on it).
- Community home: the repo plus a Discord seems inevitable for this audience — when?
