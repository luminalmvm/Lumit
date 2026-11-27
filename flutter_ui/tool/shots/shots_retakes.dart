
// `_context` is not held across the gaps — it is re-read from the live tree at
// every use, which is what the lint is there to make you do.
// ignore_for_file: use_build_context_synchronously

import 'dart:async';
import 'dart:typed_data';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/shell/export_dialog_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/state/text_documents.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'shots_common.dart';

/// A keyframe at [frame] of a 25 fps composition.
///
/// [inSpeed]/[outSpeed] are the tangent's slope in value units a second, and
/// [influence] how far along the gap the handle reaches as a *fraction* of it —
/// the graph maths clamp it to 1, so a percentage here becomes a staircase.
BridgeKeyframe _key(int frame, double value,
        {double inSpeed = 0, double outSpeed = 0, double influence = 0.33}) =>
    BridgeKeyframe(
      time: BridgeRational(num: frame, den: 25),
      value: value,
      interpIn: BridgeSideInterp.bezier(
          BridgeBezierSide(speed: inSpeed, influence: influence)),
      interpOut: BridgeSideInterp.bezier(
          BridgeBezierSide(speed: outSpeed, influence: influence)),
    );

final _settings = BridgeCompSettings(
  name: 'Opening titles',
  width: 1920,
  height: 1080,
  fpsNum: 25,
  fpsDen: 1,
  duration: BridgeRational(num: 10, den: 1),
  background: F32Array4(Float32List.fromList([0, 0, 0, 1])),
  shutterAngle: 180,
  motionBlurSamples: 16,
  motionBlurLimit: 128,
);

late LumitState state;
late LumitUiState ui;

/// A context out of the live tree, for the dialogues that take one. The Project
/// panel's Import button sits under the same Overlay the menu bar's own
/// callbacks reach, which is what `showLumitModal` needs.
BuildContext get _context => elementByKey('project-import')!;

Future<void> main() async {
  final (s, u) = await bootLumit();
  state = s;
  ui = u;

  await _stageTitles();
  runApp(shotRoot(LumitAppNew(state, ui)));
  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(7);

  await _workspaceAndExport();
  await _blendKeysAndWaveform();
  await _speedRamp();

  exit(0);
}

Future<void> _stageTitles() async {
  final project = state.project!;
  final comp =
      project.newComposition(name: 'Opening titles', settings: _settings);

  for (final file in ['Music.wav', 'Gameplay.mp4', 'Title card.mp4']) {
    comp.addFootageLayer(
      footage: project.importFootage(path: '$fixtures/$file'),
      asSequence: false,
    );
  }
  project.importFootage(path: '$fixtures/Logo.png');

  final title = comp.addTextLayer();
  title.setText(
    document: const BridgeTextDocument(style: plainTextStyle, paragraph: plainParagraphStyle, animators: [], pathOffset: BridgeScalar.static_(0), 
      text: 'Northern lights',
      size: 140,
      fill: BridgeColourRgba(r: 1, g: 1, b: 1, a: 1),
    ),
  );
  title.setTransforms(props: const [
    BridgeTransformProp.positionX,
    BridgeTransformProp.positionY,
  ], values: const [
    BridgeScalar.static_(490),
    BridgeScalar.static_(840),
  ]);

  final layers = comp.getLayers();
  for (final (index, name)
      in ['Title', 'Title card', 'Gameplay', 'Music'].indexed) {
    layers[index].rename(name: name);
  }
  layers[1].setTransform(
    prop: BridgeTransformProp.opacity,
    value: const BridgeScalar.static_(55),
  );

  ui.setSelectedComp(comp);
  ui.playheadFrame.value = 48;
}

Future<void> _workspaceAndExport() async {
  // ---- Shot: the whole workspace, without the Debug View tab --------------
  // The Window menu's tick list is the user's route to this, and its callback
  // is this call (`shell/menu_bar_frb.dart`). The panel is dropped from the
  // tree and the group simplifies, exactly as closing its tab does.
  setPanelVisible(ui.split, Panel.debug, false);
  ui.workspace.touch();
  await pause(2.5);
  // ignore: avoid_print
  print('DEBUG VISIBLE ${panelVisible(ui.split, Panel.debug)}');
  await captureUi('workspace.png');

  // ---- Shot: the export settings ------------------------------------------
  // Opened through the dialogue's own picker seam — the same call the Choose…
  // button makes — so the destination row reads as a file somebody has chosen.
  final comp = ui.selectedComp!;
  unawaited(showExportDialogFrb(
    context: _context,
    comp: comp,
    picker: () async => 'D:/Projects/Northern lights/Opening titles.mp4',
  ));
  await pause(2);
  await tapKey('export-choose', settle: 1.5);
  await captureUi('export.png', scale: 2, crop: _window());
  await tapKey('export-close', settle: 1.5);
}

