// The real file dialogues (the file_selector plugin), isolated behind plain
// functions so AppStateStub can hold them as injectable seams. A dialogue
// cannot open in a widget test, so tests supply their own stubs and never touch
// a plugin channel; this file is only ever exercised in the running app.

import 'package:file_selector/file_selector.dart';
import 'package:lumit_flutter/l10n/strings.dart';

import '../theme/theme_file.dart' show themeFileExtension;

/// The `.lum` project type group (docs/10 §1). The egui open filter also lists
/// the pre-rename `kir` leftover; a fresh frontend only ever offers `.lum`.
XTypeGroup _projectGroup() =>
    XTypeGroup(label: l10n.fileTypeProject, extensions: const ['lum']);

/// The footage type group, mirroring the egui import filter exactly
/// (crates/lumit-ui/src/app_state/layers.rs `import_footage_dialog`).
XTypeGroup _footageGroup() => XTypeGroup(
      label: l10n.fileTypeFootage,
      extensions: const [
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

/// Choose where to save an exported video, defaulting the name to the
/// resolver's [suggestedName] (the egui exporter's `set_file_name`), or null
/// when cancelled. The generic save seam the export dialogue and the share
/// exports both drive. `extension`/`label` follow the chosen format (K-201):
/// `.mp4` for video, the image extension for a sequence — where the picked
/// name is the sequence's stem, and the frames land numbered beside it.
Future<String?> pickExportSaveLocation(
  String suggestedName, {
  String extension = 'mp4',
  String label = 'MP4 video',
}) async {
  final location = await getSaveLocation(
    acceptedTypeGroups: [
      XTypeGroup(label: label, extensions: [extension])
    ],
    suggestedName: suggestedName,
  );
  return location?.path;
}

/// The `.lumfx` effect-preset type group, mirroring the egui Effects panel's
/// preset filter (`crates/lumit-ui`'s `preset.rs`).
XTypeGroup _presetGroup() =>
    XTypeGroup(label: l10n.fileTypePreset, extensions: const ['lumfx']);

/// Pick one `.lumfx` preset file to load, or null when cancelled.
Future<String?> pickPresetToOpen() async {
  final file = await openFile(acceptedTypeGroups: [_presetGroup()]);
  return file?.path;
}

/// Choose where to save a `.lumfx` preset, defaulting the name to
/// [suggestedName] and the folder to [initialDirectory] (the preset library,
/// so a plain save lands where the browser lists), or null when cancelled.
Future<String?> pickPresetSaveLocation(String suggestedName,
    {String? initialDirectory}) async {
  final location = await getSaveLocation(
    acceptedTypeGroups: [_presetGroup()],
    suggestedName: suggestedName,
    initialDirectory: initialDirectory,
  );
  return location?.path;
}

/// The keymap type group (docs/07 §15's shareable file, K-199). Plain JSON, so
/// a `.json` a user has renamed still opens.
XTypeGroup _keymapGroup() =>
    XTypeGroup(label: l10n.fileTypeKeymap, extensions: const ['json']);

/// Pick a keymap file to import, or null when the dialogue was cancelled.
Future<String?> pickKeymapToOpen() async {
  final file = await openFile(acceptedTypeGroups: [_keymapGroup()]);
  return file?.path;
}

/// The shared-theme type group (K-298), the theme's counterpart of the
/// keymap's. Lumit's own extension rather than a plain `.json`, so the picker
/// can offer just themes.
XTypeGroup _themeGroup() => XTypeGroup(
    label: l10n.fileTypeTheme, extensions: const [themeFileExtension]);

/// Pick a theme file to import, or null when the dialogue was cancelled.
Future<String?> pickThemeToOpen() async {
  final file = await openFile(acceptedTypeGroups: [_themeGroup()]);
  return file?.path;
}

/// Choose where to write a theme, defaulting the name to the theme's own
/// ([suggestedName], from `themeFileName`), or null when cancelled.
Future<String?> pickThemeSaveLocation(String suggestedName) async {
  final location = await getSaveLocation(
    acceptedTypeGroups: [_themeGroup()],
    suggestedName: suggestedName,
  );
  return location?.path;
}

/// Pick a Lumit bundle to import from After Effects (docs/11 §2.1) — a
/// **folder**, because a folder is what the Bridge's script writes: ExtendScript
/// has no zip. The reader also opens a zipped bundle, which this dialogue cannot
/// reach; unzipping it first is the route until there is a picker that offers
/// both (docs/TODO.md). Null when cancelled.
Future<String?> pickAeBundle() =>
    getDirectoryPath(confirmButtonText: l10n.chooseConfirm);

/// Pick a folder — Settings → Performance's cache location, where the disk tier
/// parks its frames (docs/07 §15). Null when the dialogue was cancelled.
Future<String?> pickFolder() =>
    getDirectoryPath(confirmButtonText: l10n.chooseConfirm);

/// Choose where to write a keymap, or null when cancelled.
Future<String?> pickKeymapSaveLocation() async {
  final location = await getSaveLocation(
    acceptedTypeGroups: [_keymapGroup()],
    suggestedName: 'lumit-keymap.json',
  );
  return location?.path;
}

/// Pick one file for an effect's File parameter (docs/08 §1.2's File kind,
/// K-265) — the LUT's `.cube`, the Lens flare's `.lens`. The schema's own
/// lower-case extensions and label drive the dialogue's filter, so a new
/// File parameter needs no new function here. Null when cancelled.
Future<String?> pickEffectInputFile(
  List<String> extensions,
  String label,
) async {
  final file = await openFile(
    acceptedTypeGroups: [XTypeGroup(label: label, extensions: extensions)],
  );
  return file?.path;
}
