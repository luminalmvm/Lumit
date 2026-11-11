// The sound clean-up effects' display: what arrives at the effect around the
// playhead, and what the effect does to it.
//
// **In plain terms.** A small spectrum, low notes on the left and high on the
// right, quiet at the bottom. The grey ground is the sound coming in. The line
// in the accent is what the effect is working from: the noise floor it learned
// or found, or the late reverb it hears. The light line is what comes out. Hum
// removal marks the hum and its harmonics instead, Click removal draws the
// sound around the playhead with each click it finds, and Reverb reduction
// says how long the room rings.
//
// **Read, not decided.** Every number is the engine's, worked out from the
// effect's own rows and a window of its input, and asked for again when the
// playhead or a row moves. Nothing is drawn while a request is out, and an
// answer that arrives after the effect has gone is dropped.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/denoise.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'status_poller.dart' show statusPoll;
import 'viewer_noise_sample.dart' show noiseMeasures;

/// The plot's size: the Levels histogram's width, a little taller.
const double _plotWidth = 200;
const double _plotHeight = 64;

/// The frequency axis, as the engine's grid has it.
const double _lowHz = 20, _highHz = 20000;

/// The level axis, in dB.
const double _topDb = 0, _bottomDb = -100;

/// What the engine's view answers for each effect that is not a plain
/// spectrum (nought): the hum, the clicks, or a spectrum with the room's
/// reverb time.
const int _kindHum = 1, _kindClicks = 2, _kindReverb = 3;

/// The sound clean-up effects this display draws for.
const Set<String> repairEffects = {
  'audio_denoise',
  'audio_dehum',
  'audio_declick',
  'audio_deess',
  'audio_dereverb',
};

class RepairDisplayFrb extends StatefulWidget {
  final LayerReference layer;
  final UuidValue effectId;
  final String matchName;
  final int playheadFrame;

  /// The effect's values, a drag in flight included, so the plot follows a
  /// row while it moves.
  final Map<String, BridgeEffectValue> values;

  /// Bumped by the panel when one of the effect's buttons is pressed.
  final int pressed;

  const RepairDisplayFrb({
    super.key,
    required this.layer,
    required this.effectId,
    required this.matchName,
    required this.playheadFrame,
    required this.values,
    required this.pressed,
  });

  @override
  State<RepairDisplayFrb> createState() => _RepairDisplayFrbState();
}

class _RepairDisplayFrbState extends State<RepairDisplayFrb> {
  BridgeRepairView? _view;
  bool _asking = false;
  bool _again = false;
  Timer? _settle;

  /// Noise reduction's Learn, while it runs: read twice a second until it
  /// lands, as the trackers' cards read theirs.
  Timer? _learning;
  BridgeNoiseStatus? _learn;

