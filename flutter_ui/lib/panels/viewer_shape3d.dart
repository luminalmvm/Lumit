// The 3D shape tool: a click in the Viewer puts the shape in hand there, at the
// comp's own depth, the way the Type tool puts a line of text.

import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/state/tools.dart';

import '../shell/model_settings_dialog.dart' show modelLightNames;
import 'viewer_shape_layer.dart' show ShapeSpace;

/// Which entry of the engine's 3D shape menu each tool makes, in its order.
const List<ToolMode> shape3dTools = [
  ToolMode.shape3dCube,
  ToolMode.shape3dSphere,
  ToolMode.shape3dCylinder,
  ToolMode.shape3dCone,
  ToolMode.shape3dTorus,
  ToolMode.shape3dPlane,
  ToolMode.shape3dPyramid,
];

/// Add 3D shape [tool] to [comp] at comp point [at], or its centre, selected,
/// as one undo step with any lights it brings.
void addShape3d(LumitUiState ui, CompositionReference comp, ToolMode tool,
    {Offset? at, int? row}) {
  final layer = comp.addMeshLayer(
    entry: shape3dTools.indexOf(tool),
    row: row,
    at: at == null ? null : Float64List.fromList([at.dx, at.dy]),
    lightNames: modelLightNames(),
  );
  ui.setSelection([layer]);
}

class ViewerShape3dLayer extends StatelessWidget {
  /// Whether a 3D shape tool is armed. Inert otherwise.
  final bool active;
  final ToolMode tool;
  final CompositionReference comp;
  final LumitUiState uiState;

  /// Where the picture sits on screen, and the comp's size in its own pixels.
  final Rect fitted;
  final Size compSize;
  final VoidCallback onChanged;

  const ViewerShape3dLayer({
    super.key,
    required this.active,
    required this.tool,
    required this.comp,
    required this.uiState,
    required this.fitted,
    required this.compSize,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    if (!active) return const SizedBox.shrink();
    return Positioned.fill(
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTapUp: (details) {
          final space = ShapeSpace.ofComp(fitted: fitted, compSize: compSize);
          final (x, y) = space.ofScreen(details.localPosition);
          addShape3d(uiState, comp, tool, at: Offset(x, y));
          onChanged();
        },
      ),
    );
  }
}
