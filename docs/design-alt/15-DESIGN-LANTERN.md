# Lumit design language: Lantern

**The style is called Lantern** (settled 2026-09-13), beside **Studio** for the canonical
language and **Desk** for the grey-room one. A style names the arrangement; a room names the
palette, which is why Lantern's own two rooms are Day and Night.

**Status: built, and history.** Lantern ships as one of the three shapes, and
[15-DESIGN.md](../15-DESIGN.md) §12 is the canonical statement of what it draws. This is the
study it was drawn from, written to the same section skeleton as the canonical document and
[15-DESIGN-DESK.md](15-DESIGN-DESK.md) so the three can be read one section against its
counterpart. Where this document and the canonical one disagree, the canonical one holds;
the notes marked *as built* below say where the build parted from the drawing.

It is the style meant to **replace Round**. Round was only ever a set of geometry tokens over
the canonical colours, and it stopped short of the one idea worth having: that a pane can be an
*object in a room* rather than a tile in a grid. This document takes that idea whole, with its
own room, its own cards and its own controls, and every function in
[07-UI-SPEC.md](../07-UI-SPEC.md) kept.

**Every colour here is Lumit's own.** The dark ramp of [15-DESIGN.md](../15-DESIGN.md) §2.1 is
not replaced, it is re-arranged: a card is `surface_1`, a band inside it `surface_2`, a control
`surface_3`, a pressed fill `surface_4`, and a lane well `surface_0`. The accent stays spruce
and the three state colours stay as they are. The only token this style adds is the **room**,
because a ramp built for panels that butt together has no name for the ground they would float
in, and Lumit's own Light canvas (§11) is what that ground is.

Visual reference: **[mockups/lantern-shell.html](mockups/lantern-shell.html)**, both rooms,
with the structural moves numbered.

---

## 0. The premises

A tool can do a professional job without looking severe about it, and that is the whole of what
this style is after. Five premises, each of which a reviewer can fail a screenshot against:

1. **The room and the cards.** Chrome is not a single surface. Each pane is a *card* — a dark,
   rounded, softly shadowed object — and the cards sit in a *room* that is a different colour
   from them. The room is the one place the eye rests; the cards are where the work is.
2. **Shape says what a thing is.** Four radii do the talking, and mixing them up is the defect:
   a full pill is an action or the fronted half of a set, a rounded rectangle is a field you
   type in or a button you press, a near-square is content sitting on a lane, and a card is the
   pane itself. The fronted one of any set is filled, so state reads from fill rather than
   from tint.
3. **One accent, spent on fills.** The accent is never the colour of a word on a card and never
   a stroke; it is the fill of the thing that is in hand, fronted, or playing.
4. **Elevation is real.** A card casts a shadow into the room because it is above it; a floating
   capsule casts a deeper one because it is higher. Nothing else casts anything, and nothing
   blurs, frosts or bevels — the canonical rule against glass stands.
5. **Friendly, not soft.** A geometric sans at ordinary sizes, the canonical marks with their
   corners eased, and the canonical density. Container labels stay small, uppercase and
   tracked, centred over the card they name, which is the one place this style keeps a kicker. It is the same tool in a nicer coat, and it must not become
   slower to read.

## 1. Relationship to the canonical language

### 1.1 Kept, unchanged

