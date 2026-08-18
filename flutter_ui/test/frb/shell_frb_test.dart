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
import 'package:lumit_flutter/shell/recovery_dialog_frb.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/shell/status_line_frb.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';
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
      await tester.tap(find.byKey(const ValueKey('settings-page-performance')));
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
      await tester.scrollUntilVisible(unaccounted, 200,
          scrollable: find.byType(Scrollable).first);
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
      await tester.tap(find.byKey(const ValueKey('settings-page-performance')));
      await tester.pumpAndSettle();
      // The page is a lazy list and the disk tier is the last group on it, so
      // it has to be scrolled to before it exists at all.
      await tester.drag(find.byKey(const ValueKey('settings-body-performance')),
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
      await tester.tap(find.byKey(const ValueKey('settings-page-performance')));
      await tester.pumpAndSettle();
      await tester.drag(find.byKey(const ValueKey('settings-body-performance')),
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

      await tester.tap(find.byKey(const ValueKey('settings-page-interface')));
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
      expect(find.textContaining('Recover unsaved work'), findsNothing);
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

      expect(find.text('Recover unsaved work'), findsOneWidget);
      expect(find.byKey(const ValueKey('recover-journal')), findsOneWidget);
      expect(find.byKey(const ValueKey('recover-autosave')), findsOneWidget);
      expect(find.byKey(const ValueKey('recover-discard')), findsOneWidget);

      // Discard leaves everything where it is — the copies are not deleted.
      await tester.tap(find.byKey(const ValueKey('recover-discard')));
      await tester.pumpAndSettle();
      expect(listAutosaves(project: path), hasLength(1));
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

  group('Export dialogue (frb)', () {
    testWidgets('Export is inert until somewhere to write is chosen',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addAdjustmentLayer();

      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-export'),
            onPressed: () => showExportDialogFrb(
              context: context,
              comp: comp,
              picker: () async => '${Directory.systemTemp.path}/out.mp4',
            ),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-export')));
      await tester.pumpAndSettle();

      expect(find.text('Export composition'), findsOneWidget);
      expect(find.text('Not chosen'), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('export-choose')));
      await tester.pumpAndSettle();
      expect(find.text('out.mp4'), findsOneWidget,
          reason: 'the chosen path is shown by its file name');

      // Starting either runs or explains itself — a machine with no GPU says
      // so where the progress would be, rather than the dialogue looking dead.
      await tester.tap(find.byKey(const ValueKey('export-start')));
      await tester.pumpAndSettle(const Duration(milliseconds: 400));
      expect(find.byKey(const ValueKey('export-close')), findsOneWidget,
          reason: 'the dialogue survives whatever the exporter said');

      exportCancel();
      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// The dialogue's fields default to the composition's own facts (K-201):
    /// the frame rate is the comp's, and the range is the work area exactly as
    /// the Timeline set it — already typed, not re-derived by the user.
    testWidgets('the rate and range default to the comp and its work area',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addAdjustmentLayer();
      // A 60 fps comp with a work area over frames 60..180 (1 s .. 3 s).
      comp.setWorkArea(
        span: const BridgeSpan(
          inPoint: BridgeRational(num: 1, den: 1),
          outPoint: BridgeRational(num: 3, den: 1),
          startOffset: BridgeRational(num: 0, den: 1),
        ),
      );

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
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-export')));
      await tester.pumpAndSettle();

      expect(find.text('Frame rate'), findsOneWidget);
      expect(find.text('60.00'), findsOneWidget,
          reason: 'the rate starts as the comp order — its own 60');
      expect(find.text('60'), findsOneWidget,
          reason: 'the range starts at the work area start');
      expect(find.text('180'), findsOneWidget,
          reason: 'and ends at the work area end');

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });

    /// An image sequence is stills: the video-only rows leave rather than
    /// sitting greyed, and the picker's suggestion follows the extension.
    testWidgets('choosing a sequence format sheds the video-only rows',
        (tester) async {
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
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-export')));
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('export-audio')), findsOneWidget);
      expect(find.byKey(const ValueKey('export-bitrate')), findsOneWidget);
      // The dialogue opens on the delivery preset, not a blank Custom
      // (docs/06 §7.5): a fresh export showing a bit rate of 0 read as
      // broken, and the preset's 16 Mb/s stamp is the proof it applied.
      expect(find.textContaining('16'), findsOneWidget,
          reason: "the YouTube 1080p60 preset's bit rate is stamped on open");
      expect(find.byKey(const ValueKey('export-audio-rate')), findsOneWidget,
          reason: 'audio has its own rate once audio is on');

      await tester.tap(find.byKey(const ValueKey('export-format')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('PNG image sequence').last);
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('export-audio')), findsNothing,
          reason: 'stills carry no sound');
      expect(find.byKey(const ValueKey('export-bitrate')), findsNothing,
          reason: 'stills are lossless');
      expect(find.byKey(const ValueKey('export-preset')), findsNothing,
          reason: 'the delivery presets are mp4 by nature');
      expect(find.textContaining('One numbered PNG per frame'), findsOneWidget,
          reason: 'the dialogue says what a sequence writes');
      // The rate and range stay: stills have both.
      expect(find.byKey(const ValueKey('export-fps')), findsOneWidget);
      expect(find.byKey(const ValueKey('export-range-start')), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('export-close')));
      await tester.pumpAndSettle();
    });
  }, skip: !engineAvailable);

  /// The boot splash is the window until boot ends (K-008): the shell must not
  /// be in the tree behind it, or the first-run question would open underneath
  /// a screen nothing can be clicked through.
  group('The boot splash', () {
    testWidgets('is the whole window, and hands over to the shell',
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
      expect(find.byType(LumitAppView), findsNothing,
          reason: 'and nothing of the application is behind it');
      // The engine's own first line, not the canned fallback: with a bridge
      // loaded the log is what the splash streams.
      expect(find.text(bootLines.first), findsNothing);
      expect(find.text(bootLog().first), findsOneWidget);

      await tester.pumpAndSettle();
      expect(find.byType(LumitAppView), findsOneWidget,
          reason: 'boot over, the shell takes the window');
    });

    testWidgets('can be stood down, for the tests that drive the shell',
        (tester) async {
      tester.view.physicalSize = const Size(1800, 1100);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: const BootGate(splash: false),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      expect(find.byType(LumitAppView), findsOneWidget);
    });
  }, skip: !engineAvailable);
}
