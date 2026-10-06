// The token list, custom themes, and the two Timeline colours.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
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
        'timelineOutOfRange', 'selectionFill', 'room',
        'curve0', 'curve1', 'curve2', 'curve3',
        'waveformRest', 'waveformLow', 'waveformMid', 'waveformHigh',
        'layerFootage', 'layerSequence', 'layerPrecomp',
        'layerSolid', 'layerText', 'layerCamera',
        // Deliberately NOT a token: the Viewer surround is strictly neutral
        // by spec (15-DESIGN §2.1/§11) — a grade cannot be judged against a
        // tinted surround, so it is the one colour taste does not reach.
        //
        // Nor are the five `port.*` wire colours, for the same kind of
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
  });

  /// The token that says "this is animated or in hand" (15-DESIGN
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
      // Desk's two rooms are the one exception to the second check: their
      // document dissolves amber into the signal, so keyed and in hand share
      // it and the keyframe's shape says the rest
      // (docs/design-alt/15-DESIGN-DESK.md 3.2).
      const oneSignal = {LumitColorScheme.greyRoom, LumitColorScheme.graphite};
      for (final scheme in LumitColorScheme.values) {
        final t = scheme.build();
        expect(contrast(t.animated, t.surface1), greaterThanOrEqualTo(3.0),
            reason: '${scheme.name} draws keyframes it cannot show');
        if (oneSignal.contains(scheme)) {
          expect(t.animated, t.accent,
              reason: '${scheme.name} has one signal, not two');
          continue;
        }
        expect(t.animated, isNot(t.accent),
            reason: '${scheme.name} would say "keyed" and "in hand" alike');
      }
      expect(LumitTheme.dark().animated, const Color(0xffd8a24a));
    });
  });
}
