// The painting tools over the picture: brush, eraser and clone stamp
// (K-227, docs/07 §2.3.4).
//
// **In plain terms.** With a painting tool in hand, dragging over the picture
// leaves a mark on the **selected layer**. The brush lays down the toolbar's
// fill colour; the eraser rubs the layer through to transparent; the clone stamp
// copies from somewhere else on the same layer, which is how a boom or a
// blemish gets painted out. The ring on the pointer is the size of the mark
// before you make it.
//
// **What is stored is the gesture, not the pixels.** A stroke crosses the bridge
// as the path the pointer took in the layer's own coordinates, plus its colour,
// width, hardness and opacity — so it is re-stamped at whatever resolution the
// frame is being rendered at, and every setting stays changeable afterwards. The
// engine's side of this is `lumit_core::paint`.
//
// **A stylus presses.** On a pen tablet each point also carries how hard it was
// pressed, and the brush's "pressure controls size" toggle decides whether that
// widens the mark (K-583). With a mouse — or with the toggle off — every point
// is a full press, which the engine stores as no pressures at all, so nothing
// about the stroke changes.
//
// **One drag is one stroke is one undo step.** The stroke is drawn on the
// overlay while the pointer is down and committed once on release, exactly as
// the type tool commits a document once. `Escape` abandons a stroke in flight;
// `Backspace` takes the last committed one back.
//
// **The clone stamp needs a source first.** `Alt`-click sets the point copied
// *from*, marked on the picture; painting then copies from that offset. Without
// one, the tool says so instead of stamping nothing — After Effects' rule and
// the same honesty every other unbuilt path here follows.

import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart' show BridgeScalar;
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import 'viewer_gizmo.dart';
import 'viewer_tool_cursor.dart';

/// How far the pointer must travel, in screen pixels, before another point is
/// kept.
///
/// A stroke is a record of a gesture, not of every pointer event: a slow drag
/// across the picture can raise hundreds of samples a second, and a path with a
/// thousand points in it costs the renderer for no visible gain. Two pixels is
/// below what any brush edge can show.
const double paintSampleDistance = 2;

/// The mark drawn where a clone stamp will copy from.
const double cloneSourceMarkSize = 7;

/// What a brush shape is called in the tool options and on a stroke's row
/// (K-548).
String brushShapeLabel(BridgeBrushShape shape) => switch (shape) {
      BridgeBrushShape.round => l10n.brushShapeRound,
      BridgeBrushShape.square => l10n.brushShapeSquare,
    };

/// Which engine mode each painting tool commits.
BridgePaintMode paintModeFor(ToolMode tool) => switch (tool) {
      ToolMode.eraser => BridgePaintMode.erase,
      ToolMode.cloneStamp => BridgePaintMode.clone,
      _ => BridgePaintMode.paint,
    };

/// [points] with anything closer than [paintSampleDistance] to the point before
/// it dropped — the thinning every stroke goes through before it is committed.
///
/// The first and last points always survive: a stroke that stopped short of
/// where the pointer stopped is a stroke that does not go where it was drawn.
List<Offset> thinStroke(List<Offset> points,
        {double minimum = paintSampleDistance}) =>
    [for (final i in thinStrokeIndices(points, minimum: minimum)) points[i]];

/// Which of [points] [thinStroke] keeps, by index — the same thinning, said in
/// a way anything travelling *beside* the path can follow.
///
/// The stylus pressures are that beside-the-path list (K-583): a pressure has
/// to be dropped with the point it belongs to, and a second thinning that
/// happened to agree would be a second thinning that one day did not.
List<int> thinStrokeIndices(List<Offset> points,
    {double minimum = paintSampleDistance}) {
  if (points.length < 2) return [for (var i = 0; i < points.length; i++) i];
  final out = <int>[0];
  for (var i = 1; i < points.length; i++) {
    if ((points[i] - points[out.last]).distance >= minimum) out.add(i);
  }
  if (out.last != points.length - 1) out.add(points.length - 1);
  return out;
}

/// [points] as a polyline, ready to be stroked.
///
/// A single point is drawn as a line to itself: a path with one point in it
/// draws nothing, and a dab is a stroke of one point that still has to leave
/// its round cap on the picture.
Path strokePath(List<Offset> points) {
  final path = Path()..moveTo(points.first.dx, points.first.dy);
  for (final p in points.skip(1)) {
    path.lineTo(p.dx, p.dy);
  }
  if (points.length == 1) path.lineTo(points.first.dx, points.first.dy);
  return path;
}

