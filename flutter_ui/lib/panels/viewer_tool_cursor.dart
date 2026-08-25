// The drawn pointers the drawing and painting tools wear over the picture
// (K-226, docs/07 §2.3.3).
//
// **In plain terms.** A tool should say what it is without you looking away
// from the picture, and no operating system ships a "rectangle tool" pointer.
// So the tools that draw wear the same crosshair the eyedropper does — the
// pointer that means *this exact pixel* — with the tool's own icon tucked just
// down and to the right of it, the way After Effects badges its pointers. The
// crosshair is where the shape starts; the badge only says which shape.
//
// **The painting tools are different, and rightly.** A brush is not a point,
// it is a *width*, so its pointer is a circle the size of the stroke it would
// leave — the one thing a painter needs to see before pressing. The badge under
// it says brush, clone stamp or eraser.
//
// **Why they are drawn and not chosen.** A system cursor is a small fixed
// picture from a list the platform ships, and none of these are on it. Drawing
// them means hiding the system pointer over the picture and painting our own —
// the same thing the Rotation, Anchor point and Razor tools already do.
//
// Everything here is a widget, not a canvas: the icons are the application's
// own [lumitIcon] set, and drawing one on a canvas would mean a second copy of
// every glyph.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/state/tools.dart';

import '../icons/icons.dart';

/// Where the pointer is, for a tool that draws its own, and the hidden system
/// cursor that goes with it (K-230).
///
/// **Why a `Listener` and not the `MouseRegion` alone.** A `MouseRegion` reports
/// *hover*, and hovering stops the instant any mouse button is held — including
/// the secondary one, which none of these tools do anything with. Taken from
/// hover alone, a drawn pointer freezes where the press landed and stays frozen
/// until the button comes up or the pointer leaves the panel: a right-click over
/// the picture pinned the hand, the magnifier and the rest in place.
/// `onPointerMove` fires whichever button is down, so the drawn pointer keeps
/// following the real one. Nothing here handles a *gesture*; this only says
/// where to draw.
///
/// [onPointer] is given the position in this widget's own coordinates, and null
/// when the pointer has left — which is what a drawn pointer should draw
/// nothing for.
class DrawnPointerRegion extends StatefulWidget {
  final ValueChanged<Offset?> onPointer;

  /// The system cursor underneath. Hidden for everything that draws its own,
  /// but the Type tool's horizontal member keeps the platform's I-beam.
  final MouseCursor cursor;

  final Widget child;

  const DrawnPointerRegion({
    super.key,
    required this.onPointer,
    required this.child,
    this.cursor = SystemMouseCursors.none,
  });

  @override
  State<DrawnPointerRegion> createState() => _DrawnPointerRegionState();
}

class _DrawnPointerRegionState extends State<DrawnPointerRegion> {
  /// Which mouse the events are arriving from, for the cursor request below.
  int? _device;

  /// Whether the pointer is over this region at all — there is nothing to hide
  /// when it is not.
  bool _inside = false;

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

  /// Every key, because the point is not *which* key it was (K-235).
  ///
  /// Alt is the one that does this on Windows — it is the key reserved for the
  /// window menu, and pressing it takes the pointer's own state with it — but a
  /// tool has no business enumerating the keys a platform might reserve. Any
  /// key that brought the arrow back is a key worth hiding it after.
  ///
  /// Never consumed: this only watches.
  bool _onKey(KeyEvent event) {
    _hideSystemCursorAgain();
    return false;
  }

  /// Ask the platform to hide the pointer again (K-235).
  ///
  /// The arrow comes back and sits beside the drawn pointer, which is two
  /// pointers — exactly what hiding the system one is for. Flutter will not
  /// re-apply a cursor by itself here, because it only does so when the answer
  /// *changes*, and "hidden" to "hidden" is no change at all.
  ///
  /// So the same request Flutter's own cursor manager makes is made directly,
  /// for the device the pointer events are arriving from. Nothing in the widget
  /// tree moves — giving the region a new identity to force the question
  /// instead rebuilds the gesture detector under it and drops any drag in
  /// flight.
  void _hideSystemCursorAgain() {
    if (widget.cursor != SystemMouseCursors.none) return;
    final device = _device;
    if (device == null || !_inside) return;
    SystemChannels.mouseCursor.invokeMethod<void>(
      'activateSystemCursor',
      <String, dynamic>{'device': device, 'kind': 'none'},
    );
  }

