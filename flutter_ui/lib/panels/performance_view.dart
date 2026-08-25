// `FramePhase` names the moment in a finished frame to read a timestamp from;
// `package:flutter/scheduler.dart` re-exports `FrameTiming` but not it.
import 'dart:ui' show FramePhase;

import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:lumit_flutter/widgets/controls.dart';

/// The frame-rate readout in the Debug View.
///
/// **It watches frames; it never asks for them.** The first version measured
/// the rate by registering a post-frame callback and calling `setState` from
/// it, then registering another. That works, but it schedules the next frame
/// from inside the current one, so the interface never goes quiet: it drew at
/// full rate for ever, whatever the editor was doing, and the meter became a
/// large part of what it was measuring. It also hung every widget test that
/// waits for the interface to settle, because settling is exactly the state it
/// made impossible.
///
/// The engine already reports what each frame cost, after the fact, through
/// [SchedulerBinding.addTimingsCallback]. Reading that costs nothing and
/// schedules nothing: when the editor is busy the numbers flow, and when
/// nothing is moving there are no frames, no callbacks, and no readout to
/// refresh — which is the honest answer rather than a made-up one.
class PerformanceMonitor extends StatefulWidget {
  const PerformanceMonitor({super.key});

  @override
  State<PerformanceMonitor> createState() => _PerformanceMonitorState();
}

/// How often the numbers on screen are refreshed. Wall-clock, and deliberately
/// far slower than the frames it reports: a readout redrawn per frame is one
/// more thing drawing per frame. Every frame is still *measured* — only the
/// drawing of the answer is paced.
const Duration _refreshEvery = Duration(milliseconds: 200);

/// How many recent frames the average and the strip cover — a second of them
/// at 60 fps.
const int _maxFrames = 60;

class _PerformanceMonitorState extends State<PerformanceMonitor> {
  /// Recent frame intervals in milliseconds, oldest first.
  List<double> timings = [];

  double fps = 0.0;
  double frameTime = 0.0;
  double average = 0.0;
  double averageFrameTime = 0;

  /// When the previous reported frame began, for the interval between them.
  Duration? previousVsync;

  /// When the numbers were last redrawn, for [_refreshEvery].
  DateTime lastRefresh = DateTime.fromMillisecondsSinceEpoch(0);

  @override
  void initState() {
    super.initState();
    SchedulerBinding.instance.addTimingsCallback(onTimings);
  }

  @override
  void dispose() {
    // Registered on the binding, which outlives this panel: left in place it
    // would go on being called for a widget that is no longer there.
    SchedulerBinding.instance.removeTimingsCallback(onTimings);
    super.dispose();
  }

  /// One batch of finished frames, as the engine reports them.
  void onTimings(List<FrameTiming> frames) {
    if (!mounted) {
      return;
    }
    for (final frame in frames) {
      final started = Duration(
        microseconds: frame.timestampInMicroseconds(FramePhase.vsyncStart),
      );
      final previous = previousVsync;
      previousVsync = started;
      // The first frame of a run has nothing to be measured against, and a
      // batch out of order (or one repeated) is not an interval.
      if (previous == null || started <= previous) {
        continue;
      }
      timings.add((started - previous).inMicroseconds / 1000);
    }
    if (timings.length > _maxFrames) {
      timings = timings.sublist(timings.length - _maxFrames);
    }

    final now = DateTime.now();
    if (timings.isEmpty || now.difference(lastRefresh) < _refreshEvery) {
      return;
    }
    lastRefresh = now;
    final last = timings.last;
    final mean = timings.reduce((a, b) => a + b) / timings.length;
    setState(() {
      frameTime = last;
      fps = last > 0 ? 1000 / last : 0;
      averageFrameTime = mean;
      average = mean > 0 ? 1000 / mean : 0;
    });
  }

  @override
  Widget build(BuildContext context) {
    final theme = ThemeScope.of(context).theme;

    return Column(
      children: [
        Row(
          spacing: 10,
          children: [
            Row(
              children: [
                Text(
                  "FPS: ",
                  style: theme.mono,
                ),
                Text(
                  fps.toStringAsFixed(0),
                  style: theme.mono.copyWith(color: msToColor(frameTime)),
                ),
              ],
            ),
            Row(
              children: [
                Text(
                  "Avg: ",
                  style: theme.mono,
                ),
                Text(
                  average.toStringAsFixed(0),
                  style:
                      theme.mono.copyWith(color: msToColor(averageFrameTime)),
                ),
              ],
            )
          ],
        ),
        SizedBox(
          height: 50,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            crossAxisAlignment: CrossAxisAlignment.end,
            children: timings
                .map((ms) => Container(
                    color: msToColor(ms),
                    child: SizedBox(width: 3, height: (ms * 0.5).clamp(0, 50))))
                .toList(),
          ),
        )
      ],
    );
  }

  Color msToColor(double ms) {
    final theme = ThemeScope.of(context).theme;

    if (ms > 30) {
      return theme.error;
    }

    if (ms > 17) {
      return theme.warning;
    }

    return theme.textMuted;
  }
}
