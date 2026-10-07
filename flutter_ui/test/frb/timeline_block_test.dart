// The block feel (docs/impl/timeline-interaction.md §4.1–4.3, TI-2).
//
// A block of keyframes is two or more selected keys: a box, a stretch handle
// at each end and a badge. Every sentence of the note's §4.3 is a claim here —
// the keys travel with the stretch while it runs, the dragged end lands on the
// shared snap targets, `Escape` abandons any drag in flight and writes
// nothing, a click on a handle falls through to the key it stands over, and
// the readouts a gesture summons are gone the moment it ends (P1).
//
// Against the real engine like every other frb panel test: a stretch that does
// not reach the document is not a stretch.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart'
    show TimelineAxis;
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('The block tools (§4.3)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
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

    /// A solid whose opacity is keyed at [frames], each key's value its own
    /// frame number so a test can tell one from another.
    LayerReference keyedLayer(dynamic p,
        {List<int> frames = const [300, 1500]}) {
      final comp = p.comp as CompositionReference;
      final layer = comp.addSolidLayer();
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
      (p.uiState as LumitUiState).model.refresh();
      return layer;
    }

    String opacityPath(LayerReference layer) =>
        '${layer.internallayerId}/transform/opacity';

    Key laneKeyOf(LayerReference layer) =>
        ValueKey<String>('tl-keys-${opacityPath(layer)}');

    List<BridgeKeyframe> opacityKeys(LayerReference layer) =>
        (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;

    /// Where the lane **draws** its diamonds this frame — the painter's own
    /// answer, which is what a person watching a stretch actually sees.
    List<double> drawnFrames(WidgetTester tester, Key laneKey) {
      final paint = find.descendant(
        of: find.byKey(laneKey),
        matching: find.byType(CustomPaint),
      );
      return ((tester.widget<CustomPaint>(paint.first).painter as dynamic)
              .frames as List)
          .cast<double>();
    }

    Set<int> selectedOn(WidgetTester tester, Key laneKey) {
      final paint = find.descendant(
        of: find.byKey(laneKey),
        matching: find.byType(CustomPaint),
      );
      return ((tester.widget<CustomPaint>(paint.first).painter as dynamic)
              .selected as Set)
          .cast<int>();
    }

    Future<void> openTransform(
        WidgetTester tester, LayerReference layer) async {
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Transform'));
      await tester.pumpAndSettle();
    }

    /// Take the whole property in hand: clicking a property's name selects it
    /// and all of its keys, which is the shortest way to a block.
    Future<void> selectTheBlock(WidgetTester tester) async {
      await tester.tap(find.text('Opacity'));
      await tester.pumpAndSettle();
    }

    /// What one frame is worth in pixels — measured, because the axis is as
    /// wide as the panel leaves it.
    double perFrameOf(WidgetTester tester, dynamic p, Key laneKey) =>
        (tester.getRect(find.byKey(laneKey)).width - TimelineAxis.pad * 2) /
        (p.comp as CompositionReference).durationFrames();

    /// Press a gesture into motion without letting go: two moves with a pump
    /// between, so the arena's slop is passed and the drag is a drag.
    Future<TestGesture> dragging(WidgetTester tester, Offset from, double dx,
        {PointerDeviceKind kind = PointerDeviceKind.mouse}) async {
      final gesture = await tester.startGesture(from, kind: kind);
      await tester.pump(const Duration(milliseconds: 60));
      await gesture.moveBy(Offset(dx / 2, 0));
      await tester.pump();
      await gesture.moveBy(Offset(dx / 2, 0));
      await tester.pump();
      return gesture;
    }

    // ---------------------------------------------------------------------
    // §4.3 — the keys travel with the stretch.
    // ---------------------------------------------------------------------

    // ---------------------------------------------------------------------
    // §4.3, §4.5 — the stretched end snaps to the shared targets.
    // ---------------------------------------------------------------------

    // ---------------------------------------------------------------------
    // P3 — Escape reverts any drag in flight, and writes nothing.
    // ---------------------------------------------------------------------

    testWidgets('Escape abandons a bar drag', (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);

      final was = p.comp.frameAtTime(time: layer.getSpan().inPoint);
      final bar =
          find.byKey(ValueKey<String>('tl-bar-body-${layer.internallayerId}'));
      final rested = tester.getRect(bar);
      final gesture = await dragging(tester, tester.getCenter(bar), 120);
      expect(tester.getRect(bar).left, greaterThan(rested.left + 8),
          reason: 'the bar was travelling before Escape was pressed');

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      await gesture.moveBy(const Offset(60, 0));
      await tester.pump();
      expect(tester.getRect(bar).left, rested.left,
          reason: 'Escape put the bar back where the drag found it');

      await gesture.up();
      await tester.pumpAndSettle();

      expect(p.comp.frameAtTime(time: layer.getSpan().inPoint), was,
          reason: 'the bar went back and the release wrote nothing');
    });

    // ---------------------------------------------------------------------
    // §2.1 — a handle's click falls through to the key beneath it.
    // ---------------------------------------------------------------------

    // ---------------------------------------------------------------------
    // 6.24 — a drag on one of several selected keys moves them all.
    // ---------------------------------------------------------------------

    /// The lane had one rule for one key and no rule at all for several: a
    /// diamond dragged out of a selection of three moved alone and quietly
    /// narrowed the catch to itself, which is not what any other surface in the
    /// panel does with a multiple selection. The graph has always applied the
    /// travel of the key in hand to the whole of it (`_editsFor`).
    testWidgets('dragging one of several selected keys carries them all',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p, frames: [300, 900, 1500]);
      await mount(tester, p);
      await openTransform(tester, layer);
      await selectTheBlock(tester);

      final lane = laneKeyOf(layer);
      expect(selectedOn(tester, lane), {0, 1, 2});
      final perFrame = perFrameOf(tester, p, lane);
      final key =
          find.byKey(ValueKey<String>('tl-key-${opacityPath(layer)}#1'));
      final gesture =
          await dragging(tester, tester.getCenter(key), perFrame * 200);

      final live = drawnFrames(tester, lane);
      expect(live, [500.0, 1100.0, 1700.0],
          reason: 'every held key travelled the same distance, live');
      expect(selectedOn(tester, lane), {0, 1, 2},
          reason: 'taking hold of a selected key keeps the selection');

      await gesture.up();
      await tester.pumpAndSettle();
      expect([
        for (final k in opacityKeys(layer)) p.comp.frameAtTime(time: k.time)
      ], [
        500,
        1100,
        1700
      ]);
    });

    testWidgets('the whole drag is one undo step', (tester) async {
      final p = withComp();
      final layer = keyedLayer(p, frames: [300, 900, 1500]);
      await mount(tester, p);
      await openTransform(tester, layer);
      await selectTheBlock(tester);

      final perFrame = perFrameOf(tester, p, laneKeyOf(layer));
      final key =
          find.byKey(ValueKey<String>('tl-key-${opacityPath(layer)}#0'));
      final gesture =
          await dragging(tester, tester.getCenter(key), perFrame * 200);
      await gesture.up();
      await tester.pumpAndSettle();
      expect(p.comp.frameAtTime(time: opacityKeys(layer).first.time), 500);

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      expect([
        for (final k in opacityKeys(layer)) p.comp.frameAtTime(time: k.time)
      ], [
        300,
        900,
        1500
      ], reason: 'one Ctrl-Z puts the whole drag back');
    });

    // ---------------------------------------------------------------------
    // 6.6 — Delete takes the selected lane keys.
    // ---------------------------------------------------------------------

    /// The lane key selection selected and eased and did nothing else: `Delete`
    /// fell straight past it to the shell, which deleted the *layer* the keys
    /// sat on. The panel claims the key first now in the ladder, above the
    /// mask rung and below nothing.
    testWidgets('Delete removes the selected lane keys, not the layer',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p, frames: [300, 900, 1500]);
      await mount(tester, p);
      await openTransform(tester, layer);

      await tester
          .tap(find.byKey(ValueKey<String>('tl-key-${opacityPath(layer)}#1')));
      await tester.pumpAndSettle();
      expect(selectedOn(tester, laneKeyOf(layer)), {1});

      p.uiState.activePane.value = Panel.timeline.pane();
      expect(p.uiState.deleteClaim?.call(), isTrue,
          reason: 'the panel claims Delete while keys are in hand');
      p.uiState.model.refresh();
      await tester.pumpAndSettle();

      expect([
        for (final k in opacityKeys(layer)) p.comp.frameAtTime(time: k.time)
      ], [
        300,
        1500
      ]);
      expect(p.uiState.model.layers.length, 1,
          reason: 'the layer the keys sat on is untouched');
    });

  }, skip: !engineAvailable);
}
