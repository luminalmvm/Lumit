---
title: Expressions
description: Drive a property from a line of script instead of from keyframes.
sidebar:
  order: 16
---

An **expression** is a line of script on a property. It is worked out afresh on every
frame, so the property follows whatever the script says rather than sitting between
keyframes.

## Put one on a property

1. Right-click the property's value field.
2. Choose **Set expression**.

The field becomes a small code editor, seeded with the value that was there, and the
number beside it shows what the expression comes to at the playhead. Type as you would in
any editor: the function names complete as you go.

Right-click the editor and choose **Remove expression** to go back to a plain value — the
property keeps the number it was showing.

Transform properties and effect parameters both take one. Anything made of several
numbers — a point, a colour — does not yet.

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

`noise` takes no seed and holds no state: the same project gives the same frames every
time it is opened.

## What it does not do

- **No After Effects library.** `wiggle`, `loopOut`, `valueAtTime`, `linear` and `ease`
  are not there. `noise` and `fit` cover much of what they were used for.
- **An expression that fails reads as −1.** Nothing is reported yet, so a property
  sitting at −1 is worth a second look at the script.
- **One property reading another that reads it back** stops rather than hanging, but it
  stops after a hundred hops rather than at the first.

## In the graph editor

An expression draws as a curve like anything else, sampled across the view. It has no
keyframes to take hold of — to change the shape, change the script.

## Related

- [Keyframes](/use/keyframes/)
- [The graph editor](/use/graph-editor/)
- [Transform](/use/transform/)
