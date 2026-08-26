// The rule that decides whether an icon lands on the pixel grid or smears
// across it. It is arithmetic about stroke widths, so it is testable without
// looking at a screen — which matters, because the symptom (icons that read as
// "crunchy") is the kind of thing only ever reported by eye.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/icons.dart';

/// The stroke a glyph drawn on a 24-unit grid draws at [size], in logical
/// pixels: 1.5 of those units. The sizes in `icons.dart` were chosen against
/// this arithmetic and the reasoning is kept there.
double strokeAt(double size) => size / 24.0 * 1.5;

void main() {
  test('the icon sizes are ones a pixel can hold', () {
    // The whole point of 16 rather than the 12-13 the panels used to pass: a
    // stroke narrower than a pixel has nowhere to land, and the renderer
    // spreads it over two at partial strength.
    expect(strokeAt(iconSize), 1.0);
    expect(strokeAt(iconSizeTransport), 1.25);
    expect(strokeAt(12), lessThan(1.0), reason: 'the size that looked crunchy');
  });

  /// The nudge is gone with the glyphs it existed for.
  ///
  /// A one-pixel stroke drawn along a whole-pixel coordinate covers half of the
  /// pixel each side and comes out doubled and grey, so every borrowed glyph
  /// used to be shifted half a pixel across on its way to the screen. Lumit's
  /// own set carries that offset in the drawings — its coordinates sit on half
  /// units of a 16-unit grid — so a shift on top of it would take the strokes
  /// *off* the centres they are already on. Now that the set draws everything,
  /// nothing may be nudged, at any display scaling.
  testWidgets('no icon is shifted on its way to the screen', (tester) async {
    for (final ratio in const [1.0, 1.5, 2.0, 3.0]) {
      for (final icon in LumitIcon.values) {
        await tester.pumpWidget(MediaQuery(
          data: MediaQueryData(devicePixelRatio: ratio),
          child: Directionality(
            textDirection: TextDirection.ltr,
            child: Center(
              child: lumitIcon(icon,
                  size: iconSize, color: const Color(0xffffffff)),
            ),
          ),
        ));
        expect(find.byType(Transform), findsNothing,
            reason: '${icon.name} at a ratio of $ratio');
      }
    }
  });
}
