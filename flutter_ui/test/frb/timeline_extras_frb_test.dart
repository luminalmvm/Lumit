// The Timeline's chrome on frb: comp tabs, cache bar, search, the parent
// picker, markers, the work area and the razor.
//
// Driven through the panel rather than in isolation, for the same reason as
// everywhere else here: what matters is that a click reaches the document.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';

import 'package:lumit_flutter/state/tools.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Timeline chrome (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      // The outline alone is 800 px of columns; the default 800×600 test
      // surface would push its right edge (and the lanes) off screen.
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

    /// Open the toolbar's ⋯ menu, where the layer/work-area/marker commands
    /// live now that the toolbar row belongs to the readouts and the search.
    Future<void> openMore(WidgetTester tester) async {
      await tester.tap(find.byKey(const ValueKey('tl-more')));
      await tester.pumpAndSettle();
    }

    testWidgets('the comp tabs show the open comps and front one',
        (tester) async {
      final p = withComp();
      final second = p.state.project!.newComposition(name: 'Titles');
      await mount(tester, p);

      expect(find.byKey(ValueKey<String>('tl-tab-${p.comp.internalid}')),
          findsOneWidget);
      expect(find.byKey(ValueKey<String>('tl-tab-${second.internalid}')),
          findsNothing,
          reason: 'a comp nobody has fronted is not an open tab');

      p.uiState.setSelectedComp(second);
      await tester.pump();
      final tab = find.byKey(ValueKey<String>('tl-tab-${second.internalid}'));
      expect(tab, findsOneWidget, reason: 'fronting a comp opens its tab');

      await tester
          .tap(find.byKey(ValueKey<String>('tl-tab-${p.comp.internalid}')));
      await tester.pump();
      expect(p.uiState.selectedComp?.internalid, p.comp.internalid);
      expect(tab, findsOneWidget, reason: 'switching away keeps the tab open');
    });

    /// The × closes only the tab: the comp stays in the project, and closing
    /// the fronted tab fronts its nearest remaining neighbour.
    testWidgets('closing a comp tab keeps the comp and fronts a neighbour',
        (tester) async {
      final p = withComp();
      final second = p.state.project!.newComposition(name: 'Titles');
      p.uiState.setSelectedComp(second);
      await mount(tester, p);

      await tester.tap(
          find.byKey(ValueKey<String>('tl-tab-close-${second.internalid}')));
      await tester.pump();

      expect(find.byKey(ValueKey<String>('tl-tab-${second.internalid}')),
          findsNothing);
      expect(p.uiState.selectedComp?.internalid, p.comp.internalid,
          reason: 'the neighbour fronted');
      expect(p.state.comps().map((c) => c.$2), contains('Titles'),
          reason: 'closing a tab never deletes the comp');

      // Closing the last tab leaves no comp fronted, and the panel says so.
      await tester.tap(
          find.byKey(ValueKey<String>('tl-tab-close-${p.comp.internalid}')));
      await tester.pump();
      expect(p.uiState.selectedComp, isNull);
      expect(find.textContaining('Open a composition'), findsOneWidget);
    });

    /// **The bridge error where the Timeline should be.** Pre-compose, step
    /// into the new comp, undo: the layers come back and the comp they were
    /// packed into stops existing — with the Timeline still fronting it, every
    /// panel read a comp the engine had never heard of. What has gone cannot
    /// stay fronted, so the user goes back where they came from.
    testWidgets('undoing away the fronted comp goes back to the previous one',
        (tester) async {
      final p = withComp();
      p.comp.addSolidLayer();
      final packed = p.comp.precompose(
        layerIds: [p.comp.getLayers().single.internallayerId],
        name: 'Packed',
        leaveAttributes: false,
        adjustDuration: false,
      );
      final inner = switch (packed.getSourceItem()!) {
        ItemReference_Composition(:final field0) => field0,
        _ => throw StateError('a Precomp layer draws from a composition'),
      };
      p.uiState.setSelectedComp(inner);
      await mount(tester, p);
      expect(p.uiState.selectedComp?.internalid, inner.internalid);

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();

      expect(p.uiState.selectedComp?.internalid, p.comp.internalid,
          reason: 'the comp the user came from fronts again');
      expect(find.byKey(ValueKey<String>('tl-tab-${inner.internalid}')),
          findsNothing,
          reason: 'and the tab it had goes with it');
      expect(tester.takeException(), isNull);
    });

    testWidgets('search narrows the outline to matching rows', (tester) async {
      final p = withComp();
      p.comp.addTextLayer();
      p.comp.addCameraLayer();
      await mount(tester, p);

      // Once each: the outline names the layer, and its bar carries no label
      // unless the setting asks for one.
      expect(find.text('Text'), findsOneWidget);
      expect(find.text('Camera'), findsOneWidget);

      await tester.enterText(find.byKey(const ValueKey('tl-search')), 'cam');
      await tester.pump();

      expect(find.text('Camera'), findsOneWidget);
      expect(find.text('Text'), findsNothing,
          reason: 'search hides the rows that do not match');
    });

    testWidgets('the parent picker parents a layer and refuses a cycle',
        (tester) async {
      final p = withComp();
      final parent = p.comp.addAdjustmentLayer();
      final child = p.comp.addCameraLayer();
      await mount(tester, p);

      expect(child.getParent(), isNull);
      await tester.tap(
          find.byKey(ValueKey<String>('tl-parent-${child.internallayerId}')));
      await tester.pumpAndSettle();
      // Numbered by place in the composition since item 6.13, so the entry
      // reads "1. Adjustment" rather than the bare name.
      await tester.tap(find.textContaining(parent.getName()).last);
      await tester.pumpAndSettle();

      expect(child.getParent(), parent.internallayerId);

      // Clearing it is a first-class choice, not an error state.
      await tester.tap(
          find.byKey(ValueKey<String>('tl-parent-${child.internallayerId}')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('None').last);
      await tester.pumpAndSettle();
      expect(child.getParent(), isNull);
    });

    testWidgets('Set in and Set out move the work area, Clear removes it',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      expect(p.comp.getWorkArea(), isNull);

      p.uiState.playheadFrame.value = 20;
      await tester.pump();
      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-work-in')));
      await tester.pumpAndSettle();

      var area = p.comp.getWorkArea();
      expect(area, isNotNull);
      expect(p.comp.frameAtTime(time: area!.inPoint), 20);

      p.uiState.playheadFrame.value = 60;
      await tester.pump();
      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-work-out')));
      await tester.pumpAndSettle();

      area = p.comp.getWorkArea();
      expect(p.comp.frameAtTime(time: area!.outPoint), 60);
      expect(p.comp.frameAtTime(time: area.inPoint), 20,
          reason: 'setting the out point leaves the in point alone');

      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-clear-work-area')));
      await tester.pumpAndSettle();
      expect(p.comp.getWorkArea(), isNull);
    });

    /// **Dragging an edge cannot leave the comp.** A pointer past either end
    /// gave a frame outside it, and a negative in point took the render worker
    /// down: cast unsigned for the cache fill it became a first frame of
    /// eighteen quintillion, `clamp` panicked on the crossed bounds, and every
    /// later frame request came back a send error. The helper the drag commits
    /// through clamps, so the handle stops at the edge.
    testWidgets('a work-area edge dragged past the comp stops at its end',
        (tester) async {
      final p = withComp();
      await mount(tester, p);
      final frames = p.comp.durationFrames();

      // Well past the end, then well before the start.
      p.comp.setWorkArea(
        span: workAreaWith(
          comp: p.comp,
          current: null,
          wanted: frames + 500,
          isStart: false,
        ),
      );
      expect(p.comp.frameAtTime(time: p.comp.getWorkArea()!.outPoint), frames,
          reason: 'the out point stops at the end of the comp');

      p.comp.setWorkArea(
        span: workAreaWith(
          comp: p.comp,
          current: p.comp.getWorkArea(),
          wanted: -500,
          isStart: true,
        ),
      );
      final area = p.comp.getWorkArea()!;
      expect(p.comp.frameAtTime(time: area.inPoint), 0,
          reason: 'and the in point at frame zero');
      expect(p.comp.frameAtTime(time: area.outPoint),
          greaterThan(p.comp.frameAtTime(time: area.inPoint)));
    });

    /// Scrubbing during playback used to be unwinnable: the engine handed back
    /// a frame every tick and each one put the playhead straight back where the
    /// transport wanted it. Taking hold of the playhead takes it off the
    /// transport, and it stays where the drag left it — the
    /// return-to-start of a normal stop would undo the very gesture.
    testWidgets('dragging the ruler during playback stops it and holds',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      p.uiState.play();
      await tester.pump();
      expect(p.uiState.playing.value, isTrue);

      await tester.drag(
          find.byKey(const ValueKey('tl-ruler')), const Offset(120, 0));
      await tester.pumpAndSettle();

      expect(p.uiState.playing.value, isFalse,
          reason: 'taking hold of the playhead stops the transport');
      expect(p.uiState.playheadFrame.value, greaterThan(0),
          reason: 'and it stays where the drag left it, not back at the start');
    });

    /// `Ctrl` held over the ruler sounds each frame the playhead lands on.
    /// Nothing here can listen, so this holds the other half: it is still the
    /// same scrub, and asking to hear a comp with no sound in it is not an
    /// error that stops the drag.
    testWidgets('a Ctrl-drag on the ruler scrubs like a plain one',
        (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      await mount(tester, p);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.drag(
          find.byKey(const ValueKey('tl-ruler')), const Offset(120, 0));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();

      expect(p.uiState.playheadFrame.value, greaterThan(0));
    });

    /// Markers on the ruler are direct manipulation now: a flag can be
    /// dragged to another moment, and its text changed from its own menu. The
    /// dialogue in the ⋯ menu is still there for adding one by hand.
    testWidgets('a marker flag drags along the ruler', (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      addMarkerFrb(p.comp, frame: 10, label: 'Chorus');
      await mount(tester, p);

      final id = p.comp.getMarkers().single.id;
      final flag = find.byKey(ValueKey<String>('tl-marker-$id'));
      expect(flag, findsOneWidget, reason: 'the marker draws on the ruler');

      await tester.drag(flag, const Offset(80, 0));
      await tester.pumpAndSettle();

      final moved = p.comp.getMarkers().single;
      expect(p.comp.frameAtTime(time: moved.time), greaterThan(10),
          reason: 'the drag moved the marker later in the comp');
      expect(moved.label, 'Chorus', reason: 'and left what it says alone');
      expect(moved.id, id, reason: 'it is the same marker, not a new one');
    });

    testWidgets('a marker can be deleted from its own menu', (tester) async {
      final p = withComp();
      p.comp.addAdjustmentLayer();
      addMarkerFrb(p.comp, frame: 10, label: 'Chorus');
      await mount(tester, p);

      await tester.tap(
          find.byKey(
              ValueKey<String>('tl-marker-${p.comp.getMarkers().single.id}')),
          buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('marker-menu-delete')));
      await tester.pumpAndSettle();

      expect(p.comp.getMarkers(), isEmpty);
    });

    /// Markers do not stack. Two flags on one frame are two things to click and
    /// one place, and the second hides the first exactly — so the newcomer wins,
    /// whether it arrives by shortcut or by being dragged on top.
    test('a marker added to an occupied frame replaces what is there', () {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      addMarkerFrb(comp, frame: 20, label: 'Chorus');
      addMarkerFrb(comp, frame: 20, label: 'Drop');
      expect(comp.getMarkers(), hasLength(1));
      expect(comp.getMarkers().single.label, 'Drop');
    });

    // --- layer markers ---------------------------------------------------

    // --- the sequence view -----------------------------------------------

    /// A Sequence layer, ready to open. Added layers land at the top of the
    /// stack, so it is always the first — which lets a test put something
    /// underneath it first.
    Future<LayerReference> sequencedLayer(dynamic p) async {
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.comp.addFootageLayer(footage: footage, asSequence: false);
      p.comp.getLayers().first.convertToSequenced();
      return p.comp.getLayers().first as LayerReference;
    }

    testWidgets('double-clicking a Sequence layer opens its clips in its row',
        (tester) async {
      final p = withComp();
      final layer = await sequencedLayer(p);
      await mount(tester, p);
      await tester.pump();

      final clip = layer.getClips().single;
      expect(find.byKey(ValueKey<String>('seq-clip-${clip.id}')), findsNothing,
          reason: 'shut until it is opened');

      final name =
          find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();

      expect(
          find.byKey(ValueKey<String>('seq-clip-${clip.id}')), findsOneWidget,
          reason: 'the clip is on screen');
      expect(find.byKey(const ValueKey('seq-envelope')), findsOneWidget,
          reason: 'and so is the speed envelope beneath it');

      // Double-clicking again shuts it.
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();
      expect(find.byKey(ValueKey<String>('seq-clip-${clip.id}')), findsNothing);
    });

    testWidgets('a drag keeps going once the readout appears', (tester) async {
      final p = withComp();
      final layer = await sequencedLayer(p);
      await mount(tester, p);
      await tester.pump();
      final name =
          find.byKey(ValueKey<String>('tl-name-${layer.internallayerId}'));
      await tester.tap(name);
      await tester.pump(kDoubleTapMinTime);
      await tester.tap(name);
      await tester.pumpAndSettle();

      final before = layer.getClips().single;
      final strip = tester.getRect(find.byKey(const ValueKey('seq-envelope')));
      final clipBox =
          tester.getRect(find.byKey(ValueKey<String>('seq-clip-${before.id}')));
      final from = Offset(clipBox.center.dx, strip.top + strip.height * 0.3);

      // One event at a time, because a single synthetic move never
      // reproduces a drag that dies part way (the same reason an earlier
      // round of tests all passed).
      final gesture = await tester.startGesture(from);
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(0, 8));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final after = layer.getClips().single;
      expect(after.speedPercent, isNotNull);
      // The whole 48px of travel, not the first step of it.
      final oneStepOnly = 100 - (8 / strip.height) * 161;
      expect(after.speedPercent!, lessThan(oneStepOnly - 20),
          reason: 'every move counted, not just the one before the readout '
              'appeared');
    });

    testWidgets('the razor cuts a sequence clip where it is clicked',
        (tester) async {
      final p = withComp();
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = p.comp.getLayers().single;
      layer.convertToSequenced();
      final sequenced = p.comp.getLayers().single;
      expect(sequenced.getClips(), hasLength(1));

      await mount(tester, p);
      await tester.pump();

      // Unarmed, a click on the bar does not cut.
      final bar = find
          .byKey(ValueKey<String>('tl-bar-body-${sequenced.internallayerId}'));
      expect(bar, findsOneWidget);
      final box = tester.getRect(bar);
      // **The middle of the clip, worked out in frames.** A Sequence layer's
      // own span is the comp's, but the clip inside it is only as long as its
      // (unreadable) media makes it, so a point a third of the way along the
      // *bar* can be past the end of the clip — where there is nothing to cut.
      // This used to be a flat `left + 8`, which is a pixel count standing in
      // for a frame: the day the outline narrowed by one column the lane grew
      // by the same amount, those 8 pixels bought fewer frames, and the razor
      // landed on the clip's first frame, where a cut is a no-op. Frames do not
      // move when a column does.
      final clip = sequenced.getClips().single;
      final middle = (clip.startFrame.toInt() + clip.endFrame.toInt()) / 2;
      final inside = Offset(
        box.left + box.width * middle / p.comp.durationFrames(),
        box.center.dy,
      );
      await tester.tapAt(inside);
      await tester.pump();
      expect(p.comp.getLayers().single.getClips(), hasLength(1),
          reason: 'the razor is a mode, not the default click');

      // The Timeline's menu item arms the toolbar's Razor tool —
      // one razor, two doors.
      await openMore(tester);
      await tester.tap(find.byKey(const ValueKey('tl-razor')));
      await tester.pumpAndSettle();
      expect(p.uiState.tools.tool, ToolMode.razor);

      await tester.tapAt(inside);
      await tester.pumpAndSettle();

      expect(p.comp.getLayers().single.getClips(), hasLength(2),
          reason: 'the armed razor cut the clip under the pointer');
    });

    // -----------------------------------------------------------------------
    // 6.43 — the Animated filter.
    // -----------------------------------------------------------------------

    /// **Animated lists what is keyed, All brings the twirls back.** The filter
    /// reaches past the twirl set entirely: a layer that has never been opened
    /// shows its keyed rows the moment the filter is on, and a keyed row's
    /// headings come with it while the ones with nothing under them go.
    testWidgets('the Animated filter lists only the rows that carry keys',
        (tester) async {
      final p = withComp();
      final layer = p.comp.addSolidLayer();
      layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([
          for (final f in [10, 40])
            BridgeKeyframe(
              time: p.comp.timeOfFrame(frame: f),
              value: f.toDouble(),
              interpIn: const BridgeSideInterp.linear(),
              interpOut: const BridgeSideInterp.linear(),
            ),
        ]),
      );
      p.uiState.model.refresh();
      await mount(tester, p);

      final id = layer.internallayerId;
      Finder row(String path) =>
          find.byKey(ValueKey<String>('tl-keys-prop-$id/$path'));

      // Shut by default, so nothing of the fold-out is drawn at all.
      expect(row('transform'), findsNothing);

      final filter = find.byKey(const ValueKey('tl-filter-animated'));
      expect(filter, findsOneWidget);
      await tester.tap(filter);
      await tester.pumpAndSettle();

      expect(row('transform'), findsOneWidget,
          reason: 'the heading that leads to the keyed row came with it');
      expect(row('transform/opacity'), findsOneWidget);
      expect(row('transform/position'), findsNothing,
          reason: 'a transform row with nothing keyed is not listed');

      // All: the twirl set is back in charge, and it says shut.
      await tester.tap(filter);
      await tester.pumpAndSettle();
      expect(row('transform'), findsNothing);
      expect(row('transform/opacity'), findsNothing);
    });

  }, skip: !engineAvailable);
}

