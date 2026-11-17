// The grade a control surface drives: the Grade panel's current node on the
// current shot, a layer or a Cut clip, or on its group's grade while the panel
// shows that. A drag stages through the same editor the Grade panel uses and
// is written as one step when the hardware goes still.
//
// The buttons reach what the Grade panel and the Shots panel do: step between
// shots and between Correction nodes, add serial, parallel, layer and outside
// nodes, bypass and reset, grab a still, and turn the panel's pages and wheel
// modes.

import 'dart:ui' show Offset;

import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/surface.dart';
import 'package:uuid/uuid.dart';

import '../state/app_state.dart';
import '../state/comp_time.dart';
import '../state/control_surfaces.dart';
import '../state/grade_engine.dart';
import '../state/grade_target.dart';
import '../state/ui_state.dart';
import 'effect_param_row_frb.dart' show EffectStackEditor, defaultEffectValue;
import 'grade_actions.dart';
import 'grade_wheel.dart' show wheelChroma, wheelMaster, wheelRgb;
import 'keyframe_controls_frb.dart' show scalarWithValueAt;

/// Rows a node's structure hangs on, which Reset node leaves alone.
const Set<String> _structure = {'space', 'combine'};

class CorrectionSurfaceTarget implements GradeSurfaceTarget {
  CorrectionSurfaceTarget(this._ui, this._app);

  final LumitUiState _ui;
  final LumitState _app;
  final EffectStackEditor _editor = EffectStackEditor();

  /// What the drag in hand has staged, by row, and the node it is on.
  final Map<String, BridgeEffectValue> _staged = {};
  (GradeShot, UuidValue)? _stagedOn;

  /// The Correction effect's rows, from the engine's declaration.
  late final Map<String, BridgeParamInfo> _rows = {
    for (final p in listParameters(effect: correctionName)) p.id: p,
  };

  /// The rows a surface reaches, which are all the panels show.
  late final Set<String> _reached = {
    for (final t in surfaceTargets())
      if (t case BridgeSurfaceTarget_Param(:final param)) param,
  };

  /// The current shot and the Corrections in view on it, in stack order.
  (GradeShot, List<BridgeEffectInstance>)? _nodes() {
    final shot = _ui.gradeShot.value;
    if (shot == null) return null;
    try {
      final scope = gradeScopeOf(_ui);
      final handles = scope == null
          ? shotHandles(shot)
          : shot.layer
              .getGradeGroupEffects(group: scope.group, part_: scope.part);
      return (
        shot,
        [
          for (final e in handles)
            if (e.name() == correctionName) e
        ]
      );
    } catch (_) {
      // The shot has gone.
      return null;
    }
  }

  /// The Grade panel's current node when it is a Correction, else the last.
  (GradeShot, BridgeEffectInstance)? _current() {
    final here = _nodes();
    if (here == null || here.$2.isEmpty) return null;
    final (shot, nodes) = here;
    final want = _ui.gradeNode.value;
    return (shot, nodes.where((n) => n.id() == want).firstOrNull ?? nodes.last);
  }

  /// Point the editor at the stack [shot]'s node lives in.
  void _aim(GradeShot shot) {
    final scope = gradeScopeOf(_ui);
    final clip = shot.clip;
    _editor.groupStack = scope == null
        ? null
        : () => shot.layer
            .getGradeGroupEffects(group: scope.group, part_: scope.part);
    _editor.clipStack = clip == null || scope != null
        ? null
        : () => shot.layer.getClipEffects(clip: clip, onCompClock: true);
  }

  /// [param] on [node], with the drag in hand written in.
  BridgeEffectValue? _value(
      GradeShot shot, BridgeEffectInstance node, String param) {
    final on = _stagedOn;
    if (on != null && on.$1.same(shot) && on.$2 == node.id()) {
      if (_staged[param] case final staged?) return staged;
    }
    try {
      return node.getValue(id: param);
    } catch (_) {
      return null;
    }
  }

  double _read(BridgeScalar s) => switch (_ui.selectedComp) {
        final comp? =>
          sampledScalar(s, timeOfFrame(comp, _ui.playheadFrame.value)),
        null => 0,
      };

  /// [s] holding [v] at the playhead: a key there for an animated row.
  BridgeScalar _put(BridgeScalar s, double v) => switch (_ui.selectedComp) {
        final comp? => scalarWithValueAt(s, v, comp, _ui.playheadFrame.value),
        null => BridgeScalar.static_(v),
      };

