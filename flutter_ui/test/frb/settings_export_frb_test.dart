// Export defaults: the store, the dialog that opens on it, and the
// Settings page that sets it — the last of the drawn-but-unbuilt pages.
//
// The store is a real file in the application's data area, which is what makes
// it a *default* rather than a session's memory. That also means these tests
// write over the machine's own answers, so each one puts back whatever it found
// before it ran.

import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/shell/export_dialog_frb.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

/// Nothing said: what an unwritten store answers, and what each test leaves
/// behind when it found nothing.
const BridgeExportDefaults nothingSaid = BridgeExportDefaults(
  preset: '',
  codec: '',
  filenameTemplate: '',
  destination: exportDestinationAsk,
  folder: '',
);

void main() {
  setUpAll(initEngineForTests);

  // The machine's own defaults are borrowed, not taken.
  late BridgeExportDefaults borrowed;
  setUp(() => borrowed = exportDefaultsGet());
  tearDown(() => exportDefaultsSet(defaults: borrowed));

  /// The word one dropdown's closed face is showing.
  String face(WidgetTester tester, String key) => tester
      .widget<Text>(find
          .descendant(
            of: find.byKey(ValueKey<String>(key)),
            matching: find.byType(Text),
          )
          .first)
      .data!;

  group('the store (frb)', () {
    test('what is set is what comes back, and every field survives', () {
      exportDefaultsSet(
        defaults: const BridgeExportDefaults(
          preset: 'YouTube 1080p60',
          codec: 'hevc',
          filenameTemplate: '{comp}-{date}',
          destination: exportDestinationFolder,
          folder: '/deliveries',
        ),
      );

      final back = exportDefaultsGet();
      expect(back.preset, 'YouTube 1080p60');
      expect(back.codec, 'hevc');
      expect(back.filenameTemplate, '{comp}-{date}');
      expect(back.destination, exportDestinationFolder);
      expect(back.folder, '/deliveries');
    });

    test('a destination nobody here recognises reads as asking', () {
      exportDefaultsSet(
        defaults: const BridgeExportDefaults(
          preset: '',
          codec: '',
          filenameTemplate: '',
          destination: 'sftp',
          folder: '',
        ),
      );

      expect(exportDefaultsGet().destination, exportDestinationAsk,
          reason: 'an answer from a newer Lumit must not send this one '
              'hunting for a folder it cannot name');
    });
  });

  group('Export dialog seeding (frb)', () {
    Future<void> open(WidgetTester tester) async {
      tester.view.physicalSize = const Size(1200, 1000);
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
        size: const Size(1200, 1000),
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-export')));
      await tester.pumpAndSettle();
    }

    testWidgets('the dialog opens on the preset the store names',
        (tester) async {
      exportDefaultsSet(
        defaults: const BridgeExportDefaults(
          preset: 'YouTube 4K60',
          codec: '',
          filenameTemplate: '',
          destination: exportDestinationAsk,
          folder: '',
        ),
      );

      await open(tester);

      expect(face(tester, 'export-preset'), 'YouTube 4K60',
          reason: 'not the first built-in, which is what it opens on when '
              'nothing has been said');
    });

    testWidgets('a default naming a preset that is gone opens on the first',
        (tester) async {
      exportDefaultsSet(
        defaults: const BridgeExportDefaults(
          preset: 'A preset nobody saved',
          codec: '',
          filenameTemplate: '',
          destination: exportDestinationAsk,
          folder: '',
        ),
      );

      await open(tester);

      expect(face(tester, 'export-preset'), 'Master');
    });

    testWidgets('a fixed folder and a template fill the destination in',
        (tester) async {
      final folder = Directory.systemTemp
          .createTempSync('lumit-export-defaults')
        ..createSync(recursive: true);
      addTearDown(() => folder.deleteSync(recursive: true));

      exportDefaultsSet(
        defaults: BridgeExportDefaults(
          preset: 'Master',
          codec: '',
          filenameTemplate: '{comp}-delivery',
          destination: exportDestinationFolder,
          folder: folder.path,
        ),
      );

      await open(tester);

      expect(face(tester, 'export-path'), 'Opening titles-delivery.mp4',
          reason: 'the engine substituted {comp} and the dialog put the file '
              'in the folder that was chosen once');
      expect(find.text(l10n.exportNotChosen), findsNothing);
    });

    testWidgets('Set as default remembers the preset in force', (tester) async {
      exportDefaultsSet(defaults: nothingSaid);

      await open(tester);
      expect(face(tester, 'export-preset'), 'Master');

      await tester.tap(find.byKey(const ValueKey('export-preset-set-default')));
      await tester.pumpAndSettle();

      final stored = exportDefaultsGet();
      expect(stored.preset, 'Master');
      expect(stored.codec, isNotEmpty,
          reason: 'the format is remembered beside the preset');
      expect(stored.destination, exportDestinationAsk,
          reason: 'the rows this button does not ask about are left alone');
    });
  });

  group('Settings → Export (frb)', () {
    Future<void> open(WidgetTester tester) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
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
        size: const Size(1200, 900),
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-settings')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('settings-page-export')));
      await tester.pumpAndSettle();
    }

    testWidgets('the page is in the sidebar showing what the store holds',
        (tester) async {
      exportDefaultsSet(
        defaults: const BridgeExportDefaults(
          preset: 'YouTube 1080p60',
          codec: '',
          filenameTemplate: '{comp}-{date}',
          destination: exportDestinationProject,
          folder: '',
        ),
      );

      await open(tester);

      expect(find.text(l10n.settingsExportPreset), findsOneWidget);
      expect(face(tester, 'settings-export-preset'), 'YouTube 1080p60');
      expect(
        tester
            .widget<HouseTextField>(
                find.byKey(const ValueKey('settings-export-template')))
            .controller
            .text,
        '{comp}-{date}',
      );
      expect(face(tester, 'settings-export-destination'),
          l10n.settingsExportBesideProject);
      expect(find.byKey(const ValueKey('settings-export-folder')), findsNothing,
          reason: 'a folder is only chosen for the policy that needs one');
    });

    testWidgets('a typed template is written to the store', (tester) async {
      exportDefaultsSet(defaults: nothingSaid);

      await open(tester);
      await tester.enterText(
        find.descendant(
          of: find.byKey(const ValueKey('settings-export-template')),
          matching: find.byType(EditableText),
        ),
        '{preset}-{date}',
      );
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(exportDefaultsGet().filenameTemplate, '{preset}-{date}');
    });

    testWidgets('choosing a fixed folder offers the picker beside it',
        (tester) async {
      exportDefaultsSet(defaults: nothingSaid);

      await open(tester);
      await tester.tap(find.byKey(const ValueKey('settings-export-destination')));
      await tester.pumpAndSettle();
      await tester.tap(find.text(l10n.settingsExportChosenFolder).last);
      await tester.pumpAndSettle();

      expect(exportDefaultsGet().destination, exportDestinationFolder);
      expect(
          find.byKey(const ValueKey('settings-export-folder')), findsOneWidget);
      expect(find.text(l10n.settingsExportNoFolder), findsOneWidget,
          reason: 'the row reports that no folder has been chosen yet');
    });

    testWidgets('Reset page puts the store back to nothing said',
        (tester) async {
      exportDefaultsSet(
        defaults: const BridgeExportDefaults(
          preset: 'YouTube 4K60',
          codec: 'hevc',
          filenameTemplate: '{comp}',
          destination: exportDestinationProject,
          folder: '',
        ),
      );

      await open(tester);
      await tester.tap(find.byKey(const ValueKey('settings-reset-page')));
      await tester.pumpAndSettle();

      expect(exportDefaultsGet(), nothingSaid);
    });
  });
}
