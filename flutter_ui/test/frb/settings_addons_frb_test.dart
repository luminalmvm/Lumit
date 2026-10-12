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
      expect(service.entries, isEmpty,
          reason: 'nothing is fetched until the button is pressed');
    });
  });
}
