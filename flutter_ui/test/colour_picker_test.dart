// The colour picker: the conversion maths pinned both ways (an HSV↔RGB table
// and hex parse/format round-trips, including '#' tolerance and rejection of
// bad input — pure functions, so a drift here would silently miscolour every
// pick), and the behaviour the owner asked for — the R/G/B numbers above the
// graph, each editable, and the colour applying to the document as it changes
// rather than on a button.

import 'dart:ui';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/colour_picker.dart';
import 'package:lumit_flutter/widgets/controls.dart';

int r(Color c) => (c.r * 255).round();
int g(Color c) => (c.g * 255).round();
int b(Color c) => (c.b * 255).round();


/// A picker opened over an overlay, with what it applied recorded.
///
/// The picker no longer *returns* a colour: it applies as it goes, so a test
/// of it is a test of what it applied and when.
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
}) async {
  final applied = _Applied();
  await tester.pumpWidget(_harness((context) => showColourPicker(
        context: context,
        position: Offset.zero,
        initial: initial,
        scale: scale,
        min: min,
        max: max,
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
      expectRgb((0, 0, 0), 0, 0, 0); // black
      expectRgb((0, 0, 1), 255, 255, 255); // white
      expectRgb((0, 1, 1), 255, 0, 0); // pure red
      expectRgb((120, 1, 1), 0, 255, 0); // pure green
      expectRgb((240, 1, 1), 0, 0, 255); // pure blue
      expectRgb((120, 0.5, 0.8), 102, 204, 102); // mid sat / mid value
    });

    test('the alpha channel is always opaque', () {
      final c = hsvToRgb(200, 0.4, 0.6);
      expect((c.a * 255).round(), 0xff);
    });
  });

  group('rgbToHsv', () {
    void expectHsv(int cr, int cg, int cb, double eh, double es, double ev) {
      final hsv = rgbToHsv(Color.fromARGB(0xff, cr, cg, cb));
      expect(hsv.$1, closeTo(eh, 1e-6), reason: 'hue');
      expect(hsv.$2, closeTo(es, 1e-6), reason: 'saturation');
      expect(hsv.$3, closeTo(ev, 1e-6), reason: 'value');
    }

    test('the conversion table (RGB → HSV)', () {
      expectHsv(0, 0, 0, 0, 0, 0); // black
      expectHsv(255, 255, 255, 0, 0, 1); // white
      expectHsv(255, 0, 0, 0, 1, 1); // pure red
      expectHsv(0, 255, 0, 120, 1, 1); // pure green
      expectHsv(0, 0, 255, 240, 1, 1); // pure blue
      expectHsv(102, 204, 102, 120, 0.5, 0.8); // mid sat / mid value
    });

    test('round-trips back through hsvToRgb', () {
      for (final sample in [
        const Color.fromARGB(0xff, 12, 200, 90),
        const Color.fromARGB(0xff, 200, 40, 160),
        const Color.fromARGB(0xff, 224, 90, 114), // the default clay accent
      ]) {
        final hsv = rgbToHsv(sample);
        final back = hsvToRgb(hsv.$1, hsv.$2, hsv.$3);
        expect([r(back), g(back), b(back)], [r(sample), g(sample), b(sample)]);
      }
    });
  });

  group('hex parse/format', () {
    test('parses six digits, tolerating a leading #', () {
      final a = parseHex('e05a72');
      final b0 = parseHex('#E05A72');
      expect(a, isNotNull);
      expect(b0, isNotNull);
      expect([r(a!), g(a), b(a)], [0xe0, 0x5a, 0x72]);
      expect([r(b0!), g(b0), b(b0)], [0xe0, 0x5a, 0x72]);
    });

    test('trims surrounding whitespace', () {
      final c = parseHex('  ff8800  ');
      expect(c, isNotNull);
      expect([r(c!), g(c), b(c)], [0xff, 0x88, 0x00]);
    });

    test('rejects malformed input', () {
      expect(parseHex(''), isNull);
      expect(parseHex('12345'), isNull); // too short
      expect(parseHex('1234567'), isNull); // too long
      expect(parseHex('gg0000'), isNull); // non-hex
      expect(parseHex('#12g456'), isNull);
      expect(parseHex('not a colour'), isNull);
    });

    test('formats as upper-case RRGGBB with no #', () {
      expect(formatHex(const Color.fromARGB(0xff, 0xe0, 0x5a, 0x72)), 'E05A72');
      expect(formatHex(const Color.fromARGB(0xff, 0, 0, 0)), '000000');
      expect(formatHex(const Color.fromARGB(0xff, 255, 136, 0)), 'FF8800');
    });

    test('round-trips through parse and format', () {
      for (final s in ['000000', 'FFFFFF', 'E05A72', '1A2B3C', 'FF8800']) {
        expect(formatHex(parseHex(s)!), s);
      }
    });
  });

  group('the picker applies as it changes', () {
    testWidgets('shows R, G and B above the graph, each one editable',
        (tester) async {
      await _openPicker(tester);
      // The three numbers of the colour it opened on, as fields.
      expect(find.text('128'), findsOneWidget);
      expect(find.text('64'), findsOneWidget);
      expect(find.text('32'), findsOneWidget);
      expect(find.byKey(const Key('colour-picker-R')), findsOneWidget);
      expect(find.byKey(const Key('colour-picker-G')), findsOneWidget);
      expect(find.byKey(const Key('colour-picker-B')), findsOneWidget);
    });

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
      // The hex field followed the number, so the two cannot disagree.
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

    testWidgets('clicking away keeps what is applied, and closes',
        (tester) async {
      final applied = await _openPicker(tester);
      await tester.tap(find.byKey(const Key('colour-picker-strip')));
      await tester.pumpAndSettle();
      final chosen = applied.commits.last;

      // A press outside the picker: the popup's own barrier.
      await tester.tapAt(const Offset(700, 500));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('colour-picker-strip')), findsNothing);
      expect(applied.commits.last, chosen,
          reason: 'nothing was rolled back on the way out');
    });
  });

  group('the channel scale follows what is being edited', () {
    /// A display colour is eight bits: 0–255, and a hex is the same value said
    /// another way.
    testWidgets('a display colour reads 0–255', (tester) async {
      await _openPicker(tester, scale: ColourScale.bytes);
      expect(find.text('128'), findsOneWidget);
      expect(find.text('64'), findsOneWidget);
      expect(find.text('32'), findsOneWidget);
    });

    /// A scene-linear colour in a float working depth is not: 0–1 is black to
    /// white, shown as decimals.
    testWidgets('a scene-linear colour reads 0–1', (tester) async {
      await _openPicker(
        tester,
        initial: const PickedColour(0.5, 0.25, 0.125),
        scale: ColourScale.unit,
        max: 4,
      );
      expect(find.text('0.500'), findsOneWidget);
      expect(find.text('0.250'), findsOneWidget);
      expect(find.text('0.125'), findsOneWidget);
    });

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
      // And the picker says the swatch and hex can no longer show it.
      expect(find.byKey(const Key('colour-picker-clipped')), findsOneWidget);
    });

    testWidgets("a channel is still held to the parameter's own range",
        (tester) async {
      final applied = await _openPicker(
        tester,
        initial: const PickedColour(0.5, 0.25, 0.125),
        scale: ColourScale.unit,
        max: 4,
      );
      await tester.tap(find.byKey(const Key('colour-picker-G')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(EditableText).first, '99');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(applied.commits.last.g, 4, reason: 'clamped at the declared max');
    });

    /// An over-range colour must survive being dragged about on the square:
    /// the graph is 0–1, so the picker carries the overshoot as a gain rather
    /// than throwing it away the moment the pointer touches the square.
    testWidgets('dragging the square keeps an over-range colour over-range',
        (tester) async {
      final applied = await _openPicker(
        tester,
        initial: const PickedColour(3, 1.5, 0.75),
        scale: ColourScale.unit,
        max: 4,
      );
      await tester.tap(find.byKey(const Key('colour-picker-square')));
      await tester.pumpAndSettle();
      expect(applied.commits.last.r, greaterThan(1),
          reason: 'the brightest channel is still above white');
    });

    /// The clipped note belongs to the float scale only: a 0–255 colour cannot
    /// leave the gamut, so the line would be noise.
    testWidgets('a display colour never shows the clipped note', (tester) async {
      await _openPicker(tester, scale: ColourScale.bytes);
      expect(find.byKey(const Key('colour-picker-clipped')), findsNothing);
    });
  });
}
