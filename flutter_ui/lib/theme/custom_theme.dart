// A theme the user made (K-202).
//
// A custom theme is a **name, a light-or-dark base, and a bag of colours** —
// not a copy of `LumitTheme`. The base matters because a theme is more than
// its colours: the mode decides which way hover shifts, whether the light or
// dark ramp underlies anything the user did not touch, and what a new token
// added in a later build defaults to. Storing the colours *over* a base is
// what lets a theme saved today still open when Lumit grows a token tomorrow.

import 'package:flutter/widgets.dart';

import 'theme.dart';
import 'theme_tokens.dart';

/// A user-made theme, as saved in the workspace file.
class CustomTheme {
  /// What the user called it. Also its identity — the dropdown shows it and
  /// the workspace stores the selection by it, so two cannot share one.
  final String name;

  /// Light or dark. Decides the base every unset colour comes from, and the
  /// behaviour (hover direction, defaulted tokens) that is not a colour.
  final ThemeMode2 mode;

  /// Token key → colour. Only what this theme carries; anything absent falls
  /// to the base.
  final Map<String, Color> colours;

  const CustomTheme({
    required this.name,
    required this.mode,
    required this.colours,
  });

  /// The scheme a custom theme is built over. One per mode, and deliberately
  /// the plainest of each: a custom theme should start from Lumit's own
  /// neutral ramp, not from somebody else's palette.
  LumitColorScheme get baseScheme =>
      mode == ThemeMode2.light ? LumitColorScheme.light : LumitColorScheme.dark;

  /// This theme, resolved: the base scheme under the shape, with the stored
  /// colours over it.
  LumitTheme build(ThemeShape shape) => applyTokens(
        baseScheme.build().copyWith(shape: shape, tokens: ShapeTokens.of(shape)),
        colours,
      );

  /// The same theme under a new name — what "save as" does.
  CustomTheme renamed(String to) =>
      CustomTheme(name: to, mode: mode, colours: colours);

  /// Capture a whole theme's colours under [name]. What the editor saves.
  factory CustomTheme.from(String name, LumitTheme theme) => CustomTheme(
        name: name,
        mode: theme.mode,
        colours: tokensOf(theme),
      );

  Map<String, dynamic> toJson() => {
        'name': name,
        'mode': mode.name,
        // Stored as `#rrggbb` rather than a number: a person opening the
        // workspace file should be able to read their own theme, and paste a
        // colour into it.
        'colours': {
          for (final e in colours.entries) e.key: _hex(e.value),
        },
      };

  static CustomTheme? fromJson(Map<String, dynamic> json) {
    final name = json['name'];
    if (name is! String || name.trim().isEmpty) return null;
    final mode = ThemeMode2.values.asNameMap()[json['mode']] ?? ThemeMode2.dark;
    final raw = json['colours'];
    final colours = <String, Color>{};
    if (raw is Map) {
      raw.forEach((key, value) {
        if (key is! String || value is! String) return;
        final colour = _parseHex(value);
        if (colour != null) colours[key] = colour;
      });
    }
    return CustomTheme(name: name, mode: mode, colours: colours);
  }
}

String _hex(Color c) {
  String two(double v) =>
      (v * 255).round().clamp(0, 255).toRadixString(16).padLeft(2, '0');
  return '#${two(c.r)}${two(c.g)}${two(c.b)}';
}

Color? _parseHex(String text) {
  final t = text.trim().replaceFirst('#', '');
  if (t.length != 6) return null;
  final value = int.tryParse(t, radix: 16);
  return value == null ? null : Color(0xff000000 | value);
}
