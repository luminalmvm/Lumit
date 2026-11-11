// The Graph panel — a layer's effect stack drawn as nodes and wires, plus the
// drivers wired into its parameters.
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
// **One read, never in a rebuild**. `getGraph` is asked when the
// selection or the document changes and held here; `bridge_call_budget_test`
// expects a hover over this canvas to cost nothing at all. Everything the
// canvas draws and every hit test it makes is arithmetic over that held copy.
//
// **Colour is the legend**. A wire and its sockets take the port's type
// colour from `theme.port`; no other colour coding appears on the canvas, and
// no colour crosses the bridge — the engine sends the type, the frontend maps
// it to the token.

import 'dart:io' show File;
import 'dart:math' as math;
import 'dart:typed_data' show Float32List;
import 'dart:ui' show PointMode;

import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart'
    show BoxHitTestEntry, BoxHitTestResult, RenderStack;
import 'package:flutter/services.dart'
    show HardwareKeyboard, KeyDownEvent, KeyEvent, LogicalKeyboardKey;
import 'package:flutter/rendering.dart' show RenderOffstage;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/graph_looks.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart' show BridgeKeyContext;
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/track.dart' show fireEffectAction;
import 'package:lumit_flutter/src/rust/api/wire_tie.dart' show BridgeWireTie;
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart' show LumitIcon;
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/fx_console_frb.dart';
import '../state/comp_time.dart' show timeOfFrame;
import '../state/dock.dart';
import '../state/drag_payloads.dart';
import '../state/external_links.dart' show effectManualUrl, openExternalLink;
import 'timeline_outline_row_frb.dart' show showLayerRowMenu;
import '../state/preview_throttle.dart';
import '../state/settings.dart' show InterfaceSettings;
import '../state/timeline_columns.dart' show ValueColumn;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/marquee.dart';
import 'comp_graph_panel.dart' show CompBoxRows, CompGraphPanel, compBoxRows;
import 'effect_commands.dart' show EffectCommands;
import 'effect_param_row_frb.dart'
    show
        EffectParamRowFrb,
        EffectStackEditor,
        instanceParameters,
        relistedRows,
        withChained;
import 'flare_window.dart' show pressFlareButton;
import 'lens_window.dart' show pressLensButton;
import 'graph_card.dart';
import 'graph_cut_fade.dart';
import 'graph_groups.dart';
import 'node_menus.dart';
import 'graph_ties.dart';
import '../state/clipboard.dart' show ClipboardKind;
import 'fx_section.dart' show fxEnableMarkScale;
import 'control_rows.dart' show paramControlRow;
import 'layer_fold_frb.dart' show effectPath;
import '../state/share.dart';
import 'placeholder.dart';
import 'shader_editor.dart'
    show InstanceHome, editExpressionOn, pressShaderButton;
import 'shader_graph.dart' show ShaderGraphPanel, ShaderGraphThumb;
import 'timeline_extras_frb.dart' show DoubleTap;
import 'viewer_tool_cursor.dart' show handPanLayer;

// The boxes' own layout and drawing live in graph_card.dart; the other
// canvases reach them through this library, as they always have.
export 'graph_card.dart';

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

/// The enable tick's cell. The Effect controls heading draws the 14px
/// checkbox [fxEnableMarkScale] larger, and the node header wears that same
/// mark at that same size, so the cell that holds it is that much larger than
/// the marks beside it — the tick is one control on two surfaces, not two
/// drawings of one.
const double graphEnableSize = graphBadgeSize * fxEnableMarkScale;

/// The twirl glyph on a node header, smaller than the house default because
/// the header is 21px rather than the 24 an Effect controls heading has
/// (the default is a default, not a law).
const double graphTwirlSize = 12;

/// A node graph box twirled open draws a control on every parameter row, so
/// it is wider than a shut one: the stopwatch, the name and the control column
/// the Node panel gives a row, inside the row inset on both sides.
const double graphNodeOpenWidth = 300;

/// A parameter row with a control in it: the 20px well with 2px of air either
/// side, on the four-pixel module. A socket-only row keeps [graphPortRowHeight].
const double graphControlRowHeight = 24;

/// The control column on an open box's row. Wide enough for every control the
/// row widget draws, the Range sliders setting's well and track included.
const double graphControlColumn = 150;

/// What a row keeps clear of the card's edge on either side, where the socket
/// sits.
const double graphRowInset = 12;

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
/// The handles run horizontally out of each socket — right out of the output,
/// left into the input. With the input to the right it is one cubic, reaching
/// half the gap or [graphWireStub], whichever is longer. With the input to the
/// left it is two cubics that meet at the midpoint. Each end hooks round
/// within the stub and turns towards the other, and the two cross the middle
/// on the line between their hooks. So a box just left of and well below the
/// one feeding it gets a wire that runs on down, not one that jogs sideways
/// halfway. The stub is never longer than half the wire, so a short wire stays
/// a short curve. Free of the painter so its geometry can be asserted directly.
///
/// [WireStyle.hanging] is a slack cable instead, which sags a quarter of its
/// own length below the straight line between the two sockets.
/// [WireStyle.angled] runs straight and turns only at 45 or 90 degrees, each
/// turn rounded off.
Path graphWirePath(Offset a, Offset b,
    {double zoom = 1, WireStyle wires = WireStyle.curved}) {
  final dx = b.dx - a.dx;
  final stub = math.min(graphWireStub * zoom, (b - a).distance / 2);
  final path = Path()..moveTo(a.dx, a.dy);
  if (wires == WireStyle.hanging) {
    final mid = (a + b) / 2;
    // The control point sits twice as low as the curve dips.
    return path
      ..quadraticBezierTo(mid.dx, mid.dy + (b - a).distance / 2, b.dx, b.dy);
  }
  if (wires == WireStyle.angled) return _angledWire(path, a, b, stub);
  if (dx >= 0) {
    final reach = math.max(stub, dx / 2);
    return path..cubicTo(a.dx + reach, a.dy, b.dx - reach, b.dy, b.dx, b.dy);
  }
  // How far each hook drops before it turns for the other end.
  final drop = ((b.dy - a.dy) / 2).clamp(-stub, stub);
  final mid = (a + b) / 2;
  return path
    ..cubicTo(a.dx + stub, a.dy, a.dx + stub, a.dy + drop, mid.dx, mid.dy)
    ..cubicTo(b.dx - stub, b.dy - drop, b.dx - stub, b.dy, b.dx, b.dy);
}

/// [graphWirePath]'s angled wire. Going backwards it runs the same course
/// turned on its side, between the ends of the two stubs, so it drops out of
/// one and into the other. Every corner is rounded by up to a quarter stub,
/// less where a leg is too short to give it.
Path _angledWire(Path path, Offset a, Offset b, double stub) {
  Offset flip(Offset p) => Offset(p.dy, p.dx);
  final corners = b.dx >= a.dx
      ? _angledRun(a, b, stub / 2)
      : [
          a,
          ..._angledRun(flip(a + Offset(stub, 0)), flip(b - Offset(stub, 0)),
                  stub / 2)
              .map(flip),
          b,
        ];
  final points = <Offset>[];
  for (final c in corners) {
    if (points.isEmpty || (c - points.last).distance > 0.01) points.add(c);
  }
  for (var i = 1; i < points.length - 1; i++) {
    final before = points[i] - points[i - 1];
    final after = points[i + 1] - points[i];
    final r =
        math.min(stub / 4, math.min(before.distance, after.distance) / 2);
    final into = points[i] - before / before.distance * r;
    final out = points[i] + after / after.distance * r;
    path
      ..lineTo(into.dx, into.dy)
      ..quadraticBezierTo(points[i].dx, points[i].dy, out.dx, out.dy);
  }
  return path..lineTo(b.dx, b.dy);
}

/// The corners of a run from [p] to [q] that leaves and arrives level: level,
/// 45 degrees, square across, 45 degrees, level again, with [level] of level
/// travel at each end where there is room for it.
List<Offset> _angledRun(Offset p, Offset q, double level) {
  final m = (p + q) / 2;
  final sx = (q.dx - p.dx).sign;
  final sy = (q.dy - p.dy).sign;
  final across = (q.dx - p.dx).abs() / 2;
  final down = (q.dy - p.dy).abs() / 2;
  var d = math.max(0.0, math.min(down, across - level));
  // A square step shorter than the level ends reads as a notch between the
  // two slopes, so they meet instead and the level ends give up the length.
  if (down - d < level / 2) d = math.min(down, across);
  return [
    p,
    Offset(m.dx - sx * d, p.dy),
    Offset(m.dx, p.dy + sy * d),
    Offset(m.dx, q.dy - sy * d),
    Offset(m.dx + sx * d, q.dy),
    q,
  ];
}

/// The glyphs in the toolbar. A size down from the
/// row glyphs' 16 (the manifest's number, not a preference).
const double graphIconSize = 13;

// The graph adds through the Ctrl+Space console: one search
// surface, one key. `shell/fx_console_frb.dart` owns its shape.

/// A group's wash: the air it leaves round its members, and the bar its name
/// sits in above them, which is the height of a box's own header.
const double graphGroupPad = 12;
const double graphGroupHead = graphNodeHeaderHeight;

/// What a saved group is called on disk: the engine's own constant, restated
/// here because the file's name needs the letters.
const String graphGroupExtension = 'lumgrp';

/// Where an unplaced box lands: the chain marches right, the drivers sit
/// below it. The drawing's own spacing: a node's width plus 88 of air, and
/// 64 along the layer's own chain.
const double _autoX = 26;
const double _autoY = 44;
const double _autoAir = 88;
const double _chainAir = 64;

/// The smallest a graph opens at, so its words stay readable.
const double _openFloor = 0.85;
const double _autoDriverY = 222;
const double _autoDriverStepY = 140;

/// How near a socket the pointer has to be to take hold of it.
const double _socketGrab = 7;

/// How near a socket a wire in hand has to come to snap to it. Over a box, the
/// nearest socket on that box that takes the wire is reached from anywhere.
const double _socketSnap = 24;

/// The pointer travel that turns a press into a drag rather than a click.
const double graphDragSlop = 3;

/// The header tint a **Custom shader** box wears, in the outer graph and on
/// every box of its inner graph alike (owner item 13b): the theme's
/// own viz family — `curve[0]`, the household ramp's lead — washed over the
/// ordinary header ground, so a programmable box reads at a glance without a
/// third colour system beside the port legend and the label palette.
Color graphShaderHeader(LumitTheme t) =>
    Color.alphaBlend(t.curve.first.withValues(alpha: 0.14), t.surface2);

/// Which theme token a port type draws in: eight types, five
/// colours, grouped as the drawing's legend groups them.
Color portColour(LumitTheme t, BridgePortType type) => switch (type) {
      BridgePortType.image ||
      BridgePortType.matte ||
      BridgePortType.depth =>
        t.port.image,
      BridgePortType.number => t.port.number,
      BridgePortType.colour => t.port.colour,
      BridgePortType.shape || BridgePortType.points => t.port.geometry,
      BridgePortType.audio => t.port.audio,
    };

/// Every item in the project, folders walked, in the order they are filed.
///
/// The engine answers one level at a time so the project panel draws only the
/// rows it shows; a list of what may be brought into a graph wants all of them,
/// and asks for it from a gesture rather than from a rebuild.
List<ItemReference> graphProjectItems(ProjectReference? project) {
  final out = <ItemReference>[];
  void walk(List<ItemReference> items) {
    for (final item in items) {
      if (item case ItemReference_Folder(:final field0)) {
        walk(field0.getChildren());
      } else {
        out.add(item);
      }
    }
  }

  walk(project?.getItems() ?? const []);
  return out;
}

/// The composition an item id names, or null when the project has none.
///
/// For the two double-clicks that open a graph: a Node graph box on this
/// canvas, and a Read box of a comp on the node graph's own.
CompositionReference? graphCompById(ProjectReference? project, UuidValue? id) {
  if (id == null) return null;
  for (final item in graphProjectItems(project)) {
    if (item case ItemReference_Composition(:final field0)) {
      if (field0.internalid == id) return field0;
    }
  }
  return null;
}

/// A stable string for a node reference, so positions and lookups can live in
/// a plain map. `BridgeNodeRef` is a freezed union without a usable key.
String graphNodeKey(BridgeNodeRef node) => switch (node) {
      BridgeNodeRef_Source() => 'source',
      BridgeNodeRef_Out() => 'out',
      BridgeNodeRef_Effect(:final field0) => 'effect:$field0',
      BridgeNodeRef_Driver(:final field0) => 'driver:$field0',
    };

/// Tab, or Shift+A as Blender has it: the keys that open the console over a
/// focused canvas, each only while Settings leaves it on.
bool graphAddKey(KeyEvent event, InterfaceSettings settings) {
  final keys = HardwareKeyboard.instance;
  if (keys.isControlPressed || keys.isAltPressed || keys.isMetaPressed) {
    return false;
  }
  return switch (event.logicalKey) {
    LogicalKeyboardKey.tab =>
      !keys.isShiftPressed && settings.tabOpensNodeSearch,
    LogicalKeyboardKey.keyA =>
      keys.isShiftPressed && settings.shiftAOpensNodeSearch,
    _ => false,
  };
}

/// M with no modifier: the key that mutes the picked boxes, as node editors
/// have it, and unmutes them again.
bool graphMuteKey(KeyEvent event) {
  final keys = HardwareKeyboard.instance;
  return event.logicalKey == LogicalKeyboardKey.keyM &&
      !keys.isControlPressed &&
      !keys.isAltPressed &&
      !keys.isMetaPressed &&
      !keys.isShiftPressed;
}

/// Where the pointer is inside the box [key] names, in that box's own pixels,
/// or null when it is somewhere else.
Offset? graphPointerIn(GlobalKey key) {
  final pointer = lastKnownPointerPosition;
  final box = key.currentContext?.findRenderObject();
  if (pointer == null || box is! RenderBox || !box.attached) return null;
  final local = box.globalToLocal(pointer);
  return (Offset.zero & box.size).contains(local) ? local : null;
}

/// What a parameter row needs to know about the wire feeding it: the
/// driver's name, the type the wire carries, and whether the source is a box
/// reading a points stream that has none.
typedef DrivenParam = ({String driver, BridgePortType type, bool noStream});

/// Which of [layer]'s effect parameters a driver is wired to, by
/// `effectId/paramId`. A wire's colour is its **source** port's type, which is
/// what the parameter is now following.
///
/// **One read, held.** Every panel that draws parameter rows — Effect controls,
/// the Node panel, the Timeline's fold-out — asks this at the moments the graph
/// can change (the selection moves, the document commits) and keeps the answer;
/// asking it from a build is exactly the traffic `bridge_call_budget_test`
/// guards against. Empty for every layer that has never been wired,
/// which is nearly all of them.
Map<String, DrivenParam> drivenParamsOf(LayerReference layer) {
  final BridgeLayerGraph graph;
  try {
    graph = layer.getGraph();
  } catch (_) {
    // The layer has gone; the rows simply draw their own controls again.
    return {};
  }
  return drivenParamsIn(graph);
}

