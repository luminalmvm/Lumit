// Resolve links identify the document; paths are only location hints.
import 'dart:convert';
import 'dart:io';

import 'package:archive/archive_io.dart';
import 'package:flutter/foundation.dart' show compute;
import 'package:flutter/widgets.dart';

import '../state/app_state.dart';
import '../state/file_dialogs.dart';
import '../widgets/controls.dart' show showLumitModal;
import 'question_window.dart';

bool _identity(String value) =>
    RegExp(r'^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$').hasMatch(value);

bool sameResolveProject(String? actual, String expected) =>
    actual != null &&
    actual.replaceAll('\\', '/').toLowerCase() ==
        expected.replaceAll('\\', '/').toLowerCase();

String _readIdentity(String path) {
  // File-backed ZIP access leaves packed footage unread. The short-lived
  // isolate keeps JSON parsing off the editor's UI thread.
  final input = InputFileStream(path);
  try {
    final archive = ZipDecoder().decodeStream(input);
    try {
      final bytes = archive.find('project.json')?.readBytes();
      if (bytes == null) throw const FormatException('Not a Lumit project');
      final id = (jsonDecode(utf8.decode(bytes)) as Map<String, dynamic>)['id'];
      if (id is! String || !_identity(id.toLowerCase())) {
        throw const FormatException('Missing Lumit project identity');
      }
      return id.toLowerCase();
    } finally {
      archive.clearSync();
    }
  } finally {
    input.closeSync();
  }
}

Future<String> resolveProjectIdentity(String path) =>
    compute(_readIdentity, path);

Future<String> _rememberLocation(String path, String? hint, String id) async {
  final local = Platform.environment['LOCALAPPDATA'];
  if (local != null) {
    final file = File('$local/Lumit/Resolve Preview/$id.location');
    final paths = <String>{path};
    if (await file.exists()) paths.addAll(await file.readAsLines());
    if (hint != null && hint.isNotEmpty) paths.add(hint);
    await file.parent.create(recursive: true);
    await file.writeAsString(paths.join('\n'), flush: true);
  }
  return path;
}

Future<String?> locateResolveProject(LumitState app, String? hint, String? id,
    {required BuildContext context}) async {
  if (id != null && !_identity(id)) {
    throw const FormatException('Invalid linked Lumit project identity');
  }
  // An explicitly chosen existing file wins over historical path hints.
  if (id == null && hint != null && await File(hint).exists()) {
    final actual = await resolveProjectIdentity(hint);
    return _rememberLocation(hint, hint, actual);
  }
  String? remembered;
  final local = Platform.environment['LOCALAPPDATA'];
  if (id == null && hint != null && local != null) {
    final folder = Directory('$local/Lumit/Resolve Preview');
    if (await folder.exists()) {
      await for (final entry in folder.list()) {
        if (entry is! File || !entry.path.endsWith('.location')) continue;
        if (!(await entry.readAsLines())
            .any((p) => sameResolveProject(p, hint))) {
          continue;
        }
        final candidate = entry.uri.pathSegments.last.split('.').first;
        if (!_identity(candidate)) continue;
        if (id != null && id != candidate) {
          throw const FormatException(
              'This path belongs to multiple linked projects. Choose its current path.');
        }
        id = candidate;
      }
    }
  }
  if (id != null && local != null) {
    final file = File('$local/Lumit/Resolve Preview/$id.location');
    if (await file.exists()) {
      remembered = (await file.readAsLines()).firstOrNull;
    }
  }
  for (final path in {remembered, hint}) {
    if (path == null || path.isEmpty || !await File(path).exists()) continue;
    try {
      final actual = await resolveProjectIdentity(path);
      if (id == null || actual == id) {
        return await _rememberLocation(path, hint, actual);
      }
    } catch (_) {
      // An invalid/stale location is not a replacement project.
    }
  }
  if (!context.mounted) return null;
  final locate = await showLumitModal<bool>(
    context: context,
    builder: (close) => QuestionWindow(
      title: 'Linked project not found',
      body: 'The linked Lumit project may have been moved or renamed. '
          'Locate it once to reconnect all its linked compositions in Resolve.',
      keyPrefix: 'resolve-project-missing',
      onClose: () => close(false),
      actions: [
        questionPlain(
            'resolve-project-missing-cancel', 'Cancel', () => close(false)),
        questionFilled('resolve-project-missing-locate', 'Locate project',
            () => close(true)),
      ],
    ),
  );
  if (locate != true || !context.mounted) return null;
  while (true) {
    final path = await pickProjectToOpen();
    if (path == null) return null;
    try {
      final actual = await resolveProjectIdentity(path);
      if (id == null || actual == id) {
        return await _rememberLocation(path, hint, actual);
      }
      app.postNotice(
          'That file is a different Lumit project. Choose the original project.',
          error: true);
    } catch (_) {
      app.postNotice('That file is not a valid Lumit project.', error: true);
    }
  }
}
