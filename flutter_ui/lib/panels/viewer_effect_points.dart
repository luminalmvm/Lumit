// The points of the selected effects, marked on the picture.
//
// A Centre or a Light is an x and a y in the panel, and nothing on the picture
// said where that was. With the effect selected, or either of the pair's rows
// in the Timeline, each point wears the anchor's mark.
//
// A point is in the layer's own pixels, since an effect runs on the layer's
// picture before its transform. So each mark goes through the layer's own map,
// the one the gizmo draws its box with, and sits where the effect has it on a
// layer that is scaled, moved, turned, parented or seen through a camera.
//
// With the Selection tool in hand a mark can be dragged: the point follows
// the pointer through the same preview a row's drag uses, and the release
// writes both halves as one undo step. Only the marks take the pointer, so the
// rest of the picture behaves as before. A point an expression works out is
// drawn in the number colour, and neither it nor a wired one moves.

import 'package:flutter/foundation.dart' show listEquals;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../state/comp_time.dart' show sampledExpression, timeOfFrame;
import '../state/dock.dart' show Panel;
import '../widgets/controls.dart' show MenuDivider, ThemeScope;
import 'effect_param_row_frb.dart'
    show
        EffectStackEditor,
        cachedListPairs,
        cachedListParameters,
        scalarValueAt;
import 'graph_panel.dart' show drivenParamsOf;
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'layer_fold_frb.dart' show effectPath;
import 'timeline_extras_frb.dart' show showMenuAt;
import 'viewer_anchor.dart' show constrainToAxis;
import 'viewer_gizmo.dart' show LayerBox, gizmoAnchorSlop;
import 'viewer_layer_map.dart' show ViewerLayerMap;
import 'viewer_menus.dart' show viewerRow;
import 'viewer_tool_cursor.dart' show paintAnchorMark;

/// A point's row changed in Effect controls or the Timeline picks its effect,
/// as After Effects does, so the point is on the picture to drag. A pick that
/// already holds it is left alone.
void pickForPoint(LumitUiState ui, LayerReference layer,
    BridgeEffectInstanceInfo fx, String param) {
  if (ui.selectedEffectsLayer?.internallayerId == layer.internallayerId &&
      ui.selectedEffects.value.contains(fx.id)) {
    return;
  }
  if (cachedListPairs(fx.name).any((p) => p.x == param || p.y == param)) {
    ui.setEffectSelection(layer, [fx.id]);
  }
}

/// One point on the picture, and what a drag on it writes.
class _Mark {
  final LayerReference layer;
  final UuidValue effect;
  final String x, y;

  /// The two halves as the document holds them, so a keyed half takes a key.
  final BridgeScalar? xWas, yWas;

  /// What a whole frame is worth in the pair's unit: the layer's own pixels,
  /// or a hundred.
  final Size span;

  /// The layer's own picture, in its own pixels, and where the layer puts
  /// those pixels on screen.
  final Size size;
  final ViewerLayerMap map;
  final Offset at;

  /// An expression works out either half, so the mark is not dragged.
  final bool driven;

  const _Mark(this.layer, this.effect, this.x, this.y, this.xWas, this.yWas,
      this.span, this.size, this.map, this.at, this.driven);

  String get key => '${layer.internallayerId}/$effect/$x';
}

class ViewerEffectPoints extends StatefulWidget {
  final CompositionReference comp;
  final LumitUiState uiState;

  /// Every layer's box as the gizmo has it, asked for only once a point is
  /// to be drawn.
  final List<LayerBox> Function() boxes;
  final Color colour;

  /// The armed tool. Only Selection drags a mark.
  final ToolMode tool;

  /// Told once a drag has been written.
  final VoidCallback onChanged;

  const ViewerEffectPoints({
    super.key,
    required this.comp,
    required this.uiState,
    required this.boxes,
    required this.colour,
    required this.tool,
    required this.onChanged,
  });

  @override
  State<ViewerEffectPoints> createState() => _ViewerEffectPointsState();
}

