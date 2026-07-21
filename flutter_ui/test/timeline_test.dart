// Phase-F3 Timeline tests: the pure geometry / degradation / snapping /
// glyph-coding / work-area / search / pan logic (no widget tree), and widget
// tests over the live panel driven by a fake DocumentBridge (comp tabs, layer
// rows, switch/scrub/select/trim wiring, twirl-down property rows, stopwatch,
// keyframe navigator, keyframe-lane drag, the layer context menu, the layer
// search filter and the work-area edge drag).

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/bridge/bridge.dart';
import 'package:lumit_flutter/panels/timeline/key_glyph.dart';
import 'package:lumit_flutter/panels/timeline/key_nav.dart';
import 'package:lumit_flutter/panels/timeline/lane_scale.dart';
import 'package:lumit_flutter/panels/timeline/lane_selection.dart';
import 'package:lumit_flutter/panels/timeline/outline_layout.dart';
import 'package:lumit_flutter/panels/timeline/ruler.dart';
import 'package:lumit_flutter/panels/timeline/search.dart';
import 'package:lumit_flutter/panels/timeline/work_area.dart';
import 'package:lumit_flutter/panels/timeline_panel.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

/// Two comps ("Scene" with two layers + a marker + a work area, "Titles" empty)
/// in the exact shape the Rust bridge emits. l0 (hero) carries a transform
/// read-back with an animated position_x (keys at frames 60 and 120).
const _twoCompJson = '''
{
  "ok": true,
  "items": [
    {
      "id": "c0", "name": "Scene", "kind": "composition", "children": [],
      "comp": {
        "width": 1920, "height": 1080,
        "fps": {"num": 60, "den": 1}, "frame_count": 300,
        "work_area": [30, 180],
        "layers": [
          {
            "id": "l0", "index": 0, "name": "hero", "kind": "footage",
            "in_frame": 60, "out_frame": 240, "label": 2,
            "switches": {"visible": true, "audible": true, "locked": false,
              "three_d": false, "collapse": false, "fx": true,
              "solo": false, "motion_blur": false},
            "transform": {
              "anchor_x": {"value": 960, "animated": false},
              "anchor_y": {"value": 540, "animated": false},
              "position_x": {"value": 960, "animated": true, "keys": [
                {"frame": 60, "value": 960, "interp_in": "Linear", "interp_out": "Linear"},
                {"frame": 120, "value": 1200, "interp_in": "Linear", "interp_out": "Linear"}
              ]},
              "position_y": {"value": 540, "animated": false},
              "scale_x": {"value": 100, "animated": false},
              "scale_y": {"value": 100, "animated": false},
              "rotation": {"value": 0, "animated": false},
              "opacity": {"value": 100, "animated": false}
            }
          },
          {
            "id": "l1", "index": 1, "name": "backdrop", "kind": "solid",
            "in_frame": 0, "out_frame": 300, "label": 0,
            "switches": {"visible": false, "audible": true, "locked": false,
              "three_d": false, "collapse": false, "fx": true,
              "solo": false, "motion_blur": false}
          }
        ],
        "markers": [120]
      }
    },
    {
      "id": "c1", "name": "Titles", "kind": "composition", "children": [],
      "comp": {"width": 1920, "height": 1080, "fps": {"num": 60, "den": 1},
        "frame_count": 120, "layers": [], "markers": []}
    }
  ],
  "can_undo": true, "can_redo": false, "path": null
}''';

/// A fake bridge that always answers with the two-comp document and records the
/// ops it is asked to run.
class _TimelineFake implements DocumentBridge {
  final List<String> ops = [];

  BridgeReply _snap() => BridgeReply.parse(_twoCompJson);

  BridgeReply _op(String record) {
    ops.add(record);
    return _snap();
  }

