# Lumit design language

**Status: canonical.** This is Lumit's `docs/DESIGN.md` in the household sense: the app-specific
design spec that stacks on top of the shared Aizome design language
(`learningLanguageMachine/docs/HOUSEHOLD-DESIGN.md`). Panel inventory, docking behaviour, and
interaction flows live in [07-UI-SPEC.md](07-UI-SPEC.md); this document owns colour, type,
density, motion, and voice. Terminology follows [01-GLOSSARY.md](01-GLOSSARY.md) exactly.

RFC-2119 keywords (MUST, SHOULD, MAY) are used with their usual force.

---

## 1. Relationship to the household system

Lumit is a household app. It inherits the Aizome design language wholesale, with two recorded
deviations (§1.2). Anyone who has used Michi, Mishka Hub, or Sukumo should recognise Lumit as
a sibling within seconds — the same accent, the same type, the same restraint — even though it
is a dense professional tool rather than a web app.

### 1.1 Inherited unchanged

- **Semantic tokens only.** Every colour in the application comes from a named theme token.
  Since egui is not CSS, the token layer is a Rust struct (§4) rather than `theme.css`, but the
  rule is identical: a hex literal in widget code is a lint failure. The sole sanctioned
  exception, per the household rule, is the application icon / favicon set.
- **Type stack.** Schibsted Grotesk for display (wordmark, workspace titles, dialog headings);
  Source Serif 4 for rare accent lines (about box, empty states); Inter for body and panel
  copy; **JetBrains Mono for all numbers** — timecode, frame numbers, speed percentages,
  property values, layer indices, labels, attribution. No exceptions to the mono-for-numbers
  rule anywhere in the UI.
- **Radii**: 4px (dense elements — clips, keyframe flags, thumbnails), 8px (buttons, inputs,
  chips), 16px (floating cards, dialogs), full (pills, playhead grab handle). No other radii.
- **Hairline elevation.** Panels and cards are flat fills separated by 1px hairline borders.
  Shadows are reserved for things that genuinely float: undocked panels being dragged, modal
  dialogs, drop-down menus, drag ghosts. No glassmorphism, no gradients-as-chrome.
- **One accent.** `clay` (Aizome crimson) is the single saturated accent: primary buttons,
  selection, the playhead, active states. Nothing else competes for that role.
- **No punishment UI.** Errors and warnings render in `fig` and `kraft`, never a red-alert.
  A dropped frame counter is information, not an alarm. Uncached timeline regions are neutral,
  not threatening (§6.3).
- **Sentence case everywhere.** ALL-CAPS exists only as the household kicker pattern:
  JetBrains Mono, 11px, +0.08em tracking, muted colour (e.g. `SOURCE TIME`, `WORK AREA`,
  `EXPORT QUEUE`).
- **Voice**: British English, calm, no exclamation marks, no emoji-as-excitement, one rationed
  running joke (§10).
- **Motion philosophy**: the user controls tempo; nothing auto-advances; a complete
  reduced-motion path exists (§8).

### 1.2 Recorded deviations

- **KD-1 · Dark-first (= decision K-004, DECIDED).** The household default is paper-light with
  a `.dark` night-print variant. Lumit inverts this: dark is the native, first-built,
  first-polished theme, because a neutral dark surround preserves colour judgement against the
  Viewer — the industry-standard reasoning behind every grading suite. Light mode is a
  documented later option (§11), not a launch requirement.
- **KD-2 · Hit-target compensation (PROPOSED; promote to the decision log per §Open
  questions).** The household accessibility gate demands ≥44px touch targets. In a timeline
  where twenty layers must be visible at once, 44px rows are impossible. Lumit's recorded
  compensation: dense-surface controls (timeline rows, keyframes, property lanes, graph editor
  handles) MUST be ≥24px in visual extent on their smaller axis **and** MUST carry ≥32px of
  interactive hit-slop; toolbar, transport, and dialog controls keep the full household ≥44px.
  See §7.2.

### 1.3 What does not apply

Lumit is a native desktop application, so the household web skeleton (React/Vite/Tailwind,
`sync-theme.sh`, FastAPI monorepo, Mishka Hub auth proxy, mobile bottom tab bar, PWA icon
rules) does not apply. The Aizome *values* still derive from the canonical `theme.css`; when
that file is repainted, Lumit's theme struct SHOULD be re-derived in the same change wave.
Person-identity colours (person 1 = `clay`, person 2 = `sky`) have no meaning in a single-user
pro tool, but `sky` remains reserved fleet-wide and MUST NOT be repurposed as a second accent.

## 2. The dark ramp

