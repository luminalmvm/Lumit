// The current Correction's windows, drawn and moved on the picture.
//
// A window is a shape in the shot's own pixels: a centre, a size and a turn,
// and for a polygon its points in that box. Each slot that is on draws its
// outline and its two softness rings; the slot picked in the Grade panel
// grows handles for everything that is a place on the picture. A drag
// previews through the effect editor, as a row's drag does, and the release
// writes every row it moved as one undo step. A keyed row takes a key at the
// playhead.
//
// A polygon is drawn point by point: while the picked slot is a polygon with
// fewer than three points, each click on the picture adds one, and Enter, or a
// click on the first point, closes it. Escape lets go of what was drawn.

import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/engine_labels.dart';
import '../theme/theme.dart';
import '../state/grade_target.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart'
    show EffectStackEditor, cachedListParameters, scalarValueAt;
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'timeline_extras_frb.dart' show showMenuAt;
import 'viewer_grade.dart';
import 'viewer_layer_map.dart';
import 'viewer_menus.dart' show viewerRow;

/// The rows that place window [slot] when it is turned on: the middle of the
/// shot's own picture, at 40% of its size, in its own pixels.
Map<String, BridgeEffectValue> gradeWindowStart(
    LumitUiState ui, GradeShot shot, int slot) {
  final comp = ui.selectedComp;
  final entry = ui.model.byId(shot.layer.internallayerId);
  if (comp == null || entry == null) return const {};
  final size = ui.layerBounds.boundsOf(entry,
      compSize: comp.getSize(), revision: ui.model.heldRevision);
  BridgeEffectValue v(double x) =>
      BridgeEffectValue.float(BridgeScalar.static_(x));
  return {
    'win${slot}_x': v(size.width / 2),
    'win${slot}_y': v(size.height / 2),
    'win${slot}_w': v(size.width * 0.4),
    'win${slot}_h': v(size.height * 0.4),
  };
}

/// One window slot as it reads at the playhead, a drag's staged values first.
class _Win {
  final int slot, shape;
  final Offset centre;
  final double hw, hh, angle, round, soft, softIn;
  final List<Offset> points;
  final bool smooth;

  const _Win(this.slot, this.shape, this.centre, this.hw, this.hh, this.angle,
      this.round, this.soft, this.softIn, this.points, this.smooth);

  double get _cos => math.cos(angle * math.pi / 180);
  double get _sin => math.sin(angle * math.pi / 180);

  /// The box's own frame to the shot's pixels, and back.
  Offset out(Offset q) => centre +
      Offset(_cos * q.dx - _sin * q.dy, _sin * q.dx + _cos * q.dy);
  Offset local(Offset p) {
    final d = p - centre;
    return Offset(_cos * d.dx + _sin * d.dy, -_sin * d.dx + _cos * d.dy);
  }

  /// The rectangle's corner radius, in pixels.
  double get radius => (round / 100).clamp(0.0, 1.0) * math.min(hw, hh);

  /// The polygon's points in the box's frame, in pixels.
  List<Offset> get box =>
      [for (final p in points) Offset(p.dx * hw, p.dy * hh)];

  /// The outline in the box's frame, grown outwards by [grow] pixels (in
  /// with a negative one).
  List<Offset> outline(double grow) => switch (shape) {
        1 => [
            for (var i = 0; i < 72; i++)
              Offset(math.max(hw + grow, 0) * math.cos(i * math.pi / 36),
                  math.max(hh + grow, 0) * math.sin(i * math.pi / 36)),
          ],
        2 => _rounded(math.max(hw + grow, 0), math.max(hh + grow, 0),
            math.max(radius + grow, 0)),
        3 => _grown(_curve(box, smooth), grow),
        _ => const [],
      };

