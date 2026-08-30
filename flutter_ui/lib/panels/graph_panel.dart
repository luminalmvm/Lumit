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
// and which boxes are twirled open, all committed by one `setGraph` per
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

import 'dart:io' show File;
import 'dart:math' as math;
import 'dart:typed_data' show Float32List;
import 'dart:ui' show PointMode;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart'
    show HardwareKeyboard, KeyDownEvent, LogicalKeyboardKey;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart' show LumitIcon, lumitIcon;
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/fx_console_frb.dart';
import '../state/dock.dart';
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/marquee.dart';
import 'fx_section.dart' show fxEnableMark, fxEnableMarkScale;
import 'placeholder.dart';
import 'shader_graph.dart' show ShaderGraphPanel;
import 'timeline_extras_frb.dart' show DoubleTap;

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

/// A mark in a node's header — the enable tick, the twirl, the no-stream
/// badge — border included. One number, so the header's row of small things
/// steps evenly whatever each of them is.
const double graphBadgeSize = 14;

/// The enable tick's cell. The Effect controls heading draws K-450's 14px
/// checkbox [fxEnableMarkScale] larger, and the node header wears that same
/// mark at that same size, so the cell that holds it is that much larger than
/// the marks beside it — the tick is one control on two surfaces, not two
/// drawings of one.
const double graphEnableSize = graphBadgeSize * fxEnableMarkScale;

/// The twirl glyph on a node header, smaller than the house default because
/// the header is 21px rather than the 24 an Effect controls heading has
/// (K-456: the default is a default, not a law).
const double graphTwirlSize = 12;

/// The dot grid's pitch on the canvas ground.
const double graphDotGrid = 20;

/// Every wire, and the dashed one in flight.
const double graphWireWidth = 1.5;

/// How far a wire must leave its socket **horizontally** before it may curve
/// back — canvas units, so it scales with the zoom like everything else.
///
/// Without a floor the cubic's handles are `±dx/2`, which is fine while the
/// consumer sits to the right of its producer and collapses to nothing the
/// moment it does not: a box dragged left of — or on top of — the one feeding
/// it left a wire that ran backwards *through* both cards and was invisible
/// behind them. Forty units is the classic node-editor S-curve: the wire
/// visibly comes out of the output socket, loops back, and goes into the input
/// socket from the left, so which end is which stays readable at any layout.
const double graphWireStub = 40;

/// One wire's curve, in **screen** coordinates (both ends already transformed).
///
/// A cubic whose handles run horizontally out of each socket — right out of the
/// output, left into the input — by half the gap between them, or by
/// [graphWireStub] when that half is smaller or points the wrong way. Free of
/// the painter so its geometry can be asserted directly.
Path graphWirePath(Offset a, Offset b, {double zoom = 1}) {
  final reach = math.max(graphWireStub * zoom, (b.dx - a.dx).abs() / 2);
  return Path()
    ..moveTo(a.dx, a.dy)
    ..cubicTo(a.dx + reach, a.dy, b.dx - reach, b.dy, b.dx, b.dy);
}

/// The glyphs in the toolbar. A size down from the
/// row glyphs' 16 (K-456: the manifest's number, not a preference).
const double graphIconSize = 13;

// The graph adds through the Ctrl+Space console (K-645, K-673): one search
// surface, one key. `shell/fx_console_frb.dart` owns its shape.

/// A group's wash: the air it leaves round its members, and the band its name
/// sits in above them (K-651, the NodeGraph drawing's own numbers).
const double graphGroupPad = 12;
const double graphGroupHead = 18;

/// What a saved group is called on disk — the engine's own constant, restated
/// here only because a file dialogue needs the letters.
const String graphGroupExtension = 'lumgrp';

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

/// The header tint a **Custom shader** box wears, in the outer graph and on
/// every box of its inner graph alike (K-675, owner item 13b): the theme's
/// own viz family — `curve[0]`, the household ramp's lead — washed over the
/// ordinary header ground, so a programmable box reads at a glance without a
/// third colour system beside the port legend and the label palette.
Color graphShaderHeader(LumitTheme t) =>
    Color.alphaBlend(t.curve.first.withValues(alpha: 0.14), t.surface2);

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

/// What a parameter row needs to know about the wire feeding it (K-471): the
/// driver's name, the type the wire carries, and whether the source is a box
/// reading a points stream that has none (K-509).
typedef DrivenParam = ({String driver, BridgePortType type, bool noStream});

/// Which of [layer]'s effect parameters a driver is wired to, by
/// `effectId/paramId`. A wire's colour is its **source** port's type, which is
/// what the parameter is now following.
///
/// **One read, held.** Every panel that draws parameter rows — Effect controls,
/// the Node panel, the Timeline's fold-out — asks this at the moments the graph
/// can change (the selection moves, the document commits) and keeps the answer;
/// asking it from a build is exactly the traffic `bridge_call_budget_test`
/// guards against (K-183). Empty for every layer that has never been wired,
/// which is nearly all of them.
Map<String, DrivenParam> drivenParamsOf(LayerReference layer) {
  final out = <String, DrivenParam>{};
  final BridgeLayerGraph graph;
  try {
    graph = layer.getGraph();
  } catch (_) {
    // The layer has gone; the rows simply draw their own controls again.
    return out;
  }
  final byRef = {for (final n in graph.nodes) graphNodeKey(n.node): n};
  for (final edge in graph.wiring.edges) {
    if (edge.to case BridgeInputRef_Param(:final node, :final port)) {
      if (node is! BridgeNodeRef_Effect) continue;
      final (fromKey, fromPort) = switch (edge.from) {
        BridgeOutputRef_Driver(node: final d, port: final p) => (
            graphNodeKey(BridgeNodeRef.driver(d)),
            p
          ),
        BridgeOutputRef_SourceMatte() => ('source', 'matte'),
        // A points wire's source is a *stack effect* (K-492), so the row names
        // the effect that hands the data over.
        BridgeOutputRef_EffectData(:final effect, :final port) => (
            graphNodeKey(BridgeNodeRef.effect(effect)),
            port
          ),
      };
      final source = byRef[fromKey];
      if (source == null) continue;
      final socket = source.outputs.where((o) => o.id == fromPort);
      if (socket.isEmpty) continue;
      out['${node.field0}/$port'] = (
        driver: source.customName ?? engineLabel(source.label),
        type: socket.first.portType,
        noStream: graphNoStream(source),
      );
    }
  }
  return out;
}

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

