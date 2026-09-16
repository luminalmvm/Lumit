// The Node panel: the parameter rows of whichever box the Graph panel has
// picked (the approved Nodes-workspace drawing).
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
//
// **The third subject** (docs/impl/node-graph-comp.md §4.3). When the fronted
// composition is a node graph the panel follows that canvas's pick instead: an
// Fx box draws the same rows and commits `setNodeGraph`, an Input box draws the
// five facts its declaration carries as a form, a Read box its item, and the
// Output box nothing at all.

import 'dart:typed_data' show Float64List;

import 'package:flutter/foundation.dart' show mapEquals;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/comp_graph.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/track.dart' show fireEffectAction;
import 'package:lumit_flutter/src/rust/lib.dart' show F64Array4;
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/preview_throttle.dart';
import '../widgets/controls.dart';
import 'comp_graph_panel.dart'
    show compInputKindWord, compInputKinds, compItemKindWord;
import 'effect_param_row_frb.dart';
import 'graph_panel.dart'
    show graphCompById, graphNodeKey, graphNoStream, graphToolbarHeight;
import 'placeholder.dart';
import 'shader_editor.dart' show ShaderHome, pressShaderButton;

/// The box the panel is drawing: which instance it is, whether it lives in the
/// graph's driver list rather than in the effect stack, and the read model it
/// was headed and filled from.
class _Picked {
  final bool driver;

  /// The box is in a **node graph composition** rather than on a layer, so its
  /// edits commit through `setNodeGraph` and preview through the graph's own
  /// request. The two share every other line, which is why this is a mark on
  /// the same record rather than a second one.
  final bool graph;
  final BridgeEffectInstanceInfo info;

  const _Picked({
    required this.driver,
    required this.info,
    this.graph = false,
  });
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
  Map<String, ({String driver, BridgePortType type, bool noStream})> _driven =
      const {};

  /// The node graph box the panel is drawing, when the fronted composition is
  /// a node graph, and the Input declaration behind it. Read at the same three
  /// moments the instance is, and never in a build.
  BridgeCompNode? _box;
  BridgeGraphInput? _input;

  /// The drag in flight on an effect box: staged, previewed, committed on
  /// release as one op.
  final EffectStackEditor _stack = EffectStackEditor();

  /// The same for a driver box, which cannot ride [EffectStackEditor] because
  /// its commit is `setGraph` rather than `setEffects` — and its preview is
  /// `renderFrameWithDriverPreview`, which stages the graph's nodes exactly as
  /// the stack preview stages the effect list.
  ({String param, BridgeEffectValue value})? _stagedDriver;

  /// The Input label being edited, so typing is not fought by a reload. Its
  /// text is set when the picked box changes, and again when the document's
  /// label moves under a field nobody is typing in (an undo, say).
  final TextEditingController _label = TextEditingController();
  final FocusNode _labelFocus = FocusNode(debugLabel: 'input label');
  UuidValue? _labelFor;

  /// The Input form's number well being dragged, so the drag shows where it
  /// has got to without committing a step per tick.
  ({String key, double value})? _numberDrag;