- **Semantic tokens only**, hex confined to the theme module.
- **Viewer-surround neutrality** (§2.1, §3.3) - `#121212` inside the Viewer card, in both rooms.
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
| Dark-first; panels butt on hairlines; Round is a shape over the same colours | **Cards in a room** (§2): dark cards, 18px radius, 12px apart, in a light room by day and a near-black one by night | Premise 1, taken whole rather than as a set of corner radii |
| Radii `{4, 8, 16, full}` | **`{16, 12, 7, 3}` and the pill** (§12): the card, a section or stage inside it, a field or button, content on a lane, and a full pill for actions and active segments | Premise 2 |
| `accent` spent on the playhead, one filled button, the active tab tick; `animated` for keyed state | **The same spruce accent, on fills, with a wider closed job list** (§3.1), and `animated` dissolved (§3.2) | Premise 3; and a keyed property is a filled stopwatch here, as in the Desk document |
| Kickers: mono caps on **every** container label | **Kickers kept for card titles alone**, centred, with the dot beside them; every other label is sentence case (§7.2) | The reference does exactly this, and a caps label earns its shout once per card rather than forty times per screen |
| Hairline elevation; `shadow_float` only on the closed list | **A card shadow on every card**, a deeper one on floating chrome; no hairlines between panes at all (§2.3) | Premise 4 |
| Input wells inset on `surface_0` | **Rounded-rect fields at 7**, not capsules; the capsule is for actions (§3.4) | Premise 2, measured off the reference |
| The toolbar is one strip that "cannot be closed, moved, tabbed or floated" (07 §1.7) | **The toolbar gains a position: Top or Left** (§12B.1). This is the one change in this document that reaches the UI spec, and it is written up as an amendment rather than assumed | A rail restores the full 44x44 target the strip gives up, and which of the two costs less is a matter of the monitor and the taste |
| Motion ≤150ms with one signature overshoot | **Kept, and this is where it lives** (§8): filled pills slide between positions, the drag ghost has its overshoot | Premise 5 says friendly; the canonical budget already allows it |

### 1.3 The honest cost

A light room by day is a light surround for a picture, with the same consequence the Rams
document states: ambient goes up, the eye adapts, the picture reads darker. The Viewer card
contains it, the picture sits on a `#121212` stage inside a dark card, so the immediate
surround is exactly what grading wants — but the room beyond the card is bright. Night exists
for the same reason Graphite does. **Day for editing and design, Night for grading**, and the
neutral stage binding in both.

## 2. The room and the cards

### 2.1 Day

**Nothing in this table is a new colour except the room.** The right-hand column is the
canonical token each one already is.

| Token | Value | Role | Canonically |
|---|---|---|---|
| `room` | `#d9d9d6` | The ground the cards sit in, and the bands above and below them | Light's canvas (§11) |
| `room_2` | `#cdcdc9` | A step in the room where one is needed, a hovered room control | Light, one step down |
| `card` | `#131517` | Every pane | `surface_1` |
| `section` | `#1a1d20` | A band inside a card: the specimen, an effect's rows, the lane area, the transport group | `surface_2` |
| `field` | `#212528` | A value box, a dropdown, an icon button, a segmented track | `surface_3` |
| `raised` | `#2b3034` | The filled half of a two-state segment, pressed fills, scroll thumbs, the marker pill | `surface_4` |
| `sunk` | `#0b0c0e` | The lane well and graph paper, inset below the card | `surface_0` |
| `viewer_surround` | `#121212` | The stage inside the Viewer card, exactly neutral, in both rooms | unchanged |

The card sits 12.9:1 against the day room, so a card edge needs no border by day. The stage is
the one place this costs something: `viewer_surround` against `surface_1` is 1.02, so the stage
is bounded by a **hairline** rather than by value. That keeps the surround strictly neutral and
at Lumit's own figure, which §2.1 requires and no arrangement may trade away.

### 2.2 Night

`room` `#0b0c0e`, `room_2` `#131517`, and every other token unchanged. The room drops to the
canonical canvas and the cards do not move at all, so a card gains a `hairline` edge inside its
shadow to keep reading. Night is Day with the lights turned down, not a second design, and it
is the cheapest thing in this document: at night the whole style is the shipped dark ramp with
gaps and corners.

### 2.3 Text, lines, shadows

All four text tiers are the canonical ones, unchanged, and all four clear their floors on a
card without retuning:

| Token | Value | Role | On `card` |
|---|---|---|---|
| `text_primary` | `#eef1f2` | Card titles, values, the selected name | 16.1 |
| `text_secondary` | `#c2c8cb` | Body, property names, pill words | 10.8 |
| `text_muted` | `#8b9296` | Container labels, column headers, hints, inactive pills | 5.8 (5.4 on `section`) |
| `text_disabled` | `#5e666b` | Disabled controls only | 3.1 |
| `room_ink` | `#16181a` / `#eef1f2` | The menu words and the readouts that sit on the room | 12.6 on the day room |
| `hairline` | `#26292c` | The one line: between sections, round the stage, and the rail's seams | |

