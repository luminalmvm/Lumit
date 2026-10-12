// The export dialog and the queue window measured against the approved
// drawing, band by band.
//
// **Why this file exists.** `shell_frb_test` is about what the dialog *does* —
// what a button reaches, what a field sets, what queueing an export leaves
// behind. This one is about what it *looks like*, and specifically about the
// numbers the drawing's own computed styles resolved to: the frame, the title
// strip, the page tabs, a group, a row, the controls in it, the footer.
//
// A value that disagrees with the drawing is a defect (§12A.6), so each
// expectation carries the drawing's own number in its reason.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/export_dialog_frb.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Export metrics (frb)', () {
    /// Open the dialog the way the application does, in a view large enough to
    /// hold every group at once.
    Future<void> open(WidgetTester tester, {double height = 1000}) async {
      tester.view.physicalSize = Size(1200, height);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Opening titles');
      comp.addAdjustmentLayer();
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-export'),
            onPressed: () => showExportDialogFrb(context: context, comp: comp),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
        size: Size(1200, height),
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-export')));
      await tester.pumpAndSettle();
    }

    /// The colour a group's own box is outlined in — hairline at rest, the
    /// accent while a tab is pointing at it.
    Color? groupBorder(WidgetTester tester, String name) {
      final box = tester.widget<Container>(find
          .descendant(
            of: find.byKey(ValueKey<String>('export-group-$name')),
            matching: find.byType(Container),
          )
          .first);
      return (box.decoration! as BoxDecoration).border?.top.color;
    }

    /// 6a. **Clicking a tab brings its section to the top of the body** and
    /// lights the box it landed on, so the eye knows where it was taken.
    testWidgets('a tab scrolls its section into view and lights it',
        (tester) async {
      // A window short enough that the last sections are genuinely off-screen.
      await open(tester, height: 520);

      final bodyTop =
          tester.getRect(find.byKey(const ValueKey('export-group-output'))).top;
      final before =
          tester.getRect(find.byKey(const ValueKey('export-group-metadata')));
      expect(before.top, greaterThan(bodyTop + 400),
          reason: 'Metadata is a long way down the page to begin with');

      await tester
          .tap(find.byKey(const ValueKey('export-tab-ExportSection.metadata')));
      await tester.pumpAndSettle();

      final after =
          tester.getRect(find.byKey(const ValueKey('export-group-metadata')));
      expect(after.top, lessThan(before.top),
          reason: 'the page scrolled to the section the tab names');
      // The box it landed on is lit for a moment, in the accent the tab strip
      // already uses to say "this one".
      final t = ThemeScope.of(tester
              .element(find.byKey(const ValueKey('export-group-metadata'))))
          .theme;
      expect(groupBorder(tester, 'metadata'), t.accent,
          reason: 'the section it jumped to is lit while you look for it');
      await tester.pump(exportSectionFlash);
      await tester.pumpAndSettle();
      expect(groupBorder(tester, 'metadata'), t.hairline,
          reason: 'and settles back to an ordinary group directly after');
    });

    /// 6b. **A short window scrolls rather than squishing** (§12A.6). An
    /// overflow is an error in a widget test, so opening the dialog in a window
    /// too short for its groups is the whole assertion.
    testWidgets('a window too short for the groups scrolls the body',
        (tester) async {
      tester.view.physicalSize = const Size(900, 420);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addAdjustmentLayer();
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-export'),
            onPressed: () => showExportDialogFrb(context: context, comp: comp),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(900, 420),
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-export')));
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('export-footer')), findsOneWidget,
          reason: 'the footer is never scrolled away');
      expect(find.byType(SingleChildScrollView), findsWidgets);

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

  }, skip: !engineAvailable);
}
