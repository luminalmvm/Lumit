// Manual screenshots, sweep: the Audio workspace (K-574, K-690/K-691,
// K-697/K-698/K-699), photographed on a real project rather than a staged one.
//
// audio-workspace · audio-mixer · audio-panel · audio-timeline-spectrogram ·
// audio-graph-duck · audio-playback…
//
// Unlike its sister sweeps this one does not build a comp: the Mixer draws a
// strip per *sounding* row and the lanes draw real peaks, so a staged solid
// would photograph an empty desk. It opens a document instead — the path in
// `LUMIT_SHOTS_PROJECT`, with its media reachable beside it, which is the
// condition docs/impl/ui-performance.md §2.1 calls "media beside project:
// resolves" — fronts the comp named in `LUMIT_SHOTS_COMP` (or the first one),
// and works the real controls: the twirls, the lane-mode chip, the beat
// source, Generate, the templates, the transport.
//
// Playback comes **last** on purpose: it is the one step that drives the
// engine hardest, and a sweep that leads with it would take every other
// picture down with it.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1
//   $env:LUMIT_SHOTS_PROJECT='<dir>/Something.lum'
//   flutter run -d windows -t tool/shots/shots_audio.dart

import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/audio_panel_frb.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/panels/mixer_panel_frb.dart';
import 'package:lumit_flutter/panels/spectral_lane_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';
import 'package:lumit_flutter/src/rust/api/beats.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/viewer_view.dart';

import 'shots_common.dart';

void say(Object? what) {
  // ignore: avoid_print
  print('AUDIO SWEEP: $what');
}

/// A wheel notch over [at], the way a long outline is scrolled by hand — the
/// audio row of a sixty-layer comp is off the bottom of the list, and a twirl
/// cannot be tapped before it is on screen.
Future<void> scrollAt(Offset at, double dy, {int times = 1}) async {
  for (var i = 0; i < times; i++) {
    GestureBinding.instance.handlePointerEvent(PointerScrollEvent(
      position: at,
      scrollDelta: Offset(0, dy),
    ));
    await pause(0.08);
  }
  await pause(0.5);
}

/// The outline, scrolled to its end — where the last layer and everything its
/// twirls open live.
Future<void> scrollOutlineToEnd() async {
  final panel = boxOfTypeNamed('TimelinePanelFrb');
  if (panel == null) return;
  await scrollAt(
    Offset(panel.left + 120, panel.top + panel.height * 0.6),
    200,
    times: 30,
  );
}

/// A panel crop that keeps the tab strip above it and nothing outside its own
/// right-hand edge — [Rect.inflate] pushed the Audio panel's crop past the
/// window and lost a column of controls to the clamp.
Rect? panelCrop(Type type) {
  final box = boxOfType(type);
  return box == null
      ? null
      : Rect.fromLTRB(box.left - paneCardInset, box.top - dockTabInset,
          box.right + paneCardInset, box.bottom + paneCardInset);
}

/// Wait until the mix is actually making a sound, so a playback shot is
/// photographed with the meters reading rather than while the decoder is still
/// filling. Says what it saw either way — a machine with no output device
/// never gets there, and that is worth reading in the log rather than guessing
/// at from a dark meter.
Future<void> waitForSound({double seconds = 30}) async {
  for (var i = 0; i * 0.5 < seconds; i++) {
    final clock = audioClock();
    final meters = audioMeters();
    var loudest = 0.0;
    for (final strip in meters) {
      loudest = loudest < strip.peakLeft ? strip.peakLeft : loudest;
      loudest = loudest < strip.peakRight ? strip.peakRight : loudest;
    }
    if (i % 6 == 0 || loudest > 0.001) {
      say('clock ${clock.seconds.toStringAsFixed(2)}s '
          'playing=${clock.playing} loaded=${clock.loaded} '
          'strips=${meters.length} loudest=${loudest.toStringAsFixed(4)}');
    }
    if (loudest > 0.001) return;
    await pause(0.5);
  }
  say('the mix never reached the meters');
}

