// The shape tools and the Pen over the picture: the drag that draws a mask, and
// the Pen's point-by-point path (K-222, K-223, docs/07 §2.3).
//
// **In plain terms.** With a shape tool in hand and a layer selected, dragging
// over the picture draws a mask on that layer — a rectangle, a rounded
// rectangle, an ellipse, a polygon or a star, between the two corners you
// dragged, with Shift keeping it square. The **Pen** is different: it builds a
// path a point at a time, and clicking its first point again closes and applies
// it. (That gesture was briefly on the polygon tool; it is After Effects' pen,
// and it belongs on the Pen — K-223.)
//
// **What it does with nothing selected.** Makes a *shape layer* — the art
// itself rather than a hole in someone else's picture (K-237). It is the same
// gesture and the same geometry; the only difference is which thing the path
// ends up belonging to, and therefore which coordinates it is built in: the
// layer's when there is one, the composition's when there is not.
//
// The geometry is in viewer_shapes.dart and is pure; this is the gesture and
// the drawing.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'viewer_gizmo.dart';
import 'viewer_tool_cursor.dart';
import 'viewer_shapes.dart';

/// How big the "this click closes the path" ring is drawn, in screen pixels.
///
/// The same order as the closing tolerance itself (10 screen pixels, see
/// [withinClosingDistance]), so the mark is honest about the target it stands
/// for rather than being a decoration near it.
const double closingRingRadius = 10;

/// How solid a tool draws the thing it is about to make (K-238).
///
/// Shared by the shape tools and the Pen so every preview reads the same way.
/// Low enough that the picture underneath still shows — which is what says the
/// shape is not there yet — and high enough to answer "what colour is this?".
const double previewOpacity = 0.5;

/// The shape tools over the picture.
class ViewerShapeLayer extends StatefulWidget {
  /// Whether a shape tool is armed. Inert otherwise.
  final bool active;

  final ToolMode tool;
  final LumitState state;
  final LumitUiState uiState;

  /// Every layer with its box, top first — for the layer being masked and the
  /// map that turns the pointer into layer coordinates.
  final List<LayerBox> boxes;

  /// The composition, for the shape layer a drag makes when nothing is
  /// selected (K-237).
  final CompositionReference comp;

  /// Where the picture sits on screen, and the comp's own size — the two that
  /// turn a pointer into composition pixels when there is no layer to ask.
  final Rect fitted;
  final Size compSize;

  final Color accent;

  final VoidCallback onChanged;

  const ViewerShapeLayer({
    super.key,
    required this.active,
    required this.tool,
    required this.state,
    required this.uiState,
    required this.boxes,
    required this.comp,
    required this.fitted,
    required this.compSize,
    required this.accent,
    required this.onChanged,
  });

  @override
  State<ViewerShapeLayer> createState() => _ViewerShapeLayerState();
}

class _ViewerShapeLayerState extends State<ViewerShapeLayer> {
  /// The drag in flight, in screen space.
  Offset? _from;
  Offset? _to;

  /// Where the pointer went down, for the same reason every other tool records
  /// it (K-217): the framework only reports a drag once it has travelled its
  /// slop, and a shape that started 18px from where you pressed is the wrong
  /// shape.
  Offset? _downAt;

  /// The path being built with the Pen, and the pointer drawing its next edge.
  PathDraft _draft = const PathDraft();
  Offset? _penPointer;

  /// Where the pointer is, for the drawn cursor (K-226). Tracked for every
  /// shape tool, not only the Pen, because every one of them wears one.
  Offset? _pointer;

  /// The handle being pulled out of the vertex just placed, if the click that
  /// placed it turned into a drag.
  Offset? _handleFrom;
  Offset? _handleTo;

