# Lumit design language — the Rams envisioning

**Status: speculative. Not canonical, not binding on any code.** This is a complete
alternative to [15-DESIGN.md](../15-DESIGN.md), written to the same section skeleton so the
two can be read side by side, one section against its counterpart. Where a section number
here matches one there, it is answering the same question with a different answer. Where this
document is silent, the canonical one still holds.

It re-derives Lumit's colour, type, density, motion and voice from **Dieter Rams' ten
principles** and the Braun/Vitsœ design practice behind them. Panel inventory, docking and
interaction flows are unchanged and still live in [07-UI-SPEC.md](../07-UI-SPEC.md);
terminology still follows [01-GLOSSARY.md](../01-GLOSSARY.md) exactly.

RFC-2119 keywords are used with their usual force *within this document's own world* — they
bind an implementation of this style, not the shipping one.

Visual reference: [mockups/shell.html](mockups/shell.html), which draws the same shell under
the canonical style, this one, and the Bauhaus one from a single markup.

---

## 0. The ten principles, as rules

The principles are not decoration on this document; they are its argument. Each one below
resolves to something a reviewer can fail a screenshot against.

| # | Principle | What it binds here |
|---|---|---|
| 1 | **Innovative** | Innovation is spent on the *instrument*, never on the chrome: progressive preview, the cache bar's honesty, the Retime lens pair. Chrome that draws attention to its own novelty is a defect (§8, §12). |
| 2 | **Useful** | Every element earns its pixels by answering a question the editor actually asks. The audit in §12A deletes five things the canonical resting state draws. |
| 3 | **Aesthetic** | Beauty is a consequence of the module (§7.1) and of restraint, not an applied layer. There is no decorative element anywhere: no gradient, no glow, no dot, no texture. |
| 4 | **Understandable** | One colour, one referent (§3). One form, one behaviour (§5). A control's appearance states what it does before it is touched. |
| 5 | **Unobtrusive** | The application is a **grey room around a picture**. Chrome is neutral by default and everywhere; the only saturated thing on screen is the signal (§3.1) and the picture itself. |
| 6 | **Honest** | Nothing simulates a material it is not. No glass, no paper grain, no fake depth, no shadow that implies a height nothing has. A progress reading shows real progress or shows a count (§10). |
| 7 | **Long-lasting** | No trend surface: no gradient chrome, no blur, no rounded-card fashion, no accent chosen because it looks current. The palette is the one Braun shipped for forty years because it dated the least. |
| 8 | **Thorough to the last detail** | §7.1's module admits no exceptions, and §12A.6 replaces the canonical metrics table with six numbers rather than forty. A stray pixel is a defect, not a tolerance. |
| 9 | **Environmentally friendly** | Read literally for a GPU application: **idle costs nothing** (§8.4). Chrome that animates while nobody is touching it is banned outright, and the repaint budget is a design rule, not a performance one. |
| 10 | **As little design as possible** | *Weniger, aber besser.* Where the canonical language offers a choice, this one usually removes it. There is one shape, one type family, one signal colour, six heights, and no theme gallery. |

**The test.** A screen passes this style when you cannot remove anything from it without
losing a fact the editor needs. That is a harsher gate than "does it look good", and it is
the only one that matters here.

## 1. Relationship to the canonical language

### 1.1 Kept, unchanged

These survive because they are already Rams-correct, and re-deriving them would only have
produced them again:

- **Semantic tokens only**, with hex literals confined to the theme module.
- **Viewer-surround neutrality** (§2.1, §3.2) — strengthened, in fact: this style makes the
  surround the *same* grey in both rooms (§2.1).
- **Hairline elevation**, no glassmorphism, no gradients-as-chrome.
- **No punishment UI.** A dropped-frame counter is information.
- **The user controls tempo**; nothing auto-advances.
- **Never colour alone** — and here it goes further: colour is the *second* encoding
  everywhere, never the first (§3.2).
- The **hit-target compensation** of KD-2, unchanged: dense controls ≥24px visual and ≥32px
  slop; chrome controls ≥44px.
- **British English, sentence case, no exclamation marks, no emoji.**
- **AccessKit from day one**, keyboard operability of every control, visible focus.

### 1.2 Overturned, with the reason

