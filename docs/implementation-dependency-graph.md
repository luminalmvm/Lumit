# Remaining-work dependency graph

Companion to [implementation-audit-2026-07-20.md](implementation-audit-2026-07-20.md). An arrow
points **from a prerequisite to the box that relies on it** — so you read chains left/top → down
as build order. Boxes with no arrows are free-standing: they can be picked up any time.

Markers inside boxes:

- **👁** — built and CI-green, but the behaviour needs the owner's interactive check. Once you
  confirm one, tell me and it gets a ✅.
- **✅** — completed. **Kept in the graph, not pruned** — so the owner can still find and
  independently verify each finished item later. They stay put permanently.
- **Decision:** — blocked on an owner decision before implementation can start.
- **(post-v1)** — future by design per the docs; drawn faintly at the bottom for completeness.

Maintenance rule: whenever a feature lands or a 👁 is confirmed, this file is updated in the same
commit (add/flip the ✅ marker, re-wire arrows), so the graph stays the live picture of progress.
Completed boxes are never removed — the graph is a growing record, not just a to-do list.

Layout rule: subgraphs that arrows run *between* are declared next to each other, and the
free-standing ones (Media and colour, Audio, Keymap, Distribution, Plugins — no arrows in or
out) are declared together at the end. Mermaid places boxes roughly in declaration order, so
an independent group left in the middle pushes connected ones apart and drags their arrows
across the whole picture. Keep new subgraphs on the same principle: beside what they connect
to, or at the end if they connect to nothing.

