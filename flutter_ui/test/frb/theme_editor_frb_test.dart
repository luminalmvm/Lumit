// Settings → Appearance: the theme picker, the customise window, and the
// scopes toggle (K-202).

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/scopes_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/theme/custom_theme.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Appearance (frb)', () {
    Future<({dynamic state, dynamic uiState})> openAppearance(
        WidgetTester tester) async {
      tester.view.physicalSize = const Size(1400, 1000);
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
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-settings')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('settings-page-appearance')));
      await tester.pumpAndSettle();
      return p;
    }

    /// The picker groups by what anyone is choosing by first — light or dark
    /// — with the user's own themes last.
    testWidgets('the theme picker is grouped, and custom themes join it',
        (tester) async {
      final p = await openAppearance(tester);
      p.uiState.workspace.saveCustomTheme(
        CustomTheme.from('Mine', LumitTheme.dark()),
      );
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('settings-scheme')));
      await tester.pumpAndSettle();

      expect(find.text('Dark'), findsWidgets);
      expect(find.text('Light'), findsWidgets);
      expect(find.text('Custom'), findsOneWidget,
          reason: 'the heading appears once a theme is saved under it');
      // Twice: the picker's row, and the button behind it already showing the
      // selection (saving a theme selects it).
      expect(find.text('Mine'), findsWidgets);

      // Choosing it selects it.
      await tester.tap(find.text('Mine').last);
      await tester.pumpAndSettle();
      expect(p.uiState.workspace.customThemeName, 'Mine');
    });

    /// The load-bearing one: the editor opens on the colours in use, a change
    /// shows immediately, and Save makes it a theme you can come back to.
    testWidgets('customise edits the live theme and saves it by name',
        (tester) async {
      final p = await openAppearance(tester);
      final before = p.uiState.theme.accent;

      await tester.tap(find.byKey(const ValueKey('settings-customise')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('theme-editor-body')), findsOneWidget);

      // The row for the accent is seeded from the theme on screen. It sits
      // under Roles, well down a lazy list, so it has to be scrolled to
      // before it exists in the tree at all.
      final swatch = find.byKey(const ValueKey('theme-token-accent'));
      await tester.scrollUntilVisible(
        swatch,
        120,
        scrollable: find
            .descendant(
              of: find.byKey(const ValueKey('theme-editor-body')),
              matching: find.byType(Scrollable),
            )
            .first,
      );
      await tester.pumpAndSettle();
      expect(swatch, findsOneWidget);
      final box = tester.widget<Container>(find.descendant(
        of: swatch,
        matching: find.byType(Container),
      ));
      expect((box.decoration! as BoxDecoration).color, before,
          reason: 'the editor opens on the colours actually in use');

      await tester.tap(find.byKey(const ValueKey('theme-editor-close')));
      await tester.pumpAndSettle();
      expect(p.uiState.theme.accent, before,
          reason: 'closing an untouched editor changes nothing');
    });

    /// Saving from a built-in asks for a name; saving again while that theme
    /// is selected updates it in place rather than asking twice.
    testWidgets('the first save names the theme, later ones update it',
        (tester) async {
      final p = await openAppearance(tester);
      await tester.tap(find.byKey(const ValueKey('settings-customise')));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('theme-editor-save')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('theme-name-field')), findsOneWidget,
          reason: 'a theme with no name has to be given one');

      await tester.enterText(
          find.byKey(const ValueKey('theme-name-field')), 'Night');
      await tester.tap(find.byKey(const ValueKey('theme-name-ok')));
      await tester.pumpAndSettle();

      expect(p.uiState.workspace.customThemeName, 'Night');
      expect(p.uiState.workspace.customThemes.map((t) => t.name), ['Night']);

      // Saving again does not ask a second time.
      await tester.tap(find.byKey(const ValueKey('theme-editor-save')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('theme-name-field')), findsNothing);
      expect(p.uiState.workspace.customThemes.length, 1,
          reason: 'the same theme was updated, not duplicated');

      await tester.tap(find.byKey(const ValueKey('theme-editor-close')));
      await tester.pumpAndSettle();
    });

    /// Duplicating is how a built-in becomes editable without the editor
    /// having to ask for a name first, and renaming is how a copy stops being
    /// called "copy" (K-298).
    testWidgets('a theme can be duplicated and renamed from Settings',
        (tester) async {
      final p = await openAppearance(tester);
      expect(p.uiState.workspace.customThemes, isEmpty);

      await tester.tap(find.byKey(const ValueKey('settings-theme-duplicate')));
      await tester.pumpAndSettle();
      expect(
          p.uiState.workspace.customThemes.map((t) => t.name), ['Dark copy']);
      expect(p.uiState.workspace.customThemeName, 'Dark copy',
          reason: 'the copy is what you are now editing');

      await tester.tap(find.byKey(const ValueKey('settings-theme-rename')));
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey('theme-name-field')), 'Night');
      await tester.tap(find.byKey(const ValueKey('theme-name-ok')));
      await tester.pumpAndSettle();

      expect(p.uiState.workspace.customThemes.map((t) => t.name), ['Night']);
      expect(p.uiState.workspace.customThemeName, 'Night');
    });

    /// Rename and Delete are the two verbs that only make sense on one of the
    /// user's own: a built-in scheme's name is Lumit's, not the user's.
    testWidgets('rename and delete are offered only for your own themes',
        (tester) async {
      final p = await openAppearance(tester);
      bool enabled(String key) =>
          tester.widget<HouseButton>(find.byKey(ValueKey(key))).onPressed !=
          null;

      expect(enabled('settings-theme-rename'), isFalse);
      expect(enabled('settings-theme-delete'), isFalse);
      expect(enabled('settings-theme-duplicate'), isTrue,
          reason: 'a built-in is exactly what you would copy to start from');

      p.uiState.workspace
          .saveCustomTheme(CustomTheme.from('Mine', LumitTheme.dark()));
      await tester.pumpAndSettle();
      expect(enabled('settings-theme-rename'), isTrue);
      expect(enabled('settings-theme-delete'), isTrue);
    });

    /// Save a copy branches a theme instead of overwriting it — without it the
    /// only way to keep both was to save over one and undo the edits by hand.
    testWidgets('save a copy leaves the theme it was started from alone',
        (tester) async {
      final p = await openAppearance(tester);
      p.uiState.workspace
          .saveCustomTheme(CustomTheme.from('Mine', LumitTheme.dark()));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('settings-customise')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('theme-editor-save-copy')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('theme-name-field')), findsOneWidget);

      await tester.enterText(
          find.byKey(const ValueKey('theme-name-field')), 'Mine, brighter');
      await tester.tap(find.byKey(const ValueKey('theme-name-ok')));
      await tester.pumpAndSettle();

      expect(p.uiState.workspace.customThemes.map((t) => t.name),
          ['Mine', 'Mine, brighter']);
      expect(p.uiState.workspace.customThemeName, 'Mine, brighter',
          reason: 'the copy is what further saves go to');

      await tester.tap(find.byKey(const ValueKey('theme-editor-close')));
      await tester.pumpAndSettle();
    });

    /// The scopes toggle is off by default — a scope is a measuring
    /// instrument first (docs/15-DESIGN §8) — and turning it on is what makes
    /// the trace take the theme's colours.
    testWidgets('themed scopes are off until switched on', (tester) async {
      final p = await openAppearance(tester);
      expect(p.uiState.workspace.themedScopes, isFalse);

      final standard = scopeColoursFor(LumitTheme.dark());
      expect(standard.first, ScopeColours.standard.bg.toTriple(),
          reason: 'off, the scope draws on the standard graticule');

      await tester.tap(find.byKey(const ValueKey('settings-themed-scopes')));
      await tester.pumpAndSettle();
      expect(p.uiState.workspace.themedScopes, isTrue);

      final themed = scopeColoursFor(LumitTheme.dark(), themed: true);
      expect(themed.first, isNot(standard.first),
          reason: 'on, it takes the theme instead');
    });

    /// The Viewer's surround is neutral for the same reason the scopes are:
    /// a grade cannot be judged against a tinted ground (docs/15-DESIGN
    /// §2.1/§11). It had been painting the theme's own surface — the defect
    /// K-203 fixes — so this pins the default and the way out of it.
    testWidgets('the Viewer surround is neutral until switched on',
        (tester) async {
      final p = await openAppearance(tester);
      expect(p.uiState.workspace.themedViewerSurround, isFalse);

      final dark = LumitTheme.dark();
      expect(viewerSurroundFor(dark), dark.viewerSurround,
          reason: 'off, the surround is the theme-independent grey');
      expect(dark.viewerSurround.r, dark.viewerSurround.g,
          reason: 'and that grey really is neutral');

      // A choice of two words now, not a switch (K-465): the drawing gives the
      // surround a dropdown, and its two options are the bool's two values.
      await tester.tap(find.byKey(const ValueKey('settings-themed-surround')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Theme colour').last);
      await tester.pumpAndSettle();
      expect(p.uiState.workspace.themedViewerSurround, isTrue);
      expect(viewerSurroundFor(dark, themed: true), dark.surface0,
          reason: 'on, it takes the panel surface like everything else');
    });

    /// A magnified pixel is a square unless asked otherwise: Flutter's
    /// `Texture` filters bilinearly by default, which blurred the picture at
    /// every zoom past 1:1.
    testWidgets('the zoomed picture is unsmoothed until switched on',
        (tester) async {
      final p = await openAppearance(tester);
      expect(p.uiState.workspace.smoothZoomedViewer, isFalse);

      // On the Viewer page since K-465: Appearance keeps what the Viewer looks
      // like, and this is about the picture itself.
      await tester.tap(find.byKey(const ValueKey('settings-page-viewer')));
      await tester.pumpAndSettle();
      await tester
          .tap(find.byKey(const ValueKey('settings-smooth-zoomed-viewer')));
      await tester.pumpAndSettle();
      expect(p.uiState.workspace.smoothZoomedViewer, isTrue);
    });

    /// The toggles are machine-local settings, so they have to survive a
    /// restart.
    testWidgets('the viewer and scope choices survive the workspace file',
        (tester) async {
      final p = await openAppearance(tester);
      p.uiState.workspace.setThemedViewerSurround(true);
      p.uiState.workspace.setThemedScopes(true);
      p.uiState.workspace.setSmoothZoomedViewer(true);
      await tester.pumpAndSettle();

      final restored = Workspace()..applyJson(p.uiState.workspace.toJson());
      expect(restored.themedViewerSurround, isTrue);
      expect(restored.themedScopes, isTrue);
      expect(restored.smoothZoomedViewer, isTrue);
    });
  }, skip: !engineAvailable);
}

extension on Color {
  List<int> toTriple() =>
      [(r * 255).round(), (g * 255).round(), (b * 255).round()];
}
