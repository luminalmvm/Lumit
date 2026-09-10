// The custom fade editor (docs/impl/audio-timeline.md §3): the box that shapes
// a fade, or the two curves of a crossfade, by hand.
//
// A crossfade is two curves in one box - the outgoing clip's running from the
// top left down to the bottom right, the incoming one's from the bottom left up
// to the top right - because that is what the join looks like on the lane, and
// dragging one against the other is the whole point of the box. A lone fade
// shows the one curve, from the corner it starts in.
//
// **Keep level** ties the two together: a drag on one curve puts the power
// complement of it on the other, so the two gains keep their squares summing to
// one and the join stays as loud as the default is. The complement of Fast is
// Fast, so an untouched overlap is already there.
//
// The grab and drag arithmetic is the Easing panel's, cloned rather than shared:
// that box holds one rising curve and this one holds two facing curves, and the
// twenty lines it takes to say so twice are fewer than the ones it would take to
// make a single box mean both.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'clip_fades.dart';
import 'easing_curve.dart';

/// The box, its room, and how near the pointer must come to take hold of a
/// handle. A gain has nowhere to go past silence or full level, so the
/// handles stay inside the box and the box needs no reach around it.
const double _boxSide = 140;
const double _marginX = 20;
const double _marginY = 12;
const double _grabRadius = 18;
const double _popoverWidth = _boxSide + _marginX * 2;

/// A shape's name, by the id [clipFadeShapes] carries it under. Kept out of the
/// pure file so the maths tests without a locale.
String clipFadeShapeName(String id) => switch (id) {
      'linear' => l10n.clipFadeLinear,
      'fast' => l10n.clipFadeFast,
      'slow' => l10n.clipFadeSlow,
      'smooth' => l10n.clipFadeSmooth,
      _ => l10n.clipFadeSharp,
    };

