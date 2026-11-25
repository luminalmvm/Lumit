
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'dart:typed_data';

import 'package:lumit_flutter/src/rust/api/folder.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/state/text_documents.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';

import 'shots_common.dart';

BridgeCompSettings _settings(String name, int seconds) => BridgeCompSettings(
      name: name,
      width: 1920,
      height: 1080,
      fpsNum: 25,
      fpsDen: 1,
      duration: BridgeRational(num: seconds, den: 1),
      // The dialog's own defaults: black ground, the 180-degree shutter at
      // its 16 samples. Stated here because the constructor requires
      // every field, so the fixture cannot drift from what a new comp gets.
      background: F32Array4(Float32List.fromList([0, 0, 0, 1])),
      shutterAngle: 180,
      motionBlurSamples: 16,
      motionBlurLimit: 128,
    );

Future<void> main() async {
  final (state, ui) = await bootLumit();
  final project = state.project!;

  final comp = project.newComposition(
    name: 'Opening titles',
    settings: _settings('Opening titles', 10),
  );
  final titleCard = project.newComposition(
      name: 'Title card', settings: _settings('Title card', 3));
  final lowerThird = project.newComposition(
      name: 'Lower third', settings: _settings('Lower third', 5));

  final folders = <String, FolderReference>{};
  for (final (name, label) in [('Footage', 1), ('Audio', 6)]) {
    final folder = project.newFolder(name: name);
    ItemReference.folder(folder).setLabel(label: label);
    folders[name] = folder;
  }

  for (final (file, label, folder) in [
    ('Music.wav', 6, 'Audio'),
    ('Gameplay.mp4', 1, 'Footage'),
    ('Title card.mp4', 1, 'Footage'),
  ]) {
    final footage = project.importFootage(path: '$fixtures/$file');
    comp.addFootageLayer(footage: footage, asSequence: false);
    final item = ItemReference.footage(footage);
    item.setLabel(label: label);
    item.moveToFolder(folder: folders[folder]!.internalid);
  }
  project.importFootage(path: '$fixtures/Logo.png');

  final title = comp.addTextLayer();
  title.rename(name: 'Title');
  title.setText(
    document: const BridgeTextDocument(style: plainTextStyle, paragraph: plainParagraphStyle, animators: [], pathOffset: BridgeScalar.static_(0), 
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

  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));

  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(6);

  await captureUi('workspace.png');

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
      crop: spanOfType(ViewerPanelFrb)!.inflate(paneCardInset + 2));

  exit(0);
}
