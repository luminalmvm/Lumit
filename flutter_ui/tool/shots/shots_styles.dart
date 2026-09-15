// Style and colour scheme review shots (docs/15-DESIGN.md section 12).
//
// Not manual shots: these are review artefacts, so they go to
// `C:/tmp/lumit-shots/styles` and never near `web-docs`. The sweep refuses to
// run without `LUMIT_SHOTS_OUT` set, because the default destination is the
// site's asset folder and a style review has no business writing there.
//
// The scene is sweep 1's, staged once. The sweep then walks a table of style
// and scheme pairs, switching each one the way Settings, Appearance does, and
// photographs the whole window every time, so the pictures differ in nothing
// but the look under review.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1
//   $env:LUMIT_SHOTS_OUT='C:/tmp/lumit-shots/styles'
//   flutter run -d windows -t tool/shots/shots_styles.dart

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'shots_common.dart';

/// One picture: the file name and the look it shows.
class _Look {
  const _Look(
    this.name,
    this.shape,
    this.scheme, {
    this.room = LanternRoom.auto,
    // The style's own choice unless a look says otherwise, which is what a
    // fresh install shows.
    this.toolBar = ToolBarPosition.auto,
    this.bars = ViewerBars.auto,
  });

  final String name;
  final ThemeShape shape;
  final LumitColorScheme scheme;
  final LanternRoom room;
  final ToolBarPosition toolBar;
  final ViewerBars bars;
}

const _looks = [
  // Studio, the look as it was, under the stock scheme and four new pairs.
  _Look('studio-dark', ThemeShape.studio, LumitColorScheme.dark),
  _Look('studio-mallow-dark', ThemeShape.studio, LumitColorScheme.mallowDark),
  _Look('studio-glacier-light', ThemeShape.studio,
      LumitColorScheme.glacierLight),
  _Look('studio-hearth-dark', ThemeShape.studio, LumitColorScheme.hearthDark),
  _Look('studio-chalk-light', ThemeShape.studio, LumitColorScheme.chalkLight),
  // Desk, in its own two rooms first, then three schemes on the same shell.
  _Look('desk-grey-room', ThemeShape.desk, LumitColorScheme.greyRoom),
  _Look('desk-graphite', ThemeShape.desk, LumitColorScheme.graphite),
  _Look('desk-slate-dark', ThemeShape.desk, LumitColorScheme.slateDark),
  _Look('desk-vellum-light', ThemeShape.desk, LumitColorScheme.vellumLight),
  _Look('desk-chalk-dark', ThemeShape.desk, LumitColorScheme.chalkDark),
  _Look('desk-grey-room-rail-deck', ThemeShape.desk, LumitColorScheme.greyRoom,
      toolBar: ToolBarPosition.left, bars: ViewerBars.deck),
  // Lantern in the day room.
  _Look('lantern-dark-day', ThemeShape.lantern, LumitColorScheme.dark),
  _Look('lantern-neon-dark-day', ThemeShape.lantern,
      LumitColorScheme.neonDark),
  // A light scheme: the style's choice stands it in the night room.
  _Look('lantern-tavern-light', ThemeShape.lantern,
      LumitColorScheme.tavernLight),
  _Look('lantern-gilt-dark-day', ThemeShape.lantern,
      LumitColorScheme.giltDark),
  // Lantern in the night room.
  _Look('lantern-arcane-dark-night', ThemeShape.lantern,
      LumitColorScheme.arcaneDark,
      room: LanternRoom.night),
  _Look('lantern-nocturne-dark-night', ThemeShape.lantern,
      LumitColorScheme.nocturneDark,
      room: LanternRoom.night),
  // Lantern with the rail and the deck.
  _Look('lantern-canopy-light-rail-deck', ThemeShape.lantern,
      LumitColorScheme.canopyLight,
      toolBar: ToolBarPosition.left, bars: ViewerBars.deck),
  _Look('lantern-tavern-dark-night-rail-deck', ThemeShape.lantern,
      LumitColorScheme.tavernDark,
      room: LanternRoom.night,
      toolBar: ToolBarPosition.left,
      bars: ViewerBars.deck),
];

Future<void> main() async {
  final (state, ui) = await bootLumit();
  if (Platform.environment['LUMIT_SHOTS_OUT'] == null) {
    // ignore: avoid_print
    print('SKIPPED: set LUMIT_SHOTS_OUT, these are review artefacts, and the '
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
    document: const BridgeTextDocument(
      animators: [],
      pathOffset: BridgeScalar.static_(0),
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
  // Two effects on the Title, so the Effect controls card has rows to show in
  // every look rather than an empty panel.
  title.addEffect(name: 'blur');
  title.addEffect(name: 'glow');

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
  // The layout preset the default layout already is, applied so the workspace
  // strip has an active member to show.
  ui.workspace.applyWorkspacePreset(WorkspacePreset.edit);

  // The sweeps photograph the shell; the welcome screen has its own sweep.
  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));

  await pause(2);
  // A 1920 by 1080 client area: the frame takes 16 across and 39 down.
  await sizeWindow(1936, 1119);
  // The media is read and the two effects rendered before the first frame;
  // on a busy machine that takes longer than a bare scene does.
  await pause(14);

  // Selecting the Title raises its gizmo and points Effect controls at its
  // stack; fronting the tab puts that stack in the left card for every look.
  ui.setSelection([title]);
  // The Project panel stays on the left and the Effect controls join the
  // right group, fronted, so a shot shows both.
  movePanel(ui.workspace.dock, Panel.effectControls.pane(),
      Panel.effectsAndPresets.pane(), DropPosition.stack);
  ui.workspace.touch();
  ui.frontPanel(Panel.effectControls);
  await pause(3);

  for (final look in _looks) {
    // The calls Settings, Appearance makes, against the sweep's throwaway
    // store: the style, the scheme, then the three rows the styles added.
    ui.workspace.setShape(look.shape);
    ui.workspace.setScheme(look.scheme);
    ui.workspace.interface
      ..room = look.room
      ..toolBarPosition = look.toolBar
      ..viewerBars = look.bars;
    ui.workspace.recompose();
    ui.workspace.settingsChanged();
    await pause(3);
    await captureUi('${look.name}.png');
  }

  exit(0);
}
