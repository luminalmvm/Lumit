---
title: Expression reference
description: Every name a Rhai or a JavaScript expression can read, and the limits each runs under.
sidebar:
  order: 1.5
---

An [expression](/use/expressions/) is written in **Rhai** or in **JavaScript**. The two
read different names.

## Rhai

A Rhai expression is one expression, not a script of statements.

### Values

| Name | Is |
| --- | --- |
| `time` | The composition's time, in seconds. |
| `comp_width`, `comp_height` | The composition's size in pixels, as whole numbers. |
| `comp_fps` | The composition's frame rate, cut down to a whole number. |
| `num_layers` | How many layers the composition has. |
| `num_markers` | How many markers the composition has. |
| `cut_in`, `cut_out` | The layer's in point and out point, in seconds. |

`time` is always there. The rest need a composition, and `cut_in` and `cut_out` a layer.

### Maths

| Function | Returns |
| --- | --- |
| `sin(x)`, `cos(x)` | The sine and cosine of `x`, in radians. |
| `sinh(x)`, `cosh(x)` | The hyperbolic sine and cosine. |
| `floor(x)`, `ceil(x)`, `round(x)` | `x` rounded down, up, or to the nearest whole number. |
| `abs(x)` | `x` without its sign. |
| `clamp(x, a, b)` | `x` held between `a` and `b`. |
| `noise(x)` | A smooth wander between 0 and 1. The same `x` gives the same number on every run. |
| `smoothstep(x)` | `x * x * (3 - 2 * x)`. |
| `fit(x, old_min, old_max, new_min, new_max)` | `x` moved from one range to another. |
| `fit_clamped(x, old_min, old_max, new_min, new_max)` | The same, held inside the new range. |
| `fit01(x, min, max)` | `x` moved from 0 to 1 onto `min` to `max`. |
| `look_at(from, to)` | The Orientation, `[x, y, z]`, that turns a layer's z axis from one point to look at the other. |
| `to_f64(x)` | `x` as a number. |

Each takes a whole number or a fraction. Rhai's own standard functions are there as well.
`eval` and `import` are switched off, and `print` writes nowhere.

### The composition and its layers

| Name | Is |
| --- | --- |
| `comp()` | The composition. `comp().name` is its name, and `comp().active_camera` the Camera layer it is seen through. |
| `layer()` | The layer the expression is on. |
| `layer("Name")` | The first layer of the composition with that name. |

A layer answers to:

| Name | Is |
| --- | --- |
| `.name` | The layer's name. |
| `.time` | The layer's own clock: `time` less its in point. |
| `.x`, `.y`, `.z` | Position. |
| `.rotation`, `.rotation_z` | Rotation. |
| `.rotation_x`, `.rotation_y` | X Rotation and Y Rotation. |
| `.orientation` | Orientation, `[x, y, z]`, as it turns between its keyframes. |
| `.scale_x`, `.scale_y`, `.scale_z` | Scale. |
| `.anchor_x`, `.anchor_y`, `.anchor_z` | Anchor point. |
| `.opacity` | Opacity. |
| `.zoom` | A camera's zoom. |
| `.point_of_interest` | A camera's point of interest, `[x, y, z]`. |

A property read this way is read at that layer's own time, keyframes and expression
included. A name that matches no layer reads −1 for every number, and `Invalid Layer
Reference` for `.name`.

### Points between spaces

Each is called on a layer, takes a point or a direction as a list, and takes a time in
seconds after it to read the layers at another moment.

| Function | Carries |
| --- | --- |
| `.to_comp(p)`, `.from_comp(p)` | A point in the layer's pixels to where it is on the picture, and back. |
| `.to_world(p)`, `.from_world(p)` | A point in the layer's pixels to the composition's 3D space, and back. |
| `.to_comp_vec(v)`, `.from_comp_vec(v)` | A direction on the layer to one on the picture, and back. |
| `.to_world_vec(v)`, `.from_world_vec(v)` | A direction on the layer to one in the composition's 3D space, and back. |
| `.from_comp_to_surface(p)` | A point on the picture to where it meets the layer's own plane, `[x, y]`. |

