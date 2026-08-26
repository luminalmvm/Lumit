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
import 'package:lumit_flutter/icons/lumit_icon.dart' as glyph;
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

    Rect band(WidgetTester tester, String key) =>
        tester.getRect(find.byKey(ValueKey<String>(key)));

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

    /// The colour one tab's word is drawn in — bright for the section in force.
    Color? tabColour(WidgetTester tester, ExportSection section) => tester
        .widget<Text>(find
            .descendant(
              of: find.byKey(ValueKey<String>('export-tab-$section')),
              matching: find.byType(Text),
            )
            .first)
        .style
        ?.color;

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

    /// 2a. **The close mark sits in the corner.** The drawing pads the strip
    /// by 14 either side and pushes the mark to the far end of it, so the
    /// glyph's right edge is 14 in from the frame — not somewhere in the
    /// middle of the strip, which is where two competing flexible children
    /// used to leave it whenever the composition's name was short.
    testWidgets('the close mark is at the strip\'s far corner', (tester) async {
      await open(tester);

      final strip = band(tester, 'export-title-strip');
      final mark = tester.getRect(find.descendant(
        of: find.byKey(const ValueKey('export-close')),
        matching: find.byType(glyph.LumitIcon),
      ));
      expect(strip.right - mark.right, closeTo(dialogPadding, 0.01),
          reason: 'the drawing insets the mark by the strip\'s own 14');
      expect(mark.width, dialogCloseGlyph);
      expect(mark.center.dy, closeTo(strip.center.dy, 1),
          reason: 'and centred in the strip\'s height');
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

    /// 6. **The tabs name the sections, and the page holds them all.** The
    /// dialog is one scrolling page (K-485), so every group is in the tree at
    /// once and a tab is a place on it rather than a page of its own.
    testWidgets('the page holds every section at once', (tester) async {
      await open(tester);

      for (final section in ExportSection.values) {
        expect(
            find.byKey(ValueKey<String>('export-tab-$section')), findsOneWidget,
            reason: 'the drawing gives the strip six tabs');
      }
      for (final group in const [
        'output',
        'composition',
        'time',
        'picture',
        'colour',
        'audio',
        'metadata',
      ]) {
        expect(
            find.byKey(ValueKey<String>('export-group-$group')), findsOneWidget,
            reason: 'one page holds every group, Composition included');
      }
      expect(find.text('STILL'), findsNothing,
          reason: 'a still is an image sequence of one frame (K-485)');
    });

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

    /// 6b. **The strip follows the page.** Scrolling the body down to a later
    /// section moves the selected tab with it — the tab says where you are, not
    /// where you last clicked.
    testWidgets('the tab strip follows the scroll', (tester) async {
      await open(tester, height: 520);
      final t = ThemeScope.of(tester.element(find.byType(DialogFrame))).theme;
      expect(tabColour(tester, ExportSection.output), t.kickerOn.color);

      final before = band(tester, 'export-group-output').top;
      await tester.drag(find.byKey(const ValueKey('export-group-output')),
          const Offset(0, -900));
      await tester.pumpAndSettle();
      expect(band(tester, 'export-group-output').top, lessThan(before),
          reason: 'the body scrolls under the drag');

      expect(tabColour(tester, ExportSection.output), isNot(t.kickerOn.color),
          reason: 'Output is behind you once the page has moved past it');
      expect(
        ExportSection.values
            .where((s) => tabColour(tester, s) == t.kickerOn.color),
        hasLength(1),
        reason: 'exactly one tab is in force, whatever the scroll',
      );
    });

    /// 6c. **The right column of a paired row.** Its label column is the
    /// narrower 78 so the frame rate's value well always fits beside its list,
    /// and every control in that column shares one left edge and one right
    /// (K-485: the drawing asks for 212 of control in a 173 column).
    testWidgets('the paired right column aligns and fits its value well',
        (tester) async {
      await open(tester);

      final rate = band(tester, 'export-rate-source');
      final well = band(tester, 'export-fps');
      final effects = band(tester, 'export-effects');
      expect(well.width, exportNumberWell,
          reason: "the drawing's value well is 56, and it never shrinks");
      expect(well.right, closeTo(rate.right + 6 + exportNumberWell, 0.01),
          reason: 'the well stands 6 after the list, inside the same row');
      expect(effects.right, closeTo(well.right, 0.01),
          reason: 'every right-column control ends at one edge');
      expect(effects.left, closeTo(rate.left, 0.01),
          reason: 'and begins at one edge');

      final label = tester.getRect(find
          .ancestor(
              of: find.text('Frame rate'), matching: find.byType(SizedBox))
          .first);
      expect(label.width, exportLabelColumnPaired,
          reason: 'the right column extends its control left into its label');
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

    /// 6d. **The preset strip is chrome, not a row** (K-487). A preset sets
    /// and saves every section of this dialog, so it sits in a band of its own
    /// under the tab row and above the scrolling body — full width, 8 above a
    /// 22px control and 8 below it. No rule under it (owner): standing
    /// outside every section already says it is global.
    testWidgets('the preset strip is a band under the tabs', (tester) async {
      await open(tester);

      final tabs = band(tester, 'export-tabs');
      final strip = band(tester, 'export-preset-strip');
      final output = band(tester, 'export-group-output');

      expect(strip.width, exportDialogWidth,
          reason: 'the strip is the dialog\'s full width, as the tab row is');
      expect(strip.top, tabs.bottom,
          reason: 'it sits directly under the tab row');
      expect(strip.height, exportPresetStrip,
          reason: 'K-588: two lines of 22 at rest, 8 above each and 8 under '
              'the last, and no rule under the band');
      expect(output.top, greaterThanOrEqualTo(strip.bottom),
          reason: 'and above the body, which scrolls under it');

      // It is not a tab, and the scroll-spy neither reads it nor is read by it.
      expect(find.byKey(const ValueKey('export-tab-ExportSection.preset')),
          findsNothing);
      expect(ExportSection.values, hasLength(6));
    });

    /// 6e. **Nothing in the strip is clipped**, which is the whole of the fix:
    /// *Save as…* used to share a 173px paired column with a list and *Edit*,
    /// and lost. Each button is now its own content's width, with air after
    /// the last of them.
    ///
    /// K-588's *Set as default* is why the band has two lines: at 180px wide
    /// it overflowed the strip by 118 standing beside the other two, so it took
    /// a line of its own, starting under the list rather than under the label.
    testWidgets('the preset controls have room to breathe', (tester) async {
      await open(tester);

      final strip = band(tester, 'export-preset-strip');
      final list = band(tester, 'export-preset');
      final edit = band(tester, 'export-preset-edit');
      final saveAs = band(tester, 'export-preset-save-as');
      final setDefault = band(tester, 'export-preset-set-default');

      expect(list.left - strip.left,
          dialogPadding + 12 + exportLabelColumn + exportRowGap,
          reason: 'inset to the group rows own edge (14 + their inner 12), '
              'then the 100 and 10, so Preset lines up with the labels below');
      expect(list.width, exportPresetDropdown,
          reason: 'the list itself is 220');
      expect(edit.left - list.right, exportPresetStripGap);
      expect(saveAs.left - edit.right, exportPresetStripGap);
      expect(setDefault.left, list.left,
          reason: 'K-588: on its own line, under the list it acts on');
      expect(setDefault.top - saveAs.bottom, exportPresetStripGap,
          reason: 'the strip\'s own 8 between one line and the next');

      // The label is drawn whole — a clipped button is a Text narrower than
      // its own word, which is what the old column produced.
      final label = tester.getRect(find.descendant(
        of: find.byKey(const ValueKey('export-preset-save-as')),
        matching: find.byType(Text),
      ));
      expect(saveAs.width, closeTo(label.width + 24 + 2, 0.01),
          reason: '§12A.4: 12 either side of an outlined label — and the '
              'button\'s own 1px edge either side of that — no less');
      expect(strip.right - dialogPadding - saveAs.right, greaterThan(60),
          reason: 'and the first line still has air after it at 640');
      expect(strip.right - dialogPadding - setDefault.right, greaterThan(0),
          reason: 'as does the second');
    });

    /// 6f. **Naming a preset happens in the same strip**, on a second line
    /// under the list it is naming — not in the body, which scrolls away.
    testWidgets('the name row opens inside the strip', (tester) async {
      await open(tester);

      final before = band(tester, 'export-preset-strip');
      expect(find.byKey(const ValueKey('export-preset-name')), findsNothing);

      await tester.tap(find.byKey(const ValueKey('export-preset-save-as')));
      await tester.pumpAndSettle();

      final after = band(tester, 'export-preset-strip');
      final name = band(tester, 'export-preset-name');
      expect(after.height,
          before.height + dialogControlHeight + exportPresetStripGap,
          reason: 'the strip grows by one line of 22 and its 8 of air');
      expect(after.contains(name.center), isTrue,
          reason: 'the field is in the strip, not in the scrolling body');
      expect(find.byKey(const ValueKey('export-preset-save')), findsOneWidget);
      expect(
          find.byKey(const ValueKey('export-preset-cancel')), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('export-preset-cancel')));
      await tester.pumpAndSettle();
      expect(band(tester, 'export-preset-strip').height, before.height);
    });

    /// 6g. **A saved preset's name row carries four controls, and still fits.**
    /// *Delete* joins *Save* and *Cancel* the moment the preset in force is
    /// one of the user's own, and 220 of field plus four buttons is eight
    /// pixels more than the strip has — an overflow, which is a defect
    /// (§12A.6). The buttons keep their content width, so the field gives.
    testWidgets('the name row fits when a preset can also be deleted',
        (tester) async {
      await open(tester);

      await tester.tap(find.byKey(const ValueKey('export-preset-save-as')));
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey('export-preset-name')), 'Metrics preset');
      await tester.tap(find.byKey(const ValueKey('export-preset-save')));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('export-preset-edit')));
      await tester.pumpAndSettle();
      final strip = band(tester, 'export-preset-strip');
      final delete = band(tester, 'export-preset-delete');
      final cancel = band(tester, 'export-preset-cancel');
      expect(find.byKey(const ValueKey('export-preset-delete')), findsOneWidget,
          reason: "a preset of one's own can be taken off the list");
      expect(delete.right, lessThanOrEqualTo(cancel.left));
      expect(cancel.right, lessThanOrEqualTo(strip.right - dialogPadding),
          reason: 'and the last of the four is still inside the strip');

      await tester.tap(find.byKey(const ValueKey('export-preset-delete')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
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
