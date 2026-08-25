// The DragValueField right-click menu (egui's built-in drag-value menu):
// Reset (only when a default is known) / Copy / Paste over the system clipboard,
// with the field's own clamp on paste.

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
    expect(find.text('Copy'), findsOneWidget);
    expect(find.text('Paste'), findsOneWidget);

    await tester.tap(find.text('Reset'));
    await tester.pumpAndSettle();
    expect(changed, 10);
  });

  testWidgets('without a default, Reset is not offered', (tester) async {
    await tester.pumpWidget(_harness(DragValueField(
      value: 5,
      min: 0,
      max: 100,
      onChanged: (_) {},
    )));

    await tester.tap(find.text('5'), buttons: kSecondaryButton);
    await tester.pumpAndSettle();
    expect(find.text('Reset'), findsNothing);
    expect(find.text('Copy'), findsOneWidget);
    expect(find.text('Paste'), findsOneWidget);
  });

  testWidgets('Copy puts the plain value on the clipboard', (tester) async {
    final clipboard = <String, Object?>{};
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.setData') {
          clipboard['text'] = (call.arguments as Map)['text'];
        }
        return null;
      },
    );

    await tester.pumpWidget(_harness(DragValueField(
      value: 37,
      min: 0,
      max: 100,
      onChanged: (_) {},
    )));

    await tester.tap(find.text('37'), buttons: kSecondaryButton);
    await tester.pumpAndSettle();
    await tester.tap(find.text('Copy'));
    await tester.pumpAndSettle();

    expect(clipboard['text'], '37');
    tester.binding.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, null);
  });

  testWidgets('Paste parses the clipboard and clamps to the field range',
      (tester) async {
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.getData') {
          return <String, Object?>{'text': '250'}; // above max, will clamp
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
    await tester.tap(find.text('Paste'));
    await tester.pumpAndSettle();

    expect(changed, 100, reason: 'pasted 250 clamps to max 100');
    tester.binding.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, null);
  });
}
