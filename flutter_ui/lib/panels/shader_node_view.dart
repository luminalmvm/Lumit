// The Node panel inside a Custom shader's graph: the shader's boxes, a click
// from being picked on the canvas, and the picked one under the shader's name.
// A picked box lists its sockets and what each is wired to, and a Parameter
// box also shows the facts its control is made from. An edit is one commit,
// one undo step, as every gesture on the canvas is.

import 'dart:convert';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../widgets/controls.dart';
import '../widgets/edge_fade.dart';
import 'effect_param_row_frb.dart' show effectCellWidth;
import 'fx_section.dart' show fxKeyColumnWidth, fxTwoColumnRow;
import 'shader_editor.dart' show InstanceHome;
import 'shader_graph.dart'
    show shaderMenuKey, shaderNodeWord, shaderPortColour, shaderPortWord;

/// The header the Node panel draws, so this view wears the same strip.
typedef ShaderNodeHeader = Widget Function(String name,
    {String? crumb, VoidCallback? onCrumb, void Function(Offset at)? onMenu});

class ShaderNodeView extends StatefulWidget {
  final ShaderGraphEntry entry;
  final ShaderNodeHeader header;

  const ShaderNodeView({super.key, required this.entry, required this.header});

  @override
  State<ShaderNodeView> createState() => _ShaderNodeViewState();
}

class _ShaderNodeViewState extends State<ShaderNodeView> {
  LumitUiState? _ui;
  List<BridgeShaderGraphNode> _nodes = const [];
  Map<int, Map<String, dynamic>> _settings = const {};
  List<Map<String, dynamic>> _edges = const [];

  /// The number being dragged, shown as it goes and committed once.
  ({String key, double value})? _drag;

  /// While a row's menu is up the list stays, though the right-click has
  /// made that row's box the pick.
  bool _holdIndex = false;

