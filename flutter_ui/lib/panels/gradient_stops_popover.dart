// The gradient editor: the ramp of a shape's gradient fill, with a handle for
// each colour along it.
//
// The two ends are the shape's own Fill and Gradient colour and stay where
// they are, as the two points in the Timeline are what aim them. A click on
// the ramp puts a stop between them, a drag slides it, and pulling it away from
// the ramp takes it off again.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../icons/lumit_icon.dart';
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';

/// The most stops between the two ends, `MAX_GRADIENT_STOPS` in `lumit-core`'s
/// `shape.rs`.
const int maxGradientStops = 16;

/// A gradient as the editor holds it: the two ends, and the stops between
/// them in order. Opacities are 0..100 and positions a per cent of the ramp.
class GradientRamp {
  final BridgeColourRgba startColour;
  final double startOpacity;
  final BridgeColourRgba endColour;
  final double endOpacity;
  final List<BridgeGradientStop> between;

  const GradientRamp({
    required this.startColour,
    required this.startOpacity,
    required this.endColour,
    required this.endOpacity,
    required this.between,
  });
}

/// A stop's colour as the screen shows it, at the opacity it carries.
Color _shown(BridgeColourRgba c, double opacity) => documentColour(
      (c.r.clamp(0.0, 1.0) * 255).round(),
      (c.g.clamp(0.0, 1.0) * 255).round(),
      (c.b.clamp(0.0, 1.0) * 255).round(),
      (c.a.clamp(0.0, 1.0) * opacity.clamp(0.0, 100.0) / 100 * 255).round(),
    );

/// The ramp over a chequered ground, so a faded stop reads as clear.
class GradientRampPainter extends CustomPainter {
  final GradientRamp ramp;
  final LumitTheme theme;
  final Rect? bar;

  const GradientRampPainter({required this.ramp, required this.theme, this.bar});

  @override
  void paint(Canvas canvas, Size size) {
    final box = bar ?? Offset.zero & size;
    canvas.save();
    canvas.clipRect(box);
    const cell = 5.0;
    canvas.drawRect(box, Paint()..color = theme.surface0);
    final check = Paint()..color = theme.surface4;
    for (var row = 0; row * cell < box.height; row++) {
      for (var col = row.isEven ? 0 : 1; col * cell < box.width; col += 2) {
        canvas.drawRect(
            Rect.fromLTWH(
                box.left + col * cell, box.top + row * cell, cell, cell),
            check);
      }
    }
    canvas.drawRect(
      box,
      Paint()
        ..shader = LinearGradient(
          colors: [
            _shown(ramp.startColour, ramp.startOpacity),
            for (final s in ramp.between) _shown(s.colour, s.opacity),
            _shown(ramp.endColour, ramp.endOpacity),
          ],
          stops: [
            0,
            for (final s in ramp.between) (s.position / 100).clamp(0.0, 1.0),
            1,
          ],
        ).createShader(box),
    );
    canvas.restore();
    canvas.drawRect(
      box,
      Paint()
        ..color = theme.hairlineStrong
        ..strokeWidth = 1
        ..style = PaintingStyle.stroke,
    );
  }

  @override
  bool shouldRepaint(GradientRampPainter old) =>
      old.ramp != ramp || old.theme != theme || old.bar != bar;
}

