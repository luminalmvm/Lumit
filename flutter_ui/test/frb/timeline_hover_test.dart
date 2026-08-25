// Hover and cursors on the Timeline (docs/impl/timeline-interaction.md §4.1,
// §4.2, §7 — polish 25 and 26, TI-10).
//
// Two of the study's rules are asserted here. **P2 — the cursor is the
// affordance**: every distinct grab offers a distinct pointer before the button
// goes down, and a surface that will do nothing shows the plain arrow. **P1 —
// feedback is transient and local**: what a hover summons appears under the
// pointer and leaves with it, and the resting panel keeps every pixel it had.
//
// Driven through the real panel like every other frb test: a cursor nobody can
// reach is not an affordance.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('The hover ladder and the cursor table (TI-10)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(1280, 600),
      ));
      await tester.pump();
    }

    /// A real mouse the framework's tracker follows — `TestPointer.hover` sent
    /// straight to the binding never fires a `MouseRegion`, so a test using it
    /// would pass whatever the widget does.
    Future<TestGesture> mouse(WidgetTester tester) async {
      final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await gesture.addPointer(location: Offset.zero);
      addTearDown(gesture.removePointer);
      return gesture;
    }

    Finder barCursor(LayerReference layer) =>
        find.byKey(ValueKey<String>('tl-bar-cursor-${layer.internallayerId}'));

    MouseCursor cursorOn(WidgetTester tester, Finder f) =>
        tester.widget<MouseRegion>(f).cursor;

    Color edgeInk(WidgetTester tester, LayerReference layer) => tester
        .widget<ColoredBox>(find.descendant(
          of: find
              .byKey(ValueKey<String>('tl-bar-edge-${layer.internallayerId}')),
          matching: find.byType(ColoredBox),
        ))
        .color;

    // -----------------------------------------------------------------------
    // §4.1 — the bar.
    // -----------------------------------------------------------------------

    /// P2: a bar's body is a handle, and it says so before the button goes
    /// down. It said nothing at all until now — only the two end strips did.
    testWidgets('a bar\'s body offers the grab, and grips while it is held',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      expect(cursorOn(tester, barCursor(layer)), SystemMouseCursors.grab,
          reason: 'the body of a bar can be picked up (§4.1)');

      final rect = tester.getRect(barCursor(layer));
      final gesture = await tester.startGesture(
        Offset(rect.center.dx, rect.center.dy),
        kind: PointerDeviceKind.mouse,
      );
      await tester.pump(const Duration(milliseconds: 60));
      await gesture.moveBy(const Offset(20, 0));
      await tester.pump();

      expect(cursorOn(tester, barCursor(layer)), SystemMouseCursors.grabbing,
          reason: 'the hand closes while the bar is actually in it');

      await gesture.up();
      await tester.pumpAndSettle();
      expect(cursorOn(tester, barCursor(layer)), SystemMouseCursors.grab,
          reason: 'and opens again on release (P1)');
    });

    /// The trim zones lie over the body's own region and keep their arrows —
    /// the cursor table has to survive the region that was put underneath it.
    testWidgets('a bar\'s ends keep the resize arrows over the body\'s grab',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      expect(
        find.descendant(
          of: barCursor(layer),
          matching: find.byWidgetPredicate((w) =>
              w is MouseRegion &&
              w.cursor == SystemMouseCursors.resizeLeftRight),
        ),
        findsNWidgets(2),
        reason: 'both trim strips still say which way they pull (§4.1)',
      );
    });

    /// A locked bar is a fact, not a handle (§4.1): no move, no trim — so the
    /// plain arrow, and not `forbidden`, which belongs to a refused drop.
    testWidgets('a locked bar takes the plain arrow', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      await tester.tap(
          find.byKey(ValueKey<String>('tl-locked-${layer.internallayerId}')));
      await tester.pumpAndSettle();
      expect(layer.getSwitches().locked, isTrue);

      expect(cursorOn(tester, barCursor(layer)), SystemMouseCursors.basic,
          reason: 'nothing here can be grabbed, so nothing offers to be');
    });

    /// The other half of §4.1's plain-arrow sentence, which the package left
    /// unchecked: an **armed razor** is not a grab either. A press here cuts
    /// the layer, and a cut is not something the bar is picked up by, so the
    /// body drops its grab for as long as the tool is armed. (The study's
    /// scissors pointer stays deferred — gap 25 — so the plain arrow is the
    /// whole of the answer here.)
    testWidgets('an armed razor takes the grab off a bar', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);
      expect(cursorOn(tester, barCursor(layer)), SystemMouseCursors.grab);

      p.uiState.tools.select(ToolMode.razor);
      await tester.pumpAndSettle();
      expect(cursorOn(tester, barCursor(layer)), SystemMouseCursors.basic,
          reason: 'a press would cut, not carry (§4.1)');

      p.uiState.tools.select(ToolMode.select);
      await tester.pumpAndSettle();
      expect(cursorOn(tester, barCursor(layer)), SystemMouseCursors.grab,
          reason: 'and disarming gives the handle back (P1)');
    });

    /// Polish 26: a hovered bar firms its leading edge — and puts it straight
    /// back when the pointer leaves. Nothing at rest (P1).
    testWidgets('hovering a bar lifts its leading edge, and lets it go',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      await mount(tester, p);

      final resting = edgeInk(tester, layer);
      final gesture = await mouse(tester);
      await gesture.moveTo(tester.getCenter(barCursor(layer)));
      await tester.pumpAndSettle();

      final hovered = edgeInk(tester, layer);
      expect(hovered, isNot(resting),
          reason: 'the edge answers the pointer (§4.1)');

      await gesture.moveTo(Offset.zero);
      await tester.pumpAndSettle();
      expect(edgeInk(tester, layer), resting,
          reason: 'and the resting bar is exactly what it was (P1)');
    });

    // -----------------------------------------------------------------------
    // §4.2 — the lane keys.
    // -----------------------------------------------------------------------

    /// A keyed opacity, twirled open, so the property draws its own lane.
    Future<LayerReference> keyedRow(WidgetTester tester, dynamic p) async {
      final comp = p.comp as CompositionReference;
      final layer = comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [200, 800])
            BridgeKeyframe(
              time: comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      (p.uiState as LumitUiState).model.refresh();
      await mount(tester, p);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-twirl-${layer.internallayerId}')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Transform'));
      await tester.pumpAndSettle();
      return layer;
    }

    /// Which key the lane's painter is brightening this frame — the painter's
    /// own answer, which is what a person watching the row actually sees.
    int? hoveredOn(WidgetTester tester, LayerReference layer) {
      final lane = find.byKey(ValueKey<String>(
          'tl-keys-${layer.internallayerId}/transform/opacity'));
      final paint = find.descendant(
        of: find.descendant(
          of: lane,
          matching: find.byKey(const ValueKey<String>('tl-lane-diamonds')),
        ),
        matching: find.byType(CustomPaint),
      );
      return (tester.widget<CustomPaint>(paint.first).painter as dynamic)
          .hovered as int?;
    }

    /// Polish 26, §4.2: the key under the pointer brightens — the
    /// pre-selection hint — and nothing else on the row moves or appears.
    testWidgets('a hovered lane key brightens, and only that one',
        (tester) async {
      final p = withComp();
      final layer = await keyedRow(tester, p);

      expect(hoveredOn(tester, layer), isNull,
          reason: 'a resting lane brightens nothing');

      final slot = find.byKey(ValueKey<String>(
          'tl-key-slot-${layer.internallayerId}/transform/opacity#1'));
      expect(slot, findsOneWidget, reason: 'the second key has a grab slot');

      final gesture = await mouse(tester);
      await gesture.moveTo(tester.getCenter(slot));
      await tester.pumpAndSettle();

      expect(hoveredOn(tester, layer), 1,
          reason: 'the key under the pointer is the one that answers');
      expect(
          find.byKey(const ValueKey<String>('tl-key-drag-hint')), findsNothing,
          reason: 'the time and the value wait for a drag (§4.2, P1)');

      await gesture.moveTo(Offset.zero);
      await tester.pumpAndSettle();
      expect(hoveredOn(tester, layer), isNull,
          reason: 'and the row goes back to what it was (P1)');
    });

    // -----------------------------------------------------------------------
    // §7 — the marker flag.
    // -----------------------------------------------------------------------

    /// Polish 26: a marker's pill brightens one step under the pointer. The
    /// flag is one widget wherever it is drawn, so the ruler's markers and a
    /// layer's own answer the same way.
    testWidgets('a hovered marker flag lifts its pill, and puts it back',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      addMarkerFrb(p.comp, frame: 40, label: 'Chorus');
      await mount(tester, p);

      final id = p.comp.getMarkers().single.id;
      final flag = find.byKey(ValueKey<String>('tl-marker-$id'));
      expect(flag, findsOneWidget);

      Color pillInk() => (tester
              .widget<Container>(find.descendant(
                of: flag,
                matching: find.byType(Container),
              ))
              .decoration! as BoxDecoration)
          .color!;

      final resting = pillInk();
      final gesture = await mouse(tester);
      await gesture.moveTo(tester.getCenter(flag));
      await tester.pumpAndSettle();

      expect(pillInk(), isNot(resting), reason: 'the pill answers the pointer');

      await gesture.moveTo(Offset.zero);
      await tester.pumpAndSettle();
      expect(pillInk(), resting, reason: 'and nothing is left behind (P1)');
    });
  }, skip: !engineAvailable);
}