/// How a mark is stroked: round cap, round join, and a width that cannot fall
/// below a hairline or run away past the canvas — the recipe every overlay that
/// draws a brushed stroke shares, so a paint stroke and a roto scribble look
/// like the same instrument.
Paint strokePaint(Color colour, double width) => Paint()
  ..color = colour
  ..style = PaintingStyle.stroke
  ..strokeWidth = width.clamp(1.0, 4000.0)
  ..strokeCap = StrokeCap.round
  ..strokeJoin = StrokeJoin.round;

/// How hard the stylus is pressing, 0..1, or **1.0 for anything that is not a
/// stylus** — a mouse, a finger, a tablet that reports no range. That is the
/// whole of "if no stylus is present nothing changes" (K-583): a full press
/// everywhere is stored as no pressures at all.
double stylusPressure(PointerEvent event) {
  if (event.kind != PointerDeviceKind.stylus &&
      event.kind != PointerDeviceKind.invertedStylus) {
    return 1.0;
  }
  final span = event.pressureMax - event.pressureMin;
  if (span <= 0) return 1.0;
  return ((event.pressure - event.pressureMin) / span).clamp(0.0, 1.0);
}

/// The painting tools over the picture.
class ViewerPaintLayer extends StatefulWidget {
  /// Whether a painting tool is armed. Inert otherwise.
  final bool active;

  final ToolMode tool;
  final LumitState state;
  final LumitUiState uiState;

  /// Every layer with its box, top first — for the layer being painted on and
  /// the map that turns the pointer into layer coordinates.
  final List<LayerBox> boxes;

  /// The picture's magnification, so a brush width in layer pixels draws the
  /// ring it would really leave.
  final double viewScale;

  final VoidCallback onChanged;

  const ViewerPaintLayer({
    super.key,
    required this.active,
    required this.tool,
    required this.state,
    required this.uiState,
    required this.boxes,
    required this.viewScale,
    required this.onChanged,
  });

  @override
  State<ViewerPaintLayer> createState() => _ViewerPaintLayerState();
}

class _ViewerPaintLayerState extends State<ViewerPaintLayer> {
  /// Where the pointer is, for the ring.
  Offset? _pointer;

  /// The stroke in flight, in screen coordinates.
  final List<Offset> _stroke = [];

  /// How hard the stylus was pressing at each of those points, 0..1 (K-583).
  /// Parallel to [_stroke], and all 1.0 when there is no stylus.
  final List<double> _pressures = [];

  /// The pressure of the last pointer event seen. The drag callbacks carry no
  /// pressure of their own, so it is read off the raw stream in the `Listener`
  /// wrapped round them — which sees each event before the gesture recogniser
  /// does, so what is read here is the press that made this point.
  double _pressure = 1.0;

  /// Where the press landed. The framework only reports a drag once it has
  /// travelled its slop, and a stroke that began 18px along is the wrong stroke
  /// (K-217's trap, and every tool since).
  Offset? _downAt;

