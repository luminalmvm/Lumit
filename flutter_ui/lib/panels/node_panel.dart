// The Node panel: the parameter rows of whichever box the Graph panel has
// picked (K-471, the approved Nodes-workspace drawing).
//
// **In plain terms.** The Graph panel draws boxes. Click one and this panel
// lists what that box can be set to — the same rows the Effect controls panel
// draws for an effect, but for one box at a time and for *drivers* too, which
// the effect stack has no place for. The header says which box is picked and
// how many of its parameters a wire has taken over.
//
// **Why it is not simply Effect controls.** That panel is the whole stack, in
// stack order, with Transform and Source above it; this one answers a
// different question — "what is selected on the canvas" — and the selection it
// follows names boxes (drivers, the Source, the Layer out) that a stack list
// cannot. The *rows* are shared: [EffectParamRowFrb] draws them, so a driven
// row here and a driven row in Effect controls are the same widget.
//
// **How an edit reaches the document.** Exactly as the stack's does: the
// staged-instance path (docs/impl/node-graph.md §5). An effect box rides
// [EffectStackEditor] — stage on a fresh handle, commit the whole stack on
// release, one `SetLayerEffects`. A driver box stages the same way and commits
// `setGraph(drivers, wiring)`, one `SetLayerGraph`, one undo step.

import 'package:flutter/foundation.dart' show mapEquals;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart';
import 'graph_panel.dart' show graphNodeKey, graphToolbarHeight;
import 'placeholder.dart';

/// The box the panel is drawing: which instance it is, whether it lives in the
/// graph's driver list rather than in the effect stack, and the read model it
/// was headed and filled from.
class _Picked {
  final bool driver;
  final BridgeEffectInstanceInfo info;

  const _Picked({required this.driver, required this.info});
}

class NodePanelFrb extends StatefulWidget {
  const NodePanelFrb({super.key});

  @override
  State<NodePanelFrb> createState() => _NodePanelFrbState();
}

class _NodePanelFrbState extends State<NodePanelFrb> {
  LumitUiState? _ui;
  LayerReference? _layer;

  /// The picked box's instance, read at the three moments it can change — the
  /// pick moves, the layer changes, the document commits. Never in a build:
  /// this panel redraws on every playhead frame, and a read in that path is
  /// the traffic `bridge_call_budget_test` guards against.
  _Picked? _picked;

  /// Which of the picked box's parameters a driver is wired to, by parameter
  /// id, with the driver's name and what its wire carries — the same shape
  /// Effect controls holds, keyed by parameter alone because there is only
  /// ever one box here.
  Map<String, ({String driver, BridgePortType type})> _driven = const {};

  /// The drag in flight on an effect box: staged, previewed, committed on
  /// release as one op.
  final EffectStackEditor _stack = EffectStackEditor();