  /// Whether the Pen is in hand. Only the Pen itself builds a path; its four
  /// siblings (add/delete/convert vertex, mask feather) edit a *finished* one,
  /// which is not built (docs/TODO.md).
  bool get _isPen => widget.tool == ToolMode.pen;

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_onKey);
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    super.dispose();
  }

  /// Escape abandons a path in progress; Backspace takes back its last point.
  /// Both are what every path tool does, and both are why a half-drawn path is
  /// never a trap.
  ///
  /// **`Ctrl+Z` takes back a point too, while a path is being built** (K-232).
  /// This is the one place the application's undo means something narrower than
  /// "undo the last edit": the points are not in the document yet — the path is
  /// applied in one op when it closes — so an undo pressed mid-path used to
  /// sail past every point placed and undo whatever the user had done *before*
  /// picking up the Pen, which is never what was meant. It goes back to the
  /// document's own undo the moment the path is empty.
  bool _onKey(KeyEvent event) {
    if (!widget.active || !_isPen || _draft.isEmpty) return false;
    if (event is! KeyDownEvent) return false;
    if (event.logicalKey == LogicalKeyboardKey.escape) {
      setState(() => _draft = const PathDraft());
      return true;
    }
    final undo = event.logicalKey == LogicalKeyboardKey.keyZ &&
        (HardwareKeyboard.instance.isControlPressed ||
            HardwareKeyboard.instance.isMetaPressed);
    if (undo || event.logicalKey == LogicalKeyboardKey.backspace) {
      setState(() => _draft = _draft.withoutLast());
      return true;
    }
    return false;
  }

  /// Whether a click where the pointer is would **close** the path (K-232).
  ///
  /// The closing tolerance is a fixed number of screen pixels, and until this
  /// was drawn there was nothing at all to say how near "near enough" was: you
  /// clicked, and either the path closed or it grew a point you did not want.
  /// The mark is the answer to "how close do I need to be" — the first vertex
  /// grows a ring, and the pointer says a click will close rather than place.
  /// Answered through [_space], so the ring appears whether the path is
  /// becoming a mask or a shape layer — it used to need a selected layer, so
  /// the half of the gesture that makes a shape layer had no closing mark.
  bool get _wouldClose {
    if (!_isPen || !_draft.canClose) return false;
    final at = _penPointer;
    final start = _draft.first;
    if (at == null || start == null) return false;
    final space = _space;
    return withinClosingDistance(
      space.ofScreen(at),
      start,
      screenScale: space.screenScale,
    );
  }

  /// The layer a shape would be drawn on: the primary selection.
  LayerBox? get _target {
    final ids = widget.uiState.selectedLayerIds;
    for (final box in widget.boxes) {
      if (ids.contains(box.id)) return box;
    }
    return null;
  }

  /// The space the art being drawn lives in, and how to get it back on screen
  /// (K-238).
  ///
  /// The preview used to be drawn only when a layer was selected, because it
  /// asked that layer's map to place every point. With nothing selected — the
  /// case that makes a *shape layer*, which is most of the reason to reach for
  /// a shape tool — there was no map, so nothing was drawn: you dragged and saw
  /// nothing until you let go. The composition's own placement is the map in
  /// that case.
  ShapeSpace get _space {
    final box = _target;
    if (box != null) return ShapeSpace.ofLayer(box);
    return ShapeSpace.ofComp(
        fitted: widget.fitted, compSize: widget.compSize);
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return const SizedBox.shrink();
    final t = ThemeScope.of(context).theme;
    final target = _target;
    final space = _space;
    return Positioned.fill(
      // The system pointer is hidden, because the drawn pointer below replaces
      // it (K-226): the eyedropper's crosshair, badged with this tool's own
      // icon.
      child: DrawnPointerRegion(
        onPointer: (at) => setState(() {
          _pointer = at;
          // The Pen also draws the edge it would place next, from the last
          // point placed to here.
          _penPointer = _isPen ? at : null;
        }),
        child: Listener(
          onPointerDown: (event) => _downAt = event.localPosition,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTapUp: _isPen ? _onPenTap : null,
            onPanStart: _onPanStart,
            onPanUpdate: _onPanUpdate,
            onPanEnd: (_) => _onPanEnd(),
            onPanCancel: _onPanCancel,
            child: Stack(children: [
              Positioned.fill(
                child: CustomPaint(
                  painter: ShapePreviewPainter(
                    tool: widget.tool,
                    space: space,
                    // What the art would be committed with. A mask has no
                    // colour of its own — it cuts — so a drag on a selected
                    // layer previews in the accent instead of promising a fill
                    // that will never appear.
                    fill: target == null
                        ? colourOf(widget.uiState.tools.fill)
                        : widget.accent,
                    stroke: target == null
                        ? colourOf(widget.uiState.tools.stroke)
                        : null,
                    strokeWidth: target == null
                        ? widget.uiState.tools.strokeWidth * space.screenScale
                        : 0,
                    from: _from,
                    to: _to,
                    square: HardwareKeyboard.instance.isShiftPressed,
                    draft: _draft,
                    penPointer: _penPointer,
                    handleFrom: _handleFrom,
                    handleTo: _handleTo,
                    closing: _wouldClose,
                    accent: widget.accent,
                  ),
                ),
              ),
              ToolPointer(
                at: _pointer,
                tool: widget.tool,
                mark: t.textPrimary,
                outline: t.surface0,
              ),
            ]),
          ),
        ),
      ),
    );
  }

  // --- The dragged shapes ---------------------------------------------------

  void _onPanStart(DragStartDetails details) {
    final at = _downAt ?? details.localPosition;
    // Where the crosshair is drawn is [DrawnPointerRegion]'s business, whichever
    // button is down (K-230).
    if (_isPen) {
      // A click that became a drag: the vertex lands where the press was, and
      // the drag pulls its handles out.
      setState(() {
        _handleFrom = at;
        _handleTo = details.localPosition;
      });
      return;
    }
    setState(() {
      _from = at;
      _to = details.localPosition;
    });
  }

  void _onPanUpdate(DragUpdateDetails details) {
    setState(() {
      _pointer = details.localPosition;
      if (_isPen) {
        _handleTo = details.localPosition;
        _penPointer = details.localPosition;
      } else {
        _to = details.localPosition;
      }
    });
  }

  void _onPanEnd() {
    if (_isPen) {
      _finishHandleDrag();
      return;
    }
    final from = _from;
    final to = _to;
    setState(() {
      _from = null;
      _to = null;
    });
    if (from == null || to == null) return;
    // A drag of a few pixels is a slip of the hand, not a shape.
    if ((to - from).distance < 4) return;

    // The layer's coordinates when one is selected, the composition's when
    // not — the same path either way, and only what it will belong to differs
    // (K-237): a mask on the layer, or a new shape layer at the top of the
    // composition.
    final space = _space;
    final path = shapePath(
      tool: widget.tool,
      from: space.ofScreen(from),
      to: space.ofScreen(to),
      square: HardwareKeyboard.instance.isShiftPressed,
    );
    final box = _target;
    if (box == null) {
      _commitShapeLayer(path);
    } else {
      _commit(box, path);
    }
  }

  void _onPanCancel() => setState(() {
        _from = null;
        _to = null;
        _handleFrom = null;
        _handleTo = null;
      });

  // --- The Pen --------------------------------------------------------------

  /// A plain click: place a corner, or close the path when it lands on the
  /// first point.
  void _onPenTap(TapUpDetails details) {
    // The path is built in the layer's coordinates when there is a layer, and
    // in the composition's when there is not — the same path either way, and
    // the difference is only which thing it will belong to (K-237).
    final space = _space;
    final at = space.ofScreen(details.localPosition);
    final start = _draft.first;
    if (start != null &&
        _draft.canClose &&
        withinClosingDistance(at, start, screenScale: space.screenScale)) {
      final path = _draft.vertices;
      setState(() => _draft = const PathDraft());
      final box = _target;
      if (box == null) {
        _commitShapeLayer(path);
      } else {
        _commit(box, path);
      }
      return;
    }
    setState(() => _draft = _draft.withCorner(at));
  }

  /// A click that turned into a drag: the vertex is placed where the press was
  /// and its handles are pulled out to the pointer.
  void _finishHandleDrag() {
    final from = _handleFrom;
    final to = _handleTo;
    setState(() {
      _handleFrom = null;
      _handleTo = null;
    });
    if (from == null || to == null) return;
    final space = _space;
    setState(() => _draft = _draft.withBezier(
          space.ofScreen(from),
          space.ofScreen(to),
          independent: HardwareKeyboard.instance.isAltPressed,
        ));
  }

  // --- Committing -----------------------------------------------------------

  /// A new shape layer holding this art, at the top of the composition — what a
  /// shape tool or the Pen does with nothing selected (K-237).
  ///
  /// The art takes the toolbar's fill, and its stroke when one has a width: the
  /// two swatches that had nothing to paint until there were shape layers.
  void _commitShapeLayer(List<BridgeVertex> path) {
    if (path.length < 2) return;
    final tools = widget.uiState.tools;
    final name = shapeMaskName(widget.tool);
    try {
      final layer = widget.comp.addShapeLayer(
        name: name,
        contents: [
          BridgeShapeItem(
            id: UuidValue.fromString(const Uuid().v4()),
            name: name,
            vertices: path,
            // Always closed: a shape tool draws a closed figure, and the Pen
            // commits only when its path has come back to its first point.
            closed: true,
            fill: tools.fillRgba,
            stroke: tools.strokeWidth > 0 ? tools.strokeRgba : null,
            strokeWidth: tools.strokeWidth,
            opacity: 100,
          ),
        ],
      );
      widget.uiState.setSelection([layer]);
      widget.onChanged();
    } catch (_) {
      widget.state.postNotice(l10n.couldNotAddShapeLayer, error: true);
    }
  }

  void _commit(LayerBox box, List<BridgeVertex> path) {
    if (path.length < 2) return;
    try {
      box.layer.addMask(
        mask: shapeMask(
          vertices: path,
          name: maskName(widget.tool, box.masks.length),
        ),
      );
      widget.onChanged();
    } catch (_) {
      // The layer went away, or the engine refused the path. Nothing on
      // screen: the same calm refusal every other tool gives.
    }
  }
}

