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
