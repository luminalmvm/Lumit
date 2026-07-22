// Widget tests for the Viewer transform gizmo (viewer_overlays.dart): the
// bounding box / scale handles / anchor pan-behind / body drag committing the
// right transform ops. Uses a fake bridge whose snapshot carries a solid layer
// with a full transform read-back and a native raster size, and a fake frame
// renderer so no real engine or platform texture is touched.
//
// Geometry is chosen so screen == comp pixels (viewScale 1): comp 1000×500,
// the solid 200×100, anchor (100,50), position (500,250), scale 100%, rotation
// 0. The anchor cross then sits at (500,250) and the box spans (400,200)-(600,
// 300), so the hand-computed drag targets below land on exact integers.

import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/bridge/bridge.dart';
import 'package:lumit_flutter/panels/preview_source.dart';
import 'package:lumit_flutter/panels/viewer_overlays.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

/// A bridge whose snapshot has one solid layer with a full transform read-back.
/// [scaleXAnimated] flips `scale_x` to keyframed so the animation-aware commit
/// path (addKeyframe) can be exercised. Records setTransform / addKeyframe.
class _GizmoFake implements DocumentBridge {
  final List<String> ops = [];
  final bool scaleXAnimated;
  _GizmoFake({this.scaleXAnimated = false});

  String get _json => '''
  {
    "ok": true,
    "items": [
      {
        "id": "c1", "name": "Scene", "kind": "composition", "children": [],
        "comp": {
          "width": 1000, "height": 500, "fps": {"num": 30, "den": 1},
          "frame_count": 300,
          "layers": [
            {"id":"ls","index":0,"name":"BG","kind":"solid",
             "in_frame":0,"out_frame":300,"label":0,"switches":{},
             "solid_size":[200,100],
             "transform":{
               "anchor_x":{"value":100,"animated":false,"keys":[]},
               "anchor_y":{"value":50,"animated":false,"keys":[]},
               "position_x":{"value":500,"animated":false,"keys":[]},
               "position_y":{"value":250,"animated":false,"keys":[]},
               "scale_x":{"value":100,"animated":$scaleXAnimated,
                 "keys":${scaleXAnimated ? '[{"frame":0,"value":100,"interp_in":"Linear","interp_out":"Linear"}]' : '[]'}},
               "scale_y":{"value":100,"animated":false,"keys":[]},
               "rotation":{"value":0,"animated":false,"keys":[]}
             }}
          ],
          "markers": []
        }
      }
    ],
    "can_undo": false, "can_redo": false, "path": null
  }''';

  BridgeReply _snap() => BridgeReply.parse(_json);
  BridgeReply _op(String r) {
    ops.add(r);
    return _snap();
  }

  @override
  BridgeReply snapshot() => _snap();

  @override
  BridgeReply setTransform(
          String c, String l, String property, double value) =>
      _op('set:$c/$l/$property=${value.toStringAsFixed(1)}');

  @override
  BridgeReply addKeyframe(
          String c, String l, String property, int frame, double value) =>
      _op('key:$c/$l/$property@$frame=${value.toStringAsFixed(1)}');

  @override
  BridgeReply setEffectParamColour(String c, String l, String e, String p,
          double r, double g, double b, double a) =>
      _op('fxcolour:$c/$l/$e/$p=${r.toStringAsFixed(2)},'
          '${g.toStringAsFixed(2)},${b.toStringAsFixed(2)},'
          '${a.toStringAsFixed(2)}');

  @override
  dynamic noSuchMethod(Invocation invocation) => _snap();
}

/// A frame renderer that offers no shared texture (so no platform channel) and
/// answers nothing — the gizmo needs only the geometry, not a picture.
class _NullRenderer implements FrameRenderer {
  @override
  bool get supportsCompRender => true;
  @override
  bool get supportsSharedTexture => false;
  @override
  void requestComp(String compId, int frame, double scale, int generation,
          void Function(DecodedFrame?) onFrame) =>
      onFrame(DecodedFrame(width: 2, height: 2, rgba: Uint8List(16)));
  @override
  void requestShared(String compId, int frame, int generation,
          void Function(SharedFrame?) onFrame) =>
      onFrame(null);
  @override
  void requestDecode(String itemId, int frame, int generation,
          void Function(DecodedFrame?) onFrame) =>
      onFrame(null);
  @override
  void requestScopeTrace(int kind, String compId, int frame, double scale,
          int bg, int trace, int red, int green, int blue, int generation,
          void Function(Uint8List?) onTrace) =>
      onTrace(null);
  @override
  void requestThumbnail(String itemId, int maxEdge, int generation,
          void Function(DecodedFrame?) onFrame) =>
      onFrame(null);
  @override
  void dispose() {}
}

/// A [_NullRenderer] whose comp render answers a solid red 2×2 frame, so the
/// eyedropper has known pixels to sample from the read-back path.
class _RedRenderer extends _NullRenderer {
  @override
  void requestComp(String compId, int frame, double scale, int generation,
      void Function(DecodedFrame?) onFrame) {
    final px = Uint8List(2 * 2 * 4);
    for (var i = 0; i < px.length; i += 4) {
      px[i] = 255; // red
      px[i + 3] = 255; // opaque
    }
    onFrame(DecodedFrame(width: 2, height: 2, rgba: px));
  }
}

Widget _host(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: MediaQuery(
        data: const MediaQueryData(size: Size(1000, 500)),
        child: ThemeScope(
          theme: LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(
            initialEntries: [OverlayEntry(builder: (_) => child)],
          ),
        ),
      ),
    );

