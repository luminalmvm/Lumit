// The user's own easing shapes: the ones they drew in the easing editor and
// kept (item R, docs/07 §5.4).
//
// In plain terms: the editor ships seven shapes. This is where the eighth one
// goes — the one somebody bent by hand and would like back tomorrow. A saved
// easing is a name and four numbers, nothing more, and applying one takes the
// same road a shipped preset takes.
//
// **It belongs to the person, not to the project.** A shape somebody likes is a
// working habit, like a favourite effect or a custom theme, and a project handed
// to somebody else has no business carrying the sender's collection of eases
// into their copy. So it lives beside the workspace store — the same folder the
// settings file and the project thumbnails are in, which means
// `Workspace.storeOverride` redirects it in a test for nothing. A file of its
// own rather than a key in the settings, because it is a list that grows on its
// own schedule and nothing else in the settings file wants to be rewritten when
// it does.

import 'dart:convert';
import 'dart:io';

import 'package:lumit_flutter/panels/easing_curve.dart';

import 'workspace.dart';

/// One shape the user kept: what they called it, and the curve.
class SavedEasing {
  final String name;
  final EasingCurve curve;

  const SavedEasing(this.name, this.curve);

  Map<String, dynamic> toJson() => {
        'name': name,
        'curve': [curve.x1, curve.y1, curve.x2, curve.y2],
      };

  /// Null for anything that is not a name and four numbers — a hand-edited
  /// file, or one written by a build that stored something else. One bad entry
  /// must not cost the user the rest of their collection.
  static SavedEasing? fromJson(Map<String, dynamic> j) {
    final name = j['name'];
    final c = j['curve'];
    if (name is! String || name.isEmpty) return null;
    if (c is! List || c.length != 4 || c.any((n) => n is! num)) return null;
    final n = c.cast<num>();
    return SavedEasing(
      name,
      EasingCurve(n[0].toDouble(), n[1].toDouble(), n[2].toDouble(),
          n[3].toDouble()),
    );
  }
}

/// The collection, read once and written on every change.
///
/// Static rather than a provided object because there is exactly one of it and
/// exactly one screen that shows it; the editor reads [all] when it is built
/// and rebuilds itself after each change it makes.
class CustomEasings {
  CustomEasings._();

  static List<SavedEasing> _list = [];
  static bool _loaded = false;

  /// `%APPDATA%\lumit\easings.json`, or the scratch folder in a test.
  static File storeFile() => File(
        '${Workspace.storeFile().parent.path}'
        '${Platform.pathSeparator}easings.json',
      );

  /// Every saved shape, in the order they were kept.
  static List<SavedEasing> get all {
    if (!_loaded) reload();
    return List.unmodifiable(_list);
  }

  /// Read the file again — at first use, and in a test that has just pointed
  /// the store somewhere else.
  static void reload() {
    _loaded = true;
    _list = [];
    try {
      final f = storeFile();
      if (!f.existsSync()) return;
      final j = jsonDecode(f.readAsStringSync());
      if (j is! List) return;
      for (final entry in j) {
        if (entry is Map) {
          final e = SavedEasing.fromJson(entry.cast<String, dynamic>());
          if (e != null) _list.add(e);
        }
      }
    } catch (_) {
      // A corrupt file reads as an empty collection — never a crash.
    }
  }

  /// Keep [curve] under [wanted], and return the name it landed under: two
  /// shapes cannot share a name, because the name is all there is to tell them
  /// apart in the row. Returns null when [wanted] is blank.
  static String? add(String wanted, EasingCurve curve) {
    final name = _free(wanted.trim());
    if (name == null) return null;
    _list = [..._list, SavedEasing(name, curve)];
    _write();
    return name;
  }

  /// Rename one, keeping its place in the row. Returns the name it now has, or
  /// null when [from] is not a saved shape or [to] is blank.
  static String? rename(String from, String to) {
    final at = _list.indexWhere((e) => e.name == from);
    if (at < 0) return null;
    final wanted = to.trim();
    if (wanted.isEmpty) return null;
    if (wanted == from) return from;
    final name = _free(wanted);
    if (name == null) return null;
    _list = [..._list]..[at] = SavedEasing(name, _list[at].curve);
    _write();
    return name;
  }

  /// Forget one.
  static void delete(String name) {
    _list = [..._list.where((e) => e.name != name)];
    _write();
  }

  /// [wanted] when no saved shape holds it, else the same with a number after
  /// it; null when it is blank. The same rule custom themes follow, so
  /// saving twice in a row never quietly overwrites the first attempt.
  static String? _free(String wanted) {
    if (wanted.isEmpty) return null;
    var tried = wanted;
    for (var n = 2; _list.any((e) => e.name == tried); n++) {
      tried = '$wanted $n';
    }
    return tried;
  }

  static void _write() {
    try {
      final f = storeFile();
      f.parent.createSync(recursive: true);
      f.writeAsStringSync(const JsonEncoder.withIndent('  ')
          .convert([for (final e in _list) e.toJson()]));
    } catch (_) {
      // Persistence is best-effort; the session keeps working without it.
    }
  }
}
