# Retiming / speed-ramping research — Vegas Pro, After Effects, Premiere Pro

Research for Kiriko's hybrid retime system (AE-style graph editing + Vegas-style clip-on-track workflow).
Date: 2026-07-12. Sources: VEGAS docs/forums (vegas-magazine.com, helpmax mirror of official docs, Creative COW VEGAS forum, vegascreativesoftware.info — note that forum now 301-redirects to forum.borisfx.com), Adobe helpx (via mirrors — helpx.adobe.com timed out repeatedly; content verified via nexrender/motionarray/noble/pond5 which quote the official mechanics), Blackmagic forum, RE:Vision/Boris FX.

---

## 1. VEGAS Pro velocity envelopes

**Model.** A velocity envelope is a per-event automation curve overlaid directly on the event on the timeline (green line). Its domain is **event time** (not source time); its value is **instantaneous playback rate as a percentage**. It is inserted via right-click → Insert/Remove Envelope → Velocity.

- **Range**: −100% to **+1000%** in current versions (VEGAS 11-ish and earlier capped at **+300%**; the cap was a long-running community complaint — threads titled "is it possible to speed up velocity past 300%?" etc.). 100% = normal, 0% = freeze, negative = reverse.
- **Reverse**: any negative value plays the media backwards at that rate. Constraint: an event **cannot reverse past the first frame of the media file** — once the integrated source position hits 0, VEGAS holds frame 0 for the remainder of the event. So reverse is clamped by the source extent, silently.
- **Freeze frames**: set a point (or a run between two points) to 0%. Editors also use the point's **Hold** fade type to sustain a value (Hold sustains any velocity, not just 0).
- **Point editing**: double-click the line to add a point; drag up/down (Ctrl+drag for fine increments); right-click point → "Set to…" for an exact numeric value; presets for 0% and 50%. **Curve between points** is set per-segment by right-clicking the segment: Linear, Fast (log), Slow (log), Smooth (cubic), Sharp (cubic), Hold — the same fade-type vocabulary as every other Vegas envelope. There are **no bezier handles** — this is the big precision limitation vs AE/Premiere (forum thread: "Curved Velocity Envelopes?").
- **Event length interaction — the defining trait**: **the event never auto-resizes.** Velocity changes how fast source media is consumed inside a fixed event box. Speed up and the media runs out before the event's right edge — VEGAS then **loops the media by default** (or holds, if looping is off) past a small triangular **notch ("tiny V") drawn at the top edge of the event marking where the media actually ends**. Slow down and the event now only shows the front portion of the take. Editors must manually trim the event edge to the notch. This is simultaneously the most-complained-about paper cut ("Velocity Envelope duration" threads) and the reason the workflow scales: the event is a stable rectangle you can butt against other events.
- **Audio**: velocity envelopes are **video-only**; the paired audio event is untouched (feature-request threads exist for audio velocity). Montage editors don't care — they mute game audio and cut to music.
- **Semantics note**: VEGAS integrates the envelope — the source position at event time t is ∫₀ᵗ v(τ)dτ (v as a fraction). The envelope is the *speed lens*; there is no exposed value-graph/time-map lens at all. That is exactly the half Kiriko can add.

**Supersampling** (project-level "video supersampling envelope", set on the timeline ruler): raises the internal temporal sampling rate (1–8×) so that **VEGAS-generated motion** — track motion, pan/crop keyframes, transitions — is computed at sub-frame positions and blended. Crucially it **does not synthesise new frames from source video**; community consensus (Creative COW "Video Supersampling. Does anyone use it?") is that it does nothing for velocity-slowed source footage and is widely misrecommended for slow motion. Worth knowing so Kiriko doesn't cargo-cult it.

**Resample modes** (per-event property, current trio): **Smart Resample** (VEGAS decides; blends adjacent frames when event rate ≠ project rate), **Force Resample** (always blend), **Disable Resample** (nearest-frame / step cadence). Resample here = **frame blending**, not optical flow. Trade-off reported on forums: Disable gives "ghost-free" crisp frames but visible choppiness/judder on speed changes (3× on 59.94fps = discarding 2 of every 3 frames); Smart/Force give smooth motion but ghosting/"excessive motion blur" that gaming editors hate. Gaming/montage editors near-universally set **Disable Resample** as their first-project-setup step and accept "a little choppy over the blur". VEGAS has **no built-in optical-flow retime**; the answer to smooth extreme slow-mo in Vegas is "shoot high fps" or "buy Twixtor".

