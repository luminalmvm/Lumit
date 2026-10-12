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
property keeps the number it was showing. The same menu lists **JavaScript** and **Rhai**,
with the expression's language marked.

Transform properties and effect parameters both take one. Anything made of several
numbers — a point, a colour — does not yet, and the menu simply does not offer it there.

## Choose the language

An expression is written in **JavaScript** or in **Rhai**, and each one keeps its own
choice.

- **JavaScript** is the language After Effects expressions are written in, and one
  written for After Effects runs as written.
- **Rhai** is one line of [Rhai](https://rhai.rs).

| Where | The language is set |
| --- | --- |
| A property's editor | In its right-click menu |
| **Animation ▸ Add expression** | In the dropdown under the text, at the right |
| **Window ▸ Expressions** | In the dropdown at the right of the bottom bar |
| An Expression box in the node graph | In the same dropdown, under **Edit expression…** |

A new expression starts in Rhai. **Edit ▸ Settings ▸ Interface ▸ New expressions are
written in** changes that. An expression imported from After Effects arrives as JavaScript.

The text is coloured for the language it is set to, and run in that language whatever it
says. `7 / 2` is `3` in Rhai and `3.5` in JavaScript.

## What Rhai can read

`time` is the composition's time in seconds; `layer().time` is the layer's own clock,
counted from its in point.

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

## What JavaScript can read

Statements, `var`, functions, loops, arrays, objects and strings all work, and arithmetic
on arrays adds and scales them as it does in After Effects.

| | |
| --- | --- |
| **Values** | `time`, `value`, `index`, `inPoint`, `outPoint`, `startTime`, `width`, `height`, `name`, `hasParent`, `parent`, `numKeys` |
| **The composition** | `thisComp`, `comp("Name")` — `.width`, `.height`, `.duration`, `.frameDuration`, `.name`, `.numLayers`, `.layer("Name")`, `.layer(1)` |
| **A layer** | `thisLayer`, or one from `thisComp.layer(…)` — `.name`, `.index`, `.inPoint`, `.outPoint`, `.startTime`, `.hasParent`, `.parent`, `.transform`, `.position`, `.anchorPoint`, `.scale`, `.rotation`, `.opacity`, `.effect("Name")` |
| **An effect's row** | `effect("Name")("Row")` — `.value`, `.valueAtTime(t)`, `.numKeys`, `.key(n)`, `.nearestKey(t)`, `.velocity`, `.speed`, `.wiggle(…)`, `.loopIn(…)`, `.loopOut(…)` |
| **Motion** | `wiggle(freq, amp, octaves, amp_mult, t)`, `valueAtTime(t)`, `posterizeTime(fps)` |
| **Random** | `seedRandom(seed, timeless)`, `random()`, `gaussRandom()`, `noise()` |
| **Interpolation** | `linear`, `ease`, `easeIn`, `easeOut`, `clamp` |
| **Vectors** | `add`, `sub`, `mul`, `div`, `length`, `normalize`, `dot`, `cross` |
| **Conversion** | `degreesToRadians`, `radiansToDegrees`, `timeToFrames`, `framesToTime`, `rgbToHsl`, `hslToRgb` |
| **JavaScript** | `Math`, `parseInt`, `parseFloat`, `isNaN`, and the array and string methods |

An effect is found by the name on its header, and a row by its label.
`effect("Transform")("Position")` reads both halves of a point as one array.

```js
wiggle(3, 20)                              // wander twenty either side, three times a second
value + [effect("Shake")("Amount"), 0]     // push a position sideways by a slider
seedRandom(index, true); random(0, 360)    // one fixed angle for each layer
```

- **`value`** is the number the property held when the expression was put on it.
- **A property holds keyframes or an expression, not both.** `loopOut()` repeats another
  row's keyframes, as in `effect("Shake")("Amount").loopOut()`, and returns `value` on the
  property it is written on.
- **`wiggle` and `random` are not After Effects' own numbers.** The same settings give the
  same amount of movement along a different path, and the same path on every machine.

:::note[Not yet built]
`toComp`, `fromComp`, `toWorld`, `sourceRectAtTime`, `smooth`, markers, text properties,
`footage`, `new`, classes and regular expressions.
:::

## Save expressions to use again

**Window ▸ Expressions** opens a panel that keeps expressions between projects.

- **New** clears the editor. With a property selected that already has an expression, the
  editor starts with it.
- **Save** stores the script and its language under the name in the name field. A name
  already in the list is replaced.
- **Delete** removes the selected entry.
- **Apply** puts the script on every property row selected in the Timeline, in the language
  the dropdown shows. Double-clicking an entry does the same.
- **Export…** writes every saved expression to one file. **Import…** adds a file's
  expressions to yours. A name you already have for a different script or language is kept,
  and the new one gets a number after it.

The search box matches names and scripts.

## Notes

- **A Rhai expression that fails reads as −1.** A JavaScript expression that fails leaves
  the property at `value`. Nothing is reported yet.
- **A text layer's expression is Rhai.**

- **One property reading another that reads it back** stops after a hundred cycles 
  rather than at the first.

## In the graph editor

An expression draws as a curve like anything else, sampled across the view. It has no
keyframes to take hold of — to change the shape, change the script.

## Related

- [Keyframes](/use/keyframes/)
- [Custom controls](/effects/controls/custom-controls/)
- [The graph editor](/use/graph-editor/)
- [The node graph](/use/nodes/)
- [Transform](/use/transform/)