| Canonical rule | This style | Why |
|---|---|---|
| **Dark-first**, `surface_0` at `#0b0c0e` | **Grey-first.** The default room is a calibrated light grey; Graphite is its equal counterpart, not a fallback (§2, §11) | Rams' world is light grey because a light grey room shows an object honestly. Near-black chrome is a fashion of the last decade, and it fails principle 7. Grading practice agrees: a picture is judged against a *mid* surround, not a black one |
| `viewer_surround` differs by mode (`#121212` dark, `#9c9c9c`–`#b4b4b4` light) | **One grey, `#7f7f7f`, in both rooms** (§2.1) | Principle 6. The surround is a measuring instrument. An instrument that changes its reading when you change the room's lighting is dishonest |
| **Two faces** — Hanken Grotesk and Geist Mono — and *mono for all numbers, absolutely* | **One face** (IBM Plex Sans), tabular figures everywhere a number appears; mono kept for exactly one job (§7.2) | Principle 10. Mono was solving jitter; `tnum` solves jitter. A second face to solve an already-solved problem is design that is not needed |
| **Kickers**: mono, caps, +0.12em, on every container label | **Lowercase 9px labels**, tracked +0.04em, muted (§7.2) | Braun engraves `volume`, not `VOLUME`. Caps are a raised voice, and forty container labels raising their voice at once is noise, not hierarchy |
| Radii `{4, 8, 16, full}` | **`{2, 2, 2, full}`** — one radius, plus true circles for true dials (§12) | Principle 10, and Braun's own tooling radius. Four radii is three decisions per widget |
| `shadow_float`: black @50%, 0/15, blur 50 | **0/1, blur 3, black @14%**, on the same closed list (§2.3) | Principle 6. A menu is a millimetre above the panel, not five centimetres. A 50px blur draws a height nothing has |
| Two saturated roles (`accent` + `animated`), two closed job lists | **One signal** plus three state marks, and `animated` dissolved into form (§3.1, §6.2) | Principle 4. Two "this is in hand" colours is one too many, and the canonical doc's own final open question already suspects it |
| Forty-odd canonical chrome heights (§12A.6) | **Six** (§12A.6) | Principle 8, read the way Vitsœ reads it: thoroughness is a *pitch*, not a spreadsheet |
| Seven named schemes, plus a full theme editor | **Two rooms, no gallery** (§11) | Principle 10. A theme editor is an admission that the theme is not good enough. Keep the `.lumtheme` file format for accessibility overrides; drop the shipped palettes |
| Round as an alternative shape (§12) | **Withdrawn.** There is one shape | Principle 10 again. Round exists to look current, which is principle 7's failure mode |

### 1.3 What this style does not change

Every behaviour in [07-UI-SPEC.md](../07-UI-SPEC.md): the dock, the drop zones, workspaces,
the keymap, what each panel contains and how it responds. This is a language, not a
re-plan. If adopting it requires a panel to gain or lose a control, the change belongs in the
UI spec and should be argued there on its own merits.

## 2. The two rooms

There is one ramp, instantiated at two lightnesses. They are the same design; neither is the
other's inversion, because the eye is not symmetrical and white cannot get brighter.

### 2.1 Grey room (the default)

Braun's light-grey and off-white plastics, at the values they actually held: warm-neutral,
never blue, never pure white.

| Token | Value | Role |
|---|---|---|
| `surface_0` | `#dedcd8` | The ground: application background, timeline well, graph paper, and **input wells**, which are inset |
| `surface_1` | `#ecebe7` | Panel bodies — the default fill |
| `surface_2` | `#e4e2de` | Faint surfaces: header strips, bottom bars, layer rows |
| `surface_3` | `#f7f6f3` | Hover and floating only: menus, popovers, hovered rows |
| `surface_4` | `#d2d0cb` | Pressed fills, raised chips, scrollbar thumbs |
| `viewer_surround` | `#7f7f7f` | The Viewer's pasteboard — **exactly neutral, and the same in both rooms** |

### 2.2 Graphite

| Token | Value | Role |
|---|---|---|
| `surface_0` | `#1c1c1b` | as above |
| `surface_1` | `#262625` | |
| `surface_2` | `#2e2e2c` | |
| `surface_3` | `#383836` | |
| `surface_4` | `#454542` | |
| `viewer_surround` | `#7f7f7f` | **unchanged from the grey room** |

### 2.3 Text and hairlines

Contrast figures are measured against `surface_1`, the surface the great majority of text
actually sits on.

| Token | Grey room | Graphite | Role | Contrast (grey / graphite) |
|---|---|---|---|---|
| `text_primary` | `#1b1b1a` | `#f0efeb` | Headings, values being edited, primary copy | 14.5 / 13.2 |
| `text_secondary` | `#4c4c49` | `#c4c2bc` | Body copy, property names | 7.2 / 8.5 |
| `text_muted` | `#696864` | `#918f89` | Labels, hints, inactive words | 4.7 / 4.7 |
| `text_disabled` | `#848380` | `#78756f` | Disabled controls only | 3.2 / 3.3 |
| `hairline` | `#cdcbc6` | `#353532` | 1px borders between panels, rows, cards | — |
| `hairline_strong` | `#a9a7a1` | `#4e4e4a` | Dividers that must be found; pressed fill | — |

