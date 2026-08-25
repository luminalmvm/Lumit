# Kiriko research: plugin ecosystems & After Effects interoperability

Research date: 2026-07-12. Sources: openfx.readthedocs.io, openeffects.org, ASWF announcements, Adobe AE SDK docs (ae-plugins.docsforadobe.dev), RE:Vision/BorisFX product pages, GitHub reverse-engineering projects, CLAP/VST3 ecosystem writing.

---

## 1. OpenFX (OFX)

### What it is
- Open standard C API for 2D image-processing plugins ("The OFX Image Effect Plug-in API"), in production use since ~2004. Originally created by The Foundry, stewarded by the Open Effects Association, and **since August 2022 hosted by the Academy Software Foundation (ASWF)** — the OFA dissolved and its directors (Gary Oberbrunner, Pierre Jasmin/RE:Vision, Dennis Adams/MAGIX, etc.) formed the ASWF Technical Steering Committee. Development is fully public at `github.com/AcademySoftwareFoundation/openfx`.
- **Licensing: BSD 3-Clause** on the headers, spec, and support/example libraries. No membership fee, no certification, no royalties. Anyone may implement a host or plugin. This is the single lowest-risk interop surface available to Kiriko.
- Current spec: **OFX 1.5** (released Sept 2024; 1.5.1 docs current as of Mar 2026).

### Hosts
- Commercial: **DaVinci Resolve + Fusion** (Blackmagic), **Nuke** (Foundry), **Flame** (Autodesk), **VEGAS Pro** (MAGIX), Baselight (FilmLight), Scratch (Assimilate), Nucoda, Silhouette (BorisFX), Mistika, HitFilm (historically), Avid Media Composer (via AVX↔OFX support in recent versions), Autograph (Left Angle), Catalyst Edit (Sony).
- Open source: **Natron** (a full OFX host — its entire effect stack is OFX, incl. `openfx-misc` ~80 plugins under GPL), plus ShotCut/MLT experiments.
- Key strategic fact for Kiriko: **VEGAS Pro is an OFX host, and Vegas is where the gaming-edit community's staples already run.** RE:Vision explicitly ships and documents Twixtor and ReelSmart Motion Blur OFX builds with Vegas Pro installers/serial types. So the "Twixtor/RSMB in a non-Adobe host" problem is already solved commercially — via OFX, not via AE plugin emulation.