**No hairline separates panes.** The room does. Inside a card, a hairline separates one
section from the next, and nothing else is ruled: rows are told apart by their own rounded
fills and by the gaps between them.

**Shadows.** `card_shadow` = `0 6 18 rgba(15,15,20,.18)` plus `0 1 2 rgba(15,15,20,.12)` by
day, doubled by night where the room is darker than the card's own shadow; *as built* it is
`0 3 12 black@10%` plus `0 1 2 black@6%` in both rooms: wide and faint, so the shadow fades
round the corner instead of stopping where the straight edge does, which is what made a card
read as a rectangle with its corners cut out;
`float_shadow` = `0 10 30 black@40%` on the rail's flyout, menus, popovers, and the pills that
sit on the room. Nothing else casts.

## 3. Colour

### 3.1 The accent

| Token | Value | Referent | Canonically |
|---|---|---|---|
| `accent` | `#35785e` | **In hand, fronted, or playing**, as a fill | spruce, unchanged |
| `accent_hover` | `#478a70` | The hover step on a fill | unchanged |
| `accent_ink` | `#eef1f2` | The word or glyph on an accent fill | see below |
| `accent_soft` | accent @26% | The selected row's fill, the work-area band | the §6.5 selection fill |

**The accent is spruce, and it is the shipped one.** What changes is not the hue but how much
of it there is: the canonical language spends the accent on a playhead, one filled button and a
tab tick, and this style spends it on every fronted thing, because a filled pill is how this
arrangement says which one of a set is in force. That is a wider job list, not a second colour,
and it is still a closed list: the armed tool, the fronted workspace, mode and comp tab, the
play button, the selection, the playhead, the work area, the keyed stopwatch, Add effect and
Export.

**The one place this parts from the canonical language is the label on that fill.** §7.1 sets
it as `surface_0` on the accent, which measures 3.73:1 and misses the 4.5 text floor. Here the
label is `text_primary`, at 4.63:1. *As built*: the canonical §7.1 now states the label per
shape, `surface_0` under Studio and Desk and `text_primary` under Lantern, read off
`accentInk`; the active pill's label is the same ink.

The rest of the job list is unchanged: **the accent never sets a word on a card and never draws
a stroke.** The two strokes it draws are the selected section's inset ring (§5) and the gizmo
on the picture, and a gizmo is a tool rather than chrome.

### 3.2 States, and `animated` dissolved

The three state colours are the canonical ones, unchanged, and all three clear their floors on
a card:

| Token | Value | Referent | On `card` |
|---|---|---|---|
| `success` | `#5fcfae` | Done as asked: the cache-ready bar, a held cache run, `in use` | 9.6 |
| `warning` | `#dd9a82` | Look here: `missing`, overrun, the held peak | 7.9 |
| `error` | `#d1729c` | Failed: the banner's mark, the clip lamp | 5.8 |

Mint sitting beside spruce is the arrangement Lumit already ships, so it is not this style's
problem to solve. It is covered the way the canonical language covers everything: **never
colour alone**, so a cache tier also differs in height and a badge also differs in its word.

`animated` goes, for the reasons the Desk document gives in its own §3.2: a keyed property is
a **filled stopwatch**, a filled accent circle against a ring, its keys are marks on its lane,
and selection is one idea with one colour. The work-area band takes `accent_soft` and the
focused field takes the ordinary focus ring.

### 3.3 The neutrality zone

Binding. The Viewer's stage is `#121212` with a 12px radius inside the card, and within 48px
of the picture nothing is saturated except overlays on the picture itself: the gizmo, the
selection-name chip, the guides.

*As built*, the Viewer's pane draws no card of its own: its two strips stand as pills on the
room and the stage is a card with the card shadow, so the room shows between the pills and
the picture, the same colour that shows between panels.

