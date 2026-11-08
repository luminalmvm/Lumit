// The Align panel: three grids. A dot on the first moves the selected layers'
// anchors to that point of their boxes without moving the picture. A dot on the
// second moves each layer so that point of its box meets the same point of the
// composition. The third lines the selected layers up with each other and
// spaces them out. The Viewer does all of it, with the code behind its Anchor
// point tool and its right-click Align. A right-click offers the same again by
// name.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart';
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show MenuEntry;
import '../shell/menu_layer_frb.dart' show selectedEntries;
import '../widgets/controls.dart';
import 'layer_fit.dart';
import 'timeline_layer_menu_frb.dart' show showMenuEntriesAt;

/// The most a grid grows to. Past this the dots are only taking up room.
const double _largestGrid = 112;

/// The height of the row of buttons under the composition's grid.
const double _footHeight = 26;

class AlignPanel extends StatelessWidget {
  const AlignPanel({super.key});

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final app = Provider.of<LumitState>(context, listen: false);
    return ListenableBuilder(
      listenable: Listenable.merge([ui.model, ui.selectedLayers]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        // Cameras, lights and sound have no box to line up.
        final boxes = selectedEntries(ui)
            .where((e) => !const {
                  BridgeLayerKind.camera,
                  BridgeLayerKind.light,
                  BridgeLayerKind.audio,
                  BridgeLayerKind.bus,
                }.contains(e.info.kind))
            .length;
        final live = boxes > 0;
        Widget labelled(String label, Widget child) => Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(label,
                    style: t.body.copyWith(color: t.textMuted),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis),
                const SizedBox(height: 6),
                child,
              ],
            );
        Widget grid(String label, bool anchor, double side) => labelled(
              label,
              _DotGrid(
                key: ValueKey<String>(anchor ? 'align-anchor' : 'align-comp'),
                size: side,
                onPick:
                    live ? (at) => ui.requestAlign(at, anchor: anchor) : null,
                onMenu: (place) => _menu(context, place, app, ui, boxes),
              ),
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
            final across = math.min((box.maxWidth - 2 * gap) / 3,
                box.maxHeight - label - _footHeight);
            final down = math.min(box.maxWidth,
                (box.maxHeight - 2 * gap - _footHeight) / 3 - label);
            final side =
                math.min(math.max(math.max(across, down), 32.0), _largestGrid);
            return Align(
              alignment: AlignmentDirectional.topStart,
              child: FittedBox(
                fit: BoxFit.scaleDown,
                child: Flex(
                  direction: across >= down ? Axis.horizontal : Axis.vertical,
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    grid(l10n.transformAnchorPoint, true, side),
                    const SizedBox.square(dimension: gap),
                    Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        grid(l10n.alignComposition, false, side),
                        _FitButtons(side: side, app: app, ui: ui),
                      ],
                    ),
                    const SizedBox.square(dimension: gap),
                    labelled(
                      l10n.alignSelection,
                      GestureDetector(
                        onSecondaryTapDown: (d) => _menu(
                            context, d.globalPosition, app, ui, boxes),
                        child: _SelectionGrid(
                            side: side, ui: ui, picked: boxes),
                      ),
                    ),
                  ],
                ),
              ),
            );
          }),
        );
      },
    );
  }

  /// The panel's commands by name, by the same road a press takes.
  void _menu(BuildContext context, Offset at, LumitState app, LumitUiState ui,
      int picked) {
    const middle = Offset(0.5, 0.5);
    final live = picked > 0;
    final on = ui.workspace.interface;
    showMenuEntriesAt(context, at, [
      MenuEntry(l10n.alignAnchorToCentre,
          live ? () => ui.requestAlign(middle, anchor: true) : null),
      MenuEntry(l10n.alignCentreInComp,
          live ? () => ui.requestAlign(middle, anchor: false) : null),
      MenuEntry.divider(),
      ...fitRows(app, ui),
      MenuEntry.divider(),
      MenuEntry(l10n.alignDistributeAcross,
          picked < 3 ? null : () => ui.requestArrange(distribute: Axis.horizontal)),
      MenuEntry(l10n.alignDistributeDown,
          picked < 3 ? null : () => ui.requestArrange(distribute: Axis.vertical)),
      MenuEntry.divider(),
      MenuEntry(l10n.settingsAlignToFirstPicked, () {
        on.alignToFirstPicked = !on.alignToFirstPicked;
        ui.workspace.settingsChanged();
      }, checked: on.alignToFirstPicked),
    ]);
  }
}

