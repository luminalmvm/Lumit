// Files dropped anywhere on the window are imported, the way files dropped on
// the Project panel are. An extension package dropped anywhere opens its
// install window instead, a style is taken in and used, a graphics template
// joins the library, and a script is run after a question.
//
// The drop plugin tells every drop target about every drop, and each one acts
// if the drop was inside its own box. A target the size of the window would
// therefore act alongside the Project panel's own and import the files twice.
// So this one stands down whenever the drop landed inside another target, by
// the same test the plugin makes, and that target is left to do its own work.

import 'dart:async';

import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../panels/project_panel_frb.dart' show importDroppedPaths;
import '../panels/template_place.dart' show installTemplateFiles;
import '../state/file_dialogs.dart' show droppedPaths;
import 'extension_install_frb.dart';
import 'lottie_import_frb.dart';
import 'style_import.dart';
import '../src/rust/api/styles.dart';
import '../state/app_state.dart';
import '../state/dock.dart';
import '../state/ui_state.dart';
import 'run_script_frb.dart';

class WindowDropFrb extends StatelessWidget {
  final Widget child;
  const WindowDropFrb({super.key, required this.child});

  @override
  Widget build(BuildContext context) => DropTarget(
        onDragDone: (details) async {
          final paths = await droppedPaths(details.files);
          if (!context.mounted) return;
          final ui = context.read<LumitUiState>();
          // Wherever it lands, since no other target takes a package.
          final package = packagePathFromArgs(paths);
          if (package != null) {
            unawaited(installExtensionPackage(
                context, context.read<LumitState>(), ui.extensions, package));
          }
          // A style too, and a zipped folder that holds one.
          final style = stylePathFromArgs(paths) ??
              paths
                  .where((path) => path.toLowerCase().endsWith('.zip'))
                  .where((path) =>
                      styleReadPackage(path: path) is BridgeStyleOutcome_Ready)
                  .firstOrNull;
          if (style != null) {
            unawaited(
                importStyle(context, context.read<LumitState>(), ui, style));
          }
          // And graphics templates, which join the library.
          final templates = paths
              .where((path) => path.toLowerCase().endsWith('.lumt'))
              .toList();
          if (templates.isNotEmpty) {
            unawaited(installTemplateFiles(
                context, context.read<LumitState>(), ui, templates));
          }
          if (_inAnotherTarget(context, details.globalPosition)) return;
          // A Lottie makes a composition of its own; the rest are footage.
          final rest = await importLottieFrb(
              context, context.read<LumitState>(), paths);
          if (!context.mounted) return;
          final imported = await importDroppedPaths(context.read<LumitState>(),
              rest.where((path) => !isScriptFile(path)).toList());
          // Nothing lit up under the drop, so show where the files went.
          if (imported) ui.frontPanel(Panel.project);
          if (context.mounted) await runDroppedScripts(context, paths);
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