  static List<Offset> _rounded(double hw, double hh, double r) {
    r = math.min(r, math.min(hw, hh));
    final out = <Offset>[];
    for (final (cx, cy, from) in [
      (hw - r, hh - r, 0.0),
      (-hw + r, hh - r, 0.5),
      (-hw + r, -hh + r, 1.0),
      (hw - r, -hh + r, 1.5),
    ]) {
      for (var i = 0; i <= 8; i++) {
        final a = (from + i / 16) * math.pi;
        out.add(Offset(cx + r * math.cos(a), cy + r * math.sin(a)));
      }
    }
    return out;
  }

  /// The engine's outline: the points, or a closed Catmull-Rom curve through
  /// them cut into four lines a span.
  static List<Offset> _curve(List<Offset> p, bool smooth) {
    final n = p.length;
    if (!smooth || n < 3) return p;
    final out = <Offset>[];
    for (var i = 0; i < n; i++) {
      final p0 = p[(i + n - 1) % n], p1 = p[i];
      final p2 = p[(i + 1) % n], p3 = p[(i + 2) % n];
      for (var s = 0; s < 4; s++) {
        final t = s / 4, t2 = t * t, t3 = t2 * t;
        out.add((p1 * 2 +
                (p2 - p0) * t +
                (p0 * 2 - p1 * 5 + p2 * 4 - p3) * t2 +
                (p1 * 3 - p0 - p2 * 3 + p3) * t3) *
            0.5);
      }
    }
    return out;
  }

  /// [line] moved out along each corner's normal, for a softness ring.
  static List<Offset> _grown(List<Offset> line, double grow) {
    final n = line.length;
    if (n < 3 || grow == 0) return line;
    var area = 0.0;
    for (var i = 0; i < n; i++) {
      final a = line[i], b = line[(i + 1) % n];
      area += a.dx * b.dy - b.dx * a.dy;
    }
    final side = area > 0 ? 1.0 : -1.0;
    Offset normal(Offset a, Offset b) {
      final d = b - a;
      final l = d.distance;
      return l == 0 ? Offset.zero : Offset(d.dy, -d.dx) / l * side;
    }

    return [
      for (var i = 0; i < n; i++)
        () {
          final m = normal(line[(i + n - 1) % n], line[i]) +
              normal(line[i], line[(i + 1) % n]);
          final l = m.distance;
          return l == 0 ? line[i] : line[i] + m / l * grow;
        }(),
    ];
  }
}

/// What a drag on a window holds.
enum _Grip { centre, corner, edgeX, edgeY, turn, soft, softIn, round, aim, point }

class ViewerGradeWindows extends StatefulWidget {
  final LumitUiState uiState;
  final CompositionReference comp;
  final Rect fitted;
  final Size compSize;

  /// The shot's layer onto the screen, or null for a straight comp mapping.
  final ViewerLayerMap? Function(UuidValue layer) mapOf;

  const ViewerGradeWindows({
    super.key,
    required this.uiState,
    required this.comp,
    required this.fitted,
    required this.compSize,
    required this.mapOf,
  });

  @override
  State<ViewerGradeWindows> createState() => _ViewerGradeWindowsState();
}

class _ViewerGradeWindowsState extends State<ViewerGradeWindows> {
  final ViewerGradeWriter _writer = ViewerGradeWriter();
  final FocusNode _focus = FocusNode(debugLabel: 'viewer-grade-windows');

  LumitUiState get _ui => widget.uiState;
  GradeTarget get _g => _ui.grade;

  /// The drag in hand: its grip, the slot as it was, where it started in the
  /// shot's pixels, and the rows it has moved so far.
  _Grip? _grip;
  _Win? _from;
  Offset _down = Offset.zero;
  int _point = -1;
  Map<String, BridgeEffectValue> _moved = {};

  /// The polygon point picked, which Delete removes.
  int? _picked;

  /// A polygon being drawn, in the shot's pixels, and the slot it is for.
  List<Offset>? _drawing;
  int _drawingSlot = 0;

  @override
  void dispose() {
    _writer.clear();
    _focus.dispose();
    super.dispose();
  }

  // --- Reading ---

  late ViewerLayerMap _map;

