// The Viewer's **rulers** and the **guides** dragged out of them (K-689,
// docs/07 §2.2 item 6).
//
// **In plain terms.** Two strips along the top and left edges of the picture,
// counting the composition's own pixels — so a 1920×1080 comp reads 0 to 1920
// across whatever magnification it is being looked at. Press in a strip and
// drag onto the picture and a line comes out with the pointer: a **guide**, the
// straight edge every layout is built against. Drag one back into the strip and
// it is gone.
//
// Everything here is **display only**. A guide is not in the composition, no
// export has ever seen one, and none of it crosses the bridge; guides ride the
// per-project session beside the overlay switches (K-689), so a comp opens with
// the lines it was left with and Ctrl+Z never undoes one.
//
// **Where the numbers live.** A guide's position is kept in **comp pixels** —
// the units the rulers count and the units a layer's Position is measured in —
// and turned into a place on screen from the picture's own rectangle, which is
// what makes guides pan and zoom with the shot instead of floating over it.

import 'dart:math' as math;

import 'package:flutter/foundation.dart' show listEquals;
import 'package:flutter/widgets.dart';

import '../state/workspace.dart' show ViewerGuide;

/// How deep a ruler strip is. Two rows of a 9px label with room to breathe, and
/// the same 18 the Viewer's own pickers stand at (docs/07 §2.2).
const double viewerRulerBand = 18;

/// How near, in screen pixels, the pointer has to come to a guide before the
/// drag is the guide's rather than the picture's. The Timeline's own slop
/// (`snapSlopPixels`), for the same reason: a hair under half a row.
const double viewerGuideGrab = 8;

/// How far apart, in screen pixels, two labelled ticks are allowed to get
/// before the ruler steps up to a coarser count.
const double viewerRulerLabelGap = 64;

/// The gap between labelled ticks, in **comp pixels**, at magnification
/// [viewScale] (the picture's width on screen over the comp's own).
///
/// A 1 / 2 / 5 ladder, which is the ladder every ruler and every axis in the
/// application uses: the numbers stay round at every step, so a reader never
/// has to work out what a tick is worth. The step is the first rung that keeps
/// labelled ticks at least [viewerRulerLabelGap] apart on screen — measured in
/// *pixels*, so the magnification is the precision control, exactly as it is
/// for the Timeline's magnet.
double viewerRulerStep(double viewScale,
    {double minGapPx = viewerRulerLabelGap}) {
  // A collapsed picture has no scale to divide by; 100 is a sane ruler for a
  // comp nobody can see, and nothing is drawn at that size anyway.
  if (!viewScale.isFinite || viewScale <= 0) return 100;
  final wanted = minGapPx / viewScale;
  if (!wanted.isFinite || wanted <= 0) return 100;
  final decade = math.pow(10, (math.log(wanted) / math.ln10).floor()).toDouble();
  for (final rung in const [1.0, 2.0, 5.0]) {
    if (decade * rung >= wanted) return decade * rung;
  }
  return decade * 10;
}

/// Where a guide sits on screen, given the picture's rectangle and the comp's
/// own size — the one conversion everything here shares.
double viewerGuideScreen(
  ViewerGuide guide, {
  required Rect picture,
  required Size compSize,
}) {
  if (guide.vertical) {
    final scale = compSize.width == 0 ? 1.0 : picture.width / compSize.width;
    return picture.left + guide.at * scale;
  }
  final scale = compSize.height == 0 ? 1.0 : picture.height / compSize.height;
  return picture.top + guide.at * scale;
}

/// The inverse: where a point on screen falls in comp pixels along one axis.
double viewerGuideComp(
  double screen, {
  required bool vertical,
  required Rect picture,
  required Size compSize,
}) {
  if (vertical) {
    final scale = compSize.width == 0 ? 1.0 : picture.width / compSize.width;
    return scale == 0 ? 0 : (screen - picture.left) / scale;
  }
  final scale = compSize.height == 0 ? 1.0 : picture.height / compSize.height;
  return scale == 0 ? 0 : (screen - picture.top) / scale;
}

/// The rulers, the guides, and the drags that make and move them.
///
/// One widget rather than three because they are one mechanism: the strips are
/// where guides come from, and a guide dropped back on a strip is a guide
/// deleted. It draws in the **stage's** coordinates, so [picture] is the
/// picture's rectangle in those same coordinates.
class ViewerRulers extends StatefulWidget {
  /// Whether the strips themselves are drawn. Guides are drawn whenever there
  /// are any: hiding the rulers is about the edges of the panel, and a guide
  /// you placed is a thing you placed.
  final bool rulers;

