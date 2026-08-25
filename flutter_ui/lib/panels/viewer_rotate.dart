// The Rotation tool (K-219, docs/07 §1.7): the curved pointer, and the drag
// that turns whatever is selected about its own anchor point.
//
// **In plain terms.** With this tool in hand the pointer becomes a curved arrow
// — the mark After Effects uses — and dragging anywhere over the picture turns
// the selected layer(s). It turns them about their **anchor point**, which is
// the whole reason the anchor exists: it is the pin the layer spins on. Holding
// Shift locks the turn to 45° steps.
//
// **Why the pointer is painted rather than picked.** No operating system ships a
// "rotate" cursor, and Flutter can only ask for cursors the platform has. So the
// system pointer is hidden over the picture and this draws its own — which is
// also what makes it able to *turn*: the arc leans the way the layer would turn
// from where the pointer is, sharper at a corner and shallower along an edge, so
// the pointer itself says which way the drag will go. That is a real cursor
// behaviour we could not otherwise have, and the only reason hiding the system
// one is worth it.
//
// The arithmetic — which way the arc leans, how tight it is, and what angle a
// drag has swept — is pure and tested; the widget only listens and commits.

import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../state/preview_throttle.dart';
import 'viewer_gizmo.dart';
import 'viewer_tool_cursor.dart';

/// How wide the drawn pointer's arc is, in radians: shallow when the pointer is
/// out along an edge, tight when it is out towards a corner.
///
/// The difference is the whole point of the two shapes. An edge is a *direction*
/// — the layer turns past you — so the arc is long and lazy; a corner is a
/// *pivot*, where both axes are turning at once, so the arc closes up. After
/// Effects draws the same distinction, and it is what tells you where you are on
/// the layer without looking away from the picture.
const double rotateCursorEdgeSweep = 1.1; // ~63°
const double rotateCursorCornerSweep = 2.1; // ~120°

/// How many positions the pointer takes: the four edges and the four corners,
/// and nothing between (K-230).
///
/// It used to lean by a continuously varying angle, which was true to the
/// geometry and worse to read — a mark that is never twice the same shape is a
/// mark the eye has to re-read every time. Eight settled shapes are eight things
/// to recognise, and the one that is showing says which quarter of the layer the
/// pointer is in, which is all it was ever telling you.
const int rotateCursorPositions = 8;

/// The radius the arc is drawn at, in screen pixels. A cursor, so a fixed size
/// on screen at any magnification.
const double rotateCursorRadius = 9;

/// How the rotation pointer is drawn at [pointer] for [box].
///
/// [angle] is the direction the arc faces, in radians, **snapped to one of the
/// eight compass points of the layer's own box** — north, north-east, east and
/// round. [sweep] follows from which of the eight it landed on: the four edges
/// take the long lazy arc, the four corners the tight one. So the pointer has
/// eight settled shapes rather than a continuum, and which one is showing says
/// which part of the layer you are over.
///
/// The compass is the *layer's*, not the screen's: it is measured in layer
/// space, so "the top-right corner" stays the top-right corner of the layer when
/// the layer is turned upside down, and the drawn arc turns with it.
///
/// With no box — nothing selected — there is nothing to lean round, so the arc
/// takes the edge shape and points up: a pointer that says "rotate" without
/// claiming a direction it cannot know.
({double angle, double sweep}) rotateCursorFor({
  required Offset pointer,
  LayerBox? box,
}) {
  const up = (angle: -math.pi / 2, sweep: rotateCursorEdgeSweep);
  if (box == null) return up;
  final w = box.bounds.width;
  final h = box.bounds.height;
  if (w <= 0 || h <= 0) return up;

  // Where the pointer is over the box, in halves-of-the-box from its middle:
  // (0, 0) dead centre, (1, 1) the bottom-right corner. Normalising by the
  // box's own size is what makes "corner" mean the corner rather than 45° on
  // screen, which on a wide layer is nowhere near one.
  final p = box.map.layerOf(pointer);
  final a = (p.dx / w - 0.5) * 2;
  final b = (p.dy / h - 0.5) * 2;
  if (a.abs() < 1e-9 && b.abs() < 1e-9) return up;

  // The nearest of the eight, as a number of eighths of a turn.
  final step =
      (math.atan2(b, a) / (2 * math.pi / rotateCursorPositions)).round();
  final settled = step * 2 * math.pi / rotateCursorPositions;
  // An odd eighth is a diagonal, which is a corner; an even one is square out
  // from an edge.
  final sweep = step.isOdd ? rotateCursorCornerSweep : rotateCursorEdgeSweep;

  // Back out to a screen direction through the box's own map, so the settled
  // direction is drawn where that part of the layer actually is.
  final centre = box.map.toScreen(w / 2, h / 2);
  final outward = box.map.toScreen(
    w / 2 * (1 + math.cos(settled)),
    h / 2 * (1 + math.sin(settled)),
  );
  final radial = outward - centre;
  if (radial.distance < 1e-6) return (angle: up.angle, sweep: sweep);
  return (angle: math.atan2(radial.dy, radial.dx), sweep: sweep);
}

