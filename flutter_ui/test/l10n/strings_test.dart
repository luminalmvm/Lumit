// The localisation seam (K-303): choosing a language, falling back when a
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

  test('English is what is loaded before anything asks', () {
    expect(l10n.menuFile, 'File');
  });

  test('choosing a language takes effect on the next read', () {
    useLocale(const Locale('de'));
    // Nothing is translated into German yet, so this proves the fallback rather
    // than the translation — which is the behaviour that matters while the work
    // is in progress on Crowdin.
    expect(l10n.menuFile, isNotEmpty);
  });

  test('a language nobody has translated into opens in English', () {
    useLocale(const Locale('ja'));
    expect(l10n.menuFile, 'File');
  });

  test('a hand-edited settings file cannot stop Lumit opening', () {
    for (final tag in ['', 'not-a-language', 'zz_ZZ', '!!']) {
      expect(() => useLocale(localeFromTag(tag)), returnsNormally,
          reason: 'the tag $tag must resolve to something');
      expect(l10n.menuFile, isNotEmpty);
    }
  });

  test('a locale resolves to its own language before it falls back', () {
    // A machine set to Swiss German gets the German strings, not English.
    expect(resolveLocale(const Locale('de', 'CH')).languageCode, 'de');
    expect(resolveLocale(const Locale('en', 'AU')).languageCode, 'en');
  });

  test('a tag survives the trip through the settings file', () {
    for (final locale in Strings.supportedLocales) {
      expect(localeFromTag(localeTag(locale)), locale);
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
