// Files, for the browser build.
//
// A page has no file system, and `dart:io` there only throws. So the browser
// build gives `File` and `Directory` one of its own, through `IOOverrides`,
// and every part of Lumit that reads or writes a file works unchanged. The
// files are the engine's, kept in memory (`lumit_ingress::files`), so a file
// picked in the page is one the engine can open and a project it saved is one
// the page can hand over. The ones worth keeping outlast the page in the
// browser's own storage (web/files.js).

import 'dart:async';
import 'dart:convert' show Encoding, LineSplitter, utf8;
import 'dart:io' hide Platform;
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:typed_data';

import 'package:file_selector/file_selector.dart' show XFile;
import 'package:lumit_flutter/src/rust/api/web_files.dart';
import 'package:web/web.dart' as web;
import 'package:lumit_flutter/state/web_media_web.dart';

@JS('lumitFiles.restore')
external JSPromise<JSAny?> _restore();

@JS('lumitFiles.used')
external JSPromise<JSNumber?> _used();

@JS('lumitFiles.pick')
external JSPromise<JSArray<web.File>?> _pick(
    JSString description, JSArray<JSString> extensions);

@JS('lumitFiles.reopen')
external JSPromise<JSNumber> _reopen(JSString project);

@JS('lumitFiles.allow')
external JSPromise<JSArray<JSString>> _allow();

@JS('lumitFiles.forget')
external JSPromise<JSAny?> _forget(JSArray<JSString> projects);

/// Put the browser's files in place of `dart:io`'s, before anything reads one.
/// The engine's store is what they live in, so this comes after the bridge.
Future<void> installFiles() async {
  await _restore().toDart;
  IOOverrides.global = _WebOverrides();
}

/// How much this browser keeps for Lumit on this computer, in bytes.
Future<int?> storageUsed() async => (await _used().toDart)?.toDartDouble.round();

/// Forget the files opened here that none of [projects] uses, and the caches.
Future<void> forgetUnused(List<String> projects) =>
    _forget([for (final p in projects) p.toJS].toJS).toDart;

/// Open again the video and sound [project] uses where the browser still
/// lets the page read it. How many files it will only give back once the
/// person allows it ([allowFootage]).
Future<int> reopenFootage(String project) async {
  try {
    return (await _reopen(project.toJS).toDart).toDartInt;
  } catch (_) {
    return 0;
  }
}

/// Ask for those files, on a click. The paths that came back.
Future<List<String>> allowFootage() async =>
    [for (final path in (await _allow().toDart).toDart) path.toDart];

/// Have the browser ask before the page goes, while [unsaved] says so.
void warnBeforeLeaving(bool Function() unsaved) {
  web.window.addEventListener(
      'beforeunload',
      (web.Event event) {
        if (unsaved()) event.preventDefault();
      }.toJS);
}

/// How many files the page has been handed, which keeps their folders apart.
int _picked = 0;

/// A folder of its own for the next file the page is handed.
String _newFolder() =>
    '/web/${DateTime.now().millisecondsSinceEpoch.toRadixString(36)}${_picked++}';

/// Put a file picked or dropped in the page where the engine can open it, and
/// say where that is: `/web/<id>/<its name>`.
///
/// Video and sound stay where the browser has them and are read by its own
/// decoders (web_media_web.dart), so they are never copied in here. An empty
/// answer is a file this browser can't decode.
Future<String> keepPicked(XFile file) async {
  if (_media.contains(file.name.split('.').last.toLowerCase())) {
    final response = await web.window.fetch(file.path.toJS).toDart;
    final blob = await response.blob().toDart;
    return await openWebMedia(web.File([blob].toJS, file.name)) ?? '';
  }
  final path = '${_newFolder()}/${file.name}';
  _put(path, await file.readAsBytes());
  return path;
}

/// Pick [label] files with the browser's own picker, which lets a later page
/// open them again, and put each where [keepPicked] would. Null where the
/// browser has no such picker.
Future<List<String>?> pickFilesHere(
    String label, List<String> extensions) async {
  final files =
      await _pick(label.toJS, [for (final e in extensions) e.toJS].toJS).toDart;
  if (files == null) return null;
  final paths = <String>[];
  for (final file in files.toDart) {
    if (_media.contains(file.name.split('.').last.toLowerCase())) {
      paths.add(await openWebMedia(file) ?? '');
    } else {
      paths.add('${_newFolder()}/${file.name}');
      _put(paths.last, (await file.arrayBuffer().toDart).toDart.asUint8List());
    }
  }
  return paths;
}

