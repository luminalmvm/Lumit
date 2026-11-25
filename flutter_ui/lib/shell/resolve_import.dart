import 'dart:convert';
import 'dart:io';
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/file_dialogs.dart';
import '../state/ui_state.dart';
import '../src/rust/api/project.dart' show compsInFile;
import '../widgets/controls.dart' show showLumitModal;
import 'question_window.dart';
import 'resolve_project.dart';

bool _importing = false;

Future<void> _reply(File file, Map<String, String> value) async {
  final pending = File('${file.path}.tmp');
  await pending.writeAsString(jsonEncode(value), flush: true);
  await pending.rename(file.path);
}

Future<void> importResolveSelection(
    LumitState app, LumitUiState ui, String requestPath,
    {required BuildContext context}) async {
  final file = File(requestPath);
  final reply = File('${file.parent.path}/imported.json');
  if (_importing) {
    await _reply(reply, {'error': l10n.resolveImportRunning});
    return;
  }
  _importing = true;
  try {
    if (app.busy.value != null || app.opening.value) {
      throw StateError(l10n.resolveBusy);
    }
    final request =
        jsonDecode(await file.readAsString()) as Map<String, dynamic>;
    var path = request['project'] as String?;
    final source = request['source'] as String;
    final comps = await compsInFile(path: source);
    if (comps == null || comps.length != 1) {
      throw StateError(l10n.resolveOneComp);
    }
    if (!context.mounted) {
      await _reply(reply, {'cancelled': '1'});
      return;
    }
    if (path == null) {
      final create = await showLumitModal<bool>(
          context: context,
          builder: (close) => QuestionWindow(
                title: l10n.resolveProjectTitle,
                body: l10n.resolveProjectBody(
                    '${request['project_name']}'),
                keyPrefix: 'resolve-project',
                onClose: () => close(null),
                actions: [
                  questionPlain('resolve-project-existing',
                      l10n.resolveProjectExisting, () => close(false)),
                  questionFilled('resolve-project-new',
                      l10n.resolveProjectNew, () => close(true)),
                ],
              ));
      if (create != null) {
        final name = (request['project_name'] as String? ??
                l10n.resolveProjectDefaultName)
            .replaceAll(RegExp(r'[<>:"/\\|?*\x00-\x1f]'), '_')
            .trim();
        path = create
            ? await pickProjectSaveLocation(suggestedName: '$name.lum')
            : await pickProjectToOpen();
      }
    } else {
      path = await locateResolveProject(
          app, path, request['projectId'] as String?,
          context: context);
    }
    if (path == null) {
      await _reply(reply, {'cancelled': '1'});
      return;
    }
    if (!sameResolveProject(app.project?.path(), path)) {
      if (await File(path).exists()) {
        await app.openProject(path);
        if (!sameResolveProject(app.project?.path(), path)) {
          await _reply(reply, {'cancelled': '1'});
          return;
        }
      } else {
        final previous = app.project;
        await app.newProject(blank: true);
        if (identical(previous, app.project)) {
          await _reply(reply, {'cancelled': '1'});
          return;
        }
        await app.project!.save(path: path);
      }
    }
    final imported = await app.project!.bringComps(
        path: source,
        comps: [comps.single.id],
        earlierSave: false,
        source: 'Resolve');
    if (imported == null || imported.length != 1) {
      throw StateError(l10n.resolveImportClipsFailed);
    }
    final comp = imported.single;
    ui.setSelectedComp(comp);
    await app.project!.save(path: path);
    app.refreshWindowTitle();
    final name =
        app.comps().firstWhere((c) => c.$1.internalid == comp.internalid).$2;
    await _reply(reply, {
      'project': path,
      'projectId': await resolveProjectIdentity(path),
      'composition': comp.internalid.toString(),
      'name': name,
    });
  } catch (error) {
    await _reply(reply, {'error': resolveErrorText(error)});
    app.postNotice(l10n.resolveImportFailed(resolveErrorText(error)),
        error: true);
  } finally {
    _importing = false;
  }
}