  Offset _screen(Offset p) => _map.toScreen(p.dx, p.dy);
  Offset _layer(Offset s) => _map.layerOf(s);

  /// Shot pixels per screen pixel, near [at].
  double _px(Offset at) {
    final a = _layer(_screen(at)), b = _layer(_screen(at) + const Offset(1, 0));
    final d = (b - a).distance;
    return d <= 0 || !d.isFinite ? 1 : d;
  }

  ViewerLayerMap _straight() => ViewerLayerMap.of(
        positionX: 0,
        positionY: 0,
        anchorX: 0,
        anchorY: 0,
        scaleXPercent: 100,
        scaleYPercent: 100,
        rotationDegrees: 0,
        origin: widget.fitted.topLeft,
        viewScale: widget.compSize.width == 0
            ? 1
            : widget.fitted.width / widget.compSize.width,
      );

  BridgeEffectValue? _value(BridgeEffectInstanceInfo node, String id) {
    if (_writer.editor.stagedValue(node.id, id) case final staged?) {
      return staged;
    }
    for (final v in node.values) {
      if (v.id == id) return v.value;
    }
    return null;
  }

  double _float(BridgeEffectInstanceInfo node, String id) {
    final v = _value(node, id);
    if (v is! BridgeEffectValue_Float) return 0;
    return scalarValueAt(v.field0, widget.comp, _ui.playheadFrame.value) ?? 0;
  }

  _Win? _win(BridgeEffectInstanceInfo node, int n) {
    final shape = switch (_value(node, 'win${n}_shape')) {
      BridgeEffectValue_Choice(:final field0) => field0,
      _ => 0,
    };
    if (shape == 0) return null;
    final points = switch (_value(node, 'win${n}_points')) {
      BridgeEffectValue_Shape(:final field0) => [
          for (final p in field0)
            if (p.length >= 2) Offset(p[0], p[1])
        ],
      _ => <Offset>[],
    };
    final smooth = switch (_value(node, 'win${n}_smooth')) {
      BridgeEffectValue_Bool(:final field0) => field0,
      _ => true,
    };
    double f(String r) => _float(node, 'win${n}_$r');
    return _Win(
        n,
        shape,
        Offset(f('x'), f('y')),
        math.max(f('w'), 0) / 2,
        math.max(f('h'), 0) / 2,
        f('angle'),
        f('round'),
        math.max(f('soft'), 0),
        math.max(f('soft_in'), 0),
        points,
        smooth);
  }

  // --- Writing ---

  BridgeEffectValue _floatFor(
      BridgeEffectInstanceInfo node, String id, double v) {
    final was = node.values.where((p) => p.id == id).firstOrNull?.value;
    return BridgeEffectValue.float(was is BridgeEffectValue_Float
        ? scalarWithValueAt(was.field0, v, widget.comp, _ui.playheadFrame.value)
        : BridgeScalar.static_(v));
  }

  BridgeEffectValue _shapeOf(List<Offset> points) => BridgeEffectValue.shape([
        for (final p in points)
          Float32List.fromList([p.dx.clamp(-1.0, 1.0), p.dy.clamp(-1.0, 1.0)])
      ]);

  void _live(GradeShot shot, BridgeEffectInstanceInfo node,
      Map<String, BridgeEffectValue> rows) {
    _moved = {..._moved, ...rows};
    _writer.live(_ui, widget.comp, shot, node.id, rows);
  }

  void _commit(GradeShot shot, UuidValue node, Map<String, BridgeEffectValue> rows) {
    if (rows.isEmpty) return;
    _writer.write(context.read<LumitState>(), _ui, shot, node, rows);
  }

  // --- Drags ---

  void _start(_Grip grip, _Win w, Offset screen, {int point = -1}) {
    _focus.requestFocus();
    setState(() {
      _grip = grip;
      _from = w;
      _down = _layer(screen);
      _point = point;
      _moved = {};
      if (grip == _Grip.point) _picked = point;
    });
  }

