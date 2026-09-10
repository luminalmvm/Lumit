# The camera

How the Camera layer is modelled, placed, drawn through, moved and migrated. This note is
the authoritative how for docs/03 §5.6 and §9.3, docs/06 §1.4 and docs/07 §2.3.5. The specs
say what; this says how, with the arithmetic that has to agree across four crates and one
Dart file.

## 1. The model

`LayerKind::Camera` carries everything a camera is beyond its placement:

```rust
Camera {
    /// Focal distance in comp pixels: the plane that far in front of the eye renders 1:1.
    zoom: Property,
    /// One-node aims by its rotation; two-node aims at `point_of_interest`.
    two_node: bool,
    /// Where a two-node camera looks, in comp pixels. Inert on a one-node camera,
    /// but kept, so switching node types loses nothing.
    point_of_interest: [Property; 3],
    depth_of_field: bool,
    /// Distance from the eye to the sharp plane, comp pixels.
    focus_distance: Property,
    /// Circle-of-confusion scale, comp pixels. See §5 for the units.
    aperture: Property,
    /// Percent, 100 is the full circle of confusion.
    blur_level: Property,
    /// Focus distance follows zoom while set; the dialog writes both.
    lock_to_zoom: bool,
    /// The sensor's width in millimetres, the presentation unit the settings
    /// dialog converts zoom through. Never read by the renderer.
    film_size_mm: f64,
    solve_link: Option<Uuid>,
    correction_base: Option<Box<CameraPose>>,
}
```

Every new field has a serde default so a file from before this note opens unchanged:
one-node, point of interest at the old position (§7), depth of field off, focus distance
equal to zoom, aperture `zoom / 56`, blur level 100, film size 36.

**Position is the eye.** The layer's `position_x/y/z` is where the camera sits, as in After
Effects. The plane `zoom` in front of it along its forward axis renders 1:1 and centred. A
camera created by `add_camera_layer` sits at `(cx, cy, -zoom)` looking down +z, so a fresh
camera changes no picture. Before this note the position was the point looked at with the
eye `zoom` behind it; §7 converts old files.

The placement rows a camera shows are Point of interest (two-node only), Position with all
three axes, and the three rotations. No anchor, scale or opacity: they mean nothing on a
viewpoint and were rows that did nothing. The same cut applies to a Light. Neither kind has
a 3D switch: a camera and a light are three-dimensional by being what they are, and the
Timeline's 3D cell is blank on their rows. Camera options are a second heading under the
Transform rows: Zoom, Depth of field, Focus distance, Aperture, Blur level.

The animatable camera channels ride `TransformProp`: `PoiX`, `PoiY`, `PoiZ`, `Zoom`,
`FocusDistance`, `Aperture`, `BlurLevel`. `Layer::prop(prop)` answers the eleven transform
channels from the transform group and the seven camera channels from the kind, `None` on a
layer that is not a camera. `Op::SetTransformProperty` writes through it and refuses a
camera channel on any other layer with `OpError::PropNotOnLayer`. This is what gives the new
rows their stopwatch, lane, graph curve, reveal key and expression for nothing: every surface
walks `transformGroups`, and the list is the only thing that grew.

## 2. The pose

`CameraPose` is the evaluated placement the renderer and the tools share:

```rust
pub struct CameraPose {
    pub zoom: f64,
    /// The eye, comp pixels.
    pub position: (f64, f64, f64),
    /// The effective rotation in degrees, the compositor's `Ry · Rx · Rz` order.
    pub rotation_deg: (f64, f64, f64),
    /// Present while depth of field is on.
    pub dof: Option<CameraDof>,
}
pub struct CameraDof { pub focus_distance: f64, pub aperture: f64, pub blur_level: f64 }
```

A one-node camera's rotation is its stored rotation. A two-node camera's rotation is the
look-at rotation composed with the stored one, so the rows still add an offset on top of the
aim the way After Effects' Orientation does:

```rust
// forward under Ry·Rx·Rz is (sin y · cos x, -sin x, cos y · cos x)
let f = normalise(poi - eye);
let look = (asin(-f.y), atan2(f.x, f.z), 0.0);            // (rx, ry, rz)
let m = rot_y(look.1) * rot_x(look.0) * rot_y(ry) * rot_x(rx) * rot_z(rz);
rotation_deg = euler_yxz(m);                                // decompose, degrees
```

`euler_yxz` reads `M = Ry·Rx·Rz` back: `M[1][2] = -sin x`, `M[0][2] = sin y cos x`,
`M[2][2] = cos y cos x`, `M[1][0] = cos x sin z`, `M[1][1] = cos x cos z`, spending a
gimbal-locked turn on y. It is the same decomposition `euler_of_transpose` in
lumit-render's track module applies to a solve. A point of interest on top of the eye has no
direction; the aim is then the stored rotation alone.