  /// The clone stamp's source, in the *layer's* coordinates, so it stays put on
  /// the picture while the view is panned or zoomed.
  Offset? _cloneSource;

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_onKey);
    _escapeRelease = EscapeLadder.register(EscapeRung.gesture, _escape);
  }

  /// How to stand down from the ladder.
  VoidCallback? _escapeRelease;

  /// Escape abandons the stroke being drawn — the ladder's gesture rung
  /// (widgets/escape_ladder.dart), so it is taken back before a menu closes.
  bool _escape() {
    if (!widget.active || _stroke.isEmpty) return false;
    setState(_clearStroke);
    return true;
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    _escapeRelease?.call();
    _escapeRelease = null;
    super.dispose();
  }

  bool get _isClone => widget.tool == ToolMode.cloneStamp;

  /// The path and its pressures go together or not at all.
  void _clearStroke() {
    _stroke.clear();
    _pressures.clear();
  }

  /// The layer being painted on: the primary selection, as the shape tools use.
  LayerBox? get _target =>
      primarySelectedBox(widget.boxes, widget.uiState.selectedLayerIds);

  /// Backspace takes back the last stroke committed — the reason a bad stroke
  /// is never a trip to the Timeline. (Escape, which abandons the stroke still
  /// being drawn, is [_escape] on the ladder.)
  bool _onKey(KeyEvent event) {
    if (!widget.active || event is! KeyDownEvent) return false;
    if (event.logicalKey == LogicalKeyboardKey.backspace) {
      final box = _target;
      if (box == null) return false;
      try {
        box.layer.deleteLastStroke();
        widget.onChanged();
        return true;
      } catch (_) {
        // Nothing painted on it; the key belongs to whoever wants it next.
        return false;
      }
    }
    return false;
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return const SizedBox.shrink();
    final t = ThemeScope.of(context).theme;
    final tools = widget.uiState.tools;
    final box = _target;
    return Positioned.fill(
      // The hardware crosshair leads (K-724): the OS moves it at input rate
      // whatever the application's frame rate is doing, so it is the thing to
      // aim with; the ring below is decoration — the size of the mark, drawn
      // by the app and honestly a frame behind. Through the shared region
      // rather than a `MouseRegion` of its own, so the clone stamp's
      // `Alt`-click cannot swap the pointer under the ring (K-235) — the
      // fault the Zoom tool had, and this had for the same reason.
      child: DrawnPointerRegion(
        cursor: SystemMouseCursors.precise,
        onPointer: (at) => setState(() => _pointer = at),
        child: Listener(
          onPointerDown: (event) {
            _downAt = event.localPosition;
            _pressure = stylusPressure(event);
          },
          onPointerMove: (event) => _pressure = stylusPressure(event),
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTapUp: _onTapUp,
            onPanStart: _onPanStart,
            onPanUpdate: _onPanUpdate,
            onPanEnd: (_) => _onPanEnd(),
            onPanCancel: () => setState(_clearStroke),
            child: Stack(
              children: [
                Positioned.fill(
                  child: CustomPaint(
                    painter: _StrokePainter(
                      stroke: _stroke,
                      width: tools.brushSize * widget.viewScale,
                      colour: colourOf(tools.fill,
                          opacity: tools.brushOpacity / 100),
                      erasing: widget.tool == ToolMode.eraser,
                      hairline: t.hairlineStrong,
                      cloneSource: _isClone &&
                              box != null &&
                              _cloneSource != null
                          ? box.map.toScreen(_cloneSource!.dx, _cloneSource!.dy)
                          : null,
                      mark: t.textPrimary,
                      outline: t.surface0,
                    ),
                  ),
                ),
                ToolPointer(
                  at: _pointer,
                  tool: widget.tool,
                  mark: t.textPrimary,
                  outline: t.surface0,
                  ringRadius:
                      brushRingRadius(tools.brushSize, widget.viewScale),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  // --- The gesture ----------------------------------------------------------

  /// A click: for the clone stamp with `Alt`, that is how the source is set.
  /// Otherwise it is a single dab, which is a stroke of one point.
  void _onTapUp(TapUpDetails details) {
    final box = _target;
    if (box == null) {
      _sayNoLayer();
      return;
    }
    if (_isClone && HardwareKeyboard.instance.isAltPressed) {
      final at = box.map.layerOf(details.localPosition);
      setState(() => _cloneSource = at);
      widget.state.postNotice(l10n.cloneSourceSet);
      return;
    }
    _commit(box, [details.localPosition], [_pressure]);
  }

  void _onPanStart(DragStartDetails details) {
    final at = _downAt ?? details.localPosition;
    setState(() {
      _clearStroke();
      _stroke
        ..add(at)
        ..add(details.localPosition);
      _pressures
        ..add(_pressure)
        ..add(_pressure);
    });
  }

  void _onPanUpdate(DragUpdateDetails details) {
    setState(() {
      _stroke.add(details.localPosition);
      _pressures.add(_pressure);
    });
  }

  void _onPanEnd() {
    final points = List.of(_stroke);
    final pressures = List.of(_pressures);
    setState(_clearStroke);
    if (points.isEmpty) return;
    final box = _target;
    if (box == null) {
      _sayNoLayer();
      return;
    }
    _commit(box, points, pressures);
  }

  /// One stroke, one op, one undo step.
  void _commit(
      LayerBox box, List<Offset> screenPoints, List<double> pressures) {
    if (_isClone && _cloneSource == null) {
      widget.state.postNotice(
        l10n.cloneSourceFirst,
      );
      return;
    }
    final tools = widget.uiState.tools;
    // The pressures are thinned with the points they belong to, and a brush
    // with the toggle off commits a full press throughout — which the engine
    // stores as no pressures at all, so the stroke is the stroke it would have
    // been (K-583).
    final points = [
      for (final i in thinStrokeIndices(screenPoints))
        () {
          final layer = box.map.layerOf(screenPoints[i]);
          return BridgeStrokePoint(
            x: layer.dx,
            y: layer.dy,
            pressure: tools.brushPressureSize && i < pressures.length
                ? pressures[i]
                : 1.0,
          );
        }(),
    ];
    if (points.isEmpty) return;

    // The clone's offset is from the point being painted to the point copied
    // from, so the whole stroke keeps the relationship the first dab set — the
    // stamp everybody expects, rather than a fixed spot smeared about.
    var offsetX = 0.0;
    var offsetY = 0.0;
    if (_isClone) {
      offsetX = _cloneSource!.dx - points.first.x;
      offsetY = _cloneSource!.dy - points.first.y;
    }

    try {
      // Numbered by what the layer already carries, so the Timeline reads
      // "Brush 1, Brush 2" rather than a column of identical names.
      final number = box.layer.getPaint().length + 1;
      box.layer.addStroke(
        stroke: BridgeStroke(
          id: UuidValue.fromString(const Uuid().v4()),
          name: '${widget.tool.label} $number',
          points: points,
          colour: tools.fillRgba,
          width: tools.brushSize,
          hardness: tools.brushHardness / 100,
          shape: tools.brushShape,
          opacity: tools.brushOpacity,
          // A fresh stroke is drawn whole; Start and End are trimmed
          // afterwards, on their own Timeline rows (K-549).
          start: const BridgeScalar.static_(0),
          end: const BridgeScalar.static_(100),
          mode: paintModeFor(widget.tool),
          // A fresh mark lays its colour down; the blend is chosen after the
          // fact on the stroke's own row (K-550). Index 0 is Normal.
          blend: 0,
          cloneOffsetX: offsetX,
          cloneOffsetY: offsetY,
        ),
      );
      widget.onChanged();
    } catch (_) {
      // The layer went away mid-stroke, or the stroke had nothing in it after
      // thinning. Neither is worth a dialogue.
    }
  }

  void _sayNoLayer() => widget.state.postNotice(
        l10n.selectALayerToPaint,
      );
}

/// The stroke being drawn, and the clone stamp's source mark.
///
/// Drawn here rather than previewed through the engine: a stroke is a gesture
/// that has not happened yet, and the preview path patches *values* into a copy
/// of the document, not lists. The polyline is drawn the width the brush is, so
/// what you see under the pointer is the mark you are about to make.
class _StrokePainter extends CustomPainter {
  final List<Offset> stroke;
  final double width;
  final Color colour;
  final bool erasing;
  final Color hairline;
  final Offset? cloneSource;
  final Color mark;
  final Color outline;

  const _StrokePainter({
    required this.stroke,
    required this.width,
    required this.colour,
    required this.erasing,
    required this.hairline,
    required this.cloneSource,
    required this.mark,
    required this.outline,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final source = cloneSource;
    if (source != null) {
      // A cross, so the source reads as a *place* rather than as another
      // pointer.
      paintTwoPassStroke(outline, mark, (paint) {
        canvas.drawLine(source - const Offset(cloneSourceMarkSize, 0),
            source + const Offset(cloneSourceMarkSize, 0), paint);
        canvas.drawLine(source - const Offset(0, cloneSourceMarkSize),
            source + const Offset(0, cloneSourceMarkSize), paint);
      });
    }

    if (stroke.isEmpty) return;
    // An eraser has no colour to show, so it draws as an outline of where it
    // would rub: filling it in the theme's ink would read as painting.
    canvas.drawPath(
      strokePath(stroke),
      strokePaint(erasing ? hairline : colour, width),
    );
  }

  @override
  bool shouldRepaint(_StrokePainter old) =>
      old.stroke.length != stroke.length ||
      old.stroke.lastOrNull != stroke.lastOrNull ||
      old.width != width ||
      old.colour != colour ||
      old.erasing != erasing ||
      old.cloneSource != cloneSource;
}