  /// Where the picture is drawn in the stage, and how big the comp is — the two
  /// halves of every conversion above.
  final Rect picture;
  final Size compSize;

  /// The comp's guides, and how to write a new set back.
  final List<ViewerGuide> guides;
  final ValueChanged<List<ViewerGuide>> onGuides;

  /// The strips' own colours: neutral, because they sit inside the Viewer's
  /// neutrality zone (docs/15 §3.2). The guides are the exemption that section
  /// names — a tool overlaid on the image itself — so they carry the accent.
  final Color band;
  final Color line;
  final Color label;
  final Color guideColour;

  const ViewerRulers({
    super.key,
    required this.rulers,
    required this.picture,
    required this.compSize,
    required this.guides,
    required this.onGuides,
    required this.band,
    required this.line,
    required this.label,
    required this.guideColour,
  });

  @override
  State<ViewerRulers> createState() => _ViewerRulersState();
}

class _ViewerRulersState extends State<ViewerRulers> {
  /// The guide being dragged: which one (null for a line still coming out of a
  /// ruler), which way it runs, and where it is this instant in comp pixels.
  ///
  /// Held here rather than written through on every movement, for the reason
  /// every drag in the application holds its own gesture: the guides ride the
  /// session, and a session written per pointer sample is a file written sixty
  /// times a second.
  ({int? index, bool vertical, double at})? _drag;

  @override
  Widget build(BuildContext context) {
    final band = widget.rulers ? viewerRulerBand : 0.0;
    return Positioned.fill(
      child: Stack(
        children: [
          // The lines and the strips. Its own boundary (docs/impl/
          // ui-performance.md §2): the ruler's labels are laid out text, and
          // nothing about them changes when a frame arrives or a layer is
          // picked.
          Positioned.fill(
            child: RepaintBoundary(
              child: IgnorePointer(
                child: CustomPaint(
                  key: const ValueKey('viewer-rulers'),
                  painter: ViewerRulerPainter(
                    rulers: widget.rulers,
                    picture: widget.picture,
                    compSize: widget.compSize,
                    guides: _drawnGuides(),
                    band: widget.band,
                    line: widget.line,
                    label: widget.label,
                    guideColour: widget.guideColour,
                  ),
                ),
              ),
            ),
          ),
          // One grab strip per guide, so moving a guide costs the picture
          // underneath no hit area at all — the alternative, a layer over the
          // whole stage, would take every click meant for a layer.
          for (var i = 0; i < widget.guides.length; i++) _grab(i, band),
          // The strips themselves, last so a guide's grab strip crossing one
          // cannot steal the drag that pulls a new guide out.
          if (widget.rulers) ..._bands(),
        ],
      ),
    );
  }

  /// The guides as they are drawn this instant: the stored set, with the one in
  /// flight standing where the pointer has it.
  List<ViewerGuide> _drawnGuides() {
    final drag = _drag;
    if (drag == null) return widget.guides;
    final out = [...widget.guides];
    final line = (at: drag.at, vertical: drag.vertical);
    if (drag.index == null) {
      out.add(line);
    } else if (drag.index! < out.length) {
      out[drag.index!] = line;
    }
    return out;
  }

  Widget _grab(int index, double band) {
    final guide = widget.guides[index];
    final at = viewerGuideScreen(guide,
        picture: widget.picture, compSize: widget.compSize);
    return Positioned(
      left: guide.vertical ? at - viewerGuideGrab / 2 : 0,
      top: guide.vertical ? band : at - viewerGuideGrab / 2,
      width: guide.vertical ? viewerGuideGrab : null,
      height: guide.vertical ? null : viewerGuideGrab,
      right: guide.vertical ? null : 0,
      bottom: guide.vertical ? 0 : null,
      child: MouseRegion(
        cursor: guide.vertical
            ? SystemMouseCursors.resizeLeftRight
            : SystemMouseCursors.resizeUpDown,
        child: GestureDetector(
          key: ValueKey<String>('viewer-guide-$index'),
          behavior: HitTestBehavior.opaque,
          onPanStart: (d) => _start(index, guide.vertical, d.globalPosition),
          onPanUpdate: (d) => _move(d.globalPosition),
          onPanEnd: (_) => _end(),
          onPanCancel: () => setState(() => _drag = null),
        ),
      ),
    );
  }

