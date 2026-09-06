// Settings → Keymap and the reveal cycle, against the real engine
// (docs/07-UI-SPEC.md §15 and §4.3).
//
// The point of these is that the table and the keyboard are the *same* keymap.
// A settings page that edits a copy would look right in every screenshot and
// change nothing about what the keys do, which is the failure worth a test.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart' show RevealFilter;
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

    /// The table exists, is grouped by where a binding is live, and reads in
    /// words — an action id in the left column would mean the description
    /// never made it across the seam.
    testWidgets('the table is grouped and reads in words', (tester) async {
      await openKeymapPage(tester);

      // The first group, and its first rows, are on screen as the page opens.
      // A group's name is a section kicker now, and a kicker's capitals
      // are the style rather than the string.
      expect(find.text('ANYWHERE'), findsOneWidget);
      expect(find.text('Play or pause'), findsOneWidget);
      // The chord cell shows the chord as this platform reads it.
      expect(find.text('Space'), findsOneWidget);
      // And no row leaks its internal name.
      expect(find.text('playback.toggle'), findsNothing);

      // Further down, the table is still grouped: the Timeline's own heading
      // and one of its rows.
      await reveal(tester, find.text('TIMELINE'));
      expect(find.text('TIMELINE'), findsOneWidget,
          reason: 'the kicker, not the sidebar entry of the same name');
      await reveal(tester, find.text('Duplicate the layer'));
      expect(find.text('Duplicate the layer'), findsOneWidget);
    });

    /// Retime has one chord like everything else: Ctrl+Alt+T, AE's
    /// own, and one Windows cannot steal. The misremembered Alt+Shift+T is
    /// gone rather than kept as a second — anyone who wants it back can bind
    /// it, which is the whole point of the page.
    testWidgets('Retime has the one chord, not the misremembered pair',
        (tester) async {
      await openKeymapPage(tester);
      await reveal(tester, find.text('Give the layer a Retime'));
      expect(find.textContaining('Ctrl+Alt+T'), findsOneWidget);
      expect(find.textContaining('Alt+Shift+T'), findsNothing);
      expect(
        keymapLookup(context: BridgeKeyContext.global, chord: 'Alt+Shift+T'),
        isNull,
        reason: 'the old chord is unbound, not hidden',
      );
    });

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

    /// Reset is per row: the shipped chord comes back and nothing else moves.
    testWidgets('reset puts a row back to the shipped chord', (tester) async {
      final p = await openKeymapPage(tester);
      final cell =
          find.byKey(const ValueKey('keymap-chord-global-layer.retime.enable'));
      await reveal(tester, cell);
      await tester.tap(cell);
      await tester.pumpAndSettle();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.f6);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.f6);
      await settleFrb(
        tester,
        until: () =>
            keymapLookup(context: BridgeKeyContext.global, chord: 'F6') != null,
      );
      expect(
        keymapLookup(context: BridgeKeyContext.global, chord: 'Mod+Alt+T'),
        isNull,
        reason: 'the rebind replaced the shipped chord',
      );

      await reveal(tester, cell);
      await tester.tap(find.descendant(
        of: cell,
        matching: find.text('Reset'),
      ));
      await settleFrb(
        tester,
        until: () => p.uiState.keymap.groups.expand((g) => g.bindings).any(
            (b) => b.action == 'layer.retime.enable' && b.chord == 'Mod+Alt+T'),
      );

      expect(
        keymapLookup(context: BridgeKeyContext.global, chord: 'Mod+Alt+T'),
        'layer.retime.enable',
        reason: 'the shipped chord came back',
      );
      expect(
        keymapLookup(context: BridgeKeyContext.global, chord: 'F6'),
        isNull,
        reason: 'and the stand-in went',
      );
    });

    /// Taking a chord is never refused — refusing would make swapping two
    /// actions' keys impossible. What happens next depends on whether the two
    /// can be told apart, and both halves are pinned here because the page
    /// says something different about each.
    ///
    /// A panel taking an **app-wide** chord is a *shadow*, not a clash:
    /// the focused panel gets first refusal and the app-wide binding
    /// is the fallback, so the chord runs exactly one action and which one is
    /// never in doubt. The page says so quietly rather than asking to have it
    /// fixed — the shipped default carries one on purpose (`L` in the
    /// Timeline).
    testWidgets('a panel taking an app-wide chord is said, not warned about',
        (tester) async {
      // The Timeline's zoom-in takes Undo's app-wide chord.
      //
      // Made before the page opens, because the note is built from the keymap
      // the page finds. It is *not* awaited — a bridge Future only completes
      // on a real event-loop turn, and there is no tester to turn one until a
      // widget is pumped — but nor is it done when it returns: the call lands
      // on the engine's worker thread, and a machine quick enough to make that
      // look instant is not a machine to design a test around (the Linux
      // runner lost this race). So it is waited for at its source, which needs
      // a tree to pump: hence the placeholder.
      unawaited(keymapRebind(
        context: BridgeKeyContext.timeline,
        action: 'timeline.zoom.in',
        chord: 'Mod+Z',
      ));
      await tester.pumpWidget(const SizedBox.shrink());
      await settleFrb(tester,
          until: () =>
              keymapShadows().any((s) => s.action.contains('Zoom time in')));

      // Nothing ambiguous: the Timeline zooms, everywhere else undoes.
      expect(
        keymapLookup(context: BridgeKeyContext.timeline, chord: 'Mod+Z'),
        'timeline.zoom.in',
      );
      expect(
        keymapLookup(context: BridgeKeyContext.viewer, chord: 'Mod+Z'),
        'edit.undo',
      );

      await openKeymapPage(tester);

      expect(find.byKey(const ValueKey('keymap-shadows')), findsOneWidget);
      expect(
          find.textContaining('something else in one panel'), findsOneWidget);
      expect(find.textContaining('Undo'), findsWidgets,
          reason: 'the note names what it took the chord from');
      expect(find.byKey(const ValueKey('keymap-conflicts')), findsNothing,
          reason: 'a shadow is not something to go and fix');
    });

    /// Two bindings in the **same** context are a real clash — nothing can
    /// tell them apart — and that is what the warning banner is for.
    ///
    /// Reached by importing a file rather than by rebinding, because rebinding
    /// cannot make one: within a context the previous owner is evicted, so the
    /// chord always has exactly one holder. A shared keymap file is the way a
    /// duplicate actually arrives, which is the case worth pinning.
    testWidgets('two bindings in one context still warn', (tester) async {
      final map = jsonDecode(keymapToJson()) as Map<String, dynamic>;
      (map['bindings'] as List<dynamic>).addAll([
        {
          'context': 'Timeline',
          'chord': 'Mod+Alt+K',
          'action': 'timeline.zoom.in',
        },
        {
          'context': 'Timeline',
          'chord': 'Mod+Alt+K',
          'action': 'timeline.zoom.out',
        },
      ]);
      unawaited(keymapFromJson(json: jsonEncode(map)));
      await tester.pumpWidget(const SizedBox.shrink());
      await settleFrb(tester, until: () => keymapConflicts().isNotEmpty);

      await openKeymapPage(tester);

      expect(find.byKey(const ValueKey('keymap-conflicts')), findsOneWidget);
      expect(find.textContaining('runs two things'), findsOneWidget);
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

    /// The search box filters on the words the table shows, not only the ids
    /// underneath — searching for what you can see must find it.
    testWidgets('search filters the table by what it shows', (tester) async {
      await openKeymapPage(tester);
      await tester.enterText(
          find.byKey(const ValueKey('settings-search')), 'command palette');
      await tester.pumpAndSettle();

      expect(find.text('Open the command palette'), findsOneWidget);
      expect(find.text('Play or pause'), findsNothing);
    });
  });

  group('The reveal cycle (frb)', () {
    /// `U` opens the animated groups, `UU` everything modified, `UUU` shuts the
    /// layer — the After Effects cycle (docs/07 §4.3).
    testWidgets('U, UU and UUU are three different commands', (tester) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;

      // Opacity is changed but not keyframed, so it is modified and not
      // animated — the state that tells the two reveals apart.
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: const BridgeScalar.static_(50),
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

      // U: nothing is animated, so nothing opens.
      await pressU();
      expect(find.text('Transform'), findsNothing,
          reason: 'U reveals animated properties, and none are');

      // UU, inside the multi-tap window: the modified group opens.
      await pressU();
      expect(find.text('Transform'), findsOneWidget,
          reason: 'UU reveals what has been modified');

      // UUU: shut again.
      await pressU();
      expect(find.text('Transform'), findsNothing,
          reason: 'the third tap collapses the layer');
    });

    /// The taps only belong together if they are close in time. A `U` a second
    /// later is a fresh first tap, not a second one — otherwise a shortcut
    /// pressed twice a minute apart would collapse what it had just opened.
    testWidgets('a slow second press starts the cycle again', (tester) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: const BridgeScalar.static_(50),
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

      await pressU();
      await pressU();
      expect(find.text('Transform'), findsOneWidget, reason: 'UU opened it');

      // Past the window: this is a first tap again, and a first tap on a layer
      // with nothing animated shuts what the previous UU opened.
      await tester.pump(const Duration(milliseconds: 600));
      await pressU();
      expect(find.text('Transform'), findsNothing,
          reason: 'the cycle restarted rather than continuing to UUU');
    });

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

    /// **The reveal is the whole selection's**. `Ctrl+A` then `U` read
    /// the selection's *primary* and revealed that layer alone — and where the
    /// top layer carried no keys, which is the ordinary case, the key looked
    /// dead.
    testWidgets('U reveals every selected layer, not the primary alone',
        (tester) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      BridgeScalar keyed(double to) => BridgeScalar.keyframed([
            BridgeKeyframe(
                time: comp.timeOfFrame(frame: 0),
                value: 0,
                interpIn: const BridgeSideInterp.linear(),
                interpOut: const BridgeSideInterp.linear()),
            BridgeKeyframe(
                time: comp.timeOfFrame(frame: 12),
                value: to,
                interpIn: const BridgeSideInterp.linear(),
                interpOut: const BridgeSideInterp.linear()),
          ]);

      // Four layers, and only the last two carry keys — so a reveal that stops
      // at the primary shows nothing at all, which is the reported symptom.
      final layers = [for (var i = 0; i < 4; i++) comp.addSolidLayer()];
      layers[2]
          .setTransform(prop: BridgeTransformProp.opacity, value: keyed(100));
      layers[3]
          .setTransform(prop: BridgeTransformProp.rotation, value: keyed(45));
      p.state.notifyDocumentChanged();
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1400, 700),
      ));
      await tester.pump();

      // What the shell's `Ctrl+A` does in the Timeline: the whole
      // stack, with the top layer as primary.
      p.uiState.setSelection(comp.getLayers());
      await tester.pump();

      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyU);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyU);
      await tester.pump();

      expect(find.text('Opacity'), findsOneWidget,
          reason: 'the third layer is selected too, and its key qualifies');
      expect(find.text('Rotation'), findsOneWidget,
          reason: 'so is the fourth: U runs on every selected layer');
      expect(find.text('Position'), findsNothing,
          reason: 'and still stops at the keys');
    });

    /// A `U` filters the layers it revealed and **no others**: the reveal is a
    /// question about the selection, and answering it must not quietly hide
    /// rows on a layer nobody asked about.
    testWidgets('a keyed reveal lets go of a layer twirled open by hand',
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

      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyU);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyU);
      await tester.pump();
      expect(find.text('Rotation'), findsNothing);

      // The Transform heading, twirled by hand: the layer stops answering the
      // reveal and behaves like any other open layer again.
      await tester.tap(find.text('Transform'));
      await tester.pumpAndSettle();
      expect(find.text('Rotation'), findsOneWidget,
          reason:
              'a hand on the caret ends the filter rather than fighting it');
    });
  });

  /// **Animation ▸ Reveal properties with keyframes / with animation / all
  /// modified properties** — the `U` machinery under the menu's own
  /// words, each row a wider rule than the one above it.
  ///
  /// The menu is not the cycle: a row chosen deliberately says what it does
  /// once, so there is no tap counting and no third press that shuts what the
  /// first two opened. What separates the widest row from `UU` is here too — a
  /// heading opens *whole* for `UU`, and this opens down to the rows that were
  /// actually changed.
  group('The Reveal rows (frb)', () {
    testWidgets('each row is a wider rule than the one above it',
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

      // Opacity is keyed, Scale is driven by an expression, Rotation is merely
      // changed, and Position is where it was put — one property for each of
      // the three rules, and one for none of them.
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
        prop: BridgeTransformProp.scaleX,
        value: const BridgeScalar.expression('time * 10'),
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

      Future<void> reveal(RevealFilter filter) async {
        p.uiState.requestRevealFilter(filter);
        await tester.pump();
      }

      await reveal(RevealFilter.keyframed);
      expect(find.text('Opacity'), findsOneWidget,
          reason: 'the keyed row, which is what U shows');
      expect(find.text('Scale'), findsNothing,
          reason: 'an expression has no keyframes');
      expect(find.text('Rotation'), findsNothing);

      await reveal(RevealFilter.animated);
      expect(find.text('Scale'), findsOneWidget,
          reason: 'an expression writes the value at every frame');
      expect(find.text('Opacity'), findsOneWidget,
          reason: 'each rule is a superset, not a swap');
      expect(find.text('Rotation'), findsNothing,
          reason: 'a number somebody typed does not move');

      await reveal(RevealFilter.modified);
      expect(find.text('Rotation'), findsOneWidget,
          reason: 'the widest rule takes what was changed and left');
      expect(find.text('Opacity'), findsOneWidget);
      expect(find.text('Scale'), findsOneWidget);
      expect(find.text('Position'), findsNothing,
          reason: 'a layer sitting where it was put is not modified — which is '
              'what separates this row from UU, where the heading opens whole');
    });

    /// The rows read the **whole selection**, and the whole comp when
    /// nothing is selected — the same two rules `U` follows, because
    /// they are the same command.
    testWidgets('the reveal covers every selected layer', (tester) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      // Three layers, and only the last is changed — so a reveal that stopped
      // at the selection's primary would find nothing at all.
      final layers = [for (var i = 0; i < 3; i++) comp.addSolidLayer()];
      layers.last.setTransform(
        prop: BridgeTransformProp.rotation,
        value: const BridgeScalar.static_(45),
      );
      p.state.notifyDocumentChanged();
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1400, 700),
      ));
      await tester.pump();

      // What the shell's `Ctrl+A` does in the Timeline.
      p.uiState.setSelection(comp.getLayers());
      await tester.pump();

      p.uiState.requestRevealFilter(RevealFilter.modified);
      await tester.pump();
      expect(find.text('Rotation'), findsOneWidget,
          reason: 'the layer behind the primary was read too');
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
