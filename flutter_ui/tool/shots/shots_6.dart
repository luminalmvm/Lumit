// Manual screenshots, sweep 6: the two tree panels.
//
// project-panel · hierarchy
//
// One project serves both: a title sequence that uses a second composition as a
// precomp, which is exactly the shape the Hierarchy page is about — "this shot
// uses that comp" — and gives the Project panel a composition to hold beside
// the footage.
//
// **No folder is staged, and that is not an oversight.** The Project panel
// draws folders, and the document model has them, but nothing in the bridge
// makes one: there is no New folder command anywhere in the application. A
// folder in this shot would have to be written into a document by hand, and the
// manual would then show a control the reader cannot find.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1   # PowerShell; LUMIT_SHOTS=1 elsewhere
//   flutter run -d windows -t tool/shots/shots_6.dart

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/hierarchy_panel_frb.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';

import 'shots_common.dart';

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

  // The precomp first, so the titles can use it. Three layers inside, because a
  // precomp twirled open in the Hierarchy panel is only worth photographing if
  // there is something under it.
  final lower = project.newComposition(
    name: 'Lower third',
    settings: _settings('Lower third', 4),
  );
  final bar = lower.addSolidLayer();
  bar.rename(name: 'Bar');
  final role = lower.addTextLayer();
  role.rename(name: 'Role');
  role.setText(
    document: const BridgeTextDocument(
      text: 'Director of photography',
      size: 48,
      fill: BridgeColourRgba(r: 0.8, g: 0.82, b: 0.86, a: 1),
    ),
  );
  final person = lower.addTextLayer();
  person.rename(name: 'Name');
  person.setText(
    document: const BridgeTextDocument(
      text: 'Ada Whitcombe',
      size: 84,
      fill: BridgeColourRgba(r: 1, g: 1, b: 1, a: 1),
    ),
  );

  final comp = project.newComposition(
    name: 'Opening titles',
    settings: _settings('Opening titles', 10),
  );

  // Bottom of the stack upwards.
  final music = project.importFootage(path: '$fixtures/Music.wav');
  final gameplay = project.importFootage(path: '$fixtures/Gameplay.mp4');
  final card = project.importFootage(path: '$fixtures/Title card.mp4');
  comp.addFootageLayer(footage: music, asSequence: false);
  comp.addFootageLayer(footage: gameplay, asSequence: false);
  comp.addFootageLayer(footage: card, asSequence: false);
  comp.addPrecompLayer(comp: lower);
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

  // Imported but not placed — every project has one of those.
  final logo = project.importFootage(path: '$fixtures/Logo.png');

  final layers = comp.getLayers();
  for (final (index, name)
      in ['Title', 'Lower third', 'Title card', 'Gameplay', 'Music'].indexed) {
    layers[index].rename(name: name);
  }
  final precompLayer = layers[1];
  layers[2].setTransform(
    prop: BridgeTransformProp.opacity,
    value: const BridgeScalar.static_(55),
  );

  ui.setSelectedComp(comp);
  ui.playheadFrame.value = 48;

  runApp(shotRoot(LumitAppNew(state, ui)));
  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(7);

  // ---- Shot: the Project panel --------------------------------------------
  // A footage item is clicked first, because the panel's header is the half of
  // it that says what an item *is* — thumbnail, kind, size, rate, length — and
  // with nothing selected that header is deliberately blank.
  await tapKey('project-row-${gameplay.internalid}', settle: 2.5);
  final tree = boxOfType(ProjectPanelFrb)!;
  // Up over the dock's tab strip, so the shot says which panel it is, and down
  // only as far as the last item: the tail of a tree is empty by definition,
  // and half a picture of nothing says nothing.
  await captureUi('project-panel.png',
      scale: 2, crop: _panelCrop(tree, 'project-row-${logo.internalid}'));

  // ---- Shot: the Hierarchy panel, a precomp twirled open -------------------
  // Hierarchy is the third tab of the left group in the default arrangement, so
  // it is fronted the way the Window menu fronts it.
  activatePanelTab(ui.workspace.dock, Panel.hierarchy);
  ui.activePanel.value = Panel.hierarchy;
  ui.workspace.touch();
  await pause(2);

  // The twirl is the 14px box at the head of the row, and clicking the row
  // itself selects rather than opens — so this aims at the arrow, not the row.
  final row = boxOf('hierarchy-row-${precompLayer.internallayerId}');
  if (row != null) {
    await tapAt(Offset(row.left + 13, row.center.dy), settle: 1.5);
  }
  await captureUi(
    'hierarchy.png',
    scale: 3,
    crop: _panelCrop(boxOfType(HierarchyPanelFrb)!,
        'hierarchy-row-${layers.last.internallayerId}'),
  );

  exit(0);
}

/// A docked panel with its tab strip on top and its empty tail cut off.
///
/// `_dockTabHeight` above, because a panel photographed without the tab that
/// names it is a list of rows nobody can place; the floor comes from the last
/// row rather than from the panel, which in a working window is mostly space
/// waiting for rows that are not there yet.
Rect _panelCrop(Rect panel, String lastRowKey) {
  final last = boxOf(lastRowKey);
  return Rect.fromLTRB(
    panel.left - paneCardInset,
    panel.top - dockTabInset,
    panel.right + paneCardInset,
    last == null
        ? panel.bottom
        : (last.bottom + 12).clamp(0, panel.bottom + paneCardInset),
  );
}