  void _move(GradeShot shot, BridgeEffectInstanceInfo node, Offset screen) {
    final w = _from, grip = _grip;
    if (w == null || grip == null) return;
    final p = _layer(screen);
    final q = w.local(p);
    final n = w.slot;
    final shift = HardwareKeyboard.instance.isShiftPressed;
    String r(String row) => 'win${n}_$row';
    BridgeEffectValue f(String row, double v) => _floatFor(node, r(row), v);
    switch (grip) {
      case _Grip.centre:
        final c = w.centre + (p - _down);
        _live(shot, node, {r('x'): f('x', c.dx), r('y'): f('y', c.dy)});
      case _Grip.corner:
        var hw = q.dx.abs(), hh = q.dy.abs();
        if (shift && w.hw > 0 && w.hh > 0) {
          final s = math.max(hw / w.hw, hh / w.hh);
          hw = w.hw * s;
          hh = w.hh * s;
        }
        _live(shot, node, {r('w'): f('w', hw * 2), r('h'): f('h', hh * 2)});
      case _Grip.edgeX:
        _live(shot, node, {r('w'): f('w', q.dx.abs() * 2)});
      case _Grip.edgeY:
        _live(shot, node, {r('h'): f('h', q.dy.abs() * 2)});
      case _Grip.turn:
        final a0 = (_down - w.centre).direction, a1 = (p - w.centre).direction;
        var angle = w.angle + (a1 - a0) * 180 / math.pi;
        if (shift) angle = (angle / 15).roundToDouble() * 15;
        _live(shot, node, {r('angle'): f('angle', angle)});
      case _Grip.aim:
        final v = p - w.centre;
        var angle = math.atan2(v.dx, -v.dy) * 180 / math.pi;
        if (shift) angle = (angle / 15).roundToDouble() * 15;
        _live(shot, node, {r('angle'): f('angle', angle)});
      case _Grip.soft:
        final out = w.shape == 4 ? q.dy : q.dx - w.hw;
        _live(shot, node, {r('soft'): f('soft', math.max(out, 0))});
      case _Grip.softIn:
        final inside = w.shape == 4 ? -q.dy : w.hw - q.dx;
        _live(shot, node, {r('soft_in'): f('soft_in', math.max(inside, 0))});
      case _Grip.round:
        final most = math.min(w.hw, w.hh);
        final radius = (w.hw - q.dx).clamp(0.0, most);
        _live(shot, node,
            {r('round'): f('round', most <= 0 ? 0 : radius / most * 100)});
      case _Grip.point:
        _movePoint(shot, node, w, q);
    }
  }

  /// A polygon point to [q], in the box's frame. Inside the box only the
  /// points change; past its edge the box is fitted round them again.
  void _movePoint(GradeShot shot, BridgeEffectInstanceInfo node, _Win w, Offset q) {
    final n = w.slot;
    final i = _point;
    if (i < 0 || i >= w.points.length || w.hw <= 0 || w.hh <= 0) return;
    final norm = Offset(q.dx / w.hw, q.dy / w.hh);
    if (norm.dx.abs() <= 1 && norm.dy.abs() <= 1) {
      final points = [...w.points]..[i] = norm;
      _live(shot, node, {'win${n}_points': _shapeOf(points)});
      return;
    }
    final box = [...w.box]..[i] = q;
    _live(shot, node, _fitted(node, w, box));
  }