/// The files the person chose to save into, by where they are kept here.
final Map<String, web.FileSystemFileHandle> _chosen = {};

@JS('showSaveFilePicker')
external JSPromise<web.FileSystemFileHandle> _showSaveFilePicker(JSAny options);

/// Where to save a project: a file the person chooses, where the browser can
/// ask, and otherwise a name that is downloaded when it is saved. Null when
/// they cancelled.
Future<String?> pickSaveTarget(String suggestedName) async {
  final at = _newFolder();
  if (!globalContext.has('showSaveFilePicker')) return '$at/$suggestedName';
  try {
    final options = {'suggestedName': suggestedName}.jsify()!;
    final handle = await _showSaveFilePicker(options).toDart;
    _chosen['$at/${handle.name}'] = handle;
    return '$at/${handle.name}';
  } catch (e) {
    // Cancelled, or not allowed here, which a download still is.
    return '$e'.contains('AbortError') ? null : '$at/$suggestedName';
  }
}

/// Hand a saved file to the person: into the file they chose for it, or as a
/// download.
Future<void> handOver(String path) async {
  final bytes = _bytesOf(path);
  final chosen = _chosen[_clean(path)];
  if (chosen != null) {
    final out = await chosen.createWritable().toDart;
    await out.write(bytes.toJS).toDart;
    await out.close().toDart;
    return;
  }
  _download(bytes, _clean(path).split('/').last);
}

void _download(Uint8List bytes, String name) {
  final url = web.URL.createObjectURL(web.Blob([bytes.toJS].toJS));
  web.HTMLAnchorElement()
    ..href = url
    ..download = name
    ..click();
  // Let go once the browser has surely taken it. At once, it can cancel.
  Timer(const Duration(minutes: 1), () => web.URL.revokeObjectURL(url));
}

/// What goes to the browser's own decoders rather than into the files here.
const Set<String> _media = {
  'mp4', 'mov', 'mkv', 'avi', 'webm', 'm4v', 'mpg', 'mpeg', 'ts', 'mts', //
  'm2ts', 'wmv', 'flv', '3gp', 'ogv', 'mxf', 'wav', 'mp3', 'flac', 'aac',
  'm4a', 'ogg', 'opus', 'aiff', 'aif', 'wma',
};

/// A picked file the browser could open. [keepPicked] answers an empty path
/// for one it couldn't.
bool importsHere(String path) => path.isNotEmpty;

String _clean(String path) {
  var p = path.replaceAll('\\', '/');
  if (!p.startsWith('/')) p = '/$p';
  final parts = <String>[];
  for (final part in p.split('/')) {
    if (part.isEmpty || part == '.') continue;
    if (part == '..') {
      if (parts.isNotEmpty) parts.removeLast();
    } else {
      parts.add(part);
    }
  }
  return '/${parts.join('/')}';
}

String _parentOf(String path) {
  final p = _clean(path);
  final at = p.lastIndexOf('/');
  return at <= 0 ? '/' : p.substring(0, at);
}

void _put(String path, List<int> bytes) =>
    webFileWrite(path: _clean(path), bytes: bytes);

void _drop(String path) => webFileRemove(path: _clean(path));

FileSystemEntityType _typeOf(String path) {
  final stat = webFileStat(path: _clean(path));
  if (stat == null) return FileSystemEntityType.notFound;
  return stat.folder
      ? FileSystemEntityType.directory
      : FileSystemEntityType.file;
}

DateTime _changedOf(String path) {
  final ms = webFileStat(path: _clean(path))?.changedMs;
  return ms == null || ms == 0
      ? DateTime.now()
      : DateTime.fromMillisecondsSinceEpoch(ms.round());
}

Uint8List _bytesOf(String path) {
  final bytes = webFileRead(path: _clean(path));
  if (bytes == null) {
    throw PathNotFoundException(path, const OSError('No such file', 2));
  }
  return bytes;
}

