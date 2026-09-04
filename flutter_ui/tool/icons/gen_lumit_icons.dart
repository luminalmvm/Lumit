// Turns tool/icons/glyphs.json into lib/icons/lumit_icons.dart.
//
// Run from flutter_ui/:  dart run tool/icons/gen_lumit_icons.dart
//
// A glyph is edited in the JSON — one line, the inside of a 16x16 <svg> — and
// this script wraps each body in the one document shape the set is drawn to
// (docs/15-DESIGN.md §5) and gives it a camelCase constant so a call
// site cannot misspell a name.

import 'dart:convert';
import 'dart:io';

const _input = 'tool/icons/glyphs.json';
const _output = 'lib/icons/lumit_icons.dart';

/// The one document every glyph is wrapped in: the set's grammar, in one place.
String _document(String body) => '<svg viewBox="0 0 16 16" fill="none" '
    'stroke="currentColor" stroke-width="1.5" stroke-linecap="round" '
    'stroke-linejoin="round">$body</svg>';

/// 'Direct select' -> 'directSelect'. A glyph whose name cannot become an
/// identifier (it starts with a digit) carries an explicit "id" in the JSON.
String _camel(String name) {
  final words = name
      .split(RegExp(r'[^A-Za-z0-9]+'))
      .where((w) => w.isNotEmpty)
      .toList();
  final head = words.first.toLowerCase();
  final tail = words
      .skip(1)
      .map((w) => w[0].toUpperCase() + w.substring(1).toLowerCase());
  return [head, ...tail].join();
}

String _dartString(String s) {
  final escaped =
      s.replaceAll(r'\', r'\\').replaceAll("'", r"\'").replaceAll(r'$', r'\$');
  return "'$escaped'";
}

void main() {
  final file = File(_input);
  if (!file.existsSync()) {
    stderr.writeln('$_input not found — run this from flutter_ui/.');
    exit(1);
  }
  final data = jsonDecode(file.readAsStringSync()) as Map<String, dynamic>;
  final sections = (data['sections'] as List).cast<Map<String, dynamic>>();

  final out = StringBuffer()
    ..writeln('// GENERATED FILE — do not edit by hand.')
    ..writeln('//')
    ..writeln('// Written by tool/icons/gen_lumit_icons.dart from')
    ..writeln('// tool/icons/glyphs.json. To change a glyph, edit its one line')
    ..writeln('// in that file and run:')
    ..writeln('//')
    ..writeln('//   dart run tool/icons/gen_lumit_icons.dart')
    ..writeln('//')
    ..writeln('// The set is Lumit\'s own (docs/15-DESIGN.md §5): a 16px')
    ..writeln('// grid, a 1.5px stroke, round caps, one weight, and')
    ..writeln('// currentColor throughout, so a glyph takes the text colour of')
    ..writeln('// wherever it sits. LumitIcon (lib/icons/lumit_icon.dart) draws')
    ..writeln('// one.')
    ..writeln()
    ..writeln('abstract final class LumitIcons {');

  // 'byName' is the map's own identifier: no glyph may claim it.
  final ids = <String, String>{'byName': 'the name map'};
  final entries = <String, String>{};
  for (final section in sections) {
    out
      ..writeln()
      ..writeln('  // --- ${section['name']} ---');
    for (final glyph in (section['glyphs'] as List).cast<Map<String, dynamic>>()) {
      final name = glyph['name'] as String;
      final id = glyph['id'] as String? ?? _camel(name);
      if (ids.containsKey(id)) {
        stderr.writeln('two glyphs share the identifier $id: '
            '${ids[id]} and $name');
        exit(1);
      }
      if (entries.containsKey(name)) {
        stderr.writeln('two glyphs share the name $name');
        exit(1);
      }
      ids[id] = name;
      entries[name] = id;
      final comment = glyph['comment'] as String?;
      if (comment != null) out.writeln('  /// $comment');
      out.writeln(
          '  static const String $id =\n      ${_dartString(_document(glyph['body'] as String))};');
    }
  }

  out
    ..writeln()
    ..writeln('  /// Every glyph by its chrome word — the names tooltips and')
    ..writeln('  /// the Words setting use (docs/15-DESIGN.md §5.1).')
    ..writeln('  static const Map<String, String> byName = <String, String>{');
  entries.forEach((name, id) => out.writeln("    ${_dartString(name)}: $id,"));
  out
    ..writeln('  };')
    ..writeln('}');

  File(_output).writeAsStringSync(out.toString());
  stdout.writeln('${entries.length} glyphs written to $_output');
}
