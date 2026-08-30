// The bars' pulse: one poll of the engine's meter tap, shared by the Mixer
// strips and the Audio panel's Levels (docs/09 §3.1, K-690).
//
// In plain terms: while a mixer is on screen, something has to keep asking the
// engine "how loud is it right now" and hand the answer to the bars. This is
// that something — one timer for however many bars are drawn, publishing into
// one [ValueNotifier] that the meter *painters* listen to directly. A tick
// therefore repaints the bars and rebuilds nothing: the strips, the faders and
// the wells never hear it (the K-681 gates; docs/impl/ui-performance.md WP-2's
// listenable-inside-a-boundary shape).
//
// The peak hold — the line resting above the bar for a few seconds — is
// computed here, because docs/09 §3.1 makes it the panel's own and because a
// painter must stay a pure function of the frame it is handed.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';

/// How long the hold line rests on the loudest peak before letting go —
/// docs/09 §3.1's "a few seconds", and the board's own caption ("peak hold
/// 3 s").
const Duration peakHoldTime = Duration(seconds: 3);

/// One strip's reading, dB-ready: linear amplitudes straight off the tap,
/// converted where they are drawn.
class StripLevels {
  final double peakLeft, peakRight;
  final double rmsLeft, rmsRight;

  /// The panel's own hold (docs/09 §3.1): the loudest peak of the last few
  /// seconds, per channel.
  final double holdLeft, holdRight;

  const StripLevels({
    this.peakLeft = 0,
    this.peakRight = 0,
    this.rmsLeft = 0,
    this.rmsRight = 0,
    this.holdLeft = 0,
    this.holdRight = 0,
  });

  static const StripLevels silence = StripLevels();

  @override
  bool operator ==(Object other) =>
      other is StripLevels &&
      other.peakLeft == peakLeft &&
      other.peakRight == peakRight &&
      other.rmsLeft == rmsLeft &&
      other.rmsRight == rmsRight &&
      other.holdLeft == holdLeft &&
      other.holdRight == holdRight;

  @override
  int get hashCode =>
      Object.hash(peakLeft, peakRight, rmsLeft, rmsRight, holdLeft, holdRight);
}

/// Everything one poll learned: strips by layer id (the empty id is the
/// master), and whether anything has hit the ceiling since the lights were
/// last put out.
class AudioMeterFrame {
  final Map<String, StripLevels> strips;
  final bool clipped;

  const AudioMeterFrame(this.strips, {required this.clipped});

  static const AudioMeterFrame silence =
      AudioMeterFrame(<String, StripLevels>{}, clipped: false);

  /// The named strip's bars, silent when the engine has nothing for it — a
  /// paused transport, a strip past the meter bank, a mix not yet loaded.
  StripLevels of(String layer) => strips[layer] ?? StripLevels.silence;

  StripLevels get master => of('');
}

/// Linear amplitude to decibels for a bar, floored at −60 (the bottom of the
/// drawn scale) so silence is a number rather than −∞.
double amplitudeDb(double amplitude) =>
    amplitude <= 0 ? -60.0 : math.max(-60.0, 20.0 * math.log(amplitude) / math.ln10);

/// The fraction of a −60..0 dB bar an amplitude fills.
double meterFraction(double amplitude) =>
    ((amplitudeDb(amplitude) + 60.0) / 60.0).clamp(0.0, 1.0);

class _Hold {
  double value = 0;
  DateTime since = DateTime.fromMillisecondsSinceEpoch(0);
}

/// Polls [audioMeters] at UI rate while started, publishing frames.
///
/// One short sync call per tick — the engine reads lock-free atomics the
/// callback publishes (docs/09 §3.1) — and the notifier only fires when the
/// numbers moved, so a paused transport costs no repaints at all.
class AudioMeterFeed {
  final ValueNotifier<AudioMeterFrame> frame =
      ValueNotifier(AudioMeterFrame.silence);

  /// The clip lamp's own listenable, split from [frame] so the lamp — a
  /// widget, unlike the painted bars — rebuilds only when the light actually
  /// changes state, never per tick.
  final ValueNotifier<bool> clipped = ValueNotifier(false);

  Timer? _timer;
  final Map<String, _Hold> _holds = {};

  /// What [_tick] reads; swapped by tests to feed the meters without an
  /// engine. The application never touches it.
  @visibleForTesting
  List<BridgeAudioMeter> Function() read = audioMeters;

  /// About 30 frames a second: enough for a bar to read as live, and each
  /// tick is one sync crossing that copies a handful of numbers.
  static const Duration _period = Duration(milliseconds: 33);

  void start() => _timer ??= Timer.periodic(_period, (_) => tick());

  void stop() {
    _timer?.cancel();
    _timer = null;
  }

  /// One poll — public so a test can pulse the feed by hand.
  @visibleForTesting
  void tick() {
    final now = DateTime.now();
    final strips = <String, StripLevels>{};
    var anyClipped = false;
    for (final meter in read()) {
      anyClipped = anyClipped || meter.clipped;
      strips[meter.layer] = StripLevels(
        peakLeft: meter.peakLeft,
        peakRight: meter.peakRight,
        rmsLeft: meter.rmsLeft,
        rmsRight: meter.rmsRight,
        holdLeft: _hold('${meter.layer}/L', meter.peakLeft, now),
        holdRight: _hold('${meter.layer}/R', meter.peakRight, now),
      );
    }
    final next = AudioMeterFrame(strips, clipped: anyClipped);
    if (!mapEquals(next.strips, frame.value.strips) ||
        next.clipped != frame.value.clipped) {
      frame.value = next;
    }
    if (clipped.value != anyClipped) clipped.value = anyClipped;
  }

  /// A rising peak takes the hold at once; a fallen one keeps it for
  /// [peakHoldTime] and then lets it drop to the level that is there.
  double _hold(String key, double peak, DateTime now) {
    final hold = _holds.putIfAbsent(key, _Hold.new);
    if (peak >= hold.value || now.difference(hold.since) > peakHoldTime) {
      hold.value = peak;
      hold.since = now;
    }
    return hold.value;
  }

  void dispose() {
    stop();
    frame.dispose();
    clipped.dispose();
  }
}