/// Every file and folder inside [dir], at any depth when [recursive].
List<FileSystemEntity> _inside(String dir, bool recursive) {
  final out = <FileSystemEntity>[];
  for (final name in webFileList(path: dir)) {
    final path = dir == '/' ? '/$name' : '$dir/$name';
    if (!name.endsWith('/')) {
      out.add(_WebFile(path));
      continue;
    }
    final folder = path.substring(0, path.length - 1);
    out.add(_WebDirectory(folder));
    if (recursive) out.addAll(_inside(folder, true));
  }
  return out;
}

final class _WebOverrides extends IOOverrides {
  @override
  File createFile(String path) => _WebFile(path);

  @override
  Directory createDirectory(String path) => _WebDirectory(path);

  @override
  Directory getSystemTempDirectory() => _WebDirectory('/tmp');

  @override
  Directory getCurrentDirectory() => _WebDirectory('/');

  @override
  Future<FileSystemEntityType> fseGetType(String path, bool followLinks) async =>
      _typeOf(path);

  @override
  FileSystemEntityType fseGetTypeSync(String path, bool followLinks) =>
      _typeOf(path);

  @override
  Future<bool> fseIdentical(String path1, String path2) async =>
      _clean(path1) == _clean(path2);

  @override
  bool fseIdenticalSync(String path1, String path2) =>
      _clean(path1) == _clean(path2);

  @override
  Future<FileStat> stat(String path) async => statSync(path);

  @override
  FileStat statSync(String path) => _WebStat(path);
}

abstract class _WebEntity implements FileSystemEntity {
  _WebEntity(String path) : path = _clean(path);

  @override
  final String path;

  @override
  Uri get uri => Uri.file(path);

  @override
  bool get isAbsolute => true;

  @override
  Directory get parent => _WebDirectory(_parentOf(path));

  @override
  Future<String> resolveSymbolicLinks() async => path;

  @override
  String resolveSymbolicLinksSync() => path;

  @override
  Stream<FileSystemEvent> watch(
          {int events = FileSystemEvent.all, bool recursive = false}) =>
      const Stream.empty();

  @override
  String toString() => "$runtimeType: '$path'";

  // Anything else a page has no answer for.
  @override
  dynamic noSuchMethod(Invocation invocation) =>
      throw UnsupportedError('Not in the browser: ${invocation.memberName}');
}

class _WebFile extends _WebEntity implements File {
  _WebFile(super.path);

  @override
  File get absolute => this;

  @override
  Future<bool> exists() async => existsSync();

  @override
  bool existsSync() => _typeOf(path) == FileSystemEntityType.file;

  @override
  Future<File> create({bool recursive = false, bool exclusive = false}) async {
    createSync(recursive: recursive, exclusive: exclusive);
    return this;
  }

  @override
  void createSync({bool recursive = false, bool exclusive = false}) {
    if (!existsSync()) _put(path, Uint8List(0));
  }

  @override
  Future<FileSystemEntity> delete({bool recursive = false}) async {
    deleteSync();
    return this;
  }

  @override
  void deleteSync({bool recursive = false}) => _drop(path);

  @override
  Future<File> rename(String newPath) async => renameSync(newPath);

  @override
  File renameSync(String newPath) {
    final bytes = _bytesOf(path);
    _drop(path);
    _put(newPath, bytes);
    return _WebFile(newPath);
  }

  @override
  Future<File> copy(String newPath) async => copySync(newPath);

  @override
  File copySync(String newPath) {
    _put(newPath, Uint8List.fromList(_bytesOf(path)));
    return _WebFile(newPath);
  }

  @override
  Future<int> length() async => lengthSync();

  @override
  int lengthSync() => _bytesOf(path).length;

  @override
  Future<DateTime> lastModified() async => lastModifiedSync();

  @override
  DateTime lastModifiedSync() {
    _bytesOf(path);
    return _changedOf(path);
  }

  @override
  Future<FileStat> stat() async => statSync();

  @override
  FileStat statSync() => _WebStat(path);

  @override
  Future<Uint8List> readAsBytes() async => readAsBytesSync();

  @override
  Uint8List readAsBytesSync() => _bytesOf(path);

  @override
  Future<String> readAsString({Encoding encoding = utf8}) async =>
      readAsStringSync(encoding: encoding);

  @override
  String readAsStringSync({Encoding encoding = utf8}) =>
      encoding.decode(_bytesOf(path));

