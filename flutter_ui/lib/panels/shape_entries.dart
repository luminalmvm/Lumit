// A shape layer's contents as the bridge hands it over: one flat list of
// entries, items, modifiers and groups, each with the depth it sits at. A
// group is the entry that opens it; the entries after it one deeper are
// inside. Every edit is "the list, changed", written back whole.

import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/shape.dart';
import 'package:uuid/uuid.dart';

/// The id of whatever [entry] is.
UuidValue shapeEntryId(BridgeShapeEntry entry) => switch (entry.kind) {
      BridgeShapeEntryKind_Item(:final field0) => field0.id,
      BridgeShapeEntryKind_Modifier(:final field0) => field0.id,
      BridgeShapeEntryKind_Group(:final field0) => field0.id,
    };

/// [entry] at another depth.
BridgeShapeEntry shapeEntryAt(BridgeShapeEntry entry, int depth) =>
    BridgeShapeEntry(depth: depth, kind: entry.kind);

extension ShapeEntries on List<BridgeShapeEntry> {
  /// The pieces of art, groups opened, in list order.
  Iterable<BridgeShapeItem> get items => [
        for (final e in this)
          if (e.kind case BridgeShapeEntryKind_Item(:final field0)) field0,
      ];

  /// The modifier entries, groups opened, in list order.
  Iterable<BridgeShapeModifier> get modifiers => [
        for (final e in this)
          if (e.kind case BridgeShapeEntryKind_Modifier(:final field0)) field0,
      ];

  int _indexOf(UuidValue id) => indexWhere((e) => shapeEntryId(e) == id);

  /// One past the entry at [index] and everything inside it.
  int _blockEnd(int index) {
    final depth = this[index].depth;
    var end = index + 1;
    while (end < length && this[end].depth > depth) {
      end++;
    }
    return end;
  }

  /// The list with the item [id] replaced by [change] of it.
  List<BridgeShapeEntry> withItem(
          UuidValue id, BridgeShapeItem Function(BridgeShapeItem) change) =>
      [
        for (final e in this)
          switch (e.kind) {
            BridgeShapeEntryKind_Item(:final field0) when field0.id == id =>
              BridgeShapeEntry(
                  depth: e.depth,
                  kind: BridgeShapeEntryKind.item(change(field0))),
            _ => e,
          },
      ];

  /// The list with the modifier [id] replaced by [change] of it.
  List<BridgeShapeEntry> withModifier(UuidValue id,
          BridgeShapeModifier Function(BridgeShapeModifier) change) =>
      [
        for (final e in this)
          switch (e.kind) {
            BridgeShapeEntryKind_Modifier(:final field0)
                when field0.id == id =>
              BridgeShapeEntry(
                  depth: e.depth,
                  kind: BridgeShapeEntryKind.modifier(change(field0))),
            _ => e,
          },
      ];

  /// The list with the group [id] renamed.
  List<BridgeShapeEntry> withGroupName(UuidValue id, String name) => [
        for (final e in this)
          switch (e.kind) {
            BridgeShapeEntryKind_Group(:final field0) when field0.id == id =>
              BridgeShapeEntry(
                  depth: e.depth,
                  kind: BridgeShapeEntryKind.group(
                      BridgeShapeGroup(id: id, name: name))),
            _ => e,
          },
      ];

  /// The list without the entry [id], and without whatever sat inside it.
  List<BridgeShapeEntry> without(UuidValue id) {
    final i = _indexOf(id);
    if (i < 0) return this;
    return [...sublist(0, i), ...sublist(_blockEnd(i))];
  }

  /// The list with [kind] added after the entry [id] (after everything
  /// inside it, for a group), at the same depth. A modifier added this way
  /// acts on the entry it was added under.
  List<BridgeShapeEntry> insertedAfter(UuidValue id, BridgeShapeEntryKind kind) {
    final i = _indexOf(id);
    if (i < 0) return this;
    final end = _blockEnd(i);
    return [
      ...sublist(0, end),
      BridgeShapeEntry(depth: this[i].depth, kind: kind),
      ...sublist(end),
    ];
  }

  /// The list with [kind] added at the end of the group [id], inside it.
  List<BridgeShapeEntry> appendedInto(UuidValue id, BridgeShapeEntryKind kind) {
    final i = _indexOf(id);
    if (i < 0) return this;
    final end = _blockEnd(i);
    return [
      ...sublist(0, end),
      BridgeShapeEntry(depth: this[i].depth + 1, kind: kind),
      ...sublist(end),
    ];
  }

  /// The list with the entry [id] and everything inside it wrapped in a new
  /// group called [name].
  List<BridgeShapeEntry> grouped(UuidValue id, String name) {
    final i = _indexOf(id);
    if (i < 0) return this;
    final end = _blockEnd(i);
    return [
      ...sublist(0, i),
      BridgeShapeEntry(
          depth: this[i].depth,
          kind: BridgeShapeEntryKind.group(BridgeShapeGroup(
              id: UuidValue.fromString(const Uuid().v4()), name: name))),
      for (final e in sublist(i, end)) shapeEntryAt(e, e.depth + 1),
      ...sublist(end),
    ];
  }

  /// The list with the group [id] opened out: its entries take its place.
  List<BridgeShapeEntry> ungrouped(UuidValue id) {
    final i = _indexOf(id);
    if (i < 0) return this;
    final end = _blockEnd(i);
    return [
      ...sublist(0, i),
      for (final e in sublist(i + 1, end)) shapeEntryAt(e, e.depth - 1),
      ...sublist(end),
    ];
  }

  /// The list with the entry [id] (and everything inside it) swapped with the
  /// entry before it ([by] -1) or after it (+1) at its own depth. Unchanged
  /// where there is none.
  List<BridgeShapeEntry> moved(UuidValue id, int by) {
    final i = _indexOf(id);
    if (i < 0) return this;
    final depth = this[i].depth;
    final end = _blockEnd(i);
    if (by < 0) {
      var p = i - 1;
      while (p >= 0 && this[p].depth > depth) {
        p--;
      }
      if (p < 0 || this[p].depth < depth) return this;
      return [
        ...sublist(0, p),
        ...sublist(i, end),
        ...sublist(p, i),
        ...sublist(end),
      ];
    }
    if (end >= length || this[end].depth < depth) return this;
    final nextEnd = _blockEnd(end);
    return [
      ...sublist(0, i),
      ...sublist(end, nextEnd),
      ...sublist(i, end),
      ...sublist(nextEnd),
    ];
  }

  /// The modifiers acting on the item at [index]: the ones after it at its
  /// depth, then the ones after its group outside, and so on up. The
  /// engine's `lowered`, in the same order.
  List<BridgeShapeModifier> modifiersOn(int index) {
    final out = <BridgeShapeModifier>[];
    var depth = this[index].depth;
    for (var j = index + 1; j < length; j++) {
      final e = this[j];
      if (e.depth > depth) continue;
      depth = e.depth;
      if (e.kind case BridgeShapeEntryKind_Modifier(:final field0)) {
        out.add(field0);
      }
    }
    return out;
  }
}
