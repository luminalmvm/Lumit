// The Viewer on frb, against the real engine.
//
// The picture itself is not asserted here — what the worker publishes is a
// platform texture or a decoded frame, and neither arrives in a widget test.
// What is asserted is everything around it: the transport, the timecode, the
// magnification and channel pickers, the grid, and the move gizmo, all of which
// are the parts a user actually operates.
//
// Seven of them do still need a frame to *arrive*, because that arrival is what
// moves the playhead and bumps `frameArrived` — the engine drives playback,
// so a Viewer that is told nothing shows nothing and counts nothing.
// Those carry `skip: zeroCopyViewerUnavailable`, which is true only on a machine
// with no working zero-copy transport (see `frb_test_support.dart`). Today that
// means the Linux CI runner and its software Vulkan, so on CI these seven do not
// run at all. They are among the tests most worth having; the skip is a
// statement about the runner, not about them.
//
// Everywhere they wait for a first picture they wait with `coldWorkerRounds`,
// because a fresh project's worker builds its renderer before it reads a
// request and that is seconds on a machine with no warm shader cache. The
// waiting is what grew; every assertion is the one it always was.

import 'dart:io';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/panels/transform_rows_frb.dart' show writeScalar;
import 'package:lumit_flutter/panels/viewer_gizmo.dart';
import 'package:lumit_flutter/panels/viewer_layer_map.dart';
import 'package:lumit_flutter/panels/viewer_overlays.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_paint.dart';
import 'package:lumit_flutter/panels/viewer_rulers.dart';
import 'package:lumit_flutter/panels/viewer_tool_cursor.dart'
    show DrawnPointerRegion;
import 'package:lumit_flutter/panels/viewer_zoom.dart';
import 'package:lumit_flutter/state/dropper.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/state/viewer_view.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/widgets/dropper_overlay.dart';
import 'package:uuid/uuid.dart';

import 'frb_test_support.dart';

/// A dropper window big enough to answer for anywhere on the picture, so a drag
/// test can sweep the pointer without the read-back the engine would otherwise
/// have to do. The pixels ramp left to right, which is what makes a preview
/// that followed the pointer distinguishable from one that did not.
BridgeSampledPixels wholePicture({int width = 100, int height = 50}) {
  const side = dropperWindow;
  final centreX = width ~/ 2, centreY = height ~/ 2;
  final bytes = Uint8List(side * side * 4);
  const half = side ~/ 2;
  for (var row = 0; row < side; row++) {
    for (var col = 0; col < side; col++) {
      final x = centreX - half + col;
      final i = (row * side + col) * 4;
      bytes[i] = (x * 2).clamp(0, 255);
      bytes[i + 3] = 255;
    }
  }
  return BridgeSampledPixels(
    window: side,
    rgba: bytes,
    width: width,
    height: height,
    x: centreX,
    y: centreY,
    frame: BigInt.zero,
    layerAlone: false,
  );
}

