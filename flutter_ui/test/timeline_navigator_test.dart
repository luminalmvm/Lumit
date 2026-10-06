// The time navigator's arithmetic (T5): what window a scroll position implies,
// and what window a drag on it asks for.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_navigator.dart';

void main() {
  group('the window a scroll position implies', () {
    test('fit-to-panel is the whole composition', () {
      final w =
          navigatorWindow(offset: 0, viewport: 612, content: 612, frames: 100);
      expect(w.start, 0);
      expect(w.end, 100);
    });

    test('zoomed in, it is the slice the lanes are showing', () {
      // Four times in: the content is four viewports wide, so a quarter of the
      // comp is on screen, and an offset of one viewport starts it a quarter in.
      final w = navigatorWindow(
          offset: 612, viewport: 612, content: 612 * 4, frames: 100);
      expect(w.start, closeTo(25, 0.5));
      expect(w.end, closeTo(50, 0.5));
    });

    test('the window never leaves the composition', () {
      // The axis pads both ends, so the raw numbers run outside the comp at
      // fit — a window drawn there would hang off the strip.
      final w = navigatorWindow(
          offset: 0, viewport: 612, content: 612, frames: 100, pad: 6);
      expect(w.start, 0);
      expect(w.end, 100);
      // A degenerate comp answers with a degenerate window rather than a
      // division by nothing.
      final none =
          navigatorWindow(offset: 0, viewport: 0, content: 0, frames: 0);
      expect(none, (start: 0.0, end: 0.0));
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

    test('a pan stops at either end of the composition', () {
      expect(
        navigatorDrag(
            grab: NavigatorGrab.body,
            frame: 0,
            hold: 10,
            start: 20,
            end: 40,
            frames: 100),
        (start: 0.0, span: 20.0),
      );
      expect(
        navigatorDrag(
            grab: NavigatorGrab.body,
            frame: 100,
            hold: 10,
            start: 20,
            end: 40,
            frames: 100),
        (start: 80.0, span: 20.0),
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

    test('an end cannot be dragged through the other, or off the comp', () {
      // A window of no width is a view of no frames, and the magnification it
      // implies is a division by nothing.
      expect(
        navigatorDrag(
            grab: NavigatorGrab.end, frame: 5, start: 20, end: 40, frames: 100),
        (start: 20.0, span: 1.0),
      );
      expect(
        navigatorDrag(
            grab: NavigatorGrab.start,
            frame: 90,
            start: 20,
            end: 40,
            frames: 100),
        (start: 39.0, span: 1.0),
      );
      expect(
        navigatorDrag(
            grab: NavigatorGrab.end,
            frame: 500,
            start: 20,
            end: 40,
            frames: 100),
        (start: 20.0, span: 80.0),
      );
    });

    test('a pan can reach the padding at either end, as the scrollbar can', () {
      // Fit-to-panel shows the whole comp and the padding either side, so the
      // raw view runs from -overhang to frames + overhang.
      expect(
        navigatorDrag(
            grab: NavigatorGrab.body,
            frame: 0,
            hold: 10,
            start: 20,
            end: 40,
            frames: 100,
            overhang: 1),
        (start: -1.0, span: 20.0),
      );
      expect(
        navigatorDrag(
            grab: NavigatorGrab.body,
            frame: 500,
            hold: 10,
            start: -1,
            end: 101,
            frames: 100,
            overhang: 1),
        (start: -1.0, span: 102.0),
      );
    });
  });

  group('the zoom a window asks for', () {
    test('is the zoom that shows it, padding and all', () {
      // Read the window off a zoomed scroll and ask for it back: the same zoom.
      // `frames / span` came back a hair wider, and a pan asks on every move.
      for (final zoom in [1.5, 4.0, 37.0]) {
        const viewport = 612.0;
        final w = navigatorWindow(
            offset: 300,
            viewport: viewport,
            content: viewport * zoom,
            frames: 100,
            clamp: false);
        expect(
          navigatorZoom(span: w.end - w.start, frames: 100, viewport: viewport),
          closeTo(zoom, 1e-9),
        );
      }
    });

    test('the whole comp is fit-to-panel', () {
      expect(navigatorZoom(span: 100, frames: 100, viewport: 612), 1);
      expect(navigatorZoom(span: 0, frames: 100, viewport: 612), 1);
    });
  });
}
