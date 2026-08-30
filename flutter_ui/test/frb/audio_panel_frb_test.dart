// The Audio panel against the real engine: the three sections, the Beats
// controls over the beat engine, and the two graph templates — whose staged
// chains land as real wires the engine validates, Duck under's on the Layer
// out's Volume socket (K-697).

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/audio_meters_feed.dart';
import 'package:lumit_flutter/panels/audio_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  Future<({dynamic ui, dynamic comp})> mount(WidgetTester tester,
      {int solids = 1, bool withBlur = false}) async {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Cut');
    for (var i = 0; i < solids; i++) {
      final solid = comp.addSolidLayer();
      if (withBlur && i == 0) solid.addEffect(name: 'blur');
    }
    p.uiState.setSelectedComp(comp);
    p.uiState.selectedLayer.value = comp.getLayers().first;
    p.uiState.model.refresh();
    final feed = AudioMeterFeed();
    feed.read = () => const [];
    addTearDown(feed.dispose);
    await tester.pumpWidget(hostPanel(
      state: p.state,
      uiState: p.uiState,
      child: AudioPanelFrb(feed: feed),
    ));
    await settleFrb(tester, minRounds: 4);
    return (ui: p.uiState, comp: comp);
  }

  group('Audio panel (frb)', () {
    /// The board's three sections stand, with the Beats controls at their
    /// manifest defaults: the comp mix, the whole comp, 120 ms spacing.
    testWidgets('the three sections stand as drawn', (tester) async {
      await mount(tester);
      expect(find.text('Levels'), findsOneWidget);
      expect(find.text('Beats'), findsOneWidget);
      expect(find.text('Selected layer'), findsOneWidget);
      expect(find.byKey(const ValueKey('levels-bars')), findsOneWidget);
      expect(find.byKey(const ValueKey('audio-clip-lamp')), findsOneWidget);
      expect(find.byKey(const ValueKey('beats-source')), findsOneWidget);
      expect(find.byKey(const ValueKey('beats-sensitivity')), findsOneWidget);
      expect(find.byKey(const ValueKey('beats-range')), findsOneWidget);
      expect(find.byKey(const ValueKey('beats-spacing')), findsOneWidget);
      expect(find.byKey(const ValueKey('beats-tap')), findsOneWidget);
      expect(find.byKey(const ValueKey('beats-generate')), findsOneWidget);
      expect(find.byKey(const ValueKey('beats-clear')), findsOneWidget);
      // A solid is selected: the template buttons stand, the sound rows do
      // not — a silent layer says so instead (K-435).
      expect(find.byKey(const ValueKey('audio-drive')), findsOneWidget);
      expect(find.byKey(const ValueKey('audio-duck')), findsOneWidget);
      expect(find.text('This layer makes no sound.'), findsOneWidget);
      expect(find.byKey(const ValueKey('audio-volume-db')), findsNothing);
    });

    /// Tap tempo: pressing Tap in rhythm fills the BPM well and arms the
    /// grid caption. (In a test the taps land milliseconds apart, so the
    /// tempo clamps to the well's ceiling — the point is the route, not the
    /// number.)
    testWidgets('tap tempo arms the grid', (tester) async {
      await mount(tester);
      expect(find.text('Grid on'), findsNothing);
      await tester.tap(find.byKey(const ValueKey('beats-tap')));
      await tester.pump(const Duration(milliseconds: 40));
      await tester.tap(find.byKey(const ValueKey('beats-tap')));
      await tester.pump();
      expect(find.text('Grid on'), findsOneWidget,
          reason: 'two taps are a tempo, and a tapped tempo is an override');
    });

    /// Generate on a silent comp is the calm nothing (docs/09 §5: no audio is
    /// an answer, not an alarm), and Clear survives a comp with no beat
    /// markers at all.
    testWidgets('generate and clear survive a silent comp', (tester) async {
      await mount(tester);
      await tester.tap(find.byKey(const ValueKey('beats-generate')));
      await tester.pump();
      await settleFrb(tester, minRounds: 6);
      await tester.pump(const Duration(seconds: 1));
      await tester.tap(find.byKey(const ValueKey('beats-clear')));
      await tester.pump();
      // Nothing thrown, nothing shown: the calm path.
    });

    /// *Drive with audio…* offers the selected layer's free Number parameters
    /// and stages Audio level → Remap → Smooth onto the picked one — three
    /// boxes and three wires in one commit.
    testWidgets('Drive with audio stages the chain onto a picked parameter',
        (tester) async {
      final p = await mount(tester, withBlur: true);
      final layer = p.comp.getLayers().first;
      expect(layer.getGraphDrivers(), isEmpty);

      await tester.tap(find.byKey(const ValueKey('audio-drive')));
      await tester.pump();
      // The blur's Radius is a free Number socket; pick it.
      final offer = find.text('Gaussian blur › Radius');
      expect(offer, findsWidgets,
          reason: 'the effect\'s parameters are the menu');
      await tester.tap(offer.first);
      await tester.pump();

      final drivers = layer.getGraphDrivers();
      expect(drivers.length, 3,
          reason: 'Audio level, Remap and Smooth were staged');
      final graph = layer.getGraph();
      expect(graph.wiring.edges.length, 3,
          reason: 'level → remap → smooth → the parameter');
      final target = graph.wiring.edges.last.to;
      expect(target, isA<BridgeInputRef_Param>());
      expect((target as BridgeInputRef_Param).node,
          isA<BridgeNodeRef_Effect>(),
          reason: 'the chain ends on the effect\'s own socket');
    });

    /// *Duck under…* stages the inverted chain onto the selected layer's own
    /// Volume socket (K-697): Audio level listening to the picked layer,
    /// Remap upside down, Smooth, into the Layer out — and the engine
    /// accepts the wire, which is the whole road being proved.
    testWidgets('Duck under stages the inverted chain onto the Volume socket',
        (tester) async {
      final p = await mount(tester, solids: 2);
      final layer = p.comp.getLayers().first;
      // Distinct names, so the menu's offers can be told apart — and told
      // apart from the header, which names the selection.
      layer.rename(name: 'Music bed');
      p.comp.getLayers().last.rename(name: 'Voice over');
      p.ui.model.refresh();
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('audio-duck')));
      await tester.pump();
      // The other layer is the menu; the selected one is only in the header.
      expect(find.text('Music bed'), findsOneWidget,
          reason: 'the selection is named once, in the header — a layer '
              'cannot duck under itself');
      final offer = find.text('Voice over');
      expect(offer, findsOneWidget,
          reason: 'the other layer is what there is to duck under');
      await tester.tap(offer);
      await tester.pump();

      final graph = layer.getGraph();
      expect(layer.getGraphDrivers().length, 3);
      final toVolume = graph.wiring.edges.where((e) =>
          e.to is BridgeInputRef_Param &&
          (e.to as BridgeInputRef_Param).node is BridgeNodeRef_Out &&
          (e.to as BridgeInputRef_Param).port == 'volume');
      expect(toVolume.length, 1,
          reason: 'the chain lands on the Layer out\'s Volume socket');
      // The Out box now reports its Volume socket wired, so the Graph panel
      // draws the wire filled.
      final out = graph.nodes
          .firstWhere((n) => n.node is BridgeNodeRef_Out);
      final volumePort =
          out.inputs.firstWhere((portInfo) => portInfo.id == 'volume');
      expect(volumePort.wired, isTrue);
    });
  });
}
