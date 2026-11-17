// What a library sync does about each item, worked out from three lists: what
// is here now, what was here at the last sync (the journal), and what the
// account holds. Pure, with no files and no server, so every rule can be
// checked by a test. The rules are the table in the library's design: a
// change beats a delete, a change on both sides is a conflict, and the
// files here are never thrown away unless the account already has them.

import 'dart:convert';

/// The kinds of thing a library holds. The names are what the server files
/// them under.
enum LibraryKind {
  presets,
  styles,
  themes,
  fonts,
  shortcuts,
  lenses,
  flares,
  exports,
  luts,
  templates
}

/// One file of an item. [name] is empty for an item that is one file, else
/// its path inside the item's folder.
class LibraryFile {
  final String name, hash;
  final int size;
  const LibraryFile(this.name, this.hash, this.size);

  static LibraryFile? from(Object? j) => j is List &&
          j.length == 3 &&
          j[0] is String &&
          j[1] is String &&
          j[2] is int
      ? LibraryFile(j[0] as String, j[1] as String, j[2] as int)
      : null;

  List<Object> toJson() => [name, hash, size];

  @override
  bool operator ==(Object other) =>
      other is LibraryFile &&
      other.name == name &&
      other.hash == hash &&
      other.size == size;

  @override
  int get hashCode => Object.hash(name, hash, size);
}

/// Whether two lists of files are the same item. Both are kept in name order.
bool sameFiles(List<LibraryFile> a, List<LibraryFile> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i].name != b[i].name || a[i].hash != b[i].hash) return false;
  }
  return true;
}

/// An item as the journal or the account has it: its id, where it is, the
/// version and its files. [gone] while it is in Recently deleted.
class LibraryItem {
  final String id;
  final LibraryKind kind;
  final String path;
  final int n;
  final List<LibraryFile> files;
  final bool gone;
  const LibraryItem(this.id, this.kind, this.path, this.n, this.files,
      {this.gone = false});
}

/// An item here, by its kind and its path inside the kind's folder.
typedef LibraryKey = (LibraryKind, String);

/// What a step does, in the order a pass carries them out: what only writes
/// the journal first, then what makes room here, then what comes down, then
/// what goes up.
enum PlanAction {
  forget,
  record,
  renameHere,
  renameThere,
  keepBoth,
  deleteHere,
  deleteThere,
  download,
  conflict,
  merge,
  upload,
  revive,
}

/// One thing to do.
///
/// [path] is where the item is to be, here and in the account. [here] is
/// where it is on this computer now, when that is somewhere else or matters.
/// [base] is the version the server is asked to build on, or the version
/// recorded. [files] are what go up, come down or are recorded, and [was]
/// the files at the last sync, which a merge starts from.
class PlanStep {
  final PlanAction action;
  final LibraryKind kind;
  final String? id;
  final String path;
  final String? here;
  final int base;
  final List<LibraryFile> files;
  final List<LibraryFile> was;
  const PlanStep(this.action, this.kind, this.path,
      {this.id,
      this.here,
      this.base = 0,
      this.files = const [],
      this.was = const []});

  @override
  String toString() =>
      '${action.name} ${kind.name}/$path${here != null ? ' from $here' : ''}'
      '${id != null ? ' #$id' : ''} @$base';
}

