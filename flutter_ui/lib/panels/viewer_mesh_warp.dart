// The grid of a selected Mesh warp, drawn over its layer in the Viewer.
//
// As After Effects draws a Mesh Warp's Distortion Mesh: the lines of the grid,
// bent as the picture is, and a point where every two of them cross. With the
// Selection tool in hand a point can be dragged; the picture follows through
// the same preview an effect row's drag uses, and the release writes it as one
// undo step.
//
// A crossing is two of the instance's own rows, in per cent of the layer: 0, 0
// its upper left corner and 100, 100 its lower right. So a point is pushed
// through the layer's own transform onto the screen, and the pointer back
// through it, the way a mask's points are.
//
// With Animate mesh on, a drag keys every crossing at the playhead, so a key
// holds the whole mesh, as After Effects' stopwatch on Distortion Mesh does.
// With it off, a crossing already keyed takes a key and the others stay still.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/state/tools.dart';

import 'effect_param_row_frb.dart' show EffectStackEditor, scalarValueAt;
import 'graph_maths.dart' show keyframeAmong;
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'viewer_anchor.dart' show constrainToAxis;
import 'viewer_gizmo.dart' show LayerBox, gizmoAnchorSlop;
import 'viewer_tool_cursor.dart' show paintAnchorMark;

/// The id of one crossing's `axis`, as the engine's `mesh_warp::vertex_id`
/// spells it.
String meshVertexId(int rows, int columns, int r, int c, String axis) =>
    'mesh_${rows}x${columns}_r${r}c${c}_$axis';

/// One selected Mesh warp on one layer, as the overlay reads it.
class _Mesh {
  final LayerBox box;
  final BridgeEffectInstanceInfo fx;
  final int rows, columns;

  /// Every crossing in per cent of the layer, row by row.
  final List<Offset> pts;

  /// Every crossing's two rows as the document holds them, row by row.
  final List<(BridgeScalar?, BridgeScalar?)> held;
  final bool animate;

  const _Mesh(this.box, this.fx, this.rows, this.columns, this.pts, this.held,
      this.animate);

  Offset toScreen(Offset pct) => box.map.toScreen(
      pct.dx / 100 * box.bounds.width, pct.dy / 100 * box.bounds.height);

  Offset toPercent(Offset screen) {
    final at = box.map.layerOf(screen);
    final w = box.bounds.width == 0 ? 1.0 : box.bounds.width;
    final h = box.bounds.height == 0 ? 1.0 : box.bounds.height;
    return Offset(at.dx / w * 100, at.dy / h * 100);
  }
}

class ViewerMeshWarp extends StatefulWidget {
  final CompositionReference comp;
  final LumitUiState uiState;

  /// The layers as the Viewer places them, asked for at build time.
  final List<LayerBox> Function() boxes;
  final Color colour;

  /// The armed tool. Only Selection drags a point.
  final ToolMode tool;

  /// Told once a drag has been written.
  final VoidCallback onChanged;

  const ViewerMeshWarp({
    super.key,
    required this.comp,
    required this.uiState,
    required this.boxes,
    required this.colour,
    required this.tool,
    required this.onChanged,
  });

  @override
  State<ViewerMeshWarp> createState() => _ViewerMeshWarpState();
}

class _ViewerMeshWarpState extends State<ViewerMeshWarp> {
  final EffectStackEditor _editor = EffectStackEditor();

  /// The drag in flight: the mesh, the crossing, where the press was and the
  /// point it started from, and where it is now.
  _Mesh? _mesh;
  int _index = -1;
  Offset _down = Offset.zero;
  Offset _from = Offset.zero;
  Offset? _to;
  int? _hover;

  LumitUiState get _ui => widget.uiState;

