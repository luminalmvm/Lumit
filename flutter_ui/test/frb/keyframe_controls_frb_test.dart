// The stopwatch and keyframe navigator, against the real engine.
//
// These drive the controls through the Effect controls panel rather than in
// isolation, because what is being asserted is that a click reaches the
// *document* — a keyframe list nothing committed is not a keyframe.
//
// New coverage: v0's equivalents keyed through granular add/remove/shift ops
// that the frb API deliberately does not have (a whole `BridgeScalar` goes
// across instead), so there is nothing here to translate.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/keyframe_controls_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Keyframe controls (frb)', () {
    ({
      LumitState state,
      LumitUiState uiState,
      LayerReference layer,
      CompositionReference comp,
    }) withLayer() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = comp.getLayers().single;
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      return (state: p.state, uiState: p.uiState, layer: layer, comp: comp);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      // These exercise the Transform rows' keyframe controls, and the
      // Transform card is off by default (K-193) — so the tests ask for it,
      // exactly as a user would from Settings → Interface.
      (p.uiState as LumitUiState)
          .workspace
          .interface
          .transformInEffectControls = true;
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
      ));
      await tester.pump();
    }

    /// The opacity row's animation, whatever shape it is in.
    BridgeScalar opacityOf(LayerReference layer) =>
        layer.getTransform().opacity;

    testWidgets(
        'the stopwatch plants one key at the playhead without moving it',
        (tester) async {
      final p = withLayer();
      p.uiState.playheadFrame.value = 24;
      await mount(tester, p);

      final before = opacityOf(p.layer);
      expect(before, isA<BridgeScalar_Static>());
      final value = (before as BridgeScalar_Static).field0;

      await tester.tap(find.byKey(const ValueKey('kf-stopwatch-tf-opacity')));
      await tester.pump();

      final after = opacityOf(p.layer);
      expect(after, isA<BridgeScalar_Keyframed>());
      final keys = (after as BridgeScalar_Keyframed).field0;
      expect(keys, hasLength(1), reason: 'one key, not a curve out of nowhere');
      expect(keys.single.value, value,
          reason: 'turning animation on must not move the picture');
      expect(p.comp.frameAtTime(time: keys.single.time), 24,
          reason: 'the key landed on the playhead');
    });

    testWidgets('the stopwatch off keeps the value the curve reads there',
        (tester) async {
      final p = withLayer();

      // A ramp from 0 at frame 0 to 100 at frame 100.
      p.layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 0),
            value: 0,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 100),
            value: 100,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
        ]),
      );
      p.uiState.playheadFrame.value = 50;
      await mount(tester, p);

      await tester.tap(find.byKey(const ValueKey('kf-stopwatch-tf-opacity')));
      await tester.pump();

      final after = opacityOf(p.layer);
      expect(after, isA<BridgeScalar_Static>());
      expect((after as BridgeScalar_Static).field0, closeTo(50, 0.001),
          reason: 'the value at the playhead, not the first key');
    });

    testWidgets('the diamond adds a key, then removes it', (tester) async {
      final p = withLayer();
      p.uiState.playheadFrame.value = 0;
      await mount(tester, p);

      // Animate, then move on and add a second key.
      await tester.tap(find.byKey(const ValueKey('kf-stopwatch-tf-opacity')));
      await tester.pump();
      p.uiState.playheadFrame.value = 60;
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('kf-toggle-tf-opacity')));
      await tester.pump();
      var keys = (opacityOf(p.layer) as BridgeScalar_Keyframed).field0;
      expect(keys, hasLength(2));
      expect(
        keys.map((k) => p.comp.frameAtTime(time: k.time)).toList(),
        [0, 60],
        reason: 'keys stay strictly ascending in time',
      );

      // The same button removes the key it just added.
      await tester.tap(find.byKey(const ValueKey('kf-toggle-tf-opacity')));
      await tester.pump();
      keys = (opacityOf(p.layer) as BridgeScalar_Keyframed).field0;
      expect(keys, hasLength(1));
      expect(p.comp.frameAtTime(time: keys.single.time), 0);
    });

    /// An animation with no keys is not a curve anything can evaluate, so
    /// removing the last one has to land somewhere sensible rather than leaving
    /// an empty list the engine would refuse.
    testWidgets('removing the last key falls back to a static value',
        (tester) async {
      final p = withLayer();
      p.uiState.playheadFrame.value = 12;
      await mount(tester, p);

      await tester.tap(find.byKey(const ValueKey('kf-stopwatch-tf-opacity')));
      await tester.pump();
      final keyed =
          (opacityOf(p.layer) as BridgeScalar_Keyframed).field0.single;

      await tester.tap(find.byKey(const ValueKey('kf-toggle-tf-opacity')));
      await tester.pump();

      final after = opacityOf(p.layer);
      expect(after, isA<BridgeScalar_Static>());
      expect((after as BridgeScalar_Static).field0, keyed.value,
          reason: 'it holds what the key held');
    });

    testWidgets('the arrows jump the playhead to the neighbouring keys',
        (tester) async {
      final p = withLayer();
      p.layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final frame in [10, 40, 90])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: frame),
              value: frame.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      p.uiState.playheadFrame.value = 40;
      await mount(tester, p);

      await tester.tap(find.byKey(const ValueKey('kf-prev-tf-opacity')));
      await tester.pump();
      expect(p.uiState.playheadFrame.value, 10);

      await tester.tap(find.byKey(const ValueKey('kf-next-tf-opacity')));
      await tester.pump();
      expect(p.uiState.playheadFrame.value, 40);

      await tester.tap(find.byKey(const ValueKey('kf-next-tf-opacity')));
      await tester.pump();
      expect(p.uiState.playheadFrame.value, 90);

      // Past the last key there is nowhere to go, and the arrow is inert.
      await tester.tap(find.byKey(const ValueKey('kf-next-tf-opacity')));
      await tester.pump();
      expect(p.uiState.playheadFrame.value, 90,
          reason: 'a disabled arrow does nothing rather than wrapping around');
    });

    /// The whole point of taking a whole animation across the seam: v0 needed
    /// two ops for a key that moved in time *and* value, so a single drag left
    /// two entries in the undo history.
    testWidgets('each keyframe action is exactly one undo step',
        (tester) async {
      final p = withLayer();
      p.uiState.playheadFrame.value = 24;
      await mount(tester, p);

      await tester.tap(find.byKey(const ValueKey('kf-stopwatch-tf-opacity')));
      await tester.pump();
      expect(opacityOf(p.layer), isA<BridgeScalar_Keyframed>());

      p.state.project!.undo();
      expect(opacityOf(p.layer), isA<BridgeScalar_Static>(),
          reason: 'one undo puts the whole thing back');
    });

    /// Only the number-shaped kinds animate. A dropdown or a file path has
    /// nothing to interpolate, so those rows carry no stopwatch at all rather
    /// than one that cannot do anything.
    testWidgets('a non-animatable parameter has no stopwatch', (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      await mount(tester, p);

      final id = p.layer.getEffects().single.id();
      expect(find.byKey(ValueKey<String>('kf-stopwatch-$id-radius')),
          findsOneWidget,
          reason: 'a float parameter animates');

      final choice = listParameters(effect: 'blur')
          .where((p) => p.kind is BridgeParamKind_Choice);
      for (final param in choice) {
        expect(find.byKey(ValueKey<String>('kf-stopwatch-$id-${param.id}')),
            findsNothing,
            reason: '${param.id} is a dropdown, so it cannot animate');
      }
    });

    /// One stopwatch on a multi-axis row keys every axis it covers, and does
    /// it in one undo step — two ops for one click is what the whole-value
    /// shape exists to avoid.
    testWidgets('a multi-axis row keys all its axes as one step',
        (tester) async {
      final p = withLayer();
      p.uiState.playheadFrame.value = 12;
      await mount(tester, p);

      final before = p.layer.getTransform();
      expect(before.positionX, isA<BridgeScalar_Static>());
      expect(before.positionY, isA<BridgeScalar_Static>());

      // Position's stopwatch — one control, two properties.
      await tester.tap(find.byKey(const ValueKey('kf-stopwatch-tf-positionX')));
      await tester.pump();

      final after = p.layer.getTransform();
      expect(after.positionX, isA<BridgeScalar_Keyframed>(),
          reason: 'x was keyed');
      expect(after.positionY, isA<BridgeScalar_Keyframed>(),
          reason: 'and so was y — the row is one control');
      expect(
        p.comp.frameAtTime(
            time:
                (after.positionY as BridgeScalar_Keyframed).field0.single.time),
        12,
        reason: 'both landed on the playhead',
      );

      p.state.project!.undo();
      final undone = p.layer.getTransform();
      expect(undone.positionX, isA<BridgeScalar_Static>());
      expect(undone.positionY, isA<BridgeScalar_Static>(),
          reason: 'one undo put both back — a batch, not two ops');
    });

    testWidgets('the diamond adds and removes on every axis together',
        (tester) async {
      final p = withLayer();
      p.uiState.playheadFrame.value = 0;
      await mount(tester, p);

      await tester.tap(find.byKey(const ValueKey('kf-stopwatch-tf-positionX')));
      await tester.pump();
      p.uiState.playheadFrame.value = 40;
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('kf-toggle-tf-positionX')));
      await tester.pump();
      var tf = p.layer.getTransform();
      expect((tf.positionX as BridgeScalar_Keyframed).field0, hasLength(2));
      expect((tf.positionY as BridgeScalar_Keyframed).field0, hasLength(2),
          reason: 'the axes keep the same key times');

      await tester.tap(find.byKey(const ValueKey('kf-toggle-tf-positionX')));
      await tester.pump();
      tf = p.layer.getTransform();
      expect((tf.positionX as BridgeScalar_Keyframed).field0, hasLength(1));
      expect((tf.positionY as BridgeScalar_Keyframed).field0, hasLength(1));
    });

    /// **The fold-out's hit target** (docs/15 §5). The two layouts share one
    /// button builder, and when the Effect controls panel's fixed columns
    /// arrived (K-443) the horizontal padding was dropped to nothing for
    /// *both* — which is right for the columns, whose 18px the button's own
    /// reserved edge already fills, and wrong for the Timeline's fold-out,
    /// whose buttons quietly shrank by 6px and became harder to hit.
    testWidgets('the Timeline fold-out keeps its padded buttons',
        (tester) async {
      final p = withLayer();
      Widget controls({required bool fixedColumns}) => KeyframeControlsFrb(
            scalars: [opacityOf(p.layer)],
            comp: p.comp,
            playheadFrame: 0,
            onSeek: (_) {},
            onWrite: (_) {},
            rowKey: fixedColumns ? 'fixed' : 'loose',
            fixedColumns: fixedColumns,
          );

      await tester.pumpWidget(hostPanel(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            controls(fixedColumns: false),
            controls(fixedColumns: true),
          ],
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      final loose =
          tester.getSize(find.byKey(const ValueKey('kf-stopwatch-loose')));
      final fixed =
          tester.getSize(find.byKey(const ValueKey('kf-stopwatch-fixed')));
      expect(loose.width, fixed.width + 6,
          reason: '3px either side, as the fold-out always had');
      expect(fixed.width, 18,
          reason: 'the fixed columns are measured in unpadded buttons (K-443)');
    });

    // -------------------------------------------------------------------
    // **A colour keyframes like anything else** (K-535, owner desk test: "for
    // effects that have a color value property, I can't animate them, the
    // stopwatch is just gone").
    //
    // The engine could always do it — a colour is four independent properties
    // and `colour_at` samples them one by one. What was missing was in the
    // row: the helper that decides what a stopwatch would cover answered with
    // a single scalar, so a colour row asked for no controls and drew none.
    // -------------------------------------------------------------------

    /// The four channels of a `colour_control` effect's colour parameter.
    BridgeColour colourOf(LayerReference layer) {
      final info = layer.getEffects().single.getInfo();
      for (final p in info.values) {
        if (p.value case BridgeEffectValue_Colour(:final field0)) return field0;
      }
      throw StateError('no colour parameter on the effect');
    }

    List<BridgeKeyframe> keysOf(BridgeScalar s) =>
        s is BridgeScalar_Keyframed ? s.field0 : const [];

    testWidgets('a colour row carries the stopwatch and its navigator',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'colour_control');
      p.uiState.model.refresh();
      await mount(tester, p);
      final id = p.layer.getEffects().single.id();

      final stopwatch = find.byKey(ValueKey<String>('kf-stopwatch-$id-colour'));
      expect(stopwatch, findsOneWidget,
          reason: 'the row that could not be animated at all');
      expect(find.byKey(ValueKey<String>('kf-prev-$id-colour')), findsNothing,
          reason: 'the navigator waits until there is a curve to walk');

      await tester.tap(stopwatch);
      await tester.pump();

      expect(
          find.byKey(ValueKey<String>('kf-prev-$id-colour')), findsOneWidget);
      expect(
          find.byKey(ValueKey<String>('kf-next-$id-colour')), findsOneWidget);
      // And the swatch is still a swatch: it used to say the word `animated`
      // and stand down, so a keyed colour could not be changed.
      expect(
          find.byKey(ValueKey<String>('fx-colour-$id-colour')), findsOneWidget);
      expect(find.text('animated'), findsNothing);
    });

    /// The whole flow the owner asked for: key it, move on, change it, and
    /// find two keys with the picture between them interpolating.
    testWidgets('keyframing a colour: one key, then a second, then a ramp',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'colour_control');
      p.uiState
        ..model.refresh()
        ..playheadFrame.value = 0;
      await mount(tester, p);
      final id = p.layer.getEffects().single.id();

      final was = colourOf(p.layer);
      expect(keysOf(was.r), isEmpty, reason: 'still to begin with');
      final wasRed = (was.r as BridgeScalar_Static).field0;

      await tester.tap(find.byKey(ValueKey<String>('kf-stopwatch-$id-colour')));
      await tester.pump();

      final keyed = colourOf(p.layer);
      for (final channel in [keyed.r, keyed.g, keyed.b, keyed.a]) {
        expect(keysOf(channel), hasLength(1),
            reason: 'every channel keys together under one stopwatch');
        expect(p.comp.frameAtTime(time: keysOf(channel).single.time), 0);
      }
      expect(keysOf(keyed.r).single.value, closeTo(wasRed, 1e-6),
          reason: 'turning animation on must not move the picture');

      // Move on, and change the colour through the swatch's own picker.
      p.uiState.playheadFrame.value = 48;
      await tester.pump();
      await tester.tap(find.byKey(ValueKey<String>('fx-colour-$id-colour')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('colour-picker-R')));
      await tester.pumpAndSettle();
      // Down to nothing, which the default red is not.
      await tester.enterText(find.byType(EditableText).first, '0');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      final ramped = colourOf(p.layer);
      final red = keysOf(ramped.r);
      expect(red, hasLength(2),
          reason: 'the edit at frame 48 planted a key rather than flattening '
              'the one at frame 0');
      expect(
          red.map((k) => p.comp.frameAtTime(time: k.time)).toList(), [0, 48]);
      expect(red.first.value, closeTo(wasRed, 1e-6));
      expect(red.last.value, closeTo(0, 1e-6));

      // And between them the channel really ramps — a curve, not a step.
      final middle =
          sampleScalar(scalar: ramped.r, time: p.comp.timeOfFrame(frame: 24));
      expect(middle, lessThan(red.first.value));
      expect(middle, greaterThan(red.last.value));
    });

    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
  }, skip: !engineAvailable);
}
