// The Dart half of bridge v0 (docs/flutter-port/03-ARCHITECTURE.md "Bridge
// v0"): a thin dart:ffi wrapper over the `lumit_bridge` shared library. Dart
// calls the crate's C functions, each of which returns a Rust-owned UTF-8 JSON
// string; this side copies the string out, immediately frees it back to Rust,
// and decodes the JSON into typed Dart classes.
//
// The whole frontend must work WITHOUT the library present: `tryLoad` returns
// null when the `.dll` cannot be found or bound, and the app keeps its
// placeholder behaviour. Nothing here is imported into a code path that runs
// before a successful `tryLoad`, so the tests (which never load the library)
// stay green.

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

/// The kind of a project item, mirroring `lumit_core::model::ProjectItem`.
/// `unknown` covers a kind string a newer engine might add — drawn quietly
/// rather than crashing.
enum BridgeItemKind { footage, folder, composition, solid, unknown }

BridgeItemKind _kindOf(Object? raw) => switch (raw) {
      'footage' => BridgeItemKind.footage,
      'folder' => BridgeItemKind.folder,
      'composition' => BridgeItemKind.composition,
      'solid' => BridgeItemKind.solid,
      _ => BridgeItemKind.unknown,
    };

/// One node in the Project panel tree. Folders carry nested [children]; every
/// other kind carries an empty list.
class BridgeItem {
  final String id;
  final String name;
  final BridgeItemKind kind;
  final List<BridgeItem> children;

  const BridgeItem({
    required this.id,
    required this.name,
    required this.kind,
    required this.children,
  });

  factory BridgeItem.fromJson(Map<String, dynamic> m) {
    final rawChildren = m['children'];
    final children = <BridgeItem>[];
    if (rawChildren is List) {
      for (final child in rawChildren) {
        if (child is Map) {
          children.add(BridgeItem.fromJson(child.cast<String, dynamic>()));
        }
      }
    }
    return BridgeItem(
      id: m['id'] is String ? m['id'] as String : '',
      name: m['name'] is String ? m['name'] as String : '',
      kind: _kindOf(m['kind']),
      children: children,
    );
  }
}

/// A decoded document snapshot — the `{"ok":true, …}` reply shape.
class BridgeSnapshot {
  final List<BridgeItem> items;
  final bool canUndo;
  final bool canRedo;

  /// The loaded/last-saved project path, or null for an unsaved document.
  final String? path;

  const BridgeSnapshot({
    required this.items,
    required this.canUndo,
    required this.canRedo,
    required this.path,
  });

  factory BridgeSnapshot.fromJson(Map<String, dynamic> m) {
    final rawItems = m['items'];
    final items = <BridgeItem>[];
    if (rawItems is List) {
      for (final item in rawItems) {
        if (item is Map) {
          items.add(BridgeItem.fromJson(item.cast<String, dynamic>()));
        }
      }
    }
    return BridgeSnapshot(
      items: items,
      canUndo: m['can_undo'] == true,
      canRedo: m['can_redo'] == true,
      path: m['path'] is String ? m['path'] as String : null,
    );
  }
}

/// The result of one bridge call: a snapshot on success, or a calm error string
/// for the status line on failure. Parsing a malformed reply is itself an
/// error, never a throw.
class BridgeReply {
  final BridgeSnapshot? snapshot;
  final String? error;

  const BridgeReply.ok(this.snapshot) : error = null;
  const BridgeReply.err(this.error) : snapshot = null;

  bool get ok => error == null;

  /// Decode a reply string. `{"ok":true,…}` yields a snapshot; `{"ok":false,
  /// "error":"…"}` yields the error; anything else is reported as malformed.
  factory BridgeReply.parse(String raw) {
    Object? decoded;
    try {
      decoded = jsonDecode(raw);
    } catch (_) {
      return const BridgeReply.err('bridge returned malformed JSON');
    }
    if (decoded is! Map) {
      return const BridgeReply.err('bridge returned malformed JSON');
    }
    final map = decoded.cast<String, dynamic>();
    if (map['ok'] == true) {
      return BridgeReply.ok(BridgeSnapshot.fromJson(map));
    }
    final err = map['error'];
    return BridgeReply.err(err is String ? err : 'bridge error');
  }
}

// The C signatures. Strings cross as `Pointer<Char>`; the engine allocates the
// replies and frees them through `lumit_bridge_free_string`.
typedef _NoArgC = Pointer<Char> Function();
typedef _NoArgDart = Pointer<Char> Function();
typedef _StrArgC = Pointer<Char> Function(Pointer<Char>);
typedef _StrArgDart = Pointer<Char> Function(Pointer<Char>);
typedef _FreeC = Void Function(Pointer<Char>);
typedef _FreeDart = void Function(Pointer<Char>);

/// The set of document operations the frontend drives the engine through. The
/// real implementation is [LumitBridge] (dart:ffi over the shared library); the
/// interface exists so tests can supply a fake without loading the library or
/// touching plugin channels — every method is a pure `String → BridgeReply`
/// call, so a fake is a handful of lines.
abstract class DocumentBridge {
  BridgeReply snapshot();
  BridgeReply newProject();
  BridgeReply undo();
  BridgeReply redo();
  BridgeReply openProject(String path);