  @override
  void dispose() {
    _editor.clear();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Positioned.fill(
        child: ListenableBuilder(
          listenable: Listenable.merge([
            _ui.selectedEffects,
            _ui.model,
            _ui.playheadFrame,
            EffectStackEditor.staging,
          ]),
          builder: (context, _) {
            final meshes = _meshes();
            if (meshes.isEmpty) return const SizedBox.shrink();
            return Stack(children: [
              Positioned.fill(
                child: IgnorePointer(
                  child: CustomPaint(
                    painter: _MeshPainter(
                      [for (final m in meshes) _drawn(m)],
                      widget.colour,
                    ),
                  ),
                ),
              ),
              if (widget.tool.group == ToolGroup.select)
                for (final m in meshes)
                  for (var i = 0; i < m.pts.length; i++) _grip(m, i),
            ]);
          },
        ),
      );

  /// The lines and points of one mesh on the screen.
  _Drawn _drawn(_Mesh m) {
    final cols = m.columns + 1;
    Offset at(int r, int c) => m.pts[r * cols + c];
    final lines = <List<Offset>>[];
    // Each line of the grid, bent as the engine's Catmull-Rom surface bends
    // it, sampled eight times a cell.
    for (var r = 0; r <= m.rows; r++) {
      lines.add([
        for (final p in _curve([for (var c = 0; c <= m.columns; c++) at(r, c)]))
          m.toScreen(p)
      ]);
    }
    for (var c = 0; c <= m.columns; c++) {
      lines.add([
        for (final p in _curve([for (var r = 0; r <= m.rows; r++) at(r, c)]))
          m.toScreen(p)
      ]);
    }
    final key = '${m.box.id}/${m.fx.id}';
    return _Drawn(
      lines,
      [for (final p in m.pts) m.toScreen(p)],
      _mesh != null && '${_mesh!.box.id}/${_mesh!.fx.id}' == key
          ? _index
          : (_hover != null && _hoverKey == key ? _hover : null),
    );
  }

  String? _hoverKey;

  /// A line of crossings as a Catmull-Rom curve, the ends continued straight.
  static List<Offset> _curve(List<Offset> pts) {
    if (pts.length < 2) return pts;
    Offset p(int i) {
      if (i < 0) return pts[0] * 2 - pts[1];
      if (i >= pts.length) {
        return pts[pts.length - 1] * 2 - pts[pts.length - 2];
      }
      return pts[i];
    }

    const steps = 8;
    final out = <Offset>[pts[0]];
    for (var i = 0; i < pts.length - 1; i++) {
      final p0 = p(i - 1), p1 = p(i), p2 = p(i + 1), p3 = p(i + 2);
      for (var s = 1; s <= steps; s++) {
        final t = s / steps, t2 = t * t, t3 = t2 * t;
        out.add((p0 * (-t + 2 * t2 - t3) +
                p1 * (2 - 5 * t2 + 3 * t3) +
                p2 * (t + 4 * t2 - 3 * t3) +
                p3 * (-t2 + t3)) *
            0.5);
      }
    }
    return out;
  }

  Widget _grip(_Mesh m, int i) {
    final at = m.toScreen(m.pts[i]);
    final key = '${m.box.id}/${m.fx.id}';
    return Positioned(
      key: ValueKey<String>('viewer-mesh-point-$key-$i'),
      left: at.dx - gizmoAnchorSlop / 2,
      top: at.dy - gizmoAnchorSlop / 2,
      width: gizmoAnchorSlop,
      height: gizmoAnchorSlop,
      child: MouseRegion(
        cursor: SystemMouseCursors.move,
        onEnter: (_) => setState(() {
          _hover = i;
          _hoverKey = key;
        }),
        onExit: (_) => setState(() => _hover = null),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onPanDown: (d) => _start(
              m,
              i,
              at +
                  d.localPosition -
                  const Offset(gizmoAnchorSlop / 2, gizmoAnchorSlop / 2)),
          onPanUpdate: (d) => _move(at +
              d.localPosition -
              const Offset(gizmoAnchorSlop / 2, gizmoAnchorSlop / 2)),
          onPanEnd: (_) => _end(),
          onPanCancel: _cancel,
        ),
      ),
    );
  }

  void _start(_Mesh m, int i, Offset screen) {
    _cancel();
    setState(() {
      _mesh = m;
      _index = i;
      _down = screen;
      _from = m.pts[i];
    });
  }

  void _move(Offset screen) {
    final m = _mesh;
    if (m == null) return;
    var d = screen - _down;
    // Shift keeps it to one axis, as it does for a layer's move.
    if (HardwareKeyboard.instance.isShiftPressed) d = constrainToAxis(d);
    final to = m.toPercent(m.toScreen(_from) + d);
    _to = to;
    final frame = _ui.playheadFrame.value;
    final (r, c) = (_index ~/ (m.columns + 1), _index % (m.columns + 1));
    for (final (axis, v) in [('x', to.dx), ('y', to.dy)]) {
      final id = meshVertexId(m.rows, m.columns, r, c, axis);
      _editor.live(widget.comp, m.box.layer, m.fx.id, id,
          BridgeEffectValue.float(BridgeScalar.static_(v)),
          frame: frame, scale: _ui.viewerScale);
    }
  }

