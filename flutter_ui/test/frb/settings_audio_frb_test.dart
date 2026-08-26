// Settings → Audio (K-586): the page K-465 named and left unbuilt until the
// engine could name the machine's outputs.
//
// Everything here is deliberately independent of what sound hardware the
// machine running the test happens to have — CI has none, the owner's desktop
// has several. The two facts that hold either way are the ones asserted: the
// list always offers **System default**, which is not a particular box but a
// promise to follow the machine; and a device that is not on this machine is
// reported rather than silently played through.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart' show LumitUiState;
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

/// A name no machine has: the state a user is in after unplugging the headset
/// they pinned, which is the whole point of the fallback.
const _gone = 'A device this machine does not have';

void main() {
  setUpAll(initEngineForTests);

  group('Settings → Audio (frb)', () {
    /// Open Settings on the Audio page with [chosen] already in the settings
    /// file and in the engine, as a launch would leave it. Answers the UI
    /// state, so a test can read what the page wrote back.
    Future<LumitUiState> open(WidgetTester tester, {String? chosen}) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      // Whatever this test asked for, and the system default again afterwards:
      // the engine's audio state is process-wide, like the real application's.
      setAudioDevice(id: chosen ?? '');
      addTearDown(() => setAudioDevice(id: ''));
      final p = freshProject();
      p.uiState.workspace.audioDevice = chosen;
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
      await tester.tap(find.byKey(const ValueKey('settings-page-audio')));
      await tester.pumpAndSettle();
      return p.uiState;
    }

    testWidgets('the page is in the sidebar and offers the system default',
        (tester) async {
      final ui = await open(tester);

      expect(find.byKey(const ValueKey('settings-audio-device')), findsOneWidget,
          reason: 'the Audio page draws the output row');
      expect(find.text(l10n.settingsAudioDevice), findsOneWidget);
      // A machine with no sound card at all still gets an honest list: the
      // closed face names the system default rather than nothing.
      expect(find.text(l10n.settingsAudioSystemDefault), findsOneWidget);
      expect(ui.workspace.audioDevice, isNull);
      expect(listAudioDevices().fellBack, isFalse,
          reason: 'following the machine is never a substitution');
    });

    testWidgets('a chosen device that is not here is reported, not hidden',
        (tester) async {
      await open(tester, chosen: _gone);
      final devices = listAudioDevices();

      // The choice is kept and still named on the face — it is a pinned
      // device that happens to be unplugged, not a setting to throw away.
      expect(find.text(_gone), findsOneWidget);
      if (devices.fellBack) {
        expect(find.text(l10n.settingsAudioDeviceMissing), findsOneWidget,
            reason: 'the row says the pinned device is not here');
      } else {
        // No output on this machine at all: nothing was substituted, and the
        // row says *that* instead.
        expect(devices.active, isEmpty);
        expect(find.text(l10n.settingsAudioNoDevice), findsOneWidget);
      }
    });

    testWidgets('choosing an output writes the setting and tells the engine',
        (tester) async {
      final ui = await open(tester, chosen: _gone);

      // Pick System default off the list — the one entry every machine has.
      await tester.tap(find.byKey(const ValueKey('settings-audio-device')));
      await tester.pumpAndSettle();
      await tester.tap(find.text(l10n.settingsAudioSystemDefault).last);
      await tester.pumpAndSettle();

      expect(ui.workspace.audioDevice, isNull,
          reason: 'the settings file follows the machine again');
      expect(listAudioDevices().fellBack, isFalse,
          reason: 'the engine was told, so nothing is being substituted');
      expect(find.text(l10n.settingsAudioDeviceMissing), findsNothing);
      expect(find.text(l10n.settingsAudioSystemDefault), findsOneWidget);
    });

    testWidgets('Reset page puts the output back to the system default',
        (tester) async {
      final ui = await open(tester, chosen: _gone);

      await tester.tap(find.byKey(const ValueKey('settings-reset-page')));
      await tester.pumpAndSettle();

      expect(ui.workspace.audioDevice, isNull);
      expect(listAudioDevices().fellBack, isFalse);
    });
  });
}
