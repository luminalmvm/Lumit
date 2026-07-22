// The two-row time ruler over the lane area (parity checklist post-parity item
// 2, an owner design change built into F3 from the start): the ruler band spans
// the full two-row height the egui version only half-used. Frame/second ticks
// with zoom-adaptive labels up top, minor ticks below, markers as small flags;
// a click or drag anywhere in the band scrubs the playhead.

import 'package:flutter/widgets.dart';

import '../../bridge/bridge.dart';
import '../../state/app_state.dart';
import '../../widgets/controls.dart';
import 'lane_scale.dart';

/// The ruler band. Sits right of the outline column; its own local x=0 maps to
/// the lane's [LaneScale.trackLeft].
class TimelineRuler extends StatelessWidget {
  final AppStateStub app;
  final LaneScale scale;
  final double fps;
  final List<int> markers;

  /// The markers with their kind (bridge v0.9). When present, beats are drawn
  /// distinctly from user/chapter markers (mirroring egui panel.rs:252-290);
  /// empty falls back to drawing every [markers] frame as a user marker.
  final List<BridgeMarker> markerDetails;
  final double height;

  const TimelineRuler({
    super.key,
    required this.app,
    required this.scale,
    required this.fps,
    required this.markers,
    this.markerDetails = const [],
    this.height = 36,
  });

  int _frameAt(double localX) {
    // Local x=0 is the lane left edge (trackLeft), so shift into scale space.
    final f = scale.frameOfX(localX + scale.trackLeft);
    return snapFrame(
      f.round().clamp(0, scale.frameCount),
      fps: fps,
      markers: markers,
      snapping: app.snapping,
      pxPerFrame: scale.pxPerFrame,
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTapDown: (d) => app.goToFrame(_frameAt(d.localPosition.dx)),
      onHorizontalDragStart: (d) => app.goToFrame(_frameAt(d.localPosition.dx)),
      onHorizontalDragUpdate: (d) => app.goToFrame(_frameAt(d.localPosition.dx)),
      child: CustomPaint(
        size: Size(scale.trackWidth, height),
        painter: _RulerPainter(
          scale: scale,
          fps: fps,
          markers: markers,
          markerDetails: markerDetails,
          surface: t.surface2,
          tick: t.hairlineStrong,
          label: t.textMuted,
          marker: t.accent,
          hairline: t.hairline,
        ),
      ),
    );
  }
}

class _RulerPainter extends CustomPainter {
  final LaneScale scale;
  final double fps;
  final List<int> markers;
  final List<BridgeMarker> markerDetails;
  final Color surface, tick, label, marker, hairline;

  _RulerPainter({
    required this.scale,
    required this.fps,
    required this.markers,
    required this.markerDetails,
    required this.surface,
    required this.tick,
    required this.label,
    required this.marker,
    required this.hairline,
  });

  // Convert a comp frame to this painter's local x (local 0 = trackLeft).
  double _lx(num frame) => scale.xOfFrame(frame) - scale.trackLeft;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = surface);
    final bottom = size.height;
    // Bottom hairline parting the ruler from the lanes.
    canvas.drawLine(
      Offset(0, bottom - 0.5),
      Offset(size.width, bottom - 0.5),
      Paint()
        ..color = hairline
        ..strokeWidth = 1,
    );

    if (fps <= 0) return;
    final pxPerSecond = scale.pxPerFrame * fps;
    final spec = chooseTicks(pxPerSecond);
    final durationSeconds = scale.frameCount / fps;
    final viewStartSeconds = scale.viewStartFrame / fps;
    final viewEndSeconds = viewStartSeconds + size.width / pxPerSecond;

    final tickPaint = Paint()
      ..color = tick
      ..strokeWidth = 1;

    // Minor ticks in the lower band.
    var s = (viewStartSeconds / spec.secondsPerMinor).floor() * spec.secondsPerMinor;
    while (s <= viewEndSeconds && s <= durationSeconds + 1e-6) {
      if (s >= -1e-6) {
        final x = _lx(s * fps);
        canvas.drawLine(Offset(x, bottom - 6), Offset(x, bottom), tickPaint);
      }
      s += spec.secondsPerMinor;
    }

    // Labelled ticks: taller, with the "Ns" label in the upper band.
    var ls = (viewStartSeconds / spec.secondsPerLabel).floor() * spec.secondsPerLabel;
    while (ls <= viewEndSeconds && ls <= durationSeconds + 1e-6) {
      if (ls >= -1e-6) {
        final x = _lx(ls * fps);
        canvas.drawLine(Offset(x, bottom - 10), Offset(x, bottom), tickPaint);
        final tp = TextPainter(
          text: TextSpan(
            text: '${_trim(ls)}s',
            style: TextStyle(
              color: label,
              fontSize: 9,
              fontFamily: 'monospace',
            ),
          ),
          textDirection: TextDirection.ltr,
        )..layout();
        tp.paint(canvas, Offset(x + 3, 3));
      }
      ls += spec.secondsPerLabel;
    }

    // Markers (mirroring egui panel.rs:252-290): a beat is a faint clay tick
    // fading by confidence, starting lower in the band; a user/chapter marker is
    // a full-height solid line with a small downward flag. When the kinded
    // read-back is absent (an older library), every bare [markers] frame draws
    // as a user marker.
    final flag = Paint()..color = marker;
    void drawUser(double x) {
      canvas.drawLine(
        Offset(x, 0),
        Offset(x, bottom),
        Paint()
          ..color = marker
          ..strokeWidth = 1,
      );
      final path = Path()
        ..moveTo(x, 0)
        ..lineTo(x + 6, 2.5)
        ..lineTo(x, 5)
        ..close();
      canvas.drawPath(path, flag);
    }

    void drawBeat(double x, double confidence) {
      // The egui beat tint: accent faded by 0.25 + 0.55·confidence, from a tick
      // that begins ~a quarter down the band (never a full-height alarm).
      final c = confidence.clamp(0.0, 1.0);
      canvas.drawLine(
        Offset(x, bottom * 0.25),
        Offset(x, bottom),
        Paint()
          ..color = marker.withValues(alpha: 0.25 + 0.55 * c)
          ..strokeWidth = 1,
      );
    }

    if (markerDetails.isNotEmpty) {
      for (final m in markerDetails) {
        final x = _lx(m.frame);
        if (x < -2 || x > size.width + 2) continue;
        if (m.isBeat) {
          drawBeat(x, m.confidence ?? 0.0);
        } else {
          drawUser(x);
        }
      }
    } else {
      for (final m in markers) {
        final x = _lx(m);
        if (x < -2 || x > size.width + 2) continue;
        drawUser(x);
      }
    }
  }

  /// Trim a whole-second label ("2s"), or keep one decimal for sub-second steps.
  String _trim(double seconds) {
    if ((seconds - seconds.roundToDouble()).abs() < 1e-6) {
      return seconds.round().toString();
    }
    return seconds.toStringAsFixed(1);
  }

  @override
  bool shouldRepaint(_RulerPainter old) =>
      old.scale.pxPerFrame != scale.pxPerFrame ||
      old.scale.viewStartFrame != scale.viewStartFrame ||
      old.scale.trackWidth != scale.trackWidth ||
      old.markers != markers ||
      old.markerDetails != markerDetails ||
      old.fps != fps;
}
