// The real file dialogues (the file_selector plugin), isolated behind plain
// functions so AppStateStub can hold them as injectable seams. A dialogue
// cannot open in a widget test, so tests supply their own stubs and never touch
// a plugin channel; this file is only ever exercised in the running app.

import 'dart:io' hide Platform;

import 'package:file_selector/file_selector.dart';
import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';

import '../theme/theme_file.dart' show themeFileExtension;
import '../theme/style_sheet.dart' show styleFileExtension;
import 'app_state.dart' show LumitState;
import 'workspace.dart' show workspaceFileExtension;
import 'package:lumit_flutter/state/platform.dart'
    if (dart.library.js_interop) 'package:lumit_flutter/state/platform_web.dart';
import 'files.dart' if (dart.library.js_interop) 'files_web.dart';

// A picked file is reached through these: in the browser build it is put
// where the engine can open it, and a saved project is handed to the person.
export 'files.dart' if (dart.library.js_interop) 'files_web.dart'
    show handOver, importsHere;

/// The `.lum` project type group (docs/10 §1). The egui open filter also lists
/// the pre-rename `kir` leftover; a fresh frontend only ever offers `.lum`.
XTypeGroup _projectGroup() =>
    XTypeGroup(label: l10n.fileTypeProject, extensions: const ['lum']);

/// The `.lumt` graphics template type group, which Import adds to the
/// library.
XTypeGroup _templateGroup() => XTypeGroup(
    label: l10n.fileTypeGraphicsTemplate, extensions: const ['lumt']);

/// The extensions the footage filter offers.
///
/// Two lists in one: the containers FFmpeg reads, and every still-image format
/// `lumit_media::sequence::STILL_EXTENSIONS` names - a numbered run of any of
/// those imports as one sequence, so offering fewer here than the
/// engine reads is the dialogue hiding files that would have worked.
///
/// It is a convenience, not a gate. The dialogue also offers All files, the
/// engine refuses nothing by extension, and the probe is what decides whether a
/// file can be read - so a codec somebody has and this list has never heard of
/// still opens.
///
/// Public because the file picker is no longer the only road in: a folder
/// dropped on the Project panel is read with this same list, so what a
/// folder yields and what the dialogue offers cannot drift apart, Lottie
/// files aside.
const List<String> footageExtensions = [
  // Video containers
  'mp4', 'mov', 'mkv', 'avi', 'webm', 'm4v', 'mpg', 'mpeg', 'ts', 'mts', 'm2ts',
  'wmv', 'flv', '3gp', 'ogv', 'mxf', 'gif',
  // Stills, mirroring the engine's STILL_EXTENSIONS
  'png', 'jpg', 'jpeg', 'tif', 'tiff', 'exr', 'tga', 'targa', 'dpx', 'bmp',
  'webp', 'ppm', 'pgm', 'pnm', 'pbm',
  // FFmpeg reads HDR panoramas, and the browser has none.
  if (!kIsWeb) 'hdr',
  // Layered documents, which import as a composition of their layers
  'psd', 'ai',
  // Sound
  'wav', 'mp3', 'flac', 'aac', 'm4a', 'ogg', 'opus', 'aiff', 'aif', 'wma',
  // 3D models, and the ones the user's Blender converts
  'gltf', 'glb', 'fbx', 'obj', 'vdb', 'blend', 'usd', 'usdz', 'usdc', 'usda', 'abc',
  'ply', 'stl', 'dae',
];

/// What the footage picker offers: the footage, and Lottie files, which
/// import as a composition. A folder walk leaves these two out, so a `.json`
/// beside the footage stays where it is.
const List<String> _pickerExtensions = [...footageExtensions, 'json', 'lottie'];

/// The footage type group.
XTypeGroup _footageGroup() =>
    XTypeGroup(label: l10n.fileTypeFootage, extensions: _pickerExtensions);

/// Everything, offered under the footage group. A file whose extension nobody
/// listed - a container the machine's own codecs handle, a camera's odd suffix -
/// is still reachable, and the probe gives the honest answer about it.
XTypeGroup _anyGroup() => XTypeGroup(label: l10n.fileTypeAllFiles);

