// The camera tools: orbit, track and dolly the composition's camera
// (docs/07 §2.3.5, docs/impl/camera.md §4).
//
// **In plain terms.** A 3D composition is looked at through a **camera layer**,
// and these tools are how you move it by dragging on the picture instead of
// typing numbers into the Timeline. Orbit swings the camera around whatever it
// is pointed at; Track slides it sideways and up and down; Dolly moves it in
// and out; the unified tool is the three at once, one to a mouse button. They
// are After Effects' camera tools, and they act on the **active camera** - the
// topmost visible camera layer whose span covers the playhead - whatever is
// selected, because the camera is the thing you are looking through rather than
// a thing you are editing.
//
// **What the camera's numbers mean here.** A camera is an eye, three rotations
// and a *zoom* (the focal distance, in composition pixels). The layer's
// position is the eye, and the plane `zoom` in front of it along the camera's
// own forward axis renders 1:1 and centred. A camera can also be **two-node**:
// it is aimed at a point of interest rather than by its rotation rows, and the
// aim is composed into the pose the tools read.
//
// The **pivot** is what a drag turns around: the point of interest on a
// two-node camera, and `eye + zoom · forward` on a one-node one. Both sit dead
// centre of the frame, which is where the gizmo's mark goes.
//
// * **Orbit** swings the eye round the pivot. A one-node camera takes the new
//   angles and its eye moves to keep the pivot in front at `zoom`; a two-node
//   camera's eye circles the point of interest at the distance it already had,
//   and its rotation rows are left alone, because the aim follows the eye.
// * **Track** slides the eye along its own right and up axes, taking a two-node
//   camera's point of interest with it, so the framing moves rather than the
//   aim.
// * **Dolly** slides the eye along forward. The point of interest stays, so a
//   dolly changes the distance to it.
//
// In a view other than Active camera the same drags move the **view** rather
// than any layer. The six fixed views track and dolly but never turn: orbiting
// one carries it into the last custom view and turns that, as After Effects
// does.
//
// The maths below is pure and in composition pixels; the widget under it only
// turns drags into these calls and commits transform properties.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/system.dart';
import 'package:lumit_flutter/state/preview_throttle.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/state/viewer_view.dart';
import 'package:lumit_flutter/widgets/drag_escape.dart';

import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'transform_rows_frb.dart' show read;
import 'viewer_gizmo.dart' show LayerBox, layerAtPoint;
import 'viewer_layer_map.dart' show viewerNear;
import 'viewer_tool_cursor.dart';

/// How many degrees a pixel of drag turns the camera.
///
/// A full sweep of a 1000px-wide Viewer comes to about half a turn, which is
/// what After Effects' orbit feels like: enough to get round a scene in one
/// gesture, gentle enough to aim with.
const double orbitDegreesPerPixel = 0.25;

/// How much of the distance left a pixel of dolly drag covers.
///
/// A share rather than a length, so a dolly feels the same in a comp built at
/// 500 px and one built at 5000: dragging about 460 pixels halves the distance
/// to what you are dollying to, or doubles it going back.
const double dollyFraction = 0.0015;

/// How many pixels of dolly drag a pixel of wheel scroll is worth. A
/// Windows notch is 100 pixels of scroll, so a notch takes about 7% of the
/// distance.
const double wheelDollyPixels = 0.5;

/// The most scroll one wheel event counts for, so a coarse notch on one
/// platform is no bigger a step than a run of fine ones on another.
const double wheelNotchPixels = 120;

/// A camera's pose in the shape the tools work in: where the eye is, which way
/// it faces, how far ahead the plane it renders 1:1 sits, and what it is aimed
/// at when it is aimed at something.
///
/// [position] is the **eye** in the comp, parents and all. [rotation] is the
/// *effective* rotation - a two-node camera's aim is already composed into it  - 
/// so the axes below are always the ones the picture is drawn with.
@immutable
class CameraPose {
  final (double, double, double) position;

  /// Rotation in degrees about x, y and z, in the order the compositor applies
  /// them (`Ry · Rx · Rz`).
  final (double, double, double) rotation;

  /// The focal distance, comp pixels.
  final double zoom;

  /// Where a two-node camera looks. Kept but inert on a one-node one, exactly
  /// as the document keeps it.
  final (double, double, double) pointOfInterest;

  final bool twoNode;

  const CameraPose({
    required this.position,
    required this.rotation,
    required this.zoom,
    this.pointOfInterest = (0, 0, 0),
    this.twoNode = false,
  });

  CameraPose copyWith({
    (double, double, double)? position,
    (double, double, double)? rotation,
    double? zoom,
    (double, double, double)? pointOfInterest,
  }) =>
      CameraPose(
        position: position ?? this.position,
        rotation: rotation ?? this.rotation,
        zoom: zoom ?? this.zoom,
        pointOfInterest: pointOfInterest ?? this.pointOfInterest,
        twoNode: twoNode,
      );

  /// The camera's own three axes in composition space, from its rotations.
  ///
  /// Built with the compositor's own order (`Ry · Rx · Rz`, lumit-gpu's
  /// `camera_matrix`) so a tool moves the camera along the axes the picture is
  /// actually drawn with — the one place this arithmetic has to agree with the
  /// renderer exactly.
  ({
    (double, double, double) right,
    (double, double, double) up,
    (double, double, double) forward,
  }) get axes {
    final rx = rotation.$1 * math.pi / 180;
    final ry = rotation.$2 * math.pi / 180;
    final rz = rotation.$3 * math.pi / 180;
    final (cx, sx) = (math.cos(rx), math.sin(rx));
    final (cy, sy) = (math.cos(ry), math.sin(ry));
    final (cz, sz) = (math.cos(rz), math.sin(rz));

    // The columns of Ry · Rx · Rz.
    final right = (
      cy * cz + sy * sx * sz,
      cx * sz,
      -sy * cz + cy * sx * sz,
    );
    final up = (
      -cy * sz + sy * sx * cz,
      cx * cz,
      sy * sz + cy * sx * cz,
    );
    final forward = (
      sy * cx,
      -sx,
      cy * cx,
    );
    return (right: right, up: up, forward: forward);
  }

  /// The point a drag turns around: what a two-node camera is aimed at, and
  /// the middle of the 1:1 plane otherwise. Both land in the centre of frame.
  (double, double, double) get pivot {
    if (twoNode) return pointOfInterest;
    final f = axes.forward;
    return (
      position.$1 + f.$1 * zoom,
      position.$2 + f.$2 * zoom,
      position.$3 + f.$3 * zoom,
    );
  }
}

/// Shift: the larger of the two movements wins and the other is dropped, so a
/// level orbit stays level and a track keeps to one axis.
(double, double) _onOneAxis(double dx, double dy, bool lockAxis) {
  if (!lockAxis) return (dx, dy);
  return dx.abs() >= dy.abs() ? (dx, 0.0) : (0.0, dy);
}

/// [v] turned [deg] degrees about y, the way the compositor's `Ry` turns it.
(double, double, double) _aboutY(double deg, (double, double, double) v) {
  final a = deg * math.pi / 180;
  final (c, s) = (math.cos(a), math.sin(a));
  return (c * v.$1 + s * v.$3, v.$2, -s * v.$1 + c * v.$3);
}

