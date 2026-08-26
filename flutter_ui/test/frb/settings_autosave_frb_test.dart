// Settings → Autosave (K-587): the page K-465 named and left unbuilt until the
// engine had a timer to set.
//
// The page has only two numbers on it, and the interesting one is zero: turning
// autosave off is a setting a user is entitled to hold, so the row reports it
// rather than refusing it. What the engine then *does* with the cadence is the
// bridge crate's own test — nothing here waits for a timer.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart' show LumitUiState;
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Settings → Autosave (frb)', () {
    /// Open Settings on the Autosave page, with [minutes] and [keep] already in
    /// the settings file as a launch would leave them.
    Future<LumitUiState> open(WidgetTester tester,
        {int minutes = 5, int keep = 5}) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      p.uiState.workspace.autosaveMinutes = minutes;
      p.uiState.workspace.autosaveKeep = keep;
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
      await tester.tap(find.byKey(const ValueKey('settings-page-autosave')));
      await tester.pumpAndSettle();
      return p.uiState;
    }

    /// Type [value] into one of the page's two number wells. The editor has to
    /// be found *inside* the well: the title strip's search box is an editable
    /// too, and the whole window is on screen.
    Future<void> type(WidgetTester tester, String key, String value) async {
      await tester.tap(find.byKey(ValueKey<String>(key)));
      await tester.pump();
      await tester.enterText(
        find.descendant(
          of: find.byKey(ValueKey<String>(key)),
          matching: find.byType(EditableText),
        ),
        value,
      );
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
    }

    testWidgets('the page is in the sidebar with its two rows', (tester) async {
      await open(tester);

      expect(find.text(l10n.settingsAutosaveEvery), findsOneWidget);
      expect(find.text(l10n.settingsAutosaveKeep), findsOneWidget);
      expect(
          find.byKey(const ValueKey('settings-autosave-minutes')), findsOneWidget);
      expect(find.byKey(const ValueKey('settings-autosave-keep')), findsOneWidget);
      expect(find.text(l10n.settingsAutosaveOff), findsNothing,
          reason: 'five minutes is not off');
    });

    testWidgets('a typed interval is written to the settings file',
        (tester) async {
      final ui = await open(tester);

      await type(tester, 'settings-autosave-minutes', '12');
      expect(ui.workspace.autosaveMinutes, 12);
      expect(ui.workspace.autosaveKeep, 5, reason: 'the other row is untouched');

      await type(tester, 'settings-autosave-keep', '3');
      expect(ui.workspace.autosaveKeep, 3);
      expect(ui.workspace.autosaveMinutes, 12);
    });

    testWidgets('zero minutes is off, and the row says so', (tester) async {
      final ui = await open(tester);

      await type(tester, 'settings-autosave-minutes', '0');
      expect(ui.workspace.autosaveMinutes, 0);
      expect(find.text(l10n.settingsAutosaveOff), findsOneWidget,
          reason: 'off is reported, not refused');
    });

    testWidgets('Reset page puts both numbers back to what Lumit ships',
        (tester) async {
      final ui = await open(tester, minutes: 0, keep: 20);

      await tester.tap(find.byKey(const ValueKey('settings-reset-page')));
      await tester.pumpAndSettle();

      expect(ui.workspace.autosaveMinutes, 5);
      expect(ui.workspace.autosaveKeep, 5);
      expect(find.text(l10n.settingsAutosaveOff), findsNothing);
    });
  });
}
