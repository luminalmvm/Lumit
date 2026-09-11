# Lumit design language — the Bauhaus envisioning

**Status: speculative. Not canonical, not binding on any code.** This is a complete
alternative to [15-DESIGN.md](../15-DESIGN.md), written to the same section skeleton so the
two can be read side by side, one section against its counterpart. Where a section number
here matches one there, it is answering the same question with a different answer. Where this
document is silent, the canonical one still holds.

It re-derives Lumit's colour, type, density, motion and voice from the **Bauhaus** — Itten's
and Kandinsky's colour-and-form teaching, Bayer's single alphabet, Moholy-Nagy's constructed
page, and the workshop premise that a tool should show how it is made. Panel inventory,
docking and interaction flows are unchanged and still live in
[07-UI-SPEC.md](../07-UI-SPEC.md); terminology still follows
[01-GLOSSARY.md](../01-GLOSSARY.md) exactly.

RFC-2119 keywords are used with their usual force *within this document's own world*.

Visual reference: [mockups/shell.html](mockups/shell.html), which draws the same shell under
the canonical style, the Rams one, and this one from a single markup.

---

## 0. The five premises

The Bauhaus is not a palette, and a Bauhaus-styled application that is only primaries on white
is fancy dress. Five teachable premises do the work here, and each resolves to something a
reviewer can fail a screenshot against.

1. **Form and colour are one language, not two.** Itten's colour course and Kandinsky's
   form course were taught in the same building to the same students, and Kandinsky's 1923
   questionnaire asked them to assign a primary to each of the three primary forms. The
   answers — **triangle yellow, square red, circle blue** — are this style's spine, and §3.2
   turns them into the keyframe code, which is the single best idea in this document.
2. **One alphabet.** Bayer's *universal* dropped capitals outright, on an efficiency
   argument: we speak one sound, why write it two ways. This interface is **entirely
   lowercase** (§7.2) and that is the change you notice first.
3. **The rule is structure.** A Bauhaus page is organised by heavy rules and flush-left
   blocks hanging from them, not by boxes around things. Panels here are **plates on a grid**,
   headed by a rule (§2, §7.3), and the interface has far fewer boxes than the canonical one.
4. **Colour is precious and therefore rationed.** Three primaries, each with exactly one
   referent, plus black, white and a grey ramp (§3). The Bauhaus is loud in reproduction and
   disciplined in fact: a Bayer poster is mostly paper.
5. **Truth to the tool.** The workshop premise: a made thing shows its making. Applied here,
   that is the canonical doc's own instinct — the cache bar that admits what resolution it
   holds, the boot log that names the module that is slow, the disabled control that stays
   drawn and legible rather than hiding.

**The test.** A screen passes when its structure is legible from across the room — the plates,
the rules, the one red thing — and every colour on it can be named with one word.

## 1. Relationship to the canonical language

### 1.1 Kept, unchanged

- **Semantic tokens only**, hex confined to the theme module.
- **The Viewer neutrality zone** (§3.3) — binding, and the one place this style yields
  entirely. The picture is not a composition we are allowed to arrange.
- **No punishment UI**, **never colour alone**, **the user controls tempo**.
- **KD-2's hit-target compensation**, unchanged.
- **AccessKit from day one**, keyboard operability, visible focus.
- **British English, no exclamation marks, no emoji.**
- Every behaviour in [07-UI-SPEC.md](../07-UI-SPEC.md).

### 1.2 Overturned, with the reason

| Canonical rule | This style | Why |
|---|---|---|
| Dark-first near-black shell | **Paper-first.** Light plates on a black grid, with a night room that inverts the grid rather than the idea (§2, §11) | The Bauhaus is high contrast, not low light. A plate of paper with a black rule across it is the form the whole school worked in |
| Hairlines separate panels; no gaps between docked panels | **A 3px black grid runs between every plate** (§2.4) | Premise 3. The grid is the structure and should be visible; a 1px hairline is a hedge |
| Input wells are inset recesses | **A field is a baseline rule under it**, 2px, black at rest and red while focused (§3.4) | Premise 3, and Bayer's own forms. A recess is a simulated hollow; a rule is a mark |
| `accent` (spruce) + `animated` (amber), two closed job lists | **Three primaries, three referents** (§3.1) | Premise 4 |
| Interpolation is shape; selection and key-state are colour | **Interpolation is shape *and* colour, bound by Kandinsky's pairing; key-state is not a colour at all** (§3.2) | Premise 1. This is the idea the whole style is worth reading for |
| Radii `{4, 8, 16, full}` | **`{0, full}`** — a thing is a rectangle or it is a circle (§12) | Premise 1. There is no third primitive |
| Two faces, mono-caps kickers | **Jost\* and DM Mono, all lowercase, no capitals anywhere in chrome** (§7.2) | Premise 2 |
| Micro-motion: spring-like ease-out, one signature overshoot | **Linear, constant velocity, no easing** (§8) | Moholy-Nagy's Light-Space Modulator is a machine turning at a rate. A spring is a simulated material |
| `success` green | **Removed.** There is no success colour (§3.1) | Green is not a primary, and "it worked" does not need to be said in colour. The line reads *finished* |
| `error` fig pink, "never a harsh red" | **Black type on a yellow bar** (§3.1) | Red is spent on *now*. Black on yellow is the oldest attention pairing there is and it is not a shout, it is a sign |
| Six muted layer-type colours, "muted siblings so a full timeline is not carnival" | **Six hues from Itten's twelve-part circle at one value and one chroma** (§6.1) | The fear is right; Itten's own harmony rule is the answer to it, and it answers it with colour rather than by draining it |

