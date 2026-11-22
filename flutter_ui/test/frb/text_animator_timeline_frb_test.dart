// A Text layer's animators in the Timeline: Add animator on the Animators
// heading, After Effects' Animate, and Delete on an animator, on Animators and
// on a property row.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/panels/text_animator_rows_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/text_documents.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Text animators in the Timeline (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      tester.view.physicalSize = const Size(1280, 700);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(1280, 700),
      ));
      await tester.pump();
    }

    Future<void> twirl(WidgetTester tester, String path) async {
      await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$path')));
      await tester.pumpAndSettle();
    }

    Future<void> delete(WidgetTester tester, dynamic p) async {
      (p.uiState as LumitUiState).activePane.value = Panel.timeline.pane();
      await tester.pump();
      expect((p.uiState as LumitUiState).deleteClaim!(), isTrue);
      await tester.pumpAndSettle();
    }

    testWidgets('Add animator on the Animators heading adds one by property',
        (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      p.uiState.model.refresh();
      await mount(tester, p);
      final id = text.internallayerId.toString();
      await twirl(tester, id);

      // The heading is there before there is an animator, since it is how
      // the first one is made.
      final add = find.byKey(ValueKey<String>('tl-text-animate-$id/animators'));
      expect(add, findsOneWidget);
      await tester.tap(add);
      await tester.pumpAndSettle();
      await tester
          .tap(find.byKey(const ValueKey<String>('text-animate-scale')));
      await tester.pumpAndSettle();

      final animators = text.getText()!.animators;
      expect(animators, hasLength(1));
      expect(animators.single.properties, ['scale']);
      // Only what was added is listed, with the selector.
      final values = textAnimatorValuesOf(animators.single, solid: false);
      expect(values, contains(TextAnimatorValue.scaleX));
      expect(values, isNot(contains(TextAnimatorValue.opacity)));
      expect(
          find.byKey(ValueKey<String>('tl-text-animator-add-$id/animators/0')),
          findsNothing,
          reason: 'the animator heading shows once Animators is open');
      await tester.tap(find.descendant(of: find.byKey(ValueKey<String>('tl-group-$id/animators')), matching: find.byType(Text)).first);
      await tester.pumpAndSettle();
      expect(
          find.byKey(ValueKey<String>('tl-text-animator-add-$id/animators/0')),
          findsOneWidget);

      // The animator open shows its whole selector, as Effect controls does.
      await tester.tap(find.descendant(of: find.byKey(ValueKey<String>('tl-group-$id/animators/0')), matching: find.byType(Text)).first);
      await tester.pumpAndSettle();
      // Based on and Shape fold under the selector's Advanced.
      expect(find.byKey(const ValueKey<String>('tl-anim-0-basis')), findsNothing);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-group-$id/animators/0/advanced')));
      await tester.pumpAndSettle();
      for (final setting in ['selector', 'basis', 'shape']) {
        expect(
            find.byKey(ValueKey<String>('tl-anim-0-$setting')), findsOneWidget,
            reason: setting);
      }
      expect(
          find.byKey(const ValueKey<String>('tl-anim-0-target')), findsNothing,
          reason: 'a Range selector has no target');

      // The same menu has the presets, as Animate text does.
      await tester.tap(add);
      await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 50)));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey<String>('text-animate-preset-typeOn')),
          findsOneWidget);
    });

    testWidgets('An animator heading has its eye, solo, grip and More',
        (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      p.uiState.model.refresh();
      await mount(tester, p);
      final id = text.internallayerId.toString();
      await twirl(tester, id);
      await tester.tap(find.byKey(ValueKey<String>('tl-text-animate-$id/animators')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey<String>('text-animate-scale')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(ValueKey<String>('tl-text-animate-$id/animators')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey<String>('text-animate-opacity')));
      await tester.pumpAndSettle();
      await tester.tap(find.descendant(of: find.byKey(ValueKey<String>('tl-group-$id/animators')), matching: find.byType(Text)).first);
      await tester.pumpAndSettle();
      expect(text.getText()!.animators, hasLength(2));

      expect(find.byKey(const ValueKey<String>('tl-anim-grip-1')), findsOneWidget);
      // Named after what it was added for.
      expect(text.getText()!.animators.map((a) => a.name),
          ['Animator 1 (Scale)', 'Animator 2 (Opacity)']);
      // Add on each animator starts in the column of Add animator.
      final addAnimator = tester.getTopLeft(
          find.byKey(ValueKey<String>('tl-text-animate-$id/animators')));
      final add0 = tester.getTopLeft(find
          .byKey(ValueKey<String>('tl-text-animator-add-$id/animators/0')));
      expect(add0.dx, closeTo(addAnimator.dx, 1));
      await tester.tap(find.byKey(const ValueKey<String>('tl-anim-eye-0')));
      await tester.pumpAndSettle();
      expect(text.getText()!.animators[0].enabled, isFalse);
      await tester.tap(find.byKey(const ValueKey<String>('tl-anim-solo-1')));
      await tester.pumpAndSettle();
      expect(text.getText()!.animators[1].solo, isTrue);

      // Animator 2 open: it stays open as it moves.
      await twirl(tester, '$id/animators/1');
      expect(p.uiState.folds.timelineOpen, contains('$id/animators/1'));

      // More: the same menu as in the Effect controls panel.
      await tester.tap(find.byKey(ValueKey<String>('tl-text-animator-more-$id/animators/1')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey<String>('text-animator-move-up-1')));
      await tester.pumpAndSettle();
      expect(text.getText()!.animators.map((a) => a.properties?.single),
          ['opacity', 'scale']);
      expect(p.uiState.folds.timelineOpen, contains('$id/animators/0'));
      expect(p.uiState.folds.timelineOpen,
          isNot(contains('$id/animators/1')));

      // The grip dropped on the other heading moves it there.
      final from = tester.getCenter(find.byKey(const ValueKey<String>('tl-anim-grip-1')));
      final to = tester.getCenter(find.byKey(const ValueKey<String>('tl-anim-grip-0')));
      final g = await tester.startGesture(from);
      await tester.pump(const Duration(milliseconds: 50));
      await g.moveTo(to);
      await tester.pump();
      await g.up();
      await tester.pumpAndSettle();
      expect(text.getText()!.animators.map((a) => a.properties?.single),
          ['scale', 'opacity']);
      // Remove on the heading.
      await tester.tap(find.byKey(
          ValueKey<String>('tl-text-animator-remove-$id/animators/1')));
      await tester.pumpAndSettle();
      expect(text.getText()!.animators.map((a) => a.properties?.single),
          ['scale']);
      // The next is numbered past the one left, never taking its name.
      await tester.tap(find.byKey(ValueKey<String>('tl-text-animate-$id/animators')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey<String>('text-animate-scale')));
      await tester.pumpAndSettle();
      final names = text.getText()!.animators.map((a) => a.name).toList();
      expect(names.toSet(), hasLength(names.length), reason: '$names');
    });

    testWidgets('Delete takes off an animator, a property, or all of them',
        (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      final id = text.internallayerId.toString();
      final document = text.getText()!;
      BridgeTextAnimator animator(String name) => animatorWith(
            freshTextAnimator(name),
            properties: const ['opacity', 'rotation'],
            opacity: const BridgeScalar.static_(40),
            axes: const [
              BridgeAnimatorAxis(tag: 'wght', value: BridgeScalar.static_(700)),
            ],
          );
      text.setText(
        document: document.copyWith(animators: [
          animator('Animator 1'),
          animator('Animator 2'),
          animator('Animator 3'),
        ]),
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await twirl(tester, id);
      await tester.tap(find.descendant(of: find.byKey(ValueKey<String>('tl-group-$id/animators')), matching: find.byType(Text)).first);
      await tester.pumpAndSettle();
      await tester.tap(find.descendant(of: find.byKey(ValueKey<String>('tl-group-$id/animators/0')), matching: find.byType(Text)).first);
      await tester.pumpAndSettle();

      // A property row: Rotation comes off the list, the axis off the
      // animator, by its own ×.
      await tester
          .tap(find.byKey(const ValueKey<String>('tl-anim-0-rotation-label')));
      await tester.pump();
      await delete(tester, p);
      var first = text.getText()!.animators.first;
      expect(first.properties, ['opacity']);
      expect(text.getText()!.animators, hasLength(3), reason: 'only the row');
      await tester
          .tap(find.byKey(const ValueKey<String>('tl-anim-0-axis0-remove')));
      await tester.pumpAndSettle();
      first = text.getText()!.animators.first;
      expect(first.axes, isEmpty);

      // An animator's heading takes that animator.
      await tester.tap(find.descendant(of: find.byKey(ValueKey<String>('tl-group-$id/animators/1')), matching: find.byType(Text)).first);
      await tester.pump();
      await delete(tester, p);
      expect(text.getText()!.animators.map((a) => a.name),
          ['Animator 1', 'Animator 3']);

      // Animators takes every one.
      await tester.tap(find.descendant(of: find.byKey(ValueKey<String>('tl-group-$id/animators')), matching: find.byType(Text)).first);
      await tester.pump();
      await delete(tester, p);
      expect(text.getText()!.animators, isEmpty);
      expect(p.comp.getLayers(), hasLength(1), reason: 'the layer stays');
    });
  });
}

/// Keeps [LayerReference] and [animatorsPath] imported for the reader: the
/// paths above are the ones the fold builds.
// ignore: unused_element
String _path(LayerReference layer) =>
    animatorsPath(layer.internallayerId.toString());
