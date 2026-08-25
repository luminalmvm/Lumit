// The keyboard model of confirmation windows (K-319): the default button is
// focused on open so Enter presses it, Tab walks the window in reading order,
// and the house controls answer the keyboard at all.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  Widget host(Widget child) => Directionality(
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

  testWidgets('an autofocused button is pressed by Enter', (tester) async {
    var pressed = 0;
    await tester.pumpWidget(host(HouseButton(
      autofocus: true,
      primary: true,
      onPressed: () => pressed++,
      child: const Text('OK'),
    )));
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    expect(pressed, 1, reason: 'Enter presses the focused default button');
    await tester.sendKeyEvent(LogicalKeyboardKey.space);
    expect(pressed, 2, reason: 'Space presses it too');
  });

  testWidgets('a focused checkbox toggles on Space', (tester) async {
    var value = false;
    late StateSetter setOuter;
    await tester.pumpWidget(host(StatefulBuilder(
      builder: (context, setState) {
        setOuter = setState;
        return HouseCheckbox(
          value: value,
          onChanged: (v) => setOuter(() => value = v),
        );
      },
    )));
    await tester.pump();
    tester
        .widget<FocusableActionDetector>(find.descendant(
            of: find.byType(HouseCheckbox),
            matching: find.byType(FocusableActionDetector)))
        .focusNode!
        .requestFocus();
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.space);
    await tester.pump();
    expect(value, isTrue);
  });

  testWidgets('Escape closes a modal, the same as clicking the scrim',
      (tester) async {
    // Flutter's own DismissIntent, which `WidgetsApp` binds Escape to — so the
    // host here is a MaterialApp, as the application is. Before K-319 nothing
    // in the window claimed that intent and Escape did nothing in every
    // dialogue, despite a comment claiming it worked "via the route".
    late BuildContext ctx;
    await tester.pumpWidget(MaterialApp(
      home: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: Overlay(initialEntries: [
          OverlayEntry(builder: (c) {
            ctx = c;
            return const SizedBox.expand();
          }),
        ]),
      ),
    ));

    var answer = 'not closed';
    showLumitModal<String>(
      context: ctx,
      builder: (close) => FloatSurface(
        child: SizedBox(
          width: 200,
          height: 100,
          child: HouseButton(
            autofocus: true,
            primary: true,
            onPressed: () => close('confirmed'),
            child: const Text('OK'),
          ),
        ),
      ),
    ).then((v) => answer = v ?? 'dismissed');
    await tester.pump();
    await tester.pump();
    expect(find.text('OK'), findsOneWidget);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(answer, 'dismissed',
        reason: 'Escape dismisses with null, as the scrim does');
    expect(find.text('OK'), findsNothing);
  });

  testWidgets('a modal walks its controls in reading order', (tester) async {
    final log = <String>[];
    Widget button(String name, {bool autofocus = false}) => HouseButton(
          key: ValueKey(name),
          autofocus: autofocus,
          onPressed: () => log.add(name),
          child: Text(name),
        );
    late BuildContext ctx;
    await tester.pumpWidget(host(Builder(builder: (context) {
      ctx = context;
      return const SizedBox();
    })));
    showLumitModal<void>(
      context: ctx,
      builder: (close) => FloatSurface(
        child: SizedBox(
          width: 260,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              // Deliberately composed as two Rows nested in a Column, and the
              // default button first: widget-tree order and reading order
              // agree here, so the walk below fails if the policy is neither.
              Row(children: [button('a', autofocus: true), button('b')]),
              Row(children: [button('c'), button('d')]),
            ],
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();

    // Press the focused button, step to the next in traversal order, repeat.
    // (The app wires Tab to this same step through MaterialApp's default
    // shortcuts; the test drives the traversal directly.)
    for (var i = 0; i < 4; i++) {
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
      FocusManager.instance.primaryFocus!.nextFocus();
      await tester.pump();
    }
    expect(log, ['a', 'b', 'c', 'd'],
        reason: 'left to right, then top to bottom — reading order');
  });

  testWidgets('a value box opens its editor with the text selected',
      (tester) async {
    num value = 42;
    await tester.pumpWidget(host(DragValueField(
      value: value,
      min: 0,
      max: 100,
      onChanged: (v) => value = v,
    )));

    // A clean click opens the editor with the whole value selected.
    await tester.tap(find.byType(DragValueField));
    await tester.pump();
    final editable = tester.widget<EditableText>(find.byType(EditableText));
    expect(editable.controller.text, '42');
    expect(editable.controller.selection.baseOffset, 0);
    expect(editable.controller.selection.extentOffset, 2,
        reason: 'the value opens selected, so typing replaces it');

    // Type a new value and commit.
    await tester.enterText(find.byType(EditableText), '55');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pump();
    expect(value, 55);

    // Escape is the way back out (K-323): the editor shuts and the typed
    // number is thrown away. Every other exit — Enter, clicking away, losing
    // focus — commits, so without this a half-typed value could not be undone
    // without retyping the old one.
    await tester.tap(find.byType(DragValueField));
    await tester.pump();
    await tester.enterText(find.byType(EditableText), '99');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byType(EditableText), findsNothing,
        reason: 'Escape closes the editor');
    expect(value, 55, reason: 'and the field keeps the value it had');
  });

  testWidgets('the slider brackets a drag with start and end', (tester) async {
    final events = <String>[];
    await tester.pumpWidget(host(HouseSlider(
      value: 0.5,
      min: 0,
      max: 1,
      width: 100,
      onChanged: (_) => events.add('commit'),
      onChangeLive: (_) => events.add('live'),
      onChangeStart: () => events.add('start'),
      onChangeEnd: () => events.add('end'),
    )));
    final centre = tester.getCenter(find.byType(HouseSlider).first);
    final gesture = await tester.startGesture(centre);
    await gesture.moveBy(const Offset(20, 0));
    await gesture.moveBy(const Offset(10, 0));
    await gesture.up();
    await tester.pump();
    expect(events.first, 'start');
    expect(events.last, 'end');
    expect(events.where((e) => e == 'live').length, greaterThanOrEqualTo(1));
  });
}
