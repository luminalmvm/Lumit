/// In plain terms: this file is the one place the rest of the application asks
/// for a piece of text. Instead of writing `'Import footage'` in a button, code
/// writes `l10n.importFootage`, and `l10n` hands back that phrase in whichever
/// language the user has chosen. The phrases themselves live in `app_en.arb`
/// (English, written by hand) and in the `app_*.arb` files beside it, which come
/// back from Crowdin (K-303).
///
/// Why a plain global rather than the usual Flutter `Strings.of(context)`: a
/// good third of Lumit's text is decided outside a widget — in `state/keymap.dart`,
/// in the tool table, in the settings model — where there is no `BuildContext`
/// to ask. Threading one through all of it would be a large change for no gain,
/// because Lumit has exactly one window's worth of language at a time: there is
/// no case where two halves of the screen want different languages.
///
/// The cost of that shortcut is that changing the language does not, by itself,
/// repaint anything. It does not need to: [useLocale] is only ever called from
/// the settings model, whose `notifyListeners` already rebuilds the whole shell
/// (see `LumitAppNew` in main.dart), so the new text lands on the same frame.
library;

import 'dart:ui' show Locale, PlatformDispatcher;

import 'package:lumit_flutter/l10n/gen/app_localizations.dart';
import 'package:lumit_flutter/l10n/gen/app_localizations_en.dart';

export 'package:lumit_flutter/l10n/gen/app_localizations.dart' show Strings;

/// The current language's strings.
///
/// English until [useLocale] says otherwise, and never null — a test that pumps
/// a widget without touching localisation gets the English text rather than a
/// crash, which is what every existing test expects.
Strings l10n = StringsEn();

/// Switch the application to [locale].
///
/// Falls back to English for a locale nothing has been translated into yet, so
/// an unknown value in a settings file (or a machine set to a language Lumit has
/// never heard of) opens in English rather than failing to start.
void useLocale(Locale locale) {
  final match = resolveLocale(locale);
  // `Strings.delegate.load` returns a `SynchronousFuture` — the generated
  // classes hold constant strings, so there is nothing to wait for and the
  // callback below runs before this function returns.
  Strings.delegate.load(match).then((loaded) => l10n = loaded);
}

/// The supported locale closest to [wanted]: an exact match, else the same
/// language in another script or country, else English.
Locale resolveLocale(Locale wanted) {
  for (final l in Strings.supportedLocales) {
    if (l == wanted) return l;
  }
  for (final l in Strings.supportedLocales) {
    if (l.languageCode == wanted.languageCode) return l;
  }
  return const Locale('en');
}

/// The language tag Lumit would use with no setting saved — what the machine
/// itself is set to, narrowed to something Lumit has strings for.
Locale systemLocale() => resolveLocale(PlatformDispatcher.instance.locale);

/// The BCP-47 tag for [locale], as the settings file stores it (`en`, `de`,
/// `zh-Hans`). The inverse of [localeFromTag].
String localeTag(Locale locale) => locale.scriptCode != null
    ? '${locale.languageCode}-${locale.scriptCode}'
    : locale.languageCode;

/// The locale a saved tag names. Anything unrecognised resolves to English via
/// [resolveLocale], so a hand-edited settings file cannot stop Lumit opening.
Locale localeFromTag(String tag) {
  // A `Locale` refuses an empty language code outright, so a settings file with
  // `"language": ""` in it would take the application down on the way up rather
  // than falling back. Anything shapeless is English.
  final parts =
      tag.split(RegExp('[-_]')).where((p) => p.trim().isNotEmpty).toList();
  if (parts.isEmpty) return const Locale('en');
  final locale = parts.length > 1
      ? Locale.fromSubtags(languageCode: parts[0], scriptCode: parts[1])
      : Locale(parts[0]);
  return resolveLocale(locale);
}

/// What each supported language calls itself, for the Settings → Interface
/// picker. Endonyms rather than English names, because the person who needs to
/// find "Қазақша" in that list is not reading the word "Kazakh".
///
/// Deliberately not in the .arb: these must read the same whatever language the
/// interface is currently in, or someone who has set Lumit to a language they
/// cannot read has no way back.
const Map<String, String> languageNames = {
  'en': 'English',
  'de': 'Deutsch',
  'kk': 'Қазақша',
  'uk': 'Українська',
  'zh': '简体中文',
  'zh-Hant': '繁體中文',
};
