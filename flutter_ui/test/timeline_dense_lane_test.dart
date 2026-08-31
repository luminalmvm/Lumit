// A dense lane costs a handful of widgets, not one per key.
//
// An imported After Effects camera arrives with a baked keyframe per frame —
// thousands per property — and the lane used to build a `Positioned` +
// `MouseRegion` + `GestureDetector` for every one of them, which put the whole
// panel's rebuild, layout and hit-testing in the tens of thousands of widgets
// the moment such a layer was twirled open (the ~10 fps report on the real
// project). Past `keyLaneSlotBudget` the lane now mounts one hit-strip that
// resolves the key from the pointer; below it, the per-key widgets — and the
// widget keys the other timeline tests drive by name — are exactly as before.
//
// The painter half is pinned too: marks outside the canvas clip are never
// walked into paths, so a lane tens of thousands of pixels wide costs what the
// window shows, not what the comp holds.

import 'dart:ui' as ui;


import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/graph_maths.dart';
import 'package:lumit_flutter/panels/key_block.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/panels/timeline_metrics_frb.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/timeline_key_block_frb.dart';
import 'package:lumit_flutter/panels/timeline_key_lane_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:uuid/uuid.dart';

/// A layer entry with only the fields the lane reads filled in — the same
/// stub the razor test builds, because [KeyLane] never looks inside it.
///
/// [transform] is for the tests that *do* look inside: a shut layer's summary
/// row is gathered from whatever the layer has keyed.
BridgeLayerEntry _entry({BridgeTransform? transform}) {
  final id = UuidValue.fromString(const Uuid().v4());
  return BridgeLayerEntry(
    layer: LayerReference(
      internalprojectId: id,
      internalcompId: id,
      internallayerId: id,
    ),
    info: BridgeLayerInfo(
      volumeDb: const BridgeScalar.static_(0),
      pan: const BridgeScalar.static_(0),
      wired: false,
      textAnimators: const [],
      name: 'camera',
      kind: BridgeLayerKind.solid,
      switches: const BridgeLayerSwitches(
        visible: true,
        audible: true,
        locked: false,
        solo: false,
        threeD: false,
        fx: true,
        motionBlur: false,
        collapse: false,
        shy: false,
        acceptsLights: true,
      ),
      blend: 0,
      span: const BridgeSpan(
        inPoint: BridgeRational(num: 0, den: 1),
        outPoint: BridgeRational(num: 1, den: 1),
        startOffset: BridgeRational(num: 0, den: 1),
      ),
      inFrame: 0,
      outFrame: 200,
      clipFrames: Int64List(0),
      clips: const [],
      transform: transform ?? _still,
      axisModes: const BridgeAxisModes(
        anchor: BridgeAxisMode.combined,
        position: BridgeAxisMode.combined,
        scale: BridgeAxisMode.linked,
      ),
      effects: const [],
      styles: const [],
      label: 0,
      masks: const [],
      paint: const [],
      shapeContents: const [],
      markers: const [],
      flow: false,
      flowInputRate: const BridgeScalar.static_(0),
      trackCorrected: false,
    ),
  );
}

/// A transform with nothing animated on it.
const _still = BridgeTransform(
  anchorX: BridgeScalar.static_(0),
  anchorY: BridgeScalar.static_(0),
  positionX: BridgeScalar.static_(0),
  positionY: BridgeScalar.static_(0),
  positionZ: BridgeScalar.static_(0),
  scaleX: BridgeScalar.static_(100),
  scaleY: BridgeScalar.static_(100),
  rotation: BridgeScalar.static_(0),
  rotationX: BridgeScalar.static_(0),
  rotationY: BridgeScalar.static_(0),
  opacity: BridgeScalar.static_(100),
);

