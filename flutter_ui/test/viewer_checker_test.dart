// The transparency board behind the picture (K-230): how much of it is painted.
//
// The board is drawn a square at a time, so what it costs is the *number of
// squares*, and that number has to stay the same however far the picture is
// zoomed in. It did not: the board was a widget the size of the picture, so at
// 800 % on an HD composition it was 15360 pixels across and cost half a million
// rectangles a paint for the few thousand on screen — which is what made
// zooming in seize the whole window.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';

void main() {
  const panel = Size(900, 600);

  /// Roughly how many 8-pixel squares an area of this size costs to paint.
  double squares(Rect area) => area.isEmpty ? 0 : (area.width / 8) * (area.height / 8);

  group('The transparency board', () {
    test('a picture inside the panel is painted whole', () {
      final area = checkerArea(const Rect.fromLTWH(50, 20, 400, 300), panel);
      expect(area, const Rect.fromLTWH(50, 20, 400, 300));
    });

    test('a picture larger than the panel is painted only where it shows', () {
      // 800 % on an HD composition, centred: the picture is far bigger than the
      // panel in both directions.
      final huge = const Rect.fromLTWH(-7000, -4000, 15360, 8640);
      final area = checkerArea(huge, panel);
      expect(area, Offset.zero & panel,
          reason: 'the whole panel is board, and nothing beyond it is');
      expect(squares(area), lessThan(10000),
          reason: 'the cost is the panel\'s, not the picture\'s — it was '
              'over half a million');
    });

    test('the cost does not grow as the picture is zoomed in', () {
      final costs = <double>[];
      for (final scale in [1.0, 4.0, 16.0, 64.0]) {
        final width = 1920 * scale;
        final height = 1080 * scale;
        final picture = Rect.fromLTWH(
          (panel.width - width) / 2,
          (panel.height - height) / 2,
          width,
          height,
        );
        costs.add(squares(checkerArea(picture, panel)));
      }
      for (final cost in costs) {
        expect(cost, lessThanOrEqualTo(squares(Offset.zero & panel)));
      }
    });

    test('a picture panned right off the panel costs nothing at all', () {
      final area = checkerArea(const Rect.fromLTWH(2000, 2000, 400, 300), panel);
      expect(area.isEmpty, isTrue);
    });
  });

  // The board and the picture are given the same rectangle, but a fractional
  // one rasterises differently through an anti-aliased canvas than through the
  // platform texture — a soft row of board stuck out under the picture at some
  // zooms. Snapping the shared rectangle to whole device pixels is the fix.
  group('snapToDevicePixels', () {
    test('lands every edge on a whole device pixel', () {
      final snapped = snapToDevicePixels(
          const Rect.fromLTRB(10.3, 20.7, 410.5, 320.2), 1.0);
      expect(snapped, const Rect.fromLTRB(10, 21, 411, 320));
    });

    test('snaps to the device grid, not the logical one', () {
      // At 150 % scaling a device pixel is two thirds of a logical one.
      final snapped =
          snapToDevicePixels(const Rect.fromLTRB(0, 0, 100.2, 50.4), 1.5);
      expect(snapped.right * 1.5, closeTo((100.2 * 1.5).roundToDouble(), 1e-9));
      expect(snapped.bottom * 1.5, closeTo((50.4 * 1.5).roundToDouble(), 1e-9));
    });

    test('whole pixels pass through untouched', () {
      const whole = Rect.fromLTRB(10, 20, 410, 320);
      expect(snapToDevicePixels(whole, 2.0), whole);
    });
  });
}