```mermaid
flowchart TD

  subgraph SVERIFY["Verify first — built, awaiting your eye"]
    RTEYE["✅ Realtime adaptive preview — render-pull rework, owner-accepted:<br/>no longer freezes (one un-superseded render at a time),<br/>fed the real decode cost so the tier drops + the box is honest.<br/>Known limit (documented): dropping res doesn't cut decode, so<br/>decode-bound comps stay a bit choppy until render-ahead (RING) —<br/>Cached is the smooth path there"]
    SETTINGSMAC["✅ Settings reachable on macOS — added to the app menu at Cmd+,<br/>(native menu bar had no Settings item); owner confirmed Cmd+, works"]
    LUMAEYE["✅ Luma matte perceptual gate — owner-verified"]
    LIMEYE["✅ Master limiter — owner-verified by ear"]
    RECEYE["✅ Crash recovery — owner-accepted"]
    KEYSEYE["✅ New keybindings — owner-verified"]
    RATEEYE["✅ Retime to-Rate button — owner-verified<br/>(all further retime work deferred by owner to last)"]
    BANNEREYE["👁 Error banner fig tint (15 §10)"]
    SOLOEYE["👁 Audio solo — logic verified; re-test the live<br/>mix plan: edits heard on the next callback"]
    BEATEYE["👁 Beat sensitivity slider — now in the timeline<br/>empty-lane right-click menu; re-check there"]
    AUDIOMEMEYE["👁 Memory + instant audio — re-test with the movie:<br/>RAM within the ONE Settings budget (half RAM default),<br/>solo/mute/move/volume heard instantly (the comp waveform strip<br/>is gone, K-172 — per-layer lanes fill as decodes land)"]
  end

  subgraph SRETIME["Retime surface — owner: leave ALL of this until last"]
    FREEZE["Wire freeze_at_playhead (04 §7.3)"]
    HOLDP["Hold preset in the ramp shelf (04 §12.2)"]
    MERGEB["Wire merge_boundary on boundary delete (04 §5.4)"]
    RCHROME["Graph chrome: kink badge, RATE/MAP chips,<br/>source-extent band, media-end line (04 §6, §9)"]
    DRIFTB["Persistent fitted-drift badge (04 §9.2)"]
    REVUI["Reverse toggle + lock glyph +<br/>negative-region editing (04 §6.2)"]
    NUMENTRY["Speed-lens numeric % entry +<br/>Alt-compensated edit (04 §9.2)"]
    QUANTB["Bulk quantise boundaries to beats (04 §12.3)"]
    STRETCHOP["Stretch op rewriting the Retime store (04 §11.2)"]
    OUTTRIM["Outward trim extends map at constant speed (04 §7.3)"]
    RTCOPY["Copy/paste retime + paste-attributes (04 §8.2)"]
    FREEZE --> HOLDP
  end
  RATEEYE --> DRIFTB

  subgraph SPERF["Performance backbone"]
    TEXPOOL["Texture pool with refcounts (06 §2.2)"]
    GOV["Resource governor —<br/>RAM/VRAM budgets, ledger (13 §3)"]
    LADDER["Degradation ladder and status chip (13 §4)"]
    DEVLOST["Device-lost recovery and DRED (13 §5, 14 §4)"]
    TEXPOOL --> ROIDOD
    TEXPOOL --> GOV
    TEXPOOL --> DEVLOST
    GOV --> LADDER
  end
  RTEYE --> LADDER

  subgraph SCACHE["Cache tiers"]
    FP16DISK["fp16 LZ4 planes + colourspace marker on disk (06 §5.4) —<br/>the disk tier ALREADY does LZ4 + a magic/format/colourspace/size<br/>header for RGBA8 (today's producible format); the fp16-planes format<br/>tag is blocked on the working fp16 frame reaching the CPU, i.e. the<br/>pixel-pass migration — not a standalone task"]
    GREEDY["✅ GreedyDual eviction + pinning (06 §5.3) — ByteLru now evicts by<br/>staleness × size ÷ recompute-cost (the 'stale × cheap × large' rule),<br/>reduces to LRU for equal size/cost so callers are unchanged; pin/unpin<br/>protects the displayed frame + playhead window, bounded overage when only<br/>pins remain. Cost hints via insert_with_cost. Tested. Remaining: caller<br/>adoption (pass real costs / pin the playhead window) + VRAM→RAM demotion<br/>(waits on the VRAM tier)"]
    SQLIDX["index.db SQLite cache index (06 §5.4)"]
    VRAMTIER["VRAM cache tier (06 §5.1)"]
    CACHEBAR3["Cache-bar third tier + all-mint ramp (15 §6.3)"]
    VRAMTIER --> CACHEBAR3
  end
  TEXPOOL --> VRAMTIER

  subgraph SSPINE["Render and evaluation spine"]
    WPOOL["✅ Worker pool — built + tested<br/>(lumit-eval::pool, cores−3 min 2, two classes;<br/>its tenants — per-frame render jobs — arrive with PIXPASS;<br/>existing shell spawns are decode/IO roles, correctly off-pool)"]
    SEAMS["✅ Eval trait seams — built + tested<br/>(lumit-eval::exec: FrameSource / KernelExecutor /<br/>CacheStore + demand-pull executor over fakes)"]
    PIXPASS["Eval-graph pixel pass — the graph renders,<br/>not lumit-ui (06 §1.1). Executor core done; walking<br/>skeleton proven on the real compositor (solids, exact<br/>transform placement, linear blend, cache hits, ALL blend<br/>modes, layer masks, adjustment-layer effects — lavapipe-tested);<br/>remaining: retime vocabulary +<br/>switching preview/export onto it"]
    ROIDOD["ROI/DoD protocol, macro-tiles,<br/>per-node CPU fallback (06 §2, K-019)"]
    EXPORTC["Export compiler and baking (06 §7.2, K-024)"]
    PROFILER["Per-node profiler and<br/>render-time column (13 §7.1)"]
    GPUSUB["GPU-submit thread owns the queue (05 §2)"]
    RING["Wire render-ahead ring and pre-roll (06 §6.4)"]
    FRAMEPACE["Frame-pacing budgets B5–B7 (13 §2)"]
    CACHEDPLAY["✅ Cached playback render-gated stepping — owner-confirmed working (K-171):<br/>every frame shown, advances when cached + at realtime pace,<br/>audio pauses while awaiting a frame. Tested core (cached_step).<br/>Desk-session refinements: pace timer carries its remainder<br/>(cached_pace_carry — replay ran ~half speed and audio drifted; TF-1)<br/>and audio starts with the first frame of a ready run<br/>(readiness lookahead replaced the warm-up streak; OD-1).<br/>Remaining refinement: audio timestretch instead of pause"]
    SCOPESGPU["GPU compute scopes (06 §8, K-096)"]
    WPOOL --> PIXPASS
    SEAMS --> PIXPASS
    PIXPASS --> ROIDOD
    PIXPASS --> EXPORTC
    PIXPASS --> PROFILER
    WPOOL --> RING
    RING --> FRAMEPACE
    GPUSUB --> FRAMEPACE
    CACHEDPLAY --> RING
  end
  RING -.->|smooths decode-bound realtime| RTEYE

  subgraph SENG["Engineering and CI"]
    TYPEDTIME["Typed rational time through eval +<br/>FrameIndex newtype (14 §2)"]
    AUDIDX["No-panic indexing in the audio callback (14 §4)"]
    HOTSWEEP["Hot-path sweep → enable indexing_slicing /<br/>arithmetic_side_effects lints (14 §4)"]
    FIXTURES["◑ Stress fixture DONE, reference comp remaining (13 §1, §7.3) —<br/>lumit-project::fixtures::stress_document builds the 13 §2.1 doc<br/>(200 comps / 5,000 layers / 250,000 keyframes / 2,000 footage)<br/>deterministically from fixed UUIDs, parameterised (TINY for tests,<br/>REFERENCE for the spec); determinism + spec-count + save/open tested.<br/>Remaining: the faithful 5-layer reference comp (§1, LUT/glow/retime)"]
    BENCH["◑ Benchmark harness started (13 §7.3) — a criterion bench<br/>(lumit-project/benches/document_scale) measures the document-scale<br/>S2–S5 on the REFERENCE stress fixture: save, open (~194 ms locally),<br/>commit, undo. Runs on demand (cargo bench); compiles in CI but is not<br/>a gate yet. Remaining: the B3–B11 render/playback/export benches<br/>(need the pixel-pass migration) and wiring budgets into CI (PERFCI)"]
    PERFCI["Perf budgets gate merges in CI (13 §7.3, 16)"]
    EXRGOLD["Golden-frame EXR tests per platform (14 §6)"]
    KITTEST["egui_kittest UI test harness"]
    UICOV["Coverage gate: UI crates (14 §6)"]
    LAVACI["CI software-GPU step (lavapipe)"]
    GPUCOV["Coverage gate + kernel tests for lumit-gpu in CI"]
    FUZZ["cargo-fuzz on .lum / journal (14 §6)"]
    DENYTOOL["cargo-deny + pinned toolchain +<br/>edition 2024 (14 §7, §9)"]
    PEDANTIC["rust_2024_idioms + clippy pedantic sweep (14 §7)"]
    I18N["i18n string externalisation (14 §7, K-005)"]
    TRACING["tracing spans + diagnostics ring log +<br/>export action (14 §8, 13 §7.2)"]
    CRASHH["Crash-handler process, Crashpad (05 §8, 14 §8)"]
    GLOSSCI["Decision: glossary banned-terms CI gate —<br/>term list + scoping (14 §7, 01 §9)"]
    JCOMPACT["✅ Undo-journal compaction (14 §5) — the in-memory undo/redo<br/>history is now bounded to MAX_UNDO_DEPTH (500): commits past the<br/>cap drop the oldest steps (state untouched), redo is transitively<br/>bounded, crash recovery unaffected (disk journal is separate).<br/>Compaction story documented at the type; regression-tested"]
    AEFIXT["AE golden keyframe fixtures +<br/>denominator property test (04 impl notes)"]
    FIXTURES --> BENCH
    BENCH --> PERFCI
    FIXTURES --> EXRGOLD
    KITTEST --> UICOV
    LAVACI --> GPUCOV
    TRACING --> PROFILER
  end

  subgraph SDESIGN["Design and accessibility"]
    FONTS["Embed the full type stack:<br/>Schibsted Grotesk, Source Serif 4, JetBrains Mono (15 §1.1)"]
    TYPESCALE["Apply the documented type scale (15 §7.1)"]
    TOKENS["Split semantic theme tokens: disabled, fill_tonal,<br/>keyframe, marker, selection… — incl. Decision:<br/>Adjustment-layer colour per scheme (15 §4.1, §6.1)"]
    CONTRASTCI["Contrast-floor CI check (15 §9)"]
    ACCESSKIT["AccessKit wiring: roles, landmarks,<br/>timeline tree (15 §9)"]
    FONTS --> TYPESCALE
    TOKENS --> CONTRASTCI
  end

  subgraph SUI["UI shell"]
    THUMBS["Thumbnail render service (engine-rendered)"]
    HOVSCRUB["Project panel columns, sort, Ctrl+F,<br/>hover-scrub thumbnails (07 §3.1)"]
    THUMBSLUM["thumbs/ inside the .lum container (10 §1)"]
    PREVIEWPANEL["Preview panel: loop modes, fill-cache,<br/>mute, quality toggle (07 §9)"]
    AUDIOPANEL["Audio panel + meters (07 §10)"]
    WORKSPACES["Workspaces: presets, switcher,<br/>Alt+Shift+1–9, user CRUD (07 §1.4)"]
    VIEWERCHROME["Viewer chrome cluster: magnification steps,<br/>transparency grid, rulers/guides, bg swatch,<br/>click-to-type time, viewer locks (07 §2.2, §2.6)"]
    GIZMO["Transform gizmo: move/scale/rotate (07 §2.3)"]
    MOTIONPATHS["Motion paths in the Viewer (07 §2.4)"]
    DOCKX["Dock extras: floating frames, drop zones,<br/>backtick maximise, panel menus (07 §1)"]
    MARKERSUI["Marker ribbon interactions:<br/>create/drag/labels/layer rows (07 §4.1)"]
    SNAPX["Snapping to edits/keys/playhead +<br/>Ctrl suspend + indication (07 §4.5)"]
    EXPORTQ["Export queue list UI:<br/>reorder/retry/cancel (07 §11)"]
    PALETTEX["Palette: comps/panels categories,<br/>badges, recent-first (07 §12)"]
    PERSIST["◑ Per-comp viewer/timeline state persistence (07 §1.5)<br/>— session restore shipped (OD-4): open tabs, fronted comp,<br/>playhead, selection + twirls, per project path.<br/>Remaining: viewer locks, column state, per-comp zoom"]
    FOCUSNAV["Focus cycling Ctrl+F6, Tab traversal,<br/>arrow nav (07 §14)"]
    PERCOMPRES["Per-comp preview-resolution state (07 §2.2)"]
    THUMBS --> HOVSCRUB
    THUMBS --> THUMBSLUM
    PREVIEWPANEL --> WORKSPACES
    AUDIOPANEL --> WORKSPACES
    GIZMO --> MOTIONPATHS
  end
  RTEYE --> PREVIEWPANEL
  ACCESSKIT --> FOCUSNAV

  subgraph SFX["Effects and model"]
    STRENGTHM["K-035 universal per-effect strength matte<br/>host plumbing (08)"]
    ANIMMASK["Animated keyframed masks (03 §7)"]
    TRANSITIONS["Masked transitions — Gate 3 (16)"]
    SMOOTHZOOM["Smooth-zoom effect (16 Phase 3)"]
    FXDEFER["Doc-flagged effect deferrals: glow falloff/CA,<br/>LUT tetrahedral, echo transforms,<br/>matte-key spatial, vignette tint… (08)"]
    GRADELIB["Shipped grade-preset library ≥40 +<br/>live thumbnails (08 §3.10)"]
    ANIMMASK --> TRANSITIONS
  end
  THUMBS --> GRADELIB

  subgraph SPOST["Post-v1 by design"]
    STENCIL["Stencil / Silhouette / Alpha-add mattes (06 §3.5)"]
    TIER2["18 Tier-2 effects (08 §4)"]
    LOUDNESS["Loudness normalisation −14 LUFS (09 §8)"]
    COMPOSER["The Composer (09 §9)"]
    OTIO["OTIO interchange (10 §6)"]
    FIRSTRUN["First-run screen (K-006)"]
    SHAPEL["Shape layers (03 §9.2)"]
  end
  ANIMMASK --> SHAPEL

  subgraph SFILE["File format and relink"]
    FPRINT["✅ MediaRef content fingerprint (10 §2, 03 §3) — Fingerprint type<br/>(size + mtime + blake3 head/tail hash) on MediaRef, optional + skipped<br/>when unset so old projects round-trip byte-identical; lumit-project::<br/>fingerprint_path computes it (≤2×64 KiB read, cheap on huge footage);<br/>likely_same_content matches a moved/copied file by content. Tested.<br/>Unblocks the relink resolver (step 3)"]
    RELINK["✅ Relink resolver: 4-step + sibling auto-relink (10 §2) —<br/>lumit-project::resolve_media tries relative → absolute →<br/>fingerprint search (size-filtered walk of search roots + project<br/>tree) → Missing; path_mapping/apply_mapping relink siblings that<br/>moved the same way. Pure/tested (5 tests). Remaining: the relink<br/>dialogue UI (RELINKUI) + calling this from open()"]
    RELINKUI["Missing-footage badge + relink flow (07 §3.3)"]
    COLLECT["✅ Collect-for-sharing (10 §2, K-065) — lumit-project::<br/>collect_for_sharing copies every located reference into<br/>dest/media/ (colliding names disambiguated), rewrites refs<br/>project-relative with nothing machine-specific, reports<br/>unlocatable media instead of failing. Tested (3). Remaining:<br/>the menu command + save-into-dest wiring (lumit-ui)"]
    MIGRATE["✅ Format migration framework (10 §1, 03 §12) — an ordered<br/>Migration chain transforms raw project.json (as serde Value, before<br/>typing) version→version; open() walks an older file up to the current<br/>schema, current files take the direct path unchanged. Bounded (never<br/>loops on a malformed chain). Chain empty today (0.1.0 is first);<br/>each schema bump appends one step. Tested (3, synthetic chain)"]
    RISKYOPS["Autosave before risky ops (10 §4)"]
    TEMPLATEO["New-from-template open mode (10 §5)"]
    FPRINT --> RELINK
    RELINK --> RELINKUI
    RELINK --> COLLECT
  end

  subgraph SAE["AE import (16 Phase 4)"]
    RIFX["RIFX .aep parser in lumit-project (11 §7)"]
    MAPPER["AE→Lumit mapper: fidelity matrix +<br/>ae-effect-map.toml (11 §4, §5)"]
    BRIDGE["Bridge .zxp panel + .lum-bundle format (11 §2)"]
    IMPREPORT["Import report panel + placeholder badges (11 §6, §9)"]
    AEGOLD["AE golden-frame tests (11 §5)"]
    IMPRELINK["Footage relink chain on import (11 §2.5)"]
    TRIMP["AE Time-Remap import driver (04 §13.1)"]
    LOTTIE["Lottie/bodymovin importer (11 §8) (post-v1)"]
    RIFX --> MAPPER
    BRIDGE --> MAPPER
    MAPPER --> IMPREPORT
    MAPPER --> AEGOLD
    MAPPER --> TRIMP
    MAPPER --> LOTTIE
  end
  RELINK --> IMPRELINK
  MAPPER --> IMPRELINK

  subgraph SMEDIA["Media and colour"]
    DECODER["Persistent per-stream decoders (05 §2, 06 §3.2)"]
    HWDEC["Hardware decode → NV12/P010 on GPU,<br/>no CPU round trip (06 §3.2)"]
    CMPASS["Single colour-management compute pass<br/>(matrix + chroma + linearise)"]
    TAGS["Per-footage colour-space tags<br/>(MediaRef interpretation; 06 §3.2, 03 §3)"]
    INTERPRET["Interpret-footage dialogue (07 §3.2)"]
    CTRANS["ColourTransform display interface (06 §3.3)"]
    CHANVIEW["Viewer channel view, exposure,<br/>CM badge (07 §2.2)"]
    K031CI["K-031 golden viewer-equals-export CI test (06 §3.3)"]
    DEPTH["Project bit depth 8/16/32 (K-069, 06 §3.1)"]
    PRORES["ProRes / DNxHR encoders (06 §7.4)"]
    MASTERP["Master intermediate preset (06 §7.5)"]
    ALPHAOUT["Straight/premult alpha export option (06 §7.4)"]
    PROXYGEN["Proxy generation (06 §3.2)"]
    PROXYUI["Proxy badges and toggle (07 §3.3)"]
    IMGSEQ["Image sequences (05 §1)"]
    VREFRAME["Vertical one-click reframe crop (06 §7.5)"]
    DECODER --> HWDEC
    HWDEC --> CMPASS
    TAGS --> CMPASS
    TAGS --> INTERPRET
    TAGS --> CHANVIEW
    CTRANS --> CHANVIEW
    CTRANS --> K031CI
    PRORES --> MASTERP
    PRORES --> ALPHAOUT
    PROXYGEN --> PROXYUI
  end

  subgraph SAUDIO["Audio"]
    GAIN["✅ Per-layer gain / volume keyframes (09 §3.1, §6) —<br/>shipped K-172 (desk session): Layer.volume_db, dB with a −∞ knee,<br/>envelope-baked fades identical in playback + export"]
    FADES["Fade-in/out commands (09 §6)"]
    AUDLAYER["Audio layer kind + detach-audio (09 §6)<br/>(precomp audio + carrier-volume chains shipped OD-5;<br/>precomp speakers + engine-mix sync scoping OD-8;<br/>audio-only rows already drop the eye)"]
    RINGBUF["Streaming decode + latency compensation (09 §3.1)<br/>(live per-callback mixing now ships via MixPlan;<br/>remaining: decode windows instead of whole files)"]
    SCRUBAUD["Audio scrubbing, windowed grain (09 §3.4)"]
    DEVCHG["Device-change stream rebuild (09 §3.2)"]
    PEAKS["Multi-tier sidecar peak files (09 §4)"]
    WAVELAYERS["◑ Waveforms on layers (09 §4) — shipped K-172:<br/>per-layer Waveform twirl, live-offset peaks that ride a bar<br/>drag (comp strip removed). Remaining: inside Sequence clips;<br/>a mixed lane for precomp carriers; sidecar peaks for scale"]
    TAP["Tap tempo (09 §5)"]
    BPMUI["BPM confirm/type + phase + grid-fill (09 §5)"]
    RETMUTE["Retime mutes audio + badge (09 §7)"]
    GAIN --> FADES
    GAIN --> AUDLAYER
    RINGBUF --> SCRUBAUD
    PEAKS --> WAVELAYERS
    TAP --> BPMUI
  end

  subgraph SKEYS["Keymap and input"]
    KEYMAP["✅ Keymap core built (07 §15) — new lumit-keymap crate: Chord<br/>(parse/display, order+case-insensitive, Ctrl/Cmd/Mod primary), KeyContext,<br/>ActionId, Keymap with context-precedence lookup, conflict detection<br/>(Global-overlap rule), bind/unbind/search, shareable serde file. The FULL<br/>§15 default table now ships (Global/Tools/Timeline/Graph/Viewer/Panels,<br/>conflict-free) + AE preset. Pure logic, ~96% covered (in the coverage gate).<br/>Remaining: wire the lumit-ui event path + Settings→Keymap UI to it"]
    KEYS30["Remaining ~30 bindings incl. J/K/L shuttle,<br/>I/O, comma/dot, P-S-R-T-A reveals, U/UU"]
    AEKEYS["AE keymap preset + keymap settings page (07 §15)"]
    TOOLTIPKEYS["Tooltip shortcut text (07 §13.2)"]
    MACMENU["macOS native menu ↔ keymap sync (07 §15)"]
    RAZORD["Decision: razor-on-C semantics —<br/>click-tool vs playhead cut (07 §4.4)"]
    KEYMAP --> KEYS30
    KEYMAP --> AEKEYS
    KEYMAP --> TOOLTIPKEYS
    KEYMAP --> MACMENU
    RAZORD --> KEYS30
  end

  subgraph SDIST["Distribution"]
    FLATPAK["✅ Linux Flatpak bundle — CI-green (TF-3): every run publishes<br/>an installable lumit-x86_64.flatpak (~15 MB) carrying its own<br/>FFmpeg 7.1, so it runs on any distribution"]
    LINUXCI["✅ Linux CI green — clippy, tests, GPU oracles on lavapipe<br/>(64 of them, first time they run in CI at all),<br/>release compile check (TF-3)"]
    MACAPP["macOS .app bundle + signing/notarisation"]
    WINPKG["Windows installer (MSI/MSIX) + code signing"]
    FLATHUB["Flathub submission (needs screenshots,<br/>a narrower --filesystem than host)"]
    FLATPAK --> FLATHUB
  end

  subgraph SPLUG["Plugins and expressions (post-main-app, 12 §1)"]
    SANDBOX["Sandbox process substrate:<br/>broker, shared memory, watchdog (K-066)"]
    OFXH["OFX host: suites, actions, param bridge (12 §2)"]
    OFXGPU["OFX GPU render suites +<br/>quirks table + discovery (12 §2.4–2.6)"]
    LFXH["LFX host: C ABI + validator + template (12 §3)"]
    SECMODEL["Plugin security model (12 §5)"]
    QJS["QuickJS-ng expression engine (12 §4, K-063)"]
    EXPRAPI["AE expression API subset +<br/>determinism + perf model (12 §4)"]
    EXPRREADS["sourceTime / retimeSpeed reads (04 §11.6)"]
    SANDBOX --> OFXH
    OFXH --> OFXGPU
    SANDBOX --> LFXH
    SANDBOX --> SECMODEL
    QJS --> EXPRAPI
    EXPRAPI --> EXPRREADS
  end
```