/// Pick one project file to open, or null when the dialogue was cancelled.
Future<String?> pickProjectToOpen() async {
  final file = await openFile(acceptedTypeGroups: [_projectGroup()]);
  return file == null ? null : keepPicked(file);
}

/// [path] as it is, or picked again where the browser build no longer has
/// it. Null when that was cancelled. The video and sound it uses are opened
/// again where the browser allows, and [app] offers to ask for the rest.
Future<String?> pickIfGone(String path, LumitState app) async {
  final picked = File(path).existsSync() ? path : await pickProjectToOpen();
  final asking = picked == null ? 0 : await reopenFootage(picked);
  if (asking > 0) {
    app.postNotice(l10n.webFilesAskAgain(asking),
        action: l10n.webAllowAccess, onTap: () => _allowFootage(app));
  }
  return picked;
}

/// Ask for the files [reopenFootage] couldn't open, and point the footage
/// that came back at itself again, so the engine reads it.
Future<void> _allowFootage(LumitState app) async {
  final back = (await allowFootage()).toSet();
  for (final footage in _footageIn(app.project?.getItems() ?? const [])) {
    try {
      final path = footage.filePath();
      if (back.contains(path)) footage.relink(path: path);
    } catch (_) {
      // Gone with its project. Nothing to point.
    }
  }
  app.notifyDocumentChanged();
}

Iterable<FootageReference> _footageIn(List<ItemReference> items) sync* {
  for (final item in items) {
    if (item case ItemReference_Footage(:final field0)) yield field0;
    if (item case ItemReference_Folder(:final field0)) {
      yield* _footageIn(field0.getChildren());
    }
  }
}

/// Choose where to save a project, defaulting the name to `untitled.lum` (as
/// the egui Save dialogue does), or null when cancelled.
Future<String?> pickProjectSaveLocation(
    {String suggestedName = 'untitled.lum', String? initialDirectory}) async {
  if (kIsWeb) return pickSaveTarget(suggestedName);
  final location = await getSaveLocation(
    acceptedTypeGroups: [_projectGroup()],
    suggestedName: suggestedName,
    initialDirectory: initialDirectory,
  );
  return location?.path;
}

/// Pick one or more footage files, or an empty list when cancelled. With
/// [projects] a Lumit project is offered too, which an import brings one
/// composition in from, and a graphics template, which joins the library.
Future<List<String>> pickFootage({bool projects = false}) async {
  final here = await pickFilesHere(l10n.fileTypeFootage,
      [..._pickerExtensions, if (projects) ...['lum', 'lumt']]);
  if (here != null) return here;
  final files = await openFiles(
    acceptedTypeGroups: [
      _footageGroup(),
      if (projects) _projectGroup(),
      if (projects) _templateGroup(),
      _anyGroup(),
    ],
  );
  return droppedPaths(files);
}

/// Where each of [files] is, picked or dropped, for the engine to open.
Future<List<String>> droppedPaths(List<XFile> files) async {
  final paths = <String>[];
  for (final file in files) {
    paths.add(await keepPicked(file));
  }
  return paths;
}

/// Choose where to save an exported video, defaulting the name to the
/// resolver's [suggestedName] (the egui exporter's `set_file_name`), or null
/// when cancelled. The generic save seam the export dialogue and the share
/// exports both drive. `extension`/`label` follow the chosen format:
/// `.mp4` for video, the image extension for a sequence — where the picked
/// name is the sequence's stem, and the frames land numbered beside it
/// ([numbered]), so the name itself is never written.
Future<String?> pickExportSaveLocation(
  String suggestedName, {
  String extension = 'mp4',
  String label = 'MP4 video',
  bool numbered = false,
  String? initialDirectory,
}) async {
  final location = await getSaveLocation(
    acceptedTypeGroups: [
      XTypeGroup(label: label, extensions: [extension])
    ],
    suggestedName: suggestedName,
    initialDirectory: initialDirectory,
  );
  final path = location?.path;
  if (path == null) return null;
  // The Windows dialogue hands back a typed name as it is, so the extension is
  // added here and the Destination row shows the file that will be written.
  if (path.toLowerCase().endsWith('.$extension')) return path;
  final whole = '$path.$extension';
  // The dialogue asked about the name as it was typed. If the name with its
  // extension is a file already, nobody has been asked about that one, so the
  // dialogue comes back with the whole name and asks.
  if (numbered || !File(whole).existsSync()) return whole;
  final file = File(whole);
  return pickExportSaveLocation(
    file.uri.pathSegments.last,
    extension: extension,
    label: label,
    initialDirectory: file.parent.path,
  );
}

