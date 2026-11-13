// The grid of a selected Mesh warp, drawn over its layer in the Viewer.
//
// As After Effects draws a Mesh Warp's Distortion Mesh: the lines of the grid,
// bent as the picture is, and a point where every two of them cross. With the
// Selection tool in hand a point can be dragged, and a click on one shows its
// four handles, which drag too: the two along its row and the two along its
// column, each pair mirrored. The picture follows through the same preview an
// effect row's drag uses, and the release writes it as one undo step.
//
// A crossing is six of the instance's own rows, in per cent of the layer: where
// it is (0, 0 the layer's upper left corner, 100, 100 its lower right) and how
// far its row handle and column handle reach. So a point is pushed through the
// layer's own transform onto the screen, and the pointer back through it, the
// way a mask's points are.
//
// With Animate mesh on, a drag keys every crossing at the playhead, so a key
// holds the whole mesh, as After Effects' stopwatch on Distortion Mesh does.
// With it off, a row already keyed takes a key and the others stay still.

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

/// The six rows of a crossing, as the engine's `mesh_warp::AXES` names them.
const meshAxes = ['x', 'y', 'hx', 'hy', 'vx', 'vy'];

/// What a press holds: the crossing itself, or one end of one of its handles.
enum _Grip { point, rowAfter, rowBefore, columnAfter, columnBefore }

/// One selected Mesh warp on one layer, as the overlay reads it.
class _Mesh {
  final LayerBox box;
  final BridgeEffectInstanceInfo fx;
  final int rows, columns;

  /// Every crossing, its row handle and its column handle, in per cent of the
  /// layer, row by row.
  final List<Offset> pts, hs, vs;

  /// Every crossing's six rows as the document holds them, row by row.
  final List<List<BridgeScalar?>> held;
  final bool animate;

  const _Mesh(this.box, this.fx, this.rows, this.columns, this.pts, this.hs,
      this.vs, this.held, this.animate);

  String get key => '${box.id}/${fx.id}';

  Offset toScreen(Offset pct) => box.map.toScreen(
      pct.dx / 100 * box.bounds.width, pct.dy / 100 * box.bounds.height);

  Offset toPercent(Offset screen) {
    final at = box.map.layerOf(screen);
    final w = box.bounds.width == 0 ? 1.0 : box.bounds.width;
    final h = box.bounds.height == 0 ? 1.0 : box.bounds.height;
    return Offset(at.dx / w * 100, at.dy / h * 100);
  }

  /// Where `grip` of crossing `i` is, per cent of the layer.
  Offset gripAt(int i, _Grip grip) => switch (grip) {
        _Grip.point => pts[i],
        _Grip.rowAfter => pts[i] + hs[i],
        _Grip.rowBefore => pts[i] - hs[i],
        _Grip.columnAfter => pts[i] + vs[i],
        _Grip.columnBefore => pts[i] - vs[i],
      };
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

  /// The crossing whose handles are shown: its mesh's key and its index.
  (String, int)? _picked;

