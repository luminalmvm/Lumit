// How many bridge calls one interaction costs — the regression trap for FFI
// chatter.
//
// Every generated call crosses the seam through the handler, so counting there
// sees everything: property reads, schema fetches, renders. The budgets pin the
// *shape* of the panels' behaviour — a rebuild that re-reads the world shows up
// here as a number jumping, long before it shows up on a profiler as a slow
// click. Found the hard way: selecting a layer was traced at >200 calls,
// because both panels rebuilt wholesale and every widget re-asked the engine
// for everything it had already been told.
//
// The budgets are deliberately loose (roughly 2x the measured cost at the time
// of writing) so honest growth — a new column, another switch — does not trip
// them, while another rebuild-the-world regression does.

import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/frb_generated.dart';
import 'package:lumit_flutter/state/comp_time.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:uuid/uuid.dart';

import 'frb_test_support.dart';

/// Counts every call that crosses the bridge, by name.
class CountingHandler extends BaseHandler {
  final Map<String, int> calls = {};
  bool counting = false;

  void _tick(String name) {
    if (counting) calls[name] = (calls[name] ?? 0) + 1;
  }

  int get total => calls.values.fold(0, (a, b) => a + b);

  void reset() => calls.clear();

  /// The counts as a readable ranking, for the failure message.
  String ranking() {
    final entries = calls.entries.toList()
      ..sort((a, b) => b.value.compareTo(a.value));
    return entries.map((e) => '${e.value}x ${e.key}').join('\n');
  }

  @override
  Future<S> executeNormal<S, E extends Object>(NormalTask<S, E> task) {
    _tick(task.constMeta.debugName);
    return super.executeNormal(task);
  }

  @override
  S executeSync<S, E extends Object, WireSyncType>(
      SyncTask<S, E, WireSyncType> task) {
    _tick(task.constMeta.debugName);
    return super.executeSync(task);
  }
}

/// Tap a widget near its left end rather than at its centre. A Timeline
/// fold-out row spans the whole outline, which is wider than the panels these
/// tests mount, so `tap` — which aims at the centre — lands off screen.
Future<void> tapNearLeft(WidgetTester tester, Finder finder) =>
    tester.tapAt(tester.getTopLeft(finder) + const Offset(5, 8));

