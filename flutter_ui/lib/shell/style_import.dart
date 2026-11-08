// Taking in a style: a `.lumstyle` file, or a style's folder zipped into one,
// from Import…, a double-click or a drop on the window. It goes into the
// styles folder and is in use at once, with the look it replaced one click
// away on the status line. A style only dresses, so nothing is asked first
// unless it would replace one of the same name.

import 'dart:io' hide Platform;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/state/platform.dart'
    if (dart.library.js_interop) 'package:lumit_flutter/state/platform_web.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/styles.dart';

import '../l10n/strings.dart';
import '../state/file_dialogs.dart';
import '../state/workspace.dart';
import '../theme/style_sheet.dart';
import 'settings_account.dart' show askBefore;

/// Whether the file at [path] is a zip: a style's folder packed into one.
bool _zipped(String path) {
  try {
    final head = File(path).openSync()..setPositionSync(0);
    final bytes = head.readSync(4);
    head.closeSync();
    return bytes.length == 4 &&
        bytes[0] == 0x50 &&
        bytes[1] == 0x4b &&
        bytes[2] == 0x03 &&
        bytes[3] == 0x04;
  } on FileSystemException {
    return false;
  }
}

String _fileName(String path) => path.split(RegExp(r'[/\\]')).last;

/// Why a style package couldn't be taken in, as a sentence.
String _why(BridgeStyleOutcome outcome, String path) => switch (outcome) {
      BridgeStyleOutcome_Ready() => '',
      BridgeStyleOutcome_NotAStyle() => l10n.styleNotOne(_fileName(path)),
      BridgeStyleOutcome_TooLarge() => l10n.styleTooLarge(_fileName(path)),
      BridgeStyleOutcome_Unsafe() => l10n.styleUnsafe(_fileName(path)),
      BridgeStyleOutcome_Failed() => l10n.styleFailed(_fileName(path)),
    };

/// Pick a style and take it in.
Future<void> importStyleFromDisk(
    BuildContext context, LumitState app, LumitUiState ui) async {
  final path = await pickStyleToOpen();
  if (path == null || !context.mounted) return;
  await importStyle(context, app, ui, path);
}

/// Take in the style at [path] and use it. One already in the styles folder
/// is only used.
Future<void> importStyle(BuildContext context, LumitState app,
    LumitUiState ui, String path) async {
  final workspace = ui.workspace;
  final folder = Workspace.stylesFolder();
  final sep = Platform.pathSeparator;
  final String name;

  /// Unpack a package into the styles folder. Whether it went in.
  Future<bool> install(String package, String shown) async {
    final done = await styleInstallPackage(path: package, styles: folder);
    if (done is! BridgeStyleOutcome_Ready) {
      app.postNotice(_why(done, shown), error: true);
      return false;
    }
    // A loose file of the same name would be a second style called that.
    try {
      File('$folder$sep${done.name}.$styleFileExtension').deleteSync();
    } on FileSystemException {
      // There was none.
    }
    return true;
  }

  if (_zipped(path)) {
    final read = styleReadPackage(path: path);
    if (read is! BridgeStyleOutcome_Ready) {
      return app.postNotice(_why(read, path), error: true);
    }
    name = read.name;
    if (!await _mayReplace(context, workspace, name)) return;
    if (!await install(path, path)) return;
  } else {
    final file = _fileName(path);
    if (!file.toLowerCase().endsWith('.$styleFileExtension')) {
      return app.postNotice(l10n.styleNotOne(file), error: true);
    }
    name = file.substring(0, file.length - styleFileExtension.length - 1);
    final inFolder = File(path).absolute.path.toLowerCase().startsWith(
        '${Directory(folder).absolute.path.toLowerCase()}$sep');
    final home = File(path).parent.path;
    if (inFolder) {
      // Already here: it is only used.
    } else if (_fileName(home).toLowerCase() == name.toLowerCase()) {
      // In a folder of its own, which comes with its pictures, icons and
      // fonts, packed and unpacked as a package would be.
      if (!await _mayReplace(context, workspace, name)) return;
      final packed = '${Directory.systemTemp.path}${sep}lumit-style-$pid.zip';
      final made = await styleMakePackage(
          style: path, wholeFolder: true, out: packed);
      var ok = false;
      if (made is BridgeStyleOutcome_Ready) {
        ok = await install(packed, path);
      } else {
        app.postNotice(_why(made, path), error: true);
      }
      try {
        File(packed).deleteSync();
      } on FileSystemException {
        // Never made.
      }
      if (!ok) return;
    } else {
      final String text;
      try {
        text = await File(path).readAsString();
      } on FileSystemException {
        return app.postNotice(l10n.styleNotOne(file), error: true);
      }
      final same = _named(workspace, name);
      if (!context.mounted) return;
      if (same?.source != text &&
          !await _mayReplace(context, workspace, name)) {
        return;
      }
      // Over the one of that name where it is, a folder of its own or not.
      final to = same == null
          ? '$folder$sep$file'
          : '${same.folder}$sep${same.name}.$styleFileExtension';
      try {
        Directory(folder).createSync(recursive: true);
        await File(to).writeAsString(text);
      } on FileSystemException {
        return app.postNotice(l10n.styleFailed(file), error: true);
      }
    }
  }
  if (!context.mounted) return;
  final before = (workspace.customStyle, workspace.themeShape);
  workspace.setCustomStyle(_named(workspace, name)?.name ?? name);
  // A picture of the same name as the one replaced is read again.
  if (workspace.activeStyle?.roomPicture case final picture?) {
    FileImage(File(picture)).evict();
  }
  app.postNotice(l10n.styleInUse(name), action: l10n.styleUndo, onTap: () {
    if (before.$1 case final style?) {
      workspace.setCustomStyle(style);
    } else {
      workspace.setShape(before.$2);
    }
  });
}

/// The style called [name], however it is cased, as the disk on Windows and
/// macOS sees one name.
StyleSheet? _named(Workspace workspace, String name) => workspace.styles
    .where((s) => s.name.toLowerCase() == name.toLowerCase())
    .firstOrNull;

/// True when no style is called [name], or the person says to replace it.
Future<bool> _mayReplace(
    BuildContext context, Workspace workspace, String name) async {
  if (_named(workspace, name) == null) return true;
  if (!context.mounted) return false;
  return askBefore(context, l10n.styleReplace(name), l10n.styleReplaceYes);
}

/// Pack the style in use into one file to share, as its folder when it has
/// one of its own.
Future<void> makeStylePackage(LumitState app, LumitUiState ui) async {
  final style = ui.workspace.activeStyle;
  if (style == null) return;
  final out = await pickStylePackageSaveLocation('${style.name}.lumstyle');
  if (out == null) return;
  final whole = Directory(style.folder).absolute.path !=
      Directory(Workspace.stylesFolder()).absolute.path;
  final file = '${style.folder}${Platform.pathSeparator}'
      '${style.name}.$styleFileExtension';
  if (File(out).absolute.path.toLowerCase() ==
      File(file).absolute.path.toLowerCase()) {
    return app.postNotice(l10n.stylePackageOverItself, error: true);
  }
  final done =
      await styleMakePackage(style: file, wholeFolder: whole, out: out);
  if (done is BridgeStyleOutcome_Ready) {
    app.postNotice(l10n.stylePackaged(_fileName(out)));
  } else {
    app.postNotice(_why(done, out), error: true);
  }
}
