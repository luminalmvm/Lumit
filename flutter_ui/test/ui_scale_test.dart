// UI scale (Settings → Interface → UI scale, K-117): the [UiScaleView] wrapper
// scales layout AND hit-testing together — the mechanism recorded in
// widgets/ui_scale.dart and docs/archive/flutter-port/05.
//
// K-560 put a presentation baseline of ×1.1 underneath the user's own factor:
// the size the owner tested at 110% is what the shipped 100% now draws. So the
// factor the view is *given* is the user's, and what it *draws* is that over
// the baseline — which is what these expectations are written against.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/widgets/ui_scale.dart';

void main() {
  testWidgets('the shipped factor draws the interface a tenth larger',
      (tester) async {
    await tester.binding.setSurfaceSize(const Size(880, 660));
    final key = GlobalKey();
    await tester.pumpWidget(Directionality(
      textDirection: TextDirection.ltr,
      child: UiScaleView(
        scale: 1.0,
        child: SizedBox.expand(key: key),
      ),
    ));

    // The baseline is a real scale, so there IS a transform at 100% now.
    expect(find.byType(Transform), findsOneWidget);
    // The child lays out at the window divided by 1.1 and paints back to fill
    // it: 880 ÷ 1.1 = 800.
    expect(tester.getSize(find.byKey(key)).width, closeTo(800, 0.01));
    expect(tester.getRect(find.byKey(key)).width, closeTo(880, 0.01));
  });

  testWidgets('a user factor that cancels the baseline is a pass-through',
      (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));
    final key = GlobalKey();
    await tester.pumpWidget(Directionality(
      textDirection: TextDirection.ltr,
      child: UiScaleView(
        // What a settings file written before K-560 at 100% migrates to.
        scale: 1 / uiScaleBaseline,
        child: SizedBox.expand(key: key),
      ),
    ));
    // Effective 1×: no Transform in the tree, nothing to invert on a pointer.
    expect(find.byType(Transform), findsNothing);
    expect(tester.getSize(find.byKey(key)), const Size(800, 600));
  });

  testWidgets('at 2× the user factor the child lays out at the drawn size',
      (tester) async {
    await tester.binding.setSurfaceSize(const Size(880, 660));
    final key = GlobalKey();
    await tester.pumpWidget(Directionality(
      textDirection: TextDirection.ltr,
      child: UiScaleView(
        scale: 2.0,
        child: SizedBox.expand(key: key),
      ),
    ));

    // The child's OWN logical size is the window divided by the drawn scale —
    // 2 × 1.1 — so nothing overflows.
    final size = tester.getSize(find.byKey(key));
    expect(size.width, closeTo(880 / 2.2, 0.01));
    expect(size.height, closeTo(660 / 2.2, 0.01));

    // Its on-screen (post-transform) rect fills the whole window — the scale
    // paints it back up to size.
    final rect = tester.getRect(find.byKey(key));
    expect(rect.width, closeTo(880, 0.01));
    expect(rect.height, closeTo(660, 0.01));
  });

  testWidgets('hit-testing stays coherent with the scaled layout',
      (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));
    var tapped = false;
    await tester.pumpWidget(Directionality(
      textDirection: TextDirection.ltr,
      child: UiScaleView(
        scale: 1.5,
        child: Align(
          alignment: Alignment.center,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () => tapped = true,
            child: const SizedBox(width: 40, height: 40),
          ),
        ),
      ),
    ));

    // tester.tap dispatches at the target's GLOBAL centre; if the Transform's
    // inverse were not applied to the pointer, this would miss the 40×40 box.
    await tester.tap(find.byType(GestureDetector));
    expect(tapped, isTrue);
  });
}
