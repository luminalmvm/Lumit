// The four places a colour config reaches the interface (docs/impl/ocio.md
// §6.2–§6.5).
//
// `colour_seam_frb_test` proves the seam: the summary arrives whole, the two
// edits are ordinary ops, a refusal crosses as an id. This file is about the
// surfaces built on it — the Viewer's picker, the export's colour dropdown, the
// Project panel's per-item submenu, and the Project settings row where the
// config itself is chosen. One config fixture serves all four, because what is
// being asserted is that each surface says the *same* thing about it.
//
// Two claims run through every group and are the ones a regression would break
// quietly:
//
// * **The config's names cross verbatim.** They are the user's own words, out
//   of the user's own file, and no surface may put them through the label
//   table.
// * **Nothing here asks the engine during a build.** The summary reads a file
//   off disk; it is fetched when the document changes and held. The
//   bridge-call budget test is the gate; these tests keep the wiring that makes
//   it possible honest.

import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/project_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/shell/export_dialog_frb.dart';
import 'package:lumit_flutter/shell/project_settings_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

/// A small, complete config: four spaces, one display with one view, and the
/// roles the resolution walk reads. The same fixture `colour_seam_frb_test`
/// uses, so the two files agree about what "loaded" looks like.
const _config = '''
ocio_profile_version: 1
roles:
  scene_linear: lin
  reference: ref
displays:
  sRGB:
    - !<View> {name: Standard, colorspace: out_srgb}
colorspaces:
  - !<ColorSpace>
    name: ref
  - !<ColorSpace>
    name: lin
  - !<ColorSpace>
    name: srgb_texture
    to_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0]}
  - !<ColorSpace>
    name: out_srgb
    from_reference: !<ExponentWithLinearTransform> {gamma: [2.4, 2.4, 2.4, 1], offset: [0.055, 0.055, 0.055, 0], direction: inverse}
''';

