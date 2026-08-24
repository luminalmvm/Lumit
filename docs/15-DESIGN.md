# Lumit design language

**Status: canonical.** Lumit's design language began as the app-specific layer over the shared
household Aizome system; since the 2026-08-23 redesign (K-438) it **stands on its own** — the
household system is lineage, not a constraint. Panel inventory, docking behaviour, and
interaction flows live in [07-UI-SPEC.md](07-UI-SPEC.md); this document owns colour, type,
density, motion, and voice. Terminology follows [01-GLOSSARY.md](01-GLOSSARY.md) exactly.

RFC-2119 keywords (MUST, SHOULD, MAY) are used with their usual force.

---

## 1. Relationship to the household system

Lumit's design descends from the Aizome design language, and the values that made it a good
starting point are deliberately retained. But as of K-438 **Lumit's type and palette decisions
stand on their own**: no household file is an upstream for Lumit's fonts or colours, and a
change to the household system implies nothing here.

### 1.1 Deliberately retained

- **Semantic tokens only.** Every colour in the application comes from a named theme token.
  The token layer is a struct (§4): a hex literal in widget code is a lint failure. The sole
  sanctioned exception is the application icon / favicon set.
- **Dark-first** (K-004's half that stands): dark is the native, first-built, first-polished
  theme, because a neutral dark surround preserves colour judgement against the Viewer.
- **Viewer-surround neutrality** (§2.1, §3.2): the surround of the Viewer image area is
  strictly neutral grey, whatever else the theme does.
- **Type discipline.** The faces changed (§7.1 — Hanken Grotesk and Geist Mono, K-438), but
  the rule did not: **mono for all numbers** — timecode, frame numbers, speed percentages,
  property values, layer indices, labels, attribution — with no exceptions anywhere in the UI.
- **Radii**: 4px (dense elements — clips, keyframe flags, thumbnails), 8px (buttons, inputs,
  chips), 16px (floating cards, dialogs), full (pills, playhead grab handle). No other radii.
- **Hairline elevation.** Panels and cards are flat fills separated by 1px hairline borders.
  Shadows are reserved for things that genuinely float: undocked panels being dragged, modal
  dialogs, drop-down menus, drag ghosts. No glassmorphism, no gradients-as-chrome.
- **Accent discipline.** `accent` (clay) stays the one saturated accent in chrome, now with a
  deliberately short job list (§3.1); the only other stateful colour is `animated` (§3.1),
  whose job list is shorter still.
- **No punishment UI.** Errors and warnings render in `fig` and `kraft`, never a red-alert.
  A dropped frame counter is information, not an alarm. Uncached timeline regions are neutral,
  not threatening (§6.3).
- **Sentence case everywhere.** ALL-CAPS exists only as the kicker pattern (§7.1): Geist Mono,
  9–11px, +0.08–0.12em tracking, muted colour (e.g. `SOURCE TIME`, `WORK AREA`,
  `EXPORT QUEUE`) — and the redesign spends kickers on every container label (§7.1).
- **Voice**: British English, calm, no exclamation marks, no emoji-as-excitement, one rationed
  running joke (§10).
- **Motion philosophy**: the user controls tempo; nothing auto-advances; a complete
  reduced-motion path exists (§8).

### 1.2 Recorded deviations (historical)

These were recorded when the household system was still the upstream; both remain true, and
KD-1's substance is now simply Lumit's own rule.

- **KD-1 · Dark-first (= decision K-004, DECIDED).** The household default was paper-light;
  Lumit inverted this from the start for the Viewer-neutrality reason above. Light mode is a
  shipped token swap (§11), not a second design.
- **KD-2 · Hit-target compensation (= decision K-116, DECIDED).** The household accessibility
  gate demands ≥44px touch targets. In a timeline where twenty layers must be visible at once,
  44px rows are impossible. Lumit's recorded compensation: dense-surface controls (timeline
  rows, keyframes, property lanes, graph editor handles) MUST be ≥24px in visual extent on their
  smaller axis **and** MUST carry ≥32px of interactive hit-slop; toolbar, transport, and dialog
  controls keep the full ≥44px. See §7.2.

### 1.3 What does not apply

Lumit is a native desktop application, so the household web skeleton (React/Vite/Tailwind,
`sync-theme.sh`, FastAPI monorepo, mobile bottom tab bar, PWA icon rules) never applied. Since
K-438 the household `theme.css` is no longer an upstream either: Lumit's palette is re-derived
from nothing, and a household repaint implies no change wave here. Person-identity colours have
no meaning in a single-user pro tool; `sky` survives only as a curve colour (`curve[0]`), and
nothing becomes a second accent — the second stateful colour that exists, `animated`, is not an
accent and has a closed job list (§3.1).

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
| `surface_0` | `#0b0c0e` | The canvas: application background, timeline well, graph paper — and **input wells**, which are inset, never raised |
| `surface_1` | `#131517` | Panel bodies — the default fill, and the active dock tab |
| `surface_2` | `#1a1d20` | Faint surfaces: tab bars, bottom bars, panel headers, layer rows |
| `surface_3` | `#212528` | Hover and floating only: menus, popovers, hovered rows |
| `surface_4` | `#2b3034` | Hover fills on `surface_3` ground, raised chips, scrollbar thumbs |
| `viewer_surround` | `#121212` | The Viewer's pasteboard — **exactly neutral, R = G = B** |

Rules:

- **Three greys at rest (K-439).** A panel in its resting state shows at most three surface
  values: `surface_0` (canvas), `surface_1` (body), `surface_2` (header strip). `surface_3`
  and `surface_4` exist for hover states and floating chrome only — a panel nobody is
  pointing at never paints them. **Input wells are inset on `surface_0`** — darker than the
  panel they sit in, never a raised fill — so an editable value reads as a recess the number
  sits in, and the resting panel stays three values however many fields it carries.
- No surface is pure black; no text is pure white (household rule, kept).
- **The surround of the Viewer image area MUST be `viewer_surround` — strictly neutral grey,
  never tinted.** This includes the pasteboard around the rendered frame, the transparency
  grid's two greys, and letterbox bars. A user MAY darken it (towards `#101010`) or lighten it
  in Viewer settings; every option on that slider is neutral. **One opt-in leaves neutral
  (K-203):** Settings → Appearance → Viewer offers a switch that paints the surround in the
  theme's own panel surface, **off by default**, on the owner's request and for the same
  reason the scopes toggle exists — off-spec, deliberate, and a matter of taste. The default
  and every shipped scheme stay neutral, and the surround remains the one colour the theme
  editor does not offer.
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
are *fill* steps, never stroke changes. Under the three-greys rule (§2.1, K-439) the idle
step is quiet — a widget at rest sits on its panel's own surface (or `surface_2` where a
strip already earns it), and `surface_3` → `hairline_strong` are the hover → pressed steps;
input wells are the §2.1 inset, not a raised fill. **Buttons are the one exception** (K-450):
the secondary button rests as a `hairline_strong` *outline* over the panel's own surface,
because the approved dialog pattern (§12A.4) needs a button to be findable beside the single
filled action without adding a fourth grey — the exception is buttons only, and it is still a
resting outline, not a stroke that changes with hover or press.
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
| `accent` (clay) | `#e05a72` | THE accent, with a deliberately short job list (K-439): **the single filled button per surface, the playhead, and the active tab tick**. The selection tokens (§6.5) derive from it. Everything else in chrome is grey |
| `animated` | `#d8a24a` (placeholder, tunable) | "This is animated or in hand": **keyframe diamonds, stopwatch-on, selected keyframes, selected gizmo handles, the focused value field, and the work-area band** — a desaturated warm amber, quieter than `accent`. That list is closed: **if a third kind of use appears, it is wrong** |
| `accent_hover` (clay-deep) | `#ea7288` | Hover/active shift of the accent (lighter in dark) |
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

