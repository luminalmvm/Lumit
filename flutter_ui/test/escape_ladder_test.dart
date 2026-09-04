// One Escape, one step back (docs/07-UI-SPEC.md §14.1).
//
// The bug these pin: every surface used to add its own handler to
// `HardwareKeyboard`, which runs all of them on every press whatever the ones
// before returned. So a press meant for a drag also shut the menu behind it and
// cleared the selection behind that. The ladder is the arbiter — one press is
// taken by the innermost rung with something to take back, and by nothing else.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/drag_escape.dart';
import 'package:lumit_flutter/widgets/escape_ladder.dart';

void main() {
  Widget host() => Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(
            initialEntries: [
              OverlayEntry(
                builder: (_) => const Align(
                  alignment: Alignment.topLeft,
                  child: SizedBox(
                    key: ValueKey('anchor'),
                    width: 40,
                    height: 20,
                  ),
                ),
              ),
            ],
          ),
        ),
      );

  void openMenu(BuildContext context) {
    showLumitPopup<void>(
      context: context,
      position: Offset.zero,
      builder: (close) => const FloatSurface(child: Text('menu')),
    );
  }

  tearDown(closeLumitPopups);

  testWidgets('a drag in flight takes Escape before the open menu does',
      (tester) async {
    await tester.pumpWidget(host());
    final anchor = tester.element(find.byKey(const ValueKey('anchor')));

    var reverted = false;
    final drag = DragEscape()..begin(() => reverted = true);
    addTearDown(drag.dispose);
    openMenu(anchor);
    await tester.pump();
    expect(find.text('menu'), findsOneWidget);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();

    expect(reverted, isTrue, reason: 'the drag was put back');
    expect(find.text('menu'), findsOneWidget,
        reason: 'the menu the drag was started over stays up');
    expect(drag.end(), isFalse, reason: 'an abandoned drag commits nothing');
  });

  testWidgets('the open menu takes Escape before a selection is cleared',
      (tester) async {
    await tester.pumpWidget(host());
    final anchor = tester.element(find.byKey(const ValueKey('anchor')));

    var cleared = false;
    addTearDown(EscapeLadder.register(EscapeRung.selection, () {
      cleared = true;
      return true;
    }));
    openMenu(anchor);
    await tester.pump();

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();

    expect(find.text('menu'), findsNothing, reason: 'the chain went');
    expect(cleared, isFalse, reason: 'the selection is still what was picked');
  });

  testWidgets('a selection is cleared when nothing above it claims the press',
      (tester) async {
    await tester.pumpWidget(host());

    var cleared = false;
    addTearDown(EscapeLadder.register(EscapeRung.selection, () {
      cleared = true;
      return true;
    }));

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();

    expect(cleared, isTrue);
  });

  testWidgets('a rung with nothing to take back passes the press down',
      (tester) async {
    await tester.pumpWidget(host());

    var cleared = false;
    // A gesture rung that is registered but has nothing in flight — the state
    // every mounted tool sits in until a drag starts.
    addTearDown(EscapeLadder.register(EscapeRung.gesture, () => false));
    addTearDown(EscapeLadder.register(EscapeRung.selection, () {
      cleared = true;
      return true;
    }));

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();

    expect(cleared, isTrue, reason: 'the idle gesture rung stood aside');
  });
}
