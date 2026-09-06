// The Audio workspace opens the sound lanes (the approved
// AudioWorkspace board): applying the preset twirls the Audio group and the
// Waveform lane open on every layer that carries sound, so the Timeline shows
// the board's own picture — waves, rubber bands and lane chips — rather than
// a stack of shut rows. A layer with no sound is left alone, and pressing the
// strip's word again re-opens a lane a hand has shut.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/state/dock.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  testWidgets('the Audio preset opens the waveform lane of a sounding layer',
      (tester) async {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Cut');
    p.uiState.setSelectedComp(comp);
    final music = p.state.project!.importFootage(path: _toneWavFile());
    comp.addFootageLayer(footage: music, asSequence: false);
    final sounding = comp.getLayers().single.internallayerId;
    // A solid makes a picture and no sound: the preset must leave it shut.
    comp.addSolidLayer();
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
    // The audio probe is a real trip into FFmpeg.
    await settleFrb(tester, minRounds: 8);

    final band = find.byKey(ValueKey<String>('tl-volume-band-$sounding'));
    expect(band, findsNothing,
        reason: 'nothing is open until the preset asks for it');

    p.uiState.workspace.applyWorkspacePreset(WorkspacePreset.audio);
    await settleFrb(tester, minRounds: 8);
    expect(band, findsOneWidget,
        reason: 'the Audio arrangement is drawn with its sound lanes open');
    expect(find.text('Audio'), findsWidgets,
        reason: 'the Audio group heading is on the outline');

    // A hand shuts the lane; pressing the strip's word again re-opens it.
    await tester.tap(find.text('Waveform'));
    await tester.pump();
    expect(band, findsNothing, reason: 'the lane is still a twirl');
    p.uiState.workspace.applyWorkspacePreset(WorkspacePreset.audio);
    await settleFrb(tester, minRounds: 8);
    expect(band, findsOneWidget,
        reason: 're-applying the preset opens the lane again');

    // The Edit preset opens nothing: only Audio's board draws lanes open.
    await tester.tap(find.text('Waveform'));
    await tester.pump();
    p.uiState.workspace.applyWorkspacePreset(WorkspacePreset.edit);
    await settleFrb(tester, minRounds: 4);
    expect(band, findsNothing);
  });
}

/// A real, probeable WAV: half a second of 8 kHz mono square wave. Written
/// synchronously — an awaited async `dart:io` call in a `testWidgets` body
/// hangs the test outright.
String _toneWavFile() {
  final dir = Directory.systemTemp.createTempSync('lumit-lanes');
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
