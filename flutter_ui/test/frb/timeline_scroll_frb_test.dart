// Scrolling the Timeline: the gesture a Mac trackpad makes, and the two halves
// staying level.
//
// **Both of these were reported from a real Mac and neither shows on a mouse.**
// A two-finger trackpad scroll arrives as a *pan gesture*, not as the wheel's
// pointer signal, so the lane area's marquee — a pan recogniser over the whole
// ground — won it in the arena and the panel could not be scrolled at all. And
// the lane side carries a bottom bar the outline did not, so its rows had a
// shorter viewport, scrolled further, and the halves came apart at the end of a
// long stack.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
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

    testWidgets('both halves can scroll exactly as far as each other',
        (tester) async {
      final p = withManyLayers();
      await mount(tester, p);

      final extents = verticalPositions(tester)
          .map((p) => p.maxScrollExtent)
          .toSet()
          .toList();
      expect(extents, hasLength(1),
          reason: 'the outline reserves the lane bottom bar\'s height, so one '
              'half cannot run past the other: $extents');
    });
  });
}
