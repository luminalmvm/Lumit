// The Razor tool over the Timeline's lanes (K-220, docs/07 §4.4): the blade
// pointer, the line that shows where the cut lands, and which layers a click
// actually cuts.
//
// **In plain terms.** With the razor in hand the pointer becomes a blade and a
// vertical line follows it across the lanes: that line is where the cut will
// happen. Clicking a layer's bar cuts it *there* — not at the playhead — which
// is the whole difference between a razor and the Cut-at-playhead command.
// Holding Shift cuts every layer that spans that moment at once, the way
// Premiere's razor cuts all tracks.
//
// **Two kinds of cut, because there are two kinds of layer.** A Sequence layer
// holds clips, so cutting it makes an **edit point** inside it and the layer
// stays one layer. Everything else **splits into two layers**, which is what
// After Effects does — both halves keep the source, effects, masks and
// keyframes, and each takes half the span. The engine decides which is which;
// this only asks (`cut_clip_at` or `split_at`).

import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

/// How big the scissors pointer is drawn, in screen pixels.
const double razorCursorSize = 16;

/// The layers a razor click at [frame] should cut.
///
/// [clicked] is the bar under the pointer, or null when the click landed on
/// empty lane space. Plain: only what was clicked. [allLayers] (Shift): every
/// layer whose span contains that moment, whether it was clicked or not —
/// including the clicked one.
///
/// A layer is only a target while the cut would land **strictly inside** it:
/// cutting at an end makes a layer of no length, which the engine refuses
/// anyway, and offering it would make Shift look as though it had done
/// something to layers it had not.
List<BridgeLayerEntry> razorTargets(
  List<BridgeLayerEntry> layers,
  int frame, {
  required BridgeLayerEntry? clicked,
  required bool allLayers,
}) {
  bool spans(BridgeLayerEntry entry) =>
      frame > entry.info.inFrame.toInt() && frame < entry.info.outFrame.toInt();

  if (allLayers) {
    return [
      for (final entry in layers)
        if (spans(entry)) entry,
    ];
  }
  if (clicked == null || !spans(clicked)) return const [];
  return [clicked];
}

/// Cut every layer in [targets] at [frame], and say whether anything happened.
///
/// A Sequence layer gains an edit point; anything else splits in two. Each is a
/// single op, so each is a single undo step (docs/07 §4.7) — a Shift-cut across
/// five layers is five steps, which is honest: it is five edits.
///
/// A refusal is silence, not an error: the engine declines a clip an eased ramp
/// cannot be cut through, and a razor that threw a dialogue at the user for
/// clicking slightly wrong would be worse than one that does nothing.
bool razorCut(List<BridgeLayerEntry> targets, int frame) {
  var cut = false;
  for (final entry in targets) {
    try {
      if (entry.info.kind == BridgeLayerKind.sequence) {
        entry.layer.cutClipAt(frame: frame);
      } else {
        entry.layer.splitAt(frame: frame);
      }
      cut = true;
    } catch (_) {
      // Nothing cuttable there. The next layer still gets its turn.
    }
  }
  return cut;
}

/// The blade pointer and the cut line, over whatever [child] draws.
///
/// Wrapped round the lanes rather than laid over them as a sibling: the line
/// has to span every row, and the pointer must not be clipped to one bar.
/// Neither takes a gesture — the bars keep their own clicks and drags.
class RazorOverlay extends StatefulWidget {
  /// Whether the Razor tool is armed.
  final bool active;

  /// Where a cut at screen x would actually land, in this overlay's own pixels.
  ///
  /// The cut has always been quantised — `TimelineAxis.frameAt` rounds — while
  /// the line drawn under the blade followed the pointer continuously, so the
  /// two disagreed by up to half a frame and the mark was not where the edge
  /// bit. Given the same function the cut uses, the line says the truth (and,
  /// with the magnet on, says it about markers and edit points too).
  ///
  /// Null leaves the line under the pointer, which is what a caller with no
  /// axis to quantise against should get.
  final double Function(double x)? snapX;

  /// The pointer's colours: the mark, and the outline that keeps it legible
  /// over a bar of any label colour.
  final Color mark;
  final Color outline;

  final Widget child;

  const RazorOverlay({
    super.key,
    required this.active,
    this.snapX,
    required this.mark,
    required this.outline,
    required this.child,
  });

  @override
  State<RazorOverlay> createState() => _RazorOverlayState();
}

class _RazorOverlayState extends State<RazorOverlay> {
  Offset? _pointer;

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return widget.child;
    return MouseRegion(
      // Hidden and replaced, for the same reason the Rotation tool's is
      // (K-219): no platform ships a razor, and a system arrow inside the drawn
      // blade would read as two pointers.
      cursor: SystemMouseCursors.none,
      onEnter: (event) => setState(() => _pointer = event.localPosition),
      onHover: (event) => setState(() => _pointer = event.localPosition),
      onExit: (_) => setState(() => _pointer = null),
      child: Stack(
        children: [
          widget.child,
          Positioned.fill(
            child: RepaintBoundary(
              child: IgnorePointer(
                child: Stack(
                  children: [
                    Positioned.fill(
                      child: CustomPaint(
                        painter: _RazorCutLinePainter(
                          at: _pointer,
                          // The line marks where the edge bites, not where the
                          // pointer is; the blade above still follows the hand.
                          lineX: _pointer == null
                              ? null
                              : (widget.snapX?.call(_pointer!.dx) ??
                                  _pointer!.dx),
                          mark: widget.mark,
                        ),
                      ),
                    ),
                    if (_pointer != null)
                      Positioned(
                        left: _pointer!.dx - razorCursorSize / 2,
                        top: _pointer!.dy - razorCursorSize / 2,
                        // The application's own scissors, drawn twice: the halo
                        // copy a pixel down and across, then the ink over it, so
                        // it is legible on a bar of any label colour. The same
                        // trick the badged tool pointers use.
                        child: Stack(
                          children: [
                            Transform.translate(
                              offset: const Offset(1, 1),
                              child: lumitIcon(LumitIcon.razor,
                                  size: razorCursorSize, color: widget.outline),
                            ),
                            lumitIcon(LumitIcon.razor,
                                size: razorCursorSize, color: widget.mark),
                          ],
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// The line down the lanes that says where the cut lands (K-235).
///
/// **This is the mark that matters**, and it is now the only one drawn here:
/// the pointer itself is the application's own scissors, placed as a widget
/// above. A hand-drawn blade leaning off the point it cuts at needed a second
/// mark to say where the edge actually bit — and once the line says that, the
/// pointer only has to say *which tool is in hand*, which the toolbar's own
/// icon already says better than a bespoke drawing of one.
class _RazorCutLinePainter extends CustomPainter {
  final Offset? at;

  /// Where the line goes: the pointer's x put through the caller's snap, so it
  /// stands where the cut will actually land.
  final double? lineX;
  final Color mark;

  const _RazorCutLinePainter({
    required this.at,
    required this.lineX,
    required this.mark,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final x = lineX;
    if (at == null || x == null) return;
    canvas.drawLine(
      Offset(x, 0),
      Offset(x, size.height),
      Paint()
        ..color = mark.withValues(alpha: 0.7)
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(_RazorCutLinePainter old) =>
      old.at != at || old.lineX != lineX || old.mark != mark;
}
