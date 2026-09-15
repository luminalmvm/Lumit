# Alternative design languages

**Status: built, and history.** Two of the styles studied here ship as shapes, **Desk** and
**Lantern**, beside **Studio**. [docs/15-DESIGN.md](../15-DESIGN.md) is again the one
canonical design language Lumit is built to, and its §12 states the three shapes; nothing in
this folder binds code. These documents are where the two newer shapes were drawn, kept as the
reasoning behind each rule, with notes marked *as built* where the build parted from the
drawing. Where a document here and the canonical one disagree, the canonical one holds.

These are *envisionings*: complete re-derivations of Lumit's colour, type, density, motion and
voice from a stated design tradition, written to the canonical document's own section skeleton
so the two can be read side by side, one section against its counterpart. Each ends with what
it was expected to cost to ship as an alternative style.

| Document | The tradition | The one-line difference |
|---|---|---|
| [15-DESIGN-DESK.md](15-DESIGN-DESK.md) | Ten principles of good design, and the product practice behind them | A calibrated grey room, one signal orange, one typeface, a tool rail and a deck, and every dimension on a 4px module |
| [15-DESIGN-LANTERN.md](15-DESIGN-LANTERN.md) | A pane as an object in a room, not a tile in a grid | Lumit's own dark ramp arranged as cards in a light room, the spruce accent on fills, four radii that each mean one thing, and a toolbar that sits top or left. The style meant to replace Round |
| [15-DESIGN-BAUHAUS.md](15-DESIGN-BAUHAUS.md) | Itten and Kandinsky's colour-and-form teaching; Bayer's single alphabet | Set aside for now; kept for the two findings below |

**The inventories** are the walk-through lists: every surface, control and state in the UI spec,
one checkbox each, with how that style draws it and whether the treatment is drawn on the
mockup, specified in the document, derived from the system, or still open. They carry the same
lines in the same order, so they can be read side by side.

- **[DESK-INVENTORY.md](DESK-INVENTORY.md)** for Desk.
- **[LANTERN-INVENTORY.md](LANTERN-INVENTORY.md)** for Lantern.

**Mockups**, each drawing the full shell in two rooms with the structural moves numbered:

- **[mockups/rams-shell.html](mockups/rams-shell.html)**: the grey room, from scratch.
- **[mockups/lantern-shell.html](mockups/lantern-shell.html)**: Lantern, from scratch, with a
  switch for the toolbar's two positions.
- [mockups/shell.html](mockups/shell.html) — the earlier control: the canonical layout under
  three token sets from one markup, so the token-swap claim can be judged before any layout
  changes.

## Naming the styles

Sharp and Round name geometry, which stops being true the moment a style changes more than
its corners, and "Dieter" would name a person. The rule that replaces them:

> **A style names the arrangement. A room names the palette.**

That is the whole reason *Slate*, *Graphite* and *Ink* are wrong for this list: they read as
swatches, and Lumit already has a picker full of swatches (Dark, Dark blue, Light, Gruvbox,
Catppuccin). A style name that could sit in that picker is a style name in the wrong picker.
So every candidate below is a place or a thing you work at, and none of them is a colour.

**Settled, 2026-09-13:**

| Today | Name | Why |
|---|---|---|
| Sharp (canonical) | **Studio** | The default professional arrangement, panels meeting flush, and what the others are measured against |
| the grey-room envisioning | **Desk** | A mixing desk: a rail of tools, a deck under the picture, a dial, a strip of readouts |
| Round, replaced by the cards envisioning | **Lantern** | Dark cards lit inside a light room |

**Console** was the obvious word for Desk and is ruled out on purpose: Lumit already has an FX
console ([07-UI-SPEC.md](../07-UI-SPEC.md) §12.2), and two consoles is one too many.

The files are named to match: `15-DESIGN-DESK.md` and `DESK-INVENTORY.md` were
`15-DESIGN-RAMS.md` and `GREY-ROOM-INVENTORY.md` until the styles were built. In code the
shape is `ThemeShape { studio, desk, lantern }`.

Each style keeps its own two rooms, and those are free to sound like palettes because they
are: grey room and graphite, day and night, and the canonical's dark and light. As built,
Desk's two are colour schemes, Grey room and Graphite, offered in their own row under the
Shape chips and usable under any shape; Lantern's two are the Room setting, Day or Night.

## What each document keeps

All three leave [07-UI-SPEC.md](../07-UI-SPEC.md) alone, with **one named exception**. The
dock, the drop zones, the workspaces, the keymap, what each panel contains and how it responds
are unchanged; so is KD-2's hit-target compensation, the Viewer neutrality zone, AccessKit from
day one, the no-punishment rule, and British English. These are languages, not re-plans.

Two amendments were proposed in [15-DESIGN-LANTERN.md](15-DESIGN-LANTERN.md), both
machine-local settings, both binding on **every** style rather than on the one that raised
them, and both now in the UI spec:

1. **Toolbar position: Top or Left** ([07-UI-SPEC.md](../07-UI-SPEC.md) §1.7). The identical
   toolbar is carried either way; only where it stands changes. The default is the style's
   choice: the rail under Desk, the strip elsewhere.
2. **Range sliders: on or off** ([07-UI-SPEC.md](../07-UI-SPEC.md) §6). The track beside a
   ranged parameter's number is optional, because someone who types and scrubs pays for it on
   every ranged row and never touches it. On is the default under every shape.

A person's own pick holds whatever the style; only the style's choice follows it. A third
setting came with the build: the Viewer bars gained a fourth arrangement, **Deck**, drawn
from both studies' decks (§2.2 there), and it too defaults to the style's choice.

All keep the canonical document's *behavioural* rulings in §12A wholesale — the fixed
column edges, Tab hopping pre-selected values, the matte column's conditional width, the
per-format capability table, the degradation ladder. What they change is what the resting state
draws.

## Two findings worth keeping regardless

These fell out of writing the envisionings and hold whether or not any is built:

1. **`surface_0` carries two jobs.** It is the ground *between* panels and the ground *inside*
   one (the timeline well, graph paper, input wells). They only want the same colour because
   the canonical shell is dark throughout; any light-shelled scheme splits them. See
   [15-DESIGN-BAUHAUS.md](15-DESIGN-BAUHAUS.md) §4.
2. **`ShapeTokens` is the right hook for non-colour grammar.** Both styles need a *label case*
   and a *stroke weight* alongside the existing radii and gaps, and neither is a colour.
   See [15-DESIGN-DESK.md](15-DESIGN-DESK.md) §12 and
   [15-DESIGN-BAUHAUS.md](15-DESIGN-BAUHAUS.md) §12.

## The experiment every document was waiting on

Each proposed a shell lighter than the canonical near-black one, for different reasons, and
each closed on the same question: **does a lighter shell survive beside a picture being
graded?** The counter-argument is real: a bright surround raises the room's ambient level and
the eye adapts upward, so the picture reads darker and flatter than it is. The build answers
it with two safeguards rather than a ruling: the Viewer stage stays the fixed neutral of
15-DESIGN §2.1 under every shape and scheme, and Lantern's room is a setting, Day or Night,
so the light ground is one click from the canvas colour whenever a grade needs it.
