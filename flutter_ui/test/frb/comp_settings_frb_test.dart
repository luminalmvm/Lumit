// The Composition settings / New composition dialogue.
//
// Two things are worth testing here and both were bugs before it: what the
// dialogue *writes* when only the frame rate changes, and how the two text
// fields read what is typed into them. A rate typed as a decimal still has to
// reach the engine as the exact pair, and a duration typed as a wall-clock time
// has to survive a rate change untouched — that is the whole of the fix.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/comp_settings_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/state/timecode.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  group('the rate field reads a decimal as an exact rate', () {
    test('the NTSC family comes back exact, not as the rounded decimal', () {
      // 23.976 is a *rounding* of 24000/1001, so no arithmetic on the rounded
      // number recovers the rate — it has to be matched by name.
      expect(parseRate('23.976'), (24000, 1001));
      expect(parseRate('29.97'), (30000, 1001));
      expect(parseRate('59.94'), (60000, 1001));
    });
  });

  group('the duration field is a length of time', () {
    test('HH:MM:SS.mmm round-trips through exact seconds', () {
      final parsed = parseDurationHms('00:00:11.892');
      expect(parsed, isNotNull);
      expect(formatDurationHms(parsed!), '00:00:11.892');
    });
  });

  group('the duration field speaks HH:MM:SS:FF timecode', () {
    test('timecode round-trips at plain, NTSC and high rates', () {
      expect(timecodeOfRate(90, 60, 1), '00:00:01:30');
      // 29.97 counts thirty frames to the second (the Viewer's own rule).
      expect(timecodeOfRate(899, 30000, 1001), '00:00:29:29');
      // A wide rate widens the frames field rather than lying in two digits.
      expect(timecodeOfRate(7135, 600, 1), '00:00:11:535');
      expect(framesOfTimecode('00:00:11:535', 600, 1), 7135);
      expect(framesOfTimecode('00:00:01:30', 60, 1), 90);
    });
  });

  group('the dialogue against the engine', () {
    setUpAll(initEngineForTests);

    /// **The regression this dialogue exists to fix.** Change only the rate and
    /// press Save: the comp must keep its length and its layers their timing.
    /// The old dialogue wrote yesterday's frame *count* back at the new rate,
    /// which halved or doubled the comp under layers that had not moved.
    testWidgets('changing only the rate does not retime the comp',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final spanBefore = comp.getLayers().single.getSpan();
      expect(comp.durationFrames(), 1800, reason: '30 s at the default 60 fps');

      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => GestureDetector(
            key: const ValueKey('open'),
            behavior: HitTestBehavior.opaque,
            onTap: () => showCompSettingsFrb(context: context, comp: comp),
            child: const SizedBox(width: 200, height: 40),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.tap(find.byKey(const ValueKey('open')));
      await tester.pumpAndSettle();

      expect(find.text('00:00:30:00'), findsOneWidget,
          reason: 'the duration opens as timecode at the comp rate');

      await tester.enterText(find.byKey(const ValueKey('comp-fps')), '30');
      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      final after = comp.getSettings();
      expect((after.fpsNum, after.fpsDen), (30, 1));
      expect(after.duration, const BridgeRational(num: 30, den: 1),
          reason: 'still thirty seconds long');
      expect(comp.durationFrames(), 900,
          reason: 'the same thirty seconds, counted half as finely');
      expect(comp.getLayers().single.getSpan(), spanBefore,
          reason: 'the layer occupies the same time — the rate is not a speed');
    });
  });

  /// The dialog measured against its own drawing. It is the same popup
  /// the export dialog is built from, at its own width and with its own row:
  /// a 110px label column, 12 after it, rows of 30.
  group('New composition metrics (frb)', () {
    setUpAll(initEngineForTests);

    /// The two sections the drawing added are real edits, not decoration: a
    /// shutter set here reaches the composition it makes.
    testWidgets('the shutter the dialog sets is the comp\'s own',
        (tester) async {
      tester.view.physicalSize = const Size(1000, 800);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      CompositionReference? made;
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => GestureDetector(
            key: const ValueKey('open'),
            behavior: HitTestBehavior.opaque,
            onTap: () async => made = await showNewCompositionFrb(
                context: context, project: p.state.project!),
            child: const SizedBox(width: 200, height: 40),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1000, 800),
      ));
      await tester.tap(find.byKey(const ValueKey('open')));
      await tester.pumpAndSettle();

      // Scrubbed the way a mouse does it: a plain drag is one unit a pixel.
      final gesture = await tester.startGesture(
        tester.getCenter(find.byKey(const ValueKey('comp-samples'))),
        kind: PointerDeviceKind.mouse,
      );
      await gesture.moveBy(const Offset(2, 0));
      await tester.pump();
      await gesture.moveBy(const Offset(8, 0));
      await tester.pump();
      await gesture.moveBy(const Offset(8, 0));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
      expect(
          (tester.widget(find.byKey(const ValueKey('comp-samples')))
                  as DragValueField)
              .value,
          greaterThan(16),
          reason: 'the field took the scrub');
      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      expect(made, isNotNull, reason: 'the dialog made the comp');
      expect(made!.getSettings().motionBlurSamples, greaterThan(16),
          reason: 'the sample count the dialog was left on is the comp\'s');
    });
  }, skip: !engineAvailable);
}
