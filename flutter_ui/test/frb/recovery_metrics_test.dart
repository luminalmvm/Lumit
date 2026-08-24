// The recovery dialogue measured against the pattern the Export and New
// composition drawings share (K-444, K-469).
//
// **Why this file exists.** `shell_frb_test`'s Recovery group is about what the
// dialogue *does* — what it offers, what each choice reaches. This one is about
// what it *looks like*: the frame, the kicker title strip with the project's
// name beside it, the label-left rows, and the footer carrying the factual
// line, the outlined way out and the single filled action.
//
// There is no drawing of this dialogue's own, so what is pinned here is the
// pattern rather than a measured mockup: a value that disagrees with
// `dialog_frame.dart` is a defect (§12A.6).

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/lumit_icon.dart' as glyph;
import 'package:lumit_flutter/icons/lumit_icons.dart';
import 'package:lumit_flutter/shell/dialog_frame.dart';
import 'package:lumit_flutter/shell/recovery_dialog_frb.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Recovery metrics (frb)', () {
    /// Open the dialogue the way the application does, on a project that has a
    /// real autosave beside it — written by the engine, so the listing and the
    /// footer's count are genuine.
    Future<void> open(WidgetTester tester) async {
      tester.view.physicalSize = const Size(1000, 700);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      p.state.project!.newComposition(name: 'Scene');
      final dir = Directory.systemTemp.createTempSync('lumit-recover-metrics');
      final path = '${dir.path}/Northern lights.lum';
      // A real autosave beside the project, so the count in the footer and the
      // file name in the autosave row are the engine's own.
      p.state.project!.autosave(projectPath: path, keep: 3);

      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-recover'),
            onPressed: () => showRecoveryDialogFrb(
              context: context,
              state: p.state,
              projectPath: path,
            ),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1000, 700),
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-recover')));
      await tester.pumpAndSettle();
    }

    Rect band(WidgetTester tester, String key) =>
        tester.getRect(find.byKey(ValueKey<String>(key)));

    /// 1. **The frame.** The dialogue's own width, a 30px title strip over its
    /// hairline, and a 45px footer at the bottom.
    testWidgets('the dialogue is the pattern\'s frame', (tester) async {
      await open(tester);

      final title = band(tester, 'recover-title-strip');
      final footer = band(tester, 'recover-footer');
      expect(title.width, recoveryDialogWidth,
          reason: 'the dialogue frames itself at 520 (K-458: its own number)');
      expect(title.height, dialogTitleStrip + 1,
          reason: '§12A.4: a dialog title strip is 30, over a hairline');
      expect(footer.height, dialogFooterHeight,
          reason: '10 above a 24px button and 10 below it, over a hairline');
      expect(footer.width, recoveryDialogWidth);
    });

    /// 2. **The title strip** names the dialogue as a kicker, carries the
    /// project it is about beside it, and holds the way out.
    testWidgets('the title strip names the project', (tester) async {
      await open(tester);

      expect(find.text('RECOVER UNSAVED WORK'), findsOneWidget,
          reason: 'the title is a kicker — capitals are the style');
      expect(find.text('Northern lights.lum'), findsOneWidget,
          reason: "the project's own file name, beside the kicker");
      expect(find.byKey(const ValueKey('recover-close')), findsOneWidget);
    });

    /// 3. **A row** is a name in a fixed 160px column with 12 after it — the
    /// dialogue's own row, not the Export drawing's (K-458).
    testWidgets('a source row is the pattern\'s row', (tester) async {
      await open(tester);

      final label = tester.getRect(find
          .ancestor(
              of: find.text('Replay the edit journal'),
              matching: find.byType(SizedBox))
          .first);
      expect(label.width, recoveryLabelColumn,
          reason: "the dialogue's name column is 160");

      final help = tester
          .getRect(find.text('Everything up to the moment the session ended.'));
      expect(help.left - label.right, greaterThanOrEqualTo(recoveryRowGap),
          reason: 'the sentence stands 12 after the name column');

      // The autosave row reports which copy "the newest" is — a fact about
      // that choice, in the row offering it.
      expect(find.textContaining('.autosave'), findsOneWidget,
          reason: "the newest autosave's own file name, under its sentence");
    });

    /// 4. **The footer** states the fact and carries the two actions, the
    /// single filled one last and both 24 tall (§12A.4).
    testWidgets('the footer states the facts and the actions', (tester) async {
      await open(tester);

      final summary = band(tester, 'recover-summary');
      final discard = band(tester, 'recover-discard');
      final recover = band(tester, 'recover-apply');
      expect(find.textContaining('1 autosave'), findsOneWidget,
          reason: 'the factual line counts what was found beside the project');
      expect(summary.left, lessThan(discard.left),
          reason: 'the fact reads first, the actions sit at the far end');
      expect(discard.height, dialogFooterButton,
          reason: 'a footer button is 24 tall');
      expect(recover.height, dialogFooterButton);
      expect(recover.left, greaterThan(discard.right),
          reason: 'the single filled action is last (§12A.4)');
    });

    /// 5. **The frame is the shared one.** No `FloatSurface` popup any more:
    /// the dialogue is built from `DialogFrame`, so a change to the pattern
    /// reaches it without a second edit.
    testWidgets('the dialogue wears the shared frame', (tester) async {
      await open(tester);
      expect(find.byType(DialogFrame), findsOneWidget);
    });

    /// 6. **The picked source says so**, and picking the other moves the mark
    /// — the body is a choice, the footer is the act.
    testWidgets('the journal opens picked, and the choice moves',
        (tester) async {
      await open(tester);

      MenuRow row(String key) =>
          tester.widget<MenuRow>(find.byKey(ValueKey<String>(key)));
      expect(row('recover-journal').selected, isTrue,
          reason: 'the journal loses nothing, so it is what opens picked');
      expect(row('recover-autosave').selected, isFalse);

      await tester.tap(find.byKey(const ValueKey('recover-autosave')));
      await tester.pumpAndSettle();
      expect(row('recover-autosave').selected, isTrue);
      expect(row('recover-journal').selected, isFalse,
          reason: 'one source at a time — the mark moves, it does not add');

      await tester.tap(find.byKey(const ValueKey('recover-close')));
      await tester.pumpAndSettle();
    });

    /// 7. **The close mark is the set's**, at the size the pattern draws it.
    testWidgets('the close mark is the set\'s glyph', (tester) async {
      await open(tester);

      final mark = tester.widget<glyph.LumitIcon>(find
          .descendant(
            of: find.byKey(const ValueKey('recover-close')),
            matching: find.byType(glyph.LumitIcon),
          )
          .first);
      expect(mark.glyph, LumitIcons.close);
      expect(mark.size, dialogCloseGlyph);
    });
  }, skip: !engineAvailable);
}