  @override
  BridgeReply snapshot() => _snap();
  @override
  BridgeReply newProject() => _snap();
  @override
  BridgeReply undo() => _snap();
  @override
  BridgeReply redo() => _snap();
  @override
  BridgeReply openProject(String p) => _snap();
  @override
  BridgeReply saveProject(String p) => _snap();
  @override
  BridgeReply newComposition(String name) => _snap();
  @override
  BridgeReply importFootage(String p) => _snap();
  @override
  BridgeReply setLayerSwitch(
          String compId, String layerId, String switchName, bool value) =>
      _op('switch:$compId/$layerId/$switchName=$value');
  @override
  BridgeReply editLayerSpan(
          String compId, String layerId, String edit, int frame) =>
      _op('span:$compId/$layerId/$edit@$frame');
  @override
  BridgeReply setTransform(
          String compId, String layerId, String property, double value) =>
      _op('transform:$compId/$layerId/$property=$value');
  @override
  BridgeReply addMarker(String compId, int frame) => _op('marker:$compId@$frame');
  @override
  BridgeReply addSolidLayer(String compId) => _op('add_solid:$compId');
  @override
  BridgeReply addTextLayer(String compId) => _op('add_text:$compId');
  @override
  BridgeReply addCameraLayer(String compId) => _op('add_camera:$compId');
  @override
  BridgeReply addAdjustmentLayer(String compId) => _op('add_adjustment:$compId');
  @override
  BridgeReply addSequenceLayer(String compId) => _op('add_sequence:$compId');
  @override
  BridgeReply deleteLayer(String compId, String layerId) =>
      _op('delete_layer:$compId/$layerId');
  @override
  BridgeReply duplicateLayer(String compId, String layerId) =>
      _op('duplicate_layer:$compId/$layerId');
  @override
  BridgeReply setCompSettings(String compId, String name, int width, int height,
          int fpsNum, int fpsDen, int durationFrames) =>
      _op('comp_settings:$compId');
  @override
  BridgeReply togglePropertyAnimated(
          String compId, String layerId, String property, int frame) =>
      _op('stopwatch:$compId/$layerId/$property@$frame');
  @override
  BridgeReply addKeyframe(String compId, String layerId, String property,
          int frame, double value) =>
      _op('add_key:$compId/$layerId/$property@$frame=$value');
  @override
  BridgeReply removeKeyframe(
          String compId, String layerId, String property, int frame) =>
      _op('remove_key:$compId/$layerId/$property@$frame');
  @override
  BridgeReply shiftKeyframes(String compId, String layerId, String property,
          List<int> frames, int delta) =>
      _op('shift_keys:$compId/$layerId/$property+$delta');
  @override
  BridgeReply setWorkAreaEdge(String compId, int frame, bool isOut) =>
      _op('work_area:$compId@$frame/out=$isOut');
  @override
  List<BridgeEffectInfo> listEffects() => const [];
  @override
  BridgeReply addEffect(String compId, String layerId, String effectName) =>
      _op('add_effect:$compId/$layerId/$effectName');
  @override
  BridgeReply removeEffect(String compId, String layerId, String effectId) =>
      _op('remove_effect:$compId/$layerId/$effectId');
  @override
  BridgeReply setEffectEnabled(
          String compId, String layerId, String effectId, bool enabled) =>
      _op('effect_enabled:$compId/$layerId/$effectId=$enabled');
  @override
  BridgeReply setEffectParamScalar(String compId, String layerId,
          String effectId, String paramName, double value) =>
      _op('effect_scalar:$compId/$layerId/$effectId/$paramName=$value');
  @override
  BridgeReply setEffectParamColour(String compId, String layerId,
          String effectId, String paramName, double r, double g, double b,
          double a) =>
      _op('effect_colour:$compId/$layerId/$effectId/$paramName');
  // Bridge v0.4 stubs (unused by these tests; return the snapshot).
  @override
  BridgeReply setKeyframeInterp(String compId, String layerId, String property,
          int frame, String interpIn, String interpOut, double speedIn,
          double influenceIn, double speedOut, double influenceOut) =>
      _snap();
  @override
  BridgeReply setRetimeEnabled(String compId, String layerId, bool enabled) =>
      _snap();
  @override
  BridgeReply setRetimeSpeed(String compId, String layerId, double speed) =>
      _snap();
  @override
  BridgeReply setSegmentPreset(
          String compId, String layerId, int frame, String ease) =>
      _snap();
  @override
  BridgeReply segmentToRate(String compId, String layerId, int frame) =>
      _snap();
  @override
  BridgeReply dragBoundary(
          String compId, String layerId, int index, int frame) =>
      _snap();
  @override
  List<BridgeBlendMode> listBlendModes() => const [];
  @override
  BridgeReply setBlendMode(String compId, String layerId, String mode) =>
      _snap();
  @override
  BridgeReply setMatte(String compId, String layerId, String source,
          String channel, bool inverted) =>
      _snap();
  @override
  BridgeReply setParent(String compId, String layerId, String parent) =>
      _snap();
  @override
  BridgeReply setMotionBlur(String compId, bool enabled, double shutterAngle,
          double shutterPhase, int samples) =>
      _snap();
  @override
  BridgeReply addMask(String compId, String layerId, String kind) => _snap();
  @override
  BridgeExportPreset exportPreset(
          String presetName, String compName, String template) =>
      BridgeExportPreset.idle;
  @override
  BridgeReply startExport(String compId, String specJson, String outPath) =>
      _snap();
  @override
  BridgeExportState exportPoll() => BridgeExportState.idle;
  @override
  BridgeReply exportCancel() => _snap();

