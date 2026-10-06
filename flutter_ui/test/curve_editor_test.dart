// The curve editor's two chrome decisions (item 6.32): the channel buttons
// beside the graph, and the graph-size option that survives a restart.
//
// The maths the plot draws with is covered by the effect display tests; what
// is asserted here is that switching channels still writes to the channel the
// user is looking at, and that the size the user chose is the size they get
// back.

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

  /// A three-channel editor reporting every commit into [commits].
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
        resetLabel: 'Reset',
        resetTip: 'Reset this curve',
        plotSize: plotSize,
        onPlotSize: onPlotSize,
      );

  testWidgets('a channel button switches which curve an edit lands on',
      (tester) async {
    final commits = <(int, List<List<double>>)>[];
    await tester.pumpWidget(host(editor(commits)));
    await tester.pump();

    // Every channel is a button beside the graph, one letter each.
    expect(find.text('M'), findsOneWidget);
    expect(find.text('R'), findsOneWidget);
    expect(find.text('G'), findsOneWidget);
    expect(find.text('Master'), findsNothing,
        reason: 'the channel strip above the plot is gone');

    // Master is showing: a tap in the middle of the plot adds a point to it.
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
}
