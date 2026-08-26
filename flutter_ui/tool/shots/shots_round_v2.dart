// Round v2 review shots (K-394, docs/15-DESIGN.md §12.1).
//
// Not manual shots: these are review artefacts, so they go to
// `C:/tmp/lumit-shots/round-v2` and never near `web-docs`. The sweep refuses
// to run without `LUMIT_SHOTS_OUT` set, because the default destination is the
// site's asset folder and a shape review has no business writing there.
//
// The scene is sweep 1's — the same project `workspace.png` is staged from —
// with one thing changed: the workspace is set to Round before the app is
// built, which is what Settings ▸ Appearance ▸ Shape does. The Sequence layer
// the timeline shot needs is added *after* the workspace shot, so that frame
// stays the manual's scene exactly.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1
//   $env:LUMIT_SHOTS_OUT='C:/tmp/lumit-shots/round-v2'
//   flutter run -d windows -t tool/shots/shots_round_v2.dart

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'shots_common.dart';

Future<void> main() async {
  final (state, ui) = await bootLumit();
  if (Platform.environment['LUMIT_SHOTS_OUT'] == null) {
    // ignore: avoid_print
    print('SKIPPED: set LUMIT_SHOTS_OUT — these are review artefacts, and the '
        'default destination is the manual.');
    exit(0);
  }
  Directory(shotsOut).createSync(recursive: true);
  final project = state.project!;

  final comp = project.newComposition(
    name: 'Opening titles',
    settings: BridgeCompSettings(
      name: 'Opening titles',
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

  for (final file in ['Music.wav', 'Gameplay.mp4', 'Title card.mp4']) {
    comp.addFootageLayer(
      footage: project.importFootage(path: '$fixtures/$file'),
      asSequence: false,
    );
  }
  project.importFootage(path: '$fixtures/Logo.png');

  final title = comp.addTextLayer();
  title.rename(name: 'Title');
  title.setText(
    document: const BridgeTextDocument(pathOffset: BridgeScalar.static_(0), 
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

  // The one difference from sweep 1: the shape the review is about. This is
  // the call Settings ▸ Appearance makes, against the sweep's throwaway store.
  ui.workspace.setShape(ThemeShape.round);
  // The layout preset the default layout already is, applied so the workspace
  // strip has an active member to show. Without it nothing in that strip is
  // current, and "which one is in force" cannot be judged from a picture of a
  // strip where none of them is.
  ui.workspace.applyWorkspacePreset(WorkspacePreset.edit);

  runApp(shotRoot(LumitAppNew(state, ui)));

  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(6);

  // ---- Shot: the whole workspace, in Round --------------------------------
  await captureUi('round-workspace.png');

  // ---- Shot: the Viewer bar as a strip of its own -------------------------
  // The one thing a whole-workspace frame is too small to judge: under Round
  // the bar is parted from the picture by the tile gap and sits below it, so
  // the crop takes the bottom of the picture, the gap and the strip.
  final barBox = boxOf('viewer-bar')!;
  // ignore: avoid_print
  print('BAR $barBox / STAGE ${boxOf('viewer-stage')}');
  await captureUi(
    'round-viewer-bar.png',
    scale: 3,
    crop: Rect.fromLTRB(barBox.left - 10, barBox.top - 40, barBox.right + 10,
        barBox.bottom + 12),
  );

  // ---- Shot: tabs, chips and the transport pill ---------------------------
  // Three things at opposite ends of the upper band: the dock tabs with their
  // header dots at the top left, and the Viewer's floating bar — dropdown
  // chips, then the transport gathered into one pill — along the bottom. The
  // band is squeezed first so the crop is a band of controls rather than a
  // picture with a control at each corner; the splitter is where anybody
  // would drag it to see more Timeline.
  ui.workspace.dock.shares[0] = 0.32;
  ui.workspace.dock.shares[1] = 0.68;
  ui.workspace.touch();
  await pause(3);
  final viewer = boxOfType(ViewerPanelFrb)!;
  final pill = boxOf('viewer-transport-pill');
  // ignore: avoid_print
  print('PILL $pill');
  await captureUi(
    'round-toolbar-tabs.png',
    scale: 2,
    crop:
        Rect.fromLTRB(2, viewer.top - 34, viewer.right + 8, viewer.bottom + 8),
  );

  // ---- Shot: capsule bars, and a Sequence's clips -------------------------
  // Added after the workspace shot so that frame is the manual's scene. A
  // Sequence layer cut into three is the only way to have clips on screen, and
  // double-clicking the row is how the program opens their view (K-248).
  comp.addFootageLayer(
    footage: project.importFootage(path: '$fixtures/Gameplay.mp4'),
    asSequence: true,
  );
  final cut = comp.getLayers().first;
  cut.rename(name: 'Cut');
  cut.cutClipAt(frame: 55);
  cut.cutClipAt(frame: 105);
  // ignore: avoid_print
  print('CLIPS ${cut.getClips().length}');
  ui.workspace.dock.shares[0] = 0.55;
  ui.workspace.dock.shares[1] = 0.45;
  ui.workspace.touch();
  await pause(3);
  await doubleTapKey('tl-name-${cut.internallayerId}', settle: 2.5);

  final ruler = boxOf('tl-ruler')!;
  await captureUi(
    'round-timeline.png',
    scale: 1.5,
    crop: Rect.fromLTRB(2, ruler.top - 60, ruler.right + 6,
        boxOf('tl-zoom-slider')!.bottom + 8),
  );

  exit(0);
}
