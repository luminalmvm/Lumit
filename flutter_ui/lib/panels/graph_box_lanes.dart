// The keyed rows of the box picked on a node graph's canvas, as lanes under
// the Timeline's ruler, so its keys can be seen, moved and eased as a layer's
// are. The lanes and everything done to them are the Effect controls key
// strip's ([FxKeyStripHost], [FxKeyLane]), laid out on the ruler's axis.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';

import '../l10n/engine_labels.dart';
import '../state/dock.dart' show Panel;
import '../widgets/controls.dart' show ThemeScope;
import 'effect_param_row_frb.dart' show cachedListParameters;
import 'fx_key_strip.dart';

class GraphBoxLanes extends StatefulWidget {
  final CompositionReference comp;
  final int frames;

  /// The ruler's width, which the lanes share so a key sits under its frame.
  final double width;

  const GraphBoxLanes({
    super.key,
    required this.comp,
    required this.frames,
    required this.width,
  });

  @override
  State<GraphBoxLanes> createState() => _GraphBoxLanesState();
}

class _GraphBoxLanesState extends State<GraphBoxLanes> {
  LumitUiState? _ui;

  /// The picked Fx box, read when the pick moves or the document commits and
  /// never in a build.
  BridgeEffectInstanceInfo? _box;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (identical(ui, _ui)) return;
    _unbind();
    _ui = ui;
    ui.compGraphNode.addListener(_reload);
    ui.model.addListener(_reload);
    _reload();
  }

  @override
  void didUpdateWidget(GraphBoxLanes old) {
    super.didUpdateWidget(old);
    if (old.comp != widget.comp) _reload();
  }

  void _unbind() {
    _ui?.compGraphNode.removeListener(_reload);
    _ui?.model.removeListener(_reload);
  }

  @override
  void dispose() {
    _unbind();
    super.dispose();
  }

  void _reload() {
    if (!mounted) return;
    final picked = _ui?.compGraphNode.value;
    BridgeEffectInstanceInfo? box;
    if (picked != null) {
      try {
        for (final instance in widget.comp.getNodeGraphInstances()) {
          if (instance.id() == picked.id) {
            box = instance.getInfo();
            break;
          }
        }
      } catch (_) {
        // The comp or the box has gone; there is nothing to draw.
      }
    }
    setState(() => _box = box);
  }

  /// One parameter's channels into the value the box stores: four for a
  /// colour, one otherwise. The Effect controls row's own write.
  void _write(
      BridgeEffectInstanceInfo box, String param, List<BridgeScalar> next) {
    final value = next.length == 4
        ? BridgeEffectValue.colour(
            BridgeColour(r: next[0], g: next[1], b: next[2], a: next[3]))
        : BridgeEffectValue.float(next.single);
    final comp = widget.comp;
    try {
      final instances = comp.getNodeGraphInstances();
      for (final instance in instances) {
        if (instance.id() == box.id) instance.setValue(id: param, value: value);
      }
      comp.setNodeGraph(
          instances: instances, wiring: comp.getNodeGraph().wiring);
    } catch (_) {
      // The graph changed under the gesture; re-reading is the recovery.
    }
    _ui?.model.refresh();
  }

  @override
  Widget build(BuildContext context) {
    final box = _box;
    if (box == null) return const SizedBox.shrink();
    final values = {for (final v in box.values) v.id: v.value};
    // Only the rows with keys: a lane with nothing on it is a row to scroll
    // past on the way to the canvas.
    final rows = <(BridgeParamInfo, List<BridgeScalar>)>[
      for (final param in [
        ...cachedListParameters(box.name),
        ...box.derivedParams,
      ])
        if (switch (values[param.id]) {
          BridgeEffectValue_Float(:final field0) => [field0],
          BridgeEffectValue_Colour(:final field0) => [
              field0.r,
              field0.g,
              field0.b,
              field0.a,
            ],
          _ => null,
        }
            case final scalars?
            when scalars.first is BridgeScalar_Keyframed)
          (param, scalars),
    ];
    if (rows.isEmpty) return const SizedBox.shrink();
    final t = ThemeScope.of(context).theme;
    return FxKeyStripHost(
      spanStart: 0,
      spanFrames: widget.frames,
      width: widget.width,
      panel: Panel.timeline,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final (param, scalars) in rows)
            DecoratedBox(
              decoration: BoxDecoration(
                color: t.surface0,
                border: Border(bottom: BorderSide(color: t.hairline)),
              ),
              child: SizedBox(
                height: t.density.laneRow,
                child: Stack(
                  children: [
                    Positioned(
                      left: 8,
                      top: 0,
                      bottom: 0,
                      child: IgnorePointer(
                        child: Center(
                          child: Text(
                            t.propertyCase(engineLabel(param.label)),
                            style: t.small,
                          ),
                        ),
                      ),
                    ),
                    Positioned.fill(
                      child: FxKeyLane(
                        rowKey: '${box.id}-${param.id}',
                        scalars: scalars,
                        onWrite: (next) => _write(box, param.id, next),
                      ),
                    ),
                  ],
                ),
              ),
            ),
        ],
      ),
    );
  }
}
