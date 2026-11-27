
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  Future<({LumitState state, LumitUiState uiState})> mount(
      WidgetTester tester) async {
    tester.view.physicalSize = const Size(1800, 1100);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    final p = freshProject();
    p.uiState.setSelectedComp(p.state.project!.newComposition(name: 'Scene'));
    await tester.pumpWidget(LumitAppNew(p.state, p.uiState, welcome: false));
    await tester.pump(const Duration(seconds: 3));
    await tester.pump();
    return p;
  }

  Future<void> press(WidgetTester tester, LogicalKeyboardKey key) async {
    await tester.sendKeyEvent(key);
    await tester.pump();
    await tester.pump();
  }

  FocusNode? focused(WidgetTester tester) =>
      tester.binding.focusManager.primaryFocus;

  testWidgets('Tab with nothing focused leaves the menu bar alone',
      (tester) async {
    final p = await mount(tester);
    expect(focused(tester), isNot(isA<ControlFocusNode>()));

    await press(tester, LogicalKeyboardKey.tab);
    await press(tester, LogicalKeyboardKey.escape);
    expect(focused(tester), isNot(isA<ControlFocusNode>()),
        reason: 'no control took the focus');

    var played = 0;
    p.uiState.togglePlayRequest.addListener(() => played++);
    await press(tester, LogicalKeyboardKey.space);
    expect(played, 1, reason: 'the shortcuts still answer');
  });

  testWidgets('Tab moves on from a focused control', (tester) async {
    await mount(tester);
    tester
        .widgetList<Focus>(find.descendant(
            of: find.byKey(const ValueKey<String>('menu-File')),
            matching: find.byType(Focus)))
        .map((focus) => focus.focusNode)
        .whereType<ControlFocusNode>()
        .first
        .requestFocus();
    await tester.pump();
    final first = focused(tester);
    expect(first, isA<ControlFocusNode>());

    await press(tester, LogicalKeyboardKey.tab);
    expect(focused(tester), isA<ControlFocusNode>());
    expect(focused(tester), isNot(first));
  });
}
