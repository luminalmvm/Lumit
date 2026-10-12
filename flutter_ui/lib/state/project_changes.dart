// What changed between two saves of a project, worked out from the two
// project files themselves.
//
// A project file is a zip holding `manifest.json` and `project.json`. The
// second is the whole document, and everything in it that a person made has
// an id. So two files can be compared thing by thing: which compositions,
// layers, effects and footage one has that the other has not, and which
// layers are in both and no longer the same.
//
// The files come from outside, so they are read like anything from outside.
// Every offset is checked, nothing is unpacked past a limit, and a file that
// can't be read gives no answer, which is better than a wrong one.

import 'dart:convert';
import 'dart:io';
import 'dart:isolate';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';

/// The most a project's document may unpack to and still be compared.
const int _mostUnpacked = 192 * 1024 * 1024;

/// Takes what a zip entry unpacks to, and stops at the size it was said to
/// be, so a file that lies about itself can't fill memory.
class _Bounded implements Sink<List<int>> {
  final int most;
  final BytesBuilder out = BytesBuilder(copy: false);
  _Bounded(this.most);

  @override
  void add(List<int> chunk) {
    if (out.length + chunk.length > most) {
      throw const FormatException('unpacks to more than it said');
    }
    out.add(chunk);
  }

  @override
  void close() {}
}

/// The entry called [name] in the zip [file], unpacked. Null when it is not
/// there, is larger than [most], or the zip is not one this reads.
@visibleForTesting
Uint8List? zipEntry(Uint8List file, String name, {int most = _mostUnpacked}) {
  try {
    final data = ByteData.sublistView(file);
    int u16(int at) => data.getUint16(at, Endian.little);
    int u32(int at) => data.getUint32(at, Endian.little);
    // The list of entries is found from a record at the very end, which is
    // 22 bytes and may have up to 65,535 of comment after it.
    var end = file.length - 22;
    final first = math.max(0, end - 0xffff);
    while (end >= first && u32(end) != 0x06054b50) {
      end--;
    }
    if (end < first) return null;
    final count = u16(end + 10);
    var at = u32(end + 16);
    for (var i = 0; i < count; i++) {
      if (u32(at) != 0x02014b50) return null;
      final method = u16(at + 10);
      final packed = u32(at + 20);
      final size = u32(at + 24);
      final named = u16(at + 28);
      final local = u32(at + 42);
      final called = utf8.decode(
          Uint8List.sublistView(file, at + 46, at + 46 + named),
          allowMalformed: true);
      at += 46 + named + u16(at + 30) + u16(at + 32);
      if (called != name) continue;
      if (size > most || u32(local) != 0x04034b50) return null;
      final start = local + 30 + u16(local + 26) + u16(local + 28);
      final bytes = Uint8List.sublistView(file, start, start + packed);
      // Stored as it is, or deflated. Nothing else is written into a .lum.
      if (method == 0) return packed == size ? bytes : null;
      if (method != 8) return null;
      final sink = _Bounded(size);
      final inflate = ZLibDecoder(raw: true).startChunkedConversion(sink);
      // A piece at a time, so one that unpacks too far is stopped early.
      for (var from = 0; from < bytes.length; from += 65536) {
        inflate.add(Uint8List.sublistView(
            bytes, from, math.min(from + 65536, bytes.length)));
      }
      inflate.close();
      return sink.out.length == size ? sink.out.takeBytes() : null;
    }
    return null;
  } catch (_) {
    // An offset past the end, or bytes that are not what they claim to be.
    return null;
  }
}

/// What a project file holds, as far as comparing two of them needs.
class ProjectShape {
  /// The Lumit that wrote the file, as its manifest says.
  final String writer;

  /// Each composition's name, by its id, in the project's own order.
  final Map<String, String> comps = {};

  /// Each composition's layers by their ids, with a digest of everything
  /// the layer holds.
  final Map<String, Map<String, String>> layers = {};

  /// The ids of the effects on each layer, by the layer's id.
  final Map<String, Set<String>> effects = {};

  /// The id of every footage item.
  final Set<String> footage = {};

  ProjectShape(this.writer);
}