### 1.3 The honest cost

A light, high-contrast shell beside a picture raises the room's ambient level and the eye
adapts upward, which makes the picture read darker and flatter than it is. **This is a real
cost and this document does not pretend otherwise.** The neutrality zone (§3.3) contains it,
and the night room (§11) exists for the same reason a colourist's suite is dim. The plain
recommendation: **the paper room for editing and design work, the night room for grading**,
and the Viewer's 48px neutral belt binding in both.

## 2. Plates on a grid

### 2.1 Paper room (the default)

| Token | Value | Role |
|---|---|---|
| `grid` | `#121211` | **The ground between plates**, and the window inset. Not a panel colour — it is the structure |
| `surface_1` | `#f4f2ec` | The plate: panel bodies |
| `surface_2` | `#e6e2d8` | Faint strips: headers, bottom bars, layer rows |
| `surface_3` | `#ffffff` | Floating and hover: menus, popovers, hovered rows |
| `surface_4` | `#d5d0c2` | Pressed fills, chips, scrollbar thumbs |
| `well` | `#ddd8cb` | The timeline well, graph paper — the ground *inside* a plate |
| `viewer_surround` | `#7f7f7f` | Strictly neutral, unchanged in both rooms |

### 2.2 Night room

| Token | Value | Role |
|---|---|---|
| `grid` | `#cfccc3` | The grid drawn light on dark — the structure survives the inversion |
| `surface_1` | `#1a1a19` | |
| `surface_2` | `#232320` | |
| `surface_3` | `#2c2c29` | |
| `surface_4` | `#3a3a36` | |
| `well` | `#111110` | |
| `viewer_surround` | `#7f7f7f` | unchanged |

### 2.3 Text and rules

| Token | Paper | Night | Role | Contrast on `surface_1` |
|---|---|---|---|---|
| `text_primary` | `#121211` | `#f4f2ec` | Headings, values, primary copy | 16.7 / 15.6 |
| `text_secondary` | `#3d3b36` | `#c8c5bc` | Body copy, property names | 10.0 / 10.1 |
| `text_muted` | `#6f6c64` | `#8f8c84` | Labels, hints, inactive words | 4.7 / 5.2 |
| `text_disabled` | `#8a8679` | `#6b6862` | Disabled controls only | 3.3 / 3.1 |
| `rule` | `#121211` | `#f4f2ec` | **The 2px structural rule.** Under a container label, under a field, under the fronted tab | — |
| `hairline` | `#cfcabc` | `#3a3a36` | 1px, used only *inside* a plate where a rule would be too loud: between rows in a long list | — |

**Two weights of line and no more.** A **rule** is 2px and black (or paper, at night); it is
structure and it is always horizontal. A **hairline** is 1px and grey; it separates rows in a
list and does nothing else. A box drawn around something is neither, and there are almost
none — the exceptions are the Export dialog's group fences and a badge, both named in §12A.

### 2.4 The grid

Plates are separated from each other, and from the window edge, by **3px of `grid`**. That is
the whole elevation system: there is **no shadow anywhere in the application**, including on
menus and modals, which instead sit on `surface_3` with a 2px `rule` around them. A menu is
not floating above the panel; it is a smaller plate laid on top, and it should look like one.

Structurally this is the canonical Round shape's `tile_gap` and `window_inset` with a radius
of zero and no card shadow — which is why §12 says implementing it is mostly a `ShapeTokens`
entry rather than new machinery.

## 3. Three colours

### 3.1 One referent each

