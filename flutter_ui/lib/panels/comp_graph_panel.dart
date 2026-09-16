// The node graph composition's canvas, the Graph panel's third subject
// (docs/impl/node-graph-comp.md §4.2).
//
// **In plain terms.** A node graph is a composition whose picture is made by
// boxes and wires instead of a layer stack. This canvas draws those boxes:
// Read boxes bringing footage, solids and comps in, Input boxes standing for
// values handed in from outside, effect and driver boxes from the catalogue, a
// Merge, a Switch, and one Output box whose picture the comp shows.
//
// **What it shares with the layer's graph** is the canvas and nothing else
// (§4.2): the ground, the card, the layout, the wire cubic and the metrics all
// come from `graph_panel.dart`. What it does not share is the chain machinery,
// which exists only because a layer's image chain is its effect list. A node
// graph stores its image edges, so they draw through the ordinary stored-edge
// loop, an image socket takes a drag like any other, and branching falls out of
// the rule the canvas already has: only the destination is exclusive.
//
// **One read, never in a rebuild.** `getNodeGraph` is asked when the comp or
// the document changes and held here; hovering this canvas costs nothing at
// all, which `bridge_call_budget_test` holds it to.
//
// **One gesture, one `setNodeGraph`, one undo step.** The wiring is read,
// edited and handed straight back with the staged boxes, so a wire, a drag, a
// rename and a delete each undo whole.

import 'dart:math' as math;
import 'dart:typed_data' show Float64List;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart'
    show HardwareKeyboard, KeyDownEvent, LogicalKeyboardKey;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/comp_graph.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/lib.dart' show F64Array4;
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart' show LumitIcon, lumitIcon;
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/fx_console_frb.dart';
import '../state/dock.dart';
import '../state/drag_payloads.dart';
import '../state/preview_throttle.dart';
import '../state/timeline_columns.dart' show ValueColumn;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/marquee.dart';
import 'effect_param_row_frb.dart'
    show
        EffectParamRowFrb,
        cachedListParameterGroups,
        cachedListParameters,
        disabledParams,
        paramGroupVisible,
        paramRidersFor;
import 'graph_panel.dart';
import 'placeholder.dart';
import 'shader_editor.dart' show ShaderHome, pressShaderButton;
import 'timeline_extras_frb.dart' show DoubleTap;

/// This canvas's idea of identity, as [graphNodeKey] is the layer canvas's.
String compNodeKey(UuidValue id) => 'node:$id';

/// The four kinds an Input box can carry, in the order the console lists them.
const List<BridgeInputKind> compInputKinds = BridgeInputKind.values;

/// An Input kind's word.
String compInputKindWord(BridgeInputKind kind) => switch (kind) {
      BridgeInputKind.picture => l10n.graphInputPicture,
      BridgeInputKind.number => l10n.graphInputNumber,
      BridgeInputKind.angle => l10n.graphInputAngle,
      BridgeInputKind.colour => l10n.graphInputColour,
    };

/// A project item's kind, as the project panel spells it.
String compItemKindWord(ItemReference item) => switch (item) {
      ItemReference_Footage() => l10n.projectTypeFootage,
      ItemReference_Solid() => l10n.projectTypeSolid,
      ItemReference_Composition() => l10n.projectTypeComposition,
      ItemReference_Folder() => l10n.projectTypeFolder,
    };

/// The item id behind a project reference.
UuidValue compItemId(ItemReference item) => switch (item) {
      ItemReference_Footage(:final field0) => field0.internalid,
      ItemReference_Solid(:final field0) => field0.internalid,
      ItemReference_Composition(:final field0) => field0.internalid,
      ItemReference_Folder(:final field0) => field0.internalid,
    };

/// Whether a wire of [from] may land on a socket of [into].
///
/// Types must be equal, with one exception (§1.4): a **matte socket takes an
/// image wire** as well as a matte one, because what the row means by matte is
/// the picture's own channel, chosen by the row's Channel setting.
bool compTypesFit(BridgePortType from, BridgePortType into) =>
    from == into ||
    (into == BridgePortType.matte && from == BridgePortType.image);

/// The sockets a catalogue entry would draw **in a node graph**, before it is
/// in the document. Auto-wire and the console's filter work from these.
///
/// The listing carries the parameter sockets an entry declares, and its data
/// outputs; the picture's own `input` and `output` are not in it, because on a
/// layer they are the effect list itself. A box that makes a picture is the one
/// with no data outputs of its own, which is the listing's own mark for an
/// image effect, and in a graph it draws both.
(List<BridgePort>, List<BridgePort>) compDeclaredPorts(BridgeEffectInfo info) {
  final picture = info.outputs.isEmpty;
  // A Switch takes its first picture on `in0`, every other box on `input`:
  // the names `ports_of` gives them once the box is in the document (§1.4).
  final input = BridgePort(
      id: info.name == 'switch' ? 'in0' : 'input',
      label: '',
      portType: BridgePortType.image,
      wired: false);
  const output = BridgePort(
      id: 'output', label: '', portType: BridgePortType.image, wired: false);
  return (
    [if (picture) input, ...info.inputs],
    [if (picture) output, ...info.outputs],
  );
}

/// What an open box draws: its rows, the riders folded onto each of them, and
/// the rows another control has taken over.
typedef CompBoxRows = ({
  List<BridgeParamInfo> rows,
  Map<String, List<BridgeParamInfo>> riders,
  Set<String> disabled,
});

/// The rows an open box draws, by the Effect controls panel's own rules, so a
/// box and the panel never disagree about what an effect is showing.
///
/// [params] is the schema's list with the instance's derived rows after it, and
/// [values] and [hidden] are the instance's. Out go the rows a plugin is
/// hiding, the members of a group whose `visible_when` is unmet, the riders
/// that belong beside their host, and a curve, whose editor is no 24px row.
CompBoxRows compBoxRows(
  String effect,
  List<BridgeParamInfo> params,
  Map<String, BridgeEffectValue> values,
  Set<String> hidden,
) {
  final shown = [
    for (final p in params)
      if (!hidden.contains(p.id)) p,
  ];
  final gated = <String>{
    for (final g in cachedListParameterGroups(effect))
      if (!paramGroupVisible(g, values)) ...g.params,
  };
  final riders = <String, List<BridgeParamInfo>>{};
  for (final p in shown) {
    final beside = paramRidersFor(shown, p);
    if (beside.isNotEmpty) riders[p.id] = beside;
  }
  final folded = {
    for (final beside in riders.values)
      for (final p in beside) p.id,
  };
  return (
    rows: [
      for (final p in shown)
        if (!gated.contains(p.id) &&
            !folded.contains(p.id) &&
            p.kind is! BridgeParamKind_Curve)
          p,
    ],
    riders: riders,
    disabled: disabledParams(effect, values),
  );
}

/// How far apart several boxes dropped at once are stacked, canvas units.
const double _dropStep = 100;

/// A wire in hand on this canvas, and the stored wire it took hold of.
class _Flight extends GraphFlight {
  final BridgeCompEdge? detached;
  _Flight(super.from, super.to, {this.detached});
}

class CompGraphPanel extends StatefulWidget {
  /// The node graph this canvas draws.
  final CompositionReference comp;

  /// The catalogue seams, injected by tests so the console can be asserted
  /// without the real registry. The layer canvas's `driversLister` twins.
  final List<BridgeEffectInfo> Function()? nodesLister;
  final List<BridgeEffectInfo> Function()? effectsLister;

  /// The panel this canvas sits in. The Graph panel and the Timeline can both
  /// show one, so each answers the editing keys only while its own is active.
  final Panel host;

  const CompGraphPanel({
    super.key,
    required this.comp,
    this.nodesLister,
    this.effectsLister,
    this.host = Panel.graph,
  });

  @override
  State<CompGraphPanel> createState() => _CompGraphPanelState();
}

class _CompGraphPanelState extends State<CompGraphPanel> {
  LumitUiState? _ui;

  /// The held graph. Read when the comp or the document changes, never in a
  /// build, which is what the budget test is guarding.
  BridgeCompGraph? _graph;

  /// Its boxes by key, so a gesture holding a key alone can reach the box.
  Map<String, BridgeCompNode> _byKey = const {};

  /// The Input declarations by key, for the kind each box wears as its kicker.
  Map<String, BridgeGraphInput> _inputs = const {};

  /// Each Fx box's values and the rows it lists, read once with the graph so
  /// an open box's controls draw from what the canvas holds: one `getInfo`
  /// per box per document change, and nothing on a rebuild.
  Map<String, BridgeEffectInstanceInfo> _infos = const {};
  Map<String, CompBoxRows> _params = const {};

  /// A drag on a box's control: shown at once, previewed through the render
  /// request, committed once on release.
  ({UuidValue node, String param, BridgeEffectValue value})? _staged;
  final PreviewThrottle _preview = PreviewThrottle();