  /// The driver drag's own rate limit, the twin of the one inside
  /// [EffectStackEditor]: the first tick goes at once and the newest of those
  /// that follow goes when the interval is up, so a fast drag cannot queue up
  /// renders the pointer has already outrun.
  final PreviewThrottle _driverPreview = PreviewThrottle();

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (identical(ui, _ui)) return;
    _unbind();
    _ui = ui;
    ui.selectedLayer.addListener(_reload);
    ui.graphNode.addListener(_reload);
    ui.compGraphNode.addListener(_reload);
    ui.model.addListener(_reload);
    _reload();
  }

  void _unbind() {
    _ui?.selectedLayer.removeListener(_reload);
    _ui?.graphNode.removeListener(_reload);
    _ui?.compGraphNode.removeListener(_reload);
    _ui?.model.removeListener(_reload);
  }

  @override
  void dispose() {
    _unbind();
    // A held tick would fire into a panel that is gone.
    _driverPreview.cancel();
    _stack.clear();
    _label.dispose();
    _labelFocus.dispose();
    super.dispose();
  }

  /// The one read. Everything the rows draw comes from here.
  void _reload() {
    if (!mounted) return;
    final ui = _ui;
    final comp = ui?.selectedComp;
    if (ui != null && comp != null && ui.model.isNodeGraph) {
      _reloadGraphBox(ui, comp);
      return;
    }
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
    var driven =
        const <String, ({String driver, BridgePortType type, bool noStream})>{};
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
      _box = null;
      _input = null;
      _stagedDriver = null;
      if (!mapEquals(driven, _driven)) _driven = driven;
    });
  }

  /// The same read for a **node graph's** pick: the box, its Input declaration
  /// where it has one, and, for an Fx box, the staged instance whose rows this
  /// panel draws.
  void _reloadGraphBox(LumitUiState ui, CompositionReference comp) {
    final picked = ui.compGraphNode.value;
    BridgeCompNode? box;
    BridgeGraphInput? input;
    _Picked? row;
    var driven =
        const <String, ({String driver, BridgePortType type, bool noStream})>{};
    if (picked != null) {
      try {
        final graph = comp.getNodeGraph();
        box = graph.nodes.where((n) => n.id == picked.id).firstOrNull;
        input = graph.wiring.inputs
            .where((i) => i.id == picked.id)
            .map((i) => i.input)
            .firstOrNull;
        if (box?.kind == BridgeCompNodeKind.fx) {
          for (final instance in comp.getNodeGraphInstances()) {
            if (instance.id() == picked.id) {
              row = _Picked(driver: false, graph: true, info: instance.getInfo());
              break;
            }
          }
          driven = _drivenInGraph(graph, picked.id);
        }
      } catch (_) {
        // The comp or the box has gone since the pick was made; the
        // placeholder is the honest answer until the canvas catches up.
      }
    }
    // A new pick fills the field; so does the document's label moving while
    // nobody is typing in it. Without the second test an undo left the old
    // text sitting there, and the next lost-focus submit wrote it back.
    if (input != null &&
        (_labelFor != picked?.id ||
            (!_labelFocus.hasFocus && _label.text != input.label))) {
      _label.text = input.label;
      _labelFor = picked?.id;
    }
    if (input == null) _labelFor = null;
    setState(() {
      _layer = null;
      _picked = row;
      _box = box;
      _input = input;
      _stagedDriver = null;
      if (!mapEquals(driven, _driven)) _driven = driven;
    });
  }

  /// Which of this box's parameters a wire is feeding, by socket id: the same
  /// decoration the layer's graph draws, read off the node graph's own wires.
  Map<String, ({String driver, BridgePortType type, bool noStream})>
      _drivenInGraph(BridgeCompGraph graph, UuidValue id) {
    final out =
        <String, ({String driver, BridgePortType type, bool noStream})>{};
    final byId = {for (final n in graph.nodes) n.id: n};
    for (final edge in graph.wiring.edges) {
      if (edge.to != id) continue;
      final source = byId[edge.from];
      if (source == null) continue;
      final socket = source.outputs.where((o) => o.id == edge.fromPort);
      if (socket.isEmpty) continue;
      out[edge.toPort] = (
        driver: source.customName ?? engineLabel(source.label),
        type: socket.first.portType,
        noStream: false,
      );
    }
    return out;
  }

  /// Which of [node]'s parameters a wire is feeding, by parameter id. A wire's
  /// colour is its **source** port's type — what the parameter is now
  /// following — which is the same reading Effect controls takes.
  Map<String, ({String driver, BridgePortType type, bool noStream})> _drivenOf(
    LayerReference layer,
    BridgeNodeRef node,
  ) {
    final out =
        <String, ({String driver, BridgePortType type, bool noStream})>{};
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
          // A points wire's source is a *stack effect*.
          BridgeOutputRef_EffectData(:final effect, :final port) => (
              graphNodeKey(BridgeNodeRef.effect(effect)),
              port
            ),
        };
        final source = byRef[fromKey];
        if (source == null) continue;
        final socket = source.outputs.where((o) => o.id == fromPort);
        if (socket.isEmpty) continue;
        out[port] = (
          driver: source.customName ?? engineLabel(source.label),
          type: socket.first.portType,
          noStream: graphNoStream(source),
        );
      }
    }
    return out;
  }

  // --- Writing -------------------------------------------------------------

  /// A release, or a typed value: one op, one undo step.
  void _write(UuidValue effect, String param, BridgeEffectValue value) {
    if (_picked?.graph ?? false) {
      // A node graph box: the same staged-instance path, committed by the
      // graph's own op with the wiring exactly as it stands.
      final comp = _ui?.selectedComp;
      if (comp == null) return;
      _driverPreview.cancel();
      _stagedDriver = (param: param, value: value);
      try {
        comp.setNodeGraph(
          instances: _graphInstancesWith(comp, effect),
          wiring: comp.getNodeGraph().wiring,
        );
      } catch (_) {
        // The graph changed under us, or the edit was refused; re-reading is
        // the recovery.
      }
      _stagedDriver = null;
      _ui?.model.refresh();
      return;
    }
    final layer = _layer;
    if (layer == null) return;
    if (_picked?.driver ?? false) {
      // A release ends the drag: a held preview tick would render provisional
      // values *after* the commit, putting the pre-commit picture back up.
      _driverPreview.cancel();
      _stagedDriver = (param: param, value: value);
      try {
        layer.setGraph(
          drivers: _driversWith(layer, effect),
          wiring: layer.getGraph().wiring,
        );
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
    final ui = _ui;
    if (ui == null) return;
    if (_picked?.graph ?? false) {
      setState(() => _stagedDriver = (param: param, value: value));
      final graph = ui.selectedComp;
      if (graph == null) return;
      // Read inside the closure: a held tick must send the newest staged
      // value, not the one that was current when it was held.
      _driverPreview.request(() => graph.renderFrameWithGraphPreview(
            frame: BigInt.from(ui.playheadFrame.value),
            scale: ui.viewerScale,
            instances: _graphInstancesWith(graph, effect),
          ));
      return;
    }
    final layer = _layer;
    if (layer == null) return;
    final comp = ui.selectedComp;
    if (_picked?.driver ?? false) {
      setState(() => _stagedDriver = (param: param, value: value));
      if (comp == null) return;
      // The drivers are read *inside* the closure: a held tick must send the
      // newest staged value, not the one that was current when it was held.
      _driverPreview.request(() => comp.renderFrameWithDriverPreview(
            frame: BigInt.from(ui.playheadFrame.value),
            scale: ui.viewerScale,
            layer: layer,
            drivers: _driversWith(layer, effect),
          ));
      return;
    }
    if (comp == null) return;
    setState(() => _stack.live(comp, layer, effect, param, value,
        frame: ui.playheadFrame.value, scale: ui.viewerScale));
  }

  /// A press on one of the picked box's buttons, the same press Effect controls
  /// makes. A driver has no buttons, and a box in a node graph has no layer to
  /// send an engine event to, so there only the frontend's own buttons answer.
  void _press(_Picked picked, UuidValue effect, String param) {
    final ui = _ui;
    if (ui == null) return;
    final comp = ui.selectedComp;
    final layer = _layer;
    final frame = BigInt.from(ui.playheadFrame.value);
    final home = picked.graph
        ? comp == null
            ? null
            : ShaderHome.graph(comp,
                draw: (staged) => comp.renderFrameWithGraphPreview(
                    frame: frame, scale: ui.viewerScale, instances: staged))
        : layer == null
            ? null
            : ShaderHome.layer(layer,
                draw: comp == null
                    ? null
                    : (staged) => comp.renderFrameWithPreview(
                        frame: frame,
                        scale: ui.viewerScale,
                        layer: layer,
                        effects: staged));
    if (picked.info.name == 'custom_shader' &&
        home != null &&
        pressShaderButton(
          context: context,
          home: home,
          effect: effect,
          param: param,
          onApplied: ui.model.refresh,
        )) {
      return;
    }
    if (picked.info.name == 'node_graph' && param == 'open') {
      final project = Provider.of<LumitState>(context, listen: false).project;
      final inner = graphCompById(project, picked.info.nodeGraphComp);
      if (inner != null) ui.setSelectedComp(inner);
      return;
    }
    if (picked.graph || layer == null) return;
    try {
      fireEffectAction(
          layer: layer, effect: effect, param: param, frame: frame);
    } catch (_) {
      // Refused; the effect's own status line says why.
    }
  }

  /// The layer's driver nodes, freshly read, with the drag in progress written
  /// into the one being dragged — what both the preview and the commit send.
  List<BridgeEffectInstance> _driversWith(
      LayerReference layer, UuidValue node) {
    final drivers = layer.getGraphDrivers();
    final staged = _stagedDriver;
    if (staged != null) {
      for (final instance in drivers) {
        if (instance.id() == node) {
          instance.setValue(id: staged.param, value: staged.value);
        }
      }
    }
    return drivers;
  }

  /// The graph's boxes, freshly read, with the drag in progress written into
  /// the one being dragged, which is what both the preview and the commit send.
  List<BridgeEffectInstance> _graphInstancesWith(
      CompositionReference comp, UuidValue node) {
    final instances = comp.getNodeGraphInstances();
    final staged = _stagedDriver;
    if (staged != null) {
      for (final instance in instances) {
        if (instance.id() == node) {
          instance.setValue(id: staged.param, value: staged.value);
        }
      }
    }
    return instances;
  }

  /// What a row should *show*, which during a drag is the staged value.
  BridgeEffectValue? _staged(UuidValue effect, String param) {
    final driver = _stagedDriver;
    if ((_picked?.driver ?? false) || (_picked?.graph ?? false)) {
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
    // A node graph's Read, Input and Output boxes carry no parameters, so they
    // draw their own small face rather than a row list.
    if (comp != null && picked == null && _box != null) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _header(context, _boxName(_box!)),
          Expanded(child: _boxFace(ui, comp, _box!)),
        ],
      );
    }
    if (picked == null || comp == null || (layer == null && !picked.graph)) {
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
          _header(context,
              picked.info.customName ?? engineLabel(picked.info.name)),
          Expanded(child: _rows(ui, picked, layer, playhead)),
        ],
      ),
    );
  }

  /// What a box with no parameters is called: the item's name for a Read, the
  /// Input's own word, and the engine's word for the Output.
  String _boxName(BridgeCompNode box) => switch (box.kind) {
        BridgeCompNodeKind.read || BridgeCompNodeKind.input =>
          box.customName ?? box.label,
        _ => engineLabel(box.label),
      };

  /// The panel's own strip: the kicker, the box's name, and how many of its
  /// parameters a wire has taken over. The dock draws the tab bar above it,
  /// and the strip matches the Graph panel's in height and colour so the two
  /// columns of the workspace line up.
  Widget _header(BuildContext context, String name) {
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
              name,
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
    LayerReference? layer,
    int playhead,
  ) {
    final id = picked.info.id;
    // A node graph has no layers, so a Layer or MaskPath row has none to pick
    // from: it is a socket on the box instead, and the row draws its dash.
    final owner = layer?.internallayerId ?? id;
    final owners = layer == null ? const <BridgeLayerEntry>[] : ui.model.layers;
    final values = {for (final v in picked.info.values) v.id: v.value};
    // The schema's rows, then the ones the instance's own state derives: a
    // nested Node graph box's Input rows arrive that way, as they do in
    // Effect controls.
    final params = [
      ...cachedListParameters(picked.info.name),
      ...picked.info.derivedParams,
    ];

    // **A point is one row, here as everywhere**. An `_x`/`_y` pair of
    // floats folds into a single Position row with one label, one stopwatch
    // over both channels and — where the pair is declared in pixels — the
    // dropper that picks the point off the Viewer. Points sample's query point
    // is the first driver parameter to want it (points-stream.md §2.2), and
    // wanting it in this panel is what a driver's parameters have always
    // wanted: the same row Effect controls draws.
    //
    // The **chain** between the halves is deliberately absent rather than
    // dead: tying a pair is a write on the instance's `linkedPairs`, which is
    // the effect stack's own op, and a driver commits through `setGraph`. A
    // query point's two channels are a place, not a size — nothing here scales.
    final rows = <Widget>[];
    for (var i = 0; i < params.length; i++) {
      final param = params[i];
      final next = i + 1 < params.length ? params[i + 1] : null;
      if (next != null &&
          param.id.endsWith('_x') &&
          next.id == '${param.id.substring(0, param.id.length - 2)}_y' &&
          param.kind is BridgeParamKind_Float &&
          next.kind is BridgeParamKind_Float) {
        rows.add(EffectPointRowFrb(
          key: ValueKey<String>('node-row-$id-${param.id}-pair'),
          effectId: id,
          xParam: param,
          yParam: next,
          xValue: _staged(id, param.id) ?? values[param.id],
          yValue: _staged(id, next.id) ?? values[next.id],
          comp: ui.selectedComp!,
          playheadFrame: playhead,
          onSeek: (frame) => ui.playheadFrame.value = frame,
          onWrite: _write,
          onLive: _live,
          twoColumn: true,
        ));
        i += 1;
        continue;
      }
      rows.add(EffectParamRowFrb(
        key: ValueKey<String>('node-row-$id-${param.id}'),
        effectId: id,
        param: param,
        value: _staged(id, param.id) ?? values[param.id],
        comp: ui.selectedComp!,
        ownerLayerId: owner,
        ownerLayers: owners,
        playheadFrame: playhead,
        onSeek: (frame) => ui.playheadFrame.value = frame,
        onWrite: _write,
        onLive: _live,
        twoColumn: true,
        siblings: values,
        driven: _driven[param.id],
        onAction: picked.driver ? null : (e, p) => _press(picked, e, p),
      ));
    }
    // A **wire-only** input draws no row at all, and needs no code to say so:
    // it is a signature port, never a schema parameter, so it is not in the
    // list this walks. Its socket on the box is the whole of its surface.
    return ListView(
      padding: const EdgeInsets.symmetric(vertical: 4),
      children: rows,
    );
  }

  // --- The boxes with no parameters ----------------------------------------

  Widget _boxFace(
      LumitUiState ui, CompositionReference comp, BridgeCompNode box) {
    if (box.kind == BridgeCompNodeKind.input && _input != null) {
      return _inputForm(comp, box, _input!);
    }
    if (box.kind == BridgeCompNodeKind.read) return _readFace(ui, box);
    // The Output is the picture the Viewer already shows: it has nothing to
    // set and says so.
    return PlaceholderPanel(
      icon: LumitIcon.nodes,
      title: l10n.panelNode,
      hint: l10n.nodeNoRows,
    );
  }

  /// An Input box's five facts, the form the Custom shader's Parameter node
  /// uses. Each edit commits the changed declaration as one op.
  Widget _inputForm(
      CompositionReference comp, BridgeCompNode box, BridgeGraphInput input) {
    final channels = input.kind == BridgeInputKind.colour ? 4 : 1;
    return ListView(
      padding: const EdgeInsets.symmetric(vertical: 4),
      children: [
        _formRow(
          l10n.graphInputKind,
          BareDropdown<BridgeInputKind>(
            key: const ValueKey('node-input-kind'),
            value: input.kind,
            options: compInputKinds,
            label: compInputKindWord,
            onChanged: (kind) =>
                _writeInput(comp, box, _edited(input, kind: kind)),
          ),
        ),
        _formRow(
          l10n.graphInputName,
          SizedBox(
            width: 140,
            child: HouseTextField(
              key: const ValueKey('node-input-label'),
              controller: _label,
              focusNode: _labelFocus,
              width: double.infinity,
              submitOnLostFocus: true,
              onSubmitted: (name) =>
                  _writeInput(comp, box, _edited(input, label: name.trim())),
            ),
          ),
        ),
        _formRow(
          l10n.graphInputDefault,
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (var i = 0; i < channels; i++) ...[
                if (i > 0) const SizedBox(width: 4),
                _number('node-input-default-$i', input.default_[i], (v) {
                  final next = Float64List.fromList(input.default_.toList());
                  next[i] = v;
                  _writeInput(
                      comp, box, _edited(input, defaults: F64Array4(next)));
                }),
              ],
            ],
          ),
        ),
        _formRow(
          l10n.graphInputMin,
          _number('node-input-min', input.min,
              (v) => _writeInput(comp, box, _edited(input, min: v))),
        ),
        _formRow(
          l10n.graphInputMax,
          _number('node-input-max', input.max,
              (v) => _writeInput(comp, box, _edited(input, max: v))),
        ),
        _formRow(
          l10n.graphInputUnit,
          BareDropdown<BridgeUnit>(
            key: const ValueKey('node-input-unit'),
            value: input.unit,
            // The four units an Input declares; the rest belong to parameters
            // the schema writes.
            options: const [
              BridgeUnit.raw,
              BridgeUnit.px,
              BridgeUnit.degrees,
              BridgeUnit.percent,
            ],
            label: _unitWord,
            onChanged: (unit) =>
                _writeInput(comp, box, _edited(input, unit: unit)),
          ),
        ),
      ],
    );
  }

  /// A Read box: what it brings in, and the way into a composition.
  Widget _readFace(LumitUiState ui, BridgeCompNode box) {
    final t = ThemeScope.of(context).theme;
    return ListView(
      padding: const EdgeInsets.symmetric(vertical: 4),
      children: [
        _formRow(
          l10n.graphInputKind,
          Text(
            box.missing
                ? l10n.projectItemMissing
                : (box.item == null ? '' : compItemKindWord(box.item!)),
            key: const ValueKey('node-read-kind'),
            style: t.small,
          ),
        ),
        if (box.item case ItemReference_Composition(:final field0))
          _formRow(
            '',
            HouseButton(
              key: const ValueKey('node-read-open'),
              small: true,
              onPressed: () => ui.setSelectedComp(field0),
              child: Text(l10n.graphOpen, style: t.small),
            ),
          ),
      ],
    );
  }

  /// One row of a form: its name on the left, its control beside it, in the two
  /// columns every other row in this panel is laid out in.
  Widget _formRow(String name, Widget control) {
    final t = ThemeScope.of(context).theme;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 3),
      child: Row(
        children: [
          SizedBox(
              width: 84,
              child: Text(name, style: t.small.copyWith(color: t.textMuted))),
          control,
        ],
      ),
    );
  }

  /// One number well of the Input form. A drag shows where it has got to and
  /// commits **once**, on release: without the live half every accumulated
  /// tick was its own `SetCompGraph`, so one drag took a dozen undos to put
  /// back. The float row's own split.
  Widget _number(String key, double value, ValueChanged<double> onChanged) =>
      SizedBox(
        width: 72,
        child: DragValueField(
          key: ValueKey<String>(key),
          value: _numberDrag?.key == key ? _numberDrag!.value : value,
          min: -1000000,
          max: 1000000,
          decimals: 3,
          speed: 0.5,
          onChangeLive: (v) =>
              setState(() => _numberDrag = (key: key, value: v.toDouble())),
          onDragCancel: () => setState(() => _numberDrag = null),
          onChanged: (v) {
            setState(() => _numberDrag = null);
            onChanged(v.toDouble());
          },
        ),
      );

  String _unitWord(BridgeUnit unit) => switch (unit) {
        BridgeUnit.percent => l10n.unitPercent,
        BridgeUnit.px => l10n.unitPixels,
        BridgeUnit.degrees => l10n.unitDegrees,
        // An Input declares one of four units; every other one is a plain
        // number as far as this form is concerned.
        _ => l10n.unitRaw,
      };

  BridgeGraphInput _edited(
    BridgeGraphInput input, {
    String? label,
    BridgeInputKind? kind,
    F64Array4? defaults,
    double? min,
    double? max,
    BridgeUnit? unit,
  }) =>
      BridgeGraphInput(
        id: input.id,
        label: label ?? input.label,
        kind: kind ?? input.kind,
        default_: defaults ?? input.default_,
        min: min ?? input.min,
        max: max ?? input.max,
        unit: unit ?? input.unit,
      );

  /// One edited Input declaration, back into the wiring: one `setNodeGraph`,
  /// one undo step, exactly as every gesture on the canvas is.
  void _writeInput(
      CompositionReference comp, BridgeCompNode box, BridgeGraphInput changed) {
    try {
      final w = comp.getNodeGraph().wiring;
      comp.setNodeGraph(
        instances: comp.getNodeGraphInstances(),
        wiring: BridgeCompWiring(
          reads: w.reads,
          inputs: [
            for (final i in w.inputs)
              if (i.id == box.id)
                BridgeInputNode(id: i.id, input: changed)
              else
                i,
          ],
          output: w.output,
          edges: w.edges,
          layout: w.layout,
          exposed: w.exposed,
          groups: w.groups,
        ),
      );
    } catch (_) {
      // Refused, or the graph moved under us; re-reading is the recovery.
    }
    _ui?.model.refresh();
    _reload();
  }
}
