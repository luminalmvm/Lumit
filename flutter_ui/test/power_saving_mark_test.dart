// The status line's Power saving word: shown while the system says it is on
// battery or saving power, gone when it is not, and nothing when the question
// cannot be answered.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/power_saving_mark.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

Widget _harness(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: child,
      ),
    );

void main() {
  testWidgets('the word follows the power state', (tester) async {
    var saving = true;
    await tester.pumpWidget(_harness(PowerSavingMark(
      wrap: (mark) => mark,
      ask: () => saving,
    )));
    expect(find.byKey(const ValueKey('status-power-saving')), findsOneWidget);

    saving = false;
    await tester.pump(const Duration(seconds: 6));
    expect(find.byKey(const ValueKey('status-power-saving')), findsNothing);
  });

  testWidgets('a question that fails shows nothing', (tester) async {
    await tester.pumpWidget(_harness(PowerSavingMark(
      wrap: (mark) => mark,
      ask: () => throw StateError('no bridge'),
    )));
    expect(find.byKey(const ValueKey('status-power-saving')), findsNothing);
  });
}
