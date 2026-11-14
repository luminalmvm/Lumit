// A gradient's Blend is in view: the built-in Gradient's and every extension
// gradient's Mix row, with the Blend beside it, is drawn with the effect's own
// rows rather than in the Compositing fold, which starts shut.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/comp_graph_panel.dart' show compBoxRows;
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart'
    show blendInView, compositingRowIds, instanceParameters;
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
            of: find.byKey(ValueKey<String>('fx-row-$id-blend')),
            matching: find.byKey(ValueKey<String>('fx-choice-$id-blend'))),
        findsOneWidget,
        reason: 'the Blend is a row of its own, without opening Compositing',
      );
      expect(
        find.descendant(
            of: find.byKey(ValueKey<String>('fx-row-$id-mix')),
            matching: find.byKey(ValueKey<String>('fx-choice-$id-blend'))),
        findsNothing,
        reason: 'not beside Mix as well',
      );
      // Matte, Invert and Channel too: each a row of its own, in view.
      for (final row in ['matte', 'matte_invert', 'matte_channel']) {
        expect(find.byKey(ValueKey<String>('fx-row-$id-$row')), findsOneWidget,
            reason: '$row is a row of its own, without opening Compositing');
      }
      final ids = [for (final r in instanceParameters(effect.getInfo())) r.id];
      expect(
          ids.sublist(ids.length - compositingRowIds.length), compositingRowIds,
          reason: 'the compositing rows come after the effect\'s own rows');

      // A shut node graph box keeps them, the Matte as its socket.
      final info = effect.getInfo();
      final box = compBoxRows(
        name,
        instanceParameters(info),
        {for (final v in info.values) v.id: v.value},
        const {},
      );
      final shut = [for (final r in box.telling) r.id];
      expect(shut,
          containsAllInOrder(['mix', 'blend', 'matte_invert', 'matte_channel']),
          reason: 'a shut box shows them at their defaults');
      expect(shut, isNot(contains('matte')),
          reason: 'the Matte is the box\'s socket');
      expect(box.riders['mix'], isNull,
          reason: 'the box draws the Blend once, as its own row');
      expect(box.riders['matte'], isNull);

      // The Timeline lists what the panel lists: the compositing rows, and
      // no colour the Colours count leaves out.
      final fold = [
        for (final r in foldEffectParams(
            info, {for (final v in info.values) v.id: v.value}))
          r.id
      ];
      expect(fold, containsAllInOrder(compositingRowIds));
      if (name == 'gradient') {
        expect(fold, isNot(contains('colour_2')),
            reason: 'Colours is Two: no colour in between');
      }
    });
  }
}
