// Timeline snapping arithmetic (docs/07-UI-SPEC.md §4.5).
//
// Pure, so checked here against hand-computed cases rather than by dragging in
// a widget tree — the same reasoning timeline_drag_test.dart follows for the
// row-height maths.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_snap.dart';

void main() {
  group('What a drag lands on', () {
    // Ten pixels per frame: the default eight-pixel slop is therefore
    // 0.8 frames, which makes every case below easy to reason about.
    const perFrame = 10.0;

    test('the magnet off leaves the frame exactly where the pointer put it',
        () {
      final r = snapFrame(
        frame: 12.37,
        targets: const [SnapTarget(12, SnapKind.playhead)],
        perFrame: perFrame,
        magnet: false,
      );
      expect(r.frame, 12.37, reason: 'a key may sit between frames');
      expect(r.caught, isNull);
    });

    test('with nothing near, the magnet still rounds to a whole frame', () {
      final r = snapFrame(
        frame: 12.37,
        targets: const [SnapTarget(40, SnapKind.marker)],
        perFrame: perFrame,
        magnet: true,
      );
      expect(r.frame, 12);
      expect(r.caught, isNull, reason: 'a whole frame is not a target to draw');
    });

    test('a target within the slop takes the drag exactly', () {
      final r = snapFrame(
        frame: 12.37,
        targets: const [SnapTarget(12.5, SnapKind.marker)],
        perFrame: perFrame,
        magnet: true,
      );
      expect(r.frame, 12.5, reason: 'it lands ON the marker, not near it');
      expect(r.caught, const SnapTarget(12.5, SnapKind.marker));
    });

    test('a target just outside the slop does not reach', () {
      // 0.8 frames is the slop at this zoom; 0.9 away is outside it.
      final r = snapFrame(
        frame: 12.0,
        targets: const [SnapTarget(12.9, SnapKind.marker)],
        perFrame: perFrame,
        magnet: true,
      );
      expect(r.frame, 12, reason: 'the whole-frame fallback, not the marker');
      expect(r.caught, isNull);
    });

    test('the nearest target wins when several are in reach', () {
      final r = snapFrame(
        frame: 12.4,
        targets: const [
          SnapTarget(12.0, SnapKind.layerIn),
          SnapTarget(12.5, SnapKind.marker),
          SnapTarget(12.7, SnapKind.playhead),
        ],
        perFrame: perFrame,
        magnet: true,
      );
      expect(r.caught?.kind, SnapKind.marker);
      expect(r.frame, 12.5);
    });

    test('two equally close targets keep the first, so gathering order is '
        'not a hidden input', () {
      final r = snapFrame(
        frame: 12.5,
        targets: const [
          SnapTarget(12.4, SnapKind.keyframe),
          SnapTarget(12.6, SnapKind.marker),
        ],
        perFrame: perFrame,
        magnet: true,
      );
      expect(r.caught?.kind, SnapKind.keyframe);
    });

    /// **The spec's rule that makes snapping feel right at every zoom**:
    /// distance is measured in screen pixels, never in time. The same target,
    /// the same distance in frames, is caught when zoomed out and missed when
    /// zoomed in.
    test('the reach is in pixels, so the magnification is the precision', () {
      const target = [SnapTarget(14, SnapKind.marker)];
      // Zoomed out: two frames away is 8 px at 4 px/frame... just outside.
      expect(
        snapFrame(
                frame: 12, targets: target, perFrame: 4.0, magnet: true)
            .caught,
        isNull,
      );
      // Zoomed further out: the same two frames is 6 px, and it catches.
      expect(
        snapFrame(frame: 12, targets: target, perFrame: 3.0, magnet: true)
            .caught,
        const SnapTarget(14, SnapKind.marker),
      );
      // Zoomed in: 100 px away, nowhere near.
      expect(
        snapFrame(frame: 12, targets: target, perFrame: 50.0, magnet: true)
            .caught,
        isNull,
      );
    });

    test('a collapsed axis snaps to whole frames and reaches for nothing', () {
      final r = snapFrame(
        frame: 12.37,
        targets: const [SnapTarget(12.5, SnapKind.marker)],
        perFrame: 0,
        magnet: true,
      );
      expect(r.frame, 12);
      expect(r.caught, isNull, reason: 'never a division by zero');
    });

    test('no targets at all is the whole-frame magnet, unchanged (K-190)', () {
      expect(
        snapFrame(
                frame: 12.6, targets: const [], perFrame: perFrame, magnet: true)
            .frame,
        13,
      );
    });

    test('every kind of target is reachable, because each is a snap the spec '
        'names', () {
      for (final kind in SnapKind.values) {
        final r = snapFrame(
          frame: 12.4,
          targets: [SnapTarget(12.5, kind)],
          perFrame: perFrame,
          magnet: true,
        );
        expect(r.caught?.kind, kind, reason: '$kind must be snappable');
      }
    });
  });

  group('Suspending a snap mid-drag', () {
    test('Ctrl held suspends it', () {
      expect(snapSuspended(controlPressed: true), isTrue);
      expect(snapSuspended(controlPressed: false), isFalse);
    });
  });

  /// **A cut lands on a frame, and the line says where** (owner, 2026-08-06).
  ///
  /// The cut was always quantised — `TimelineAxis.frameAt` rounds — while the
  /// blade's line followed the pointer continuously, so the two disagreed by up
  /// to half a frame and the mark stood where the edge did not bite. Both now
  /// read one function; these are the cases that function has to get right.
  group('Where a razor cut lands', () {
    const perFrame = 10.0;

    /// The razor's rule, as the panel applies it: snap, then round, because a
    /// clip boundary is a whole frame whatever caught it.
    double razorFrame(double x, List<SnapTarget> targets, {bool magnet = true}) =>
        snapFrame(
          frame: x / perFrame,
          targets: targets,
          perFrame: perFrame,
          magnet: magnet,
        ).frame.roundToDouble();

    test('with nothing near, a cut lands on the nearest frame', () {
      expect(razorFrame(124, const []), 12);
      expect(razorFrame(126, const []), 13);
    });

    test('the magnet off still lands on a frame, because a clip boundary is '
        'one', () {
      expect(razorFrame(126, const [], magnet: false), 13);
    });

    test('a marker in reach takes the cut, and it is still a whole frame', () {
      // The marker sits between frames; the cut may not.
      expect(
        razorFrame(124, const [SnapTarget(12.4, SnapKind.marker)]),
        12,
      );
      expect(
        razorFrame(126, const [SnapTarget(12.6, SnapKind.marker)]),
        13,
      );
    });

    test('an edit point in reach takes it exactly', () {
      expect(razorFrame(124, const [SnapTarget(13, SnapKind.editPoint)]), 13);
    });
  });
}