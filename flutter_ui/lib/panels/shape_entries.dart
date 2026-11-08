// A shape layer's contents as the bridge hands it over: one flat list of
// entries, items, modifiers and groups, each with the depth it sits at. A
// group is the entry that opens it; the entries after it one deeper are
// inside. Every edit is "the list, changed", written back whole.

import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/shape.dart';
import 'package:uuid/uuid.dart';

/// The id of whatever [entry] is.
UuidValue shapeEntryId(BridgeShapeEntry entry) => switch (entry.kind) {
      BridgeShapeEntryKind_Item(:final field0) => field0.id,
      BridgeShapeEntryKind_Modifier(:final field0) => field0.id,
      BridgeShapeEntryKind_Group(:final field0) => field0.id,
    };

/// Whether [item]'s shape moves: keyed, or made from numbers that move.
bool shapeMoves(BridgeShapeItem item) {
  final p = item.primitive;
  if (p == null) return item.pathKeys.isNotEmpty;
  return [
    p.positionX,
    p.positionY,
    p.width,
    p.height,
    p.roundness,
    p.points,
    p.rotation,
    p.outerRadius,
    p.innerRadius,
    p.outerRoundness,
    p.innerRoundness,
  ].any((s) => s is! BridgeScalar_Static);
}

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

  /// One past the entry at [index], everything inside it and, for art, the
  /// modifiers that are its own. An own modifier acts on the art right above
  /// it, so it goes where that art goes.
  int _withOwnEnd(int index) {
    final depth = this[index].depth;
    var end = _blockEnd(index);
    if (this[index].kind is BridgeShapeEntryKind_Modifier) return end;
    while (end < length && this[end].depth == depth && _isOwn(end)) {
      end++;
    }
    return end;
  }

  bool _isOwn(int index) => switch (this[index].kind) {
        BridgeShapeEntryKind_Modifier(:final field0) => field0.own,
        _ => false,
      };

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
  List<BridgeShapeEntry> withGroupName(UuidValue id, String name) =>
      withGroup(id, name: name);

  /// The list with the group [id] renamed, or its eye switched.
  List<BridgeShapeEntry> withGroup(UuidValue id,
          {String? name, bool? hidden}) =>
      [
        for (final e in this)
          switch (e.kind) {
            BridgeShapeEntryKind_Group(:final field0) when field0.id == id =>
              BridgeShapeEntry(
                  depth: e.depth,
                  kind: BridgeShapeEntryKind.group(BridgeShapeGroup(
                      id: id,
                      name: name ?? field0.name,
                      hidden: hidden ?? field0.hidden))),
            _ => e,
          },
      ];

  /// The list without the entry [id], without whatever sat inside it and
  /// without its own modifiers.
  List<BridgeShapeEntry> without(UuidValue id) {
    final i = _indexOf(id);
    if (i < 0) return this;
    return [...sublist(0, i), ...sublist(_withOwnEnd(i))];
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
              id: UuidValue.fromString(const Uuid().v4()),
              name: name,
              hidden: false))),
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

  /// The list with the entry [id] (with everything inside it and its own
  /// modifiers) swapped with the entry before it ([by] -1) or after it (+1) at
  /// its own depth. Unchanged where there is none.
  List<BridgeShapeEntry> moved(UuidValue id, int by) {
    final i = _indexOf(id);
    if (i < 0) return this;
    final depth = this[i].depth;
    final art = this[i].kind is! BridgeShapeEntryKind_Modifier;
    final end = _withOwnEnd(i);
    if (by < 0) {
      var p = i - 1;
      while (p >= 0 && this[p].depth > depth) {
        p--;
      }
      // Art swaps with the art before it, whose own modifiers go with it.
      while (art && p > 0 && this[p].depth == depth && _isOwn(p)) {
        p--;
        while (p > 0 && this[p].depth > depth) {
          p--;
        }
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
    final nextEnd = art ? _withOwnEnd(end) : _blockEnd(end);
    return [
      ...sublist(0, i),
      ...sublist(end, nextEnd),
      ...sublist(i, end),
      ...sublist(nextEnd),
    ];
  }

  /// The modifiers acting on the item at [index]: the ones after it at its
  /// depth, then the ones after its group outside, and so on up, eyes off
  /// left out. One added from an item's Add (`own`) reaches back to the art
  /// right above it and no further. The engine's `lowered`, in the same
  /// order.
  List<BridgeShapeModifier> modifiersOn(int index) {
    final out = <BridgeShapeModifier>[];
    var depth = this[index].depth;
    var mine = true;
    for (var j = index + 1; j < length; j++) {
      final e = this[j];
      if (e.depth > depth) continue;
      if (e.depth < depth) {
        // Out of a group: the group is the art the next ones follow.
        depth = e.depth;
        mine = true;
      }
      switch (e.kind) {
        case BridgeShapeEntryKind_Modifier(:final field0):
          if (!field0.hidden && (mine || !field0.own)) out.add(field0);
        case BridgeShapeEntryKind_Item(:final field0)
            when field0.combine != 0 &&
                this[j - 1].depth == e.depth &&
                this[j - 1].kind is BridgeShapeEntryKind_Item:
          // Combined into the item above: drawn as one with it.
          break;
        default:
          mine = false;
      }
    }
    return out;
  }

  /// [moved], unless the move would take an entry into or out of the
  /// global modifiers at the end: those stay below every shape.
  List<BridgeShapeEntry> movedKeepingGlobals(UuidValue id, int by) {
    final next = moved(id, by);
    Iterable<UuidValue> globals(List<BridgeShapeEntry> e) =>
        e.sublist(e.globalStart).map(shapeEntryId);
    return globals(next).toSet().containsAll(globals(this)) &&
            globals(this).toSet().containsAll(globals(next))
        ? next
        : this;
  }

  /// Where the layer's global modifiers start: the run at the end of the
  /// top list of modifiers that are not an item's own, which act on every
  /// shape above them. [length] when there are none.
  int get globalStart {
    var i = length;
    while (i > 0 &&
        this[i - 1].depth == 0 &&
        switch (this[i - 1].kind) {
          BridgeShapeEntryKind_Modifier(:final field0) => !field0.own,
          _ => false,
        }) {
      i--;
    }
    return i;
  }
}
