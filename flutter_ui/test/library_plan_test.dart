// The library sync's planner: every row of the design's table, and the rule
// that a sync never throws away files the account does not hold.

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/library_plan.dart';

const _p = LibraryKind.presets;

List<LibraryFile> _f(String hash) => [LibraryFile('', hash, 1)];

LibraryItem _i(String id, String path, int n, String hash,
        {bool gone = false, LibraryKind kind = _p}) =>
    LibraryItem(id, kind, path, n, _f(hash), gone: gone);

typedef _Row = ({
  String name,
  Map<LibraryKey, List<LibraryFile>> local,
  List<LibraryItem> journal,
  List<LibraryItem> remote,
  Set<LibraryKind>? kindsOn,
  Set<LibraryKey> onlyHere,
  Set<String> removed,
  Set<String> got,
  Set<String> held,
  Map<String, String> renamed,
  bool fold,
  List<String> want,
});

_Row _row(
  String name, {
  Map<LibraryKey, List<LibraryFile>> local = const {},
  List<LibraryItem> journal = const [],
  List<LibraryItem> remote = const [],
  Set<LibraryKind>? kindsOn,
  Set<LibraryKey> onlyHere = const {},
  Set<String> removed = const {},
  Set<String> got = const {},
  Set<String> held = const {},
  Map<String, String> renamed = const {},
  bool fold = false,
  required List<String> want,
}) =>
    (
      name: name,
      local: local,
      journal: journal,
      remote: remote,
      kindsOn: kindsOn,
      onlyHere: onlyHere,
      removed: removed,
      got: got,
      held: held,
      renamed: renamed,
      fold: fold,
      want: want,
    );

