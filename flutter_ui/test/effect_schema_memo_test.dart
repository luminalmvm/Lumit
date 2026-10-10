// What the effect cards remember of the catalogue while the plugin scan is
// still running. A project opened before the scan reached a plugin left it
// as `OFX:ORG.SPEKTRAFILM` with no rows for the rest of the session. Pure, so
// the engine's three reads are stood in for.

import 'dart:async';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

const _plugin = 'ofx:org.spektrafilm';

BridgeEffectInfo _entry(String name) => BridgeEffectInfo(
      name: name,
      label: name,
      category: '',
      categoryLabel: '',
      namespace: '',
      inputs: const [],
      outputs: const [],
    );

/// A catalogue the scan can add [_plugin] to, counting what is read from it.
class _Catalogue {
  bool found = false;
  int reads = 0;

  late final memo = EffectSchemaMemo(
    readEffects: () {
      reads++;
      return [_entry('blur'), if (found) _entry(_plugin)];
    },
    readParameters: (effect) {
      reads++;
      return [
        if (effect == 'blur' || found)
          const BridgeParamInfo(
            id: 'amount',
            label: 'Amount',
            kind: BridgeParamKind.seed(),
            unit: BridgeUnit.raw,
            derived: false,
          ),
      ];
    },
    readGroups: (effect) {
      reads++;
      return [
        if (effect == _plugin && found)
          BridgeParamGroup(
            label: 'Film',
            params: const ['amount'],
            collapsed: false,
            visibleWhenValues: Uint32List(0),
          ),
      ];
    },
  );

  bool listed(String effect) =>
      memo.effectsFor(effect).any((info) => info.name == effect);
}

void main() {
  test('a plugin asked for before the scan reaches it is found afterwards',
      () async {
    final catalogue = _Catalogue();
    final memo = catalogue.memo;
    final scan = Completer<void>();
    final scanned = memo.scanned(() => scan.future);

    // The project is opened straight away, with the plugin on a layer.
    expect(catalogue.listed('blur'), isTrue);
    expect(catalogue.listed(_plugin), isFalse);
    expect(memo.parameters(_plugin), isEmpty);
    expect(memo.groups(_plugin), isEmpty);

    // The scan reaches it, and the cards have it without waiting for the end.
    catalogue.found = true;
    expect(catalogue.listed(_plugin), isTrue);
    expect(memo.parameters(_plugin), hasLength(1));
    expect(memo.groups(_plugin), hasLength(1));

    scan.complete();
    await scanned;
    expect(catalogue.listed(_plugin), isTrue);
    expect(memo.parameters(_plugin), hasLength(1));
    expect(memo.groups(_plugin), hasLength(1));
  });

  test('a built-in card drawn during the scan does not hide the plugins',
      () async {
    final catalogue = _Catalogue();
    final scan = Completer<void>();
    final scanned = catalogue.memo.scanned(() => scan.future);
    expect(catalogue.listed('blur'), isTrue);

    catalogue.found = true;
    scan.complete();
    await scanned;
    expect(catalogue.listed(_plugin), isTrue);
  });

  test('each answer is read once, and a built-in once even during the scan',
      () async {
    final catalogue = _Catalogue()..found = true;
    final memo = catalogue.memo;
    final scan = Completer<void>();
    final scanned = memo.scanned(() => scan.future);

    void ask(String effect) {
      catalogue.listed(effect);
      memo.parameters(effect);
      memo.groups(effect);
    }

    ask('blur');
    final once = catalogue.reads;
    ask('blur');
    expect(catalogue.reads, once);

    scan.complete();
    await scanned;
    ask('blur');
    ask(_plugin);
    final settled = catalogue.reads;
    ask('blur');
    ask(_plugin);
    expect(catalogue.reads, settled);
  });
}
