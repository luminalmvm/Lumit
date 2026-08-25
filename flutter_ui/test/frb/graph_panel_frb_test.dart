// What the Graph panel *does* — the gestures, and what each one commits.
//
// Every document operation here is genuine (see frb_test_support.dart), so a
// claim about "one undo step" is a claim about the real journal rather than
// about a mock. What the panel *looks like* is `graph_panel_metrics_test.dart`.
//
// The two rules these tests exist to hold:
//
//  * **one gesture, one undo step** — wiring a driver into a parameter, or
//    deleting a wired box, comes back whole with a single undo; and
//  * **the panel declines what it can decline itself** — a drop between two
//    sockets of different types never reaches the engine, because both types
//    are in the read model already (docs/17, "The layer graph").

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

import 'frb_test_support.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initEngineForTests);

  group('Graph panel (frb)', () {
    /// A comp with one solid carrying one Gaussian blur, selected.
    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withBlur() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      p.uiState.selectedLayer.value = layer;
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    /// Add `name` as a driver at a spot on the canvas, outside the panel, so a
    /// test can start from a graph that already has one.
    UuidValue seedDriver(LayerReference layer, String name, Offset at) {
      final made = layer.newDriver(name: name);
      final id = made.id();
      final graph = layer.getGraph();
      layer.setGraph(
        drivers: [...layer.getGraphDrivers(), made],
        wiring: BridgeGraphWiring(
          edges: graph.wiring.edges,
          layout: [
            ...graph.wiring.layout,
            BridgeNodePosition(
                node: BridgeNodeRef.driver(id), x: at.dx, y: at.dy),
          ],
          exposed: graph.wiring.exposed,
        ),
      );
      return id;
    }

    Future<void> mount(WidgetTester tester, dynamic p,
        {List<BridgeEffectInfo> Function()? drivers}) async {
      const size = Size(900, 600);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: GraphPanelFrb(driversLister: drivers),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: size,
      ));
      await tester.pump();
    }

    Finder socket(String node, String port) =>
        find.byKey(ValueKey<String>('graph-socket-$node-$port'));

    /// The effect box's key, for a layer whose only effect is the blur.
    String effectKey(LayerReference layer) => graphNodeKey(
        layer.getGraph().nodes.firstWhere((n) => n.matchName == 'blur').node);

    testWidgets('the whole chain is drawn, Source to Layer out',
        (tester) async {
      final p = withBlur();
      await mount(tester, p);

      expect(find.byKey(const ValueKey<String>('graph-node-source')),
          findsOneWidget);
      expect(find.byKey(ValueKey<String>('graph-node-${effectKey(p.layer)}')),
          findsOneWidget);
      expect(
          find.byKey(const ValueKey<String>('graph-node-out')), findsOneWidget);
      // The Layer out box's Audio socket is drawn, unfilled and honest: audio
      // comes only from a footage layer's own stream in this phase (K-435).
      expect(socket('out', 'audio'), findsOneWidget);
    });

    /// The `E` badge grows the box to every parameter socket. Until it is on,
    /// a number socket nobody has wired is not drawn at all.
    testWidgets('exposure shows the parameter sockets, and is one op',
        (tester) async {
      final p = withBlur();
      await mount(tester, p);
      final key = effectKey(p.layer);

      expect(socket(key, 'radius'), findsNothing);
      await tester.tap(find.byKey(ValueKey<String>('graph-badge-E-$key')));
      await tester.pump();

      expect(socket(key, 'radius'), findsOneWidget);
      expect(p.layer.getGraph().wiring.exposed, hasLength(1));

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.layer.getGraph().wiring.exposed, isEmpty,
          reason: 'one gesture, one undo step');
    });

    testWidgets('dragging a driver output onto a parameter wires it, once',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);
      final key = effectKey(p.layer);
      await tester.tap(find.byKey(ValueKey<String>('graph-badge-E-$key')));
      await tester.pump();

      final from = tester.getCenter(socket('driver:$wiggle', 'value'));
      final to = tester.getCenter(socket(key, 'radius'));
      await tester.dragFrom(from, to - from);
      await tester.pump();

      final edges = p.layer.getGraph().wiring.edges;
      expect(edges, hasLength(1));
      expect(edges.single.from,
          BridgeOutputRef.driver(node: wiggle, port: 'value'));

      // One undo takes the wire off and leaves the driver — the exposure and
      // the wire were two gestures, so they are two steps.
      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.layer.getGraph().wiring.edges, isEmpty);
      expect(p.layer.getGraphDrivers(), hasLength(1));
    });

    /// The engine would refuse this, and its refusal is the backstop. The
    /// panel's own job is to decline it *first*, from the two port types it is
    /// already holding — so the gesture costs nothing at all.
    testWidgets('a drop between two different types is declined here',
        (tester) async {
      final p = withBlur();
      final cycle = seedDriver(p.layer, 'colour_cycle', const Offset(30, 300));
      await mount(tester, p);
      final key = effectKey(p.layer);
      await tester.tap(find.byKey(ValueKey<String>('graph-badge-E-$key')));
      await tester.pump();

      final from = tester.getCenter(socket('driver:$cycle', 'colour'));
      final to = tester.getCenter(socket(key, 'radius'));
      await tester.dragFrom(from, to - from);
      await tester.pump();

      expect(p.layer.getGraph().wiring.edges, isEmpty,
          reason: 'a colour does not fit a number, and nothing was committed');
    });

    /// The Source box's Matte output is the one feed the graph adds that the
    /// Matte row could never offer: the layer's own masked source alpha at
    /// that point in the chain (§1.4).
    testWidgets('the source matte wires into an effect matte', (tester) async {
      final p = withBlur();
      await mount(tester, p);
      final key = effectKey(p.layer);

      final from = tester.getCenter(socket('source', 'matte'));
      final to = tester.getCenter(socket(key, 'matte'));
      await tester.dragFrom(from, to - from);
      await tester.pump();

      final edges = p.layer.getGraph().wiring.edges;
      expect(edges, hasLength(1));
      expect(edges.single.from, const BridgeOutputRef.sourceMatte());
    });

    testWidgets('deleting a wired driver takes its wire with it, in one step',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);
      final key = effectKey(p.layer);
      await tester.tap(find.byKey(ValueKey<String>('graph-badge-E-$key')));
      await tester.pump();
      final from = tester.getCenter(socket('driver:$wiggle', 'value'));
      final to = tester.getCenter(socket(key, 'radius'));
      await tester.dragFrom(from, to - from);
      await tester.pump();
      expect(p.layer.getGraph().wiring.edges, hasLength(1));

      // Pick the driver box, then Delete.
      await tester.tapAt(tester.getCenter(
          find.byKey(ValueKey<String>('graph-node-driver:$wiggle'))));
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.delete);
      await tester.pump();

      expect(p.layer.getGraphDrivers(), isEmpty);
      expect(p.layer.getGraph().wiring.edges, isEmpty);

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.layer.getGraphDrivers(), hasLength(1));
      expect(p.layer.getGraph().wiring.edges, hasLength(1),
          reason: 'the box and its wire went together, so they come back '
              'together — one undo step');
    });

    /// Heal off is the "unplug it first" rule: a box that still carries a wire
    /// is left exactly where it is.
    testWidgets('with Heal off, a wired box is not deleted', (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);
      final key = effectKey(p.layer);
      await tester.tap(find.byKey(ValueKey<String>('graph-badge-E-$key')));
      await tester.pump();
      final from = tester.getCenter(socket('driver:$wiggle', 'value'));
      final to = tester.getCenter(socket(key, 'radius'));
      await tester.dragFrom(from, to - from);
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey<String>('graph-heal')));
      await tester.pump();
      await tester.tapAt(tester.getCenter(
          find.byKey(ValueKey<String>('graph-node-driver:$wiggle'))));
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.delete);
      await tester.pump();

      expect(p.layer.getGraphDrivers(), hasLength(1));
      expect(p.layer.getGraph().wiring.edges, hasLength(1));
    });

    /// Auto-wire: let a wire go over empty canvas, pick a node from the search,
    /// and the wire is on it when it lands.
    testWidgets('the Tab search adds a driver and auto-wire joins it',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);

      final from = tester.getCenter(socket('driver:$wiggle', 'value'));
      await tester.dragFrom(from, const Offset(220, 60));
      await tester.pump();
      expect(
          find.byKey(const ValueKey<String>('graph-search')), findsOneWidget);

      await tester
          .tap(find.byKey(const ValueKey<String>('graph-search-smooth')));
      await tester.pump();

      final graph = p.layer.getGraph();
      expect(p.layer.getGraphDrivers(), hasLength(2));
      expect(graph.wiring.edges, hasLength(1));
      expect(graph.wiring.edges.single.from,
          BridgeOutputRef.driver(node: wiggle, port: 'value'));
    });

    /// **The box and its wire arrive in one commit** (docs/impl/node-graph.md
    /// §3), which is what makes the whole gesture one undo step. The ports come
    /// off the catalogue entry, so the socket is known before the node is in
    /// the document and there is nothing left to do in a second op.
    testWidgets('the added driver and its auto-wire are one undo step',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);

      await tester.dragFrom(tester.getCenter(socket('driver:$wiggle', 'value')),
          const Offset(220, 60));
      await tester.pump();
      await tester
          .tap(find.byKey(const ValueKey<String>('graph-search-smooth')));
      await tester.pump();
      expect(p.layer.getGraphDrivers(), hasLength(2));
      expect(p.layer.getGraph().wiring.edges, hasLength(1));

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.layer.getGraphDrivers(), hasLength(1),
          reason: 'one undo takes the new box away');
      expect(p.layer.getGraph().wiring.edges, isEmpty,
          reason: 'and the wire with it — the two were one commit');
    });

    /// **The search shows what the wire in hand could land on** (WP3), so the
    /// footer's promise — "connects the dragged wire where it fits" — is true
    /// of every row it offers.
    testWidgets('the Tab search filters by the dragged wire\'s type',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      final cycle = seedDriver(p.layer, 'colour_cycle', const Offset(30, 440));
      await mount(tester, p);

      // A number in hand: the drivers that take a number are offered.
      await tester.dragFrom(tester.getCenter(socket('driver:$wiggle', 'value')),
          const Offset(240, 60));
      await tester.pump();
      expect(find.byKey(const ValueKey<String>('graph-search-smooth')),
          findsOneWidget);
      // A press anywhere puts the popover away.
      await tester.tapAt(const Offset(860, 560));
      await tester.pump();

      // A colour in hand: nothing in the v1 set takes one, and the list says so
      // rather than offering a row that could not connect.
      await tester.dragFrom(tester.getCenter(socket('driver:$cycle', 'colour')),
          const Offset(240, 60));
      await tester.pump();
      expect(
          find.byKey(const ValueKey<String>('graph-search')), findsOneWidget);
      expect(find.byKey(const ValueKey<String>('graph-search-smooth')),
          findsNothing);
      expect(find.byKey(const ValueKey<String>('graph-search-wiggle')),
          findsNothing);

      // Without a wire the whole family is back.
      await tester.tapAt(const Offset(860, 560));
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await tester.pump();
      expect(find.byKey(const ValueKey<String>('graph-search-smooth')),
          findsOneWidget);
    });

    /// **Removing a wired effect is one op** (K-471 §1.5). The stack's own
    /// removal prunes the graph inside the same commit, so the panel neither
    /// unplugs first nor leaves a dangling edge behind — and one undo brings
    /// the effect and its wiring back together.
    testWidgets('deleting a wired effect takes its wires with it, in one step',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);
      final key = effectKey(p.layer);
      await tester.tap(find.byKey(ValueKey<String>('graph-badge-E-$key')));
      await tester.pump();
      final from = tester.getCenter(socket('driver:$wiggle', 'value'));
      await tester.dragFrom(
          from, tester.getCenter(socket(key, 'radius')) - from);
      await tester.pump();
      expect(p.layer.getGraph().wiring.edges, hasLength(1));

      // Pick the *effect* box and delete it.
      await tester.tapAt(
          tester.getCenter(find.byKey(ValueKey<String>('graph-node-$key'))));
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.delete);
      await tester.pump();

      expect(p.layer.getEffects(), isEmpty);
      expect(p.layer.getGraph().wiring.edges, isEmpty,
          reason: 'the wire went with the box it named');
      expect(p.layer.getGraphDrivers(), hasLength(1),
          reason: 'the driver itself is not the stack\'s to remove');

      // The proof the prune is for: the next graph write is accepted.
      final moved = p.layer.getGraph();
      p.layer.setGraph(
        drivers: p.layer.getGraphDrivers(),
        wiring: BridgeGraphWiring(
          edges: moved.wiring.edges,
          layout: [
            BridgeNodePosition(node: BridgeNodeRef.driver(wiggle), x: 4, y: 4),
          ],
          exposed: moved.wiring.exposed,
        ),
      );

      p.state.project!.undo();
      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.layer.getEffects(), hasLength(1));
      expect(p.layer.getGraph().wiring.edges, hasLength(1),
          reason: 'one undo restores the effect and its wire together');
    });

    testWidgets('with Auto-wire off the node lands unwired', (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);

      await tester.tap(find.byKey(const ValueKey<String>('graph-auto-wire')));
      await tester.pump();
      await tester.dragFrom(tester.getCenter(socket('driver:$wiggle', 'value')),
          const Offset(220, 60));
      await tester.pump();
      await tester
          .tap(find.byKey(const ValueKey<String>('graph-search-smooth')));
      await tester.pump();

      expect(p.layer.getGraphDrivers(), hasLength(2));
      expect(p.layer.getGraph().wiring.edges, isEmpty);
    });

    /// **The stack view can never lie** (K-471 §1.1). The graph's image chain
    /// is derived from the effect list, so reordering the stack — in the
    /// Effect controls panel, in the Timeline, anywhere — moves the boxes.
    testWidgets('a reorder in the stack view moves the boxes', (tester) async {
      final p = withBlur();
      p.layer.addEffect(name: 'exposure');
      p.uiState.model.refresh();
      await tester.pump();
      await mount(tester, p);

      Iterable<String> chain() => p.layer
          .getGraph()
          .nodes
          .where((n) => n.matchName.isNotEmpty)
          .map((n) => n.matchName);
      expect(chain(), ['blur', 'exposure']);

      final blurBefore = tester.getRect(
          find.byKey(ValueKey<String>('graph-node-${effectKey(p.layer)}')));

      final stack = p.layer.getEffects();
      p.layer.reorderEffect(effect: stack.first, newIndex: 1);
      p.uiState.model.refresh();
      await tester.pump();

      expect(chain(), ['exposure', 'blur']);
      final blurAfter = tester.getRect(
          find.byKey(ValueKey<String>('graph-node-${effectKey(p.layer)}')));
      expect(blurAfter.left, greaterThan(blurBefore.left),
          reason: 'the blur is second in the list now, so second along the '
              'chain — the graph has no second opinion about the order');
    });

    /// Bypass is the existing `enabled` flag on both kinds of box; the border
    /// is dashed either way, and the op is the one that kind already had.
    testWidgets('the B badge bypasses an effect and a driver alike',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);
      final key = effectKey(p.layer);

      await tester.tap(find.byKey(ValueKey<String>('graph-badge-B-$key')));
      await tester.pump();
      expect(p.layer.getEffects().single.enabled(), isFalse);

      await tester
          .tap(find.byKey(ValueKey<String>('graph-badge-B-driver:$wiggle')));
      await tester.pump();
      expect(p.layer.getGraphDrivers().single.enabled(), isFalse);
    });

    /// A box's position is document data: it persists, it travels, and a drag
    /// stages it and commits once (K-344).
    testWidgets('dragging a box commits its position once', (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);

      final box = find.byKey(ValueKey<String>('graph-node-driver:$wiggle'));
      await tester.dragFrom(tester.getCenter(box), const Offset(40, 20));
      await tester.pump();

      final placed = p.layer
          .getGraph()
          .wiring
          .layout
          .firstWhere((l) => l.node == BridgeNodeRef.driver(wiggle));
      expect(placed.x, 70);
      expect(placed.y, 320);
    });
  });
}
