// File → Project settings: the project's anti-aliasing setting.
//
// Its value lives in the *project* rather than in this machine's settings file,
// which is what these tests are actually about: the control has to write through
// to the engine's document, not to a Dart-side copy that nothing reads. So each
// of these drives the real bridge and then asks the project what it now holds.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/project_settings_frb.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Project settings (frb)', () {
    Future<({dynamic state, dynamic uiState})> openRendering(
        WidgetTester tester) async {
      tester.view.physicalSize = const Size(1400, 1000);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);

      final p = freshProject();
      await tester.pumpWidget(hostPanel(
        child: Builder(
          builder: (context) => HouseButton(
            key: const ValueKey('open-settings'),
            onPressed: () =>
                showProjectSettingsFrb(context, p.state.project!),
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

    testWidgets('choosing a count writes it into the project', (tester) async {
      final p = await openRendering(tester);

      await tester.tap(find.byKey(const ValueKey('project-anti-aliasing')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('2 samples').last);
      await tester.pumpAndSettle();

      expect(p.state.project!.antiAliasing(), 2,
          reason: 'the control must write through to the document, '
              'not to a copy the engine never sees');
    });
  });
}