  @override
  DecodedFrame? decodeFrame(String itemId, int frame) => null;
}

Widget _host(AppStateStub app) => Directionality(
      textDirection: TextDirection.ltr,
      child: MediaQuery(
        data: const MediaQueryData(),
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          // An Overlay so the layer context menu (showLumitPopup) has somewhere
          // to mount; ThemeScope stays above it so popups read the theme.
          child: Overlay(
            initialEntries: [
              OverlayEntry(builder: (_) => TimelinePanel(app: app)),
            ],
          ),
        ),
      ),
    );

/// Open a layer's twirl so its transform property rows are shown.
Future<void> _openTwirl(WidgetTester tester, String layerId) async {
  await tester.tap(find.byKey(ValueKey('twirl:$layerId')));
  await tester.pump();
}

void main() {
  group('LaneScale (time↔pixel under zoom)', () {
    test('zoom 1 fits the whole comp from frame 0', () {
      final s = LaneScale.fit(
          trackLeft: 100, trackWidth: 600, frameCount: 300, zoom: 1);
      expect(s.viewStartFrame, 0);
      expect(s.pxPerFrame, closeTo(2.0, 1e-9));
      expect(s.xOfFrame(0), closeTo(100, 1e-9));
      expect(s.xOfFrame(300), closeTo(700, 1e-9));
      expect(s.frameOfX(400), closeTo(150, 1e-9));
    });

    test('zoom doubles pixels-per-frame and never scrolls past the ends', () {
      final s = LaneScale.fit(
        trackLeft: 0,
        trackWidth: 600,
        frameCount: 300,
        zoom: 2,
        desiredStartFrame: 999, // clamped to (300 - 150)
      );
      expect(s.pxPerFrame, closeTo(4.0, 1e-9));
      expect(s.viewStartFrame, closeTo(150, 1e-9));
    });

    test('a degenerate comp still yields a usable scale', () {
      final s =
          LaneScale.fit(trackLeft: 0, trackWidth: 100, frameCount: 0, zoom: 1);
      expect(s.frameCount, 1);
      expect(s.pxPerFrame.isFinite, isTrue);
    });
  });

  group('LaneScale.clampViewStart (horizontal pan)', () {
    test('at fit the view is pinned to the comp start', () {
      expect(
        LaneScale.clampViewStart(desired: 120, frameCount: 300, zoom: 1),
        0,
      );
    });

    test('a pan past the end clamps to frames − visible', () {
      // zoom 2 → 150 frames visible, so the furthest start is 150.
      expect(
        LaneScale.clampViewStart(desired: 9999, frameCount: 300, zoom: 2),
        closeTo(150, 1e-9),
      );
    });

    test('a pan before the start clamps to 0', () {
      expect(
        LaneScale.clampViewStart(desired: -40, frameCount: 300, zoom: 3),
        0,
      );
    });

    test('canPan is false at fit and true once zoomed', () {
      final fit =
          LaneScale.fit(trackLeft: 0, trackWidth: 600, frameCount: 300, zoom: 1);
      final zoomed =
          LaneScale.fit(trackLeft: 0, trackWidth: 600, frameCount: 300, zoom: 2);
      expect(fit.canPan, isFalse);
      expect(zoomed.canPan, isTrue);
    });
  });

  group('keyShapeFor (interpolation glyph coding)', () {
    test('linear both sides is a diamond', () {
      expect(keyShapeFor('Linear', 'Linear'), KeyShape.diamond);
    });

    test('a hold on either side is a square', () {
      expect(keyShapeFor('Hold', 'Linear'), KeyShape.square);
      expect(keyShapeFor('Linear', 'Hold'), KeyShape.square);
    });

    test('a bezier on either side (no hold) is a circle', () {
      expect(keyShapeFor('Linear', 'Bezier'), KeyShape.circle);
      expect(keyShapeFor('Bezier', 'Linear'), KeyShape.circle);
    });

    test('hold wins over bezier', () {
      expect(keyShapeFor('Hold', 'Bezier'), KeyShape.square);
    });
  });

  group('keyNavTargets (◄ ◆ ► resolution)', () {
    test('between keys: prev + next set, not on a key', () {
      final t = keyNavTargets(const [60, 120], 90);
      expect(t.prev, 60);
      expect(t.next, 120);
      expect(t.onKey, isFalse);
    });

    test('on a key: onKey true, prev/next skip it', () {
      final t = keyNavTargets(const [60, 120], 60);
      expect(t.onKey, isTrue);
      expect(t.prev, isNull);
      expect(t.next, 120);
    });

    test('past the last key: only prev', () {
      final t = keyNavTargets(const [60, 120], 200);
      expect(t.prev, 120);
      expect(t.next, isNull);
      expect(t.onKey, isFalse);
    });
  });

  group('lane selection', () {
    const a = LaneKeyId('l0', 'position_x', 60);
    const b = LaneKeyId('l0', 'position_x', 120);

    test('a plain click replaces the selection', () {
      final sel = <LaneKeyId>{a};
      laneSelectClick(sel, b, additive: false);
      expect(sel, {b});
    });

    test('an additive click toggles membership', () {
      final sel = <LaneKeyId>{a};
      laneSelectClick(sel, b, additive: true);
      expect(sel, {a, b});
      laneSelectClick(sel, a, additive: true);
      expect(sel, {b});
    });

    test('groupKeysForShift buckets by channel, sorted', () {
      final groups = groupKeysForShift(const [
        LaneKeyId('l0', 'position_x', 120),
        LaneKeyId('l0', 'position_x', 60),
        LaneKeyId('l1', 'opacity', 10),
      ]);
      expect(groups[('l0', 'position_x')], [60, 120]);
      expect(groups[('l1', 'opacity')], [10]);
    });
  });

  group('workAreaEdgeAt (band edge hit-test)', () {
    test('a pointer near the in edge grabs it', () {
      expect(workAreaEdgeAt(52, 50, 300), WorkAreaEdge.inEdge);
    });

    test('a pointer near the out edge grabs it', () {
      expect(workAreaEdgeAt(303, 50, 300), WorkAreaEdge.outEdge);
    });

    test('a pointer in the middle grabs neither', () {
      expect(workAreaEdgeAt(180, 50, 300), isNull);
    });

    test('a tie goes to the nearer edge', () {
      expect(workAreaEdgeAt(56, 50, 60), WorkAreaEdge.outEdge);
    });
  });

  group('layerMatchesSearch', () {
    test('an empty query matches everything', () {
      expect(layerMatchesSearch('hero', ''), isTrue);
      expect(layerMatchesSearch('hero', '   '), isTrue);
    });

    test('a case-insensitive substring matches', () {
      expect(layerMatchesSearch('Backdrop', 'drop'), isTrue);
      expect(layerMatchesSearch('Backdrop', 'BACK'), isTrue);
    });

    test('a non-substring does not match', () {
      expect(layerMatchesSearch('hero', 'zzz'), isFalse);
    });
  });

  group('chooseColumns (degradation order)', () {
    test('a wide outline shows the whole footage cluster', () {
      final c = chooseColumns(260, canAudio: true, isPrecomp: false);
      expect(c.eye, isTrue);
      expect(c.speaker, isTrue);
      expect(c.solo, isTrue);
      expect(c.lock, isTrue);
      expect(c.fx, isTrue);
      expect(c.motionBlur, isTrue);
      expect(c.threeD, isTrue);
      expect(c.index, isTrue);
    });

    test('columns drop in order: collapse/3D, then fx/MB, then solo…', () {
      final c = chooseColumns(120, canAudio: true, isPrecomp: true);
      expect(c.eye, isTrue, reason: 'eye survives longest');
      expect(c.collapse, isFalse);
      expect(c.threeD, isFalse);
      expect(c.motionBlur, isFalse);
    });

    test('a tiny width shows glyph + name only', () {
      final c = chooseColumns(36, canAudio: true, isPrecomp: false);
      expect(c.eye, isFalse);
      expect(c.lock, isFalse);
      expect(c.index, isFalse);
      expect(c, isA<OutlineColumns>());
    });

    test('non-audio layers never reserve a speaker', () {
      final c = chooseColumns(260, canAudio: false, isPrecomp: false);
      expect(c.speaker, isFalse);
    });
  });

  group('snapFrame', () {
    test('snapping off is the identity', () {
      expect(
        snapFrame(77, fps: 60, markers: const [120], snapping: false, pxPerFrame: 2),
        77,
      );
    });

    test('a near whole-second lands on the second', () {
      expect(
        snapFrame(61, fps: 60, markers: const [], snapping: true, pxPerFrame: 2),
        60,
      );
    });

    test('a marker wins when it is the closest candidate', () {
      expect(
        snapFrame(119, fps: 60, markers: const [120], snapping: true, pxPerFrame: 4),
        120,
      );
    });

    test('nothing within the threshold is left alone', () {
      expect(
        snapFrame(77, fps: 60, markers: const [120], snapping: true, pxPerFrame: 4),
        77,
      );
    });
  });

  group('Timeline panel (fake bridge)', () {
    testWidgets('comp tabs render every composition', (tester) async {
      final app = AppStateStub(bridge: _TimelineFake());
      await tester.pumpWidget(_host(app));
      expect(find.text('Scene'), findsOneWidget);
      expect(find.text('Titles'), findsOneWidget);
    });

    testWidgets('clicking a comp pill fronts that comp', (tester) async {
      final app = AppStateStub(bridge: _TimelineFake());
      await tester.pumpWidget(_host(app));
      expect(app.frontCompIdResolved, 'c0');
      await tester.tap(find.text('Titles'));
      await tester.pump();
      expect(app.frontCompIdResolved, 'c1');
    });

    testWidgets('layer rows render their names', (tester) async {
      final app = AppStateStub(bridge: _TimelineFake());
      await tester.pumpWidget(_host(app));
      expect(find.text('hero'), findsOneWidget);
      expect(find.text('backdrop'), findsOneWidget);
    });

    testWidgets('tapping the eye toggles visible off through the op',
        (tester) async {
      final fake = _TimelineFake();
      final app = AppStateStub(bridge: fake);
      await tester.pumpWidget(_host(app));
      await tester.tap(find.byKey(const ValueKey('sw:l0:visible')));
      await tester.pump();
      expect(fake.ops, contains('switch:c0/l0/visible=false'));
    });

    testWidgets('tapping a bar selects the layer', (tester) async {
      final app = AppStateStub(bridge: _TimelineFake());
      await tester.pumpWidget(_host(app));
      await tester.tapAt(const Offset(500, 97));
      await tester.pump();
      expect(app.selectedLayer, 'l1');
    });

    testWidgets('dragging the left edge issues a trim_in at the dragged frame',
        (tester) async {
      final fake = _TimelineFake();
      final app = AppStateStub(bridge: fake)..snapping = false;
      await tester.pumpWidget(_host(app));
      const ppf = 532 / 300;
      final startX = 260 + 60 * ppf; // l0 in-point (frame 60) left edge
      const dx = 30.0;
      final expected = ((startX + dx - 260) / ppf).round();
      await tester.dragFrom(Offset(startX, 75), const Offset(dx, 0));
      await tester.pump();
      expect(fake.ops, contains('span:c0/l0/trim_in@$expected'));
    });

    testWidgets('a scrub click on the ruler moves the playhead', (tester) async {
      final app = AppStateStub(bridge: _TimelineFake())..snapping = false;
      await tester.pumpWidget(_host(app));
      const ppf = 532 / 300;
      final x = 260 + 150 * ppf; // frame 150
      await tester.tapAt(Offset(x, 46)); // ruler band y
      await tester.pump();
      expect(app.previewFrame, 150);
    });

    testWidgets('the twirl reveals the transform property rows',
        (tester) async {
      final app = AppStateStub(bridge: _TimelineFake());
      await tester.pumpWidget(_host(app));
      expect(find.text('Position'), findsNothing);
      await _openTwirl(tester, 'l0');
      expect(find.text('Transform'), findsOneWidget);
      expect(find.text('Anchor point'), findsOneWidget);
      expect(find.text('Position'), findsOneWidget);
      expect(find.text('Opacity'), findsOneWidget);
    });

    testWidgets('the stopwatch toggles animation through the op',
        (tester) async {
      final fake = _TimelineFake();
      final app = AppStateStub(bridge: fake);
      await tester.pumpWidget(_host(app));
      await _openTwirl(tester, 'l0');
      await tester.tap(find.byKey(const ValueKey('stopwatch:l0:position_x')));
      await tester.pump();
      expect(fake.ops, contains('stopwatch:c0/l0/position_x@0'));
    });

    testWidgets('the navigator diamond adds a key between keys', (tester) async {
      final fake = _TimelineFake();
      final app = AppStateStub(bridge: fake);
      await tester.pumpWidget(_host(app));
      await _openTwirl(tester, 'l0');
      app.goToFrame(90); // between the keys at 60 and 120
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('nav-toggle:l0:position_x')));
      await tester.pump();
      expect(
        fake.ops.any((o) => o.startsWith('add_key:c0/l0/position_x@90')),
        isTrue,
      );
    });

    testWidgets('the navigator diamond removes the key on the playhead',
        (tester) async {
      final fake = _TimelineFake();
      final app = AppStateStub(bridge: fake);
      await tester.pumpWidget(_host(app));
      await _openTwirl(tester, 'l0');
      app.goToFrame(60); // exactly on a key
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('nav-toggle:l0:position_x')));
      await tester.pump();
      expect(fake.ops, contains('remove_key:c0/l0/position_x@60'));
    });

    testWidgets('dragging a keyframe commits one shiftKeyframes',
        (tester) async {
      final fake = _TimelineFake();
      final app = AppStateStub(bridge: fake)..snapping = false;
      await tester.pumpWidget(_host(app));
      await _openTwirl(tester, 'l0');
      const ppf = 532 / 300;
      final rowRect =
          tester.getRect(find.byKey(const ValueKey('prop:l0:position_x')));
      final glyphX = 260 + 60 * ppf; // key at frame 60
      const dx = 40.0;
      final expected = (60 + dx / ppf).round() - 60;
      await tester.dragFrom(Offset(glyphX, rowRect.center.dy), const Offset(dx, 0));
      await tester.pump();
      final shifts = fake.ops.where((o) => o.startsWith('shift_keys')).toList();
      expect(shifts, ['shift_keys:c0/l0/position_x+$expected']);
    });

    testWidgets('right-clicking a keyframe removes it', (tester) async {
      final fake = _TimelineFake();
      final app = AppStateStub(bridge: fake);
      await tester.pumpWidget(_host(app));
      await _openTwirl(tester, 'l0');
      const ppf = 532 / 300;
      final rowRect =
          tester.getRect(find.byKey(const ValueKey('prop:l0:position_x')));
      final glyphX = 260 + 120 * ppf; // key at frame 120
      await tester.tapAt(
        Offset(glyphX, rowRect.center.dy),
        buttons: kSecondaryButton,
      );
      await tester.pump();
      expect(fake.ops, contains('remove_key:c0/l0/position_x@120'));
    });

    testWidgets('the context menu Duplicate calls duplicateLayer',
        (tester) async {
      final fake = _TimelineFake();
      final app = AppStateStub(bridge: fake);
      await tester.pumpWidget(_host(app));
      await tester.tap(find.text('hero'), buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Duplicate'));
      await tester.pumpAndSettle();
      expect(fake.ops, contains('duplicate_layer:c0/l0'));
    });

    testWidgets('the search filter hides non-matching rows', (tester) async {
      final app = AppStateStub(bridge: _TimelineFake());
      await tester.pumpWidget(_host(app));
      expect(find.byKey(const ValueKey('twirl:l1')), findsOneWidget);
      await tester.enterText(find.byType(EditableText), 'hero');
      await tester.pump();
      // The hero row stays; the backdrop row is filtered out. (The search box
      // itself now reads "hero", so assert on the rows' twirls, not the text.)
      expect(find.byKey(const ValueKey('twirl:l0')), findsOneWidget);
      expect(find.byKey(const ValueKey('twirl:l1')), findsNothing);
    });

    testWidgets('dragging a work-area edge calls setWorkAreaEdge',
        (tester) async {
      final fake = _TimelineFake();
      final app = AppStateStub(bridge: fake)..snapping = false;
      await tester.pumpWidget(_host(app));
      const ppf = 532 / 300;
      final ruler = tester.getRect(find.byType(TimelineRuler));
      // The in edge sits at frame 30; its handle is centred on that lane x
      // (the ruler's own left is the lane left).
      final inX = ruler.left + 30 * ppf;
      await tester.dragFrom(Offset(inX, ruler.center.dy), const Offset(40, 0));
      await tester.pump();
      expect(
        fake.ops.any((o) =>
            o.startsWith('work_area:c0@') && o.endsWith('/out=false')),
        isTrue,
      );
    });
  });
}
