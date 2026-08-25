// The three panel halves of the K-412/K-413/K-414 sitting, against the real
// engine: Curves' curve editor, Levels' histogram row, and the Slider control.
//
// Every document operation here is genuine (frb_test_support.dart), so a write
// asserted below is a value the engine actually holds — which is the point:
// what a curve editor must not do is look right and commit something else.

import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/levels_display_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/panels/scopes_panel_frb.dart'
    show scopeColoursFor;
import 'package:lumit_flutter/theme/theme.dart';
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

    // ---------------------------------------------------------------- K-412

    testWidgets('Curves draws one tabbed editor, at the identity diagonal',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'curves');
      await mount(tester, p);
      final id = p.layer.getEffects().single.id();

      // Five channels, one editor: the tabs name them, and there is exactly
      // one plot rather than one per channel (docs/08 §3.30).
      for (final channel in ['Master', 'Red', 'Green', 'Blue', 'Alpha']) {
        expect(find.text(channel), findsOneWidget, reason: '$channel is a tab');
      }
      expect(find.byType(CurveEditor), findsOneWidget,
          reason: 'five curves, one plot');
      expect(
          find.byKey(ValueKey<String>('fx-curves-$id-plot-0')), findsOneWidget,
          reason: 'Master is the channel showing first');

      // The rows that are not curves keep their ordinary rows.
      expect(find.text('Mix'), findsOneWidget);

      // The default really is the diagonal, and nothing has been written.
      expect(curveOf(p.layer, 'master'), [
        [0.0, 0.0],
        [1.0, 1.0]
      ]);
    });

    /// **A channel curve draws in its channel's colour** (owner, desk test).
    /// Every plot used to be the theme's primary text colour, so a Curves Red
    /// tab and a Master tab were the same picture and the tab strip was the
    /// only way to tell which was up. Red, Green and Blue take the standard
    /// R, G and B — the ones every other channel reading in the application
    /// uses — and Master and Alpha stay themed, because they are not a
    /// channel of colour.
    testWidgets('a Curves channel draws in its own colour', (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'curves');
      await mount(tester, p);
      final id = p.layer.getEffects().single.id();
      final t = LumitTheme.dark();

      Color lineNow() =>
          tester.widget<CurveEditor>(find.byType(CurveEditor)).line ??
          t.textPrimary;

      // Tab 0 is Master: no channel colour of its own.
      expect(lineNow(), t.textPrimary);

      for (final (tab, wanted) in [
        (1, ScopeColours.standard.red),
        (2, ScopeColours.standard.green),
        (3, ScopeColours.standard.blue),
      ]) {
        await tester
            .tap(find.byKey(ValueKey<String>('fx-curves-$id-tab-$tab')));
        await tester.pumpAndSettle();
        expect(lineNow(), wanted, reason: 'channel $tab draws as itself');
      }

      // Alpha is not a colour channel, so it takes the theme's own.
      await tester.tap(find.byKey(ValueKey<String>('fx-curves-$id-tab-4')));
      await tester.pumpAndSettle();
      expect(lineNow(), t.textPrimary);
    });

    /// And the setting hands the whole graph back to the theme for anyone who
    /// wants it that way. Off by default, so this is the deviation.
    testWidgets('the theme-colour setting takes the channel colours off',
        (tester) async {
      final p = withLayer();
      p.uiState.workspace.themedEffectGraphs = true;
      p.layer.addEffect(name: 'curves');
      await mount(tester, p);
      final id = p.layer.getEffects().single.id();

      await tester.tap(find.byKey(ValueKey<String>('fx-curves-$id-tab-1')));
      await tester.pumpAndSettle();
      expect(tester.widget<CurveEditor>(find.byType(CurveEditor)).line, isNull,
          reason: 'no colour of its own; the plot takes the theme figure');
    });

    /// Levels' histogram is the same claim on the other display: the ground
    /// and the luma trace are chrome and stay themed, the three channel humps
    /// are a measurement and take the standard R, G and B.
    test('the Levels histogram keeps its channel colours', () {
      final t = LumitTheme.dark();
      const standard = ScopeColours.standard;
      Uint8List rgb(Color c) => Uint8List.fromList(
          [(c.r * 255).round(), (c.g * 255).round(), (c.b * 255).round()]);

      final plain = levelsHistogramColours(t, themed: false);
      expect(plain[2], rgb(standard.red));
      expect(plain[3], rgb(standard.green));
      expect(plain[4], rgb(standard.blue));
      expect(plain[0], rgb(t.surface0), reason: 'the ground is still chrome');
      expect(plain[1], rgb(t.textPrimary),
          reason: 'and so is the luma trace, which is what Master reads on');

      // With the setting on, every one of the five is the theme's.
      expect(levelsHistogramColours(t, themed: true),
          scopeColoursFor(t, themed: true));
    });

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

    /// The display-only spline (see curve_editor.dart's header) must at least
    /// agree with the engine about the one curve everything depends on: the
    /// identity is a straight line, and a two-point curve is its own line.
    test('the drawn spline is the straight line an identity curve is', () {
      for (final x in [0.0, 0.125, 0.375, 0.5, 0.75, 1.0]) {
        expect(curveSample(curveIdentity, x), closeTo(x, 1e-9));
      }
      // Its own secant, end to end, which is what the clamped end condition
      // buys — a plain Catmull-Rom would bow away from it.
      const steep = [
        [0.2, 0.0],
        [0.8, 1.0]
      ];
      expect(curveSample(steep, 0.5), closeTo(0.5, 1e-9));
      // And a bent curve stays inside the square rather than bulging past the
      // highest point it passes through.
      const shoulder = [
        [0.0, 0.0],
        [0.4, 0.8],
        [0.7, 1.0],
        [1.0, 1.0]
      ];
      for (var i = 0; i <= 100; i++) {
        final y = curveSample(shoulder, i / 100);
        expect(y, inInclusiveRange(0.0, 1.0));
      }
    });

    // ---------------------------------------------------------------- K-413

    testWidgets('Levels draws its histogram, handles and output bar',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'levels');
      await mount(tester, p);

      expect(find.byType(LevelsDisplayFrb), findsOneWidget);
      // The histogram itself may be empty in the harness — no worker has
      // answered — so its presence is asserted, not its pixels.
      expect(find.byKey(const ValueKey('fx-levels-histogram')), findsOneWidget);
      expect(find.byKey(const ValueKey('fx-levels-input-handles')),
          findsOneWidget);
      expect(
          find.byKey(const ValueKey('fx-levels-output-bar')), findsOneWidget);
      expect(find.byKey(const ValueKey('fx-levels-output-handles')),
          findsOneWidget);

      // Presentation only: every number still has its own row, and none of
      // them has moved.
      expect(find.text('Input black'), findsWidgets);
      expect(find.text('Output white'), findsWidgets);
      final fx = p.layer.getEffects().single;
      expect(
          (fx.getValue(id: 'master_in_black') as BridgeEffectValue_Float)
              .field0,
          const BridgeScalar.static_(0));
      expect(
          (fx.getValue(id: 'master_in_white') as BridgeEffectValue_Float)
              .field0,
          const BridgeScalar.static_(1));
    });

    testWidgets('dragging the input black handle writes input black',
        (tester) async {
      final p = withLayer();
      p.layer.addEffect(name: 'levels');
      await mount(tester, p);

      final strip = find.byKey(const ValueKey('fx-levels-input-handles'));
      final box = tester.getRect(strip);
      // Grab at the left end — where input black sits — and pull it right.
      await tester.dragFrom(
          Offset(box.left + 2, box.center.dy), Offset(box.width * 0.3, 0));
      await tester.pumpAndSettle();

      final black = (p.layer.getEffects().single.getValue(id: 'master_in_black')
              as BridgeEffectValue_Float)
          .field0 as BridgeScalar_Static;
      expect(black.field0, greaterThan(0.2),
          reason: 'the handle moved to roughly a third across');
      expect(black.field0, lessThan(0.4));
    });

    // ---------------------------------------------------------------- K-414

    testWidgets('the Controls category holds the five identity effects',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      await tester.tap(find.byKey(const ValueKey('fx-add')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('fx-category-controls')));
      await tester.pumpAndSettle();

      for (final label in [
        'Slider control',
        'Angle control',
        'Checkbox control',
        'Colour control',
        'Point control',
      ]) {
        expect(find.text(label), findsOneWidget, reason: '$label is offered');
      }

      await tester.tap(find.text('Point control'));
      await tester.pumpAndSettle();
      expect(p.layer.getEffects(), hasLength(1));
      // Its `_x`/`_y` pair folds into one row, like every other point.
      expect(find.text('Point'), findsOneWidget);
      expect(find.text('Point y'), findsNothing);
    });

    testWidgets('each Controls effect renders the control its kind asks for',
        (tester) async {
      final p = withLayer();
      for (final name in [
        'slider_control',
        'angle_control',
        'checkbox_control',
        'colour_control',
      ]) {
        p.layer.addEffect(name: name);
      }
      await mount(tester, p);
      final stack = p.layer.getEffects();

      expect(find.byKey(ValueKey<String>('fx-float-${stack[0].id()}-slider')),
          findsOneWidget,
          reason: 'the Slider control is an unbounded float, not a track');
      expect(find.text('Angle'), findsWidgets);
      expect(find.byKey(ValueKey<String>('fx-bool-${stack[2].id()}-checkbox')),
          findsOneWidget);
      expect(find.byKey(ValueKey<String>('fx-colour-${stack[3].id()}-colour')),
          findsOneWidget);
    });

    testWidgets('a closed range draws a track, and a drag on it commits once',
        (tester) async {
      final p = withLayer();
      // Completion is the catalogue's one genuinely closed range (K-414):
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

    /// The other half of "the kind is the control, not the storage" (K-414):
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
