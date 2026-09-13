# Lumit design language — Lantern

**Status: speculative. Not canonical, not binding on any code.** This is the third
alternative to [15-DESIGN.md](../15-DESIGN.md), written to the same section skeleton as the
canonical document and [15-DESIGN-RAMS.md](15-DESIGN-RAMS.md) so the three can be read one
section against its counterpart. Where this document is silent, the canonical one holds.

It is the style meant to **replace Round**. The canonical §12.1 took OUTLOUD's Lyrica editor
as a reference for a *shape* — a set of geometry tokens over the canonical colours — and
rejected the reference's own arrangement (a light shell around dark cards) as "a different
theme, not a shape". This document takes it as what it is: a whole style, with its own room,
its own cards, its own type and its own controls, and every function in
[07-UI-SPEC.md](../07-UI-SPEC.md) kept.

**On the reference.** The Lyrica pages could not be fetched from the sandbox this was written
in, so the reference is read through the cues the canonical document records from it
(stadium controls; the transport gathered into one pill; filled-pill actives; bigger cards;
the header dot; capsule bars; dot slider thumbs; a light shell around dark cards) and memory
of the app. Corrections against the real thing are expected and welcome.

Visual reference: **[mockups/lantern-shell.html](mockups/lantern-shell.html)**, both rooms,
with the structural moves numbered.

---

## 0. The premises

Lyrica is a consumer-grade tool that does a pro job without looking like one, and that is the
whole of what this style borrows. Five premises, each of which a reviewer can fail a
screenshot against:

1. **The room and the cards.** Chrome is not a single surface. Each pane is a *card* — a dark,
   rounded, softly shadowed object — and the cards sit in a *room* that is a different colour
   from them. The room is the one place the eye rests; the cards are where the work is.
2. **Everything you can press is a capsule or a circle.** No rectangle is interactive. A
   button is a circle, a control with a word in it is a stadium, and the fronted one of any set
   is filled with the accent. State reads from fill, not from tint.
3. **One accent, spent on fills.** The accent is never the colour of a word on a card and never
   a stroke; it is the fill of the thing that is in hand, fronted, or playing.
4. **Elevation is real.** A card casts a shadow into the room because it is above it; a floating
   capsule casts a deeper one because it is higher. Nothing else casts anything, and nothing
   blurs, frosts or bevels — the canonical rule against glass stands.
5. **Friendly, not soft.** The type is a geometric sans in sentence case at ordinary sizes; the
   marks are the canonical marks with their corners eased; the density is the canonical density.
   It is the same tool, in a nicer coat, and it must not become slower to read.

## 1. Relationship to the canonical language

### 1.1 Kept, unchanged

- **Semantic tokens only**, hex confined to the theme module.
- **Viewer-surround neutrality** (§2.1, §3.3) — `#7f7f7f` inside the Viewer card, in both rooms.
- **No punishment UI**, **never colour alone**, **the user controls tempo**.
- **KD-2's hit-target compensation.**
- **Mono for all numbers** (canonical §7.1) — kept absolutely, in a friendlier mono (§7.2).
- **British English, sentence case, no exclamation marks, no emoji**, and the one rationed joke.
- **AccessKit from day one**, keyboard operability of every control, visible focus.
- **Every behaviour** in [07-UI-SPEC.md](../07-UI-SPEC.md) — the dock, the drop zones, workspaces,
  floating windows, the keymap.
- **Every behavioural ruling in the canonical §12A** — fixed column edges, Tab hopping
  pre-selected values, the conditional matte column, the degradation ladder, the minimum
  widths, the Viewer bar's shedding order, the metrics tables' *heights* (see §7.1).

### 1.2 Overturned, with the reason

