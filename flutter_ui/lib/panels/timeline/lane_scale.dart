// The Timeline's time↔pixel geometry and snapping, all pure so it can be unit
// tested without a widget tree. Ported from the egui `lane_view`/`drag_secs`
// (crates/lumit-ui/src/shell/widgets.rs) but worked in comp *frames* rather than
// seconds — the playhead, layer in/out points and markers are all integer comp
// frames, so a frame-native scale avoids a fps round-trip on every position.
//
// In plain terms: this file knows how to turn "frame 120" into "so many pixels
// from the left of the lane" and back, how far apart to space the ruler ticks so
// they never crowd, and where a dragged edge should land when snapping is on.

import 'dart:math' as math;

/// The horizontal lane view: how comp frames map to x pixels at a given zoom.
///
/// [zoom] 1.0 fits the whole comp across [trackWidth]; larger zooms in. The view
/// never scrolls past the comp ends, so at zoom 1 it shows the whole comp from
/// frame 0 (mirroring the egui `lane_view` clamp).
class LaneScale {
  /// The x (in the panel's local space) of the lane's left edge — right of the
  /// layer outline column.
  final double trackLeft;

  /// The lane's drawable width in pixels.
  final double trackWidth;

  /// The comp's duration in whole frames (at least 1).
  final int frameCount;

  /// The zoom factor, clamped to [1, 400].
  final double zoom;

  /// The clamped left-edge comp frame of the view.
  final double viewStartFrame;

  const LaneScale._({
    required this.trackLeft,
    required this.trackWidth,
    required this.frameCount,
    required this.zoom,
    required this.viewStartFrame,
  });

  /// Build a clamped lane scale. [desiredStartFrame] is the wanted left edge
  /// (0 anchors the view at the comp start); it is clamped so the view never
  /// scrolls past the comp ends.
  factory LaneScale.fit({
    required double trackLeft,
    required double trackWidth,
    required int frameCount,
    required double zoom,
    double desiredStartFrame = 0,
  }) {
    final z = zoom.clamp(1.0, 400.0);
    final frames = math.max(frameCount, 1);
    final visible = frames / z;
    final start = desiredStartFrame.clamp(0.0, math.max(0.0, frames - visible));
    return LaneScale._(
      trackLeft: trackLeft,
      trackWidth: trackWidth,
      frameCount: frames,
      zoom: z,
      viewStartFrame: start.toDouble(),
    );
  }

  /// Clamp a desired left-edge frame so the view never scrolls past the comp
  /// ends: 0 at zoom 1 (the whole comp fits), else within `[0, frames - visible]`.
  /// The horizontal pan (shift-wheel and the scrollbar) commits through this so
  /// the ruler and lanes stay locked to one view.
  static double clampViewStart({
    required double desired,
    required int frameCount,
    required double zoom,
  }) {
    final z = zoom.clamp(1.0, 400.0);
    final frames = math.max(frameCount, 1);
    final visible = frames / z;
    return desired.clamp(0.0, math.max(0.0, frames - visible)).toDouble();
  }

  /// Whether the view is zoomed past the fit (so a horizontal pan is possible).
  bool get canPan => pxPerFrame > 0 && frameCount / math.max(pxPerFrame, 1e-9) > 0
      ? zoom > 1.0 + 1e-9
      : false;

  /// The fraction of the comp visible in the lane at this zoom (1 at fit).
  double get visibleFraction => (1.0 / zoom).clamp(0.0, 1.0);

  /// Pixels per comp frame at this zoom.
  double get pxPerFrame => trackWidth * zoom / math.max(frameCount, 1);

  /// The x pixel of comp frame [frame].
  double xOfFrame(num frame) => trackLeft + (frame - viewStartFrame) * pxPerFrame;

  /// The comp frame (fractional) under x pixel [x].
  double frameOfX(double x) =>
      viewStartFrame + (x - trackLeft) / math.max(pxPerFrame, 1e-9);
}

/// Which tick spacing the ruler should use, in whole seconds.
class TickSpec {
  /// Seconds between labelled (numbered) ticks.
  final double secondsPerLabel;

  /// Seconds between minor (unlabelled) ticks.
  final double secondsPerMinor;

  const TickSpec(this.secondsPerLabel, this.secondsPerMinor);
}

/// Choose a zoom-adaptive tick spacing so labels stay ~[minLabelPx] apart and
/// minor ticks never fall below [minMinorPx]. [pxPerSecond] is the displayed
/// scale (pxPerFrame × fps).
TickSpec chooseTicks(
  double pxPerSecond, {
  double minLabelPx = 56,
  double minMinorPx = 7,
}) {
  const steps = <double>[
    0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600,
  ];
  final pps = math.max(pxPerSecond, 1e-9);
  var label = steps.last;
  for (final s in steps) {
    if (s * pps >= minLabelPx) {
      label = s;
      break;
    }
  }
  // Subdivide the label step into minor ticks, backing off if they crowd.
  var minor = label / 5;
  if (minor * pps < minMinorPx) minor = label / 2;
  if (minor * pps < minMinorPx) minor = label;
  return TickSpec(label, minor);
}

/// Snap a dragged comp [frame] to the nearest whole second or marker, when
/// [snapping] is on and the candidate is within ~6 px of the cursor. Returns
/// [frame] unchanged when snapping is off or nothing is near.
int snapFrame(
  int frame, {
  required double fps,
  required List<int> markers,
  required bool snapping,
  required double pxPerFrame,
}) {
  if (!snapping || pxPerFrame <= 0) return frame;
  final threshold = 6.0 / pxPerFrame; // ~6 px, expressed in frames
  int? best;
  var bestDist = threshold;
  void consider(int candidate) {
    final d = (candidate - frame).abs().toDouble();
    if (d <= bestDist) {
      bestDist = d;
      best = candidate;
    }
  }

  if (fps > 0) {
    final second = (frame / fps).round();
    consider((second * fps).round());
  }
  for (final m in markers) {
    consider(m);
  }
  return best ?? frame;
}