/// Open the gradient editor at [position].
///
/// [onPreview] is called while a handle or a number is being dragged, and
/// [onCommit] once it is let go, so a drag is one step to undo.
Future<void> showGradientStopsPopover({
  required BuildContext context,
  required Offset position,
  required GradientRamp ramp,
  required ValueChanged<GradientRamp> onPreview,
  required ValueChanged<GradientRamp> onCommit,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => _GradientStopsPopover(
        ramp: ramp,
        onPreview: onPreview,
        onCommit: onCommit,
      ),
    );

/// One colour on the ramp while it is being edited. A class rather than a
/// record, as a stop being dragged past another has to stay the same stop.
class _Stop {
  double position;
  BridgeColourRgba colour;
  double opacity;
  _Stop(this.position, this.colour, this.opacity);
}

const double _width = 292;
const double _inset = 14;
const double _barTop = 10;
const double _barHeight = 22;
const double _handleHeight = 14;
const double _stripHeight = _barTop + _barHeight + _handleHeight + 8;

/// How near the pointer must come to take hold of a handle, and how far from
/// the ramp a stop is pulled before it comes off.
const double _grabRadius = 8;
const double _pullOff = 34;

class _GradientStopsPopover extends StatefulWidget {
  final GradientRamp ramp;
  final ValueChanged<GradientRamp> onPreview;
  final ValueChanged<GradientRamp> onCommit;

  const _GradientStopsPopover({
    required this.ramp,
    required this.onPreview,
    required this.onCommit,
  });

  @override
  State<_GradientStopsPopover> createState() => _GradientStopsPopoverState();
}

class _GradientStopsPopoverState extends State<_GradientStopsPopover> {
  late final _Stop _start =
      _Stop(0, widget.ramp.startColour, widget.ramp.startOpacity);
  late final _Stop _end =
      _Stop(100, widget.ramp.endColour, widget.ramp.endOpacity);
  late final List<_Stop> _between = [
    for (final s in widget.ramp.between) _Stop(s.position, s.colour, s.opacity),
  ];
  late _Stop _selected = _start;

  /// The stop the pointer has hold of, whether it has been pulled far enough
  /// to come off, and whether the press has changed anything yet.
  _Stop? _dragging;
  bool _pulledOff = false;
  bool _dirty = false;

  Rect get _bar =>
      const Rect.fromLTWH(_inset, _barTop, _width - _inset * 2, _barHeight);

  bool _isEnd(_Stop s) => identical(s, _start) || identical(s, _end);

  double _xOf(_Stop s) => _bar.left + s.position / 100 * _bar.width;

  /// The ramp as it stands, leaving out a stop that is being pulled off.
  GradientRamp get _ramp {
    final between = [
      for (final s in _between)
        if (!(_pulledOff && identical(s, _dragging))) s,
    ]..sort((a, b) => a.position.compareTo(b.position));
    return GradientRamp(
      startColour: _start.colour,
      startOpacity: _start.opacity,
      endColour: _end.colour,
      endOpacity: _end.opacity,
      between: [
        for (final s in between)
          BridgeGradientStop(
              position: s.position, colour: s.colour, opacity: s.opacity),
      ],
    );
  }

  /// What the ramp reads at [position], for a stop put down there.
  _Stop _sampled(double position) {
    final stops = [_start, ..._between, _end]
      ..sort((a, b) => a.position.compareTo(b.position));
    var before = stops.first;
    var after = stops.last;
    for (final s in stops) {
      if (s.position <= position) before = s;
    }
    for (final s in stops.reversed) {
      if (s.position >= position) after = s;
    }
    final span = after.position - before.position;
    final u = span <= 0 ? 0.0 : (position - before.position) / span;
    double mix(double a, double b) => a + (b - a) * u;
    return _Stop(
      position,
      BridgeColourRgba(
        r: mix(before.colour.r, after.colour.r),
        g: mix(before.colour.g, after.colour.g),
        b: mix(before.colour.b, after.colour.b),
        a: mix(before.colour.a, after.colour.a),
      ),
      mix(before.opacity, after.opacity),
    );
  }

  void _grab(Offset local) {
    _Stop? best;
    var bestD = _grabRadius;
    for (final s in [_start, ..._between, _end]) {
      final d = (local.dx - _xOf(s)).abs();
      if (d > bestD) continue;
      bestD = d;
      best = s;
    }
    if (best != null) {
      setState(() {
        _selected = best!;
        _dragging = _isEnd(best) ? null : best;
        _pulledOff = false;
        _dirty = false;
      });
      return;
    }
    // Nothing under the pointer: a press on the ramp puts a stop there.
    if (local.dx < _bar.left ||
        local.dx > _bar.right ||
        _between.length >= maxGradientStops) {
      return;
    }
    final made =
        _sampled((local.dx - _bar.left) / _bar.width * 100);
    setState(() {
      _between.add(made);
      _selected = made;
      _dragging = made;
      _pulledOff = false;
      _dirty = true;
    });
    widget.onPreview(_ramp);
  }

  void _drag(Offset local) {
    final held = _dragging;
    if (held == null) return;
    final away = math.max(_bar.top - local.dy, local.dy - _stripHeight);
    setState(() {
      held.position =
          ((local.dx - _bar.left) / _bar.width * 100).clamp(0.0, 100.0);
      _pulledOff = away > _pullOff;
      _dirty = true;
    });
    widget.onPreview(_ramp);
  }

  void _release() {
    final held = _dragging;
    if (held == null) return;
    setState(() {
      if (_pulledOff) {
        _between.remove(held);
        _selected = _start;
      }
      _dragging = null;
      _pulledOff = false;
    });
    if (_dirty) widget.onCommit(_ramp);
    _dirty = false;
  }

  void _remove() {
    if (_isEnd(_selected)) return;
    setState(() {
      _between.remove(_selected);
      _selected = _start;
    });
    widget.onCommit(_ramp);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      width: _width,
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border.all(color: t.hairline),
        borderRadius: BorderRadius.circular(t.tokens.floatRadius),
        boxShadow: t.floatShadow,
      ),
      clipBehavior: Clip.antiAlias,
      child: Entrance.content(
        spec: ThemeScope.of(context).motion.content,
        rise: ThemeScope.of(context).motion.contentRise,
        blur: ThemeScope.of(context).motion.contentBlur,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              height: t.density.laneRow,
              decoration: BoxDecoration(
                color: t.surface2,
                border: Border(bottom: BorderSide(color: t.hairline)),
              ),
              padding: const EdgeInsets.symmetric(horizontal: 10),
              child: Align(
                alignment: Alignment.centerLeft,
                child: Text(t.kickerCase(l10n.shapeGradientStops),
                    style: t.kickerOn),
              ),
            ),
            // The empty drag handlers put the ramp in the gesture arena ahead
            // of an enclosing scroll, and the Listener hears the first
            // pointer-down rather than waiting out the drag slop.
            GestureDetector(
              behavior: HitTestBehavior.opaque,
              onVerticalDragUpdate: (_) {},
              onHorizontalDragUpdate: (_) {},
              child: Listener(
                key: const ValueKey<String>('grad-ramp'),
                onPointerDown: (e) => _grab(e.localPosition),
                onPointerMove: (e) => _drag(e.localPosition),
                onPointerUp: (_) => _release(),
                onPointerCancel: (_) => _release(),
                child: CustomPaint(
                  size: const Size(_width, _stripHeight),
                  painter: GradientRampPainter(
                      ramp: _ramp, theme: t, bar: _bar),
                  foregroundPainter: _HandlesPainter(
                    bar: _bar,
                    handles: [
                      for (final s in [_start, ..._between, _end])
                        (
                          x: _xOf(s),
                          colour: _shown(s.colour, 100),
                          selected: identical(s, _selected),
                          leaving: _pulledOff && identical(s, _dragging),
                        ),
                    ],
                    theme: t,
                  ),
                ),
              ),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(_inset, 2, _inset, 10),
              child: _controls(t),
            ),
          ],
        ),
      ),
    );
  }

  /// The selected stop's colour and numbers, each under its own label.
  Widget _controls(LumitTheme t) {
    final stop = _selected;
    final end = _isEnd(stop);
    // A column as wide as its control, so a longer word in another language
    // is cut short rather than pushing the row out of the popover.
    Widget under(String label, double width, Widget child) => SizedBox(
          width: width,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(label,
                  style: t.small,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis),
              const SizedBox(height: 4),
              SizedBox(
                height: 20,
                child: Align(alignment: Alignment.centerLeft, child: child),
              ),
            ],
          ),
        );
    return Row(
      crossAxisAlignment: CrossAxisAlignment.end,
      children: [
        under(
          l10n.shapeGradientStopColour,
          52,
          ColourSwatchButton(
            key: const ValueKey<String>('grad-stop-colour'),
            colour: _shown(stop.colour, 100),
            // A shape's colours are scene-linear, so the picker counts in 0-1.
            scale: ColourScale.unit,
            onPicked: (p) {
              setState(() => stop.colour =
                  BridgeColourRgba(r: p.r, g: p.g, b: p.b, a: stop.colour.a));
              widget.onCommit(_ramp);
            },
          ),
        ),
        const SizedBox(width: 12),
        under(
          l10n.shapeGradientStopOpacity,
          68,
          SizedBox(
            width: 68,
            child: DragValueField(
              key: const ValueKey<String>('grad-stop-opacity'),
              value: stop.opacity,
              min: 0,
              max: 100,
              decimals: 1,
              suffix: '%',
              onChanged: (v) {
                setState(() => stop.opacity = v.toDouble());
                widget.onCommit(_ramp);
              },
              onChangeLive: (v) {
                setState(() => stop.opacity = v.toDouble());
                widget.onPreview(_ramp);
              },
            ),
          ),
        ),
        const SizedBox(width: 12),
        under(
          l10n.shapeGradientStopPosition,
          68,
          SizedBox(
            width: 68,
            // The ends sit where the gradient's two points put them, so only
            // a stop between them has a place of its own to set.
            child: end
                ? Align(
                    alignment: Alignment.centerLeft,
                    child: Text('${stop.position.round()}%',
                        style: t.mono.copyWith(color: t.textDisabled)),
                  )
                : DragValueField(
                    key: const ValueKey<String>('grad-stop-position'),
                    value: stop.position,
                    min: 0,
                    max: 100,
                    decimals: 1,
                    suffix: '%',
                    onChanged: (v) {
                      setState(() => stop.position = v.toDouble());
                      widget.onCommit(_ramp);
                    },
                    onChangeLive: (v) {
                      setState(() => stop.position = v.toDouble());
                      widget.onPreview(_ramp);
                    },
                  ),
          ),
        ),
        const Spacer(),
        LumitTooltip(
          message: l10n.shapeGradientStopRemove,
          child: HouseButton(
            key: const ValueKey<String>('grad-stop-remove'),
            small: true,
            padding: const EdgeInsets.symmetric(horizontal: 6),
            onPressed: end ? null : _remove,
            child: LumitIcon(
              LumitIcons.delete,
              colour: end ? t.textDisabled : t.textPrimary,
              semanticLabel: l10n.shapeGradientStopRemove,
            ),
          ),
        ),
      ],
    );
  }
}

