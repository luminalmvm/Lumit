
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

Widget _harness(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: Overlay(
          initialEntries: [
            OverlayEntry(builder: (_) => Center(child: child)),
          ],
        ),
      ),
    );

void main() {
  testWidgets('right-click offers Reset / Copy / Paste; Reset uses the default',
      (tester) async {
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform, (call) async => null);
    addTearDown(() => tester.binding.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, null));
    num? changed;
    await tester.pumpWidget(_harness(DragValueField(
      value: 42,
      min: 0,
      max: 100,
      resetTo: 10,
      onChanged: (v) => changed = v,
    )));

    await tester.tap(find.text('42'), buttons: kSecondaryButton);
    await tester.pumpAndSettle();

    expect(find.text('Reset'), findsOneWidget);
    expect(find.text('Copy value'), findsOneWidget);
    expect(find.text('Paste value'), findsOneWidget);

    await tester.tap(find.text('Reset'));
    await tester.pumpAndSettle();
    expect(changed, 10);
  });

  testWidgets('Paste parses the clipboard and clamps to the field range',
      (tester) async {
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.getData') {
          return <String, Object?>{'text': '250'};
        }
        return null;
      },
    );

    num? changed;
    await tester.pumpWidget(_harness(DragValueField(
      value: 20,
      min: 0,
      max: 100,
      onChanged: (v) => changed = v,
    )));

    await tester.tap(find.text('20'), buttons: kSecondaryButton);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Paste value'));
    await tester.pumpAndSettle();

    expect(changed, 100, reason: 'pasted 250 clamps to max 100');
    tester.binding.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, null);
  });
}
