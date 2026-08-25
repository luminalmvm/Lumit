// The layer driver graph as Dart sees it (K-471, K-472; docs/17, docs/impl/node-graph.md §5).
//
// What is asserted here is the *contract*: that the shapes the Graph panel will
// be built on cross the bridge intact, that reading a layer's whole graph is one
// call, and that a gesture the panel will make — drop a driver, wire it, undo —
// is one op through the real engine. What the panel *draws* belongs to WP3;
// there is no panel yet, and this file deliberately pumps no widgets.
//
// Every document operation here is genuine; see frb_test_support.dart.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/engine_labels.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  // No widget is pumped here, but opening a project sets the window title
  // through a platform channel, and a channel with no binding behind it throws.
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initEngineForTests);

  group('Layer graph (frb)', () {
    /// A comp with one solid carrying one Gaussian blur, selected by nobody.
    ({LumitState state, LayerReference layer}) withBlur() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      return (state: p.state, layer: layer);
    }

    test('the whole graph crosses in one call, chain first', () {
      final p = withBlur();
      final graph = p.layer.getGraph();

      expect(
        graph.nodes.map((n) => n.node.runtimeType).toList(),
        [BridgeNodeRef_Source, BridgeNodeRef_Effect, BridgeNodeRef_Out],
        reason: 'Source, one box per effect in stack order, then Layer out',
      );
      expect(graph.nodes[1].label, 'Gaussian blur');
      expect(graph.wiring.edges, isEmpty);
      expect(p.layer.getGraphDrivers(), isEmpty);

      // A socket carries its type, never a colour: the frontend maps the type
      // to a `port.*` theme token itself (K-472).
      final out = graph.nodes[1].outputs.single;
      expect(out.id, 'output');
      expect(out.portType, BridgePortType.image);

      // Every word the engine sent has a translation entry (K-303).
      for (final node in graph.nodes) {
        expect(hasEngineLabel(node.label), isTrue,
            reason: '"${node.label}" has no entry in engine_labels.dart');
        for (final port in [...node.inputs, ...node.outputs]) {
          expect(hasEngineLabel(port.label), isTrue,
              reason: '"${port.label}" has no entry in engine_labels.dart');
        }
      }
    });

    test('dropping a driver and wiring it is one op and one undo step', () {
      final p = withBlur();
      final blur = p.layer.getGraph().nodes[1].node as BridgeNodeRef_Effect;

      final wiggle = p.layer.newDriver(name: 'wiggle');
      final id = wiggle.id();
      p.layer.setGraph(
        drivers: [wiggle],
        wiring: BridgeGraphWiring(
          edges: [
            BridgeGraphEdge(
              from: BridgeOutputRef.driver(node: id, port: 'value'),
              to: BridgeInputRef.param(node: blur, port: 'radius'),
            ),
          ],
          layout: [BridgeNodePosition(node: BridgeNodeRef.driver(id), x: 8, y: 4)],
          exposed: const [],
        ),
      );

      final graph = p.layer.getGraph();
      final driver = graph.nodes.last;
      expect(driver.node, BridgeNodeRef.driver(id));
      expect(driver.label, 'Wiggle');
      expect(driver.outputs.single.wired, isTrue);
      expect(graph.wiring.layout.single.x, 8);
      expect(
        graph.nodes[1].inputs.firstWhere((p) => p.id == 'radius').wired,
        isTrue,
        reason: 'the driven parameter draws a filled socket',
      );

      // The driver's parameters ride the ordinary staged-instance path.
      final staged = p.layer.getGraphDrivers();
      expect(staged.single.getParameters(), contains('amount'));

      p.state.project!.undo();
      expect(p.layer.getGraph().wiring.edges, isEmpty);
      expect(p.layer.getGraphDrivers(), isEmpty,
          reason: 'the node and its wire arrived together and leave together');
    });

    // The refusal's *sentence* is asserted in Rust
    // (`crates/lumit-bridge/src/api/tests.rs`), because a sync call's
    // `BridgeError` reaches Dart as an opaque throw — true of every op on this
    // seam, not of this one. What matters here is that the write is declined
    // and the document is left exactly as it was; the panel's own job is to
    // refuse a mismatched drop before it ever commits, since both sockets carry
    // their type in the read model.
    test('a wire between two different types is refused', () {
      final p = withBlur();
      final blur = p.layer.getGraph().nodes[1].node as BridgeNodeRef_Effect;
      final cycle = p.layer.newDriver(name: 'colour_cycle');

      expect(
        () => p.layer.setGraph(
          drivers: [cycle],
          wiring: BridgeGraphWiring(
            edges: [
              BridgeGraphEdge(
                from: BridgeOutputRef.driver(node: cycle.id(), port: 'colour'),
                to: BridgeInputRef.param(node: blur, port: 'radius'),
              ),
            ],
            layout: const [],
            exposed: const [],
          ),
        ),
        throwsA(anything),
      );
      expect(p.layer.getGraph().wiring.edges, isEmpty,
          reason: 'a refused write leaves the document exactly as it was');
    });

    test('the Drivers family lists separately from the effects', () {
      final drivers = listDrivers();
      expect(drivers.map((d) => d.name), contains('wiggle'));
      expect(drivers.every((d) => d.category == 'drivers'), isTrue);
      expect(hasEngineLabel(drivers.first.categoryLabel), isTrue);
      expect(
        listEffects().map((e) => e.name),
        isNot(contains('wiggle')),
        reason: 'a driver changes no pixel, so it is not an Add-effect entry',
      );
    });
  });
}
