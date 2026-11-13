// A gradient's Blend is in view: the built-in Gradient's and every extension
// gradient's Mix row, with the Blend beside it, is drawn with the effect's own
// rows rather than in the Compositing fold, which starts shut.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/comp_graph_panel.dart' show compBoxRows;
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart'
    show blendInView, instanceParameters;
import 'package:lumit_flutter/panels/layer_fold_frb.dart' show foldEffectParams;
import 'package:lumit_flutter/src/rust/api/extensions.dart'
    show extensionRescan;

import 'frb_test_support.dart';

void main() {
  setUpAll(() async {
    await initEngineForTests();
    await extensionRescan();
  });

  const gradients = [
    'gradient',
    'lfx:siceliot/conical-gradient',
    'lfx:siceliot/sweep-gradient',
    'lfx:siceliot/multi-point-gradient',
    'lfx:siceliot/gradient-map',
  ];

  test('the gradients are the effects whose Blend is in view', () {
    for (final name in gradients) {
      expect(blendInView(name), isTrue, reason: name);
    }
    expect(blendInView('blur'), isFalse);
    expect(blendInView('lfx:siceliot/voronoi'), isFalse);
  });

  for (final name in gradients) {
    testWidgets('$name shows its Blend on its Mix row, after its own rows',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: name);
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

      final effect = layer.getEffects().single;
      final id = effect.id();
      expect(
        find.descendant(
            of: find.byKey(ValueKey<String>('fx-row-$id-mix')),
            matching: find.byKey(ValueKey<String>('fx-choice-$id-blend'))),
        findsOneWidget,
        reason: 'the Blend is in view without opening Compositing',
      );
      final ids = [for (final r in instanceParameters(effect.getInfo())) r.id];
      final mix = ids.indexOf('mix');
      expect(ids[mix + 1], 'blend');
      final matte = ids.indexOf('matte');
      expect(mix, matte < 0 ? ids.length - 2 : matte - 2,
          reason: 'Mix and Blend come after the effect\'s own rows');

      // A shut node graph box keeps the Mix row, and the Blend after it.
      final info = effect.getInfo();
      final box = compBoxRows(
        name,
        instanceParameters(info),
        {for (final v in info.values) v.id: v.value},
        const {},
      );
      expect([for (final r in box.telling) r.id],
          containsAllInOrder(['mix', 'blend']),
          reason: 'a shut box shows the Blend at its defaults');

      // The Timeline lists what the panel lists: the Blend, and no colour
      // the Colours count leaves out.
      final fold = [
        for (final r in foldEffectParams(
            info, {for (final v in info.values) v.id: v.value}))
          r.id
      ];
      expect(fold, contains('blend'));
      if (name == 'gradient') {
        expect(fold, isNot(contains('colour_2')),
            reason: 'Colours is Two: no colour in between');
      }
    });
  }
}
