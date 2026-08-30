// The inner shader graph — the inside of one Custom shader, drawn in the
// Graph panel (K-642, docs/impl/custom-shader.md §4, CS4/CS5).
//
// **In plain terms.** A Custom shader can hold a graph instead of typed code:
// boxes for the picture coming in, boxes for adding and multiplying, one box
// for the picture going out. Double-clicking the effect opens this view —
// entered like a precomp, with a breadcrumb back — and every wire drawn here
// recompiles the shader through the same road typed text takes.
//
// **Nothing here decides anything** (the thin-view rule). The engine owns the
// vocabulary (`listShaderNodes`), the port types and the compile
// (`shaderGraphView`); this canvas draws what it is told and forwards
// gestures. A drop that would mistype or loop is refused *visually and
// op-free* by building the candidate graph and asking the engine — the panel
// never learns the type rules.
//
// **One gesture, one commit, one undo step.** Every edit stages
// `setShaderGraph` on the instance and commits the stack, exactly as the
// shader editor's Apply does — so a wire, a drag and a delete each undo whole.
//
// The canvas machinery is shared with the outer Graph panel (§4.2 — "reuses
// the canvas rather than the model"): the dot grid, the node frame, the wire
// cubic and the card metrics all come from `graph_panel.dart`.

import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart' show KeyDownEvent, LogicalKeyboardKey;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/fx_console_frb.dart';
import '../state/dock.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'graph_panel.dart'
    show
        GraphGroundPainter,
        GraphNodeFrame,
        graphDashPath,
        graphWirePath,
        graphDotGrid,
        graphNodeHeaderHeight,
        graphOutNodeWidth,
        graphPortRowHeight,
        graphSocketSize,
        graphToolbarHeight,
        graphWireWidth;

/// The engine's word for a box, in the user's language. The kinds cross the
/// bridge as ids, never as English (K-303) — these words are the frontend's.
String shaderNodeWord(String kind) => switch (kind) {
      'picture' => l10n.shaderNodePicture,
      'picture2' => l10n.shaderNodePicture2,
      'matte' => l10n.shaderNodeMatte,
      'uv' => l10n.shaderNodeUv,
      'time' => l10n.shaderNodeTime,
      'seed' => l10n.shaderNodeSeed,
      'param' => l10n.shaderNodeParam,
      'add' => l10n.shaderNodeAdd,
      'subtract' => l10n.shaderNodeSubtract,
      'multiply' => l10n.shaderNodeMultiply,
      'divide' => l10n.shaderNodeDivide,
      'modulo' => l10n.shaderNodeModulo,
      'mix' => l10n.shaderNodeMix,
      'clamp' => l10n.shaderNodeClamp,
      'saturate' => l10n.shaderNodeSaturate,
      'pow' => l10n.shaderNodePow,
      'sqrt' => l10n.shaderNodeSqrt,
      'abs' => l10n.shaderNodeAbs,
      'sign' => l10n.shaderNodeSign,
      'min' => l10n.shaderNodeMin,
      'max' => l10n.shaderNodeMax,
      'floor' => l10n.shaderNodeFloor,
      'ceil' => l10n.shaderNodeCeil,
      'fract' => l10n.shaderNodeFract,
      'step' => l10n.shaderNodeStep,
      'smoothstep' => l10n.shaderNodeSmoothstep,
      'sin' => l10n.shaderNodeSin,
      'cos' => l10n.shaderNodeCos,
      'atan2' => l10n.shaderNodeAtan2,
      'length' => l10n.shaderNodeLength,
      'distance' => l10n.shaderNodeDistance,
      'dot' => l10n.shaderNodeDot,
      'normalize' => l10n.shaderNodeNormalize,
      'split' => l10n.shaderNodeSplit,
      'combine2' => l10n.shaderNodeCombine2,
      'combine3' => l10n.shaderNodeCombine3,
      'combine4' => l10n.shaderNodeCombine4,
      'swizzle' => l10n.shaderNodeSwizzle,
      'sample' => l10n.shaderNodeSample,
      'luminance' => l10n.shaderNodeLuminance,
      'premultiply' => l10n.shaderNodePremultiply,
      'unpremultiply' => l10n.shaderNodeUnpremultiply,
      'tint' => l10n.shaderNodeTint,
      'blend' => l10n.shaderNodeBlend,
      'result' => l10n.shaderNodeResult,
      _ => kind,
    };

