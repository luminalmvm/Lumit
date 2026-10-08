// The shell surfaces on frb: Settings, recovery, the command palette.
//
// The Settings window and the recovery dialogue read the engine, so they run
// against it. The palette's ranking is pure and is tested as a function, because
// what matters about it is which command comes first — not how it is drawn.

import 'dart:io';
import 'dart:ui' show AppExitResponse;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/command_palette_frb.dart';
import 'package:lumit_flutter/shell/splash.dart' show bootLines;
import 'package:lumit_flutter/shell/export_dialog_frb.dart';
import 'package:lumit_flutter/shell/export_queue_frb.dart';
import 'package:lumit_flutter/shell/recovery_dialog_frb.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/shell/status_line_frb.dart';
import 'package:lumit_flutter/shell/unsaved_changes_frb.dart';
import 'package:lumit_flutter/shell/welcome_frb.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/retime.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Command palette ranking', () {
    test('a subsequence matches, and an absent letter does not', () {
      expect(paletteScore('nc', 'New composition'), isNotNull,
          reason: 'initials are the point of a palette');
      expect(paletteScore('', 'anything'), 0, reason: 'empty matches all');
      expect(paletteScore('zzz', 'New composition'), isNull);
      expect(paletteScore('NC', 'new composition'), isNotNull,
          reason: 'matching ignores case both ways');
    });
  });

  group('Settings window (frb)', () {
    testWidgets('it reads the engine and its buttons reach it', (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-settings'),
            onPressed: () => showSettingsWindowFrb(context),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-settings')));
      await tester.pumpAndSettle();

      // General opens first. What this build *is* is no longer stated here —
      // that is Help ▸ About Lumit now; Settings is for what you
      // change, and a version number is not that.
      expect(find.textContaining('lumit-bridge'), findsNothing);
      expect(find.byKey(const ValueKey('settings-reset-workspace')),
          findsOneWidget);

      // The engine's own readouts and buttons live on Performance.
      await tester
          .tap(find.byKey(const ValueKey('settings-page-previewAndCache')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('settings-tier')), findsOneWidget);
      expect(find.byKey(const ValueKey('settings-cache-used')), findsOneWidget);

      // The budget is a typed number now, not a pick from a list:
      // dragging it changes what the engine holds, not just the label.
      final before = cacheStats().budgetBytes.toInt();
      await tester.drag(find.byKey(const ValueKey('settings-cache-budget')),
          const Offset(60, 0));
      await tester.pumpAndSettle();
      expect(cacheStats().budgetBytes.toInt(), greaterThan(before),
          reason: 'the drag reached the engine');

      await tester.tap(find.byKey(const ValueKey('settings-cache-clear')));
      await tester.pump();
      expect(cacheStats().entries.toInt(), 0);

      await tester.tap(find.byKey(const ValueKey('settings-tier-reset')));
      await tester.pump();
      expect(playbackTier().tier, 1);

      // Where the memory has gone, at the foot of the page: the rows
      // above each report one store, and this reports the whole process and
      // what none of them accounts for. Scrolled to, because the page is
      // taller than the window — and a memory report is a thing you go and
      // look for.
      final unaccounted =
          find.byKey(const ValueKey('settings-memory-unaccounted'));
      // The page's own scrollable, named rather than taken as the first in the
      // tree: the title strip's search field carries one of its own.
      await tester.scrollUntilVisible(unaccounted, 200,
          scrollable: find
              .descendant(
                of: find.byKey(const ValueKey('settings-body-previewAndCache')),
                matching: find.byType(Scrollable),
              )
              .first);
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('settings-memory-process')),
          findsOneWidget);
      expect(unaccounted, findsOneWidget);
      expect(find.byKey(const ValueKey('settings-memory-gpu')), findsOneWidget);
      expect(find.byKey(const ValueKey('settings-memory-decoders')),
          findsOneWidget);
      // A real number, not a placeholder: the platform under the test answers
      // its own size, so the row shows bytes rather than an em dash.
      expect(
        tester.widget<Text>(unaccounted).data ?? '',
        anyOf(contains('MB'), contains('GB')),
        reason: 'the report is wired to the engine, not a stub',
      );
    });

    /// The disk tier's controls: its budget reaches the engine, and where the
    /// frames go is a choice the settings file remembers (docs/07 §15). The
    /// folder picker itself cannot open in a widget test, so what is checked is
    /// that choosing the custom location offers it — and that the two locations
    /// which need no folder take effect on the spot.
    testWidgets('the disk cache has a budget and a place to live',
        (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-settings'),
            onPressed: () => showSettingsWindowFrb(context),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-settings')));
      await tester.pumpAndSettle();
      await tester
          .tap(find.byKey(const ValueKey('settings-page-previewAndCache')));
      await tester.pumpAndSettle();
      // The page is a lazy list and the disk tier is the last group on it, so
      // it has to be scrolled to before it exists at all.
      await tester.drag(
          find.byKey(const ValueKey('settings-body-previewAndCache')),
          const Offset(0, -400));
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('settings-disk-used')), findsOneWidget);
      final before = diskCacheStats().budgetBytes.toInt();
      await tester.drag(find.byKey(const ValueKey('settings-disk-budget')),
          const Offset(60, 0));
      await tester.pumpAndSettle();
      expect(diskCacheStats().budgetBytes.toInt(), greaterThan(before),
          reason: 'the drag reached the engine');
      expect(p.uiState.workspace.performance.diskBudgetBytes,
          diskCacheStats().budgetBytes.toInt(),
          reason: 'and the settings file remembers it for the next launch');

      // No folder is needed to sit beside the project, so that choice is live
      // immediately and is written down by name.
      await tester.tap(find.byKey(const ValueKey('settings-disk-location')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Beside the project').last);
      await tester.pumpAndSettle();
      expect(p.uiState.workspace.performance.diskCacheLocation,
          BridgeCacheLocation.besideProject.name);

      // The custom location grows a Choose… button beside the dropdown; the
      // others do not, because they have nothing to choose.
      expect(find.byKey(const ValueKey('settings-disk-folder')), findsNothing);
      await tester.tap(find.byKey(const ValueKey('settings-disk-location')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('A folder I choose').last);
      await tester.pumpAndSettle();
      expect(
          find.byKey(const ValueKey('settings-disk-folder')), findsOneWidget);

      // Leave the engine on its default, since the location is process-wide.
      setDiskCacheLocation(location: BridgeCacheLocation.appData, folder: '');
    });

    /// **Nothing was lost in the rebuild.** The window was taken apart
    /// and put back to a new drawing with six pages instead of five, and every
    /// control it hosted has to still be somewhere. This walks the pages and
    /// names them: a setting dropped on the way would fail here rather than be
    /// found missing by whoever wanted it.
    testWidgets('every setting the window hosts is on one of its pages',
        (tester) async {
      const pages = <String, List<String>>{
        'general': [
          'settings-language',
          'settings-reset-workspace',
          'settings-auto-update',
          'settings-check-updates',
        ],
        'appearance': [
          'settings-scheme',
          'settings-theme-swatches',
          'settings-shape-studio',
          'settings-shape-desk',
          'settings-desk-room-grey',
          'settings-desk-room-graphite',
          'settings-shape-lantern',
          'settings-lantern-room',
          'settings-customise',
          'settings-theme-duplicate',
          'settings-theme-rename',
          'settings-theme-delete',
          'settings-theme-import',
          'settings-theme-export',
          'settings-ui-scale',
          'settings-ui-scale-value',
          'settings-tooltips',
          'settings-animation',
          'settings-compact',
          'settings-themed-scopes',
          'settings-themed-surround',
          'settings-viewer-bars',
          'settings-tool-bar-position',
          'settings-range-sliders',
          'settings-command-box',
          'settings-icon-set',
          'settings-icons-reload',
          'settings-multiwave',
          'settings-waveform-from-bottom',
        ],
        'timeline': [
          'settings-retime-speed-lens',
          'settings-retime-in-seconds',
          'settings-video-as-sequence',
          'settings-paste-at-original-time',
          'settings-playhead-stays',
          'settings-transform-in-fx',
          'settings-easing-in-popup',
          'settings-right-click-node-search',
          'settings-tab-node-search',
          'settings-shift-a-node-search',
        ],
        'viewer': [
          'settings-smooth-zoomed-viewer',
          'settings-show-tone-map',
        ],
        // The runtime row's own buttons are left out: which of Install, Load
        // and Remove is drawn depends on what the machine running the suite has
        // installed, and `settings_addons_frb_test` asserts that pair instead.
        'addons': [
          'settings-addon-check',
          'settings-addon-install-from-file',
          'settings-addon-show-folder',
        ],
        'previewAndCache': [
          'settings-playback-mode',
          'settings-tier-reset',
          'settings-cache-budget',
          'settings-cache-clear',
          'settings-vram-budget',
          'settings-vram-clear',
          'settings-disk-budget',
          'settings-disk-location',
          'settings-disk-scope',
          'settings-disk-clear',
        ],
        'shortcuts': [
          'keymap-preset-lumit',
          'keymap-preset-ae',
          'keymap-import',
          'keymap-export',
        ],
      };

      final p = freshProject();
      tester.view.physicalSize = const Size(1400, 1000);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-settings'),
            onPressed: () => showSettingsWindowFrb(context),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-settings')));
      await tester.pumpAndSettle();

      for (final page in pages.entries) {
        await tester
            .tap(find.byKey(ValueKey<String>('settings-page-${page.key}')));
        await tester.pumpAndSettle();
        for (final control in page.value) {
          final finder = find.byKey(ValueKey<String>(control));
          // The page is a lazy list, so a row below the fold is not built
          // until it is scrolled to. Scroll to it rather than asserting the
          // window happens to be tall enough for every page — which is what
          // it did until the Appearance page grew a row.
          if (finder.evaluate().isEmpty) {
            await tester.scrollUntilVisible(finder, 120,
                scrollable: find.byType(Scrollable).last);
            await tester.pumpAndSettle();
          }
          expect(finder, findsOneWidget,
              reason: '$control belongs to the ${page.key} page');
          // The room rows only exist while their shape is in force, so the
          // Desk and Lantern chips are pressed on the way past them.
          if (control == 'settings-shape-desk' ||
              control == 'settings-shape-lantern') {
            await tester.tap(finder);
            await tester.pumpAndSettle();
          }
        }
      }

      // And the frame's own three, on every page.
      expect(find.byKey(const ValueKey('settings-search')), findsOneWidget);
      expect(find.byKey(const ValueKey('settings-reset-page')), findsOneWidget);
      expect(find.byKey(const ValueKey('settings-close')), findsOneWidget);
    });

    testWidgets('the appearance controls change the shell theme',
        (tester) async {
      final p = freshProject();
      expect(p.uiState.scheme, LumitColorScheme.dark);

      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-settings'),
            onPressed: () => showSettingsWindowFrb(context),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-settings')));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('settings-page-appearance')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('settings-scheme')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Light').last);
      await tester.pumpAndSettle();

      expect(p.uiState.scheme, LumitColorScheme.light);
      expect(p.uiState.theme.mode, isNot(ThemeMode2.dark),
          reason: 'the derived theme follows the choice');

      // Desk's rooms are a row of their own: picking the shape leaves the
      // scheme alone, and a room chip is the one tap that changes it.
      await tester.tap(find.byKey(const ValueKey('settings-shape-desk')));
      await tester.pumpAndSettle();
      expect(p.uiState.scheme, LumitColorScheme.light,
          reason: 'picking Desk never changes the scheme by itself');
      await tester
          .tap(find.byKey(const ValueKey('settings-desk-room-graphite')));
      await tester.pumpAndSettle();
      expect(p.uiState.scheme, LumitColorScheme.graphite);
      expect(p.uiState.theme.accent, const Color(0xffe8712a));
    });
  }, skip: !engineAvailable);

  group('Recovery (frb)', () {
    testWidgets('an autosave beside the project offers the three choices',
        (tester) async {
      final p = freshProject();
      p.state.project!.newComposition(name: 'Scene');

      final dir = Directory.systemTemp.createTempSync('lumit-recover-some');
      final path = '${dir.path}/scene.lum';
      // A real autosave, written by the engine, so the listing is genuine.
      p.state.project!.autosave(projectPath: path, keep: 3);
      expect(listAutosaves(project: path), hasLength(1));

      await tester.pumpWidget(hostPanel(
        child: Builder(builder: (context) {
          return HouseButton(
            key: const ValueKey('recover'),
            onPressed: () => showRecoveryDialogFrb(
              context: context,
              state: p.state,
              projectPath: path,
            ),
            child: const Text('Recover'),
          );
        }),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('recover')));
      await tester.pumpAndSettle();

      // The title is a kicker now the dialogue wears the shared frame, and
      // Studio draws a kicker as written.
      expect(find.text('Recover work'), findsOneWidget);
      expect(find.byKey(const ValueKey('recover-journal')), findsOneWidget);
      expect(find.byKey(const ValueKey('recover-autosave')), findsOneWidget);
      expect(find.byKey(const ValueKey('recover-discard')), findsOneWidget);

      // Not restoring leaves everything where it is — the copies are not
      // deleted.
      await tester.tap(find.byKey(const ValueKey('recover-discard')));
      await tester.pumpAndSettle();
      expect(listAutosaves(project: path), hasLength(1));
    });

    /// Each button is its own answer, and the close mark is none of
    /// them — the shape changed, what the dialogue can answer did not.
    ///
    /// *Restore all changes* is the case driven here because replaying the
    /// journal is synchronous engine work. Opening an autosave is not: it goes
    /// through `state.openProject`, whose future never completes in a widget
    /// test's fake-async zone.
    testWidgets('each button is its own answer', (tester) async {
      // Held outside `run` on purpose. A second `pumpWidget` in one test does
      // not re-root the tree under a modal-capable host, so the opener element
      // — and the closure inside it — is the first run's. The dialogue it
      // raises is freshly built either way, so what it answers is genuine; the
      // answer just has to land somewhere both runs can read.
      RecoveryChoice? choice;

      Future<RecoveryChoice?> run(
        String tempPrefix,
        Future<void> Function() act,
      ) async {
        choice = null;
        final p = freshProject();
        p.state.project!.newComposition(name: 'Scene');
        final dir = Directory.systemTemp.createTempSync(tempPrefix);
        final path = '${dir.path}/scene.lum';
        // A saved file to replay the journal onto — written outside the fake
        // clock, because saving is a real asynchronous call — and an autosave
        // beside it so the dialogue has something to offer at all.
        await tester.runAsync(() => p.state.project!.save(path: path));
        p.state.project!.autosave(projectPath: path, keep: 3);

        await tester.pumpWidget(hostPanel(
          child: Builder(builder: (context) {
            return HouseButton(
              key: const ValueKey('recover'),
              onPressed: () async {
                choice = await showRecoveryDialogFrb(
                  context: context,
                  state: p.state,
                  projectPath: path,
                );
              },
              child: const Text('Recover'),
            );
          }),
          state: p.state,
          uiState: p.uiState,
          size: const Size(700, 600),
        ));
        await tester.pump();
        await tester.tap(find.byKey(const ValueKey('recover')));
        await tester.pumpAndSettle();
        await act();
        await tester.pumpAndSettle();
        return choice;
      }

      // The filled, focused action: every change since the save.
      expect(
        await run('lumit-recover-journal', () async {
          await tester.tap(find.byKey(const ValueKey('recover-journal')));
        }),
        RecoveryChoice.journal,
      );

      // The leftmost: open the saved file as it is.
      expect(
        await run('lumit-recover-none', () async {
          await tester.tap(find.byKey(const ValueKey('recover-discard')));
        }),
        RecoveryChoice.discard,
      );

      // The close mark is no answer at all: the project opens as it was saved.
      expect(
        await run('lumit-recover-close', () async {
          await tester.tap(find.byKey(const ValueKey('recover-close')));
        }),
        isNull,
      );
    });
  }, skip: !engineAvailable);

  group('Unsaved changes (frb)', () {
    /// Guards lost work, and a window that can no longer be closed. New, Close
    /// project, Open, an import and quitting all pass the one check in
    /// LumitState, so New, Open and quitting stand for the rest here.
    testWidgets('leaving a project with unsaved changes asks first',
        (tester) async {
      tester.view.physicalSize = const Size(1800, 1100);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      final state = p.state;
      // The welcome screen is the window, and the question opens over it.
      await tester.pumpWidget(hostPanel(
        child: const BootGate(splash: false),
        state: state,
        uiState: p.uiState,
      ));
      await tester.pump();
      final question = find.byKey(const ValueKey('unsaved-save'));

      Future<AppExitResponse?> quit() async {
        AppExitResponse? answer;
        tester.binding.handleRequestAppExit().then((a) => answer = a);
        await tester.pumpAndSettle();
        return answer;
      }

      // A clean project goes at once, and quitting is not held up.
      final clean = state.project;
      state.newProject();
      await tester.pump();
      expect(identical(state.project, clean), isFalse);
      expect(question, findsNothing);
      expect(await quit(), AppExitResponse.exit);

      // With work in it, each way out asks, and Cancel changes nothing.
      state.project!.newComposition(name: 'Scene');
      final kept = state.project;
      Future<void> cancel() async {
        expect(question, findsOneWidget);
        await tester.tap(find.byKey(const ValueKey('unsaved-cancel')));
        await tester.pumpAndSettle();
        expect(identical(state.project, kept), isTrue);
        expect(kept!.isDirty(), isTrue);
        expect(state.comps(), hasLength(1));
      }

      state.newProject();
      await tester.pumpAndSettle();
      await cancel();

      state.openProject('nowhere.lum');
      await tester.pumpAndSettle();
      await cancel();
      expect(state.opening.value, isFalse);

      AppExitResponse? answer;
      tester.binding.handleRequestAppExit().then((a) => answer = a);
      await tester.pumpAndSettle();
      expect(answer, isNull, reason: 'the window waits for the answer');
      await cancel();
      expect(answer, AppExitResponse.cancel);

      // Save on a project with no file asks where. Backing out of that keeps
      // the project open.
      final dir = Directory.systemTemp.createTempSync('lumit-unsaved');
      addTearDown(() => dir.deleteSync(recursive: true));
      final path = '${dir.path}/scene.lum';
      String? picked;
      final gate = tester.element(find.byType(BootGate));
      state.askUnsaved = () => askUnsavedChangesFrb(gate, state, p.uiState,
          savePicker: () async => picked);
      state.newProject();
      await tester.pumpAndSettle();
      await tester.tap(question);
      await tester.pumpAndSettle();
      expect(identical(state.project, kept), isTrue);
      expect(question, findsNothing);

      // Saved for real, the project is on disk before it goes.
      picked = path;
      state.newProject();
      await tester.pumpAndSettle();
      await tester.tap(question);
      await settleFrb(tester, until: () => !identical(state.project, kept));
      expect(identical(state.project, kept), isFalse);
      expect(File(path).existsSync(), isTrue);

      // Discard lets it go, and so lets the window close.
      state.project!.newComposition(name: 'Scratch');
      tester.binding.handleRequestAppExit().then((a) => answer = a);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('unsaved-discard')));
      await tester.pumpAndSettle();
      expect(answer, AppExitResponse.exit);
    });
  }, skip: !engineAvailable);

  group('Status line (frb)', () {
    /// The strip stays empty while there is nothing to say, follows the
    /// export through running to its outcome, and offers Cancel only while
    /// something is actually cancellable. Driven through the injected poll,
    /// so no engine has to run a real export.
    ///
    /// The strip polls only while an export is live: each start is announced
    /// through [statusLineExportStarted], as the export dialogue and the
    /// snapshot do, and the poll follows the export to its outcome on its
    /// own from there. An idle strip makes no bridge calls at all.
    testWidgets('the status line follows an export through its states',
        (tester) async {
      var state = const BridgeExportState.idle();
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: StatusLineFrb(poll: () => state),
        state: p.state,
        uiState: p.uiState,
      ));

      await tester.pump(const Duration(milliseconds: 600));
      expect(find.byKey(const ValueKey('status-export-progress')), findsNothing,
          reason: 'idle says nothing');

      state = BridgeExportState.running(
          frame: BigInt.from(30), total: BigInt.from(120), encoder: 'x264');
      statusLineExportStarted.value++;
      await tester.pump(const Duration(milliseconds: 600));
      expect(find.textContaining('frame 30 of 120'), findsOneWidget);
      expect(find.byKey(const ValueKey('status-export-cancel')), findsOneWidget,
          reason: 'a running export can be cancelled from the strip');

      // No new signal: the poll that saw "running" keeps ticking until the
      // export leaves that state, so the outcome arrives on its own.
      state = const BridgeExportState.done(path: 'C:/out/final.mp4');
      await tester.pump(const Duration(milliseconds: 600));
      expect(find.textContaining('Exported to'), findsOneWidget);
      expect(find.byKey(const ValueKey('status-export-cancel')), findsNothing,
          reason: 'nothing to cancel any more');

      state = const BridgeExportState.failed(error: 'cancelled');
      statusLineExportStarted.value++;
      await tester.pump(const Duration(milliseconds: 600));
      expect(find.text('Export cancelled'), findsOneWidget);
    });

    /// The left end of the strip: whether the document is saved. Fails
    /// without the engine's `is_dirty` (saved_revision stamped on save).
    testWidgets('the saved state follows edits and saves', (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: StatusLineFrb(poll: () => const BridgeExportState.idle()),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      expect(find.text('Not saved yet'), findsOneWidget,
          reason: 'a fresh untouched project has nothing to lose');

      // The strip redraws on document notifications rather than a poll, so
      // the edit announces itself the way every edit in the application does.
      p.state.project!.newComposition(name: 'Scene');
      p.state.notifyDocumentChanged();
      await tester.pump(const Duration(milliseconds: 600));
      expect(find.text('Unsaved changes'), findsOneWidget);

      final dir = Directory.systemTemp.createTempSync('lumit-status');
      addTearDown(() => dir.deleteSync(recursive: true));
      // Not awaited: save is an async frb call, and its continuation only
      // lands on the real turns settleFrb provides.
      p.state.project!.save(path: '${dir.path}/probe.lum');
      await settleFrb(tester, until: () => !p.state.project!.isDirty());
      // As the application's own save path does once the write lands.
      p.state.notifyDocumentChanged();
      await tester.pump(const Duration(milliseconds: 600));
      expect(find.text('Saved'), findsOneWidget,
          reason: 'the save stamped the revision clean');
    });

    /// The notice area: the latest message shows with its close button, and
    /// closing it leaves the strip quiet.
    testWidgets('a notice shows in the strip until closed', (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: StatusLineFrb(poll: () => const BridgeExportState.idle()),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      expect(find.byKey(const ValueKey('status-notice')), findsNothing);

      p.state.postNotice('Could not open C:/gone.lum', error: true);
      await tester.pump();
      expect(find.byKey(const ValueKey('status-notice')), findsOneWidget);
      expect(find.textContaining('Could not open'), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('status-notice-close')));
      await tester.pump();
      expect(find.byKey(const ValueKey('status-notice')), findsNothing,
          reason: 'every notice carries its close button');
    });
  }, skip: !engineAvailable);

  group('Export dialog (frb)', () {
    /// Open the dialog over a fresh comp, in a view big enough for its frame.
    Future<void> open(
      WidgetTester tester, {
      Future<String?> Function()? picker,
      void Function(LumitState state, CompositionReference comp)? before,
    }) async {
      tester.view.physicalSize = const Size(1200, 1000);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addAdjustmentLayer();
      before?.call(p.state, comp);

      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-export'),
            onPressed: () => showExportDialogFrb(
                context: context, comp: comp, picker: picker),
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

    /// The dialog's fields default to the composition's own facts: the
    /// frame rate is the comp's, and the span is the work area exactly as the
    /// Timeline set it — already typed, not re-derived by the user.
    testWidgets('the rate and span default to the comp and its work area',
        (tester) async {
      await open(tester, before: (_, comp) {
        // A 60 fps comp with a work area over frames 60..180 (1 s .. 3 s).
        comp.setWorkArea(
          span: const BridgeSpan(
            inPoint: BridgeRational(num: 1, den: 1),
            outPoint: BridgeRational(num: 3, den: 1),
            startOffset: BridgeRational(num: 0, den: 1),
          ),
        );
      });

      expect(find.text('Frame rate'), findsOneWidget);
      expect(find.text('Composition · 60'), findsOneWidget,
          reason: "the rate starts as the comp's own 60");
      expect(find.text('Work area · 60–180'), findsOneWidget,
          reason: 'the span starts as the work area the Timeline set');
      expect(find.textContaining('120 frames'), findsOneWidget,
          reason: 'and the footer counts exactly those frames');

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// What a format can carry decides what is live: an mp4 has
    /// no alpha and only eight bits, a PNG sequence has both but no sound and
    /// no bitrate, and a WAV has no picture at all. Every one of those controls
    /// is **drawn** in each case — a control that vanished would leave the
    /// person wondering whether they had imagined it — and dead where the
    /// format cannot honour it.
    testWidgets('the format decides what is live and what is dead',
        (tester) async {
      await open(tester);

      // A dropdown's face is a HouseButton: no `onPressed` is the disabled
      // face, which is exactly what a format that cannot honour the row asks
      // for.
      bool live(String key) =>
          tester
              .widget<HouseButton>(find.descendant(
                of: find.byKey(ValueKey<String>(key)),
                matching: find.byType(HouseButton),
              ))
              .onPressed !=
          null;

      // An mp4: sound and a bitrate, no alpha and one depth.
      expect(live('export-audio'), isTrue);
      expect(live('export-channels'), isFalse,
          reason: 'no v1 codec in an mp4 carries alpha (docs/06 §7.4)');
      expect(live('export-depth'), isFalse, reason: 'an mp4 is eight bits');
      expect(
          tester
              .widget<HouseCheckbox>(
                  find.byKey(const ValueKey('export-bitrate-auto')))
              .value,
          isTrue,
          reason: 'the bitrate starts on Auto, which is the preset default');

      await tester.tap(find.byKey(const ValueKey('export-type-imageSequence')));
      await tester.pumpAndSettle();
      expect(live('export-channels'), isTrue, reason: 'stills carry alpha');
      expect(live('export-depth'), isTrue, reason: 'and either depth');
      expect(live('export-audio'), isFalse,
          reason: 'a folder of stills is mute');
      expect(find.textContaining('One numbered PNG per frame'), findsOneWidget,
          reason: 'the dialog says what a sequence writes');

      await tester.tap(find.byKey(const ValueKey('export-type-audioOnly')));
      await tester.pumpAndSettle();
      expect(live('export-audio'), isTrue);
      expect(live('export-channels'), isFalse,
          reason: 'a sound file has no picture to put channels in');
      // The rate and span stay whatever the format: every export has both.
      expect(find.byKey(const ValueKey('export-fps')), findsOneWidget);
      expect(find.byKey(const ValueKey('export-span')), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// A spec the format cannot carry is refused *here*, in the footer, before
    /// anything is queued — and the actions go inert until it is answered. The
    /// engine refuses the same thing as a backstop; the point of asking early
    /// is that the message arrives while the fields are still on screen.
    testWidgets('a refusal stands in the footer and holds the actions',
        (tester) async {
      final target = '${Directory.systemTemp.path}/refused.mp4';
      await open(tester, picker: () async => target);
      await tester.tap(find.byKey(const ValueKey('export-choose')));
      await tester.pumpAndSettle();
      expect(
          tester
              .widget<HouseButton>(find.byKey(const ValueKey('export-start')))
              .onPressed,
          isNotNull);

      // Sixteen bits in an mp4: the Depth row is dead for exactly this reason,
      // so the refusal is reached through a preset instead — a stored spec is
      // not filtered by the dialog on the way in.
      await tester.tap(find.byKey(const ValueKey('export-type-imageSequence')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('export-depth')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('16 bit').last);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('export-type-video')));
      await tester.pumpAndSettle();

      expect(find.textContaining('16-bit'), findsOneWidget,
          reason:
              "the engine's own words, in the footer where the summary was");
      expect(
          tester
              .widget<HouseButton>(find.byKey(const ValueKey('export-start')))
              .onPressed,
          isNull,
          reason: 'nothing is queued that the file cannot carry');

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// A preset is the whole settings payload under a name: saving one lists
    /// it, applying it fills the fields back in, and deleting it takes it off
    /// the list. The built-ins are read-only and say so.
    testWidgets('a preset saves, applies and is forgotten again',
        (tester) async {
      await open(tester);

      // A built-in refuses to be edited rather than opening a field that
      // cannot be used.
      await tester.tap(find.byKey(const ValueKey('export-preset-edit')));
      await tester.pumpAndSettle();
      expect(find.textContaining('built-in'), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('export-type-imageSequence')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('export-preset-save-as')));
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey('export-preset-name')), 'Test stills');
      await tester.tap(find.byKey(const ValueKey('export-preset-save')));
      await tester.pumpAndSettle();

      // Back to a video export, then the preset puts the stills back.
      await tester.tap(find.byKey(const ValueKey('export-type-video')));
      await tester.pumpAndSettle();
      expect(find.text('H.264 video (.mp4)'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('export-preset')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Test stills').last);
      await tester.pumpAndSettle();
      expect(find.text('PNG image sequence'), findsOneWidget,
          reason: 'the preset carried the format it was saved with');

      // And it can be taken off the list again.
      await tester.tap(find.byKey(const ValueKey('export-preset-edit')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('export-preset-delete')));
      await tester.pumpAndSettle();
      expect(exportPresetList().any((p) => p.name == 'Test stills'), isFalse);

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// Every row that was drawn dead is a setting now. Changing each one and
    /// saving the result as a preset is the assertion, because a preset is the
    /// whole settings payload: what comes back out of the store is what the
    /// dialog put into the spec.
    testWidgets('the rows that were drawn dead now reach the spec',
        (tester) async {
      await open(tester);

      /// Pick an option out of a list by the words on it. The menu is drawn
      /// over the page, so the last match is the one in the menu rather than
      /// the closed face of some other row.
      Future<void> pick(String id, String option) async {
        await tester.tap(find.byKey(ValueKey<String>(id)));
        await tester.pumpAndSettle();
        await tester.tap(find.text(option).last);
        await tester.pumpAndSettle();
      }

      await pick('export-proxies', 'Use all proxies');
      await pick('export-guide-layers', 'Current settings');
      await pick('export-motion-blur', 'Off for all layers');
      await pick('export-retime-blend', 'Off for all layers');
      await pick('export-resample', 'High');
      await pick('export-colour-space', 'Rec. 2020');
      await pick('export-audio-sample-rate', '44.100 kHz');
      await pick('export-audio-layout', 'Mono');

      await tester.tap(find.byKey(const ValueKey('export-preset-save-as')));
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey('export-preset-name')), 'Live rows');
      await tester.tap(find.byKey(const ValueKey('export-preset-save')));
      await tester.pumpAndSettle();

      final stored = exportPresetGet(name: 'Live rows')!;
      expect(stored.useProxies, isTrue);
      expect(stored.renderGuides, isTrue);
      expect(stored.motionBlur, 2, reason: 'off for all layers is the third');
      expect(stored.retimeBlend, 1, reason: 'and the second of two');
      expect(stored.resample, 'high');
      expect(stored.colourSpace, 'rec2020',
          reason: 'the space crosses as its stored name, not its label');
      expect(stored.audioRate, 44100);
      expect(stored.audioChannels, 1, reason: 'one channel is the fold-down');

      exportPresetDelete(name: 'Live rows');
      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// A layer whose Flow engine names a model this machine cannot run stops
    /// the export in the footer, because a file is a thing the user keeps and
    /// docs/08 §3.1 says an export never quietly downgrades. Preview is the
    /// half that substitutes and says so; this half refuses
    /// (docs/impl/addons.md §6.3).
    testWidgets('an export that needs an addon this machine lacks is refused',
        (tester) async {
      final target = '${Directory.systemTemp.path}/needs-an-addon.mp4';
      CompositionReference? subject;
      await open(tester, picker: () async => target, before: (state, comp) {
        subject = comp;
        final footage = state.project!.importFootage(path: 'C:/c/shot.mov');
        comp.addFootageLayer(footage: footage, asSequence: false);
        final layer = comp.getLayers().last;
        layer.setFlowEnabled(on_: true);
        layer.setFlowParams(
          params: BridgeFlowParams(
            engine: 1,
            resolution: 0,
            detail: 1,
            smoothness: 50,
            occlusion: 0,
            fallback: 0,
            hudGuard: true,
            always: false,
          ),
        );
      });
      await tester.tap(find.byKey(const ValueKey('export-choose')));
      await tester.pumpAndSettle();

      // On a machine with the pack installed there is nothing to refuse, so
      // Export isn't pressed at all. A real export started here is still
      // winding down when the next test presses Export.
      if (!subject!.addonNeeded()) {
        await tester.tap(find.byKey(const ValueKey('export-close')));
        await tester.pumpAndSettle();
        return;
      }

      await tester.tap(find.byKey(const ValueKey('export-start')));
      await tester.pumpAndSettle();

      expect(find.text('An addon this project needs is not installed'),
          findsOneWidget,
          reason: 'the footer says what is wrong, in the reader\'s language');
      expect(find.byKey(const ValueKey('export-open-addons')), findsOneWidget,
          reason: 'and the page that mends it is one press away');
      expect(exportQueueList().where((i) => i.path == target), isEmpty,
          reason: 'nothing was queued and nothing was written');
      expect(find.text('Export queue'), findsNothing,
          reason: 'the dialogue stays up, holding what was typed into it');

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// Both footer actions queue the export — the difference is whether the
    /// queue runs — and the queue window opens on top, so nothing is ever
    /// started somewhere the user cannot see it.
    testWidgets('Add to queue queues without starting, and shows the queue',
        (tester) async {
      final target = '${Directory.systemTemp.path}/queued.mp4';
      await open(tester, picker: () async => target);
      await tester.tap(find.byKey(const ValueKey('export-choose')));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('export-add-to-queue')));
      await tester.pumpAndSettle();

      expect(find.text('Export queue'), findsOneWidget,
          reason: 'the queue window opens over the closed dialog');
      final queued = exportQueueList().where((i) => i.path == target).toList();
      expect(queued, hasLength(1), reason: "the item is on the engine's list");
      expect(queued.single.state, isA<BridgeExportQueueState_Waiting>(),
          reason: 'Add to queue adds; it does not start');
      expect(queued.single.compName, 'Scene',
          reason: "the row carries the comp's name as it was at queue time");

      // And the row can be taken off again from the window.
      await tester.tap(find
          .byKey(ValueKey<String>('export-queue-drop-${queued.single.id}')));
      await tester.pumpAndSettle();
      expect(exportQueueList().where((i) => i.path == target), isEmpty);

      await tester.tap(find.byKey(const ValueKey('export-queue-dismiss')));
      await tester.pumpAndSettle();
    });

    /// EXPORT is the same call with the queue let loose: the item leaves
    /// Waiting the moment the window opens, and whatever the machine can
    /// actually do — encode it, or refuse for want of a GPU — the queue says
    /// so calmly and the row can be taken off again.
    ///
    /// Last in the group deliberately: it leaves the process-wide queue
    /// *running*, which is exactly what a test asserting "Add to queue does
    /// not start" must not have happen to it first.
    testWidgets('EXPORT starts the queue and reports whatever happens',
        (tester) async {
      final target = '${Directory.systemTemp.path}/exported.mp4';
      await open(tester, picker: () async => target);
      await tester.tap(find.byKey(const ValueKey('export-choose')));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('export-start')));
      await tester.pumpAndSettle(const Duration(milliseconds: 400));

      final item = exportQueueList().firstWhere((i) => i.path == target);
      expect(item.state, isNot(isA<BridgeExportQueueState_Waiting>()),
          reason: 'Export lets the queue run rather than leaving it waiting');
      expect(
          item.state,
          anyOf(
            isA<BridgeExportQueueState_Running>(),
            isA<BridgeExportQueueState_Done>(),
            isA<BridgeExportQueueState_Failed>(),
          ),
          reason: 'it either runs or explains itself — never neither');

      exportQueueCancel(id: item.id);
      exportQueueRemove(id: item.id);
      await tester.tap(find.byKey(const ValueKey('export-queue-dismiss')));
      await tester.pumpAndSettle();
      File(target).existsSync() ? File(target).deleteSync() : null;
    });
  }, skip: !engineAvailable);

  /// The queue's order is the order the exports run in, so it is draggable —
  /// with the application's own reorder gesture, and only for what is still
  /// waiting (the engine refuses a row that is running, has run, or has gone).
  /// The list and the move are both injected, because what is asserted here is
  /// the window's gesture rather than the engine's slot.
  group('Export queue reorder (frb)', () {
    BridgeExportQueueItem item(int id, String name,
            {BridgeExportQueueState state =
                const BridgeExportQueueState.waiting()}) =>
        BridgeExportQueueItem(
          id: id,
          compName: name,
          path: 'C:/exports/$name.mp4',
          preset: '',
          codec: 'h264',
          rangeStartFrame: -1,
          rangeEndFrame: -1,
          state: state,
        );

    /// Drag one row by [dy], in steps rather than one jump: a lift needs a
    /// frame between the moves to follow the pointer, and the row is grabbed
    /// at its own centre — which is bare between the columns, and is the whole
    /// reason the row is an opaque hit target.
    Future<void> dragRow(WidgetTester tester, String key, double dy) async {
      final gesture = await tester
          .startGesture(tester.getCenter(find.byKey(ValueKey(key))));
      await tester.pump();
      for (var step = 0; step < 6; step++) {
        await gesture.moveBy(Offset(0, dy / 6));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();
    }

    /// The drag is carried at Full and marked with a line below it (docs/15
    /// §8.1). Either way the row lands where it was dropped.
    for (final level in AnimationLevel.values) {
      testWidgets('a waiting row is dragged to another place, at ${level.name}',
          (tester) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final items = [item(1, 'One'), item(2, 'Two'), item(3, 'Three')];
      final moves = <(int, int)>[];
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        animationLevel: level,
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-queue'),
            onPressed: () => showExportQueueFrb(
              context: context,
              list: () => List.of(items),
              move: ({required int id, required int index}) {
                moves.add((id, index));
                final row = items.removeAt(items.indexWhere((i) => i.id == id));
                items.insert(index.clamp(0, items.length), row);
              },
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

      // Held over the first row, the line stands on the edge the row would
      // land against, and nothing has moved. Only where the drag is marked.
      if (level != AnimationLevel.all) {
        final held = await tester.startGesture(tester
            .getCenter(find.byKey(const ValueKey('export-queue-item-3'))));
        await tester.pump();
        for (var step = 0; step < 6; step++) {
          await held.moveBy(const Offset(0, -2 * exportQueueRow / 6));
          await tester.pump();
        }
        final line = find.byKey(const ValueKey('export-queue-drop-line'));
        expect(line, findsOneWidget);
        expect(
            find.descendant(
                of: line,
                matching: find.byKey(const ValueKey('export-queue-item-1'))),
            findsOneWidget,
            reason: 'on the row it is aimed at');
        final border =
            (tester.widget<DecoratedBox>(line).decoration as BoxDecoration)
                .border! as Border;
        expect(border.top.width, 2, reason: 'over it: the row came from below');
        expect(border.bottom, BorderSide.none);
        expect(moves, isEmpty, reason: 'nothing moves until the drop');
        await held.cancel();
        await tester.pump();
        expect(line, findsNothing);
      }

      // The last row, dragged up onto the first. In steps, because a drag
      // reported as one jump is consumed starting the gesture and lands the
      // avatar back where it began.
      await dragRow(tester, 'export-queue-item-3', -2 * exportQueueRow);

      expect(moves, [(3, 0)],
          reason: 'the row that was dragged, and the place it was dropped on');
      expect(items.map((i) => i.id), [3, 1, 2],
          reason: 'and the list came back in the new order');

      await tester.tap(find.byKey(const ValueKey('export-queue-dismiss')));
      await tester.pumpAndSettle();
    });
    }

  }, skip: !engineAvailable);

  /// The boot splash is the window until boot ends, and the welcome screen is
  /// the window after it: the shell must not be in the tree behind either, or
  /// the first-run question would open underneath a screen nothing can be
  /// clicked through.
  group('The boot splash', () {
    testWidgets('is the whole window, and hands over to the welcome screen',
        (tester) async {
      tester.view.physicalSize = const Size(1800, 1100);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: const BootGate(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump(const Duration(milliseconds: 250));

      expect(find.text('Lumit'), findsOneWidget, reason: 'the splash is up');
      expect(find.byType(WelcomeScreenFrb), findsNothing);
      expect(find.byType(LumitAppView), findsNothing,
          reason: 'and nothing of the application is behind it');
      // The engine's own first line, not the canned fallback: with a bridge
      // loaded the log is what the splash streams.
      expect(find.text(bootLines.first), findsNothing);
      expect(find.text(bootLog().first), findsOneWidget);

      await tester.pumpAndSettle();
      expect(find.byType(WelcomeScreenFrb), findsOneWidget,
          reason: 'boot over, the welcome screen takes the window');
      expect(find.byType(LumitAppView), findsNothing,
          reason: 'and the shell is still not behind it');

      // New project is the way straight through.
      await tester.tap(find.byKey(const ValueKey('welcome-card-new')));
      await tester.pumpAndSettle();
      expect(find.byType(LumitAppView), findsOneWidget);
    });

  }, skip: !engineAvailable);
}
