// The points of the selected effects, marked on the picture.
//
// A Centre or a Light is an x and a y in the panel, and nothing on the picture
// said where that was. With the effect selected, or either of the pair's rows
// in the Timeline, each point wears the anchor's mark at the spot a pick there
// would write.
//
// With the Selection tool in hand a mark can be dragged: the point follows
// the pointer through the same preview a row's drag uses, and the release
// writes both halves as one undo step. Only the marks take the pointer, so the
// rest of the picture behaves as before.

import 'package:flutter/foundation.dart' show listEquals;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

import 'effect_param_row_frb.dart'
    show
        EffectStackEditor,
        cachedListPairs,
        cachedListParameters,
        scalarValueAt;
import 'graph_panel.dart' show drivenParamsOf;
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'layer_fold_frb.dart' show effectPath;
import 'viewer_anchor.dart' show constrainToAxis;
import 'viewer_gizmo.dart' show gizmoAnchorSlop;
import 'viewer_tool_cursor.dart' show paintAnchorMark;

/// One point on the picture, and what a drag on it writes.
class _Mark {
  final LayerReference layer;
  final UuidValue effect;
  final String x, y;

  /// The two halves as the document holds them, so a keyed half takes a key.
  final BridgeScalar? xWas, yWas;

  /// What a whole frame is worth in the pair's unit: comp pixels, or a hundred.
  final Size span;
  final Offset at;

  const _Mark(this.layer, this.effect, this.x, this.y, this.xWas, this.yWas,
      this.span, this.at);

  String get key => '${layer.internallayerId}/$effect/$x';
}

class ViewerEffectPoints extends StatefulWidget {
  final CompositionReference comp;
  final LumitUiState uiState;

  /// Where the picture is drawn in this panel, at the current magnification.
  final Rect fitted;
  final Size compSize;
  final Color colour;

  /// The armed tool. Only Selection drags a mark.
  final ToolMode tool;

  /// Told once a drag has been written.
  final VoidCallback onChanged;

  const ViewerEffectPoints({
    super.key,
    required this.comp,
    required this.uiState,
    required this.fitted,
    required this.compSize,
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
  bool _hoverWired = false;

  /// The drag in flight: the mark, where the press was, the point it started
  /// from, and where it is now.
  _Mark? _held;
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
          ]),
          builder: (context, _) {
            final marks = _marks();
            if (marks.isEmpty) return const SizedBox.shrink();
            final lit = [
              for (final m in marks)
                if (m.key == _held?.key || (m.key == _hover && !_hoverWired))
                  m.at
            ];
            return Stack(
              children: [
                Positioned.fill(
                  child: IgnorePointer(
                    child: CustomPaint(
                      painter: _PointsPainter(
                          [for (final m in marks) m.at], lit, widget.colour),
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
          cursor: m.key == _hover && !_hoverWired
              ? SystemMouseCursors.move
              : MouseCursor.defer,
          onEnter: (_) => setState(() {
            _hover = m.key;
            _hoverWired = _wired(m);
          }),
          onExit: (_) => setState(() => _hover = null),
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onPanDown: (d) => _start(m, d.localPosition),
            onPanUpdate: (d) => _move(d.localPosition),
            onPanEnd: (_) => _end(),
            onPanCancel: _cancel,
          ),
        ),
      );

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
      _down = at;
      _from = Offset(x, y);
    });
  }

  /// The pick's own sum run backwards: screen travel over the picture's width
  /// is a fraction of the frame, and the span says what a frame is worth.
  void _move(Offset at) {
    final m = _held;
    final fitted = widget.fitted;
    if (m == null || fitted.isEmpty) return;
    var d = at - _down;
    // Shift keeps it to one axis, as it does for a layer's move.
    if (HardwareKeyboard.instance.isShiftPressed) d = constrainToAxis(d);
    final to = _from +
        Offset(d.dx / fitted.width * m.span.width,
            d.dy / fitted.height * m.span.height);
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
    final fitted = widget.fitted;
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
          // The pick's own rule, so the mark sits where a pick would put it:
          // px is comp pixels, per cent is of the frame, and nothing else is
          // a position.
          final unit = params.where((p) => p.id == pair.x).firstOrNull?.unit;
          final span = switch (unit) {
            BridgeUnit.px => widget.compSize,
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
          double? at(String id) => scalarValueAt(
              scalar(staged[(fx.id, id)]) ?? held(id), widget.comp, frame);
          final x = at(pair.x), y = at(pair.y);
          if (x == null || y == null) continue;
          out.add(_Mark(
            entry.layer,
            fx.id,
            pair.x,
            pair.y,
            held(pair.x),
            held(pair.y),
            span,
            Offset(
              fitted.left + x / span.width * fitted.width,
              fitted.top + y / span.height * fitted.height,
            ),
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
  final Color colour;

  const _PointsPainter(this.points, this.lit, this.colour);

  @override
  void paint(Canvas canvas, Size size) {
    for (final at in points) {
      paintAnchorMark(canvas, at, colour);
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
      !listEquals(old.points, points) ||
      !listEquals(old.lit, lit);
}
