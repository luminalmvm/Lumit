// The Graph panel — a layer's effect stack drawn as nodes and wires, plus the
// drivers wired into its parameters (K-471, K-472, K-473).
//
// **In plain terms.** A layer's effects are a list: the picture goes in at the
// top, each effect changes it, the result comes out at the bottom. This panel
// draws that same list as boxes joined left to right, and adds a second kind of
// box — a *driver*, which makes no picture but a value (a wobbling number, the
// loudness of the music, a turning colour). A wire from a driver into a
// parameter's socket makes that parameter follow the value instead of its own
// keyframes.
//
// **The one rule** (docs/impl/node-graph.md §1.1). The effect list is still the
// only authority for the picture. The Source box, one box per effect in stack
// order and the Layer out box are *derived* from it every time the engine is
// asked, so the graph has no second opinion to disagree with — and every wire
// gesture on the image chain lowers to the ordinary effect-stack commit. What
// this panel edits is the additive half: driver boxes, wires, canvas positions
// and which boxes wear the `E` badge, all committed by one `setGraph` per
// gesture and therefore one undo step apiece.
//
// **One read, never in a rebuild** (K-183). `getGraph` is asked when the
// selection or the document changes and held here; `bridge_call_budget_test`
// expects a hover over this canvas to cost nothing at all. Everything the
// canvas draws and every hit test it makes is arithmetic over that held copy.
//
// **Colour is the legend** (K-472). A wire and its sockets take the port's type
// colour from `theme.port`; no other colour coding appears on the canvas, and
// no colour crosses the bridge — the engine sends the type, the frontend maps
// it to the token.

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart' show KeyDownEvent, LogicalKeyboardKey;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart' show LumitIcon;
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'placeholder.dart';

// --- The drawing's own numbers (NodeGraph, Nodes-workspace) ---------------
//
// Every one of these is measured off the approved drawing's computed styles,
// and `graph_metrics_test.dart` holds the panel to them.

/// The panel's own toolbar: the layer's name, the two toggles, frame-all and
/// the zoom readout. The dock draws the tab strip above it.
const double graphToolbarHeight = 22;

/// A node card's content width — what the ports lay out in. The card is two
/// pixels wider, one for the border on each side.
const double graphNodeWidth = 150;

/// The Layer out box is narrower: it has a short name and two sockets, and
/// nothing that wants 150 px of room.
const double graphOutNodeWidth = 120;

/// The header strip, counting the hairline beneath it.
const double graphNodeHeaderHeight = 21;

/// One port row — an input on the left, an output on the right, or both.
const double graphPortRowHeight = 18;

/// A socket, border included. Filled is wired, hollow is not.
const double graphSocketSize = 9;

/// The `E` and `B` badges in a node's header, border included.
const double graphBadgeSize = 14;

/// The dot grid's pitch on the canvas ground.
const double graphDotGrid = 20;

/// Every wire, and the dashed one in flight.
const double graphWireWidth = 1.5;

/// The glyphs in the toolbar and the search popover. A size down from the
/// row glyphs' 16 (K-456: the manifest's number, not a preference).
const double graphIconSize = 13;

/// The Tab search popover: the width of its content (the box is two wider,
/// one for the hairline each side), and the three bands down it.
const double graphSearchWidth = 220;
const double graphSearchHeadHeight = 25;
const double graphSearchRowHeight = 20;
const double graphSearchFootHeight = 19;

/// Where an unplaced box lands: the chain marches right, the drivers sit
/// below it. The drawing's own spacing — a node's width plus 88 of air.
const double _autoX = 26;
const double _autoY = 44;
const double _autoStepX = graphNodeWidth + 88;
const double _autoDriverY = 222;
const double _autoDriverStepY = 140;

/// How near a socket the pointer has to be to take hold of it.
const double _socketGrab = 7;

/// The pointer travel that turns a press into a drag rather than a click.
const double _dragSlop = 3;

/// Which theme token a port type draws in (K-472 §6.1): seven types, five
/// colours, grouped as the drawing's legend groups them.
Color portColour(LumitTheme t, BridgePortType type) => switch (type) {
      BridgePortType.image || BridgePortType.matte => t.port.image,
      BridgePortType.number => t.port.number,
      BridgePortType.colour => t.port.colour,
      BridgePortType.shape || BridgePortType.points => t.port.geometry,
      BridgePortType.audio => t.port.audio,
    };

/// A stable string for a node reference, so positions and lookups can live in
/// a plain map. `BridgeNodeRef` is a freezed union without a usable key.
String graphNodeKey(BridgeNodeRef node) => switch (node) {
      BridgeNodeRef_Source() => 'source',
      BridgeNodeRef_Out() => 'out',
      BridgeNodeRef_Effect(:final field0) => 'effect:$field0',
      BridgeNodeRef_Driver(:final field0) => 'driver:$field0',
    };

/// The effect this box stands for, or null for the other three kinds.
UuidValue? _effectIdOf(BridgeNodeRef node) =>
    node is BridgeNodeRef_Effect ? node.field0 : null;

UuidValue? _driverIdOf(BridgeNodeRef node) =>
    node is BridgeNodeRef_Driver ? node.field0 : null;

/// A port type that the image chain owns. The chain is wired by construction —
/// the picture always flows straight down the stack — so an image socket takes
/// no drag: reordering is the stack view's gesture, and this panel is a second
/// view of it, not a second opinion about it.
bool _isChainType(BridgePortType type) => type == BridgePortType.image;

/// One socket as the canvas draws it: which box, which port, which side.
class _Socket {
  final BridgeNodeRef node;
  final BridgePort port;
  final bool isInput;
  final Offset at;
  const _Socket(this.node, this.port, this.isInput, this.at);
}

/// One box, laid out: where it sits and which sockets it shows.
class _Box {
  final BridgeGraphNode node;
  final Rect rect;
  final List<BridgePort> inputs;
  final List<BridgePort> outputs;
  const _Box(this.node, this.rect, this.inputs, this.outputs);

  Offset? socket(String portId, bool isInput) {
    final list = isInput ? inputs : outputs;
    final i = list.indexWhere((p) => p.id == portId);
    if (i < 0) return null;
    return Offset(
      isInput ? rect.left : rect.right,
      rect.top +
          1 +
          graphNodeHeaderHeight +
          i * graphPortRowHeight +
          graphPortRowHeight / 2,
    );
  }
}

