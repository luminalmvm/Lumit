// Phase F2 tests: preview resolution logic, scope maths (pure), and the Viewer
// widget (a fake bridge decodes a synthetic frame → an image; a missing item
// shows the slate; play advances the playhead over pumped ticks).

import 'dart:ui' as ui;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/bridge/bridge.dart';
import 'package:lumit_flutter/panels/preview_source.dart';
import 'package:lumit_flutter/panels/scope_maths.dart';
import 'package:lumit_flutter/panels/scopes_panel.dart';
import 'package:lumit_flutter/panels/viewer_panel.dart';
import 'package:lumit_flutter/panels/viewer_texture_controller.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

// --- Fakes -----------------------------------------------------------------

/// A DocumentBridge that answers one prepared snapshot and one decode result.
class _FrameBridge implements DocumentBridge {
  final BridgeSnapshot snap;
  final DecodedFrame? frame;
  final List<String> decoded = [];
  _FrameBridge(this.snap, this.frame);

  @override
  BridgeReply snapshot() => BridgeReply.ok(snap);
  @override
  DecodedFrame? decodeFrame(String itemId, int frame) {
    decoded.add('$itemId@$frame');
    return this.frame;
  }

  // Unused ops for these tests.
  @override
  BridgeReply newProject() => BridgeReply.ok(snap);
  @override
  BridgeReply undo() => BridgeReply.ok(snap);
  @override
  BridgeReply redo() => BridgeReply.ok(snap);
  @override
  BridgeReply openProject(String p) => BridgeReply.ok(snap);
  @override
  BridgeReply saveProject(String p) => BridgeReply.ok(snap);
  @override
  BridgeReply newComposition(String name) => BridgeReply.ok(snap);
  @override
  BridgeReply importFootage(String p) => BridgeReply.ok(snap);
  @override
  BridgeReply setLayerSwitch(String c, String l, String s, bool v) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply editLayerSpan(String c, String l, String e, int f) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply setTransform(String c, String l, String p, double v) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply addMarker(String c, int f) => BridgeReply.ok(snap);
  @override
  BridgeReply addSolidLayer(String c) => BridgeReply.ok(snap);
  @override
  BridgeReply addTextLayer(String c) => BridgeReply.ok(snap);
  @override
  BridgeReply addCameraLayer(String c) => BridgeReply.ok(snap);
  @override
  BridgeReply addAdjustmentLayer(String c) => BridgeReply.ok(snap);
  @override
  BridgeReply addSequenceLayer(String c) => BridgeReply.ok(snap);
  @override
  BridgeReply addFootageLayer(String c, String itemId) => BridgeReply.ok(snap);
  @override
  BridgeReply reorderLayer(String c, String layerId, int newIndex) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply deleteLayer(String c, String l) => BridgeReply.ok(snap);
  @override
  BridgeReply duplicateLayer(String c, String l) => BridgeReply.ok(snap);
  @override
  BridgeReply setCompSettings(
          String c, String n, int w, int h, int fn, int fd, int df) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply togglePropertyAnimated(String c, String l, String p, int f) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply addKeyframe(String c, String l, String p, int f, double v) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply removeKeyframe(String c, String l, String p, int f) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply shiftKeyframes(
          String c, String l, String p, List<int> frames, int delta) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply setWorkAreaEdge(String c, int f, bool isOut) =>
      BridgeReply.ok(snap);
  @override
  List<BridgeEffectInfo> listEffects() => const [];
  @override
  BridgeReply addEffect(String c, String l, String e) => BridgeReply.ok(snap);
  @override
  BridgeReply removeEffect(String c, String l, String e) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply setEffectEnabled(String c, String l, String e, bool enabled) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply setEffectParamScalar(
          String c, String l, String e, String p, double v) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply setEffectParamColour(String c, String l, String e, String p,
          double r, double g, double b, double a) =>
      BridgeReply.ok(snap);
  // Bridge v0.4 stubs.
  @override
  BridgeReply setKeyframeInterp(String c, String l, String p, int f, String ii,
          String io, double si, double fi, double so, double fo) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply setRetimeEnabled(String c, String l, bool e) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply setRetimeSpeed(String c, String l, double s) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply setSegmentPreset(String c, String l, int f, String e) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply segmentToRate(String c, String l, int f) => BridgeReply.ok(snap);
  @override
  BridgeReply dragBoundary(String c, String l, int i, int f) =>
      BridgeReply.ok(snap);
  @override
  List<BridgeBlendMode> listBlendModes() => const [];
  @override
  BridgeReply setBlendMode(String c, String l, String m) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply setMatte(String c, String l, String s, String ch, bool i) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply setParent(String c, String l, String p) => BridgeReply.ok(snap);
  @override
  BridgeReply setMotionBlur(String c, bool e, double a, double p, int s) =>
      BridgeReply.ok(snap);
  @override
  BridgeReply addMask(String c, String l, String k) => BridgeReply.ok(snap);
  @override
  BridgeExportPreset exportPreset(String p, String c, String t) =>
      BridgeExportPreset.idle;
  @override
  BridgeReply startExport(String c, String s, String o) =>
      BridgeReply.ok(snap);
  @override
  BridgeExportState exportPoll() => BridgeExportState.idle;
  @override
  BridgeReply exportCancel() => BridgeReply.ok(snap);
}

/// A bridge that also offers composited-comp rendering. Extends [_FrameBridge]
/// (inheriting the single-layer `decodeFrame` and every document op) and adds
/// the [CompRenderBridge] capability, logging its comp render calls so a test
/// can assert which path the [PreviewSource] chose. [supports] toggles
/// `supportsCompRender`; [compResult] is what a render returns (null models a
/// no-adapter / failed render).
class _CompBridge extends _FrameBridge implements CompRenderBridge {
  final DecodedFrame? compResult;
  final bool supports;
  final List<String> renderedComps = [];

  _CompBridge(
    BridgeSnapshot snap, {
    DecodedFrame? decodeResult,
    this.compResult,
    this.supports = true,
  }) : super(snap, decodeResult);

  @override
  bool get supportsCompRender => supports;

  @override
  DecodedFrame? renderCompFrame(String compId, int frame, double scale) {
    renderedComps.add('$compId@$frame');
    return compResult;
  }
}