Rect _window() {
  final root = shotRootKey.currentContext!.findRenderObject()! as RenderBox;
  final screen = Offset.zero & root.size;
  final box = boxOfType(FloatSurface);
  if (box == null) return screen;
  return box.inflate(56).intersect(screen);
}

Future<void> _blendKeysAndWaveform() async {
  state.newProject();
  await pause(1.5);
  final project = state.project!;
  final comp =
      project.newComposition(name: 'Opening titles', settings: _settings);

  final logoItem = project.importFootage(path: '$fixtures/Logo.png');
  comp.addFootageLayer(
    footage: project.importFootage(path: '$fixtures/Music.wav'),
    asSequence: false,
  );
  comp.addSolidLayer();
  for (final file in ['Gameplay.mp4', 'Title card.mp4']) {
    comp.addFootageLayer(
      footage: project.importFootage(path: '$fixtures/$file'),
      asSequence: false,
    );
  }
  comp.addFootageLayer(footage: logoItem, asSequence: false);
  final title = comp.addTextLayer();
  title.setText(
    document: const BridgeTextDocument(style: plainTextStyle, paragraph: plainParagraphStyle, animators: [], pathOffset: BridgeScalar.static_(0), 
      text: 'Northern lights',
      size: 140,
      fill: BridgeColourRgba(r: 1, g: 1, b: 1, a: 1),
    ),
  );
  comp.addAdjustmentLayer();

  final layers = comp.getLayers();
  for (final (index, name) in [
    'Grade',
    'Title',
    'Logo',
    'Title card',
    'Gameplay',
    'Background',
    'Music',
  ].indexed) {
    layers[index].rename(name: name);
  }
  final card = layers[3];
  final music = layers[6];

  title.setTransforms(props: const [
    BridgeTransformProp.positionX,
    BridgeTransformProp.positionY,
  ], values: [
    BridgeScalar.keyframed(
        [_key(0, 470), _key(50, 490), _key(200, 490), _key(240, 470)]),
    BridgeScalar.keyframed(
        [_key(0, 900), _key(50, 840), _key(200, 840), _key(240, 900)]),
  ]);
  title.setTransform(
    prop: BridgeTransformProp.opacity,
    value: BridgeScalar.keyframed(
        [_key(0, 0), _key(25, 100), _key(215, 100), _key(245, 0)]),
  );
  title.setTransforms(props: const [
    BridgeTransformProp.scaleX,
    BridgeTransformProp.scaleY,
  ], values: [
    BridgeScalar.keyframed([_key(0, 104), _key(50, 100), _key(240, 100)]),
    BridgeScalar.keyframed([_key(0, 104), _key(50, 100), _key(240, 100)]),
  ]);

  layers[2].setSpan(
    span: BridgeSpan(
      inPoint: comp.timeOfFrame(frame: 62),
      outPoint: comp.timeOfFrame(frame: 162),
      startOffset: comp.timeOfFrame(frame: 0),
    ),
  );

  card.setTransform(
      prop: BridgeTransformProp.opacity, value: const BridgeScalar.static_(55));
  card.setBlend(index: 8);
  card.setSwitch(switch_: BridgeLayerSwitch.motionBlur, on_: true);
  music.setSwitch(switch_: BridgeLayerSwitch.locked, on_: true);

  ui.setSelectedComp(comp);
  ui.playheadFrame.value = 48;
  await pause(5);

  final titleId = title.internallayerId.toString();
  final cardId = card.internallayerId.toString();
  final musicId = music.internallayerId.toString();

  // ---- Shot: the blend-mode dropdown --------------------------------------
  // The list is taller than the room under the row, so it opens upwards over
  // the Viewer — which is where the crop has to reach to show it.
  await tapKey('tl-blend-$cardId', settle: 1.6);
  await captureUi(
    'blend-modes.png',
    scale: 2,
    crop: _blendCrop(cardId),
  );
  await tapAt(const Offset(240, 560), settle: 1.2);

  // ---- Shot: keyframes on a property lane ---------------------------------
  // Reaches the outline's left edge, so each row of diamonds is beside the
  // name of the property it belongs to.
  await _timelineShare(0.45);
  await tapKey('tl-twirl-$titleId');
  await tapKey('tl-twirl-${transformPath(titleId)}');
  await pause(1.5);
  final ruler = boxOf('tl-ruler')!;
  final position = boxOf('kf-stopwatch-tl-tf-positionX')!;
  await captureUi(
    'keyframes-lane.png',
    scale: 2,
    crop: Rect.fromLTRB(
        2, position.top - 70, ruler.right + 4, position.bottom + 92),
  );
  await tapKey('tl-twirl-$titleId');

  // ---- Shot: a footage layer's waveform in its lane -----------------------
  // Stood on the floor of its row rather than centred about silence: the
  // setting is Settings ▸ Interface ▸ Editing, and it is what
  // puts a 22-pixel row's whole height under signal.
  ui.workspace.interface.waveformsFromBottom = true;
  ui.workspace.settingsChanged();
  await tapKey('tl-twirl-$musicId');
  await tapKey('tl-twirl-${audioPath(musicId)}');
  await tapKey('tl-twirl-${waveformPath(musicId)}');
  // The peaks are fetched from the engine and arrive a moment later.
  await pause(6);
  final wave = boxOf('tl-wave-$musicId')!;
  await captureUi(
    'waveform.png',
    scale: 3,
    crop: Rect.fromLTRB(2, boxOf('tl-rowbody-$musicId')!.top - 2,
        boxOf('tl-ruler')!.right + 4, wave.bottom + 4),
  );
  await tapKey('tl-twirl-$musicId');
}

