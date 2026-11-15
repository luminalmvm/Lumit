// The Audio panel against the real engine: the three sections, the Beats
// controls over the beat engine, Drive with audio's staged chain, which lands
// as real wires the engine validates, and Lower behind's duck.

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

    /// *Lower behind…* opens its popover on the comp's other sounding layers,
    /// and a pick lowers the selected layer behind it with the engine's usual
    /// settings, through the layer's own duck rather than a driver chain.
    testWidgets('Lower behind sets a duck on the picked layer',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Cut');
      p.uiState.setSelectedComp(comp);
      final tone = p.state.project!.importFootage(path: _toneWavFile());
      comp.addFootageLayer(footage: tone, asSequence: false);
      comp.addFootageLayer(footage: tone, asSequence: false);
      final layers = comp.getLayers();
      // Distinct names, so the offer can be told apart from the header,
      // which names the selection.
      layers.first.rename(name: 'Music bed');
      layers.last.rename(name: 'Voice over');
      p.uiState.selectedLayer.value = layers.first;
      p.uiState.model.refresh();
      final feed = AudioMeterFeed();
      feed.read = () => const [];
      addTearDown(feed.dispose);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        child: AudioPanelFrb(feed: feed),
      ));
      await settleFrb(tester, minRounds: 8);

      await tester.tap(find.byKey(const ValueKey('audio-duck')));
      await tester.pump();
      final offer = find.text('Voice over');
      expect(offer, findsOneWidget,
          reason: 'the other layer is what there is to lower behind');
      await tester.tap(offer);
      await tester.pump();

      final duck = layers.first.getInfo().duck;
      expect(duck?.behind, layers.last.internallayerId);
      expect(duck?.depthDb, -12, reason: "the engine's usual amount");
      expect(layers.first.getGraphDrivers(), isEmpty,
          reason: 'no driver chain is staged any more');
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