## Suggested attack order

1. **The 👁 queue** — pure verification, no code. Mostly cleared: realtime (accepted), cached
   playback (confirmed), macOS Settings, luma matte, limiter, crash recovery, keybindings and
   the Rate button are all owner-confirmed. Remaining eyes: error-banner tint, audio solo on the
   live mix, the beat-sensitivity slider, and memory+instant-audio on the movie.
2. **The spine: worker pool + eval seams → pixel pass.** ← *in progress.* The single biggest
   fan-out in the graph — it unlocks ROI/DoD, the export compiler, and the per-node profiler,
   and it is the architecture 05/06 actually specify. Worker pool and the trait seams are done;
   the walking skeleton (executor driving the real compositor through the seams) is being widened
   one vocabulary slice at a time — solids, transform placement, opacity, linear blend, all blend
   modes, layer masks and adjustment-layer effects proven (lavapipe-tested); retime is the last
   vocabulary slice — before preview and export switch onto it.
3. **Texture pool → governor → degradation ladder** (plus device-lost recovery and the VRAM
   tier) — the performance backbone; nothing in 13 can be enforced without it.
4. **Media and colour: persistent decoders → hardware decode → CM pass, with colour tags.**
   Correctness-visible (Rec.709 footage is currently mis-tagged as sRGB) and a prerequisite for
   the Interpret dialogue and channel view.