The household dark theme ("night print") is a deliberately *tinted* deep indigo
(`#0f1d2b` family). Lumit MUST NOT use it as-is: a blue-cast surround skews the eye's white
balance and corrupts grading decisions in the Viewer. Lumit's ramp is therefore derived from
the night print by stripping chroma almost entirely — the indigo survives only as a whisper
(chroma so low it reads as neutral at arm's length) — and the immediate Viewer surround drops
to strictly neutral grey.

As of K-084 the ramp's *structure* follows rerun.io's viewer (`re_ui`): a near-black canvas,
panels one small step above it, and floating surfaces a clear step above those. The hues stay
Lumit's own — this section's values are the K-084 system.

### 2.1 Surface ramp

Five surface levels on the rerun-inspired structure (K-084): the canvas sits near black,
panels barely above it, and each step up earns real contrast — the deep end of the ramp is
where the depth lives. Values are targets; they MAY be tuned ±3 points of lightness during
implementation, but the ordering, the near-neutrality, and the strict neutrality of
`viewer_surround` are binding.

| Token | Value | Role |
|---|---|---|
| `surface_0` | `#0b0c0e` | The canvas: application background, timeline well, graph paper |
| `surface_1` | `#131517` | Panel bodies — the default fill, and the active dock tab |
| `surface_2` | `#1a1d20` | Faint surfaces: tab bars, bottom bars, panel headers, layer rows |
| `surface_3` | `#212528` | Floating surfaces: menus, popovers, input wells; idle widget fills |
| `surface_4` | `#2b3034` | Hover fills, raised chips, slider tracks, scrollbar thumbs |
| `viewer_surround` | `#121212` | The Viewer's pasteboard — **exactly neutral, R = G = B** |

Rules:

- No surface is pure black; no text is pure white (household rule, kept).
- **The surround of the Viewer image area MUST be `viewer_surround` — strictly neutral grey,
  never tinted.** This includes the pasteboard around the rendered frame, the transparency
  grid's two greys, and letterbox bars. A user MAY darken it (towards `#101010`) or lighten it
  in Viewer settings; every option on that slider is neutral.
- All other surfaces are *near*-neutral: a residual cool cast (blue channel a point or two
  above red) keeps kinship with the fleet, but chroma MUST stay low enough that no panel reads
  as "blue" next to the Viewer.

### 2.2 Text hierarchy

The darker ramp buys headroom: every tier gains contrast over its predecessor on the old
ramp while keeping the same roles.

| Token | Value | Role | Contrast on `surface_1` |
|---|---|---|---|
| `text_primary` | `#eef1f2` | Headings, values being edited, primary copy | ≈ 15.5:1 |
| `text_secondary` | `#c2c8cb` | Panel body copy, property names | ≈ 10.3:1 |
| `text_muted` | `#8b9296` | Kickers, hints, inactive labels, attribution | ≈ 5.5:1 |
| `text_disabled` | `#5e666b` | Disabled controls only | exempt (≥3:1 kept anyway) |

### 2.3 Hairlines

| Token | Value | Role |
|---|---|---|
| `hairline` | `#26292c` (≈ `text_primary` @ 11%) | Default 1px borders between panels, rows, cards; the dock's 1px tile gaps |
| `hairline_strong` | `#3c4145` (≈ `text_primary` @ 22%) | Dividers that must be found, Null layer outlines; doubles as the pressed widget fill |

Hairlines are the *only* default elevation between panels **under the Sharp shape**.
Interactive widgets are **borderless** (K-084, the rerun grammar): idle, hovered and pressed
are *fill* steps (`surface_3` → `surface_4` → `hairline_strong`), never stroke changes.
`shadow_float` (black @ 50%, offset 0/15, blur 50 — rerun's float shadow) is permitted solely
on: modal dialogs, menus/popovers, panels while being drag-undocked, and drag ghosts (clips or
assets in flight) — **under Sharp**. The Round shape (K-092, §7.3) is a deliberate exception:
ordinary docked panes there are floating cards with their own small shadow (`ShapeTokens::
ROUND.card_shadow`, distinct from and smaller than `shadow_float`), so "docked" no longer
implies "no shadow" once Round is picked.

## 3. Saturated colour

### 3.1 Roles

| Token | Value | Role in Lumit |
|---|---|---|
| `accent` (clay) | `#e05a72` | THE accent: primary buttons, selection, playhead, active tab, focused keyframes |
| `accent_hover` (clay-deep) | `#ea7288` | Hover/active shift of the accent (lighter in dark, per household) |
| `success` (olive) | `#5fcfae` | Success, completed exports, cache-bar family root (§6.3) |
| `warning` (kraft) | `#dd9a82` | Warnings, overrun hatching, missing-footage placeholders, "close" feedback |
| `error` (fig) | `#d1729c` | Errors — decode failures, export failures, invalid expressions. Never a harsh red |
| `sky` | `#8ee3ef` | **Reserved** (household person-2 identity). Not used as a semantic role in Lumit; the hex appears only via `viz_1` in charts/curves |
| `disabled` (cloud) | `#6d8794` | Disabled glyphs where `text_disabled` is too quiet |
| `fill_tonal` (oat) | `#24404f` → desaturated to `#2b3438` | Tonal fills behind informational chips |

The household identities — indigo (aizome), crimson (clay), mint — MAY appear in a dark UI as:
the wordmark and about box (crimson accent syllable), chart/curve strokes (`viz_*`), and the
muted layer-type family (§6.1). They MUST NOT appear as large fields of saturated colour in
panel chrome.

### 3.2 The Viewer neutrality zone

**Within 48px of the Viewer image area, the UI MUST be strictly neutral** — `viewer_surround`,
neutral greys, `text_secondary`/`text_muted` type only. Saturated colour (including `accent`)
is banned inside the zone except for:

- transform gizmos, mask paths, and guides *overlaid on the image itself* (these are tools, and
  they are user-toggleable, including a "neutral handles" option that renders them in grey);
- the safe-margin/grid overlays, which default to neutral.

The Viewer toolbar sits outside the zone; its active-state ticks use `accent` like any other
toolbar. Scopes panels are exempt (their traces are content), but their chrome follows the
same neutral rule since they sit beside the Viewer in the Colour workspace.

## 4. Tokens in Rust

### 4.1 The theme struct

egui has no cascade, so the token layer is a plain struct, constructed once per theme and
passed by reference. Shape (illustrative — exact module layout per
[05-ARCHITECTURE.md](05-ARCHITECTURE.md)):

```rust
/// Every colour Lumit ever paints. Constructed by `Theme::dark()` (and later
/// `Theme::light()`). Widget code receives `&Theme` and never constructs colours.
pub struct Theme {
    // surfaces (§2.1)
    pub surface_0: Color32,
    pub surface_1: Color32,
    pub surface_2: Color32,
    pub surface_3: Color32,
    pub surface_4: Color32,
    pub viewer_surround: Color32,   // MUST satisfy r == g == b

    // text (§2.2)
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub text_disabled: Color32,

    // hairlines (§2.3)
    pub hairline: Color32,
    pub hairline_strong: Color32,

    // roles (§3.1)
    pub accent: Color32,
    pub accent_hover: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
    pub disabled: Color32,
    pub fill_tonal: Color32,

    // editor semantics (§6)
    pub layer: LayerColours,        // per layer type
    pub curve: [Color32; 4],        // graph editor curve ramp (viz_1..4)
    pub keyframe: KeyframeColours,
    pub cache: CacheColours,        // vram / ram / disk / uncached
    pub marker: MarkerColours,      // manual / beat
    pub overrun_hatch: Color32,
    pub waveform: WaveformColours,
    pub selection: SelectionColours, // fill / border / focus_ring / drop_target
    pub shadow_float: Shadow,
}
```

Binding rules:

- **All colours in widget code come from `&Theme`.** `Color32::from_rgb`,
  `Color32::from_hex`, and hex literals are permitted only inside the theme module. This is
  enforced in CI (clippy `disallowed-methods` outside `theme/`, plus a grep gate), per
  [14-ENGINEERING-RULES.md](14-ENGINEERING-RULES.md). A hex literal in widget code is a lint
  failure, exactly as it would be in a household component.
- Derived alphas (e.g. `accent` @ 16% selection fill) are computed in the theme constructor and
  stored as their own fields — widget code does not do colour arithmetic either.
- The app icon is the sole hex exception, mirroring the household favicon rule.
- egui's own `Visuals` is populated *from* `Theme` in one place, so stock egui widgets agree
  with custom ones.

### 4.2 Household → Rust mapping

Token *names* are frozen fleet-wide as CSS custom properties; in Rust they map 1:1 to
identifiers. This table is the contract:

| Household token | Rust identifier | Notes |
|---|---|---|
| `bg-paper` | `surface_0` | ground |
| `bg-paper-mid` | `surface_1` | cards/panels |
| `bg-paper-deep` | `surface_2` | tracks/wells |
| — (new) | `surface_3`, `surface_4` | dense tools need two extra steps |
| — (new) | `viewer_surround` | Lumit-only, grading-neutral |
| `text-ink` | `text_primary` | |
| `text-ink-mid` | `text_secondary` | |
| `text-ink-soft` | `text_muted` | |
| `border-line` | `hairline` | |
| `border-line-strong` | `hairline_strong` | |
| `clay` / `clay-deep` | `accent` / `accent_hover` | |
| `olive` | `success` | |
| `kraft` | `warning` | |
| `fig` | `error` | |
| `cloud` | `disabled` | |
| `oat` | `fill_tonal` | desaturated for Lumit |
| `sky` | — | reserved; surfaces only via `curve[0]` |
| `viz-1..4` | `curve[0..3]` | graph editor + scopes chrome accents |
| `shadow-float` | `shadow_float` | floating chrome only |

New Lumit-only tokens (everything in §6) follow the household "Kakeibo rule": they live in
Lumit's theme alone and are promoted to the shared palette only if a second app ever wants
them.

## 5. Iconography

The **Iconoir** set (MIT), embedded as an icon font via the `iconflow` crate (K-085,
reversing this section's earlier hand-drawn-only rule): one consistent, professionally drawn
family, rendered as glyphs so every icon takes the theme colour exactly like text —
`text_secondary` at rest, `text_primary` on hover, `accent` when active — at 16px for panel
toolbars, 20px for the transport. Layer-type glyphs in the Timeline are tinted with the
layer-type family (§6.1). Rules that stand: monochrome only, no filled multi-colour icons,
and **no emoji or bare symbol characters in UI ever** — a glyph is either from the icon set
or deliberately painter-drawn (keyframe diamonds on tracks); never a Unicode character we
hope the user's fonts happen to carry. Every icon name used must resolve in the embedded
pack (CI-tested).

## 6. Editor-specific semantic tokens

These are the token families the household doc has no vocabulary for. All values below are
dark-theme targets, tunable ±10% lightness in implementation; the *relationships* (muted
family, brightness orderings, redundant non-colour encodings) are binding.

### 6.1 Layer-type colours

Every layer type has an identity colour used as: a 3px tab on the left edge of the layer's
bar in the Timeline (owner amendment: the tab rides the bar, not the outline row — the
outline stays glyph-free), a ~12% tonal tint over the bar's fill, and the tint of its type
glyph where one appears. Labels on the bars are always JetBrains Mono 11px. The family MUST
read as *muted siblings* — desaturated, mid-lightness, clearly quieter than `accent` — so a
full Timeline looks organised, not carnival. Selection (accent) must visibly beat every one
of them.

| Layer type | Token | Value |
|---|---|---|
| Footage layer | `layer.footage` | `#56707f` (steel) |
| Sequence layer | `layer.sequence` | `#5a6a8c` (indigo — the flagship type carries the household's own colour) |
| Precomp layer | `layer.precomp` | `#7a5a74` (plum) |
| Solid layer | `layer.solid` | `#5c6165` (neutral) |
| Text layer | `layer.text` | `#8c8468` (parchment) |
| Shape layer | `layer.shape` | `#558a95` (cyan-steel) |
| Null layer | `layer.null` | outline only, `hairline_strong` — nulls render nothing, so their bar is hollow |
| Adjustment layer | `layer.adjustment` | `#8c6b58` (kraft-brown) |
| Audio layer | `layer.audio` | `#46786d` (mint-teal) |
| Camera layer | `layer.camera` | `#806f4a` (dry gold) |
| Light layer | `layer.light` | `#96854f` (pale gold) |

Colour is never the only encoding: each type also has a distinct glyph, and clips inside a
Sequence layer show thumbnails/waveforms.

### 6.2 Graph editor: keyframes and curves

- Curve strokes take the `curve[0..3]` ramp (`#8ee3ef`, `#aef3e7`, `#e8a7b4`, `#d8cba0`) in
  dimension order (x, y, z, w) — the household viz ramp, unchanged. Single-dimension
  properties use `curve[0]`.
- The value graph and the speed graph of a Retime use the same stroke colour — they are views
  of the same data and must look like it.
- Keyframe markers at rest: `text_secondary` fills with `surface_1` outline. Selected:
  `accent` fill. Hovered: `accent` @ 40% halo. Interpolation is encoded by *shape* (diamond
  linear, square hold, circle bezier), never by colour alone.
- Bezier handles: `text_muted` stems, `accent` when grabbed.
- Graph paper: `surface_0` ground, `hairline` minor gridlines, `hairline_strong` at zero/100%
  lines. Axis numbers: JetBrains Mono 11px `text_muted`.

### 6.3 Cache bar

The cache bar is a 4px stripe under the time ruler showing cached frames per tier. All three
tiers are the `success` (olive/mint) family — cached is *good news*, quiet and cool:

| Tier | Token | Value | Fill height |
|---|---|---|---|
| VRAM cache | `cache.vram` | `#5fcfae` | 4px (full) |
| RAM cache | `cache.ram` | `#3f9077` | 3px |
| Disk cache | `cache.disk` | `#2c6353` | 2px |
| Uncached | `cache.uncached` | `surface_3` | 4px, neutral |

Tiers therefore differ in *both* brightness and fill height, so the bar reads without colour
vision. Per the household no-punishment rule, **uncached is neutral, never alarming** — no
amber, no red, no pulsing. An uncached timeline is the normal starting state of every project,
not a failure.

### 6.4 Overrun, markers, waveforms

- **Overrun** (Retime requesting time beyond the media, per K-022): the affected span of the
  clip/layer bar is overlaid with `warning` (kraft) 45° hatching — 1px lines, 4px pitch, 60%
  opacity — plus a mono `HOLD` tag when the span is wide enough. Beneath the hatch sits a
  ~14% `warning` wash so the span reads as one piece at timeline sizes, a 1px `warning` tick
  marks the exact exhaustion point, and hovering the span says what it means ("Source ends
  here — holding the last frame"). Warning, not error: the render is well-defined
  (boundary-frame hold), the editor just needs to see it.
- **Beat markers**: `marker.beat` = `#aef3e7` (mint) 1px ticks in the ruler with a small
  triangular head. Manual markers: `marker.manual` = `text_secondary`; span markers draw a
  hairline-bounded band. Marker labels: mono 11px.
- **Clip waveforms**: `waveform.rest` = `#5d8a96` (muted steel-cyan) filled envelope at 80%
  opacity on `surface_2`; on selected clips the envelope brightens to `text_secondary`.
  Waveforms never render in `accent` — they are content, not state.

### 6.5 Selection, focus, drop targets

- **Selection**: `accent` 1px border + `accent` @ 16% fill on clips, layers, keyframes, assets.
  The playhead is a 1px `accent` line with a full-radius grab handle (≥24px visual, §7.2).
- **Focus ring** (the household `ring-clay` equivalent): every focusable control shows a 1px
  `accent` stroke offset 1px outside its bounds when keyboard-focused. Focus is never
  invisible; egui's `Visuals` focus stroke is set from this token so stock widgets comply.
- **Drop targets** (asset drags, panel docking, clip insertion points): 1.5px dashed `accent`
  border + `accent` @ 10% fill; an insertion caret between clips is a 2px `accent` line. Dock
  previews use the same treatment at panel scale.

## 7. Density and type scale

### 7.1 Scale

Lumit is a pro tool; the household 16px body default gives way to an 11–13px UI scale:

| Size | Face | Use |
|---|---|---|
| 11px | JetBrains Mono, +0.08em, caps | Kickers, layer bar labels, axis numbers, attribution |
| 12px | Inter | Panel body copy, property names, menus, buttons |
| 13px | JetBrains Mono | Property values, timecode fields, frame numbers, speed percentages |
| 14px | Inter Medium | Dialog body emphasis, panel tab labels |
| 16px | Schibsted Grotesk | Dialog titles, workspace names |
| 20px | JetBrains Mono | The transport's main timecode readout |
| 24px+ | Schibsted Grotesk / Source Serif 4 | About box, onboarding, empty states only |

**The mono-for-numbers rule is absolute**: timecode, frame numbers, speed percentages,
parameter values, durations, and counts are ALWAYS JetBrains Mono with tabular figures
(`tnum`), so scrubbing a value never causes horizontal jitter. Editable numeric fields keep
mono while focused.

### 7.2 Hit targets (recorded deviation KD-2)

- Toolbar, transport, dialog, and Viewer-toolbar controls: ≥44px hit extent (household gate).
- Dense-surface controls (Timeline rows, clips, keyframes, curve handles, property lanes,
  cache bar): ≥24px visual extent on the smaller axis, with hit-slop extending the
  interactive region to ≥32px. Keyframes render at 9px but hit-test at 32px with
  nearest-wins disambiguation; adjacent slop regions split at their midpoint.
- Timeline row height: 28px default, 24px minimum at the densest zoom setting; property lanes
  24px. Nothing interactive ever hit-tests below 32px in either axis.

### 7.3 Spacing

Household spacing scale (4/8/12/16/24/32…) with the dense end doing the work: 4px within
control clusters, 8px between clusters, 12px panel padding, 16px dialog padding. **Under the
Sharp shape**, panels butt together separated by a single `hairline`; there are no gaps
between docked panels. **Under Round** (K-092), this is the point of the shape: a real gap
(`ShapeTokens::ROUND.tile_gap`, painted as the canvas colour) opens between every pane and
from the window edge (`window_inset`), and each pane becomes its own rounded card
(`card_radius`/`card_padding`) — see the new Round subsection after §11. Spacing itself (this
section's 4/8/12/16px scale) does not vary by shape; only radius, gap, inset and shadow do.

## 8. Motion

- **The user controls tempo.** Nothing auto-advances, no scroll hijack, no easing applied to
  scroll or zoom. Timeline zoom tracks the wheel/gesture 1:1.
- Micro-motion (hover fills, panel tab underlines, drawer/menu entrances, drop-target
  pulses) uses egui's animation utilities with spring-like ease-out, **≤150ms**, transform
  and opacity only. One signature interaction, per the household budget: the drag ghost —
  clips and assets in flight lag the cursor slightly and settle with a single small
  overshoot on drop.
- **Animation level** (K-092): a three-tier in-app setting — **All** (this section's ≤150ms
  budget in full), **Minimal** (a fast ~50ms snap — still perceptible as motion, not a hard
  cut), **None** (springs don't mount — animation times set to zero, drag ghosts pin to the
  cursor, drop-target pulses become static fills; the OS's own reduced-motion request maps
  onto this tier). Any meaning carried by motion is also carried by colour or text at every
  tier. Backed by one lever over egui's own animation timing, so it reaches what egui's
  internals animate today (collapsing headers, resizable-panel expand/collapse, scrollbar
  fade, dialog fade-in) — it does not retroactively animate Lumit's own menus/dropdowns, which
  have no animation of their own yet regardless of this setting.
- **Playback is not motion.** The Viewer playing at 60fps, scrub feedback, progressive
  preview refinement, and waveform scrolling are *content*, exempt from all of the above,
  and never gated by reduced-motion.

## 9. Accessibility

- **AccessKit** (already in the K-012 stack) is wired from day one: every control exposes
  role, name, and value; panels are landmarks; the Timeline exposes layers/clips/keyframes as
  a navigable tree.
- **Keyboard operability of every control** — every panel reachable by shortcut, every
  control focusable and operable, every drag having a keyboard equivalent (nudge keys move
  clips/keyframes by frame; modifier for 10 frames).
- **Visible focus** everywhere, per §6.5.
- **Contrast floors on the dark ramp** (WCAG 2.1, against the surface the text sits on):
  `text_primary` ≥7:1 (AAA); `text_secondary` ≥7:1; `text_muted` — the floor for the 11px
  mono labels — ≥4.5:1 (AA); disabled states exempt but kept ≥3:1; non-text interactive
  boundaries (selection borders, focus rings, keyframe markers) ≥3:1 against their ground.
  These are CI-checked from the theme struct's actual values.
- **Never colour alone**: cache tiers differ in fill height (§6.3), keyframe interpolation in
  shape (§6.2), layer types in glyph (§6.1), overrun in hatching plus a text tag (§6.4).

## 10. Voice and copy

- British English, sentence case, calm, no exclamation marks, no emoji. UI strings go through
  the i18n table (K-005).
- The app is **"Lumit"** — never abbreviated in UI. Features use glossary names exactly:
  Retime (not time remap), speed (not velocity), clip (not event), layer (not track), export
  (not render), playhead (not CTI). [01-GLOSSARY.md](01-GLOSSARY.md) §9 is binding for copy.
- **Errors are banners, not modal storms.** A failed decode, a lost GPU device, or a failed
  export post a `fig`-tinted banner strip at the top of the relevant panel — factual, one
  sentence, one action: *"Couldn't decode clip 'render_04.mp4' — the file may have moved.
  Relink…"*. Modals are reserved for questions Lumit genuinely cannot proceed without
  answering. Nothing shakes, flashes, or plays a sound.
- Progress copy is factual mono: `Exporting — 41% · 02:12 remaining`. Completion is quiet:
  *"Export finished."* with a reveal-in-folder action.
- **The one rationed running joke** lives in the about box, nowhere else: a single serif line
  under the version number — *"Named for Edo lumit: glass, cut precisely."* That is the
  entire joke budget. No pun-laden tooltips, no wacky empty states, no easter eggs in error
  copy.
- Empty states are soft and factual: *"No compositions yet — import footage or create a comp
  to begin."*
- Attribution/licence lines: mono 11px `text_muted`.

## 11. Light mode (K-092)

Light mode shipped as `Theme::light()` — a token swap, not a redesign, exactly as this
section originally proposed: no widget code changes, only the `Theme` struct's values differ.
One uniform panel colour (white) on a soft neutral canvas, per the owner's explicit call —
**not** per-panel colour-tinting; that idea is wanted and stays on the table as a future
customisable setting, not built here. Surfaces keep the same *roles* as §2.1 (`surface_1` =
panel body, `surface_2` = faint/tab-bar chrome, `surface_3` = floating, `surface_4` =
hover/pressed fill), re-derived at the light end rather than mirror-inverted: since white is
already the brightest possible value, "elevation" reads as a light-grey wash rather than
further brightening past white. `viewer_surround` is **not** mode-mirrored — it stays in the
same fixed neutral mid-grey neighbourhood (`#9c9c9c`–`#b4b4b4`, per this section's original
target) in both Dark and Light, for the same reason §2.1 already decouples it from chrome
brightness: grading judgement needs a surround that doesn't shift under the artist. Text,
hairlines (the same "≈ `text_primary` at N%" rule, re-run against the new near-black anchor),
and roles (accent/success/warning/error, re-picked at reduced lightness rather than naively
inverted — a value as light as the dark-mode accent washes out on white) all follow. The
household `clay`/`olive`/`kraft`/`fig` light values this section originally pointed at aren't
available in this checkout; Lumit's light-mode role colours are its own derivations rather
than a port. `with_accent`'s hover-shift direction now depends on mode: brightening reads as
"more prominent" on a dark surface, so Dark brightens on hover; Light darkens by the same
amount instead. The §9 contrast floors are re-run against the light ramp, not assumed to carry
over from the dark one's numbers.

### 11.1 Named colour schemes (K-097)

Beyond Dark, Dark blue and Light, `Theme` carries four ready-made community palettes as
first-class schemes, each a full token set built the same way as the three above rather than
a re-tint of them: **Gruvbox dark** and **Gruvbox light** (morhetz's warm, retro
cream-and-charcoal pair), and **Catppuccin Mocha** and **Catppuccin Latte** (Catppuccin's
indigo-tinted dark/light pair). Selecting between all seven is `ColorScheme`, which
supersedes the `ThemeMode` × `ThemeVariant` split as the picker's underlying model while
`ThemeMode`/`ThemeVariant` stay in place for the settings that still address them directly.

Every scheme maps its own palette onto the *same* roles this document already defines —
no new tokens, no widget-code changes:

| Role | Gruvbox | Catppuccin |
|---|---|---|
| `surface_0..4` | The palette's own background ramp (`bg0..bg4` dark; the light ramp mirrors §11's "elevation is a darker wash" structure) | `crust`/`base`/`surface0..2` dark; `mantle`/`base`/`crust` mirrored the same way light |
| `text_primary..disabled` | `fg0..fg3` | `text`/`subtext1`/`overlay1`/`overlay0` |
| `accent` | Orange (`#fe8019` dark, `#af3a03` light) | Mauve (`#cba6f7` Mocha, `#8839ef` Latte) |
| `viewer_surround` / scopes | Unchanged: strictly neutral and `ScopeColours::STANDARD`, exactly as every other scheme (§2.1, §11) — a named scheme changes chrome, never the grading-neutral surfaces |

The two dark schemes' `error` role takes each palette's calmer red where the palette offers a
choice (Gruvbox's *neutral* red rather than its "bright" one) — a curation call in the same
no-punishment-red spirit as §3.1, not a claim that every community palette's boldest red is
banned outright. `curve[0..3]` and `layer.*` draw four/six further distinct, muted hues from
each palette rather than reusing `accent`, `success`, `warning` or `error` again, matching how
§6.1/§6.2 keep those families visually separate from the semantic roles.

## 12. The Round shape (K-092)

The Figma-UI3-inspired alternative to this document's default Sharp system: panels float as
rounded, softly-shadowed cards with a real gap between them and from the window edge, rather
than butting edge-to-edge behind a hairline (§7.3, §2.3). Explicitly not glassmorphism or
neumorphism — flat fills, no blur, no inset/outset bevel; the shadow is the only elevation cue
Round adds. Every geometry number Sharp vs Round differs on lives in one place
(`ShapeTokens`, on `Theme`): control/float radii (larger under Round, so a button doesn't look
unfinished inside a rounded card), the docked-pane card's own radius/padding, the inter-pane
gap width, the window-edge inset, and the card's shadow. Colours are unaffected by shape —
Round on Dark and Round on Light both exist, independent of `ThemeMode`. Every panel, the
Viewer included, cards identically; there is no exemption (an earlier option — keeping the
Viewer flush as a deliberate exception — was considered and rejected: consistency won, and
K-074's "no top bit" rule is specifically about the tab bar, not panel margins, so it isn't
affected either way). A stated, permanent limitation: stacked tab-bar containers (a group of
panels sharing tabs) stay square-cornered under Round — `egui_tiles` 0.12.0's `Behavior` trait
has no hook to round a tab bar's own container, and patching the crate for this alone isn't
planned.

## 13. New-panel checklist

The Lumit equivalent of the household §9 checklist. Every new panel or feature MUST satisfy:

1. All colours from `&Theme`; zero hex literals (CI enforces); any genuinely new semantic gets
   a named token in the theme module first.
2. All numbers in JetBrains Mono tabular figures; kickers 11px mono caps; body 12px Inter.
3. Terminology audited against [01-GLOSSARY.md](01-GLOSSARY.md); no banned terms in strings,
   identifiers, or docs.
4. Flat `surface_1` panel, `hairline` separation, radii from {4, 8, 16, full}; shadow only if
   the thing genuinely floats.
5. `accent` is the only saturated state colour; success/warn/error via
   `success`/`warning`/`error`; nothing within 48px of the Viewer image breaks neutrality.
6. Hit targets: ≥44px chrome controls, ≥24px visual + ≥32px slop dense controls (KD-2).
7. Keyboard path for every interaction, AccessKit roles/names, visible focus ring, contrast
   floors met (§9); any colour encoding paired with a non-colour one.
8. Micro-motion ≤150ms, reduced-motion path complete, user tempo never taken.
9. Copy: British English, sentence case, no exclamation marks, banner errors, no new jokes.
10. Works at the dense end: test at minimum row height, minimum panel width, and 125%/150%
    Windows display scaling.

## Brand: the mark and the splash (K-008)

**The mark.** A faceted glass form inherited from the project's Kiriko era — a point-up
hexagon with hairline facet spokes and an inner facet ring, three facets cut in clay that
read as a K. It predates the Lumit name (K-083) and is due for redesign with the retheme;
the letterform in particular no longer matches. Files:
`assets/brand/lumit-mark.svg` (transparent) and `lumit-icon.svg` (dark rounded tile —
the app icon; with the mark's own colours these are the only permitted hex values outside
the theme module: facet hairlines `#3d4042`, outline `#5c6165`, clay `#e05a72`, mist fill
`#22262a→#141618`). The wordmark is the word "Lumit" set in Schibsted Grotesk beside or
beneath the mark; no custom lettering. The mark MUST also be paintable from theme tokens
in code (it is pure strokes, no raster assets) so the splash and about box never ship
image files.

**Status of the current mark: approved placeholder** (owner, 2026-07-13). The destination
for the brand's artwork is more ambitious, in the owner's words: a **broken-glass look**,
styled like something out of Persona 5 — hard-edged silhouettes, aggressive shard-shaped
composition, beautiful graphic stylisation. Direction for the eventual splash art: the
Lumit mark or a silhouette figure seen through/composed of fractured glass shards, flat
high-contrast shapes on the dark ramp with clay as the single cutting colour, mist filling
the negative space. Persona 5 is the energy reference, not a template to copy — no borrowed
assets or traced compositions; Lumit's own geometry (the hexagonal facet grammar above)
supplies the shard language. The boot-log splash below ships with the placeholder mark now;
the art replaces the mark's slot without changing the splash's structure.

**The splash.** A small frameless window, centred on the monitor (~460×300), surface_0,
shown while the application boots:

- Contents: the mark (≈96 px), the wordmark, version in 10 px mono, and the **boot log** —
  a JetBrains Mono list that shows each module and effect as it initialises ("Workspace:
  restored", "GPU: <adapter> via <backend>", "Effects: 24 built-in", "OFX: scanning
  <vendor>…"). This is real plumbing, not theatre: modules and the effect/OFX registries
  append to the boot log as they come up, so slow items (plugin scans, font loads) are
  visible and attributable, AE-splash style.
- A 2 px clay progress hairline along the card's bottom edge; total minimum dwell ≈ 1 s so
  the log is readable, no maximum (the splash stays until boot genuinely finishes).
- Calm rules apply: no animation beyond the log lines appearing and the hairline's
  progress, nothing pulses, reduced-motion shows the same thing (lines appear without
  fades). When boot completes the same window gains decorations and expands into the
  application window.
- Failure honesty: a module that fails to initialise shows its line in kraft with a short
  reason, and the splash proceeds — the app opens degraded rather than hanging on a
  spinner (K-018's spirit at boot time).

## Open questions

- **Promote KD-2 to the decision log?** The hit-target compensation deserves a numbered entry
  in [02-DECISIONS.md](02-DECISIONS.md) (proposed as K-006) so it carries the same weight as
  K-004 rather than living only here.
- **Exact ramp values under real hardware.** §2.1 targets were chosen on paper; they MUST be
  validated on a consumer gaming monitor (the audience's hardware — often wide-gamut,
  aggressively vivid presets) before being frozen. Does `surface_0` at `#141618` hold up on an
  sRGB laptop panel at low brightness?
- **Viewer surround options.** Should the neutral-surround slider expose named stops
  (Dark/Mid/Match panel) or a continuous value? Grading convention favours a couple of fixed,
  documented greys.
- **Layer-type colour user overrides.** AE users expect per-layer label colours. If Lumit
  offers them, the picker SHOULD be a curated muted swatch set derived from §6.1, not a free
  colour wheel — otherwise the Timeline's calm is one preset pack away from destruction.
- **egui text rendering at 11px.** K-012 flags text polish as a known egui risk; if 11px mono
  kickers render poorly on Windows ClearType, the dense scale may need to shift to 12/13/14px.
  Decide after the first Timeline prototype.
- **Wide-gamut / HDR Viewer output.** When the Viewer gains HDR output, the neutrality zone
  rules need restating in display-referred terms; the SDR spec here deliberately ignores it.
