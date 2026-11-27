import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_metrics_frb.dart';

void main() {
  const row = 20.0;
  const count = 200;
  const viewport = 200.0;

  Widget stack(ScrollController scroll) => Directionality(
        textDirection: TextDirection.ltr,
        child: Align(
          alignment: Alignment.topLeft,
          child: SizedBox(
            width: 100,
            height: viewport,
            child: SingleChildScrollView(
              controller: scroll,
              child: LazyBlocks(
                controller: scroll,
                heights: List<double>.filled(count, row),
                viewport: viewport,
                builder: (context, i) =>
                    SizedBox(height: row, child: Text('b$i')),
              ),
            ),
          ),
        ),
      );

  bool built(int i) => find.text('b$i').evaluate().isNotEmpty;

  testWidgets('a scroll builds what is seen at once and the margin after',
      (tester) async {
    final scroll = ScrollController();
    await tester.pumpWidget(stack(scroll));
    expect(built(0) && built(19), isTrue);

    scroll.jumpTo(600);
    await tester.pump();
    for (var i = 30; i < 40; i++) {
      expect(built(i), isTrue, reason: 'row $i is in view');
    }
    expect(built(49), isFalse, reason: 'row 49 is a screenful away');

    for (var f = 0; f < 20; f++) {
      await tester.pump();
    }
    for (var i = 40; i < 50; i++) {
      expect(built(i), isTrue, reason: 'row $i was filled in');
    }
    expect(scroll.position.maxScrollExtent, count * row - viewport);
  });
}