/// The whole canvas worked out from the held graph: every box's rectangle and
/// every socket's centre, in canvas units. Rebuilt from arithmetic each build —
/// no bridge call, no allocation the panel keeps.
class _Layout {
  final List<_Box> boxes;
  final Map<String, _Box> byKey;

  _Layout(this.boxes)
      : byKey = {for (final b in boxes) graphNodeKey(b.node.node): b};

  static _Layout of(
    BridgeLayerGraph graph,
    Map<String, Offset> positions,
    Set<String> exposed,
  ) {
    final boxes = <_Box>[];
    var chain = 0;
    var drivers = 0;
    for (final node in graph.nodes) {
      final key = graphNodeKey(node.node);
      final isDriver = node.node is BridgeNodeRef_Driver;
      final placed = positions[key] ??
          (isDriver
              ? Offset(_autoX + drivers * _autoStepX,
                  _autoDriverY + drivers * _autoDriverStepY)
              : Offset(_autoX + chain * _autoStepX, _autoY));
      isDriver ? drivers++ : chain++;

      // A **driver** draws every socket it has: the box is small, and its
      // ports are the whole of what it is for — both drawings draw them so.
      // An **effect** draws the picture's own sockets always, and a parameter
      // socket when a wire is on it or when the box wears its `E` badge
      // (§1.4). Exposure grows the box; it is not a second kind of wiring.
      final open = exposed.contains(key) || isDriver;
      List<BridgePort> shown(List<BridgePort> ports) => [
            for (final p in ports)
              if (_alwaysDrawn(p.portType) || p.wired || open) p,
          ];
      final inputs = shown(node.inputs);
      final outputs = shown(node.outputs);
      final width =
          node.node is BridgeNodeRef_Out ? graphOutNodeWidth : graphNodeWidth;
      final rows = math.max(inputs.length, outputs.length);
      boxes.add(_Box(
        node,
        Rect.fromLTWH(
          placed.dx,
          placed.dy,
          width + 2,
          2 + graphNodeHeaderHeight + rows * graphPortRowHeight,
        ),
        inputs,
        outputs,
      ));
    }
    return _Layout(boxes);
  }

  /// The socket nearest [at], within grabbing distance, or null.
  _Socket? socketAt(Offset at) {
    for (final box in boxes) {
      for (final (isInput, ports) in [
        (true, box.inputs),
        (false, box.outputs)
      ]) {
        for (final port in ports) {
          final centre = box.socket(port.id, isInput);
          if (centre != null && (centre - at).distance <= _socketGrab) {
            return _Socket(box.node.node, port, isInput, centre);
          }
        }
      }
    }
    return null;
  }

  _Box? boxAt(Offset at) {
    for (final box in boxes.reversed) {
      if (box.rect.contains(at)) return box;
    }
    return null;
  }

  /// The image chain, in stack order: Source, each effect, Layer out. Its
  /// wires are not stored anywhere — they *are* the list.
  List<_Box> get chain => [
        for (final b in boxes)
          if (b.node.node is! BridgeNodeRef_Driver) b,
      ];
}

/// Sockets the picture's own path owns, always drawn whether wired or not.
bool _alwaysDrawn(BridgePortType type) =>
    type == BridgePortType.image ||
    type == BridgePortType.matte ||
    type == BridgePortType.audio;

/// A wire being dragged: where it left, and where the pointer is now.
class _InFlight {
  final _Socket from;
  Offset to;
  _InFlight(this.from, this.to);
}

/// A box being dragged: which one, and where it started.
class _NodeDrag {
  final String key;
  final Offset grab;
  final Offset origin;
  _NodeDrag(this.key, this.grab, this.origin);
}

class GraphPanelFrb extends StatefulWidget {
  /// The driver catalogue seam, injected by tests so the popover can be
  /// asserted without the real registry.
  final List<BridgeEffectInfo> Function()? driversLister;

  const GraphPanelFrb({super.key, this.driversLister});

  @override
  State<GraphPanelFrb> createState() => _GraphPanelFrbState();
}

class _GraphPanelFrbState extends State<GraphPanelFrb> {
  LumitUiState? _ui;
  LayerReference? _layer;

  /// The held graph. Read on selection and on document change — never in a
  /// build, which is what the budget test is guarding.
  BridgeLayerGraph? _graph;

  /// The layer's name, taken at the same moment, so the Source box can be
  /// headed without asking the model during a paint.
  String _layerName = '';

  /// Canvas positions, staged: a drag moves this map and the release commits
  /// it, the K-344 pattern. Seeded from the document on each read.
  Map<String, Offset> _positions = {};

  /// The picked box. Mirrored into [LumitUiState.graphNode] on every write, so
  /// the Node panel — which is a different pane of the dock, and may not even
  /// be in the arrangement — follows the pick without this panel having to
  /// know it is there. A getter pair rather than a `_select` method because
  /// every site that already assigns the field then keeps working unchanged.
  BridgeNodeRef? _selectedNode;

  BridgeNodeRef? get _selected => _selectedNode;

  set _selected(BridgeNodeRef? node) {
    _selectedNode = node;
    _ui?.graphNode.value = node;
  }

  Offset _pan = Offset.zero;
  double _zoom = 1;

  /// Adding a node wires it up in the same commit; deleting one takes its
  /// wires with it. Both on, both the drawing's state, and both `animated`
  /// while on because that is what a pill switch is everywhere (K-465).
  bool _autoWire = true;
  bool _heal = true;

  _InFlight? _flight;
  _NodeDrag? _nodeDrag;
  Offset? _panFrom;
  Offset? _pressAt;

  /// The Tab search: where it opened, and the wire (if any) that opened it.
  Offset? _searchAt;
  _Socket? _searchWire;
  final TextEditingController _search = TextEditingController();
  final FocusNode _searchFocus = FocusNode();

  final FocusNode _canvasFocus = FocusNode(debugLabel: 'graph canvas');

  @override
  void initState() {
    super.initState();
    _search.addListener(() => setState(() {}));
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (identical(ui, _ui)) return;
    _unbind();
    _ui = ui;
    ui.selectedLayer.addListener(_reload);
    ui.model.addListener(_reload);
    _reload();
  }

