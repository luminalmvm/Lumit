// How the icon set is drawn, and where a person's own icons come from.
//
// Every glyph in the set is an SVG string drawn with strokes, so one weight
// applies to the whole set at once: the style's own (Desk engraves at 1.25,
// the others draw at 1.5), or the one the person picked in Settings. On top
// of that a folder of SVG files, one per icon and named after it
// (`pointer.svg`, `razor.svg`), replaces single icons or, dropped in whole,
// the lot. The folder lives beside the workspace store and is read once at
// launch and again from the Reload button.

import 'dart:io';

import 'package:lumit_flutter/theme/theme.dart';

/// The built-in weights a person can ask for.
enum IconSet {
  /// The style's own weight: Desk's 1.25, the others' 1.5.
  styleChoice,

  /// The set as drawn, 1.5.
  regular,

  /// Thin, cut lines at 1.25.
  engraved,

  /// Heavy lines at 2.
  bold,
}

abstract final class IconStyle {
  /// The weight a picked set draws at, or null for the style's own.
  static double? weight;

  /// The folder a person's own icons are read from, once known.
  static String? folder;

  static final Map<String, String> _overrides = {};

  /// Cached restyled glyphs, keyed by the source and the weight.
  static final Map<String, String> _restyled = {};

  /// The weight in force under `t`.
  static double weightFor(LumitTheme t) => weight ?? t.tokens.strokeWeight;

  static double? weightOf(IconSet set) => switch (set) {
        IconSet.styleChoice => null,
        IconSet.regular => 1.5,
        IconSet.engraved => 1.25,
        IconSet.bold => 2.0,
      };

  /// The SVG to draw for `name` (a [LumitIcon] member's name): the person's
  /// own file if one is in the folder, else the set's glyph at the weight in
  /// force.
  static String resolve(String name, String glyph, LumitTheme t) {
    final own = _overrides[name];
    if (own != null) return own;
    final w = weightFor(t);
    if (w == 1.5) return glyph;
    return _restyled.putIfAbsent(
        '$w|$glyph', () => glyph.replaceAll('stroke-width="1.5"', 'stroke-width="$w"'));
  }

  /// Read every `<name>.svg` in [folder]; a missing folder is no icons.
  static void reload() {
    _overrides.clear();
    final dir = folder;
    if (dir == null) return;
    final d = Directory(dir);
    if (!d.existsSync()) return;
    for (final entry in d.listSync()) {
      if (entry is! File || !entry.path.toLowerCase().endsWith('.svg')) {
        continue;
      }
      final name = entry.uri.pathSegments.last;
      final text = entry.readAsStringSync();
      if (text.contains('<svg')) {
        _overrides[name.substring(0, name.length - 4)] = text;
      }
    }
  }

  /// How many icons the folder replaces, for the Settings row.
  static int get overrideCount => _overrides.length;
}
