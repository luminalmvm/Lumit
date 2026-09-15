// What the node graph composition's canvas *does*: the gestures, and what
// each one commits (docs/impl/node-graph-comp.md §4.2).
//
// Every document operation here is genuine (see frb_test_support.dart), so a
// claim about "one undo step" is a claim about the real journal. The two rules
// these tests exist to hold are the layer canvas's own:
//
//  * **one gesture, one undo step**. A wire, a delete, a twirl and a tick each
//    come back whole with a single undo; and
//  * **the panel declines what it can decline itself**. A number dropped on an
//    image socket never reaches the engine, because both types are in the read
//    model already.

import 'dart:typed_data' show Float64List;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/panels/node_panel.dart';
import 'package:lumit_flutter/src/rust/api/comp_graph.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/lib.dart' show F64Array4;
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/drag_payloads.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:uuid/uuid.dart';

import 'frb_test_support.dart';

/// The console's category-kicker key stem, so the strip's order can be read
/// off the tree.
const String _cat = 'fx-console-cat-';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initEngineForTests);

  group('Node graph canvas (frb)', () {
    /// A project holding one node graph, fronted, and one comp beside it.
    ({LumitState state, LumitUiState uiState, CompositionReference graph})
        withGraph() {
      final p = freshProject();
      final graph = p.state.project!.newNodeGraph(name: 'Graph');
      p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(graph);
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, graph: graph);
    }

    Future<void> mount(WidgetTester tester, dynamic p,
        {Widget? child, Size size = const Size(900, 600)}) async {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: child ?? const GraphPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: size,
      ));
      await tester.pump();
    }

    /// The wiring as it stands with one change folded in: how a test seeds a
    /// graph without going through a gesture.
    BridgeCompWiring wiringWith(
      BridgeCompWiring w, {
      List<BridgeReadNode>? reads,
      List<BridgeInputNode>? inputs,
      List<BridgeCompNodePosition>? layout,
    }) =>
        BridgeCompWiring(
          reads: reads ?? w.reads,
          inputs: inputs ?? w.inputs,
          output: w.output,
          edges: w.edges,
          layout: layout ?? w.layout,
          exposed: w.exposed,
          groups: w.groups,
        );

    UuidValue fresh() => UuidValue.fromString(const Uuid().v4());

    /// A Read of a project item, placed, from outside the panel.
    UuidValue seedReadOf(
        CompositionReference graph, UuidValue item, Offset at) {
      final id = fresh();
      final w = graph.getNodeGraph().wiring;
      graph.setNodeGraph(
        instances: graph.getNodeGraphInstances(),
        wiring: wiringWith(
          w,
          reads: [...w.reads, BridgeReadNode(id: id, item: item)],
          layout: [
            ...w.layout,
            BridgeCompNodePosition(node: id, x: at.dx, y: at.dy),
          ],
        ),
      );
      return id;
    }

    /// A Read of an imported clip, placed, from outside the panel.
    UuidValue seedRead(
            CompositionReference graph, LumitState state, Offset at) =>
        seedReadOf(
            graph,
            state.project!
                .importFootage(path: 'C:/clips/shot.mov')
                .internalid,
            at);

    /// A catalogue box, placed, from outside the panel. [bound] is the comp a
    /// nested Node graph box applies, which that one entry requires.
    UuidValue seedFx(CompositionReference graph, String name, Offset at,
        {CompositionReference? bound}) {
      final made = graph.newGraphInstance(name: name, graph: bound);
      final id = made.id();
      final w = graph.getNodeGraph().wiring;
      graph.setNodeGraph(
        instances: [...graph.getNodeGraphInstances(), made],
        wiring: wiringWith(w, layout: [
          ...w.layout,
          BridgeCompNodePosition(node: id, x: at.dx, y: at.dy),
        ]),
      );
      return id;
    }

    /// An Input box, placed, from outside the panel.
    UuidValue seedInput(CompositionReference graph, Offset at) {
      final id = fresh();
      final w = graph.getNodeGraph().wiring;
      graph.setNodeGraph(
        instances: graph.getNodeGraphInstances(),
        wiring: wiringWith(
          w,
          inputs: [
            ...w.inputs,
            BridgeInputNode(
              id: id,
              input: BridgeGraphInput(
                id: 'amount',
                label: 'Amount',
                kind: BridgeInputKind.number,
                default_: F64Array4(Float64List(4)),
                min: 0,
                max: 100,
                unit: BridgeUnit.raw,
              ),
            ),
          ],
          layout: [
            ...w.layout,
            BridgeCompNodePosition(node: id, x: at.dx, y: at.dy),
          ],
        ),
      );
      return id;
    }

    Finder socket(UuidValue node, String port) =>
        find.byKey(ValueKey<String>('graph-socket-node:$node-$port'));
    Finder card(UuidValue node) =>
        find.byKey(ValueKey<String>('graph-node-node:$node'));

    Future<void> wire(WidgetTester tester, UuidValue from, String out,
        UuidValue to, String into) async {
      final a = tester.getCenter(socket(from, out));
      final b = tester.getCenter(socket(to, into));
      await tester.dragFrom(a, b - a);
      await tester.pump();
    }

    /// The console's own door, and one row of it run by name.
    Future<void> addFromConsole(WidgetTester tester, LumitUiState ui,
        String query, String label) async {
      ui.activePane.value = Panel.graph.pane();
      expect(ui.consoleClaim!(), isTrue);
      await tester.pump();
      await tester.enterText(
          find.byKey(const ValueKey('fx-console-query')), query);
      await tester.pump();
      await tester.tap(find.byKey(ValueKey<String>('fx-console-item-$label')));
      await tester.pump();
    }

    testWidgets('a node graph draws the comp canvas, a layer comp does not',
        (tester) async {
      final p = withGraph();
      await mount(tester, p);
      expect(find.byKey(const ValueKey<String>('comp-graph-canvas')),
          findsOneWidget);
      expect(find.byKey(const ValueKey<String>('graph-canvas')), findsNothing);

      // A comp with layers is the layer's own graph again.
      final scene = p.state.project!.newComposition(name: 'Layers');
      final layer = scene.addSolidLayer();
      p.uiState
        ..setSelectedComp(scene)
        ..selectedLayer.value = layer;
      p.uiState.model.refresh();
      await tester.pump();
      expect(find.byKey(const ValueKey<String>('comp-graph-canvas')),
          findsNothing);
      expect(
          find.byKey(const ValueKey<String>('graph-canvas')), findsOneWidget);
    });

    testWidgets('the Output box draws, with its one socket', (tester) async {
      final p = withGraph();
      await mount(tester, p);
      final out = p.graph.getNodeGraph().wiring.output;
      expect(card(out), findsOneWidget);
      expect(socket(out, 'input'), findsOneWidget);
    });

    /// The console's own door: Ctrl+Space over the canvas, then a row.
    testWidgets('the console adds a Read, and one undo takes it away',
        (tester) async {
      final p = withGraph();
      await mount(tester, p);
      p.uiState.activePane.value = Panel.graph.pane();
      expect(p.uiState.consoleClaim, isNotNull);
      expect(p.uiState.consoleClaim!(), isTrue);
      await tester.pump();

      expect(
          find.byKey(const ValueKey<String>('fx-console-bar')), findsOneWidget);
      // The project's other composition is offered as a Read box; this graph
      // itself is not, since a box reading its own comp is a loop.
      expect(find.byKey(const ValueKey<String>('fx-console-item-Graph')),
          findsNothing);

      // And the list is in §4.2's order: the project's items, Input, the
      // effects, then the boxes only a graph can hold.
      final kickers = [
        for (final element in find
            .byWidgetPredicate((w) =>
                w.key is ValueKey<String> &&
                (w.key! as ValueKey<String>).value.startsWith(_cat))
            .evaluate())
          (element.widget.key! as ValueKey<String>)
              .value
              .substring(_cat.length),
      ];
      expect(kickers.take(3).toList(),
          ['*all', l10n.projectTypeComposition, l10n.graphInput]);
      expect(kickers.indexOf(l10n.fxCompositing),
          greaterThan(kickers.indexOf(l10n.fxBlurAndSharpen)),
          reason: 'Merge and Switch come after the effects');
      expect(kickers.indexOf(l10n.fxControls),
          greaterThan(kickers.indexOf(l10n.fxBlurAndSharpen)),
          reason: 'the drivers file under Controls, at the end of the effects');
      // Reached by typing, as a hand reaches a row past the list's fold.
      await tester.enterText(
          find.byKey(const ValueKey('fx-console-query')), 'Scene');
      await tester.pump();
      await tester
          .tap(find.byKey(const ValueKey<String>('fx-console-item-Scene')));
      await tester.pump();

      expect(p.graph.getNodeGraph().wiring.reads, hasLength(1));
      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.graph.getNodeGraph().wiring.reads, isEmpty,
          reason: 'one gesture, one undo step');
    });

    /// One picture into two boxes and back through a Merge: the fork a layer
    /// stack cannot draw, and what this canvas exists for.
    testWidgets('an image forks into two boxes and merges', (tester) async {
      final p = withGraph();
      final read = seedRead(p.graph, p.state, const Offset(20, 40));
      final blur = seedFx(p.graph, 'blur', const Offset(260, 20));
      final glow = seedFx(p.graph, 'glow', const Offset(260, 240));
      final merge = seedFx(p.graph, 'merge', const Offset(520, 120));
      p.uiState.model.refresh();
      await mount(tester, p);
      final out = p.graph.getNodeGraph().wiring.output;

      await wire(tester, read, 'output', blur, 'input');
      await wire(tester, read, 'output', glow, 'input');
      await wire(tester, blur, 'output', merge, 'input');
      await wire(tester, glow, 'output', merge, 'background');
      await wire(tester, merge, 'output', out, 'input');

      final edges = p.graph.getNodeGraph().wiring.edges;
      expect(edges, hasLength(5),
          reason: 'one output feeds any number of inputs');
      expect(edges.where((e) => e.from == read), hasLength(2),
          reason: 'the fork is two wires out of one socket');
      // And they are drawn: every wire has both its ends on the canvas.
      expect(find.byKey(const ValueKey<String>('comp-graph-canvas')),
          findsOneWidget);
      for (final edge in edges) {
        expect(card(edge.from), findsOneWidget);
        expect(card(edge.to), findsOneWidget);
      }
    });

    testWidgets('a number dropped on an image socket is declined here',
        (tester) async {
      final p = withGraph();
      final blur = seedFx(p.graph, 'blur', const Offset(300, 40));
      final wiggle = seedFx(p.graph, 'wiggle', const Offset(20, 280));
      p.uiState.model.refresh();
      await mount(tester, p);
      final was = p.graph.documentRevision();

      // Out of a value socket into an image one: the direction is right, so
      // the type is the only thing that can refuse it.
      final from = tester.getCenter(socket(wiggle, 'value'));
      final to = tester.getCenter(socket(blur, 'input'));
      await tester.dragFrom(from, to - from);
      await tester.pump();
      expect(p.graph.getNodeGraph().wiring.edges, isEmpty,
          reason: 'a number does not fit a picture, and nothing was committed');
      expect(p.graph.documentRevision(), was,
          reason: 'nothing crossed the bridge');

      // The same wire onto a socket the number does fit: one op.
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-node:$blur')));
      await tester.pump();
      final twirled = p.graph.documentRevision();
      final radius = tester.getCenter(socket(blur, 'radius'));
      await tester.dragFrom(from, radius - from);
      await tester.pump();

      final edges = p.graph.getNodeGraph().wiring.edges;
      expect(edges, hasLength(1));
      expect(edges.single.toPort, 'radius');
      expect(p.graph.documentRevision(), twirled + BigInt.one,
          reason: 'one gesture, one op');
    });

    testWidgets('a composition dragged onto the canvas becomes a Read',
        (tester) async {
      final p = withGraph();
      final scene = p.state.project!.newComposition(name: 'Dropped');
      await mount(
        tester,
        p,
        child: Column(children: [
          Draggable<CompDragData>(
            data: CompDragData(scene, 'Dropped'),
            hitTestBehavior: HitTestBehavior.opaque,
            feedback: const SizedBox(width: 8, height: 8),
            child: const SizedBox(
                key: ValueKey<String>('drag-source'), width: 60, height: 20),
          ),
          const Expanded(child: GraphPanelFrb()),
        ]),
      );

      // The source is a bare box, so the finder is not itself the hit target;
      // the drag lands on the canvas all the same.
      await tester.drag(find.byKey(const ValueKey<String>('drag-source')),
          const Offset(200, 300),
          warnIfMissed: false);
      await tester.pumpAndSettle();

      final reads = p.graph.getNodeGraph().wiring.reads;
      expect(reads, hasLength(1));
      expect(reads.single.item, scene.internalid);
    });

    /// This graph dropped on itself is nothing at all: a box naming the comp
    /// it is in can only ever degrade to a passthrough.
    testWidgets('the fronted graph dropped on its own canvas is declined',
        (tester) async {
      final p = withGraph();
      await mount(
        tester,
        p,
        child: Column(children: [
          Draggable<CompDragData>(
            data: CompDragData(p.graph, 'Graph'),
            hitTestBehavior: HitTestBehavior.opaque,
            feedback: const SizedBox(width: 8, height: 8),
            child: const SizedBox(
                key: ValueKey<String>('drag-source'), width: 60, height: 20),
          ),
          const Expanded(child: GraphPanelFrb()),
        ]),
      );
      final was = p.graph.documentRevision();

      await tester.drag(find.byKey(const ValueKey<String>('drag-source')),
          const Offset(200, 300),
          warnIfMissed: false);
      await tester.pumpAndSettle();

      expect(p.graph.getNodeGraph().nodes, hasLength(1),
          reason: 'the Output box, and nothing else');
      expect(p.graph.documentRevision(), was);
    });

    testWidgets('deleting a wired box heals the gap, in one step',
        (tester) async {
      final p = withGraph();
      final read = seedRead(p.graph, p.state, const Offset(20, 40));
      final blur = seedFx(p.graph, 'blur', const Offset(300, 40));
      p.uiState.model.refresh();
      await mount(tester, p);
      final out = p.graph.getNodeGraph().wiring.output;

      await wire(tester, read, 'output', blur, 'input');
      await wire(tester, blur, 'output', out, 'input');
      expect(p.graph.getNodeGraph().wiring.edges, hasLength(2));

      p.uiState.activePane.value = Panel.graph.pane();
      await tester.tapAt(tester.getCenter(card(blur)));
      await tester.pump();
      expect(p.uiState.deleteClaim!(), isTrue);
      await tester.pump();

      final edges = p.graph.getNodeGraph().wiring.edges;
      expect(edges, hasLength(1), reason: 'Heal joined the two ends');
      expect(edges.single.from, read);
      expect(edges.single.to, out);

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.graph.getNodeGraph().wiring.edges, hasLength(2),
          reason: 'one gesture, one undo step');
    });

    testWidgets('the Output box is never deleted', (tester) async {
      final p = withGraph();
      await mount(tester, p);
      p.uiState.activePane.value = Panel.graph.pane();
      final out = p.graph.getNodeGraph().wiring.output;
      await tester.tapAt(tester.getCenter(card(out)));
      await tester.pump();
      expect(p.uiState.deleteClaim!(), isFalse,
          reason: 'a graph always has exactly one Output');
      await tester.pump();
      expect(card(out), findsOneWidget);
    });

    testWidgets('the twirl and the tick each commit', (tester) async {
      final p = withGraph();
      final blur = seedFx(p.graph, 'blur', const Offset(60, 40));
      p.uiState.model.refresh();
      await mount(tester, p);

      expect(socket(blur, 'radius'), findsNothing);
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-node:$blur')));
      await tester.pump();
      expect(socket(blur, 'radius'), findsOneWidget);
      expect(p.graph.getNodeGraph().wiring.exposed, hasLength(1));

      await tester.tap(find.byKey(ValueKey<String>('graph-enable-node:$blur')));
      await tester.pump();
      expect(
        p.graph.getNodeGraph().nodes.firstWhere((n) => n.id == blur).enabled,
        isFalse,
      );

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(
        p.graph.getNodeGraph().nodes.firstWhere((n) => n.id == blur).enabled,
        isTrue,
        reason: 'one gesture, one undo step',
      );
    });

    testWidgets('a picked box publishes itself and the Node panel follows it',
        (tester) async {
      final p = withGraph();
      final blur = seedFx(p.graph, 'blur', const Offset(60, 40));
      p.uiState.model.refresh();
      await mount(
        tester,
        p,
        child: const Row(children: [
          SizedBox(width: 600, child: GraphPanelFrb()),
          Expanded(child: NodePanelFrb()),
        ]),
      );

      await tester.tapAt(tester.getCenter(card(blur)));
      await tester.pump();
      expect(p.uiState.compGraphNode.value?.id, blur);
      expect(p.uiState.compGraphNode.value?.picture, isTrue,
          reason: 'a box that makes a picture offers the Viewer chip');
      expect(find.byKey(ValueKey<String>('node-row-$blur-radius')),
          findsOneWidget);

      // And a value typed on that row commits through the graph's own op.
      final field = find.byKey(ValueKey<String>('fx-float-$blur-radius'));
      await tester.tap(field);
      await tester.pump();
      await tester.enterText(find.descendant(of: field, matching: find.byType(EditableText)), '12');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();

      final values = p.graph
          .getNodeGraphInstances()
          .firstWhere((i) => i.id() == blur)
          .getInfo()
          .values;
      expect(
        values.where((v) => v.id == 'radius').map((v) => v.value).single,
        isA<BridgeEffectValue_Float>().having(
            (v) => stillValue(v.field0), 'radius', closeTo(12, 0.001)),
      );
    });

    /// Auto-wire: a box added while one is picked lands after it, and takes
    /// over whatever that box was feeding.
    testWidgets('a box added while one is picked is wired after it',
        (tester) async {
      final p = withGraph();
      final read = seedRead(p.graph, p.state, const Offset(20, 40));
      p.uiState.model.refresh();
      await mount(tester, p);
      final out = p.graph.getNodeGraph().wiring.output;
      await wire(tester, read, 'output', out, 'input');

      p.uiState.activePane.value = Panel.graph.pane();
      await tester.tapAt(tester.getCenter(card(read)));
      await tester.pump();
      expect(p.uiState.consoleClaim!(), isTrue);
      await tester.pump();
      await tester.enterText(
          find.byKey(const ValueKey('fx-console-query')), 'exposure');
      await tester.pump();
      await tester
          .tap(find.byKey(const ValueKey<String>('fx-console-item-Exposure')));
      await tester.pump();

      final edges = p.graph.getNodeGraph().wiring.edges;
      final added = p.uiState.compGraphNode.value!.id;
      expect(edges, hasLength(2));
      expect(
        edges.where((e) => e.from == read && e.to == added),
        hasLength(1),
        reason: 'the new box sits after the one that was picked',
      );
      expect(
        edges.where((e) => e.from == added && e.to == out),
        hasLength(1),
        reason: 'and it took over feeding the Output',
      );
    });

    /// N7: a loose box let go over a wire falls into it.
    testWidgets('a box dropped on a wire is spliced in', (tester) async {
      final p = withGraph();
      final read = seedRead(p.graph, p.state, const Offset(40, 40));
      final blur = seedFx(p.graph, 'blur', const Offset(40, 300));
      p.uiState.model.refresh();
      await mount(tester, p);
      final out = p.graph.getNodeGraph().wiring.output;
      await wire(tester, read, 'output', out, 'input');
      expect(p.graph.getNodeGraph().wiring.edges, hasLength(1));

      // Onto the middle of the one wire on the canvas.
      final from = tester.getCenter(socket(read, 'output'));
      final to = tester.getCenter(socket(out, 'input'));
      final middle = (from + to) / 2;
      final box = tester.getCenter(card(blur));
      await tester.dragFrom(box, middle - box);
      await tester.pump();

      final edges = p.graph.getNodeGraph().wiring.edges;
      expect(edges, hasLength(2), reason: 'the wire split in two');
      expect(edges.where((e) => e.from == read && e.to == blur), hasLength(1));
      expect(edges.where((e) => e.from == blur && e.to == out), hasLength(1));
    });

    /// The one change to the layer canvas: a Node graph box on a layer opens
    /// the composition it applies, as a Read box of a comp does here.
    testWidgets('double-clicking a Node graph box opens its comp',
        (tester) async {
      final p = withGraph();
      final scene = p.state.project!.newComposition(name: 'Host');
      final layer = scene.addSolidLayer();
      layer.addNodeGraphEffect(graph: p.graph);
      p.uiState
        ..setSelectedComp(scene)
        ..selectedLayer.value = layer;
      p.uiState.model.refresh();
      await mount(tester, p);

      final node = layer
          .getGraph()
          .nodes
          .firstWhere((n) => n.matchName == 'node_graph');
      final at = tester.getCenter(
          find.byKey(ValueKey<String>('graph-node-${graphNodeKey(node.node)}')));
      await tester.tapAt(at);
      await tester.pump(const Duration(milliseconds: 40));
      await tester.tapAt(at);
      await tester.pump();

      expect(p.uiState.selectedComp?.internalid, p.graph.internalid,
          reason: 'the box with an inside opens it, as a precomp does');
    });

    testWidgets('an Input box draws its five facts, and the label commits',
        (tester) async {
      final p = withGraph();
      final input = seedInput(p.graph, const Offset(60, 40));
      p.uiState.model.refresh();
      await mount(
        tester,
        p,
        child: const Row(children: [
          SizedBox(width: 600, child: GraphPanelFrb()),
          Expanded(child: NodePanelFrb()),
        ]),
      );

      await tester.tapAt(tester.getCenter(card(input)));
      await tester.pump();
      expect(p.uiState.compGraphNode.value?.id, input);
      expect(p.uiState.compGraphNode.value?.picture, isFalse,
          reason: 'a value Input makes no picture, so it offers no chip');
      expect(find.byKey(const ValueKey('node-input-kind')), findsOneWidget);
      expect(find.byKey(const ValueKey('node-input-min')), findsOneWidget);
      expect(find.byKey(const ValueKey('node-input-max')), findsOneWidget);
      expect(find.byKey(const ValueKey('node-input-unit')), findsOneWidget);

      await tester.enterText(
          find.byKey(const ValueKey('node-input-label')), 'Wobble');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(
        p.graph.getNodeGraph().wiring.inputs.single.input.label,
        'Wobble',
      );

      // An undo puts the label back, and the field with it: stale text here
      // was written back by the next lost-focus submit.
      FocusManager.instance.primaryFocus?.unfocus();
      await tester.pump();
      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(
        tester
            .widget<HouseTextField>(
                find.byKey(const ValueKey('node-input-label')))
            .controller
            .text,
        'Amount',
      );
    });

    /// A drag on a form well is **one** op: the live half shows where it has
    /// got to, and the release commits.
    testWidgets('a drag on the Minimum well leaves one history step',
        (tester) async {
      final p = withGraph();
      final input = seedInput(p.graph, const Offset(60, 40));
      p.uiState.model.refresh();
      await mount(
        tester,
        p,
        child: const Row(children: [
          SizedBox(width: 600, child: GraphPanelFrb()),
          Expanded(child: NodePanelFrb()),
        ]),
      );
      await tester.tapAt(tester.getCenter(card(input)));
      await tester.pump();

      final was = p.graph.documentRevision();
      final well = find.byKey(const ValueKey('node-input-min'));
      final drag = await tester.startGesture(tester.getCenter(well));
      for (var i = 0; i < 8; i++) {
        await drag.moveBy(const Offset(6, 0));
        await tester.pump();
      }
      await drag.up();
      await tester.pump();

      expect(p.graph.documentRevision(), was + BigInt.one,
          reason: 'one drag, one op');
      expect(p.graph.getNodeGraph().wiring.inputs.single.input.min,
          isNot(0));
    });

    /// A Read box's own face: what it brings in, and the way into a comp.
    testWidgets('a Read of a comp draws its kind and opens it', (tester) async {
      final p = withGraph();
      final inner = p.state.project!.newComposition(name: 'Inner');
      final read = seedReadOf(p.graph, inner.internalid, const Offset(60, 40));
      p.uiState.model.refresh();
      await mount(
        tester,
        p,
        child: const Row(children: [
          SizedBox(width: 600, child: GraphPanelFrb()),
          Expanded(child: NodePanelFrb()),
        ]),
      );

      await tester.tapAt(tester.getCenter(card(read)));
      await tester.pump();
      expect(
        tester
            .widget<Text>(find.byKey(const ValueKey('node-read-kind')))
            .data,
        l10n.projectTypeComposition,
      );
      await tester.tap(find.byKey(const ValueKey('node-read-open')));
      await tester.pump();
      expect(p.uiState.selectedComp?.internalid, inner.internalid,
          reason: 'Open fronts the comp the box reads');
    });

    /// A layer-reference row is a **socket** in a node graph (§4.3): there are
    /// no layers to pick from, so the row draws its dash instead of a picker.
    testWidgets('a Layer row in a node graph draws no picker', (tester) async {
      final p = withGraph();
      final wrap = seedFx(p.graph, 'light_wrap', const Offset(60, 40));
      p.uiState.model.refresh();
      await mount(
        tester,
        p,
        child: const Row(children: [
          SizedBox(width: 600, child: GraphPanelFrb()),
          Expanded(child: NodePanelFrb()),
        ]),
      );

      await tester.tapAt(tester.getCenter(card(wrap)));
      await tester.pump();
      expect(find.byKey(ValueKey<String>('node-row-$wrap-background')),
          findsOneWidget);
      expect(find.byKey(ValueKey<String>('fx-layer-$wrap-background')),
          findsNothing,
          reason: 'the wire is the reference here');
    });

    /// A nested graph's Inputs are rows on the box that applies it (§1.5), so
    /// the panel draws the derived half of the list as well as the declared.
    testWidgets('a nested Node graph box draws the graph\'s Input row',
        (tester) async {
      final p = withGraph();
      final inner = p.state.project!.newNodeGraph(name: 'Inner');
      seedInput(inner, const Offset(20, 20));
      final nested =
          seedFx(p.graph, 'node_graph', const Offset(60, 40), bound: inner);
      p.uiState.model.refresh();
      await mount(
        tester,
        p,
        child: const Row(children: [
          SizedBox(width: 600, child: GraphPanelFrb()),
          Expanded(child: NodePanelFrb()),
        ]),
      );

      await tester.tapAt(tester.getCenter(card(nested)));
      await tester.pump();
      expect(find.byKey(ValueKey<String>('node-row-$nested-amount')),
          findsOneWidget,
          reason: "the graph's own Input is a row on the box");
    });

    /// **Auto-wire off the anchor's own socket.** The Output box, a value
    /// Input and every driver have no `output`, and a wire naming a socket a
    /// box has not got is refused, which used to lose the whole add.
    testWidgets('a box added with the Output picked feeds the Output',
        (tester) async {
      final p = withGraph();
      await mount(tester, p);
      final out = p.graph.getNodeGraph().wiring.output;
      await tester.tapAt(tester.getCenter(card(out)));
      await tester.pump();
      await addFromConsole(tester, p.uiState, 'Gaussian', 'Gaussian blur');

      final added = p.uiState.compGraphNode.value!.id;
      expect(card(added), findsOneWidget, reason: 'the box landed');
      expect(
        p.graph.getNodeGraph().wiring.edges.where(
            (e) => e.from == added && e.to == out && e.toPort == 'input'),
        hasLength(1),
      );
    });

    testWidgets('a box added with a driver picked lands unwired',
        (tester) async {
      final p = withGraph();
      final wiggle = seedFx(p.graph, 'wiggle', const Offset(40, 260));
      p.uiState.model.refresh();
      await mount(tester, p);
      await tester.tapAt(tester.getCenter(card(wiggle)));
      await tester.pump();
      await addFromConsole(tester, p.uiState, 'Gaussian', 'Gaussian blur');

      final added = p.uiState.compGraphNode.value!.id;
      expect(card(added), findsOneWidget,
          reason: 'a Wiggle has no picture to hand on, so the box still lands');
      expect(p.graph.getNodeGraph().wiring.edges, isEmpty);
    });

    /// A Switch grows a spare socket one beyond the last wired (§1.4).
    testWidgets('a Switch grows its spare socket as one is wired',
        (tester) async {
      final p = withGraph();
      final read = seedRead(p.graph, p.state, const Offset(20, 40));
      final chooser = seedFx(p.graph, 'switch', const Offset(300, 40));
      p.uiState.model.refresh();
      await mount(tester, p);
      expect(socket(chooser, 'in0'), findsOneWidget);
      expect(socket(chooser, 'in1'), findsNothing);

      await wire(tester, read, 'output', chooser, 'in0');
      expect(socket(chooser, 'in1'), findsOneWidget,
          reason: 'one beyond the last wired socket');
    });

    testWidgets('a Switch added with a Read picked is wired on in0',
        (tester) async {
      final p = withGraph();
      final read = seedRead(p.graph, p.state, const Offset(20, 40));
      p.uiState.model.refresh();
      await mount(tester, p);
      await tester.tapAt(tester.getCenter(card(read)));
      await tester.pump();
      await addFromConsole(tester, p.uiState, 'Switch', 'Switch');

      final added = p.uiState.compGraphNode.value!.id;
      expect(card(added), findsOneWidget, reason: 'the box landed');
      final box =
          p.graph.getNodeGraph().nodes.firstWhere((n) => n.id == added);
      expect(box.inputs.map((s) => s.id),
          containsAll(<String>['in0', 'index']));
      expect(
        p.graph.getNodeGraph().wiring.edges.where(
            (e) => e.from == read && e.to == added && e.toPort == 'in0'),
        hasLength(1),
        reason: 'a Switch takes its first picture on in0, not on an input',
      );
    });
  }, skip: !engineAvailable);
}
