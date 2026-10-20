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

An editor opens over the row, seeded with the value that was there. The number under the
text is what the expression comes to at the playhead, and the function names complete as
you type. When the expression fails, the reason is written there instead, and the row
shows its value in red.

| | |
| --- | --- |
| **Enter** | A new line |
| **Tab** | Four spaces |
| **Shift+Tab** | Up to four spaces off the start of each line the selection touches |
| Click away, or **Ctrl+Enter** | Keep the expression |
| **Escape** | Put back what was there |

**Tab** and **Shift+Tab** do the same in the Expressions panel, the
**Animation ▸ Add expression** editor and the shader editor.

The row then shows the expression and its value, and a click on either opens the editor
again. In the Timeline the expression is written along the property's lane, and a click
there opens the editor across the lane.

**Remove expression**, in the editor and in the row's right-click menu, goes back to a
plain value. The property keeps the number it was showing. The same menu lists
**JavaScript** and **Rhai**, with the expression's language marked.

**Animation ▸ Add expression** opens the same editor for the property rows selected in the
Timeline, and puts the expression on every one of them.

Transform properties and effect parameters both take one. An effect's point takes one on
each half, and is then drawn as two rows. A colour does not yet, and the menu does not offer
it there.

## Choose the language

An expression is written in **JavaScript** or in **Rhai**, and each one keeps its own
choice.

- **JavaScript** is the language After Effects expressions are written in, and one
  written for After Effects runs as written.