  /// Save to [path]; an empty string saves to the loaded path (an error reply
  /// if the document has never been saved).
  BridgeReply saveProject(String path);
  BridgeReply newComposition(String name);

  /// Add a footage item referencing the media file at [path]. No probing yet
  /// (F2 adds it) — the item just carries the path.
  BridgeReply importFootage(String path);
}

/// The loaded `lumit_bridge` library, bound to typed calls. Construct with
/// [tryLoad]; a null result means the app runs on its placeholders.
class LumitBridge implements DocumentBridge {
  final _NoArgDart _version;
  final _NoArgDart _newProject;
  final _StrArgDart _openProject;
  final _StrArgDart _saveProject;
  final _NoArgDart _snapshot;
  final _StrArgDart _newComposition;
  final _StrArgDart _importFootage;
  final _NoArgDart _undo;
  final _NoArgDart _redo;
  final _FreeDart _freeString;

  LumitBridge._(DynamicLibrary lib)
      : _version = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_version',
        ),
        _newProject = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_new_project',
        ),
        _openProject = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_open_project',
        ),
        _saveProject = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_save_project',
        ),
        _snapshot = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_snapshot',
        ),
        _newComposition = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_new_composition',
        ),
        _importFootage = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_import_footage',
        ),
        _undo = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_undo',
        ),
        _redo = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_redo',
        ),
        _freeString = lib.lookupFunction<_FreeC, _FreeDart>(
          'lumit_bridge_free_string',
        );

  /// Load the library and bind it, or return null if it cannot be found or a
  /// symbol is missing. Never throws — a failure is just "run on placeholders".
  static LumitBridge? tryLoad() {
    for (final candidate in _candidatePaths()) {
      try {
        final lib = DynamicLibrary.open(candidate);
        return LumitBridge._(lib);
      } catch (_) {
        // Try the next candidate.
      }
    }
    return null;
  }

  /// Where the library might live, in the order the runner should try:
  /// beside the executable first (the shipped layout), then the Cargo debug
  /// output relative to the working directory (the developer layout), then the
  /// bare name so the OS loader's own search path gets a turn.
  static List<String> _candidatePaths() {
    const name = 'lumit_bridge.dll';
    final paths = <String>[];
    try {
      final exeDir = File(Platform.resolvedExecutable).parent.path;
      paths.add('$exeDir${Platform.pathSeparator}$name');
    } catch (_) {
      // resolvedExecutable can be unavailable in some hosts; skip it.
    }
    final cwd = Directory.current.path;
    final sep = Platform.pathSeparator;
    paths.add('$cwd$sep..$sep..$sep..${sep}target${sep}debug$sep$name');
    paths.add('$cwd$sep..${sep}target${sep}debug$sep$name');
    paths.add(name);
    return paths;
  }

  /// `{"version":"…","abi":1,"ok":true}` as the raw decoded map, or null if the
  /// reply is malformed. Used for a boot-time handshake / log line.
  Map<String, dynamic>? version() {
    final raw = _callNoArg(_version);
    try {
      final decoded = jsonDecode(raw);
      return decoded is Map ? decoded.cast<String, dynamic>() : null;
    } catch (_) {
      return null;
    }
  }

  @override
  BridgeReply snapshot() => BridgeReply.parse(_callNoArg(_snapshot));
  @override
  BridgeReply newProject() => BridgeReply.parse(_callNoArg(_newProject));
  @override
  BridgeReply undo() => BridgeReply.parse(_callNoArg(_undo));
  @override
  BridgeReply redo() => BridgeReply.parse(_callNoArg(_redo));

  @override
  BridgeReply openProject(String path) =>
      BridgeReply.parse(_callStrArg(_openProject, path));

  /// Save to [path]; an empty string saves to the loaded path (an error reply
  /// if the document has never been saved).
  @override
  BridgeReply saveProject(String path) =>
      BridgeReply.parse(_callStrArg(_saveProject, path));

  @override
  BridgeReply newComposition(String name) =>
      BridgeReply.parse(_callStrArg(_newComposition, name));

  @override
  BridgeReply importFootage(String path) =>
      BridgeReply.parse(_callStrArg(_importFootage, path));

  // Call → copy the reply out → free it back to Rust. The copy must happen
  // before the free, so `toDartString` runs inside the try and the free in the
  // finally.
  String _callNoArg(_NoArgDart fn) {
    final ptr = fn();
    if (ptr == nullptr) {
      return '{"ok":false,"error":"bridge returned a null reply"}';
    }
    try {
      return ptr.cast<Utf8>().toDartString();
    } finally {
      _freeString(ptr);
    }
  }

  String _callStrArg(_StrArgDart fn, String arg) {
    final argPtr = arg.toNativeUtf8();
    try {
      final ptr = fn(argPtr.cast<Char>());
      if (ptr == nullptr) {
        return '{"ok":false,"error":"bridge returned a null reply"}';
      }
      try {
        return ptr.cast<Utf8>().toDartString();
      } finally {
        _freeString(ptr);
      }
    } finally {
      malloc.free(argPtr);
    }
  }
}
