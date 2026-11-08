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

import 'dart:io' show File;
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
import 'package:lumit_flutter/src/rust/api/graph_looks.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart' show BridgeKeyContext;
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/wire_tie.dart' show BridgeWireTie;
import 'package:lumit_flutter/src/rust/lib.dart' show F64Array4;
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart' show LumitIcon;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/fx_console_frb.dart';
import '../state/dock.dart';
import '../state/drag_payloads.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/marquee.dart';
import 'node_menus.dart';
import 'graph_value_row.dart';
import 'effect_param_row_frb.dart'
    show
        cachedListParameterGroups,
        instanceParameters,
        defaultEffectValue,
        disabledParams,
        isDefaultEffectValue,
        paramGroupVisible,
        paramRidersFor,
        relistedRows;
import 'graph_cut_fade.dart';
import 'graph_groups.dart';
import 'graph_panel.dart';
import 'graph_ties.dart';
import 'placeholder.dart';
import 'shader_editor.dart' show InstanceHome;
import 'shader_graph.dart' show ShaderGraphThumb;
import 'timeline_extras_frb.dart' show DoubleTap, showCompTabMenuFrb;
import 'viewer_tool_cursor.dart' show handPanLayer;

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
/// Types must be equal, with two exceptions (§1.4): a **matte socket takes an
/// image wire** as well as a matte one, because what the row means by matte is
/// the picture's own channel, chosen by the row's Channel setting. A depth
/// socket takes one too, as a grey depth pass. The engine refuses that on Lens
/// flare, which only reads metres.
bool compTypesFit(BridgePortType from, BridgePortType into) =>
    from == into ||
    (into == BridgePortType.matte && from == BridgePortType.image) ||
    (into == BridgePortType.depth && from == BridgePortType.image);

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

/// What a box draws: every row it has, the rows an open box lists, the rows a
/// shut one keeps, the group heading drawn before a row, the riders folded
/// onto each row, and the rows another control has taken over.
typedef CompBoxRows = ({
  List<BridgeParamInfo> rows,
  List<BridgeParamInfo> open,
  List<BridgeParamInfo> telling,
  Map<String, String> kickers,
  Map<String, List<BridgeParamInfo>> riders,
  Set<String> disabled,
});