XTypeGroup _easingsGroup() =>
    XTypeGroup(label: l10n.fileTypeEasings, extensions: const ['json']);

/// Where to write a flare design, or null when cancelled.
Future<String?> pickFlareSaveLocation(String suggestedName) async {
  final path = (await getSaveLocation(
    acceptedTypeGroups: [
      XTypeGroup(label: l10n.flareFileType, extensions: const ['flare']),
    ],
    suggestedName: '$suggestedName.flare',
  ))
      ?.path;
  if (path == null || path.toLowerCase().endsWith('.flare')) return path;
  return '$path.flare';
}

/// Where to write a frame as a picture, or null when cancelled.
Future<String?> pickFrameSaveLocation(String suggestedName) async {
  final path = (await getSaveLocation(
    acceptedTypeGroups: [
      XTypeGroup(label: 'PNG', extensions: const ['png']),
    ],
    suggestedName: '$suggestedName.png',
  ))
      ?.path;
  if (path == null || path.toLowerCase().endsWith('.png')) return path;
  return '$path.png';
}

/// Where to write a grade as a `.cube` LUT, or null when cancelled.
Future<String?> pickLutSaveLocation(String suggestedName) async {
  final path = (await getSaveLocation(
    acceptedTypeGroups: [
      XTypeGroup(label: l10n.fxLut, extensions: const ['cube']),
    ],
    suggestedName: '$suggestedName.cube',
  ))
      ?.path;
  if (path == null || path.toLowerCase().endsWith('.cube')) return path;
  return '$path.cube';
}

/// Pick a colour decision (`.cdl`, `.cc` or `.ccc`) to import, or null when
/// cancelled.
Future<String?> pickCdlToOpen() async => (await openFile(acceptedTypeGroups: [
      XTypeGroup(label: l10n.gradeCdl, extensions: const ['cdl', 'cc', 'ccc']),
    ]))
        ?.path;

/// Where to export a node as a colour decision, or null when cancelled. A
/// `.cdl` is written as a decision list, anything else as a `.cc`.
Future<String?> pickCdlSaveLocation(String suggestedName) async {
  final path = (await getSaveLocation(
    acceptedTypeGroups: [
      XTypeGroup(label: l10n.gradeCdl, extensions: const ['cc', 'cdl']),
    ],
    suggestedName: '$suggestedName.cc',
  ))
      ?.path;
  if (path == null) return null;
  final lower = path.toLowerCase();
  return lower.endsWith('.cc') || lower.endsWith('.cdl') ? path : '$path.cc';
}

/// Pick a file of easing shapes to import, or null when cancelled.
Future<String?> pickEasingsToOpen() async =>
    (await openFile(acceptedTypeGroups: [_easingsGroup()]))?.path;

/// Where to export the saved easing shapes, or null when cancelled.
Future<String?> pickEasingsSaveLocation() async {
  final path = (await getSaveLocation(
    acceptedTypeGroups: [_easingsGroup()],
    suggestedName: 'easings.json',
  ))
      ?.path;
  if (path == null || path.toLowerCase().endsWith('.json')) return path;
  return '$path.json';
}

/// The `.lumfx` effect-preset type group, mirroring the egui Effects panel's
/// preset filter (`crates/lumit-ui`'s `preset.rs`).
XTypeGroup _presetGroup() =>
    XTypeGroup(label: l10n.fileTypePreset, extensions: const ['lumfx']);

/// What a preset picker opens: Lumit's own `.lumfx`, and an After Effects
/// `.ffx`, which the engine converts as it reads it. One group, so both kinds
/// show without changing the dialogue's filter.
XTypeGroup _presetOpenGroup() => XTypeGroup(
    label: l10n.fileTypePresetToOpen, extensions: const ['lumfx', 'ffx']);

/// Pick one preset file to load, or null when cancelled.
Future<String?> pickPresetToOpen() async {
  final file = await openFile(acceptedTypeGroups: [_presetOpenGroup()]);
  return file?.path;
}

