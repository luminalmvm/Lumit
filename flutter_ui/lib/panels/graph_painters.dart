// The graph editor's painters: the grid and the curves, a key's glyph, and a
// tangent endpoint's ring. Split out of graph_editor_frb.dart, which
// re-exports them. The handles painter stays with the pane — it reads the
// state's private geometry.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import '../widgets/controls.dart';

// ---------------------------------------------------------------------------
// Painters.
// ---------------------------------------------------------------------------

/// A keyframe's glyph, coded by interpolation: diamond for linear, circle for
/// an eased (bezier) key, square for hold — the same coding the lanes will
/// learn (docs/07 §4.3). On the speed lens every dot is a circle.
class KeyGlyphPainter extends CustomPainter {
  final BridgeKeyframe key_;
  final Color colour;
  final bool speedDot;
  const KeyGlyphPainter({
    required this.key_,
    required this.colour,
    required this.speedDot,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()..color = colour;
    final half = size.width / 2;
    // An automatic side is an eased side: it has a tangent, so its key is a
    // circle like any other eased one.
    if (speedDot ||
        key_.interpIn is BridgeSideInterp_Bezier ||
        key_.interpOut is BridgeSideInterp_Bezier ||
        key_.interpIn is BridgeSideInterp_Auto ||
        key_.interpOut is BridgeSideInterp_Auto) {
      canvas.drawCircle(Offset(half, half), half - 1, paint);
      return;
    }
    if (key_.interpOut is BridgeSideInterp_Hold) {
      canvas.drawRect(
          Rect.fromLTWH(1, 1, size.width - 2, size.height - 2), paint);
      return;
    }
    canvas.drawPath(
      Path()
        ..moveTo(half, 0)
        ..lineTo(size.width, half)
        ..lineTo(half, size.height)
        ..lineTo(0, half)
        ..close(),
      paint,
    );
  }

  @override
  bool shouldRepaint(KeyGlyphPainter old) =>
      old.colour != colour ||
      old.speedDot != speedDot ||
      old.key_.interpIn != key_.interpIn ||
      old.key_.interpOut != key_.interpOut;
}

/// A tangent endpoint's dot: the drawing's **hollow ring** — a `text_primary`
/// stroke round a hole punched in the pane's own ground, so the curve running
/// under it does not read as running through it.
class HandleDotPainter extends CustomPainter {
  final Color colour;
  final Color fill;

  /// The pointer is over the ring's target: the stroke comes up to full
  /// strength, and goes back down when it leaves (P1 — nothing at rest).
  final bool hovered;

  const HandleDotPainter(
      {required this.colour, required this.fill, this.hovered = false});

  @override
  void paint(Canvas canvas, Size size) {
    final centre = Offset(size.width / 2, size.height / 2);
    final r = size.width / 2 - 1;
    canvas.drawCircle(centre, r, Paint()..color = fill);
    canvas.drawCircle(
      centre,
      r,
      Paint()
        ..color = hovered ? colour : colour.withValues(alpha: 0.8)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(HandleDotPainter old) =>
      old.colour != colour || old.fill != fill || old.hovered != hovered;
}

/// A tangent handle's dot with its own hover state, so brightening one ring
/// repaints that ring rather than rebuilding the pane under the pointer.
class HandleRing extends StatefulWidget {
  final double size;
  const HandleRing({super.key, required this.size});

  @override
  State<HandleRing> createState() => HandleRingState();
}

class HandleRingState extends State<HandleRing> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return MouseRegion(
      // A handle swings: the drag that matters is the vertical one, and the
      // cursor says so before the button goes down (P2).
      cursor: SystemMouseCursors.resizeUpDown,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: SizedBox(
        width: widget.size,
        height: widget.size,
        child: Center(
          child: SizedBox(
            width: 8,
            height: 8,
            child: CustomPaint(
              painter: HandleDotPainter(
                colour: t.textPrimary,
                fill: t.surface0,
                hovered: _hovered,
              ),
            ),
          ),
        ),
      ),
    );
  }
}
