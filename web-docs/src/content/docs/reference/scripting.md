---
title: Scripting reference
description: Every command a script or MCP can run, and its arguments.
sidebar:
  order: 4
---

<!-- Written by `cargo test -p lumit-script regenerate_scripting_reference -- --ignored`
from the command table in crates/lumit-script. Edit the table, not this page. -->

Every command a [script](/use/scripts/) or
[MCP](/use/command-line/#let-another-program-edit-a-project) can run, with its arguments. A
script also runs one as `lumit.command("layer.add", { ... })`.

A time is a frame on the comp's rate, or exact seconds as `{ "num": 1001, "den": 24000 }`.

## project.info

Where the project is saved, how much it holds, and whether undo and redo have anything to do.

Script: `lumit.project.info()`. MCP tool: `project_info`.

No arguments.

## project.save

Save the project, where it was last saved or to a new path.

Script: `lumit.project.save({ ... })`. MCP tool: `project_save`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `path` | string |  |  |

## undo

Undo the last step.

MCP tool: `undo`. A script cannot run it.

No arguments.

## redo

Redo the last undone step.

MCP tool: `redo`. A script cannot run it.

No arguments.

## comp.list

Every comp in the project, with its settings.

Script: `lumit.comp.list()`. MCP tool: `comp_list`.

No arguments.

## comp.get

One comp's settings and its layers.

Script: `lumit.comp.get({ ... })`. MCP tool: `comp_get`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |

## comp.new

Make a comp, 1920x1080 at 60 fps for 30 seconds unless told otherwise.

Script: `lumit.comp.new({ ... })`. MCP tool: `comp_new`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `name` | string |  |  |
| `width` | integer (16 to 16384) |  |  |
| `height` | integer (16 to 16384) |  |  |
| `frame_rate` | integer (1 to 999) or `{num, den}` |  | 1 to 999 frames a second |
| `duration` | integer or `{num, den}` |  | At most 3 hours |
| `background` | list of 4 numbers |  | Scene-linear RGBA |
| `folder` | string or null |  | The folder to file it in, by id or name, or null for the top of the panel. Left out, it goes in the Compositions folder |

## comp.set

Change a comp's name, size, frame rate, duration or background.

Script: `lumit.comp.set({ ... })`. MCP tool: `comp_set`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `name` | string |  |  |
| `width` | integer (16 to 16384) |  |  |
| `height` | integer (16 to 16384) |  |  |
| `frame_rate` | integer (1 to 999) or `{num, den}` |  | 1 to 999 frames a second |
| `duration` | integer or `{num, den}` |  | At most 3 hours |
| `background` | list of 4 numbers |  | Scene-linear RGBA |

## comp.precompose

Move layers into a new comp, and put that comp in their place as one layer.

Script: `lumit.comp.precompose({ ... })`. MCP tool: `comp_precompose`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layers` | list of strings | yes | The layers' ids, or their names in the comp |
| `name` | string |  | The new comp's name. Pre-comp and a number if left out |
| `leave_attributes` | boolean |  | For one layer: leave its transform, effects and masks on the new layer, and move only its source |

## comp.open

Show a comp in the Viewer.

Script: `lumit.comp.open({ ... })`. MCP tool: `comp_open`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |

## comp.active

The comp open in the Viewer, if there is one.

Script: `lumit.comp.active()`. MCP tool: `comp_active`.

No arguments.

## comp.frame

Render a frame of a comp to a PNG, and answer its path, width and height.

Script: `lumit.comp.frame({ ... })`. MCP tool: `comp_frame`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `time` | integer or `{num, den}` |  | The frame, 0 when left out |
| `scale` | number (over 0, at most 1) |  | Of the comp's size, 1 when left out |
| `out` | string |  | The .png file to write. Without it, a numbered file beside the project |

## item.list

Every item in the project panel: comps, footage, folders and solids, with the folder each is in, and the size, rate and length of footage and solids.

Script: `lumit.item.list()`. MCP tool: `item_list`.

No arguments.

## item.set

Rename an item, file it in a folder, set the rate a footage item plays at, or set its comment.

Script: `lumit.item.set({ ... })`. MCP tool: `item_set`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `item` | string | yes | The item's id or name |
| `name` | string |  |  |
| `folder` | string or null |  | The folder's id or name, or null for none |
| `frame_rate` | integer (1 to 999) or `{num, den}` or null |  | 1 to 999 frames a second, or a video's own rate |
| `comment` | string |  | Empty for none |

## folder.new

Make a folder in the project panel.

Script: `lumit.folder.new({ ... })`. MCP tool: `folder_new`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `name` | string |  |  |
| `parent` | string |  | The folder to make it in, by id or name |

## layer.add

Add a null, camera, solid, footage, precomp, text, shape, adjustment or light layer to a comp.

Script: `lumit.layer.add({ ... })`. MCP tool: `layer_add`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `kind` | `"null"`, `"camera"`, `"solid"`, `"footage"`, `"precomp"`, `"text"`, `"shape"`, `"adjustment"`, `"light"` | yes |  |
| `item` | string |  | For footage: the footage item's id or name. For a precomp: the comp's |
| `name` | string |  |  |
| `index` | integer (at least 0) |  | Where in the stack, 0 is the top |
| `colour` | list of 3 numbers (0 to 1) |  | For a solid: its colour as a colour picker shows it, 0 to 1. White if left out |
| `width` | integer (1 to 16384) |  | For a solid, the comp's if left out |
| `height` | integer (1 to 16384) |  |  |
| `fields` | object |  | Any of the layer's fields as a .lum file keeps them, laid over the new layer as layer.write lays them. A text layer turns about the middle of its words unless these set its anchor |

## shape.add

Add a shape layer drawn from paths, each item in the form a .lum file keeps it: path, fill, stroke, stroke_width, opacity, gradient and the rest.

Script: `lumit.shape.add({ ... })`. MCP tool: `shape_add`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `name` | string |  |  |
| `index` | integer (at least 0) |  | Where in the stack, 0 is the top |
| `items` | list of object | yes | The shape items as a .lum file keeps them. Each path is {vertices:[{pos:[x,y],tan_in:[x,y],tan_out:[x,y]}],closed}, in comp pixels with tangents relative to their vertex. Colours are linear-light RGBA. An id and a name are made for any item without one |

## layer.read

One layer whole, in the form a .lum file keeps it, to change and give back to layer.write.

Script: `lumit.layer.read({ ... })`. MCP tool: `layer_read`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string | yes | The layer's id, or its name in the comp |

## layer.write

Change any part of a layer, in the form a .lum file keeps it: its kind and contents, text, transform, masks, effects, styles, paint, switches and the rest.

Script: `lumit.layer.write({ ... })`. MCP tool: `layer_write`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string | yes | The layer's id, or its name in the comp |
| `fields` | object | yes | Laid over the layer as JSON Merge Patch does: objects merge key by key, null takes a field away, anything else replaces it, and a choice such as {"Keyframed": [...]} replaces another such as {"Static": 1}. Times are exact seconds as [num, den], colours linear-light RGBA. The layer keeps its id and its place |

## comp.write

Change a comp's motion blur, lighting or node graph, in the form a .lum file keeps them.

Script: `lumit.comp.write({ ... })`. MCP tool: `comp_write`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `fields` | object | yes | Each laid over the comp's own as JSON Merge Patch does |
| `fields.motion_blur` | object |  |  |
| `fields.lighting` | object |  |  |
| `fields.graph` | object |  |  |

## layer.composite

Set how a layer meets the layers below: its blend mode, and a track matte from another layer of its comp.

Script: `lumit.layer.composite({ ... })`. MCP tool: `layer_composite`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string | yes | The layer's id, or its name in the comp |
| `blend` | string |  | A blend mode as a .lum file names it: Normal, Add, Screen, Multiply, Overlay, SoftLight and the rest |
| `matte` | string or null |  | The matte layer's id or name, or null for none |
| `luma` | boolean |  | Matte by brightness rather than alpha |
| `inverted` | boolean |  |  |

## layer.get

One layer: its span, parent, switches and transform.

Script: `lumit.layer.get({ ... })`. MCP tool: `layer_get`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string | yes | The layer's id, or its name in the comp |

## layer.set

Change a layer's name, parent, span, switches, 3D switch, rotation order or place in the stack.

Script: `lumit.layer.set({ ... })`. MCP tool: `layer_set`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string | yes | The layer's id, or its name in the comp |
| `name` | string |  |  |
| `parent` | string or null |  | The parent's id or name, or null for none. The layer stays where it is at the comp's start |
| `jump` | boolean |  | With parent: leave the transform as it is, so the layer moves with its new parent |
| `in` | integer or `{num, den}` |  | A frame on the comp's rate, or exact seconds |
| `out` | integer or `{num, den}` |  | A frame on the comp's rate, or exact seconds |
| `start_offset` | integer or `{num, den}` |  | A frame on the comp's rate, or exact seconds |
| `three_d` | boolean |  |  |
| `index` | integer (at least 0) |  | Where in the stack, 0 is the top |
| `two_node` | boolean |  | For a camera, whether it aims at its point of interest |
| `after_effects_order` | boolean |  | Turn X, Y and Z Rotation in After Effects' order rather than Lumit's. The rotations are rewritten so the layer keeps its pose |
| `switches` | object |  |  |
| `switches.visible` | boolean |  |  |
| `switches.audible` | boolean |  |  |
| `switches.solo` | boolean |  |  |
| `switches.locked` | boolean |  |  |
| `switches.shy` | boolean |  |  |
| `switches.guide` | boolean |  |  |
| `switches.motion_blur` | boolean |  |  |
| `switches.fx` | boolean |  |  |
| `switches.accepts_lights` | boolean |  |  |

## layer.delete

Delete a layer.

Script: `lumit.layer.delete({ ... })`. MCP tool: `layer_delete`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string | yes | The layer's id, or its name in the comp |

## layer.keys

Set one transform or camera property's whole animation: a value, a list of keys, or an expression.

Rotation turns about Z.

Script: `lumit.layer.keys({ ... })`. MCP tool: `layer_keys`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string | yes | The layer's id, or its name in the comp |
| `property` | `"rotation"`, `"rotation_x"`, `"rotation_y"`, `"opacity"`, `"zoom"`, `"focus_distance"`, `"aperture"`, `"anchor"`, `"scale"`, `"position"`, `"point_of_interest"`, `"orientation"` | yes |  |
| `value` | number or list of 2 numbers or list of 2 to 3 numbers or list of 3 numbers |  | Z is left as it is when there are two; X, Y and Z, in After Effects' order |
| `keys` | list |  |  |
| `keys[].time` | integer or `{num, den}` | yes | A frame on the comp's rate, or exact seconds |
| `keys[].value` | number or list of 2 numbers or list of 2 to 3 numbers or list of 3 numbers | yes | Z is left as it is when there are two; X, Y and Z, in After Effects' order |
| `keys[].interp` | `"linear"`, `"hold"` |  |  |
| `keys[].in` | `"linear"`, `"hold"`, `"auto"`, `"clamped"` or `{speed, influence}` or list of `{speed, influence}` |  | Linear, hold, auto or clamped, or an ease, or a list with an ease for each value |
| `keys[].in.speed` | number |  | Units a second |
| `keys[].in.influence` | number (0.1 to 100) |  | Per cent of the span |
| `keys[].out` | `"linear"`, `"hold"`, `"auto"`, `"clamped"` or `{speed, influence}` or list of `{speed, influence}` |  | Linear, hold, auto or clamped, or an ease, or a list with an ease for each value |
| `keys[].out.speed` | number |  | Units a second |
| `keys[].out.influence` | number (0.1 to 100) |  | Per cent of the span |
| `expression` | string |  | A JavaScript expression, as After Effects writes them |

Needs one of `value`, `keys` or `expression`.

## markers.set

Replace a comp's markers, or a layer's when a layer is named.

Script: `lumit.markers.set({ ... })`. MCP tool: `markers_set`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string |  | The layer's id, or its name in the comp |
| `markers` | list | yes |  |
| `markers[].id` | string |  | An existing marker's id, to keep what it was |
| `markers[].time` | integer or `{num, den}` | yes | A frame on the comp's rate, or exact seconds |
| `markers[].label` | string |  |  |
| `markers[].colour` | integer (0 to 255) |  |  |
| `markers[].duration` | integer |  | Frames it spans |
| `markers[].chapter` | boolean |  | Whether it starts a chapter |

## effect.list

The effects on a layer, top first, with their controls.

Script: `lumit.effect.list({ ... })`. MCP tool: `effect_list`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string | yes | The layer's id, or its name in the comp |

## effect.get

One effect on a layer, with every control it has.

Script: `lumit.effect.get({ ... })`. MCP tool: `effect_get`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string | yes | The layer's id, or its name in the comp |
| `effect` | string or integer (at least 0) | yes | The effect's id or name, or its place in the stack, 0 the top |

## effect.add

Add an effect to the bottom of a layer's stack, by Lumit's name for it or the name an imported project gives it.

Script: `lumit.effect.add({ ... })`. MCP tool: `effect_add`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string | yes | The layer's id, or its name in the comp |
| `effect` | string | yes | Lumit's id or name for the effect, or the match name or name an imported project gives it, such as ADBE Gaussian Blur 2 |
| `name` | string |  | The name it shows |

## effect.set

Switch an effect on or off, rename it, or set one of its controls: a value, a list of keys, or an expression.

Script: `lumit.effect.set({ ... })`. MCP tool: `effect_set`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string | yes | The layer's id, or its name in the comp |
| `effect` | string or integer (at least 0) | yes | The effect's id or name, or its place in the stack, 0 the top |
| `enabled` | boolean |  |  |
| `name` | string or null |  | The name it shows, or null for the effect's own |
| `param` | string |  | A control's id or name. After Effects' match name for one, such as ADBE Gaussian Blur 2-0001, takes After Effects' values and converts them as importing does |
| `value` | any |  | A number, a list for a point or a colour, true or false for a switch, or an option's number |
| `keys` | list |  |  |
| `keys[].time` | integer or `{num, den}` | yes | A frame on the comp's rate, or exact seconds |
| `keys[].value` | number or list of 1 to 4 numbers | yes |  |
| `keys[].interp` | `"linear"`, `"hold"` |  |  |
| `keys[].in` | `"linear"`, `"hold"`, `"auto"`, `"clamped"` or `{speed, influence}` or list of `{speed, influence}` |  | Linear, hold, auto or clamped, or an ease, or a list with an ease for each value |
| `keys[].in.speed` | number |  | Units a second |
| `keys[].in.influence` | number (0.1 to 100) |  | Per cent of the span |
| `keys[].out` | `"linear"`, `"hold"`, `"auto"`, `"clamped"` or `{speed, influence}` or list of `{speed, influence}` |  | Linear, hold, auto or clamped, or an ease, or a list with an ease for each value |
| `keys[].out.speed` | number |  | Units a second |
| `keys[].out.influence` | number (0.1 to 100) |  | Per cent of the span |
| `expression` | string |  | A JavaScript expression, as After Effects writes them |

## footage.import

Import files as footage items.

Script: `lumit.footage.import({ ... })`. MCP tool: `footage_import`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `paths` | list of strings | yes |  |

## lottie.import

Import a Lottie (.json or .lottie) as comps, and say what didn't come across.

Script: `lumit.lottie.import({ ... })`. MCP tool: `lottie_import`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `path` | string | yes | The .json or .lottie file |

## export.start

Start exporting a comp.

Script: `lumit.export.start({ ... })`. MCP tool: `export_start`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `out` | string |  | The file to write |

## export.vector

Export a comp as a Lottie, a dotLottie or an animated SVG, and say what won't come across.

Script: `lumit.export.vector({ ... })`. MCP tool: `export_vector`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `out` | string | yes | The file to write: .json, .lottie or .svg |
| `format` | `"lottie"`, `"dotlottie"`, `"svg"` |  | Without it, from the file's extension |
| `text` | `"shapes"`, `"text"` |  | Text as outlines (the default) or as text naming its font |
| `minify` | boolean |  |  |

## export.status

How an export is going.

Script: `lumit.export.status({ ... })`. MCP tool: `export_status`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `job` | integer (at least 0) | yes |  |

## export.cancel

Stop an export.

Script: `lumit.export.cancel({ ... })`. MCP tool: `export_cancel`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `job` | integer (at least 0) | yes |  |

## comp.controls

A comp's controls and their values, after setting the template's own values when told to.

Script: `lumit.comp.controls({ ... })`. MCP tool: `comp_controls`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `set` | object |  | Control, by id or label, to its value: a number, a point as [x, y], a colour as #RRGGBB or [r, g, b, a] scene-linear, true or false, a menu option's name or number from 0, text, a font as {family, face}, or a footage item's or comp's id or name |

## layer.values

The values a copy of a graphics template has: its own, set or fed by a data row, or the template's.

Script: `lumit.layer.values({ ... })`. MCP tool: `layer_values`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `layer` | string | yes | The layer's id, or its name in the comp |
| `clip` | string |  | For a copy that is a clip on a Sequence layer: the clip's id |
| `set` | object |  | Control, by id or label, to its value: a number, a point as [x, y], a colour as #RRGGBB or [r, g, b, a] scene-linear, true or false, a menu option's name or number from 0, text, a font as {family, face}, or a footage item's or comp's id or name |
| `reset` | boolean or list of strings |  | Controls that follow the template again, by id or label, or true for all |
| `row` | integer (at least 1) or null |  | The data row to follow, counting from 1, or null to stop |

## data.rows

The data linked to a graphics template, after linking a CSV, TSV or JSON file or saying what its columns feed.

Script: `lumit.data.rows({ ... })`. MCP tool: `data_rows`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `file` | string |  | A CSV, TSV or JSON file to link. Its columns feed the controls of their names |
| `feeds` | object |  | Column name to what it feeds: a control's id or label, length, file name, or null for nothing |

## data.export

Export a graphics template once for each row of its data.

Script: `lumit.data.export({ ... })`. MCP tool: `data_export`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `rows` | list of 2 integers (at least 1) |  | The first and last row, counting from 1. All of them if left out |
| `out` | string |  | The files' name, which may hold {Column}, {row}, {comp} and {date} and a folder |
| `layer` | string |  | A copy's layer, by id or name: the comp it sits in is exported with the copy following each row. The template on its own if left out |
| `clip` | string |  | With layer, for a copy that is a clip: the clip's id |

## template.list

The graphics templates in the library.

Script: `lumit.template.list()`. MCP tool: `template_list`.

No arguments.

## template.place

Bring a graphics template into the project, and place it in a comp when told to.

Script: `lumit.template.place({ ... })`. MCP tool: `template_place`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `template` | string | yes | Its name in the library, or a .lumt file |
| `comp` | string |  | The comp to place it in, by id or name, as a Precomp layer |
| `at` | integer or `{num, den}` |  | Where the layer starts. The start if left out |

## template.save

Save a comp to the library as a graphics template.

Script: `lumit.template.save({ ... })`. MCP tool: `template_save`.

| Argument | Type | Required | Meaning |
| --- | --- | --- | --- |
| `comp` | string | yes | The comp's id or name |
| `name` | string |  | The comp's name if left out |
| `group` | string |  | The folder in the library, parted by /. The top if left out |
| `pack_fonts` | boolean |  | Put the font files its text names in the file |