### Plugin vendors shipping OFX builds
- **RE:Vision Effects**: Twixtor (v7/v8), ReelSmart Motion Blur (v5/v6), DE:Noise, RE:Lens etc. — OFX hosts listed: Nuke, Resolve, Fusion Studio, Vegas Pro, HitFilm, Catalyst Edit, Natron, Scratch, Nucoda, Silhouette, Flame, Autograph, Media Composer. (Pierre Jasmin of RE:Vision sits on the OpenFX TSC — the vendor most important to gaming edits is one of the standard's maintainers.)
- **BorisFX**: Sapphire, Continuum, Mocha Pro all ship "OFX" editions (Resolve, Vegas, Nuke, Flame, Silhouette).
- **Others**: NewBlueFX, FXhome, Nobe/Time in Pixels (Color Remap), ntsc-rs (open-source NTSC artifact effect ships an OFX build), Neat Video (denoise, OFX for Resolve/Vegas), Dehancer, FilmConvert.
- Notable absences: the Adobe-only ecosystem — Video Copilot (Element 3D, Saber, Optical Flares), Red Giant Universe partially (has/had OFX for Resolve/Vegas but Trapcode Particular/Form are AE-only), Plugin Everything (Deep Glow), aescripts tooling. **Assume anything built on AEGP/SmartFX/custom AE UI never comes over.**

### What implementing an OFX host involves
Architecture: plugin is a shared library (`.ofx` bundle) exporting `OfxGetPlugin(n)`; host passes an `OfxHost` struct whose `fetchSuite(name, version)` hands back C function-pointer tables ("suites"). Everything else is properties (string-keyed attribute/value sets) and "actions" (string-named messages sent to the plugin: describe, createInstance, render, etc.).

Minimum viable host must implement:
- **Property Suite** (`OfxPropertySuiteV1`) — the load-bearing one; every object (host, effect, clip, image, param) is described by property sets. Get/set for int/double/string/pointer, arrays. Tedious but mechanical.
- **Image Effect Suite** (`OfxImageEffectSuiteV1`) — effect/instance lifecycle, clip access (`clipGetImage` returns a property set with data pointer, rowbytes, bounds, pixel depth, components).
- **Parameter Suite** (`OfxParameterSuiteV1`) — parameter definition (double, int, RGBA, choice, boolean, string, custom, group, page) plus **animation: `paramGetValueAtTime`** — the host owns keyframes/curves, the plugin just asks for values at a time. This maps cleanly onto Kiriko's own animation system.
- **Memory Suite, Multithread Suite, Message Suite** — small.
- Actions to dispatch: `onLoad`, `describe`, `describeInContext` (contexts: filter, general, generator, transition, retimer), `createInstance`, `getRegionOfDefinition`, `getRegionsOfInterest`, `getFramesNeeded` (critical for retimers like Twixtor — they request source frames at other times), `render`, `isIdentity`, `instanceChanged`, `getClipPreferences`.
- Pixel formats: hosts advertise supported depths (8-bit int, 16-bit int, 32-bit float) and components (RGBA/RGB/Alpha). Kiriko can advertise float-RGBA only and let plugins adapt — legal per spec, hosts commonly restrict.
- **Interact suite / custom overlays**: OpenGL-based on-canvas widgets (optional; many plugins degrade gracefully, but Mocha-style plugins need it). OFX 1.5 added a **Draw Suite** (host-abstracted 2D drawing so overlays don't require the host to hand plugins a raw GL context) — Resolve/Fusion 18.5+ and Baselight 6+ support it; RE:Vision already uses it.
- **GPU rendering**: OFX 1.4 added the (originally Resolve-driven) OpenGL render support; **OFX 1.5 formalised the GPU Rendering Suite with CUDA, Metal, and OpenCL (images and buffers)** — host passes a command queue/stream property (`id<MTLCommandQueue>`, `cl_command_queue`, CUDA stream) plus GPU-resident buffers; plugin enqueues async work and returns without syncing. Vulkan is *not* in the standard yet (discussed upstream). A wgpu/Vulkan-native Kiriko would interop via Metal on macOS and CUDA/OpenCL on Windows/Linux, or fall back to CPU float buffers (all major OFX plugins retain CPU paths).
- Honest sizing: a conformant host is **months, not weeks** — the spec is big, and the real cost is the long tail of per-plugin quirk fixing. Community consensus: every host implements OFX slightly differently and plugins carry per-host workaround tables; Natron's source (`NatronGitHub/Natron` HostSupport lib) and the openfx Support/HostSupport C++ libraries are the best references. The `openfx-misc` plugin set + ntsc-rs make excellent free conformance test subjects before touching commercial plugins.
- Business reality check: commercial vendors gate activation per-host (RE:Vision sells host-specific serials and lists supported hosts). Plugins mostly work in unknown hosts, but official vendor support for "Kiriko" requires outreach — the ASWF TSC is approachable and has an interest in new hosts.

### OFX's known design mistakes (to avoid in Kiriko's own API)
- Stringly-typed property soup: everything via untyped get/set on string keys; typos compile fine and fail at runtime.
- Underspecified threading model → divergent host behaviour; plugins ship `if (host == Resolve) ...` tables.
- No built-in UI toolkit story beyond GL interacts → plugins with rich UI (Sapphire's effect browser) do painful custom work per host.
- In-process only: a crashing plugin kills the host; no sandboxing story.
- Slow spec evolution while proprietary (2004–2022 saw few releases); versioned-suite negotiation is good, but hosts advertise versions they only half-implement.

---

## 2. Adobe AE plugin SDK and the .aex question

### The SDK model
- **Effect plugins** (`.aex` on Win, plugin bundle on macOS): C/C++ against the AE SDK. Entry point `EffectMain` receives command selectors (`PF_Cmd_GLOBAL_SETUP`, `PF_Cmd_PARAMS_SETUP`, `PF_Cmd_RENDER`, …) with `PF_InData`/`PF_OutData`; pixels via `PF_EffectWorld`. **PiPL** (Plug-in Property List, inherited from Photoshop) is a compiled resource describing the plugin without executing it — largely supplanted by dynamic outflags at GLOBAL_SETUP but still required in every plugin.
- **SmartFX**: the extension for 32-bit float and the modern render model — `PF_Cmd_SMART_PRE_RENDER` (declare what input rects you need, checkout layers at times, mix state into the frame-cache GUID) + `PF_Cmd_SMART_RENDER`. Adobe's own SDK docs concede SmartFX "has created significant technical distance between After Effects and other hosting environments."
- **AEGP plugins**: "General Purpose" plugins that link into the whole app via PICA suites — can walk/modify the project, add menu items, render, do IO. These are effectively *application extensions*, not effects; there is no meaningful way for another host to support them because they assume the entire AE object model.
- Beyond C++: ExtendScript/CEP panels and now **UXP** panels; plus the expressions engine (see §4).

### Can a third-party host load .aex plugins? (No — treat as out of scope)
- **Technically**: partially possible for the simplest legacy (non-SmartFX) effects — you'd have to re-implement `PF_InData`, dozens of PICA suites, PiPL resource parsing, AE's iteration callbacks, its World/rowbytes conventions, and its parameter UI/DRAWBOT drawing suites, all from headers plus behavioural reverse engineering of undocumented semantics. Modern plugins are SmartFX + AEGP-suite-calling + GPU (DirectX/Metal via Adobe's GPU suites) + licensing frameworks that fingerprint the host — each a rabbit hole. The valuable plugins (Twixtor, Element 3D, Particular) are exactly the ones using the deepest surface area.
- **Adobe's explicit position** (SDK, "Other Hosts" page): "Adobe neither supports nor recommends the creation of Adobe-compatible third-party hosts… while it may be possible to create a partially functional host by reverse engineering from the plug-in API specification, we do not recommend it and will not support you in doing so."
- **Legally**: the SDK is obtained under an Adobe licence agreement whose terms are aimed at building plugins *for Adobe hosts*; building a competing host against the SDK headers invites contract claims even where clean-room interop reverse engineering has fair-use precedent (Sega v. Accolade, Sony v. Connectix, Google v. Oracle on API reuse). Additionally, third-party plugin EULAs license the plugin for use *in supported hosts*; vendors' activation systems would refuse anyway. Risk/benefit is terrible: enormous engineering cost, legal exposure, vendor hostility, and the same vendors already ship OFX.
- **Precedents**: (a) **Adobe Premiere Pro** loads many AE effect plugins — but that's Adobe hosting Adobe's own API. (b) **Grass Valley EDIUS** ships an official "After Effects plug-in bridge" (64-bit AE plugins, explicitly "operation not guaranteed") — the only shipping third-party attempt found, it supports a subset and is widely reported as hit-and-miss; notable that Grass Valley is a large company with legal cover and it still only half-works. (c) Historic hosts (Boris RED, older Vegas via wrappers, Fusion's ancient AE bridge) all abandoned the approach. Conclusion: **do not attempt .aex loading. Say so explicitly in the design doc and route the demand through OFX + native effects.**

---

## 3. Kiriko's own plugin API design

### Inspiration: what CLAP got right (vs VST3, vs OFX)
- **CLAP** (Bitwig + u-he, 1.0 in June 2022, MIT-licensed): pure **C ABI**, zero platform dependencies, one header set; everything beyond a tiny core is an **extension** — a named C interface the plugin/host query by string ID and version (`host->get_extension(host, CLAP_EXT_PARAMS)`). Extensions make it future-proof without OFX's property soup: interfaces are *typed structs of function pointers*, not string-keyed value bags. It also specifies the **threading contract explicitly** (main-thread vs audio-thread annotations on every function) — the single biggest fix over both VST3 and OFX. MIT licence vs Steinberg's dual GPL/proprietary licensing (which is exactly the kind of gatekeeping that motivated CLAP after VST2's licence was killed).
- **VST3 lessons (negative)**: licence gatekeeping kills goodwill; giant C++ COM-style interface hierarchy makes bindings painful (C++ ABI across compilers is misery — hence CLAP/OFX both chose C); forced rearchitecting with no migration path burned the ecosystem.
- **OFX lessons (negative)**: see §1 — stringly-typed properties, vague threading, no crash isolation, host quirk divergence. OFX lesson (positive): *host-owned parameters and animation* — plugin declares params, host owns storage/keyframes/UI and answers `getValueAtTime`. Keep that; it's what makes plugin params first-class citizens in the timeline and in expressions.

### Recommended shape for the Kiriko Plugin API ("KFX")
1. **Stable C ABI core + named, versioned extensions** (CLAP model). Core: plugin factory, descriptor (id, name, version, categories), instance lifecycle, param declaration, render. Everything else (GPU interop, overlays, retiming/frames-needed, text, audio) is an extension negotiated by string ID + semver. Write the canonical header in C; ship first-party Rust and C++ wrappers.
2. **Typed parameter declaration, host-owned animation**: param kinds float/int/bool/choice/color/point/curve/string/file/group; declared once with ranges, defaults, flags (animatable, hidden, uses-secret). Host owns keyframes and hands the plugin a time-resolved value block per render — plugins never store param state. This gives expressions/scripting access to third-party params for free.
3. **Out-of-process by default, in-process as opt-in "trusted" mode.** Run plugins in a sandboxed worker process (one per plugin bundle or a shared pool). Crash = effect renders as pass-through with a calm badge, host survives (aligns with the household no-punishment ethos: a plugin crash must never take the project down). IPC via shared-memory rings for frames + a small message protocol for the control plane. Precedents: audio's yabridge/PluginDoctor-style bridging proves per-frame IPC latency is acceptable; for video at 24–60 fps with frames in shared memory the overhead is trivial compared to render cost.
4. **GPU texture sharing** for the fast path: platform-native shareable handles — IOSurface/Metal shared textures on macOS, DXGI shared handles/D3D12 fences on Windows, Vulkan external memory + timeline semaphores (dma-buf) on Linux; wgpu can import all three. Contract: host passes an imported texture + fence, plugin renders into a provided output texture, signals fence. Fallback contract: CPU float32 RGBA in shared memory (mandatory to implement; GPU is the optional extension) so simple plugins are simple.
5. **Versioning discipline**: ABI-frozen core structs (size-prefixed for extension), semver'd extension interfaces, host and plugin both report versions, and a published conformance test kit + a `kfx-validator` (CLAP ships a proxy layer that catches threading bugs — copy that idea). Publish under MIT/BSD from day one so vendors can vendor the headers.
6. **Adoption path**: because Kiriko also hosts OFX, KFX competes only where OFX is weak — UI-rich effects, generators with canvas interaction, retimers wanting host motion vectors, script-visible params. Don't try to out-standard the standard; KFX is the first-party API, OFX is the compatibility API.

---

## 4. Scripting & expressions extensibility

### What AE does
- Expressions = per-property JavaScript snippets evaluated per frame, returning the property value; a defined API surface (`thisComp`, `thisLayer`, `time`, `value`, `wiggle()`, `loopOut()`, interpolation helpers, layer/property navigation).
- Two engines: **Legacy ExtendScript** (ES3, 1999) and, since AE 16.0 (2019), the **"JavaScript engine" — V8**, ES2018, up to ~5× faster at render time. Adobe pre-processes legacy-syntax expressions before V8 evaluation. Scripting (app automation) is separate: ExtendScript / modern UXP.
- Lesson: the expression *API surface* (names above) matters more than the engine — thousands of copy-pasted expressions and preset packs assume `wiggle`, `loopOut`, `valueAtTime`, `posterizeTime`. If Kiriko implements the same names with the same semantics, the community's expression knowledge transfers wholesale.

### Engine options for a native app
| Engine | Size/embed | Speed | Sandboxing | Determinism | Notes |
|---|---|---|---|---|---|
| **QuickJS(-ng)** | tiny C, few files, trivial embed | interpreter (~V8-jitless class); fine for per-property snippets | excellent — memory/time limits built in, no ambient IO | strong: refcount GC, no JIT tiers, same behaviour every run/platform | ES2020+; the pragmatic default |
| **V8 (rusty_v8/deno_core)** | huge dep, heavy build | fastest by far (JIT) | good — isolates, heap limits (Cloudflare Workers model); deno_core adds ops/permissions | JIT tiering is internal-nondeterministic but *results* are deterministic if API is pure; float math IEEE-consistent | right choice only if expressions become a perf bottleneck across thousands of properties |
| **Hermes (static)** | mid | good (bytecode AOT) | good | good | worth watching; Adobe itself is rumoured to explore it, RN pedigree |
| **rhai / Lua(JIT)** | tiny | LuaJIT very fast | good | good | wrong-language: breaks AE expression compatibility, which is the whole point |

- **Recommendation**: QuickJS-ng embedded in the render core, with the AE-compatible expression API implemented natively (Rust) and exposed as host functions. Per-frame evaluation of small snippets is exactly QuickJS's sweet spot; V8's JIT advantage mostly evaporates on 200-character snippets with host-call-dominated cost, and QuickJS gives byte-identical behaviour on every render node.
- **Determinism rules for render-farm/cross-machine consistency** (write these into the spec): pin one engine version per project-file version; forbid ambient nondeterminism in the expression environment — no `Date.now`, no wall clock, `Math.random` replaced by AE-style `seedRandom(seed, timeless)` (seeded, per-layer/property, exactly AE's model); no IO, no async, no shared mutable globals across properties; fixed evaluation order defined by the dependency graph; document IEEE-754 double semantics (JS numbers are doubles — bit-identical across x86/ARM for the ops JS exposes, unlike C fast-math). Time-limit + memory-limit each evaluation (QuickJS interrupt handler) so a runaway expression degrades to last-good-value + calm badge rather than freezing the app.
- App-level scripting (automation, panels) is a separate, capability-gated runtime — deno_core or QuickJS with an explicit permission model (Deno-style: fs/net grants per script) — not the per-frame expressions engine.

---

## 5. AE project import (.aep)

### The format
- `.aep` is a **RIFX container** (RIFF with big-endian sizes; first four bytes `RIFX`, form type `Egg!`). Inside: nested LIST chunks (`Fold`, `Item`, `Layr`, `tdgp` property groups, `tdbs`/`tdb4` property metadata, `cdat` keyframe/value data, `Utf8` name chunks, `ldta` layer data, `cdta` comp data, `fdta`…). Chunk *shapes* are known; many field *semantics* are still guesswork. AE can also "Save a Copy As XML" (`.aepx`, since CS4, still present in current AE) — but the XML mirrors the RIFX tree and embeds the interesting binary chunks as hex/base64 blobs, so XML does not remove the reverse-engineering problem, it just removes the RIFF parsing.
- **Public reverse engineering state**:
  - `boltframe/aftereffects-aep-parser` (Go): items/folders/comps, layers, effect instances and (match-name-identified) properties; explicitly partial; used Kaitai for exploration.
  - `forticheprod/aep_parser` (Python, Kaitai Struct .ksy): the most complete open description — items, layers, effects, markers, keyframes for many property types; actively maintained by a studio (Fortiche — the Arcane studio) for pipeline introspection, not full round-trip.
  - `inlife/aftereffects-project-research`: raw notes + sample projects, older.
  - Nobody has public **write** support or complete keyframe/bezier semantics for every property class; effect parameters are stored keyed by match name with type-specific binary layouts, and third-party effect param blobs are opaque.
- Realism: direct parsing today can recover project structure (folders/comps/layer stacks, sources/footage paths, layer in/out/start, basic transform keyframes) but bezier easing (influence/velocity pairs), per-channel expression storage, masks-with-feather details, and text layer data get progressively hairier, and Adobe changes chunk details across versions with zero documentation.

### The proven pattern: run inside AE and export JSON (bodymovin's approach)
- **bodymovin/Lottie**: a CEP extension whose ExtendScript backend walks the live AE DOM (compositions → layers → properties) and serialises to Lottie JSON. It never touches .aep bytes — AE itself is the parser. This is *the* battle-tested route: every property, keyframe, easing handle, mask path, and expression string is available through the scripting DOM with documented semantics, and it works on any .aep the user's AE can open (any version, since AE upconverts).
- **AEUX** (Google, ex-sketch2ae): the reverse direction (Sketch/Figma → AE) — a panel + ExtendScript that ingests a JSON layer description and *builds* AE layers. Relevance: it defined a practical JSON interchange schema for design layers and proved the panel+JSON pipeline in both directions. (Note for the doc: AEUX imports *into* AE; the exporter-out-of-AE precedent is bodymovin/Lottie, plus tools like Overlord (AE↔Illustrator live) and LottieFiles' plugin.)

### Strategy options, ranked
- **(a) Kiriko Exporter panel (ExtendScript/CEP now, UXP later) — RECOMMENDED.** Ships as a free .zxp; walks `app.project` and emits "Kiriko Interchange JSON" (superset of what Lottie captures: comps, all layer types, full keyframe data incl. temporal/spatial bezier params via `keyInTemporalEase`/`keyOutTemporalEase`/`keyInSpatialTangent` etc., expressions as source strings, masks, track mattes, blend modes, time remap, parenting, cameras/lights, effect instances as match-name + param values/keyframes, footage references with relative paths). Requires the user to own AE for the migration moment only — acceptable: the target user is migrating *from* AE. This gets ~90% semantic fidelity for the cost of one script, with zero legal exposure (the scripting DOM is documented public API).
- **(b) Direct .aep parsing — secondary, best-effort.** Build on the Kaitai `.ksy` from forticheprod (check licence; it's open) for "open an .aep without AE" — recover comp/layer/keyframe skeleton, show unsupported bits as placeholders with a report. Never promise fidelity; version-fragile. Worth having because "drag an .aep in and get *something*" is a killer demo and covers users who no longer have AE installed.
- **(c) Lottie JSON import — cheap, do it.** Well-documented schema, `lottie-web` as reference implementation, huge template ecosystem (LottieFiles). Limited to shape/text/image layers + supported effects, but it's a free interchange on-ramp and the parser doubles as a validator for (a)'s schema thinking.
- **(d) What maps vs not** (honest table for the doc):
  - Maps cleanly: comp hierarchy/settings, layer types (solid/footage/precomp/shape/text/null/adjustment/camera/light), transforms + parenting, keyframes incl. bezier ease + hold, spatial paths, masks + modes + feather, track mattes, blend modes, time remap/stretch, markers, guides, expressions (source strings — re-evaluated by Kiriko's engine; will run to the extent the API surface is implemented), audio levels.
  - Maps partially: text layers (per-character 3D, animators — big surface), layer styles, camera DOF/lights (if Kiriko has 3D), effect *parameters* for effects where Kiriko has an equivalent (match-name → Kiriko-effect mapping table: Gaussian Blur, Curves, Glow, Transform, Fill…) or an installed OFX equivalent.
  - Does not map: **third-party effect internals** (Element 3D scenes, Particular systems — parameters exist as blobs/values but the renderer doesn't), AE-specific renderers (C4D/Advanced 3D), Rotobrush strokes, puppet pins (format known-ish but engine differs), essential graphics rigs, plugin-private data. Import policy: keep unknown effects as inert "missing effect" placeholders that preserve params, so round-trip/repair is possible.

---

## 6. Presets & template ecosystems

- **.ffx animation presets**: RIFX binary (same container family as .aep; chunks like `tdmn` match names, `tdsp`/`tdot`, `bescbeso`, `FaFX` head; property data mirrors .aep property blobs). No public complete parser; closed and version-drifting. **Feasibility of native .ffx import: low — skip for v1.** Two mitigations: (1) the AE exporter panel from §5(a) can *apply a preset inside AE* then export the resulting properties (presets become just properties — free coverage for the migration flow); (2) text-animator-style presets could later ride the same partial RIFX property parser as (b). Note pseudo-effects: much of the community "preset" economy is .ffx files that only define parameter UI (pseudo effects) driven by expressions — those become importable the moment expressions + custom param groups exist.
- **Community CC/preset packs for gaming edits** (the "CC pack" economy on YouTube/Payhip): overwhelmingly **just .aep project files** (+ occasional .ffx and font folders) containing adjustment-layer stacks of built-in effects + expressions. They import via route §5(a)/(b) to the extent the referenced effects exist in Kiriko — which argues for prioritising AE-built-in-effect parity (Glow, CC effects equivalents, Curves, Optics Compensation-style distort, Directional Blur…) since that's what packs are made of.
- **Motion-graphics template markets** (Envato/Motion Array .aep templates, .mogrt for Premiere): .mogrt is a zip of an .aep + Essential Graphics manifest — same import problem as .aep plus a JSON manifest; possible later, not v1. Lottie/LottieFiles marketplace is importable via §5(c) immediately.
- **OFX side presets**: Sapphire/Continuum presets are vendor-format but load *inside the plugin's own UI*, so they come along free once the plugin runs under Kiriko's OFX host.

---

## Bottom line for the design doc
1. **OFX host: yes** — it is the only sanctioned road to Twixtor/RSMB/Sapphire, the exact plugins Kiriko's audience needs, all of which already ship OFX builds proven in Vegas/Resolve. Budget a full milestone (property/param/image-effect suites, float-RGBA, CPU first, then OFX 1.5 GPU suite per platform + Draw Suite), use Natron + openfx-misc + ntsc-rs as the conformance bench, and engage the ASWF TSC early.
2. **.aex loading: no** — technically a swamp (SmartFX/AEGP surface), explicitly unsupported by Adobe, legally exposed, and redundant given OFX coverage of the plugins that matter. State this in the doc.
3. **First-party API**: CLAP-shaped C ABI ("KFX") — typed extension interfaces, host-owned animated params, out-of-process sandbox with shared-memory frames and platform shared-texture fast path, MIT-licensed headers + validator.
4. **Expressions**: QuickJS-ng with AE-compatible API surface (`wiggle`, `loopOut`, `seedRandom`…), deterministic-by-construction rules for farm consistency; deno_core/QuickJS with Deno-style permissions for app scripting.
5. **AE import**: primary = free ExtendScript/CEP exporter panel running inside AE emitting Kiriko Interchange JSON (bodymovin's proven pattern, ~90% fidelity); secondary = best-effort direct RIFX .aep parsing built on the Kaitai community work; plus cheap Lottie import. Unknown effects preserved as placeholders. Skip native .ffx; cover presets via the in-AE export path.

### Key sources
- https://openeffects.org/ ; https://openfx.readthedocs.io/en/main/ (spec, 1.5 release notes, Rendering/GPU, Image Effect Suite, OfxHost struct)
- https://www.aswf.io/blog/openfx-v1-5/ ; ASWF adoption PR (Aug 2022); https://github.com/AcademySoftwareFoundation/openfx (BSD-3)
- https://revisionfx.com/downloads/… (Twixtor V8 / RSMB V6 OFX host lists) ; https://ntsc.rs/docs/openfx-plugin/
- https://github.com/NatronGitHub/openfx-misc ; https://github.com/NatronGitHub/Natron
- https://ae-plugins.docsforadobe.dev/ (SDK overview, SmartFX, PiPL, AEGP, **ppro/other-hosts** for the Adobe quote)
- EDIUS AE plug-in bridge: https://wwwapps.grassvalley.com/manuals/EDIUS7_USER_EN/l06/l0_6_l1_15_l2_3.html
- https://github.com/free-audio/clap ; librearts.org CLAP retrospectives ; https://github.com/Tremus/CPLUG
- https://helpx.adobe.com/after-effects/using/legacy-and-extend-script-engine.html (V8/ES2018 vs ExtendScript/ES3) ; motiondeveloper.com engine articles
- https://github.com/boltframe/aftereffects-aep-parser ; https://github.com/forticheprod/aep_parser ; https://github.com/inlife/aftereffects-project-research ; justsolve.archiveteam.org/wiki/After_Effects
- https://aescripts.com/bodymovin/ ; https://github.com/airbnb/lottie-web ; https://google.github.io/AEUX/
- .ffx: fileinfo/filext format notes; rendertom/PseudoEffect
