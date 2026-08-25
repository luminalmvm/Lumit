// The shared chrome primitives, held to docs/15-DESIGN.md's redesign rules
// (K-438/K-439): the kicker every container label is set in, the inset well an
// editable number sits in, and the one filled button a surface is allowed.
//
// These are the pieces every panel inherits, so they are asserted here on the
// primitives themselves rather than once per panel.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/marquee.dart';

void main() {
  final t = LumitTheme.dark();

  Widget host(Widget child) => Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: t,
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(
            initialEntries: [
              OverlayEntry(builder: (_) => Center(child: child))
            ],
          ),
        ),
      );

  group('The kicker (§7.1)', () {
    test('is Geist Mono, 9-11 px, wide-tracked, caps-toned and muted', () {
      final k = t.kicker;
      expect(k.fontFamily, LumitTheme.monoFontFamily);
      expect(k.fontSize, inInclusiveRange(9, 11));
      // Tracking is stated in ems by the spec and in logical pixels by
      // Flutter, so it is the ratio that has to land in the band — with a
      // float epsilon on each end, because dividing the one by the other puts
      // an exact 0.12em (1.08px at 9) a bit-error outside it.
      final em = k.letterSpacing! / k.fontSize!;
      expect(em, inInclusiveRange(0.08 - 1e-9, 0.12 + 1e-9));
      expect(k.color, t.textMuted);
    });

    /// State reads from colour alone — a heavier or larger active label would
    /// shuffle every tab beside it each time the front one changed.
    test('the active variant differs only in colour', () {
      expect(t.kickerOn.color, t.textPrimary);
      expect(t.kickerOn.fontSize, t.kicker.fontSize);
      expect(t.kickerOn.fontWeight, t.kicker.fontWeight);
      expect(t.kickerOn.letterSpacing, t.kicker.letterSpacing);
      expect(t.kickerOn.fontFamily, t.kicker.fontFamily);
    });
  });

  group('The value well (§2.1, §3.1)', () {
    BoxDecoration wellOf(WidgetTester tester) => tester
        .widget<Container>(find.descendant(
          of: find.byType(DragValueField),
          matching: find.byType(Container),
        ))
        .decoration! as BoxDecoration;

    TextStyle numberOf(WidgetTester tester) => tester
        .widget<Text>(find.descendant(
          of: find.byType(DragValueField),
          matching: find.byType(Text),
        ))
        .style!;

    Widget field({bool keyed = false}) => DragValueField(
          value: 42,
          min: 0,
          max: 100,
          keyed: keyed,
          onChanged: (_) {},
        );

    /// At rest the well is a recess, not a raised box: darker than the panel
    /// it sits in, inside a plain hairline, with the number in mono.
    testWidgets('rests as a surface0 inset inside a hairline', (tester) async {
      await tester.pumpWidget(host(field()));
      await tester.pump();

      final d = wellOf(tester);
      expect(d.color, t.surface0);
      expect((d.border! as Border).top.color, t.hairline);

      final style = numberOf(tester);
      expect(style.fontFamily, LumitTheme.monoFontFamily);
      expect(style.fontSize, wellTextSize,
          reason: '§7.1: property values are 11px mono, the mockups\' own');
      expect(style.color, t.textPrimary);
    });

    /// A keyed property rests `animated` — the only other stateful colour in
    /// chrome (§3.1), and the well is where a keyframed value says so.
    testWidgets('rests animated when the property is keyed', (tester) async {
      await tester.pumpWidget(host(field(keyed: true)));
      await tester.pump();

      expect(numberOf(tester).color, t.animated);
    });

    /// While the value is actually in hand it turns accent — and goes back the
    /// moment the pointer lifts, because feedback leaves no trace (§12A.5).
    testWidgets('turns accent while being dragged, and back on release',
        (tester) async {
      await tester.pumpWidget(host(field()));
      await tester.pump();

      final gesture = await tester.startGesture(
        tester.getCenter(find.byType(DragValueField)),
        kind: PointerDeviceKind.mouse,
      );
      await gesture.moveBy(const Offset(20, 0));
      await tester.pump();

      expect(numberOf(tester).color, t.accent);
      expect((wellOf(tester).border! as Border).top.color, t.accent);

      // A second move, so the drag actually ticks: the first is spent starting
      // the gesture, and a released drag that never ticked is treated as a
      // click and opens the editor instead (K-319).
      await gesture.moveBy(const Offset(20, 0));
      await tester.pump();

      await gesture.up();
      await tester.pump();
      expect(numberOf(tester).color, t.textPrimary,
          reason: 'the scrub left no trace behind');
    });
  });

  /// A well you *type* into answers focus the same way a well you scrub does:
  /// the `animated` edge, the one focus that means "you are about to change a
  /// value" (§3.1, §6.5). [HouseTextField] had simply never answered at all,
  /// so the drawings' focused well had no counterpart on screen.
  testWidgets('a focused text well takes the animated edge', (tester) async {
    final controller = TextEditingController(text: 'Comp 2');
    addTearDown(controller.dispose);
    final focus = FocusNode();
    addTearDown(focus.dispose);
    await tester.pumpWidget(
      host(HouseTextField(controller: controller, focusNode: focus)),
    );
    await tester.pump();

    Color edge() => ((tester
                .widget<Container>(find.descendant(
                  of: find.byType(HouseTextField),
                  matching: find.byType(Container),
                ))
                .decoration! as BoxDecoration)
            .border! as Border)
        .top
        .color;

    expect(edge(), t.hairline, reason: 'at rest it is a plain recess');
    await tester.tap(find.byType(HouseTextField));
    await tester.pumpAndSettle();
    expect(edge(), t.animated,
        reason: 'never the accent — §3.1 keeps that list closed');
  });

  /// THE single filled button per surface — the whole of the accent's button
  /// job (§3.1), with the label at the far end of the ramp in mono capitals.
  testWidgets('the primary button is the accent fill', (tester) async {
    await tester.pumpWidget(host(
      HouseButton(primary: true, onPressed: () {}, child: const Text('Export')),
    ));
    await tester.pump();

    final d = tester
        .widget<AnimatedContainer>(find.descendant(
          of: find.byType(HouseButton),
          matching: find.byType(AnimatedContainer),
        ))
        .decoration! as BoxDecoration;
    expect(d.color, t.accent);

    final style = tester
        .widget<DefaultTextStyle>(find
            .descendant(
              of: find.byType(HouseButton),
              matching: find.byType(DefaultTextStyle),
            )
            .first)
        .style;
    expect(style.color, t.surface0);
    expect(style.fontFamily, LumitTheme.monoFontFamily);
    expect(find.text('EXPORT'), findsOneWidget,
        reason: 'capitals are the style, not the arb string');
  });

  /// A secondary action is an outline over the panel's own surface, so a
  /// resting panel never gains a fourth grey (§2.1's three greys at rest).
  testWidgets('the secondary button is an outline, not a fill', (tester) async {
    await tester.pumpWidget(host(
      HouseButton(onPressed: () {}, child: const Text('Cancel')),
    ));
    await tester.pump();

    final d = tester
        .widget<AnimatedContainer>(find.descendant(
          of: find.byType(HouseButton),
          matching: find.byType(AnimatedContainer),
        ))
        .decoration! as BoxDecoration;
    expect(d.color, isNull);
    expect((d.border! as Border).top.color, t.hairlineStrong);
  });

  /// A dialog footer states its buttons' height (45 = 10 + 24 + 10, §12A.4)
  /// rather than letting the label decide it, so every footer button is handed
  /// a box taller than its own words. The label has to sit in the middle of
  /// that box — it used to be painted at the top of it, which is what "the
  /// text sits off-centre" was.
  /// A dialog footer states its buttons' height — 45 = 10 + 24 + 10 (§12A.4) —
  /// rather than letting the label decide it, so every footer button is handed
  /// a box taller than its own words.
  ///
  /// **The label's *height* is what has to be measured, not only its centre.**
  /// A paragraph handed a tight box stretches to fill it and then paints its
  /// single line at the top, so a stretched label reports a perfectly centred
  /// rectangle while the words sit visibly high — which is exactly the bug this
  /// pins. A label whose box is still its own line box, centred in the button,
  /// is a label whose glyphs are centred.
  ///
  /// Both faces are measured, because the primary one changes typeface as well
  /// as colour — mono capitals at 9px, whose line box is a different height
  /// again — and must not be assumed to follow the secondary.
  testWidgets('a button given a height centres its label in it',
      (tester) async {
    // One tree, not one pump per case: this file's host builds its child into
    // an `Overlay`'s `initialEntries`, which are read once and never rebuilt,
    // so a second `pumpWidget` would silently measure the first tree again.
    Widget cased(String key, Widget button, {double? height}) => SizedBox(
          key: ValueKey<String>(key),
          height: height,
          child: button,
        );
    await tester.pumpWidget(host(Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        cased(
          'free-secondary',
          HouseButton(onPressed: () {}, child: const Text('Add to queue')),
        ),
        cased(
          'boxed-secondary',
          HouseButton(onPressed: () {}, child: const Text('Add to queue')),
          height: 24,
        ),
        cased(
          'free-primary',
          HouseButton(
              primary: true, onPressed: () {}, child: const Text('Export')),
        ),
        cased(
          'boxed-primary',
          HouseButton(
              primary: true, onPressed: () {}, child: const Text('Export')),
          height: 24,
        ),
      ],
    )));
    await tester.pump();

    Rect labelOf(String key) => tester.getRect(find.descendant(
          of: find.byKey(ValueKey<String>(key)),
          matching: find.byType(Text),
        ));
    Rect boxOf(String key) => tester.getRect(find.byKey(ValueKey<String>(key)));

    for (final face in ['secondary', 'primary']) {
      final free = labelOf('free-$face');
      final label = labelOf('boxed-$face');
      final box = boxOf('boxed-$face');
      expect(label.height, closeTo(free.height, 0.01),
          reason: '$face: the label was stretched to the button rather than '
              'centred in it, which paints the words at the top of the box');
      expect(label.center.dy, closeTo(box.center.dy, 0.5),
          reason: '$face: the label is not in the middle of the button');
      // A button left to its own devices must not have grown or moved: the
      // centring may only take effect where a height was imposed. 3 of padding
      // and 1 of border above and below the line box, and nothing else.
      expect(boxOf('free-$face').height, closeTo(free.height + 8, 0.01),
          reason: '$face: an unconstrained button is still its label\'s own '
              'height');
    }
  });

  /// **What is selected is one colour, and it is not the accent** (K-439,
  /// docs/impl/timeline-interaction.md P4). The marquee is shared by the
  /// Timeline's lanes and the graph, and it drew its box in `accent` — where
  /// the closed list is the playhead, the one filled button and the active
  /// tab's tick, and a box in it read as a second playhead being dragged out.
  testWidgets('the marquee box is text_primary over a faint wash',
      (tester) async {
    await tester.pumpWidget(host(SizedBox(
      width: 200,
      height: 200,
      child: MarqueeSelect(onSelect: (_, __) {}, onClear: () {}),
    )));
    await tester.pump();

    final from =
        tester.getTopLeft(find.byType(MarqueeSelect)) + const Offset(20, 20);
    final gesture = await tester.startGesture(from);
    await tester.pump(const Duration(milliseconds: 60));
    await gesture.moveBy(const Offset(40, 40));
    await tester.pump();
    await gesture.moveBy(const Offset(40, 40));
    await tester.pump();

    final d = tester
        .widget<Container>(find.descendant(
          of: find.byType(MarqueeSelect),
          matching: find.byType(Container),
        ))
        .decoration! as BoxDecoration;
    expect((d.border! as Border).top.color, t.textPrimary);
    expect(d.color, t.textPrimary.withValues(alpha: marqueeWashAlpha));
    await gesture.up();
    await tester.pumpAndSettle();
  });
}
