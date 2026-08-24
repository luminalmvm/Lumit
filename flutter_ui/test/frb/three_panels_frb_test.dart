// Effects & presets, Scopes and Hierarchy on frb, against the real engine.
//
// All three were `PlaceholderPanel`s, so there is nothing to migrate; v0 never
// built the preset *listing* at all. What is asserted is that each reaches the
// document — an effect list nothing can apply from is a picture of a panel.

import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/lumit_icon.dart' as glyph;
import 'package:lumit_flutter/icons/lumit_icons.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effects_presets_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/state/drag_payloads.dart';
import 'package:lumit_flutter/panels/hierarchy_panel_frb.dart';
import 'package:lumit_flutter/panels/scopes_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/widgets/controls.dart';

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

    testWidgets('effect rows are draggable, carrying EffectDragData',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);
      expect(find.byType(Draggable<EffectDragData>), findsWidgets);
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
      await tester.pumpAndSettle();

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

      expect(p.layer.getEffects(), hasLength(1));
      expect(p.layer.getEffects().single.name(), 'blur');

      // The search field filters the library too.
      await tester.enterText(
          find.byKey(const ValueKey('fx-search')), 'zzz-nothing');
      await tester.pump();
      expect(find.byKey(const ValueKey('preset-item-Soft glow')), findsNothing);
    });

    /// **Every heading twirls.** The category folds its effects away and lets
    /// them back, and the saved-preset group does the same.
    testWidgets('a category twirls its effects away and back', (tester) async {
      final p = withLayer();
      final dir = Directory.systemTemp.createTempSync('lumit-preset-fold');
      final path = '${dir.path}/glow.lumfx';
      final donor = withLayer();
      donor.layer.addEffect(name: 'blur');
      File(path).writeAsStringSync(donor.layer.savePreset(name: 'Soft glow'));
      await mount(tester, p,
          presetsLister: () =>
              [BridgePresetInfo(name: 'Soft glow', path: path)]);

      expect(find.byKey(const ValueKey('fx-item-blur')), findsOneWidget,
          reason: 'every category arrives open');

      await tester.tap(find.byKey(const ValueKey('fx-group-blur_sharpen')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-item-blur')), findsNothing,
          reason: 'a shut category shows its heading and nothing else');
      expect(
          find.byKey(const ValueKey('fx-group-blur_sharpen')), findsOneWidget,
          reason: 'the heading stays — it is the way back');

      await tester.tap(find.byKey(const ValueKey('fx-group-blur_sharpen')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-item-blur')), findsOneWidget);

      // The saved-preset group folds by the same gesture.
      await tester.tap(find.byKey(const ValueKey('fx-group-*saved-presets')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('preset-item-Soft glow')), findsNothing);
      expect(find.text('Saved presets'), findsOneWidget);
    });

    /// Each heading remembers its own state: folding one leaves the rest as
    /// they were.
    testWidgets('a fold is one category\'s, not the panel\'s', (tester) async {
      final p = withLayer();
      await mount(tester, p);

      // Whatever the engine's first non-blur effect is, so the test does not
      // hold a second copy of the schema.
      final other = listEffects()
          .firstWhere((effect) => effect.category != 'blur_sharpen');

      await tester.tap(find.byKey(const ValueKey('fx-group-blur_sharpen')));
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('fx-item-blur')), findsNothing);
      expect(
          find.byKey(ValueKey<String>('fx-item-${other.name}')), findsOneWidget,
          reason: 'the other categories were not touched');
    });

    /// **A search overrides the folds** — a search that hides what it found is
    /// a trap — and clearing it puts them back exactly as they were.
    testWidgets('a live search shows matches inside shut categories',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      await tester.tap(find.byKey(const ValueKey('fx-group-blur_sharpen')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-item-blur')), findsNothing);

      await tester.enterText(find.byKey(const ValueKey('fx-search')), 'blur');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-item-blur')), findsOneWidget,
          reason: 'a match shows whatever the heading over it was doing');

      await tester.enterText(find.byKey(const ValueKey('fx-search')), '');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-item-blur')), findsNothing,
          reason: 'the fold was remembered through the search, not discarded');
    });

    /// The glyph is the set's twirl: right while the category is shut, down
    /// while it is open — the same pair the Timeline folds with.
    testWidgets('the heading wears the set\'s twirl', (tester) async {
      final p = withLayer();
      await mount(tester, p);

      String twirl() => tester
          .widget<glyph.LumitIcon>(find
              .descendant(
                of: find.byKey(const ValueKey('fx-group-blur_sharpen')),
                matching: find.byType(glyph.LumitIcon),
              )
              .first)
          .glyph;

      expect(twirl(), LumitIcons.collapse,
          reason: 'open is the triangle pointing down');

      await tester.tap(find.byKey(const ValueKey('fx-group-blur_sharpen')));
      await tester.pumpAndSettle();
      expect(twirl(), LumitIcons.expand,
          reason: 'shut is the triangle pointing right');
    });

    testWidgets('a file that is not a preset changes nothing', (tester) async {
      final p = withLayer();
      final dir = Directory.systemTemp.createTempSync('lumit-preset-bad');
      final path = '${dir.path}/notes.txt';
      File(path).writeAsStringSync('this is not a preset');

      await mount(tester, p, loadPicker: () async => path);
      await tester.tap(find.byKey(const ValueKey('preset-load')));
      await tester.pumpAndSettle();

      expect(p.layer.getEffects(), isEmpty,
          reason: 'a picker takes any file, so this is a normal thing to do');
    });

    testWidgets('without a layer the preset buttons are inert', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      await tester.pumpWidget(hostPanel(
        child: const EffectsPresetsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      expect(find.text('Select a layer'), findsOneWidget);
      // Tapping does nothing rather than raising — no picker opens.
      await tester.tap(find.byKey(const ValueKey('preset-save')));
      await tester.pump();
    });
  }, skip: !engineAvailable);

  group('Scopes (frb)', () {
    testWidgets('without a composition it says so', (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: const ScopesPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      expect(find.textContaining('Select a composition'), findsOneWidget);
    });

    /// The narrow-dock rule (docs/TODO shell): a toolbar that does not fit
    /// scrolls sideways instead of painting the overflow stripe — which in a
    /// test surfaces as a thrown RenderFlex exception, so none is the pass.
    testWidgets('a narrow dock scrolls the toolbar instead of striping',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addAdjustmentLayer();
      p.uiState.setSelectedComp(comp);
      await tester.pumpWidget(hostPanel(
        child: const ScopesPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(140, 300),
      ));
      await tester.pump();
      expect(tester.takeException(), isNull,
          reason: 'no overflow at 140 px wide');
    });

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

    /// Five triples, from the theme — the engine refuses anything else, and a
    /// panel that sent four would draw nothing with no visible reason.
    testWidgets('the theme supplies exactly five colour triples',
        (tester) async {
      late List<Object> colours;
      await tester.pumpWidget(hostPanel(
        child: Builder(builder: (context) {
          colours = scopeColoursFor(ThemeScope.of(context).theme);
          return const SizedBox.shrink();
        }),
        state: freshProject().state,
        uiState: freshProject().uiState,
      ));
      await tester.pump();

      expect(colours, hasLength(5));
      for (final triple in colours) {
        expect((triple as List).length, 3);
      }
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

    testWidgets('a precomp layer expands to show what is inside it',
        (tester) async {
      final p = freshProject();
      final inner = p.state.project!.newComposition(name: 'Inner');
      inner.addCameraLayer();
      final outer = p.state.project!.newComposition(name: 'Outer');
      // A precomp layer is a composition placed into another comp.
      outer.addPrecompLayer(comp: inner);
      p.uiState.setSelectedComp(outer);

      await tester.pumpWidget(hostPanel(
        child: const HierarchyPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      expect(find.text('precomp'), findsOneWidget);
      expect(find.text('Camera'), findsNothing,
          reason: 'a closed precomp does not show its insides');

      final row = outer.getLayers().single;
      await tester.tap(
          find.byKey(ValueKey<String>('hierarchy-row-${row.internallayerId}')));
      await tester.pump();
      // The twirl is the small target at the row's left edge.
      final twirl = tester.getTopLeft(find.byKey(
              ValueKey<String>('hierarchy-row-${row.internallayerId}'))) +
          const Offset(13, 11);
      await tester.tapAt(twirl);
      await tester.pumpAndSettle();

      expect(find.text('Camera'), findsOneWidget,
          reason: 'the nested comp layers appear, indented');
    });

    testWidgets('an empty composition says so', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        child: const HierarchyPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      expect(find.textContaining('no layers yet'), findsOneWidget);
    });
  }, skip: !engineAvailable);
}
