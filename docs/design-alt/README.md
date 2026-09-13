# Alternative design languages

**Status: speculative.** Nothing in this folder is canonical, and nothing here binds any
code. [docs/15-DESIGN.md](../15-DESIGN.md) remains the one design language Lumit is built to.

These are *envisionings*: complete re-derivations of Lumit's colour, type, density, motion and
voice from a stated design tradition, written to the canonical document's own section skeleton
so the two can be read side by side, one section against its counterpart. They exist to be
judged before anything is built, and each ends with what it would actually cost to ship as an
alternative style.

| Document | The tradition | The one-line difference |
|---|---|---|
| [15-DESIGN-RAMS.md](15-DESIGN-RAMS.md) | Dieter Rams' ten principles; Braun and Vitsœ | A calibrated grey room, one signal orange, one typeface, a tool rail and a deck, and every dimension on a 4px module |
| [15-DESIGN-LANTERN.md](15-DESIGN-LANTERN.md) | OUTLOUD's Lyrica editor, taken as a whole style | Dark cards in a light room, capsules and circles for everything you can press, a floating tool dock, the transport in one pill — the style meant to replace Round |
| [15-DESIGN-BAUHAUS.md](15-DESIGN-BAUHAUS.md) | Itten and Kandinsky's colour-and-form teaching; Bayer's single alphabet | Set aside for now; kept for the two findings below |

**[GREY-ROOM-INVENTORY.md](GREY-ROOM-INVENTORY.md)** is the walk-through list for the Rams
style: every surface, control and state in the UI spec, one checkbox each, with how the grey
room draws it and whether that is drawn, specified, derived or still open.

**Mockups**, each drawing the full shell in two rooms with the structural moves numbered:

- **[mockups/rams-shell.html](mockups/rams-shell.html)** — the grey room, from scratch.
- **[mockups/lantern-shell.html](mockups/lantern-shell.html)** — Lantern, from scratch.
- [mockups/shell.html](mockups/shell.html) — the earlier control: the canonical layout under
  three token sets from one markup, so the token-swap claim can be judged before any layout
  changes.

## Naming the styles

The shipped names — Sharp, Round — name geometry, and "Dieter" would name a person. A style
is a room you work in, so the proposal is one-word names for the rooms, chosen so that none
describes a shape or borrows a name:

| Today | Proposed | Why |
|---|---|---|
| Sharp (canonical) | **Slate** | Flat, dark, hairline-edged, and what everything else is measured against |
| the Rams envisioning | **Instrument** | The deck, the dial, the rail, the engraved scale: a thing with controls on it |
| Round → the Lyrica envisioning | **Lantern** | Dark cards lit inside a light room |

Alternatives, if any of those fails to land: *Bench* or *Studio* for the Rams room; *Capsule*,
*Bloom* or *Pebble* for the Lyrica one; *Ink* for the canonical. Each style's two rooms keep
their own names inside it (grey room / graphite; day / night), and the canonical's stay dark /
light.

## What each document keeps

All three leave [07-UI-SPEC.md](../07-UI-SPEC.md) entirely alone. The dock, the drop zones, the
workspaces, the keymap, what each panel contains and how it responds are unchanged; so is
KD-2's hit-target compensation, the Viewer neutrality zone, AccessKit from day one, the
no-punishment rule, and British English. These are languages, not re-plans.

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
   See [15-DESIGN-RAMS.md](15-DESIGN-RAMS.md) §12 and
   [15-DESIGN-BAUHAUS.md](15-DESIGN-BAUHAUS.md) §12.

## The experiment every document is waiting on

Each proposes a shell lighter than the canonical near-black one, for different reasons, and
each closes on the same unanswered question: **does a lighter shell survive beside a picture
being graded?** The counter-argument is real — a bright surround raises the room's ambient
level and the eye adapts upward, so the picture reads darker and flatter than it is. It is
testable on the target hardware in an afternoon, and neither style should be built before it is.
