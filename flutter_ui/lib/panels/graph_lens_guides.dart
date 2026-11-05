// What the graph marks over its curves (docs/07 §5.1).
//
// Value, speed and acceleration are one chain: going down a graph is taking
// the slope, going up is adding up the area. Two marks show it:
//
// * hollow rings on the turning points, in the speed and acceleration graphs:
//   where the speed is at its fastest or slowest and where the acceleration
//   pushes or brakes hardest (the value graph has none);
// * in the acceleration graph, each curve's number at the playhead, in the
//   pane's corner.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'graph_key_fields.dart' show keyFieldRow;
import 'graph_channels.dart';
import 'graph_maths.dart';
import 'timeline_extras_frb.dart';

/// Which reading [channel] draws in under [lens]: a mask's shape has no value
/// to plot, so it draws its rate of change in the value lens too.
GraphLens guideLensOf(GraphChannel channel, GraphLens lens) =>
    channel.isPath && lens == GraphLens.value ? GraphLens.speed : lens;

/// Whether [channel] has a curve the guides can read: keys to walk, not a
/// flat line, not an expression, and not the Vegas envelope (which reads in
/// per cent of playback speed, not in the property's own unit).
bool guidesRead(GraphChannel channel, List<BridgeKeyframe> keys,
        {required bool envelope}) =>
    !envelope &&
    !channel.isStatic &&
    channel.scalar is! BridgeScalar_Expression &&
    keys.length >= 2;

/// [channel]'s reading at [seconds] under [lens].
double guideValueAt(GraphChannel channel, List<BridgeKeyframe> keys,
        GraphLens lens, double seconds) =>
    switch (guideLensOf(channel, lens)) {
      GraphLens.value => channel.drawnValueAt(keys, seconds),
      GraphLens.speed => evaluateKeysSpeed(keys, seconds),
      GraphLens.acceleration => evaluateKeysAcceleration(keys, seconds),
    };

/// The turning points of [channel]'s reading between its keys, in seconds:
/// where the curve drawn under [lens] peaks or dips, and in the value graph
/// also where it is steepest (the speed's peaks and dips). The keys themselves
/// are left out — a key is drawn already.
List<double> guideTurningPoints(
        GraphChannel channel, List<BridgeKeyframe> keys, GraphLens lens) =>
    turningPointsOf(keys, guideLensOf(channel, lens));

/// [guideTurningPoints] for keys drawn as [shown].
List<double> turningPointsOf(List<BridgeKeyframe> keys, GraphLens shown,
    {int samples = 64}) {
  final readings = [
    shown,
    if (shown == GraphLens.value) GraphLens.speed,
  ];
  final out = <double>[];
  for (var k = 0; k + 1 < keys.length; k++) {
    final t0 = rationalSeconds(keys[k].time);
    final t1 = rationalSeconds(keys[k + 1].time);
    if (t1 - t0 <= 1e-9) continue;
    // A hold or a straight span has no turning point inside it.
    if (keys[k].interpOut is BridgeSideInterp_Hold) continue;
    for (final reading in readings) {
      double at(double t) => switch (reading) {
            GraphLens.value => evaluateKeys(keys, t),
            GraphLens.speed => evaluateKeysSpeed(keys, t),
            GraphLens.acceleration => evaluateKeysAcceleration(keys, t),
          };
      // Sampled just inside the span: at the keys the speed and acceleration
      // step, which is not a turning point.
      final step = (t1 - t0) / samples;
      final span = _spread(at, t0, t1, samples);
      if (span <= 1e-9) continue;
      // The way the curve last went, and where it last moved. Two equal
      // samples either side of a peak are a flat step, not a turn, so the
      // turn is where the way changes across them.
      var previous = at(t0 + step * 0.5);
      var way = 0;
      var movedAt = t0 + step * 0.5;
      for (var i = 1; i < samples; i++) {
        final t = t0 + step * (i + 0.5);
        final v = at(t);
        final d = v - previous;
        previous = v;
        if (d.abs() <= span * 1e-9) continue;
        final now = d > 0 ? 1 : -1;
        if (way != 0 && now != way) out.add((movedAt + t - step) / 2);
        way = now;
        movedAt = t;
      }
    }
  }
  return out;
}

double _spread(double Function(double) at, double t0, double t1, int n) {
  var lo = double.infinity, hi = double.negativeInfinity;
  for (var i = 0; i <= n; i++) {
    final v = at(t0 + (t1 - t0) * i / n);
    if (v < lo) lo = v;
    if (v > hi) hi = v;
  }
  return hi - lo;
}