  /// Canvas positions, staged: a drag moves this map and the release commits.
  Map<String, Offset> _positions = {};

  /// **The picked boxes**, by key, in the order they were picked. The anchor,
  /// the box picked last, is what the Node panel and the Viewer chip follow.
  final Map<String, UuidValue> _selection = {};

  String? _renaming;
  Offset _pan = Offset.zero;
  double _zoom = 1;
  bool _autoWire = true;
  bool _heal = true;
  bool _snapToGrid = true;

  _Flight? _flight;
  GraphNodeDrag? _nodeDrag;
  BridgeCompEdge? _dropWire;
  Offset? _panFrom;
  Offset? _pressAt;
  bool _claimed = false;
  Offset? _marqueeFrom;
  Offset? _marqueeTo;
  bool _marqueeAdds = false;
  bool _searching = false;
  bool _menuPress = false;
  Size _viewport = Size.zero;

  /// The canvas's own render box, so a drop from the project panel lands
  /// where the pointer let go rather than where the panel starts.
  final GlobalKey _canvasKey = GlobalKey();

  final DoubleTap _boxTaps = DoubleTap();
  String? _boxTapKey;
  final FocusNode _canvasFocus = FocusNode(debugLabel: 'comp graph canvas');

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (!identical(ui, _ui)) {
      _unbind();
      _ui = ui;
      ui.model.addListener(_reload);
      ui.selectAllRequest.addListener(_onSelectAllRequested);
      if (ui.consoleClaim != _consoleClaim) _priorConsoleClaim = ui.consoleClaim;
      ui.consoleClaim = _consoleClaim;
      if (ui.deleteClaim != _deleteClaim) _priorDeleteClaim = ui.deleteClaim;
      ui.deleteClaim = _deleteClaim;
      if (ui.copyClaim != _copyClaim) _priorCopyClaim = ui.copyClaim;
      ui.copyClaim = _copyClaim;
      if (ui.pasteClaim != _pasteClaim) _priorPasteClaim = ui.pasteClaim;
      ui.pasteClaim = _pasteClaim;
    }
    _reload();
  }

  @override
  void didUpdateWidget(CompGraphPanel old) {
    super.didUpdateWidget(old);
    if (old.comp != widget.comp) {
      _selection.clear();
      _publishPick();
      _reload();
    }
  }

  bool Function()? _priorDeleteClaim;
  bool Function()? _priorConsoleClaim;
  bool Function()? _priorCopyClaim;
  bool Function()? _priorPasteClaim;

  bool _deleteClaim() {
    final ui = _ui;
    if (!mounted || ui == null || ui.activePanel != widget.host) {
      return _priorDeleteClaim?.call() ?? false;
    }
    return _deleteSelected() || (_priorDeleteClaim?.call() ?? false);
  }

  bool _consoleClaim() {
    final ui = _ui;
    if (!mounted ||
        ui == null ||
        ui.activePanel != widget.host ||
        _graph == null ||
        _searching) {
      return _priorConsoleClaim?.call() ?? false;
    }
    _openSearch(graphClearSpot(
      _toCanvas(Offset(_viewport.width / 2, _viewport.height / 2)),
      [for (final b in _layout().boxes) b.rect],
    ));
    return true;
  }

  bool _copyClaim() {
    final ui = _ui;
    if (!mounted || ui == null || ui.activePanel != widget.host) {
      return _priorCopyClaim?.call() ?? false;
    }
    return _copySelected(ui) || (_priorCopyClaim?.call() ?? false);
  }

  bool _pasteClaim() {
    final ui = _ui;
    if (!mounted || ui == null || ui.activePanel != widget.host) {
      return _priorPasteClaim?.call() ?? false;
    }
    return _pasteBoxes(ui) || (_priorPasteClaim?.call() ?? false);
  }

  void _unbind() {
    _ui?.model.removeListener(_reload);
    _ui?.selectAllRequest.removeListener(_onSelectAllRequested);
    if (_ui?.deleteClaim == _deleteClaim) _ui!.deleteClaim = _priorDeleteClaim;
    if (_ui?.consoleClaim == _consoleClaim) {
      _ui!.consoleClaim = _priorConsoleClaim;
    }
    if (_ui?.copyClaim == _copyClaim) _ui!.copyClaim = _priorCopyClaim;
    if (_ui?.pasteClaim == _pasteClaim) _ui!.pasteClaim = _priorPasteClaim;
  }

  @override
  void dispose() {
    _unbind();
    _preview.cancel();
    _canvasFocus.dispose();
    super.dispose();
  }

  void _onSelectAllRequested() {
    final ui = _ui;
    if (!mounted || ui == null) return;
    if (!ui.selectAllRequestIsFor(widget.host)) return;
    setState(() => _pick([for (final n in _graph?.nodes ?? const []) n.id]));
  }

  /// The one read. Everything the canvas draws comes from here.
  void _reload() {
    if (!mounted) return;
    BridgeCompGraph? graph;
    try {
      graph = widget.comp.getNodeGraph();
    } catch (_) {
      // The comp has gone, or is not a node graph after all; the placeholder
      // is the honest answer until the selection catches up.
      graph = null;
    }
    // The rows an open box draws, worked out by [compBoxRows]: every parameter
    // the schema lists and the ones the instance derives, put through the same
    // rules the Effect controls panel applies. Read here, beside the graph,
    // never in a build.
    final infos = <String, BridgeEffectInstanceInfo>{};
    final params = <String, CompBoxRows>{};
    if (graph != null) {
      try {
        for (final instance in widget.comp.getNodeGraphInstances()) {
          final key = compNodeKey(instance.id());
          final info = instance.getInfo();
          infos[key] = info;
          params[key] = compBoxRows(
            info.name,
            [...cachedListParameters(info.name), ...info.derivedParams],
            {for (final v in info.values) v.id: v.value},
            info.hiddenRows.toSet(),
          );
        }
      } catch (_) {
        // The comp moved under us; the boxes draw their sockets and the
        // next read fills the rows in.
      }
    }
    setState(() {
      _graph = graph;
      _infos = infos;
      _params = params;
      _byKey = {
        for (final n in graph?.nodes ?? const <BridgeCompNode>[])
          compNodeKey(n.id): n,
      };
      _inputs = {
        for (final i in graph?.wiring.inputs ?? const <BridgeInputNode>[])
          compNodeKey(i.id): i.input,
      };
      _positions = {
        for (final p
            in graph?.wiring.layout ?? const <BridgeCompNodePosition>[])
          compNodeKey(p.node): Offset(p.x, p.y),
      };
      // A box that has gone leaves the pick; the rest of the pick stands.
      if (_selection.keys.any((k) => !_byKey.containsKey(k))) {
        _selection.removeWhere((key, _) => !_byKey.containsKey(key));
        _publishPick();
      }
    });
  }

  // --- The pick -----------------------------------------------------------

  void _pick(Iterable<UuidValue> ids) {
    _selection
      ..clear()
      ..addEntries([for (final id in ids) MapEntry(compNodeKey(id), id)]);
    _publishPick();
  }

  /// The anchor, published for the Node panel to draw and the Viewer chip to
  /// name. The label rides along so neither of them costs a call, and the
  /// picture mark says whether this box can be looked *at*: a driver, a value
  /// Input and the Output make no picture, so they offer no chip (§4.5).
  void _publishPick() {
    final ui = _ui;
    if (ui == null) return;
    final key = _selection.keys.isEmpty ? null : _selection.keys.last;
    final node = key == null ? null : _byKey[key];
    ui.compGraphNode.value = node == null
        ? null
        : (
            id: node.id,
            label: _titleOf(node),
            picture: node.outputs.any((p) => p.portType == BridgePortType.image),
          );
  }

  // --- Committing ---------------------------------------------------------

  /// The wiring as it stands, with the staged positions folded in. Every write
  /// goes through here, so a gesture that moves a box and one that draws a
  /// wire commit the same shape.
  /// [pending] is the box this gesture is adding, which the held read model
  /// has not seen yet: without it the new box's own place would be dropped as
  /// a position naming nothing, and the box would land where it was
  /// auto-placed rather than where it was let go.
  BridgeCompWiring _wiringNow({
    List<BridgeReadNode>? reads,
    List<BridgeInputNode>? inputs,
    List<BridgeCompEdge>? edges,
    List<UuidValue>? exposed,
    List<BridgeCompNodeGroup>? groups,
    Map<String, UuidValue> pending = const {},
  }) {
    final w = _graph!.wiring;
    return BridgeCompWiring(
      reads: reads ?? w.reads,
      inputs: inputs ?? w.inputs,
      output: w.output,
      edges: edges ?? w.edges,
      layout: [
        for (final e in _positions.entries)
          if (_byKey[e.key]?.id ?? pending[e.key] case final id?)
            BridgeCompNodePosition(node: id, x: e.value.dx, y: e.value.dy),
      ],
      exposed: exposed ?? w.exposed,
      groups: groups ?? w.groups,
    );
  }

  /// One gesture, one `setNodeGraph`, one undo step. A refusal leaves the
  /// document exactly as it was: the panel declines what it can decline
  /// itself, and the engine's refusal is the backstop behind that.
  void _commit(BridgeCompWiring wiring,
      {List<BridgeEffectInstance>? instances}) {
    try {
      widget.comp.setNodeGraph(
        instances: instances ?? widget.comp.getNodeGraphInstances(),
        wiring: wiring,
      );
    } catch (_) {
      // Refused, or the comp moved under us. Either way the document is
      // untouched and re-reading is the recovery.
    }
    _ui?.model.refresh();
    _reload();
  }

  // --- What a box looks like ----------------------------------------------

  String _titleOf(BridgeCompNode node) => switch (node.kind) {
        // A Read and an Input are named by the document, not by the engine's
        // vocabulary, so neither goes through the label table.
        BridgeCompNodeKind.read || BridgeCompNodeKind.input => node.label,
        _ => engineLabel(node.label),
      };

  /// What this box says under its name: the item's kind for a Read, or the
  /// missing mark where the item has gone, and the value's kind for an Input.
  String? _kickerOf(BridgeCompNode node) {
    if (node.kind == BridgeCompNodeKind.read) {
      if (node.missing) return l10n.projectItemMissing;
      return node.item == null ? null : compItemKindWord(node.item!);
    }
    if (node.kind == BridgeCompNodeKind.input) {
      final input = _inputs[compNodeKey(node.id)];
      return input == null ? null : compInputKindWord(input.kind);
    }
    return null;
  }

  /// A Switch's sockets are `in0`, `in1`, … with no words of their own, so the
  /// canvas draws the index rather than an empty row.
  List<BridgePort> _labelled(List<BridgePort> ports) => [
        for (final p in ports)
          if (p.label.isEmpty)
            BridgePort(
              id: p.id,
              label: p.id.replaceAll(RegExp('[^0-9]'), ''),
              portType: p.portType,
              wired: p.wired,
            )
          else
            p,
      ];

  /// Whether this box makes a value rather than a picture, which decides the
  /// row it auto-places on, exactly as a driver does on the layer canvas.
  bool _makesValue(BridgeCompNode node) =>
      node.outputs.isNotEmpty &&
      !node.outputs.any((p) => p.portType == BridgePortType.image);

  GraphLayout _layout() {
    final exposed = _graph!.wiring.exposed.map(compNodeKey).toSet();
    final boxes = <GraphBox>[];
    var upper = 0;
    var lower = 0;
    for (final node in _graph!.nodes) {
      final key = compNodeKey(node.id);
      final value = _makesValue(node);
      final placed = _positions[key] ??
          graphAutoPlace(value ? lower : upper,
              lower: value, width: graphNodeOpenWidth);
      value ? lower++ : upper++;
      final fx = node.kind == BridgeCompNodeKind.fx;
      final open = exposed.contains(key) || value;
      // An open box gives every parameter a row with its control on it, and
      // is wider for them; a shut one, and a box with nothing to set, keeps
      // the drawing's width.
      final rows =
          open ? _params[key]?.rows ?? const <BridgeParamInfo>[] : const [];
      boxes.add(graphLayoutBox(
        (
          key: key,
          title: _titleOf(node),
          customName: node.customName,
          kicker: _kickerOf(node),
          // A Read whose item has gone draws transparent, so its box draws
          // dashed: the same mark a bypassed box wears, for the same reading.
          enabled: node.enabled && !node.missing,
          tick: fx,
          twirl: fx,
          // A Read and an Fx box take a name of their own. An Input's name is
          // its label, which the Node panel's form edits.
          rename: fx || node.kind == BridgeCompNodeKind.read,
          // A box with an inside: a Custom shader, and a nested node graph.
          tinted: node.matchName == 'custom_shader' ||
              node.matchName == 'node_graph',
          inputs: _labelled(node.inputs),
          outputs: _labelled(node.outputs),
        ),
        placed,
        open: open,
        params: [for (final p in rows) p.id],
        width: node.kind == BridgeCompNodeKind.output
            ? graphOutNodeWidth
            : rows.isEmpty
                ? graphNodeWidth
                : graphNodeOpenWidth,
      ));
    }
    return GraphLayout(boxes);
  }

  // --- A box's own controls -----------------------------------------------

  /// One parameter row on an open box: the Node panel's row, drawn from the
  /// values the canvas holds. It writes through the same op the panel's edit
  /// does and previews through the same request.
  Widget _paramRow(String key, String param) {
    final node = _byKey[key];
    final info = _infos[key];
    final held = _params[key];
    final p = held?.rows.where((p) => p.id == param).firstOrNull;
    final ui = _ui;
    if (node == null ||
        info == null ||
        held == null ||
        p == null ||
        ui == null) {
      return const SizedBox.shrink();
    }
    final values = {for (final v in info.values) v.id: v.value};
    final staged = _staged;
    BridgeEffectValue? valueOf(String id) =>
        staged != null && staged.node == node.id && staged.param == id
            ? staged.value
            : values[id];
    return EffectParamRowFrb(
      key: ValueKey<String>('graph-row-$key-$param'),
      effectId: node.id,
      param: p,
      value: valueOf(param),
      comp: widget.comp,
      // A node graph has no layers: a layer row is a socket on the box, and
      // the row draws its dash (§4.3).
      ownerLayerId: node.id,
      ownerLayers: const [],
      playheadFrame: ui.playheadFrame.value,
      onSeek: (frame) => ui.playheadFrame.value = frame,
      onWrite: _writeParam,
      onLive: _liveParam,
      rowPadding: EdgeInsets.zero,
      valueColumn: const ValueColumn(graphControlColumn, graphRowInset),
      siblings: values,
      // Greyed where another control has taken the row over, so a drag on the
      // box cannot commit an op that changes no pixel.
      enabled: !held.disabled.contains(param),
      riders: [
        for (final r in held.riders[param] ?? const <BridgeParamInfo>[])
          (r, valueOf(r.id)),
      ],
      // A Custom shader's two buttons press the way they do in Effect controls.
      // The only other Action a box carries is the Node graph's Open graph, and
      // the canvas already knows how to go in.
      onAction: (effect, param) {
        if (node.matchName == 'custom_shader' &&
            pressShaderButton(
              context: context,
              home: ShaderHome.graph(
                widget.comp,
                draw: (staged) => widget.comp.renderFrameWithGraphPreview(
                  frame: BigInt.from(ui.playheadFrame.value),
                  scale: ui.viewerScale,
                  instances: staged,
                ),
              ),
              effect: effect,
              param: param,
              onApplied: () {
                ui.model.refresh();
                _reload();
              },
            )) {
          return;
        }
        _enterBox(node);
      },
    );
  }

  /// The staged boxes with one value written into one of them: what a
  /// preview sends and what a commit sends.
  List<BridgeEffectInstance> _instancesWith(
      UuidValue node, String param, BridgeEffectValue value) {
    final instances = widget.comp.getNodeGraphInstances();
    for (final instance in instances) {
      if (instance.id() == node) instance.setValue(id: param, value: value);
    }
    return instances;
  }

  /// A typed value, or the release of a drag: one `setNodeGraph`, one undo
  /// step, exactly what the Node panel's edit commits.
  void _writeParam(UuidValue node, String param, BridgeEffectValue value) {
    _preview.cancel();
    final List<BridgeEffectInstance> instances;
    try {
      instances = _instancesWith(node, param, value);
    } catch (_) {
      return;
    }
    _staged = null;
    _commit(_wiringNow(), instances: instances);
  }

  /// A drag tick: the row shows it and the Viewer previews it; nothing is
  /// committed.
  void _liveParam(UuidValue node, String param, BridgeEffectValue value) {
    final ui = _ui;
    if (ui == null) return;
    setState(() => _staged = (node: node, param: param, value: value));
    // Read inside the closure: a held tick must send the newest staged
    // value, not the one that was current when it was held.
    _preview.request(() {
      try {
        widget.comp.renderFrameWithGraphPreview(
          frame: BigInt.from(ui.playheadFrame.value),
          scale: ui.viewerScale,
          instances: _instancesWith(node, param, value),
        );
      } catch (_) {
        // The graph moved under the drag; the release re-reads.
      }
    });
  }

  // --- Wires --------------------------------------------------------------

  String _fromKey(BridgeCompEdge e) => compNodeKey(e.from);
  String _toKey(BridgeCompEdge e) => compNodeKey(e.to);

  bool _touches(BridgeCompEdge e, String key) =>
      _fromKey(e) == key || _toKey(e) == key;

  /// Where one stored wire starts and ends, and the type it carries: the
  /// source port's, which is the type the wire is.
  (Offset, Offset, BridgePortType)? _edgeEnds(
      GraphLayout layout, BridgeCompEdge e) {
    final fromBox = layout.byKey[_fromKey(e)];
    final toBox = layout.byKey[_toKey(e)];
    if (fromBox == null || toBox == null) return null;
    final from = fromBox.socket(e.fromPort, false);
    final to = toBox.socket(e.toPort, true);
    if (from == null || to == null) return null;
    final i = fromBox.outputs.indexWhere((p) => p.id == e.fromPort);
    return (from, to, fromBox.outputs[i < 0 ? 0 : i].portType);
  }

  BridgeCompEdge? _edgeInto(GraphSocket socket) {
    final id = _byKey[socket.node]?.id;
    if (id == null) return null;
    for (final e in _graph!.wiring.edges) {
      if (e.to == id && e.toPort == socket.port.id) return e;
    }
    return null;
  }

  GraphSocket? _sourceSocket(BridgeCompEdge edge, GraphLayout layout) {
    final box = layout.byKey[_fromKey(edge)];
    if (box == null) return null;
    final i = box.outputs.indexWhere((p) => p.id == edge.fromPort);
    final at = box.socket(edge.fromPort, false);
    if (i < 0 || at == null) return null;
    return GraphSocket(box.key, box.outputs[i], false, at);
  }

  /// Whether these two sockets may be joined, decided **here** from the two
  /// port types the read model already carries. A mismatched drop is declined
  /// without a bridge call; the engine's refusal is the backstop.
  bool _accepts(GraphSocket from, GraphSocket to) {
    if (from.isInput == to.isInput) return false;
    final out = from.isInput ? to : from;
    final into = from.isInput ? from : to;
    if (out.node == into.node) return false;
    if (!compTypesFit(out.port.portType, into.port.portType)) return false;
    return !_wouldLoop(out.node, into.node);
  }

  /// Whether joining these would close a loop. The walk is over the stored
  /// wires, exactly as the engine's is.
  bool _wouldLoop(String from, String into) {
    final seen = <String>{};
    final queue = <String>[into];
    while (queue.isNotEmpty) {
      final at = queue.removeLast();
      if (at == from) return true;
      if (!seen.add(at)) continue;
      for (final edge in _graph!.wiring.edges) {
        if (_fromKey(edge) == at) queue.add(_toKey(edge));
      }
    }
    return false;
  }

  /// Draw or re-route a wire. Only the destination is exclusive: a producer
  /// goes on feeding everything it already fed, which is what makes a fork.
  void _connect(GraphSocket a, GraphSocket b, {BridgeCompEdge? without}) {
    final from = a.isInput ? b : a;
    final to = a.isInput ? a : b;
    final source = _byKey[from.node]?.id;
    final dest = _byKey[to.node]?.id;
    if (source == null || dest == null) return;
    _commit(_wiringNow(edges: [
      for (final e in _graph!.wiring.edges)
        if (!(e.to == dest && e.toPort == to.port.id) && e != without) e,
      BridgeCompEdge(
          from: source, fromPort: from.port.id, to: dest, toPort: to.port.id),
    ]));
  }

  void _removeEdge(BridgeCompEdge edge) {
    _commit(_wiringNow(edges: [
      for (final e in _graph!.wiring.edges)
        if (e != edge) e,
    ]));
  }

  // --- Adding a box -------------------------------------------------------

  UuidValue get _newId => UuidValue.fromString(const Uuid().v4());

  /// The wires a box added at [made] rides in on.
  ///
  /// With a **wire in hand** it joins whichever of the new box's sockets fits.
  /// Otherwise **Auto-wire** puts it after the picked box: what that box's
  /// picture socket fed, this box's picture socket now takes, and if the
  /// picked box fed the Output this one feeds the Output instead.
  List<BridgeCompEdge> _wiredIn(
    UuidValue made,
    List<BridgePort> inputs,
    List<BridgePort> outputs,
    GraphSocket? wire,
    UuidValue? after,
  ) {
    final edges = [..._graph!.wiring.edges];
    if (!_autoWire) return edges;
    if (wire != null) {
      final held = _byKey[wire.node]?.id;
      if (held == null) return edges;
      for (final port in wire.isInput ? outputs : inputs) {
        final socket =
            GraphSocket(compNodeKey(made), port, !wire.isInput, Offset.zero);
        if (!_accepts(wire, socket)) continue;
        final joined = wire.isInput
            ? BridgeCompEdge(
                from: made,
                fromPort: port.id,
                to: held,
                toPort: wire.port.id)
            : BridgeCompEdge(
                from: held,
                fromPort: wire.port.id,
                to: made,
                toPort: port.id);
        edges.removeWhere(
            (e) => e.to == joined.to && e.toPort == joined.toPort);
        edges.add(joined);
        return edges;
      }
      return edges;
    }
    if (after == null) return edges;
    final takes =
        inputs.where((p) => p.portType == BridgePortType.image).firstOrNull;
    final gives =
        outputs.where((p) => p.portType == BridgePortType.image).firstOrNull;
    if (gives == null) return edges;
    // The picked box's own picture socket, read off the model rather than
    // assumed: the Output box, a value Input and every driver have none, and
    // a wire naming a socket a box has not got is refused, which lost the
    // whole add.
    final anchor = _byKey[compNodeKey(after)];
    final source = anchor?.outputs
        .where((p) => p.portType == BridgePortType.image)
        .firstOrNull;
    if (source == null) {
      // Added with the **Output** picked, the new box feeds it; added after
      // anything else with no picture to give, it lands unwired.
      final into = after == _graph!.wiring.output
          ? anchor?.inputs.firstOrNull
          : null;
      if (into == null) return edges;
      edges.removeWhere((e) => e.to == after && e.toPort == into.id);
      edges.add(BridgeCompEdge(
          from: made, fromPort: gives.id, to: after, toPort: into.id));
      return edges;
    }
    if (takes == null) return edges;
    // What the picked box fed, the new box feeds instead.
    final onward = [
      for (final e in edges)
        if (e.from == after && e.fromPort == source.id) e,
    ];
    for (final e in onward) {
      edges.remove(e);
      edges.add(BridgeCompEdge(
          from: made, fromPort: gives.id, to: e.to, toPort: e.toPort));
    }
    edges.removeWhere((e) => e.to == made && e.toPort == takes.id);
    edges.add(BridgeCompEdge(
        from: after, fromPort: source.id, to: made, toPort: takes.id));
    return edges;
  }

  /// The box the pick would auto-wire after: one picked box, and only one.
  UuidValue? get _anchor =>
      _selection.length == 1 ? _selection.values.single : null;

  /// Project items brought in, at the spot they were let go of.
  ///
  /// A drag from the project panel carries the whole selection, so several
  /// land in one commit and therefore one undo step, stepped down the canvas
  /// so they do not sit on top of each other. Only the first takes the wire in
  /// hand, there being one wire.
  void _addRead(List<ItemReference> items, Offset at, {GraphSocket? wire}) {
    final graph = _graph;
    if (graph == null || items.isEmpty) return;
    const output = BridgePort(
        id: 'output',
        label: '',
        portType: BridgePortType.image,
        wired: false);
    final reads = [...graph.wiring.reads];
    final pending = <String, UuidValue>{};
    var edges = graph.wiring.edges;
    for (final item in items) {
      final made = _newId;
      final key = compNodeKey(made);
      _positions[key] = at + Offset(0, pending.length * _dropStep);
      pending[key] = made;
      reads.add(BridgeReadNode(id: made, item: compItemId(item)));
      if (pending.length == 1) {
        edges = _wiredIn(made, const [], const [output], wire, null);
      }
    }
    _commit(_wiringNow(reads: reads, edges: edges, pending: pending));
    setState(() => _pick(pending.values.toList()));
  }

  /// A value or picture handed in from outside. Its id is what names the row
  /// the graph draws outside itself, so it has to be unique here.
  void _addInput(BridgeInputKind kind, Offset at, {GraphSocket? wire}) {
    final graph = _graph;
    if (graph == null) return;
    final made = _newId;
    final key = compNodeKey(made);
    _positions[key] = at;
    final taken = {for (final i in graph.wiring.inputs) i.input.id};
    final stem = kind.name;
    var id = stem;
    for (var n = 2; taken.contains(id); n++) {
      id = '${stem}_$n';
    }
    final picture = kind == BridgeInputKind.picture;
    final port = BridgePort(
      id: picture ? 'output' : 'value',
      label: '',
      portType: switch (kind) {
        BridgeInputKind.picture => BridgePortType.image,
        BridgeInputKind.colour => BridgePortType.colour,
        _ => BridgePortType.number,
      },
      wired: false,
    );
    _commit(_wiringNow(
      inputs: [
        ...graph.wiring.inputs,
        BridgeInputNode(
          id: made,
          input: BridgeGraphInput(
            id: id,
            label: compInputKindWord(kind),
            kind: kind,
            default_: F64Array4(Float64List.fromList(
                kind == BridgeInputKind.colour
                    ? [1, 1, 1, 1]
                    : [0, 0, 0, 0])),
            min: 0,
            max: switch (kind) {
              BridgeInputKind.angle => 360,
              BridgeInputKind.colour => 1,
              _ => 100,
            },
            unit: switch (kind) {
              BridgeInputKind.angle => BridgeUnit.degrees,
              _ => BridgeUnit.raw,
            },
          ),
        ),
      ],
      edges: _wiredIn(made, const [], [port], wire, null),
      pending: {key: made},
    ));
    setState(() => _pick([made]));
  }

  /// A catalogue box: an effect, a driver, a Merge, a Switch, or a nested node
  /// graph. `newGraphInstance` deliberately does not commit, so the box, its
  /// place and its wires all land in one op.
  void _addFx(BridgeEffectInfo info, Offset at,
      {GraphSocket? wire, CompositionReference? graph}) {
    if (_graph == null) return;
    final after = _anchor;
    final BridgeEffectInstance made;
    final List<BridgeEffectInstance> instances;
    try {
      made = widget.comp.newGraphInstance(name: info.name, graph: graph);
      instances = [...widget.comp.getNodeGraphInstances(), made];
    } catch (_) {
      return;
    }
    final id = made.id();
    final key = compNodeKey(id);
    _positions[key] = at;
    final (inputs, outputs) = compDeclaredPorts(info);
    _commit(
      _wiringNow(
        edges: _wiredIn(id, inputs, outputs, wire, after),
        // A new box starts open, its rows showing; a box already in a saved
        // graph keeps whatever the file says.
        exposed: [..._graph!.wiring.exposed, id],
        pending: {key: id},
      ),
      instances: instances,
    );
    setState(() => _pick([id]));
  }

  // --- The console --------------------------------------------------------

  /// Ctrl+Space, Tab, Shift+A, a right-click and a wire let go over empty
  /// canvas all open the console, the
  /// same popover the shell opens, wearing this canvas's own list: the
  /// project's items as Read boxes, the four Input kinds, the effects, then
  /// the boxes only a graph can hold.
  Future<void> _openSearch(Offset at, {GraphSocket? wire}) async {
    if (_searching) return;
    setState(() => _searching = true);
    final project = Provider.of<LumitState>(context, listen: false).project;
    final items = graphProjectItems(project);
    // Which of the project's comps are node graphs, asked once per comp as the
    // console opens. The model is the one answer that carries the fact, and
    // a gesture is where a read like this belongs.
    final graphs = <UuidValue, CompositionReference>{};
    for (final item in items) {
      if (item case ItemReference_Composition(:final field0)) {
        try {
          if (field0.getModel().isNodeGraph) {
            graphs[field0.internalid] = field0;
          }
        } catch (_) {
          // The comp has gone since the list was made; it simply is not
          // offered.
        }
      }
    }
    final entries = <FxConsoleEntry>[];
    // The project's items first, as Read boxes. A node graph among them is
    // *applied* rather than read, so it is offered as a nested box instead.
    final nested =
        listGraphNodes().where((e) => e.name == 'node_graph').firstOrNull;
    const image = BridgePort(
        id: 'output',
        label: '',
        portType: BridgePortType.image,
        wired: false);
    for (final item in items) {
      // Never this graph itself: a box reading the comp it is in is a loop,
      // and the engine would refuse it.
      if (compItemId(item) == widget.comp.internalid) continue;
      final inner = graphs[compItemId(item)];
      if (inner != null && nested != null) {
        if (wire != null && !_fitsEntry(nested, wire)) continue;
        entries.add(FxConsoleEntry(
          label: item.name(),
          kind: FxConsoleKind.effect,
          group: engineLabel(nested.label),
          run: () => _addFx(nested, at, wire: wire, graph: inner),
        ));
        continue;
      }
      if (wire != null && !_fitsPorts(const [], const [image], wire)) continue;
      entries.add(FxConsoleEntry(
        label: item.name(),
        kind: FxConsoleKind.effect,
        group: compItemKindWord(item),
        run: () => _addRead([item], at, wire: wire),
      ));
    }
    // Then the four Input kinds, the effects, and the boxes only a graph can
    // hold: Merge and Switch under Compositing, then the drivers.
    for (final kind in compInputKinds) {
      if (wire != null && !_fitsInput(kind, wire)) continue;
      entries.add(FxConsoleEntry(
        label: compInputKindWord(kind),
        kind: FxConsoleKind.effect,
        group: l10n.graphInput,
        run: () => _addInput(kind, at, wire: wire),
      ));
    }
    // `listEffects` carries the drivers too, filed under Controls; here they
    // come from the graph's own listing instead, so no entry is offered twice.
    final nodes = (widget.nodesLister ?? listGraphNodes)();
    final nodeNames = {for (final node in nodes) node.name};
    for (final info in [
      for (final effect in (widget.effectsLister ?? listEffects)())
        if (!nodeNames.contains(effect.name)) effect,
      ...nodes,
    ]) {
      if (info.name == 'node_graph') continue;
      if (wire != null && !_fitsEntry(info, wire)) continue;
      entries.add(FxConsoleEntry(
        label: engineLabel(info.label),
        kind: FxConsoleKind.effect,
        group: engineLabel(info.categoryLabel),
        run: () => _addFx(info, at, wire: wire),
      ));
    }
    try {
      await showFxConsoleFrb(
        context: context,
        anchor: lastKnownPointerPosition,
        model: FxConsoleModel(
          keyHint: wire == null ? l10n.fxConsoleKey : null,
          footer: wire == null ? l10n.graphConsoleAdds : l10n.graphSearchWires,
          entries: entries,
        ),
      );
    } finally {
      if (mounted) setState(() => _searching = false);
    }
  }

  /// Whether any socket these declared ports offer could take the wire in
  /// hand. The same type rule [_accepts] applies, asked of a box that does
  /// not exist yet.
  bool _fitsPorts(
    List<BridgePort> inputs,
    List<BridgePort> outputs,
    GraphSocket wire,
  ) =>
      (wire.isInput ? outputs : inputs).any((port) => wire.isInput
          ? compTypesFit(port.portType, wire.port.portType)
          : compTypesFit(wire.port.portType, port.portType));

  bool _fitsEntry(BridgeEffectInfo info, GraphSocket wire) {
    final (inputs, outputs) = compDeclaredPorts(info);
    return _fitsPorts(inputs, outputs, wire);
  }

  bool _fitsInput(BridgeInputKind kind, GraphSocket wire) {
    if (!wire.isInput) return false;
    final type = switch (kind) {
      BridgeInputKind.picture => BridgePortType.image,
      BridgeInputKind.colour => BridgePortType.colour,
      _ => BridgePortType.number,
    };
    return compTypesFit(type, wire.port.portType);
  }

  // --- Box gestures -------------------------------------------------------

  List<String> _targets(String key) =>
      _selection.containsKey(key) ? _selection.keys.toList() : [key];

  void _toggleExposed(String key) {
    final on = !_graph!.wiring.exposed.any((e) => compNodeKey(e) == key);
    final targets = {
      for (final k in _targets(key))
        if (_byKey[k]?.id case final id?) k: id,
    };
    _commit(_wiringNow(exposed: [
      for (final e in _graph!.wiring.exposed)
        if (!targets.containsKey(compNodeKey(e))) e,
      if (on) ...targets.values,
    ]));
  }

  /// Bypass, on the staged instance: one `setNodeGraph`, one undo step
  /// however many boxes the press acts on.
  void _toggleBypass(String key, bool enabled) {
    if (_graph == null) return;
    final on = !enabled;
    final ids = {
      for (final k in _targets(key))
        if (_byKey[k] case final node?)
          if (node.kind == BridgeCompNodeKind.fx) node.id,
    };
    if (ids.isEmpty) return;
    final List<BridgeEffectInstance> instances;
    try {
      instances = widget.comp.getNodeGraphInstances();
      for (final instance in instances) {
        if (ids.contains(instance.id())) instance.setEnabled(enabled: on);
      }
    } catch (_) {
      return;
    }
    _commit(_wiringNow(), instances: instances);
  }

  /// The user's own name for a box: an Fx box stages it on its instance, a
  /// Read box carries it in the wiring. One op either way.
  void _rename(String key, String name) {
    setState(() => _renaming = null);
    final graph = _graph;
    final node = _byKey[key];
    if (graph == null || node == null) return;
    final trimmed = name.trim();
    if (node.kind == BridgeCompNodeKind.read) {
      _commit(_wiringNow(reads: [
        for (final r in graph.wiring.reads)
          if (r.id == node.id)
            BridgeReadNode(
                id: r.id,
                item: r.item,
                customName: trimmed.isEmpty ? null : trimmed)
          else
            r,
      ]));
      return;
    }
    if (node.kind != BridgeCompNodeKind.fx) return;
    final List<BridgeEffectInstance> instances;
    try {
      instances = widget.comp.getNodeGraphInstances();
      for (final instance in instances) {
        if (instance.id() == node.id) instance.setCustomName(name: name);
      }
    } catch (_) {
      return;
    }
    _commit(_wiringNow(), instances: instances);
  }

  /// Delete every picked box but the Output, which the graph must always have.
  ///
  /// **Heal** decides what happens to the gap: on, whatever fed the box's
  /// `input` is joined to every input its `output` fed. Its wires, its
  /// position, its exposure and its group membership go in the same commit
  /// either way, so the document never holds a wire naming a box that is not
  /// there.
  bool _deleteSelected() {
    final graph = _graph;
    if (graph == null || _selection.isEmpty) return false;
    final victims = {
      for (final entry in _selection.entries)
        if (entry.value != graph.wiring.output) entry.key: entry.value,
    };
    if (victims.isEmpty) return false;

    var edges = [...graph.wiring.edges];
    for (final key in victims.keys) {
      final id = victims[key]!;
      if (_heal) {
        final feeder = edges
            .where((e) => e.to == id && e.toPort == 'input')
            .firstOrNull;
        if (feeder != null) {
          final onward = [
            for (final e in edges)
              if (e.from == id && e.fromPort == 'output') e,
          ];
          for (final e in onward) {
            edges.add(BridgeCompEdge(
                from: feeder.from,
                fromPort: feeder.fromPort,
                to: e.to,
                toPort: e.toPort));
          }
        }
      }
      edges = [
        for (final e in edges)
          if (!_touches(e, key)) e,
      ];
    }

    final ids = victims.values.toSet();
    final instances = <BridgeEffectInstance>[];
    try {
      for (final instance in widget.comp.getNodeGraphInstances()) {
        if (!ids.contains(instance.id())) instances.add(instance);
      }
    } catch (_) {
      return false;
    }
    for (final key in victims.keys) {
      _positions.remove(key);
      _selection.remove(key);
    }
    _publishPick();
    _commit(
      _wiringNow(
        reads: [
          for (final r in graph.wiring.reads)
            if (!ids.contains(r.id)) r,
        ],
        inputs: [
          for (final i in graph.wiring.inputs)
            if (!ids.contains(i.id)) i,
        ],
        edges: edges,
        exposed: [
          for (final e in graph.wiring.exposed)
            if (!ids.contains(e)) e,
        ],
        groups: [
          for (final g in graph.wiring.groups)
            BridgeCompNodeGroup(
              name: g.name,
              colour: g.colour,
              members: [
                for (final m in g.members)
                  if (!ids.contains(m)) m,
              ],
            ),
        ],
      ),
      instances: instances,
    );
    return true;
  }

  // --- Copy and paste -----------------------------------------------------

  /// Copy the picked boxes as saved-group text, never the Output. Where their
  /// top-left corner stood rides along, so a paste with the pointer elsewhere
  /// lands just off the originals.
  bool _copySelected(LumitUiState ui) {
    final graph = _graph;
    if (graph == null) return false;
    final ids = [
      for (final id in _selection.values)
        if (id != graph.wiring.output) id,
    ];
    if (ids.isEmpty) return false;
    final layout = _layout();
    final corners = [
      for (final id in ids)
        if (layout.byKey[compNodeKey(id)] case final box?) box.rect.topLeft,
    ];
    final String text;
    try {
      text = widget.comp.saveGraphGroup(name: '', colour: 0, nodes: ids);
    } catch (_) {
      return false;
    }
    ui.clipboard.boxes = (
      text: text,
      at: corners.fold(corners.firstOrNull ?? Offset.zero,
          (a, b) => Offset(math.min(a.dx, b.dx), math.min(a.dy, b.dy))),
    );
    return true;
  }

  /// Paste the copied boxes at the pointer, or beside where they were copied
  /// from when the pointer is off this canvas. One op, and they become the pick.
  bool _pasteBoxes(LumitUiState ui) {
    final held = ui.clipboard.boxes;
    if (_graph == null || held == null) return false;
    final at = _pointerOnCanvas ??
        held.at + const Offset(graphDotGrid * 2, graphDotGrid * 2);
    final List<UuidValue> ids;
    try {
      ids = widget.comp.pasteGraphBoxes(text: held.text, x: at.dx, y: at.dy);
    } catch (_) {
      // Refused whole, so the document is as it was.
      return true;
    }
    ui.model.refresh();
    _reload();
    setState(() => _pick(ids));
    return true;
  }

  /// Where the pointer is on this canvas, in canvas units, or null when it is
  /// somewhere else.
  Offset? get _pointerOnCanvas {
    final local = graphPointerIn(_canvasKey);
    return local == null ? null : _toCanvas(local);
  }

  // --- Dropping a box into a wire (N7) ------------------------------------

  ({BridgeCompEdge edge, GraphSocket into, GraphSocket outOf})? _dropInsert(
      GraphLayout layout) {
    final drag = _nodeDrag;
    final graph = _graph;
    if (drag == null || graph == null || drag.origins.length != 1) return null;
    final box = layout.byKey[drag.key];
    if (box == null) return null;
    if (graph.wiring.edges.any((e) => _touches(e, box.key))) return null;

    final at = (_positions[drag.key] ?? box.rect.topLeft) +
        Offset(box.rect.width / 2, box.rect.height / 2);
    for (final edge in graph.wiring.edges) {
      final ends = _edgeEnds(layout, edge);
      if (ends == null) continue;
      if (graphWireDistance(ends.$1, ends.$2, at) > graphWireGrab) continue;
      final into = _freeSocket(box, ends.$3, isInput: true);
      final outOf = _freeSocket(box, ends.$3, isInput: false);
      if (into != null && outOf != null) {
        return (edge: edge, into: into, outOf: outOf);
      }
    }
    return null;
  }

  GraphSocket? _freeSocket(GraphBox box, BridgePortType type,
      {required bool isInput}) {
    for (final port in isInput ? box.inputs : box.outputs) {
      if (isInput && port.wired) continue;
      if (isInput
          ? !compTypesFit(type, port.portType)
          : port.portType != type) {
        continue;
      }
      final at = box.socket(port.id, isInput);
      if (at != null) return GraphSocket(box.key, port, isInput, at);
    }
    return null;
  }

  // --- Pointer work -------------------------------------------------------

  Offset _toCanvas(Offset local) => (local - _pan) / _zoom;

  void _down(PointerDownEvent event, GraphLayout layout) {
    _canvasFocus.requestFocus();
    _menuPress = false;
    if (_claimed) {
      _claimed = false;
      return;
    }
    final at = _toCanvas(event.localPosition);
    _pressAt = event.localPosition;

    final socket = layout.socketAt(at);
    if (socket != null) {
      // A wire already there is grabbed by its **far** end: drop it on another
      // input to move it, or on nothing at all to take it off. An output draws
      // a new wire, which is what lets one output feed any number of inputs.
      final held = socket.isInput ? _edgeInto(socket) : null;
      final grabbed = held == null ? null : _sourceSocket(held, layout);
      setState(() => _flight = grabbed == null
          ? _Flight(socket, at)
          : _Flight(grabbed, at, detached: held));
      return;
    }

    final box = layout.boxAt(at);
    if (box != null) {
      final key = box.key;
      final node = _byKey[key];
      final again =
          _boxTaps.tap(at: event.localPosition, slop: 6) && _boxTapKey == key;
      _boxTapKey = key;
      if (again && node != null && _enterBox(node)) return;
      final keys = HardwareKeyboard.instance;
      final toggle = keys.isControlPressed || keys.isMetaPressed;
      final add = keys.isShiftPressed;
      final picked = _selection.containsKey(key);
      setState(() {
        if (node == null) {
          // The box has gone since the layout was made; nothing to pick.
        } else if (toggle) {
          if (_selection.remove(key) == null) _selection[key] = node.id;
          _publishPick();
        } else if (add) {
          _selection[key] = node.id;
          _publishPick();
        } else if (!picked) {
          _pick([node.id]);
        }
        final moving = picked && !toggle && !add
            ? _selection.keys
            : <String>[if (_selection.containsKey(key)) key];
        _nodeDrag = GraphNodeDrag(
          key,
          at,
          {
            for (final k in moving)
              k: _positions[k] ?? layout.byKey[k]?.rect.topLeft ?? Offset.zero,
          },
          collapse: picked && !toggle && !add,
        );
      });
      return;
    }

    if (event.buttons == kMiddleMouseButton) {
      setState(() => _panFrom = _pan - event.localPosition);
      return;
    }
    // A right-click on empty ground opens the console on release.
    if (event.buttons == kSecondaryMouseButton &&
        _ui!.workspace.interface.rightClickOpensNodeSearch) {
      _menuPress = true;
      return;
    }
    final keys = HardwareKeyboard.instance;
    final additive =
        keys.isShiftPressed || keys.isControlPressed || keys.isMetaPressed;
    setState(() {
      if (!additive) _pick(const []);
      _marqueeFrom = event.localPosition;
      _marqueeTo = null;
      _marqueeAdds = additive;
    });
  }

  /// A double-click on a box with an inside: a Read of a composition and a
  /// nested node graph both open that comp, as entering a precomp does.
  bool _enterBox(BridgeCompNode node) {
    final project = Provider.of<LumitState>(context, listen: false).project;
    if (node.item case ItemReference_Composition(:final field0)) {
      _ui?.setSelectedComp(field0);
      return true;
    }
    if (node.matchName != 'node_graph') return false;
    UuidValue? bound;
    try {
      for (final instance in widget.comp.getNodeGraphInstances()) {
        if (instance.id() == node.id) {
          bound = instance.nodeGraphCompId();
          break;
        }
      }
    } catch (_) {
      return false;
    }
    final inner = graphCompById(project, bound);
    if (inner == null) return false;
    _ui?.setSelectedComp(inner);
    return true;
  }

  void _move(PointerMoveEvent event, GraphLayout layout) {
    final at = _toCanvas(event.localPosition);
    if (_flight case final flight?) {
      setState(() => flight.to = at);
      return;
    }
    if (_nodeDrag case final drag?) {
      setState(() {
        var delta = at - drag.grab;
        final origin = _snapToGrid ? drag.origins[drag.key] : null;
        if (origin != null) {
          final raw = origin + delta;
          delta += Offset(
                (raw.dx / graphDotGrid).round() * graphDotGrid,
                (raw.dy / graphDotGrid).round() * graphDotGrid,
              ) -
              raw;
        }
        for (final entry in drag.origins.entries) {
          _positions[entry.key] = entry.value + delta;
        }
        _dropWire = _dropInsert(layout)?.edge;
      });
      return;
    }
    if (_marqueeFrom != null) {
      setState(() => _marqueeTo = event.localPosition);
      return;
    }
    if (_panFrom case final from?) {
      setState(() => _pan = from + event.localPosition);
    }
  }

  void _up(PointerUpEvent event, GraphLayout layout) {
    final at = _toCanvas(event.localPosition);
    final moved = _pressAt == null ||
        (event.localPosition - _pressAt!).distance > graphDragSlop;

    if (_flight case final flight?) {
      setState(() => _flight = null);
      final landed = layout.socketAt(at);
      if (flight.detached case final held?) {
        if (moved && landed != null && _accepts(flight.from, landed)) {
          _connect(flight.from, landed, without: held);
        } else {
          _removeEdge(held);
        }
        return;
      }
      if (landed == null) {
        // Onto empty canvas: the console opens with the wire still in hand.
        if (moved) _openSearch(at, wire: flight.from);
        return;
      }
      if (_accepts(flight.from, landed)) _connect(flight.from, landed);
      // A mismatched drop is simply declined: nothing crosses the bridge.
      return;
    }

    if (_nodeDrag case final drag?) {
      final insert = moved ? _dropInsert(layout) : null;
      setState(() {
        _nodeDrag = null;
        _dropWire = null;
        if (!moved && drag.collapse) {
          final id = _selection[drag.key];
          if (id != null) _pick([id]);
        }
      });
      if (insert != null) {
        final source = _byKey[insert.outOf.node]?.id;
        final dest = _byKey[insert.into.node]?.id;
        if (source != null && dest != null) {
          // The wire splits, and the box's new place rides the same write.
          _commit(_wiringNow(edges: [
            for (final e in _graph!.wiring.edges)
              if (e != insert.edge) e,
            BridgeCompEdge(
                from: insert.edge.from,
                fromPort: insert.edge.fromPort,
                to: dest,
                toPort: insert.into.port.id),
            BridgeCompEdge(
                from: source,
                fromPort: insert.outOf.port.id,
                to: insert.edge.to,
                toPort: insert.edge.toPort),
          ]));
          return;
        }
      }
      if (moved && _graph != null) _commit(_wiringNow());
      return;
    }

    if (_menuPress) {
      _menuPress = false;
      if (!moved) _openSearch(at);
      return;
    }

    if (_marqueeFrom case final from?) {
      final to = _marqueeTo;
      final adds = _marqueeAdds;
      setState(() {
        _marqueeFrom = null;
        _marqueeTo = null;
        if (to == null) return;
        // Wholly inside, the house rule for every rubber band here.
        final band = Rect.fromPoints(_toCanvas(from), _toCanvas(to));
        final caught = [
          for (final box in layout.boxes)
            if (band.contains(box.rect.topLeft) &&
                band.contains(box.rect.bottomRight))
              if (_byKey[box.key]?.id case final id?) id,
        ];
        _pick([if (adds) ..._selection.values, ...caught]);
      });
      return;
    }
    setState(() => _panFrom = null);
  }

  void _frameAll(Size viewport, GraphLayout layout) {
    if (layout.boxes.isEmpty) return;
    var bounds = layout.boxes.first.rect;
    for (final box in layout.boxes) {
      bounds = bounds.expandToInclude(box.rect);
    }
    bounds = bounds.inflate(20);
    final zoom = math
        .min(viewport.width / bounds.width, viewport.height / bounds.height)
        .clamp(0.2, 2.0);
    setState(() {
      _zoom = zoom;
      _pan = Offset(
        (viewport.width - bounds.width * zoom) / 2 - bounds.left * zoom,
        (viewport.height - bounds.height * zoom) / 2 - bounds.top * zoom,
      );
    });
  }

  void _wheel(PointerSignalEvent event) {
    if (event is! PointerScrollEvent) return;
    final was = _zoom;
    final next = (was * (event.scrollDelta.dy > 0 ? 0.9 : 1.1)).clamp(0.2, 2.0);
    if (next == was) return;
    final anchor = (event.localPosition - _pan) / was;
    setState(() {
      _zoom = next;
      _pan = event.localPosition - anchor * next;
    });
  }

  // --- Dropping from the project panel ------------------------------------

  void _dropped(Object data, Offset global) {
    final box = _canvasKey.currentContext?.findRenderObject();
    if (box is! RenderBox || _graph == null) return;
    final at = _toCanvas(box.globalToLocal(global));
    switch (data) {
      case FootageDragData(:final footage):
        _addRead([for (final clip in footage) ItemReference.footage(clip)], at);
      case CompDragData(comp: final dropped):
        // Never this graph itself: a box naming the comp it is in can only
        // ever degrade to a passthrough, and the console leaves it out too.
        if (dropped.internalid == widget.comp.internalid) return;
        var nodeGraph = false;
        try {
          nodeGraph = dropped.getModel().isNodeGraph;
        } catch (_) {
          return;
        }
        if (!nodeGraph) {
          _addRead([ItemReference.composition(dropped)], at);
          return;
        }
        // A node graph is applied rather than read, so it lands as a nested
        // Node graph box bound to that comp.
        final entry =
            listGraphNodes().where((e) => e.name == 'node_graph').firstOrNull;
        if (entry != null) _addFx(entry, at, graph: dropped);
      case EffectDragData(:final name):
        // From Effects & presets: the box the console would add, where it
        // was let go.
        final info = [
          ...(widget.effectsLister ?? listEffects)(),
          ...(widget.nodesLister ?? listGraphNodes)(),
        ].where((e) => e.name == name && name != 'node_graph').firstOrNull;
        if (info != null) _addFx(info, at);
    }
  }

  // --- Drawing ------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final graph = _graph;
    if (graph == null) {
      return PlaceholderPanel(
        icon: LumitIcon.nodes,
        title: l10n.panelGraph,
        hint: l10n.graphNoComp,
      );
    }
    final layout = _layout();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _toolbar(t, layout),
        Expanded(
          child: LayoutBuilder(
            builder: (context, box) => _canvas(t, layout, box.biggest),
          ),
        ),
      ],
    );
  }

  Widget _toolbar(LumitTheme t, GraphLayout layout) => Container(
        key: const ValueKey('comp-graph-toolbar'),
        height: graphToolbarHeight,
        color: t.surface1,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        child: Row(
          children: [
            const Spacer(),
            Text(l10n.graphAutoWire, style: t.kicker),
            const SizedBox(width: 10),
            HouseToggle(
              key: const ValueKey('comp-graph-auto-wire'),
              value: _autoWire,
              onChanged: (on) => setState(() => _autoWire = on),
            ),
            const SizedBox(width: 10),
            Text(l10n.graphHeal, style: t.kicker),
            const SizedBox(width: 10),
            HouseToggle(
              key: const ValueKey('comp-graph-heal'),
              value: _heal,
              onChanged: (on) => setState(() => _heal = on),
            ),
            const SizedBox(width: 10),
            LumitTooltip(
              message: _snapToGrid ? l10n.tipSnapOn : l10n.tipSnapOff,
              child: HouseButton(
                key: const ValueKey('comp-graph-snap'),
                small: true,
                frameless: !_snapToGrid,
                padding: const EdgeInsets.symmetric(horizontal: 4),
                onPressed: () => setState(() => _snapToGrid = !_snapToGrid),
                child: lumitIcon(LumitIcon.magnet,
                    size: graphIconSize,
                    color: _snapToGrid ? t.textPrimary : t.textMuted),
              ),
            ),
            const SizedBox(width: 10),
            LumitTooltip(
              message: l10n.graphFrameAll,
              child: GestureDetector(
                key: const ValueKey('comp-graph-frame-all'),
                behavior: HitTestBehavior.opaque,
                onTap: () => _frameAll(_viewport, layout),
                child: glyph.LumitIcon(LumitIcons.frameAll,
                    size: graphIconSize, colour: t.textMuted),
              ),
            ),
            const SizedBox(width: 10),
            Text(
              l10n.graphZoom((_zoom * 100).round()),
              key: const ValueKey('comp-graph-zoom'),
              style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
            ),
          ],
        ),
      );

  Widget _canvas(LumitTheme t, GraphLayout layout, Size size) {
    _viewport = size;
    return Focus(
      focusNode: _canvasFocus,
      onKeyEvent: (node, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        // Only the canvas itself: a value well on a box, or a rename field,
        // is typing these.
        if (node.hasPrimaryFocus &&
            (event.logicalKey == LogicalKeyboardKey.delete ||
                event.logicalKey == LogicalKeyboardKey.backspace)) {
          _deleteSelected();
          return KeyEventResult.handled;
        }
        // Only the canvas itself: a rename field inside it types these.
        if (node.hasPrimaryFocus &&
            graphAddKey(event, _ui!.workspace.interface)) {
          _openSearch(_pointerOnCanvas ??
              graphClearSpot(
                _toCanvas(Offset(_viewport.width / 2, _viewport.height / 2)),
                [for (final b in _layout().boxes) b.rect],
              ));
          return KeyEventResult.handled;
        }
        return KeyEventResult.ignored;
      },
      child: DragTarget<Object>(
        onWillAcceptWithDetails: (details) =>
            details.data is FootageDragData ||
            details.data is CompDragData ||
            details.data is EffectDragData,
        onAcceptWithDetails: (details) => _dropped(details.data, details.offset),
        builder: (context, candidate, _) => Listener(
          onPointerDown: (e) => _down(e, layout),
          onPointerMove: (e) => _move(e, layout),
          onPointerUp: (e) => _up(e, layout),
          onPointerSignal: _wheel,
          behavior: HitTestBehavior.opaque,
          child: ClipRect(
            child: Container(
              key: const ValueKey('comp-graph-canvas'),
              color: t.surface0,
              foregroundDecoration: candidate.isEmpty
                  ? null
                  : BoxDecoration(border: Border.all(color: t.accent, width: 2)),
              child: Stack(
                key: _canvasKey,
                clipBehavior: Clip.hardEdge,
                children: [
                  Positioned.fill(
                    child: RepaintBoundary(
                      child: CustomPaint(
                        key: const ValueKey('comp-graph-ground'),
                        painter: GraphGroundPainter(
                          pan: _pan,
                          zoom: _zoom,
                          grid: t.surface2,
                          ground: t.surface0,
                        ),
                      ),
                    ),
                  ),
                  Positioned.fill(
                    child: CustomPaint(
                      painter: _CompWirePainter(
                        wires: [
                          for (final edge in _graph!.wiring.edges)
                            if (_edgeEnds(layout, edge) case final ends?)
                              (
                                ends.$1,
                                ends.$2,
                                edge == _dropWire
                                    ? t.animated
                                    : portColour(t, ends.$3),
                                edge == _dropWire ? 2.0 : 1.0
                              ),
                        ],
                        flight: _flight == null
                            ? null
                            : (_flight!.from.at, _flight!.to),
                        dragged: t.textPrimary,
                        pan: _pan,
                        zoom: _zoom,
                      ),
                    ),
                  ),
                  Positioned.fill(
                    child: Transform(
                      transform: Matrix4.identity()
                        ..setEntry(0, 3, _pan.dx)
                        ..setEntry(1, 3, _pan.dy)
                        ..setEntry(0, 0, _zoom)
                        ..setEntry(1, 1, _zoom),
                      child: Stack(
                        clipBehavior: Clip.none,
                        children: [
                          for (final group in _graph!.wiring.groups)
                            if (graphGroupRect([
                              for (final member in group.members)
                                if (layout.byKey[compNodeKey(member)]
                                    case final box?)
                                  box.rect,
                            ])
                                case final rect?)
                              _groupWash(t, group, rect),
                          for (final box in layout.boxes)
                            Positioned(
                              left: box.rect.left,
                              top: box.rect.top,
                              child: GraphNodeCard(
                                box: box,
                                selected: _selection.containsKey(box.key),
                                exposed: _graph!.wiring.exposed
                                    .any((e) => compNodeKey(e) == box.key),
                                onOwnPress: () => _claimed = true,
                                onExpose: () => _toggleExposed(box.key),
                                onBypass: () =>
                                    _toggleBypass(box.key, box.card.enabled),
                                renaming: _renaming == box.key,
                                onStartRename: () =>
                                    setState(() => _renaming = box.key),
                                onRenamed: (name) => _rename(box.key, name),
                                onRenameCancelled: () =>
                                    setState(() => _renaming = null),
                                paramRow: (param) => _paramRow(box.key, param),
                              ),
                            ),
                        ],
                      ),
                    ),
                  ),
                  if (_marqueeFrom != null && _marqueeTo != null)
                    Positioned.fromRect(
                      key: const ValueKey('comp-graph-marquee'),
                      rect: Rect.fromPoints(_marqueeFrom!, _marqueeTo!),
                      child: const MarqueeBox(),
                    ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  /// One group's tinted wash and its name: the label palette, indexed, as the
  /// layer canvas draws it.
  Widget _groupWash(LumitTheme t, BridgeCompNodeGroup group, Rect rect) {
    final colour = t.labelColour(group.colour);
    return Positioned.fromRect(
      key: ValueKey<String>('comp-graph-group-${group.name}'),
      rect: rect,
      child: IgnorePointer(
        child: Container(
          decoration: BoxDecoration(
            color: colour.withValues(alpha: 0.05),
            border: Border.all(color: colour.withValues(alpha: 0.18)),
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          ),
          padding: EdgeInsets.fromLTRB(graphGroupPad, 3, graphGroupPad, 0),
          alignment: Alignment.topLeft,
          child: Text(group.name,
              style: t.kicker.copyWith(color: colour),
              maxLines: 1,
              overflow: TextOverflow.ellipsis),
        ),
      ),
    );
  }
}

/// Every wire on this canvas: the stored ones, coloured by the type they
/// carry, and the dashed one in hand.
class _CompWirePainter extends CustomPainter {
  final List<(Offset, Offset, Color, double)> wires;
  final (Offset, Offset)? flight;
  final Color dragged;
  final Offset pan;
  final double zoom;

  const _CompWirePainter({
    required this.wires,
    required this.flight,
    required this.dragged,
    required this.pan,
    required this.zoom,
  });

  Offset _screen(Offset canvas) => canvas * zoom + pan;

  @override
  void paint(Canvas canvas, Size size) {
    for (final (from, to, colour, weight) in wires) {
      _wire(canvas, from, to, colour, dashes: false, weight: weight);
    }
    if (flight case final f?) {
      _wire(canvas, f.$1, f.$2, dragged, dashes: true);
    }
  }

  void _wire(Canvas canvas, Offset from, Offset to, Color colour,
      {required bool dashes, double weight = 1}) {
    final path = graphWirePath(_screen(from), _screen(to), zoom: zoom);
    final paint = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = graphWireWidth * zoom * weight;
    canvas.drawPath(dashes ? graphDashPath(path) : path, paint);
  }

  @override
  bool shouldRepaint(_CompWirePainter old) => true;
}
