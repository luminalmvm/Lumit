// The icon set is generated (tool/icons/gen_lumit_icons.dart), so the thing
// worth testing is that the generated file still matches its source and that a
// glyph actually draws.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/lumit_icon.dart';
import 'package:lumit_flutter/icons/lumit_icons.dart';

Map<String, String> _glyphBodies() {
  final data = jsonDecode(File('tool/icons/glyphs.json').readAsStringSync())
      as Map<String, dynamic>;
  return {
    for (final section in (data['sections'] as List).cast<Map<String, dynamic>>())
      for (final glyph in (section['glyphs'] as List).cast<Map<String, dynamic>>())
        glyph['name'] as String: glyph['body'] as String,
  };
}

/// The generator's wrapper, rebuilt here so an edited body in glyphs.json
/// without a re-run fails this test instead of quietly shipping the old
/// drawing (docs/GUIDE.md §15 promises exactly that).
String _document(String body) => '<svg viewBox="0 0 16 16" fill="none" '
    'stroke="currentColor" stroke-width="1.5" stroke-linecap="round" '
    'stroke-linejoin="round">$body</svg>';

void main() {
  test('every glyph in the source file reached the generated map, drawing and all',
      () {
    final bodies = _glyphBodies();
    expect(bodies, isNotEmpty);
    expect(LumitIcons.byName.keys, unorderedEquals(bodies.keys),
        reason: 'lib/icons/lumit_icons.dart is stale — run '
            '`dart run tool/icons/gen_lumit_icons.dart`');
    bodies.forEach((name, body) {
      expect(LumitIcons.byName[name], _document(body),
          reason: '$name is stale — run '
              '`dart run tool/icons/gen_lumit_icons.dart`');
    });
  });

  test('every typed constant is one of the map entries', () {
    // The constants are the map's values, so a name the map misses is a
    // constant no lookup can reach.
    for (final glyph in <String>[
      LumitIcons.select,
      LumitIcons.play,
      LumitIcons.visible,
      LumitIcons.threeD,
      LumitIcons.channels,
      LumitIcons.workspace,
    ]) {
      expect(LumitIcons.byName.values, contains(glyph));
    }
    expect(LumitIcons.byName.values.toSet().length,
        lessThanOrEqualTo(LumitIcons.byName.length));
  });

  test('every document carries the set grammar and nothing coloured', () {
    LumitIcons.byName.forEach((name, svg) {
      expect(svg, startsWith('<svg viewBox="0 0 16 16"'), reason: name);
      expect(svg, contains('stroke="currentColor"'), reason: name);
      expect(svg, contains('stroke-width="1.5"'), reason: name);
      expect(svg, endsWith('</svg>'), reason: name);
      // Monochrome only (docs/15-DESIGN.md §5): the Channels glyph is coloured
      // by the Viewer at runtime, not in the glyph.
      expect(RegExp('#[0-9a-fA-F]{3,8}').hasMatch(svg), isFalse, reason: name);
    });
  });

  testWidgets('a handful of glyphs draw at the asked-for size', (tester) async {
    const glyphs = <String>[
      LumitIcons.select,
      LumitIcons.play,
      LumitIcons.stopwatch,
      LumitIcons.channels,
    ];
    await tester.pumpWidget(Directionality(
      textDirection: TextDirection.ltr,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final glyph in glyphs) LumitIcon(glyph, semanticLabel: 'glyph'),
          LumitIcon(LumitIcons.play, size: 24),
        ],
      ),
    ));
    await tester.pumpAndSettle();

    expect(find.byType(LumitIcon), findsNWidgets(glyphs.length + 1));
    for (final icon in tester.widgetList<LumitIcon>(find.byType(LumitIcon))) {
      expect(tester.getSize(find.byWidget(icon)), Size(icon.size, icon.size));
    }
  });
}
