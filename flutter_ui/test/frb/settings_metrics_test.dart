// The Settings window measured against the approved drawing, band by band.
//
// **Why this file exists.** `shell_frb_test` is about what the window *does* —
// what a button reaches, what a drag sets, which page a control lives on. This
// one is about what it *looks like*, and specifically about the numbers the
// drawing's own computed styles resolved to (K-465): the frame, the title
// strip, the sidebar, a section, a row, the controls in it, the footer. Nothing
// here names a private widget class, because none of these claims is about how
// the window is built — each is something a person could point at on screen and
// measure with a ruler.
//
// A value that disagrees with the drawing is a defect, so each expectation
// carries the drawing's own number in its reason.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/settings_rows.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Settings metrics (frb)', () {
    final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);

    /// Open the window the way the application does, in a view large enough to
    /// hold it at the size it asks for.
    Future<dynamic> open(WidgetTester tester) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-settings'),
            onPressed: () => showSettingsWindowFrb(context),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1200, 900),
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-settings')));
      await tester.pumpAndSettle();
      return p;
    }

    Future<void> showAppearance(WidgetTester tester) async {
      await tester.tap(find.byKey(const ValueKey('settings-page-appearance')));
      await tester.pumpAndSettle();
    }

    Rect band(WidgetTester tester, String key) =>
        tester.getRect(find.byKey(ValueKey<String>(key)));

    /// The box a piece of text sits in, [depth] ancestors up — the label column
    /// round a row's name, the band round a section's kicker.
    Rect boxRound(WidgetTester tester, String text, Type type) =>
        tester.getRect(find
            .ancestor(of: find.text(text), matching: find.byType(type))
            .first);

    /// 1. **The frame.** 760×520 with a 30px title strip over its hairline and
    /// a 43px footer under it, which leaves the drawing's 446 for the page.
    testWidgets('the window is the drawing\'s frame', (tester) async {
      await open(tester);

      final title = band(tester, 'settings-title-strip');
      final footer = band(tester, 'settings-footer');
      expect(title.width, settingsWindowSize.width,
          reason: 'the drawing frames the window at 760 wide');
      expect(title.height, settingsTitleStrip + 1,
          reason: '§12A.4: a dialog title strip is 30, over a hairline');
      expect(footer.height, settingsFooterHeight,
          reason: '8 above a 26px button and 8 below it, over a hairline');
      expect(footer.bottom - title.top, settingsWindowSize.height,
          reason: 'the drawing frames the window at 520 tall');
      expect(footer.top - title.bottom, 446,
          reason: '520 less the 31 of title strip and the 43 of footer');
    });

    /// 2. **The title strip.** A kicker, the search well at the drawing's
    /// 174×20, and the close mark at the size the drawing renders it (K-456).
    testWidgets('the title strip carries a kicker, a search and a close',
        (tester) async {
      await open(tester);

      final strip = tester.widget<Container>(
          find.byKey(const ValueKey('settings-title-strip')));
      expect((strip.decoration! as BoxDecoration).color, t.surface2,
          reason: 'the drawing computes the strip a shade above the page');

      final kicker = tester.widget<Text>(find.text('SETTINGS'));
      expect(kicker.style!.fontFamily, LumitTheme.monoFontFamily);
      expect(kicker.style!.fontSize, t.kicker.fontSize,
          reason:
              'a dialog title is a kicker like every other container label');
      expect(kicker.style!.color, t.textPrimary);

      final search = band(tester, 'settings-search');
      expect(search.width, settingsSearchWidth);
      expect(search.height, settingsSearchHeight,
          reason:
              'the drawing renders the search well 20 tall, not a well\'s 22');

      expect(band(tester, 'settings-close').height, settingsTitleStrip,
          reason:
              'the close mark takes the strip\'s full height as its target');
    });

    /// 3. **The sidebar.** 160 wide including its rule, entries 24 tall, and
    /// the page in force marked by a 2px accent tick rather than a fill of it.
    testWidgets('the sidebar is the drawing\'s column of pages',
        (tester) async {
      await open(tester);
      await showAppearance(tester);

      final entry = band(tester, 'settings-page-general');
      expect(entry.width, settingsSidebarWidth - 1,
          reason: '160 of column, one of which is the rule beside the page');
      expect(entry.height, settingsNavRow, reason: 'the drawing draws 24');

      final on = tester.widget<Container>(find
          .descendant(
            of: find.byKey(const ValueKey('settings-page-appearance')),
            matching: find.byType(Container),
          )
          .first);
      final decoration = on.decoration! as BoxDecoration;
      expect(decoration.color, t.surface2);
      expect((decoration.border! as Border).left.color, t.accent);
      expect((decoration.border! as Border).left.width, settingsNavTick);
    });

    /// 4. **A section, and a row.** The drawing's grid: a 30px kicker band,
    /// then rows of 30 whose label column is 190 wide.
    testWidgets('a section and its rows are the drawing\'s grid',
        (tester) async {
      await open(tester);
      await showAppearance(tester);

      expect(boxRound(tester, 'THEME', SizedBox).height,
          settingsSectionHeaderHeight,
          reason: '12 above the kicker, its line, 4 below');

      final row = boxRound(tester, 'Colour scheme', ConstrainedBox);
      expect(row.height, settingsRowHeight,
          reason: '§12A.4: a dialog row is 30');
      expect(row.width, settingsWindowSize.width - settingsSidebarWidth,
          reason: 'the page is what the sidebar leaves: 600');
      expect(boxRound(tester, 'Colour scheme', SizedBox).width,
          settingsLabelColumn,
          reason: 'the drawing fixes the label column at 190');
    });

    /// 5. **The controls.** A dialog's dropdown and well are 22 (§12A.6), the
    /// closed face reads `body` on `surface2`, and the switch is the drawing's
    /// 22×12 pill in `animated` — never the accent (§3.1).
    testWidgets('the controls are the drawing\'s sizes and colours',
        (tester) async {
      final p = await open(tester);
      await showAppearance(tester);

      final scheme = band(tester, 'settings-scheme');
      expect(scheme.height, settingsControlHeight,
          reason: '§12A.6: a dialog dropdown is 22');
      expect(scheme.width, 180, reason: 'the drawing\'s wide face');

      final face = tester.widget<DefaultTextStyle>(find
          .descendant(
            of: find.byKey(const ValueKey('settings-scheme')),
            matching: find.byType(DefaultTextStyle),
          )
          .first);
      expect(face.style.color, t.body.color,
          reason: 'a closed face reads secondary, not primary');
      expect(face.style.fontSize, t.body.fontSize);

      final toggle = band(tester, 'settings-compact');
      expect(toggle.width, 26, reason: 'a 22px pill in its focus box');
      expect(toggle.height, 16, reason: 'a 12px pill in its focus box');

      // On, it is the amber the drawing computes — which is the `animated`
      // token, and not the accent.
      await tester.tap(find.byKey(const ValueKey('settings-compact')));
      await tester.pumpAndSettle();
      expect(p.uiState.workspace.interface.compact, isTrue);
      final pill = tester.widget<AnimatedContainer>(find
          .descendant(
            of: find.byKey(const ValueKey('settings-compact')),
            matching: find.byType(AnimatedContainer),
          )
          .first);
      expect((pill.decoration! as BoxDecoration).color, t.animated);
    });

    /// 6. **The accent row.** Five swatches of 14, the one in force ringed,
    /// and the hex of whatever the accent actually is beside them.
    testWidgets('the accent row is five swatches and a hex', (tester) async {
      final p = await open(tester);
      await showAppearance(tester);

      expect(LumitTheme.accentPresets.length, 5);
      for (final colour in LumitTheme.accentPresets) {
        final swatch = tester.getRect(
            find.byKey(ValueKey<String>('settings-accent-${_hex(colour)}')));
        expect(swatch.width, 14);
        expect(swatch.height, 14);
      }

      expect(find.byKey(const ValueKey('settings-accent-hex')), findsOneWidget);
      final hex = tester
          .widget<Text>(find.byKey(const ValueKey('settings-accent-hex')));
      expect(hex.style!.fontFamily, LumitTheme.monoFontFamily);
      expect(hex.style!.fontSize, 10,
          reason: '§7.1: a unit rider is 10px mono');
      expect(hex.data, '#e05a72', reason: 'the dark scheme\'s own accent');

      // A swatch sets the accent, and the readout follows it.
      await tester.tap(find.byKey(ValueKey<String>(
          'settings-accent-${_hex(LumitTheme.accentPresets[1])}')));
      await tester.pumpAndSettle();
      expect(p.uiState.workspace.accentOverride, LumitTheme.accentPresets[1]);
    });

    /// 7. **The footer.** Its note, and two buttons of the drawing's 26.
    testWidgets('the footer says what it says, on 26px buttons',
        (tester) async {
      await open(tester);

      expect(find.text('Changes apply immediately'), findsOneWidget);
      expect(band(tester, 'settings-reset-page').height, settingsFooterButton);
      expect(
          band(tester, 'settings-close-button').height, settingsFooterButton);
    });

    /// 8. **Appearance reads in the order the work happens in**: pick a
    /// scheme, make it your own, then tune whichever one is in force. Accent
    /// and shape sat above the rows that create the thing they tune.
    testWidgets('the Appearance rows are in the owner\'s order',
        (tester) async {
      await open(tester);
      await showAppearance(tester);

      double topOf(String label) => boxRound(tester, label, ConstrainedBox).top;
      final order = [
        'Colour scheme',
        'Custom colours',
        'Your themes',
        'Accent',
        'Shape',
      ];
      final tops = [for (final row in order) topOf(row)];
      for (var i = 1; i < order.length; i++) {
        expect(tops[i], greaterThan(tops[i - 1]),
            reason: '${order[i]} must read after ${order[i - 1]}');
      }
    });

    /// 9. **Tooltips are a switch, not a picker** (K-476), and the switch
    /// survives the trip out to the settings file and back.
    testWidgets('the tooltip setting is a switch that round-trips',
        (tester) async {
      final p = await open(tester);
      await showAppearance(tester);

      expect(find.byKey(const ValueKey('settings-tooltips')), findsOneWidget);
      expect(find.text('Short'), findsNothing,
          reason: 'there is no longer form to choose, so no picker');
      final pill = band(tester, 'settings-tooltips');
      expect(pill.width, 26, reason: 'the drawing\'s pill, as every flag row');
      expect(pill.height, 16);

      expect(p.uiState.workspace.interface.showTooltips, isTrue);
      await tester.tap(find.byKey(const ValueKey('settings-tooltips')));
      await tester.pumpAndSettle();
      expect(p.uiState.workspace.interface.showTooltips, isFalse);

      // Out to the settings file and back: off has to still be off next time.
      final settings = p.uiState.workspace.interface;
      expect(
          InterfaceSettings.fromJson(settings.toJson()).showTooltips, isFalse);
    });

    /// 10. **The scale row's note wraps.** The drawing measures it at 107 wide
    /// and 26 tall — two lines of it — rather than one line clipped short.
    testWidgets('the scale note reads on two lines', (tester) async {
      await open(tester);
      await showAppearance(tester);

      final note = find.text('applies on release');
      // The `%` rider beside it is one line of the same style, so twice its
      // height is what "wrapped onto two lines" means without hard-coding a
      // font's metrics.
      final oneLine = tester.getRect(find.text('%')).height;
      expect(tester.getRect(note).height, closeTo(oneLine * 2, 1),
          reason: 'the drawing draws the note on two lines, not one clipped '
              'to fit');
    });
  }, skip: !engineAvailable);
}

/// The drawing's own way of writing a colour, matching the window's.
String _hex(Color c) {
  String pair(double channel) =>
      (channel * 255).round().toRadixString(16).padLeft(2, '0');
  return '#${pair(c.r)}${pair(c.g)}${pair(c.b)}';
}
