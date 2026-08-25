// One active menu chain at a time (K-519).
//
// The bug this pins: every menu, dropdown and picker used to push an overlay
// entry of its own, with a click-away barrier of its own and no knowledge of
// the others. Moving the pointer quickly across the menu bar, an Add-effect
// list and a picker left several menus on screen at once, each wanting its own
// click to dismiss. `showLumitPopup` now keeps one chain: opening a popup from
// outside every open popup replaces the chain, opening one from *inside* a
// popup extends it, and one click away — or one Escape — takes the lot.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

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

  /// A popup showing [label], opened from [context].
  void open(BuildContext context, String label, {Offset at = Offset.zero}) {
    showLumitPopup<void>(
      context: context,
      position: at,
      builder: (close) => FloatSurface(
        child: MenuRow(
          key: ValueKey<String>('row-$label'),
          onPressed: () => close(null),
          child: Text(label),
        ),
      ),
    );
  }

  tearDown(closeLumitPopups);

  testWidgets('opening a second popup takes the first one down',
      (tester) async {
    await tester.pumpWidget(host());
    final anchor = tester.element(find.byKey(const ValueKey('anchor')));

    open(anchor, 'first');
    await tester.pump();
    expect(find.text('first'), findsOneWidget);

    // The pointer crossed onto another opener before the first menu had been
    // dismissed. Only the second one may live.
    open(anchor, 'second', at: const Offset(60, 60));
    await tester.pump();
    expect(find.text('first'), findsNothing,
        reason: 'the first chain went when the second one started');
    expect(find.text('second'), findsOneWidget);
  });

  testWidgets('a popup opened from inside one extends the chain',
      (tester) async {
    await tester.pumpWidget(host());
    final anchor = tester.element(find.byKey(const ValueKey('anchor')));

    late BuildContext inside;
    showLumitPopup<void>(
      context: anchor,
      position: Offset.zero,
      builder: (close) => FloatSurface(
        child: Builder(builder: (context) {
          inside = context;
          return const Text('parent');
        }),
      ),
    );
    await tester.pump();

    open(inside, 'flyout', at: const Offset(60, 60));
    await tester.pump();
    expect(find.text('parent'), findsOneWidget,
        reason: 'a flyout is part of its menu, not a menu that replaces it');
    expect(find.text('flyout'), findsOneWidget);
  });

  testWidgets('one click away dismisses the whole chain', (tester) async {
    await tester.pumpWidget(host());
    final anchor = tester.element(find.byKey(const ValueKey('anchor')));

    late BuildContext inside;
    showLumitPopup<void>(
      context: anchor,
      position: Offset.zero,
      builder: (close) => FloatSurface(
        child: Builder(builder: (context) {
          inside = context;
          return const Text('parent');
        }),
      ),
    );
    await tester.pump();
    open(inside, 'flyout', at: const Offset(60, 60));
    await tester.pump();
    expect(lumitPopupOpen, isTrue);

    // Well clear of both surfaces, on the barrier.
    await tester.tapAt(const Offset(400, 500));
    await tester.pump();
    expect(find.text('parent'), findsNothing);
    expect(find.text('flyout'), findsNothing);
    expect(lumitPopupOpen, isFalse,
        reason: 'one click away, not one per menu that happened to be open');
  });

  testWidgets('Escape dismisses the whole chain', (tester) async {
    await tester.pumpWidget(host());
    final anchor = tester.element(find.byKey(const ValueKey('anchor')));

    late BuildContext inside;
    showLumitPopup<void>(
      context: anchor,
      position: Offset.zero,
      builder: (close) => FloatSurface(
        child: Builder(builder: (context) {
          inside = context;
          return const Text('parent');
        }),
      ),
    );
    await tester.pump();
    open(inside, 'flyout', at: const Offset(60, 60));
    await tester.pump();

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();
    expect(find.text('parent'), findsNothing);
    expect(find.text('flyout'), findsNothing);
    expect(lumitPopupOpen, isFalse);
  });

  testWidgets('picking a row closes the popup and leaves nothing behind',
      (tester) async {
    await tester.pumpWidget(host());
    final anchor = tester.element(find.byKey(const ValueKey('anchor')));

    open(anchor, 'only');
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('row-only')));
    await tester.pump();
    expect(find.text('only'), findsNothing);
    expect(lumitPopupOpen, isFalse);
  });
}
