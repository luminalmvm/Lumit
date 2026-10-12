// The transparency board behind the picture: how much of it is painted.
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
  });
}