  bool get _learns => widget.matchName == 'audio_denoise';

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _ask());
    if (_learns) noiseMeasures.addListener(_watchLearn);
  }

  /// A Learn has been asked for, here or from a key or a menu: follow it, and
  /// draw the profile it leaves.
  void _watchLearn() {
    if (!mounted || !_learns) return;
    final BridgeNoiseStatus next;
    try {
      next = noiseStatus(layer: widget.layer, effect: widget.effectId);
    } catch (_) {
      _learning?.cancel();
      _learning = null;
      return;
    }
    final was = _learn;
    if (next != was) setState(() => _learn = next);
    if (next.busy) {
      _learning ??= Timer.periodic(statusPoll, (_) => _watchLearn());
      return;
    }
    _learning?.cancel();
    _learning = null;
    if (was?.busy ?? false) unawaited(_ask());
  }

  @override
  void didUpdateWidget(RepairDisplayFrb old) {
    super.didUpdateWidget(old);
    if (old.pressed != widget.pressed) _watchLearn();
    if (old.playheadFrame != widget.playheadFrame ||
        old.pressed != widget.pressed ||
        !_sameValues(old.values, widget.values)) {
      // A burst of moves (playback, a drag) asks once it settles a little,
      // and at most one request is ever out.
      _settle?.cancel();
      _settle = Timer(const Duration(milliseconds: 60), _ask);
    }
  }

  @override
  void dispose() {
    if (_learns) noiseMeasures.removeListener(_watchLearn);
    _learning?.cancel();
    _settle?.cancel();
    super.dispose();
  }

  static bool _sameValues(
      Map<String, BridgeEffectValue> a, Map<String, BridgeEffectValue> b) {
    if (a.length != b.length) return false;
    for (final entry in a.entries) {
      if (b[entry.key] != entry.value) return false;
    }
    return true;
  }

  Future<void> _ask() async {
    if (!mounted) return;
    if (_asking) {
      _again = true;
      return;
    }
    _asking = true;
    try {
      final view = await repairView(
          layer: widget.layer,
          effect: widget.effectId,
          frame: widget.playheadFrame);
      if (mounted) setState(() => _view = view);
    } catch (_) {
      // The layer or the effect went away under the card: keep the last
      // picture until the panel drops this display.
    } finally {
      _asking = false;
      if (_again && mounted) {
        _again = false;
        unawaited(_ask());
      }
    }
  }

  String _caption(BridgeRepairView? v) {
    final learn = _learn;
    if (learn != null && learn.busy) return l10n.denoiseMeasuring;
    if (learn != null && learn.failure != 0) return l10n.denoiseNoPicture;
    if (v == null) return '';
    if (_learns && v.count > 0) return l10n.repairLearned;
    return switch (v.kind) {
      _kindHum when v.value > 0 => l10n.repairHumAt(v.value.toStringAsFixed(1)),
      // Nothing stands out here, which under speech says little: the effect
      // reads the whole sound and may still find the hum. So no line.
      _kindHum => '',
      _kindClicks => l10n.repairClicks(v.count),
      _kindReverb when v.value > 0 =>
        l10n.repairReverbTime(v.value.toStringAsFixed(2)),
      _ => '',
    };
  }

  /// De-esser's Frequency, drawn as a mark on the axis.
  double? _crossover() {
    if (widget.matchName != 'audio_deess') return null;
    final v = widget.values['frequency'];
    return switch (v) {
      BridgeEffectValue_Float(:final field0) => switch (field0) {
          BridgeScalar_Static(:final field0) => field0,
          _ => null,
        },
      _ => null,
    };
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final v = _view;
    final caption = _caption(v);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            key: ValueKey<String>('fx-repair-plot-${widget.effectId}'),
            width: _plotWidth,
            height: _plotHeight,
            color: t.surface0,
            child: CustomPaint(
              painter: v == null
                  ? null
                  : v.kind == _kindClicks
                      ? _ClicksPainter(
                          wave: v.wave,
                          marks: v.marks,
                          waveColour: t.textMuted,
                          markColour: t.accent,
                        )
                      : _SpectrumPainter(
                          view: v,
                          crossover: _crossover(),
                          ground: t.textMuted,
                          floor: t.accent,
                          output: t.textPrimary,
                          grid: t.hairline,
                        ),
            ),
          ),
          if (v != null && v.kind != _kindClicks)
            SizedBox(
              width: _plotWidth,
              child: CustomPaint(
                size: const Size(_plotWidth, 12),
                painter: _AxisPainter(
                  style: t.small.copyWith(color: t.textMuted, fontSize: 9),
                  labels: [
                    l10n.repairAxisHertz('100'),
                    l10n.repairAxisKilohertz('1'),
                    l10n.repairAxisKilohertz('10'),
                  ],
                ),
              ),
            ),
          if (caption.isNotEmpty) ...[
            const SizedBox(height: 2),
            Text(
              caption,
              key: ValueKey<String>('fx-repair-caption-${widget.effectId}'),
              style: t.small.copyWith(
                color: t.textMuted,
                fontFeatures: const [FontFeature.tabularFigures()],
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ],
        ],
      ),
    );
  }
}

/// Where a frequency sits across the plot, 0 to 1, on a log axis.
double _xOf(double hz) =>
    (math.log(hz.clamp(_lowHz, _highHz) / _lowHz) / math.log(_highHz / _lowHz));

/// Where a level sits down the plot, 0 at the top.
double _yOf(double db) => ((_topDb - db) / (_topDb - _bottomDb)).clamp(0.0, 1.0);

class _SpectrumPainter extends CustomPainter {
  final BridgeRepairView view;
  final double? crossover;
  final Color ground, floor, output, grid;

  const _SpectrumPainter({
    required this.view,
    required this.crossover,
    required this.ground,
    required this.floor,
    required this.output,
    required this.grid,
  });

  /// One curve as a path across the plot: the engine's points are evenly
  /// spaced in log frequency, so point i sits at i / (n - 1) of the width.
  Path _line(List<double> db, Size size) {
    final path = Path();
    for (var i = 0; i < db.length; i++) {
      final x = i / (db.length - 1) * size.width;
      final y = _yOf(db[i]) * size.height;
      i == 0 ? path.moveTo(x, y) : path.lineTo(x, y);
    }
    return path;
  }

  /// A curve drawn only where it is on the plot: a stretch lying along the
  /// bottom edge is the effect doing nothing there, not a line to read.
  Path _lineAbove(List<double> db, Size size) {
    final path = Path();
    var down = false;
    for (var i = 0; i < db.length; i++) {
      if (db[i] <= _bottomDb + 0.5) {
        down = false;
        continue;
      }
      final x = i / (db.length - 1) * size.width;
      final y = _yOf(db[i]) * size.height;
      down ? path.lineTo(x, y) : path.moveTo(x, y);
      down = true;
    }
    return path;
  }

