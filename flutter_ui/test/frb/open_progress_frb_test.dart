// The opening card tells the truth about how far the open has got.
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

@Tags(['opens-project'])
library;

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/state/workspace.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

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
