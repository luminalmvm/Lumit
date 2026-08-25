// A theme as a file you can send somebody (K-298).
//
// **In plain terms.** A custom theme normally lives inside the workspace file,
// which is machine-local: it never leaves the computer it was made on. This is
// the same theme written out on its own, so it can be posted, put in a repo, or
// carried to another machine — the keymap has had one of these since K-199, and
// a theme is the other thing in Lumit worth sharing.
//
// **The file is a small, readable JSON document**: a marker saying what it is, a
// version, the theme's name, its light-or-dark base, and the colours as
// `#rrggbb`. Readable because a theme is a thing people tinker with — the same
// reasoning that put hex rather than numbers in the workspace file. The colours
// are exactly `CustomTheme.toJson`'s, so the two forms never drift apart.
//
// **Reading is forgiving in one direction and strict in the other.** A file
// carrying colour keys this build has never heard of loads fine (they are
// ignored, and `applyTokens` takes the rest from the base), and so does one
// written by a newer Lumit — forward tolerance is the whole reason a theme is
// stored over a base rather than as a copy of the struct. What is refused is a
// file that is not a theme at all, and it is refused with a sentence rather
// than an exception, because picking the wrong file is a normal thing to do.

import 'dart:convert';

import 'custom_theme.dart';
import 'package:lumit_flutter/l10n/strings.dart';

/// The extension a shared theme is written under. Lumit's own, so the file
/// picker can offer just these and the system can associate them later.
const String themeFileExtension = 'lumtheme';

/// What the file says it is. Checked on read, so a stray `.json` renamed to
/// `.lumtheme` is refused rather than half-loaded.
const String themeFileFormat = 'lumit-theme';

/// The document version. Bumped only if the *shape* changes — adding colours
/// does not, because unknown keys are already ignored and missing ones already
/// fall back to the base.
const int themeFileVersion = 1;

/// The theme as the text of a file. Indented, and with a trailing newline, so
/// it reads well in an editor and diffs a line at a time.
String encodeThemeFile(CustomTheme theme) {
  const encoder = JsonEncoder.withIndent('  ');
  final document = <String, dynamic>{
    'format': themeFileFormat,
    'version': themeFileVersion,
    ...theme.toJson(),
  };
  return '${encoder.convert(document)}\n';
}

/// What reading a theme file came to: a theme, or a sentence saying why not.
/// Exactly one of the two is set.
class ThemeFileRead {
  final CustomTheme? theme;

  /// Why the file was refused, in the voice the settings page shows it in.
  final String? refusal;

  const ThemeFileRead.loaded(CustomTheme this.theme) : refusal = null;
  const ThemeFileRead.refused(String this.refusal) : theme = null;
}

/// Read a theme file's text. Never throws: a file that is not a theme comes
/// back as a refusal, because choosing the wrong file is a normal thing to do
/// and an editor should say so rather than fall over.
ThemeFileRead readThemeFile(String text) {
  Object? parsed;
  try {
    parsed = jsonDecode(text);
  } catch (_) {
    return ThemeFileRead.refused(l10n.themeFileNotATheme);
  }
  if (parsed is! Map) {
    return ThemeFileRead.refused(l10n.themeFileNotATheme);
  }
  final json = parsed.cast<String, dynamic>();
  // The marker is required when present and wrong, and forgiven when absent:
  // a theme lifted straight out of a workspace file has the same three fields
  // and no marker, and refusing that would be pedantry rather than safety.
  final format = json['format'];
  if (format is String && format != themeFileFormat) {
    return ThemeFileRead.refused(l10n.themeFileNotATheme);
  }
  final theme = CustomTheme.fromJson(json);
  if (theme == null) {
    return ThemeFileRead.refused(l10n.themeFileNoName);
  }
  if (theme.colours.isEmpty) {
    return ThemeFileRead.refused(l10n.themeFileNoColours);
  }
  return ThemeFileRead.loaded(theme);
}

/// The file name to suggest for [themeName]: its own name, lower case, with
/// anything a filesystem would rather not carry turned into a hyphen.
String themeFileName(String themeName) {
  final slug = themeName
      .trim()
      .toLowerCase()
      .replaceAll(RegExp('[^a-z0-9]+'), '-')
      .replaceAll(RegExp('^-+|-+\$'), '');
  return '${slug.isEmpty ? 'theme' : slug}.$themeFileExtension';
}
