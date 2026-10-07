// Ports of the Rust theme tests (crates/lumit-ui/src/theme.rs) so the Dart
// tables cannot silently drift from the Rust ones.

import 'dart:io';
import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';

int r(Color c) => (c.r * 255).round();
int g(Color c) => (c.g * 255).round();
int b(Color c) => (c.b * 255).round();

void main() {
  test(
      'scheme mode matches built theme and is light for the light schemes only',
      () {
    const lightSchemes = [
      LumitColorScheme.light,
      LumitColorScheme.gruvboxLight,
      LumitColorScheme.catppuccinLatte,
      LumitColorScheme.mallowLight,
      LumitColorScheme.glacierLight,
      LumitColorScheme.hearthLight,
      LumitColorScheme.chalkLight,
      LumitColorScheme.vellumLight,
      LumitColorScheme.neonLight,
      LumitColorScheme.canopyLight,
      LumitColorScheme.slateLight,
      LumitColorScheme.nocturneLight,
      LumitColorScheme.tavernLight,
      LumitColorScheme.arcaneLight,
      LumitColorScheme.giltLight,
      LumitColorScheme.greyRoom,
    ];
    // Seven from the Rust frontend, twelve pairs, and Desk's two rooms.
    expect(LumitColorScheme.values, hasLength(7 + 12 * 2 + 2));
    for (final scheme in LumitColorScheme.values) {
      expect(scheme.build().mode, scheme.mode);
      expect(
        scheme.mode,
        lightSchemes.contains(scheme) ? ThemeMode2.light : ThemeMode2.dark,
        reason: 'wrong mode for $scheme',
      );
    }
  });

  test('dark scheme viewer surround is exactly neutral (r == g == b)', () {
    for (final scheme in [
      LumitColorScheme.dark,
      LumitColorScheme.darkBlue,
      LumitColorScheme.gruvboxDark,
      LumitColorScheme.catppuccinMocha,
      LumitColorScheme.mallowDark,
      LumitColorScheme.glacierDark,
      LumitColorScheme.hearthDark,
      LumitColorScheme.chalkDark,
      LumitColorScheme.vellumDark,
      LumitColorScheme.neonDark,
      LumitColorScheme.canopyDark,
      LumitColorScheme.slateDark,
      LumitColorScheme.nocturneDark,
      LumitColorScheme.tavernDark,
      LumitColorScheme.arcaneDark,
      LumitColorScheme.giltDark,
      LumitColorScheme.graphite,
    ]) {
      final c = scheme.build().viewerSurround;
      expect(g(c), r(c), reason: '$scheme viewer surround not neutral');
      expect(b(c), r(c), reason: '$scheme viewer surround not neutral');
    }
  });

  test('both faces are named by the theme and bundled by pubspec', () {
    // A family the theme asks for but pubspec never declares renders as the
    // platform default and nothing complains, so pin the pair together.
    expect(LumitTheme.fontFamily, 'Hanken Grotesk');
    expect(LumitTheme.dark().mono.fontFamily, 'Geist Mono');
    final pubspec = File('pubspec.yaml').readAsStringSync();
    expect(pubspec, contains('family: Hanken Grotesk'));
    expect(pubspec, contains('family: Geist Mono'));
  });
}
