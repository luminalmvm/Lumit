// The menu bar on frb, tested against the real engine.
//
// The port landed untested; this is that gap closed. There was almost nothing to
// migrate — v0's menu bar had exactly one test (Composition ▸ Add solid layer, in
// project_placement_test.dart, against a fake bridge) — so these are new
// coverage rather than a translation.
//
// Every document operation here is genuine. See frb_test_support.dart for why
// these are integration tests rather than fake-bridge unit tests, and for the
// fake-async/real-async seam `settleFrb` exists to cross.
//
// **The one ordering constraint.** `openProject` clears the engine's
// process-wide project registry, which invalidates every reference any other
// test is holding. The round-trip test that calls it is therefore last, and
// builds everything it needs within itself.

import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/state/external_links.dart';
import 'package:lumit_flutter/state/viewer_view.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:provider/provider.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Menu bar (frb)', () {
    /// Mount the menu bar over a fresh engine-backed project, arranged the way
    /// the real shell arranges it.
    ///
    /// The `watch` pair is load-bearing and deliberately mirrors
    /// `_LumitAppViewState` in main.dart: `LumitMenuBarFrb` takes its project as
    /// a constructor argument and reads `LumitUiState` with `context.read`, so it
    /// does not subscribe to either notifier itself — an ancestor that watches
    /// both is what makes Undo/Redo and Composition settings grey and ungrey.
    /// Mounting it bare would test an arrangement that does not ship.
    Future<({LumitState state, LumitUiState uiState})> mount(
      WidgetTester tester, {
      Future<String?> Function()? openPicker,
      Future<String?> Function()? savePicker,
      Future<List<String>> Function()? footagePicker,
    }) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        // Along the top, where the shell puts it. Centred — which is what an
        // overlay entry does with a bar that has no height of its own — the
        // File menu had only half the window beneath it to open into, and
        // `showLumitPopup` pulled it back on screen by sliding it *up over the
        // bar*: correct behaviour for a menu with nowhere to go, and an
        // arrangement that does not ship, in which no heading can be hovered
        // while a menu is open.
        child: Align(
          alignment: Alignment.topLeft,
          child: Builder(builder: (context) {
            final state = context.watch<LumitState>();
            context.watch<LumitUiState>();
            return LumitMenuBarFrb(
              app: state,
              openPicker: openPicker,
              savePicker: savePicker,
              footagePicker: footagePicker,
            );
          }),
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      return p;
    }

    /// Open a top-level menu and tap one of its rows.
    ///
    /// Two pumps rather than `pumpAndSettle`: the popup is an overlay entry and
    /// the host disables animation, so one frame each is enough — and
    /// `pumpAndSettle` would spin on anything the engine has left in flight.
    /// Open a menu and pick a row, scrolling to it first.
    ///
    /// The Composition menu is taller than an 800x600 test surface, so it
    /// scrolls — and a row below the fold has to be brought into view before it
    /// can be tapped, which is what a user does with the wheel.
    /// Open [menu] and click [item]. [under] names a submenu to step through
    /// first — Window → Workspaces → Audio (K-194).
    Future<void> choose(WidgetTester tester, String menu, String item,
        {String? under}) async {
      await tester.tap(find.byKey(ValueKey<String>('menu-$menu')));
      await tester.pump();
      if (under != null) {
        await tester.tap(find.text(under));
        await tester.pump();
      }
      await tester.ensureVisible(find.text(item));
      await tester.pump();
      await tester.tap(find.text(item));
      await tester.pump();
    }

    /// New composition asks for its settings first (K-180), so every route to a
    /// comp goes through the dialogue: choose the command, then press Create.
    Future<void> makeComp(WidgetTester tester) async {
      await choose(tester, 'Composition', 'New composition');
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();
    }

    /// Dismiss an open menu through its full-screen barrier, without choosing
    /// anything.
    /// Well below the menus, and inside the 800x600 test surface — a tap outside
    /// it is not delivered at all, so the menu would silently stay open.
    Future<void> dismiss(WidgetTester tester) async {
      await tester.tapAt(const Offset(400, 500));
      await tester.pump();
    }

    /// Every item in the project, folders flattened — a composition is filed
    /// into the Compositions auto-folder, so it is never one of the roots.
    List<ItemReference> allItems(LumitState state) {
      List<ItemReference> walk(List<ItemReference> items) => [
            for (final i in items) ...[
              i,
              if (i is ItemReference_Folder) ...walk(i.field0.getChildren()),
            ]
          ];
      return walk(state.project?.getItems() ?? const []);
    }

    /// The startup race the 2026-08-25 run log caught: openProject clears the
    /// engine's registry before _adopt lands the new reference, so a rebuild
    /// inside that window builds the bar with a project every call refuses.
    /// The bar must build disabled rather than throw (the same answer a null
    /// project gets).
    testWidgets('a dead project reference builds the bar, not an error',
        (tester) async {
      final p = await mount(tester);
      // Close the project in the engine while the state keeps the reference -
      // exactly what the mid-swap window holds.
      p.state.project!.close();
      await tester.pump();
      // A selection notification is what rebuilt the bar in the wild. A new
      // list instance, because an identical value does not notify.
      p.uiState.selectedLayers.value = List.of(p.uiState.selectedLayers.value);
      await tester.pump();
      expect(tester.takeException(), isNull,
          reason: 'a dead reference reads as no project, not as a throw');
      expect(find.byKey(const ValueKey('menu-File')), findsOneWidget);
      p.state.project = null; // the teardown close would throw the same way
    });

    testWidgets('File shows its items', (tester) async {
      await mount(tester);
      await tester.tap(find.byKey(const ValueKey<String>('menu-File')));
      await tester.pump();

      for (final item in [
        'New',
        'Open project…',
        'Open recent',
        'Save',
        'Save as…',
        'Import…',
        'Export…',
        'Project settings…',
        'Close project (Not implemented)',
      ]) {
        expect(find.text(item), findsOneWidget, reason: 'File ▸ $item');
      }
      await dismiss(tester);
      expect(find.text('New'), findsNothing,
          reason: 'the barrier closes the menu without choosing anything');
    });

    /// The project's own settings are not in Settings (K-286): Settings is
    /// this machine's, and a value saved in the `.lum` is not.
    testWidgets('File ▸ Project settings… opens a window of its own',
        (tester) async {
      await mount(tester);
      await choose(tester, 'File', 'Project settings…');
      await tester.pumpAndSettle();

      expect(
          find.byKey(const ValueKey('project-anti-aliasing')), findsOneWidget);
      expect(
          find.byKey(const ValueKey('settings-page-appearance')), findsNothing,
          reason: 'it is its own window, not a page of Settings');
    });

    testWidgets('Edit and Composition show their items', (tester) async {
      await mount(tester);

      await tester.tap(find.byKey(const ValueKey<String>('menu-Edit')));
      await tester.pump();
      expect(find.text('Undo'), findsOneWidget);
      expect(find.text('Redo'), findsOneWidget);
      await dismiss(tester);

      await tester.tap(find.byKey(const ValueKey<String>('menu-Composition')));
      await tester.pump();
      expect(find.text('New composition'), findsOneWidget);
      expect(find.text('Composition settings…'), findsOneWidget);
    });

    testWidgets('Copy and Paste carry a layer, landing it at the playhead',
        (tester) async {
      // K-275: Copy takes the selected layer whole and Paste puts it in the
      // comp on screen, at the playhead. The engine does the carrying; what is
      // tested here is that the menu is wired to it and to the setting.
      final p = await mount(tester);
      await makeComp(tester);
      final comp = p.uiState.selectedComp!;
      final source = comp.addSolidLayer();
      source.rename(name: 'Hero');
      source.addEffect(name: 'blur');
      p.uiState.setSelection([source]);
      await tester.pump();

      await choose(tester, 'Edit', 'Copy');
      p.uiState.playheadFrame.value = 30;
      await choose(tester, 'Edit', 'Paste');
      await tester.pump();

      final layers = comp.getLayers();
      expect(layers.length, 2, reason: 'the paste made a second layer');
      final pasted = p.uiState.selectedLayer.value!;
      expect(pasted.internallayerId, isNot(source.internallayerId),
          reason: 'and selected it, as every editor does');
      expect(pasted.getName(), 'Hero', reason: 'the name travels');
      expect(pasted.getEffects().length, 1, reason: 'and so does the stack');
      // Frame 30 in seconds, on whatever rate the comp actually runs at.
      final settings = comp.getSettings();
      final atFrame30 = 30 * settings.fpsDen / settings.fpsNum;
      final span = pasted.getSpan();
      expect(span.inPoint.num / span.inPoint.den, closeTo(atFrame30, 1e-9),
          reason: 'the in point lands on the playhead');

      // The setting sends it to the time it was copied from instead.
      p.uiState.workspace.interface.pasteLayersAtOriginalTime = true;
      p.uiState.playheadFrame.value = 60;
      await choose(tester, 'Edit', 'Paste');
      await tester.pump();
      final atOriginal = p.uiState.selectedLayer.value!.getSpan();
      expect(atOriginal.inPoint.num, 0,
          reason: 'with the setting on it keeps the time it was copied at');
    });

    testWidgets('Cut copies the layer before removing it', (tester) async {
      final p = await mount(tester);
      await makeComp(tester);
      final comp = p.uiState.selectedComp!;
      final source = comp.addSolidLayer();
      p.uiState.setSelection([source]);
      await tester.pump();

      await choose(tester, 'Edit', 'Cut');
      await tester.pump();
      expect(comp.getLayers(), isEmpty, reason: 'the layer went');

      await choose(tester, 'Edit', 'Paste');
      await tester.pump();
      expect(comp.getLayers().length, 1,
          reason: 'and came back, so Cut did copy before deleting');
    });

    testWidgets('New composition creates one, fronts it, and names it for you',
        (tester) async {
      final p = await mount(tester);
      expect(p.uiState.selectedComp, isNull);

      await makeComp(tester);

      final comps = allItems(p.state).whereType<ItemReference_Composition>();
      expect(comps.length, 1, reason: 'the menu committed one composition');
      expect(
        p.uiState.selectedComp?.internalid,
        comps.single.field0.internalid,
        reason: 'a comp you just made is the one you want to work on',
      );
      // A blank name is passed through so the engine picks the next "Comp N".
      expect(comps.single.name(), 'Comp 1');
    });

    testWidgets('Composition settings… is disabled until a comp is fronted',
        (tester) async {
      final p = await mount(tester);

      await choose(tester, 'Composition', 'Composition settings…');
      // The dialogue heading prints as a capitals kicker (§12A.4).
      expect(find.text('COMPOSITION SETTINGS'), findsNothing,
          reason: 'no comp is fronted, so the row does nothing when pressed');

      // Front one, and the same row now opens the dialogue.
      await makeComp(tester);
      expect(p.uiState.selectedComp, isNotNull);
      await choose(tester, 'Composition', 'Composition settings…');
      await tester.pump();

      expect(find.text('COMPOSITION SETTINGS'), findsOneWidget,
          reason: 'the dialogue heading');
    });

    testWidgets('Import footage imports every picked path', (tester) async {
      final p = await mount(
        tester,
        footagePicker: () async => ['C:/clips/a.mov', 'C:/clips/b.mov'],
      );

      await choose(tester, 'File', 'Import footage…');
      await tester.pump();

      final names = allItems(p.state)
          .whereType<ItemReference_Footage>()
          .map((f) => f.name())
          .toList();
      expect(names, containsAll(<String>['a.mov', 'b.mov']));
    });

    testWidgets('a cancelled picker changes nothing', (tester) async {
      final p = await mount(
        tester,
        footagePicker: () async => <String>[],
        savePicker: () async => null,
      );

      await choose(tester, 'File', 'Import footage…');
      await tester.pump();
      expect(p.state.project!.getItems(), isEmpty);

      await choose(tester, 'File', 'Save');
      await settleFrb(tester);
      expect(p.state.project!.path(), isNull,
          reason: 'cancelling the location dialogue must not write anything');
    });

    testWidgets('Undo and Redo grey out with the document history',
        (tester) async {
      final p = await mount(tester);
      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);

      Color? colourOf(String label) =>
          tester.widget<Text>(find.text(label)).style?.color;

      // A fresh document has nothing either way.
      await tester.tap(find.byKey(const ValueKey<String>('menu-Edit')));
      await tester.pump();
      expect(colourOf('Undo'), t.textDisabled);
      expect(colourOf('Redo'), t.textDisabled);
      await dismiss(tester);

      // One edit, and Undo lights up.
      await makeComp(tester);
      expect(p.state.project!.history().canUndo, isTrue);

      await tester.tap(find.byKey(const ValueKey<String>('menu-Edit')));
      await tester.pump();
      expect(colourOf('Undo'), isNot(t.textDisabled),
          reason:
              'an item you can see is disabled tells you the document state');
      await tester.tap(find.text('Undo'));
      await tester.pump();

      expect(p.state.project!.getItems(), isEmpty,
          reason: 'Undo reached the engine, not just the menu');
      expect(p.state.project!.history().canRedo, isTrue);

      // Undone: the pair swaps over.
      await tester.tap(find.byKey(const ValueKey<String>('menu-Edit')));
      await tester.pump();
      expect(colourOf('Undo'), t.textDisabled);
      expect(colourOf('Redo'), isNot(t.textDisabled));
      await tester.tap(find.text('Redo'));
      await tester.pump();

      expect(allItems(p.state).whereType<ItemReference_Composition>().length, 1,
          reason: 'Redo put it back');
    });

    testWidgets(
        'Save prompts once, then saves in place; Save as always prompts',
        (tester) async {
      final dir = Directory.systemTemp.createTempSync('lumit-menu-save');
      final first = '${dir.path}/first.lum';
      final second = '${dir.path}/second.lum';

      var prompts = 0;
      final picks = <String>[first, second];
      final p = await mount(
        tester,
        savePicker: () async {
          prompts++;
          return picks.removeAt(0);
        },
      );
      await makeComp(tester);

      // Never saved: Save has to ask where.
      await choose(tester, 'File', 'Save');
      await settleFrb(tester, until: () => File(first).existsSync());
      expect(prompts, 1);
      expect(File(first).existsSync(), isTrue);
      expect(p.state.project!.path(), first);

      // Saved once: Save now writes in place without asking again.
      await choose(tester, 'File', 'Save');
      await settleFrb(tester);
      expect(prompts, 1,
          reason: 'a project with a path is saved, not asked about');

      // Save as asks every time, and moves the project to the new location.
      await choose(tester, 'File', 'Save as…');
      await settleFrb(tester, until: () => File(second).existsSync());
      expect(prompts, 2);
      expect(File(second).existsSync(), isTrue);
      expect(p.state.project!.path(), second);
    });

    // LAST: `openProject` clears the engine's project registry, so every
    // reference held by an earlier test dies here. Nothing may run after it.
    testWidgets('a saved project opens again with its contents intact',
        (tester) async {
      final dir = Directory.systemTemp.createTempSync('lumit-menu-roundtrip');
      final path = '${dir.path}/round.lum';

      final p = await mount(
        tester,
        savePicker: () async => path,
        openPicker: () async => path,
        footagePicker: () async => ['C:/clips/hero.mov'],
      );
      await makeComp(tester);
      await choose(tester, 'File', 'Import footage…');
      await tester.pump();
      await choose(tester, 'File', 'Save');
      await settleFrb(tester, until: () => File(path).existsSync());
      expect(File(path).existsSync(), isTrue,
          reason: 'nothing to open otherwise');

      // A new, empty project, then open the saved one over the top of it.
      await choose(tester, 'File', 'New');
      await tester.pump();
      expect(p.state.project!.getItems(), isEmpty);

      // Reading the document is an async frb call now, so the open lands on
      // settleFrb's real event-loop turns rather than on a pump. Adoption is
      // what is being waited for: the held reference is another project's.
      final before = p.state.project;
      await choose(tester, 'File', 'Open project…');
      await settleFrb(tester, until: () => !identical(p.state.project, before));

      final names = allItems(p.state).map((i) => i.name()).toList();
      expect(names, contains('hero.mov'));
      expect(names, contains('Comp 1'),
          reason: 'the composition came back, filed where it was');
    });
    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
    /// The port shipped a menu with three items per menu where the previous
    /// frontend had layer creation, clip and marker commands, beat detection
    /// and a Window menu. Each of these reaches the document.
    testWidgets('Layer ▸ New creates every kind of layer', (tester) async {
      final p = await mount(tester);
      await makeComp(tester);
      final comp = p.uiState.selectedComp!;

      for (final item in [
        'Solid',
        'Text',
        'Camera',
        'Adjustment',
        'Sequence',
      ]) {
        final before = comp.getLayers().length;
        await choose(tester, 'Layer', item, under: 'New');
        await tester.pump();
        expect(comp.getLayers(), hasLength(before + 1),
            reason: '$item added one');
      }
    });

    testWidgets('the layer items are disabled without a composition',
        (tester) async {
      final p = await mount(tester);
      expect(p.uiState.selectedComp, isNull);

      // Pressing it must be a no-op rather than a crash — a disabled row that
      // throws when clicked is worse than one that is simply absent.
      await choose(tester, 'Layer', 'Solid', under: 'New');
      await tester.pump();
      expect(p.uiState.selectedComp, isNull);
    });

    testWidgets('Add marker at playhead marks the fronted comp',
        (tester) async {
      final p = await mount(tester);
      await makeComp(tester);
      final comp = p.uiState.selectedComp!;
      p.uiState.playheadFrame.value = 30;

      await choose(tester, 'Composition', 'Add marker at playhead');
      await tester.pump();

      expect(comp.getMarkers(), hasLength(1));
      expect(comp.frameAtTime(time: comp.getMarkers().single.time), 30,
          reason: 'it landed on the playhead, not at zero');
    });

    testWidgets('Clear beat markers is calm on a comp with none',
        (tester) async {
      final p = await mount(tester);
      await makeComp(tester);
      await choose(tester, 'Composition', 'Clear beat markers');
      await tester.pump();
      expect(p.uiState.selectedComp!.getMarkers(), isEmpty);
    });

    /// The palette's four categories (docs/07 §12): commands, and now every
    /// effect, comp and panel under its own badge; Enter on each does its
    /// kind of thing. The taught shortcut shows only where a real binding
    /// exists.
    testWidgets('the palette carries effects, comps and panels',
        (tester) async {
      final p = await mount(tester);
      final comp = p.state.project!.newComposition(name: 'Scene beta');
      final layer = comp.addSolidLayer();
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      await tester.pump();

      await choose(tester, 'Window', 'Command palette…');
      await tester.pump();

      // Each category surfaces under its badge when searched for (the list
      // is lazy, so the badges are asserted where their rows are on screen).
      final query = find.byKey(const ValueKey('palette-query'));
      await tester.enterText(query, 'timeline');
      await tester.pump();
      expect(find.text('Panel'), findsWidgets);

      await tester.enterText(query, 'undo');
      await tester.pump();
      expect(find.text('Ctrl+Z'), findsOneWidget,
          reason: 'undo teaches its real shortcut, and only real ones taught');

      // An effect entry applies to the selected layer.
      await tester.enterText(query, 'gaussian');
      await tester.pump();
      expect(find.text('Effect'), findsWidgets);
      await tester
          .tap(find.byKey(const ValueKey('palette-item-Gaussian blur')));
      await tester.pumpAndSettle();
      expect(layer.getEffects().single.name(), 'blur');

      // A comp entry fronts its comp; the recent run ranks it first next time.
      await choose(tester, 'Window', 'Command palette…');
      await tester.pump();
      await tester.enterText(
          find.byKey(const ValueKey('palette-query')), 'scene beta');
      await tester.pump();
      expect(find.text('Comp'), findsWidgets);
      await tester.tap(find.byKey(const ValueKey('palette-item-Scene beta')));
      await tester.pumpAndSettle();
      expect(p.uiState.selectedComp?.internalid, comp.internalid);
    });

    /// **`Ctrl+Shift+P` was bound to nothing.** The palette's list of commands
    /// is declared beside the menu items so the two cannot drift apart, so the
    /// shortcut asks *this* bar for the palette rather than assembling a second
    /// list of its own — which is the drift that note exists to prevent.
    testWidgets('the palette shortcut opens the menu bar\'s own palette',
        (tester) async {
      final p = await mount(tester);
      expect(find.byKey(const ValueKey('palette-query')), findsNothing);

      p.uiState.requestPalette();
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('palette-query')), findsOneWidget);
      // The same list the menu route builds, not a shorter copy.
      await tester.enterText(
          find.byKey(const ValueKey('palette-query')), 'composition');
      await tester.pump();
      expect(find.byKey(const ValueKey('palette-item-New composition')),
          findsOneWidget);
    });

    /// The four shipped workspace presets (docs/07 §1.6): each rearranges the
    /// dock to its factory layout; the same panel inventory throughout, and a
    /// distinct arrangement per preset.
    testWidgets('the Window menu applies the four workspace presets',
        (tester) async {
      final p = await mount(tester);

      // The presets live under their own heading now (K-194).
      await choose(tester, 'Window', 'Effects', under: 'Workspace');
      await tester.pump();
      expect(panelsIn(p.uiState.split),
          panelsIn(presetLayout(WorkspacePreset.effects)));
      expect(p.uiState.split.toJson(),
          isNot(presetLayout(WorkspacePreset.colour).toJson()),
          reason: 'the presets are genuinely different arrangements');

      await choose(tester, 'Window', 'Audio', under: 'Workspace');
      await tester.pump();
      expect(p.uiState.split.toJson(),
          presetLayout(WorkspacePreset.audio).toJson());

      // Reset still means the default (Edit) arrangement.
      await choose(tester, 'Window', 'Reset workspace', under: 'Workspace');
      await tester.pump();
      expect(panelsIn(p.uiState.split), panelsIn(defaultLayout()));
    });

    testWidgets('the Window menu offers the palette, reset and settings',
        (tester) async {
      final p = await mount(tester);

      await tester.tap(find.byKey(const ValueKey<String>('menu-Window')));
      await tester.pump();
      expect(find.text('Command palette…'), findsOneWidget);
      // The arrangements sit behind their own heading (K-194), and Settings
      // moved to Edit where every Windows application keeps it (K-244).
      expect(find.text('Workspace'), findsOneWidget);
      expect(find.text('Settings…'), findsNothing);
      expect(find.text('Reset workspace'), findsNothing,
          reason: 'reset lives with the arrangements it undoes');
      await dismiss(tester);

      // Reset puts a rearranged workspace back to the default.
      p.uiState.workspace.dock = DockSplit(
        DockAxis.vertical,
        [DockPane(Panel.viewer), DockPane(Panel.timeline)],
        [0.5, 0.5],
      );
      await choose(tester, 'Window', 'Reset workspace', under: 'Workspace');
      await tester.pump();
      expect(panelsIn(p.uiState.split), panelsIn(defaultLayout()),
          reason: 'the default arrangement is back');
    });

    /// The bar is the shape of the finished application, not of today's build
    /// (K-244): a command that is specified and unbuilt is still listed, marked
    /// and disabled, so nobody has to guess whether it is missing or broken.
    testWidgets('unbuilt commands are listed, marked and disabled',
        (tester) async {
      await mount(tester);
      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);

      await tester.tap(find.byKey(const ValueKey<String>('menu-Animation')));
      await tester.pump();
      expect(find.text('Keyframe speed… (Not implemented)'), findsOneWidget);
      expect(
        tester
            .widget<Text>(find.text('Keyframe speed… (Not implemented)'))
            .style
            ?.color,
        t.textDisabled,
      );
      await dismiss(tester);

      // Every menu the specification names is on the bar, in its order.
      for (final title in [
        'File',
        'Edit',
        'Composition',
        'Layer',
        'Effect',
        'Animation',
        'View',
        'Window',
        'Help',
      ]) {
        expect(find.byKey(ValueKey<String>('menu-$title')), findsOneWidget,
            reason: '$title is on the bar');
      }
    });

    /// View ▸ Resolution is a real raster reduction (docs/07 §2.2 item 2): it
    /// changes the `scale` every render request carries, so the engine makes
    /// fewer pixels rather than the panel drawing the same ones smaller.
    testWidgets('View ▸ Resolution changes what the engine is asked for',
        (tester) async {
      final p = await mount(tester);
      // The tier is per composition since K-357, so there has to be one.
      await makeComp(tester);
      expect(p.uiState.previewResolution, PreviewResolution.auto,
          reason: 'Auto is the default — it renders what the panel can show');

      // A panel showing a quarter of the comp: Auto follows it, and the fixed
      // tiers do not. That difference is the point of having both.
      p.uiState.reportViewerScale(0.25);
      expect(p.uiState.viewerScale, closeTo(0.25, 1e-9));

      await choose(tester, 'View', 'Half', under: 'Resolution');
      expect(p.uiState.previewResolution, PreviewResolution.half);
      expect(p.uiState.viewerScale, closeTo(0.5, 1e-9),
          reason: 'Half is half of the composition, not of the panel');

      await choose(tester, 'View', 'Quarter', under: 'Resolution');
      expect(p.uiState.viewerScale, closeTo(0.25, 1e-9));

      await choose(tester, 'View', 'Full', under: 'Resolution');
      expect(p.uiState.viewerScale, closeTo(1.0, 1e-9),
          reason: 'Full is comp resolution whatever the panel is showing');

      await choose(tester, 'View', 'Third', under: 'Resolution');
      expect(p.uiState.viewerScale, closeTo(1.0 / 3.0, 1e-9));

      await choose(tester, 'View', 'Auto', under: 'Resolution');
      expect(p.uiState.viewerScale, closeTo(0.25, 1e-9),
          reason: 'and Auto is back to following the panel');
    });

    /// The magnification rows *ask* the Viewer rather than doing it here:
    /// "fit" is a rule only the panel can resolve, and the panel need not even
    /// be mounted for the row to be harmless.
    testWidgets('View ▸ Zoom in asks the Viewer for a magnification',
        (tester) async {
      final p = await mount(tester);
      await makeComp(tester);

      await choose(tester, 'View', 'Zoom in');
      expect(p.uiState.viewerZoomRequest.value?.$2, ViewerZoomCommand.zoomIn);

      // Twice is twice: the serial is what stops a repeated request being
      // swallowed as "no change".
      final first = p.uiState.viewerZoomRequest.value!.$1;
      await choose(tester, 'View', 'Zoom in');
      expect(p.uiState.viewerZoomRequest.value!.$1, greaterThan(first));

      await choose(tester, 'View', 'Fit');
      expect(p.uiState.viewerZoomRequest.value?.$2, ViewerZoomCommand.fit);
    });

    /// Shortcuts are the engine's (K-199): a row shows whatever the keymap
    /// currently binds to its action, so a rebind changes the menus too.
    testWidgets('a row teaches the chord its action answers to',
        (tester) async {
      final p = await mount(tester);

      await tester.tap(find.byKey(const ValueKey<String>('menu-File')));
      await tester.pump();
      expect(find.text('Ctrl+S'), findsOneWidget, reason: 'Save');
      expect(find.text('Ctrl+Shift+S'), findsOneWidget, reason: 'Save as');
      expect(find.text('Ctrl+Alt+N'), findsOneWidget, reason: 'New');
      await dismiss(tester);

      // The row reads the live keymap rather than a chord of its own: the
      // engine is the only place a binding is written down (K-199).
      expect(p.uiState.keymap.chordFor('file.save'), 'Ctrl+S');
      expect(p.uiState.keymap.rawChordFor('file.save'), 'Mod+S');
    });

    /// The Window menu's panel list: ticked when the panel is in the
    /// arrangement, and clicking one adds or drops it. Persistence comes free
    /// — what is stored is the arrangement, and this changes the arrangement.
    ///
    /// **And the menu stays open while you do it** (K-520). Panels are ticked
    /// several at a time, so the row is pressed again here without reopening
    /// anything — which is also what proves the tick redraws in place rather
    /// than showing what it said when the menu was raised.
    testWidgets('the Window menu ticks the panels and toggles them',
        (tester) async {
      final p = await mount(tester);
      expect(panelsIn(p.uiState.split), contains(Panel.scopes));

      await choose(tester, 'Window', Panel.scopes.title);
      await tester.pump();
      expect(panelsIn(p.uiState.split), isNot(contains(Panel.scopes)),
          reason: 'the tick came off and the panel went with it');
      expect(p.uiState.workspace.toJson()['dock'].toString(),
          isNot(contains(Panel.scopes.name)),
          reason: 'the stored arrangement is what persists it');

      expect(find.text(Panel.scopes.title), findsOneWidget,
          reason: 'a toggle row leaves the menu up');
      await tester.tap(find.text(Panel.scopes.title));
      await tester.pump();
      expect(panelsIn(p.uiState.split), contains(Panel.scopes),
          reason: 'and back again, without opening the menu a second time');

      // A row that is not a toggle still closes it.
      await tester.tap(find.text('Command palette…'));
      await tester.pump();
      expect(find.text(Panel.scopes.title), findsNothing,
          reason: 'an ordinary command closes the menu as it always did');
      await dismiss(tester);
    });

    /// **The bar is chrome: it spans the window, one colour, from the left.**
    ///
    /// Making it scroll sideways (so nine headings cannot overflow a narrow
    /// window) made it shrink-wrap to the width of those headings, and the
    /// shell's Column then centred that stub with the backdrop showing either
    /// side. Both symptoms, one cause.
    ///
    /// **The window has to be wider than the headings for this to be visible
    /// at all.** The nine of them come to a little over 800px, so on the
    /// default 800×600 test surface a shrink-wrapped bar is clamped to the full
    /// width and looks perfect — which is exactly how the fault shipped. It is
    /// pumped here at a real window size, in the Column `_LumitAppViewState`
    /// puts it in — that Column is the whole mechanism, because a Column gives
    /// its children *loose* cross-axis constraints, which is what lets a
    /// shrink-wrapping child stay narrow and be centred. (`hostPanel` alone
    /// puts the bar in an Overlay, which forces full width and hides the
    /// fault; the whole `LumitAppNew` reproduces it too, but drags in an
    /// unrelated Debug-panel overflow at this size.)
    testWidgets('the bar spans the window, from the left edge', (tester) async {
      tester.view.physicalSize = const Size(1280, 720);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: Builder(builder: (context) {
          final state = context.watch<LumitState>();
          context.watch<LumitUiState>();
          return Column(children: [LumitMenuBarFrb(app: state)]);
        }),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      final bar = tester.getRect(find.byType(LumitMenuBarFrb));
      expect(bar.left, 0, reason: 'flush to the left edge, not centred');
      expect(bar.width, 1280,
          reason: 'the full width of the window, so one colour spans it');
      expect(
          tester.getTopLeft(find.byKey(const ValueKey<String>('menu-File'))).dx,
          lessThan(20),
          reason: 'File is the first heading, at the left');
    });

    /// The update row is live rather than listed-and-dead (K-296). It is not
    /// *pressed* here: pressing it asks GitHub, and a test suite has no
    /// business on the network — what the press does is `updates_test.dart`,
    /// against a service whose seams are stopped up.
    testWidgets('Help ▸ Check for updates is a built command', (tester) async {
      await mount(tester);
      await tester.tap(find.byKey(const ValueKey<String>('menu-Help')));
      await tester.pump();
      expect(find.text('Check for updates'), findsOneWidget);
      expect(find.text('Check for updates (Not implemented)'), findsNothing);
      await dismiss(tester);
    });

    /// The two documentation rows hand a web address to the desktop (K-279).
    /// The launcher is stopped up: a test suite must never open a browser.
    testWidgets('Help ▸ the documentation rows open the docs site',
        (tester) async {
      final asked = <String>[];
      final real = openExternalLink;
      openExternalLink = (url) async {
        asked.add(url);
        return true;
      };
      addTearDown(() => openExternalLink = real);

      await mount(tester);
      await choose(tester, 'Help', 'Lumit help');
      await tester.pump();
      expect(asked, ['https://docs.lumitlab.com/']);

      await choose(tester, 'Help', 'Lumit online guides');
      await tester.pump();
      expect(asked.last, 'https://docs.lumitlab.com/start/first-composition/');
    });

    /// A machine with no browser registered leaves a row that does nothing,
    /// which reads as broken. It says so in the status line instead.
    testWidgets('a link the desktop will not take says so', (tester) async {
      final real = openExternalLink;
      openExternalLink = (_) async => false;
      addTearDown(() => openExternalLink = real);

      final p = await mount(tester);
      await choose(tester, 'Help', 'Lumit help');
      await tester.pump();
      expect(p.state.notice.value?.message, contains('docs.lumitlab.com'));
      expect(p.state.notice.value?.error, isTrue);
    });

    /// Only a web address is ever handed over, whatever a caller passes.
    test('the launcher refuses anything that is not a web address', () async {
      expect(await launchInDefaultBrowser('file:///etc/passwd'), isFalse);
      expect(await launchInDefaultBrowser('javascript:alert(1)'), isFalse);
      expect(await launchInDefaultBrowser('https://'), isFalse);
      expect(await launchInDefaultBrowser('not a url at all'), isFalse);
    });

    testWidgets('Help ▸ About Lumit opens the About window', (tester) async {
      await mount(tester);
      await choose(tester, 'Help', 'About Lumit');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('about-close')), findsOneWidget);
      // What Settings ▸ General used to say, said here instead (K-244).
      expect(find.textContaining('lumit-bridge'), findsOneWidget);
    });

    /// The Effect menu is the browser as a menu: a submenu per category, each
    /// effect applying to *every* selected layer, and the whole thing dead with
    /// nothing selected.
    testWidgets('the Effect menu applies to every selected layer',
        (tester) async {
      final p = await mount(tester);
      await makeComp(tester);
      final comp = p.uiState.selectedComp!;

      // Nothing selected: the rows are there and do nothing.
      await choose(tester, 'Effect', 'Gaussian blur', under: 'Blur & sharpen');
      await tester.pump();
      expect(comp.getLayers(), isEmpty);

      final a = comp.addSolidLayer();
      final b = comp.addSolidLayer();
      p.uiState.setSelection([a, b]);
      await tester.pump();

      await choose(tester, 'Effect', 'Gaussian blur', under: 'Blur & sharpen');
      await tester.pump();
      expect(a.getEffects().single.name(), 'blur');
      expect(b.getEffects().single.name(), 'blur',
          reason: 'every selected layer, not just the primary (K-217)');
    });

    testWidgets('Open recent lists what the workspace remembers',
        (tester) async {
      final p = await mount(tester);
      p.uiState.workspace.rememberProject('C:/projects/yesterday.lum');
      p.state.notifyDocumentChanged();
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey<String>('menu-File')));
      await tester.pump();
      await tester.tap(find.text('Open recent'));
      await tester.pump();
      expect(find.text('C:/projects/yesterday.lum'), findsOneWidget);
    });

    /// A pointer that can hover, for the two tests below. The menus are driven
    /// by hover as much as by clicks, and a test's synthetic taps carry no
    /// pointer at all unless one is added.
    Future<TestGesture> mouse(WidgetTester tester) async {
      final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await gesture.addPointer(location: Offset.zero);
      addTearDown(gesture.removePointer);
      return gesture;
    }

    /// **Once a menu is open the bar is in menus.** Crossing another heading
    /// hands over to it, rather than leaving the first menu up until it is
    /// clicked away and the second one clicked open.
    testWidgets('a heading hands over to the next one on hover',
        (tester) async {
      await mount(tester);
      final pointer = await mouse(tester);

      // Nothing open: the bar is inert under a passing pointer.
      await pointer
          .moveTo(tester.getCenter(find.byKey(const ValueKey('menu-Edit'))));
      await tester.pump();
      expect(find.text('Redo'), findsNothing,
          reason: 'hover alone must not start dropping menus');

      // Onto the heading being clicked, the way a real pointer arrives: the
      // handover is an *arrival* on a heading, and a pointer that never left
      // Edit has not arrived anywhere.
      await pointer
          .moveTo(tester.getCenter(find.byKey(const ValueKey('menu-File'))));
      await tester.tap(find.byKey(const ValueKey<String>('menu-File')));
      await tester.pump();
      expect(find.text('Open recent'), findsOneWidget);

      await pointer
          .moveTo(tester.getCenter(find.byKey(const ValueKey('menu-Edit'))));
      await tester.pump();
      await tester.pump();
      expect(find.text('Redo'), findsOneWidget, reason: 'Edit took over');
      expect(find.text('Open recent'), findsNothing,
          reason: 'and File went with it');

      await dismiss(tester);
      await pointer
          .moveTo(tester.getCenter(find.byKey(const ValueKey('menu-Layer'))));
      await tester.pump();
      expect(find.text('Pre-compose…'), findsNothing,
          reason: 'dismissed means out of menus again');
    });

    /// A submenu flies out under the pointer and takes itself back when the
    /// pointer moves on to another row — Open recent here, the Effect
    /// categories by the same mechanism.
    testWidgets('a submenu opens on hover and closes when you move off',
        (tester) async {
      final p = await mount(tester);
      p.uiState.workspace.rememberProject('C:/projects/yesterday.lum');
      p.state.notifyDocumentChanged();
      await tester.pump();
      final pointer = await mouse(tester);

      await tester.tap(find.byKey(const ValueKey<String>('menu-File')));
      await tester.pump();

      await pointer.moveTo(tester.getCenter(find.text('Open recent')));
      await tester.pump();
      await tester.pump();
      expect(find.text('C:/projects/yesterday.lum'), findsOneWidget,
          reason: 'resting on the row is enough to see what is behind it');

      await pointer.moveTo(tester.getCenter(find.text('Save')));
      await tester.pump();
      await tester.pump();
      expect(find.text('C:/projects/yesterday.lum'), findsNothing,
          reason: 'the flyout goes back when another row takes the pointer');
      expect(find.text('Open recent'), findsOneWidget,
          reason: 'the menu it flew out of is still up');
    });
  }, skip: !engineAvailable);
}