/// A small icon button, the panel's one kind.
Widget _iconButton({
  required Key key,
  required String tip,
  required String glyph,
  required VoidCallback? onPressed,
  required double size,
}) =>
    SizedBox.square(
      dimension: size,
      child: LumitTooltip(
        message: tip,
        child: HouseButton(
          key: key,
          frameless: true,
          small: true,
          padding: EdgeInsets.zero,
          onPressed: onPressed,
          child: Center(child: LumitIcon(glyph)),
        ),
      ),
    );

/// Fit to comp, width and height, under the composition's grid.
class _FitButtons extends StatelessWidget {
  final double side;
  final LumitState app;
  final LumitUiState ui;

  const _FitButtons({required this.side, required this.app, required this.ui});

  @override
  Widget build(BuildContext context) {
    final rows = fitRows(app, ui);
    final cell = side / 3;
    Widget button(int row, String id, String glyph) => _iconButton(
          key: ValueKey<String>('align-fit-$id'),
          tip: rows[row].label ?? '',
          glyph: glyph,
          onPressed: rows[row].onPressed,
          size: math.min(cell, _footHeight),
        );
    return SizedBox(
      width: side,
      height: _footHeight,
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          button(0, 'comp', LumitIcons.fitToComp),
          button(2, 'width', LumitIcons.fitToCompWidth),
          button(3, 'height', LumitIcons.fitToCompHeight),
        ],
      ),
    );
  }
}

/// Six buttons lining the selection up with itself, and two spacing it out,
/// taking up the room a grid of dots does.
class _SelectionGrid extends StatelessWidget {
  final double side;
  final LumitUiState ui;

  /// How many layers with a box are picked.
  final int picked;

  const _SelectionGrid(
      {required this.side, required this.ui, required this.picked});

  @override
  Widget build(BuildContext context) {
    // The same size as the fit buttons, so the columns meet the dots beside.
    final cell = math.min(side / 3, _footHeight);
    Widget align(String id, String tip, String glyph, double? x, double? y) =>
        _iconButton(
          key: ValueKey<String>('align-selection-$id'),
          tip: tip,
          glyph: glyph,
          onPressed:
              picked < 2 ? null : () => ui.requestArrange(x: x, y: y),
          size: cell,
        );
    Widget distribute(String id, String tip, String glyph, Axis axis) =>
        _iconButton(
          key: ValueKey<String>('align-distribute-$id'),
          tip: tip,
          glyph: glyph,
          onPressed:
              picked < 3 ? null : () => ui.requestArrange(distribute: axis),
          size: cell,
        );
    return SizedBox.square(
      dimension: side,
      child: Column(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
            align('left', l10n.viewerAlignLeft, LumitIcons.alignEdgeLeft, 0, null),
            align('centre', l10n.viewerAlignCentre, LumitIcons.alignEdgeCentre,
                0.5, null),
            align('right', l10n.viewerAlignRight, LumitIcons.alignEdgeRight, 1,
                null),
          ]),
          Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
            align('top', l10n.viewerAlignTop, LumitIcons.alignEdgeTop, null, 0),
            align('middle', l10n.viewerAlignMiddle, LumitIcons.alignEdgeMiddle,
                null, 0.5),
            align('bottom', l10n.viewerAlignBottom, LumitIcons.alignEdgeBottom,
                null, 1),
          ]),
          Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
            distribute('across', l10n.alignDistributeAcross,
                LumitIcons.distributeAcross, Axis.horizontal),
            distribute('down', l10n.alignDistributeDown,
                LumitIcons.distributeDown, Axis.vertical),
            SizedBox.square(dimension: cell),
          ]),
        ],
      ),
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