void main() {
  setUpAll(initEngineForTests);

  group('Viewer (frb)', () {
    ({
      LumitState state,
      LumitUiState uiState,
      CompositionReference comp,
      LayerReference layer,
    }) withLayer() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addAdjustmentLayer();
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      return (state: p.state, uiState: p.uiState, comp: comp, layer: layer);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(700, 500),
      ));
      await tester.pump();
    }

    /// Press a control on the Viewer bar, scrolling it into view first.
    ///
    /// **The bar scrolls when the panel is narrower than it wants** (docs/07
    /// §2.2), and this Viewer is 700 px — narrower than the bar has wanted
    /// since the clock went in front of the transport, and narrower again
    /// since the guides menu and the snapshot pair arrived. A tap on a
    /// control that has scrolled off the end lands on nothing and reads as a
    /// transport that does not work, which is exactly how it read the first
    /// time. Anyone on a narrow dock scrolls first too.
    Future<void> pressBar(WidgetTester tester, String key) async {
      final button = find.byKey(ValueKey<String>(key));
      await tester.ensureVisible(button);
      await tester.pump();
      await tester.tap(button);
      await tester.pump();
    }

    /// Open one of the header's three pickers and choose the row [key].
    Future<void> pickHeaderRow(
        WidgetTester tester, String picker, String key) async {
      await pressBar(tester, picker);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(ValueKey<String>(key)));
      await tester.pumpAndSettle();
    }

    /// Open the bottom bar's view menu and choose the row [key].
    Future<void> pickViewRow(WidgetTester tester, String key) async {
      await pressBar(tester, 'viewer-guides-menu');
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(ValueKey<String>(key)));
      await tester.pumpAndSettle();
    }

    /// Turn the tone map on or off, which is a row in the header's
    /// colour-pipeline menu rather than a button on the bar.
    Future<void> flipToneMap(WidgetTester tester) =>
        pickHeaderRow(tester, 'viewer-colour', 'viewer-tone-map');

    /// **The dropper's magnifier belongs to the pointer being over the
    /// picture.** Two things it used to get wrong: it appeared the instant the
    /// tool was armed, sitting where the *previous* pick had left the pointer,
    /// and it stayed on once the pointer had gone.
    testWidgets(
        'the magnifier appears only while the pointer is on the picture',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      DropperArm arm() => DropperArm(
            id: 'test',
            reads: DropperReads.colour,
            label: 'Key colour',
            onPick: (_) {},
          );

      p.uiState.armDropper(arm());
      await tester.pump();
      expect(find.byType(DropperViewfinder), findsNothing,
          reason: 'armed, but the pointer has not been near the picture');

      final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await gesture.addPointer(location: Offset.zero);
      addTearDown(gesture.removePointer);

      final stage = find.byType(DropperLayer);
      await gesture.moveTo(tester.getCenter(stage));
      await tester.pump();
      expect(find.byType(DropperViewfinder), findsOneWidget,
          reason: 'the pointer is on the picture');

      // The pasteboard around the picture is not the picture: a 16:9 comp in
      // this panel leaves a band top and bottom.
      await gesture.moveTo(tester.getTopLeft(stage) + const Offset(4, 4));
      await tester.pump();
      expect(find.byType(DropperViewfinder), findsNothing,
          reason: 'off the picture, there is nothing to magnify');

      // Back on, then disarmed and armed again: the new arm must not inherit
      // the last one's pointer position.
      await gesture.moveTo(tester.getCenter(stage));
      await tester.pump();
      expect(find.byType(DropperViewfinder), findsOneWidget);

      p.uiState.disarmDropper();
      await tester.pump();
      expect(find.byType(DropperViewfinder), findsNothing);

      p.uiState.armDropper(arm());
      await tester.pump();
      expect(find.byType(DropperViewfinder), findsNothing,
          reason: 'a fresh arm starts with the pointer nowhere');
    });

    /// **The scroll crash.** Scrolling over the Viewer with the dropper armed
    /// zooms the picture, which relays the panel out under the magnifier. The
    /// magnifier is in the application's overlay, so working out where to put
    /// it from render objects *while that rebuild is happening* asserts
    /// `attached` and takes the whole window red. Its position is worked out
    /// when the pointer moves instead, and used as a plain number afterwards.
    testWidgets('scrolling with the dropper armed does not throw',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      p.uiState.armDropper(DropperArm(
        id: 'test',
        reads: DropperReads.colour,
        label: 'Key colour',
        onPick: (_) {},
      ));
      await tester.pump();

      final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await gesture.addPointer(location: Offset.zero);
      addTearDown(gesture.removePointer);
      final centre = tester.getCenter(find.byType(DropperLayer));
      await gesture.moveTo(centre);
      await tester.pump();
      expect(find.byType(DropperViewfinder), findsOneWidget);

      // An ordinary wheel scroll: the Viewer zooms about the pointer.
      await tester.sendEventToBinding(
        PointerScrollEvent(position: centre, scrollDelta: const Offset(0, -60)),
      );
      await tester.pump();
      expect(tester.takeException(), isNull,
          reason: 'zooming under it is fine');

      // And again the other way, with the magnifier still up.
      await tester.sendEventToBinding(
        PointerScrollEvent(position: centre, scrollDelta: const Offset(0, 120)),
      );
      await tester.pump();
      expect(tester.takeException(), isNull);
      expect(find.byType(DropperViewfinder), findsOneWidget,
          reason: 'and it is still following the pointer');
    });

    /// Where the picture is drawn, as the panel hands it to the stage.
    Rect drawnPicture(WidgetTester tester) =>
        tester.widget<ViewerStage>(find.byType(ViewerStage)).fitted;

    /// A drag across the stage on whichever button is asked for.
    Future<void> dragStage(WidgetTester tester, Offset by,
        {required int buttons}) async {
      final from =
          tester.getCenter(find.byKey(const ValueKey('viewer-stage')));
      final pointer = TestPointer(3, PointerDeviceKind.mouse, null, buttons);
      await tester.sendEventToBinding(pointer.down(from));
      await tester.pump();
      for (var i = 1; i <= 4; i++) {
        await tester.sendEventToBinding(pointer.move(from + by * (i / 4)));
        await tester.pump();
      }
      await tester.sendEventToBinding(pointer.up());
      await tester.pump();
    }

    /// **The middle button pans the picture** (docs/07 §2.2), as it does in
    /// After Effects, Blender and Resolve. Whatever tool is armed, because it
    /// is read off the pointer rather than won in the gesture arena.
    testWidgets('a middle-button drag pans the picture', (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final before = drawnPicture(tester);
      await dragStage(tester, const Offset(40, -30),
          buttons: kMiddleMouseButton);
      final after = drawnPicture(tester);

      expect(after.left, closeTo(before.left + 40, 1));
      expect(after.top, closeTo(before.top - 30, 1));
      expect(after.size, before.size,
          reason: 'a pan moves the picture, it does not resize it');
    });

    /// An armed picker owns the picture: it holds still while pixels are being
    /// read off it, so the pan stands down until the tool is put away.
    testWidgets('a middle-button drag does not pan under an armed picker',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      p.uiState.armDropper(DropperArm(
        id: 'test',
        reads: DropperReads.colour,
        label: 'Key colour',
        onPick: (_) {},
      ));
      await tester.pump();

      final before = drawnPicture(tester);
      await dragStage(tester, const Offset(40, -30),
          buttons: kMiddleMouseButton);
      expect(drawnPicture(tester), before);
    });

    /// **The magnifier is on screen for the whole pick, and it shows what the
    /// release will commit** (docs/07 §6.1). The owner reported it missing
    /// after the redesign; nothing had been taken out of it, but a pick drag
    /// panned the picture out from under the pointer while every window read
    /// cost a fresh composite, so the grid a pick was aimed with was a grid of
    /// empty cells. This pins the part the arithmetic can promise: the centre
    /// cell of the grid, at the region on show, IS the committed value.
    testWidgets('the magnifier follows a pick drag and shows what it commits',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final picked = <DropperSample>[];
      p.uiState.armDropper(DropperArm(
        id: 'test',
        reads: DropperReads.colour,
        label: 'Key colour',
        onPick: picked.add,
      ));
      p.uiState.dropperPatch.value = wholePicture();
      await tester.pump();

      final centre = tester.getCenter(find.byType(DropperLayer));
      final gesture = await tester.startGesture(centre);
      await tester.pump();
      expect(find.byType(DropperViewfinder), findsOneWidget,
          reason: 'the grid is up while the pick is being made');

      await gesture.moveTo(centre + const Offset(30, 0));
      await tester.pump();
      final shown =
          tester.widget<DropperViewfinder>(find.byType(DropperViewfinder));
      expect(find.byType(DropperViewfinder), findsOneWidget,
          reason: 'and it followed the drag rather than being left behind');

      // What the grid is drawing at its centre, worked out the way the grid
      // itself works it out — from the window it holds, at the region it shows.
      final atCentre = sampleFromWindow(
          shown.window!, shown.region, shown.centre.$1, shown.centre.$2);

      await gesture.up();
      await tester.pump();

      expect(picked.length, 1);
      expect(picked.single.r, closeTo(atCentre.r, 1e-9),
          reason: 'the committed colour is the one under the centre cell');
      expect(picked.single.region, atCentre.region);

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });

    /// **Shift+scroll sizes the sample, and nothing else** (docs/07 §6.1):
    /// 1×1 → 3×3 → 5×5 → 7×7 → 9×9 and back, holding at both ends rather than
    /// wrapping, never zooming the picture out from under the pixel being
    /// aimed at, and never costing the engine a thing — the window in hand
    /// already holds every pixel a wider region could want.
    testWidgets('Shift+scroll steps the sampled region under the magnifier',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final picked = <DropperSample>[];
      p.uiState.armDropper(DropperArm(
        id: 'test',
        reads: DropperReads.colour,
        label: 'Key colour',
        onPick: picked.add,
      ));
      p.uiState.dropperPatch.value = wholePicture();
      await tester.pump();

      Rect picture() =>
          tester.widget<DropperLayer>(find.byType(DropperLayer)).fitted;
      int region() => tester
          .widget<DropperViewfinder>(find.byType(DropperViewfinder))
          .region;

      final gesture = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await gesture.addPointer(location: Offset.zero);
      addTearDown(gesture.removePointer);
      final centre = tester.getCenter(find.byType(DropperLayer));
      await gesture.moveTo(centre);
      await tester.pump();
      expect(region(), 1, reason: 'this pixel and no other, to start with');

      final unzoomed = picture();
      Future<void> notch(double dy) async {
        await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
        await tester.sendEventToBinding(
          PointerScrollEvent(position: centre, scrollDelta: Offset(0, dy)),
        );
        await tester.pump();
        await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      }

      await notch(-60);
      expect(region(), 3);
      await notch(-60);
      expect(region(), 5);
      expect(picture(), unzoomed,
          reason: 'sizing the sample must not zoom the picture');

      await notch(60);
      expect(region(), 3, reason: 'and the other way steps back down');

      // The ends hold: a size settled on is not lost to one extra notch.
      for (var i = 0; i < 6; i++) {
        await notch(60);
      }
      expect(region(), 1);
      for (var i = 0; i < 8; i++) {
        await notch(-60);
      }
      expect(region(), dropperGrid,
          reason: 'the region can never exceed the grid it is drawn in');

      // And the size on show is the size the pick takes.
      await tester.tapAt(centre);
      await tester.pump();
      expect(picked.single.region, dropperGrid);

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });

    /// **A pick is a drag** (docs/07 §6.1). The press writes nothing; it
    /// starts a gesture that stages the sample under the pointer and previews
    /// it, and the release commits **once** — the value where the pointer let
    /// go, not the value where it went down. That is the finding: arming a
    /// position picker and pressing wrote the position immediately, so the
    /// drag that followed moved only the magnifier.
    ///
    /// The window is put in by hand: what the engine reads back is a real
    /// round trip, and none of the arithmetic under test needs one.
    testWidgets('a pick drag previews as it goes and commits once on release',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final picked = <DropperSample>[];
      final previewed = <DropperSample>[];
      var reverts = 0;
      p.uiState.armDropper(DropperArm(
        id: 'test',
        reads: DropperReads.colour,
        label: 'Key colour',
        onPick: picked.add,
        onPreview: previewed.add,
        onRevert: () => reverts += 1,
      ));
      p.uiState.dropperPatch.value = wholePicture();
      await tester.pump();

      final stage = find.byType(DropperLayer);
      final centre = tester.getCenter(stage);
      final gesture = await tester.startGesture(centre);
      await tester.pump();
      expect(picked, isEmpty,
          reason: 'the press stages the value, it does not write it');

      // A sweep right, in steps, each one past the preview interval so the
      // throttle lets it out rather than coalescing the lot into one.
      for (var step = 1; step <= 4; step++) {
        await gesture.moveTo(centre + Offset(step * 12.0, 0));
        await tester.pump(const Duration(milliseconds: 25));
      }
      expect(previewed.length, greaterThan(1),
          reason: 'the drag previewed as it went');
      expect(previewed.last.xFrac, greaterThan(previewed.first.xFrac),
          reason: 'and the preview followed the pointer across the picture');
      expect(picked, isEmpty, reason: 'still nothing committed mid-drag');

      await gesture.up();
      await tester.pump();

      expect(picked.length, 1, reason: 'one commit for the whole gesture');
      expect(picked.single.xFrac, closeTo(previewed.last.xFrac, 1e-9),
          reason: 'and it is the value the pointer let go on');
      expect(reverts, 0);
      expect(p.uiState.dropper.value, isNull,
          reason: 'the tool put itself away');

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });

    /// **An armed pick takes the drag off the pan** (docs/07 §6.1).
    ///
    /// The finding: picking a colour dragged the preview about with it. The
    /// dropper reads raw pointer events, and a `Listener` never joins the
    /// gesture arena, so the Viewer's own pan recogniser went on winning it
    /// underneath the pick. Both legs are asserted, because the fix is an
    /// arbitration and not a deletion: unarmed, the drag still pans.
    testWidgets('a pick drag samples without panning the picture',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);
      // The tool whose whole job over the picture is the drag this is about.
      p.uiState.tools.select(ToolMode.hand);
      await tester.pump();

      // Where the picture sits at the current magnification and pan — the
      // Viewer's transform, said in the one number the dropper cares about.
      Rect picture() =>
          tester.widget<DropperLayer>(find.byType(DropperLayer)).fitted;
      final stage = find.byKey(const ValueKey('viewer-stage'));

      final unarmed = picture();
      await tester.drag(stage, const Offset(40, 24));
      await tester.pump();
      final panned = picture();
      expect(panned.topLeft, isNot(unarmed.topLeft),
          reason: 'nothing armed: a drag over the Viewer still pans');

      final picked = <DropperSample>[];
      p.uiState.armDropper(DropperArm(
        id: 'test',
        reads: DropperReads.colour,
        label: 'Key colour',
        onPick: picked.add,
        onPreview: (_) {},
      ));
      p.uiState.dropperPatch.value = wholePicture();
      await tester.pump();

      final from = tester.getTopLeft(stage) + panned.center;
      final gesture = await tester.startGesture(from);
      for (var step = 1; step <= 4; step++) {
        await gesture.moveTo(from + Offset(step * 12.0, 0));
        await tester.pump(const Duration(milliseconds: 25));
      }
      expect(picture(), panned,
          reason: 'the picking drag left the preview where it was');

      await gesture.up();
      await tester.pump();
      expect(picked.length, 1, reason: 'and it was a pick, not a pan');
      expect(picture(), panned, reason: 'still where it was after the commit');

      // Disarmed by the commit: the pan comes back.
      await tester.drag(stage, const Offset(-40, -24));
      await tester.pump();
      expect(picture().topLeft, isNot(panned.topLeft),
          reason: 'the pick is over, so the drag is the pan\'s again');

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });

    /// **Escape mid-drag puts back what was staged** — the convention every
    /// staged gesture in the application keeps. Nothing was committed, so the
    /// revert has only the preview to undo, and no pick may be written.
    testWidgets('Escape during a pick drag reverts and writes nothing',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final picked = <DropperSample>[];
      var reverts = 0;
      p.uiState.armDropper(DropperArm(
        id: 'test',
        reads: DropperReads.colour,
        label: 'Key colour',
        onPick: picked.add,
        onPreview: (_) {},
        onRevert: () => reverts += 1,
      ));
      p.uiState.dropperPatch.value = wholePicture();
      await tester.pump();

      final centre = tester.getCenter(find.byType(DropperLayer));
      final gesture = await tester.startGesture(centre);
      await tester.pump();
      await gesture.moveTo(centre + const Offset(40, 0));
      await tester.pump(const Duration(milliseconds: 25));

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();

      expect(reverts, 1, reason: 'what the drag was showing is put back');
      expect(picked, isEmpty, reason: 'nothing was ever committed');
      expect(p.uiState.dropper.value, isNull);

      // And the release that follows the Escape must not resurrect the pick.
      await gesture.up();
      await tester.pump();
      expect(picked, isEmpty);

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });

    testWidgets('without a composition the empty stage offers the ways in',
        (tester) async {
      // A project with no comps at all shows the welcome's three
      // start cards in the stage; the "select a composition" sentence is
      // for a project that has comps with none fronted (EmptyStageFrb
      // decides between the two).
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      expect(find.text('New project'), findsOneWidget);
      expect(find.text('Open'), findsOneWidget);
      expect(find.textContaining('Select a composition'), findsNothing);
    });

    testWidgets('the transport steps, homes and ends within the comp',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);
      final last = p.comp.durationFrames() - 1;

      await pressBar(tester, 'viewer-step-forward');
      expect(p.uiState.playheadFrame.value, 1);

      await pressBar(tester, 'viewer-step-back');
      expect(p.uiState.playheadFrame.value, 0);

      // Stepping back from the start stays at the start rather than going
      // negative — a frame before the comp is not a frame.
      await pressBar(tester, 'viewer-step-back');
      expect(p.uiState.playheadFrame.value, 0);

      await pressBar(tester, 'viewer-end');
      expect(p.uiState.playheadFrame.value, last);

      await pressBar(tester, 'viewer-step-forward');
      expect(p.uiState.playheadFrame.value, last,
          reason: 'and the end is the end');

      await pressBar(tester, 'viewer-home');
      expect(p.uiState.playheadFrame.value, 0);
    });

    /// **Playback runs in the engine.** Note what this test does *not*
    /// do: elapse any fake time. `settleFrb` gives real event-loop turns and
    /// deliberately advances no `FakeAsync` clock, so a Flutter `Ticker` would
    /// never fire during it. The playhead moves here purely because the engine
    /// chose frames and each arriving frame said which one it was — which is the
    /// whole point of the move, and would fail if a clock crept back into Dart.
    testWidgets('play advances the playhead, and stopping returns it',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      await pressBar(tester, 'viewer-play');
      await tester.pump();
      expect(p.uiState.playing.value, isTrue);
      await settleFrb(tester,
          minRounds: 6,
          maxRounds: coldWorkerRounds,
          until: () => p.uiState.playheadFrame.value > 0);
      expect(p.uiState.playheadFrame.value, greaterThan(0),
          reason: 'the engine chose frames and the playhead followed them');

      await pressBar(tester, 'viewer-play');
      await tester.pump();
      expect(p.uiState.playing.value, isFalse);
      await settleFrb(tester, minRounds: 12, maxRounds: 12);
      // The playhead goes back to where play was asked for. Playback is
      // a preview of the moment being worked on, so stopping returns you to it
      // rather than leaving you wherever the picture happened to stop. In-flight
      // frames are included — a late arrival must not drag it off again.
      expect(p.uiState.playheadFrame.value, 0,
          reason: 'stopping puts the playhead back where play started');

      // Degradation is stated by the reading rather than by a badge that
      // comes and goes: the tier a frame was made at is the pixel
      // count in "1920×1080 → 960×540", so the bar never changes shape
      // mid-playback. The while-playing half is not asserted — it races a
      // live controller.
      expect(find.byKey(const ValueKey('viewer-readout')), findsOneWidget,
          reason: 'the reading is always there, whatever the tier');
    }, skip: zeroCopyViewerUnavailable);

    /// The other half of the returning playhead: Settings ▸ Interface ▸
    /// Editing puts the old After Effects behaviour back, and then stopping
    /// leaves the playhead on the frame that was on screen.
    testWidgets('the playhead stays put when the setting asks it to',
        (tester) async {
      final p = withLayer();
      p.uiState.workspace.interface.playheadStaysOnStop = true;
      await mount(tester, p);

      await pressBar(tester, 'viewer-play');
      await tester.pump();
      await settleFrb(tester,
          minRounds: 6,
          maxRounds: coldWorkerRounds,
          until: () => p.uiState.playheadFrame.value > 0);

      await pressBar(tester, 'viewer-play');
      await tester.pump();
      await settleFrb(tester, minRounds: 12, maxRounds: 12);
      expect(p.uiState.playheadFrame.value, greaterThan(0),
          reason: 'the setting keeps the playhead where the picture stopped');
    }, skip: zeroCopyViewerUnavailable);

    /// Running off the end is the engine's to notice: it knows the length and it
    /// is the one counting. The frontend is *told*, and that is the only reason
    /// its transport goes back to showing a play button.
    testWidgets('playback ends on its own at the end of the composition',
        (tester) async {
      final p = withLayer();
      // A tenth of a second, so the end arrives inside a test rather than in the
      // thirty seconds a default comp lasts.
      final was = p.comp.getSettings();
      p.comp.setSettings(
        settings: BridgeCompSettings(
          name: was.name,
          width: 160,
          height: 90,
          fpsNum: was.fpsNum,
          fpsDen: was.fpsDen,
          background: was.background,
          shutterAngle: was.shutterAngle,
          motionBlurSamples: was.motionBlurSamples,
          duration: const BridgeRational(num: 1, den: 10),
        ),
      );
      await mount(tester, p);
      expect(p.comp.durationFrames(), 6, reason: '0.1 s at 60 fps');

      await pressBar(tester, 'viewer-play');
      await tester.pump();
      // Six frames of a software render under a loaded parallel suite can
      // outlast the old four-second ceiling; the wait grows, the assertion
      // does not - the engine must still end the run entirely on its own.
      await settleFrb(tester,
          minRounds: 6, maxRounds: 600, until: () => !p.uiState.playing.value);

      expect(p.uiState.playing.value, isFalse,
          reason: 'the engine said it ended; nothing in Dart worked it out');
    });

    testWidgets('the timecode reads HH:MM:SS:FF at the comp rate',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      expect(find.text('00:00:00:00'), findsOneWidget);

      // A new comp is 60 fps, so frame 90 is one and a half seconds in.
      p.uiState.playheadFrame.value = 90;
      await tester.pump();
      expect(find.text('00:00:01:30'), findsOneWidget);
    });

    /// 29.97 counts thirty frames to the second of timecode, which is what every
    /// editor shows — the last frame of a second is :29, not an impossible :28.
    testWidgets('a drop-frame rate still counts a whole second of frames',
        (tester) async {
      final p = withLayer();
      final settings = p.comp.getSettings();
      p.comp.setSettings(
        settings: BridgeCompSettings(
          name: settings.name,
          width: settings.width,
          height: settings.height,
          fpsNum: 30000,
          fpsDen: 1001,
          duration: settings.duration,
          background: settings.background,
          shutterAngle: settings.shutterAngle,
          motionBlurSamples: settings.motionBlurSamples,
        ),
      );
      await mount(tester, p);

      p.uiState.playheadFrame.value = 29;
      await tester.pump();
      expect(find.text('00:00:00:29'), findsOneWidget);

      p.uiState.playheadFrame.value = 30;
      await tester.pump();
      expect(find.text('00:00:01:00'), findsOneWidget);
    });

    testWidgets('the magnification, channel and grid controls are live',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      await tester.tap(find.byKey(const ValueKey('viewer-zoom')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('100%').last);
      await tester.pumpAndSettle();
      expect(find.text('100%'), findsOneWidget,
          reason: 'the picker shows what was chosen');

      await pressBar(tester, 'viewer-channel');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Alpha').last);
      await tester.pumpAndSettle();
      expect(find.byType(ColorFiltered), findsWidgets,
          reason: 'a single channel is drawn through a filter');

      // The grid is on by default and toggles off.
      expect(find.byKey(const ValueKey('viewer-grid')), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('viewer-grid')));
      await tester.pump();
    });

    /// **The Viewer's two strips carry the drawing's own controls, in its own
    /// order**, in place of the single bar. The arrangement is the decision,
    /// so this is what asserts it: the keys left to right, and nothing about
    /// pixels.
    testWidgets("the Viewer's strips are in the drawing's order",
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      expect(headerKeys(tester), [
        // The header: the magnification, the quality, the colour pipeline.
        'viewer-zoom', 'viewer-resolution', 'viewer-colour',
      ]);

      expect(barKeys(tester), [
        // The ways of looking, then the seam and the snapshot.
        'viewer-grid', 'viewer-guides-menu', 'viewer-channel',
        'viewer-exposure-reset', 'viewer-exposure',
        // The snapshot pair: take, then show.
        'viewer-snapshot', 'viewer-snapshot-show',
        // The transport and its clock.
        'viewer-home', 'viewer-step-back', 'viewer-play',
        'viewer-step-forward', 'viewer-end', 'viewer-timecode',
        // The right-hand end: the reading, which is not a control.
        'viewer-readout',
      ]);

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });

    /// **The grid-and-guides menu draws over the picture and nowhere else**
    /// (docs/07 §2.2 items 5–6). Both entries are checkable, both marks
    /// are painted by the display, and turning the last one off takes the
    /// painter out of the tree rather than leaving it drawing nothing.
    testWidgets('the guides menu turns the grid and the safe areas on',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final overlay = find.byKey(const ValueKey('viewer-overlay-guides'));
      expect(overlay, findsNothing, reason: 'nothing is drawn to begin with');

      Future<void> pick(String entry) async {
        await pressBar(tester, 'viewer-guides-menu');
        await tester.pumpAndSettle();
        await tester.tap(find.byKey(ValueKey<String>(entry)));
        await tester.pumpAndSettle();
      }

      await pick('viewer-guides-grid');
      expect(p.uiState.viewerOverlays.grid, isTrue);
      expect(overlay, findsOneWidget);
      expect(
        tester
            .widget<CustomPaint>(
              find.descendant(of: overlay, matching: find.byType(CustomPaint)),
            )
            .painter,
        isA<ViewerOverlayPainter>()
            .having((x) => x.grid, 'grid', isTrue)
            .having((x) => x.safeAreas, 'safeAreas', isFalse),
      );

      await pick('viewer-guides-safe');
      expect(p.uiState.viewerOverlays,
          (grid: true, safeAreas: true, rulers: false));

      // Off again, one at a time: the painter goes only when the last mark has.
      await pick('viewer-guides-grid');
      expect(overlay, findsOneWidget, reason: 'the safe areas are still on');
      await pick('viewer-guides-safe');
      expect(overlay, findsNothing);
    });

    /// **The rulers stand on the panel, not on the picture** (docs/07
    /// §2.2 item 6): turning them on moves the picture out from under them,
    /// which is what makes a guide dragged out of a strip land on the shot
    /// rather than on a band covering it. And a guide is a mark over one comp,
    /// so it rides that comp's session.
    testWidgets('the rulers inset the picture, and guides come out of them',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final rulers = find.byKey(const ValueKey('viewer-rulers'));
      expect(rulers, findsNothing, reason: 'no rulers, and no guides either');

      await pickViewRow(tester, 'viewer-guides-rulers');
      expect(p.uiState.viewerOverlays.rulers, isTrue);
      expect(rulers, findsOneWidget);
      final painter = tester.widget<CustomPaint>(rulers).painter;
      expect(
        painter,
        isA<ViewerRulerPainter>().having(
          (x) => x.picture.left,
          'the picture starts past the strip',
          greaterThanOrEqualTo(viewerRulerBand),
        ),
      );

      // Out of the top strip and onto the picture: a horizontal guide, kept
      // against this comp.
      final stage = tester.getTopLeft(find.byType(ViewerStage));
      await tester.dragFrom(
          stage + const Offset(200, viewerRulerBand / 2), const Offset(0, 120));
      await tester.pumpAndSettle();
      expect(p.uiState.guides.length, 1);
      expect(p.uiState.guides.single.vertical, isFalse);
      expect(
        p.uiState.session().guides[p.comp.internalid.toString()],
        p.uiState.guides,
        reason: 'a guide is written down with where the user was',
      );

      // And the menu takes them all off again.
      await pickViewRow(tester, 'viewer-guides-clear');
      expect(p.uiState.guides, isEmpty);
    });

    /// **A snapshot is a second picture, and releasing the button is its whole
    /// lifecycle** (docs/07 §2.2 item 14). Nothing here crosses the
    /// bridge: the stage photographs its own [RepaintBoundary], and Show puts
    /// the photograph back over the live picture while it is held.
    /// **Two marks**, not the merged one: Take photographs the picture on a
    /// plain click, and Show beside it puts the photograph back over the live
    /// one while it is held. Show is muted until a photograph exists, which is
    /// what makes a taken snapshot findable at all — the merged mark said
    /// nothing about either.
    testWidgets('Take photographs the picture and Show compares against it',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final take = find.byKey(const ValueKey('viewer-snapshot'));
      final show = find.byKey(const ValueKey('viewer-snapshot-show'));
      final shown = find.byKey(const ValueKey('viewer-snapshot-overlay'));

      // Nothing photographed yet, so Show is deaf rather than flashing the
      // live picture at itself.
      await tester.ensureVisible(show);
      await tester.pump();
      final empty = await tester.startGesture(tester.getCenter(show));
      await tester.pump(const Duration(milliseconds: 300));
      expect(shown, findsNothing);
      await empty.up();
      await tester.pump();

      await pressBar(tester, 'viewer-snapshot');
      // The photograph is taken off the render tree, which is a real async
      // round trip rather than a frame.
      await tester.pumpAndSettle();
      expect(shown, findsNothing, reason: 'taking one does not display it');
      expect(take, findsOneWidget);

      await tester.ensureVisible(show);
      await tester.pump();
      final hold = await tester.startGesture(tester.getCenter(show));
      await tester.pump();
      expect(shown, findsOneWidget,
          reason: 'held down, the picture is swapped');

      await hold.up();
      await tester.pump();
      expect(shown, findsNothing, reason: 'let go, the live picture is back');

      // And Take is still a plain click: holding it must not compare, now that
      // the second mark is what does.
      await tester.ensureVisible(take);
      await tester.pump();
      final again = await tester.startGesture(tester.getCenter(take));
      await tester.pump(const Duration(milliseconds: 300));
      expect(shown, findsNothing, reason: 'holding Take compares nothing');
      await again.up();
      await tester.pumpAndSettle();
    });

    /// **A snapshot never stores more pixels than the panel can show.** The
    /// boundary it is photographed from is the picture's rectangle, which is
    /// the *comp* at this magnification and not the panel: an HD comp at 400 %
    /// is 7680 logical pixels across, and photographing that at the device's
    /// own ratio asks for a few hundred million pixels — on a button with no
    /// warning on it. Uncapped this assertion misses by an order of magnitude
    /// (and the run before it allocates a gigabyte), so the cap is the
    /// regression, not the advice. The bound is the *region* photographed
    /// rather than the resolution it is photographed at, and the photograph
    /// goes back over the part of the picture it came from — which is the
    /// second pair of assertions here. What it keeps of the detail is
    /// pinned in viewer_snapshot_crop_test.dart, where the pixels are readable.
    testWidgets('a snapshot taken at 400 % stays the size of the panel',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      await pressBar(tester, 'viewer-zoom');
      await tester.pumpAndSettle();
      await tester.tap(find.text('400%').last);
      await tester.pumpAndSettle();

      await pressBar(tester, 'viewer-snapshot');
      await tester.pumpAndSettle();

      final mark = find.byKey(const ValueKey('viewer-snapshot-show'));
      await tester.ensureVisible(mark);
      await tester.pump();
      final hold = await tester.startGesture(tester.getCenter(mark));
      await tester.pump();

      final image = tester
          .widget<RawImage>(
              find.byKey(const ValueKey('viewer-snapshot-overlay')))
          .image!;
      final panel = tester.getSize(find.byType(ViewerPanelFrb));
      final ratio = tester.view.devicePixelRatio;
      // A little slack: the cap covers both edges, so the longer one comes out
      // at the panel's size and the other at or above it.
      expect(image.width, lessThanOrEqualTo((panel.width * ratio).ceil() + 2),
          reason: 'the photograph is ${image.width} px across on a '
              '${panel.width} px panel');
      expect(
          image.height, lessThanOrEqualTo((panel.height * ratio).ceil() + 2));
      expect(image.width, greaterThan(1), reason: 'and it is still a picture');

      // And it is put back over the slice it was taken from, not stretched
      // across the whole 400 % picture: at this magnification that slice is at
      // most the panel.
      final over = tester
          .getRect(find.byKey(const ValueKey('viewer-snapshot-overlay')));
      expect(over.width, lessThanOrEqualTo(panel.width + 1));
      expect(over.height, lessThanOrEqualTo(panel.height + 1));

      await hold.up();
      await tester.pump();
    });

    /// **How good the preview is, asked once** (docs/07 §2.2 item 2).
    /// The header's middle picker names the preview resolution, and its menu
    /// carries both answers: the resolutions, and the two playback behaviours
    /// whose button the drawing takes off the bar.
    testWidgets('the quality picker sets the resolution and the playback mode',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      expect(find.byKey(const ValueKey('viewer-resolution')), findsOneWidget);

      // **Opened once**: every row in this menu is an option row, so
      // the menu stays up and the next answer is one tap away.
      await pressBar(tester, 'viewer-resolution');
      await tester.pumpAndSettle();

      Future<void> pick(String key) async {
        await tester.tap(find.byKey(ValueKey<String>(key)));
        await tester.pumpAndSettle();
      }

      await pick('viewer-quality-half');
      expect(p.uiState.previewResolution, PreviewResolution.half,
          reason: 'the bar reaches the same state the View menu sets — the '
              'resolution→scale arithmetic itself is pinned in '
              'menu_bar_frb_test.dart');

      expect(p.uiState.workspace.performance.playback, PlaybackMode.everyFrame,
          reason: 'every frame is the shipped default');
      await pick('viewer-playback-adaptive');
      expect(p.uiState.workspace.performance.playback, PlaybackMode.adaptive,
          reason: 'and the choice is remembered, not just drawn');

      await pick('viewer-playback-everyFrame');
      expect(p.uiState.workspace.performance.playback, PlaybackMode.everyFrame);

      // Three choices, and the menu never went away — which is what makes
      // comparing two tiers a matter of looking rather than of reopening.
      expect(find.byKey(const ValueKey('viewer-quality-auto')), findsOneWidget);
    });

    /// **An option row leaves the menu open, and the pointer leaving takes it
    /// down**. The stay-open half is what the picker test above walks;
    /// this pins the way out, because a menu that stays and cannot be got rid
    /// of by moving away is worse than one that shuts too eagerly.
    testWidgets('the quality menu goes when the pointer leaves it',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      await pressBar(tester, 'viewer-resolution');
      await tester.pumpAndSettle();

      // A pointer that is on the menu, then off it. Before an option is
      // picked the menu ignores the pointer leaving — it was opened by a
      // click and a click is what takes it away.
      final pointer = TestPointer(1, PointerDeviceKind.mouse);
      final row = find.byKey(const ValueKey('viewer-quality-half'));
      await tester.sendEventToBinding(pointer.hover(tester.getCenter(row)));
      await tester.pumpAndSettle();
      await tester.sendEventToBinding(pointer.hover(const Offset(5, 500)));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('viewer-quality-half')), findsOneWidget,
          reason: 'nothing was picked, so nothing armed the way out');

      await tester.tap(row);
      await tester.pumpAndSettle();
      expect(p.uiState.previewResolution, PreviewResolution.half);
      expect(find.byKey(const ValueKey('viewer-quality-half')), findsOneWidget,
          reason: 'the option row left the menu up');

      await tester.sendEventToBinding(pointer.hover(tester.getCenter(row)));
      await tester.pumpAndSettle();
      await tester.sendEventToBinding(pointer.hover(const Offset(5, 500)));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('viewer-quality-half')), findsNothing,
          reason: 'and the pointer leaving is the way out');
    });

    /// **Auto and Full are not the same tier**. Auto renders what the
    /// panel can show — which is what the Viewer has always in fact done, and
    /// why it is the default — while Full means composition resolution
    /// whatever the panel is showing. Before this there was no way to ask for
    /// the latter at all: the tier labelled Full was silently Auto.
    testWidgets('Auto follows the panel, Full does not', (tester) async {
      final p = withLayer();
      p.uiState.workspace.performance.playback = PlaybackMode.everyFrame;
      await mount(tester, p);

      // The panel is smaller than the comp, so the two must differ.
      p.uiState.reportViewerScale(0.25);
      expect(p.uiState.previewResolution, PreviewResolution.full,
          reason: 'Full is the default');
      expect(p.uiState.viewerScale, closeTo(1.0, 1e-9),
          reason: 'Full is comp resolution whatever the panel shows');

      p.uiState.setPreviewResolution(PreviewResolution.auto);
      expect(p.uiState.viewerScale, closeTo(0.25, 1e-9),
          reason: 'Auto renders only what the panel can show');

      p.uiState.setPreviewResolution(PreviewResolution.third);
      expect(p.uiState.viewerScale, closeTo(1.0 / 3.0, 1e-9),
          reason: 'a fixed tier is the tier you asked for');
    });

    /// **The resolution is remembered per composition** (docs/07 §2.2):
    /// a heavy shot can preview at Quarter while the title card beside it does
    /// not, and fronting one back shows its own tier rather than the other's.
    testWidgets('the preview resolution is per composition', (tester) async {
      final p = withLayer();
      final other = p.state.project!.newComposition(name: 'Other');
      await mount(tester, p);

      p.uiState.setPreviewResolution(PreviewResolution.quarter);
      expect(p.uiState.previewResolution, PreviewResolution.quarter);

      p.uiState.setSelectedComp(other);
      expect(p.uiState.previewResolution, PreviewResolution.full,
          reason: 'a comp never set is at the default');

      // And Auto is a choice like any other now that it is not the default:
      // stored, remembered per comp, and not mistaken for "never chosen".
      p.uiState.setPreviewResolution(PreviewResolution.auto);
      expect(
          p.uiState.session().previewResolutions[other.internalid.toString()],
          'auto');

      p.uiState.setSelectedComp(p.comp);
      expect(p.uiState.previewResolution, PreviewResolution.quarter,
          reason: 'and the first comp kept its own');

      // It rides the session blob, so it survives into the project's ui_state,
      // not the document, so no op and no undo step.
      expect(
          p.uiState.session().previewResolutions[p.comp.internalid.toString()],
          'quarter');
    });

    /// **The background swatch is a document edit** (docs/07 §2.2 item
    /// 10) — unlike everything else on that half of the bar, which are ways of
    /// looking. So it goes through an op, reaches the export, and undoes.
    testWidgets('the background swatch writes the comp and undoes',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      await pressBar(tester, 'viewer-guides-menu');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('viewer-background')), findsOneWidget);
      await tester.tapAt(const Offset(4, 4));
      await tester.pumpAndSettle();
      final before = p.comp.background();
      expect(before[3], 1.0, reason: 'a comp starts on opaque black');

      p.comp.setBackground(
        rgba: F32Array4(Float32List.fromList([0.5, 0.25, 0.125, 1.0])),
      );
      final after = p.comp.background();
      expect(after[0], closeTo(0.5, 1e-6));
      expect(after[1], closeTo(0.25, 1e-6));

      p.state.project!.undo();
      expect(p.comp.background()[0], closeTo(before[0], 1e-6),
          reason: 'one undo puts the backdrop back');
    });

    /// A scrub of [pixels] on a [DragValueField]. The first `kDragSlopDefault`
    /// pixels of any drag go on getting it recognised as a drag at all — a real
    /// one loses the same slop — so what is asked for is the slop plus the part
    /// meant to count.
    Future<void> scrub(WidgetTester tester, Finder box, double pixels) =>
        tester.drag(box, Offset(pixels.sign * kDragSlopDefault + pixels, 0));

    /// **The exposure box reads signed stops to one decimal** (docs/07
    /// §2.2 item 12). The sign is not decoration: zero is the middle of this
    /// control's range, not its floor, so `+1.4` and `-2.3` are different
    /// readings and a bare `1.4` would be ambiguous about which.
    testWidgets('the exposure box reads signed stops and scrubs',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final box = find.byKey(const ValueKey('viewer-exposure'));
      expect(box, findsOneWidget);
      expect(find.text('+0.0'), findsOneWidget,
          reason: 'neutral still reads signed, and to one decimal');

      // A tenth of a stop per pixel: 14 pixels right is +1.4.
      await scrub(tester, box, 14);
      await tester.pump();
      expect(find.text('+1.4'), findsOneWidget);
      expect(p.uiState.viewerLook.stops, closeTo(1.4, 1e-9),
          reason: 'the drag reached the state the engine is told from');

      // And back through zero to the other side of it.
      await scrub(tester, box, -37);
      await tester.pump();
      expect(find.text('-2.3'), findsOneWidget);
      expect(p.uiState.viewerLook.stops, closeTo(-2.3, 1e-9));

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });

    /// The tone map is a row in the colour-pipeline menu (item 13) —
    /// inside the display transform it is part of. One pick on, one pick off.
    testWidgets('the tone-map row is in the colour menu and flips',
        (tester) async {
      final p = withLayer();
      p.uiState.workspace.interface.showToneMap = true;
      await mount(tester, p);

      expect(p.uiState.viewerLook.toneMap, isFalse);

      await flipToneMap(tester);
      expect(p.uiState.viewerLook.toneMap, isTrue);

      await flipToneMap(tester);
      expect(p.uiState.viewerLook.toneMap, isFalse);

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });

    /// **The colour pipeline says what you are looking at** (docs/07 §2.2
    /// item 8). Always in the header, naming the display transform, and
    /// while either preview-only control is engaged it is where the Viewer
    /// says the picture on screen is not the export.
    testWidgets('the colour picker says when a view is engaged',
        (tester) async {
      final p = withLayer();
      // The tone map is asked for; this test drives it, so it asks.
      p.uiState.workspace.interface.showToneMap = true;
      await mount(tester, p);
      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);

      final picker = find.byKey(const ValueKey('viewer-colour'));
      expect(picker, findsOneWidget, reason: 'it is always in the header');

      Text faceText() => tester.widget<Text>(
          find.descendant(of: picker, matching: find.byType(Text)).first);

      expect(faceText().data, 'Linear → sRGB');
      expect(faceText().style?.color, isNull,
          reason: "at rest it takes the dropdown face's own colour");

      // The tone map is engaged: the face, not just the control, says so.
      await flipToneMap(tester);
      expect(faceText().data, contains('preview'));
      expect(faceText().data, contains('Linear → sRGB'),
          reason: 'it still names the transform it is showing through');
      expect(faceText().style?.color, t.accent);

      // Back to neutral, back to a plain statement of the transform.
      await flipToneMap(tester);
      expect(faceText().data, 'Linear → sRGB');

      // And the exposure engages it on its own.
      await scrub(tester, find.byKey(const ValueKey('viewer-exposure')), 10);
      await tester.pump();
      expect(faceText().data, contains('preview'));

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });

    /// The tone map is asked for, not given: its row is out of the colour
    /// menu unless Settings → Interface says otherwise, while the exposure
    /// stays on the bar whatever the setting says.
    testWidgets('the tone-map row is absent until the setting asks for it',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      await pressBar(tester, 'viewer-colour');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('viewer-tone-map')), findsNothing);
      await tester.tapAt(const Offset(4, 4));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('viewer-exposure')), findsOneWidget,
          reason: 'only the tone map is gated, not the exposure');

      p.uiState.workspace.interface.showToneMap = true;
      await mount(tester, p);
      await pressBar(tester, 'viewer-colour');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('viewer-tone-map')), findsOneWidget);
      await tester.tapAt(const Offset(4, 4));
      await tester.pumpAndSettle();

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });

    /// Hiding the button must not strand an engaged tone map: a session saved
    /// while it was on would otherwise keep changing the picture with nothing
    /// left to turn it off. The setting gates the *look*, not just the button.
    testWidgets('a stored tone map is disengaged while the setting is off',
        (tester) async {
      final p = withLayer();
      p.uiState.workspace.interface.showToneMap = true;
      await mount(tester, p);

      await flipToneMap(tester);
      expect(p.uiState.viewerLook.toneMap, isTrue);

      p.uiState.workspace.interface.showToneMap = false;
      await tester.pump();
      expect(p.uiState.viewerLook.toneMap, isFalse,
          reason: 'the look the Viewer and the engine read is disengaged');
      expect(p.uiState.session().viewerLooks[p.comp.internalid.toString()],
          (stops: 0.0, toneMap: true),
          reason: 'the stored value is untouched, so turning it back on '
              'returns the comp to how it was');

      p.uiState.workspace.interface.showToneMap = true;
      await tester.pump();
      expect(p.uiState.viewerLook.toneMap, isTrue);

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });

    /// **Both controls are per composition**: they are a way of looking
    /// at one comp, so fronting another must show that one's own view rather
    /// than carrying the first one's exposure across.
    testWidgets('the exposure and tone map are remembered per composition',
        (tester) async {
      final p = withLayer();
      p.uiState.workspace.interface.showToneMap = true;
      final other = p.state.project!.newComposition(name: 'Other');
      await mount(tester, p);

      await scrub(tester, find.byKey(const ValueKey('viewer-exposure')), 20);
      await flipToneMap(tester);
      expect(p.uiState.viewerLook, (stops: 2.0, toneMap: true));

      p.uiState.setSelectedComp(other);
      await tester.pump();
      expect(p.uiState.viewerLook, (stops: 0.0, toneMap: false),
          reason: 'a comp never looked at is looked at neutrally');
      expect(find.text('+0.0'), findsOneWidget);

      p.uiState.setSelectedComp(p.comp);
      await tester.pump();
      expect(p.uiState.viewerLook, (stops: 2.0, toneMap: true));
      expect(find.text('+2.0'), findsOneWidget);

      // And it is written into the session, which is what carries it into the
      // project's `ui_state` blob — not into the document, so no op
      // and no undo step.
      expect(p.uiState.session().viewerLooks[p.comp.internalid.toString()],
          (stops: 2.0, toneMap: true));

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });

    /// The one place in this port where a single gesture is two ops: x and y are
    /// separate properties in the model.
    testWidgets('dragging a selected layer repositions it', (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final before = p.layer.getTransform();
      final beforeX = (before.positionX as BridgeScalar_Static).field0;

      // The layer fills the comp, so the middle of the picture is inside it —
      // there is no handle to find any more: the body is the handle.
      final stage = find.byType(ViewerPanelFrb);
      final gesture = await tester.startGesture(tester.getCenter(stage));
      await tester.pump();
      for (var i = 0; i < 8; i++) {
        await gesture.moveBy(const Offset(6, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final after = p.layer.getTransform();
      expect(
          (after.positionX as BridgeScalar_Static).field0, greaterThan(beforeX),
          reason: 'the drag reached the document');
    });

    /// **One gesture, one undo step.** x and y are separate properties
    /// in the model, and writing them separately made a single drag two steps:
    /// the first Ctrl+Z put the layer back along one axis only, which reads as
    /// the undo being broken rather than as two honest edits. The batch op the
    /// Anchor point tool already used is what fixes it.
    testWidgets('a drag is one undo step, not one per axis', (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final before = p.layer.getTransform();
      final beforeX = (before.positionX as BridgeScalar_Static).field0;
      final beforeY = (before.positionY as BridgeScalar_Static).field0;

      final stage = find.byType(ViewerPanelFrb);
      final gesture = await tester.startGesture(tester.getCenter(stage));
      await tester.pump();
      for (var i = 0; i < 8; i++) {
        await gesture.moveBy(const Offset(6, 5));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final moved = p.layer.getTransform();
      expect((moved.positionX as BridgeScalar_Static).field0,
          isNot(closeTo(beforeX, 1e-9)));
      expect((moved.positionY as BridgeScalar_Static).field0,
          isNot(closeTo(beforeY, 1e-9)));

      p.state.project!.undo();

      final after = p.layer.getTransform();
      expect((after.positionX as BridgeScalar_Static).field0,
          closeTo(beforeX, 1e-9));
      expect((after.positionY as BridgeScalar_Static).field0,
          closeTo(beforeY, 1e-9),
          reason: 'one undo puts back the whole drag, both axes at once');
    });

    testWidgets(
        'with the Hand tool a drag pans the view and leaves the layer'
        ' alone', (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.hand);
      await mount(tester, p);

      final before = p.layer.getTransform();
      final beforeX = (before.positionX as BridgeScalar_Static).field0;

      final stage = find.byType(ViewerPanelFrb);
      final gesture = await tester.startGesture(tester.getCenter(stage));
      await tester.pump();
      for (var i = 0; i < 8; i++) {
        await gesture.moveBy(const Offset(6, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect((p.layer.getTransform().positionX as BridgeScalar_Static).field0,
          beforeX,
          reason: 'the Hand tool moves the picture, never the layer');
    });

    testWidgets(
        'a drag from empty space marquees, and takes what is wholly'
        ' inside it', (tester) async {
      final p = withLayer();
      // A small solid, so the marquee can enclose it without enclosing the
      // comp-sized adjustment layer above it.
      final solid = p.comp.addSolidLayer();
      solid.setTransform(
          prop: BridgeTransformProp.scaleX, value: BridgeScalar.static_(10));
      solid.setTransform(
          prop: BridgeTransformProp.scaleY, value: BridgeScalar.static_(10));
      p.uiState.clearSelection();
      p.uiState.model.refresh();
      await mount(tester, p);

      // Sweep the whole panel: everything wholly inside is taken, and the
      // adjustment layer's box is exactly the comp, so it qualifies too.
      final stage = tester.getRect(find.byKey(const ValueKey('viewer-stage')));
      final gesture =
          await tester.startGesture(stage.topLeft + const Offset(2, 2));
      await tester.pump();
      await gesture.moveTo(stage.center);
      await tester.pump();
      await gesture.moveTo(stage.bottomRight - const Offset(2, 2));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      expect(p.uiState.selectedLayers.value, isNotEmpty,
          reason: 'a marquee over everything selects something');
      expect(
          p.uiState.selectedLayers.value
              .any((l) => l.internallayerId == solid.internallayerId),
          isTrue,
          reason: 'the small solid is wholly inside the sweep');
    });

    testWidgets('an animated position gets no box, so nothing drags it',
        (tester) async {
      final p = withLayer();
      // A position that is a curve has no single point to drag.
      p.layer.setTransform(
        prop: BridgeTransformProp.positionX,
        value: BridgeScalar.keyframed([
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 0),
            value: 0,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
          BridgeKeyframe(
            time: p.comp.timeOfFrame(frame: 30),
            value: 400,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ),
        ]),
      );
      p.uiState.model.refresh();
      await mount(tester, p);

      final stage = find.byType(ViewerPanelFrb);
      final gesture = await tester.startGesture(tester.getCenter(stage));
      await tester.pump();
      await gesture.moveBy(const Offset(40, 0));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      final after = p.layer.getTransform();
      expect(after.positionX, isA<BridgeScalar_Keyframed>(),
          reason: 'a curve is not overwritten by a drag it never accepted');
    });

    /// Where the picture is drawn inside the panel, worked out the way the
    /// panel works it out: the stage is the panel less its bar, and the comp is
    /// fitted into it. The gizmo's handles sit on this rectangle for a
    /// comp-sized layer, which is what lets a test grab one.
    Rect fittedRect(WidgetTester tester, CompositionReference comp) {
      // Measured rather than worked out from the panel less a bar height: the
      // Viewer wears a header strip as well as a bottom bar, and a
      // hard-coded number here silently moves every picture coordinate the
      // moment either strip changes.
      final stage = tester.getRect(find.byKey(const ValueKey('viewer-stage')));
      final size = comp.getSize();
      final scale =
          math.min(stage.width / size.width, stage.height / size.height);
      final drawn = Size(size.width * scale, size.height * scale);
      return Rect.fromLTWH(
        stage.left + (stage.width - drawn.width) / 2,
        stage.top + (stage.height - drawn.height) / 2,
        drawn.width,
        drawn.height,
      );
    }

    /// The magnification the Viewer's own picker is showing, as a fraction, or
    /// null while it says "Fit".
    ///
    /// The observable for a zoom *out*, now that the scale reported to the
    /// engine deliberately does not follow one down.
    double? shownZoom(WidgetTester tester) {
      for (final text in tester.widgetList<Text>(find.descendant(
        of: find.byKey(const ValueKey('viewer-zoom')),
        matching: find.byType(Text),
      ))) {
        final label = text.data;
        if (label == null || !label.endsWith('%')) continue;
        return double.parse(label.substring(0, label.length - 1)) / 100;
      }
      return null;
    }

    /// **A layer switched off is not on the picture at all.** Its eye
    /// being off is how you get it out of the way; a box round something
    /// invisible, and a click that selected it, put it right back in the way.
    testWidgets(
        'a hidden layer is neither drawn nor clickable, and the one'
        ' under it takes the click', (tester) async {
      final p = withLayer();
      // A second comp-sized layer on top of the first, then switched off.
      final above = p.comp.addSolidLayer();
      above.setSwitch(switch_: BridgeLayerSwitch.visible, on_: false);
      p.uiState.clearSelection();
      p.uiState.model.refresh();
      await mount(tester, p);

      await tester.tapAt(fittedRect(tester, p.comp).center);
      await tester.pumpAndSettle();

      expect(p.uiState.selectedLayer.value?.internallayerId,
          p.layer.internallayerId,
          reason: 'the click fell through the hidden layer to the one below');
      expect(
          p.uiState.selectedLayers.value
              .any((l) => l.internallayerId == above.internallayerId),
          isFalse,
          reason: 'and the hidden layer was never a target');
    });

    /// **A drag takes what is selected, whatever is on top of it.**
    /// A layer chosen in the Timeline could not be dragged wherever anything
    /// covered it: the press swapped the selection for the topmost layer and
    /// moved that instead.
    testWidgets(
        'a drag inside the selection moves the selected layer, not the'
        ' one above it', (tester) async {
      final p = withLayer();
      // A second comp-sized layer, added last and therefore on top of the one
      // the test selects.
      final above = p.comp.addSolidLayer();
      p.uiState.setSelection([p.layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      final aboveBefore =
          (above.getTransform().positionX as BridgeScalar_Static).field0;
      final belowBefore =
          (p.layer.getTransform().positionX as BridgeScalar_Static).field0;

      final gesture =
          await tester.startGesture(fittedRect(tester, p.comp).center);
      await tester.pump();
      for (var i = 0; i < 8; i++) {
        await gesture.moveBy(const Offset(6, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect((p.layer.getTransform().positionX as BridgeScalar_Static).field0,
          greaterThan(belowBefore),
          reason: 'the layer that was selected is the layer that moved');
      expect((above.getTransform().positionX as BridgeScalar_Static).field0,
          closeTo(aboveBefore, 1e-9),
          reason: 'the layer on top was never picked up');
      expect(p.uiState.selectedLayer.value?.internallayerId,
          p.layer.internallayerId,
          reason: 'and the selection was not quietly swapped either');
    });

    testWidgets(
        'clicking picks the layer under the pointer, and Shift adds to'
        ' the selection', (tester) async {
      final p = withLayer();
      final second = p.comp.addSolidLayer();
      p.uiState.clearSelection();
      p.uiState.model.refresh();
      await mount(tester, p);

      // Both layers are comp-sized, so the middle of the picture is inside
      // both and the topmost — the solid, added last and therefore on top —
      // takes the click.
      await tester.tapAt(fittedRect(tester, p.comp).center);
      await tester.pumpAndSettle();
      expect(p.uiState.selectedLayers.value.length, 1);
      expect(p.uiState.selectedLayer.value?.internallayerId,
          second.internallayerId,
          reason: 'the topmost layer takes the click');

      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.tapAt(fittedRect(tester, p.comp).center);
      await tester.pumpAndSettle();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);

      expect(p.uiState.selectedLayers.value.length, isNot(1),
          reason: 'Shift-clicking the same layer takes it back out again');
    });

    testWidgets(
        'a Null layer can be picked on the picture, though it draws'
        ' nothing', (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Rig');
      final nul = comp.addNullLayer();
      p.uiState.setSelectedComp(comp);
      p.uiState.model.refresh();
      await mount(tester, p);

      final fitted = fittedRect(tester, comp);
      // The Null's own 100x100 box sits on the comp's middle.
      await tester.tapAt(fitted.center);
      await tester.pumpAndSettle();
      expect(
          p.uiState.selectedLayer.value?.internallayerId, nul.internallayerId,
          reason: 'a layer with no pixels is still a layer you can point at');

      // Well outside that small box, and there is nothing else in the comp.
      await tester.tapAt(fitted.center + const Offset(200, 0));
      await tester.pumpAndSettle();
      expect(p.uiState.selectedLayers.value, isEmpty);
    });

    testWidgets('clicking empty space clears the selection', (tester) async {
      final p = withLayer();
      await mount(tester, p);
      expect(p.uiState.selectedLayers.value, isNotEmpty);

      // The very corner of the *stage* is outside the fitted picture, so it is
      // outside every layer's box. The panel's own corner is the header strip,
      // which is chrome and takes no click for the picture.
      final panel = tester.getRect(find.byKey(const ValueKey('viewer-stage')));
      await tester.tapAt(panel.topLeft + const Offset(2, 2));
      await tester.pumpAndSettle();

      expect(p.uiState.selectedLayers.value, isEmpty);
      expect(p.uiState.selectedLayer.value, isNull,
          reason: 'the primary follows the selection');
    });

    /// The selected layer's box on screen, for a comp-sized layer scaled to
    /// [scalePercent] about its own middle: the fitted picture, shrunk about
    /// its centre. Half size keeps the handles well inside the window, where a
    /// gesture can reach them — a corner handle on a comp-sized layer sits on
    /// the window's own edge.
    Rect boxRect(
        WidgetTester tester, CompositionReference comp, double scalePercent) {
      final fitted = fittedRect(tester, comp);
      final factor = scalePercent / 100.0;
      return Rect.fromCenter(
        center: fitted.center,
        width: fitted.width * factor,
        height: fitted.height * factor,
      );
    }

    /// A layer at half size, so its handles are reachable.
    void halveIt(LayerReference layer) {
      layer.setTransform(
          prop: BridgeTransformProp.scaleX, value: BridgeScalar.static_(50));
      layer.setTransform(
          prop: BridgeTransformProp.scaleY, value: BridgeScalar.static_(50));
    }

    testWidgets('dragging a corner handle scales the layer', (tester) async {
      final p = withLayer();
      halveIt(p.layer);
      p.uiState.model.refresh();
      await mount(tester, p);

      final before =
          (p.layer.getTransform().scaleX as BridgeScalar_Static).field0;
      final box = boxRect(tester, p.comp, 50);

      final gesture = await tester.startGesture(box.bottomRight);
      await tester.pump();
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(10, 6));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final after =
          (p.layer.getTransform().scaleX as BridgeScalar_Static).field0;
      expect(after, greaterThan(before),
          reason: 'pulling the corner away from the anchor grows the layer');
    });

    testWidgets('dragging the rotation knob turns the layer', (tester) async {
      final p = withLayer();
      halveIt(p.layer);
      p.uiState.model.refresh();
      await mount(tester, p);

      final box = boxRect(tester, p.comp, 50);
      final knob = Offset(box.center.dx, box.top - gizmoRotateReach);

      final gesture = await tester.startGesture(knob);
      await tester.pump();
      // Round towards the right-hand side: a clockwise sweep about the middle.
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(20, 10));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final rotation = p.layer.getTransform().rotation;
      expect((rotation as BridgeScalar_Static).field0, isNot(0),
          reason: 'the knob wrote a rotation');
    });

    /// The layer controls are a mark over the picture like the grid and the
    /// safe areas, so they are a row in the same view menu.
    testWidgets('the layer-controls row is in the view menu and toggles',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      await pickViewRow(tester, 'viewer-wireframes');
      // Hiding the controls must not disturb the selection or the picture: it
      // is a drawing switch, nothing more.
      expect(p.uiState.selectedLayers.value, isNotEmpty);
    });

    /// The Zoom tool armed, on a comp bigger than the panel so there is room
    /// to zoom in before the clamp.
    Future<
        ({
          LumitState state,
          LumitUiState uiState,
          CompositionReference comp,
          LayerReference layer
        })> withZoomTool(
      WidgetTester tester, {
      AnimationLevel motion = AnimationLevel.none,
    }) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.zoom);
      // These read the magnification through `viewerScale`, which only tracks
      // it on Auto — a fixed tier is the tier you asked for whatever the panel
      // is showing, and Full is now the default.
      p.uiState.setPreviewResolution(PreviewResolution.auto);
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
        animationLevel: motion,
      ));
      await tester.pumpAndSettle();
      return p;
    }

    testWidgets('the Zoom tool zooms in where it is clicked, and out with Alt',
        (tester) async {
      final p = await withZoomTool(tester);
      final fitted = fittedRect(tester, p.comp);
      final before = p.uiState.viewerScale;

      await tester.tapAt(fitted.center + const Offset(60, 20));
      await tester.pumpAndSettle();
      final zoomedIn = p.uiState.viewerScale;
      expect(zoomedIn, greaterThan(before),
          reason: 'a click magnifies about the point it landed on');
      expect(zoomedIn, closeTo(before * zoomToolStep, 1e-6));

      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.tapAt(fitted.center + const Offset(60, 20));
      await tester.pumpAndSettle();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);

      expect(p.uiState.viewerScale, closeTo(before, 1e-6),
          reason: 'Alt+click undoes the click before it');
    });

    testWidgets('dragging a box with the Zoom tool fits that box to the panel',
        (tester) async {
      final p = await withZoomTool(tester);
      final fitted = fittedRect(tester, p.comp);
      final before = p.uiState.viewerScale;

      // A quarter-width sweep in the middle of the picture.
      final from = fitted.center - Offset(fitted.width / 8, fitted.height / 8);
      final to = fitted.center + Offset(fitted.width / 8, fitted.height / 8);
      final gesture = await tester.startGesture(from);
      await tester.pump();
      await gesture.moveTo(Offset(to.dx, from.dy));
      await tester.pump();
      await gesture.moveTo(to);
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      expect(p.uiState.viewerScale, greaterThan(before * 2),
          reason: 'a quarter of the picture fills the panel');
    });

    testWidgets('a box drag with Alt zooms out instead', (tester) async {
      final p = await withZoomTool(tester);
      final fitted = fittedRect(tester, p.comp);
      // The magnification it starts at, off the picture rather than off the
      // scale reported to the engine — that one no longer follows a zoom out.
      final before = fitted.width / p.comp.getSize().width;

      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      final from = fitted.center - Offset(fitted.width / 8, fitted.height / 8);
      final to = fitted.center + Offset(fitted.width / 8, fitted.height / 8);
      final gesture = await tester.startGesture(from);
      await tester.pump();
      // In steps: a single jump gives the recogniser a start and an end with
      // no update between them, so the box would be the width of the slop.
      await gesture.moveTo(Offset(to.dx, from.dy));
      await tester.pump();
      await gesture.moveTo(to);
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);

      expect(shownZoom(tester), isNotNull);
      expect(shownZoom(tester)!, lessThan(before));
    });

    testWidgets('a tiny wobble of a drag is a click, not a box',
        (tester) async {
      final p = await withZoomTool(tester);
      final fitted = fittedRect(tester, p.comp);
      final before = p.uiState.viewerScale;

      final gesture = await tester.startGesture(fitted.center);
      await tester.pump();
      await gesture.moveBy(const Offset(3, 2));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      // A few pixels of travel is a hand, not an intention: fitting a
      // three-pixel box to the panel would throw the picture into orbit. It
      // takes the click's own step instead.
      expect(p.uiState.viewerScale, closeTo(before * zoomToolStep, 1e-6));
    });

    testWidgets('the zoom flies rather than jumping when the shell animates',
        (tester) async {
      final p = await withZoomTool(tester, motion: AnimationLevel.all);
      final fitted = fittedRect(tester, p.comp);
      final before = p.uiState.viewerScale;

      await tester.tapAt(fitted.center);
      // Part-way through the flight the magnification is between the two.
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 40));
      final midway = p.uiState.viewerScale;
      expect(midway, greaterThan(before));
      expect(midway, lessThan(before * zoomToolStep),
          reason: 'it is on its way, not there yet');

      await tester.pumpAndSettle();
      expect(p.uiState.viewerScale, closeTo(before * zoomToolStep, 1e-6),
          reason: 'and it lands exactly where it was sent');
    });

    testWidgets('with motion off the zoom lands on the first frame',
        (tester) async {
      final p = await withZoomTool(tester);
      final fitted = fittedRect(tester, p.comp);
      final before = p.uiState.viewerScale;

      await tester.tapAt(fitted.center);
      await tester.pump();

      expect(p.uiState.viewerScale, closeTo(before * zoomToolStep, 1e-6),
          reason: 'no animation means the hard cut, immediately');
    });

    testWidgets(
        'the Rotation tool turns the selection about its anchor, and'
        ' leaves unselected layers alone', (tester) async {
      final p = withLayer();
      final other = p.comp.addSolidLayer();
      // Only the adjustment layer is selected.
      p.uiState.setSelection([p.layer]);
      p.uiState.tools.select(ToolMode.rotate);
      p.uiState.model.refresh();
      await mount(tester, p);

      final fitted = fittedRect(tester, p.comp);
      // A quarter-turn about the middle: straight up, round to the right.
      final gesture = await tester
          .startGesture(Offset(fitted.center.dx, fitted.center.dy - 100));
      await tester.pump();
      await gesture
          .moveTo(Offset(fitted.center.dx + 70, fitted.center.dy - 70));
      await tester.pump();
      await gesture.moveTo(Offset(fitted.center.dx + 100, fitted.center.dy));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      final turned =
          (p.layer.getTransform().rotation as BridgeScalar_Static).field0;
      expect(turned, closeTo(90, 0.5),
          reason: 'the angle swept about the anchor is the angle written');
      expect((other.getTransform().rotation as BridgeScalar_Static).field0, 0,
          reason: 'a layer that was not selected does not turn');
    });

    /// **The wireframe turns with the picture, not after it.** The
    /// picture is previewed at the new angle while the drag is in flight; the
    /// boxes are drawn from the document, which still holds the old one, so
    /// they sat still until the button came up. The angle in flight is
    /// published where the layer that draws the boxes can read it.
    testWidgets('the boxes follow a turn while it is still being made',
        (tester) async {
      final p = withLayer();
      p.uiState.setSelection([p.layer]);
      p.uiState.tools.select(ToolMode.rotate);
      p.uiState.model.refresh();
      await mount(tester, p);

      expect(p.uiState.liveRotations.value, isEmpty,
          reason: 'nothing is turning yet');

      final fitted = fittedRect(tester, p.comp);
      final gesture = await tester
          .startGesture(Offset(fitted.center.dx, fitted.center.dy - 100));
      await tester.pump();
      // Two moves, because the first is what the framework spends recognising
      // the drag: the update that carries the turn is the one after it.
      await gesture
          .moveTo(Offset(fitted.center.dx + 40, fitted.center.dy - 92));
      await tester.pump();
      await gesture
          .moveTo(Offset(fitted.center.dx + 70, fitted.center.dy - 70));
      await tester.pump();

      final live = p.uiState.liveRotations.value[p.layer.internallayerId];
      expect(live, isNotNull,
          reason: 'the angle in flight is published as the drag happens');
      expect(live!, closeTo(45, 1), reason: 'and it is the angle swept so far');
      expect(p.layer.getTransform().rotation, isA<BridgeScalar_Static>());
      expect((p.layer.getTransform().rotation as BridgeScalar_Static).field0, 0,
          reason: 'while the document has not been written to at all');

      await gesture.up();
      await tester.pumpAndSettle();
      expect(p.uiState.liveRotations.value, isEmpty,
          reason: 'and the moment it lands, the document is the only truth');
    });

    /// **And the wireframe follows a value scrub, for the same reason.** The
    /// turn above is a drag on the picture; this is a drag in the property
    /// rows, which previews the picture through the same provisional-transform
    /// path. The rows publish what they are previewing and the boxes read it,
    /// so Position and Scale move the box as they are dragged rather than on
    /// release. The document is not written to until the drag lands.
    testWidgets('the boxes follow a value scrub while it is still being made',
        (tester) async {
      final p = withLayer();
      p.uiState.setSelection([p.layer]);
      p.uiState.model.refresh();
      await mount(tester, p);

      ViewerLayerMap mapOfBox() => tester
          .widget<ViewerGizmoLayer>(find.byType(ViewerGizmoLayer))
          .boxes
          .firstWhere((b) => b.id == p.layer.internallayerId)
          .map;

      final settled = mapOfBox();
      expect(p.uiState.liveTransforms.value, isEmpty,
          reason: 'nothing is being scrubbed yet');

      // What a Position drag in the rows publishes: the document's transform
      // with the one property replaced, exactly what it sends for the picture.
      final committed = p.layer.getTransform();
      p.uiState.liveTransforms.value = {
        p.layer.internallayerId: writeScalar(
          committed,
          BridgeTransformProp.positionX,
          BridgeScalar.static_((settled.px) + 120),
        ),
      };
      await tester.pump();

      expect(mapOfBox().px, closeTo(settled.px + 120, 0.01),
          reason: 'the box is drawn from the value being dragged');
      expect((p.layer.getTransform().positionX as BridgeScalar_Static).field0,
          closeTo(settled.px, 0.01),
          reason: 'while the document still holds the old one');

      // Release: the row clears what it published and the document is the
      // only truth again. A value left behind here would freeze the box.
      p.uiState.liveTransforms.value = const {};
      await tester.pump();
      expect(mapOfBox().px, closeTo(settled.px, 0.01),
          reason: 'and the box goes back to what the document says');
    });

    testWidgets('Shift locks the turn to 45 degrees', (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.rotate);
      await mount(tester, p);

      final fitted = fittedRect(tester, p.comp);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      final gesture = await tester
          .startGesture(Offset(fitted.center.dx, fitted.center.dy - 100));
      await tester.pump();
      // A little over 30 degrees round: without the lock it would write ~34.
      await gesture
          .moveTo(Offset(fitted.center.dx + 56, fitted.center.dy - 83));
      await tester.pump();
      await gesture
          .moveTo(Offset(fitted.center.dx + 58, fitted.center.dy - 81));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);

      final turned =
          (p.layer.getTransform().rotation as BridgeScalar_Static).field0;
      expect(turned % 45, closeTo(0, 1e-6),
          reason: 'held Shift, so it lands on a 45-degree step');
    });

    testWidgets('the Rotation tool picks a layer when you click one',
        (tester) async {
      final p = withLayer();
      p.uiState.clearSelection();
      p.uiState.tools.select(ToolMode.rotate);
      await mount(tester, p);

      await tester.tapAt(fittedRect(tester, p.comp).center);
      await tester.pumpAndSettle();

      expect(p.uiState.selectedLayer.value?.internallayerId,
          p.layer.internallayerId,
          reason: 'a rotation tool you cannot choose a layer with is a trip'
              ' back to the toolbar between every turn');
    });

    testWidgets('with nothing selected the Rotation tool turns nothing',
        (tester) async {
      final p = withLayer();
      p.uiState.clearSelection();
      p.uiState.tools.select(ToolMode.rotate);
      await mount(tester, p);

      final fitted = fittedRect(tester, p.comp);
      final gesture = await tester
          .startGesture(Offset(fitted.center.dx, fitted.center.dy - 100));
      await tester.pump();
      await gesture.moveTo(Offset(fitted.center.dx + 60, fitted.center.dy));
      await tester.pump();
      await gesture.moveTo(Offset(fitted.center.dx + 100, fitted.center.dy));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      expect(
          (p.layer.getTransform().rotation as BridgeScalar_Static).field0, 0);
    });

    testWidgets(
        'the Anchor point tool slides the pivot and leaves the picture'
        ' where it was', (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.anchor);
      await mount(tester, p);

      final before = p.layer.getTransform();
      double at(BridgeScalar s) => (s as BridgeScalar_Static).field0;
      final anchorBefore = (at(before.anchorX), at(before.anchorY));
      final positionBefore = (at(before.positionX), at(before.positionY));

      final fitted = fittedRect(tester, p.comp);
      final gesture = await tester.startGesture(fitted.center);
      await tester.pump();
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(10, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final after = p.layer.getTransform();
      expect(at(after.anchorX), isNot(anchorBefore.$1),
          reason: 'the pivot moved');
      // Pan behind: the anchor moved right, so Position moved right by exactly
      // as much (the layer is unscaled and unturned), and the picture did not
      // move at all.
      final anchorDelta = at(after.anchorX) - anchorBefore.$1;
      final positionDelta = at(after.positionX) - positionBefore.$1;
      expect(positionDelta, closeTo(anchorDelta, 0.001),
          reason: 'Position compensated exactly, so nothing appeared to move');
      expect(at(after.anchorY), closeTo(anchorBefore.$2, 0.001),
          reason: 'a sideways drag does not move the pivot vertically');
    });

    /// **The pivot goes where you point.** It used to be a *nudge*:
    /// the drag was measured from the press and added to the anchor the layer
    /// already had, so you could push a pivot towards somewhere but never put
    /// it anywhere. A click now places it, and a drag keeps it under the
    /// pointer the whole way.
    testWidgets('the Anchor point tool puts the pivot where you click',
        (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.anchor);
      await mount(tester, p);

      double at(BridgeScalar s) => (s as BridgeScalar_Static).field0;
      final fitted = fittedRect(tester, p.comp);
      // A quarter of the way in from the layer's top-left, which for a
      // comp-sized layer is a quarter of the comp.
      final target =
          fitted.topLeft + Offset(fitted.width / 4, fitted.height / 4);
      await tester.tapAt(target);
      await tester.pumpAndSettle();

      final size = p.comp.getSize();
      final scale = fitted.width / size.width;
      final after = p.layer.getTransform();
      expect(at(after.anchorX), closeTo((target.dx - fitted.left) / scale, 1),
          reason: 'the pivot is where the pointer was, not a nudge from where '
              'it started');
      expect(at(after.anchorY), closeTo((target.dy - fitted.top) / scale, 1));
    });

    /// **And the edit reaches the panels that show it.** The Timeline's
    /// Anchor Point rows and the Effect controls both draw from the read model,
    /// so an edit the Viewer commits has to refresh it — the tool's own
    /// picture updating is not the same thing as the numbers updating.
    testWidgets('an anchor edit reaches the read model the panels draw from',
        (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.anchor);
      await mount(tester, p);

      double? modelAnchorX() {
        final entry = p.uiState.model.byId(p.layer.internallayerId);
        final x = entry?.info.transform.anchorX;
        return x is BridgeScalar_Static ? x.field0 : null;
      }

      final before = modelAnchorX();
      final fitted = fittedRect(tester, p.comp);
      await tester.tapAt(fitted.center + const Offset(60, 0));
      await tester.pumpAndSettle();

      expect(modelAnchorX(), isNot(before),
          reason: 'the model the Timeline rows read is the one that moved');
    });

    /// **Every tool's edit, not just the anchor's.** A drag with the Selection
    /// tool and a turn with the Rotation tool go the same way: the Viewer
    /// commits, and the read model the Timeline's rows and the Effect controls
    /// draw from has to be refreshed, or the numbers sit still while the
    /// picture moves.
    testWidgets('a move and a turn reach the read model too', (tester) async {
      double? modelValue(LumitUiState ui, LayerReference layer,
          BridgeScalar Function(BridgeTransform) pick) {
        final entry = ui.model.byId(layer.internallayerId);
        final tf = entry?.info.transform;
        if (tf == null) return null;
        final v = pick(tf);
        return v is BridgeScalar_Static ? v.field0 : null;
      }

      for (final (tool, pick, what)
          in <(ToolMode, BridgeScalar Function(BridgeTransform), String)>[
        (ToolMode.select, (tf) => tf.positionX, 'a move'),
        (ToolMode.rotate, (tf) => tf.rotation, 'a turn'),
      ]) {
        final p = withLayer();
        p.uiState.setSelection([p.layer]);
        p.uiState.tools.select(tool);
        p.uiState.model.refresh();
        await mount(tester, p);

        final before = modelValue(p.uiState, p.layer, pick);
        final fitted = fittedRect(tester, p.comp);
        final gesture = await tester
            .startGesture(Offset(fitted.center.dx, fitted.center.dy - 80));
        await tester.pump();
        for (var i = 0; i < 6; i++) {
          await gesture.moveBy(const Offset(12, 6));
          await tester.pump();
        }
        await gesture.up();
        await tester.pumpAndSettle();

        expect(modelValue(p.uiState, p.layer, pick), isNot(before),
            reason: '$what must reach the model the panels draw from');
      }
    });

    testWidgets('the Anchor point tool picks a layer when you click one',
        (tester) async {
      final p = withLayer();
      p.uiState.clearSelection();
      p.uiState.tools.select(ToolMode.anchor);
      await mount(tester, p);

      await tester.tapAt(fittedRect(tester, p.comp).center);
      await tester.pumpAndSettle();

      expect(p.uiState.selectedLayer.value?.internallayerId,
          p.layer.internallayerId);
    });

    testWidgets(
        'the gizmo\'s centre handle pans behind, and a drag beside it'
        ' still moves the layer', (tester) async {
      final p = withLayer();
      halveIt(p.layer);
      p.uiState.model.refresh();
      await mount(tester, p);

      double at(BridgeScalar s) => (s as BridgeScalar_Static).field0;
      final before = p.layer.getTransform();
      final anchorBefore = at(before.anchorX);
      final positionBefore = at(before.positionX);

      // Dead on the pivot — the middle of a layer anchored on its own centre.
      final box = boxRect(tester, p.comp, 50);
      var gesture = await tester.startGesture(box.center);
      await tester.pump();
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(8, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final panned = p.layer.getTransform();
      expect(at(panned.anchorX), isNot(anchorBefore),
          reason: 'the pivot moved');
      expect(at(panned.positionX) - positionBefore,
          closeTo((at(panned.anchorX) - anchorBefore) / 2, 0.001),
          reason: 'Position compensated (at 50%, half the layer-pixel delta), '
              'so the picture did not move');

      // A press a little way off the pivot is an ordinary move again.
      final anchorNow = at(p.layer.getTransform().anchorX);
      final positionNow = at(p.layer.getTransform().positionX);
      gesture = await tester.startGesture(box.center + const Offset(40, 0));
      await tester.pump();
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(8, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final moved = p.layer.getTransform();
      expect(at(moved.anchorX), closeTo(anchorNow, 0.001),
          reason: 'a body drag leaves the pivot alone');
      expect(at(moved.positionX), greaterThan(positionNow));
    });

    /// The shape tools. With a layer selected a drag draws a **mask** on
    /// it; with nothing selected there is nothing to mask, and the status line
    /// says so rather than the drag vanishing into silence.
    testWidgets('a shape drag adds a mask to the selected layer',
        (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.shapeEllipse);
      await mount(tester, p);
      expect(p.layer.getMasks(), isEmpty);

      final fitted = fittedRect(tester, p.comp);
      final gesture =
          await tester.startGesture(fitted.center - const Offset(60, 40));
      await tester.pump();
      await gesture.moveTo(fitted.center);
      await tester.pump();
      await gesture.moveTo(fitted.center + const Offset(60, 40));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      final masks = p.layer.getMasks();
      expect(masks, hasLength(1));
      expect(masks.single.name, 'Ellipse');
      expect(masks.single.closed, isTrue);
      expect(masks.single.vertices, hasLength(4),
          reason: 'an ellipse is four cubics');
      // Drawn in layer space, so the mask sits where the drag was: the comp is
      // 1920x1080 and the drag was about its middle.
      final xs = [for (final v in masks.single.vertices) v.x];
      expect(xs.reduce((a, b) => a < b ? a : b), greaterThan(0));
      expect(xs.reduce((a, b) => a > b ? a : b), lessThan(1920));
    });

    /// The Pen with nothing selected makes a shape layer too: the same
    /// path, and the only difference is what it will belong to.
    testWidgets('the Pen with nothing selected closes onto a shape layer',
        (tester) async {
      final p = withLayer();
      p.uiState.clearSelection();
      p.uiState.tools.select(ToolMode.pen);
      await mount(tester, p);

      final fitted = fittedRect(tester, p.comp);
      final first = fitted.center;
      await tester.tapAt(first);
      await tester.pumpAndSettle();
      await tester.tapAt(first + const Offset(80, 0));
      await tester.pumpAndSettle();
      await tester.tapAt(first + const Offset(80, 60));
      await tester.pumpAndSettle();
      expect(p.comp.getLayers().length, 1, reason: 'still being drawn');

      // Clicking the first point again closes the path and applies it.
      await tester.tapAt(first);
      await tester.pumpAndSettle();

      final layers = p.comp.getLayers();
      expect(layers.length, 2);
      expect(layers.first.getKind(), BridgeLayerKind.shape);
      expect(layers.first.getShapeContents().single.vertices, hasLength(3));
      expect(p.layer.getMasks(), isEmpty,
          reason: 'the layer that was not selected was not masked');
    });

    testWidgets('the Pen places points and closes on the first one',
        (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.pen);
      await mount(tester, p);

      final fitted = fittedRect(tester, p.comp);
      final first = fitted.center - const Offset(80, 60);
      await tester.tapAt(first);
      await tester.pumpAndSettle();
      await tester.tapAt(fitted.center + const Offset(80, -60));
      await tester.pumpAndSettle();
      await tester.tapAt(fitted.center + const Offset(0, 70));
      await tester.pumpAndSettle();
      expect(p.layer.getMasks(), isEmpty,
          reason: 'an open path is a shape being drawn, not a mask yet');

      // Clicking the first point again closes it, and that is what applies it.
      await tester.tapAt(first);
      await tester.pumpAndSettle();

      final masks = p.layer.getMasks();
      expect(masks, hasLength(1));
      // Numbered, not named for the tool: every path the Pen draws is a path,
      // so the number is the only thing that tells two of them apart.
      expect(masks.single.name, 'Mask 1');
      expect(masks.single.vertices, hasLength(3));
      expect(masks.single.closed, isTrue);
    });

    testWidgets('the polygon tool drags out a five-sided mask', (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.shapePolygon);
      await mount(tester, p);

      final fitted = fittedRect(tester, p.comp);
      final gesture =
          await tester.startGesture(fitted.center - const Offset(70, 70));
      await tester.pump();
      await gesture.moveTo(fitted.center);
      await tester.pump();
      await gesture.moveTo(fitted.center + const Offset(70, 70));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      final masks = p.layer.getMasks();
      expect(masks, hasLength(1));
      expect(masks.single.name, 'Polygon');
      expect(masks.single.vertices, hasLength(5),
          reason: 'a polygon is a shape you drag out, not a path you build');
    });

    /// Mask points are editable on the picture: they draw as squares on
    /// the path, a marquee gathers them, and dragging moves them.
    testWidgets('a mask\'s points can be swept up and dragged', (tester) async {
      final p = withLayer();
      // A small rectangle mask in the middle of the comp, so its points sit
      // well inside the picture.
      p.layer.addMask(
        mask: BridgeMask(
          id: UuidValue.fromString(const Uuid().v4()),
          name: 'Rectangle',
          vertices: const [
            BridgeVertex(
                x: 860, y: 440, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 1060, y: 440, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 1060, y: 640, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 860, y: 640, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
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
      p.uiState.model.refresh();
      await mount(tester, p);

      final before = p.layer.getMasks().single.vertices;
      final fitted = fittedRect(tester, p.comp);
      // Where the mask's top-left point is on screen: layer space maps 1:1 to
      // the comp here, so it is the fitted rect scaled.
      Offset onScreen(double x, double y) => Offset(
            fitted.left + x / 1920 * fitted.width,
            fitted.top + y / 1080 * fitted.height,
          );

      // Sweep the top two points only, starting from empty space *outside* the
      // picture: a press inside a selected layer moves that layer, which is
      // what the Selection tool has always done and what After Effects
      // does. The surround is the empty part a marquee starts from.
      final panel = tester.getRect(find.byKey(const ValueKey('viewer-stage')));
      final gesture =
          await tester.startGesture(panel.topLeft + const Offset(2, 2));
      await tester.pump();
      await gesture.moveTo(onScreen(1000, 480));
      await tester.pump();
      await gesture.moveTo(onScreen(1100, 500));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
      // Nothing has moved yet — a sweep only chooses.
      expect(p.layer.getMasks().single.vertices.first.x, before.first.x);

      // Now drag one of the caught points; both should travel.
      final drag = await tester.startGesture(onScreen(860, 440));
      await tester.pump();
      // Past the framework's pan slop, which is larger than the touch slop.
      for (var i = 0; i < 10; i++) {
        await drag.moveBy(const Offset(6, 0));
        await tester.pump();
      }
      await drag.up();
      await tester.pumpAndSettle();

      final after = p.layer.getMasks().single.vertices;
      expect(after[0].x, greaterThan(before[0].x),
          reason: 'the swept top-left point moved');
      expect(after[1].x, greaterThan(before[1].x),
          reason: 'and so did the other one the sweep caught');
      expect(after[3].x, closeTo(before[3].x, 0.001),
          reason: 'the points the sweep missed stayed put');
    });

    /// **A shape layer's own art is correctable on the picture**, by the same
    /// gesture a mask's points take. Before this, art could be drawn and then
    /// only redrawn.
    testWidgets("a shape layer's points can be swept up and dragged",
        (tester) async {
      final p = withLayer();
      // A shape layer with a square of its own, in comp coordinates — the
      // layer maps 1:1 to the comp, so its points sit where the maths below
      // says they do.
      final shape = p.comp.addShapeLayer(
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
      p.uiState.setSelection([shape]);
      p.uiState.model.refresh();
      await mount(tester, p);

      final before = shape.getShapeContents().single.vertices;
      final fitted = fittedRect(tester, p.comp);
      Offset onScreen(double x, double y) => Offset(
            fitted.left + x / 1920 * fitted.width,
            fitted.top + y / 1080 * fitted.height,
          );

      // Sweep the top two points, starting from empty space outside the
      // picture — exactly as the mask case does. Art coordinates are where the
      // art is drawn: this layer's box starts at the art's own corner,
      // so a point at art (400, 200) is at composition (400, 200).
      final panel = tester.getRect(find.byKey(const ValueKey('viewer-stage')));
      final gesture =
          await tester.startGesture(panel.topLeft + const Offset(2, 2));
      await tester.pump();
      await gesture.moveTo(onScreen(500, 250));
      await tester.pump();
      await gesture.moveTo(onScreen(700, 300));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
      expect(shape.getShapeContents().single.vertices.first.x, before.first.x,
          reason: 'a sweep only chooses; nothing has moved yet');

      final drag = await tester.startGesture(onScreen(400, 200));
      await tester.pump();
      // Past the framework's pan slop, which is larger than the touch slop.
      for (var i = 0; i < 10; i++) {
        await drag.moveBy(const Offset(6, 0));
        await tester.pump();
      }
      await drag.up();
      await tester.pumpAndSettle();

      final after = shape.getShapeContents().single.vertices;
      expect(after[0].x, greaterThan(before[0].x),
          reason: 'the swept top-left point moved');
      expect(after[1].x, greaterThan(before[1].x),
          reason: 'and so did the other one the sweep caught');
      expect(after[3].x, closeTo(before[3].x, 0.001),
          reason: 'the points the sweep missed stayed put');
      expect(after[0].y, closeTo(before[0].y, 0.001),
          reason: 'a horizontal drag moves nothing vertically');
    });

    /// A layer can carry a mask *and* be a shape layer, and the two sets of
    /// points are written back by different calls. This is the case that would
    /// break if the keys naming them ever collided.
    testWidgets('a mask on a shape layer edits apart from the art',
        (tester) async {
      final p = withLayer();
      final shape = p.comp.addShapeLayer(
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
      shape.addMask(
        mask: BridgeMask(
          id: UuidValue.fromString(const Uuid().v4()),
          name: 'Mask',
          vertices: const [
            BridgeVertex(
                x: 300, y: 300, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 500, y: 300, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 500, y: 500, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
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
      p.uiState.setSelection([shape]);
      p.uiState.model.refresh();
      await mount(tester, p);

      final artBefore = shape.getShapeContents().single.vertices;
      final maskBefore = shape.getMasks().single.vertices;
      final fitted = fittedRect(tester, p.comp);
      Offset onScreen(double x, double y) => Offset(
            fitted.left + x / 1920 * fitted.width,
            fitted.top + y / 1080 * fitted.height,
          );

      // Drag one of the ART's points, at the composition coordinates it is
      // drawn at. The mask must not follow.
      final drag = await tester.startGesture(onScreen(400, 200));
      await tester.pump();
      for (var i = 0; i < 10; i++) {
        await drag.moveBy(const Offset(6, 0));
        await tester.pump();
      }
      await drag.up();
      await tester.pumpAndSettle();

      expect(shape.getShapeContents().single.vertices[0].x,
          greaterThan(artBefore[0].x),
          reason: 'the art point moved');
      final maskAfter = shape.getMasks().single.vertices;
      for (var i = 0; i < maskAfter.length; i++) {
        expect(maskAfter[i].x, closeTo(maskBefore[i].x, 0.001),
            reason: 'the mask on the same layer is a different path');
        expect(maskAfter[i].y, closeTo(maskBefore[i].y, 0.001));
      }
    });

    /// The Type tool: a click on empty picture makes a text layer where
    /// it landed, typing previews rather than writing, and ending the edit
    /// writes the document once.
    testWidgets('the Type tool makes a text layer where you click',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState
        ..setSelectedComp(comp)
        ..tools.select(ToolMode.typeHorizontal);
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();

      final fitted = fittedRect(tester, comp);
      final at = fitted.center + const Offset(40, 20);
      await tester.tapAt(at);
      await tester.pumpAndSettle();

      final layers = comp.getLayers();
      expect(layers, hasLength(1), reason: 'the click made one');
      final layer = layers.single;
      expect(layer.getText(), isNotNull, reason: 'and it is a text layer');
      expect(layer.getText()!.text, isEmpty,
          reason: 'an empty line, waiting to be typed into');

      // Where it landed, in comp pixels.
      final size = comp.getSize();
      final scale = fitted.width / size.width;
      final tf = layer.getTransform();
      double still(dynamic s) => (s as dynamic).field0 as double;
      expect(still(tf.positionX), closeTo((at.dx - fitted.left) / scale, 0.5));
      expect(still(tf.positionY), closeTo((at.dy - fitted.top) / scale, 0.5));

      // Typing does not touch the document — that is what the preview path is
      // for — and ending the edit writes it once.
      await tester.enterText(find.byType(EditableText), 'Hello');
      await tester.pump();
      expect(layer.getText()!.text, isEmpty,
          reason: 'still previewing; the document is untouched');

      p.uiState.tools.select(ToolMode.select);
      await tester.pumpAndSettle();
      expect(layer.getText()!.text, 'Hello',
          reason: 'putting the tool down ends the edit and writes it');
    });

    /// **Two undo steps for a whole typing session, and no more.**
    ///
    /// Making the layer used to be three ops and finishing the edit two more,
    /// so `Ctrl+Z` walked back through states nobody had ever seen: an empty
    /// box, then the word "Text", then at last the layer going away. Making it
    /// is one step now and typing into it is another, so the first undo takes
    /// back what was typed and the very next one removes the layer.
    testWidgets('a typed layer undoes in two steps: the words, then the layer',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState
        ..setSelectedComp(comp)
        ..tools.select(ToolMode.typeHorizontal);
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();

      await tester.tapAt(fittedRect(tester, comp).center);
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(EditableText), 'Title');
      await tester.pump();
      p.uiState.tools.select(ToolMode.select);
      await tester.pumpAndSettle();
      expect(comp.getLayers().single.getText()!.text, 'Title');

      p.state.project!.undo();
      expect(comp.getLayers(), hasLength(1),
          reason: 'the first undo is the typing, not the layer');
      expect(comp.getLayers().single.getText()!.text, isEmpty);

      p.state.project!.undo();
      expect(comp.getLayers(), isEmpty,
          reason: 'and the very next one removes the layer, whole');
    });

    testWidgets('a Type click with nothing typed leaves no layer behind',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState
        ..setSelectedComp(comp)
        ..tools.select(ToolMode.typeHorizontal);
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();

      await tester.tapAt(fittedRect(tester, comp).center);
      await tester.pumpAndSettle();
      expect(comp.getLayers(), hasLength(1));

      p.uiState.tools.select(ToolMode.select);
      await tester.pumpAndSettle();
      expect(comp.getLayers(), isEmpty,
          reason: 'a stray click must not leave an empty text layer');
    });

    testWidgets('the Type tool edits the text layer you click on',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addTextLayer();
      p.uiState
        ..setSelectedComp(comp)
        ..tools.select(ToolMode.typeHorizontal);
      p.uiState.model.refresh();
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();

      await tester.tapAt(fittedRect(tester, comp).center);
      await tester.pumpAndSettle();
      expect(comp.getLayers(), hasLength(1),
          reason: 'clicking an existing text layer edits it rather than '
              'making another');
      expect(find.byType(EditableText), findsOneWidget);
      expect(
          tester
              .widget<EditableText>(find.byType(EditableText))
              .controller
              .text,
          'Text',
          reason: 'seeded with what it says');

      await tester.enterText(find.byType(EditableText), 'Retitled');
      await tester.pump();
      p.uiState.tools.select(ToolMode.select);
      await tester.pumpAndSettle();
      expect(layer.getText()!.text, 'Retitled');
    });

    /// Painting: a drag on the selected layer leaves a stroke, and one
    /// drag is one stroke and one undo step.
    testWidgets('a brush drag paints a stroke on the selected layer',
        (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.brush);
      await mount(tester, p);

      expect(find.byType(ViewerPaintLayer), findsOneWidget);
      // The hardware crosshair leads — the overlay asks the platform
      // for the precise pointer instead of hiding it, so aiming happens at
      // input rate however slowly the application is repainting. The ring is
      // decoration.
      expect(
        tester
            .widget<DrawnPointerRegion>(find.descendant(
                of: find.byType(ViewerPaintLayer),
                matching: find.byType(DrawnPointerRegion)))
            .cursor,
        SystemMouseCursors.precise,
      );
      expect(p.layer.getPaint(), isEmpty);

      final fitted = fittedRect(tester, p.comp);
      final gesture = await tester.startGesture(fitted.center);
      await tester.pump();
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(10, 4));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final strokes = p.layer.getPaint();
      expect(strokes, hasLength(1), reason: 'one drag, one stroke');
      expect(strokes.single.name, startsWith('Brush'));
      expect(strokes.single.mode, BridgePaintMode.paint);
      expect(strokes.single.points.length, greaterThan(1),
          reason: 'the path the pointer took, not one dab');
      expect(strokes.single.width, p.uiState.tools.brushSize);

      // The stroke is in layer coordinates: the middle of the picture is the
      // middle of a comp-sized layer.
      final size = p.comp.getSize();
      expect(strokes.single.points.first.x,
          closeTo(size.width / 2, size.width * 0.02));

      p.state.project!.undo();
      p.uiState.model.refresh();
      expect(p.layer.getPaint(), isEmpty, reason: 'one undo step');
    });

    /// The brush shape chosen in the tool options is the shape the stroke is
    /// committed with. Round unless somebody picks otherwise, so a
    /// project painted before there was a choice reads back the way it was.
    testWidgets('the brush commits the shape chosen in the tool options',
        (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.brush);
      await mount(tester, p);

      final fitted = fittedRect(tester, p.comp);
      await tester.tapAt(fitted.center);
      await tester.pumpAndSettle();
      expect(p.layer.getPaint().single.shape, BridgeBrushShape.round,
          reason: 'the shape everything was painted with before');

      p.uiState.tools.brushShape = BridgeBrushShape.square;
      await tester.pumpAndSettle();
      await tester.tapAt(fitted.center + const Offset(0, 30));
      await tester.pumpAndSettle();
      expect(p.layer.getPaint().last.shape, BridgeBrushShape.square);
      expect(p.layer.getPaint().first.shape, BridgeBrushShape.round,
          reason:
              'and the one already painted keeps the shape it was made with');
    });

    /// A stylus's pressure rides in with the points and widens the mark, and
    /// the brush's own toggle turns that off, which stores a full press
    /// throughout and so the stroke a mouse would have made.
    testWidgets('a stylus drag commits the pressure it was drawn with',
        (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.brush);
      await mount(tester, p);
      final fitted = fittedRect(tester, p.comp);

      // The drag callbacks carry no pressure, so the events are made by hand:
      // this is exactly the stream a pen tablet raises.
      Future<void> penDrag(Offset from, List<double> presses) async {
        var at = from;
        tester.binding.handlePointerEvent(PointerDownEvent(
            pointer: 8,
            kind: PointerDeviceKind.stylus,
            position: at,
            pressure: presses.first,
            pressureMin: 0,
            pressureMax: 1));
        await tester.pump();
        for (final press in presses.skip(1)) {
          at += const Offset(12, 0);
          tester.binding.handlePointerEvent(PointerMoveEvent(
              pointer: 8,
              kind: PointerDeviceKind.stylus,
              position: at,
              delta: const Offset(12, 0),
              pressure: press,
              pressureMin: 0,
              pressureMax: 1));
          await tester.pump();
        }
        tester.binding.handlePointerEvent(PointerUpEvent(
            pointer: 8, kind: PointerDeviceKind.stylus, position: at));
        await tester.pumpAndSettle();
      }

      const presses = <double>[1, 0.9, 0.75, 0.5, 0.3, 0.2];
      await penDrag(fitted.center, presses);
      final pressed = p.layer.getPaint().single.points;
      expect(pressed.last.pressure, closeTo(0.2, 1e-6),
          reason: 'the press the pen ended on');
      expect(pressed.any((point) => point.pressure < 0.5), isTrue,
          reason: 'and the lighter part of the gesture came across');

      // With the toggle off the same pen leaves the stroke a mouse would: a
      // full press at every point, which the engine stores as none at all.
      p.uiState.tools.brushPressureSize = false;
      await tester.pumpAndSettle();
      await penDrag(fitted.center + const Offset(0, 40), presses);
      expect(
        p.layer.getPaint().last.points.map((point) => point.pressure),
        everyElement(1.0),
      );
    });

    testWidgets('the eraser and the clone stamp commit their own modes',
        (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.eraser);
      await mount(tester, p);
      final fitted = fittedRect(tester, p.comp);

      Future<void> paintAt(Offset from) async {
        final gesture = await tester.startGesture(from);
        await tester.pump();
        for (var i = 0; i < 6; i++) {
          await gesture.moveBy(const Offset(9, 0));
          await tester.pump();
        }
        await gesture.up();
        await tester.pumpAndSettle();
      }

      await paintAt(fitted.center);
      expect(p.layer.getPaint().single.mode, BridgePaintMode.erase);

      // The clone stamp refuses to stamp until it has been given a source.
      p.uiState.tools.select(ToolMode.cloneStamp);
      await tester.pump();
      await paintAt(fitted.center + const Offset(0, 40));
      expect(p.layer.getPaint(), hasLength(1),
          reason: 'no source yet, so nothing was stamped');
      expect(p.state.notice.value?.message, contains('clone source'));

      // Alt-click sets it, and then the stroke lands with the offset it implies.
      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.tapAt(fitted.center - const Offset(80, 0));
      await tester.pumpAndSettle();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await paintAt(fitted.center + const Offset(0, 40));

      final strokes = p.layer.getPaint();
      expect(strokes, hasLength(2));
      expect(strokes.last.mode, BridgePaintMode.clone);
      expect(strokes.last.cloneOffsetX, lessThan(0),
          reason: 'the source was to the left of where the stroke began');
    });

    testWidgets('painting with nothing selected says what to do instead',
        (tester) async {
      final p = withLayer();
      p.uiState.clearSelection();
      p.uiState.tools.select(ToolMode.brush);
      await mount(tester, p);

      final gesture =
          await tester.startGesture(fittedRect(tester, p.comp).center);
      await tester.pump();
      for (var i = 0; i < 6; i++) {
        await gesture.moveBy(const Offset(9, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      expect(p.layer.getPaint(), isEmpty);
      expect(
          p.state.notice.value?.message, contains('Select a layer to paint'));
    });

    /// The other half of the shape tools' gesture: with nothing
    /// selected they make a **shape layer** rather than saying they cannot.
    testWidgets('a shape drag with nothing selected makes a shape layer',
        (tester) async {
      final p = withLayer();
      p.uiState.clearSelection();
      p.uiState.tools.select(ToolMode.shapeRectangle);
      await mount(tester, p);

      final before = p.comp.getLayers().length;
      final fitted = fittedRect(tester, p.comp);
      final gesture = await tester.startGesture(fitted.center);
      await tester.pump();
      await gesture.moveBy(const Offset(40, 30));
      await tester.pump();
      await gesture.moveBy(const Offset(40, 30));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      final layers = p.comp.getLayers();
      expect(layers.length, before + 1, reason: 'one drag, one shape layer');
      final shape = layers.first;
      expect(shape.getKind(), BridgeLayerKind.shape,
          reason: 'and it is at the top of the stack');
      final contents = shape.getShapeContents();
      expect(contents, hasLength(1));
      expect(contents.single.name, 'Rectangle');
      expect(contents.single.vertices, hasLength(4));
      expect(contents.single.fill, isNotNull,
          reason: "it takes the toolbar's fill");

      // The art lands where it was drawn: the drag began at the middle of the
      // picture, so the layer's position is the middle of the comp.
      final size = p.comp.getSize();
      double still(dynamic s) => (s as dynamic).field0 as double;
      expect(still(shape.getTransform().positionX),
          closeTo(size.width / 2, size.width * 0.02));

      // And the new layer is what is selected, so the next drag masks it.
      expect(p.uiState.selectedLayerIds, contains(shape.internallayerId));
    });

    /// Undo a shape layer and the next drag must draw another one.
    ///
    /// It did not. Making a shape layer *selects* it, so the next drag masks
    /// it — the gesture's whole point. Undo then removed the layer but left its
    /// id in the selection, so the tool still believed a layer was selected and
    /// tried to add a mask to one that no longer existed. The engine refused,
    /// the refusal was swallowed, and the drag did nothing at all.
    testWidgets('a shape can be drawn again after undoing the last one',
        (tester) async {
      final p = withLayer();
      p.uiState.clearSelection();
      p.uiState.tools.select(ToolMode.shapeRectangle);
      await mount(tester, p);

      final before = p.comp.getLayers().length;
      final fitted = fittedRect(tester, p.comp);
      Future<void> drawAt(Offset centre) async {
        final gesture = await tester.startGesture(centre);
        await tester.pump();
        await gesture.moveBy(const Offset(40, 30));
        await tester.pump();
        await gesture.moveBy(const Offset(40, 30));
        await tester.pump();
        await gesture.up();
        await tester.pumpAndSettle();
      }

      await drawAt(fitted.center);
      expect(p.comp.getLayers().length, before + 1);

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      expect(p.comp.getLayers().length, before,
          reason: 'the undo took the shape layer back');

      await drawAt(fitted.center - const Offset(30, 20));
      expect(p.comp.getLayers().length, before + 1,
          reason: 'the next drag draws another shape layer, and does not try '
              'to mask the one the undo removed');
      expect(p.comp.getLayers().first.getKind(), BridgeLayerKind.shape);
    });

    testWidgets('a shape layer takes the toolbar\'s stroke when it has a width',
        (tester) async {
      final p = withLayer();
      p.uiState.clearSelection();
      p.uiState.tools
        ..select(ToolMode.shapeEllipse)
        ..strokeWidth = 6;
      await mount(tester, p);

      final fitted = fittedRect(tester, p.comp);
      final gesture = await tester.startGesture(fitted.center);
      await tester.pump();
      await gesture.moveBy(const Offset(50, 40));
      await tester.pump();
      await gesture.moveBy(const Offset(30, 20));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();

      final item = p.comp.getLayers().first.getShapeContents().single;
      expect(item.stroke, isNotNull);
      expect(item.strokeWidth, 6);

      // With no width there is no outline to draw. (The selection is cleared
      // first: the layer just made is selected, so another drag would mask it
      // rather than make a second shape layer.)
      p.uiState.clearSelection();
      p.uiState.tools.strokeWidth = 0;
      await tester.pump();
      final second =
          await tester.startGesture(fitted.topLeft + const Offset(20, 20));
      await tester.pump();
      await second.moveBy(const Offset(40, 40));
      await tester.pump();
      await second.moveBy(const Offset(20, 20));
      await tester.pump();
      await second.up();
      await tester.pumpAndSettle();
      expect(p.comp.getLayers().first.getShapeContents().single.stroke, isNull);
    });

    /// The camera tools: a drag moves the composition's active camera,
    /// and with no camera the tool says so rather than swallowing the gesture.
    testWidgets('the camera tools orbit, track and dolly the active camera',
        (tester) async {
      final p = withLayer();
      final camera = p.comp.addCameraLayer();
      p.uiState.model.refresh();
      p.uiState.tools.select(ToolMode.cameraOrbit);
      await mount(tester, p);

      double still(dynamic s) => (s as dynamic).field0 as double;
      final before = camera.getTransform();
      final centre = fittedRect(tester, p.comp).center;

      Future<void> drag(Offset by) async {
        final gesture = await tester.startGesture(centre);
        await tester.pump();
        for (var i = 0; i < 6; i++) {
          await gesture.moveBy(by / 6);
          await tester.pump();
        }
        await gesture.up();
        await tester.pumpAndSettle();
      }

      // Orbit: the rotations change and the point being looked at does not.
      await drag(const Offset(120, 0));
      var after = camera.getTransform();
      expect(still(after.rotationY), isNot(still(before.rotationY)));
      expect(still(after.positionX), closeTo(still(before.positionX), 0.001),
          reason: 'an orbit swings round what the camera looks at');

      // Track: the position moves, the rotations do not.
      p.uiState.tools.select(ToolMode.cameraPan);
      await tester.pump();
      final beforeTrack = camera.getTransform();
      await drag(const Offset(0, 90));
      after = camera.getTransform();
      expect(still(after.positionY), isNot(still(beforeTrack.positionY)));
      expect(
          still(after.rotationY), closeTo(still(beforeTrack.rotationY), 1e-9));

      // Dolly: it moves along the view axis.
      p.uiState.tools.select(ToolMode.cameraDolly);
      await tester.pump();
      final beforeDolly = camera.getTransform();
      await drag(const Offset(150, 0));
      after = camera.getTransform();
      expect(still(after.positionZ), greaterThan(still(beforeDolly.positionZ)));
    });

    testWidgets('a camera drag with no camera says what to do instead',
        (tester) async {
      final p = withLayer();
      p.uiState.tools.select(ToolMode.cameraOrbit);
      await mount(tester, p);

      await tester.tapAt(fittedRect(tester, p.comp).center);
      await tester.pumpAndSettle();
      expect(p.state.notice.value?.message, contains('add a camera layer'));
    });

    testWidgets('a missing footage layer raises the badge', (tester) async {
      final p = withLayer();
      final gone = p.state.project!.importFootage(path: 'C:/nowhere/gone.mp4');
      p.comp.addFootageLayer(footage: gone, asSequence: false);
      await mount(tester, p);

      await settleFrb(
        tester,
        until: () =>
            find.byKey(const ValueKey('viewer-missing')).evaluate().isNotEmpty,
      );
      expect(find.byKey(const ValueKey('viewer-missing')), findsOneWidget);
      expect(find.textContaining('missing file'), findsOneWidget);
    });
    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.

    /// Silence must never stop the picture: on a machine with no sound device
    /// the transport still runs, on the wall clock.
    testWidgets('playback works without a sound device', (tester) async {
      final p = withLayer();
      await mount(tester, p);

      await pressBar(tester, 'viewer-play');
      await tester.pump();
      await settleFrb(tester,
          minRounds: 6,
          maxRounds: coldWorkerRounds,
          until: () => p.uiState.playheadFrame.value > 0);

      expect(p.uiState.playheadFrame.value, greaterThan(0),
          reason:
              'no audio device, so the engine falls back to its wall clock');

      await pressBar(tester, 'viewer-play');
      await tester.pump();
      expect(audioClock().playing, isFalse,
          reason: 'pausing the transport pauses the sound too');
    }, skip: zeroCopyViewerUnavailable);

    /// The shell's space bar drives the transport through LumitUiState, so the
    /// key is a quiet no-op when no Viewer is mounted.
    testWidgets('the transport request from the shell starts and stops it',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      p.uiState.requestTogglePlay();
      await tester.pump();
      await settleFrb(tester,
          minRounds: 6,
          maxRounds: coldWorkerRounds,
          until: () => p.uiState.playheadFrame.value > 0);
      expect(p.uiState.playheadFrame.value, greaterThan(0),
          reason: 'space started playback');

      p.uiState.requestTogglePlay();
      await tester.pump();
      await settleFrb(tester, minRounds: 4, maxRounds: 4);
      final stopped = p.uiState.playheadFrame.value;
      await settleFrb(tester, minRounds: 8, maxRounds: 8);
      expect(p.uiState.playheadFrame.value, stopped,
          reason: 'and space stopped it');
    }, skip: zeroCopyViewerUnavailable);

    /// A transport belongs under the picture. Asserted by position rather than
    /// by reading the widget tree's shape, because what matters is where the
    /// user's eye and pointer go.
    testWidgets('the transport sits below the picture', (tester) async {
      final p = withLayer();
      await mount(tester, p);

      final play = tester.getCenter(find.byKey(const ValueKey('viewer-play')));
      final stage = tester.getRect(find.byType(ViewerPanelFrb));
      expect(play.dy, greaterThan(stage.center.dy),
          reason: 'below the middle of the panel, not above it');
    });

    /// Moving the playhead from anywhere must repaint the Viewer. Only the
    /// Viewer's own transport used to render, so dragging the Timeline's
    /// playhead — or pressing an arrow key — moved the playhead and left the
    /// picture on the old frame.
    testWidgets('a playhead move from outside the Viewer renders',
        (tester) async {
      final p = withLayer();
      final sub = p.state.onWorkerResponse.listen((_) {});
      addTearDown(sub.cancel);
      await mount(tester, p);

      // Exactly what the Timeline ruler and the arrow keys do: set it.
      final before = p.uiState.frameArrived.value;
      p.uiState.playheadFrame.value = 12;
      await tester.pump();

      // The first render of a session also builds the renderer, so allow for
      // that before asserting anything about the picture. Frames arrive as
      // shared-texture handles; in a widget test the platform channel
      // has no handler so no texture registers, but every arrival still bumps
      // `frameArrived` — which is the fact being asserted.
      await settleFrb(
        tester,
        until: () => p.uiState.frameArrived.value > before,
        minRounds: 10,
        maxRounds: coldWorkerRounds,
      );
      expect(p.uiState.frameArrived.value, greaterThan(before),
          reason: 'a frame was rendered for the moved playhead');
    }, skip: zeroCopyViewerUnavailable);

    /// A still Viewer must go quiet. While the in-flight rule was being built
    /// it re-asked for the frame it had just been given, so the engine rendered
    /// the same picture over and over for as long as the panel was open.
    /// Scroll-zoom (docs/07 §2.2): the wheel leans the picture in about the
    /// cursor. Observable through the scale the Viewer reports to the engine —
    /// zooming in shows more comp pixels per screen pixel, so it rises — and
    /// through the picker showing a true percentage between its steps.
    testWidgets('the wheel zooms the picture about the cursor', (tester) async {
      final p = withLayer();
      // Auto, because this reads the magnification through the preview scale
      // and only Auto follows the panel; Full is the default.
      p.uiState.setPreviewResolution(PreviewResolution.auto);
      await mount(tester, p);

      final before = p.uiState.viewerScale;
      final centre = tester.getCenter(find.byType(ViewerPanelFrb));
      final pointer = TestPointer(1, PointerDeviceKind.mouse);
      pointer.hover(centre);
      // Three notches in.
      for (var i = 0; i < 3; i++) {
        await tester.sendEventToBinding(pointer.scroll(const Offset(0, -120)));
        await tester.pump();
      }

      expect(p.uiState.viewerScale, greaterThan(before),
          reason: 'zooming in raises the on-screen fraction of the comp');
      // The picker tells the truth about a zoom between its steps.
      expect(find.textContaining('%'), findsWidgets);

      // And back out well past fit. The picture really does get smaller — and
      // the resolution the engine is asked for does **not** follow it down:
      // zooming out means "let me see more of it", not "make it coarser", and
      // lowering it threw away every cached frame to do so.
      for (var i = 0; i < 8; i++) {
        await tester.sendEventToBinding(pointer.scroll(const Offset(0, 120)));
        await tester.pump();
      }
      expect(shownZoom(tester), isNotNull);
      expect(shownZoom(tester)!, lessThan(before));
      expect(p.uiState.viewerScale, closeTo(before, 1e-9));
    });

    testWidgets('a still playhead stops asking for renders', (tester) async {
      final p = withLayer();
      await mount(tester, p);

      var frames = 0;
      final sub = p.state.onWorkerResponse.listen((msg) {
        // **A published picture, and nothing else.** The idle cache fill is
        // SUPPOSED to work while the playhead is still and announces
        // each banked frame; what must go quiet is the PICTURE being
        // re-rendered and re-published.
        //
        // This used to count every message that was not a `CacheFilled`, which
        // has not meant "a render" for some time: one render also reports its
        // progress (docs/13 §7.1) and is measured an idle turn AFTER it
        // was served. Those arrive around the picture rather than with
        // it, so whether a trailing one landed before or after the count was
        // taken decided the test — and the count it was compared against was
        // taken the moment `frameArrived` bumped, which is the middle of that
        // spread. Counting the publish itself is both stabler and stricter: a
        // second picture is exactly the regression, and now nothing else can
        // stand in for one.
        if (msg is WorkerResponse_RenderedSharedTexture ||
            msg is WorkerResponse_RenderedDMABuf) {
          frames++;
        }
      });
      addTearDown(sub.cancel);

      // Let the mount render land, then count what follows it.
      await settleFrb(tester,
          minRounds: 10,
          maxRounds: coldWorkerRounds,
          until: () => p.uiState.frameArrived.value > 0);
      final settled = frames;

      await settleFrb(tester, minRounds: 20, maxRounds: 20);
      expect(frames, settled,
          reason: 'nothing moved, so nothing should have been rendered');
    });

    /// **The stale-picture regression.** The Viewer asked for a frame when the
    /// playhead moved and at no other time, so an edit made with the playhead
    /// still — typing an opacity, adding an effect, anything another panel
    /// commits — left the old picture on screen until something moved the
    /// playhead. Playing was the usual accident that fixed it, which is exactly
    /// how it was reported: "the Viewer does not update until I play".
    testWidgets('an edit with the playhead still redraws the picture',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);
      // The mount's own picture, which is a cold worker's first frame.
      await settleFrb(tester,
          minRounds: 10,
          maxRounds: coldWorkerRounds,
          until: () => p.uiState.frameArrived.value > 0);
      final before = p.uiState.frameArrived.value;
      final playhead = p.uiState.playheadFrame.value;

      p.layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: const BridgeScalar.static_(25),
      );
      await settleFrb(tester,
          minRounds: 8, until: () => p.uiState.frameArrived.value > before);

      expect(p.uiState.frameArrived.value, greaterThan(before),
          reason: 'the edit asked for the picture again');
      expect(p.uiState.playheadFrame.value, playhead,
          reason: 'and did it without moving the playhead to force it');
    }, skip: zeroCopyViewerUnavailable);

    /// Pressing play with the playhead already at the end used to do nothing at
    /// all: the clock read past the end on its first tick, so it stopped again
    /// immediately, and every-frame's pump had no frame left to ask for. The
    /// rewind is the engine's now — it is the half that knows where the end is.
    testWidgets('play from the end starts from the beginning', (tester) async {
      final p = withLayer();
      await mount(tester, p);
      final last = p.comp.durationFrames() - 1;
      p.uiState.playheadFrame.value = last;
      await tester.pump();

      p.uiState.requestTogglePlay();
      await tester.pump();
      await settleFrb(tester,
          minRounds: 6,
          maxRounds: coldWorkerRounds,
          until: () => p.uiState.playheadFrame.value < last);

      expect(p.uiState.playheadFrame.value, lessThan(100),
          reason: 'it rewound rather than sitting at the end doing nothing');
    }, skip: zeroCopyViewerUnavailable);

    /// Every-frame plays WITH sound now: audio plays while rendering holds the
    /// comp's rate, and the worker pauses it if the picture falls genuinely
    /// behind (it used to be silenced outright).
    /// Headless there is no output device or mix, so what is asserted is the
    /// seam: play in every-frame starts cleanly, the clock stays readable,
    /// and stopping silences whatever there was.
    testWidgets('every-frame playback starts the sound like adaptive',
        (tester) async {
      final p = withLayer();
      p.uiState.workspace.performance.playback = PlaybackMode.everyFrame;
      await mount(tester, p);

      await pressBar(tester, 'viewer-play');
      await tester.pump();
      expect(audioClock().seconds, greaterThanOrEqualTo(0),
          reason: 'the sound path engaged without a fault');

      await pressBar(tester, 'viewer-play');
      await tester.pump();
      expect(audioClock().playing, isFalse, reason: 'stop silences it');
    });

    testWidgets('stepping takes the sound with it', (tester) async {
      final p = withLayer();
      await mount(tester, p);

      // The seek must not throw whatever the device situation is — it is on the
      // path of every arrow key.
      await pressBar(tester, 'viewer-step-forward');
      await tester.pump();
      expect(p.uiState.playheadFrame.value, 1);
      expect(audioClock().seconds, greaterThanOrEqualTo(0));
    });

    /// **LAST in this file**: `openProject` clears the engine's project
    /// registry, so every reference an earlier test holds dies here.
    ///
    /// The missing-file badge probes each footage layer over the bridge, one
    /// round trip each, and those answers can still be in flight when another
    /// document replaces the one they were asked about — which is exactly what
    /// opening a project does. Unguarded, the probe threw `InvalidProject` into
    /// nobody's hands and the console filled with an unhandled exception.
    testWidgets('a footage probe survives the document being replaced',
        (tester) async {
      final dir = Directory.systemTemp.createTempSync('lumit-viewer-swap');
      final other = '${dir.path}/other.lum';

      final p = withLayer();
      // Enough layers that the probe loop — one bridge round trip per layer,
      // in order — is still working through them when the open lands.
      for (var i = 0; i < 30; i++) {
        final gone =
            p.state.project!.importFootage(path: 'C:/nowhere/gone$i.mp4');
        p.comp.addFootageLayer(footage: gone, asSequence: false);
      }
      p.state.project!.save(path: other);
      await settleFrb(tester, until: () => File(other).existsSync());

      await mount(tester, p);
      // Straight into the open, with the badge's probes unanswered: the
      // registry is cleared while they are on the wire.
      final adopted = p.state.project;
      p.state.openProject(other);
      await settleFrb(tester,
          until: () => !identical(p.state.project, adopted));

      expect(tester.takeException(), isNull);
    });
  }, skip: !engineAvailable);
}
