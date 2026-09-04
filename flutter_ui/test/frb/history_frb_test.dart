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
import 'package:lumit_flutter/l10n/engine_labels.dart';
import 'package:lumit_flutter/shell/history_dialog_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart' show BridgeScalar_Static;
import 'package:lumit_flutter/src/rust/api/layer.dart' show BridgeSpan;
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('the History list', () {
    test('names the edits that were made, in order', () {
      final p = freshProject();
      final project = p.state.project!;
      final comp = project.newComposition(name: 'Scene');
      comp.addSolidLayer();
      comp.addTextLayer();

      // A batch is named after the first thing it did, which is what
      // makes the solid's row read "Add item": making a solid layer starts by
      // adding the solid itself to the project, and the layer follows in the
      // same step.
      final rows = project.historyEntries();
      expect(rows.map((e) => e.name).toList(),
          <String>['Add item', 'Add item', 'Add layer']);
      expect(rows.every((e) => !e.undone), isTrue);
      expect(project.appliedSteps(), rows.length);
    });

    test('an undone step stays on the list, greyed, and comes back on redo',
        () {
      final p = freshProject();
      final project = p.state.project!;
      final comp = project.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final total = project.historyEntries().length;

      project.undo();
      final after = project.historyEntries();
      expect(after.length, total, reason: 'the row is still listed');
      expect(after.last.undone, isTrue);
      expect(project.appliedSteps(), total - 1);

      project.redo();
      expect(project.historyEntries().last.undone, isFalse);
      expect(project.appliedSteps(), total);
    });

    test('jumping to a row puts the document where that row says', () {
      final p = freshProject();
      final project = p.state.project!;
      final comp = project.newComposition(name: 'Scene');
      final atComp = project.appliedSteps();
      comp.addSolidLayer();
      comp.addSolidLayer();
      final atEnd = project.appliedSteps();
      expect(comp.getLayers().length, 2);

      project.jumpHistory(applied: atComp);
      expect(comp.getLayers(), isEmpty, reason: 'back to before either layer');
      expect(project.appliedSteps(), atComp);

      project.jumpHistory(applied: atEnd);
      expect(comp.getLayers().length, 2, reason: 'and forward again');

      // Past the end stops at the end rather than failing.
      project.jumpHistory(applied: atEnd + 50);
      expect(project.appliedSteps(), atEnd);
    });

    test('every name the engine can send is a name this build can translate',
        () {
      final p = freshProject();
      final project = p.state.project!;
      final comp = project.newComposition(name: 'Scene');
      comp.addSolidLayer().rename(name: 'Backdrop');
      comp.setWorkArea(
        span: BridgeSpan(
          inPoint: comp.timeOfFrame(frame: 10),
          outPoint: comp.timeOfFrame(frame: 20),
          startOffset: comp.timeOfFrame(frame: 0),
        ),
      );
      for (final row in project.historyEntries()) {
        expect(hasEngineLabel(row.name), isTrue,
            reason: '"${row.name}" has no entry in engine_labels.dart');
      }
    });
  }, skip: !engineAvailable);

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
    test('trim makes the comp its work area and one undo puts it back', () {
      final p = freshProject();
      final project = p.state.project!;
      final comp = project.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      final wasDuration = comp.durationFrames();
      final wasSpan = layer.getSpan();
      comp.setWorkArea(
        span: BridgeSpan(
          inPoint: comp.timeOfFrame(frame: 10),
          outPoint: comp.timeOfFrame(frame: 25),
          startOffset: comp.timeOfFrame(frame: 0),
        ),
      );

      comp.trimToWorkArea();
      expect(comp.durationFrames(), 15);
      expect(comp.getWorkArea(), isNull,
          reason: 'the trimmed comp is its own work area');
      expect(comp.frameAtTime(time: layer.getSpan().inPoint), -10,
          reason: 'the layer slid back by the work area start');

      project.undo();
      expect(comp.durationFrames(), wasDuration);
      expect(comp.frameAtTime(time: layer.getSpan().inPoint),
          comp.frameAtTime(time: wasSpan.inPoint));
      expect(comp.getWorkArea(), isNotNull,
          reason: 'the work area comes back, uncut by the shorter comp');
      expect(comp.frameAtTime(time: comp.getWorkArea()!.outPoint), 25);
    });

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

    test('a region smaller than a pixel is refused', () {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      expect(() => comp.cropToRegion(region: const [0.5, 0.5, 0.5, 0.5]),
          throwsA(anything));
      expect(() => comp.cropToRegion(region: const [0.1, 0.2]),
          throwsA(anything));
    });
  }, skip: !engineAvailable);
}
