// The Animators section on frb: a text layer's letters moved one at a time.
//
// Driven through the Effect controls panel, like the Source rows beside it,
// because "which rows appear" is half of what the section does: a layer that is
// not a text layer has no letters to animate and shows nothing at all.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/graph_editor_frb.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/panels/text_animator_rows_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Text animators (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    // Deliberately without turning the layer cards on: the Animators section
    // has to be reachable with the panel as it ships, unlike Source
    // and Transform which move to the Timeline's fold.
    Future<void> mount(WidgetTester tester, dynamic p) async {
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(560, 900),
      ));
      await tester.pump();
    }

    testWidgets('the animators section adds, edits and removes an animator',
        (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      text.setText(
        document: BridgeTextDocument(
          text: 'Lumit',
          size: 72,
          fill: text.getText()!.fill,
          pathOffset: text.getText()!.pathOffset,
          animators: const [],
        ),
      );
      p.uiState.selectedLayer.value = text;
      await mount(tester, p);

      // A kicker now: capitals on the way to the screen.
      expect(find.text('ANIMATORS'), findsOneWidget);
      expect(text.getText()!.animators, isEmpty);

      await tester.tap(find.byKey(const ValueKey('text-animator-add')));
      await tester.pump();
      expect(text.getText()!.animators.length, 1);
      // A fresh animator changes nothing until a number is moved, and its
      // range covers the whole of the words.
      final fresh = text.getText()!.animators.first;
      expect(fresh.selector.start, const BridgeScalar.static_(0));
      expect(fresh.selector.end, const BridgeScalar.static_(100));
      expect(fresh.opacity, const BridgeScalar.static_(100));
      expect(fresh.basisIsCharacters, isTrue);

      // Its rows are there and they write: the range's end, and the choice of
      // what the range counts.
      await tester.tap(find.byKey(const ValueKey('range-end-0-0')));
      await tester.pump();
      await tester.enterText(find.byType(EditableText).last, '25');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(text.getText()!.animators.first.selector.end,
          const BridgeScalar.static_(25));

      // The words the layer says are untouched by an animator edit — the
      // document is written whole, so anything dropped would be deleted.
      expect(text.getText()!.text, 'Lumit');
      expect(text.getText()!.size, 72);

      await tester.tap(find.byKey(const ValueKey('text-animator-remove-0')));
      await tester.pump();
      expect(text.getText()!.animators, isEmpty);
      expect(text.getText()!.text, 'Lumit');
    });

    testWidgets(
        'an animator gets fold rows and the graph editor gets its curves',
        (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      p.uiState.selectedLayer.value = text;
      await mount(tester, p);
      await tester.tap(find.byKey(const ValueKey('text-animator-add')));
      await tester.pump();

      final entry = p.comp.getModel().layers.single;
      final id = entry.layer.internallayerId.toString();
      expect(entry.info.textAnimators.length, 1,
          reason: 'the read model carries the animators');

      // The fold: a heading, the animator's own twirl, and a row per number.
      final rows = layerFoldRows(
        entry: entry,
        open: everyFoldPath,
        hasAudio: false,
      );
      expect(
        rows.whereType<FoldGroupRow>().map((r) => r.path),
        contains(animatorsPath(id)),
      );
      expect(
        rows.whereType<FoldAnimatorValueRow>().map((r) => r.value).toSet(),
        TextAnimatorValue.values.toSet(),
        reason: 'every number an animator has needs a row to key it on',
      );

      // The graph editor resolves one of those rows to a curve, with no bridge
      // call and no second model.
      final path = '${animatorPath(id, 0)}/${TextAnimatorValue.rangeOffset.name}';
      expect(foldRowPath(id, rows.whereType<FoldAnimatorValueRow>().firstWhere(
          (r) => r.value == TextAnimatorValue.rangeOffset)), path);
      final channels = graphChannels(layers: [entry], selected: [path]);
      expect(channels.length, 1);
      expect(channels.single.scalar, const BridgeScalar.static_(0));
      expect(channels.single.label, contains('Range offset'));

      // And a graph edit on it lands on the document.
      commitChannelEdits({channels.single: const BridgeScalar.static_(40)});
      expect(text.getText()!.animators.first.selector.offset,
          const BridgeScalar.static_(40));
    });

    testWidgets('a layer with no letters has no animators section',
        (tester) async {
      final p = withComp();
      final solid = p.comp.addSolidLayer();
      p.uiState.selectedLayer.value = solid;
      await mount(tester, p);
      expect(find.text('ANIMATORS'), findsNothing);
      expect(find.byKey(const ValueKey('text-animator-add')), findsNothing);
    });
  });
}

/// Reads the same way the panel's picker does, so the assertion above does not
/// have to name the generated enum in two places.
extension on BridgeTextAnimator {
  bool get basisIsCharacters =>
      selector.basis == BridgeSelectorBasis.characters;
}