  double _clamp(String param, double v) => switch (_rows[param]?.kind) {
        BridgeParamKind_Float(:final hardMin, :final hardMax) => v.clamp(
            hardMin ?? double.negativeInfinity, hardMax ?? double.infinity),
        BridgeParamKind_Slider(:final min, :final max) => v.clamp(min, max),
        BridgeParamKind_Colour(:final min, :final max) => v.clamp(min, max),
        _ => v,
      };

  /// A wheel's colour [c] with its chroma moved on [axis] 0 or 1, or its
  /// master on 2, by [by].
  BridgeEffectValue _moved(String param, BridgeColour c, int? axis, double by) {
    final rgb = [_read(c.r), _read(c.g), _read(c.b)];
    var master = wheelMaster(rgb);
    var chroma = wheelChroma(rgb);
    switch (axis) {
      case 0:
        chroma += Offset(by, 0);
      case 1:
        chroma += Offset(0, by);
      default:
        master += by;
    }
    return _colour(param, c, wheelRgb(master, chroma));
  }

  BridgeEffectValue _colour(String param, BridgeColour c, List<double> rgb) =>
      BridgeEffectValue.colour(BridgeColour(
        r: _put(c.r, _clamp(param, rgb[0])),
        g: _put(c.g, _clamp(param, rgb[1])),
        b: _put(c.b, _clamp(param, rgb[2])),
        a: c.a,
      ));

  @override
  void nudge(String param, int? axis, double by) {
    final here = _current();
    final comp = _ui.selectedComp;
    if (here == null || comp == null) return;
    final (shot, node) = here;
    final id = node.id();
    final on = _stagedOn;
    if (on != null && (!on.$1.same(shot) || on.$2 != id)) {
      // Another node: what was staged on the last one is written first.
      commit();
    }
    final next = switch (_value(shot, node, param)) {
      BridgeEffectValue_Float(:final field0) => BridgeEffectValue.float(
          _put(field0, _clamp(param, _read(field0) + by))),
      BridgeEffectValue_Colour(:final field0) =>
        _moved(param, field0, axis, by),
      _ => null,
    };
    if (next == null) return;
    _staged[param] = next;
    _stagedOn = (shot, id);
    _aim(shot);
    _editor.live(comp, shot.layer, id, param, next,
        frame: _ui.playheadFrame.value, scale: _ui.viewerScale);
  }

  @override
  void commit() {
    final on = _stagedOn;
    _stagedOn = null;
    if (on == null || _staged.isEmpty) return;
    _write(on.$1, on.$2, Map.of(_staged));
    _staged.clear();
  }

  void _write(
      GradeShot shot, UuidValue node, Map<String, BridgeEffectValue> values) {
    _aim(shot);
    _editor.writeAll(shot.layer, node, values);
    _app.notifyDocumentChanged();
    _ui.model.refresh();
  }

  /// [param]'s resting value on [node], at the playhead for a keyed row,
  /// only [axis] of a wheel when given.
  BridgeEffectValue? _rest(GradeShot shot, BridgeEffectInstance node,
      String param, int? axis) {
    final row = _rows[param];
    final rest = row == null ? null : defaultEffectValue(row.kind);
    return switch ((_value(shot, node, param), rest)) {
      (
        BridgeEffectValue_Float(field0: final was),
        BridgeEffectValue_Float(field0: final d)
      ) =>
        BridgeEffectValue.float(_put(was, _read(d))),
      (
        BridgeEffectValue_Colour(field0: final was),
        BridgeEffectValue_Colour(field0: final d)
      ) =>
        _colour(param, was, () {
          final rgb = [_read(was.r), _read(was.g), _read(was.b)];
          final chroma = wheelChroma(rgb);
          final master = wheelMaster(rgb);
          final restMaster = wheelMaster([_read(d.r), _read(d.g), _read(d.b)]);
          return switch (axis) {
            0 => wheelRgb(master, Offset(0, chroma.dy)),
            1 => wheelRgb(master, Offset(chroma.dx, 0)),
            2 => wheelRgb(restMaster, chroma),
            _ => [_read(d.r), _read(d.g), _read(d.b)],
          };
        }()),
      (_, final d) => d,
    };
  }

