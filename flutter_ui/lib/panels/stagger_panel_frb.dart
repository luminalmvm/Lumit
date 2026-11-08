// The Stagger panel: start the selected layers one after another, a set
// number of frames apart, or line them up on one frame again. Its order is
// also the one a stagger drag on bars or keys follows.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/menu_layer_frb.dart' show selectedEntries;
import '../state/dock.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'graph_maths.dart';
import 'layer_fold_frb.dart';
import 'stagger.dart';
import 'text_target.dart' show textCell;
import 'timeline_extras_frb.dart' show showMenuAt;

/// Put the Stagger panel up and in front: what Layer ▸ Stagger layers… and
/// the row menu do.
void showStaggerPanel(LumitUiState ui) {
  setPanelVisible(ui.split, Panel.stagger, true);
  activatePanelTab(ui.split, Panel.stagger);
  ui.activePane.value = Panel.stagger.pane();
  ui.workspace.touch();
}

String staggerOrderLabel(LayerStaggerOrder order) => switch (order) {
      LayerStaggerOrder.topDown => l10n.easeStaggerTopDown,
      LayerStaggerOrder.bottomUp => l10n.easeStaggerBottomUp,
      LayerStaggerOrder.clicked => l10n.staggerClicked,
      LayerStaggerOrder.random => l10n.staggerRandom,
    };

class StaggerPanelFrb extends StatefulWidget {
  const StaggerPanelFrb({super.key});

  @override
  State<StaggerPanelFrb> createState() => _StaggerPanelFrbState();
}

class _StaggerPanelFrbState extends State<StaggerPanelFrb> {
  late final LumitUiState _ui =
      Provider.of<LumitUiState>(context, listen: false);

  /// The step while its well is being dragged, kept off the workspace store
  /// until the drag ends.
  int? _live;

  /// The overlap while its well is being dragged, the same way.
  int? _liveOverlap;

  /// Stagger the picked layers, or line them up with [lineUp]. All of it, the
  /// moves and the fades, is one undo step.
  void _apply({bool lineUp = false}) {
    final comp = _ui.selectedComp;
    final picked = [
      for (final entry in selectedEntries(_ui))
        if (!entry.info.switches.locked) entry,
    ];
    if (comp == null || picked.length < 2) return;
    final workspace = _ui.workspace;
    final ordered = inStaggerOrder(picked, workspace.staggerOrder,
        clicked: _ui.selectionInClickOrder,
        layerOf: (e) => e.layer.internallayerId);
    final overlap = workspace.staggerOverlap;
    final ids = [for (final e in ordered) e.layer.internallayerId];
    final spans = [for (final e in ordered) (e.info.inFrame, e.info.outFrame)];
    final earliest =
        spans.map((s) => s.$1).reduce((a, b) => a < b ? a : b);
    final starts = lineUp
        ? [for (final _ in spans) earliest]
        : overlap > 0
            ? endToEndStarts(spans, overlap)
            : [
                for (var i = 0; i < spans.length; i++)
                  earliest + i * workspace.staggerStep,
              ];
    final project = Provider.of<LumitState>(context, listen: false).project;
    asOneUndoStep(project, () {
      try {
        comp.fanLayers(layerIds: ids, deltas: [
          for (var i = 0; i < spans.length; i++) starts[i] - spans[i].$1,
        ]);
      } catch (_) {
        return;
      }
      if (!workspace.staggerCrossfade || overlap < 1) return;
      final placed = [
        for (var i = 0; i < ordered.length; i++)
          (
            entry: ordered[i],
            start: starts[i],
            end: starts[i] + spans[i].$2 - spans[i].$1,
          ),
      ];
      if (lineUp) {
        for (final p in placed) {
          _fade(p.entry, p.start, p.start + overlap, rising: true);
          _fade(p.entry, p.end - overlap, p.end, rising: false);
        }
        return;
      }
      final stack = [for (final e in _ui.model.layers) e.layer.internallayerId];
      int rowOf(BridgeLayerEntry e) => stack.indexOf(e.layer.internallayerId);
      for (var i = 1; i < placed.length; i++) {
        final (going, coming) = (placed[i - 1], placed[i]);
        if (coming.start >= going.end) continue;
        // The one on top is the one that has to fade for the other to show:
        // whichever way round they are, the picture is the same dissolve.
        if (rowOf(coming.entry) < rowOf(going.entry)) {
          _fade(coming.entry, coming.start, going.end, rising: true);
        } else {
          _fade(going.entry, coming.start, going.end, rising: false);
        }
      }
    });
    Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
  }

  /// Opacity keys taking [entry] up from nothing or down to it across two
  /// frames, in a straight line, in place of any keys it has between them. The
  /// level it fades to or from is what it holds there now.
  void _fade(BridgeLayerEntry entry, int from, int to, {required bool rising}) {
    if (to <= from) return;
    final (fpsNum, fpsDen) = _ui.model.fpsExact;
    final fps = _ui.model.fps;
    final BridgeScalar opacity;
    try {
      opacity = entry.layer.getTransform().opacity;
    } catch (_) {
      return;
    }
    if (opacity is BridgeScalar_Expression) return;
    double frameOf(BridgeKeyframe key) => rationalSeconds(key.time) * fps;
    final level = evaluateScalar(opacity, (rising ? to : from) / fps);
    BridgeKeyframe key(int frame, double value) => BridgeKeyframe(
          time: timeOfSubframe(frame.toDouble(), fpsNum, fpsDen),
          value: value,
          interpIn: const BridgeSideInterp.linear(),
          interpOut: const BridgeSideInterp.linear(),
        );
    final keys = [
      for (final k in keysOf(opacity))
        if (frameOf(k) < from || frameOf(k) > to) k,
      key(from, rising ? 0 : level),
      key(to, rising ? level : 0),
    ]..sort((a, b) => frameOf(a).compareTo(frameOf(b)));
    try {
      entry.layer.setTransform(
          prop: BridgeTransformProp.opacity,
          value: BridgeScalar.keyframed(keys));
    } catch (_) {}
  }