  /// The box fitted round [box] (points in the window's frame, pixels), the
  /// points written into it again, the turn kept.
  Map<String, BridgeEffectValue> _fitted(
      BridgeEffectInstanceInfo node, _Win w, List<Offset> box) {
    final n = w.slot;
    var lo = box.first, hi = box.first;
    for (final p in box) {
      lo = Offset(math.min(lo.dx, p.dx), math.min(lo.dy, p.dy));
      hi = Offset(math.max(hi.dx, p.dx), math.max(hi.dy, p.dy));
    }
    final mid = (lo + hi) / 2;
    final hw = math.max((hi.dx - lo.dx) / 2, 0.5);
    final hh = math.max((hi.dy - lo.dy) / 2, 0.5);
    final c = w.out(mid);
    return {
      'win${n}_x': _floatFor(node, 'win${n}_x', c.dx),
      'win${n}_y': _floatFor(node, 'win${n}_y', c.dy),
      'win${n}_w': _floatFor(node, 'win${n}_w', hw * 2),
      'win${n}_h': _floatFor(node, 'win${n}_h', hh * 2),
      'win${n}_points': _shapeOf([
        for (final p in box) Offset((p.dx - mid.dx) / hw, (p.dy - mid.dy) / hh)
      ]),
    };
  }

  void _end(GradeShot shot, UuidValue node) {
    final moved = _moved;
    setState(() {
      _grip = null;
      _from = null;
      _moved = {};
    });
    if (moved.isEmpty) {
      _writer.clear();
      return;
    }
    _commit(shot, node, moved);
  }

  void _cancel() {
    if (_grip == null) return;
    _writer.clear();
    setState(() {
      _grip = null;
      _from = null;
      _moved = {};
    });
    _ui.requestFrame();
  }

  // --- Polygon drawing and points ---

  void _addPoint(Offset screen) {
    final drawn = _drawing;
    if (drawn == null) return;
    if (drawn.length >= 3 && (_screen(drawn.first) - screen).distance <= 8) {
      _close();
      return;
    }
    if (drawn.length >= 32) return;
    _focus.requestFocus();
    setState(() => _drawing = [...drawn, _layer(screen)]);
  }

  /// The drawn points become the slot's polygon, its box fitted round them
  /// square to the picture, as one step.
  void _close() {
    final drawn = _drawing;
    final shot = _g.shot.value;
    final node = shot == null ? null : gradeNodeOf(_ui, shot);
    setState(() => _drawing = null);
    if (drawn == null || drawn.length < 3 || shot == null || node == null) {
      return;
    }
    final n = _drawingSlot;
    final flat = _Win(n, 3, Offset.zero, 1, 1, 0, 0, 0, 0, const [], true);
    _commit(shot, node.id, {
      ..._fitted(node, flat, drawn),
      'win${n}_angle': _floatFor(node, 'win${n}_angle', 0),
    });
  }

  void _deletePoint(GradeShot shot, BridgeEffectInstanceInfo node, _Win w) {
    final i = _picked;
    if (i == null || i >= w.points.length) return;
    setState(() => _picked = null);
    _commit(shot, node.id,
        {'win${w.slot}_points': _shapeOf([...w.points]..removeAt(i))});
  }

