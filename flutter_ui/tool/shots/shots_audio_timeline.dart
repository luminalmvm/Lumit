// Manual screenshots, sweep: the Audio timeline panel on a staged comp.
//
// audio-timeline-workspace · audio-timeline · audio-timeline-twirl ·
// audio-timeline-fade-menu · audio-timeline-fade-editor ·
// audio-timeline-clip-open · audio-timeline-spectral · edit-sound-mix
//
// Staged rather than opened from a project: two music rows, one of them a
// track of two overlapping clips with a fade at each end, and a video with
// sound that stands greyed until its audio is detached. The fixtures are the
// ones every other sweep uses (Music.wav under C:/tmp/lumit-shots) plus Talk.mp4,
// a short test picture with a tone, made with ffmpeg beside them.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1
//   flutter run -d windows -t tool/shots/shots_audio_timeline.dart

import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/audio_timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'shots_common.dart';

void say(Object? what) {
  // ignore: avoid_print
  print('AUDIO TIMELINE SWEEP: $what');
}

/// Right-click a point in window coordinates.
Future<void> rightTapAt(Offset at, {double settle = 1}) async {
  const id = 9001;
  GestureBinding.instance.handlePointerEvent(
      PointerDownEvent(pointer: id, position: at, buttons: kSecondaryButton));
  await pause(0.06);
  GestureBinding.instance
      .handlePointerEvent(PointerUpEvent(pointer: id, position: at));
  await pause(settle);
}

Rect? panelCrop(Type type) {
  final box = boxOfType(type);
  return box == null
      ? null
      : Rect.fromLTRB(box.left - paneCardInset, box.top - dockTabInset,
          box.right + paneCardInset, box.bottom + paneCardInset);
}