  /// The same for a driver box, which cannot ride [EffectStackEditor] because
  /// its commit is `setGraph` rather than `setEffects`.
  ///
  /// ponytail: no live preview while a driver's number is dragged — the
  /// preview call takes a staged *stack*, and there is no staged-graph render
  /// yet. The field under the pointer shows the staged value and the release
  /// commits; add the preview when `renderFrameWithPreview` learns to carry a
  /// staged graph.
  ({String param, BridgeEffectValue value})? _stagedDriver;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (identical(ui, _ui)) return;
    _unbind();
    _ui = ui;
    ui.selectedLayer.addListener(_reload);
    ui.graphNode.addListener(_reload);
    ui.model.addListener(_reload);
    _reload();
  }

  void _unbind() {
    _ui?.selectedLayer.removeListener(_reload);
    _ui?.graphNode.removeListener(_reload);
    _ui?.model.removeListener(_reload);
  }

  @override
  void dispose() {
    _unbind();
    super.dispose();
  }

  /// The one read. Everything the rows draw comes from here.
  void _reload() {
    if (!mounted) return;
    final layer = _ui?.selectedLayer.value;
    final node = _ui?.graphNode.value;
    final (UuidValue? id, bool driver) = switch (node) {
      BridgeNodeRef_Effect(:final field0) => (field0, false),
      BridgeNodeRef_Driver(:final field0) => (field0, true),
      // The Source and the Layer out are derived boxes: they carry ports, not
      // parameters, so there is nothing here to list for them.
      _ => (null, false),
    };
    _Picked? picked;
    var driven = const <String, ({String driver, BridgePortType type})>{};
    if (layer != null && id != null) {
      try {
        for (final instance
            in driver ? layer.getGraphDrivers() : layer.getEffects()) {
          if (instance.id() == id) {
            picked = _Picked(driver: driver, info: instance.getInfo());
            break;
          }
        }
        if (picked != null) driven = _drivenOf(layer, node!);
      } catch (_) {
        // The layer has gone since the pick was made; the placeholder is the
        // honest answer until the selection catches up.
      }
    }
    setState(() {
      _layer = layer;
      _picked = picked;
      _stagedDriver = null;
      if (!mapEquals(driven, _driven)) _driven = driven;
    });
  }

  /// Which of [node]'s parameters a wire is feeding, by parameter id. A wire's
  /// colour is its **source** port's type — what the parameter is now
  /// following — which is the same reading Effect controls takes.
  Map<String, ({String driver, BridgePortType type})> _drivenOf(
    LayerReference layer,
    BridgeNodeRef node,
  ) {
    final out = <String, ({String driver, BridgePortType type})>{};
    final graph = layer.getGraph();
    final byRef = {for (final n in graph.nodes) graphNodeKey(n.node): n};
    final want = graphNodeKey(node);
    for (final edge in graph.wiring.edges) {
      if (edge.to case BridgeInputRef_Param(node: final to, :final port)) {
        if (graphNodeKey(to) != want) continue;
        final (fromKey, fromPort) = switch (edge.from) {
          BridgeOutputRef_Driver(node: final d, port: final p) => (
              graphNodeKey(BridgeNodeRef.driver(d)),
              p
            ),
          BridgeOutputRef_SourceMatte() => ('source', 'matte'),
        };
        final source = byRef[fromKey];
        if (source == null) continue;
        final socket = source.outputs.where((o) => o.id == fromPort);
        if (socket.isEmpty) continue;
        out[port] = (
          driver: source.customName ?? engineLabel(source.label),
          type: socket.first.portType,
        );
      }
    }
    return out;
  }

  // --- Writing -------------------------------------------------------------

  /// A release, or a typed value: one op, one undo step.
  void _write(UuidValue effect, String param, BridgeEffectValue value) {
    final layer = _layer;
    if (layer == null) return;
    if (_picked?.driver ?? false) {
      _stagedDriver = (param: param, value: value);
      try {
        final drivers = layer.getGraphDrivers();
        for (final instance in drivers) {
          if (instance.id() == effect) {
            instance.setValue(id: param, value: value);
          }
        }
        layer.setGraph(drivers: drivers, wiring: layer.getGraph().wiring);
      } catch (_) {
        // The graph changed under us, or the edit was refused (§1.5);
        // re-reading is the recovery.
      }
      _stagedDriver = null;
    } else {
      _stack.write(layer, effect, param, value);
    }
    _ui?.model.refresh();
  }

  /// A drag tick: show it, do not commit it.
  void _live(UuidValue effect, String param, BridgeEffectValue value) {
    final layer = _layer;
    final ui = _ui;
    if (layer == null || ui == null) return;
    if (_picked?.driver ?? false) {
      setState(() => _stagedDriver = (param: param, value: value));
      return;
    }
    final comp = ui.selectedComp;
    if (comp == null) return;
    setState(() => _stack.live(comp, layer, effect, param, value,
        frame: ui.playheadFrame.value, scale: ui.viewerScale));
  }

  /// What a row should *show*, which during a drag is the staged value.
  BridgeEffectValue? _staged(UuidValue effect, String param) {
    final driver = _stagedDriver;
    if (_picked?.driver ?? false) {
      return driver != null && driver.param == param ? driver.value : null;
    }
    return _stack.stagedValue(effect, param);
  }

  // --- Drawing -------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final picked = _picked;
    final layer = _layer;
    final comp = ui.selectedComp;
    if (picked == null || layer == null || comp == null) {
      return PlaceholderPanel(
        icon: LumitIcon.nodes,
        title: l10n.panelNode,
        hint: l10n.nodeNoSelection,
      );
    }
    // The keyframe controls read the playhead — which key is under it, whether
    // the diamond is filled — so the rows redraw when it moves.
    return ValueListenableBuilder<int>(
      valueListenable: ui.playheadFrame,
      builder: (context, playhead, _) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _header(context, picked),
          Expanded(child: _rows(ui, picked, layer, playhead)),
        ],
      ),
    );
  }

  /// The panel's own strip: the kicker, the box's name, and how many of its
  /// parameters a wire has taken over. The dock draws the tab bar above it,
  /// and the strip matches the Graph panel's in height and colour so the two
  /// columns of the workspace line up.
  Widget _header(BuildContext context, _Picked picked) {
    final t = ThemeScope.of(context).theme;
    return Container(
      key: const ValueKey('node-header'),
      height: graphToolbarHeight,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 10),
      child: Row(
        children: [
          Expanded(
            child: Text(
              picked.info.customName ?? engineLabel(picked.info.name),
              key: const ValueKey('node-name'),
              style: t.body,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          if (_driven.isNotEmpty)
            Text(
              l10n.nodeDrivenCount(_driven.length),
              key: const ValueKey('node-driven-count'),
              style: t.kicker,
            ),
        ],
      ),
    );
  }

  Widget _rows(
    LumitUiState ui,
    _Picked picked,
    LayerReference layer,
    int playhead,
  ) {
    final id = picked.info.id;
    final values = {for (final v in picked.info.values) v.id: v.value};
    return ListView(
      padding: const EdgeInsets.symmetric(vertical: 4),
      children: [
        for (final param in cachedListParameters(picked.info.name))
          EffectParamRowFrb(
            key: ValueKey<String>('node-row-$id-${param.id}'),
            effectId: id,
            param: param,
            value: _staged(id, param.id) ?? values[param.id],
            comp: ui.selectedComp!,
            ownerLayerId: layer.internallayerId,
            ownerLayers: ui.model.layers,
            playheadFrame: playhead,
            onSeek: (frame) => ui.playheadFrame.value = frame,
            onWrite: _write,
            onLive: _live,
            twoColumn: true,
            siblings: values,
            driven: _driven[param.id],
          ),
      ],
    );
  }
}