/// [v] turned [deg] degrees about x, the way the compositor's `Rx` turns it.
(double, double, double) _aboutX(double deg, (double, double, double) v) {
  final a = deg * math.pi / 180;
  final (c, s) = (math.cos(a), math.sin(a));
  return (v.$1, c * v.$2 - s * v.$3, s * v.$2 + c * v.$3);
}

/// The pose after an **orbit** drag of [dx], [dy] screen pixels.
///
/// Horizontal movement swings the camera around the pivot (yaw); vertical
/// movement lifts it over the top or drops it underneath (pitch). The pivot
/// never moves, which is precisely what makes this an orbit rather than a pan.
/// It is [around] when given, a point anywhere in the comp, and the camera's
/// own [CameraPose.pivot] otherwise.
CameraPose orbitCamera(
  CameraPose pose,
  double dx,
  double dy, {
  bool lockAxis = false,
  (double, double, double)? around,
}) {
  final (ax, ay) = _onOneAxis(dx, dy, lockAxis);
  final (pitch0, yaw0, roll) = pose.rotation;
  final yaw = yaw0 + ax * orbitDegreesPerPixel;
  // Dragging **up** lifts the camera over the top, which means tilting it to
  // look *down* — a negative x rotation in the compositor's frame, where +y is
  // down the screen. Getting this the other way round is the classic inverted
  // orbit.
  //
  // Clamped rather than wrapped: past a quarter turn the camera is looking
  // straight down and the next pixel of drag flips the picture over, which no
  // orbit control anywhere does.
  final pitch = (pitch0 + ay * orbitDegreesPerPixel).clamp(-89.9, 89.9);
  final pivot = around ?? pose.pivot;
  // The swing is one turn of the whole scene about the pivot: back off the
  // old heading, tip by the change in pitch, round to the new heading. It
  // takes the camera's old axes to its new ones, so whatever it carries keeps
  // its place in the frame.
  (double, double, double) swing((double, double, double) p) {
    final r = _aboutY(
        yaw,
        _aboutX(pitch - pitch0,
            _aboutY(-yaw0, (p.$1 - pivot.$1, p.$2 - pivot.$2, p.$3 - pivot.$3))));
    return (pivot.$1 + r.$1, pivot.$2 + r.$2, pivot.$3 + r.$3);
  }

  return pose.copyWith(
    position: swing(pose.position),
    // A two-node camera's point of interest swings with it, which leaves it
    // where it is when it is the pivot. Its rows are left alone: its aim is
    // worked out from the eye and that point, so turning them as well would
    // double the swing.
    pointOfInterest:
        pose.twoNode ? swing(pose.pointOfInterest) : pose.pointOfInterest,
    rotation: pose.twoNode ? pose.rotation : (pitch, yaw, roll),
  );
}

/// The comp point at depth [w] under [at], comp pixels of a picture [size]
/// seen through [pose]. [w] is the depth over the zoom, as the view rows and
/// the layer maps measure it.
(double, double, double) pointUnder(
    CameraPose pose, Offset at, Size size, double w) {
  final a = pose.axes;
  final x = (at.dx - size.width / 2) * w;
  final y = (at.dy - size.height / 2) * w;
  final z = pose.zoom * w;
  final e = pose.position;
  return (
    e.$1 + a.right.$1 * x + a.up.$1 * y + a.forward.$1 * z,
    e.$2 + a.right.$2 * x + a.up.$2 * y + a.forward.$2 * z,
    e.$3 + a.right.$3 * x + a.up.$3 * y + a.forward.$3 * z,
  );
}

/// Where the comp point [p] lands in a picture [size] seen through [pose], in
/// comp pixels, or null when it is behind the eye.
Offset? pictureOf(CameraPose pose, (double, double, double) p, Size size) {
  final a = pose.axes;
  final d = (
    p.$1 - pose.position.$1,
    p.$2 - pose.position.$2,
    p.$3 - pose.position.$3
  );
  double dot((double, double, double) u) =>
      u.$1 * d.$1 + u.$2 * d.$2 + u.$3 * d.$3;
  final z = dot(a.forward);
  if (z <= 1e-6) return null;
  final k = pose.zoom / z;
  return Offset(
      size.width / 2 + dot(a.right) * k, size.height / 2 + dot(a.up) * k);
}

/// The pose after a **track** drag: the eye slides along its own right and up
/// axes, taking a two-node camera's point of interest with it.
///
/// The picture follows the pointer rather than running away from it — dragging
/// right moves the *view* right, which means moving the camera left, the same
/// sense the Hand tool has.
///
/// [scale] converts screen pixels to composition pixels (the Viewer's
/// magnification). [depth] is how far away the point that stays under the
/// pointer is, as depth over the zoom: 1 is the plane the camera sees 1:1.
/// After Effects' Pan under cursor gives it the depth of whatever was under
/// the pointer at the press.
CameraPose trackCamera(
  CameraPose pose,
  double dx,
  double dy, {
  required double scale,
  bool lockAxis = false,
  double depth = 1,
}) {
  final (ax, ay) = _onOneAxis(dx, dy, lockAxis);
  final k = (scale <= 0 ? 1.0 : 1 / scale) * depth;
  final right = pose.axes.right;
  final up = pose.axes.up;
  final mx = -ax * k;
  final my = -ay * k;
  final step = (
    right.$1 * mx + up.$1 * my,
    right.$2 * mx + up.$2 * my,
    right.$3 * mx + up.$3 * my,
  );
  return pose.copyWith(
    position: (
      pose.position.$1 + step.$1,
      pose.position.$2 + step.$2,
      pose.position.$3 + step.$3,
    ),
    pointOfInterest: pose.twoNode
        ? (
            pose.pointOfInterest.$1 + step.$1,
            pose.pointOfInterest.$2 + step.$2,
            pose.pointOfInterest.$3 + step.$3,
          )
        : pose.pointOfInterest,
  );
}

/// The pose after a **dolly** of [travel] pixels, After Effects' Dolly toward
/// cursor: the eye moves straight at [toward], and positive goes in.
///
/// Each pixel takes a share of the distance that is left, so the eye slows as
/// it closes in and never reaches the point. Without [toward] the camera goes
/// at its own pivot and a two-node camera's point of interest stays where it
/// is; towards anything else the point of interest travels with the eye, so
/// the aim is kept.
CameraPose dollyCamera(
  CameraPose pose,
  double travel, {
  (double, double, double)? toward,
}) {
  final t = toward ?? pose.pivot;
  final k = math.exp(-travel * dollyFraction);
  final e = pose.position;
  final eye = (
    t.$1 - (t.$1 - e.$1) * k,
    t.$2 - (t.$2 - e.$2) * k,
    t.$3 - (t.$3 - e.$3) * k,
  );
  final poi = pose.pointOfInterest;
  return pose.copyWith(
    position: eye,
    pointOfInterest: pose.twoNode && toward != null
        ? (
            poi.$1 + eye.$1 - e.$1,
            poi.$2 + eye.$2 - e.$2,
            poi.$3 + eye.$3 - e.$3,
          )
        : poi,
  );
}

