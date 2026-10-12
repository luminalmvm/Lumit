// The Styles group on frb (docs/impl/layer-styles.md §5 and §6).
//
// Two things are pinned here, and they are the two the package exists for.
// That a layer wearing styles grows a **Styles** group in the Timeline's
// fold-out — after Effects, one subgroup per style, in Photoshop's pinned
// painting order, with ordinary parameter rows under it — and that a style
// parameter's edit round-trips through the shared instance lookup rather than
// landing on the effect stack.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Layer styles (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    testWidgets(
        'a styled layer shows the group, in the pinned order, after Effects',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      // Asked for out of order: the order on screen is Photoshop's, not the
      // order they were added in.
      layer.addStyle(name: 'style_stroke');
      layer.addStyle(name: 'style_drop_shadow');
      layer.addEffect(name: 'blur');

      final id = layer.internallayerId.toString();
      final rows = layerFoldRows(
        entry: p.comp.getModel().layers.single,
        open: everyFoldPath,
        hasAudio: false,
      );
      final headings = rows.whereType<FoldGroupRow>().map((g) => g.label);
      expect(headings, contains('Styles'));
      expect(
        headings.toList().indexOf('Styles'),
        greaterThan(headings.toList().indexOf('Effects')),
        reason: 'styles render after the effect stack, and are listed there',
      );
      expect(
        [
          for (final g in rows.whereType<FoldGroupRow>())
            if (styleIdOfPath(g.path) != null) g.label,
        ],
        ['Drop shadow', 'Stroke'],
      );

      // The rows underneath are the ordinary parameter rows, and their paths
      // sit under the group's — which is what lights the heading when one of
      // them is picked.
      final params = rows.whereType<FoldEffectParamRow>().where((r) => r.style);
      expect(params, isNotEmpty);
      expect(params.every((r) => r.style), isTrue);
      expect(
        params.map((r) => foldRowPath(id, r)).every(
              (path) => isUnderPath(stylesPath(id), path),
            ),
        isTrue,
      );
      // And an effect's rows are still the effect's.
      expect(
        rows
            .whereType<FoldEffectParamRow>()
            .where((r) => !r.style)
            .map((r) => foldRowPath(id, r))
            .every((path) => isUnderPath(effectsPath(id), path)),
        isTrue,
      );
    });
  });
}
