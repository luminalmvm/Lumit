---
title: Expressions
description: Drive a property from a line of script instead of from keyframes.
sidebar:
  order: 16
---

An **expression** is a line of script on a property, calculated every frame.

## Put one on a property

1. Right-click the property's value field.
2. Choose **Set expression**.

The field becomes a small code editor, seeded with the value that was there, and the
number beside it shows what the expression comes to at the playhead. Type as you would in
any editor: the function names complete as you go.

Right-click the editor and choose **Remove expression** to go back to a plain value — the
property keeps the number it was showing.

Transform properties and effect parameters both take one. Anything made of several
numbers — a point, a colour — does not yet, and the menu simply does not offer it there.

## What the script can read

The language is [Rhai](https://rhai.rs). `time` is the composition's time in seconds;
`layer().time` is the layer's own clock, counted from its in point.

| | |
| --- | --- |
| **Constants** | `time`, `comp_width`, `comp_height`, `comp_fps`, `num_layers`, `num_markers`, `cut_in`, `cut_out` |
| **Maths** | `sin`, `cos`, `sinh`, `cosh`, `floor`, `ceil`, `round`, `abs`, `clamp`, `noise`, `smoothstep`, `fit`, `fit_clamped`, `fit01` |
| **The composition** | `comp().name` |
| **A layer** | `layer()` for this one, `layer("Name")` for another — `.name`, `.time`, `.x`, `.y`, `.rotation`, `.scale_x`, `.scale_y`, `.anchor_x`, `.anchor_y`, `.opacity` |

A few to start from:

```rust
time * 90                          // a turn of ninety degrees a second
layer("Sun").x + 20                // twenty pixels behind another layer
noise(time * 2) * 50               // a smooth wander, the same on every run
fit(layer().time, 0, 2, 0, 100)    // nought to a hundred over two seconds
```

## Save expressions to use again

**Window ▸ Expressions** opens a panel that keeps expressions between projects.

- **New** clears the editor. With a property selected that already has an expression, the
  editor starts with it.
- **Save** stores the script under the name in the name field. A name already in the list
  is replaced.
- **Delete** removes the selected entry.
- **Apply** puts the script on every property row selected in the Timeline. Double-clicking
  an entry does the same.
- **Export…** writes every saved expression to one file. **Import…** adds a file's
  expressions to yours. A name you already have for a different script is kept, and the new
  one gets a number after it.

The search box matches names and scripts.

## Notes

- **An expression that fails reads as −1.** Nothing is reported yet, so a property
  sitting at −1 is worth a second look at the script.

- **One property reading another that reads it back** stops after a hundred cycles 
  rather than at the first.

## In the graph editor

An expression draws as a curve like anything else, sampled across the view. It has no
keyframes to take hold of — to change the shape, change the script.

## Related

- [Keyframes](/use/keyframes/)
- [The graph editor](/use/graph-editor/)
- [The node graph](/use/nodes/)
- [Transform](/use/transform/)
