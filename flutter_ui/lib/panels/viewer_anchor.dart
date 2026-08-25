// The Anchor point tool — After Effects calls it Pan Behind (K-220,
// docs/07 §1.7): drag a layer's anchor without the picture moving.
//
// **In plain terms.** The anchor point is the spot a layer scales and rotates
// about, and it is also the spot Position places. Move it naively and the layer
// jumps, because the same Position now means somewhere else. This tool moves it
// *and* compensates Position by exactly the amount that cancels the jump — so
// the picture stays where it is and only the pivot slides. That is what "pan
// behind" means, and it is the difference between this tool and typing new
// anchor numbers into Effect controls.
//
// **The two modifiers, both After Effects'.**
// * `Shift` constrains the drag to one axis, so a pivot can be moved straight
//   across a face without drifting up or down.
// * `Ctrl` (`Cmd`) snaps the anchor to the layer's own key points — the four
//   corners, the four edge midpoints, and the centre — which is how a pivot
//   lands *exactly* on a corner rather than nearly on one. The snap is measured
//   in screen pixels, so it is as precise as the magnification allows and no
//   more (docs/07 §4.5's rule for every snap in the application).
//
// The maths is `panBehindPosition` in viewer_layer_map.dart, which the egui
// frontend's anchor overlay used and which is already unit-tested; the snapping
// and the axis lock are pure functions here.

import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../state/preview_throttle.dart';
import 'viewer_gizmo.dart';
import 'viewer_layer_map.dart';
import 'viewer_tool_cursor.dart';

/// How close, in screen pixels, a snapped anchor has to be to a key point.
const double anchorSnapDistance = 12;

/// The size of the drawn pointer's crosshair, in screen pixels.
const double anchorCursorSize = 9;

/// The layer's own key points, in layer space: the four corners, the four edge
/// midpoints, and the centre — nine places a pivot is usually wanted.
List<Offset> anchorKeyPoints(Size bounds) => [
      for (final y in const [0.0, 0.5, 1.0])
        for (final x in const [0.0, 0.5, 1.0])
          Offset(x * bounds.width, y * bounds.height),
    ];

/// [candidate] (layer space) snapped to the nearest key point within
/// [anchorSnapDistance] **on screen**, or unchanged when none is near.
///
/// Measured on screen rather than in layer pixels on purpose: a layer scaled to
/// 10% would otherwise snap from half a screen away, and one scaled to 1000%
/// would never snap at all. This is the same rule docs/07 §4.5 sets for the
/// Timeline's snapping — the distance a user can see is the distance that
/// counts.
Offset snapAnchor(Offset candidate, LayerBox box) {
  final onScreen = box.map.toScreen(candidate.dx, candidate.dy);
  Offset? best;
  var bestDistance = anchorSnapDistance;
  for (final point in anchorKeyPoints(box.bounds)) {
    final d = (box.map.toScreen(point.dx, point.dy) - onScreen).distance;
    if (d <= bestDistance) {
      bestDistance = d;
      best = point;
    }
  }
  return best ?? candidate;
}

/// [delta] with the smaller of its two components dropped — Shift's axis lock.
///
/// In *screen* space, because the lock is about the gesture the hand is making,
/// not about the layer's own axes: dragging straight across the screen should
/// stay straight across the screen even on a layer that is turned.
Offset constrainToAxis(Offset delta) => delta.dx.abs() >= delta.dy.abs()
    ? Offset(delta.dx, 0)
    : Offset(0, delta.dy);

/// Ctrl (Cmd on a Mac): the snap modifier, spelled the way the keymap spells
/// its primary modifier (state/keymap.dart).
bool isPrimaryModifierHeld() => defaultTargetPlatform == TargetPlatform.macOS
    ? HardwareKeyboard.instance.isMetaPressed
    : HardwareKeyboard.instance.isControlPressed;

/// Where the anchor should sit, in layer space, for a pointer at [screen] —
/// with the two modifiers every pivot gesture shares (K-220): Shift locks the
/// drag to one screen axis measured from [lockFrom], Ctrl (Cmd) snaps to the
/// layer's own key points. Shared by this tool and the gizmo's anchor handle
/// (K-221), so the two cannot drift apart.
Offset wantedAnchorAt(LayerBox box, Offset screen, {Offset? lockFrom}) {
  var at = screen;
  if (lockFrom != null && HardwareKeyboard.instance.isShiftPressed) {
    at = lockFrom + constrainToAxis(screen - lockFrom);
  }
  final wanted = box.map.layerOf(at);
  return isPrimaryModifierHeld() ? snapAnchor(wanted, box) : wanted;
}

/// The Position that keeps the picture still while [box]'s anchor moves to
/// [anchor] — the pan-behind sum bound to a box's own numbers, shared by every
/// gesture that slides a pivot. The maths itself is `panBehindPosition` in
/// viewer_layer_map.dart, ported and unit-tested.
Offset panBehindFor(LayerBox box, Offset anchor) => panBehindPosition(
      oldAnchor: Offset(box.map.ax, box.map.ay),
      newAnchor: anchor,
      position: Offset(box.map.px, box.map.py),
      scaleXPercent: box.map.sx * 100,
      scaleYPercent: box.map.sy * 100,
      rotationDegrees: box.rotationDegrees,
    );

