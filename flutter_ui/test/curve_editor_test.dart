
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/curve_editor.dart';

void main() {
  Widget host(Widget child, {double width = 300}) => Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          // No Overlay: its entries are built once, so a second `pumpWidget`
          // would keep showing the first tree and the test would assert
          // against a stale layout.
          child: Center(child: SizedBox(width: width, child: child)),
        ),
      );

  Widget editor(
    List<(int, List<List<double>>)> commits, {
    double plotSize = curvePlotSizeDefault,
    ValueChanged<double>? onPlotSize,
  }) =>
      CurveChannelEditor(
        keyPrefix: 'curves',
        labels: const ['Master', 'Red', 'Green'],
        curves: const [curveIdentity, curveIdentity, curveIdentity],
        onLive: (_, __) {},
        onCommit: (c, p) => commits.add((c, p)),
        resetTip: 'Reset this curve',
        plotSize: plotSize,
        onPlotSize: onPlotSize,
      );

  testWidgets('a channel button switches which curve an edit lands on',
      (tester) async {
    final commits = <(int, List<List<double>>)>[];
    await tester.pumpWidget(host(editor(commits)));
    await tester.pump();

    expect(find.text('M'), findsOneWidget);
    expect(find.text('R'), findsOneWidget);
    expect(find.text('G'), findsOneWidget);
    expect(find.text('Master'), findsNothing,
        reason: 'the channel strip above the plot is gone');

    await tester.tapAt(tester.getCenter(find.byType(CurveEditor)));
    await tester.pump();
    expect(commits.single.$1, 0);
    expect(commits.single.$2.length, 3);

    commits.clear();
    await tester.tap(find.byKey(const ValueKey<String>('curves-tab-2')));
    await tester.pump();
    await tester.tapAt(tester.getCenter(find.byType(CurveEditor)));
    await tester.pump();
    expect(commits.single.$1, 2,
        reason: 'the edit must land on the channel showing');
    expect(commits.single.$2.length, 3);
  });

  testWidgets('a corner point is caught by the half of its knob past the square',
      (tester) async {
    final commits = <List<List<double>>>[];
    const size = 150.0;
    await tester.pumpWidget(host(
      CurveEditor(
        points: curveIdentity,
        size: size,
        onLive: (_) {},
        onCommit: commits.add,
      ),
    ));
    await tester.pump();
    final topLeft = tester.getTopLeft(find.byType(CurveEditor));
    final grab = topLeft + const Offset(size - 3, 3);
    await tester.dragFrom(grab, const Offset(0, 40));
    await tester.pump();
    expect(commits, isNotEmpty, reason: 'the drag took hold of the point');
    expect(commits.last.last[1], lessThan(1.0),
        reason: 'the white point came down');
  });

  testWidgets('a drag redraws the plot while nothing else rebuilds it',
      (tester) async {
    const size = 150.0;
    await tester.pumpWidget(host(
      CurveEditor(
        points: curveIdentity,
        size: size,
        onLive: (_) {},
        onCommit: (_) {},
      ),
    ));
    await tester.pump();
    final plot = find.descendant(
        of: find.byType(CurveEditor), matching: find.byType(CustomPaint));
    final topLeft = tester.getTopLeft(find.byType(CurveEditor));
    final gesture = await tester.startGesture(topLeft + const Offset(size - 3, 3));
    await gesture.moveBy(const Offset(0, 20));
    await tester.pump();
    final first = tester.widget<CustomPaint>(plot).painter!;
    await gesture.moveBy(const Offset(0, 20));
    await tester.pump();
    final second = tester.widget<CustomPaint>(plot).painter!;
    expect(second.shouldRepaint(first), isTrue,
        reason: 'the point moved, so the plot must repaint');
    await gesture.up();
  });
}
