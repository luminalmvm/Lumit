// The handles on a picked part of a 3D model: a thin box round what it moves,
// its pivot, and the 3D gizmo there. A drag writes the part's Position or
// Rotation rows through the staged path, one undo step on release.
//
// Nothing crosses the bridge while the pointer moves but the preview. Where the
// part is comes from the engine once a frame while one is picked, in its layer's
// own pixels, which is the space its rows are in.

import 'dart:math' as math;

import 'package:flutter/gestures.dart' show PointerDeviceKind, kPrimaryButton;
import 'package:flutter/services.dart' show HardwareKeyboard;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/model_rows.dart';
import 'package:uuid/uuid.dart';

import '../state/comp_time.dart';
import '../state/tools.dart';
import '../state/ui_state.dart';
import 'effect_param_row_frb.dart' show EffectStackEditor;
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'model_fold_rows.dart' show modelRowsById;
import 'viewer_gizmo.dart' show LayerBox;
import 'viewer_gizmo_3d.dart';
import '../widgets/controls.dart' show ThemeScope;
import '../widgets/drag_escape.dart';

const _moves = ['position_x', 'position_y', 'position_z'];
const _turns = ['rotation_x', 'rotation_y', 'rotation_z'];

/// How far a Shift drag steps: ten pixels, or fifteen degrees.
const double _moveStep = 10;
const double _turnStep = math.pi / 12;

/// The handles on the part `uiState.pickedPart` names, over [boxes]' layer.
class ViewerPartGizmo extends StatefulWidget {
  final CompositionReference comp;
  final LumitUiState uiState;
  final List<LayerBox> boxes;
  final int? revision;
  final VoidCallback onChanged;

  const ViewerPartGizmo({
    super.key,
    required this.comp,
    required this.uiState,
    required this.boxes,
    required this.revision,
    required this.onChanged,
  });

  @override
  State<ViewerPartGizmo> createState() => _ViewerPartGizmoState();
}

class _ViewerPartGizmoState extends State<ViewerPartGizmo> {
  final EffectStackEditor _editor = EffectStackEditor();
  final DragEscape _escape = DragEscape();
  BridgeModelPartPlace? _place;
  (String, int, int?)? _placeOf;

  GizmoPick? _hot;
  GizmoPick? _held;
  Gizmo3d? _heldGizmo;
  RingTurn? _ring;
  Offset _from = Offset.zero;
  Offset? _pointer;
  Vec3 _startMove = (0, 0, 0);
  Vec3 _startTurn = (0, 0, 0);
  Vec3 _moved = (0, 0, 0);
  Vec3 _turned = (0, 0, 0);
  double _swept = 0;

  @override
  void dispose() {
    _escape.dispose();
    _editor.clear();
    super.dispose();
  }

  LayerBox? get _box {
    final picked = widget.uiState.pickedPart.value;
    if (picked == null) return null;
    if (!widget.uiState.selectedLayers.value
        .any((l) => l.internallayerId == picked.layer)) {
      return null;
    }
    for (final box in widget.boxes) {
      if (box.id == picked.layer && box.cube != null && box.map.projected) {
        return box;
      }
    }
    return null;
  }

  BridgeModelPart? get _part {
    final picked = widget.uiState.pickedPart.value;
    for (final entry in widget.uiState.model.layers) {
      if (entry.layer.internallayerId != picked?.layer) continue;
      for (final part in entry.info.modelRows?.parts ?? const <BridgeModelPart>[]) {
        if (part.id.toString() == picked?.part) return part;
      }
    }
    return null;
  }

  BridgeEffectInstanceInfo? get _rows {
    final picked = widget.uiState.pickedPart.value;
    for (final entry in widget.uiState.model.layers) {
      if (entry.layer.internallayerId != picked?.layer) continue;
      final rows = entry.info.modelRows;
      return rows == null ? null : modelRowsById(rows, picked!.part);
    }
    return null;
  }

  BridgeScalar? _scalar(String id) {
    for (final v in _rows?.values ?? const <BridgeParamValue>[]) {
      if (v.id == id && v.value is BridgeEffectValue_Float) {
        return (v.value as BridgeEffectValue_Float).field0;
      }
    }
    return null;
  }

