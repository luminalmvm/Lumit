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
import 'package:lumit_flutter/panels/graph_maths.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/src/rust/frb_generated.dart';
import 'package:lumit_flutter/state/animated_mask_paths.dart';
import 'package:lumit_flutter/state/comp_time.dart';
import 'package:lumit_flutter/state/dropper.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/state/viewer_view.dart';
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
      // what is left here — the one per-frame crossing ui-performance §4.5
      // still wants gone, and the one the frb suite shows is load-bearing
      // (§7's WP-4 note), so it is WP-5's to move, not this budget's.
      expect(
        counter.total,
        lessThan(40),
        reason: 'a zoom drag re-read far too much:\n${counter.ranking()}',
      );
    });

    /// **A column-seam drag reaches the engine not at all** (K-529). Column
    /// widths are pure view state, and the owner's report of a lagging seam
    /// was the panel rebuilding whole on every pointer move — so what is
    /// guarded here is the seam's own bill: nought.
    testWidgets('a column-seam drag costs no bridge calls', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
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

      final seam = find.byKey(const ValueKey('tl-seam-identity'));
      final before = tester.getRect(seam);
      counter
        ..reset()
        ..counting = true;
      final gesture = await tester.startGesture(before.center);
      await tester.pump(const Duration(milliseconds: 100));
      for (var i = 0; i < 60; i++) {
        await gesture.moveBy(const Offset(2, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();
      counter.counting = false;

      expect(tester.getRect(seam).left, greaterThan(before.left + 50),
          reason: 'the drag actually widened the column');
      // ignore: avoid_print
      print('SEAM DRAG COST ${counter.total} calls\n${counter.ranking()}');
      // **N = 4.** A sixty-move drag, and the only thing that may cross the
      // seam is the read model's once-a-frame revision check after the one
      // commit on release. Nothing per move, because nothing about a column
      // width is the document's.
      expect(
        counter.total,
        lessThanOrEqualTo(4),
        reason: 'a seam drag reached the engine at all:\n${counter.ranking()}',
      );
    });

    /// **A graph handle drag is bounded, and free where the picture cannot
    /// change** (K-529). The owner's report: dragging a tangent handle fired
    /// calls by the hundred per second wherever it was made. Two things bound
    /// it — the preview throttle, which coalesces the ticks between renders,
    /// and the span guard, which does not ask for a render at all when the
    /// playhead is outside the stretch an ease can change.
    testWidgets('a graph handle drag costs a bounded number of calls',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          // Eased on both sides, because a tangent handle is only drawn on a
          // bezier one — a linear key has no handle to take hold of.
          for (final f in [30, 90, 150])
            BridgeKeyframe(
              time: comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: easyEase,
              interpOut: easyEase,
            ),
        ]),
      );
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

      final id = layer.internallayerId;
      await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
      await tester.pump();
      await tapNearLeft(
          tester, find.byKey(ValueKey<String>('tl-group-$id/transform')));
      await tester.pump();
      await tester.tap(find.text('Opacity'));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('tl-graph')));
      await tester.pump();
      await tester.tap(find.byKey(
          ValueKey<String>('graph-key-$id/transform/opacity@opacity#1')));
      await tester.pump();

      final handle = find.byKey(
          ValueKey<String>('graph-handle-$id/transform/opacity@opacity#1-out'));
      expect(handle, findsOneWidget,
          reason: 'a selected key draws its handles');

      Future<int> costOfDrag() async {
        counter
          ..reset()
          ..counting = true;
        final gesture = await tester.startGesture(tester.getCenter(handle));
        await tester.pump(const Duration(milliseconds: 100));
        for (var i = 0; i < 60; i++) {
          await gesture.moveBy(const Offset(0, -1));
          await tester.pump(const Duration(milliseconds: 8));
        }
        await gesture.up();
        await tester.pumpAndSettle();
        counter.counting = false;
        return counter.total;
      }

      // The playhead sits outside the two keys the ease runs between, so no
      // frame on screen can differ: the drag asks for no preview at all, and
      // the whole bill is the one commit on release.
      p.uiState.playheadFrame.value = 200;
      await tester.pump();
      final away = await costOfDrag();
      // ignore: avoid_print
      print('HANDLE DRAG (playhead away) $away calls\n${counter.ranking()}');
      // **N = 12** for a sixty-move drag: the commit, and the read model's
      // revision checks around it. Nothing per move.
      expect(away, lessThanOrEqualTo(12),
          reason: 'a handle drag away from the span it changes previewed '
              'anyway:\n${counter.ranking()}');

      // With the playhead between the two keys the previews are wanted — and
      // still bounded, because the throttle coalesces them.
      p.uiState.playheadFrame.value = 60;
      await tester.pump();
      final inside = await costOfDrag();
      // ignore: avoid_print
      print(
          'HANDLE DRAG (playhead inside) $inside calls\n${counter.ranking()}');
      // **N = 80** for the same sixty moves — the measured cost is 57, which
      // is the throttle's rate over the drag's length rather than one bill per
      // move. What must never come back is a count that scales with the
      // moves: unthrottled, sixty of them cost two calls each and the total
      // lands past 120.
      expect(inside, lessThanOrEqualTo(80),
          reason: 'a handle drag previewed once per pointer move:\n'
              '${counter.ranking()}');
      expect(inside, greaterThan(away),
          reason: 'and it does preview where the picture can differ');
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
        // The folder row is in here because it is the panel's drop target
        // (K-451): a drop target that asked the engine what it could offer
        // per rebuild would put the chatter back exactly where it was.
        find.text('Compositions'),
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

    /// The Graph panel's whole canvas is arithmetic over one held read
    /// (K-183, docs/impl/node-graph.md §5): `getGraph` is asked when the
    /// selection or the document changes, and nothing about a box needs a
    /// second question. Moving the pointer over the canvas — over boxes, over
    /// sockets, over empty ground — must therefore cost nothing at all.
    testWidgets('hovering the Graph panel\'s canvas asks the engine nothing',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      layer.addEffect(name: 'blur');
      p.uiState.selectedLayer.value = layer;
      p.uiState.model.refresh();

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        child: const GraphPanelFrb(),
        size: const Size(900, 600),
      ));
      await settleFrb(tester, minRounds: 4);

      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      addTearDown(mouse.removePointer);
      await tester.pump();

      counter
        ..reset()
        ..counting = true;
      for (var pass = 0; pass < 2; pass++) {
        for (final spot in [
          tester.getCenter(
              find.byKey(const ValueKey<String>('graph-node-source'))),
          tester
              .getCenter(find.byKey(const ValueKey<String>('graph-node-out'))),
          tester.getCenter(find.byKey(const ValueKey<String>('graph-legend'))),
          const Offset(700, 500),
        ]) {
          await mouse.moveTo(spot);
          await tester.pump();
        }
      }
      counter.counting = false;

      expect(
        counter.total,
        0,
        reason: 'the canvas re-read the engine on a hover:\n'
            '${counter.ranking()}',
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
      // One move first: the batched sampler learns which curves are on screen
      // from the frame before, so the very first frame after a mount is the
      // one it cannot batch. What is measured below is the steady state — the
      // second frame of a scrub onwards, and every frame of playback.
      p.uiState.playheadFrame.value = 29;
      await tester.pump();
      await settleFrb(tester, minRounds: 2, maxRounds: 6);

      counter
        ..reset()
        ..counting = true;
      p.uiState.playheadFrame.value = 30;
      await tester.pump();
      await settleFrb(tester, minRounds: 2, maxRounds: 6);
      counter.counting = false;

      // ignore: avoid_print
      print('SCRUB COST ${counter.total} calls\n${counter.ranking()}');
      // **Not once, either** (ui-performance §4.5): the conversion is exact
      // arithmetic the engine owns and Dart may not do (docs/14 §2), so it
      // moves off the frame rather than into Dart — a miss brings back the
      // whole page of frames around it, and a scrub reads the rest of the
      // page out of memory.
      expect(
        counter.calls['composition_reference_time_of_frame'] ?? 0,
        0,
        reason: 'the playhead frame was converted on the frame that moved:\n'
            '${counter.ranking()}',
      );
      // **One sampling call, whatever the number of lanes open.** It used to be
      // one per animated row — six here, and one per row on the owner's own
      // projects, where a `U` opens dozens. Every row on screen wants the same
      // question answered at the same time, so the first to ask carries the
      // rest with it (`sampledScalar`).
      expect(
        counter.calls['sample_scalar'] ?? 0,
        0,
        reason: 'a row sampled its curve on its own:\n${counter.ranking()}',
      );
      expect(
        counter.calls['sample_scalars'] ?? 0,
        lessThanOrEqualTo(1),
        reason: 'the sampling was not batched into one call:\n'
            '${counter.ranking()}',
      );
      // Measured at 7 when it was one call per row; two now — the conversion
      // and the batch. The keyframe rows used to walk their key lists asking
      // `frame_at_time` per key as well, which is what put this at 67.
      expect(
        counter.total,
        lessThan(20),
        reason: 'a scrub re-read too much across the bridge:\n'
            '${counter.ranking()}',
      );
    });

    /// **A mask is asked about only when its interpolated shape is drawn**
    /// (K-342, ui-performance §4.5). The wireframe wants the shape the picture
    /// has, rather than the one the drawing tools last wrote, on exactly two
    /// conditions: the path is keyed, and the layer is outlined. Both are known
    /// here for nothing — `pathKeys` rides in the read model and the outline
    /// set is the selection — and asking anyway cost ~0.7 ms on every frame of
    /// every scrub of the owner's project, where three keyed masks sit on
    /// layers a scrub is not looking at.
    ///
    /// All three states are pinned, because any one alone is worthless: a
    /// still mask must cost nothing, an unselected keyed mask must cost
    /// nothing, and a **selected** keyed mask must still be asked per frame or
    /// the wireframe goes back to drawing where the shape was.
    testWidgets('a mask is asked about only while its drawn path moves',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addSolidLayer();
      final maskId = UuidValue.fromString(const Uuid().v4());
      layer.addMask(
        mask: BridgeMask(
          id: maskId,
          name: 'Rectangle',
          vertices: const [
            BridgeVertex(
                x: 100, y: 100, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 300, y: 100, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 300, y: 300, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
          ],
          closed: true,
          inverted: false,
          opacity: const BridgeScalar.static_(100),
          mode: BridgeMaskMode.add,
          feather: const BridgeScalar.static_(0),
          vertexFeather: const [],
          expansion: const BridgeScalar.static_(0),
          pathKeys: const [],
        ),
      );
      p.uiState.setSelectedComp(comp);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(900, 600),
        child: const ViewerPanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);

      /// The Viewer's own question, asked the way the stage asks it: the held
      /// model and the outline set decide together, and thirty frames go by.
      int callsOverASweep() {
        final paths = AnimatedMaskPaths();
        final model = p.uiState.model;
        final outlined = p.uiState.outlinedLayerIds;
        final animated = model.heldLayers.any((entry) =>
            outlined.contains(entry.layer.internallayerId.toString()) &&
            entry.info.masks.any((m) => m.pathKeys.isNotEmpty));
        counter
          ..reset()
          ..counting = true;
        for (var frame = 0; frame < 30; frame++) {
          paths.refresh(
            comp: comp,
            frame: frame,
            revision: model.heldRevision,
            anyAnimated: animated,
          );
        }
        counter.counting = false;
        return counter.calls['composition_reference_animated_mask_paths_at'] ??
            0;
      }

      // The premise, stated as a measurement rather than assumed: a mask with
      // no path keys is listed at no frame, so an empty answer at one frame is
      // an empty answer at all of them.
      expect(comp.animatedMaskPathsAt(frame: 0), isEmpty);
      expect(comp.animatedMaskPathsAt(frame: 17), isEmpty);
      p.uiState.setSelection([layer]);
      expect(
        callsOverASweep(),
        0,
        reason: 'a scrub asked the engine about a mask that does not move',
      );

      // Key the path. Selected, the sweep must go back to asking: the vertices
      // it wants genuinely differ frame by frame now.
      layer.toggleMaskPathKey(id: maskId, time: comp.timeOfFrame(frame: 0));
      layer.toggleMaskPathKey(id: maskId, time: comp.timeOfFrame(frame: 60));
      p.uiState.model.refresh();
      await settleFrb(tester, minRounds: 4);
      expect(comp.animatedMaskPathsAt(frame: 17), isNotEmpty,
          reason: 'the path did not take keys, so the rest proves nothing');
      expect(
        callsOverASweep(),
        30,
        reason: 'a keyed mask stopped being read at the frame on screen, so '
            'the wireframe draws where the shape was, not where it is',
      );

      // And deselected — the state the owner's project scrubs in, where three
      // keyed masks sit on layers nothing is outlining — it costs nothing
      // again, because no outline is drawn to want the shape.
      p.uiState.setSelection(const []);
      expect(
        callsOverASweep(),
        0,
        reason: 'a scrub interpolated a mask on a layer nothing outlines',
      );
    });

    /// **The batch does not grow with the lanes.** The test above pins the
    /// shape on six rows; this one doubles the rows and asks for the same
    /// number of calls, which is the claim that actually matters — the owner's
    /// complaint was that scrubbing got worse the more lanes were open.
    testWidgets('a scrub costs the same with twice the rows open',
        (tester) async {
      Future<int> callsForScrub(int layers) async {
        final p = freshProject();
        final comp = p.state.project!.newComposition(name: 'Scene');
        for (var i = 0; i < layers; i++) {
          final layer = comp.addSolidLayer();
          final props = [
            BridgeTransformProp.opacity,
            BridgeTransformProp.rotation,
            BridgeTransformProp.scaleX,
          ];
          for (var j = 0; j < props.length; j++) {
            // A curve of its own per row, so no two rows ask the same question
            // — a document where every layer animates identically would batch
            // to one answer whatever the batching did.
            final lift = i * props.length + j + 1;
            layer.setTransform(
              prop: props[j],
              value: BridgeScalar.keyframed([
                for (final (f, v) in [(0, 20.0 + lift), (60, 80.0 + lift)])
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
        for (final layer in comp.getLayers()) {
          final id = layer.internallayerId.toString();
          await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
          await tester.pump();
          await tapNearLeft(
              tester, find.byKey(ValueKey<String>('tl-group-$id/transform')));
          await tester.pump();
        }
        await settleFrb(tester, minRounds: 4, maxRounds: 8);

        // One move to bring this comp's rows into the batch — the very first
        // frame after a mount is the one nothing has been asked for yet — and
        // then the move that is measured. That is playback's steady state, and
        // it is what a scrub is after its first frame.
        p.uiState.playheadFrame.value = 17;
        await tester.pump();
        await settleFrb(tester, minRounds: 2, maxRounds: 6);

        // Nothing remembered at the frame about to be asked for, or the count
        // would be answered out of memory and pass for free.
        clearCompTimeCache();
        counter
          ..reset()
          ..counting = true;
        p.uiState.playheadFrame.value = 18;
        await tester.pump();
        await settleFrb(tester, minRounds: 2, maxRounds: 6);
        counter.counting = false;
        // **The cold case, which is what a scrub actually is.** The memory was
        // emptied above, so this frame is one nothing has been asked about —
        // the state a sweep across an unvisited stretch is in on every frame
        // of it, and where `time_of_frame` used to cost ~0.6 ms a frame
        // (ui-performance §3.4). One crossing warms the page it lands in.
        expect(
          counter.calls['composition_reference_time_of_frame'] ?? 0,
          0,
          reason: 'a frame nobody had asked about was converted on its own:\n'
              '${counter.ranking()}',
        );
        expect(
          counter.calls['composition_reference_times_of_frames'] ?? 0,
          lessThanOrEqualTo(1),
          reason: 'the page was warmed more than once for one frame:\n'
              '${counter.ranking()}',
        );
        return counter.total;
      }

      final few = await callsForScrub(2);
      final many = await callsForScrub(4);
      // ignore: avoid_print
      print('SCRUB COST 2 layers $few, 4 layers $many');
      expect(many, lessThanOrEqualTo(few),
          reason: 'the scrub got dearer as lanes were opened');
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
      // Auto is the tier that follows the panel, so it is the one a resize can
      // change the render scale of at all (K-430); Full is the default since
      // K-670, and on a fixed tier a resize correctly asks for nothing.
      p.uiState.setPreviewResolution(PreviewResolution.auto);

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

    /// **Nor must a pick drag** (docs/07 §6.1). A window is 129 pixels a side
    /// and the magnifier's 9×9 is cut out of it in Dart, so a sweep across the
    /// picture is meant to cost the engine *nothing at all* until the pointer
    /// nears the window's edge. That is the whole reason a window is read
    /// rather than a pixel: on the far side of `sample_pixels` a read that
    /// misses everything held composites the picture, and a composite per
    /// pointer move is what the window exists to prevent.
    testWidgets('a pick drag reads nothing while its window covers it',
        (tester) async {
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

      final picked = <DropperSample>[];
      p.uiState.armDropper(DropperArm(
        id: 'test',
        reads: DropperReads.colour,
        label: 'Key colour',
        onPick: picked.add,
      ));
      // A window centred on a raster smaller than itself, so it answers for
      // anywhere the pointer can go and no read is due on any of the moves
      // below. The pixels do not matter; the covering does.
      p.uiState.dropperPatch.value = BridgeSampledPixels(
        window: dropperWindow,
        rgba: Uint8List(dropperWindow * dropperWindow * 4),
        width: 40,
        height: 30,
        x: 20,
        y: 15,
        frame: BigInt.zero,
        layerAlone: false,
      );
      await tester.pump();

      final centre = tester.getCenter(find.byType(ViewerPanelFrb));
      final gesture = await tester.startGesture(centre);
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

      expect(picked.length, 1,
          reason: 'the gesture really was a pick on the picture');
      // ignore: avoid_print
      print('PICK DRAG COST ${counter.total} calls\n${counter.ranking()}');
      expect(
        counter.calls['composition_reference_sample_pixels'] ?? 0,
        0,
        reason: 'the drag re-read pixels it already held:\n'
            '${counter.ranking()}',
      );
      expect(
        counter.total,
        lessThan(12),
        reason: 'a pick drag asked the engine things:\n${counter.ranking()}',
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
            trimStart: const BridgeScalar.static_(0),
            trimEnd: const BridgeScalar.static_(100),
            trimOffset: const BridgeScalar.static_(0),
            dashes: const [],
            dashOffset: const BridgeScalar.static_(0),
            gradient: 0,
            gradientColour: null,
            gradientStartX: const BridgeScalar.static_(0),
            gradientStartY: const BridgeScalar.static_(0),
            gradientEndX: const BridgeScalar.static_(0),
            gradientEndY: const BridgeScalar.static_(0),
            combine: 0,
            pathKeys: const [],
            offsetAmount: const BridgeScalar.static_(0),
            repeatCopies: const BridgeScalar.static_(1),
            repeatOffset: const BridgeScalar.static_(0),
            repeatAnchorX: const BridgeScalar.static_(0),
            repeatAnchorY: const BridgeScalar.static_(0),
            repeatPositionX: const BridgeScalar.static_(0),
            repeatPositionY: const BridgeScalar.static_(0),
            repeatRotation: const BridgeScalar.static_(0),
            repeatScale: const BridgeScalar.static_(100),
            repeatStartOpacity: const BridgeScalar.static_(100),
            repeatEndOpacity: const BridgeScalar.static_(100),
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
      // Measured rather than worked out from the panel less a bar height: the
      // Viewer wears a header strip as well as a bottom bar (K-466).
      final stage = tester.getRect(find.byKey(const ValueKey('viewer-stage')));
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

    /// **An edit's follow-on is one wave** (WP-5, ui-performance §4.5).
    ///
    /// A committed op moves the document revision, and every mounted panel
    /// used to answer that by re-asking its own questions **one layer at a
    /// time**: the Viewer walked every layer for its footage list, the
    /// Timeline read every layer's graph, source and volume for its bar
    /// bounds, and the comp-tab strip re-read the settings of every
    /// composition in the project. On the owner's project one switch click
    /// cost 306 crossings and 96 ms of engine time.
    ///
    /// What this pins is the shape rather than the number: one model read for
    /// the revision, and **no per-layer walk from any mounted panel**. The
    /// facts those walks went for ride in the read model now, so a walk
    /// coming back is a count that should be zero and is not.
    testWidgets('an edit refreshes the model once and walks no layer',
        (tester) async {
      final p = freshProject();
      final inner = p.state.project!.newComposition(name: 'Inner');
      inner.addSolidLayer();
      final comp = p.state.project!.newComposition(name: 'Scene');
      // A mixed stack, because each kind was a different walk: a solid asked
      // for its definition, a precomp for its comp's size and its length, and
      // any layer at all for its graph.
      for (var i = 0; i < 5; i++) {
        comp.addSolidLayer();
        comp.addPrecompLayer(comp: inner);
      }
      final layers = comp.getLayers();
      p.uiState.setSelectedComp(comp);
      p.uiState.model.refresh();

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(900, 900),
        child: const Column(children: [
          SizedBox(height: 420, child: ViewerPanelFrb()),
          Expanded(child: TimelinePanelFrb()),
        ]),
      ));
      await settleFrb(tester, minRounds: 8);

      counter
        ..reset()
        ..counting = true;
      // Exactly what a click on a switch cell does: the op, the committing
      // panel's own refresh, and — a turn later — the engine's own report of
      // the same change, which is the second half of the wave.
      layers.first.setSwitch(switch_: BridgeLayerSwitch.locked, on_: true);
      p.uiState.model.refresh();
      p.state.handleChange(ScopedChange(
        project: p.state.project!,
        item: ItemReference.composition(comp),
        layer: layers.first,
        items: false,
      ));
      await tester.pump();
      await settleFrb(tester, minRounds: 4, maxRounds: 8);
      counter.counting = false;

      // ignore: avoid_print
      print('EDIT FOLLOW-ON COST ${counter.total} calls\n${counter.ranking()}');
      expect(
        counter.calls['composition_reference_get_model'] ?? 0,
        lessThanOrEqualTo(1),
        reason: 'the read model was re-read twice for one revision:\n'
            '${counter.ranking()}',
      );
      for (final walk in [
        'layer_reference_get_source_item',
        'layer_reference_get_graph',
        'layer_reference_get_volume_db',
      ]) {
        expect(
          counter.calls[walk] ?? 0,
          0,
          reason: '$walk was asked per layer behind one edit '
              '(${layers.length} layers mounted):\n${counter.ranking()}',
        );
      }
      // The Viewer asks its comp two questions when an edit lands — its
      // settings and its pixel size — and nothing asks per *item*: the
      // comp-tab strip's cached walk holds, because a layer edit cannot add,
      // remove or rename a composition.
      expect(
        counter.calls['composition_reference_get_settings'] ?? 0,
        lessThanOrEqualTo(2),
        reason: 'the settings of every comp in the project were re-read:\n'
            '${counter.ranking()}',
      );
    });
  }, skip: !engineAvailable);
}
