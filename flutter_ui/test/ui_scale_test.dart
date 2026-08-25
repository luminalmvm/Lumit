// UI scale (Settings → Interface → UI scale, K-117): the [UiScaleView] wrapper
// scales layout AND hit-testing together — the mechanism recorded in
// widgets/ui_scale.dart and docs/archive/flutter-port/05.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/widgets/ui_scale.dart';

void main() {
  testWidgets('at 1x it is a plain pass-through (no Transform inserted)',
      (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));
    final key = GlobalKey();
    await tester.pumpWidget(Directionality(
      textDirection: TextDirection.ltr,
      child: UiScaleView(
        scale: 1.0,
        child: SizedBox.expand(key: key),
      ),
    ));
    expect(find.byType(Transform), findsNothing);
    // The child fills the window unchanged.
    expect(tester.getSize(find.byKey(key)), const Size(800, 600));
  });

  testWidgets('at 2x the child lays out at half size but paints full-window',
      (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));
    final key = GlobalKey();
    await tester.pumpWidget(Directionality(
      textDirection: TextDirection.ltr,
      child: UiScaleView(
        scale: 2.0,
        child: SizedBox.expand(key: key),
      ),
    ));

    // The child's OWN logical size is the window divided by the scale — layout
    // happened at the scaled size, so nothing overflows.
    final size = tester.getSize(find.byKey(key));
    expect(size.width, closeTo(400, 0.01));
    expect(size.height, closeTo(300, 0.01));

    // Its on-screen (post-transform) rect fills the whole window — the scale
    // paints it back up to size.
    final rect = tester.getRect(find.byKey(key));
    expect(rect.width, closeTo(800, 0.01));
    expect(rect.height, closeTo(600, 0.01));
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
