// The scrub **modifier ladder** (docs/impl/timeline-interaction.md polish 27,
// `Caddis study/notes-editor-ux.md` §3): a value drag has four sensitivities —
// `Shift` ×10, nothing held ×1, `Ctrl` ×0.1, `Alt` ×0.01 — pressing one
// mid-drag takes effect at once, and while the drag runs a floating chip shows
// all four with the one in force boxed. Covers both drag surfaces —
// DragValueField and the draggable TimeReadout clock.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/state/timecode.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/time_readout.dart';

/// An `Overlay` above the field, as the application's own root has: the ladder
/// chip is an overlay entry, so a host without one would quietly show nothing
/// and the tests below would assert against an absence.
Widget _host(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: Overlay(
          initialEntries: [OverlayEntry(builder: (_) => Center(child: child))],
        ),
      ),
    );

/// The same host without the Overlay, for a widget pumped over and over in one
/// test: an `Overlay` keeps the entries it was given at its first mount, so a
/// second `pumpWidget` through [_host] would go on showing the first child.
Widget _bare(Widget child) => Directionality(
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

  /// The study's fourth rung, under `Ctrl` (polish 27). Lumit had three.
  testWidgets('alt makes the drag finer still: a hundredth per pixel',
      (tester) async {
    expect(await dragBy10px(tester, modifier: LogicalKeyboardKey.altLeft),
        closeTo(0.1, 1e-9));
  });

  /// A ladder needs one answer when two rungs are held at once, and the order
  /// is fixed rather than guessed: coarse beats fine.
  testWidgets('shift wins over the finer rungs held with it', (tester) async {
    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    expect(
        await dragBy10px(tester, modifier: LogicalKeyboardKey.shiftLeft), 100);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
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

  // -------------------------------------------------------------------------
  // The floating ladder (polish 27, study §3): the four rungs shown at once,
  // the active one boxed, only while a scrub runs.
  // -------------------------------------------------------------------------

  group('The floating ladder', () {
    /// Which rungs are drawn in a box — the boxed one is the level in force.
    List<String> boxed(WidgetTester tester) => [
          for (final label in ScrubLadder.labels)
            if ((((tester
                            .widget<Container>(find
                                .ancestor(
                                  of: find.text(label),
                                  matching: find.byType(Container),
                                )
                                .first)
                            .decoration! as BoxDecoration)
                        .border! as Border)
                    .top
                    .color ==
                LumitTheme.dark().textPrimary))
              label,
        ];

    testWidgets('shows all four rungs and boxes the one in force',
        (tester) async {
      for (final (factor, label) in [
        (0.01, l10n.scrubLadderAlt),
        (0.1, l10n.scrubLadderCtrl),
        (1.0, l10n.scrubLadderBase),
        (10.0, l10n.scrubLadderShift),
      ]) {
        await tester.pumpWidget(_bare(ScrubLadder(factor: factor)));
        await tester.pump();
        for (final rung in ScrubLadder.labels) {
          expect(find.text(rung), findsOneWidget,
              reason: 'the whole ladder is shown, not only the rung in force');
        }
        expect(boxed(tester), [label],
            reason: 'exactly the level $factor is boxed');
      }
    });

    /// P1: the chip is put up by the gesture and taken down with it, and the
    /// resting field is exactly what it was.
    testWidgets('rides a scrub and leaves with it', (tester) async {
      await tester.pumpWidget(_host(DragValueField(
        value: 0,
        min: -1000,
        max: 1000,
        onChanged: (_) {},
      )));
      expect(find.byType(ScrubLadder), findsNothing, reason: 'nothing at rest');

      final gesture = await tester.startGesture(
        tester.getCenter(find.byType(DragValueField)),
        kind: PointerDeviceKind.mouse,
      );
      await gesture.moveBy(const Offset(30, 0));
      await tester.pump();

      expect(find.byType(ScrubLadder), findsOneWidget,
          reason: 'the ladder is up while the drag runs');
      expect(boxed(tester), [l10n.scrubLadderBase],
          reason: 'nothing held is the base rung');

      // A modifier pressed **without the pointer moving** still changes what
      // the next pixel is worth, so the chip cannot wait for a drag update.
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
      expect(boxed(tester), [l10n.scrubLadderCtrl],
          reason: 'the box follows the key, not the pointer');
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
      expect(boxed(tester), [l10n.scrubLadderBase],
          reason: 'and follows it back up again');

      await gesture.up();
      await tester.pump();
      expect(find.byType(ScrubLadder), findsNothing,
          reason: 'gone the moment the pointer lifts (P1)');
    });
  });
}