class _ViewerEffectPointsState extends State<ViewerEffectPoints> {
  final EffectStackEditor _editor = EffectStackEditor();

  String? _hover;

  /// The mark under the pointer is driven, by an expression or a wire.
  bool _hoverDriven = false;

  /// The drag in flight: the mark, the corner of its patch on screen and the
  /// press within it, the point it started from, and where it is now.
  _Mark? _held;
  Offset _patch = Offset.zero;
  Offset _down = Offset.zero;
  Offset _from = Offset.zero;
  Offset? _to;

  @override
  void dispose() {
    _editor.clear();
    super.dispose();
  }

  LumitUiState get _ui => widget.uiState;

  @override
  Widget build(BuildContext context) => Positioned.fill(
        child: ListenableBuilder(
          listenable: Listenable.merge([
            _ui.selectedEffects,
            _ui.selectedProperties,
            _ui.model,
            _ui.playheadFrame,
            EffectStackEditor.staging,
            // A layer moved in the Viewer or scrubbed in its rows carries its
            // points with it.
            _ui.liveTransforms,
          ]),
          builder: (context, _) {
            final marks = _marks();
            if (marks.isEmpty) return const SizedBox.shrink();
            final lit = [
              for (final m in marks)
                if (m.key == _held?.key || (m.key == _hover && !_hoverDriven))
                  m.at
            ];
            return Stack(
              children: [
                Positioned.fill(
                  child: IgnorePointer(
                    child: CustomPaint(
                      painter: _PointsPainter(
                          [for (final m in marks) m.at],
                          lit,
                          [for (final m in marks) m.driven],
                          widget.colour,
                          ThemeScope.of(context).theme.port.number),
                    ),
                  ),
                ),
                if (widget.tool.group == ToolGroup.select)
                  for (final m in marks) _grip(m),
              ],
            );
          },
        ),
      );

