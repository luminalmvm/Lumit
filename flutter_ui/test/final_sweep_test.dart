// The final integration sweep (docs/flutter-port/06): the cross-agent seams and
// thin remainders closed together — Ctrl+C/V keyframe-clipboard routing, the
// preview-scale downsample threading, the timeline cache bar, the layer context
// menu (Rename in place, the categorised Add-effect submenu, Convert / Trim),
// the effect drag-onto-a-layer-row drop target, Project-panel thumbnails, and a
// couple of the DragValueField Reset targets.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/bridge/bridge.dart';
import 'package:lumit_flutter/panels/effect_controls_panel.dart';
import 'package:lumit_flutter/panels/preview_source.dart';
import 'package:lumit_flutter/panels/project_panel.dart';
import 'package:lumit_flutter/panels/timeline/cache_bar.dart';
import 'package:lumit_flutter/panels/timeline_panel.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

/// A fake offering every capability the sweep touches: DocumentBridge (via
/// noSuchMethod → ok snapshot), EditOpsBridge (recording the identity ops),
/// CompRenderBridge (recording the render scale), CacheControlBridge (fake
/// stats), and ThumbnailBridge (a 2×2 synthetic thumbnail). The snapshot carries
/// a plain footage layer, a RETIMED footage layer, and a root footage item.
class _Fake
    implements
        DocumentBridge,
        EditOpsBridge,
        CompRenderBridge,
        CacheControlBridge,
        ThumbnailBridge {
  final List<String> ops = [];
  final List<double> renderScales = [];
  int cacheEntries = 0;

  static const _json = '''
  {
    "ok": true,
    "items": [
      {
        "id": "c1", "name": "Scene", "kind": "composition", "children": [],
        "comp": {
          "width": 1920, "height": 1080, "fps": {"num": 60, "den": 1},
          "frame_count": 300,
          "layers": [
            {"id":"lf","index":0,"name":"clip","kind":"footage",
             "in_frame":0,"out_frame":300,"label":0,"switches":{}},
            {"id":"lr","index":1,"name":"retimed","kind":"footage",
             "in_frame":0,"out_frame":300,"label":0,"switches":{},
             "retime":{"reverse":false,"interpolation":"nearest",
                       "boundaries":[],"segments":[]}}
          ],
          "markers": []
        }
      },
      {"id":"f1","name":"shot.mov","kind":"footage","children":[],"status":"ok"}
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
  List<BridgeEffectInfo> listEffects() => const [
        BridgeEffectInfo(
            name: 'blur',
            label: 'Gaussian blur',
            category: 'blur_sharpen',
            categoryLabel: 'Blur & sharpen'),
        BridgeEffectInfo(
            name: 'glow',
            label: 'Glow',
            category: 'stylise',
            categoryLabel: 'Stylise'),
      ];

  @override
  List<BridgeBlendMode> listBlendModes() => const [];

  @override
  BridgeReply addEffect(String c, String l, String name) =>
      _op('addeffect:$c/$l/$name');
  @override
  BridgeReply renameLayer(String c, String l, String n) =>
      _op('rename_layer:$c/$l/"$n"');
  @override
  BridgeReply convertToSequenced(String c, String l) => _op('convert:$c/$l');
  @override
  BridgeReply trimToSourceEnd(String c, String l) => _op('trim:$c/$l');

  // --- CompRenderBridge (records the scale the resolution picker threads) ---
  @override
  bool get supportsCompRender => true;
  @override
  DecodedFrame? renderCompFrame(String compId, int frame, double scale) {
    renderScales.add(scale);
    cacheEntries++;
    return DecodedFrame(
        width: 2, height: 2, rgba: Uint8List(2 * 2 * 4)..fillRange(0, 16, 200));
  }

  // --- CacheControlBridge (fake aggregate stats) --------------------------
  @override
  bool get supportsCacheControl => true;
  @override
  BridgeCacheStats clearCache() {
    cacheEntries = 0;
    return const BridgeCacheStats();
  }

  @override
  BridgeCacheStats setCacheBudget(int bytes) => BridgeCacheStats(
      budgetBytes: bytes, entries: cacheEntries);
  @override
  BridgeCacheStats cacheStats() =>
      BridgeCacheStats(entries: cacheEntries, budgetBytes: 512 << 20);

  // --- ThumbnailBridge (a solid 2×2 thumbnail) ----------------------------
  @override
  bool get supportsThumbnail => true;
  @override
  DecodedFrame? thumbnail(String itemId, int maxEdge) => DecodedFrame(
      width: 2, height: 2, rgba: Uint8List(2 * 2 * 4)..fillRange(0, 16, 180));

  @override
  dynamic noSuchMethod(Invocation invocation) => _snap();
}

/// A [FrameRenderer] that records the comp-render scale and the thumbnail
/// requests, answering a tiny frame synchronously — so the scale threading and
/// the thumbnail seam are observable without a real engine.
class _RecordingRenderer implements FrameRenderer {
  final List<double> scales = [];
  final List<String> thumbRequests = [];

  @override
  bool get supportsCompRender => true;
  @override
  bool get supportsSharedTexture => false;

  @override
  void requestComp(String compId, int frame, double scale, int generation,
      void Function(DecodedFrame?) onFrame) {
    scales.add(scale);
    onFrame(DecodedFrame(width: 2, height: 2, rgba: Uint8List(16)));
  }

  @override
  void requestPreview(String compId, int frame, double scale, int generation,
          void Function(DecodedFrame?) onFrame) =>
      onFrame(null);

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
      void Function(DecodedFrame?) onFrame) {
    thumbRequests.add('$itemId@$maxEdge');
    onFrame(DecodedFrame(
        width: 2, height: 2, rgba: Uint8List(2 * 2 * 4)..fillRange(0, 16, 180)));
  }

  @override
  void dispose() {}
}

Widget _host(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: MediaQuery(
        data: const MediaQueryData(size: Size(900, 640)),
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

void main() {
  group('Keyframe clipboard routing (Ctrl+C/V seam)', () {
    test('copy/paste route to the installed Timeline handlers', () {
      final app = AppStateStub(bridge: _Fake());
      var copied = 0;
      var pasted = 0;
      app.copyKeyframesHandler = () => copied++;
      app.pasteKeyframesHandler = () => pasted++;
      app.copySelectedKeyframes();
      app.pasteKeyframes();
      expect(copied, 1);
      expect(pasted, 1);
    });

    test('with no Timeline mounted both are quiet no-ops', () {
      final app = AppStateStub(bridge: _Fake());
      // No handlers installed — must not throw.
      expect(app.copySelectedKeyframes, returnsNormally);
      expect(app.pasteKeyframes, returnsNormally);
    });
  });

  group('Preview scale threading', () {
    test('the comp render adopts the picker factor', () async {
      final renderer = _RecordingRenderer();
      final app = AppStateStub(
        bridge: _Fake(),
        previewRendererFactory: (_) => renderer,
      );
      app.setPreviewScale(PreviewScale.half);
      // Instantiating the source resolves the current frame through the renderer.
      final source = app.previewSource;
      await Future<void>.delayed(Duration.zero);
      expect(renderer.scales, isNotEmpty);
      expect(renderer.scales.last, closeTo(0.5, 1e-9));
      source.dispose();
    });
  });

  group('Cache bar', () {
    test('warmFrameRanges collapses to contiguous half-open ranges', () {
      expect(warmFrameRanges(const {}), isEmpty);
      expect(warmFrameRanges({5}), [(5, 6)]);
      expect(warmFrameRanges({0, 1, 2, 5, 6, 9}), [(0, 3), (5, 7), (9, 10)]);
    });

    test('noteFrameWarmed tracks the RAM tier, scoped per comp and scale', () {
      final app = AppStateStub(bridge: _Fake());
      app.noteFrameWarmed('c1', 10);
      app.noteFrameWarmed('c1', 11);
      expect(app.warmFramesFor('c1'), {10, 11});
      // A different comp is a different scope.
      expect(app.warmFramesFor('c2'), isEmpty);
      // Changing the preview scale re-scopes (egui folds the quality tag in).
      app.setPreviewScale(PreviewScale.half);
      app.noteFrameWarmed('c1', 20);
      expect(app.warmFramesFor('c1'), {20});
    });

    test('clearing the cache empties the warm set and bumps the revision', () {
      final app = AppStateStub(bridge: _Fake());
      app.noteFrameWarmed('c1', 3);
      final before = app.cacheBarRevision.value;
      app.clearCache();
      expect(app.warmFramesFor('c1'), isEmpty);
      expect(app.cacheBarRevision.value, greaterThan(before));
    });

    test('a document edit (adopt) invalidates the warm set', () {
      final app = AppStateStub(bridge: _Fake());
      app.noteFrameWarmed('c1', 7);
      expect(app.warmFramesFor('c1'), isNotEmpty);
      // Any op adopts a fresh snapshot → the engine invalidates rendered frames.
      app.addSolidLayer('c1');
      expect(app.warmFramesFor('c1'), isEmpty);
    });
  });

  group('Layer context menu wiring', () {
    Future<void> openLayerMenu(WidgetTester tester, String name) async {
      await tester.tap(find.text(name), buttons: kSecondaryButton);
      await tester.pumpAndSettle();
    }

    testWidgets('Convert to sequenced calls the op', (tester) async {
      await tester.binding.setSurfaceSize(const Size(900, 640));
      final fake = _Fake();
      final app = AppStateStub(bridge: fake);
      await tester.pumpWidget(_host(TimelinePanel(app: app)));
      await tester.pumpAndSettle();

      await openLayerMenu(tester, 'clip');
      await tester.tap(find.text('Convert to sequenced layer'));
      await tester.pumpAndSettle();
      expect(fake.ops, contains('convert:c1/lf'));
    });

    testWidgets('Trim to source end shows only for a retimed footage layer',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(900, 640));
      final fake = _Fake();
      final app = AppStateStub(bridge: fake);
      await tester.pumpWidget(_host(TimelinePanel(app: app)));
      await tester.pumpAndSettle();

      // Plain footage: no Trim entry.
      await openLayerMenu(tester, 'clip');
      expect(find.text('Trim to source end'), findsNothing);
      await tester.tapAt(const Offset(5, 5)); // dismiss
      await tester.pumpAndSettle();

      // Retimed footage: Trim appears and calls the op.
      await openLayerMenu(tester, 'retimed');
      expect(find.text('Trim to source end'), findsOneWidget);
      await tester.tap(find.text('Trim to source end'));
      await tester.pumpAndSettle();
      expect(fake.ops, contains('trim:c1/lr'));
    });

    testWidgets('Add effect opens the categorised picker and applies',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(900, 640));
      final fake = _Fake();
      final app = AppStateStub(bridge: fake);
      await tester.pumpWidget(_host(TimelinePanel(app: app)));
      await tester.pumpAndSettle();

      await openLayerMenu(tester, 'clip');
      await tester.tap(find.text('Add effect'));
      await tester.pumpAndSettle();
      // The category headings and effect rows are present.
      expect(find.text('Blur & sharpen'), findsOneWidget);
      await tester.tap(find.text('Glow'));
      await tester.pumpAndSettle();
      expect(fake.ops, contains('addeffect:c1/lf/glow'));
    });

    testWidgets('Rename opens the in-place editor and commits', (tester) async {
      await tester.binding.setSurfaceSize(const Size(900, 640));
      final fake = _Fake();
      final app = AppStateStub(bridge: fake);
      await tester.pumpWidget(_host(TimelinePanel(app: app)));
      await tester.pumpAndSettle();

      await openLayerMenu(tester, 'clip');
      await tester.tap(find.text('Rename'));
      await tester.pumpAndSettle();
      final field = find.byKey(const ValueKey('layer-rename-lf'));
      expect(field, findsOneWidget);
      await tester.enterText(field, 'hero');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(fake.ops, contains('rename_layer:c1/lf/"hero"'));
    });
  });

  group('Effect drag onto a layer row', () {
    testWidgets('dropping an effect on a row applies it to that layer',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(900, 640));
      final fake = _Fake();
      final app = AppStateStub(bridge: fake);
      await tester.pumpWidget(_host(Column(children: [
        Draggable<EffectDragData>(
          data: const EffectDragData('blur', 'Gaussian blur'),
          dragAnchorStrategy: pointerDragAnchorStrategy,
          feedback: const SizedBox(width: 40, height: 12),
          child: const SizedBox(
              width: 120, height: 24, child: Text('DRAG ME')),
        ),
        SizedBox(
          height: 560,
          child: TimelinePanel(app: app),
        ),
      ])));
      await tester.pumpAndSettle();

      final start = tester.getCenter(find.text('DRAG ME'));
      final target = tester.getCenter(find.text('clip'));
      final gesture = await tester.startGesture(start);
      await tester.pump();
      await gesture.moveTo(Offset(start.dx, start.dy + 10));
      await tester.pump();
      await gesture.moveTo(target);
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
      expect(fake.ops, contains('addeffect:c1/lf/blur'));
    });
  });

  group('DragValueField Reset targets at their sites', () {
    testWidgets('a transform axis offers Reset to its default', (tester) async {
      await tester.binding.setSurfaceSize(const Size(480, 640));
      final app = AppStateStub(bridge: _Fake())..selectLayer('lf');
      await tester.pumpWidget(_host(EffectControlsPanel(app: app)));
      await tester.pumpAndSettle();
      // Right-click the Position x axis field → Reset is offered (resetTo: seed).
      await tester.tap(find.byKey(const ValueKey('axis-position_x')),
          buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      expect(find.text('Reset'), findsOneWidget);
    });
  });

  group('Project-panel thumbnails', () {
    testWidgets('a footage row decodes and shows a thumbnail image',
        (tester) async {
      await tester.binding.setSurfaceSize(const Size(360, 500));
      final app = AppStateStub(bridge: _Fake());
      // The decode is genuinely async (a microtask then the engine's
      // decodeImageFromPixels), so mount and let real async run inside runAsync,
      // then pump the resulting frame.
      await tester.runAsync(() async {
        await tester.pumpWidget(_host(ProjectPanel(app: app)));
        await Future<void>.delayed(const Duration(milliseconds: 100));
      });
      await tester.pump();
      expect(find.byType(RawImage), findsWidgets);
    });

    testWidgets(
        'the decode rides the renderer seam and re-decodes on an epoch bump '
        '(TF round 5)', (tester) async {
      await tester.binding.setSurfaceSize(const Size(360, 500));
      final renderer = _RecordingRenderer();
      final app = AppStateStub(
        bridge: _Fake(),
        previewRendererFactory: (_) => renderer,
      );
      await tester.runAsync(() async {
        await tester.pumpWidget(_host(ProjectPanel(app: app)));
        await Future<void>.delayed(const Duration(milliseconds: 100));
      });
      await tester.pump();
      expect(renderer.thumbRequests, contains('f1@56'),
          reason: 'the thumbnail decode went through the off-thread seam, '
              'never a synchronous FFI call on the UI isolate');

      final before = renderer.thumbRequests.length;
      await tester.runAsync(() async {
        app.undo(); // adopts a fresh snapshot → the document epoch bumps
        await tester.pump();
        await Future<void>.delayed(const Duration(milliseconds: 100));
      });
      await tester.pump();
      expect(renderer.thumbRequests.length, greaterThan(before),
          reason: 'the epoch bump re-decodes the row thumbnail');
    });
  });
}
