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
// than any layer. The six fixed views are not movable: dragging in one says so
// and changes nothing.
//
// The maths below is pure and in composition pixels; the widget under it only
// turns drags into these calls and commits transform properties.

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

import 'viewer_tool_cursor.dart';

/// How many degrees a pixel of drag turns the camera.
///
/// A full sweep of a 1000px-wide Viewer comes to about half a turn, which is
/// what After Effects' orbit feels like: enough to get round a scene in one
/// gesture, gentle enough to aim with.
const double orbitDegreesPerPixel = 0.25;

/// How much of the camera's focal distance a pixel of dolly drag covers.
///
/// Proportional rather than absolute, so a dolly feels the same in a comp built
/// at 500 px and one built at 5000: dragging the width of the picture roughly
/// halves or doubles the distance to what you are looking at.
const double dollyFraction = 0.0015;

/// A camera's pose in the shape the tools work in: where the eye is, which way
/// it faces, how far ahead the plane it renders 1:1 sits, and what it is aimed
/// at when it is aimed at something.
///
/// [position] is the **eye**, as the document stores it. [rotation] is the
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

/// The pose after an **orbit** drag of [dx], [dy] screen pixels.
///
/// Horizontal movement swings the camera around the pivot (yaw); vertical
/// movement lifts it over the top or drops it underneath (pitch). The pivot
/// never moves, which is precisely what makes this an orbit rather than a pan.
CameraPose orbitCamera(
  CameraPose pose,
  double dx,
  double dy, {
  bool lockAxis = false,
}) {
  final (ax, ay) = _onOneAxis(dx, dy, lockAxis);
  final yaw = pose.rotation.$2 + ax * orbitDegreesPerPixel;
  // Dragging **up** lifts the camera over the top, which means tilting it to
  // look *down* — a negative x rotation in the compositor's frame, where +y is
  // down the screen. Getting this the other way round is the classic inverted
  // orbit.
  //
  // Clamped rather than wrapped: past a quarter turn the camera is looking
  // straight down and the next pixel of drag flips the picture over, which no
  // orbit control anywhere does.
  final pitch =
      (pose.rotation.$1 + ay * orbitDegreesPerPixel).clamp(-89.9, 89.9);
  final turned = (pitch, yaw, pose.rotation.$3);
  final pivot = pose.pivot;
  final f = pose.copyWith(rotation: turned).axes.forward;
  // A two-node camera keeps whatever distance it had to the point it is aimed
  // at; a one-node one keeps its pivot on the plane it renders 1:1.
  final reach = pose.twoNode
      ? math.sqrt([
          pose.position.$1 - pivot.$1,
          pose.position.$2 - pivot.$2,
          pose.position.$3 - pivot.$3,
        ].fold(0.0, (sum, v) => sum + v * v))
      : pose.zoom;
  return pose.copyWith(
    position: (
      pivot.$1 - f.$1 * reach,
      pivot.$2 - f.$2 * reach,
      pivot.$3 - f.$3 * reach,
    ),
    // The rows of a two-node camera are left alone: its aim is worked out from
    // where the eye now is, so turning them as well would double the swing.
    rotation: pose.twoNode ? pose.rotation : turned,
  );
}

