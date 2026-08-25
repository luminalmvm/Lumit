// The safe hover triangle (K-318): the geometry, and the submenu behaviour it
// exists for — crossing a sibling row on the diagonal to a flyout must not
// take the flyout away, and settling on a sibling still must.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/hover_intent.dart';

void main() {
  group('SafeTriangle', () {
    // A flyout to the right of the pointer, the everyday shape.
    final flyout = const Rect.fromLTWH(100, 0, 150, 200);
    final triangle = SafeTriangle.towards(const Offset(90, 100), flyout);

    test('contains points on the diagonal towards the flyout', () {
      expect(triangle.contains(const Offset(95, 100)), isTrue);
      expect(triangle.contains(const Offset(99, 120)), isTrue);
      expect(triangle.contains(const Offset(99, 60)), isTrue);
    });

    test('excludes points behind the apex or far off the diagonal', () {
      expect(triangle.contains(const Offset(20, 100)), isFalse);
      expect(triangle.contains(const Offset(95, 400)), isFalse);
      expect(triangle.contains(const Offset(95, -200)), isFalse);
    });

    test('slop lets a wobble off the exact edge still count', () {
      // Just past the flyout's bottom corner: outside the strict triangle
      // (whose corner sits at y=200), inside the slop-grown one.
      expect(triangle.contains(const Offset(99.5, 202)), isTrue);
      final strict = SafeTriangle(
          const Offset(90, 100), const Offset(100, 0), const Offset(100, 200));
      expect(strict.contains(const Offset(99.5, 202)), isFalse);
    });

    test('faces the near edge whichever side the apex is on', () {
      final fromRight = SafeTriangle.towards(const Offset(300, 100), flyout);
      expect(fromRight.contains(const Offset(280, 100)), isTrue);
      expect(fromRight.contains(const Offset(90, 100)), isFalse);
    });
  });

  group('SubmenuRow with the safe triangle', () {
    Widget host(Widget child) => Directionality(
          textDirection: TextDirection.ltr,
          child: ThemeScope(
            theme: LumitTheme.dark(),
            animationLevel: AnimationLevel.none,
            showTooltips: false,
            child: Overlay(
              initialEntries: [
                OverlayEntry(
                  builder: (_) => Align(
                    alignment: Alignment.topLeft,
                    child: SizedBox(
                      width: 180,
                      child: FloatSurface(
                        child: Column(
                          mainAxisSize: MainAxisSize.min,
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            SubmenuRow(
                              key: const ValueKey('sub'),
                              closeParent: () {},
                              submenu: (dismiss) => FloatSurface(
                                child: SizedBox(
                                  width: 160,
                                  height: 120,
                                  child: MenuRow(
                                    key: const ValueKey('flyout-row'),
                                    onPressed: dismiss,
                                    child: const Text('inside'),
                                  ),
                                ),
                              ),
                              child: const Text('submenu'),
                            ),
                            MenuRow(
                              key: const ValueKey('sibling'),
                              onPressed: () {},
                              child: const Text('sibling'),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        );

    Future<TestGesture> hoverAt(WidgetTester tester, Offset at) async {
      final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await gesture.addPointer(location: at);
      addTearDown(gesture.removePointer);
      await tester.pump();
      return gesture;
    }

    testWidgets('crossing the sibling towards the flyout keeps it open',
        (tester) async {
      await tester.pumpWidget(host(const SizedBox()));
      final gesture =
          await hoverAt(tester, tester.getCenter(find.byKey(const ValueKey('sub'))));
      await tester.pump();
      expect(find.byKey(const ValueKey('flyout-row')), findsOneWidget,
          reason: 'hovering the submenu row opens its flyout');
      // A frame so the flyout is measured and the guard armed.
      await tester.pump();

      // Move diagonally: over the sibling row, but on the way to the flyout.
      final sub = tester.getCenter(find.byKey(const ValueKey('sub')));
      final flyout =
          tester.getCenter(find.byKey(const ValueKey('flyout-row')));
      await gesture.moveTo(Offset.lerp(sub, flyout, 0.35)!);
      await tester.pump(const Duration(milliseconds: 50));
      expect(find.byKey(const ValueKey('flyout-row')), findsOneWidget,
          reason: 'inside the safe triangle the flyout must stay');

      // Reaching the flyout settles it.
      await gesture.moveTo(flyout);
      await tester.pump(const Duration(milliseconds: 400));
      expect(find.byKey(const ValueKey('flyout-row')), findsOneWidget,
          reason: 'the pointer arrived; nothing may take the flyout away');
    });

    testWidgets('settling on the sibling still closes the flyout',
        (tester) async {
      await tester.pumpWidget(host(const SizedBox()));
      final gesture =
          await hoverAt(tester, tester.getCenter(find.byKey(const ValueKey('sub'))));
      await tester.pump();
      expect(find.byKey(const ValueKey('flyout-row')), findsOneWidget);
      await tester.pump();

      // Sit on the sibling row (inside the triangle, but unmoving) past the
      // grace period: the sibling wins and the flyout goes.
      final sub = tester.getCenter(find.byKey(const ValueKey('sub')));
      final flyout =
          tester.getCenter(find.byKey(const ValueKey('flyout-row')));
      await gesture.moveTo(Offset.lerp(sub, flyout, 0.35)!);
      await tester.pump(menuHoverGrace + const Duration(milliseconds: 50));
      await tester.pump();
      expect(find.byKey(const ValueKey('flyout-row')), findsNothing,
          reason: 'a pointer that stops on a sibling meant the sibling');
    });

    testWidgets('moving straight down the menu closes the flyout at once',
        (tester) async {
      await tester.pumpWidget(host(const SizedBox()));
      final gesture =
          await hoverAt(tester, tester.getCenter(find.byKey(const ValueKey('sub'))));
      await tester.pump();
      expect(find.byKey(const ValueKey('flyout-row')), findsOneWidget);
      await tester.pump();

      // Straight down the left edge of the rows — nowhere near the diagonal.
      final sib = tester.getCenter(find.byKey(const ValueKey('sibling')));
      await gesture.moveTo(Offset(12, sib.dy));
      await tester.pump();
      await tester.pump();
      expect(find.byKey(const ValueKey('flyout-row')), findsNothing,
          reason: 'outside the triangle the switch is immediate');
    });

    testWidgets('the debug overlay draws without changing what the guard does',
        (tester) async {
      // The Debug panel's switch (K-318) only draws. It must decide nothing —
      // the same journey must end the same way — and it must take its overlay
      // down again, since an OverlayEntry outlives the widget that inserted it.
      debugShowSafeTriangles.value = true;
      addTearDown(() => debugShowSafeTriangles.value = false);

      await tester.pumpWidget(host(const SizedBox()));
      final gesture = await hoverAt(
          tester, tester.getCenter(find.byKey(const ValueKey('sub'))));
      await tester.pump();
      await tester.pump();
      expect(find.byKey(const ValueKey('flyout-row')), findsOneWidget);

      final sub = tester.getCenter(find.byKey(const ValueKey('sub')));
      final flyout =
          tester.getCenter(find.byKey(const ValueKey('flyout-row')));
      await gesture.moveTo(Offset.lerp(sub, flyout, 0.35)!);
      await tester.pump(const Duration(milliseconds: 50));
      expect(find.byKey(const ValueKey('flyout-row')), findsOneWidget,
          reason: 'drawing the triangle must not change the guard');

      // Straight down the rows: the flyout goes, and the drawing with it.
      final sib = tester.getCenter(find.byKey(const ValueKey('sibling')));
      await gesture.moveTo(Offset(12, sib.dy));
      await tester.pump();
      await tester.pump();
      expect(find.byKey(const ValueKey('flyout-row')), findsNothing);
      await tester.pumpWidget(const SizedBox());
      expect(tester.takeException(), isNull,
          reason: 'the overlay comes down with the surface that raised it');
    });
  });
}