**The toolbar never enters the belt in either position** (§12B.1). Left, it is a rail in the
room, outside the Viewer card altogether. Top, it is a strip in the room above every card. The
first draft floated it over the stage, inside the belt, and the belt is the reason that is
withdrawn rather than a matter of taste.

### 3.4 The field

An editable value is **its number in a rounded rectangle**, `field`-filled, 22 tall, at
`ShapeTokens.wellRadius` (7 under Lantern, 2 under Studio and Desk), which is the same token
that draws the colour swatch, the time readout well and the text well;
the number in mono at 11 and the unit rider in `text_muted` at 9.5. It is the canonical inset
well with a smaller radius than a pill: a capsule reads as a thing to press, and a value box is
a thing to type in. Drag to scrub, click to type with the whole value selected, and the focus
ring is a 1.5px `accent` stroke outside it.

**Every property row ends in a reset arrow**, muted, shown on the row whatever the value is,
which is the cheapest undo an editor can offer. It writes the
parameter's declared default as one op, exactly as the effect heading's Reset does for a whole
card.

## 4. The token layer

Structurally the canonical `LumitTheme`; the delta is what it carries:

```
- pub surface_0..4          -> pub room, room_2, card, section, field, raised, float
                               (seven, because the room and the card are two grounds:
                                the same split the Bauhaus document found for surface_0)
- pub accent_hover          -> (removed: hover is a surface step)
- pub animated              -> (removed: 3.2)
- pub success/warning/error -> pub on / attention / fault
+                              pub room_ink: Color32,     // words and readouts on the room
+                              pub card_shadow, float_shadow
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

**The canonical muted family, unchanged** (§6.1 there): footage `#56707f`, sequence `#5a6a8c`,
precomp `#7a5a74`, solid and adjustment `#5c6165`, text `#8c8468`, camera `#806f4a`, with audio
`#46786d` and the rest of the reserved values waiting exactly as they are.

An earlier draft brightened all six on the argument that a muted family disappears on a dark
card. It does not: the card is `surface_1`, which is the surface the family was picked against
in the first place, so the only thing that changed is the shape around it.

Drawn as: the **avatar circle** beside the layer's name in the outline, a 16px disc in the hue
with the type glyph inside it in `card`, and the **bar** on the lane, desaturated over the lane
ground with a solid leading edge, at the content radius of 3. The bar is not a capsule: a
capsule hides where a clip starts and ends, and the start of a bar is the one thing a Timeline
is read for. **Selection still beats every one of these hues**, because a selected bar wears a
2px `accent` ring outside it.

### 6.2 Keyframes and curves

The canonical marks — diamond linear, square hold, hourglass bezier — split at the vertical
centre, at 11px, with their corners eased by 1px. Rest `text_secondary`, selected `accent`.
Curve strokes take the canonical `curve[0..3]` ramp unchanged: `#8ee3ef` / `#aef3e7` /
`#e8a7b4` / `#d8cba0`. Bezier handles are `text_muted` stems with **round dot ends**, `accent`
while grabbed. Graph paper is `sunk` with `hairline` lines and the zero and 100% lines in
`text_muted`.

### 6.3 The cache bar

The canonical two-family, three-height design, drawn as **rounded runs with a 2px gap** between
neighbours so each run reads as its own capsule: `success` for held and `cache_disk` `#5f93b8`
for disk, at full, 70% and 45% height. Uncached draws nothing.

### 6.4 Overrun, markers, waveforms

- **Overrun**: `attention` hatching over a 14% wash with the `hold` tag; unchanged.
- **Markers**: the canonical `marker` grey (`#c4c4c4` on a dark card) as an **8px dot** on the
  ruler's floor with its label in a `raised` pill, one per frame. The colour is unchanged; only
  the flag's shape is, because a triangle exists to stand on a bar and this ruler has none.
- **Waveforms**: the canonical `waveform.rest` `#5d8a96`, filled envelope with the rms core
  solid inside it, and the multiwave stack ranked by value exactly as §6.4 there sets it out.

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

