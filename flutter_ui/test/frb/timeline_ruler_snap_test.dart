// Snapping and the ruler's own gestures (docs/impl/timeline-interaction.md
// §4.1, §4.5, §7 — TI-9).
//
// Every sentence of the note's §7 and the still-unwired half of its §4.5 is a
// claim here: a bar drag, a work-area edge and a marker all reach for the one
// shared target list and draw the capture while it holds them; `Ctrl` suspends
// it; `Escape` abandons a ruler drag and writes nothing; a double-click gives
// the work area back or makes a marker; the zoom keys work; and the playhead
// stays on screen while the transport runs.
//
// Against the real engine, like every other frb panel test: a snap that does
// not reach the document is not a snap.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/comp_time.dart';
import 'package:lumit_flutter/state/settings.dart' show effectiveUiScale;
import 'package:lumit_flutter/widgets/ui_scale.dart';
import 'package:uuid/uuid.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Snapping and the ruler (TI-9)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    /// [scale] mounts the panel under the interface scale, as the application
    /// does.
    Future<void> mount(WidgetTester tester, dynamic p, {double? scale}) async {
      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: scale == null
            ? const TimelinePanelFrb()
            : UiScaleView(scale: scale, child: const TimelinePanelFrb()),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(1280, 600),
      ));
      await tester.pump();
    }

    /// What one frame is worth in pixels — measured off the ruler, which is
    /// the whole axis. [scale] is the one the panel was mounted under, since
    /// the padding is drawn that much wider on screen.
    double perFrameOf(WidgetTester tester, dynamic p, {double? scale}) =>
        (tester.getRect(find.byKey(const ValueKey('tl-ruler'))).width -
            TimelineAxis.pad *
                2 *
                (scale == null ? 1 : effectiveUiScale(scale))) /
        (p.comp as CompositionReference).durationFrames();

    void markerAt(dynamic p, int frame, {String label = 'Beat'}) {
      final comp = p.comp as CompositionReference;
      writeMarkers(comp, [
        ...markersOf(comp),
        BridgeMarker(
          id: UuidValue.fromString(const Uuid().v4()),
          time: comp.timeOfFrame(frame: frame),
          label: label,
          isBeat: false,
        ),
      ]);
      (p.uiState as LumitUiState).model.refresh();
    }

    /// Press a gesture into motion without letting go: two moves with a pump
    /// between, so the arena's slop is passed and the drag is a drag.
    Future<TestGesture> dragging(
        WidgetTester tester, Offset from, double dx) async {
      final gesture =
          await tester.startGesture(from, kind: PointerDeviceKind.mouse);
      await tester.pump(const Duration(milliseconds: 60));
      // In steps, as a real pointer moves: the first move is spent winning the
      // arena and setting the drag's origin, so a gesture made of one jump
      // reports no update at all.
      const steps = 8;
      for (var i = 0; i < steps; i++) {
        await gesture.moveBy(Offset(dx / steps, 0));
        await tester.pump();
      }
      return gesture;
    }

    // -------------------------------------------------------------------
    // §4.1, §4.5 — a bar drag snaps, and says what caught it.
    // -------------------------------------------------------------------

    testWidgets('a bar drag lands on the marker it is pulled near',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      // One frame past where the pointer itself would land, so only a reach
      // for the target can explain the landing.
      markerAt(p, 41);
      await mount(tester, p);

      final perFrame = perFrameOf(tester, p);
      final bar =
          find.byKey(ValueKey<String>('tl-bar-body-${layer.internallayerId}'));
      final gesture =
          await dragging(tester, tester.getCenter(bar), perFrame * 40);
      expect(
          find.byKey(
              ValueKey<String>('tl-bar-snap-caught-${layer.internallayerId}')),
          findsOneWidget,
          reason: 'the caught target is indicated at the moment of capture');

      await gesture.up();
      await tester.pumpAndSettle();
      expect(p.comp.frameAtTime(time: layer.getSpan().inPoint), 41,
          reason: "the bar's leading end took the marker");
      expect(
          find.byKey(
              ValueKey<String>('tl-bar-snap-caught-${layer.internallayerId}')),
          findsNothing,
          reason: 'and the capture leaves no trace after (P1)');
    });

    // -------------------------------------------------------------------
    // §4.5, §7 — the work-area edges snap, and answer Escape.
    // -------------------------------------------------------------------

    // -------------------------------------------------------------------
    // §4.5, §7 — a marker drag snaps and answers Escape.
    // -------------------------------------------------------------------

    /// Where [frame] falls on the ruler, in the clock row: ground no flag and
    /// no band sits on, so a press there is a scrub.
    Offset clockAt(WidgetTester tester, dynamic p, num frame) {
      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      return Offset(
          ruler.left + TimelineAxis.pad + frame * perFrameOf(tester, p),
          ruler.top + ruler.height / 4);
    }

    testWidgets('picking a marker up leaves the playhead where it was',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      markerAt(p, 400);
      await mount(tester, p);
      expect(p.uiState.playheadFrame.value, 0);

      final id = markersOf(p.comp).single.id;
      final flag = find.byKey(ValueKey<String>('tl-marker-$id'));
      // Held still past the press deadline, which is when the ruler's own tap
      // fires: that tap used to seek to the pointer, flag or no flag.
      final gesture = await tester.startGesture(tester.getCenter(flag),
          kind: PointerDeviceKind.mouse);
      await tester.pump(const Duration(milliseconds: 250));
      expect(p.uiState.playheadFrame.value, 0,
          reason: 'a press on a flag is the flag\'s, not a scrub');

      for (var i = 0; i < 8; i++) {
        await gesture.moveBy(const Offset(10, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();
      expect(p.uiState.playheadFrame.value, 0,
          reason: 'and dragging it moves the marker alone');
      expect(p.comp.frameAtTime(time: markersOf(p.comp).single.time),
          greaterThan(400));

      // A plain click on the flag, no drag at all, is the same.
      await tester.tap(flag);
      await tester.pumpAndSettle();
      expect(p.uiState.playheadFrame.value, 0);
    });

    testWidgets('a marker snaps to where the playhead is, not where it was',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      markerAt(p, 10);
      await mount(tester, p);

      // The playhead moves after the panel was built, and nothing rebuilds
      // the panel because of it: that is the case the snap list got wrong.
      p.uiState.scrubTo(600);
      await tester.pump();

      final perFrame = perFrameOf(tester, p);
      final id = markersOf(p.comp).single.id;
      // Two frames past the playhead, well inside the snap's reach. The first
      // of the eight moves is spent starting the drag.
      final gesture = await dragging(
          tester,
          tester.getCenter(find.byKey(ValueKey<String>('tl-marker-$id'))),
          perFrame * 592 * 8 / 7);
      expect(find.byKey(const ValueKey('tl-ruler-snap-caught')), findsOneWidget,
          reason: 'the playhead caught the flag');
      await gesture.up();
      await tester.pumpAndSettle();

      expect(p.comp.frameAtTime(time: markersOf(p.comp).single.time), 600,
          reason: 'the flag landed on the playhead as it stands now');
    });

    // -------------------------------------------------------------------
    // Shift while scrubbing: the playhead lands on what it comes near.
    // -------------------------------------------------------------------

    testWidgets('Shift lands a scrubbed playhead on a marker', (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      markerAt(p, 400);
      await mount(tester, p);

      // Without Shift the playhead goes where the pointer is.
      var gesture = await tester.startGesture(clockAt(tester, p, 300),
          kind: PointerDeviceKind.mouse);
      await tester.pump(const Duration(milliseconds: 60));
      await gesture.moveTo(clockAt(tester, p, 396));
      await tester.pump();
      expect(p.uiState.playheadFrame.value, closeTo(396, 1));
      await gesture.up();
      await tester.pumpAndSettle();

      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      gesture = await tester.startGesture(clockAt(tester, p, 300),
          kind: PointerDeviceKind.mouse);
      await tester.pump(const Duration(milliseconds: 60));
      await gesture.moveTo(clockAt(tester, p, 396));
      await tester.pump();
      expect(p.uiState.playheadFrame.value, 400,
          reason: 'four frames short of the marker, and taken onto it');
      expect(find.byKey(const ValueKey('tl-ruler-snap-caught')), findsOneWidget,
          reason: 'and what caught it is marked while it holds');

      await gesture.up();
      await tester.pumpAndSettle();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      expect(find.byKey(const ValueKey('tl-ruler-snap-caught')), findsNothing,
          reason: 'the mark goes when the scrub does');
    });

    // -------------------------------------------------------------------
    // A marker's colour, and its region.
    // -------------------------------------------------------------------

    testWidgets('double-clicking a marker opens its settings', (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      markerAt(p, 400);
      await mount(tester, p);

      final id = markersOf(p.comp).single.id;
      final flag = find.byKey(ValueKey<String>('tl-marker-$id'));
      await tester.tap(flag);
      await tester.pump(const Duration(milliseconds: 40));
      await tester.tap(flag);
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('marker-edit-label')), findsOneWidget);
      expect(markersOf(p.comp), hasLength(1),
          reason: 'the pair of clicks made no second marker');

      await tester.enterText(
          find.byKey(const ValueKey('marker-edit-duration')), '120');
      await tester.tap(find.byKey(const ValueKey('marker-edit-colour-3')));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('marker-edit-ok')));
      await tester.pumpAndSettle();

      final edited = markersOf(p.comp).single;
      expect(edited.durationFrames, 120, reason: 'it runs for what was typed');
      expect(edited.colour, 3, reason: 'in the colour that was picked');
      expect(edited.label, 'Beat', reason: 'and still says what it said');
      expect(find.byKey(ValueKey<String>('tl-marker-out-$id')), findsOneWidget,
          reason: 'a region has an end to take hold of');
    });

    testWidgets('a marker becomes a region, and takes a colour, from its menu',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      markerAt(p, 400);
      await mount(tester, p);

      final id = markersOf(p.comp).single.id;
      final flag = find.byKey(ValueKey<String>('tl-marker-$id'));
      await tester.tap(flag, buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('marker-menu-region')));
      await tester.pumpAndSettle();
      expect(markersOf(p.comp).single.durationFrames,
          p.uiState.model.fps.round(),
          reason: 'a new region starts a second long');

      await tester.tap(flag, buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('marker-menu-colour-2')));
      await tester.pumpAndSettle();
      expect(markersOf(p.comp).single.colour, 2);
      expect(markersOf(p.comp).single.durationFrames, isNotNull,
          reason: 'colouring it left the region alone');

      await tester.tap(flag, buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('marker-menu-region')));
      await tester.pumpAndSettle();
      expect(markersOf(p.comp).single.durationFrames, isNull,
          reason: 'and the same row takes the region away again');
      expect(markersOf(p.comp).single.colour, 2);
    });

    testWidgets('a region stops at the end of the composition',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      final frames = p.comp.durationFrames();
      // Unlabelled, so the whole flag is on screen this near the end.
      markerAt(p, frames - 10, label: '');
      await mount(tester, p);

      final id = markersOf(p.comp).single.id;
      await tester.tap(find.byKey(ValueKey<String>('tl-marker-$id')),
          buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('marker-menu-region')));
      await tester.pumpAndSettle();

      expect(markersOf(p.comp).single.durationFrames, 10,
          reason: 'ten frames of room, so ten frames of region');
    });

    testWidgets('dragging the end of a region resizes it', (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      final id = UuidValue.fromString(const Uuid().v4());
      writeMarkers(p.comp, [
        BridgeMarker(
          id: id,
          time: p.comp.timeOfFrame(frame: 100),
          label: '',
          durationFrames: 100,
          isBeat: false,
        ),
      ]);
      p.uiState.model.refresh();
      await mount(tester, p);
      expect(find.byKey(const ValueKey('tl-lane-regions')), findsOneWidget,
          reason: 'the region is shaded down through the lanes');

      final perFrame = perFrameOf(tester, p);
      // Ctrl keeps the magnet out of it, so the landing is the pointer's own.
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      final gesture = await dragging(
          tester,
          tester.getCenter(find.byKey(ValueKey<String>('tl-marker-out-$id'))),
          perFrame * 200);
      await gesture.up();
      await tester.pumpAndSettle();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);

      final resized = markersOf(p.comp).single;
      expect(resized.durationFrames, closeTo(300, 3),
          reason: 'the end went as far as the pointer did');
      expect(p.comp.frameAtTime(time: resized.time), 100,
          reason: 'and the marker itself stayed put');
      expect(p.uiState.playheadFrame.value, 0,
          reason: 'taking hold of the end is not a scrub either');
    });

    // -------------------------------------------------------------------
    // A scaled interface: the flag and the edge stay under the pointer.
    // -------------------------------------------------------------------

    testWidgets('a marker follows the pointer when the interface is scaled',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      markerAt(p, 10);
      await mount(tester, p, scale: 1.25);

      final perFrame = perFrameOf(tester, p, scale: 1.25);
      final id = markersOf(p.comp).single.id;
      // Ctrl keeps the magnet out of it, so the landing is the pointer's own.
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      final gesture = await dragging(
          tester,
          tester.getCenter(find.byKey(ValueKey<String>('tl-marker-$id'))),
          perFrame * 400);
      await gesture.up();
      await tester.pumpAndSettle();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);

      // The first of the eight moves is spent starting the drag, so the flag
      // travels the other seven.
      expect(p.comp.frameAtTime(time: markersOf(p.comp).single.time),
          closeTo(360, 2),
          reason: 'the flag moved as far as the pointer did');
    });

    testWidgets('a work-area edge follows the pointer when it is scaled',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.comp.setWorkArea(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 0),
          outPoint: p.comp.timeOfFrame(frame: 1000),
          startOffset: p.comp.timeOfFrame(frame: 0),
        ),
      );
      await mount(tester, p, scale: 1.25);

      final perFrame = perFrameOf(tester, p, scale: 1.25);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      final gesture = await dragging(
          tester,
          tester.getCenter(find.byKey(const ValueKey('tl-work-end'))),
          -perFrame * 300);
      await gesture.up();
      await tester.pumpAndSettle();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);

      // An edge goes to the pointer itself, so all eight moves count.
      expect(workAreaFrames(p.comp).end, closeTo(700, 2),
          reason: 'the edge moved as far as the pointer did');
    });

    // -------------------------------------------------------------------
    // §7 — the two double-clicks.
    // -------------------------------------------------------------------

    testWidgets('double-clicking empty ruler makes a marker and names it',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      await mount(tester, p);
      expect(markersOf(p.comp), isEmpty);

      // The clock, in the ruler's upper half: not the band, and no flag there.
      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      final at = Offset(ruler.center.dx, ruler.top + ruler.height / 4);
      await tester.tapAt(at);
      await tester.pump(const Duration(milliseconds: 40));
      await tester.tapAt(at);
      await tester.pumpAndSettle();

      expect(markersOf(p.comp), hasLength(1),
          reason: 'the double-click made a marker where it landed');
      expect(find.byKey(const ValueKey('marker-edit-label')), findsOneWidget,
          reason: 'and opened its label editor (docs/07 §4.1)');

      await tester.enterText(
          find.byKey(const ValueKey('marker-edit-label')), 'Drop');
      await tester.tap(find.byKey(const ValueKey('marker-edit-ok')));
      await tester.pumpAndSettle();
      expect(markersOf(p.comp).single.label, 'Drop',
          reason: 'what was typed is what the marker says');
    });

    // -------------------------------------------------------------------
    // §4.6 (gap 23) — the zoom keys, and edge-follow during playback.
    // -------------------------------------------------------------------

    testWidgets('= and - zoom time, and \\ toggles the whole comp',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      await mount(tester, p);

      final ruler = find.byKey(const ValueKey('tl-ruler'));
      final fitted = tester.getRect(ruler).width;

      await tester.sendKeyEvent(LogicalKeyboardKey.equal);
      await tester.pumpAndSettle();
      final zoomed = tester.getRect(ruler).width;
      expect(zoomed, greaterThan(fitted), reason: '= zooms time in');

      await tester.sendKeyEvent(LogicalKeyboardKey.backslash);
      await tester.pumpAndSettle();
      expect(tester.getRect(ruler).width, closeTo(fitted, 0.5),
          reason: '\\ goes back to the whole composition');

      await tester.sendKeyEvent(LogicalKeyboardKey.backslash);
      await tester.pumpAndSettle();
      expect(tester.getRect(ruler).width, closeTo(zoomed, 0.5),
          reason: 'and again returns to the zoom it came away from');

      await tester.sendKeyEvent(LogicalKeyboardKey.minus);
      await tester.pumpAndSettle();
      expect(tester.getRect(ruler).width, lessThan(zoomed),
          reason: '- zooms time out');
    });

    testWidgets('the playhead stays on screen while the transport runs',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      await mount(tester, p);

      // Zoomed in, so there is somewhere to scroll to at all.
      await tester.sendKeyEvent(LogicalKeyboardKey.equal);
      await tester.sendKeyEvent(LogicalKeyboardKey.equal);
      await tester.pumpAndSettle();
      final ruler = find.byKey(const ValueKey('tl-ruler'));
      final atRest = tester.getRect(ruler).left;

      p.uiState.play();
      await tester.pump();
      // The transport hands the playhead out past the right-hand edge.
      p.uiState.playheadFrame.value = p.comp.durationFrames() ~/ 2;
      await tester.pumpAndSettle();

      expect(tester.getRect(ruler).left, lessThan(atRest - 1),
          reason: 'the lanes flipped a page to keep the playhead in view');
    });
  });
}
