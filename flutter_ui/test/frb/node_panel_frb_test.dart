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

    Future<void> mount(WidgetTester tester, dynamic p, Widget child) async {
      const size = Size(340, 400);
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
