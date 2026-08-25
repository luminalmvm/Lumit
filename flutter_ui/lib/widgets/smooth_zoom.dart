// Zooming that flies rather than cuts, in one place (docs/07-UI-SPEC.md §4.6).
//
// **In plain terms.** Changing magnification is a *place* changing, not a value
// being nudged: jump straight from one zoom to another and the reader loses
// where they were. The Viewer has flown since K-218; the Timeline's time zoom,
// the graph editor's and the Project panel's thumbnails all still cut. This is
// the Viewer's shape lifted out so the other three read it rather than writing
// it three more times.
//
// **Two things it adds over a plain animation.**
//
// *Zoom is a ratio, so it moves geometrically.* Lerping 1 → 8 linearly spends
// half the flight between 4 and 8, which reads as a lurch then a crawl; lerping
// the logarithm spends equal time on equal *ratios*, which is what the eye
// calls even. The Viewer already does this and the comment there says why.
//
// *Notches that arrive fast zoom further.* A wheel rolled hard should cover
// ground; the same wheel rolled one notch at a time should be precise. So a
// notch is worth more the sooner it follows the last one — and when the hand
// stops, the flight in progress finishes and settles rather than being cut off
// where the last notch left it.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

/// How long a zoom flight lasts when nothing interrupts it.
///
/// Matched to the Viewer's own (K-218): long enough to read as motion, short
/// enough that a second notch lands inside it, which is what makes a rolled
/// wheel feel continuous rather than stepped.
const Duration smoothZoomFlight = Duration(milliseconds: 140);

/// Below this gap between notches the wheel counts as *rolled* rather than
/// clicked, and each notch starts being worth more.
const Duration smoothZoomFastGap = Duration(milliseconds: 120);

/// The most a single notch may be multiplied by, however hard the wheel is
/// rolled. Without a ceiling a fast flick crosses the whole zoom range in one
/// gesture and there is no way back to where you were.
const double smoothZoomMaxBoost = 4.0;

/// How much a notch counts for, given how long since the last one.
///
/// One notch on its own is worth exactly itself. Notches arriving faster than
/// [smoothZoomFastGap] are worth progressively more, up to [smoothZoomMaxBoost]
/// — so a rolled wheel covers ground and a clicked one stays precise. The curve
/// is linear in the *gap*, which is the thing the hand controls directly.
///
/// Pure, and the whole of the acceleration rule, so it is tested on its own.
double zoomBoost(Duration sinceLast, {double max = smoothZoomMaxBoost}) {
  final gap = sinceLast.inMicroseconds;
  final fast = smoothZoomFastGap.inMicroseconds;
  if (gap <= 0) return max;
  if (gap >= fast) return 1;
  // gap == fast → 1; gap → 0 → max.
  return 1 + (max - 1) * (1 - gap / fast);
}

/// A zoom factor that flies to where it is sent.
///
/// Holds two numbers: where the zoom *is* (what a caller should draw at, and
/// what [value] answers) and where it is going. Sending it somewhere new while
/// it is already moving retargets the flight from wherever it has reached, so a
/// rolled wheel is one continuous motion rather than a series of restarts.
///
/// A `ChangeNotifier`, so a caller rebuilds on it the way it would on any other
/// listenable, and disposes it with its state.
class SmoothZoom extends ChangeNotifier {
  SmoothZoom({
    required TickerProvider vsync,
    double initial = 1,
    this.min = 0.01,
    this.max = 512,
    Duration Function()? clock,
  })  : _value = initial,
        _target = initial,
        _from = initial,
        _clock = clock ?? _defaultClock {
    _controller = AnimationController(vsync: vsync, duration: smoothZoomFlight)
      ..addListener(_tick);
  }

  /// A monotonic clock, injectable so the acceleration can be driven from a
  /// test without waiting in real time.
  static Duration _defaultClock() =>
      Duration(microseconds: DateTime.now().microsecondsSinceEpoch);