/// A port's word. The single-letter ids (`a`, `b`, `x`, `t`, …) are shown
/// verbatim — they are maths symbols, not words — and the rest are the
/// frontend's own.
String shaderPortWord(String id) => switch (id) {
      'colour' => l10n.shaderPortColour,
      'picture' => l10n.shaderPortPicture,
      'strength' => l10n.shaderPortStrength,
      'uv' => l10n.shaderPortUv,
      'seconds' => l10n.shaderPortSeconds,
      'seed' => l10n.shaderPortSeed,
      'value' => l10n.shaderPortValue,
      'lo' => l10n.shaderPortLo,
      'hi' => l10n.shaderPortHi,
      'edge' => l10n.shaderPortEdge,
      'vector' => l10n.shaderPortVector,
      'base' => l10n.shaderPortBase,
      'blend' => l10n.shaderPortBlend,
      'amount' => l10n.shaderPortAmount,
      'tint' => l10n.shaderPortTint,
      _ => id,
    };

/// Which theme token a shader port type draws in: the widths one to three are
/// numbers, a vec4 is a colour, and a picture is the image family — the same
/// legend the outer canvas keeps (K-472).
Color shaderPortColour(LumitTheme t, BridgeShaderTy ty) => switch (ty) {
      BridgeShaderTy.f32 ||
      BridgeShaderTy.vec2 ||
      BridgeShaderTy.vec3 =>
        t.port.number,
      BridgeShaderTy.vec4 => t.port.colour,
      BridgeShaderTy.picture => t.port.image,
    };

/// The stored graph as this canvas holds it: the parsed JSON, kept as maps so
/// keys this build has never heard of ride through a commit unharmed (K-065).
class _Inner {
  final List<Map<String, dynamic>> nodes;
  final List<Map<String, dynamic>> edges;
  final Map<int, Offset> layout;

  _Inner(this.nodes, this.edges, this.layout);

  static _Inner parse(String json) {
    final raw = jsonDecode(json);
    final nodes = <Map<String, dynamic>>[];
    final edges = <Map<String, dynamic>>[];
    final layout = <int, Offset>{};
    if (raw is Map) {
      for (final n in raw['nodes'] as List? ?? const []) {
        if (n is Map) nodes.add(n.cast<String, dynamic>());
      }
      for (final e in raw['edges'] as List? ?? const []) {
        if (e is Map) edges.add(e.cast<String, dynamic>());
      }
      for (final p in raw['layout'] as List? ?? const []) {
        if (p is Map && p['node'] is int) {
          layout[p['node'] as int] = Offset(
            (p['x'] as num?)?.toDouble() ?? 0,
            (p['y'] as num?)?.toDouble() ?? 0,
          );
        }
      }
    }
    return _Inner(nodes, edges, layout);
  }

  /// A fresh graph: one Result box, nothing wired — what entering a shader
  /// that has never had a graph starts from.
  static _Inner fresh() => _Inner(
        [
          {'id': 1, 'kind': 'result'},
        ],
        [],
        {},
      );

  _Inner clone() => _Inner(
        [for (final n in nodes) Map<String, dynamic>.of(n)],
        [for (final e in edges) Map<String, dynamic>.of(e)],
        Map<int, Offset>.of(layout),
      );

  String encode() => jsonEncode({
        'nodes': nodes,
        'edges': edges,
        if (layout.isNotEmpty)
          'layout': [
            for (final e in layout.entries)
              {'node': e.key, 'x': e.value.dx, 'y': e.value.dy},
          ],
      });

  int nextId() {
    var top = 0;
    for (final n in nodes) {
      final id = n['id'];
      if (id is int && id > top) top = id;
    }
    return top + 1;
  }
}

