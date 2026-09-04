import 'package:flutter/material.dart';
import 'package:lumit_flutter/panels/viewer_tool_cursor.dart' show paintMarquee;

/// The Viewer's **region of interest** (docs/07 §2.2 item 7): the
/// sub-rectangle of the composition the engine composites, so working on one
/// corner of a heavy shot does not cost the whole frame.
///
/// Two jobs, and they are deliberately in one widget because they are two
/// halves of one idea. While [arming] is on, a drag sweeps out a new region.
/// Whether it is on or not, a region that exists is outlined, so it is never
/// possible to be looking at a corner of a shot without being told.
///
/// Rectangles cross this boundary as **fractions of the picture**, not pixels:
/// which pixel a point is depends on the raster the engine settles on, and it
/// settles on different ones at different preview resolutions. Fractions mean
/// the same thing at every one of them.
class ViewerRegionLayer extends StatefulWidget {
  /// Whether a drag should sweep out a new region. Off: this draws the
  /// existing region and lets every pointer through to what is beneath.
  final bool arming;

  /// Where the picture is on screen, so a screen point can be read as a
  /// fraction of the composition.
  final Rect fitted;

  /// The region in force, as `[u0, v0, u1, v1]`, or null for the whole frame.
  final List<double>? region;

  /// A swept region, in the same fractions. Null when the drag was too small
  /// to mean anything, which reads as "clear".
  final void Function(List<double>? region) onRegion;

  final Color accent;

  const ViewerRegionLayer({
    super.key,
    required this.arming,
    required this.fitted,
    required this.region,
    required this.onRegion,
    required this.accent,
  });

  @override
  State<ViewerRegionLayer> createState() => _ViewerRegionLayerState();
}

class _ViewerRegionLayerState extends State<ViewerRegionLayer> {
  Offset? _from;
  Offset? _to;

  Rect? get _dragged {
    final (from, to) = (_from, _to);
    if (from == null || to == null) return null;
    return Rect.fromPoints(from, to);
  }

  /// The screen rectangle a set region occupies — what gets outlined when
  /// nothing is being dragged.
  Rect? get _existing {
    final r = widget.region;
    if (r == null || r.length != 4) return null;
    final f = widget.fitted;
    return Rect.fromLTRB(
      f.left + r[0] * f.width,
      f.top + r[1] * f.height,
      f.left + r[2] * f.width,
      f.top + r[3] * f.height,
    );
  }

  /// A screen rectangle as fractions of the picture, clamped to it — a drag
  /// that runs off the edge of the frame means "to the edge", which is what
  /// anyone dragging off the edge intends.
  List<double> _fractions(Rect box) {
    final f = widget.fitted;
    double u(double x) => ((x - f.left) / f.width).clamp(0.0, 1.0);
    double v(double y) => ((y - f.top) / f.height).clamp(0.0, 1.0);
    return [u(box.left), v(box.top), u(box.right), v(box.bottom)];
  }

  @override
  Widget build(BuildContext context) {
    final outline = _dragged ?? _existing;
    final painter = Positioned.fill(
      child: IgnorePointer(
        child: CustomPaint(
          painter: _RegionPainter(box: outline, accent: widget.accent),
        ),
      ),
    );
    // Not arming: the outline only, with nothing taking pointers. A region in
    // force must not make the picture beneath it unclickable.
    if (!widget.arming) return painter;
    return Positioned.fill(
      child: MouseRegion(
        cursor: SystemMouseCursors.precise,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onPanStart: (d) => setState(() {
            _from = d.localPosition;
            _to = d.localPosition;
          }),
          onPanUpdate: (d) => setState(() => _to = d.localPosition),
          onPanEnd: (_) {
            final box = _dragged;
            setState(() {
              _from = null;
              _to = null;
            });
            // A drag of a few pixels is a click that wobbled. Reading it as a
            // region would leave someone looking at a postage stamp and
            // wondering where their shot went, so it clears instead.
            if (box == null || (box.width < 8 && box.height < 8)) {
              widget.onRegion(null);
              return;
            }
            widget.onRegion(_fractions(box));
          },
          onPanCancel: () => setState(() {
            _from = null;
            _to = null;
          }),
          child: Stack(children: [painter]),
        ),
      ),
    );
  }
}

/// The region's outline — the same marquee every "I am sweeping an area" mark
/// in the Viewer uses, so the gesture reads the same whichever one it is.
class _RegionPainter extends CustomPainter {
  final Rect? box;
  final Color accent;

  const _RegionPainter({required this.box, required this.accent});

  @override
  void paint(Canvas canvas, Size size) {
    final rect = box;
    if (rect == null) return;
    paintMarquee(canvas, rect, accent);
  }

  @override
  bool shouldRepaint(_RegionPainter old) =>
      old.box != box || old.accent != accent;
}
