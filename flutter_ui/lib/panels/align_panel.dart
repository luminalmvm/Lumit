// The Align panel: two grids of nine dots. A dot on the first moves the
// selected layers' anchors to that point of their boxes without moving the
// picture. A dot on the second moves each layer so that point of its box
// meets the same point of the composition. The Viewer does both, with the
// code behind its Anchor point tool and its right-click Align. A right-click
// offers the two a person wants most, the middle of each.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/menu_layer_frb.dart' show selectedEntries;
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart' show showMenuAt;

/// The most a grid grows to. Past this the dots are only taking up room.
const double _largestGrid = 112;

class AlignPanel extends StatelessWidget {
  const AlignPanel({super.key});

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return ListenableBuilder(
      listenable: Listenable.merge([ui.model, ui.selectedLayers]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        // Cameras, lights and sound have no box to line up.
        final live = selectedEntries(ui).any((e) => !const {
              BridgeLayerKind.camera,
              BridgeLayerKind.light,
              BridgeLayerKind.audio,
              BridgeLayerKind.bus,
            }.contains(e.info.kind));
        Widget grid(String label, bool anchor, double side) => Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(label,
                    style: t.body.copyWith(color: t.textMuted),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis),
                const SizedBox(height: 6),
                _DotGrid(
                  key: ValueKey<String>(anchor ? 'align-anchor' : 'align-comp'),
                  size: side,
                  onPick: live
                      ? (at) => ui.requestAlign(at, anchor: anchor)
                      : null,
                  onMenu: (place) => _menu(context, place, ui, live),
                ),
              ],
            );
        return Container(
          color: t.surface0,
          padding: const EdgeInsets.all(16),
          child: LayoutBuilder(builder: (context, box) {
            // The grids grow with the panel up to a size that still reads as a
            // small control, side by side or stacked, whichever lets them be
            // bigger. A label and its gap take about 22 high; the FittedBox
            // catches anything that doesn't fit.
            const label = 22.0, gap = 24.0;
            final across =
                math.min((box.maxWidth - gap) / 2, box.maxHeight - label);
            final down =
                math.min(box.maxWidth, (box.maxHeight - gap) / 2 - label);
            final side = math.max(math.max(across, down), 32.0);
            return Align(
              alignment: AlignmentDirectional.topStart,
              child: FittedBox(
                fit: BoxFit.scaleDown,
                child: Flex(
                  direction: across >= down ? Axis.horizontal : Axis.vertical,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    grid(l10n.transformAnchorPoint, true,
                        math.min(side, _largestGrid)),
                    const SizedBox.square(dimension: gap),
                    grid(l10n.alignComposition, false,
                        math.min(side, _largestGrid)),
                  ],
                ),
              ),
            );
          }),
        );
      },
    );
  }

  /// The middle of each, by the same road a press on a dot takes.
  void _menu(BuildContext context, Offset at, LumitUiState ui, bool live) {
    const middle = Offset(0.5, 0.5);
    showMenuAt<void>(
      context: context,
      position: at,
      rows: (close) => [
        MenuRow(
          key: const ValueKey('align-menu-anchor'),
          enabled: live,
          onPressed: () {
            close(null);
            ui.requestAlign(middle, anchor: true);
          },
          child: MenuLabel(l10n.alignAnchorToCentre, enabled: live),
        ),
        MenuRow(
          key: const ValueKey('align-menu-comp'),
          enabled: live,
          onPressed: () {
            close(null);
            ui.requestAlign(middle, anchor: false);
          },
          child: MenuLabel(l10n.alignCentreInComp, enabled: live),
        ),
      ],
    );
  }
}

/// Nine dots joined by lines. A press picks the nearest dot, as a point of a
/// box from 0 to 1 across and down. Greyed and inert with no [onPick].
class _DotGrid extends StatefulWidget {
  final double size;
  final ValueChanged<Offset>? onPick;

  /// A right press, at this point in the window.
  final ValueChanged<Offset> onMenu;

  const _DotGrid({
    super.key,
    required this.size,
    required this.onPick,
    required this.onMenu,
  });

  /// The gap from the edge to the outer dots, so they draw whole.
  static double insetFor(double size) => size / 9;

  @override
  State<_DotGrid> createState() => _DotGridState();
}

class _DotGridState extends State<_DotGrid> {
  Offset? _hover;

  /// The dot nearest [local], each axis 0, 0.5 or 1.
  Offset _dotAt(Offset local) {
    final inset = _DotGrid.insetFor(widget.size);
    final span = widget.size - 2 * inset;
    double snap(double v) => ((v - inset) / span * 2).round().clamp(0, 2) / 2;
    return Offset(snap(local.dx), snap(local.dy));
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final pick = widget.onPick;
    return MouseRegion(
      cursor: pick == null ? MouseCursor.defer : SystemMouseCursors.click,
      onHover: (e) => setState(() => _hover = _dotAt(e.localPosition)),
      onExit: (_) => setState(() => _hover = null),
      child: GestureDetector(
        onTapUp: pick == null ? null : (d) => pick(_dotAt(d.localPosition)),
        onSecondaryTapDown: (d) => widget.onMenu(d.globalPosition),
        child: CustomPaint(
          size: Size.square(widget.size),
          painter: _DotGridPainter(
            hover: pick == null ? null : _hover,
            // Neutral until the pointer is on one: the colour is the answer to
            // where this press would land.
            line: pick == null ? t.hairline : t.hairlineStrong,
            dot: pick == null ? t.textDisabled : t.textSecondary,
            hot: t.accent,
          ),
        ),
      ),
    );
  }
}

class _DotGridPainter extends CustomPainter {
  final Offset? hover;
  final Color line;
  final Color dot;
  final Color hot;

  const _DotGridPainter({
    required this.hover,
    required this.line,
    required this.dot,
    required this.hot,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final inset = _DotGrid.insetFor(size.width);
    final span = size.width - 2 * inset;
    Offset at(double x, double y) => Offset(inset + x * span, inset + y * span);
    // Dots grow slower than the grid, so a big one stays light.
    final r = 2 + size.width / 45;
    final stroke = Paint()
      ..color = line
      ..strokeWidth = 1;
    for (final v in const [0.0, 0.5, 1.0]) {
      canvas.drawLine(at(v, 0), at(v, 1), stroke);
      canvas.drawLine(at(0, v), at(1, v), stroke);
    }
    for (final y in const [0.0, 0.5, 1.0]) {
      for (final x in const [0.0, 0.5, 1.0]) {
        final on = hover == Offset(x, y);
        canvas.drawCircle(
            at(x, y), on ? r * 4 / 3 : r, Paint()..color = on ? hot : dot);
      }
    }
  }

  @override
  bool shouldRepaint(_DotGridPainter old) =>
      old.hover != hover || old.line != line || old.dot != dot || old.hot != hot;
}
