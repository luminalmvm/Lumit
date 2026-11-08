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

/// Whether [channel] has a curve the guides can read: keys to walk, not a
/// flat line, not an expression, and not the Vegas envelope (which reads in
/// per cent of playback speed, not in the property's own unit).
bool guidesRead(GraphChannel channel, List<BridgeKeyframe> keys,
        {required bool envelope}) =>
    !envelope &&
    !channel.isStatic &&
    channel.scalar is! BridgeScalar_Expression &&
    keys.length >= 2;

/// The reading at [seconds] under [lens]: speed, or acceleration.
double guideValueAt(List<BridgeKeyframe> keys, GraphLens lens, double seconds) =>
    lens == GraphLens.acceleration
        ? evaluateKeysAcceleration(keys, seconds)
        : evaluateKeysSpeed(keys, seconds);

/// The turning points of the reading under [lens] between the keys, in seconds:
/// where the curve drawn peaks or dips. The keys themselves are left out, as a
/// key is drawn already. Only the spans that reach between [from] and [to] and
/// last at least [shortest] seconds are sampled, so what a repaint costs follows
/// the view and not the length of the curve.
List<double> turningPointsOf(List<BridgeKeyframe> keys, GraphLens lens,
    {int samples = 64,
    double from = double.negativeInfinity,
    double to = double.infinity,
    double shortest = 0}) {
  final out = <double>[];
  for (var k = 0; k + 1 < keys.length; k++) {
    final t0 = rationalSeconds(keys[k].time);
    final t1 = rationalSeconds(keys[k + 1].time);
    if (t1 - t0 <= 1e-9 || t1 - t0 < shortest) continue;
    if (t1 < from || t0 > to) continue;
    // A hold or a straight span has no turning point inside it.
    if (keys[k].interpOut is BridgeSideInterp_Hold) continue;
    double at(double t) => lens == GraphLens.acceleration
        ? accelerationInSpan(keys, k, t)
        : speedInSpan(keys, k, t);
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

const double _ringRadius = 3.5;

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
    if (fps <= 0 || axis.perFrame <= 0 || lens == GraphLens.value) return;
    final left = viewportLeft, right = viewportLeft + viewportWidth;
    final from = axis.frameAtExact(left) / fps;
    final to = axis.frameAtExact(right) / fps;
    // A span narrower than a ring has no room to show one.
    final shortest = 2 * _ringRadius / axis.perFrame / fps;
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
      for (final t in turningPointsOf(keys, lens,
          from: from, to: to, shortest: shortest)) {
        final px = axis.xOf(t * fps);
        if (px < left - 4 || px > right + 4) continue;
        canvas.drawCircle(
            Offset(px, _yOf(guideValueAt(keys, lens, t), size)),
            _ringRadius,
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

/// The acceleration graph's corner: each curve's acceleration at the playhead,
/// in its own colour.
class GraphLensReadout extends StatelessWidget {
  final List<GraphChannel> channels;
  final List<List<BridgeKeyframe>> shownKeys;
  final List<bool> envelope;
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
      final v = evaluateKeysAcceleration(keys, seconds);
      final text = '${graphNumberText(v)} ${l10n.unitPerSecondSquared(unit)}';
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

  /// A plot too narrow for the pill: the graph in force as one word, which
  /// opens the three.
  final bool compact;

  const GraphLensSwitch(
      {super.key,
      required this.lens,
      required this.onLens,
      this.compact = false});

  static final _lenses = [
    ('graph-lens-value', GraphLens.value),
    ('graph-lens-speed', GraphLens.speed),
    ('graph-lens-acceleration', GraphLens.acceleration),
  ];

  static String _word(GraphLens lens) => switch (lens) {
        GraphLens.value => l10n.clipboardValueColumn,
        GraphLens.speed => l10n.graphSpeed,
        GraphLens.acceleration => l10n.graphAcceleration,
      };

  static String _tip(GraphLens lens) => switch (lens) {
        GraphLens.value => l10n.tipValueGraph,
        GraphLens.speed => l10n.tipSpeedGraph,
        GraphLens.acceleration => l10n.tipAccelerationGraph,
      };

  /// The three graphs, the one in force ticked.
  void _menu(BuildContext context, Offset at) => showMenuAt<void>(
        context: context,
        position: at,
        rows: (close) => [
          for (final (keyName, which) in _lenses)
            MenuRow(
              key: ValueKey<String>('$keyName-row'),
              onPressed: () {
                close(null);
                onLens(which);
              },
              child: MenuLabel(_tip(which), ticked: lens == which),
            ),
        ],
      );

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      key: const ValueKey<String>('graph-lens-switch'),
      onSecondaryTapUp: (d) => _menu(context, d.globalPosition),
      child: compact
          ? Builder(
              builder: (context) => GestureDetector(
                key: const ValueKey<String>('graph-lens-compact'),
                behavior: HitTestBehavior.opaque,
                onTap: () {
                  final box = context.findRenderObject();
                  if (box is! RenderBox) return;
                  _menu(context,
                      box.localToGlobal(Offset(0, box.size.height + 2)));
                },
                child: Container(
                  height: Segment.height,
                  padding: const EdgeInsets.symmetric(horizontal: 10),
                  decoration: BoxDecoration(
                    color: t.surface0,
                    borderRadius:
                        BorderRadius.circular(t.tokens.actionRadius),
                  ),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(t.kickerCase(_word(lens)), style: t.kickerOn),
                      const SizedBox(width: 6),
                      Caret(colour: t.textMuted),
                    ],
                  ),
                ),
              ),
            )
          : Segment(
              children: [
                for (final (keyName, which) in _lenses)
                  LumitTooltip(
                    message: _tip(which),
                    child: SegmentOption(
                      key: ValueKey<String>(keyName),
                      active: lens == which,
                      onTap: () => onLens(which),
                      child: Text(t.kickerCase(_word(which)),
                          style: lens == which ? t.kickerOn : t.kicker),
                    ),
                  ),
              ],
            ),
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
/// * then one of two motions from this key, chosen on a pill:
///   * **Speed profile**: from rest, speed up by **Acceleration** to the
///     **Cruise speed**, hold it, slow by **Deceleration** to rest at the end
///     **Value** or **Frame** (the one typed last decides; [speedProfile],
///     [keysWithSpeedProfile]);
///   * **Gravity**: leave at **Start speed** and change speed by
///     **Acceleration** every second, the acceleration constant and the speed
///     growing, to the end **Value** or **Frame** ([constantAcceleration],
///     [keysWithConstantAcceleration]).
///
///   Either writes the keys it needs: the key after this one becomes the end,
///   or one is added.
///
/// Every field writes as it is set and the box stays up. Each write hands back
/// what the key now reads, or null when nothing was written. The box then says
/// so and keeps the numbers the document holds.
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
  required KeyBoxReading? Function(double frame, double value) onFrameValue,
  required KeyBoxReading? Function(bool isOut, double target) onAcceleration,
  required KeyBoxReading? Function(
          double peakSpeed, double accelEnd, double cruiseEnd, double end)
      onProfile,
  required KeyBoxReading? Function(double startSpeed, double acceleration,
          double endFrame, double endValue)
      onGravity,
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
        onProfile: onProfile,
        onGravity: onGravity,
      ),
    );

/// What the key a box is open on reads after a write: its two accelerations
/// (null on a side with no key beyond it) and how late its frame can go.
typedef KeyBoxReading = ({
  double? accelerationIn,
  double? accelerationOut,
  double maxFrame
});

/// The two motions the box writes from a key.
enum _Motion { profile, gravity }

/// Which end a motion was last given: where it stops, or when.
enum _End { value, frame }

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
  final KeyBoxReading? Function(double frame, double value) onFrameValue;
  final KeyBoxReading? Function(bool isOut, double target) onAcceleration;
  final KeyBoxReading? Function(
      double peakSpeed, double accelEnd, double cruiseEnd, double end) onProfile;
  final KeyBoxReading? Function(double startSpeed, double acceleration,
      double endFrame, double endValue) onGravity;

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
    required this.onProfile,
    required this.onGravity,
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
  late double _maxFrame = widget.maxFrame;

  /// Whether the last thing typed could not be written.
  bool _refused = false;

  /// Take in what a write hands back, and whether anything was written.
  bool _wrote(KeyBoxReading? now) {
    _refused = now == null;
    if (now == null) return false;
    _in = now.accelerationIn;
    _out = now.accelerationOut;
    _maxFrame = now.maxFrame;
    return true;
  }

  _Motion _motion = _Motion.profile;

  /// The speed profile: its three rates, and its end — the key after this one
  /// where there is one.
  double _rise = 0;
  double _cruise = 0;
  double _fall = 0;
  late double _profileValue = widget.next?.value ?? widget.value;
  late double _profileFrame = widget.next?.frame ?? widget.frame;
  _End _profileEnd = _End.value;

  /// Gravity: the start speed (this key's own out speed where it has a key
  /// after it, else at rest), the acceleration, and the end.
  late double _startSpeed = widget.next == null ? 0 : widget.startSpeed;
  double _gravity = 0;
  late double _gravityValue = widget.next?.value ?? widget.value;
  late double _gravityFrame = widget.next?.frame ?? widget.frame;
  _End _gravityEnd = _End.value;

  /// Lay the speed profile out from what it holds, and write it where it
  /// works out. Typed by the end value, it goes the way that value lies; by
  /// the end frame, the way the cruise speed's sign says.
  void _solveProfile() {
    final fps = widget.fps;
    if (fps <= 0) return;
    final double sign;
    final ({
      double peakSpeed,
      double accelSeconds,
      double cruiseSeconds,
      double decelSeconds,
      double distance
    })? p;
    if (_profileEnd == _End.value) {
      final d = _profileValue - _value;
      sign = d < 0 ? -1 : 1;
      p = speedProfile(
          acceleration: _rise.abs(),
          cruiseSpeed: _cruise.abs(),
          deceleration: _fall.abs(),
          distance: d.abs());
    } else {
      sign = _cruise < 0 ? -1 : 1;
      p = speedProfile(
          acceleration: _rise.abs(),
          cruiseSpeed: _cruise.abs(),
          deceleration: _fall.abs(),
          seconds: (_profileFrame - _frame) / fps);
    }
    // Not all typed yet, which is not a refusal.
    if (p == null) {
      _refused = false;
      return;
    }
    final accelEnd = _frame + p.accelSeconds * fps;
    final cruiseEnd = accelEnd + p.cruiseSeconds * fps;
    final end = cruiseEnd + p.decelSeconds * fps;
    if (_wrote(widget.onProfile(sign * p.peakSpeed, accelEnd, cruiseEnd, end))) {
      _profileFrame = end;
      _profileValue = _value + sign * p.distance;
    }
  }

  /// Work gravity's end out from its start speed, acceleration and the end
  /// typed last, and write it where it works out.
  void _solveGravity() {
    final fps = widget.fps;
    if (fps <= 0) return;
    final fit = constantAcceleration(
      _value,
      startSpeed: _startSpeed,
      acceleration: _gravity,
      endValue: _gravityEnd == _End.value ? _gravityValue : null,
      seconds:
          _gravityEnd == _End.frame ? (_gravityFrame - _frame) / fps : null,
    );
    if (fit == null) {
      // No motion or no end typed yet is not a refusal. An end it never
      // reaches is.
      final waiting = (_gravity == 0 && _startSpeed == 0) ||
          (_gravityEnd == _End.value
              ? _gravityValue == _value
              : _gravityFrame <= _frame);
      _refused = !waiting;
      return;
    }
    final endFrame = _frame + fit.seconds * fps;
    if (_wrote(
        widget.onGravity(_startSpeed, _gravity, endFrame, fit.endValue))) {
      _gravityValue = fit.endValue;
      _gravityFrame = endFrame;
    }
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
        decimals: 0, min: widget.minFrame, max: _maxFrame, set: (v) {
      if (_wrote(widget.onFrameValue(v, _value))) _frame = v;
    });
    final valueField = field(
        l10n.graphKeyValueField, 'graph-accel-value', _value,
        decimals: 2, set: (v) {
      if (_wrote(widget.onFrameValue(_frame, v))) _value = v;
    });
    final inField = switch (_in) {
      final accIn? => field(l10n.graphEaseIn, 'graph-accel-in', accIn,
            decimals: 2,
            suffix: perSecondSquared,
            set: (v) => _wrote(widget.onAcceleration(false, v))),
      null => const SizedBox.shrink(),
    };
    final outField = switch (_out) {
      final accOut? => field(l10n.graphEaseOut, 'graph-accel-out', accOut,
            decimals: 2,
            suffix: perSecondSquared,
            set: (v) => _wrote(widget.onAcceleration(true, v))),
      null => const SizedBox.shrink(),
    };
    const wide = 84.0;
    final profile = [
      pair(
        field(l10n.graphAcceleration, 'graph-profile-acceleration', _rise,
            decimals: 2,
            min: 0,
            suffix: perSecondSquared,
            label: wide, set: (v) {
          _rise = v;
          _solveProfile();
        }),
        field(l10n.graphDeceleration, 'graph-profile-deceleration', _fall,
            decimals: 2,
            min: 0,
            suffix: perSecondSquared,
            label: wide, set: (v) {
          _fall = v;
          _solveProfile();
        }),
      ),
      pair(
        field(l10n.graphCruiseSpeed, 'graph-profile-cruise', _cruise,
            decimals: 2, suffix: perSecond, label: wide, set: (v) {
          _cruise = v;
          _solveProfile();
        }),
        const SizedBox.shrink(),
      ),
      pair(
        field(l10n.graphKeyValueField, 'graph-profile-value', _profileValue,
            decimals: 2, label: wide, set: (v) {
          _profileValue = v;
          _profileEnd = _End.value;
          _solveProfile();
        }),
        field(l10n.graphKeyFrameField, 'graph-profile-frame', _profileFrame,
            decimals: 2, min: _frame, label: wide, set: (v) {
          _profileFrame = v;
          _profileEnd = _End.frame;
          _solveProfile();
        }),
      ),
    ];
    final gravity = [
      pair(
        field(l10n.graphStartSpeed, 'graph-gravity-speed', _startSpeed,
            decimals: 2, suffix: perSecond, label: wide, set: (v) {
          _startSpeed = v;
          _solveGravity();
        }),
        field(l10n.graphAcceleration, 'graph-gravity-acceleration', _gravity,
            decimals: 2,
            suffix: perSecondSquared,
            label: wide, set: (v) {
          _gravity = v;
          _solveGravity();
        }),
      ),
      pair(
        field(l10n.graphKeyValueField, 'graph-gravity-value', _gravityValue,
            decimals: 2, label: wide, set: (v) {
          _gravityValue = v;
          _gravityEnd = _End.value;
          _solveGravity();
        }),
        field(l10n.graphKeyFrameField, 'graph-gravity-frame', _gravityFrame,
            decimals: 2, min: _frame, label: wide, set: (v) {
          _gravityFrame = v;
          _gravityEnd = _End.frame;
          _solveGravity();
        }),
      ),
    ];
    return FloatSurface(
      width: 400,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          pair(frameField, valueField),
          if (_in != null || _out != null) pair(inField, outField),
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 6, 4, 2),
            child: Align(
              alignment: Alignment.centerLeft,
              child: Segment(
                key: const ValueKey<String>('graph-motion-switch'),
                children: [
                  for (final (keyName, which, label) in [
                    (
                      'graph-motion-profile',
                      _Motion.profile,
                      l10n.graphSpeedProfile
                    ),
                    ('graph-motion-gravity', _Motion.gravity, l10n.graphGravity),
                  ])
                    SegmentOption(
                      key: ValueKey<String>(keyName),
                      active: _motion == which,
                      onTap: () => setState(() {
                        _motion = which;
                        _refused = false;
                      }),
                      child: Text(t.kickerCase(label),
                          style: _motion == which ? t.kickerOn : t.kicker),
                    ),
                ],
              ),
            ),
          ),
          ...switch (_motion) {
            _Motion.profile => profile,
            _Motion.gravity => gravity,
          },
          if (_refused)
            Padding(
              padding: const EdgeInsets.fromLTRB(4, 4, 4, 2),
              child: Text(
                l10n.graphBoxRefused,
                key: const ValueKey<String>('graph-box-refused'),
                style: t.mono.copyWith(fontSize: 10, color: t.warning),
              ),
            ),
        ],
      ),
    );
  }
}