- **Rhai** is one line of [Rhai](https://rhai.rs).

| Where | The language is set |
| --- | --- |
| A property's editor | In the dropdown under the text, or the row's right-click menu |
| **Animation ▸ Add expression** | In the dropdown under the text, at the right |
| **Window ▸ Expressions** | In the dropdown at the right of the bottom bar |
| An Expression box in the node graph | In the same dropdown, under **Edit expression…** |

A new expression starts in Rhai. **Edit ▸ Settings ▸ Timeline ▸ Editing ▸ New expressions
are written in** changes that. An expression imported from After Effects arrives as JavaScript.

The text is coloured for the language it is set to, and run in that language whatever it
says. `7 / 2` is `3` in Rhai and `3.5` in JavaScript.

## What Rhai can read

`time` is the composition's time in seconds; `layer().time` is the layer's own clock,
counted from its in point.

| | |
| --- | --- |
| **Constants** | `time`, `comp_width`, `comp_height`, `comp_fps`, `num_layers`, `num_markers`, `cut_in`, `cut_out` |
| **Maths** | `sin`, `cos`, `sinh`, `cosh`, `floor`, `ceil`, `round`, `abs`, `clamp`, `noise`, `smoothstep`, `fit`, `fit_clamped`, `fit01`, `look_at(from, to)` |
| **The composition** | `comp().name`, `comp().active_camera` |
| **A layer** | `layer()` for this one, `layer("Name")` for another — `.name`, `.time`, `.x`, `.y`, `.rotation`, `.scale_x`, `.scale_y`, `.scale_z`, `.anchor_x`, `.anchor_y`, `.anchor_z`, `.opacity`, `.z`, `.rotation_x`, `.rotation_y`, `.rotation_z`, `.orientation`, and a camera's `.zoom` and `.point_of_interest` |
| **3D** | `.to_comp(p)`, `.from_comp(p)`, `.to_world(p)`, `.from_world(p)`, `.to_comp_vec(v)`, `.from_comp_vec(v)`, `.to_world_vec(v)`, `.from_world_vec(v)`, `.from_comp_to_surface(p)` on a layer, each with an optional time after the point |

A few to start from:

```rust
time * 90                          // a turn of ninety degrees a second
layer("Sun").x + 20                // twenty pixels behind another layer
noise(time * 2) * 50               // a smooth wander, the same on every run
layer("Marker").to_comp([0, 0, 0])[0]   // on Position X, where a 3D null is on the picture
layer("Marker").to_comp([0, 0, 0])[1]   // and on Position Y
fit(layer().time, 0, 2, 0, 100)    // nought to a hundred over two seconds
```

## What JavaScript can read

Statements, `var`, functions, loops, arrays, objects and strings all work, and arithmetic
on arrays adds and scales them as it does in After Effects.

| | |
| --- | --- |
| **Values** | `time`, `value`, `index`, `inPoint`, `outPoint`, `startTime`, `width`, `height`, `name`, `hasParent`, `parent`, `numKeys` |
| **The composition** | `thisComp`, `comp("Name")` — `.width`, `.height`, `.duration`, `.frameDuration`, `.name`, `.numLayers`, `.layer("Name")`, `.layer(1)`, `.activeCamera` |
| **A layer** | `thisLayer`, or one from `thisComp.layer(…)` — `.name`, `.index`, `.inPoint`, `.outPoint`, `.startTime`, `.hasParent`, `.parent`, `.transform`, `.position`, `.anchorPoint`, `.scale`, `.rotation`, `.xRotation`, `.yRotation`, `.zRotation`, `.orientation`, `.opacity`, `.effect("Name")`, `.sourceRectAtTime(t)`, and a camera's `.pointOfInterest` and `.zoom` |
| **3D** | `toComp(p, t)`, `fromComp(p, t)`, `toWorld(p, t)`, `fromWorld(p, t)`, `toCompVec(v, t)`, `fromCompVec(v, t)`, `toWorldVec(v, t)`, `fromWorldVec(v, t)`, `fromCompToSurface(p, t)` on a layer |
| **An effect's row** | `effect("Name")("Row")` — `.value`, `.valueAtTime(t)`, `.numKeys`, `.key(n)`, `.nearestKey(t)`, `.velocity`, `.speed`, `.wiggle(…)`, `.loopIn(…)`, `.loopOut(…)` |
| **Motion** | `wiggle(freq, amp, octaves, amp_mult, t)`, `valueAtTime(t)`, `posterizeTime(fps)` |
| **Random** | `seedRandom(seed, timeless)`, `random()`, `gaussRandom()`, `noise()` |
| **Interpolation** | `linear`, `ease`, `easeIn`, `easeOut`, `clamp` |
| **Vectors** | `add`, `sub`, `mul`, `div`, `length`, `normalize`, `dot`, `cross`, `lookAt` |
| **Conversion** | `degreesToRadians`, `radiansToDegrees`, `timeToFrames`, `framesToTime`, `rgbToHsl`, `hslToRgb` |
| **JavaScript** | `Math`, `parseInt`, `parseFloat`, `isNaN`, and the array and string methods |

An effect is found by the name on its header, and a row by its label.
`effect("Transform")("Position")` reads both halves of a point as one array.

```js
wiggle(3, 20)                              // wander twenty either side, three times a second
value + [effect("Shake")("Amount"), 0]     // push a position sideways by a slider
seedRandom(index, true); random(0, 360)    // one fixed angle for each layer
thisComp.layer("Marker").toComp([0, 0, 0]) // on Position X and Y, pin a 2D layer to a 3D null
```

- **`value`** is the number the property held when the expression was put on it. On half
  of a combined point, a transform's or an effect's, it is the whole point. On a separated
  row it is that row's number. A list the expression answers with gives each half its own
  number. One saved by an older Lumit reads one number, and a list gives every half its
  first.
- **A property holds keyframes or an expression, not both.** `loopOut()` repeats another
  row's keyframes, as in `effect("Shake")("Amount").loopOut()`, and returns `value` on the
  property it is written on.
- **`wiggle` and `random` are not After Effects' own numbers.** The same settings give the
  same amount of movement along a different path, and the same path on every machine.
- **`sourceRectAtTime`** measures as [Measure](/effects/drivers/measure/) does, in the
  layer's own pixels.

## 3D

A 3D layer's, a camera's and a light's `position`, `anchorPoint`, `scale` and `orientation`
are three numbers. `orientation` reads the turn between keys as the picture has it.
`lookAt(from, to)` gives the Orientation that points a layer's z axis from one point at the
other, with no turn about z.

- **`toComp`** carries a point in the layer's own pixels to the picture. A 3D layer is
  seen through the active camera, and the third number is After Effects' own:
  zoom × (1 − 1 / depth), with depth the distance ahead of the camera along its view.
  A 2D layer gives as many numbers as it was given, and ignores the camera.
- **`fromComp`** goes back. A point with no third number, or a third number of 0, is
  taken one pixel ahead of the camera.
- **`fromCompToSurface`** gives the point on the layer's own plane that the picture shows
  there.
- **`toWorld`** and **`fromWorld`** carry a point to and from the composition's 3D space,
  parents and all, with no camera.
- **The `Vec` forms** carry a direction rather than a point. Through a camera they follow
  After Effects: the third number of `toCompVec([0, 0, 1])` is above 0 while the layer's
  front faces the camera.

:::note[Not yet built]
`lookAt`, `smooth`, markers, text properties, `footage`, `new`, classes and regular
expressions.
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

The editor also follows the Timeline. Select a property that has an expression and the
editor shows it, with the property's name and its value at the playhead underneath. What
is typed is written to the property on clicking away, or with **Ctrl+Enter**.

An entry picked from the list stays in the editor when the selection moves. **New** goes
back to following.

## Notes

- **A Rhai expression that fails reads as −1.** A JavaScript expression that fails leaves
  the property at `value`. The editor shows why, and the row turns red.
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