  void _unbind() {
    _ui?.selectedLayer.removeListener(_reload);
    _ui?.model.removeListener(_reload);
  }

  @override
  void dispose() {
    _unbind();
    _search.dispose();
    _searchFocus.dispose();
    _canvasFocus.dispose();
    super.dispose();
  }

  /// The one read. Everything the canvas draws comes from here.
  void _reload() {
    if (!mounted) return;
    final layer = _ui?.selectedLayer.value;
    BridgeLayerGraph? graph;
    var name = '';
    if (layer != null) {
      try {
        graph = layer.getGraph();
        name = layer.getInfo().name;
      } catch (_) {
        // The layer has gone since the selection was made; the placeholder is
        // the honest answer until the selection catches up.
        graph = null;
      }
    }
    setState(() {
      _layer = layer;
      _graph = graph;
      _layerName = name;
      _positions = {
        for (final p in graph?.wiring.layout ?? const <BridgeNodePosition>[])
          graphNodeKey(p.node): Offset(p.x, p.y),
      };
      final present =
          (graph?.nodes ?? const []).map((n) => graphNodeKey(n.node)).toSet();
      if (_selected != null && !present.contains(graphNodeKey(_selected!))) {
        _selected = null;
      }
    });
  }

  // --- Committing ---------------------------------------------------------

  /// The wiring as it stands, with the staged positions folded in. Every write
  /// goes through here, so a gesture that moves a box and one that draws a wire
  /// commit the same shape.
  BridgeGraphWiring _wiringNow({
    List<BridgeGraphEdge>? edges,
    List<BridgeNodeRef>? exposed,
  }) {
    final w = _graph!.wiring;
    return BridgeGraphWiring(
      edges: edges ?? w.edges,
      layout: [
        for (final e in _positions.entries)
          if (_refOf(e.key) case final ref?)
            BridgeNodePosition(node: ref, x: e.value.dx, y: e.value.dy),
      ],
      exposed: exposed ?? w.exposed,
    );
  }

  /// The node reference a position key stands for, or null when the box it
  /// named has gone — a position for a deleted node is dropped rather than
  /// carried, since the engine refuses a layout entry it cannot place.
  BridgeNodeRef? _refOf(String key) {
    for (final node in _graph!.nodes) {
      if (graphNodeKey(node.node) == key) return node.node;
    }
    return null;
  }

  /// One gesture, one `setGraph`, one undo step. A refusal leaves the document
  /// exactly as it was — the panel declines what it can decline itself (a type
  /// mismatch), and the engine's refusal is the backstop behind that.
  void _commit(BridgeGraphWiring wiring,
      {List<BridgeEffectInstance>? drivers}) {
    final layer = _layer;
    if (layer == null) return;
    try {
      layer.setGraph(
        drivers: drivers ?? layer.getGraphDrivers(),
        wiring: wiring,
      );
    } catch (_) {
      // Refused, or the layer moved under us. Either way the document is
      // untouched and re-reading is the recovery.
    }
    _ui?.model.refresh();
    _reload();
  }

  /// Draw or re-route a wire. A wire may be pulled from either end, so the two
  /// sockets are sorted here rather than at each call site. An occupied input
  /// is re-routed rather than doubled (§1.1); a type mismatch never gets this
  /// far, having been declined without a bridge call.
  void _connect(_Socket a, _Socket b) {
    final from = a.isInput ? b : a;
    final to = a.isInput ? a : b;
    final source = _outputRef(from);
    final dest = _inputRef(to);
    if (source == null || dest == null) return;
    _commit(_wiringNow(edges: [
      for (final e in _graph!.wiring.edges)
        if (e.to != dest) e,
      BridgeGraphEdge(from: source, to: dest),
    ]));
  }

  void _disconnect(_Socket socket) {
    final dest = _inputRef(socket);
    if (dest == null) return;
    _commit(_wiringNow(edges: [
      for (final e in _graph!.wiring.edges)
        if (e.to != dest) e,
    ]));
  }

  BridgeOutputRef? _outputRef(_Socket socket) {
    if (socket.isInput) return null;
    if (socket.node is BridgeNodeRef_Source) {
      // The layer's own masked source alpha at that point in the chain — the
      // one feed the graph adds that the Matte row could not offer (§1.4).
      return socket.port.portType == BridgePortType.matte
          ? const BridgeOutputRef.sourceMatte()
          : null;
    }
    final driver = _driverIdOf(socket.node);
    if (driver == null) return null;
    return BridgeOutputRef.driver(node: driver, port: socket.port.id);
  }

  BridgeInputRef? _inputRef(_Socket socket) {
    if (!socket.isInput) return null;
    if (socket.port.portType == BridgePortType.matte) {
      final effect = _effectIdOf(socket.node);
      if (effect != null) return BridgeInputRef.matte(effect: effect);
      return null;
    }
    return BridgeInputRef.param(node: socket.node, port: socket.port.id);
  }

  /// Whether these two sockets may be joined, decided **here** from the two
  /// port types both sides carry in the read model. A mismatched drop is
  /// declined without a bridge call: the engine's refusal is the backstop, not
  /// the message channel (docs/17, "The layer graph").
  bool _accepts(_Socket from, _Socket to) {
    if (from.isInput == to.isInput) return false;
    final out = from.isInput ? to : from;
    final into = from.isInput ? from : to;
    if (out.node == into.node) return false;
    if (_isChainType(out.port.portType) || _isChainType(into.port.portType)) {
      return false;
    }
    // The Layer out box's Audio socket is drawn, unfilled and honest: audio
    // comes only from a footage layer's own stream in this phase (K-435).
    if (into.node is BridgeNodeRef_Out) return false;
    return out.port.portType == into.port.portType;
  }

  // --- Node gestures ------------------------------------------------------

  void _toggleExposed(BridgeNodeRef node) {
    final key = graphNodeKey(node);
    final was = _graph!.wiring.exposed.any((e) => graphNodeKey(e) == key);
    _commit(_wiringNow(exposed: [
      for (final e in _graph!.wiring.exposed)
        if (graphNodeKey(e) != key) e,
      if (!was) node,
    ]));
  }

