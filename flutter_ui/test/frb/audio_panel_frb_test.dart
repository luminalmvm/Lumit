// The Audio panel against the real engine: the three sections, the Beats
// controls over the beat engine, and the two graph templates — whose staged
// chains land as real wires the engine validates, Duck under's on the Layer
// out's Volume socket (K-697).

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/audio_meters_feed.dart';
import 'package:lumit_flutter/panels/audio_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
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

    /// The board's Pan row carries the Mixer's own pot beside the value well:
    /// turning it writes the same `Layer.pan` every other control writes.
    testWidgets('the pan pot turns the selected layer\'s balance',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Cut');
      p.uiState.setSelectedComp(comp);
      final music = p.state.project!.importFootage(path: _toneWavFile());
      comp.addFootageLayer(footage: music, asSequence: false);
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
      // The audio probe is a real trip into FFmpeg; the sound rows appear
      // once it answers.
      await settleFrb(tester, minRounds: 8);

      final pot = find.byKey(const ValueKey('audio-pan-pot'));
      expect(pot, findsOneWidget, reason: 'the dial is drawn beside the well');

      // A mouse, as on the desk this app ships for: touch slop swallows a
      // short test drag. Several moves, because the first only wins the
      // recogniser the arena and a real drag is a stream of them anyway.
      final gesture = await tester.startGesture(tester.getCenter(pot),
          kind: PointerDeviceKind.mouse);
      for (var i = 0; i < 3; i++) {
        await gesture.moveBy(const Offset(0, -10));
        await tester.pump();
      }
      await gesture.up();
      await tester.pump();

      final pan = p.uiState.model.heldLayers.first.info.pan;
      expect(pan, isA<BridgeScalar_Static>());
      expect((pan as BridgeScalar_Static).field0, greaterThan(30),
          reason: 'a real travel up is a real turn to the right, committed '
              'once on release');
      // The pot also answers a double-click (recentre), so its recogniser
      // holds the double-tap window open after the release; let it lapse
      // rather than ending the test with its timer pending.
      await tester.pump(kDoubleTapTimeout);
    });
  });
}

/// A real, probeable WAV: half a second of 8 kHz mono square wave, so the
/// probe says the layer can make a sound and the sound rows are drawn.
/// Written synchronously — an awaited async `dart:io` call in a `testWidgets`
/// body hangs the test outright.
String _toneWavFile() {
  final dir = Directory.systemTemp.createTempSync('lumit-panel-pan');
  final file = File('${dir.path}/tone.wav');
  const rate = 8000;
  const samples = 4000;
  const dataBytes = samples * 2;
  final out = BytesBuilder();
  void ascii(String s) => out.add(s.codeUnits);
  void u16(int v) => out.add([v & 0xff, (v >> 8) & 0xff]);
  void u32(int v) =>
      out.add([v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff, (v >> 24) & 0xff]);
  ascii('RIFF');
  u32(36 + dataBytes);
  ascii('WAVE');
  ascii('fmt ');
  u32(16);
  u16(1);
  u16(1);
  u32(rate);
  u32(rate * 2);
  u16(2);
  u16(16);
  ascii('data');
  u32(dataBytes);
  final data = Uint8List(dataBytes);
  for (var i = 0; i < samples; i++) {
    final v = (i ~/ 9).isEven ? 12000 : -12000;
    data[i * 2] = v & 0xff;
    data[i * 2 + 1] = (v >> 8) & 0xff;
  }
  out.add(data);
  file.writeAsBytesSync(out.toBytes());
  return file.path;
}
