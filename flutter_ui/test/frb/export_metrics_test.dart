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
import 'package:lumit_flutter/shell/dialog_frame.dart';
import 'package:lumit_flutter/shell/export_dialog_frb.dart';
import 'package:lumit_flutter/shell/export_queue_frb.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Export metrics (frb)', () {
    /// Open the dialog the way the application does, in a view large enough to
    /// hold every group at once.
    Future<void> open(WidgetTester tester) async {
      tester.view.physicalSize = const Size(1200, 1000);
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
        size: const Size(1200, 1000),
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-export')));
      await tester.pumpAndSettle();
    }

    Rect band(WidgetTester tester, String key) =>
        tester.getRect(find.byKey(ValueKey<String>(key)));

    /// 1. **The frame.** 640 wide, a 30px title strip over its hairline, a
    /// 26px tab row over its own, and a 45px footer at the bottom.
    testWidgets('the dialog is the drawing\'s frame', (tester) async {
      await open(tester);

      final title = band(tester, 'export-title-strip');
      final tabs = band(tester, 'export-tabs');
      final footer = band(tester, 'export-footer');
      expect(title.width, exportDialogWidth,
          reason: 'the drawing frames the dialog at 640 wide');
      expect(title.height, dialogTitleStrip + 1,
          reason: '§12A.4: a dialog title strip is 30, over a hairline');
      expect(tabs.height, dialogTabRow + 1,
          reason: "the drawing's page-tab row is 26, over a hairline");
      expect(tabs.top, title.bottom,
          reason: 'the tabs sit directly under the title strip');
      expect(footer.height, dialogFooterHeight,
          reason: '10 above a 24px button and 10 below it, over a hairline');
      expect(footer.width, exportDialogWidth);
    });

    /// 2. **The title strip** names the dialog and the composition it is
    /// about, and carries the way out.
    testWidgets('the title strip names the composition', (tester) async {
      await open(tester);
      expect(find.text('EXPORT'), findsWidgets,
          reason: 'the title is a kicker — mono capitals');
      expect(find.text('Opening titles'), findsOneWidget,
          reason: "the composition's own name, beside the kicker");
      expect(find.byKey(const ValueKey('export-close')), findsOneWidget);
    });

    /// 3. **A group** is a hairline box inset 14 from the dialog's edges —
    /// the drawing computes 612 inside a 640 frame — with its kicker notched
    /// into the top edge.
    testWidgets('a group is the drawing\'s box', (tester) async {
      await open(tester);

      final frame = band(tester, 'export-title-strip');
      final output = band(tester, 'export-group-output');
      expect(output.left - frame.left, dialogPadding,
          reason: 'the body insets 14 from the frame');
      expect(output.width, exportDialogWidth - dialogPadding * 2,
          reason: 'the drawing measures a group at 612 in a 640 frame');

      // The air between two groups: 10 of gap, and the 8 each group holds
      // above itself for the kicker on its edge.
      final composition = band(tester, 'export-group-composition');
      expect(composition.top - output.bottom, dialogGroupGap,
          reason: 'the drawing sets 10 between groups');
    });

    /// 4. **A row** is a label in a fixed 100px column, 10 after it, and 28
    /// tall — the Export drawing's own, not the New composition dialog's
    /// (K-458: each drawing measures itself).
    testWidgets('a row is the drawing\'s row', (tester) async {
      await open(tester);

      final label = tester.getRect(find
          .ancestor(of: find.text('Type'), matching: find.byType(SizedBox))
          .first);
      expect(label.width, exportLabelColumn,
          reason: "the drawing's label column is 100");

      final control = band(tester, 'export-format');
      expect(control.height, dialogControlHeight,
          reason: '§12A.6: a dropdown in a dialog is 22');
      expect(control.left - label.right, exportRowGap,
          reason: 'the control stands 10 after the label column');
    });

    /// 5. **The footer** carries the factual line and the two actions, the
    /// filled one last and both 24 tall.
    testWidgets('the footer states the facts and the actions', (tester) async {
      await open(tester);

      final summary = band(tester, 'export-summary');
      final add = band(tester, 'export-add-to-queue');
      final export = band(tester, 'export-start');
      expect(summary.left, lessThan(add.left),
          reason: 'the summary reads first, the actions sit at the far end');
      expect(add.height, dialogFooterButton,
          reason: "the drawing's footer buttons are 24 tall");
      expect(export.height, dialogFooterButton);
      expect(export.left, greaterThan(add.right),
          reason: 'the single filled action is last (§12A.4)');
      expect(find.text('EXPORT'), findsWidgets,
          reason: "the filled action's label is a kicker (K-450)");
    });

    /// 6. **The tabs.** The drawing's row of pages, with Output in force.
    /// Colour and Metadata are not here: there is nothing to put on them yet,
    /// and an empty page is a promise the dialog cannot keep (K-465).
    testWidgets('the page tabs front one group at a time', (tester) async {
      await open(tester);

      expect(find.byKey(const ValueKey('export-tab-ExportPage.output')),
          findsOneWidget);
      expect(find.byKey(const ValueKey('export-group-audio')), findsOneWidget,
          reason: 'the Output page holds every group, as the drawing draws it');

      await tester
          .tap(find.byKey(const ValueKey('export-tab-ExportPage.time')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('export-group-time')), findsOneWidget);
      expect(find.byKey(const ValueKey('export-group-output')), findsNothing,
          reason: 'a page other than Output fronts the group it names');
    });

    /// 7. **The queue window** is the same pattern at its own width: the
    /// dialog's title strip and footer, unchanged (K-444).
    testWidgets('the queue window wears the dialog pattern', (tester) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-queue'),
            onPressed: () => showExportQueueFrb(
              context: context,
              list: () => const [],
            ),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1200, 900),
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-queue')));
      await tester.pumpAndSettle();

      expect(band(tester, 'export-queue-title-strip').height,
          dialogTitleStrip + 1);
      expect(band(tester, 'export-queue-footer').height, dialogFooterHeight);
      expect(band(tester, 'export-queue-title-strip').width, exportQueueWidth);
      expect(find.byKey(const ValueKey('export-queue-empty')), findsOneWidget,
          reason: 'an empty queue says so rather than showing a blank list');

      await tester.tap(find.byKey(const ValueKey('export-queue-dismiss')));
      await tester.pumpAndSettle();
    });
  }, skip: !engineAvailable);
}
