// Manual screenshots, sweep 1: workspace.png and viewer.png.
//
// Stages a plausible project through the real engine, shows the real editor,
// photographs it, and quits. See `shots_common.dart` for why a sweep is an app
// entrypoint rather than an integration test.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1   # PowerShell; LUMIT_SHOTS=1 elsewhere
//   flutter run -d windows -t tool/shots/shots_1.dart

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/folder.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';

import 'shots_common.dart';

/// One family of comps: 1920×1080 at 25 fps, differing only in how long they
/// run. The panel's Size and fps columns then read the same down the folder,
/// which is what the drawing shows.
BridgeCompSettings _settings(String name, int seconds) => BridgeCompSettings(
      name: name,
      width: 1920,
      height: 1080,
      fpsNum: 25,
      fpsDen: 1,
      duration: BridgeRational(num: seconds, den: 1),
    );

Future<void> main() async {
  final (state, ui) = await bootLumit();
  final project = state.project!;

  // A comp that looks like somebody's evening: ten seconds of it.
  final comp = project.newComposition(
    name: 'Opening titles',
    settings: _settings('Opening titles', 10),
  );
  // Two more, empty: they are here so the Compositions folder and the comp
  // tabs read like a project somebody has been working in rather than one
  // comp on its own. Nothing is photographed inside them.
  final titleCard = project.newComposition(
      name: 'Title card', settings: _settings('Title card', 3));
  final lowerThird = project.newComposition(
      name: 'Lower third', settings: _settings('Lower third', 5));

  // The folders a project of this shape would have, filled below: the drawing
  // shows the footage filed rather than loose at the root. Compositions is the
  // engine's own auto-folder, filled by `newComposition` above. Their labels
  // are what tints the folder icons (§12A.3a).
  final folders = <String, FolderReference>{};
  for (final (name, label) in [('Footage', 1), ('Audio', 6)]) {
    final folder = project.newFolder(name: name);
    ItemReference.folder(folder).setLabel(label: label);
    folders[name] = folder;
  }

  // Bottom of the stack upwards: each call puts its layer on top of the last.
  // The fixture files are named the way the layers should read, so nothing
  // needs renaming afterwards. The label is the item's colour tag — azure for
  // the video, indigo for the music.
  for (final (file, label, folder) in [
    ('Music.wav', 6, 'Audio'),
    ('Gameplay.mp4', 1, 'Footage'),
    ('Title card.mp4', 1, 'Footage'),
  ]) {
    final footage = project.importFootage(path: '$fixtures/$file');
    comp.addFootageLayer(footage: footage, asSequence: false);
    final item = ItemReference.footage(footage);
    item.setLabel(label: label);
    // Filed as the drawing shows it — the panel's own gesture, through the
    // same bridge call a drag onto a folder row makes.
    item.moveToFolder(folder: folders[folder]!.internalid);
  }
  // Imported but not placed — a project usually has one of those, and the
  // later sweeps need a still in the Project panel.
  project.importFootage(path: '$fixtures/Logo.png');

  // The top layer: the words the comp is named after, low in frame. A text
  // layer is what somebody would actually have on top of a title sequence,
  // and it gives the Viewer's gizmo something legible to sit around.
  final title = comp.addTextLayer();
  title.rename(name: 'Title');
  title.setText(
    document: const BridgeTextDocument(
      text: 'Northern lights',
      size: 140,
      fill: BridgeColourRgba(r: 1, g: 1, b: 1, a: 1),
    ),
  );
  // The anchor is where the starter document's centre was, so a longer line
  // runs off to the right of it unless Position is pulled back.
  title.setTransforms(props: const [
    BridgeTransformProp.positionX,
    BridgeTransformProp.positionY,
  ], values: const [
    BridgeScalar.static_(490),
    BridgeScalar.static_(840),
  ]);

  // Fading up over the first second, so the Timeline has keyframe diamonds to
  // show on a bar and the graph editor has a curve. Two keys, both at whole
  // frames, so the sweep is the same picture every time it runs.
  title.setTransform(
    prop: BridgeTransformProp.opacity,
    value: const BridgeScalar.keyframed([
      BridgeKeyframe(
        time: BridgeRational(num: 1, den: 1),
        value: 0,
        interpIn: BridgeSideInterp.linear(),
        interpOut: BridgeSideInterp.linear(),
      ),
      BridgeKeyframe(
        time: BridgeRational(num: 2, den: 1),
        value: 100,
        interpIn: BridgeSideInterp.linear(),
        interpOut: BridgeSideInterp.linear(),
      ),
    ]),
  );
  // One comp marker on the ruler, two seconds in — the cue somebody would
  // actually leave themselves, and what makes a marker flag visible in the
  // sweep. Markers are moments, not spans: a comp marker has no duration to
  // give it.
  addMarkerFrb(comp, frame: 50, label: 'Drop');

  final layers = comp.getLayers();
  // The panels show a layer's own name, which starts as the file's. Names
  // without extensions are what somebody an hour into the job would have.
  // The label is set beside the name rather than left to the layer kind's
  // default, so the outline chips and the lane bars are the same colours in
  // every run: mint for the words, azure for the picture, indigo for the
  // sound.
  for (final (index, (name, label)) in [
    ('Title', 4),
    ('Title card', 1),
    ('Gameplay', 1),
    ('Music', 6),
  ].indexed) {
    layers[index].rename(name: name);
    layers[index].setLabel(label: label);
  }
  // The title card half-dissolved over the footage, which is what the picture
  // would actually look like at this point in a cut.
  layers[1].setTransform(
    prop: BridgeTransformProp.opacity,
    value: const BridgeScalar.static_(55),
  );

  // Fronting a comp is what opens its tab, so three fronts leave three tabs in
  // that order — and the last one decides which comp is on screen, which is
  // still the one every shot in this sweep is of.
  for (final open in [comp, titleCard, lowerThird, comp]) {
    ui.setSelectedComp(open);
  }
  ui.playheadFrame.value = 48;

  runApp(shotRoot(LumitAppNew(state, ui)));

  // Size the window before anything is photographed: the shots want a real
  // working window, not the runner's 1280×720 default.
  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(6);

  // Shot 1 — the whole workspace: Viewer, Timeline and Project docked.
  await captureUi('workspace.png');

  // Shot 2 — the Viewer with a layer selected and its transform gizmo up.
  // Selection is what raises the gizmo; the Selection tool is armed by default
  // and so is the wireframes switch.
  ui.setSelection([title]);
  await pause(3);
  // Cropped to the Viewer and its bar: the caption is about the Viewer, and a
  // gizmo lost in a full-window shot is not "showing". Measured off the panel
  // rather than typed in, because Round insets the content by the pane card's
  // padding and parts the bar from the picture — a crop of fixed numbers is a
  // crop that only fits one of the two shapes.
  // The Viewer is a bare pane — it carries no tab strip — so the crop grows to
  // its pane card and no further.
  await captureUi('viewer.png',
      crop: boxOfType(ViewerPanelFrb)!.inflate(paneCardInset + 2));

  exit(0);
}
