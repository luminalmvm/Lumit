// The camera tools: orbit, track and dolly the composition's camera
// (K-229, docs/07 §2.3.5).
//
// **In plain terms.** A 3D composition is looked at through a **camera layer**,
// and these three tools are how you move it by dragging on the picture instead
// of typing numbers into the Timeline. Orbit swings the camera around whatever
// it is pointed at; Track slides it sideways and up and down; Dolly moves it in
// and out. They are After Effects' three camera tools, and they act on the
// **active camera** — the topmost visible camera layer whose span covers the
// playhead — whatever is selected, because the camera is the thing you are
// looking through rather than a thing you are editing.
//
// **What the camera's numbers mean here.** Lumit's camera is a position, three
// rotations and a *zoom* (the focal distance, in composition pixels). The plane
// at the camera's own position renders 1:1 and centred — so **the camera's
// position is the point it is looking at**, and the eye sits `zoom` behind it
// along the camera's own forward axis. That is the whole geometry, and it makes
// the three tools very simple:
//
// * **Orbit** changes the rotations and leaves the position alone. The eye,
//   being derived from both, swings round the point being looked at.
// * **Track** slides the position along the camera's own right and up axes, so
//   the eye travels with it and the picture slides.
// * **Dolly** slides the position along the camera's forward axis, moving the
//   eye and what it is looking at together, in and out of the scene.
//
// Lumit's camera has **no separate point of interest** (After Effects' two-node
// camera): the pivot is the point the camera is already looking at. Adding one
// is an engine change and is in TODO.md.
//
// The maths below is pure and in composition pixels; the widget under it only
// turns drags into these calls and commits transform properties.

import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/system.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

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

/// A camera's pose in the shape the tools work in: where it looks from, what it
/// looks at, and how far apart those are.
///
/// [position] is the point being looked at — the plane that renders 1:1 —
/// exactly as the document stores it. [distance] is the focal distance (the
/// document's zoom), which is how far the eye sits behind it.
@immutable
class CameraPose {
  final (double, double, double) position;

  /// Rotation in degrees about x, y and z, in the order the compositor applies
  /// them (`Ry · Rx · Rz`).
  final (double, double, double) rotation;
  final double distance;

  const CameraPose({
    required this.position,
    required this.rotation,
    required this.distance,
  });

  CameraPose copyWith({
    (double, double, double)? position,
    (double, double, double)? rotation,
    double? distance,
  }) =>
      CameraPose(
        position: position ?? this.position,
        rotation: rotation ?? this.rotation,
        distance: distance ?? this.distance,
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
}

/// The pose after an **orbit** drag of [dx], [dy] screen pixels.
///
/// Horizontal movement swings the camera around the point it is looking at
/// (yaw); vertical movement lifts it over the top or drops it underneath
/// (pitch). The position never changes, which is precisely what makes this an
/// orbit rather than a pan: the eye is derived from the rotations, so it travels
/// round a fixed centre.
///
/// [lockAxis] is `Shift`: the larger of the two movements wins and the other is
/// dropped, so a level orbit stays level.
CameraPose orbitCamera(
  CameraPose pose,
  double dx,
  double dy, {
  bool lockAxis = false,
}) {
  var ax = dx;
  var ay = dy;
  if (lockAxis) {
    if (ax.abs() >= ay.abs()) {
      ay = 0;
    } else {
      ax = 0;
    }
  }
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
  return pose.copyWith(rotation: (pitch, yaw, pose.rotation.$3));
}

/// The pose after a **track** drag: the camera slides along its own right and
/// up axes, taking what it is looking at with it.
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
  var ax = dx;
  var ay = dy;
  if (lockAxis) {
    if (ax.abs() >= ay.abs()) {
      ay = 0;
    } else {
      ax = 0;
    }
  }
  final k = scale <= 0 ? 1.0 : 1 / scale;
  final right = pose.axes.right;
  final up = pose.axes.up;
  final mx = -ax * k;
  final my = -ay * k;
  return pose.copyWith(
    position: (
      pose.position.$1 + right.$1 * mx + up.$1 * my,
      pose.position.$2 + right.$2 * mx + up.$2 * my,
      pose.position.$3 + right.$3 * mx + up.$3 * my,
    ),
  );
}

