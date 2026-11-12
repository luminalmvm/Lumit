// Scrolling the Timeline: the gesture a Mac trackpad makes, the two halves
// staying level, and the middle button dragging the view about.
//
// **Both of these were reported from a real Mac and neither shows on a mouse.**
// A two-finger trackpad scroll arrives as a *pan gesture*, not as the wheel's
// pointer signal, so the lane area's marquee — a pan recogniser over the whole
// ground — won it in the arena and the panel could not be scrolled at all. And
// the lane side carries a bottom bar the outline did not, so its rows had a
// shorter viewport, scrolled further, and the halves came apart at the end of a
// long stack.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart' show TimelineRuler;
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Timeline scrolling (frb)', () {
    /// A stack tall enough that both halves have somewhere to scroll to.
    ({LumitState state, LumitUiState uiState}) withManyLayers() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      for (var i = 0; i < 30; i++) {
        comp.addSolidLayer();
      }
      p.uiState.setSelectedComp(comp);
      return p;
    }

    Future<void> mount(
        WidgetTester tester, ({LumitState state, LumitUiState uiState}) p) async {
      tester.view.physicalSize = const Size(1400, 500);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(1400, 500),
        child: const TimelinePanelFrb(),
      ));
      await tester.pump();
      await settleFrb(tester, minRounds: 6);
    }

    /// Every vertical scrollable in the panel, by the position each holds.
    List<ScrollPosition> verticalPositions(WidgetTester tester) => tester
        .stateList<ScrollableState>(find.byType(Scrollable))
        .map((s) => s.position)
        .where((p) => p.axis == Axis.vertical && p.maxScrollExtent > 0)
        .toList();

    testWidgets('a trackpad two-finger scroll moves the rows', (tester) async {
      final p = withManyLayers();
      await mount(tester, p);

      final before = verticalPositions(tester).map((p) => p.pixels).toList();
      expect(before, isNotEmpty, reason: 'there is somewhere to scroll to');

      // The gesture a Mac trackpad makes: a pan-zoom sequence, not a wheel.
      final pad = await tester.createGesture(kind: PointerDeviceKind.trackpad);
      await pad.panZoomStart(tester.getCenter(find.byType(TimelinePanelFrb)));
      await tester.pump();
      await pad.panZoomUpdate(
        tester.getCenter(find.byType(TimelinePanelFrb)),
        pan: const Offset(0, -120),
      );
      await tester.pump();
      await pad.panZoomEnd();
      await tester.pumpAndSettle();

      final after = verticalPositions(tester).map((p) => p.pixels).toList();
      expect(after.any((px) => px > 0), isTrue,
          reason: 'the trackpad scrolls the panel rather than drawing a '
              'selection box over it');
    });

    /// The lanes' one horizontal position, once there is somewhere to scroll to.
    ScrollPosition horizontalPosition(WidgetTester tester) => tester
        .stateList<ScrollableState>(find.byType(Scrollable))
        .map((s) => s.position)
        .firstWhere(
            (p) => p.axis == Axis.horizontal && p.maxScrollExtent > 0);

    /// Zoom time in far enough that the lanes are wider than the panel, so a
    /// horizontal pan has room to move.
    Future<void> zoomIn(WidgetTester tester, Offset at) async {
      final pointer = TestPointer(1, PointerDeviceKind.mouse);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendEventToBinding(pointer.hover(at));
      for (var i = 0; i < 6; i++) {
        await tester.sendEventToBinding(pointer.scroll(const Offset(0, -1)));
        await tester.pump();
      }
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();
    }

    /// A drag over the lanes, on whichever button is asked for.
    Future<void> drag(WidgetTester tester, Offset from, Offset by,
        {required int buttons}) async {
      final pointer = TestPointer(2, PointerDeviceKind.mouse, null, buttons);
      await tester.sendEventToBinding(pointer.down(from));
      await tester.pump();
      // In steps, because a drag arrives as a stream of small moves and the
      // pan adds each one up.
      for (var i = 1; i <= 4; i++) {
        await tester.sendEventToBinding(pointer.move(from + by * (i / 4)));
        await tester.pump();
      }
      await tester.sendEventToBinding(pointer.up());
      await tester.pump();
    }


    /// A scrub taken past the lanes' right end scrolls them, and the playhead
    /// stays under the pointer once it moves again. The drag's own local
    /// position is measured through the press's transform, so it used to put
    /// the playhead back by however far the lanes had scrolled.
    testWidgets('a scrub past the edge keeps the playhead under the pointer',
        (tester) async {
      final p = withManyLayers();
      await mount(tester, p);
      final lanes = tester.getCenter(find.byType(LayerArea));
      await zoomIn(tester, lanes);
      await tester.sendEventToBinding(
          TestPointer(1, PointerDeviceKind.mouse).hover(lanes));
      final position = horizontalPosition(tester);
      // Back to the start, so there is room to scroll towards the end.
      position.jumpTo(0);
      await tester.pump();

      final ruler = find.byKey(const ValueKey('tl-ruler'));
      // The ruler is as wide as the comp: its visible part is the viewport's.
      final view = tester
          .stateList<ScrollableState>(find.byType(Scrollable))
          .firstWhere((s) => s.position == position)
          .context
          .findRenderObject()! as RenderBox;
      final seen = view.localToGlobal(Offset.zero) & view.size;
      final rulerRect = tester.getRect(ruler);
      final rect = Rect.fromLTRB(
          seen.left, rulerRect.top, seen.right, rulerRect.bottom);
      final pointer = TestPointer(3, PointerDeviceKind.mouse, null, kPrimaryButton);
      final from = Offset(rect.left + rect.width * 0.5, rect.center.dy);
      await tester.sendEventToBinding(pointer.down(from));
      await tester.pump();
      final edge = Offset(rect.right - 4, rect.center.dy);
      for (var i = 1; i <= 6; i++) {
        await tester.sendEventToBinding(pointer.move(Offset.lerp(from, edge, i / 6)!));
        await tester.pump();
      }
      // Let the edge scroll run.
      for (var i = 0; i < 20; i++) {
        await tester.pump(const Duration(milliseconds: 50));
      }
      expect(position.pixels, greaterThan(0), reason: 'the lanes scrolled');
      // A step back from the edge, which stops the scroll.
      final back = Offset(rect.right - 200, rect.center.dy);
      await tester.sendEventToBinding(pointer.move(back));
      await tester.pump();

      final box = tester.renderObject<RenderBox>(ruler);
      final axis = tester.widget<TimelineRuler>(find.byType(TimelineRuler)).axis;
      final under = axis.frameAt(box.globalToLocal(back).dx);
      expect(p.uiState.playheadFrame.value, closeTo(under, 1),
          reason: 'the playhead is the frame under the pointer');
      await tester.sendEventToBinding(pointer.up());
      await tester.pumpAndSettle();
    });
    /// **The middle button drags the view about** (docs/07 §4.6), the way it
    /// does in After Effects, Blender and Resolve. Both ways at once, and the
    /// view moves against the drag so the lanes follow the pointer.
    testWidgets('a middle-button drag pans the lanes both ways',
        (tester) async {
      final p = withManyLayers();
      await mount(tester, p);

      final lanes = tester.getCenter(find.byType(LayerArea));
      await zoomIn(tester, lanes);

      final acrossBefore = horizontalPosition(tester).pixels;
      expect(acrossBefore, greaterThan(0),
          reason: 'the zoom left the lanes somewhere to scroll from');
      expect(verticalPositions(tester).every((p) => p.pixels == 0), isTrue,
          reason: 'and the rows start at the top');

      // Right and up, so the lanes go left and down under the pointer.
      await drag(tester, lanes, const Offset(60, -80),
          buttons: kMiddleMouseButton);

      expect(horizontalPosition(tester).pixels, lessThan(acrossBefore),
          reason: 'dragging right takes the view back in time');
      expect(verticalPositions(tester).any((p) => p.pixels > 0), isTrue,
          reason: 'and dragging up takes it down the stack');
    });
  });
}