final _rows = <_Row>[
  _row('same, same: nothing',
      local: {(_p, 'A.lumfx'): _f('h1')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 1, 'h1')],
      want: []),
  _row('changed here: upload on the journal version',
      local: {(_p, 'A.lumfx'): _f('h2')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 1, 'h1')],
      want: ['upload presets/A.lumfx from A.lumfx #1 @1']),
  _row('changed there: download',
      local: {(_p, 'A.lumfx'): _f('h1')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 2, 'h2')],
      want: ['download presets/A.lumfx from A.lumfx #1 @2']),
  _row('changed both the same way: record',
      local: {(_p, 'A.lumfx'): _f('h2')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 2, 'h2')],
      want: ['record presets/A.lumfx #1 @2']),
  _row('changed both differently: conflict',
      local: {(_p, 'A.lumfx'): _f('h2')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 2, 'h3')],
      want: ['conflict presets/A.lumfx from A.lumfx #1 @2']),
  _row('deleted here, same there: delete there',
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 1, 'h1')],
      want: ['deleteThere presets/A.lumfx #1 @1']),
  _row('deleted here, changed there: the change wins',
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 2, 'h2')],
      want: ['download presets/A.lumfx #1 @2']),
  _row('same here, deleted there: to the trash here',
      local: {(_p, 'A.lumfx'): _f('h1')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 1, 'h1', gone: true)],
      want: ['deleteHere presets/A.lumfx from A.lumfx #1 @0']),
  _row('changed here, deleted there: revived',
      local: {(_p, 'A.lumfx'): _f('h2')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 1, 'h1', gone: true)],
      want: ['revive presets/A.lumfx #1 @1']),
  _row('deleted on both: forgotten',
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 1, 'h1', gone: true)],
      want: ['forget presets/A.lumfx #1 @0']),
  _row('new here: upload as a new item',
      local: {(_p, 'B.lumfx'): _f('h1')},
      want: ['upload presets/B.lumfx from B.lumfx @0']),
  _row('new there: download',
      remote: [_i('2', 'B.lumfx', 1, 'h1')],
      want: ['download presets/B.lumfx #2 @1']),
  _row('new there, kind off here: not on this computer',
      remote: [_i('2', 'B.lumfx', 1, 'h1')],
      kindsOn: {LibraryKind.themes},
      want: []),
  _row('new there, kind off here, fetched by Get',
      remote: [_i('2', 'B.lumfx', 1, 'h1')],
      kindsOn: {LibraryKind.themes},
      got: {'2'},
      want: ['download presets/B.lumfx #2 @1']),
  _row('kind off: nothing goes up',
      local: {(_p, 'B.lumfx'): _f('h1')},
      kindsOn: {LibraryKind.themes},
      want: []),
  _row('kind off: a synced item is left alone both ways',
      local: {(_p, 'A.lumfx'): _f('h2')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 2, 'h3')],
      kindsOn: {LibraryKind.themes},
      want: []),
  _row('first meeting, the same files: record',
      local: {(_p, 'B.lumfx'): _f('h1')},
      remote: [_i('2', 'B.lumfx', 4, 'h1')],
      want: ['record presets/B.lumfx #2 @4']),
  _row('first meeting, different files: keep both',
      local: {(_p, 'B.lumfx'): _f('h1')},
      remote: [_i('2', 'B.lumfx', 1, 'h2')],
      want: [
        'keepBoth presets/B (Studio).lumfx from B.lumfx @0',
        'download presets/B.lumfx #2 @1',
      ]),
  _row('moved here with the same files: a rename, history kept',
      local: {(_p, 'Glows/A.lumfx'): _f('h1')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 1, 'h1')],
      want: ['renameThere presets/Glows/A.lumfx #1 @1']),
  _row('moved here and changed: a new item and a delete',
      local: {(_p, 'Glows/A.lumfx'): _f('h2')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 1, 'h1')],
      want: [
        'deleteThere presets/A.lumfx #1 @1',
        'upload presets/Glows/A.lumfx from Glows/A.lumfx @0',
      ]),
  _row('renamed here with its name inside: upload under the new path',
      local: {(_p, 'Soft.lumfx'): _f('h2')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 1, 'h1')],
      renamed: {'1': 'Soft.lumfx'},
      want: ['upload presets/Soft.lumfx from Soft.lumfx #1 @1']),
  _row('renamed there: renamed here',
      local: {(_p, 'A.lumfx'): _f('h1')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'Glows/A.lumfx', 2, 'h1')],
      want: ['renameHere presets/Glows/A.lumfx from A.lumfx #1 @2']),
  _row('renamed and changed there: downloaded to the new path',
      local: {(_p, 'A.lumfx'): _f('h1')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'B.lumfx', 2, 'h2')],
      want: ['download presets/B.lumfx from A.lumfx #1 @2']),
  _row('export presets changed on both: merged',
      local: {(LibraryKind.exports, 'export-presets.json'): _f('h2')},
      journal: [
        _i('1', 'export-presets.json', 1, 'h1', kind: LibraryKind.exports)
      ],
      remote: [
        _i('1', 'export-presets.json', 2, 'h3', kind: LibraryKind.exports)
      ],
      want: [
        'merge exports/export-presets.json from export-presets.json #1 @2'
      ]),
  _row('export presets meeting for the first time: merged, not set aside',
      local: {(LibraryKind.exports, 'export-presets.json'): _f('h2')},
      remote: [
        _i('1', 'export-presets.json', 1, 'h3', kind: LibraryKind.exports)
      ],
      want: [
        'merge exports/export-presets.json from export-presets.json #1 @1'
      ]),
  _row('kept only here: never goes up',
      local: {(_p, 'B.lumfx'): _f('h1')},
      onlyHere: {(_p, 'B.lumfx')},
      want: []),
  _row('kept only here: one of the same name there is not written over it',
      local: {(_p, 'B.lumfx'): _f('h1')},
      remote: [_i('2', 'B.lumfx', 1, 'h2')],
      onlyHere: {(_p, 'B.lumfx')},
      want: []),
  _row('removed from this computer: not fetched again',
      remote: [_i('2', 'B.lumfx', 1, 'h1')],
      removed: {'2'},
      want: []),
  _row('in conflict: waits for the person',
      local: {(_p, 'A.lumfx'): _f('h2')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 3, 'h3')],
      held: {'1'},
      want: []),
  _row('an empty group folder is an item',
      local: {(_p, 'Empty'): const []},
      want: ['upload presets/Empty from Empty @0']),
  _row('case only, the same files: takes the account\'s case',
      local: {(_p, 'Looks/glow.lumfx'): _f('h1')},
      remote: [_i('2', 'Looks/Glow.lumfx', 1, 'h1')],
      fold: true,
      want: ['renameHere presets/Looks/Glow.lumfx from Looks/glow.lumfx #2 @1']),
  _row('case only, different files: keep both, never written over',
      local: {(_p, 'Looks/glow.lumfx'): _f('h1')},
      remote: [_i('2', 'Looks/Glow.lumfx', 1, 'h2')],
      fold: true,
      want: [
        'keepBoth presets/Looks/glow (Studio).lumfx from Looks/glow.lumfx @0',
        'download presets/Looks/Glow.lumfx #2 @1',
      ]),
  _row('case only where case counts: two items',
      local: {(_p, 'Looks/glow.lumfx'): _f('h1')},
      remote: [_i('2', 'Looks/Glow.lumfx', 1, 'h2')],
      want: [
        'download presets/Looks/Glow.lumfx #2 @1',
        'upload presets/Looks/glow.lumfx from Looks/glow.lumfx @0',
      ]),
  _row('renamed here in case only: renamed there',
      local: {(_p, 'a.lumfx'): _f('h1')},
      journal: [_i('1', 'A.lumfx', 1, 'h1')],
      remote: [_i('1', 'A.lumfx', 1, 'h1')],
      fold: true,
      want: ['renameThere presets/a.lumfx #1 @1']),
];