**Why montage/gaming editors love this workflow** (Se7enSins tutorial, countless "velocity sync" YouTube tutorials for Minecraft/Fortnite/CoD montages):
1. Everything happens **on one track**: drop clip, ramp it, razor it, butt the next clip against it. No precomps, no nesting, no layer stack.
2. The **3-point pattern**: point before the kill, point on the kill, point after — middle segment fast, hold the moment slow — hand-synced to a waveform on the audio track directly below. Beat alignment is visual (envelope points vs waveform transients on adjacent tracks).
3. Fixed event length means ramping a clip **doesn't ripple the rest of the timeline** — beats already synced stay synced.
4. Envelope is visible and editable **in situ at timeline zoom level** — no modal graph editor, no second panel.

## 2. Vegas: playback rate vs velocity envelope vs time stretch

Three stacked, independent mechanisms:

| Mechanism | Where | Range | Shape | Audio |
|---|---|---|---|---|
| **Playback rate** | Event Properties dialog (video event) | 0.25–4.0× | constant | video-only (audio event has its own pitch/stretch props) |
| **Ctrl+drag time stretch** | drag event edge with Ctrl | 25%–400% (same 0.25–4× internally — it *sets* playback rate to fit the new length) | constant | stretches paired audio with pitch preservation options |
| **Velocity envelope** | envelope on event | −100%…+1000% | time-varying curve | video only |

They **multiply**: old-cap era editors stacked 300% envelope × 400% rate = 12×; today 1000% × 4× = **40×** max without pre-rendering. Beyond that: render-and-reimport, or nest .veg project files (nesting multiplies again). Key conceptual difference: Ctrl+drag is *length-driven* (choose the duration, rate follows — the only Vegas mechanism where duration and rate are linked); velocity envelope is *rate-driven* (choose the rate curve, then hand-trim the length).

## 3. After Effects time remapping

**Model.** Enabling Layer → Time → Enable Time Remapping adds a **Time Remap property**: an animatable channel whose **value is source time (seconds) and whose keyframes sit in comp/layer time** — i.e. the canonical **time-map f: comp time → source time**. AE seeds two keyframes (first frame ↦ 0, last frame ↦ source duration) and makes the layer's out-point freely extensible: **beyond the last keyframe the value holds, so the layer can be stretched past the source and shows a held frame** (basis of freeze-and-extend, loops via `loopOut` expressions, etc.).

- **Value graph**: plots f directly. Slope 1 = normal speed, <1 slow, >1 fast, 0 = freeze, **negative slope = reverse** (values simply descend). Non-monotonic curves are fully legal — palindrome loops, scrubby "time echo" effects.
- **Speed graph**: the same channel viewed as **derivative f′** (shown in seconds/second). You can edit it, but for Time Remap this is notorious: adjusting speed-graph influence handles indirectly reshapes the value curve and easily produces unintended reverse dips or kinks; every serious tutorial ("Time remapping graph not looking right", Adobe community) tells you to do speed ramps **in the value graph with eased keyframes** and use the speed graph read-only for verification. Lesson for Kiriko: the derivative *view* is loved, the derivative *edit* on top of an integral store is where AE fumbled the UX.
- **Freeze frames**: Layer → Time → Freeze Frame plants a single **Hold keyframe**; or two identical-value keyframes; hold segments render as flat runs in the value graph / zero in the speed graph.
- **Time stretch** (Layer Stretch %) is the separate constant-factor mechanism (linear rescale of the whole layer, changes layer duration, −100% reverses); **Timewarp** effect is the third mechanism (effect-based, optical-flow capable, speed- or source-frame-driven).
- **Frame synthesis**: per-layer Frame Blending switch — **Frame Mix** (blend/ghost) or **Pixel Motion** (per-pixel optical flow). Community guidance: try both, pick the cleaner; Pixel Motion is higher quality until it hits occlusions, then it smears.
- **Constraints/pain**: remapping a nested comp remaps *the entire comp's contents* (all animation inside speeds up too); stills/text must be **precomposed** before they can be remapped; heavy time effects conflict.

## 4. Premiere Pro time remapping

