// The splash streams the engine's real boot log when one is supplied (the F0
// promise, op from bridge v0.7 `boot_log`), and falls back to the canned chrome
// lines without a bridge.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/splash.dart';
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
  testWidgets('streams the engine boot log when given one', (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));
    var done = false;
    await tester.pumpWidget(_harness(SplashOverlay(
      lines: const ['lumit-bridge 0.7.0', 'ABI v7', 'compositor: linked'],
      onDone: () => done = true,
    )));

    // A little way in, the first engine line is up (and not the canned line).
    await tester.pump(const Duration(milliseconds: 250));
    expect(find.text('lumit-bridge 0.7.0'), findsOneWidget);
    expect(find.text(bootLines.first), findsNothing);

    await tester.pumpAndSettle();
    expect(done, isTrue, reason: 'the splash completes and calls onDone');
  });

  testWidgets('falls back to the canned lines without a boot log',
      (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));
    await tester.pumpWidget(_harness(SplashOverlay(
      lines: const [], // no bridge → empty log
      onDone: () {},
    )));

    await tester.pump(const Duration(milliseconds: 250));
    expect(find.text(bootLines.first), findsOneWidget);
    await tester.pumpAndSettle();
  });

  testWidgets('a null boot log also falls back to the canned lines',
      (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));
    await tester.pumpWidget(_harness(SplashOverlay(onDone: () {})));

    await tester.pump(const Duration(milliseconds: 250));
    expect(find.text(bootLines.first), findsOneWidget);
    await tester.pumpAndSettle();
  });
}