The canonical §12A.6 heights are kept for everything that scrolls, and the card chrome takes
more generous air. *As built* (`DensityTokens.lanternRegular`): **the row pitch is 28**, 26
drawn plus a 2px gap, so no seam is ruled between rows; **the card title line is 36**; the
property row stays the canonical 27, the ruler the derived 47, the **cache bar 4**, the
**navigator band 12** and the **scrollbar 7**. The toolbar is a 44 band carrying 32 pills
as a strip and 44 wide as a rail, the workspace pill 28; the top band is 40
(`DensityTokens.menuBar`, §12B.2); the Timeline's own header line is 28, the 22 tab pill
with a 3px margin either side (§12B.5); the deck is drawn at the title line's 36 (§12B.4), not the
52 first drawn. Compact (`lanternCompact`) is the canonical Compact under the same pitch and
title line.

### 7.2 Type

**Two faces, both SIL OFL, both bundled: Plus Jakarta Sans** for words and **DM Mono** for
every number, timecode, count and readout. Jakarta is a geometric sans with a friendly lowercase
and clean figures; DM Mono is the calmest geometric mono on offer and pairs with it.

| Size | Face | Use |
|---|---|---|
| 9px | DM Mono | Ruler numbers, the frame count's total, tiny readouts |
| 9.5px | Jakarta | Column headers, hints, the status pill, in `text_muted` |
| 10px, caps, +0.14em | Jakarta **medium** | **The card title**, centred, in `text_muted`, with the dot to its left |
| 10.5px | Jakarta | Pill words, section names |
| 11px | Jakarta / DM Mono | Body and property names / values in fields |
| 15px | DM Mono medium | The clock |
| 24px+ | Jakarta | About box, welcome, empty states |

**One kicker, and it is the card title.** A small tracked caps label centred over
each panel with a lit dot beside it, and that one label is worth the shout: it is the only
thing on screen naming what you are looking at, it appears once per card rather than forty
times per screen, and centring it is what stops it competing with the controls that share its
line. Everything else is sentence case with capitals only where English puts them. This
reverses an earlier draft of this document, which banned caps outright.

### 7.3 Hit targets

KD-2 unchanged, and **this style asks for no concession at all**, which the first draft did.
Left, the rail's buttons are 36 in a 44 column with 4 either side, so the aimed axis clears the
chrome floor. Top, the strip keeps the canonical 44 across and 32 down, two more than the
shipped 30, because a strip in the room can afford them where a strip welded between the menu
bar and the dock could not. *As built*, the strip is a 44 band and the pills on it are 32.

## 8. Motion

The canonical §8 in full — ≤150ms, transform and opacity, spring-like ease-out, the three
tiers, playback exempt — **and this is the style the signature interaction was written for**:
the drag ghost lags and settles with its small overshoot; a filled pill **slides** between
positions when a segmented set changes (workspace, mode, comp tab); the tool dock's armed circle
slides likewise. Under *None* every one of them cuts.

## 9. Accessibility

The canonical §9 in full. Contrast figures are in §2.3 (`text_muted` 4.6:1 on `card`); the
accent-filled pill's black word is 15.3:1 in both rooms. **A filled pill is never the
only encoding of a state**: the fronted workspace is also first in reading order and named in
the Window menu; the armed tool is also the one with the index in its tooltip.

## 10. Voice

Canonical §10 unchanged, including the joke. Pill words and section names are sentence case
("Add effect", "Paste at playhead"); the card title is the one thing set in caps, and it is set
from the same string ("EFFECT CONTROLS" is `Effect controls` with a text transform, never a
second string in the table). Tooltips stay one or two words.

## 11. The two rooms

§2's two token sets are the whole difference; no widget branches on the room. Day is the style;
Night is what a grader sets and forgets.

## 12. Shape

Four radii and the pill, each meaning one thing, which is premise 2:

| Radius | What wears it |
|---|---|
| **16** `cardRadius` | The pane itself |
| **12** `sectionRadius` | A band inside a card: the Viewer stage, the lane area, an effect's rows, the specimen |
| **7** `controlRadius` | A value field, a dropdown, an icon button, a segmented track |
| **3** `contentRadius` | Content on a lane: a layer bar, a clip, a cache run |
| **pill** `actionRadius` | An action button, the fronted half of a segment, a chip, a badge |

```
static const lantern = ShapeTokens(
  controlRadius: 7,         // fields, dropdowns, icon buttons
  actionRadius: stadium,    // actions and active segments
  sectionRadius: 12,
  contentRadius: 3,
  floatRadius: 12,          // menus, popovers, the rail's flyout
  cardRadius: 16,
  cardPadding: 0,           // the card's own title line does the inset
  tileGap: 10,
  windowInset: 10,
  cardShadow: [card_shadow],
);
```

plus the two grammar fields both other envisionings want, `labelCase` (caps for the card title
alone) and `strokeWeight` (1.5, round caps), and *as built* `pillInset` (3): the margin
between a pill and the filled state inside it, on every side, with the inner corner the outer
less 3 so the margin stays the same the whole way round. The concentric rule applies to every
filled state inside a pill.

**The radius is the affordance, so mixing them is a defect**, not a matter of taste: a capsule
value box reads as a button and gets clicked instead of typed into, and a rounded-rect action
reads as a field and does not get pressed. That is the one rule a reviewer can fail a
screenshot against fastest.

Round's stated permanent limitation, that stacked tab-bar containers stay square-cornered, is
solved rather than inherited: **a tab group is one card, and its tabs sit as pills on the
card's own title line.** The docking container has the hook. The fronted tab is
`surface_2`-filled with a `text_primary` label and carries no dot; the card's title line
carries one 6px accent dot at its left corner instead.

*As built*, the room is the other way round from the scheme by default: a dark scheme's
cards stand in the day room, a light scheme's in the night room, so the cards are always
the objects and the room the ground. Settings, Appearance, Room offers the style's choice,
Day or Night. Words that stand on the room itself, the menus of the top band, take a room
ink (dark on a light room, light on a dark one) rather than the scheme's text colour, which
is what left the menus unreadable on a light room.

## 12B. The shell, laid out again

Every panel, control and behaviour kept, with the chrome laid out on this style's own grammar.

### 12B.1 The toolbar, and its two positions

**This is the one change in this document that reaches [07-UI-SPEC.md](../07-UI-SPEC.md), and
it is proposed rather than assumed.** §1.7 says the toolbar "cannot be closed, moved, tabbed or
floated" and argues for a horizontal strip on the ground that a full-width 44px band is height
taken from the panels for nothing. That argument is sound and it is only half the picture: a
**rail** spends width instead of height, and which of the two a person would rather spend
depends on their monitor. A 16:9 laptop is short of height; a 21:9 has width to burn.

So the amendment is one word. "Cannot be closed, tabbed or floated" stands; **moved** becomes a
setting:

> **Settings, Appearance, Toolbar position: Top or Left.** Machine-local, like UI scale. It is
> not workspace state, because it is a fact about the person rather than about an arrangement,
> and switching workspace must not move the tools out from under the hand.

Both positions carry **the same toolbar**: every group from §1.7 in the same order, the same
flyouts, the same chords, the same cycling, the same disabled-and-labelled treatment for a tool
that is not built, and the same tool-options area. Nothing is added or dropped by the choice,
which is the rule the Viewer bars' split, top and bottom arrangements already follow (§2.2).

**The row is three pills, not eighteen buttons.** Whichever position is set, the toolbar is:

1. **The tool pill.** Every tool from §1.7 in one capsule, the seams inside it, the snapping
   switch at its end. A tool button is bare inside the pill and only the armed one is filled,
   so the row reads as one object with one thing lit rather than as a field of buttons.
2. **The tool options pill**, beside it, carrying the armed tool's own settings and nothing
   else. It is drawn deaf and labelled while the armed tool has no options (Selection draws
   nothing), rather than vanishing, so the row does not change width as tools are cycled.