/// The WGSL type group — what `Load from file…` on a Custom shader offers
/// (docs/impl/custom-shader.md §1.1). A bare shader somebody sent you; the text
/// is **copied into the instance**, because a project must be one file that
/// opens on another machine.
XTypeGroup _shaderGroup() =>
    XTypeGroup(label: l10n.fileTypeShader, extensions: const ['wgsl']);

/// Pick a script to run, JavaScript or After Effects', or null when
/// cancelled.
Future<String?> pickScript() async {
  final file = await openFile(acceptedTypeGroups: [
    XTypeGroup(label: l10n.fileTypeScript, extensions: const ['js', 'jsx'])
  ]);
  return file?.path;
}

/// Pick a program, or null when cancelled.
Future<String?> pickProgram() async => (await openFile())?.path;

/// Pick a script to assemble a cut from: plain text or Final Draft. Null
/// when cancelled.
Future<String?> pickTranscriptScript() async {
  final file = await openFile(acceptedTypeGroups: [
    XTypeGroup(
        label: l10n.transcriptAssembleTitle,
        extensions: const ['txt', 'md', 'fdx']),
  ]);
  return file?.path;
}

/// Pick one `.wgsl` shader to load, or null when cancelled.
Future<String?> pickShaderToOpen() async {
  final file = await openFile(acceptedTypeGroups: [_shaderGroup()]);
  return file?.path;
}

/// Choose where to save a `.lumfx` preset, defaulting the name to
/// [suggestedName] and the folder to [initialDirectory] (the preset library,
/// so a plain save lands where the browser lists), or null when cancelled.
Future<String?> pickPresetSaveLocation(String suggestedName,
    {String? initialDirectory, bool material = false}) async {
  final location = await getSaveLocation(
    acceptedTypeGroups: [
      material
          ? XTypeGroup(
              label: l10n.fileTypeMaterialPreset, extensions: const ['lummat'])
          : _presetGroup()
    ],
    suggestedName: suggestedName,
    initialDirectory: initialDirectory,
  );
  return location?.path;
}

/// The OCIO configuration type group: the config file itself, which is
/// `config.ocio` by convention but need not be named that.
XTypeGroup _ocioGroup() =>
    XTypeGroup(label: l10n.fileTypeOcioConfig, extensions: const ['ocio']);

/// Pick a colour config for the project, or null when cancelled.
Future<String?> pickOcioConfig() async {
  final file = await openFile(acceptedTypeGroups: [_ocioGroup()]);
  return file?.path;
}

/// The keymap type group (docs/07 §15's shareable file). Plain JSON, so
/// a `.json` a user has renamed still opens.
XTypeGroup _keymapGroup() =>
    XTypeGroup(label: l10n.fileTypeKeymap, extensions: const ['json']);

/// Pick a keymap file to import, or null when the dialogue was cancelled.
Future<String?> pickKeymapToOpen() async {
  final file = await openFile(acceptedTypeGroups: [_keymapGroup()]);
  return file?.path;
}

/// After Effects' shortcut files, which are plain `.txt`.
XTypeGroup _afterEffectsShortcutsGroup() => XTypeGroup(
    label: l10n.fileTypeAfterEffectsShortcuts, extensions: const ['txt']);

/// Where After Effects keeps its settings on this machine, or null when it
/// isn't there. The shortcut files sit in an `aeks` folder under each version
/// in here, which is not a place anyone finds by browsing.
Future<String?> _afterEffectsSettingsFolder() async {
  final env = Platform.environment;
  final path = Platform.isWindows
      ? '${env['APPDATA']}\\Adobe\\After Effects'
      : '${env['HOME']}/Library/Preferences/Adobe/After Effects';
  return await Directory(path).exists() ? path : null;
}

/// Pick an After Effects shortcut file to read, or null when cancelled.
Future<String?> pickAfterEffectsShortcuts() async {
  final file = await openFile(
    acceptedTypeGroups: [_afterEffectsShortcutsGroup()],
    initialDirectory: await _afterEffectsSettingsFolder(),
  );
  return file?.path;
}

/// The shared-theme type group, the theme's counterpart of the
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