/// The turning points, painted as hollow rings over one pane's curves.
class GraphLensGuidesPainter extends CustomPainter {
  final List<GraphChannel> channels;
  final List<List<BridgeKeyframe>> shownKeys;
  final List<bool> envelope;
  final GraphLens lens;
  final TimelineAxis axis;
  final double fps;
  final (double, double) range;
  final List<Color> palette;

  /// The stretch of the comp-wide canvas on screen, so only that is sampled.
  final double viewportLeft;
  final double viewportWidth;

  GraphLensGuidesPainter({
    required this.channels,
    required this.shownKeys,
    required this.envelope,
    required this.lens,
    required this.axis,
    required this.fps,
    required this.range,
    required this.palette,
    required this.viewportLeft,
    required this.viewportWidth,
  });

  double _yOf(double v, Size size) {
    final (lo, hi) = range;
    final span = (hi - lo).abs() < 1e-12 ? 1.0 : hi - lo;
    return size.height - (v - lo) / span * size.height;
  }

  @override
  void paint(Canvas canvas, Size size) {
    // The value graph shows no rings: its keys and handles say enough.
    if (fps <= 0 || lens == GraphLens.value) return;
    final left = viewportLeft, right = viewportLeft + viewportWidth;
    for (var c = 0; c < channels.length; c++) {
      final channel = channels[c];
      final keys = shownKeys[c];
      if (!guidesRead(channel, keys, envelope: envelope[c])) continue;
      final colour = palette[channel.colourIndex % palette.length];

      // The turning points, as hollow rings on the curve.
      final ring = Paint()
        ..color = colour
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.2;
      for (final t in guideTurningPoints(channel, keys, lens)) {
        final px = axis.xOf(t * fps);
        if (px < left - 4 || px > right + 4) continue;
        canvas.drawCircle(
            Offset(px, _yOf(guideValueAt(channel, keys, lens, t), size)),
            3.5,
            ring);
      }
    }
  }

  @override
  bool shouldRepaint(GraphLensGuidesPainter old) =>
      old.range != range ||
      old.lens != lens ||
      old.viewportLeft != viewportLeft ||
      old.viewportWidth != viewportWidth ||
      !identical(old.channels, channels) ||
      !identical(old.shownKeys, shownKeys);
}

/// The pane's corner: each curve's number at the playhead in its own colour
/// and unit.
class GraphLensReadout extends StatelessWidget {
  final List<GraphChannel> channels;
  final List<List<BridgeKeyframe>> shownKeys;
  final List<bool> envelope;
  final GraphLens lens;
  final double fps;
  final int frame;
  final List<Color> palette;
  final TextStyle style;
  final Color ground;

  /// At most this many curves are read out; the rest are on the graph.
  static const int most = 6;

  const GraphLensReadout({
    super.key,
    required this.channels,
    required this.shownKeys,
    required this.envelope,
    required this.lens,
    required this.fps,
    required this.frame,
    required this.palette,
    required this.style,
    required this.ground,
  });

  @override
  Widget build(BuildContext context) {
    final seconds = fps <= 0 ? 0.0 : frame / fps;
    final rows = <Widget>[];
    for (var c = 0; c < channels.length && rows.length < most; c++) {
      final channel = channels[c];
      final keys = shownKeys[c];
      if (!guidesRead(channel, keys, envelope: envelope[c])) continue;
      final unit = graphChannelUnit(channel) ?? '';
      final shown = guideLensOf(channel, lens);
      final v = guideValueAt(channel, keys, lens, seconds);
      final text = switch (shown) {
        GraphLens.value => '${graphNumberText(v)} $unit',
        GraphLens.speed => '${graphNumberText(v)} ${l10n.unitPerSecond(unit)}',
        GraphLens.acceleration =>
          '${graphNumberText(v)} ${l10n.unitPerSecondSquared(unit)}',
      };
      rows.add(Text('${channel.label}  ${text.trim()}',
          style: style.copyWith(
              color: palette[channel.colourIndex % palette.length])));
    }
    if (rows.isEmpty) return const SizedBox.shrink();
    return IgnorePointer(
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 3),
        decoration: BoxDecoration(
          color: ground,
          borderRadius: BorderRadius.circular(4),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            ...rows,
          ],
        ),
      ),
    );
  }
}

/// Which graph the pane draws — Value, Speed or Acceleration — as a pill in
/// the graph's top right corner, under the Layers / Graph switch. One click,
/// one graph.
class GraphLensSwitch extends StatelessWidget {
  final GraphLens lens;
  final ValueChanged<GraphLens> onLens;

