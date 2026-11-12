// The Mixer's door to the live mix: a held fader, pot or send level is heard
// at once and written when it is let go, and the faders' places come back
// from the engine while the comp plays.

import 'package:lumit_flutter/src/rust/api/audio.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart';

import '../state/comp_time.dart';
import '../state/ui_state.dart';

/// [entry]'s fader, pot and the send to [bus].
BridgeMixTarget volumeOf(BridgeLayerEntry entry) =>
    BridgeMixTarget.volume(layer: entry.layer.internallayerId);
BridgeMixTarget panOf(BridgeLayerEntry entry) =>
    BridgeMixTarget.pan(layer: entry.layer.internallayerId);

/// Every move while [target] is held. Heard at once, and written as keys
/// when a pass records it.
void mixerTouch(
    CompositionReference comp, BridgeMixTarget target, double value) {
  try {
    comp.mixerTouch(target: target, value: value);
  } catch (_) {
    // A comp that has gone answers nothing.
  }
}

/// [target] let go. Stopped, the value is written as one undo step, a key at
/// the playhead when the property is keyed.
void mixerRelease(
    CompositionReference comp, BridgeMixTarget target, LumitUiState ui) {
  final at = timeOfFrame(comp, ui.playheadFrame.value);
  try {
    comp.mixerRelease(
        target: target,
        playheadSeconds:
            ui.playing.value || at.den == 0 ? null : at.num / at.den);
  } catch (_) {
    // A locked layer refuses, and quietly.
  }
}

/// A held [target] put back at [old]: touched there, then let go.
void mixerCancel(CompositionReference comp, BridgeMixTarget target,
    double old, LumitUiState ui) {
  mixerTouch(comp, target, old);
  mixerRelease(comp, target, ui);
}

/// Where one strip's controls stand now: its fader in dB, its pan and its
/// sends' levels in its send order.
class StripPosition {
  final double volumeDb;
  final double pan;
  final List<double> sends;

  const StripPosition(
      {required this.volumeDb, required this.pan, this.sends = const []});

  @override
  bool operator ==(Object other) =>
      other is StripPosition &&
      other.volumeDb == volumeDb &&
      other.pan == pan &&
      other.sends.length == sends.length &&
      Iterable.generate(sends.length).every((i) => other.sends[i] == sends[i]);

  @override
  int get hashCode => Object.hash(volumeDb, pan, sends.length);
}

/// Where every strip's controls stand at the clock, by layer id, the master
/// under the empty id.
Map<String, StripPosition> mixerPositions() => {
      for (final p in audioPositions())
        p.layer: StripPosition(
            volumeDb: p.volumeDb, pan: p.pan, sends: List.of(p.sends)),
    };
