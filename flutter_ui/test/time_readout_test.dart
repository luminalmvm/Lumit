// The clock readouts (K-287): a fixed slot, a click that types, a clamp at
// both ends of the composition, and a drag for the rows that had one.
//
// The width test is the point of the widget: a readout whose box changes size
// as it counts is what moved the Timeline's search field through every second
// of playback.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/timecode.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/time_readout.dart';

void main() {
  Widget host(Widget child) => Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Align(
            alignment: Alignment.topLeft,
            child: Row(mainAxisSize: MainAxisSize.min, children: [child]),
          ),
        ),
      );

  Widget clock(
    int frame, {
    required ValueChanged<int> onCommit,
    int maxFrame = 300,
    int minFrame = 0,
    bool draggable = false,
    ValueChanged<int>? onDragLive,
  }) =>
      TimeReadout(
        key: const ValueKey('clock'),
        frame: frame,
        format: (f) => timecodeOfRate(f, 24, 1),
        parse: (text) => framesOfTimecode(text, 24, 1),
        widthChars: timecodeChars(24, 1),
        style: LumitTheme.dark().mono,
        onCommit: onCommit,
        minFrame: minFrame,
        maxFrame: maxFrame,
        draggable: draggable,
        onDragLive: onDragLive,
      );

  testWidgets('the slot is the same width whatever the time says',
      (tester) async {
    await tester.pumpWidget(host(clock(0, onCommit: (_) {})));
    final atZero = tester.getSize(find.byKey(const ValueKey('clock')));

    // The widest digits, a two-digit frames field and a minutes field: every
    // shape the readout can take, in one number.
    await tester.pumpWidget(host(clock(288, onCommit: (_) {})));
    expect(tester.getSize(find.byKey(const ValueKey('clock'))), atZero,
        reason: 'the box does not resize as the number counts');
  });

  testWidgets('clicking types a time, in the format it was showing',
      (tester) async {
    int? committed;
    await tester.pumpWidget(host(clock(0, onCommit: (f) => committed = f)));

    await tester.tap(find.byKey(const ValueKey('clock')));
    await tester.pump();
    expect(find.byType(EditableText), findsOneWidget,
        reason: 'the readout became a field');

    await tester.enterText(find.byType(EditableText), '00:00:02:00');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pump();
    expect(committed, 48, reason: 'two seconds at 24 fps');
  });

  testWidgets('a time outside the composition lands on the nearest end',
      (tester) async {
    final asked = <int>[];
    await tester
        .pumpWidget(host(clock(0, onCommit: asked.add, maxFrame: 120)));

    await tester.tap(find.byKey(const ValueKey('clock')));
    await tester.pump();
    await tester.enterText(find.byType(EditableText), '01:00:00:00');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pump();
    expect(asked, [120], reason: 'past the end is the end, not an error');
  });

  testWidgets('a time before the start lands on the start', (tester) async {
    final asked = <int>[];
    // A readout whose floor is not zero — a Retime row's, say — so a time
    // below it has somewhere to be clamped to.
    await tester.pumpWidget(
        host(clock(60, onCommit: asked.add, minFrame: 48, maxFrame: 300)));

    await tester.tap(find.byKey(const ValueKey('clock')));
    await tester.pump();
    await tester.enterText(find.byType(EditableText), '00:00:00:00');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pump();
    expect(asked, [48]);
  });

  testWidgets('text that is not a time changes nothing', (tester) async {
    var commits = 0;
    await tester.pumpWidget(host(clock(12, onCommit: (_) => commits++)));

    await tester.tap(find.byKey(const ValueKey('clock')));
    await tester.pump();
    await tester.enterText(find.byType(EditableText), 'soon');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pump();

    expect(commits, 0);
    expect(find.text('00:00:00:12'), findsOneWidget,
        reason: 'the readout went back to showing where things really are');
  });

  testWidgets('escape leaves the time alone', (tester) async {
    var commits = 0;
    await tester.pumpWidget(host(clock(12, onCommit: (_) => commits++)));

    await tester.tap(find.byKey(const ValueKey('clock')));
    await tester.pump();
    await tester.enterText(find.byType(EditableText), '00:00:05:00');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();

    expect(commits, 0);
    expect(find.text('00:00:00:12'), findsOneWidget);
  });

  testWidgets('a draggable readout ticks whole frames and commits once',
      (tester) async {
    final live = <int>[];
    final committed = <int>[];
    await tester.pumpWidget(host(clock(
      10,
      onCommit: committed.add,
      draggable: true,
      onDragLive: live.add,
    )));

    await tester.drag(find.byKey(const ValueKey('clock')), const Offset(40, 0));
    await tester.pump();

    expect(live, isNotEmpty, reason: 'the drag moved the value as it went');
    expect(committed, hasLength(1), reason: 'and committed once, on release');
    expect(committed.single, greaterThan(10));
  });
}
