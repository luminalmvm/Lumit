// The zero-copy Viewer transport, exercised on a REAL window.
//
// This is the test no widget test could be: it runs inside a real Windows
// runner with a real embedder, so registering a shared texture and asking
// Flutter to composite it actually happens. The failure being hunted reports
// nothing — the texture registers, and is then silently never drawn — so what
// is asserted is the runner's own count of how many times Flutter came back
// for the descriptor (`frameReady`'s return value).
//
// Run:  flutter test integration_test/shared_texture_test.dart -d windows

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/frb_generated.dart';
import 'package:lumit_flutter/state/workspace.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('the shared texture is actually composited', (tester) async {
    await BridgeLib.init();
    final state = LumitState()..newProject();
    final ui = LumitUiState(state, workspace: Workspace());

    final comp = state.project!.newComposition(name: 'Scene');
    comp.addSolidLayer();
    // A bright, asymmetric colour, so a screenshot can tell "the texture's
    // pixels reached the screen" from "Flutter composited a transparent
    // nothing" — and a channel-order mistake shows as the wrong colour rather
    // than passing unseen (orange with R and B swapped is blue).
    // The solid sits inside the auto-created Solids folder, so walk the tree.
    Iterable<ItemReference> flat(Iterable<ItemReference> items) sync* {
      for (final item in items) {
        yield item;
        if (item is ItemReference_Folder) yield* flat(item.field0.getChildren());
      }
    }

    final solid = flat(state.project!.getItems())
        .whereType<ItemReference_Solid>()
        .single
        .field0;
    final def = solid.getDefinition();
    solid.setDefinition(
        definition: BridgeSolidDef(
      name: def.name,
      colour: const BridgeColourRgba(r: 1, g: 0.5, b: 0, a: 1),
      width: def.width,
      height: def.height,
    ));
    ui.setSelectedComp(comp);

    debugPrint('DDD transport=${viewerTransport()}');

    await tester.pumpWidget(MaterialAppShim(state: state, ui: ui));
    await tester.pumpAndSettle();

    // Drive frames through the real pipeline: move the playhead, let the
    // worker render, let the runner register and announce.
    for (var f = 0; f < 30; f++) {
      ui.playheadFrame.value = f % 60;
      await tester.pump(const Duration(milliseconds: 33));
      // Real async turns so the frb stream events and channel replies land.
      await tester.runAsync(() => Future<void>.delayed(
            const Duration(milliseconds: 15),
          ));
    }
    await tester.pumpAndSettle();

    debugPrint('DDD textureId=${ui.viewerFrameid.value} '
        'available=${ui.controller.available} '
        'neverDrawn=${ui.controller.neverDrawn} '
        'announced=${ui.controller.debugAnnounced} '
        'drawn=${ui.controller.debugDrawn} '
        'textureId2=${ui.viewerFrameid.value}');

    expect(ui.controller.available, isTrue,
        reason: 'the texture path must not have latched itself off');
    expect(ui.controller.debugDrawn, greaterThan(0),
        reason: 'Flutter must actually composite the shared texture');

    // Hold the window so an outside screenshot can read the actual pixels —
    // whether the magenta reached the screen is not knowable from in here.
    debugPrint('DDD holding window for screenshot');
    for (var i = 0; i < 120; i++) {
      await tester.pump(const Duration(milliseconds: 100));
      await tester
          .runAsync(() => Future<void>.delayed(const Duration(milliseconds: 90)));
    }
  });
}

/// The minimum shell the Viewer needs, without dragging the whole app in.
class MaterialAppShim extends StatelessWidget {
  final LumitState state;
  final LumitUiState ui;
  const MaterialAppShim({super.key, required this.state, required this.ui});

  @override
  Widget build(BuildContext context) {
    return LumitAppNew(state, ui);
  }
}