Future<(AppStateStub, _GizmoFake)> _pumpGizmo(
  WidgetTester tester, {
  bool scaleXAnimated = false,
}) async {
  await tester.binding.setSurfaceSize(const Size(1000, 500));
  final fake = _GizmoFake(scaleXAnimated: scaleXAnimated);
  final app = AppStateStub(bridge: fake)..selectLayer('ls');
  final source = PreviewSource(app, renderer: _NullRenderer());
  await tester.pumpWidget(_host(
    SizedBox(
      width: 1000,
      height: 500,
      child: ViewerInteractionLayer(
        app: app,
        source: source,
        imageRect: const Rect.fromLTWH(0, 0, 1000, 500),
        compWidth: 1000,
        compHeight: 500,
      ),
    ),
  ));
  await tester.pump();
  return (app, fake);
}

void main() {
  testWidgets('the gizmo draws the box, eight scale handles and the anchor',
      (tester) async {
    await _pumpGizmo(tester);
    // The anchor cross and each of the eight scale handles are hit areas.
    expect(find.byKey(const ValueKey('gizmo-anchor')), findsOneWidget);
    for (final id in ['tl', 'tr', 'br', 'bl', 't', 'r', 'b', 'l']) {
      expect(find.byKey(ValueKey('gizmo-scale-$id')), findsOneWidget);
    }
    expect(find.byKey(const ValueKey('gizmo-body')), findsOneWidget);
  });

  testWidgets('a body drag commits position through setTransform',
      (tester) async {
    final (_, fake) = await _pumpGizmo(tester);
    // Start inside the box but away from the centre (the anchor) and the
    // corners, so the body detector wins.
    await tester.dragFrom(const Offset(450, 220), const Offset(50, 20));
    await tester.pump();
    expect(fake.ops, contains('set:c1/ls/position_x=550.0'));
    expect(fake.ops, contains('set:c1/ls/position_y=270.0'));
  });

  testWidgets('an anchor drag commits the pan-behind (anchor + position)',
      (tester) async {
    final (_, fake) = await _pumpGizmo(tester);
    // The anchor cross sits at (500,250); drag it +30 in x.
    await tester.dragFrom(const Offset(500, 250), const Offset(30, 0));
    await tester.pump();
    // New anchor (130,50); pan-behind holds the layer visually fixed, so
    // position compensates to (530,250).
    expect(fake.ops, contains('set:c1/ls/anchor_x=130.0'));
    expect(fake.ops, contains('set:c1/ls/anchor_y=50.0'));
    expect(fake.ops, contains('set:c1/ls/position_x=530.0'));
    expect(fake.ops, contains('set:c1/ls/position_y=250.0'));
  });

  testWidgets('a corner scale drag commits both axes', (tester) async {
    final (_, fake) = await _pumpGizmo(tester);
    // The BR handle sits at (600,300); drag it so both axes double.
    await tester.dragFrom(const Offset(600, 300), const Offset(100, 50));
    await tester.pump();
    expect(fake.ops, contains('set:c1/ls/scale_x=200.0'));
    expect(fake.ops, contains('set:c1/ls/scale_y=200.0'));
  });

  testWidgets('an edge scale drag moves only its axis', (tester) async {
    final (_, fake) = await _pumpGizmo(tester);
    // The right-edge handle sits at (600,250); drag it +100 in x → 200% on x.
    await tester.dragFrom(const Offset(600, 250), const Offset(100, 0));
    await tester.pump();
    expect(fake.ops, contains('set:c1/ls/scale_x=200.0'));
    // No y op — the edge handle drives one axis only.
    expect(fake.ops.any((o) => o.contains('scale_y')), isFalse);
  });

  testWidgets('a keyed property commits a keyframe, not a static set',
      (tester) async {
    final (_, fake) = await _pumpGizmo(tester, scaleXAnimated: true);
    // The right-edge handle drives scale_x, which is keyframed here → the
    // animation-aware commit writes a key at the playhead (frame 0).
    await tester.dragFrom(const Offset(600, 250), const Offset(100, 0));
    await tester.pump();
    expect(fake.ops, contains('key:c1/ls/scale_x@0=200.0'));
  });

  testWidgets(
      'the eyedropper samples the read-back frame and commits through the op '
      '(TF round 5: no synchronous render)', (tester) async {
    await tester.binding.setSurfaceSize(const Size(1000, 500));
    final fake = _GizmoFake();
    final app = AppStateStub(bridge: fake)..selectLayer('ls');
    // Arm BEFORE pumping so the build takes the eyedropper branch (the shell
    // normally rebuilds the stage on the app notifier).
    app.armEyedropper(const EyedropperArm(
      compId: 'c1',
      layerId: 'ls',
      effectId: 'e1',
      paramName: 'tint',
    ));
    late PreviewSource source;
    await tester.runAsync(() async {
      source = PreviewSource(app, renderer: _RedRenderer());
      await tester.pumpWidget(_host(
        SizedBox(
          width: 1000,
          height: 500,
          child: ViewerInteractionLayer(
            app: app,
            source: source,
            imageRect: const Rect.fromLTWH(0, 0, 1000, 500),
            compWidth: 1000,
            compHeight: 500,
          ),
        ),
      ));
      // Let the read-back frame's image decode land (displayedFrame set).
      await Future<void>.delayed(const Duration(milliseconds: 80));
    });
    await tester.pump();
    expect(source.displayedFrame, isNotNull,
        reason: 'the sample reads the frame already read back — no render');

    await tester.tapAt(const Offset(500, 250));
    await tester.pump();
    expect(fake.ops, contains('fxcolour:c1/ls/e1/tint=1.00,0.00,0.00,1.00'),
        reason: 'the sampled red committed through setEffectParamColour');
    expect(app.eyedropperArmed, isFalse, reason: 'the commit disarms');
    source.dispose();
  });
}
