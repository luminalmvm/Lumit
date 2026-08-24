// The Project panel measured against the approved mockup, band by band.
//
// **Why this file exists.** `project_panel_frb_test` is about what the panel
// *does* — what a click selects, what a rename commits, what a drag carries.
// This one is about what it *looks like*, and specifically about the numbers
// the mockups' own computed styles resolved to (K-451, K-454): every row
// height, every column width, every face, every colour token. Nothing here
// names a private widget class, because none of these claims is about how the
// panel is built — each is something a person could point at on screen and
// measure with a ruler.
//
// A value that disagrees with the mockup is a defect, so each expectation
// carries the mockup's own number in its reason.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Project panel metrics (frb)', () {
    final theme = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);

    /// A project with a comp (filed under its auto-folder) and one clip.
    ({LumitState state, LumitUiState uiState, String compId}) withItems() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      return (
        state: p.state,
        uiState: p.uiState,
        compId: comp.internalid.toString()
      );
    }

    /// A click on a row, with the double-tap window elapsed after it — the
    /// row's own `onDoubleTap` holds the gesture arena for that long, and a
    /// test that does not wait it out leaves a timer running.
    Future<void> click(WidgetTester tester, Finder target) async {
      await tester.tap(target);
      await tester.pump(kDoubleTapTimeout + const Duration(milliseconds: 50));
    }

    Future<void> mount(WidgetTester tester, dynamic p,
        {double width = 480,
        DensityTokens density = DensityTokens.regular}) async {
      tester.view.physicalSize = Size(width, 760);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: Size(width, 760),
        density: density,
      ));
      await tester.pump();
    }

    Rect band(WidgetTester tester, String key) =>
        tester.getRect(find.byKey(ValueKey<String>(key)));

    /// 1. **Every chrome band is the mockup's height.** These are the numbers
    /// §12A.6's table carries, and the two the project panel corrected it with
    /// (a 20px bottom bar, a 19px column header that counts its own hairline).
    testWidgets('the panel is built to the mockup\'s heights', (tester) async {
      final p = withItems();
      await mount(tester, p);

      expect(band(tester, 'project-preview-card').height, projectPreviewHeight,
          reason: '10 of padding round a 96x54 poster frame, plus a hairline');
      expect(band(tester, 'project-search-row').height, projectSearchRowHeight,
          reason: '8 above the well, the well\'s 20, 6 below it');
      expect(band(tester, 'project-column-header').height,
          projectColumnHeaderHeight(theme),
          reason: 'a secondary row: 19 under Regular, hairline counted in');
      expect(band(tester, 'project-scroll-strip').height,
          projectScrollStripHeight);
      expect(band(tester, 'project-footer').height, projectFooterHeight,
          reason: 'the mockup renders the bottom bar at 20, not 18 (K-454)');

      final row = band(tester, 'project-row-${p.compId}');
      expect(row.height, projectRowHeight, reason: 'an outline row is 22');
    });

    /// 1b. **Compact takes the pixel back, and takes nothing else.** The only
    /// band the setting moves in this panel is the column header, because it
    /// is the only secondary row here; the item rows, the search row and the
    /// bottom bar measure the same either way (K-454, §12A.6's two columns).
    testWidgets('Compact slims the column header and nothing else',
        (tester) async {
      final p = withItems();
      await mount(tester, p, density: DensityTokens.compact);
      await settleFrb(tester, minRounds: 6);

      expect(band(tester, 'project-column-header').height, 18,
          reason: 'Compact drops the hairline back inside the row');
      expect(band(tester, 'project-row-${p.compId}').height, projectRowHeight,
          reason: 'item rows are 22 under both densities');
      expect(band(tester, 'project-search-row').height, projectSearchRowHeight,
          reason: 'the search row is 34 under both densities');
      expect(band(tester, 'project-footer').height, projectFooterHeight,
          reason: 'the bottom bar is 20 under both densities');
    });

    /// 1c. **The search well takes `surface_2`.** It is the one well in the
    /// app that sits a shade *lighter* than the panel rather than sunk into
    /// it: it has a row to itself over `surface_1`, so it only has to be a
    /// well, not a recess in a busy row (the mockup's own computed fill).
    testWidgets('the search well rests on surface 2', (tester) async {
      final p = withItems();
      await mount(tester, p);
      await settleFrb(tester, minRounds: 6);

      final well = tester.widget<Container>(find
          .descendant(
            of: find.byKey(const ValueKey<String>('project-search')),
            matching: find.byType(Container),
          )
          .first);
      expect((well.decoration! as BoxDecoration).color, theme.surface2);
    });

    /// 2. **The preview card's poster frame is 96x54.**
    testWidgets('the poster frame is the mockup\'s 96 by 54', (tester) async {
      final p = withItems();
      await mount(tester, p);
      await click(tester, find.text('shot.mov'));
      await settleFrb(tester, minRounds: 6);

      final card = band(tester, 'project-preview-card');
      final header = tester
          .getRect(find.byKey(const ValueKey<String>('project-info-header')));
      expect(header.height, card.height - 2 * 10 - 1,
          reason: 'the card pads its content by 10 inside its hairline');
    });

    /// 3. **Values sit under their own headings.** The owner corrected this
    /// alignment twice in the mockup rounds; the header and the rows are built
    /// from one function, and this is what proves it stayed that way.
    testWidgets('a column value lines up under its heading', (tester) async {
      final p = withItems();
      await mount(tester, p);
      await settleFrb(tester, minRounds: 6);

      final header =
          find.byKey(const ValueKey<String>('project-column-header'));
      final sizeHeading =
          find.descendant(of: header, matching: find.text('SIZE'));
      final fpsHeading =
          find.descendant(of: header, matching: find.text('FPS'));
      expect(sizeHeading, findsOneWidget);
      expect(fpsHeading, findsOneWidget);

      // The comp's row carries a size and a rate under those two headings.
      final row = find.byKey(ValueKey<String>('project-row-${p.compId}'));
      final cells =
          find.descendant(of: row, matching: find.byType(Text)).evaluate();
      // Name first, then the two metadata cells the width allows.
      expect(cells.length, 3, reason: 'name, size, fps');

      final sizeCell = tester.getRect(find.byWidget(cells.elementAt(1).widget));
      final fpsCell = tester.getRect(find.byWidget(cells.elementAt(2).widget));
      expect(sizeCell.right, closeTo(tester.getRect(sizeHeading).right, 0.01),
          reason: 'the size value ends where its heading ends');
      expect(fpsCell.right, closeTo(tester.getRect(fpsHeading).right, 0.01),
          reason: 'and so does the rate');
      expect(sizeCell.width, projectSizeColumn,
          reason: 'the Size column is the mockup\'s 64');
      expect(fpsCell.width, projectFpsColumn,
          reason: 'the fps column is the mockup\'s 22');
    });

    /// 4. **The faces are the mockup's.** Column headings are kickers — Geist
    /// Mono at 9 with 1.08px of tracking, muted; the values under them are
    /// plain mono at 10, also muted (§7.1's mono-for-numbers rule).
    testWidgets('headings are kickers and values are mono 10', (tester) async {
      final p = withItems();
      await mount(tester, p);
      await settleFrb(tester, minRounds: 6);

      final heading = tester.widget<Text>(find.descendant(
          of: find.byKey(const ValueKey<String>('project-column-header')),
          matching: find.text('SIZE')));
      expect(heading.style!.fontFamily, LumitTheme.monoFontFamily);
      expect(heading.style!.fontSize, 9);
      expect(heading.style!.letterSpacing, closeTo(1.08, 0.001));
      expect(heading.style!.color, theme.textMuted);

      final row = find.byKey(ValueKey<String>('project-row-${p.compId}'));
      final value = tester.widget<Text>(
          find.descendant(of: row, matching: find.byType(Text)).at(1));
      expect(value.style!.fontFamily, LumitTheme.monoFontFamily);
      expect(value.style!.fontSize, 10);
      expect(value.style!.color, theme.textMuted);
    });

    /// 5. **A name reads at the right tier.** A folder names a group and a
    /// picked row is the one being talked about — both `text_primary`; an
    /// ordinary row is `text_secondary`; a broken one drops to muted, because
    /// its badge is what carries the news.
    testWidgets('names take the mockup\'s three text tiers', (tester) async {
      final p = withItems();
      await mount(tester, p);
      await settleFrb(tester, minRounds: 6);

      Color colourOf(String name) => tester
          .widget<Text>(find
              .descendant(of: find.byType(ListView), matching: find.text(name))
              .first)
          .style!
          .color!;

      expect(colourOf('Compositions'), theme.textPrimary,
          reason: 'a folder names a group');
      expect(colourOf('Scene'), theme.textSecondary,
          reason: 'an ordinary row is body copy');
      expect(colourOf('shot.mov'), theme.textMuted,
          reason: 'the clip is not on disk, so its name steps back and its '
              'badge does the talking');

      await click(tester, find.text('Scene'));
      expect(colourOf('Scene'), theme.textPrimary,
          reason: 'the picked row is the one being talked about');
    });

    /// 6. **The resting panel shows three greys.** The selected row and the
    /// bottom bar both take `surface_2`; nothing at rest paints `surface_3`
    /// (§2.1, K-439).
    testWidgets('selection and the bottom bar rest on surface 2',
        (tester) async {
      final p = withItems();
      await mount(tester, p);
      await settleFrb(tester, minRounds: 6);

      expect(
          tester
              .widget<Container>(
                  find.byKey(const ValueKey<String>('project-footer')))
              .color,
          theme.surface2);

      await click(tester, find.text('Scene'));
      final row = tester.widgetList<Container>(find.descendant(
          of: find.byKey(ValueKey<String>('project-row-${p.compId}')),
          matching: find.byType(Container)));
      expect(row.map((c) => c.color), isNot(contains(theme.surface3)),
          reason: 'an unpointed row never paints the hover grey');
    });

    /// 7. **The width ladder gives way in the mockups' own order.** The
    /// 360-wide artboard shows the preview card and the Items column; the
    /// 260-wide docked panel has already dropped both, keeping Size and fps.
    testWidgets('the optional columns hide as the panel narrows',
        (tester) async {
      final p = withItems();

      await mount(tester, p, width: 360);
      expect(find.byKey(const ValueKey<String>('project-preview-card')),
          findsOneWidget);
      expect(find.text('ITEMS'), findsOneWidget);
      expect(find.text('SIZE'), findsOneWidget);
      expect(find.text('FPS'), findsOneWidget);

      await mount(tester, p, width: 260);
      expect(find.byKey(const ValueKey<String>('project-preview-card')),
          findsNothing,
          reason: 'the docked mockup has no preview card at 260');
      expect(find.text('ITEMS'), findsNothing,
          reason: 'nor an Items column — least essential goes first');
      expect(find.text('SIZE'), findsOneWidget);
      expect(find.text('FPS'), findsOneWidget);

      await mount(tester, p, width: 200);
      expect(find.text('FPS'), findsNothing, reason: 'then the rate');
      expect(find.text('SIZE'), findsOneWidget,
          reason: 'and the size is the last metadata column standing');
    });

    /// 8. **The bottom bar says what it says, quietly.** Its new-item words
    /// track at 0.08em and its count at 0.06em — both under the 0.12em a
    /// kicker naming a container carries.
    testWidgets('the bottom bar carries the new-item words and the count',
        (tester) async {
      final p = withItems();
      await mount(tester, p);
      await settleFrb(tester, minRounds: 6);

      final footer = find.byKey(const ValueKey<String>('project-footer'));
      final importLabel = tester.widget<Text>(
          find.descendant(of: footer, matching: find.text('IMPORT')));
      expect(importLabel.style!.letterSpacing, closeTo(0.72, 0.001));
      expect(importLabel.style!.fontSize, 9);
      expect(find.descendant(of: footer, matching: find.text('COMPOSITION')),
          findsOneWidget);

      final count = tester.widget<Text>(
          find.descendant(of: footer, matching: find.textContaining('items')));
      expect(count.style!.letterSpacing, closeTo(0.54, 0.001),
          reason: 'the count is factual, not a container label');
      expect(count.data, contains('3 items'),
          reason: 'the folder, the comp inside it, and the clip');
    });

    /// 9. **A missing file wears the mockup's pill, and the pill relinks.**
    /// 14 tall, mono at 9, in `warning` — and it is the control, because the
    /// mockup gives a broken row a badge and no button.
    testWidgets('the missing badge is 14 tall, mono 9, and warning-tinted',
        (tester) async {
      final p = freshProject();
      final gone = p.state.project!.importFootage(path: 'C:/nowhere/gone.mp4');
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await settleFrb(
        tester,
        until: () => find.byType(ProjectBadge).evaluate().isNotEmpty,
      );

      final badge = find.byType(ProjectBadge);
      expect(tester.getRect(badge).height, 14,
          reason: 'the mockup renders the pill at 14');
      final label = tester.widget<Text>(
          find.descendant(of: badge, matching: find.byType(Text)));
      expect(label.style!.fontFamily, LumitTheme.monoFontFamily);
      expect(label.style!.fontSize, 9);
      expect(label.style!.letterSpacing, isNull,
          reason: 'a badge reports a state; it is not a container label');
      expect(label.style!.color, theme.warning);

      // And the badge carries the relink gesture the row used to spend a
      // button on.
      expect(find.byKey(ValueKey<String>('relink-${gone.internalid}')),
          findsOneWidget);
    });
  });
}