/// The pose after a dolly of [travel] pixels in a fixed view, which stays
/// orthographic: the eye keeps its distance and the zoom grows instead, so
/// the picture is magnified and its perspective never changes. [at] is the
/// pointer in comp pixels from the middle of the picture, and what is under
/// it stays under it. [depth] is the distance to the plane the view frames.
CameraPose zoomView(
  CameraPose pose,
  double travel, {
  Offset at = Offset.zero,
  required double depth,
}) {
  final zoom = pose.zoom * math.exp(travel * dollyFraction);
  final shift = depth * (1 / pose.zoom - 1 / zoom);
  final a = pose.axes;
  final (x, y) = (at.dx * shift, at.dy * shift);
  final e = pose.position;
  return pose.copyWith(
    zoom: zoom,
    position: (
      e.$1 + a.right.$1 * x + a.up.$1 * y,
      e.$2 + a.right.$2 * x + a.up.$2 * y,
      e.$3 + a.right.$3 * x + a.up.$3 * y,
    ),
  );
}

/// [pose] moved along its own forward axis, its aim kept, so that [points]
/// fill nine tenths of a picture [size]: After Effects' Look at. A two-node
/// camera's point of interest goes to their middle. Null with no points.
CameraPose? framePoints(
    CameraPose pose, List<(double, double, double)> points, Size size) {
  if (points.isEmpty || size.isEmpty) return null;
  final lo = [double.infinity, double.infinity, double.infinity];
  final hi = [-double.infinity, -double.infinity, -double.infinity];
  for (final p in points) {
    final xyz = [p.$1, p.$2, p.$3];
    for (var i = 0; i < 3; i++) {
      lo[i] = math.min(lo[i], xyz[i]);
      hi[i] = math.max(hi[i], xyz[i]);
    }
  }
  final c = ((lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2);
  final a = pose.axes;
  final halfW = size.width * 0.45;
  final halfH = size.height * 0.45;
  double dot((double, double, double) u, (double, double, double) v) =>
      u.$1 * v.$1 + u.$2 * v.$2 + u.$3 * v.$3;
  // How far back from the middle the eye has to stand for each point to land
  // inside the frame, and in front of the eye.
  var reach = 0.0;
  for (final p in points) {
    final r = (p.$1 - c.$1, p.$2 - c.$2, p.$3 - c.$3);
    final z = dot(r, a.forward);
    reach = math.max(reach, dot(r, a.right).abs() * pose.zoom / halfW - z);
    reach = math.max(reach, dot(r, a.up).abs() * pose.zoom / halfH - z);
    reach = math.max(reach, pose.zoom * 0.05 - z);
  }
  final f = a.forward;
  return pose.copyWith(
    position: (c.$1 - f.$1 * reach, c.$2 - f.$2 * reach, c.$3 - f.$3 * reach),
    pointOfInterest: pose.twoNode ? c : pose.pointOfInterest,
  );
}

/// [framePoints]' twin for a fixed view, which stays orthographic: the eye
/// slides sideways until the middle of [points] is on its axis and keeps its
/// distance, and the zoom grows or shrinks until they fill nine tenths of a
/// picture [size]. [depth] is the distance to the plane the view frames.
/// Null with no points.
CameraPose? frameFlat(CameraPose pose, List<(double, double, double)> points,
    Size size, double depth) {
  if (points.isEmpty || size.isEmpty || depth <= 0) return null;
  final a = pose.axes;
  final e = pose.position;
  double dot((double, double, double) u, (double, double, double) p) =>
      u.$1 * (p.$1 - e.$1) + u.$2 * (p.$2 - e.$2) + u.$3 * (p.$3 - e.$3);
  final xs = [for (final p in points) dot(a.right, p)];
  final ys = [for (final p in points) dot(a.up, p)];
  final (x0, x1) = (xs.reduce(math.min), xs.reduce(math.max));
  final (y0, y1) = (ys.reduce(math.min), ys.reduce(math.max));
  final (mx, my) = ((x0 + x1) / 2, (y0 + y1) / 2);
  // A point `x` off the axis on the framed plane sits `x · zoom / depth`
  // from the middle of the picture.
  final fits = [
    if (x1 - x0 > 1e-9) size.width * 0.45 / ((x1 - x0) / 2),
    if (y1 - y0 > 1e-9) size.height * 0.45 / ((y1 - y0) / 2),
  ];
  return pose.copyWith(
    position: (
      e.$1 + a.right.$1 * mx + a.up.$1 * my,
      e.$2 + a.right.$2 * mx + a.up.$2 * my,
      e.$3 + a.right.$3 * mx + a.up.$3 * my,
    ),
    zoom: fits.isEmpty ? pose.zoom : depth * fits.reduce(math.min),
  );
}

/// A camera layer at the top of the active view's comp, placed where that
/// custom view looks from, as one undo step. False in any other view.
bool cameraFromView(LumitState app, LumitUiState ui) {
  final view = ui.views.active;
  final comp = view == null ? null : ui.compFor(view);
  if (view == null || comp == null || !ui.viewOf(view.engineId).orbits) {
    return false;
  }
  final seen = ui.viewPoseOf(view.engineId);
  if (seen == null) return false;
  final pose = poseOfView(seen);
  final at = pose.pivot;
  LayerReference? made;
  try {
    asOneUndoStep(app.project, () {
      final camera = comp.addCameraLayer();
      made = camera;
      camera.setTransforms(props: const [
        BridgeTransformProp.positionX,
        BridgeTransformProp.positionY,
        BridgeTransformProp.positionZ,
        BridgeTransformProp.rotationX,
        BridgeTransformProp.rotationY,
        BridgeTransformProp.rotation,
        BridgeTransformProp.zoom,
        BridgeTransformProp.poiX,
        BridgeTransformProp.poiY,
        BridgeTransformProp.poiZ,
      ], values: [
        for (final v in [
          pose.position.$1,
          pose.position.$2,
          pose.position.$3,
          pose.rotation.$1,
          pose.rotation.$2,
          pose.rotation.$3,
          pose.zoom,
          at.$1,
          at.$2,
          at.$3,
        ])
          BridgeScalar.static_(v),
      ]);
    });
  } catch (_) {
    // A comp that has gone.
    return false;
  }
  app.notifyDocumentChanged();
  ui.model.refresh();
  if (made case final camera?) ui.setSelection([camera]);
  return true;
}

/// Which move a mouse button asks the unified tool for: left orbits, middle
/// tracks, right dollies (docs/impl/camera.md §4).
ToolMode cameraMoveForButtons(int buttons) {
  if (buttons & kMiddleMouseButton != 0) return ToolMode.cameraPan;
  if (buttons & kSecondaryMouseButton != 0) return ToolMode.cameraDolly;
  return ToolMode.cameraOrbit;
}

/// A view's pose as the tools work in it: one-node, and never aimed.
CameraPose poseOfView(BridgeCameraPose view) => CameraPose(
      position: (view.x, view.y, view.z),
      rotation: (view.rotationX, view.rotationY, view.rotationZ),
      zoom: view.zoom,
    );

/// The same pose on the way back to the engine.
BridgeCameraPose viewOfPose(CameraPose pose) => BridgeCameraPose(
      zoom: pose.zoom,
      x: pose.position.$1,
      y: pose.position.$2,
      z: pose.position.$3,
      rotationX: pose.rotation.$1,
      rotationY: pose.rotation.$2,
      rotationZ: pose.rotation.$3,
    );

/// Put [pose]'s [props] on the camera [layer] at [frame], starting from its
/// transform [tf]: a keyed property takes a key there and a still one stays
/// still, as typing the value into its row would. Every camera write goes
/// through here, with the pose in the comp's own space. False when the
/// camera has gone.
bool writeCameraPose(
  LayerReference layer,
  BridgeTransform tf,
  CameraPose pose,
  Set<BridgeTransformProp> props, {
  required CompositionReference comp,
  required int frame,
}) {
  try {
    // A parented camera's own values are in its parent's space, so the
    // engine carries the pose back there and that is what is written.
    final own = layer.cameraOwnAim(
      frame: BigInt.from(frame),
      aim: BridgeCameraAim(
        pose: viewOfPose(pose),
        poiX: pose.pointOfInterest.$1,
        poiY: pose.pointOfInterest.$2,
        poiZ: pose.pointOfInterest.$3,
      ),
    );
    double valueOf(BridgeTransformProp prop) => switch (prop) {
          BridgeTransformProp.positionX => own.pose.x,
          BridgeTransformProp.positionY => own.pose.y,
          BridgeTransformProp.positionZ => own.pose.z,
          BridgeTransformProp.rotationX => own.pose.rotationX,
          BridgeTransformProp.rotationY => own.pose.rotationY,
          BridgeTransformProp.rotation => own.pose.rotationZ,
          BridgeTransformProp.zoom => pose.zoom,
          BridgeTransformProp.poiX => own.poiX,
          BridgeTransformProp.poiY => own.poiY,
          BridgeTransformProp.poiZ => own.poiZ,
          // Nothing else is a camera's placement, so nothing else is asked.
          _ => 0,
        };
    layer.setTransforms(
      props: props.toList(),
      values: [
        for (final prop in props)
          scalarWithValueAt(read(tf, prop), valueOf(prop), comp, frame),
      ],
    );
    return true;
  } catch (_) {
    return false;
  }
}

/// The camera tools over the picture.
class ViewerCameraLayer extends StatefulWidget {
  /// Whether a camera tool is armed. Inert otherwise.
  final bool active;

  final ToolMode tool;
  final CompositionReference comp;

  /// The engine id of the view the tools are used in, which is the view a
  /// custom view drag moves.
  final int viewId;
  final LumitState state;
  final LumitUiState uiState;

  /// Where the picture sits on screen, for the pivot mark and the drag's scale.
  final Rect fitted;

  /// The composition's own size, for the same two.
  final Size compSize;

  /// Every layer's box, asked for when an orbit needs what is under the
  /// pointer, the scene or the selection.
  final List<LayerBox> Function() boxes;

  final Color mark;
  final Color outline;
  final Color accent;

  final VoidCallback onChanged;

  const ViewerCameraLayer({
    super.key,
    required this.active,
    required this.tool,
    required this.comp,
    required this.viewId,
    required this.state,
    required this.uiState,
    required this.fitted,
    required this.compSize,
    required this.boxes,
    required this.mark,
    required this.outline,
    required this.accent,
    required this.onChanged,
  });

  @override
  State<ViewerCameraLayer> createState() => _ViewerCameraLayerState();
}

class _ViewerCameraLayerState extends State<ViewerCameraLayer> {
  Offset? _pointer;

  @override
  void initState() {
    super.initState();
    widget.uiState.viewerCameraRequest.addListener(_onCommand);
  }

  @override
  void didUpdateWidget(ViewerCameraLayer old) {
    super.didUpdateWidget(old);
    if (old.uiState == widget.uiState) return;
    old.uiState.viewerCameraRequest.removeListener(_onCommand);
    widget.uiState.viewerCameraRequest.addListener(_onCommand);
  }

  @override
  void dispose() {
    widget.uiState.viewerCameraRequest.removeListener(_onCommand);
    // A drag cut short by a tool switch or the panel closing must not leave
    // the pointer frozen where the drag began: the freeze is a platform-wide
    // state, and only this widget knows it was asked for.
    if (_locked) thawCursor();
    _throttle.cancel();
    _escape.dispose();
    _closeUndoStep();
    if (_start != null) widget.uiState.cameraDragging = false;
    super.dispose();
  }

  /// What is being moved and the pose it had when the drag began - the whole
  /// gesture is relative to that, so a drag never compounds its own rounding.
  /// A null layer with a pose is the Viewer's own custom view.
  LayerReference? _acting;
  bool _actingView = false;
  CameraPose? _start;
  Offset _delta = Offset.zero;

  /// The camera's transform as the drag found it. Every step of the drag
  /// writes from this, so a keyed camera keeps replacing the one key at the
  /// playhead rather than piling them up.
  BridgeTransform? _startTransform;

  /// Whether the drag has written to the camera, so Escape knows there is a
  /// step to take back.
  bool _wrote = false;

  /// Escape takes the drag back, putting the camera or the view where it was.
  final DragEscape _escape = DragEscape();

  /// What puts a view back as the drag found it, for Escape.
  VoidCallback? _revertView;

  /// Whether an undo group is open, so a whole drag, or a run of wheel
  /// notches, is one step however many writes it made.
  bool _grouped = false;

  /// Whether the open group is a wheel run, which closes once the wheel
  /// rests or anything else happens first.
  bool _wheelRun = false;
  Timer? _wheelRest;

  /// What the orbit in hand swings round, in the comp, or null for the
  /// camera's own pivot.
  (double, double, double)? _pivot;

  /// How far away the point a track keeps under the pointer is, as depth over
  /// the zoom: what was under the pointer at the press.
  double _trackDepth = 1;

  /// What a dolly goes at: the point under the pointer at the press, or null
  /// for the camera's own pivot. In a fixed view, where the pointer was in
  /// comp pixels from the middle of the picture, and how far away the plane
  /// the view frames is.
  (double, double, double)? _dollyToward;
  Offset _dollyAt = Offset.zero;
  double _framed = 0;

  /// The orbit's pivot while no drag runs, held against what moves it so a
  /// hover walks no boxes.
  Object? _idleKey;
  (double, double, double)? _idlePivot;

  void _openUndoStep() {
    if (_grouped) return;
    try {
      widget.state.project?.beginUndoGroup();
      _grouped = true;
    } catch (_) {
      // No project, so no history to keep tidy.
    }
  }

  void _closeUndoStep() {
    if (_wheelRun) {
      _wheelRun = false;
      _wheelRest?.cancel();
      if (widget.state.closeOpenStep == _closeUndoStep) {
        widget.state.closeOpenStep = null;
      }
      GestureBinding.instance.pointerRouter
          .removeGlobalRoute(_endRunOnPointer);
      widget.uiState.playheadFrame.removeListener(_closeUndoStep);
    }
    if (!_grouped) return;
    _grouped = false;
    try {
      widget.state.project?.endUndoGroup();
    } catch (_) {
      // The project closed mid-drag.
    }
  }

  /// A run of wheel notches stays one undo step until the wheel rests, a
  /// button goes down anywhere, the playhead moves, or a key or the history
  /// is about to move ([LumitState.settleHistory]), so nothing else can land
  /// inside it. A trackpad's coasting after the fingers lift is part of the
  /// same run. While playing each notch is its own step, as each keys a
  /// different frame.
  void _openWheelRun() {
    _wheelRest?.cancel();
    if (widget.uiState.playing.value) {
      _closeUndoStep();
      return;
    }
    if (!_wheelRun) {
      _closeUndoStep();
      _openUndoStep();
      if (!_grouped) return;
      _wheelRun = true;
      widget.state.closeOpenStep = _closeUndoStep;
      GestureBinding.instance.pointerRouter.addGlobalRoute(_endRunOnPointer);
      widget.uiState.playheadFrame.addListener(_closeUndoStep);
    }
    _wheelRest = Timer(const Duration(milliseconds: 500), _closeUndoStep);
  }

  void _endRunOnPointer(PointerEvent event) {
    if (event is PointerDownEvent) _closeUndoStep();
  }

  /// Which move the unified tool was given by the button that started the drag.
  ToolMode _move = ToolMode.cameraOrbit;

  /// A custom view is a message to the engine per movement, so it goes out at
  /// the rate every other live drag uses.
  final PreviewThrottle _throttle = PreviewThrottle();

  /// Where the pointer is being held for the length of the drag, and whether
  /// this platform could hold it there at all. Off the lock, the drag
  /// falls back to reading the movement between events, exactly as it did.
  Offset? _anchor;
  bool _locked = false;

  /// The active camera, and what the answer was worked out against.
  ///
  /// Held, because this layer rebuilds on every movement of the pointer — the
  /// drawn pointer has to follow it — and finding the camera is **not** free:
  /// the evaluated pose and the composition's rate are both reads across the
  /// bridge. Moving the mouse over the picture with a camera tool in hand was
  /// making both of them, dozens of times a second, to re-answer a question
  /// only an edit or the playhead can change.
  ({LayerReference layer, CameraPose pose, BridgeTransform transform})? _held;
  BigInt? _heldRevision;
  int? _heldFrame;

  /// Whether the active camera was passed over because an expression places
  /// it, which is a different thing to say than there being no camera.
  bool _expression = false;

  /// The comp's rate, held against the revision the walk last crossed the
  /// bridge at. Only an edit can move it, so a playhead move re-walks the held
  /// model - which camera is live can change with the frame - without asking
  /// the engine for the rate again.
  double? _fps;

  /// What a drag moves: the fronted comp's active camera, or the Viewer's own
  /// view. Null when there is nothing to move, which is a comp with no live
  /// camera or one placed by an expression.
  ({LayerReference? layer, CameraPose pose})? get _target {
    final view = widget.uiState.viewOf(widget.viewId);
    if (view != ViewerView.activeCamera) {
      final pose = widget.uiState.viewPoseOf(widget.viewId);
      if (pose == null) return null;
      // Read fresh rather than held: a view moves without the document
      // or the playhead moving, and this costs nothing to build.
      return (layer: null, pose: poseOfView(pose));
    }
    // The **held** revision, not a checked one. Reading the checking
    // getter asks the engine whether the document has moved — and this runs on
    // every rebuild, which for a tool that draws its own pointer means every
    // movement of the mouse. That was the whole of the camera tools' chatter.
    final revision = widget.uiState.model.heldRevision;
    final frame = widget.uiState.playheadFrame.value;
    if (_heldRevision != revision) _fps = null;
    if (_heldRevision != revision || _heldFrame != frame) {
      _heldRevision = revision;
      _heldFrame = frame;
      _held = _findCamera(frame);
    }
    final held = _held;
    return held == null ? null : (layer: held.layer, pose: held.pose);
  }

  /// Whether the Viewer is in one of the six fixed views, which never turn
  /// and stay orthographic.
  bool get _fixedView {
    final view = widget.uiState.viewOf(widget.viewId);
    return view != ViewerView.activeCamera && !view.orbits;
  }

  /// How far in front of [pose]'s eye the plane through the comp centre is,
  /// which is the plane a fixed view frames.
  double _framedDepth(CameraPose pose) {
    final f = pose.axes.forward;
    final e = pose.position;
    final c = (widget.compSize.width / 2, widget.compSize.height / 2, 0.0);
    return (c.$1 - e.$1) * f.$1 + (c.$2 - e.$2) * f.$2 + (c.$3 - e.$3) * f.$3;
  }

  /// [p]'s depth in front of [pose]'s eye, over its zoom.
  static double _depthOf(CameraPose pose, (double, double, double) p) {
    final f = pose.axes.forward;
    final e = pose.position;
    return ((p.$1 - e.$1) * f.$1 + (p.$2 - e.$2) * f.$2 + (p.$3 - e.$3) * f.$3) /
        pose.zoom;
  }

  /// The walk itself. Everything but the two reads noted above comes off the
  /// read model.
  ({LayerReference layer, CameraPose pose, BridgeTransform transform})?
      _findCamera(int frame) {
    _expression = false;
    for (final entry in widget.uiState.model.heldLayers) {
      final info = entry.info;
      if (info.kind != BridgeLayerKind.camera) continue;
      if (!info.switches.visible) continue;
      if (!_liveAt(info.span, frame)) continue;
      final channels = info.transform.camera;
      if (channels == null) continue;
      final tf = info.transform;
      final twoNode = info.camera?.twoNode ?? false;
      // A keyed camera moves like a still one, the drag landing a key at the
      // playhead. An expression has no value of its own for a drag to change.
      // The point of interest only counts on a camera that is aimed by it.
      final placement = [
        tf.positionX,
        tf.positionY,
        tf.positionZ,
        tf.rotationX,
        tf.rotationY,
        tf.rotation,
        if (twoNode) ...[channels.poiX, channels.poiY, channels.poiZ],
      ];
      if (placement.any((s) => s is BridgeScalar_Expression)) {
        _expression = true;
        return null;
      }
      // The *effective* pose in the comp: a two-node camera's aim, a solve
      // link and the parents are already composed into it, which the rows on
      // their own do not carry, and the point of interest is in the comp too.
      BridgeCameraAim? evaluated;
      try {
        evaluated = entry.layer.cameraAimAt(frame: BigInt.from(frame));
      } catch (_) {
        // The layer went away between the model and the read.
      }
      if (evaluated == null) return null;
      final pose = evaluated.pose;
      return (
        layer: entry.layer,
        pose: CameraPose(
          position: (pose.x, pose.y, pose.z),
          rotation: (pose.rotationX, pose.rotationY, pose.rotationZ),
          zoom: pose.zoom,
          pointOfInterest: (evaluated.poiX, evaluated.poiY, evaluated.poiZ),
          twoNode: twoNode,
        ),
        transform: tf,
      );
    }
    return null;
  }

  /// Whether [span] covers [frame]. The span is in seconds as rationals — the
  /// document's own clock — so the frame is put on that clock rather than the
  /// span being rounded to frames.
  bool _liveAt(BridgeSpan span, int frame) {
    double seconds(BridgeRational r) =>
        r.den.toInt() == 0 ? 0 : r.num.toDouble() / r.den.toDouble();
    // The comp's own rate, held against the revision: the walk runs on every
    // playhead move, and the rate can only change with an edit.
    var rate = _fps;
    if (rate == null) {
      try {
        rate = widget.comp.fps();
      } catch (_) {
        return true;
      }
      _fps = rate;
    }
    if (rate <= 0) return true;
    final t = frame / rate;
    return t >= seconds(span.inPoint) && t < seconds(span.outPoint);
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return const SizedBox.shrink();
    final unified = widget.tool == ToolMode.cameraUnified;
    final stack = Stack(
      children: [
        Positioned.fill(
          child: CustomPaint(
            painter: _CameraGizmoPainter(
              pivot: _pivotMark(),
              orbiting: widget.tool == ToolMode.cameraOrbit ||
                  (unified && _move == ToolMode.cameraOrbit),
              mark: widget.mark,
              outline: widget.outline,
              accent: widget.accent,
            ),
          ),
        ),
        ToolPointer(
          at: _pointer,
          tool: widget.tool,
          mark: widget.mark,
          outline: widget.outline,
        ),
      ],
    );
    return Positioned.fill(
      // The hardware crosshair leads; the badge beside it, drawn by
      // the app, only says which camera move.
      child: DrawnPointerRegion(
        cursor: SystemMouseCursors.precise,
        onPointer: (at) => setState(() => _pointer = at),
        // The unified tool reads raw pointers, because a pan recogniser is
        // told nothing about which button is down and the button is the whole
        // of what picks the move. The stage's own pan is switched off while a
        // camera tool is armed, so nothing underneath takes the drag instead.
        child: Listener(
          onPointerSignal: _wheel,
          // A trackpad's two-finger push dollies as the wheel does.
          onPointerPanZoomUpdate: (e) =>
              _dollyBy(e.localPosition, -e.panDelta.dy),
          child: unified
              ? Listener(
                  behavior: HitTestBehavior.opaque,
                  onPointerDown: (e) {
                    _move = cameraMoveForButtons(e.buttons);
                    _begin(e.localPosition);
                  },
                  onPointerMove: (e) => _drag(e.localPosition, e.delta),
                  onPointerUp: (_) => _end(),
                  onPointerCancel: (_) => _end(),
                  child: stack,
                )
              : GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onTapUp: (_) {
                    if (_target == null) _sayNoCamera();
                  },
                  onPanStart: (d) => _begin(d.localPosition),
                  onPanUpdate: (d) => _drag(d.localPosition, d.delta),
                  onPanEnd: (_) => _end(),
                  onPanCancel: _end,
                  child: stack,
                ),
        ),
      ),
    );
  }

  /// The wheel dollies while a camera tool is in hand, and the Viewer's own
  /// wheel zoom stands aside for it. With nothing to move it is left to the
  /// zoom.
  void _wheel(PointerSignalEvent event) {
    if (event is! PointerScrollEvent || _start != null || _target == null) {
      return;
    }
    final dy = event.scrollDelta.dy.clamp(-wheelNotchPixels, wheelNotchPixels);
    GestureBinding.instance.pointerSignalResolver.register(
        event, (_) => _dollyBy(event.localPosition, -dy * wheelDollyPixels));
  }

  /// One step of the wheel or the trackpad: [travel] pixels of dolly toward
  /// what is under [at], away from you going in, as in After Effects.
  void _dollyBy(Offset at, double travel) {
    final target = _target;
    if (_start != null || target == null || travel == 0) return;
    _aimAt(at, target.pose);
    final pose = _dollied(target.pose, travel);
    final layer = target.layer;
    if (layer == null) {
      widget.uiState.setViewerViewPose(viewOfPose(pose), viewId: widget.viewId);
      return;
    }
    final tf = _held?.transform;
    if (tf == null) return;
    _openWheelRun();
    _writeCamera(layer, tf, pose, _movedProps(pose, ToolMode.cameraDolly));
  }

  void _sayNoCamera() => widget.state.postNotice(
      _expression ? l10n.cameraExpressionRefused : l10n.noCameraToMove);

  void _begin(Offset at) {
    final target = _target;
    if (target == null) {
      _sayNoCamera();
      return;
    }
    // The pointer is pinned where it was pressed for as long as the drag lasts.
    // Moving a camera is a gesture with no *place* — nothing on the picture is
    // being aimed at — so a pointer that wanders out of the Viewer, and finally
    // into the corner of the screen where it stops moving at all, is a drag
    // that ends before the user does. It reappears where it started when the
    // button comes up, which is what every 3D application does.
    _anchor = at;
    _locked = freezeCursor();
    // Everything under the pointer is found through the pose the boxes were
    // drawn with, before an orbit carries a fixed view anywhere.
    final orbit = _activeMove == ToolMode.cameraOrbit;
    var start = target.pose;
    var pivot = orbit ? _pivotFor(at, start) : null;
    _aimAt(at, start);
    _revertView = target.layer == null ? _viewReverter() : null;
    if (orbit && _fixedView) (start, pivot) = _toCustom(start, pivot);
    // A wheel run still open is its own step, not the start of this one.
    _closeUndoStep();
    if (target.layer != null) _openUndoStep();
    _wrote = false;
    _escape.begin(_abandon);
    widget.uiState.cameraDragging = true;
    setState(() {
      _acting = target.layer;
      _actingView = target.layer == null;
      _start = start;
      _startTransform = _held?.transform;
      _pivot = pivot;
      _delta = Offset.zero;
    });
  }

  /// Where a track and a dolly begun at [at] hold on: the depth of what is
  /// under the pointer, or of the camera's own pivot with nothing there (the
  /// framed plane in a fixed view).
  void _aimAt(Offset at, CameraPose pose) {
    final hit = _hitAt(at, pose);
    final fixed = _fixedView;
    final framed = fixed ? _framedDepth(pose) : 0.0;
    final fallback =
        fixed ? framed / pose.zoom : _depthOf(pose, pose.pivot);
    _trackDepth = hit?.depth ?? fallback;
    _framed = framed;
    _dollyAt = _toPicture(at) -
        Offset(widget.compSize.width / 2, widget.compSize.height / 2);
    // A two-node camera with nothing under the pointer dollies at its point
    // of interest and keeps it.
    _dollyToward = hit?.point ??
        (pose.twoNode
            ? null
            : pointUnder(pose, _toPicture(at), widget.compSize, fallback));
  }

  /// [pose] after a dolly of [travel] toward what [_aimAt] found.
  CameraPose _dollied(CameraPose pose, double travel) => _fixedView
      ? zoomView(pose, travel, at: _dollyAt, depth: _framed)
      : dollyCamera(pose, travel, toward: _dollyToward);

  /// What puts the Viewer's view back as it is now, should Escape take the
  /// drag back, including an orbit that carried a fixed view into a custom one.
  VoidCallback _viewReverter() {
    final ui = widget.uiState;
    final id = widget.viewId;
    final look = ui.viewOf(id);
    final pose = ui.viewPoseOf(id);
    final custom = ui.lastCustomViewOf(id);
    final customPose = ui.heldViewPose(custom, viewId: id);
    return () {
      if (ui.viewOf(id) == custom && custom != look && customPose != null) {
        ui.setViewerViewPose(customPose, viewId: id);
      }
      ui.setViewerView(look, viewId: id);
      if (pose != null) ui.setViewerViewPose(pose, viewId: id);
    };
  }

  /// A fixed view never turns, so an orbit begun in one carries its framing
  /// into the Viewer's last custom view and turns that: the same picture of
  /// the plane the fixed view frames, seen from the custom view's zoom at the
  /// distance that keeps its size. The orbit swings round [pivot], or the
  /// middle of that plane.
  (CameraPose, (double, double, double)) _toCustom(
      CameraPose pose, (double, double, double)? pivot) {
    final ui = widget.uiState;
    final id = widget.viewId;
    final f = pose.axes.forward;
    final framed = _framedDepth(pose);
    final e = pose.position;
    final middle = (e.$1 + f.$1 * framed, e.$2 + f.$2 * framed,
        e.$3 + f.$3 * framed);
    ui.setViewerView(ui.lastCustomViewOf(id), viewId: id);
    final custom = ui.viewPoseOf(id);
    if (custom == null) return (pose, pivot ?? middle);
    final reach = custom.zoom * framed / pose.zoom;
    final moved = CameraPose(
      position: (
        middle.$1 - f.$1 * reach,
        middle.$2 - f.$2 * reach,
        middle.$3 - f.$3 * reach,
      ),
      rotation: pose.rotation,
      zoom: custom.zoom,
    );
    ui.setViewerViewPose(viewOfPose(moved), viewId: id);
    return (moved, pivot ?? middle);
  }

  void _drag(Offset at, Offset delta) {
    if (_start == null || !_escape.running) return;
    final anchor = _anchor;
    if (_locked && anchor != null) {
      // Measured from where the pointer is *held*, not from the last event:
      // putting the pointer back is itself a movement, and the delta the
      // framework reports for that one exactly undoes the real one. Against the
      // anchor, the put-back event reads as no movement at all, which is the
      // truth of it.
      final moved = at - anchor;
      if (moved == Offset.zero) return;
      setState(() => _delta += moved);
      restoreFrozenCursor();
    } else {
      setState(() => _delta += delta);
    }
    _write(preview: true);
  }

  void _end() {
    final commit = _escape.end();
    if (commit && _start != null && _delta != Offset.zero) {
      _write(preview: false);
    }
    _closeUndoStep();
    widget.uiState.cameraDragging = false;
    if (_locked) thawCursor();
    _locked = false;
    _anchor = null;
    _revertView = null;
    setState(() {
      _acting = null;
      _actingView = false;
      _start = null;
      _startTransform = null;
      _pivot = null;
      _delta = Offset.zero;
    });
  }

  /// Escape mid-drag: the camera or the view goes back where it was, and the
  /// rest of the drag moves nothing.
  void _abandon() {
    _throttle.cancel();
    if (_actingView) {
      _revertView?.call();
      return;
    }
    final wrote = _wrote;
    _closeUndoStep();
    if (!wrote) return;
    try {
      widget.state.project?.undo();
    } catch (_) {
      // The project closed mid-drag.
    }
    widget.onChanged();
  }

  /// Which move the drag is: the armed tool's, or the button's under the
  /// unified tool.
  ToolMode get _activeMove =>
      widget.tool == ToolMode.cameraUnified ? _move : widget.tool;

  /// Screen pixels per comp pixel.
  double get _scale => widget.compSize.width == 0
      ? 1.0
      : widget.fitted.width / widget.compSize.width;

  /// A point on screen as comp pixels of the picture.
  Offset _toPicture(Offset at) => (at - widget.fitted.topLeft) / _scale;

  /// The nearest 3D layer under [at] seen through [pose]: where the pointer
  /// meets it, and its depth over the zoom. A light is its own point.
  ({(double, double, double) point, double depth})? _hitAt(
      Offset at, CameraPose pose) {
    final hit = layerAtPoint(widget.boxes(), at);
    if (hit == null || !hit.map.projected) return null;
    final m = hit.map;
    final on = hit.marker ? m.toScreen(m.ax, m.ay) : at;
    final w = hit.marker ? m.depthOf(m.ax, m.ay) : m.depthAt(at);
    if (w == null || w <= viewerNear) return null;
    return (
      point: pointUnder(pose, _toPicture(on), widget.compSize, w),
      depth: w,
    );
  }

  /// What an orbit begun at [at] swings round under the tool options' choice,
  /// in the comp, or null for the camera's own pivot, which is also where a
  /// choice that finds nothing falls back to.
  (double, double, double)? _pivotFor(Offset? at, CameraPose pose) {
    final choice = widget.uiState.tools.cameraPivot;
    if (choice == CameraPivot.pointOfInterest) return null;
    if (choice == CameraPivot.cursor) {
      return at == null ? null : _hitAt(at, pose)?.point;
    }
    final boxes = widget.boxes();
    final selected = widget.uiState.selectedLayerIds;
    final picked = choice == CameraPivot.selection
        ? _middleOf(boxes.where((b) => selected.contains(b.id)), pose)
        : null;
    return picked ?? _middleOf(boxes, pose);
  }

  /// The middle of the box round every 3D layer in [boxes], in the comp, or
  /// null with none.
  (double, double, double)? _middleOf(
      Iterable<LayerBox> boxes, CameraPose pose) {
    final points = _cornersOf(boxes, pose);
    if (points.isEmpty) return null;
    final lo = [double.infinity, double.infinity, double.infinity];
    final hi = [-double.infinity, -double.infinity, -double.infinity];
    for (final p in points) {
      final xyz = [p.$1, p.$2, p.$3];
      for (var i = 0; i < 3; i++) {
        lo[i] = math.min(lo[i], xyz[i]);
        hi[i] = math.max(hi[i], xyz[i]);
      }
    }
    return ((lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2);
  }

  /// The corners of every 3D layer in [boxes], and a light's own point, in
  /// the comp.
  List<(double, double, double)> _cornersOf(
      Iterable<LayerBox> boxes, CameraPose pose) {
    final out = <(double, double, double)>[];
    void take(LayerBox box, double x, double y) {
      final w = box.map.depthOf(x, y);
      if (w.abs() < 1e-9) return;
      out.add(pointUnder(
          pose, _toPicture(box.map.toScreen(x, y)), widget.compSize, w));
    }

    for (final box in boxes) {
      if (!box.map.projected) continue;
      if (box.marker) {
        take(box, box.map.ax, box.map.ay);
        continue;
      }
      final s = box.bounds;
      take(box, 0, 0);
      take(box, s.width, 0);
      take(box, 0, s.height);
      take(box, s.width, s.height);
    }
    return out;
  }

  /// Answer a Look at or a Reset from the View menu, in the active view only.
  /// The active camera moves as one undo step, keyed where it is keyed; any
  /// other view just moves.
  void _onCommand() {
    final request = widget.uiState.viewerCameraRequest.value;
    if (request == null || !mounted || _start != null) return;
    if (widget.uiState.views.active?.engineId != widget.viewId) return;
    final target = _target;
    if (target == null) {
      _sayNoCamera();
      return;
    }
    final layer = target.layer;
    final CameraPose? to;
    switch (request.$2) {
      case ViewerCameraCommand.lookAtSelected ||
            ViewerCameraCommand.lookAtAll:
        final selected = widget.uiState.selectedLayerIds;
        final boxes = widget.boxes().where((b) =>
            request.$2 == ViewerCameraCommand.lookAtAll ||
            selected.contains(b.id));
        final points = _cornersOf(boxes, target.pose);
        // A fixed view magnifies to fit, so it stays flat.
        to = _fixedView
            ? frameFlat(target.pose, points, widget.compSize,
                _framedDepth(target.pose))
            : framePoints(target.pose, points, widget.compSize);
        if (to == null) {
          widget.state.postNotice(l10n.noLayersToLookAt);
          return;
        }
      case ViewerCameraCommand.reset:
        if (layer == null) {
          widget.uiState.resetViewerView(viewId: widget.viewId);
          return;
        }
        // Where a new camera stands: square on to the middle of the comp,
        // seeing it whole at its own zoom.
        final (w, h) = (widget.compSize.width, widget.compSize.height);
        to = target.pose.copyWith(
          position: (w / 2, h / 2, -target.pose.zoom),
          rotation: (0, 0, 0),
          pointOfInterest: (w / 2, h / 2, 0),
        );
    }
    if (layer == null) {
      widget.uiState.setViewerViewPose(viewOfPose(to), viewId: widget.viewId);
      return;
    }
    final tf = _held?.transform;
    if (tf == null) return;
    final reset = request.$2 == ViewerCameraCommand.reset;
    _writeCamera(layer, tf, to, {
      BridgeTransformProp.positionX,
      BridgeTransformProp.positionY,
      BridgeTransformProp.positionZ,
      if (reset) ...{
        BridgeTransformProp.rotationX,
        BridgeTransformProp.rotationY,
        BridgeTransformProp.rotation,
      },
      if (to.twoNode) ...{
        BridgeTransformProp.poiX,
        BridgeTransformProp.poiY,
        BridgeTransformProp.poiZ,
      },
    });
  }

  /// Where the pivot mark goes on screen, or null for none: the drag's pivot
  /// while one runs, and otherwise the one an orbit would take now. Under
  /// Cursor that is not known until the press.
  Offset? _pivotMark() {
    final start = _start;
    final CameraPose pose;
    final (double, double, double) at;
    if (start != null) {
      pose = _moved() ?? start;
      at = _activeMove == ToolMode.cameraOrbit
          ? (_pivot ?? start.pivot)
          : pose.pivot;
    } else {
      final target = _target;
      if (target == null) return null;
      pose = target.pose;
      final orbits = widget.tool == ToolMode.cameraOrbit ||
          widget.tool == ToolMode.cameraUnified;
      final choice = widget.uiState.tools.cameraPivot;
      if (orbits && choice == CameraPivot.cursor) return null;
      at = (orbits ? _idle(pose) : null) ?? pose.pivot;
    }
    final p = pictureOf(pose, at, widget.compSize);
    return p == null ? null : widget.fitted.topLeft + p * _scale;
  }

  /// The scene's or the selection's middle for the mark, asked again only
  /// when the document, the frame, the view or the selection has moved.
  (double, double, double)? _idle(CameraPose pose) {
    final ui = widget.uiState;
    final key = (
      ui.tools.cameraPivot,
      ui.model.heldRevision,
      ui.playheadFrame.value,
      ui.viewPoseOf(widget.viewId),
      Object.hashAllUnordered(ui.selectedLayerIds),
      widget.fitted,
    );
    if (key != _idleKey) {
      _idleKey = key;
      _idlePivot = _pivotFor(null, pose);
    }
    return _idlePivot;
  }

  /// The pose the drag so far implies.
  CameraPose? _moved() {
    final start = _start;
    if (start == null) return null;
    final shift = HardwareKeyboard.instance.isShiftPressed;
    return switch (_activeMove) {
      ToolMode.cameraOrbit => orbitCamera(start, _delta.dx, _delta.dy,
          lockAxis: shift, around: _pivot),
      ToolMode.cameraPan => trackCamera(start, _delta.dx, _delta.dy,
          scale: _scale, lockAxis: shift, depth: _trackDepth),
      // Whichever axis carries the movement, so the gesture works either way
      // round. Down or right goes in, which is After Effects' sense.
      ToolMode.cameraDolly => _dollied(start,
          _delta.dx.abs() >= _delta.dy.abs() ? _delta.dx : _delta.dy),
      _ => start,
    };
  }

  /// Write the pose. A drag on the camera is one undo step, the group opened
  /// when it began.
  ///
  /// [preview] is a live update while the drag is in flight; the values are the
  /// same either way, because a camera has no preview path of its own (a
  /// preview patches *one layer's* transform, and moving the camera changes
  /// what every layer looks like).
  void _write({required bool preview}) {
    final pose = _moved();
    if (pose == null) return;
    if (_actingView) {
      // A view is not the document, so this is a message and a re-render and
      // nothing else. Throttled while the drag runs, sent outright at the end.
      final view = viewOfPose(pose);
      if (preview) {
        _throttle.request(() =>
            widget.uiState.setViewerViewPose(view, viewId: widget.viewId));
      } else {
        _throttle.cancel();
        widget.uiState.setViewerViewPose(view, viewId: widget.viewId);
      }
      return;
    }
    final layer = _acting;
    final tf = _startTransform;
    if (layer == null || tf == null) return;
    if (_writeCamera(layer, tf, pose, _movedProps(pose, _activeMove))) {
      _wrote = true;
    }
  }

  /// What [move] changes on the camera. Only what moved is written, so a
  /// keyed camera gains no keys on rows the drag left alone: a two-node
  /// camera's rotation rows stay put, as its aim follows the eye, and its
  /// point of interest travels with a track, and with an orbit or a dolly
  /// that is not round or at it.
  Set<BridgeTransformProp> _movedProps(CameraPose pose, ToolMode move) {
    final poiMoves = pose.twoNode &&
        switch (move) {
          ToolMode.cameraPan => true,
          ToolMode.cameraOrbit => _pivot != null,
          ToolMode.cameraDolly => _dollyToward != null,
          _ => false,
        };
    return {
      BridgeTransformProp.positionX,
      BridgeTransformProp.positionY,
      BridgeTransformProp.positionZ,
      // Under a turned parent an orbit changes all three of the camera's own
      // rows, so Z is written with the other two.
      if (!pose.twoNode && move == ToolMode.cameraOrbit) ...{
        BridgeTransformProp.rotationX,
        BridgeTransformProp.rotationY,
        BridgeTransformProp.rotation,
      },
      if (poiMoves) ...{
        BridgeTransformProp.poiX,
        BridgeTransformProp.poiY,
        BridgeTransformProp.poiZ,
      },
    };
  }

  bool _writeCamera(LayerReference layer, BridgeTransform tf, CameraPose pose,
      Set<BridgeTransformProp> props) {
    final wrote = writeCameraPose(layer, tf, pose, props,
        comp: widget.comp, frame: widget.uiState.playheadFrame.value);
    if (wrote) widget.onChanged();
    return wrote;
  }
}

/// The camera gizmo: the point the camera is turning around, and - while
/// orbiting - the circle it would swing round.
class _CameraGizmoPainter extends CustomPainter {
  final Offset? pivot;
  final bool orbiting;
  final Color mark;
  final Color outline;
  final Color accent;

  const _CameraGizmoPainter({
    required this.pivot,
    required this.orbiting,
    required this.mark,
    required this.outline,
    required this.accent,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final at = pivot;
    if (at == null) return;
    const reach = 10.0;
    paintTwoPassStroke(outline, mark, (paint) {
      canvas.drawLine(
          at - const Offset(reach, 0), at + const Offset(reach, 0), paint);
      canvas.drawLine(
          at - const Offset(0, reach), at + const Offset(0, reach), paint);
    });
    if (!orbiting) return;
    // The orbit's own circle, faint: it says which point the swing goes round
    // without drawing attention away from the picture.
    canvas.drawCircle(
      at,
      reach * 3,
      Paint()
        ..color = accent.withValues(alpha: 0.5)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(_CameraGizmoPainter old) =>
      old.pivot != pivot ||
      old.orbiting != orbiting ||
      old.mark != mark ||
      old.outline != outline ||
      old.accent != accent;
}