/// The pose after a **track** drag: the eye slides along its own right and up
/// axes, taking a two-node camera's point of interest with it.
///
/// The picture follows the pointer rather than running away from it — dragging
/// right moves the *view* right, which means moving the camera left, the same
/// sense the Hand tool has.
///
/// [scale] converts screen pixels to composition pixels (the Viewer's
/// magnification), so a drag moves the picture the distance the pointer moved.
CameraPose trackCamera(
  CameraPose pose,
  double dx,
  double dy, {
  required double scale,
  bool lockAxis = false,
}) {
  final (ax, ay) = _onOneAxis(dx, dy, lockAxis);
  final k = scale <= 0 ? 1.0 : 1 / scale;
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

/// The pose after a **dolly** drag: the eye moves along its forward axis, in or
/// out of the scene, leaving what it is aimed at where it is.
///
/// Dragging **down or right** goes in, which is After Effects' sense. The
/// distance moved is proportional to how far away the camera already is, so a
/// dolly across a wide shot covers ground and one in a close-up creeps.
CameraPose dollyCamera(CameraPose pose, double dx, double dy) {
  // Whichever axis carries the movement, so the gesture works either way round.
  final travel = dx.abs() >= dy.abs() ? dx : dy;
  final step = travel * dollyFraction * pose.zoom;
  final forward = pose.axes.forward;
  return pose.copyWith(
    position: (
      pose.position.$1 + forward.$1 * step,
      pose.position.$2 + forward.$2 * step,
      pose.position.$3 + forward.$3 * step,
    ),
  );
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

/// The camera tools over the picture.
class ViewerCameraLayer extends StatefulWidget {
  /// Whether a camera tool is armed. Inert otherwise.
  final bool active;

  final ToolMode tool;
  final CompositionReference comp;
  final LumitState state;
  final LumitUiState uiState;

  /// Where the picture sits on screen, for the pivot mark and the drag's scale.
  final Rect fitted;

  /// The composition's own size, for the same two.
  final Size compSize;

  final Color mark;
  final Color outline;
  final Color accent;

  final VoidCallback onChanged;

  const ViewerCameraLayer({
    super.key,
    required this.active,
    required this.tool,
    required this.comp,
    required this.state,
    required this.uiState,
    required this.fitted,
    required this.compSize,
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
  void dispose() {
    // A drag cut short by a tool switch or the panel closing must not leave
    // the pointer frozen where the drag began: the freeze is a platform-wide
    // state, and only this widget knows it was asked for.
    if (_locked) thawCursor();
    _throttle.cancel();
    super.dispose();
  }

  /// What is being moved and the pose it had when the drag began - the whole
  /// gesture is relative to that, so a drag never compounds its own rounding.
  /// A null layer with a pose is the Viewer's own custom view.
  LayerReference? _acting;
  bool _actingView = false;
  CameraPose? _start;
  Offset _delta = Offset.zero;

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
  ({LayerReference layer, CameraPose pose})? _held;
  BigInt? _heldRevision;
  int? _heldFrame;

  /// The comp's rate, held against the revision the walk last crossed the
  /// bridge at. Only an edit can move it, so a playhead move re-walks the held
  /// model - which camera is live can change with the frame - without asking
  /// the engine for the rate again.
  double? _fps;

  /// What a drag moves: the fronted comp's active camera, or the Viewer's own
  /// custom view. Null when there is nothing to move, which is a comp with no
  /// live camera, one whose camera is keyframed, or a fixed view.
  ({LayerReference? layer, CameraPose pose})? get _target {
    final view = widget.uiState.viewerView;
    if (view != ViewerView.activeCamera) {
      final pose = widget.uiState.viewerViewPose;
      if (!view.movable || pose == null) return null;
      // Read fresh rather than held: a custom view moves without the document
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

  /// The walk itself. Everything but the two reads noted above comes off the
  /// read model.
  ({LayerReference layer, CameraPose pose})? _findCamera(int frame) {
    for (final entry in widget.uiState.model.heldLayers) {
      final info = entry.info;
      if (info.kind != BridgeLayerKind.camera) continue;
      if (!info.switches.visible) continue;
      if (!_liveAt(info.span, frame)) continue;
      final channels = info.transform.camera;
      if (channels == null) continue;
      double still(BridgeScalar s) =>
          s is BridgeScalar_Static ? s.field0 : double.nan;
      final tf = info.transform;
      final twoNode = info.camera?.twoNode ?? false;
      // A camera whose placement is keyframed has no single value for a drag to
      // add to - the same rule the layer gizmo follows. The point of interest
      // only counts on a camera that is aimed by it.
      final placement = [
        still(tf.positionX),
        still(tf.positionY),
        still(tf.positionZ),
        still(tf.rotationX),
        still(tf.rotationY),
        still(tf.rotation),
        still(channels.zoom),
        if (twoNode) ...[
          still(channels.poiX),
          still(channels.poiY),
          still(channels.poiZ),
        ],
      ];
      if (placement.any((v) => v.isNaN)) return null;
      // The *effective* pose: a two-node camera's aim and a solve link are
      // already composed into it, which the rows on their own do not carry.
      BridgeCameraPose? evaluated;
      try {
        evaluated = entry.layer.cameraPoseAt(frame: BigInt.from(frame));
      } catch (_) {
        // The layer went away between the model and the read.
      }
      if (evaluated == null) return null;
      return (
        layer: entry.layer,
        pose: CameraPose(
          position: (evaluated.x, evaluated.y, evaluated.z),
          rotation: (
            evaluated.rotationX,
            evaluated.rotationY,
            evaluated.rotationZ
          ),
          zoom: evaluated.zoom,
          pointOfInterest: (
            still(channels.poiX),
            still(channels.poiY),
            still(channels.poiZ),
          ),
          twoNode: twoNode,
        ),
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
              // The pivot is what the camera is looking at, which is by
              // construction the middle of the frame - a one-node camera's
              // 1:1 plane and a two-node camera's point of interest both sit
              // on its forward axis.
              pivot: _target == null ? null : widget.fitted.center,
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
    );
  }

  void _sayNoCamera() => widget.state.postNotice(l10n.noCameraToMove);

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
    setState(() {
      _acting = target.layer;
      _actingView = target.layer == null;
      _start = target.pose;
      _delta = Offset.zero;
    });
  }

  void _drag(Offset at, Offset delta) {
    if (_start == null) return;
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
    if (_start != null && _delta != Offset.zero) _write(preview: false);
    if (_locked) thawCursor();
    _locked = false;
    _anchor = null;
    setState(() {
      _acting = null;
      _actingView = false;
      _start = null;
      _delta = Offset.zero;
    });
  }

  /// The pose the drag so far implies.
  CameraPose? _moved() {
    final start = _start;
    if (start == null) return null;
    final shift = HardwareKeyboard.instance.isShiftPressed;
    final scale = widget.compSize.width == 0
        ? 1.0
        : widget.fitted.width / widget.compSize.width;
    final move =
        widget.tool == ToolMode.cameraUnified ? _move : widget.tool;
    return switch (move) {
      ToolMode.cameraOrbit =>
        orbitCamera(start, _delta.dx, _delta.dy, lockAxis: shift),
      ToolMode.cameraPan =>
        trackCamera(start, _delta.dx, _delta.dy, scale: scale, lockAxis: shift),
      ToolMode.cameraDolly => dollyCamera(start, _delta.dx, _delta.dy),
      _ => start,
    };
  }

  /// Write the pose. Every camera drag is one undo step per property, the same
  /// as the layer gizmo's — the properties are separate in the model and there
  /// is no batched op for a camera move (docs/TODO.md).
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
        _throttle.request(() => widget.uiState.setViewerViewPose(view));
      } else {
        _throttle.cancel();
        widget.uiState.setViewerViewPose(view);
      }
      return;
    }
    final layer = _acting;
    if (layer == null) return;
    // A two-node camera's rows are left alone by an orbit, and its point of
    // interest travels with a track: what is written is what moved.
    final props = <BridgeTransformProp>[
      BridgeTransformProp.positionX,
      BridgeTransformProp.positionY,
      BridgeTransformProp.positionZ,
      if (!pose.twoNode) ...[
        BridgeTransformProp.rotationX,
        BridgeTransformProp.rotationY,
      ],
      if (pose.twoNode) ...[
        BridgeTransformProp.poiX,
        BridgeTransformProp.poiY,
        BridgeTransformProp.poiZ,
      ],
    ];
    final values = <BridgeScalar>[
      BridgeScalar.static_(pose.position.$1),
      BridgeScalar.static_(pose.position.$2),
      BridgeScalar.static_(pose.position.$3),
      if (!pose.twoNode) ...[
        BridgeScalar.static_(pose.rotation.$1),
        BridgeScalar.static_(pose.rotation.$2),
      ],
      if (pose.twoNode) ...[
        BridgeScalar.static_(pose.pointOfInterest.$1),
        BridgeScalar.static_(pose.pointOfInterest.$2),
        BridgeScalar.static_(pose.pointOfInterest.$3),
      ],
    ];
    try {
      layer.setTransforms(props: props, values: values);
      widget.onChanged();
    } catch (_) {
      // The camera was deleted mid-drag.
    }
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
