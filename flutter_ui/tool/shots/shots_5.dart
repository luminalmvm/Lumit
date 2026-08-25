// Manual screenshots, sweep 5: the effects panels.
//
// effect-controls · effect-menu · effects-presets · presets · scopes
//
// Staged in the Effects workspace, which is the arrangement these pages are
// about: Effect controls in its own column beside the Project panel, Effects &
// presets down the right with Scopes tabbed behind it.
//
// The two saved presets are real ones — the selected layer's stack, written out
// through `savePreset` into the library folder the panel lists, exactly as the
// Save preset button does — and they are **deleted again on the way out**, so a
// sweep does not leave things in the machine owner's library.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1   # PowerShell; LUMIT_SHOTS=1 elsewhere
//   flutter run -d windows -t tool/shots/shots_5.dart

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/effects_presets_panel_frb.dart';
import 'package:lumit_flutter/panels/scopes_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';

import 'shots_common.dart';

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
      duration: BridgeRational(num: 10, den: 1),
      background: F32Array4(Float32List.fromList([0, 0, 0, 1])),
      shutterAngle: 180,
      motionBlurSamples: 16,
    ),
  );

  comp.addFootageLayer(
    footage: project.importFootage(path: '$fixtures/Gameplay.mp4'),
    asSequence: false,
  );
  comp.addFootageLayer(
    footage: project.importFootage(path: '$fixtures/Title card.mp4'),
    asSequence: false,
  );
  final title = comp.addTextLayer();
  title.setText(
    document: const BridgeTextDocument(
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
  for (final (index, name) in ['Title', 'Title card', 'Gameplay'].indexed) {
    layers[index].rename(name: name);
  }
  final card = layers[1];
  final gameplay = layers[2];
  card.setTransform(
    prop: BridgeTransformProp.opacity,
    value: const BridgeScalar.static_(55),
  );

  // Two effects on one layer: a grade and a bloom, in the order somebody would
  // put them — the stack runs top to bottom, so the glow is picking up an
  // already-graded picture.
  gameplay.addEffect(name: 'exposure');
  gameplay.addEffect(name: 'glow');
  // Moved off their defaults, so the panel reads as a grade somebody set rather
  // than as two effects dropped on a layer a second ago. Staged on the handles
  // and committed in one `setEffects`, which is the shape every stack edit has.
  final stack = gameplay.getEffects();
  stack[0].setValue(
      id: 'stops',
      value: const BridgeEffectValue.float(BridgeScalar.static_(0.6)));
  stack[1].setValue(
      id: 'radius',
      value: const BridgeEffectValue.float(BridgeScalar.static_(48)));
  stack[1].setValue(
      id: 'intensity',
      value: const BridgeEffectValue.float(BridgeScalar.static_(1.6)));
  stack[1].setValue(
      id: 'threshold',
      value: const BridgeEffectValue.float(BridgeScalar.static_(0.62)));
  gameplay.setEffects(effects: stack);

  // The saved-preset library. Written before the panel is ever built, because
  // it reads the folder once when it is created — which is also the only way a
  // user's own presets are ever there before the application starts.
  final library = presetsDirPath();
  final written = <File>[];
  if (library != null) {
    for (final name in ['Neon grade', 'Soft bloom']) {
      // Written whether or not one of this name is already there, and taken
      // away again on the way out either way. It used to skip an existing file
      // and then delete nothing — so a sweep that died before its last shot
      // left two presets in the library, and the *next* run skipped them,
      // never deleted them, and photographed them in every panel that lists
      // the library.
      final file = File('$library/$name.lumfx')
        ..writeAsStringSync(gameplay.savePreset(name: name));
      written.add(file);
    }
  } else {
    // ignore: avoid_print
    print('NO PRESET LIBRARY: presets.png cannot be staged');
  }

  ui.setSelectedComp(comp);
  ui.playheadFrame.value = 48;
  ui.setSelection([gameplay]);
  // The arrangement these pages are about.
  ui.workspace.applyWorkspacePreset(WorkspacePreset.effects);

  // The sweeps photograph the shell; the welcome screen has its own sweep.
  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));
  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(7);

  /// The whole of a docked panel, chrome and all — grown out to the pane card
  /// it sits in, which under Round is the rounded edge the design is made of.
  Rect panel(Type type) => boxOfType(type)!.inflate(paneCardInset);

  // ---- Shot: Effect controls, with two effects on one layer ---------------
  // Nothing is clicked here on purpose: a card arrives twirled **open**, so a
  // sweep that pressed the twirls would be a sweep that shut them and
  // photographed two names with no parameters under them.
  final controls = panel(EffectControlsPanelFrb);
  // Cut a little under the last row rather than at the panel's floor: the tail
  // of an effect stack is empty by definition, and two thirds of this shot
  // being empty panel is two thirds of it saying nothing.
  final lastRow = boxOf('fx-row-${gameplay.getEffects()[1].id()}-mix');
  await captureUi(
    'effect-controls.png',
    scale: 2,
    crop: Rect.fromLTRB(controls.left, controls.top, controls.right,
        (lastRow?.bottom ?? controls.bottom - 24) + 24),
  );

  // ---- Shot: the Effects & presets panel ----------------------------------
  await captureUi(
    'effects-presets.png',
    scale: 2,
    crop: panel(EffectsPresetsPanelFrb),
  );

  // ---- Shot: saved presets at the top of the panel ------------------------
  // The library sits above the built-ins, so the crop is the top of the same
  // panel: the two saved names, and the first heading under them for context.
  final browser = panel(EffectsPresetsPanelFrb);
  await captureUi(
    'presets.png',
    scale: 3,
    crop: Rect.fromLTRB(
        browser.left, browser.top, browser.right, browser.top + 190),
  );

  // ---- Shot: the Effect menu, category by category ------------------------
  // Opened from the bar and then one category opened off it, because the whole
  // of what this menu is is categories with effects behind them — a shot of the
  // six headings alone would not show that.
  await tapKey('menu-Effect', settle: 1.2);
  await tapKey('menu-sub-Colour', settle: 1.4);
  await captureUi(
    'effect-menu.png',
    scale: 2,
    crop: const Rect.fromLTWH(0, 0, 810, 380),
  );
  await tapAt(const Offset(1500, 700), settle: 1);

  // ---- Shot: the scopes reading the current frame -------------------------
  // Scopes is tabbed behind Effects & presets in this arrangement, so it is
  // brought to the front the way the panel-focus chord brings it — and then
  // left alone, because a trace is a real render that arrives on the worker
  // stream a moment later.
  activatePanelTab(ui.workspace.dock, Panel.scopes);
  ui.activePanel.value = Panel.scopes;
  ui.workspace.touch();
  await pause(2);
  // A nudge of the playhead: the panel asks for a trace when the frame moves,
  // and it has been sitting on frame 48 since the window opened.
  ui.playheadFrame.value = 60;
  await pause(6);
  await captureUi('scopes.png', scale: 2, crop: panel(ScopesPanelFrb));

  for (final file in written) {
    file.deleteSync();
  }
  exit(0);
}
