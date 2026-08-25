// The Node preview panel: the picture at whichever box the Graph panel has
// picked (K-448, K-486).
//
// Three things are held here. Its **face**, against the approved
// Nodes-workspace drawing's panel family — the 22px strip, the subject in the
// header, the picture on the Viewer's own pasteboard rather than on a colour
// invented for it. Its **coupling**: the pick is published to the shell by the
// Graph panel, so this panel follows it without either knowing the other is
// mounted. And what it does with a box that has **no picture** — a driver makes
// a number, so the panel says so rather than sitting empty as though a render
// were on its way.
//
// The picture itself is not asserted on here: it arrives from the render
// worker, and what it contains is the engine's own proof
// (`lumit-render/tests/node_prefix_preview.rs`, which holds the claim that
// matters — the preview differs from the Viewer by exactly the effects after
// the picked box).
//
// Every document operation is genuine (see frb_test_support.dart).

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_panel.dart' show graphToolbarHeight;
import 'package:lumit_flutter/panels/node_preview_panel.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:uuid/uuid.dart';

import 'frb_test_support.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initEngineForTests);

  group('Node preview panel (frb)', () {
    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withBlur() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      p.uiState.setSelectedComp(comp);
      p.uiState.selectedLayer.value = layer;
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    BridgeNodeRef effectRef(LayerReference layer) =>
        layer.getGraph().nodes.firstWhere((n) => n.matchName == 'blur').node;

    Future<void> mount(WidgetTester tester, dynamic p) async {
      const size = Size(340, 300);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const NodePreviewPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: size,
      ));
      await tester.pump();
    }

    /// The spelling is the whole of what crosses: a picture node names itself,
    /// and a box that makes no picture names nothing, so the panel can tell the
    /// two apart without asking the engine.
    test('only the image boxes name a picture', () {
      expect(nodePreviewSpelling(const BridgeNodeRef.source()), 'source');
      expect(nodePreviewSpelling(const BridgeNodeRef.out()), 'out');
      final id = const Uuid().v7obj();
      expect(nodePreviewSpelling(BridgeNodeRef.effect(id)), id.toString());
      expect(nodePreviewSpelling(BridgeNodeRef.driver(id)), isNull);
      expect(nodePreviewSpelling(null), isNull);
    });

    /// The cap is the engine's, and the two must not drift: asking for more
    /// than the engine will send is a panel quietly showing a smaller picture
    /// than it thinks it asked for.
    test('the panel asks for no more than the engine sends', () {
      expect(nodePreviewMaxEdge, 256);
    });

    testWidgets('says so when nothing has been picked', (tester) async {
      final p = withBlur();
      await mount(tester, p);
      expect(find.byKey(const ValueKey('node-preview-header')), findsNothing);
      expect(find.text(l10n.nodePreviewNoSelection), findsOneWidget);
    });

    testWidgets('draws the picked box on the Viewer\'s own pasteboard',
        (tester) async {
      final p = withBlur();
      p.uiState.graphNode.value = effectRef(p.layer);
      await mount(tester, p);

      final theme =
          LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);

      // The strip: the drawing's 22, the panel family's own surface, and the
      // subject named in it.
      final header = find.byKey(const ValueKey('node-preview-header'));
      expect(header, findsOneWidget);
      expect(tester.getRect(header).height, graphToolbarHeight,
          reason: 'the drawing\'s 22px panel strip');
      expect(
        (tester.widget<Container>(header).color),
        theme.surface1,
        reason: 'the same band the Node and Graph panels wear',
      );
      expect(find.byKey(const ValueKey('node-preview-name')), findsOneWidget);

      // The mat: the Viewer's pasteboard token, so the two pictures sit on the
      // same ground rather than on two greys that nearly match.
      final stage = find.byKey(const ValueKey('node-preview-stage'));
      expect(stage, findsOneWidget);
      expect(tester.widget<Container>(stage).color, theme.viewerSurround);
      expect(find.text(l10n.nodePreviewNoPicture), findsNothing,
          reason: 'an effect box does make a picture');
    });

    testWidgets('a driver is named, and said to have no picture',
        (tester) async {
      final p = withBlur();
      final made = p.layer.newDriver(name: 'wiggle');
      // The id before the commit: `setGraph` takes the staged handle, and a
      // handle that has been handed over is not one to ask questions of.
      final wiggle = made.id();
      p.layer.setGraph(
        drivers: [...p.layer.getGraphDrivers(), made],
        wiring: p.layer.getGraph().wiring,
      );
      p.uiState.model.refresh();
      p.uiState.graphNode.value = BridgeNodeRef.driver(wiggle);
      await mount(tester, p);

      expect(find.byKey(const ValueKey('node-preview-name')), findsOneWidget,
          reason: 'the header still says which box is picked');
      expect(find.byKey(const ValueKey('node-preview-stage')), findsNothing);
      expect(find.text(l10n.nodePreviewNoPicture), findsOneWidget);
    });

    testWidgets('follows the pick as it moves from one box to another',
        (tester) async {
      final p = withBlur();
      p.uiState.graphNode.value = const BridgeNodeRef.source();
      await mount(tester, p);
      final source = tester
          .widget<Text>(find.byKey(const ValueKey('node-preview-name')))
          .data;

      p.uiState.graphNode.value = effectRef(p.layer);
      await tester.pump();
      final effect = tester
          .widget<Text>(find.byKey(const ValueKey('node-preview-name')))
          .data;
      expect(effect, isNot(source),
          reason: 'the panel names the box it is now showing');

      // And a pick cleared leaves the empty face rather than the last picture
      // under a name that no longer applies.
      p.uiState.graphNode.value = null;
      await tester.pump();
      expect(find.text(l10n.nodePreviewNoSelection), findsOneWidget);
    });
  });
}
