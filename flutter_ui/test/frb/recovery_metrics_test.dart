// The recovery dialogue measured against the pattern the Export and New
// composition drawings share (K-444, K-469, K-487).
//
// **Why this file exists.** `shell_frb_test`'s Recovery group is about what the
// dialogue *does* — what each button answers, what it reaches. This one is
// about what it *looks like*: the narrow frame, the kicker title strip, the one
// sentence, and the three stacked buttons under it.
//
// There is no drawing of this dialogue's own, so what is pinned here is the
// pattern rather than a measured mockup: a value that disagrees with
// `dialog_frame.dart` is a defect (§12A.6).
//
// **What this file cannot measure.** A widget test draws in the test font,
// where every glyph is a square of the type size — so a label's width here is
// roughly double what Hanken Grotesk renders, and pinning a *text* width from
// this file would pin a fiction. The width in `recovery_dialog_frb.dart` is
// derived from the real font's advance widths instead; what is checked here is
// the consequence that survives either font: three full-width buttons, one
// above the next, none of them clipped.

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
    /// real autosave beside it — written by the engine, so there is genuinely
    /// something to recover.
    Future<void> open(WidgetTester tester) async {
      tester.view.physicalSize = const Size(1000, 700);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      p.state.project!.newComposition(name: 'Scene');
      final dir = Directory.systemTemp.createTempSync('lumit-recover-metrics');
      final path = '${dir.path}/Northern lights.lum';
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
          reason: 'the dialogue frames itself at 350 — two-thirds of the 520 '
              'it was (K-487)');
      expect(title.height, dialogTitleStrip + 1,
          reason: '§12A.4: a dialog title strip is 30, over a hairline');
      expect(
          footer.height,
          dialogFooterPad * 2 +
              dialogFooterButton * 3 +
              dialogFooterStackGap * 2 +
              1,
          reason: '10 above, three 24px buttons 8 apart, 10 below, over the '
              'footer\'s own hairline (K-487)');
      expect(footer.width, recoveryDialogWidth);
    });

    /// 2. **The title strip** names the dialogue as a kicker and holds the way
    /// out. Nothing else: the narrow face carries no subject beside it.
    testWidgets('the title strip is the kicker and the way out',
        (tester) async {
      await open(tester);

      expect(find.text('RECOVER WORK'), findsOneWidget,
          reason: 'the title is a kicker — capitals are the style');
      expect(find.byKey(const ValueKey('recover-close')), findsOneWidget);
    });

    /// 3. **The body is one sentence** — the owner's own words, asking a
    /// question and punctuated as one. No rows, no source picker: the buttons
    /// are the choice (K-487).
    testWidgets('the body is the question and nothing else', (tester) async {
      await open(tester);

      expect(
          find.text('It looks like Lumit crashed with unsaved changes, would '
              'you like to restore them?'),
          findsOneWidget);
      expect(find.byType(MenuRow), findsNothing,
          reason: 'the source-choice rows went with K-487');
      expect(find.byKey(const ValueKey('recover-summary')), findsNothing,
          reason: 'three buttons take the footer; the count went to nothing');
    });

    /// 4. **The footer carries the three answers in the owner's order**, one
    /// above the next, each 24 tall and each at the footer's full width — the
    /// ladder's step 2 rather than three elided words (§12A.6, K-487). The
    /// filled one is last, which in a column means the bottom.
    testWidgets('the three buttons stack in order', (tester) async {
      await open(tester);

      final footer = band(tester, 'recover-footer');
      final none = band(tester, 'recover-discard');
      final autosave = band(tester, 'recover-autosave');
      final all = band(tester, 'recover-journal');

      for (final button in [none, autosave, all]) {
        expect(button.height, dialogFooterButton,
            reason: 'a footer button is 24 tall, stacked or not');
        expect(button.width, footer.width - dialogPadding * 2,
            reason: "a stacked action takes the footer's full width, so no "
                'label can be clipped whatever it is translated to');
      }
      expect(autosave.top - none.bottom, dialogFooterStackGap,
          reason: "the owner's order, 8 apart: don't restore, then autosave");
      expect(all.top - autosave.bottom, dialogFooterStackGap,
          reason: 'and the single filled action last — the bottom (§12A.4)');
    });

    /// 5. **The sentence sets in the narrow face.** Two lines at 350, in the
    /// body's own 14px inset, with the buttons clear underneath it.
    testWidgets('the question sets above the buttons', (tester) async {
      await open(tester);

      final frame = band(tester, 'recover-title-strip');
      final question = tester.getRect(find.textContaining('It looks like'));
      expect(question.left - frame.left, dialogPadding,
          reason: 'the body insets 14 from the frame');
      expect(question.width,
          lessThanOrEqualTo(recoveryDialogWidth - dialogPadding * 2),
          reason: 'and never paints outside it');
      expect(question.bottom, lessThan(band(tester, 'recover-footer').top),
          reason: 'the sentence is read before the answers are offered');
    });

    /// 6. **The frame is the shared one**, so a change to the pattern reaches
    /// this dialogue without a second edit.
    testWidgets('the dialogue wears the shared frame', (tester) async {
      await open(tester);
      expect(find.byType(DialogFrame), findsOneWidget);

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
