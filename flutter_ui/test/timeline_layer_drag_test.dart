// A layer drag at Minimal and None: no row moves, a line marks where the drop
// would land, and the layer lands there. The targeting arithmetic has its own
// file (timeline_drag_test.dart). This one needs no engine.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_metrics_frb.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  const row = 22.0;

  /// A stack of named blocks, in [order], wrapped the way both halves of the
  /// Timeline wrap theirs.
  Widget stack(
    LayerDragState drag,
    ScrollController scroll,
    List<String> order, {
    AnimationLevel level = AnimationLevel.all,
    ThemeShape shape = ThemeShape.studio,
  }) {
    final heights = List<double>.filled(order.length, row);
    return Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.forScheme(LumitColorScheme.dark, shape),
        animationLevel: level,
        showTooltips: false,
        child: Align(
          alignment: Alignment.topLeft,
          child: SizedBox(
            width: 200,
            height: 400,
            child: SingleChildScrollView(
              controller: scroll,
              child: LazyBlocks(
                controller: scroll,
                heights: heights,
                viewport: 400,
                raised: drag.raised,
                builder: (context, i) => LayerDragSlide(
                  drag: drag,
                  heights: heights,
                  index: i,
                  child: SizedBox(
                    key: ValueKey<String>(order[i]),
                    height: row,
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  double top(WidgetTester tester, String name) =>
      tester.getRect(find.byKey(ValueKey<String>(name))).top;

  test('the drop mark stands on the seam the layer would land against', () {
    final heights = [20.0, 30.0, 40.0, 50.0];
    expect(layerDragMark(heights, null), isNull);
    expect(layerDragMark(heights, const LayerDrag(1, 1)), isNull,
        reason: 'a drop that moves nothing marks nothing');
    expect(layerDragMark(heights, const LayerDrag(0, 2)), 90,
        reason: 'taken down, it lands under the row it is aimed at');
    expect(layerDragMark(heights, const LayerDrag(3, 1)), 20,
        reason: 'taken up, it lands over it');
    expect(layerDragMark(heights, const LayerDrag(3, 0)), 0);
    expect(layerDragMark(heights, const LayerDrag(0, 3)), 140);
  });

  /// docs/15-DESIGN §8.1: at *Minimal* and *None* a layer is not carried. No
  /// row moves, and a line marks the place the drop would take.
  testWidgets('a drag that is not carried moves nothing and marks the drop',
      (tester) async {
    for (final level in [AnimationLevel.minimal, AnimationLevel.none]) {
      final drag = LayerDragState();
      final scroll = ScrollController();
      const before = ['a', 'b', 'c'];
      final heights = List<double>.filled(before.length, row);
      await tester.pumpWidget(stack(drag, scroll, before, level: level));

      drag.lift(0, carries: false);
      drag.carry(heights, 40);
      await tester.pump();
      expect(drag.value, const LayerDrag(0, 2));
      expect({for (final name in before) name: top(tester, name)},
          {'a': 0.0, 'b': row, 'c': row * 2},
          reason: 'at $level every row stays where it stands');
      expect(drag.mark.value, row * 3, reason: 'under the last row');
      expect(drag.band.value, isNull, reason: 'nothing is lifted');
      expect(drag.raised.value, isNull);
      expect(tester.hasRunningAnimations, isFalse);

      // Back up past where it began, and on to the top.
      drag.carry(heights, 0);
      await tester.pump();
      expect(drag.mark.value, isNull, reason: 'back where it began');

      drag.carry(heights, 40);
      final landed = drag.release(heights);
      expect(landed, const LayerDrag(0, 2));
      expect(drag.mark.value, isNull, reason: 'the line goes with the drag');
      await tester.pumpWidget(
          stack(drag, scroll, const ['b', 'c', 'a'], level: level));
      expect({for (final name in before) name: top(tester, name)},
          {'b': 0.0, 'c': row, 'a': row * 2},
          reason: 'the new order, with nothing travelling to it');
      expect(tester.hasRunningAnimations, isFalse);

      await tester.pumpWidget(const SizedBox());
      drag.dispose();
      scroll.dispose();
    }
  });

  testWidgets('a cancelled mark takes the line away and moves nothing',
      (tester) async {
    final drag = LayerDragState();
    final scroll = ScrollController();
    addTearDown(drag.dispose);
    addTearDown(scroll.dispose);
    const order = ['a', 'b', 'c'];
    final heights = List<double>.filled(order.length, row);
    await tester
        .pumpWidget(stack(drag, scroll, order, level: AnimationLevel.none));

    drag.lift(2, carries: false);
    drag.carry(heights, -40);
    expect(drag.mark.value, 0, reason: 'over the top row');
    final landed = drag.release(heights, cancelled: true);
    expect(landed, const LayerDrag(2, 2));
    expect(drag.mark.value, isNull);
    await tester.pump();
    expect(top(tester, 'c'), row * 2);
  });
}