| Canonical rule | Lantern | Why |
|---|---|---|
| Dark-first; panels butt on hairlines; Round is a shape over the same colours | **Cards in a room** (§2): dark cards, 18px radius, 12px apart, in a light room by day and a near-black one by night | Premise 1 — the reference's own arrangement, taken whole |
| Radii `{4, 8, 16, full}` | **`{18, 12, 10, full}`**: cards, sub-cards and stages, rows, and everything interactive a capsule or a circle (§12) | Premise 2 |
| `accent` spent on the playhead, one filled button, the active tab tick; `animated` for keyed state | **One accent on fills** — the armed tool, the fronted workspace and mode and comp tab, the play button, the selection, the playhead, the work area, Add effect, Export — and `animated` dissolved (§3.2) | Premise 3; and a keyed property is a filled stopwatch here, as in the Rams document |
| Kickers: mono caps on every container label | **Card titles in 12px semibold sentence case with the header dot**; column headers and hints 9.5px muted sentence case; no caps anywhere in chrome (§7.2) | Premise 5, and the canonical §12.1's own rejection of shouted headers |
| Hairline elevation; `shadow_float` only on the closed list | **A card shadow on every card**, a deeper one on floating capsules; no hairlines between panes at all (§2.3) | Premise 4 |
| Input wells inset on `surface_0` | **Capsule wells** — the same inset, rounded full (§3.4) | Premise 2 |
| The toolbar strip | **A floating tool dock**: a horizontal capsule at the foot of the Viewer's stage, over the neutral surround (§12B.1) | The reference floats its toolbar over the canvas |
| Motion ≤150ms with one signature overshoot | **Kept, and this is where it lives** (§8): filled pills slide between positions, the drag ghost has its overshoot | Premise 5 says friendly; the canonical budget already allows it |

### 1.3 The honest cost

A light room by day is a light surround for a picture, with the same consequence the Rams
document states: ambient goes up, the eye adapts, the picture reads darker. The Viewer card
contains it — the picture sits on a `#7f7f7f` stage inside a dark card, so the immediate
surround is exactly what grading wants — but the room beyond the card is bright. Night exists
for the same reason Graphite does. **Day for editing and design, Night for grading**, and the
neutral stage binding in both.

## 2. The room and the cards

### 2.1 Day

| Token | Value | Role |
|---|---|---|
| `room` | `#e4e4ea` | The ground the cards sit in, and the top band |
| `room_2` | `#d9d9e1` | A step in the room where one is needed (a hovered room control) |
| `card` | `#1e1e25` | Every pane |
| `card_2` | `#27272f` | Sub-cards and strips inside a card: the specimen, an inspector section, the lane area, the transport pill |
| `well` | `#15151b` | Inset: value capsules, the search, the lane ground, segmented-pill tracks |
| `float` | `#2d2d37` | Floating capsules: the tool dock, menus, popovers, the at-effect chip |
| `raised` | `#35353f` | Pressed fills, scroll thumbs, the marker pill |
| `viewer_surround` | `#7f7f7f` | The stage inside the Viewer card — exactly neutral, in both rooms |

### 2.2 Night

`room` `#0f0f13` · `room_2` `#17171c` · `card` `#1c1c22` · `card_2` `#25252c` · `well` `#121217`
· `float` `#2b2b33` · `raised` `#33333c` · `viewer_surround` `#7f7f7f`. The cards barely
move; the room does. Night is Day with the lights turned down, not a second design.

### 2.3 Text, lines, shadows

| Token | Value | Role | Contrast on `card` |
|---|---|---|---|
| `text_primary` | `#f3f3f7` | Titles, values, the selected name | 14.9 |
| `text_secondary` | `#c3c3cd` | Body, property names, pill words | 9.5 |
| `text_muted` | `#8d8d9a` | Column headers, hints, inactive pills | 4.6 |
| `text_disabled` | `#5c5c69` | Disabled controls | 2.3 (exempt) |
| `room_ink` | `#23232b` / `#e9e9ef` | The menus on the room, by room | — |
| `hairline` | `#33333d` / `#30303a` | The only line: a seam inside a capsule, the tool dock's dividers | — |

**No hairline separates panes.** The room does. Inside a card, rows are separated by their own
rounded fills and by gaps, not by lines; the one place a line appears is a seam inside a
capsule (the transport's dividers, the tool dock's two seams).

**Shadows.** `card_shadow` = `0 6 18 rgba(20,20,30,.20)` + `0 1 2 rgba(20,20,30,.12)` by day,
deeper by night; `float_shadow` = `0 10 30 black@35%` (55% at night) on the tool dock, menus,
popovers, and the workspace and palette pills on the top band. Nothing else casts.