**Model.** Per-clip **Time Remapping → Speed** channel, displayed as a horizontal white **rubber band** across the clip in the track (and in Effect Controls). This is a **speed-lens UI over a keyframed speed channel** — Premiere's storage is speed keyframes, not a value graph; no value-graph view exists.

- Ctrl/Cmd-click (or pen tool) the band to add a **speed keyframe**; drag band segments up/down (Shift = 5% steps). Range ~**1%–1000%** per segment.
- A speed keyframe is initially an instant step. **Alt/Option-drag splits the keyframe into two halves**; the gap between the halves becomes a **linear ramp**, and dragging the blue **bezier handles** that appear curves the ramp (ease in/out). This "split-the-keyframe" gesture is Premiere's signature — and its most-reported fiddliness.
- **Reverse**: Cmd/Ctrl-drag a keyframe's right half backwards — creates a segment marked with **left-pointing arrows** that plays in reverse, and Premiere **automatically appends a forward-playing segment** returning to where you were ("it goes backwards, then shows the clip as it originally was forward") — you can't just end in reverse; editors razor off the unwanted tail. Reverse-only sections are awkward.
- **Freeze**: Cmd+Alt-drag a keyframe apart — creates a **freeze segment marked with vertical tick marks (hatching)**.
- **Duration behaviour**: like Vegas, **the clip's timeline duration does not change and nothing ripples** — speeding a section up just consumes more source before the fixed out-point (or reveals media past it); slowing down means the out-point shows an earlier source frame. Contrast the *other* Premiere mechanisms — Speed/Duration dialog and Rate Stretch tool — which are constant-factor and do change duration (with an optional "Ripple Edit, Shifting Trailing Clips" checkbox).
- **Interpolation**: per-clip Time Interpolation = **Frame Sampling / Frame Blending / Optical Flow**; Optical Flow needs rendering and exhibits the usual warping.
- **Extras**: optional time-remap **motion blur** with shutter-angle control (Effect Controls).
- **Pain points** (Adobe community, Creative COW): the rubber band needs a **tall track** to be usable at all — at default track height it is unclickable; recurring regressions where the band "won't move up nor down" (acknowledged bug in 14.3.2, recurred later); keyframes hard to grab/drag at timeline zoom; and the killer interaction bug: **Time Remapping and Motion/effect keyframes coexist badly** — effect keyframes are indexed to clip time, so retiming after animating shifts every effect keyframe, with "widely varied behaviour". No numeric entry on the band itself (must round-trip to Effect Controls).

## 5. Pain-point summary per approach

**AE**: layer-based — every cut is a new layer, so "cut ramped clips back-to-back" means stacking/staircasing layers or precomposing; no ripple concept; ~5–8 min of Graph Editor per ramp reported (third-party ramp-preset tools exist to sell around this); enabling remap on a comp remaps everything inside; must manually drag the layer out-point after slowing (mirror of Vegas trimming, but in a modal, non-timeline way); speed-graph editing of the remap channel is treacherous.
**Vegas**: no bezier control (fixed fade-type curve vocabulary only); historical 300% cap and current 1000%×4× ceiling force render-and-reimport or .veg nesting for extreme speed; manual trim-to-notch after every ramp (loop-by-default bites beginners — clip silently repeats); envelope is video-only (audio doesn't follow); no time-map view (you can't see *which source frame* lands on a beat, only the rate curve); no optical flow.
**Premiere**: band unusable at default track height; split-keyframe/bezier gesture fiddly and bug-prone across versions; reverse forces an appended forward segment; effect keyframes not retime-aware; speed-lens only — no value graph anywhere.

## 6. Optical flow / frame interpolation for slow motion

