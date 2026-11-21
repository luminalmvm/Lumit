// A Range selector's Start, End and Offset as handles on the picture: a bar
// under the selected Text layer's words for each Range selector, Start and
// End as two flags along it and Offset as a diamond between them. Dragging a
// flag moves that end; dragging the diamond slides the window. The value is
// written once, when the drag is let go, so a drag is one undo step; a keyed
// number gets a key at the playhead, as a scrub on its row would.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart' show BridgeLayerKind;

import '../state/layer_bounds.dart' show measuredText;
import 'graph_maths.dart' show evaluateScalar, rationalSeconds;
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'text_animator_rows_frb.dart';
import 'viewer_gizmo.dart' show LayerBox;

/// One Range selector's bar: which animator and selector, and its numbers.
class _Bar {
  final int animator;
  final int selector;
  final BridgeRangeSelector s;
  const _Bar(this.animator, this.selector, this.s);
}

/// Which handle a drag holds.
enum _Grip { start, end, offset }

class ViewerRangeHandles extends StatefulWidget {
  final bool active;
  final CompositionReference comp;
  final LumitUiState uiState;
  final List<LayerBox> boxes;
  final Color accent;
  final VoidCallback onChanged;

  const ViewerRangeHandles({
    super.key,
    required this.active,
    required this.comp,
    required this.uiState,
    required this.boxes,
    required this.accent,
    required this.onChanged,
  });

  @override
  State<ViewerRangeHandles> createState() => _ViewerRangeHandlesState();
}

class _ViewerRangeHandlesState extends State<ViewerRangeHandles> {
  /// The bar and handle held, and the value it is dragged to so far.
  (_Bar, _Grip)? _held;
  double? _dragged;
  double _from = 0;