/// The Rotation tool over the picture.
class ViewerRotateLayer extends StatefulWidget {
  /// Whether the tool is armed. Inert otherwise — no pointer taken, no cursor
  /// hidden.
  final bool active;

  final CompositionReference comp;
  final LumitUiState uiState;

  /// Every layer with its box, top first — for the click that selects and for
  /// the anchors the turn happens about.
  final List<LayerBox> boxes;

  /// The pointer's own colours: the mark and the outline that keeps it legible
  /// over any picture.
  final Color mark;
  final Color outline;

  final VoidCallback onChanged;

  const ViewerRotateLayer({
    super.key,
    required this.active,
    required this.comp,
    required this.uiState,
    required this.boxes,
    required this.mark,
    required this.outline,
    required this.onChanged,
  });

  @override
  State<ViewerRotateLayer> createState() => _ViewerRotateLayerState();
}

class _ViewerRotateLayerState extends State<ViewerRotateLayer> {
  Offset? _pointer;

  /// Where the turn is measured from: the point the pointer went *down*, not
  /// where the framework recognised the drag.
  ///
  /// The same trap the gizmo's handles fell into (K-217): a pan is only
  /// recognised after the pointer has travelled its slop, so `DragStartDetails`
  /// is already some way round the circle — and every turn came out short by
  /// however far that was.
  Offset? _from;
  Offset? _downAt;

  /// Each selected layer's rotation when the drag began, by layer id. Captured
  /// once: every update adds the swept angle to *these*, so a drag that goes
  /// out and comes back leaves the layer where it started rather than
  /// accumulating a little error each frame.
  final Map<String, double> _startRotations = {};

  final PreviewThrottle _throttle = PreviewThrottle();

  @override
  void dispose() {
    _throttle.cancel();
    // A drag interrupted by the panel going away must not leave the boxes
    // turned to an angle nothing is committing.
    widget.uiState.liveRotations.value = const {};
    super.dispose();
  }

  /// The selected layers' boxes, in stacking order.
  List<LayerBox> get _selected {
    final ids = widget.uiState.selectedLayerIds;
    return [for (final box in widget.boxes) if (ids.contains(box.id)) box];
  }