  @override
  Future<List<String>> readAsLines({Encoding encoding = utf8}) async =>
      readAsLinesSync(encoding: encoding);

  @override
  List<String> readAsLinesSync({Encoding encoding = utf8}) =>
      const LineSplitter().convert(readAsStringSync(encoding: encoding));

  @override
  Future<File> writeAsBytes(List<int> bytes,
      {FileMode mode = FileMode.write, bool flush = false}) async {
    writeAsBytesSync(bytes, mode: mode, flush: flush);
    return this;
  }

  @override
  void writeAsBytesSync(List<int> bytes,
      {FileMode mode = FileMode.write, bool flush = false}) {
    final appending = mode == FileMode.append || mode == FileMode.writeOnlyAppend;
    if (appending && existsSync()) {
      _put(path, [..._bytesOf(path), ...bytes]);
    } else {
      _put(path, bytes);
    }
  }

  @override
  Future<File> writeAsString(String contents,
      {FileMode mode = FileMode.write,
      Encoding encoding = utf8,
      bool flush = false}) async {
    writeAsStringSync(contents, mode: mode, encoding: encoding, flush: flush);
    return this;
  }

  @override
  void writeAsStringSync(String contents,
          {FileMode mode = FileMode.write,
          Encoding encoding = utf8,
          bool flush = false}) =>
      writeAsBytesSync(encoding.encode(contents), mode: mode, flush: flush);

  @override
  Stream<List<int>> openRead([int? start, int? end]) {
    final bytes = _bytesOf(path);
    return Stream.value(
        Uint8List.sublistView(bytes, start ?? 0, end ?? bytes.length));
  }
}

class _WebDirectory extends _WebEntity implements Directory {
  _WebDirectory(super.path);

  @override
  Directory get absolute => this;

  @override
  Future<bool> exists() async => existsSync();

  @override
  bool existsSync() => _typeOf(path) == FileSystemEntityType.directory;

  @override
  Future<Directory> create({bool recursive = false}) async {
    createSync(recursive: recursive);
    return this;
  }

  @override
  void createSync({bool recursive = false}) => webFileMakeFolder(path: path);

  int _made = 0;

  @override
  Future<Directory> createTemp([String? prefix]) async => createTempSync(prefix);

  @override
  Directory createTempSync([String? prefix]) {
    final made =
        _WebDirectory('$path/${prefix ?? ''}${DateTime.now().microsecondsSinceEpoch}${_made++}');
    made.createSync();
    return made;
  }

  @override
  Future<FileSystemEntity> delete({bool recursive = false}) async {
    deleteSync(recursive: recursive);
    return this;
  }

  @override
  void deleteSync({bool recursive = false}) {
    for (final file in _inside(path, true).whereType<File>()) {
      _drop(file.path);
    }
    webFileRemove(path: path);
  }

  @override
  Future<Directory> rename(String newPath) async => renameSync(newPath);

  @override
  Directory renameSync(String newPath) {
    final to = _clean(newPath);
    webFileMakeFolder(path: to);
    for (final file in _inside(path, true).whereType<File>()) {
      final bytes = _bytesOf(file.path);
      _drop(file.path);
      _put('$to${file.path.substring(path.length)}', bytes);
    }
    webFileRemove(path: path);
    return _WebDirectory(to);
  }

  @override
  Future<FileStat> stat() async => statSync();

  @override
  FileStat statSync() => _WebStat(path);

  @override
  Stream<FileSystemEntity> list(
          {bool recursive = false, bool followLinks = true}) =>
      Stream.fromIterable(
          listSync(recursive: recursive, followLinks: followLinks));

  @override
  List<FileSystemEntity> listSync(
          {bool recursive = false, bool followLinks = true}) =>
      _inside(path, recursive);
}

class _WebStat implements FileStat {
  _WebStat(String path)
      : type = _typeOf(path),
        size = webFileStat(path: _clean(path))?.size ?? -1,
        modified = _changedOf(path);

  @override
  final FileSystemEntityType type;

  @override
  final int size;

  @override
  final DateTime modified;

  @override
  DateTime get changed => modified;

  @override
  DateTime get accessed => modified;

  @override
  int get mode => 0x1b6;

  @override
  String modeString() => 'rw-rw-rw-';
}