/// The pose after a **dolly** drag: the camera moves along its forward axis,
/// eye and subject together, in or out of the scene.
///
/// Dragging **down or right** goes in, which is After Effects' sense. The
/// distance moved is proportional to how far away the camera already is, so a
/// dolly across a wide shot covers ground and one in a close-up creeps.
CameraPose dollyCamera(CameraPose pose, double dx, double dy) {
  // Whichever axis carries the movement, so the gesture works either way round.
  final travel = dx.abs() >= dy.abs() ? dx : dy;
  final step = travel * dollyFraction * pose.distance;
  final forward = pose.axes.forward;
  return pose.copyWith(
    position: (
      pose.position.$1 + forward.$1 * step,
      pose.position.$2 + forward.$2 * step,
      pose.position.$3 + forward.$3 * step,
    ),
  );
}

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
    super.dispose();
  }

  /// The camera being moved and the pose it had when the drag began — the whole
  /// gesture is relative to that, so a drag never compounds its own rounding.
  LayerReference? _acting;
  CameraPose? _start;
  Offset _delta = Offset.zero;

  /// Where the pointer is being held for the length of the drag, and whether
  /// this platform could hold it there at all (K-230). Off the lock, the drag
  /// falls back to reading the movement between events, exactly as it did.
  Offset? _anchor;
  bool _locked = false;

  /// The active camera, and what the answer was worked out against.
  ///
  /// Held, because this layer rebuilds on every movement of the pointer — the
  /// drawn pointer has to follow it — and finding the camera is **not** free:
  /// the layer's focal distance and the composition's rate are both reads
  /// across the bridge. Moving the mouse over the picture with a camera tool in
  /// hand was making both of them, dozens of times a second, to re-answer a
  /// question only an edit or the playhead can change (K-230).
  ({LayerReference layer, CameraPose pose})? _held;
  BigInt? _heldRevision;
  int? _heldFrame;

  /// The comp's rate and each camera's focal distance, held against the
  /// revision the walk last crossed the bridge at. Only an edit can move
  /// either, so a playhead move re-walks the held model — which camera is live
  /// can change with the frame — without re-asking the engine anything.
  double? _fps;
  final Map<UuidValue, double> _zooms = {};

  /// The active camera layer: the topmost visible Camera whose span covers the
  /// playhead, which is the one the renderer looks through.
  ({LayerReference layer, CameraPose pose})? get _camera {
    // The **held** revision, not a checked one (K-232). Reading the checking
    // getter asks the engine whether the document has moved — and this runs on
    // every rebuild, which for a tool that draws its own pointer means every
    // movement of the mouse. That was the whole of the camera tools' chatter.
    final revision = widget.uiState.model.heldRevision;
    final frame = widget.uiState.playheadFrame.value;
    if (_heldRevision != revision) {
      _fps = null;
      _zooms.clear();
    }
    if (_heldRevision != revision || _heldFrame != frame) {
      _heldRevision = revision;
      _heldFrame = frame;
      _held = _findCamera(frame);
    }
    return _held;
  }

  /// The walk itself. Everything but the two reads noted above comes off the
  /// read model (K-184).
  ({LayerReference layer, CameraPose pose})? _findCamera(int frame) {
    for (final entry in widget.uiState.model.heldLayers) {
      final info = entry.info;
      if (info.kind != BridgeLayerKind.camera) continue;
      if (!info.switches.visible) continue;
      if (!_liveAt(info.span, frame)) continue;
      final tf = info.transform;
      double still(BridgeScalar s) =>
          s is BridgeScalar_Static ? s.field0 : double.nan;
      final pose = CameraPose(
        position: (
          still(tf.positionX),
          still(tf.positionY),
          still(tf.positionZ)
        ),
        rotation: (
          still(tf.rotationX),
          still(tf.rotationY),
          still(tf.rotation)
        ),
        distance: _distanceOf(entry.layer),
      );
      // A camera whose placement is keyframed has no single value for a drag to
      // add to — the same rule the layer gizmo follows.
      if ([
        pose.position.$1,
        pose.position.$2,
        pose.position.$3,
        pose.rotation.$1,
        pose.rotation.$2,
        pose.rotation.$3,
        pose.distance,
      ].any((v) => v.isNaN)) {
        return null;
      }
      return (layer: entry.layer, pose: pose);
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

  /// The layer's focal distance, held against the revision for the same
  /// reason as the rate. NaN — keyframed, or a layer that went away between
  /// the model and the read — is held too; the answer is the same until the
  /// document moves.
  double _distanceOf(LayerReference layer) {
    final held = _zooms[layer.internallayerId];
    if (held != null) return held;
    var distance = double.nan;
    try {
      final zoom = layer.getCameraZoom();
      if (zoom is BridgeScalar_Static) distance = zoom.field0;
    } catch (_) {
      // The layer went away between the model and the read.
    }
    _zooms[layer.internallayerId] = distance;
    return distance;
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return const SizedBox.shrink();
    return Positioned.fill(
      // The system pointer is hidden, because the drawn pointer below replaces
      // it (K-226).
      child: DrawnPointerRegion(
        onPointer: (at) => setState(() => _pointer = at),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapUp: (_) {
            if (_camera == null) _sayNoCamera();
          },
          onPanStart: _onPanStart,
          onPanUpdate: _onPanUpdate,
          onPanEnd: (_) => _onPanEnd(),
          onPanCancel: _onPanEnd,
          child: Stack(
            children: [
              Positioned.fill(
                child: CustomPaint(
                  painter: _CameraGizmoPainter(
                    // The pivot is the point the camera looks at, which is by
                    // construction the middle of the frame.
                    pivot: _camera == null ? null : widget.fitted.center,
                    orbiting: widget.tool == ToolMode.cameraOrbit,
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
          ),
        ),
      ),
    );
  }

  void _sayNoCamera() => widget.state.postNotice(
        l10n.noCameraToMove,
      );

  void _onPanStart(DragStartDetails details) {
    final camera = _camera;
    if (camera == null) {
      _sayNoCamera();
      return;
    }
    // The pointer is pinned where it was pressed for as long as the drag lasts
    // (K-230). Moving a camera is a gesture with no *place* — nothing on the
    // picture is being aimed at — so a pointer that wanders out of the Viewer,
    // and finally into the corner of the screen where it stops moving at all,
    // is a drag that ends before the user does. It reappears where it started
    // when the button comes up, which is what every 3D application does.
    _anchor = details.localPosition;
    _locked = freezeCursor();
    setState(() {
      _acting = camera.layer;
      _start = camera.pose;
      _delta = Offset.zero;
    });
  }

  void _onPanUpdate(DragUpdateDetails details) {
    if (_acting == null) return;
    final anchor = _anchor;
    if (_locked && anchor != null) {
      // Measured from where the pointer is *held*, not from the last event:
      // putting the pointer back is itself a movement, and the delta the
      // framework reports for that one exactly undoes the real one. Against the
      // anchor, the put-back event reads as no movement at all, which is the
      // truth of it.
      final moved = details.localPosition - anchor;
      if (moved == Offset.zero) return;
      setState(() => _delta += moved);
      restoreFrozenCursor();
    } else {
      setState(() => _delta += details.delta);
    }
    _write(preview: true);
  }

  void _onPanEnd() {
    if (_acting != null && _delta != Offset.zero) _write(preview: false);
    if (_locked) thawCursor();
    _locked = false;
    _anchor = null;
    setState(() {
      _acting = null;
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
    return switch (widget.tool) {
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
  /// same either way, because a camera has no preview path of its own (K-183's
  /// preview patches *one layer's* transform, and moving the camera changes what
  /// every layer looks like).
  void _write({required bool preview}) {
    final layer = _acting;
    final pose = _moved();
    if (layer == null || pose == null) return;
    try {
      layer.setTransforms(
        props: const [
          BridgeTransformProp.positionX,
          BridgeTransformProp.positionY,
          BridgeTransformProp.positionZ,
          BridgeTransformProp.rotationX,
          BridgeTransformProp.rotationY,
        ],
        values: [
          BridgeScalar.static_(pose.position.$1),
          BridgeScalar.static_(pose.position.$2),
          BridgeScalar.static_(pose.position.$3),
          BridgeScalar.static_(pose.rotation.$1),
          BridgeScalar.static_(pose.rotation.$2),
        ],
      );
      widget.onChanged();
    } catch (_) {
      // The camera was deleted mid-drag.
    }
  }
}

/// The camera gizmo: the point the camera is looking at, and — while orbiting —
/// the circle it would swing round.
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
