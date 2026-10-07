// The shared chrome primitives, held to docs/15-DESIGN.md's redesign rules:
// the kicker every container label is set in, the inset well an editable
// number sits in, and the one filled button a surface is allowed.
//
// These are the pieces every panel inherits, so they are asserted here on the
// primitives themselves rather than once per panel.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

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

  /// **A choice a list cannot honour is drawn dead, not dropped**. The
  /// export's colour dropdown lists every space the project's colour config
  /// names, and some of them the config cannot actually deliver; removing those
  /// rows would leave the reader hunting for a name they know is in their own
  /// file, so the row stays, quiet, with the reason on hover.
  testWidgets('a dropdown option with a reason cannot be chosen',
      (tester) async {
    String? chosen;
    await tester.pumpWidget(host(BareDropdown<String>(
      value: 'a',
      options: const ['a', 'b'],
      label: (v) => v,
      disabledReason: (v) => v == 'b' ? 'not from here' : null,
      onChanged: (v) => chosen = v,
    )));
    await tester.pump();

    await tester.tap(find.byType(BareDropdown<String>));
    await tester.pumpAndSettle();

    expect(tester.widget<Text>(find.text('b')).style?.color, t.textDisabled,
        reason: 'listed, and visibly not on offer');
    await tester.tap(find.text('b'));
    await tester.pumpAndSettle();
    expect(chosen, isNull, reason: 'a dead row writes nothing');
    expect(find.text('b'), findsOneWidget,
        reason: 'and it does not close the menu either');

    await tester.tap(find.text('a').last);
    await tester.pumpAndSettle();
    expect(chosen, 'a');
  });

  /// **A null option can be chosen.** Several dropdowns offer "no particular
  /// one" as their first entry — Follow the machine on General, System default
  /// on Audio — and the menu answers `null` when it is *dismissed*, so picking
  /// that entry and clicking away used to be the same answer: the option was in
  /// the list, drew a tick, and could not be taken.
  testWidgets('a dropdown whose options include null can pick it',
      (tester) async {
    String? chosen = 'a';
    var picks = 0;
    await tester.pumpWidget(host(BareDropdown<String?>(
      value: 'a',
      options: const [null, 'a'],
      label: (v) => v ?? 'none',
      onChanged: (v) {
        chosen = v;
        picks++;
      },
    )));
    await tester.pump();

    await tester.tap(find.byType(BareDropdown<String?>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('none'));
    await tester.pumpAndSettle();
    expect(picks, 1, reason: 'the null row answered');
    expect(chosen, isNull);

    // And dismissing still writes nothing, which is the reason the two were
    // ever confused.
    await tester.tap(find.byType(BareDropdown<String?>));
    await tester.pumpAndSettle();
    await tester.tapAt(const Offset(5, 5));
    await tester.pumpAndSettle();
    expect(picks, 1);
  });
}
