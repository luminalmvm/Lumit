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

@Tags(['opens-project'])
library;

import 'dart:io';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/lumit_icon.dart' as glyph;
import 'package:lumit_flutter/icons/lumit_icons.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/panels/viewer_gizmo.dart';
import 'package:lumit_flutter/panels/viewer_camera.dart' show CameraPose;
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
    view: 0,
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

    /// Open the bottom bar's view menu and choose the row [key].
    Future<void> pickViewRow(WidgetTester tester, String key) async {
      await pressBar(tester, 'viewer-guides-menu');
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(ValueKey<String>(key)));
      await tester.pumpAndSettle();
    }

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

    /// A six-frame comp with its work area on frames 1 to 4, so a loop mode
    /// has an end to reach inside a test. A tenth of a second, so the end
    /// arrives inside a test rather than in the thirty seconds a default comp
    /// lasts.
    void shortWorkArea(dynamic p) {
      final comp = p.comp as CompositionReference;
      final was = comp.getSettings();
      comp.setSettings(
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
      comp.setWorkArea(
        span: BridgeSpan(
          inPoint: comp.timeOfFrame(frame: 1),
          outPoint: comp.timeOfFrame(frame: 4),
          startOffset: const BridgeRational(num: 0, den: 1),
        ),
      );
    }

    /// Running off the end is the engine's to notice: it knows the length and it
    /// is the one counting. The frontend is *told*, and that is the only reason
    /// its transport goes back to showing a play button. The playhead is parked
    /// past the work area, which is the one run that does not loop.
    testWidgets('playback ends on its own at the end of the composition',
        (tester) async {
      final p = withLayer();
      shortWorkArea(p);
      await mount(tester, p);
      expect(p.comp.durationFrames(), 6, reason: '0.1 s at 60 fps');
      p.uiState.playheadFrame.value = 5;
      await tester.pump();

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

    testWidgets(
        'the mute mark silences the output and shows the muted speaker',
        (tester) async {
      final p = withLayer();
      p.uiState.workspace.interface.viewerBars = ViewerBars.deck;
      await mount(tester, p);

      String glyphUnder(String key) => tester
          .widget<glyph.LumitIcon>(find.descendant(
            of: find.byKey(ValueKey<String>(key)),
            matching: find.byType(glyph.LumitIcon),
          ))
          .glyph;

      expect(audioMuted(), isFalse);
      expect(glyphUnder('viewer-mute'), LumitIcons.audio);

      await pressBar(tester, 'viewer-mute');
      expect(audioMuted(), isTrue, reason: 'the engine was told');
      expect(p.uiState.audioMuted.value, isTrue);
      expect(glyphUnder('viewer-mute'), LumitIcons.muted);

      await pressBar(tester, 'viewer-mute');
      expect(audioMuted(), isFalse);
      expect(glyphUnder('viewer-mute'), LumitIcons.audio);
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

    /// **A keyed position draws its motion path, and its keys drag there**
    /// (docs/07 §2.4). The body of such a layer still does not drag — a curve
    /// has no single value for a drag to add to — but its box is drawn at the
    /// playhead's value, so it can be picked, and a press on a key's box on
    /// the path moves that key: one op, one undo step.
    testWidgets('a keyed position draws its path, and a key drags as one step',
        (tester) async {
      final p = withLayer();
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
      await tester.pumpAndSettle();

      // The path is on the picture: the selected layer's box carries it.
      final painter = tester
          .widget<CustomPaint>(find.byKey(const ValueKey('viewer-gizmo')))
          .painter as dynamic;
      final drawn = painter.motionPaths as List<LayerBox>;
      expect(drawn.where((b) => b.motionPath != null), hasLength(1),
          reason: 'a keyed position draws its motion path');
      expect(drawn.single.motionPath!.keys, hasLength(2));

      // A body drag still leaves the curve alone.
      final stage = find.byType(ViewerPanelFrb);
      var gesture = await tester.startGesture(tester.getCenter(stage));
      await tester.pump();
      await gesture.moveBy(const Offset(40, 0));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
      final held = p.layer.getTransform().positionX as BridgeScalar_Keyframed;
      expect(held.field0.map((k) => k.value), [0, 400],
          reason: 'a curve is not overwritten by a drag it never accepted');

      // Dragging the last key's box moves that key. It sits at (400, y) in
      // comp pixels, on the picture's own placement.
      final fitted = fittedRect(tester, p.comp);
      final scale = fitted.width / p.comp.getSize().width;
      final y = (p.layer.getTransform().positionY as BridgeScalar_Static).field0;
      final dot = fitted.topLeft + Offset(400 * scale, y * scale);
      // One long first move, as the body drag above makes: a touch pan is
      // recognised at twice the touch slop, and a slower start hands the
      // arena to a neighbour before the gizmo's pan can claim it.
      gesture = await tester.startGesture(dot);
      await tester.pump();
      await gesture.moveBy(const Offset(40, 0));
      await tester.pump();
      for (var i = 0; i < 2; i++) {
        await gesture.moveBy(const Offset(10, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pumpAndSettle();

      final moved = p.layer.getTransform().positionX as BridgeScalar_Keyframed;
      expect(moved.field0[1].value, closeTo(400 + 60 / scale, 0.5),
          reason: 'the key followed the pointer at the picture\'s scale');
      expect(moved.field0[0].value, 0, reason: 'the other key stayed');
      expect(p.layer.getTransform().positionY, isA<BridgeScalar_Static>(),
          reason: 'an axis with no key there is not written');

      p.state.project!.undo();
      final undone = p.layer.getTransform().positionX as BridgeScalar_Keyframed;
      expect(undone.field0[1].value, 400,
          reason: 'one undo puts the whole drag back');
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

      // The pose the tools work in: the layer's position is the **eye**, and
      // the pivot is the point `zoom` in front of it (docs/impl/camera.md §4).
      CameraPose poseOf(BridgeTransform tf) => CameraPose(
            position: (
              still(tf.positionX),
              still(tf.positionY),
              still(tf.positionZ)
            ),
            rotation: (
              still(tf.rotationX),
              still(tf.rotationY),
              still(tf.rotation)
            ),
            zoom: still(tf.camera!.zoom),
          );

      // Orbit: the rotations change, the eye swings, and what the camera is
      // looking at stays exactly where it was.
      await drag(const Offset(120, 0));
      var after = camera.getTransform();
      expect(still(after.rotationY), isNot(still(before.rotationY)));
      expect(poseOf(after).pivot.$1, closeTo(poseOf(before).pivot.$1, 0.001),
          reason: 'an orbit swings round what the camera looks at');
      expect(still(after.positionX), isNot(still(before.positionX)),
          reason: 'and the eye is what moved');

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

    testWidgets('relinking the missing footage clears the badge',
        (tester) async {
      final p = withLayer();
      final gone = p.state.project!.importFootage(path: 'C:/nowhere/gone.mp4');
      p.comp.addFootageLayer(footage: gone, asSequence: false);
      await mount(tester, p);
      final badge = find.byKey(const ValueKey('viewer-missing'));
      await settleFrb(tester, until: () => badge.evaluate().isNotEmpty);
      expect(badge, findsOneWidget);

      // The layers stay the same, only the file behind one of them changes.
      gone.relink(path: _silentWavFile());
      await settleFrb(tester, until: () => badge.evaluate().isEmpty);
      expect(badge, findsNothing);
    });
    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.

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

/// A tenth of a second of silent 8-bit mono WAV, which the engine probes as
/// real media.
String _silentWavFile() {
  final file = File(
      '${Directory.systemTemp.createTempSync('lumit-relink').path}/back.wav');
  final out = BytesBuilder();
  void u16(int v) => out.add([v & 0xff, v >> 8]);
  void u32(int v) =>
      out.add([v & 0xff, v >> 8 & 0xff, v >> 16 & 0xff, v >> 24]);
  out.add('RIFF'.codeUnits);
  u32(36 + 800);
  out.add('WAVEfmt '.codeUnits);
  u32(16);
  u16(1); // PCM
  u16(1); // mono
  u32(8000);
  u32(8000);
  u16(1);
  u16(8);
  out.add('data'.codeUnits);
  u32(800);
  out.add(List.filled(800, 128));
  file.writeAsBytesSync(out.takeBytes());
  return file.path;
}