  double _value(String id) {
    final s = _scalar(id);
    if (s == null) return 0;
    if (s case BridgeScalar_Static(:final field0)) return field0;
    return sampledScalar(
        s, timeOfFrame(widget.comp, widget.uiState.playheadFrame.value));
  }

  Vec3 _three(List<String> ids) =>
      (_value(ids[0]), _value(ids[1]), _value(ids[2]));

  /// Where the part is at the frame on screen, asked once a frame and held.
  BridgeModelPartPlace? _placed(LayerBox box) {
    final part = widget.uiState.pickedPart.value!.part;
    final frame = widget.uiState.playheadFrame.value;
    final key = (part, frame, widget.revision);
    if (key == _placeOf) return _place;
    _placeOf = key;
    try {
      _place = box.layer.getModelPartPlace(
          part_: UuidValue.fromString(part), atFrame: frame);
    } catch (_) {
      // The layer went away between the pick and this frame.
      _place = null;
    }
    return _place;
  }

  Vec3? _pivot(BridgeModelPartPlace place) {
    final p = place.pivot;
    return p.length == 3 ? (p[0], p[1], p[2]) : null;
  }

  bool get _turning => widget.uiState.tools.tool.group == ToolGroup.rotate;

  bool get _tools =>
      _turning || widget.uiState.tools.tool.group == ToolGroup.select;

  /// Whether an expression writes the rows the handles would.
  bool get _driven => (_turning ? _turns : _moves)
      .any((id) => _scalar(id) is BridgeScalar_Expression);

  /// The handles at the frame on screen, null where a skin places the part and
  /// its rows move nothing, or the tool in hand has none to offer.
  Gizmo3d? _handles(LayerBox box, BridgeModelPartPlace place) {
    if (!_tools || (_part?.skinned ?? true)) return null;
    final at = _pivot(place);
    if (at == null) return null;
    return Gizmo3d.inLayer(box.map, at: at, turns: rotationAxes(_three(_turns)));
  }

  void _down(PointerDownEvent event, Gizmo3d gizmo) {
    if (event.buttons != kPrimaryButton || _driven) return;
    final pick = gizmo.pick(event.localPosition, turning: _turning);
    if (pick == null) return;
    _escape.begin(_cancel);
    setState(() {
      _held = pick;
      _heldGizmo = gizmo;
      _from = event.localPosition;
      _pointer = event.localPosition;
      _startMove = _three(_moves);
      _startTurn = _three(_turns);
      _moved = _startMove;
      _turned = _startTurn;
      _swept = 0;
      _ring = pick.kind == GizmoKind.ring
          ? RingTurn(gizmo, pick.axis, event.localPosition)
          : null;
    });
  }

  void _move(PointerMoveEvent event) {
    final gizmo = _heldGizmo, held = _held;
    if (gizmo == null || held == null || !_escape.running) return;
    final to = event.localPosition;
    final shift = HardwareKeyboard.instance.isShiftPressed;
    setState(() {
      _pointer = to;
      if (_ring case final ring?) {
        final swept = ring.sweptTo(to);
        _swept = shift ? (swept / _turnStep).roundToDouble() * _turnStep : swept;
        _turned = turnedAbout(_startTurn, gizmo.turns[held.axis], _swept,
            near: _turned);
      } else {
        final step = shift ? _moveStep : null;
        final d = switch (held.kind) {
          GizmoKind.arrow => gizmo.along(held.axis, _from, to, step: step),
          GizmoKind.screen => gizmo.onScreen(_from, to),
          _ => gizmo.across(held.axis, _from, to, step: step),
        };
        if (d != null) {
          _moved = (_startMove.$1 + d.$1, _startMove.$2 + d.$2, _startMove.$3 + d.$3);
        }
      }
    });
    _write(live: true);
  }

  void _up(PointerUpEvent event) {
    if (!_escape.end()) return;
    final changed = _moved != _startMove || _turned != _startTurn;
    if (changed) _write(live: false);
    setState(() {
      _held = null;
      _heldGizmo = null;
      _ring = null;
      _hot = null;
    });
    if (changed) widget.onChanged();
  }

