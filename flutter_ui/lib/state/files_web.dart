// Files, for the browser build.
//
// A page has no file system, and `dart:io` there only throws. So the browser
// build gives `File` and `Directory` one of its own, through `IOOverrides`,
// and every part of Lumit that reads or writes a file works unchanged. Files
// live in memory. Those under the settings folder are also kept in the
// browser's local storage, so the settings outlast the page.

import 'dart:async';
import 'dart:convert';
import 'dart:io' hide Platform;
import 'dart:typed_data';

import 'package:web/web.dart' as web;

/// Where the settings are kept (`HOME` in platform_web.dart, then `.config`).
const String _kept = '/home/.config/';

/// The prefix every kept file has in local storage.
const String _key = 'lumit-file:';

final Map<String, Uint8List> _files = {};
final Map<String, DateTime> _changed = {};
final Set<String> _dirs = {'/'};

/// Put the browser's files in place of `dart:io`'s, before anything reads one.
Future<void> installFiles() async {
  final storage = web.window.localStorage;
  for (var i = 0; i < storage.length; i++) {
    final key = storage.key(i);
    if (key == null || !key.startsWith(_key)) continue;
    final path = key.substring(_key.length);
    final value = storage.getItem(key);
    if (value == null) continue;
    try {
      _put(path, base64Decode(value), keep: false);
    } on FormatException {
      storage.removeItem(key);
    }
  }
  IOOverrides.global = _WebOverrides();
}

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

void _makeDirs(String path) {
  for (var p = _clean(path); !_dirs.contains(p); p = _parentOf(p)) {
    _dirs.add(p);
  }
}

void _put(String path, List<int> bytes, {bool keep = true}) {
  final p = _clean(path);
  _makeDirs(_parentOf(p));
  _files[p] = bytes is Uint8List ? bytes : Uint8List.fromList(bytes);
  _changed[p] = DateTime.now();
  if (keep && p.startsWith(_kept)) {
    try {
      web.window.localStorage.setItem('$_key$p', base64Encode(_files[p]!));
    } catch (_) {
      // Full. The file is still here for as long as the page is open.
    }
  }
}

void _drop(String path) {
  final p = _clean(path);
  _files.remove(p);
  _changed.remove(p);
  if (p.startsWith(_kept)) web.window.localStorage.removeItem('$_key$p');
}

FileSystemEntityType _typeOf(String path) {
  final p = _clean(path);
  if (_files.containsKey(p)) return FileSystemEntityType.file;
  if (_dirs.contains(p)) return FileSystemEntityType.directory;
  return FileSystemEntityType.notFound;
}

Uint8List _bytesOf(String path) {
  final bytes = _files[_clean(path)];
  if (bytes == null) {
    throw PathNotFoundException(path, const OSError('No such file', 2));
  }
  return bytes;
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
  bool existsSync() => _files.containsKey(path);

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
    return _changed[path] ?? DateTime.now();
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
  bool existsSync() => _dirs.contains(path);

  @override
  Future<Directory> create({bool recursive = false}) async {
    createSync(recursive: recursive);
    return this;
  }

  @override
  void createSync({bool recursive = false}) => _makeDirs(path);

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
    final inside = '$path/';
    for (final file in _files.keys.where((f) => f.startsWith(inside)).toList()) {
      _drop(file);
    }
    _dirs.removeWhere((d) => d == path || d.startsWith(inside));
  }

  @override
  Future<Directory> rename(String newPath) async => renameSync(newPath);

  @override
  Directory renameSync(String newPath) {
    final to = _clean(newPath);
    final inside = '$path/';
    for (final file in _files.keys.where((f) => f.startsWith(inside)).toList()) {
      final bytes = _files[file]!;
      _drop(file);
      _put('$to/${file.substring(inside.length)}', bytes);
    }
    _dirs.removeWhere((d) => d == path || d.startsWith(inside));
    _makeDirs(to);
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
      {bool recursive = false, bool followLinks = true}) {
    final inside = path == '/' ? '/' : '$path/';
    bool under(String p) =>
        p != path &&
        p.startsWith(inside) &&
        (recursive || !p.substring(inside.length).contains('/'));
    return [
      for (final d in _dirs.where(under)) _WebDirectory(d),
      for (final f in _files.keys.where(under)) _WebFile(f),
    ];
  }
}

class _WebStat implements FileStat {
  _WebStat(String path)
      : type = _typeOf(path),
        size = _files[_clean(path)]?.length ?? -1,
        modified = _changed[_clean(path)] ?? DateTime.now();

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