  InstanceHome get _home => widget.entry.home;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ui = context.read<LumitUiState>();
    if (identical(ui, _ui)) return;
    _unbind();
    _ui = ui;
    ui.model.addListener(_reload);
    ui.shaderGraphNode.addListener(_reload);
    _reload();
  }

  @override
  void didUpdateWidget(ShaderNodeView old) {
    super.didUpdateWidget(old);
    if (old.entry.effect != widget.entry.effect) _reload();
  }

  void _unbind() {
    _ui?.model.removeListener(_reload);
    _ui?.shaderGraphNode.removeListener(_reload);
    lumitPopupUp.removeListener(_menuGone);
  }

  @override
  void dispose() {
    _unbind();
    super.dispose();
  }

  /// The stored graph, as maps, and the shader's instance in a fresh stack.
  (List<BridgeEffectInstance>, BridgeEffectInstance, Map<String, dynamic>)?
      _read() {
    try {
      final stack = _home.read();
      for (final inst in stack) {
        if (inst.id() != widget.entry.effect) continue;
        final raw = inst.shaderGraph();
        final graph = raw == null ? null : jsonDecode(raw);
        if (graph is! Map) return null;
        return (stack, inst, graph.cast<String, dynamic>());
      }
    } catch (_) {
      // The effect has gone from under the view; the canvas leaves too.
    }
    return null;
  }

  void _reload() {
    if (!mounted) return;
    final read = _read();
    final settings = <int, Map<String, dynamic>>{};
    final edges = <Map<String, dynamic>>[];
    var nodes = const <BridgeShaderGraphNode>[];
    if (read != null) {
      for (final n in read.$3['nodes'] as List? ?? const []) {
        if (n is Map && n['id'] is int) {
          settings[n['id'] as int] =
              (n['settings'] as Map?)?.cast<String, dynamic>() ?? {};
        }
      }
      for (final e in read.$3['edges'] as List? ?? const []) {
        if (e is Map) edges.add(e.cast<String, dynamic>());
      }
      nodes = shaderGraphView(graph: jsonEncode(read.$3)).nodes;
    }
    setState(() {
      _nodes = nodes;
      _settings = settings;
      _edges = edges;
    });
  }

  /// One box's settings, changed by [edit], as one commit.
  void _write(int id, void Function(Map<String, dynamic> settings) edit) {
    final read = _read();
    if (read == null) return;
    final (stack, inst, graph) = read;
    for (final n in graph['nodes'] as List? ?? const []) {
      if (n is Map && n['id'] == id) {
        final settings = Map<String, dynamic>.of(
            (n['settings'] as Map?)?.cast<String, dynamic>() ?? const {});
        edit(settings);
        n['settings'] = settings;
      }
    }
    try {
      inst.setShaderGraph(graph: jsonEncode(graph));
      _home.commit(stack);
    } catch (_) {
      // Refused, or the stack moved under us; the reload shows what stands.
    }
    _ui?.model.refresh();
  }

  /// A box's menu, or with [id] null the shader's ground menu, opened by the
  /// canvas, which owns every row of it. [hold] keeps the list on screen
  /// until the menu goes.
  void _askMenu(int? id, Offset at, {bool hold = false}) {
    final ui = _ui;
    if (ui == null) return;
    if (hold) setState(() => _holdIndex = true);
    ui.graphBoxMenuAsk.value = null;
    ui.graphBoxMenuAsk.value = (key: shaderMenuKey(id), at: at);
    if (!hold) return;
    if (lumitPopupUp.value) {
      lumitPopupUp.addListener(_menuGone);
    } else {
      setState(() => _holdIndex = false);
    }
  }

  void _menuGone() {
    if (lumitPopupUp.value) return;
    lumitPopupUp.removeListener(_menuGone);
    if (mounted) setState(() => _holdIndex = false);
  }

  String _boxName(BridgeShaderGraphNode n) => switch (n.label) {
        final own? => '${shaderNodeWord(n.kind)} $own',
        null => shaderNodeWord(n.kind),
      };

  @override
  Widget build(BuildContext context) {
    final ui = _ui;
    final pickedId = ui?.shaderGraphNode.value;
    final picked = _nodes.where((n) => n.id == pickedId).firstOrNull;
    if (picked == null || _holdIndex) {
      void groundMenu(Offset at) => _askMenu(null, at);
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          widget.header(widget.entry.effectName, onMenu: groundMenu),
          Expanded(
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onSecondaryTapUp: (d) => groundMenu(d.globalPosition),
              child: _index(),
            ),
          ),
        ],
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        widget.header(
          _boxName(picked),
          crumb: widget.entry.effectName,
          onCrumb: () => ui?.shaderGraphNode.value = null,
          onMenu: (at) => _askMenu(picked.id, at),
        ),
        Expanded(
          child: EdgeFade(
            axis: Axis.vertical,
            child: ListView(
              padding: const EdgeInsets.symmetric(vertical: 4),
              children: [
                if (picked.kind == 'param') ...[
                  ..._paramRows(picked.id),
                  const SizedBox(height: 8),
                ],
                ..._portRows(picked),
              ],
            ),
          ),
        ),
      ],
    );
  }

  /// Every box, a click from being picked: the layer index's rows.
  Widget _index() {
    final t = ThemeScope.of(context).theme;
    return EdgeFade(
      axis: Axis.vertical,
      child: ListView(
        padding: const EdgeInsets.symmetric(vertical: 4),
        children: [
          for (final n in _nodes)
            HoverMark(
              builder: (context, lit) => GestureDetector(
                key: ValueKey<String>('node-shader-index-${n.id}'),
                behavior: HitTestBehavior.opaque,
                onTap: () => _ui?.shaderGraphNode.value = n.id,
                onSecondaryTapUp: (d) =>
                    _askMenu(n.id, d.globalPosition, hold: true),
                child: Container(
                  height: t.density.laneRow,
                  padding: const EdgeInsets.symmetric(horizontal: 10),
                  color: t.surface2.withValues(alpha: lit),
                  child: Row(
                    children: [
                      SizedBox(
                        width: 18,
                        child: lumitIcon(LumitIcon.nodes,
                            size: 12, color: t.textMuted),
                      ),
                      Expanded(
                        child: Text(
                          shaderNodeWord(n.kind),
                          style: t.body,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                      if (n.label case final own?)
                        Flexible(
                          child: Text(own,
                              style: t.kicker,
                              overflow: TextOverflow.ellipsis),
                        ),
                    ],
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }

  /// A box's sockets, as the card has them: each input with the box that
  /// feeds it, then each output with every box it feeds. A click on the
  /// other box picks it.
  List<Widget> _portRows(BridgeShaderGraphNode box) {
    final byId = {for (final n in _nodes) n.id: n};
    // The far box, and its socket's word where it has more than one.
    String far(BridgeShaderGraphNode n, int port, {required bool isInput}) {
      final ports = isInput ? n.inputs : n.outputs;
      return ports.length > 1 && port < ports.length
          ? '${_boxName(n)} · ${shaderPortWord(ports[port].id)}'
          : _boxName(n);
    }

    Widget input(int i, BridgeShaderPort port) {
      final edge = _edges
          .where((e) => e['to'] == box.id && (e['to_port'] ?? 0) == i)
          .firstOrNull;
      final from = byId[edge?['from']];
      return _portRow(port, i,
          isInput: true,
          other: from,
          otherWord: from == null
              ? ''
              : far(from, edge?['from_port'] as int? ?? 0, isInput: false));
    }

    return [
      for (final (i, port) in box.inputs.indexed) input(i, port),
      if (box.inputs.isNotEmpty && box.outputs.isNotEmpty)
        const SizedBox(height: 8),
      for (final (i, port) in box.outputs.indexed) ...[
        for (final edge in _edges.where(
            (e) => e['from'] == box.id && (e['from_port'] ?? 0) == i))
          if (byId[edge['to']] case final to?)
            _portRow(port, i, isInput: false, other: to,
                otherWord: far(to, edge['to_port'] as int? ?? 0, isInput: true)),
        if (!_edges
            .any((e) => e['from'] == box.id && (e['from_port'] ?? 0) == i))
          _portRow(port, i, isInput: false),
      ],
    ];
  }

  /// One socket's row: its dot in its type's colour, filled while wired, then
  /// its word and the box at the wire's other end.
  Widget _portRow(BridgeShaderPort port, int index,
      {required bool isInput,
      BridgeShaderGraphNode? other,
      String otherWord = ''}) {
    final t = ThemeScope.of(context).theme;
    final colour = shaderPortColour(t, port.ty);
    final dot = Padding(
      padding: EdgeInsets.symmetric(horizontal: (fxKeyColumnWidth - 7) / 2),
      child: Container(
        width: 7,
        height: 7,
        decoration: BoxDecoration(
          color: other == null ? null : colour,
          shape: BoxShape.circle,
          border: Border.all(color: colour),
        ),
      ),
    );
    return KeyedSubtree(
      key: ValueKey<String>(
          'node-shader-port-${isInput ? 'in' : 'out'}-$index-${other?.id}'),
      child: fxTwoColumnRow(
        context: context,
        name: Text(shaderPortWord(port.id),
            style: t.body, overflow: TextOverflow.ellipsis),
        keyframeControls: dot,
        control: other == null
            ? const SizedBox.shrink()
            : HoverMark(
                cursor: SystemMouseCursors.click,
                builder: (context, lit) => GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onTap: () => _ui?.shaderGraphNode.value = other.id,
                  child: Text(
                    otherWord,
                    style: t.body.copyWith(
                        color: Color.lerp(t.textSecondary, t.textPrimary, lit)),
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ),
      ),
    );
  }

  static const _kinds = ['slider', 'bounded', 'dial', 'colour', 'point'];
  static const _units = ['', 'px', 'deg', 's'];

  String _kindWord(String kind) => switch (kind) {
        'bounded' => l10n.shaderParamBounded,
        'dial' => l10n.shaderParamDial,
        'colour' => l10n.shaderParamColour,
        'point' => l10n.shaderParamPoint,
        _ => l10n.shaderParamSlider,
      };

  String _unitWord(String unit) => switch (unit) {
        'px' => l10n.unitPixels,
        'deg' => l10n.unitDegrees,
        's' => l10n.unitSeconds,
        _ => l10n.unitRaw,
      };

  /// A Parameter box's facts: what kind of control it is, where it starts, and
  /// for a slider its range and unit. The name is the box's own, from Rename.
  List<Widget> _paramRows(int id) {
    final s = _settings[id] ?? const <String, dynamic>{};
    final kind = _kinds.contains(s['kind']) ? s['kind'] as String : 'slider';
    final channels = switch (kind) { 'colour' => 4, 'point' => 2, _ => 1 };
    final fallback = kind == 'colour' ? 1.0 : 0.0;
    final raw = s['default'];
    final defaults = [
      for (var i = 0; i < channels; i++)
        switch (raw) {
          final List<dynamic> list when i < list.length && list[i] is num =>
            (list[i] as num).toDouble(),
          final num one when i == 0 => one.toDouble(),
          _ => fallback,
        },
    ];
    final ranged = kind == 'slider' || kind == 'bounded';
    final unit = _units.contains(s['unit']) ? s['unit'] as String : '';
    return [
      _row(
        l10n.graphInputKind,
        BareDropdown<String>(
          key: const ValueKey('node-shader-kind'),
          value: kind,
          options: _kinds,
          label: _kindWord,
          // Another kind takes its own default, of its own width.
          onChanged: (next) => _write(id, (s) {
            s['kind'] = next;
            s.remove('default');
          }),
        ),
      ),
      _row(
        l10n.graphInputDefault,
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (var i = 0; i < channels; i++) ...[
              if (i > 0) const SizedBox(width: 4),
              _number('node-shader-default-$i', defaults[i], (v) {
                final next = [...defaults]..[i] = v;
                _write(id, (s) => s['default'] = channels == 1 ? v : next);
              }),
            ],
          ],
        ),
      ),
      if (ranged) ...[
        _row(
          l10n.graphInputMin,
          _number('node-shader-min', (s['min'] as num?)?.toDouble() ?? 0,
              (v) => _write(id, (s) => s['min'] = v)),
        ),
        _row(
          l10n.graphInputMax,
          _number('node-shader-max', (s['max'] as num?)?.toDouble() ?? 1,
              (v) => _write(id, (s) => s['max'] = v)),
        ),
        _row(
          l10n.graphInputUnit,
          BareDropdown<String>(
            key: const ValueKey('node-shader-unit'),
            value: unit,
            options: _units,
            label: _unitWord,
            onChanged: (next) => _write(id, (s) {
              next.isEmpty ? s.remove('unit') : s['unit'] = next;
            }),
          ),
        ),
      ],
    ];
  }

  /// One row on the Node panel's own columns: the name where a layer's rows
  /// put theirs, the control in their value column.
  Widget _row(String name, Widget control) {
    final t = ThemeScope.of(context).theme;
    return fxTwoColumnRow(
      context: context,
      name: Text(name, style: t.body, overflow: TextOverflow.ellipsis),
      control: control,
    );
  }

  Widget _number(String key, double value, ValueChanged<double> onChanged) =>
      SizedBox(
        width: effectCellWidth,
        child: DragValueField(
          key: ValueKey<String>(key),
          value: _drag?.key == key ? _drag!.value : value,
          min: -1000000,
          max: 1000000,
          decimals: 3,
          speed: 0.5,
          onChangeLive: (v) =>
              setState(() => _drag = (key: key, value: v.toDouble())),
          onDragCancel: () => setState(() => _drag = null),
          onChanged: (v) {
            setState(() => _drag = null);
            onChanged(v.toDouble());
          },
        ),
      );
}