| Token | Paper | Night | Referent | Reads as |
|---|---|---|---|---|
| `red` | `#d3241e` | `#e8493d` | **now, and in hand** — the playhead, the selection edge, the single filled action, the focus rule under a field, the fronted tab's rule | square · static · a position |
| `yellow` | `#f2c200` | `#f6cf2e` | **look here** — the work-area band, overrun, missing footage, a fault banner's ground | triangle · directed · attention |
| `blue` | `#17479e` | `#5b8ee0` | **content held** — cache runs, curves, waveforms, wires and sockets, the ways data is drawn | circle · continuous · a quantity |

There is no fourth. Specifically:

- **There is no success colour.** An export that finished says `finished` in `text_primary`
  and stops saying anything else. Green is not a primary; more to the point, a thing that went
  as asked does not need a lamp.
- **There is no error colour, because red is taken.** A fault is **`text_primary` on a
  `yellow` bar** — the banner strip the canonical §10 already specifies, with its ground
  changed. Black on yellow is 11.2:1 in the paper room and 12.4:1 at night, which is the
  highest-contrast pairing anywhere in this document, and it is a *sign*, not a shout. This
  satisfies the no-punishment rule better than a pink does: nothing is alarming, and it is
  impossible to miss.
- **Yellow cannot mark; yellow can only ground.** At 1.5:1 against paper, a yellow 3px mark on
  a plate is invisible. Hence §3.2's keyline rule, and hence yellow's jobs above are all
  *fields* — a band, a hatch, a bar — never a small mark on its own.

**Every coloured mark carries a 1px `rule` keyline around it.** Klee outlines, Bayer outlines,
and the reason is exactly this one: flat colour on a light ground needs an edge to hold its
shape. It also makes every mark legible without colour vision, which is §9's requirement and
this style's answer to it.

### 3.2 The colour–form code

**This is the idea.** Kandinsky's students paired the triangle with yellow, the square with
red and the circle with blue. A compositor already encodes keyframe interpolation by shape.
The two codes are the same code:

| Interpolation | Form | Colour | Why the pairing is right, not merely available |
|---|---|---|---|
| **Hold** | square | `red` | A held value is static, weighted, fixed at a point — Kandinsky's square exactly |
| **Linear** | triangle | `yellow` | A constant rate is directed motion; the triangle points, and the mark's two halves point in and out |
| **Bezier** | circle | `blue` | A smooth curve is continuous and unbounded; the circle is the form with no corner in it |

The canonical splitting rule is kept whole and improves under the pairing: **every mark is
split at its vertical centre, the left half answering for the incoming side and the right half
for the outgoing**, so a key that eases in and holds out is literally half a blue circle
beside half a red square. That is more readable than the canonical hourglass-versus-diamond
set, not less, and it is readable at 11px because each half is a distinct silhouette before it
is a distinct colour.

**Consequently there is no `animated` token.** A property that is keyed has keys on its lane,
and the keys say everything. The stopwatch is a **filled black disc when on and a ring when
off** — form again — and the focused value field takes the `red` focus rule like every other
control. The canonical doc's own closing open question suspected this token of doing too much;
this style resolves it by giving its work to the marks that were already there.

### 3.3 The neutrality zone

**Binding, and this style's one unconditional surrender.** Within 48px of the Viewer image
area the interface is strictly neutral: `viewer_surround`, greys, `text_secondary` and
`text_muted` only. No primary, no rule in colour, no plate edge in anything but grey.

Overlays on the picture itself — transform gizmos, mask paths, guides — are tools and may be
coloured, with the canonical user-toggleable "neutral handles" option intact. The Viewer's two
bars sit outside the zone and follow §3.1 like any other chrome.

The paper room's plates stop at the belt and the belt is grey. On a bright plate that step is
visible and deliberate: it is the mount board around a print.

### 3.4 A field is a rule

The canonical language says an editable value is *mono text in an inset well*, and the well is
what says "editable". This style says the same thing with a line:

> **A field is its value with a 2px `rule` under it**, running the field's full width. At
> rest the rule is `text_primary`; while the field is focused the rule is `red`; while its
> value is being dragged the rule is `red` and the value goes `text_primary`; when the field
> is disabled the rule is `hairline`.

No box, no recess, no fill. It is Bayer's form field, it costs one line instead of four, and
it is the single largest reduction in drawn elements anywhere in this document — the canonical
Timeline draws a well around the timecode, the frame count, the search, every matte, blend and
parent picker and every property value, and all of them become a rule.

## 4. The token layer

Structurally identical to the canonical §4.1 — a struct, one per room, passed by reference,
hex confined to the theme module. The delta:

