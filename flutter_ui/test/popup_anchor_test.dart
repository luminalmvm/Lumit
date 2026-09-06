// A popup anchors in the space it is drawn in.
//
// The bug this pins is the one the owner saw after changing the UI scale:
// every menu, dropdown and context menu says where it wants to be with a
// `localToGlobal` on the control that opened it — a **window** coordinate —
// and then gets laid out inside an [Overlay], whose space is only the window's
// while nothing between them transforms the picture. The UI scale transforms
// it (widgets/ui_scale.dart), so at 125% a menu opened halfway down the window
// appeared a further quarter of the way down, further out the lower it went.
//
// `showLumitPopup` converts the anchor through the overlay's own box, so the
// menu lands under the pointer at every scale. The claim is written the way a
// person would check it: open a menu at a point, measure where it turned up.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/ui_scale.dart';

void main() {
  /// The application's own arrangement in miniature: the scale view wrapping
  /// an overlay, with a control in it to open a menu from.
  Widget host(double userScale) => Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: UiScaleView(
            scale: userScale,
            child: Overlay(
              initialEntries: [
                OverlayEntry(
                  builder: (_) => const Positioned(
                    left: 120,
                    top: 90,
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
        ),
      );

  tearDown(closeLumitPopups);

  /// Where the menu turned up, against where the control asked for it — both
  /// in window coordinates, which is what a person's eye and a pointer use.
  Future<void> menuLandsUnderTheControl(
    WidgetTester tester,
    double userScale,
  ) async {
    await tester.binding.setSurfaceSize(const Size(1200, 800));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(host(userScale));

    // Exactly what every call site computes: the control's own box, in window
    // coordinates, just under its bottom edge.
    final anchor = tester.renderObject<RenderBox>(
      find.byKey(const ValueKey('anchor')),
    );
    final asked = anchor.localToGlobal(Offset(0, anchor.size.height));

    showLumitPopup<void>(
      context: tester.element(find.byKey(const ValueKey('anchor'))),
      position: asked,
      // A bare box, not a `FloatSurface`: the surface's own padding and edge
      // would be measured as part of the offset under test.
      builder: (close) => const SizedBox(
        key: ValueKey('menu'),
        width: 120,
        height: 60,
      ),
    );
    await tester.pump();

    final landed = tester.getRect(find.byKey(const ValueKey('menu'))).topLeft;
    expect(landed.dx, closeTo(asked.dx, 0.5),
        reason: 'the menu was asked for at ${asked.dx} across');
    expect(landed.dy, closeTo(asked.dy, 0.5),
        reason: 'the menu was asked for at ${asked.dy} down');
  }

  testWidgets('a menu opens where the control is, at the shipped scale',
      (tester) => menuLandsUnderTheControl(tester, 1.0));

  testWidgets('and still does when the interface is scaled up',
      (tester) => menuLandsUnderTheControl(tester, 1.25));

  testWidgets('and when it is scaled back to native size',
      (tester) => menuLandsUnderTheControl(tester, 1 / uiScaleBaseline));

  testWidgets('and when it is scaled down further',
      (tester) => menuLandsUnderTheControl(tester, 0.75));
}
