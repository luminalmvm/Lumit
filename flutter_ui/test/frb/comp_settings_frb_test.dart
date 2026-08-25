// The Composition settings / New composition dialogue (K-180).
//
// Two things are worth testing here and both were bugs before it: what the
// dialogue *writes* when only the frame rate changes, and how the two text
// fields read what is typed into them. A rate typed as a decimal still has to
// reach the engine as the exact pair, and a duration typed as a wall-clock time
// has to survive a rate change untouched — that is the whole of the fix.

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/lumit_icon.dart' as glyph;
import 'package:lumit_flutter/icons/lumit_icons.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart' show addMarkerFrb;
import 'package:lumit_flutter/shell/comp_settings_frb.dart';
import 'package:lumit_flutter/shell/dialog_frame.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart' show BridgeSpan;
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

    test('a whole rate is that rate over one', () {
      expect(parseRate('60'), (60, 1));
      expect(parseRate('600'), (600, 1));
    });

    test('any other decimal is read on the thousandths grid and reduced', () {
      expect(parseRate('12.5'), (25, 2));
    });

    test('nonsense is refused rather than guessed at', () {
      expect(parseRate(''), isNull);
      expect(parseRate('0'), isNull);
      expect(parseRate('-30'), isNull);
      expect(parseRate('fast'), isNull);
    });
  });

  group('the duration field is a length of time', () {
    test('HH:MM:SS.mmm round-trips through exact seconds', () {
      final parsed = parseDurationHms('00:00:11.892');
      expect(parsed, isNotNull);
      expect(formatDurationHms(parsed!), '00:00:11.892');
    });

    test('a colon before the milliseconds reads the same as a full stop', () {
      expect(
          parseDurationHms('00:00:11:892'), parseDurationHms('00:00:11.892'));
    });

    test('shorter forms mean what they obviously mean', () {
      expect(parseDurationHms('1:30'), parseDurationHms('00:01:30.000'));
      expect(parseDurationHms('11.892'), parseDurationHms('00:00:11.892'));
    });

    test('an exact thirty seconds prints as thirty seconds', () {
      expect(formatDurationHms(const BridgeRational(num: 30, den: 1)),
          '00:00:30.000');
    });

    test('nonsense is refused, so a typo cannot shorten a comp', () {
      expect(parseDurationHms('soon'), isNull);
      expect(parseDurationHms(''), isNull);
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

    test('shorter forms mean what they obviously mean', () {
      expect(framesOfTimecode('1:30', 60, 1), 90 * 60);
      expect(framesOfTimecode('30', 60, 1), 30 * 60);
      expect(framesOfTimecode('soon', 60, 1), isNull);
      expect(framesOfTimecode('', 60, 1), isNull);
    });

    test('audio lengths read HH:MM:SS:mmm — milliseconds, not frames', () {
      expect(timecodeOfSecondsMs(11.892), '00:00:11:892');
      expect(timecodeOfSecondsMs(90.5), '00:01:30:500');
      expect(timecodeOfSecondsMs(0), '00:00:00:000');
    });

    test('a duration prints as timecode and parses back to exact seconds', () {
      final second = secondsOfFrames(600, 600, 1);
      expect(second.num.toInt(), 1);
      expect(second.den.toInt(), 1);
      expect(timecodeOfDuration(second, 600, 1), '00:00:01:000');

      // The whole loop: what the field shows, read back at the same rate,
      // is the seconds the document stores.
      final shown = timecodeOfDuration(second, 600, 1);
      final frames = framesOfTimecode(shown, 600, 1)!;
      final stored = secondsOfFrames(frames, 600, 1);
      expect(stored.num.toInt() / stored.den.toInt(), 1.0);
    });
  });

  test('the aspect label is the shape in its smallest whole numbers', () {
    expect(aspectRatioLabel(1920, 816), '40 : 17');
    expect(aspectRatioLabel(1920, 1080), '16 : 9');
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

    testWidgets('a drop-frame preset reaches the engine as its exact pair',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');

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

      await tester.enterText(find.byKey(const ValueKey('comp-fps')), '23.976');
      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      final after = comp.getSettings();
      expect((after.fpsNum, after.fpsDen), (24000, 1001),
          reason: 'a decimal in the field, the exact rate in the document');
    });

    /// The list beside the field says what the field says: a preset by name, or
    /// "Custom" for a rate of one's own. It used to read "Presets" whatever was
    /// typed, which told you nothing about the comp.
    testWidgets('the presets list reads Custom for a rate of one\'s own',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');

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

      expect(find.text('60'), findsWidgets, reason: 'the default rate');
      // Scoped to the rate list: the Preset row above it says Custom for its
      // own reasons — a size and a rate that match no whole format (K-469).
      Finder customRate() => find.descendant(
            of: find.byKey(const ValueKey('comp-fps-presets')),
            matching: find.text('Custom'),
          );
      expect(customRate(), findsNothing,
          reason: '60 is a preset, so it is named');

      await tester.enterText(find.byKey(const ValueKey('comp-fps')), '600');
      await tester.pump();

      expect(customRate(), findsOneWidget,
          reason: 'and it follows the field as it is typed into');
    });

    testWidgets('Cancel writes nothing', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final before = comp.getSettings();

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

      await tester.enterText(find.byKey(const ValueKey('comp-name')), 'Nope');
      await tester.tap(find.byKey(const ValueKey('comp-cancel')));
      await tester.pumpAndSettle();

      expect(comp.getSettings().name, before.name);
    });

    /// The other half of "a rate is not a speed" (K-572): the comp keeps its
    /// length, and the playhead keeps its **moment**. Writing the old frame
    /// number back at the new rate moved the playhead through the comp — at
    /// half the rate it landed twice as far in.
    testWidgets('changing the rate leaves the playhead at the same moment',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      // One second in, at the default 60 fps.
      p.uiState.playheadFrame.value = 60;

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
      await tester.enterText(find.byKey(const ValueKey('comp-fps')), '24');
      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      expect(p.uiState.playheadFrame.value, 24,
          reason: 'still one second in, counted at 24');
    });

    /// A moment that no frame of the new rate lands on goes to the nearest one,
    /// not to the frame before it — otherwise every touch of the rate field
    /// walks the playhead backwards.
    testWidgets('a moment between frames takes the nearest', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      // 62/60 s — 24.8 frames at 24 fps, so frame 25, where a floor gives 24.
      p.uiState.playheadFrame.value = 62;

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
      await tester.enterText(find.byKey(const ValueKey('comp-fps')), '24');
      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      expect(p.uiState.playheadFrame.value, 25);
    });

    /// There is one playhead, and it belongs to the comp being looked at. The
    /// Project panel can open settings for any comp in the bin; changing one of
    /// those must not move the playhead of the comp on screen.
    testWidgets('another comp\'s rate leaves this playhead alone',
        (tester) async {
      final p = freshProject();
      final fronted = p.state.project!.newComposition(name: 'Fronted');
      final other = p.state.project!.newComposition(name: 'Other');
      p.uiState.setSelectedComp(fronted);
      p.uiState.playheadFrame.value = 60;

      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => GestureDetector(
            key: const ValueKey('open'),
            behavior: HitTestBehavior.opaque,
            onTap: () => showCompSettingsFrb(context: context, comp: other),
            child: const SizedBox(width: 200, height: 40),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.tap(find.byKey(const ValueKey('open')));
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const ValueKey('comp-fps')), '24');
      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      expect((other.getSettings().fpsNum, other.getSettings().fpsDen), (24, 1));
      expect(p.uiState.playheadFrame.value, 60);
    });

    /// Markers and the work area are stored as rational time, not as frame
    /// numbers, so a rate change is not their business at all. Asserted rather
    /// than assumed: the moment either of them starts counting frames, the
    /// playhead is not the only thing that needs converting.
    testWidgets('markers and the work area keep their moments', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      addMarkerFrb(comp, frame: 90);
      comp.setWorkArea(
        span: BridgeSpan(
          inPoint: const BridgeRational(num: 1, den: 2),
          outPoint: const BridgeRational(num: 2, den: 1),
          startOffset: const BridgeRational(num: 0, den: 1),
        ),
      );
      final markerBefore = comp.getMarkers().single.time;
      final areaBefore = comp.getWorkArea()!;

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
      await tester.enterText(find.byKey(const ValueKey('comp-fps')), '24');
      await tester.tap(find.byKey(const ValueKey('comp-apply')));
      await tester.pumpAndSettle();

      expect(comp.getMarkers().single.time, markerBefore,
          reason: 'a marker is a time, and the time did not change');
      final areaAfter = comp.getWorkArea()!;
      expect(
          (areaAfter.inPoint, areaAfter.outPoint),
          (areaBefore.inPoint, areaBefore.outPoint),
          reason: 'and so are both ends of the work area');
    });
  });

  /// The dialog measured against its own drawing (K-469). It is the same popup
  /// the export dialog is built from, at its own width and with its own row:
  /// a 110px label column, 12 after it, rows of 30.
  group('New composition metrics (frb)', () {
    setUpAll(initEngineForTests);

    Future<void> open(WidgetTester tester) async {
      tester.view.physicalSize = const Size(1000, 800);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => GestureDetector(
            key: const ValueKey('open'),
            behavior: HitTestBehavior.opaque,
            onTap: () => showNewCompositionFrb(
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
    }

    Rect band(WidgetTester tester, String key) =>
        tester.getRect(find.byKey(ValueKey<String>(key)));

    testWidgets('the dialog is the drawing\'s frame', (tester) async {
      await open(tester);

      final title = band(tester, 'comp-title-strip');
      final footer = band(tester, 'comp-footer');
      expect(title.width, compDialogWidth,
          reason: 'the drawing frames this dialog at 520 wide');
      expect(title.height, dialogTitleStrip + 1,
          reason: '§12A.4: a dialog title strip is 30, over a hairline');
      expect(footer.height, dialogFooterHeight,
          reason: '10 above a 24px button and 10 below it, over a hairline');
      expect(find.text('NEW COMPOSITION'), findsOneWidget,
          reason: 'the title is a kicker — mono capitals');

      await tester.tap(find.byKey(const ValueKey('comp-cancel')));
      await tester.pumpAndSettle();
    });

    testWidgets('a row is 110 and 12, in 30', (tester) async {
      await open(tester);

      final label = tester.getRect(find
          .ancestor(of: find.text('Name'), matching: find.byType(SizedBox))
          .first);
      expect(label.width, compLabelColumn,
          reason: "this drawing's label column is 110, not Export's 100");
      final well = band(tester, 'comp-name');
      expect(well.height, dialogControlHeight,
          reason: '§12A.6: a well in a dialog is 22');
      expect(well.left - label.right, compRowGap,
          reason: 'the control stands 12 after the label column');

      await tester.tap(find.byKey(const ValueKey('comp-cancel')));
      await tester.pumpAndSettle();
    });

    testWidgets('the drawing\'s sections and their rows are all here',
        (tester) async {
      await open(tester);

      expect(find.text('FRAME'), findsOneWidget);
      expect(find.text('MOTION BLUR'), findsOneWidget);
      for (final key in [
        'comp-preset',
        'comp-width',
        'comp-height',
        'comp-size-lock',
        'comp-aspect',
        'comp-fps',
        'comp-fps-presets',
        'comp-duration',
        'comp-duration-reading',
        'comp-background',
        'comp-shutter-angle',
        'comp-samples',
      ]) {
        expect(find.byKey(ValueKey<String>(key)), findsOneWidget,
            reason: '$key is on the drawing');
      }

      await tester.tap(find.byKey(const ValueKey('comp-cancel')));
      await tester.pumpAndSettle();
    });

    /// The mark between width and height is a **chain**, as the drawing draws
    /// it — not a padlock, which says the numbers cannot be changed at all
    /// rather than that they move together. It reads the state, so tying and
    /// untying swap the glyph.
    testWidgets('the size link is the chain, not a padlock', (tester) async {
      await open(tester);

      String glyphIn(String key) => tester
          .widget<glyph.LumitIcon>(find.descendant(
            of: find.byKey(ValueKey<String>(key)),
            matching: find.byType(glyph.LumitIcon),
          ))
          .glyph;

      expect(glyphIn('comp-size-lock'), LumitIcons.link,
          reason: 'the ratio is kept, so the two sides read as chained');
      await tester.tap(find.byKey(const ValueKey('comp-size-lock')));
      await tester.pumpAndSettle();
      expect(glyphIn('comp-size-lock'), LumitIcons.unlink,
          reason: 'untied, the chain is drawn broken');

      await tester.tap(find.byKey(const ValueKey('comp-cancel')));
      await tester.pumpAndSettle();
    });

    /// Every numeric well in this drawing reads from the right, so the digits
    /// of one line up with the digits of the next. The rate is the one that
    /// is typed rather than scrubbed, and it was reading from the left.
    testWidgets('the frame rate reads from the right of its well',
        (tester) async {
      await open(tester);

      expect(
        tester
            .widget<EditableText>(find.descendant(
              of: find.byKey(const ValueKey('comp-fps')),
              matching: find.byType(EditableText),
            ))
            .textAlign,
        TextAlign.right,
      );

      await tester.tap(find.byKey(const ValueKey('comp-cancel')));
      await tester.pumpAndSettle();
    });

    /// The two sections the drawing added are real edits, not decoration: a
    /// shutter set here reaches the composition it makes (K-120, K-469).
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