```
- pub surface_0: Color32,       → pub grid: Color32,   // the ground *between* plates
+                                 pub well: Color32,   // the ground *inside* a plate
      // the canonical surface_0 carries both jobs; under this style they are
      // black and near-paper respectively, so they must split. See §2.1.
- pub accent / accent_hover     → pub red: Color32,
- pub animated                  → (removed: §3.2)
- pub success                   → (removed: §3.1)
- pub warning                   → pub yellow: Color32,
- pub error                     → (removed: a fault is text_primary on yellow — §3.1)
- pub fill_tonal                → (removed: informational chips are rule-bounded)
+                                 pub blue: Color32,
+                                 pub rule: Color32,   // the 2px structural line
```

**`surface_0` splitting into `grid` and `well` is the one genuinely structural finding in this
document**, and it is worth recording even if this style is never built: the canonical token
carries two jobs that only happen to want the same colour because the canonical shell is dark
throughout. Any light-shelled scheme hits it.

The `.lumtheme` format, the custom-theme machinery and the seven shipped schemes are all
unaffected — this is two more schemes beside them.

## 5. Iconography

The set is redrawn from **three primitives: circle, square, triangle**, on the canonical
16-unit grid.

- **2px stroke, butt caps, one weight.** No round caps: a Bauhaus mark is cut, not drawn.
- **Geometric construction only.** Every arc is a circle or a quarter of one; every angle is
  0°, 45° or 90°. A glyph that needs a 37° line is the wrong glyph.
- **Monochrome via `currentColor`**, exactly as canonically ruled: `text_secondary` at rest,
  `text_primary` on hover, `red` when active.
- **The Channels indicator keeps its own colour** — it is the one glyph whose subject is
  colour. Its three circles are the three primaries' own construction and it is, by accident,
  already the most Bauhaus object in the application.
- **A bypassed effect draws as a dashed outline**, canonically ruled and kept.
- **No emoji, no bare symbol characters, ever.** Kept verbatim.

The canonical rule that *a glyph has to be readable as the thing it names* is the constraint
that will fight the geometric discipline hardest, and where the two collide the canonical rule
wins: a mark that reads as something else is a defect however pure its construction.

**Chrome labels** keep the canonical three-way setting (words / icons / icons everywhere),
with words the default and the tooltip carrying the word in every mode.

## 6. Editor semantics

### 6.1 Layer types — Itten's circle

The canonical language drains the layer family so a full timeline is not a carnival. That fear
is correct and Itten's own harmony rule answers it without draining anything: **take six hues
from the twelve-part colour circle, hold value and chroma constant across all six, and the set
harmonises by construction.**

| Layer type | Paper | Night | Itten position |
|---|---|---|---|
| Footage | `#2f6ea3` | `#5e9ac9` | blue |
| Sequence | `#4d5fa8` | `#7d8cd0` | blue-violet |
| Precomp | `#8a4e96` | `#ab77b6` | violet |
| Solid / Adjustment | `#6f6f6a` | `#94948d` | neutral (no hue — a solid has none) |
| Text | `#a8621d` | `#cd8a45` | orange |
| Camera | `#8e7a12` | `#b8a034` | yellow-orange |

Drawn as the canonical 3px tab on the left edge of the layer's bar plus a ~10% tint over its
fill, with the type glyph tinted to match. Reserved, unchanged in intent: Shape (blue-green),
Null (outline only, a hollow bar), Audio (green), Light (yellow).

**Selection still beats every one of them**, because a selection is not a tint: it is a **2px
`red` edge on all four sides of the bar**, which no 3px tab and no 10% wash can be mistaken
for.

### 6.2 Keyframes and curves

- §3.2 is the whole of the keyframe design. Marks are 11px point to point in the lanes and in
  the graph, each with its 1px keyline, each split at the vertical centre.
- **A selected key inverts**: its fill becomes `rule` and its keyline becomes the colour it
  had. The shape is unchanged, so what is selected and what its interpolation is are two facts
  read from one mark without either hiding the other.
- Curve strokes take four steps around the same circle, one value, one chroma:
  `#2f6ea3` / `#2a8a6e` / `#a8621d` / `#8a4e96` (paper), one step lighter at night. Dimension
  order x, y, z, w; single-dimension properties take the first.
- Bezier handles: `text_muted` stems, `red` while grabbed.
- **Graph paper is ruled, not dotted**: `well` ground, `hairline` minor lines, a 2px `rule` at
  zero and at 100%. A graph's two structural lines are structure and take the structural
  weight.

### 6.3 The cache bar