  @override
  void paint(Canvas canvas, Size size) {
    final hair = Paint()
      ..color = grid
      ..strokeWidth = 1;
    for (final hz in const [100.0, 1000.0, 10000.0]) {
      final x = (_xOf(hz) * size.width).roundToDouble() + 0.5;
      canvas.drawLine(Offset(x, 0), Offset(x, size.height), hair);
    }
    final input = view.input;
    if (input.length > 1) {
      final fill = _line(input, size)
        ..lineTo(size.width, size.height)
        ..lineTo(0, size.height)
        ..close();
      canvas.drawPath(fill, Paint()..color = ground.withValues(alpha: 0.35));
    }
    final stroke = Paint()
      ..style = PaintingStyle.stroke
      ..strokeJoin = StrokeJoin.round;
    if (view.floor.length > 1) {
      canvas.drawPath(
          _line(view.floor, size),
          stroke
            ..color = floor.withValues(alpha: 0.5)
            ..strokeWidth = 1);
    }
    if (view.threshold.length > 1) {
      canvas.drawPath(
          _lineAbove(view.threshold, size),
          stroke
            ..color = floor
            ..strokeWidth = 1);
    }
    if (view.output.length > 1) {
      canvas.drawPath(
          _line(view.output, size),
          stroke
            ..color = output
            ..strokeWidth = 1.25);
    }
    // The hum and its harmonics, as ticks along the top.
    if (view.kind == _kindHum && view.value > 0) {
      final tick = Paint()
        ..color = floor
        ..strokeWidth = 1;
      for (var h = 1; h <= math.max(1, view.count); h++) {
        final x = (_xOf(view.value * h) * size.width).roundToDouble() + 0.5;
        canvas.drawLine(Offset(x, 0), Offset(x, size.height * 0.3), tick);
      }
    }
    final cross = crossover;
    if (cross != null) {
      final x = (_xOf(cross) * size.width).roundToDouble() + 0.5;
      canvas.drawLine(
          Offset(x, 0),
          Offset(x, size.height),
          Paint()
            ..color = floor
            ..strokeWidth = 1);
    }
  }

  @override
  bool shouldRepaint(_SpectrumPainter old) =>
      old.view != view ||
      old.crossover != crossover ||
      old.ground != ground ||
      old.floor != floor ||
      old.output != output ||
      old.grid != grid;
}

/// The axis under a spectrum: 100, 1k and 10k under their hairlines.
class _AxisPainter extends CustomPainter {
  final TextStyle style;

  /// The marks' labels at 100 Hz, 1 kHz and 10 kHz.
  final List<String> labels;

  const _AxisPainter({required this.style, required this.labels});

  @override
  void paint(Canvas canvas, Size size) {
    for (final (hz, label) in [
      (100.0, labels[0]),
      (1000.0, labels[1]),
      (10000.0, labels[2]),
    ]) {
      final text = TextPainter(
        text: TextSpan(text: label, style: style),
        textDirection: TextDirection.ltr,
      )..layout();
      final x = _xOf(hz) * size.width - text.width / 2;
      text.paint(canvas, Offset(x, 1));
      text.dispose();
    }
  }

  @override
  bool shouldRepaint(_AxisPainter old) =>
      old.style != style ||
      old.labels[0] != labels[0] ||
      old.labels[1] != labels[1] ||
      old.labels[2] != labels[2];
}

/// The sound around the playhead as columns of its range, with each click
/// found marked across it.
class _ClicksPainter extends CustomPainter {
  /// Min and max per column, -1 to 1, in pairs.
  final List<double> wave;

  /// Each click's start and end as fractions of the window, in pairs.
  final List<double> marks;
  final Color waveColour, markColour;

  const _ClicksPainter({
    required this.wave,
    required this.marks,
    required this.waveColour,
    required this.markColour,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final mid = size.height / 2;
    final columns = wave.length ~/ 2;
    final line = Paint()
      ..color = waveColour.withValues(alpha: 0.7)
      ..strokeWidth = 1;
    for (var c = 0; c < columns; c++) {
      final x = (c + 0.5) / columns * size.width;
      canvas.drawLine(Offset(x, mid - wave[c * 2 + 1] * mid),
          Offset(x, mid - wave[c * 2] * mid), line);
    }
    // Ticks along the top, as Hum removal marks its harmonics, so a stretch
    // dense with clicks still shows the sound under them.
    final mark = Paint()..color = markColour;
    for (var i = 0; i + 1 < marks.length; i += 2) {
      final a = (marks[i] * size.width).floorToDouble();
      final b = marks[i + 1] * size.width;
      canvas.drawRect(
          Rect.fromLTRB(a, 0, math.max(b, a + 1), size.height * 0.3), mark);
    }
  }

  @override
  bool shouldRepaint(_ClicksPainter old) =>
      old.wave != wave ||
      old.marks != marks ||
      old.waveColour != waveColour ||
      old.markColour != markColour;
}
