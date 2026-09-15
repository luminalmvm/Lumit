// Settings → Addons: the page the optional downloads live on.
//
// What the *service* does with a catalogue, a download and a digest is
// `test/addons_test.dart`, which fakes every seam and never touches an engine.
// This file asserts the other half: that the page is in the sidebar, that its
// three sections are drawn from what the engine actually answers on this
// machine, and that every control on it carries the name the shell test table
// looks for.
//
// Nothing here presses Check. That button is the one thing on the page that
// reaches the network, and a test that pressed it would be asking GitHub for a
// file in the middle of a suite. The rows a catalogue would add (an offer, its
// progress bar and its Cancel) are asserted in the service test instead.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart' show LumitUiState;
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/state/addons.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Settings → Addons (frb)', () {
    /// Open Settings and front the Addons page, either by tapping its entry or,
    /// when [straightThere], by asking for it as the window opens.
    Future<LumitUiState> open(WidgetTester tester,
        {bool straightThere = false}) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-settings'),
            onPressed: () => showSettingsWindowFrb(
              context,
              initialPage:
                  straightThere ? SettingsPage.addons : SettingsPage.general,
            ),
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
      if (!straightThere) {
        await tester.tap(find.byKey(const ValueKey('settings-page-addons')));
        await tester.pumpAndSettle();
      }
      return p.uiState;
    }

    testWidgets('the page is in the sidebar with its three sections',
        (tester) async {
      await open(tester);

      expect(find.text(l10n.settingsGroupAddonRuntime.toUpperCase()),
          findsOneWidget);
      expect(find.text(l10n.settingsGroupAddonsInstalled.toUpperCase()),
          findsOneWidget);
      expect(find.text(l10n.settingsGroupAddonsAvailable.toUpperCase()),
          findsOneWidget);
      expect(find.byKey(const ValueKey('settings-addon-check')), findsOneWidget);
      expect(find.byKey(const ValueKey('settings-addon-install-from-file')),
          findsOneWidget);
      expect(find.byKey(const ValueKey('settings-addon-show-folder')),
          findsOneWidget);
    });

    testWidgets('the runtime row offers the one thing worth doing to it',
        (tester) async {
      final ui = await open(tester);

      // Which of the two is drawn depends on the machine the suite runs on, so
      // the assertion is on the pair rather than on either one.
      final installed = ui.addons.runtimeInstalled != null;
      expect(find.byKey(const ValueKey('settings-addon-runtime-install')),
          installed ? findsNothing : findsOneWidget);
      expect(find.byKey(const ValueKey('settings-addon-runtime-remove')),
          installed ? findsOneWidget : findsNothing);
      expect(find.byKey(const ValueKey('settings-addon-runtime-load')),
          installed ? findsOneWidget : findsNothing);
    });

    testWidgets('an addon whose manifest gives a licence address gets a link',
        (tester) async {
      final ui = await open(tester);

      // Which addons are here depends on the machine the suite runs on, so the
      // assertion is on the rule rather than on a row: an address means a
      // link, and no address means nothing at all.
      for (final addon in ui.addons.installed) {
        expect(
          find.byKey(ValueKey<String>('settings-addon-licence-${addon.id}')),
          addon.licenceUrl.isEmpty ? findsNothing : findsOneWidget,
        );
      }
    });

    testWidgets('the engine is read on entry, not on every rebuild',
        (tester) async {
      final ui = await open(tester);
      final service = ui.addons;

      // The page draws from the reading `_showPage` took: the folder is where
      // the engine says addons live, and the installed list is its scan.
      expect(service.folder, isNotNull);
      expect(service.stage, AddonStage.idle);
      expect(service.failure, isNull);
      if (service.packs.isEmpty) {
        expect(find.byKey(const ValueKey('settings-addons-none')),
            findsOneWidget);
      }
      expect(find.text(l10n.settingsAddonsNotChecked), findsOneWidget,
          reason: 'nothing is fetched until the button is pressed');
    });

    testWidgets('another page can open Settings straight at it', (tester) async {
      await open(tester, straightThere: true);

      expect(find.byKey(const ValueKey('settings-body-addons')), findsOneWidget);
      expect(find.byKey(const ValueKey('settings-addon-check')), findsOneWidget);
    });

    testWidgets('Reset page leaves an addon alone', (tester) async {
      final ui = await open(tester);
      final was = [for (final a in ui.addons.installed) a.id];

      await tester.tap(find.byKey(const ValueKey('settings-reset-page')));
      await tester.pumpAndSettle();

      expect([for (final a in ui.addons.installed) a.id], was,
          reason: 'an install is not a setting Reset may undo');
    });
  });
}
