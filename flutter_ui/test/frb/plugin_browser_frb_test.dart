// A discovered plugin, as the two panels that show one draw it (docs/12 §2.6).
//
// The engine's own scan has its tests where it lives
// (`crates/lumit-ofx/tests/discover.rs`, `crates/lumit-bridge`'s
// `a_discovered_plugin_lists_under_its_own_grouping_with_its_provenance`). What
// is asserted here is the frontend's whole share of the feature: that a plugin
// groups and folds under its **own** declared heading like any category, that
// the search finds it, that its row says where it came from, and that an effect
// whose plugin has died or been switched off wears a calm badge rather than
// stopping anything.
//
// The catalogue is injected, so none of this depends on the machine running the
// tests having an OFX plugin installed — which no CI machine does.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/effects_presets_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart'
    show ThemeScope, closeLumitPopups;

import 'frb_test_support.dart';

/// One catalogue entry, as `list_effects` hands it over.
BridgeEffectInfo entry({
  required String name,
  required String label,
  required String category,
  required String categoryLabel,
  required String namespace,
}) =>
    BridgeEffectInfo(
      name: name,
      label: label,
      category: category,
      categoryLabel: categoryLabel,
      namespace: namespace,
      inputs: const [],
      outputs: const [],
    );

/// One built-in and two plugins: one that declared a grouping, one that did
/// not — the two headings the panel has to find words for.
List<BridgeEffectInfo> aCatalogueWithPlugins() => [
      entry(
        name: 'blur',
        label: 'Gaussian blur',
        category: 'blur_sharpen',
        categoryLabel: 'Blur & sharpen',
        namespace: 'builtin',
      ),
      entry(
        name: 'ofx:com.example.wobbler',
        label: 'Wobbler',
        category: 'ofx/Example/Distort',
        categoryLabel: 'Example/Distort',
        namespace: 'ofx',
      ),
      entry(
        name: 'ofx:com.example.plain',
        label: 'Plain thing',
        category: 'ofx',
        categoryLabel: '',
        namespace: 'ofx',
      ),
      // An audio plugin (AP5): one Audio plugins group for the lot, unheaded
      // by the engine so the panel words it, beside the OFX groups.
      entry(
        name: 'clap:com.example.eq',
        label: 'Example EQ',
        category: 'audio',
        categoryLabel: '',
        namespace: 'audio',
      ),
    ];

