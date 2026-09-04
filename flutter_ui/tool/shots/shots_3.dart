// Manual screenshots, sweep 3: the graph editor, a speed ramp, and a Sequence
// layer.
//
// sequence-layer · graph-editor · speed-ramp
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1   # PowerShell; LUMIT_SHOTS=1 elsewhere
//   flutter run -d windows -t tool/shots/shots_3.dart
//
// Its own sweep because the project it wants is a different one: a rough cut
// with a Sequence layer in it and a shot that has been retimed, rather than
// the title sequence sweep 2 stages.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/panels/transform_rows_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'shots_common.dart';

/// A keyframe at [frame] of a 25 fps comp.
///
/// [inSpeed]/[outSpeed] are the tangent's slope in value units a second, and
/// [influence] how far along the gap the handle reaches, as a fraction of it —
/// the graph maths clamp it to 1, so a percentage here becomes a staircase. Both matter here: the
/// speed lens draws the *rate* of a curve, so a shape that only looks eased on
/// the value graph can be nonsense read as speed.
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

Future<void> main() async {
  final (state, ui) = await bootLumit();
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
    ),
  );

  final gameplay = project.importFootage(path: '$fixtures/Gameplay.mp4');
  final card = project.importFootage(path: '$fixtures/Title card.mp4');

  comp.addFootageLayer(
      footage: project.importFootage(path: '$fixtures/Music.wav'),
      asSequence: false);
  // The retimed shot: a plain footage layer whose Retime property is keyed, so
  // the source runs slowly at first and then catches up.
  comp.addFootageLayer(footage: card, asSequence: false);
  // The cut itself: one Sequence layer, razored into three clips.
  comp.addFootageLayer(footage: gameplay, asSequence: true);
  final title = comp.addTextLayer();
  title.setText(
    document: const BridgeTextDocument(animators: [], pathOffset: BridgeScalar.static_(0), 
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
  final chapter = layers[0];
  final cut = layers[1];
  final slow = layers[2];

  // ---- The Sequence layer: three clips, cut back to back ------------------
  // ignore: avoid_print
  print('KIND ${cut.getKind()}');
  // The source is six seconds long, so the row is 150 frames and the razor
  // has to land inside it.
  cut.cutClipAt(frame: 55);
  cut.cutClipAt(frame: 105);
  // ignore: avoid_print
  print('CLIPS ${cut.getClips().length}');

  // ---- The retimed layer: a ramp, not a constant --------------------------
  // Retime is off until it is switched on (Ctrl+Alt+T), which installs the
  // identity map; the keys below then bend it. Source time crawls for the
  // first four seconds and then runs, which is what a speed ramp is.
  // Stretched over the whole composition first: a retimed layer can be any
  // length, and a ramp is only a ramp if it has room to run.
  slow.setSpan(
    span: BridgeSpan(
      inPoint: comp.timeOfFrame(frame: 0),
      outPoint: comp.timeOfFrame(frame: 250),
      startOffset: comp.timeOfFrame(frame: 0),
    ),
  );
  // ignore: avoid_print
  print('RETIME ON ${slow.toggleRetimeProperty()}');
  // Source seconds against local ones: barely moving for the first four, then
  // running to the end of the five-second source.
  //
  // Two keys with different slopes rather than three with flat handles: the
  // speed lens reads the *rate* of this curve, and a key whose handles are
  // level is a key the source stops dead on — which draws as a spike, not a
  // ramp.
  slow.setRetimeProperty(
    value: BridgeScalar.keyframed([
      _key(0, 0, outSpeed: 0.15, influence: 0.3),
      _key(240, 4.6, inSpeed: 0.85, influence: 0.3),
    ]),
  );

  // ---- The title's own animation, for the value graph ---------------------
  chapter.setTransforms(props: const [
    BridgeTransformProp.positionX,
    BridgeTransformProp.positionY,
  ], values: [
    BridgeScalar.keyframed(
        [_key(0, 300), _key(80, 700), _key(160, 900), _key(240, 1250)]),
    BridgeScalar.keyframed(
        [_key(0, 620), _key(80, 540), _key(160, 520), _key(240, 440)]),
  ]);
  chapter.setTransform(
    prop: BridgeTransformProp.opacity,
    value: BridgeScalar.keyframed(
        [_key(0, 0), _key(40, 100), _key(200, 100), _key(245, 0)]),
  );

  ui.setSelectedComp(comp);
  ui.playheadFrame.value = 60;

  // The sweeps photograph the shell; the welcome screen has its own sweep.
  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));
  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(6);

  final chapterId = chapter.internallayerId.toString();
  final cutId = cut.internallayerId.toString();
  final slowId = slow.internallayerId.toString();

  Future<void> timelineShare(double share) async {
    ui.workspace.dock.shares[0] = 1 - share;
    ui.workspace.dock.shares[1] = share;
    ui.workspace.touch();
    await pause(2);
  }

  // ---- Shot: a Sequence layer, clips cut back-to-back ---------------------
  // Double-clicking the row is what opens a Sequence layer's view, so
  // the clips are shown the way the program shows them.
  await timelineShare(0.45);
  await doubleTapKey('tl-name-$cutId', settle: 2);
  // Ends under the last row: four layers do not fill a Timeline opened wide
  // enough for a sequence view, and the empty half is not the subject.
  await captureUi(
    'sequence-layer.png',
    scale: 2,
    crop: Rect.fromLTRB(2, timelinePanelBox().top, boxOf('tl-ruler')!.right + 4,
        boxOf('tl-rowbody-${layers[3].internallayerId}')!.bottom + 12),
  );

  // ---- Shot: the graph editor, showing the value graph --------------------
  // Close the sequence view again, open the title's transform, pick the three
  // animated rows, and switch the lanes for the graph.
  await doubleTapKey('tl-name-$cutId', settle: 1.5);
  await timelineShare(0.38);
  await tapKey('tl-twirl-$chapterId');
  await tapKey('tl-twirl-${transformPath(chapterId)}');
  await pause(1.5);
  final positionGroup = transformGroups(
          threeD: false,
          modes: const BridgeAxisModes(
            anchor: BridgeAxisMode.combined,
            position: BridgeAxisMode.combined,
            scale: BridgeAxisMode.linked,
          ))
      .firstWhere((g) => g.axes.first.prop.name.startsWith('position'));
  ui.requestSelectProperty(transformGroupPath(chapterId, positionGroup));
  await pause(0.8);
  await tapKey('tl-graph', settle: 2);
  await tapKey('graph-lens-value', settle: 2);
  await captureUi('graph-editor.png', scale: 2, crop: timelinePanelBox());

  // ---- Shot: a speed ramp, through the Speed lens -------------------------
  // The Retime row of the retimed layer, read as speed rather than as value:
  // the curve that crawls and then runs.
  await tapKey('tl-graph', settle: 1.5);
  await tapKey('tl-twirl-$chapterId');
  await tapKey('tl-twirl-$slowId');
  await pause(1.5);
  ui.requestSelectProperty(retimePath(slowId));
  await pause(1);
  await tapKey('tl-graph', settle: 2);
  await tapKey('graph-lens-speed', settle: 2);
  await captureUi('speed-ramp.png', scale: 2, crop: timelinePanelBox());

  exit(0);
}
