// Where a comp's layer groups land on its rows (K-702).
//
// `groupFolds` is the whole of the Timeline's own grouping logic: which layer
// carries a group's header row, and which layers a shut fold takes off the
// list. Everything else about a group is the engine's — the run it draws over,
// its combined span, its switch faces — so this is what there is to check, and
// it is pure, which is why it is checked here rather than by clicking in a
// widget tree (the rule timeline_rows_test.dart follows).

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_metrics_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:uuid/uuid.dart';

/// Four stand-in layer ids, in stack order.
final _layers = [
  for (var i = 1; i <= 4; i++)
    UuidValue.fromString('0000000$i-0000-4000-8000-000000000000'),
];

BridgeLayerGroup _group(String id, List<UuidValue> members) => BridgeLayerGroup(
      id: UuidValue.fromString(id),
      name: 'Titles',
      label: 0,
      members: members,
      inFrame: 0,
      outFrame: 100,
      visible: true,
      audible: true,
      solo: false,
      locked: false,
    );

const _gid = 'aaaaaaaa-0000-4000-8000-000000000000';
const _gid2 = 'bbbbbbbb-0000-4000-8000-000000000000';

void main() {
  group('A group hangs its header on its topmost member', () {
    test('an open group hides nothing and heads its first member', () {
      final folds = groupFolds(
        groups: [_group(_gid, [_layers[1], _layers[2]])],
        folded: const {},
      );
      expect(folds.headers.keys, [_layers[1].toString()]);
      expect(folds.headers[_layers[1].toString()]!.folded, isFalse);
      expect(folds.hidden, isEmpty,
          reason: 'an open group takes no row off the list');
    });

    test('a shut group hides every member but the one carrying the header', () {
      final folds = groupFolds(
        groups: [
          _group(_gid, [_layers[1], _layers[2], _layers[3]])
        ],
        folded: {_gid},
      );
      // The carrier stays in the row list — it is what the header is drawn on —
      // and its own body stands down instead.
      expect(folds.headers.keys, [_layers[1].toString()]);
      expect(folds.headers[_layers[1].toString()]!.folded, isTrue);
      expect(folds.hidden,
          {_layers[2].toString(), _layers[3].toString()});
      expect(folds.hidden, isNot(contains(_layers[1].toString())));
    });

    test('a layer outside the group is never hidden', () {
      final folds = groupFolds(
        groups: [_group(_gid, [_layers[1], _layers[2]])],
        folded: {_gid},
      );
      expect(folds.hidden, isNot(contains(_layers[0].toString())));
      expect(folds.hidden, isNot(contains(_layers[3].toString())));
    });

    test('two groups fold independently', () {
      final folds = groupFolds(
        groups: [
          _group(_gid, [_layers[0], _layers[1]]),
          _group(_gid2, [_layers[2], _layers[3]]),
        ],
        folded: {_gid2},
      );
      expect(folds.headers.length, 2);
      expect(folds.headers[_layers[0].toString()]!.folded, isFalse);
      expect(folds.headers[_layers[2].toString()]!.folded, isTrue);
      expect(folds.hidden, {_layers[3].toString()});
    });

    test('a group whose layers are all gone draws no row at all', () {
      // The engine answers an empty member list for a group nothing resolves
      // to; there is no row to hang a header on, and nothing to hide.
      final folds = groupFolds(
        groups: [_group(_gid, const [])],
        folded: {_gid},
      );
      expect(folds.headers, isEmpty);
      expect(folds.hidden, isEmpty);
    });
  });
}