/// A bridge that offers the GPU scope pass (K-096 v1) on top of comp rendering.
/// Records its `renderScope` calls (kind + comp@frame) so a test can assert the
/// panel chose the engine path, and returns [traceBytes] (null models a declined
/// render → the panel's CPU fallback). [supportsScope] toggles the capability.
class _ScopeBridge extends _CompBridge implements ScopeTraceBridge {
  final bool supportsScope;
  final Uint8List? traceBytes;
  final List<String> scopeCalls = [];

  _ScopeBridge(
    super.snap, {
    super.decodeResult,
    super.compResult,
    this.supportsScope = true,
    this.traceBytes,
  });

  @override
  bool get supportsScopeTrace => supportsScope;

  @override
  Uint8List? renderScope(int kind, String compId, int frame, double scale,
      int bg, int trace, int red, int green, int blue) {
    scopeCalls.add('$kind:$compId@$frame');
    return traceBytes;
  }
}

/// A bridge that offers the zero-copy shared-texture path (K-177) AND the
/// read-back comp path (so the throttled Scopes render has something to return).
/// [sharedResult] is what `renderToShared` gives back (null models a machine
/// where the shared path is unavailable this frame); [supportsShared] toggles
/// the capability flag.
class _SharedBridge extends _FrameBridge
    implements SharedTextureBridge, CompRenderBridge {
  final SharedFrame? sharedResult;
  final DecodedFrame? compResult;
  final bool supportsShared;
  final List<String> renderedShared = [];

  _SharedBridge(
    BridgeSnapshot snap, {
    DecodedFrame? decodeResult,
    this.sharedResult,
    this.compResult,
    this.supportsShared = true,
  }) : super(snap, decodeResult);

  @override
  bool get supportsSharedTexture => supportsShared;

  @override
  SharedFrame? renderToShared(String compId, int frame) {
    renderedShared.add('$compId@$frame');
    return sharedResult;
  }

  @override
  bool get supportsCompRender => compResult != null;

  @override
  DecodedFrame? renderCompFrame(String compId, int frame, double scale) =>
      compResult;
}

/// Install a mock handler on the viewer-texture channel that answers `register`
/// with [textureId] (and swallows the rest), returning a tear-down that removes
/// it. When [textureId] is null, `register` returns null (a failed registration).
void Function() _mockViewerTextureChannel(
    WidgetTester tester, int? textureId) {
  const channel = MethodChannel(ViewerTextureController.channelName);
  final calls = <String>[];
  tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel,
      (call) async {
    calls.add(call.method);
    if (call.method == 'register') return textureId;
    return null;
  });
  return () => tester.binding.defaultBinaryMessenger
      .setMockMethodCallHandler(channel, null);
}

// --- Builders --------------------------------------------------------------

BridgeSwitches _switches({bool visible = true}) => BridgeSwitches(
      visible: visible,
      audible: true,
      locked: false,
      threeD: false,
      collapse: false,
      fx: true,
      solo: false,
      motionBlur: false,
    );

BridgeLayer _layer({
  required String name,
  required int inFrame,
  required int outFrame,
  BridgeLayerKind kind = BridgeLayerKind.footage,
  bool visible = true,
  int index = 0,
}) =>
    BridgeLayer(
      id: 'l$index',
      index: index,
      name: name,
      kind: kind,
      inFrame: inFrame,
      outFrame: outFrame,
      label: 0,
      switches: _switches(visible: visible),
    );

BridgeItem _footage(String name, {BridgeMediaStatus status = BridgeMediaStatus.ok}) =>
    BridgeItem(
      id: 'item-$name',
      name: name,
      kind: BridgeItemKind.footage,
      children: const [],
      status: status,
    );

BridgeComp _comp(List<BridgeLayer> layers,
        {int w = 4, int h = 4, int fps = 24, int frames = 48}) =>
    BridgeComp(
      width: w,
      height: h,
      fps: BridgeFps(fps, 1),
      frameCount: frames,
      layers: layers,
      markers: const [],
    );

BridgeSnapshot _snapshot(BridgeComp comp, List<BridgeItem> footage) =>
    BridgeSnapshot(
      items: [
        BridgeItem(
          id: 'c1',
          name: 'Scene',
          kind: BridgeItemKind.composition,
          children: const [],
          comp: comp,
        ),
        ...footage,
      ],
      canUndo: false,
      canRedo: false,
      path: null,
    );

/// A tiny opaque RGBA frame of one colour.
DecodedFrame _solid(int w, int h, int r, int g, int b) {
  final px = Uint8List(w * h * 4);
  for (var i = 0; i < px.length; i += 4) {
    px[i] = r;
    px[i + 1] = g;
    px[i + 2] = b;
    px[i + 3] = 255;
  }
  return DecodedFrame(width: w, height: h, rgba: px);
}

Widget _wrap(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: child,
      ),
    );

/// The viewer-texture channel a test injects — the same name the default
/// controller uses, so a mock handler on that name answers either.
const MethodChannel _defaultTextureChannel =
    MethodChannel(ViewerTextureController.channelName);

