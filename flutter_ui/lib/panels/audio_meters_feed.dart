// The bars' pulse: one poll of the engine's meter tap, shared by the Mixer
// strips and the Audio panel's Levels (docs/09 §3.1).
//
// In plain terms: while a mixer is on screen, something has to keep asking the
// engine "how loud is it right now" and hand the answer to the bars. This is
// that something — one timer for however many bars are drawn, publishing into
// one [ValueNotifier] that the meter *painters* listen to directly. A tick
// therefore repaints the bars and rebuilds nothing: the strips, the faders and
// the wells never hear it (the rebuild gates; docs/impl/ui-performance.md
// WP-2's listenable-inside-a-boundary shape).
//
// The peak hold, the line resting above the bar, is computed here, because
// docs/09 §3.1 makes it the panel's own and because a painter must stay a pure
// function of the frame it is handed.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart';

import 'mixer_live.dart';

/// How long every meter's hold line rests on the loudest peak before letting
/// go, in seconds. Nought draws no line, and a negative number holds it until
/// [resetMeterPeaks]. Set from the person's settings, and shared because a
/// Mixer and an Audio panel side by side should not disagree about it.
int meterPeakHoldSeconds = 3;

/// The feeds polling now, for [resetMeterPeaks] to reach.
final Set<AudioMeterFeed> _running = {};

/// Forget every held peak and put the clip and limiter lights out. The mix is
/// untouched.
void resetMeterPeaks() {
  resetAudioClip();
  for (final feed in _running) {
    feed._holds.clear();
    feed._limited = false;
    feed.clips.value = const {};
    // A parked poll would leave the lines where they are.
    feed.wake();
  }
}

/// Start the master's integrated loudness and its true peak afresh, and the
/// history drawn of it.
void resetLoudness() {
  resetAudioLoudness();
  for (final feed in _running) {
    feed.history.clear();
    feed.wake();
  }
}

/// The master's recent short-term loudness, oldest first: a ring of the last
/// [length] readings, taken ten times a second while the comp plays. Minus
/// infinity where nothing was measured. View state only; it notifies as it
/// moves, for the graph's painter.
class LoudnessHistory extends ChangeNotifier {
  /// Thirty seconds of readings.
  static const int length = 300;

  final Float64List _ring = Float64List(length);
  int _head = 0;
  int _count = 0;

  int get count => _count;

  /// The [i]th reading still held, nought the oldest.
  double operator [](int i) => _ring[(_head - _count + i) % length];

  void add(double lufs) {
    _ring[_head] = lufs;
    _head = (_head + 1) % length;
    if (_count < length) _count++;
    notifyListeners();
  }

  void clear() {
    if (_count == 0) return;
    _count = 0;
    notifyListeners();
  }
}

/// The master's loudness as drawn: short-term and integrated in LUFS, minus
/// infinity while there is none, and the true peak in dB, rounded to the tenth
/// the numbers show so a reading that has not moved on screen changes nothing.
/// The momentary loudness is in LUFS too, and the range in LU.
class LoudnessLevels {
  final double momentary, shortTerm, integrated, truePeak, range;

  const LoudnessLevels({
    this.momentary = double.negativeInfinity,
    this.shortTerm = double.negativeInfinity,
    this.integrated = double.negativeInfinity,
    this.truePeak = double.negativeInfinity,
    // The engine's own range before anything is measured.
    this.range = 0,
  });

  static const LoudnessLevels silence = LoudnessLevels();

  factory LoudnessLevels.of(BridgeAudioLoudness l) {
    // A reading with nothing behind it is minus infinity, never NaN, so an
    // unchanged reading compares equal and redraws nothing.
    double tenth(double v) => v.isNaN
        ? double.negativeInfinity
        : v.isFinite
            ? (v * 10).roundToDouble() / 10
            : v;
    return LoudnessLevels(
      momentary: tenth(l.momentary),
      range: tenth(l.range),
      shortTerm: tenth(l.shortTerm),
      integrated: tenth(l.integrated),
      truePeak: tenth(l.truePeak > 0
          ? 20 * math.log(l.truePeak) / math.ln10
          : double.negativeInfinity),
    );
  }