  /// Bypass. A driver rides the staged-instance path with `setGraph` as its
  /// commit; an effect rides the stack's own call, exactly as its card in
  /// Effect controls does. One op either way.
  void _toggleBypass(BridgeGraphNode node) {
    final layer = _layer;
    if (layer == null) return;
    final driver = _driverIdOf(node.node);
    try {
      if (driver != null) {
        final staged = layer.getGraphDrivers();
        for (final instance in staged) {
          if (instance.id() == driver) {
            instance.setEnabled(enabled: !node.enabled);
            layer.setGraph(drivers: staged, wiring: _wiringNow());
            break;
          }
        }
      } else if (_effectIdOf(node.node) case final effect?) {
        for (final instance in layer.getEffects()) {
          if (instance.id() == effect) {
            layer.setEffectEnabled(effect: instance, enabled: !node.enabled);
            break;
          }
        }
      }
    } catch (_) {
      // The stack or the graph moved under us; re-reading is the recovery.
    }
    _ui?.model.refresh();
    _reload();
  }

  /// Delete the selected box.
  ///
  /// **Heal** decides what happens to the wires on it. On, they go with it in
  /// the same commit; off, a box that still carries a wire is left alone —
  /// unplug it first, and the document never passes through a state where a
  /// wire names a box that is not there.
  ///
  /// The image chain heals by construction either way: taking an effect out of
  /// the list joins its neighbours, because the list *is* the chain.
  void _deleteSelected() {
    final node = _selected;
    final layer = _layer;
    if (node == null || layer == null || _graph == null) return;
    if (node is BridgeNodeRef_Source || node is BridgeNodeRef_Out) return;

    final key = graphNodeKey(node);
    final kept = [
      for (final e in _graph!.wiring.edges)
        if (!_touches(e, node)) e,
    ];
    if (!_heal && kept.length != _graph!.wiring.edges.length) return;

    final driver = _driverIdOf(node);
    if (driver != null) {
      final staged = [
        for (final d in layer.getGraphDrivers())
          if (d.id() != driver) d,
      ];
      _positions.remove(key);
      _selected = null;
      _commit(_wiringNow(edges: kept), drivers: staged);
      return;
    }

    // An effect's removal is the stack's own op, and one op is all it takes:
    // `Op::SetLayerEffects` prunes the edges, positions and badges naming the
    // box that went, inside the same commit, so this is one write and one undo
    // step exactly as a driver's removal is.
    final effect = _effectIdOf(node);
    if (effect == null) return;
    try {
      for (final instance in layer.getEffects()) {
        if (instance.id() == effect) {
          layer.removeEffect(effect: instance);
          break;
        }
      }
    } catch (_) {
      // Refused; the document is as it was.
    }
    _positions.remove(key);
    _selected = null;
    _ui?.model.refresh();
    _reload();
  }

  bool _touches(BridgeGraphEdge edge, BridgeNodeRef node) {
    final key = graphNodeKey(node);
    final from = switch (edge.from) {
      BridgeOutputRef_Driver(:final node) =>
        graphNodeKey(BridgeNodeRef.driver(node)),
      BridgeOutputRef_SourceMatte() => 'source',
    };
    final to = switch (edge.to) {
      BridgeInputRef_Param(:final node) => graphNodeKey(node),
      BridgeInputRef_Matte(:final effect) =>
        graphNodeKey(BridgeNodeRef.effect(effect)),
    };
    return from == key || to == key;
  }

  // --- The Tab search -----------------------------------------------------

  void _openSearch(Offset at, {_Socket? wire}) {
    setState(() {
      _searchAt = at;
      _searchWire = wire;
      _search.text = '';
    });
    _searchFocus.requestFocus();
  }

  void _closeSearch() => setState(() {
        _searchAt = null;
        _searchWire = null;
      });

  /// Drop a driver where the search opened, and — with Auto-wire on and a wire
  /// in hand — join it to whichever of the new box's sockets fits.
  ///
  /// `newDriver` deliberately does not commit, because dropping a node is
  /// rarely the whole gesture (docs/17, "The layer graph").
  ///
  /// **The wire rides in the same commit as the box** (docs/impl/node-graph.md
  /// §3), which is what makes "drag a wire out, pick a driver" one undo step
  /// rather than two. It can, because a catalogue entry carries the ports it
  /// declares — the socket is known before the node is in the document.
  void _addDriver(BridgeEffectInfo info) {
    final layer = _layer;
    final at = _searchAt;
    if (layer == null || at == null || _graph == null) return;
    final wire = _searchWire;
    _closeSearch();

    final BridgeEffectInstance made;
    try {
      made = layer.newDriver(name: info.name);
    } catch (_) {
      return;
    }
    final ref = BridgeNodeRef.driver(made.id());
    _positions[graphNodeKey(ref)] = at;

    final edges = [..._graph!.wiring.edges];
    final joined =
        _autoWire && wire != null ? _autoWireEdge(ref, info, wire) : null;
    if (joined != null) {
      // An occupied input is re-routed rather than doubled (§1.1), the same
      // rule `_connect` applies to a wire drawn by hand.
      edges.removeWhere((e) => e.to == joined.to);
      edges.add(joined);
    }
    try {
      layer.setGraph(
        drivers: [...layer.getGraphDrivers(), made],
        wiring: BridgeGraphWiring(
          edges: edges,
          layout: [
            ..._wiringNow().layout,
            BridgeNodePosition(node: ref, x: at.dx, y: at.dy),
          ],
          exposed: _graph!.wiring.exposed,
        ),
      );
    } catch (_) {
      return;
    }
    _ui?.model.refresh();
    _reload();
    setState(() => _selected = ref);
  }

  /// The wire joining the box about to be added to the wire in hand, or null
  /// when none of the sockets it declares fits.
  ///
  /// Every socket the entry *declares* is considered, not only the ones the box
  /// would draw: an unexposed box shows nothing but the picture's own path, and
  /// auto-wire is about what the node can take, not about what is on screen.
  /// The ports come from the catalogue entry rather than from the read model,
  /// which is what lets this be worked out before the node is committed.
  BridgeGraphEdge? _autoWireEdge(
    BridgeNodeRef added,
    BridgeEffectInfo info,
    _Socket wire,
  ) {
    // A wire let go of an output looks for an input on the new box, and the
    // other way about.
    for (final port in wire.isInput ? info.outputs : info.inputs) {
      final socket = _Socket(added, port, !wire.isInput, Offset.zero);
      if (!_accepts(wire, socket)) continue;
      final source = _outputRef(wire.isInput ? socket : wire);
      final dest = _inputRef(wire.isInput ? wire : socket);
      if (source == null || dest == null) continue;
      return BridgeGraphEdge(from: source, to: dest);
    }
    return null;
  }

