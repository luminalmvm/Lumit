// Every-frame footage playback, measured on a REAL window.
//
// The question is a number: how many frames per second does every-frame mode
// sustain with a single 1080p60 footage layer? Widget tests cannot answer it —
// their clock is fake and their async is spliced — so this drives the real
// pipeline end to end: decode, composite, read-back, bridge, display.
//
// Run:  flutter test integration_test/playback_bench_test.dart -d windows
// Needs C:/tmp/test1080p60.mp4 (ffmpeg testsrc2, 1920x1080, 60 fps, 10 s).

import 'dart:async';
import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/frb_generated.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/state/workspace.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('every-frame playback rate with one footage layer',
      (tester) async {
    const path = 'C:/tmp/test1080p60.mp4';
    if (!File(path).existsSync()) {
      markTestSkipped('no test footage at $path');
      return;
    }

    await BridgeLib.init();
    final state = LumitState()..newProject();
    final ui = LumitUiState(state, workspace: Workspace());
    ui.workspace.performance.playback = PlaybackMode.everyFrame;

    final comp = state.project!.newComposition(name: 'Bench');
    final footage = state.project!.importFootage(path: path);
    comp.addFootageLayer(footage: footage, asSequence: false);
    ui.setSelectedComp(comp);

    await tester.pumpWidget(LumitAppNew(state, ui));
    await tester.pumpAndSettle();

    // Let the first frame land (decoder spin-up, renderer build).
    ui.playheadFrame.value = 0;
    for (var i = 0; i < 100 && ui.viewerFrameid.value == null; i++) {
      await tester.pump(const Duration(milliseconds: 50));
      await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 40)));
    }
    debugPrint('DDD warm textureId=${ui.viewerFrameid.value}');

    // Play. Every-frame mode advances the playhead per delivered frame, so the
    // wall time for N frames IS the pipeline's sustained rate. The whole wait
    // lives inside one runAsync so the real event loop free-runs: a pump-slice
    // loop out here would throttle delivery to its own granularity and measure
    // itself rather than the pipeline.
    const target = 240;
    final sw = Stopwatch();
    await tester.runAsync(() async {
      final done = Completer<void>();
      void check() {
        if (ui.playheadFrame.value >= target && !done.isCompleted) {
          done.complete();
        }
      }

      ui.playheadFrame.addListener(check);
      ui.requestTogglePlay();
      sw.start();
      await done.future.timeout(const Duration(seconds: 60), onTimeout: () {});
      sw.stop();
      ui.playheadFrame.removeListener(check);
      ui.requestTogglePlay();
    });

    final frames = ui.playheadFrame.value;
    final fps = frames / (sw.elapsedMilliseconds / 1000.0);
    debugPrint('DDD frames=$frames in ${sw.elapsedMilliseconds}ms '
        '= ${fps.toStringAsFixed(1)} fps');
  });
}