Where there is no answer the list comes back empty.

### In a node graph

| Name | Is |
| --- | --- |
| `input("Name")` | The value of the graph's Input with that id or label. A picture Input, or a name the graph does not have, reads −1. |
| `input_1`, `input_2`, … | The inputs of an [Expression](/effects/drivers/expression/) box. |

### What a Rhai expression comes to

A property takes one number. `true` reads as 1 and `false` as 0. Anything else, and an
expression that fails, reads as −1.

### Rhai limits

| Limit | |
| --- | --- |
| Operations in one run | 100,000 |
| A string | 1 MB |
| An array | 16,384 items |
| A map | 16,384 entries |
| Properties reading one another | 100 deep |

## JavaScript

### The language

Statements, `var`, `let` and `const`, functions and arrow functions, `if`, `switch`, `for`,
`for…of`, `for…in`, `while`, `do…while`, `break`, `continue`, `return`, `throw`, `try`
with `catch` and `finally`, arrays, objects, strings, template strings, the spread `...`,
`?.`, `??`, `typeof` and `void`.

`new`, `class`, `delete`, `await`, `yield`, `import`, generators and regular expressions
are refused, with the line they are on.

Arithmetic on arrays works as it does in After Effects: `[10, 20] + [1, 2]` is `[11, 22]`.

### Values

| Name | Is |
| --- | --- |
| `time` | The composition's time, in seconds. |
| `value` | What the property held before the expression was put on it. Both halves of a point, as an array. |
| `thisComp` | The composition. |
| `thisLayer` | The layer the expression is on. |
| `thisProperty` | The property the expression is on. |
| `index`, `inPoint`, `outPoint`, `startTime`, `width`, `height`, `name`, `hasParent`, `parent`, `transform`, `position`, `anchorPoint`, `scale`, `rotation`, `xRotation`, `yRotation`, `zRotation`, `orientation`, `opacity` | The same as on `thisLayer`. |
| `numKeys`, `velocity`, `speed` | The same as on `thisProperty`. |
| `colorDepth` | Always 32. |
| `Math`, `undefined`, `NaN`, `Infinity` | As in JavaScript. |
| `input_1`, `input_2`, … | The inputs of an [Expression](/effects/drivers/expression/) box. |

### A composition

`thisComp`, or `comp("Name")` for another composition of the project.

| Name | Is |
| --- | --- |
| `.width`, `.height` | Its size in pixels. |
| `.duration` | Its length, in seconds. |
| `.frameDuration` | One frame, in seconds. |
| `.name` | Its name. |
| `.numLayers` | How many layers it has. |
| `.layer("Name")`, `.layer(n)` | A layer by name, or by number from 1 at the top. |
| `.activeCamera` | The Camera layer the composition is seen through. A composition with no camera fails the expression. |
| `.pixelAspect` | Always 1. |
| `.displayStartTime` | Always 0. |

### A layer

| Name | Is |
| --- | --- |
| `.name` | Its name. |
| `.index` | Its number, from 1 at the top. |
| `.inPoint`, `.outPoint` | Its in and out points, in seconds. |
| `.startTime` | Where its own time zero sits in the composition, in seconds. |
| `.width`, `.height` | The composition's size. |
| `.hasParent`, `.parent` | Whether it has a parent, and the parent layer or `null`. |
| `.active`, `.enabled` | Its visibility switch. |
| `.position`, `.anchorPoint`, `.scale` | Each a property of two numbers, or three on a 3D layer, a camera or a light. |
| `.rotation`, `.zRotation`, `.xRotation`, `.yRotation`, `.opacity` | Each a property of one number. |
| `.orientation` | A property of three numbers. |
| `.pointOfInterest`, `.zoom` | On a Camera layer. |
| `.transform`, `.cameraOption` | The same properties, as a group. |
| `.effect("Name")`, `.effect(n)` | An effect, by the name on its header or by its number from 1. |
| `.sourceRectAtTime(t)` | The box round what the layer shows at `t` seconds, or now: its `.top`, `.left`, `.width` and `.height`, in the layer's own pixels. |