  /// Whether any socket this catalogue entry declares could take the wire in
  /// hand — the same type rule [_accepts] applies on the canvas, asked of a box
  /// that does not exist yet. It is what the Tab search filters by, so picking
  /// an entry from the list can never fail to connect.
  bool _fitsWire(BridgeEffectInfo info, _Socket wire) =>
      (wire.isInput ? info.outputs : info.inputs).any((port) =>
          !_isChainType(port.portType) && port.portType == wire.port.portType);

  // --- Pointer work -------------------------------------------------------

  Offset _toCanvas(Offset local) => (local - _pan) / _zoom;

  void _down(PointerDownEvent event, _Layout layout) {
    _canvasFocus.requestFocus();
    if (_searchAt != null) {
      _closeSearch();
      return;
    }
    final at = _toCanvas(event.localPosition);
    _pressAt = event.localPosition;

    final socket = layout.socketAt(at);
    if (socket != null && !_isChainType(socket.port.portType)) {
      setState(() => _flight = _InFlight(socket, at));
      return;
    }

    final box = layout.boxAt(at);
    if (box != null) {
      final key = graphNodeKey(box.node.node);
      setState(() {
        _selected = box.node.node;
        _nodeDrag = _NodeDrag(key, at, _positions[key] ?? box.rect.topLeft);
      });
      // The graph and the stack share one selection (K-300), so picking a box
      // fronts the same effect in Effect controls and the Timeline.
      if (_effectIdOf(box.node.node) case final effect?) {
        if (_layer case final layer?) {
          _ui?.setEffectSelection(layer, [effect]);
        }
      }
      return;
    }

    setState(() {
      _selected = null;
      _panFrom = _pan - event.localPosition;
    });
  }

  void _move(PointerMoveEvent event, _Layout layout) {
    final at = _toCanvas(event.localPosition);
    if (_flight case final flight?) {
      setState(() => flight.to = at);
      return;
    }
    if (_nodeDrag case final drag?) {
      setState(() => _positions[drag.key] = drag.origin + (at - drag.grab));
      return;
    }
    if (_panFrom case final from?) {
      setState(() => _pan = from + event.localPosition);
    }
  }

  void _up(PointerUpEvent event, _Layout layout) {
    final at = _toCanvas(event.localPosition);
    final moved = _pressAt == null ||
        (event.localPosition - _pressAt!).distance > _dragSlop;

    if (_flight case final flight?) {
      setState(() => _flight = null);
      final landed = layout.socketAt(at);
      if (!moved && flight.from.isInput && flight.from.port.wired) {
        // A press and release on a wired input takes its wire off.
        _disconnect(flight.from);
        return;
      }
      if (landed == null) {
        // Onto empty canvas: the search opens with the wire still in hand, and
        // auto-wire joins it to the node that is picked.
        if (moved) _openSearch(at, wire: flight.from);
        return;
      }
      if (_accepts(flight.from, landed)) {
        _connect(flight.from, landed);
      }
      // A mismatched drop is simply declined: nothing crosses the bridge.
      return;
    }

    if (_nodeDrag != null) {
      setState(() => _nodeDrag = null);
      if (moved && _graph != null) _commit(_wiringNow());
      return;
    }
    setState(() => _panFrom = null);
  }

