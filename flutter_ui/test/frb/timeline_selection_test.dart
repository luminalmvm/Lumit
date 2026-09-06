// The Timeline's selection model (docs/impl/timeline-interaction.md §2).
//
// One model for Layers mode's lanes, Keys mode and the graph, and every
// sentence of §2 is a claim here: a marquee starts from any ground in either
// mode, `Shift` and `Ctrl` make it additive, a property's name is its row's
// "select all", a lane key carries the graph key's menu, and `Ctrl`+click on a
// keyed row plants a key.
//
// Written against the real engine like every other frb panel test: a selection
// that does not reach the document is not a selection.

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/easing_curve.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Timeline selection', () {
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

    /// A solid whose [prop] is keyed at [frames], each key's value its own
    /// frame number so a test can tell one from another.
    LayerReference keyedLayer(
      dynamic p, {
      List<int> frames = const [300, 1500],
      BridgeTransformProp prop = BridgeTransformProp.opacity,
    }) {
      final comp = p.comp as CompositionReference;
      final layer = comp.addSolidLayer();
      layer.setTransform(
        prop: prop,
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

    /// Which of a lane's diamonds are drawn selected — the painter's own
    /// answer, which is the one the block box, the badge and F9 all read.
    Set<int> selectedOn(WidgetTester tester, Key laneKey) {
      final paint = find.descendant(
        of: find.byKey(laneKey),
        matching: find.byType(CustomPaint),
      );
      return ((tester.widget<CustomPaint>(paint.first).painter as dynamic)
              .selected as Set<int>)
          .cast<int>();
    }

    /// Twirl a layer open in Layers mode and open its Transform group, which
    /// is where the Opacity lane appears.
    Future<void> openTransform(
        WidgetTester tester, LayerReference layer) async {
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Transform'));
      await tester.pumpAndSettle();
    }

    /// Drag a box from [from] to [to], both in global coordinates.
    ///
    /// Moved in two steps with a pump between: the first move is what takes the
    /// gesture out of the arena's slop, and a single jump can be read as a
    /// fling rather than a drag.
    Future<void> boxFrom(WidgetTester tester, Offset from, Offset to) async {
      final gesture = await tester.startGesture(from);
      await tester.pump(const Duration(milliseconds: 100));
      await gesture.moveTo(Offset(
          from.dx + (to.dx - from.dx) / 4, from.dy + (to.dy - from.dy) / 4));
      await tester.pump();
      await gesture.moveTo(to);
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
    }

    /// Hold [key] down for the body of [run] — a modifier held across a whole
    /// gesture, which is what "held when the drag starts" needs.
    Future<void> holding(
      WidgetTester tester,
      LogicalKeyboardKey key,
      Future<void> Function() run,
    ) async {
      await tester.sendKeyDownEvent(key);
      try {
        await run();
      } finally {
        await tester.sendKeyUpEvent(key);
      }
    }

    List<BridgeKeyframe> opacityKeys(LayerReference layer) =>
        (layer.getTransform().opacity as BridgeScalar_Keyframed).field0;

    // ---------------------------------------------------------------------
    // §2.1 — where a marquee may start, and what a modifier does to it.
    // ---------------------------------------------------------------------

    /// **A marquee can start on a layer's own row, beside its bar.** The row is
    /// a statement — the bar and its summary diamonds — and everything on it
    /// that is not the bar itself is ground.
    testWidgets('a marquee starts on a layer\'s row beside its bar',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);

      final bar = tester.getRect(
          find.byKey(ValueKey<String>('tl-bar-body-${layer.internallayerId}')));
      final lane = tester.getRect(find.byKey(laneKeyOf(layer)));
      // Left of the bar: the axis's own padding, which is the layer row's
      // ground.
      expect(bar.left, greaterThan(lane.left + 2),
          reason: 'there is ground beside the bar to start on');

      await boxFrom(tester, Offset(lane.left + 1, bar.top + 2),
          Offset(lane.right - 1, lane.bottom - 1));
      expect(selectedOn(tester, laneKeyOf(layer)), hasLength(2),
          reason: 'the box began on the bar row and still caught the keys');
    });

    /// **And below the last layer.** The ground under the stack is ground.
    testWidgets('a marquee starts below the last layer', (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);

      final lane = tester.getRect(find.byKey(laneKeyOf(layer)));
      final area =
          tester.getRect(find.byKey(const ValueKey('tl-lane-marquee')));
      expect(area.bottom, greaterThan(lane.bottom + 20),
          reason: 'there is empty ground below the stack');

      await boxFrom(tester, Offset(lane.left + 1, lane.bottom + 20),
          Offset(lane.right - 1, lane.top + 1));
      expect(selectedOn(tester, laneKeyOf(layer)), hasLength(2),
          reason: 'a box drawn upwards from the empty ground caught them');
    });

    /// **Plain marquee replaces; `Shift` held at the drag's start adds.**
    testWidgets('a plain marquee replaces the selection', (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);

      final lane = tester.getRect(find.byKey(laneKeyOf(layer)));
      final first =
          find.byKey(ValueKey<String>('tl-key-${opacityPath(layer)}#0'));
      await tester.tap(first);
      await tester.pumpAndSettle();
      expect(selectedOn(tester, laneKeyOf(layer)), {0});

      // A box round the *second* key only.
      final second = tester.getRect(
          find.byKey(ValueKey<String>('tl-key-${opacityPath(layer)}#1')));
      await boxFrom(tester, Offset(second.left - 6, lane.top + 1),
          Offset(second.right + 6, lane.bottom - 1));
      expect(selectedOn(tester, laneKeyOf(layer)), {1},
          reason: 'the plain box replaced what was in hand');
    });

    testWidgets('Shift held when the marquee starts adds to the selection',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);

      final lane = tester.getRect(find.byKey(laneKeyOf(layer)));
      await tester
          .tap(find.byKey(ValueKey<String>('tl-key-${opacityPath(layer)}#0')));
      await tester.pumpAndSettle();
      expect(selectedOn(tester, laneKeyOf(layer)), {0});

      final second = tester.getRect(
          find.byKey(ValueKey<String>('tl-key-${opacityPath(layer)}#1')));
      await holding(
        tester,
        LogicalKeyboardKey.shiftLeft,
        () => boxFrom(tester, Offset(second.left - 6, lane.top + 1),
            Offset(second.right + 6, lane.bottom - 1)),
      );
      expect(selectedOn(tester, laneKeyOf(layer)), {0, 1},
          reason: 'the box added to the standing selection');
    });

    // ---------------------------------------------------------------------
    // §2.1 — clicking keys.
    // ---------------------------------------------------------------------

    /// Clicking a key selects exactly that key and deselects the rest; `Shift`
    /// and `Ctrl` toggle one in and out without touching the others.
    testWidgets('clicking a key replaces, Ctrl-clicking toggles',
        (tester) async {
      final p = withComp();
      // Three, and the gestures land on the **middle** one: the block's two
      // stretch handles stand over the outer keys of any selection, and a
      // handle is a control that takes the drag (§2.1's own carve-out).
      final layer = keyedLayer(p, frames: [300, 900, 1500]);
      await mount(tester, p);
      await openTransform(tester, layer);
      final lane = laneKeyOf(layer);
      Finder key(int i) =>
          find.byKey(ValueKey<String>('tl-key-${opacityPath(layer)}#$i'));

      await tester.tap(key(0));
      await tester.pumpAndSettle();
      expect(selectedOn(tester, lane), {0});

      await tester.tap(key(1));
      await tester.pumpAndSettle();
      expect(selectedOn(tester, lane), {1}, reason: 'a plain click replaces');

      await holding(tester, LogicalKeyboardKey.controlLeft, () async {
        await tester.tap(key(2));
        await tester.pumpAndSettle();
      });
      expect(selectedOn(tester, lane), {1, 2}, reason: 'Ctrl added the other');

      // The whole row, then a plain click on the middle key: exactly that key
      // is left, and the rest are let go.
      await tester.tap(find.text('Opacity'));
      await tester.pumpAndSettle();
      expect(selectedOn(tester, lane), {0, 1, 2});
      await tester.tap(key(1));
      await tester.pumpAndSettle();
      expect(selectedOn(tester, lane), {1},
          reason: 'a click on a key selects exactly that key');

      await holding(tester, LogicalKeyboardKey.shiftLeft, () async {
        await tester.tap(key(1));
        await tester.pumpAndSettle();
      });
      expect(selectedOn(tester, lane), isEmpty,
          reason: 'Shift toggled the same key back out');
    });

    // ---------------------------------------------------------------------
    // §2.1 — a property's name is its row's "select all".
    // ---------------------------------------------------------------------

    /// **Clicking a property's name selects the property and all of its keys.**
    /// Two or more of them are the block, so the box and its badge appear from
    /// a name exactly as they do from a marquee.
    testWidgets('a property name selects its keys, and they are the block',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);

      expect(selectedOn(tester, laneKeyOf(layer)), isEmpty);
      await tester.tap(find.text('Opacity'));
      await tester.pumpAndSettle();

      expect(selectedOn(tester, laneKeyOf(layer)), {0, 1},
          reason: 'the name is the row\'s select-all');
      expect(find.byKey(const ValueKey('tl-block-box')), findsOneWidget,
          reason: 'two selected keys are the block, however they were picked');
    });

    /// `Ctrl`-click toggles the property's keys in and out of the standing key
    /// selection.
    testWidgets('Ctrl-clicking a property name toggles its keys',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);
      final row = find.text('Opacity');

      await tester.tap(row);
      await tester.pumpAndSettle();
      expect(selectedOn(tester, laneKeyOf(layer)), {0, 1});

      await holding(tester, LogicalKeyboardKey.controlLeft, () async {
        await tester.tap(row);
        await tester.pumpAndSettle();
      });
      expect(selectedOn(tester, laneKeyOf(layer)), isEmpty,
          reason: 'Ctrl took the row and its keys back out');
    });

    /// `Shift`-click extends the visible run of rows **and takes their keys
    /// with it**.
    testWidgets('Shift-clicking a property name takes the run\'s keys',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      // A second keyed property on the same layer, so the run has two rows.
      layer.setTransform(
        prop: BridgeTransformProp.rotation,
        value: BridgeScalar.keyframed([
          for (final f in [300, 900, 1500])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      p.uiState.model.refresh();
      await mount(tester, p);
      await openTransform(tester, layer);

      final rotationPath = '${layer.internallayerId}/transform/rotation';
      await tester.tap(find.text('Opacity'));
      await tester.pumpAndSettle();
      await holding(tester, LogicalKeyboardKey.shiftLeft, () async {
        await tester.tap(find.text('Rotation'));
        await tester.pumpAndSettle();
      });

      expect(selectedOn(tester, laneKeyOf(layer)), {0, 1},
          reason: 'the row the run started on kept its keys');
      expect(selectedOn(tester, ValueKey<String>('tl-keys-$rotationPath')),
          {0, 1, 2},
          reason: 'and the row it reached brought its own');
    });

    /// **The stopwatch stays what it is** — the animate toggle, never a
    /// selector. Nor is the value well.
    testWidgets('the stopwatch does not select the row\'s keys',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);

      await tester
          .tap(find.byKey(const ValueKey('kf-stopwatch-tl-tf-opacity')));
      await tester.pumpAndSettle();
      // The stopwatch stopped the animation, so there is no lane left to read
      // — which is itself the proof it did the one thing it does.
      expect(find.byKey(laneKeyOf(layer)), findsNothing,
          reason: 'the stopwatch animates and un-animates, and nothing else');
    });

    /// **Clicking a layer's row selects the layer** and does not gather the
    /// layer's keys: a layer's name is not a select-all for everything under
    /// it.
    testWidgets('clicking a layer\'s row does not select its keys',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);

      // The layer's own bar is the row's handle on the lane side.
      await tester.tap(
          find.byKey(ValueKey<String>('tl-bar-body-${layer.internallayerId}')));
      await tester.pumpAndSettle();
      expect(p.uiState.selectedLayer.value?.internallayerId,
          layer.internallayerId);
      expect(selectedOn(tester, laneKeyOf(layer)), isEmpty,
          reason: 'picking the layer picked no keys');
    });

    // ---------------------------------------------------------------------
    // §2.2 — letting go.
    // ---------------------------------------------------------------------

    /// A plain click on any ground deselects everything.
    testWidgets('a plain click on lane ground lets everything go',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);

      await tester.tap(find.text('Opacity'));
      await tester.pumpAndSettle();
      expect(selectedOn(tester, laneKeyOf(layer)), {0, 1});

      final lane = tester.getRect(find.byKey(laneKeyOf(layer)));
      await tester.tapAt(Offset(lane.left + 2, lane.bottom + 40));
      await tester.pumpAndSettle();
      expect(selectedOn(tester, laneKeyOf(layer)), isEmpty);
      expect(p.uiState.selectedLayer.value, isNull);
    });

    /// Closing a fold drops what was inside it, keys included.
    testWidgets('shutting a fold drops the keys it held', (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);

      await tester.tap(find.text('Opacity'));
      await tester.pumpAndSettle();
      expect(selectedOn(tester, laneKeyOf(layer)), {0, 1});

      final twirl =
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}'));
      await tester.tap(twirl);
      await tester.pumpAndSettle();
      expect(find.byKey(laneKeyOf(layer)), findsNothing);

      // The twirl alone brings it back: the Transform group inside is still
      // remembered open, which is what a fold's memory is for.
      await tester.tap(twirl);
      await tester.pumpAndSettle();
      expect(selectedOn(tester, laneKeyOf(layer)), isEmpty,
          reason: 'the selection did not come back with the fold');
    });

    // ---------------------------------------------------------------------
    // §2.1 — the lane key's own menu, and planting one.
    // ---------------------------------------------------------------------

    /// **Right-clicking a lane key opens the graph key's menu** — Linear /
    /// Easy ease / Hold / Ease… / Delete key — and a right-click on an
    /// unselected key selects it first, so the menu acts on what was clicked.
    testWidgets('a lane key\'s menu holds the key it was opened on',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);

      final second = tester.getRect(
          find.byKey(ValueKey<String>('tl-key-${opacityPath(layer)}#1')));
      final gesture = await tester.startGesture(second.center,
          kind: PointerDeviceKind.mouse, buttons: kSecondaryMouseButton);
      await gesture.up();
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('tl-key-menu-linear')), findsOneWidget);
      expect(find.byKey(const ValueKey('tl-key-menu-ease')), findsOneWidget);
      expect(find.byKey(const ValueKey('tl-key-menu-shape')), findsOneWidget);
      expect(find.byKey(const ValueKey('tl-key-menu-delete')), findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('tl-key-menu-hold')));
      await tester.pumpAndSettle();

      final keys = opacityKeys(layer);
      expect(keys[1].interpOut, isA<BridgeSideInterp_Hold>(),
          reason:
              'the right-click selected the key it landed on, then held it');
      expect(keys[0].interpOut, isA<BridgeSideInterp_Linear>(),
          reason: 'and left the key it did not land on alone');
    });

    /// Delete key removes the selection — the whole of it, in one undo step.
    testWidgets('the menu\'s Delete key removes the selected keys',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p, frames: [300, 900, 1500]);
      await mount(tester, p);
      await openTransform(tester, layer);

      await tester.tap(find.text('Opacity'));
      await tester.pumpAndSettle();

      // The middle key, which no stretch handle stands over.
      final middle = tester.getRect(
          find.byKey(ValueKey<String>('tl-key-${opacityPath(layer)}#1')));
      final gesture = await tester.startGesture(middle.center,
          kind: PointerDeviceKind.mouse, buttons: kSecondaryMouseButton);
      await gesture.up();
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('tl-key-menu-delete')));
      await tester.pumpAndSettle();

      expect(layer.getTransform().opacity, isA<BridgeScalar_Static>(),
          reason: 'the whole selection went, not only the key clicked');
      p.state.project!.undo();
      expect(opacityKeys(layer), hasLength(3),
          reason: 'one gesture, one undo step');
    });

    /// **`Ctrl`+click on empty lane space of a keyed row plants a key** at that
    /// time on that property (docs/07 §4.3). The new key takes the value the
    /// curve already reads there, so planting one moves nothing.
    testWidgets('Ctrl+click plants a key on a keyed lane', (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);

      expect(opacityKeys(layer), hasLength(2));
      final lane = tester.getRect(find.byKey(laneKeyOf(layer)));
      await holding(tester, LogicalKeyboardKey.controlLeft, () async {
        await tester.tapAt(Offset(lane.center.dx, lane.center.dy));
        await tester.pumpAndSettle();
      });

      final after = opacityKeys(layer);
      expect(after, hasLength(3),
          reason: 'a key was planted where it was clicked');
      final frames = [for (final k in after) p.comp.frameAtTime(time: k.time)];
      expect(frames.first, 300);
      expect(frames.last, 1500);
      expect(frames[1], greaterThan(300));
      expect(frames[1], lessThan(1500));
    });

    /// A plain click on that same ground still lets go, rather than planting.
    testWidgets('a plain click on a keyed lane plants nothing', (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      await mount(tester, p);
      await openTransform(tester, layer);

      final lane = tester.getRect(find.byKey(laneKeyOf(layer)));
      await tester.tapAt(lane.center);
      await tester.pumpAndSettle();
      expect(opacityKeys(layer), hasLength(2));
    });

    // ---------------------------------------------------------------------
    // §4.4 — the catch walk steps by each layer's real block height.
    // ---------------------------------------------------------------------

    /// **The marquee catches the right keys below an open Sequence view.** The
    /// walk stepped by `rowHeight` per row and ignored `sequenceExtra`, so
    /// everything below a view sat adrift by the view's own height: a box
    /// drawn round a row caught nothing, and the block box drew itself over
    /// the wrong one.
    testWidgets('a marquee catches keys below an open Sequence view',
        (tester) async {
      final p = withComp();
      final layer = keyedLayer(p);
      final seq = p.comp.addSequenceLayer();
      p.uiState.model.refresh();
      await mount(tester, p);

      // Double-clicking a Sequence layer's bar opens its view. Retried: the
      // first tap selects and can rebuild the row under the second.
      final bar =
          find.byKey(ValueKey<String>('tl-bar-body-${seq.internallayerId}'));
      final room =
          find.byKey(ValueKey<String>('tl-seq-room-${seq.internallayerId}'));
      for (var attempt = 0; attempt < 3; attempt++) {
        await tester.tap(bar);
        await tester.pump(const Duration(milliseconds: 30));
        await tester.tap(bar);
        await tester.pumpAndSettle();
        await settleFrb(tester, until: () => room.evaluate().isNotEmpty);
        if (room.evaluate().isNotEmpty) break;
        await tester.pump(const Duration(milliseconds: 400));
      }
      expect(room, findsOneWidget, reason: 'the Sequence view is open');

      await openTransform(tester, layer);
      final lane = tester.getRect(find.byKey(laneKeyOf(layer)));
      await boxFrom(tester, Offset(lane.left + 1, lane.top + 1),
          Offset(lane.right - 1, lane.bottom - 1));
      expect(selectedOn(tester, laneKeyOf(layer)), hasLength(2),
          reason: 'the walk stepped over the view\'s room, not through it');
    });

    // ---------------------------------------------------------------------
    // The easing claim: what a preset tile presses.
    // ---------------------------------------------------------------------

    /// **An applied ease writes the drawn tangents, and however many layers
    /// the selection spans it is one undo step** (the rule is that a
    /// multi-selection edit is one edit). The write is one op per layer, so
    /// without the undo group a two-layer apply was two steps — this fails
    /// without the `asOneUndoStep` round `_applyEasing`.
    testWidgets('an applied ease writes its tangents as one undo step',
        (tester) async {
      final p = withComp();
      final a = keyedLayer(p);
      final b = keyedLayer(p);
      await mount(tester, p);
      // By group-row key rather than [openTransform]'s text tap: with two
      // layers twirled open there are two rows called Transform.
      for (final layer in [a, b]) {
        await tester.tap(find
            .byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(
            ValueKey<String>('tl-group-${layer.internallayerId}/transform')));
        await tester.pumpAndSettle();
      }

      // One box round all four keys, across both lanes.
      final laneA = tester.getRect(find.byKey(laneKeyOf(a)));
      final laneB = tester.getRect(find.byKey(laneKeyOf(b)));
      await boxFrom(
          tester,
          Offset(laneA.left + 1, math.min(laneA.top, laneB.top) + 1),
          Offset(laneA.right - 1, math.max(laneA.bottom, laneB.bottom) - 1));
      expect(selectedOn(tester, laneKeyOf(a)), hasLength(2));
      expect(selectedOn(tester, laneKeyOf(b)), hasLength(2));

      // The claim the Timeline publishes while it can take a shape — the very
      // callback a preset tile presses.
      final apply = p.uiState.easingApply.value;
      expect(apply, isNotNull, reason: 'lane view publishes the claim');
      apply!(easingPresets.firstWhere((e) => e.id == 'sineIn').curve);
      await tester.pumpAndSettle();

      for (final layer in [a, b]) {
        final keys = opacityKeys(layer);
        // Sine in (0.12, 0, 0.39, 0): flat out of the first key over 12% of
        // the span, and into the second across the remaining 61%.
        final out = keys[0].interpOut;
        expect(out, isA<BridgeSideInterp_Bezier>());
        expect((out as BridgeSideInterp_Bezier).field0.speed,
            closeTo(0, 1e-9));
        expect(out.field0.influence, closeTo(0.12, 1e-9));
        final inTo = keys[1].interpIn;
        expect(inTo, isA<BridgeSideInterp_Bezier>());
        expect((inTo as BridgeSideInterp_Bezier).field0.influence,
            closeTo(0.61, 1e-9));
      }

      p.state.project!.undo();
      expect(opacityKeys(a)[0].interpOut, isA<BridgeSideInterp_Linear>(),
          reason: 'one undo takes the whole apply back');
      expect(opacityKeys(b)[0].interpOut, isA<BridgeSideInterp_Linear>(),
          reason: 'both layers in the one step — one press, one edit');
    });
  }, skip: !engineAvailable);
}