/// The shared-workspace type group (docs/07 §1.4), the arrangement's
/// counterpart of the theme's.
XTypeGroup _workspaceGroup() => XTypeGroup(
    label: l10n.fileTypeWorkspace, extensions: const [workspaceFileExtension]);

/// Pick a workspace file to import, or null when the dialogue was cancelled.
Future<String?> pickWorkspaceToOpen() async {
  final file = await openFile(acceptedTypeGroups: [_workspaceGroup()]);
  return file?.path;
}

/// Choose where to write a workspace, defaulting the name to its own, or null
/// when cancelled.
Future<String?> pickWorkspaceSaveLocation(String suggestedName) async {
  final location = await getSaveLocation(
    acceptedTypeGroups: [_workspaceGroup()],
    suggestedName: suggestedName,
  );
  return location?.path;
}

/// The saved expressions type group.
XTypeGroup _expressionsGroup() => XTypeGroup(
    label: l10n.fileTypeExpressions, extensions: const ['lumexpressions']);

/// Pick an expressions file to import, or null when the dialogue was cancelled.
Future<String?> pickExpressionsToOpen() async {
  final file = await openFile(acceptedTypeGroups: [_expressionsGroup()]);
  return file?.path;
}

/// Choose where to write the saved expressions, or null when cancelled.
Future<String?> pickExpressionsSaveLocation() async {
  final location = await getSaveLocation(
    acceptedTypeGroups: [_expressionsGroup()],
    suggestedName: 'expressions.lumexpressions',
  );
  final path = location?.path;
  if (path == null) return null;
  // The Windows dialogue hands back a typed name as it is, and Import only
  // lists files that end this way.
  return path.toLowerCase().endsWith('.lumexpressions')
      ? path
      : '$path.lumexpressions';
}

/// The After Effects import's own type group: the project file itself,
/// and the zip a Bridge bundle travels in.
XTypeGroup _aeGroup() => XTypeGroup(
      label: l10n.fileTypeAeProject,
      extensions: const ['aep', 'zip'],
    );

/// Pick an After Effects project to import — the `.aep` itself, or a
/// zipped Lumit Bridge bundle. Which one it is is the engine's to decide from
/// the bytes; this dialogue only has to offer both. Null when cancelled.
Future<String?> pickAeProject() async {
  final file = await openFile(acceptedTypeGroups: [_aeGroup()]);
  return file?.path;
}

/// Pick a Lumit Bridge bundle **folder** (docs/11 §2.1), because a folder is
/// what the Bridge's script writes: ExtendScript has no zip. The quieter of the
/// two import routes — a zipped bundle goes through [pickAeProject]. Null when
/// cancelled.
Future<String?> pickAeBundle() =>
    getDirectoryPath(confirmButtonText: l10n.chooseConfirm);

/// Pick a folder — Settings → Performance's cache location, where the disk tier
/// parks its frames (docs/07 §15), or one to import. Null when the dialogue
/// was cancelled.
Future<String?> pickFolder() =>
    getDirectoryPath(confirmButtonText: l10n.chooseConfirm);

/// Pick a CSV, TSV or JSON file of rows for a graphics template's data, or
/// null when cancelled.
Future<String?> pickTemplateData() async =>
    (await openFile(acceptedTypeGroups: [
      XTypeGroup(
          label: l10n.templateDataFileType,
          extensions: const ['csv', 'tsv', 'tab', 'json']),
    ]))
        ?.path;

/// Where to write a starting CSV for a graphics template's data, or null when
/// cancelled.
Future<String?> pickTemplateDataSaveLocation(String suggestedName) async {
  final path = (await getSaveLocation(
    acceptedTypeGroups: [
      XTypeGroup(label: l10n.templateDataFileType, extensions: const ['csv']),
    ],
    suggestedName: '$suggestedName.csv',
  ))
      ?.path;
  if (path == null || path.toLowerCase().endsWith('.csv')) return path;
  return '$path.csv';
}

/// An addon's own description file, the `addon.json` that names what the pack
/// is and which files belong to it (docs/impl/addons.md §4).
XTypeGroup _addonGroup() =>
    XTypeGroup(label: l10n.fileTypeAddon, extensions: const ['json']);