/// The Anchor point tool over the picture.
class ViewerAnchorLayer extends StatefulWidget {
  /// Whether the tool is armed. Inert otherwise.
  final bool active;

  final CompositionReference comp;
  final LumitUiState uiState;

  /// Every layer with its box, top first.
  final List<LayerBox> boxes;

  final Color mark;
  final Color outline;
  final Color accent;

  final VoidCallback onChanged;

  const ViewerAnchorLayer({
    super.key,
    required this.active,
    required this.comp,
    required this.uiState,
    required this.boxes,
    required this.mark,
    required this.outline,
    required this.accent,
    required this.onChanged,
  });

  @override
  State<ViewerAnchorLayer> createState() => _ViewerAnchorLayerState();
}

class _ViewerAnchorLayerState extends State<ViewerAnchorLayer> {
  Offset? _pointer;

  /// The press, for the same reason every other tool records it (K-217): a drag
  /// is only recognised once the pointer has travelled its slop, and a pivot
  /// that jumped by that much on the first frame of every drag would be
  /// unusable.
  Offset? _downAt;

  /// The layer being panned behind, captured at the press so the maths is
  /// relative to where it started rather than to a document it is changing.
  LayerBox? _acting;

  final PreviewThrottle _throttle = PreviewThrottle();

  @override
  void dispose() {
    _throttle.cancel();
    super.dispose();
  }

  /// The layer this tool acts on: the one under the pointer if it is selected
  /// or nothing is, else the primary selection — the same "what would this
  /// gesture touch?" the Selection tool answers.
  LayerBox? _targetAt(Offset at) {
    final ids = widget.uiState.selectedLayerIds;
    final under = layerAtPoint(widget.boxes, at);
    if (under != null && (ids.isEmpty || ids.contains(under.id))) return under;
    for (final box in widget.boxes) {
      if (ids.contains(box.id)) return box;
    }
    return under;
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return const SizedBox.shrink();
    final at = _pointer;
    final target = _acting ?? (at == null ? null : _targetAt(at));
    return Positioned.fill(
      child: DrawnPointerRegion(
        onPointer: (at) => setState(() => _pointer = at),
        child: Listener(
          onPointerDown: (event) => _downAt = event.localPosition,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTapUp: _onTapUp,
            onPanStart: _onPanStart,
            onPanUpdate: _onPanUpdate,
            onPanEnd: (_) => _onPanEnd(),
            onPanCancel: _onPanEnd,
            child: CustomPaint(
              painter: _AnchorCursorPainter(
                at: at,
                // Where the anchor is going, so the drag shows the pivot moving
                // even though the picture deliberately does not.
                anchor: target == null
                    ? null
                    : _acting != null && at != null
                        ? target.map.toScreen(
                            _wantedAnchor(target, at).dx,
                            _wantedAnchor(target, at).dy,
                          )
                        : target.anchorScreen,
                mark: widget.mark,
                outline: widget.outline,
                accent: widget.accent,
              ),
            ),
          ),
        ),
      ),
    );
  }

  /// A click **puts the pivot where you clicked** (K-232), on the layer it
  /// lands on — and picks that layer, as every tool's click does.
  ///
  /// Shift is the exception and stays a selection gesture: it adds to or takes
  /// away from the selection without moving anything, because a click that both
  /// changed what was selected and moved that layer's pivot would be two edits
  /// nobody asked for at once.
  void _onTapUp(TapUpDetails details) {
    final at = details.localPosition;
    final hit = layerAtPoint(widget.boxes, at);
    if (hit == null) {
      widget.uiState.clearSelection();
      return;
    }
    if (HardwareKeyboard.instance.isShiftPressed) {
      widget.uiState.toggleSelected(hit.layer);
      return;
    }
    widget.uiState.setSelection([hit.layer]);
    _place(hit, at);
  }

  /// Move [box]'s anchor to the pointer at [at], with Position compensating so
  /// the picture does not move. One op, so one undo step.
  void _place(LayerBox box, Offset at) {
    final anchor = _wantedAnchor(box, at);
    final position = panBehindFor(box, anchor);
    try {
      box.layer.setTransforms(
        props: const [
          BridgeTransformProp.anchorX,
          BridgeTransformProp.anchorY,
          BridgeTransformProp.positionX,
          BridgeTransformProp.positionY,
        ],
        values: [
          BridgeScalar.static_(anchor.dx),
          BridgeScalar.static_(anchor.dy),
          BridgeScalar.static_(position.dx),
          BridgeScalar.static_(position.dy),
        ],
      );
      widget.onChanged();
    } catch (_) {
      // The layer went away between the press and the release.
    }
  }

  void _onPanStart(DragStartDetails details) {
    final at = _downAt ?? details.localPosition;
    final target = _targetAt(at);
    // Dragging a layer's pivot is working on that layer, so it becomes the
    // selection — the same rule the Selection tool's body drag follows.
    if (target != null &&
        !widget.uiState.selectedLayerIds.contains(target.id)) {
      widget.uiState.setSelection([target.layer]);
    }
    setState(() {
      _acting = target;
      _pointer = details.localPosition;
    });
  }

  /// Where the anchor should sit, in layer space, for the pointer at [at].
  ///
  /// **The pointer's own position, not a nudge** (K-232). The tool used to
  /// measure the drag from the press and add it to the anchor the layer already
  /// had, so grabbing anywhere and pushing moved the pivot by that much. That
  /// makes placing a pivot a matter of aim-then-correct: you cannot put it
  /// somewhere, only push it towards somewhere. Now the pivot goes where the
  /// pointer is — a click puts it there, and a drag keeps it under the pointer
  /// the whole way.
  ///
  /// Shift still locks to one screen axis, measured from where the press
  /// landed; Ctrl (Cmd) still snaps to the layer's own key points.
  Offset _wantedAnchor(LayerBox box, Offset at) =>
      wantedAnchorAt(box, at, lockFrom: _downAt ?? at);

  void _onPanUpdate(DragUpdateDetails details) {
    setState(() => _pointer = details.localPosition);
    final box = _acting;
    final at = _pointer;
    if (box == null || at == null) return;
    final anchor = _wantedAnchor(box, at);
    final position = panBehindFor(box, anchor);
    _throttle.request(() {
      try {
        widget.comp.renderFrameWithTransformPreview(
          frame: BigInt.from(widget.uiState.playheadFrame.value),
          scale: widget.uiState.viewerScale,
          layer: box.layer,
          transform: transformWith(
            box.layer.getTransform(),
            anchorX: anchor.dx,
            anchorY: anchor.dy,
            positionX: position.dx,
            positionY: position.dy,
          ),
        );
      } catch (_) {
        // A preview is a courtesy (K-217); the commit still lands.
      }
    });
  }

  void _onPanEnd() {
    final box = _acting;
    final at = _pointer;
    _throttle.cancel();
    // One op for all four properties, so one drag is one undo step: the anchor
    // and the position are only meaningful together here — half of this edit
    // would move the picture, which is the one thing pan-behind promises not to
    // do.
    if (box != null && at != null && at != _downAt) _place(box, at);
    setState(() {
      _acting = null;
      _downAt = null;
    });
  }
}