Future<void> _speedRamp() async {
  state.newProject();
  await pause(1.5);
  final project = state.project!;
  final comp = project.newComposition(
    name: 'Rough cut',
    settings: BridgeCompSettings(
      name: 'Rough cut',
      width: 1920,
      height: 1080,
      fpsNum: 25,
      fpsDen: 1,
      duration: BridgeRational(num: 10, den: 1),
      background: F32Array4(Float32List.fromList([0, 0, 0, 1])),
      shutterAngle: 180,
      motionBlurSamples: 16,
      motionBlurLimit: 128,
    ),
  );

  final card = project.importFootage(path: '$fixtures/Title card.mp4');
  comp.addFootageLayer(
      footage: project.importFootage(path: '$fixtures/Music.wav'),
      asSequence: false);
  comp.addFootageLayer(footage: card, asSequence: false);
  comp.addFootageLayer(
      footage: project.importFootage(path: '$fixtures/Gameplay.mp4'),
      asSequence: true);
  final title = comp.addTextLayer();
  title.setText(
    document: const BridgeTextDocument(style: plainTextStyle, paragraph: plainParagraphStyle, animators: [], pathOffset: BridgeScalar.static_(0), 
      text: 'Chapter two',
      size: 120,
      fill: BridgeColourRgba(r: 1, g: 1, b: 1, a: 1),
    ),
  );

  final layers = comp.getLayers();
  for (final (index, name)
      in ['Chapter title', 'Cut', 'Slow reveal', 'Music'].indexed) {
    layers[index].rename(name: name);
  }
  final slow = layers[2];

  slow.setSpan(
    span: BridgeSpan(
      inPoint: comp.timeOfFrame(frame: 0),
      outPoint: comp.timeOfFrame(frame: 250),
      startOffset: comp.timeOfFrame(frame: 0),
    ),
  );
  // ignore: avoid_print
  print('RETIME ON ${slow.toggleRetimeProperty()}');
  // The last key sits at the **end of the composition**, not ten frames short
  // of it. At frame 240 the map ran out and the last ten frames held their
  // value, which the speed lens drew — correctly — as a cliff to zero: the
  // source really does stop there. Running the map to 250 is the fix a person
  // would make, and the curve then reads as one ramp from edge to edge.
  slow.setRetimeProperty(
    value: BridgeScalar.keyframed([
      _key(0, 0, outSpeed: 0.15, influence: 0.3),
      _key(250, 4.8, inSpeed: 0.85, influence: 0.3),
    ]),
  );

  ui.setSelectedComp(comp);
  ui.playheadFrame.value = 60;
  await pause(5);

  await _timelineShare(0.38);
  await tapKey('tl-twirl-${slow.internallayerId}');
  await pause(1.5);
  ui.requestSelectProperty(retimePath(slow.internallayerId.toString()));
  await pause(1);
  await tapKey('tl-graph', settle: 2);
  await tapKey('graph-lens-speed', settle: 2.5);
  final ruler = boxOf('tl-ruler')!;
  await captureUi(
    'speed-ramp.png',
    scale: 2,
    crop: Rect.fromLTRB(
        2,
        ruler.top - 28 - paneCardInset,
        ruler.right + 4 + paneCardInset,
        boxOf('tl-zoom-slider')!.bottom + 8 + paneCardInset),
  );
}

/// The open blend-mode list, with the outline it belongs to beside it.
///
/// Sideways it is the band the caption is about: the outline, from the window's
/// edge to a little past the seam. Up and down it is measured off the list
/// itself — the list is as tall as its rows, and Round's rows are taller than
/// Sharp's, so a band of fixed numbers cut the last modes off.
Rect _blendCrop(String cardId) {
  final list =
      openPopup()!.expandToInclude(boxOf('tl-blend-$cardId')!.inflate(12));
  return Rect.fromLTRB(2, list.top, boxOf('tl-ruler')!.left + 40, list.bottom);
}

Future<void> _timelineShare(double share) async {
  ui.workspace.dock.shares[0] = 1 - share;
  ui.workspace.dock.shares[1] = share;
  ui.workspace.touch();
  await pause(2);
}