/// The rectangle a group's wash covers, in canvas units: its members' own
/// bounds, plus air all round and a band above for its name.
///
/// **Derived, never stored** (K-651). A group holds names, not geometry, so the
/// wash follows a member the moment it is dragged and cannot go stale. Null
/// where the canvas is drawing none of the members — a group whose boxes have
/// all gone draws nothing rather than a rectangle round the origin.
Rect? graphGroupRect(Iterable<Rect> members) {
  Rect? bounds;
  for (final member in members) {
    bounds = bounds == null ? member : bounds.expandToInclude(member);
  }
  if (bounds == null) return null;
  return Rect.fromLTRB(
    bounds.left - graphGroupPad,
    bounds.top - graphGroupPad - graphGroupHead,
    bounds.right + graphGroupPad,
    bounds.bottom + graphGroupPad,
  );
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
      // socket when a wire is on it or when the box is twirled open
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

/// Sockets that are always drawn, wired or not: the picture's own path, and
/// every **wire-only** port beside it.
///
/// A wire-only port has no stored value and therefore no panel row anywhere
/// (points-stream.md §2.2) — a matte's, an audio stream's, a points stream's.
/// The socket is the only way to reach it, so hiding it behind the twirl
/// would hide the whole port. A *parameter* socket is different: the row is
/// still there in Effect controls, so the canvas may keep it folded away.
bool _alwaysDrawn(BridgePortType type) =>
    type == BridgePortType.image ||
    type == BridgePortType.matte ||
    type == BridgePortType.audio ||
    type == BridgePortType.points;

/// A box that consumes a points stream with **nothing wired into it**.
///
/// Structural, not per-frame: the read model says whether the socket carries a
/// wire, and the engine's contract says what an empty stream answers — Count
/// nought and Nearest distance "nothing is anywhere near" (points-stream.md
/// §2.2). So the panel can say so without asking for a value, which is what
/// K-509 asks of it and what keeps this off the rebuild path (K-183).
bool graphNoStream(BridgeGraphNode node) =>
    node.inputs.any((p) => p.portType == BridgePortType.points && !p.wired);

/// Where one stored wire starts and ends in canvas units, and the type it
/// carries — the *source* port's, which is the type the wire is. Null when
/// either end names a box that is not on the canvas.
///
/// Read by the painter, which colours the wire by that type, and by the drop
/// test, which asks how near a dragged box is to the curve (N7).
(Offset, Offset, BridgePortType)? _edgeEnds(_Layout layout, BridgeGraphEdge e) {
  final (fromKey, fromPort) = switch (e.from) {
    BridgeOutputRef_Driver(:final node, :final port) => (
        graphNodeKey(BridgeNodeRef.driver(node)),
        port
      ),
    BridgeOutputRef_SourceMatte() => ('source', 'matte'),
    BridgeOutputRef_EffectData(:final effect, :final port) => (
        graphNodeKey(BridgeNodeRef.effect(effect)),
        port
      ),
  };
  final (toKey, toPort) = switch (e.to) {
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
  final i = fromBox.outputs.indexWhere((p) => p.id == fromPort);
  return (from, to, fromBox.outputs[i < 0 ? 0 : i].portType);
}

/// How near a wire a dropped box has to land to fall into it, canvas units.
const double _wireGrab = 18;

/// How far [at] is from a wire's curve, sampled along it.
///
/// A wire is one cubic and twenty points describe it closely enough to say
/// whether a box was dropped on it — this runs during a drag, over a handful of
/// wires, and never in an idle rebuild.
double _wireDistance(Offset a, Offset b, Offset at) {
  var best = double.infinity;
  for (final metric in graphWirePath(a, b).computeMetrics()) {
    for (var i = 0; i <= 20; i++) {
      final point =
          metric.getTangentForOffset(metric.length * i / 20)?.position;
      if (point != null) best = math.min(best, (point - at).distance);
    }
  }
  return best;
}

/// A wire being dragged: where it left, and where the pointer is now.
class _InFlight {
  final _Socket from;

  /// The stored wire this drag took hold of, when the press landed on an input
  /// that already had one. A wire is grabbed by its **far** end — the drag
  /// leaves the producer's socket and follows the pointer — so letting go
  /// somewhere else re-routes it and letting go of nothing takes it off. Null
  /// for a wire being drawn afresh.
  final BridgeGraphEdge? detached;

  /// An **image-chain** wire in hand (K-674): the chain index of the box the
  /// grabbed wire feeds. The chain's wires are not stored anywhere — they are
  /// the effect list — so the drag carries the index rather than an edge, and
  /// the drop lowers to the stack's own ops. Null for every stored wire.
  final int? chain;

  Offset to;
  _InFlight(this.from, this.to, {this.detached, this.chain});
}

/// Boxes being dragged: where the pointer took hold, and where each box being
/// moved started.
///
/// A drag on a box that is **part of the picked set moves the whole set**
/// (docs/07 §4.5's rule for every surface here), so this carries an origin per
/// box rather than one.
class _NodeDrag {
  /// The box actually pressed — what a release that never moved collapses the
  /// selection to, because a plain click replaces.
  final String key;
  final Offset grab;
  final Map<String, Offset> origins;

  /// The press landed on a box that was already picked, so the pick was left
  /// alone on the way down. A click that does not move then means what a plain
  /// click always means: this box, and only this box.
  final bool collapse;
  _NodeDrag(this.key, this.grab, this.origins, {this.collapse = false});
}

class GraphPanelFrb extends StatefulWidget {
  /// The driver catalogue seam, injected by tests so the popover can be
  /// asserted without the real registry.
  final List<BridgeEffectInfo> Function()? driversLister;

  /// The saved-group library seam, injected for the same reason — a test says
  /// what is in the folder rather than writing to the user's own library.
  final List<BridgePresetInfo> Function()? groupsLister;

  /// Where "Save group…" writes, injected so a test never opens a system file
  /// dialogue. Null takes the picker, starting in the preset library folder.
  final Future<String?> Function()? groupSavePicker;

  const GraphPanelFrb({
    super.key,
    this.driversLister,
    this.groupsLister,
    this.groupSavePicker,
  });

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

  /// **The picked boxes**, by node key, in the order they were picked
  /// (K-533, K-523).
  ///
  /// A set rather than one box, which is what Delete, Bypass, Expose and
  /// `Ctrl+A` were all waiting for: they were singular because the selection
  /// was, not because any of them is singular by nature. Keyed by
  /// [graphNodeKey] because that string is this panel's idea of identity
  /// everywhere else — the positions map, the layout, the exposed list — and
  /// the reference is kept as the value because the commits need it.
  ///
  /// Two things follow the pick, and neither panel knows this one exists:
  /// [LumitUiState.graphNode] carries the **anchor** — the box picked last —
  /// because the Node panel draws one box's rows and that is singular the way
  /// a rename is; and [LumitUiState.setEffectSelection] carries every picked
  /// effect, because the graph's box and the Effect controls heading are one
  /// selection (K-300).
  final Map<String, BridgeNodeRef> _selection = {};

  /// The anchor: the box picked last, or null with nothing picked.
  BridgeNodeRef? get _selected =>
      _selection.isEmpty ? null : _selection.values.last;

  bool _isPicked(BridgeNodeRef node) =>
      _selection.containsKey(graphNodeKey(node));

  /// Replace the pick outright, and tell the two surfaces that follow it.
  ///
  /// Called from inside `setState` at every site, exactly as the old field
  /// assignment was — the mirroring is what must not be forgotten, so it lives
  /// here rather than at five call sites.
  void _pick(Iterable<BridgeNodeRef> nodes) {
    _selection
      ..clear()
      ..addEntries([for (final n in nodes) MapEntry(graphNodeKey(n), n)]);
    _publishPick();
  }

  void _publishPick() {
    _ui?.graphNode.value = _selected;
    final layer = _layer;
    if (layer == null) return;
    // In stack order, which is the order `selectedEffects` is documented to
    // hold and the order the graph's own node list is already in. A pick of
    // drivers alone carries no effects, and an empty list clears — which is
    // right: the heading in Effect controls should not stay lit for a box that
    // is not an effect.
    _ui?.setEffectSelection(layer, [
      for (final node in _graph?.nodes ?? const <BridgeGraphNode>[])
        if (_selection.containsKey(graphNodeKey(node.node)))
          if (_effectIdOf(node.node) case final effect?) effect,
    ]);
  }

  /// The box whose name is an inline editor, by node key (K-321). A
  /// double-click on a card's name opens it — the canvas reads its pointers
  /// through a raw `Listener`, so the card's own double-tap costs the
  /// selection nothing: a press still picks the box the instant it lands.
  String? _renamingNode;

  Offset _pan = Offset.zero;
  double _zoom = 1;

  /// Adding a node wires it up in the same commit; deleting one takes its
  /// wires with it. Both on, both the drawing's state, and both `animated`
  /// while on because that is what a pill switch is everywhere (K-465).
  bool _autoWire = true;
  bool _heal = true;

  /// The snap magnet (K-659): while on — and it starts on, drawn
  /// lit in the toolbar — a node drag lands on the dot grid's own pitch
  /// (K-626), so boxes line up without nudging. Session view state, exactly
  /// as Auto-wire and Heal are.
  bool _snapToGrid = true;

  _InFlight? _flight;
  _NodeDrag? _nodeDrag;

  /// The double-click that enters a Custom shader's inner graph (K-642), and
  /// which box the last press landed on so two clicks on two boxes are not one
  /// double-click.
  final DoubleTap _boxTaps = DoubleTap();
  String? _boxTapKey;

  /// The wire the box being dragged is over and would drop into (N7), drawn
  /// picked while it is. Held so the highlight and the release agree.
  BridgeGraphEdge? _dropWire;
  Offset? _panFrom;
  Offset? _pressAt;

  /// Set by a control on a card on its way down, so the canvas behind it
  /// leaves the press alone: pressing a box's enable tick or its twirl is not
  /// picking the box. The canvas reads every pointer itself — that is how a
  /// socket is grabbed without a gesture detector per socket — so it cannot
  /// tell a press on a control from a press on the card, and the control has
  /// to say.
  bool _claimed = false;

  /// The rubber band in flight, in the canvas widget's own pixels (so it is
  /// drawn without the pan/zoom transform and turned into canvas units only
  /// when it is released), and whether it adds to the standing pick.
  Offset? _marqueeFrom;
  Offset? _marqueeTo;
  bool _marqueeAdds = false;

  /// Whether the search console is up, so a second ask cannot stack another.
  bool _searching = false;

  final FocusNode _canvasFocus = FocusNode(debugLabel: 'graph canvas');

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (identical(ui, _ui)) return;
    _unbind();
    _ui = ui;
    ui.selectedLayer.addListener(_reload);
    ui.model.addListener(_reload);
    // Entering (or leaving) a Custom shader's inner graph swaps this panel's
    // whole face (K-642 §4.2), from either surface the double-click lives on.
    ui.shaderGraphEntry.addListener(_onShaderEntry);
    // `Ctrl+A` here means every box on this canvas (K-522), which it can only
    // mean now the pick is a set.
    ui.selectAllRequest.addListener(_onSelectAllRequested);
    // **Ctrl+Space with this panel focused adds to the graph** (K-673). The
    // shell's console applies to the selected layers; the same key over this
    // canvas opens the same popover wearing the canvas's own list. Chained the
    // way the Delete claim is, because there is one claim and the inner shader
    // graph wants it too when it is the panel's face.
    if (ui.consoleClaim != _consoleClaim) _priorConsoleClaim = ui.consoleClaim;
    ui.consoleClaim = _consoleClaim;
    // **Delete means the picked boxes while this panel is the focused one**
    // (K-234's mechanism). Claimed rather than left to the canvas's own focus
    // handler: the shell answers Delete on the hardware keyboard, which runs
    // *before* the focus tree and swallows the key, so a picked node had its
    // layer deleted out from under it instead. The shell asks this claim first
    // and stands down when it says yes.
    //
    // Chained onto whatever held the claim before — the Timeline, for its mask
    // rows — because there is one claim and more than one panel that wants it.
    if (ui.deleteClaim != _deleteClaim) _priorDeleteClaim = ui.deleteClaim;
    ui.deleteClaim = _deleteClaim;
    _reload();
  }

  /// The claim this panel had to displace to take Delete.
  bool Function()? _priorDeleteClaim;

  bool _deleteClaim() {
    final ui = _ui;
    if (!mounted || ui == null || ui.activePanel.value != Panel.graph) {
      return _priorDeleteClaim?.call() ?? false;
    }
    return _deleteSelected() || (_priorDeleteClaim?.call() ?? false);
  }

  /// The claim this panel had to displace to take Ctrl+Space.
  bool Function()? _priorConsoleClaim;

  /// Ctrl+Space, answered while this panel is the focused one and showing the
  /// layer's own graph (a Custom shader's inner graph holds a claim of its own
  /// above this one). The chosen box lands mid-canvas; a wire drag has its own
  /// door into [_openSearch] and never comes through here.
  bool _consoleClaim() {
    final ui = _ui;
    if (!mounted ||
        ui == null ||
        ui.activePanel.value != Panel.graph ||
        ui.shaderGraphEntry.value != null ||
        _graph == null ||
        _searching) {
      return _priorConsoleClaim?.call() ?? false;
    }
    _openSearch(_toCanvas(Offset(_viewport.width / 2, _viewport.height / 2)));
    return true;
  }

  void _onShaderEntry() {
    if (mounted) setState(() {});
  }

  void _unbind() {
    _ui?.selectedLayer.removeListener(_reload);
    _ui?.model.removeListener(_reload);
    _ui?.shaderGraphEntry.removeListener(_onShaderEntry);
    _ui?.selectAllRequest.removeListener(_onSelectAllRequested);
    if (_ui?.deleteClaim == _deleteClaim) {
      _ui!.deleteClaim = _priorDeleteClaim;
    }
    if (_ui?.consoleClaim == _consoleClaim) {
      _ui!.consoleClaim = _priorConsoleClaim;
    }
  }

  /// Pick every box the canvas is drawing — Source and Layer out included,
  /// because they are picked by a click like any other box and the commands
  /// that cannot touch them already say so themselves.
  void _onSelectAllRequested() {
    final ui = _ui;
    if (!mounted || ui == null) return;
    if (!ui.selectAllRequestIsFor(Panel.graph)) return;
    setState(() => _pick([for (final n in _graph?.nodes ?? const []) n.node]));
  }

  @override
  void dispose() {
    _unbind();
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
      // A box that has gone leaves the pick; the rest of the pick stands.
      final present =
          (graph?.nodes ?? const []).map((n) => graphNodeKey(n.node)).toSet();
      if (_selection.keys.any((k) => !present.contains(k))) {
        _selection.removeWhere((key, _) => !present.contains(key));
        _publishPick();
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
    List<BridgeNodeGroup>? groups,
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
      groups: groups ?? w.groups,
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
  ///
  /// **An output is not an input.** Only the destination is exclusive: the
  /// edges kept are every one that does not land on this socket, so a producer
  /// goes on feeding everything it already fed and a second wire out of it is
  /// an addition rather than a replacement. [without] is the wire this gesture
  /// took off an input on its way here, which leaves in the same commit.
  void _connect(_Socket a, _Socket b, {BridgeGraphEdge? without}) {
    final from = a.isInput ? b : a;
    final to = a.isInput ? a : b;
    final source = _outputRef(from);
    final dest = _inputRef(to);
    if (source == null || dest == null) return;
    _commit(_wiringNow(edges: [
      for (final e in _graph!.wiring.edges)
        if (e.to != dest && e != without) e,
      BridgeGraphEdge(from: source, to: dest),
    ]));
  }

  /// Take one wire off, as its own `setGraph` and so its own undo step.
  void _removeEdge(BridgeGraphEdge edge) {
    _commit(_wiringNow(edges: [
      for (final e in _graph!.wiring.edges)
        if (e != edge) e,
    ]));
  }

  // --- The image chain's own wires (K-674) --------------------------------

  /// A chain wire let go (owner item 10). The chain **is** the effect list
  /// (§1.1), so both answers lower to the stack's own ops, each one op and so
  /// one undo step:
  ///
  /// * **on another chain input** — re-route, which §1.1 names outright:
  ///   "rewiring the chain = reorder". The box whose input took the drop
  ///   moves to sit right after the wire's source; dropped on the Layer out,
  ///   it is the source that moves, to the end of the list.
  /// * **on empty canvas** — the connection goes, and the only honest way a
  ///   derived wire can go is for the box it fed to leave the list: the
  ///   effect is removed, its neighbours joining by construction — the
  ///   inverse of N7's drop-into-a-wire insert. The Heal toggle governs
  ///   stored wires; the chain, as Delete already documents, "heals by
  ///   construction either way". Unplugging the Layer out takes the last
  ///   effect, which is the box that connection is.
  ///
  /// A drop on anything else — a driver socket, a parameter — is declined
  /// without a bridge call: the picture's path cannot leave the chain.
  void _chainDrop(int held, _Socket? landed, _Layout layout) {
    final layer = _layer;
    final chain = layout.chain;
    if (layer == null || held < 1 || held >= chain.length) return;
    final upstream = _effectIdOf(chain[held - 1].node.node);

    if (landed == null) {
      final victim =
          _effectIdOf(chain[held].node.node) ?? upstream;
      if (victim == null) return; // Source → Out: no effect to take out.
      try {
        for (final instance in layer.getEffects()) {
          if (instance.id() == victim) {
            layer.removeEffect(effect: instance);
            break;
          }
        }
      } catch (_) {
        // Refused, or the stack moved under us; re-reading is the recovery.
      }
      _ui?.model.refresh();
      _reload();
      return;
    }

    if (!_isChainType(landed.port.portType) || !landed.isInput) return;
    final j = chain.indexWhere(
        (b) => graphNodeKey(b.node.node) == graphNodeKey(landed.node));
    if (j < 0 || j == held) return;
    final moved = _effectIdOf(chain[j].node.node);
    if (moved == null) {
      // Dropped on the Layer out: the wire's source becomes the last effect.
      if (upstream != null) _reorderToEnd(layer, upstream);
    } else {
      _reorderAfter(layer, moved, upstream);
    }
    _ui?.model.refresh();
    _reload();
  }

  /// Move [moved] to sit immediately after [after] in the stack — null puts
  /// it first, which is what a wire out of the Source means. One `reorder`
  /// op, so one undo step.
  void _reorderAfter(LayerReference layer, UuidValue moved, UuidValue? after) {
    try {
      final stack = layer.getEffects();
      final ids = [for (final e in stack) e.id()];
      final from = ids.indexOf(moved);
      if (from < 0 || moved == after) return;
      var to = after == null ? 0 : ids.indexOf(after) + 1;
      if (after != null && to == 0) return;
      // `newIndex` is the slot in the list *after* removal.
      if (from < to) to -= 1;
      if (to != from) layer.reorderEffect(effect: stack[from], newIndex: to);
    } catch (_) {
      // Refused, or the stack moved under us; re-reading is the recovery.
    }
  }

  void _reorderToEnd(LayerReference layer, UuidValue moved) {
    try {
      final stack = layer.getEffects();
      final from = stack.indexWhere((e) => e.id() == moved);
      if (from < 0 || from == stack.length - 1) return;
      layer.reorderEffect(effect: stack[from], newIndex: stack.length - 1);
    } catch (_) {
      // Refused, or the stack moved under us; re-reading is the recovery.
    }
  }

  /// The stored wire landing on [socket], if one does.
  BridgeGraphEdge? _edgeInto(_Socket socket) {
    final dest = _inputRef(socket);
    if (dest == null) return null;
    for (final e in _graph!.wiring.edges) {
      if (e.to == dest) return e;
    }
    return null;
  }

  /// The socket a stored wire leaves from, as the canvas draws it.
  _Socket? _sourceSocket(BridgeGraphEdge edge, _Layout layout) {
    final box = layout.byKey[_sourceKey(edge)];
    if (box == null) return null;
    final portId = switch (edge.from) {
      BridgeOutputRef_Driver(:final port) => port,
      BridgeOutputRef_SourceMatte() => 'matte',
      BridgeOutputRef_EffectData(:final port) => port,
    };
    final i = box.outputs.indexWhere((p) => p.id == portId);
    final at = box.socket(portId, false);
    if (i < 0 || at == null) return null;
    return _Socket(box.node.node, box.outputs[i], false, at);
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
    if (_driverIdOf(socket.node) case final driver?) {
      return BridgeOutputRef.driver(node: driver, port: socket.port.id);
    }
    // A **stack effect's** declared data output — the first wire whose source
    // is the effect list itself (K-492). The picture's own `output` port never
    // reaches here: a chain socket takes no drag at all.
    if (_effectIdOf(socket.node) case final effect?) {
      return BridgeOutputRef.effectData(effect: effect, port: socket.port.id);
    }
    return null;
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
    // comes only from a footage layer's own stream in this phase (K-435). Its
    // Volume socket is the one exception (K-697): a number wired there drives
    // the layer's own Volume — the Duck under landing.
    if (into.node is BridgeNodeRef_Out && into.port.id != 'volume') return false;
    if (out.port.portType != into.port.portType) return false;
    return !_wouldLoop(out.node, into.node);
  }

  /// Whether joining [from] to [into] would close a loop.
  ///
  /// v1 makes one constructible for the first time (points-stream.md §1.2):
  /// Points sample reads Particulate's stream and its Count drives
  /// Particulate's Emit rate — the stream depending on the parameters and the
  /// parameters on the stream. The engine refuses it at commit with the
  /// `Cycle` sentence and that refusal is the backstop, but a refusal the
  /// panel swallows looks to the user like a gesture that did nothing. So the
  /// drop is declined *here*, from the edges the panel is already holding, and
  /// nothing crosses the bridge.
  ///
  /// The walk is over the stored wires only, exactly as the engine's is: the
  /// image chain is the effect list and cannot loop.
  bool _wouldLoop(BridgeNodeRef from, BridgeNodeRef into) {
    final target = graphNodeKey(from);
    final seen = <String>{};
    final queue = <String>[graphNodeKey(into)];
    while (queue.isNotEmpty) {
      final at = queue.removeLast();
      if (at == target) return true;
      if (!seen.add(at)) continue;
      for (final edge in _graph!.wiring.edges) {
        if (_sourceKey(edge) == at) queue.add(_destKey(edge));
      }
    }
    return false;
  }

  String _sourceKey(BridgeGraphEdge edge) => switch (edge.from) {
        BridgeOutputRef_Driver(:final node) =>
          graphNodeKey(BridgeNodeRef.driver(node)),
        BridgeOutputRef_SourceMatte() => 'source',
        BridgeOutputRef_EffectData(:final effect) =>
          graphNodeKey(BridgeNodeRef.effect(effect)),
      };

  String _destKey(BridgeGraphEdge edge) => switch (edge.to) {
        BridgeInputRef_Param(:final node) => graphNodeKey(node),
        BridgeInputRef_Matte(:final effect) =>
          graphNodeKey(BridgeNodeRef.effect(effect)),
      };

  // --- Node gestures ------------------------------------------------------

  /// The boxes a command pressed on [node] acts on (K-523): the whole pick
  /// when this box is part of it, and this box alone when it is not — the rule
  /// every other surface here follows.
  List<BridgeNodeRef> _targets(BridgeNodeRef node) =>
      _isPicked(node) ? _selection.values.toList() : [node];

  void _toggleExposed(BridgeNodeRef node) {
    final key = graphNodeKey(node);
    // The pressed box's new state, for all of them, so a pick of mixed twirls
    // comes out even. One `setGraph`, so one undo step however many it is.
    final on = !_graph!.wiring.exposed.any((e) => graphNodeKey(e) == key);
    final targets = {for (final n in _targets(node)) graphNodeKey(n): n};
    _commit(_wiringNow(exposed: [
      for (final e in _graph!.wiring.exposed)
        if (!targets.containsKey(graphNodeKey(e))) e,
      if (on) ...targets.values,
    ]));
  }

  /// Bypass. A driver rides the staged-instance path with `setGraph` as its
  /// commit; an effect rides the stack's own call, exactly as its card in
  /// Effect controls does. One op either way.
  void _toggleBypass(BridgeGraphNode node) {
    final layer = _layer;
    if (layer == null) return;
    // The pressed box's new state, for every box the press acts on (K-523), so
    // a pick of mixed bypasses comes out even.
    final on = !node.enabled;
    final targets = {for (final n in _targets(node.node)) graphNodeKey(n)};
    final drivers = <UuidValue>{};
    final effects = <UuidValue>{};
    for (final n in _graph?.nodes ?? const <BridgeGraphNode>[]) {
      if (!targets.contains(graphNodeKey(n.node))) continue;
      if (_driverIdOf(n.node) case final id?) drivers.add(id);
      if (_effectIdOf(n.node) case final id?) effects.add(id);
    }

    final project = Provider.of<LumitState>(context, listen: false).project;
    final group = project != null && drivers.length + effects.length > 1;
    if (group) project.beginUndoGroup();
    try {
      if (drivers.isNotEmpty) {
        // Every picked driver rides one staged list and one `setGraph`.
        final staged = layer.getGraphDrivers();
        for (final instance in staged) {
          if (drivers.contains(instance.id())) {
            instance.setEnabled(enabled: on);
          }
        }
        layer.setGraph(drivers: staged, wiring: _wiringNow());
      }
      for (final instance in layer.getEffects()) {
        if (effects.contains(instance.id())) {
          layer.setEffectEnabled(effect: instance, enabled: on);
        }
      }
    } catch (_) {
      // The stack or the graph moved under us; re-reading is the recovery.
    }
    if (group) project.endUndoGroup();
    _ui?.model.refresh();
    _reload();
  }

  /// The user's own name for a box (K-321), committed the way its bypass is:
  /// a driver stages on the graph's driver list and commits `setGraph`, an
  /// effect stages on the stack and commits `setEffects`. One op, one undo
  /// step, either way — and no new call across the bridge: `set_custom_name`
  /// is already on `BridgeEffectInstance`, which is the handle both lists hand
  /// out.
  ///
  /// An empty name clears back to the box's own label; the engine's own
  /// `set_custom_name` trims and does that, so nothing here has to.
  void _renameNode(BridgeGraphNode node, String name) {
    setState(() => _renamingNode = null);
    final layer = _layer;
    if (layer == null) return;
    final driver = _driverIdOf(node.node);
    try {
      if (driver != null) {
        final staged = layer.getGraphDrivers();
        for (final instance in staged) {
          if (instance.id() == driver) {
            instance.setCustomName(name: name);
            layer.setGraph(drivers: staged, wiring: _wiringNow());
            break;
          }
        }
      } else if (_effectIdOf(node.node) case final effect?) {
        // The whole stack is staged and committed together, exactly as the
        // Effect controls card's own rename does it: `setEffectEnabled` has a
        // committing op of its own but a custom name has not, so the staged
        // copy is what carries it home.
        final stack = layer.getEffects();
        for (final instance in stack) {
          if (instance.id() == effect) {
            instance.setCustomName(name: name);
            layer.setEffects(effects: stack);
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

  /// Delete every picked box.
  ///
  /// **Heal** decides what happens to the wires on them. On, they go with the
  /// boxes in the same commit; off, a box that still carries a wire is left
  /// alone — unplug it first, and the document never passes through a state
  /// where a wire names a box that is not there. With several picked that is
  /// decided per box, so an unwired one still goes.
  ///
  /// The image chain heals by construction either way: taking an effect out of
  /// the list joins its neighbours, because the list *is* the chain.
  ///
  /// **The kept edges are worked out against every victim at once**, which is
  /// what keeps a plural delete one undo step rather than a cascade: the
  /// drivers leave in a single `setGraph` carrying those edges, and where
  /// effects go too the whole thing is wrapped in one undo group.
  ///
  /// Answers whether it took anything. A pick of nothing but the picture's own
  /// ends is not this panel's Delete, and the claim has to say so.
  bool _deleteSelected() {
    final layer = _layer;
    if (layer == null || _graph == null || _selection.isEmpty) return false;

    // Source and Layer out are the picture's own ends, not boxes anyone put
    // there: they are picked like any other and deleted like none.
    final victims = [
      for (final node in _selection.values)
        if (node is! BridgeNodeRef_Source && node is! BridgeNodeRef_Out)
          if (_heal || !_graph!.wiring.edges.any((e) => _touches(e, node)))
            node,
    ];
    if (victims.isEmpty) return false;

    final kept = [
      for (final e in _graph!.wiring.edges)
        if (!victims.any((v) => _touches(e, v))) e,
    ];
    final drivers = {
      for (final v in victims)
        if (_driverIdOf(v) case final id?) id,
    };
    final effects = {
      for (final v in victims)
        if (_effectIdOf(v) case final id?) id,
    };

    // One undo step for the gesture. A pick of drivers alone is already one
    // `setGraph`, and a pick of effects alone one `SetLayerEffects` each — the
    // group is what makes a mixed pick, or several effects, undo whole.
    final project = Provider.of<LumitState>(context, listen: false).project;
    final group = project != null && drivers.length + effects.length > 1;
    if (group) project.beginUndoGroup();

    if (drivers.isNotEmpty) {
      // The wires go with them in the same write.
      _commit(
        _wiringNow(edges: kept),
        drivers: [
          for (final d in layer.getGraphDrivers())
            if (!drivers.contains(d.id())) d,
        ],
      );
    }
    if (effects.isNotEmpty) {
      // An effect's removal is the stack's own op, and one op is all it takes:
      // `Op::SetLayerEffects` prunes the edges, positions and exposures naming the
      // box that went, inside the same commit, so this is one write per effect
      // exactly as a driver's removal is one for all of them.
      try {
        for (final instance in layer.getEffects()) {
          if (effects.contains(instance.id())) {
            layer.removeEffect(effect: instance);
          }
        }
      } catch (_) {
        // Refused; what had already gone stays gone and the rest stands.
      }
    }
    if (group) project.endUndoGroup();

    for (final v in victims) {
      _positions.remove(graphNodeKey(v));
      _selection.remove(graphNodeKey(v));
    }
    _publishPick();
    _ui?.model.refresh();
    _reload();
    return true;
  }

  bool _touches(BridgeGraphEdge edge, BridgeNodeRef node) {
    final key = graphNodeKey(node);
    return _sourceKey(edge) == key || _destKey(edge) == key;
  }

  // --- The console --------------------------------------------------------

  /// Ctrl+Space, and a wire let go over empty canvas, open **the console**
  /// (K-645, K-673) — the same popover the shell opens, with a foot line
  /// saying what a row will do. One search surface, two doors: what the
  /// canvas contributes is the list, the spot the box lands on, and the
  /// sentence.
  Future<void> _openSearch(Offset at, {_Socket? wire}) async {
    if (_searching) return;
    setState(() => _searching = true);
    final all = (widget.driversLister ?? listDrivers)();
    // `listEffects` carries the drivers too (K-645); here they come from the
    // drivers listing instead, because the canvas's own add places the box on
    // the drop spot and rides the wire in one commit.
    final driverNames = {for (final driver in all) driver.name};
    try {
      await showFxConsoleFrb(
        context: context,
        anchor: lastKnownPointerPosition,
        model: FxConsoleModel(
          // A wire summoned this by a drop, not by a key, so it wears none.
          keyHint: wire == null ? l10n.fxConsoleKey : null,
          footer: wire == null ? l10n.graphConsoleAdds : l10n.graphSearchWires,
          entries: [
            // The drivers first — the graph's own family. With a wire in hand
            // the list is the entries that wire could actually land on, which
            // is what makes the foot's sentence true: pick one and it is
            // connected.
            for (final driver in all)
              if (wire == null || _fitsWire(driver, wire))
                FxConsoleEntry(
                  label: engineLabel(driver.label),
                  kind: FxConsoleKind.effect,
                  group: engineLabel(driver.categoryLabel),
                  run: () => _addDriver(driver, at, wire),
                ),
            // Then every effect (K-673): chosen, it joins the layer's stack,
            // and the stack is the chain — so the box appears wired into the
            // picture's own path, K-445's auto-wire by construction. Only
            // with no wire in hand: a dragged wire is a value looking for a
            // socket, and the chain's sockets take no wire.
            if (wire == null)
              for (final effect in listEffects())
                if (!driverNames.contains(effect.name))
                  FxConsoleEntry(
                    label: engineLabel(effect.label),
                    kind: FxConsoleKind.effect,
                    group: engineLabel(effect.categoryLabel),
                    run: () => _addEffect(effect),
                  ),
            // The saved groups, beside the drivers they are made of (K-651).
            // Only with no wire in hand: a group is a rig, not a socket, so
            // there is nothing for the wire to land on.
            if (wire == null)
              for (final saved in (widget.groupsLister ?? listNodeGroups)())
                FxConsoleEntry(
                  label: saved.name,
                  kind: FxConsoleKind.effect,
                  group: l10n.graphGroup,
                  run: () => _insertGroup(saved.path, at),
                ),
          ],
        ),
      );
    } finally {
      if (mounted) setState(() => _searching = false);
    }
  }

  /// The stack's own add, which is the graph's add for an effect: the chain
  /// *is* the list, so the new box appears at the chain's end with the
  /// picture's wires already on it, and the op is the one the Effect menu
  /// commits.
  void _addEffect(BridgeEffectInfo info) {
    final layer = _layer;
    if (layer == null) return;
    try {
      layer.addEffect(name: info.name);
    } catch (_) {
      return;
    }
    _ui?.model.refresh();
    _reload();
  }

  // --- Named groups (K-651) -----------------------------------------------

  /// The picked driver boxes — what a group can hold. The Source, the Layer
  /// out and the stack's effects are derived from the layer, so a group naming
  /// one could only ever be dropped back onto a layer with the same stack.
  List<BridgeNodeRef> get _groupable => [
        for (final node in _selection.values)
          if (node is BridgeNodeRef_Driver) node,
      ];

  /// Name the picked boxes and write them to the library — **one gesture, two
  /// halves** (K-651). Naming a set is what draws its wash, and the same name
  /// is what the file is called, exactly as an effect preset takes its display
  /// name from the file it was saved as.
  Future<void> _saveGroup() async {
    final layer = _layer;
    final graph = _graph;
    final members = _groupable;
    if (layer == null || graph == null || members.isEmpty) return;

    final picker = widget.groupSavePicker;
    final path = picker != null
        ? await picker()
        : await pickNodeGroupSaveLocation('group.lumgrp',
            initialDirectory: presetsDirPath());
    if (path == null || !mounted) return;
    final name = path
        .split(RegExp(r'[/\\]'))
        .last
        .replaceAll('.$graphGroupExtension', '');
    // The next chip along, skipping index 0 — that one is the palette's quiet
    // default, and a wash that reads as "no colour" is not a region.
    final colour = graph.wiring.groups.length + 1;

    _commit(_wiringNow(groups: [
      ...graph.wiring.groups,
      BridgeNodeGroup(name: name, colour: colour, members: members),
    ]));
    // Written from the *committed* graph, so the file and the wash carry the
    // same name, colour and members however the commit was healed.
    try {
      File(path).writeAsStringSync(
          layer.saveNodeGroup(name: name, colour: colour, nodes: members));
    } catch (_) {
      // The library is the user's folder; a failed write leaves the group on
      // the canvas, which is the half that did land.
    }
  }

  /// Drop a saved group at `at` — one commit, so one undo step however many
  /// boxes and wires it carries.
  void _insertGroup(String path, Offset at) {
    final layer = _layer;
    if (layer == null) return;
    final String text;
    try {
      text = File(path).readAsStringSync();
    } catch (_) {
      return;
    }
    try {
      layer.insertNodeGroup(text: text, x: at.dx, y: at.dy);
    } catch (_) {
      return;
    }
    _ui?.model.refresh();
    _reload();
  }

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
  void _addDriver(BridgeEffectInfo info, Offset at, _Socket? wire) {
    final layer = _layer;
    if (layer == null || _graph == null) return;

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
          groups: _graph!.wiring.groups,
        ),
      );
    } catch (_) {
      return;
    }
    _ui?.model.refresh();
    _reload();
    setState(() => _pick([ref]));
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

  // --- Dropping a box into a wire (N7) ------------------------------------

  /// The wire the box being dragged would fall into, and the two sockets of
  /// its own that would take its ends — null unless a single box carrying no
  /// wires at all is sitting over one it can carry.
  ///
  /// The wire splits: what fed the consumer now feeds this box, and this box
  /// feeds the consumer. Standard node-editor behaviour, and one `setGraph`, so
  /// one undo step like every other gesture here.
  ({BridgeGraphEdge edge, _Socket into, _Socket outOf})? _dropInsert(
      _Layout layout) {
    final drag = _nodeDrag;
    final graph = _graph;
    // One box, and only a box nothing is joined to: dropping a wired node on a
    // wire would ask what happens to the wires it already has, and the answer
    // "nothing" would be a surprise either way.
    if (drag == null || graph == null || drag.origins.length != 1) return null;
    final box = layout.byKey[drag.key];
    if (box == null) return null;
    if (graph.wiring.edges.any((e) => _touches(e, box.node.node))) return null;

    // Where the box is *now*, which the held layout is a frame behind on.
    final at = (_positions[drag.key] ?? box.rect.topLeft) +
        Offset(box.rect.width / 2, box.rect.height / 2);

    for (final edge in graph.wiring.edges) {
      final ends = _edgeEnds(layout, edge);
      if (ends == null) continue;
      if (_wireDistance(ends.$1, ends.$2, at) > _wireGrab) continue;
      final into = _freeSocket(box, ends.$3, isInput: true);
      final outOf = _freeSocket(box, ends.$3, isInput: false);
      // Both types match the wire's by construction and the box has nothing
      // joined to it, so neither half can mismatch or close a loop; the
      // engine's own refusal stays the backstop.
      if (into != null && outOf != null) {
        return (edge: edge, into: into, outOf: outOf);
      }
    }
    return null;
  }

  /// This box's first socket of [type] on the given side that could take the
  /// wire — the picture's own path excluded, since that is the effect list's
  /// and not this gesture's.
  _Socket? _freeSocket(_Box box, BridgePortType type, {required bool isInput}) {
    for (final port in isInput ? box.inputs : box.outputs) {
      if (_isChainType(port.portType) || port.portType != type) continue;
      if (isInput && port.wired) continue;
      final at = box.socket(port.id, isInput);
      if (at != null) return _Socket(box.node.node, port, isInput, at);
    }
    return null;
  }

  // --- Pointer work -------------------------------------------------------

  Offset _toCanvas(Offset local) => (local - _pan) / _zoom;

  void _down(PointerDownEvent event, _Layout layout) {
    _canvasFocus.requestFocus();
    if (_claimed) {
      _claimed = false;
      return;
    }
    // A press while the console is up never reaches here: it floats over the
    // canvas in the overlay and its own click-away layer catches the click
    // that means "never mind".
    final at = _toCanvas(event.localPosition);
    _pressAt = event.localPosition;

    final socket = layout.socketAt(at);
    if (socket != null && _isChainType(socket.port.portType)) {
      // **The image chain's wires can be picked up too** (K-674, owner item
      // 10: "connections between effect boxes can't be removed"). Pressing a
      // chain *input* takes hold of the wire feeding it, by its far end,
      // exactly as a stored wire is grabbed — drop it on another chain input
      // to re-route (a stack reorder), or on empty canvas to take the fed box
      // out of the chain. A chain *output* still takes no drag: one output
      // feeding one input is the list's own law, and a fresh chain wire is
      // not a thing that can exist.
      if (socket.isInput) {
        final chain = layout.chain;
        final i = chain.indexWhere(
            (b) => graphNodeKey(b.node.node) == graphNodeKey(socket.node));
        if (i >= 1) {
          final upstream = chain[i - 1];
          final portId = i - 1 == 0 ? 'image' : 'output';
          final k = upstream.outputs.indexWhere((p) => p.id == portId);
          final from = upstream.socket(portId, false);
          if (k >= 0 && from != null) {
            setState(() => _flight = _InFlight(
                _Socket(upstream.node.node, upstream.outputs[k], false, from),
                at,
                chain: i));
            return;
          }
        }
      }
      // Fall through: a press on a chain output lands on the box beneath.
    } else if (socket != null) {
      // **A wire that is already there is grabbed by its far end** (owner, desk
      // test: "connections that already exist can't be un-done"). Pressing a
      // wired input used to start a *second* wire from that input, which no
      // drop could accept — an input takes one wire — so an existing wire could
      // only be taken off by clicking its socket dead on. Now the press picks
      // the wire up: drop it on another input to move it, or on nothing at all
      // to take it off. Pressing an **output** still draws a new wire, which is
      // what lets one output feed any number of inputs.
      final held = socket.isInput ? _edgeInto(socket) : null;
      final grabbed = held == null ? null : _sourceSocket(held, layout);
      setState(() => _flight = grabbed == null
          ? _InFlight(socket, at)
          : _InFlight(grabbed, at, detached: held));
      return;
    }

    final box = layout.boxAt(at);
    if (box != null) {
      final key = graphNodeKey(box.node.node);
      // **Double-clicking a Custom shader box enters its inner graph** (K-642
      // — "entering a shader node works like entering a precomp"). Counted
      // with [DoubleTap] because the canvas reads raw pointers; the first
      // press still picks the box, exactly as a precomp's first click selects.
      final again = _boxTaps.tap(at: event.localPosition, slop: 6) &&
          _boxTapKey == key;
      _boxTapKey = key;
      if (again && box.node.matchName == 'custom_shader') {
        if ((_effectIdOf(box.node.node), _layer) case (final effect?, final layer?)) {
          _ui?.enterShaderGraph(layer, effect,
              effectName: box.node.customName ?? engineLabel(box.node.label));
          return;
        }
      }
      final keys = HardwareKeyboard.instance;
      final toggle = keys.isControlPressed || keys.isMetaPressed;
      final add = keys.isShiftPressed;
      final held = _isPicked(box.node.node);
      setState(() {
        // **Click replaces, Ctrl toggles, Shift adds** — the three rules a
        // layer row, a project row and an effect heading all follow, because a
        // selection that behaved one way here and another there would be two
        // selections to learn.
        if (toggle) {
          if (_selection.remove(key) == null) {
            _selection[key] = box.node.node;
          }
          _publishPick();
        } else if (add) {
          _selection[key] = box.node.node;
          _publishPick();
        } else if (!held) {
          _pick([box.node.node]);
        }
        // A press inside something already picked takes the whole pick with
        // it, so several boxes move together (docs/07 §4.5). A release that
        // never moved collapses it back to this box, which is what the plain
        // click above would have done.
        final moving = held && !toggle && !add
            ? _selection.keys
            : <String>[if (_selection.containsKey(key)) key];
        _nodeDrag = _NodeDrag(
          key,
          at,
          {
            for (final k in moving)
              k: _positions[k] ?? layout.byKey[k]?.rect.topLeft ?? Offset.zero,
          },
          collapse: held && !toggle && !add,
        );
      });
      return;
    }

    // **Empty canvas.** The primary button sweeps a selection box; the middle
    // button pans, which is where panning went when the box took the drag it
    // used to have (K-533). A press with no modifier clears, exactly as it always did;
    // an additive one keeps what is picked and adds the catch to it.
    if (event.buttons == kMiddleMouseButton) {
      setState(() => _panFrom = _pan - event.localPosition);
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

  void _move(PointerMoveEvent event, _Layout layout) {
    final at = _toCanvas(event.localPosition);
    if (_flight case final flight?) {
      setState(() => flight.to = at);
      return;
    }
    if (_nodeDrag case final drag?) {
      setState(() {
        var delta = at - drag.grab;
        // The magnet: the grabbed box lands on the dot grid's pitch, and the
        // rest of the pick rides the same snapped delta, so a group keeps its
        // internal spacing while its anchor lines up.
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

  void _up(PointerUpEvent event, _Layout layout) {
    final at = _toCanvas(event.localPosition);
    final moved = _pressAt == null ||
        (event.localPosition - _pressAt!).distance > _dragSlop;

    if (_flight case final flight?) {
      setState(() => _flight = null);
      final landed = layout.socketAt(at);
      if (flight.chain case final held?) {
        // A press that never travelled is no gesture at all here: a stored
        // wire unplugs on a stationary click, but a chain wire's discard
        // costs an effect, and that must never be the price of a slip.
        if (moved) _chainDrop(held, landed, layout);
        return;
      }
      if (flight.detached case final held?) {
        // The wire in hand came off an input. Dropped on another socket that
        // will take it, it moves there; dropped on anything else — empty
        // canvas, a socket that refuses it, or the very socket it came off,
        // which is the press-and-release that always unplugged — it goes.
        if (moved && landed != null && _accepts(flight.from, landed)) {
          _connect(flight.from, landed, without: held);
        } else {
          _removeEdge(held);
        }
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

    if (_nodeDrag case final drag?) {
      // Asked before the drag is let go of, since it is what the answer is
      // worked out from.
      final insert = moved ? _dropInsert(layout) : null;
      setState(() {
        _nodeDrag = null;
        _dropWire = null;
        // The press that kept a standing pick, released without moving: that
        // is a plain click, and a plain click replaces.
        if (!moved && drag.collapse) {
          final node = _selection[drag.key];
          if (node != null) _pick([node]);
        }
      });
      if (insert != null) {
        final source = _outputRef(insert.outOf);
        final dest = _inputRef(insert.into);
        if (source != null && dest != null) {
          // The wire splits, and the box's new position rides the same write.
          _commit(_wiringNow(edges: [
            for (final e in _graph!.wiring.edges)
              if (e != insert.edge) e,
            BridgeGraphEdge(from: insert.edge.from, to: dest),
            BridgeGraphEdge(from: source, to: insert.edge.to),
          ]));
          return;
        }
      }
      if (moved && _graph != null) _commit(_wiringNow());
      return;
    }

    if (_marqueeFrom case final from?) {
      final to = _marqueeTo;
      final adds = _marqueeAdds;
      setState(() {
        _marqueeFrom = null;
        _marqueeTo = null;
        if (to == null) return;
        // **Wholly inside**, the house rule for every rubber band here
        // (docs/07 §4.5): a box the band clipped a corner off was not what the
        // gesture pointed at.
        final band = Rect.fromPoints(_toCanvas(from), _toCanvas(to));
        final caught = [
          for (final box in layout.boxes)
            if (band.contains(box.rect.topLeft) &&
                band.contains(box.rect.bottomRight))
              box.node.node,
        ];
        _pick([if (adds) ..._selection.values, ...caught]);
      });
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
    // Inside a Custom shader (K-642 §4.2): the panel's whole face is the
    // inner graph, breadcrumb and all, until Escape or a crumb brings the
    // layer's own graph back.
    if (_ui?.shaderGraphEntry.value case final entry?) {
      return ShaderGraphPanel(
        entry: entry,
        onExit: () => _ui?.exitShaderGraph(),
      );
    }
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
            // Named regions of the canvas: what is picked becomes a group with
            // a wash of its own, and the same act writes it to the library
            // (K-651). Greyed until at least one driver box is picked, since a
            // group of derived boxes is a group nothing could re-insert.
            HouseButton(
              key: const ValueKey('graph-save-group'),
              small: true,
              frameless: true,
              onPressed: _groupable.isEmpty ? null : _saveGroup,
              child: Text(l10n.graphSaveGroup, style: t.small),
            ),
            const SizedBox(width: 10),
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
            // The snap magnet, beside the view controls where the board draws
            // it: on reads as the glyph at foreground strength on the button's
            // own face, off frameless and muted — the Timeline's own magnet,
            // in the Timeline's own dress (no accent; §3.1's list is closed).
            LumitTooltip(
              message: _snapToGrid ? l10n.tipSnapOn : l10n.tipSnapOff,
              child: HouseButton(
                key: const ValueKey('graph-snap'),
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
        // No Tab door (K-673): Ctrl+Space is the console's one key, answered
        // through [_consoleClaim] so it works with focus anywhere in the app.
        if (event.logicalKey == LogicalKeyboardKey.delete ||
            event.logicalKey == LogicalKeyboardKey.backspace) {
          _deleteSelected();
          return KeyEventResult.handled;
        }
        return KeyEventResult.ignored;
      },
      // The search floats in the overlay rather than here (K-645): the canvas
      // takes every pointer that lands on it — that is how a socket is grabbed
      // without a gesture detector per socket — so a popover drawn inside it
      // would have its presses read as presses on the ground behind it.
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
                    // The ground is its own layer: it changes with the pan and
                    // the zoom and with nothing else, so the wires above it may
                    // redraw on every pointer move without it.
                    Positioned.fill(
                      child: RepaintBoundary(
                        child: CustomPaint(
                          key: const ValueKey('graph-ground'),
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
                        painter: _GraphPainter(
                          layout: layout,
                          edges: _graph!.wiring.edges,
                          flight: _flight,
                          dropWire: _dropWire,
                          pan: _pan,
                          zoom: _zoom,
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
                            // The named regions, under every box and taking no
                            // pointer: a wash is something to read, never
                            // something to press (K-651).
                            for (final group in _graph!.wiring.groups)
                              if (graphGroupRect([
                                for (final member in group.members)
                                  if (layout.byKey[graphNodeKey(member)]
                                      case final box?)
                                    box.rect,
                              ])
                                  case final rect?)
                                _groupWash(t, group, rect),
                            for (final box in layout.boxes)
                              Positioned(
                                left: box.rect.left,
                                top: box.rect.top,
                                child: _NodeCard(
                                  box: box,
                                  title: box.node.node is BridgeNodeRef_Source
                                      ? _layerName
                                      : engineLabel(box.node.label),
                                  selected: _isPicked(box.node.node),
                                  exposed: _graph!.wiring.exposed.any((e) =>
                                      graphNodeKey(e) ==
                                      graphNodeKey(box.node.node)),
                                  onOwnPress: () => _claimed = true,
                                  onExpose: () => _toggleExposed(box.node.node),
                                  onBypass: () => _toggleBypass(box.node),
                                  renaming: _renamingNode ==
                                      graphNodeKey(box.node.node),
                                  onStartRename: () => setState(() =>
                                      _renamingNode =
                                          graphNodeKey(box.node.node)),
                                  onRenamed: (name) =>
                                      _renameNode(box.node, name),
                                  // Escape: shut the editor, rename nothing
                                  // (K-323).
                                  onRenameCancelled: () =>
                                      setState(() => _renamingNode = null),
                                ),
                              ),
                          ],
                        ),
                      ),
                    ),
                    // Over the boxes and outside the pan/zoom transform: the
                    // band is drawn where the pointer is, not where the canvas
                    // is. The face is the application's one selection box
                    // ([MarqueeBox]).
                    if (_marqueeFrom != null && _marqueeTo != null)
                      Positioned.fromRect(
                        key: const ValueKey('graph-marquee'),
                        rect: Rect.fromPoints(_marqueeFrom!, _marqueeTo!),
                        child: const MarqueeBox(),
                      ),
                    _legend(t),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }

  /// One group's tinted wash and its name. The colour is a **label chip**
  /// (K-188's palette, indexed) — the graph's own colour coding is the port
  /// types and nothing else, so a region borrows the application's other
  /// palette rather than inventing a third.
  Widget _groupWash(LumitTheme t, BridgeNodeGroup group, Rect rect) {
    final colour = t.labelColour(group.colour.toInt());
    return Positioned.fromRect(
      key: ValueKey<String>('graph-group-${group.name}'),
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
}

/// One box on the canvas: a header strip carrying its enable tick, its twirl
/// and its name, then the port rows with their sockets sitting on the border.
class _NodeCard extends StatelessWidget {
  final _Box box;
  final String title;
  final bool selected;
  final bool exposed;
  final VoidCallback onExpose;
  final VoidCallback onBypass;

  /// A control on this card has taken the press, so the canvas behind it
  /// leaves the pick alone.
  final VoidCallback onOwnPress;

  /// The name is an inline editor rather than a label (K-321), its commit —
  /// empty clears back to the box's own label — and the Escape that throws the
  /// edit away instead (K-323). The same contract the Effect controls heading
  /// has, because it is the same rename.
  final bool renaming;
  final VoidCallback onStartRename;
  final ValueChanged<String> onRenamed;
  final VoidCallback onRenameCancelled;

  const _NodeCard({
    required this.box,
    required this.title,
    required this.selected,
    required this.exposed,
    required this.onExpose,
    required this.onBypass,
    required this.onOwnPress,
    required this.renaming,
    required this.onStartRename,
    required this.onRenamed,
    required this.onRenameCancelled,
  });

  bool get _derived =>
      box.node.node is BridgeNodeRef_Source ||
      box.node.node is BridgeNodeRef_Out;

  /// Whether this box has anything to twirl open. A **driver** draws every
  /// socket it has whatever its exposure says — the box is small and its
  /// ports are the whole of what it is for — so a twirl on one would be a
  /// control that answers nothing, and it is left off.
  bool get _foldable => !_derived && box.node.node is! BridgeNodeRef_Driver;

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
            child: GraphNodeFrame(
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
          // A Custom shader box wears the viz tint (K-675) so the one box
          // with an inside reads at a glance.
          color: box.node.matchName == 'custom_shader'
              ? graphShaderHeader(t)
              : t.surface2,
          border: Border(bottom: BorderSide(color: t.hairline)),
        ),
        child: Row(
          children: [
            // **Enable tick, twirl, name** — the order an Effect controls
            // heading reads in (K-443), because on a node card it is the same
            // grammar and, for the tick, the same control. What the box *is
            // doing* comes before what the header does to what is under it.
            if (!_derived) ...[
              _enable(t),
              const SizedBox(width: 2),
            ],
            if (_foldable) ...[
              _twirl(t),
              const SizedBox(width: 2),
            ],
            // **The name takes every pixel the marks leave** (owner, desk
            // test). It used to be a `Flexible` name beside a `Spacer`, and a
            // `Spacer` is an `Expanded` of flex 1: the two shared the header's
            // free space half each, so a name ellipsised with half the strip
            // standing empty beside it. One `Expanded` holding the names puts
            // the whole remainder at their disposal — the controls are
            // fixed-width — so a name cuts only when it is genuinely out of
            // room.
            Expanded(child: renaming ? _editor(t) : _names(t)),
            // **Nothing wired in** (K-509). A driver that reads a stream and
            // has none answers its documented no-op — a distance so large it
            // pins whatever it drives at the far end of the range — and the
            // box is the one place that can say so before the wire is drawn.
            if (graphNoStream(box.node)) ...[
              const SizedBox(width: 4),
              LumitTooltip(
                message: l10n.graphNoStream,
                child: Container(
                  key: ValueKey<String>(
                      'graph-no-stream-${graphNodeKey(box.node.node)}'),
                  width: graphBadgeSize,
                  height: graphBadgeSize,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                    border: Border.all(color: t.warning),
                  ),
                  child: Text('!',
                      style: t.mono.copyWith(fontSize: 8, color: t.warning)),
                ),
              ),
            ],
          ],
        ),
      );

  /// The box's type name and, where the user has given it one, their own name
  /// beside it. **Double-click either to rename the box** (owner, desk test):
  /// the derived boxes are left out, since the Source shows the layer's name
  /// and the Out is the layer's own end — neither is a thing with a name of
  /// its own to give.
  Widget _names(LumitTheme t) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onDoubleTap: _derived ? null : onStartRename,
        child: Row(
          children: [
            Flexible(
              child: Text(
                title,
                key: ValueKey<String>(
                    'graph-node-name-${graphNodeKey(box.node.node)}'),
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
          ],
        ),
      );

  /// The inline rename, opened with the current name selected and committing
  /// on Enter or on clicking away — the contract every inline rename in the
  /// application has (K-243), and the one the Effect controls heading's own
  /// editor keeps.
  Widget _editor(LumitTheme t) => _NodeNameField(
        key: ValueKey<String>(
            'graph-node-rename-${graphNodeKey(box.node.node)}'),
        initial: box.node.customName ?? '',
        onDone: onRenamed,
        onCancel: onRenameCancelled,
      );

  /// **Pressing a control on the header is not picking the box** (the
  /// Timeline's rule for its switch cells, K-452). The canvas reads its
  /// pointers through one `Listener` above this card, and a child listener is
  /// dispatched first — so the flag is set before the canvas decides what the
  /// press meant. Without it, switching one of four picked boxes off collapsed
  /// the pick to that box and then bypassed only it.
  Widget _claim(Widget child) =>
      Listener(onPointerDown: (_) => onOwnPress(), child: child);

  /// **The enable tick**, left of the name — the Effect controls heading's own
  /// switch face ([fxEnableMark]), because a box's enable and its effect
  /// card's switch are one control drawn on two surfaces. Off, the box's
  /// border goes dashed and its name goes quiet (15-DESIGN §12A.7): a bypassed
  /// box is not a fault, it is the one box deliberately doing nothing.
  Widget _enable(LumitTheme t) => _claim(SizedBox(
        width: graphEnableSize,
        height: graphEnableSize,
        child: Center(
          child: fxEnableMark(
            key:
                ValueKey<String>('graph-enable-${graphNodeKey(box.node.node)}'),
            on: box.node.enabled,
            onChanged: (_) => onBypass(),
          ),
        ),
      ));

  /// **The twirl**, which opens the box up. Shut, an effect draws the
  /// picture's own sockets and whatever is already wired; open, it draws a
  /// socket for every parameter it has (`LayerGraph::exposed`). The same mark
  /// and the same reading as the twirl on an Effect controls heading — what is
  /// under the header, shown or folded away.
  Widget _twirl(LumitTheme t) => _claim(GestureDetector(
        key: ValueKey<String>('graph-twirl-${graphNodeKey(box.node.node)}'),
        behavior: HitTestBehavior.opaque,
        onTap: onExpose,
        child: SizedBox(
          width: graphBadgeSize,
          height: graphBadgeSize,
          child: Center(
            child: lumitIcon(
              exposed ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
              size: graphTwirlSize,
              color: exposed ? t.textPrimary : t.textMuted,
            ),
          ),
        ),
      ));

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
/// Public because the inner shader graph's cards wear the identical frame
/// (K-642 §4.2 — the canvas is shared, the model is not).
class GraphNodeFrame extends StatelessWidget {
  final Color colour;
  final Color fill;
  final bool dashed;
  final double radius;
  const GraphNodeFrame({
    super.key,
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
    canvas.drawPath(graphDashPath(Path()..addRRect(rect)), stroke);
  }

  @override
  bool shouldRepaint(_FramePainter old) =>
      old.colour != colour ||
      old.fill != fill ||
      old.dashed != dashed ||
      old.radius != radius;
}

/// A path as 3-on, 3-off dashes — the drawing's own stroke-dasharray. Public
/// for the reason [GraphNodeFrame] is: the inner shader graph draws its wire
/// in hand with the same stroke.
Path graphDashPath(Path path) {
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

/// The canvas ground: `surface_0` under its dot grid (docs/15 §12).
///
/// **In plain terms.** The dots are drawn where the canvas's own grid lines
/// cross, so zooming out used to bring more of them into view — at the smallest
/// zoom, eleven times as many as at 100%, each one its own little circle, which
/// is the whole of the panel's zoomed-out lag. Two things fix it, and neither
/// changes what the ground looks like at 100%:
///
/// * The grid **thins as it shrinks**. Whenever the zoom would push the dots
///   closer together on screen than the pitch they are drawn at, the grid skips
///   every other line — twice the canvas spacing, the same spacing on screen —
///   so the number of dots on screen stays put however far out the canvas goes.
/// * They are drawn in **one call**, as points, rather than one circle apiece.
///
/// It is also its own painter behind its own [RepaintBoundary], because it
/// answers to nothing but the pan, the zoom and the theme: hovering a node or
/// dragging a wire redraws every wire above it and leaves the ground alone.
class GraphGroundPainter extends CustomPainter {
  final Offset pan;
  final double zoom;
  final Color grid;
  final Color ground;

  const GraphGroundPainter({
    required this.pan,
    required this.zoom,
    required this.grid,
    required this.ground,
  });

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = ground);
    var pitch = graphDotGrid * zoom;
    if (pitch <= 0) return;
    while (pitch < graphDotGrid) {
      pitch *= 2;
    }
    // `%` on a positive divisor is never negative, so the first dot of each run
    // sits in [0, pitch) and the counts below are exactly the dots on screen.
    final startX = pan.dx % pitch;
    final startY = pan.dy % pitch;
    final columns = math.max(0, ((size.width - startX) / pitch).ceil());
    final rows = math.max(0, ((size.height - startY) / pitch).ceil());
    if (columns == 0 || rows == 0) return;
    final points = Float32List(columns * rows * 2);
    var i = 0;
    for (var c = 0; c < columns; c++) {
      final x = startX + c * pitch;
      for (var r = 0; r < rows; r++) {
        points[i++] = x;
        points[i++] = startY + r * pitch;
      }
    }
    canvas.drawRawPoints(
      PointMode.points,
      points,
      Paint()
        ..color = grid
        ..strokeCap = StrokeCap.round
        ..strokeWidth = 2,
    );
  }

  @override
  bool shouldRepaint(GraphGroundPainter old) =>
      old.pan != pan ||
      old.zoom != zoom ||
      old.grid != grid ||
      old.ground != ground;
}

/// Every wire on the canvas.
class _GraphPainter extends CustomPainter {
  final _Layout layout;
  final List<BridgeGraphEdge> edges;
  final _InFlight? flight;

  /// The wire a box is being dragged over and would drop into (N7).
  final BridgeGraphEdge? dropWire;
  final Offset pan;
  final double zoom;
  final Color dragged;
  final LumitTheme theme;

  const _GraphPainter({
    required this.layout,
    required this.edges,
    required this.flight,
    required this.dropWire,
    required this.pan,
    required this.zoom,
    required this.dragged,
    required this.theme,
  });

  Offset _screen(Offset canvas) => canvas * zoom + pan;

  @override
  void paint(Canvas canvas, Size size) {
    // The image chain's wires are not stored anywhere: they *are* the effect
    // list, read left to right. Drawing them from the box order is what makes
    // the stack view impossible to contradict.
    final chain = layout.chain;
    for (var i = 0; i + 1 < chain.length; i++) {
      // The chain wire in the hand (K-674) is the dashed flight, not a drawn
      // segment: the wire has visibly left its input.
      if (flight?.chain == i + 1) continue;
      final from = chain[i].socket(i == 0 ? 'image' : 'output', false);
      final to = chain[i + 1]
          .socket(i + 1 == chain.length - 1 ? 'image' : 'input', true);
      if (from != null && to != null) {
        _wire(canvas, from, to, theme.port.image, dashes: false);
      }
    }

    for (final edge in edges) {
      final ends = _edgeEnds(layout, edge);
      if (ends == null) continue;
      // The wire a box is being dropped into wears the canvas's own picked
      // colour while it is under one (N7, K-473) — the same mark a picked box
      // wears, because both say "this is what the release will act on".
      final into = edge == dropWire;
      _wire(canvas, ends.$1, ends.$2,
          into ? theme.animated : portColour(theme, ends.$3),
          dashes: false, weight: into ? 2 : 1);
    }

    if (flight case final f?) {
      _wire(canvas, f.from.at, f.to, dragged, dashes: true);
    }
  }

  /// One wire: [graphWirePath]'s cubic, which is the curve both drawings draw
  /// — with the minimum stub that keeps it visible when the consumer sits left
  /// of its producer.
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
  bool shouldRepaint(_GraphPainter old) => true;
}

/// A node card's inline rename field: the box's current custom name, selected
/// on open because a name is retyped far more often than amended, committed on
/// Enter or on clicking away (K-243) and thrown away on Escape (K-323).
///
/// Empty is a real answer — it clears the custom name and the card goes back to
/// showing the box's own label — so an empty field is committed like any other.
class _NodeNameField extends StatefulWidget {
  final String initial;
  final ValueChanged<String> onDone;
  final VoidCallback onCancel;

  const _NodeNameField({
    super.key,
    required this.initial,
    required this.onDone,
    required this.onCancel,
  });

  @override
  State<_NodeNameField> createState() => _NodeNameFieldState();
}

class _NodeNameFieldState extends State<_NodeNameField> {
  late final TextEditingController _controller = TextEditingController(
    text: widget.initial,
  )..selection = TextSelection(
      baseOffset: 0,
      extentOffset: widget.initial.length,
    );

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => HouseTextField(
        controller: _controller,
        width: double.infinity,
        autofocus: true,
        submitOnLostFocus: true,
        onSubmitted: widget.onDone,
        onCancelled: widget.onCancel,
      );
}
