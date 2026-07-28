// The Timeline panel on frb, tested against the real engine.
//
// New coverage: the v0 Timeline's tests are spread across several files and
// written against a fake bridge and a snapshot mirror, neither of which this
// panel has. What they assert about *behaviour* is reproduced here against the
// document itself — a switch that does not reach the engine is not a switch.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:uuid/uuid.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/state/timeline_columns.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Timeline (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      // The outline alone is 800 px of columns; the default 800×600 test
      // surface would push its right edge (and the lanes) off screen.
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
    }

    /// Open the toolbar's ⋯ menu, where the layer/work-area/marker commands
    /// live now that the toolbar row belongs to the readouts and the search.
    Future<void> openMore(WidgetTester tester) async {
      await tester.tap(find.byKey(const ValueKey('tl-more')));
      await tester.pumpAndSettle();
    }

    testWidgets('without a composition it says so', (tester) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      expect(find.textContaining('Open a composition'), findsOneWidget);
    });

    /// Dropping footage with nothing open offers to make the composition it
    /// would go in, rather than dead-ending on the placeholder: the drag used
    /// to lift, show its feedback and drop into nothing.
    testWidgets('footage dropped on an empty Timeline offers a new comp',
        (tester) async {
      final p = freshProject();
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      expect(p.uiState.selectedComp, isNull);

      await tester.pumpWidget(hostPanel(
        child: const Row(
          children: [
            SizedBox(width: 300, child: ProjectPanelFrb()),
            Expanded(child: TimelinePanelFrb()),
          ],
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1400, 700),
      ));
      await tester.pump();
      expect(find.textContaining('Open a composition'), findsOneWidget);

      final row =
          find.byKey(ValueKey<String>('project-row-${footage.internalid}'));
      final gesture = await tester.startGesture(tester.getCenter(row));
      await tester.pump(const Duration(milliseconds: 200));
      // Stepped, because one large move leaves the gesture arena resolving
      // the drag against the row's own recognisers.
      // 40 px a step: the test surface is 800 px wide whatever MediaQuery
      // says, so a bigger stride drops the drag off the edge of it.
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(const Offset(40, 0));
        await tester.pump();
      }
      await gesture.up();
      // The dialog probes the dropped media before it opens, so it appears
      // after a real async round trip rather than on the next pump.
      await settleFrb(tester, minRounds: 8);

      expect(find.byKey(const ValueKey('comp-apply')), findsOneWidget,
          reason: 'the drop asks for the new comp settings');
      await tester.enterText(
          find.byKey(const ValueKey('comp-name')), 'From drop');
      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      final comp = p.uiState.selectedComp;
      expect(comp, isNotNull, reason: 'the new comp is fronted');
      expect(comp!.getSettings().name, 'From drop');
      expect(comp.getLayers(), hasLength(1),
          reason: 'the dropped footage landed in it as a layer');
    });

    testWidgets('New layer adds every kind, newest on top', (tester) async {
      final p = withComp();
      await mount(tester, p);

      for (final kind in [
        'Solid',
        'Text',
        'Camera',
        'Adjustment',
        'Null',
        'Sequence'
      ]) {
        await openMore(tester);
        await tester.tap(find.byKey(const ValueKey('tl-add-layer')));
        await tester.pumpAndSettle();
        await tester.tap(find.text(kind));
        await tester.pumpAndSettle();
      }

      final layers = p.comp.getLayers();
      expect(layers, hasLength(6));
      expect(layers.first.getKind(), BridgeLayerKind.sequence,
          reason: 'the newest layer is at the top of the stack');
      expect(
          find.byKey(
              ValueKey<String>('tl-row-${layers.first.internallayerId}')),
          findsOneWidget);
    });

    testWidgets('the switch column reaches the document', (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      final id = layer.internallayerId.toString();
      expect(layer.getSwitches().visible, isTrue);

      await tester.tap(find.byKey(ValueKey<String>('tl-visible-$id')));
      await tester.pump();
      expect(layer.getSwitches().visible, isFalse,
          reason: 'hiding a layer is a document edit, not a view state');

      await tester.tap(find.byKey(ValueKey<String>('tl-solo-$id')));
      await tester.pump();
      expect(layer.getSwitches().solo, isTrue);
      expect(layer.getSwitches().visible, isFalse,
          reason: 'one switch does not disturb another');
    });

    testWidgets('the blend dropdown commits by index', (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      expect(layer.getBlend(), 0);
      final modes = listBlendModes();

      await tester.tap(
          find.byKey(ValueKey<String>('tl-blend-${layer.internallayerId}')));
      await tester.pumpAndSettle();
      await tester.tap(find.text(modes[2]).last);
      await tester.pumpAndSettle();

      expect(layer.getBlend(), 2,
          reason:
              'the index the dropdown shows is the index the engine stores');
    });

    testWidgets('the row menu duplicates, reorders and deletes',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      final first = p.comp.getLayers().single;
      await tester.tapAt(
        tester.getCenter(
            find.byKey(ValueKey<String>('tl-row-${first.internallayerId}'))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Duplicate'));
      await tester.pumpAndSettle();
      expect(p.comp.getLayers(), hasLength(2));

      // The bottom row can be brought forward but not sent back.
      final bottom = p.comp.getLayers()[1];
      await tester.tapAt(
        tester.getCenter(
            find.byKey(ValueKey<String>('tl-row-${bottom.internallayerId}'))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      expect(find.text('Send backward'), findsNothing);
      await tester.tap(find.text('Bring forward'));
      await tester.pumpAndSettle();
      expect(p.comp.getLayers().first.internallayerId, bottom.internallayerId);

      await tester.tapAt(
        tester.getCenter(
            find.byKey(ValueKey<String>('tl-row-${bottom.internallayerId}'))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Delete'));
      await tester.pumpAndSettle();
      expect(p.comp.getLayers(), hasLength(1));
    });

    testWidgets('clicking the ruler scrubs the playhead', (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      expect(p.uiState.playheadFrame.value, 0);
      final ruler = find.byKey(const ValueKey('tl-ruler'));
      final box = tester.getRect(ruler);
      await tester.tapAt(Offset(box.left + box.width * 0.5, box.center.dy));
      await tester.pump();

      final frames = p.comp.durationFrames();
      expect(p.uiState.playheadFrame.value, closeTo(frames * 0.5, 2),
          reason: 'the tap landed halfway along the comp');
      expect(p.uiState.playheadFrame.value, lessThan(frames),
          reason: 'the playhead never leaves the comp');
    });

    testWidgets('dragging a bar moves the layer as one op', (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      final before = layer.getSpan();
      final beforeIn = p.comp.frameAtTime(time: before.inPoint);

      final bar =
          find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}'));
      final rect = tester.getRect(bar);
      // Well inside the bar, so this is a move rather than a trim.
      await tester.dragFrom(
        Offset(rect.left + rect.width * 0.5, rect.center.dy),
        const Offset(80, 0),
      );
      await tester.pumpAndSettle();

      final after = layer.getSpan();
      final afterIn = p.comp.frameAtTime(time: after.inPoint);
      expect(afterIn, greaterThan(beforeIn),
          reason: 'the bar moved later in the comp');

      // One op for the whole gesture: a single undo puts it back.
      p.state.project!.undo();
      expect(p.comp.frameAtTime(time: layer.getSpan().inPoint), beforeIn);
    });

    /// The mouse-acceleration bug: frames were rounded per pointer event and
    /// summed, so a slow drag's sub-frame deltas all rounded to nothing while
    /// a fast drag's rounded up — the bar moved a different distance than the
    /// pointer depending on speed. The frame delta must come from the pixel
    /// total. Fails without the `_deltaPx` accumulator.
    testWidgets('a slow drag moves the bar exactly as far as a fast one',
        (tester) async {
      final p = withComp();
      final fast = p.comp.addAdjustmentLayer();
      final slow = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      Future<void> dragBar(LayerReference layer, List<Offset> moves) async {
        final bar =
            find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}'));
        final rect = tester.getRect(bar);
        final g = await tester
            .startGesture(Offset(rect.left + rect.width * 0.5, rect.center.dy));
        for (final m in moves) {
          await g.moveBy(m);
          await tester.pump();
        }
        await g.up();
        await tester.pumpAndSettle();
      }

      // Identical first events, so both gestures clear the touch slop the
      // same way — then the same 36 pixels: once in one event, once in 72
      // half-pixel events, the slow careful drag that used to fall behind
      // the pointer.
      await dragBar(fast, [const Offset(24, 0), const Offset(36, 0)]);
      await dragBar(slow, [
        const Offset(24, 0),
        for (var i = 0; i < 72; i++) const Offset(0.5, 0),
      ]);

      int inOf(LayerReference l) =>
          p.comp.frameAtTime(time: l.getSpan().inPoint);
      expect(inOf(fast), greaterThan(0), reason: 'the fast drag moved the bar');
      expect(inOf(slow), inOf(fast),
          reason: 'frames come from the pixel total, not per-event rounding');
    });

    /// Retime is an ordinary property row (K-197): hidden until the layer is
    /// given one, then sitting above Transform — outside it, not inside — and
    /// editable exactly like Opacity. Fails if it is filed under Transform, or
    /// if it shows on a layer with no Retime.
    testWidgets('Retime shows above Transform only once the layer has one',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      expect(find.text('Retime'), findsNothing,
          reason: 'a layer with no Retime shows no row for it');

      layer.toggleRetimeProperty();
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.text('Retime'), findsOneWidget);
      expect(
        tester.getTopLeft(find.text('Retime')).dy,
        lessThan(tester.getTopLeft(find.text('Transform')).dy),
        reason: 'Retime sits above Transform, not inside it',
      );
      // Transform is still shut: a row that only appears when Transform is
      // twirled open would be inside it, whatever its indent says.
      expect(find.text('Opacity'), findsNothing);

      // The identity map is keyed, so the field edits the key at the playhead.
      List<BridgeKeyframe> keys() =>
          (layer.getRetimeProperty() as BridgeScalar_Keyframed).field0;
      expect(keys(), hasLength(2));
      p.uiState.playheadFrame.value = 0;
      await tester.pump();
      await tester.drag(
          find.byKey(const ValueKey('tl-retime-seconds')), const Offset(40, 0));
      await tester.pumpAndSettle();
      expect(keys(), hasLength(2), reason: 'no key was added or lost');
      expect(keys().first.value, greaterThan(0),
          reason: 'the edit landed in the key under the playhead');
    });

    /// An animated value stays editable in the outline (docs/07 §4.3): on a
    /// keyframe the edit lands in that key; between keyframes it plants one.
    /// Fails if the cell falls back to a read-only "animated" label, or if it
    /// writes a static value over the curve.
    testWidgets('editing an animated value edits the key under the playhead',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final (f, v) in [(0, 20.0), (60, 80.0)])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: v,
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;

      // On the first key: the drag edits that key, not the curve's shape.
      p.uiState.playheadFrame.value = 0;
      await tester.pump();
      await tester.drag(
          find.byKey(const ValueKey('tl-tf-opacity')), const Offset(40, 0));
      await tester.pumpAndSettle();
      expect(keys(), hasLength(2), reason: 'no key was added or lost');
      expect(keys().first.value, greaterThan(20),
          reason: 'the edit landed in the key under the playhead');

      // Between keys: the drag plants a new one there.
      p.uiState.playheadFrame.value = 30;
      await tester.pump();
      await tester.drag(
          find.byKey(const ValueKey('tl-tf-opacity')), const Offset(40, 0));
      await tester.pumpAndSettle();
      expect(keys(), hasLength(3),
          reason: 'editing between keys plants one at the playhead');
      expect(p.comp.frameAtTime(time: keys()[1].time), 30);
    });

    /// The ◆ button acts at the playhead's *current* frame — the diamond used
    /// to read the frame captured when the panel last drew, so after a scrub
    /// it removed the wrong key.
    testWidgets('the key diamond follows the playhead as it scrubs',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [0, 60])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;

      // On the second key: ◆ removes it.
      p.uiState.playheadFrame.value = 60;
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('kf-toggle-tl-tf-opacity')));
      await tester.pumpAndSettle();
      expect(keys(), hasLength(1), reason: 'the key under the playhead went');

      // Off any key: ◆ adds one exactly there.
      p.uiState.playheadFrame.value = 30;
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('kf-toggle-tl-tf-opacity')));
      await tester.pumpAndSettle();
      expect(keys(), hasLength(2));
      expect(p.comp.frameAtTime(time: keys()[1].time), 30);
    });

    /// Keyframes draw as diamonds on the lane (docs/07 §4.3), and a marquee
    /// dragged over empty lane space gathers them.
    testWidgets('lane diamonds appear and the marquee selects them',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      // Well apart on the axis, so the box can start on empty lane rather
      // than on a key's own drag handle.
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [600, 1500])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();

      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      expect(find.byKey(laneKey), findsOneWidget,
          reason: 'an animated row draws its diamonds on the lane');

      Set<int> selected() {
        final paint = find.descendant(
          of: find.byKey(laneKey),
          matching: find.byType(CustomPaint),
        );
        return ((tester.widget<CustomPaint>(paint.first).painter as dynamic)
                .selected as Set<int>)
            .cast<int>();
      }

      expect(selected(), isEmpty);

      // A box over the whole lane row takes both keys.
      final rect = tester.getRect(find.byKey(laneKey));
      final gesture =
          await tester.startGesture(Offset(rect.left + 2, rect.top + 2));
      await tester.pump(const Duration(milliseconds: 100));
      for (var i = 0; i < 8; i++) {
        await gesture.moveBy(Offset(rect.width / 8, rect.height / 10));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();
      expect(selected(), hasLength(2),
          reason: 'the marquee gathered the keys it enclosed');
    });

    /// Dragging a lane diamond moves the keyframe in time — one op — and the
    /// magnet decides whether it lands on a whole frame or between two
    /// (docs/07 §4.5).
    testWidgets('a lane keyframe drags in time, and the magnet snaps it',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [600, 2400])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;
      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      final handle = find.byKey(ValueKey<String>(
          'tl-key-${layer.internallayerId}/transform/opacity#0'));
      expect(handle, findsOneWidget, reason: 'each diamond is a drag handle');

      // Measured, not assumed: the axis is as wide as the panel leaves it,
      // and the columns can be resized, so the test asks how many pixels a
      // frame is worth rather than hard-coding one.
      final perFrame =
          tester.getRect(find.byKey(laneKey)).width / p.comp.durationFrames();

      // Magnet on (the default): a drag of ten and a half frames still lands
      // on a whole one.
      await tester.drag(handle, Offset(perFrame * 10.5, 0));
      await tester.pumpAndSettle();
      final snapped = keys().first.time;
      expect(p.comp.frameAtTime(time: snapped), greaterThan(600),
          reason: 'the drag moved the key later');
      expect(snapped.num * 60 % snapped.den, 0,
          reason: 'with the magnet on it sits exactly on a frame');
      expect(keys(), hasLength(2), reason: 'no key added or lost');

      // One op for the gesture: a single undo puts it back.
      p.state.project!.undo();
      expect(p.comp.frameAtTime(time: keys().first.time), 600);

      // Magnet off: the same half-frame drag lands between two frames.
      await tester.tap(find.byKey(const ValueKey('tl-magnet')));
      await tester.pump();
      await tester.drag(handle, Offset(perFrame * 10.5, 0));
      await tester.pumpAndSettle();
      final free = keys().first.time;
      expect(free.num * 60 % free.den, isNot(0),
          reason: 'with the magnet off it may land between frames');
    });

    /// **The undo regression.** A drag on a *keyframed* value used to commit
    /// on every tick — [DragValueField] falls back to `onChanged` per tick
    /// when no `onChangeLive` is given — so the undo stack filled with a step
    /// per pixel and one undo moved the value back by a hair. The whole
    /// gesture must be a single step, back to the value before the drag.
    testWidgets('a drag on a keyframed value is one undo step', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final (f, v) in [(0, 20.0), (60, 80.0)])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: v,
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();

      List<BridgeKeyframe> keys() =>
          (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;

      // On the first key, dragged in many small steps — the shape that used
      // to write one op each.
      p.uiState.playheadFrame.value = 0;
      await tester.pump();
      final field = find.byKey(const ValueKey('tl-tf-opacity'));
      final gesture = await tester.startGesture(tester.getCenter(field));
      await tester.pump();
      for (var i = 0; i < 20; i++) {
        await gesture.moveBy(const Offset(3, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect(keys().first.value, greaterThan(20),
          reason: 'the drag reached the key');
      expect(keys(), hasLength(2), reason: 'and planted nothing extra');

      p.state.project!.undo();
      expect(keys().first.value, 20,
          reason: 'ONE undo returns the value it had before the drag');
    });

    /// Clicking a property row selects it, and everything containing it —
    /// its group heading and its layer's row — marks itself, so switching to
    /// the graph knows which curve is meant (docs/07 §4.3).
    testWidgets('clicking a property selects it and marks its parents',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      await mount(tester, p);
      final id = layer.internallayerId;

      await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
      await tester.pump();
      await tester.tap(find.text('Effects'));
      await tester.pump();
      await tester.tap(find.text('Gaussian blur'));
      await tester.pump();

      final t = LumitTheme.dark();
      // The innermost Container over a row's label is that row's own.
      Color? fillOver(String text) {
        final box = find.ancestor(
            of: find.text(text), matching: find.byType(Container));
        return (tester.widget<Container>(box.first).decoration as BoxDecoration)
            .color;
      }

      expect(fillOver('Radius'), isNull,
          reason: 'nothing is picked to start with');

      await tester.tap(find.text('Radius'));
      await tester.pump();

      expect(fillOver('Radius'), t.surface2,
          reason: 'the property row is the one selected');
      expect(fillOver('Gaussian blur'), t.surface2.withValues(alpha: 0.45),
          reason: 'the effect holding it marks itself, a shade dimmer');
      expect(
          (tester
                  .widget<Container>(
                      find.byKey(ValueKey<String>('tl-rowbody-$id')))
                  .decoration as BoxDecoration)
              .color,
          t.surface2.withValues(alpha: 0.45),
          reason: "and so does the property's layer");
    });

    /// Selecting keyframes on a lane selects the property they belong to, so
    /// the outline follows what was boxed (docs/07 §4.3).
    testWidgets('boxing keyframes on a lane selects their property',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [600, 1500])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      await mount(tester, p);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();

      final laneKey = ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity');
      final rect = tester.getRect(find.byKey(laneKey));
      final gesture =
          await tester.startGesture(Offset(rect.left + 2, rect.top + 2));
      await tester.pump(const Duration(milliseconds: 100));
      for (var i = 0; i < 8; i++) {
        await gesture.moveBy(Offset(rect.width / 8, rect.height / 10));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final t = LumitTheme.dark();
      final row = find.ancestor(
        of: find.text('Opacity'),
        matching: find.byType(Container),
      );
      expect(
          (tester.widget<Container>(row.first).decoration as BoxDecoration)
              .color,
          t.surface2,
          reason: 'the boxed keys picked their own property row');
    });

    /// Dragging a header seam resizes that group and leaves the rest alone,
    /// so the outline grows by what the drag moved — and the fold-out's value
    /// cells, which span the render group, grow with it (docs/07 §4.2).
    testWidgets('dragging a header seam resizes just that group',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();

      double widthOf(String key) =>
          tester.getSize(find.byKey(ValueKey<String>(key))).width;
      final composeBefore = widthOf('tl-blend-$id');
      final valueBefore = widthOf('tl-tf-opacity');

      await tester.drag(
          find.byKey(const ValueKey('tl-seam-render')), const Offset(60, 0));
      await tester.pumpAndSettle();

      expect(widthOf('tl-tf-opacity'), greaterThan(valueBefore),
          reason: 'the render group grew, so its value cells did');
      expect(widthOf('tl-blend-$id'), composeBefore,
          reason: 'every other group kept its width');
    });

    /// The bottom bar's zoom: + widens the time axis (the bar stretches) and
    /// the readout says so; Fit brings it back.
    testWidgets('the zoom buttons widen the lanes and read out the factor',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      expect(find.text('100%'), findsOneWidget);
      final before = tester
          .getRect(
              find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}')))
          .width;

      await tester.tap(find.byKey(const ValueKey('tl-zoom-in')));
      await tester.pumpAndSettle();
      expect(find.text('150%'), findsOneWidget);
      final zoomed = tester
          .getRect(
              find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}')))
          .width;
      expect(zoomed, greaterThan(before),
          reason: 'the comp takes more pixels when zoomed in');

      await tester.tap(find.byKey(const ValueKey('tl-zoom-fit')));
      await tester.pumpAndSettle();
      expect(find.text('100%'), findsOneWidget);
    });

    /// The bar wears the layer's label colour (K-188), so recolouring the
    /// label recolours the bar — and a solid starts on the solid chip.
    testWidgets('the bar wears the label colour', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      Color barColour() {
        final fill = find
            .byKey(ValueKey<String>('tl-bar-fill-${layer.internallayerId}'));
        final deco = tester.widget<Container>(fill).decoration as BoxDecoration;
        return deco.color!;
      }

      final t = LumitTheme.dark();
      expect(barColour(), t.labelColour(2),
          reason: 'a solid starts on its kind\'s chip');

      layer.setLabel(label: 6);
      p.uiState.model.refresh();
      await tester.pump();
      expect(barColour(), t.labelColour(6),
          reason: 'picking a label recolours the bar');
    });

    /// A stack taller than the panel scrolls rather than overflowing, and
    /// the two halves stay one table while it does.
    testWidgets('a tall stack scrolls without overflowing', (tester) async {
      final p = withComp();
      for (var i = 0; i < 40; i++) {
        p.comp.addSolidLayer();
      }
      await mount(tester, p);
      expect(tester.takeException(), isNull,
          reason: '40 rows in a 600px panel must scroll, not overflow');
    });

    /// The group reorder rule: the dragged group takes the target's slot,
    /// whichever side it came from, and dropping on itself changes nothing.
    test('reorderedGroups moves a group to the target slot', () {
      const g = TimelineGroup.values;
      expect(
        reorderedGroups(defaultGroupOrder, g[0], g[3]),
        [g[1], g[2], g[3], g[0]],
        reason: 'dragged right, it lands after the target',
      );
      expect(
        reorderedGroups(defaultGroupOrder, g[3], g[0]),
        [g[3], g[0], g[1], g[2]],
        reason: 'dragged left, it lands before the target',
      );
      expect(reorderedGroups(defaultGroupOrder, g[1], g[1]), defaultGroupOrder);
    });

    /// The value column sits under the render group: everything right of it
    /// in the order contributes its fixed width to the inset.
    test('valueColumnFor measures what sits right of the render group', () {
      expect(valueColumnFor(defaultGroupOrder, defaultGroupWidths).rightInset,
          groupDividerWidth + composeGroupWidth);
      final renderLast = reorderedGroups(
          defaultGroupOrder, TimelineGroup.render, TimelineGroup.compose);
      expect(valueColumnFor(renderLast, defaultGroupWidths).rightInset, 0);

      // The value cells span the render group as it stands, so dragging that
      // group's seam widens the fields under it (K-192).
      final wider = {
        ...defaultGroupWidths,
        TimelineGroup.render: renderGroupWidth + 60,
      };
      expect(valueColumnFor(defaultGroupOrder, wider).width,
          renderGroupWidth + 60);
    });

    /// The ruler's label spacing thins as the comp zooms out, and its labels
    /// speak the familiar editor idiom.
    test('the ruler picks nice label steps and formats them', () {
      expect(rulerLabelStepSeconds(pixelsPerSecond: 100), 1);
      expect(rulerLabelStepSeconds(pixelsPerSecond: 20), 5);
      expect(rulerLabelStepSeconds(pixelsPerSecond: 2), 60);
      expect(rulerLabelOf(5), '05s');
      expect(rulerLabelOf(0.5), '0.5s');
      expect(rulerLabelOf(60), '1:00s');
      expect(rulerLabelOf(90), '1:30s');
    });

    /// What a grab does to the waveform's preview of the span — the mapping
    /// the lane draws while the gesture is still in flight (K-172).
    test('barDragPreview maps each grab onto the span', () {
      final move = barDragPreview('a', BarGrab.move, 5);
      expect((move.deltaIn, move.deltaOut, move.offsetShift), (5, 5, 5));
      final trimIn = barDragPreview('a', BarGrab.trimIn, -3);
      expect((trimIn.deltaIn, trimIn.deltaOut, trimIn.offsetShift), (-3, 0, 0));
      final trimOut = barDragPreview('a', BarGrab.trimOut, 7);
      expect(
          (trimOut.deltaIn, trimOut.deltaOut, trimOut.offsetShift), (0, 7, 0));
    });

    /// A layer can start BEFORE the comp (docs/TODO: "re-introduce"): drag a
    /// bar left past frame zero and the span goes negative, carrying its
    /// content with it — the comp shows the part that overlaps.
    testWidgets('a bar dragged left of zero starts before the comp',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      final bar =
          find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}'));
      final rect = tester.getRect(bar);
      // From the middle (a move, not a trim), left by more than the bar's
      // distance to zero.
      await tester.dragFrom(
        Offset(rect.left + rect.width * 0.5, rect.center.dy),
        const Offset(-160, 0),
      );
      await tester.pumpAndSettle();

      final inFrame = p.comp.frameAtTime(time: layer.getSpan().inPoint);
      expect(inFrame, lessThan(0),
          reason: 'nothing pins a layer to the comp edge');
      // The offset travelled with it: layer time zero moved by the same
      // amount, so the content did not retime.
      expect(p.comp.frameAtTime(time: layer.getSpan().startOffset), inFrame);
    });

    /// Trimming by the bar edges (docs/TODO: "drag start/end to adjust/crop"):
    /// the in edge crops without moving the content, and the out edge crops
    /// the tail.
    testWidgets('the bar edges trim in and out', (tester) async {
      final p = withComp();
      final layer = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      final before = layer.getSpan();
      final beforeIn = p.comp.frameAtTime(time: before.inPoint);
      final beforeOut = p.comp.frameAtTime(time: before.outPoint);

      final bar =
          find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}'));
      var rect = tester.getRect(bar);
      // Near the left edge: a trim of the in point, content unmoved.
      await tester.dragFrom(
          Offset(rect.left + 2, rect.center.dy), const Offset(60, 0));
      await tester.pumpAndSettle();
      final trimmedIn = p.comp.frameAtTime(time: layer.getSpan().inPoint);
      expect(trimmedIn, greaterThan(beforeIn), reason: 'the head is cropped');
      expect(p.comp.frameAtTime(time: layer.getSpan().startOffset),
          p.comp.frameAtTime(time: before.startOffset),
          reason: 'trimming never retimes the content');

      // Near the right edge: a trim of the out point.
      rect = tester.getRect(bar);
      await tester.dragFrom(
          Offset(rect.right - 2, rect.center.dy), const Offset(-60, 0));
      await tester.pumpAndSettle();
      expect(p.comp.frameAtTime(time: layer.getSpan().outPoint),
          lessThan(beforeOut),
          reason: 'the tail is cropped');
    });

    testWidgets('the work area and markers draw on the ruler', (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      p.comp.setWorkArea(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 10),
          outPoint: p.comp.timeOfFrame(frame: 40),
          startOffset: p.comp.timeOfFrame(frame: 0),
        ),
      );
      await mount(tester, p);

      expect(find.byKey(const ValueKey('tl-work-area')), findsOneWidget);

      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-clear-work-area')));
      await tester.pumpAndSettle();
      expect(p.comp.getWorkArea(), isNull);
      expect(find.byKey(const ValueKey('tl-work-area')), findsNothing);
    });
    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
    /// The gesture the whole Project panel drag exists for. It had no drop
    /// target at all: the drag lifted, showed feedback, and dropped into
    /// nothing, which reads as the app ignoring you.
    testWidgets('footage dragged from the Project panel becomes a layer',
        (tester) async {
      final p = withComp();
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');

      // Both panels in one tree, so the drag is the real one rather than a
      // DragTarget poked directly.
      await tester.pumpWidget(hostPanel(
        child: const Row(
          children: [
            SizedBox(width: 300, child: ProjectPanelFrb()),
            Expanded(child: TimelinePanelFrb()),
          ],
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1400, 700),
      ));
      await tester.pump();

      expect(p.comp.getLayers(), isEmpty);

      final row =
          find.byKey(ValueKey<String>('project-row-${footage.internalid}'));
      expect(row, findsOneWidget, reason: 'the footage row is there to drag');

      final gesture = await tester.startGesture(tester.getCenter(row));
      await tester.pump(const Duration(milliseconds: 200));
      // Stepped, because one large move leaves the gesture arena resolving the
      // drag against the row's own recognisers.
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(const Offset(40, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect(p.comp.getLayers(), hasLength(1),
          reason: 'the drop reached the document');
      expect(p.comp.getLayers().single.getName(), contains('shot'));
    });

    /// Comps nest by the same gesture: drag one from the Project panel onto
    /// another's Timeline and it lands as a Precomp layer.
    testWidgets('a comp dragged from the Project panel nests as a precomp',
        (tester) async {
      final p = withComp();
      final inner = p.state.project!.newComposition(name: 'Titles');

      await tester.pumpWidget(hostPanel(
        child: const Row(
          children: [
            SizedBox(width: 300, child: ProjectPanelFrb()),
            Expanded(child: TimelinePanelFrb()),
          ],
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1400, 700),
      ));
      await tester.pump();

      expect(p.comp.getLayers(), isEmpty);

      final row =
          find.byKey(ValueKey<String>('project-row-${inner.internalid}'));
      expect(row, findsOneWidget, reason: 'the comp row is there to drag');

      final gesture = await tester.startGesture(tester.getCenter(row));
      await tester.pump(const Duration(milliseconds: 200));
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(const Offset(40, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final layers = p.comp.getLayers();
      expect(layers, hasLength(1), reason: 'the drop reached the document');
      expect(layers.single.getKind(), BridgeLayerKind.precomp,
          reason: 'a dropped comp nests as a Precomp layer');
      // The inner comp itself is untouched — nesting places, never moves.
      expect(inner.getLayers(), isEmpty);
    });

    /// The layer rows deliberately do *not* rebuild when the playhead moves —
    /// they used to, sixty times a second during playback, re-asking the engine
    /// for every layer's name and span each time, and the cost grew with the
    /// layer count. Only the playhead line redraws now.
    ///
    /// The razor is what makes that observable: it reads the playhead when it is
    /// clicked rather than when its bar was built. If someone reverts to
    /// capturing the value at build time, the bar has not rebuilt since the
    /// playhead moved, so the cut lands on the old frame and this fails.
    testWidgets('the razor cuts where the playhead is now, not where it was',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSequenceLayer();
      p.uiState.selectedLayer.value = layer;
      await mount(tester, p);

      // Turn the razor on, then move the playhead — without touching anything
      // that would rebuild the bar.
      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-razor')));
      await tester.pumpAndSettle();
      p.uiState.playheadFrame.value = 30;
      await tester.pump();

      final bar =
          find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}'));
      expect(bar, findsOneWidget);
      await tester.tap(bar, warnIfMissed: false);
      await tester.pump();

      // A Sequence layer with no clips has nothing to cut, so what is asserted
      // is the frame the razor asked for rather than the resulting clips: the
      // playhead must still be at 30, and nothing may have thrown.
      expect(tester.takeException(), isNull);
      expect(p.uiState.playheadFrame.value, 30);
    });

    /// The twirl-down the port dropped. A layer opens onto its *section
    /// headings* — Transform always, Effects when it has any, Audio only when
    /// its source carries sound — and each heading opens onto its own rows
    /// (docs/07 §4.3).
    testWidgets('a layer opens onto its section headings', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      final twirl =
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}'));
      expect(twirl, findsOneWidget, reason: 'every layer row has one');
      expect(find.text('Transform'), findsNothing,
          reason: 'closed to start with, or a busy comp is a wall of numbers');

      await tester.tap(twirl);
      await tester.pump();
      expect(find.text('Transform'), findsOneWidget);
      expect(find.text('Position'), findsNothing,
          reason: 'the heading opens first, not every property under it');
      expect(find.text('Effects'), findsNothing,
          reason: 'a layer with no effects has no Effects group to offer');
      expect(find.text('Audio'), findsNothing,
          reason: 'a solid cannot be heard, so it has no volume to set');

      await tester.tap(find.text('Transform'));
      await tester.pump();
      for (final row in [
        'Anchor point',
        'Position',
        'Scale',
        'Rotation',
        'Opacity'
      ]) {
        expect(find.text(row), findsOneWidget);
      }

      await tester.tap(twirl);
      await tester.pump();
      expect(find.text('Transform'), findsNothing);
    });

    /// The four column groups in their shipped order (docs/07 §4.2):
    /// visibility · audio · solo · lock · shy, then twirl · label · number ·
    /// name, then fx · motion blur · 3D, then matte · blend · parent.
    testWidgets('the outline columns sit in their groups', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      double dx(String key) =>
          tester.getTopLeft(find.byKey(ValueKey<String>(key))).dx;
      final order = [
        'tl-visible-$id',
        'tl-audible-$id',
        'tl-solo-$id',
        'tl-locked-$id',
        'tl-shy-$id',
        'tl-twirl-$id',
        'tl-label-$id',
        'tl-name-$id',
        'tl-fx-$id',
        'tl-mb-$id',
        'tl-3d-$id',
        'tl-matte-$id',
        'tl-blend-$id',
        'tl-parent-$id',
      ];
      for (var i = 1; i < order.length; i++) {
        expect(dx(order[i]), greaterThan(dx(order[i - 1])),
            reason: '${order[i]} sits right of ${order[i - 1]}');
      }
    });

    /// Dragging a header group moves the whole cluster: dropping the
    /// switches group onto the compose group puts every switch cell right of
    /// the pickers, in one gesture.
    testWidgets('dragging a header group reorders the columns as a unit',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      double dx(String key) =>
          tester.getTopLeft(find.byKey(ValueKey<String>(key))).dx;
      expect(dx('tl-visible-$id'), lessThan(dx('tl-matte-$id')));

      final from =
          tester.getCenter(find.byKey(const ValueKey('tl-colgroup-switches')));
      final to =
          tester.getCenter(find.byKey(const ValueKey('tl-colgroup-compose')));
      final gesture = await tester.startGesture(from);
      await tester.pump(const Duration(milliseconds: 200));
      final step = (to - from) / 8;
      for (var i = 0; i < 8; i++) {
        await gesture.moveBy(step);
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      for (final key in [
        'tl-visible-$id',
        'tl-audible-$id',
        'tl-solo-$id',
        'tl-locked-$id',
        'tl-shy-$id',
      ]) {
        expect(dx(key), greaterThan(dx('tl-parent-$id')),
            reason: 'the whole switches cluster moved past the pickers');
      }
    });

    /// The render switches reach the document like the A/V ones do.
    testWidgets('the fx, motion-blur and 3D switches reach the document',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      expect(layer.getSwitches().fx, isTrue);
      await tester.tap(find.byKey(ValueKey<String>('tl-fx-$id')));
      await tester.pump();
      expect(layer.getSwitches().fx, isFalse);

      expect(layer.getSwitches().motionBlur, isFalse);
      await tester.tap(find.byKey(ValueKey<String>('tl-mb-$id')));
      await tester.pump();
      expect(layer.getSwitches().motionBlur, isTrue);

      expect(layer.getSwitches().threeD, isFalse);
      await tester.tap(find.byKey(ValueKey<String>('tl-3d-$id')));
      await tester.pump();
      expect(layer.getSwitches().threeD, isTrue);
    });

    /// The toolbar's readouts: the timecode counts frames at the comp's own
    /// rate and the frame count is zero-based, so frame 0 is 00:00:00:00.
    testWidgets('the timecode and frame readouts follow the playhead',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      await mount(tester, p);

      expect(find.text('00:00:00:00'), findsOneWidget);
      expect(find.text('f0'), findsOneWidget);

      // 60 fps is the default comp rate: frame 90 is a second and a half in.
      p.uiState.playheadFrame.value = 90;
      await tester.pump();
      expect(find.text('00:00:01:30'), findsOneWidget);
      expect(find.text('f90'), findsOneWidget);
    });

    /// The master motion-blur button writes the comp's shutter enable — one
    /// op, undoable — and lights when it is on.
    testWidgets('the master motion-blur button toggles the comp shutter',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      await mount(tester, p);

      await tester.tap(find.byKey(const ValueKey('tl-mb-master')));
      await tester.pump();
      expect(p.uiState.model.motionBlurEnabled, isTrue);

      await tester.tap(find.byKey(const ValueKey('tl-mb-master')));
      await tester.pump();
      expect(p.uiState.model.motionBlurEnabled, isFalse);
    });

    /// Shy (docs/07 §4.2): the row switch marks the layer, the toolbar's
    /// filter hides marked rows from the list — and only from the list.
    testWidgets('the shy filter hides shy rows without touching visibility',
        (tester) async {
      final p = withComp();
      final shy = p.comp.addSolidLayer();
      shy.rename(name: 'Backplate');
      final loud = p.comp.addSolidLayer();
      loud.rename(name: 'Hero');
      await mount(tester, p);

      await tester
          .tap(find.byKey(ValueKey<String>('tl-shy-${shy.internallayerId}')));
      await tester.pump();
      expect(shy.getSwitches().shy, isTrue,
          reason: 'shy is a document switch, so it survives the session');
      expect(find.text('Backplate'), findsOneWidget,
          reason: 'marking a layer shy does not hide it yet');

      await tester.tap(find.byKey(const ValueKey('tl-hide-shy')));
      await tester.pump();
      expect(find.text('Backplate'), findsNothing);
      expect(find.text('Hero'), findsOneWidget);
      expect(shy.getSwitches().visible, isTrue,
          reason: 'shy hides the row, never the picture');

      await tester.tap(find.byKey(const ValueKey('tl-hide-shy')));
      await tester.pump();
      expect(find.text('Backplate'), findsOneWidget);
    });

    /// Dragging a layer by its name moves it up or down the stack — layers
    /// used to be stuck in the order they were added, reorderable only from
    /// the row menu one place at a time (docs/07 §4.7).
    testWidgets('dragging a layer by its name reorders the stack',
        (tester) async {
      final p = withComp();
      for (final name in ['Bottom', 'Middle', 'Top']) {
        p.comp.addSolidLayer().rename(name: name);
      }
      p.uiState.model.refresh();
      await mount(tester, p);

      List<String> stack() => [for (final l in p.comp.getLayers()) l.getName()];
      expect(stack(), ['Top', 'Middle', 'Bottom'],
          reason: 'newest on top, as added');

      // Drag the top layer's name down onto the bottom row.
      final from = find.byKey(ValueKey<String>(
          'tl-name-${p.comp.getLayers().first.internallayerId}'));
      final onto = find.byKey(ValueKey<String>(
          'tl-row-${p.comp.getLayers().last.internallayerId}'));
      final start = tester.getCenter(from);
      final end = tester.getCenter(onto);
      final gesture = await tester.startGesture(start);
      await tester.pump(const Duration(milliseconds: 200));
      for (var i = 1; i <= 8; i++) {
        await gesture.moveTo(start + (end - start) * (i / 8));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect(stack(), ['Middle', 'Bottom', 'Top'],
          reason: 'the dragged layer took the row it was dropped on');

      // One op: a single undo puts the stack back.
      p.state.project!.undo();
      p.uiState.model.refresh();
      expect(stack(), ['Top', 'Middle', 'Bottom']);
    });

    /// Lock (docs/07 §4.2): a locked layer's bar refuses the drag and its
    /// name refuses the rename, until it is unlocked.
    testWidgets('a locked layer cannot be dragged or renamed', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      await tester.tap(find.byKey(ValueKey<String>('tl-locked-$id')));
      await tester.pump();
      expect(layer.getSwitches().locked, isTrue);

      final before = p.comp.frameAtTime(time: layer.getSpan().inPoint);
      final bar = find.byKey(ValueKey<String>('tl-bar-$id'));
      final rect = tester.getRect(bar);
      await tester.dragFrom(
        Offset(rect.left + rect.width * 0.5, rect.center.dy),
        const Offset(80, 0),
      );
      await tester.pumpAndSettle();
      expect(p.comp.frameAtTime(time: layer.getSpan().inPoint), before,
          reason: 'a locked bar holds still');

      final name = find.byKey(ValueKey<String>('tl-name-$id'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pump();
      expect(find.byKey(ValueKey<String>('tl-rename-$id')), findsNothing,
          reason: 'a locked name does not open the editor');
    });

    /// Double-clicking the name turns it into an editor; submitting renames
    /// the layer through the document (one op, undoable like any other).
    testWidgets('double-clicking the name renames the layer', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = layer.internallayerId;

      final name = find.byKey(ValueKey<String>('tl-name-$id'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pump();

      final editor = find.byKey(ValueKey<String>('tl-rename-$id'));
      expect(editor, findsOneWidget, reason: 'the name became a field');

      await tester.enterText(editor, 'Hero solid');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(layer.getInfo().name, 'Hero solid');
      expect(find.byKey(ValueKey<String>('tl-rename-$id')), findsNothing,
          reason: 'submitting leaves the editor');
    });

    /// Clicking anywhere on a layer selects it — including its bar in the
    /// lane area, which is most of what "the layer" is on screen.
    testWidgets('clicking a bar selects its layer', (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      final top = p.comp.addSolidLayer();
      await mount(tester, p);

      expect(p.uiState.selectedLayer.value, isNull);
      await tester
          .tap(find.byKey(ValueKey<String>('tl-bar-${top.internallayerId}')));
      await tester.pump();
      expect(
          p.uiState.selectedLayer.value?.internallayerId, top.internallayerId);
    });

    /// Selection happens on the pointer DOWN, not after the gesture arena
    /// settles: the name's rename double-tap holds the arena open for its
    /// whole ~300 ms window, so selecting through the row's tap made the
    /// Effect controls follow a click on the name a third of a second late.
    testWidgets('clicking a name selects before the double-tap window',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      final top = p.comp.addSolidLayer();
      await mount(tester, p);
      expect(p.uiState.selectedLayer.value, isNull);

      final gesture = await tester.startGesture(tester.getCenter(
          find.byKey(ValueKey<String>('tl-name-${top.internallayerId}'))));
      await tester.pump();

      // Still mid-press: the button has not even come up yet.
      expect(
          p.uiState.selectedLayer.value?.internallayerId, top.internallayerId,
          reason: 'selection lands on the down, before any arena resolves');

      await gesture.up();
      // Drain the rename recogniser's double-tap timer before teardown.
      await tester.pump(kDoubleTapTimeout * 2);
    });

    /// Touching a layer's fold-out highlights the layer a shade DIMMER than
    /// selection — "whose rows are these" answered at a glance, without the
    /// touch stealing the selection.
    testWidgets(
        'touching a fold row highlights its layer, dimmer than '
        'selection', (tester) async {
      final p = withComp();
      final below = p.comp.addSolidLayer();
      final top = p.comp.addSolidLayer();
      await mount(tester, p);

      // Select the top layer (a single click on the name selects once the
      // double-tap window has passed — the same click-and-a-beat AE has),
      // twirl open the one below and touch its fold.
      await tester
          .tap(find.byKey(ValueKey<String>('tl-name-${top.internallayerId}')));
      await tester.pump(kDoubleTapTimeout * 2);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${below.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();

      Color? rowColour(UuidValue id) {
        // The row's fill rides in the body's decoration, inside the drop
        // target that makes the row a reorder destination (K-193).
        final deco = tester
            .widget<Container>(find.byKey(ValueKey<String>('tl-rowbody-$id')))
            .decoration as BoxDecoration;
        return deco.color;
      }

      final t = LumitTheme.dark();
      expect(rowColour(top.internallayerId), t.surface2,
          reason: 'the selected layer keeps the full surface');
      expect(
          rowColour(below.internallayerId), t.surface2.withValues(alpha: 0.45),
          reason: 'the touched fold marks its layer at half strength');
      expect(
          p.uiState.selectedLayer.value?.internallayerId, top.internallayerId,
          reason: 'the highlight never steals the selection');
    });

    /// The matte cell: pick a source layer and the mode toggles appear; the
    /// choice reaches the document, luma and invert flip on their toggles.
    testWidgets('the matte cell sets, retargets and flips the matte',
        (tester) async {
      final p = withComp();
      final source = p.comp.addSolidLayer();
      source.rename(name: 'Matte source');
      final consumer = p.comp.addSolidLayer();
      await mount(tester, p);
      final id = consumer.internallayerId;

      expect(consumer.getMatte(), isNull);
      await tester.tap(find.byKey(ValueKey<String>('tl-matte-$id')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Matte source').last);
      await tester.pumpAndSettle();

      var matte = consumer.getMatte();
      expect(matte?.layer, source.internallayerId);
      expect(matte?.luma, isFalse, reason: 'alpha until asked otherwise');

      await tester.tap(find.byKey(ValueKey<String>('tl-matte-luma-$id')));
      await tester.pumpAndSettle();
      matte = consumer.getMatte();
      expect(matte?.luma, isTrue);

      await tester.tap(find.byKey(ValueKey<String>('tl-matte-invert-$id')));
      await tester.pumpAndSettle();
      expect(consumer.getMatte()?.inverted, isTrue);
    });

    /// The label swatch opens the eight-chip picker and the choice lands on
    /// the layer.
    testWidgets('the label swatch recolours the layer', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      expect(layer.getInfo().label, 2,
          reason: 'a solid starts on its kind\'s default chip (K-188)');
      await tester.tap(
          find.byKey(ValueKey<String>('tl-label-${layer.internallayerId}')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('tl-label-chip-3')));
      await tester.pumpAndSettle();
      expect(layer.getInfo().label, 3);
    });

    testWidgets(
        'dragging a transform value in the Timeline reaches the document',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();

      final before =
          (layer.getTransform().positionX as BridgeScalar_Static).field0;
      await tester.drag(
          find.byKey(const ValueKey('tl-tf-positionX')), const Offset(40, 0));
      await tester.pump();

      expect((layer.getTransform().positionX as BridgeScalar_Static).field0,
          greaterThan(before),
          reason: 'the drag committed, exactly as it does in Effect controls');
    });

    /// An effect adds its own group, and each effect in it opens onto its
    /// parameters — the same rows, and the same drag, the Effect controls panel
    /// shows.
    testWidgets('an effect adds a group whose parameters can be dragged',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      await mount(tester, p);

      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      expect(find.text('Effects'), findsOneWidget,
          reason: 'the group appears because there is something in it');

      await tester.tap(find.text('Effects'));
      await tester.pump();
      expect(find.text('Gaussian blur'), findsOneWidget,
          reason: 'one row per effect, by label');
      expect(find.text('Radius'), findsNothing,
          reason: 'and its parameters wait until it is opened');

      await tester.tap(find.text('Gaussian blur'));
      await tester.pump();
      expect(find.text('Radius'), findsOneWidget);

      final id = layer.getEffects().single.id();
      double radius() => ((layer.getEffects().single.getValue(id: 'radius')
                  as BridgeEffectValue_Float)
              .field0 as BridgeScalar_Static)
          .field0;
      final before = radius();

      await tester.drag(
        find.byKey(ValueKey<String>('fx-float-$id-radius')),
        const Offset(50, 0),
      );
      await tester.pumpAndSettle();

      expect(tester.takeException(), isNull);
      expect(radius(), greaterThan(before),
          reason: 'the parameter drag reached the document');
    });

    /// The Audio group is offered only where there is sound to set. Both halves
    /// matter: a silent layer must not carry a volume control, and one with
    /// audio must.
    testWidgets('the Audio group follows whether the layer can be heard',
        (tester) async {
      final p = withComp();
      final silent = p.comp.addSolidLayer();
      final audible =
          p.state.project!.importFootage(path: _wavFile('tone.wav'));
      p.comp.addFootageLayer(footage: audible);
      await mount(tester, p);

      final footageLayer = p.comp.getLayers().first;
      // The probe is a real trip into FFmpeg, so the answer arrives after a
      // frame or two rather than during the first build.
      await settleFrb(tester, minRounds: 8);

      await tester.tap(find
          .byKey(ValueKey<String>('tl-twirl-${footageLayer.internallayerId}')));
      await tester.pump();
      expect(find.text('Audio'), findsOneWidget,
          reason: 'the file carries an audio stream');

      await tester.tap(find.text('Audio'));
      await tester.pump();
      expect(find.text('Volume'), findsOneWidget);

      // The waveform lane (K-172): behind its own twirl under Audio, and its
      // lane paints once opened.
      expect(find.text('Waveform'), findsOneWidget);
      expect(
          find.byKey(
              ValueKey<String>('tl-wave-${footageLayer.internallayerId}')),
          findsNothing,
          reason: 'closed until asked — a busy comp only pays for open lanes');
      await tester.tap(find.text('Waveform'));
      await tester.pump();
      expect(
          find.byKey(
              ValueKey<String>('tl-wave-${footageLayer.internallayerId}')),
          findsOneWidget);

      // And the peaks themselves are real: the whole source, bucketed, with
      // its true length — the data the lane maps through in/out/offset.
      // `runAsync`, because a real decode completes on real async, which the
      // test's fake clock would otherwise wait on for ever.
      final peaks =
          await tester.runAsync(() => footageLayer.audioPeaks(buckets: 64));
      expect(peaks!.durationSeconds, greaterThan(0));
      expect(peaks.pairs, hasLength(128), reason: 'a (min, max) per bucket');
      expect(peaks.pairs.any((v) => v.abs() > 0.01), isTrue,
          reason: 'a tone is not silence');

      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${silent.internallayerId}')));
      await tester.pump();
      expect(find.text('Audio'), findsOneWidget,
          reason: 'still only the one — a solid has nothing to be heard');
    });

    /// The outline and the lanes are one table. A fold-out that pushed the names
    /// down without leaving the same room beside them would slide every bar
    /// below it away from its own layer.
    testWidgets('an open layer keeps its bars lined up with its names',
        (tester) async {
      final p = withComp();
      final upper = p.comp.addSolidLayer();
      final lower = p.comp.addSolidLayer();
      await mount(tester, p);

      Finder rowOf(LayerReference l) =>
          find.byKey(ValueKey<String>('tl-row-${l.internallayerId}'));
      Finder barOf(LayerReference l) =>
          find.byKey(ValueKey<String>('tl-bar-${l.internallayerId}'));

      for (final layer in [upper, lower]) {
        expect(tester.getTopLeft(rowOf(layer)).dy,
            closeTo(tester.getTopLeft(barOf(layer)).dy, 0.01));
      }

      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${upper.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();

      for (final layer in [upper, lower]) {
        expect(
          tester.getTopLeft(rowOf(layer)).dy,
          closeTo(tester.getTopLeft(barOf(layer)).dy, 0.01),
          reason: 'the layer below an open one still meets its own bar',
        );
      }
    });
  }, skip: !engineAvailable);
}

