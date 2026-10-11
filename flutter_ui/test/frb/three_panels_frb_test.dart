// Effects & presets, Scopes and Hierarchy on frb, against the real engine.
//
// All three were `PlaceholderPanel`s, so there is nothing to migrate; v0 never
// built the preset *listing* at all. What is asserted is that each reaches the
// document — an effect list nothing can apply from is a picture of a panel.

import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effects_presets_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/panels/hierarchy_panel_frb.dart';
import 'package:lumit_flutter/panels/scopes_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Effects & presets (frb)', () {
    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withLayer() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addAdjustmentLayer();
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    Future<void> mount(
      WidgetTester tester,
      dynamic p, {
      Future<String?> Function()? savePicker,
      Future<String?> Function()? loadPicker,
      List<BridgePresetInfo> Function()? presetsLister,
    }) async {
      await tester.pumpWidget(hostPanel(
        child: EffectsPresetsPanelFrb(
          savePicker: savePicker,
          loadPicker: loadPicker,
          // Tests never read the user's real library unless they say so.
          presetsLister: presetsLister ?? () => const [],
        ),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
      ));
      await tester.pump();
    }

    testWidgets('the list is the engine schema, grouped and searchable',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      expect(find.text('Gaussian blur'), findsOneWidget);
      expect(find.byKey(const ValueKey('fx-item-blur')), findsOneWidget);

      await tester.enterText(find.byKey(const ValueKey('fx-search')), 'blur');
      await tester.pump();
      expect(find.byKey(const ValueKey('fx-item-blur')), findsOneWidget);

      await tester.enterText(
          find.byKey(const ValueKey('fx-search')), 'zzz-nothing');
      await tester.pump();
      expect(find.text('No effects match'), findsOneWidget);
    });

    testWidgets('double-clicking applies to the selected layer',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      expect(p.layer.getEffects(), isEmpty);
      // The gap has to be at least `kDoubleTapMinTime`; anything shorter is not
      // a double tap and the row does nothing.
      final row = find.byKey(const ValueKey('fx-item-blur'));
      await tester.tap(row);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(row);
      await tester.pumpAndSettle();

      expect(p.layer.getEffects(), hasLength(1));
      expect(p.layer.getEffects().single.name(), 'blur');
    });

    testWidgets('a preset saves to a file and loads back onto a layer',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      final dir = Directory.systemTemp.createTempSync('lumit-preset');
      final path = '${dir.path}/look.lumfx';

      // Both seams injected once: remounting between the save and the load
      // would replace the tree the tap is about to land in.
      await mount(tester, p,
          savePicker: () async => path, loadPicker: () async => path);
      await tester.tap(find.byKey(const ValueKey('preset-save')));
      await tester.pumpAndSettle();
      expect(File(path).existsSync(), isTrue);
      expect(File(path).readAsStringSync(), contains('blur'));

      // Load it back: the stack grows, and the copy is its own instance.
      final before = p.layer.getEffects().single.id();
      await tester.tap(find.byKey(const ValueKey('preset-load')));
      // The file is read off the interface's thread.
      await settleFrb(tester, until: () => p.layer.getEffects().length == 2);

      final after = p.layer.getEffects();
      expect(after, hasLength(2));
      expect(after[1].id(), isNot(before),
          reason: 'a loaded preset is a fresh instance, never a shared id');
    });

    /// The library listing (docs/TODO: saved presets were not listed at all):
    /// a preset in the library appears under its saved name, the search field
    /// filters it, and a double-click applies its whole stack to the layer.
    testWidgets('a library preset is listed and applies on double-click',
        (tester) async {
      final p = withLayer();
      final dir = Directory.systemTemp.createTempSync('lumit-preset-lib');
      final path = '${dir.path}/glow.lumfx';
      final donor = withLayer();
      donor.layer.addEffect(name: 'blur');
      File(path).writeAsStringSync(donor.layer.savePreset(name: 'Soft glow'));

      await mount(tester, p,
          presetsLister: () =>
              [BridgePresetInfo(name: 'Soft glow', path: path)]);

      expect(find.text('Saved presets'), findsOneWidget);
      final row = find.byKey(const ValueKey('preset-item-Soft glow'));
      expect(row, findsOneWidget);

      await tester.tap(row);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(row);
      await tester.pumpAndSettle();
      await settleFrb(tester, until: () => p.layer.getEffects().isNotEmpty);

      expect(p.layer.getEffects(), hasLength(1));
      expect(p.layer.getEffects().single.name(), 'blur');

      // The search field filters the library too.
      await tester.enterText(
          find.byKey(const ValueKey('fx-search')), 'zzz-nothing');
      await tester.pump();
      expect(find.byKey(const ValueKey('preset-item-Soft glow')), findsNothing);
    });

    /// **Favourites** (owner, desk test). The star was drawn and did nothing.
    /// Starring a row gathers it under a Favourites heading above everything
    /// else, the star toggles from either place, and — because a favourite is
    /// a preference rather than a view state — it is written to the workspace
    /// rather than kept in the widget.
    testWidgets('starring an effect gathers it under Favourites',
        (tester) async {
      final store = '${Directory.systemTemp.createTempSync('lumit-fav').path}'
          '${Platform.pathSeparator}workspace.json';
      Workspace.storeOverride = store;
      addTearDown(() => Workspace.storeOverride = null);

      final p = withLayer();
      await mount(tester, p);

      expect(find.text('Favourites'), findsNothing,
          reason: 'nothing starred, so no standing instruction to star '
              'something');

      await tester.tap(find.byKey(const ValueKey('fx-star-blur')));
      await tester.pumpAndSettle();

      expect(find.text('Favourites'), findsOneWidget);
      expect(find.byKey(const ValueKey('fav-item-blur')), findsOneWidget,
          reason: 'the starred effect is under the new heading');
      expect(find.byKey(const ValueKey('fx-item-blur')), findsOneWidget,
          reason: 'and still in its own category, which is where it lives');
      expect(p.uiState.workspace.isFavouriteEffect('blur'), isTrue,
          reason: 'the star is a preference, so it went to the workspace');

      // It twirls like every other heading.
      await tester.tap(find.byKey(const ValueKey('fx-group-*favourites')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fav-item-blur')), findsNothing);
      expect(find.text('Favourites'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('fx-group-*favourites')));
      await tester.pumpAndSettle();

      // And the star comes off from either row.
      await tester.tap(find.byKey(const ValueKey('fx-star-blur')).first);
      await tester.pumpAndSettle();
      expect(find.text('Favourites'), findsNothing);
      expect(p.uiState.workspace.isFavouriteEffect('blur'), isFalse);
    });

    testWidgets('a file that is not a preset changes nothing', (tester) async {
      final p = withLayer();
      final dir = Directory.systemTemp.createTempSync('lumit-preset-bad');
      final path = '${dir.path}/notes.txt';
      File(path).writeAsStringSync('this is not a preset');

      await mount(tester, p, loadPicker: () async => path);
      await tester.tap(find.byKey(const ValueKey('preset-load')));
      await settleFrb(tester);

      expect(p.layer.getEffects(), isEmpty,
          reason: 'a picker takes any file, so this is a normal thing to do');
    });

  }, skip: !engineAvailable);

  group('Scopes (frb)', () {
    testWidgets('it offers the four traces and waits for one', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addAdjustmentLayer();
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        child: const ScopesPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      // No GPU trace arrives in a widget test, so the panel says what it is
      // doing rather than showing an empty box.
      expect(find.text('Waiting for a trace'), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('scope-kind')));
      await tester.pumpAndSettle();
      for (final label in [
        'Waveform',
        'RGB parade',
        'Vectorscope',
        'Histogram'
      ]) {
        expect(find.text(label), findsWidgets, reason: label);
      }
      await tester.tap(find.text('Histogram').last);
      await tester.pumpAndSettle();
      expect(find.text('Histogram'), findsOneWidget);
    });

  }, skip: !engineAvailable);

  group('Hierarchy (frb)', () {
    testWidgets('it lists the front comp layers and selects one',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final camera = comp.addCameraLayer();
      comp.addTextLayer();
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        child: const HierarchyPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      expect(find.text('Camera'), findsOneWidget);
      expect(find.text('Text'), findsOneWidget);

      await tester.tap(find
          .byKey(ValueKey<String>('hierarchy-row-${camera.internallayerId}')));
      await tester.pump();
      expect(p.uiState.selectedLayer.value?.internallayerId,
          camera.internallayerId);
    });

  }, skip: !engineAvailable);
}
