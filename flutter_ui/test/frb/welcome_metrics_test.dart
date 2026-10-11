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

@Tags(['opens-project'])
library;

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
import 'package:lumit_flutter/shell/welcome_frb.dart';
import 'package:lumit_flutter/state/workspace.dart';

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

    // --- The ruler --------------------------------------------------------

    // --- What it does -----------------------------------------------------

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
      // Let the open end, so the timer that offers its Cancel is not left.
      await settleFrb(tester, until: () => !p.state.opening.value);
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

  }, skip: !engineAvailable);
}