/// Pick the `addon.json` of a pack to install from disk, with the files it
/// names sitting beside it. Null when cancelled.
Future<String?> pickAddonManifest() async {
  final file = await openFile(acceptedTypeGroups: [_addonGroup()]);
  return file?.path;
}

/// A style, as a file or its folder packed into one. A zip is offered too,
/// since that is how a folder is often sent.
XTypeGroup _styleGroup() => XTypeGroup(
    label: l10n.fileTypeStyle, extensions: const [styleFileExtension, 'zip']);

/// Pick a style to take in, or null when cancelled.
Future<String?> pickStyleToOpen() async {
  final file = await openFile(acceptedTypeGroups: [_styleGroup()]);
  return file?.path;
}

/// Choose where to save a style's package, named [suggestedName] to begin
/// with, or null when cancelled.
Future<String?> pickStylePackageSaveLocation(String suggestedName) async {
  final location = await getSaveLocation(
    acceptedTypeGroups: [
      XTypeGroup(
          label: l10n.fileTypeStyle, extensions: const [styleFileExtension]),
    ],
    suggestedName: suggestedName,
  );
  final path = location?.path;
  if (path == null) return null;
  // The Windows dialogue hands back a typed name as it is.
  return path.toLowerCase().endsWith('.$styleFileExtension')
      ? path
      : '$path.$styleFileExtension';
}

/// An extension packed as one file.
XTypeGroup _extensionPackageGroup() => XTypeGroup(
    label: l10n.fileTypeExtensionPackage, extensions: const ['lfx']);

/// Pick a `.lfx` package to install, or null when cancelled.
Future<String?> pickExtensionPackage() async {
  final file = await openFile(acceptedTypeGroups: [_extensionPackageGroup()]);
  return file?.path;
}

/// Choose where to save a package, named [suggestedName] to begin with, or
/// null when cancelled.
Future<String?> pickExtensionPackageSaveLocation(String suggestedName) async {
  final location = await getSaveLocation(
    acceptedTypeGroups: [_extensionPackageGroup()],
    suggestedName: suggestedName,
  );
  final path = location?.path;
  if (path == null) return null;
  // The Windows dialogue hands back a typed name as it is.
  return path.toLowerCase().endsWith('.lfx') ? path : '$path.lfx';
}

/// Choose where to write a keymap, or null when cancelled.
Future<String?> pickKeymapSaveLocation() async {
  final location = await getSaveLocation(
    acceptedTypeGroups: [_keymapGroup()],
    suggestedName: 'lumit-keymap.json',
  );
  return location?.path;
}

/// Pick one file for an effect's File parameter (docs/08 §1.2's File kind) —
/// the LUT's `.cube`, the Lens flare's `.lens`. The schema's own lower-case
/// extensions and label drive the dialogue's filter, so a new File parameter
/// needs no new function here. Null when cancelled.
Future<String?> pickEffectInputFile(
  List<String> extensions,
  String label,
) async {
  final file = await openFile(
    acceptedTypeGroups: [XTypeGroup(label: label, extensions: extensions)],
  );
  return file?.path;
}

/// The subtitle files File ▸ Import ▸ Captions… reads.
XTypeGroup _captionsGroup() => XTypeGroup(
      label: l10n.fileTypeCaptions,
      extensions: const [
        'srt',
        'vtt',
        'ttml',
        'dfxp',
        'xml',
        'ass',
        'ssa',
        'lrc'
      ],
    );

/// Pick a subtitle file to make a caption layer from, or null when cancelled.
Future<String?> pickCaptionsToOpen() async {
  final file = await openFile(acceptedTypeGroups: [_captionsGroup()]);
  return file == null ? null : keepPicked(file);
}

/// Where to write captions as a file ending [extension], or null when
/// cancelled. In the browser build the file is then given [handOver].
Future<String?> pickCaptionsSaveLocation(
    String suggestedName, String extension) async {
  if (kIsWeb) return pickSaveTarget('$suggestedName.$extension');
  final path = (await getSaveLocation(
    acceptedTypeGroups: [
      XTypeGroup(label: l10n.fileTypeCaptions, extensions: [extension]),
    ],
    suggestedName: '$suggestedName.$extension',
  ))
      ?.path;
  if (path == null || path.toLowerCase().endsWith('.$extension')) return path;
  return '$path.$extension';
}
