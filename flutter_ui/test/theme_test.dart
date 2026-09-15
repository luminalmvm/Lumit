// Ports of the Rust theme tests (crates/lumit-ui/src/theme.rs) so the Dart
// tables cannot silently drift from the Rust ones.

import 'dart:io';
import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';

int r(Color c) => (c.r * 255).round();
int g(Color c) => (c.g * 255).round();
int b(Color c) => (c.b * 255).round();

void main() {
  /// Studio is Sharp under its new name: every number the drawings measured.
  test('shape tokens studio matches the drawings', () {
    const t = ShapeTokens.studio;
    expect(t.controlRadius, 2);
    expect(t.floatRadius, 6);
    expect(t.cardRadius, 0);
    expect(t.cardPadding, 0);
    expect(t.tileGap, 1.0);
    expect(t.windowInset, 0.0);
    expect(t.cardShadow, isEmpty);
    expect(t.labelCase, LabelCase.caps);
    expect(t.titleCentred, isFalse);
    expect(t.headerDot, isFalse);
    expect(t.strokeWeight, 1.5);
    expect(t.sectionRadius, 2);
    expect(t.actionRadius, 2);
    expect(t.wellRadius, 2);
    expect(t.contentRadius, 2);
    expect(t.roomed, isFalse);
    expect(t.pillInset, 0);
    expect(ShapeTokens.of(ThemeShape.studio), same(t));
  });

  /// Desk: flush like Studio, a flatter float, lowercase labels and a lighter
  /// stroke (docs/design-alt/15-DESIGN-RAMS.md §12).
  test('shape tokens desk carry the grey-room geometry', () {
    const t = ShapeTokens.desk;
    // Square everywhere, and the panels are plates in a 4px chassis.
    expect(t.controlRadius, 0);
    expect(t.floatRadius, 0);
    expect(t.cardRadius, 0);
    expect(t.cardPadding, 0);
    expect(t.tileGap, 4.0);
    expect(t.windowInset, 4.0);
    expect(t.cardShadow, isEmpty);
    expect(t.labelCase, LabelCase.lower);
    expect(t.titleCentred, isFalse);
    expect(t.headerDot, isFalse);
    expect(t.strokeWeight, 1.25);
    expect(t.sectionRadius, 0);
    expect(t.actionRadius, 0);
    expect(t.wellRadius, 0);
    expect(t.contentRadius, 0);
    expect(t.roomed, isFalse);
    expect(t.pillInset, 0);
    expect(ShapeTokens.of(ThemeShape.desk), same(t));
  });

  /// Lantern: four radii and the pill, each meaning one thing
  /// (docs/design-alt/15-DESIGN-LANTERN.md §12).
  test('shape tokens lantern carry the card-in-a-room geometry', () {
    const t = ShapeTokens.lantern;
    expect(t.controlRadius, 7);
    expect(t.floatRadius, 12);
    expect(t.cardRadius, 16);
    expect(t.cardPadding, 0);
    expect(t.tileGap, 10.0);
    expect(t.windowInset, 10.0);
    expect(t.cardShadow, hasLength(2));
    // Quiet on purpose: a deeper shadow darkened the gaps between cards.
    expect(t.cardShadow[0].offset, const Offset(0, 3));
    expect(t.cardShadow[0].blurRadius, 12);
    expect(t.cardShadow[0].color, const Color(0x1A000000));
    expect(t.cardShadow[1].offset, const Offset(0, 1));
    expect(t.cardShadow[1].blurRadius, 2);
    expect(t.cardShadow[1].color, const Color(0x10000000));
    expect(t.labelCase, LabelCase.caps);
    expect(t.titleCentred, isTrue);
    expect(t.headerDot, isTrue);
    expect(t.strokeWeight, 1.5);
    expect(t.sectionRadius, 12);
    expect(t.actionRadius, ShapeTokens.stadium,
        reason: 'actions are capsules; a value box is not');
    expect(t.wellRadius, 7);
    expect(t.contentRadius, 3);
    expect(t.roomed, isTrue);
    expect(t.pillInset, 3, reason: 'the filled state sits 3 inside the pill');
    expect(ShapeTokens.of(ThemeShape.lantern), same(t));
  });

  test('a kicker is cased by the shape', () {
    expect(
        LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.studio)
            .kickerCase('Time line'),
        'TIME LINE');
    expect(
        LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.desk)
            .kickerCase('Time line'),
        'time line');
    expect(
        LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.lantern)
            .kickerCase('Time line'),
        'TIME LINE');
  });

  /// Desk's module: every row a multiple of four, and Compact drops the row
  /// and the property row one module each and nothing else.
  test('the desk densities sit on the four-pixel module', () {
    const r = DensityTokens.deskRegular;
    expect(r.laneRow, 24);
    expect(r.secondaryRow, 20);
    expect(r.timelineChromeRow, 24);
    expect(r.timelineHeaderRow, 24);
    expect(r.timelineChromeControl, 20);
    expect(r.inRowPicker, 16);
    expect(r.dropdownFace, 20);
    expect(r.propertyRow, 28);
    expect(r.headerStrip, 24);
    expect(r.cacheBar, 4);
    expect(r.navigatorBand, 16);
    expect(r.scrollbar, 8);
    expect(r.menuBar, 28, reason: '15-DESIGN-DESK.md 12B.2: one 28px strip');

    const c = DensityTokens.deskCompact;
    expect(c.laneRow, 20);
    expect(c.secondaryRow, 20);
    expect(c.timelineChromeRow, 24);
    expect(c.timelineHeaderRow, 24);
    expect(c.timelineChromeControl, 20);
    expect(c.inRowPicker, 16);
    expect(c.dropdownFace, 20);
    expect(c.propertyRow, 24);
    expect(c.headerStrip, 24);
    expect(c.cacheBar, 4);
    expect(c.navigatorBand, 16);
    expect(c.scrollbar, 8);
    expect(c.menuBar, 28);

    expect(DensityTokens.forShape(ThemeShape.desk, false), same(r));
    expect(DensityTokens.forShape(ThemeShape.desk, true), same(c));
    expect(DensityTokens.forShape(ThemeShape.studio, false),
        same(DensityTokens.regular));
    expect(DensityTokens.forShape(ThemeShape.studio, true),
        same(DensityTokens.compact));
  });

  /// Studio keeps the chrome measures it always drew.
  test('the studio densities carry the chrome statics', () {
    for (final d in [DensityTokens.regular, DensityTokens.compact]) {
      expect(d.headerStrip, 22);
      expect(d.cacheBar, 3);
      expect(d.navigatorBand, 12);
      expect(d.scrollbar, 7);
      expect(d.menuBar, 26, reason: 'the menu bar the app has always drawn');
    }
  });

  /// Lantern is the surveyed set under a 28 row pitch and a 36 title line.
  test('the lantern densities keep the row pitch and card title line', () {
    const r = DensityTokens.lanternRegular;
    expect(r.laneRow, 28);
    expect(r.secondaryRow, 19);
    expect(r.timelineChromeRow, 24);
    expect(r.timelineHeaderRow, 23);
    expect(r.timelineChromeControl, 20);
    expect(r.inRowPicker, 18);
    expect(r.dropdownFace, 20);
    expect(r.propertyRow, 27);
    expect(r.headerStrip, 36);
    expect(r.cacheBar, 4);
    expect(r.navigatorBand, 12);
    expect(r.scrollbar, 7);
    expect(r.menuBar, 40,
        reason: 'the band plus the 44 tool strip under it fit inside 84');

    const c = DensityTokens.lanternCompact;
    expect(c.laneRow, 28);
    expect(c.secondaryRow, 18);
    expect(c.timelineChromeRow, 18);
    expect(c.timelineHeaderRow, 18);
    expect(c.timelineChromeControl, isNull);
    expect(c.inRowPicker, 16);
    expect(c.dropdownFace, 18);
    expect(c.propertyRow, 26);
    expect(c.headerStrip, 36);
    expect(c.cacheBar, 4);
    expect(c.navigatorBand, 12);
    expect(c.scrollbar, 7);
    expect(c.menuBar, 40);

    expect(DensityTokens.forShape(ThemeShape.lantern, false), same(r));
    expect(DensityTokens.forShape(ThemeShape.lantern, true), same(c));
  });

  /// The room is a colour like the Timeline pair: defaulted from the mode,
  /// and carried by every scheme whether or not its shape draws it.
  test('the room defaults from the mode and rides through copyWith', () {
    // The room is the other way round from the scheme: a light room for a
    // dark scheme, a dark one for a light scheme, and the ink on it follows.
    expect(LumitTheme.dark().room, LumitTheme.dayRoom);
    expect(LumitTheme.light().room, LumitTheme.nightRoom);
    expect(LumitTheme.dark().roomInk.computeLuminance(), lessThan(0.1));
    expect(LumitTheme.light().roomInk.computeLuminance(), greaterThan(0.8));
    final night = LumitTheme.dark().copyWith(room: LumitTheme.dark().surface0);
    expect(night.room, LumitTheme.dark().surface0);
    expect(night.copyWith(shape: ThemeShape.lantern).room, night.room);
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
      'scheme mode matches built theme and is light for the light schemes only',
      () {
    const lightSchemes = [
      LumitColorScheme.light,
      LumitColorScheme.gruvboxLight,
      LumitColorScheme.catppuccinLatte,
      LumitColorScheme.mallowLight,
      LumitColorScheme.glacierLight,
      LumitColorScheme.hearthLight,
      LumitColorScheme.chalkLight,
      LumitColorScheme.vellumLight,
      LumitColorScheme.neonLight,
      LumitColorScheme.canopyLight,
      LumitColorScheme.slateLight,
      LumitColorScheme.nocturneLight,
      LumitColorScheme.tavernLight,
      LumitColorScheme.arcaneLight,
      LumitColorScheme.giltLight,
      LumitColorScheme.greyRoom,
    ];
    // Seven from the Rust frontend, twelve pairs, and Desk's two rooms.
    expect(LumitColorScheme.values, hasLength(7 + 12 * 2 + 2));
    for (final scheme in LumitColorScheme.values) {
      expect(scheme.build().mode, scheme.mode);
      expect(
        scheme.mode,
        lightSchemes.contains(scheme) ? ThemeMode2.light : ThemeMode2.dark,
        reason: 'wrong mode for $scheme',
      );
    }
  });

  test('every label is unique and non-empty', () {
    final labels = [for (final s in LumitColorScheme.values) s.label];
    expect(labels.length, LumitColorScheme.values.length);
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

  /// BOTH stock pairs are *derived* from `defaultAccent` — spruce —
  /// so re-tuning is one edit and neither hover can drift off the ±0x12 step a
  /// user-picked accent gets. Light's hand-darkened exception is gone with
  /// clay: spruce clears the contrast floor on white unaided.
  test('the stock schemes are defaultAccent put through with_accent', () {
    expect(LumitTheme.defaultAccent, const Color(0xff35785e));

    final dark = LumitTheme.dark();
    expect(dark.accent, LumitTheme.defaultAccent);
    expect(dark.accentHover, const Color(0xff478a70));
    expect(dark.accentHover,
        LumitTheme.dark().withAccent(LumitTheme.defaultAccent).accentHover);

    final light = LumitTheme.light();
    expect(light.accent, LumitTheme.defaultAccent,
        reason: 'no exception any more - spruce clears the floor');
    expect(light.accentHover, const Color(0xff23664c));
    expect(light.accentHover,
        LumitTheme.light().withAccent(LumitTheme.defaultAccent).accentHover);

    // Clay stands second in the Settings swatch row.
    expect(LumitTheme.accentPresets.first, LumitTheme.defaultAccent);
    expect(LumitTheme.accentPresets[1], const Color(0xFFE05A72));
  });

  test('dark scheme viewer surround is exactly neutral (r == g == b)', () {
    for (final scheme in [
      LumitColorScheme.dark,
      LumitColorScheme.darkBlue,
      LumitColorScheme.gruvboxDark,
      LumitColorScheme.catppuccinMocha,
      LumitColorScheme.mallowDark,
      LumitColorScheme.glacierDark,
      LumitColorScheme.hearthDark,
      LumitColorScheme.chalkDark,
      LumitColorScheme.vellumDark,
      LumitColorScheme.neonDark,
      LumitColorScheme.canopyDark,
      LumitColorScheme.slateDark,
      LumitColorScheme.nocturneDark,
      LumitColorScheme.tavernDark,
      LumitColorScheme.arcaneDark,
      LumitColorScheme.giltDark,
      LumitColorScheme.graphite,
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
    // Spruce, the default accent both schemes build from.
    expect(r(dark.accent), 0x35);
    expect(g(dark.accent), 0x78);
    expect(b(dark.accent), 0x5e);

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

  /// Desk's two rooms (docs/design-alt/15-DESIGN-DESK.md 2 and 3): one ramp
  /// at two lightnesses, one signal doing the accent's job and the animated
  /// one, the same neutral surround in both.
  test('the desk rooms carry the document\'s values', () {
    final grey = LumitTheme.greyRoom();
    expect(grey.mode, ThemeMode2.light);
    expect(grey.surface0, const Color(0xffdedcd8));
    expect(grey.surface3, const Color(0xfff7f6f3));
    expect(grey.textPrimary, const Color(0xff1b1b1a));
    expect(grey.hairlineStrong, const Color(0xffa9a7a1));
    expect(grey.accent, const Color(0xffb8500d));
    expect(grey.accentHover, const Color(0xffca621f),
        reason: 'a step lighter than the signal');
    expect(grey.animated, grey.accent, reason: 'amber is dissolved');
    expect(grey.viewerSurround, const Color(0xff7f7f7f));
    expect(grey.curve, [
      grey.layer.footage,
      grey.layer.precomp,
      grey.layer.text,
      grey.layer.camera,
    ]);

    final graphite = LumitTheme.graphite();
    expect(graphite.mode, ThemeMode2.dark);
    expect(graphite.surface0, const Color(0xff1c1c1b));
    expect(graphite.surface4, const Color(0xff454542));
    expect(graphite.textPrimary, const Color(0xfff0efeb));
    expect(graphite.hairline, const Color(0xff353532));
    expect(graphite.accent, const Color(0xffe8712a));
    expect(graphite.animated, graphite.accent);
    expect(graphite.viewerSurround, grey.viewerSurround,
        reason: 'the surround is the same in both rooms');
    expect(graphite.layer.sequence, const Color(0xff74839d));
    expect(graphite.curve, [
      graphite.layer.footage,
      graphite.layer.precomp,
      graphite.layer.text,
      graphite.layer.camera,
    ]);

    expect(LumitColorScheme.greyRoom.label, 'Grey room');
    expect(LumitColorScheme.graphite.label, 'Graphite');
    expect(LumitColorScheme.greyRoom.build().surface0, grey.surface0);
    expect(LumitColorScheme.graphite.build().surface0, graphite.surface0);
  });

  test('label colours cycle over one distinct chip per layer kind', () {
    final t = LumitTheme.dark();
    // A dedicated bright palette, not the theme's role colours: the
    // chips colour the lane bars, so they must be tellable apart. Nine since
    // the Null layer got one of its own.
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
    // docs/15-DESIGN §7.1 sets 11px Hanken Grotesk (regular) for body copy, menus and
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

  test('the type scale sits at the design sizes', () {
    // docs/15-DESIGN §7.1: 11px body, 10px small, 9px caption — one step
    // tighter than the original scale, by owner request.
    final t = LumitTheme.dark();
    expect(t.body.fontSize, 11);
    expect(t.bodyPrimary.fontSize, 11);
    expect(t.small.fontSize, 10);
    expect(t.caption.fontSize, 9);
    expect(t.heading.fontSize, 16);
  });

  test('both faces are named by the theme and bundled by pubspec', () {
    // A family the theme asks for but pubspec never declares renders as the
    // platform default and nothing complains, so pin the pair together.
    expect(LumitTheme.fontFamily, 'Hanken Grotesk');
    expect(LumitTheme.dark().mono.fontFamily, 'Geist Mono');
    final pubspec = File('pubspec.yaml').readAsStringSync();
    expect(pubspec, contains('family: Hanken Grotesk'));
    expect(pubspec, contains('family: Geist Mono'));
  });
}
