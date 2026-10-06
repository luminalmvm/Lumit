// What the Graph panel *does* — the gestures, and what each one commits.
//
// Every document operation here is genuine (see frb_test_support.dart), so a
// claim about "one undo step" is a claim about the real journal rather than
// about a mock.
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
    show kDoubleTapMinTime;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/drag_payloads.dart';
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
          outUnwired: false,
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

    /// **P0 — Delete with a node picked deleted the layer** (owner, desk
    /// test). The canvas answered the key through the focus tree, but the
    /// shell answers Delete on the hardware keyboard, which runs *before* the
    /// focus tree and swallows the key: the picked box was never asked about,
    /// and the layer under it went instead. The panel claims Delete now, and
    /// the shell stands down when the claim says yes.
    testWidgets(
        'a picked box claims Delete rather than leaving it to the shell',
        (tester) async {
      final p = withBlur();
      await mount(tester, p);
      p.uiState.activePane.value = Panel.graph.pane();

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

    /// **Ctrl+Space is the graph's one add surface**: with the panel
    /// focused, the shell's console stands down and this one offers the
    /// effects beside the drivers — a chosen effect joins the stack, so its
    /// box lands on the chain.
    testWidgets('the console adds an effect to the chain', (tester) async {
      final p = withBlur();
      await mount(tester, p);

      p.uiState.activePane.value = Panel.graph.pane();
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

    /// **The stack view can never lie**. The graph's image chain
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

    /// A box's position is document data: it persists, it travels, and a drag
    /// stages it and commits once. The magnet is on by default
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
      expect(placed().x, 80, reason: 'snapped to the dot grid');
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

    // -------------------------------------------------------------------
    // **The pick is a set**. Delete, Bypass and Expose were singular because
    // the selection was, not because any of them is singular by nature — and
    // `Ctrl+A` had nothing here to mean.
    //
    // The pick is read through `selectedEffects`, which is where the graph
    // publishes it: the box and the Effect controls heading are one
    // selection, so what the canvas has picked is exactly what that list says.
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

    // --- The image chain's own wires ---------------------------------------
    //
    // The chain is the effect list (§1.1), so every gesture on its wires
    // lowers to something the stack already had: re-route = reorder, and
    // **disconnect = bypass** - the effect keeps its slot and stops
    // drawing, which is the state its own enable tick sets. The Layer out is
    // the one box with no effect to bypass: unplugging it is the layer
    // drawing nothing.

    List<String> chainNames(LayerReference layer) =>
        [for (final e in layer.getEffects()) e.getInfo().name];

    List<bool> chainEnabled(LayerReference layer) =>
        [for (final e in layer.getEffects()) e.enabled()];

    Finder chainInput(LayerReference layer, int i) => find.byKey(ValueKey<String>(
        'graph-socket-effect:${stackIds(layer)[i]}-input'));

    testWidgets('a chain wire dropped on empty bypasses the fed effect',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);
      expect(chainNames(p.layer), ['blur', 'exposure']);

      // Grab the wire feeding the exposure box and drop it on bare canvas.
      final at = tester.getCenter(chainInput(p.layer, 1));
      await tester.dragFrom(at, const Offset(40, 220));
      await tester.pump();

      expect(chainNames(p.layer), ['blur', 'exposure'],
          reason: 'the effect keeps its slot in the stack');
      expect(chainEnabled(p.layer), [true, false],
          reason: 'and stops drawing - the state its own tick sets');
      expect(find.byKey(const ValueKey<String>('fx-console-bar')), findsNothing,
          reason: 'a wire being taken off is not a wire looking for a node');

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(chainEnabled(p.layer), [true, true],
          reason: 'one gesture, one undo step');
    });

    testWidgets('unplugging the Layer out leaves the layer drawing nothing',
        (tester) async {
      final p = withTwoEffects();
      await mount(tester, p);

      final at = tester
          .getCenter(find.byKey(const ValueKey<String>('graph-socket-out-image')));
      await tester.dragFrom(at, const Offset(40, 220));
      await tester.pump();

      expect(p.layer.getGraph().wiring.outUnwired, isTrue,
          reason: 'the layer itself is what came unplugged');
      expect(chainNames(p.layer), ['blur', 'exposure'],
          reason: 'and it cost the stack nothing');
      expect(chainEnabled(p.layer), [true, true],
          reason: 'no effect was bypassed by it either');

      // Back in: the wire lands on the Layer out and the layer draws again.
      final from = tester.getCenter(find.byKey(
          ValueKey<String>('graph-socket-effect:${stackIds(p.layer)[1]}-output')));
      await tester.dragFrom(from, at - from);
      await tester.pump();
      expect(p.layer.getGraph().wiring.outUnwired, isFalse);
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

    // --- Named groups -----------------------------------------------------

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

    /// **Naming a set is one act with two halves**: the wash appears on
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
          outUnwired: false,
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
      p.uiState.activePane.value = Panel.graph.pane();
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

    /// From Effects & presets: an effect joins the stack as the console's does,
    /// and a driver lands where it was let go. One op each.
    testWidgets('an effect or a driver dragged onto the canvas is added',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      p.uiState.selectedLayer.value = layer;
      p.uiState.model.refresh();
      const size = Size(900, 600);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      Widget source(String name, String key) => Draggable<EffectDragData>(
            data: EffectDragData(name, name),
            hitTestBehavior: HitTestBehavior.opaque,
            feedback: const SizedBox(width: 8, height: 8),
            child: SizedBox(key: ValueKey<String>(key), width: 60, height: 20),
          );
      await tester.pumpWidget(hostPanel(
        child: Column(children: [
          Row(children: [
            source('exposure', 'drag-effect'),
            source('wiggle', 'drag-driver'),
          ]),
          const Expanded(child: GraphPanelFrb()),
        ]),
        state: p.state,
        uiState: p.uiState,
        size: size,
      ));
      await tester.pump();
      final was = comp.documentRevision();

      await tester.drag(find.byKey(const ValueKey<String>('drag-effect')),
          const Offset(300, 350),
          warnIfMissed: false);
      await tester.pumpAndSettle();
      expect([for (final e in layer.getEffects()) e.getInfo().name],
          ['blur', 'exposure']);
      expect(comp.documentRevision(), was + BigInt.one, reason: 'one op');

      await tester.drag(find.byKey(const ValueKey<String>('drag-driver')),
          const Offset(300, 350),
          warnIfMissed: false);
      await tester.pumpAndSettle();
      final drivers = layer.getGraphDrivers();
      expect(drivers, hasLength(1));
      final canvas =
          tester.getTopLeft(find.byKey(const ValueKey('graph-canvas')));
      final from =
          tester.getTopLeft(find.byKey(const ValueKey<String>('drag-driver')));
      final place = layer.getGraph().wiring.layout.firstWhere(
          (l) => l.node == BridgeNodeRef.driver(drivers.single.id()));
      expect(Offset(place.x, place.y),
          from + const Offset(300, 350) - canvas,
          reason: 'the driver sits where it was let go');
      expect(comp.documentRevision(), was + BigInt.two, reason: 'one op');
    });
  });
}
