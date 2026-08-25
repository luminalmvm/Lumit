# After Effects: deep model research (for Kiriko design docs)

Research date: 2026-07-12. Sources: Adobe helpx documentation (via search extracts — helpx.adobe.com blocked direct fetch), AE Scripting Guide (ae-scripting.docsforadobe.dev), AE Expression Reference (ae-expressions.docsforadobe.dev), AE C++ SDK guide (ae-plugins.docsforadobe.dev), ProVideo Coalition (Chris Zwar's AE & Performance series), School of Motion, Puget Systems, Creative COW / Adobe community threads. Where a detail could not be independently verified it is marked *(verify)*.

---

## 1. Panel / workspace anatomy

### 1.1 The frame/panel/viewer model

- The AE UI is a single **application window** subdivided into a tree of **frames**; each frame holds a **panel group** (tabbed panels). Panels can also float in separate windows.
- **Panels** are singleton tool windows (Project, Info, Preview…). **Viewers** are panels that can display one of several *items* (compositions, layers, footage) chosen from a viewer menu in its tab — Composition, Layer, and Footage panels are all viewers. You can open multiple viewers and lock a viewer to a specific item (padlock icon on the tab). A locked viewer will not switch when you open a different comp.
- **Docking behaviour**: dragging a panel tab shows **drop zones**. The centre zone of a group = *group* (add as a new tab). The four edge zones of a group/frame = *dock* (split the frame, insert adjacent, all groups resize to accommodate). Window edges also expose docking zones. Dragging out with Ctrl/Cmd (or to outside the app window) creates a **floating window**. Panels can also be **stacked** (vertically collapsible stacks, used in the Essential Graphics-era skinny side rail).
- `~` (tilde) with the pointer over a panel maximises that panel to the full window (toggle).
- Each panel has a **panel menu** (hamburger, top-right) with panel-specific options (e.g. Composition panel: "Composition Settings", "Transparency Grid"; Timeline: column visibility).

### 1.2 Major panels (Window menu inventory)

| Panel | Role / key contents |
|---|---|
| **Project** | Bin tree of footage items, comps, folders, solids folder. Columns: type, size, media duration, file path, date, comment; interpretation info readout at top (shows selected item's dimensions, PAR, fps, duration, colour profile). Search field. Footage interpretation (fps override, alpha interpretation, field separation, pulldown, loop) lives on items here. 8/16/32-bpc project depth button at panel bottom (Alt-click cycles; opens Project Settings otherwise). |
| **Composition viewer** | The render view. See §2. |
| **Layer viewer** | Shows a single layer's *source* (pre-transform), used for masks-on-source, paint, Roto Brush strokes, tracker regions, and slip-editing via its own trim bar. Has a "View" menu (Masks / Anchor Point Path / Roto Brush & Refine Edge / Motion Tracker Points…) and a Render checkbox (show source vs rendered-with-effect result). |
| **Footage viewer** | Preview of a project footage item, with set-in/out for insert/overlay edits into a comp *(rarely used)*. |
| **Timeline** | One tab per open comp. Left side: layer outline (columns: A/V features, label, #, name/source, switches, modes/track-matte, parent & link, plus optional In/Out/Duration/Stretch columns). Right side: time ruler, work area bar, layer duration bars, keyframes, Graph Editor toggle (shares this area), comp-level buttons (motion blur enable, frame blending enable, draft 3D, shy, Graph Editor, composition flowchart, render-time profiler in 2022+). |
| **Effect Controls** | Per-layer stack of applied effects; each effect shows its parameter UI (sliders, angle dials, colour pickers, point pickers, custom-drawn UIs), enable checkbox (fx), Reset, About, and compositing options (effect masks). Tabbed per layer. |
| **Effects & Presets** | Searchable tree of all effects grouped by category, plus Animation Presets (`.ffx` files, includes property/keyframe/expression bundles). Apply by double-click (selected layers) or drag onto a layer/viewer. |
| **Preview** | Playback transport plus preview configuration per shortcut key (spacebar / numpad 0 / Shift+numpad 0 each carry their own behaviour set): frame rate override, skip (render every Nth frame), resolution override, full-screen, loop mode, "Cache Before Playback", range (work area / entire duration / around current time), what invokes stop, audio mute. |
| **Info** | Pixel colour under cursor (8bpc/decimal/hex/HDR values per project depth), pointer X/Y, selected-layer in/out/duration, and streaming status messages during renders. |
| **Audio** | Level meters + per-layer audio level sliders. |
| **Align** | Align layers (left/h-centre/right/top/v-centre/bottom) relative to Selection or Composition; distribute (needs ≥3 layers). |
| **Character / Paragraph** | Text styling: font family/style, size, fill & stroke colours, stroke width & stroke-over-fill order, kerning/tracking, leading, baseline shift, vertical/horizontal scale, faux bold/italic, all caps/small caps, super/subscript; Paragraph: alignment, justification, indents, space before/after, text direction. |
| **Tracker** | Point tracker UI: Track Motion / Stabilize Motion / Track Camera (fires the 3D Camera Tracker effect) / Warp Stabilizer buttons; track type (position/rotation/scale), feature region + search region + attach point handles (edited in Layer viewer), Analyze 1 frame / forward / backward, Apply (with dimension choice), Edit Target. Also entry point to mocha AE (bundled planar tracker). |
| **Lumetri Scopes** | Video scopes rendered from the active viewer: Waveform (RGB / Luma / YC / YC no chroma), Vectorscope (YUV and HLS), Histogram, RGB/YUV Parade; configurable colour space (Rec.601/709/2020) and 8-bit/float readout. |
| **Paint / Brushes** | For the Brush, Clone Stamp, Eraser tools (used in the Layer viewer): opacity/flow/mode/duration (constant, write-on, single frame, custom), clone presets; Brushes = brush tip editor. |
| **Motion Sketch** | Records freehand pointer movement as Position keyframes in real time (capture speed %, smoothing, show wireframe/background). |
| **The Smoother** | Reduces/smooths selected keyframes with a tolerance value (applies to temporal or spatial curves). |
| **The Wiggler** | Legacy: inserts randomised keyframes between two selected keyframes (frequency/magnitude, per-dimension) — the keyframe-baked ancestor of the `wiggle()` expression. |
| **Mask Interpolation** | "Smart Mask Interpolation": generates intermediate mask-path keyframes with vertex-matching controls. |
| **Essential Graphics** | Builds Motion Graphics templates (`.mogrt`): drag properties in to expose them as primary/"Essential Properties", group, add comments; export for Premiere Pro. |
| **Properties panel** (2023, 23.3+) | Context-sensitive quick access to selected layer's transform, text, and shape properties without twirling the timeline. |
| **Media Browser / Libraries / Adobe Color Themes / Learn / Effects manager (24.x)** | Asset browsing, CC Libraries, onboarding, plugin enable/disable. |
| **Flowchart** | Read-only node-graph visualisation of comp/layer/effect dependencies. |
| **Metadata** | XMP metadata for footage items. |
| **Render Queue** | A timeline-area tab; see §8. |

### 1.3 Workspaces

- **Window > Workspace** presets: Default, Learn, Standard, Small Screen, Animation, Effects, Essential Graphics, Colour, Motion Tracking, Paint, Text, Minimal *(set varies by version)*. Shown as clickable tabs in a workspace bar under the menu bar; overflow in a `>>` menu; "Edit Workspaces…" reorders/hides them.
- Workspaces persist automatically: any layout change is remembered per workspace until **Reset "name" to Saved Layout**. **Save as New Workspace…** creates named custom workspaces; **Save Changes to this Workspace** commits the current state. Stored in the user preferences folder (per-version text/XML prefs), not in the project file.
- **Locked viewers are not saved** into workspaces — they reference a specific project item, so a workspace saved with a locked viewer drops the lock (Adobe documents: unlock all viewers before saving a workspace).
- Panel sizes are proportional: docking resizes siblings rather than overlapping; there is no free-form overlap inside a window (only floating windows overlap).

---

## 2. Composition viewer specifics

Bottom bar of the Composition panel, left to right (canonical order varies slightly by version):

1. **Magnification ratio popup** — zoom presets (…, 6.25%, 12.5%, 25%, 33.3%, 50%, 100%, 200%, 400%, 800%, 1600%, 3200%, 6400% *(verify exact ends)*), plus **Fit** and **Fit up to 100%**. Zoom is display-only scaling of the rendered buffer — it never changes render resolution by itself (but see Auto resolution). Mouse wheel/`,`/`.` zoom; Ctrl+drag pans *(space+drag pans)*.
2. **Resolution/Down Sample Factor popup** — **Full / Half / Third / Quarter / Auto / Custom…**. This is the *actual raster resolution* the comp renders at for display:
   - Full: every pixel. Half: every 2nd pixel in each axis → 1/4 the pixels, roughly 4× faster. Third: 1/9 the pixels. Quarter: 1/16 the pixels. Custom: arbitrary x/y factors.
   - **Auto**: renders only the pixels needed at the current zoom (e.g. at 50% zoom, renders Half). This is the default in modern versions.
   - The setting is per-comp (also stored in Composition Settings) and interacts with the Preview panel's own resolution override.
   - Preview-only; final output resolution comes from Render Settings.
3. **Fast Previews menu** (lightning bolt): **Off (Final Quality)**, **Adaptive Resolution** (degrades resolution while scrubbing/dragging, limit configurable 1/2–1/16 in Previews preferences), **Draft** (3D: no lights/shadows/DOF quality), **Fast Draft**, **Wireframe**. Adaptive Resolution only kicks in during interaction, then re-renders at the set resolution.
4. **Toggle Transparency Grid** — checkerboard behind transparent pixels instead of the comp background colour.
5. **Toggle Mask and Shape Path Visibility**.
6. **Region of Interest (ROI)** — drag a rectangle; AE renders *only* that region (huge preview speed win). "Composition > Crop Comp to Region of Interest" makes it permanent. ROI does not affect final render.
7. **Show Channel and Color Management Settings** (coloured circles icon): show **RGB / Red / Green / Blue / Alpha / RGB Straight** (alpha displayed as greyscale matte; single channels optionally **Colorize**d); also per-viewer colour management: **Use Display Color Management** toggle, simulate output, and in 22.6+ the OCIO/ACES view transform picker when the project is in OCIO colour mode.
8. **Take Snapshot / Show Snapshot** (camera icon; hold Show to compare against the stored frame; Shift+F5–F8 store up to 4).
9. **Grid and guide options** (choose): **Title/Action Safe** overlays (configurable %s in Grids & Guides prefs), **Proportional Grid**, **Grid**, **Guides**, **Rulers** (Ctrl/Cmd+R; drag guides out of rulers; snap-to-guides; lock guides; guides import/export via View menu in recent versions).
10. **Toggle Pixel Aspect Ratio Correction** — display-only stretch so non-square-PAR comps look correct on square-pixel monitors (introduces display resampling artefacts).
11. **Exposure/Gamma controls** — a viewer-only **Adjust Exposure** f-stop control (and Reset) for inspecting HDR/32-bpc content; does not affect rendering.
12. **3D controls**: **3D View popup** (Active Camera, Front/Left/Top/etc. orthographic, Custom View 1–3), **Select View Layout** (1 / 2 / 4 views with shared or independent view settings), **Draft 3D** and **3D Ground Plane** toggles (Advanced 3D era), **3D Renderer** readout.
13. **Current time** readout (click to go-to-time) and **camera/renderer buttons** vary by version.

**View Options** dialog (panel menu) toggles handles, motion paths, effect controls (on-viewer UIs), masks, camera/light wireframes. **Wireframe interactions** exist both as a Fast Preview mode and legacy preference.

The viewer shows only the **current frame** of the render pipeline; everything about playback is the Preview system + caches (§7).

---

## 3. Composition / layer model

### 3.1 Composition settings

- **Basic tab**: width × height (max **30,000 × 30,000 px**, memory-limited — a 30k×30k 8-bpc frame ≈ 3.5 GB; layers/footage share the same 30,000 px cap), **Pixel Aspect Ratio** (square, D1/DV NTSC 0.91, anamorphic 2:1, etc.), **Frame Rate** (custom 1–99 fps; drop/non-drop timecode display), **Resolution** (the same Full/Half/Third/Quarter/Custom as the viewer — stored per comp), **Start Timecode**, **Duration** (max **3 hours** per comp *(verify: some versions allow more via project timecode base)*), **Background Color** (display + collapsed/nested rendering context; comps are transparent where nothing renders — the bg colour is *not* baked in unless flattened into output without alpha).
- **Advanced tab**: **Anchor** grid (which corner content sticks to when resizing the comp), **"Preserve frame rate when nested or in render queue"** (comp keeps its own fps when nested — otherwise nested comps are sampled at the parent's fps), **"Preserve resolution when nested"**, **Motion Blur**: **Shutter Angle** (0–720°; 180° = half-frame exposure; effective blur window = angle/360 × frame duration), **Shutter Phase** (−360–360°, offset of blur window relative to frame time; default −90° with 180° angle centres blur on the frame), **Samples Per Frame** (default 16; the *fixed* sample count used for 3D layers and some effects) and **Adaptive Sample Limit** (max 256 *(range 2–256)*; 2D layer motion blur adaptively picks samples up to this limit).
- **3D Renderer tab**: Classic 3D / Cinema 4D (CINERENDER, extrusion + curvature) / **Advanced 3D** (23.6/24.x+: real-time GPU renderer, image-based lighting HDRI, 3D model import glb/gltf).
- **Bit depth is a project setting, not per-comp**: 8/16/32-bpc float, set in Project Settings > Color (Alt/Opt-click the depth button in the Project panel). Colour working space (ICC profile or OCIO/ACES config since 22.6) is also project-level; "Linearize working space" and "Blend colors using 1.0 gamma" options change compositing maths.

### 3.2 Layer types

Every layer is either **footage-backed** (its source is a project item) or **synthetic**:

- **Footage layer** — video, still, image sequence, audio; source has its own interpretation (fps, alpha straight/premultiplied, fields, colour profile). Stills default to comp duration or a preference-set duration.
- **Solid** — comp-independent fixed-colour raster source (a footage item stored in the project's "Solids" folder; multiple layers can share one solid source; "affect all layers using this solid" checkbox in Solid Settings).
- **Text** — vector, per-character rasterised; source text keyframable (hold interpolation); the **animator system** (Animators + Range Selectors + per-character properties incl. per-character 3D) is its own mini keyframe/selector model.
- **Shape** — vector layer containing a tree of **Groups**, path primitives (rectangle/rounded/ellipse/polystar/bezier Path), **Fill/Stroke/Gradient Fill/Gradient Stroke** (with dashes, taper & wave on strokes in 17.x+), and **path operations**: Merge Paths, Offset Paths, Pucker & Bloat, Repeater (transform-copies!), Round Corners, Trim Paths, Twist, Wiggle Paths, Wiggle Transform, Zig Zag. Shapes are always continuously rasterised.
- **Null Object** — invisible 100×100 layer used as a parenting/expression target (renders nothing).
- **Camera** — one-node or two-node (with Point of Interest); zoom/focal length/film size/angle of view; DOF (focus distance, aperture, blur level); only affects 3D layers.
- **Light** — Parallel / Spot / Point / Ambient; intensity, colour, cone angle/feather, falloff (none/smooth/inverse-square), casts shadows (per-light) with shadow darkness/diffusion; 3D layers have material options (accepts shadows/lights, ambient/diffuse/specular…).
- **Adjustment layer** — see §6.3.
- **Precomposition layer** — a nested comp used as a source.
- **Guide layers** — any layer flagged as guide renders in previews but not in final output (unless enabled in render settings).
- Also: audio-only layers, camera/light do not intercept 2D order; **Environment Layer** flag for 360 footage in Classic 3D *(VR feature set)*.

### 3.3 Columns, switches, and modes (Timeline)

**A/V Features column**: Video (eyeball), Audio (speaker), **Solo** (renders only soloed layers in preview *and, per render settings option, output*), **Lock**.

**Switches column** (one icon per layer; comp-level "enable" master buttons exist for motion blur, frame blending, draft 3D):

| Switch | Behaviour |
|---|---|
| **Shy** (little figure) | With the comp-level Shy button on, shy layers are hidden *from the timeline list only* — they still render. |
| **Collapse Transformations / Continuously Rasterize** (sunburst) | For precomp layers = collapse transformations (§6.2). For vector footage (AI/PDF/EPS) = continuously rasterize: re-rasterise from vectors after transform each frame instead of rasterising at source size then transforming. Shape/text layers always behave this way (switch has no effect on shapes/text except enabling per-character 3D quality nuances *(text/shape always CR)*). |
| **Quality** (\ / forward-slash) | Draft (no anti-aliasing, lower-quality sampling) vs Best; a third state **Bilinear vs Bicubic** sampling choice for scaled bitmap layers (Best quality only). |
| **Effect (fx)** | Master enable of all effects on the layer. |
| **Frame Blending** (dashed vs solid backslash) | Off → **Frame Mix** → **Pixel Motion**; only rendered when the comp-level Frame Blending master button is on (previews) — Render Settings decide for output ("On for Checked Layers"). |
| **Motion Blur** | Per-layer; sampled per comp Advanced settings; needs comp master button for preview; Render Settings decide for output. |
| **Adjustment Layer** | Toggles adjustment behaviour on any layer (a footage layer can become an adjustment layer — its content is ignored, alpha/luminance still gates via masks). |
| **3D Layer** | Promotes the layer into 3D space (adds Z components, Orientation + X/Y/Z Rotation, Material Options). 2D layers between 3D layers break 3D interaction ("render break"), splitting 3D sets that can no longer intersect/shadow each other. |

**Modes column**: blending mode dropdown, **Preserve Underlying Transparency (T)** checkbox (layer only shows where underlying composite is opaque), and the **Track Matte** controls (§6.4).

**Parent & Link column**: parenting dropdown/pick-whip (transform inheritance, not opacity/effects); also the home of property pick-whips for expressions.

### 3.4 Layer time: in/out vs source

- A layer has an **In point** and **Out point** in comp time, plus a **source offset** (where its media starts relative to comp time). Trimming (Alt+[ / Alt+]) hides media without destroying it; dragging the layer bar body moves layer+trim together; dragging the trimmed bar's texture in the Layer panel or Ctrl-drag = **slip edit** (moves source under fixed in/out).
- **Stretch** column: time stretch % (100 = normal, 50 = double speed, negative = reversed — a −100% stretch reverses playback around a chosen anchor). Time stretch rescales *existing keyframes on that layer's source*? — No: stretch affects the layer's source playback **and** the timing of its keyframes (all property keyframes stretch with it) *(verified behaviour: keyframes stretch; use precomp to avoid)*.
- Still images have arbitrary in/out; video layers cannot extend beyond source duration unless time-remapped, time-stretched, or looped in Interpretation.
- Sequence layers, split layer (Ctrl+Shift+D), and markers (layer markers + comp markers, with duration, used by expressions) complete the time model.

---

## 4. Keyframe system

### 4.1 Temporal interpolation types

Per keyframe, separately for **incoming** and **outgoing** interpolation:

- **Linear** — constant rate between keyframes; value graph = straight segments; speed graph = flat steps with instant jumps at keys. Icon: diamond.
- **Bezier** — fully manual tangent handles; in/out handles independent. Icon: hourglass.
- **Continuous Bezier** — manual handles but in/out are collinear (one straight line through the keyframe; changing one side changes the other's direction, lengths independent). 
- **Auto Bezier** — AE picks smooth tangents automatically (equal in/out speed, direction bisecting neighbours); adjusting a handle converts it to Continuous Bezier. Icon: circle.
- **Hold** — value freezes until the next keyframe (outgoing only, no interpolation). Icon: square. Combination icons appear when in/out types differ (e.g. linear-in/hold-out).
- Toggle: Ctrl/Cmd-click keyframe cycles linear↔auto-bezier; **Easy Ease** (F9), Ease In (Shift+F9), Ease Out (Ctrl+Shift+F9) set bezier with **speed = 0 and influence = 33.33%** on the eased side(s).

### 4.2 Spatial interpolation

- Multi-dimensional spatial properties (Position, anchor point, effect point controls) have a second, independent interpolation domain: the **motion path** in comp space. Each keyframe is a path vertex with spatial bezier tangents (Linear / Bezier / Continuous / Auto Bezier spatially).
- Default spatial interpolation is **Auto Bezier**; preference "Default Spatial Interpolation to Linear" switches it. Motion path dots in the viewer are per-frame samples — dot spacing visualises speed.
- Temporal and spatial are orthogonal: the spatial path defines *where* the value travels; temporal interpolation defines *when along the path* (arc-length parameterisation: the speed graph for Position is speed along the path in px/sec).

### 4.3 Roving keyframes

- Spatial-property keyframes (except first and last) can **"rove across time"**: a roving keyframe keeps its spatial position but surrenders its time — AE re-times it automatically so speed is smoothed/constant across the span between the nearest non-roving keyframes. Dragging a roving key in the timeline pins it again. Purpose: keep a complex path shape while removing speed bumps at intermediate vertices.

### 4.4 Keyframe velocity & influence — the maths

Every bezier temporal keyframe carries, per side (incoming/outgoing) and per dimension:

- **Speed** — the instantaneous rate at the keyframe, in *value units per second* (px/sec for position — a single magnitude along the path; %/sec for scale per dimension; °/sec for rotation).
- **Influence** — a percentage in **[0.1, 100]** (this exact range is in the scripting API's `KeyframeEase` object: `speed` float, `influence` 0.1–100). Influence = *how far toward the neighbouring keyframe this side's tangent extends*, as a fraction of the inter-keyframe time interval.

**Value-curve construction.** Between keyframes K1 = (t1, v1) and K2 = (t2, v2), with Δt = t2 − t1, outgoing speed s1/influence i1 (as fraction, i.e. %/100) and incoming speed s2/influence i2, AE evaluates a **cubic bezier in (time, value) space** with control points:

```
P0 = (t1, v1)
P1 = (t1 + i1·Δt,  v1 + s1·i1·Δt)     // outgoing tangent of K1
P2 = (t2 − i2·Δt,  v2 − s2·i2·Δt)     // incoming tangent of K2
P3 = (t2, v2)
```

i.e. each handle leaves its keyframe along slope = speed and stretches i·Δt horizontally. To evaluate at comp time t, solve the (monotonic) time component `t = B_x(u)` for the bezier parameter u, then value = `B_y(u)`. Linear interpolation is the degenerate case s1 = s2 = (v2−v1)/Δt (any influence — AE reports 16.67% for linear keys in the velocity dialog *(verify exact reported figure)*).

**Speed graph.** Plots |dV/dt| over time (for spatial props: speed along the path; for scalars: signed derivative is shown as magnitude with direction implied *(value graph is signed; speed graph of Time Remap is signed/percentage-like)*). Consequences:

- The curve's *height* at a keyframe = that side's Speed number.
- The *horizontal reach* of a speed-graph handle = Influence (% of the interval). Dragging a handle **toward the neighbouring keyframe increases influence**; toward its own keyframe decreases it. Dragging vertically changes Speed.
- Easy Ease = speed 0, influence 33.33% ⇒ the classic ease curve. Pushing influence toward ~100% on both facing sides forces the middle of the interval to compensate with a violent speed spike (the area under the speed graph between two keys is fixed = |v2 − v1|; that integral constraint is *why* flattening the ends steepens the middle).
- **Keyframe Velocity dialog** (Ctrl+Shift+K): numeric incoming/outgoing Speed + Influence, and a **Continuous** checkbox that locks incoming speed = outgoing speed.
- **Exaggerated/overshoot** motion = speeds larger than the average slope, or negative-going value-graph tangents.

### 4.5 Graph Editor

- Toggled per-timeline (Graph Editor button); replaces layer bars with curves for selected/graph-flagged properties ("include this property in graph editor set" icon).
- **Graph type menu**: Auto-Select Graph Type (value graph for scalar properties, speed graph for spatial ones — because a 2D/3D value curve can't be edited as a single scalar), Edit Value Graph, Edit Speed Graph; plus "Show Reference Graph" (the other graph, ghosted, read-only).
- Value graph editing directly manipulates the bezier tangents described above (for **separated dimensions** — Position can be split into X/Y/Z scalar properties via "Separate Dimensions", changing it into independent scalar curves and *discarding* the unified motion-path spatial bezier model).
- Editing aids: transform box (scale groups of keyframes in time/value; Ctrl for taper), snap, auto-zoom, show audio waveforms/layer in-out/expressions results ("post-expression graph"), jerk-free normalised view. Buttons along the bottom apply interpolation (hold/linear/auto-bezier), easy-ease variants, and open the velocity dialog.

---

## 5. Time remapping

### 5.1 Model

- **Layer > Time > Enable Time Remapping** (Ctrl+Alt+T) adds a keyframable **Time Remap** property: a function **mapping layer/comp time (horizontal) → source time in seconds (value)**. Two initial keyframes are created: at the layer's in point (value = source start, typically 0:00) and at the source's end (value = source duration).
- **Value graph semantics**: slope = playback rate. Slope 1 = 100% speed; upward slope = forwards; **downward = reverse**; **flat = freeze frame**; steeper = faster. Any interpolation type is legal (hold keyframes give staccato jumps; ease gives speed ramps).
- **Speed graph** for Time Remap shows the derivative as a speed percentage-like value; easing time-remap keys in the speed graph creates smooth ramps. Adobe notes audio pitch follows the Time Remap speed graph directly.
- Because it's just a property, Time Remap can host **expressions** (e.g. `loopOut()` to loop footage) and is itself the standard idiom for freeze frames: **Layer > Time > Freeze Frame** enables time remapping with a single **Hold** keyframe at the current time (also "Freeze on Last Frame" in newer versions).

### 5.2 Interaction with layer duration

- Enabling Time Remap makes the layer's out point independent of source length: the layer bar can be **extended to the full comp duration**; beyond the last Time Remap keyframe the last keyframe's source frame holds (and before the first, the first holds). This is the mechanism for freeze-extend endings.
- The mapping clamps at the source's first/last frame — values beyond the source range hold those frames.
- Time stretch and time remap compose (stretch applies after remap in layer time *(verify order)*); remapped layers can then be precomped for further nesting.
- **Important subtlety**: the Time Remap value refers to *source footage time*, so keyframes elsewhere on the layer (masks, effects) remain in layer/comp time — they are NOT retimed by Time Remap (unlike time stretch, which stretches keyframes).

### 5.3 Frame blending

When a layer plays at non-native rates (stretch, remap, or comp fps ≠ footage fps), in-between frames must be synthesised. Layer switch states:

- **Frame Mix** (dashed icon): cross-dissolves the two nearest source frames weighted by fractional position. Cheap; produces ghosting/double-exposure on fast motion.
- **Pixel Motion** (solid icon): per-pixel **optical flow** — motion vectors are estimated between neighbouring frames and new intermediate frames are *synthesised by warping*. Much smoother slow motion; computationally expensive; artefacts ("tearing", wobble) where flow estimation fails (occlusions, motion-blurred or repetitive texture). The **Timewarp** effect exposes the same engine with more control (vector detail, smoothing, weighting) and a "Pixel Motion (Frame Blend)" hybrid that blends where no motion and synthesises where motion exists.
- Requires the comp-level **Enable Frame Blending** master button for previews; Render Settings ("On for Checked Layers") governs output. Frame blending applies to nested comps too (treats the precomp render as footage).

---

## 6. Compositing model

### 6.1 Precompose / nesting

- **Layer > Pre-compose** (Ctrl+Shift+C): moves selected layer(s) into a new nested comp, replaced by a single precomp layer. Options:
  - **Leave all attributes** (single layer only): the new precomp contains just the raw source sized to the *layer source*; effects/masks/transforms/keyframes stay on the precomp layer in the parent.
  - **Move all attributes**: everything moves into the new comp, which takes the *parent comp's* size/duration; the outer layer gets fresh defaults. Checkbox: "Adjust composition duration to the time span of the selected layers" and "Open New Composition".
- Nesting = using any comp as a layer source (also via drag from Project panel). Default render model: the nested comp is **fully rendered to an intermediate buffer at its own size** (with its own resolution/quality context), then treated as footage in the parent (masked/effected/transformed like any raster layer). Comp-level switches propagate: changing quality/motion-blur enables in a parent can recursively affect nested comps per the "Switches Affect Nested Comps" preference.

### 6.2 Collapse transformations

- The sunburst switch on a precomp layer **defers the nested comp's per-layer transforms so they concatenate with the parent's transform** instead of rasterising at the nested comp boundary. Effects:
  - No intermediate raster at nested-comp size: content transformed once by combined matrices → **no double resampling, no clipping at the precomp's frame edges** (content outside the nested comp bounds becomes visible), better quality when scaling up.
  - **3D-ness passes through**: 3D layers inside a collapsed precomp interact (intersect, shadow) with 3D layers in the parent, and are viewed through the *parent's* camera.
  - **Blending modes of inner layers composite against the parent's stack** (the precomp layer's own mode becomes fixed *(collapsed precomps historically forced "no own blend mode"; since CS5.5-ish a collapsed comp's inner modes propagate)*).
  - Continuously rasterize is the same switch semantics applied to vector footage: rasterisation happens *after* the full transform chain each frame.
  - **Breaking collapse**: applying an effect, layer style, mask, matte, motion blur nuance, or certain switches to the collapsed layer forces AE to render an intermediate anyway ("the layer is rendered then the effect applies" — quality reverts to the rasterise-then-transform pipeline at parent size), while retaining some collapse properties. Render order per layer is masks → effects → transform; collapse re-orders transform *ahead of* the parent's effects by eliminating the boundary.

### 6.3 Adjustment layers

- Any layer with the adjustment switch on stops rendering its own content; instead **its effect stack is applied to the composite of all layers below it** in the stack (within the same comp). Its masks/alpha and opacity **attenuate where and how strongly** the effects apply (the processed result is blended back by the adjustment layer's alpha). Transforms on the adjustment layer transform its alpha, not the picture. Standard uses: comp-wide grades, blurs, effect regionalisation. An adjustment layer that is also 3D interacts with camera/lights in surprising ways (renders effects across layers behind it in camera space) — commonly avoided.

### 6.4 Track mattes

- Classic model (≤ 22.x): a layer takes its transparency from **the layer directly above** it; modes: **Alpha Matte, Alpha Inverted Matte, Luma Matte, Luma Inverted Matte**; the matte layer's video is auto-disabled.
- **23.0 (2023) rework — selectable track mattes**: the Modes column gains a **matte layer dropdown + pick-whip**; *any* layer can be the matte regardless of stacking order, one matte layer can serve **multiple target layers**, and two toggles replace the 4-mode menu: **Alpha/Luma** toggle and **Inverted** toggle. Matte layer video can stay on. (Internally stored as `trackMatteType` + a layer reference.)
- Matte sampling happens **after** the matte layer's own masks/effects/transforms (its rendered output is the matte).
- Related: **Preserve Underlying Transparency (T)**, and **Stencil/Silhouette blending modes** which matte *all* layers below in the comp rather than one target.

### 6.5 Blending modes

38 modes in 8 documented categories (menu order):

- **Normal**: Normal, Dissolve, Dancing Dissolve.
- **Subtractive (darken)**: Darken, Multiply, Color Burn, Classic Color Burn, Linear Burn, Darker Color.
- **Additive (lighten)**: Add, Lighten, Screen, Color Dodge, Classic Color Dodge, Linear Dodge, Lighter Color.
- **Complex**: Overlay, Soft Light, Hard Light, Linear Light, Vivid Light, Pin Light, Hard Mix.
- **Difference**: Difference, Classic Difference, Exclusion, Subtract, Divide.
- **HSL**: Hue, Saturation, Color, Luminosity.
- **Matte**: Stencil Alpha, Stencil Luma, Silhouette Alpha, Silhouette Luma (gate/black-out all layers below).
- **Utility**: Behind *(verify — Behind is Photoshop)*, Alpha Add (adds alphas without re-multiplying — fixes seams on edge-abutting layers), Luminescent Premul (composites premultiplied HDR glows without clipping).
- "Classic" variants are AE 4.x-compatible maths kept for legacy projects. Blend maths runs in working colour space; "Blend colors using 1.0 gamma" project setting changes results.

### 6.6 Masks

- Masks are **bezier paths on a layer** (or **RotoBezier**, where tangents are automatic with per-vertex tension). Unlimited masks per layer, listed in draw order. Also parametric-from-tools rect/ellipse/polystar shapes converted to beziers.
- Per-mask properties: **Mask Path** (keyframable; first-vertex controls interpolation correspondence), **Mask Feather** (x/y separable Gaussian-style falloff), **Mask Opacity**, **Mask Expansion** (grow/shrink in px), **Inverted** checkbox.
- **Mask modes** combine top-to-bottom into the layer's transparency: **None** (path inert — used for effects paths/text-on-path), **Add**, **Subtract**, **Intersect**, **Lighten** (max where overlapping), **Darken** (min), **Difference** (XOR).
- **Variable-width feather** (CS6+): the Mask Feather tool places feather points along the path with independent radii/tension.
- Masks precede effects in the per-layer render order (masks → effects → transforms); effects like Stroke/Fill can reference mask paths. Smart Mask Interpolation panel assists vertex-matched morphing between mask keyframes.

### 6.7 Rotoscoping tools

- **Roto Brush** (tool used in the Layer panel): paint green foreground / red (Alt) background strokes on a **base frame**; a segmentation **span** propagates the matte frame-to-frame; corrections on any frame update propagation. Backed by the "Roto Brush & Refine Edge" effect on the layer. **Freeze** button caches/locks the segmentation for the span (stored in project on disk cache) so it stops re-propagating; unfreeze to edit.
- **Refine Edge tool**: partial-transparency band for hair/motion blur; **Refine Hard Matte / Refine Soft Matte** effects expose the same cleanup (smooth, feather, contrast, shift edge, chatter reduction, motion-blur-aware).
- **Roto Brush 2.0** (2020) replaced the propagation with an ML model; **Roto Brush 3.0** (2023/24.x) upgraded the model again (better with hair, overlapping limbs, translucency).
- Adjacent: keyframed masks remain the precision fallback; **mocha AE** (bundled CEP plugin) does planar-tracked splines exportable as AE masks; **Content-Aware Fill** panel (2019+) removes objects using surrounding-frame synthesis.

---

## 7. Caching & preview architecture

### 7.1 Renderer history — why AE previews get slow

- AE's renderer historically produced **one frame at a time on a single render thread**: for each frame, walk the comp's dependency graph bottom-up (per layer: source → masks → effects → transform → blend/matte into the stack), with each effect a serial node. CPU parallelism existed only *inside* well-threaded effects.
- CS3–CC 2015 had "Render Multiple Frames Simultaneously" (forked whole background processes, RAM-hungry, fragile); it was **removed in 13.5 (CC 2015)** when the architecture was rebuilt to separate the UI thread from a single render thread (responsiveness over throughput) — leaving AE effectively single-frame for ~7 years.
- Other slowness sources: everything renders at comp bit depth (8/16/32f conversions), colour management transforms, effects mostly CPU (a GPU-accelerated subset exists), expressions evaluated per frame per property, nested comp intermediates, and frame blending/motion blur multiplying sample counts.

### 7.2 Multi-Frame Rendering (MFR, 2022 / v22.0)

- Re-architecture allowing **multiple frames to render concurrently on separate threads within one process** for previews, Render Queue, AME, and aerender. Requires effects to declare **thread safety**; a single non-thread-safe effect drops that comp branch to single-frame concurrency.
- **Dynamic Composition Analysis** continuously profiles render cost and available headroom, adjusting concurrency on the fly (up to ~10–17 concurrent frames observed on big machines; scales with cores *and* RAM since every concurrent frame needs its own working buffers — Adobe guidance ≈ 2 GB+ RAM per additional render thread). Targets keeping the machine below full saturation so the UI stays responsive; uses up to ~90% of available cores *(verify %)*.
- **Speculative Preview**: when idle, AE renders the comp in the background to fill the cache (Composition > Preview > Cache Frames When Idle).
- **Composition Profiler / Render Time indicator** (timeline column): per-layer/per-effect render-time readout to find bottlenecks.

### 7.3 Caches

- **RAM cache (green bars)** in the time ruler of Timeline/Layer/Footage panels = frames held in RAM, playable in real time. Filled by playback ("cache before playback" renders ahead then plays), scrubbing, idle speculative rendering.
- **Disk cache (blue bars)** = frames persisted to the disk-cache folder (Preferences > Media & Disk Cache: location + max size). AE **never plays directly from disk cache**; blue frames are promoted (fast-loaded) into RAM before playback. The disk cache **persists across sessions** ("persistent disk cache").
- **Global Performance Cache** (since CS6): cached frames are keyed by a **hash of everything contributing to the frame**, not by timeline position — so a cached frame is reused if a layer is moved in time, a comp is duplicated, an edit is undone/redone, or the same nested comp appears elsewhere. This is why undoing an edit instantly restores green bars.
- **Invalidation**: any change that alters a frame's dependency hash invalidates exactly the affected frames (edit an effect parameter → all frames of that comp span where the layer is visible go un-cached; upstream footage changes propagate down). Old frames are evicted LRU when RAM fills. RAM cache clears on quit; disk cache survives. Manual purge: **Edit > Purge > All Memory & Disk Cache / All Memory / Image Cache Memory / Snapshot**.
- **Media cache** (separate concept, shared with Premiere): conformed audio (CFA/PEK) and imported-media accelerators (MPEG index) — not rendered frames.
- **Preview (RAM preview) mechanics** today: press Space/0 → AE renders forward from the play position into RAM cache at the Preview panel's resolution/fps/skip settings; when "Cache Before Playback" is on it fills the range first, then plays in real time; uncached playback plays as fast as it renders unless frames are skipped to keep audio sync ("mercy playback" *(informal term)*).
- **Pre-render / proxies**: Render-queue a nested comp and set the result as a **proxy** (post-render action "Import & Replace Usage" / "Set Proxy") — the manual escape hatch for expensive subtrees; proxy toggles per item in Project panel; Render Settings choose "Use Proxies" or not.

---

## 8. Render Queue & output

### 8.1 Render Queue panel

- Comps are queued (Ctrl+M / Composition > Add to Render Queue). Each **render item** = one render pass with: **Render Settings**, ≥1 **Output Module(s)** (one render, multiple simultaneous encodes), output path, status (Queued/Needs Output/Rendering/Done/Failed), log, and elapsed/remaining readouts. "Notify" bell per item (2022+ sends OS/CC-app notification).
- **Render Settings** (templates: Best Settings, Draft Settings, DV Settings, Multi-Machine, Custom; editable via Edit > Templates > Render Settings): Quality (Best/Draft/Wireframe), Resolution (Full/Half/…), Disk Cache use, Proxy Use, Effects (Current/All On/All Off), Solo Switches (Current/All Off), Guide Layers, Color Depth override, Frame Blending (On for Checked Layers/Off), Motion Blur (On for Checked Layers/Off), Field Render/Pulldown, Time Span (work area / comp length / custom), custom Frame Rate override, Skip Existing Files (for multi-machine image-sequence rendering).
- **Output Module** (templates: Lossless [default], Lossless with Alpha, AIFF, etc.): Format (QuickTime, AVI, **H.264 — re-added natively in 22.3 with 4444/10-bit options via AME-derived encoder** *(verify exact codecs list per version)*, DPX/Cineon, OpenEXR (+sequences), PNG/TIFF/JPEG sequences, ProRes on all platforms since ~17.x, WAV/AIFF/MP3 audio), format options (codec settings), **Channels: RGB / Alpha / RGB+Alpha**, Depth, **Color: Straight vs Premultiplied**, colour-profile embedding/output colour management, Resize (with lock aspect + quality), Crop (T/L/B/R or use ROI), Audio (rate/depth/channels, on/off/auto). **Post-Render Action**: None / Import / Import & Replace Usage / Set Proxy.
- **aerender** — headless CLI renderer binary shipping with AE (`aerender -project x.aep -comp "Comp 1" -RStemplate ... -OMtemplate ... -output ...`), supports MFR flags; the basis of most render-farm integrations.

### 8.2 Adobe Media Encoder handoff

- **Composition > Add to Adobe Media Encoder Queue** (Ctrl+Alt+M) or the Render Queue's "Queue in AME" button sends the comp via **Dynamic Link**: AME hosts a headless AE instance that renders frames on demand while AME encodes — the project stays live (subsequent saves can change the render if not yet processed *(AME snapshots on queue by default — verify setting)*).
- **What transfers**: comp reference + output filename/destination. **What does not**: Render Queue output-module settings (format/channels) — AME uses its own format/preset system (H.264/HEVC hardware encoding, adaptive bitrate presets, destination publishing). Render Settings templates also don't map. This split (RQ = quality-first/frame formats; AME = delivery codecs/background encode) is a workflow fact of life Adobe documents explicitly.

---

## 9. Expressions

### 9.1 Language & engines

- Expressions are **per-property scripts whose final evaluated statement's value replaces (or is combined with) the property's keyframed value, every frame**. They cannot modify other properties (read-only graph access) — one property, one output, matching the property's dimension count (Number, Array [2]/[3]/[4], String, Path).
- **Engines**: since 16.0 (CC 2019) the default is a modern **JavaScript engine (V8, ECMAScript 2018)**; the **Legacy ExtendScript** engine (ES3, 1999) remains a per-project setting (File > Project Settings > Expressions). JS engine is stricter (if/else syntax, `this` binding differences, `snapshot`-style preprocessing of legacy idioms is applied except inside `eval()` or `.jsx` function libraries). JS engine supports modern syntax (let/const, arrow functions, template literals) and is markedly faster.
- Editor: inline in the timeline (expression field per property, enable toggle =, show-post-expression-graph button, **pick whip** drag-to-write property references, language menu of snippets), with a resizable editor, syntax highlighting, and error ribbon (expression errors disable that expression and show a warning banner — render continues with keyframe value).

### 9.2 Object model / property linking

- Global scope per evaluation: `thisComp`, `thisLayer`, `thisProperty`, `time` (comp time in **seconds** at the evaluated frame), `value` (the pre-expression value), `comp("name")`, `footage("name")`.
- Traversal: `thisComp.layer("Null 1").transform.position`, `layer.effect("Slider Control")("Slider")`, mask/path access `content("Shape 1").content("Path 1").path`, text `text.sourceText` (incl. per-style API `.style.setFontSize()` since 17.0), camera/light attributes, `layer.marker.key(1).time`, comp markers.
- Property methods: `valueAtTime(t)`, `velocity`, `velocityAtTime(t)`, `speed`, `speedAtTime(t)`, `numKeys`, `key(i)` (.time/.value/.index), `nearestKey(t)`, `propertyGroup(n)`, `wiggle(...)`, `temporalWiggle(...)`, `smooth(width, samples, t)`, `loopIn/loopOut/loopInDuration/loopOutDuration`.
- Layer space transforms: `toComp()`, `fromComp()`, `toWorld()`, `fromWorld()`, `toCompVec()` etc. — essential for expression-driven parenting/screen-space math. `sourceRectAtTime(t, includeExtents)` for text/shape bounds. `sampleImage(point, radius, postEffect, t)` reads rendered pixels (expensive).
- Interpolation/util: `linear(t, tMin, tMax, v1, v2)`, `ease()`, `easeIn()`, `easeOut()` (Hermite ease), `clamp`, `length`, `normalize`, `lookAt`, `seedRandom(seed, timeless)`, `random()`, `gaussRandom()`, `noise()` (Perlin), `timeToFrames()/framesToTime()`, `posterizeTime(fps)` (locks evaluation clock to a lower rate), `createPath(points, inTangents, outTangents, closed)`.

### 9.3 Canonical functions

- `wiggle(freq, amp, octaves = 1, amp_mult = 0.5, t = time)` — fractal-noise offset **added to the property value**, matching its dimension; per-layer/property deterministic seed (stable across renders; `seedRandom` shifts it). Runs in the property's own space (position wiggle is pre-transform).
- `loopOut(type = "cycle", numKeyframes = 0)` / `loopIn(...)` — extrapolate beyond the last/first keyframe: **"cycle"** (repeat), **"pingpong"** (alternate reversed), **"offset"** (repeat with cumulative value offset), **"continue"** (extend at final velocity — no numKeyframes arg). `numKeyframes` counts *segments back from the end* to loop (0 = all). `loopOutDuration(type, duration)` loops a time span instead.
- Time-based retiming idioms: `valueAtTime(time - delay)` (echo/followers), Time Remap + `loopOut()` (footage looping), index-based delays `delay*index` for cascades.

### 9.4 Performance behaviour

- Expressions are evaluated **per property, per rendered frame** — an expensive expression on a heavily-instanced property (e.g. 500 layers × wiggle) can dominate render time; the Composition Profiler shows this since 2022.
- **17.0 (2020) optimisations**: AE detects expressions whose result **cannot change over time** and evaluates them **once per comp** instead of per frame; anything wrapped in `posterizeTime()` evaluates at the posterised rate only; expression evaluation was moved onto the render thread with a rewritten evaluator (Adobe claimed up to ~5× on expression-heavy comps *(marketing figure)*), and time-remap+expression interaction was sped up.
- Expressions are **thread-safe under MFR** (each render thread evaluates independently), but expressions that pull rendered pixels (`sampleImage`) or force other layers/comps to render at other times (`valueAtTime` on rendered content is fine — it's property data; sampleImage is the pixel case) create cross-frame dependencies that serialise work.
- Expression *errors* are per-frame: an expression can succeed at t=0 and fail later (e.g. out-of-range key index); AE disables it at first failure with a banner.
- **Master Properties / Essential Properties** (15.1+) interact with expressions: a nested comp's exposed properties can be driven per-instance in the parent, effectively parameterising precomps (choice: "use instance value" pulls parent-set values through the nested comp's expressions).

---

## Appendix: render order (single layer, classic pipeline)

`source → masks (top-to-bottom, mode-combined) → effects (top-to-bottom) → layer styles → transform (anchor/pos/scale/rot) → motion blur sampling → blend mode / track matte / preserve-transparency compositing into stack below` — continuously-rasterised/collapsed layers move rasterisation after transform; adjustment layers substitute "composite below" as their effect input. Comp-level order is bottom layer first (2D), or z-sorted per 3D renderer within contiguous 3D sets, with 2D layers acting as render breaks.

## Key sources

- helpx.adobe.com/after-effects/using/: workspaces-panels-viewers, previewing, composition-basics, modifying-using-views, keyframe-interpolation, speed, time-stretching-time-remapping, precomposing-nesting-pre-rendering, track-mattes-and-traveling-mattes, blending-modes-layer-styles, alpha-channels-masks-mattes, roto-brush-refine-matte, memory-storage1, multi-frame-rendering, basics-rendering-exporting, legacy-and-extend-script-engine, improve-performance
- ae-scripting.docsforadobe.dev (KeyframeEase: speed float, influence 0.1–100), ae-expressions.docsforadobe.dev, ae-plugins.docsforadobe.dev (MFR thread-safety)
- ProVideo Coalition — Chris Zwar: "AE 2022: Multi-Frame Rendering has arrived", "After Effects & Performance" series (parts 2, 14)
- Puget Systems MFR hardware analyses; School of Motion (caching, graph editor, render via AME); screenlight.tv "Definitive guide to RAM previews and disk caches"; frame.io blog (2023 track mattes); designkkashi.com graph editor guide