/// The pan-behind pointer: the anchor mark itself, with a small arrow at its
/// tail — After Effects' own pairing, and it says exactly what the tool moves.
///
/// The mark at the *pointer* is the cursor; the ring drawn at the layer's anchor
/// is where that pivot actually is, which is the thing being aimed.
class _AnchorCursorPainter extends CustomPainter {
  final Offset? at;
  final Offset? anchor;
  final Color mark;
  final Color outline;
  final Color accent;

  const _AnchorCursorPainter({
    required this.at,
    required this.anchor,
    required this.mark,
    required this.outline,
    required this.accent,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final pivot = anchor;
    if (pivot != null) {
      paintAnchorMark(canvas, pivot, accent, reach: 9);
    }

    final point = at;
    if (point == null) return;
    canvas.save();
    canvas.translate(point.dx, point.dy);
    paintTwoPassStroke(outline, mark, (paint) => _cursor(canvas, paint),
        outlineWidth: 3.2, markWidth: 1.4, rounded: true);
    canvas.restore();
  }

  /// A **reticle**, centred on the point it acts at (K-235).
  ///
  /// It used to carry a small arrow off its tail, down and to the right, so the
  /// mark would read as a pointer rather than as an overlay. That was a lie
  /// about the one thing a pointer must be honest about: the arrow's tip is not
  /// where the pivot lands — the middle of the ring is — so the mark pointed at
  /// somewhere the tool was not going to act.
  ///
  /// The arms stop short of the middle. A reticle's gap is what leaves the
  /// exact point visible instead of covering it with the mark that is supposed
  /// to be aiming at it.
  void _cursor(Canvas canvas, Paint paint) {
    const r = anchorCursorSize / 2;
    canvas.drawCircle(Offset.zero, r, paint);
    for (final (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)]) {
      canvas.drawLine(
        Offset(dx * r * 0.55, dy * r * 0.55),
        Offset(dx * r * 2.0, dy * r * 2.0),
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(_AnchorCursorPainter old) =>
      old.at != at ||
      old.anchor != anchor ||
      old.mark != mark ||
      old.outline != outline ||
      old.accent != accent;
}

/// Straight-line distance, for the tests' convenience.
@visibleForTesting
double distanceBetween(Offset a, Offset b) =>
    math.sqrt(math.pow(a.dx - b.dx, 2) + math.pow(a.dy - b.dy, 2));