/// Whether any step takes files away from here that neither the journal nor
/// the account holds. Steps are followed in order.
String? _losesBytes(_Row row, List<PlanStep> steps) {
  final held = {
    for (final i in [...row.journal, ...row.remote])
      for (final f in i.files) f.hash
  };
  // Named as the file system names them: in one case, when it ignores case.
  String key(LibraryKind k, String path) =>
      row.fold ? '${k.name}/$path'.toLowerCase() : '${k.name}/$path';
  final here = {
    for (final MapEntry(key: k, :value) in row.local.entries)
      key(k.$1, k.$2): value
  };
  for (final s in steps) {
    final at = key(s.kind, s.path);
    final from = s.here == null ? null : key(s.kind, s.here!);
    bool loses(String k) =>
        here[k]?.any((f) => !held.contains(f.hash)) ?? false;
    switch (s.action) {
      case PlanAction.download || PlanAction.deleteHere:
        if (loses(at) || (from != null && loses(from))) return '$s';
        here.remove(from);
        if (s.action == PlanAction.download) {
          here[at] = s.files;
        } else {
          here.remove(at);
        }
      case PlanAction.renameHere || PlanAction.keepBoth:
        if (at != from && here.containsKey(at)) return '$s';
        final moved = here.remove(from);
        if (moved != null) here[at] = moved;
      default:
    }
  }
  return null;
}

void main() {
  for (final row in _rows) {
    test(row.name, () {
      final steps = planLibrary(
        local: row.local,
        journal: {for (final j in row.journal) j.id: j},
        remote: {for (final r in row.remote) r.id: r},
        kindsOn: row.kindsOn ?? LibraryKind.values.toSet(),
        machine: 'Studio',
        onlyHere: row.onlyHere,
        removed: row.removed,
        got: row.got,
        held: row.held,
        renamed: row.renamed,
        foldCase: row.fold,
      );
      expect([for (final s in steps) '$s'], row.want);
      expect(_losesBytes(row, steps), isNull);
    });
  }

  test('the guard keeps files the account does not hold', () {
    final local = {
      (_p, 'A.lumfx'): _f('mine'),
      (_p, 'B.lumfx'): _f('mine too'),
      (_p, 'C.lumfx'): _f('h1'),
    };
    final known = [_i('1', 'A.lumfx', 2, 'h2'), _i('3', 'C.lumfx', 1, 'h1')];
    final steps = guardPlan([
      PlanStep(PlanAction.download, _p, 'A.lumfx',
          id: '1', here: 'A.lumfx', base: 2, files: _f('h2')),
      const PlanStep(PlanAction.deleteHere, _p, 'B.lumfx',
          id: '2', here: 'B.lumfx', base: 1),
      const PlanStep(PlanAction.renameHere, _p, 'B.lumfx',
          id: '3', here: 'C.lumfx', base: 1),
      const PlanStep(PlanAction.deleteHere, _p, 'C.lumfx',
          id: '3', here: 'C.lumfx'),
    ], local, known);
    expect([for (final s in steps) '$s'], [
      'conflict presets/A.lumfx from A.lumfx #1 @2',
      'revive presets/B.lumfx #2 @1',
      'deleteHere presets/C.lumfx from C.lumfx #3 @0',
    ]);
  });

  test('keep-both and free names', () {
    final taken = {'presets/b (studio).lumfx', 'presets/a.lumfx'};
    expect(keepBothName(_p, 'B.lumfx', 'Studio', taken),
        'B (Studio) 2.lumfx');
    expect(keepBothName(_p, 'Glows/B.lumfx', 'Studio', taken),
        'Glows/B (Studio).lumfx');
    expect(keepBothName(LibraryKind.styles, 'Glass', 'Mac: Mini', {}),
        'Glass (Mac- Mini)');
    expect(freePath(_p, 'A.lumfx', taken), 'A 2.lumfx');
    expect(freePath(_p, 'New.lumfx', taken), 'New.lumfx');
  });

  test('export presets merge by name', () {
    String list(Map<String, int> e) => jsonEncode([
          for (final MapEntry(:key, :value) in e.entries)
            {'name': key, 'spec': value}
        ]);
    final merged = mergeExports(
      list({'a': 1, 'b': 1, 'c': 1, 'd': 1, 'f': 1}),
      // a changed on both, c deleted here, e added here, f changed here.
      list({'a': 2, 'b': 1, 'd': 1, 'e': 1, 'f': 2}),
      // a and b changed there, d and f deleted there, g added there.
      list({'a': 3, 'b': 2, 'c': 1, 'g': 1}),
    );
    expect(jsonDecode(merged!), [
      {'name': 'a', 'spec': 2},
      {'name': 'b', 'spec': 2},
      {'name': 'e', 'spec': 1},
      {'name': 'f', 'spec': 2},
      {'name': 'g', 'spec': 1},
    ]);
    expect(mergeExports(null, 'not json', '[]'), isNull);
  });
}