/// The same reading, for a panel that already holds the graph.
Map<String, DrivenParam> drivenParamsIn(BridgeLayerGraph graph) {
  final out = <String, DrivenParam>{};
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
        // A points wire's source is a *stack effect*, so the row names
        // the effect that hands the data over.
        BridgeOutputRef_EffectData(:final effect, :final port) => (
            graphNodeKey(BridgeNodeRef.effect(effect)),
            port
          ),
        BridgeOutputRef_EffectOutput(:final effect) => (
            graphNodeKey(BridgeNodeRef.effect(effect)),
            'output'
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

/// The effect this box stands for, or null for the other three kinds, and for
/// a key naming a box that has gone.
UuidValue? _effectIdOf(BridgeNodeRef? node) =>
    node is BridgeNodeRef_Effect ? node.field0 : null;

UuidValue? _driverIdOf(BridgeNodeRef? node) =>
    node is BridgeNodeRef_Driver ? node.field0 : null;

/// A port type that the image chain owns. The chain is wired by construction —
/// the picture always flows straight down the stack — so an image socket takes
/// no drag: reordering is the stack view's gesture, and this panel is a second
/// view of it, not a second opinion about it.
bool _isChainType(BridgePortType type) => type == BridgePortType.image;

/// One socket as the canvas draws it: which box (by the string [graphNodeKey]
/// hands out), which port, and which side.
class GraphSocket {
  final String node;
  final BridgePort port;
  final bool isInput;
  final Offset at;
  const GraphSocket(this.node, this.port, this.isInput, this.at);
}

/// Where an unplaced box lands: a row marching right, and a second row below
/// it for the boxes that make values rather than pictures. [width] is the
/// widest box the canvas draws, so the row steps clear of it.
Offset graphAutoPlace(int index,
    {bool lower = false, double width = graphNodeWidth}) {
  final step = width + _autoAir;
  return lower
      ? Offset(_autoX + index * step, _autoDriverY + index * _autoDriverStepY)
      : Offset(_autoX + index * step, _autoY);
}

/// A spot near [at] with no box already on it, in canvas units.
///
/// The console has no pointer to aim by, so it puts every box on the middle of
/// the view. Without this the second box lands exactly on the first, and
/// pressing a twirl or grabbing a socket then answers for whichever box is on
/// top, which reads as the box doing nothing at all.
Offset graphClearSpot(Offset at, Iterable<Rect> taken) {
  const step = 28.0;
  var spot = at;
  for (var n = 0; n < 40; n++) {
    final near = taken.any((r) => (r.topLeft - spot).distance < step);
    if (!near) return spot;
    spot += const Offset(step, step);
  }
  return spot;
}

/// Whether [context] is drawn. The dock keeps a pane behind another tab
/// alive but offstage, and such a canvas must not answer for the one in
/// front.
bool graphOnStage(BuildContext context) {
  RenderObject? node = context.findRenderObject();
  if (node == null || !node.attached) return false;
  while (node != null) {
    if (node is RenderOffstage && node.offstage) return false;
    final parent = node.parent;
    node = parent is RenderObject ? parent : null;
  }
  return true;
}

/// The view that fits [rects] in [viewport] with a little air round them,
/// the zoom held to [maxZoom]. Null when there is nothing to fit.
({Offset pan, double zoom})? graphFrame(Size viewport, Iterable<Rect> rects,
    {double maxZoom = 2}) {
  Rect? bounds;
  for (final r in rects) {
    bounds = bounds?.expandToInclude(r) ?? r;
  }
  if (bounds == null || viewport.isEmpty) return null;
  bounds = bounds.inflate(20);
  final zoom = math
      .min(viewport.width / bounds.width, viewport.height / bounds.height)
      .clamp(0.2, maxZoom);
  return (
    zoom: zoom,
    pan: Offset(
      (viewport.width - bounds.width * zoom) / 2 - bounds.left * zoom,
      (viewport.height - bounds.height * zoom) / 2 - bounds.top * zoom,
    ),
  );
}

/// The rectangle a group's wash covers, in canvas units: its members' own
/// bounds, plus air all round and a band above for its name.
///
/// **Derived, never stored**. A group holds names, not geometry, so the
/// wash follows a member the moment it is dragged and cannot go stale. Null
/// where the canvas is drawing none of the members — a group whose boxes have
/// all gone draws nothing rather than a rectangle round the origin.
Rect? graphGroupRect(Iterable<Rect> members,
    {double head = graphGroupHead}) {
  Rect? bounds;
  for (final member in members) {
    bounds = bounds == null ? member : bounds.expandToInclude(member);
  }
  if (bounds == null) return null;
  return Rect.fromLTRB(
    bounds.left - graphGroupPad,
    bounds.top - graphGroupPad - head,
    bounds.right + graphGroupPad,
    bounds.bottom + graphGroupPad,
  );
}

/// The whole canvas worked out from the held graph: every box's rectangle and
/// every socket's centre, in canvas units. Rebuilt from arithmetic each build —
/// no bridge call, no allocation the panel keeps.
class GraphLayout {
  final List<GraphBox> boxes;
  final Map<String, GraphBox> byKey;

  GraphLayout(this.boxes) : byKey = {for (final b in boxes) b.key: b};

  /// The socket nearest [at], within grabbing distance, or null.
  GraphSocket? socketAt(Offset at) {
    for (final box in boxes) {
      // A folded box's ends stand for every wire on it, so they take none.
      if (box.collapsed) continue;
      for (final (isInput, ports) in [
        (true, box.inputs),
        (false, box.outputs)
      ]) {
        for (final port in ports) {
          final centre = box.socket(port.id, isInput);
          if (centre != null && (centre - at).distance <= _socketGrab) {
            return GraphSocket(box.key, port, isInput, centre);
          }
        }
      }
    }
    return null;
  }

  /// The socket a wire let go of at [at] would join: the nearest one that
  /// [fits], within [_socketSnap] of the pointer or anywhere on the box under
  /// it. Null where none would.
  GraphSocket? snapAt(Offset at, bool Function(GraphSocket) fits) {
    final over = boxAt(at);
    GraphSocket? best;
    var nearest = double.infinity;
    for (final box in boxes) {
      if (box.collapsed) continue;
      for (final (isInput, ports) in [
        (true, box.inputs),
        (false, box.outputs)
      ]) {
        for (final port in ports) {
          final centre = box.socket(port.id, isInput);
          if (centre == null) continue;
          final d = (centre - at).distance;
          if (d >= nearest || (d > _socketSnap && box != over)) continue;
          final socket = GraphSocket(box.key, port, isInput, centre);
          if (!fits(socket)) continue;
          best = socket;
          nearest = d;
        }
      }
    }
    return best;
  }

  GraphBox? boxAt(Offset at) {
    for (final box in boxes.reversed) {
      if (box.rect.contains(at)) return box;
    }
    return null;
  }
}

/// This layer's graph as boxes: the chain marching right, the drivers below it.
///
/// A **driver** draws every socket it has: the box is small, and its ports are
/// the whole of what it is for. An **effect** draws the picture's own sockets
/// always, and a parameter socket when a wire is on it or when the box is
/// twirled open. An open box draws a control on each of its [params] rows, as
/// a node graph composition's does.
GraphLayout _layoutOf(
  BridgeLayerGraph graph,
  Map<String, Offset> positions,
  Map<String, double> widths,
  Set<String> exposed,
  String layerName,
  Map<String, CompBoxRows> params, {
  BridgeNodeLooks? looks,
}) {
  final colours = {
    for (final c in looks?.colours ?? const <BridgeNodeColour>[])
      graphNodeKey(c.node): c.colour,
  };
  final folded = {...?looks?.collapsed.map(graphNodeKey)};
  final boxes = <GraphBox>[];
  var chainX = _autoX;
  var drivers = 0;
  for (final node in graph.nodes) {
    final key = graphNodeKey(node.node);
    final isDriver = node.node is BridgeNodeRef_Driver;
    final derived = _derivedNode(node.node);
    final held = params[key];
    // The chain steps on each box's shut width, so a chain of seven opens at
    // a size you can read. A box twirled open eats into the air after it.
    final shut = node.node is BridgeNodeRef_Out
        ? graphOutNodeWidth
        : held?.telling.isNotEmpty ?? false
            ? graphNodeOpenWidth
            : graphNodeWidth;
    final placed = positions[key] ??
        (isDriver
            ? graphAutoPlace(drivers, lower: true, width: graphNodeOpenWidth)
            : Offset(chainX, _autoY));
    if (isDriver) {
      drivers++;
    } else {
      chainX += shut + _chainAir;
    }
    final open = exposed.contains(key) || isDriver;
    // Open, every row under its group's heading; shut, the rows that tell
    // something: a wire on them, a key, or a value off its default.
    final rows = held == null
        ? const <BridgeParamInfo>[]
        : open
            ? held.open
            : held.telling;
    boxes.add(graphLayoutBox(
      (
        key: key,
        title: node.node is BridgeNodeRef_Source
            ? layerName
            : engineLabel(node.label),
        customName: node.customName,
        kicker: null,
        enabled: node.enabled,
        tick: !derived,
        twirl: !derived && !isDriver,
        rename: !derived,
        tinted: node.matchName == 'custom_shader',
        inputs: node.inputs,
        outputs: node.outputs,
        noStream: graphNoStream(node),
      ),
      placed,
      open: open,
      params: [for (final p in rows) p.id],
      kickers: open ? held?.kickers ?? const {} : const {},
      width: graphBoxWidth(
          node.node is BridgeNodeRef_Out
          ? graphOutNodeWidth
          : rows.isEmpty
              ? graphNodeWidth
              : graphNodeOpenWidth,
          widths[key],
          open: rows.isNotEmpty),
      foot: node.matchName == 'custom_shader' ? graphShaderFoot : 0,
      colour: colours[key] ?? 0,
      collapsed: folded.contains(key),
    ));
  }
  return GraphLayout(boxes);
}

/// The picture's own ends, which are read off the effect list rather than put
/// there: they carry no tick, no twirl and no name of their own.
bool _derivedNode(BridgeNodeRef node) =>
    node is BridgeNodeRef_Source || node is BridgeNodeRef_Out;

/// The image chain, in stack order: Source, each effect, Layer out. Its wires
/// are not stored anywhere: they *are* the list.
List<GraphBox> _chainOf(GraphLayout layout) => [
      for (final b in layout.boxes)
        if (!b.key.startsWith('driver:')) b,
    ];

/// A box that consumes a points stream with **nothing wired into it**.
///
/// Structural, not per-frame: the read model says whether the socket carries a
/// wire, and the engine's contract says what an empty stream answers — Count
/// nought and Nearest distance "nothing is anywhere near" (points-stream.md
/// §2.2). So the panel can say so without asking for a value, which keeps
/// this off the rebuild path.
bool graphNoStream(BridgeGraphNode node) =>
    graphNoStreamPorts(node.inputs, node.matchName);

/// The same question asked of a box's sockets.
/// One wire is enough: Merge points has four sockets and reads any of them.
/// Measure measures its layer when no stream is wired, so it never asks.
bool graphNoStreamPorts(List<BridgePort> inputs, String matchName) {
  if (matchName == 'measure') return false;
  final points = inputs.where((p) => p.portType == BridgePortType.points);
  return points.isNotEmpty && !points.any((p) => p.wired);
}

/// Where one stored wire starts and ends in canvas units, and the type it
/// carries — the *source* port's, which is the type the wire is. Null when
/// either end names a box that is not on the canvas.
///
/// Read by the painter, which colours the wire by that type, and by the drop
/// test, which asks how near a dragged box is to the curve (N7).
(Offset, Offset, BridgePortType)? _edgeEnds(
    GraphLayout layout, BridgeGraphEdge e) {
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
    BridgeOutputRef_EffectOutput(:final effect) => (
        graphNodeKey(BridgeNodeRef.effect(effect)),
        'output'
      ),
  };
  final (toKey, toPort) = _inputEnd(e);
  final fromBox = layout.byKey[fromKey];
  final toBox = layout.byKey[toKey];
  if (fromBox == null || toBox == null) return null;
  final from = fromBox.socket(fromPort, false);
  final to = toBox.socket(toPort, true);
  if (from == null || to == null) return null;
  final i = fromBox.outputs.indexWhere((p) => p.id == fromPort);
  return (from, to, fromBox.outputs[i < 0 ? 0 : i].portType);
}

/// The box and socket a stored wire ends in.
(String, String) _inputEnd(BridgeGraphEdge e) => switch (e.to) {
      BridgeInputRef_Param(:final node, :final port) => (
          graphNodeKey(node),
          port
        ),
      BridgeInputRef_Matte(:final effect) => (
          graphNodeKey(BridgeNodeRef.effect(effect)),
          'matte'
        ),
    };

/// Every wire the layer canvas draws, as the ties name them, each with the
/// stored wire it is (null for one of the picture's chain) and the type it
/// carries. [held] is the chain wire in hand, which is not drawn.
List<(TieWire, BridgeGraphEdge?, BridgePortType)> _layerWires(
    GraphLayout layout, List<BridgeGraphEdge> edges,
    {int? held, bool outUnwired = false}) {
  // The image chain's wires are not stored anywhere: they *are* the effect
  // list, read left to right. Drawing them from the box order is what makes
  // the stack view impossible to contradict.
  //
  // **A bypassed effect is not in the chain**, so the wire steps
  // over it: from the last box still in to the next one, leaving the
  // bypassed box floating with hollow sockets. That is the same picture the
  // engine renders - the effect is skipped - and it is why switching a box
  // off in Effect controls visibly unplugs it here.
  //
  // The Source and the Layer out always count as in: the picture starts and
  // ends at them whatever the stack does, so an all-bypassed stack still
  // draws exactly one wire, Source to Layer out. Unplugging the Layer out
  // itself (`outUnwired`) takes that last segment away too - nothing feeds
  // the layer, and the layer draws nothing.
  //
  // The indices are the FULL chain's throughout: `_InFlight.chain` and every
  // gesture in `_down` count boxes the same way, and renumbering them here
  // would put the dashed wire on the wrong socket.
  final chain = _chainOf(layout);
  final inChain = [
    for (var i = 0; i < chain.length; i++)
      if (i == 0 || i == chain.length - 1 || chain[i].card.enabled) i,
  ];
  final wires = <(TieWire, BridgeGraphEdge?, BridgePortType)>[];
  for (var k = 0; k + 1 < inChain.length; k++) {
    final i = inChain[k];
    final j = inChain[k + 1];
    // The wire in the hand is the dashed flight, not a drawn
    // segment: it has visibly left its input. The flight names the box it
    // feeds, which after a skip is anywhere in (i, j].
    if (held != null && held > i && held <= j) continue;
    // Nothing is plugged into the Layer out at all.
    if (outUnwired && j == chain.length - 1) continue;
    final port = j == chain.length - 1 ? 'image' : 'input';
    final from = chain[i].socket(i == 0 ? 'image' : 'output', false);
    final to = chain[j].socket(port, true);
    if (from != null && to != null) {
      wires.add((
        (key: tieWireKey(chain[j].key, port), from: from, to: to),
        null,
        BridgePortType.image,
      ));
    }
  }
  for (final edge in edges) {
    final ends = _edgeEnds(layout, edge);
    if (ends == null) continue;
    final (box, socket) = _inputEnd(edge);
    wires.add((
      (key: tieWireKey(box, socket), from: ends.$1, to: ends.$2),
      edge,
      ends.$3,
    ));
  }
  return wires;
}

/// How near a wire a dropped box has to land to fall into it, canvas units.
const double graphWireGrab = 18;

/// How far [at] is from a wire's curve, sampled along it.
///
/// Twenty points along a wire describe it closely enough to say whether a box
/// was dropped on it — this runs during a drag, over a handful of
/// wires, and never in an idle rebuild.
double graphWireDistance(Offset a, Offset b, Offset at,
    {WireStyle wires = WireStyle.curved}) {
  var best = double.infinity;
  for (final metric in graphWirePath(a, b, wires: wires).computeMetrics()) {
    for (var i = 0; i <= 20; i++) {
      final point =
          metric.getTangentForOffset(metric.length * i / 20)?.position;
      if (point != null) best = math.min(best, (point - at).distance);
    }
  }
  return best;
}

/// A wire being dragged: where it left, and where the pointer is now. What
/// every canvas here carries, and all the painters read.
class GraphFlight {
  final GraphSocket from;
  Offset to;

  /// The socket the wire would join if let go of now. The loose end is drawn
  /// on it, so the wire shows where it is going before the drop.
  GraphSocket? snap;
  GraphFlight(this.from, this.to);

  Offset get end => snap?.at ?? to;
}

/// The layer canvas's wire in hand, which knows about the image chain besides.
class _InFlight extends GraphFlight {
  /// The stored wire this drag took hold of, when the press landed on an input
  /// that already had one. A wire is grabbed by its **far** end — the drag
  /// leaves the producer's socket and follows the pointer — so letting go
  /// somewhere else re-routes it and letting go of nothing takes it off. Null
  /// for a wire being drawn afresh.
  final BridgeGraphEdge? detached;

  /// An **image-chain** wire in hand: the chain index of the box the
  /// grabbed wire feeds. The chain's wires are not stored anywhere — they are
  /// the effect list — so the drag carries the index rather than an edge, and
  /// the drop lowers to the stack's own ops. Null for every stored wire.
  final int? chain;

  /// Whether a chain wire was drawn **from an output** rather than picked
  /// up by the input it feeds. The two gestures differ in one place only:
  /// letting go of nothing. A wire picked off an input is being taken away,
  /// so the drop takes its effect out of the stack; a wire drawn out of an
  /// output is being offered somewhere, and dropping it on the ground is a
  /// change of mind, not an instruction to delete the box it came from.
  final bool fromOutput;

  _InFlight(super.from, super.to,
      {this.detached, this.chain, this.fromOutput = false});
}

/// Boxes being dragged: where the pointer took hold, and where each box being
/// moved started.
///
/// A drag on a box that is **part of the picked set moves the whole set**
/// (docs/07 §4.5's rule for every surface here), so this carries an origin per
/// box rather than one.
class GraphNodeDrag {
  /// The box actually pressed — what a release that never moved collapses the
  /// selection to, because a plain click replaces.
  final String key;
  final Offset grab;
  final Map<String, Offset> origins;

  /// The press landed on a box that was already picked, so the pick was left
  /// alone on the way down. A click that does not move then means what a plain
  /// click always means: this box, and only this box.
  final bool collapse;
  GraphNodeDrag(this.key, this.grab, this.origins, {this.collapse = false});
}

/// How near a box's left or right edge the pointer takes the edge, in screen
/// pixels either side of it.
const double graphEdgeGrab = 4;

/// The narrowest a shut box can be dragged.
const double graphMinNodeWidth = 90;

/// The widest a box can be dragged.
const double graphMaxNodeWidth = 900;

/// The narrowest a box can be: a shut one keeps its name, an open one a name
/// and a short value.
double graphMinWidth({required bool open}) => open ? 140 : graphMinNodeWidth;

/// A box's content width: the one an edge drag gave it, held inside what the
/// box can take, or [fallback] where it has none.
double graphBoxWidth(double fallback, double? dragged, {required bool open}) =>
    dragged == null
        ? fallback
        : dragged.clamp(graphMinWidth(open: open), graphMaxNodeWidth);

/// The box whose left or right edge is under [at], and which edge. The Layer
/// out and a box whose width cannot change are left out by [resizable].
({GraphBox box, bool left})? graphEdgeAt(
    GraphLayout layout, Offset at, double zoom,
    {bool Function(GraphBox box)? resizable}) {
  final reach = graphEdgeGrab / zoom;
  for (final box in layout.boxes.reversed) {
    final r = box.rect;
    if (at.dy < r.top || at.dy > r.bottom) continue;
    if (resizable != null && !resizable(box)) continue;
    if ((at.dx - r.left).abs() <= reach) return (box: box, left: true);
    if ((at.dx - r.right).abs() <= reach) return (box: box, left: false);
    if (r.contains(at)) return null;
  }
  return null;
}

/// The left-right arrows while the pointer rests on a box's edge or drags
/// one; the cursor of whatever is underneath otherwise. Laid over the whole
/// canvas and let through, so it takes no press.
Widget graphEdgeCursor(bool on) => Positioned.fill(
      child: MouseRegion(
        opaque: false,
        cursor: on ? SystemMouseCursors.resizeLeftRight : MouseCursor.defer,
      ),
    );

/// A box's edge in hand: the side, where the press was and the box as it
/// stood, so every move works from the start rather than adding up steps.
class GraphEdgeDrag {
  final String key;
  final bool left;
  final double grab;
  final Rect from;
  final bool open;
  GraphEdgeDrag(this.key, this.left, this.grab, this.from, {required this.open});

  /// The content width and left edge for the pointer at [x]: the right edge
  /// stays put while the left one moves, and the reverse.
  ({double width, double x}) at(double x) {
    final start = from.width - 2;
    final width = (left ? start - (x - grab) : start + (x - grab))
        .clamp(graphMinWidth(open: open), graphMaxNodeWidth);
    return (width: width, x: left ? from.right - width - 2 : from.left);
  }
}

class GraphPanelFrb extends StatefulWidget {
  /// The driver catalogue seam, injected by tests so the popover can be
  /// asserted without the real registry.
  final List<BridgeEffectInfo> Function()? driversLister;

  /// The saved-group library seam, injected for the same reason — a test says
  /// what is in the folder rather than writing to the user's own library.
  final List<BridgePresetInfo> Function()? groupsLister;

  /// The folder "Save group…" writes into, injected so a test never writes
  /// to the user's own library. Null takes the preset library folder.
  final String? Function()? groupLibrary;

  /// The node graph canvas's own two catalogue seams, handed straight on.
  final List<BridgeEffectInfo> Function()? graphNodesLister;
  final List<BridgeEffectInfo> Function()? effectsLister;

  const GraphPanelFrb({
    super.key,
    this.driversLister,
    this.groupsLister,
    this.groupLibrary,
    this.graphNodesLister,
    this.effectsLister,
  });

  @override
  State<GraphPanelFrb> createState() => _GraphPanelFrbState();
}

class _GraphPanelFrbState extends State<GraphPanelFrb>
    with GraphGroups<GraphPanelFrb>, GraphTies<GraphPanelFrb> {
  LumitUiState? _ui;
  LayerReference? _layer;

  /// The held graph. Read on selection and on document change — never in a
  /// build, which is what the budget test is guarding.
  BridgeLayerGraph? _graph;

  /// The held graph's boxes by node key, so a gesture holding nothing but the
  /// key can reach the reference, the match name and the enabled mark the
  /// commits want. Rebuilt with every read, so a box that has gone is simply
  /// not in it.
  Map<String, BridgeGraphNode> _byKey = const {};

  /// Each effect and driver box's instance, and the rows it draws when open,
  /// read beside the graph and never in a build.
  Map<String, BridgeEffectInstanceInfo> _infos = const {};
  Map<String, CompBoxRows> _params = const {};

  /// A drag on an effect box's control: staged, previewed, committed on
  /// release as one op, as the Node panel's is.
  final EffectStackEditor _stack = EffectStackEditor();

  /// The same for a driver box, whose commit is `setGraph`.
  ({String key, Map<String, BridgeEffectValue> values})? _driverDrag;
  final PreviewThrottle _driverPreview = PreviewThrottle();

  /// The layer's name, taken at the same moment, so the Source box can be
  /// headed without asking the model during a paint.
  String _layerName = '';

  /// Canvas positions, staged: a drag moves this map and the release commits
  /// it. Seeded from the document on each read.
  Map<String, Offset> _positions = {};

  /// The widths edge drags gave boxes, by key.
  Map<String, double> _widths = {};

  /// A box edge in hand, and the edge the pointer is resting on.
  GraphEdgeDrag? _edgeDrag;
  bool _edgeHover = false;

  /// **The picked boxes**, by node key, in the order they were picked.
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
  /// selection.
  final Map<String, BridgeNodeRef> _selection = {};

  /// The anchor: the box picked last, or null with nothing picked.
  BridgeNodeRef? get _selected =>
      _selection.isEmpty ? null : _selection.values.last;

  bool _isPicked(String key) => _selection.containsKey(key);

  /// The held box behind a key, or null when it has gone.
  BridgeGraphNode? _node(String key) => _byKey[key];

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

  /// The shared project's people, whose picked boxes are ringed here. Its
  /// roster only moves while a project is shared.
  ShareState? _share;

  /// Someone came, went or picked something else. Nothing happens here
  /// while the project is not shared.
  void _onRoster() {
    if (!mounted) return;
    final now = _share?.active ?? false;
    if (now || _sharing) setState(() => _sharing = now);
  }

  bool _sharing = false;

  /// What a box is called in a shared project: an effect by the path the
  /// Timeline and the Effect controls panel know it by, so one picked in any
  /// of the three is marked in all of them, and a driver by a name of its
  /// own. Null for the two ends of the graph.
  String? _boxName(LayerReference layer, String key) {
    final owner = layer.internallayerId.toString();
    final colon = key.indexOf(':');
    if (colon < 0) return null;
    final id = key.substring(colon + 1);
    return key.startsWith('effect:')
        ? effectPath(owner, id)
        : '$owner/drivers/$id';
  }

  void _publishPick() {
    groupPick = null;
    _ui?.graphNode.value = _selected;
    final layer = _layer;
    if (layer == null) return;
    // A driver is not an effect, so it is not in the selection below. In a
    // shared project the others are told of it the way a panel with no
    // selection tells them of a row.
    if (_sharing) {
      for (final key in _selection.keys) {
        if (!key.startsWith('driver:')) continue;
        if (_boxName(layer, key) case final name?) _ui?.touchProperty(name);
      }
    }
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

  /// The box whose name is an inline editor, by node key. A
  /// double-click on a card's name opens it — the canvas reads its pointers
  /// through a raw `Listener`, so the card's own double-tap costs the
  /// selection nothing: a press still picks the box the instant it lands.
  String? _renamingNode;

  Offset _pan = Offset.zero;
  double _zoom = 1;

  /// Adding a node wires it up in the same commit; deleting one takes its
  /// wires with it. Both on, both the drawing's state, and both `animated`
  /// while on because that is what a pill switch is everywhere.
  bool _autoWire = true;
  bool _heal = true;

  /// The snap magnet: while on — and it starts on, drawn
  /// lit in the toolbar — a node drag lands on the dot grid's own pitch,
  /// so boxes line up without nudging. Session view state, exactly
  /// as Auto-wire and Heal are.
  bool _snapToGrid = true;

  _InFlight? _flight;
  GraphNodeDrag? _nodeDrag;

  /// The double-click that enters a Custom shader's inner graph, and
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

  /// A right-click on empty canvas, waiting for its release.
  bool _menuPress = false;

  /// The box a right press landed on, whose menu opens if the release never
  /// moved.
  String? _menuBox;

  @override
  List<TieWire> tieWiresNow() {
    final graph = _graph;
    if (graph == null) return const [];
    return _wiresOf(_layoutOf(
      graph,
      _positions,
      _widths,
      graph.wiring.exposed.map(graphNodeKey).toSet(),
      _layerName,
      _params,
      looks: _looks,
    ));
  }

  /// The boxes as they stand, for a placement that has to miss them.
  List<GraphBox> _boxesNow() {
    final graph = _graph;
    if (graph == null) return const [];
    return _layoutOf(
      graph,
      _positions,
      _widths,
      graph.wiring.exposed.map(graphNodeKey).toSet(),
      _layerName,
      _params,
      looks: _looks,
    ).boxes;
  }

  /// The canvas's own box, so a drop and the pointer are measured in it.
  final GlobalKey _canvasKey = GlobalKey();

  final FocusNode _canvasFocus = FocusNode(debugLabel: 'graph canvas');

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (identical(ui, _ui)) return;
    _unbind();
    _ui = ui;
    _share ??= Provider.of<LumitState>(context, listen: false).share
      ..roster.addListener(_onRoster);
    _sharing = _share!.active;
    ui.selectedLayer.addListener(_reload);
    ui.model.addListener(_reload);
    // Entering (or leaving) a Custom shader's inner graph swaps this panel's
    // whole face, from either surface the double-click lives on.
    ui.shaderGraphEntry.addListener(_onShaderEntry);
    // `Ctrl+A` here means every box on this canvas, which it can only
    // mean now the pick is a set.
    ui.selectAllRequest.addListener(_onSelectAllRequested);
    // The Node panel's index of boxes asks for a pick the same way the
    // canvas publishes one.
    ui.graphNode.addListener(_onGraphNodeAsked);
    ui.frameBoxRequest.addListener(_onFrameAsked);
    ui.graphBoxMenuAsk.addListener(_onBoxMenuAsked);
    // A reveal that opened this panel asked before it was listening: the
    // pick and the framing are answered once the canvas has drawn.
    if (ui.graphNode.value != null || ui.frameBoxRequest.value != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        _onGraphNodeAsked();
        _onFrameAsked();
      });
    }
    // **Ctrl+Space with this panel focused adds to the graph**. The
    // shell's console applies to the selected layer; the same key over this
    // canvas opens the same popover wearing the canvas's own list. Chained the
    // way the Delete claim is, because there is one claim and the inner shader
    // graph wants it too when it is the panel's face.
    if (ui.consoleClaim != _consoleClaim) _priorConsoleClaim = ui.consoleClaim;
    ui.consoleClaim = _consoleClaim;
    ui.nodeSearchExhibit.addListener(_onSearchExhibit);
    // **Delete means the picked boxes while this panel is the focused one**,
    // claimed rather than left to the canvas's own focus handler: the shell
    // answers Delete on the hardware keyboard, which runs *before* the focus
    // tree and swallows the key, so a picked node had its layer deleted out
    // from under it instead. The shell asks this claim first and stands down
    // when it says yes.
    //
    // Chained onto whatever held the claim before — the Timeline, for its mask
    // rows — because there is one claim and more than one panel that wants it.
    if (ui.deleteClaim != _deleteClaim) _priorDeleteClaim = ui.deleteClaim;
    ui.deleteClaim = _deleteClaim;
    // Ctrl+C and Ctrl+V over the layer's graph mean the picked effects, as
    // they do in Effect controls. A node graph composition and a shader
    // hold claims of their own above this one.
    if (ui.copyClaim != _copyClaim) _priorCopyClaim = ui.copyClaim;
    ui.copyClaim = _copyClaim;
    if (ui.pasteClaim != _pasteClaim) _priorPasteClaim = ui.pasteClaim;
    ui.pasteClaim = _pasteClaim;
    _reload();
  }

  bool Function()? _priorCopyClaim;
  bool Function()? _priorPasteClaim;

  bool _ownsClipboard(LumitUiState ui) =>
      mounted &&
      ui.activePanel == Panel.graph &&
      !_showsCompGraph &&
      ui.shaderGraphEntry.value == null;

  bool _copyClaim() {
    final ui = _ui;
    if (ui == null || !_ownsClipboard(ui)) {
      return _priorCopyClaim?.call() ?? false;
    }
    return _copyPicked(ui) || (_priorCopyClaim?.call() ?? false);
  }

  bool _pasteClaim() {
    final ui = _ui;
    if (ui == null || !_ownsClipboard(ui)) {
      return _priorPasteClaim?.call() ?? false;
    }
    return _pasteEffects(ui) || (_priorPasteClaim?.call() ?? false);
  }

  /// The claim this panel had to displace to take Delete.
  bool Function()? _priorDeleteClaim;

  bool _deleteClaim() {
    final ui = _ui;
    if (!mounted ||
        ui == null ||
        ui.activePanel != Panel.graph ||
        _showsCompGraph) {
      return _priorDeleteClaim?.call() ?? false;
    }
    return tieDelete() ||
        _deleteSelected() ||
        (_priorDeleteClaim?.call() ?? false);
  }

  /// Whether the panel's face is the **node graph composition's** canvas,
  /// which holds its own pick and answers the keys itself. Read off the held
  /// model, so asking costs nothing.
  bool get _showsCompGraph => _ui?.model.isNodeGraph ?? false;

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
        ui.activePanel != Panel.graph ||
        ui.shaderGraphEntry.value != null ||
        _showsCompGraph ||
        _graph == null ||
        _searching) {
      return _priorConsoleClaim?.call() ?? false;
    }
    _openSearch(graphClearSpot(
      _toCanvas(Offset(_viewport.width / 2, _viewport.height / 2)),
      [for (final b in _boxesNow()) b.rect],
    ));
    return true;
  }

  void _onShaderEntry() {
    if (mounted) setState(() {});
  }

  void _unbind() {
    _share?.roster.removeListener(_onRoster);
    _share = null;
    _ui?.nodeSearchExhibit.removeListener(_onSearchExhibit);
    _exhibitDown?.call();
    _exhibitDown = null;
    _ui?.selectedLayer.removeListener(_reload);
    _ui?.model.removeListener(_reload);
    _ui?.shaderGraphEntry.removeListener(_onShaderEntry);
    _ui?.selectAllRequest.removeListener(_onSelectAllRequested);
    _ui?.graphNode.removeListener(_onGraphNodeAsked);
    _ui?.graphBoxMenuAsk.removeListener(_onBoxMenuAsked);
    _ui?.frameBoxRequest.removeListener(_onFrameAsked);
    if (_ui?.deleteClaim == _deleteClaim) {
      _ui!.deleteClaim = _priorDeleteClaim;
    }
    if (_ui?.consoleClaim == _consoleClaim) {
      _ui!.consoleClaim = _priorConsoleClaim;
    }
    if (_ui?.copyClaim == _copyClaim) _ui!.copyClaim = _priorCopyClaim;
    if (_ui?.pasteClaim == _pasteClaim) _ui!.pasteClaim = _priorPasteClaim;
  }

  /// Pick every box the canvas is drawing — Source and Layer out included,
  /// because they are picked by a click like any other box and the commands
  /// that cannot touch them already say so themselves.
  void _onSelectAllRequested() {
    final ui = _ui;
    if (!mounted || ui == null || _showsCompGraph) return;
    if (!ui.selectAllRequestIsFor(Panel.graph)) return;
    setState(() => _pick([for (final n in _graph?.nodes ?? const []) n.node]));
  }

  /// A pick asked for from outside, by the Node panel's index: the box
  /// becomes the pick here, where the pick lives. The canvas's own publish
  /// lands here too and finds the box already picked.
  void _onGraphNodeAsked() {
    final node = _ui?.graphNode.value;
    if (!mounted || _showsCompGraph || !graphOnStage(context)) return;
    // The Node panel going back to its index clears the pick with it.
    if (node == null) {
      if (_selection.isNotEmpty) setState(() => _pick(const []));
      return;
    }
    final key = graphNodeKey(node);
    if (_selection.containsKey(key) || !_byKey.containsKey(key)) return;
    setState(() => _pick([node]));
  }

  /// The Node panel's heading asking for a box's menu: the box becomes the
  /// pick, and the menu opens with every row the canvas has.
  void _onBoxMenuAsked() {
    final ask = _ui?.graphBoxMenuAsk.value;
    if (!mounted || ask == null || _showsCompGraph || !graphOnStage(context)) {
      return;
    }
    final node = _byKey[ask.key];
    if (node == null) return;
    _ui!.graphBoxMenuAsk.value = null;
    if (!_isPicked(ask.key)) setState(() => _pick([node.node]));
    _boxMenu(ask.key, ask.at);
  }

  @override
  void dispose() {
    _unbind();
    // A held tick would fire into a panel that is gone.
    _driverPreview.cancel();
    _stack.clear();
    _canvasFocus.dispose();
    super.dispose();
  }

  /// The one read. Everything the canvas draws comes from here.
  void _reload() {
    if (!mounted) return;
    final layer = _ui?.selectedLayer.value;
    // Another layer's graph opens as a graph first opens: at rest or framed.
    if (layer?.internallayerId != _layer?.internallayerId) {
      _frameNext = true;
      _viewMoved = false;
    }
    BridgeLayerGraph? graph;
    var notes = const <BridgeGraphNote>[];
    var ties = const <BridgeWireTie>[];
    BridgeNodeLooks? looks;
    var name = '';
    final infos = <String, BridgeEffectInstanceInfo>{};
    final params = <String, CompBoxRows>{};
    if (layer != null) {
      try {
        graph = layer.getGraph();
        notes = layer.getGraphNotes();
        ties = layer.getGraphTies();
        looks = layer.getGraphLooks();
        name = layer.getInfo().name;
        final ports = {for (final n in graph.nodes) graphNodeKey(n.node): n.inputs};
        // The rows a box draws, by the Effect controls panel's rules.
        void read(BridgeNodeRef node, BridgeEffectInstance instance) {
          final key = graphNodeKey(node);
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
            {for (final p in ports[key] ?? const <BridgePort>[]) if (p.wired) p.id},
            layer.internallayerId,
          );
        }

        for (final instance in layer.getEffects()) {
          read(BridgeNodeRef.effect(instance.id()), instance);
        }
        for (final instance in layer.getGraphDrivers()) {
          read(BridgeNodeRef.driver(instance.id()), instance);
        }
      } catch (_) {
        // The layer has gone since the selection was made; the placeholder is
        // the honest answer until the selection catches up.
        graph = null;
      }
    }
    setState(() {
      _layer = layer;
      _graph = graph;
      _infos = infos;
      _params = params;
      _driverDrag = null;
      _notes = graph == null ? const [] : notes;
      _ties = graph == null ? const [] : ties;
      _looks = graph == null ? null : looks;
      _byKey = {
        for (final n in graph?.nodes ?? const <BridgeGraphNode>[])
          graphNodeKey(n.node): n,
      };
      _layerName = name;
      _positions = {
        for (final p in graph?.wiring.layout ?? const <BridgeNodePosition>[])
          graphNodeKey(p.node): Offset(p.x, p.y),
      };
      _widths = {
        for (final p in graph?.wiring.layout ?? const <BridgeNodePosition>[])
          if (p.width case final w?) graphNodeKey(p.node): w,
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

  // --- A box's own controls -----------------------------------------------

  /// What a wired parameter on an effect's box reads at the playhead, dimmed
  /// beside its port name: the wire decides it, so it is shown, not edited.
  /// Null for a driver's own inputs and anything but a number.
  Widget? _drivenValue(String key, String param) {
    final layer = _layer;
    final ui = _ui;
    final comp = ui?.selectedComp;
    final info = _infos[key];
    if (layer == null ||
        ui == null ||
        comp == null ||
        info == null ||
        key.startsWith('driver:')) {
      return null;
    }
    final held = info.values.where((v) => v.id == param).firstOrNull?.value;
    if (held is! BridgeEffectValue_Float) return null;
    final t = ThemeScope.of(context).theme;
    return ValueListenableBuilder<int>(
      valueListenable: ui.playheadFrame,
      builder: (context, frame, _) => Text(
        sampleScalarWithContext(
          scalar: held.field0,
          time: timeOfFrame(comp, frame),
          layer: layer,
          row: BridgeExpressionRow.effect(effect: info.id, param: param),
        ).toStringAsFixed(2),
        key: ValueKey<String>('graph-driven-$key-$param'),
        style: t.small.copyWith(color: t.textMuted),
      ),
    );
  }

  /// One parameter row on an open box: the Node panel's row, committed and
  /// previewed the way that panel's edit is.
  Widget _paramRow(String key, String param) {
    final layer = _layer;
    final ui = _ui;
    final comp = ui?.selectedComp;
    final info = _infos[key];
    final held = _params[key];
    final p = held?.open.where((p) => p.id == param).firstOrNull;
    if (layer == null ||
        ui == null ||
        comp == null ||
        info == null ||
        held == null ||
        p == null) {
      return const SizedBox.shrink();
    }
    final driver = key.startsWith('driver:');
    final values = {for (final v in info.values) v.id: v.value};
    final drag = _driverDrag;
    BridgeEffectValue? valueOf(String id) =>
        (driver
            ? (drag?.key == key ? drag!.values[id] : null)
            : _stack.stagedValue(info.id, id)) ??
        values[id];
    Map<String, BridgeEffectValue> chained(
            String param, BridgeEffectValue value) =>
        withChained(info, param, value, comp, ui.playheadFrame.value);
    return EffectParamRowFrb(
      key: ValueKey<String>('graph-row-$key-$param'),
      // An effect on the layer's stack. A driver's rows are not a graphics
      // template's to name.
      controlRow:
          driver ? null : paramControlRow(layer.internallayerId, info.id, p),
      effectId: info.id,
      param: p,
      value: valueOf(param),
      comp: comp,
      ownerLayerId: layer.internallayerId,
      ownerLayers: ui.model.layers,
      playheadFrame: ui.playheadFrame.value,
      onSeek: (frame) => ui.playheadFrame.value = frame,
      onWrite: (_, param, value) =>
          _writeParams(key, info, chained(param, value)),
      onLive: (_, param, value) =>
          _liveParams(key, info, chained(param, value)),
      rowPadding: EdgeInsets.zero,
      valueColumn: const ValueColumn(graphControlColumn, graphRowInset),
      siblings: values,
      // Greyed where another control has taken the row over.
      enabled: !held.disabled.contains(param),
      riders: [
        for (final r in held.riders[param] ?? const <BridgeParamInfo>[])
          (r, valueOf(r.id)),
      ],
      onAction: (effect, param) => _pressParam(info, driver, effect, param),
    );
  }

  /// The layer's drivers, freshly read, with [values] written into one of
  /// them: what a driver box's preview and commit both send.
  List<BridgeEffectInstance> _driversWith(LayerReference layer, UuidValue node,
      Map<String, BridgeEffectValue> values) {
    final drivers = layer.getGraphDrivers();
    for (final instance in drivers) {
      if (instance.id() != node) continue;
      for (final entry in values.entries) {
        instance.setValue(id: entry.key, value: entry.value);
      }
    }
    return drivers;
  }

  /// A typed value, or the release of a drag: one op, one undo step.
  void _writeParams(String key, BridgeEffectInstanceInfo info,
      Map<String, BridgeEffectValue> values) {
    final layer = _layer;
    if (layer == null) return;
    if (key.startsWith('driver:')) {
      _driverPreview.cancel();
      final List<BridgeEffectInstance> drivers;
      try {
        drivers = _driversWith(layer, info.id, values);
      } catch (_) {
        return;
      }
      _commit(_wiringNow(), drivers: drivers);
      return;
    }
    _stack.writeAll(layer, info.id, values);
    _ui?.model.refresh();
  }

  /// A drag tick: the row shows it and the Viewer previews it, and nothing is
  /// committed.
  void _liveParams(String key, BridgeEffectInstanceInfo info,
      Map<String, BridgeEffectValue> values) {
    final ui = _ui;
    final layer = _layer;
    final comp = ui?.selectedComp;
    if (ui == null || layer == null || comp == null) return;
    if (key.startsWith('driver:')) {
      setState(() => _driverDrag = (key: key, values: values));
      _driverPreview.request(() {
        try {
          comp.renderFrameWithDriverPreview(
            frame: ui.playheadFrame.value,
            scale: ui.viewerScale,
            layer: layer,
            drivers: _driversWith(layer, info.id, values),
          );
        } catch (_) {
          // The graph moved under the drag; the release re-reads.
        }
      });
      return;
    }
    setState(() {
      for (final entry in values.entries) {
        _stack.live(comp, layer, info.id, entry.key, entry.value,
            frame: ui.playheadFrame.value, scale: ui.viewerScale);
      }
    });
  }

  /// A press on one of a box's buttons, the same press Effect controls and the
  /// Node panel make.
  void _pressParam(BridgeEffectInstanceInfo info, bool driver,
      UuidValue effect, String param) {
    final ui = _ui;
    final layer = _layer;
    if (ui == null || layer == null) return;
    final comp = ui.selectedComp;
    final frame = ui.playheadFrame.value;
    final home = driver
        ? InstanceHome.drivers(layer)
        : InstanceHome.layer(layer,
            draw: comp == null
                ? null
                : (staged) => comp.renderFrameWithPreview(
                    frame: frame,
                    scale: ui.viewerScale,
                    layer: layer,
                    effects: staged));
    void applied() => ui.model.refresh();
    if (info.name == 'expression' &&
        editExpressionOn(
            context: context,
            home: home,
            effect: effect,
            param: param,
            onApplied: applied)) {
      return;
    }
    if (info.name == 'custom_shader' &&
        pressShaderButton(
            context: context,
            home: home,
            effect: effect,
            param: param,
            onApplied: applied)) {
      return;
    }
    if (pressLensButton(
        context: context,
        home: home,
        effect: effect,
        param: param,
        onApplied: applied)) {
      return;
    }
    if (pressFlareButton(
        context: context,
        home: home,
        effect: effect,
        param: param,
        onApplied: applied,
        comp: comp,
        frame: frame)) {
      return;
    }
    if (info.name == 'node_graph' && param == 'open') {
      final project = Provider.of<LumitState>(context, listen: false).project;
      final inner = graphCompById(project, info.nodeGraphComp);
      if (inner != null) ui.setSelectedComp(inner);
      return;
    }
    if (driver) return;
    try {
      fireEffectAction(layer: layer, effect: effect, param: param, frame: frame);
    } catch (_) {
      // Refused; the effect's own status line says why.
    }
  }
  // --- Committing ---------------------------------------------------------

  /// The wiring as it stands, with the staged positions folded in. Every write
  /// goes through here, so a gesture that moves a box and one that draws a wire
  /// commit the same shape.
  BridgeGraphWiring _wiringNow({
    List<BridgeGraphEdge>? edges,
    List<BridgeNodeRef>? exposed,
    List<BridgeNodeGroup>? groups,
    bool? outUnwired,
  }) {
    final w = _graph!.wiring;
    return BridgeGraphWiring(
      edges: edges ?? w.edges,
      layout: [
        for (final e in _positions.entries)
          if (_refOf(e.key) case final ref?)
            BridgeNodePosition(
                node: ref,
                x: e.value.dx,
                y: e.value.dy,
                width: _widths[e.key]),
      ],
      exposed: exposed ?? w.exposed,
      groups: groups ?? w.groups,
      outUnwired: outUnwired ?? w.outUnwired,
    );
  }

  /// The node reference a position key stands for, or null when the box it
  /// named has gone — a position for a deleted node is dropped rather than
  /// carried, since the engine refuses a layout entry it cannot place.
  BridgeNodeRef? _refOf(String key) => _byKey[key]?.node;

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
  void _connect(GraphSocket a, GraphSocket b, {BridgeGraphEdge? without}) {
    final from = a.isInput ? b : a;
    final to = a.isInput ? a : b;
    final source = _sourceOf(from);
    final dest = _destOf(to);
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

  // --- The image chain's own wires ----------------------------------------

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
  void _chainDrop(int held, GraphSocket? landed, GraphLayout layout,
      {bool drawn = false}) {
    final layer = _layer;
    final chain = _chainOf(layout);
    if (layer == null || held < 1 || held >= chain.length) return;
    final upstream = _effectIdOf(_refOf(chain[held - 1].key));

    if (landed == null) {
      // A wire drawn out of an output and let go of on the ground is a
      // change of mind. Only a wire pulled off an input disconnects.
      if (drawn) return;
      // **Disconnecting is bypassing**. The effect keeps its slot in
      // the stack and stops drawing - which is the state its own enable tick
      // sets, reached from the other side. The Layer out has no effect to
      // bypass: unplugging *it* is the layer itself drawing nothing, which is
      // the one disconnection the graph has to carry a flag for.
      _disconnect(layer, chain[held]);
      return;
    }

    if (!_isChainType(landed.port.portType) || !landed.isInput) return;
    final j = chain.indexWhere((b) => b.key == landed.node);
    if (j < 0) return;
    // **The box the wire lands on comes back on** (owner's rule), and
    // if the wire also says it belongs somewhere else, it moves. A drop back
    // onto the socket the wire came off (`j == held`) is the plain reconnect:
    // no reorder, but the box goes back in the chain.
    final reconnect = _reconnectOf(chain[j]);
    final moved = _effectIdOf(_refOf(chain[j].key));
    final reorders = j != held && (moved != null || upstream != null);
    final project = Provider.of<LumitState>(context, listen: false).project;
    final group = project != null && reconnect != null && reorders;
    if (group) project.beginUndoGroup();
    if (reconnect != null) reconnect();
    if (reorders) {
      if (moved == null) {
        // Dropped on the Layer out: the wire's source becomes the last effect.
        if (upstream != null) _reorderToEnd(layer, upstream);
      } else {
        _reorderAfter(layer, moved, upstream);
      }
    }
    if (group) project.endUndoGroup();
    _ui?.model.refresh();
    _reload();
  }

  /// Take [box] out of the image chain: an effect is bypassed, the Layer out
  /// is unplugged. One commit either way, so one undo step.
  void _disconnect(LayerReference layer, GraphBox box) {
    if (_effectIdOf(_refOf(box.key)) case final victim?) {
      // Already bypassed: a second drop is not a second undo step.
      if (!box.card.enabled) return;
      try {
        for (final instance in layer.getEffects()) {
          if (instance.id() == victim) {
            layer.setEffectEnabled(effect: instance, enabled: false);
            break;
          }
        }
      } catch (_) {
        // Refused, or the stack moved under us; re-reading is the recovery.
      }
    } else if (box.key == 'out') {
      if (_graph!.wiring.outUnwired) return;
      _commit(_wiringNow(outUnwired: true));
    } else {
      return; // The Source makes the picture; it has nothing to unplug.
    }
    _ui?.model.refresh();
    _reload();
  }

  /// What putting [box] back in the chain costs, or null when it is already
  /// in it. Returned as a closure so the caller can decide whether it and a
  /// reorder need one undo group between them.
  void Function()? _reconnectOf(GraphBox box) {
    final layer = _layer;
    if (layer == null) return null;
    if (_effectIdOf(_refOf(box.key)) case final id?) {
      if (box.card.enabled) return null;
      return () {
        try {
          for (final instance in layer.getEffects()) {
            if (instance.id() == id) {
              layer.setEffectEnabled(effect: instance, enabled: true);
              break;
            }
          }
        } catch (_) {
          // Refused, or the stack moved under us; re-reading is the recovery.
        }
      };
    }
    if (box.key == 'out' && _graph!.wiring.outUnwired) {
      return () => _commit(_wiringNow(outUnwired: false));
    }
    return null;
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
  BridgeGraphEdge? _edgeInto(GraphSocket socket) {
    final dest = _destOf(socket);
    if (dest == null) return null;
    for (final e in _graph!.wiring.edges) {
      if (e.to == dest) return e;
    }
    return null;
  }

  /// The socket a stored wire leaves from, as the canvas draws it.
  GraphSocket? _sourceSocket(BridgeGraphEdge edge, GraphLayout layout) {
    final box = layout.byKey[_sourceKey(edge)];
    if (box == null) return null;
    final portId = switch (edge.from) {
      BridgeOutputRef_Driver(:final port) => port,
      BridgeOutputRef_SourceMatte() => 'matte',
      BridgeOutputRef_EffectData(:final port) => port,
      BridgeOutputRef_EffectOutput() => 'output',
    };
    final i = box.outputs.indexWhere((p) => p.id == portId);
    final at = box.socket(portId, false);
    if (i < 0 || at == null) return null;
    return GraphSocket(box.key, box.outputs[i], false, at);
  }

  /// The two ends of a wire, from a socket the canvas is drawing. The
  /// reference behind the key is looked up here, so everything above works in
  /// keys alone.
  BridgeOutputRef? _sourceOf(GraphSocket socket) {
    final node = _refOf(socket.node);
    return node == null ? null : _outputRef(node, socket.port);
  }

  BridgeInputRef? _destOf(GraphSocket socket) {
    final node = _refOf(socket.node);
    return node == null || !socket.isInput
        ? null
        : _inputRef(node, socket.port);
  }

  BridgeOutputRef? _outputRef(BridgeNodeRef node, BridgePort port) {
    if (node is BridgeNodeRef_Source) {
      // The layer's own masked source alpha at that point in the chain — the
      // one feed the graph adds that the Matte row could not offer (§1.4).
      return port.portType == BridgePortType.matte
          ? const BridgeOutputRef.sourceMatte()
          : null;
    }
    if (_driverIdOf(node) case final driver?) {
      return BridgeOutputRef.driver(node: driver, port: port.id);
    }
    // A **stack effect's** declared data output, or its picture, which only
    // ever goes into the Matte of an effect below it.
    if (_effectIdOf(node) case final effect?) {
      return port.portType == BridgePortType.image
          ? BridgeOutputRef.effectOutput(effect: effect)
          : BridgeOutputRef.effectData(effect: effect, port: port.id);
    }
    return null;
  }

  BridgeInputRef? _inputRef(BridgeNodeRef node, BridgePort port) {
    if (port.portType == BridgePortType.matte) {
      final effect = _effectIdOf(node);
      if (effect != null) return BridgeInputRef.matte(effect: effect);
      return null;
    }
    return BridgeInputRef.param(node: node, port: port.id);
  }

  /// Whether these two sockets may be joined, decided **here** from the two
  /// port types both sides carry in the read model. A mismatched drop is
  /// declined without a bridge call: the engine's refusal is the backstop, not
  /// the message channel (docs/17, "The layer graph").
  /// What a wire in hand may snap to: a chain wire only a chain input, which
  /// is all [_chainDrop] takes, and any other wire what [_accepts] takes.
  /// A chain wire drawn out of an effect's output may also go into a Matte.
  bool Function(GraphSocket) _fits(_InFlight flight) => flight.chain == null
      ? (s) => _accepts(flight.from, s)
      : (s) =>
          (s.isInput && _isChainType(s.port.portType)) ||
          (flight.fromOutput && _accepts(flight.from, s));

  bool _accepts(GraphSocket from, GraphSocket to) {
    if (from.isInput == to.isInput) return false;
    final out = from.isInput ? to : from;
    final into = from.isInput ? from : to;
    if (out.node == into.node) return false;
    // An effect's picture into the Matte of an effect below it.
    if (out.port.portType == BridgePortType.image &&
        into.port.portType == BridgePortType.matte) {
      final order = [for (final n in _graph!.nodes) graphNodeKey(n.node)];
      final above = order.indexOf(out.node);
      return _effectIdOf(_refOf(out.node)) != null &&
          above >= 0 &&
          above < order.indexOf(into.node);
    }
    if (_isChainType(out.port.portType) || _isChainType(into.port.portType)) {
      return false;
    }
    // The Layer out box's Audio socket is drawn, unfilled and honest: audio
    // comes only from a footage layer's own stream in this phase. Its
    // Volume socket is the one exception: a number wired there drives
    // the layer's own Volume — the Duck under landing.
    if (into.node == 'out' && into.port.id != 'volume') return false;
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
  bool _wouldLoop(String from, String into) {
    final target = from;
    final seen = <String>{};
    final queue = <String>[into];
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
        BridgeOutputRef_EffectData(:final effect) ||
        BridgeOutputRef_EffectOutput(:final effect) =>
          graphNodeKey(BridgeNodeRef.effect(effect)),
      };

  String _destKey(BridgeGraphEdge edge) => switch (edge.to) {
        BridgeInputRef_Param(:final node) => graphNodeKey(node),
        BridgeInputRef_Matte(:final effect) =>
          graphNodeKey(BridgeNodeRef.effect(effect)),
      };

  // --- Node gestures ------------------------------------------------------

  /// The boxes a command pressed on [node] acts on: the whole pick
  /// when this box is part of it, and this box alone when it is not — the rule
  /// every other surface here follows.
  List<BridgeNodeRef> _targets(String key) => _isPicked(key)
      ? _selection.values.toList()
      : [if (_refOf(key) case final ref?) ref];

  void _toggleExposed(String key) {
    // The pressed box's new state, for all of them, so a pick of mixed twirls
    // comes out even. One `setGraph`, so one undo step however many it is.
    final on = !_graph!.wiring.exposed.any((e) => graphNodeKey(e) == key);
    final targets = {for (final n in _targets(key)) graphNodeKey(n): n};
    _commit(_wiringNow(exposed: [
      for (final e in _graph!.wiring.exposed)
        if (!targets.containsKey(graphNodeKey(e))) e,
      if (on) ...targets.values,
    ]));
  }

  /// Bypass. A driver rides the staged-instance path with `setGraph` as its
  /// commit; an effect rides the stack's own call, exactly as its card in
  /// Effect controls does. One op either way.
  void _toggleBypass(String key, bool enabled) {
    final layer = _layer;
    if (layer == null) return;
    // The pressed box's new state, for every box the press acts on, so
    // a pick of mixed bypasses comes out even.
    final on = !enabled;
    final targets = {for (final n in _targets(key)) graphNodeKey(n)};
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

  /// M on the canvas: the picked boxes are bypassed, or brought back when the
  /// last one picked already is. The Source and the Layer out have no bypass.
  void _mutePicked() {
    for (final key in _selection.keys.toList().reversed) {
      final node = _node(key);
      if (node == null || _derivedNode(node.node)) continue;
      _toggleBypass(key, node.enabled);
      return;
    }
  }

  /// The user's own name for a box, committed the way its bypass is:
  /// a driver stages on the graph's driver list and commits `setGraph`, an
  /// effect stages on the stack and commits `setEffects`. One op, one undo
  /// step, either way — and no new call across the bridge: `set_custom_name`
  /// is already on `BridgeEffectInstance`, which is the handle both lists hand
  /// out.
  ///
  /// An empty name clears back to the box's own label; the engine's own
  /// `set_custom_name` trims and does that, so nothing here has to.
  void _renameNode(String key, String name) {
    setState(() => _renamingNode = null);
    final layer = _layer;
    final node = _refOf(key);
    if (layer == null || node == null) return;
    final driver = _driverIdOf(node);
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
      } else if (_effectIdOf(node) case final effect?) {
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
      // The wires go with them in the same write, and their places in any
      // group.
      _commit(
        _wiringNow(
          edges: kept,
          groups: _bridgeGroups(groupsWithout(victims.map(graphNodeKey))),
        ),
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

  bool _touches(BridgeGraphEdge edge, BridgeNodeRef node) =>
      _touchesKey(edge, graphNodeKey(node));

  bool _touchesKey(BridgeGraphEdge edge, String key) =>
      _sourceKey(edge) == key || _destKey(edge) == key;

  // --- Menus --------------------------------------------------------------

  /// An effect box's page in the manual, asked as its menu opens.
  String? _manualOf(BridgeGraphNode node) {
    if (_effectIdOf(node.node) == null) return null;
    final info =
        listEffects().where((e) => e.name == node.matchName).firstOrNull;
    return info == null
        ? null
        : effectManualUrl(
            name: info.name,
            category: info.category,
            label: info.label,
            namespace: info.namespace);
  }

  /// A box's menu, over the pick as it stands: the right press made an
  /// unpicked box the pick, so every row acts on the whole pick.
  void _boxMenu(String key, Offset at) {
    final graph = _graph;
    final node = _node(key);
    final ui = _ui;
    if (_layer == null || graph == null || node == null || ui == null) return;
    final derived = _derivedNode(node.node);
    final driver = node.node is BridgeNodeRef_Driver;
    final picked = _selection.keys.toSet();
    final effects = _pickedEffects();
    final wired =
        graph.wiring.edges.any((e) => picked.any((k) => _touchesKey(e, k)));
    final enter = switch (node.matchName) {
      'custom_shader' => l10n.shaderEditorTitle,
      'node_graph' => l10n.fxOpenGraph,
      _ => null,
    };
    showGraphBoxMenu(
      context,
      at,
      // The effect's own menu while only effects are picked. A pick with a
      // driver or an end in it keeps the box rows, which act on all of it.
      effect: picked.every((k) => k.startsWith('effect:'))
          ? _effectCommands(key)
          : null,
      bypassed: derived ? null : !node.enabled,
      onBypass: derived ? null : () => _toggleBypass(key, node.enabled),
      onRename: derived ? null : () => setState(() => _renamingNode = key),
      expanded: derived || driver || _isCollapsed(key)
          ? null
          : graph.wiring.exposed.any((e) => graphNodeKey(e) == key),
      onExpand: derived || driver || _isCollapsed(key)
          ? null
          : () => _toggleExposed(key),
      enterLabel: enter,
      onEnter: enter == null ? null : () => _enterBox(node),
      // An effect after this box in the chain, as the wire out of it offers.
      onAddNode: driver || picked.length > 1
          ? null
          : () => _openSearch(
              _boxesNow()
                      .where((b) => b.key == key)
                      .firstOrNull
                      ?.rect
                      .centerRight ??
                  Offset.zero,
              chain: true,
              before: _chainAfter(node.node)),
      onCopy: effects.isEmpty ? null : () => _copyPicked(ui),
      onPaste: () => _pasteEffects(ui),
      canPaste: ui.clipboard.kind == ClipboardKind.effects,
      onDuplicate: effects.isEmpty ? null : _duplicatePicked,
      onGroup: groupPicked,
      onSaveGroup: () => saveGroup(),
      canSaveGroup: canSaveGroup,
      onUngroup: ungroupPicked,
      canUngroup: groups.any((g) => g.members.any(picked.contains)),
      onReveal: switch (_effectIdOf(node.node)) {
        final id? => () => _reveal(_layer!, id),
        null => null,
      },
      onManual: switch (_manualOf(node)) {
        final url? => () => openExternalLink(url),
        null => null,
      },
      // The picture stops at an effect box or the Source; a driver and the
      // Layer out make no point to stop at. The right-click has picked the box,
      // so the chip's point is this one.
      viewHere: ui.viewerPrefixPoint == null ? null : ui.atSelectedEffect.value,
      onViewHere: ui.viewerPrefixPoint == null
          ? null
          : () => ui.setAtSelectedEffect(!ui.atSelectedEffect.value),
      onDisconnectAll: wired ? _disconnectPicked : null,
      collapsed: _isCollapsed(key),
      onCollapse: () => _restyle(key, collapsed: !_isCollapsed(key)),
      colour: _looks?.colours
              .where((c) => graphNodeKey(c.node) == key)
              .firstOrNull
              ?.colour ??
          0,
      onColour: (colour) => _restyle(key, colour: colour),
      onDelete: derived ? null : _deleteSelected,
      locked: ui.model.byId(_layer!.internallayerId)?.info.switches.locked ??
          false,
    );
  }

  /// What the effect behind box [key] can do, through the commands its
  /// headings use, over the picked effects. Null for a box that isn't one.
  EffectCommands? _effectCommands(String key) {
    final ui = _ui;
    final layer = _layer;
    final comp = ui?.selectedComp;
    final info = _infos[key];
    if (ui == null ||
        layer == null ||
        comp == null ||
        info == null ||
        !key.startsWith('effect:')) {
      return null;
    }
    // The commands act on the effect selection, so it is the pick here.
    ui.setEffectSelection(layer, _pickedEffects());
    return EffectCommands(
        info: info, layer: layer, comp: comp, onStackChanged: _effectRowDone);
  }

  /// An effect's row, or the shell's Duplicate, has changed the stack. The
  /// pick follows the effect selection, so Duplicate's copies are picked here
  /// as they are in Effect controls, and the graph is read again.
  void _effectRowDone() {
    final ui = _ui;
    final layer = _layer;
    if (!mounted || ui == null || layer == null) return;
    if (ui.selectedEffectsLayer?.internallayerId == layer.internallayerId) {
      _selection
        ..clear()
        ..addEntries([
          for (final id in ui.selectedEffects.value)
            MapEntry(graphNodeKey(BridgeNodeRef.effect(id)),
                BridgeNodeRef.effect(id)),
        ]);
    }
    Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
    ui.model.refresh();
    _reload();
  }

  /// The effect a box added after [node] goes in front of: the next one in the
  /// stack, or null for the chain's end.
  UuidValue? _chainAfter(BridgeNodeRef node) {
    if (node is BridgeNodeRef_Out) return null;
    List<UuidValue> ids;
    try {
      ids = [for (final e in _layer?.getEffects() ?? const []) e.id()];
    } catch (_) {
      return null;
    }
    final at = switch (node) {
      BridgeNodeRef_Effect(:final field0) => ids.indexOf(field0),
      _ => -1,
    };
    return at + 1 < ids.length ? ids[at + 1] : null;
  }

  /// Each box's colour and fold, read with the graph.
  BridgeNodeLooks? _looks;

  bool _isCollapsed(String key) =>
      _looks?.collapsed.any((n) => graphNodeKey(n) == key) ?? false;

  /// Colour or fold the pressed box, and the rest of the pick with it, as one
  /// undo step.
  void _restyle(String key, {int? colour, bool? collapsed}) {
    final layer = _layer;
    if (layer == null) return;
    final nodes = _targets(key);
    try {
      if (colour != null) {
        layer.setGraphBoxColour(nodes: nodes, colour: colour);
      }
      if (collapsed != null) {
        layer.setGraphBoxCollapsed(nodes: nodes, collapsed: collapsed);
      }
    } catch (_) {
      // The layer moved under us. Re-reading is the recovery.
    }
    _ui?.model.refresh();
    _reload();
  }

  /// Reveal in Effect controls: the shell opens the panel, fronts it and
  /// hands it the effect to pick and scroll to.
  void _reveal(LayerReference layer, UuidValue effect) =>
      _ui?.revealEffectCard(layer, effect);

  /// An effect's Reveal in node graph: the box is brought to the middle of the
  /// canvas on the next layout, at the zoom in force.
  void _onFrameAsked() {
    final node = _ui?.frameBoxRequest.value;
    if (!mounted || node == null || _showsCompGraph) return;
    _ui!.frameBoxRequest.value = null;
    setState(() => _frameKey = graphNodeKey(node));
  }

  /// The box the next layout centres on, or null.
  String? _frameKey;

  /// A socket's menu: its wire off, and the box's unused sockets shown or
  /// hidden. The chain's wires are the effect list and are not stored, so
  /// a chain socket has nothing to take off.
  void _socketMenu(GraphSocket socket, Offset at) {
    final graph = _graph;
    final node = _node(socket.node);
    if (graph == null || node == null) return;
    VoidCallback? disconnect;
    if (socket.isInput) {
      if (_edgeInto(socket) case final edge?) {
        disconnect = () => _removeEdge(edge);
      }
    } else if (_sourceOf(socket) case final source?
        when graph.wiring.edges.any((e) => e.from == source)) {
      disconnect = () => _commit(_wiringNow(edges: [
            for (final e in graph.wiring.edges)
              if (e.from != source) e,
          ]));
    }
    final twirl =
        !_derivedNode(node.node) && node.node is! BridgeNodeRef_Driver;
    showGraphSocketMenu(
      context,
      at,
      onDisconnect: disconnect,
      expanded: twirl
          ? graph.wiring.exposed.any((e) => graphNodeKey(e) == socket.node)
          : null,
      onExpand: twirl ? () => _toggleExposed(socket.node) : null,
    );
  }

  /// The wire nearest [at] within a wire's grab distance, with its stored
  /// edge, which is null for one of the picture's chain.
  (TieWire, BridgeGraphEdge?)? _wireAt(GraphLayout layout, Offset at) {
    final graph = _graph;
    if (graph == null) return null;
    final all = _layerWires(layout, graph.wiring.edges,
        outUnwired: graph.wiring.outUnwired);
    final wires = [for (final (w, _, _) in all) w];
    (TieWire, BridgeGraphEdge?)? best;
    var nearest = graphWireGrab;
    for (final (w, edge, _) in all) {
      final d = tieWireDistance(w, at, wires);
      if (d < nearest) {
        nearest = d;
        best = (w, edge);
      }
    }
    return best;
  }

  /// A wire's menu. A chain wire takes an effect into the chain where it
  /// runs; a stored wire takes a driver between its two ends, and can be
  /// taken off.
  void _wireMenu(GraphLayout layout, TieWire wire, BridgeGraphEdge? edge,
      Offset at, Offset position) {
    // A chain wire's key names the box it feeds; the Layer out is no effect,
    // so a wire into it inserts at the chain's end.
    final fed = edge == null
        ? _effectIdOf(_refOf(wire.key.substring(0, wire.key.indexOf('/'))))
        : null;
    // A stored wire's input end, which the search holds in hand.
    GraphSocket? into;
    if (edge != null) {
      final (key, port) = _inputEnd(edge);
      final box = layout.byKey[key];
      final socket = box?.inputs.where((p) => p.id == port).firstOrNull;
      final centre = box?.socket(port, true);
      if (socket != null && centre != null) {
        into = GraphSocket(key, socket, true, centre);
      }
    }
    showGraphWireMenu(
      context,
      position,
      onInsert: edge == null
          ? () => _openSearch(at, chain: true, before: fed)
          : into == null
              ? null
              : () => _openSearch(at, wire: into, feed: edge.from),
      onReroute: () => tieAddPoint(wire.key, at),
      onDelete: edge == null ? null : () => _removeEdge(edge),
    );
  }

  void _canvasMenu(GraphLayout layout, Offset at, Offset position) {
    final ui = _ui;
    if (ui == null) return;
    final nodes = [
      for (final n in _graph?.nodes ?? const <BridgeGraphNode>[]) n.node
    ];
    showGraphCanvasMenu(
      context,
      position,
      onAddNode: () => _openSearch(at),
      onPaste: () => _pasteEffects(ui),
      canPaste: ui.clipboard.kind == ClipboardKind.effects,
      onSelectAll: () => setState(() => _pick(nodes)),
      onFrameSelected: () => _frameSelected(_viewport, layout),
      canFrameSelected: _selection.isNotEmpty,
      onFrameAll: () => _frameAll(_viewport, layout),
      onAddNote: addNote,
      onGroup: groupPicked,
      canGroup: canGroup,
      onCollapseAll: nodes.isEmpty ? null : () => _foldAll(nodes, true),
      canCollapseAll: nodes.any((n) => !_isCollapsed(graphNodeKey(n))),
      onExpandAll: nodes.isEmpty ? null : () => _foldAll(nodes, false),
      canExpandAll: nodes.any((n) => _isCollapsed(graphNodeKey(n))),
    );
  }

  /// Fold or open every box, as one undo step.
  void _foldAll(List<BridgeNodeRef> nodes, bool collapsed) {
    try {
      _layer?.setGraphBoxCollapsed(nodes: nodes, collapsed: collapsed);
    } catch (_) {
      // The layer moved under us. Re-reading is the recovery.
    }
    _ui?.model.refresh();
    _reload();
  }

  /// Every stored wire off the picked boxes, in one write. The chain's wires
  /// are the effect list and stay.
  void _disconnectPicked() {
    final picked = _selection.keys.toSet();
    _commit(_wiringNow(edges: [
      for (final e in _graph!.wiring.edges)
        if (!picked.any((k) => _touchesKey(e, k))) e,
    ]));
  }

  // --- Copy and paste -----------------------------------------------------

  /// The picked effects, in stack order. A driver has no clipboard shape.
  List<UuidValue> _pickedEffects() => [
        for (final node in _graph?.nodes ?? const <BridgeGraphNode>[])
          if (_selection.containsKey(graphNodeKey(node.node)))
            if (_effectIdOf(node.node) case final effect?) effect,
      ];

  /// Copy the picked effects, as the Effect controls heading does.
  bool _copyPicked(LumitUiState ui) {
    final layer = _layer;
    final ids = _pickedEffects();
    if (layer == null || ids.isEmpty) return false;
    try {
      ui.copyEffectsToClipboard(layer.copyEffects(effects: ids));
    } catch (_) {
      return false;
    }
    return true;
  }

  /// Paste the effects on the clipboard onto the layer; they join the
  /// chain's end. One op.
  bool _pasteEffects(LumitUiState ui) {
    final layer = _layer;
    final text = ui.clipboard.text;
    if (layer == null ||
        text == null ||
        ui.clipboard.kind != ClipboardKind.effects) {
      return false;
    }
    try {
      layer.pasteEffects(text: text, atFrame: ui.playheadFrame.value);
    } catch (_) {
      return false;
    }
    Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
    ui.model.refresh();
    _reload();
    return true;
  }

  /// The picked effects again, each copy after its original and picked in its
  /// place: the effect menu's Duplicate. The clipboard is left as it was.
  void _duplicatePicked() {
    final ids = _pickedEffects();
    if (ids.isEmpty) return;
    _effectCommands(graphNodeKey(BridgeNodeRef.effect(ids.first)))
        ?.duplicate(context);
  }

  // --- The console --------------------------------------------------------

  /// Ctrl+Space, and a wire let go over empty canvas, open **the console** —
  /// the same popover the shell opens, with a foot line
  /// saying what a row will do. One search surface, two doors: what the
  /// canvas contributes is the list, the spot the box lands on, and the
  /// sentence. Tab, Shift+A and a right-click are the Ctrl+Space door.
  Future<void> _openSearch(Offset at,
      {GraphSocket? wire,
      bool chain = false,
      UuidValue? before,
      BridgeOutputRef? feed}) async {
    if (_searching) return;
    setState(() => _searching = true);
    try {
      await showFxConsoleFrb(
        context: context,
        anchor: lastKnownPointerPosition,
        model: _searchModel(at,
            wire: wire, chain: chain, before: before, feed: feed),
      );
    } finally {
      if (mounted) setState(() => _searching = false);
    }
  }

  /// What the console lists over this canvas, for a box that would land on
  /// [at] and ride [wire] when there is one in hand. Asked from a wire of
  /// the picture's chain ([chain]), it lists the effects alone, and the one
  /// chosen goes into the chain just before [before].
  FxConsoleModel _searchModel(Offset at,
      {GraphSocket? wire,
      bool chain = false,
      UuidValue? before,
      BridgeOutputRef? feed}) {
    final all = (widget.driversLister ?? listDrivers)();
    // `listEffects` carries the drivers too; here they come from the
    // drivers listing instead, because the canvas's own add places the box on
    // the drop spot and rides the wire in one commit.
    final driverNames = {for (final driver in all) driver.name};
    return FxConsoleModel(
      // A wire summoned this by a drop, not by a key, so it wears none.
      keyHint: wire == null ? _ui?.keymap.chordFor('console.open') : null,
      entries: [
        // The drivers first — the graph's own family. With a wire in hand
        // the list is the entries that wire could actually land on, which
        // is what makes the foot's sentence true: pick one and it is
        // connected.
        for (final driver in all)
          if (!chain && (wire == null || _fitsWire(driver, wire)))
            FxConsoleEntry(
              label: engineLabel(driver.label),
              kind: FxConsoleKind.effect,
              group: engineLabel(driver.categoryLabel),
              run: () => _addDriver(driver, at, wire, feed: feed),
            ),
        // Then every effect: chosen, it joins the layer's stack,
        // and the stack is the chain — so the box appears wired into the
        // picture's own path, auto-wired by construction. Only
        // with no wire in hand: a dragged wire is a value looking for a
        // socket, and the chain's sockets take no wire.
        if (wire == null)
          for (final effect in listEffects())
            if (!driverNames.contains(effect.name))
              FxConsoleEntry(
                label: engineLabel(effect.label),
                kind: FxConsoleKind.effect,
                group: engineLabel(effect.categoryLabel),
                run: () => chain
                    ? _addEffectBefore(effect.name, before)
                    : _addEffect(effect.name),
              ),
        // The saved groups, beside the drivers they are made of.
        // Only with no wire in hand: a group is a rig, not a socket, so
        // there is nothing for the wire to land on.
        if (wire == null && !chain)
          for (final saved in (widget.groupsLister ?? listNodeGroups)())
            FxConsoleEntry(
              label: saved.name,
              kind: FxConsoleKind.effect,
              group: l10n.graphGroup,
              run: () => _insertGroup(saved.path, at),
            ),
      ],
    );
  }

  /// What takes the shown search down, while it is up.
  VoidCallback? _exhibitDown;

  /// The search shown and not used ([LumitUiState.nodeSearchExhibit]): the
  /// console with this canvas's own list, over the middle of the canvas and
  /// near its top so the list has the canvas to open over. Nothing is shown
  /// while the panel's face is something other than a layer's graph.
  void _onSearchExhibit() {
    _exhibitDown?.call();
    _exhibitDown = null;
    final ui = _ui;
    if (!mounted || ui == null || !ui.nodeSearchExhibit.value) return;
    final canvas = _canvasKey.currentContext?.findRenderObject();
    if (_graph == null || canvas is! RenderBox || !canvas.hasSize) return;
    _exhibitDown = showFxConsoleExhibitFrb(
      context: context,
      anchor: canvas
          .localToGlobal(Offset(canvas.size.width / 2, canvas.size.height / 5)),
      model: _searchModel(graphClearSpot(
        _toCanvas(Offset(_viewport.width / 2, _viewport.height / 2)),
        [for (final b in _boxesNow()) b.rect],
      )),
    );
  }

  /// The stack's own add, which is the graph's add for an effect: the chain
  /// *is* the list, so the new box appears at the chain's end with the
  /// picture's wires already on it, and the op is the one the Effect menu
  /// commits.
  void _addEffect(String name) {
    final layer = _layer;
    if (layer == null || !(_ui?.addEffect(layer, name) ?? false)) return;
    _reload();
  }

  /// An effect into the chain just before [before], or at the chain's end
  /// when the wire ran into the Layer out. The stack's add puts it last, so
  /// moving it up is a second write, wrapped with the first as one undo step.
  void _addEffectBefore(String name, UuidValue? before) {
    final layer = _layer;
    if (layer == null) return;
    final project = Provider.of<LumitState>(context, listen: false).project;
    if (before != null) project?.beginUndoGroup();
    try {
      if ((_ui?.addEffect(layer, name) ?? false) && before != null) {
        final stack = layer.getEffects();
        final at = stack.indexWhere((i) => i.id() == before);
        if (at >= 0 && at < stack.length - 1) {
          final added = stack.removeLast();
          stack.insert(at, added);
          layer.setEffects(effects: stack);
        }
      }
    } catch (_) {
      // The stack moved under us; what was added stands where it landed.
    }
    if (before != null) project?.endUndoGroup();
    _ui?.model.refresh();
    _reload();
  }

  /// An effect dragged in from Effects & presets goes where the console puts
  /// it: a driver lands where it was let go, anything else joins the stack.
  void _dropped(EffectDragData data, Offset global) {
    final box = _canvasKey.currentContext?.findRenderObject();
    if (box is! RenderBox || _graph == null) return;
    final driver = (widget.driversLister ?? listDrivers)()
        .where((d) => d.name == data.name)
        .firstOrNull;
    if (driver != null) {
      _addDriver(driver, _toCanvas(box.globalToLocal(global)), null);
    } else {
      _addEffect(data.name);
    }
  }

  /// Where the pointer is on this canvas, in canvas units, or null when it is
  /// somewhere else.
  Offset? get _pointerOnCanvas {
    final local = graphPointerIn(_canvasKey);
    return local == null ? null : _toCanvas(local);
  }

  // --- Named groups -------------------------------------------------------

  @override
  List<GraphGroup> get groups => [
        for (final g
            in _graph?.wiring.groups ?? const <BridgeNodeGroup>[])
          GraphGroup(g.name, g.colour, g.members.map(graphNodeKey).toList()),
      ];

  List<BridgeNodeGroup> _bridgeGroups(List<GraphGroup> groups) => [
        for (final g in groups)
          BridgeNodeGroup(name: g.name, colour: g.colour, members: [
            for (final key in g.members)
              if (_refOf(key) case final ref?) ref,
          ]),
      ];

  @override
  void commitGroups(List<GraphGroup> next) {
    if (_graph == null) return;
    // A box nobody has dragged has no place of its own in the document, and a
    // saved group keeps each box's place. So a box in a group is given the
    // one it is drawn at.
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
          if (_refOf(key) case final ref?) ref,
      ]);

  /// The Source and the Layer out are the layer's own, so a saved group
  /// leaves them behind.
  @override
  bool groupCarries(String key) => switch (_refOf(key)) {
        BridgeNodeRef_Driver() || BridgeNodeRef_Effect() => true,
        _ => false,
      };

  @override
  String groupText(String name, int colour, List<String> members) =>
      _layer!.saveNodeGroup(name: name, colour: colour, nodes: [
        for (final key in members)
          if (_refOf(key) case final ref?) ref,
      ]);

  @override
  String get groupExtension => graphGroupExtension;

  @override
  String? get groupLibrary =>
      widget.groupLibrary != null ? widget.groupLibrary!() : super.groupLibrary;

  @override
  List<BridgePresetInfo> savedGroups() =>
      (widget.groupsLister ?? listNodeGroups)();

  @override
  void insertSavedGroup(String path) => _insertGroup(path, noteSpot);

  /// The held notes, read with the graph.
  List<BridgeGraphNote> _notes = const [];

  @override
  List<BridgeGraphNote> get notes => _notes;

  @override
  void commitNotes(List<BridgeGraphNote> next) {
    try {
      _layer?.setGraphNotes(notes: next);
    } catch (_) {
      // Refused, or the layer moved under us. Re-reading is the recovery.
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
      _layer?.setGraphTies(ties: next);
    } catch (_) {
      // Refused, or the layer moved under us. Re-reading is the recovery.
    }
    _ui?.model.refresh();
    _reload();
  }

  @override
  double get tieZoom => _zoom;

  List<TieWire> _wiresOf(GraphLayout layout) => [
        for (final (w, _, _) in _layerWires(
            layout, _graph?.wiring.edges ?? const [],
            held: _flight?.chain,
            outUnwired: _graph?.wiring.outUnwired ?? false))
          w,
      ];

  @override
  Offset get noteSpot => graphClearSpot(
        _toCanvas(Offset(_viewport.width / 2, _viewport.height / 2)),
        [for (final b in _boxesNow()) b.rect],
      );

  @override
  bool get groupSnaps => _snapToGrid;

  @override
  double get groupZoom => _zoom;

  @override
  LumitUiState? get groupUi => _ui;

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
  /// [feed] is the source a stored wire carried before the new box took its
  /// input end: the box goes between the two, fed from it where it has an
  /// input of the wire's type.
  void _addDriver(BridgeEffectInfo info, Offset at, GraphSocket? wire,
      {BridgeOutputRef? feed}) {
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
      if (feed != null && wire != null) {
        final takes = info.inputs
            .where((p) => p.portType == wire.port.portType)
            .firstOrNull;
        final dest = takes == null ? null : _inputRef(ref, takes);
        if (dest != null) edges.add(BridgeGraphEdge(from: feed, to: dest));
      }
    }
    try {
      layer.setGraph(
        drivers: [...layer.getGraphDrivers(), made],
        wiring: BridgeGraphWiring(
          edges: edges,
          layout: [
            ..._wiringNow().layout,
            BridgeNodePosition(node: ref, x: at.dx, y: at.dy, width: null),
          ],
          exposed: _graph!.wiring.exposed,
          groups: _graph!.wiring.groups,
          outUnwired: _graph!.wiring.outUnwired,
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
    GraphSocket wire,
  ) {
    final held = _refOf(wire.node);
    if (held == null) return null;
    // A wire let go of an output looks for an input on the new box, and the
    // other way about.
    for (final port in wire.isInput ? info.outputs : info.inputs) {
      final socket =
          GraphSocket(graphNodeKey(added), port, !wire.isInput, Offset.zero);
      if (!_accepts(wire, socket)) continue;
      final source = wire.isInput
          ? _outputRef(added, port)
          : _outputRef(held, wire.port);
      final dest = wire.isInput
          ? _inputRef(held, wire.port)
          : _inputRef(added, port);
      if (source == null || dest == null) continue;
      return BridgeGraphEdge(from: source, to: dest);
    }
    return null;
  }

  /// Whether any socket this catalogue entry declares could take the wire in
  /// hand — the same type rule [_accepts] applies on the canvas, asked of a box
  /// that does not exist yet. It is what the Tab search filters by, so picking
  /// an entry from the list can never fail to connect.
  bool _fitsWire(BridgeEffectInfo info, GraphSocket wire) =>
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
  ({BridgeGraphEdge edge, GraphSocket into, GraphSocket outOf})? _dropInsert(
      GraphLayout layout) {
    final drag = _nodeDrag;
    final graph = _graph;
    // One box, and only a box nothing is joined to: dropping a wired node on a
    // wire would ask what happens to the wires it already has, and the answer
    // "nothing" would be a surprise either way.
    if (drag == null || graph == null || drag.origins.length != 1) return null;
    final box = layout.byKey[drag.key];
    if (box == null) return null;
    if (graph.wiring.edges.any((e) => _touchesKey(e, box.key))) return null;

    // Where the box is *now*, which the held layout is a frame behind on.
    final at = (_positions[drag.key] ?? box.rect.topLeft) +
        Offset(box.rect.width / 2, box.rect.height / 2);

    // Measured along each wire as its ties route it.
    final wires = _wiresOf(layout);
    for (final edge in graph.wiring.edges) {
      final ends = _edgeEnds(layout, edge);
      if (ends == null) continue;
      final wire = (key: tieWireKey(_inputEnd(edge).$1, _inputEnd(edge).$2), from: ends.$1, to: ends.$2);
      if (tieWireDistance(wire, at, wires) > graphWireGrab) continue;
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
  GraphSocket? _freeSocket(GraphBox box, BridgePortType type,
      {required bool isInput}) {
    for (final port in isInput ? box.inputs : box.outputs) {
      if (_isChainType(port.portType) || port.portType != type) continue;
      if (isInput && port.wired) continue;
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
    // A press while the console is up never reaches here: it floats over the
    // canvas in the overlay and its own click-away layer catches the click
    // that means "never mind".
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
    // The right button asks a socket rather than pulling on it.
    if (socket != null && event.buttons == kSecondaryMouseButton) {
      _socketMenu(socket, event.position);
      return;
    }
    if (socket != null && _isChainType(socket.port.portType)) {
      // **The image chain's wires can be picked up too** (owner item
      // 10: "connections between effect boxes can't be removed"). Pressing a
      // chain *input* takes hold of the wire feeding it, by its far end,
      // exactly as a stored wire is grabbed — drop it on another chain input
      // to re-route (a stack reorder), or on empty canvas to take the fed box
      // out of the chain. A chain *output* still takes no drag: one output
      // feeding one input is the list's own law, and a fresh chain wire is
      // not a thing that can exist.
      if (socket.isInput) {
        final chain = _chainOf(layout);
        final i = chain.indexWhere((b) => b.key == socket.node);
        if (i >= 1) {
          final upstream = chain[i - 1];
          final portId = i - 1 == 0 ? 'image' : 'output';
          final k = upstream.outputs.indexWhere((p) => p.id == portId);
          final from = upstream.socket(portId, false);
          if (k >= 0 && from != null) {
            setState(() => _flight = _InFlight(
                GraphSocket(upstream.key, upstream.outputs[k], false, from),
                at,
                chain: i));
            return;
          }
        }
      }
      // **A chain output draws a wire too** (owner, 2026-09-01: "you should
      // be able to drag wires from one output"). Pressing an image output
      // takes a wire in hand from the box that makes the picture; dropping it
      // on another box's image input puts this box immediately before that
      // one, which is the same stack reorder the input grab already lowers to.
      // Without this the gesture everybody tries first - output to input - did
      // nothing at all, and the press moved the box instead.
      final chain = _chainOf(layout);
      final i = chain.indexWhere((b) => b.key == socket.node);
      if (i >= 0 && i < chain.length - 1) {
        setState(() => _flight =
            _InFlight(socket, at, chain: i + 1, fromOutput: true));
        return;
      }
      // Fall through: the Layer out has no output, so its press is a box press.
    }
    // **A box's left or right edge drags its width**, as node editors have it;
    // a socket on the edge still wins the press.
    if (socket == null && event.buttons == kPrimaryMouseButton) {
      if (graphEdgeAt(layout, at, _zoom) case final edge?) {
        setState(() => _edgeDrag = GraphEdgeDrag(
            edge.box.key, edge.left, at.dx, edge.box.rect,
            open: edge.box.hasControls));
        return;
      }
    }
    if (socket != null && !_isChainType(socket.port.portType)) {
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
      final key = box.key;
      final node = _node(key);
      // **Double-clicking a Custom shader box enters its inner graph**, and a
      // Node graph box opens the composition it applies: entering either works
      // like entering a precomp. Counted with [DoubleTap] because the canvas
      // reads raw pointers; the first press still picks the box, exactly as a
      // precomp's first click selects.
      // A right press picks and grabs as the left does, and a release that
      // never moved opens the box's menu. It counts for no double-click.
      final right = event.buttons == kSecondaryMouseButton;
      final again = !right &&
          _boxTaps.tap(at: event.localPosition, slop: 6) &&
          _boxTapKey == key;
      _boxTapKey = key;
      if (again && node != null) {
        if (_enterBox(node)) return;
      }
      final keys = HardwareKeyboard.instance;
      final toggle = !right && (keys.isControlPressed || keys.isMetaPressed);
      final add = !right && keys.isShiftPressed;
      final held = _isPicked(key);
      _menuBox = right ? key : null;
      setState(() {
        // **Click replaces, Ctrl toggles, Shift adds** — the three rules a
        // layer row, a project row and an effect heading all follow, because a
        // selection that behaved one way here and another there would be two
        // selections to learn.
        if (node == null) {
          // The box has gone since the layout was made; nothing to pick.
        } else if (toggle) {
          if (_selection.remove(key) == null) {
            _selection[key] = node.node;
          }
          _publishPick();
        } else if (add) {
          _selection[key] = node.node;
          _publishPick();
        } else if (!held) {
          _pick([node.node]);
        }
        // A press inside something already picked takes the whole pick with
        // it, so several boxes move together (docs/07 §4.5). A release that
        // never moved collapses it back to this box, which is what the plain
        // click above would have done.
        final moving = held && !toggle && !add
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
          collapse: held && !toggle && !add && !right,
        );
      });
      return;
    }

    // **Empty canvas.** The primary button sweeps a selection box; the middle
    // button pans, which is where panning went when the box took the drag it
    // used to have. A press with no modifier clears, exactly as it always did;
    // an additive one keeps what is picked and adds the catch to it.
    if (event.buttons == kMiddleMouseButton) {
      setState(() => _panFrom = _pan - event.localPosition);
      return;
    }
    // The right button opens a menu, or the console, on release, and a
    // right-button drag ties the wires it crosses.
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

  /// A Custom shader box's picture of its inner graph, which a click opens.
  /// Null for every other box.
  Widget? _shaderThumb(String key) {
    final layer = _layer;
    final node = _node(key);
    if (layer == null || node == null || node.matchName != 'custom_shader') {
      return null;
    }
    final effect = _effectIdOf(node.node);
    if (effect == null) return null;
    return ShaderGraphThumb(
      home: InstanceHome.layer(layer),
      effect: effect,
      height: graphShaderThumbHeight,
      onOpen: () => _enterBox(node),
    );
  }

  /// A double-click on a box with an inside: a Custom shader opens its inner
  /// graph, a Node graph box opens the composition it applies. False for every
  /// other box, which is a plain press.
  bool _enterBox(BridgeGraphNode node) {
    final layer = _layer;
    final effect = _effectIdOf(node.node);
    if (layer == null || effect == null) return false;
    if (node.matchName == 'custom_shader') {
      _ui?.enterShaderGraph(layer, effect,
          effectName: node.customName ?? engineLabel(node.label));
      return true;
    }
    if (node.matchName != 'node_graph') return false;
    UuidValue? bound;
    try {
      for (final instance in layer.getEffects()) {
        if (instance.id() == effect) {
          bound = instance.nodeGraphCompId();
          break;
        }
      }
    } catch (_) {
      return false;
    }
    final comp = graphCompById(
        Provider.of<LumitState>(context, listen: false).project, bound);
    if (comp == null) return false;
    _ui?.setSelectedComp(comp);
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
        flight.snap = layout.snapAt(at, _fits(flight));
      });
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
      final landed = layout.snapAt(at, _fits(flight)) ?? layout.socketAt(at);
      if (flight.chain case final held?) {
        // Out of an effect's output into a Matte below: a wire, not a move.
        if (moved &&
            flight.fromOutput &&
            landed != null &&
            landed.port.portType == BridgePortType.matte &&
            _accepts(flight.from, landed)) {
          _connect(flight.from, landed);
          return;
        }
        // A press that never travelled is no gesture at all here: a stored
        // wire unplugs on a stationary click, but a chain wire's discard
        // costs an effect, and that must never be the price of a slip.
        if (moved) {
          _chainDrop(held, landed, layout, drawn: flight.fromOutput);
        }
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
      // Boxes let go over a group's frame move into it, in the same write
      // as their new place.
      final joined = groupsAfterDrop(moved ? drag.origins.keys : const [], at);
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
      final menuBox = _menuBox;
      _menuBox = null;
      if (!moved && menuBox != null) {
        _boxMenu(menuBox, event.position);
        return;
      }
      if (insert != null) {
        final source = _sourceOf(insert.outOf);
        final dest = _destOf(insert.into);
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
      if (_wireAt(layout, at) case final hit?) {
        _wireMenu(layout, hit.$1, hit.$2, at, event.position);
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
        // **Wholly inside**, the house rule for every rubber band here
        // (docs/07 §4.5): a box the band clipped a corner off was not what the
        // gesture pointed at.
        final band = Rect.fromPoints(_toCanvas(from), _toCanvas(to));
        final caught = [
          for (final box in layout.boxes)
            if (band.contains(box.rect.topLeft) &&
                band.contains(box.rect.bottomRight))
              if (_refOf(box.key) case final ref?) ref,
        ];
        _pick([if (adds) ..._selection.values, ...caught]);
      });
      return;
    }
    setState(() => _panFrom = null);
  }

  /// Fit every box on screen, with a little air round the edges. Asked for
  /// by hand, so the panel's own resizes leave the view be after it.
  void _frameAll(Size viewport, GraphLayout layout) {
    _viewMoved = true;
    _frame(graphFrame(viewport, [for (final b in layout.boxes) b.rect]));
  }

  /// The picked boxes, or every box when none is picked.
  void _frameSelected(Size viewport, GraphLayout layout) {
    _viewMoved = true;
    _frame(graphFrame(viewport, [
      for (final b in layout.boxes)
        if (_selection.isEmpty || _selection.containsKey(b.key)) b.rect
    ]));
  }

  void _frame(({Offset pan, double zoom})? view) {
    if (view == null) return;
    setState(() {
      _zoom = view.zoom;
      _pan = view.pan;
    });
  }

  void _wheel(PointerSignalEvent event) {
    if (event is! PointerScrollEvent) return;
    final was = _zoom;
    final next = (was * (event.scrollDelta.dy > 0 ? 0.9 : 1.1)).clamp(0.2, 2.0);
    if (next == was) return;
    // Zoom about the pointer, so what is under it stays under it.
    final anchor = (event.localPosition - _pan) / was;
    _viewMoved = true;
    setState(() {
      _zoom = next;
      _pan = event.localPosition - anchor * next;
    });
  }

  // --- Drawing ------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Inside a Custom shader: the panel's whole face is the
    // inner graph, breadcrumb and all, until Escape or a crumb brings the
    // layer's own graph back.
    if (_ui?.shaderGraphEntry.value case final entry?) {
      return ShaderGraphPanel(
        entry: entry,
        onExit: () => _ui?.exitShaderGraph(),
      );
    }
    // **The third subject** (docs/impl/node-graph-comp.md §4.2): a composition
    // whose picture is made by boxes and wires. Read off the held model, never
    // asked for, and before the layer branch because a node graph has no
    // layers for that branch to draw.
    if (_ui case final ui?) {
      if (ui.selectedComp case final comp? when _showsCompGraph) {
        return CompGraphPanel(
          comp: comp,
          nodesLister: widget.graphNodesLister,
          effectsLister: widget.effectsLister,
        );
      }
    }
    final graph = _graph;
    if (graph == null) {
      return PlaceholderPanel(
        icon: LumitIcon.nodes,
        title: l10n.panelGraph,
        hint: l10n.graphNoLayer,
      );
    }

    final layout = _layoutOf(
      graph,
      _positions,
      _widths,
      graph.wiring.exposed.map(graphNodeKey).toSet(),
      _layerName,
      _params,
      looks: _looks,
    );
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
          onFrameSelected: () => _frameSelected(_viewport, layout),
          canFrameSelected: _selection.isNotEmpty,
          onFrameAll: () => _frameAll(_viewport, layout),
        ),
        child: _toolbar(t, layout),
      );

  Widget _toolbar(LumitTheme t, GraphLayout layout) => Container(
        key: const ValueKey('graph-toolbar'),
        height: graphToolbarHeight,
        color: t.surface1,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        child: Row(
          children: [
            Expanded(
              // The layer's name is the layer: its menu, as anywhere a layer
              // is shown.
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onSecondaryTapUp: (d) {
                  if (_layer case final layer?) {
                    showLayerRowMenu(
                        context, layer.internallayerId, d.globalPosition);
                  }
                },
                child: Text(_layerName,
                    key: const ValueKey('graph-layer-name'),
                    style: t.body.copyWith(color: t.textMuted),
                    overflow: TextOverflow.ellipsis),
              ),
            ),
            // The controls fold into one mark when the panel is narrow. The
            // three switches wear the magnet's dress: on is the glyph on the
            // button's own face, off is frameless and muted.
            graphToolbarFold(
              markKey: const ValueKey('graph-toolbar-more'),
              [
                ...groupFoldItems(t),
                graphToolFold(t,
                    key: const ValueKey('graph-auto-wire'),
                    tip: l10n.graphAutoWire,
                    icon: LumitIcons.autoWire,
                    on: _autoWire,
                    onPressed: () => setState(() => _autoWire = !_autoWire)),
                graphToolFold(t,
                    key: const ValueKey('graph-heal'),
                    tip: l10n.graphHeal,
                    icon: LumitIcons.heal,
                    on: _heal,
                    onPressed: () => setState(() => _heal = !_heal)),
                graphToolFold(t,
                    key: const ValueKey('graph-snap'),
                    tip: _snapToGrid ? l10n.tipSnapOn : l10n.tipSnapOff,
                    icon: LumitIcons.snap,
                    on: _snapToGrid,
                    gap: 10,
                    onPressed: () =>
                        setState(() => _snapToGrid = !_snapToGrid)),
                graphFrameAllFold(t,
                    key: const ValueKey('graph-frame-all'),
                    onPressed: () => _frameAll(_viewport, layout)),
              ],
            ),
            Text(
              l10n.graphZoom((_zoom * 100).round()),
              key: const ValueKey('graph-zoom'),
              style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
            ),
          ],
        ),
      );

  Size _viewport = Size.zero;

  /// The next layout sets the view: on the first, and when the layer changes.
  bool _frameNext = true;

  /// The panel size the view was last set for. Until the user pans or zooms,
  /// a new size frames the boxes again, as the node graph composition does.
  Size? _framedFor;
  bool _viewMoved = false;

  Widget _canvas(LumitTheme t, GraphLayout layout, Size size) {
    _viewport = size;
    final cut = GraphCutFade.cuts(
        size, [for (final b in layout.boxes) b.rect], _pan, _zoom);
    if (_frameKey case final key? when layout.byKey.containsKey(key)) {
      _frameKey = null;
      _frameNext = false;
      _viewMoved = true;
      final box = layout.byKey[key]!.rect;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        // At rest, not at the zoom a framed-all graph left: the box is what
        // was asked for, readable.
        // A quarter in from the left, with the box's chain running on to the
        // right: a box's card is wider than its collapsed face, so centring
        // the card pushed the face to the edge.
        setState(() {
          _zoom = 1;
          _pan = Offset(size.width / 4 - box.left, size.height / 2 - box.center.dy);
        });
      });
    }
    if (layout.boxes.isNotEmpty &&
        (_frameNext || (!_viewMoved && _framedFor != size))) {
      _frameNext = false;
      _framedFor = size;
      // The boxes open in the middle of the panel, never larger than they
      // are. After the frame, as a view change is.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        final rects = [for (final b in layout.boxes) b.rect];
        final fit = graphFrame(size, rects, maxZoom: 1);
        // Never too small to read: a chain longer than the panel opens on
        // its start and runs on under the fade.
        if (fit != null && fit.zoom < _openFloor) {
          final all = rects.reduce((a, b) => a.expandToInclude(b));
          _frame((
            zoom: _openFloor,
            pan: Offset(20 - all.left * _openFloor,
                size.height / 2 - all.center.dy * _openFloor),
          ));
        } else {
          _frame(fit);
        }
      });
    }
    return Focus(
      focusNode: _canvasFocus,
      onKeyEvent: (node, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        // Ctrl+Space works with focus anywhere, through [_consoleClaim]. Tab
        // and Shift+A only while the canvas has focus.
        if (event.logicalKey == LogicalKeyboardKey.delete ||
            event.logicalKey == LogicalKeyboardKey.backspace) {
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
          if (_byKey[key] case final n? when !_derivedNode(n.node)) {
            setState(() => _renamingNode = key);
            return KeyEventResult.handled;
          }
        }
        // F, Shift+F and Ctrl+D, as the keymap has them for the node canvases.
        if (node.hasPrimaryFocus) {
          switch (_ui?.keymap.actionFor(BridgeKeyContext.nodes, event)) {
            case 'nodes.frame.selected':
              _frameSelected(_viewport, layout);
              return KeyEventResult.handled;
            case 'nodes.frame.all':
              _frameAll(_viewport, layout);
              return KeyEventResult.handled;
            // The shell has already duplicated the picked effects, as the
            // Edit menu's Duplicate does, and picked the copies. They
            // become the pick here too.
            case 'effect.duplicate':
              _effectRowDone();
              return KeyEventResult.handled;
          }
        }
        // Only the canvas itself: a rename field inside it types these.
        if (node.hasPrimaryFocus &&
            graphAddKey(event, _ui!.workspace.interface)) {
          if (_graph != null) {
            _openSearch(_pointerOnCanvas ??
                graphClearSpot(
                  _toCanvas(Offset(_viewport.width / 2, _viewport.height / 2)),
                  [for (final b in _boxesNow()) b.rect],
                ));
          }
          return KeyEventResult.handled;
        }
        return KeyEventResult.ignored;
      },
      // The search floats in the overlay rather than here: the canvas
      // takes every pointer that lands on it — that is how a socket is grabbed
      // without a gesture detector per socket — so a popover drawn inside it
      // would have its presses read as presses on the ground behind it.
      child: Stack(
        key: _canvasKey,
        children: [
          Positioned.fill(
            child: Listener(
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
                child: Container(
                key: const ValueKey('graph-canvas'),
                // Clipped, so a box or a group's frame pushed past the top
                // edge goes under the toolbar and not over it.
                decoration: BoxDecoration(color: t.surface0),
                clipBehavior: Clip.hardEdge,
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
                          outUnwired: _graph!.wiring.outUnwired,
                          wires: ThemeScope.of(context).wireStyle,
                          ties: tieView(),
                          picked: _selection.keys.toSet(),
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
                            // The groups' frames, under every box.
                            ...groupWashes(t, () => _claimed = true),
                            ...graphNotes(t, () => _claimed = true),
                            // A ring round each box someone else in the
                            // shared project has picked, in their colour.
                            // Only walked while the project is shared.
                            if (_sharing && _layer != null)
                              for (final box in layout.boxes)
                                if (_boxName(_layer!, box.key) case final name?)
                                  for (final (held, colour) in (_share!
                                              .inHand(_ui?.selectedComp)[name] ??
                                          const <int>[])
                                      .indexed)
                                    Positioned(
                                      key: ValueKey<String>(
                                          'graph-share-box-${box.key}-$held'),
                                      left: box.rect.left - 3 - 3.0 * held,
                                      top: box.rect.top - 3 - 3.0 * held,
                                      width: box.rect.width + 6 + 6.0 * held,
                                      height: box.rect.height + 6 + 6.0 * held,
                                      child: IgnorePointer(
                                        child: DecoratedBox(
                                          decoration: BoxDecoration(
                                            borderRadius: BorderRadius.circular(
                                                t.tokens.cardRadius + 3),
                                            border: Border.all(
                                                color: t.personColour(colour),
                                                width: 1.5),
                                          ),
                                        ),
                                      ),
                                    ),
                            for (final box in layout.boxes)
                              Positioned(
                                left: box.rect.left,
                                top: box.rect.top,
                                width: box.rect.width,
                                height: box.rect.height,
                                child: GraphNodeCard(
                                  box: box,
                                  selected: _isPicked(box.key),
                                  exposed: _graph!.wiring.exposed.any(
                                      (e) => graphNodeKey(e) == box.key),
                                  onOwnPress: () => _claimed = true,
                                  onExpose: () => _toggleExposed(box.key),
                                  onBypass: () =>
                                      _toggleBypass(box.key, box.card.enabled),
                                  renaming: _renamingNode == box.key,
                                  onStartRename: () =>
                                      setState(() => _renamingNode = box.key),
                                  onRenamed: (name) =>
                                      _renameNode(box.key, name),
                                  // Escape: shut the editor, rename nothing.
                                  onRenameCancelled: () =>
                                      setState(() => _renamingNode = null),
                                  foot: _shaderThumb(box.key),
                                  paramRow: (param) =>
                                      _paramRow(box.key, param),
                                  drivenValue: (param) =>
                                      _drivenValue(box.key, param),
                                  snap: _flight?.snap?.node == box.key
                                      ? _flight!.snap
                                      : null,
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
                  ],
                ),
              ))),
            ),
          ),
          // An effect dragged in from Effects & presets. Translucent, so every
          // other press still reaches the canvas under it.
          Positioned.fill(
            child: DragTarget<EffectDragData>(
              onAcceptWithDetails: (details) =>
                  _dropped(details.data, details.offset),
              builder: (context, candidate, _) => candidate.isEmpty
                  ? const SizedBox.expand()
                  : IgnorePointer(
                      child: Container(
                        decoration: BoxDecoration(
                            border: Border.all(color: t.accent, width: 2)),
                      ),
                    ),
            ),
          ),
          handPanLayer(_ui!.tools, t, (d) => setState(() => _pan += d)),
          graphEdgeCursor(_edgeHover || _edgeDrag != null),
        ],
      ),
    );
  }

}

/// A toolbar switch drawn as the snap magnet is: the glyph on the button's
/// own face while on, frameless and muted while off. One dress for every
/// switch on the strip.
Widget graphToolSwitch(
  LumitTheme t, {
  required Key key,
  required String tip,
  required String icon,
  required bool on,
  required VoidCallback onPressed,
}) =>
    LumitTooltip(
      message: tip,
      child: HouseButton(
        key: key,
        small: true,
        frameless: !on,
        padding: const EdgeInsets.symmetric(horizontal: 4),
        onPressed: onPressed,
        child: glyph.LumitIcon(icon,
            size: graphIconSize, colour: on ? t.textPrimary : t.textMuted),
      ),
    );

/// A toolbar switch as an item of the row that folds into the overflow mark
/// when the panel is narrow: the switch as drawn, and the menu row it folds
/// into, ticked while it is on.
FoldItem graphToolFold(
  LumitTheme t, {
  required Key key,
  required String tip,
  required String icon,
  required bool on,
  required VoidCallback onPressed,
  double gap = 6,
}) =>
    FoldItem(
      label: tip,
      icon: glyph.LumitIcon(icon,
          size: graphIconSize, colour: on ? t.textPrimary : t.textMuted),
      selected: on,
      onPressed: onPressed,
      child: Padding(
        padding: EdgeInsets.only(right: gap),
        child: graphToolSwitch(t,
            key: key, tip: tip, icon: icon, on: on, onPressed: onPressed),
      ),
    );

/// The toolbar's frame-all mark as a fold item.
FoldItem graphFrameAllFold(LumitTheme t,
        {required Key key, required VoidCallback onPressed}) =>
    FoldItem(
      label: l10n.graphFrameAll,
      icon: glyph.LumitIcon(LumitIcons.frameAll,
          size: graphIconSize, colour: t.textMuted),
      onPressed: onPressed,
      child: Padding(
        padding: const EdgeInsets.only(right: 10),
        child: LumitTooltip(
          message: l10n.graphFrameAll,
          action: 'nodes.frame.all',
          child: HoverMark(
            swells: true,
            builder: (context, lit) => GestureDetector(
              key: key,
              behavior: HitTestBehavior.opaque,
              onTap: onPressed,
              child: glyph.LumitIcon(LumitIcons.frameAll,
                  size: graphIconSize,
                  colour: Color.lerp(t.textMuted, t.textPrimary, lit)),
            ),
          ),
        ),
      ),
    );

/// The toolbar's controls, folding from the end into one mark rather than
/// overflowing. They sit at the row's end, against the zoom, and the name
/// beside them keeps its share.
Widget graphToolbarFold(List<FoldItem> items, {required Key markKey}) =>
    Flexible(
      child: Align(
        alignment: Alignment.centerRight,
        child: FoldingRow(
          markKey: markKey,
          markWidth: 22,
          markHeight: graphToolbarHeight,
          items: items,
        ),
      ),
    );

/// The boxes' own stack, under the canvas's pan and zoom.
///
/// **In plain terms.** A stack answers the pointer only inside its own
/// rectangle, and this one is the size of the panel while what it holds is the
/// whole canvas. Zoomed out, panned, or in a short panel, a box drawn outside
/// that rectangle looked right and took no press: its tick, its twirl and its
/// value wells were dead, and a drag on a well moved the box instead. This one
/// asks its boxes wherever the pointer is.
///
/// Public for the reason [GraphNodeFrame] is: all three canvases lay their
/// boxes out the same way.
class GraphBoxStack extends Stack {
  const GraphBoxStack({super.key, super.children})
      : super(clipBehavior: Clip.none);

  @override
  RenderStack createRenderObject(BuildContext context) => _RenderBoxStack(
        alignment: alignment,
        textDirection: textDirection ?? Directionality.maybeOf(context),
        fit: fit,
        clipBehavior: clipBehavior,
      );
}

class _RenderBoxStack extends RenderStack {
  _RenderBoxStack({
    super.alignment,
    super.textDirection,
    super.fit,
    super.clipBehavior,
  });

  @override
  bool hitTest(BoxHitTestResult result, {required Offset position}) {
    if (!hitTestChildren(result, position: position)) return false;
    result.add(BoxHitTestEntry(this, position));
    return true;
  }
}

/// A path as 3-on, 3-off dashes — the drawing's own stroke-dasharray. Public
/// for the reason [GraphNodeFrame] is: the inner shader graph draws its wire
/// in hand with the same stroke.
Path graphDashPath(Path path, {double dash = 3}) {
  final out = Path();
  for (final metric in path.computeMetrics()) {
    var at = 0.0;
    var on = true;
    while (at < metric.length) {
      final to = math.min(at + dash, metric.length);
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
  final GraphLayout layout;
  final List<BridgeGraphEdge> edges;
  final _InFlight? flight;

  /// The wire a box is being dragged over and would drop into (N7).
  final BridgeGraphEdge? dropWire;
  final Offset pan;
  final double zoom;
  final Color dragged;
  final LumitTheme theme;

  /// Nothing is plugged into the Layer out, so the layer draws nothing
  /// and the last wire is not drawn either.
  final bool outUnwired;
  final WireStyle wires;
  final TieView ties;

  /// The picked boxes, whose wires draw heavier so what a pick is joined to
  /// reads at a glance.
  final Set<String> picked;

  const _GraphPainter({
    required this.layout,
    required this.edges,
    required this.flight,
    required this.dropWire,
    required this.pan,
    required this.zoom,
    required this.dragged,
    required this.theme,
    required this.outUnwired,
    required this.wires,
    required this.ties,
    required this.picked,
  });

  /// Whether a wire ends on a picked box: either end sits on the box's edge.
  bool _lit(TieWire wire) {
    if (picked.isEmpty) return false;
    for (final key in picked) {
      final rect = layout.byKey[key]?.rect.inflate(graphSocketSize);
      if (rect != null &&
          (rect.contains(wire.from) || rect.contains(wire.to))) {
        return true;
      }
    }
    return false;
  }

  Offset _screen(Offset canvas) => canvas * zoom + pan;

  @override
  void paint(Canvas canvas, Size size) {
    paintTiedWires(
        canvas,
        [
          for (final (wire, edge, type) in _layerWires(layout, edges,
              held: flight?.chain, outUnwired: outUnwired))
            // The wire a box is being dropped into wears the canvas's own
            // picked colour while it is under one (N7) — the same mark a
            // picked box wears, because both say "this is what the release
            // will act on".
            (
              wire,
              edge != null && edge == dropWire
                  ? theme.animated
                  : portColour(theme, type),
              (edge != null && edge == dropWire) || _lit(wire) ? 2.0 : 1.0,
            ),
        ],
        ties,
        pan: pan,
        zoom: zoom,
        style: wires,
        theme: theme);

    if (flight case final f?) {
      // A wire drawn out of an input runs the other way, pointer to socket,
      // so it leaves the input heading left as a stored wire would.
      final (a, b) = f.from.isInput
          ? (f.end, f.from.at)
          : (ties.via ?? f.from.at, f.end);
      _wire(canvas, a, b, dragged, dashes: true);
    }
  }

  /// One wire: [graphWirePath]'s cubic, which is the curve both drawings draw
  /// — with the minimum stub that keeps it visible when the consumer sits left
  /// of its producer.
  void _wire(Canvas canvas, Offset from, Offset to, Color colour,
      {required bool dashes, double weight = 1}) {
    final path = graphWirePath(_screen(from), _screen(to),
        zoom: zoom, wires: wires);
    final paint = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = graphWireWidth * zoom * weight;
    canvas.drawPath(dashes ? graphDashPath(path) : path, paint);
  }

  @override
  bool shouldRepaint(_GraphPainter old) => true;
}

/// A node card's inline rename field, and a group bar's: the current name, selected
/// on open because a name is retyped far more often than amended, committed on
/// Enter or on clicking away and thrown away on Escape.
///
/// Empty is a real answer — it clears the custom name and the card goes back to
/// showing the box's own label — so an empty field is committed like any other.
class GraphNameField extends StatefulWidget {
  final String initial;
  final ValueChanged<String> onDone;
  final VoidCallback onCancel;

  /// Many lines, for a note: Enter starts a new one and clicking away is
  /// what commits.
  final bool multiline;
  final String? hint;

  const GraphNameField({
    super.key,
    required this.initial,
    required this.onDone,
    required this.onCancel,
    this.multiline = false,
    this.hint,
  });

  @override
  State<GraphNameField> createState() => _GraphNameFieldState();
}

class _GraphNameFieldState extends State<GraphNameField> {
  late final TextEditingController _controller = TextEditingController(
    text: widget.initial,
  )..selection = TextSelection(
      baseOffset: 0,
      extentOffset: widget.initial.length,
    );

  // Autofocus does nothing here, since the canvas took focus on the press
  // that opened the rename, so the field asks for it outright.
  final FocusNode _focus = FocusNode(debugLabel: 'box rename');

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void dispose() {
    _controller.dispose();
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => HouseTextField(
        controller: _controller,
        focusNode: _focus,
        width: double.infinity,
        autofocus: true,
        submitOnLostFocus: true,
        multiline: widget.multiline,
        topAlign: widget.multiline,
        hint: widget.hint,
        onSubmitted: widget.onDone,
        onCancelled: widget.onCancel,
      );
}
