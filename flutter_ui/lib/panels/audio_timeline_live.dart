// What the Audio timeline asks of the live mix: a lane or band moved while
// the comp plays, the stretch a pass is writing, and the picture of a mix.
// One function per call, so this file is the only place those calls meet
// the panel.

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart';
import 'package:uuid/uuid.dart';

import 'audio_master_track.dart' show masterLaneId;
import 'automation_lane.dart'
    show LaneKind, lanePanPath, laneSendPath, laneVolumePath;

/// The control a lane or band rides: a layer's Volume, Pan or send to [bus],
/// or with no [layer] the master's Volume.
typedef LiveTarget = ({LayerReference? layer, LaneKind kind, UuidValue? bus});

/// One point a pass has written, in comp seconds and the control's own units.
typedef PassPoint = ({double seconds, double value});

/// Whether a held control is recorded: the comp playing, and the control's
/// track in a mode that writes (Trim writes its offset).
bool laneRecords(BridgeAutomationMode mode, {required bool playing}) =>
    playing && mode != BridgeAutomationMode.read;

BridgeMixTarget _mixTarget(LiveTarget target) {
  final layer = target.layer?.internallayerId;
  if (layer == null) return const BridgeMixTarget.master();
  return switch (target.kind) {
    LaneKind.volume => BridgeMixTarget.volume(layer: layer),
    LaneKind.pan => BridgeMixTarget.pan(layer: layer),
    LaneKind.send => BridgeMixTarget.send(layer: layer, bus: target.bus!),
  };
}

/// A move on [target] while it is held and recorded: heard at once, and
/// written as keys when playback stops. Says whether the mix took it, so the
/// release writes nothing of its own.
bool laneTouch(CompositionReference comp, LiveTarget target, double value) {
  try {
    comp.mixerTouch(target: _mixTarget(target), value: value);
    return true;
  } catch (_) {
    return false;
  }
}

/// [target] let go after [laneTouch] took it.
void laneRelease(CompositionReference comp, LiveTarget target) {
  try {
    comp.mixerRelease(target: _mixTarget(target));
  } catch (_) {
    // A comp that has gone has nothing to let go of.
  }
}

/// The lane a pass line is drawn on.
String _laneIdOf(BridgeMixTarget target) => switch (target) {
      BridgeMixTarget_Volume(:final layer) => laneVolumePath('$layer'),
      BridgeMixTarget_Pan(:final layer) => lanePanPath('$layer'),
      BridgeMixTarget_Send(:final layer, :final bus) =>
        laneSendPath('$layer', bus),
      BridgeMixTarget_Master() => masterLaneId,
    };

/// What the running pass has drawn since point [from] of pass [pass]: its
/// number, the next point to ask from, and the new points and their controls
/// by lane id. Null when there is nothing to ask.
({
  int pass,
  int next,
  Map<String, List<PassPoint>> points,
  Map<String, BridgeMixTarget> targets
})? passPointsSince(CompositionReference comp,
    {required int pass, required int from}) {
  final BridgeAutomationPassSince since;
  try {
    since = comp.automationPassSince(pass: pass, from: from);
  } catch (_) {
    return null;
  }
  return (
    pass: since.pass,
    next: since.next,
    points: {
      for (final line in since.lines)
        _laneIdOf(line.target): [
          for (final p in line.points) (seconds: p.seconds, value: p.value),
        ],
    },
    targets: {
      for (final line in since.lines) _laneIdOf(line.target): line.target,
    },
  );
}

/// The mix's peaks over a window of comp seconds: the whole mix with no
/// [bus], else what is routed to the bus before its rack and fader.
Future<BridgeAudioPeaks?> busPeaks(
  CompositionReference comp, {
  UuidValue? bus,
  required double startSeconds,
  required double endSeconds,
  required int buckets,
}) =>
    comp.audioMixPeaks(
        startSeconds: startSeconds,
        endSeconds: endSeconds,
        buckets: buckets,
        bus: bus);
