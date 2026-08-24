// The two rules that decide whether an icon lands on the pixel grid or smears
// across it. Both are arithmetic about stroke widths, so both are testable
// without looking at a screen — which matters, because the symptom (icons that
// read as "crunchy") is the kind of thing only ever reported by eye.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/icons.dart';

/// The stroke an Iconoir glyph draws at [size], in logical pixels: 1.5 units
/// of its own 24-unit grid.
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

  /// The nudge: a one-pixel stroke drawn along a whole-pixel coordinate covers
  /// half of the pixel each side, so it comes out doubled and grey. Half a
  /// pixel across puts it on a pixel centre.
  ///
  /// Drawn here with the magnet, which is one of the icons Lumit's own set has
  /// no glyph for yet and so still comes from Iconoir: the nudge is Iconoir's
  /// 24-unit grid meeting the pixel grid. The own set carries its offset in the
  /// drawings, and is deliberately not nudged again.
  testWidgets(
      'a one-pixel stroke is nudged onto the grid, a two-pixel one is'
      ' left alone', (tester) async {
    Future<Offset> translationAt(double ratio, double size) async {
      await tester.pumpWidget(MediaQuery(
        data: MediaQueryData(devicePixelRatio: ratio),
        child: Directionality(
          textDirection: TextDirection.ltr,
          child: Center(
            child: lumitIcon(LumitIcon.magnet,
                size: size, color: const Color(0xffffffff)),
          ),
        ),
      ));
      final transform = tester.widget<Transform>(find.descendant(
        of: find.byType(SizedBox).first,
        matching: find.byType(Transform),
      ));
      return Offset(transform.transform.getTranslation().x,
          transform.transform.getTranslation().y);
    }

    // 100% scaling: a 1px stroke, so half a pixel across.
    expect(await translationAt(1.0, iconSize), const Offset(0.5, 0.5));
    // 200%: the stroke is a whole 2 device pixels and already covers them —
    // moving it is what would blur it.
    expect(await translationAt(2.0, iconSize), Offset.zero);
    // 150% (the common Windows setting): 1.5 and 1.875 device pixels, both of
    // which land nearest 2 — even, so left alone. Neither is a whole number of
    // pixels; there is nothing a nudge can do about that.
    expect(await translationAt(1.5, iconSize), Offset.zero);
    expect(await translationAt(1.5, iconSizeTransport), Offset.zero);
    // 300% on a panel icon: 3 device pixels, odd again, so half of one.
    expect(await translationAt(3.0, iconSize), Offset(0.5 / 3.0, 0.5 / 3.0));
  });
}
