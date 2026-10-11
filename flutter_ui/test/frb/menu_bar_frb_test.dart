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

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/engine_labels.dart';
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
      ThemeShape shape = ThemeShape.studio,
    }) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        shape: shape,
        density: DensityTokens.forShape(shape, false),
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
    /// first — Window → Workspaces → Audio.
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

    /// New composition asks for its settings first, so every route to a
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

    testWidgets('Copy and Paste carry a layer, landing it at the playhead',
        (tester) async {
      // Copy takes the selected layer whole and Paste puts it in the comp on
      // screen, at the playhead. The engine does the carrying; what is tested
      // here is that the menu is wired to it and to the setting.
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

    testWidgets('the layer console applies an effect to the primary layer only',
        (tester) async {
      final p = await mount(tester);
      final comp = p.state.project!.newComposition(name: 'Scene');
      final a = comp.addSolidLayer();
      final b = comp.addSolidLayer();
      p.uiState
        ..setSelectedComp(comp)
        ..setSelection([a, b]);
      await tester.pump();

      p.uiState.requestConsole();
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey('fx-console-query')), 'Gaussian blur');
      await tester.pump();
      await tester
          .tap(find.byKey(const ValueKey('fx-console-item-Gaussian blur')));
      await tester.pumpAndSettle();

      expect(a.getEffects().single.name(), 'blur');
      expect(b.getEffects(), isEmpty,
          reason: 'the primary layer alone, not every selected layer');
    });

    testWidgets('Import footage imports every picked path', (tester) async {
      final p = await mount(
        tester,
        footagePicker: () async => ['C:/clips/a.mov', 'C:/clips/b.mov'],
      );

      await choose(tester, 'File', 'Import footage…');
      // The files are read off the interface's thread.
      await settleFrb(tester,
          until: () =>
              allItems(p.state).whereType<ItemReference_Footage>().length >= 2);

      final names = allItems(p.state)
          .whereType<ItemReference_Footage>()
          .map((f) => f.name())
          .toList();
      expect(names, containsAll(<String>['a.mov', 'b.mov']));
    });

    testWidgets('Undo and Redo grey out with the document history',
        (tester) async {
      final p = await mount(tester);
      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.studio);

      Color? colourOf(String label) =>
          tester.widget<Text>(find.text(label)).style?.color;

      // A fresh document has nothing either way.
      await tester.tap(find.byKey(const ValueKey<String>('menu-Edit')));
      await tester.pump();
      expect(colourOf('Undo'), t.textDisabled);
      expect(colourOf('Redo'), t.textDisabled);
      await dismiss(tester);

      // One edit, and Undo lights up and names it, in the History window's
      // own words.
      await makeComp(tester);
      expect(p.state.project!.history().canUndo, isTrue);
      final step = engineLabel(p.state.project!.historyEntries().last.name);

      await tester.tap(find.byKey(const ValueKey<String>('menu-Edit')));
      await tester.pump();
      expect(colourOf('Undo $step'), isNot(t.textDisabled),
          reason:
              'an item you can see is disabled tells you the document state');
      await tester.tap(find.text('Undo $step'));
      await tester.pump();

      expect(p.state.project!.getItems(), isEmpty,
          reason: 'Undo reached the engine, not just the menu');
      expect(p.state.project!.history().canRedo, isTrue);

      // Undone: the pair swaps over, and the name goes with it.
      await tester.tap(find.byKey(const ValueKey<String>('menu-Edit')));
      await tester.pump();
      expect(colourOf('Undo'), t.textDisabled);
      expect(colourOf('Redo $step'), isNot(t.textDisabled));
      await tester.tap(find.text('Redo $step'));
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

    // Text to shapes and Text to points: the copy lands beside the
    // original, which is still there and still a Type layer.
    testWidgets('Layer ▸ Create turns a text layer into shapes and into points',
        (tester) async {
      final p = await mount(tester);
      await makeComp(tester);
      final comp = p.uiState.selectedComp!;
      final text = comp.addTextLayer();
      p.uiState.setSelection([text]);
      await tester.pump();

      await choose(tester, 'Layer', 'Shapes from text', under: 'Create');
      await tester.pump();
      expect(comp.getLayers(), hasLength(2), reason: 'a copy beside it');
      expect(text.getText()!.text, 'Text',
          reason: 'the original is kept, still saying what it said');

      await choose(tester, 'Layer', 'Points from text', under: 'Create');
      await tester.pump();
      expect(comp.getLayers(), hasLength(3));
      expect(
          comp
              .getLayers()
              .any((l) => l.getEffects().any((e) => e.name() == 'emit_from_image')),
          isTrue,
          reason: 'the points copy emits from the words');
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

    /// The four shipped workspace presets (docs/07 §1.6): each rearranges the
    /// dock to its factory layout; the same panel inventory throughout, and a
    /// distinct arrangement per preset.
    testWidgets('the Window menu applies the four workspace presets',
        (tester) async {
      final p = await mount(tester);

      // The presets live under their own heading now.
      await choose(tester, 'Window', 'Effects', under: 'Workspace');
      await tester.pump();
      expect(panelsIn(p.uiState.split),
          panelsIn(presetLayout(WorkspacePreset.effects)));
      expect(p.uiState.split.toJson(),
          isNot(presetLayout(WorkspacePreset.colour).toJson()),
          reason: 'the presets are genuinely different arrangements');

      // 'Audio' now names two rows of the Window menu — the workspace under
      // its heading, and the Audio *panel* in the tick list — so this taps
      // the flyout's own row, which is the one on top.
      await tester.tap(find.byKey(const ValueKey<String>('menu-Window')));
      await tester.pump();
      await tester.tap(find.text('Workspace'));
      await tester.pump();
      await tester.tap(find.text('Audio').last);
      await tester.pump();
      expect(p.uiState.split.toJson(),
          presetLayout(WorkspacePreset.audio).toJson());

      // Reset still means the default (Edit) arrangement.
      await choose(tester, 'Window', 'Reset workspace', under: 'Workspace');
      await tester.pump();
      expect(panelsIn(p.uiState.split), panelsIn(defaultLayout()));
    });

    /// View ▸ Resolution is a real raster reduction (docs/07 §2.2 item 2): it
    /// changes the `scale` every render request carries, so the engine makes
    /// fewer pixels rather than the panel drawing the same ones smaller.
    testWidgets('View ▸ Resolution changes what the engine is asked for',
        (tester) async {
      final p = await mount(tester);
      // The tier is per composition, so there has to be one.
      await makeComp(tester);
      expect(p.uiState.previewResolution, PreviewResolution.full,
          reason: 'Full is the default — comp resolution whatever the '
              'panel happens to be showing');

      // A panel showing a quarter of the comp: Auto follows it, and the fixed
      // tiers do not. That difference is the point of having both.
      p.uiState.reportViewerScale(0.25);
      expect(p.uiState.viewerScale, closeTo(1.0, 1e-9),
          reason: 'the default does not follow the panel down');

      // **The submenu is opened once**: a resolution row is an option
      // row, so it leaves the menu up and the next tier is one tap away. Five
      // tiers, one opening — which is the whole point of the convention.
      await tester.tap(find.byKey(const ValueKey('menu-View')));
      await tester.pump();
      await tester.tap(find.text('Resolution'));
      await tester.pump();

      Future<void> pick(String tier) async {
        await tester.tap(find.text(tier));
        await tester.pump();
      }

      await pick('Half');
      expect(p.uiState.previewResolution, PreviewResolution.half);
      expect(p.uiState.viewerScale, closeTo(0.5, 1e-9),
          reason: 'Half is half of the composition, not of the panel');
      expect(find.text('Quarter'), findsOneWidget,
          reason: 'and the menu is still there to pick the next one from');

      await pick('Quarter');
      expect(p.uiState.viewerScale, closeTo(0.25, 1e-9));

      await pick('Full');
      expect(p.uiState.viewerScale, closeTo(1.0, 1e-9),
          reason: 'Full is comp resolution whatever the panel is showing');

      await pick('Third');
      expect(p.uiState.viewerScale, closeTo(1.0 / 3.0, 1e-9));

      await pick('Auto');
      expect(p.uiState.viewerScale, closeTo(0.25, 1e-9),
          reason: 'and Auto is back to following the panel');

      await dismiss(tester);
      expect(find.text('Quarter'), findsNothing,
          reason: 'a click away still takes the menu down');
    });

    /// The Window menu's panel list: ticked when the panel is in the
    /// arrangement, and clicking one adds or drops it. Persistence comes free
    /// — what is stored is the arrangement, and this changes the arrangement.
    ///
    /// **And the menu stays open while you do it**. Panels are ticked
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

    /// Only a web address is ever handed over, whatever a caller passes.
    test('the launcher refuses anything that is not a web address', () async {
      expect(await launchInDefaultBrowser('file:///etc/passwd'), isFalse);
      expect(await launchInDefaultBrowser('javascript:alert(1)'), isFalse);
      expect(await launchInDefaultBrowser('https://'), isFalse);
      expect(await launchInDefaultBrowser('not a url at all'), isFalse);
    });

    /// The Effect menu is the browser as a menu: a submenu per category, each
    /// effect applying to the primary layer only, and the whole thing dead with
    /// nothing selected.
    testWidgets('the Effect menu applies to the primary layer only',
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
      expect(b.getEffects(), isEmpty,
          reason: 'the primary layer alone, not every selected layer');
    });

  }, skip: !engineAvailable);
}