The canonical design is kept whole, including the finding that drove it — brightness alone was
illegible on a 3px stripe, so tiers differ in **both** value and fill height (full / 70% /
45%). The colours move onto this palette:

| State | Colour | Height |
|---|---|---|
| Held, this resolution | `blue` | full |
| Held, coarser | `blue` at 55% | 70% / 45% |
| On disk, this resolution | `blue` at 35%, **hatched at 45°** | full |
| On disk, coarser | as above | 70% / 45% |
| Uncached | nothing drawn | — |

The disk tier is distinguished by **hatching rather than by a second hue**, because §3.1
rations colour and because a hatch is the honest mark for "present but not immediate" — it is
the same device §6.4 uses for overrun, meaning the same thing: *this is real, with a
qualification*. Uncached stays neutral and undrawn, per the no-punishment rule.

### 6.4 Overrun, markers, waveforms

- **Overrun**: `yellow` 45° hatching, 1px lines at 4px pitch, over a ~14% wash, with the
  lowercase `hold` tag and the 1px exhaustion tick. Canonically ruled, kept, recoloured.
- **Markers**: a plain grey flag — the canonical ruling, kept for the canonical reason, and
  more necessary here than there: with three primaries all spoken for, a marker that took one
  would be claiming to mean something it does not. `#565656` on paper, `#c4c4c4` at night.
  The upward triangle standing on the cache bar with its `surface_4` backdrop pill is kept,
  and the triangle is the one place a triangle is not yellow — which is stated here rather
  than discovered later.
- **Waveforms** are content and take `blue`: the envelope at 80% with the rms core solid
  inside it. The multiwave stack ranks its three bands **by value within `blue`**, not by hue
  — `#8fb0d8` / `#3d6fae` / `#102f66` on paper, running the other way at night — exactly the
  canonical ranking argument, which is right.

### 6.5 Selection, focus, drop targets

- **Selection**: a **2px `red` edge** with no fill. The canonical 16% accent wash goes: a wash
  under a selection is a second statement, and on a paper plate it tints the content. An edge
  is the mark.
- **Focus**: a 2px `red` **rule under the control** where the control has a baseline (fields,
  wells, rows), and a 2px `red` edge where it does not (buttons, chips, glyph toggles). One
  colour, two placements, both structural.
- **Drop targets**: a 2px dashed `red` edge with no fill; an insertion caret is a 2px `red`
  line. Dock previews the same at panel scale.
- **The playhead** is a 1px `red` line with an **8×8 `red` square head standing on its
  corner** at the top of the ruler. Square, because the playhead is a position and §3.2 says
  positions are squares; the canonical downward triangle would be claiming to be yellow. The
  ≥24px grab target is unchanged.

## 7. Density, type, and the page

### 7.1 The grid is 4, and the plate hangs from its rule

Spacing is the canonical 4/8/12/16 scale, with one addition: the 3px `grid` between plates
(§2.4). Heights are the canonical §12A.6 table, unchanged — this style is not making the
module argument the Rams envisioning makes, and the approved artboard heights are a real
survey that would be thrown away for nothing.

What changes is **how a container announces itself**:

> **A container label is flush left, lowercase, and a 2px `rule` runs from the end of the word
> to the far edge of the container**, on the label's own baseline. The label sits in the rule,
> not above a box.

That is the Bauhaus page in one element, it replaces the canonical kicker *and* the panel
header strip's bottom hairline with a single mark, and it makes every panel in the application
read as a titled plate from across the room.

### 7.2 Type

**Two faces, both SIL OFL, both bundled:**

- **Jost\*** for all interface text — Owen Earl's revival of Renner's *Futura*, which is the
  geometric sans the Bauhaus circle actually produced. Its lowercase is built from the circle
  and the line, which is the same construction as §5's icons.
- **DM Mono** for numbers: timecode, frame numbers, percentages, property values, durations,
  counts. Geometric, calm, tabular. **The canonical mono-for-numbers rule is kept absolutely**
  — it is a functional rule and premise 5 has no quarrel with it.

Herbert Bayer's *universal* (1925) is the ancestor and is not available as a usable text face;
Jost\* is the shipping answer and the resemblance is real.

**There are no capitals in chrome.** Not in labels, not in tabs, not in dialog titles, not in
buttons, not in menu items, not in the status bar. This is Bayer's rule and his argument for
it was an efficiency argument, which is the right kind of argument for a tool. Two carve-outs,
both for things that are not the application speaking:

1. **Content the user typed** — layer names, file names, expression source — is theirs and is
   shown as they wrote it.
