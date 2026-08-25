// The token list, custom themes, and the two Timeline colours (K-202).

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/custom_theme.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/theme/theme_file.dart';
import 'package:lumit_flutter/theme/theme_tokens.dart';

void main() {
  group('the token list', () {
    /// The editor draws one row per token, so a token added to `LumitTheme`
    /// and not listed here is a colour nobody can reach with nothing to say
    /// so. This counts the struct's own colours against the list — the check
    /// that fails the day the two drift.
    test('covers every colour on the theme', () {
      final t = LumitTheme.dark();
      // Every colour LumitTheme carries, by hand — the point is to restate
      // it independently, so a field added to one and not the other shows up.
      final onTheStruct = <String>{
        'surface0', 'surface1', 'surface2', 'surface3', 'surface4',
        'textPrimary', 'textSecondary', 'textMuted', 'textDisabled',
        'hairline', 'hairlineStrong',
        'accent', 'accentHover', 'animated', 'success', 'warning', 'error',
        'cacheDisk',
        'marker',
        'timelineOutOfRange', 'selectionFill',
        'curve0', 'curve1', 'curve2', 'curve3',
        'waveformRest', 'waveformLow', 'waveformMid', 'waveformHigh',
        'layerFootage', 'layerSequence', 'layerPrecomp',
        'layerSolid', 'layerText', 'layerCamera',
        // Deliberately NOT a token: the Viewer surround is strictly neutral
        // by spec (15-DESIGN §2.1/§11) — a grade cannot be judged against a
        // tinted surround, so it is the one colour taste does not reach.
        //
        // Nor are the five `port.*` wire colours (K-472), for the same kind of
        // reason: on the Graph panel colour *is* the legend — the strip along
        // the canvas says "amber is a number" — so a palette taste could
        // retint would be a legend that lies. See [PortColours].
      };
      final listed = themeTokens.map((t) => t.key).toSet();
      expect(listed, onTheStruct);
      expect(t.viewerSurround, isNotNull,
          reason: 'still there, just not offered');
    });

    test('every token reads and writes its own field', () {
      const probe = Color(0xff123456);
      for (final token in themeTokens) {
        final changed = token.write(LumitTheme.dark(), probe);
        expect(token.read(changed), probe,
            reason: '${token.key} does not read back what it wrote');
        // And it changed *only* its own field.
        final others = themeTokens.where((o) => o.key != token.key);
        for (final other in others) {
          expect(other.read(changed), other.read(LumitTheme.dark()),
              reason: '${token.key} also moved ${other.key}');
        }
      }
    });

    test('every token has a label, a description and a group', () {
      for (final token in themeTokens) {
        expect(token.label, isNotEmpty, reason: token.key);
        expect(token.description, isNotEmpty, reason: token.key);
        expect(token.group, isNotEmpty, reason: token.key);
      }
      expect(themeTokenGroups, contains('Surfaces'));
    });

    /// Reading a theme out and applying it back is the round trip a save and
    /// a reload make — it has to land on the same colours.
    test('tokensOf and applyTokens round-trip a theme', () {
      final source = LumitTheme.gruvboxDark();
      final rebuilt = applyTokens(LumitTheme.dark(), tokensOf(source));
      for (final token in themeTokens) {
        expect(token.read(rebuilt), token.read(source), reason: token.key);
      }
    });

    /// A theme saved by another build carries keys this one may not know, and
    /// may be missing ones it does. Neither is an error: the unknown is
    /// ignored, the missing keeps the base's colour.
    test('applyTokens tolerates unknown and missing keys', () {
      final base = LumitTheme.dark();
      final applied = applyTokens(base, {
        'accent': const Color(0xffabcdef),
        'a.token.from.the.future': const Color(0xff000000),
      });
      expect(applied.accent, const Color(0xffabcdef));
      expect(applied.surface0, base.surface0,
          reason: 'untouched keeps its own');
    });
  });

  group('the Timeline colours', () {
    /// The whole point of the pair: a selected row has to be tellable from
    /// both grounds it can sit on, in either mode.
    test('selection reads apart from both grounds, light and dark', () {
      double luma(Color c) => 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
      for (final theme in [LumitTheme.dark(), LumitTheme.light()]) {
        final gap = (luma(theme.selectionFill) - luma(theme.surface1)).abs();
        final gapOut =
            (luma(theme.selectionFill) - luma(theme.timelineOutOfRange)).abs();
        expect(gap, greaterThan(0.02),
            reason: '${theme.mode} selection is too close to the panel');
        expect(gapOut, greaterThan(0.02),
            reason: '${theme.mode} selection is too close to the wash');
      }
    });

    /// Out-of-range is darker than the work area in *both* modes — on a light
    /// scheme the surfaces go up, so the only direction left is down.
    test('the out-of-range wash is darker than the work area in both modes',
        () {
      double luma(Color c) => 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
      for (final theme in [LumitTheme.dark(), LumitTheme.light()]) {
        expect(luma(theme.timelineOutOfRange), lessThan(luma(theme.surface1)),
            reason: '${theme.mode} wash is not darker than the work area');
      }
    });

    /// The light scheme needs a bigger step than the dark one: the same
    /// difference reads as less on a bright ground.
    test('the light wash steps further than the dark one', () {
      double luma(Color c) => 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
      final dark = LumitTheme.dark();
      final light = LumitTheme.light();
      final darkStep = luma(dark.surface1) - luma(dark.timelineOutOfRange);
      final lightStep = luma(light.surface1) - luma(light.timelineOutOfRange);
      expect(lightStep, greaterThan(darkStep));
    });

    /// A scheme may state its own, and then the default must not overwrite it.
    test('a scheme can state its own Timeline colours', () {
      final t = LumitTheme(
        mode: ThemeMode2.dark,
        surface0: const Color(0xff000000),
        surface1: const Color(0xff101010),
        surface2: const Color(0xff202020),
        surface3: const Color(0xff303030),
        surface4: const Color(0xff404040),
        viewerSurround: const Color(0xff121212),
        textPrimary: const Color(0xffffffff),
        textSecondary: const Color(0xffdddddd),
        textMuted: const Color(0xff999999),
        textDisabled: const Color(0xff666666),
        hairline: const Color(0xff222222),
        hairlineStrong: const Color(0xff333333),
        accent: const Color(0xffff0000),
        accentHover: const Color(0xffff3333),
        animated: const Color(0xffffaa00),
        success: const Color(0xff00ff00),
        warning: const Color(0xffffff00),
        error: const Color(0xffff00ff),
        cacheDisk: const Color(0xff0000ff),
        curve: const [Color(0xff111111)],
        layer: const LayerColours(
          footage: Color(0xff111111),
          sequence: Color(0xff222222),
          precomp: Color(0xff333333),
          solid: Color(0xff444444),
          text: Color(0xff555555),
          camera: Color(0xff666666),
        ),
        timelineOutOfRange: const Color(0xff0a0a0a),
        selectionFill: const Color(0xff5a5a5a),
      );
      expect(t.timelineOutOfRange, const Color(0xff0a0a0a));
      expect(t.selectionFill, const Color(0xff5a5a5a));
    });
  });

  group('custom themes', () {
    test('a theme survives its own JSON', () {
      final made = CustomTheme.from('Mine', LumitTheme.catppuccinMocha());
      final back = CustomTheme.fromJson(made.toJson());
      expect(back, isNotNull);
      expect(back!.name, 'Mine');
      expect(back.mode, ThemeMode2.dark);
      for (final token in themeTokens) {
        expect(back.colours[token.key], made.colours[token.key],
            reason: token.key);
      }
    });

    /// Stored as readable hex, so somebody can open the workspace file and
    /// paste a colour into their theme.
    test('colours are stored as readable hex', () {
      final json = CustomTheme.from('Mine', LumitTheme.dark()).toJson();
      final colours = json['colours'] as Map;
      expect(colours['accent'], '#e05a72');
    });

    test('a nameless or malformed theme is refused rather than half-loaded',
        () {
      expect(CustomTheme.fromJson({'mode': 'dark'}), isNull);
      expect(CustomTheme.fromJson({'name': '   '}), isNull);
      // A theme with unreadable colours still loads — it just falls back to
      // the base for those, which is a working theme rather than none.
      final partial = CustomTheme.fromJson({
        'name': 'Odd',
        'mode': 'light',
        'colours': {'accent': 'not a colour', 'surface0': '#112233'},
      });
      expect(partial, isNotNull);
      expect(partial!.colours.containsKey('accent'), isFalse);
      expect(partial.colours['surface0'], const Color(0xff112233));
    });

    /// The base matters: a light custom theme resolves over the light ramp,
    /// so anything it does not carry is light too.
    test('a custom theme builds over the ramp its mode names', () {
      final theme = CustomTheme(
        name: 'Barely',
        mode: ThemeMode2.light,
        colours: {'accent': const Color(0xff00ff00)},
      ).build(ThemeShape.sharp);
      expect(theme.accent, const Color(0xff00ff00));
      expect(theme.mode, ThemeMode2.light);
      expect(theme.surface0, LumitTheme.light().surface0,
          reason: 'what it does not carry comes from the light ramp');
    });
  });

  /// The token that says "this is animated or in hand" (K-439, 15-DESIGN
  /// §3.1). Every scheme has to carry one — a keyframe diamond nobody can see
  /// is worse than no colour at all — and it has to hold against the panel it
  /// is drawn on, which on a light scheme means a much darker amber than the
  /// dark ramp's.
  group('the animated token', () {
    /// WCAG relative luminance, so "3:1" here means what it means everywhere
    /// else in the spec rather than a plain average.
    double luminance(Color c) {
      double channel(double v) => v <= 0.03928
          ? v / 12.92
          : math.pow((v + 0.055) / 1.055, 2.4) as double;
      return 0.2126 * channel(c.r) +
          0.7152 * channel(c.g) +
          0.0722 * channel(c.b);
    }

    double contrast(Color a, Color b) {
      final la = luminance(a), lb = luminance(b);
      return (math.max(la, lb) + 0.05) / (math.min(la, lb) + 0.05);
    }

    test('every scheme carries one that reads on its own panel', () {
      for (final scheme in LumitColorScheme.values) {
        final t = scheme.build();
        expect(contrast(t.animated, t.surface1), greaterThanOrEqualTo(3.0),
            reason: '${scheme.name} draws keyframes it cannot show');
        expect(t.animated, isNot(t.accent),
            reason: '${scheme.name} would say "keyed" and "in hand" alike');
      }
      expect(LumitTheme.dark().animated, const Color(0xffd8a24a));
    });

    test('the editor can edit it', () {
      expect(themeTokens.map((t) => t.key), contains('animated'));
    });

    /// A theme file written before the token existed carries no `animated`
    /// key. It must still load, taking the colour from its base — the whole
    /// reason a theme is stored over a base rather than as a copy.
    test('a theme file saved without it still loads', () {
      final made = CustomTheme.from('Mine', LumitTheme.dark());
      final older = CustomTheme(
        name: made.name,
        mode: made.mode,
        colours: {...made.colours}..remove('animated'),
      );
      final read = readThemeFile(encodeThemeFile(older));
      expect(read.refusal, isNull);
      expect(read.theme!.build(ThemeShape.sharp).animated,
          LumitTheme.dark().animated);
    });

    test('and one that carries it keeps it', () {
      final made = CustomTheme.from('Mine', LumitTheme.gruvboxLight());
      final read = readThemeFile(encodeThemeFile(made));
      expect(
          read.theme!.colours['animated'], LumitTheme.gruvboxLight().animated);
    });
  });
}
