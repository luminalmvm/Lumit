// The keyed rows of the box picked on a node graph's canvas, or of every box
// with nothing picked, as lanes under the Timeline's ruler, so their keys can
// be seen, moved and eased as a layer's are. The lanes and everything done to
// them are the Effect controls key strip's ([FxKeyStripHost], [FxKeyLane]),
// laid out on the ruler's axis.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';

import '../l10n/engine_labels.dart';
import '../state/dock.dart' show Panel;
import '../widgets/controls.dart' show ThemeScope;
import 'effect_param_row_frb.dart' show cachedListParameters, effectLabelOf;
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

  /// The picked Fx box, or every one with nothing picked, read when the pick
  /// moves or the document commits and never in a build.
  List<BridgeEffectInstanceInfo> _boxes = const [];

  /// Whether the lanes are the one picked box's, which need no box name.
  bool _picked = false;

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
    var boxes = <BridgeEffectInstanceInfo>[];
    try {
      for (final instance in widget.comp.getNodeGraphInstances()) {
        if (picked == null || instance.id() == picked.id) {
          boxes.add(instance.getInfo());
        }
      }
    } catch (_) {
      // The comp or the box has gone; there is nothing to draw.
      boxes = [];
    }
    setState(() {
      _boxes = boxes;
      _picked = picked != null;
    });
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

  /// [box]'s rows with keys: a lane with nothing on it is a row to scroll
  /// past on the way to the canvas.
  List<(BridgeEffectInstanceInfo, BridgeParamInfo, List<BridgeScalar>)>
      _keyedRows(BridgeEffectInstanceInfo box) {
    final values = {for (final v in box.values) v.id: v.value};
    return [
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
          (box, param, scalars),
    ];
  }

  @override
  Widget build(BuildContext context) {
    final rows = [for (final box in _boxes) ..._keyedRows(box)];
    if (rows.isEmpty) return const SizedBox.shrink();
    // With nothing picked each lane says whose it is.
    String label(BridgeEffectInstanceInfo box, BridgeParamInfo param) {
      final own = engineLabel(param.label);
      return _picked
          ? own
          : '${box.customName ?? effectLabelOf(box.name)} · $own';
    }
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
          for (final (box, param, scalars) in rows)
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
                            t.propertyCase(label(box, param)),
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