  /// The drag in flight: the mesh, the crossing and what of it is held, how
  /// far the pointer has gone since the press, where the held thing started,
  /// and where it is now.
  _Mesh? _mesh;
  int _index = -1;
  _Grip _grip = _Grip.point;
  Offset _travel = Offset.zero;
  Offset _from = Offset.zero;
  Offset? _to;

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
            final select = widget.tool.group == ToolGroup.select;
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
              if (select)
                for (final m in meshes) ...[
                  for (var i = 0; i < m.pts.length; i++)
                    _gripWidget(m, i, _Grip.point),
                  if (_picked case (final key, final i)
                      when key == m.key && i < m.pts.length)
                    for (final g in _Grip.values.skip(1)) _gripWidget(m, i, g),
                ],
            ]);
          },
        ),
      );

  /// The lines, points and shown handles of one mesh on the screen.
  _Drawn _drawn(_Mesh m) {
    final cols = m.columns + 1;
    final curves = <List<Offset>>[];
    // Each edge of each cell, the cubic its two crossings and their handles
    // draw, as the engine's patches have it.
    for (var r = 0; r <= m.rows; r++) {
      for (var c = 0; c <= m.columns; c++) {
        final i = r * cols + c;
        if (c < m.columns) {
          final j = i + 1;
          curves.add([
            m.toScreen(m.pts[i]),
            m.toScreen(m.pts[i] + m.hs[i]),
            m.toScreen(m.pts[j] - m.hs[j]),
            m.toScreen(m.pts[j]),
          ]);
        }
        if (r < m.rows) {
          final j = i + cols;
          curves.add([
            m.toScreen(m.pts[i]),
            m.toScreen(m.pts[i] + m.vs[i]),
            m.toScreen(m.pts[j] - m.vs[j]),
            m.toScreen(m.pts[j]),
          ]);
        }
      }
    }
    final picked = switch (_picked) {
      (final key, final i) when key == m.key && i < m.pts.length => i,
      _ => null,
    };
    return _Drawn(
      curves,
      [for (final p in m.pts) m.toScreen(p)],
      picked == null
          ? const []
          : [
              for (final g in _Grip.values.skip(1))
                (m.toScreen(m.pts[picked]), m.toScreen(m.gripAt(picked, g))),
            ],
      picked,
    );
  }

  Widget _gripWidget(_Mesh m, int i, _Grip grip) {
    final at = m.toScreen(m.gripAt(i, grip));
    const half = Offset(gizmoAnchorSlop / 2, gizmoAnchorSlop / 2);
    return Positioned(
      key: ValueKey<String>('viewer-mesh-${grip.name}-${m.key}-$i'),
      left: at.dx - half.dx,
      top: at.dy - half.dy,
      width: gizmoAnchorSlop,
      height: gizmoAnchorSlop,
      child: MouseRegion(
        cursor: SystemMouseCursors.move,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onPanDown: (_) => _start(m, i, grip),
          // The pointer's travel, not its place: a drag's events keep the
          // grip's box as it was at the press, while the grip itself follows
          // the point, so a place read off the moved grip ran ahead of the
          // pointer by however far the grip had gone, faster at every tick.
          onPanUpdate: (d) => _move(d.delta),
          onPanEnd: (_) => _end(),
          onPanCancel: _cancel,
        ),
      ),
    );
  }

  void _start(_Mesh m, int i, _Grip grip) {
    _cancel();
    setState(() {
      _mesh = m;
      _index = i;
      _grip = grip;
      _travel = Offset.zero;
      _from = m.gripAt(i, grip);
      // A press on a crossing shows its handles, as in After Effects.
      if (grip == _Grip.point) _picked = (m.key, i);
    });
  }

  /// The two rows a drag of the held grip writes, with the values it gives
  /// them for the held thing at `to` (per cent of the layer).
  Map<String, double> _rowsFor(_Mesh m, Offset to) {
    final (r, c) = (_index ~/ (m.columns + 1), _index % (m.columns + 1));
    String id(String axis) => meshVertexId(m.rows, m.columns, r, c, axis);
    final p = m.pts[_index];
    return switch (_grip) {
      _Grip.point => {id('x'): to.dx, id('y'): to.dy},
      _Grip.rowAfter => {id('hx'): to.dx - p.dx, id('hy'): to.dy - p.dy},
      _Grip.rowBefore => {id('hx'): p.dx - to.dx, id('hy'): p.dy - to.dy},
      _Grip.columnAfter => {id('vx'): to.dx - p.dx, id('vy'): to.dy - p.dy},
      _Grip.columnBefore => {id('vx'): p.dx - to.dx, id('vy'): p.dy - to.dy},
    };
  }

  void _move(Offset delta) {
    final m = _mesh;
    if (m == null) return;
    _travel += delta;
    var d = _travel;
    // Shift keeps it to one axis, as it does for a layer's move.
    if (HardwareKeyboard.instance.isShiftPressed) d = constrainToAxis(d);
    final to = m.toPercent(m.toScreen(_from) + d);
    _to = to;
    final frame = _ui.playheadFrame.value;
    for (final MapEntry(:key, :value) in _rowsFor(m, to).entries) {
      _editor.live(widget.comp, m.box.layer, m.fx.id, key,
          BridgeEffectValue.float(BridgeScalar.static_(value)),
          frame: frame, scale: _ui.viewerScale);
    }
  }

  void _end() {
    final m = _mesh, to = _to;
    if (m != null && to != null) {
      final frame = _ui.playheadFrame.value;
      final moved = _rowsFor(m, to);
      final cols = m.columns + 1;
      final values = <String, BridgeEffectValue>{};
      for (var i = 0; i < m.pts.length; i++) {
        final (r, c) = (i ~/ cols, i % cols);
        final now = [
          m.pts[i].dx,
          m.pts[i].dy,
          m.hs[i].dx,
          m.hs[i].dy,
          m.vs[i].dx,
          m.vs[i].dy,
        ];
        for (var a = 0; a < meshAxes.length; a++) {
          final id = meshVertexId(m.rows, m.columns, r, c, meshAxes[a]);
          final drag = moved[id];
          // A row the drag did not touch is written only when the whole mesh
          // is being keyed.
          if (drag == null && !m.animate) continue;
          values[id] = BridgeEffectValue.float(
              _scalar(m.held[i][a], drag ?? now[a], frame, m.animate));
        }
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
        // Read into a map once: a big grid holds thousands of rows.
        final values = {for (final v in fx.values) v.id: v.value};
        BridgeScalar? held(String id) => switch (values[id]) {
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
        // A fresh handle reaches a third of a cell, as the engine's
        // `rest_handles` has it.
        final hRest = 100 / columns / 3, vRest = 100 / rows / 3;
        final pts = <Offset>[], hs = <Offset>[], vs = <Offset>[];
        final heldRows = <List<BridgeScalar?>>[];
        for (var r = 0; r <= rows; r++) {
          for (var c = 0; c <= columns; c++) {
            String id(String axis) => meshVertexId(rows, columns, r, c, axis);
            pts.add(Offset(number(id('x'), c * 100 / columns),
                number(id('y'), r * 100 / rows)));
            hs.add(Offset(number(id('hx'), hRest), number(id('hy'), 0)));
            vs.add(Offset(number(id('vx'), 0), number(id('vy'), vRest)));
            heldRows.add([for (final a in meshAxes) held(id(a))]);
          }
        }
        out.add(_Mesh(box, fx, rows, columns, pts, hs, vs, heldRows, animate));
      }
    }
    return out;
  }
}

/// One mesh as it is painted.
class _Drawn {
  /// Each edge of each cell as a cubic: start, two controls, end.
  final List<List<Offset>> curves;
  final List<Offset> points;

  /// The picked crossing's handles, each from the crossing to its end.
  final List<(Offset, Offset)> handles;
  final int? picked;

  const _Drawn(this.curves, this.points, this.handles, this.picked);
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
      final path = Path();
      for (final c in m.curves) {
        path
          ..moveTo(c[0].dx, c[0].dy)
          ..cubicTo(c[1].dx, c[1].dy, c[2].dx, c[2].dy, c[3].dx, c[3].dy);
      }
      canvas.drawPath(path, line);
      for (var i = 0; i < m.points.length; i++) {
        paintAnchorMark(canvas, m.points[i], colour,
            reach: m.picked == i ? 10 : 6);
      }
      final fill = Paint()..color = colour;
      for (final (from, to) in m.handles) {
        canvas
          ..drawLine(from, to, line)
          ..drawCircle(to, 3, fill);
      }
    }
  }

  @override
  bool shouldRepaint(_MeshPainter old) => true;
}