void main() {
  final counter = CountingHandler();

  setUpAll(() async {
    final stem = Platform.isWindows
        ? 'lumit_bridge.dll'
        : Platform.isMacOS
            ? 'liblumit_bridge.dylib'
            : 'liblumit_bridge.so';
    await BridgeLib.init(
      externalLibrary: ExternalLibrary.open('../target/debug/$stem'),
      handler: counter,
    );
  });

  group('Bridge call budget', () {
    testWidgets('selecting a layer costs a bounded number of bridge calls',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      // Two layers with an effect each — a small but honest document.
      comp.addSolidLayer().addEffect(name: 'blur');
      comp.addTextLayer().addEffect(name: 'sharpen');
      p.uiState.setSelectedComp(comp);
      final target = comp.getLayers().first;

      // The other layer starts selected, so the click below changes the
      // selection rather than setting it for the first time — the everyday
      // gesture, and the one that was traced at >200 calls.
      p.uiState.selectedLayer.value = comp.getLayers().last;

      counter
        ..reset()
        ..counting = true;
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(800, 600),
        child: Row(children: const [
          SizedBox(width: 500, height: 600, child: TimelinePanelFrb()),
          Expanded(child: EffectControlsPanelFrb()),
        ]),
      ));
      await tester.pump();
      await settleFrb(tester, minRounds: 8);
      counter.counting = false;
      // ignore: avoid_print
      print('MOUNT COST ${counter.total} calls\n${counter.ranking()}');

      // Twirl the target layer open — Transform and its effect too — which is
      // how a layer is actually being worked on when it gets clicked.
      final id = target.internallayerId.toString();
      await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
      await tester.pump();
      // Near its left end, not its centre: a fold row spans the whole outline,
      // and the outline is wider than this panel (the render-time column
      // widened it again, K-276), so the row's centre is off screen.
      await tapNearLeft(
          tester, find.byKey(ValueKey<String>('tl-group-$id/transform')));
      await tester.pump();
      await settleFrb(tester, minRounds: 4, maxRounds: 8);

      counter
        ..reset()
        ..counting = true;
      // On the name, not the row's centre: the centre of a full outline row
      // lands on the blend dropdown, and a fixed offset lands on whichever
      // cell the column groups put there — the name cell is the safe target.
      final name =
          find.byKey(ValueKey<String>('tl-name-${target.internallayerId}'));
      await tester.tapAt(tester.getTopLeft(name) + const Offset(5, 8));
      await tester.pump(const Duration(milliseconds: 350));
      await settleFrb(tester, minRounds: 4, maxRounds: 8);
      counter.counting = false;

      expect(p.uiState.selectedLayer.value?.equals(layer: target), isTrue,
          reason: 'the click actually changed the selection');
      // ignore: avoid_print
      print('CLICK COST ${counter.total} calls\n${counter.ranking()}');
      // Measured at 5 with the read model in place (K-184) and its revision
      // check folded to one per frame: a selection is pure interface state, so
      // what remains is that one check plus the ruler's own reads. The cap
      // stays roughly 2x measured so honest growth does not trip it.
      expect(
        counter.total,
        lessThan(12),
        reason: 'one click re-read far too much across the bridge:\n'
            '${counter.ranking()}',
      );
    });

    /// **Markers used to cost a bridge call per rebuild and one per frame of
    /// drag.** `get_markers` walked the whole list across the seam on every
    /// ruler build — sixty times a second while playback runs — and a drag
    /// committed a document write for every frame it crossed, which is what
    /// made dragging a flag feel heavy. The list is remembered in Dart until
    /// the document changes, and a drag writes once, on release.
    testWidgets('markers cost nothing per rebuild and one write per drag',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);
      addMarkerFrb(comp, frame: 40, label: 'Chorus');

      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(1280, 600),
        child: const TimelinePanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);

      // Ten rebuilds of the ruler, driven the way playback drives it.
      counter
        ..reset()
        ..counting = true;
      for (var i = 1; i <= 10; i++) {
        p.uiState.playheadFrame.value = i;
        await tester.pump();
        tester.element(find.byType(TimelineRuler)).markNeedsBuild();
        await tester.pump();
      }
      counter.counting = false;
      // ignore: avoid_print
      print('MARKER REBUILD COST ${counter.total} calls\n${counter.ranking()}');
      expect(
        counter.total,
        lessThan(4),
        reason: 'the ruler re-read the marker list on every rebuild:\n'
            '${counter.ranking()}',
      );

      // And the drag: one write, not one per frame crossed.
      final flag = find
          .byKey(ValueKey<String>('tl-marker-${markersOf(comp).single.id}'));
      counter
        ..reset()
        ..counting = true;
      await tester.drag(flag, const Offset(120, 0));
      await tester.pump();
      await settleFrb(tester, minRounds: 4, maxRounds: 8);
      counter.counting = false;
      // ignore: avoid_print
      print('MARKER DRAG COST ${counter.total} calls\n${counter.ranking()}');
      expect(
        counter.calls['composition_reference_set_markers'] ?? 0,
        1,
        reason: 'a drag must write the document once, on release:\n'
            '${counter.ranking()}',
      );
      expect(
        counter.total,
        lessThan(40),
        reason: 'dragging a marker re-read far too much:\n${counter.ranking()}',
      );

      // Adding one. Most of what this costs is the ordinary fan-out of *any*
      // document change — every panel re-reads what it draws — so the budget
      // that matters is the marker's own share of it, which is the read, the
      // write, and one time conversion per existing marker.
      counter
        ..reset()
        ..counting = true;
      addMarkerFrb(comp, frame: 90, label: '2');
      p.state.notifyDocumentChanged();
      await tester.pump();
      await settleFrb(tester, minRounds: 4, maxRounds: 8);
      counter.counting = false;
      // ignore: avoid_print
      print('MARKER ADD COST ${counter.total} calls\n${counter.ranking()}');
      expect(
        (counter.calls['composition_reference_get_markers'] ?? 0) +
            (counter.calls['composition_reference_set_markers'] ?? 0),
        lessThan(4),
        reason: 'adding a marker read or wrote the list more than once:\n'
            '${counter.ranking()}',
      );
    });

    /// **Dragging the zoom slider used to re-read the world per frame.**
    /// The zoom was a plain field, so every step of a drag — and every tick of
    /// a flight — rebuilt the whole panel: the work area came back across the
    /// bridge two to four times, the cache bar asked for the composition's
    /// whole cache map, and the outline rebuilt every row for a change that
    /// happens entirely to the right of the seam. That is the "super super
    /// laggy" the owner reported (K-293). Only the lane side listens to the
    /// zoom now, and the cache bar holds its read until a frame arrives.
    testWidgets('dragging the zoom slider asks the engine almost nothing',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      comp.addTextLayer();
      p.uiState.setSelectedComp(comp);

      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(1280, 600),
        child: const TimelinePanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);

      double barWidth() => tester
          .getRect(
              find.byKey(ValueKey<String>('tl-bar-${layer.internallayerId}')))
          .width;
      final before = barWidth();
      final track =
          tester.getRect(find.byKey(const ValueKey('tl-zoom-slider')));
      counter
        ..reset()
        ..counting = true;
      // Eight steps along the track, the way a hand moves it — not one jump,
      // because the cost being guarded is *per step*. The first is spent
      // crossing the drag slop, which is what starts the drag.
      final gesture =
          await tester.startGesture(Offset(track.left + 2, track.center.dy));
      await tester.pump();
      for (var i = 0; i < 8; i++) {
        await gesture.moveBy(Offset(track.width / 10, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();
      counter.counting = false;

      // A drag that did nothing would cost nothing too, so say that it moved.
      expect(barWidth(), greaterThan(before),
          reason: 'the drag actually zoomed');
      // ignore: avoid_print
      print('ZOOM DRAG COST ${counter.total} calls\n${counter.ranking()}');

      expect(
        counter.calls['composition_reference_cached_frames'] ?? 0,
        lessThan(3),
        reason: 'the cache map was re-read while only the zoom moved:\n'
            '${counter.ranking()}',
      );
      expect(
        counter.calls['composition_reference_get_work_area'] ?? 0,
        lessThan(3),
        reason: 'the work area was re-read per step of the drag:\n'
            '${counter.ranking()}',
      );
      // Loose, in the house style, and the per-name budgets above are the
      // teeth: what must not happen is a count that scales with the number of
      // steps. The revision check the read model makes once a frame is most of
      // what is left here.
      expect(
        counter.total,
        lessThan(40),
        reason: 'a zoom drag re-read far too much:\n${counter.ranking()}',
      );
    });

    /// Hovering the Project panel used to re-fetch names (and once, the
    /// thumbnail) on every enter/exit, because each row asked the engine
    /// again on rebuild. The names ride in on the panel's walk and the
    /// thumbnails live in a RAM cache now, so moving the mouse across the
    /// rows must cost nothing at the seam.
    testWidgets('hovering project rows costs no bridge calls', (tester) async {
      final p = freshProject();
      p.state.project!.newComposition(name: 'Scene');
      p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.state.project!.importFootage(path: 'C:/clips/other.avi');

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        child: const ProjectPanelFrb(),
      ));
      // Let the probes (status, media info, thumbnails) finish and cache.
      await settleFrb(tester, minRounds: 8);

      final rows = [
        find.text('Scene'),
        find.text('shot.mov'),
        find.text('other.avi'),
      ];
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      addTearDown(mouse.removePointer);
      await tester.pump();

      counter
        ..reset()
        ..counting = true;
      // Back and forth across every row, twice.
      for (var pass = 0; pass < 2; pass++) {
        for (final row in rows) {
          await mouse.moveTo(tester.getCenter(row));
          await tester.pump();
        }
      }
      counter.counting = false;

      expect(
        counter.total,
        0,
        reason: 'hovering re-read the engine:\n${counter.ranking()}',
      );
    });

    /// Twirling a layer open changes nothing in the document, so it should
    /// cost nothing at the seam. It used to cost a stack of
    /// `document_revision` calls: the read model checked whether the document
    /// had moved once per *getter*, and a rebuilding timeline reads several.
    testWidgets('twirling a layer open costs few bridge calls', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer().addEffect(name: 'blur');
      comp.addTextLayer();
      p.uiState.setSelectedComp(comp);
      final target = comp.getLayers().first;

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(900, 700),
        child: const TimelinePanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);

      final id = target.internallayerId.toString();
      counter
        ..reset()
        ..counting = true;
      await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
      await tester.pump();
      await settleFrb(tester, minRounds: 2, maxRounds: 6);
      await tapNearLeft(
          tester, find.byKey(ValueKey<String>('tl-group-$id/transform')));
      await tester.pump();
      await settleFrb(tester, minRounds: 2, maxRounds: 6);
      counter.counting = false;

      // ignore: avoid_print
      print('TWIRL COST ${counter.total} calls\n${counter.ranking()}');
      expect(
        counter.total,
        lessThan(12),
        reason: 'opening a twirl re-read the engine:\n${counter.ranking()}',
      );
    });

    /// Clicking a new spot on the ruler used to cost one `time_of_frame` per
    /// animated row on screen — the same question, asked twenty ways, because
    /// each row converted the playhead frame for itself. The answers are
    /// remembered in `state/comp_time.dart` now, so a scrub asks once.
    testWidgets('moving the playhead converts the frame once', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      // Several animated properties, which is what multiplied the calls.
      for (var i = 0; i < 3; i++) {
        final layer = comp.addSolidLayer();
        for (final prop in [
          BridgeTransformProp.opacity,
          BridgeTransformProp.rotation,
        ]) {
          layer.setTransform(
            prop: prop,
            value: BridgeScalar.keyframed([
              for (final (f, v) in [(0, 20.0), (60, 80.0)])
                BridgeKeyframe(
                  time: comp.timeOfFrame(frame: f),
                  value: v,
                  interpIn: const BridgeSideInterp.linear(),
                  interpOut: const BridgeSideInterp.linear(),
                ),
            ]),
          );
        }
      }
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(900, 700),
        child: const TimelinePanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);
      // Twirl every layer's Transform open, so the rows that sample at the
      // playhead are actually on screen.
      for (final layer in comp.getLayers()) {
        final id = layer.internallayerId.toString();
        await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
        await tester.pump();
        await tapNearLeft(
            tester, find.byKey(ValueKey<String>('tl-group-$id/transform')));
        await tester.pump();
      }
      await settleFrb(tester, minRounds: 4, maxRounds: 8);

      counter
        ..reset()
        ..counting = true;
      p.uiState.playheadFrame.value = 30;
      await tester.pump();
      await settleFrb(tester, minRounds: 2, maxRounds: 6);
      counter.counting = false;

      // ignore: avoid_print
      print('SCRUB COST ${counter.total} calls\n${counter.ranking()}');
      expect(
        counter.calls['composition_reference_time_of_frame'] ?? 0,
        lessThanOrEqualTo(1),
        reason: 'each row converted the playhead frame for itself:\n'
            '${counter.ranking()}',
      );
      // Measured at 7: one conversion, plus one sample per animated row —
      // those are genuinely different questions. The keyframe rows used to
      // walk their key lists asking `frame_at_time` per key as well, which is
      // what put this at 67.
      expect(
        counter.total,
        lessThan(20),
        reason: 'a scrub re-read too much across the bridge:\n'
            '${counter.ranking()}',
      );
    });

    /// **The Viewer must ask the engine nothing to show a frame it has.**
    ///
    /// A frame arriving moves the playhead and rebuilds the Viewer's bar. That
    /// bar used to ask two questions on each rebuild: `playback_tier` (twice —
    /// two widgets show the tier) and `viewer_transport`, which reports what
    /// this build compiled to and is thus a constant. At 24 fps that was ~72
    /// calls a second before playback did anything of use, and it grew with the
    /// rate: a 60 fps composition paid 180.
    ///
    /// The tier rides in on the frame now, and the transport is read once.
    /// Neither question crosses the boundary on a rebuild, thus the budget is
    /// zero and not "a few".
    testWidgets('a rebuilt Viewer bar asks the engine nothing', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(900, 600),
        child: const ViewerPanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);

      counter
        ..reset()
        ..counting = true;
      // What a frame arriving does: the playhead moves, and the tier the frame
      // was made at is published. Ten of them, which is under half a second of
      // playback.
      for (var frame = 1; frame <= 10; frame++) {
        p.uiState.playheadFrame.value = frame;
        p.uiState.previewTier.value = frame.isEven ? 2 : 1;
        await tester.pump();
      }
      counter.counting = false;

      // ignore: avoid_print
      print('VIEWER BAR COST ${counter.total} calls\n${counter.ranking()}');
      expect(
        counter.calls['composition_reference_playback_tier'] ?? 0,
        0,
        reason: 'the tier rides in on the frame; nobody asks for it',
      );
      expect(
        counter.calls['viewer_transport'] ?? 0,
        0,
        reason: 'a compile-time constant is read once, not per frame',
      );
      // The exposure box and the tone-map switch (K-314) are told to the engine
      // when they *change*, and are drawn from the value the frontend already
      // holds. A rebuild must not restate them: that would be a call per frame
      // for a setting that has not moved, and every one of them would ask for
      // the frame again in turn.
      expect(
        counter.calls['composition_reference_set_display_view'] ?? 0,
        0,
        reason: 'the display view is pushed on change, never on a rebuild',
      );
      // What is left is one `render_frame` for each move of the playhead —
      // the request the move is for. Measured at 10 for 10 frames; the cap is
      // two for each frame, so honest growth does not trip it.
      expect(
        counter.total,
        lessThanOrEqualTo(20),
        reason: 'a frame arriving re-read the engine:\n${counter.ranking()}',
      );
      // Ten renders were asked for; let the last of them come back before the
      // test ends, or the progress tracker's timer is still pending. Waiting on
      // the condition rather than a round count keeps this independent of how
      // long a frame happens to take — which under the load of the whole suite
      // is longer than for this file alone, and is why it failed there and
      // passed here.
      await settleFrb(
        tester,
        until: () => p.uiState.previewProgress.idle,
        maxRounds: 100,
      );
    });

    /// **A Viewer that has grown asks for the frame again** (K-430).
    ///
    /// On Auto the scale a frame is rendered at is whatever the panel could
    /// show when it laid itself out, and the first layout of a session happens
    /// at whatever size the window opened at. Nothing re-requested the frame
    /// when the panel then grew — growing a panel is neither an edit nor a move
    /// of the playhead — so the first picture stayed at the coarser scale until
    /// something else happened to move.
    ///
    /// This lives among the budgets because it is one: the fix asks for a frame
    /// from a layout, and a layout runs constantly. The count is what says it
    /// asks once for the change rather than once per frame.
    testWidgets('a Viewer that grows asks for the frame again', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);

      final width = ValueNotifier<double>(400);
      addTearDown(width.dispose);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(1200, 700),
        child: ValueListenableBuilder<double>(
          valueListenable: width,
          builder: (context, w, _) => Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              width: w,
              height: 700,
              child: const ViewerPanelFrb(),
            ),
          ),
        ),
      ));
      await settleFrb(tester, minRounds: 8);
      final narrow = p.uiState.viewerScale;

      counter
        ..reset()
        ..counting = true;
      width.value = 1100;
      // Two frames: the layout that measures the wider panel, then the one the
      // request it schedules is made on.
      await tester.pump();
      await tester.pump();
      counter.counting = false;

      expect(p.uiState.viewerScale, greaterThan(narrow),
          reason: 'the wider panel can show more of the composition');
      expect(
        counter.calls['composition_reference_render_frame'] ?? 0,
        greaterThan(0),
        reason: 'the picture stayed at the scale the window opened at:\n'
            '${counter.ranking()}',
      );
      // Once for the change, not once for each layout it caused.
      expect(
        counter.calls['composition_reference_render_frame'] ?? 0,
        lessThanOrEqualTo(2),
        reason: 'a layout asked for a frame every time it ran:\n'
            '${counter.ranking()}',
      );

      await settleFrb(
        tester,
        until: () => p.uiState.previewProgress.idle,
        maxRounds: 100,
      );
    });

    /// **Panning the picture must ask the engine nothing (K-230).**
    ///
    /// A pan moves where the picture is drawn and changes nothing else, but it
    /// rebuilt the whole panel — which re-read the composition's settings, its
    /// size, and every layer's source item, once per movement of the pointer.
    /// At the rate a mouse reports that was hundreds of calls a second, one of
    /// them walking the whole layer list, to re-answer questions only an edit
    /// can change.
    testWidgets('panning with the Hand tool asks the engine nothing',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      for (var i = 0; i < 3; i++) {
        comp.addSolidLayer();
      }
      p.uiState.setSelectedComp(comp);
      p.uiState.tools.select(ToolMode.hand);

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(900, 600),
        child: const ViewerPanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);

      final centre = tester.getCenter(find.byType(ViewerPanelFrb));
      final gesture = await tester.startGesture(centre);
      // Let the mount's own traffic finish before the count starts: what is
      // being measured is the cost of the *movement*, not of what the panel was
      // still doing when the pointer went down.
      await settleFrb(tester, minRounds: 4);

      counter
        ..reset()
        ..counting = true;
      for (var i = 0; i < 20; i++) {
        await gesture.moveBy(const Offset(3, 2));
        // **With time on the clock.** A bare `pump()` does not advance it, so
        // every frame carries the same timestamp — and code that groups its
        // work "once per frame" then sees one frame for the whole gesture and
        // this test sees a cost that does not exist in a running application.
        await tester.pump(const Duration(milliseconds: 16));
      }
      counter.counting = false;
      await gesture.up();
      await tester.pump();

      // ignore: avoid_print
      print('PAN COST ${counter.total} calls\n${counter.ranking()}');
      // Twenty movements, and the composition is read **at most once** in all
      // of them — the once being an edit event arriving mid-gesture and
      // dropping the held answers, which is exactly what should drop them.
      // Before this it was one read per movement, and the layer walk with it.
      expect(
        counter.calls['composition_reference_get_settings'] ?? 0,
        lessThanOrEqualTo(1),
        reason: 'the panel re-read the composition as the pointer moved:\n'
            '${counter.ranking()}',
      );
      expect(
        counter.calls['composition_reference_get_layers'] ?? 0,
        lessThanOrEqualTo(1),
        reason: 'the panel walked the layers as the pointer moved:\n'
            '${counter.ranking()}',
      );
      // Measured at 7 for twenty movements. The cap is what one invalidation
      // costs on a three-layer composition, with room for a fourth layer.
      expect(
        counter.total,
        lessThan(12),
        reason: 'a pan re-read the composition:\n${counter.ranking()}',
      );
    });

    /// **Nor must moving the pointer with a camera tool in hand (K-230).**
    ///
    /// That layer redraws on every movement — its pointer is drawn, so it has
    /// to — and finding the active camera reads the layer's focal distance and
    /// the composition's rate across the bridge. Hovering the picture was
    /// making both, dozens of times a second, without a button being pressed.
    testWidgets('hovering with a camera tool armed asks the engine nothing',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      comp.addCameraLayer();
      p.uiState.setSelectedComp(comp);
      p.uiState.tools.select(ToolMode.cameraOrbit);
      p.uiState.model.refresh();

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(900, 600),
        child: const ViewerPanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);

      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      addTearDown(mouse.removePointer);
      final centre = tester.getCenter(find.byType(ViewerPanelFrb));
      await mouse.moveTo(centre);
      await tester.pump();

      counter
        ..reset()
        ..counting = true;
      for (var i = 0; i < 20; i++) {
        await mouse.moveTo(centre + Offset(i * 3.0, i * 2.0));
        // Real frames, with time between them — see the note in the pan test
        // above. Without it this test measured zero while the tool was asking
        // the engine for the document's revision on every single frame.
        await tester.pump(const Duration(milliseconds: 16));
      }
      counter.counting = false;

      // ignore: avoid_print
      print('CAMERA HOVER COST ${counter.total} calls\n${counter.ranking()}');
      expect(
        counter.total,
        0,
        reason: 'hovering re-found the camera:\n${counter.ranking()}',
      );
    });

    /// **Nor must fronting a composition, while nobody is looking through
    /// anything (K-314).**
    ///
    /// The Viewer's exposure and tone map are per composition, so fronting one
    /// is what puts its view on the renderer — but a view that is neutral onto
    /// a renderer already neutral is nothing to say and nothing to undo, and
    /// the ask for the frame that followed it was a second whole composite on
    /// top of the one the fronting itself asks for. That is every tab click in
    /// every session where neither control has been touched.
    testWidgets('fronting a comp costs no extra frame while neutral',
        (tester) async {
      final p = freshProject();
      final a = p.state.project!.newComposition(name: 'A');
      a.addSolidLayer();
      final b = p.state.project!.newComposition(name: 'B');
      b.addSolidLayer();
      p.uiState.setSelectedComp(a);
      p.uiState.model.refresh();

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(900, 600),
        child: const ViewerPanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);

      counter
        ..reset()
        ..counting = true;
      p.uiState.setSelectedComp(b);
      await tester.pump();
      p.uiState.setSelectedComp(a);
      await tester.pump();
      counter.counting = false;

      // ignore: avoid_print
      print('NEUTRAL FRONTING COST ${counter.total} calls\n'
          '${counter.ranking()}');
      expect(
        counter.calls['composition_reference_set_display_view'] ?? 0,
        0,
        reason: 'a neutral view was pushed onto a neutral renderer:\n'
            '${counter.ranking()}',
      );
      // One composite per fronting is the picture the user asked to see. Two
      // is the frame plus the one the view push added behind it.
      expect(
        counter.calls['composition_reference_render_frame'] ?? 0,
        lessThanOrEqualTo(2),
        reason: 'fronting asked for the frame twice:\n${counter.ranking()}',
      );
    });

    /// **A path drag shows the picture it is making (K-308).**
    ///
    /// Dragging a point used to move the wireframe and leave the picture until
    /// the release, so an edit to a shape was a guess right up to the moment it
    /// was committed. The preview is throttled like every other live drag, so
    /// what this pins is that it happens at all — and that it stays a preview
    /// rather than a write.
    testWidgets('dragging a path point previews the picture', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final shape = comp.addShapeLayer(
        name: 'Square',
        contents: [
          BridgeShapeItem(
            id: UuidValue.fromString(const Uuid().v4()),
            name: 'Rectangle',
            vertices: const [
              BridgeVertex(
                  x: 400, y: 200, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
              BridgeVertex(
                  x: 600, y: 200, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
              BridgeVertex(
                  x: 600, y: 400, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
              BridgeVertex(
                  x: 400, y: 400, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            ],
            closed: true,
            fill: const BridgeColourRgba(r: 1, g: 1, b: 1, a: 1),
            stroke: null,
            strokeWidth: 0,
            opacity: 100,
          ),
        ],
      );
      p.uiState
        ..setSelectedComp(comp)
        ..setSelection([shape]);
      p.uiState.model.refresh();

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(900, 600),
        child: const ViewerPanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);

      // Where the art is drawn: its own coordinates, because a shape layer's
      // box starts at the art's own corner (K-308).
      const barHeight = 26.0;
      final panel = tester.getRect(find.byType(ViewerPanelFrb));
      final stage = Rect.fromLTWH(
          panel.left, panel.top, panel.width, panel.height - barHeight);
      final size = comp.getSize();
      final w = size.width.toDouble();
      final h = size.height.toDouble();
      final scale = math.min(stage.width / w, stage.height / h);
      final fitted = Rect.fromCenter(
        center: stage.center,
        width: w * scale,
        height: h * scale,
      );
      final at = Offset(
        fitted.left + 400 / w * fitted.width,
        fitted.top + 200 / h * fitted.height,
      );

      final gesture = await tester.startGesture(at);
      await settleFrb(tester, minRounds: 4);
      counter
        ..reset()
        ..counting = true;
      for (var i = 0; i < 20; i++) {
        await gesture.moveBy(const Offset(3, 2));
        await tester.pump(const Duration(milliseconds: 16));
      }
      counter.counting = false;
      await gesture.up();
      await tester.pump();

      // ignore: avoid_print
      print('POINT DRAG COST ${counter.total} calls\n${counter.ranking()}');
      expect(
        counter.calls[
                'composition_reference_render_frame_with_shape_preview'] ??
            0,
        greaterThan(0),
        reason: 'the drag showed no picture until it was let go:\n'
            '${counter.ranking()}',
      );
      expect(
        counter.calls['layer_reference_set_shape_contents'] ?? 0,
        0,
        reason: 'a drag previews and commits once, on release',
      );
      // Twenty movements, throttled: the preview and the transform it reads,
      // not a request per pointer report.
      expect(
        counter.total,
        lessThan(30),
        reason: 'a point drag asked the engine too often:\n'
            '${counter.ranking()}',
      );
    });
  }, skip: !engineAvailable);
}
