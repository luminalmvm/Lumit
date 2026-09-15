import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/fx_section.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

/// The label column is fixed until the control column has its room, and takes
/// the rest of the row from there, so a long name shows on a wide panel.
void main() {
  const fixed = fxKeyColumnWidth + 4 + fxLabelColumnWidth + fxControlColumnWidth;

  test('the label keeps its width until the controls have theirs', () {
    expect(fxLabelWidthFor(200), fxLabelColumnWidth);
    expect(fxLabelWidthFor(fixed), fxLabelColumnWidth);
    expect(fxLabelWidthFor(fixed + 150), fxLabelColumnWidth + 150);
    expect(fxLabelWidthFor(double.infinity), fxLabelColumnWidth);
  });

  Future<double> labelWidthAt(WidgetTester tester, double width) async {
    const key = ValueKey('name');
    await tester.pumpWidget(Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: Align(
          alignment: Alignment.topLeft,
          child: SizedBox(
            width: width,
            child: Builder(
              builder: (context) => fxTwoColumnRow(
                context: context,
                name: const SizedBox.expand(key: key),
                control: const SizedBox(width: 50),
              ),
            ),
          ),
        ),
      ),
    ));
    return tester.getSize(find.byKey(key)).width;
  }

  testWidgets('a wider row hands its spare width to the name', (tester) async {
    expect(await labelWidthAt(tester, 300), fxLabelColumnWidth);
    expect(await labelWidthAt(tester, fixed + 120), fxLabelColumnWidth + 120);
  });
}