- **Mechanics**: estimate per-pixel motion vectors between frame pairs; synthesise intermediates by warping both neighbours along the vectors and blending. Quality ladder: nearest/duplicate frames (Vegas Disable Resample, Premiere Frame Sampling) → frame blending/ghosting (Vegas Smart/Force Resample, Premiere Frame Blending, AE Frame Mix) → optical flow (AE Pixel Motion, Premiere Optical Flow, Resolve Optical Flow "Enhanced Better") → **ML-assisted flow** (Resolve **Speed Warp**, DaVinci Neural Engine; RIFE-family open models) → **Twixtor Pro** (RE:Vision proprietary per-pixel tracking; Pro tier adds **foreground/background separation masks and tracking-point guidance**, which is why it survives — you can hand-fix occlusions).
- **Typical failure artefacts** (consistent across Avid/Blackmagic/Creative COW/Philip Bloom discussions): **tearing/stretching at occlusion boundaries** (objects crossing, limbs against background — the classic "rubber legs"); **frame-edge warping** on pans/dollies (new content entering frame has no correspondence); **motion-blurred elements warp massively**; repetitive textures and low-contrast regions confuse matching; very large per-frame displacement breaks correspondence entirely.
- **Quality expectations**: Speed Warp "close to no artifacts" at 2× slow on clean footage but extremely processor-intensive (users bake/cache it); Twixtor remains preferred where hand-fixing is needed; all methods degrade sharply below ~20–30% speed on 24/30fps source — the community rule of thumb is 60fps source for 30–40% playback, 120fps+ for 20%, interpolation only bridging the remainder.
- **Kiriko implication**: retime data model and frame-synthesis engine must be decoupled; the model should expose *fractional source-frame positions* to whatever synthesiser (nearest / blend / flow) is active, per-clip, with per-clip mode override (the Vegas "Disable Resample for gameplay" culture is a real requirement, not a legacy quirk).

## 7. Data-model options for Kiriko's hybrid retime

### Shared invariants (any option)

