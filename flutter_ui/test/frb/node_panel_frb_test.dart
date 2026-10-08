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
          groups: graph.wiring.groups,
          outUnwired: false,
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

    /// An Expression box drew Edit expression and did nothing when it was
    /// pressed. It opens the dialogue and Apply writes the text.
    testWidgets("an Expression driver's Edit expression writes its text",
        (tester) async {
      final p = withBlur();
      final box = seedWiredDriver(p.layer, 'expression');
      p.uiState.graphNode.value = BridgeNodeRef.driver(box);
      await mount(tester, p, const NodePanelFrb(), width: 600);

      await tester.tap(find.byKey(ValueKey<String>('fx-action-$box-edit')));
      await tester.pumpAndSettle();
      await tester.enterText(
          find.descendant(
              of: find.byKey(const ValueKey('expression-text')),
              matching: find.byType(EditableText)),
          'time * 2');
      await tester.tap(find.byKey(const ValueKey('expression-confirm')));
      await tester.pumpAndSettle();
      expect(p.layer.getGraphDrivers().single.expressionSource(), 'time * 2');
    });

    /// Add input gives an Expression box a socket and a row. Remove input
    /// takes them away again, with the wire that was plugged in, in one op.
    testWidgets("an Expression driver's inputs are added and removed",
        (tester) async {
      final p = withBlur();
      final box = seedWiredDriver(p.layer, 'expression');
      p.uiState.graphNode.value = BridgeNodeRef.driver(box);
      await mount(tester, p, const NodePanelFrb(), width: 600);

      List<String> sockets() => [
            for (final s in p.layer
                .getGraph()
                .nodes
                .firstWhere((n) => n.node == BridgeNodeRef.driver(box))
                .inputs)
              s.id,
          ];
      final socket = BridgeInputRef.param(
          node: BridgeNodeRef.driver(box), port: 'input_1');
      bool plugged() =>
          p.layer.getGraph().wiring.edges.any((e) => e.to == socket);

      await tester
          .tap(find.byKey(ValueKey<String>('fx-action-$box-add_input')));
      await tester.pump();
      expect(sockets(), ['input_1']);
      expect(find.byKey(ValueKey<String>('node-row-$box-input_1')),
          findsOneWidget);

      // A Math box wired into the new input.
      final math = p.layer.newDriver(name: 'math');
      final graph = p.layer.getGraph();
      p.layer.setGraph(
        drivers: [...p.layer.getGraphDrivers(), math],
        wiring: BridgeGraphWiring(
          edges: [
            ...graph.wiring.edges,
            BridgeGraphEdge(
              from: BridgeOutputRef.driver(node: math.id(), port: 'value'),
              to: socket,
            ),
          ],
          layout: graph.wiring.layout,
          exposed: graph.wiring.exposed,
          groups: graph.wiring.groups,
          outUnwired: false,
        ),
      );
      p.uiState.model.refresh();
      await tester.pump();
      expect(plugged(), isTrue);

      await tester
          .tap(find.byKey(ValueKey<String>('fx-action-$box-remove_input')));
      await tester.pump();
      expect(sockets(), isEmpty);
      expect(plugged(), isFalse, reason: 'the wire goes with its socket');
      expect(p.layer.getGraph().wiring.edges, hasLength(1),
          reason: 'the wire into the blur is untouched');

      p.state.project!.undo();
      expect(sockets(), ['input_1']);
      expect(plugged(), isTrue, reason: 'one undo step brings both back');
    });

    /// **Audio level's Source row is a dropdown that starts on the comp**, and
    /// the Audio row under it names no layer until one is picked
    /// (docs/impl/audio-nodes.md §3). The layer list offers **every** layer,
    /// not only the ones that draw a picture: music arrives as an audio-only
    /// clip, which has no picture, so the one picker in the catalogue that
    /// exists to point at sound was the one picker that left it out. A Null
    /// stands in for it here, having no picture either and no fixture to
    /// decode.
    testWidgets('an Audio level starts on the comp with no layer named',
        (tester) async {
      final p = withBlur();
      final comp = p.uiState.selectedComp!;
      comp.addNullLayer();
      final made = p.layer.newDriver(name: 'audio_level');
      final id = made.id();
      p.layer.setGraph(
        drivers: [...p.layer.getGraphDrivers(), made],
        wiring: p.layer.getGraph().wiring,
      );
      p.uiState.model.refresh();
      p.uiState.graphNode.value = BridgeNodeRef.driver(id);
      await mount(tester, p, const NodePanelFrb());

      expect(find.text(l10n.fxAudioThisComp), findsOneWidget,
          reason: 'the Source row starts on the composition\'s own mix');
      expect(find.byKey(ValueKey<String>('node-row-$id-source')), findsOneWidget,
          reason: 'and it is a row of its own, not the layer picker\'s empty '
              'entry');

      await tester.tap(find.byKey(ValueKey<String>('fx-layer-$id-audio')));
      await tester.pumpAndSettle();
      final names = [for (final e in p.uiState.model.layers) e.info.name];
      expect(names, hasLength(2));
      // The Null is the layer with no picture; it must still be offered.
      final nulls = p.uiState.model.layers
          .where((e) => e.info.kind == BridgeLayerKind.nullLayer);
      expect(nulls, hasLength(1));
      expect(find.textContaining(nulls.single.info.name), findsWidgets,
          reason: 'a layer that draws nothing may still be the one that sounds');
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