void main() {
  setUpAll(initEngineForTests);

  group('the plugin browser (frb)', () {
    Future<void> mount(WidgetTester tester, dynamic p) async {
      await tester.pumpWidget(hostPanel(
        child: EffectsPresetsPanelFrb(
          presetsLister: () => const [],
          effectsLister: aCatalogueWithPlugins,
        ),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
      ));
      await tester.pump();
    }

    testWidgets('a plugin heads its own group, and the group twirls shut',
        (tester) async {
      final p = freshProject();
      await mount(tester, p);

      // Under the plugin's own declared menu path, not under one of ours.
      expect(find.text('Example/Distort'), findsOneWidget);
      expect(find.byKey(const ValueKey('fx-item-ofx:com.example.wobbler')),
          findsOneWidget);

      // A plugin that declared no grouping gets the panel's own word for it,
      // rather than a blank heading.
      expect(find.text('Plugins'), findsOneWidget);

      // And it folds exactly as a category does.
      await tester
          .tap(find.byKey(const ValueKey('fx-group-ofx/Example/Distort')));
      await tester.pump();
      expect(find.byKey(const ValueKey('fx-item-ofx:com.example.wobbler')),
          findsNothing,
          reason: 'right is shut, the same twirl every heading has');
      expect(find.byKey(const ValueKey('fx-item-blur')), findsOneWidget,
          reason: 'and folding one heading leaves the others alone');

      await tester
          .tap(find.byKey(const ValueKey('fx-group-ofx/Example/Distort')));
      await tester.pump();
      expect(find.byKey(const ValueKey('fx-item-ofx:com.example.wobbler')),
          findsOneWidget);
    });

    testWidgets('the search finds a plugin by its name and by its heading',
        (tester) async {
      final p = freshProject();
      await mount(tester, p);

      await tester.enterText(
          find.byKey(const ValueKey('fx-search')), 'wobbler');
      await tester.pump();
      expect(find.byKey(const ValueKey('fx-item-ofx:com.example.wobbler')),
          findsOneWidget);
      expect(find.byKey(const ValueKey('fx-item-blur')), findsNothing);

      // The heading is searchable too, which for a plugin is the vendor's own
      // menu path — often the only word somebody remembers.
      await tester.enterText(
          find.byKey(const ValueKey('fx-search')), 'distort');
      await tester.pump();
      expect(find.byKey(const ValueKey('fx-item-ofx:com.example.wobbler')),
          findsOneWidget);

      await tester.enterText(find.byKey(const ValueKey('fx-search')), 'blur');
      await tester.pump();
      expect(find.byKey(const ValueKey('fx-item-ofx:com.example.wobbler')),
          findsNothing);
    });

    testWidgets(
        'a row says where it came from, and only a plugin can be '
        'switched off', (tester) async {
      final p = freshProject();
      await mount(tester, p);

      Future<void> rightClick(String name) async {
        final gesture = await tester.startGesture(
            tester.getCenter(find.byKey(
              ValueKey<String>('fx-item-$name'),
            )),
            kind: PointerDeviceKind.mouse,
            buttons: kSecondaryMouseButton);
        await gesture.up();
        await tester.pump();
      }

      await rightClick('ofx:com.example.wobbler');
      expect(
          find.byKey(const ValueKey('fx-provenance-ofx:com.example.wobbler')),
          findsOneWidget);
      expect(find.text('From an OpenFX plugin'), findsOneWidget);
      expect(find.text('Switch this plugin off'), findsOneWidget);

      // Away, and then the built-in: the same menu, one line, no command —
      // there is nothing to switch off about an effect that ships with Lumit.
      closeLumitPopups();
      await tester.pump();
      await rightClick('blur');
      expect(find.text('Built in'), findsOneWidget);
      expect(find.text('Switch this plugin off'), findsNothing);
    });

    /// The browser's share of AP5: an audio plugin lists under the one Audio
    /// plugins heading, a group that folds like any other, and its context
    /// menu says where it came from and offers the switch.
    testWidgets(
        'an audio plugin heads the Audio plugins group and can be '
        'switched off', (tester) async {
      final p = freshProject();
      await mount(tester, p);

      expect(find.text('Audio plugins'), findsOneWidget);
      expect(find.byKey(const ValueKey('fx-item-clap:com.example.eq')),
          findsOneWidget);

      // The group folds exactly as a category does.
      await tester.tap(find.byKey(const ValueKey('fx-group-audio')));
      await tester.pump();
      expect(find.byKey(const ValueKey('fx-item-clap:com.example.eq')),
          findsNothing);
      await tester.tap(find.byKey(const ValueKey('fx-group-audio')));
      await tester.pump();

      final gesture = await tester.startGesture(
          tester.getCenter(
              find.byKey(const ValueKey('fx-item-clap:com.example.eq'))),
          kind: PointerDeviceKind.mouse,
          buttons: kSecondaryMouseButton);
      await gesture.up();
      await tester.pump();
      expect(find.text('From an audio plugin'), findsOneWidget,
          reason: 'the provenance names the kind of plugin');
      expect(find.text('Switch this plugin off'), findsOneWidget,
          reason: 'the disable toggle is per plugin, in the same menu');
      closeLumitPopups();
      await tester.pump();
    });
  }, skip: !engineAvailable);

  group('the Audio group in Effect controls (frb)', () {
    /// A layer whose stack holds one picture effect and one audio plugin —
    /// the plugin as a saved project would carry it, so no plugin need be
    /// installed on the machine running this (docs/12 §1's placeholder).
    ({LumitState state, LumitUiState uiState, dynamic layer}) withRack() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      layer.loadPreset(
          text: '{"format":1,"name":"rack","effects":[{'
              '"id":"018f0000-0000-7000-8000-000000000001",'
              '"effect":{"namespace":"Clap",'
              '"match_name":"clap:com.example.eq","version":1},'
              '"enabled":true,"params":[]}]}');
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    testWidgets(
        'an audio plugin card sits under the Audio heading, wears the calm '
        'badge for a missing plugin, and the heading twirls', (tester) async {
      final p = withRack();
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      expect(find.byKey(const ValueKey('fx-audio-group')), findsOneWidget,
          reason: 'a stack with an audio entry grows the Audio heading');
      expect(find.byKey(const ValueKey('fx-card-1')), findsOneWidget,
          reason: 'the plugin card is an ordinary card, at its stack index');
      expect(find.byKey(const ValueKey('fx-card-0')), findsOneWidget,
          reason: 'the picture effect stays in the stack above');

      // The machine running this has no such plugin, so the card wears the
      // calm badge the OFX grammar defined — rows kept, nothing lost.
      expect(find.text('This plugin is not installed on this machine'),
          findsOneWidget);

      // The heading twirls its rack away and back, leaving the stack alone.
      await tester.tap(find.byKey(const ValueKey('fx-audio-group')));
      await tester.pump();
      expect(find.byKey(const ValueKey('fx-card-1')), findsNothing);
      expect(find.byKey(const ValueKey('fx-card-0')), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('fx-audio-group')));
      await tester.pump();
      expect(find.byKey(const ValueKey('fx-card-1')), findsOneWidget);
    });

    testWidgets('a stack with no audio entry grows no Audio heading',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      p.uiState.model.refresh();
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      expect(find.byKey(const ValueKey('fx-audio-group')), findsNothing);
    });
  }, skip: !engineAvailable);

  group('the effect badge', () {
    /// The badge alone, with no engine and no card around it — it is a pure
    /// function of the two fields the read model carries.
    Future<void> pumpBadge(
      WidgetTester tester, {
      String? reason,
      String? detail,
    }) async {
      await tester.pumpWidget(Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Builder(
            builder: (context) =>
                effectBadgeRow(context,
                    id: 'one', reason: reason, detail: detail) ??
                const SizedBox.shrink(),
          ),
        ),
      ));
      await tester.pump();
    }

    testWidgets('a switched-off plugin says so, calmly', (tester) async {
      await pumpBadge(tester, reason: 'plugin_disabled');
      expect(find.byKey(const ValueKey('fx-badge-one')), findsOneWidget);
      expect(find.text('This plugin is switched off'), findsOneWidget);
      expect(find.byKey(const ValueKey('fx-badge-detail-one')), findsNothing,
          reason: 'a plugin nobody asked to run has nothing to explain');
    });

    testWidgets('a failed plugin carries its own words underneath',
        (tester) async {
      await pumpBadge(
        tester,
        reason: 'plugin_failed',
        detail: 'nothing, before the deadline',
      );
      expect(
          find.text('This plugin did not render this frame'), findsOneWidget);
      expect(find.text('nothing, before the deadline'), findsOneWidget);
    });

    testWidgets('a missing plugin and an unknown effect are told apart',
        (tester) async {
      await pumpBadge(tester, reason: 'plugin_missing');
      expect(find.text('This plugin is not installed on this machine'),
          findsOneWidget);

      await pumpBadge(tester, reason: 'unknown_effect');
      expect(find.text('This build does not know this effect'), findsOneWidget);
    });

    testWidgets('and an effect that is behaving wears nothing at all',
        (tester) async {
      await pumpBadge(tester);
      expect(find.byKey(const ValueKey('fx-badge-one')), findsNothing);

      // A reason from a newer engine that this build has no words for draws
      // nothing rather than a raw key.
      await pumpBadge(tester, reason: 'something_new');
      expect(find.byKey(const ValueKey('fx-badge-one')), findsNothing);
    });
  });
}
