// Sharing a theme, and the shelf of verbs around one (K-298): the file a theme
// is written to and read from, duplicating, importing under a free name, and
// renaming.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/widgets.dart';
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

    /// Readable, because a theme is a thing people tinker with — the same
    /// reasoning that put hex rather than numbers in the workspace file.
    test('the file says what it is, and reads as text', () {
      final text = encodeThemeFile(CustomTheme.from('Mine', LumitTheme.dark()));
      final json = jsonDecode(text) as Map<String, dynamic>;
      expect(json['format'], themeFileFormat);
      expect(json['version'], themeFileVersion);
      expect((json['colours'] as Map)['accent'], '#e05a72');
      expect(text.contains('\n  '), isTrue, reason: 'indented, not one line');
      expect(text.endsWith('\n'), isTrue);
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

    /// Forward tolerance is the reason a theme is stored over a base at all: a
    /// file from a newer Lumit still opens, with the colours this build knows.
    test('a theme from a newer Lumit still opens', () {
      final read = readThemeFile(jsonEncode({
        'format': themeFileFormat,
        'version': themeFileVersion + 7,
        'name': 'From the future',
        'mode': 'dark',
        'colours': {
          'accent': '#112233',
          'a.token.from.the.future': '#445566',
        },
        'something.else.entirely': true,
      }));
      expect(read.theme, isNotNull);
      expect(read.theme!.colours['accent'], const Color(0xff112233));

      // The unknown key is *carried*, not dropped. Dropping it here would make
      // this build quietly strip a newer Lumit's colours the next time it wrote
      // the workspace file — the same parser reads both — which is the data
      // loss forward tolerance exists to prevent. What matters is that it
      // changes nothing: `applyTokens` names the tokens it knows and never
      // consults a key it does not.
      final built = read.theme!.build(ThemeShape.sharp);
      final base = LumitColorScheme.dark.build().copyWith(
            shape: ThemeShape.sharp,
            tokens: ShapeTokens.of(ThemeShape.sharp),
          );
      expect(built.accent, const Color(0xff112233),
          reason: 'the colour this build knows is taken from the file');
      expect(built.surface0, base.surface0,
          reason: 'and everything else still comes from the base');
    });

    /// A theme lifted straight out of a workspace file has the same three
    /// fields and no marker. Refusing that would be pedantry rather than
    /// safety.
    test('a theme without the marker is still a theme', () {
      final read = readThemeFile(
          jsonEncode(CustomTheme.from('Plain', LumitTheme.light()).toJson()));
      expect(read.theme?.name, 'Plain');
      expect(read.theme?.mode, ThemeMode2.light);
    });

    test('the suggested file name is one a filesystem will take', () {
      expect(themeFileName('My theme'), 'my-theme.lumtheme');
      expect(themeFileName('  Ocean / Night  '), 'ocean-night.lumtheme');
      expect(themeFileName('***'), 'theme.lumtheme',
          reason: 'a name with nothing usable in it still saves');
    });
  });

  group('the shelf of verbs', () {
    test('duplicating a built-in makes an editable theme of it', () {
      final ws = _workspace('theme-duplicate');
      final name = ws.duplicateActiveTheme();

      expect(name, 'Dark copy');
      expect(ws.customThemeName, name, reason: 'the copy is selected');
      expect(ws.customThemes.single.name, name);
      // The copy really is what was on screen, colour for colour.
      for (final token in themeTokens) {
        expect(token.read(ws.theme), token.read(LumitTheme.dark()),
            reason: token.key);
      }
    });

    test('duplicating twice numbers the second rather than overwriting', () {
      final ws = _workspace('theme-duplicate-twice');
      final first = ws.duplicateActiveTheme();
      ws.setScheme(LumitColorScheme.dark);
      final second = ws.duplicateActiveTheme();

      expect(first, 'Dark copy');
      expect(second, 'Dark copy 2');
      expect(ws.customThemes.length, 2);
    });

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

    test('renaming keeps the theme selected and where it was in the list', () {
      final ws = _workspace('theme-rename');
      ws.saveCustomTheme(CustomTheme.from('One', LumitTheme.dark()));
      ws.saveCustomTheme(CustomTheme.from('Two', LumitTheme.dark()));
      ws.setCustomTheme('One');

      expect(ws.renameCustomTheme('One', 'Night'), 'Night');
      expect(ws.customThemes.map((t) => t.name), ['Night', 'Two']);
      expect(ws.customThemeName, 'Night');
      expect(ws.activeCustomTheme, isNotNull,
          reason: 'the selection followed the name');

      // Onto a name already taken: numbered, never merged into the other.
      expect(ws.renameCustomTheme('Night', 'Two'), 'Two 2');
      expect(ws.customThemes.map((t) => t.name), ['Two 2', 'Two']);
      // And a rename of something that is not a saved theme changes nothing.
      expect(ws.renameCustomTheme('Nothing', 'Anything'), isNull);
      expect(ws.customThemes.length, 2);
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
