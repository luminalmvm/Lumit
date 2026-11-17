// Lottie files on their way in. Import footage and a drop on the window hand
// every path here first: the Lottie ones go to the engine's own import, which
// makes a composition of each, and the rest carry on to the footage import.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/vector.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../main.dart';
import 'ae_report_frb.dart';

/// Import the Lottie files among [paths] and answer the others. The last
/// composition made is fronted, and an import that changed or left
/// something out shows the import report.
Future<List<String>> importLottieFrb(
    BuildContext context, LumitState app, List<String> paths) async {
  final project = app.project;
  if (project == null) return paths;
  final rest = <String>[];
  var brought = false;
  for (final path in paths) {
    // Only a .json or a .lottie can be one, so only those are asked about:
    // the rest go on their way without a round trip, in the order they came.
    final dot = path.lastIndexOf('.');
    final ext = dot < 0 ? '' : path.substring(dot + 1).toLowerCase();
    if ((ext != 'json' && ext != 'lottie') ||
        !await isLottieFile(path: path)) {
      rest.add(path);
      continue;
    }
    try {
      final made = await project.importLottie(path: path);
      brought = true;
      // The rest still come in when the caller has gone. Only fronting the
      // comp and showing the report need it.
      if (!context.mounted) continue;
      context.read<LumitUiState>().setSelectedComp(made.comp);
      if (made.report.rows.isNotEmpty) {
        await showAeImportReport(context: context, report: made.report);
      }
    } catch (_) {
      app.postNotice(l10n.couldNotOpen(path), error: true);
    }
  }
  if (brought) app.notifyDocumentChanged();
  return rest;
}
