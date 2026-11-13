# Glossary

Every identifier, comment, UI string, commit and doc uses these terms. A new concept gets
its entry here first. AE and Vegas names are noted only to help people arriving from them.

## 1. Project structure

| Term | Meaning |
|---|---|
| **Project** | The whole document, saved as a `.lum` file. One open at a time |
| **Asset** | Anything in the Project panel: footage, audio, sequences, stills, comps |
| **Footage item** | An asset pointing at a media file. Lumit never changes the file |
| **Interpretation** | How a footage item's file is read: its alpha, frame rate, fields, pixel aspect ratio, loop and colour. Kept on the item. The verb is **interpret** |
| **3D model** | A footage item pointing at a glTF, GLB, FBX or OBJ file: meshes, materials and pictures. Never just *model*, which a **Model pack** holds |
| **File layer** | One layer of a layered image file such as a PSD or an Illustrator document, which a footage item can read on its own |
| **Audio item** | An asset pointing at an audio file |
| **Audio stream** | One of the sounds a media file carries side by side, such as the game and the microphone of a screen recording. A file arrives with all of them mixed on one layer or clip unless Settings says otherwise. To **split** gives each a row of its own, to **merge** puts them back on one. Not *audio track* |
| **Transcript** | The words said in a footage item's sound, each with when it starts and ends, how sure the hearing was and who said it, in the item's own time. Made by **transcribing** |
| **Speaker** | Someone who speaks in a transcript. One job that hears several items gives the same person the same speaker in each |
| **Packed project** | A `.lum` that also carries its footage files inside it. The verbs are **pack** and **unpack** |
| **Shared project** | A project several people edit at once. The **host** shares it and owns the file, the **guests** join with an **invite**, which is written as an **invite link**. One that is **kept online** is held by Lumit's server, which cannot read it and puts the edits in order, so nobody's machine has to be on |
| **Relay** | A server that passes a shared project's edits between a host and guests who cannot reach each other. It cannot read them. Somebody's own, or **Lumit's relay**, which comes with Lumit Pro |
| **Shared preview** | An address where an export of a comp is watched in a browser and commented on, frame by frame. The window that lists them is **Previews**. The people watching are **reviewers**, never *viewers*, and each upload to the link is a **version**. Comes with Lumit Pro |
| **Folder** | A group in the Project panel. Not *bin* |
| **Comment** | A line of text kept on an asset. Scripts set and read it, as After Effects' item comment |
| **Composition (comp)** | Resolution, frame rate, duration, background, and a layer stack or a node graph |
| **Node graph** | A comp whose picture is made by nodes and wires instead of layers |
| **Multicam comp** | A comp whose Sequence layers are the **angles** of one shoot, each holding one camera's or recorder's files at the moments they were filmed. Made from footage by **New multicam comp**, which **syncs** the files by their sound, their timecode, their In marks or their first markers |
| **Angle** | One Sequence layer of a multicam comp, numbered from the top. A **sound angle** is an audio-only one, such as a recorder |
| **Graphics template** | A comp with **controls**, saved to the library as a `.lumt` file to place in any project. Never just *template*, which Settings uses for the project a new one starts from |
| **Control** | A row inside a comp that every Precomp layer or clip placing it can set for itself. To **add to controls** makes a row one. Each placing is a **copy**, and a copy that has not set a control follows the template |
| **Data** | Rows from a CSV, TSV or JSON file feeding a graphics template's controls, one version for each row. A copy can **follow** a row |
| **Intro**, **Outro** | The stretch at a graphics template's start and at its end that keeps its speed when a copy is made longer or shorter |

## 2. Layers

A **layer** is one row in a comp. The bottom layer draws first and each one above
composites over it.

