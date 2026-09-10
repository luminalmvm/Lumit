// A double-click is timed on the clock a test drives, so a slow machine
// cannot turn two quick taps into two single ones.
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';

void main() {
  testWidgets(
      'two taps inside the window are a double-click however slow the machine',
      (tester) async {
    final taps = DoubleTap();
    expect(taps.tap(), isFalse);
    // Real time passes, as it does on a busy runner, but the test's clock does not.
    await tester.runAsync(() => Future<void>.delayed(
        kDoubleTapTimeout + const Duration(milliseconds: 50)));
    expect(taps.tap(), isTrue);
  });

  testWidgets('two taps a whole window apart are two clicks', (tester) async {
    final taps = DoubleTap();
    expect(taps.tap(), isFalse);
    await tester.pump(kDoubleTapTimeout);
    expect(taps.tap(), isFalse);
  });
}
