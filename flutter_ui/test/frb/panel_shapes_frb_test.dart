// The Project panel and the dialog pattern under Desk and Lantern: the few
// places their chrome asks which shape it is in, asserted under each so a
// change that reaches the other shapes fails here rather than on screen.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/lumit_icon.dart' as glyph;
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/shell/comp_settings_frb.dart';
import 'package:lumit_flutter/shell/dialog_frame.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  Rect band(WidgetTester tester, String key) =>
      tester.getRect(find.byKey(ValueKey<String>(key)));

  /// The corner of the first decorated box under [of].
  BorderRadius? radiusOf(WidgetTester tester, Finder of) {
    final box = tester.widget<Container>(
        find.descendant(of: of, matching: find.byType(Container)).first);
    return (box.decoration as BoxDecoration).borderRadius as BorderRadius?;
  }

  /// The colour a house button paints its word in.
  Color inkOf(WidgetTester tester, Finder button) => tester
      .widget<DefaultTextStyle>(find
          .descendant(of: button, matching: find.byType(DefaultTextStyle))
          .first)
      .style
      .color!;

  group('Project panel', () {
    /// A project with one missing clip, so a row carries a badge.
    Future<void> mount(WidgetTester tester, ThemeShape shape) async {
      tester.view.physicalSize = const Size(480, 760);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      p.state.project!.importFootage(path: 'C:/nowhere/gone.mp4');
      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        shape: shape,
      ));
      await settleFrb(
        tester,
        until: () => find.byType(ProjectBadge).evaluate().isNotEmpty,
      );
    }

    final footer = find.byKey(const ValueKey<String>('project-footer'));
    final newFolder = find.byKey(const ValueKey<String>('project-new-folder'));

    testWidgets('under Lantern the bottom line is pills and the badge is one',
        (tester) async {
      await mount(tester, ThemeShape.lantern);

      expect(tester.widget(newFolder), isA<HouseButton>(),
          reason: 'the new-item controls are ghost pills');
      expect(find.descendant(of: footer, matching: find.text('FOLDER')),
          findsOneWidget);
      expect(radiusOf(tester, newFolder),
          BorderRadius.circular(ShapeTokens.stadium));
      expect(radiusOf(tester, find.byType(ProjectBadge)),
          BorderRadius.circular(ShapeTokens.stadium));
      expect(find.byKey(const ValueKey<String>('project-seam-items')),
          findsOneWidget,
          reason: 'the column-header seams stay');
    });

    testWidgets('under Desk the words go lowercase and nothing else moves',
        (tester) async {
      await mount(tester, ThemeShape.desk);

      expect(tester.widget(newFolder), isA<GestureDetector>(),
          reason: 'bare glyph and word, as under Studio');
      expect(find.descendant(of: footer, matching: find.text('folder')),
          findsOneWidget);
      expect(tester.getRect(newFolder).height, projectFooterIconSize);
      expect(radiusOf(tester, find.byType(ProjectBadge)),
          BorderRadius.circular(ShapeTokens.desk.actionRadius));
    });
  });

  group('Dialog pattern', () {
    Future<LumitTheme> mount(WidgetTester tester, ThemeShape shape) async {
      tester.view.physicalSize = const Size(800, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      late LumitTheme theme;
      await tester.pumpWidget(hostPanel(
        child: Builder(builder: (context) {
          final t = theme = ThemeScope.of(context).theme;
          return Center(
            child: DialogFrame(width: 400, children: [
              dialogTitleBar(t,
                  title: 'Export',
                  subject: 'Scene',
                  onClose: () {},
                  keyPrefix: 'probe'),
              dialogGroup(t, 'Output', [const SizedBox(height: 20)],
                  key: const ValueKey<String>('probe-group')),
              dialogFooter(t, keyPrefix: 'probe', actions: [
                HouseButton(
                    key: const ValueKey<String>('probe-go'),
                    primary: true,
                    onPressed: () {},
                    child: const Text('Export')),
              ]),
            ]),
          );
        }),
        state: p.state,
        uiState: p.uiState,
        size: const Size(800, 600),
        shape: shape,
      ));
      await tester.pump();
      return theme;
    }

    final go = find.byKey(const ValueKey<String>('probe-go'));
    final dot = find.byKey(const ValueKey<String>('probe-title-dot'));

    testWidgets('under Lantern the title is centred behind its dot',
        (tester) async {
      final t = await mount(tester, ThemeShape.lantern);

      final strip = band(tester, 'probe-title-strip');
      final title = tester.getRect(find.descendant(
          of: find.byKey(const ValueKey<String>('probe-title-strip')),
          matching: find.text('EXPORT')));
      expect(dot, findsOneWidget);
      expect(tester.getRect(dot).right, lessThanOrEqualTo(title.left),
          reason: 'the dot sits before the title');
      expect(
          (title.center.dx - strip.center.dx).abs(), lessThan(strip.width / 4),
          reason: 'the title sits in the middle of the strip');
      final mark = tester.getRect(find.descendant(
        of: find.byKey(const ValueKey<String>('probe-close')),
        matching: find.byType(glyph.LumitIcon),
      ));
      expect(strip.right - mark.right, closeTo(dialogPadding, 0.01),
          reason: 'the close mark keeps its corner');
      expect(
          radiusOf(tester, find.byKey(const ValueKey<String>('probe-group'))),
          BorderRadius.circular(t.tokens.sectionRadius));
      expect(t.tokens.sectionRadius, 12);
      expect(inkOf(tester, go), t.textPrimary);
    });

    testWidgets('under Desk the title is the lowercase label at the left',
        (tester) async {
      final t = await mount(tester, ThemeShape.desk);

      final strip = band(tester, 'probe-title-strip');
      final title = tester.getRect(find.descendant(
          of: find.byKey(const ValueKey<String>('probe-title-strip')),
          matching: find.text('export')));
      expect(dot, findsNothing);
      expect(title.left, closeTo(strip.left + dialogPadding, 0.01));
      expect(
          radiusOf(tester, find.byKey(const ValueKey<String>('probe-group'))),
          BorderRadius.circular(ShapeTokens.desk.sectionRadius));
      expect(inkOf(tester, go), t.surface0);
    });

    /// A dialog that draws a box of its own wears the same section corner
    /// as the titled groups, so Lantern's dialogs agree with themselves.
    testWidgets(
        'under Lantern the comp settings swatch takes the section corner',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => GestureDetector(
            key: const ValueKey('open'),
            behavior: HitTestBehavior.opaque,
            onTap: () => showCompSettingsFrb(context: context, comp: comp),
            child: const SizedBox(width: 200, height: 40),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
        shape: ThemeShape.lantern,
      ));
      await tester.tap(find.byKey(const ValueKey('open')));
      await tester.pumpAndSettle();

      final t = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.lantern);
      expect(
          radiusOf(
              tester, find.byKey(const ValueKey<String>('comp-background'))),
          BorderRadius.circular(t.tokens.sectionRadius));

      await tester.tap(find.byKey(const ValueKey('comp-cancel')));
      await tester.pumpAndSettle();
    });
  });
}
