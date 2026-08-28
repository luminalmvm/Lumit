// The opening card tells the truth about how far the open has got (K-628).
//
// The bar used to sweep, because nothing reported anything: it said "working"
// and nothing else, for however many seconds a project full of precomps took.
// The engine now names each phase of the read as it begins and says what share
// of the whole open sits behind it, and the frontend closes the last stretch —
// the render worker starting and answering — at one.
//
// Two promises are worth holding to, and both are here: the engine's own
// report only ever rises and never claims the frame it has not made, and the
// card reaches its end before it comes down. A bar that went backwards would
// read as work being undone; one that vanished at eighty per cent would read
// as an open that gave up.
//
// `openProject` clears the engine's project registry, so this file stands
// alone, exactly as `session_restore_frb_test.dart` does.

import 'dart:io';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/state/workspace.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  testWidgets('the engine reports the open phase by phase', (tester) async {
    final dir = Directory.systemTemp.createTempSync('lumit-open-progress');
    final path = '${dir.path}/progress.lum';

    final state = LumitState()..newProject();
    LumitUiState(state, workspace: Workspace());
    final project = state.project!;
    project.newComposition(name: 'Scene').addSolidLayer();
    project.save(path: path);
    await settleFrb(tester, until: () => File(path).existsSync());
    expect(File(path).existsSync(), isTrue, reason: 'nothing to reopen');

    // The engine's half, read straight off the stream rather than through the
    // card: a project this small opens in one turn of the event loop, so what
    // the card would have *drawn* is a question of timing, and what the engine
    // *said* is not.
    //
    // The sink is listened to after the call has been started, not before: it
    // has no stream until it has been handed to one, and nothing is lost in
    // between because the stream buffers.
    final sink = RustStreamSink<OpenProgress>();
    final pending =
        LumitBridgeState.openProject(path: path, onProgressStream: sink);
    final reports = <OpenProgress>[];
    final watching = sink.stream.listen(reports.add);
    await settleFrb(tester,
        until: () => reports.length >= OpenPhase.values.length);
    expect(await pending, isNotNull, reason: 'the project would not open');
    await watching.cancel();

    expect(reports.map((r) => r.phase), OpenPhase.values,
        reason: 'every phase of the read is named, in the order it happens');
    for (var i = 1; i < reports.length; i++) {
      expect(reports[i].fraction, greaterThan(reports[i - 1].fraction),
          reason: 'the fill went backwards at report $i');
    }
    expect(reports.first.fraction, 0,
        reason: 'the card starts empty rather than part-filled');
    expect(reports.last.fraction, lessThan(1),
        reason: 'the engine never claims the frame it has not made');

    dir.deleteSync(recursive: true);
  });

  testWidgets('the card fills to the end before it comes down', (tester) async {
    final dir = Directory.systemTemp.createTempSync('lumit-open-done');
    final path = '${dir.path}/done.lum';

    final state = LumitState()..newProject();
    LumitUiState(state, workspace: Workspace());
    state.project!.newComposition(name: 'Scene').addSolidLayer();
    state.project!.save(path: path);
    await settleFrb(tester, until: () => File(path).existsSync());

    // Not awaited, for `session_restore_frb_test`'s reason: the continuation of
    // an async frb call only lands on the event-loop turns settleFrb provides.
    final adopted = state.project;
    state.openProject(path);
    expect(state.opening.value, isTrue);
    expect(state.openProgress.value, isNotNull,
        reason: 'the card is determinate from its first frame, not a sweep '
            'that turns into a bar a moment later');
    expect(state.openProgress.value!.fraction, 0);

    await settleFrb(tester, until: () => !identical(state.project, adopted));

    // No Viewer is mounted in a widget test, so no frame is ever served —
    // `previewReady` is the frontend saying the last stretch is done, whether
    // that came from a picture or from a project with no picture to wait for.
    state.previewReady();
    expect(state.openProgress.value!.fraction, 1,
        reason: 'the bar reaches its end before the card goes');
    expect(state.opening.value, isFalse, reason: 'and then the card goes');

    dir.deleteSync(recursive: true);
  });
}