/// Open the fade editor at [position].
///
/// [outgoing] is the shape falling across the join and [incoming] the one
/// rising; a lone fade passes null for the side it does not have. [onApply] is
/// called with both, and the panel writes each to the clip it belongs to.
Future<void> showClipFadePopover({
  required BuildContext context,
  required Offset position,
  required BridgeClipFadeShape? outgoing,
  required BridgeClipFadeShape? incoming,
  required void Function(
          BridgeClipFadeShape? outgoing, BridgeClipFadeShape? incoming)
      onApply,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => _ClipFadePopover(
        outgoing: outgoing,
        incoming: incoming,
        onApply: (out, into) {
          close(null);
          onApply(out, into);
        },
      ),
    );

class _ClipFadePopover extends StatefulWidget {
  final BridgeClipFadeShape? outgoing;
  final BridgeClipFadeShape? incoming;
  final void Function(
      BridgeClipFadeShape? outgoing, BridgeClipFadeShape? incoming) onApply;

  const _ClipFadePopover({
    required this.outgoing,
    required this.incoming,
    required this.onApply,
  });

  @override
  State<_ClipFadePopover> createState() => _ClipFadePopoverState();
}

class _ClipFadePopoverState extends State<_ClipFadePopover> {
  late BridgeClipFadeShape? _out = widget.outgoing;
  late BridgeClipFadeShape? _in = widget.incoming;

  /// On by default, and only offered on a crossfade: a lone fade has nothing to
  /// keep level against.
  bool _keep = true;

  /// Which knob the pointer has hold of: the side, and 1 or 2.
  ({bool outgoing, int handle})? _dragging;

  bool get _pair => _out != null && _in != null;

  Rect get _box => Rect.fromLTWH(_marginX, _marginY, _boxSide, _boxSide);

  /// A control point in widget coordinates.
  ///
  /// Both shapes are stored as the gain of a fade **in**, and a fade out reads
  /// the same curve backwards - so the outgoing curve is drawn with its time
  /// mirrored and its level the right way up, which puts it at the top left and
  /// brings it down to the bottom right.
  Offset _pointOf(bool outgoing, int handle) {
    final curve = fadeCurveOf((outgoing ? _out : _in)!);
    return _boxPoint(outgoing, handle == 1 ? curve.x1 : curve.x2,
        handle == 1 ? curve.y1 : curve.y2);
  }

  Offset _boxPoint(bool outgoing, double x, double y) => Offset(
        outgoing ? _box.right - x * _box.width : _box.left + x * _box.width,
        _box.bottom - y * _box.height,
      );

  /// A position in the box back into curve space, undoing the mirror.
  ({double x, double y}) _curveSpace(bool outgoing, Offset local) {
    final b = _box;
    return (
      x: outgoing
          ? (b.right - local.dx) / b.width
          : (local.dx - b.left) / b.width,
      y: (b.bottom - local.dy) / b.height,
    );
  }

  void _grab(Offset local) {
    ({bool outgoing, int handle})? best;
    var bestD = _grabRadius;
    for (final outgoing in [false, true]) {
      if ((outgoing ? _out : _in) == null) continue;
      for (final handle in [1, 2]) {
        final d = (local - _pointOf(outgoing, handle)).distance;
        if (d > bestD) continue;
        bestD = d;
        best = (outgoing: outgoing, handle: handle);
      }
    }
    if (best == null) return;
    setState(() => _dragging = best);
    _drag(local);
  }

  void _drag(Offset local) {
    final held = _dragging;
    if (held == null) return;
    final point = _curveSpace(held.outgoing, local);
    final shape = customFadeShape(fadeCurveOf((held.outgoing ? _out : _in)!)
        .withHandle(
            first: held.handle == 1,
            x: point.x.clamp(0.0, 1.0),
            y: point.y.clamp(0.0, 1.0)));
    setState(() {
      if (held.outgoing) {
        _out = shape;
      } else {
        _in = shape;
      }
      if (_keep && _pair) _keepLevel(dragged: shape, outgoing: held.outgoing);
    });
  }

  /// The side that was not dragged, set to the power complement of the one that
  /// was: a fit, because a complement is not itself a cubic.
  void _keepLevel(
      {required BridgeClipFadeShape dragged, required bool outgoing}) {
    final other =
        customFadeShape(fitFadeCurve((u) => keepLevelGain(dragged, u)));
    if (outgoing) {
      _in = other;
    } else {
      _out = other;
    }
  }

  void _pick(BridgeClipFadeShape shape) => setState(() {
        if (_out != null) _out = shape;
        if (_in != null) _in = shape;
      });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      width: _popoverWidth,
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border.all(color: t.hairline),
        borderRadius: BorderRadius.circular(t.tokens.floatRadius),
        boxShadow: t.floatShadow,
      ),
      clipBehavior: Clip.antiAlias,
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
              child: Text(l10n.clipFadeShapeTitle.toUpperCase(),
                  style: t.kickerOn),
            ),
          ),
          // The empty drag handlers put the box in the gesture arena, where the
          // inner member beats an enclosing scroll; the Listener does the work,
          // because it hears the first pointer-down rather than waiting out the
          // drag slop.
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onVerticalDragUpdate: (_) {},
            onHorizontalDragUpdate: (_) {},
            child: Listener(
              key: const ValueKey<String>('atl-fade-box'),
              onPointerDown: (e) => _grab(e.localPosition),
              onPointerMove: (e) => _drag(e.localPosition),
              onPointerUp: (_) => setState(() => _dragging = null),
              onPointerCancel: (_) => setState(() => _dragging = null),
              child: CustomPaint(
                size: const Size(_popoverWidth, _boxSide + _marginY * 2),
                painter: _FadeBoxPainter(
                  box: _box,
                  outgoing: _out == null ? null : fadeCurveOf(_out!),
                  incoming: _in == null ? null : fadeCurveOf(_in!),
                  theme: t,
                ),
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(10, 4, 10, 8),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (_pair) _keepRow(t),
                const SizedBox(height: 6),
                Wrap(
                  spacing: 4,
                  runSpacing: 4,
                  children: [
                    for (final preset in clipFadeShapes)
                      HouseButton(
                        key: ValueKey<String>('atl-fade-preset-${preset.id}'),
                        small: true,
                        padding: const EdgeInsets.symmetric(horizontal: 6),
                        onPressed: () => _pick(preset.shape),
                        child: Text(clipFadeShapeName(preset.id),
                            style: t.body.copyWith(color: t.textPrimary)),
                      ),
                  ],
                ),
                const SizedBox(height: 8),
                Row(
                  mainAxisAlignment: MainAxisAlignment.end,
                  children: [
                    HouseButton(
                      key: const ValueKey<String>('atl-fade-apply'),
                      small: true,
                      padding: const EdgeInsets.symmetric(horizontal: 10),
                      onPressed: () => widget.onApply(_out, _in),
                      child: Text(l10n.apply,
                          style: t.body.copyWith(color: t.textPrimary)),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _keepRow(LumitTheme t) => Row(
        children: [
          HouseCheckbox(
            key: const ValueKey<String>('atl-fade-keep'),
            value: _keep,
            onChanged: (on) => setState(() {
              _keep = on;
              if (on) _keepLevel(dragged: _in!, outgoing: false);
            }),
          ),
          const SizedBox(width: 6),
          Text(l10n.clipFadeKeepLevel, style: t.body),
        ],
      );
}

/// The box, the two curves and their handles.
class _FadeBoxPainter extends CustomPainter {
  final Rect box;

  /// The curve falling across the join, and the one rising: either may be
  /// absent, which is a lone fade.
  final EasingCurve? outgoing;
  final EasingCurve? incoming;
  final LumitTheme theme;

  const _FadeBoxPainter({
    required this.box,
    required this.outgoing,
    required this.incoming,
    required this.theme,
  });

  Offset _at(bool falling, double x, double y) => Offset(
        falling ? box.right - x * box.width : box.left + x * box.width,
        box.bottom - y * box.height,
      );

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(
      box,
      Paint()
        ..color = theme.hairline
        ..strokeWidth = 1
        ..style = PaintingStyle.stroke,
    );
    _curve(canvas, incoming, falling: false, colour: theme.curve.first);
    _curve(canvas, outgoing,
        falling: true,
        colour: theme.curve.length > 1 ? theme.curve[1] : theme.curve.first);
  }

  void _curve(Canvas canvas, EasingCurve? curve,
      {required bool falling, required Color colour}) {
    if (curve == null) return;
    final stem = Paint()
      ..color = theme.textDisabled
      ..strokeWidth = 1;
    final p1 = _at(falling, curve.x1, curve.y1);
    final p2 = _at(falling, curve.x2, curve.y2);
    canvas.drawLine(_at(falling, 0, 0), p1, stem);
    canvas.drawLine(_at(falling, 1, 1), p2, stem);

    final path = Path()..moveTo(_at(falling, 0, 0).dx, _at(falling, 0, 0).dy);
    const steps = 64;
    for (var i = 1; i <= steps; i++) {
      final u = i / steps;
      final point = _at(falling, curve.xAt(u), curve.yAt(u));
      path.lineTo(point.dx, point.dy);
    }
    canvas.drawPath(
      path,
      Paint()
        ..color = colour
        ..strokeWidth = 2
        ..style = PaintingStyle.stroke,
    );
    final knob = Paint()..color = theme.accent;
    canvas.drawCircle(p1, 4.5, knob);
    canvas.drawCircle(p2, 4.5, knob);
  }

  @override
  bool shouldRepaint(_FadeBoxPainter old) =>
      old.box != box ||
      old.outgoing != outgoing ||
      old.incoming != incoming ||
      old.theme != theme;
}
