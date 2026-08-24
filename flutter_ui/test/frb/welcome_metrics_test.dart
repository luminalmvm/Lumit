// The welcome screen, measured against the approved drawing, and made to work.
//
// **Why this file exists.** The screen is the first thing anybody sees, and it
// is built entirely to one mockup (K-451, K-464): every width, every row
// height, every face is a number read off that drawing, and a value that
// disagrees with it is a defect. So the first half of this file is a ruler.
//
// The second half is the behaviour: three cards that start work, a recents list
// that opens what it lists, a Clear that empties it and a × that takes one row
// off it. None of that is visible in a screenshot, and all of it is the point.

import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart'
    show captureViewerPicturePng;
import 'package:lumit_flutter/shell/menu_bar_frb.dart'
    show projectThumbnailCapture, saveProjectFrb;
import 'package:lumit_flutter/shell/welcome_frb.dart';
import 'package:lumit_flutter/state/external_links.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'frb_test_support.dart';

/// A real 1×1 PNG, so the widget that is handed it decodes rather than falling
/// into its error builder — the point of the test that renders one is that a
/// *picture* appears, not a placeholder wearing an `Image`'s name.
final Uint8List onePixelPng = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8'
  'z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
);

void main() {
  setUpAll(initEngineForTests);

  group('Welcome screen (frb)', () {
    final theme = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);

    /// Three remembered projects, newest first once the store has them.
    const paths = [
      '/home/ed/Projects/Camera tests/Train POV.lum',
      '/home/ed/Projects/Opening titles/Opening titles.lum',
      '/home/ed/Desktop/Set Me Free Edit/Set me free.lum',
    ];

    late bool done;

    Future<
        ({
          LumitState state,
          LumitUiState uiState,
        })> mount(
      WidgetTester tester, {
      List<String> recents = const [],
      Future<String?> Function()? openPicker,
      Future<String?> Function()? savePicker,
      Size size = const Size(900, 600),
    }) async {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      for (final path in recents) {
        p.uiState.workspace.rememberProject(path);
      }
      done = false;
      await tester.pumpWidget(hostPanel(
        child: WelcomeScreenFrb(
          onDone: () => done = true,
          openPicker: openPicker,
          savePicker: savePicker,
        ),
        state: p.state,
        uiState: p.uiState,
        size: size,
      ));
      await tester.pump();
      return p;
    }

    Rect band(WidgetTester tester, String key) =>
        tester.getRect(find.byKey(ValueKey<String>(key)));

    TextStyle styleOf(WidgetTester tester, String text) =>
        tester.widget<Text>(find.text(text)).style!;

    // --- The ruler --------------------------------------------------------

    /// 1. **The stack is 560 wide and its blocks are 28 apart.** Cards,
    /// recents and footer all sit in the same column, which is what makes the
    /// page read as one thing rather than three.
    testWidgets('the column is the drawing\'s 560', (tester) async {
      await mount(tester, recents: paths);

      expect(welcomeColumnWidth, 560);
      expect(welcomeBlockGap, 28);
      expect(band(tester, 'welcome-recent-well').width, welcomeColumnWidth,
          reason: 'the recents well is the full column');
      expect(band(tester, 'welcome-footer').width, welcomeColumnWidth);
      // 28 of air between the cards' bottom and the recents heading's top.
      final cards = band(tester, 'welcome-card-new');
      final header = band(tester, 'welcome-recent-header');
      expect(header.top - cards.bottom, welcomeBlockGap);
    });

    /// 2. **The wordmark is mono at 22 with 0.08em of tracking**, in the top
    /// of the text ramp — the one place on the page that is a brand mark
    /// rather than a phrase.
    testWidgets('the wordmark is the drawing\'s', (tester) async {
      await mount(tester);
      final mark = tester
          .widget<Text>(find.byKey(const ValueKey('welcome-wordmark')))
          .style!;

      expect(mark.fontSize, welcomeWordmarkSize, reason: '22 in the drawing');
      expect(mark.letterSpacing, welcomeWordmarkTracking, reason: '0.08em');
      expect(mark.fontFamily, LumitTheme.monoFontFamily);
      expect(mark.color, theme.textPrimary);
    });

    /// 3. **Three start cards, 180 × 63 with 10 between them.** The height is
    /// 14 of padding round a 13px title, a 4px gap and the 9px note, with the
    /// hairline counted in — §12A.6's rule that a mockup height is the
    /// effective one.
    testWidgets('the start cards are the drawing\'s size', (tester) async {
      await mount(tester);

      final left = band(tester, 'welcome-card-new');
      final middle = band(tester, 'welcome-card-blank');
      final right = band(tester, 'welcome-card-open');

      for (final card in [left, middle, right]) {
        expect(card.height, welcomeCardHeight, reason: '63 in the drawing');
        expect(card.width, 180, reason: '(560 - 2 × 10) / 3');
      }
      expect(middle.left - left.right, welcomeCardGap);
      expect(right.left - middle.right, welcomeCardGap);
      expect(right.right - left.left, welcomeColumnWidth);

      // A card's title is body at 13 in the primary grey; its note is the
      // kicker face at the drawing's looser 0.06em, sentence case.
      final title = styleOf(tester, l10n.welcomeNewProject);
      expect(title.fontSize, 13);
      expect(title.color, theme.textPrimary);
      final note = styleOf(tester, l10n.welcomeNewProjectNote);
      expect(note.fontSize, 9);
      expect(note.letterSpacing, 0.54);
      expect(note.color, theme.textMuted);
      expect(note.fontFamily, LumitTheme.monoFontFamily);
    });

    /// 4. **The recents heading is an 18px strip**, and it is the one kicker
    /// on the page that is capitalised: Recent names the container under it,
    /// Clear is an action beside it.
    testWidgets('the recents heading is a kicker strip', (tester) async {
      await mount(tester, recents: paths);

      expect(band(tester, 'welcome-recent-header').height,
          welcomeRecentHeaderHeight);
      expect(find.text(l10n.welcomeRecent.toUpperCase()), findsOneWidget,
          reason: 'a container label is set in capitals (§7.1)');
      final recent = styleOf(tester, l10n.welcomeRecent.toUpperCase());
      expect(recent.letterSpacing, theme.kicker.letterSpacing,
          reason: 'the full 0.12em, unlike the sentence-case kickers');
      expect(styleOf(tester, l10n.welcomeClearRecent).letterSpacing, 0.54);
    });

    /// 5. **Three rows measure 160 inside a hairline.** 52 apiece (K-468: the
    /// row grew from 40 to carry a picture), with a seam under all but the
    /// last, so the eye reads 53 / 53 / 52.
    testWidgets('the recents well is the drawing\'s height', (tester) async {
      await mount(tester, recents: paths);

      expect(welcomeRecentRowHeight, 52,
          reason: '8 of air either side of a 36-tall thumbnail');
      final well = band(tester, 'welcome-recent-well');
      expect(well.height, 160,
          reason: '53 + 53 + 52, plus a hairline either '
              'side');
      expect(band(tester, 'welcome-recent-row-0').height,
          welcomeRecentRowHeight + 1);
      expect(
          band(tester, 'welcome-recent-row-2').height, welcomeRecentRowHeight,
          reason: 'the last row has no seam under it');
    });

    /// 6. **A row's columns are the drawing's**, with the forget button the
    /// owner asked for taking its room out of the flexible name column — step
    /// 1 of §12A.6's ladder, which is where width is meant to come from.
    ///
    /// **And there is no format column** (K-468). The reserved 120px that held
    /// `1920×1080 · 25` is gone entirely: a size and a rate belong to a
    /// composition, and a project has as many of those as it likes.
    testWidgets('a recent row carries the drawing\'s columns', (tester) async {
      await mount(tester, recents: paths);

      final row = band(tester, 'welcome-recent-row-0');
      final close = band(tester, 'welcome-recent-close-0');
      expect(close.width, welcomeForgetColumnWidth);
      expect(row.right - close.right, welcomeRecentRowPadding.right,
          reason: 'the forget button sits at the far right of the row');

      // The picture opens the row: 16:9, sized to it, hard against the row's
      // own padding, and 12 before the name begins.
      final thumb = band(tester, 'welcome-recent-thumb-0');
      expect(thumb.width, welcomeThumbWidth);
      expect(thumb.height, welcomeThumbHeight);
      expect(thumb.width / thumb.height, 16 / 9, reason: '64 × 36 is 16:9');
      expect(thumb.left - row.left, welcomeRecentRowPadding.left);
      // 8 of air above it. Measured from the row's top rather than its centre:
      // this row carries the seam under it, so its rectangle is a pixel taller
      // than the space the picture is centred in.
      expect(thumb.top - row.top,
          (welcomeRecentRowHeight - welcomeThumbHeight) / 2);

      // Nothing sits between the name and the date but [welcomeNameGap]: with
      // the format column gone the name column takes the room, and the date's
      // 70 is the only fixed column left before the ×.
      final date = tester.getRect(find.text(l10n.welcomeToday).first);
      final nameBand = tester.getRect(find.text('Set me free'));
      expect(date.left - nameBand.right, greaterThan(0),
          reason: 'the name never runs into the date');
      expect(
          row.width -
              welcomeRecentRowPadding.horizontal -
              welcomeThumbWidth -
              welcomeThumbGap -
              welcomeNameGap -
              welcomeDateColumnWidth -
              welcomeForgetGap -
              welcomeForgetColumnWidth,
          348,
          reason: 'the name column is 348 inside the well\'s hairline — 28 '
              'wider than it was before the format column went (K-468)');

      // The newest project is the last one remembered.
      expect(find.text('Set me free'), findsOneWidget,
          reason: 'the name is the file\'s, without its extension');
      final name = styleOf(tester, 'Set me free');
      expect(name.fontSize, 11);
      expect(name.color, theme.textPrimary);

      final path = styleOf(tester, shortenHomePath(paths.last));
      expect(path.fontSize, 9);
      expect(path.color, theme.textDisabled);
      expect(path.fontFamily, LumitTheme.monoFontFamily);

      // Opened just now, so every row says so — mono at 10, muted, and 70 of
      // room hard right.
      final today = find.text(l10n.welcomeToday);
      expect(today, findsNWidgets(3));
      expect(tester.getRect(today.first).width, welcomeDateColumnWidth);
      final when = tester.widget<Text>(today.first).style!;
      expect(when.fontSize, 10);
      expect(when.color, theme.textMuted);
    });

    /// 7. **The footer is a 28px strip carrying two 24px outlined buttons**,
    /// and **no filled action at all**: the drawing spends none of the accent
    /// here, and §3.1's rule is a ceiling of one rather than a floor.
    testWidgets('the footer is two outlined links', (tester) async {
      await mount(tester);

      expect(band(tester, 'welcome-footer').height, welcomeFooterHeight);
      expect(band(tester, 'welcome-manual').height, welcomeButtonHeight);
      expect(band(tester, 'welcome-whats-new').height, welcomeButtonHeight);
      expect(styleOf(tester, l10n.welcomeManual).color, theme.textSecondary,
          reason: 'the label reads in the secondary grey, as the drawing sets '
              'it');
    });

    // --- What it does -----------------------------------------------------

    /// 8. **Blank project hands the window over and nothing else.** The empty
    /// project the application boots with is already loaded, so this card has
    /// nothing to make.
    testWidgets('Blank project opens the shell', (tester) async {
      final p = await mount(tester);

      await tester.tap(find.byKey(const ValueKey('welcome-card-blank')));
      await tester.pump();

      expect(done, isTrue);
      expect(p.state.project, isNotNull);
      expect(p.state.project!.path(), isNull,
          reason: 'saved later, as the '
              'card says');
    });

    /// 9. **A cancelled picker leaves the screen up.** Somebody who backed out
    /// of choosing a folder has not started work, and must not be dropped into
    /// an editor they did not ask for.
    testWidgets('a cancelled picker does not hand over', (tester) async {
      await mount(tester, savePicker: () async => null);

      await tester.tap(find.byKey(const ValueKey('welcome-card-new')));
      await tester.pumpAndSettle();

      expect(done, isFalse);
    });

    /// 10. **Clear empties the whole list**, and the well says so rather than
    /// collapsing to nothing. No question first: the one destructive control
    /// that asks is the disk cache, because that one throws away work with
    /// nothing to undo — a list of paths is rebuilt by opening a file.
    testWidgets('Clear empties the recents', (tester) async {
      final p = await mount(tester, recents: paths);
      expect(
          find.byKey(const ValueKey('welcome-recent-row-0')), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('welcome-clear-recent')));
      await tester.pump();

      expect(p.uiState.workspace.recentProjects, isEmpty);
      expect(
          find.byKey(const ValueKey('welcome-recent-empty')), findsOneWidget);
      expect(find.text(l10n.welcomeNoRecent), findsOneWidget);
    });

    /// 11. **The × forgets exactly one row**, and forgets nothing else. It is
    /// the innermost hit on the row, so pressing it never also opens the
    /// project underneath it.
    testWidgets('the × forgets one project and opens none', (tester) async {
      final p = await mount(tester, recents: paths);

      await tester.tap(find.byKey(const ValueKey('welcome-recent-close-1')));
      await tester.pump();

      expect(p.uiState.workspace.recentProjects, [paths.last, paths.first],
          reason: 'the middle row went and the other two stayed');
      expect(done, isFalse, reason: 'the × is not a way into the editor');
      expect(
          find.byKey(const ValueKey('welcome-recent-row-1')), findsOneWidget);
      expect(find.byKey(const ValueKey('welcome-recent-row-2')), findsNothing);
    });

    /// 12. **The links go to the right pages**, and say so in the status line
    /// when the desktop will not follow one.
    testWidgets('Manual and What\'s new follow their links', (tester) async {
      final p = await mount(tester);
      final asked = <String>[];
      final was = openExternalLink;
      openExternalLink = (url) async {
        asked.add(url);
        return false;
      };
      addTearDown(() => openExternalLink = was);

      await tester.tap(find.byKey(const ValueKey('welcome-manual')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('welcome-whats-new')));
      await tester.pumpAndSettle();

      expect(asked, [lumitDocsUrl, lumitReleasesUrl]);
      expect(done, isFalse, reason: 'reading the manual is not starting work');
      expect(p.state.notice.value?.error, isTrue,
          reason: 'a desktop that would not take the link says so');
    });

    /// 13. **A recent row opens its project and hands the window over.** The
    /// read itself is the engine's and takes as long as it takes; what the row
    /// owes is to ask for it and get out of the way, so the shell comes up
    /// behind its own progress card rather than the welcome screen sitting
    /// there while a document loads.
    testWidgets('a recent row opens the project', (tester) async {
      final p = await mount(tester, recents: paths);

      await tester.tap(find.byKey(const ValueKey('welcome-recent-row-0')));
      await tester.pump();

      expect(done, isTrue, reason: 'the shell takes the window');
      expect(p.state.opening.value, isTrue,
          reason: 'and the document is already being read behind it');
    });

    // --- The picture on a row (K-468) -------------------------------------

    /// A `.lum` to save to, and the thumbnail it would be filed under, both
    /// cleaned up after the test. The thumbnails themselves land beside the
    /// workspace store, which the harness has already redirected into a scratch
    /// folder — so a test run never writes a picture into the developer's own
    /// `%APPDATA%`.
    ({String project, File thumb}) scratchProject(String name) {
      final dir = Directory.systemTemp.createTempSync('lumit-thumb');
      addTearDown(() {
        try {
          dir.deleteSync(recursive: true);
        } catch (_) {}
      });
      final project = '${dir.path}${Platform.pathSeparator}$name.lum';
      final thumb = Workspace.thumbnailFile(project);
      addTearDown(() {
        if (thumb.existsSync()) thumb.deleteSync();
      });
      return (project: project, thumb: thumb);
    }

    /// 14. **A project's saved picture is found by its path and drawn.** The
    /// key is a digest of the path, so nothing about anybody's folder names
    /// reaches the file system and two projects both called `Untitled.lum`
    /// keep their own picture.
    testWidgets('a saved thumbnail is found by the project\'s path',
        (tester) async {
      expect(Workspace.thumbnailKey(paths.first),
          isNot(Workspace.thumbnailKey(paths.last)),
          reason: 'two projects never share a key');
      expect(Workspace.thumbnailFile(paths.last).path,
          contains(Workspace.thumbnailKey(paths.last)));
      expect(Workspace.thumbnailFile(paths.last).parent.path,
          Workspace.thumbnailDir().path,
          reason: 'beside the settings file, never inside the .lum');

      // The newest project is the last one remembered, so it is row 0.
      Workspace.writeThumbnail(paths.last, onePixelPng);
      addTearDown(() => Workspace.thumbnailFile(paths.last).deleteSync());

      await mount(tester, recents: paths);

      final slot = find.byKey(const ValueKey('welcome-recent-thumb-0'));
      expect(find.descendant(of: slot, matching: find.byType(Image)),
          findsOneWidget,
          reason: 'the saved picture is what the row shows');
      expect(
          find.descendant(
              of: slot,
              matching:
                  find.byKey(const ValueKey('welcome-recent-thumb-empty'))),
          findsNothing);
    });

    /// 15. **A project with no picture shows a quiet placeholder, and no
    /// words.** Never saved since the feature landed, moved since it was, or a
    /// capture that failed: all of them are ordinary, and a row that has to
    /// explain its own blank has stopped being a list.
    testWidgets('a project with no thumbnail shows the placeholder',
        (tester) async {
      await mount(tester, recents: paths);

      for (var i = 0; i < paths.length; i++) {
        final slot = find.byKey(ValueKey<String>('welcome-recent-thumb-$i'));
        expect(
            find.descendant(
                of: slot,
                matching:
                    find.byKey(const ValueKey('welcome-recent-thumb-empty'))),
            findsOneWidget,
            reason: 'row $i has no picture on disk');
        expect(find.descendant(of: slot, matching: find.byType(Image)),
            findsNothing);
      }
      // The slot is the same size either way, so a well full of projects that
      // have never been saved is the same shape as one full of pictures.
      expect(band(tester, 'welcome-recent-thumb-0').width, welcomeThumbWidth);
      expect(band(tester, 'welcome-recent-thumb-0').height, welcomeThumbHeight);
    });

    /// 16. **A save files the picture, and a later save replaces it.** One
    /// file per project, overwritten, rather than a folder that grows a picture
    /// for every save anybody ever made.
    testWidgets('a save writes the thumbnail and overwrites it',
        (tester) async {
      final scratch = scratchProject('Saved');
      var shot = onePixelPng;
      projectThumbnailCapture = () async => shot;
      addTearDown(() => projectThumbnailCapture = captureViewerPicturePng);

      final p = await mount(tester);
      // `runAsync`, because the write itself is a real bridge call on a worker
      // thread: awaited inside the test's fake clock it would never finish.
      await tester.runAsync(() => saveProjectFrb(p.state, p.uiState,
          forcePicker: true, picker: () async => scratch.project));

      expect(scratch.thumb.existsSync(), isTrue,
          reason: 'the save filed the project\'s picture');
      expect(scratch.thumb.readAsBytesSync(), onePixelPng);

      // A different picture, saved again over the same project.
      shot = Uint8List.fromList([...onePixelPng, 0, 1, 2]);
      await tester.runAsync(() => saveProjectFrb(p.state, p.uiState));

      expect(scratch.thumb.readAsBytesSync(), shot,
          reason: 'the second save replaced the first picture');
    });

    /// 17. **A capture that fails costs a picture and nothing else.** No
    /// Viewer up, a boundary that has not painted, a driver that will not read
    /// the texture back — the save has already happened by then and must not
    /// be told about any of it.
    testWidgets('a failing capture does not fail the save', (tester) async {
      final scratch = scratchProject('Unphotographed');
      projectThumbnailCapture = () async => throw StateError('no Viewer');
      addTearDown(() => projectThumbnailCapture = captureViewerPicturePng);

      final p = await mount(tester);
      await tester.runAsync(() => saveProjectFrb(p.state, p.uiState,
          forcePicker: true, picker: () async => scratch.project));

      expect(File(scratch.project).existsSync(), isTrue,
          reason: 'the project itself was written');
      expect(p.state.project!.path(), isNotNull);
      expect(p.state.notice.value?.error, isNot(isTrue),
          reason: 'the user is told the save worked, because it did');
      expect(scratch.thumb.existsSync(), isFalse,
          reason: 'and the row simply shows its placeholder');
    });
  }, skip: !engineAvailable);
}