/// One socket as the canvas draws it.
class _Sock {
  final int node;
  final int port;
  final bool isInput;
  final BridgeShaderTy ty;
  final Offset at;
  const _Sock(this.node, this.port, this.isInput, this.ty, this.at);
}

/// One box, laid out.
class _Box {
  final BridgeShaderGraphNode node;
  final Rect rect;
  const _Box(this.node, this.rect);

  Offset socket(int port, {required bool isInput}) => Offset(
        isInput ? rect.left : rect.right,
        rect.top +
            1 +
            graphNodeHeaderHeight +
            port * graphPortRowHeight +
            graphPortRowHeight / 2,
      );
}

/// A wire in hand: the socket it left, where the pointer is, and — when the
/// press picked up a stored wire by its far end — the edge that leaves in the
/// same commit as wherever it lands.
class _Flight {
  final _Sock from;
  Offset to;
  final Map<String, dynamic>? detached;
  _Flight(this.from, this.to, {this.detached});
}

/// The inner graph inside the Graph panel: a breadcrumb strip and the canvas.
class ShaderGraphPanel extends StatefulWidget {
  final ShaderGraphEntry entry;
  final VoidCallback onExit;

  const ShaderGraphPanel({super.key, required this.entry, required this.onExit});

  @override
  State<ShaderGraphPanel> createState() => _ShaderGraphPanelState();
}

class _ShaderGraphPanelState extends State<ShaderGraphPanel> {
  LumitUiState? _ui;

  /// The held graph and the engine's reading of it. One read on entry and on
  /// document change — never in a rebuild (K-183).
  _Inner? _graph;
  BridgeShaderGraphView? _view;

  /// Staged positions: a drag moves this map and the release commits it.
  Map<int, Offset> _positions = {};

  final Set<int> _selection = {};

  Offset _pan = Offset.zero;
  double _zoom = 1;

  _Flight? _flight;
  ({int node, Offset grab, Map<int, Offset> origins})? _drag;
  Offset? _panFrom;
  Offset? _pressAt;

  /// Whether the console is up, so a second ask cannot stack another.
  bool _searching = false;

  /// The vocabulary, asked for once per session (it is a table in the
  /// engine's own code and cannot move).
  static List<BridgeShaderNodeKind>? _kinds;