**Elevation is a hairline and a gap, never a wash.** `shadow_float` is
**0/1, blur 3, black @14%** — enough to say "this is a millimetre above the panel", which is
the truth, and permitted on the same closed list the canonical doc names: modal dialogs,
menus and popovers, panels being drag-undocked, and drag ghosts. Nothing else in the
application casts a shadow, including under hover.

**The three-greys rule stands**, and this style leans on it harder: a panel nobody is
pointing at shows `surface_0`, `surface_1` and `surface_2` and nothing else.

## 3. The signal

### 3.1 One colour, one meaning

| Token | Grey room | Graphite | Referent |
|---|---|---|---|
| `signal` | `#b8500d` | `#e8712a` | **Now, and in hand.** The playhead; the single filled action a surface is allowed; the active workspace tab's rule; a selection's edge; a value being dragged; the keyboard focus ring |
| `on` | `#2f6f4a` | `#5cb083` | A process that finished as asked: an export complete, a cache run held |
| `attention` | `#8a5f04` | `#d2a23f` | Something the editor should look at but nothing is broken: overrun, missing footage, a format that cannot carry a setting |
| `fault` | `#a6331e` | `#e58367` | Something failed: a decode, an export, an expression |

That is the whole colour system outside the picture and the three content families of §6.
Everything else is grey. If a fifth referent appears, the answer is a form, not a colour.

**The signal never sets type.** It is a lamp, a fill and a 1–2px mark; it is never the colour
of a word. This keeps it legible at its own low text-contrast (4.2:1 in the grey room) without
compromise, and it keeps the rule simple: if you can read it, it is grey. The one filled
action carries a **`#ffffff` label on the signal fill** in the grey room (5.0:1) and
**`#17110c`** on it in Graphite (6.1:1) — the far end of the ramp from the text around it,
which is the canonical Round rule generalised.

### 3.2 `animated` is dissolved

The canonical language spends a second saturated colour — amber `animated` — on seven jobs,
and its own final open question doubts two of them. This style removes the token.

Each job is re-answered by form, which principle 4 prefers anyway because a form is readable
without colour vision and at 3px:

| Canonical job for `animated` | Answer here |
|---|---|
| Keyframe diamonds | The mark's own shape already says the interpolation (§6.2). Its *presence* says it is keyed |
| Stopwatch on | **Filled** square, against an outlined one off |
| Selected keyframes | `signal` — a selection is a selection, and the editor has one word for it |
| Selected gizmo handles | `signal`, and inside the neutrality zone the user's "neutral handles" option still applies |
| The focused value field | The `signal` focus ring, like every other control. There is no special focus |
| The work-area band | The band is **the absence of a dim**: outside the work area, the lane ground steps one value darker. A band drawn *in* is a second thing on the ruler; a band left *undimmed* is the same fact with nothing added |
| A selected node's border | `signal` |

The result is one stateful colour, one focus, one selection, and a keyframe that is still
readable in a black-and-white screenshot.

### 3.3 The neutrality zone

Unchanged from the canonical §3.2 and, if anything, easier to obey here: within 48px of the
Viewer image area the UI is strictly neutral, gizmos and guides excepted. Because the room is
grey rather than near-black, the transition from chrome to `viewer_surround` is a small step
rather than a cliff — which is the honest arrangement for judging a picture, and the reason
finishing suites are painted the grey they are.

## 4. The token layer

Structurally identical to the canonical §4.1: a plain struct, one per room, passed by
reference, with the no-hex rule and the CI job unchanged. The delta is what it carries:

```
- pub accent: Color32,          → pub signal: Color32,
- pub accent_hover: Color32,    → (removed: see §8.2 — there is no hover colour, only a
                                   surface step, which the surfaces already carry)
- pub animated: Color32,        → (removed: §3.2)
- pub success / warning / error → pub on / attention / fault   (renamed to their referents)
- pub disabled: Color32,        → (removed: text_disabled already says it)
- pub fill_tonal: Color32,      → (removed: informational chips are hairline-bounded,
                                   not filled — §12A.3a)
+                                 pub module: f32,   // §7.1 — the 4px unit, one number
```

The struct loses five fields and gains one. That is principle 10 applied to the code as well
as the screen, and it is the clearest signal that this style is cheaper to maintain than the
one it replaces, not more expensive.

**No theme gallery.** Settings → Appearance offers **Grey room / Graphite** and nothing else.
The `.lumtheme` file format is kept, because an accessibility override is a real need and a
user's high-contrast variant is their business; the seven shipped palettes go. A design that
ships seven palettes has not decided what it looks like.