  /// Fit every box on screen, with a little air round the edges.
  void _frameAll(Size viewport, _Layout layout) {
    if (layout.boxes.isEmpty) return;
    var bounds = layout.boxes.first.rect;
    for (final box in layout.boxes) {
      bounds = bounds.expandToInclude(box.rect);
    }
    bounds = bounds.inflate(20);
    final zoom = math
        .min(
          viewport.width / bounds.width,
          viewport.height / bounds.height,
        )
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
    // Zoom about the pointer, so what is under it stays under it.
    final anchor = (event.localPosition - _pan) / was;
    setState(() {
      _zoom = next;
      _pan = event.localPosition - anchor * next;
    });
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
        hint: l10n.graphNoLayer,
      );
    }

    final layout = _Layout.of(
      graph,
      _positions,
      graph.wiring.exposed.map(graphNodeKey).toSet(),
    );

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

  Widget _toolbar(LumitTheme t, _Layout layout) => Container(
        key: const ValueKey('graph-toolbar'),
        height: graphToolbarHeight,
        color: t.surface1,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        child: Row(
          children: [
            Expanded(
              child: Text(_layerName,
                  key: const ValueKey('graph-layer-name'),
                  style: t.body.copyWith(color: t.textMuted),
                  overflow: TextOverflow.ellipsis),
            ),
            Text(l10n.graphAutoWire, style: t.kicker),
            const SizedBox(width: 10),
            HouseToggle(
              key: const ValueKey('graph-auto-wire'),
              value: _autoWire,
              onChanged: (on) => setState(() => _autoWire = on),
            ),
            const SizedBox(width: 10),
            Text(l10n.graphHeal, style: t.kicker),
            const SizedBox(width: 10),
            HouseToggle(
              key: const ValueKey('graph-heal'),
              value: _heal,
              onChanged: (on) => setState(() => _heal = on),
            ),
            const SizedBox(width: 10),
            LumitTooltip(
              message: l10n.graphFrameAll,
              child: GestureDetector(
                key: const ValueKey('graph-frame-all'),
                behavior: HitTestBehavior.opaque,
                onTap: () => _frameAll(_viewport, layout),
                child: glyph.LumitIcon(LumitIcons.frameAll,
                    size: graphIconSize, colour: t.textMuted),
              ),
            ),
            const SizedBox(width: 10),
            Text(
              l10n.graphZoom((_zoom * 100).round()),
              key: const ValueKey('graph-zoom'),
              style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
            ),
          ],
        ),
      );

  Size _viewport = Size.zero;

  Widget _canvas(LumitTheme t, _Layout layout, Size size) {
    _viewport = size;
    return Focus(
      focusNode: _canvasFocus,
      onKeyEvent: (node, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        if (event.logicalKey == LogicalKeyboardKey.tab && _searchAt == null) {
          _openSearch(_toCanvas(Offset(size.width / 2, size.height / 2)));
          return KeyEventResult.handled;
        }
        if (event.logicalKey == LogicalKeyboardKey.delete ||
            event.logicalKey == LogicalKeyboardKey.backspace) {
          _deleteSelected();
          return KeyEventResult.handled;
        }
        return KeyEventResult.ignored;
      },
      // The popover is a sibling of the canvas rather than a child of it: the
      // canvas takes every pointer that lands on it — that is how a socket is
      // grabbed without a gesture detector per socket — and a press inside the
      // search would otherwise be read as a press on the ground behind it.
      child: Stack(
        children: [
          Positioned.fill(
            child: Listener(
              onPointerDown: (e) => _down(e, layout),
              onPointerMove: (e) => _move(e, layout),
              onPointerUp: (e) => _up(e, layout),
              onPointerSignal: _wheel,
              behavior: HitTestBehavior.opaque,
              child: Container(
                key: const ValueKey('graph-canvas'),
                color: t.surface0,
                child: Stack(
                  clipBehavior: Clip.hardEdge,
                  children: [
                    Positioned.fill(
                      child: CustomPaint(
                        painter: _GraphPainter(
                          layout: layout,
                          edges: _graph!.wiring.edges,
                          flight: _flight,
                          pan: _pan,
                          zoom: _zoom,
                          grid: t.surface2,
                          ground: t.surface0,
                          theme: t,
                          dragged: t.textPrimary,
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
                            for (final box in layout.boxes)
                              Positioned(
                                left: box.rect.left,
                                top: box.rect.top,
                                child: _NodeCard(
                                  box: box,
                                  title: box.node.node is BridgeNodeRef_Source
                                      ? _layerName
                                      : engineLabel(box.node.label),
                                  selected: _selected != null &&
                                      graphNodeKey(_selected!) ==
                                          graphNodeKey(box.node.node),
                                  exposed: _graph!.wiring.exposed.any((e) =>
                                      graphNodeKey(e) ==
                                      graphNodeKey(box.node.node)),
                                  onExpose: () => _toggleExposed(box.node.node),
                                  onBypass: () => _toggleBypass(box.node),
                                ),
                              ),
                          ],
                        ),
                      ),
                    ),
                    _legend(t),
                  ],
                ),
              ),
            ),
          ),
          if (_searchAt != null) _searchPopover(t),
        ],
      ),
    );
  }

  /// The legend along the canvas's bottom edge: colour *is* the type, and this
  /// strip is what says so (K-445).
  Widget _legend(LumitTheme t) => Positioned(
        left: 10,
        bottom: 8,
        child: Row(
          key: const ValueKey('graph-legend'),
          children: [
            Text(l10n.graphTypes, style: t.kicker),
            for (final (colour, word) in [
              (t.port.image, l10n.graphTypeImage),
              (t.port.number, l10n.graphTypeNumber),
              (t.port.colour, l10n.graphTypeColour),
              (t.port.geometry, l10n.graphTypeGeometry),
              (t.port.audio, l10n.graphTypeAudio),
            ]) ...[
              const SizedBox(width: 12),
              Container(
                width: 7,
                height: 7,
                decoration:
                    BoxDecoration(color: colour, shape: BoxShape.circle),
              ),
              const SizedBox(width: 4),
              Text(word, style: t.kicker.copyWith(letterSpacing: 0.54)),
            ],
          ],
        ),
      );

  Widget _searchPopover(LumitTheme t) {
    final at = _searchAt!;
    final needle = _search.text.trim().toLowerCase();
    final all = (widget.driversLister ?? listDrivers)();
    // With a wire in hand the list is the entries that wire could actually land
    // on, which is what makes the footer's sentence true: pick one and it is
    // connected. Without one, everything.
    final wire = _searchWire;
    final shown = [
      for (final d in all)
        if ((needle.isEmpty ||
                engineLabel(d.label).toLowerCase().contains(needle)) &&
            (wire == null || _fitsWire(d, wire)))
          d,
    ];
    final screen = at * _zoom + _pan;
    return Positioned(
      left: screen.dx,
      top: screen.dy,
      // Not a `FloatSurface`: that one insets its child by 6 all round, and
      // this popover's bands run edge to edge — the head's hairline is the
      // full width of the box, which is how the drawing draws it. The float's
      // *shadow* and radius are shared, so it still reads as one of the
      // family rather than as a stray card.
      child: Container(
        // The content width, plus the hairline each side — the same reading
        // the node card takes.
        width: graphSearchWidth + 2,
        decoration: BoxDecoration(
          color: t.surface1,
          borderRadius: BorderRadius.circular(t.tokens.floatRadius),
          border: Border.all(color: t.hairline),
          boxShadow: t.floatShadow,
        ),
        child: Column(
          key: const ValueKey('graph-search'),
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              key: const ValueKey('graph-search-head'),
              height: graphSearchHeadHeight,
              padding: const EdgeInsets.symmetric(horizontal: 10),
              decoration: BoxDecoration(
                border: Border(bottom: BorderSide(color: t.hairline)),
              ),
              child: Row(
                children: [
                  glyph.LumitIcon(LumitIcons.search,
                      size: graphIconSize, colour: t.textMuted),
                  const SizedBox(width: 8),
                  Expanded(
                    child: HouseTextField(
                      key: const ValueKey('graph-search-field'),
                      controller: _search,
                      focusNode: _searchFocus,
                      width: 120,
                    ),
                  ),
                  const SizedBox(width: 8),
                  Text(l10n.graphSearchKey,
                      style: t.kicker.copyWith(letterSpacing: 0.54)),
                ],
              ),
            ),
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 4),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final driver in shown)
                    GestureDetector(
                      key: ValueKey<String>('graph-search-${driver.name}'),
                      behavior: HitTestBehavior.opaque,
                      onTap: () => _addDriver(driver),
                      child: Container(
                        height: graphSearchRowHeight,
                        padding: const EdgeInsets.symmetric(horizontal: 10),
                        child: Row(
                          children: [
                            Expanded(
                              child: Text(engineLabel(driver.label),
                                  style: t.body,
                                  overflow: TextOverflow.ellipsis),
                            ),
                            Text(engineLabel(driver.categoryLabel),
                                style: t.kicker),
                          ],
                        ),
                      ),
                    ),
                ],
              ),
            ),
            Container(
              key: const ValueKey('graph-search-foot'),
              height: graphSearchFootHeight,
              padding: const EdgeInsets.symmetric(horizontal: 10),
              alignment: Alignment.centerLeft,
              decoration: BoxDecoration(
                border: Border(top: BorderSide(color: t.hairline)),
              ),
              child: Text(
                wire == null ? l10n.graphSearchAdds : l10n.graphSearchWires,
                style: t.kicker.copyWith(letterSpacing: 0.54),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// One box on the canvas: a header strip with its name and badges, then the
/// port rows with their sockets sitting on the border.
class _NodeCard extends StatelessWidget {
  final _Box box;
  final String title;
  final bool selected;
  final bool exposed;
  final VoidCallback onExpose;
  final VoidCallback onBypass;

  const _NodeCard({
    required this.box,
    required this.title,
    required this.selected,
    required this.exposed,
    required this.onExpose,
    required this.onBypass,
  });

  bool get _derived =>
      box.node.node is BridgeNodeRef_Source ||
      box.node.node is BridgeNodeRef_Out;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final rows = math.max(box.inputs.length, box.outputs.length);
    return SizedBox(
      key: ValueKey<String>('graph-node-${graphNodeKey(box.node.node)}'),
      width: box.rect.width,
      height: box.rect.height,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Positioned.fill(
            child: _NodeFrame(
              // A bypassed box draws its border dashed, both drawings; the
              // selected one draws it in `animated` (K-473).
              colour: selected ? t.animated : t.hairline,
              dashed: !box.node.enabled,
              fill: t.surface1,
              radius: t.tokens.controlRadius,
            ),
          ),
          Positioned(
            left: 1,
            top: 1,
            width: box.rect.width - 2,
            child: _header(t),
          ),
          for (var i = 0; i < rows; i++)
            Positioned(
              left: 1,
              top: 1 + graphNodeHeaderHeight + i * graphPortRowHeight,
              width: box.rect.width - 2,
              height: graphPortRowHeight,
              child: _row(t, i),
            ),
        ],
      ),
    );
  }

  Widget _header(LumitTheme t) => Container(
        height: graphNodeHeaderHeight,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        decoration: BoxDecoration(
          color: t.surface2,
          border: Border(bottom: BorderSide(color: t.hairline)),
        ),
        child: Row(
          children: [
            Flexible(
              child: Text(
                title,
                style: box.node.enabled ? t.kickerOn : t.kicker,
                overflow: TextOverflow.ellipsis,
              ),
            ),
            // A box the user has named of their own says so beside its type
            // name, quieter and less tracked — the same reading the drawing
            // gives the Audio level box called "Music".
            if (box.node.customName case final own?) ...[
              const SizedBox(width: 6),
              Flexible(
                child: Text(own,
                    style: t.kicker.copyWith(letterSpacing: 0.54),
                    overflow: TextOverflow.ellipsis),
              ),
            ],
            const Spacer(),
            if (!_derived) ...[
              _badge(t, 'E', exposed, onExpose, t.textPrimary),
              const SizedBox(width: 4),
              _badge(t, 'B', !box.node.enabled, onBypass, t.error),
            ],
          ],
        ),
      );

  /// The `E` and `B` marks. `B` on is the one place the error family appears
  /// on this canvas (15-DESIGN §12A.7) — a bypassed box is not a fault, but it
  /// is the one box that is deliberately doing nothing.
  Widget _badge(
    LumitTheme t,
    String mark,
    bool on,
    VoidCallback onTap,
    Color onColour,
  ) =>
      GestureDetector(
        key: ValueKey<String>(
            'graph-badge-$mark-${graphNodeKey(box.node.node)}'),
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Container(
          width: graphBadgeSize,
          height: graphBadgeSize,
          alignment: Alignment.center,
          decoration: BoxDecoration(
            color: on ? t.surface4 : null,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(color: on ? onColour : t.hairlineStrong),
          ),
          child: Text(
            mark,
            style: t.mono.copyWith(
              fontSize: 8,
              color: on ? onColour : t.textMuted,
            ),
          ),
        ),
      );

  Widget _row(LumitTheme t, int i) {
    final input = i < box.inputs.length ? box.inputs[i] : null;
    final output = i < box.outputs.length ? box.outputs[i] : null;
    return Stack(
      clipBehavior: Clip.none,
      children: [
        if (input != null)
          Positioned.fill(
            child: Padding(
              padding: const EdgeInsets.only(left: 12),
              child: Align(
                alignment: Alignment.centerLeft,
                child: Text(engineLabel(input.label),
                    key: ValueKey<String>(
                        'graph-port-${graphNodeKey(box.node.node)}-in-${input.id}'),
                    style:
                        t.small.copyWith(color: portColour(t, input.portType)),
                    overflow: TextOverflow.ellipsis),
              ),
            ),
          ),
        if (output != null)
          Positioned.fill(
            child: Padding(
              padding: const EdgeInsets.only(right: 12),
              child: Align(
                alignment: Alignment.centerRight,
                child: Text(engineLabel(output.label),
                    key: ValueKey<String>(
                        'graph-port-${graphNodeKey(box.node.node)}-out-${output.id}'),
                    style:
                        t.small.copyWith(color: portColour(t, output.portType)),
                    overflow: TextOverflow.ellipsis),
              ),
            ),
          ),
        if (input != null)
          Positioned(
            left: -graphSocketSize / 2 - 1,
            top: (graphPortRowHeight - graphSocketSize) / 2,
            child: _socket(t, input),
          ),
        if (output != null)
          Positioned(
            right: -graphSocketSize / 2 - 1,
            top: (graphPortRowHeight - graphSocketSize) / 2,
            child: _socket(t, output),
          ),
      ],
    );
  }

  /// A filled socket is wired, a hollow one is not (15-DESIGN §12A.7).
  Widget _socket(LumitTheme t, BridgePort port) {
    final colour = portColour(t, port.portType);
    return Container(
      key: ValueKey<String>(
          'graph-socket-${graphNodeKey(box.node.node)}-${port.id}'),
      width: graphSocketSize,
      height: graphSocketSize,
      decoration: BoxDecoration(
        color: port.wired ? colour : t.surface1,
        shape: BoxShape.circle,
        border: Border.all(color: colour),
      ),
    );
  }
}

