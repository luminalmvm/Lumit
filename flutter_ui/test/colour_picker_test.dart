
import 'dart:ui';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/colour_picker.dart';
import 'package:lumit_flutter/widgets/controls.dart';

int r(Color c) => (c.r * 255).round();
int g(Color c) => (c.g * 255).round();
int b(Color c) => (c.b * 255).round();


class _Applied {
  final List<PickedColour> previews = [];
  final List<PickedColour> commits = [];
}

Widget _harness(void Function(BuildContext) open) => Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: Overlay(
          initialEntries: [
            OverlayEntry(
              builder: (context) => Center(
                child: GestureDetector(
                  key: const Key('open'),
                  behavior: HitTestBehavior.opaque,
                  onTap: () => open(context),
                  child: const SizedBox(width: 40, height: 20),
                ),
              ),
            ),
          ],
        ),
      ),
    );

Future<_Applied> _openPicker(
  WidgetTester tester, {
  PickedColour initial = const PickedColour(0.5019607843, 0.2509803921, 0.1254901960),
  ColourScale scale = ColourScale.bytes,
  double min = 0,
  double max = 1,
  SwatchShelf? shelf,
  bool linear = false,
}) async {
  final applied = _Applied();
  await tester.pumpWidget(_harness((context) => showColourPicker(
        context: context,
        position: Offset.zero,
        initial: initial,
        scale: scale,
        min: min,
        max: max,
        shelf: shelf,
        linear: linear,
        onPreview: applied.previews.add,
        onCommit: applied.commits.add,
      )));
  await tester.tap(find.byKey(const Key('open')));
  await tester.pumpAndSettle();
  return applied;
}

