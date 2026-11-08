// The two small boxes a retime asks through: how far and how fast a ramp goes,
// and how long a freeze holds.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../state/settings.dart' show InterfaceSettings;
import '../widgets/controls.dart';
import 'graph_key_fields.dart';
import 'graph_maths.dart';
import 'timeline_extras_frb.dart' show Segment, SegmentOption;

/// The speeds a ramp most often goes to, one click each.
const List<int> rampChips = [20, 50, 100, 200, 400];

const double _rampWidth = 236;
const double _rampLabel = 42;

/// The name of a curve the speed can take.
String speedCurveName(SpeedCurve curve) => switch (curve) {
      SpeedCurve.linear => l10n.speedCurveLinear,
      SpeedCurve.fast => l10n.speedCurveFast,
      SpeedCurve.slow => l10n.speedCurveSlow,
      SpeedCurve.smooth => l10n.speedCurveSmooth,
      SpeedCurve.sharp => l10n.speedCurveSharp,
      SpeedCurve.hold => l10n.speedCurveHold,
    };

/// Ask how a ramp goes: to which speed, over how long, along which curve,
/// starting from what the last ramp asked for in [settings]. `Enter` or the
/// button runs it, writes the answers back and calls [saved], and [onRamp]
/// hears the speed in per cent, the length in frames and the curve.
Future<void> showRampPopover({
  required BuildContext context,
  required Offset position,
  required double fps,
  required InterfaceSettings settings,
  required VoidCallback saved,
  required void Function(double percent, int frames, SpeedCurve curve) onRamp,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => _RampPopover(
        fps: fps,
        settings: settings,
        onRamp: (percent, frames, curve) {
          close(null);
          saved();
          onRamp(percent, frames, curve);
        },
      ),
    );

class _RampPopover extends StatefulWidget {
  final double fps;
  final InterfaceSettings settings;
  final void Function(double percent, int frames, SpeedCurve curve) onRamp;

  const _RampPopover(
      {required this.fps, required this.settings, required this.onRamp});

  @override
  State<_RampPopover> createState() => _RampPopoverState();
}

class _RampPopoverState extends State<_RampPopover> {
  final FocusScopeNode _scope = FocusScopeNode(debugLabel: 'ramp');
  late double _rampTo = widget.settings.retimeRampTo;
  late int _rampFrames = widget.settings.retimeRampFrames.clamp(1, 100000);
  late bool _rampInSeconds = widget.settings.retimeRampInSeconds;
  late SpeedCurve _rampCurve = SpeedCurve.values.firstWhere(
      (c) => c.name == widget.settings.retimeRampEase,
      orElse: () => SpeedCurve.linear);

  /// Counts chip presses. The To well opens being typed in, and a typed well
  /// keeps the text it opened with, so a chip opens it afresh on the new speed.
  int _chipped = 0;

  @override
  void dispose() {
    _scope.dispose();
    super.dispose();
  }

  void _go() {
    widget.settings
      ..retimeRampTo = _rampTo
      ..retimeRampFrames = _rampFrames
      ..retimeRampInSeconds = _rampInSeconds
      ..retimeRampEase = _rampCurve.name;
    widget.onRamp(_rampTo, _rampFrames, _rampCurve);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final fps = widget.fps <= 0 ? 1.0 : widget.fps;
    return FocusScope(
      node: _scope,
      child: FloatSurface(
        width: _rampWidth,
        child: Padding(
          padding: const EdgeInsets.all(4),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              keyFieldRow(t, l10n.retimeRampTo, 'ramp-to-$_chipped', _rampTo,
                  min: -10000,
                  max: 10000,
                  decimals: 2,
                  suffix: l10n.unitSymbolPercent,
                  labelWidth: _rampLabel,
                  keyed: false,
                  autofocus: true,
                  submitted: _go,
                  set: (v) => setState(() => _rampTo = v)),
              Padding(
                padding: const EdgeInsetsDirectional.only(
                    start: _rampLabel + 10, top: 2, bottom: 4),
                child: Wrap(
                  spacing: 2,
                  runSpacing: 2,
                  children: [
                    for (final chip in rampChips)
                      HouseButton(
                        key: ValueKey<String>('ramp-chip-$chip'),
                        small: true,
                        frameless: _rampTo != chip,
                        padding: const EdgeInsets.symmetric(
                            horizontal: 5, vertical: 1),
                        onPressed: () => setState(() {
                          _rampTo = chip.toDouble();
                          _chipped++;
                        }),
                        child: Text('$chip',
                            style: t.mono.copyWith(
                                fontSize: wellTextSize,
                                color: _rampTo == chip
                                    ? t.textPrimary
                                    : t.textSecondary)),
                      ),
                  ],
                ),
              ),
              Row(
                children: [
                  Expanded(
                    child: keyFieldRow(
                      t,
                      l10n.retimeRampOver,
                      'ramp-over',
                      _rampInSeconds
                          ? _rampFrames / fps
                          : _rampFrames.toDouble(),
                      min: _rampInSeconds ? 1 / fps : 1,
                      max: 100000,
                      decimals: _rampInSeconds ? 2 : 0,
                      labelWidth: _rampLabel,
                      keyed: false,
                      submitted: _go,
                      set: (v) => setState(() {
                        final frames = _rampInSeconds ? v * fps : v;
                        _rampFrames = frames.round() < 1 ? 1 : frames.round();
                      }),
                    ),
                  ),
                  const SizedBox(width: 4),
                  Segment(
                    key: const ValueKey('ramp-unit'),
                    children: [
                      for (final seconds in const [false, true])
                        SegmentOption(
                          active: _rampInSeconds == seconds,
                          onTap: () => setState(() => _rampInSeconds = seconds),
                          child: Text(
                              t.kickerCase(seconds
                                  ? l10n.unitSymbolSeconds
                                  : l10n.unitSymbolFramesShort),
                              style: _rampInSeconds == seconds
                                  ? t.kickerOn
                                  : t.kicker),
                        ),
                    ],
                  ),
                  const SizedBox(width: 4),
                ],
              ),
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
                child: Row(
                  children: [
                    SizedBox(
                      width: _rampLabel,
                      child: Text(l10n.retimeRampEase,
                          style: t.body,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis),
                    ),
                    const SizedBox(width: 6),
                    Expanded(
                      child: BareDropdown<SpeedCurve>(
                        key: const ValueKey('ramp-ease'),
                        dense: true,
                        value: _rampCurve,
                        options: SpeedCurve.values,
                        label: speedCurveName,
                        onChanged: (curve) =>
                            setState(() => _rampCurve = curve),
                      ),
                    ),
                  ],
                ),
              ),
              Padding(
                padding: const EdgeInsets.fromLTRB(4, 6, 4, 2),
                child: Align(
                  alignment: AlignmentDirectional.centerEnd,
                  child: HouseButton(
                    key: const ValueKey('ramp-go'),
                    primary: true,
                    small: true,
                    onPressed: _go,
                    child: Text(l10n.retimeRampGo),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Ask how long a freeze holds, in seconds from [seconds]. `Enter` or the button
/// runs it, and [onHold] hears the seconds.
Future<void> showHoldPopover({
  required BuildContext context,
  required Offset position,
  required double seconds,
  required ValueChanged<double> onHold,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => _HoldPopover(
        seconds: seconds,
        onHold: (held) {
          close(null);
          onHold(held);
        },
      ),
    );

class _HoldPopover extends StatefulWidget {
  final double seconds;
  final ValueChanged<double> onHold;

  const _HoldPopover({required this.seconds, required this.onHold});

  @override
  State<_HoldPopover> createState() => _HoldPopoverState();
}

class _HoldPopoverState extends State<_HoldPopover> {
  final FocusScopeNode _scope = FocusScopeNode(debugLabel: 'hold');
  late double _seconds = widget.seconds;

  @override
  void dispose() {
    _scope.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FocusScope(
      node: _scope,
      child: FloatSurface(
        width: 204,
        child: Padding(
          padding: const EdgeInsets.all(4),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              keyFieldRow(t, l10n.duration, 'hold-length', _seconds,
                  min: 0.01,
                  max: 100000,
                  decimals: 2,
                  suffix: l10n.unitSymbolSeconds,
                  labelWidth: 56,
                  keyed: false,
                  autofocus: true,
                  submitted: () => widget.onHold(_seconds),
                  set: (v) => setState(() => _seconds = v)),
              Padding(
                padding: const EdgeInsets.fromLTRB(4, 6, 4, 2),
                child: Align(
                  alignment: AlignmentDirectional.centerEnd,
                  child: HouseButton(
                    key: const ValueKey('hold-go'),
                    primary: true,
                    small: true,
                    onPressed: () => widget.onHold(_seconds),
                    child: Text(l10n.retimeFreeze),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
