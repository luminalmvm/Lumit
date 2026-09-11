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
| [15-DESIGN-RAMS.md](15-DESIGN-RAMS.md) | Dieter Rams' ten principles; Braun and Vitsœ | A calibrated grey room, one signal orange, one typeface, and every dimension on a 4px module |
| [15-DESIGN-BAUHAUS.md](15-DESIGN-BAUHAUS.md) | Itten and Kandinsky's colour-and-form teaching; Bayer's single alphabet | Paper plates on a black grid, three primaries with one referent each, no capitals anywhere, and Kandinsky's square/triangle/circle as the keyframe code |

**[mockups/shell.html](mockups/shell.html)** draws the Project panel, Viewer, Effect controls
and Timeline under all three languages from one markup, with a switch between them and between
each language's two rooms. Open it in a browser. Every visible difference between the three is
a token value or a grammar switch — no panel is laid out twice, which is the same claim the
canonical document makes about Light mode, tested against two languages it was not designed
alongside.

## What each document keeps

Both leave [07-UI-SPEC.md](../07-UI-SPEC.md) entirely alone. The dock, the drop zones, the
workspaces, the keymap, what each panel contains and how it responds are unchanged; so is
KD-2's hit-target compensation, the Viewer neutrality zone, AccessKit from day one, the
no-punishment rule, and British English. These are languages, not re-plans.

Both also keep the canonical document's *behavioural* rulings in §12A wholesale — the fixed
column edges, Tab hopping pre-selected values, the matte column's conditional width, the
per-format capability table, the degradation ladder. What they change is what the resting state
draws.

## Two findings worth keeping regardless

Both fell out of writing these and hold whether or not either style is ever built:

1. **`surface_0` carries two jobs.** It is the ground *between* panels and the ground *inside*
   one (the timeline well, graph paper, input wells). They only want the same colour because
   the canonical shell is dark throughout; any light-shelled scheme splits them. See
   [15-DESIGN-BAUHAUS.md](15-DESIGN-BAUHAUS.md) §4.
2. **`ShapeTokens` is the right hook for non-colour grammar.** Both styles need a *label case*
   and a *stroke weight* alongside the existing radii and gaps, and neither is a colour.
   See [15-DESIGN-RAMS.md](15-DESIGN-RAMS.md) §12 and
   [15-DESIGN-BAUHAUS.md](15-DESIGN-BAUHAUS.md) §12.

## The experiment both documents are waiting on

Each proposes a shell lighter than the canonical near-black one, for different reasons, and
each closes on the same unanswered question: **does a lighter shell survive beside a picture
being graded?** The counter-argument is real — a bright surround raises the room's ambient
level and the eye adapts upward, so the picture reads darker and flatter than it is. It is
testable on the target hardware in an afternoon, and neither style should be built before it is.