## 5. Iconography

The canonical set's grammar is already close to right and is kept: one 16-unit grid, one
weight, monochrome via `currentColor`, one icon per chrome word, no emoji ever. Three changes:

- **Stroke 1.5px → 1.25px, butt caps, no round caps.** Braun's engraved marks are thin and
  cut, not drawn with a felt tip. Round caps read as friendly; this style is not friendly, it
  is clear.
- **A glyph is drawn from the smallest number of strokes that names the thing.** Where the
  canonical set draws a recognisable object, this one draws the *operation*. The one
  place this bites: the four painter-drawn exceptions in `icons.dart` stay painter-drawn, but
  each is redrawn on the same 1.25px discipline so the set has one weight and not one-and-a-bit.
- **The Channels indicator keeps its colour** — it is the one glyph whose subject *is*
  colour, and principle 6 forbids drawing it in grey and pretending otherwise.

**Chrome labels: Words is the default and stays the default.** The canonical three-way
setting (§5.1) survives, because a person who knows the tool cold should be able to shed the
words. But under principle 4 a glyph that has to be learnt is worse than a word that does
not, so Words is what a new installation shows, and the tooltip still carries the word in
every mode.

## 6. Editor semantics

The three content families below are **content, not chrome**. They are allowed colour because
they are data being drawn, exactly as the picture is; they are not part of the signal system
and must never be confused with it. Each family is drawn at one value and one chroma so that
none shouts over another — and so that `signal` beats every one of them, which is the property
the whole arrangement depends on.

### 6.1 Layer types

Six identity colours, each a 3px tab on the left edge of the layer's bar plus a ~10% tint
over its fill, exactly as canonically specified. The *values* change: this family is
re-derived at one lightness and one low chroma, so a full timeline reads as a set of
grey-greens and grey-blues that happen to differ, rather than as six opinions.

| Layer type | Grey room | Graphite |
|---|---|---|
| Footage | `#6f7f88` | `#77878f` |
| Sequence | `#6c7a94` | `#74839d` |
| Precomp | `#8a7284` | `#917a8c` |
| Solid / Adjustment | `#7c7c78` | `#84847f` |
| Text | `#8b8570` | `#938d78` |
| Camera | `#8a7c5c` | `#928463` |

Reserved, unchanged in intent: Shape, Null (outline only), Audio, Light.

### 6.2 Keyframes and curves

- Interpolation is **shape, and shape alone**: diamond linear, square hold, hourglass bezier
  in the lanes, circle bezier in the graph. Split at the vertical centre, left half incoming,
  right half outgoing, as canonically specified — that ruling is excellent and is kept
  verbatim.
- **At rest a key is `text_secondary` filled with a `surface_1` outline. Selected, it is
  `signal`.** There is no third state and no `animated`.
- Curve strokes take a four-step ramp in dimension order, re-derived at one value:
  `#4a6b7d` / `#5c7a63` / `#8a6a72` / `#7d7458` in the grey room, one step lighter in
  Graphite. A single-dimension property uses the first.
- Bezier handles: `text_muted` stems, `signal` while grabbed. Selected is `signal` too — see
  §3.2.
- Graph paper: `surface_0` ground, `hairline` minor lines, `hairline_strong` at zero and 100%.

### 6.3 The cache bar

The canonical design here is genuinely good — two families, three fill heights, uncached
drawn as nothing — and its reasoning (brightness alone was illegible on a 3px stripe) is
exactly the kind of finding principle 8 is about. It is kept whole, with the colours moved
onto this palette: **`on` for held**, and a steel `#4a6b7d` (grey room) / `#5f8196`
(Graphite) for the disk tier. Fill heights stay full / 70% / 45%.

### 6.4 Overrun, markers, waveforms

- **Overrun**: `attention` hatching at 45°, 1px lines, 4px pitch, over a ~12% wash, with the
  mono `hold` tag. Unchanged but for the case of the tag (§7.2).
- **Markers**: a plain grey flag, as canonically ruled, and for the canonical reason — a
  marker says *here*, not *good* or *careful*. `#565656` in the grey room, `#c4c4c4` in
  Graphite. The upward triangle standing on the cache bar, the `surface_4` backdrop pill and
  the one-marker-per-frame rule are all kept.
- **Waveforms**: one muted steel, `#5a7b87` in the grey room and `#6d9aa6` in Graphite, with
  the multiwave stack ranked by value as canonically ruled. Never `signal`.

### 6.5 Selection, focus, drop targets

One treatment, used everywhere, because the editor has one idea of "this is the thing you
have":