/// The rows a box draws, by the Effect controls panel's own rules, so a box
/// and the panel never disagree about what an effect is showing.
///
/// [params] is the schema's list with the instance's derived rows after it, and
/// [values], [hidden] and [greyed] are the instance's. Out go the rows a
/// plugin is hiding, the members of a group whose `visible_when` is unmet, the
/// riders that belong beside their host, and a curve, whose editor is no 24px
/// row. The rows a plugin has greyed are drawn disabled.
///
/// A **shut** box keeps the rows that tell something: a parameter a wire is
/// on ([wired] names the sockets), and one that is keyed or off its default.
/// An **open** box lists every row under its group's heading, except that a
/// group the schema folds shut shows only the rows that tell something.
/// [owner] is the layer the effect sits on, where a layer-valued row starts.
CompBoxRows compBoxRows(
  String effect,
  List<BridgeParamInfo> params,
  Map<String, BridgeEffectValue> values,
  Set<String> hidden, [
  Iterable<String> greyed = const [],
  Set<String> wired = const {},
  UuidValue? owner,
]) {
  final shown = [
    for (final p in params)
      if (!hidden.contains(p.id)) p,
  ];
  final groups = cachedListParameterGroups(effect);
  final gated = <String>{
    for (final g in groups)
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
  final rows = [
    for (final p in shown)
      if (!gated.contains(p.id) &&
          !folded.contains(p.id) &&
          p.kind is! BridgeParamKind_Curve)
        p,
  ];
  bool offDefault(BridgeParamInfo p) {
    final fallback = defaultEffectValue(p.kind);
    final value = values[p.id];
    return fallback != null &&
        value != null &&
        !isDefaultEffectValue(value, fallback, owner: owner);
  }

  bool tells(BridgeParamInfo p) =>
      wired.contains(p.id) || [p, ...?riders[p.id]].any(offDefault);
  final shut = <String>{
    for (final g in groups)
      if (g.collapsed && g.label.isNotEmpty) ...g.params,
  };
  // A rider rides beside its host in the panel; the box has no room for
  // that, so it takes the row after its host.
  List<BridgeParamInfo> withRiders(Iterable<BridgeParamInfo> hosts) => [
        for (final p in hosts) ...[p, ...?riders[p.id]],
      ];
  final open = withRiders([
    for (final p in rows)
      if (!shut.contains(p.id) || tells(p)) p,
  ]);
  final listed = {for (final p in open) p.id};
  final kickers = <String, String>{};
  for (final g in groups) {
    if (g.label.isEmpty) continue;
    final first = g.params.where(listed.contains).firstOrNull;
    if (first != null) kickers[first] = g.label;
  }
  return (
    rows: rows,
    open: open,
    telling: withRiders([for (final p in rows) if (tells(p)) p]),
    kickers: kickers,
    riders: riders,
    disabled: {...disabledParams(effect, values), ...greyed},
  );
}

/// Press one of a node graph box's engine-side buttons: Analyse and Cancel on
/// Remove background and Depth, Propagate and Cancel on a Roto brush.
///
/// These three read a file, and a box's file is the footage its picture comes
/// from. A press that cannot start says why in the status area, because the
/// box itself has nowhere to say it and a button that silently does nothing is
/// what this replaces. A refusal the effect names itself, a missing addon or
/// another analysis already running, is left to its status card, and so is a
/// file that is wired in and will not read.
void pressGraphBox(BuildContext context, CompositionReference comp,
    UuidValue effect, String param) {
  try {
    comp.fireGraphAction(effect: effect, param: param);
  } catch (_) {
    var wired = true;
    try {
      comp.graphBoxSource(effect: effect, frame: 0);
    } catch (_) {
      wired = false;
    }
    if (!wired) {
      Provider.of<LumitState>(context, listen: false)
          .postNotice(l10n.graphBoxNoFootage);
    }
  }
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

class _CompGraphPanelState extends State<CompGraphPanel>
    with GraphGroups<CompGraphPanel>, GraphTies<CompGraphPanel> {
  LumitUiState? _ui;

  /// The held graph. Read when the comp or the document changes, never in a
  /// build, which is what the budget test is guarding.
  BridgeCompGraph? _graph;

  /// The composition's name, for the toolbar. Read with the graph.
  String _name = '';

  /// Its boxes by key, so a gesture holding a key alone can reach the box.
  Map<String, BridgeCompNode> _byKey = const {};

  /// The Input declarations by key, for the kind each box wears as its kicker.
  Map<String, BridgeGraphInput> _inputs = const {};

  /// Each Fx box's values and the rows it lists, read once with the graph so
  /// an open box's controls draw from what the canvas holds: one `getInfo`
  /// per box per document change, and nothing on a rebuild.
  Map<String, BridgeEffectInstanceInfo> _infos = const {};
  Map<String, CompBoxRows> _params = const {};

  /// Canvas positions, staged: a drag moves this map and the release commits.
  Map<String, Offset> _positions = {};

  /// The widths edge drags gave boxes, by key.
  Map<String, double> _widths = {};

  /// A box edge in hand, and the edge the pointer is resting on.
  GraphEdgeDrag? _edgeDrag;
  bool _edgeHover = false;

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

  /// The next layout sets the view: on the first, and when the comp changes.
  bool _frameNext = true;

  /// The panel size the view was last set for. Until the user pans or zooms,
  /// a new size sets it again, so a panel that settles smaller than it first
  /// laid out does not leave the boxes off its edge.
  Size? _framedFor;
  bool _viewMoved = false;

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
      // The Node panel's index of boxes asks for a pick, and its heading
      // asks for a box's menu.
      ui.compGraphPickAsk.addListener(_onCompNodeAsked);
      ui.graphBoxMenuAsk.addListener(_onBoxMenuAsked);
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
      _frameNext = true;
      _viewMoved = false;
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
    return tieDelete() ||
        _deleteSelected() ||
        (_priorDeleteClaim?.call() ?? false);
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
    _ui?.compGraphPickAsk.removeListener(_onCompNodeAsked);
    _ui?.graphBoxMenuAsk.removeListener(_onBoxMenuAsked);
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
    _canvasFocus.dispose();
    super.dispose();
  }

  void _onSelectAllRequested() {
    final ui = _ui;
    if (!mounted || ui == null) return;
    if (!ui.selectAllRequestIsFor(widget.host)) return;
    setState(() => _pick([for (final n in _graph?.nodes ?? const []) n.id]));
  }

  /// Whether this canvas is the one the Node panel talks to: the Graph
  /// panel's, drawn. The Timeline's keeps its own pick, as it always has,
  /// and one the dock keeps alive behind a tab leaves it to the one in front.
  bool get _answersAsks => widget.host == Panel.graph && graphOnStage(context);

  /// A pick asked for from outside, by the Node panel's index: the box
  /// becomes the pick here, where the pick lives. An ask for no box is the
  /// panel going back to its index, which clears the pick with it.
  void _onCompNodeAsked() {
    final ask = _ui?.compGraphPickAsk.value;
    if (!mounted || ask == null || !_answersAsks) return;
    _ui!.compGraphPickAsk.value = null;
    final id = ask.id;
    if (id == null) {
      if (_selection.isNotEmpty) setState(() => _pick(const []));
      return;
    }
    final key = compNodeKey(id);
    if (_selection.containsKey(key) || !_byKey.containsKey(key)) return;
    setState(() => _pick([id]));
  }

  /// The Node panel's heading asking for a box's menu: the box becomes the
  /// pick, and the menu opens with every row the canvas has.
  void _onBoxMenuAsked() {
    final ask = _ui?.graphBoxMenuAsk.value;
    if (!mounted || ask == null || !_answersAsks) return;
    final node = _byKey[ask.key];
    if (node == null) return;
    _ui!.graphBoxMenuAsk.value = null;
    if (!_selection.containsKey(ask.key)) setState(() => _pick([node.id]));
    _boxMenu(ask.key, ask.at);
  }

  /// The one read. Everything the canvas draws comes from here.
  void _reload() {
    if (!mounted) return;
    BridgeCompGraph? graph;
    var name = '';
    try {
      name = widget.comp.getSettings().name;
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
    var notes = const <BridgeGraphNote>[];
    var ties = const <BridgeWireTie>[];
    BridgeCompNodeLooks? looks;
    if (graph != null) {
      try {
        notes = widget.comp.getNodeGraphNotes();
        ties = widget.comp.getNodeGraphTies();
        looks = widget.comp.getNodeGraphLooks();
        final ports = {for (final n in graph.nodes) n.id: n.inputs};
        for (final instance in widget.comp.getNodeGraphInstances()) {
          final id = instance.id();
          final key = compNodeKey(id);
          final info = instance.getInfo();
          infos[key] = info;
          params[key] = compBoxRows(
            info.name,
            relistedRows(
              instanceParameters(info),
              info.rowOptions,
            ),
            {for (final v in info.values) v.id: v.value},
            info.hiddenRows.toSet(),
            info.disabledRows,
            {for (final p in ports[id] ?? const <BridgePort>[]) if (p.wired) p.id},
          );
        }
      } catch (_) {
        // The comp moved under us; the boxes draw their sockets and the
        // next read fills the rows in.
      }
    }
    setState(() {
      _graph = graph;
      _name = name;
      _notes = notes;
      _ties = ties;
      _colours = {
        for (final c in looks?.colours ?? const <BridgeCompNodeColour>[])
          compNodeKey(c.node): c.colour,
      };
      _folded = {...?looks?.collapsed.map(compNodeKey)};
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
      _widths = {
        for (final p
            in graph?.wiring.layout ?? const <BridgeCompNodePosition>[])
          if (p.width case final w?) compNodeKey(p.node): w,
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
    groupPick = null;
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
            BridgeCompNodePosition(
                node: id,
                x: e.value.dx,
                y: e.value.dy,
                width: _widths[e.key]),
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
      // An open box gives every parameter a row and is wider for them; a
      // shut one keeps the rows that tell something, and a box with nothing
      // to say keeps the drawing's width.
      final held = _params[key];
      final rows = held == null
          ? const <BridgeParamInfo>[]
          : open
              ? held.open
              : held.telling;
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
          noStream: graphNoStreamPorts(node.inputs, node.matchName),
        ),
        placed,
        open: open,
        params: [for (final p in rows) p.id],
        kickers: open ? held?.kickers ?? const {} : const {},
        width: graphBoxWidth(
            node.kind == BridgeCompNodeKind.output
            ? graphOutNodeWidth
            : rows.isEmpty
                ? graphNodeWidth
                : graphNodeOpenWidth,
            _widths[key],
            open: rows.isNotEmpty),
        foot: node.matchName == 'custom_shader' ? graphShaderFoot : 0,
        colour: _colours[key] ?? 0,
        collapsed: _folded.contains(key),
      ));
    }
    return GraphLayout(boxes);
  }

  /// A Custom shader box's picture of its inner graph, which a click opens.
  /// Null for every other box.
  Widget? _shaderThumb(String key) {
    final node = _byKey[key];
    if (node == null || node.matchName != 'custom_shader') return null;
    return ShaderGraphThumb(
      home: InstanceHome.graph(widget.comp),
      effect: node.id,
      height: graphShaderThumbHeight,
      onOpen: () => _enterBox(node),
    );
  }

  // --- A box's own controls -----------------------------------------------

  /// One parameter row on an open box: its name and its value at the
  /// playhead, read from the values the canvas holds. It only reads; the Node
  /// panel's row is where the value is set.
  Widget _paramRow(String key, String param) {
    final info = _infos[key];
    final held = _params[key];
    final p = held?.open.where((p) => p.id == param).firstOrNull;
    final ui = _ui;
    if (info == null || held == null || p == null || ui == null) {
      return const SizedBox.shrink();
    }
    return GraphValueRow(
      key: ValueKey<String>('graph-row-$key-$param'),
      param: p,
      value: info.values.where((v) => v.id == param).firstOrNull?.value,
      comp: widget.comp,
      frame: ui.playheadFrame.value,
      // Greyed where another control has taken the row over.
      enabled: !held.disabled.contains(param),
    );
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
  /// [insertOn] is the stored wire the box is going into the middle of: its
  /// old source feeds the box's first input of the wire's type.
  List<BridgeCompEdge> _wiredIn(
    UuidValue made,
    List<BridgePort> inputs,
    List<BridgePort> outputs,
    GraphSocket? wire,
    UuidValue? after, {
    BridgeCompEdge? insertOn,
  }) {
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
        if (insertOn != null) {
          final gives = _byKey[compNodeKey(insertOn.from)]
              ?.outputs
              .where((p) => p.id == insertOn.fromPort)
              .firstOrNull
              ?.portType;
          final takes = gives == null
              ? null
              : inputs
                  .where((p) => compTypesFit(gives, p.portType))
                  .firstOrNull;
          if (takes != null) {
            edges.add(BridgeCompEdge(
                from: insertOn.from,
                fromPort: insertOn.fromPort,
                to: made,
                toPort: takes.id));
          }
        }
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
      {GraphSocket? wire, CompositionReference? graph, BridgeCompEdge? insertOn}) {
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
        edges: _wiredIn(id, inputs, outputs, wire, after, insertOn: insertOn),
        // A new box starts shut; a box already in a saved graph keeps
        // whatever the file says.
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
  /// [insertOn] is the stored wire whose input end [wire] is: the box
  /// chosen goes between the wire's two ends.
  Future<void> _openSearch(Offset at,
      {GraphSocket? wire, BridgeCompEdge? insertOn}) async {
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
          run: () => _addFx(nested, at,
              wire: wire, graph: inner, insertOn: insertOn),
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
        run: () => _addFx(info, at, wire: wire, insertOn: insertOn),
      ));
    }
    // The saved groups, last. A wire in hand has no one socket of a whole
    // group to land on, so they are left out while one is held.
    if (wire == null) {
      for (final saved in listGraphGroups()) {
        entries.add(FxConsoleEntry(
          label: saved.name,
          kind: FxConsoleKind.effect,
          group: l10n.graphGroup,
          run: () => _insertGroup(saved.path, at),
        ));
      }
    }
    try {
      await showFxConsoleFrb(
        context: context,
        anchor: lastKnownPointerPosition,
        model: FxConsoleModel(
          keyHint:
              wire == null ? _ui?.keymap.chordFor('console.open') : null,
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

  /// M on the canvas: the picked Fx boxes are bypassed, or brought back when
  /// the last one picked already is. Only an Fx box has a bypass.
  void _mutePicked() {
    for (final key in _selection.keys.toList().reversed) {
      final node = _byKey[key];
      if (node == null || node.kind != BridgeCompNodeKind.fx) continue;
      _toggleBypass(key, node.enabled);
      return;
    }
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
        groups: _bridgeGroups(groupsWithout(victims.keys)),
      ),
      instances: instances,
    );
    return true;
  }

  // --- Repeats ------------------------------------------------------------

  /// A frame round each Repeat start, its end and the boxes between them,
  /// under everything else. Worked out from the boxes each build, as a
  /// group's is, and it takes no pointer.
  List<Widget> _repeatFrames(LumitTheme t) => [
        for (final r in _graph?.repeats ?? const <BridgeCompRepeat>[])
          if (graphGroupRect([
            for (final id in [r.start, ...r.members, r.end])
              if (_boxRects[compNodeKey(id)] case final rect?) rect,
          ], head: 0)
              case final rect?)
            Positioned.fromRect(
              rect: rect.inflate(graphGroupPad),
              child: IgnorePointer(
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    color: t.accent.withValues(alpha: 0.04),
                    border: Border.all(color: t.accent.withValues(alpha: 0.45)),
                    borderRadius:
                        BorderRadius.all(Radius.circular(t.tokens.cardRadius)),
                  ),
                ),
              ),
            ),
      ];

  // --- Groups -------------------------------------------------------------

  @override
  List<GraphGroup> get groups => [
        for (final g
            in _graph?.wiring.groups ?? const <BridgeCompNodeGroup>[])
          GraphGroup(g.name, g.colour, g.members.map(compNodeKey).toList()),
      ];

  List<BridgeCompNodeGroup> _bridgeGroups(List<GraphGroup> groups) => [
        for (final g in groups)
          BridgeCompNodeGroup(name: g.name, colour: g.colour, members: [
            for (final key in g.members)
              if (_byKey[key]?.id case final id?) id,
          ]),
      ];

  @override
  void commitGroups(List<GraphGroup> next) {
    if (_graph == null) return;
    // A saved group keeps each box's place, so a box nobody has dragged is
    // given the one it is drawn at.
    for (final g in next) {
      for (final key in g.members) {
        if (_boxRects[key] case final rect?) {
          _positions.putIfAbsent(key, () => rect.topLeft);
        }
      }
    }
    _commit(_wiringNow(groups: _bridgeGroups(next)));
  }

  /// Where each box was drawn by the last build.
  Map<String, Rect> _boxRects = const {};

  @override
  Map<String, Rect> get boxRects => _boxRects;

  @override
  List<String> get pickedKeys => _selection.keys.toList();

  @override
  void pickKeys(Iterable<String> keys) => _pick([
        for (final key in keys)
          if (_byKey[key]?.id case final id?) id,
      ]);

  /// Every graph has its own Output, so a saved group leaves it behind.
  @override
  bool groupCarries(String key) {
    final id = _byKey[key]?.id;
    return id != null && id != _graph?.wiring.output;
  }

  @override
  String groupText(String name, int colour, List<String> members) =>
      widget.comp.saveGraphGroup(name: name, colour: colour, nodes: [
        for (final key in members)
          if (_byKey[key]?.id case final id?) id,
      ]);

  @override
  String get groupExtension => 'lumngrp';

  @override
  List<BridgePresetInfo> savedGroups() => listGraphGroups();

  @override
  void insertSavedGroup(String path) => _insertGroup(path, noteSpot);

  /// The held notes, read with the graph.
  List<BridgeGraphNote> _notes = const [];

  @override
  List<BridgeGraphNote> get notes => _notes;

  @override
  void commitNotes(List<BridgeGraphNote> next) {
    try {
      widget.comp.setNodeGraphNotes(notes: next);
    } catch (_) {
      // Refused, or the comp moved under us. Re-reading is the recovery.
    }
    _ui?.model.refresh();
    _reload();
  }

  /// The held ties, read with the graph.
  List<BridgeWireTie> _ties = const [];

  @override
  List<BridgeWireTie> get ties => _ties;

  @override
  void commitTies(List<BridgeWireTie> next) {
    try {
      widget.comp.setNodeGraphTies(ties: next);
    } catch (_) {
      // Refused, or the comp moved under us. Re-reading is the recovery.
    }
    _ui?.model.refresh();
    _reload();
  }

  @override
  double get tieZoom => _zoom;

  /// Every wire as the ties name it, and where it runs.
  List<(TieWire, BridgeCompEdge, BridgePortType)> _tieWires(
          GraphLayout layout) =>
      [
        for (final edge in _graph?.wiring.edges ?? const <BridgeCompEdge>[])
          if (_edgeEnds(layout, edge) case final ends?)
            (
              (
                key: tieWireKey(_toKey(edge), edge.toPort),
                from: ends.$1,
                to: ends.$2
              ),
              edge,
              ends.$3,
            ),
      ];

  List<TieWire> _wiresOf(GraphLayout layout) =>
      [for (final (w, _, _) in _tieWires(layout)) w];

  @override
  List<TieWire> tieWiresNow() => _wiresOf(_layout());

  @override
  Offset get noteSpot => graphClearSpot(
        _toCanvas(Offset(_viewport.width / 2, _viewport.height / 2)),
        [for (final b in _layout().boxes) b.rect],
      );

  @override
  bool get groupSnaps => _snapToGrid;

  @override
  double get groupZoom => _zoom;

  @override
  LumitUiState? get groupUi => _ui;

  /// Drop a saved group at `at`: one op, so one undo step however many boxes
  /// and wires it carries.
  void _insertGroup(String path, Offset at) {
    try {
      widget.comp.insertGraphGroup(
          text: File(path).readAsStringSync(), x: at.dx, y: at.dy);
    } catch (_) {
      // The file has gone or is not a group, so nothing was added.
      return;
    }
    _ui?.model.refresh();
    _reload();
  }

  // --- Menus --------------------------------------------------------------

  /// The box a right press landed on, whose menu opens if the release never
  /// moved.
  String? _menuBox;

  /// A box's menu, over the pick as it stands: the right press made an
  /// unpicked box the pick, so every row acts on the whole pick.
  void _boxMenu(String key, Offset at) {
    final graph = _graph;
    final node = _byKey[key];
    final ui = _ui;
    if (graph == null || node == null || ui == null) return;
    final fx = node.kind == BridgeCompNodeKind.fx;
    final output = node.id == graph.wiring.output;
    final picked = _selection.keys.toSet();
    final wired = graph.wiring.edges.any((e) => picked.any((k) => _touches(e, k)));
    final enter = switch (node) {
      BridgeCompNode(matchName: 'custom_shader') => l10n.shaderEditorTitle,
      BridgeCompNode(matchName: 'node_graph') => l10n.fxOpenGraph,
      BridgeCompNode(item: ItemReference_Composition()) => l10n.graphOpenComp,
      _ => null,
    };
    // The Output alone is the picture: what it can do is take a box in front
    // of it, and a group of one is no group.
    final outputAlone = output && picked.length == 1;
    showGraphBoxMenu(
      context,
      at,
      bypassed: fx ? !node.enabled : null,
      onBypass: fx ? () => _toggleBypass(key, node.enabled) : null,
      onRename: fx || node.kind == BridgeCompNodeKind.read
          ? () => setState(() => _renaming = key)
          : null,
      expanded: fx && !_folded.contains(key)
          ? graph.wiring.exposed.any((e) => compNodeKey(e) == key)
          : null,
      onExpand: fx && !_folded.contains(key) ? () => _toggleExposed(key) : null,
      enterLabel: enter,
      onEnter: enter == null ? null : () => _enterBox(node),
      onAddNode: outputAlone
          ? () => _feedOutput(key)
          : picked.length > 1 || node.outputs.isEmpty
              ? null
              : () => _addAfter(key),
      onCopy: outputAlone ? null : () => _copySelected(ui),
      onPaste: () => _pasteBoxes(ui),
      canPaste: ui.clipboard.boxes != null,
      onDuplicate: outputAlone ? null : () => _duplicate(ui),
      onGroup: outputAlone ? null : groupPicked,
      onSaveGroup: outputAlone ? null : () => saveGroup(),
      canSaveGroup: canSaveGroup,
      onUngroup: outputAlone &&
              !groups.any((g) => g.members.any(picked.contains))
          ? null
          : ungroupPicked,
      canUngroup: groups.any((g) => g.members.any(picked.contains)),
      // Every box that makes a picture but the Output, which is the picture
      // the Viewer shows anyway. The right-click has picked the box, so the
      // chip's point is this one.
      viewHere: output || ui.viewerGraphPoint == null
          ? null
          : ui.atSelectedEffect.value,
      onViewHere: output || ui.viewerGraphPoint == null
          ? null
          : () => ui.setAtSelectedEffect(!ui.atSelectedEffect.value),
      onDisconnectAll: wired ? _disconnectPicked : null,
      collapsed: _folded.contains(key),
      onCollapse: () => _restyle(key, collapsed: !_folded.contains(key)),
      colour: _colours[key] ?? 0,
      onColour: (colour) => _restyle(key, colour: colour),
      onDelete: outputAlone ? null : _deleteSelected,
    );
  }

  /// Each box's colour and fold, read with the graph, by box key.
  Map<String, int> _colours = const {};
  Set<String> _folded = const {};

  /// Colour or fold the pressed box, and the rest of the pick with it, as one
  /// undo step.
  void _restyle(String key, {int? colour, bool? collapsed}) {
    final nodes = [
      for (final k in _targets(key))
        if (_byKey[k]?.id case final id?) id,
    ];
    try {
      if (colour != null) {
        widget.comp.setNodeGraphBoxColour(nodes: nodes, colour: colour);
      }
      if (collapsed != null) {
        widget.comp
            .setNodeGraphBoxCollapsed(nodes: nodes, collapsed: collapsed);
      }
    } catch (_) {
      // The comp moved under us. Re-reading is the recovery.
    }
    _ui?.model.refresh();
    _reload();
  }

  /// The search, its pick landing in front of the Output: on the wire into it
  /// where there is one, wired straight in where there isn't.
  void _feedOutput(String key) {
    final box = _layout().byKey[key];
    final socket = box?.inputs.firstOrNull;
    final centre = socket == null ? null : box?.socket(socket.id, true);
    if (box == null || socket == null || centre == null) return;
    final into = GraphSocket(key, socket, true, centre);
    final edge = _edgeInto(into);
    final at = box.rect.topLeft - const Offset(graphOutNodeWidth + 60, 0);
    _openSearch(at, wire: into, insertOn: edge);
  }

  /// Add node… after a box: on the wire leaving it, as the wire's Insert
  /// node… does, or wired from its output when nothing leaves it.
  void _addAfter(String key) {
    final layout = _layout();
    final box = layout.byKey[key];
    final id = _byKey[key]?.id;
    final out = box?.outputs.firstOrNull;
    final from = out == null ? null : box!.socket(out.id, false);
    if (box == null || id == null || out == null || from == null) return;
    final at = graphClearSpot(box.rect.topRight + const Offset(48, 0),
        [for (final b in layout.boxes) b.rect]);
    final edge = _graph!.wiring.edges
        .where((e) => e.from == id && e.fromPort == out.id)
        .firstOrNull;
    final next = edge == null ? null : layout.byKey[_toKey(edge)];
    final socket = next?.inputs.where((p) => p.id == edge!.toPort).firstOrNull;
    final into = socket == null ? null : next!.socket(socket.id, true);
    if (edge != null && socket != null && into != null) {
      _openSearch(at,
          wire: GraphSocket(_toKey(edge), socket, true, into), insertOn: edge);
    } else {
      _openSearch(at, wire: GraphSocket(key, out, false, from));
    }
  }

  /// A socket's menu: its wire off, and the box's unused sockets shown or
  /// hidden.
  void _socketMenu(GraphSocket socket, Offset at) {
    final graph = _graph;
    final node = _byKey[socket.node];
    if (graph == null || node == null) return;
    VoidCallback? disconnect;
    if (socket.isInput) {
      if (_edgeInto(socket) case final edge?) {
        disconnect = () => _removeEdge(edge);
      }
    } else if (graph.wiring.edges
        .any((e) => e.from == node.id && e.fromPort == socket.port.id)) {
      disconnect = () => _commit(_wiringNow(edges: [
            for (final e in graph.wiring.edges)
              if (e.from != node.id || e.fromPort != socket.port.id) e,
          ]));
    }
    final fx = node.kind == BridgeCompNodeKind.fx;
    showGraphSocketMenu(
      context,
      at,
      onDisconnect: disconnect,
      expanded: fx
          ? graph.wiring.exposed.any((e) => compNodeKey(e) == socket.node)
          : null,
      onExpand: fx ? () => _toggleExposed(socket.node) : null,
    );
  }

  /// The stored wire nearest [at], within a wire's grab distance.
  BridgeCompEdge? _wireAt(GraphLayout layout, Offset at) {
    final all = _tieWires(layout);
    final wires = [for (final (w, _, _) in all) w];
    BridgeCompEdge? best;
    var nearest = graphWireGrab;
    for (final (w, edge, _) in all) {
      final d = tieWireDistance(w, at, wires);
      if (d < nearest) {
        nearest = d;
        best = edge;
      }
    }
    return best;
  }

  /// A wire's menu: a box between its two ends, a tie on it, or off.
  void _wireMenu(
      GraphLayout layout, BridgeCompEdge edge, Offset at, Offset position) {
    // The wire's input end, which the search holds in hand.
    final box = layout.byKey[_toKey(edge)];
    final socket = box?.inputs.where((p) => p.id == edge.toPort).firstOrNull;
    final centre = box?.socket(edge.toPort, true);
    final into = socket == null || centre == null
        ? null
        : GraphSocket(_toKey(edge), socket, true, centre);
    showGraphWireMenu(
      context,
      position,
      onInsert: into == null
          ? null
          : () => _openSearch(at, wire: into, insertOn: edge),
      onReroute: () => tieAddPoint(tieWireKey(_toKey(edge), edge.toPort), at),
      onDelete: () => _removeEdge(edge),
    );
  }

  void _canvasMenu(GraphLayout layout, Offset at, Offset position) {
    final ui = _ui;
    if (ui == null) return;
    showGraphCanvasMenu(
      context,
      position,
      onAddNode: () => _openSearch(at),
      onPaste: () => _pasteBoxes(ui),
      canPaste: ui.clipboard.boxes != null,
      onSelectAll: () => setState(
          () => _pick([for (final n in _graph?.nodes ?? const []) n.id])),
      onFrameSelected: () =>
          _frameByHand(() => _frameSelected(_viewport, layout)),
      canFrameSelected: _selection.isNotEmpty,
      onFrameAll: () => _frameByHand(() => _frameAll(_viewport, layout)),
      onAddNote: addNote,
      onGroup: groupPicked,
      canGroup: canGroup,
      onCollapseAll: _byKey.isEmpty ? null : () => _foldAll(true),
      canCollapseAll: _byKey.keys.any((k) => !_folded.contains(k)),
      onExpandAll: _byKey.isEmpty ? null : () => _foldAll(false),
      canExpandAll: _byKey.keys.any(_folded.contains),
    );
  }

  /// Fold or open every box, as one undo step.
  void _foldAll(bool collapsed) {
    try {
      widget.comp.setNodeGraphBoxCollapsed(
          nodes: [for (final n in _byKey.values) n.id], collapsed: collapsed);
    } catch (_) {
      // The comp moved under us. Re-reading is the recovery.
    }
    _ui?.model.refresh();
    _reload();
  }

  /// Every wire off the picked boxes, in one write.
  void _disconnectPicked() {
    final picked = _selection.keys.toSet();
    _commit(_wiringNow(edges: [
      for (final e in _graph!.wiring.edges)
        if (!picked.any((k) => _touches(e, k))) e,
    ]));
  }

  /// The picked boxes again, beside the originals: a copy and a paste in
  /// one, with the clipboard left as it was.
  void _duplicate(LumitUiState ui) {
    final was = ui.clipboard.boxes;
    if (!_copySelected(ui)) return;
    final held = ui.clipboard.boxes;
    ui.clipboard.boxes = was;
    if (held == null || _graph == null) return;
    final at = held.at + const Offset(graphDotGrid * 2, graphDotGrid * 2);
    final List<UuidValue> ids;
    try {
      ids = widget.comp.pasteGraphBoxes(text: held.text, x: at.dx, y: at.dy);
    } catch (_) {
      return;
    }
    ui.model.refresh();
    _reload();
    setState(() => _pick(ids));
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
    // Measured along each wire as its ties route it.
    final wires = _wiresOf(layout);
    for (final edge in graph.wiring.edges) {
      final ends = _edgeEnds(layout, edge);
      if (ends == null) continue;
      final wire = (key: tieWireKey(_toKey(edge), edge.toPort), from: ends.$1, to: ends.$2);
      if (tieWireDistance(wire, at, wires) > graphWireGrab) continue;
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
    // A press in a name or a note being typed leaves the keyboard there.
    if (!groupTypingPress()) _canvasFocus.requestFocus();
    _menuPress = false;
    if (_claimed) {
      _claimed = false;
      return;
    }
    final at = _toCanvas(event.localPosition);
    _pressAt = event.localPosition;

    // Ctrl and a press on a point is a press on the socket its wires leave.
    final socket =
        layout.socketAt(tieWireFrom(event, at, _wiresOf(layout)) ?? at);
    // A group's bar, unless a socket is poking out over it.
    if (groupDown(event, at, (drag) => _nodeDrag = drag,
        unless: socket != null)) {
      return;
    }
    // **A box's left or right edge drags its width**, as on the layer
    // canvas; a socket on the edge still wins the press.
    if (socket == null && event.buttons == kPrimaryMouseButton) {
      if (graphEdgeAt(layout, at, _zoom) case final edge?) {
        setState(() => _edgeDrag = GraphEdgeDrag(
            edge.box.key, edge.left, at.dx, edge.box.rect,
            open: edge.box.hasControls));
        return;
      }
    }
    // The right button asks a socket rather than pulling on it.
    if (socket != null && event.buttons == kSecondaryMouseButton) {
      _socketMenu(socket, event.position);
      return;
    }
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
      // A right press picks and grabs as the left does, and a release that
      // never moved opens the box's menu. It counts for no double-click.
      final right = event.buttons == kSecondaryMouseButton;
      final again = !right &&
          _boxTaps.tap(at: event.localPosition, slop: 6) &&
          _boxTapKey == key;
      _boxTapKey = key;
      if (again && node != null && _enterBox(node)) return;
      final keys = HardwareKeyboard.instance;
      final toggle = !right && (keys.isControlPressed || keys.isMetaPressed);
      final add = !right && keys.isShiftPressed;
      final picked = _selection.containsKey(key);
      _menuBox = right ? key : null;
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
          // A menu is about the pick as it stands, so a right press on a
          // standing pick leaves it whole.
          collapse: picked && !toggle && !add && !right,
        );
      });
      return;
    }

    if (event.buttons == kMiddleMouseButton) {
      setState(() => _panFrom = _pan - event.localPosition);
      return;
    }
    // A right-click on empty ground opens a menu, or the console, on
    // release, and a right-button drag ties the wires it crosses.
    if (tieDown(event, at, _wiresOf(layout))) {
      _menuPress = event.buttons == kSecondaryMouseButton;
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
  /// nested node graph both open that comp, as entering a precomp does, and a
  /// Custom shader opens its inner graph.
  bool _enterBox(BridgeCompNode node) {
    final project = Provider.of<LumitState>(context, listen: false).project;
    if (node.item case ItemReference_Composition(:final field0)) {
      _ui?.setSelectedComp(field0);
      return true;
    }
    if (node.matchName == 'custom_shader') {
      _ui?.enterShaderGraphInComp(widget.comp, node.id,
          effectName: node.customName ?? engineLabel(node.label));
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
    if (_edgeDrag case final edge?) {
      final next = edge.at(at.dx);
      setState(() {
        _widths[edge.key] = next.width;
        _positions[edge.key] = Offset(next.x, edge.from.top);
      });
      return;
    }
    if (_flight case final flight?) {
      setState(() {
        flight.to = at;
        flight.snap = layout.snapAt(at, (s) => _accepts(flight.from, s));
      });
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
        groupDragOver(drag.origins.keys, at);
      });
      return;
    }
    if (tieMove(at, _wiresOf(layout))) return;
    if (_marqueeFrom != null) {
      setState(() => _marqueeTo = event.localPosition);
      return;
    }
    if (_panFrom case final from?) {
      _viewMoved = true;
      setState(() => _pan = from + event.localPosition);
    }
  }

  void _up(PointerUpEvent event, GraphLayout layout) {
    final at = _toCanvas(event.localPosition);
    final moved = _pressAt == null ||
        (event.localPosition - _pressAt!).distance > graphDragSlop;

    if (_edgeDrag case final edge?) {
      // A box's new width, kept with its position: one commit, one undo step.
      setState(() => _edgeDrag = null);
      if (moved) {
        _positions.putIfAbsent(edge.key, () => edge.from.topLeft);
        _commit(_wiringNow());
      }
      return;
    }

    if (_flight case final flight?) {
      setState(() => _flight = null);
      tieWireLanded();
      final landed = layout.snapAt(at, (s) => _accepts(flight.from, s)) ??
          layout.socketAt(at);
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
      // Boxes let go over a group's frame move into it, in the same write
      // as their new place.
      final joined = groupsAfterDrop(moved ? drag.origins.keys : const [], at);
      setState(() {
        _nodeDrag = null;
        _dropWire = null;
        if (!moved && drag.collapse) {
          final id = _selection[drag.key];
          if (id != null) _pick([id]);
        }
      });
      final menuBox = _menuBox;
      _menuBox = null;
      if (!moved && menuBox != null) {
        _boxMenu(menuBox, event.position);
        return;
      }
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
      if (moved && _graph != null) {
        _commit(_wiringNow(
            groups: joined == null ? null : _bridgeGroups(joined)));
      }
      return;
    }

    if (tieUp(at, moved, _wiresOf(layout), menuAt: event.position)) {
      _menuPress = false;
      return;
    }
    if (_menuPress) {
      _menuPress = false;
      if (moved || _graph == null) return;
      // A wire under the pointer takes the click; the ground opens the
      // console or its own menu, as the setting says.
      if (_wireAt(layout, at) case final edge?) {
        _wireMenu(layout, edge, at, event.position);
      } else if (_ui!.workspace.interface.rightClickOpensNodeSearch) {
        _openSearch(at);
      } else {
        _canvasMenu(layout, at, event.position);
      }
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

  void _frameAll(Size viewport, GraphLayout layout, {double maxZoom = 2}) =>
      _frame(graphFrame(viewport, [for (final b in layout.boxes) b.rect],
          maxZoom: maxZoom));

  /// The picked boxes, or every box when none is picked.
  void _frameSelected(Size viewport, GraphLayout layout) => _frame(graphFrame(
      viewport,
      [
        for (final b in layout.boxes)
          if (_selection.isEmpty || _selection.containsKey(b.key)) b.rect
      ]));

  void _frame(({Offset pan, double zoom})? view) {
    if (view == null) return;
    setState(() {
      _zoom = view.zoom;
      _pan = view.pan;
    });
  }

  /// Framing asked for by hand, which the panel's own resizes then leave be.
  void _frameByHand(VoidCallback frame) {
    _viewMoved = true;
    frame();
  }

  void _wheel(PointerSignalEvent event) {
    if (event is! PointerScrollEvent) return;
    final was = _zoom;
    final next = (was * (event.scrollDelta.dy > 0 ? 0.9 : 1.1)).clamp(0.2, 2.0);
    if (next == was) return;
    final anchor = (event.localPosition - _pan) / was;
    _viewMoved = true;
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
    _boxRects = {for (final box in layout.boxes) box.key: box.rect};
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _toolbarMenu(t, layout),
        if (groupShelf(t) case final shelf?) shelf,
        Expanded(
          child: LayoutBuilder(
            builder: (context, box) => _canvas(t, layout, box.biggest),
          ),
        ),
      ],
    );
  }

  /// The toolbar, whose right-click lists its switches and the framing.
  Widget _toolbarMenu(LumitTheme t, GraphLayout layout) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onSecondaryTapUp: (d) => showGraphToolbarMenu(
          context,
          d.globalPosition,
          autoWire: _autoWire,
          onAutoWire: () => setState(() => _autoWire = !_autoWire),
          heal: _heal,
          onHeal: () => setState(() => _heal = !_heal),
          snap: _snapToGrid,
          onSnap: () => setState(() => _snapToGrid = !_snapToGrid),
          onFrameSelected: () =>
              _frameByHand(() => _frameSelected(_viewport, layout)),
          canFrameSelected: _selection.isNotEmpty,
          onFrameAll: () => _frameByHand(() => _frameAll(_viewport, layout)),
        ),
        child: _toolbar(t, layout),
      );

  Widget _toolbar(LumitTheme t, GraphLayout layout) => Container(
        key: const ValueKey('comp-graph-toolbar'),
        height: graphToolbarHeight,
        color: t.surface1,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        child: Row(
          children: [
            // The graph's name is the composition: its menu, as its tab has.
            Expanded(
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onSecondaryTapUp: (d) => showCompTabMenuFrb(
                  context: context,
                  comp: widget.comp,
                  position: d.globalPosition,
                  onChanged: () => _ui?.model.refresh(),
                ),
                child: Text(_name,
                    key: const ValueKey('comp-graph-name'),
                    style: t.body.copyWith(color: t.textMuted),
                    overflow: TextOverflow.ellipsis),
              ),
            ),
            // The controls fold into one mark when the panel is narrow.
            graphToolbarFold(
              markKey: const ValueKey('comp-graph-toolbar-more'),
              [
                ...groupFoldItems(t),
                graphToolFold(t,
                    key: const ValueKey('comp-graph-auto-wire'),
                    tip: l10n.graphAutoWire,
                    icon: LumitIcons.autoWire,
                    on: _autoWire,
                    onPressed: () => setState(() => _autoWire = !_autoWire)),
                graphToolFold(t,
                    key: const ValueKey('comp-graph-heal'),
                    tip: l10n.graphHeal,
                    icon: LumitIcons.heal,
                    on: _heal,
                    onPressed: () => setState(() => _heal = !_heal)),
                graphToolFold(t,
                    key: const ValueKey('comp-graph-snap'),
                    tip: _snapToGrid ? l10n.tipSnapOn : l10n.tipSnapOff,
                    icon: LumitIcons.snap,
                    on: _snapToGrid,
                    gap: 10,
                    onPressed: () =>
                        setState(() => _snapToGrid = !_snapToGrid)),
                graphFrameAllFold(t,
                    key: const ValueKey('comp-graph-frame-all'),
                    onPressed: () =>
                        _frameByHand(() => _frameAll(_viewport, layout))),
              ],
            ),
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
    final cut = GraphCutFade.cuts(
        size, [for (final b in layout.boxes) b.rect], _pan, _zoom);
    if (layout.boxes.isNotEmpty &&
        (_frameNext || (!_viewMoved && _framedFor != size))) {
      _frameNext = false;
      _framedFor = size;
      // The boxes open in the middle of the panel, never larger than they
      // are. After the frame, as a view change is.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _frameAll(size, layout, maxZoom: 1);
      });
    }
    return Focus(
      focusNode: _canvasFocus,
      onKeyEvent: (node, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        // Only the canvas itself: a rename field is typing these.
        if (node.hasPrimaryFocus &&
            (event.logicalKey == LogicalKeyboardKey.delete ||
                event.logicalKey == LogicalKeyboardKey.backspace)) {
          _deleteSelected();
          return KeyEventResult.handled;
        }
        if (node.hasPrimaryFocus && graphMuteKey(event)) {
          _mutePicked();
          return KeyEventResult.handled;
        }
        if (node.hasPrimaryFocus &&
            groupKey(event) == KeyEventResult.handled) {
          return KeyEventResult.handled;
        }
        // Enter renames the one picked box, as it renames a picked effect.
        if (node.hasPrimaryFocus &&
            event.logicalKey == LogicalKeyboardKey.enter &&
            _selection.length == 1) {
          final key = _selection.keys.single;
          if (_byKey[key] case final n?
              when n.kind == BridgeCompNodeKind.fx ||
                  n.kind == BridgeCompNodeKind.read) {
            setState(() => _renaming = key);
            return KeyEventResult.handled;
          }
        }
        // F, Shift+F and Ctrl+D, as the keymap has them for the node canvases.
        if (node.hasPrimaryFocus) {
          switch (_ui?.keymap.actionFor(BridgeKeyContext.nodes, event)) {
            case 'nodes.frame.selected':
              _frameByHand(() => _frameSelected(_viewport, layout));
              return KeyEventResult.handled;
            case 'nodes.frame.all':
              _frameByHand(() => _frameAll(_viewport, layout));
              return KeyEventResult.handled;
            case 'effect.duplicate':
              _duplicate(_ui!);
              return KeyEventResult.handled;
          }
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
      child: Stack(
        fit: StackFit.passthrough,
        children: [
      DragTarget<Object>(
        onWillAcceptWithDetails: (details) =>
            details.data is FootageDragData ||
            details.data is CompDragData ||
            details.data is EffectDragData,
        onAcceptWithDetails: (details) => _dropped(details.data, details.offset),
        builder: (context, candidate, _) => Listener(
          onPointerDown: (e) => _down(e, layout),
          onPointerMove: (e) => _move(e, layout),
          onPointerUp: (e) => _up(e, layout),
          onPointerHover: (e) {
            final at = _toCanvas(e.localPosition);
            tieHover(at, _wiresOf(layout));
            final edge = layout.socketAt(at) == null &&
                graphEdgeAt(layout, at, _zoom) != null;
            if (edge != _edgeHover) setState(() => _edgeHover = edge);
          },
          onPointerSignal: _wheel,
          behavior: HitTestBehavior.opaque,
          child: tieCursorRegion(GraphCutFade(
            left: cut.left,
            right: cut.right,
            top: cut.top,
            bottom: cut.bottom,
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
                          for (final (wire, edge, type) in _tieWires(layout))
                            (
                              wire,
                              edge == _dropWire
                                  ? t.animated
                                  : portColour(t, type),
                              // Heavier on a picked box, so what the pick
                              // is joined to reads at a glance.
                              edge == _dropWire ||
                                      _selection.containsKey(_fromKey(edge)) ||
                                      _selection.containsKey(_toKey(edge))
                                  ? 2.0
                                  : 1.0
                            ),
                        ],
                        ties: tieView(),
                        theme: t,
                        flight: _flight == null
                            ? null
                            : _flight!.from.isInput
                                ? (_flight!.end, _flight!.from.at)
                                : (_flight!.from.at, _flight!.end),
                        dragged: t.textPrimary,
                        pan: _pan,
                        zoom: _zoom,
                        wireStyle: ThemeScope.of(context).wireStyle,
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
                      child: GraphBoxStack(
                        children: [
                          ..._repeatFrames(t),
                          ...groupWashes(t, () => _claimed = true),
                          ...graphNotes(t, () => _claimed = true),
                          for (final box in layout.boxes)
                            Positioned(
                              left: box.rect.left,
                              top: box.rect.top,
                              width: box.rect.width,
                              height: box.rect.height,
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
                                foot: _shaderThumb(box.key),
                                snap: _flight?.snap?.node == box.key
                                    ? _flight!.snap
                                    : null,
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
          ))),
        ),
      ),
      handPanLayer(_ui!.tools, t, (d) => setState(() => _pan += d)),
      graphEdgeCursor(_edgeHover || _edgeDrag != null),
        ],
      ),
    );
  }
}

/// Every wire on this canvas: the stored ones, coloured by the type they
/// carry, and the dashed one in hand.
class _CompWirePainter extends CustomPainter {
  final List<(TieWire, Color, double)> wires;
  final TieView ties;
  final LumitTheme theme;
  final (Offset, Offset)? flight;
  final Color dragged;
  final Offset pan;
  final double zoom;
  final WireStyle wireStyle;

  const _CompWirePainter({
    required this.wires,
    required this.ties,
    required this.theme,
    required this.flight,
    required this.dragged,
    required this.pan,
    required this.zoom,
    required this.wireStyle,
  });

  Offset _screen(Offset canvas) => canvas * zoom + pan;

  @override
  void paint(Canvas canvas, Size size) {
    paintTiedWires(canvas, wires, ties,
        pan: pan, zoom: zoom, style: wireStyle, theme: theme);
    if (flight case final f?) {
      _wire(canvas, ties.via ?? f.$1, f.$2, dragged, dashes: true);
    }
  }

  void _wire(Canvas canvas, Offset from, Offset to, Color colour,
      {required bool dashes, double weight = 1}) {
    final path = graphWirePath(_screen(from), _screen(to),
        zoom: zoom, wires: wireStyle);
    final paint = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = graphWireWidth * zoom * weight;
    canvas.drawPath(dashes ? graphDashPath(path) : path, paint);
  }

  @override
  bool shouldRepaint(_CompWirePainter old) => true;
}