/// Every step that brings this computer and the account level.
///
/// [local] is what is here now, [journal] what was here at the last sync by
/// id, [remote] what the account holds by id. [kindsOn] are the kinds kept
/// on this computer. [onlyHere] are items never to go up, [removed] items
/// taken off this computer, [got] items fetched by Get although their kind is
/// off, [held] items in conflict that wait for the person, and [renamed]
/// items renamed here since the last sync, by id. [machine] names this
/// computer in a kept-both copy's name. With [foldCase], as on Windows and
/// macOS, two paths that differ only in case are one file here.
List<PlanStep> planLibrary({
  required Map<LibraryKey, List<LibraryFile>> local,
  required Map<String, LibraryItem> journal,
  required Map<String, LibraryItem> remote,
  required Set<LibraryKind> kindsOn,
  required String machine,
  Set<LibraryKey> onlyHere = const {},
  Set<String> removed = const {},
  Set<String> got = const {},
  Set<String> held = const {},
  Map<String, String> renamed = const {},
  bool foldCase = false,
}) {
  final steps = <PlanStep>[];
  bool kept(LibraryItem i) =>
      (kindsOn.contains(i.kind) || got.contains(i.id)) &&
      !removed.contains(i.id);
  String at(LibraryKind kind, String path) =>
      foldCase ? _fold((kind, path)) : '${kind.name}/$path';
  // The items here by the name this file system knows them by.
  final here = {for (final k in local.keys) at(k.$1, k.$2): k};
  // What the journal accounts for here is never taken for anything else.
  final claimed = {
    for (final j in journal.values)
      if (here.containsKey(at(j.kind, renamed[j.id] ?? j.path)))
        at(j.kind, renamed[j.id] ?? j.path)
  };

  /// Files the same on both sides: only the path can differ. A rename here
  /// wins over one there.
  void level(LibraryItem j, LibraryItem r, String lp, List<LibraryFile> l) {
    if (lp != j.path && lp != r.path) {
      steps.add(PlanStep(PlanAction.renameThere, j.kind, lp,
          id: j.id, base: r.n, files: l));
    } else if (lp != r.path) {
      steps.add(PlanStep(PlanAction.renameHere, j.kind, r.path,
          id: j.id, here: lp, base: r.n, files: l));
    } else if (r.n != j.n) {
      steps.add(PlanStep(PlanAction.record, j.kind, r.path,
          id: j.id, base: r.n, files: l));
    }
  }

  final ids = journal.keys.toList()..sort();
  for (final id in ids) {
    final j = journal[id]!;
    if (!kept(j) || held.contains(id)) continue;
    // Where it is here, spelt as the file is.
    final found = here[at(j.kind, renamed[id] ?? j.path)];
    var lp = found?.$2 ?? renamed[id] ?? j.path;
    var l = local[found];
    if (l == null && j.files.isNotEmpty) {
      // Gone from its place, and exactly its files are at a new one of the
      // same kind: moved here, which keeps its history.
      for (final MapEntry(:key, :value) in local.entries) {
        if (key.$1 == j.kind &&
            !claimed.contains(at(key.$1, key.$2)) &&
            sameFiles(value, j.files)) {
          claimed.add(at(key.$1, key.$2));
          (lp, l) = (key.$2, value);
          break;
        }
      }
    }
    final r = remote[id];
    final live = r != null && !r.gone;
    final rChanged = r == null || r.n != j.n;
    if (l == null) {
      if (!live) {
        steps.add(PlanStep(PlanAction.forget, j.kind, j.path, id: id));
      } else if (!rChanged) {
        steps.add(PlanStep(PlanAction.deleteThere, j.kind, j.path,
            id: id, base: j.n));
      } else {
        // A change there beats a delete here.
        steps.add(PlanStep(PlanAction.download, j.kind, r.path,
            id: id, base: r.n, files: r.files));
      }
      continue;
    }
    final lChanged = !sameFiles(l, j.files);
    if (!live) {
      steps.add(lChanged
          ? PlanStep(PlanAction.revive, j.kind, lp,
              id: id, base: r?.n ?? 0, files: l)
          : PlanStep(PlanAction.deleteHere, j.kind, lp, id: id, here: lp));
      continue;
    }
    if (!lChanged && !rChanged) {
      if (lp != j.path) {
        steps.add(PlanStep(PlanAction.renameThere, j.kind, lp,
            id: id, base: j.n, files: l));
      }
    } else if (lChanged && !rChanged) {
      steps.add(PlanStep(PlanAction.upload, j.kind, lp,
          id: id, here: lp, base: j.n, files: l));
    } else if (sameFiles(r.files, l)) {
      level(j, r, lp, l);
    } else if (!lChanged) {
      steps.add(PlanStep(PlanAction.download, j.kind, r.path,
          id: id, here: lp, base: r.n, files: r.files));
    } else {
      steps.add(PlanStep(
          j.kind == LibraryKind.exports
              ? PlanAction.merge
              : PlanAction.conflict,
          j.kind,
          r.path,
          id: id,
          here: lp,
          base: r.n,
          files: r.files,
          was: j.files));
    }
  }

  // New in the account: not in the journal, so never met here before.
  final arriving = <String, LibraryItem>{
    for (final r in remote.values)
      if (!r.gone && !journal.containsKey(r.id) && kept(r))
        at(r.kind, r.path): r
  };
  final taken = {
    for (final k in local.keys) _fold(k),
    for (final r in arriving.values) _fold((r.kind, r.path)),
  };
  final news = local.keys.toList()
    ..sort((a, b) => '${a.$1.name}/${a.$2}'.compareTo('${b.$1.name}/${b.$2}'));
  for (final key in news) {
    if (claimed.contains(at(key.$1, key.$2)) ||
        onlyHere.contains(key) ||
        !kindsOn.contains(key.$1)) {
      continue;
    }
    final files = local[key]!;
    final r = arriving.remove(at(key.$1, key.$2));
    if (r == null) {
      steps.add(PlanStep(PlanAction.upload, key.$1, key.$2,
          here: key.$2, files: files));
    } else if (sameFiles(r.files, files)) {
      // First meeting, and they are the same: nothing moves but the case of
      // its name, which takes the account's.
      steps.add(r.path == key.$2
          ? PlanStep(PlanAction.record, key.$1, key.$2,
              id: r.id, base: r.n, files: files)
          : PlanStep(PlanAction.renameHere, key.$1, r.path,
              id: r.id, here: key.$2, base: r.n, files: files));
    } else if (key.$1 == LibraryKind.exports) {
      // Export presets are one file the app reads by its name, so a copy set
      // aside would vanish from the app: they merge by entry instead.
      steps.add(PlanStep(PlanAction.merge, key.$1, r.path,
          id: r.id, here: key.$2, base: r.n, files: r.files));
    } else {
      // First meeting, and they differ: the one coming in keeps the path and
      // this one steps aside under this computer's name.
      final to = keepBothName(key.$1, key.$2, machine, taken);
      taken.add(_fold((key.$1, to)));
      steps
        ..add(PlanStep(PlanAction.keepBoth, key.$1, to,
            here: key.$2, files: files))
        ..add(PlanStep(PlanAction.download, key.$1, r.path,
            id: r.id, base: r.n, files: r.files));
    }
  }
  for (final r in arriving.values) {
    // A path taken here by something kept only here, or by something the
    // journal has, waits rather than writes over it.
    if (here.containsKey(at(r.kind, r.path))) continue;
    steps.add(PlanStep(PlanAction.download, r.kind, r.path,
        id: r.id, base: r.n, files: r.files));
  }

  final order = {for (final (i, s) in steps.indexed) s: i};
  steps.sort((a, b) {
    final by = a.action.index - b.action.index;
    return by != 0 ? by : order[a]! - order[b]!;
  });
  return guardPlan(steps, local, [...journal.values, ...remote.values],
      foldCase: foldCase);
}

