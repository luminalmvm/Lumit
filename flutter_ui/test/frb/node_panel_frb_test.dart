// The Node panel: it draws whichever box the Graph panel has picked.
//
// Two things are being held here. The first is the **coupling** — the Graph
// panel publishes its pick to the shell, and this panel follows it, without
// either panel knowing the other is mounted. The second is that a *driver* box
// gets its rows too: the effect stack has no place for one, which is the whole
// reason this panel exists beside Effect controls.
//
// Every document operation is genuine (see frb_test_support.dart).

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/panels/node_panel.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

import 'frb_test_support.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initEngineForTests);

  group('Node panel (frb)', () {
    /// A comp with one solid carrying one Gaussian blur, selected.
    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withBlur() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      p.uiState.setSelectedComp(comp);
      p.uiState.selectedLayer.value = layer;
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    /// Add `name` as a driver, wired into the blur's `radius`, so the panel
    /// has both a driver box to draw and a driven row to count.
    UuidValue seedWiredDriver(LayerReference layer, String name) {
      final made = layer.newDriver(name: name);
      final id = made.id();
      final graph = layer.getGraph();
      final effect = graph.nodes.firstWhere((n) => n.matchName == 'blur');
      layer.setGraph(
        drivers: [...layer.getGraphDrivers(), made],
        wiring: BridgeGraphWiring(
          edges: [
            ...graph.wiring.edges,
            BridgeGraphEdge(
              from: BridgeOutputRef.driver(node: id, port: 'value'),
              to: BridgeInputRef.param(node: effect.node, port: 'radius'),
            ),
          ],
          layout: graph.wiring.layout,
          exposed: graph.wiring.exposed,
        ),
      );
      return id;
    }

    BridgeNodeRef effectRef(LayerReference layer) =>
        layer.getGraph().nodes.firstWhere((n) => n.matchName == 'blur').node;

    Future<void> mount(WidgetTester tester, dynamic p, Widget child,
        {double width = 340}) async {
      final size = Size(width, 400);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: child,
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: size,
      ));
      await tester.pump();
    }

    testWidgets('says so when nothing has been picked', (tester) async {
      final p = withBlur();
      await mount(tester, p, const NodePanelFrb());
      expect(find.byKey(const ValueKey('node-header')), findsNothing);
      expect(find.text(l10n.nodeNoSelection), findsOneWidget);
    });

    testWidgets('draws the picked effect box: its name and its rows',
        (tester) async {
      final p = withBlur();
      p.uiState.graphNode.value = effectRef(p.layer);
      await mount(tester, p, const NodePanelFrb());

      expect(find.byKey(const ValueKey('node-name')), findsOneWidget);
      final id = p.layer.getEffects().single.id();
      for (final param in cachedListParameters('blur')) {
        expect(find.byKey(ValueKey<String>('node-row-$id-${param.id}')),
            findsOneWidget,
            reason: '${param.id} is one of the picked box\'s parameters');
      }
      // Nothing is wired, so the header carries no count at all rather than a
      // zero: a tally of nothing is noise.
      expect(find.byKey(const ValueKey('node-driven-count')), findsNothing);
    });

    testWidgets('a driver box gets its rows too, which no stack list can show',
        (tester) async {
      final p = withBlur();
      final wiggle = seedWiredDriver(p.layer, 'wiggle');
      p.uiState.graphNode.value = BridgeNodeRef.driver(wiggle);
      await mount(tester, p, const NodePanelFrb());

      final params = cachedListParameters('wiggle');
      expect(params, isNotEmpty, reason: 'a driver declares parameters');
      for (final param in params) {
        expect(find.byKey(ValueKey<String>('node-row-$wiggle-${param.id}')),
            findsOneWidget);
      }
    });

    testWidgets('the header counts the parameters a wire has taken over',
        (tester) async {
      final p = withBlur();
      seedWiredDriver(p.layer, 'wiggle');
      p.uiState.graphNode.value = effectRef(p.layer);
      await mount(tester, p, const NodePanelFrb());

      expect(find.byKey(const ValueKey('node-driven-count')), findsOneWidget);
      expect(find.text(l10n.nodeDrivenCount(1)), findsOneWidget);
    });

    testWidgets('follows the pick as it moves from one box to another',
        (tester) async {
      final p = withBlur();
      final wiggle = seedWiredDriver(p.layer, 'wiggle');
      p.uiState.graphNode.value = effectRef(p.layer);
      await mount(tester, p, const NodePanelFrb());
      final blurId = p.layer.getEffects().single.id();
      expect(find.byKey(ValueKey<String>('node-row-$blurId-radius')),
          findsOneWidget);

      p.uiState.graphNode.value = BridgeNodeRef.driver(wiggle);
      await tester.pump();
      expect(find.byKey(ValueKey<String>('node-row-$blurId-radius')),
          findsNothing);
      expect(
          find.byKey(ValueKey<String>(
              'node-row-$wiggle-${cachedListParameters('wiggle').first.id}')),
          findsOneWidget);
    });

    /// **A driver's number is dragged live** (WP4). The drag stages the value
    /// and asks for a preview frame through `renderFrameWithDriverPreview`,
    /// which substitutes the graph's nodes on a throwaway copy exactly as the
    /// stack preview substitutes the effect list; the document is written once,
    /// on release.
    ///
    /// The second tick is the load-bearing part. A `BridgeEffectInstance`
    /// handed to a preview call is *moved* — frb disposes the Dart side of it —
    /// so a panel that read the driver handles once and reused them would throw
    /// `DroppableDisposedException` on the tick after the first. Two moves with
    /// the throttle's interval between them is what forces a second real call.
    testWidgets('dragging a driver value previews live and commits once',
        (tester) async {
      final p = withBlur();
      final wiggle = seedWiredDriver(p.layer, 'wiggle');
      p.uiState.graphNode.value = BridgeNodeRef.driver(wiggle);
      await mount(tester, p, const NodePanelFrb());

      double amount() =>
          ((p.layer.getGraphDrivers().single.getValue(id: 'amount')
                      as BridgeEffectValue_Float)
                  .field0 as BridgeScalar_Static)
              .field0;
      final before = amount();

      final gesture = await tester
          .startGesture(tester.getCenter(find.byKey(ValueKey<String>(
        'fx-float-$wiggle-amount',
      ))));
      await gesture.moveBy(const Offset(30, 0));
      await tester.pump();
      expect(amount(), before, reason: 'a drag tick previews; it never writes');

      await tester.pump(const Duration(milliseconds: 40));
      await gesture.moveBy(const Offset(30, 0));
      await tester.pump(const Duration(milliseconds: 40));
      expect(tester.takeException(), isNull,
          reason: 'each preview tick reads its own handles');
      expect(amount(), before, reason: 'still nothing written');

      await gesture.up();
      await tester.pumpAndSettle();
      expect(amount(), greaterThan(before),
          reason: 'the release reached the document');

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(amount(), before,
          reason: 'the whole drag was one op, so one undo puts it back');
    });

    // --- Points sample's rows (points-stream.md §2.2, §4.3) ---------------

    /// Put Particulate on the layer beside the blur and add a Points sample
    /// driver whose Count drives the blur's Radius. The stream itself is left
    /// unwired: that is the state K-509 is about.
    UuidValue seedSample(LayerReference layer) {
      layer.addEffect(name: 'particulate');
      final made = layer.newDriver(name: 'points_sample');
      final id = made.id();
      final graph = layer.getGraph();
      final blur = graph.nodes.firstWhere((n) => n.matchName == 'blur');
      layer.setGraph(
        drivers: [...layer.getGraphDrivers(), made],
        wiring: BridgeGraphWiring(
          edges: [
            ...graph.wiring.edges,
            BridgeGraphEdge(
              from: BridgeOutputRef.driver(node: id, port: 'count'),
              to: BridgeInputRef.param(node: blur.node, port: 'radius'),
            ),
          ],
          layout: graph.wiring.layout,
          exposed: graph.wiring.exposed,
        ),
      );
      return id;
    }

    /// **One row for the point, none for the stream.** Position is an `_x`/`_y`
    /// pair and folds like every other point pair (K-443), dropper and all;
    /// the Points input is wire-only — no stored value, nothing to keyframe —
    /// so there is nothing here for it to be a row of.
    testWidgets('Points sample folds its Position and draws no stream row',
        (tester) async {
      final p = withBlur();
      final sample = seedSample(p.layer);
      p.uiState.graphNode.value = BridgeNodeRef.driver(sample);
      // A point row carries two wells, the unit and the crosshair, so it wants
      // more of the control column than a single number does. See the note on
      // narrow panes in `EffectPointRowFrb`.
      await mount(tester, p, const NodePanelFrb(), width: 430);

      expect(find.byKey(ValueKey<String>('node-row-$sample-position_x-pair')),
          findsOneWidget);
      expect(find.byKey(ValueKey<String>('node-row-$sample-position_x')),
          findsNothing);
      expect(find.byKey(ValueKey<String>('node-row-$sample-position_y')),
          findsNothing);
      expect(find.text('Position'), findsOneWidget);
      expect(find.text('Position y'), findsNothing);

      // The query point is px@comp, so it takes the crosshair that picks a
      // place off the Viewer (K-260).
      expect(find.byKey(ValueKey<String>('dropper-fx-$sample-position_x')),
          findsOneWidget);

      expect(
          find.byKey(ValueKey<String>('node-row-$sample-points')), findsNothing,
          reason: 'a wire-only input has no row anywhere');
    });

    /// **The hazard, on the row it reaches** (K-509). A driven row cannot show
    /// the number arriving along its wire — that is a per-frame value and no
    /// rebuild may ask for one — but it can say that the box at the far end
    /// has no stream, which is the case where the number is a documented
    /// no-op rather than a measurement.
    testWidgets('a row driven by a streamless sample wears the warning',
        (tester) async {
      final p = withBlur();
      final sample = seedSample(p.layer);
      p.uiState.graphNode.value = effectRef(p.layer);
      await mount(tester, p, const NodePanelFrb());

      final id = p.layer.getEffects().first.id();
      expect(
          find.byKey(ValueKey<String>('fx-driven-$id-radius')), findsOneWidget);
      expect(find.byKey(ValueKey<String>('fx-no-stream-$id-radius')),
          findsOneWidget);
      expect(find.text(l10n.graphNoStream), findsOneWidget);
      expect(find.text(l10n.graphDriven), findsNothing,
          reason: 'the word is the state: this row follows a no-op');

      // Wire the stream in and the mark goes: the wire now carries something.
      final particulate = p.layer
          .getGraph()
          .nodes
          .firstWhere((n) => n.matchName == 'particulate')
          .node as BridgeNodeRef_Effect;
      final now = p.layer.getGraph();
      p.layer.setGraph(
        drivers: p.layer.getGraphDrivers(),
        wiring: BridgeGraphWiring(
          edges: [
            ...now.wiring.edges,
            BridgeGraphEdge(
              from: BridgeOutputRef.effectData(
                  effect: particulate.field0, port: 'points'),
              to: BridgeInputRef.param(
                  node: BridgeNodeRef.driver(sample), port: 'points'),
            ),
          ],
          layout: now.wiring.layout,
          exposed: now.wiring.exposed,
        ),
      );
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.byKey(ValueKey<String>('fx-no-stream-$id-radius')),
          findsNothing);
    });

    /// The coupling. Clicking a box on the canvas is what fills this panel,
    /// and neither panel is told the other exists — the pick goes through the
    /// shell, so the Node panel works whether or not the graph is on screen.
    testWidgets('the Graph panel publishes its pick to the shell',
        (tester) async {
      final p = withBlur();
      const size = Size(900, 600);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const GraphPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: size,
      ));
      await tester.pump();

      expect(p.uiState.graphNode.value, isNull);
      final key = graphNodeKey(effectRef(p.layer));
      await tester.tapAt(
          tester.getCenter(find.byKey(ValueKey<String>('graph-node-$key'))));
      await tester.pump();
      expect(p.uiState.graphNode.value, isNotNull);
      expect(graphNodeKey(p.uiState.graphNode.value!), key);
    });
  });
}