- **Canonical store = time-map** f: clip-output time → source time (AE's representation). Reasons: it's exact under editing (no integration drift), reverse is just negative slope, freeze is zero slope, cut = evaluate f at the razor point, and the speed view is derivable. Storing speed canonically (Vegas/Premiere) forces integration to find source frames — cutting a speed-keyframed clip requires integrating from the clip head every time, and float integration error accumulates and shifts frames.
- **Speed lens = derivative view**: render s(t) = f′(t) as the Vegas-style overlay on the clip; edits made in the speed lens are compiled back into the value store by **local re-integration with pinned anchors** (see below). Both lenses always available; sub-frame sampling everywhere (f returns float source seconds; synthesiser decides nearest/blend/flow).
- **Reverse**: permitted iff the curve model allows negative slope; clamp source domain to [0, source duration] with explicit **hold regions** at the ends (Vegas silently holds frame 0 on reverse-past-start — make that state visible, an "exhausted" hatch like Premiere's freeze ticks / Vegas's notch).
- **Cut boundaries**: razor at output time t splits into two clips whose maps are f restricted to each side; right clip's map re-based so its f(0) = f(t) exactly (frame-accurate, no re-integration). Each clip is independent after the cut — Vegas behaviour, what montage editors expect.
- **Duration coupling**: retime never auto-ripples (Vegas/Premiere behaviour, beat-sync-safe) but the UI must show the **media-exhaustion notch** and offer one-keystroke "trim/extend event to media end", optionally as a ripple or non-ripple trim. This single affordance fixes Vegas's worst paper cut without giving up the fixed-box model.

### Option A — Piecewise-cubic monotonic-by-default value curve ("AE store, Vegas skin")

Store per clip: ordered keyframes {(tᵢ, sᵢ, interp, tangents)} in output time with cubic-Hermite segments; a per-segment `hold` flag; a clip-level `allowReverse` toggle (off = monotone-clamped tangents à la Fritsch–Carlson, so speed-lens edits can never accidentally dip negative — directly fixing AE's speed-graph footgun; on = full AE freedom).
Speed-lens edit semantics: dragging a speed segment scales the slope of the corresponding value segment(s) while **pinning the keyframe source-values on either side of the edited region's neighbours**; the slack is absorbed by the next segment (Vegas feel: "everything to the right shifts in source, event length fixed"). A modifier pins *nothing* and shifts all downstream source values (AE feel).
Pros: one curve type, exact razoring, both lenses lossless (derivative of cubic is quadratic — drawable exactly). Cons: speed lens shows curved (quadratic) segments rather than Vegas's straight lines; users pasting "set 300% here" get a value-curve whose speed isn't constant unless tangents are managed — needs a "constant-speed segment" keyframe mode.

### Option B — Piecewise-linear-in-speed segments with exact rational integration ("Vegas store, AE skin")

Store per clip: speed keyframes {(tᵢ, vᵢ, ease)} where segments are constant/linear/eased in **speed**, plus a stored, always-maintained **source offset table**: for each keyframe, the exact integrated source position Sᵢ (kept as rational/fixed-point, recomputed incrementally on edit). The value graph is derived (piecewise quadratic) and *editable*: dragging a value-graph point solves for the segment speeds that pass through it.
Pros: speed lens is native and matches editor intuition ("300% here"), constant-speed runs are first-class, beat-sync workflow is 1:1 Vegas. Cons: value-graph edits are a solver (can fail/need clamping); non-monotonic freeform curves (scrubby time-art) awkward; two coupled arrays to keep consistent — the offset table is essentially caching the integral, and every edit must invalidate downstream entries (bug surface).

### Option C — Dual-primitive segments (recommended)

Store per clip an ordered list of **retime segments** over output time, each one of exactly two primitive kinds, plus the exact source position at each boundary (part of the segment, not a cache):
1. **RateSegment { duration, v₀, v₁, ease }** — speed-defined; source advance = closed-form integral (linear/eased speed integrates to quadratic/known form; store boundary source positions as rationals so razoring is exact).
2. **MapSegment { duration, cubic value curve }** — value-defined; for freezes, reverses, and hand-drawn graph work.
Plus segment flags: `hold`, `reverse-allowed`. The two lenses simply edit whichever primitive a segment is, and **lens editing converts primitives on demand** (grab a RateSegment in the value graph → it converts to MapSegment, and vice versa, with exact boundary preservation). A whole-clip constant rate is one RateSegment — unifying Vegas's three mechanisms (playback rate / stretch / envelope) into one model: Ctrl+drag-stretch = scale the single RateSegment's duration holding source span; envelope point = split RateSegment.
Invariants: boundary source positions are authoritative and continuous (C0 always; C1 optional per boundary — a visible "kink" badge when speeds mismatch, since montage editors *want* hard rate cuts); reverse only inside MapSegments or RateSegments with v<0 and `reverse-allowed`; source-domain clamping produces explicit trailing HoldSegments rather than silent behaviour.
Pros: each lens edits its natural primitive losslessly (no solver in the common paths); constant-speed is exactly constant; razor = split segment with exact boundary math; serialisation is small and human-diffable (good for a docs-first repo). Cons: two primitive types = more code paths; conversion rules must be specified carefully (spec them in the design doc: RateSegment→MapSegment is exact; MapSegment→RateSegment is a least-squares fit and warns).

### Recommendation

**Option C**, with Option A's monotone-clamp default for the speed lens. It gives Vegas's workflow truths (fixed clip box, one-track cutting, no ripple, in-situ speed overlay, hard-cut-friendly C0 boundaries) on AE's mathematical foundation (exact output→source map, value-graph power, reverse and freeze as first-class), while keeping frame synthesis (nearest/blend/optical-flow) an orthogonal per-clip render policy exactly as all three NLEs ended up doing.

---

### Key sources
- Vegas velocity envelope detail: vegas-magazine.com/velocity-envelope/ (range −100…1000%, notch, no auto-resize, Set-to, presets); vegaspro.helpmax.net Event Envelopes; Creative COW threads: "Velocity Envelope duration", "More than 300% velocity increase", "velocity faster than 4x", "Velocity with Disable Resample", "Video Supersampling. Does anyone use it?", "Curved Velocity Envelopes?" (vegascreativesoftware.info, now redirecting to forum.borisfx.com)
- Vegas montage/beat-sync culture: Se7enSins "How to: Sync with Velocity (Sony Vegas)"; many Vegas velocity-sync tutorials (Minecraft/Fortnite montage community)
- AE: helpx.adobe.com/after-effects/using/time-stretching-time-remapping.html (fetched via summaries; direct fetch timed out); nexrender.com/blog/time-remapping-after-effects; motionarray.com AE time-remap guide; Adobe community "Time Remapping graph not looking right"
- Premiere: helpx.adobe.com change-clip-speed-and-duration-using-time-remapping (via summaries); blog.pond5.com time-remapping guide; blog.nobledesktop.com (reverse arrows + forced forward tail); Adobe community "Can't drag time remapping keyframes" / 14.3.2 FAQ; Creative COW "Time Remapping conflict with Motion and other Effect Keyframes"
- Interpolation: revisionfx.com/products/twixtor; borisfx.com "Best Twixtor Alternative"; forum.blackmagicdesign.com "Optical Flow" (Speed Warp quality/cost, edge + motion-blur warping); blog.frame.io "Mixing Frame Rates part 3"; philipbloom.net Twixtor extreme slow-mo; Avid community "Slo mo artifacts"
