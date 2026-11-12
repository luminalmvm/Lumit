// Low-spec mode: which machines are offered it, and that the switch and what
// it turned down are kept with the settings.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/low_spec_offer.dart';
import 'package:lumit_flutter/state/settings.dart';

void main() {
  const gib = 1 << 30;

  test('a 2 GB card or 8 GB of memory is offered Low-spec mode', () {
    expect(looksLowSpec(vram: 2 * gib, ram: 32 * gib), isTrue);
    expect(looksLowSpec(vram: 8 * gib, ram: 8 * gib), isTrue);
    expect(looksLowSpec(vram: 8 * gib, ram: 32 * gib), isFalse);
  });

  test('a size not known counts for nothing', () {
    expect(looksLowSpec(vram: 0, ram: 32 * gib), isFalse);
    expect(looksLowSpec(vram: 8 * gib, ram: 0), isFalse);
  });

  test('an older file has Low-spec mode off and not yet offered', () {
    final p = PerformanceSettings.fromJson(const {});
    expect(p.lowSpecMode, isFalse);
    expect(p.lowSpecOffered, isFalse);
    expect(p.lowSpecWas, isNull);
  });

  test('the switch and what it turned down round-trip', () {
    final p = PerformanceSettings(
      lowSpecMode: true,
      lowSpecOffered: true,
      lowSpecWas: {'animation': 'all', 'drag_limit': 0},
    );
    final back = PerformanceSettings.fromJson(p.toJson());
    expect(back.lowSpecMode, isTrue);
    expect(back.lowSpecOffered, isTrue);
    expect(back.lowSpecWas, {'animation': 'all', 'drag_limit': 0});
  });
}
