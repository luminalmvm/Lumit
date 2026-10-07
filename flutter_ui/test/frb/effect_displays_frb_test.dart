// Three panel halves against the real engine: Curves' curve editor, Levels'
// histogram row, and the Slider control.
//
// Every document operation here is genuine (frb_test_support.dart), so a write
// asserted below is a value the engine actually holds — which is the point:
// what a curve editor must not do is look right and commit something else.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/widgets/curve_editor.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Effect displays (frb)', () {
    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withLayer() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = comp.getLayers().single;
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    Future<void> mount(
      WidgetTester tester,
      ({LumitState state, LumitUiState uiState, LayerReference layer}) p,
    ) async {
      p.uiState.workspace.interface.transformInEffectControls = false;
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
    }

    List<List<double>> curveOf(LayerReference layer, String param) =>
        switch (layer.getEffects().single.getValue(id: param)) {
          BridgeEffectValue_Curve(:final field0) => [
              for (final xy in field0) [xy[0].toDouble(), xy[1].toDouble()],
            ],
          _ => const [],
        };

    // --------------------------------------------------------------- Curves

    testWidgets('a tab shows that channel, and Reset restores only it',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'curves');
      await mount(tester, p);
      final id = p.layer.getEffects().single.id();

      // Bend Blue, by clicking the middle of its plot to add a point.
      await tester.tap(find.byKey(ValueKey<String>('fx-curves-$id-tab-3')));
      await tester.pump();
      expect(
          find.byKey(ValueKey<String>('fx-curves-$id-plot-3')), findsOneWidget);

      final plot = find.byType(CurveEditor);
      final centre = tester.getCenter(plot);
      await tester.tapAt(centre + const Offset(0, -20));
      await tester.pumpAndSettle();

      expect(curveOf(p.layer, 'blue'), hasLength(3),
          reason: 'a click on the plot adds a point to the channel showing');
      expect(curveOf(p.layer, 'master'), hasLength(2),
          reason: 'and to that channel only');

      await tester.tap(find.byKey(ValueKey<String>('fx-curves-$id-reset')));
      await tester.pumpAndSettle();
      expect(
          curveOf(p.layer, 'blue'),
          [
            [0.0, 0.0],
            [1.0, 1.0]
          ],
          reason: 'Reset puts this channel back to the diagonal');
    });

    testWidgets('dragging a point moves it, and the engine holds what moved',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'curves');
      await mount(tester, p);

      // Plant a mid-point to drag, then take hold of it and pull it up.
      final plot = find.byType(CurveEditor);
      final centre = tester.getCenter(plot);
      await tester.tapAt(centre);
      await tester.pumpAndSettle();
      expect(curveOf(p.layer, 'master'), hasLength(3));
      final before = curveOf(p.layer, 'master')[1];

      await tester.dragFrom(centre, const Offset(0, -30));
      await tester.pumpAndSettle();

      final after = curveOf(p.layer, 'master');
      expect(after, hasLength(3),
          reason: 'a drag moves a point, never adds one');
      expect(after[1][1], greaterThan(before[1]),
          reason: 'dragging up lifts that input’s output');
      expect(after.first, [0.0, 0.0], reason: 'the ends stay put');
      expect(after.last, [1.0, 1.0]);
    });

    testWidgets('a point dragged well clear of the square is dropped',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'curves');
      await mount(tester, p);

      final plot = find.byType(CurveEditor);
      final centre = tester.getCenter(plot);
      await tester.tapAt(centre);
      await tester.pumpAndSettle();
      expect(curveOf(p.layer, 'master'), hasLength(3));

      // Straight down, far past the bottom edge.
      await tester.dragFrom(centre, const Offset(0, 220));
      await tester.pumpAndSettle();
      expect(curveOf(p.layer, 'master'), hasLength(2),
          reason: 'the point is gone, and the two ends remain');
    });

    // --------------------------------------------------------------- Levels

    testWidgets('the channel buttons aim the handles at that channel',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'levels');
      await mount(tester, p);

      // Master, Red, Green and Blue — the effect's own four groups, and no
      // Alpha, because Levels has no alpha lane.
      for (var i = 0; i < 4; i++) {
        expect(
            find.byKey(ValueKey<String>('fx-levels-tab-$i')), findsOneWidget);
      }
      expect(find.byKey(const ValueKey('fx-levels-tab-4')), findsNothing);

      await tester.tap(find.byKey(const ValueKey('fx-levels-tab-1')));
      await tester.pumpAndSettle();

      final strip = find.byKey(const ValueKey('fx-levels-input-handles'));
      final box = tester.getRect(strip);
      await tester.dragFrom(
          Offset(box.left + 2, box.center.dy), Offset(box.width * 0.3, 0));
      await tester.pumpAndSettle();

      double black(String param) =>
          ((p.layer.getEffects().single.getValue(id: param)
                      as BridgeEffectValue_Float)
                  .field0 as BridgeScalar_Static)
              .field0;
      expect(black('red_in_black'), greaterThan(0.2),
          reason: 'the drag reached the channel the buttons chose');
      expect(black('red_in_black'), lessThan(0.4));
      expect(black('master_in_black'), 0, reason: 'and Master is untouched');
    });

    // --------------------------------------------------------------- Slider

    testWidgets('a closed range draws a track, and a drag on it commits once',
        (tester) async {
      final p = withLayer();
      // Completion is the catalogue's one genuinely closed range:
      // a wipe is between not begun and complete, and there is no picture
      // either side of that.
      p.layer.addEffect(name: 'linear_wipe');
      await mount(tester, p);
      final id = p.layer.getEffects().single.id();

      final track = find.byKey(ValueKey<String>('fx-slider-$id-completion'));
      expect(track, findsOneWidget, reason: 'a Slider kind draws a track');
      expect(find.byKey(ValueKey<String>('fx-float-$id-completion')),
          findsOneWidget,
          reason: 'with the number beside it, still typable and keyframable');

      double completion() =>
          ((p.layer.getEffects().single.getValue(id: 'completion')
                      as BridgeEffectValue_Float)
                  .field0 as BridgeScalar_Static)
              .field0;
      final before = completion();

      final box = tester.getRect(track);
      await tester.dragFrom(
          Offset(box.left + 4, box.center.dy), Offset(box.width * 0.5, 0));
      await tester.pumpAndSettle();

      final after = completion();
      expect(after, isNot(before), reason: 'the drag reached the document');
      expect(after, inInclusiveRange(0, 100),
          reason: 'and never leaves the closed range');
    });

    /// The other half of "the kind is the control, not the storage":
    /// a closed range can still be driven by an expression, which means the
    /// number beside the track must offer the same menu entry the plain float
    /// row offers. It did not, so adopting the kind on the four wipes'
    /// Completion quietly took the entry away from a parameter that had it.
    testWidgets('a closed range still offers an expression', (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'linear_wipe');
      await mount(tester, p);
      final id = p.layer.getEffects().single.id();

      await tester.tap(find.byKey(ValueKey<String>('fx-float-$id-completion')),
          buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      expect(find.text('Set expression'), findsOneWidget,
          reason: 'a Slider keeps every float affordance (docs/08 §1.2)');

      await tester.tap(find.text('Set expression'));
      await tester.pumpAndSettle();
      // Seeded with the value showing, so turning one on moves no picture.
      expect(
          p.layer.getEffects().single.getValue(id: 'completion'),
          isA<BridgeEffectValue_Float>().having(
              (v) => v.field0, 'scalar', isA<BridgeScalar_Expression>()));
    });
  }, skip: !engineAvailable);
}