/// The handles under the ramp, one per stop.
class _HandlesPainter extends CustomPainter {
  final Rect bar;
  final List<({double x, Color colour, bool selected, bool leaving})> handles;
  final LumitTheme theme;

  const _HandlesPainter({
    required this.bar,
    required this.handles,
    required this.theme,
  });

  @override
  void paint(Canvas canvas, Size size) {
    // The selected one last, so it is never under a neighbour.
    final ordered = [
      for (final h in handles)
        if (!h.selected) h,
      for (final h in handles)
        if (h.selected) h,
    ];
    for (final h in ordered) {
      final top = bar.bottom + 1;
      final shape = Path()
        ..moveTo(h.x, top)
        ..lineTo(h.x + 5, top + 5)
        ..lineTo(h.x + 5, top + _handleHeight)
        ..lineTo(h.x - 5, top + _handleHeight)
        ..lineTo(h.x - 5, top + 5)
        ..close();
      final fade = h.leaving ? 0.3 : 1.0;
      canvas.drawPath(
          shape, Paint()..color = h.colour.withValues(alpha: fade));
      canvas.drawPath(
        shape,
        Paint()
          ..color = (h.selected ? theme.accent : theme.textMuted)
              .withValues(alpha: fade)
          ..strokeWidth = h.selected ? 1.5 : 1
          ..style = PaintingStyle.stroke,
      );
    }
  }

  @override
  bool shouldRepaint(_HandlesPainter old) => true;
}
