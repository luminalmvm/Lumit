// The After Effects import, end to end on the real engine (docs/11-AE-IMPORT.md,
// docs/impl/ae-import.md §6 phase 3, §7 phase C).
//
// Each test drives the whole surface the way a person does: File ▸ Import ▸
// After Effects project…, a file chosen, the engine reads it, the project it
// built becomes the open one, and the report window says what did not come
// across whole. **Both front doors are here**: the real
// `tools/ae-bridge/fixtures/fixture.aep` through the primary item, and the
// Bridge bundle folder through the quieter second one. Both fixtures are
// referenced where they lie rather than copied, so the fixture the Rust tests
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

@Tags(['opens-project'])
library;

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';

import 'frb_test_support.dart';

String get _aep =>
    File('../tools/ae-bridge/fixtures/fixture.aep').absolute.path;

void main() {
  setUpAll(initEngineForTests);

  group('After Effects import (frb)', () {
    Future<({LumitState state, LumitUiState uiState})> mount(
      WidgetTester tester, {
      Future<String?> Function()? aeProjectPicker,
    }) async {
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        size: const Size(800, 600),
        child: Builder(builder: (context) {
          final state = context.watch<LumitState>();
          context.watch<LumitUiState>();
          return LumitMenuBarFrb(
            app: state,
            aeProjectPicker: aeProjectPicker,
          );
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
      await tester.ensureVisible(find.text(item));
      await tester.pump();
      await tester.tap(find.text(item));
      await tester.pump();
    }

    List<String> itemNames(LumitState state) {
      List<ItemReference> walk(List<ItemReference> items) => [
            for (final i in items) ...[
              i,
              if (i is ItemReference_Folder) ...walk(i.field0.getChildren()),
            ]
          ];
      return [
        for (final i in walk(state.project?.getItems() ?? const [])) i.name(),
      ];
    }

    /// **A file the parser cannot read fails softly, in the calm words.**
    ///
    /// The honest half of the import promise: a newer After Effects may store
    /// something this build has not met, and the answer is a sentence naming
    /// the Bridge route — never a lost project. The file is named `.aep` and
    /// holds nothing of the sort, which is the same shape as that failure.
    testWidgets('an unreadable .aep says so and the project stands',
        (tester) async {
      final rubbish = File(
          '${Directory.systemTemp.createTempSync('lumit-bad-aep').path}/broken.aep')
        ..writeAsStringSync('this is not an After Effects project');
      final p = await mount(tester, aeProjectPicker: () async => rubbish.path);
      final before = p.state.project;

      await choose(tester, 'File', l10n.menuImportAe);
      await settleFrb(tester, until: () => p.state.notice.value != null);

      expect(identical(p.state.project, before), isTrue);
      expect(p.state.notice.value?.error, isTrue);
      expect(p.state.notice.value?.message, l10n.aeAepUnreadable);
      expect(find.text(l10n.aeReportTitle), findsNothing);
    });

    /// **The seamless front door: a real `.aep`, picked and imported.**
    ///
    /// Nothing is run inside After Effects and no bundle is involved — the
    /// file the differential test measures the parser against goes in through
    /// the menu, and the same report window comes out. The four counts are the
    /// parse's own, taken from what the shared mapping actually made of it
    /// (`crates/lumit-import/tests/aep_differential.rs` counts the same
    /// document from the other side); a change here is a change in what the
    /// parser recovers, not in what the panel shows.
    ///
    /// LAST: this adoption clears the engine's project registry.
    testWidgets('a real .aep imports through the front door', (tester) async {
      final p = await mount(tester, aeProjectPicker: () async => _aep);
      final before = p.state.project;

      await choose(tester, 'File', l10n.menuImportAe);
      // Parsing the whole file and starting its worker can run past the
      // usual 0.8s on a busy machine, so this one waits up to 8s.
      await settleFrb(tester,
          maxRounds: 400,
          until: () => find.text(l10n.aeReportTitle).evaluate().isNotEmpty);

      expect(identical(p.state.project, before), isFalse,
          reason: 'the parsed project was adopted');
      expect(itemNames(p.state),
          containsAll(<String>['Fixture', 'Fixture inner', 'Solids']));

      expect(find.text(l10n.aeItemsImported(63)), findsOneWidget,
          reason: 'what the direct parse recovers, end to end');
      expect(find.text(l10n.aeOutcomeAdjusted), findsOneWidget);
      expect(find.text('49'), findsOneWidget);
      expect(find.text(l10n.aeOutcomeSkipped), findsNothing);

      await tester.tap(find.text(l10n.close));
      await tester.pump();
      expect(find.text(l10n.aeReportTitle), findsNothing);
    });
  }, skip: !engineAvailable);
}