## 3. Colour

### 3.1 The accent

| Token | Day | Night | Referent |
|---|---|---|---|
| `accent` | `#6a5aeb` | `#7c6cff` | **In hand, fronted, or playing** — as a fill only |
| `accent_ink` | `#ffffff` | `#ffffff` | The word on an accent fill (5.0:1 by day) |
| `accent_soft` | accent @22% | | The selected row's fill; the work-area band |

The accent is violet-blue because the reference's palette is cool and because it has to sit
beside six warm-and-cool layer hues without being mistaken for one. It is retunable — both
rooms build from one `default_accent` exactly as the canonical `with_accent` does — but the
job list is not: **the accent never sets a word on a card and never draws a stroke**. The one
stroke it draws is the selected sub-card's inset ring (§5) and the gizmo on the picture, which
is a tool.

### 3.2 States, and `animated` dissolved

| Token | Value | Referent |
|---|---|---|
| `on` | `#4fc48f` | Done as asked; the cache-ready bar; a held cache run; `in use` |
| `attention` | `#e9b84a` | Look here: `missing`, overrun, the held peak |
| `fault` | `#f26d8d` | Failed: the banner's mark, the clip lamp |

`animated` goes, for the reasons the Rams document gives (§3.2 there): a keyed property is a
**filled stopwatch** (a filled accent circle against a ring), its keys are marks on its lane,
and selection is one idea with one colour. The work-area band takes `accent_soft`; the focused
field takes the ordinary focus ring.

### 3.3 The neutrality zone

Binding. The Viewer's stage is `#7f7f7f` with a 12px radius inside the card; within 48px of the
picture nothing is saturated except overlays on the picture itself (the gizmo, the selection
name capsule, guides). **The tool dock floats over the surround at the stage's foot, and the
stage's padding is sized so the dock's accent-filled armed tool is never inside the belt.**
That is a layout constraint, stated here so it is not lost: the stage's bottom padding is
52 and the dock is 38 tall, so the dock's top edge is at least 48 from a picture that fits.

### 3.4 The capsule well

An editable value is **its number in a capsule** — a `well`-filled stadium 22 tall, the number
in mono at 11 and the unit rider in `text_muted` at 9.5. It is the canonical inset well with
its corners rounded full; drag to scrub, click to type, the whole value selected; the focus
ring is a 1.5px `accent` stroke round the capsule.

## 4. The token layer

Structurally the canonical `LumitTheme`; the delta is what it carries:

```
- pub surface_0..4        → pub room, room_2, card, card_2, well, float, raised
                              (seven, because the room and the card are two grounds —
                               the same split the Bauhaus document found for surface_0)
- pub accent_hover        → (removed: hover is a surface step)
- pub animated            → (removed: §3.2)
- pub success/warning/error → pub on / attention / fault
+                           pub room_ink: Color32,       // the menus on the room
+                           pub card_shadow, float_shadow
```

`ShapeTokens.lantern` (§12) carries the geometry. The `.lumtheme` machinery, the custom-theme
editor and the seven shipped schemes are unaffected; this is two more schemes and one more
shape beside them — which is the whole reason it is the cheapest of the three envisionings to
build.

## 5. Iconography

The canonical set, unchanged in grammar (16 grid, one weight, `currentColor`, no emoji), with
**round caps and round joins** and the stroke at **1.5px**: a friendlier line than the Rams
document's 1.25 butt-capped one, and the canonical set's own. Every glyph sits in a **circle
button** (§12), so a glyph never needs a box of its own. The Channels mark keeps its three
colours. The chrome-labels setting is kept, with words the default.

## 6. Editor semantics

### 6.1 Layer types — a brighter family

On a dark card the canonical muted family disappears, and the reference's own bars are vivid.
Six hues at one value, brighter than the canonical set and still one clear step below the
accent:

| Layer type | Value |
|---|---|
| Footage | `#5fa3d6` |
| Sequence | `#7d8ee0` |
| Precomp | `#c17ad1` |
| Solid / Adjustment | `#9a9aa6` |
| Text | `#e5b56a` |
| Camera | `#d7c36a` |
| Audio (reserved, drawn here) | `#55c9b4` |