/// The shape being dragged, and the polygon being built.
/// Where the art being drawn lives, and how to put it on screen.
///
/// A shape tool draws in the selected layer's coordinates when there is one and
/// in the composition's when there is not (K-237). Both are a pair of maps and
/// nothing else, so the preview takes the pair rather than a layer it might not
/// have — which is what used to leave the shape-layer half of the gesture with
/// no preview at all.
class ShapeSpace {
  final Offset Function(double x, double y) toScreen;
  final (double, double) Function(Offset at) ofScreen;

  /// How many screen pixels one of this space's pixels covers — what every
  /// fixed-screen-distance rule (the Pen's closing ring, K-232) divides by.
  final double screenScale;

  const ShapeSpace({
    required this.toScreen,
    required this.ofScreen,
    this.screenScale = 1,
  });

  /// The selected layer's own coordinates, through its map.
  factory ShapeSpace.ofLayer(LayerBox box) => ShapeSpace(
        toScreen: box.map.toScreen,
        ofScreen: (at) {
          final p = box.map.layerOf(at);
          return (p.dx, p.dy);
        },
        screenScale: box.map.viewScale * box.map.sx,
      );

  /// The composition's own placement — the space a new shape layer's art is
  /// built in when there is no layer to ask (K-237). The same conversion the
  /// Type tool places a click with, so the two cannot drift apart.
  factory ShapeSpace.ofComp({required Rect fitted, required Size compSize}) {
    final scale = compSize.width == 0 ? 1.0 : fitted.width / compSize.width;
    return ShapeSpace(
      toScreen: (x, y) =>
          Offset(fitted.left + x * scale, fitted.top + y * scale),
      ofScreen: (at) =>
          ((at.dx - fitted.left) / scale, (at.dy - fitted.top) / scale),
      screenScale: scale,
    );
  }
}