The solve link's correction lane (`track::correct`) stays channel-wise and passes the stored
pose's `dof` through unchanged: a solve has no depth of field.

## 3. The matrix

`lumit_gpu::camera_matrix` keeps its perspective, `x' = cx + (x - cx) · zoom / (z' + zoom)`
in a space where the eye sits at `z' = -zoom`. Only the placement changes: the camera's own
frame is undone by

```
cam_place = T(position) · Ry(ry) · Rx(rx) · Rz(rz) · T(-cx, -cy, +zoom)
matrix    = persp · cam_place⁻¹
```

The last translation is what moved: it was `T(-cx, -cy, 0)` when the position was the plane
looked at. The default camera at `(cx, cy, -zoom)` makes `cam_place` the identity, so an
untouched camera still draws every 3D layer exactly where the 2D pass would.

A solve converts to this with `position = C`, the camera centre itself, in place of the old
`C + Rᵀ·(0, 0, f)` push. `zoom = f` and the Euler angles of `Rᵀ` are unchanged.

## 4. The tools

The tools work in the eye model. The **pivot** is the point of interest on a two-node camera
and `eye + zoom · forward` on a one-node one. The axes come from the effective rotation with
the same column formulas as before (`viewer_camera.dart`, `CameraPose.axes`).

- **Orbit** swings the eye round the pivot. One-node: new angles from the drag, then
  `eye = pivot - zoom · forward(new)`, and the rotation rows take the new angles. Two-node:
  the eye moves to `poi - |eye - poi| · forward(new)` and the rotation rows are left alone,
  because the aim follows the eye by itself. The pitch clamps at `±89.9°` as before.
- **Track** slides the eye along right and up against the drag, divided by the Viewer's
  magnification. A two-node camera's point of interest slides with it, so the framing
  moves rather than the aim.
- **Dolly** slides the eye along forward by `drag · 0.0015 · zoom`. A two-node camera's
  point of interest stays, so a dolly changes the distance to it.
- **Unified** is the three on one tool: the left button orbits, the middle tracks, the
  right dollies. It is the first tool of the camera group, as it is under After Effects'
  `C`.

In a view other than the active camera (§6) the same drags move the **view**, never a
layer, which is what After Effects does with its custom views.

## 5. Depth of field

Applied in `realise_segment`, after the lighting pass and before placement, to every 3D
layer while the active camera carries a `dof`. The circle of confusion on screen, in comp
pixels:

```
d      = dot(anchor_world - eye, forward)          // the layer's depth, comp px
coc    = aperture · |d - focus| / d · (zoom / focus) · blur_level / 100
radius = coc / 2 · d / zoom / (|scale| / 100) · render_scale    // in the layer's own texels
```

`anchor_world` is the layer's position taken through its parent placement (`pre`). A layer
at or behind the eye (`d ≤ 0`) and a radius under half a texel are left sharp. The blur is
`FxEngine::blur` at that radius with edge repeat, the same separable gaussian every Blur
effect uses. The whole layer takes one radius, read at its anchor: a plane leaning through
the focus plane blurs evenly rather than across itself.

The units are After Effects' rather than a lens maker's. Aperture is a scale in comp
pixels, not a diameter; the dialog's F-stop is `zoom / (10 · aperture)`, which is the
relation the After Effects dialog shows between its own three numbers, and the default
aperture `zoom / 56` reads f/5.6. Iris shape, rotation, roundness, diffraction fringe and the
highlight controls are not built.

The pose carries the `dof`, and the frame key hashes the pose, so switching depth of field
on, or dragging Aperture, names a different frame. Nested comps keep their own cameras and
their own depth of field.

## 6. Views and wireframes

The Viewer's 3D view is panel state, like its channel and magnification: Active camera,
Front, Back, Left, Right, Top, Bottom, Custom view 1 to 3. A view other than Active camera
renders the composition through a pose of the view's own, handed to
`render_frame_with_view`, which replaces the top-level comp's camera before the draws are
built. The frame key sees the replaced pose, so a view's frames never displace the active
camera's. Export never sees a view.

The six fixed views look at the comp centre `(cx, cy, 0)` from `D = 100 · width` away with
`zoom = D`, which is orthographic to the pixel. Their rotations, from the forward formula
in §2: Front `(0, 0, 0)`; Back `(0, 180, 0)`; Left `(0, 90, 0)`; Right `(0, -90, 0)`; Top
`(-90, 0, 0)`; Bottom `(90, 0, 0)`. The custom views start at a three-quarter view,
`(-25, 35, 0)` from `zoom = width · 50/36 · 2` away, and keep whatever the camera tools do
to them until the panel closes. `lumit_core::camera::view_pose` builds all of them;
the frontend holds the custom ones once it has moved them.