Drawn as: the **avatar circle** beside the layer's name in the outline (a 16px disc in the
hue, the type glyph inside it in `card`), and the **capsule bar** on the lane at 55% of the
hue, 90% when selected. **Selection still beats every one of them**: a selected bar wears a 2px
`accent` ring outside its capsule.

### 6.2 Keyframes and curves

The canonical marks — diamond linear, square hold, hourglass bezier — split at the vertical
centre, at 11px, with their corners eased by 1px. Rest `text_secondary`, selected `accent`.
Curve strokes take the four-step ramp re-derived on this family: `#5fa3d6` / `#55c9b4` /
`#e5b56a` / `#c17ad1`. Bezier handles are `text_muted` stems with **round dot ends**, `accent`
while grabbed. Graph paper is `well` with `hairline` lines and the zero and 100% lines in
`text_muted`.

### 6.3 The cache bar

The canonical two-family, three-height design, drawn as **rounded runs with a 2px gap** between
neighbours so each run reads as its own capsule: `on` for held, `#5fa3d6` for disk; full / 70%
/ 45%. Uncached draws nothing.

### 6.4 Overrun, markers, waveforms

- **Overrun**: `attention` hatching over a 14% wash with the `hold` tag; unchanged.
- **Markers**: a **grey dot** (8px, `#c4c4c4` on the card) on the ruler's floor with its label
  in a `raised` pill; one per frame. The triangle was the canonical flag's answer to standing
  on a bar; a dot is this style's.
- **Waveforms**: `#55c9b4` at 90% opacity on the capsule, the multiwave stack ranked by value.

### 6.5 Selection, focus, drop targets

- **Selection**: a **fill** — `accent_soft` on a row, `accent` ring on a bar or a key, `accent`
  inset ring on an inspector section. No 1px accent border on a row: a filled row is the
  reference's grammar.
- **Focus**: a 1.5px `accent` ring round the control's capsule or circle.
- **Drop targets**: `accent_soft` fill with a 1.5px dashed `accent` ring; an insertion caret is
  a 2px `accent` capsule.
- **The playhead is a pin**: a 2px `accent` line with a 12px round `accent` head ringed in
  `card_2`, so it reads over any ruler ground. The head is the grab.
- **The work area** is an `accent_soft` capsule on the ruler's second row with two 6×18 round
  `accent` handles.

## 7. Density and type

### 7.1 Heights

The canonical §12A.6 heights are kept for everything that scrolls (outline rows 23 → drawn at
26 here with a 2px gap, which is the same 28 pitch the mockup uses for lanes; property rows 28;
the ruler 48), and the card chrome takes the reference's more generous air: **card titles on a
40px line, the deck 56, the top band 44, the tool dock 38**. Compact is the canonical Compact:
the rows lose their gap and a pixel.

### 7.2 Type

**Two faces, both SIL OFL, both bundled: Plus Jakarta Sans** for words and **DM Mono** for
every number, timecode, count and readout. Jakarta is a geometric sans with a friendly lowercase
and clean figures; DM Mono is the calmest geometric mono on offer and pairs with it.

| Size | Face | Use |
|---|---|---|
| 9px | DM Mono | Ruler numbers, the frame count's total, tiny readouts |
| 9.5px | Jakarta | Column headers, hints, the readout pill — `text_muted`, sentence case |
| 10.5px | Jakarta | Pill words |
| 11px | Jakarta / DM Mono | Body, property names / values in capsules |
| 12px | Jakarta **semibold** | Card titles, section names |
| 15px | DM Mono medium | The clock |
| 24px+ | Jakarta | About box, welcome, empty states |

**Sentence case everywhere, capitals only where English puts them.** No kickers, no tracking,
no caps.

### 7.3 Hit targets

KD-2 unchanged. The tool dock's circles are 30 with the capsule's padding making 38; that is
below the 44 chrome floor and is compensated the dense-surface way (≥32 slop, nearest-wins),
because the dock is a floating strip aimed at from the picture and 44px circles would cover it.
**This is the one KD-2 concession this style asks for and should be tested first.**

## 8. Motion