- **Selection**: `signal` 1px border, `signal` @ 14% fill. Clips, layers, assets, keyframes,
  nodes, handles — all of them.
- **Focus**: a 1px `signal` ring offset 1px outside the control's bounds. No exceptions; the
  value field's special amber focus goes with `animated`.
- **Drop targets**: 1.5px dashed `signal` with a 10% fill; an insertion caret is a 2px
  `signal` line.
- **The playhead** is a 1px `signal` line with an 11×8 `signal` head at the top of the ruler,
  as canonically drawn. The head earns its place — a bare hairline reads as a row seam — and
  the ≥24px grab target is unchanged.

## 7. The module

### 7.1 One number

**Everything is a multiple of 4.** Not "a 4/8/12/16 spacing scale, with heights chosen per
element" — every height, every gap, every inset, every padding, in chrome and in dialogs
alike. The Vitsœ 606 is one drilled pitch and forty years of furniture hanging off it; this
is the same idea with `module = 4`.

The consequence is that the canonical §12A.6 metrics table — forty-one rows of surveyed
artboard measurements, several of which disagree with each other by a pixel and one of which
the doc itself records as the artboard disagreeing with itself — collapses into **six
heights**:

| Height | Modules | What stands at it |
|---|---|---|
| **16** | 4 | Clip bars within a lane row; in-row pickers; the search well |
| **20** | 5 | Secondary rows: filter rows, panel bottom bars, the key readout; value wells; dropdown faces |
| **24** | 6 | **The row.** Outline rows, lane rows, panel header strips, the Viewer's two bars, composition tabs, timeline chrome rows |
| **28** | 7 | Property and effect-parameter rows, effect section headings |
| **32** | 8 | Dialog rows, the dialog title strip, dialog controls |
| **48** | 12 | The timeline ruler (2 × the row), dialog footers |

Plus two atoms that are not heights: the **cache bar at 4**, and the **hairline at 1**.

**Compact is one substitution, not a column**: the row becomes 20 and the property row 24 —
each drops one module. Nothing else changes, no type resizes, and there is no second table to
keep in step with the first.

This is a genuine claim and should be tested as one: build one panel to it and see whether
anything the editor needs stops fitting. §12A.6 records what is expected to hurt.

### 7.2 Type

**One face: IBM Plex Sans** (SIL OFL), bundled, with **tabular figures (`tnum`) switched on
globally**. Plex is the nearest available thing to the neo-grotesque Braun set everything in:
neutral, plain-numeralled, drawn for small sizes, and — the reason it wins over Inter or
Archivo — it ships a mono cut from the same skeleton for the one place mono is honest.

**The mono-for-numbers rule is retired, and this is the most arguable thing in this
document.** Its purpose was to stop horizontal jitter while scrubbing; `tnum` stops horizontal
jitter while scrubbing. A second face bundled to solve a solved problem is design that was not
needed, which is principle 10's whole subject. **IBM Plex Mono survives for exactly one job**:
text that is genuinely machine output and should look like it — the boot log, file paths, and
expression source. Timecode, frame counts, percentages, durations and property values are set
in Plex Sans with `tnum`.

> If the first prototype shows that a column of timecode is measurably harder to scan in a
> proportional face with tabular figures, this rule loses and mono comes back for numerals.
> That is the honest version of the claim, and it is the one experiment this style most needs
> to run.

**Labels are lowercase.** The canonical kicker — mono, caps, +0.12em, on every container
label in the application — is replaced by:

> **The label**: 9px, +0.04em, `text_muted`, sentence case with no capital unless the word is
> a proper noun. `timeline`, `source time`, `work area`, `export queue`.

Braun engraves `volume`. Forty container labels shouting at once is not hierarchy, it is
noise, and lowercase at 9px is *quieter* and therefore more unobtrusive (principle 5) while
staying just as clearly "the application's word rather than the user's".

| Size | Use |
|---|---|
| 9px, +0.04em, muted | **The label**: panel titles, section headers, column headers, tab labels, dialog titles, attribution, ruler numbers, the status bar |
| 10px | Units beside a value (`px`, `%`, `°`), outline-row readouts, secondary notes and hints, layer bar labels |
| 11px | Panel body copy, property names, menus, buttons, **and every value in a well** |
| 13px | Dialog body emphasis — the one thing in chrome above 11px |
| 22px+ | About box, the welcome screen, empty states only — outside chrome |

Nothing in chrome is bold. Weight is not a hierarchy this style uses; size and value are.

### 7.3 Hit targets

