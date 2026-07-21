// The real file dialogues (the file_selector plugin), isolated behind plain
// functions so AppStateStub can hold them as injectable seams. A dialogue
// cannot open in a widget test, so tests supply their own stubs and never touch
// a plugin channel; this file is only ever exercised in the running app.

import 'package:file_selector/file_selector.dart';

/// The `.lum` project type group (docs/10 §1). The egui open filter also lists
/// the pre-rename `kir` leftover; a fresh frontend only ever offers `.lum`.
XTypeGroup _projectGroup() =>
    const XTypeGroup(label: 'Lumit project', extensions: ['lum']);

/// The footage type group, mirroring the egui import filter exactly
/// (crates/lumit-ui/src/app_state/layers.rs `import_footage_dialog`).
XTypeGroup _footageGroup() => const XTypeGroup(
      label: 'Footage',
      extensions: [
        'mp4',
        'mov',
        'mkv',
        'avi',
        'webm',
        'png',
        'jpg',
        'jpeg',
        'wav',
        'mp3',
        'flac',
      ],
    );

/// Pick one project file to open, or null when the dialogue was cancelled.
Future<String?> pickProjectToOpen() async {
  final file = await openFile(acceptedTypeGroups: [_projectGroup()]);
  return file?.path;
}

/// Choose where to save a project, defaulting the name to `untitled.lum` (as
/// the egui Save dialogue does), or null when cancelled.
Future<String?> pickProjectSaveLocation() async {
  final location = await getSaveLocation(
    acceptedTypeGroups: [_projectGroup()],
    suggestedName: 'untitled.lum',
  );
  return location?.path;
}

/// Pick one or more footage files, or an empty list when cancelled.
Future<List<String>> pickFootage() async {
  final files = await openFiles(acceptedTypeGroups: [_footageGroup()]);
  return [for (final f in files) f.path];
}
