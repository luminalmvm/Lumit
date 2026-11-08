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
  // The rows these tests reach live inside the Export window's folds.
  setUp(() => exportOpenSections.addAll(ExportSection.values));

  group('Export metrics (frb)', () {
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
