// Hover behaviour of the shared controls: the tooltip's lifetime, and the fact
// that hovering must not move anything.
//
// Both of these are bugs the project owner hit in the running app rather than
// anything a panel test would have caught, so they are asserted here on the
// controls themselves.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  /// A real mouse the framework's tracker follows. `TestPointer.hover` sent
  /// straight to the binding does not update the mouse tracker, so `MouseRegion`
  /// never fires and a test using it would pass whatever the widget does.
  Future<TestGesture> mouse(WidgetTester tester) async {
    final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await gesture.addPointer(location: Offset.zero);
    addTearDown(gesture.removePointer);
    return gesture;
  }

  Widget host(Widget child, {bool tooltips = true, LumitTheme? theme}) =>
      Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: theme ?? LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: tooltips,
          child: Overlay(
            initialEntries: [
              OverlayEntry(builder: (_) => Center(child: child))
            ],
          ),
        ),
      );

  group('Hover does not move the layout', () {
    /// A `BoxDecoration`'s border insets its child, so a border that only
    /// exists on hover makes the control 2 px bigger each way the moment the
    /// pointer touches it — and everything beside it shifts.
    testWidgets('a HouseButton is the same size hovered and not',
        (tester) async {
      await tester.pumpWidget(host(
        HouseButton(onPressed: () {}, child: const Text('Press')),
      ));
      await tester.pump();

      final button = find.byType(HouseButton);
      final before = tester.getSize(button);

      final gesture = await mouse(tester);
      await gesture.moveTo(tester.getCenter(button));
      await tester.pumpAndSettle();

      expect(tester.getSize(button), before,
          reason: 'hovering must not resize the control');
    });

    testWidgets('neighbouring controls do not shift when one is hovered',
        (tester) async {
      await tester.pumpWidget(host(
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            HouseButton(onPressed: () {}, child: const Text('One')),
            HouseButton(onPressed: () {}, child: const Text('Two')),
          ],
        ),
      ));
      await tester.pump();

      final second = find.text('Two');
      final before = tester.getTopLeft(second);

      final gesture = await mouse(tester);
      await gesture.moveTo(tester.getCenter(find.text('One')));
      await tester.pumpAndSettle();

      expect(tester.getTopLeft(second), before,
          reason: 'its neighbour stayed put');
    });
  });

  group('The active control', () {
    BoxDecoration decorationOf(WidgetTester tester) =>
        tester.widget<AnimatedContainer>(
          find.descendant(
            of: find.byType(HouseButton),
            matching: find.byType(AnimatedContainer),
          ),
        ).decoration! as BoxDecoration;

    Color labelOf(WidgetTester tester) =>
        tester.widget<DefaultTextStyle>(
          find.descendant(
            of: find.byType(HouseButton),
            matching: find.byType(DefaultTextStyle),
          ).first,
        ).style.color!;

    /// Round's loudest cue: which one is in force reads from the fill, and the
    /// label flips to the far end of the ramp so it survives the accent.
    testWidgets('under Round it is the filled accent pill', (tester) async {
      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.round);
      await tester.pumpWidget(host(
        HouseButton(active: true, onPressed: () {}, child: const Text('Mask')),
        theme: t,
      ));
      await tester.pump();

      expect(decorationOf(tester).color, t.accent);
      expect(labelOf(tester), t.surface0);
    });

    /// Sharp does not take the fill: the armed tint, not a filled pill.
    testWidgets('under Sharp it stays the tint', (tester) async {
      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);
      await tester.pumpWidget(host(
        HouseButton(active: true, onPressed: () {}, child: const Text('Mask')),
        theme: t,
      ));
      await tester.pump();

      final d = decorationOf(tester);
      expect(d.color, isNot(t.accent), reason: 'a tint, not the accent itself');
      expect(d.color!.a, lessThan(0.5));
      expect(labelOf(tester), t.textPrimary, reason: 'the label does not flip');
    });

    /// The state must not blink off under the pointer — hovering the active
    /// one lifts it rather than replacing it with the hover fill.
    testWidgets('hovering an active control keeps it accent', (tester) async {
      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.round);
      await tester.pumpWidget(host(
        HouseButton(active: true, onPressed: () {}, child: const Text('Mask')),
        theme: t,
      ));
      await tester.pump();

      final gesture = await mouse(tester);
      await gesture.moveTo(tester.getCenter(find.byType(HouseButton)));
      await tester.pumpAndSettle();

      // Either accent — the lift is the point, the fill staying the accent
      // family is the invariant. (Whether the framework reports the hover at
      // all depends on its highlight mode, which is not what this asserts.)
      expect(decorationOf(tester).color, anyOf(t.accent, t.accentHover));
      expect(decorationOf(tester).color, isNot(t.surface4),
          reason: 'the hover fill must not take the active state away');
    });
  });

  group('Tooltip lifetime', () {
    testWidgets('a tooltip appears after the delay and goes on leaving',
        (tester) async {
      await tester.pumpWidget(host(
        const LumitTooltip(
          message: 'Explain this',
          child: SizedBox(width: 60, height: 20),
        ),
      ));
      await tester.pump();

      final gesture = await mouse(tester);
      await gesture.moveTo(tester.getCenter(find.byType(LumitTooltip)));
      await tester.pump(const Duration(milliseconds: 600));
      expect(find.text('Explain this'), findsOneWidget);

      await gesture.moveTo(const Offset(5, 5));
      await tester.pump();
      expect(find.text('Explain this'), findsNothing);
    });

    /// The stuck-tooltip bug. Leaving *during* the delay used to let the tip
    /// appear anyway, after the pointer had gone — so nothing was left to
    /// dismiss it and it stayed on screen indefinitely.
    testWidgets('leaving before the delay elapses shows nothing',
        (tester) async {
      await tester.pumpWidget(host(
        const LumitTooltip(
          message: 'Explain this',
          child: SizedBox(width: 60, height: 20),
        ),
      ));
      await tester.pump();

      final gesture = await mouse(tester);
      await gesture.moveTo(tester.getCenter(find.byType(LumitTooltip)));
      // Away again well inside the 500 ms delay.
      await tester.pump(const Duration(milliseconds: 100));
      await gesture.moveTo(const Offset(5, 5));

      // Past when it would have appeared, and then some.
      await tester.pump(const Duration(milliseconds: 900));
      expect(find.text('Explain this'), findsNothing,
          reason: 'a tip nobody is hovering must never appear');
    });

    testWidgets('the tooltip is off entirely when the setting is off',
        (tester) async {
      await tester.pumpWidget(host(
        const LumitTooltip(
          message: 'Explain this',
          child: SizedBox(width: 60, height: 20),
        ),
        tooltips: false,
      ));
      await tester.pump();

      final gesture = await mouse(tester);
      await gesture.moveTo(tester.getCenter(find.byType(LumitTooltip)));
      await tester.pump(const Duration(milliseconds: 900));
      expect(find.text('Explain this'), findsNothing);
    });
  });
}
