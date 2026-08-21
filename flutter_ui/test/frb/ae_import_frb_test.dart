// The After Effects import, end to end on the real engine (docs/11-AE-IMPORT.md,
// docs/impl/ae-import.md §6 phase 3).
//
// One test drives the whole surface the way a person does: File ▸ Import After
// Effects bundle…, a folder chosen, the engine reads it, the project it built
// becomes the open one, and the report window says what did not come across
// whole. The bundle is `crates/lumit-import/tests/fixtures/synthetic.lum-bundle`
// — referenced where it lies rather than copied, so the fixture the Rust tests
// pin is the fixture the panel is proved against.
//
// **This file's import clears the engine's process-wide project registry**, the
// way `openProject` does, because an import *is* an open (see `api::state::adopt`).
// It therefore lives on its own rather than among tests holding references.
//
// The footage the fixture names — `/media/clip.mp4` — is deliberately not on any
// machine, which is what makes the relink path visible here: it must import as
// an offline item with a row saying so, and must never hold the import up
// (docs/11 §2.5).

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/ae_report_frb.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/src/rust/api/import.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';

import 'frb_test_support.dart';

/// The hand-written bundle the `lumit-import` tests use, as an absolute path:
/// the engine resolves the footage inside it against this folder.
String get _bundle =>
    Directory('../crates/lumit-import/tests/fixtures/synthetic.lum-bundle')
        .absolute
        .path;

void main() {
  setUpAll(initEngineForTests);

  group('After Effects import (frb)', () {
    Future<({LumitState state, LumitUiState uiState})> mount(
      WidgetTester tester, {
      required Future<String?> Function() bundlePicker,
    }) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        size: const Size(800, 600),
        child: Builder(builder: (context) {
          final state = context.watch<LumitState>();
          context.watch<LumitUiState>();
          return LumitMenuBarFrb(app: state, bundlePicker: bundlePicker);
        }),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      return p;
    }

    Future<void> choose(WidgetTester tester, String menu, String item) async {
      await tester.tap(find.byKey(ValueKey<String>('menu-$menu')));
      await tester.pump();
      // The AE route lives under the File menu's Import submenu.
      await tester.tap(find.text(l10n.menuImport));
      await tester.pump();
      await tester.ensureVisible(find.text(item));
      await tester.pump();
      await tester.tap(find.text(item));
      await tester.pump();
    }

    /// Every item in the project, folders flattened.
    List<String> itemNames(LumitState state) {
      List<ItemReference> walk(List<ItemReference> items) => [
            for (final i in items) ...[
              i,
              if (i is ItemReference_Folder) ...walk(i.field0.getChildren()),
            ]
          ];
      return [
        for (final i in walk(state.project?.getItems() ?? const []))
          i.name(),
      ];
    }

    testWidgets('a folder that is not a bundle leaves the project alone',
        (tester) async {
      final elsewhere = Directory.systemTemp.createTempSync('lumit-not-bundle');
      final p = await mount(tester, bundlePicker: () async => elsewhere.path);
      final before = p.state.project;

      await choose(tester, 'File', l10n.menuImportAe);
      await settleFrb(tester,
          until: () => p.state.notice.value != null);

      expect(identical(p.state.project, before), isTrue,
          reason: 'the open project stands; an import that cannot read its '
              'folder is the picker\'s problem, not the document\'s');
      expect(p.state.notice.value?.error, isTrue);
      expect(find.text(l10n.aeReportTitle), findsNothing,
          reason: 'there is no report for an import that never happened');
    });

    // LAST: adopting the imported document clears the engine's project
    // registry, so every reference held above dies here.
    testWidgets('a bundle imports, and the report says what changed',
        (tester) async {
      final p = await mount(tester, bundlePicker: () async => _bundle);
      final before = p.state.project;

      await choose(tester, 'File', l10n.menuImportAe);
      await settleFrb(tester,
          until: () => find.text(l10n.aeReportTitle).evaluate().isNotEmpty);

      // The document arrived: both of the fixture's compositions, and the
      // footage item whose file is nowhere — offline, not omitted.
      expect(identical(p.state.project, before), isFalse,
          reason: 'the imported project was adopted');
      final names = itemNames(p.state);
      expect(names, containsAll(<String>['Main', 'Nested', 'clip.mp4']));

      // The report is up, with docs/11 §9's summary line over its rows.
      expect(find.text(l10n.aeReportTitle), findsOneWidget);
      expect(find.text(l10n.aeSummary(16, 13, 1, 1)), findsOneWidget,
          reason: 'the four counts of the synthetic bundle. A change here is a '
              'change in what the mapping does, not in what the panel shows');

      // Rows are sentences written on this side from the engine's id and its
      // facts, not the engine's English handed through (K-303). The first row
      // is the one that reads a pair of booleans and picks its phrasing.
      expect(find.text(l10n.aeNestedPreserveRate), findsOneWidget);
      expect(find.textContaining('nested_preserve_ignored'), findsNothing,
          reason: 'the id is a key, never something a person reads');

      // Filtering narrows to one grade and back. Both of these are single-row
      // grades, so the whole answer is on screen.
      await tester.tap(find.byKey(const ValueKey('ae-filter-placeholder')));
      await tester.pump();
      expect(find.text(l10n.aeEffectPlaceholder('ADBE CurvesCustom')),
          findsOneWidget);
      expect(find.text(l10n.aeNestedPreserveRate), findsNothing,
          reason: 'an adjusted row is not a placeholder');

      await tester.tap(find.byKey(const ValueKey('ae-filter-skipped')));
      await tester.pump();
      expect(find.text(l10n.aePropertyUnreadable('ADBE CurvesCustom-0001')),
          findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('ae-filter-all')));
      await tester.pump();
      expect(find.text(l10n.aeNestedPreserveRate), findsOneWidget);

      // The relink's own row is the last one — the list scrolls to it rather
      // than the panel being asked to draw fourteen rows at once.
      await tester.drag(
          find.byKey(const ValueKey('ae-report-rows')), const Offset(0, -600));
      await tester.pump();
      expect(find.text(l10n.aeMediaNotFound), findsOneWidget,
          reason: 'missing media is reported, and never blocks the import');

      // Every grade the engine can send is named in the reader's language.
      for (final grade in BridgeImportOutcome.values) {
        expect(outcomeLabel(grade), isNotEmpty);
      }

      await tester.tap(find.text(l10n.close));
      await tester.pump();
      expect(find.text(l10n.aeReportTitle), findsNothing);
    });
  }, skip: !engineAvailable);
}
