
import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/hover_intent.dart';

void main() {
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
      await tester.pump();

      final sub = tester.getCenter(find.byKey(const ValueKey('sub')));
      final flyout =
          tester.getCenter(find.byKey(const ValueKey('flyout-row')));
      await gesture.moveTo(Offset.lerp(sub, flyout, 0.35)!);
      await tester.pump(const Duration(milliseconds: 50));
      expect(find.byKey(const ValueKey('flyout-row')), findsOneWidget,
          reason: 'inside the safe triangle the flyout must stay');

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

      final sub = tester.getCenter(find.byKey(const ValueKey('sub')));
      final flyout =
          tester.getCenter(find.byKey(const ValueKey('flyout-row')));
      await gesture.moveTo(Offset.lerp(sub, flyout, 0.35)!);
      await tester.pump(menuHoverGrace + const Duration(milliseconds: 50));
      await tester.pump();
      expect(find.byKey(const ValueKey('flyout-row')), findsNothing,
          reason: 'a pointer that stops on a sibling meant the sibling');
    });
  });
}
