// Phase F2 tests: preview resolution logic, scope maths (pure), and the Viewer
// widget (a fake bridge decodes a synthetic frame → an image; a missing item
// shows the slate; play advances the playhead over pumped ticks).

import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/bridge/bridge.dart';
import 'package:lumit_flutter/panels/preview_source.dart';
import 'package:lumit_flutter/panels/scope_maths.dart';
import 'package:lumit_flutter/panels/viewer_panel.dart';
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

void main() {
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
}
