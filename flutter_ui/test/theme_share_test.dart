// Sharing a theme, and the shelf of verbs around one: the file a theme
// is written to and read from, duplicating, importing under a free name, and
// renaming.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/custom_theme.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/theme/theme_file.dart';
import 'package:lumit_flutter/theme/theme_tokens.dart';

/// A settings file of this test's own: every workspace call that changes a
/// theme saves, and the store is machine state a test must not reach.
String _scratchStore(String name) =>
    '${Directory.systemTemp.path}${Platform.pathSeparator}'
    'lumit-test-$name${Platform.pathSeparator}workspace.json';

/// A workspace writing somewhere harmless, torn down after the test.
Workspace _workspace(String name) {
  Workspace.storeOverride = _scratchStore(name);
  addTearDown(() => Workspace.storeOverride = null);
  return Workspace();
}

void main() {
  group('the theme file', () {
    /// The whole point: a theme written out on one machine is the same theme
    /// when it is read back on another.
    test('a theme survives being written and read', () {
      final made = CustomTheme.from('Mine', LumitTheme.catppuccinMocha());
      final read = readThemeFile(encodeThemeFile(made));
      expect(read.refusal, isNull);
      expect(read.theme!.name, 'Mine');
      expect(read.theme!.mode, ThemeMode2.dark);
      for (final token in themeTokens) {
        expect(read.theme!.colours[token.key], made.colours[token.key],
            reason: token.key);
      }
    });

    /// Picking the wrong file is a normal thing to do, so it comes back as a
    /// sentence rather than an exception.
    test('what is not a theme is refused with a reason', () {
      for (final text in ['not json at all', '[]', '"a string"']) {
        final read = readThemeFile(text);
        expect(read.theme, isNull, reason: text);
        expect(read.refusal, isNotNull, reason: text);
      }
      // JSON, an object, and honest about not being a theme.
      expect(
          readThemeFile('{"format": "lumit-keymap", "bindings": []}').refusal,
          'That file is not a Lumit theme.');
      // A theme with no name could not be selected once it was in.
      expect(readThemeFile('{"colours": {"accent": "#ffffff"}}').theme, isNull);
      // And a named theme with nothing in it is a file that would change
      // nothing, which is worth saying rather than silently importing.
      expect(readThemeFile('{"name": "Empty", "colours": {}}').theme, isNull);
    });
  });

  group('the shelf of verbs', () {
    test('an import never overwrites a theme of the same name', () {
      final ws = _workspace('theme-import');
      ws.saveCustomTheme(CustomTheme.from('Ocean', LumitTheme.dark()));

      final incoming = CustomTheme.from('Ocean', LumitTheme.light());
      final landed = ws.importCustomTheme(incoming);

      expect(landed, 'Ocean 2');
      expect(ws.customThemes.map((t) => t.name), ['Ocean', 'Ocean 2']);
      expect(ws.customThemeName, 'Ocean 2', reason: 'an import selects itself');
      expect(ws.customThemes.first.mode, ThemeMode2.dark,
          reason: 'the one that was already there is untouched');
      expect(ws.theme.mode, ThemeMode2.light);
    });

    test('an imported theme survives the workspace file', () {
      final ws = _workspace('theme-import-persist');
      final read = readThemeFile(
          encodeThemeFile(CustomTheme.from('Ocean', LumitTheme.gruvboxDark())));
      ws.importCustomTheme(read.theme!);

      final restored = Workspace()..applyJson(ws.toJson());
      expect(restored.customThemes.single.name, 'Ocean');
      expect(restored.customThemeName, 'Ocean');
      for (final token in themeTokens) {
        expect(token.read(restored.theme), token.read(LumitTheme.gruvboxDark()),
            reason: token.key);
      }
    });
  });
}
