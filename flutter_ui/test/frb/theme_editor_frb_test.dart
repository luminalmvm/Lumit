// Settings → Appearance: the theme picker, the customise window, and the
// scopes toggle.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
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

  }, skip: !engineAvailable);
}

extension on Color {
}