void main() {
  group('hsvToRgb', () {
    void expectRgb(Hsv hsv, int er, int eg, int eb) {
      final c = hsvToRgb(hsv.$1, hsv.$2, hsv.$3);
      expect([r(c), g(c), b(c)], [er, eg, eb], reason: '$hsv');
    }

    test('the conversion table (HSV → RGB)', () {
      expectRgb((0, 0, 0), 0, 0, 0);
      expectRgb((0, 0, 1), 255, 255, 255);
      expectRgb((0, 1, 1), 255, 0, 0);
      expectRgb((120, 1, 1), 0, 255, 0);
      expectRgb((240, 1, 1), 0, 0, 255);
      expectRgb((120, 0.5, 0.8), 102, 204, 102);
    });
  });

  group('rgbToHsv', () {
    test('round-trips back through hsvToRgb', () {
      for (final sample in [
        const Color.fromARGB(0xff, 12, 200, 90),
        const Color.fromARGB(0xff, 200, 40, 160),
        const Color.fromARGB(0xff, 224, 90, 114),
      ]) {
        final hsv = rgbToHsv(sample);
        final back = hsvToRgb(hsv.$1, hsv.$2, hsv.$3);
        expect([r(back), g(back), b(back)], [r(sample), g(sample), b(sample)]);
      }
    });
  });

  group('hex parse/format', () {
    test('round-trips through parse and format', () {
      for (final s in ['000000', 'FFFFFF', 'E05A72', '1A2B3C', 'FF8800']) {
        expect(formatHex(parseHex(s)!), s);
      }
    });
  });

  group('the picker applies as it changes', () {
    testWidgets('a typed channel applies immediately and moves the picker',
        (tester) async {
      final applied = await _openPicker(tester);
      await tester.tap(find.byKey(const Key('colour-picker-R')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(EditableText).first, '255');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(applied.commits, isNotEmpty, reason: 'applied without a button');
      expect((applied.commits.last.r * 255).round(), 255);
      expect(find.text('FF4020'), findsOneWidget);
    });

    testWidgets('dragging the square previews, and settles on release',
        (tester) async {
      final applied = await _openPicker(tester);
      final square = find.byKey(const Key('colour-picker-square'));
      final gesture =
          await tester.startGesture(tester.getCenter(square), kind: PointerDeviceKind.mouse);
      await gesture.moveBy(const Offset(20, -10));
      await tester.pump();
      expect(applied.previews, isNotEmpty,
          reason: 'the picture follows the pointer');
      final duringDrag = applied.commits.length;
      await gesture.up();
      await tester.pumpAndSettle();
      expect(applied.commits.length, duringDrag + 1,
          reason: 'one settled edit for the whole drag');
    });

    testWidgets('the eyedropper takes the next click', (tester) async {
      final applied = await _openPicker(tester);
      await tester.tap(find.byKey(const Key('colour-picker-dropper')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('colour-picker-square')),
          warnIfMissed: false);
      await tester.pumpAndSettle();
      expect(applied.commits, isEmpty, reason: 'the square never saw it');
      expect(find.byKey(const Key('colour-picker-square')), findsOneWidget,
          reason: 'the picker is still open');
      await tester.tap(find.byKey(const Key('colour-picker-square')));
      await tester.pumpAndSettle();
      expect(applied.commits, isNotEmpty);
    });

    testWidgets('a linear colour reads as the screen shows it',
        (tester) async {
      final initial = PickedColour(1, linearOf(0x8f / 255), 0);
      final applied = await _openPicker(tester, initial: initial, linear: true);
      expect(find.text('FF8F00'), findsOneWidget,
          reason: 'the hex is what the screen shows');
      await tester.enterText(find.byType(EditableText).last, '808080');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(applied.commits.last.r, closeTo(linearOf(0x80 / 255), 1e-4));
      expect(applied.commits.last.r, closeTo(0.2158, 1e-3));
    });

    test('screenOf and linearOf undo each other, past white too', () {
      for (final v in [-0.5, 0.0, 0.002, 0.2, 0.5, 1.0, 2.5]) {
        expect(linearOf(screenOf(v)), closeTo(v, 1e-9));
      }
    });

    testWidgets('Cancel puts back the colour it opened with', (tester) async {
      const initial = PickedColour(0.5, 0.25, 0.125);
      final applied = await _openPicker(tester, initial: initial);
      await tester.tap(find.byKey(const Key('colour-picker-square')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('colour-picker-cancel')));
      await tester.pumpAndSettle();
      expect(applied.commits.last, initial);
      expect(find.byKey(const Key('colour-picker-square')), findsNothing,
          reason: 'and closes');
    });
  });

  group('the channel scale follows what is being edited', () {
    /// **The HDR case.** A tint whose parameter reaches 4 must be typeable to
    /// 2.5 — clamping it at white in the picker loses the value the engine
    /// would happily carry (fp16 goes to 65504).
    testWidgets('a channel can be typed above 1 when the parameter allows it',
        (tester) async {
      final applied = await _openPicker(
        tester,
        initial: const PickedColour(0.5, 0.25, 0.125),
        scale: ColourScale.unit,
        max: 4,
      );
      await tester.tap(find.byKey(const Key('colour-picker-R')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(EditableText).first, '2.5');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(applied.commits.last.r, closeTo(2.5, 1e-9),
          reason: 'the value reached the document unclamped');
      expect(find.byKey(const Key('colour-picker-clipped')), findsOneWidget);
    });
  });

  group('the project colour shelf', () {
    SwatchShelf recording(List<BridgeSwatch> held, List<List<BridgeSwatch>> writes) =>
        SwatchShelf(held, writes.add);

    BridgeSwatch swatch(double r, double g, double b, {String name = ''}) =>
        BridgeSwatch(r: r, g: g, b: b, a: 1, name: name);

    testWidgets('a kept colour applies on a click', (tester) async {
      final writes = <List<BridgeSwatch>>[];
      final applied = await _openPicker(
        tester,
        shelf: recording([swatch(1, 0, 0), swatch(0, 0, 1, name: 'Sky')], writes),
      );
      expect(find.byKey(const Key('colour-picker-shelf')), findsOneWidget);

      await tester.tap(find.byKey(const Key('colour-picker-shelf-1')));
      await tester.pumpAndSettle();
      expect(applied.commits.last, const PickedColour(0, 0, 1));
      expect(writes, isEmpty, reason: 'applying a colour does not edit the shelf');
    });

    testWidgets('the plus keeps the colour being edited', (tester) async {
      final writes = <List<BridgeSwatch>>[];
      await _openPicker(
        tester,
        initial: const PickedColour(1, 0, 0),
        shelf: recording([swatch(0, 0, 1, name: 'Sky')], writes),
      );
      await tester.tap(find.byKey(const Key('colour-picker-shelf-add')));
      await tester.pumpAndSettle();

      expect(writes.length, 1, reason: 'one write, so one undo step');
      expect(writes.single.length, 2);
      expect(writes.single.first.name, 'Sky',
          reason: 'the colour already on the shelf keeps its name');
      final kept = writes.single.last;
      expect([kept.r, kept.g, kept.b], [1, 0, 0]);
      expect(find.byKey(const Key('colour-picker-shelf-1')), findsOneWidget);
    });
  });
}
