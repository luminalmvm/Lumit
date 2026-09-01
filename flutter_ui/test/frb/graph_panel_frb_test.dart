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

import 'dart:io';

import 'package:flutter/gestures.dart'
    show PointerScrollEvent, kDoubleTapMinTime;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/panels/viewer_prefix_chip.dart';
import 'package:lumit_flutter/state/dock.dart';
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
          groups: graph.wiring.groups,
        ),
      );
      return id;
    }

    Future<void> mount(WidgetTester tester, dynamic p,
        {List<BridgeEffectInfo> Function()? drivers,
        List<BridgePresetInfo> Function()? groups,
        Future<String?> Function()? groupSave}) async {
      const size = Size(900, 600);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: GraphPanelFrb(
          driversLister: drivers,
          groupsLister: groups,
          groupSavePicker: groupSave,
        ),
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

    /// The header twirl grows the box to every parameter socket. Until it is
    /// open, a number socket nobody has wired is not drawn at all.
    testWidgets('exposure shows the parameter sockets, and is one op',
        (tester) async {
      final p = withBlur();
      await mount(tester, p);
      final key = effectKey(p.layer);

      expect(socket(key, 'radius'), findsNothing);
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
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
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
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

    /// **N5, first half — a wire that is already there could not be taken off**
    /// (owner, desk test). Pressing a wired input used to start a *second* wire
    /// out of it, which no drop could accept, so the only way to unplug was to
    /// click the socket dead on without moving a pixel. A press on a wired
    /// input now takes hold of the wire itself, by its far end.
    testWidgets('a wire pulled off its input and dropped on nothing goes',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);
      final key = effectKey(p.layer);
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
      await tester.pump();

      var from = tester.getCenter(socket('driver:$wiggle', 'value'));
      final radius = tester.getCenter(socket(key, 'radius'));
      await tester.dragFrom(from, radius - from);
      await tester.pump();
      expect(p.layer.getGraph().wiring.edges, hasLength(1));

      // Off the input, out onto bare canvas.
      await tester.dragFrom(radius, const Offset(0, 220));
      await tester.pump();
      expect(p.layer.getGraph().wiring.edges, isEmpty);
      expect(find.byKey(const ValueKey<String>('fx-console-bar')), findsNothing,
          reason: 'a wire being taken off is not a wire looking for a node');

      // And it is one undo step of its own, like every other gesture here.
      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.layer.getGraph().wiring.edges, hasLength(1));
    });

    /// The same grab, let go on another socket: the wire moves there rather
    /// than doubling, because an input takes one wire.
    testWidgets('a wire pulled off its input onto another input moves',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);
      final key = effectKey(p.layer);
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
      await tester.pump();

      final out = tester.getCenter(socket('driver:$wiggle', 'value'));
      final radius = tester.getCenter(socket(key, 'radius'));
      await tester.dragFrom(out, radius - out);
      await tester.pump();

      final mix = tester.getCenter(socket(key, 'mix'));
      await tester.dragFrom(radius, mix - radius);
      await tester.pump();

      final edges = p.layer.getGraph().wiring.edges;
      expect(edges, hasLength(1));
      expect(edges.single.to,
          isA<BridgeInputRef_Param>().having((e) => e.port, 'port', 'mix'));
    });

    /// **N5, second half — one output feeds any number of inputs.** Only the
    /// destination is exclusive: a second wire drawn out of a producer is an
    /// addition, never a replacement.
    testWidgets('one driver output fans out to two parameters', (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);
      final key = effectKey(p.layer);
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
      await tester.pump();

      var out = tester.getCenter(socket('driver:$wiggle', 'value'));
      final radius = tester.getCenter(socket(key, 'radius'));
      await tester.dragFrom(out, radius - out);
      await tester.pump();

      out = tester.getCenter(socket('driver:$wiggle', 'value'));
      final mix = tester.getCenter(socket(key, 'mix'));
      await tester.dragFrom(out, mix - out);
      await tester.pump();

      final edges = p.layer.getGraph().wiring.edges;
      expect(edges, hasLength(2), reason: 'the first wire is still there');
      expect(
        edges.every((e) =>
            e.from == BridgeOutputRef.driver(node: wiggle, port: 'value')),
        isTrue,
      );
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
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
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

    /// **P0 — Delete with a node picked deleted the layer** (owner, desk
    /// test). The canvas answered the key through the focus tree, but the
    /// shell answers Delete on the hardware keyboard, which runs *before* the
    /// focus tree and swallows the key: the picked box was never asked about,
    /// and the layer under it went instead. The panel claims Delete now
    /// (K-234's mechanism) and the shell stands down when the claim says yes.
    testWidgets(
        'a picked box claims Delete rather than leaving it to the shell',
        (tester) async {
      final p = withBlur();
      await mount(tester, p);
      p.uiState.activePanel.value = Panel.graph;

      expect(p.uiState.deleteClaim, isNotNull,
          reason: 'the panel claims Delete while it is mounted');
      expect(p.uiState.deleteClaim!(), isFalse,
          reason: 'with nothing picked the key is not this panel\'s, and the '
              'shell goes on to the selected layer as it always did');

      final key = effectKey(p.layer);
      await tester.tapAt(
          tester.getCenter(find.byKey(ValueKey<String>('graph-node-$key'))));
      await tester.pump();

      expect(p.uiState.deleteClaim!(), isTrue,
          reason: 'a picked box is what Delete is about here');
      await tester.pump();
      expect(p.layer.getEffects(), isEmpty,
          reason: 'and the box is the thing '
              'that went — not the layer it was drawn for');
    });

    testWidgets('deleting a wired driver takes its wire with it, in one step',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);
      final key = effectKey(p.layer);
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
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
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
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

    /// Auto-wire: let a wire go over empty canvas, pick a node from the
    /// console, and the wire is on it when it lands.
    testWidgets('the console adds a driver and auto-wire joins it',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);

      final from = tester.getCenter(socket('driver:$wiggle', 'value'));
      await tester.dragFrom(from, const Offset(220, 60));
      await tester.pump();
      expect(
          find.byKey(const ValueKey<String>('fx-console-bar')), findsOneWidget);

      await tester
          .tap(find.byKey(const ValueKey<String>('fx-console-item-Smooth')));
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
          .tap(find.byKey(const ValueKey<String>('fx-console-item-Smooth')));
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

    /// **Scrolling the console never zooms the canvas beneath** (owner item
    /// 12). The console floats in the overlay, whose click-catcher is opaque
    /// to hit tests, so a wheel over the popover — list or margin — cannot
    /// reach the canvas's own scroll-zoom listener. The old inner-graph
    /// popover leaked exactly this way by sitting *inside* the canvas's
    /// listener; this holds the road every console now takes.
    testWidgets('a wheel over the console leaves the canvas zoom alone',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);

      String zoom() =>
          tester.widget<Text>(find.byKey(const ValueKey('graph-zoom'))).data!;
      final before = zoom();

      await tester.dragFrom(tester.getCenter(socket('driver:$wiggle', 'value')),
          const Offset(240, 80));
      await tester.pump();
      expect(
          find.byKey(const ValueKey<String>('fx-console-bar')), findsOneWidget);

      // Over the result list…
      final list = tester.getCenter(
          find.byKey(const ValueKey<String>('fx-console-item-Smooth')));
      await tester.sendEventToBinding(
          PointerScrollEvent(position: list, scrollDelta: const Offset(0, 40)));
      await tester.pump();
      // …and over the invisible catcher, well away from the popover.
      await tester.sendEventToBinding(const PointerScrollEvent(
          position: Offset(60, 560), scrollDelta: Offset(0, 40)));
      await tester.pump();

      expect(zoom(), before,
          reason: 'the scroll is the console\'s, never the graph\'s');
    });

    /// **The search shows what the wire in hand could land on** (WP3), so the
    /// footer's promise — "connects the dragged wire where it fits" — is true
    /// of every row it offers.
    testWidgets('the console filters by the dragged wire\'s type',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      final cycle = seedDriver(p.layer, 'colour_cycle', const Offset(30, 440));
      await mount(tester, p);

      // A number in hand: the drivers that take a number are offered.
      await tester.dragFrom(tester.getCenter(socket('driver:$wiggle', 'value')),
          const Offset(240, 60));
      await tester.pump();
      expect(find.byKey(const ValueKey<String>('fx-console-item-Smooth')),
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
          find.byKey(const ValueKey<String>('fx-console-bar')), findsOneWidget);
      expect(find.byKey(const ValueKey<String>('fx-console-item-Smooth')),
          findsNothing);
      expect(find.byKey(const ValueKey<String>('fx-console-item-Wiggle')),
          findsNothing);

      // Without a wire — Ctrl+Space, answered through the claim (K-673) —
      // the whole family is back.
      await tester.tapAt(const Offset(860, 560));
      await tester.pump();
      p.uiState.activePanel.value = Panel.graph;
      expect(p.uiState.consoleClaim!(), isTrue,
          reason: 'the graph claims the console while it is the focused panel');
      await tester.pump();
      expect(find.byKey(const ValueKey<String>('fx-console-item-Smooth')),
          findsOneWidget);
    });

    /// **Ctrl+Space is the graph's one add surface** (K-673): with the panel
    /// focused, the shell's console stands down and this one offers the
    /// effects beside the drivers — a chosen effect joins the stack, so its
    /// box lands on the chain.
    testWidgets('the console adds an effect to the chain', (tester) async {
      final p = withBlur();
      await mount(tester, p);

      p.uiState.activePanel.value = Panel.graph;
      expect(p.uiState.consoleClaim!(), isTrue);
      await tester.pump();
      expect(
          find.byKey(const ValueKey<String>('fx-console-bar')), findsOneWidget);

      // Reach the row the way a hand does, by typing: the effects list after
      // the driver family, past the list's fold.
      await tester.enterText(
          find.byKey(const ValueKey('fx-console-query')), 'exposure');
      await tester.pump();
      await tester.tap(
          find.byKey(const ValueKey<String>('fx-console-item-Exposure')));
      await tester.pump();

      expect([for (final e in p.layer.getEffects()) e.getInfo().name],
          ['blur', 'exposure'],
          reason: 'the chosen effect joined the stack, which is the chain');
    });

    /// The claim is the graph's only while the graph is focused: anywhere
    /// else, the shell's own console answers as it always did.
    testWidgets('the console claim stands down when another panel is focused',
        (tester) async {
      final p = withBlur();
      await mount(tester, p);

      p.uiState.activePanel.value = Panel.timeline;
      expect(p.uiState.consoleClaim!(), isFalse,
          reason: 'not this panel\'s key, so the shell\'s console opens');
      expect(find.byKey(const ValueKey<String>('fx-console-bar')), findsNothing);
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
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
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
          groups: moved.wiring.groups,
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
          .tap(find.byKey(const ValueKey<String>('fx-console-item-Smooth')));
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
    testWidgets('the enable tick bypasses an effect and a driver alike',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);
      final key = effectKey(p.layer);

      await tester.tap(find.byKey(ValueKey<String>('graph-enable-$key')));
      await tester.pump();
      expect(p.layer.getEffects().single.enabled(), isFalse);

      await tester
          .tap(find.byKey(ValueKey<String>('graph-enable-driver:$wiggle')));
      await tester.pump();
      expect(p.layer.getGraphDrivers().single.enabled(), isFalse);

      // A driver draws every socket it has whatever its exposure says, so it
      // carries the tick and no twirl — a control that answered nothing would
      // be worse than none.
      expect(find.byKey(ValueKey<String>('graph-twirl-$key')), findsOneWidget);
      expect(find.byKey(ValueKey<String>('graph-twirl-driver:$wiggle')),
          findsNothing);
    });

    // --- The points wire (K-492, K-494, points-stream.md §4.3) -------------
    //
    // The first wire whose *source* is a stack effect. Everything else on this
    // canvas was already true of it — teal came from `PortColours` in WP1, the
    // type rule is the one every other drop takes — so what these hold is the
    // arm that was missing and the two states the wire can be in.

    /// A layer carrying Particulate, with a Points sample driver beside it.
    ({
      LumitState state,
      LumitUiState uiState,
      LayerReference layer,
      UuidValue sample
    }) withPoints() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'particulate');
      p.uiState.selectedLayer.value = layer;
      final sample = seedDriver(layer, 'points_sample', const Offset(30, 300));
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, layer: layer, sample: sample);
    }

    String particulateKey(LayerReference layer) => graphNodeKey(layer
        .getGraph()
        .nodes
        .firstWhere((n) => n.matchName == 'particulate')
        .node);

    /// **A wire-only socket is always drawn** — there is no row anywhere else
    /// to reach it from, which is what separates it from a parameter socket
    /// the twirl folds away.
    testWidgets('the Points sockets are drawn without exposing anything',
        (tester) async {
      final p = withPoints();
      await mount(tester, p);

      expect(socket(particulateKey(p.layer), 'points'), findsOneWidget,
          reason: 'the effect declares a data output; it has no row');
      expect(socket('driver:${p.sample}', 'points'), findsOneWidget);
      // The driver's two numbers are its whole purpose, so the box shows them
      // as ports — a driver draws every socket it has.
      expect(socket('driver:${p.sample}', 'count'), findsOneWidget);
      expect(socket('driver:${p.sample}', 'nearest_distance'), findsOneWidget);
      expect(socket(particulateKey(p.layer), 'emit_rate'), findsNothing,
          reason: 'a parameter socket still waits for the twirl');
    });

    testWidgets('Particulate\'s Points output wires into the driver, once',
        (tester) async {
      final p = withPoints();
      await mount(tester, p);
      final key = particulateKey(p.layer);

      final from = tester.getCenter(socket(key, 'points'));
      final to = tester.getCenter(socket('driver:${p.sample}', 'points'));
      await tester.dragFrom(from, to - from);
      await tester.pump();

      final edges = p.layer.getGraph().wiring.edges;
      expect(edges, hasLength(1));
      expect(
        edges.single.from,
        isA<BridgeOutputRef_EffectData>()
            .having((e) => e.port, 'port', 'points'),
        reason: 'the source is the stack effect itself (K-492)',
      );
      expect(edges.single.to,
          isA<BridgeInputRef_Param>().having((e) => e.port, 'port', 'points'));

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.layer.getGraph().wiring.edges, isEmpty,
          reason: 'one gesture, one undo step');
    });

    /// The Tab search's filter answers from the catalogue (PS3), so a teal
    /// wire in hand offers the entries that declare a Points input — which in
    /// v1 is Points sample and nothing else.
    testWidgets('the Tab search offers Points sample to a teal wire',
        (tester) async {
      final p = withPoints();
      await mount(tester, p);

      await tester.dragFrom(
          tester.getCenter(socket(particulateKey(p.layer), 'points')),
          const Offset(200, 120));
      await tester.pump();

      expect(
          find.byKey(const ValueKey<String>('fx-console-bar')), findsOneWidget);
      expect(
          find.byKey(const ValueKey<String>('fx-console-item-Points sample')),
          findsOneWidget);
      expect(find.byKey(const ValueKey<String>('fx-console-item-Wiggle')),
          findsNothing,
          reason: 'a wiggle has nothing a points stream could land on');
    });

    /// **The loop is declined before it is committed.** Particulate feeding a
    /// sample whose Count feeds Particulate's Emit rate is the one genuine
    /// cycle v1 makes constructible (points-stream.md §1.2). The engine
    /// refuses it with the `Cycle` sentence and that is the backstop; a
    /// refusal the panel swallows would look like a gesture that did nothing,
    /// so the second drop never leaves the panel.
    testWidgets('a wire that would close a loop is declined here',
        (tester) async {
      final p = withPoints();
      await mount(tester, p);
      final key = particulateKey(p.layer);

      final from = tester.getCenter(socket(key, 'points'));
      final to = tester.getCenter(socket('driver:${p.sample}', 'points'));
      await tester.dragFrom(from, to - from);
      await tester.pump();
      expect(p.layer.getGraph().wiring.edges, hasLength(1));

      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
      await tester.pump();
      final back = tester.getCenter(socket('driver:${p.sample}', 'count'));
      final onto = tester.getCenter(socket(key, 'emit_rate'));
      await tester.dragFrom(back, onto - back);
      await tester.pump();

      expect(p.layer.getGraph().wiring.edges, hasLength(1),
          reason: 'the stream would depend on the parameter it feeds');
    });

    /// **The hazard, made visible** (K-509). A Points sample with nothing
    /// wired in answers its documented no-op — a distance so large it pins
    /// whatever it drives at the far end of the range — so the box says so
    /// until a stream reaches it.
    testWidgets('a sample with no stream wears the warning mark',
        (tester) async {
      final p = withPoints();
      await mount(tester, p);
      final mark =
          find.byKey(ValueKey<String>('graph-no-stream-driver:${p.sample}'));
      expect(mark, findsOneWidget);

      final key = particulateKey(p.layer);
      final from = tester.getCenter(socket(key, 'points'));
      final to = tester.getCenter(socket('driver:${p.sample}', 'points'));
      await tester.dragFrom(from, to - from);
      await tester.pump();

      expect(mark, findsNothing, reason: 'the stream arrived');
      expect(find.byKey(ValueKey<String>('graph-no-stream-$key')), findsNothing,
          reason: 'the producer reads no stream of its own');
    });

    /// A box's position is document data: it persists, it travels, and a drag
    /// stages it and commits once (K-344). The magnet is on by default
    /// (2026-08-30 board), so what commits is the dot grid's nearest pitch.
    testWidgets('dragging a box commits its position once, on the grid',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);

      final box = find.byKey(ValueKey<String>('graph-node-driver:$wiggle'));
      await tester.dragFrom(tester.getCenter(box), const Offset(40, 20));
      await tester.pump();

      BridgeNodePosition placed() => p.layer
          .getGraph()
          .wiring
          .layout
          .firstWhere((l) => l.node == BridgeNodeRef.driver(wiggle));
      // Raw would be (70, 320); the magnet lands it on the 20px pitch.
      expect(placed().x, 80, reason: 'snapped to the dot grid (K-626)');
      expect(placed().y, 320);

      // Magnet off: the same drag commits exactly where the hand left it.
      await tester.tap(find.byKey(const ValueKey('graph-snap')));
      await tester.pump();
      await tester.dragFrom(
          tester.getCenter(box), const Offset(-13, -7));
      await tester.pump();
      expect(placed().x, 67, reason: 'off means off — no snapping');
      expect(placed().y, 313);
    });

    /// **N7 — a box dropped on a wire falls into it.** The wire splits: what
    /// fed the consumer now feeds the box, and the box feeds the consumer. One
    /// `setGraph`, so one undo step, like every other gesture on this canvas.
    testWidgets('an unwired box dropped on a wire is inserted into it',
        (tester) async {
      final p = withBlur();
      final first = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      final spare = seedDriver(p.layer, 'wiggle', const Offset(30, 460));
      await mount(tester, p);
      final key = effectKey(p.layer);
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
      await tester.pump();

      final out = tester.getCenter(socket('driver:$first', 'value'));
      final radius = tester.getCenter(socket(key, 'radius'));
      await tester.dragFrom(out, radius - out);
      await tester.pump();
      expect(p.layer.getGraph().wiring.edges, hasLength(1));

      // The cubic's handles run horizontally out of each socket by the same
      // reach, so the point halfway along it is the midpoint of the two ends.
      final middle = (tester.getCenter(socket('driver:$first', 'value')) +
              tester.getCenter(socket(key, 'radius'))) /
          2;
      final box = find.byKey(ValueKey<String>('graph-node-driver:$spare'));
      final grab = tester.getCenter(box);
      await tester.dragFrom(grab, middle - grab);
      await tester.pump();

      final edges = p.layer.getGraph().wiring.edges;
      expect(edges, hasLength(2));
      final blur = p.layer
          .getGraph()
          .nodes
          .firstWhere((n) => n.matchName == 'blur')
          .node;
      expect(
        edges.any((e) =>
            e.from == BridgeOutputRef.driver(node: first, port: 'value') &&
            e.to ==
                BridgeInputRef.param(
                    node: BridgeNodeRef.driver(spare), port: 'amount')),
        isTrue,
        reason: 'what fed the parameter now feeds the box',
      );
      expect(
        edges.any((e) =>
            e.from == BridgeOutputRef.driver(node: spare, port: 'value') &&
            e.to == BridgeInputRef.param(node: blur, port: 'radius')),
        isTrue,
        reason: 'and the box feeds the parameter',
      );

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.layer.getGraph().wiring.edges, hasLength(1),
          reason: 'one gesture, one undo step');
    });

    /// A box that already carries wires is only being moved: dropping it on a
    /// wire would leave the question of what became of its own.
    testWidgets('a wired box dragged over a wire is only moved',
        (tester) async {
      final p = withBlur();
      final first = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      final other = seedDriver(p.layer, 'wiggle', const Offset(30, 460));
      await mount(tester, p);
      final key = effectKey(p.layer);
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-$key')));
      await tester.pump();

      var out = tester.getCenter(socket('driver:$first', 'value'));
      final radius = tester.getCenter(socket(key, 'radius'));
      await tester.dragFrom(out, radius - out);
      await tester.pump();
      out = tester.getCenter(socket('driver:$other', 'value'));
      final mix = tester.getCenter(socket(key, 'mix'));
      await tester.dragFrom(out, mix - out);
      await tester.pump();
      expect(p.layer.getGraph().wiring.edges, hasLength(2));

      final middle = (tester.getCenter(socket('driver:$first', 'value')) +
              tester.getCenter(socket(key, 'radius'))) /
          2;
      final box = find.byKey(ValueKey<String>('graph-node-driver:$other'));
      final grab = tester.getCenter(box);
      await tester.dragFrom(grab, middle - grab);
      await tester.pump();

      expect(p.layer.getGraph().wiring.edges, hasLength(2),
          reason: 'the drop moved the box and nothing else');
    });

    /// **A box is renamed by double-clicking its name** (owner, desk test).
    /// Both kinds commit the way their bypass does — a driver through
    /// `setGraph`, a stack effect through the staged `setEffects` — so each is
    /// one op and one undo step, and no call was added to the bridge for it:
    /// `set_custom_name` is already on the instance handle both lists hand out.
    Future<void> doubleTapName(WidgetTester tester, String key) async {
      final name = find.byKey(ValueKey<String>('graph-node-name-$key'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();
    }

    Future<void> renameBox(WidgetTester tester, String key, String to) async {
      await doubleTapName(tester, key);
      final field = find.byKey(ValueKey<String>('graph-node-rename-$key'));
      expect(field, findsOneWidget, reason: 'the double-click opened it');
      await tester.enterText(field, to);
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
    }

    String? customNameOf(LayerReference layer, String key) => layer
        .getGraph()
        .nodes
        .firstWhere((n) => graphNodeKey(n.node) == key)
        .customName;

    testWidgets('renaming an effect box round-trips in one undo',
        (tester) async {
      final p = withBlur();
      await mount(tester, p);
      final key = effectKey(p.layer);

      await renameBox(tester, key, 'Soften the sign');
      expect(customNameOf(p.layer, key), 'Soften the sign');

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(customNameOf(p.layer, key), isNull,
          reason: 'one gesture, one undo step');
    });

    testWidgets('renaming a driver box round-trips in one undo',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      await mount(tester, p);
      final key = 'driver:$wiggle';

      await renameBox(tester, key, 'Camera shake');
      expect(customNameOf(p.layer, key), 'Camera shake');

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(customNameOf(p.layer, key), isNull,
          reason: 'a driver commits through setGraph, and once');
    });

    /// An empty name is a real answer: it clears the custom name and the card
    /// goes back to the box's own label.
    testWidgets('an empty name clears back to the effect\'s own label',
        (tester) async {
      final p = withBlur();
      await mount(tester, p);
      final key = effectKey(p.layer);

      await renameBox(tester, key, 'Soften the sign');
      expect(customNameOf(p.layer, key), 'Soften the sign');
      expect(find.text('Soften the sign'), findsOneWidget);

      await renameBox(tester, key, '   ');
      expect(customNameOf(p.layer, key), isNull,
          reason: 'whitespace is no name at all');
      expect(find.text('Soften the sign'), findsNothing,
          reason: 'the card shows the box\'s own label again');
    });

    /// The Source and Layer out boxes have no name of their own to give: the
    /// Source shows the layer's name, the Out is the layer's own end.
    testWidgets('the derived boxes cannot be renamed', (tester) async {
      final p = withBlur();
      await mount(tester, p);

      await doubleTapName(tester, 'source');
      expect(find.byKey(const ValueKey<String>('graph-node-rename-source')),
          findsNothing);
    });

    // -------------------------------------------------------------------
    // **The pick is a set** (K-533, and with it K-523 and K-522). Delete, Bypass and Expose were
    // singular because the selection was, not because any of them is singular
    // by nature — and `Ctrl+A` had nothing here to mean.
    //
    // The pick is read through `selectedEffects`, which is where the graph
    // publishes it: the box and the Effect controls heading are one selection
    // (K-300), so what the canvas has picked is exactly what that list says.
    // -------------------------------------------------------------------

    /// A layer with two effect boxes between Source and Layer out, in a comp
    /// the shell has fronted — so the read model holds the layer and things
    /// derived from the pick (the Viewer's chip) can be read off it.
    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withTwoEffects() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      layer.addEffect(name: 'exposure');
      p.uiState.selectedLayer.value = layer;
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    List<UuidValue> stackIds(LayerReference layer) =>
        [for (final e in layer.getEffects()) e.id()];

    Finder boxOf(LayerReference layer, int i) =>
        find.byKey(ValueKey<String>('graph-node-effect:${stackIds(layer)[i]}'));

    Future<void> clickBox(WidgetTester tester, Finder box,
        {LogicalKeyboardKey? held}) async {
      if (held != null) await tester.sendKeyDownEvent(held);
      await tester.tapAt(tester.getCenter(box));
      if (held != null) await tester.sendKeyUpEvent(held);
      await tester.pump();
    }

    testWidgets('a click replaces the pick and Ctrl-click toggles it',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);
      final ids = stackIds(p.layer);

      await clickBox(tester, boxOf(p.layer, 0));
      expect(p.uiState.selectedEffects.value, [ids[0]]);

      await clickBox(tester, boxOf(p.layer, 1),
          held: LogicalKeyboardKey.controlLeft);
      expect(p.uiState.selectedEffects.value, ids,
          reason: 'Ctrl added the second, in stack order');

      await clickBox(tester, boxOf(p.layer, 0),
          held: LogicalKeyboardKey.controlLeft);
      expect(p.uiState.selectedEffects.value, [ids[1]],
          reason: 'and a second Ctrl-click on a picked box takes it out again');

      // A plain click on one of several picked boxes collapses the pick to it.
      await clickBox(tester, boxOf(p.layer, 1),
          held: LogicalKeyboardKey.shiftLeft);
      expect(p.uiState.selectedEffects.value, [ids[1]]);
      await clickBox(tester, boxOf(p.layer, 0),
          held: LogicalKeyboardKey.shiftLeft);
      expect(p.uiState.selectedEffects.value, ids, reason: 'Shift adds');
      await clickBox(tester, boxOf(p.layer, 0));
      expect(p.uiState.selectedEffects.value, [ids[0]]);
    });

    /// **A box swept on empty canvas** — the application's own rubber band,
    /// caught wholly inside as it is everywhere else. The chain lies in one
    /// row, so a band that takes the two effects between Source and Layer out
    /// proves both halves of that rule at once.
    testWidgets('a marquee on empty canvas takes the boxes wholly inside it',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);
      final ids = stackIds(p.layer);

      final first = tester.getRect(boxOf(p.layer, 0));
      final second = tester.getRect(boxOf(p.layer, 1));
      final band = first.expandToInclude(second).inflate(8);
      // The corner it starts from is the gap between Source and the first
      // effect: empty canvas, which is what makes this a band and not a drag.
      await tester.dragFrom(band.topLeft, band.bottomRight - band.topLeft);
      await tester.pump();

      expect(p.uiState.selectedEffects.value, ids);
      expect(find.byKey(const ValueKey('graph-marquee')), findsNothing,
          reason: 'the band goes when it is let go of');
    });

    /// **Deleting several boxes is one undo step.** Each effect leaves by the
    /// stack's own op, so without a group a pick of two would take two undos
    /// to bring back — and the second one would be the gesture before it.
    testWidgets('Delete takes the whole pick, and one undo brings it back',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);
      expect(p.layer.getEffects(), hasLength(2));

      await clickBox(tester, boxOf(p.layer, 0));
      await clickBox(tester, boxOf(p.layer, 1),
          held: LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.delete);
      await tester.pump();
      expect(p.layer.getEffects(), isEmpty);

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.layer.getEffects(), hasLength(2),
          reason: 'one gesture, one undo step');
    });

    testWidgets('the enable tick bypasses every picked box', (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);
      final ids = stackIds(p.layer);

      await clickBox(tester, boxOf(p.layer, 0));
      await clickBox(tester, boxOf(p.layer, 1),
          held: LogicalKeyboardKey.controlLeft);
      await tester.tap(find.byKey(ValueKey<String>('graph-enable-effect:'
          '${ids[0]}')));
      await tester.pump();

      expect([for (final e in p.layer.getEffects()) e.getInfo().enabled],
          [false, false]);
    });

    testWidgets('the twirl exposes every picked box', (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);
      final ids = stackIds(p.layer);

      await clickBox(tester, boxOf(p.layer, 0));
      await clickBox(tester, boxOf(p.layer, 1),
          held: LogicalKeyboardKey.controlLeft);
      await tester.tap(find.byKey(ValueKey<String>('graph-twirl-effect:'
          '${ids[0]}')));
      await tester.pump();

      expect(p.layer.getGraph().wiring.exposed, hasLength(2));
      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.layer.getGraph().wiring.exposed, isEmpty,
          reason: 'one `setGraph`, so one undo step however many were picked');
    });

    /// `Ctrl+A` here means this canvas, not the composition's layers (K-522).
    testWidgets('Ctrl+A picks every box on the canvas', (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);

      p.uiState.activePanel.value = Panel.graph;
      expect(p.uiState.requestSelectAll(), isTrue,
          reason: 'the graph claims the chord now that it can answer it');
      await tester.pump();

      expect(p.uiState.selectedEffects.value, stackIds(p.layer),
          reason: 'both effect boxes; Source and Layer out are picked too, '
              'and carry no effect id to publish');
    });

    /// **The Viewer chip wants exactly one** (K-528). Its name is derived, so
    /// a pick of several must make it go away by itself rather than by anyone
    /// remembering to turn it off.
    testWidgets('the prefix chip names one picked box and no more',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);

      await clickBox(tester, boxOf(p.layer, 0));
      expect(prefixChipName(p.uiState), isNotNull);

      await clickBox(tester, boxOf(p.layer, 1),
          held: LogicalKeyboardKey.controlLeft);
      expect(prefixChipName(p.uiState), isNull,
          reason: 'two picked is no single point to stop at');
    });

    // --- The image chain's own wires (K-674, owner item 10) ----------------
    //
    // The chain is the effect list (§1.1), so every gesture on its wires
    // lowers to the stack's own ops: re-route = reorder, discard = the fed
    // box leaves the list, neighbours joining by construction.

    List<String> chainNames(LayerReference layer) =>
        [for (final e in layer.getEffects()) e.getInfo().name];

    Finder chainInput(LayerReference layer, int i) => find.byKey(ValueKey<String>(
        'graph-socket-effect:${stackIds(layer)[i]}-input'));

    testWidgets('a chain wire dropped on empty takes the fed effect out',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);
      expect(chainNames(p.layer), ['blur', 'exposure']);

      // Grab the wire feeding the exposure box and drop it on bare canvas.
      final at = tester.getCenter(chainInput(p.layer, 1));
      await tester.dragFrom(at, const Offset(40, 220));
      await tester.pump();

      expect(chainNames(p.layer), ['blur'],
          reason: 'the connection went, and with it the box it fed — the '
              'honest inverse of dropping a box into a wire');
      expect(find.byKey(const ValueKey<String>('fx-console-bar')), findsNothing,
          reason: 'a wire being taken off is not a wire looking for a node');

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(chainNames(p.layer), ['blur', 'exposure'],
          reason: 'one gesture, one undo step');
    });

    testWidgets('unplugging the Layer out takes the last effect',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);

      final at = tester
          .getCenter(find.byKey(const ValueKey<String>('graph-socket-out-image')));
      await tester.dragFrom(at, const Offset(40, 220));
      await tester.pump();

      expect(chainNames(p.layer), ['blur'],
          reason: 'the wire into Layer out is the last effect\'s place');
    });

    testWidgets('a chain wire dropped on another chain input reorders',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);

      // The wire Source → blur, dropped on exposure's input: the Source feeds
      // exposure now, so exposure moves to the head of the stack.
      final from = tester.getCenter(chainInput(p.layer, 0));
      final to = tester.getCenter(chainInput(p.layer, 1));
      await tester.dragFrom(from, to - from);
      await tester.pump();

      expect(chainNames(p.layer), ['exposure', 'blur'],
          reason: 'rewiring the chain is a reorder (§1.1)');

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(chainNames(p.layer), ['blur', 'exposure'],
          reason: 'one reorder op, one undo step');
    });

    /// The gesture everybody tries first, and the one that used to do nothing:
    /// press an output, drag to an input. It lowers to the same reorder the
    /// input grab does - the box whose output was pulled ends up feeding the
    /// box it was dropped on.
    testWidgets('a chain output dragged onto a later input reorders',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);
      expect(chainNames(p.layer), ['blur', 'exposure']);

      // exposure's output, dropped on... there is nothing after it, so take
      // the other direction: the Source's output onto exposure's input, which
      // says the Source feeds exposure and moves it to the head.
      final from = tester
          .getCenter(find.byKey(const ValueKey<String>('graph-socket-source-image')));
      final to = tester.getCenter(chainInput(p.layer, 1));
      await tester.dragFrom(from, to - from);
      await tester.pump();

      expect(chainNames(p.layer), ['exposure', 'blur'],
          reason: 'an output drag reorders exactly as an input drag does');

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(chainNames(p.layer), ['blur', 'exposure'],
          reason: 'one reorder op, one undo step');
    });

    /// A wire *drawn* from an output and let go of on the ground is a change
    /// of mind. Only a wire *pulled off* an input is a removal - which is why
    /// the two gestures cannot share one drop.
    testWidgets('an output wire dropped on empty canvas changes nothing',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);

      final from = tester
          .getCenter(find.byKey(const ValueKey<String>('graph-socket-source-image')));
      await tester.dragFrom(from, const Offset(0, 260));
      await tester.pump();

      expect(chainNames(p.layer), ['blur', 'exposure'],
          reason: 'a drawn wire dropped on nothing deletes nothing');
    });

    testWidgets('a chain wire dropped on the Layer out moves its source last',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);

      // The wire blur → exposure, dropped on the Layer out: blur feeds the
      // out now, so blur moves to the end of the stack.
      final from = tester.getCenter(chainInput(p.layer, 1));
      final to = tester
          .getCenter(find.byKey(const ValueKey<String>('graph-socket-out-image')));
      await tester.dragFrom(from, to - from);
      await tester.pump();

      expect(chainNames(p.layer), ['exposure', 'blur']);
    });

    testWidgets('a stationary press on a chain input changes nothing',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);

      await tester.tapAt(tester.getCenter(chainInput(p.layer, 1)));
      await tester.pump();

      expect(chainNames(p.layer), ['blur', 'exposure'],
          reason: 'a chain discard costs an effect, so a slip must not be one');
    });

    testWidgets('a chain wire dropped on a driver socket is declined',
        (tester) async {
      final p = withTwoEffects();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(30, 300));
      p.uiState.model.refresh();
      await tester.pump();
      await mount(tester, p);

      final from = tester.getCenter(chainInput(p.layer, 1));
      final to = tester.getCenter(socket('driver:$wiggle', 'amount'));
      await tester.dragFrom(from, to - from);
      await tester.pump();

      expect(chainNames(p.layer), ['blur', 'exposure'],
          reason: 'the picture\'s path cannot leave the chain, and nothing '
              'crossed the bridge');
    });

    // --- Named groups (K-651) ---------------------------------------------

    /// A temporary library folder, cleaned up with the test.
    Directory library() {
      final dir = Directory.systemTemp.createTempSync('lumit-groups');
      addTearDown(() {
        try {
          dir.deleteSync(recursive: true);
        } catch (_) {}
      });
      return dir;
    }

    Finder driverBox(UuidValue id) =>
        find.byKey(ValueKey<String>('graph-node-driver:$id'));

    /// **Naming a set is one act with two halves** (K-651): the wash appears on
    /// the canvas and the same name goes into the library, so a rig that took
    /// five minutes to wire is one row in the search from then on.
    testWidgets(
        'Save group names the pick, washes the canvas and writes a file',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(60, 300));
      final smooth = seedDriver(p.layer, 'smooth', const Offset(320, 300));
      final path = '${library().path}/Audio rig.lumgrp';
      await mount(tester, p, groupSave: () async => path);

      await clickBox(tester, driverBox(wiggle));
      await clickBox(tester, driverBox(smooth),
          held: LogicalKeyboardKey.controlLeft);
      await tester.tap(find.byKey(const ValueKey('graph-save-group')));
      await tester.pumpAndSettle();

      final group = p.layer.getGraph().wiring.groups.single;
      expect(group.name, 'Audio rig', reason: 'the file names the group');
      expect(group.members, hasLength(2));
      expect(group.colour, isNot(0),
          reason: 'index 0 is the quiet default of the palette, not a region');
      expect(File(path).existsSync(), isTrue);
      expect(find.byKey(const ValueKey<String>('graph-group-Audio rig')),
          findsOneWidget,
          reason: 'and the wash is drawn behind its members');
    });

    /// **The wires inside come back wired** — the whole reason a group is worth
    /// saving — and the drop is one undo step.
    testWidgets('a saved group is offered by the search and dropped whole',
        (tester) async {
      final p = withBlur();
      final wiggle = seedDriver(p.layer, 'wiggle', const Offset(60, 300));
      final smooth = seedDriver(p.layer, 'smooth', const Offset(320, 300));
      // Wire one into the other, so the file carries a wire of its own.
      final graph = p.layer.getGraph();
      p.layer.setGraph(
        drivers: p.layer.getGraphDrivers(),
        wiring: BridgeGraphWiring(
          edges: [
            ...graph.wiring.edges,
            BridgeGraphEdge(
              from: BridgeOutputRef.driver(node: wiggle, port: 'value'),
              to: BridgeInputRef.param(
                  node: BridgeNodeRef.driver(smooth), port: 'value'),
            ),
          ],
          layout: graph.wiring.layout,
          exposed: graph.wiring.exposed,
          groups: graph.wiring.groups,
        ),
      );
      final path = '${library().path}/Audio rig.lumgrp';
      File(path).writeAsStringSync(p.layer.saveNodeGroup(
        name: 'Audio rig',
        colour: 2,
        nodes: [BridgeNodeRef.driver(wiggle), BridgeNodeRef.driver(smooth)],
      ));
      p.uiState.model.refresh();

      await mount(tester, p,
          groups: () => [BridgePresetInfo(name: 'Audio rig', path: path)]);
      await tester.tapAt(const Offset(600, 500));
      await tester.pump();
      p.uiState.activePanel.value = Panel.graph;
      expect(p.uiState.consoleClaim!(), isTrue);
      await tester.pump();
      // The saved groups list after every driver and effect, past the list's
      // fold — so reach the row the way a hand does, by typing.
      await tester.enterText(
          find.byKey(const ValueKey('fx-console-query')), 'audio rig');
      await tester.pump();
      await tester
          .tap(find.byKey(const ValueKey<String>('fx-console-item-Audio rig')));
      await tester.pump();

      expect(p.layer.getGraphDrivers(), hasLength(4),
          reason: 'the two saved boxes arrived beside the two they came from');
      final dropped = p.layer.getGraph().wiring.groups.single;
      expect(dropped.name, 'Audio rig');
      expect(dropped.colour, 2);
      expect(p.layer.getGraph().wiring.edges, hasLength(2),
          reason: 'the wire inside the set came back, re-pointed');

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.layer.getGraphDrivers(), hasLength(2),
          reason: 'one undo takes the whole rig away');
      expect(p.layer.getGraph().wiring.groups, isEmpty);
    });
  });
}
