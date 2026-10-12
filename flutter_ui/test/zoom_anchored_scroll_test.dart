// The zoom's anchored scroll (docs/07-UI-SPEC.md §4.6): the arithmetic
// on its own, and the correction happening inside layout rather than beside it.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/widgets/zoom_anchored_scroll.dart';

void main() {
  group('The anchored scroll', () {
    /// Build a horizontal scroller of [width] over a 200-pixel viewport.
    Future<void> show(
      WidgetTester tester,
      ZoomAnchoredScrollController controller,
      double width,
    ) async {
      await tester.pumpWidget(Directionality(
        textDirection: TextDirection.ltr,
        child: Align(
          alignment: Alignment.topLeft,
          child: SizedBox(
            width: 200,
            height: 50,
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              controller: controller,
              child: SizedBox(width: width, height: 50),
            ),
          ),
        ),
      ));
    }

    testWidgets('holds the anchored frame where it was, as the content grows',
        (tester) async {
      final controller = ZoomAnchoredScrollController();
      addTearDown(controller.dispose);
      await show(tester, controller, 1000);
      controller.jumpTo(460);
      await tester.pump();
      // Frame 50 of 100 is at x=500 in 1000 pixels, so it is showing at x=40.
      expect(controller.offset, 460);

      // Zoom: the content doubles, and the frame must stay at x=40.
      controller.hold(
          const ZoomAnchor(frame: 50, viewportX: 40, frames: 100));
      await show(tester, controller, 2000);
      expect(controller.offset, closeTo(960, 0.01),
          reason: 'frame 50 is at x=1000 now, and 1000 - 960 is still x=40');
    });

    /// One-shot on purpose: an anchor that outlived the zoom would be applied
    /// by the next unrelated layout — a window resize — and drag the view back
    /// to a zoom the reader had since scrolled away from.
    testWidgets('an anchor is spent by the layout that applies it',
        (tester) async {
      final controller = ZoomAnchoredScrollController();
      addTearDown(controller.dispose);
      await show(tester, controller, 1000);
      controller.hold(const ZoomAnchor(frame: 50, viewportX: 40, frames: 100));
      await show(tester, controller, 2000);
      expect(controller.anchor, isNull, reason: 'the zoom spent it');

      // An ordinary scroll afterwards stays where it is put, through as many
      // further layouts as the panel cares to do.
      controller.jumpTo(100);
      await show(tester, controller, 1600);
      await show(tester, controller, 1600);
      expect(controller.offset, 100,
          reason: 'a spent anchor cannot pull the view back to the zoom');
    });
  });
}
