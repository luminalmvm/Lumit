// The shell surfaces on frb: Settings, recovery, the command palette.
//
// The Settings window and the recovery dialogue read the engine, so they run
// against it. The palette's ranking is pure and is tested as a function, because
// what matters about it is which command comes first — not how it is drawn.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/cache_confirm_frb.dart';
import 'package:lumit_flutter/shell/command_palette_frb.dart';
import 'package:lumit_flutter/shell/splash.dart' show bootLines;
import 'package:lumit_flutter/shell/export_dialog_frb.dart';
import 'package:lumit_flutter/shell/export_queue_frb.dart';
import 'package:lumit_flutter/shell/recovery_dialog_frb.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/shell/status_line_frb.dart';
import 'package:lumit_flutter/shell/welcome_frb.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
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

    test('an earlier, tighter match ranks first', () {
      // "comp" is contiguous and early in one, late in the other.
      final settings = paletteScore('comp', 'Composition settings')!;
      final created = paletteScore('comp', 'New composition')!;
      expect(settings, lessThan(created));
    });

    test('a scattered match ranks below a contiguous one', () {
      final tight = paletteScore('save', 'Save as…')!;
      final loose = paletteScore('save', 'Show all viewer edges')!;
      expect(tight, lessThan(loose));
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
      // that is Help ▸ About Lumit now (K-244); Settings is for what you
      // change, and a version number is not that.
      expect(find.textContaining('lumit-bridge'), findsNothing);
      expect(find.byKey(const ValueKey('settings-reset-workspace')),
          findsOneWidget);

      // The engine's own readouts and buttons live on Performance (K-193).
      await tester
          .tap(find.byKey(const ValueKey('settings-page-previewAndCache')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('settings-tier')), findsOneWidget);
      expect(find.byKey(const ValueKey('settings-cache-used')), findsOneWidget);

      // The budget is a typed number now (K-194), not a pick from a list:
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

      // Where the memory has gone (K-294), at the foot of the page: the rows
      // above each report one store, and this reports the whole process and
      // what none of them accounts for. Scrolled to, because the page is
      // taller than the window — and a memory report is a thing you go and
      // look for.
      final unaccounted =
          find.byKey(const ValueKey('settings-memory-unaccounted'));
      // The page's own scrollable, named rather than taken as the first in the
      // tree: the title strip's search field carries one of its own (K-465).
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

    /// **A project can be told to cache somewhere of its own** (docs/06 §5.4).
    /// The scope control decides where the answer is kept: in the settings file
    /// for every project, or inside this `.lum` so it travels with a copy of it.
    /// Switching back to Everything clears the project's override rather than
    /// copying the application's answer into it, so the project follows along
    /// afterwards.
    testWidgets('a project can keep its own cache location', (tester) async {
      final p = freshProject();
      expect(p.state.project!.cacheLocation(), isNull,
          reason: 'a fresh project follows the application');

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
      await tester.drag(
          find.byKey(const ValueKey('settings-body-previewAndCache')),
          const Offset(0, -400));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('settings-disk-scope')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('This project').last);
      await tester.pumpAndSettle();
      expect(p.state.project!.cacheLocation(), isNotNull,
          reason: 'the project now carries its own answer');

      // And it is a document change, so it undoes like any other edit.
      p.state.project!.undo();
      expect(p.state.project!.cacheLocation(), isNull);
    });

    /// **Clearing the disk tier asks first.** The other two cost a re-render;
    /// this one deletes files that may be a night's work, and there is nothing
    /// to undo. With nothing parked there is nothing to ask about, so no
    /// dialogue appears — a question about deleting nothing is only noise.
    testWidgets('clearing the disk cache asks before deleting', (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('clear-disk'),
            onPressed: () => confirmClearDiskCache(context),
            child: const Text('Clear'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('clear-disk')));
      await tester.pumpAndSettle();

      if (diskCacheStats().entries == BigInt.zero) {
        expect(find.byKey(const ValueKey('disk-clear-confirm')), findsNothing,
            reason: 'nothing parked, so nothing to ask about');
        return;
      }
      expect(find.byKey(const ValueKey('disk-clear-confirm')), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('disk-clear-cancel')));
      await tester.pumpAndSettle();
      expect(diskCacheStats().entries, isNot(BigInt.zero),
          reason: 'saying no keeps the frames');
    });

    /// The pages are the point of the window: each shows its own settings and
    /// only its own, and a preference edited on one sticks (K-193).
    testWidgets('the pages divide the settings, and a choice persists',
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

      // General is open: nothing from another page is on screen with it.
      expect(find.byKey(const ValueKey('settings-reset-workspace')),
          findsOneWidget);
      expect(find.byKey(const ValueKey('settings-scheme')), findsNothing);
      expect(find.byKey(const ValueKey('settings-cache-budget')), findsNothing);

      await tester.tap(find.byKey(const ValueKey('settings-page-timeline')));
      await tester.pumpAndSettle();
      expect(
          find.byKey(const ValueKey('settings-reset-workspace')), findsNothing);

      // The Transform card's toggle: off by default, and it stays where it
      // is put (K-193).
      expect(p.uiState.workspace.interface.transformInEffectControls, isFalse,
          reason: 'the Effect controls panel is about effects by default');
      await tester.tap(find.byKey(const ValueKey('settings-transform-in-fx')));
      await tester.pumpAndSettle();
      expect(p.uiState.workspace.interface.transformInEffectControls, isTrue);
    });

    /// **Nothing was lost in the rebuild** (K-465). The window was taken apart
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
          'settings-shape-sharp',
          'settings-shape-round',
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
        ],
        'viewer': [
          'settings-smooth-zoomed-viewer',
          'settings-show-tone-map',
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
        }
      }

      // And the frame's own three, on every page.
      expect(find.byKey(const ValueKey('settings-search')), findsOneWidget);
      expect(find.byKey(const ValueKey('settings-reset-page')), findsOneWidget);
      expect(find.byKey(const ValueKey('settings-close')), findsOneWidget);
    });

    /// The search hides the rows whose names do not match, and says so when it
    /// has hidden all of them.
    testWidgets('the title strip\'s search filters the page', (tester) async {
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
      await tester.tap(find.byKey(const ValueKey('settings-page-appearance')));
      await tester.pumpAndSettle();

      await tester.enterText(
          find.byKey(const ValueKey('settings-search')), 'accent');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('settings-accent-hex')), findsOneWidget);
      expect(find.byKey(const ValueKey('settings-scheme')), findsNothing,
          reason: 'a row whose name does not match is hidden');

      await tester.enterText(
          find.byKey(const ValueKey('settings-search')), 'zzz');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('settings-no-matches')), findsOneWidget);
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
    });
  }, skip: !engineAvailable);

  group('Recovery (frb)', () {
    testWidgets('with nothing beside the project no dialogue is offered',
        (tester) async {
      final p = freshProject();
      final dir = Directory.systemTemp.createTempSync('lumit-recover-none');
      late RecoveryChoice? choice;

      await tester.pumpWidget(hostPanel(
        child: Builder(builder: (context) {
          return HouseButton(
            key: const ValueKey('recover'),
            onPressed: () async {
              choice = await showRecoveryDialogFrb(
                context: context,
                state: p.state,
                projectPath: '${dir.path}/scene.lum',
              );
            },
            child: const Text('Recover'),
          );
        }),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('recover')));
      await tester.pumpAndSettle();

      expect(choice, isNull,
          reason: 'a project with nothing to recover raises no dialogue');
      expect(find.textContaining('RECOVER WORK'), findsNothing);
    });

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

      // The title is a kicker now the dialogue wears the shared frame, so the
      // capitals are the style rather than the string (K-444).
      expect(find.text('RECOVER WORK'), findsOneWidget);
      expect(find.byKey(const ValueKey('recover-journal')), findsOneWidget);
      expect(find.byKey(const ValueKey('recover-autosave')), findsOneWidget);
      expect(find.byKey(const ValueKey('recover-discard')), findsOneWidget);

      // Not restoring leaves everything where it is — the copies are not
      // deleted.
      await tester.tap(find.byKey(const ValueKey('recover-discard')));
      await tester.pumpAndSettle();
      expect(listAutosaves(project: path), hasLength(1));
    });

    /// Each button is its own answer (K-488), and the close mark is none of
    /// them — the shape changed, what the dialogue can answer did not.
    ///
    /// *Restore all changes* is the case driven here because replaying the
    /// journal is synchronous engine work. Opening an autosave is not: it goes
    /// through `state.openProject`, whose future never completes in a widget
    /// test's fake-async zone. That the autosave button is present and on the
    /// same row is pinned in `recovery_metrics_test` instead.
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
      void Function(CompositionReference comp)? before,
    }) async {
      tester.view.physicalSize = const Size(1200, 1000);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addAdjustmentLayer();
      before?.call(comp);

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

    /// Both footer actions are inert until somewhere to write is chosen, and
    /// choosing shows the file by its own name.
    testWidgets('Export is inert until somewhere to write is chosen',
        (tester) async {
      await open(tester,
          picker: () async => '${Directory.systemTemp.path}/out.mp4');

      expect(find.text('Not chosen'), findsOneWidget);
      expect(
          tester
              .widget<HouseButton>(find.byKey(const ValueKey('export-start')))
              .onPressed,
          isNull,
          reason: 'nowhere to write is nothing to export');

      await tester.tap(find.byKey(const ValueKey('export-choose')));
      await tester.pumpAndSettle();
      expect(find.text('out.mp4'), findsOneWidget,
          reason: 'the chosen path is shown by its file name');
      expect(
          tester
              .widget<HouseButton>(find.byKey(const ValueKey('export-start')))
              .onPressed,
          isNotNull);

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// Queueing closes the dialog and shows the queue — and the queue is now
    /// raised while the dialog is still standing, so no window is ever asked
    /// to find the Overlay from a context on its way out. The error that
    /// wording guards against is the red "looking up a deactivated widget's
    /// ancestor is unsafe" screen the owner saw after an export; the scrim's
    /// own half of it is covered in `modal_window_test`.
    testWidgets('queueing an export leaves no deactivated context behind',
        (tester) async {
      await open(tester,
          picker: () async => '${Directory.systemTemp.path}/queued.mp4');
      await tester.tap(find.byKey(const ValueKey('export-choose')));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('export-add-to-queue')));
      await tester.pumpAndSettle();

      expect(tester.takeException(), isNull,
          reason: 'the queue went up before the dialog came down');
      expect(find.byKey(const ValueKey('export-queue-title-strip')),
          findsOneWidget,
          reason: 'and the queue is what is on screen now');

      await tester.tap(find.byKey(const ValueKey('export-queue-dismiss')));
      await tester.pumpAndSettle();
    });

    /// The dialog's fields default to the composition's own facts (K-201): the
    /// frame rate is the comp's, and the span is the work area exactly as the
    /// Timeline set it — already typed, not re-derived by the user.
    testWidgets('the rate and span default to the comp and its work area',
        (tester) async {
      await open(tester, before: (comp) {
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

    /// What a format can carry decides what is live (K-479, K-485): an mp4 has
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
      // for (K-479).
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

    /// *Still* is gone (K-485): an image sequence of one frame is a still, and
    /// the span already says how many frames there are.
    testWidgets('there is no Still output type', (tester) async {
      await open(tester);
      expect(find.byKey(const ValueKey('export-type-still')), findsNothing);
      expect(find.text('STILL'), findsNothing);
      expect(find.text('AUDIO ONLY'), findsOneWidget,
          reason: 'the drawing\'s fourth type is the one that stayed');
      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// The bitrate is three answers, not two (K-479). *Auto* works one out from
    /// the frame and the rate; unticking it hands over a field, and a **blank**
    /// field means the encoder chooses its own quality — which is why the
    /// footer stops estimating a size the moment it is blank.
    testWidgets('Auto, a typed rate and a blank field are three answers',
        (tester) async {
      await open(tester);

      expect(find.textContaining('≈'), findsOneWidget,
          reason: 'Auto knows the rate, so the footer can estimate a size');

      await tester.tap(find.byKey(const ValueKey('export-bitrate-auto')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('export-bitrate')), findsOneWidget,
          reason: 'unticking Auto hands over the field');
      expect(find.textContaining('≈'), findsNothing,
          reason: 'a blank field is a quality nobody chose and a size nobody '
              'can estimate (K-119)');

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// The crop is four numbers in pixels at composition size (K-419), and the
    /// engine resolves them: the Picture group's reading and the footer's size
    /// are one answer, from `crop_for`.
    testWidgets('a crop takes pixels off the delivered frame', (tester) async {
      await open(tester);

      expect(find.text('Final 1920 × 1080'), findsOneWidget);
      await tester.drag(
          find.byKey(const ValueKey('export-crop-left')), const Offset(120, 0));
      await tester.pumpAndSettle();

      expect(find.text('Final 1920 × 1080'), findsNothing,
          reason: 'the crop changed the frame that will be written');
      expect(find.byKey(const ValueKey('export-crop-reading')), findsOneWidget,
          reason: 'and the group says what it left');

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

    /// A preset is the whole settings payload under a name (K-479's store):
    /// saving one lists it, applying it fills the fields back in, and deleting
    /// it takes it off the list. The built-ins are read-only and say so.
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

    /// Metadata is an ordered key/value set, because the order lands in the
    /// file (docs/06 §7.4). The five classic fields lead; an empty one writes
    /// nothing at all, which is why they can sit there unfilled.
    testWidgets('metadata keeps the order it was typed in', (tester) async {
      await open(tester);

      await tester.enterText(
          find.byKey(const ValueKey('export-metadata-0')), 'Opening titles');
      await tester.enterText(
          find.byKey(const ValueKey('export-metadata-1')), 'Nobody');
      await tester.pumpAndSettle();

      expect(find.text('Opening titles'), findsWidgets);
      expect(find.text('Nobody'), findsWidgets);
      expect(find.byKey(const ValueKey('export-metadata-remove-0')),
          findsOneWidget,
          reason: 'every field can be taken off the list');

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// The size the file will be is one answer built from two rows: the
    /// Composition group's resolution, and Picture's own Resize when it is
    /// ticked. Both read back in the footer, which is what the user checks.
    testWidgets('the resolution and the resize agree on one size',
        (tester) async {
      await open(tester);

      expect(find.text('Final 1920 × 1080'), findsOneWidget);
      expect(find.textContaining('1920×1080'), findsOneWidget,
          reason: 'the footer states the size the file will be');

      await tester.tap(find.byKey(const ValueKey('export-resolution')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('One 2 · 960 × 540').last);
      await tester.pumpAndSettle();
      expect(find.text('Final 960 × 540'), findsOneWidget,
          reason: 'half resolution halves what is written');

      // Resize wins over it: an explicit size is an explicit size.
      await tester.tap(find.byKey(const ValueKey('export-resize')));
      await tester.pumpAndSettle();
      expect(find.text('Final 1920 × 1080'), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// Every row K-485 drew dead is a setting now (K-493, K-497, K-498, K-501,
    /// K-502, carried over the seam by K-503). Changing each one and saving the
    /// result as a preset is the assertion, because a preset is the whole
    /// settings payload: what comes back out of the store is what the dialog
    /// put into the spec.
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

    /// The new rows obey the capability row exactly as the old ones do
    /// (K-479): AAC stores coefficients rather than samples, so an mp4 or an
    /// m4a offers one sample width and a `.wav` offers both; a sound file has
    /// no colour to state and no picture to resize.
    testWidgets('a format that cannot carry a setting says so', (tester) async {
      await open(tester);

      bool live(String key) =>
          tester
              .widget<HouseButton>(find.descendant(
                of: find.byKey(ValueKey<String>(key)),
                matching: find.byType(HouseButton),
              ))
              .onPressed !=
          null;

      // An mp4: the rate and the layout are free, the width is not.
      expect(live('export-audio-sample-rate'), isTrue);
      expect(live('export-audio-layout'), isTrue);
      expect(live('export-audio-depth'), isFalse,
          reason: 'AAC has no sample width to set (K-493)');
      expect(live('export-colour-space'), isTrue,
          reason: 'an mp4 states its colour in its own box');
      expect(live('export-resample'), isTrue);

      await tester.tap(find.byKey(const ValueKey('export-type-audioOnly')));
      await tester.pumpAndSettle();
      expect(live('export-colour-space'), isFalse,
          reason: 'a sound file has no colour to state');
      expect(live('export-resample'), isFalse,
          reason: 'and no picture to resize');
      expect(live('export-audio-depth'), isFalse,
          reason: 'the m4a leading the sound formats is still AAC');

      await tester.tap(find.byKey(const ValueKey('export-format')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('WAV (uncompressed)').last);
      await tester.pumpAndSettle();
      expect(live('export-audio-depth'), isTrue,
          reason: 'a .wav takes either width, through pcm_s16le and pcm_s24le');

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// The Colour section lists what the *format* can state, and says which of
    /// the two it is doing (K-498): a still sequence carries no tag, so it is
    /// offered only the space an untagged file is universally taken to be.
    testWidgets('the colour list and its reading follow the container',
        (tester) async {
      await open(tester);

      await tester.tap(find.byKey(const ValueKey('export-colour-space')));
      await tester.pumpAndSettle();
      for (final space in const [
        'sRGB / Rec.709',
        'Linear',
        'Rec. 709',
        'Rec. 2020',
        'Display P3',
      ]) {
        expect(find.text(space), findsWidgets,
            reason: 'an mp4 carries the whole built-in family');
      }
      await tester.tap(find.text('Display P3').last);
      await tester.pumpAndSettle();
      expect(find.text('The file states this space in its own header.'),
          findsOneWidget,
          reason: 'and the section says the container will state it');

      await tester.tap(find.byKey(const ValueKey('export-type-imageSequence')));
      await tester.pumpAndSettle();
      expect(find.textContaining('states no space'), findsOneWidget,
          reason: 'a still sequence tags nothing, and the reading says so');
      expect(
          tester
              .widget<HouseButton>(find.descendant(
                of: find.byKey(const ValueKey('export-colour-space')),
                matching: find.byType(HouseButton),
              ))
              .onPressed,
          isNull,
          reason: 'one space is not a choice');

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// A setting the format cannot honour is still refused in the footer
    /// before anything is queued — the new rows reach `ExportSpec::check` the
    /// same way the depth always has (K-479, K-493).
    testWidgets('a 24-bit setting an AAC file cannot carry is refused here',
        (tester) async {
      final target = '${Directory.systemTemp.path}/refused-24.m4a';
      await open(tester, picker: () async => target);
      await tester.tap(find.byKey(const ValueKey('export-type-audioOnly')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('export-format')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('WAV (uncompressed)').last);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('export-choose')));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('export-audio-depth')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('24 bit').last);
      await tester.pumpAndSettle();
      expect(
          tester
              .widget<HouseButton>(find.byKey(const ValueKey('export-start')))
              .onPressed,
          isNotNull,
          reason: 'a .wav carries twenty-four bits happily');

      await tester.tap(find.byKey(const ValueKey('export-format')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('M4A (AAC)').last);
      await tester.pumpAndSettle();

      expect(find.textContaining('24'), findsWidgets,
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

      expect(find.text('EXPORT QUEUE'), findsOneWidget,
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
  /// waiting (K-503: the engine refuses a row that is running, has run, or has
  /// gone). The list and the move are both injected, because what is asserted
  /// here is the window's gesture rather than the engine's slot.
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

    testWidgets('a waiting row is dragged to another place', (tester) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final items = [item(1, 'One'), item(2, 'Two'), item(3, 'Three')];
      final moves = <(int, int)>[];
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
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

    testWidgets('a row that has already run does not move', (tester) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final items = [
        item(1, 'One'),
        item(2, 'Two', state: const BridgeExportQueueState.done()),
      ];
      final moves = <(int, int)>[];
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-queue'),
            onPressed: () => showExportQueueFrb(
              context: context,
              list: () => List.of(items),
              move: ({required int id, required int index}) =>
                  moves.add((id, index)),
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

      await dragRow(tester, 'export-queue-item-2', -exportQueueRow);
      expect(moves, isEmpty,
          reason: 'what has already run has no place left to take');

      await tester.tap(find.byKey(const ValueKey('export-queue-dismiss')));
      await tester.pumpAndSettle();
    });
  }, skip: !engineAvailable);

  /// The boot splash is the window until boot ends (K-008), and the welcome
  /// screen is the window after it (K-464): the shell must not be in the tree
  /// behind either, or the first-run question would open underneath a screen
  /// nothing can be clicked through.
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

      // Blank project is the way straight through.
      await tester.tap(find.byKey(const ValueKey('welcome-card-blank')));
      await tester.pumpAndSettle();
      expect(find.byType(LumitAppView), findsOneWidget);
    });

    /// Settings ▸ General can stand the welcome screen down for every launch
    /// (K-481). Lumit then opens straight into the shell, whose Viewer offers
    /// the same three ways to start until something is displayed — so the
    /// setting hides no choice.
    testWidgets('the launch setting is honoured', (tester) async {
      tester.view.physicalSize = const Size(1800, 1100);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      p.uiState.workspace.showWelcomeOnLaunch = false;
      await tester.pumpWidget(hostPanel(
        child: const BootGate(splash: false),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pumpAndSettle();

      expect(find.byType(WelcomeScreenFrb), findsNothing);
      expect(find.byType(LumitAppView), findsOneWidget);
      expect(find.byKey(const ValueKey('welcome-card-new')), findsOneWidget,
          reason: 'the shell\'s own empty stage offers the same three ways in');
    });

    testWidgets('can be stood down, for the tests that drive the shell',
        (tester) async {
      tester.view.physicalSize = const Size(1800, 1100);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: const BootGate(splash: false, welcome: false),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      expect(find.byType(LumitAppView), findsOneWidget);
    });
  }, skip: !engineAvailable);
}