  @override
  bool operator ==(Object other) =>
      other is LoudnessLevels &&
      other.shortTerm == shortTerm &&
      other.integrated == integrated &&
      other.truePeak == truePeak &&
      other.momentary == momentary &&
      other.range == range;

  @override
  int get hashCode =>
      Object.hash(shortTerm, integrated, truePeak, momentary, range);
}

/// One strip's reading, dB-ready: linear amplitudes straight off the tap,
/// converted where they are drawn.
class StripLevels {
  final double peakLeft, peakRight;
  final double rmsLeft, rmsRight;

  /// The panel's own hold (docs/09 §3.1): the loudest peak of the last few
  /// seconds, per channel.
  final double holdLeft, holdRight;

  /// How far the strip is held down, in dB: ducked, or the master limited.
  final double gainReductionDb;

  const StripLevels({
    this.peakLeft = 0,
    this.peakRight = 0,
    this.rmsLeft = 0,
    this.rmsRight = 0,
    this.holdLeft = 0,
    this.holdRight = 0,
    this.gainReductionDb = 0,
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
      other.holdRight == holdRight &&
      other.gainReductionDb == gainReductionDb;

  @override
  int get hashCode => Object.hash(peakLeft, peakRight, rmsLeft, rmsRight,
      holdLeft, holdRight, gainReductionDb);
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
/// numbers moved, so a paused transport costs no repaints at all. Started with
/// a `wake`, the poll also parks itself once the bars have been silent a
/// second with no hold left to fall, and that wakes it again.
class AudioMeterFeed {
  final ValueNotifier<AudioMeterFrame> frame =
      ValueNotifier(AudioMeterFrame.silence);

  /// The clip lamp's own listenable, split from [frame] so the lamp — a
  /// widget, unlike the painted bars — rebuilds only when the light actually
  /// changes state, never per tick.
  final ValueNotifier<bool> clipped = ValueNotifier(false);

  /// The master's loudness, its own listenable for the same reason: its
  /// numbers rebuild when they change, and nothing else does.
  final ValueNotifier<LoudnessLevels> loudness =
      ValueNotifier(LoudnessLevels.silence);

  /// The short-term loudness over the last half minute of playing.
  final LoudnessHistory history = LoudnessHistory();
  int _historyTicks = 0;

  /// Whether the controls' places and the pass are read while playing: the
  /// Mixer's faders want them, the Audio panel's levels do not.
  bool _follow = true;

  /// The strips whose sound has reached full scale since their lamp was last
  /// put out, by layer id. Sticky, like the limiter's light.
  final ValueNotifier<Set<String>> clips = ValueNotifier(const {});

  /// Where the faders, pots and send levels stand at the clock while the comp
  /// plays, by layer id. Empty while stopped, so the strips read the model.
  final ValueNotifier<Map<String, StripPosition>> positions =
      ValueNotifier(const {});

  /// Whether the limiter has held the master down since the light was put out.
  bool _limited = false;

  /// The controls an automation pass is writing now. Empty while stopped.
  final ValueNotifier<Set<BridgeMixTarget>> recording =
      ValueNotifier(const {});

  int _passTicks = 0;

  /// Whether the comp is playing, which is when [positions] and [recording]
  /// are read.
  ValueListenable<bool>? _playing;

  Timer? _timer;
  final Map<String, _Hold> _holds = {};

  /// What [_tick] reads; swapped by tests to feed the meters without an
  /// engine. The application never touches it.
  @visibleForTesting
  List<BridgeAudioMeter> Function() read = audioMeters;

  /// What [tick] reads the loudness with, asked only when the engine has a
  /// master to measure.
  @visibleForTesting
  BridgeAudioLoudness Function() readLoudness = audioLoudness;

  /// What [tick] reads the faders' places with while the comp plays.
  @visibleForTesting
  Map<String, StripPosition> Function() readPositions = mixerPositions;

  /// What [tick] reads the pass being recorded with while the comp plays,
  /// set by the panel that knows the comp.
  List<BridgeAutomationLine> Function() readPass = () => const [];

  /// About 30 frames a second: enough for a bar to read as live, and each
  /// tick is one sync crossing that copies a handful of numbers.
  static const Duration _period = Duration(milliseconds: 33);

  /// What starts a parked poll again: the transport starting, or the
  /// playhead moving under a scrub that may be heard.
  Listenable? _wake;

  /// When something was last heard, or the poll last woken.
  DateTime _heard = DateTime.fromMillisecondsSinceEpoch(0);

  /// How long the bars stay silent before the poll parks. Long enough for a
  /// scrub's burst to reach the tap after the playhead moved.
  static const Duration _quiet = Duration(seconds: 1);

  void start(
      {Listenable? wake, ValueListenable<bool>? playing, bool follow = true}) {
    _running.add(this);
    _wake = wake?..addListener(this.wake);
    _playing = playing;
    _follow = follow;
    this.wake();
  }

  /// Put out one strip's clip lamp.
  void clearClip(String layer) {
    if (!clips.value.contains(layer)) return;
    clips.value = {...clips.value}..remove(layer);
  }

  /// Poll again, because something may be about to sound.
  void wake() {
    _heard = DateTime.now();
    _timer ??= Timer.periodic(_period, (_) => tick());
  }

  void stop() {
    _running.remove(this);
    _wake?.removeListener(wake);
    _wake = null;
    _playing = null;
    _timer?.cancel();
    _timer = null;
  }

  /// One poll — public so a test can pulse the feed by hand.
  @visibleForTesting
  void tick() {
    final now = DateTime.now();
    final strips = <String, StripLevels>{};
    var anyClipped = false;
    Set<String>? lamps;
    for (final meter in read()) {
      anyClipped = anyClipped || meter.clipped;
      final reduction = meter.gainReductionDb;
      if (meter.kind == BridgeMeterKind.master && reduction > 0.1) {
        _limited = true;
      }
      if (meter.layer.isNotEmpty &&
          math.max(meter.peakLeft, meter.peakRight) >= 1 &&
          !clips.value.contains(meter.layer)) {
        (lamps ??= {...clips.value}).add(meter.layer);
      }
      strips[meter.layer] = StripLevels(
        gainReductionDb: reduction,
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
    if (lamps != null) clips.value = lamps;
    anyClipped = anyClipped || _limited;
    if (clipped.value != anyClipped) clipped.value = anyClipped;
    final playing = _playing?.value ?? false;
    final follow = playing && _follow;
    final placed = follow ? readPositions() : const <String, StripPosition>{};
    if (!mapEquals(placed, positions.value)) positions.value = placed;
    // The pass carries every point written so far, so it is asked a few
    // times a second rather than every tick.
    if (!follow || _passTicks++ % 8 == 0) {
      final written = follow
          ? {
              for (final line in readPass())
                if (line.points.isNotEmpty) line.target,
            }
          : const <BridgeMixTarget>{};
      if (!setEquals(written, recording.value)) recording.value = written;
    }
    if (strips.containsKey('')) {
      loudness.value = LoudnessLevels.of(readLoudness());
      if (playing && _historyTicks++ % 3 == 0) {
        history.add(loudness.value.shortTerm);
      }
    }
    if (playing || strips.values.any((s) => s != StripLevels.silence)) {
      _heard = now;
    } else if (_wake != null && now.difference(_heard) > _quiet) {
      _timer?.cancel();
      _timer = null;
    }
  }

  /// A rising peak takes the hold at once; a fallen one keeps it for
  /// [meterPeakHoldSeconds] and then lets it drop to the level that is there.
  double _hold(String key, double peak, DateTime now) {
    final seconds = meterPeakHoldSeconds;
    if (seconds == 0) return 0;
    final hold = _holds.putIfAbsent(key, _Hold.new);
    if (peak >= hold.value ||
        (seconds > 0 && now.difference(hold.since).inSeconds >= seconds)) {
      hold.value = peak;
      hold.since = now;
    }
    return hold.value;
  }

  void dispose() {
    stop();
    frame.dispose();
    clipped.dispose();
    loudness.dispose();
    history.dispose();
    clips.dispose();
    positions.dispose();
    recording.dispose();
  }
}