  void _cancel() {
    if (_held == null) return;
    _editor.clear();
    // The picture on screen is the drag's, so put back what the layer holds.
    final layer = _box?.layer;
    if (layer != null) {
      try {
        widget.comp.renderFrameWithPreview(
          frame: widget.uiState.playheadFrame.value,
          scale: widget.uiState.viewerScale,
          layer: layer,
          effects: layer.getModelRows(),
        );
      } catch (_) {
        // The layer went away mid-drag, there is nothing to put back.
      }
    }
    setState(() {
      _held = null;
      _heldGizmo = null;
      _ring = null;
    });
  }

  /// The rows the drag in hand has changed, staged while it runs and written
  /// as one op when it ends.
  void _write({required bool live}) {
    final box = _box;
    final picked = widget.uiState.pickedPart.value;
    if (box == null || picked == null) return;
    final frame = widget.uiState.playheadFrame.value;
    final ids = _ring != null ? _turns : _moves;
    final now = _ring != null ? _turned : _moved;
    final was = _ring != null ? _startTurn : _startMove;
    final values = <String, BridgeEffectValue>{};
    for (final (i, id) in ids.indexed) {
      final (v, start) = switch (i) {
        0 => (now.$1, was.$1),
        1 => (now.$2, was.$2),
        _ => (now.$3, was.$3),
      };
      final scalar = _scalar(id);
      if (scalar == null || v == start) continue;
      values[id] = BridgeEffectValue.float(
          scalarWithValueAt(scalar, v, widget.comp, frame));
    }
    final part = UuidValue.fromString(picked.part);
    if (!live) {
      _editor.writeAll(box.layer, part, values);
      return;
    }
    values.forEach((id, value) => _editor.live(
        widget.comp, box.layer, part, id, value,
        frame: frame, scale: widget.uiState.viewerScale));
  }

  /// What the drag in hand has changed, by the pointer.
  List<String> get _readout {
    if (_held == null) return const [];
    if (_ring != null) {
      return [gizmoAmount(_swept * 180 / math.pi, unit: '°')];
    }
    final d = (_moved.$1 - _startMove.$1, _moved.$2 - _startMove.$2, _moved.$3 - _startMove.$3);
    return [
      if (d.$1 != 0) 'x ${gizmoAmount(d.$1)}',
      if (d.$2 != 0) 'y ${gizmoAmount(d.$2)}',
      if (d.$3 != 0) 'z ${gizmoAmount(d.$3)}',
    ];
  }

  /// The box as the drag in hand has it: it moves with a move, and turns about
  /// the pivot with a turn.
  List<Vec3> _corners(BridgeModelPartPlace place, Vec3 pivot) {
    final c = place.corners;
    if (c.length != 24) return const [];
    final gizmo = _heldGizmo;
    final d = (_moved.$1 - _startMove.$1, _moved.$2 - _startMove.$2, _moved.$3 - _startMove.$3);
    final turn = _ring != null && gizmo != null
        ? turnAbout(gizmo.turns[_held!.axis], _swept)
        : null;
    return [
      for (var i = 0; i < 24; i += 3)
        switch (turn) {
          final turn? => () {
              final (x, y, z) = turned3(
                  turn, (c[i] - pivot.$1, c[i + 1] - pivot.$2, c[i + 2] - pivot.$3));
              return (pivot.$1 + x, pivot.$2 + y, pivot.$3 + z);
            }(),
          null => _held == null
              ? (c[i], c[i + 1], c[i + 2])
              : (c[i] + d.$1, c[i + 1] + d.$2, c[i + 2] + d.$3),
        },
    ];
  }