5. **The two mid-size fan-outs:** the remappable keymap (four dependents) and the fingerprint →
   relink chain (three dependents, plus AE import needs it later).
6. **Fixtures → benchmark harness → perf CI** — turns 13-PERFORMANCE-RULES from aspiration
   into gates, per the standing rules in 16.
7. **AE import, then plugins/expressions** — Phase 4 order per 16-ROADMAP.

Free-standing boxes (no arrows) are good gap-fillers between the chains — e.g. GreedyDual
eviction, scopes-on-GPU, cargo-deny/toolchain pinning, the smooth-zoom effect, journal
compaction, device-change audio rebuild.

## Engine-local vs wired work (status)

A run of engine-crate work has landed and is CI-green: the render-spine walking skeleton now
covers all blend modes, layer masks and adjustment-layer effects through the trait seams;
GreedyDual cache eviction + pinning; the bounded undo journal (§5 compaction); the whole
file-format engine chain (content fingerprint → 4-step relink resolver → collect-for-sharing →
schema-migration framework); the deterministic stress fixture + a document-scale benchmark; and
a new `lumit-keymap` crate carrying the full §15 binding table with conflict detection. All were
built and tested **locally** (engine crates + lumit-gpu on lavapipe) and verified in CI.

What remains is, by nature, **not cleanly engine-local** — it needs one of: the pixel-pass
**migration** (switching preview/export onto the executor, which also unblocks the fp16 disk
format, the ROI/DoD protocol, the export compiler and the per-node profiler); **lumit-ui**
wiring (the relink dialogue, collect/keymap/settings surfaces, the Preview/Audio panels, viewer
chrome, gizmos, workspaces); **GPU** work (VRAM tier, texture pool → governor → degradation
ladder, device-loss recovery); **media/FFmpeg** (persistent decoders, hardware decode, the
colour-management pass, proxies) or **audio** (fades-as-commands, streaming windows — per-layer gains shipped, K-172); or an
**owner decision** (razor-on-C, the Adjustment-layer colour, the glossary CI gate). Those are
picked up in the app crate (built via CI) or once the migration lands.
