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

    Future<void> mount(WidgetTester tester, dynamic p) async {
      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(1280, 600),
      ));
      await tester.pump();
    }

    /// What one frame is worth in pixels — measured off the ruler, which is
    /// the whole axis.
    double perFrameOf(WidgetTester tester, dynamic p) =>
        (tester.getRect(find.byKey(const ValueKey('tl-ruler'))).width -
            TimelineAxis.pad * 2) /
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

    testWidgets('Ctrl suspends a bar drag\'s magnet', (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      p.uiState.model.refresh();
      markerAt(p, 41);
      await mount(tester, p);

      final perFrame = perFrameOf(tester, p);
      final bar =
          find.byKey(ValueKey<String>('tl-bar-body-${layer.internallayerId}'));
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      final gesture =
          await dragging(tester, tester.getCenter(bar), perFrame * 40);
      await gesture.up();
      await tester.pumpAndSettle();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);

      expect(p.comp.frameAtTime(time: layer.getSpan().inPoint), isNot(41),
          reason: 'Ctrl suspends the magnet, targets and all');
    });

    // -------------------------------------------------------------------
    // §4.5, §7 — the work-area edges snap, and answer Escape.
    // -------------------------------------------------------------------

    testWidgets('a work-area edge lands on the marker it is dragged near',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      // A work area narrower than the comp, so its end handle stands clear of
      // the panel's own right-hand gutter and there is a band to see move.
      p.comp.setWorkArea(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 0),
          outPoint: p.comp.timeOfFrame(frame: 1000),
          startOffset: p.comp.timeOfFrame(frame: 0),
        ),
      );
      // A frame past where the pointer itself would land, so only a reach for
      // the target can explain the landing — and on the far side of the
      // handle, because a flag's label pill runs to the right of its point and
      // would otherwise be the thing under the press.
      markerAt(p, 1060);
      await mount(tester, p);

      final perFrame = perFrameOf(tester, p);
      final handle = find.byKey(const ValueKey('tl-work-end'));
      final gesture =
          await dragging(tester, tester.getCenter(handle), perFrame * 59);
      expect(find.byKey(const ValueKey('tl-ruler-snap-caught')), findsOneWidget,
          reason: 'the capture line marks what caught the edge');

      await gesture.up();
      await tester.pumpAndSettle();
      expect(workAreaFrames(p.comp).end, 1060,
          reason: 'the edge took the marker rather than the pointer');
      expect(find.byKey(const ValueKey('tl-ruler-snap-caught')), findsNothing,
          reason: 'and the capture is gone with the gesture (P1)');
    });

    testWidgets('Escape abandons a work-area drag and writes nothing',
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
      await mount(tester, p);

      final perFrame = perFrameOf(tester, p);
      final gesture = await dragging(
          tester,
          tester.getCenter(find.byKey(const ValueKey('tl-work-end'))),
          -perFrame * 300);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      expect(workAreaFrames(p.comp).end, 1000,
          reason: 'the abandoned drag wrote nothing at all');
    });

    // -------------------------------------------------------------------
    // §4.5, §7 — a marker drag snaps and answers Escape.
    // -------------------------------------------------------------------

    testWidgets('a marker lands on the work-area edge it is dragged near',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.comp.setWorkArea(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 0),
          outPoint: p.comp.timeOfFrame(frame: 50),
          startOffset: p.comp.timeOfFrame(frame: 0),
        ),
      );
      markerAt(p, 10);
      await mount(tester, p);

      final perFrame = perFrameOf(tester, p);
      final id = markersOf(p.comp).single.id;
      final gesture = await dragging(
          tester,
          tester.getCenter(find.byKey(ValueKey<String>('tl-marker-$id'))),
          perFrame * 39);
      expect(find.byKey(const ValueKey('tl-ruler-snap-caught')), findsOneWidget,
          reason: 'the flag says what it caught while it holds it');

      await gesture.up();
      await tester.pumpAndSettle();
      expect(p.comp.frameAtTime(time: markersOf(p.comp).single.time), 50,
          reason: 'the flag took the work-area edge, not the pointer');
    });

    testWidgets('Escape abandons a marker drag and leaves it where it was',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      markerAt(p, 10);
      await mount(tester, p);

      final perFrame = perFrameOf(tester, p);
      final id = markersOf(p.comp).single.id;
      final gesture = await dragging(
          tester,
          tester.getCenter(find.byKey(ValueKey<String>('tl-marker-$id'))),
          perFrame * 100);
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      expect(p.comp.frameAtTime(time: markersOf(p.comp).single.time), 10,
          reason: 'the abandoned drag wrote nothing');
    });

    // -------------------------------------------------------------------
    // §7 — the two double-clicks.
    // -------------------------------------------------------------------

    testWidgets('double-clicking the work-area band gives the comp back',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      p.comp.setWorkArea(
        span: BridgeSpan(
          inPoint: p.comp.timeOfFrame(frame: 20),
          outPoint: p.comp.timeOfFrame(frame: 60),
          startOffset: p.comp.timeOfFrame(frame: 0),
        ),
      );
      await mount(tester, p);
      expect(workAreaFrames(p.comp).whole, isFalse);

      // The band's own row — the whole ruler now — half way along it.
      final band = tester.getRect(find.byKey(const ValueKey('tl-work-area')));
      final at = Offset(band.center.dx, band.center.dy);
      await tester.tapAt(at);
      await tester.pump(const Duration(milliseconds: 40));
      await tester.tapAt(at);
      await tester.pumpAndSettle();

      expect(p.comp.getWorkArea(), isNull,
          reason: 'the work area is the whole comp again (docs/07 §4.1)');
    });

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

    /// §7's last sentence on that gesture: **cancelling the label editor
    /// leaves the marker.** The double-click is what made it; the dialogue
    /// only names it, so backing out of the naming is not an undo of the
    /// making.
    testWidgets('cancelling the label editor leaves the marker',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      await mount(tester, p);

      final ruler = tester.getRect(find.byKey(const ValueKey('tl-ruler')));
      final at = Offset(ruler.center.dx, ruler.top + ruler.height / 4);
      await tester.tapAt(at);
      await tester.pump(const Duration(milliseconds: 40));
      await tester.tapAt(at);
      await tester.pumpAndSettle();
      expect(markersOf(p.comp), hasLength(1));

      await tester.tap(find.byKey(const ValueKey('marker-edit-cancel')));
      await tester.pumpAndSettle();

      expect(markersOf(p.comp), hasLength(1),
          reason: 'the marker outlives the dialogue that would have named it');
      expect(markersOf(p.comp).single.label, isEmpty,
          reason: 'and it says nothing, which is what an unnamed one says');
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
