// Manual screenshots, sweep: the Timeline's three modes, inspected.
//
// layers-shut · layers-open · layers-selected · keys-mode · graph-mode
//
// Staged the way an inspection wants it: two layers, one with a Glow and
// four opacity keyframes wearing every interpolation the marks can say —
// linear (diamond), bezier (hourglass), held (square), and a bezier-in /
// held-out split — plus position keys, so the open fold shows shaped marks
// on two lanes, the shut bar shows summary diamonds, and Keys and Graph
// modes show the same truth their own way. The twirl and the mode tabs are
// clicked, not reached into, like every sweep.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1
//   flutter run -d windows -t tool/shots/shots_modes.dart

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
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
    ),
  );
  ui.setSelectedComp(comp);

  // The inspected layer: a Glow, and opacity keys wearing every mark —
  // linear, bezier both sides, held both sides, and the split pair.
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
  // A second animated lane, so the open fold is more than one row.
  title.setTransform(
    prop: BridgeTransformProp.positionX,
    value: BridgeScalar.keyframed([
      _key(25, 400, _linear, _bezier),
      _key(100, 1500, _bezier, _linear),
    ]),
  );

  // A shut neighbour whose bar wears the summary diamonds.
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

  // Inflate for the tab strip, then clamp inside the photographed boundary
  // itself — the client area, not the window we asked for: a crop that pokes
  // even one pixel past the frame is an ffmpeg refusal and a silent
  // full-window shot.
  Rect panel() {
    final b = boxOfType(TimelinePanelFrb)!.inflate(dockTabInset);
    final frame = shotRootKey.currentContext!.size!;
    return Rect.fromLTRB(
        b.left.clamp(0, frame.width),
        b.top.clamp(0, frame.height),
        b.right.clamp(0, frame.width),
        b.bottom.clamp(0, frame.height));
  }

  // 1 — Layers mode, everything shut: summary diamonds on both bars.
  await captureUi('layers-shut.png', crop: panel());

  // 2 — the Title layer twirled open: two lanes of shaped marks.
  final id = title.internallayerId;
  await tapKey('tl-twirl-$id');
  await pause(1);
  // The lanes live under the groups: Transform for the shaped marks,
  // Effects for the Glow's rows.
  await tapKey('tl-twirl-$id/transform');
  await tapKey('tl-twirl-$id/effects');
  await pause(1);
  await captureUi('layers-open.png', crop: panel());

  // 3 — a key selected: the second opacity key (the hourglass), clicked.
  // The opacity lane's row id is the transform-group path the fold builds.
  await tapKey('tl-key-$id/transform/opacity#1', settle: 1);
  await captureUi('layers-selected.png', crop: panel());

  // 4 — Keys mode, the same comp as the dope sheet reads it.
  await tapKey('tl-view-keys');
  await pause(1);
  await captureUi('keys-mode.png', crop: panel());

  // 5 — Graph mode: the opacity curve with its mixed segments.
  await tapKey('tl-graph');
  await pause(1);
  await captureUi('graph-mode.png', crop: panel());

  exit(0);
}
