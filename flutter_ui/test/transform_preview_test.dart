// The transform-preview fast path (the drag-a-numeric-field perf fix): a drag
// tick stages an in-memory-only engine edit and re-renders just the current
// frame, without the full commit path's undo entry, journal write, whole-
// document JSON round-trip and full-cache invalidation. Three layers:
// DragValueField's own drag-lifecycle callbacks (pure widget tests), the
// AppStateStub preview/commit/cancel API (documentEpoch and
// transformPreviewRevision bookkeeping), and PreviewSource's ephemeral
// preview-render path (coalescing a flood of ticks to one render in flight).

import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/bridge/bridge.dart';
import 'package:lumit_flutter/panels/preview_source.dart';
import 'package:lumit_flutter/state/app_state.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

Widget _harness(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: Overlay(
          initialEntries: [
            OverlayEntry(builder: (_) => Center(child: child)),
          ],
        ),
      ),
    );

/// A one-comp, one-footage-layer snapshot, minimal enough that
/// `frontCompIdResolved` resolves without any panel around it.
const _oneLayerJson = '''
{
  "ok": true,
  "items": [
    {
      "id": "c1", "name": "Scene", "kind": "composition", "children": [],
      "comp": {
        "width": 100, "height": 100, "fps": {"num": 30, "den": 1},
        "frame_count": 60,
        "layers": [
          {"id": "l0", "index": 0, "name": "top", "kind": "footage",
           "in_frame": 0, "out_frame": 60, "label": 0, "switches": {}}
        ],
        "markers": []
      }
    }
  ],
  "can_undo": false, "can_redo": false, "path": null
}''';

/// A bridge that implements both [DocumentBridge] and [PreviewTransformBridge]
/// — the real `LumitBridge`'s shape once ABI 11 is loaded. Records every
/// preview/commit/cancel call; [supportsPreview] lets a test flip the
/// capability flag without changing the fake's Dart type (so the `is
/// PreviewTransformBridge` gate — which the un-implementing [_PlainBridge]
/// below tests instead — always passes here).
class _PreviewFake
    implements DocumentBridge, PreviewTransformBridge, CompRenderBridge {
  final List<String> previewCalls = [];
  final List<String> commitCalls = [];
  bool cancelCalled = false;
  bool supportsPreview = true;
  DecodedFrame? previewFrameResult;

  BridgeReply _snap() => BridgeReply.parse(_oneLayerJson);

  @override
  BridgeReply snapshot() => _snap();

  @override
  BridgeReply setTransform(
      String compId, String layerId, String property, double value) {
    commitCalls.add('$compId/$layerId/$property=$value');
    return _snap();
  }

  @override
  bool get supportsPreviewTransform => supportsPreview;

  @override
  BridgeReply previewTransform(
      String compId, String layerId, String property, double value) {
    previewCalls.add('$compId/$layerId/$property=$value');
    return const BridgeReply.ok(null);
  }

  @override
  void cancelTransformPreview() {
    cancelCalled = true;
  }

  @override
  DecodedFrame? renderPreviewFrame(String compId, int frame, double scale) =>
      previewFrameResult;

  // CompRenderBridge: the comp path is not what's under test here (only the
  // separate preview-render route is), but PreviewSource gates its whole
  // preview-tick path on `supportsCompRender`, so this must read true.
  @override
  bool get supportsCompRender => true;
  @override
  DecodedFrame? renderCompFrame(String compId, int frame, double scale) =>
      null;

  @override
  dynamic noSuchMethod(Invocation invocation) => _snap();
}

/// A plain [DocumentBridge] that does NOT implement [PreviewTransformBridge]
/// at all — an old-ABI bridge, or any fake that predates the capability.
class _PlainBridge implements DocumentBridge {
  BridgeReply _snap() => BridgeReply.parse(_oneLayerJson);

  @override
  BridgeReply snapshot() => _snap();

  @override
  dynamic noSuchMethod(Invocation invocation) => _snap();
}

/// A [FrameRenderer] whose `requestPreview` replies are deferred until
/// [flushOne] is called, so a test can observe "one render in flight" and
/// drive the coalescing behaviour deterministically. Every other request
/// answers null/false synchronously (unused by these tests beyond letting
/// PreviewSource's constructor resolve once without erroring).
class _DeferredPreviewRenderer implements FrameRenderer {
  final List<void Function()> _pending = [];
  int previewCalls = 0;

  @override
  bool get supportsCompRender => true;
  @override
  bool get supportsSharedTexture => false;

  @override
  void requestComp(String compId, int frame, double scale, int generation,
          void Function(DecodedFrame?) onFrame) =>
      onFrame(null);

  @override
  void requestPreview(String compId, int frame, double scale, int generation,
      void Function(DecodedFrame?) onFrame) {
    previewCalls++;
    _pending.add(() => onFrame(
        DecodedFrame(width: 2, height: 2, rgba: Uint8List(2 * 2 * 4))));
  }

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