  /// The two strips: the top one pulls out a horizontal guide, the left one a
  /// vertical guide — which way round is the way round every editor has it.
  List<Widget> _bands() => [
        Positioned(
          left: 0,
          top: 0,
          right: 0,
          height: viewerRulerBand,
          child: _bandDetector('viewer-ruler-top', vertical: false),
        ),
        Positioned(
          left: 0,
          top: viewerRulerBand,
          bottom: 0,
          width: viewerRulerBand,
          child: _bandDetector('viewer-ruler-left', vertical: true),
        ),
      ];

  Widget _bandDetector(String key, {required bool vertical}) => MouseRegion(
        cursor: vertical
            ? SystemMouseCursors.resizeLeftRight
            : SystemMouseCursors.resizeUpDown,
        child: GestureDetector(
          key: ValueKey<String>(key),
          behavior: HitTestBehavior.opaque,
          onPanStart: (d) => _start(null, vertical, d.globalPosition),
          onPanUpdate: (d) => _move(d.globalPosition),
          onPanEnd: (_) => _end(),
          onPanCancel: () => setState(() => _drag = null),
        ),
      );

  /// Where a global pointer position falls in this layer's own coordinates.
  ///
  /// Global rather than local because the drag starts in one child (a strip, a
  /// grab bar) and travels across the whole stage: a local position is measured
  /// against whichever box took the gesture, and a guide dragged out of the top
  /// strip would be measured against an 18px-tall box for the rest of its
  /// journey.
  Offset _local(Offset global) {
    final box = context.findRenderObject();
    return box is RenderBox ? box.globalToLocal(global) : global;
  }

  void _start(int? index, bool vertical, Offset global) {
    final at = _local(global);
    setState(() => _drag = (
          index: index,
          vertical: vertical,
          at: viewerGuideComp(vertical ? at.dx : at.dy,
              vertical: vertical,
              picture: widget.picture,
              compSize: widget.compSize),
        ));
  }

  void _move(Offset global) {
    final drag = _drag;
    if (drag == null) return;
    final at = _local(global);
    setState(() => _drag = (
          index: drag.index,
          vertical: drag.vertical,
          at: viewerGuideComp(drag.vertical ? at.dx : at.dy,
              vertical: drag.vertical,
              picture: widget.picture,
              compSize: widget.compSize),
        ));
  }

  /// Let go. A guide dropped **on the picture** is kept; one dropped anywhere
  /// else — back on a ruler, off the side of the panel — is dropped, which is
  /// how a guide is deleted and how a drag started by accident costs nothing.
  void _end() {
    final drag = _drag;
    setState(() => _drag = null);
    if (drag == null) return;
    final screen = viewerGuideScreen((at: drag.at, vertical: drag.vertical),
        picture: widget.picture, compSize: widget.compSize);
    final inside = drag.vertical
        ? screen >= widget.picture.left && screen <= widget.picture.right
        : screen >= widget.picture.top && screen <= widget.picture.bottom;
    final out = [...widget.guides];
    if (drag.index == null) {
      if (!inside) return;
      out.add((at: drag.at, vertical: drag.vertical));
    } else if (drag.index! < out.length) {
      if (inside) {
        out[drag.index!] = (at: drag.at, vertical: drag.vertical);
      } else {
        out.removeAt(drag.index!);
      }
    }
    widget.onGuides(out);
  }
}

/// The strips, their ticks and the guides, in one painter.
///
/// Painted in that order for a reason: a guide runs the width of the picture
/// and would otherwise run *through* the ruler counting it, which reads as a
/// line escaping the panel rather than a mark on the shot.
class ViewerRulerPainter extends CustomPainter {
  final bool rulers;
  final Rect picture;
  final Size compSize;
  final List<ViewerGuide> guides;
  final Color band;
  final Color line;
  final Color label;
  final Color guideColour;

  const ViewerRulerPainter({
    required this.rulers,
    required this.picture,
    required this.compSize,
    required this.guides,
    required this.band,
    required this.line,
    required this.label,
    required this.guideColour,
  });

  @override
  void paint(Canvas canvas, Size size) {
    _paintGuides(canvas, size);
    if (rulers) _paintBands(canvas, size);
  }

  void _paintGuides(Canvas canvas, Size size) {
    if (guides.isEmpty) return;
    final paint = Paint()
      ..color = guideColour
      ..strokeWidth = 1;
    // Where the strips end: a guide under one of them is a guide the ruler
    // would be drawn over anyway.
    final edge = rulers ? viewerRulerBand : 0.0;
    for (final guide in guides) {
      final at = viewerGuideScreen(guide, picture: picture, compSize: compSize);
      if (!at.isFinite || at < edge) continue;
      if (guide.vertical) {
        canvas.drawLine(Offset(at, edge), Offset(at, size.height), paint);
      } else {
        canvas.drawLine(Offset(edge, at), Offset(size.width, at), paint);
      }
    }
  }