2. **Proper nouns inside a sentence** and **the product name** keep their capitals in prose
   (the about box, an error banner's sentence). A label is not prose.

| Size | Face | Use |
|---|---|---|
| 9px, +0.06em | Jost\* | **The container label** (§7.1), and the attribution line |
| 9px | DM Mono | Ruler numbers, per-effect cost readouts, the status bar |
| 10px | Jost\* | Secondary notes, field captions, layer bar labels, in-row picker labels |
| 10px | DM Mono | Units beside a value, outline-row readouts |
| 11px | Jost\* | Panel body copy, property names, menus, buttons |
| 11px | DM Mono | Property values, timecode fields, frame numbers, speed percentages |
| 13px | Jost\* Medium | Dialog body emphasis |
| 24px+ | Jost\* | About box, welcome screen, empty states — outside chrome |

**Weight is a Bauhaus tool and this style uses it**, sparingly and structurally: the one
Medium in the table, and nothing else in chrome is anything but Regular.

### 7.3 Hit targets

Unchanged from KD-2 in every particular: ≥44px for toolbar, transport, dialog and Viewer-bar
controls; ≥24px visual with ≥32px slop on dense surfaces; the tool strip's 44-across-30-down
compromise intact. A style argument does not get to move an accessibility floor.

## 8. Motion

**Machine motion: linear, constant velocity, no easing anywhere in chrome.** Moholy-Nagy's
Light-Space Modulator turns at a rate; Schlemmer's dancers move in straight lines and arcs.
A spring simulates a material the interface does not have, and premise 5 is against it.

- Micro-motion budget **≤120ms**, linear, transform and opacity only.
- **The canonical signature drag ghost is withdrawn** — no lag, no overshoot. A dragged clip
  travels with the cursor and stops.
- **Rotation is the permitted motion.** Where something must indicate work in progress and a
  real proportion is unavailable, it turns at a constant rate — which is the one honest
  indeterminate indicator, and it is a circle, which is the form for continuous.
- Timeline zoom tracks the wheel 1:1; nothing auto-advances; no scroll hijack.
- The three animation tiers (All / Minimal / None) are kept, with the OS reduced-motion
  request mapping onto None, and every meaning carried by motion also carried by colour or
  text at every tier.
- **Playback is not motion** and is exempt, canonically and here.

## 9. Accessibility

The canonical §9 is kept in full. Three things this style adds or strengthens:

- **The keyline (§3.1) is an accessibility device before it is a stylistic one.** Every
  coloured mark has a 1px black edge, so every mark holds its silhouette without colour
  vision, on any ground, at 3px.
- **The colour–form code (§3.2) is redundant by construction.** Interpolation is readable from
  shape alone, from colour alone, and from the two together — which is a stronger position
  than the canonical set, where the amber selection state is colour-only.
- **Contrast floors are re-run against both rooms** and §2.3's table carries the figures.
  `text_muted` clears 4.5:1 in both. The one figure to watch is `red` on paper at 4.6:1 —
  fine for text and marks, and it is why `red` is never asked to carry a label at 9px.

The yellow fault banner at 11.2:1 is, deliberately, the most legible thing in the
application. The thing that went wrong should be.

## 10. Voice

The canonical §10 is kept: British English, calm, no exclamation marks, no emoji, glossary-exact
names, banner errors rather than modal storms, factual progress copy, soft empty states,
i18n through the string table.

Changes:

- **Lowercase throughout** (§7.2), with the two carve-outs named there.
- **The one rationed joke is kept**, and it is the right instinct: a single line in the about
  box and nowhere else. The Bauhaus was not humourless — Schlemmer's stage was a joke that
  took itself entirely seriously — but it was disciplined about where the joke went, which is
  exactly what "one line in the about box" is.
- **A fault banner is one sentence on yellow with one action**, and the sentence keeps its
  sentence capital because it is prose: *"Couldn't decode clip 'render_04.mp4' — the file may
  have moved. Relink…"*

## 11. The two rooms

§2's two token sets are the whole of the difference: no widget branches on the room, no layout
changes, no glyph is redrawn. The grid inverts with the room (black grid on paper, paper grid
at night) because the grid is structure and structure must stay visible; the surround does
not move, because it is an instrument.

**Which room is the default.** The paper room is the one this style is *for* — it is the
Bayer poster, and it is the reason to build this at all. §1.3 states the cost honestly and the
recommendation follows from it: paper for editing and design, night for grading, and a person
who grades all day should set night and never think about it again.

## 12. Shape

There is one shape and it is square. Radii are **0** everywhere, with **full** reserved for
controls that are genuinely circular: the stopwatch, a colour swatch, a radio, the channel
indicator, a rotary control. Nothing is a rounded rectangle, because a rounded rectangle is
neither of the two forms this style has.

The canonical Round shape (§12, §12.1) is withdrawn — stadium controls, filled pills, bigger
cards and the header dot all go — but the *mechanism* behind it is exactly what this style
needs. Implemented, this is a `ShapeTokens` entry:

```
static const bauhaus = ShapeTokens(
  controlRadius: 0, floatRadius: 0, cardRadius: 0,
  cardPadding: 0, tileGap: 3, windowInset: 3, cardShadow: [],
);
```

plus two new grammar fields the canonical struct does not have and both styles in this folder
want: a **label case** and a **stroke weight**.

## 12A. The resting state

Every *behaviour* ruling in the canonical §12A survives untouched — the fixed column edges,
Tab hopping pre-selected values, the reserved keyframe-navigation slot, the matte column's
conditional width, the vector-pair linking arithmetic, the per-format capability table, the
degradation ladder, the minimum-width enforcement, the Viewer bar's shedding order, the
metrics tables. What follows is only what is drawn differently.

### 12A.1 Timeline

- **The panel's label and its rule** run across the header strip; the composition tabs sit in
  the rule, the fronted one marked by a `red` 2px rule under its word (§7.1, §3.1). The one
  filled `export` action at the far right is a `red` fill with a paper label.
- **Every well becomes a rule** (§3.4): the timecode, the frame count, the layer search, the
  matte, blend and parent pickers. The Timeline loses roughly a dozen drawn boxes per screen
  and reads, as a result, considerably quieter than the canonical one despite the primaries.
- **The ruler is double height and reads as one band**, canonically ruled and kept, with the
  labelled tick crossing its waist 7px each way.
- **The work area is a `yellow` band** from the ruler's handles down through the lanes, drawn
  behind the cache bar, with its two drag handles running the ruler's full height as solid
  tabs in the band's own colour. Canonically ruled; recoloured; the 4px-wide tab with its 1px
  corner is kept exactly.
- **Layer bars fill with their Itten hue at ~10% behind a 3px tab**, square ends, with a solid
  leading edge. A selected bar takes the 2px `red` edge (§6.1).
- **Keyframe marks are §3.2's**, at 11px, half-scale on a shut layer's summary row.
- **A row switch is a bare glyph**, `text_primary` on and `text_muted` off, never a primary.
  Canonically ruled and important here: the switches column is the one dense place where a
  colour would be read as meaning and must not be.

### 12A.2 Graph mode

The outline is the Layers outline, identical, as canonically ruled. Curves take §6.2's four
steps; the pane's zero and 100% lines are 2px `rule`; the key readout row at the outline's
foot keeps its two influence fields, each now a rule rather than a well.

### 12A.3 Properties and effect controls

Fixed column edges, the ragged stopwatch column against the dead-straight label edge, Tab
hopping pre-selected — all canonical and kept. The stopwatch is §3.2's filled disc or ring.
Vector pairs are two equal values with a link glyph between them and **one rule under the
pair**, which states the "these are two halves of one measurement" fact that the canonical
single unit rider already makes. Units are lowercase mono at 10, muted.

### 12A.4 Dialogs

The canonical pattern is kept — title strip, tab row, label-left rows in a fixed column,
titled groups, a footer with a summary line and at most one filled action, content-sized
buttons, the stacking footer. Recoloured and recased:

- The dialog's title is a lowercase label with its rule running to the far edge.
- The tab row's fronted tab is `text_primary` over a 2px `red` rule.
- **Group fences are kept as boxes in the Export dialog** — one of the two surviving boxes in
  the application — because a fence around a group of rows is doing structural work a rule
  cannot do in that layout. It is a 1px `hairline` box with the lowercase label notched into
  its top edge, canonically drawn.
- The disabled-section treatment is canonical and kept verbatim: drawn, legible, deaf, with
  the group's own label going `text_disabled`. It is premise 5 exactly.

### 12A.5 Project panel

Six bands, canonical heights, canonical column rules. The changes: state badges become
**rule-bounded rather than tinted-outline** — `in use` in a 1px `text_secondary` box,
`missing` in a 1px `yellow` box with a `yellow` wash, `proxy` in `text_muted` — and the search
well becomes a rule with the colour-swatch filter sitting on its left, unchanged in behaviour.

### 12A.6 Viewer bars

Two 22px strips, canonical heights, canonical shedding order, the transport last. The channel
picker's coloured closed face is kept (§5). **The selection's name drawn on the picture** is
`red` at 9px inside a `red` hairline — the canonical placement, 16 in and 8 down, with
`animated` replaced by the one colour that means the same thing.

## 13. New-panel checklist

1. Every colour from the theme struct; zero hex literals outside it.
2. **Nothing is capitalised** except user content and prose proper nouns (§7.2).
3. Container labels are lowercase, flush left, with their rule running to the far edge.
4. **Two line weights only**: a 2px black rule for structure, a 1px grey hairline between
   rows in a list. No boxes without a named exception.
5. **No fields with edges**: a value is its number and a rule (§3.4).
6. Three colours, one referent each (§3.1). Every coloured mark carries its keyline.
7. Radii 0, or full where the control is genuinely round. **No shadows anywhere.**
8. Interpolation is shape and colour together, split at the vertical centre (§3.2).
9. Nothing within 48px of the Viewer image breaks neutrality.
10. Hit targets: ≥44 chrome, ≥24 visual + ≥32 slop dense; contrast floors met in both rooms.
11. Motion is linear, ≤120ms, transform and opacity only; rotation is the one indeterminate.
12. Keyboard path for every interaction; AccessKit roles and names; visible focus rule.

## Brand

The twin-keyframe mark survives this style better than it survives any other, because it is
already two flat forms and an additive overlap — which is a Bauhaus construction that happens
to also be a true statement about compositing.

Changes: **the gradients flatten to two flat primaries**, `blue` and `red`, with the overlap
white. The keyframes become squares standing on their corners rather than rounded diamonds
(§12: there are no rounded rectangles). The `.lum`, `.lumfx` and `.lumtheme` document icons
follow, and the theme icon's three overlapping swatches — already an equilateral construction
with a computed circumradius — become the three primaries and need no other change, which is
a small piece of evidence that the canonical brand work was closer to this than it knew.

The **splash** keeps the boot log, which is premise 5 in its purest form: real plumbing
reporting itself, the slow module named, a failure shown with a short reason and the
application opening degraded. Its progress hairline runs in `red`. The **broken-glass Persona
5 art direction is withdrawn and replaced**: the splash's artwork is a **constructed
composition of the three forms in the three primaries on the plate**, in the manner of the
1923 exhibition posters — asymmetric, flush to a diagonal, built from circles, squares and
triangles and nothing else. It is the one surface in the application where the composition is
allowed to be the point, which is exactly what a splash is for.

## What it would cost to build

- **Two `LumitColorScheme` entries** (`bauhausPaper`, `bauhausNight`), each a full token set —
  the same shape of work the codebase has already done seven times.
- **`LumitTheme` must split `surface_0` into `grid` and `well`** (§4). This is the one change
  that touches the canonical schemes too, because they all currently read `surface_0` for both
  jobs; the migration is mechanical (every existing scheme sets both fields to its current
  `surface_0`) and it is worth doing regardless of whether this style ships.
- **A `ShapeTokens.bauhaus` entry** (§12), plus the two grammar fields — `labelCase` and
  `strokeWeight` — that both alternative styles in this folder need.
- **The field-is-a-rule change (§3.4) is the invasive one.** Every value well in the
  application is one widget, `HouseWell`-shaped; if it is genuinely one widget the change is
  one file, and if it is not, this is the thing to find out before committing to the style.
- **The icon set would be redrawn** (§5). That is real work — the canonical set is complete and
  owes nothing — and it is the single largest cost here. A staged route draws the twenty
  glyphs the Timeline and Viewer actually use, judges the style on those, and finishes the set
  only if it is adopted.

## Open questions

- **Does the paper room survive beside a picture?** §1.3 states the cost and does not solve
  it. This is the same experiment the Rams envisioning needs, run on a brighter shell, and it
  is the decision everything else here depends on.
- **Does the colour–form code hold at 11px?** §3.2 is the best idea in this document and it
  is asserted, not tested. Half a blue circle beside half a red square at 11px on a 23px row,
  at 125% and 150% Windows scaling, with the keyline drawn — build that one strip first.
- **Is a 3px grid between panels affordable?** On a 1920×1080 laptop with a dense workspace
  the grid costs real pixels and the dock's minimum widths were computed without it. §2.4 may
  have to become 2px, which weakens the structure but not the idea.
- **Does removing the selection wash cost more than it saves?** §6.5 replaces the canonical
  16% accent fill with a 2px edge. On a lane of twenty bars, an edge may be harder to find
  than a wash. If so, the wash returns at 10% and the edge stays.
- **Lowercase and screen-reader output.** §7.2 lowercases the visible label; the AccessKit
  name should keep its natural case, because a screen reader's pronunciation of an acronym
  depends on it. This is a one-line rule but it must be written down before any string is
  lowercased in the table rather than in the widget.
