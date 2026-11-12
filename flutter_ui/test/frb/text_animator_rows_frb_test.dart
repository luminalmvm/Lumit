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
import 'package:lumit_flutter/state/text_documents.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

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
    /// Add animator ▸ Blank animator: the button opens the menu every Add
    /// animator has.
    Future<void> addBlank(WidgetTester tester) async {
      await tester.tap(find.byKey(const ValueKey('text-animator-add')));
      await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 50)));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('text-animate-blank')));
    }

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
          style: plainTextStyle,
          paragraph: plainParagraphStyle,
          text: 'Lumit',
          size: 72,
          fill: text.getText()!.fill,
          pathOffset: text.getText()!.pathOffset,
          animators: const [],
        ),
      );
      p.uiState.selectedLayer.value = text;
      await mount(tester, p);

      // A kicker now, which Studio draws as written.
      expect(find.text('Animators'), findsOneWidget);
      expect(text.getText()!.animators, isEmpty);

      await addBlank(tester);
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
      await addBlank(tester);
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
        textAnimatorValuesOf(entry.info.textAnimators.single, solid: false)
            .toSet(),
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

    testWidgets(
        'a Proximity selector measures from a target layer and has its own rows',
        (tester) async {
      final p = withComp();
      final target = p.comp.addNullLayer();
      final text = p.comp.addTextLayer();
      p.uiState.selectedLayer.value = text;
      p.uiState.model.refresh();
      await mount(tester, p);
      await addBlank(tester);
      await tester.pump();
      expect(
          find.byKey(const ValueKey('text-animator-kind-0')), findsOneWidget);
      expect(find.byKey(const ValueKey('range-start-0-0')), findsOneWidget);

      // Switched to Proximity, aimed at the null.
      final fresh = text.getText()!.animators.single;
      writeTextAnimatorDocument(
        text,
        animatorWith(fresh,
            selector: selectorWith(fresh.selector,
                kind: BridgeSelectorKind.proximity,
                target: target.internallayerId)),
      );
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.byKey(const ValueKey('range-start-0-0')), findsNothing,
          reason: 'a Proximity selector has no range');
      expect(find.byKey(const ValueKey('range-radius-0-0')), findsOneWidget);
      expect(
          find.byKey(const ValueKey('text-animator-target-0')), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('text-animator-invert-0')));
      await tester.pump();
      final selector = text.getText()!.animators.single.selector;
      expect(selector.invert, isTrue);
      expect(selector.target, target.internallayerId);
      expect(selector.kind, BridgeSelectorKind.proximity);

      // The fold lists the radius instead of the range.
      final entry = p.comp
          .getModel()
          .layers
          .firstWhere((e) => e.layer.internallayerId == text.internallayerId);
      final values =
          textAnimatorValuesOf(entry.info.textAnimators.single, solid: false);
      expect(values, contains(TextAnimatorValue.radius));
      expect(values, isNot(contains(TextAnimatorValue.rangeStart)));
    });

    testWidgets('a font axis on an animator is a keyable row of its own',
        (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      p.uiState.selectedLayer.value = text;
      await mount(tester, p);
      await addBlank(tester);
      await tester.pump();
      final fresh = text.getText()!.animators.single;
      writeTextAnimatorDocument(
        text,
        animatorWith(fresh, axes: const [
          BridgeAnimatorAxis(tag: 'wght', value: BridgeScalar.static_(800)),
        ]),
      );
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.byKey(const ValueKey('anim-axis-wght-0-0')), findsOneWidget);

      // The fold and the graph see it as one more number.
      final animator = text.getText()!.animators.single;
      expect(textAnimatorValuesOf(animator, solid: false),
          contains(TextAnimatorValue.axis0));
      expect(textAnimatorScalarOf(animator, TextAnimatorValue.axis0),
          const BridgeScalar.static_(800));
      expect(textAnimatorValueLabel(TextAnimatorValue.axis0, animator),
          'Font axis Weight');
      writeTextAnimatorScalar(
        layer: text,
        index: 0,
        value: TextAnimatorValue.axis0,
        to: const BridgeScalar.static_(300),
      );
      expect(text.getText()!.animators.single.axes.single.value,
          const BridgeScalar.static_(300));

      await tester.tap(find.byKey(const ValueKey('anim-axis-remove-wght-0')));
      await tester.pump();
      expect(text.getText()!.animators.single.axes, isEmpty);
    });
  });
}

/// Write [animator] as [layer]'s only animator, the rest of its document as
/// it is.
void writeTextAnimatorDocument(
    LayerReference layer, BridgeTextAnimator animator) {
  final document = layer.getText()!;
  layer.setText(document: document.copyWith(animators: [animator]));
}

/// Reads the same way the panel's picker does, so the assertion above does not
/// have to name the generated enum in two places.
extension on BridgeTextAnimator {
  bool get basisIsCharacters =>
      selector.basis == BridgeSelectorBasis.characters;
}