  void _setStep(num v) {
    setState(() => _live = null);
    _ui.workspace.setStagger(step: v.round());
  }

  void _setOverlap(num v) {
    setState(() => _liveOverlap = null);
    _ui.workspace.setStagger(overlap: v.round());
  }

  /// A frames well, as Offset and Overlap both are.
  Widget _frames(LumitTheme t, String key, int value, int resetTo,
          void Function(int?) live, void Function(num) set) =>
      Row(
        children: [
          SizedBox(
            width: 64,
            child: DragValueField(
              key: ValueKey<String>(key),
              value: value,
              min: 0,
              max: 100000,
              decimals: 0,
              resetTo: resetTo,
              onChangeLive: (v) => setState(() => live(v.round())),
              onChangeEnd: set,
              onDragCancel: () => setState(() => live(null)),
              onChanged: set,
            ),
          ),
          const SizedBox(width: 6),
          Text(l10n.unitFramesCount(value),
              style: t.body.copyWith(color: t.textMuted)),
        ],
      );

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: Listenable.merge([_ui.model, _ui.selectedLayers]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        final workspace = _ui.workspace;
        final overlap = _liveOverlap ?? workspace.staggerOverlap;
        final live =
            selectedEntries(_ui).where((e) => !e.info.switches.locked).length >
                1;
        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onSecondaryTapDown: (d) => showMenuAt<void>(
            context: context,
            position: d.globalPosition,
            rows: (close) {
              void run(void Function() act) {
                close(null);
                act();
              }

              return [
                MenuRow(
                  key: const ValueKey('stagger-menu-apply'),
                  enabled: live,
                  onPressed: () => run(_apply),
                  child: MenuLabel(l10n.staggerApply, enabled: live),
                ),
                MenuRow(
                  key: const ValueKey('stagger-menu-line-up'),
                  enabled: live,
                  onPressed: () => run(() => _apply(lineUp: true)),
                  child: MenuLabel(l10n.staggerLineUp, enabled: live),
                ),
                const MenuDivider(),
                MenuRow(
                  key: const ValueKey('stagger-menu-reset'),
                  onPressed: () => run(() => setState(() => workspace.setStagger(
                      step: 1,
                      order: LayerStaggerOrder.topDown,
                      overlap: 0,
                      crossfade: false))),
                  child: Text(l10n.reset),
                ),
              ];
            },
          ),
          child: Container(
          color: t.surface0,
          child: ListView(
            padding: const EdgeInsets.only(top: 6),
            children: [
              // An overlap lays the layers end to end, which leaves the offset
              // nothing to say.
              IgnorePointer(
                ignoring: overlap > 0,
                child: Opacity(
                  opacity: overlap > 0 ? 0.4 : 1,
                  child: textCell(
                    t,
                    l10n.staggerOffset,
                    _frames(t, 'stagger-step', _live ?? workspace.staggerStep,
                        1, (v) => _live = v, _setStep),
                  ),
                ),
              ),
              textCell(
                t,
                l10n.staggerOverlap,
                _frames(t, 'stagger-overlap', overlap, 0,
                    (v) => _liveOverlap = v, _setOverlap),
              ),
              textCell(
                t,
                l10n.staggerCrossfade,
                HouseCheckbox(
                  key: const ValueKey('stagger-crossfade'),
                  value: workspace.staggerCrossfade,
                  onChanged: overlap > 0
                      ? (on) => setState(
                          () => workspace.setStagger(crossfade: on))
                      : null,
                ),
              ),
              textCell(
                t,
                l10n.staggerOrder,
                SizedBox(
                  width: 120,
                  child: BareDropdown<LayerStaggerOrder>(
                    key: const ValueKey('stagger-order'),
                    dense: true,
                    value: workspace.staggerOrder,
                    options: LayerStaggerOrder.values,
                    label: staggerOrderLabel,
                    onChanged: (order) =>
                        setState(() => workspace.setStagger(order: order)),
                  ),
                ),
              ),
              Padding(
                // Under the values, so the buttons read as what the two rows
                // above are for.
                padding:
                    const EdgeInsetsDirectional.fromSTEB(102, 8, 10, 8),
                child: Row(
                  children: [
                    HouseButton(
                      key: const ValueKey('stagger-apply'),
                      onPressed: live ? _apply : null,
                      child: Text(l10n.staggerApply),
                    ),
                    const SizedBox(width: 8),
                    HouseButton(
                      key: const ValueKey('stagger-line-up'),
                      onPressed: live ? () => _apply(lineUp: true) : null,
                      child: Text(l10n.staggerLineUp),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
        );
      },
    );
  }
}
