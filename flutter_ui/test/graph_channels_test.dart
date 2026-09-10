// Which curve a selected property row resolves to on the graph. The Volume
// row used to resolve to nothing: its keys drew as lane diamonds and on the
// rubber band, but the graph had no channel for them, so Delete, the eases
// and the block tools all fell through with the keys still there. Pure, so it
// is checked against a stub entry rather than the engine.

import 'package:flutter_rust_bridge/flutter_rust_bridge.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/graph_channels.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

BridgeKeyframe key(int seconds, double value) => BridgeKeyframe(
      time: BridgeRational(num: seconds, den: 1),
      value: value,
      interpIn: const BridgeSideInterp.linear(),
      interpOut: const BridgeSideInterp.linear(),
    );

/// A layer entry with only the fields the channel builder reads filled in.
BridgeLayerEntry entry({required BridgeScalar volumeDb}) {
  final id = UuidValue.fromString(const Uuid().v4());
  return BridgeLayerEntry(
    layer: LayerReference(
      internalprojectId: id,
      internalcompId: id,
      internallayerId: id,
    ),
    info: BridgeLayerInfo(
      volumeDb: volumeDb,
      pan: const BridgeScalar.static_(0),
      wired: false,
      textAnimators: const [],
      name: 'Music',
      kind: BridgeLayerKind.audio,
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
      outFrame: 10,
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

void main() {
  group('the Volume row resolves to a graph channel', () {
    final keyed = BridgeScalar.keyframed([key(0, 0), key(2, -12)]);
    final music = entry(volumeDb: keyed);
    final id = music.layer.internallayerId.toString();
    final path = '${audioPath(id)}/volume';

    test('carrying the same keys the lane diamonds draw', () {
      final channels = graphChannels(layers: [music], selected: [path]);
      expect(channels, hasLength(1));
      final channel = channels.single;
      expect(channel.volume, isTrue);
      expect(channel.path, path);
      expect(channel.scalar, keyed);
      expect([for (final k in channel.keys) k.value], [0.0, -12.0]);
    });

    test('in decibels', () {
      final channel = graphChannels(layers: [music], selected: [path]).single;
      expect(graphChannelUnit(channel), 'dB');
    });

    test('a static Volume is a channel too, as a static transform is', () {
      final still = entry(volumeDb: const BridgeScalar.static_(-6));
      final channels = graphChannels(
        layers: [still],
        selected: [
          '${audioPath(still.layer.internallayerId.toString())}/volume'
        ],
      );
      expect(channels.single.isStatic, isTrue);
      expect(channels.single.staticValue, -6);
    });
  });
}
