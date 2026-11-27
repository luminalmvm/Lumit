
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'shots_common.dart';

BridgeKeyframe _key(
        int frame, double value, BridgeSideInterp inn, BridgeSideInterp out) =>
    BridgeKeyframe(
      time: BridgeRational(num: frame, den: 25),
      value: value,
      interpIn: inn,
      interpOut: out,
    );

const _linear = BridgeSideInterp.linear();
const _hold = BridgeSideInterp.hold();
const _bezier =
    BridgeSideInterp.bezier(BridgeBezierSide(speed: 0, influence: 0.33));

Future<void> main() async {
  final (state, ui) = await bootLumit();
  final project = state.project!;

  final comp = project.newComposition(
    name: 'Opening titles',
    settings: BridgeCompSettings(
      name: 'Opening titles',
      width: 1920,
      height: 1080,
      fpsNum: 25,
      fpsDen: 1,
      duration: const BridgeRational(num: 10, den: 1),
      background: F32Array4(Float32List.fromList([0, 0, 0, 1])),
      shutterAngle: 180,
      motionBlurSamples: 16,
      motionBlurLimit: 128,
    ),
  );
  ui.setSelectedComp(comp);

  final title = comp.addSolidLayer();
  title.addEffect(name: 'glow');
  title.setTransform(
    prop: BridgeTransformProp.opacity,
    value: BridgeScalar.keyframed([
      _key(25, 0, _linear, _linear),
      _key(50, 100, _bezier, _bezier),
      _key(75, 60, _hold, _hold),
      _key(100, 90, _bezier, _hold),
    ]),
  );
  title.setTransform(
    prop: BridgeTransformProp.positionX,
    value: BridgeScalar.keyframed([
      _key(25, 400, _linear, _bezier),
      _key(100, 1500, _bezier, _linear),
    ]),
  );

  final gameplay = comp.addSolidLayer();
  gameplay.setTransform(
    prop: BridgeTransformProp.opacity,
    value: BridgeScalar.keyframed([
      _key(40, 100, _linear, _linear),
      _key(120, 40, _bezier, _bezier),
    ]),
  );

  ui.setSelection([title]);
  ui.playheadFrame.value = 60;

  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));

  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(5);

  // Over half the window to the Timeline. The rows are built as they are
  // needed, so a property below the fold is not merely out of shot — it is not
  // in the tree at all, and a sweep reaching for it finds nothing. Opacity is
  // the fifth transform row, and the default share does not reach it.
  ui.workspace.dock.shares[0] = 0.45;
  ui.workspace.dock.shares[1] = 0.55;
  ui.workspace.touch();
  await pause(2);

  await captureUi('layers-shut.png', crop: timelinePanelBox());

  final id = title.internallayerId;
  await tapKey('tl-twirl-$id');
  await pause(1);
  // The lanes live under the groups: Transform for the shaped marks,
  // Effects for the Glow's rows.
  await tapKey('tl-twirl-$id/transform');
  await tapKey('tl-twirl-$id/effects');
  await pause(1);
  await captureUi('layers-open.png', crop: timelinePanelBox());

  // 3 — a key selected: the second opacity key (the hourglass), clicked.
  // The opacity lane's row id is the transform-group path the fold builds.
  await tapKey('tl-key-$id/transform/opacity#1', settle: 1);
  await captureUi('layers-selected.png', crop: timelinePanelBox());

  // 4 — Graph mode: the opacity curve with its mixed segments. A key being
  // selected is not enough — Graph mode draws the curves of the *properties*
  // picked out in the outline, and with none picked it says so instead — so
  // the opacity row's name is clicked before the mode is switched.
  // A property row carries no key of its own — only its label's tap — so this
  // is the one place the sweep asks the panel directly instead of clicking.
  ui.requestSelectProperty('$id/transform/opacity');
  await pause(0.8);
  await tapKey('tl-graph');
  await pause(1);
  await captureUi('graph-mode.png', crop: timelinePanelBox());

  exit(0);
}