**Editable values (K-439).** An editable value is **mono text in an inset well** (§2.1) at
rest — the well is what says "editable", not a colour. The text turns `animated` when the
property is keyframed, and `accent` while the value is actually being dragged. This resolves
the old open question on editable-value colour: colour alone (the After Effects way) would
break the Viewer neutrality zone, and the well does the job on every surface. The focused
value field draws its focus in `animated` rather than the general focus ring — it is the one
focus that means "you are about to change a value".

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

## 4. The token layer

### 4.1 The theme struct

There is no cascade, so the token layer is a plain struct, constructed once per theme and
passed by reference. Since the Flutter port (K-174/K-182) it lives Dart-side as `LumitTheme`
(`flutter_ui/lib/theme/theme.dart`); the sketch below keeps the original Rust-flavoured
shape as the token inventory (illustrative — `Color32` reads as `Color`):

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
    pub animated: Color32,          // the closed K-439 job list (§3.1), nothing else
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

**Custom themes (K-202).** Every colour listed here is editable by the user in Settings →
Appearance → Customise…, and a saved custom theme is *a name, a light-or-dark base, and the
colours over it* — so a theme keeps working when a token is added, taking the new one from
its base. `flutter_ui/lib/theme/theme_tokens.dart` is the single declaration of what is
editable (a test counts it against the struct); `viewer_surround` is deliberately absent for
the §2.1 reason.

**Sharing a theme (K-298).** A theme is also a file: `.lumtheme`, an indented JSON document
carrying a format marker, a version, and the same name/base/colours the workspace file
stores (`flutter_ui/lib/theme/theme_file.dart`). Settings → Appearance offers **Duplicate,
Rename…, Delete, Import… and Export…** beside **Customise…**, the editor offers **Save a
copy…**, and the picker carries an eight-swatch preview of the selection. A theme read from
a file is applied over its base like any other, so one written by a newer Lumit still opens
with the colours this build knows; a name already taken is numbered rather than overwritten. Two tokens were added with it, both defaulting from the mode rather than
being restated per scheme: `timeline_out_of_range` (the Timeline's ground outside the work
area) and `selection_fill` (under a selected row, half-strength under a highlighted one —
its own colour because a selection has to out-contrast whichever ground it lands on, which
on a light scheme means going *darker* while the surfaces go lighter).

**v1 status of this struct.** The struct above is the target shape. The shipped `LumitTheme`
(`flutter_ui/lib/theme/theme.dart`) carries the structural roles — the surfaces, text, hairlines,
`accent`/`accent_hover`, `success`/`warning`/`error`, the `curve[4]` ramp, `layer`
(`LayerColours`, §6.1) — plus two the code has split out that this listing does not yet name:
`scope` (`ScopeColours`, the four scope-chrome accents), `cache_disk` (the disk tier of the
cache bar, §6.3), `marker` (comp markers on the time ruler, §6.4 — the first of the
`marker` grouping to be split out, K-254; the beat variant still waits) and `waveform`
(`WaveformColours`: `rest` plus the three multiwave bands, K-280). Not yet split into their own tokens, and derived ad-hoc from existing roles in
v1: `disabled` and `fill_tonal` (the `cloud`/`oat` mappings below are reserved, not present);
the `keyframe`, `overrun_hatch` and `selection` groupings (widgets reach
for `text_secondary`, `accent`, `warning`, etc. directly); and `shadow_float`. Splitting each
into a named token — so no widget derives a semantic colour itself — is the standing direction,
done as each area is next touched; the no-hex rule already holds regardless.

Binding rules:

- **All colours in widget code come from the theme.** Hex literals and colour constructors
  are permitted only inside `flutter_ui/lib/theme/`. CI's `no-hex-outside-theme` job greps
  every Rust crate (no colour may live engine-side at all); on the Dart side the rule holds
  by convention and review today — a Dart-side lint gate is owed ([TODO.md](TODO.md)). A hex
  literal in widget code is a defect, exactly as it would be in a household component.
- Derived alphas (e.g. `accent` @ 16% selection fill) are computed in the theme constructor and
  stored as their own fields — widget code does not do colour arithmetic either.
- The app icon is the sole hex exception, mirroring the household favicon rule.
- The toolkit's own default styling is populated *from* `Theme` in one place, so stock
  widgets agree with custom ones.

### 4.2 Household → Rust mapping (lineage, no longer binding)

This table recorded where each token name came from while the household system was the
upstream. Since K-438 it is kept as history — the names still hold, but no household file
constrains their values, and new tokens (`animated`, everything in §6) need no household
counterpart:

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

New Lumit-only tokens (everything in §6, and `animated`) live in Lumit's theme alone; there
is no shared palette to promote them to any more.

## 5. Iconography

