// Settings → Keymap and the reveal cycle, against the real engine
// (docs/07-UI-SPEC.md §15 and §4.3).
//
// The point of these is that the table and the keyboard are the *same* keymap.
// A settings page that edits a copy would look right in every screenshot and
// change nothing about what the keys do, which is the failure worth a test.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/viewer_view.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  // Every test here edits the one session keymap, so each starts from the
  // shipped default rather than from whatever the last one left.
  setUp(() => keymapLoadPreset(preset: BridgeKeymapPreset.lumit));
  tearDownAll(() => keymapLoadPreset(preset: BridgeKeymapPreset.lumit));

  group('Settings → Keymap (frb)', () {
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

    /// Scroll the settings body until [finder] is built and on screen.
    ///
    /// The table is a lazy list — a few hundred rows, of which the window shows
    /// eight — so a row further down does not exist in the tree until something
    /// scrolls to it. Asserting without this tests the viewport height, not the
    /// table.
    Future<void> reveal(WidgetTester tester, Finder finder) async {
      await tester.scrollUntilVisible(
        finder,
        80,
        scrollable: find
            .descendant(
              of: find.byKey(const ValueKey('settings-body-shortcuts')),
              matching: find.byType(Scrollable),
            )
            .first,
      );
      await tester.pumpAndSettle();
    }

    /// The load-bearing one: clicking a chord and pressing keys changes what
    /// the *keyboard* does, not just what the table says.
    testWidgets('rebinding a row rebinds the key itself', (tester) async {
      final p = await openKeymapPage(tester);
      expect(
        keymapLookup(context: BridgeKeyContext.global, chord: 'Mod+S'),
        'file.save',
      );

      final cell = find.byKey(const ValueKey('keymap-chord-global-file.save'));
      await reveal(tester, cell);
      await tester.tap(cell);
      await tester.pumpAndSettle();
      expect(find.text('Press a shortcut…'), findsOneWidget);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.f5);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.f5);
      await tester.pumpAndSettle();

      expect(
        keymapLookup(context: BridgeKeyContext.global, chord: 'F5'),
        'file.save',
        reason: 'the engine took the new chord',
      );
      expect(
        keymapLookup(context: BridgeKeyContext.global, chord: 'Mod+S'),
        isNull,
        reason: 'and the old one stopped meaning it',
      );
      // The redraw waits on a real event-loop turn: the rebind is a bridge
      // call, and its Future completes on a port message that the widget
      // tester's fake clock never delivers on its own.
      await settleFrb(
        tester,
        until: () => p.uiState.keymap.groups
            .expand((g) => g.bindings)
            .any((b) => b.action == 'file.save' && b.chord == 'F5'),
      );
      await reveal(tester, cell);
      expect(
        find.descendant(of: cell, matching: find.text('F5')),
        findsOneWidget,
        reason: 'the table redrew with the chord the engine took',
      );
    });

    /// **A keymap saved by an older build must not take a new key away**.
    /// This is what actually broke `Ctrl+C` in the owner's app while
    /// every test here passed: a stored keymap replaced the whole map on
    /// start-up, so `edit.copy` — added after that file was written — had no
    /// chord at all. Tests start from the shipped defaults; only a real session
    /// has a file.
    testWidgets('a stored keymap without Copy in it still copies',
        (tester) async {
      final map = jsonDecode(keymapToJson()) as Map<String, dynamic>;
      (map['bindings'] as List<dynamic>).removeWhere((b) =>
          ((b as Map)['action'] as String).startsWith('edit.c') ||
          b['action'] == 'edit.paste');
      unawaited(keymapFromJson(json: jsonEncode(map)));
      await tester.pumpWidget(const SizedBox.shrink());
      await settleFrb(
        tester,
        until: () =>
            keymapLookup(context: BridgeKeyContext.global, chord: 'Mod+C') !=
            null,
      );

      expect(keymapLookup(context: BridgeKeyContext.global, chord: 'Mod+C'),
          'edit.copy',
          reason: 'the stored file is laid over the defaults, not swapped for '
              'them, so an action it never heard of keeps its key');
    });
  });

  group('The scroll wheel (frb)', () {
    /// Someone who wants Alt+wheel to zoom the Timeline gets exactly that, and
    /// Ctrl+wheel stops zooming.
    testWidgets('the Timeline zooms on whichever modifier is set',
        (tester) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);
      // A bridge future only lands on a real event-loop turn.
      await tester.runAsync(() => keymapSetWheel(
          action: BridgeWheelAction.zoomTime,
          modifier: BridgeWheelModifier.alt));
      p.uiState.keymap.refresh();

      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1400, 700),
      ));
      await tester.pumpAndSettle();

      final bar = find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}'));
      double width() => tester.getRect(bar).width;
      final mouse = TestPointer(1, PointerDeviceKind.mouse);
      Future<void> wheelWith(LogicalKeyboardKey key) async {
        await tester.sendKeyDownEvent(key);
        await tester.sendEventToBinding(mouse.hover(tester.getCenter(bar)));
        await tester.sendEventToBinding(mouse.scroll(const Offset(0, -100)));
        await tester.sendKeyUpEvent(key);
        await tester.pumpAndSettle();
      }

      final before = width();
      await wheelWith(LogicalKeyboardKey.controlLeft);
      expect(width(), moreOrLessEquals(before, epsilon: 0.5),
          reason: 'Ctrl no longer zooms once Alt has the job');

      await wheelWith(LogicalKeyboardKey.altLeft);
      expect(width(), greaterThan(before), reason: 'Alt+wheel zoomed in');
    });
  });

  group('The reveal cycle (frb)', () {
    /// **`U` stops at the keys; `UU` opens the headings whole**.
    ///
    /// The first tap used to open the *groups* holding animation, and a group
    /// opens whole — so one keyed Opacity unrolled Position, Scale, Rotation and
    /// Anchor beside it, and "reveal animated properties" showed a screenful of
    /// properties with nothing on them. It now keeps the keyed rows and the
    /// headings above them, which is the same arithmetic the Animated strip
    /// does; everything is still one more tap away.
    testWidgets('U reveals only the keyed rows, and UU reveals them all',
        (tester) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;

      // Opacity is keyed; Rotation is merely changed. Both live under the same
      // Transform heading, which is what makes the two taps tell apart.
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          BridgeKeyframe(
              time: comp.timeOfFrame(frame: 0),
              value: 100,
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear()),
          BridgeKeyframe(
              time: comp.timeOfFrame(frame: 12),
              value: 0,
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear()),
        ]),
      );
      layer.setTransform(
        prop: BridgeTransformProp.rotation,
        value: const BridgeScalar.static_(45),
      );
      p.state.notifyDocumentChanged();

      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1400, 700),
      ));
      await tester.pump();

      Future<void> pressU() async {
        await tester.sendKeyDownEvent(LogicalKeyboardKey.keyU);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.keyU);
        await tester.pump();
      }

      // U: the heading, the keyed row under it, and nothing else.
      await pressU();
      expect(find.text('Transform'), findsOneWidget,
          reason: 'the heading over a keyed row is kept, so the row is placed');
      expect(find.text('Opacity'), findsOneWidget,
          reason: 'the keyed property is what U was asked for');
      expect(find.text('Rotation'), findsNothing,
          reason: 'a changed but unkeyed property is not animated, and U opens '
              'down to the keys rather than opening the heading whole');
      expect(find.text('Position'), findsNothing,
          reason: 'nor is an untouched sibling dragged along by the heading');

      // UU, inside the window: the modified reveal, and a heading opens whole.
      await pressU();
      expect(find.text('Rotation'), findsOneWidget,
          reason: 'the second tap opens everything under the heading');
      expect(find.text('Position'), findsOneWidget);
      expect(find.text('Opacity'), findsOneWidget,
          reason:
              'the keyed row is still there — UU is a superset, not a swap');

      // UUU: shut, exactly as before.
      await pressU();
      expect(find.text('Transform'), findsNothing,
          reason: 'the third tap collapses the layer, cycle unchanged');
    });
  });

  /// The Viewer's own commands name keymap actions rather than carrying chords
  /// of their own, which only works if the ids match the engine's. A
  /// typo here would show as a menu row with no shortcut beside it and a chord
  /// that runs nothing — two silent failures rather than one loud one.
  test('the Viewer view commands name actions the keymap has', () {
    final actions = {
      for (final group in keymapGroups())
        for (final binding in group.bindings) binding.action,
    };
    for (final zoom in ViewerZoomCommand.values) {
      expect(actions, contains(zoom.action));
    }
    for (final resolution in PreviewResolution.values) {
      // Auto and Third have no chord of their own (docs/07 §15 names three
      // tiers), so `action` is null for them by design — nothing to look up.
      final action = resolution.action;
      if (action == null) continue;
      expect(actions, contains(action));
    }
  }, skip: !engineAvailable);
}