  /// The box the pointer's shape leans round: the layer being turned, or — with
  /// nothing selected — the one under the pointer, so the cursor still reads as
  /// "this is what you would be turning".
  LayerBox? _shapeBox(Offset at) {
    final selected = _selected;
    if (selected.isNotEmpty) return selected.first;
    return layerAtPoint(widget.boxes, at);
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return const SizedBox.shrink();
    final at = _pointer;
    final shape = at == null
        ? null
        : rotateCursorFor(pointer: at, box: _shapeBox(at));

    return Positioned.fill(
      // The system pointer is hidden, because what replaces it is drawn below —
      // and a system arrow sitting inside the curved mark would read as two
      // pointers.
      child: DrawnPointerRegion(
        onPointer: (at) => setState(() => _pointer = at),
        child: Listener(
          onPointerDown: (event) => _downAt = event.localPosition,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
          // Clicking picks what to turn, exactly as it does with the Selection
          // tool: a rotation tool you cannot choose a layer with would send you
          // back to the toolbar between every turn.
            onTapUp: _onTapUp,
            onPanStart: _onPanStart,
            onPanUpdate: _onPanUpdate,
            onPanEnd: (_) => _onPanEnd(),
            onPanCancel: _onPanEnd,
            child: CustomPaint(
              painter: _RotateCursorPainter(
                at: at,
                angle: shape?.angle ?? 0,
                sweep: shape?.sweep ?? rotateCursorEdgeSweep,
                mark: widget.mark,
                outline: widget.outline,
              ),
            ),
          ),
        ),
      ),
    );
  }

  void _onTapUp(TapUpDetails details) {
    final hit = layerAtPoint(widget.boxes, details.localPosition);
    if (hit == null) {
      widget.uiState.clearSelection();
      return;
    }
    if (HardwareKeyboard.instance.isShiftPressed) {
      widget.uiState.toggleSelected(hit.layer);
    } else {
      widget.uiState.setSelection([hit.layer]);
    }
  }

  void _onPanStart(DragStartDetails details) {
    _startRotations.clear();
    for (final box in _selected) {
      if (box.scalable) _startRotations[box.id.uuid] = box.rotationDegrees;
    }
    setState(() {
      _from = _downAt ?? details.localPosition;
      _pointer = details.localPosition;
    });
  }

  void _onPanUpdate(DragUpdateDetails details) {
    setState(() => _pointer = details.localPosition);
    final turned = _rotations();
    // What the boxes over the picture are drawn at while the turn is in flight
    // (K-230). Published for every selected layer, not only the one that
    // previews: the wireframe follows the pointer whether or not the picture
    // underneath can.
    widget.uiState.liveRotations.value = {
      for (final (box, degrees) in turned) box.id: degrees,
    };
    if (turned.length != 1) return;
    // One layer previews live, as everywhere else: the engine patches a single
    // layer's transform into its clone of the document, so a set being turned
    // shows on release. The boxes turn either way.
    final (box, degrees) = turned.single;
    _throttle.request(() {
      try {
        widget.comp.renderFrameWithTransformPreview(
          frame: BigInt.from(widget.uiState.playheadFrame.value),
          scale: widget.uiState.viewerScale,
          layer: box.layer,
          transform: transformWith(box.layer.getTransform(), rotation: degrees),
        );
      } catch (_) {
        // A preview is a courtesy; the turn still lands (K-217).
      }
    });
  }

  void _onPanEnd() {
    final turned = _rotations();
    _throttle.cancel();
    // The document is about to hold the angle itself, so the in-flight one has
    // to go — a stale entry here would keep the box turned past the commit.
    widget.uiState.liveRotations.value = const {};
    for (final (box, degrees) in turned) {
      try {
        box.layer.setTransform(
          prop: BridgeTransformProp.rotation,
          value: BridgeScalar.static_(degrees),
        );
      } catch (_) {
        // A layer deleted mid-drag. The rest still turn.
      }
    }
    setState(() {
      _from = null;
      _startRotations.clear();
    });
    if (turned.isNotEmpty) widget.onChanged();
  }

  /// Where every selected layer would be turned to, for the drag as it stands.
  ///
  /// The swept angle is measured about the **first** selected layer's anchor and
  /// applied to all of them, so a set turns as one gesture rather than each
  /// layer chasing a different angle from the same pointer. Each still turns
  /// about *its own* anchor, which is what the transform means.
  List<(LayerBox, double)> _rotations() {
    final from = _from;
    final to = _pointer;
    final selected = _selected;
    if (from == null || to == null || selected.isEmpty) return const [];
    final pivot = selected.first.anchorScreen;
    final uniform = HardwareKeyboard.instance.isShiftPressed;
    final out = <(LayerBox, double)>[];
    for (final box in selected) {
      final start = _startRotations[box.id.uuid];
      if (start == null) continue;
      out.add((
        box,
        rotationForDrag(
          anchor: pivot,
          from: from,
          to: to,
          current: start,
          uniform: uniform,
        )
      ));
    }
    return out;
  }
}

/// The curved-arrow pointer, drawn at the pointer's own position.
///
/// Two passes: a thicker stroke in the outline colour and the mark over it, so
/// it stays readable over black, over white and over anything between — the
/// same trick every operating system's own cursors use.
class _RotateCursorPainter extends CustomPainter {
  final Offset? at;
  final double angle;
  final double sweep;
  final Color mark;
  final Color outline;

  const _RotateCursorPainter({
    required this.at,
    required this.angle,
    required this.sweep,
    required this.mark,
    required this.outline,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final centre = at;
    if (centre == null) return;
    canvas.save();
    canvas.translate(centre.dx, centre.dy);
    // The arc is drawn round the pointer and turned so its middle faces away
    // from the anchor: the curve then always reads as "round the pivot".
    canvas.rotate(angle);
    paintTwoPassStroke(outline, mark, (paint) => _draw(canvas, paint),
        outlineWidth: 3.4, markWidth: 1.6, rounded: true);
    canvas.restore();
  }

  void _draw(Canvas canvas, Paint paint) {
    final rect = Rect.fromCircle(center: Offset.zero, radius: rotateCursorRadius);
    canvas.drawArc(rect, -sweep / 2, sweep, false, paint);

    // An arrowhead at each end, tangent to the arc, so the mark says the turn
    // goes either way — which it does.
    for (final end in [-sweep / 2, sweep / 2]) {
      final point = Offset(
        math.cos(end) * rotateCursorRadius,
        math.sin(end) * rotateCursorRadius,
      );
      // The tangent at this end, pointing away from the arc's middle.
      final direction = end < 0 ? -1.0 : 1.0;
      final tangent =
          Offset(-math.sin(end), math.cos(end)) * direction;
      final normal = Offset(-tangent.dy, tangent.dx);
      const head = 4.0;
      canvas.drawLine(
          point, point - tangent * head + normal * head * 0.6, paint);
      canvas.drawLine(
          point, point - tangent * head - normal * head * 0.6, paint);
    }
  }

  @override
  bool shouldRepaint(_RotateCursorPainter old) =>
      old.at != at ||
      old.angle != angle ||
      old.sweep != sweep ||
      old.mark != mark ||
      old.outline != outline;
}