/// A node's frame, drawn rather than decorated so that a bypassed one can be
/// dashed — Flutter's `Border` has no dash.
class _NodeFrame extends StatelessWidget {
  final Color colour;
  final Color fill;
  final bool dashed;
  final double radius;
  const _NodeFrame({
    required this.colour,
    required this.fill,
    required this.dashed,
    required this.radius,
  });

  @override
  Widget build(BuildContext context) => CustomPaint(
        painter: _FramePainter(colour, fill, dashed, radius),
      );
}

class _FramePainter extends CustomPainter {
  final Color colour;
  final Color fill;
  final bool dashed;
  final double radius;
  const _FramePainter(this.colour, this.fill, this.dashed, this.radius);

  @override
  void paint(Canvas canvas, Size size) {
    final rect = RRect.fromRectAndRadius(
      Rect.fromLTWH(0.5, 0.5, size.width - 1, size.height - 1),
      Radius.circular(radius),
    );
    canvas.drawRRect(rect, Paint()..color = fill);
    final stroke = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    if (!dashed) {
      canvas.drawRRect(rect, stroke);
      return;
    }
    canvas.drawPath(_dash(Path()..addRRect(rect)), stroke);
  }

  @override
  bool shouldRepaint(_FramePainter old) =>
      old.colour != colour ||
      old.fill != fill ||
      old.dashed != dashed ||
      old.radius != radius;
}