String _fold(LibraryKey k) => '${k.$1.name}/${k.$2}'.toLowerCase();

/// [steps] with any that would take files away from this computer, when the
/// account does not hold them, turned into ones that keep them.
///
/// The planner should never make such a step. This is the last word on it
/// all the same: a download over an item whose files the account lacks
/// becomes a conflict, a delete here becomes a revival, and a rename onto an
/// item is dropped. Steps are followed in order, so an item a step before
/// has moved away is not counted as still there. With [foldCase] two paths
/// that differ only in case are one item, as the file system has them.
List<PlanStep> guardPlan(List<PlanStep> steps,
    Map<LibraryKey, List<LibraryFile>> local, Iterable<LibraryItem> known,
    {bool foldCase = false}) {
  final held = {
    for (final item in known)
      for (final f in item.files) f.hash
  };
  String key(LibraryKind kind, String path) =>
      foldCase ? _fold((kind, path)) : '${kind.name}/$path';
  // By the name the file system knows it by: its path as spelt, its files.
  final here = {
    for (final MapEntry(key: k, :value) in local.entries)
      key(k.$1, k.$2): (k.$2, value)
  };
  bool safe(String at) =>
      here[at]?.$2.every((f) => held.contains(f.hash)) ?? true;
  final out = <PlanStep>[];
  for (final s in steps) {
    final at = key(s.kind, s.path);
    final from = s.here == null ? null : key(s.kind, s.here!);
    switch (s.action) {
      case PlanAction.download:
        if (!safe(at) || (from != null && !safe(from))) {
          if (s.id != null) {
            out.add(PlanStep(PlanAction.conflict, s.kind, s.path,
                id: s.id,
                here: here[at]?.$1 ?? s.here,
                base: s.base,
                files: s.files));
          }
          continue;
        }
        if (from != null) here.remove(from);
        here[at] = (s.path, s.files);
      case PlanAction.deleteHere:
        if (!safe(at)) {
          out.add(PlanStep(PlanAction.revive, s.kind, here[at]!.$1,
              id: s.id, base: s.base, files: here[at]!.$2));
          continue;
        }
        here.remove(at);
      case PlanAction.renameHere || PlanAction.keepBoth:
        // Onto another item is dropped. A change of case alone is not.
        if (from == null || (at != from && here.containsKey(at))) continue;
        final moved = here.remove(from);
        if (moved != null) here[at] = (s.path, moved.$2);
      default:
    }
    out.add(s);
  }
  return out;
}

