// Patching an update out of the file the last one came from.
//
// Every release attaches a blockmap beside the file the updater fetches. It
// is that file cut into pieces of about 64 KB, cut where the bytes themselves
// say to, so a piece that only moved is still found. The updater cuts the
// file it kept from last time the same way, copies every piece the two share
// and fetches the rest by range. The release pipeline runs this same code
// (tool/blockmap.dart), so both ends always cut alike.

import 'dart:convert';
import 'dart:io';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:crypto/crypto.dart';

/// What a release calls the blockmap of [assetName].
String blockmapNameFor(String assetName) => '$assetName.blockmap';

/// The most a blockmap may weigh. A 100 MB file maps to about 60 KB.
const int blockmapMaxBytes = 4 << 20;

const int _minPiece = 16 * 1024;
const int _maxPiece = 256 * 1024;

// A cut wherever the low 16 bits of the rolling hash are zero, which is about
// every 64 KB past the minimum.
const int _cutMask = 0xFFFF;

/// A fixed random number per byte value, the same on every machine.
final Uint32List _gear = () {
  final table = Uint32List(256);
  var x = 0x9E3779B9;
  for (var i = 0; i < 256; i++) {
    x ^= (x << 13) & 0xFFFFFFFF;
    x ^= x >> 17;
    x ^= (x << 5) & 0xFFFFFFFF;
    table[i] = x;
  }
  return table;
}();

/// One piece of a file: its length and the first 128 bits of its SHA-256.
typedef Piece = ({int size, String hash});

Piece _piece(Uint8List bytes) => (
      size: bytes.length,
      hash: sha256.convert(bytes).toString().substring(0, 32),
    );

/// Cut [file] into pieces. Reads the whole file, so keep it off the interface
/// thread ([piecesInBackground]).
List<Piece> piecesOf(File file) {
  final pieces = <Piece>[];
  final raf = file.openSync();
  try {
    final buffer = Uint8List(1 << 20);
    final piece = BytesBuilder(copy: false);
    var h = 0;
    var size = 0;
    while (true) {
      final read = raf.readIntoSync(buffer);
      if (read == 0) break;
      var start = 0;
      for (var i = 0; i < read; i++) {
        h = ((h << 1) + _gear[buffer[i]]) & 0xFFFFFFFF;
        size++;
        if (size >= _maxPiece || (size >= _minPiece && (h & _cutMask) == 0)) {
          piece.add(Uint8List.sublistView(buffer, start, i + 1));
          pieces.add(_piece(piece.takeBytes()));
          start = i + 1;
          size = 0;
          h = 0;
        }
      }
      // The buffer is reused, so the tail of this read is copied out.
      if (start < read) piece.add(buffer.sublist(start, read));
    }
    if (piece.length > 0) pieces.add(_piece(piece.takeBytes()));
  } finally {
    raf.closeSync();
  }
  return pieces;
}

/// [piecesOf] on another isolate. A 100 MB file takes about a second.
Future<List<Piece>> piecesInBackground(String path) =>
    Isolate.run(() => piecesOf(File(path)));

/// A blockmap as the release attaches it.
String encodeBlockmap(List<Piece> pieces) => jsonEncode({
      'pieces': [
        for (final p in pieces) [p.size, p.hash]
      ],
    });

/// The pieces out of a blockmap, or null when it isn't one.
List<Piece>? decodeBlockmap(String text) {
  try {
    final raw = (jsonDecode(text) as Map)['pieces'] as List;
    return [
      for (final p in raw) (size: (p as List)[0] as int, hash: p[1] as String),
    ];
  } catch (_) {
    return null;
  }
}

/// A stretch of the new file: fetched from the new file at [from] when
/// [fetch], otherwise copied from the old file at [from].
typedef Run = ({bool fetch, int from, int size});

/// How to make the new file, mapped by [wanted], out of the old one, cut into
/// [have]. Neighbouring pieces are joined into one run, so each fetch is one
/// request.
List<Run> patchPlan(List<Piece> wanted, List<Piece> have) {
  final where = <String, int>{};
  var at = 0;
  for (final p in have) {
    where.putIfAbsent(p.hash, () => at);
    at += p.size;
  }
  final runs = <Run>[];
  at = 0;
  for (final p in wanted) {
    final old = where[p.hash];
    final run = (fetch: old == null, from: old ?? at, size: p.size);
    final last = runs.isEmpty ? null : runs.last;
    if (last != null &&
        last.fetch == run.fetch &&
        last.from + last.size == run.from) {
      runs.last =
          (fetch: run.fetch, from: last.from, size: last.size + run.size);
    } else {
      runs.add(run);
    }
    at += p.size;
  }
  return runs;
}

/// How many bytes [plan] fetches.
int bytesToFetch(List<Run> plan) => [
      for (final r in plan)
        if (r.fetch) r.size
    ].fold(0, (sum, size) => sum + size);