Unchanged from KD-2, and the module happens to agree with it: 44 is 11 modules, 32 is 8, 24 is
6. Dense-surface controls keep ≥24px visual on the smaller axis and ≥32px of slop; toolbar,
transport and dialog controls keep ≥44px. The tool strip's 44-across-30-down compromise
becomes 44-across-**32**-down, which is the module's nearest step and gains two pixels of
comfort for nothing.

## 8. Motion

### 8.1 Less of it

- Micro-motion budget **≤100ms** (from 150), transform and opacity only, and **ease-out
  only** — no spring, no overshoot anywhere in chrome.
- **The one signature interaction is withdrawn.** The canonical drag ghost lags the cursor and
  settles with a small overshoot on drop. A clip does not have mass; pretending it does is
  principle 6's exact failure. The ghost pins to the cursor and lands where it is dropped.
- Timeline zoom still tracks the wheel 1:1; nothing auto-advances; no scroll hijack.

### 8.2 There is no hover colour

Hover and press are **surface steps**, never hue changes and never strokes appearing: idle on
the panel's own surface, `surface_3` hovered, `hairline_strong` pressed. This is the
canonical rerun-derived grammar, kept, and it is why `accent_hover` disappears from the token
struct (§4).

### 8.3 The three tiers stay

All / Minimal / None, with the OS reduced-motion request mapping onto None, and any meaning
carried by motion also carried by colour or text at every tier. Playback, scrubbing,
progressive refinement and waveform scrolling are content and are exempt.

### 8.4 Idle costs nothing

Principle 9, read for what this application actually is:

- **No chrome element animates while nobody is touching it.** No pulse, no blink, no
  shimmer, no breathing accent, no live dot on a panel header.
- **A panel nobody is pointing at issues no repaint.** The frame loop is driven by input,
  playback and engine events. An idle Lumit on a laptop should be measurable at zero
  discrete-GPU wakeups, and that is a design requirement here, not only a performance one.
- **The splash's boot log is the exception, and it is honest motion**: lines appear because
  something genuinely finished.

## 9. Accessibility

The canonical §9 is kept in full — AccessKit roles and names, keyboard operability of every
control, the safe-triangle submenu rule, reading-order tab traversal, modal focus scopes,
CI-checked contrast floors, never colour alone.

Two things this style adds:

- **Colour is the second encoding everywhere, never the first.** §3.2's dissolution of
  `animated` was done for this reason as much as for principle 10: after it, every state in
  the editor is legible in a greyscale screenshot.
- **The contrast floors are re-run against both rooms**, and §2.3's table is where the figures
  live. `text_muted` clears 4.5:1 in both, which the canonical dark ramp's `#8b9296` also did;
  the difference is that here the figure is quoted for the grey room too rather than assumed
  to carry over.

## 10. Voice

Unchanged from the canonical §10 in almost every particular: British English, sentence case,
calm, no exclamation marks, no emoji, glossary-exact feature names, banner errors rather than
modal storms, factual progress copy, soft empty states.

Two changes:

- **Sentence case now includes the labels** (§7.2), so the *whole* interface is one case.
- **The one rationed joke goes.** The canonical about box carries a serif line under the
  version number. Principle 3 says the aesthetic is a consequence of the work; a joke is an
  applied layer, and one joke is a precedent for the second. The about box carries the mark,
  the version, the licences, and the names of the people involved — which is what an about box
  is for.

And one thing made explicit, because principle 6 deserves it stated: **a progress reading
shows real progress or a real count, never a spinner standing in for knowledge we do not
have.** `Exporting — 41% · 02:12 remaining` when the estimate is sound; `Exporting —
frame 103 of 250` when it is not; never an indeterminate bar implying a measurement nobody
made.

## 11. The two rooms are one design

§2's two token sets are the whole of the difference. No widget code branches on the room; no
layout changes; no glyph is redrawn. The surround does not move (§1.2). The signal keeps one
hue and shifts value by room, which is the canonical `with_accent` behaviour and is kept.

**Which room is the default** is the one open decision worth taking deliberately rather than
by habit, and §2's argument is that it should be the grey one. A person editing at night in a
dark studio will want Graphite, and Graphite is a first-class room precisely so that wanting
it is not a compromise.

## 12. Shape

**There is one.** Round (canonical §12 and §12.1) is withdrawn — the stadium controls, the
filled-pill actives, the bigger cards, the header dot, the capsule bars, the tile gap and the
card shadow all go with it. Panels butt together separated by a single hairline; there are no
gaps between docked panels and no inset from the window edge.

Radii: **2px on everything**, and **full only where the control is genuinely round** — which,
in this style, is a real category rather than a styling choice:

