// What the Mixer's keys do to the picked strips, and the solo rules the desk
// draws by.

import 'dart:math' as math;

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/mixer.dart';
import 'package:uuid/uuid.dart';

import '../state/app_state.dart';
import '../state/comp_time.dart';
import '../state/ui_state.dart';
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'mixer_live.dart';
import 'mixer_meter.dart' show faderFloorDb;
import 'mixer_rack.dart' show eachAsOne;

/// Run the Mixer's [action] on the picked strips of [desk]. False when it had
/// nothing to act on, so the key goes on to whoever else wants it.
bool runMixerAction({
  required String action,
  required LumitUiState ui,
  required LumitState app,
  required CompositionReference comp,
  required List<BridgeLayerEntry> desk,
  required Map<String, StripPosition> positions,
  required void Function(UuidValue id) show,
}) {
  final picked = [
    for (final e in desk)
      if (ui.selectedLayerIds.contains(e.layer.internallayerId)) e,
  ];
  switch (action) {
    case 'mixer.strip.prev' || 'mixer.strip.next':
      final step = action == 'mixer.strip.next' ? 1 : -1;
      final from = picked.isEmpty
          ? (step > 0 ? -1 : desk.length)
          : desk.indexOf(step > 0 ? picked.last : picked.first);
      show(desk[(from + step).clamp(0, desk.length - 1)].layer.internallayerId);
      return true;
    case 'mixer.fader.up' ||
          'mixer.fader.down' ||
          'mixer.fader.up.fine' ||
          'mixer.fader.down.fine' ||
          'mixer.fader.reset':
      if (picked.isEmpty) return false;
      final by = switch (action) {
        'mixer.fader.up' => 0.5,
        'mixer.fader.down' => -0.5,
        'mixer.fader.up.fine' => 0.1,
        'mixer.fader.down.fine' => -0.1,
        _ => null,
      };
      final frame = ui.playheadFrame.value;
      asOneUndoStep(app.project, () {
        for (final e in picked) {
          final now = positions[e.layer.internallayerId.toString()]?.volumeDb ??
              (e.info.volumeDb is BridgeScalar_Static
                  ? (e.info.volumeDb as BridgeScalar_Static).field0
                  : sampledScalar(e.info.volumeDb, timeOfFrame(comp, frame)));
          final db =
              by == null ? 0.0 : ((math.max(now, -60) + by) * 10).round() / 10;
          final target = volumeOf(e);
          mixerTouch(comp, target, db.clamp(faderFloorDb, 12));
          mixerRelease(comp, target, ui);
        }
      });
      return true;
    case 'mixer.mute' || 'mixer.solo':
      if (picked.isEmpty) return false;
      final mute = action == 'mixer.mute';
      final first = picked.first.info.switches;
      try {
        comp.setSwitchOnLayers(
            clicked: picked.first.layer.internallayerId,
            layers: [for (final e in picked) e.layer.internallayerId],
            switch_: mute ? BridgeLayerSwitch.audible : BridgeLayerSwitch.solo,
            on_: mute ? !first.audible : !first.solo);
      } catch (_) {}
      return true;
    case 'mixer.solo.alone':
      if (picked.isEmpty) return false;
      soloAlone(app, comp, desk, picked);
      return true;
    case 'mixer.automation.next':
      if (picked.isEmpty) return false;
      const modes = BridgeAutomationMode.values;
      final next =
          modes[(picked.first.info.automationMode.index + 1) % modes.length];
      eachAsOne(app, picked, (e) => e.layer.setAutomationMode(mode: next));
      return true;
  }
  return false;
}

/// Solo [picked] and nothing else on the desk, as one undo step.
void soloAlone(LumitState app, CompositionReference comp,
    List<BridgeLayerEntry> desk, List<BridgeLayerEntry> picked) {
  final ids = {for (final e in picked) e.layer.internallayerId};
  final others = [
    for (final e in desk)
      if (e.info.switches.solo && !ids.contains(e.layer.internallayerId))
        e.layer.internallayerId,
  ];
  asOneUndoStep(app.project, () {
    try {
      if (others.isNotEmpty) {
        comp.setSwitchOnLayers(
            clicked: others.first,
            layers: others,
            switch_: BridgeLayerSwitch.solo,
            on_: false);
      }
      comp.setSwitchOnLayers(
          clicked: ids.first,
          layers: ids.toList(),
          switch_: BridgeLayerSwitch.solo,
          on_: true);
    } catch (_) {
      // A locked layer refuses, and quietly.
    }
  });
}

/// The strips another strip's solo silences, by the engine's own rule: with
/// any strip soloed, a strip still sounds when it is soloed or solo safe or
/// fed by one that is, or when it feeds a soloed one. Solo safe keeps its own
/// way out but pulls nothing into the solo.
Set<UuidValue> soloSilenced(List<BridgeLayerEntry> desk) {
  if (!desk.any((e) => e.info.switches.solo)) return const {};
  bool marked(BridgeLayerEntry e) => e.info.switches.solo || e.info.soloSafe;
  Iterable<UuidValue> into(BridgeLayerEntry e) => [
        if (e.info.output case final out?) out,
        for (final s in e.info.sends) s.bus,
      ];
  final up = {
    for (final e in desk)
      if (marked(e)) e.layer.internallayerId
  };
  final down = {
    for (final e in desk)
      if (e.info.switches.solo) e.layer.internallayerId
  };
  // Walk the routes until nothing changes: a short desk, a few passes.
  for (var changed = true; changed;) {
    changed = false;
    for (final e in desk) {
      final id = e.layer.internallayerId;
      if (up.contains(id)) {
        for (final to in into(e)) {
          changed = up.add(to) || changed;
        }
      }
      if (!down.contains(id) && into(e).any(down.contains)) {
        changed = down.add(id) || changed;
      }
    }
  }
  return {
    for (final e in desk)
      if (!up.contains(e.layer.internallayerId) &&
          !down.contains(e.layer.internallayerId))
        e.layer.internallayerId,
  };
}
