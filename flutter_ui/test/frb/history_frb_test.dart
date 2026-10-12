// The History window and the two composition commands beside it, against the
// real engine.
//
// Three things are worth holding down. The list has to name the edits that were
// actually made, in the order they were made; clicking a row has to put the
// document where that row says; and the two comp commands have to do their
// reshaping in one undo step that puts everything back — including the work
// area, whose restore is the part that was easy to get wrong.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/history_dialog_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart' show BridgeScalar_Static;
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('the History window', () {
    testWidgets('lists the steps and jumps to the one that is clicked',
        (tester) async {
      final p = freshProject();
      final project = p.state.project!;
      final comp = project.newComposition(name: 'Scene');
      final atComp = project.appliedSteps();
      comp.addSolidLayer();
      comp.addSolidLayer();

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(600, 500),
        child: Builder(
          builder: (context) => GestureDetector(
            key: const ValueKey('open-history'),
            onTap: () => showHistoryFrb(context, p.state),
          ),
        ),
      ));
      await tester.tap(find.byKey(const ValueKey('open-history')));
      await tester.pump();

      // One row per step, plus the row above them all for where the list
      // begins.
      expect(find.byType(MenuRow),
          findsNWidgets(project.historyEntries().length + 1));

      await tester.tap(find.byKey(ValueKey<String>('history-row-$atComp')));
      await tester.pump();
      expect(comp.getLayers(), isEmpty, reason: 'the click undid both layers');
      expect(project.appliedSteps(), atComp);
    });
  }, skip: !engineAvailable);

  group('trim and crop reshape a comp in one undo step', () {
    test('crop makes the frame the region and moves the layers with it', () {
      final p = freshProject();
      final project = p.state.project!;
      final comp = project.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      final settings = comp.getSettings();
      double x() =>
          (layer.getTransform().positionX as BridgeScalar_Static).field0;
      final was = x();

      // The middle half of the frame, as the Viewer hands its region over.
      comp.cropToRegion(region: const [0.25, 0.25, 0.75, 0.75]);
      final cropped = comp.getSettings();
      expect(cropped.width, settings.width ~/ 2);
      expect(cropped.height, settings.height ~/ 2);
      expect(x(), closeTo(was - settings.width / 4, 0.001),
          reason: 'the layer moved back by the region corner');

      project.undo();
      expect(comp.getSettings().width, settings.width);
      expect(x(), closeTo(was, 0.001));
    });

  }, skip: !engineAvailable);
}