| Layer | Meaning |
|---|---|
| **Footage layer** | One footage item. Has Retime |
| **Sequence layer** | Clips cut back to back on one row, each with its own source, trim and Retime. The layer's own effects apply to the whole output |
| **Precomp layer** | Another comp as a source. The verb is **precompose**. Has Retime |
| **Solid**, **Text**, **Shape**, **Null** layers | Flat colour, styled text, vector shapes, a transform-only rig |
| **Caption layer** | A Text layer that shows **captions**: a few timed words at a time, styled like any text. One made from a layer follows that layer's clips, trims and Retime, so its words stay on the speech through any edit. In code one caption is a `Cue` |
| **Adjustment layer** | Applies its effects to everything below. Also a switch any drawing layer can carry |
| **Layer group** | A header row folding layers together in the Timeline. Organisation only, the render never reads it |
| **Audio layer** | An audio item, or footage's audio |
| **Bus layer** | Sound routed to it from other layers, through its own effects and Volume. A layer's **output** goes to the master or a bus, and a **send** copies its sound to a bus, before or after its Volume. A bus can feed another bus, and passes sound for the whole composition |
| **Gain** | A layer's or clip's level before its effects and Volume, set by hand or by Normalise. Volume is the fader |
| **Lower behind** | A layer's level drops while another layer or a bus is sounding |
| **Solo safe** | A layer that keeps sounding while another is soloed |
| **Automation mode** | How a fader writes its keys while the comp plays: Read, Touch, Latch, Write or Trim |
| **Loudness target** | The loudness and true peak a comp's mix is made for |
| **Camera**, **Light** layers | 3D viewpoint and light. Only affect 3D layers |
| **Model layer** | A 3D model in the comp's 3D space. Always 3D. Its **Model settings** (Up axis, Units, Fit to comp) set how big it is, so its Scale stays 100%. Its **Materials** rows change the file's materials on the layer |
| **Take** | One embedded animation in a 3D model's file. A Model layer's **Take** row picks it by name |
| **Part** | One node of a 3D model's file, listed under its Model layer. Only a part with a changed row is saved |
| **Smoke and fire** | An effect that turns a 3D layer into a box of simulated gas, which the volumetric walk draws. A `.vdb` file opens as a Model layer with no faces and draws the same way. |
| **Depth points** | An effect that draws a shot as points in the comp's 3D space, each on its ray from a camera, placed by the depth from Analyse or a Depth layer |
| **Geometry options** | A Text or Shape layer's Extrusion depth and bevel, which make its outline a solid in the comp's 3D space, with **Front**, **Bevel**, **Side** and **Back** materials |
| **Card** | A flat Model layer with no 3D model behind it, made by **Layer ▸ New ▸ Card** or **Create card** |
| **Projected from** | A material row naming the Camera layer whose view lays its Base colour layer on a Model layer, in place of the model's uvs. Never *camera mapping* |
| **3D shape** | A cube, sphere, cylinder, cone, torus, plane or pyramid Lumit makes itself, in the comp's 3D space. Its **Geometry options** are its size and key like any row, and its **Surface** is a material. `LayerKind::Mesh` in code |

- **Clip**: an entry inside a Sequence layer, and only there. Two clips may overlap, never
  three. On a picture layer the overlap is a **dissolve**: the later clip comes in as the
  earlier goes out, through the later clip's fade-in shape, for as long as they overlap. On
  an audio-only layer it is a crossfade. A picture overlap may instead be a **transition**,
  one of the Transition effects run over the earlier clip. A fade at an end nothing overlaps
  runs to transparent, or to silence. A cut between clips is an **edit point**.
- **Linked clips**: a picture clip and its sound clip, on two Sequence layers, that move,
  trim and cut together while Linked is on.
- **Multicam clip**: a clip that plays one angle of a multicam comp, its picture on a picture
  layer and its sound on an audio-only one. To **cut to** an angle cuts the clip at the
  playhead and switches the piece after it. To **switch** changes a clip's angle without a
  cut. To **flatten** puts the angle's own footage clips in the clip's place.
- **Ripple**: an edit that moves everything after it by the same amount. **Insert** places
  a clip and ripples, **overwrite** places it over what is there. **Roll** moves one edit
  point, **slip** changes which source a clip shows, **slide** moves a clip between its
  neighbours.
- **Lift** removes a span and leaves the gap, **extract** removes it and closes the gap.
  **Match frame** loads a clip's footage into the source view on the frame the comp shows.
- **Anchor point**: the point the transform pivots around. Position places it.
- **Orient**: whether a layer turns by itself, along its motion path or towards the camera.
- **Parenting**: a layer can follow another layer's transform. No cycles.
- **Switches**: per-layer toggles. Visible, audible, solo, lock, shy, quality, motion blur,
  adjustment, 3D, collapse.

## 3. Animation