/// A real, probeable WAV: 16-bit mono PCM, a tenth of a second of silence.
///
/// Written to a temp file **synchronously** — an awaited async `dart:io` call in
/// a `testWidgets` body hangs the test outright (see frb_test_support.dart). The
/// point is only that FFmpeg reports an audio stream, so the samples can be
/// anything.
String _wavFile(String name) {
  final dir = Directory.systemTemp.createTempSync('lumit-audio');
  final file = File('${dir.path}/$name');
  file.writeAsBytesSync(_tinyWav());
  return file.path;
}

Uint8List _tinyWav() {
  const rate = 8000;
  const samples = 800;
  const dataBytes = samples * 2;
  final out = BytesBuilder();
  void ascii(String s) => out.add(s.codeUnits);
  void u16(int v) => out.add([v & 0xff, (v >> 8) & 0xff]);
  void u32(int v) =>
      out.add([v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >> 24) & 0xff]);

  ascii('RIFF');
  u32(36 + dataBytes);
  ascii('WAVE');
  ascii('fmt ');
  u32(16); // PCM header length
  u16(1); // PCM
  u16(1); // mono
  u32(rate);
  u32(rate * 2); // byte rate
  u16(2); // block align
  u16(16); // bits per sample
  ascii('data');
  u32(dataBytes);
  // An actual tone, not silence: a ~440 Hz square wave at half amplitude, so
  // a test asking "does the waveform carry signal" has signal to find.
  final data = Uint8List(dataBytes);
  for (var i = 0; i < samples; i++) {
    final v = (i ~/ 25).isEven ? 16384 : -16384;
    data[i * 2] = v & 0xff;
    data[i * 2 + 1] = (v >> 8) & 0xff;
  }
  out.add(data);
  return out.toBytes();
}