> **The dial.** A control whose value is continuous, bounded and set by feel rather than by
> typing — the zoom, the exposure, a rotation — MAY be drawn as a **circular dial with an
> engraved tick scale and one index mark**. It is the Braun form, and it is the honest form:
> a thing you turn looks like a thing you turn. A dial always sits beside its own value in a
> well, because a dial is imprecise and typing is not, and the two together are what the
> canonical §2.1 well was reaching for.

This is the one place this document proposes something the canonical language has no
counterpart for, and it is offered as the single element of principle 1: innovation spent
once, on a control, not on chrome.

## 12A. The resting state

The canonical §12A is a long and largely excellent set of rulings, most of which are about
*behaviour* and survive untouched: the fixed column edges, Tab hopping pre-selected values,
the reserved keyframe-navigation slot, the vector-pair linking arithmetic, the per-format
capability table, the degradation ladder, the minimum-width enforcement, the Viewer bar's
shedding order. All of that is kept.

What changes is what the resting state *draws*. Principle 2 says every element answers a
question the editor asks; this is the audit.

### 12A.1 Removed

| Element | Why it goes |
|---|---|
| **The canonical kicker's capitals** | §7.2. Forty labels, one case, lowercase |
| **`animated`'s seven uses** | §3.2. Each re-answered by form or by `signal` |
| **The work-area band as a painted band** | §3.2. Outside the work area, the lane ground steps one value darker. The band is the undimmed part |
| **The Round shape entirely** | §12 |
| **The header dot** | Canonical §12.1 calls it "the reference's quiet live-mark … decorative, never a status light". Principle 3: there are no decorative elements |
| **The composition tab's and workspace tab's two different active treatments** | One treatment: the fronted tab's word goes to `text_primary` and a 2px `signal` rule sits under it. The canonical language already reached this for panel tabs; this extends it to all three |
| **Six shipped colour schemes and the theme editor** | §4 |

### 12A.2 Kept, with the numbers re-cut to the module

The Timeline, Project panel, Welcome screen, dialogs and Viewer bars keep their approved
anatomy — six bands in the Project panel, the double-height ruler reading as one band, the
560-wide welcome column, the label-left dialog rows, the Viewer's two 22px strips — with
every height taken to §7.1's nearest module:

| Canonical | Here | Note |
|---|---|---|
| Panel header strip 22 | **24** | The row |
| Viewer bottom bar 22 | **24** | |
| Secondary rows 19 / 18 | **20 / 20** | Compact no longer differs here |
| Timeline chrome rows 24 and 23 | **24 and 24** | The owner's desktop ruling asked for ≥20 and got 24; the second row joins it |
| Outline and lane rows 23 / 22 | **24 / 20** | Regular gains a pixel, Compact loses two |
| Property rows 27 / 26 | **28 / 24** | |
| Ruler, derived: 47 / 36 | **48 / 40** | Still derived — the ruler is exactly what the outline spends on its two chrome rows, which is what keeps the two halves of the Timeline meeting |
| Cache bar 3 | **4** | |
| Value wells 20 | **20** | Already on the module |
| Dialog title strip and rows 30 | **32** | |
| Dialog footer 45 | **48** | |
| Welcome column 560 wide, blocks 28 apart | **560 / 28** | Already on the module |
| Settings window 760×520 | **760×520** | Already on the module |

**What is expected to hurt**, stated in advance so the prototype can be judged honestly:

1. The Timeline outline row at 24 is one pixel taller than the approved 23, which costs
   roughly one visible layer every twenty-four rows. That is the module's price and it is
   thought to be worth paying.
2. Compact's row at 20 is two pixels tighter than the canonical 22, which pushes the 16px clip
   bar and the 18px in-row picker close to their floors. If a picker cannot sit in a 20px row
   without breaking KD-2, Compact's row becomes 24 and Compact stops differing from Regular in
   the outline — which would be a real loss and the clearest argument against §7.1.
3. The Viewer's two bars at 24 rather than 22 take 4px of picture. The bars shed in the
   canonical order and the transport is still last.

### 12A.3 Dialogs

The canonical pattern is kept whole — title strip, optional tab row, label-left rows with the
label in a fixed column, label-titled groups, a footer with a summary line and at most one
filled action, buttons sized by their content, the stacking footer when the actions will not
fit one line. Three notes:

- The label column widths (190/12, 110/12, 100/10) become **192/12, 112/12, 100/8** — the
  module, and each still pinned by its own metrics test.
- **A disabled control is drawn, legible and deaf**, exactly as the canonical Export dialog
  rules. That ruling is principle 6 in full and it is kept verbatim, section-darkening
  included.
- **The single filled action is a ceiling, not a floor.** Kept, and used more: the welcome
  screen spends none, Settings' footer spends none.

### 12A.4 Feedback