/// The same, with [position] (its first axis), [rotation] and [opacity] keyed
/// — three fold rows of the layer's own, whatever times each is given.
BridgeTransform _keyed({
  List<BridgeKeyframe> position = const [],
  List<BridgeKeyframe> rotation = const [],
  List<BridgeKeyframe> opacity = const [],
}) =>
    BridgeTransform(
      anchorX: const BridgeScalar.static_(0),
      anchorY: const BridgeScalar.static_(0),
      positionX: BridgeScalar.keyframed(position),
      positionY: const BridgeScalar.static_(0),
      positionZ: const BridgeScalar.static_(0),
      scaleX: const BridgeScalar.static_(100),
      scaleY: const BridgeScalar.static_(100),
      rotation: BridgeScalar.keyframed(rotation),
      rotationX: const BridgeScalar.static_(0),
      rotationY: const BridgeScalar.static_(0),
      opacity: BridgeScalar.keyframed(opacity),
    );

/// `count` linear keys, one per frame at 25 fps — the shape a baked AE
/// import has.
List<BridgeKeyframe> _bakedKeys(int count) => [
      for (var i = 0; i < count; i++)
        BridgeKeyframe(
          time: BridgeRational(num: i, den: 25),
          value: i.toDouble(),
          interpIn: const BridgeSideInterp.linear(),
          interpOut: const BridgeSideInterp.linear(),
        ),
    ];

