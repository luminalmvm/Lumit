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
      expect(band(tester, 'project-search').height, 20,
          reason: 'the well itself renders the stated 20 - it once '
              'shrink-wrapped its text to 16');
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

    /// 3b. **The glyphs are the mockup's sizes, and the row cluster with
    /// them** (K-456: the 16 grid is what a glyph is drawn on, not what it
    /// displays at). A row's twirl slot and type mark are 13 square, so a name
    /// starts at 8 + 13 + 8 + 13 + 8 = 50, and a child sits one indent step
    /// further in. The bottom bar's controls draw at 13 too.
    testWidgets('glyphs are 13 in a row and on the bottom bar', (tester) async {
      final p = withItems();
      await mount(tester, p, width: 360);
      await settleFrb(tester, minRounds: 6);

      // A folder's twirl is the one glyph in a row with a key of its own.
      final twirl = find.byWidgetPredicate((w) =>
          w is GestureDetector &&
          w.key is ValueKey<String> &&
          (w.key! as ValueKey<String>).value.startsWith('project-twirl-'));
      expect(twirl, findsWidgets);
      expect(tester.getRect(twirl.first).size,
          const Size(projectRowIconSize, projectRowIconSize),
          reason: 'the mockup draws a row\'s glyph 13 square');

      // Twirl slot, type mark and the two 8px gaps put a top-level name at 50.
      const nameLeft = projectRowPadding +
          projectRowIconSize +
          projectRowGap +
          projectRowIconSize +
          projectRowGap;
      expect(nameLeft, 50, reason: 'the mockup\'s own name column');
      expect(tester.getRect(find.text('Compositions')).left,
          closeTo(nameLeft, 0.01),
          reason: 'the row cluster holds the mockup\'s name column');
      expect(tester.getRect(find.text('Scene')).left,
          closeTo(nameLeft + projectIndentPerDepth, 0.01),
          reason: 'a child is one indent step further in');

      // The bottom bar's controls are a size up, as its mockup computes them.
      expect(
          tester
              .getRect(find.byKey(const ValueKey<String>('project-new-folder')))
              .height,
          projectFooterIconSize,
          reason: 'the bottom bar draws its glyphs at 13');
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
    /// 4a. **A wider panel widens Path, not Name** (owner, desk test). It used
    /// to be the other way about: Name was the flexible column and Path a
    /// fixed 40, so every pixel the panel gained went to names that already
    /// fitted while the one value longer than its column by nature stayed
    /// clipped at 40.
    testWidgets('the panel\'s spare width all lands in the Path column',
        (tester) async {
      final p = withItems();
      final header =
          find.byKey(const ValueKey<String>('project-column-header'));
      double heading(WidgetTester tester, String word) => tester
          .getRect(find.descendant(of: header, matching: find.text(word)))
          .width;

      // The mockup's own artboard is too narrow for the arrangement Name asks
      // for, so Name is still the column that gives and the drawing is
      // untouched: 148 of name and a 40 of path, exactly as before.
      await mount(tester, p, width: 360);
      await settleFrb(tester, minRounds: 6);
      expect(heading(tester, 'NAME'), 148,
          reason: 'the 360 artboard draws as it always did');
      expect(heading(tester, 'PATH'), projectPathColumn,
          reason: 'and Path is at its narrowest there');

      await mount(tester, p, width: 480);
      await settleFrb(tester, minRounds: 6);
      expect(heading(tester, 'NAME'), projectNameColumn,
          reason: 'past the width it asks for, Name settles at its own');

      await mount(tester, p, width: 560);
      await settleFrb(tester, minRounds: 6);
      expect(heading(tester, 'NAME'), projectNameColumn,
          reason: 'and keeps it however wide the panel grows');
      expect(heading(tester, 'SIZE'), projectSizeColumn,
          reason: 'and so does every column between');
      expect(heading(tester, 'PATH'), projectPathColumn + 128,
          reason: 'every pixel past the arrangement is Path\'s: 560 less the '
              'insets, the Name column and the other three columns');
    });

    /// 4b. **The seams between the headings drag**, on the Timeline outline's
    /// own rule: a seam widens the column to its left and every other column
    /// keeps its width, so the drag moves one boundary and nothing else.
    testWidgets('dragging a column seam moves that column and no other',
        (tester) async {
      final p = withItems();
      await mount(tester, p, width: 560);
      await settleFrb(tester, minRounds: 6);

      final header =
          find.byKey(const ValueKey<String>('project-column-header'));
      double heading(String word) => tester
          .getRect(find.descendant(of: header, matching: find.text(word)))
          .width;

      // The seam right of NAME — the one drawn before the Items column.
      final seam = find.byKey(const ValueKey<String>('project-seam-name'));
      expect(seam, findsOneWidget);
      await tester.drag(seam, const Offset(40, 0));
      await tester.pump();

      expect(heading('NAME'), projectNameColumn + 40,
          reason: 'the seam widened the column it follows');
      expect(heading('SIZE'), projectSizeColumn,
          reason: 'and left the columns between alone');
      expect(heading('PATH'), projectPathColumn + 128 - 40,
          reason: 'Path gave up exactly what Name took, since it holds the '
              'panel\'s slack');

      // Back past its minimum: the column stops there rather than vanishing.
      await tester.drag(seam, const Offset(-400, 0));
      await tester.pump();
      expect(heading('NAME'), minProjectColumnWidth(ProjectColumn.name),
          reason: 'a column stops at what its cells need');
    });

    /// 4c. **A column whose cells cannot use more room has no handle** — the
    /// Timeline's [groupIsFixedWidth] rule. Items counts children and fps
    /// writes a rate; both are as wide as their number and no wider.
    testWidgets('a fixed-width column offers no seam handle', (tester) async {
      final p = withItems();
      await mount(tester, p, width: 560);
      await settleFrb(tester, minRounds: 6);

      expect(find.byKey(const ValueKey<String>('project-seam-name')),
          findsOneWidget,
          reason: 'Name takes hold');
      expect(find.byKey(const ValueKey<String>('project-seam-size')),
          findsOneWidget,
          reason: 'and so does Size');
      for (final fixed in [ProjectColumn.items, ProjectColumn.fps]) {
        expect(projectColumnIsFixedWidth(fixed), isTrue);
        // The seam is still drawn — the gap has to be there — but it carries
        // no hairline and no handle, so it is not a key the panel offers.
        final gap = find.byKey(ValueKey<String>('project-seam-${fixed.name}'));
        expect(tester.widgetList(gap).length, 1,
            reason: 'the gap keeps its width so the columns stay aligned');
      }
      expect(projectColumnIsFixedWidth(ProjectColumn.path), isTrue,
          reason: 'Path has no width of its own to drag: it is the slack');
    });

    testWidgets('the optional columns hide as the panel narrows',
        (tester) async {
      final p = withItems();

      await mount(tester, p, width: 360);
      expect(find.byKey(const ValueKey<String>('project-preview-card')),
          findsOneWidget);
      expect(find.text('ITEMS'), findsOneWidget);
      expect(find.text('SIZE'), findsOneWidget);
      expect(find.text('FPS'), findsOneWidget);
      expect(find.text('PATH'), findsOneWidget,
          reason: 'the 360 artboard shows every column');

      await mount(tester, p, width: 260);
      expect(find.byKey(const ValueKey<String>('project-preview-card')),
          findsNothing,
          reason: 'the docked mockup has no preview card at 260');
      expect(find.text('ITEMS'), findsNothing,
          reason: 'nor an Items column — least essential goes first');
      expect(find.text('PATH'), findsNothing,
          reason: 'nor a Path column, which leaves at the same step');
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

      expect(find.descendant(of: footer, matching: find.text('FOLDER')),
          findsOneWidget,
          reason: 'the mockup draws Folder beside Composition');

      final count = tester.widget<Text>(
          find.descendant(of: footer, matching: find.textContaining('items')));
      expect(count.style!.letterSpacing, closeTo(0.54, 0.001),
          reason: 'the count is factual, not a container label');
      expect(count.data, contains('3 items'),
          reason: 'the folder, the comp inside it, and the clip');
    });

    /// 8a. **The count reads `1 missing · 10 items`.** The total is the bar's
    /// last word — hard right, where the eye looks for it — and the missing
    /// half sits to its left, still the "show only missing" control.
    testWidgets('the missing half reads before the item total', (tester) async {
      final p = freshProject();
      p.state.project!.newComposition(name: 'Scene');
      p.state.project!.importFootage(path: 'C:/nowhere/gone.mp4');
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await settleFrb(
        tester,
        until: () =>
            find.byKey(const ValueKey('missing-toggle')).evaluate().isNotEmpty,
      );

      final footer = find.byKey(const ValueKey<String>('project-footer'));
      final missing =
          find.descendant(of: footer, matching: find.textContaining('missing'));
      final items =
          find.descendant(of: footer, matching: find.textContaining('items'));
      expect(missing, findsOneWidget);
      expect(items, findsOneWidget);
      expect(tester.widget<Text>(missing).data, startsWith('1 missing'),
          reason: 'the two halves stay two strings, ordered by the layout');
      expect(tester.getRect(missing).right,
          lessThanOrEqualTo(tester.getRect(items).left),
          reason: 'missing first, then the total at the bar\'s far right');
    });

    /// 8b. **The Path column is quieter than its neighbours.** It is the one
    /// column carrying context rather than a fact about the item, so both the
    /// heading and the value sit at `text_disabled` where the rest are
    /// `text_muted` (§12A.3a, and the mockup's own two greys).
    testWidgets('the Path column and its heading are text_disabled',
        (tester) async {
      final p = withItems();
      await mount(tester, p, width: 360);
      await settleFrb(tester, minRounds: 6);

      final path = tester.widget<Text>(find.text('PATH'));
      expect(path.style!.color, theme.textDisabled,
          reason: 'the mockup hushes the Path heading below the other four');
      expect(path.style!.fontFamily, LumitTheme.monoFontFamily);
      expect(path.style!.fontSize, 9);

      final size = tester.widget<Text>(find.text('SIZE'));
      expect(size.style!.color, theme.textMuted,
          reason: 'and only that one — the rest keep the kicker grey');
    });

    /// 8c. **The `in use` badge is the missing badge's twin in `success`.**
    /// Same 14px pill, same mono 9 with no tracking; a badge reports a state,
    /// so it is deliberately not a kicker (§12A.3a).
    testWidgets('the in use badge is a 14px success pill, mono 9',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final used = p.state.project!.importFootage(path: 'C:/clips/used.mov');
      comp.addFootageLayer(footage: used, asSequence: false);
      await mount(tester, (state: p.state, uiState: p.uiState, compId: ''));
      await settleFrb(tester, minRounds: 6);

      final badge = find.byKey(ValueKey<String>('in-use-${used.internalid}'));
      expect(tester.getRect(badge).height, 14,
          reason: 'the mockup renders the pill at 14, as it does missing');
      final label = tester.widget<Text>(
          find.descendant(of: badge, matching: find.byType(Text)));
      expect(label.style!.fontFamily, LumitTheme.monoFontFamily);
      expect(label.style!.fontSize, 9);
      expect(label.style!.letterSpacing, isNull,
          reason: 'a badge reports a state; it is not a container label');
      expect(label.style!.color, theme.success);
    });

    /// 8d. **The colour chips are the mockup's six 6px dots.** Five palette
    /// colours and a neutral one, in a row beside the search well.
    testWidgets('the filter chips are six 6px dots beside the search well',
        (tester) async {
      final p = withItems();
      await mount(tester, p, width: 360);
      await settleFrb(tester, minRounds: 6);

      final strip = find.byKey(const ValueKey<String>('project-label-chips'));
      expect(strip, findsOneWidget);
      for (final label in [...projectFilterLabels, 'none']) {
        final chip = find.byKey(ValueKey<String>('project-label-chip-$label'));
        expect(chip, findsOneWidget);
        expect(tester.getRect(chip).size, const Size(6, 6),
            reason: 'the mockup draws each chip 6 by 6');
      }
      // Inside the search row, and to the right of the well.
      final well = tester.getRect(find.byKey(const ValueKey('project-search')));
      expect(tester.getRect(strip).left, greaterThanOrEqualTo(well.right));
    });

    /// 8e. **The search row measured at the mockup's own width** (owner,
    /// 2026-08-24). The owner kept reading the app's well as wider than the
    /// drawing's, so it was probed at 1:1 — the artboard is 360 across, and
    /// the manifest resolves the well to 279x20 with a 59-wide chip strip and
    /// a 6 between them.
    ///
    /// It was 282x20 with a 62-wide strip and no gap: the strip gave every
    /// chip a leading 3, including the first, which spent the row's gap inside
    /// the strip and left the well three pixels over. Every other number in
    /// the row — the 8 either side, the 20, the fill, the border — already
    /// agreed, so this is the whole of the difference, and it is worth a test
    /// because three pixels is exactly the size of thing that comes back.
    testWidgets('the search row is the mockup\'s row, at the mockup\'s width',
        (tester) async {
      final p = withItems();
      await mount(tester, p, width: 360);

      final row = band(tester, 'project-search-row');
      final well = band(tester, 'project-search');
      final chips = band(tester, 'project-label-chips');

      expect(row.width, 360);
      expect(well.left - row.left, 8,
          reason: 'the row is padded 8 at the left');
      expect(well.size, const Size(279, 20),
          reason: 'the manifest resolves the well to 279 by 20');
      expect(chips.left - well.right, 6,
          reason: 'the mockup\'s row is a flex line with gap: 6');
      expect(chips.width, 59,
          reason: 'six 6px dots, 3 apart, in a strip padded 4 either side');
      expect(row.right - chips.right, 8,
          reason: 'and padded 8 at the right, like every other row');
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
