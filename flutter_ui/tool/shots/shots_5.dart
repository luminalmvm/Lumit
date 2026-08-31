// Manual screenshots, sweep 5: the effects panels.
//
// effect-controls · effect-menu · effects-presets · presets · scopes ·
// effects-presets-ofx
//
// The last of those is **conditional**. `lib/main.dart` scans for OFX plugins
// at start-up; a sweep boots without that, so this one asks for the scan
// itself and photographs the panel again with the plugins in it. On a machine
// with none installed the scan comes back empty, and the sweep says so and
// takes no picture rather than shipping a shot of a heading that is not there.
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
import 'package:lumit_flutter/src/rust/lib.dart';
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
    document: const BridgeTextDocument(animators: [], pathOffset: BridgeScalar.static_(0), 
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
  Rect panel(Type type) => dockedPanelBox(type)!.inflate(paneCardInset);

  /// The same, with the dock's tab strip above it — for the shots whose caption
  /// is about *this panel*, which is a thing the reader finds by the name on
  /// its tab. A crop that starts at the content is a column of rows that could
  /// belong to anything.
  Rect tabbed(Type type) {
    final b = panel(type);
    return Rect.fromLTRB(b.left, b.top - dockTabInset, b.right, b.bottom);
  }

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
    crop: tabbed(EffectsPresetsPanelFrb),
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
  // From the corner, because the bar the menu drops out of is half the subject.
  // The far edges are measured off the two open surfaces rather than fixed: a
  // category added to the menu, or an effect added to a category, moves them.
  final menus = openPopupsBox()!;
  await captureUi(
    'effect-menu.png',
    scale: 2,
    crop: Rect.fromLTRB(0, 0, menus.right, menus.bottom),
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
  await captureUi('scopes.png', scale: 2, crop: tabbed(ScopesPanelFrb));

  // ---- Shot: the panel with an OFX plugin's own heading in it --------------
  // The scan is the same call the application makes as it starts, so what
  // appears is what a reader with those plugins installed would see. A plugin
  // declares its own grouping rather than borrowing one of Lumit's ten, so its
  // heading is far enough down a full list to be off the bottom of the panel —
  // the built-in categories are folded away to bring it up.
  activatePanelTab(ui.workspace.dock, Panel.effectsAndPresets);
  ui.activePanel.value = Panel.effectsAndPresets;
  ui.workspace.touch();
  await pause(1.5);

  final scan = await rescanPlugins();
  final plugins =
      listEffects().where((e) => e.namespace == 'ofx').toList(growable: false);
  // ignore: avoid_print
  print('PLUGINS registered ${scan.registered.length}, '
      'skipped ${scan.skipped.length}, listed ${plugins.length}');
  for (final line in scan.skipped) {
    // ignore: avoid_print
    print('PLUGIN SKIPPED $line');
  }
  if (plugins.isEmpty) {
    // ignore: avoid_print
    print('NO OFX PLUGINS: effects-presets-ofx.png skipped');
  } else {
    final pluginGroups = plugins.map((e) => e.category).toSet();
    // One effect out of each built-in group, to tell an open fold from a shut
    // one: a shut group's rows are not built, so the absence of its first row
    // is how the sweep knows not to click the heading again and flap it open.
    final firstRow = <String, String>{};
    for (final effect in listEffects()) {
      if (pluginGroups.contains(effect.category)) continue;
      firstRow.putIfAbsent(effect.category, () => effect.name);
    }
    // Several passes, because the list builds only the rows it is showing: a
    // heading below the fold is not on screen to be clicked, and each fold that
    // shuts pulls the next one up into reach.
    for (var pass = 0; pass < 6; pass++) {
      for (final MapEntry(key: group, value: row) in firstRow.entries) {
        if (elementByKey('fx-item-$row') == null) continue;
        await tapKey('fx-group-$group', settle: 0.12);
      }
    }
    await pause(2);
    await captureUi(
      'effects-presets-ofx.png',
      scale: 2,
      crop: tabbed(EffectsPresetsPanelFrb),
    );
  }

  for (final file in written) {
    file.deleteSync();
  }
  exit(0);
}
