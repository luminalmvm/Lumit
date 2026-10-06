// The graph editor against the real engine: the AE-style full-height pane
// (docs/07 §5) — selected properties as curves, key drags, easing, the F9
// family, the speed lens, and keyframe copy/paste.

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_editor_frb.dart';
import 'package:lumit_flutter/panels/graph_maths.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Graph editor (frb)', () {
    ({
      LumitState state,
      LumitUiState uiState,
      CompositionReference comp,
      LayerReference layer,
    }) withLayer() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addAdjustmentLayer();
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      return (state: p.state, uiState: p.uiState, comp: comp, layer: layer);
    }

    /// A ramp on Opacity: `frames[i]` holds the value `frames[i]`.
    void animateOpacity(
      CompositionReference comp,
      LayerReference layer, {
      List<int> frames = const [0, 100],
    }) {
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in frames)
            BridgeKeyframe(
              time: comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
    }

    List<BridgeKeyframe> opacityKeys(LayerReference layer) =>
        (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;

    /// The opacity channel's key ids, as the pane names them.
    String opacityKey(LayerReference layer, int index) =>
        'graph-key-${layer.internallayerId}/transform/opacity@opacity#$index';

    /// Click a property's name in Graph mode's own outline — the filtered
    /// animated list (§3.3), whose rows are already flat, so the name is the
    /// whole gesture. A **still** property is not on the Animated list, so
    /// Show is flipped to All to reach it.
    Future<void> pickProperty(WidgetTester tester, String label) async {
      await tester.tap(find.text(label));
      await tester.pump();
    }

    /// Mount the panel in Graph mode with a property picked.
    ///
    /// **The outline is the Layers outline**: the graph's own
    /// colour-ticked filtered list is gone, so a property is reached by
    /// twirling the layer open exactly as it is in Layers mode, and the
    /// `layersOutline` flag those tests used to pass has nothing left to
    /// switch.
    Future<void> mountGraph(WidgetTester tester, dynamic p,
        {bool selectOpacity = true}) async {
      // The outline alone is ~740 px of columns; the default 800×600 test
      // surface would push the graph pane off screen.
      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(1280, 600),
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('tl-graph')));
      await tester.pump();
      if (!selectOpacity) return;
      final layer = (p as dynamic).layer as LayerReference;
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();
      await pickProperty(tester, 'Opacity');
    }

    /// The same chain with the playhead **between** keys: the drag
    /// starts by planting a key at the playhead — holding the value already
    /// there, so nothing moves — and the graph then carries that key live.
    /// This is the everyday shape of the reported bug: nobody drags a value
    /// while parked exactly on an existing key.
    testWidgets('a drag between keys plants one and the graph carries it',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer);
      // Frame 31, not a rounder number: 31/60 s as a double times 60 is not
      // 31.0, which is exactly the float mismatch that made the old preview
      // insert a duplicate key instead of replacing. Frame 50 is
      // float-exact and cannot catch it.
      p.uiState.scrubTo(31);
      await mountGraph(tester, p);

      expect(opacityKeys(p.layer).length, 2);

      final field = find.byKey(const ValueKey<String>('tl-tf-opacity'));
      final gesture = await tester.startGesture(tester.getCenter(field));
      await tester.pump();
      // The first move is spent crossing the recogniser's slop; the second is
      // the first that ticks.
      await gesture.moveBy(const Offset(30, 0));
      await tester.pump();
      await gesture.moveBy(const Offset(30, 0));
      await tester.pump();

      expect(opacityKeys(p.layer).length, 3,
          reason: 'the drag planted a key at the playhead as it began');
      expect(rowValueDrag.value, isNotNull);
      // Exactly three glyphs. The preview once matched keys by *float* frame
      // equality, and frame 31 at 60 fps does not read back as 31.0, so the
      // drag's key was inserted BESIDE the planted one instead of replacing
      // it — one extra key, every later diamond one index off, the dragged
      // key drawn at the next key's place.
      expect(
          find.byWidgetPredicate((w) =>
              w.key is ValueKey<String> &&
              ((w.key as ValueKey<String>).value)
                  .startsWith('graph-key-${p.layer.internallayerId}/')),
          findsNWidgets(3),
          reason: 'replaced in place, never duplicated');
      final planted = find.byKey(ValueKey<String>(opacityKey(p.layer, 1)));
      expect(planted, findsOneWidget,
          reason: 'and the graph shows the planted key mid-gesture');
      final during = tester.getCenter(planted);
      final lastBefore = tester
          .getCenter(find.byKey(ValueKey<String>(opacityKey(p.layer, 2))));

      await gesture.moveBy(const Offset(30, 0));
      await tester.pump();
      expect(tester.getCenter(planted).dy, lessThan(during.dy),
          reason: 'the planted key follows the pointer');
      expect(
          tester
              .getCenter(find.byKey(ValueKey<String>(opacityKey(p.layer, 2)))),
          lastBefore,
          reason: 'the keys after the playhead do not move with the drag');

      await gesture.up();
      await tester.pump();
      expect(rowValueDrag.value, isNull);
    });

    testWidgets('dragging a key moves it in time and value as one undo step',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer);
      await mountGraph(tester, p);

      final before = opacityKeys(p.layer);
      final beforeFrame = p.comp.frameAtTime(time: before[1].time);

      await _drag(tester, find.byKey(ValueKey<String>(opacityKey(p.layer, 1))),
          const Offset(60, 40));

      final after = opacityKeys(p.layer);
      expect(after, hasLength(2));
      expect(p.comp.frameAtTime(time: after[1].time), greaterThan(beforeFrame),
          reason: 'it moved later');
      expect(after[1].value, lessThan(before[1].value),
          reason: 'and dragging down lowered the value in the same gesture');

      p.state.project!.undo();
      final undone = opacityKeys(p.layer);
      expect(p.comp.frameAtTime(time: undone[1].time), beforeFrame,
          reason: 'one undo puts back both the time and the value');
      expect(undone[1].value, before[1].value);
    });

    /// Two keys cannot share a frame: the channel refuses the landing and
    /// keeps what it had.
    testWidgets('a key dragged onto its neighbour does not land there',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 10]);
      await mountGraph(tester, p);

      await _drag(tester, find.byKey(ValueKey<String>(opacityKey(p.layer, 1))),
          const Offset(-900, 0));

      final keys = opacityKeys(p.layer);
      expect(keys, hasLength(2), reason: 'neither key was lost');
      final frames = keys.map((k) => p.comp.frameAtTime(time: k.time)).toList();
      expect(frames[0], isNot(frames[1]), reason: 'they still differ in time');
    });

    /// The bottom bar's easing buttons act on the selected keys — and F9 does
    /// the same from the keyboard (docs/07 §5.3).
    testWidgets('the bottom bar buttons and F9 set the selected keys\' easing',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer);
      await mountGraph(tester, p);

      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 0))));
      await tester.pump();

      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-interp-hold')));
      await tester.tap(find.byKey(const ValueKey('graph-interp-hold')));
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer)[0].interpOut, isA<BridgeSideInterp_Hold>());

      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-interp-bezier')));
      await tester.tap(find.byKey(const ValueKey('graph-interp-bezier')));
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer)[0].interpOut, isA<BridgeSideInterp_Bezier>());

      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-interp-linear')));
      await tester.tap(find.byKey(const ValueKey('graph-interp-linear')));
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer)[0].interpOut, isA<BridgeSideInterp_Linear>());

      await tester.sendKeyEvent(LogicalKeyboardKey.f9);
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer)[0].interpIn, isA<BridgeSideInterp_Bezier>(),
          reason: 'F9 easy-eases the selection');
    });

    /// The bottom bar's Tangents run — Auto / Clamp / Free (§6.3). The mode is
    /// stored per key side, and the round trip out to Auto and back hands the
    /// custom ease over untouched, which is the study's explicit bar.
    testWidgets('the Tangents buttons set the mode and keep the custom ease',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer);
      await mountGraph(tester, p);

      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 0))));
      await tester.pump();

      // Shape the side by hand first: an ease the automatic modes must not eat.
      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-interp-bezier')));
      await tester.tap(find.byKey(const ValueKey('graph-interp-bezier')));
      await tester.pumpAndSettle();
      final custom = opacityKeys(p.layer)[0].interpOut;
      expect(custom, isA<BridgeSideInterp_Bezier>());

      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-tangent-auto')));
      await tester.tap(find.byKey(const ValueKey('graph-tangent-auto')));
      await tester.pumpAndSettle();
      final automatic = opacityKeys(p.layer)[0].interpOut;
      expect(automatic, isA<BridgeSideInterp_Auto>());
      expect((automatic as BridgeSideInterp_Auto).field0.clamped, isFalse);

      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-tangent-clamp')));
      await tester.tap(find.byKey(const ValueKey('graph-tangent-clamp')));
      await tester.pumpAndSettle();
      final clamped = opacityKeys(p.layer)[0].interpOut;
      expect(clamped, isA<BridgeSideInterp_Auto>());
      expect((clamped as BridgeSideInterp_Auto).field0.clamped, isTrue);

      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-tangent-free')));
      await tester.tap(find.byKey(const ValueKey('graph-tangent-free')));
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer)[0].interpOut, custom,
          reason: 'Free → Auto → Free gives the shaped ease back');
      expect(opacityKeys(p.layer)[0].interpIn, isA<BridgeSideInterp_Bezier>(),
          reason: 'the mode is per side, and the strip sets both');
    });

    /// The shaped ease: a curve drawn once in the unit box, stamped on
    /// every **span** whose two ends are selected — and only from the value
    /// lens, because the shape is drawn against value travel.
    ///
    /// Driven through the button's *popup* mode, because this test mounts the
    /// Timeline alone: in panel mode the button docks a pane that only the full
    /// shell renders, and what is under test here is the stamping, not where
    /// the editor is shown. `easing_panel_frb_test.dart` covers the panel, and
    /// the two tests below cover which of them the button reaches for.
    testWidgets('the Easing button stamps one shape across the spans',
        (tester) async {
      final p = withLayer();
      p.uiState.workspace.interface.easingInPopup = true;
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);

      // The editor is a popup: a click outside it takes it back, so the
      // selection is made first and the box opened over it.
      Future<void> openEditor() async {
        await tester
            .ensureVisible(find.byKey(const ValueKey('graph-interp-easing')));
        await tester.tap(find.byKey(const ValueKey('graph-interp-easing')));
        await tester.pumpAndSettle();
      }

      // A lone key names no travel: applying leaves the document as it was.
      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 0))));
      await tester.pump();
      await openEditor();
      // The popup keeps the smallest layout: the overlay would offer it the
      // whole window to grow into, and the panel is where width is spent.
      expect(
          tester.getSize(find.byKey(const ValueKey('easing-box'))).width, 210);
      // A tile now applies as it loads, and the Apply press repeats
      // it — both are no-ops on a lone key.
      await tester.ensureVisible(find.text('Slow start'));
      await tester.tap(find.text('Slow start'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Apply'.toUpperCase()));
      await tester.tap(find.text('Apply'.toUpperCase()));
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer)[0].interpOut, isA<BridgeSideInterp_Linear>(),
          reason: 'one key on its own has no span to shape');
      await tester.ensureVisible(find.text('Close'));
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();

      // Both ends of the first span selected: that span takes the shape, and
      // the span beyond the selection does not.
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 1))));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();
      await openEditor();
      await tester.ensureVisible(find.text('Slow start'));
      await tester.tap(find.text('Slow start'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Apply'.toUpperCase()));
      await tester.tap(find.text('Apply'.toUpperCase()));
      await tester.pumpAndSettle();

      final keys = opacityKeys(p.layer);
      final out0 = keys[0].interpOut;
      expect(out0, isA<BridgeSideInterp_Bezier>());
      // Slow start: flat out of the first key, and the reach is the handle's
      // own x — a third of the span (docs/impl/keyframe-eval.md §1).
      expect((out0 as BridgeSideInterp_Bezier).field0.speed, closeTo(0, 1e-9));
      expect(out0.field0.influence, closeTo(1 / 3, 1e-9));
      expect(keys[1].interpOut, isA<BridgeSideInterp_Linear>(),
          reason: 'the span past the selection was left alone');

      // The speed lens takes the button away, so a shape cannot be stamped on a
      // graph the user is not looking at.
      await tester.ensureVisible(find.text('Close'));
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-lens-speed')));
      await tester.tap(find.byKey(const ValueKey('graph-lens-speed')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('graph-interp-easing')), findsNothing);
    });

    /// A joined pair moves *together and live*: the partner must follow while
    /// the pointer is down, not jump into place on release.
    testWidgets('dragging one handle swings its partner live', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);

      // Easy-ease the middle key so both sides are joined beziers, then select
      // it to bring its handles out.
      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 1))));
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.f9);
      await tester.pumpAndSettle();

      final base =
          'graph-handle-${p.layer.internallayerId}/transform/opacity@opacity#1';
      final outHandle = find.byKey(ValueKey<String>('$base-out'));
      final inHandle = find.byKey(ValueKey<String>('$base-in'));
      expect(outHandle, findsOneWidget);
      expect(inHandle, findsOneWidget);

      final keyPoint = tester
          .getCenter(find.byKey(ValueKey<String>(opacityKey(p.layer, 1))));
      final inBefore = tester.getCenter(inHandle);
      final lengthBefore = (inBefore - keyPoint).distance;

      // Drag the out handle upward, and look *mid-gesture*.
      final gesture = await tester.startGesture(tester.getCenter(outHandle));
      await tester.pump();
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(-2, -8));
        await tester.pump();
      }
      final outMid = tester.getCenter(outHandle);
      final inMid = tester.getCenter(inHandle);
      expect(inMid.dy, greaterThan(inBefore.dy + 2),
          reason: 'the partner swung the opposite way during the drag');

      // Opposite through the key, at the length it started with: a handle
      // keeps its *visual* length however far the pair swings.
      final outDir = outMid - keyPoint;
      final inDir = inMid - keyPoint;
      final cross = outDir.dx * inDir.dy - outDir.dy * inDir.dx;
      expect(cross.abs() / (outDir.distance * inDir.distance), lessThan(0.08),
          reason: 'the two handles stayed in one straight line');
      expect(inDir.distance, closeTo(lengthBefore, 1),
          reason: 'the partner kept its on-screen length');

      await gesture.up();
      await tester.pumpAndSettle();
      final key = opacityKeys(p.layer)[1];
      expect(key.interpIn, isA<BridgeSideInterp_Bezier>());
      expect(key.interpOut, isA<BridgeSideInterp_Bezier>());
    });

    /// The speed lens: each key is an in dot and an out dot that move
    /// independently (docs/07 §5.1).
    testWidgets('the speed lens shows independent in and out dots',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);

      // The button strip scrolls sideways in a narrow panel; bring the lens
      // switch into view first.
      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-lens-speed')));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('graph-lens-speed')));
      await tester.pump();

      final base =
          'graph-key-${p.layer.internallayerId}/transform/opacity@opacity#1';
      expect(find.byKey(ValueKey<String>('$base-in')), findsOneWidget);
      expect(find.byKey(ValueKey<String>('$base-out')), findsOneWidget);

      // Dragging the out dot down converts that side to a bezier with a
      // negative-or-lower speed, leaving the in side alone.
      await _drag(tester, find.byKey(ValueKey<String>('$base-out')),
          const Offset(0, 60));
      final key = opacityKeys(p.layer)[1];
      expect(key.interpOut, isA<BridgeSideInterp_Bezier>(),
          reason: 'the dragged side became a shaped ease');
      expect(key.interpIn, isA<BridgeSideInterp_Linear>(),
          reason: 'the other side did not move');
    });

    /// Ctrl+C / Ctrl+V: the in-app clipboard carries full easing, and pasting
    /// lands the earliest key on the playhead.
    testWidgets('copy and paste land keys on the playhead', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 20]);
      await mountGraph(tester, p);

      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 1))));
      await tester.pump();
      // The chord itself is the shell's — it asks the claim this panel
      // registers, which is what a shell test drives end to end
      // (`Ctrl+C with keyframes selected copies those`). Here the claim is
      // called directly, because this test mounts the panel and not the shell.
      expect(p.uiState.copyClaim!(), isTrue);
      await tester.pump();

      p.uiState.playheadFrame.value = 75;
      await tester.pump();
      expect(p.uiState.pasteClaim!(), isTrue);
      await tester.pumpAndSettle();

      final frames = opacityKeys(p.layer)
          .map((k) => p.comp.frameAtTime(time: k.time))
          .toList();
      expect(frames, contains(75),
          reason: 'the earliest pasted key lands on the playhead');
      expect(frames, hasLength(3));
    });

    /// Delete is claimed the same way, and for the same reason: it was
    /// answered on the hardware keyboard here, which claims nothing — every
    /// handler runs on every key, so deleting a graph key also let the shell
    /// delete the layer the key belonged to.
    testWidgets('a picked graph key claims Delete', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 20, 40]);
      await mountGraph(tester, p);
      p.uiState.activePane.value = Panel.timeline.pane();

      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 1))));
      await tester.pump();

      expect(p.uiState.deleteClaim!(), isTrue,
          reason: 'the picked key is what Delete is about, and a claim is what '
              'makes the shell stand down from the layer');
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer), hasLength(2), reason: 'the key went');
    });

    /// **A closed range graphs like the float it is**. The Slider kind
    /// says which control to draw, not how the number is stored — docs/08 §1.2
    /// names the graph editor among the affordances it keeps. When the four
    /// wipes' Completion adopted the kind, this channel resolver was still
    /// asking for `Float` by name and dropped it, so a keyframed Completion
    /// could no longer be opened as a curve at all.
    testWidgets('graphChannels resolves a closed-range effect parameter',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'linear_wipe');
      final staged = p.layer.getEffects();
      final fxId = staged.single.id();
      for (final instance in staged) {
        instance.setValue(
          id: 'completion',
          value: BridgeEffectValue.float(BridgeScalar.keyframed([
            for (final (f, v) in [(0, 0.0), (100, 100.0)])
              BridgeKeyframe(
                time: p.comp.timeOfFrame(frame: f),
                value: v,
                interpIn: const BridgeSideInterp.linear(),
                interpOut: const BridgeSideInterp.linear(),
              ),
          ])),
        );
      }
      p.layer.setEffects(effects: staged);
      final id = p.layer.internallayerId.toString();
      p.uiState.model.refresh();

      final channels = graphChannels(
        layers: p.uiState.model.layers,
        selected: ['$id/effects/$fxId/completion'],
      );
      expect(channels, hasLength(1),
          reason: 'a Slider kind is a float and belongs in the graph');
      expect(channels.single.keys, hasLength(2));

      // **M28 leftover.** The parameter's hard range reaches the graph, so the
      // line drawn while a key is dragged can be held inside the range the
      // engine will put the value in. Completion is closed 0..100.
      expect(channels.single.hardBounds, (0.0, 100.0));
      expect(channels.single.clampToBounds(140), 100);
      expect(channels.single.clampToBounds(-20), 0);
      expect(channels.single.clampToBounds(40), 40);

      // **And the line drawn between two in-range keys is held there too** —
      // the rest of M28. Both keys below sit exactly on the bound and both
      // lean away from it, so the cubic between them bulges past 100: a
      // completion the parameter cannot hold, drawn as if it could.
      const lean =
          BridgeSideInterp.bezier(BridgeBezierSide(speed: 240, influence: 60));
      final bulging = [
        for (final (f, v) in [(0, 100.0), (100, 100.0)])
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: f),
            value: v,
            interpIn: lean,
            interpOut: lean,
          ),
      ];
      final at = [
        for (var f = 1; f < 100; f++)
          rationalSeconds(p.comp.timeOfFrame(frame: f))
      ];
      expect(at.map((t) => evaluateKeys(bulging, t)).reduce(math.max),
          greaterThan(100),
          reason: 'the span really does overshoot — otherwise this proves '
              'nothing about the clamp');
      expect(
          at
              .map((t) => channels.single.drawnValueAt(bulging, t))
              .reduce(math.max),
          100);
    });

    // --- the Vegas speed envelope ---------------------------------------

    /// Turn the preference on the way Settings does, then open the layer's
    /// Retime row — which is where the default-lens rule fires.
    Future<void> openRetime(WidgetTester tester, dynamic p,
        {required bool vegas}) async {
      (p.uiState as LumitUiState).workspace.interface.retimeOpensToSpeed =
          vegas;
      final layer = (p as dynamic).layer as LayerReference;
      layer.toggleRetimeProperty();
      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(1280, 600),
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('tl-graph')));
      await tester.pump();
      // The graph's outline is the Layers outline, so the Retime row
      // is reached by twirling the layer open, as it is in Layers mode.
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Retime'));
      await tester.pump();
    }

    /// The Vegas edit: drag a point's speed and the frames after it change,
    /// while every keyframe time stays exactly where it was.
    testWidgets('dragging an envelope point re-times without moving a key',
        (tester) async {
      final p = withLayer();
      await openRetime(tester, p, vegas: true);

      List<BridgeKeyframe> retimeKeys() =>
          keysOf(p.layer.getRetimeProperty() as BridgeScalar);
      final timesBefore = [
        for (final k in retimeKeys()) p.comp.frameAtTime(time: k.time)
      ];
      final lastBefore = retimeKeys().last.value;

      // Drag the first point upwards: faster, so more source is consumed.
      await _drag(
          tester,
          find.byKey(ValueKey<String>(
              'graph-key-${p.layer.internallayerId}/retime#0-out')),
          const Offset(0, -60));

      final after = retimeKeys();
      expect(after.last.value, greaterThan(lastBefore),
          reason: 'speeding the first span up advances further into the '
              'source by the end');
      expect(after.first.value, closeTo(0, 1e-6),
          reason: 'the start is pinned — a clip still begins where it began');
      expect([
        for (final k in after) p.comp.frameAtTime(time: k.time)
      ], timesBefore, reason: 'no keyframe moved in time: beats stay synced');
    });

    // The straightness invariant this lens shares with the sequence view —
    // moving a point in time keeps its speed and re-works the values, so each
    // span stays the line its two points describe — is pinned in
    // `graph_maths_test.dart` against `moveEnvelopePoint` itself. A widget
    // test here cannot see it: a dot's speed comes from the pointer's own
    // height, so *every* drag re-integrates on commit and the bend never
    // survives to be asserted on. The unit test fails without the fix; this
    // one could not, so it is not written.

    // --- planting and lifting keys, and Shift-constrained drags -----------

    testWidgets('double-clicking the curve plants a key without moving it',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 100]);
      await mountGraph(tester, p);

      final before = opacityKeys(p.layer);
      expect(before, hasLength(2));
      final atHalf = evaluateKeys(before, 0.5);

      // Halfway between the two keys: on a straight span that is exactly on
      // the curve, whatever the framing happens to be.
      final base =
          'graph-key-${p.layer.internallayerId}/transform/opacity@opacity';
      final a = tester.getCenter(find.byKey(ValueKey<String>('$base#0')));
      final b = tester.getCenter(find.byKey(ValueKey<String>('$base#1')));
      final mid = Offset((a.dx + b.dx) / 2, (a.dy + b.dy) / 2);
      await tester.tapAt(mid);
      await tester.pump(kDoubleTapMinTime);
      await tester.tapAt(mid);
      await tester.pumpAndSettle();

      final after = opacityKeys(p.layer);
      expect(after, hasLength(3), reason: 'a key was planted');
      // The curve is unchanged where it already was: planting a point is a
      // place to grab, not an edit.
      expect(evaluateKeys(after, 0.5), closeTo(atHalf, 1e-6));
    });

    testWidgets('Alt-clicking a key lifts it', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);
      expect(opacityKeys(p.layer), hasLength(3));

      final key = find.byKey(ValueKey<String>(
          'graph-key-${p.layer.internallayerId}/transform/opacity@opacity#1'));
      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.tap(key);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.pumpAndSettle();

      expect(opacityKeys(p.layer), hasLength(2), reason: 'the middle key went');
    });

    /// Shift holds a key drag to one axis, chosen by which way the pointer
    /// went furthest in pixels.
    testWidgets('Shift holds a key drag to one axis', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);

      final id =
          'graph-key-${p.layer.internallayerId}/transform/opacity@opacity#1';
      // Compared as sets, because a key dragged far enough in time overtakes
      // its neighbour and the list re-sorts — which says nothing about
      // whether the constraint held.
      List<double> values() =>
          [for (final k in opacityKeys(p.layer)) k.value]..sort();
      List<int> frames() => [
            for (final k in opacityKeys(p.layer))
              p.comp.frameAtTime(time: k.time)
          ]..sort();
      final beforeValues = values();
      final beforeFrames = frames();

      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      // Mostly sideways, a little up: the sideways travel wins, so the value
      // must not move at all.
      await _drag(
          tester, find.byKey(ValueKey<String>(id)), const Offset(40, -12));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);

      expect(frames(), isNot(beforeFrames), reason: 'it moved in time');
      for (var i = 0; i < beforeValues.length; i++) {
        expect(values()[i], closeTo(beforeValues[i], 1e-6),
            reason: 'and not at all in value');
      }
    });

    // --- the handles, and what a dragged key takes with it (§6.1–6.2) -----

    /// Keys spread across the whole composition, so a handle's reach — a third
    /// of the gap to its neighbour — is worth enough pixels to aim at. Keys a
    /// few frames apart on a long comp draw their handles *on top of* the key,
    /// which is honest geometry and useless for a test about which of the two
    /// the pointer grabbed.
    void spreadOpacity(dynamic p) {
      final last = (p.comp as CompositionReference).durationFrames() - 1;
      animateOpacity(p.comp as CompositionReference, p.layer as LayerReference,
          frames: [0, last ~/ 2, last]);
    }

    // --- TI-6: Graph mode's drawn surface (§3.3, §6.3) --------------------

    /// A ramp on Position x, so the pane can be given two curves in unlike
    /// units — which is what Normalise exists for.
    void animatePositionX(
      CompositionReference comp,
      LayerReference layer, {
      List<int> frames = const [0, 100],
      double scale = 10,
    }) {
      layer.setTransform(
        prop: BridgeTransformProp.positionX,
        value: BridgeScalar.keyframed([
          for (final f in frames)
            BridgeKeyframe(
              time: comp.timeOfFrame(frame: f),
              value: f * scale,
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
    }

    /// **A property row is what puts a curve on the pane**, in both
    /// views — the tick that used to do it went with the graph's own outline,
    /// and selecting a row was always the other way in.
    testWidgets('picking a property row puts its curve on the pane',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer);
      animatePositionX(p.comp, p.layer);
      await mountGraph(tester, p, selectOpacity: false);
      final id = p.layer.internallayerId;

      Finder glyph(String channel) =>
          find.byKey(ValueKey<String>('graph-key-$id/$channel#0'));

      await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();

      await tester.tap(find.text('Opacity'));
      await tester.pump();
      expect(glyph('transform/opacity@opacity'), findsOneWidget);

      // A second row, added to the selection rather than replacing it: both
      // curves share the pane, on one value scale (the per-curve ranges died
      // with Normalise).
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.tap(find.text('Position'));
      await tester.pump();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      expect(glyph('transform/opacity@opacity'), findsOneWidget,
          reason: 'the first curve stayed');
      expect(glyph('transform/positionX@positionX'), findsOneWidget);

      // One value scale for both, so 100 and 1000 are nowhere near each other
      // — which is what "the curves share the value scale again" means.
      double topOf(String channel) => tester
          .getCenter(find.byKey(ValueKey<String>('graph-key-$id/$channel#1')))
          .dy;
      expect(
          (topOf('transform/opacity@opacity') -
                  topOf('transform/positionX@positionX'))
              .abs(),
          greaterThan(20));
    });

    // --- TI-7: the transform box and numeric entry (§6.2) -----------------

    /// Put two keys in hand — the first and the last of a ramp spread across
    /// the whole composition ([spreadOpacity]), so the box is worth aiming at
    /// in both axes and the key between them is a witness that only the
    /// selection moves. Keys a few frames apart on a long comp draw a box a
    /// dozen pixels wide, whose edges sit under their own keys' targets.
    Future<void> selectEnds(WidgetTester tester, dynamic p) async {
      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 0))));
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 2))));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
    }

    /// The box's left and right edges scale **time** about the opposite edge
    /// (docs/07 §5.3).
    testWidgets("the box's right edge scales time about the left edge",
        (tester) async {
      final p = withLayer();
      spreadOpacity(p);
      await mountGraph(tester, p);
      await selectEnds(tester, p);

      List<int> frames() => [
            for (final k in opacityKeys(p.layer))
              p.comp.frameAtTime(time: k.time)
          ];
      final before = frames();
      final values = [for (final k in opacityKeys(p.layer)) k.value];

      await _drag(tester, find.byKey(const ValueKey('graph-box-right')),
          const Offset(-80, 0));

      final after = frames();
      expect(after[0], before[0], reason: 'the edge not in hand is the anchor');
      expect(after[2], lessThan(before[2]),
          reason: 'and the edge in hand went where the pointer put it');
      expect(after[1], before[1],
          reason: 'a key the selection does not hold does not move');
      expect([for (final k in opacityKeys(p.layer)) k.value], values,
          reason: 'a time edge changes nothing about value');

      p.state.project!.undo();
      expect(frames(), before, reason: 'the whole scale is one undo step');
    });

    /// The readout pill rides with the scale and leaves with it (P1), and
    /// `Escape` abandons the whole gesture (P3, §8's gap 19).
    testWidgets('Escape abandons a box scale and writes nothing',
        (tester) async {
      final p = withLayer();
      spreadOpacity(p);
      await mountGraph(tester, p);
      await selectEnds(tester, p);

      List<int> frames() => [
            for (final k in opacityKeys(p.layer))
              p.comp.frameAtTime(time: k.time)
          ];
      final before = frames();
      final hint = find.byKey(const ValueKey('graph-box-hint'));
      expect(hint, findsNothing, reason: 'nothing at rest');

      final gesture = await tester.startGesture(
          tester.getCenter(find.byKey(const ValueKey('graph-box-right'))));
      await tester.pump();
      await gesture.moveBy(const Offset(-60, 0));
      await tester.pump();
      expect(hint, findsOneWidget, reason: 'the readout rides under the hand');

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      expect(hint, findsNothing, reason: 'and goes with the abandoned gesture');

      // The pointer carries on moving, as a real one does: an abandoned drag
      // must not follow it.
      await gesture.moveBy(const Offset(-60, 0));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      expect(frames(), before, reason: 'Escape wrote nothing at all');
    });

    /// **Numeric entry** (docs/07 §5.3): double-clicking a key opens its exact
    /// frame, value and influences.
    testWidgets('double-clicking a key opens its exact fields', (tester) async {
      final p = withLayer();
      spreadOpacity(p);
      await mountGraph(tester, p);

      final key = find.byKey(ValueKey<String>(opacityKey(p.layer, 1)));
      await tester.tap(key);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(key);
      await tester.pumpAndSettle();

      for (final field in const ['frame', 'value', 'in', 'out']) {
        expect(
            find.byKey(ValueKey<String>('graph-fields-$field')), findsOneWidget,
            reason: 'the exact $field field');
      }

      final was = p.comp.frameAtTime(time: opacityKeys(p.layer)[1].time);
      await _type(tester, 'graph-fields-value', '80');

      final keys = opacityKeys(p.layer);
      expect(keys, hasLength(3), reason: 'typing a value plants nothing');
      expect(keys[1].value, 80, reason: 'the key holds exactly what was typed');
      expect(p.comp.frameAtTime(time: keys[1].time), was,
          reason: 'and nothing else about it changed');
    });

    /// A typed frame moves the key, and cannot be typed past its neighbours —
    /// the fields are bounded by them, because the box is holding an index
    /// into a list a re-sort would shuffle.
    testWidgets('a typed frame moves the key, inside its neighbours',
        (tester) async {
      final p = withLayer();
      spreadOpacity(p);
      await mountGraph(tester, p);

      final key = find.byKey(ValueKey<String>(opacityKey(p.layer, 1)));
      await tester.tap(key);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(key);
      await tester.pumpAndSettle();

      final last = p.comp.durationFrames() - 1;
      await _type(tester, 'graph-fields-frame', '${last + 500}');

      final keys = opacityKeys(p.layer);
      expect(keys, hasLength(3));
      expect(p.comp.frameAtTime(time: keys[1].time), last - 1,
          reason: 'held one frame short of the key after it, never past it');
      expect(p.comp.frameAtTime(time: keys[2].time), last,
          reason: 'which is still where it was');
    });

    // --- Keyframe speed (docs/07 §5.3) --------------------------------------

    /// The key menu's *Keyframe speed…* opens the dialogue on the key's four
    /// numbers, and Apply writes the ones typed - and only those.
    testWidgets('the key menu\'s Keyframe speed writes the typed speed',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);

      await tester.tapAt(
        tester.getCenter(find.byKey(ValueKey<String>(opacityKey(p.layer, 1)))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Keyframe speed…'));
      await tester.pumpAndSettle();

      for (final well in const [
        'speed-in',
        'influence-in',
        'speed-out',
        'influence-out'
      ]) {
        expect(find.byKey(ValueKey<String>('key-$well')), findsOneWidget,
            reason: 'the $well well');
      }
      // A straight key on a straight ramp arrives and leaves at one speed, so
      // Continuous opens ticked, so untick it to type one side alone.
      tester
          .widget<HouseCheckbox>(find.byKey(const ValueKey('key-continuous')))
          .onChanged!(false);
      await tester.pump();
      await _type(tester, 'key-speed-out', '30');
      await tester.tap(find.byKey(const ValueKey('keyframe-confirm')));
      await tester.pumpAndSettle();

      final key = opacityKeys(p.layer)[1];
      expect(key.interpOut, isA<BridgeSideInterp_Bezier>());
      expect((key.interpOut as BridgeSideInterp_Bezier).field0.speed, 30,
          reason: 'the typed speed, in units per second');
      expect(sideInfluence(key.interpOut), closeTo(1 / 3, 1e-9),
          reason: 'the reach it already showed');
      expect(key.interpIn, isA<BridgeSideInterp_Linear>(),
          reason: 'the side not typed into is untouched');
    });

  }, skip: !engineAvailable);
}

/// Click a value well open and type [text] into it, then commit with Enter.
///
/// The field is found *inside* the well rather than as the first
/// [EditableText] on screen: the Timeline panel has fields of its own, and the
/// popover is a route over the top of them.
Future<void> _type(WidgetTester tester, String key, String text) async {
  final well = find.byKey(ValueKey<String>(key));
  await tester.tap(well);
  await tester.pump();
  await tester.enterText(
      find.descendant(of: well, matching: find.byType(EditableText)), text);
  await tester.testTextInput.receiveAction(TextInputAction.done);
  await tester.pumpAndSettle();
}

/// Drag from a widget's centre in steps, as a real pointer moves.
Future<void> _drag(WidgetTester tester, Finder from, Offset by) async {
  final gesture = await tester.startGesture(tester.getCenter(from));
  await tester.pump();
  const steps = 10;
  for (var i = 0; i < steps; i++) {
    await gesture.moveBy(by / steps.toDouble());
    await tester.pump();
  }
  await gesture.up();
  await tester.pumpAndSettle();
}