  /// The patch round a mark that takes the pointer, as big as the pivot's.
  Widget _grip(_Mark m) => Positioned(
        key: ValueKey<String>('viewer-fx-point-${m.key}'),
        left: m.at.dx - gizmoAnchorSlop / 2,
        top: m.at.dy - gizmoAnchorSlop / 2,
        width: gizmoAnchorSlop,
        height: gizmoAnchorSlop,
        child: MouseRegion(
          cursor: m.key != _hover
              ? MouseCursor.defer
              : _hoverDriven
                  ? SystemMouseCursors.forbidden
                  : SystemMouseCursors.move,
          onEnter: (_) => setState(() {
            _hover = m.key;
            _hoverDriven = m.driven || _wired(m);
          }),
          onExit: (_) => setState(() => _hover = null),
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onSecondaryTapUp: (d) => _menu(m, d.globalPosition),
            onPanDown: (d) => _start(m, d.localPosition),
            onPanUpdate: (d) => _move(d.localPosition),
            onPanEnd: (_) => _end(),
            onPanCancel: _cancel,
          ),
        ),
      );

  /// A point's own menu: where it can go, its key at the playhead, and its
  /// rows. A point worked out by an expression or a wire goes nowhere.
  void _menu(_Mark m, Offset at) {
    final fixed = m.driven || _wired(m);
    final frame = _ui.playheadFrame.value;
    void place(Offset to) {
      _editor.writeAll(m.layer, m.effect, {
        m.x: _value(m.xWas, to.dx, frame),
        m.y: _value(m.yWas, to.dy, frame),
      });
      widget.onChanged();
    }

    double? def(String id) => switch (cachedListParameters(_fxName(m))
            .where((p) => p.id == id)
            .firstOrNull
            ?.kind) {
          BridgeParamKind_Float(:final default_) => default_,
          _ => null,
        };
    final defX = def(m.x), defY = def(m.y);
    showMenuAt<void>(
      context: context,
      position: at,
      rows: (closeWith) {
        void close() => closeWith(null);
        return [
          viewerRow(
              l10n.viewerMenuCentrePoint,
              fixed
                  ? null
                  : () => place(Offset(m.span.width / 2, m.span.height / 2)),
              close,
              id: 'viewer-fx-point-centre'),
          viewerRow(
              l10n.viewerMenuResetPoint,
              fixed || defX == null || defY == null
                  ? null
                  : () => place(Offset(defX, defY)),
              close,
              id: 'viewer-fx-point-reset'),
          const MenuDivider(),
          viewerRow(l10n.viewerMenuPointReveal, () {
            _ui.setEffectSelection(m.layer, [m.effect]);
            _ui.frontPanel(Panel.effectControls);
          }, close, id: 'viewer-fx-point-reveal'),
        ];
      },
    );
  }

  String _fxName(_Mark m) {
    for (final entry in _ui.model.layers) {
      if (entry.layer.internallayerId != m.layer.internallayerId) continue;
      for (final fx in entry.info.effects) {
        if (fx.id == m.effect) return fx.name;
      }
    }
    return '';
  }

  /// Whether a wire drives either half. The graph is read here, on a hover or
  /// a press, never in a build.
  bool _wired(_Mark m) {
    final driven = drivenParamsOf(m.layer);
    return driven.containsKey('${m.effect}/${m.x}') ||
        driven.containsKey('${m.effect}/${m.y}');
  }

  void _start(_Mark m, Offset at) {
    _cancel();
    if (_wired(m)) return;
    final frame = _ui.playheadFrame.value;
    final x = scalarValueAt(m.xWas, widget.comp, frame);
    final y = scalarValueAt(m.yWas, widget.comp, frame);
    if (x == null || y == null) return;
    setState(() {
      _held = m;
      _patch = m.at - const Offset(gizmoAnchorSlop / 2, gizmoAnchorSlop / 2);
      _down = at;
      _from = Offset(x, y);
    });
  }

  /// The pointer's travel taken back through the layer's map into its own
  /// pixels, and the span says what a frame of those is worth.
  void _move(Offset at) {
    final m = _held;
    if (m == null) return;
    var d = at - _down;
    // Shift keeps it to one axis, as it does for a layer's move.
    if (HardwareKeyboard.instance.isShiftPressed) d = constrainToAxis(d);
    final from = m.map.layerOf(_patch + _down);
    final now = m.map.layerOf(_patch + _down + d);
    final to = _from +
        Offset((now.dx - from.dx) / m.size.width * m.span.width,
            (now.dy - from.dy) / m.size.height * m.span.height);
    _to = to;
    final frame = _ui.playheadFrame.value;
    for (final (id, was, v) in [(m.x, m.xWas, to.dx), (m.y, m.yWas, to.dy)]) {
      _editor.live(widget.comp, m.layer, m.effect, id, _value(was, v, frame),
          frame: frame, scale: _ui.viewerScale);
    }
  }

  /// A keyed half takes a key at the playhead and a still one stays still,
  /// the rule a typed value and a pick both follow.
  BridgeEffectValue _value(BridgeScalar? was, double v, int frame) =>
      BridgeEffectValue.float(was == null
          ? BridgeScalar.static_(v)
          : scalarWithValueAt(was, v, widget.comp, frame));

  void _end() {
    final m = _held, to = _to;
    if (m != null && to != null) {
      final frame = _ui.playheadFrame.value;
      // Both halves as one op, so the drag is one undo step.
      _editor.writeAll(m.layer, m.effect, {
        m.x: _value(m.xWas, to.dx, frame),
        m.y: _value(m.yWas, to.dy, frame),
      });
      widget.onChanged();
    }
    setState(() {
      _held = null;
      _to = null;
    });
  }

  void _cancel() {
    if (_held == null) return;
    _editor.clear();
    setState(() {
      _held = null;
      _to = null;
    });
  }

  List<_Mark> _marks() {
    final picked = _ui.selectedEffects.value;
    final rows = _ui.selectedProperties.value;
    if (picked.isEmpty && rows.isEmpty) return const [];
    final owner = _ui.selectedEffectsLayer?.internallayerId;
    final staged = EffectStackEditor.staging.value;
    final frame = _ui.playheadFrame.value;
    late final boxes = {for (final b in widget.boxes()) b.id: b};
    final out = <_Mark>[];
    for (final entry in _ui.model.layers) {
      final layerId = entry.layer.internallayerId;
      for (final fx in entry.info.effects) {
        final path = effectPath('$layerId', '${fx.id}');
        final whole = (layerId == owner && picked.contains(fx.id)) ||
            rows.contains(path);
        final params = cachedListParameters(fx.name);
        for (final pair in cachedListPairs(fx.name)) {
          if (!whole &&
              !rows.contains('$path/${pair.x}') &&
              !rows.contains('$path/${pair.y}')) {
            continue;
          }
          // A layer that is not on the picture has nowhere to put a mark.
          final box = boxes[layerId];
          if (box == null || box.bounds.isEmpty) continue;
          // px is the layer's own pixels, per cent is of its own frame, and
          // nothing else is a position.
          final unit = params.where((p) => p.id == pair.x).firstOrNull?.unit;
          final span = switch (unit) {
            BridgeUnit.px => box.bounds,
            BridgeUnit.percent => const Size(100, 100),
            _ => Size.zero,
          };
          if (span.isEmpty) continue;
          BridgeScalar? scalar(BridgeEffectValue? v) => switch (v) {
                BridgeEffectValue_Float(:final field0) => field0,
                _ => null,
              };
          BridgeScalar? held(String id) =>
              scalar(fx.values.where((v) => v.id == id).firstOrNull?.value);
          // What a drag has staged first, so the mark follows the picture.
          double? at(String id) {
            final s = scalar(staged[(fx.id, id)]) ?? held(id);
            return s is BridgeScalar_Expression
                ? sampledExpression(s, timeOfFrame(widget.comp, frame),
                    entry.layer, _ui.model.heldRevision,
                    row: BridgeExpressionRow.effect(effect: fx.id, param: id))
                : scalarValueAt(s, widget.comp, frame);
          }

          final x = at(pair.x), y = at(pair.y);
          if (x == null || y == null) continue;
          final lx = x / span.width * box.bounds.width;
          final ly = y / span.height * box.bounds.height;
          // Behind the camera, a point has no place on the picture.
          if (!box.map.inFront(lx, ly)) continue;
          out.add(_Mark(
            entry.layer,
            fx.id,
            pair.x,
            pair.y,
            held(pair.x),
            held(pair.y),
            span,
            box.bounds,
            box.map,
            box.map.toScreen(lx, ly),
            held(pair.x) is BridgeScalar_Expression ||
                held(pair.y) is BridgeScalar_Expression,
          ));
        }
      }
    }
    return out;
  }
}

class _PointsPainter extends CustomPainter {
  final List<Offset> points;

  /// The marks under the pointer or in hand, drawn a touch heavier.
  final List<Offset> lit;

  /// Which of [points] an expression works out, drawn in [drivenColour].
  final List<bool> driven;
  final Color colour, drivenColour;

  const _PointsPainter(
      this.points, this.lit, this.driven, this.colour, this.drivenColour);

  @override
  void paint(Canvas canvas, Size size) {
    for (var i = 0; i < points.length; i++) {
      paintAnchorMark(canvas, points[i], driven[i] ? drivenColour : colour);
    }
    for (final at in lit) {
      canvas.drawCircle(
          at, 4, Paint()..color = colour.withValues(alpha: 0.35));
      paintAnchorMark(canvas, at, colour, reach: 10);
    }
  }

  @override
  bool shouldRepaint(_PointsPainter old) =>
      old.colour != colour ||
      old.drivenColour != drivenColour ||
      !listEquals(old.points, points) ||
      !listEquals(old.driven, driven) ||
      !listEquals(old.lit, lit);
}
