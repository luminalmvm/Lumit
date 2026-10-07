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
  });
}