  void _end() {
    final m = _mesh, to = _to;
    if (m != null && to != null) {
      final frame = _ui.playheadFrame.value;
      final cols = m.columns + 1;
      final values = <String, BridgeEffectValue>{};
      for (var i = 0; i < m.pts.length; i++) {
        final moved = i == _index;
        final (r, c) = (i ~/ cols, i % cols);
        final (xWas, yWas) = m.held[i];
        // A crossing the drag did not touch is written only when the whole
        // mesh is being keyed.
        if (!moved && !m.animate) continue;
        final p = moved ? to : m.pts[i];
        values[meshVertexId(m.rows, m.columns, r, c, 'x')] =
            BridgeEffectValue.float(_scalar(xWas, p.dx, frame, m.animate));
        values[meshVertexId(m.rows, m.columns, r, c, 'y')] =
            BridgeEffectValue.float(_scalar(yWas, p.dy, frame, m.animate));
      }
      _editor.writeAll(m.box.layer, m.fx.id, values);
      widget.onChanged();
    }
    setState(() {
      _mesh = null;
      _to = null;
    });
  }

  /// The value `v` at `frame`: keyed when the row already is or the mesh is
  /// being keyed, still otherwise.
  BridgeScalar _scalar(BridgeScalar? was, double v, int frame, bool key) {
    if (was is BridgeScalar_Keyframed) {
      return scalarWithValueAt(was, v, widget.comp, frame);
    }
    if (!key) return BridgeScalar.static_(v);
    return BridgeScalar.keyframed([
      keyframeAmong(const [], widget.comp.timeOfFrame(frame: frame), v),
    ]);
  }

  void _cancel() {
    if (_mesh == null) return;
    _editor.clear();
    setState(() {
      _mesh = null;
      _to = null;
    });
  }

  /// Every Mesh warp picked in the Effect controls, on a layer the Viewer
  /// places.
  List<_Mesh> _meshes() {
    final picked = _ui.selectedEffects.value;
    if (picked.isEmpty) return const [];
    final owner = _ui.selectedEffectsLayer?.internallayerId;
    if (owner == null) return const [];
    final staged = EffectStackEditor.staging.value;
    final frame = _ui.playheadFrame.value;
    final out = <_Mesh>[];
    for (final box in widget.boxes()) {
      if (box.layer.internallayerId != owner) continue;
      if (box.bounds.isEmpty) continue;
      final entry = _ui.model.byId(owner);
      if (entry == null) continue;
      for (final fx in entry.info.effects) {
        if (fx.name != 'mesh_warp' || !picked.contains(fx.id)) continue;
        BridgeScalar? held(String id) =>
            switch (fx.values.where((v) => v.id == id).firstOrNull?.value) {
              BridgeEffectValue_Float(:final field0) => field0,
              _ => null,
            };
        double number(String id, double fallback) {
          final s = switch (staged[(fx.id, id)]) {
            BridgeEffectValue_Float(:final field0) => field0,
            _ => held(id),
          };
          return scalarValueAt(s, widget.comp, frame) ?? fallback;
        }

        final rows = number('rows', 7).round().clamp(1, 31);
        final columns = number('columns', 7).round().clamp(1, 31);
        final animate = switch (
            fx.values.where((v) => v.id == 'animate_mesh').firstOrNull?.value) {
          BridgeEffectValue_Bool(:final field0) => field0,
          _ => false,
        };
        final pts = <Offset>[];
        final heldRows = <(BridgeScalar?, BridgeScalar?)>[];
        for (var r = 0; r <= rows; r++) {
          for (var c = 0; c <= columns; c++) {
            final x = meshVertexId(rows, columns, r, c, 'x');
            final y = meshVertexId(rows, columns, r, c, 'y');
            pts.add(Offset(
                number(x, c * 100 / columns), number(y, r * 100 / rows)));
            heldRows.add((held(x), held(y)));
          }
        }
        out.add(_Mesh(box, fx, rows, columns, pts, heldRows, animate));
      }
    }
    return out;
  }
}

/// One mesh as it is painted.
class _Drawn {
  final List<List<Offset>> lines;
  final List<Offset> points;

  /// The crossing under the pointer or in hand.
  final int? lit;

  const _Drawn(this.lines, this.points, this.lit);
}

class _MeshPainter extends CustomPainter {
  final List<_Drawn> meshes;
  final Color colour;

  const _MeshPainter(this.meshes, this.colour);

  @override
  void paint(Canvas canvas, Size size) {
    final line = Paint()
      ..color = colour.withValues(alpha: 0.8)
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    for (final m in meshes) {
      for (final l in m.lines) {
        if (l.length < 2) continue;
        final path = Path()..moveTo(l.first.dx, l.first.dy);
        for (final p in l.skip(1)) {
          path.lineTo(p.dx, p.dy);
        }
        canvas.drawPath(path, line);
      }
      for (var i = 0; i < m.points.length; i++) {
        final lit = m.lit == i;
        if (lit) {
          canvas.drawCircle(
              m.points[i], 4, Paint()..color = colour.withValues(alpha: 0.35));
        }
        paintAnchorMark(canvas, m.points[i], colour, reach: lit ? 10 : 6);
      }
    }
  }

  @override
  bool shouldRepaint(_MeshPainter old) => true;
}