Future<void> main() async {
  final (state, ui) = await bootLumit();
  ui.workspace.applyWorkspacePreset(WorkspacePreset.audio);
  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));
  await pause(2);
  await sizeWindow(2400, 1340);
  await pause(2);

  final project = state.project!;
  final defaults = BridgeCompSettings.defaults();
  final comp = project.newComposition(
    name: 'Cut',
    settings: BridgeCompSettings(
      name: 'Cut',
      width: defaults.width,
      height: defaults.height,
      fpsNum: defaults.fpsNum,
      fpsDen: defaults.fpsDen,
      duration: const BridgeRational(num: 40, den: 1),
      background: defaults.background,
      shutterAngle: defaults.shutterAngle,
      motionBlurSamples: defaults.motionBlurSamples,
    ),
  );
  final music = project.importFootage(path: '$fixtures/Music.wav');
  final talk = project.importFootage(path: '$fixtures/Talk.mp4');
  // Top to bottom: the video with its sound, a music row, another music row
  // that becomes a track of two clips.
  comp.addFootageLayer(footage: music, asSequence: false);
  comp.addFootageLayer(footage: music, asSequence: false);
  comp.addFootageLayer(footage: talk, asSequence: false);
  ui.setSelectedComp(comp);
  ui.model.refresh();
  await pause(3);

  final layers = comp.getLayers();
  say('${layers.length} layers');
  // The bottom music row is the track of two clips: converted, its clip
  // given a fade out, and a second clip laid across that end so the overlap
  // is a crossfade with a fade in of its own at the far end.
  final track = layers.last;
  track.convertToSequenced();
  var clips = track.getClips();
  final first = clips.single;
  final fps = ui.model.fps;
  track.trimClip(
      clip: first.id,
      startFrame: first.startFrame,
      endFrame: first.startFrame + (fps * 8).round());
  clips = track.getClips();
  track.setClipFade(
    clip: clips.single.id,
    fadeIn:
        const BridgeClipFade(seconds: 1.5, shape: BridgeClipFadeShape.smooth()),
    fadeOut:
        const BridgeClipFade(seconds: 1.0, shape: BridgeClipFadeShape.fast()),
  );
  track.addClip(
      footage: music,
      atFrame: clips.single.endFrame - (fps * 2).round(),
      overlap: true);
  clips = track.getClips();
  final second = clips.last;
  track.trimClip(
      clip: second.id,
      startFrame: second.startFrame,
      endFrame: second.startFrame + (fps * 8).round());
  clips = track.getClips();
  track.setClipFade(
    clip: clips.last.id,
    fadeOut:
        const BridgeClipFade(seconds: 2.0, shape: BridgeClipFadeShape.slow()),
  );
  // The middle music row keeps its own single-file lane, with a volume ride.
  final plain = layers[1];
  plain.setVolumeDb(value: const BridgeScalar.static_(-6));
  ui.setSelection([plain]);
  ui.model.refresh();
  comp.audioPrepare();
  ui.scrubTo((fps * 5).round());
  await pause(6);
  ui.workspace.applyWorkspacePreset(WorkspacePreset.audio);
  await pause(4);

  await captureUi('audio-timeline-workspace.png');
  await captureUi('audio-timeline.png', crop: panelCrop(AudioTimelinePanelFrb));

  // The twirl on the plain music row: straight on to Volume.
  final plainId = plain.internallayerId.toString();
  await tapKey('atl-twirl-$plainId', settle: 1.5);
  await captureUi('audio-timeline-twirl.png',
      crop: panelCrop(AudioTimelinePanelFrb));
  await tapKey('atl-twirl-$plainId', settle: 1);

  // The fade menu on the first clip's fade in, then the custom editor.
  clips = track.getClips();
  final clipA = clips.first;
  final boxA = boxOf('atl-clip-${clipA.id}');
  if (boxA != null) {
    await rightTapAt(Offset(boxA.left + 8, boxA.bottom - 6), settle: 1.5);
    await captureUi('audio-timeline-fade-menu.png',
        crop: panelCrop(AudioTimelinePanelFrb));
    final custom = await tapKey('atl-fade-custom-${clipA.id}', settle: 2);
    say('custom row $custom');
    await captureUi('audio-timeline-fade-editor.png');
    closeLumitPopups();
    await pause(1);
  }

  // The overlap's own menu: the crossfade shapes.
  final boxB = boxOf('atl-clip-${clips.last.id}');
  if (boxA != null && boxB != null) {
    final x = (boxB.left + boxA.right) / 2;
    await rightTapAt(Offset(x, boxA.bottom - 6), settle: 1.5);
    await captureUi('audio-timeline-crossfade-menu.png',
        crop: panelCrop(AudioTimelinePanelFrb));
    // Custom on an overlap opens the editor with both curves and Keep level.
    final pair = await tapKey('atl-fade-custom-${clips.last.id}', settle: 2) ||
        await tapKey('atl-fade-custom-${clipA.id}', settle: 2);
    say('crossfade editor $pair');
    await captureUi('audio-timeline-crossfade-editor.png');
    closeLumitPopups();
    await pause(1);
  }

  // An effect on the first clip, and its twirl dropped down. A picture
  // effect stands in, as the menu's audio plugins are whatever the machine
  // has installed; the rows under the clip look the same either way.
  try {
    track.addClipEffect(clip: clipA.id, name: 'blur');
  } catch (e) {
    say('clip effect: $e');
  }
  ui.model.refresh();
  await pause(2);
  say('clip A effects: ${track.getClips().first.effects.length}');
  say('twirl tapped: ${await tapKey('atl-clip-twirl-${clipA.id}', settle: 2)}');
  await captureUi('audio-timeline-clip-open.png',
      crop: panelCrop(AudioTimelinePanelFrb));
  await captureUi('audio-timeline-clip-open-workspace.png');
  await tapKey('atl-clip-twirl-${clipA.id}', settle: 1);

  // The track in spectral mode.
  final trackId = track.internallayerId.toString();
  await tapKey('atl-lane-mode-$trackId', settle: 6);
  await captureUi('audio-timeline-spectral.png',
      crop: panelCrop(AudioTimelinePanelFrb));
  await tapKey('atl-lane-mode-$trackId', settle: 2);

  // Back to Edit: the layer Timeline folds the tracks behind the Sound mix row.
  ui.workspace.applyWorkspacePreset(WorkspacePreset.edit);
  await pause(5);
  await captureUi('edit-sound-mix.png', crop: panelCrop(TimelinePanelFrb));
  await captureUi('edit-sound-mix-workspace.png');
  // The row's own menu: open the workspace, or convert the mix to a precomp.
  say('mix menu ${await rightTapKey('tl-sound-mix-row', settle: 1.5)}');
  // Whole window: the row is at the foot, so the menu drops past the panel.
  await captureUi('edit-sound-mix-menu.png');
  closeLumitPopups();
  await pause(1);

  say('done');
  exit(0);
}
