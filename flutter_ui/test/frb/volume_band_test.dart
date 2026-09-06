// The volume rubber band (the AudioWorkspace board): the Volume curve
// drawn on the waveform lane, grabbable near its own line — a drag writes
// volume keyframes through the same `setVolumeDb` every other control uses.
//
// Against the real engine, like every frb panel test: a band that does not
// reach the document is a picture of a control.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/volume_band_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  testWidgets('the rubber band writes volume keyframes and drags their level',
      (tester) async {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Cut');
    p.uiState.setSelectedComp(comp);
    final music = p.state.project!.importFootage(path: _toneWavFile());
    comp.addFootageLayer(footage: music, asSequence: false);
    p.uiState.model.refresh();

    tester.view.physicalSize = const Size(1280, 600);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(hostPanel(
      child: const TimelinePanelFrb(),
      state: p.state,
      uiState: p.uiState,
      size: const Size(1280, 600),
    ));
    await tester.pump();
    // The audio probe is a real trip into FFmpeg; the Audio group appears
    // once it answers.
    await settleFrb(tester, minRounds: 8);

    final layer = comp.getLayers().first;
    final id = layer.internallayerId;

    // Open Audio, then the Waveform lane the band rides on.
    await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
    await tester.pump();
    await tester.tap(find.text('Audio'));
    await tester.pump();
    await tester.tap(find.text('Waveform'));
    await tester.pump();

    final band = find.byKey(ValueKey<String>('tl-volume-band-$id'));
    expect(band, findsOneWidget, reason: 'the band rides the waveform lane');

    final rect = tester.getRect(band);
    const span = volumeBandTopDb - volumeBandFloorDb;
    // Where a level draws inside the band, mirroring the widget's mapping.
    double yOf(double db) =>
        rect.top + 1 + (volumeBandTopDb - db) / span * (rect.height - 2);

    BridgeScalar volume() => p.uiState.model.heldLayers
        .firstWhere((e) => e.layer.internallayerId == id)
        .info
        .volumeDb;

    // --- Ctrl-click plants keys on the line, at the level it already reads.
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.tapAt(Offset(rect.left + rect.width * 0.3, yOf(0)));
    await tester.pump();
    expect((volume() as BridgeScalar_Keyframed).field0, hasLength(1),
        reason: 'the first click planted');
    await tester.pump();
    await tester.tapAt(Offset(rect.left + rect.width * 0.6, yOf(0)));
    await tester.pump();
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    var keys = (volume() as BridgeScalar_Keyframed).field0;
    expect(keys, hasLength(2), reason: 'two planted keys');
    expect(keys[0].value, closeTo(0, 0.5),
        reason: 'a planted key takes the level the line already read');

    // --- A vertical drag on the second key pulls its level down.
    final gesture = await tester.startGesture(
      Offset(rect.left + rect.width * 0.6, yOf(keys[1].value)),
      kind: PointerDeviceKind.mouse,
    );
    await tester.pump(const Duration(milliseconds: 60));
    for (var i = 0; i < 12; i++) {
      await gesture.moveBy(const Offset(0, 4));
      await tester.pump();
    }
    await gesture.up();
    await tester.pump();
    keys = (volume() as BridgeScalar_Keyframed).field0;
    expect(keys[1].value, lessThan(-10),
        reason: 'a real travel is a real drop at the band scale');
    expect(keys[0].value, closeTo(0, 0.5), reason: 'the other key held still');

    // --- Alt-click lifts a key; lifting the last leaves the level static at
    // what the key read.
    await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
    await tester.tapAt(Offset(rect.left + rect.width * 0.3, yOf(keys[0].value)));
    await tester.pump();
    await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
    keys = (volume() as BridgeScalar_Keyframed).field0;
    expect(keys, hasLength(1), reason: 'the clicked key went');
  });

  testWidgets('a static level drags whole, without minting keys',
      (tester) async {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Cut');
    p.uiState.setSelectedComp(comp);
    final music = p.state.project!.importFootage(path: _toneWavFile());
    comp.addFootageLayer(footage: music, asSequence: false);
    p.uiState.model.refresh();

    tester.view.physicalSize = const Size(1280, 600);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(hostPanel(
      child: const TimelinePanelFrb(),
      state: p.state,
      uiState: p.uiState,
      size: const Size(1280, 600),
    ));
    await tester.pump();
    await settleFrb(tester, minRounds: 8);

    final layer = comp.getLayers().first;
    final id = layer.internallayerId;
    await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$id')));
    await tester.pump();
    await tester.tap(find.text('Audio'));
    await tester.pump();
    await tester.tap(find.text('Waveform'));
    await tester.pump();

    final rect =
        tester.getRect(find.byKey(ValueKey<String>('tl-volume-band-$id')));
    const span = volumeBandTopDb - volumeBandFloorDb;
    final yOfZero =
        rect.top + 1 + volumeBandTopDb / span * (rect.height - 2);

    final gesture = await tester.startGesture(
      Offset(rect.left + rect.width * 0.4, yOfZero),
      kind: PointerDeviceKind.mouse,
    );
    await tester.pump(const Duration(milliseconds: 60));
    for (var i = 0; i < 12; i++) {
      await gesture.moveBy(const Offset(0, 4));
      await tester.pump();
    }
    await gesture.up();
    await tester.pump();

    final volume = p.uiState.model.heldLayers
        .firstWhere((e) => e.layer.internallayerId == id)
        .info
        .volumeDb;
    expect(volume, isA<BridgeScalar_Static>(),
        reason: 'dragging a flat line moves the level, not a key nobody made');
    expect((volume as BridgeScalar_Static).field0, lessThan(-5));
  });
}

/// A real, probeable WAV: half a second of 8 kHz mono square wave. Written
/// synchronously — an awaited async `dart:io` call in a `testWidgets` body
/// hangs the test outright.
String _toneWavFile() {
  final dir = Directory.systemTemp.createTempSync('lumit-band');
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