class ShapePreviewPainter extends CustomPainter {
  final ToolMode tool;
  final ShapeSpace space;

  /// The fill and stroke the art would be committed with, so the preview shows
  /// the shape rather than only its outline (K-238). Translucent, because it is
  /// a shape that does not exist yet.
  final Color fill;
  final Color? stroke;
  final double strokeWidth;

  final Offset? from;
  final Offset? to;
  final bool square;
  final PathDraft draft;
  final Offset? penPointer;
  final Offset? handleFrom;
  final Offset? handleTo;

  /// Whether a click where the pointer is would close the path (K-232).
  final bool closing;
  final Color accent;

  const ShapePreviewPainter({
    required this.tool,
    required this.space,
    required this.fill,
    required this.stroke,
    required this.strokeWidth,
    required this.from,
    required this.to,
    required this.square,
    required this.draft,
    required this.penPointer,
    required this.handleFrom,
    required this.handleTo,
    required this.closing,
    required this.accent,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final outline = Paint()
      ..color = accent
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;

    // The dragged shape, drawn as the real path rather than as its bounding
    // box: an ellipse being dragged should look like an ellipse.
    if (from != null && to != null) {
      final path = shapePath(
        tool: tool,
        from: space.ofScreen(from!),
        to: space.ofScreen(to!),
        square: square,
      );
      if (path.isNotEmpty) {
        final screen = _screenPath(path, closed: true);
        _paintPreview(canvas, screen);
        canvas.drawPath(screen, outline);
      }
    }

    if (draft.isEmpty && handleFrom == null) return;

    if (draft.vertices.isNotEmpty) {
      // A path still being built is previewed filled too, so what the Pen is
      // making looks like what it will make. Closed for the fill even though
      // the path is open — an unclosed run of points has no inside otherwise,
      // and the shape it commits to will be closed.
      if (draft.vertices.length > 2) {
        _paintPreview(canvas, _screenPath(draft.vertices, closed: true));
      }
      canvas.drawPath(
        _screenPath(draft.vertices, closed: false),
        outline,
      );
      // Every placed vertex, and the first one larger: it is the one a click
      // has to land on to close the shape.
      for (var i = 0; i < draft.vertices.length; i++) {
        final v = draft.vertices[i];
        final at = space.toScreen(v.x, v.y);
        canvas.drawCircle(at, i == 0 ? 5 : 3, Paint()..color = accent);
      }
      // Near enough to close: the first vertex grows a ring, and the pointer
      // wears one too (K-232). Two marks rather than one, because the question
      // has two halves — *which* point closes the path, and whether the click
      // about to be made is that one.
      if (closing) {
        final ring = Paint()
          ..color = accent
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.5;
        final first = draft.vertices.first;
        canvas.drawCircle(
            space.toScreen(first.x, first.y), closingRingRadius, ring);
        final pointer = penPointer;
        if (pointer != null) {
          canvas.drawCircle(pointer, closingRingRadius * 0.6, ring);
        }
      }
      // The edge that would be drawn if the pointer clicked now — as the curve
      // it would actually be, not as a straight line (K-230).
      //
      // The last point placed may have handles pulled out of it, and those
      // handles bend the edge *leaving* it. Drawing that edge straight promised
      // one shape and delivered another the moment the next point landed. The
      // curve is the same cubic the committed path uses, with the pointer
      // standing in for a vertex that has no handles yet.
      //
      // **While the next vertex's own handles are being pulled out** (K-232)
      // the edge stops being a guess: the vertex is already placed — it is
      // where the press landed — so the curve runs to *there*, and bends into
      // it by the handle facing back along the path, which is the mirror of the
      // one under the pointer. It is the shape that will exist the moment the
      // button comes up, drawn as it is being aimed rather than after.
      final landing = handleFrom ?? penPointer;
      if (landing != null) {
        final last = draft.vertices.last;
        final from = space.toScreen(last.x, last.y);
        final out =
            space.toScreen(last.x + last.tanOutX, last.y + last.tanOutY);
        // The new vertex's *in* handle. A vertex with no handles yet — an
        // ordinary hover — has none, so the curve runs straight into it.
        final into = _incomingHandle(landing);
        canvas.drawPath(
          Path()
            ..moveTo(from.dx, from.dy)
            ..cubicTo(out.dx, out.dy, into.dx, into.dy, landing.dx, landing.dy),
          Paint()
            ..color = accent.withValues(alpha: 0.5)
            ..style = PaintingStyle.stroke
            ..strokeWidth = 1,
        );
      }
    }

    // The handles being pulled out of a vertex, with their mirror — the same
    // pair of arms the graph editor draws on a keyframe.
    final hFrom = handleFrom;
    final hTo = handleTo;
    if (hFrom != null && hTo != null) {
      final mirrored = hFrom * 2 - hTo;
      canvas.drawLine(hFrom, hTo, outline);
      if (!HardwareKeyboard.instance.isAltPressed) {
        canvas.drawLine(hFrom, mirrored, outline);
        canvas.drawCircle(mirrored, 3, Paint()..color = accent);
      }
      canvas.drawCircle(hTo, 3, Paint()..color = accent);
      canvas.drawCircle(hFrom, 4, Paint()..color = accent);
    }
  }

  /// The shape as it would be committed — the fill, then the stroke — under the
  /// accent outline that says it is still being drawn (K-238).
  ///
  /// **Translucent on purpose.** A solid preview is indistinguishable from a
  /// shape that already exists, and this one does not: nothing is in the
  /// document until the drag ends. Half opacity reads as "this is what you are
  /// about to get" while still showing the colour, which is the thing the fill
  /// swatch could not answer before.
  void _paintPreview(Canvas canvas, Path path) {
    canvas.drawPath(
      path,
      Paint()
        ..color = fill.withValues(alpha: fill.a * previewOpacity)
        ..style = PaintingStyle.fill,
    );
    final edge = stroke;
    if (edge == null || strokeWidth <= 0) return;
    canvas.drawPath(
      path,
      Paint()
        ..color = edge.withValues(alpha: edge.a * previewOpacity)
        ..style = PaintingStyle.stroke
        ..strokeWidth = strokeWidth
        ..strokeJoin = StrokeJoin.round,
    );
  }

  /// The control point the edge *arrives* at [landing] through.
  ///
  /// While a vertex's handles are being dragged out, the one that faces back
  /// along the path is the mirror of the one under the pointer — unless Alt has
  /// broken the pair, in which case the incoming side keeps where it was, which
  /// is the vertex itself. With no drag in flight there are no handles yet and
  /// the edge arrives straight.
  Offset _incomingHandle(Offset landing) {
    final hTo = handleTo;
    if (handleFrom == null || hTo == null) return landing;
    if (HardwareKeyboard.instance.isAltPressed) return landing;
    return landing * 2 - hTo;
  }

  /// A mask path in layer space, as a screen path — cubics between each pair of
  /// vertices, using their facing handles, which is exactly how the engine
  /// reads the same numbers. The cubic walk itself is [bezierPath], shared
  /// with the gizmo's outlines.
  Path _screenPath(List<BridgeVertex> vertices, {required bool closed}) =>
      bezierPath(
        count: vertices.length,
        at: (i) => space.toScreen(vertices[i].x, vertices[i].y),
        tangentOut: (i) => space.toScreen(vertices[i].x + vertices[i].tanOutX,
            vertices[i].y + vertices[i].tanOutY),
        tangentIn: (i) => space.toScreen(vertices[i].x + vertices[i].tanInX,
            vertices[i].y + vertices[i].tanInY),
        closed: closed,
      );

  @override
  bool shouldRepaint(ShapePreviewPainter old) => true;
}
