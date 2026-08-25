// The graph editor against the real engine: the AE-style full-height pane
// (docs/07 §5) — selected properties as curves, key drags, easing, the F9
// family, the speed lens, and keyframe copy/paste.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_editor_frb.dart';
import 'package:uuid/uuid.dart';
import 'package:lumit_flutter/panels/graph_maths.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/theme/theme.dart';

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
      // Selection lives in the outline: twirl the layer, open Transform,
      // click the property's name (docs/07 §4.3).
      final layer = (p as dynamic).layer as LayerReference;
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();
      await tester.tap(find.text('Opacity'));
      await tester.pump();
    }

    testWidgets('with nothing selected the pane says how to start',
        (tester) async {
      final p = withLayer();
      await mountGraph(tester, p, selectOpacity: false);
      expect(find.textContaining('Select a property'), findsOneWidget);
    });

    testWidgets(
        'selecting a property shows its keys; a static one is still a line',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 40, 90]);
      await mountGraph(tester, p);

      for (var i = 0; i < 3; i++) {
        expect(find.byKey(ValueKey<String>(opacityKey(p.layer, i))),
            findsOneWidget);
      }
      expect(find.byKey(const ValueKey('graph-marquee')), findsOneWidget);
      expect(find.byKey(const ValueKey('tl-ruler')), findsOneWidget,
          reason: 'the graph keeps the time ruler');

      // A static property joins as a flat line: no keys, no complaints.
      await tester.tap(find.text('Position'));
      await tester.pump();
      expect(tester.takeException(), isNull);
    });

    /// **The graph follows a value drag in the outline, while the pointer is
    /// still down** (K-333/K-334). The row publishes each tick (`rowValueDrag`)
    /// and the pane draws the key through it; the release commits and the
    /// publication clears. Fails if any link of that chain breaks — the wiring
    /// this bug shipped without twice.
    testWidgets('the graph key follows an outline value drag mid-gesture',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer);
      await mountGraph(tester, p);

      final glyph = find.byKey(ValueKey<String>(opacityKey(p.layer, 0)));
      final before = tester.getCenter(glyph);

      // Grab the outline row's value field and drag, without letting go.
      final field = find.byKey(const ValueKey<String>('tl-tf-opacity'));
      final gesture = await tester.startGesture(tester.getCenter(field));
      await tester.pump();
      await gesture.moveBy(const Offset(30, 0));
      await tester.pump();
      await gesture.moveBy(const Offset(30, 0));
      await tester.pump();

      expect(rowValueDrag.value, isNotNull,
          reason: 'the row publishes the provisional value each tick');
      final during = tester.getCenter(glyph);
      expect(during.dy, lessThan(before.dy),
          reason: 'the key draws at the dragged value while the pointer '
              'is still down');

      await gesture.up();
      await tester.pump();
      expect(rowValueDrag.value, isNull,
          reason: 'the release commits and the publication clears');
      expect(opacityKeys(p.layer).first.value, greaterThan(0),
          reason: 'and the document now holds what the drag showed');
    });

    /// The same chain with the playhead **between** keys (K-334): the drag
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
      // insert a duplicate key instead of replacing (K-336). Frame 50 is
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
      // key drawn at the next key's place (K-336).
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

    /// An **effect parameter's** drag feeds the same chain (K-334) — the wiring
    /// the transform rows got first, which is exactly how "still not fixed"
    /// shipped: the reporter was dragging an effect value.
    testWidgets('the graph follows an effect value drag mid-gesture',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'blur');
      // Animate the radius so the channel has a curve.
      final staged = p.layer.getEffects();
      final id = staged.single.id();
      for (final instance in staged) {
        instance.setValue(
          id: 'radius',
          value: BridgeEffectValue.float(BridgeScalar.keyframed([
            for (final (f, v) in [(0, 0.0), (100, 50.0)])
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
      p.uiState.model.refresh();
      await mountGraph(tester, p, selectOpacity: false);

      // Select the Radius property the outline way.
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${p.layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Effects'));
      await tester.pump();
      await tester.tap(find.text('Gaussian blur'));
      await tester.pump();
      await tester.tap(find.text('Radius'));
      await tester.pump();

      final glyphKey =
          'graph-key-${p.layer.internallayerId}/effects/$id/radius#0';
      final glyph = find.byKey(ValueKey<String>(glyphKey));
      expect(glyph, findsOneWidget, reason: 'the radius curve is on screen');
      final before = tester.getCenter(glyph);

      final field = find.byKey(ValueKey<String>('fx-float-$id-radius'));
      final gesture = await tester.startGesture(tester.getCenter(field));
      await tester.pump();
      await gesture.moveBy(const Offset(30, 0));
      await tester.pump();
      await gesture.moveBy(const Offset(30, 0));
      await tester.pump();

      expect(rowValueDrag.value, isNotNull,
          reason: 'the effect row publishes like a transform row');
      expect(tester.getCenter(glyph).dy, lessThan(before.dy),
          reason: 'the radius key draws at the dragged value mid-gesture');

      await gesture.up();
      await tester.pump();
      expect(rowValueDrag.value, isNull);
    });

    /// The screenshot bug (K-336): drag the **Retime** readout on a frame with
    /// no key, and the diamonds floated off the curve — every glyph past the
    /// insertion drew with one key's x and another's y, because x read the
    /// document's keys while y read the preview's longer list. The first tick
    /// now plants a key (so the preview replaces, never inserts) and both
    /// coordinates read one list either way.
    testWidgets(
        'a Retime drag on a keyless frame keeps the diamonds on the curve',
        (tester) async {
      final p = withLayer();
      p.layer.toggleRetimeProperty();
      p.uiState.scrubTo(31);
      p.uiState.model.refresh();
      await mountGraph(tester, p, selectOpacity: false);

      final id = p.layer.internallayerId;
      await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('tl-retime-name')));
      await tester.pump();

      String glyph(int i) => 'graph-key-$id/retime#$i';
      expect(find.byKey(ValueKey<String>(glyph(0))), findsOneWidget,
          reason: 'the identity map has its first key on screen');
      expect(find.byKey(ValueKey<String>(glyph(1))), findsOneWidget);
      final lastBefore =
          tester.getCenter(find.byKey(ValueKey<String>(glyph(1))));

      final field = find.byKey(const ValueKey('tl-retime-seconds'));
      final gesture = await tester.startGesture(tester.getCenter(field));
      await tester.pump();
      await gesture.moveBy(const Offset(30, 0));
      await tester.pump();
      await gesture.moveBy(const Offset(30, 0));
      await tester.pump();

      expect(rowValueDrag.value, isNotNull,
          reason: 'the Retime row publishes its drag');
      expect(find.byKey(ValueKey<String>(glyph(2))), findsOneWidget,
          reason: 'the first tick planted a key, so three diamonds show');
      // The key that was the last is now index 2 of three; its position must
      // not have moved — with the mixed-list bug it drew at the middle key's x.
      expect(
          tester.getCenter(find.byKey(ValueKey<String>(glyph(2)))), lastBefore,
          reason: 'the keys after the playhead hold still, x and y both');

      await gesture.up();
      await tester.pump();
      expect(rowValueDrag.value, isNull);
      final keys =
          (p.layer.getRetimeProperty() as BridgeScalar_Keyframed).field0;
      expect(keys.length, 3, reason: 'plant plus the dragged write persisted');
    });

    /// One gesture, one op: the key moves in time AND value, and one undo
    /// puts both back.
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

    testWidgets('the key menu eases and deletes; easing shows handles',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer);
      await mountGraph(tester, p);

      await tester.tapAt(
        tester.getCenter(find.byKey(ValueKey<String>(opacityKey(p.layer, 0)))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Easy ease'));
      await tester.pumpAndSettle();

      expect(opacityKeys(p.layer)[0].interpOut, isA<BridgeSideInterp_Bezier>(),
          reason: 'the AE easy-ease constant went to the document');

      // Selecting the eased key shows its out-side tangent handle.
      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 0))));
      await tester.pump();
      expect(
          find.byKey(ValueKey<String>(
              'graph-handle-${p.layer.internallayerId}/transform/opacity@opacity#0-out')),
          findsOneWidget);

      await tester.tapAt(
        tester.getCenter(find.byKey(ValueKey<String>(opacityKey(p.layer, 0)))),
        buttons: kSecondaryButton,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Delete key'));
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer), hasLength(1));
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

      await tester.tap(find.byKey(const ValueKey('graph-interp-hold')));
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer)[0].interpOut, isA<BridgeSideInterp_Hold>());

      await tester.tap(find.byKey(const ValueKey('graph-interp-bezier')));
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer)[0].interpOut, isA<BridgeSideInterp_Bezier>());

      await tester.tap(find.byKey(const ValueKey('graph-interp-linear')));
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer)[0].interpOut, isA<BridgeSideInterp_Linear>());

      await tester.sendKeyEvent(LogicalKeyboardKey.f9);
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer)[0].interpIn, isA<BridgeSideInterp_Bezier>(),
          reason: 'F9 easy-eases the selection');
    });

    /// The shaped ease (K-348): a curve drawn once in the unit box, stamped on
    /// every **span** whose two ends are selected — and only from the value
    /// lens, because the shape is drawn against value travel.
    ///
    /// Driven through the *popup* mode of K-349, because this test mounts the
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
      await tester.tap(find.text('Slow start'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Apply'.toUpperCase()));
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer)[0].interpOut, isA<BridgeSideInterp_Linear>(),
          reason: 'one key on its own has no span to shape');
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();

      // Both ends of the first span selected: that span takes the shape, and
      // the span beyond the selection does not.
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 1))));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();
      await openEditor();
      await tester.tap(find.text('Slow start'));
      await tester.pumpAndSettle();
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
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-lens-speed')));
      await tester.tap(find.byKey(const ValueKey('graph-lens-speed')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('graph-interp-easing')), findsNothing);
    });

    /// Which door the button opens is Settings ▸ Interface ▸ Editing (K-349).
    /// The panel is the default because it outlasts a selection change; the
    /// popup is the deviation, for anyone who would rather not spend a column.
    testWidgets('by default the Easing button docks the panel', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer);
      await mountGraph(tester, p);
      expect(panelVisible(p.uiState.split, Panel.easing), isFalse,
          reason: 'the panel is in no default arrangement');

      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-interp-easing')));
      await tester.tap(find.byKey(const ValueKey('graph-interp-easing')));
      await tester.pumpAndSettle();

      expect(panelVisible(p.uiState.split, Panel.easing), isTrue);
      expect(p.uiState.activePanel.value, Panel.easing,
          reason: 'a panel you just asked for is the one you want to look at');
      expect(find.text('Apply'.toUpperCase()), findsNothing,
          reason: 'nothing floats over the footer in panel mode');
    });

    testWidgets('a second press does not dock it twice', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer);
      await mountGraph(tester, p);

      for (var i = 0; i < 2; i++) {
        await tester
            .ensureVisible(find.byKey(const ValueKey('graph-interp-easing')));
        await tester.tap(find.byKey(const ValueKey('graph-interp-easing')));
        await tester.pumpAndSettle();
      }

      final panels = panelsIn(p.uiState.split);
      expect(panels.where((x) => x == Panel.easing), hasLength(1));
    });

    /// The claim the Easing panel presses (K-349) tracks the lens, so a panel
    /// docked elsewhere in the shell can grey its Apply without ever being told
    /// what is selected.
    testWidgets('the shell claim follows the lens', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer);
      await mountGraph(tester, p);
      expect(p.uiState.easingApply.value, isNotNull);

      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-lens-speed')));
      await tester.tap(find.byKey(const ValueKey('graph-lens-speed')));
      await tester.pumpAndSettle();
      expect(p.uiState.easingApply.value, isNull,
          reason: 'a shape drawn against value travel does not belong here');

      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-lens-value')));
      await tester.tap(find.byKey(const ValueKey('graph-lens-value')));
      await tester.pumpAndSettle();
      expect(p.uiState.easingApply.value, isNotNull);
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

    /// Over and over: the partner stays opposite and stays the same length on
    /// screen, including out at the near-vertical extreme and back again. It
    /// is the *length* that must hold, not the movement — a drag that only
    /// lengthens an already-steep tangent barely turns the line, so the
    /// partner rightly barely stirs.
    testWidgets('the partner keeps its length over repeated drags',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);

      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 1))));
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.f9);
      await tester.pumpAndSettle();

      final base =
          'graph-handle-${p.layer.internallayerId}/transform/opacity@opacity#1';
      final outHandle = find.byKey(ValueKey<String>('$base-out'));
      final inHandle = find.byKey(ValueKey<String>('$base-in'));

      final keyPoint = tester
          .getCenter(find.byKey(ValueKey<String>(opacityKey(p.layer, 1))));
      final length0 = (tester.getCenter(inHandle) - keyPoint).distance;

      // Three drags out toward the vertical, then three back: the partner is
      // the same length at every step, and still opposite at the end.
      for (final step in const [-6.0, -6.0, -6.0, 6.0, 6.0, 6.0]) {
        final gesture = await tester.startGesture(tester.getCenter(outHandle));
        await tester.pump();
        for (var i = 0; i < 5; i++) {
          await gesture.moveBy(Offset(0, step));
          await tester.pump();
        }
        await gesture.up();
        await tester.pumpAndSettle();

        expect((tester.getCenter(inHandle) - keyPoint).distance,
            closeTo(length0, 1),
            reason: 'the partner is still the length it started at');
      }

      final outDir = tester.getCenter(outHandle) - keyPoint;
      final inDir = tester.getCenter(inHandle) - keyPoint;
      expect(
          (outDir.dx * inDir.dy - outDir.dy * inDir.dx).abs() /
              (outDir.distance * inDir.distance),
          lessThan(0.1),
          reason: 'and still in one straight line through the key');
    });

    /// Alt at the start of a drag breaks the pair; the partner then holds
    /// still while the dragged side moves.
    testWidgets('Alt-dragging a handle breaks the pair', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);

      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 1))));
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.f9);
      await tester.pumpAndSettle();

      final base =
          'graph-handle-${p.layer.internallayerId}/transform/opacity@opacity#1';
      final inBefore =
          tester.getCenter(find.byKey(ValueKey<String>('$base-in')));

      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      final gesture = await tester.startGesture(
          tester.getCenter(find.byKey(ValueKey<String>('$base-out'))));
      await tester.pump();
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(-2, -8));
        await tester.pump();
      }
      final inMid = tester.getCenter(find.byKey(ValueKey<String>('$base-in')));
      expect((inMid - inBefore).distance, lessThan(2),
          reason: 'the broken partner did not follow');
      await gesture.up();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.pumpAndSettle();
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

    /// A speed dot drags sideways too: that is how a keyframe moves in time
    /// without leaving the speed graph.
    testWidgets('a speed dot drags the keyframe in time', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);
      await tester
          .ensureVisible(find.byKey(const ValueKey('graph-lens-speed')));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('graph-lens-speed')));
      await tester.pump();

      final before = p.comp.frameAtTime(time: opacityKeys(p.layer)[1].time);
      await _drag(
          tester,
          find.byKey(ValueKey<String>(
              'graph-key-${p.layer.internallayerId}/transform/opacity@opacity#1-out')),
          const Offset(70, 0));

      final after = p.comp.frameAtTime(time: opacityKeys(p.layer)[1].time);
      expect(after, greaterThan(before),
          reason: 'the dot carried its keyframe later in time');
      expect(opacityKeys(p.layer), hasLength(3), reason: 'nothing was lost');
    });

    /// Ctrl+C / Ctrl+V: the in-app clipboard carries full easing, and pasting
    /// lands the earliest key on the playhead.
    testWidgets('copy and paste land keys on the playhead', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 20]);
      await mountGraph(tester, p);

      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 1))));
      await tester.pump();
      // The chord itself is the shell's since K-300 — it asks the claim this
      // panel registers, which is what a shell test drives end to end
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

    /// **A row with no keyframes still has a value, and Copy takes it**
    /// (K-301). With the row selected and no individual key picked, `Ctrl+C`
    /// used to find nothing to copy, give up, and quietly copy the whole layer
    /// instead — so the one thing the user was pointing at was the one thing
    /// that did not travel.
    testWidgets('a static row copies its value, and pasting puts it back',
        (tester) async {
      final p = withLayer();
      p.layer.setTransform(
          prop: BridgeTransformProp.opacity,
          value: const BridgeScalar.static_(40));
      await mountGraph(tester, p);

      expect(p.uiState.copyClaim!(), isTrue,
          reason: 'the selected row is what Copy takes, keys or no keys');

      p.layer.setTransform(
          prop: BridgeTransformProp.opacity,
          value: const BridgeScalar.static_(90));
      p.uiState.model.refresh();
      await tester.pump();

      expect(p.uiState.pasteClaim!(), isTrue);
      await tester.pumpAndSettle();
      expect(p.layer.getTransform().opacity, const BridgeScalar.static_(40),
          reason: 'the copied value came back as a value, not as a keyframe');
    });

    /// Copy and paste belong to the keyframes, not to the graph: a selection
    /// boxed up on a *lane* copies and pastes the same way (K-196).
    testWidgets('copy and paste work from the lane view too', (tester) async {
      final p = withLayer();
      // Spread out, so a marquee can take one key and leave the other.
      animateOpacity(p.comp, p.layer, frames: [600, 1500]);
      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1280, 600),
      ));
      await tester.pump();
      // Lane view — no Graph toggle. Twirl open and box the second key.
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${p.layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.text('Transform'));
      await tester.pump();

      final lane = find.byKey(ValueKey<String>(
          'tl-keys-${p.layer.internallayerId}/transform/opacity'));
      final rect = tester.getRect(lane);
      final start = Offset(rect.left + rect.width * 0.5, rect.top + 1);
      final gesture = await tester.startGesture(start);
      await tester.pump(const Duration(milliseconds: 100));
      final end = rect.bottomRight - const Offset(1, 1);
      for (var i = 1; i <= 8; i++) {
        await gesture.moveTo(start + (end - start) * (i / 8));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect(p.uiState.copyClaim!(), isTrue);
      await tester.pump();
      p.uiState.playheadFrame.value = 90;
      await tester.pump();
      expect(p.uiState.pasteClaim!(), isTrue);
      await tester.pumpAndSettle();

      final frames = opacityKeys(p.layer)
          .map((k) => p.comp.frameAtTime(time: k.time))
          .toList();
      expect(frames, contains(90),
          reason: 'the key boxed on the lane pasted onto the playhead');
    });

    /// Ctrl+click on a second property overlays it: Position contributes one
    /// curve per axis, with its own keys.
    testWidgets('Ctrl+click adds properties; Position graphs both axes',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer);
      p.layer.setTransform(
        prop: BridgeTransformProp.positionX,
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
      await mountGraph(tester, p);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.tap(find.text('Position'));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();

      final id = p.layer.internallayerId;
      expect(
          find.byKey(ValueKey<String>(opacityKey(p.layer, 0))), findsOneWidget,
          reason: 'opacity stayed selected');
      expect(
          find.byKey(ValueKey<String>(
              'graph-key-$id/transform/positionX@positionX#0')),
          findsOneWidget,
          reason: "position x's keys joined the graph");
    });

    /// The pure channel builder: a transform row fans out per axis, an effect
    /// float parameter is one channel, and colours follow selection order.
    testWidgets('graphChannels resolves selection into coloured channels',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer);
      final id = p.layer.internallayerId.toString();
      final ui = p.uiState;
      ui.model.refresh();

      final channels = graphChannels(
        layers: ui.model.layers,
        selected: ['$id/transform/positionX', '$id/transform/opacity'],
      );
      expect(channels.map((c) => c.id).toList(), [
        '$id/transform/positionX@positionX',
        '$id/transform/positionX@positionY',
        '$id/transform/opacity@opacity',
      ]);
      expect(channels.map((c) => c.colourIndex).toList(), [0, 1, 2]);
      expect(channels.last.keys, hasLength(2));
    });

    /// **A closed range graphs like the float it is** (K-414). The Slider kind
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
    });

    /// **A mask's numbers reach the graph** (K-341), and so does its **shape**
    /// once it is keyed (K-344) — as the interpolation parameter, whose slope
    /// is the rate the shape is changing at. A *still* shape has no keys and so
    /// no curve, and stays out.
    testWidgets("graphChannels resolves a mask's numbers and its keyed shape",
        (tester) async {
      final p = withLayer();
      p.layer.addMask(
        mask: BridgeMask(
          id: UuidValue.fromString(const Uuid().v4()),
          name: 'Ellipse',
          vertices: const [
            BridgeVertex(
                x: 0, y: 0, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 10, y: 0, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 10, y: 8, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
          ],
          closed: true,
          inverted: false,
          opacity: const BridgeScalar.static_(100),
          mode: BridgeMaskMode.add,
          feather: const BridgeScalar.static_(0),
          expansion: const BridgeScalar.static_(0),
          pathKeys: const [],
        ),
      );
      final id = p.layer.internallayerId.toString();
      final maskId = p.layer.getMasks().single.id;
      p.uiState.model.refresh();

      final channels = graphChannels(
        layers: p.uiState.model.layers,
        selected: [
          '$id/masks/$maskId/opacity',
          '$id/masks/$maskId/feather',
          '$id/masks/$maskId/path',
        ],
      );
      expect(
          channels.map((c) => c.id).toList(),
          [
            '$id/masks/$maskId/opacity',
            '$id/masks/$maskId/feather',
          ],
          reason: 'the shape has no curve, so it is not a channel');
      expect(channels.first.label, contains('Ellipse'));
    });

    // --- the Vegas speed envelope (K-247) -------------------------------

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
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('tl-retime-name')));
      await tester.pump();
    }

    testWidgets('a Retime opens to Velocity, and as one dot per key',
        (tester) async {
      final p = withLayer();
      await openRetime(tester, p, vegas: true);

      // The preference chose the lens on the way in (K-246).
      final base = 'graph-key-${p.layer.internallayerId}/retime#0';
      expect(find.byKey(ValueKey<String>('$base-out')), findsOneWidget,
          reason: 'the speed view is showing');
      // …and the envelope is one point per key, not the two-sided pair the
      // ordinary speed graph draws (K-247).
      expect(find.byKey(ValueKey<String>('$base-in')), findsNothing);
      final second = 'graph-key-${p.layer.internallayerId}/retime#1';
      expect(find.byKey(ValueKey<String>('$second-in')), findsNothing);
      expect(find.byKey(ValueKey<String>('$second-out')), findsOneWidget);
    });

    testWidgets('with the preference off a Retime opens to Time',
        (tester) async {
      final p = withLayer();
      await openRetime(tester, p, vegas: false);
      // The value view names its keys without a side.
      expect(
          find.byKey(ValueKey<String>(
              'graph-key-${p.layer.internallayerId}/retime#0')),
          findsOneWidget);
    });

    /// The Vegas edit: drag a point's speed and the frames after it change,
    /// while every keyframe time stays exactly where it was (K-022, K-247).
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

    /// Lifting a key is `Alt`-click or the Pen, never a double-click on the
    /// key itself.
    ///
    /// Registering a double-tap there would make Flutter hold *every* single
    /// tap back until the double-tap timer expired, and a single tap is the
    /// commonest gesture in the pane — so clicking a key to select it would
    /// have gained a visible delay. Clicking a key twice therefore does
    /// nothing but select it, twice.
    testWidgets('clicking a key twice does not lift it', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);
      final key = find.byKey(ValueKey<String>(
          'graph-key-${p.layer.internallayerId}/transform/opacity@opacity#1'));

      await tester.tap(key);
      await tester.pump();
      expect(opacityKeys(p.layer), hasLength(3));

      await tester.tap(key);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(key);
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer), hasLength(3),
          reason:
              'the key is still there — double-click plants, it does not lift');
    });

    testWidgets('the last key of a channel refuses to be lifted',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0]);
      await mountGraph(tester, p);
      final key = find.byKey(ValueKey<String>(
          'graph-key-${p.layer.internallayerId}/transform/opacity@opacity#0'));
      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.tap(key);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.pumpAndSettle();
      expect(opacityKeys(p.layer), hasLength(1),
          reason: 'a keyframed property is never left with no keys at all');
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

    testWidgets('Shift the other way holds the frame instead', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);

      final id =
          'graph-key-${p.layer.internallayerId}/transform/opacity@opacity#1';
      final beforeFrames = [
        for (final k in opacityKeys(p.layer)) p.comp.frameAtTime(time: k.time)
      ];
      final beforeValue = opacityKeys(p.layer)[1].value;

      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await _drag(
          tester, find.byKey(ValueKey<String>(id)), const Offset(-12, 60));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);

      expect([
        for (final k in opacityKeys(p.layer)) p.comp.frameAtTime(time: k.time)
      ], beforeFrames, reason: 'every frame is held');
      expect(opacityKeys(p.layer)[1].value, lessThan(beforeValue),
          reason: 'and the value moved');
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

    /// Bring a key's handles out: easy-ease it (F9) and leave it selected.
    Future<String> easedAndSelected(WidgetTester tester, dynamic p,
        {int index = 1}) async {
      await tester
          .tap(find.byKey(ValueKey<String>(opacityKey(p.layer, index))));
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.f9);
      await tester.pumpAndSettle();
      return 'graph-handle-${(p.layer as LayerReference).internallayerId}'
          '/transform/opacity@opacity#$index';
    }

    /// **The reported fault** (§6.1): the glyph moved under the pointer while
    /// the handle line and its dot went on reading the document's unmoved key,
    /// so the line stretched to a stranded endpoint and the dot never stirred.
    /// Everything is measured from one moved list now, so it has to travel
    /// *before* the release, not on it.
    testWidgets('a dragged key carries its handles with it, live',
        (tester) async {
      final p = withLayer();
      spreadOpacity(p);
      await mountGraph(tester, p);
      final base = await easedAndSelected(tester, p);

      final keyFinder = find.byKey(ValueKey<String>(opacityKey(p.layer, 1)));
      final outHandle = find.byKey(ValueKey<String>('$base-out'));
      expect(outHandle, findsOneWidget);

      final keyBefore = tester.getCenter(keyFinder);
      final handleBefore = tester.getCenter(outHandle);

      final gesture = await tester.startGesture(keyBefore);
      await tester.pump();
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(0, -6));
        await tester.pump();
      }
      final keyMid = tester.getCenter(keyFinder);
      final handleMid = tester.getCenter(outHandle);

      expect(keyMid.dy, lessThan(keyBefore.dy - 4),
          reason: 'the key itself moved while the pointer was down');
      expect(handleMid.dy - handleBefore.dy,
          closeTo(keyMid.dy - keyBefore.dy, 1.5),
          reason: 'and its handle travelled with it, before the release');

      await gesture.up();
      await tester.pumpAndSettle();
    });

    /// The drawing's handle line: 2 on, 2 off, in `text_primary`. Solid
    /// `warning` was neither (K-439 — `warning` has no job on this pane).
    testWidgets('handle lines are dashed hairlines in text_primary',
        (tester) async {
      final p = withLayer();
      spreadOpacity(p);
      await mountGraph(tester, p);
      await easedAndSelected(tester, p);

      final t = LumitTheme.dark();
      final lines = find.byKey(const ValueKey('graph-handle-lines'));
      final painter = tester.widget<CustomPaint>(lines).painter as dynamic;
      expect(painter.colour, t.textPrimary,
          reason: 'selection speaks in text_primary, never warning');

      // Three segments in a row: a solid line would paint one and stop.
      final dash = t.textPrimary.withValues(alpha: 0.8);
      expect(
        lines,
        paints
          ..line(color: dash)
          ..line(color: dash)
          ..line(color: dash),
        reason: 'the line is cut into dashes, not drawn whole',
      );
    });

    /// The endpoint is the drawing's hollow ring — a `text_primary` stroke
    /// round a hole in the pane's own ground — and it brightens under the
    /// pointer and dims again when it leaves (P1). The cursor over it says
    /// which way it swings before the button goes down (P2).
    testWidgets('a handle endpoint is a hollow ring that answers the pointer',
        (tester) async {
      final p = withLayer();
      spreadOpacity(p);
      await mountGraph(tester, p);
      final base = await easedAndSelected(tester, p);
      final outHandle = find.byKey(ValueKey<String>('$base-out'));

      final t = LumitTheme.dark();
      dynamic ring() => tester
          .widget<CustomPaint>(find.descendant(
              of: outHandle, matching: find.byType(CustomPaint)))
          .painter as dynamic;
      expect(ring().colour, t.textPrimary);
      expect(ring().fill, t.surface0,
          reason: 'hollow: the ground shows through the ring');
      expect(ring().hovered, isFalse, reason: 'nothing at rest');

      expect(
          tester
              .widget<MouseRegion>(find
                  .descendant(of: outHandle, matching: find.byType(MouseRegion))
                  .first)
              .cursor,
          SystemMouseCursors.resizeUpDown);

      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      addTearDown(mouse.removePointer);
      await mouse.moveTo(tester.getCenter(outHandle));
      await tester.pump();
      expect(ring().hovered, isTrue, reason: 'the ring brightens under it');

      await mouse.moveTo(Offset.zero);
      await tester.pump();
      expect(ring().hovered, isFalse, reason: 'and leaves nothing behind');
    });

    /// A selected key is `text_primary` and one size step larger — never the
    /// accent (K-439) — and its grab target does not change size with it.
    testWidgets('a selected key draws in text_primary, one step larger',
        (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);

      final t = LumitTheme.dark();
      final key = find.byKey(ValueKey<String>(opacityKey(p.layer, 1)));
      Finder glyph() =>
          find.descendant(of: key, matching: find.byType(CustomPaint));
      final restingGlyph = tester.getSize(glyph());
      final restingTarget = tester.getSize(key);
      expect((tester.widget<CustomPaint>(glyph()).painter as dynamic).colour,
          isNot(t.accent));

      expect(
          tester
              .widget<MouseRegion>(find
                  .descendant(of: key, matching: find.byType(MouseRegion))
                  .first)
              .cursor,
          SystemMouseCursors.move,
          reason: 'a key moves in time and value, and says so');

      await tester.tap(key);
      await tester.pump();

      expect((tester.widget<CustomPaint>(glyph()).painter as dynamic).colour,
          t.textPrimary,
          reason: 'what is selected is text_primary, never the accent');
      expect(tester.getSize(glyph()).width, greaterThan(restingGlyph.width),
          reason: 'and one size step larger');
      expect(tester.getSize(key), restingTarget,
          reason: 'the target under it does not move with the mark');
    });

    /// The drawing's value hint pill: one key in hand, its frame, value and
    /// the influence of each side, gone when the selection is not exactly one.
    testWidgets('one selected key carries the value hint pill', (tester) async {
      final p = withLayer();
      animateOpacity(p.comp, p.layer, frames: [0, 50, 100]);
      await mountGraph(tester, p);

      final pill = find.byKey(const ValueKey('graph-value-hint'));
      expect(pill, findsNothing, reason: 'nothing at rest');

      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 1))));
      await tester.pump();
      expect(pill, findsOneWidget);
      expect(find.textContaining('f50 · 50 · 33 / 33 %'), findsOneWidget,
          reason: 'the frame, the value, and both influences in per cent');

      // A block of keys has its own badge; the single-key readout stands down.
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.tap(find.byKey(ValueKey<String>(opacityKey(p.layer, 2))));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
      expect(pill, findsNothing, reason: 'two keys are a block, not a key');
    });
  }, skip: !engineAvailable);
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