  const GraphLensSwitch({super.key, required this.lens, required this.onLens});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Segment(
      key: const ValueKey<String>('graph-lens-switch'),
      children: [
        for (final (keyName, which, label, tip) in [
          (
            'graph-lens-value',
            GraphLens.value,
            l10n.clipboardValueColumn,
            l10n.tipValueGraph
          ),
          (
            'graph-lens-speed',
            GraphLens.speed,
            l10n.graphSpeed,
            l10n.tipSpeedGraph
          ),
          (
            'graph-lens-acceleration',
            GraphLens.acceleration,
            l10n.graphAcceleration,
            l10n.tipAccelerationGraph
          ),
        ])
          LumitTooltip(
            message: tip,
            child: SegmentOption(
              key: ValueKey<String>(keyName),
              active: lens == which,
              onTap: () => onLens(which),
              child: Text(t.kickerCase(label),
                  style: lens == which ? t.kickerOn : t.kicker),
            ),
          ),
      ],
    );
  }
}

/// Open the **numeric entry** box on one keyframe in the acceleration graph.
///
/// Two columns, so it stays short:
///
/// * **Frame** and **Value**, as in the other graphs;
/// * **In** and **Out**: the acceleration each side has at the key, in the
///   property's unit per second squared. A typed one reshapes that side
///   ([sideForAcceleration]). A side with no key beyond it has no field;
/// * **Constant acceleration**: from this key the speed changes evenly. It
///   leaves at **Start speed** (0 is at rest) and changes by **Acceleration**
///   every second, to **End speed** at the end **Value** and **Frame**. Of
///   Acceleration, End speed, Value and Frame, the two typed last decide the
///   other two ([constantAcceleration]), and the key after this one becomes
///   the end — added where there is none ([keysWithConstantAcceleration]).
///
/// Every field writes as it is set and the box stays up.
Future<void> showKeyAccelerationPopover({
  required BuildContext context,
  required Offset position,
  required double fps,
  required double frame,
  required double value,
  required double minFrame,
  required double maxFrame,
  required double? accelerationIn,
  required double? accelerationOut,
  required double startSpeed,
  required ({double frame, double value})? next,
  required String unit,
  required void Function(double frame, double value) onFrameValue,
  required void Function(bool isOut, double target) onAcceleration,
  required void Function(double startSpeed, double acceleration,
          double endFrame, double endValue)
      onConstant,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => _KeyAccelerationPopover(
        fps: fps,
        frame: frame,
        value: value,
        minFrame: minFrame,
        maxFrame: maxFrame,
        accelerationIn: accelerationIn,
        accelerationOut: accelerationOut,
        startSpeed: startSpeed,
        next: next,
        unit: unit,
        onFrameValue: onFrameValue,
        onAcceleration: onAcceleration,
        onConstant: onConstant,
      ),
    );

/// Which of the constant stretch's numbers was typed. The start speed is
/// always given; two of these four decide the other two.
enum _Constant { acceleration, endSpeed, value, frame }

class _KeyAccelerationPopover extends StatefulWidget {
  final double fps;
  final double frame;
  final double value;
  final double minFrame;
  final double maxFrame;
  final double? accelerationIn;
  final double? accelerationOut;
  final double startSpeed;
  final ({double frame, double value})? next;
  final String unit;
  final void Function(double frame, double value) onFrameValue;
  final void Function(bool isOut, double target) onAcceleration;
  final void Function(double startSpeed, double acceleration, double endFrame,
      double endValue) onConstant;

  const _KeyAccelerationPopover({
    required this.fps,
    required this.frame,
    required this.value,
    required this.minFrame,
    required this.maxFrame,
    required this.accelerationIn,
    required this.accelerationOut,
    required this.startSpeed,
    required this.next,
    required this.unit,
    required this.onFrameValue,
    required this.onAcceleration,
    required this.onConstant,
  });

  @override
  State<_KeyAccelerationPopover> createState() =>
      _KeyAccelerationPopoverState();
}

class _KeyAccelerationPopoverState extends State<_KeyAccelerationPopover> {
  /// What the box holds, as typed: it outlives the channel it opened on.
  late double _frame = widget.frame;
  late double _value = widget.value;
  late double? _in = widget.accelerationIn;
  late double? _out = widget.accelerationOut;

  /// The constant stretch. With a key after this one, it reads as the stretch
  /// that would reach that key from this key's own out speed; alone, it is at
  /// rest with nothing typed yet.
  late double _startSpeed = widget.next == null ? 0 : widget.startSpeed;
  late double _endValue = widget.next?.value ?? widget.value;
  late double _endFrame = widget.next?.frame ?? widget.frame;
  late double _acceleration = _fit?.acceleration ?? 0;
  late double _endSpeed = _fit?.endSpeed ?? _startSpeed;

  ({double acceleration, double endSpeed, double endValue, double seconds})?
      get _fit {
    final next = widget.next;
    if (next == null || widget.fps <= 0) return null;
    return constantAcceleration(widget.value,
        startSpeed: _startSpeed,
        endValue: next.value,
        seconds: (next.frame - widget.frame) / widget.fps);
  }

  /// The two numbers typed last, newest first; the other two are worked out.
  final List<_Constant> _typed = [_Constant.acceleration, _Constant.frame];

  void _setConstant(_Constant which, double v) {
    _typed
      ..remove(which)
      ..insert(0, which);
    if (_typed.length > 2) _typed.removeLast();
    switch (which) {
      case _Constant.acceleration:
        _acceleration = v;
      case _Constant.endSpeed:
        _endSpeed = v;
      case _Constant.value:
        _endValue = v;
      case _Constant.frame:
        _endFrame = v;
    }
    _solve();
  }

  /// Work the two numbers not typed out from the two that were and the start
  /// speed, and write the stretch, where they meet.
  void _solve() {
    if (widget.fps <= 0) return;
    final fit = constantAcceleration(
      _value,
      startSpeed: _startSpeed,
      acceleration:
          _typed.contains(_Constant.acceleration) ? _acceleration : null,
      endSpeed: _typed.contains(_Constant.endSpeed) ? _endSpeed : null,
      endValue: _typed.contains(_Constant.value) ? _endValue : null,
      seconds: _typed.contains(_Constant.frame)
          ? (_endFrame - _frame) / widget.fps
          : null,
    );
    if (fit == null) return;
    _acceleration = fit.acceleration;
    _endSpeed = fit.endSpeed;
    _endValue = fit.endValue;
    _endFrame = _frame + fit.seconds * widget.fps;
    widget.onConstant(_startSpeed, _acceleration, _endFrame, _endValue);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final perSecondSquared = l10n.unitPerSecondSquared(widget.unit);
    final perSecond = l10n.unitPerSecond(widget.unit);
    Widget pair(Widget left, Widget right) => Row(children: [
          Expanded(child: left),
          Expanded(child: right),
        ]);
    Widget field(String name, String key, double v,
            {required int decimals,
            num min = -1000000,
            num max = 1000000,
            String? suffix,
            double label = 40,
            required ValueChanged<double> set}) =>
        keyFieldRow(t, name, key, v,
            min: min,
            max: max,
            decimals: decimals,
            suffix: suffix,
            labelWidth: label,
            set: (v) => setState(() => set(v)));
    final frameField = field(
        l10n.graphKeyFrameField, 'graph-accel-frame', _frame,
        decimals: 0, min: widget.minFrame, max: widget.maxFrame, set: (v) {
      _frame = v;
      widget.onFrameValue(_frame, _value);
    });
    final valueField = field(
        l10n.graphKeyValueField, 'graph-accel-value', _value,
        decimals: 2, set: (v) {
      _value = v;
      widget.onFrameValue(_frame, _value);
    });
    final inField = switch (_in) {
      final accIn? => field(l10n.graphEaseIn, 'graph-accel-in', accIn,
            decimals: 2, suffix: perSecondSquared, set: (v) {
          _in = v;
          widget.onAcceleration(false, v);
        }),
      null => const SizedBox.shrink(),
    };
    final outField = switch (_out) {
      final accOut? => field(l10n.graphEaseOut, 'graph-accel-out', accOut,
            decimals: 2, suffix: perSecondSquared, set: (v) {
          _out = v;
          widget.onAcceleration(true, v);
        }),
      null => const SizedBox.shrink(),
    };
    return FloatSurface(
      width: 380,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          pair(frameField, valueField),
          if (_in != null || _out != null) pair(inField, outField),
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 6, 4, 0),
            child: Text(t.kickerCase(l10n.graphConstantAcceleration),
                style: t.kicker),
          ),
          pair(
            field(l10n.graphStartSpeed, 'graph-constant-start-speed',
                _startSpeed,
                decimals: 2, suffix: perSecond, label: 72, set: (v) {
              _startSpeed = v;
              _solve();
            }),
            field(l10n.graphEndSpeed, 'graph-constant-end-speed', _endSpeed,
                decimals: 2,
                suffix: perSecond,
                label: 72,
                set: (v) => _setConstant(_Constant.endSpeed, v)),
          ),
          field(l10n.graphAcceleration, 'graph-constant-acceleration',
              _acceleration,
              decimals: 2,
              suffix: perSecondSquared,
              label: 72,
              set: (v) => _setConstant(_Constant.acceleration, v)),
          pair(
            field(l10n.graphKeyValueField, 'graph-constant-value', _endValue,
                decimals: 2,
                label: 72,
                set: (v) => _setConstant(_Constant.value, v)),
            field(l10n.graphKeyFrameField, 'graph-constant-frame', _endFrame,
                decimals: 2,
                min: _frame,
                label: 72,
                set: (v) => _setConstant(_Constant.frame, v)),
          ),
        ],
      ),
    );
  }
}