void main() {
  const laneWidth = 806.0;
  const rowId = 'layer/transform/volume';

  Widget harness(
    List<BridgeKeyframe> keys, {
    int? frames,
    void Function(int, bool)? onSelectKey,
    ValueChanged<KeyStretch>? onMoveKeys,
    void Function(int, Offset)? onKeyMenu,
  }) {
    frames ??= keys.length;
    return Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: Center(
          child: SizedBox(
            width: laneWidth,
            height: 16,
            child: KeyLane(
              entry: _entry(),
              row: const FoldVolumeRow(depth: 2),
              rowId: rowId,
              keys: keys,
              axis: TimelineAxis(frames: frames, width: laneWidth),
              fps: 25,
              fpsNum: 25,
              fpsDen: 1,
              magnet: false,
              barShift: 0,
              snapTargets: const [],
              selectedKeys: const {},
              stretch: ValueNotifier<KeyStretch?>(null),
              onSelectKey: onSelectKey ?? (_, __) {},
              onKeyMenu: onKeyMenu ?? (_, __) {},
              onMoveKeys: onMoveKeys ?? (_) {},
              onChanged: () {},
            ),
          ),
        ),
      ),
    );
  }

  testWidgets('a dense lane mounts the strip, not a widget per key',
      (tester) async {
    await tester.pumpWidget(harness(_bakedKeys(2000)));
    expect(
      find.byKey(const ValueKey<String>('tl-key-strip-$rowId')),
      findsOneWidget,
    );
    expect(
      find.byKey(const ValueKey<String>('tl-key-slot-$rowId#0')),
      findsNothing,
    );
    // The whole lane, diamonds and hit surface included, is a bounded handful
    // of elements — not thousands. Fails loudly if the per-key path leaks back.
    var elements = 0;
    void count(Element e) {
      elements += 1;
      e.visitChildren(count);
    }

    WidgetsBinding.instance.rootElement!.visitChildren(count);
    expect(elements, lessThan(100),
        reason: 'a 2,000-key lane grew $elements elements');
  });

  testWidgets('a hand-keyed lane keeps its per-key slots and their names',
      (tester) async {
    await tester.pumpWidget(harness(_bakedKeys(3)));
    expect(
      find.byKey(const ValueKey<String>('tl-key-$rowId#0')),
      findsOneWidget,
    );
    expect(
      find.byKey(const ValueKey<String>('tl-key-strip-$rowId')),
      findsNothing,
    );
  });

  testWidgets('the strip still selects and drags the key under the pointer',
      (tester) async {
    final selected = <int>[];
    KeyStretch? moved;
    final keys = _bakedKeys(200);
    await tester.pumpWidget(harness(
      keys,
      onSelectKey: (i, _) => selected.add(i),
      onMoveKeys: (m) => moved = m,
    ));
    final strip = find.byKey(const ValueKey<String>('tl-key-strip-$rowId'));
    final axis = TimelineAxis(frames: 200, width: laneWidth);
    final origin = tester.getTopLeft(strip);
    // Key #100's own pixel, dead centre of the row.
    final at = origin + Offset(axis.xOf(100), 8);
    final gesture = await tester.startGesture(at);
    await gesture.moveBy(const Offset(60, 0));
    await tester.pump();
    await gesture.up();
    await tester.pump();
    expect(selected, [100], reason: 'the drag should have taken key 100');
    expect(moved, isNotNull, reason: 'the release should commit the travel');
  });

  testWidgets('ground beyond the keys falls through the strip',
      (tester) async {
    // A hundred baked keys crowded into the first frames of a long comp: the
    // right-hand stretch of the lane has no key within a slot's reach, and a
    // press there must fall through to the marquee below, exactly as the
    // ground between per-key slots always has.
    await tester.pumpWidget(harness(_bakedKeys(100), frames: 2000));
    final strip = find.byKey(const ValueKey<String>('tl-key-strip-$rowId'));
    final origin = tester.getTopLeft(strip);
    final hit = tester.hitTestOnBinding(origin + const Offset(700, 8));
    expect(
      hit.path.any((e) => '${e.target.runtimeType}' == '_RenderKeyStripHit'),
      isFalse,
      reason: 'open ground must not be claimed by the strip',
    );
    // And the keys themselves still are claimed.
    final onKey = tester.hitTestOnBinding(origin + const Offset(10, 8));
    expect(
      onKey.path.any((e) => '${e.target.runtimeType}' == '_RenderKeyStripHit'),
      isTrue,
      reason: 'a key mark must be grabbable through the strip',
    );
  });

  test('the painter walks the clip, not the comp', () {
    final frames = [for (var i = 0; i < 10000; i++) i.toDouble()];
    // Ten thousand keys laid over forty thousand pixels, window on the first
    // eight hundred: the walk may only pay for what the clip shows.
    final axis = TimelineAxis(frames: 10000, width: 40000);
    final painter = LaneKeysPainter(
      frames: frames,
      selected: const {},
      axis: axis,
      colour: const Color(0xFF808080),
      chosen: const Color(0xFFFFFFFF),
    );
    final canvas = _CountingCanvas(const Rect.fromLTRB(0, 0, 800, 16));
    painter.paint(canvas, const Size(40000, 16));
    expect(canvas.paths, greaterThan(0));
    expect(canvas.paths, lessThan(300),
        reason: '${canvas.paths} paths drawn for an 800 px window');
  });

  /// **A shut layer's row draws places, not keys.** The summary row converted
  /// every key on the layer to a frame on every rebuild of the bar, so the
  /// baked camera paid for seventeen thousand conversions to draw two
  /// thousand diamonds, each one under six others.
  group("A shut layer's summary row", () {
    List<BridgeKeyframe> summaryOf(BridgeTransform transform) => layerRows(
          layers: [_entry(transform: transform)],
          open: const {},
          rowHeight: 16,
          hasAudio: const {},
        ).single.summaryKeys;

    test('draws one diamond a frame, not one a key', () {
      final baked = _bakedKeys(300);
      final summary =
          summaryOf(_keyed(position: baked, rotation: baked, opacity: baked));
      expect(summary.length, 300, reason: '900 keys stand on 300 frames');
      expect(
        summary.map((k) => rationalSeconds(k.time)).toSet().length,
        300,
        reason: 'and no time is named twice',
      );
    });

    test('keeps every frame of a hand-keyed layer', () {
      final summary = summaryOf(_keyed(
        position: _bakedKeys(2),
        opacity: [
          const BridgeKeyframe(
            time: BridgeRational(num: 5, den: 25),
            value: 50,
            interpIn: BridgeSideInterp.linear(),
            interpOut: BridgeSideInterp.linear(),
          ),
        ],
      ));
      expect(
        summary.map((k) => rationalSeconds(k.time)).toSet(),
        {0.0, 1 / 25, 5 / 25},
        reason: 'two properties, three distinct times, three diamonds',
      );
    });
  });
}

/// Counts [drawPath] calls and answers the clip; everything else is inert.
class _CountingCanvas implements Canvas {
  final Rect clip;
  int paths = 0;
  _CountingCanvas(this.clip);

  @override
  void drawPath(ui.Path path, ui.Paint paint) => paths += 1;

  @override
  Rect getLocalClipBounds() => clip;

  @override
  dynamic noSuchMethod(Invocation invocation) => null;
}
