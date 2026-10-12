// Which layers a razor click cuts.
//
// The rule is small and easy to get wrong in ways nobody notices until a stray
// Shift-click has cut six layers that were only *nearly* under the pointer, so
// it is a pure function with its own tests. What the cut then *does* — an edit
// point in a Sequence layer, a split into two layers otherwise — is the
// engine's, and is tested against it in test/frb/.

import 'package:flutter_rust_bridge/flutter_rust_bridge.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_razor.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

void main() {
  /// A layer entry spanning `[inFrame, outFrame)`, with only the fields the
  /// razor reads filled in.
  BridgeLayerEntry entry(String name, int inFrame, int outFrame,
      {BridgeLayerKind kind = BridgeLayerKind.solid}) {
    final id = UuidValue.fromString(const Uuid().v4());
    return BridgeLayerEntry(
      layer: LayerReference(
        internalprojectId: id,
        internalcompId: id,
        internallayerId: id,
      ),
      info: BridgeLayerInfo(
        collapseForced: false,
        volumeDb: const BridgeScalar.static_(0),
        pan: const BridgeScalar.static_(0),
        wired: false,
        textAnimators: const [],
        name: name,
        kind: kind,
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
        inFrame: inFrame,
        outFrame: outFrame,
        clipFrames: Int64List(0),
        clips: const [],
        transform: BridgeTransform(
          anchorX: const BridgeScalar.static_(0),
          anchorY: const BridgeScalar.static_(0),
          positionX: const BridgeScalar.static_(0),
          positionY: const BridgeScalar.static_(0),
          positionZ: const BridgeScalar.static_(0),
          scaleX: const BridgeScalar.static_(100),
          scaleY: const BridgeScalar.static_(100),
          rotation: const BridgeScalar.static_(0),
          rotationX: const BridgeScalar.static_(0),
          rotationY: const BridgeScalar.static_(0),
          opacity: const BridgeScalar.static_(100),
        ),
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

  final top = entry('top', 0, 100);
  final middle = entry('middle', 20, 60);
  final late = entry('late', 80, 200);
  final layers = [top, middle, late];

  group('A plain click', () {
    test('cuts the layer it landed on, and nothing else', () {
      final targets =
          razorTargets(layers, 30, clicked: middle, allLayers: false);
      expect(targets.map((e) => e.info.name), ['middle']);
    });
  });

  group('A Shift-click', () {
    test('cuts every layer the time falls inside', () {
      final targets = razorTargets(layers, 30, clicked: middle, allLayers: true);
      expect(targets.map((e) => e.info.name), ['top', 'middle']);
    });
  });
}
