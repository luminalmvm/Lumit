// The keyboard picture on Settings → Shortcuts and the After Effects import,
// against the real engine.
//
// Both read the same keymap the keys do, so each test checks the picture and
// the lookup together.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

/// A few lines in After Effects' own layout, with Split layer moved off its
/// usual chord.
const _afterEffectsFile = '''
# After Effects Shortcut Preferences (modify at your own risk)

["CSwitchboard"]
	"SplitLayer" = "(Ctrl+Shift+X)"
	"DeselectAll" = "(F2)(Ctrl+Shift+A)"

["CEggAppTool"]
	"ToolHand" = "()"
''';

void main() {
  setUpAll(initEngineForTests);

  setUp(() => keymapLoadPreset(preset: BridgeKeymapPreset.lumit));
  tearDownAll(() => keymapLoadPreset(preset: BridgeKeymapPreset.lumit));

  Future<({LumitState state, LumitUiState uiState})> openKeymapPage(
      WidgetTester tester) async {
    tester.view.physicalSize = const Size(1400, 1000);
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
    ));
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('open-settings')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('settings-page-shortcuts')));
    await tester.pumpAndSettle();
    return p;
  }

  Future<TestGesture> mouse(WidgetTester tester) async {
    final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await gesture.addPointer(location: Offset.zero);
    addTearDown(gesture.removePointer);
    return gesture;
  }

  Future<void> pointAt(
      WidgetTester tester, TestGesture gesture, String key) async {
    await gesture
        .moveTo(tester.getCenter(find.byKey(ValueKey('keymap-key-$key'))));
    await tester.pump();
  }

  Finder inReadout(String text) => find.descendant(
        of: find.byKey(const ValueKey('keymap-key-readout')),
        matching: find.textContaining(text, findRichText: true),
      );

  group('the keyboard picture (frb)', () {
    testWidgets('pointing at a key names what it does and where',
        (tester) async {
      await openKeymapPage(tester);
      final gesture = await mouse(tester);

      // L is the one key the shipped map gives two meanings.
      await pointAt(tester, gesture, 'L');
      expect(inReadout('Shuttle forwards'), findsOneWidget);
      expect(inReadout('Anywhere'), findsOneWidget);
      expect(inReadout('Reveal Audio'), findsOneWidget);
      expect(inReadout('Timeline'), findsOneWidget);

      // A key with nothing on it says so.
      await pointAt(tester, gesture, 'F1');
      expect(inReadout('Not set'), findsOneWidget);
    });

    testWidgets('clicking a modifier shows the keys it changes',
        (tester) async {
      await openKeymapPage(tester);
      final gesture = await mouse(tester);

      await pointAt(tester, gesture, 'S');
      expect(inReadout('Save the project'), findsNothing);

      await tester.tap(find.byKey(const ValueKey('keymap-key-Mod')));
      await tester.pump();
      await pointAt(tester, gesture, 'S');
      expect(inReadout('Ctrl+S'), findsOneWidget);
      expect(inReadout('Save the project'), findsOneWidget);

      // And it comes off again.
      await tester.tap(find.byKey(const ValueKey('keymap-key-Mod')));
      await tester.pump();
      await pointAt(tester, gesture, 'S');
      expect(inReadout('Save the project'), findsNothing);
    });

    testWidgets('a second chord lights its key too', (tester) async {
      await openKeymapPage(tester);
      final gesture = await mouse(tester);

      // The table shows Ctrl+Right for Next frame. Page Down steps one as well.
      await pointAt(tester, gesture, 'PageDown');
      expect(inReadout('Next frame'), findsOneWidget);
    });

    testWidgets('it steps aside while a search is typed', (tester) async {
      await openKeymapPage(tester);
      expect(find.byKey(const ValueKey('keymap-key-K')), findsOneWidget);

      await tester.enterText(
          find.byKey(const ValueKey('settings-search')), 'palette');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('keymap-key-K')), findsNothing);
      expect(find.text('Open the command palette'), findsOneWidget);
    });
  });

  group('After Effects shortcuts (frb)', () {
    testWidgets('a shortcut file moves the keys and the picture',
        (tester) async {
      final p = await openKeymapPage(tester);

      final result = await tester
          .runAsync(() => p.uiState.keymap.importAfterEffects(_afterEffectsFile));
      await tester.pumpAndSettle();
      expect(result, 3);

      expect(
        keymapLookup(context: BridgeKeyContext.timeline, chord: 'Mod+Shift+X'),
        'layer.split',
      );
      expect(
        keymapLookup(context: BridgeKeyContext.timeline, chord: 'Mod+Shift+D'),
        isNull,
        reason: 'the old chord went with it',
      );
      expect(
        keymapLookup(context: BridgeKeyContext.global, chord: 'F2'),
        'edit.deselect.all',
      );
      // Left empty in After Effects, so left empty here.
      expect(
        keymapLookup(context: BridgeKeyContext.tools, chord: 'H'),
        isNull,
      );
      // Everything else is the After Effects preset.
      expect(
        keymapLookup(context: BridgeKeyContext.global, chord: 'J'),
        'keyframe.prev',
      );

      final gesture = await mouse(tester);
      await tester.tap(find.byKey(const ValueKey('keymap-key-Mod')));
      await tester.tap(find.byKey(const ValueKey('keymap-key-Shift')));
      await tester.pump();
      await pointAt(tester, gesture, 'X');
      expect(inReadout('Split the layer at the playhead'), findsOneWidget);
    });

    testWidgets('a file that is not one is refused and changes nothing',
        (tester) async {
      final p = await openKeymapPage(tester);

      final result = await tester.runAsync(
          () => p.uiState.keymap.importAfterEffects('{"bindings": []}'));
      expect(result, isNull);
      expect(
        keymapLookup(context: BridgeKeyContext.global, chord: 'J'),
        'playback.shuttle.reverse',
        reason: 'still the Lumit default',
      );
    });
  });
}