**Lumit draws its own icon set** (K-440, superseding K-085's Iconoir pick). The set is drawn
and adopted application-wide: `flutter_ui/lib/icons/icons.dart` resolves each name the panels
ask for to its glyph. Iconoir stays only where the set still owes a drawing — the deep tools
(puppet, roto, vertex, camera navigation), the star and solid marks, the fx switch, the label
tag, the snap magnet, tone map and the node panel's mark — listed in `docs/TODO.md`; a name
crosses over the day its glyph exists, and no panel changes. The set's grammar is fixed:

- **16px grid, 1.5px stroke, round caps, one weight.** Every glyph is drawn on the same
  16-unit grid with the same stroke; there are no filled and outlined variants of one idea,
  and no second weight.
- **Monochrome via `currentColor`**: a glyph takes the text colour of wherever it sits —
  `text_secondary` at rest, `text_primary` on hover, `accent` when active — exactly like a
  word would. **The one glyph with real colour of its own is the Viewer's Channels
  indicator**, whose circles fill per viewed channel (all three plus a white centre for RGB;
  a single circle for R, G or B). Nothing else in the set carries colour.
- **One icon per chrome word.** The set covers the tools, layer switches, transport,
  timeline and graph controls, keyframes, effects, the Viewer bar, the Project panel, the
  graph panel and the application chrome — so the Icons setting (§5.1) has a glyph for
  every word it replaces.
- **A bypassed effect draws as a dashed outline**, not a dimmed row — state shown by the
  glyph's own border, so the label stays readable.
- Layer-type glyphs in the Timeline are tinted with the layer-type family (§6.1).
- Pixel honesty carries over from K-209: a 1.5px stroke is offset so it lands on pixel
  centres rather than straddling boundaries at 100% scaling. **The 16 is the drawing grid,
  not the display size** (K-456, superseding K-209's fixed 16 where a mockup renders
  smaller): every glyph is still drawn on the 16 grid, and each panel renders it at the
  size its own approved mockup computed — in the Project panel, 13 in the tree's rows and
  13 on the bottom bar. The slight softening of the stroke at a non-native size is the
  mockups' own look and is accepted as such.
- Rules that stand from the beginning: monochrome only (bar the Channels indicator above),
  no filled multi-colour icons, and **no emoji or bare symbol characters in UI ever** — a
  glyph is either from the icon set or deliberately painter-drawn (keyframe diamonds on
  lanes); never a Unicode character we hope the user's fonts happen to carry. Every icon
  name used must resolve in the embedded set (CI-tested).

### 5.1 Chrome labels: Words / Icons / Icons everywhere (K-440)

A three-way setting decides whether chrome speaks in words or glyphs:

- **Words** (the default): buttons, tabs and toggles carry their text labels.
- **Icons**: buttons, tabs and toggles become glyphs; **panel titles stay text**.
- **Icons everywhere**: panel titles become glyphs too.

**Tooltips always carry the word**, whichever mode is set — the glyph never strands its
meaning. And a tooltip is **one or two words, never more** (§10); content the user typed
(layer names, values, file names) is never iconified in any mode.

## 6. Editor-specific semantic tokens

These are the token families the household doc has no vocabulary for. All values below are
dark-theme targets, tunable ±10% lightness in implementation; the *relationships* (muted
family, brightness orderings, redundant non-colour encodings) are binding.

### 6.1 Layer-type colours

Every layer type has an identity colour used as: a 3px tab on the left edge of the layer's
bar in the Timeline (owner amendment: the tab rides the bar, not the outline row — the
outline stays glyph-free), a ~12% tonal tint over the bar's fill, and the tint of its type
glyph where one appears. Labels on the bars are always Geist Mono 11px. The family MUST
read as *muted siblings* — desaturated, mid-lightness, clearly quieter than `accent` — so a
full Timeline looks organised, not carnival. Selection (accent) must visibly beat every one
of them.

v1 ships an identity colour token for six layer kinds. The `LayerColours` class
(`flutter_ui/lib/theme/theme.dart`) carries exactly these six; the panels map each layer
kind to its token and glyph.

| Layer type | Token | Value | v1 |
|---|---|---|---|
| Footage layer | `layer.footage` | `#56707f` (steel) | ✓ |
| Sequence layer | `layer.sequence` | `#5a6a8c` (indigo — the flagship type carries the household's own colour) | ✓ |
| Precomp layer | `layer.precomp` | `#7a5a74` (plum) | ✓ |
| Solid layer | `layer.solid` | `#5c6165` (neutral) | ✓ |
| Text layer | `layer.text` | `#8c8468` (parchment) | ✓ |
| Camera layer | `layer.camera` | `#806f4a` (dry gold) | ✓ |

Three further kinds exist today without a token of their own: the **Adjustment layer**
borrows `layer.solid` (neutral) and the Solid glyph, since it renders no source of its own
(when it earns a distinct identity the natural value is `#8c6b58`, kraft-brown); the
**Shape layer** (K-237) and **Null layer** (K-206) likewise borrow — their reserved values
below stand for when each earns its token.

Reserved values (Shape and Null are modelled but untokened; Audio and Light have no
`LayerKind` variant yet):

| Layer type | Token | Value |
|---|---|---|
| Shape layer | `layer.shape` | `#558a95` (cyan-steel) |
| Null layer | `layer.null` | outline only, `hairline_strong` — nulls render nothing, so their bar is hollow |
| Audio layer | `layer.audio` | `#46786d` (mint-teal) |
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
  `animated` fill (K-439). Hovered: `animated` @ 40% halo. Interpolation is encoded by
  *shape* (diamond linear, square hold, circle bezier), never by colour alone.
- Bezier handles: `text_muted` stems, `animated` when selected, `accent` while grabbed.
- Graph paper: `surface_0` ground, `hairline` minor gridlines, `hairline_strong` at zero/100%
  lines. Axis numbers: Geist Mono 11px `text_muted`.

### 6.3 Cache bar

The cache bar is a thin stripe along the floor of the time ruler — on the work-area
band's own row, with the band painting behind it — showing which frames are cached, per
tier. Cached is *good news* — quiet and cool, never alarming.

All three of Nebula's tiers ship (K-214, docs/06 §5.6). Every run draws as a 3px band at
the ruler's floor:

| State | Token | Value | Meaning |
|---|---|---|---|
| Held, this resolution | `success` | `#5fcfae` (mint) | on the card or in memory: plays right now |
| Held, coarser | `success` at 40% | dimmed mint | held, but would be re-rendered at this size |
| On disk, this resolution | `cache_disk` | `#5f93b8` (steel blue) | parked on disk, promotable |
| On disk, coarser | `cache_disk` at 40% | dimmed steel blue | parked, and coarser than shown |
| Uncached | — | (no bar) | neutral — the normal starting state |

Mint reads as hot (playable now); the disk tier's cooler blue marks frames that are one
promotion away (docs/06 §5.6). The card's tier and memory's share one colour deliberately:
they answer the same question — *does this frame play now?* — and a frame in memory is one
upload from the screen. Which of the two holds it is the status line's cache meter's business,
where each tier has its own bar. The fuller design — tiers differing in *both* brightness and
fill height, so the bar reads without colour vision — lands with a dedicated tonal ramp; until
then the mint/blue hue split plus the dimming carries the distinction.
Per the no-punishment rule, **uncached is neutral, never alarming** — no amber, no
red, no pulsing. An uncached timeline is the normal starting state of every project, not a
failure.

Under the redesign (K-441) the bar also carries the **resolution tier**, so a glance says
not just "cached" but "cached at what size". **Shipped:** each strip byte carries two
nibbles — the storage state above, and the preview *divisor* the found picture was actually
made at relative to the resolution being shown. The storage state picks the **family**
(mint in memory or on the card, steel blue on disk) and the divisor picks the **step**
within it: full at full strength, half at 70%, and the faintest step at 40%.

**The faintest step means a third _or_ a quarter**, and that is deliberate. The realtime
controller renders a third as well as the half and quarter this table names, so there are
four tiers and three steps; folding the third in with the coarsest under-promises, where
giving it the half's step would tell someone a third is finer than it is. A divisor a
later engine invents lands at the same faintest step, for the same reason — "held, and
coarser than that" is the safe thing to say about a number this build does not recognise.
Nothing here changes the two truths behind the reading: a frame held at a scale no adaptive
tier renders at reads as **nothing held**, and on a sampled composition a frame the
refinement sweep has not reached wears its sample's tier.

The fuller storage split (which of memory and the card holds a frame) stays the status
line's cache meter's business, where each tier already has its own bar.

### 6.4 Overrun, markers, waveforms

- **Overrun** (Retime requesting time beyond the media, per K-022): the affected span of the
  clip/layer bar is overlaid with `warning` (kraft) 45° hatching — 1px lines, 4px pitch, 60%
  opacity — plus a mono `HOLD` tag when the span is wide enough. Beneath the hatch sits a
  ~14% `warning` wash so the span reads as one piece at timeline sizes, a 1px `warning` tick
  marks the exact exhaustion point, and hovering the span says what it means ("Source ends
  here — holding the last frame"). Warning, not error: the render is well-defined
  (boundary-frame hold), the editor just needs to see it.
- **Comp markers (shipped, K-254)**: a `marker` token of its own — a plain grey, `#c4c4c4`
  on a dark scheme and `#565656` on a light one, editable in the theme editor like any other
  role. Grey rather than a role colour on purpose: a marker says *here*, not *good* or
  *careful*, and the work area already has its own colour (`animated`, §12A.1). Under the
  redesign (K-441) the flag is an **upward triangle**, 8 wide and 6 tall, standing on the
  cache bar at the floor of the ruler's lower row and centred on the frame it marks, so its
  point aims at the clock above. What it says rides in a **backdrop pill** in `surface_4`,
  12px tall, starting at the triangle's point and running right — the triangle's left half
  stands clear of it, its right half is inside — with the label in **mono at 8px** in
  `text_primary`, rather than as loose text over the ticks. **One marker per frame** — a
  second dropped on an occupied frame replaces the first, since two flags on one moment are
  two things to click and one place.
- **Beat markers**: `marker.beat` = `#aef3e7` (mint) 1px ticks in the ruler with a small
  triangular head — still to come, and it needs a token of its own beside `marker`. Span
  markers draw a hairline-bounded band.
- **Clip waveforms (shipped, K-280)**: `waveform.rest` = `#5d8a96` (muted steel-cyan) filled
  envelope at 80% opacity on `surface_2`, with the RMS core drawn solid inside it; on selected
  clips the envelope brightens to `text_secondary` (still to come). Waveforms never render in
  `accent` — they are content, not state, and the lane that did borrow `accent` was corrected
  when this grouping became real tokens. The **multiwave** stack (K-280, K-284, K-382) adds three band
  colours beside `rest`, drawn **over one another in one lane** and so ranked by *brightness*
  rather than by hue — one silhouette with its inside showing. Painted back to front from the
  pale end of the ramp (treble, middle, bass), each band lifted 1–4 px above the one behind
  it (8% of the row, clamped) so none can hide inside another.
  `waveform.low` `#3c5c66`, `waveform.mid` `#6d9aa6`, `waveform.high` `#d4f0f6` on a dark
  scheme; on a light one the ramp runs the other way (`#9dbac2` / `#598794` / `#14333c`),
  because *darker* is what stands out on white. Band strokes are opaque — three softened
  envelopes over one another blend into a wash and lose the ranking — and only the single wave
  keeps the 80% envelope with the solid RMS core over it. All four default from the mode
  rather than being restated per scheme, and all four are editable like any other token.

### 6.5 Selection, focus, drop targets

- **Selection**: `accent` 1px border + `accent` @ 16% fill on clips, layers, assets;
  keyframe selection is `animated` (§6.2).
  The playhead is a 1px `accent` line with an 11×8px `accent` **head** at the top of the
  ruler — a downward triangle, with the line carried up into it as a 1px notch in
  `surface_0` (K-207). A bare hairline is findable only by hunting along the ruler, and at
  a glance it reads as a row seam; the head is the editor idiom for "you are here". The
  grab target stays ≥24px wide (§7.2) whatever the head draws.
- **Focus ring** (the household `ring-clay` equivalent): every focusable control shows a 1px
  `accent` stroke offset 1px outside its bounds when keyboard-focused — except the focused
  value field, which draws its focus in `animated` (§3.1). Focus is never
  invisible; the toolkit's focus stroke is set from this token so stock widgets comply.
- **Drop targets** (asset drags, panel docking, clip insertion points): 1.5px dashed `accent`
  border + `accent` @ 10% fill; an insertion caret between clips is a 2px `accent` line. Dock
  previews use the same treatment at panel scale.

## 7. Density and type scale

### 7.1 Scale

**Two faces, both SIL OFL, both bundled (K-438): Hanken Grotesk** for UI text and
**Geist Mono** for every number, timecode and container label. There is no third face in
chrome — no display face, no serif accent. This supersedes the household stack (Schibsted
Grotesk / Source Serif 4 / Inter / JetBrains Mono) outright.

Lumit is a pro tool; the scale is tight (K-317's 10–13px band stands) and **nothing in
chrome sits above 13px except dialog body emphasis**:

| Size | Face | Use |
|---|---|---|
| 9–11px (**9 shipped**, +0.12em, regular) | Geist Mono, caps, `text_muted` | **Kickers — every container label**: panel titles, properties section headers, column headers, tab labels, dialog titles, attribution. The shipped value is the approved mockups' own `.kick` (K-451) |
| 9px | Geist Mono, caps, +0.12em, `surface_0` on the `accent` fill | **The filled primary action's label** — the one filled button a surface is allowed (§3.1, §12A.4). A kicker in every respect but its colour, which the fill under it decides |
| 10px | Hanken Grotesk | Secondary notes and hints (`small`); field captions — never for anything the user has to act on; **layer bar labels** and the labels of the in-row pickers beside them (matte, blend, parent), both the approved mockups' own size (K-451) |
| 11px | Hanken Grotesk | Panel body copy, property names, menus, buttons |
| 11px | Geist Mono | **Property values in wells, timecode fields, frame numbers, speed percentages** — the approved mockups compute every `.well` and every timecode at 11 (K-454). It had been recorded here as 13, a size the mockups use nowhere |
| 10px | Geist Mono | Units beside a value (`px`, `%`, `°`), and the readouts in an outline row — a layer's number, a property's value, the ms column |
| 9px | Geist Mono, letter-spacing normal | Ruler labels, per-effect cost readouts, the status bar. Plain mono, **not** a kicker: same size, no tracking, no capitals |
| 13–14px | Hanken Grotesk Medium | Dialog body emphasis — the one thing in chrome above 13px |
| 24px+ | Hanken Grotesk | About box, onboarding, empty states only — outside chrome |

**Containers are kickers; content is sentence case.** Everything the *application* names —
panel titles, properties section headers, column headers, tab labels, dialog titles — is a
kicker: 9–11px Geist Mono caps, +0.08–0.12em tracking, muted. Everything the *user* names or
edits — layer names, values, file names — is sentence-case Hanken Grotesk or mono per the
rule below. **Units are plain mono, never caps** — `px`, `%`, `s` sit beside their value in
Geist Mono lowercase, not in the kicker style.

**The mono-for-numbers rule is absolute**: timecode, frame numbers, speed percentages,
parameter values, durations, and counts are ALWAYS Geist Mono with tabular figures
(`tnum`), so scrubbing a value never causes horizontal jitter. Editable numeric fields keep
mono while focused.

### 7.2 Hit targets (recorded deviation KD-2, = K-116)

- Toolbar, transport, dialog, and Viewer-toolbar controls: ≥44px hit extent (household gate).
  **The tool strip keeps this across and not down** (K-230): its buttons are 44px wide, which
  is the axis the row is read and aimed along, in a strip 30px tall. The strip runs the full
  width of the window, so a 44px band of mostly empty chrome is height taken from the panels
  underneath for nothing; the 16px icon (§5) still has room around it.
- Dense-surface controls (Timeline rows, clips, keyframes, curve handles, property lanes,
  cache bar): ≥24px visual extent on the smaller axis, with hit-slop extending the
  interactive region to ≥32px. Keyframes render at 9px but hit-test at 32px with
  nearest-wins disambiguation; adjacent slop regions split at their midpoint.
- Row and chrome heights are **§12A.6's table** (K-451), which supersedes the 28px/24px rows
  written here before the redesign: an outline or lane row is 23 and a secondary row 19
  under Regular, 22 and 18 under Compact (K-454 — read the height off the table's two
  columns, never off this sentence). The
  floors above still govern what sits *inside* those rows — bars, keyframes, curve handles
  and the cache bar keep their ≥24px visual and ≥32px hit extent along the axis they are
  aimed at. A chrome row shorter than the floor is the mockup's height, not a licence to
  shrink the targets in it: where the two cannot both hold, the visual keeps the table's
  height and the note beside the code says which floor gave way.

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
  pulses) uses spring-like ease-out, **≤150ms**, transform
  and opacity only. One signature interaction, per the household budget: the drag ghost —
  clips and assets in flight lag the cursor slightly and settle with a single small
  overshoot on drop.
- **Animation level** (K-092): a three-tier in-app setting — **All** (this section's ≤150ms
  budget in full), **Minimal** (a fast ~50ms snap — still perceptible as motion, not a hard
  cut), **None** (springs don't mount — animation times set to zero, drag ghosts pin to the
  cursor, drop-target pulses become static fills; the OS's own reduced-motion request maps
  onto this tier). Any meaning carried by motion is also carried by colour or text at every
  tier. Backed by one lever over the toolkit's own animation timing, so it reaches what the
  toolkit animates internally — it does not retroactively animate Lumit's own menus/dropdowns, which
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
- **Visible focus** everywhere, per §6.5. Every house control — button, checkbox, radio,
  value box — is focusable, draws the accent focus ring while focused, and answers `Enter`
  (and `Space`, where pressing is what it does). Tab visits them in **reading order**: left
  to right, then top to bottom, by where they sit on screen rather than by the order the
  layout code composed them. A modal window is its own focus scope, so Tab cycles inside it
  (K-319, docs/07 §4.2).
- **A pointer travelling to a submenu is not hovering what it passes over** (K-318). Menus
  hold an open flyout while the pointer is inside the triangle from where it left the owning
  row to that flyout's near edge — the "safe triangle". A pointer that *stops* on another row
  still switches, after a 300ms grace; one that plainly moves elsewhere switches at once. No
  animation is involved and nothing is delayed that the user did not aim at.
- **Contrast floors on the dark ramp** (WCAG 2.1, against the surface the text sits on):
  `text_primary` ≥7:1 (AAA); `text_secondary` ≥7:1; `text_muted` — the floor for the 11px
  mono labels — ≥4.5:1 (AA); disabled states exempt but kept ≥3:1; non-text interactive
  boundaries (selection borders, focus rings, keyframe markers) ≥3:1 against their ground.
  These are CI-checked from the theme struct's actual values.
- **Never colour alone**: cache tiers differ in fill height (§6.3), keyframe interpolation in
  shape (§6.2), layer types in glyph (§6.1), overrun in hatching plus a text tag (§6.4).

## 10. Voice and copy

- British English, sentence case, calm, no exclamation marks, no emoji. UI strings go through
  the i18n table (K-005) — `flutter_ui/lib/l10n/app_en.arb`, translated on Crowdin (K-303).
  British English is the source and stays the source; there is no en-US.
- **A tooltip is a name, not a lesson**: **one or two words, never more** (K-440,
  tightening [07-UI-SPEC.md](07-UI-SPEC.md) §13.2's "under five"). Under the Icons chrome
  setting (§5.1) the tooltip is where the word lives, which is exactly why it must stay a
  word. Explanation belongs in the settings row's own sentence, in an empty state, or
  nowhere.
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
| `viewer_surround` / scopes | Unchanged: strictly neutral and `ScopeColours::STANDARD`, exactly as every other scheme (§2.1, §11) — a named scheme changes chrome, never the grading-neutral surfaces. **A user may opt scopes into the theme's colours** (Settings → Appearance, off by default, K-202) and, separately, the Viewer surround into the theme's panel surface (off by default, K-203); both default to neutral, and the surround remains the one colour the theme editor does not offer |

The two dark schemes' `error` role takes each palette's calmer red where the palette offers a
choice (Gruvbox's *neutral* red rather than its "bright" one) — a curation call in the same
no-punishment-red spirit as §3.1, not a claim that every community palette's boldest red is
banned outright. `curve[0..3]` and `layer.*` draw four/six further distinct, muted hues from
each palette rather than reusing `accent`, `success`, `warning` or `error` again, matching how
§6.1/§6.2 keep those families visually separate from the semantic roles.

## 12. The Round shape (K-092)

**Sharp is the redesign's reference shape** (K-441): the 2026-08-23 redesign is designed,
mocked up and landed under Sharp, and Round is revisited against the finished Sharp shell
afterwards rather than co-designed with it.

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
panels sharing tabs) stay square-cornered under Round — the docking container offers no hook
to round a tab bar's own container, and patching it for this alone isn't
planned.

### 12.1 Round v2 — the bubble commit (K-394)

Round v1 rounded corners; v2 commits to the shape, on cues from a reference the owner
picked (OUTLOUD's Lyrica editor) — cues, not a copy. All of it is `ShapeTokens` reads
and shape-conditional widget geometry; colours, strings and Sharp are untouched.

- **Stadium controls.** Under Round, a button, chip, tab, dropdown or timecode chip is a
  full capsule: radius = half its own height (`control_radius` becomes the stadium
  sentinel under Round rather than a number that approximates one). The transport's
  buttons additionally sit together inside one pill container on the Viewer bar. Under
  Round that whole bar is a tile of its own **below** the picture — parted from it by the
  tile gap, with the canvas showing through, and never laid over the frame — while staying
  inside the Viewer panel, so docking or moving the panel carries it; Sharp keeps the bar
  welded to the panel's bottom edge.
- **Bigger cards.** `card_radius` 14 → 18 and `float_radius` 12 → 16, so a menu is not
  squarer than the card that spawned it.
- **Filled-pill actives.** The active tab / mode chip / segmented option fills with
  `accent` and its label flips to `surface0` — the far end of the ramp from the text,
  which is the dark label on a dark scheme and the light one on a light scheme without
  either being spelled out twice (Round on Light exists, §12). Inactive stays ghost.
  This is the reference's loudest cue: state reads at a glance from fill, not from text
  tint.
- **Dot slider thumbs.** The thin track with a round thumb on it, which `HouseSlider`
  already drew — under both shapes, and still does. Recorded as a cue the reference and
  Lumit happened to agree on, not as a change: nothing shipped for it, and making it
  Round-only would have taken the dot away from Sharp for no reason.
- **Capsule bars.** Timeline layer bars and Sequence clips draw with stadium ends under
  Round (at the bar's own height). Keyframe diamonds, the playhead and the rulers do
  not change — they are marks, not surfaces.
- **The header dot.** Each panel header carries a small accent dot under Round — the
  reference's quiet live-mark. Decorative, never a status light; it does not blink,
  fill or change colour.
- **Under Sharp a panel's tab is bare text.** The header strip's tabs are kickers on the
  strip's own grey — no fill, no outline, no tick — and the fronted one is marked by its
  word brightening to `text_primary` while the rest stay `text_muted`, exactly as the
  mockups compute them. It had worn an accent outline, which spends the accent on a
  resting state and makes the one lit tab read as a button to press. This is the same
  ruling the composition tabs already carry (§12A.1): §3.1's "active tab tick" means the
  workspace tabs, and nothing else. Round is untouched — its filled accent pill (K-394)
  says the same thing with the fill.

Rejected cues, with the reasons (K-394): uppercase panel titles *as display text* (since
K-438 panel titles are mono-caps kickers, §7.1 — a typographic pattern, not the reference's
shouted sans headers) and the reference's light shell around dark cards (the dark-first
surround rules of §2 and the neutral Viewer pasteboard, K-203, are binding — an inverted
shell is a different theme, not a shape).

## 12A. The redesigned resting state: timeline, graph, properties, dialogs

The 2026-08-23 redesign (K-441–K-444) fixed the resting layout of the main surfaces. A set of
approved mockups governs the exact panel layouts; the rules below are the ones binding enough
to write down. The mockup sources land in the repository under `docs/redesign/mockups/` with
the implementation programme's first phase; until then the approved set is held by the owner.
**Sharp is the redesign's reference shape**: every rule here is designed and
judged under Sharp first, and Round (§12) is revisited once the Sharp redesign has landed.

### 12A.1 Timeline (K-441)

- **Modes are Layers, Keys and Graph** (K-455, K-458). Keys is the dope sheet built to the
  approved Keys drawing — the whole of it, editing tools included (K-458): the
  block-selection box with stretch handles and its `n keys · n f` badge, the Ease
  popover, and the bottom bar's Interpolation / Reverse / Copy / Paste-at-playhead
  strip. Layers mode carries an **Animated
  filter** (`U`): on, the outline shows only keyframed properties across all layers; All
  restores the full twirl-down lists. Block selection, end-handle stretch and the Ease
  popover are Layers behaviours, not a mode of their own.
- **Composition tabs run the full width of the panel header**, between the panel's own
  `TIMELINE` kicker at the far left and **one filled `EXPORT`** at the far right — the
  single filled action a surface is allowed (§3.1), running the File menu's own Export
  command rather than a second route to the same dialog. The row above the outline
  puts the timecode and frame count at its far left and the Layers / Keys / Graph mode tabs at its
  far right. **The frame count says how far through**: the frame in hand, then the comp's
  whole length after it — `f48 / 250`, the total quieter again than the number it follows.
- **The column headers are kicker words, not icons**: Switches · # · Layer · Matte ·
  Blend · Parent · ms. A column header names a container, and §7.1 sets every container
  label as a kicker; the switch cells keep their marks, because those are the controls.
  The bottom bar's toggles carry the same words as the headings they show and hide.
- **A layer number column** sits between the twirl and the label dot, in muted mono; the
  dot follows it and belongs to the name it colours, so the cluster reads twirl · number ·
  dot · name (K-458). **The three marks stand 8 apart** — the mockup's own row gap, where
  the outline had run the twirl hard against the dot and given the number 4.
- **The matte, blend and parent columns start at their content's width** (K-458): the
  mockup's 84 / 84 / 64 dropdown faces, the matte cell carrying its two mode toggles on top
  of that. Each seam still drags wider and stays where it is put.
- **The open composition tab carries no accent tick** — the seated surface colour alone
  marks it, as the mockup draws it. (The workspace tabs keep their accent underline;
  §3.1's "active tab tick" means those.)
- **The switches and Modes columns are fixed at their minimum width** — their toggles
  never stretch to fill a wider column, so there is nothing to gain by widening either,
  and neither seam is a drag handle (K-448; Modes added 2026-08-24 on the owner's word).
  Modes is exactly its five switch cells; the fold-out's value cells align to that span,
  so they are as wide as the column they sit under and no wider.
- **Comp-wide toggles live in the bottom bar**, not the timecode row: shy, motion blur
  and the overflow menu sit beside the Parent column toggle after a divider, so the
  comp-wide switches read apart from the column-visibility toggles.
- **The zoom control sits at the bottom left of the lane area** (a deliberate deviation
  from the mockup's bottom-right placement — it stays where the hand already is).
- **The layer search stays.** It sits in the row above the outline as an inset well
  stretched between the frame counter on the left and the mode tabs on the right,
  filtering the layer list as it does today; the redesign restyles it, never removes it.
- **The ruler is double height**: times and the playhead head in the upper half; markers and
  the work area in the lower. A marker is an upward triangle sitting on the cache bar, half
  inside its backdrop pill and half outside to its left; the pill starts at the triangle's
  point. **A marker that carries a duration draws a bar** running from its own frame for
  that long, on the same floor the flag stands on and hushed under it: the flag is what is
  read and aimed at, and a span at full strength read as a second work-area band. The bar
  takes no gestures — a span has no editing control yet, and one that could be grabbed but
  not moved would promise one. Minor ticks subdivide as zoom grows until one tick is one
  frame, and no tick is drawn closer than **30px** to its neighbour — which is the mockup's own density at the
  resting zoom, three half-second ticks between labels two seconds apart (K-451).
- **The cache bar is drawn on the ruler's floor, coloured by resolution tier** (§6.3) —
  on the work-area band's own row, inside the ruler's 36, not as a strip of its own
  beneath it. The band paints behind it; the marker flags stand on it.
- **A few pixels of padding sit either side of the ruler** in every timeline mode, so a
  keyframe or work-area handle on the first or last frame stays visible and grabbable.
- **The work area is one band** in `animated`, from the ruler's handles down through the
  lanes, drawn behind the cache bar.
- **Trimmed layers keep a faint outline of the full source extent**, showing how far each
  end can still be extended; clips inside a Sequence layer get the same per clip.
- **Layer bars (and the clips inside a Sequence layer) fill desaturated with a solid
  leading edge**, so a lane full of layers reads organised rather than carnival, and each
  bar's start still lands with a snap.
- **Keyframe diamonds on layer rows draw at half the row scale.**
- **The panel's bottom bar carries a toggle for the switches / modes / parent columns**
  (K-448), so the outline pares down to names and bars when the columns are not in use.

### 12A.1a Keys mode — the dope sheet (K-455)

- Keys keeps the **same double-height ruler, cache bar, work area, markers, playhead and
  zoom bar** as Layers mode, so all three views scroll the same range and nothing jumps on
  a switch. Only the body under them changes.
- **The outline's columns stand down.** In their place the second row carries the sheet's
  filters: **Show — All / Animated** (Animated is the default; it is K-441's filter, and
  `U` is the same reveal from the keyboard) and a scope of **Layers / Selected only**.
- **A layer's row, then its properties flat.** The layer's own row carries its twirl, its
  label-colour bullet, its name and — quietly, at the right — how many properties are
  listed under it. Each property is one row named by the container it came out of:
  `Transform · Position`, `Glow · Intensity`, with the value it reads at the playhead in
  `animated` at the right. There are no group headings: the flattening is the point.
- **The layer's lane is a band in its label colour**, carrying the half-scale summary
  diamonds while the layer is shut, exactly as its bar does in Layers mode; a twirled-open
  layer leaves the diamonds to the properties. The lane of a **picked** property is washed
  at foreground strength so it can be followed across the sheet.
- **A key's shape is its interpolation** (§6.2's rule, brought out of the graph): diamond
  linear, square hold, circle bezier. Keys draw larger here than on a Layers lane — they
  are the subject of the view.
- **The mode carries its drawing's editing tools** (K-458, superseding K-455's
  restriction): the block-selection box with stretch handles and its `n keys · n f`
  badge, the Ease popover on a selection, and the bottom bar's Interpolation /
  Reverse / Copy / Paste-at-playhead strip. Selecting a key, dragging one in time,
  snapping, the marquee and the undo step remain the lanes' own shared machinery,
  running under a different arrangement; the block tools live in the same machinery
  so Layers gains them wherever K-441 already names them as Layers behaviours.

### 12A.2 Graph mode (K-442)

- Graph mode keeps the **same double-height ruler, work area, cache bar and playhead** as
  Layers mode, so the two modes scroll the same range and nothing jumps on switch.
- The outline is the **filtered animated list** (Animated, the default, or All — there is no
  Selected), each row carrying a **tick in its curve's colour**. A setting restores an
  outline identical to Layers mode for those who want one shape everywhere.
- **Curves run flat to both edges of the visible area**; value labels live in a **fixed
  right-hand gutter**, never on the curve.
- The **thin horizontal scrollbar and the Value / Speed tool strip** (tangent mode, ease
  presets, Fit, zoom) run along the bottom of the **graph side only** — neither ever appears
  in the outline.
- Bezier, linear and hold segments are drawn distinctly (§6.2's shape rule, extended to the
  stroke).

### 12A.3 Properties and effect controls (K-443)

- **Fixed column edges.** Every row lays out on the same x positions: the stopwatch first;
  then a **reserved keyframe-navigation slot that stays empty until the property is
  animated** (so the label never moves when animation begins); then the label; then the
  control column at a fixed x. Rows that cannot be keyed simply omit the stopwatch — a
  ragged stopwatch column, a dead-straight label edge.
- **Tab hops values, pre-selected.** Tab and Shift+Tab move focus between value fields in
  visual order, skipping dropdowns and toggles (those stay reachable by ordinary focus
  traversal), and the field's content arrives selected so typing replaces it outright.
  The hop cycles within the panel. This is a binding behaviour of every value field, not
  a styling note.
- The stopwatch is **square under Sharp**.
- **Vector pairs are two equal wells with a link glyph between them.** **Shipped:** which
  parameters *are* a pair comes from the declaration (`list_pairs`), and which pairs are
  **chained** is remembered on the effect instance — empty means every pair separate, which
  is what every older project means, and toggling one is one undo step like every other
  effect edit. **Linked means proportional**: dragging or typing one well scales the other
  by the same factor, and that arithmetic lives in the panel for the life of the gesture,
  deliberately not in the model. Two cases leave the sibling alone rather than guessing — a
  value of **nought** has no factor at all, and a **keyframed** sibling would need a decision
  about what "scale a curve" means before it could be written.
- **Units are plain mono, never caps** (§7.1): mono at 10, muted, no tracking, in the gap
  beside the value. **Shipped:** the unit comes from the parameter's own declaration, so
  `centre_x` reads `%` on Radial blur and `px` on the dozen effects whose centre is px@comp
  — a distinction the frontend's own id-keyed table could not draw, and got wrong for all of
  them in the same direction. A parameter whose number genuinely has no unit draws no rider,
  and neither does a control carrying no number. A **pair** draws one rider for the two, after
  both wells: x and y are two halves of one measurement.
- **Position-type parameters get a crosshair point picker** — pick the point on the Viewer —
  exactly as colour parameters have an eyedropper. Which pairs those are is the declaration's
  answer too: a pair in `px@comp` writes fraction x comp size, a pair in per cent writes
  fraction x 100, and a pair measured in anything else is not a position and gets no
  crosshair.
- The per-effect **Mix row** carries blend mode and matte channel; the **Matte row** carries
  an invert.

### 12A.3a Project panel (K-451, K-454)

The approved mockup gives the panel six bands, top to bottom — preview card, search row,
column headers, item rows, scrollbar strip, bottom bar — at the heights §12A.6's table now
carries. The rules worth writing down:

- **The column headings are kicker words and the values sit under them.** Name (flexible),
  then Items, Size, fps and Path, each a fixed width, right-aligned, right-anchored, with
  8px between them and the same 8px inset at the right edge as the rows below. The header
  and the rows are laid out from one description of the columns, because the alignment
  came apart twice in the mockup rounds when they were not.
- **Values are mono at 10, muted**; the Path column is quieter again (`text_disabled`),
  because it is the one column that is context rather than fact about the item.
- **Per-type icon tints on media rows**, drawn from the layer-label palette (K-188), not
  from §6.1's muted layer family: azure for picture footage, indigo for sound, amber for
  solids. Folders and compositions stay muted — a folder's mark is the twirl beside it.
- **State reads as an outlined badge** beside the name: `in use` in `success`, `missing`
  in `warning`, each outlined in its own colour at ~28%, mono at 9 with no tracking. A
  badge reports a state, so it is deliberately **not** a kicker.
- **The search well is an inset well** (§2.1), the row's full width, at the standard 20.
- **The bottom bar carries the new-item controls at the left** — icon plus a 0.08em kicker
  word — **and a factual count at the right** (`1 missing · 10 items`) in mono at 0.06em,
  sentence case, never capitals: it is a statement, not a container label. The item total
  sits hard right, where the eye looks for it, and the missing half reads to its left —
  and that half is the "show only missing" filter. The two are separate strings laid out
  in that order, never one sentence with fragments spliced into it.
- **Glyphs render at 13 throughout, rows and bottom bar alike** (K-456, §5): the 16 grid is
  what they are drawn on, and the mockup's computed sizes are what they display at.
- **Width degrades in this order**: the preview card goes first (the docked mockup has
  none at 260), then Path, then Items, then fps, then Size; below the panel's minimum the
  tree scrolls sideways under the strip that has always been drawn for it. The bottom
  bar's words are shed before its icons, per §12A.6's step 4.

### 12A.4 Dialogs (K-444, K-449)

Every popup is built in-window today and becomes a **real OS window** when multi-window
ships (K-449: the toolkit support is not stable yet; ordinary framework dialogs are the
migration path). Either way they all share one pattern:

- a kicker title strip;
- an optional page-tab row under it (Export: General / Encoder / Colour / Metadata);
- label-left / control-right rows, labels in a fixed 110px column;
- hairline-separated, kicker-titled groups;
- a footer strip carrying a summary line (mono, factual: `600 frames · 10.0 s · 1920×1080 ·
  60 fps`) and **the single filled action**;
- paired buttons share one width (K-448).

### 12A.5 Feedback is transient and local (K-439)

Feedback appears under the cursor, lasts as long as the gesture, and leaves no trace: the
drag-scrub modifier ladder shows only while dragging; drop-target highlights appear only
over the target; things attached to what is being dragged move with the drag. **Nothing
changes the resting state** — a panel nobody is touching looks exactly as §2.1's three-greys
rule says it does.

### 12A.6 Viewer bar (K-448)

**The transport lives in the Viewer bar.** By default the bar's items may split between a
top and a bottom bar; a setting gathers everything into a single bar, at the top or the
bottom. K-411's instrument grouping carries over whichever arrangement is set
([07-UI-SPEC.md](07-UI-SPEC.md) §2.2).

### 12A.6 Metrics and degradation (K-451)

**The approved mockups' heights are canonical.** Chrome is built to these logical-pixel
values, not approximations of them; a mismatch is a defect. Vertical metrics never squish —
when a panel is too short, its content scrolls.

**Two densities, and Regular is the default** (K-454). Regular is what the mockups actually
render — their *effective* heights, content plus the seams and borders painted around it.
**Compact** is the settings toggle, a pixel or two off the rows that carry stacks, for
someone who would rather see four more layers than the air between them. Where the two
columns agree there is nothing to choose: that row measures the same either way, and it is
a plain constant in the code rather than a token with two equal values.

| Element | Regular | Compact |
|---|---|---|
| Panel header strip (title and tabs, composition tabs) | 22 | 22 |
| Secondary rows: timecode/search/mode row, column headers, filter rows, panel bottom bars | 19 | 18 |
| Outline and lane rows | 23 | 22 |
| Clip bars within a lane row | 16 | 16 |
| In-row pickers (the Timeline's matte, blend and parent cells), label at 10px | 18 | 16 |
| Dropdown closed face elsewhere in a panel or a bar | 20 | 18 |
| Property and effect-parameter rows | 27 | 26 |
| Effect section headings | 24 | 24 |
| Timeline ruler (times above, markers and work area below) | 38 | 36 |
| Cache bar (counted inside the ruler, so the clock above it is the rest) | 3 | 3 |
| Value wells in panels, the number inside them 11px mono | 20 | 20 |
| Project panel: preview card (10 of padding round a 96×54 poster frame, plus its hairline) | 75 | 75 |
| Project panel: search row (8 above the well, its 20, 6 below) | 34 | 34 |
| Project panel: item rows (the mockup draws these without the Timeline's seam) | 22 | 22 |
| Project panel: column-header row (a secondary row, hairline counted in) | 19 | 18 |
| Project panel: state badges (`in use`, `missing`) | 14 | 14 |
| Project panel: horizontal scrollbar strip (a 4px track inset 8 either side) | 6 | 6 |
| Project panel: bottom bar (new-item controls and the item count) | 20 | 20 |
| Project panel: glyphs — the twirl and type marks in a row (K-456) | 13 | 13 |
| Project panel: glyphs — the bottom bar's new-item controls (K-456) | 13 | 13 |
| Dialog title strip and dialog rows | 30 | 30 |
| Dialog page-tab row | 26 | 26 |
| Dialog value wells and dropdowns | 22 | 22 |
| Status bar | 18 | 18 |
| Graph-side horizontal scrollbar | 7 | 7 |

**The ruler is derived, not declared.** The lane side gives the ruler exactly what the
outline side spends on its two secondary rows, which is the whole reason the two halves of
the Timeline line up row for row. The mockup's own ruler measures a pixel under the sum of
the two rows it faces — the artboard disagrees with itself there — and of the two readings
only this one can be true of a panel whose halves have to meet.

**The Compact column is the set of values the editor shipped with** before the toggle
existed. Nothing about it is a second design: no colour, no size of type, no spacing across
a row changes with it, and the degradation ladder below is the same under both.

**The pieces inside a row are the mockups' too**, and are pinned by
`timeline_alignment_test` and, for the Project panel, `project_panel_metrics_test`: a layer's label colour is a **6px dot**, its number stands in an
**18px column** set in muted mono at 10, the keyframe diamond on a shut layer's row is a **4px square stood
on its corner** (≈5.7px point to point, against the 8px one a property's own lane draws), a labelled ruler tick is **7px** tall
against a minor tick's **4**, and under Sharp a bar's ends are **square** — the stadium ends are
Round's, and are the whole of that shape's difference here.

**When width runs out, things give way in this order** — earlier steps must be exhausted
before later ones, and nothing ever paints outside its box:

1. **Flexible text ellipsises**: names, paths and other user text truncate with an
   ellipsis on one line; values and units never truncate.
2. **Secondary control runs wrap**: riders and control clusters (Mix and Matte's blend,
   channel and invert; similar runs elsewhere) drop to a second line inside their column
   rather than eliding their words.
3. **Optional metadata columns hide**, least essential first (path, then fps, then size);
   names and states stay.
4. **Toolbars overflow**: buttons that no longer fit collapse into a single overflow menu
   at the end of the bar rather than shrinking or clipping.
5. **Below a panel's minimum width, it scrolls horizontally** instead of degrading
   further. Every panel declares a minimum; the dock will not shrink it past that.

The user-facing column toggles (switches, modes, parent) are the user's, not the
degradation ladder's — the ladder never flips them.

## 13. New-panel checklist

The Lumit equivalent of the household §9 checklist. Every new panel or feature MUST satisfy:

1. All colours from `&Theme`; zero hex literals (CI enforces); any genuinely new semantic gets
   a named token in the theme module first.
2. All numbers in Geist Mono tabular figures; every container label a 9–11px mono-caps
   kicker; body 11px Hanken Grotesk; nothing in chrome above 13px except dialog body
   emphasis (§7.1); at rest, at most three surface values (§2.1).
3. Terminology audited against [01-GLOSSARY.md](01-GLOSSARY.md); no banned terms in strings,
   identifiers, or docs.
4. Flat `surface_1` panel, `hairline` separation, radii from {4, 8, 16, full}; shadow only if
   the thing genuinely floats.
5. `accent` and `animated` are the only saturated state colours, each on its closed job
   list (§3.1); success/warn/error via `success`/`warning`/`error`; nothing within 48px of
   the Viewer image breaks neutrality.
6. Hit targets: ≥44px chrome controls, ≥24px visual + ≥32px slop dense controls (KD-2).
7. Keyboard path for every interaction, AccessKit roles/names, visible focus ring, contrast
   floors met (§9); any colour encoding paired with a non-colour one.
8. Micro-motion ≤150ms, reduced-motion path complete, user tempo never taken.
9. Copy: British English, sentence case, no exclamation marks, banner errors, no new jokes.
10. Works at the dense end: test at minimum row height, minimum panel width, and 125%/150%
    Windows display scaling.

## Brand: the mark and the splash (K-008, K-251)

**The mark: the twin keyframes** (K-251, replacing the Kiriko facet placeholder). Two
rounded keyframe diamonds side by side, as on a timeline — cool blue on the left,
violet-magenta on the right — and where they overlap the light goes additive white: a
white keyframe burning in the middle. Keyframes are motion, the overlap is compositing,
the white is luminance; the whole program in one glyph. The design followed a corpus
study of ~1,270 top-chart and editing-app icons (K-251 records the findings): at most
two hue families, one large glyph, dark tile, and none of the category's burned imagery
(play button, film strip, clapperboard, lens ring, colour wheel, AI sparkle).

Files, all in `assets/brand/`; the raster forms are regenerated by
`scripts/gen-icons.py`, except the macOS application icon, which Xcode compiles (K-309):

| File | Role |
|---|---|
| `lumit-mark.svg` | The bare mark, transparent — the Windows/Linux app icon (no tile; the white core is enclosed by the coloured keys, so it survives any background) |
| `lumit-icon.svg` | The mark on the dark rounded tile, flat. Reference drawing and single-image hand-out; nothing ships from it |
| `lumit-icon.icon` | **The macOS application icon** (K-309): the same composition as six Icon Composer layers, so macOS 26 lights it as Liquid Glass and renders the dark/tinted/clear variants. Xcode generates the pre-26 flat `.icns` from the same layers |
| `lumit-project.svg` / `.ico` | `.lum` project documents: dark folded-corner file, the twin keys, `LUM` kicker |
| `lumit-preset.svg` / `.ico` | `.lumfx` presets: same chassis, a single violet key (one applied stack), `LUMFX` kicker |
| `lumit-theme.svg` / `.ico` | `.lumtheme` colour themes (K-298): same chassis, three overlapping swatches in the two key gradients and the core white — colours rather than keyframes, because that is what the file carries — `THEME` kicker. The centres sit on an equilateral triangle whose circumradius **is** the swatch radius, so all three circles pass through the one point at the centre and share no area — which is what makes them equally visible, each giving up the same lens to each neighbour. They overlap **cyclically**: blue over white, violet over blue, white over violet, which no painting order can produce, so the violet swatch is clipped to outside the white circle instead. Nothing is painted where something else will cover it: a hidden shape still shows its softened edge pixels through the join as a hairline, two quarter-opacity rims stacking read as a blot, and a rim drawn as two arcs meeting end to end leaves a seam |

The SVG sources carry the mark's own palette and are the only permitted hex values
outside the theme module: keys `#86e2ff→#2f6fe0` (blue) and `#8a70ff→#ff4f9e`
(violet-magenta), core white/`#eaf4ff`, rim `#0c0e14`, tile `#16181d→#0d0f13`, bloom
`#b7c6e2`, document chassis `#181b21→#101217`, fold `#272b34`, kicker `#aab6c6`. The
wordmark is the word "Lumit" set beside or beneath the mark; no custom lettering. It was
drawn in Schibsted Grotesk and rides outlined in the SVG assets — the face is a brand
artefact, not part of the UI bundle (§7.1); whether the wordmark is redrawn in Hanken
Grotesk is a brand decision taken when the mark is next touched.

**The macOS layers carry no lighting of their own** (K-309). The layer SVGs inside
`lumit-icon.icon` are the flat icon's own geometry with three things deliberately
removed: the tile's corner radius, because macOS applies the platform squircle mask
itself; the drop shadow under the keys, because Liquid Glass generates the shadow per
layer; and the keys' dark rim stroke, which in the flat icon stands in for a lit edge
that the system now draws for real. Painting any of them in means it doubles up in
every appearance the system offers — the same edge twice, once painted and once lit. The mark MUST also be paintable from theme-module constants in code
(four rounded rects and three gradients, no raster assets) so the splash and about box
never ship image files.

**Splash art direction (unchanged).** The destination for the splash's artwork remains,
in the owner's words, a **broken-glass look**, styled like something out of Persona 5 —
hard-edged silhouettes, aggressive shard-shaped composition, beautiful graphic
stylisation: the mark or a silhouette figure seen through fractured glass shards, flat
high-contrast shapes on the dark ramp, mist filling the negative space. Persona 5 is the
energy reference, not a template to copy — no borrowed assets or traced compositions.
The boot-log splash below ships with the plain mark now; the art replaces the mark's
slot without changing the splash's structure.

**The splash.** A small frameless window, centred on the monitor (~460×300), surface_0,
shown while the application boots:

- Contents: the mark (≈96 px), the wordmark, version in 10 px mono, and the **boot log** —
  a Geist Mono list that shows each module and effect as it initialises ("Workspace:
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

- **Exact ramp values under real hardware.** §2.1 targets were chosen on paper; they MUST be
  validated on a consumer gaming monitor (the audience's hardware — often wide-gamut,
  aggressively vivid presets) before being frozen. Does `surface_0` at `#0b0c0e` hold up on an
  sRGB laptop panel at low brightness?
- **Viewer surround options.** Should the neutral-surround slider expose named stops
  (Dark/Mid/Match panel) or a continuous value? Grading convention favours a couple of fixed,
  documented greys.
- **Layer-type colour user overrides.** AE users expect per-layer label colours. If Lumit
  offers them, the picker SHOULD be a curated muted swatch set derived from §6.1, not a free
  colour wheel — otherwise the Timeline's calm is one preset pack away from destruction.
- **Text rendering at 11px.** K-012 flags text polish as a known risk; if 11px mono kickers
  render poorly on Windows ClearType, the dense scale may need to shift to 12/13/14px.
  Decide after the first Timeline prototype.
- **Wide-gamut / HDR Viewer output.** When the Viewer gains HDR output, the neutrality zone
  rules need restating in display-referred terms; the SDR spec here deliberately ignores it.
- **`animated`'s closed list.** K-439 closes the token's jobs at six, but two of them — the
  focused value field and the work-area band — are not keyframe state or a selected handle,
  and the sparing-use intent behind the token argues for trimming them. Confirm with the
  owner; if either goes, a superseding decision entry trims the list.