/// Read what [file] holds, the bytes of a `.lum`. Null when they can't be
/// read as one.
ProjectShape? projectShape(Uint8List file) {
  try {
    final document = zipEntry(file, 'project.json');
    if (document == null) return null;
    final doc = jsonDecode(utf8.decode(document));
    final items = doc is Map ? doc['items'] : null;
    if (items is! List) return null;
    var writer = '';
    final manifest = zipEntry(file, 'manifest.json', most: 1 << 20);
    if (manifest != null) {
      final said = jsonDecode(utf8.decode(manifest));
      if (said is Map) writer = '${said['written_by']} ${said['schema_version']}';
    }
    final shape = ProjectShape(writer);
    for (final item in items) {
      if (item is! Map) continue;
      final footage = item['Footage'];
      if (footage is Map && footage['id'] is String) {
        shape.footage.add(footage['id'] as String);
      }
      final comp = item['Composition'];
      if (comp is! Map || comp['id'] is! String) continue;
      final id = comp['id'] as String;
      shape.comps[id] = '${comp['name'] ?? ''}';
      final layers = shape.layers[id] = {};
      final within = comp['layers'];
      if (within is! List) continue;
      for (final layer in within) {
        if (layer is! Map || layer['id'] is! String) continue;
        final layerId = layer['id'] as String;
        layers[layerId] = '${sha1.convert(utf8.encode(jsonEncode(layer)))}';
        final effects = layer['effects'];
        shape.effects[layerId] = {
          if (effects is List)
            for (final effect in effects)
              if (effect is Map && effect['id'] is String)
                effect['id'] as String,
        };
      }
    }
    return shape;
  } catch (_) {
    return null;
  }
}

/// The same, off the UI thread, and only when [file] is the bytes [hash]
/// says it is.
Future<ProjectShape?> readProjectShape(Uint8List file, String hash) =>
    Isolate.run(() =>
        '${sha256.convert(file)}' == hash ? projectShape(file) : null);

/// What is different in one save of a project from the save before it.
class ProjectChanges {
  int compsAdded = 0, compsRemoved = 0, compsRenamed = 0;
  int layersAdded = 0, layersRemoved = 0, layersChanged = 0;
  int effectsAdded = 0, effectsRemoved = 0;
  int footageAdded = 0, footageRemoved = 0;

  /// The names of the compositions those layers are in, in the project's
  /// own order.
  final List<String> addedIn = [], removedIn = [], changedIn = [];

  bool get none =>
      compsAdded + compsRemoved + compsRenamed == 0 &&
      layersAdded + layersRemoved + layersChanged == 0 &&
      effectsAdded + effectsRemoved + footageAdded + footageRemoved == 0;
}

/// Compare two saves of one project by the ids of what is in them.
///
/// A composition that is new, or gone, is counted once and its layers are
/// not counted again. Effects are counted on layers that are in both saves.
ProjectChanges changesBetween(ProjectShape before, ProjectShape after) {
  final changes = ProjectChanges();
  // A different Lumit may write the same layer differently, which would
  // make every layer look changed. Added and removed go by id, and hold.
  final sameWriter = before.writer == after.writer;
  changes.footageAdded = after.footage.difference(before.footage).length;
  changes.footageRemoved = before.footage.difference(after.footage).length;
  changes.compsRemoved =
      before.comps.keys.where((id) => !after.comps.containsKey(id)).length;
  for (final MapEntry(key: id, value: name) in after.comps.entries) {
    final was = before.comps[id];
    if (was == null) {
      changes.compsAdded++;
      continue;
    }
    if (was != name) changes.compsRenamed++;
    final then = before.layers[id] ?? const <String, String>{};
    final now = after.layers[id] ?? const <String, String>{};
    var added = 0, changed = 0;
    for (final MapEntry(key: layer, value: digest) in now.entries) {
      final old = then[layer];
      if (old == null) {
        added++;
        continue;
      }
      if (sameWriter && old != digest) changed++;
      final had = before.effects[layer] ?? const <String>{};
      final has = after.effects[layer] ?? const <String>{};
      changes.effectsAdded += has.difference(had).length;
      changes.effectsRemoved += had.difference(has).length;
    }
    final removed = then.keys.where((layer) => !now.containsKey(layer)).length;
    changes.layersAdded += added;
    changes.layersRemoved += removed;
    changes.layersChanged += changed;
    if (added > 0) changes.addedIn.add(name);
    if (removed > 0) changes.removedIn.add(name);
    if (changed > 0) changes.changedIn.add(name);
  }
  return changes;
}