Unchanged: transient, local, under the cursor, lasting as long as the gesture, leaving no
trace. A panel nobody is touching looks exactly as §2 says it does.

## 13. New-panel checklist

1. Every colour from the theme struct; zero hex literals outside it.
2. **Every height, gap and inset a multiple of 4** (§7.1), and one of the six heights where
   it is a height.
3. One face, `tnum` on, nothing bold, nothing in chrome above 11px except dialog body
   emphasis; every container label a 9px lowercase muted label.
4. At most three surface values at rest; hairline separation; radius 2; shadow only on the
   closed float list.
5. `signal` is the only stateful colour, on §3.1's list; `on`/`attention`/`fault` for the
   three states; the content families of §6 are content and never state.
6. **Every state legible in a greyscale screenshot.**
7. Hit targets: ≥44 chrome, ≥24 visual + ≥32 slop dense.
8. Keyboard path for every interaction; AccessKit roles and names; visible focus ring;
   contrast floors met against both rooms.
9. **Nothing animates at rest, and an idle panel issues no repaint** (§8.4).
10. Copy: British English, sentence case throughout, no exclamation marks, banner errors,
    no jokes.
11. Terminology audited against the glossary.
12. **Remove one more thing.** Then check nothing was lost.

## Brand

The twin-keyframe mark is kept — two keyframe diamonds side by side with an additive white
where they overlap. It is a good mark and it is honest: keyframes are motion, the overlap is
compositing, the white is luminance.

One change: **the gradients flatten to solids.** The mark becomes two flat colours and a
white, which is what it already is at 16px and what it should be at 512. Principle 7 — a
gradient dates, a flat mark does not — and principle 6 — the gradient implies a lighting
nothing in the mark has.

The splash keeps the boot log, which is the most Rams-correct thing in the application: real
plumbing reporting itself, slow items visible and attributable, a failure shown in
`attention` with a short reason and the application opening degraded rather than hanging on a
spinner. **The broken-glass splash art is withdrawn** — it is decoration, and principle 3
says the aesthetic is a consequence of the work rather than a layer over it. The splash is the
mark, the wordmark, the version, and the log.

## What it would cost to build

The shipping architecture is already shaped for this, which is the reason this document is
worth taking seriously rather than only reading.

- **Two new `LumitColorScheme` entries** (`ramsGrey`, `ramsGraphite`) building a
  `LumitTheme` each. This is where the great majority of the work is, and it is the kind
  of work the codebase already does seven times.
- **`ShapeTokens` needs two new fields**, not a new mechanism: a `labelCase` and a
  `strokeWeight`, because §7.2's lowercase labels and §5's 1.25px icons are grammar, not
  colour, and the existing Sharp/Round split is exactly the hook for them.
- **The module (§7.1) is the invasive part.** Every height currently taken from §12A.6's table
  would have to come from one constant, which means touching the metrics tests
  (`timeline_alignment_test`, `project_panel_metrics_test`, `viewer_metrics_test`) rather than
  the widgets. That is a day of arithmetic and a day of arguing about two of the pixels.
- **Removing `animated` is a rename**, not a redesign: seven call sites, each already
  named in §3.2 with what it becomes.
- **The dial (§12) is genuinely new work** and should be prototyped on the zoom control alone
  before anything else here is committed to.

A staged route, if one is wanted: ship the two schemes first and judge them; then the label
case; then the module; and treat the dial as a separate proposal on its own merits.

## Open questions

- **Does the grey room survive a real grading session?** §2's argument is that a mid-grey
  surround is what finishing suites use and that near-black chrome is a fashion. The counter
  is that a light shell raises the room's ambient level and the eye adapts upward, which makes
  the picture look darker than it is. This is testable on the target hardware in an
  afternoon and should be, before anything is built.
- **Does `tnum` in a proportional face really replace mono for timecode?** §7.2 stakes the
  type system on it and names the experiment. If it fails, mono returns for numerals only and
  the rest of §7.2 stands unchanged.
- **Does Compact survive the module?** §12A.2's second expected pain. If a 20px row cannot
  carry an 18px picker within KD-2, either Compact loses its distinctness or the module gains
  an exception — and an exception in §7.1 is a hole in principle 8.
- **Is one signal enough for a compositor?** §3.2 removed a colour the canonical language
  spends on seven jobs. The honest risk is that "selected keyframe" and "selected clip" now
  look identical and the eye wants them not to. The answer, if so, is a form difference, not a
  second colour.
- **The dial's keyboard and accessibility story.** §9 demands a keyboard equivalent for every
  drag. A dial's is arrow keys with a modifier for coarse steps, which is straightforward —
  but its AccessKit role and its value announcement need writing before it ships anywhere.