3. **The workspace pill**, at the far end of the same row.

- **Top.** A strip in the room under the top band, spanning the window, above every card. The
  three pills read left, left, right. 44 across and 32 down: the shipped strip is 30, and a
  strip that floats in the room rather than being welded between the menu bar and the dock can
  afford the two extra pixels. *As built*, the strip is a 44 band carrying 32 pills, and the
  workspace pill is 28.
- **Left.** A rail in the room down the left edge, buttons 36 in a 44 column, the groups in the
  same order with a seam after the six simple tools, the snapping switch at the foot, and the
  flyout opening to the **right** of the rail. This restores the full 44x44 target §1.7
  explicitly gives up, at the cost of 44px of width. The tool pill runs down the rail.

**Under Left the options pill and the workspace pill move up onto the top band** (§12B.2),
and the strip above the dock is not mounted. *As built*: the 40 band carries the two pills
beside the menus, the options pill standing there in its own right rather than opening from
the armed tool; the options pill does not sit under the rail at the foot. A 44px column has
no room for a number field or six words, and the band already has. That is a placement, not
a change in what either carries, and it is the only asymmetry in the setting. Top is the
default under every shape, Lantern included: changing the shape must not silently change the
toolbar's position.

The armed tool is an accent-filled button with a black glyph in either position. *As built*,
the fill is a 26 disc centred in a bare cell and the glyph is `surface_0`, in both positions.
**The toolbar lives in the room and never over the picture**, which is what withdrew the first draft's
floating dock: a dock over the stage sat inside the Viewer's 48px neutrality belt (§3.3), and
an accent-filled button is exactly the thing the belt exists to keep out.

### 12B.2 The top band

The room's own 44px band, not a card: the mark; the nine menus as sentence-case words in
`room_ink` (the system bar on macOS); a **workspace pill** — a `card`-filled capsule with the
fronted name as an accent-filled pill; the **command palette** as a capsule at the right.

*As built*, the band is 40 (`DensityTokens.menuBar`). The workspace pill's labels are the
body face in sentence case, and the fronted name's fill is inset by `pillInset` (3) on every
side, its corner the pill's less 3, the concentric rule of §12. Under Top the pill rides the
toolbar strip; under Left it rides this band with the tool options pill (§12B.1).

### 12B.3 Cards

Each pane is a card with a header line at the density token's 36
(`DensityTokens.lanternRegular.headerStrip`): the **header dot** (6px `accent`, decorative,
never a status light) at its left corner, the title centred as the kicker in `text_primary`
(`kickerOn`), and the pane's own controls at the right as pills and circles. A tab group's
header carries pill tabs, the fronted one `surface_2`-filled with a `text_primary` label and
no dot of its own (§12); *as built*, the fill stands off the tab's slot by `pillInset` (3) on
every side, its corner the stadium less 3. A pane standing alone carries the same line with the dot and its
name centred; the Viewer and the Timeline draw their own header and get no dock title line.
Docking is unchanged.

### 12B.4 The Viewer

- **The bar above the stage**, a 28px pill with semicircular ends whose picker faces sit 4
  in from every edge, the inner corner the outer less that 4 (*as built*: the concentric
  rule, so no curve runs inside another), holds the ways of looking as pills and circles:
  magnification,
  resolution, the channel mark, board, view menu, 3D view, exposure, the snapshot pair, the
  colour pipeline. The degradation reading sits at the right in `text_muted`.
- **The stage** is a `#121212` sub-card with 12px corners; the picture centred in it above the
  dock's band; the selection name as an accent capsule top-left; the at-effect chip as a
  `float` capsule top-right.
- **The deck**, drawn at the header strip's 36 (*as built*; 56 was the first drawing): the
  clock at 15px with the frame count beside it; **the transport as one `surface_3` pill on
  the `surface_2` strip** (a `surface_2` pill would vanish against its ground), 26 tall *as
  built* so it sits in the deck with air above and below, with a 34px
  accent play capsule at its centre and the four marks either side; preview mode and
  quality as two separate pickers; the cache-ready bar as a 54×6 rounded meter with its
  figure. Loop mode and audio mute arrive with the engine step that carries them. §9's
  Preview panel, docked by the shell, and one of the four Viewer bars arrangements (07 §2.2).