  void _at(PointerEvent event) {
    _device = event.device;
    widget.onPointer(event.localPosition);
  }

  @override
  Widget build(BuildContext context) => MouseRegion(
        cursor: widget.cursor,
        // The enter is the `MouseRegion`'s alone: it fires when the panel
        // appears under a pointer that is not moving, which no move event would.
        onEnter: (event) {
          _inside = true;
          _at(event);
        },
        onExit: (_) {
          _inside = false;
          widget.onPointer(null);
        },
        child: Listener(
          onPointerHover: _at,
          onPointerMove: _at,
          child: widget.child,
        ),
      );
}

/// One figure in two passes — a thick stroke in the outline colour, then the
/// mark over it — the trick every drawn pointer here uses to stay legible over
/// a black picture and a white one alike. [draw] is called once per pass with
/// the pass's paint.
void paintTwoPassStroke(
  Color outline,
  Color mark,
  void Function(Paint paint) draw, {
  double outlineWidth = 3.0,
  double markWidth = 1.0,
  bool rounded = false,
}) {
  for (final (colour, width) in [(outline, outlineWidth), (mark, markWidth)]) {
    final paint = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = width;
    if (rounded) {
      paint
        ..strokeCap = StrokeCap.round
        ..strokeJoin = StrokeJoin.round;
    }
    draw(paint);
  }
}

/// The sweep rectangle every marquee draws — a faint fill under a hairline
/// edge — shared so "I am sweeping an area" reads the same whichever tool is
/// in hand.
void paintMarquee(Canvas canvas, Rect rect, Color accent) {
  canvas.drawRect(rect, Paint()..color = accent.withValues(alpha: 0.12));
  canvas.drawRect(
    rect,
    Paint()
      ..color = accent
      ..strokeWidth = 1
      ..style = PaintingStyle.stroke,
  );
}

/// The anchor point's mark — a small ring with a cross through it, the same
/// figure the anchor-point tool's icon carries — shared by the gizmo's pivot
/// handle and the Anchor point tool so the two read as one idea.
void paintAnchorMark(Canvas canvas, Offset at, Color colour,
    {double reach = 8}) {
  final paint = Paint()
    ..color = colour
    ..style = PaintingStyle.stroke
    ..strokeWidth = 1;
  canvas.drawCircle(at, 4, paint);
  canvas.drawLine(at - Offset(reach, 0), at + Offset(reach, 0), paint);
  canvas.drawLine(at - Offset(0, reach), at + Offset(0, reach), paint);
}

/// How far the tool's badge sits from the pointer, and how big it is drawn.
///
/// Down and to the right, out of the way of what is being drawn: a badge above
/// or to the left would sit on the shape the user is dragging out.
const Offset toolBadgeOffset = Offset(7, 7);
const double toolBadgeSize = 13;

/// How long each arm of the drawn crosshair is, in screen pixels.
const double toolCrosshairReach = 8;

/// The smallest and largest a brush ring is drawn at, whatever the width says.
///
/// A one-pixel brush would otherwise have an invisible pointer, and a very wide
/// one would fill the picture — the ring is a pointer, not the stroke itself.
const double minBrushRingRadius = 3;
const double maxBrushRingRadius = 200;

/// The drawn pointer for a tool that draws: a crosshair, or a brush ring, with
/// the tool's icon badged beside it.
///
/// [at] is in the same coordinates as the layer this is placed in — panel-local
/// for every caller here. A null [at] draws nothing, which is what a pointer
/// that has left the picture should do.
class ToolPointer extends StatelessWidget {
  final Offset? at;
  final ToolMode tool;

  /// The ink and the halo behind it, so the pointer is legible on a white
  /// picture and on a black one alike.
  final Color mark;
  final Color outline;

  /// The radius of the ring, for the painting tools. Null draws a crosshair.
  final double? ringRadius;

  const ToolPointer({
    super.key,
    required this.at,
    required this.tool,
    required this.mark,
    required this.outline,
    this.ringRadius,
  });