The canonical §8 in full — ≤150ms, transform and opacity, spring-like ease-out, the three
tiers, playback exempt — **and this is the style the signature interaction was written for**:
the drag ghost lags and settles with its small overshoot; a filled pill **slides** between
positions when a segmented set changes (workspace, mode, comp tab); the tool dock's armed circle
slides likewise. Under *None* every one of them cuts.

## 9. Accessibility

The canonical §9 in full. Contrast figures are in §2.3 (`text_muted` 4.6:1 on `card`); the
accent-filled pill's white word is 5.0:1 by day and 5.6:1 by night. **A filled pill is never the
only encoding of a state**: the fronted workspace is also first in reading order and named in
the Window menu; the armed tool is also the one with the index in its tooltip.

## 10. Voice

Canonical §10 unchanged, including the joke. Card titles and pill words are sentence case
("Effect controls", "Add effect", "Paste at playhead"); tooltips one or two words.

## 11. The two rooms

§2's two token sets are the whole difference; no widget branches on the room. Day is the style;
Night is what a grader sets and forgets.

## 12. Shape

```
static const lantern = ShapeTokens(
  controlRadius: stadium,   // every control a capsule; buttons are circles
  floatRadius: 16,          // menus, popovers, the tool dock's capsule ends
  cardRadius: 18,           // the pane
  cardPadding: 0,           // the card's own header line does the inset
  tileGap: 12,              // between cards
  windowInset: 12,          // from the room's edge
  cardShadow: [card_shadow],
);
```

plus the two grammar fields both other envisionings want — `labelCase` (sentence) and
`strokeWeight` (1.5, round caps) — and one this style alone needs: **`subCardRadius: 12`** for
the stages, sections and lane areas inside a card, and **`rowRadius: 10`**.

Round's stated permanent limitation — stacked tab-bar containers stay square-cornered — is
inherited and should be solved rather than restated: a tab group here is a card whose header
line carries pill tabs, and the docking container needs the hook.

## 12B. The shell, laid out again

Every panel, control and behaviour kept; the chrome laid out on the reference's grammar.

### 12B.1 The floating tool dock

