// The localisation seam: choosing a language, falling back when a
// translation is missing, and surviving a settings file that names a language
// Lumit has never heard of.
//
// These do not check any particular translation — a translator's work is theirs
// to get right — only that the machinery around it cannot leave the interface
// blank, English-when-it-should-not-be, or unable to open.

import 'dart:ui' show Locale;

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';

void main() {
  // Every test in this file changes the global, so put it back afterwards or a
  // later test file in the same process inherits a German interface.
  tearDown(() => useLocale(const Locale('en')));

  test('a hand-edited settings file cannot stop Lumit opening', () {
    for (final tag in ['', 'not-a-language', 'zz_ZZ', '!!']) {
      expect(() => useLocale(localeFromTag(tag)), returnsNormally,
          reason: 'the tag $tag must resolve to something');
      expect(l10n.menuFile, isNotEmpty);
    }
  });

  test('every supported language names itself in the picker', () {
    // Without this, a language shipped without an endonym would show a blank
    // row — and somebody who had already chosen it could not read their way
    // back to English.
    for (final locale in Strings.supportedLocales) {
      expect(languageNames[localeTag(locale)], isNotNull,
          reason: 'no endonym for ${localeTag(locale)}');
    }
    expect(languageNames.keys.toSet(),
        Strings.supportedLocales.map(localeTag).toSet());
  });
}
