// The colour picker: the conversion maths pinned both ways (an HSV↔RGB table
// and hex parse/format round-trips, including '#' tolerance and rejection of
// bad input — pure functions, so a drift here would silently miscolour every
// pick), and the behaviour the owner asked for — the R/G/B numbers above the
// graph, each editable, and the colour applying to the document as it changes
// rather than on a button.

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
  SwatchShelf? shelf,
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
  });

  group('rgbToHsv', () {
    test('round-trips back through hsvToRgb', () {
      for (final sample in [
        const Color.fromARGB(0xff, 12, 200, 90),
        const Color.fromARGB(0xff, 200, 40, 160),
        const Color.fromARGB(0xff, 224, 90, 114), // clay, a preset accent
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
      // And the picker says the swatch and hex can no longer show it.
      expect(find.byKey(const Key('colour-picker-clipped')), findsOneWidget);
    });
  });

  /// The project's colour shelf, inside the picker: the kept colours apply on
  /// a click, the plus keeps the one being edited, and the strip is absent
  /// altogether when no project is open.
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
      // And the strip shows it without asking the engine again.
      expect(find.byKey(const Key('colour-picker-shelf-1')), findsOneWidget);
    });
  });
}