### Points between spaces

Each is called on a layer, or with no dot in front on the layer the expression is on. `t`
is a time in seconds, and `time` when left out.

| Function | Carries |
| --- | --- |
| `toComp(p, t)`, `fromComp(p, t)` | A point in the layer's pixels to where it is on the picture, and back. |
| `toWorld(p, t)`, `fromWorld(p, t)` | A point in the layer's pixels to the composition's 3D space, and back. |
| `toCompVec(v, t)`, `fromCompVec(v, t)` | A direction on the layer to one on the picture, and back. |
| `toWorldVec(v, t)`, `fromWorldVec(v, t)` | A direction on the layer to one in the composition's 3D space, and back. |
| `fromCompToSurface(p, t)` | A point on the picture to where it meets the layer's own plane. |

A layer scaled to nothing, or seen edge on, has no way back and fails the expression. See
[Expressions](/use/expressions/#3d).

### An effect

`effect("Name")` reads an effect of the layer the expression is on. The name is matched
exactly first, then without regard to case.

| Name | Is |
| --- | --- |
| `.name` | The name on its header. |
| `.active`, `.enabled` | Whether it is switched on. |
| `effect("Name")("Row")`, `effect("Name")(n)` | One of its rows, by label or by number from 1. |

`"Position"` names both halves of a point. A tickbox reads as 1 or 0, a menu as its
choice counted from 1, and a colour as four numbers.

### A property

A transform property, an effect's row, or `thisProperty`.

| Name | Is |
| --- | --- |
| `.value` | Its value at `time`. |
| `.valueAtTime(t)` | Its value at `t` seconds. |
| `.velocity`, `.velocityAtTime(t)` | Its rate of change per second. |
| `.speed`, `.speedAtTime(t)` | The size of that rate, as one number. |
| `.numKeys` | How many keyframes it has. |
| `.key(n)` | Keyframe `n`, from 1: its `.time`, `.value` and `.index`. |
| `.nearestKey(t)` | The keyframe nearest `t`, the same way. |
| `.wiggle(freq, amp, octaves, amp_mult, t)` | See below. `.temporalWiggle` is the same. |
| `.loopIn(type, numKeyframes)`, `.loopOut(type, numKeyframes)` | Repeats the keyframes before the first or after the last. `numKeyframes` limits the stretch repeated to that many spans of keyframes beside it; 0, the default, is all of them. |
| `.loopInDuration(type, duration)`, `.loopOutDuration(type, duration)` | The same, with the stretch given in seconds. |
| `.smooth()` | The value, unchanged. |

`valueAtTime`, `velocityAtTime`, `speedAtTime`, `key`, `nearestKey`, `wiggle` and the four
loops can also be called with no dot in front, on the property the expression is on.

The loop `type` is `"cycle"`, `"pingpong"`, `"offset"` or `"continue"`. With none given it
is `"cycle"`.

### Motion

| Function | Returns |
| --- | --- |
| `wiggle(freq, amp, octaves, amp_mult, t)` | The property's value, wandering `amp` either side of it `freq` times a second. `octaves` is 1 to 8 and starts at 1, `amp_mult` starts at 0.5, and `t` at `time`. |
| `posterizeTime(fps)` | Holds `time` to steps of `1 / fps` for the rest of the expression, and returns it. |

### Random numbers

| Function | Returns |
| --- | --- |
| `random()`, `random(max)`, `random(min, max)` | A number from 0 to 1, from 0 to `max`, or from `min` to `max`. Arrays give an array. |
| `gaussRandom()`, `gaussRandom(max)`, `gaussRandom(min, max)` | The same range, with the numbers gathered round its middle. |
| `seedRandom(seed, timeless)` | Nothing. Starts the numbers again from `seed`. With `timeless` true they no longer change with time. |
| `noise(x)`, `noise([x, y, z])` | Smooth noise at that place. |

The numbers depend on the layer, the property and the time, and on nothing else. The same
project gives the same numbers on every machine.

### Blending

| Function | Returns |
| --- | --- |
| `linear(t, a, b)` | `a` at `t` of 0 and `b` at `t` of 1, in a straight line. |
| `linear(t, tMin, tMax, a, b)` | The same, as `t` runs from `tMin` to `tMax`. |
| `ease(…)` | The same two forms, slow at both ends. |
| `easeIn(…)` | Slow at the start. |
| `easeOut(…)` | Slow at the end. |
| `clamp(value, a, b)` | `value` held between `a` and `b`. |

`t` outside its range is held at the nearer end. `a`, `b` and `value` can be arrays.

### Vectors

| Function | Returns |
| --- | --- |
| `add(a, b)`, `sub(a, b)`, `mul(a, b)`, `div(a, b)` | The same as `+`, `-`, `*` and `/`. |
| `length(a)` | The length of `a`. |
| `length(a, b)` | The distance from `a` to `b`. |
| `normalize(a)` | `a` at a length of 1. |
| `dot(a, b)` | The dot product. |
| `cross(a, b)` | The cross product, as three numbers. |
| `lookAt(from, to)` | The Orientation that turns a layer's z axis from one point to look at the other. |

### Conversion

| Function | Returns |
| --- | --- |
| `degreesToRadians(d)`, `radiansToDegrees(r)` | The angle in the other unit. |
| `timeToFrames(t, fps)` | `t` seconds as a whole frame. `t` starts at `time` and `fps` at the composition's. |
| `framesToTime(frames, fps)` | That many frames, in seconds. |
| `rgbToHsl([r, g, b, a])` | `[h, s, l, a]`. |
| `hslToRgb([h, s, l, a])` | `[r, g, b, a]`. |
| `parseFloat(text)`, `parseInt(text)` | The number at the front of the text, or `NaN`. |
| `isNaN(x)`, `isFinite(x)` | `true` or `false`. |
| `Number(x)`, `String(x)`, `Boolean(x)` | `x` as that kind of value. |

### JavaScript's own

| On | Names |
| --- | --- |
| `Math` | `PI`, `E`, `LN2`, `LN10`, `LOG2E`, `LOG10E`, `SQRT2`, `SQRT1_2`, `abs`, `acos`, `asin`, `atan`, `atan2`, `cbrt`, `ceil`, `cos`, `exp`, `floor`, `hypot`, `log`, `log2`, `log10`, `max`, `min`, `pow`, `random`, `round`, `sign`, `sin`, `sqrt`, `tan`, `trunc` |
| An array | `length`, `push`, `pop`, `shift`, `unshift`, `slice`, `concat`, `indexOf`, `includes`, `join`, `reverse`, `map`, `filter`, `forEach`, `some`, `every`, `find`, `findIndex`, `reduce`, `sort`, `toString` |
| A string | `length`, `toUpperCase`, `toLowerCase`, `trim`, `indexOf`, `lastIndexOf`, `includes`, `startsWith`, `endsWith`, `charAt`, `charCodeAt`, `slice`, `substring`, `substr`, `split`, `replace`, `repeat`, `padStart`, `padEnd`, `toString` |
| A number | `toFixed`, `toString` |

`Math.random()` draws from the same numbers as `random()`.

### What a JavaScript expression comes to

A property takes one number. From an array it takes its own: the first for the x half of a
point, the second for the y half. An expression that fails, or comes to something that is
not a number, leaves the property at `value`.

### JavaScript limits

| Limit | |
| --- | --- |
| Steps in one run, in a preview or an export | 2,000,000 |
| Steps in one run, when a panel reads the value | 100,000 |
| Functions calling functions | 64 deep |
| An array, or an object | 16,384 items |
| A string | 1 MB |
| The text of one expression | 256 KB |
| Properties reading one another | 100 deep |

### Not built

`smooth`, which returns the value unchanged, markers, text properties, `footage`, and
reading a group by its internal name, as in `thisLayer("ADBE Transform Group")`.

## Related

- [Expressions](/use/expressions/)
- [Expression](/effects/drivers/expression/)
- [Custom controls](/effects/controls/custom-controls/)