| Term | Meaning |
|---|---|
| **Property** | A named animatable value. Properties nest in **property groups** |
| **Keyframe** | A time and value, with hold, linear or bezier (**speed** and **influence**, AE's maths) |
| **Graph editor** | Edits curves as a **value graph**, a **speed graph** or an **acceleration graph**. Three views of the same data |
| **Expression** | A per-property script that computes the value each frame. Written in **JavaScript** or **Rhai**, which the expression's own language choice says |
| **Custom controls** | An effect holding a named set of controls for expressions to read. What an After Effects pseudo effect imports as |
| **Marker** | A labelled point or span. **Beat markers** come from audio onset detection. Footage items carry markers too, set in the source view |
| **Motion blur** | Three things: the layer switch, the **Motion blur** effect (optical flow), and **Accumulation motion blur** (re-renders and averages) |

## 4. Time and Retime

Four timebases, and code always says which one a number is in: **source time**, **clip
time**, **layer time**, **comp time**.

**Retime** is the one retiming system: a map from layer or clip time to source time, made
of segments. The value graph is AE's time remapping, the speed graph is Vegas' velocity.
They are two views, not two features.

| Term | Meaning |
|---|---|
| **Speed** | The slope of the Retime map. 100% normal, 0% freeze, negative reverse |
| **Freeze** | A stretch at speed 0 |
| **Overrun** | Retime asking past the media's ends. Holds the end frame, marked in the Timeline, never moves edit points |
| **Frame interpolation** | Nearest, blend or flow, for in-between source frames |
| **Stretch** | A command that rewrites the Retime map for a new speed. Not a stored multiplier |

## 5. Render, preview, export

Not interchangeable.

| Term | Meaning |
|---|---|
| **Render** | The engine making pixels, for anything |
| **Preview** | Playback inside Lumit. Never writes user files |
| **Export** | Writing a media file. May **bake**, which never changes the project |
| **Stem** | One bus exported as a file of its own |
| **Evaluation graph** | What the layer stack compiles into. Users never see the term |
| **Cache** | Frames stored in VRAM, RAM and disk tiers, keyed by content hash |
| **Proxy** | A smaller copy of a footage file, kept beside the original and read in its place while working |
| **Stand-in** | In a shared project, a small copy of a whole clip sent to someone who has not got the file, and read as if it were the file. Not a *proxy*, which needs its original beside it |
| **Preview resolution** | Full, Half, Third, Quarter, Auto |
| **Adaptive degradation** | Lowering preview quality under load. Never touches export |
| **Lottie** | An animation written as shapes and keys that a player draws in an app or on a page. Its file is a `.json`, and a **dotLottie** (`.lottie`) is one zipped with its pictures |
| **Animated SVG** | An SVG file whose animation plays by itself in a browser |
| **Vector export** | Writing a comp as a Lottie, a dotLottie or an animated SVG rather than as pixels. Its **check** lists, layer by layer, what won't come across |

## 6. Compositing

| Term | Meaning |
|---|---|
| **Mask** | A bezier path gating a layer's alpha |
| **Matte** | Another layer's alpha or luma gating this one. Not *track matte* |
| **Roto brush** | Builds a matte from painted strokes, carried by optical flow. **Refine edge** is its soft boundary |
| **Blend mode** | How a layer composites over what's below |
| **Effect** | One operation in a layer's **effect stack**, or one box in a node graph |
| **Source layer** | Another layer an effect reads as an input, such as the one Emit from image takes its points from |
| **Scene pass** | What a 3D precomp knows about its pixels, handed to effects: **Normals**, **Position**, **Part id** and **Depth** |
| **Part** | A node of a 3D model, each material of a node with more than one, or a whole 3D layer, as a scene pass counts them. Its number comes from its layer and its names. **Part matte** keeps the parts picked |
| **Driver** | A node that makes a value rather than a picture and drives a parameter through a wire |
| **Read**, **Input**, **Output** nodes | Bring an item in, take a value or picture from outside, the one picture the graph shows |
| **Merge**, **Switch**, **Time offset** nodes | A over B, pick one input, show the input at another time |
| **Split channels**, **Combine channels** | Picture into four greyscale pictures, and back |
| **Repeat start**, **Repeat end** nodes | Run the boxes wired between them several times, or once for each point or path. **Feedback** works each time on the last time's picture, **Copies** keeps every time and lays them over each other |
| **Node graph effect** | Applies a node graph to a layer |
| **Wire**, **port** | A connection on the Graph panel, and the typed socket it plugs into |
| **Points stream** | Per-frame particle data an effect produces. Never stored |
| **Shapes** | Paths on a wire: a Read box hands out the paths of the shape layers in the comp it reads |
| **Collide with** | A layer whose shapes and masks Particulate's particles hit. With one set, each frame's particles are worked out from the frame before |
| **Flock** | Particulate's particles steering by their neighbours: apart, the same way, together. Worked out from the frame before, as Collide with is |
| **Sparks** | Small particles Particulate throws out as one of its particles dies, hits something or flies. Worked out from the frame before |
| **Simulate** | Particulate worked out step by step on the graphics card, up to its Max particles, under fields and what it hits. Saved as kept states, or baked to the project's cache |
| **Trails** | Particulate drawing each frame's particles over the picture it kept from the frame before, faded, zoomed and turned, so they leave smears |
| **Volumetric light** | A light seen in the air it shines through: its beam, the shafts 3D layers cut in it, its glow |
| **Environment light** | A light from a picture of everything round the scene, an HDRI, that lights 3D layers and models and can show behind them. Its picture is its **Source** layer |
| **Working space** | Scene-linear, premultiplied, fp16 (fp32 opt-in) |
| **OCIO config** | An OpenColorIO config a project can name |

## 7. Interface

| Term | Meaning |
|---|---|
| **Panel** | A dockable piece of UI |
| **Window** | One of Lumit's operating-system windows. The **main window** has the menu bar. A **floating window** holds panels taken out of it, arranged as the main window's are |
| **Video monitor** | The Viewer's picture alone, filling a display of its own |
| **Workspace** | A saved panel layout |
| **Profile** | One person's settings on a machine: appearance, shortcuts, layout, preferences and what they have saved. Never anything about the machine |
| **Account** | A Lumit account, which a profile can be signed in to. Optional |
| **Lumit Pro** | The paid plan on an account: Lumit's relay, **sync**, which keeps a profile the same on every machine, and **backup** |
| **Backup** | A copy of a project that Lumit Pro keeps each time it is saved. Its earlier saves are its **history** |
| **Remote export** | An export one of an account's computers does for another. The computer that asks **sends** it, and the file comes back or stays where it was made. Comes with Lumit Pro |
| **Graph panel** | Draws a layer's effects as nodes and wires. Not the evaluation graph |
| **Viewer** | Shows a comp, footage or layer. Holds one, two or four **views** |
| **Timeline** | A comp's layers against time |
| **Cut timeline** | The same layers as tracks of clips, for cutting |
| **Transcript panel** | What the Cut timeline's composition says, or the source view's footage, as text. Deleting, moving or copying words cuts the composition to match |
| **Filler words** | Hesitations such as *um*, phrases said only to fill, and words said twice in a row, which the Transcript panel finds and takes out |
| **Pause** | A silence between two words while transcribed speech plays on. A stretch with no transcribed speech under it is a **break** |
| **Source view** | A Viewer view of one footage item, with **In** and **Out** marks on the stretch to place |
| **Angle viewer** | The source view showing every angle of a multicam comp at once, in a grid. Clicking an angle cuts to it |
| **Work area** | The span used for preview and default export |
| **Playhead** | The current time. Not *CTI* |
| **Scopes** | Waveform, RGB parade, vectorscope, histogram, CIE chromaticity. One, two or four in a panel |
| **Grade panel** | The selected layer's grade as wheels, primaries, curves and a look, over its Colour effects |
| **Look** | A preset of Colour effects in the library's Looks group, picked from the Grade panel. It takes the place of the layer's grade |
| **Easing panel** | One curve shape applied to selected keyframe spans |
| **Retime panel** | The selected layer's speed, length, freeze, reverse and in-between frames |
| **Text panel** | Sets a text layer's font, size, spacing and outline |
| **Paragraph panel** | Sets how a text layer's lines are aligned and spaced |
| **Expressions panel** | Keeps saved expressions and puts one on the selected properties |
| **Captions panel** | Lists the selected caption layer's captions to read, retype, retime, split and join |
| **Stack** (of panels) | A panel group drawn as twirled panels one above another, not as tabs |
| **Flowchart** | A comp drawn between the comps that place it and the comps it places |

## 8. Extensibility

| Term | Meaning |
|---|---|
| **OFX** | OpenFX. Lumit hosts it |
| **LFX** | An effect an extension carries: a WGSL shader and an optional interface page |
| **CLAP**, **VST3** | Audio plugin standards Lumit hosts. No VST2 |
| **Addon** | An optional download from Settings: the model runtime and model packs. Analysis only, never generation: a model measures a picture or hears speech, and makes no picture of its own |
| **Model pack** | One addon holding one model |
| **Extension** | A folder somebody else made, with an `extension.json`, that adds panels or effects. Its panel is shown in the dock. It reaches Lumit only through what its manifest asked to be allowed. Not *plugin*, which is an effect |
| **Preset** | Saved effects, properties or animation |
| **Script** | A JavaScript file that edits a project through the same commands the command line and MCP use. After Effects `.jsx` scripts run as they are |

## 9. Words we don't use

| Don't say | Say |
|---|---|
| Track, line (a timeline row) | Layer, Sequence layer |
| Velocity (the quantity) | Speed |
| Time remap | Retime |
| Bin | Folder |
| CTI | Playhead |
| Render (meaning export) | Export |
| Event | Clip |
| Pre-render (user-facing) | Cache, bake |

- **To track** something through a shot is the trade's verb and stays. A track is one
  followed feature, never a timeline row.
- **To clip** in keying and colour (clip black, clipped highlights) stays.
- The Audio timeline and Cut timeline panels call their rows tracks, in their own strings
  and files only. Everywhere else it's a Sequence layer, with `audio_only` set for sound.
- The Retime graph labels its lenses Time and Velocity. Speed stays the word for the
  quantity.
- **Switch** means both the node and a layer's toggles.