In every view but Active camera the picture wears **wireframes**: each 3D layer's rectangle,
each camera's frustum (the eye, the four corners of the comp-sized rectangle at its zoom
along its own axes, the four edges joining them, and a line to the point of interest on a
two-node camera), and each light as a small diamond. `CompositionReference::wireframes(frame,
view)` gathers them in the engine and returns every point already projected to comp pixels
through the view's own matrix, with a flag for points behind the view's eye, which the
painter skips. Asked once per frame change or view change, never per rebuild. Theme colours:
layers in `outline`, cameras in the theme's camera colour, lights in its light colour.

## 7. Migration

Schema `0.2.0` becomes `0.3.0`. The migration visits every Camera layer of every comp in the
raw JSON and:

1. copies the old position into `point_of_interest`, which is exactly what it was;
2. rewrites `position_x/y/z` as `old - zoom · forward(rotation)`.

When zoom and both out-of-plane rotations are static the shift is one constant per axis,
subtracted from every keyframe's value with tangents untouched: exact. When any of them is
animated the three position properties are resampled at the union of all their key times
with linear keys, which is what a baked solve already is. `correction_base`, when present,
gets the same shift. A camera that had no rotation and no zoom keys, which is every camera
made by hand, converts without loss.

## 8. Import

An After Effects camera's `ADBE Position` is the eye and maps straight to position. `ADBE
Anchor Point` on a camera is its point of interest and maps to `point_of_interest`; the
camera is two-node when its auto-orient is `CAMERA_OR_POINT_OF_INTEREST`. `ADBE Camera
Depth of Field`, `ADBE Camera Focus Distance`, `ADBE Camera Aperture` and `ADBE Camera Blur
Level` map by name. The report row that said a point of interest was not carried goes.

## 9. Settings dialog

Layer > Camera settings edits the non-animatable part through one op,
`SetCameraSettings { two_node, depth_of_field, lock_to_zoom, film_size_mm }`, and writes
static values into the animatable rows through the ordinary `set_transforms`. Presets are
the After Effects focal lengths (15, 20, 24, 28, 35, 50, 80, 135, 200 mm). The lens
arithmetic lives in `lumit_core::camera` and crosses the bridge as sync calls:

```
focal_mm  = zoom · film_mm / comp_w
angle_deg = 2 · atan(comp_w / (2 · zoom))
f_stop    = zoom / (10 · aperture)
```

and their inverses. Units are pixels, millimetres or inches on the dialog's choice; the
document holds pixels and millimetres only.

## 10. Traps

- The axes in `viewer_camera.dart` are the columns of `Ry·Rx·Rz` and must stay so; the
  rotation the tools read is the *effective* one from `CameraPose`, never the rows on a
  two-node camera.
- `set_transforms` with `BridgeTransformProp.values` resets a layer; a camera channel on a
  footage layer is an error, so Reset uses the layer's own row list.
- The transform preview (`BridgeTransform::write_at`) writes camera channels into the kind,
  which needs the layer, not the transform group.
- The 3D cell being blank on a camera row does not make `switches.three_d` true; the rows
  come from the kind, and the renderer never reads the switch on a camera.
- A fixed view's `zoom` is huge and its eye far; `f32` in the matrix is fine at `100 ·
  width` and not at `1e6 · width`.

## 11. Test plan

- **lumit-core**: `euler_yxz` round-trips the three elementary rotations and their
  products; a two-node pose's forward points at the point of interest; a two-node pose with
  zero rows equals the one-node pose at the look-at angles; `Layer::prop` answers the seven
  camera channels on a camera and none elsewhere; `SetTransformProperty` on `Zoom` is
  invertible through the store; `view_pose` for Front is the default camera at distance
  `100 · width`, and Top looks down +y; the lens formulas invert each other.
- **lumit-project**: a 0.2.0 file with a hand-placed camera opens with the eye `zoom`
  behind the old position and the point of interest at the old position; an animated
  rotation resamples position at the union of key times; the schema reads 0.3.0.
- **lumit-gpu**: the default camera at `(cx, cy, -zoom)` composites a 3D layer where the
  2D pass does; a layer at `z = zoom` draws at half size.
- **lumit-render**: with depth of field on, a 3D layer at twice the focus distance comes
  out blurred and one on the focus plane does not; the frame key changes with `aperture`;
  `to_camera_pose` places the eye at the solve's centre and projects a world point where the
  compositor does.
- **lumit-import**: a two-node camera arrives two-node with its point of interest, and the
  removed report reason is gone from the report.
- **lumit-bridge**: the seven camera channels read and write through `get_transform` and
  `set_transforms`, and a camera channel on a solid is refused; `SetCameraSettings` round
  trips; `wireframes` projects the default camera's frustum corners onto the comp's corners
  in the Front view.
- **flutter_ui**: orbit keeps the pivot fixed in the eye model; a two-node orbit leaves the
  rotation rows alone; the unified tool picks its move by button; a camera row shows the
  eye, no anchor, scale or opacity, and a blank 3D cell; a light row likewise; the view
  picker renders the wireframes for a view and nothing in Active camera.
