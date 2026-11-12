// The Grade panel grades the shot it shows: a picked Cut clip's own stack,
// never the Sequence layer the clip sits on.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/grade_panel_frb.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  testWidgets('a wheel drag on a picked clip grades that clip, as one step',
      (tester) async {
    final p = freshProject();
    final project = p.state.project!;
    final comp = project.newComposition(name: 'Scene');
    final seq = comp.addSequenceLayer();
    final footage = project.importFootage(path: 'C:/clips/shot.mov');
    seq.addClip(footage: footage, atFrame: 0, overlap: false);
    p.uiState
      ..setSelectedComp(comp)
      ..selectedLayer.value = seq;
    final clip = p.uiState.model.byId(seq.internallayerId)!.info.clips.single.id;
    p.uiState.cutClip.value = (layer: seq, clip: clip);

    const size = Size(480, 760);
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(hostPanel(
        child: const GradePanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: size));
    await tester.pump();

    await tester.drag(
        find.byKey(const ValueKey('grade-wheel-lift')), const Offset(40, 0));
    // Past the wheel's double-click window, so no timer outlives the test.
    await tester.pump(const Duration(milliseconds: 500));

    List<String> clipEffects() => [
          for (final e in seq.getClipEffects(clip: clip, onCompClock: false))
            e.name()
        ];
    expect(clipEffects(), ['correction'],
        reason: 'the first touch adds Correction 1 to the clip');
    expect(seq.getEffects(), isEmpty,
        reason: 'the Sequence layer itself is left alone');

    project.undo();
    expect(clipEffects(), isEmpty,
        reason: 'the drag and the node it added are one undo step');
  });
}