  /// Complete the oldest still-pending preview render.
  void flushOne() {
    if (_pending.isNotEmpty) _pending.removeAt(0)();
  }
}

void main() {
  group('DragValueField drag lifecycle', () {
    // A single moveBy comfortably clears Flutter's own pan-gesture recognition
    // slop (~18 logical px by default) so the gesture is unambiguously
    // recognised as a horizontal drag rather than a tap.
    const bigMove = Offset(40, 0);

    testWidgets('onChangeStart fires once at drag start', (tester) async {
      var starts = 0;
      await tester.pumpWidget(_harness(DragValueField(
        value: 10,
        min: 0,
        max: 100,
        onChanged: (_) {},
        onChangeStart: () => starts++,
      )));

      final gesture = await tester.startGesture(
          tester.getCenter(find.byType(DragValueField)));
      await gesture.moveBy(bigMove);
      await tester.pump();
      await gesture.up();
      await tester.pump();

      expect(starts, 1);
    });

    testWidgets(
        'onChangeLive fires per tick instead of onChanged, when provided',
        (tester) async {
      final live = <num>[];
      var changed = 0;
      await tester.pumpWidget(_harness(DragValueField(
        value: 10,
        min: 0,
        max: 100,
        onChanged: (_) => changed++,
        onChangeLive: (v) => live.add(v),
        onChangeEnd: (_) {},
      )));

      final gesture = await tester.startGesture(
          tester.getCenter(find.byType(DragValueField)));
      await gesture.moveBy(bigMove);
      await tester.pump();
      await gesture.moveBy(bigMove);
      await tester.pump();
      await gesture.up();
      await tester.pump();

      expect(live, isNotEmpty);
      expect(live, orderedEquals(live.toList()..sort()),
          reason: 'dragging right only ever increases the value');
      expect(changed, 0, reason: 'onChangeLive replaces onChanged mid-drag');
    });

    testWidgets(
        'onChangeEnd fires once on release with the last ticked value',
        (tester) async {
      final live = <num>[];
      num? ended;
      var endedCalls = 0;
      var changed = 0;
      await tester.pumpWidget(_harness(DragValueField(
        value: 10,
        min: 0,
        max: 100,
        onChanged: (_) => changed++,
        onChangeLive: (v) => live.add(v),
        onChangeEnd: (v) {
          endedCalls++;
          ended = v;
        },
      )));

      final gesture = await tester.startGesture(
          tester.getCenter(find.byType(DragValueField)));
      // The first move only resolves the gesture arena in favour of the
      // drag (Flutter's own pan-recognition slop); the first genuine
      // onHorizontalDragUpdate — and so the first live tick — arrives on
      // a subsequent move.
      await gesture.moveBy(bigMove);
      await tester.pump();
      await gesture.moveBy(bigMove);
      await tester.pump();
      await gesture.up();
      await tester.pump();

      expect(endedCalls, 1);
      expect(live, isNotEmpty);
      expect(ended, live.last);
      expect(changed, 0);
    });

    testWidgets(
        'a gesture cancel fires onDragCancel, not onChangeEnd or onChanged',
        (tester) async {
      num? ended;
      var changed = 0;
      var cancels = 0;
      await tester.pumpWidget(_harness(DragValueField(
        value: 10,
        min: 0,
        max: 100,
        onChanged: (_) => changed++,
        onChangeLive: (_) {},
        onChangeEnd: (v) => ended = v,
        onDragCancel: () => cancels++,
      )));

      final gesture = await tester.startGesture(
          tester.getCenter(find.byType(DragValueField)));
      await gesture.moveBy(bigMove);
      await tester.pump();
      await gesture.cancel();
      await tester.pump();

      expect(cancels, 1);
      expect(ended, isNull);
      expect(changed, 0);
    });

    testWidgets(
        'without onChangeLive/onChangeEnd, dragging falls back to onChanged '
        'every tick (byte-for-byte the pre-fix behaviour)', (tester) async {
      final changed = <num>[];
      await tester.pumpWidget(_harness(DragValueField(
        value: 10,
        min: 0,
        max: 100,
        onChanged: (v) => changed.add(v),
      )));

      final gesture = await tester.startGesture(
          tester.getCenter(find.byType(DragValueField)));
      await gesture.moveBy(bigMove);
      await tester.pump();
      await gesture.moveBy(bigMove);
      await tester.pump();
      await gesture.up();
      await tester.pump();

      // Every tick AND the release both call onChanged (there is no separate
      // onChangeEnd to take over).
      expect(changed, isNotEmpty);
      expect(changed.last, changed.reduce((a, b) => a > b ? a : b),
          reason: 'the release re-sends the last ticked value, not a new one');
    });
  });

  group('AppStateStub transform preview', () {
    test('previewTransform stages the engine edit without a big-notify or '
        'a documentEpoch bump, and bumps transformPreviewRevision', () {
      final fake = _PreviewFake();
      final app = AppStateStub(bridge: fake);
      final epochBefore = app.documentEpoch;
      var notifies = 0;
      app.addListener(() => notifies++);
      final revBefore = app.transformPreviewRevision.value;

      app.previewTransform('c1', 'l0', 'position_x', 42.0);

      expect(fake.previewCalls, ['c1/l0/position_x=42.0']);
      expect(app.transformEdits['l0/position_x'], 42.0);
      expect(app.documentEpoch, epochBefore,
          reason: 'a preview never adopts a snapshot');
      expect(notifies, 0,
          reason: 'a preview never fires the big ChangeNotifier');
      expect(app.transformPreviewRevision.value, revBefore + 1);
    });

    test('commitTransform commits for real — bumps documentEpoch, fires the '
        'big notifier, and calls the plain setTransform path', () {
      final fake = _PreviewFake();
      final app = AppStateStub(bridge: fake);
      final epochBefore = app.documentEpoch;
      var notifies = 0;
      app.addListener(() => notifies++);

      app.commitTransform('c1', 'l0', 'position_x', 55.0);

      expect(fake.commitCalls, ['c1/l0/position_x=55.0']);
      expect(app.documentEpoch, epochBefore + 1);
      expect(notifies, greaterThan(0));
    });

    test('cancelTransformPreview drops the session edit without touching '
        'documentEpoch, and tells the engine to drop its overlay', () {
      final fake = _PreviewFake();
      final app = AppStateStub(bridge: fake);
      app.previewTransform('c1', 'l0', 'position_x', 42.0);
      final epochBefore = app.documentEpoch;
      final revBefore = app.transformPreviewRevision.value;

      app.cancelTransformPreview('l0', 'position_x');

      expect(fake.cancelCalled, isTrue);
      expect(app.transformEdits.containsKey('l0/position_x'), isFalse);
      expect(app.documentEpoch, epochBefore);
      expect(app.transformPreviewRevision.value, revBefore + 1);
    });

    test('supportsPreviewTransform reflects the bridge capability flag', () {
      final fake = _PreviewFake()..supportsPreview = false;
      final app = AppStateStub(bridge: fake);
      expect(app.supportsPreviewTransform, isFalse);

      fake.supportsPreview = true;
      expect(app.supportsPreviewTransform, isTrue);
    });

    test('a bridge that does not implement PreviewTransformBridge at all '
        'makes previewTransform/cancelTransformPreview quiet no-ops', () {
      final app = AppStateStub(bridge: _PlainBridge());
      expect(app.supportsPreviewTransform, isFalse);

      // Neither call should throw, and neither should touch transformEdits —
      // the caller (effect_controls_panel.dart) is expected to gate on
      // supportsPreviewTransform before wiring these at all; this is the
      // defensive no-op for anything that calls them anyway.
      app.previewTransform('c1', 'l0', 'position_x', 1.0);
      expect(app.transformEdits.containsKey('l0/position_x'), isFalse);
      app.cancelTransformPreview('l0', 'position_x');
    });
  });

  group('PreviewSource transform-preview render', () {
    testWidgets(
        'a preview tick renders and shows a frame without an epoch bump',
        (tester) async {
      final fake = _PreviewFake()
        ..previewFrameResult =
            DecodedFrame(width: 2, height: 2, rgba: Uint8List(2 * 2 * 4));
      final app = AppStateStub(bridge: fake);
      final source = PreviewSource(app);
      addTearDown(source.dispose);

      final epochBefore = app.documentEpoch;
      final generationBefore = source.generation;
      await tester.runAsync(() async {
        app.previewTransform('c1', 'l0', 'position_x', 5.0);
        // ui.decodeImageFromPixels completes on a real engine callback, not
        // fake-async time — runAsync lets it actually complete.
        await Future<void>.delayed(const Duration(milliseconds: 80));
      });
      await tester.pump();

      expect(source.generation, greaterThan(generationBefore));
      expect(source.compActive, isTrue);
      expect(app.documentEpoch, epochBefore);
    });

    testWidgets(
        'a flood of ticks while a render is in flight coalesces to one '
        'more render, not one per tick', (tester) async {
      final fake = _PreviewFake();
      final app = AppStateStub(bridge: fake);
      final renderer = _DeferredPreviewRenderer();
      final source = PreviewSource(app, renderer: renderer);
      addTearDown(source.dispose);

      app.previewTransform('c1', 'l0', 'position_x', 10.0);
      expect(renderer.previewCalls, 1,
          reason: 'the first tick issues a render');

      app.previewTransform('c1', 'l0', 'position_x', 20.0);
      app.previewTransform('c1', 'l0', 'position_x', 30.0);
      expect(renderer.previewCalls, 1,
          reason: 'further ticks coalesce while one render is in flight');

      renderer.flushOne();
      await tester.pump();
      expect(renderer.previewCalls, 2,
          reason: 'exactly one more render for the coalesced ticks');
    });
  });
}