### 12B.5 The Timeline

- Header: title and dot; the comp tabs as a `well`-tracked segmented pill; the Layers / Graph
  segmented pill and the accent-filled **Export** at the right.
- The outline's chrome row: the layer-search capsule, shy and motion blur as small circles, and
  the three group toggles as ghost pills at the right.
- Column headers 9.5px muted; rows 26 with a 10px radius and a 2px gap; the selected row filled
  `accent_soft`; switches and modes as bare glyphs; pickers as 20px capsules.
- The lane area is a `card_2` sub-card (*as built*: the ruler and the rows clipped together
  to the section radius); the ruler's scale on its upper half; the work-area
  capsule and round handles; rounded cache runs; dot markers; the pin playhead. Bars are
  capsules at 55% of the hue; outside the work area the lane ground dims.
- The foot: the ease strip and its pills, then the measuring clock; on the lane side the zoom
  as a track with a white dot thumb between the two landscape glyphs, the magnet, the scrollbar.

*As built*: one 36 header line carries the title, the composition tabs, the timecode and
frame count, the Layers / Graph pill and Export. A bar is not a capsule (§6.1): it is an 18
tall rectangle with 3px corners in its 28 row, the hue at 55%, and it always carries the
layer's name, by request. The lanes stand on a `surface_2` band; a selected row takes
`accent_soft` behind its bar and the bar wears the 2px accent ring outside it.

### 12B.6 The inspector

One list, each section a **sub-card** (`section`, 12px): the heading with twirl, the house
**toggle switch** for enable, scaled into the 18px enable slot and lit in the house toggle's
own on colour, the scheme's `animated`, as it draws everywhere; the name, the cost and the ×; rows of stopwatch circle,
navigator, name, field and reset arrow; the selected section wears the accent inset ring. Add
effect is the accent-filled pill at the foot, Save preset a ghost one.

**The range slider is a setting, and it is the second amendment this document proposes.**
A parameter whose whole meaning lives inside a range can draw a **track and thumb beside its
number**, which is a second grip on one value rather than a second control. 07 §6 already
specifies that row for a `Slider` parameter. What is new here is that it becomes optional:

> **Settings, Appearance, Range sliders: on or off.** Machine-local. Off, a ranged parameter
> draws its number alone and the row is exactly as wide as every other row. On, the track sits
> beside the number.

It belongs to **every style, not to Lantern**. Someone who types values and scrubs will never
touch a track and is paying for it on every ranged row in the stack; someone shaping a wipe or
a mix wants it under the hand. Neither is wrong, so neither is the default for the other.
*As built*, on is the default under every shape: it is the row the canonical spec draws and
what every install had, and changing the shape must not silently change it.

### 12B.7 The readout pill

The status line keeps **Lumit's own order** (`status_line_frb.dart`): whether the document is
saved, the cache meters with their exact figures, the measuring clock, the latest notice with
its close mark, then the background jobs with their progress and a Cancel that works from
anywhere. **The only thing this style changes is that each of those takes a bubble of its
own**, sitting on the room rather than in a strip welded under the dock. Nothing is added,
nothing is dropped, and nothing is reordered.

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

- **Does a card arrangement cost too much screen?** Ten pixels of gap between panes, plus the
  window inset, is roughly 40 pixels of width and 30 of height a tiled dock keeps. On a laptop
  that is most of a layer row. Worth measuring against the dock's own minimum widths before
  this is built.
- **The light room by day** beside a graded picture — the same experiment all three
  envisionings need, and here the Viewer card contains it best.
- **Does a dot marker survive on a busy ruler?** The canonical triangle was chosen to stand on
  the cache bar; a dot may read as a cache run's end.
- **Name.** See the folder README's naming proposal; "Lantern" is a working title.
