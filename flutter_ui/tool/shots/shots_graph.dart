// Manual screenshots, sweep: the Node graph panel, and the workspace built
// around it (phase 3, K-471..473).
//
// nodes-workspace.png — the Nodes workspace as its own tab in the toolbar sets
// it up: the graph across most of the window, the Viewer beside it, the
// Timeline underneath.
//
// graph-panel.png — the panel itself with a real wired layer: Source → Glow →
// Layer out on the image chain, and a Wiggle driver wired into one of Glow's
// parameters, so the shot shows a node card, a typed wire in its port colour,
// and the driven socket filled.
//
// Both are photographed in the Nodes workspace rather than with the panel
// pulled into the editing dock: a graph is wide, and the narrow left-hand
// column it lands in otherwise cuts the second node off the picture.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1   # PowerShell; LUMIT_SHOTS=1 elsewhere
//   flutter run -d windows -t tool/shots/shots_graph.dart

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
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
      duration: const BridgeRational(num: 10, den: 1),
      background: F32Array4(Float32List.fromList([0, 0, 0, 1])),
      shutterAngle: 180,
      motionBlurSamples: 16,
    ),
  );
  ui.setSelectedComp(comp);
  final layer = comp.addSolidLayer();
  layer.addEffect(name: 'glow');

  // The driver, committed first so the read model can name its ports, then
  // the wire: Wiggle's value into Glow's radius.
  final wiggle = layer.newDriver(name: 'wiggle');
  layer.setGraph(
    drivers: [wiggle],
    wiring: const BridgeGraphWiring(edges: [], layout: [], exposed: [], groups: []),
  );
  final g = layer.getGraph();
  final driverNode = g.nodes.firstWhere((n) => n.node is BridgeNodeRef_Driver);
  final effectNode = g.nodes.firstWhere((n) =>
      n.node is BridgeNodeRef_Effect &&
      n.inputs.any((p) => p.portType == BridgePortType.number));
  final numberIn =
      effectNode.inputs.firstWhere((p) => p.portType == BridgePortType.number);
  layer.setGraph(
    drivers: layer.getGraphDrivers(),
    wiring: BridgeGraphWiring(
      groups: const [],
      edges: [
        BridgeGraphEdge(
          from: BridgeOutputRef.driver(
              node: (driverNode.node as BridgeNodeRef_Driver).field0,
              port: driverNode.outputs.first.id),
          to: BridgeInputRef.param(node: effectNode.node, port: numberIn.id),
        ),
      ],
      layout: const [],
      exposed: const [],
    ),
  );
  ui.setSelection([layer]);

  // The Nodes workspace, applied the way its tab in the toolbar applies it.
  ui.workspace.applyWorkspacePreset(WorkspacePreset.nodes);

  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));

  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(5);

  await captureUi('nodes-workspace.png');
  await captureUi('graph-panel.png',
      crop: boxOfType(GraphPanelFrb)?.inflate(dockTabInset));
  exit(0);
}
