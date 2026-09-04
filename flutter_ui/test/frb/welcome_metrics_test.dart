// The welcome screen, measured against the approved drawing, and made to work.
//
// **Why this file exists.** The screen is the first thing anybody sees, and it
// is built entirely to one mockup: every width, every row
// height, every face is a number read off that drawing, and a value that
// disagrees with it is a defect. So the first half of this file is a ruler.
//
// The second half is the behaviour: three cards that start work, a recents list
// that opens what it lists, a Clear that empties it and a × that takes one row
// off it. None of that is visible in a screenshot, and all of it is the point.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart'
    show captureViewerPicturePng;
import 'package:lumit_flutter/shell/menu_bar_frb.dart'
    show projectThumbnailCapture, saveProjectFrb;
import 'package:lumit_flutter/shell/about_window_frb.dart'
    show lumitProductVersion;
import 'package:lumit_flutter/shell/welcome_frb.dart';
import 'package:lumit_flutter/shell/wordmark.dart';
import 'package:lumit_flutter/state/external_links.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/brand.dart';
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

    /// 2. **The wordmark is the brand's own lockup**, not the word set
    /// in mono: the blue key, `umi`, and the violet key that is the blue one
    /// turned through 180°. The two keys are brand tokens, so they are the same
    /// in every colour scheme; only the lettering follows the theme.
    testWidgets('the wordmark is the website\'s', (tester) async {
      await mount(tester);
      final mark = tester.widget<LumitWordmark>(
          find.byKey(const ValueKey('welcome-wordmark')));

      expect(mark.height, welcomeWordmarkHeight, reason: '22 in the drawing');
      expect(mark.ground, theme.surface0, reason: 'the page it stands on');
      expect(mark.letters, brandWordmarkPaper,
          reason: 'light lettering on the dark scheme\'s ground');
      final box =
          tester.getRect(find.byKey(const ValueKey('welcome-wordmark'))).size;
      expect(box.height, welcomeWordmarkHeight);
      expect(
          box.width, closeTo(welcomeWordmarkHeight * lumitWordmarkAspect, 0.01),
          reason: 'the width follows the lockup');
    });

    /// 2b. **The drawing is the website's own file**, copied rather than
    /// redrawn: the two key gradients as the brand sets them, the `t` as the
    /// `l` flipped about the lockup's centre, and the lettering left to inherit
    /// so it can follow the theme.
    testWidgets('the wordmark asset is the site\'s drawing', (tester) async {
      final svg = await rootBundle.loadString(lumitWordmarkAsset);

      String hex(Color c) {
        String channel(double v) =>
            (v * 255).round().toRadixString(16).padLeft(2, '0');
        return '#${channel(c.r)}${channel(c.g)}${channel(c.b)}';
      }

      for (final key in [
        brandKeyJade,
        brandKeyLime,
        brandKeyBlueLight,
        brandKeyBlue,
      ]) {
        expect(svg, contains(hex(key)),
            reason: '${hex(key)} is one of the mark\'s keys');
      }
      expect(svg, contains('rotate(180 134.26 -35.16)'),
          reason: 'the t is the l flipped about the lockup\'s centre');
      expect('currentColor'.allMatches(svg).length, 3,
          reason: 'u, m and i inherit; nothing else does');
      expect(hex(brandWordmarkPaper), '#f4f6f8',
          reason: 'the token is the fill the site\'s own file carries');
    });

    /// 2c. **The lettering is chosen against the ground it stands on**, so the
    /// mark is legible under every scheme and under a custom theme nobody has
    /// seen yet. The keys are never chosen: they are the brand.
    test('the wordmark\'s lettering follows the ground', () {
      final dark =
          LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);
      final light =
          LumitTheme.forScheme(LumitColorScheme.light, ThemeShape.sharp);

      expect(wordmarkLetters(dark.surface0), brandWordmarkPaper);
      expect(wordmarkLetters(light.surface0), brandWordmarkInk,
          reason: 'dark letters on a light page');
      // A custom theme is judged the same way — by its ground, not by its name.
      expect(wordmarkLetters(const Color(0xfff7f3ea)), brandWordmarkInk,
          reason: 'a bright custom ground takes dark letters');
      expect(wordmarkLetters(const Color(0xff141414)), brandWordmarkPaper);
      expect(wordmarkLetters(null), brandWordmarkPaper,
          reason: 'no ground to judge: the mark as it is usually seen');
    });

    /// 3. **Two start cards, 63 tall with 10 between them.** The height is
    /// 14 of padding round a 13px title, a 4px gap and the 9px note, with the
    /// hairline counted in — §12A.6's rule that a mockup height is the
    /// effective one. There were three until the folder-first card came off
    /// the page; the two that are left share the same 560 column.
    testWidgets('the start cards are the drawing\'s size', (tester) async {
      await mount(tester);

      expect(find.byKey(const ValueKey('welcome-card-blank')), findsNothing,
          reason: 'the page offers New project and Open, and nothing else');

      final left = band(tester, 'welcome-card-new');
      final right = band(tester, 'welcome-card-open');

      for (final card in [left, right]) {
        expect(card.height, welcomeCardHeight, reason: '63 in the drawing');
        expect(card.width, 275, reason: '(560 - 10) / 2');
      }
      expect(right.left - left.right, welcomeCardGap);
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

    /// 5. **Three rows measure 160 inside a hairline.** 52 apiece (the row
    /// grew from 40 to carry a picture), with a seam under all but the
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
    /// **And there is no format column**. The reserved 120px that held
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
              'wider than it was before the format column went');

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

    /// 8. **New project hands the window over and nothing else**. The
    /// empty project the application boots with is already loaded, so this
    /// card has nothing to make and nothing to ask: where the file goes is a
    /// question for the first save. It was the *Blank project* card until the
    /// owner took the folder-first one off the page and gave this one its
    /// name.
    testWidgets('New project opens the shell, asking nothing', (tester) async {
      final p = await mount(tester);

      await tester.tap(find.byKey(const ValueKey('welcome-card-new')));
      await tester.pump();

      expect(done, isTrue);
      expect(p.state.project, isNotNull);
      expect(p.state.project!.path(), isNull,
          reason: 'saved later, as the card says');
    });

    /// 9c. **Escape closes the screen with nothing open**. It is the
    /// standard way out of anything that has taken the window, and it is safe
    /// because the shell behind it offers the same two ways to start.
    testWidgets('Escape closes the welcome screen', (tester) async {
      await mount(tester);

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();

      expect(done, isTrue);
    });

    /// 9d. **The footer says which Lumit this is**, not which library printed
    /// the boot line: "Lumit 0.2.0", the one product version Settings ▸
    /// General shows too.
    testWidgets('the version line is the product\'s, not the crate\'s',
        (tester) async {
      await mount(tester);

      final line = lumitProductVersion();
      expect(line, startsWith('Lumit '));
      expect(line, isNot(contains('lumit-bridge')),
          reason: 'the crate\'s name is for the boot log and bug reports');
      expect(find.text(line), findsOneWidget);
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

    // --- The picture on a row ---------------------------------------------

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

    /// 17. **A capture that fails costs a picture and nothing else.** A
    /// boundary that has not painted, a driver that will not read the texture
    /// back, a machine with no graphics adapter — the save has already happened
    /// by then and must not be told about any of it. **Both** roads have to
    /// fail for the row to go without: that is the point of there being two.
    testWidgets('a failing capture does not fail the save', (tester) async {
      final scratch = scratchProject('Unphotographed');
      projectThumbnailCapture = () async => throw StateError('no Viewer');
      addTearDown(() => projectThumbnailCapture = captureViewerPicturePng);
      final wasEngine = Workspace.compThumbnailPng;
      Workspace.compThumbnailPng = (comp, frame) async => null;
      addTearDown(() => Workspace.compThumbnailPng = wasEngine);

      final p = await mount(tester);
      p.uiState.setSelectedComp(p.state.project!.newComposition(name: 'Scene'));
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

    /// 18. **A save with no Viewer up still files a picture**. This is
    /// the whole regression: an After Effects conversion, a script, a save from
    /// a workspace with the Viewer closed — every one of them photographed a
    /// Viewer that was not there, filed nothing, and left the owner's welcome
    /// screen a column of empty wells. The engine draws the fronted composition
    /// instead, and the picture is filed exactly as the photograph would be.
    testWidgets('a headless save files the engine\'s picture', (tester) async {
      final scratch = scratchProject('Headless');
      // Precisely the headless case: there is no Viewer, so the photograph
      // answers null rather than throwing.
      projectThumbnailCapture = () async => null;
      addTearDown(() => projectThumbnailCapture = captureViewerPicturePng);
      final asked = <int>[];
      final wasEngine = Workspace.compThumbnailPng;
      Workspace.compThumbnailPng = (comp, frame) async {
        asked.add(frame);
        return onePixelPng;
      };
      addTearDown(() => Workspace.compThumbnailPng = wasEngine);

      final p = await mount(tester);
      p.uiState.setSelectedComp(p.state.project!.newComposition(name: 'Scene'));
      p.uiState.playheadFrame.value = 12;
      await tester.runAsync(() async {
        await saveProjectFrb(p.state, p.uiState,
            forcePicker: true, picker: () async => scratch.project);
        // The picture is deliberately not awaited by the save, so let
        // the road it was sent down finish before reading the folder.
        await Future<void>.delayed(const Duration(milliseconds: 200));
      });

      expect(asked, [12],
          reason: 'the engine was asked for the frame the playhead is on');
      expect(scratch.thumb.existsSync(), isTrue,
          reason: 'no Viewer is no longer no picture');
      expect(scratch.thumb.readAsBytesSync(), onePixelPng);
    });

    /// 19. **Opening a project with no picture grows one, once**. The
    /// backfill is what gives the owner's already-converted projects their
    /// rows back: they were saved before the engine could draw a thumbnail, so
    /// nothing but opening them will ever fill their wells.
    ///
    /// And it is genuinely once — a project that already has a picture is not
    /// redrawn on every open, because the picture it has is of the frame it was
    /// last *saved* at and a fresh one would be of frame 0.
    testWidgets('opening a project without a picture backfills it',
        (tester) async {
      final scratch = scratchProject('Converted');
      projectThumbnailCapture = () async => null;
      addTearDown(() => projectThumbnailCapture = captureViewerPicturePng);
      var drawn = 0;
      final wasEngine = Workspace.compThumbnailPng;
      Workspace.compThumbnailPng = (comp, frame) async {
        drawn++;
        return onePixelPng;
      };
      addTearDown(() => Workspace.compThumbnailPng = wasEngine);

      // A project on disk with a composition in it, and no picture filed —
      // which is every project saved before this road existed.
      final p = await mount(tester);
      p.state.project!.newComposition(name: 'Scene');
      await tester.runAsync(() => p.state.project!.save(path: scratch.project));
      if (scratch.thumb.existsSync()) scratch.thumb.deleteSync();

      final reopened = freshProject();
      await tester.runAsync(() async {
        await reopened.state.openProject(scratch.project);
        // The backfill is deliberately not awaited by the open, so let its
        // microtasks and the two bridge calls behind them run.
        await Future<void>.delayed(const Duration(milliseconds: 200));
      });

      expect(drawn, 1, reason: 'the missing picture was drawn');
      expect(scratch.thumb.existsSync(), isTrue);
      expect(scratch.thumb.readAsBytesSync(), onePixelPng);

      // Opened again, with a picture already on file: nothing is drawn.
      final again = freshProject();
      await tester.runAsync(() async {
        await again.state.openProject(scratch.project);
        await Future<void>.delayed(const Duration(milliseconds: 200));
      });
      expect(drawn, 1, reason: 'a project that has a picture keeps it');
    });
  }, skip: !engineAvailable);

  // --- The empty shell -----------------------------------------------------
  //
  // The welcome screen can be closed with nothing open, so something has to be
  // behind it. The two ways to start work stand in the Viewer until
  // something is displayed — the same three, from the same file, so they can
  // never drift apart.
  group('The empty stage (frb)', () {
    Future<({LumitState state, LumitUiState uiState})> mount(
      WidgetTester tester, {
      Future<String?> Function()? openPicker,
      bool withComposition = false,
    }) async {
      final p = freshProject();
      if (withComposition) p.state.project!.newComposition(name: 'Scene');
      await tester.pumpWidget(hostPanel(
        child: EmptyStageFrb(openPicker: openPicker),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      return p;
    }

    /// 18. **Nothing open, so the Viewer offers the two ways to start.** An
    /// empty editor whose largest panel says "select a composition" when there
    /// is no composition to select is a dead end.
    testWidgets('the two actions stand where the picture would be',
        (tester) async {
      await mount(tester);

      expect(find.byKey(const ValueKey('empty-stage')), findsOneWidget);
      expect(find.byKey(const ValueKey('welcome-card-new')), findsOneWidget);
      expect(find.byKey(const ValueKey('welcome-card-open')), findsOneWidget);
      expect(find.byKey(const ValueKey('welcome-card-blank')), findsNothing,
          reason: 'the same two the welcome offers, and no third');
      expect(find.text(l10n.selectACompositionFirst), findsNothing);
    });

    /// 19. **They go the moment there is something to show.** A project that
    /// has compositions and simply has none fronted is a different sentence,
    /// and keeps the panel's ordinary empty line.
    testWidgets('they go once the project has a composition', (tester) async {
      await mount(tester, withComposition: true);

      expect(find.byKey(const ValueKey('empty-stage')), findsNothing);
      expect(find.byKey(const ValueKey('welcome-card-new')), findsNothing);
      expect(find.text(l10n.selectACompositionFirst), findsOneWidget);
    });

    /// 20. **The actions are the welcome screen's own flows.** Open runs the
    /// same picker-then-read the File menu row runs; a cancelled one leaves the
    /// editor exactly as it was.
    testWidgets(
        'Open runs the same flow, and a cancelled picker changes '
        'nothing', (tester) async {
      var asked = 0;
      final p = await mount(tester, openPicker: () async {
        asked++;
        return null;
      });

      await tester.tap(find.byKey(const ValueKey('welcome-card-open')));
      await tester.pumpAndSettle();

      expect(asked, 1);
      expect(p.state.project!.path(), isNull);
      expect(find.byKey(const ValueKey('empty-stage')), findsOneWidget);
    });
  }, skip: !engineAvailable);
}