  @override
  Widget build(BuildContext context) {
    final at = this.at;
    if (at == null) return const SizedBox.shrink();
    return Positioned.fill(
      // Its own layer, so moving the pointer repaints the pointer and not the
      // picture or the shape being dragged out under it (K-233).
      child: RepaintBoundary(
        child: IgnorePointer(
          child: Stack(
            children: [
              Positioned.fill(
                child: CustomPaint(
                  painter: _ToolPointerPainter(
                    at: at,
                    mark: mark,
                    outline: outline,
                    ringRadius: ringRadius,
                  ),
                ),
              ),
              Positioned(
                left: at.dx + toolBadgeOffset.dx,
                top: at.dy + toolBadgeOffset.dy,
                // The icon twice: the halo copy a pixel down and across, then the
                // ink one over it. Cheaper than an outlined glyph and legible on
                // any picture, which is the whole requirement.
                child: Stack(
                  children: [
                    Transform.translate(
                      offset: const Offset(1, 1),
                      child: lumitIcon(tool.icon,
                          size: toolBadgeSize, color: outline),
                    ),
                    lumitIcon(tool.icon, size: toolBadgeSize, color: mark),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _ToolPointerPainter extends CustomPainter {
  final Offset at;
  final Color mark;
  final Color outline;
  final double? ringRadius;

  const _ToolPointerPainter({
    required this.at,
    required this.mark,
    required this.outline,
    required this.ringRadius,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final radius = ringRadius;
    if (radius != null) {
      paintTwoPassStroke(
          outline, mark, (paint) => canvas.drawCircle(at, radius, paint));
      // A dot at the centre: a wide ring alone leaves the actual point of the
      // brush unmarked, and a stroke starts at a point.
      canvas.drawCircle(at, 1, Paint()..color = mark);
      return;
    }

    paintTwoPassStroke(outline, mark, (paint) {
      canvas.drawLine(at - const Offset(toolCrosshairReach, 0),
          at + const Offset(toolCrosshairReach, 0), paint);
      canvas.drawLine(at - const Offset(0, toolCrosshairReach),
          at + const Offset(0, toolCrosshairReach), paint);
    });
  }

  @override
  bool shouldRepaint(_ToolPointerPainter old) =>
      old.at != at ||
      old.mark != mark ||
      old.outline != outline ||
      old.ringRadius != ringRadius;
}

/// The ring a brush of [width] layer pixels draws at this magnification, kept
/// within sight either way.
///
/// The painting tools are disabled on this branch (K-228) — the engine has no
/// paint strokes — so nothing calls this yet; it is the pointer they wear the
/// moment they do, and it is tested so it will be right when they arrive.
double brushRingRadius(double width, double viewScale) =>
    (width * viewScale / 2).clamp(minBrushRingRadius, maxBrushRingRadius);

/// The Hand tool's pointer: an open hand, and a closed one while it drags
/// (K-230).
///
/// **Why this is drawn.** Flutter can only ask for the pointers the platform
/// ships, and Windows ships no hand-with-fingers at all — `grab` and `grabbing`
/// are in Flutter's own list but not in the Windows embedder's, where anything
/// unknown quietly becomes the ordinary arrow. That is what the Hand tool was
/// showing: nothing. Drawing it is the only way to have it, and it buys the
/// closing hand as well, which is the half that says the pan has hold of the
/// picture.
class HandPointer extends StatelessWidget {
  final Offset? at;

  /// Whether the hand is holding: a drag in flight.
  final bool holding;
  final Color mark;
  final Color outline;

  const HandPointer({
    super.key,
    required this.at,
    required this.holding,
    required this.mark,
    required this.outline,
  });

  @override
  Widget build(BuildContext context) {
    final at = this.at;
    if (at == null) return const SizedBox.shrink();
    return Positioned.fill(
      // Its own layer, so moving the pointer repaints the pointer and not the
      // picture, the wireframes or the tool's own preview under it (K-233).
      child: RepaintBoundary(
        child: IgnorePointer(
          child: CustomPaint(
            painter: _HandPainter(
              at: at,
              holding: holding,
              mark: mark,
              outline: outline,
            ),
          ),
        ),
      ),
    );
  }
}

/// The hand itself, on a 24-unit grid centred on the pointer.
///
/// Two passes as every drawn pointer here does — a thick outline stroke, then
/// the mark over it — so it is legible over a black picture and a white one.
class _HandPainter extends CustomPainter {
  final Offset at;
  final bool holding;
  final Color mark;
  final Color outline;

  const _HandPainter({
    required this.at,
    required this.holding,
    required this.mark,
    required this.outline,
  });

  /// How tall the drawn hand is, in screen pixels.
  static const double _size = 20;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.save();
    canvas.translate(at.dx, at.dy);
    final s = _size / 24;
    canvas.scale(s, s);
    // The palm's middle sits on the pointer, which is where a hand grips.
    canvas.translate(-12, -12);
    final path = holding ? _fist() : _openHand();
    canvas.drawPath(
      path,
      Paint()
        ..color = outline
        ..style = PaintingStyle.stroke
        ..strokeWidth = 4.5
        ..strokeJoin = StrokeJoin.round
        ..strokeCap = StrokeCap.round,
    );
    canvas.drawPath(path, Paint()..color = mark);
    canvas.restore();
  }

  /// An open hand: palm, four fingers standing, thumb out to the left.
  Path _openHand() => Path()
    ..moveTo(6, 14)
    ..lineTo(6, 9)
    ..lineTo(8, 9)
    ..lineTo(8, 4)
    ..lineTo(10, 4)
    ..lineTo(10, 9)
    ..lineTo(12, 9)
    ..lineTo(12, 3)
    ..lineTo(14, 3)
    ..lineTo(14, 9)
    ..lineTo(16, 9)
    ..lineTo(16, 5)
    ..lineTo(18, 5)
    ..lineTo(18, 15)
    ..cubicTo(18, 19, 15, 21, 12, 21)
    ..cubicTo(9, 21, 6, 19, 6, 15)
    ..close();

  /// The same hand closed: the fingers curled down onto the palm, with the
  /// knuckles as the line across the top.
  Path _fist() => Path()
    ..moveTo(6, 13)
    ..cubicTo(6, 10, 8, 9, 10, 9)
    ..lineTo(16, 9)
    ..cubicTo(17, 9, 18, 10, 18, 11)
    ..lineTo(18, 15)
    ..cubicTo(18, 19, 15, 21, 12, 21)
    ..cubicTo(9, 21, 6, 19, 6, 15)
    ..close();

  @override
  bool shouldRepaint(_HandPainter old) =>
      old.at != at ||
      old.holding != holding ||
      old.mark != mark ||
      old.outline != outline;
}

/// The Hand tool over the picture: the drawn hand, and the drag that pans.
///
/// It takes the drag itself rather than letting it fall through to the panel,
/// so the pan is this layer's own gesture. Where the hand is *drawn* comes from
/// [DrawnPointerRegion], which is what keeps it following the pointer while a
/// button — any button — is held.
class ViewerHandLayer extends StatefulWidget {
  /// Whether the Hand tool is armed. Inert otherwise — no pointer taken, no
  /// system cursor hidden.
  final bool active;

  /// How far the picture should move, per pointer movement.
  final ValueChanged<Offset> onPan;

  final Color mark;
  final Color outline;

  const ViewerHandLayer({
    super.key,
    required this.active,
    required this.onPan,
    required this.mark,
    required this.outline,
  });

  @override
  State<ViewerHandLayer> createState() => _ViewerHandLayerState();
}

class _ViewerHandLayerState extends State<ViewerHandLayer> {
  Offset? _pointer;
  bool _holding = false;

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return const SizedBox.shrink();
    return Positioned.fill(
      // The system pointer is hidden, because the hand below replaces it: an
      // arrow sitting inside the drawn hand would read as two pointers (K-219's
      // rule).
      child: DrawnPointerRegion(
        onPointer: (at) => setState(() => _pointer = at),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onPanStart: (d) => setState(() {
            _holding = true;
            _pointer = d.localPosition;
          }),
          onPanUpdate: (d) {
            setState(() => _pointer = d.localPosition);
            widget.onPan(d.delta);
          },
          onPanEnd: (_) => setState(() => _holding = false),
          onPanCancel: () => setState(() => _holding = false),
          child: Stack(children: [
            const Positioned.fill(child: SizedBox.expand()),
            HandPointer(
              at: _pointer,
              holding: _holding,
              mark: widget.mark,
              outline: widget.outline,
            ),
          ]),
        ),
      ),
    );
  }
}

/// The Zoom tool's pointer: a magnifier with a plus in it, or a minus while Alt
/// says the click will zoom out (K-230).
///
/// Drawn for the same reason the hand is: Flutter's `zoomIn`/`zoomOut` are not
/// in the Windows embedder's list of pointers, so asking for one got the plain
/// arrow — a Zoom tool that looked exactly like no tool at all.
class MagnifierPointer extends StatelessWidget {
  final Offset? at;

  /// Whether the click would zoom out (the Alt modifier).
  final bool out;
  final Color mark;
  final Color outline;

  const MagnifierPointer({
    super.key,
    required this.at,
    required this.out,
    required this.mark,
    required this.outline,
  });

  @override
  Widget build(BuildContext context) {
    final at = this.at;
    if (at == null) return const SizedBox.shrink();
    return Positioned.fill(
      child: RepaintBoundary(
        child: IgnorePointer(
          child: CustomPaint(
            painter: _MagnifierPainter(
              at: at,
              out: out,
              mark: mark,
              outline: outline,
            ),
          ),
        ),
      ),
    );
  }
}

class _MagnifierPainter extends CustomPainter {
  final Offset at;
  final bool out;
  final Color mark;
  final Color outline;

  const _MagnifierPainter({
    required this.at,
    required this.out,
    required this.mark,
    required this.outline,
  });

  /// The lens' radius in screen pixels, and how far the handle runs past it.
  static const double _lens = 6.5;
  static const double _handle = 7;

  @override
  void paint(Canvas canvas, Size size) {
    // The lens sits *on* the pointer: what a magnification is anchored to is
    // the point in the middle of the glass, and that has to be the point the
    // pointer claims (docs/07 §2.2 — the comp point under the cursor stays
    // under the cursor).
    canvas.save();
    canvas.translate(at.dx, at.dy);
    const grip = 0.7071 * _lens;
    paintTwoPassStroke(outline, mark, (paint) {
      canvas.drawCircle(Offset.zero, _lens, paint);
      canvas.drawLine(
        const Offset(grip, grip),
        const Offset(grip + _handle * 0.7071, grip + _handle * 0.7071),
        paint,
      );
      // The sign inside the glass: plus in, minus out. The bar across is drawn
      // for both, so the two pointers differ by one stroke and read as one
      // family.
      const arm = 3.0;
      canvas.drawLine(const Offset(-arm, 0), const Offset(arm, 0), paint);
      if (!out) {
        canvas.drawLine(const Offset(0, -arm), const Offset(0, arm), paint);
      }
    }, outlineWidth: 3.4, markWidth: 1.6, rounded: true);
    canvas.restore();
  }

  @override
  bool shouldRepaint(_MagnifierPainter old) =>
      old.at != at ||
      old.out != out ||
      old.mark != mark ||
      old.outline != outline;
}

/// The text pointer, for the Type tool's vertical member (K-226).
///
/// Horizontal type wears the system's own I-beam — every platform has one and
/// it is the pointer everybody already reads as "you can type here". Nobody
/// ships a *sideways* one, so vertical type gets this: the same beam, turned a
/// quarter turn, so the pointer says which way the line will run.
class TextPointer extends StatelessWidget {
  final Offset? at;
  final Color mark;
  final Color outline;

  const TextPointer({
    super.key,
    required this.at,
    required this.mark,
    required this.outline,
  });

  @override
  Widget build(BuildContext context) {
    final at = this.at;
    if (at == null) return const SizedBox.shrink();
    return Positioned.fill(
      child: RepaintBoundary(
        child: IgnorePointer(
          child: CustomPaint(
            painter: _BeamPainter(at: at, mark: mark, outline: outline),
          ),
        ),
      ),
    );
  }
}

/// An I-beam lying on its side: the bar runs across, its serifs stand up.
class _BeamPainter extends CustomPainter {
  final Offset at;
  final Color mark;
  final Color outline;

  const _BeamPainter(
      {required this.at, required this.mark, required this.outline});

  @override
  void paint(Canvas canvas, Size size) {
    const reach = 7.0;
    const serif = 3.0;
    paintTwoPassStroke(outline, mark, (paint) {
      canvas.drawLine(
          at - const Offset(reach, 0), at + const Offset(reach, 0), paint);
      for (final end in [-reach, reach]) {
        canvas.drawLine(
          at + Offset(end, -serif),
          at + Offset(end, serif),
          paint,
        );
      }
    });
  }

  @override
  bool shouldRepaint(_BeamPainter old) =>
      old.at != at || old.mark != mark || old.outline != outline;
}
