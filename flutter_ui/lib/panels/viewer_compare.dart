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
// have to be in register or the seam means nothing. Only the pictures are
// wiped: the first view's bars run whole under both, and the second view
// lends its picture alone, at the first one's zoom and pan.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/dock.dart';
import '../state/viewer_views.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart' show showMenuAt;
import 'viewer_menus.dart';

/// The second view of a wipe: where across the picture the line sits, the
/// line itself, the view whose zoom and pan it takes, and what each view
/// shows where the two differ. The second side names its own composition
/// when it is another one; the first one's is in the bar. It draws right of
/// the line, or left of it once the sides are swapped.
class ViewerWipe {
  final double x;
  final Widget seam;
  final ViewerSurface registerTo;
  final String leadTag;
  final String sideTag;
  final bool nameComp;
  final bool swapped;

  const ViewerWipe({
    required this.x,
    required this.seam,
    required this.registerTo,
    this.leadTag = '',
    this.sideTag = '',
    this.nameComp = false,
    this.swapped = false,
  });
}

/// Everything right of `x`, or left of it when [swapped].
class ViewerWipeClip extends CustomClipper<Rect> {
  final double x;
  final bool swapped;
  const ViewerWipeClip(this.x, {this.swapped = false});

  @override
  Rect getClip(Size size) {
    final at = x.clamp(0.0, size.width);
    return swapped
        ? Rect.fromLTRB(0, 0, at, size.height)
        : Rect.fromLTRB(at, 0, size.width, size.height);
  }

  @override
  bool shouldReclip(ViewerWipeClip old) =>
      old.x != x || old.swapped != swapped;
}

class ViewerCompare extends StatelessWidget {
  final PaneId pane;
  final CompareMode mode;

  /// Where the divider sits, as a fraction across the box.
  final double at;
  final ValueChanged<double> onDivider;
  final Widget left;

  /// The second view: whole in a split, its picture alone in a wipe.
  final Widget Function(ViewerWipe? wipe) right;

  /// The first view, whose zoom and pan a wipe's second one takes.
  final ViewerSurface leftView;
  final ViewerSurface rightView;

  /// The second view drawn on the left, the first on the right.
  final bool swapped;

  const ViewerCompare({
    super.key,
    required this.pane,
    required this.mode,
    required this.at,
    required this.onDivider,
    required this.left,
    required this.right,
    required this.leftView,
    required this.rightView,
    this.swapped = false,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LayoutBuilder(
      builder: (context, box) {
        final x = (box.maxWidth * at).clamp(0.0, box.maxWidth);
        if (mode != CompareMode.split) {
          final ui = context.read<LumitUiState>();
          final (leadTag, sideTag) = _tags(ui);
          // Both at the whole box's size, so the two pictures are in
          // register and the seam is a seam rather than two shots meeting.
          return Stack(
            children: [
              Positioned.fill(child: left),
              Positioned.fill(
                child: right(ViewerWipe(
                  x: x,
                  seam: _seam(context, t, box, x),
                  registerTo: leftView,
                  leadTag: leadTag,
                  sideTag: sideTag,
                  nameComp: leftView.compId != rightView.compId,
                  swapped: swapped,
                )),
              ),
            ],
          );
        }
        final (first, second) =
            swapped ? (right(null), left) : (left, right(null));
        return Stack(
          children: [
            Positioned(left: 0, top: 0, bottom: 0, width: x, child: first),
            Positioned(
              left: x,
              top: 0,
              bottom: 0,
              width: box.maxWidth - x,
              child: second,
            ),
            Positioned(
              left: x - grab / 2,
              top: 0,
              bottom: 0,
              width: grab,
              child: _seam(context, t, box, x),
            ),
          ],
        );
      },
    );
  }

  /// What each side shows, where the two differ: the way it is looked at
  /// and the channel. Nothing where they look the same.
  (String, String) _tags(LumitUiState ui) {
    final a = ui.viewOf(leftView.engineId);
    final b = ui.viewOf(rightView.engineId);
    final words = <(String, String)>[
      if (a != b) (a.title, b.title),
      if (leftView.channel != rightView.channel)
        (
          viewerChannelName(leftView.channel),
          viewerChannelName(rightView.channel)
        ),
    ];
    return (
      [for (final w in words) w.$1].join(' · '),
      [for (final w in words) w.$2].join(' · '),
    );
  }

  Widget _seam(
          BuildContext context, LumitTheme t, BoxConstraints box, double x) =>
      viewerCompareSeam(context, t, box.maxWidth, x, onDivider);

  /// How wide the invisible grab strip on the divider is. Wider than the line
  /// it draws, because a one-pixel target is a target nobody hits.
  static const double grab = 11;
}

/// A compare's line, at [x] across a box [width] wide, and the grab strip on
/// it. Drawn in the accent, which §3.2 allows for a tool laid over the image.
/// The wipe against a snapshot wears the same one.
Widget viewerCompareSeam(BuildContext context, LumitTheme t, double width,
        double x, ValueChanged<double> onDivider) =>
    MouseRegion(
      cursor: SystemMouseCursors.resizeLeftRight,
      child: GestureDetector(
        key: const ValueKey('viewer-compare-seam'),
        behavior: HitTestBehavior.opaque,
        onHorizontalDragUpdate: (d) {
          if (width <= 0) return;
          onDivider(((x + d.delta.dx) / width).clamp(0.0, 1.0));
        },
        // A double click, or the menu, puts it back in the middle.
        onDoubleTap: () => onDivider(0.5),
        onSecondaryTapUp: (d) {
          final ui = context.read<LumitUiState>();
          showMenuAt<void>(
            context: context,
            position: d.globalPosition,
            rows: (close) => [
              ...viewerCompareRows(ui, () => close(null)),
              viewerRow(l10n.viewerMenuResetDivider, () => onDivider(0.5),
                  () => close(null),
                  id: 'viewer-compare-reset'),
            ],
          );
        },
        child: Center(
          child: Container(width: 1, color: t.accent),
        ),
      ),
    );