  final FocusNode _focus = FocusNode(debugLabel: 'shader graph');

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (identical(ui, _ui)) return;
    _ui?.model.removeListener(_reload);
    _ui = ui;
    ui.model.addListener(_reload);
    // While the inner graph is the panel's face, Ctrl+Space adds a shader box
    // (K-673) — the owner's "can't add options in the custom shader view".
    // Chained over the outer canvas's own claim, which stands down while a
    // shader is entered.
    if (ui.consoleClaim != _consoleClaim) _priorConsoleClaim = ui.consoleClaim;
    ui.consoleClaim = _consoleClaim;
    final held = ui.shaderGraphViews[widget.entry.effect.toString()];
    if (held != null) {
      _pan = held.pan;
      _zoom = held.zoom;
    }
    _reload();
  }

  @override
  void dispose() {
    _rememberView();
    _ui?.model.removeListener(_reload);
    if (_ui?.consoleClaim == _consoleClaim) {
      _ui!.consoleClaim = _priorConsoleClaim;
    }
    _focus.dispose();
    super.dispose();
  }

  /// The claim this view had to displace to take Ctrl+Space.
  bool Function()? _priorConsoleClaim;

  bool _consoleClaim() {
    final ui = _ui;
    if (!mounted ||
        ui == null ||
        ui.activePanel.value != Panel.graph ||
        _graph == null ||
        _searching) {
      return _priorConsoleClaim?.call() ?? false;
    }
    _openConsole(_toCanvas(const Offset(80, 80)), byKey: true);
    return true;
  }

  /// The Ctrl+Space console, wearing the shader vocabulary (K-673): every box
  /// the engine lists — the Parameter box included — and picking one drops it
  /// at [at]. A wire let go over empty canvas opens the same surface, exactly
  /// as the outer graph's does.
  ///
  /// The list is deliberately unfiltered with a wire in hand: this panel never
  /// learns the type rules (the engine is asked per drop), so it cannot
  /// promise which boxes a wire fits — the picked box lands unwired and the
  /// hand draws the wire, with the engine's refusal as the backstop.
  Future<void> _openConsole(Offset at, {bool byKey = false}) async {
    if (_searching) return;
    setState(() => _searching = true);
    final kinds = _kinds ??= listShaderNodes();
    try {
      await showFxConsoleFrb(
        context: context,
        anchor: lastKnownPointerPosition,
        model: FxConsoleModel(
          keyHint: byKey ? l10n.fxConsoleKey : null,
          footer: l10n.shaderSearchAdds,
          entries: [
            for (final k in kinds)
              if (k.kind != 'result' || !_hasResult())
                FxConsoleEntry(
                  label: shaderNodeWord(k.kind),
                  kind: FxConsoleKind.effect,
                  run: () => _addNode(k.kind, at),
                ),
          ],
        ),
      );
    } finally {
      if (mounted) setState(() => _searching = false);
    }
  }

  /// The view you left is the view you return to (§4.2), held in the session
  /// and never in the document.
  void _rememberView() {
    _ui?.shaderGraphViews[widget.entry.effect.toString()] =
        (pan: _pan, zoom: _zoom);
  }

  /// This shader's staged handle in a fresh copy of the stack, or null when
  /// the effect has gone from under the view.
  BridgeEffectInstance? _instance(List<BridgeEffectInstance> stack) {
    for (final inst in stack) {
      if (inst.id() == widget.entry.effect) return inst;
    }
    return null;
  }

  /// The one read. A shader that has never held a graph starts from a staged
  /// Result box; the graph reaches the document with the first committed
  /// gesture, which is also the moment it becomes master (§4.1).
  void _reload() {
    if (!mounted) return;
    List<BridgeEffectInstance> stack;
    try {
      stack = widget.entry.layer.getEffects();
    } catch (_) {
      stack = const [];
    }
    final inst = _instance(stack);
    if (inst == null) {
      // The effect has gone — an undo past the entry, a delete under it.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) widget.onExit();
      });
      return;
    }
    final graph = switch (inst.shaderGraph()) {
      final String json => _Inner.parse(json),
      null => _Inner.fresh(),
    };
    final view = shaderGraphView(graph: graph.encode());
    setState(() {
      _graph = graph;
      _view = view;
      _positions = Map.of(graph.layout);
      _selection.removeWhere(
          (id) => !graph.nodes.any((n) => n['id'] == id));
    });
  }

  /// One gesture, one `setShaderGraph`, one `setEffects`, one undo step.
  void _commit(_Inner graph) {
    graph.layout
      ..clear()
      ..addAll(_positions);
    List<BridgeEffectInstance> stack;
    try {
      stack = widget.entry.layer.getEffects();
    } catch (_) {
      return;
    }
    final inst = _instance(stack);
    if (inst == null) return;
    try {
      inst.setShaderGraph(graph: graph.encode());
      widget.entry.layer.setEffects(effects: stack);
    } catch (_) {
      // Refused, or the stack moved under us; re-reading is the recovery.
    }
    _ui?.model.refresh();
    _reload();
  }

  // --- Wiring -------------------------------------------------------------

  /// Join two sockets, replacing whatever the input held. The candidate graph
  /// is built and the **engine** asked; a drop that turns a compiling graph
  /// into one that does not is declined visually, and nothing commits. A graph
  /// that is already broken stays editable — refusing every edit to a broken
  /// graph would lock the user out of fixing it.
  void _connect(_Sock a, _Sock b, {Map<String, dynamic>? without}) {
    if (a.isInput == b.isInput) return;
    final out = a.isInput ? b : a;
    final into = a.isInput ? a : b;
    if (out.node == into.node) return;
    final next = _graph!.clone();
    next.edges.removeWhere((e) =>
        (e['to'] == into.node && e['to_port'] == into.port) ||
        (without != null && _sameEdge(e, without)));
    next.edges.add({
      'from': out.node,
      'from_port': out.port,
      'to': into.node,
      'to_port': into.port,
    });
    final after = shaderGraphView(graph: next.encode()).error;
    if (after != null && _view?.error == null) return;
    _commit(next);
  }

  bool _sameEdge(Map<String, dynamic> a, Map<String, dynamic> b) =>
      a['from'] == b['from'] &&
      a['from_port'] == b['from_port'] &&
      a['to'] == b['to'] &&
      a['to_port'] == b['to_port'];

  void _removeEdge(Map<String, dynamic> edge) {
    final next = _graph!.clone();
    next.edges.removeWhere((e) => _sameEdge(e, edge));
    _commit(next);
  }

  Map<String, dynamic>? _edgeInto(_Sock socket) {
    for (final e in _graph!.edges) {
      if (e['to'] == socket.node && e['to_port'] == socket.port) return e;
    }
    return null;
  }

  /// Delete the picked boxes, their wires with them in the same commit.
  bool _deleteSelected() {
    if (_selection.isEmpty || _graph == null) return false;
    final next = _graph!.clone();
    next.nodes.removeWhere((n) => _selection.contains(n['id']));
    next.edges.removeWhere((e) =>
        _selection.contains(e['from']) || _selection.contains(e['to']));
    for (final id in _selection) {
      _positions.remove(id);
    }
    _selection.clear();
    _commit(next);
    return true;
  }

  /// Add one box where the search was asked for. A Parameter box lands with a
  /// working slider so it derives a row at once; its five facts are edited on
  /// the box.
  void _addNode(String kind, Offset at) {
    final next = _graph!.clone();
    final id = next.nextId();
    final node = <String, dynamic>{'id': id, 'kind': kind};
    if (kind == 'param') {
      node['settings'] = {
        'id': 'param$id',
        'kind': 'slider',
        'min': 0,
        'max': 1,
        'default': 0,
      };
    }
    next.nodes.add(node);
    _positions[id] = at;
    _commit(next);
  }

  // --- Geometry -----------------------------------------------------------

  Offset _toCanvas(Offset local) => (local - _pan) / _zoom;

  List<_Box> _boxes() {
    final out = <_Box>[];
    var placed = 0;
    for (final node in _view?.nodes ?? const <BridgeShaderGraphNode>[]) {
      final at = _positions[node.id] ??
          Offset(40.0 + (placed % 4) * 170.0, 40.0 + (placed ~/ 4) * 120.0);
      placed++;
      final rows = math.max(node.inputs.length, node.outputs.length);
      out.add(_Box(
        node,
        Rect.fromLTWH(
          at.dx,
          at.dy,
          graphOutNodeWidth + 2,
          2 + graphNodeHeaderHeight + rows * graphPortRowHeight,
        ),
      ));
    }
    return out;
  }

  _Sock? _socketAt(List<_Box> boxes, Offset at) {
    for (final box in boxes) {
      for (final (isInput, ports) in [
        (true, box.node.inputs),
        (false, box.node.outputs)
      ]) {
        for (var i = 0; i < ports.length; i++) {
          final centre = box.socket(i, isInput: isInput);
          if ((centre - at).distance <= 7) {
            return _Sock(box.node.id, i, isInput, ports[i].ty, centre);
          }
        }
      }
    }
    return null;
  }

  _Box? _boxAt(List<_Box> boxes, Offset at) {
    for (final box in boxes.reversed) {
      if (box.rect.contains(at)) return box;
    }
    return null;
  }

  // --- Pointers -----------------------------------------------------------

  void _down(PointerDownEvent event, List<_Box> boxes) {
    _focus.requestFocus();
    final at = _toCanvas(event.localPosition);
    _pressAt = event.localPosition;

    final socket = _socketAt(boxes, at);
    if (socket != null) {
      // A wired input is grabbed by its far end, exactly as the outer canvas
      // grabs one: drop it elsewhere to move it, on nothing to take it off.
      final held = socket.isInput ? _edgeInto(socket) : null;
      _Sock? grabbed;
      if (held != null) {
        for (final box in boxes) {
          if (box.node.id == held['from']) {
            final port = held['from_port'] as int? ?? 0;
            if (port < box.node.outputs.length) {
              grabbed = _Sock(box.node.id, port, false,
                  box.node.outputs[port].ty, box.socket(port, isInput: false));
            }
          }
        }
      }
      setState(() => _flight = grabbed == null
          ? _Flight(socket, at)
          : _Flight(grabbed, at, detached: held));
      return;
    }

    final box = _boxAt(boxes, at);
    if (box != null) {
      setState(() {
        if (!_selection.contains(box.node.id)) {
          _selection
            ..clear()
            ..add(box.node.id);
        }
        _drag = (
          node: box.node.id,
          grab: at,
          origins: {
            for (final b in boxes)
              if (_selection.contains(b.node.id)) b.node.id: b.rect.topLeft,
          },
        );
      });
      return;
    }

    if (event.buttons == kMiddleMouseButton) {
      setState(() => _panFrom = _pan - event.localPosition);
      return;
    }
    setState(() {
      _selection.clear();
      _panFrom = _pan - event.localPosition;
    });
  }

  void _move(PointerMoveEvent event, List<_Box> boxes) {
    final at = _toCanvas(event.localPosition);
    if (_flight case final flight?) {
      setState(() => flight.to = at);
      return;
    }
    if (_drag case final drag?) {
      setState(() {
        var delta = at - drag.grab;
        // The magnet the outer canvas keeps: boxes land on the grid's pitch.
        final origin = drag.origins[drag.node];
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
      });
      return;
    }
    if (_panFrom case final from?) {
      setState(() {
        _pan = from + event.localPosition;
        _rememberView();
      });
    }
  }

  void _up(PointerUpEvent event, List<_Box> boxes) {
    final at = _toCanvas(event.localPosition);
    final moved = _pressAt == null ||
        (event.localPosition - _pressAt!).distance > 3;

    if (_flight case final flight?) {
      setState(() => _flight = null);
      final landed = _socketAt(boxes, at);
      if (flight.detached case final held?) {
        if (moved && landed != null) {
          _connect(flight.from, landed, without: held);
        } else {
          _removeEdge(held);
        }
        return;
      }
      if (landed != null) {
        _connect(flight.from, landed);
      } else if (moved) {
        // Onto empty canvas: the console opens, and the picked box lands
        // where the wire was let go.
        _openConsole(at);
      }
      return;
    }

    if (_drag != null) {
      setState(() => _drag = null);
      if (moved && _graph != null) _commit(_graph!.clone());
      return;
    }
    setState(() => _panFrom = null);
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
      _rememberView();
    });
  }

  // --- Drawing ------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final boxes = _boxes();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _breadcrumb(t),
        Expanded(child: _canvas(t, boxes)),
      ],
    );
  }

  /// `comp › layer › shader` — the way back, worn where the outer panel wears
  /// its toolbar (§4.2). The first two crumbs both leave: the road out of a
  /// shader is one step long however it is spelt.
  Widget _breadcrumb(LumitTheme t) {
    Widget crumb(String key, String word, VoidCallback? onTap) =>
        GestureDetector(
          key: ValueKey<String>(key),
          behavior: HitTestBehavior.opaque,
          onTap: onTap,
          child: Text(
            word,
            style: onTap == null
                ? t.kickerOn
                : t.kicker.copyWith(color: t.textMuted),
            overflow: TextOverflow.ellipsis,
          ),
        );
    final sep = Padding(
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Text('›', style: t.kicker.copyWith(color: t.textMuted)),
    );
    return Container(
      key: const ValueKey<String>('shader-breadcrumb'),
      height: graphToolbarHeight,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 10),
      child: Row(
        children: [
          crumb('shader-crumb-comp', widget.entry.compName, widget.onExit),
          sep,
          crumb('shader-crumb-layer', widget.entry.layerName, widget.onExit),
          sep,
          Expanded(
              child: crumb('shader-crumb-shader', widget.entry.effectName,
                  null)),
        ],
      ),
    );
  }

  Widget _canvas(LumitTheme t, List<_Box> boxes) => Focus(
        focusNode: _focus,
        autofocus: true,
        onKeyEvent: (node, event) {
          if (event is! KeyDownEvent) return KeyEventResult.ignored;
          if (event.logicalKey == LogicalKeyboardKey.escape) {
            widget.onExit();
            return KeyEventResult.handled;
          }
          // No Tab door (K-673): Ctrl+Space is the console's one key,
          // answered through [_consoleClaim].
          if (event.logicalKey == LogicalKeyboardKey.delete ||
              event.logicalKey == LogicalKeyboardKey.backspace) {
            return _deleteSelected()
                ? KeyEventResult.handled
                : KeyEventResult.ignored;
          }
          return KeyEventResult.ignored;
        },
        child: Listener(
          onPointerDown: (e) => _down(e, boxes),
          onPointerMove: (e) => _move(e, boxes),
          onPointerUp: (e) => _up(e, boxes),
          onPointerSignal: _wheel,
          behavior: HitTestBehavior.opaque,
          child: Container(
            key: const ValueKey<String>('shader-canvas'),
            color: t.surface0,
            child: Stack(
              clipBehavior: Clip.hardEdge,
              children: [
                Positioned.fill(
                  child: RepaintBoundary(
                    child: CustomPaint(
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
                    painter: _WirePainter(
                      boxes: boxes,
                      edges: _graph?.edges ?? const [],
                      flight: _flight,
                      pan: _pan,
                      zoom: _zoom,
                      theme: t,
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
                        for (final box in boxes)
                          Positioned(
                            left: box.rect.left,
                            top: box.rect.top,
                            child: _ShaderNodeCard(
                              box: box,
                              graph: _graph,
                              selected: _selection.contains(box.node.id),
                            ),
                          ),
                      ],
                    ),
                  ),
                ),
                // The one sentence about a graph that will not compile —
                // never red, never modal, and the canvas stays editable
                // (§2.2: being broken is a state to work in).
                if (_view?.error case final error?)
                  Positioned(
                    left: 10,
                    bottom: 8,
                    child: Text(
                      error,
                      key: const ValueKey<String>('shader-error'),
                      style: t.small.copyWith(color: t.accent),
                    ),
                  ),
              ],
            ),
          ),
        ),
      );

  bool _hasResult() =>
      _graph?.nodes.any((n) => n['kind'] == 'result') ?? false;
}