Future<void> main() async {
  final (state, ui) = await bootLumit();
  final path = Platform.environment['LUMIT_SHOTS_PROJECT'];
  if (path == null || !File(path).existsSync()) {
    say('set LUMIT_SHOTS_PROJECT to a .lum with its media beside it');
    exit(1);
  }

  // What the machine can play through: an empty list is a machine with no
  // output, which is the one condition under which a lit meter is impossible
  // rather than merely late (api/audio.rs: no device, no mix, no readings).
  final devices = listAudioDevices();
  say('audio devices: active="${devices.active}" fellBack=${devices.fellBack} '
      '${devices.devices.map((d) => d.name).join(" | ")}');

  ui.workspace.applyWorkspacePreset(WorkspacePreset.audio);
  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));
  await pause(2);
  await sizeWindow(2400, 1340);
  await pause(2);

  say('opening $path');
  await state.openProject(path);
  for (var i = 0; i < 120 && state.comps().isEmpty; i++) {
    await pause(0.5);
  }
  final comps = state.comps();
  say('${comps.length} comps');
  if (comps.isEmpty) {
    say('the project opened with no compositions');
    exit(1);
  }
  final wanted =
      (Platform.environment['LUMIT_SHOTS_COMP'] ?? 'clips').toLowerCase();
  var target = comps.first;
  for (final candidate in comps) {
    if (candidate.$2.toLowerCase() == wanted) {
      target = candidate;
      break;
    }
  }
  say('fronting ${target.$2}');
  ui.setSelectedComp(target.$1);
  await pause(6);
  // A restored session can carry its own arrangement; the preset is what this
  // sweep is photographing, so it is set again once the document is up.
  ui.workspace.applyWorkspacePreset(WorkspacePreset.audio);
  await pause(3);

  // Which rows can make a sound — the question the Mixer asks, asked here so
  // the sweep knows which layer's lane to open.
  final sounding = <BridgeLayerEntry>[];
  for (final entry in ui.model.heldLayers) {
    if (await entry.layer.hasAudio()) sounding.add(entry);
  }
  say('sounding rows: ${sounding.map((e) => e.info.name).join(", ")}');

  // The lane: the last sounding row is the music in most edits, and it is the
  // one worth a waveform. Selected first, so the Audio panel's Selected layer
  // block has something to draw.
  final BridgeLayerEntry? lane = sounding.isEmpty ? null : sounding.last;
  if (lane != null) {
    ui.setSelection([lane.layer]);
    await pause(1.5);
    final id = lane.layer.internallayerId.toString();
    // Each twirl adds rows under the one just opened, so the list is taken
    // back to its end between taps rather than once at the start.
    await scrollOutlineToEnd();
    await tapKey('tl-twirl-$id');
    await scrollOutlineToEnd();
    await tapKey('tl-twirl-$id/audio');
    await scrollOutlineToEnd();
    await tapKey('tl-twirl-$id/audio/waveform');
    await scrollOutlineToEnd();
    await pause(4);
  }

  // The beat grid, so the ruler wears its band (K-698). The source is pinned
  // to the music rather than left on the whole comp — a mixdown of sixty
  // mostly-silent picture layers is a poor thing to look for a tempo in.
  await tapKey('beats-source', settle: 1);
  final list = openPopup(margin: 0);
  say('source list at $list');
  if (list != null) {
    // The music is the last of the sounding rows, so it is the last row of
    // the list; the first is *This comp*.
    await tapAt(Offset(list.left + 40, list.bottom - 14), settle: 1);
  }
  await tapKey('beats-generate', settle: 1);
  await pause(60);
  say('grid after the button: ${target.$1.getBeatGrid()}');

  // The same call the button makes, with its answer read out — a run that
  // places nothing is a legitimate answer (quiet or arrhythmic audio) and the
  // band is then right to stay away, but the log should say which happened.
  if (lane != null) {
    try {
      final result = await target.$1.detectBeats(
        options: BridgeBeatOptions(
          sourceLayer: lane.layer.internallayerId.toString(),
          sensitivityPercent: 50,
          workAreaOnly: false,
          minSpacingMs: 120,
          bpmOverride: 0,
          phaseMs: 0,
        ),
      );
      say('detect on the music: placed=${result.placed} bpm=${result.bpm}');
    } catch (e) {
      say('detect on the music FAILED: $e');
    }
    ui.model.refresh();
    say('grid now: ${target.$1.getBeatGrid()}');
    await pause(3);
  }

  // The playhead somewhere into the comp rather than parked at nought, and
  // time for the picture to arrive — sixty-four layers of 4K take a while to
  // become one frame.
  // A twelfth in rather than a third: this edit's pictures are stacked in its
  // first fifteen seconds, and a playhead parked past them photographs an
  // empty Viewer that says nothing about the desk.
  final duration = target.$1.durationFrames();
  ui.scrubTo((duration * 0.12).round());
  await pause(45);

  // The Mixer, fronted: picking a layer fronts Effect controls over it
  // (ui_state item 6.28), which is exactly what a real session does too.
  ui.frontPanel(Panel.mixer);
  await pause(3);

  await captureUi('audio-workspace.png');
  await captureUi('audio-mixer.png', crop: panelCrop(MixerPanelFrb));
  await captureUi('audio-panel.png', crop: panelCrop(AudioPanelFrb));
  await captureUi('audio-timeline.png', crop: panelCrop(TimelinePanelFrb));

  // The same lane as a spectrogram — one press of the chip (K-699), since a
  // fresh lane starts on the multiwave stack and the cycle is
  // wave → stack → spectral.
  if (lane != null) {
    final id = lane.layer.internallayerId.toString();
    for (var i = 0; i < 3 && laneModes.of(id) != LaneMode.spectral; i++) {
      await tapKey('tl-lane-mode-$id', settle: 1);
    }
    say('lane mode now ${laneModes.of(id)}');
    await pause(8);
    await captureUi('audio-timeline-spectrogram.png',
        crop: panelCrop(TimelinePanelFrb));
    await captureUi('audio-workspace-spectrogram.png');
  }

  // Duck under… (K-697), in the smallest comp whose *top* row is the music:
  // the template's menu lists the other rows in comp order, so the music is
  // then the row a first-row press picks.
  (CompositionReference, String)? duck;
  for (final candidate in comps) {
    final layers = candidate.$1.getLayers();
    if (layers.length < 3 || layers.length > 12) continue;
    if (await layers.first.hasAudio()) {
      duck = candidate;
      break;
    }
  }
  if (duck != null) {
    say('ducking in ${duck.$2}');
    ui.setSelectedComp(duck.$1);
    await pause(5);
    ui.workspace.applyWorkspacePreset(WorkspacePreset.audio);
    await pause(2);
    final rows = ui.model.heldLayers;
    if (rows.length > 2) {
      ui.setSelection([rows[2].layer]);
      say('ducking ${rows[2].info.name} under ${rows.first.info.name}');
      await pause(2);
      ui.frontPanel(Panel.audio);
      await pause(1.5);
      await tapKey('audio-duck', settle: 1.5);
      final menu = openPopup(margin: 0);
      say('duck menu at $menu');
      if (menu != null) {
        await tapAt(Offset(menu.left + 40, menu.top + 12), settle: 2);
      }
      await pause(3);
      final graph = rows[2].layer.getGraph();
      say('graph now ${graph.nodes.length} nodes, '
          '${graph.wiring.edges.length} wires');
      ui.workspace.applyWorkspacePreset(WorkspacePreset.nodes);
      await pause(5);
      await captureUi('audio-graph-duck.png', crop: panelCrop(GraphPanelFrb));
      await captureUi('audio-graph-duck-window.png');
    }
  }

  // Playing, last: the meters light, the peak holds rest above them, the clip
  // lamp says whether the ceiling was reached, and the playhead runs.
  final playIn = duck ?? target;
  say('playing ${playIn.$2}');
  ui.setSelectedComp(playIn.$1);
  await pause(4);
  ui.workspace.applyWorkspacePreset(WorkspacePreset.audio);
  await pause(2);
  ui.frontPanel(Panel.mixer);
  await pause(2);
  ui.scrubTo((playIn.$1.durationFrames() * 0.2).round());
  await pause(4);
  ui.play();
  await waitForSound(seconds: 75);
  await captureUi('audio-playback.png');
  await captureUi('audio-playback-mixer.png', crop: panelCrop(MixerPanelFrb));
  await captureUi('audio-playback-panel.png', crop: panelCrop(AudioPanelFrb));
  await pause(1.5);
  await captureUi('audio-playback-2.png');
  ui.stopPlayback();
  await pause(2);

  // And once more on the long comp, which is the one the desk is really for —
  // at quarter preview, the way anybody plays a comp this heavy back.
  if (duck != null) {
    ui.setSelectedComp(target.$1);
    await pause(6);
    ui.workspace.applyWorkspacePreset(WorkspacePreset.audio);
    await pause(2);
    ui.frontPanel(Panel.mixer);
    ui.setPreviewResolution(PreviewResolution.quarter);
    ui.scrubTo((target.$1.durationFrames() * 0.3).round());
    await pause(10);
    ui.play();
    await waitForSound(seconds: 75);
    await captureUi('audio-playback-long.png');
    await captureUi('audio-playback-long-mixer.png',
        crop: panelCrop(MixerPanelFrb));
    await captureUi('audio-playback-long-panel.png',
        crop: panelCrop(AudioPanelFrb));
    await pause(1.5);
    await captureUi('audio-playback-long-2.png');
    ui.stopPlayback();
    await pause(2);
  }

  say('done');
  exit(0);
}
