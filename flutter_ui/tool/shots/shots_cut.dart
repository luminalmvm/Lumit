// Manual screenshots, sweep: the Cut workspace on a staged cut.
//
// cut-workspace · cut-timeline · cut-dissolve-before · cut-dissolve-middle ·
// cut-dissolve-after · cut-source-view
//
// Staged rather than opened from a project: a main track cut from the source
// view with its linked sound under it, a title dissolved in over the first
// clip, a second picture track with one clip turned into a composition, and a
// music track. The fixtures are the ones every other sweep uses plus Talk.mp4,
// a short test picture with a tone, and Plate.mp4, eight seconds of one flat
// colour, both made with ffmpeg beside them.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1
//   flutter run -d windows -t tool/shots/shots_cut.dart

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/cut_timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/cut.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';

import 'shots_common.dart';

void say(Object? what) {
  // ignore: avoid_print
  print('CUT SWEEP: $what');
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
  ui.workspace.applyWorkspacePreset(WorkspacePreset.cut);
  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));
  await pause(2);
  await sizeWindow(2400, 1340);
  await pause(2);

  final project = state.project!;
  final defaults = BridgeCompSettings.defaults();
  final comp = project.newComposition(
    name: 'Episode 12',
    settings: BridgeCompSettings(
      name: 'Episode 12',
      width: defaults.width,
      height: defaults.height,
      fpsNum: defaults.fpsNum,
      fpsDen: defaults.fpsDen,
      duration: const BridgeRational(num: 30, den: 1),
      background: defaults.background,
      shutterAngle: defaults.shutterAngle,
      motionBlurSamples: defaults.motionBlurSamples,
    ),
  );
  final talk = project.importFootage(path: '$fixtures/Talk.mp4');
  final gameplay = project.importFootage(path: '$fixtures/Gameplay.mp4');
  final title = project.importFootage(path: '$fixtures/Title card.mp4');
  final music = project.importFootage(path: '$fixtures/Music.wav');
  ui.setSelectedComp(comp);
  ui.model.refresh();
  await pause(2);
  // Choosing Cut with a composition in front is what puts the source side in
  // the Viewer.
  ui.workspace.applyWorkspacePreset(WorkspacePreset.cut);
  await pause(2);

  final fps = ui.model.fps;
  int frames(double seconds) => (seconds * fps).round();
  BridgeRational seconds(int n) => BridgeRational(num: n, den: 1);
  void report(String what, BridgeCutResult result) {
    if (result != BridgeCutResult.done) say('$what: $result');
  }

  // The main track, cut from the source view: two stretches of Talk.
  ui.openFootageView(talk);
  for (var i = 0; i < 40 && ui.itemFacts(talk) == null; i++) {
    await pause(0.25);
  }
  final source = ui.sourceView;
  if (source == null || ui.itemFacts(talk) == null) {
    say('no source view, or Talk.mp4 would not probe');
    exit(1);
  }
  ui.seekFootageView(source, 15);
  ui.markSource(source, markIn: true);
  ui.seekFootageView(source, 195);
  ui.markSource(source, markIn: false);
  ui.playheadFrame.value = 0;
  ui.placeSource(source, insert: false);
  await pause(1);
  ui.placeSource(source, insert: false);
  await pause(1);
  ui.model.refresh();

  LayerReference layerOf(BridgeLayerKind kind, {bool last = false}) {
    final found = [
      for (final entry in ui.model.layers)
        if (entry.info.kind == kind) entry.layer,
    ];
    return last ? found.last : found.first;
  }

  final main = layerOf(BridgeLayerKind.sequence);
  // A title over the join of the two, dissolved in and out.
  report(
      'title',
      comp.cutPlace(
        target: main,
        footage: title,
        sourceIn: seconds(1),
        sourceOut: seconds(5),
        atFrame: frames(5),
        insert: true,
        linked: true,
      ));
  ui.model.refresh();
  var clips = main.getClips();
  say('main track: ${[for (final c in clips) '${c.startFrame}-${c.endFrame}']}');
  final titleClip = clips.firstWhere((c) => c.startFrame == frames(5));
  report(
      'dissolve in',
      comp.cutTransition(
          clip: titleClip.id, endEdge: false, frames: frames(1), linked: true));
  report(
      'dissolve out',
      comp.cutTransition(
          clip: titleClip.id, endEdge: true, frames: frames(1), linked: true));
  // The last clip fades out.
  clips = main.getClips();
  report(
      'fade out',
      comp.cutTransition(
          clip: clips.last.id, endEdge: true, frames: frames(1), linked: true));

  // A second picture track over it, one clip of which is a composition.
  report(
      'second track',
      comp.cutPlace(
        footage: gameplay,
        sourceIn: seconds(2),
        sourceOut: seconds(5),
        atFrame: frames(1.5),
        insert: false,
        linked: false,
      ));
  ui.model.refresh();
  final over = layerOf(BridgeLayerKind.sequence);
  report(
      'second clip',
      comp.cutPlace(
        target: over,
        footage: gameplay,
        sourceIn: seconds(6),
        sourceOut: seconds(9),
        atFrame: frames(11),
        insert: false,
        linked: false,
      ));
  final made = comp.cutClipToComposition(clip: over.getClips().last.id);
  say('clip to composition: ${made != null}');

  // Music under the whole of it.
  report(
      'music',
      comp.cutPlace(
        footage: music,
        sourceIn: seconds(0),
        sourceOut: seconds(16),
        atFrame: 0,
        insert: false,
        linked: false,
      ));
  ui.model.refresh();
  comp.audioPrepare();
  await pause(4);

  // The source side stands on a frame inside its marks.
  ui.seekFootageView(source, 90);
  // The playhead in the middle of the dissolve into the title.
  clips = main.getClips();
  final incoming = clips.firstWhere((c) => c.id == titleClip.id);
  final outgoing = clips.first;
  final from = incoming.startFrame;
  final to = outgoing.endFrame;
  say('dissolve runs $from to $to');
  ui.scrubTo((from + to) ~/ 2);
  await pause(5);

  await captureUi('cut-workspace.png');
  await captureUi('cut-timeline.png', crop: panelCrop(CutTimelinePanelFrb));
  // The Viewer draws bare, with no tab strip over it, so its own box is the
  // picture wanted.
  await captureUi('cut-source-view.png', crop: boxOfType(ViewerPanelFrb));

  // A dissolve into a flat plate, photographed before, in the middle and
  // after, so the mix can be read off the picture. Not for the manual.
  final plate = project.importFootage(path: '$fixtures/Plate.mp4');
  for (var i = 0; i < 40 && ui.itemFacts(plate) == null; i++) {
    await pause(0.25);
  }
  // Clear of the track above, and with the last clip's fade taken off so the
  // only thing mixing is the dissolve.
  report(
      'fade off',
      comp.cutTransition(
          clip: main.getClips().last.id,
          endEdge: true,
          frames: 0,
          linked: true));
  report(
      'plate',
      comp.cutPlace(
        target: main,
        footage: plate,
        sourceIn: seconds(2),
        sourceOut: seconds(5),
        atFrame: frames(15),
        insert: false,
        linked: false,
      ));
  ui.model.refresh();
  clips = main.getClips();
  final plateClip = clips.firstWhere((c) => c.startFrame == frames(15));
  report(
      'plate dissolve',
      comp.cutTransition(
          clip: plateClip.id, endEdge: false, frames: frames(1), linked: false));
  ui.model.refresh();
  clips = main.getClips();
  final into = clips.firstWhere((c) => c.id == plateClip.id);
  final before = clips.lastWhere((c) => c.startFrame < into.startFrame);
  say('plate dissolve runs ${into.startFrame} to ${before.endFrame}');
  for (final (name, frame) in [
    ('before', into.startFrame - 2),
    ('middle', (into.startFrame + before.endFrame) ~/ 2),
    ('after', before.endFrame + 2),
  ]) {
    ui.scrubTo(frame);
    WidgetsBinding.instance.scheduleFrame();
    await pause(4);
    say('playhead at ${ui.playheadFrame.value} for $name');
    await captureUi('cut-dissolve-$name.png', crop: boxOfType(ViewerPanelFrb));
  }

  say('done');
  exit(0);
}
