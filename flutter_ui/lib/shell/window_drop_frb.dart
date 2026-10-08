// Files dropped anywhere on the window are imported, the way files dropped on
// the Project panel are.
//
// The drop plugin tells every drop target about every drop, and each one acts
// if the drop was inside its own box. A target the size of the window would
// therefore act alongside the Project panel's own and import the files twice.
// So this one stands down whenever the drop landed inside another target, by
// the same test the plugin makes, and that target is left to do its own work.

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../panels/project_panel_frb.dart' show importDroppedPaths;
import '../state/app_state.dart';
import '../state/dock.dart';
import '../state/ui_state.dart';

class WindowDropFrb extends StatelessWidget {
  final Widget child;
  const WindowDropFrb({super.key, required this.child});

  @override
  Widget build(BuildContext context) => DropTarget(
        onDragDone: (details) async {
          if (_inAnotherTarget(context, details.globalPosition)) return;
          final ui = context.read<LumitUiState>();
          final imported = await importDroppedPaths(
              context.read<LumitState>(), [for (final f in details.files) f.path]);
          // Nothing lit up under the drop, so show where the files went.
          if (imported) ui.frontPanel(Panel.project);
        },
        child: child,
      );
}

/// Whether [position] is inside a drop target other than the one [context]
/// builds. Asked of the whole tree, once a drop, so a target in a dialogue
/// counts as much as one in a panel.
bool _inAnotherTarget(BuildContext context, Offset position) {
  Element? own;
  context.visitChildElements((child) => own = child);
  var found = false;
  void visit(Element element) {
    if (found) return;
    final widget = element.widget;
    if (widget is DropTarget && widget.enable && !identical(element, own)) {
      final box = element.renderObject;
      if (box is RenderBox &&
          box.attached &&
          box.paintBounds.contains(box.globalToLocal(position))) {
        found = true;
        return;
      }
    }
    element.visitChildren(visit);
  }

  WidgetsBinding.instance.rootElement?.visitChildren(visit);
  return found;
}
