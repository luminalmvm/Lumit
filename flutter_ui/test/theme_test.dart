// Ports of the Rust theme tests (crates/lumit-ui/src/theme.rs) so the Dart
// tables cannot silently drift from the Rust ones.

import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';

int r(Color c) => (c.r * 255).round();
int g(Color c) => (c.g * 255).round();
int b(Color c) => (c.b * 255).round();

void main() {
  test('shape tokens sharp matches the pre-K-092 hardcoded numbers', () {
    const t = ShapeTokens.sharp;
    expect(t.controlRadius, 4);
    expect(t.floatRadius, 6);
    expect(t.cardRadius, 0);
    expect(t.cardPadding, 0);
    expect(t.tileGap, 1.0);
    expect(t.windowInset, 0.0);
    expect(t.cardShadow, isEmpty);
  });

  /// Round v2, the bubble commit (K-394, docs/15-DESIGN.md §12.1).
  test('shape tokens round carry the v2 geometry', () {
    const t = ShapeTokens.round;
    expect(t.controlRadius, ShapeTokens.stadium,
        reason: 'controls are capsules, not 8px corners');
    expect(t.floatRadius, 16);
    expect(t.cardRadius, 18,
        reason: 'a menu must not be squarer than the card that spawned it');
    // The gap/inset system stands as K-092 built it.
    expect(t.cardPadding, 10);
    expect(t.tileGap, 12.0);
    expect(t.windowInset, 12.0);
  });

  /// What the stadium sentinel rests on: a rounded rectangle scales its radii
  /// down to fit its own box, so one radius bigger than any control is tall
  /// draws a capsule at *whatever* height the control turns out to be — which
  /// is why `controlRadius` needs no second field and no call site changes.
  test('the stadium radius clamps to half a control of any height', () {
    for (final height in [16.0, 20.0, 44.0]) {
      final drawn = RRect.fromRectAndRadius(
        Rect.fromLTWH(0, 0, 120, height),
        const Radius.circular(ShapeTokens.stadium),
      ).scaleRadii();
      expect(drawn.tlRadiusY, height / 2, reason: 'at height $height');
      expect(drawn.tlRadiusX, height / 2, reason: 'at height $height');
    }
  });

  test('every colour scheme builds under both shapes', () {
    for (final scheme in LumitColorScheme.values) {
      for (final shape in ThemeShape.values) {
        final t = LumitTheme.forScheme(scheme, shape);
        expect(t.shape, shape);
        expect(t.tokens, ShapeTokens.of(shape));
      }
    }
  });

  test(
      'scheme mode matches built theme and is light for the three light schemes only',
      () {
    const lightSchemes = [
      LumitColorScheme.light,
      LumitColorScheme.gruvboxLight,
      LumitColorScheme.catppuccinLatte,
    ];
    for (final scheme in LumitColorScheme.values) {
      expect(scheme.build().mode, scheme.mode);
      expect(
        scheme.mode,
        lightSchemes.contains(scheme) ? ThemeMode2.light : ThemeMode2.dark,
        reason: 'wrong mode for $scheme',
      );
    }
  });

  test('all seven labels are unique and non-empty', () {
    final labels = [for (final s in LumitColorScheme.values) s.label];
    expect(labels.length, 7);
    expect(labels.toSet().length, labels.length);
    for (final l in labels) {
      expect(l, isNotEmpty);
    }
  });

  test('with_accent hover shift direction differs by mode (Rust test values)',
      () {
    const rgb = Color(0xff804060);
    final dark = LumitTheme.dark().withAccent(rgb);
    expect(r(dark.accentHover), 0x92);
    expect(g(dark.accentHover), 0x52);
    expect(b(dark.accentHover), 0x72);

    final light = LumitTheme.light().withAccent(rgb);
    expect(r(light.accentHover), 0x6e);
    expect(g(light.accentHover), 0x2e);
    expect(b(light.accentHover), 0x4e);
  });

  test('dark scheme viewer surround is exactly neutral (r == g == b)', () {
    for (final scheme in [
      LumitColorScheme.dark,
      LumitColorScheme.darkBlue,
      LumitColorScheme.gruvboxDark,
      LumitColorScheme.catppuccinMocha,
    ]) {
      final c = scheme.build().viewerSurround;
      expect(g(c), r(c), reason: '$scheme viewer surround not neutral');
      expect(b(c), r(c), reason: '$scheme viewer surround not neutral');
    }
  });

  test('spot-check hex fidelity against theme.rs', () {
    final dark = LumitTheme.dark();
    expect(r(dark.surface0), 0x0b);
    expect(g(dark.surface0), 0x0c);
    expect(b(dark.surface0), 0x0e);
    expect(r(dark.accent), 0xe0);
    expect(g(dark.accent), 0x5a);
    expect(b(dark.accent), 0x72);

    final mocha = LumitTheme.catppuccinMocha();
    expect(r(mocha.surface1), 0x1e);
    expect(b(mocha.surface1), 0x2e);
    expect(r(mocha.accent), 0xcb);
    expect(g(mocha.accent), 0xa6);
    expect(b(mocha.accent), 0xf7);

    final gruvLight = LumitTheme.gruvboxLight();
    expect(r(gruvLight.accent), 0xaf);
    expect(g(gruvLight.accent), 0x3a);
    expect(b(gruvLight.accent), 0x03);
  });

  test('label colours cycle over one distinct chip per layer kind', () {
    final t = LumitTheme.dark();
    // A dedicated bright palette (K-189), not the theme's role colours: the
    // chips colour the lane bars, so they must be tellable apart. Nine since
    // K-206 gave the Null layer its own.
    expect(LumitTheme.labelCount, 9);
    final chips = {
      for (var i = 0; i < LumitTheme.labelCount; i++) t.labelColour(i)
    };
    expect(chips, hasLength(LumitTheme.labelCount),
        reason: 'no two chips share a colour');
    expect(t.labelColour(LumitTheme.labelCount), t.labelColour(0),
        reason: 'the palette cycles');
  });

  test('animation levels map to the documented durations', () {
    expect(animationDuration(AnimationLevel.all).inMilliseconds, 120);
    expect(animationDuration(AnimationLevel.minimal).inMilliseconds, 50);
    expect(animationDuration(AnimationLevel.none), Duration.zero);
  });

  test('body text is regular weight; emphasis is medium and rationed', () {
    // docs/15-DESIGN §7.1 sets 11px Inter (regular) for body copy, menus and
    // buttons, with Medium reserved for emphasis (tab labels, dialog
    // headings). Everything used to render Medium because only that face was
    // bundled — this pins the lighter default so it cannot regress.
    final t = LumitTheme.dark();
    expect(t.body.fontWeight, FontWeight.w400);
    expect(t.bodyPrimary.fontWeight, FontWeight.w400);
    expect(t.small.fontWeight, FontWeight.w400);
    expect(t.caption.fontWeight, FontWeight.w400);
    expect(t.heading.fontWeight, FontWeight.w500);
    expect(t.bodyStrong.fontWeight, FontWeight.w500);
  });

  test('the type scale sits at the K-317 sizes', () {
    // docs/15-DESIGN §7.1: 11px body, 10px small, 9px caption — one step
    // tighter than the original scale, by owner request.
    final t = LumitTheme.dark();
    expect(t.body.fontSize, 11);
    expect(t.bodyPrimary.fontSize, 11);
    expect(t.small.fontSize, 10);
    expect(t.caption.fontSize, 9);
    expect(t.heading.fontSize, 16);
  });
}
