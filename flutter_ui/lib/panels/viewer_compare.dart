// Two views put together: a wipe or a split, with a draggable divider
// (docs/impl/multi-viewer.md §3.4).
//
// In plain terms: the two pictures are already on screen, each drawing its own
// texture. A compare stacks them in one box and cuts them at a line the user
// can drag. **Nothing crosses the bridge** — no engine copy, no cache entry,
// nothing an export can see. It is a way of stacking two pictures, in the same
// shape as the snapshot: a display affordance and nothing more.
//
// The two modes differ in one thing. A **split** gives each view its own half
// of the box and lets it lay out there, which is the honest way to put two
// different compositions beside each other. A **wipe** draws both at the whole
// box's size and shows the left one up to the line and the right one after it,
// which is what a before-and-after of the same shot needs: the two pictures
// have to be in register or the seam means nothing.

import 'package:flutter/widgets.dart';

import '../state/dock.dart';
import '../state/viewer_views.dart';
import '../widgets/controls.dart';

class ViewerCompare extends StatelessWidget {
  final PaneId pane;
  final CompareMode mode;

  /// Where the divider sits, as a fraction across the box.
  final double at;
  final ValueChanged<double> onDivider;
  final Widget left;
  final Widget right;

  const ViewerCompare({
    super.key,
    required this.pane,
    required this.mode,
    required this.at,
    required this.onDivider,
    required this.left,
    required this.right,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LayoutBuilder(
      builder: (context, box) {
        final x = (box.maxWidth * at).clamp(0.0, box.maxWidth);
        return Stack(
          children: [
            if (mode == CompareMode.split) ...[
              Positioned(left: 0, top: 0, bottom: 0, width: x, child: left),
              Positioned(
                left: x,
                top: 0,
                bottom: 0,
                width: box.maxWidth - x,
                child: right,
              ),
            ] else ...[
              // Both at the whole box's size, so the two pictures are in
              // register and the seam is a seam rather than two shots meeting.
              Positioned.fill(child: right),
              ClipRect(
                clipper: _LeftOf(x),
                child: Positioned.fill(child: left),
              ),
            ],
            // The line itself, and the grab strip on it. Drawn in the accent,
            // which §3.2 allows for a tool laid over the image.
            Positioned(
              left: x - _grab / 2,
              top: 0,
              bottom: 0,
              width: _grab,
              child: MouseRegion(
                cursor: SystemMouseCursors.resizeLeftRight,
                child: GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onHorizontalDragUpdate: (d) {
                    if (box.maxWidth <= 0) return;
                    onDivider(
                      ((x + d.delta.dx) / box.maxWidth).clamp(0.0, 1.0),
                    );
                  },
                  child: Center(
                    child: Container(width: 1, color: t.accent),
                  ),
                ),
              ),
            ),
          ],
        );
      },
    );
  }

  /// How wide the invisible grab strip on the divider is. Wider than the line
  /// it draws, because a one-pixel target is a target nobody hits.
  static const double _grab = 11;
}

/// Everything left of `x`.
class _LeftOf extends CustomClipper<Rect> {
  final double x;
  const _LeftOf(this.x);

  @override
  Rect getClip(Size size) => Rect.fromLTWH(0, 0, x, size.height);

  @override
  bool shouldReclip(_LeftOf old) => old.x != x;
}