The toolbar (07 §1.7) becomes **one horizontal capsule floating at the foot of the Viewer's
stage**, over the neutral surround: thirteen circle buttons in the spec's order and groups, a
seam after the six simple tools and before the snapping switch at the end. The armed tool is
an accent-filled circle; a group carries the member last used and a 3px corner triangle;
press-and-hold and right-click open the flyout **above** the dock as a second capsule; tool
options open the same way while a tool with options is armed. Unbuilt tools draw
`text_disabled` and decline. The dock is the Viewer's, so it moves with the Viewer card and
appears once per Viewer panel; a Viewer too small for it folds the groups into an overflow
circle (the canonical ladder's step 4).

### 12B.2 The top band

The room's own 44px band, not a card: the mark; the nine menus as sentence-case words in
`room_ink` (the system bar on macOS); a **workspace pill** — a `card`-filled capsule with the
fronted name as an accent-filled pill; the **command palette** as a capsule at the right.

### 12B.3 Cards

Each pane is a card with a 40px header line: the **header dot** (6px `accent`, decorative,
never a status light), the title in 12px semibold, and the pane's own controls at the right as
pills and circles. A tab group's header carries pill tabs, the fronted one `card_2`-filled.
Docking is unchanged.

### 12B.4 The Viewer

- **The bar above the stage** holds the ways of looking as pills and circles: magnification,
  resolution, the channel mark, board, view menu, 3D view, exposure, the snapshot pair, the
  colour pipeline. The degradation reading sits at the right in `text_muted`.
- **The stage** is a `#7f7f7f` sub-card with 12px corners; the picture centred in it above the
  dock's band; the selection name as an accent capsule top-left; the at-effect chip as a
  `float` capsule top-right.
- **The deck** (56): the clock as a `well` capsule at the left with the frame count beside it;
  **the transport as one `card_2` pill** with a 34px accent play button at its centre and the
  four marks either side; loop, preview mode and quality as pills; audio as a circle; the
  cache-ready bar as a 54×6 rounded meter with its figure. §9's Preview panel, docked by the
  shell.

### 12B.5 The Timeline

- Header: title and dot; the comp tabs as a `well`-tracked segmented pill; the Layers / Graph
  segmented pill and the accent-filled **Export** at the right.
- The outline's chrome row: the layer-search capsule, shy and motion blur as small circles, and
  the three group toggles as ghost pills at the right.
- Column headers 9.5px muted; rows 26 with a 10px radius and a 2px gap; the selected row filled
  `accent_soft`; switches and modes as bare glyphs; pickers as 20px capsules.
- The lane area is a `card_2` sub-card; the ruler's scale on its upper half; the work-area
  capsule and round handles; rounded cache runs; dot markers; the pin playhead. Bars are
  capsules at 55% of the hue; outside the work area the lane ground dims.
- The foot: the ease strip and its pills, then the measuring clock; on the lane side the zoom
  as a track with a white dot thumb between the two landscape glyphs, the magnet, the scrollbar.

### 12B.6 The inspector

One list, each section a **sub-card** (`card_2`, 12px): the heading with twirl, an
accent **toggle switch** for enable, the name in semibold, the cost and the ×; rows of stopwatch
circle, navigator, name and capsule wells; sliders as accent tracks with white dot thumbs; the
selected section wears the accent inset ring. Add effect is the accent-filled pill at the foot,
Save preset a ghost one.

### 12B.7 The readout pill

The status line becomes **a capsule floating in the room's bottom-right corner**: the message,
the three cache meters as 36×4 rounded bars, the frame cost and dropped count. It casts the
card shadow and covers nothing, because the dock leaves it 40px of room.

## 13. New-panel checklist

1. Every colour from the theme struct; zero hex literals outside it.
2. The pane is a card (18) in the room; sections and stages are sub-cards (12); rows 10;
   everything interactive a capsule or a circle.
3. Card title 12px semibold with the header dot; column headers 9.5 muted; sentence case
   throughout; no caps, no kickers.
4. Mono for every number.
5. The accent only as a fill on §3.1's list; `on` / `attention` / `fault` for the three states;
   the layer family and curve ramp for content.
6. Shadows: the card's and the float's, nothing else; no blur, no glass.
7. Nothing saturated within 48px of the picture; the tool dock outside the belt.
8. Hit targets: 44 chrome, 24 + 32 slop dense, the dock's 30 + slop tested.
9. Keyboard path for everything; AccessKit; the accent focus ring.
10. Motion ≤150ms with the tiers; filled pills slide; nothing animates at rest.

## Brand

The twin-keyframe mark keeps its gradients — this is the one style where a gradient on the
brand does not fight the chrome — and its keys' corners round to match the cards. The wordmark
stays. The splash is a **card in the room**: the mark, the wordmark, the boot log, and a
rounded accent progress bar along its foot.

## What it would cost to build

This is the cheapest of the three envisionings, because the canonical Round shape already built
most of its machinery:

- **Two `LumitColorScheme` entries** (`lanternDay`, `lanternNight`), each a full token set with
  the room/card split (§4) — which means the `surface_0` split the other two documents also
  want.
- **`ShapeTokens.lantern`** replacing `round`: the existing fields plus `subCardRadius` and
  `rowRadius`, and the two grammar fields.
- **The floating tool dock** is the toolbar widget re-hung inside the Viewer panel with its
  flyouts opening upward — the same re-layout the Rams rail needs, in a different direction.
- **The deck** is the Preview panel docked by the shell — shared with the Rams work.
- **Filled-pill actives, the toggle switch, the dot thumb and the capsule well** already exist
  under Round (§12.1's bubble commit); they need the new tokens, not new widgets.
- **The tab-group container's corners** are the one piece of docking work.

## Open questions

- **Is the reference read right?** The pages were unreachable; the cues are the canonical
  document's and memory's. The first thing to do is put the mockup beside the real app.
- **The tool dock's 30px circles** against KD-2's 44 chrome floor (§7.3).
- **The light room by day** beside a graded picture — the same experiment all three
  envisionings need, and here the Viewer card contains it best.
- **Does a dot marker survive on a busy ruler?** The canonical triangle was chosen to stand on
  the cache bar; a dot may read as a cache run's end.
- **Name.** See the folder README's naming proposal; "Lantern" is a working title.