  @override
  Widget build(BuildContext context) {
    final box = _box;
    final place = box == null ? null : _placed(box);
    final pivot = place == null ? null : _pivot(place);
    if (box == null || place == null || pivot == null) {
      return const SizedBox.shrink();
    }
    final gizmo = _heldGizmo ?? _handles(box, place);
    final t = ThemeScope.of(context).theme;
    return MouseRegion(
      opaque: false,
      cursor: _held != null
          ? SystemMouseCursors.grabbing
          : _hot != null
              ? SystemMouseCursors.grab
              : MouseCursor.defer,
      onHover: (event) {
        final hot = event.kind == PointerDeviceKind.mouse && !_driven
            ? gizmo?.pick(event.localPosition, turning: _turning)
            : null;
        if (hot != _hot) setState(() => _hot = hot);
      },
      onExit: (_) {
        if (_hot != null) setState(() => _hot = null);
      },
      child: Listener(
        behavior: HitTestBehavior.deferToChild,
        onPointerDown: gizmo == null ? null : (e) => _down(e, gizmo),
        onPointerMove: _move,
        onPointerUp: _up,
        onPointerCancel: (_) {
          _escape.end();
          _cancel();
        },
        // The Listener only watches, so this claims the drag from the Viewer's
        // pan, which would move the picture along with the part.
        child: GestureDetector(
          behavior: HitTestBehavior.deferToChild,
          onPanStart: (_) {},
          child: CustomPaint(
            size: Size.infinite,
            painter: _PartPainter(
              layer: box,
              gizmo: gizmo,
              corners: _corners(place, pivot),
              turning: _turning,
              driven: _driven,
              hot: _hot,
              held: _held,
              pointer: _pointer,
              readout: _readout,
              line: t.animated,
              neutral: t.textPrimary,
              outline: t.surface0,
              drivenTint: t.port.number,
              readoutStyle: t.small.copyWith(color: t.textPrimary),
              plate: t.surface0.withValues(alpha: 0.85),
            ),
          ),
        ),
      ),
    );
  }
}

class _PartPainter extends CustomPainter {
  final LayerBox layer;
  final Gizmo3d? gizmo;
  final List<Vec3> corners;
  final bool turning;
  final bool driven;
  final GizmoPick? hot;
  final GizmoPick? held;
  final Offset? pointer;
  final List<String> readout;
  final Color line;
  final Color neutral;
  final Color outline;
  final Color drivenTint;
  final TextStyle readoutStyle;
  final Color plate;

  _PartPainter({
    required this.layer,
    required this.gizmo,
    required this.corners,
    required this.turning,
    required this.driven,
    required this.hot,
    required this.held,
    required this.pointer,
    required this.readout,
    required this.line,
    required this.neutral,
    required this.outline,
    required this.drivenTint,
    required this.readoutStyle,
    required this.plate,
  });

  /// Only a handle takes the pointer, so anywhere else the layer's own gizmo
  /// gets it. Held, it keeps it.
  @override
  bool? hitTest(Offset position) =>
      held != null ||
      (!driven && gizmo?.pick(position, turning: turning) != null);

  @override
  void paint(Canvas canvas, Size size) {
    final g = gizmo;
    // The thin box: each edge joins two corners one bit of x, y or z apart.
    if (corners.length == 8) {
      final seen = [for (final c in corners) _seen(c)];
      // Neutral, so it reads apart from its layer's amber box round the lot.
      final paint = Paint()
        ..color = neutral.withValues(alpha: 0.7)
        ..strokeWidth = 1;
      for (var i = 0; i < 8; i++) {
        for (final bit in const [1, 2, 4]) {
          final j = i | bit;
          final (a, b) = (seen[i], seen[j]);
          if (j != i && a != null && b != null) canvas.drawLine(a, b, paint);
        }
      }
    }
    if (g == null) return;
    paintGizmo3d(
      canvas,
      g,
      turning: turning,
      neutral: neutral,
      outline: outline,
      hot: hot,
      held: held,
      tint: driven ? drivenTint : null,
    );
    // The pivot, which the rows turn it about.
    canvas.drawCircle(g.centre, 3.5, Paint()..color = outline);
    canvas.drawCircle(g.centre, 2.5, Paint()..color = line);
    if (pointer case final at?) {
      paintGizmoReadout(canvas, at, readout, style: readoutStyle, plate: plate);
    }
  }

  /// [p] in the layer's own pixels on screen, null behind the eye.
  Offset? _seen(Vec3 p) {
    final (x, y, z) = layer.map.inParent(p.$1, p.$2, p.$3);
    final rows = layer.map.parentRows;
    double row(int r) =>
        rows[r * 4] * x + rows[r * 4 + 1] * y + rows[r * 4 + 2] * z + rows[r * 4 + 3];
    final w = row(2);
    if (w <= 1e-3) return null;
    return layer.map.origin + Offset(row(0) / w, row(1) / w) * layer.map.viewScale;
  }

  @override
  bool shouldRepaint(_PartPainter old) => true;
}