  void _paintBands(Canvas canvas, Size size) {
    final fill = Paint()..color = band;
    final hairline = Paint()
      ..color = line
      ..strokeWidth = 1;
    canvas.drawRect(Rect.fromLTWH(0, 0, size.width, viewerRulerBand), fill);
    canvas.drawRect(
        Rect.fromLTWH(0, viewerRulerBand, viewerRulerBand, size.height), fill);
    // The seam between each strip and the picture beside it.
    canvas
      ..drawLine(Offset(0, viewerRulerBand),
          Offset(size.width, viewerRulerBand), hairline)
      ..drawLine(Offset(viewerRulerBand, viewerRulerBand),
          Offset(viewerRulerBand, size.height), hairline);
    if (picture.isEmpty) return;
    _paintTicks(canvas, size, vertical: true);
    _paintTicks(canvas, size, vertical: false);
  }

  /// One strip's ticks. [vertical] names the *axis being counted*: the top
  /// strip counts comp x, the left one comp y.
  void _paintTicks(Canvas canvas, Size size, {required bool vertical}) {
    final span = vertical ? compSize.width : compSize.height;
    final drawn = vertical ? picture.width : picture.height;
    if (span <= 0 || drawn <= 0) return;
    final scale = drawn / span;
    final step = viewerRulerStep(scale);
    final hairline = Paint()
      ..color = line
      ..strokeWidth = 1;
    // Only the part of the comp the panel can actually show, which is what
    // keeps a ruler at 800 % the same cost as one at 25 % (K-230's rule for
    // the transparency board, and the same arithmetic).
    final start = viewerGuideComp(viewerRulerBand,
        vertical: vertical, picture: picture, compSize: compSize);
    final end = viewerGuideComp(vertical ? size.width : size.height,
        vertical: vertical, picture: picture, compSize: compSize);
    final first = (start / step).floorToDouble() * step;
    for (var at = first; at <= end; at += step) {
      final screen = viewerGuideScreen((at: at, vertical: vertical),
          picture: picture, compSize: compSize);
      if (screen < viewerRulerBand) continue;
      // A minor tick every fifth of the step, unlabelled: the count between
      // two numbers, which is what makes a ruler readable without doing sums.
      for (var m = 1; m < 5; m++) {
        final minor = screen + step * scale * m / 5;
        if (minor < viewerRulerBand) continue;
        _tick(canvas, minor, vertical: vertical, depth: 4, paint: hairline);
      }
      _tick(canvas, screen,
          vertical: vertical, depth: viewerRulerBand, paint: hairline);
      _label(canvas, screen, at, vertical: vertical);
    }
  }

  void _tick(Canvas canvas, double at,
      {required bool vertical,
      required double depth,
      required Paint paint}) {
    if (vertical) {
      canvas.drawLine(
          Offset(at, viewerRulerBand - depth), Offset(at, viewerRulerBand),
          paint);
    } else {
      canvas.drawLine(
          Offset(viewerRulerBand - depth, at), Offset(viewerRulerBand, at),
          paint);
    }
  }

  /// The number beside a major tick, in comp pixels.
  ///
  /// The horizontal ruler writes it beside the tick; the vertical one writes it
  /// the same way up rather than turned on its side — a rotated numeral is a
  /// number nobody reads at a glance, and the strip is deep enough for two
  /// digits, which is what a rounded step gives it.
  void _label(Canvas canvas, double screen, double value,
      {required bool vertical}) {
    final text = TextPainter(
      text: TextSpan(
        text: value.abs() < 1e-6 ? '0' : value.toStringAsFixed(0),
        style: TextStyle(color: label, fontSize: 9),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    text.paint(
      canvas,
      vertical
          ? Offset(screen + 2, (viewerRulerBand - text.height) / 2)
          : Offset(1, screen + 2),
    );
    text.dispose();
  }

  @override
  bool shouldRepaint(ViewerRulerPainter old) =>
      old.rulers != rulers ||
      old.picture != picture ||
      old.compSize != compSize ||
      !listEquals(old.guides, guides) ||
      old.band != band ||
      old.line != line ||
      old.label != label ||
      old.guideColour != guideColour;
}