  @override
  void reset(String param, [int? axis]) {
    final here = _current();
    if (here == null) return;
    final (shot, node) = here;
    if (_rest(shot, node, param, axis) case final value?) {
      _write(shot, node.id(), {param: value});
    }
  }

  @override
  void action(String id) {
    switch (id) {
      case 'node_previous':
        _step(-1);
      case 'node_next':
        _step(1);
      case 'node_reset':
        _resetNode();
      case 'node_bypass':
        if (_current() case (final shot, final node)) {
          try {
            shot.layer
                .setEffectEnabled(effect: node, enabled: !node.enabled());
          } catch (_) {
            return;
          }
          _app.notifyDocumentChanged();
          _ui.model.refresh();
        }
      case 'grade_bypass':
        bypassGrade(_app, _ui);
      case 'node_add_serial':
        addGradeNode(_app, _ui, GradeNodeKind.serial);
      case 'node_add_parallel':
        addGradeNode(_app, _ui, GradeNodeKind.parallel);
      case 'node_add_layer':
        addGradeNode(_app, _ui, GradeNodeKind.layer);
      case 'node_add_outside':
        addGradeNode(_app, _ui, GradeNodeKind.outside);
      case 'shot_previous':
        stepShot(_ui, -1);
      case 'shot_next':
        stepShot(_ui, 1);
      case 'still_grab':
        if (canGrabStill(_ui)) grabStill(_app, _ui);
      case final page when page.startsWith('palette_'):
        final name = page.substring('palette_'.length);
        if (GradePage.values.asNameMap()[name] case final p?) {
          _ui.grade.page.value = p;
        }
      case final mode when mode.startsWith('page_'):
        final name = mode.substring('page_'.length);
        if (GradeWheelMode.values.asNameMap()[name] case final m?) {
          _ui.grade.wheelMode.value = m;
          _ui.grade.page.value = GradePage.wheels;
        }
      case final reset when reset.startsWith('reset_'):
        this.reset(reset.substring('reset_'.length));
    }
  }

  /// The Correction [by] along from the current one becomes current.
  void _step(int by) {
    final here = _current();
    final nodes = _nodes()?.$2 ?? const <BridgeEffectInstance>[];
    if (here == null || nodes.isEmpty) return;
    final at = nodes.indexWhere((n) => n.id() == here.$2.id());
    final to = (at + by).clamp(0, nodes.length - 1);
    _ui.grade.setNode(nodes[to].id());
  }

  void _resetNode() {
    final here = _current();
    if (here == null) return;
    final (shot, node) = here;
    final values = <String, BridgeEffectValue>{
      for (final param in _rows.keys)
        if (!_structure.contains(param))
          if (_rest(shot, node, param, null) case final value?) param: value,
    };
    if (values.isNotEmpty) _write(shot, node.id(), values);
  }

  /// The pages a surface steps through, in its order.
  static const List<GradePage> _pages = [
    GradePage.wheels,
    GradePage.adjust,
    GradePage.curves,
    GradePage.qualifier,
    GradePage.windows,
    GradePage.match,
    GradePage.look,
  ];

  @override
  Map<String, double> values() {
    final out = <String, double>{
      'grade_page':
          _pages.indexOf(_ui.grade.page.value).clamp(0, _pages.length - 1) +
              0.0,
      'wheel_mode': _ui.grade.wheelMode.value.index + 0.0,
    };
    final here = _current();
    if (here == null) return out;
    final (shot, node) = here;
    final nodes = _nodes()?.$2 ?? const <BridgeEffectInstance>[];
    out['node'] = nodes.indexWhere((n) => n.id() == node.id()) + 1.0;
    for (final param in _reached) {
      switch (_value(shot, node, param)) {
        case BridgeEffectValue_Float(:final field0):
          out[param] = _read(field0);
        case BridgeEffectValue_Colour(field0: final c):
          final rgb = [_read(c.r), _read(c.g), _read(c.b)];
          final chroma = wheelChroma(rgb);
          out['$param.x'] = chroma.dx;
          out['$param.y'] = chroma.dy;
          out['$param.master'] = wheelMaster(rgb);
        default:
      }
    }
    return out;
  }

  @override
  String shot() {
    final shot = _ui.gradeShot.value;
    return shot == null ? '' : gradeShotName(_ui, shot);
  }
}