void main() {
  setUpAll(initEngineForTests);

  late Directory dir;
  setUp(() => dir = Directory.systemTemp.createTempSync('lumit-ocio-ui'));
  tearDown(() => dir.deleteSync(recursive: true));

  /// Write the fixture and hand back its path.
  String writeConfig() {
    final file = File('${dir.path}${Platform.pathSeparator}config.ocio');
    file.writeAsStringSync(_config);
    return file.path;
  }

  /// Point [p] at a config — or at a path with nothing behind it — and bring
  /// the held summary into line the way a document change does.
  void useConfig(({LumitState state, LumitUiState uiState}) p, String? path) {
    p.state.project!.setColourConfig(path: path);
    p.state.notifyDocumentChanged();
  }

  group('The Viewer colour picker (§6.2)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addAdjustmentLayer();
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(700, 500),
      ));
      await tester.pump();
    }

    Future<void> openPicker(WidgetTester tester) async {
      final button = find.byKey(const ValueKey('viewer-colour'));
      await tester.ensureVisible(button);
      await tester.pump();
      await tester.tap(button);
      await tester.pumpAndSettle();
    }

    String face(WidgetTester tester) => tester
        .widget<Text>(find
            .descendant(
              of: find.byKey(const ValueKey('viewer-colour')),
              matching: find.byType(Text),
            )
            .first)
        .data!;

    /// **The day the list grows.** The picker was built as a row rather than a
    /// label "because the list grows the day a second one exists"; a loaded
    /// config is that day. One section per display, its views the rows, the
    /// built-in transform still at the top and still the way back to it.
    testWidgets('a loaded config fills the menu, and a view can be chosen',
        (tester) async {
      final p = withComp();
      useConfig((state: p.state, uiState: p.uiState), writeConfig());
      expect(p.uiState.colourSummary.loaded, isTrue,
          reason: p.uiState.colourSummary.problemEnglish);
      await mount(tester, p);

      expect(face(tester), 'Linear → sRGB',
          reason: 'nothing is chosen yet, so the built-in transform is what '
              'the picture is going through');

      await openPicker(tester);
      expect(find.text('sRGB'), findsWidgets,
          reason: "the display's own name is the section heading, verbatim");
      final view =
          find.byKey(const ValueKey('viewer-colour-view-sRGB-Standard'));
      expect(view, findsOneWidget);

      await tester.tap(view);
      await tester.pumpAndSettle();
      expect(p.uiState.colourView, ['sRGB', 'Standard'],
          reason: 'the choice is session state, held as [display, view]');
      expect(face(tester), 'Standard — sRGB',
          reason: 'the closed face names the view in force');

      // And back: the built-in row is how a view is taken off again.
      await openPicker(tester);
      await tester.tap(find.byKey(const ValueKey('viewer-colour-transform')));
      await tester.pumpAndSettle();
      expect(p.uiState.colourView, isNull);
      expect(face(tester), 'Linear → sRGB');

      await settleFrb(tester, until: () => p.uiState.previewProgress.idle);
    });
  });

  group("The export dialog's Colour section (§6.3)", () {
    Future<({LumitState state, LumitUiState uiState})> open(
      WidgetTester tester, {
      required bool withConfig,
    }) async {
      tester.view.physicalSize = const Size(1200, 1000);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Titles');
      comp.addAdjustmentLayer();
      if (withConfig) useConfig(p, writeConfig());
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-export'),
            onPressed: () => showExportDialogFrb(context: context, comp: comp),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1200, 1000),
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-export')));
      await tester.pumpAndSettle();
      return p;
    }

    Future<void> openSpaces(WidgetTester tester) async {
      final field = find.byKey(const ValueKey('export-colour-space'));
      await tester.ensureVisible(field);
      await tester.pump();
      await tester.tap(field);
      await tester.pumpAndSettle();
    }

    /// The config's output spaces sit under a heading of ours, keep their own
    /// names, and can be chosen — an OCIO export rides on top of whatever the
    /// container can state, because it is written untagged either way
    /// (docs/impl/ocio.md §5.2).
    testWidgets('a loaded config adds its own spaces, under a heading',
        (tester) async {
      await open(tester, withConfig: true);

      final managed = find.descendant(
        of: find.byKey(const ValueKey('export-ocio')),
        matching: find.byType(Text),
      );
      expect(tester.widget<Text>(managed.first).data, contains('config.ocio'),
          reason: 'the Managed by row names the file the project points at');

      await openSpaces(tester);
      expect(find.text('From the configuration'), findsOneWidget);
      expect(find.text('out_srgb'), findsOneWidget,
          reason: "the config's own word, never a key of ours");

      await tester.tap(find.text('out_srgb'));
      await tester.pumpAndSettle();

      final face = tester.widget<Text>(find
          .descendant(
            of: find.byKey(const ValueKey('export-colour-space')),
            matching: find.byType(Text),
          )
          .first);
      expect(face.data, 'out_srgb');
      // A file written through a config's transform carries no tag it could
      // honestly state, and the reading under the row says exactly that.
      expect(
          find.byKey(const ValueKey('export-colour-tagging')), findsOneWidget);
      expect(
        tester
            .widget<Text>(find.byKey(const ValueKey('export-colour-tagging')))
            .data,
        contains('untagged'),
      );
    });
  });

  group("The Project panel's colour-space submenu (§6.5)", () {
    testWidgets('a footage row takes a space from the config, and gives it up',
        (tester) async {
      final p = freshProject();
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      useConfig(p, writeConfig());

      await tester.pumpWidget(hostPanel(
        child: const ProjectPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      Future<void> openSubmenu() async {
        await tester.tapAt(
          tester.getCenter(find.descendant(
              of: find.byType(ListView), matching: find.text('shot.mov'))),
          buttons: kSecondaryButton,
        );
        await tester.pumpAndSettle();
        await tester
            .tap(find.byKey(const ValueKey('project-menu-colour-space')));
        await tester.pumpAndSettle();
      }

      await openSubmenu();
      expect(find.text('As the file says'), findsOneWidget,
          reason: 'the built-in interpretation, which every clip starts on');
      expect(find.text('srgb_texture'), findsOneWidget,
          reason: "the config's own names, verbatim");

      await tester.tap(
          find.byKey(const ValueKey('project-menu-colour-space-srgb_texture')));
      await tester.pumpAndSettle();
      expect(footage.colourSpace(), 'srgb_texture',
          reason: 'one gesture writes through to the document');

      // An ordinary op: one gesture, one undo step.
      p.state.project!.undo();
      expect(footage.colourSpace(), isNull);

      // And the row that clears it back to the built-in defaults.
      await openSubmenu();
      await tester.tap(
          find.byKey(const ValueKey('project-menu-colour-space-out_srgb')));
      await tester.pumpAndSettle();
      expect(footage.colourSpace(), 'out_srgb');

      await openSubmenu();
      await tester
          .tap(find.byKey(const ValueKey('project-menu-colour-space-none')));
      await tester.pumpAndSettle();
      expect(footage.colourSpace(), isNull);
    });
  });

  group('Project settings, the Colour group (§6.4)', () {
    Future<({LumitState state, LumitUiState uiState})> open(
      WidgetTester tester, {
      String? pick,
    }) async {
      tester.view.physicalSize = const Size(1400, 1000);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-settings'),
            onPressed: () => showProjectSettingsFrb(
              context,
              p.state.project!,
              configPicker: () async => pick,
            ),
            child: const Text('Open'),
          ),
        ),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('open-settings')));
      await tester.pumpAndSettle();
      return p;
    }

    String pathWell(WidgetTester tester) => tester
        .widget<Text>(find.descendant(
          of: find.byKey(const ValueKey('project-colour-config-path')),
          matching: find.byType(Text),
        ))
        .data!;

    /// **This is where a config is chosen** (§6.4): the project's window, not
    /// the machine's, because colour management changes what the comp looks
    /// like and so travels in the `.lum`.
    testWidgets('choosing a config loads it, and clearing gives it up',
        (tester) async {
      final path = writeConfig();
      final p = await open(tester, pick: path);

      expect(pathWell(tester), 'None');
      expect(find.textContaining('No configuration'), findsOneWidget);
      expect(
          find.descendant(
              of: find.byKey(const ValueKey('project-colour-working-space')),
              matching: find.text('Linear Rec. 709')),
          findsOneWidget,
          reason: 'the working space starts as linear Rec. 709, said outright');

      await tester
          .tap(find.byKey(const ValueKey('project-colour-config-choose')));
      await tester.pumpAndSettle();

      expect(pathWell(tester), contains('config.ocio'));
      expect(find.textContaining('Loaded:'), findsOneWidget,
          reason: 'what was read, in the calm voice');
      expect(p.state.project!.colourSummary().loaded, isTrue,
          reason: 'the control writes through to the document');

      // The working space is the project's choice once a config is loaded:
      // the second entry names the config's scene-linear space.
      await tester
          .tap(find.byKey(const ValueKey('project-colour-working-space')));
      await tester.pumpAndSettle();
      await tester.tap(find.textContaining('scene-linear space').last);
      await tester.pumpAndSettle();
      expect(p.state.project!.colourSummary().workingFromConfig, isTrue,
          reason: 'the choice reached the document');

      await tester
          .tap(find.byKey(const ValueKey('project-colour-config-clear')));
      await tester.pumpAndSettle();
      expect(pathWell(tester), 'None');
      expect(p.state.project!.colourSummary().path, '');

      // Both are ordinary edits, so undo puts the config back.
      p.state.project!.undo();
      expect(p.state.project!.colourSummary().loaded, isTrue);
    });
  });
}