void main() {
  group('ViewerTextureController (K-177)', () {
    testWidgets('registers once and re-uses the id for the same handle',
        (tester) async {
      final calls = <String>[];
      tester.binding.defaultBinaryMessenger
          .setMockMethodCallHandler(_defaultTextureChannel, (call) async {
        calls.add('${call.method}:${(call.arguments as Map)['handle'] ?? ''}');
        if (call.method == 'register') return 11;
        return null;
      });
      final c = ViewerTextureController(channel: _defaultTextureChannel);
      expect(await c.ensureRegistered(0xAA, 16, 9), 11);
      expect(await c.ensureRegistered(0xAA, 16, 9), 11,
          reason: 'the same handle/size does not re-register');
      expect(calls.where((s) => s.startsWith('register')).length, 1);
      await c.frameReady();
      expect(calls, contains('frameReady:'));
      tester.binding.defaultBinaryMessenger
          .setMockMethodCallHandler(_defaultTextureChannel, null);
    });

    testWidgets('a changed handle unregisters the old and registers anew',
        (tester) async {
      final calls = <String>[];
      var next = 20;
      tester.binding.defaultBinaryMessenger
          .setMockMethodCallHandler(_defaultTextureChannel, (call) async {
        calls.add(call.method);
        if (call.method == 'register') return next++;
        return null;
      });
      final c = ViewerTextureController(channel: _defaultTextureChannel);
      expect(await c.ensureRegistered(0xAA, 16, 9), 20);
      expect(await c.ensureRegistered(0xBB, 16, 9), 21,
          reason: 'a new handle gets a new texture id');
      expect(calls, containsAllInOrder(['register', 'unregister', 'register']));
      tester.binding.defaultBinaryMessenger
          .setMockMethodCallHandler(_defaultTextureChannel, null);
    });

    testWidgets('a missing handler latches unavailable (falls back)',
        (tester) async {
      // No handler installed → the messenger returns a null response, which
      // MethodChannel surfaces as MissingPluginException. This needs the real
      // event loop (runAsync) so the platform round-trip actually completes.
      final c = ViewerTextureController(channel: _defaultTextureChannel);
      await tester.runAsync(() async {
        expect(await c.ensureRegistered(0xAA, 16, 9), isNull);
      });
      expect(c.available, isFalse);
      expect(c.textureId, isNull);
    });

    testWidgets('the Linux branch sends the DMA-BUF register payload',
        (tester) async {
      // When an fd is supplied (the Linux DMA-BUF shape), `register` carries the
      // fd + DRM metadata instead of a handle — the platform-conditional argument
      // pack. The channel name and lifecycle are unchanged.
      Map<Object?, Object?>? registerArgs;
      tester.binding.defaultBinaryMessenger
          .setMockMethodCallHandler(_defaultTextureChannel, (call) async {
        if (call.method == 'register') {
          registerArgs = call.arguments as Map<Object?, Object?>;
          return 7;
        }
        return null;
      });
      final c = ViewerTextureController(channel: _defaultTextureChannel);
      final id = await c.ensureRegistered(0, 32, 18,
          fd: 42, stride: 128, offset: 0, fourcc: 0x34324241, modifier: 0);
      expect(id, 7);
      expect(registerArgs, isNotNull);
      expect(registerArgs!['fd'], 42);
      expect(registerArgs!['stride'], 128);
      expect(registerArgs!['fourcc'], 0x34324241);
      expect(registerArgs!.containsKey('handle'), isFalse,
          reason: 'the DMA-BUF payload carries no NT handle');
      // The same fd/size is a no-op (no second register).
      expect(await c.ensureRegistered(0, 32, 18, fd: 42), 7);
      tester.binding.defaultBinaryMessenger
          .setMockMethodCallHandler(_defaultTextureChannel, null);
    });
  });

  group('resolvePreview', () {
    final items = [_footage('clip.mp4'), _footage('bg.mp4')];

    test('the topmost visible footage layer covering the frame wins', () {
      final comp = _comp([
        _layer(name: 'clip.mp4', inFrame: 0, outFrame: 48, index: 0),
        _layer(name: 'bg.mp4', inFrame: 0, outFrame: 48, index: 1),
      ]);
      final target = resolvePreview(comp, 10, items);
      expect(target, isNotNull);
      expect(target!.item.name, 'clip.mp4', reason: 'index 0 is topmost');
    });

    test('a hidden top layer is skipped in favour of the one below', () {
      final comp = _comp([
        _layer(name: 'clip.mp4', inFrame: 0, outFrame: 48, index: 0, visible: false),
        _layer(name: 'bg.mp4', inFrame: 0, outFrame: 48, index: 1),
      ]);
      final target = resolvePreview(comp, 10, items);
      expect(target!.item.name, 'bg.mp4');
    });

    test('a frame outside every span resolves to null', () {
      final comp = _comp([
        _layer(name: 'clip.mp4', inFrame: 12, outFrame: 24, index: 0),
      ]);
      expect(resolvePreview(comp, 5, items), isNull);
      expect(resolvePreview(comp, 24, items), isNull, reason: 'out_frame is exclusive');
      expect(resolvePreview(comp, 12, items), isNotNull, reason: 'in_frame inclusive');
    });

    test('the source frame is the comp frame minus the layer in-point', () {
      final comp = _comp([
        _layer(name: 'clip.mp4', inFrame: 30, outFrame: 90, index: 0),
      ]);
      final target = resolvePreview(comp, 45, items);
      expect(target!.sourceFrame, 15);
    });

    test('a non-footage layer is never previewed', () {
      final comp = _comp([
        _layer(name: 'solid', inFrame: 0, outFrame: 48, kind: BridgeLayerKind.solid),
      ]);
      expect(resolvePreview(comp, 0, [_footage('solid')]), isNull);
    });

    test('a footage layer with no matching item resolves to null', () {
      final comp = _comp([
        _layer(name: 'ghost.mp4', inFrame: 0, outFrame: 48),
      ]);
      expect(resolvePreview(comp, 0, items), isNull);
    });
  });

  group('scope maths', () {
    test('Rec.709 luma of pure colours', () {
      expect(luma8(255, 255, 255), closeTo(1.0, 1e-6));
      expect(luma8(0, 0, 0), 0.0);
      expect(luma8(255, 0, 0), closeTo(0.2126, 1e-6));
      expect(luma8(0, 255, 0), closeTo(0.7152, 1e-6));
      expect(luma8(0, 0, 255), closeTo(0.0722, 1e-6));
      expect(luma8(128, 128, 128), closeTo(128 / 255, 1e-6));
    });

    test('a solid grey lands all its energy on one waveform row', () {
      final frame = _solid(16, 16, 128, 128, 128);
      final grids = waveformCounts(frame.rgba, 16, 16, WaveMode.luma);
      final row = valueRow(luma8(128, 128, 128));
      var inRow = 0;
      for (var x = 0; x < scopeGrid; x++) {
        inRow += grids[0][row * scopeGrid + x];
      }
      final total = grids[0].fold<int>(0, (a, b) => a + b);
      expect(inRow, total);
      expect(total, 16 * 16);
    });

    test('a solid puts every pixel in one histogram bin per channel', () {
      final frame = _solid(10, 10, 255, 0, 64);
      final bins = histogramCounts(frame.rgba, 10, 10);
      expect(bins[0][scopeGrid - 1], 100, reason: 'red maxed → top bin');
      expect(bins[1][0], 100, reason: 'green zero → bottom bin');
      for (final chan in bins) {
        expect(chan.fold<int>(0, (a, b) => a + b), 100);
      }
    });

    test('neutral grey sits at the vectorscope centre', () {
      final frame = _solid(8, 8, 128, 128, 128);
      final grid = vectorscopeCounts(frame.rgba, 8, 8);
      var peakCell = 0;
      for (var c = 1; c < grid.length; c++) {
        if (grid[c] > grid[peakCell]) peakCell = c;
      }
      final mid = (scopeGrid - 1) ~/ 2;
      final px = peakCell % scopeGrid, py = peakCell ~/ scopeGrid;
      expect((px - mid).abs() <= 1, isTrue);
      expect((py - mid).abs() <= 1, isTrue);
    });

    test('vectorscope hue targets sit in the expected chroma directions', () {
      final targets = {for (final t in vectorTargets()) t.label: t};
      // Grid fractions: x grows right (Cb+), y grows down (Cr-).
      // Blue is the most positive Cb → right of centre; red the most positive
      // Cr → above centre.
      expect(targets['B']!.x, greaterThan(0.5));
      expect(targets['Yl']!.x, lessThan(0.5), reason: 'yellow is Cb-negative');
      expect(targets['R']!.y, lessThan(0.5), reason: 'red is Cr-positive (up)');
      expect(targets['Cy']!.y, greaterThan(0.5), reason: 'cyan is Cr-negative (down)');
      // All six are off-centre (non-zero saturation).
      for (final t in targets.values) {
        final off = (t.x - 0.5).abs() + (t.y - 0.5).abs();
        expect(off, greaterThan(0.05), reason: '${t.label} is off-centre');
      }
    });

    test('strides cap the sampled count', () {
      expect(scopeStrides(100, 100), [1, 1]);
      final s = scopeStrides(4000, 4000);
      expect(s[0], greaterThan(1));
      final sampled = (4000 / s[0]).ceil() * (4000 / s[1]).ceil();
      expect(sampled, lessThanOrEqualTo(scopeMaxSamples));
    });

    test('empty or short buffers never throw', () {
      expect(waveformCounts(Uint8List(0), 0, 0, WaveMode.luma)[0].every((c) => c == 0), isTrue);
      expect(histogramCounts(Uint8List.fromList([1, 2, 3]), 4, 4)[0].every((c) => c == 0), isTrue);
      expect(vectorscopeCounts(Uint8List(0), 2, 2).every((c) => c == 0), isTrue);
    });
  });

  group('Viewer widget', () {
    testWidgets('a decoded frame paints an image on the pasteboard',
        (tester) async {
      final snap = _snapshot(
        _comp([_layer(name: 'clip.mp4', inFrame: 0, outFrame: 48)]),
        [_footage('clip.mp4')],
      );
      final bridge = _FrameBridge(snap, _solid(4, 4, 200, 40, 40));
      final app = AppStateStub(bridge: bridge);

      await tester.runAsync(() async {
        await tester.pumpWidget(_wrap(ViewerPanel(app: app)));
        // Let the async ui.Image decode complete.
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      await tester.pump();

      expect(bridge.decoded, contains('item-clip.mp4@0'));
      expect(find.byType(RawImage), findsOneWidget);
    });

    testWidgets('missing footage shows the colour-bars slate with its path',
        (tester) async {
      final snap = _snapshot(
        _comp([_layer(name: 'gone.mp4', inFrame: 0, outFrame: 48)]),
        [_footage('gone.mp4', status: BridgeMediaStatus.missing)],
      );
      final app = AppStateStub(bridge: _FrameBridge(snap, null));

      await tester.pumpWidget(_wrap(ViewerPanel(app: app)));
      await tester.pump();

      expect(find.textContaining('Missing footage'), findsOneWidget);
      expect(find.textContaining('gone.mp4'), findsOneWidget);
    });

    testWidgets('play advances the playhead over pumped ticks', (tester) async {
      final snap = _snapshot(
        _comp([_layer(name: 'clip.mp4', inFrame: 0, outFrame: 48)], fps: 24),
        [_footage('clip.mp4')],
      );
      final app = AppStateStub(bridge: _FrameBridge(snap, _solid(4, 4, 10, 10, 10)));

      await tester.pumpWidget(_wrap(ViewerPanel(app: app)));
      expect(app.previewFrame, 0);

      app.togglePlay();
      await tester.pump(); // start the ticker
      // ~250 ms at 24 fps ≈ 6 frames.
      await tester.pump(const Duration(milliseconds: 120));
      await tester.pump(const Duration(milliseconds: 120));
      expect(app.playing, isTrue);
      expect(app.previewFrame, greaterThan(0));
    });
  });

  group('PreviewSource comp-vs-single-layer selection', () {
    final snap = _snapshot(
      _comp([_layer(name: 'clip.mp4', inFrame: 0, outFrame: 48)]),
      [_footage('clip.mp4')],
    );

    testWidgets('renders the whole comp when the bridge supports it',
        (tester) async {
      final bridge = _CompBridge(
        snap,
        decodeResult: _solid(4, 4, 1, 2, 3),
        compResult: _solid(8, 8, 20, 120, 200),
      );
      final app = AppStateStub(bridge: bridge);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app);
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });

      expect(source.compActive, isTrue, reason: 'the comp path is active');
      expect(bridge.renderedComps, contains('c1@0'),
          reason: 'it rendered the front comp at the playhead frame');
      expect(bridge.decoded, isEmpty,
          reason: 'the comp path never falls back to single-layer decode');
      expect(source.image, isNotNull, reason: 'the comp frame became an image');
      expect(source.displayedFrame, isNotNull,
          reason: 'the Scopes read the composited pixels');
      source.dispose();
    });

    testWidgets('falls back to single-layer when the comp render returns null',
        (tester) async {
      // compResult null models a machine with no GPU adapter (or a transient
      // failure): the render is attempted, then the single-layer path takes over.
      final bridge = _CompBridge(
        snap,
        decodeResult: _solid(4, 4, 5, 6, 7),
        compResult: null,
      );
      final app = AppStateStub(bridge: bridge);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app);
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });

      expect(bridge.renderedComps, contains('c1@0'),
          reason: 'the comp path was tried first');
      expect(source.compActive, isFalse, reason: 'it fell back to single-layer');
      expect(bridge.decoded, contains('item-clip.mp4@0'),
          reason: 'the single-layer decode ran for the covered footage layer');
      expect(source.image, isNotNull);
      source.dispose();
    });

    testWidgets('stays single-layer when the bridge does not support comp render',
        (tester) async {
      final bridge = _CompBridge(
        snap,
        decodeResult: _solid(4, 4, 9, 9, 9),
        compResult: _solid(8, 8, 1, 1, 1),
        supports: false,
      );
      final app = AppStateStub(bridge: bridge);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app);
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });

      expect(source.compActive, isFalse);
      expect(bridge.renderedComps, isEmpty,
          reason: 'an unsupported bridge is never asked to render a comp');
      expect(bridge.decoded, contains('item-clip.mp4@0'));
      source.dispose();
    });

    testWidgets('a plain DocumentBridge (no comp capability) stays single-layer',
        (tester) async {
      final bridge = _FrameBridge(snap, _solid(4, 4, 3, 3, 3));
      final app = AppStateStub(bridge: bridge);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app);
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });

      expect(source.compActive, isFalse);
      expect(bridge.decoded, contains('item-clip.mp4@0'));
      source.dispose();
    });
  });

  // The zero-copy shared-texture path (K-177): the engine renders into a shared
  // GPU texture and Flutter samples it directly. These prove the PreviewSource
  // orchestration and the airtight fallback chain, driving the platform channel
  // through a mock messenger (no real runner) and the render through a fake
  // SharedTextureBridge.
  group('PreviewSource shared-texture path (K-177)', () {
    final snap = _snapshot(
      _comp([_layer(name: 'clip.mp4', inFrame: 0, outFrame: 48)], w: 32, h: 18),
      [_footage('clip.mp4')],
    );

    testWidgets('shows the shared texture and feeds the Scopes via read-back',
        (tester) async {
      final removeChannel = _mockViewerTextureChannel(tester, 42);
      final bridge = _SharedBridge(
        snap,
        decodeResult: _solid(4, 4, 1, 1, 1),
        sharedResult: const SharedFrame(handle: 0xABCD, width: 32, height: 18),
        compResult: _solid(8, 8, 20, 120, 200),
      );
      final app = AppStateStub(bridge: bridge);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app,
            textureController:
                ViewerTextureController(channel: _defaultTextureChannel));
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });

      expect(source.sharedActive, isTrue,
          reason: 'the zero-copy path owns the picture');
      expect(source.textureId, 42, reason: 'the runner registered a texture id');
      expect(source.sharedAspect, closeTo(32 / 18, 1e-9));
      expect(bridge.renderedShared, contains('c1@0'),
          reason: 'it rendered the front comp into the shared texture');
      expect(source.displayedFrame, isNotNull,
          reason: 'a throttled read-back still feeds the Scopes their pixels');
      removeChannel();
      source.dispose();
    });

    testWidgets('the Linux DMA-BUF frame registers via the fd payload',
        (tester) async {
      // A shared frame carrying DMA-BUF fields (the Linux shape) drives the same
      // orchestration; the register call must carry the fd, not a handle.
      const channel = MethodChannel(ViewerTextureController.channelName);
      Map<Object?, Object?>? registerArgs;
      tester.binding.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, (call) async {
        if (call.method == 'register') {
          registerArgs = call.arguments as Map<Object?, Object?>;
          return 55;
        }
        return null;
      });
      final bridge = _SharedBridge(
        snap,
        decodeResult: _solid(4, 4, 1, 1, 1),
        sharedResult: const SharedFrame(
          handle: 0,
          width: 32,
          height: 18,
          fd: 77,
          stride: 128,
          offset: 0,
          fourcc: 0x34324241,
          modifier: 0,
        ),
        compResult: _solid(8, 8, 20, 120, 200),
      );
      final app = AppStateStub(bridge: bridge);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app,
            textureController:
                ViewerTextureController(channel: _defaultTextureChannel));
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });

      expect(source.sharedActive, isTrue);
      expect(source.textureId, 55);
      expect(registerArgs, isNotNull);
      expect(registerArgs!['fd'], 77);
      expect(registerArgs!['fourcc'], 0x34324241);
      expect(registerArgs!.containsKey('handle'), isFalse);
      tester.binding.defaultBinaryMessenger
          .setMockMethodCallHandler(channel, null);
      source.dispose();
    });

    testWidgets('a null shared render falls back to the read-back comp path',
        (tester) async {
      final removeChannel = _mockViewerTextureChannel(tester, 42);
      final bridge = _SharedBridge(
        snap,
        decodeResult: _solid(4, 4, 5, 6, 7),
        sharedResult: null, // no D3D12 adapter / a transient failure
        compResult: _solid(8, 8, 9, 9, 9),
      );
      final app = AppStateStub(bridge: bridge);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app,
            textureController:
                ViewerTextureController(channel: _defaultTextureChannel));
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });

      expect(source.sharedActive, isFalse, reason: 'the shared path declined');
      expect(bridge.renderedShared, contains('c1@0'),
          reason: 'the shared path was tried first');
      expect(source.compActive, isTrue, reason: 'it fell back to the comp path');
      expect(source.image, isNotNull);
      removeChannel();
      source.dispose();
    });

    testWidgets('a missing platform channel falls back for the session',
        (tester) async {
      // No mock handler installed → invokeMethod throws MissingPluginException,
      // so registration fails and the controller latches unavailable.
      final bridge = _SharedBridge(
        snap,
        decodeResult: _solid(4, 4, 3, 3, 3),
        sharedResult: const SharedFrame(handle: 0x1, width: 32, height: 18),
        compResult: _solid(8, 8, 4, 4, 4),
      );
      final app = AppStateStub(bridge: bridge);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app,
            textureController:
                ViewerTextureController(channel: _defaultTextureChannel));
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });

      expect(source.sharedActive, isFalse,
          reason: 'registration failed → read-back path');
      expect(source.compActive, isTrue);
      expect(source.image, isNotNull);
      source.dispose();
    });

    testWidgets('an unsupported bridge never attempts the shared path',
        (tester) async {
      final removeChannel = _mockViewerTextureChannel(tester, 42);
      final bridge = _SharedBridge(
        snap,
        decodeResult: _solid(4, 4, 2, 2, 2),
        sharedResult: const SharedFrame(handle: 0x1, width: 32, height: 18),
        compResult: _solid(8, 8, 6, 6, 6),
        supportsShared: false,
      );
      final app = AppStateStub(bridge: bridge);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app,
            textureController:
                ViewerTextureController(channel: _defaultTextureChannel));
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });

      expect(source.sharedActive, isFalse);
      expect(bridge.renderedShared, isEmpty,
          reason: 'an unsupported bridge is never asked to render shared');
      expect(source.compActive, isTrue, reason: 'the comp path took over');
      removeChannel();
      source.dispose();
    });

    testWidgets('the Viewer paints a Texture widget on the shared path',
        (tester) async {
      final removeChannel = _mockViewerTextureChannel(tester, 7);
      final bridge = _SharedBridge(
        snap,
        decodeResult: _solid(4, 4, 1, 1, 1),
        sharedResult: const SharedFrame(handle: 0x9, width: 32, height: 18),
        compResult: _solid(8, 8, 30, 30, 30),
      );
      final app = AppStateStub(bridge: bridge);
      // The app's own PreviewSource uses the default controller, which resolves
      // to the real channel name the mock handler above answers.
      await tester.runAsync(() async {
        await tester.pumpWidget(_wrap(ViewerPanel(app: app)));
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      await tester.pump();

      expect(find.byType(Texture), findsOneWidget,
          reason: 'the shared texture is shown with a Texture widget');
      expect(find.byType(RawImage), findsNothing,
          reason: 'no read-back image on the zero-copy path');
      removeChannel();
    });
  });

  // The perf pass render isolate (K-176): the same PreviewSource, but the heavy
  // render/decode is handed to an off-thread [FrameRenderer]. These prove the
  // async control flow — holding the last picture while a frame is in flight,
  // and latest-wins — without spawning a real isolate (a deferred fake stands
  // in for the worker).
  group('PreviewSource off-thread renderer (perf pass)', () {
    final snap = _snapshot(
      _comp([_layer(name: 'clip.mp4', inFrame: 0, outFrame: 48)]),
      [_footage('clip.mp4')],
    );

    testWidgets('holds no image until the worker replies, then shows the comp',
        (tester) async {
      final app = AppStateStub(bridge: _FrameBridge(snap, null));
      final renderer = _QueuedRenderer()..compResult = _solid(8, 8, 10, 20, 30);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app, renderer: renderer);
        // The render is in flight but unanswered: comp mode is entered, and the
        // Viewer holds (no image) rather than blanking.
        expect(renderer.compRequests, contains('c1@0'));
        expect(source.compActive, isTrue);
        expect(source.image, isNull);
        renderer.flush();
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      expect(source.image, isNotNull, reason: 'the composited frame landed');
      expect(source.displayedFrame, isNotNull);
      source.dispose();
    });

    testWidgets('latest-wins: a newer frame supersedes the queued one',
        (tester) async {
      final app = AppStateStub(bridge: _FrameBridge(snap, null));
      final renderer = _QueuedRenderer()..compResult = _solid(8, 8, 1, 2, 3);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app, renderer: renderer);
        expect(renderer.compRequests, ['c1@0'],
            reason: 'one render in flight for the initial frame');

        // The playhead jumps twice while c1@0 is still rendering: no second
        // request goes out (at most one in flight), only the latest is kept.
        app.advancePlayback(5);
        app.advancePlayback(9);
        expect(renderer.compRequests, ['c1@0'],
            reason: 'no concurrent request while one is in flight');

        renderer.flush(); // answer c1@0; its image decodes, then the drain runs
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      // The queued want collapsed to the LATEST frame — c1@9, skipping c1@5.
      expect(renderer.compRequests, contains('c1@9'));
      expect(renderer.compRequests, isNot(contains('c1@5')));
      source.dispose();
    });

    testWidgets('a null comp reply falls back to the single-layer decode',
        (tester) async {
      final app = AppStateStub(bridge: _FrameBridge(snap, _solid(4, 4, 7, 7, 7)));
      // supportsCompRender is true, but the render returns null (no adapter):
      // the async reply falls back to the single-layer decode.
      final renderer = _QueuedRenderer()
        ..compResult = null
        ..decodeResult = _solid(4, 4, 7, 7, 7);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app, renderer: renderer);
        renderer.flush(); // comp reply null → single-layer decode queued
        renderer.flush(); // answer the decode
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      expect(renderer.compRequests, contains('c1@0'));
      expect(renderer.decodeRequests, contains('item-clip.mp4@0'),
          reason: 'the single-layer decode ran after the comp declined');
      expect(source.compActive, isFalse);
      expect(source.image, isNotNull);
      source.dispose();
    });
  });

  // The document epoch and the decoded-frame LRU (TF: the Viewer preview did not
  // live-update on an edit — the Dart LRU keyed frames without the document
  // epoch, so a stale image was served even though the engine invalidated its
  // own cache). These prove the epoch scopes the cache: an edit re-renders the
  // same frame, pure playhead motion still hits the cache, and a reply landing
  // after an edit is dropped rather than banked.
  group('PreviewSource document-epoch cache (TF live-update)', () {
    final snap = _snapshot(
      _comp([_layer(name: 'clip.mp4', inFrame: 0, outFrame: 48)]),
      [_footage('clip.mp4')],
    );

    testWidgets('an edit re-renders the same frame rather than serving the cache',
        (tester) async {
      final app = AppStateStub(bridge: _FrameBridge(snap, null));
      final renderer = _QueuedRenderer()..compResult = _solid(8, 8, 10, 20, 30);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app, renderer: renderer);
        renderer.flush(); // answer the initial comp render
        await Future<void>.delayed(const Duration(milliseconds: 80));
        expect(renderer.compRequests, ['c1@0'],
            reason: 'one render for the initial frame');
        expect(source.image, isNotNull);

        // An edit adopts a new snapshot → documentEpoch bumps. The same
        // comp/frame is wanted, but its cache key now carries the new epoch, so
        // the LRU cannot serve the pre-edit picture: a fresh render must go out.
        renderer.compResult = _solid(8, 8, 200, 100, 50);
        app.undo(); // any edit path; bumps documentEpoch and notifies
        renderer.flush();
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      expect(renderer.compRequests, ['c1@0', 'c1@0'],
          reason: 'the edit forced a fresh render of the same frame');
      expect(source.image, isNotNull, reason: 'the post-edit frame is shown');
      source.dispose();
    });

    testWidgets('playhead motion alone re-uses the cache (no re-render)',
        (tester) async {
      final app = AppStateStub(bridge: _FrameBridge(snap, null));
      final renderer = _QueuedRenderer()..compResult = _solid(8, 8, 4, 5, 6);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app, renderer: renderer);
        renderer.flush(); // frame 0
        await Future<void>.delayed(const Duration(milliseconds: 80));
        app.advancePlayback(5); // fine-grained playhead move, no epoch bump
        renderer.flush(); // frame 5
        await Future<void>.delayed(const Duration(milliseconds: 80));
        app.advancePlayback(0); // back to a frame still warm at this epoch
        await Future<void>.delayed(const Duration(milliseconds: 20));
      });
      expect(renderer.compRequests, ['c1@0', 'c1@5'],
          reason: 'returning to frame 0 is served from the cache, not re-rendered');
      source.dispose();
    });

    testWidgets('a reply landing after an epoch bump is dropped, not banked',
        (tester) async {
      final app = AppStateStub(bridge: _FrameBridge(snap, null));
      final renderer = _QueuedRenderer()..compResult = _solid(8, 8, 1, 2, 3);
      late PreviewSource source;
      await tester.runAsync(() async {
        source = PreviewSource(app, renderer: renderer);
        expect(renderer.compRequests, ['c1@0'],
            reason: 'the initial render is in flight');

        // An edit adopts a new snapshot while the first render is unanswered.
        app.undo();

        // The stale reply lands: the epoch guard drops it (never banked under the
        // old key, never shown), then the drain issues a fresh render.
        renderer.flush();
        await Future<void>.delayed(const Duration(milliseconds: 20));
        expect(renderer.compRequests, ['c1@0', 'c1@0'],
            reason: 'a fresh render was issued after the stale reply was dropped');

        renderer.flush(); // answer the fresh render
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      expect(source.image, isNotNull,
          reason: 'the post-edit frame is shown once its fresh render lands');
      source.dispose();
    });
  });

  // The GPU scope pass (K-096 v1): the ScopesPanel prefers the engine trace when
  // the loaded library offers `render_scope`, and falls back to the CPU trace
  // when it does not. These drive the panel widget against a fake bridge.
  group('ScopesPanel GPU trace (K-096 v1)', () {
    final snap = _snapshot(
      _comp([_layer(name: 'clip.mp4', inFrame: 0, outFrame: 48)]),
      [_footage('clip.mp4')],
    );

    /// A valid opaque 256×256 RGBA trace the fake engine returns.
    Uint8List traceImage() {
      final b = Uint8List(scopeGrid * scopeGrid * 4);
      for (var i = 3; i < b.length; i += 4) {
        b[i] = 255;
      }
      return b;
    }

    testWidgets('asks the engine for the trace when the pass is offered',
        (tester) async {
      final bridge = _ScopeBridge(
        snap,
        decodeResult: _solid(4, 4, 1, 1, 1),
        compResult: _solid(8, 8, 20, 120, 200),
        traceBytes: traceImage(),
      );
      final app = AppStateStub(bridge: bridge);
      await tester.runAsync(() async {
        await tester.pumpWidget(_wrap(ScopesPanel(app: app)));
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      await tester.pump();

      expect(bridge.scopeCalls, isNotEmpty,
          reason: 'the panel asked the engine to trace');
      expect(bridge.scopeCalls.first, startsWith('0:c1@'),
          reason: 'the luma kind (index 0) of the front comp');
      // No CPU single-layer decode was needed to trace.
      expect(find.byType(CustomPaint), findsWidgets);
    });

    testWidgets('falls back to the CPU trace when the pass is unsupported',
        (tester) async {
      final bridge = _ScopeBridge(
        snap,
        decodeResult: _solid(4, 4, 5, 6, 7),
        compResult: _solid(8, 8, 9, 9, 9),
        supportsScope: false,
        traceBytes: traceImage(),
      );
      final app = AppStateStub(bridge: bridge);
      await tester.runAsync(() async {
        await tester.pumpWidget(_wrap(ScopesPanel(app: app)));
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      await tester.pump();

      expect(bridge.scopeCalls, isEmpty,
          reason: 'an unsupported engine is never asked to trace');
      // The panel still renders (the CPU trace over the shown comp frame).
      expect(find.byType(CustomPaint), findsWidgets);
    });
  });

  // The off-thread scope trace (TF round 5): the panel's engine trace rides the
  // render worker seam — never a synchronous renderScope on the UI isolate,
  // which blocked behind the engine's render lock for the length of an uncached
  // comp render. Request→callback, latest-wins, the K-130 hold.
  group('ScopesPanel off-thread trace (TF round 5)', () {
    final snap = _snapshot(
      _comp([_layer(name: 'clip.mp4', inFrame: 0, outFrame: 48)]),
      [_footage('clip.mp4')],
    );

    /// A valid opaque 256×256 RGBA trace the fake worker returns.
    Uint8List traceImage() {
      final b = Uint8List(scopeGrid * scopeGrid * 4);
      for (var i = 3; i < b.length; i += 4) {
        b[i] = 255;
      }
      return b;
    }

    /// The trace image the panel's scope painter currently holds, or null.
    /// Reaches the private painter dynamically by its public `trace` field.
    ui.Image? paintedTrace(WidgetTester tester) {
      for (final p
          in tester.widgetList<CustomPaint>(find.byType(CustomPaint))) {
        final painter = p.painter;
        if (painter != null &&
            painter.runtimeType.toString() == '_ScopePainter') {
          return (painter as dynamic).trace as ui.Image?;
        }
      }
      return null;
    }

    testWidgets('the trace request rides the renderer seam, not the UI-isolate '
        'bridge', (tester) async {
      final bridge = _ScopeBridge(
        snap,
        decodeResult: _solid(4, 4, 1, 1, 1),
        compResult: _solid(8, 8, 20, 120, 200),
        traceBytes: traceImage(),
      );
      final renderer = _QueuedRenderer()
        ..compResult = _solid(8, 8, 20, 120, 200)
        ..scopeResult = traceImage();
      final app =
          AppStateStub(bridge: bridge, previewRendererFactory: (_) => renderer);
      await tester.runAsync(() async {
        await tester.pumpWidget(_wrap(ScopesPanel(app: app)));
        expect(renderer.scopeRequests.first, '0:c1@0',
            reason: 'the luma trace request went to the renderer seam');
        expect(bridge.scopeCalls, isEmpty,
            reason: 'renderScope is never called synchronously any more');
        renderer.flush();
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      await tester.pump();
      expect(paintedTrace(tester), isNotNull, reason: 'the trace was built');
      expect(bridge.scopeCalls, isEmpty);
    });

    testWidgets(
        'latest-wins drops a superseded trace, and the held trace survives a '
        'pending one (K-130)', (tester) async {
      final bridge = _ScopeBridge(
        snap,
        decodeResult: _solid(4, 4, 1, 1, 1),
        compResult: null, // the comp render declines → single-layer feeds it
        traceBytes: null,
      );
      final renderer = _QueuedRenderer()
        ..compResult = null
        ..decodeResult = _solid(4, 4, 9, 9, 9)
        ..scopeResult = traceImage();
      final app =
          AppStateStub(bridge: bridge, previewRendererFactory: (_) => renderer);
      await tester.runAsync(() async {
        await tester.pumpWidget(_wrap(ScopesPanel(app: app)));
        expect(renderer.scopeRequests.length, 1);

        // The comp render declines, then the single-layer decode lands → the
        // shown generation advances while the gen-0 trace is still in flight.
        renderer.flush('comp');
        renderer.flush('decode');
        await Future<void>.delayed(const Duration(milliseconds: 80));
        expect(renderer.scopeRequests.length, 1,
            reason: 'at most one trace in flight — no second request queued');

        // The stale reply lands: dropped (never shown), and the newest wanted
        // trace is fetched instead.
        renderer.flush('scope');
        expect(renderer.scopeRequests.length, 2,
            reason: 'the superseded trace was dropped, the newest fetched');
        expect(paintedTrace(tester), isNull,
            reason: 'the dropped trace never became the picture');

        renderer.flush('scope');
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      await tester.pump();
      expect(paintedTrace(tester), isNotNull,
          reason: 'the newest trace was built');

      // A new frame lands; while its trace is pending the last one holds.
      await tester.runAsync(() async {
        app.advancePlayback(5);
        renderer.flush('comp');
        renderer.flush('decode');
        await Future<void>.delayed(const Duration(milliseconds: 80));
        expect(renderer.scopeRequests.length, 3,
            reason: 'the new frame requested its trace');
      });
      await tester.pump();
      expect(paintedTrace(tester), isNotNull,
          reason: 'the last trace holds while the new one is pending (K-130)');
    });
  });

  // The eyedropper's async readback fallback (TF round 5): its one-off
  // full-scale render rides the worker on its OWN guard, so it neither runs on
  // the UI isolate nor delays the Viewer's own in-flight render.
  group('PreviewSource.requestSampleFrame (eyedropper fallback)', () {
    final snap = _snapshot(
      _comp([_layer(name: 'clip.mp4', inFrame: 0, outFrame: 48)]),
      [_footage('clip.mp4')],
    );

    testWidgets('renders full-scale on its own guard, not the Viewer one',
        (tester) async {
      final app = AppStateStub(bridge: _FrameBridge(snap, null));
      final renderer = _QueuedRenderer()..compResult = _solid(8, 8, 1, 2, 3);
      late PreviewSource source;
      DecodedFrame? sampled;
      await tester.runAsync(() async {
        source = PreviewSource(app, renderer: renderer);
        expect(renderer.compRequests, ['c1@0'],
            reason: 'the Viewer render is in flight');
        source.requestSampleFrame((f) => sampled = f);
        expect(renderer.compRequests.length, 2,
            reason: 'the sample renders even while the Viewer request is in '
                'flight — its own guard, never _pendingKey');
        renderer.flush();
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      expect(sampled, isNotNull);
      expect(source.image, isNotNull,
          reason: 'the Viewer picture still landed');
      source.dispose();
    });
  });
}

/// A [FrameRenderer] that defers every reply until [flush], so a test can drive
/// the async control flow deterministically (the worker isolate stand-in).
class _QueuedRenderer implements FrameRenderer {
  @override
  bool get supportsCompRender => true;

  @override
  bool get supportsSharedTexture => false;

  final List<String> compRequests = [];
  final List<String> decodeRequests = [];
  final List<String> scopeRequests = [];
  final List<String> thumbRequests = [];
  final List<(String, void Function())> _pending = [];
  DecodedFrame? compResult;
  DecodedFrame? decodeResult;
  Uint8List? scopeResult;
  DecodedFrame? thumbResult;

  @override
  void requestComp(String compId, int frame, double scale, int generation,
      void Function(DecodedFrame?) onFrame) {
    compRequests.add('$compId@$frame');
    _pending.add(('comp', () => onFrame(compResult)));
  }

  @override
  void requestPreview(String compId, int frame, double scale, int generation,
      void Function(DecodedFrame?) onFrame) {
    _pending.add(('preview', () => onFrame(null)));
  }

  @override
  void requestShared(String compId, int frame, int generation,
      void Function(SharedFrame?) onFrame) {
    _pending.add(('shared', () => onFrame(null)));
  }

  @override
  void requestDecode(String itemId, int frame, int generation,
      void Function(DecodedFrame?) onFrame) {
    decodeRequests.add('$itemId@$frame');
    _pending.add(('decode', () => onFrame(decodeResult)));
  }

  @override
  void requestScopeTrace(int kind, String compId, int frame, double scale,
      int bg, int trace, int red, int green, int blue, int generation,
      void Function(Uint8List?) onTrace) {
    scopeRequests.add('$kind:$compId@$frame');
    _pending.add(('scope', () => onTrace(scopeResult)));
  }

  @override
  void requestThumbnail(String itemId, int maxEdge, int generation,
      void Function(DecodedFrame?) onFrame) {
    thumbRequests.add('$itemId@$maxEdge');
    _pending.add(('thumb', () => onFrame(thumbResult)));
  }

  /// Answer the queued requests — all of them, or (with [only]) just the ones
  /// of that kind, so a test can land replies in a chosen order.
  void flush([String? only]) {
    final pending = List<(String, void Function())>.from(
        only == null ? _pending : _pending.where((p) => p.$1 == only));
    _pending.removeWhere(
        (p) => only == null || p.$1 == only);
    for (final (_, run) in pending) {
      run();
    }
  }

  @override
  void dispose() {}
}
