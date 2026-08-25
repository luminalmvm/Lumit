// The zoom's anchored scroll (docs/07-UI-SPEC.md §4.6, K-293): the arithmetic
// on its own, and the correction happening inside layout rather than beside it.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/widgets/zoom_anchored_scroll.dart';

void main() {
  group('Where the anchor puts the offset', () {
    test('a frame lands exactly on the point it was anchored to', () {
      // 100 frames over 1000 pixels of content in a 200-pixel viewport: ten
      // pixels a frame, so frame 50 sits at x=500 in the content and needs an
      // offset of 460 to show at x=40 on screen.
      final offset = zoomAnchorOffset(
        const ZoomAnchor(frame: 50, viewportX: 40, frames: 100),
        viewportDimension: 200,
        minScrollExtent: 0,
        maxScrollExtent: 800,
      );
      expect(offset, closeTo(460, 1e-9));
    });

    test('the same anchor at twice the width asks for twice as far in', () {
      double at(double maxScrollExtent) => zoomAnchorOffset(
            const ZoomAnchor(frame: 50, viewportX: 40, frames: 100),
            viewportDimension: 200,
            minScrollExtent: 0,
            maxScrollExtent: maxScrollExtent,
          );
      // Content 1000 → 2000 wide: the frame is now at x=1000, so the offset
      // that keeps it at x=40 is 960.
      expect(at(800), closeTo(460, 1e-9));
      expect(at(1800), closeTo(960, 1e-9));
    });

    test('the ends of the content are the ends', () {
      // Frame 0 held at x=100 would want a negative offset; there is nothing
      // to the left of the start.
      expect(
        zoomAnchorOffset(
          const ZoomAnchor(frame: 0, viewportX: 100, frames: 100),
          viewportDimension: 200,
          minScrollExtent: 0,
          maxScrollExtent: 800,
        ),
        0,
      );
      // And the last frame held at the left edge cannot pull the content
      // further than its own end.
      expect(
        zoomAnchorOffset(
          const ZoomAnchor(frame: 100, viewportX: 0, frames: 100),
          viewportDimension: 200,
          minScrollExtent: 0,
          maxScrollExtent: 800,
        ),
        800,
      );
    });

    test('a composition with no frames has nowhere to anchor', () {
      expect(
        zoomAnchorOffset(
          const ZoomAnchor(frame: 0, viewportX: 10, frames: 0),
          viewportDimension: 200,
          minScrollExtent: 0,
          maxScrollExtent: 0,
        ),
        0,
      );
    });
  });

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

    /// The whole point of correcting inside layout: the offset is never, at any
    /// moment a scrollbar could be drawn from, past the end of its own content.
    testWidgets('never leaves the offset past the end of the content',
        (tester) async {
      final controller = ZoomAnchoredScrollController();
      addTearDown(controller.dispose);
      await show(tester, controller, 2000);
      controller.jumpTo(1800);
      await tester.pump();

      // Zooming *out* shrinks the content under an offset that was valid for
      // the wider one — the case that used to leave the position out of range
      // and springing back.
      controller.hold(const ZoomAnchor(frame: 50, viewportX: 40, frames: 100));
      await show(tester, controller, 1000);
      expect(controller.offset, lessThanOrEqualTo(800),
          reason: 'the content is 1000 wide over a 200 viewport');
      expect(controller.offset, closeTo(460, 0.01));
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

    testWidgets('a released anchor does nothing', (tester) async {
      final controller = ZoomAnchoredScrollController();
      addTearDown(controller.dispose);
      await show(tester, controller, 1000);
      controller.jumpTo(0);
      controller.hold(const ZoomAnchor(frame: 50, viewportX: 40, frames: 100));
      controller.release();
      await show(tester, controller, 2000);
      expect(controller.offset, 0);
    });
  });
}