/// One box: the shared frame, the L11 header grammar (the strip an Effect
/// controls heading wears — here just the name, a shader box having no enable
/// and nothing to twirl), and a socket per port.
class _ShaderNodeCard extends StatelessWidget {
  final _Box box;
  final _Inner? graph;
  final bool selected;

  const _ShaderNodeCard({
    required this.box,
    required this.graph,
    required this.selected,
  });

  bool _wired(int port, {required bool isInput}) {
    for (final e in graph?.edges ?? const <Map<String, dynamic>>[]) {
      if (isInput && e['to'] == box.node.id && e['to_port'] == port) {
        return true;
      }
      if (!isInput && e['from'] == box.node.id && e['from_port'] == port) {
        return true;
      }
    }
    return false;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final rows = math.max(box.node.inputs.length, box.node.outputs.length);
    return SizedBox(
      key: ValueKey<String>('shader-node-${box.node.id}'),
      width: box.rect.width,
      height: box.rect.height,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Positioned.fill(
            child: GraphNodeFrame(
              colour: selected ? t.animated : t.hairline,
              dashed: false,
              fill: t.surface1,
              radius: t.tokens.controlRadius,
            ),
          ),
          Positioned(
            left: 1,
            top: 1,
            width: box.rect.width - 2,
            child: Container(
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
                      shaderNodeWord(box.node.kind),
                      key: ValueKey<String>('shader-node-name-${box.node.id}'),
                      style: t.kickerOn,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  // A Parameter box wears the user's own word for its control
                  // beside its kind, the reading a custom-named driver has.
                  if (box.node.label case final own?) ...[
                    const SizedBox(width: 6),
                    Flexible(
                      child: Text(own,
                          style: t.kicker.copyWith(letterSpacing: 0.54),
                          overflow: TextOverflow.ellipsis),
                    ),
                  ],
                ],
              ),
            ),
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

  Widget _row(LumitTheme t, int i) {
    final input = i < box.node.inputs.length ? box.node.inputs[i] : null;
    final output = i < box.node.outputs.length ? box.node.outputs[i] : null;
    return Stack(
      clipBehavior: Clip.none,
      children: [
        if (input != null)
          Positioned.fill(
            child: Padding(
              padding: const EdgeInsets.only(left: 12),
              child: Align(
                alignment: Alignment.centerLeft,
                child: Text(shaderPortWord(input.id),
                    style:
                        t.small.copyWith(color: shaderPortColour(t, input.ty)),
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
                child: Text(shaderPortWord(output.id),
                    style:
                        t.small.copyWith(color: shaderPortColour(t, output.ty)),
                    overflow: TextOverflow.ellipsis),
              ),
            ),
          ),
        if (input != null)
          Positioned(
            left: -graphSocketSize / 2 - 1,
            top: (graphPortRowHeight - graphSocketSize) / 2,
            child: _socket(t, input, i, isInput: true),
          ),
        if (output != null)
          Positioned(
            right: -graphSocketSize / 2 - 1,
            top: (graphPortRowHeight - graphSocketSize) / 2,
            child: _socket(t, output, i, isInput: false),
          ),
      ],
    );
  }

  Widget _socket(LumitTheme t, BridgeShaderPort port, int i,
      {required bool isInput}) {
    final colour = shaderPortColour(t, port.ty);
    return Container(
      key: ValueKey<String>(
          'shader-socket-${box.node.id}-${isInput ? 'in' : 'out'}-${port.id}'),
      width: graphSocketSize,
      height: graphSocketSize,
      decoration: BoxDecoration(
        color: _wired(i, isInput: isInput) ? colour : t.surface1,
        shape: BoxShape.circle,
        border: Border.all(color: colour),
      ),
    );
  }
}

/// Every wire, coloured by its source port's type, and the dashed one in hand.
class _WirePainter extends CustomPainter {
  final List<_Box> boxes;
  final List<Map<String, dynamic>> edges;
  final _Flight? flight;
  final Offset pan;
  final double zoom;
  final LumitTheme theme;

  const _WirePainter({
    required this.boxes,
    required this.edges,
    required this.flight,
    required this.pan,
    required this.zoom,
    required this.theme,
  });

  Offset _screen(Offset canvas) => canvas * zoom + pan;

  @override
  void paint(Canvas canvas, Size size) {
    final byId = {for (final b in boxes) b.node.id: b};
    for (final e in edges) {
      final from = byId[e['from']];
      final to = byId[e['to']];
      if (from == null || to == null) continue;
      final port = e['from_port'] as int? ?? 0;
      if (port >= from.node.outputs.length) continue;
      final a = from.socket(port, isInput: false);
      final b = to.socket(e['to_port'] as int? ?? 0, isInput: true);
      _wire(canvas, a, b,
          shaderPortColour(theme, from.node.outputs[port].ty),
          dashes: false);
    }
    if (flight case final f?) {
      _wire(canvas, f.from.at, f.to, theme.textPrimary, dashes: true);
    }
  }

  void _wire(Canvas canvas, Offset from, Offset to, Color colour,
      {required bool dashes}) {
    final path = graphWirePath(_screen(from), _screen(to), zoom: zoom);
    final paint = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = graphWireWidth * zoom;
    canvas.drawPath(dashes ? graphDashPath(path) : path, paint);
  }

  @override
  bool shouldRepaint(_WirePainter old) => true;
}