/// The name a local item steps aside to when another computer's item of the
/// same name comes in: `<name> (<machine>)<ext>`, numbered after the
/// brackets if that is taken too. [taken] holds `<kind>/<path>` in lower
/// case.
String keepBothName(
    LibraryKind kind, String path, String machine, Set<String> taken) {
  final cut = path.lastIndexOf('/');
  final dir = path.substring(0, cut + 1);
  final file = path.substring(cut + 1);
  final dot = file.lastIndexOf('.');
  final (stem, ext) =
      dot > 0 ? (file.substring(0, dot), file.substring(dot)) : (file, '');
  final who = machine.replaceAll(RegExp(r'[\\/:*?"<>|\x00-\x1f]'), '-').trim();
  final tag = who.isEmpty ? '' : ' ($who)';
  String named(int n) => '$dir$stem$tag${n > 1 || tag.isEmpty ? ' $n' : ''}$ext';
  var n = tag.isEmpty ? 2 : 1;
  while (taken.contains('${kind.name}/${named(n)}'.toLowerCase())) {
    n++;
  }
  return named(n);
}

/// [path], or the same numbered `<name> 2<ext>` and up when that is taken.
String freePath(LibraryKind kind, String path, Set<String> taken) =>
    taken.contains('${kind.name}/$path'.toLowerCase())
        ? keepBothName(kind, path, '', taken)
        : path;

/// The export presets as a three-way merge by each entry's `name`: an entry
/// changed on one side takes that side, changed on both takes this
/// computer's, and one deleted on one side and untouched on the other is
/// deleted. A change beats a delete. Null when either side is not a list.
String? mergeExports(String? base, String local, String remote) {
  Map<String, Object?>? byName(String? text) {
    try {
      final list = text == null ? const [] : jsonDecode(text);
      if (list is! List) return null;
      return {
        for (final e in list)
          if (e is Map && e['name'] is String) e['name'] as String: e
      };
    } catch (_) {
      return null;
    }
  }

  final b = byName(base) ?? const {};
  final l = byName(local), r = byName(remote);
  if (l == null || r == null) return null;
  String said(Object? v) => jsonEncode(v);
  final out = <Object?>[];
  for (final name in [...l.keys, ...r.keys.where((n) => !l.containsKey(n))]) {
    final was = b.containsKey(name) ? said(b[name]) : null;
    if (l.containsKey(name) && r.containsKey(name)) {
      out.add(said(l[name]) == was ? r[name] : l[name]);
    } else if (l.containsKey(name)) {
      if (said(l[name]) != was) out.add(l[name]);
    } else if (said(r[name]) != was) {
      out.add(r[name]);
    }
  }
  return const JsonEncoder.withIndent('  ').convert(out);
}
