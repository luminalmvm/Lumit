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

import 'package:flutter/gestures.dart'
    show kDoubleTapMinTime, kSecondaryMouseButton;
import 'package:flutter/services.dart' show LogicalKeyboardKey;
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/comp_graph_panel.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/panels/node_panel.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/shell/fx_console_frb.dart'
    show lastKnownPointerPosition;
import 'package:lumit_flutter/shell/menu_bar_frb.dart'
    show copySelectionFrb, pasteSelectionFrb;
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

    // --- What an open box draws (§4.2): the Node panel's row on every
    // parameter, its socket level with it, and the picture's sockets as they
    // were.

    /// Where a box sits on the canvas, from the document.
    Offset placeOf(CompositionReference graph, UuidValue id) {
      final p = graph.getNodeGraph().wiring.layout.firstWhere((l) => l.node == id);
      return Offset(p.x, p.y);
    }

    bool picked(WidgetTester tester, UuidValue id, {int at = 0}) => tester
        .widgetList<GraphNodeCard>(find.byWidgetPredicate(
            (w) => w is GraphNodeCard && w.box.key == compNodeKey(id)))
        .elementAt(at)
        .selected;

    Finder twirl(UuidValue node) =>
        find.byKey(ValueKey<String>('graph-twirl-node:$node'));
    Finder rowOn(UuidValue node, String param) =>
        find.byKey(ValueKey<String>('graph-row-node:$node-$param'));
    Finder fieldOn(UuidValue node, String param) => find.descendant(
        of: card(node),
        matching: find.byKey(ValueKey<String>('fx-float-$node-$param')));
    double radiusOf(CompositionReference graph, UuidValue blur) {
      final values = graph
          .getNodeGraphInstances()
          .firstWhere((i) => i.id() == blur)
          .getInfo()
          .values;
      final value =
          values.where((v) => v.id == 'radius').map((v) => v.value).single;
      return stillValue((value as BridgeEffectValue_Float).field0);
    }

    testWidgets("a row's control edits the parameter in one op, undone in one",
        (tester) async {
      final p = withGraph();
      final blur = seedFx(p.graph, 'blur', const Offset(60, 40));
      p.uiState.model.refresh();
      await mount(tester, p);
      await tester.tap(twirl(blur));
      await tester.pump();

      final was = p.graph.documentRevision();
      final field = fieldOn(blur, 'radius');
      await tester.tap(field);
      await tester.pump();
      await tester.enterText(
          find.descendant(of: field, matching: find.byType(EditableText)),
          '12');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(radiusOf(p.graph, blur), closeTo(12, 0.001));
      expect(p.graph.documentRevision(), was + BigInt.one, reason: 'one op');

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(radiusOf(p.graph, blur), closeTo(30, 0.001),
          reason: 'one undo step');

      // Backspace typed into the well is the well's, not the canvas's Delete.
      await tester.tap(fieldOn(blur, 'radius'));
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
      await tester.pump();
      expect(card(blur), findsOneWidget);
    });

    testWidgets('a drag on a box control commits once and moves the box nowhere',
        (tester) async {
      final p = withGraph();
      final blur = seedFx(p.graph, 'blur', const Offset(60, 40));
      p.uiState.model.refresh();
      await mount(tester, p);
      await tester.tap(twirl(blur));
      await tester.pump();

      final was = p.graph.documentRevision();
      final drag =
          await tester.startGesture(tester.getCenter(fieldOn(blur, 'radius')));
      for (var i = 0; i < 8; i++) {
        await drag.moveBy(const Offset(6, 0));
        await tester.pump();
      }
      await drag.up();
      await tester.pump();

      expect(p.graph.documentRevision(), was + BigInt.one,
          reason: 'one drag, one op');
      expect(radiusOf(p.graph, blur), isNot(closeTo(30, 0.001)));
      expect(placeOf(p.graph, blur), const Offset(60, 40),
          reason: 'the value moved, the box did not');
    });

    // --- A box lists what the Effect controls panel lists. The rows are
    // derived by that panel's own rules, so a box and the panel can never
    // disagree about what an effect is showing.

    /// The Node graph box's one Action. It drew a button that did nothing;
    /// it is the canvas's own way in (§4.2).
    testWidgets("a Node graph box's Open graph action goes in", (tester) async {
      final p = withGraph();
      final inner = p.state.project!.newNodeGraph(name: 'Inner');
      final nested =
          seedFx(p.graph, 'node_graph', const Offset(60, 40), bound: inner);
      p.uiState.model.refresh();
      await mount(tester, p);
      await tester.tap(twirl(nested));
      await tester.pump();

      await tester.tap(find.descendant(
          of: rowOn(nested, 'open'),
          matching: find.byKey(ValueKey<String>('fx-action-$nested-open'))));
      await tester.pump();
      expect(p.uiState.selectedComp?.internalid, inner.internalid,
          reason: 'the button opens the graph the box applies');
    });

    /// Type [source] into the shader editor that [edit] opens, and apply it.
    Future<void> editShader(
        WidgetTester tester, Finder edit, String source) async {
      await tester.tap(edit);
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey<String>('shader-editor-code')), source);
      await tester.pump(const Duration(milliseconds: 500));
      await tester
          .tap(find.byKey(const ValueKey<String>('shader-editor-apply')));
      await tester.pumpAndSettle();
    }

    String? shaderOn(CompositionReference graph, UuidValue node) => graph
        .getNodeGraphInstances()
        .firstWhere((i) => i.id() == node)
        .shaderSource();

    const shader = 'fn shade(uv: vec2<f32>) -> vec4<f32> {\n'
        '    return lumit_sample(uv);\n'
        '}\n';

    /// A Custom shader box drew Edit shader and did nothing when it was
    /// pressed. It opens the editor, and Apply is one op on the graph.
    testWidgets("a Custom shader box's Edit shader applies to the graph",
        (tester) async {
      final p = withGraph();
      final box = seedFx(p.graph, 'custom_shader', const Offset(60, 40));
      p.uiState.model.refresh();
      await mount(tester, p);
      await tester.tap(twirl(box));
      await tester.pump();

      final was = p.graph.documentRevision();
      await editShader(
          tester,
          find.descendant(
              of: rowOn(box, 'edit'),
              matching: find.byKey(ValueKey<String>('fx-action-$box-edit'))),
          shader);
      expect(shaderOn(p.graph, box), shader);
      expect(p.graph.documentRevision(), was + BigInt.one, reason: 'one op');
    });

    testWidgets("a double-click opens a Custom shader box's inner graph",
        (tester) async {
      final p = withGraph();
      final box = seedFx(p.graph, 'custom_shader', const Offset(60, 40));
      p.uiState.model.refresh();
      await mount(tester, p);

      expect(find.byKey(ValueKey<String>('shader-thumb-empty-$box')),
          findsOneWidget,
          reason: 'the box carries the picture of its inner graph');
      // On the first socket row, clear of that picture, which opens on one
      // click.
      final at = tester.getTopLeft(card(box)) + const Offset(75, 30);
      await tester.tapAt(at);
      await tester.pump(kDoubleTapMinTime);
      await tester.tapAt(at);
      await tester.pump();
      expect(find.byKey(const ValueKey<String>('shader-breadcrumb')),
          findsOneWidget);
      expect(find.byKey(const ValueKey<String>('shader-crumb-layer')),
          findsNothing,
          reason: 'a node graph has no layer to name');

      // An edit inside is one op on the node graph, and one undo.
      String? inner() => p.graph
          .getNodeGraphInstances()
          .firstWhere((i) => i.id() == box)
          .shaderGraph();
      final was = p.graph.documentRevision();
      p.uiState.activePane.value = Panel.graph.pane();
      expect(p.uiState.consoleClaim!(), isTrue);
      await tester.pump();
      await tester
          .tap(find.byKey(const ValueKey<String>('fx-console-item-UV')));
      await tester.pump();
      expect(inner(), contains('"uv"'));
      expect(p.graph.documentRevision(), was + BigInt.one, reason: 'one op');

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(inner(), isNull, reason: 'one undo step');

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      expect(card(box), findsOneWidget,
          reason: 'Escape returns to the node graph');
    });

    testWidgets('the marquee and a wire land on an open box', (tester) async {
      final p = withGraph();
      final read = seedRead(p.graph, p.state, const Offset(20, 40));
      final blur = seedFx(p.graph, 'blur', const Offset(300, 40));
      p.uiState.model.refresh();
      await mount(tester, p);
      await tester.tap(twirl(blur));
      await tester.pump();

      await wire(tester, read, 'output', blur, 'input');
      expect(p.graph.getNodeGraph().wiring.edges, hasLength(1));

      // A band drawn wholly round the wider box picks it, and nothing else.
      final canvas =
          tester.getTopLeft(find.byKey(const ValueKey('comp-graph-canvas')));
      await tester.dragFrom(
          canvas + const Offset(280, 20), const Offset(400, 300));
      await tester.pump();
      expect(picked(tester, blur), isTrue);
      expect(picked(tester, read), isFalse);
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

    Future<void> pickExposure(WidgetTester tester) async {
      expect(
          find.byKey(const ValueKey<String>('fx-console-bar')), findsOneWidget);
      await tester.enterText(
          find.byKey(const ValueKey('fx-console-query')), 'exposure');
      await tester.pump();
      await tester
          .tap(find.byKey(const ValueKey<String>('fx-console-item-Exposure')));
      await tester.pump();
    }

    testWidgets('an effect dragged from Effects & presets lands where it drops',
        (tester) async {
      final p = withGraph();
      final read = seedRead(p.graph, p.state, const Offset(20, 40));
      p.uiState.model.refresh();
      await mount(
        tester,
        p,
        child: Column(children: [
          const Draggable<EffectDragData>(
            data: EffectDragData('exposure', 'Exposure'),
            hitTestBehavior: HitTestBehavior.opaque,
            feedback: SizedBox(width: 8, height: 8),
            child: SizedBox(
                key: ValueKey<String>('drag-source'), width: 60, height: 20),
          ),
          const Expanded(child: GraphPanelFrb()),
        ]),
      );
      await tester.tapAt(tester.getCenter(card(read)));
      await tester.pump();
      final was = p.graph.documentRevision();
      final from =
          tester.getTopLeft(find.byKey(const ValueKey<String>('drag-source')));

      await tester.drag(find.byKey(const ValueKey<String>('drag-source')),
          const Offset(400, 300),
          warnIfMissed: false);
      await tester.pumpAndSettle();

      final added = p.graph
          .getNodeGraph()
          .nodes
          .where((n) => n.matchName == 'exposure')
          .single
          .id;
      final canvas =
          tester.getTopLeft(find.byKey(const ValueKey('comp-graph-canvas')));
      expect(placeOf(p.graph, added), from + const Offset(400, 300) - canvas,
          reason: 'the box sits where it was let go');
      expect(
          p.graph
              .getNodeGraph()
              .wiring
              .edges
              .where((e) => e.from == read && e.to == added),
          hasLength(1),
          reason: 'Auto-wire put it after the picked box, as the console does');
      expect(p.graph.documentRevision(), was + BigInt.one, reason: 'one op');
    });

    testWidgets('a right-click, Tab and Shift+A each open the console',
        (tester) async {
      final p = withGraph();
      await mount(tester, p);
      addTearDown(() => lastKnownPointerPosition = null);
      final canvas =
          tester.getTopLeft(find.byKey(const ValueKey('comp-graph-canvas')));
      final boxes = p.graph.getNodeGraph().nodes.length;

      // A right-click on empty ground, the box landing under it.
      const click = Offset(300, 200);
      await tester.tapAt(canvas + click, buttons: kSecondaryMouseButton);
      await tester.pump();
      await pickExposure(tester);
      expect(p.graph.getNodeGraph().nodes, hasLength(boxes + 1));
      expect(placeOf(p.graph, p.uiState.compGraphNode.value!.id), click);

      // Tab with the canvas focused, the box landing at the pointer.
      await tester.tapAt(canvas + const Offset(600, 450));
      await tester.pump();
      const pointer = Offset(120, 380);
      lastKnownPointerPosition = canvas + pointer;
      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await tester.pump();
      await pickExposure(tester);
      expect(p.graph.getNodeGraph().nodes, hasLength(boxes + 2));
      expect(placeOf(p.graph, p.uiState.compGraphNode.value!.id), pointer);

      // Shift+A, Blender's add key.
      await tester.tapAt(canvas + const Offset(600, 450));
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();
      await pickExposure(tester);
      expect(p.graph.getNodeGraph().nodes, hasLength(boxes + 3));

      // Ctrl+A opens nothing, and select all still picks every box.
      await tester.tapAt(canvas + const Offset(600, 450));
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
      expect(find.byKey(const ValueKey<String>('fx-console-bar')), findsNothing);
      p.uiState.activePane.value = Panel.graph.pane();
      expect(p.uiState.requestSelectAll(), isTrue);
      await tester.pump();
      for (final node in p.graph.getNodeGraph().nodes) {
        expect(picked(tester, node.id), isTrue);
      }
    });

    testWidgets('copy and paste bring the boxes and their wire, fresh',
        (tester) async {
      final p = withGraph();
      final blur = seedFx(p.graph, 'blur', const Offset(40, 40));
      final glow = seedFx(p.graph, 'glow', const Offset(300, 40));
      p.uiState.model.refresh();
      await mount(tester, p);
      addTearDown(() => lastKnownPointerPosition = null);
      await wire(tester, blur, 'output', glow, 'input');
      final before = p.graph.getNodeGraph();

      // Everything picked, the Output too: the Output is never copied.
      p.uiState.activePane.value = Panel.graph.pane();
      expect(p.uiState.requestSelectAll(), isTrue);
      await tester.pump();
      expect(copySelectionFrb(p.uiState), isTrue);

      final canvas =
          tester.getTopLeft(find.byKey(const ValueKey('comp-graph-canvas')));
      lastKnownPointerPosition = canvas + const Offset(100, 300);
      final was = p.graph.documentRevision();
      expect(await pasteSelectionFrb(p.state, p.uiState, p.graph, null), isTrue);
      await tester.pump();

      final after = p.graph.getNodeGraph();
      expect(after.nodes, hasLength(before.nodes.length + 2),
          reason: 'two boxes, and no second Output');
      expect(p.graph.documentRevision(), was + BigInt.one, reason: 'one op');
      final fresh = [
        for (final n in after.nodes)
          if (!before.nodes.any((b) => b.id == n.id)) n.id,
      ];
      expect(after.wiring.edges, hasLength(2));
      expect(
          after.wiring.edges
              .where((e) => fresh.contains(e.from) && fresh.contains(e.to)),
          hasLength(1),
          reason: 'the wire between them came along');
      expect(after.wiring.groups, isEmpty, reason: 'a paste is not a group');
      expect(placeOf(p.graph, fresh.first), const Offset(100, 300),
          reason: 'the copied corner lands at the pointer');
      expect(placeOf(p.graph, blur), const Offset(40, 40),
          reason: 'the originals are untouched');
      for (final id in fresh) {
        expect(picked(tester, id), isTrue, reason: 'the pasted boxes are picked');
      }
      expect(picked(tester, blur), isFalse);

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.graph.getNodeGraph().nodes, hasLength(before.nodes.length),
          reason: 'one undo step');
    });

    testWidgets('keys answer only on the canvas whose panel is active',
        (tester) async {
      final p = withGraph();
      final blur = seedFx(p.graph, 'blur', const Offset(40, 40));
      p.uiState.model.refresh();
      await mount(
        tester,
        p,
        size: const Size(1400, 600),
        child: const Row(children: [
          Expanded(child: GraphPanelFrb()),
          Expanded(child: TimelinePanelFrb()),
        ]),
      );
      expect(find.byType(CompGraphPanel), findsNWidgets(2));

      // Picked on the Graph panel's canvas.
      await tester.tapAt(tester.getCenter(card(blur).first));
      await tester.pump();
      expect(p.uiState.compGraphNode.value?.id, blur);

      p.uiState.activePane.value = Panel.timeline.pane();
      expect(copySelectionFrb(p.uiState), isFalse);
      expect(p.uiState.deleteClaim!(), isFalse,
          reason: 'nothing is picked on the Timeline\'s canvas');
      expect(p.uiState.requestSelectAll(), isTrue);
      await tester.pump();
      expect(picked(tester, blur, at: 1), isTrue,
          reason: 'Ctrl+A picked every box on the Timeline\'s canvas');

      p.uiState.activePane.value = Panel.graph.pane();
      expect(copySelectionFrb(p.uiState), isTrue);
      expect(p.uiState.deleteClaim!(), isTrue);
      await tester.pump();
      expect(p.graph.getNodeGraph().nodes.any((n) => n.id == blur), isFalse);
    });

  }, skip: !engineAvailable);
}