  /// The one selected Text layer's box, when exactly one is selected.
  LayerBox? get _box {
    final selected = widget.uiState.selectedLayers.value;
    if (selected.length != 1) return null;
    final id = selected.first.internallayerId;
    for (final b in widget.boxes) {
      if (b.id == id) return b;
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return const SizedBox.shrink();
    return ValueListenableBuilder<int>(
      valueListenable: widget.uiState.playheadFrame,
      builder: (context, frame, _) => _build(context, frame),
    );
  }

  Widget _build(BuildContext context, int frame) {
    final box = _box;
    if (box == null) return const SizedBox.shrink();
    final entry = widget.uiState.model.byId(box.id);
    if (entry == null || entry.info.kind != BridgeLayerKind.text) {
      return const SizedBox.shrink();
    }
    final animators = entry.info.textAnimators;
    final bars = [
      for (var i = 0; i < animators.length; i++)
        for (var k = 0; k < selectorCount(animators[i]); k++)
          if (selectorAt(animators[i], k)
              case final s
              when s.kind == BridgeSelectorKind.range &&
                  s.mode == BridgeSelectorMode.range)
            _Bar(i, k, s),
    ];
    if (bars.isEmpty) return const SizedBox.shrink();
    final document = box.layer.getText();
    if (document == null) return const SizedBox.shrink();
    final block = measuredText(document);
    if (block.lines.isEmpty) return const SizedBox.shrink();
    var left = double.infinity, right = double.negativeInfinity;
    for (final line in block.lines) {
      if (line.carets.isEmpty) continue;
      left = left < line.carets.first ? left : line.carets.first;
      right = right > line.carets.last ? right : line.carets.last;
    }
    if (!left.isFinite || right <= left) return const SizedBox.shrink();
    final below = block.lines.last.baseline + block.descent;
    final seconds = rationalSeconds(widget.comp.timeOfFrame(frame: frame));
    final text = document.text;

    // Per cent (or units) of the run ↔ layer x.
    double whole(BridgeRangeSelector s) => s.units == BridgeSelectorUnits.indexed
        ? unitCount(text, s.basis).toDouble().clamp(1, 1e9)
        : 100;
    double xOf(BridgeRangeSelector s, double v) =>
        left + (right - left) * v / whole(s);
    double vOf(BridgeRangeSelector s, double dx) =>
        dx / (right - left) * whole(s);
    double yOf(int row) => below + 10 + 14 * row / box.map.viewScale;

    double valueOf(_Bar bar, _Grip g) {
      final held = _held;
      if (held != null &&
          held.$1.animator == bar.animator &&
          held.$1.selector == bar.selector &&
          held.$2 == g &&
          _dragged != null) {
        return _dragged!;
      }
      return evaluateScalar(
          switch (g) {
            _Grip.start => bar.s.start,
            _Grip.end => bar.s.end,
            _Grip.offset => bar.s.offset,
          },
          seconds);
    }

    // Where each handle stands on screen.
    Offset at(int row, _Bar bar, _Grip g) {
      final s = bar.s;
      final offset = valueOf(bar, _Grip.offset);
      final x = switch (g) {
        _Grip.start => xOf(s, valueOf(bar, _Grip.start) + offset),
        _Grip.end => xOf(s, valueOf(bar, _Grip.end) + offset),
        _Grip.offset => xOf(
            s,
            (valueOf(bar, _Grip.start) + valueOf(bar, _Grip.end)) / 2 +
                offset),
      };
      return box.map.toScreen(x, yOf(row));
    }

    void commit(_Bar bar, _Grip g, double v) {
      final value = switch (g) {
        _Grip.start => TextAnimatorValue.rangeStart,
        _Grip.end => TextAnimatorValue.rangeEnd,
        _Grip.offset => TextAnimatorValue.rangeOffset,
      };
      final scalar = selectorScalarOf(bar.s, value);
      writeTextAnimatorScalar(
        layer: box.layer,
        index: bar.animator,
        value: value,
        to: scalar is BridgeScalar_Static
            ? BridgeScalar.static_(v)
            : scalarWithValueAt(scalar, v, widget.comp, frame),
        selector: bar.selector,
      );
      widget.onChanged();
    }

    final handles = <Widget>[];
    for (var row = 0; row < bars.length; row++) {
      final bar = bars[row];
      for (final g in _Grip.values) {
        final p = at(row, bar, g);
        handles.add(Positioned(
          left: p.dx - 6,
          top: p.dy - 6,
          width: 12,
          height: 12,
          child: MouseRegion(
            cursor: SystemMouseCursors.resizeLeftRight,
            child: GestureDetector(
              key: ValueKey<String>(
                  'viewer-range-${bar.animator}-${bar.selector}-${g.name}'),
              behavior: HitTestBehavior.opaque,
              dragStartBehavior: DragStartBehavior.down,
              onHorizontalDragStart: (_) => setState(() {
                _held = (bar, g);
                _from = valueOf(bar, g);
                _dragged = _from;
              }),
              onHorizontalDragUpdate: (d) {
                // Screen px to layer px along the bar, then to per cent.
                final a = box.map.layerOf(p);
                final b = box.map.layerOf(p + Offset(d.delta.dx, 0));
                setState(() => _dragged = (_dragged ?? _from) +
                    vOf(bar.s, b.dx - a.dx));
              },
              onHorizontalDragEnd: (_) {
                final v = _dragged;
                setState(() {
                  _held = null;
                  _dragged = null;
                });
                if (v != null && v != _from) commit(bar, g, v);
              },
              onHorizontalDragCancel: () => setState(() {
                _held = null;
                _dragged = null;
              }),
              child: CustomPaint(
                painter: _HandlePainter(grip: g, colour: widget.accent),
              ),
            ),
          ),
        ));
      }
    }
    return Stack(
      children: [
        Positioned.fill(
          child: IgnorePointer(
            child: CustomPaint(
              painter: _BarsPainter(
                lines: [
                  for (var row = 0; row < bars.length; row++)
                    (
                      box.map.toScreen(left, yOf(row)),
                      box.map.toScreen(right, yOf(row)),
                      at(row, bars[row], _Grip.start),
                      at(row, bars[row], _Grip.end),
                    ),
                ],
                colour: widget.accent,
              ),
            ),
          ),
        ),
        ...handles,
      ],
    );
  }
}

/// The bars: the words' width faint, the window between Start and End
/// strong.
class _BarsPainter extends CustomPainter {
  final List<(Offset, Offset, Offset, Offset)> lines;
  final Color colour;
  _BarsPainter({required this.lines, required this.colour});

  @override
  void paint(Canvas canvas, Size size) {
    final faint = Paint()
      ..color = colour.withValues(alpha: 0.3)
      ..strokeWidth = 1;
    final strong = Paint()
      ..color = colour
      ..strokeWidth = 2;
    for (final (a, b, s, e) in lines) {
      canvas.drawLine(a, b, faint);
      canvas.drawLine(s, e, strong);
    }
  }

  @override
  bool shouldRepaint(_BarsPainter old) => true;
}

/// Start ▶, End ◀, Offset ◆.
class _HandlePainter extends CustomPainter {
  final _Grip grip;
  final Color colour;
  _HandlePainter({required this.grip, required this.colour});

  @override
  void paint(Canvas canvas, Size size) {
    final w = size.width, h = size.height;
    final path = switch (grip) {
      _Grip.start => Path()
        ..moveTo(w * 0.5, 0)
        ..lineTo(w, h * 0.5)
        ..lineTo(w * 0.5, h)
        ..close(),
      _Grip.end => Path()
        ..moveTo(w * 0.5, 0)
        ..lineTo(0, h * 0.5)
        ..lineTo(w * 0.5, h)
        ..close(),
      _Grip.offset => Path()
        ..moveTo(w * 0.5, 0)
        ..lineTo(w, h * 0.5)
        ..lineTo(w * 0.5, h)
        ..lineTo(0, h * 0.5)
        ..close(),
    };
    canvas.drawPath(path, Paint()..color = colour);
  }

  @override
  bool shouldRepaint(_HandlePainter old) =>
      old.grip != grip || old.colour != colour;
}
