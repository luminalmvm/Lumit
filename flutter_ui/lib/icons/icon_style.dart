// How the icon set is drawn, and where a person's own icons come from.
//
// Every glyph in the set is an SVG string drawn with strokes, so one weight
// applies to the whole set at once: the style's own (Desk engraves at 1.25,
// the others draw at 1.5), or the one the person picked in Settings. On top
// of that a folder of SVG files, one per icon and named after it
// (`pointer.svg`, `razor.svg`), replaces single icons or, dropped in whole,
// the lot. The folder lives beside the workspace store and is read once at
// launch and again from the Reload button.
//
// A file may be named after the glyph (`direct-select.svg`, `directSelect.svg`)
// or after the older icon name (`pointer.svg`). Case and punctuation are
// ignored, so all of those find the same icon.

import 'dart:io';

import 'package:flutter/foundation.dart' show mapEquals;
import 'package:lumit_flutter/theme/theme.dart';

import 'icons.dart' show LumitIcon, glyphOf;
import 'lumit_icons.dart';

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

  /// A name with case and punctuation dropped: `Direct select`,
  /// `direct-select` and `directSelect` are one icon.
  static String _key(String name) =>
      name.toLowerCase().replaceAll(RegExp('[^a-z0-9]'), '');

  /// Each glyph's name, by the glyph. The set's strings are constants, so the
  /// one a call site passes is the one in this map.
  static final Map<String, String> _names = {
    for (final e in LumitIcons.byName.entries) e.value: _key(e.key),
  };

  /// The person's own file for the icon called `name`, if there is one. For
  /// the few marks that are painted rather than drawn from the set.
  static String? own(String name) => _overrides[_key(name)];

  /// The SVG to draw for `glyph`, one of the [LumitIcons]: the person's own
  /// file if one is in the folder, else the glyph at the weight in force.
  static String resolve(String glyph, LumitTheme t) {
    final name = _names[glyph];
    final own = name == null ? null : _overrides[name];
    if (own != null) return own;
    final w = weightFor(t);
    if (w == 1.5) return glyph;
    return _restyled.putIfAbsent(
        '$w|$glyph', () => glyph.replaceAll('stroke-width="1.5"', 'stroke-width="$w"'));
  }

  /// Read every `<name>.svg` in [folder]; a missing folder is no icons.
  /// Whether anything changed, so a redraw happens only when it has to.
  static bool reload() {
    final before = Map.of(_overrides);
    _overrides.clear();
    // Weakest first, so the person's own folder wins over a set, and a set
    // over the style's icons.
    for (final dir in [...under, if (folder case final own?) own]) {
      _overrides.addAll(_read(dir));
    }
    return !mapEquals(before, _overrides);
  }

  /// Folders read under the person's own, weakest first: the style's icons,
  /// then an icon set an extension carries.
  static List<String> under = const [];

  static Map<String, String> _read(String dir) {
    final found = <String, String>{};
    final d = Directory(dir);
    if (!d.existsSync()) return found;
    for (final entry in d.listSync()) {
      if (entry is! File || !entry.path.toLowerCase().endsWith('.svg')) {
        continue;
      }
      final name = entry.uri.pathSegments.last;
      final text = entry.readAsStringSync();
      if (text.contains('<svg')) {
        found[_key(name.substring(0, name.length - 4))] = text;
      }
    }
    // A file under the older name stands in for its glyph, unless the glyph
    // has a file of its own.
    for (final icon in LumitIcon.values) {
      final own = found[_key(icon.name)];
      final glyph = glyphOf(icon);
      final name = glyph == null ? null : _names[glyph];
      if (own != null && name != null) found.putIfAbsent(name, () => own);
    }
    return found;
  }

  /// How many icons the folder replaces, for the Settings row.
  static int get overrideCount => _overrides.length;
}