  KeyEventResult _key(KeyEvent e, GradeShot shot,
      BridgeEffectInstanceInfo node, _Win? picked) {
    if (e is! KeyDownEvent) return KeyEventResult.ignored;
    final k = e.logicalKey;
    if (_drawing != null) {
      if (k == LogicalKeyboardKey.enter || k == LogicalKeyboardKey.numpadEnter) {
        _close();
        return KeyEventResult.handled;
      }
      if (k == LogicalKeyboardKey.escape) {
        setState(() => _drawing = null);
        return KeyEventResult.handled;
      }
    }
    if (_picked != null && picked != null && picked.shape == 3) {
      if (k == LogicalKeyboardKey.delete || k == LogicalKeyboardKey.backspace) {
        _deletePoint(shot, node, picked);
        return KeyEventResult.handled;
      }
      if (k == LogicalKeyboardKey.escape) {
        setState(() => _picked = null);
        return KeyEventResult.handled;
      }
    }
    if (k == LogicalKeyboardKey.escape && _grip != null) {
      _cancel();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  /// A window's own menu: its shape, or off.
  void _menu(GradeShot shot, BridgeEffectInstanceInfo node, _Win w, Offset at) {
    final names = switch (cachedListParameters(correctionName)
        .where((p) => p.id == 'win${w.slot}_shape')
        .firstOrNull
        ?.kind) {
      BridgeParamKind_Choice(:final options) => options,
      _ => const <String>[],
    };
    showMenuAt<void>(
      context: context,
      position: at,
      rows: (closeWith) => [
        for (var s = 1; s < names.length; s++)
          viewerRow(engineLabel(names[s]), () {
            _commit(shot, node.id,
                {'win${w.slot}_shape': BridgeEffectValue.choice(s)});
          }, () => closeWith(null),
              id: 'viewer-window-shape-$s', ticked: w.shape == s),
        if (names.isNotEmpty) const MenuDivider(),
        viewerRow(names.isEmpty ? '' : engineLabel(names.first), () {
          _commit(shot, node.id,
              {'win${w.slot}_shape': const BridgeEffectValue.choice(0)});
          if (_g.window.value == w.slot) _g.window.value = 0;
        }, () => closeWith(null), id: 'viewer-window-off'),
      ],
    );
  }

  // --- Building ---

  @override
  Widget build(BuildContext context) {
    return Positioned.fill(
      child: ListenableBuilder(
        listenable: Listenable.merge([
          _g.window,
          _g.page,
          _g.node,
          _g.scope,
          _ui.model,
          _ui.playheadFrame,
          EffectStackEditor.staging,
        ]),
        builder: (context, _) {
          final shot = _g.shot.value;
          final node = shot == null ? null : gradeNodeOf(_ui, shot);
          final slot = _g.window.value;
          if (shot == null ||
              node == null ||
              node.name != correctionName ||
              (slot == 0 && _g.page.value != GradePage.windows)) {
            return const SizedBox.shrink();
          }
          _map = widget.mapOf(shot.layer.internallayerId) ?? _straight();
          final wins = [
            for (var n = 1; n <= 4; n++)
              if (_win(node, n) case final w?) w
          ];
          final picked = wins.where((w) => w.slot == slot).firstOrNull;
          // A polygon with nothing to it yet is drawn, point by point.
          if (picked != null && picked.shape == 3 && picked.points.length < 3) {
            if (_drawing == null || _drawingSlot != slot) {
              _drawing = [];
              _drawingSlot = slot;
            }
          } else if (_drawing != null) {
            _drawing = null;
          }
          final t = ThemeScope.of(context).theme;
          return Focus(
            focusNode: _focus,
            onKeyEvent: (_, e) => _key(e, shot, node, picked),
            child: Stack(children: [
              Positioned.fill(
                child: IgnorePointer(
                  child: CustomPaint(
                    key: const ValueKey('viewer-grade-windows'),
                    painter: _WindowsPainter(
                      [
                        for (final w in wins)
                          (
                            _ring(w, 0),
                            _ring(w, w.soft),
                            _ring(w, -w.softIn),
                            w.slot == slot
                          )
                      ],
                      [for (final p in _drawing ?? const <Offset>[]) _screen(p)],
                      t.accent,
                      t.textMuted,
                      t.surface0,
                    ),
                  ),
                ),
              ),
              if (_drawing != null)
                Positioned.fill(
                  child: MouseRegion(
                    cursor: SystemMouseCursors.precise,
                    child: GestureDetector(
                      key: const ValueKey('viewer-window-draw'),
                      behavior: HitTestBehavior.opaque,
                      onTapUp: (d) => _addPoint(d.localPosition),
                    ),
                  ),
                ),
              for (final w in wins)
                if (w.slot == slot)
                  ..._handles(t, shot, node, w)
                else
                  _handle(
                      'viewer-window-${w.slot}-pick',
                      _screen(w.centre),
                      SystemMouseCursors.click,
                      t,
                      dot: true,
                      onTap: () => _g.window.value = w.slot),
            ]),
          );
        },
      ),
    );
  }

  /// A slot's outline grown by [grow] pixels, on screen. A gradient is its
  /// line, long enough to cross any picture.
  List<Offset> _ring(_Win w, double grow) {
    if (w.shape == 4) {
      final reach =
          4.0 * math.max(widget.compSize.width, widget.compSize.height);
      return [
        _screen(w.out(Offset(-reach, grow))),
        _screen(w.out(Offset(reach, grow))),
      ];
    }
    return [for (final q in w.outline(grow)) _screen(w.out(q))];
  }

  List<Widget> _handles(LumitTheme t, GradeShot shot,
      BridgeEffectInstanceInfo node, _Win w) {
    final s = w.slot;
    final px = _px(w.centre);
    Widget grip(String name, _Grip g, Offset local, MouseCursor cursor,
            {bool dot = false, int point = -1}) =>
        _handle('viewer-window-$s-$name', _screen(w.out(local)), cursor, t,
            dot: dot,
            lit: _grip == g && (point < 0 || _point == point) ||
                (g == _Grip.point && _picked == point),
            onStart: (at) => _start(g, w, at, point: point),
            onMove: (at) => _move(shot, node, at),
            onEnd: () => _end(shot, node.id),
            onMenu: (at) => _menu(shot, node, w, at),
            onTap: g == _Grip.point
                ? () {
                    _focus.requestFocus();
                    setState(() => _picked = point);
                  }
                : null);
    final reach = 28 * px;
    if (w.shape == 4) {
      return [
        grip('centre', _Grip.centre, Offset.zero, SystemMouseCursors.move),
        grip('aim', _Grip.aim, Offset(0, -math.max(reach * 2, w.softIn + reach)),
            SystemMouseCursors.grab, dot: true),
        grip('soft', _Grip.soft, Offset(reach * 2, w.soft),
            SystemMouseCursors.resizeUpDown, dot: true),
        grip('soft-in', _Grip.softIn, Offset(reach * 2, -w.softIn),
            SystemMouseCursors.resizeUpDown, dot: true),
      ];
    }
    return [
      grip('centre', _Grip.centre, Offset.zero, SystemMouseCursors.move),
      for (final (i, x, y) in [(0, -1, -1), (1, 1, -1), (2, 1, 1), (3, -1, 1)])
        grip('corner-$i', _Grip.corner, Offset(x * w.hw, y * w.hh),
            SystemMouseCursors.precise),
      for (final x in [-1, 1])
        grip('edge-x$x', _Grip.edgeX, Offset(x * w.hw, 0),
            SystemMouseCursors.resizeLeftRight),
      for (final y in [-1, 1])
        grip('edge-y$y', _Grip.edgeY, Offset(0, y * w.hh),
            SystemMouseCursors.resizeUpDown),
      grip('turn', _Grip.turn, Offset(0, -w.hh - reach), SystemMouseCursors.grab,
          dot: true),
      grip('soft', _Grip.soft, Offset(w.hw + w.soft, reach),
          SystemMouseCursors.resizeLeftRight, dot: true),
      grip('soft-in', _Grip.softIn, Offset(w.hw - w.softIn, -reach),
          SystemMouseCursors.resizeLeftRight, dot: true),
      if (w.shape == 2)
        grip('round', _Grip.round, Offset(w.hw - w.radius, -w.hh),
            SystemMouseCursors.resizeLeftRight, dot: true),
      if (w.shape == 3)
        for (var i = 0; i < w.points.length; i++)
          grip('point-$i', _Grip.point, w.box[i], SystemMouseCursors.move,
              dot: true, point: i),
    ];
  }

  Widget _handle(String key, Offset at, MouseCursor cursor, LumitTheme t,
      {bool dot = false,
      bool lit = false,
      void Function(Offset)? onStart,
      void Function(Offset)? onMove,
      VoidCallback? onEnd,
      void Function(Offset)? onMenu,
      VoidCallback? onTap}) {
    const size = 14.0;
    return Positioned(
      key: ValueKey<String>(key),
      left: at.dx - size / 2,
      top: at.dy - size / 2,
      width: size,
      height: size,
      child: MouseRegion(
        cursor: cursor,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: onTap,
          onPanStart:
              onStart == null ? null : (d) => onStart(d.globalPosition - _origin),
          onPanUpdate:
              onMove == null ? null : (d) => onMove(d.globalPosition - _origin),
          onPanEnd: onEnd == null ? null : (_) => onEnd(),
          onPanCancel: onEnd == null ? null : _cancel,
          onSecondaryTapUp:
              onMenu == null ? null : (d) => onMenu(d.globalPosition),
          child: CustomPaint(
              painter: _GripPainter(t.accent, t.surface0, dot: dot, lit: lit)),
        ),
      ),
    );
  }

  /// Where this overlay's corner is on screen, so a handle's pointer reads
  /// in the overlay's own frame however the handle has moved.
  Offset get _origin {
    final box = context.findRenderObject();
    return box is RenderBox ? box.localToGlobal(Offset.zero) : Offset.zero;
  }
}

class _WindowsPainter extends CustomPainter {
  /// Each slot's outline, outer and inner softness rings, and whether it is
  /// the picked one, on screen.
  final List<(List<Offset>, List<Offset>, List<Offset>, bool)> wins;

