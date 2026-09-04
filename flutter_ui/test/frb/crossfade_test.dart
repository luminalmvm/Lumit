// Crossfade handles on Sequence clip joins (the AudioWorkspace
// board): where two clips overlap, the join draws the opposed-fades pair
// with a handle at either end, and dragging a handle adjusts the fade by
// trimming the clip whose ramp it is.
//
// Against the real engine: the overlap IS the crossfade — the mixer ramps
// across exactly this span — so the test asserts the document's clips moved,
// not just the picture.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  testWidgets(
      'an overlapped join wears the opposed fades, and its handle trims the fade',
      (tester) async {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Montage');
    p.uiState.setSelectedComp(comp);
    final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
    comp.addFootageLayer(footage: footage, asSequence: false);
    final layer = comp.getLayers().first..convertToSequenced();
    layer.cutClipAt(frame: 40);
    p.uiState.model.refresh();

    tester.view.physicalSize = const Size(1280, 600);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(hostPanel(
      child: const TimelinePanelFrb(),
      state: p.state,
      uiState: p.uiState,
      size: const Size(1280, 600),
    ));
    await tester.pump();

    // Open the sequence view: double-click the layer's name.
    final name =
        find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}'));
    await tester.tap(name);
    await tester.pump(kDoubleTapMinTime);
    await tester.tap(name);
    await tester.pumpAndSettle();

    var clips = layer.getClips()
      ..sort((a, b) => a.startFrame.compareTo(b.startFrame));
    final outgoing = clips[0];
    final incoming = clips[1];

    // A butt cut has no overlap, so no ramps — a hard cut on the beat.
    expect(find.byKey(ValueKey<String>('seq-crossfade-${outgoing.id}')),
        findsNothing,
        reason: 'a butt cut gets no crossfade');

    // Trim the incoming clip's start back over the join: the overlap is the
    // crossfade, so the join now wears the pair and its two handles.
    layer.trimClip(
      clip: incoming.id,
      startFrame: incoming.startFrame - 10,
      endFrame: incoming.endFrame,
    );
    p.uiState.model.refresh();
    await tester.pump();

    expect(find.byKey(ValueKey<String>('seq-crossfade-${outgoing.id}')),
        findsOneWidget,
        reason: 'an overlap is a crossfade, drawn as the opposed pair');
    expect(find.byKey(ValueKey<String>('seq-fade-in-${incoming.id}')),
        findsOneWidget);
    final outHandle =
        find.byKey(ValueKey<String>('seq-fade-out-${outgoing.id}'));
    expect(outHandle, findsOneWidget);

    // Drag the outgoing end's handle right: the outgoing clip's tail grows,
    // which is the fade getting longer.
    final endBefore = layer
        .getClips()
        .firstWhere((c) => c.id == outgoing.id)
        .endFrame
        .toInt();
    final perFrame = (tester
                .getRect(find.byKey(const ValueKey('tl-ruler')))
                .width -
            12) /
        comp.durationFrames();
    final gesture = await tester.startGesture(
      tester.getCenter(outHandle),
      kind: PointerDeviceKind.mouse,
    );
    await tester.pump(const Duration(milliseconds: 60));
    final travel = (perFrame * 8).clamp(24.0, 200.0);
    for (var i = 0; i < 8; i++) {
      await gesture.moveBy(Offset(travel / 8, 0));
      await tester.pump();
    }
    await gesture.up();
    await tester.pumpAndSettle();

    final endAfter = layer
        .getClips()
        .firstWhere((c) => c.id == outgoing.id)
        .endFrame
        .toInt();
    expect(endAfter, greaterThan(endBefore),
        reason: 'dragging the fade handle trimmed the outgoing end later');

    // And the widened overlap still draws as one crossfade.
    clips = layer.getClips()
      ..sort((a, b) => a.startFrame.compareTo(b.startFrame));
    expect(clips[0].endFrame.toInt(),
        greaterThan(clips[1].startFrame.toInt()),
        reason: 'the join is still an overlap');
    expect(find.byKey(ValueKey<String>('seq-crossfade-${outgoing.id}')),
        findsOneWidget);
  });
}