/// A path as 3-on, 3-off dashes — the drawing's own stroke-dasharray.
Path _dash(Path path) {
  final out = Path();
  for (final metric in path.computeMetrics()) {
    var at = 0.0;
    var on = true;
    while (at < metric.length) {
      final to = math.min(at + 3, metric.length);
      if (on) out.addPath(metric.extractPath(at, to), Offset.zero);
      at = to;
      on = !on;
    }
  }
  return out;
}

/// The canvas ground and every wire on it.
class _GraphPainter extends CustomPainter {
  final _Layout layout;
  final List<BridgeGraphEdge> edges;
  final _InFlight? flight;
  final Offset pan;
  final double zoom;
  final Color grid;
  final Color ground;
  final Color dragged;
  final LumitTheme theme;

  const _GraphPainter({
    required this.layout,
    required this.edges,
    required this.flight,
    required this.pan,
    required this.zoom,
    required this.grid,
    required this.ground,
    required this.dragged,
    required this.theme,
  });

  Offset _screen(Offset canvas) => canvas * zoom + pan;

  @override
  void paint(Canvas canvas, Size size) {
    _paintGrid(canvas, size);

    // The image chain's wires are not stored anywhere: they *are* the effect
    // list, read left to right. Drawing them from the box order is what makes
    // the stack view impossible to contradict.
    final chain = layout.chain;
    for (var i = 0; i + 1 < chain.length; i++) {
      final from = chain[i].socket(i == 0 ? 'image' : 'output', false);
      final to = chain[i + 1]
          .socket(i + 1 == chain.length - 1 ? 'image' : 'input', true);
      if (from != null && to != null) {
        _wire(canvas, from, to, theme.port.image, dashes: false);
      }
    }

    for (final edge in edges) {
      final ends = _endsOf(edge);
      if (ends == null) continue;
      _wire(canvas, ends.$1, ends.$2, ends.$3, dashes: false);
    }

    if (flight case final f?) {
      _wire(canvas, f.from.at, f.to, dragged, dashes: true);
    }
  }

  /// Where one stored wire starts and ends, and what colour it takes — the
  /// *source* port's type, which is the type the wire carries. Null when
  /// either end names a box that is not on the canvas.
  (Offset, Offset, Color)? _endsOf(BridgeGraphEdge edge) {
    final (fromKey, fromPort) = switch (edge.from) {
      BridgeOutputRef_Driver(:final node, :final port) => (
          graphNodeKey(BridgeNodeRef.driver(node)),
          port
        ),
      BridgeOutputRef_SourceMatte() => ('source', 'matte'),
    };
    final (toKey, toPort) = switch (edge.to) {
      BridgeInputRef_Param(:final node, :final port) => (
          graphNodeKey(node),
          port
        ),
      BridgeInputRef_Matte(:final effect) => (
          graphNodeKey(BridgeNodeRef.effect(effect)),
          'matte'
        ),
    };
    final fromBox = layout.byKey[fromKey];
    final toBox = layout.byKey[toKey];
    if (fromBox == null || toBox == null) return null;
    final from = fromBox.socket(fromPort, false);
    final to = toBox.socket(toPort, true);
    if (from == null || to == null) return null;
    final type = fromBox.outputs
        .firstWhere((p) => p.id == fromPort,
            orElse: () => fromBox.outputs.first)
        .portType;
    return (from, to, portColour(theme, type));
  }

  void _paintGrid(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = ground);
    final pitch = graphDotGrid * zoom;
    if (pitch < 6) return;
    final dot = Paint()..color = grid;
    final startX = pan.dx % pitch;
    final startY = pan.dy % pitch;
    for (var x = startX; x < size.width; x += pitch) {
      for (var y = startY; y < size.height; y += pitch) {
        canvas.drawCircle(Offset(x, y), 1, dot);
      }
    }
  }

  /// One wire: a cubic whose handles run horizontally out of each socket by
  /// half the gap between them, which is the curve both drawings draw.
  void _wire(Canvas canvas, Offset from, Offset to, Color colour,
      {required bool dashes}) {
    final a = _screen(from);
    final b = _screen(to);
    final reach = (b.dx - a.dx) / 2;
    final path = Path()
      ..moveTo(a.dx, a.dy)
      ..cubicTo(a.dx + reach, a.dy, b.dx - reach, b.dy, b.dx, b.dy);
    final paint = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = graphWireWidth * zoom;
    canvas.drawPath(dashes ? _dash(path) : path, paint);
  }

  @override
  bool shouldRepaint(_GraphPainter old) => true;
}
