// The time navigator's arithmetic (T5): what window a scroll position implies,
// and what window a drag on it asks for.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_navigator.dart';

void main() {
  group('the window a scroll position implies', () {
    test('zoomed in, it is the slice the lanes are showing', () {
      // Four times in: the content is four viewports wide, so a quarter of the
      // comp is on screen, and an offset of one viewport starts it a quarter in.
      final w = navigatorWindow(
          offset: 612, viewport: 612, content: 612 * 4, frames: 100);
      expect(w.start, closeTo(25, 0.5));
      expect(w.end, closeTo(50, 0.5));
    });
  });

  group('what a drag asks for', () {
    test('the body pans relative to where it was taken hold of', () {
      // Grabbed five frames in and dragged to 50: the frame under the pointer
      // stays under the pointer. Centring on the pointer instead would make the
      // window jump the moment it was grabbed anywhere but its exact middle.
      final d = navigatorDrag(
          grab: NavigatorGrab.body,
          frame: 50,
          hold: 5,
          start: 20,
          end: 40,
          frames: 100);
      expect(d.span, 20, reason: 'a pan is not a zoom');
      expect(d.start, 45);
      // A press on the bare track has no frame to keep: the caller asks for
      // half the span and the window arrives centred.
      expect(
        navigatorDrag(
            grab: NavigatorGrab.body,
            frame: 50,
            hold: 10,
            start: 20,
            end: 40,
            frames: 100),
        (start: 40.0, span: 20.0),
      );
    });

    test('an end zooms about the end that was not taken hold of', () {
      // Dragging the right-hand end leaves the left where it is, which is what
      // keeps the frame the eye is on from moving.
      final right = navigatorDrag(
          grab: NavigatorGrab.end, frame: 60, start: 20, end: 40, frames: 100);
      expect(right, (start: 20.0, span: 40.0));
      final left = navigatorDrag(
          grab: NavigatorGrab.start,
          frame: 10,
          start: 20,
          end: 40,
          frames: 100);
      expect(left, (start: 10.0, span: 30.0));
    });
  });
}
