// Scrub modifiers, the After Effects convention: holding Shift makes a value
// drag coarse (×10), holding Ctrl makes it fine (×0.1), and pressing either
// mid-drag takes effect at once. Covers both drag surfaces — DragValueField
// and the draggable TimeReadout clock.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/timecode.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/time_readout.dart';

Widget _host(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: Center(child: child),
      ),
    );

void main() {
  /// Drags a DragValueField 10 px with [modifier] held and returns how far the
  /// value moved. The first 30 px cross the gesture slop and are pumped away
  /// before the baseline is taken, so the measured move is exactly 10 px.
  Future<num> dragBy10px(WidgetTester tester,
      {LogicalKeyboardKey? modifier}) async {
    num value = 0;
    await tester.pumpWidget(_host(StatefulBuilder(
      builder: (_, setState) => DragValueField(
        value: value,
        min: -1000000,
        max: 1000000,
        onChanged: (v) => setState(() => value = v),
      ),
    )));

    final gesture = await tester.startGesture(
      tester.getCenter(find.byType(DragValueField)),
      kind: PointerDeviceKind.mouse,
    );
    await gesture.moveBy(const Offset(30, 0));
    await tester.pump();
    final before = value;

    if (modifier != null) await tester.sendKeyDownEvent(modifier);
    await gesture.moveBy(const Offset(10, 0));
    await tester.pump();
    if (modifier != null) await tester.sendKeyUpEvent(modifier);

    await gesture.up();
    await tester.pump();
    return value - before;
  }

  testWidgets('a plain drag moves the value one unit per pixel',
      (tester) async {
    expect(await dragBy10px(tester), 10);
  });

  testWidgets('shift makes the drag coarse: ten units per pixel',
      (tester) async {
    expect(
        await dragBy10px(tester, modifier: LogicalKeyboardKey.shiftLeft), 100);
  });

  testWidgets('ctrl makes the drag fine: a tenth of a unit per pixel',
      (tester) async {
    expect(await dragBy10px(tester, modifier: LogicalKeyboardKey.controlLeft),
        closeTo(1, 1e-9));
  });

  /// A fast drag delivers several pointer events per frame — quicker than the
  /// caller can rebuild the field with each staged value. No travel may be
  /// lost to that: the drag keeps its own running value between rebuilds.
  /// Before this, every tick within a frame restarted from the stale prop, so
  /// a screen-wide scrub lost most of its distance and read as acceleration.
  testWidgets('a drag faster than the rebuilds loses no travel',
      (tester) async {
    final emitted = <num>[];
    // The value prop is frozen at 0 — the harshest case: no rebuild ever
    // catches up with the drag.
    await tester.pumpWidget(_host(DragValueField(
      value: 0,
      min: -1000000,
      max: 1000000,
      onChanged: emitted.add,
    )));

    final gesture = await tester.startGesture(
      tester.getCenter(find.byType(DragValueField)),
      kind: PointerDeviceKind.mouse,
    );
    await gesture.moveBy(const Offset(30, 0));
    await tester.pump();
    final base = emitted.isEmpty ? 0 : emitted.last;
    // Ten moves, no pump between them: every tick lands on the same build.
    for (var i = 0; i < 10; i++) {
      await gesture.moveBy(const Offset(10, 0));
    }
    await gesture.up();
    await tester.pump();
    expect(emitted.last - base, 100,
        reason: 'all ten 10 px moves count, not only the last one');
  });

  /// Drags the clock readout with [modifier] held and returns how many frames
  /// it moved during the measured stretch. As above, an unmeasured first move
  /// crosses the slop; the measured move is exactly [px] pixels.
  Future<int> dragClock(WidgetTester tester,
      {required double px, LogicalKeyboardKey? modifier}) async {
    final live = <int>[];
    await tester.pumpWidget(_host(TimeReadout(
      key: const ValueKey('clock'),
      frame: 10,
      format: (f) => timecodeOfRate(f, 24, 1),
      parse: (text) => framesOfTimecode(text, 24, 1),
      widthChars: timecodeChars(24, 1),
      style: LumitTheme.dark().mono,
      minFrame: 0,
      maxFrame: 100000,
      draggable: true,
      onDragLive: live.add,
      onCommit: (_) {},
    )));

    final gesture = await tester.startGesture(
      tester.getCenter(find.byKey(const ValueKey('clock'))),
      kind: PointerDeviceKind.mouse,
    );
    await gesture.moveBy(const Offset(40, 0));
    await tester.pump();
    final before = live.isEmpty ? 10 : live.last;

    if (modifier != null) await tester.sendKeyDownEvent(modifier);
    await gesture.moveBy(Offset(px, 0));
    await tester.pump();
    if (modifier != null) await tester.sendKeyUpEvent(modifier);

    await gesture.up();
    await tester.pump();
    return (live.isEmpty ? 10 : live.last) - before;
  }

  testWidgets('the clock ticks one frame per four pixels, plain',
      (tester) async {
    expect(await dragClock(tester, px: 4), 1);
  });

  testWidgets('shift drags the clock ten frames per four pixels',
      (tester) async {
    expect(
        await dragClock(tester, px: 4, modifier: LogicalKeyboardKey.shiftLeft),
        10);
  });

  testWidgets('ctrl drags the clock one frame per forty pixels',
      (tester) async {
    expect(
        await dragClock(tester,
            px: 40, modifier: LogicalKeyboardKey.controlLeft),
        1);
  });
}
