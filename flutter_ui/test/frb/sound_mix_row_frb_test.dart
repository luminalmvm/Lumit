// The Sound mix row in the layer Timeline (docs/impl/audio-timeline.md §5,
// docs/07 §4.2, plan 13), against the real document.
//
// The row is the comp's mark and nothing else: no row until the comp has been
// mixed, and from then on the Audio layers stand behind it with the twirl
// bringing them back for a look. Its two gestures are here too - a double
// click opens the Audio workspace, and the menu's Convert to precomp packs the
// layers into a nested comp and takes the row away for good.

import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/widgets/controls.dart'
    show closeLumitPopups, lumitPopupOpen;

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  /// A comp with a picture layer and a layer that is nothing but sound, which
  /// is the only kind the fold takes.
  Future<
      ({
        LumitState state,
        LumitUiState ui,
        CompositionReference comp,
        String music
      })> mount(WidgetTester tester) async {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Cut');
    p.uiState.setSelectedComp(comp);
    comp.addSolidLayer();
    final wav = p.state.project!.importFootage(path: _toneWavFile());
    comp.addFootageLayer(footage: wav, asSequence: false);
    final music = [
      for (final layer in comp.getLayers())
        if (layer.getKind() == BridgeLayerKind.audio)
          layer.internallayerId.toString(),
    ].single;
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
    return (state: p.state, ui: p.uiState, comp: comp, music: music);
  }

  final row = find.byKey(const ValueKey('tl-sound-mix-row'));

  /// Mark the comp mixed the way the Audio timeline does, and let the table
  /// read the mark back.
  Future<void> mix(
      WidgetTester tester,
      ({
        LumitState state,
        LumitUiState ui,
        CompositionReference comp,
        String music
      }) p) async {
    p.comp.setSoundMix(mixed: true);
    p.ui.model.refresh();
    await settleFrb(tester, minRounds: 2);
  }

  testWidgets('the row stands on the mark, and the twirl is a peek behind it',
      (tester) async {
    final p = await mount(tester);
    final music = find.byKey(ValueKey<String>('tl-rowbody-${p.music}'));

    expect(row, findsNothing,
        reason: 'a comp that has never been mixed has no row');
    expect(music, findsOneWidget, reason: 'and keeps its Audio layer in stack');

    await mix(tester, p);

    expect(row, findsOneWidget, reason: 'the mark is what puts the row up');
    expect(music, findsNothing,
        reason: 'and a mixed comp keeps its Audio layers behind it');
    expect(
        tester
            .widget<Text>(find.byKey(const ValueKey('tl-sound-mix-count')))
            .data,
        l10n.timelineSoundMixFolded(1),
        reason: 'the row says how many it has taken');

    // The row's double click stops short of the twirl, so the tap lands on
    // the frame after it rather than a double-tap timeout later.
    await tester.tap(find.byKey(const ValueKey('tl-sound-mix-twirl')));
    await tester.pump();
    expect(music, findsOneWidget,
        reason: 'the twirl brings them back for a look');
  });

  testWidgets('a double click on the row opens the Audio workspace',
      (tester) async {
    final p = await mount(tester);
    await mix(tester, p);
    expect(p.ui.workspace.activePreset, isNot(WorkspacePreset.audio));

    // The recogniser wants the first tap's own countdown spent before the
    // second lands, and the tracker put down after it.
    final at = tester.getCenter(
        find.descendant(of: row, matching: find.text(l10n.timelineSoundMix)));
    await tester.tapAt(at);
    await tester.pump(kDoubleTapMinTime);
    await tester.tapAt(at);
    await tester.pump(kDoubleTapTimeout);
    await settleFrb(tester, minRounds: 4);

    expect(p.ui.workspace.activePreset, WorkspacePreset.audio,
        reason: 'the row is the way back to the panel that made the mix');
  });

  testWidgets('the row menu converts the mix to a precomp', (tester) async {
    final p = await mount(tester);
    await mix(tester, p);

    final gesture = await tester.startGesture(tester.getCenter(row),
        kind: PointerDeviceKind.mouse, buttons: kSecondaryMouseButton);
    await gesture.up();
    await tester.pumpAndSettle();

    expect(lumitPopupOpen, isTrue);
    expect(find.byKey(const ValueKey('tl-sound-mix-open')), findsOneWidget,
        reason: 'the menu offers the workspace as well');

    await tester.tap(find.byKey(const ValueKey('tl-sound-mix-precompose')));
    await settleFrb(tester, minRounds: 4);

    // One row stands where the Audio layers stood, and it holds sound and
    // nothing else, so it reads as an Audio row as any sound-only layer does.
    // What says it is the pack is the comp underneath it.
    final layers = p.comp.getLayers();
    expect(layers, hasLength(2), reason: 'the solid, and the mix beside it');
    expect(p.comp.soundMix(), isFalse, reason: 'the parent is not mixed now');
    expect(row, findsNothing, reason: 'so the row goes with them');

    final nested = layers
        .firstWhere((layer) => layer.getKind() == BridgeLayerKind.audio)
        .getSourceItem();
    expect(nested, isA<ItemReference_Composition>(),
        reason: 'the layer left behind stands on the mix comp, not on media');

    // Two levels down: the mix comp holds one row per track, standing on a
    // comp of the track's own, and that comp holds one layer per clip. The row
    // still reads as an Audio one, because that is what any sound-only layer
    // reads as whatever kind it is.
    final rows = (nested! as ItemReference_Composition).field0.getLayers();
    final trackComp = rows.single.getSourceItem();
    expect(trackComp, isA<ItemReference_Composition>(),
        reason: 'a track stands on a comp of its own');
    final clipRows =
        (trackComp! as ItemReference_Composition).field0.getLayers();
    expect(clipRows.single.getKind(), BridgeLayerKind.audio,
        reason: 'and it holds the Audio layer it was, as one clip');
    expect(clipRows.single.getClips(), hasLength(1));
    closeLumitPopups();
    await tester.pumpAndSettle();
  });

  testWidgets('the row belongs to the comp, not to the panel', (tester) async {
    final p = await mount(tester);
    await mix(tester, p);
    expect(row, findsOneWidget);

    // A second comp with a layer of nothing but sound, which has never been
    // near the Audio timeline.
    final other = p.state.project!.newComposition(name: 'Trailer');
    final wav = p.state.project!.importFootage(path: _toneWavFile());
    other.addFootageLayer(footage: wav, asSequence: false);
    final music = [
      for (final layer in other.getLayers())
        if (layer.getKind() == BridgeLayerKind.audio)
          layer.internallayerId.toString(),
    ].single;
    p.ui.setSelectedComp(other);
    p.ui.model.refresh();
    await settleFrb(tester, minRounds: 8);

    expect(row, findsNothing, reason: 'the mark was the first comp\'s');
    expect(find.byKey(ValueKey<String>('tl-rowbody-$music')), findsOneWidget,
        reason: 'so this one keeps its Audio layer in the stack');
  });

  testWidgets('the graph pane has no row, and keeps the Audio layers',
      (tester) async {
    final p = await mount(tester);
    await mix(tester, p);
    final music = find.byKey(ValueKey<String>('tl-rowbody-${p.music}'));
    expect(row, findsOneWidget);
    expect(music, findsNothing);

    await tester.tap(find.byKey(const ValueKey('tl-graph')));
    await settleFrb(tester, minRounds: 2);

    expect(row, findsNothing,
        reason: 'the graph half has no foot to pin the row to');
    expect(music, findsOneWidget,
        reason: 'and with no twirl to bring them back they stay in the stack');
  });

  testWidgets('a drag of the master fader is one undo step', (tester) async {
    final p = await mount(tester);
    await mix(tester, p);

    final before = p.state.project!.appliedSteps();
    final drag =
        await tester.startGesture(tester.getCenter(find.byKey(const ValueKey(
      'tl-sound-mix-db',
    ))));
    for (var i = 0; i < 5; i++) {
      await drag.moveBy(const Offset(8, 0));
      await tester.pump();
    }
    await drag.up();
    await settleFrb(tester, minRounds: 2);

    expect(p.state.project!.appliedSteps(), before + 1,
        reason: 'the well writes on release, not on every tick of travel');
  });
}

/// A real, probeable WAV: half a second of 8 kHz mono square wave. Written
/// synchronously - an awaited async `dart:io` call in a `testWidgets` body
/// hangs the test outright.
String _toneWavFile() {
  final dir = Directory.systemTemp.createTempSync('lumit-sound-mix');
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