  /// A polygon being drawn.
  final List<Offset> drawing;
  final Color accent, muted, under;

  const _WindowsPainter(
      this.wins, this.drawing, this.accent, this.muted, this.under);

  @override
  void paint(Canvas canvas, Size size) {
    for (final (outline, outer, inner, picked) in wins) {
      final ink = picked ? accent : muted;
      final closed = outline.length > 2;
      Path path(List<Offset> p) => Path()..addPolygon(p, closed);
      final line = Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 3
        ..color = under.withValues(alpha: 0.5);
      canvas.drawPath(path(outline), line);
      canvas.drawPath(
          path(outline),
          line
            ..strokeWidth = picked ? 1.4 : 1
            ..color = ink);
      final ring = Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = ink.withValues(alpha: 0.55);
      for (final r in [outer, inner]) {
        _dashed(canvas, r, closed, ring);
      }
    }
    if (drawing.isNotEmpty) {
      final line = Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.4
        ..color = accent;
      canvas.drawPath(Path()..addPolygon(drawing, false), line);
      for (final p in drawing) {
        canvas.drawCircle(p, 3.5, Paint()..color = under);
        canvas.drawCircle(p, 3.5, line);
      }
    }
  }

  static void _dashed(Canvas canvas, List<Offset> p, bool closed, Paint paint) {
    final n = p.length;
    if (n < 2) return;
    var on = true;
    var left = 4.0;
    for (var i = 0; i < (closed ? n : n - 1); i++) {
      var a = p[i];
      final b = p[(i + 1) % n];
      var span = (b - a).distance;
      while (span > 0) {
        final step = math.min(left, span);
        final c = Offset.lerp(a, b, step / (b - a).distance)!;
        if (on) canvas.drawLine(a, c, paint);
        span -= step;
        left -= step;
        a = c;
        if (left <= 0) {
          on = !on;
          left = 4;
        }
      }
    }
  }

  @override
  bool shouldRepaint(_WindowsPainter old) => true;
}

/// A handle's grip: a square for a size or a place, a round one for the rest.
class _GripPainter extends CustomPainter {
  final Color line, under;
  final bool dot, lit;
  const _GripPainter(this.line, this.under, {this.dot = false, this.lit = false});

  @override
  void paint(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    final fill = Paint()..color = lit ? line : under;
    final edge = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.2
      ..color = line;
    if (dot) {
      canvas.drawCircle(c, 3.5, fill);
      canvas.drawCircle(c, 3.5, edge);
    } else {
      final r = Rect.fromCenter(center: c, width: 7, height: 7);
      canvas.drawRect(r, fill);
      canvas.drawRect(r, edge);
    }
  }

  @override
  bool shouldRepaint(_GripPainter old) =>
      old.lit != lit || old.dot != dot || old.line != line;
}
