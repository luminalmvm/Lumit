// The drawn pointers: the brush ring's size, and that each tool badges
// its own icon.
//
// The ring is the part with arithmetic in it — a brush width is in *picture*
// pixels and the ring is drawn on *screen*, so the magnification has to come
// into it — and the part that would be wrong silently.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_tool_cursor.dart';

void main() {
  group('The brush ring', () {
    test('is the stroke it would leave, at this magnification', () {
      // A 40px brush at 1:1 is a 20px radius; at half size, 10.
      expect(brushRingRadius(40, 1), 20);
      expect(brushRingRadius(40, 0.5), 10);
      expect(brushRingRadius(40, 2), 40);
    });
  });

  /// **A drawn pointer follows the pointer whichever button is held
  /// (docs/07 §2.3.3).** Taken from hover alone it froze on a right-press: a
  /// `MouseRegion` reports hover, and hover stops the moment *any* button goes
  /// down — including the secondary one, which none of these tools handle. The
  /// hand and the magnifier stand for the whole family; they share
  /// [DrawnPointerRegion], which is where the fix lives.
  group('The drawn pointer under a held button', () {
    Widget host(Widget layer) => Directionality(
          textDirection: TextDirection.ltr,
          child: Stack(children: [layer]),
        );

    /// Presses [buttons] at [from] and drags to [to], as a real mouse would.
    Future<void> dragWith(
      WidgetTester tester, {
      required int buttons,
      required Offset from,
      required Offset to,
    }) async {
      final gesture = await tester.createGesture(
        kind: PointerDeviceKind.mouse,
        buttons: buttons,
      );
      await gesture.addPointer(location: from);
      addTearDown(() => gesture.removePointer());
      await tester.pump();
      await gesture.down(from);
      await tester.pump();
      await gesture.moveTo(to);
      await tester.pump();
      await gesture.up();
      await tester.pump();
    }

    testWidgets('the Hand follows a right-drag', (tester) async {
      await tester.pumpWidget(host(ViewerHandLayer(
        active: true,
        onPan: (_) {},
        mark: const Color(0xffffffff),
        outline: const Color(0xff000000),
      )));

      await dragWith(
        tester,
        buttons: kSecondaryButton,
        from: const Offset(100, 100),
        to: const Offset(180, 140),
      );

      expect(
        tester.widget<HandPointer>(find.byType(HandPointer)).at,
        const Offset(180, 140),
        reason: 'a hand frozen where the right button went down reads as a '
            'crashed application',
      );
    });
  });
}
