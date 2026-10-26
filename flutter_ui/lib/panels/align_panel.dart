// The Align panel: two grids of nine dots. A dot on the first moves the
// selected layers' anchors to that point of their boxes without moving the
// picture. A dot on the second moves each layer so that point of its box
// meets the same point of the composition. The Viewer does both, with the
// code behind its Anchor point tool and its right-click Align.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/menu_layer_frb.dart' show selectedEntries;
import '../widgets/controls.dart';

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
            }.contains(e.info.kind));
        Widget grid(String label, bool anchor) => Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(label, style: t.body.copyWith(color: t.textMuted)),
                const SizedBox(height: 6),
                _DotGrid(
                  key: ValueKey<String>(anchor ? 'align-anchor' : 'align-comp'),
                  onPick: live
                      ? (at) => ui.requestAlign(at, anchor: anchor)
                      : null,
                ),
              ],
            );
        return Container(
          color: t.surface0,
          alignment: Alignment.topLeft,
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(10),
            child: Wrap(
              spacing: 24,
              runSpacing: 14,
              children: [
                grid(l10n.transformAnchorPoint, true),
                grid(l10n.alignComposition, false),
              ],
            ),
          ),
        );
      },
    );
  }
}

/// Nine dots joined by lines. A press picks the nearest dot, as a point of a
/// box from 0 to 1 across and down. Greyed and inert with no [onPick].
class _DotGrid extends StatefulWidget {
  final ValueChanged<Offset>? onPick;

  const _DotGrid({super.key, required this.onPick});

  static const double size = 72;
  static const double inset = 8;

  @override
  State<_DotGrid> createState() => _DotGridState();
}

class _DotGridState extends State<_DotGrid> {
  Offset? _hover;

  /// The dot nearest [local], each axis 0, 0.5 or 1.
  Offset _dotAt(Offset local) {
    const span = _DotGrid.size - 2 * _DotGrid.inset;
    double snap(double v) =>
        ((v - _DotGrid.inset) / span * 2).round().clamp(0, 2) / 2;
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
        child: CustomPaint(
          size: const Size.square(_DotGrid.size),
          painter: _DotGridPainter(
            hover: pick == null ? null : _hover,
            line: pick == null ? t.textDisabled : t.textMuted,
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
    const inset = _DotGrid.inset;
    final span = size.width - 2 * inset;
    Offset at(double x, double y) => Offset(inset + x * span, inset + y * span);
    final stroke = Paint()
      ..color = line
      ..strokeWidth = 1.5;
    for (final v in const [0.0, 0.5, 1.0]) {
      canvas.drawLine(at(v, 0), at(v, 1), stroke);
      canvas.drawLine(at(0, v), at(1, v), stroke);
    }
    for (final y in const [0.0, 0.5, 1.0]) {
      for (final x in const [0.0, 0.5, 1.0]) {
        final on = hover == Offset(x, y);
        canvas.drawCircle(
            at(x, y), on ? 6 : 4.5, Paint()..color = on ? hot : dot);
      }
    }
  }

  @override
  bool shouldRepaint(_DotGridPainter old) =>
      old.hover != hover || old.line != line || old.dot != dot || old.hot != hot;
}