  final double min;

  /// The far end of the range. Settable, because the Timeline's is a property
  /// of the *composition* — full zoom-in is a fixed number of frames across the
  /// panel, so a longer comp zooms further (K-293).
  double max;
  final Duration Function() _clock;

  late final AnimationController _controller;
  double _value;
  double _target;
  double _from;
  Duration? _lastNudge;

  /// The magnification to draw at, this frame.
  double get value => _value;

  /// Where the flight in progress is heading — what a readout showing "the
  /// zoom" should say, since that is the number the user has asked for.
  double get target => _target;

  /// Whether a flight is in progress.
  bool get moving => _controller.isAnimating;

  void _tick() {
    // Geometric, not linear: magnification is a *ratio*, so equal time should
    // buy equal ratio. Lerping the logarithm is what does that.
    final t = _controller.value;
    _value = math.exp(
      _lerp(math.log(_from), math.log(_target), t),
    );
    notifyListeners();
  }

  static double _lerp(double a, double b, double t) => a + (b - a) * t;

  /// Send the zoom to [to], flying there over [duration].
  ///
  /// A [duration] of zero arrives at once, which is what the shell's
  /// reduced-motion setting asks for — the destination is the same either way,
  /// so nothing about the result depends on the animation running.
  void goTo(double to, {Duration duration = smoothZoomFlight}) {
    final clamped = to.clamp(min, max).toDouble();
    if (clamped == _target && !_controller.isAnimating) return;
    _from = _value;
    _target = clamped;
    if (duration == Duration.zero) {
      _controller.stop();
      _value = clamped;
      notifyListeners();
      return;
    }
    _controller
      ..duration = duration
      ..forward(from: 0);
  }

  /// One or more wheel notches, each multiplying the zoom by [factorPerNotch].
  ///
  /// The factor is raised to the boost from [zoomBoost], so notches arriving
  /// quickly go further; the flight then finishes and settles on its own, which
  /// is what makes a hard roll end cleanly rather than stopping dead wherever
  /// the last notch fell.
  void nudge(
    double factorPerNotch, {
    Duration duration = smoothZoomFlight,
  }) {
    final now = _clock();
    final since = _lastNudge == null ? smoothZoomFastGap : now - _lastNudge!;
    _lastNudge = now;
    final boost = zoomBoost(since);
    // Applied to the *target*, not to where the flight has reached, so a second
    // notch inside a flight adds to the journey instead of restarting it short.
    goTo(_target * math.pow(factorPerNotch, boost).toDouble(),
        duration: duration);
  }

  /// Put the zoom at [to] with no flight — for the caller that is restoring a
  /// remembered value rather than performing a gesture.
  void jumpTo(double to) => goTo(to, duration: Duration.zero);

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }
}

/// Where a zoom sits on a slider whose left end is 1 (the whole composition)
/// and whose right end is [maxZoom].
///
/// **Logarithmic, for the same reason the flight is** (K-293): the slider
/// should buy equal *ratio* for equal travel. On a ten-minute comp the linear
/// mapping spends nine tenths of the slider's length inside the last handful of
/// frames, so every useful zoom is crushed into the first centimetre.
///
/// A [maxZoom] of 1 or less — a composition already shorter than full zoom-in
/// shows — has nowhere to travel, so the handle sits at the left.
double zoomSliderPosition(double zoom, double maxZoom) {
  if (maxZoom <= 1) return 0;
  final t = math.log(zoom.clamp(1, maxZoom)) / math.log(maxZoom);
  return t.clamp(0.0, 1.0);
}

/// The inverse of [zoomSliderPosition]: the zoom a handle at [t] means.
double zoomForSliderPosition(double t, double maxZoom) {
  if (maxZoom <= 1) return 1;
  return math.pow(maxZoom, t.clamp(0.0, 1.0)).toDouble();
}
