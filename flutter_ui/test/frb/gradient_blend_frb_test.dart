// A gradient's Mix, Blend, Matte, Invert and Channel sit in the Compositing
// fold, as every other effect's do: in the Effect controls panel, in the
// Timeline and on a node graph box.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/comp_graph_panel.dart' show compBoxRows;
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart'
    show compositingRowIds, instanceParameters;
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
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

  for (final name in gradients) {
    testWidgets('$name keeps its compositing rows in the Compositing fold',
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
        // Tall enough for every row of the longest gradient to be on screen.
        size: const Size(480, 2400),
      ));
      await tester.pump();

      final effect = layer.getEffects().single;
      final id = effect.id();
      final info = effect.getInfo();
      final values = {for (final v in info.values) v.id: v.value};

      // The panel: the fold's heading, shut, and none of its rows.
      final heading = find.byKey(ValueKey<String>('fx-compositing-$id'));
      expect(heading, findsOneWidget);
      for (final row in compositingRowIds) {
        expect(find.byKey(ValueKey<String>('fx-row-$id-$row')), findsNothing,
            reason: '$row waits in the fold');
      }
      // Opened, each is a row of its own.
      tester.widget<GestureDetector>(heading).onTap!();
      await tester.pump();
      await tester.pump();
      for (final row in compositingRowIds) {
        expect(find.byKey(ValueKey<String>('fx-row-$id-$row')), findsOneWidget,
            reason: '$row is a row of its own in the fold');
      }

      // The Timeline: the same fold, at the foot, with the same rows.
      final path = effectPath('layer', '$id');
      List<String> fold(Set<String> open) => [
            for (final r in effectFoldRows(info, values,
                path: path,
                depth: 3,
                open: open,
                row: (param, depth) => FoldEffectParamRow(
                    info, param, values[param.id],
                    depth: depth)))
              switch (r) {
                FoldEffectParamRow(:final param) => param.id,
                FoldGroupRow(:final label) => '[$label]',
                _ => '?',
              }
          ];
      expect(fold(const {}).last, '[Compositing]');
      final opened = fold({effectGroupPath(path, 'compositing')});
      expect(
          opened.sublist(opened.indexOf('[Compositing]') + 1), compositingRowIds);
      if (name == 'gradient') {
        expect(opened, isNot(contains('colour_2')),
            reason: 'Colours is Two: no colour in between');
      }

      // A node box: nothing at the defaults while shut, and open, the same
      // rows last under the Compositing heading.
      final box =
          compBoxRows(name, instanceParameters(info), values, const {});
      expect(box.telling, isEmpty);
      final listed = [for (final r in box.open) r.id];
      expect(listed.sublist(listed.length - compositingRowIds.length),
          compositingRowIds);
      expect(box.kickers['mix'], 'Compositing');
    });
  }
}
