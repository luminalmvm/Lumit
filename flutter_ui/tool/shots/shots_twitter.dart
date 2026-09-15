// Eight 3840 by 2160 pictures for the announcement (the 1920 by 1080
// layout rendered at twice the pixel density): a real project's edit under
// the three styles, and its Audio workspace mixing.
//
// Not manual shots: they go to `C:/tmp/lumit-shots/twitter` and never near
// `web-docs`. The sweep refuses to run without `LUMIT_SHOTS_OUT` set, and
// needs `LUMIT_SHOTS_PROJECT` naming a .lum whose media is on this machine;
// `LUMIT_SHOTS_COMP` names the composition to front (the first otherwise).
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1
//   $env:LUMIT_SHOTS_OUT='C:/tmp/lumit-shots/twitter'
//   $env:LUMIT_SHOTS_PROJECT='C:/.../Something.lum'
//   flutter run -d windows -t tool/shots/shots_twitter.dart

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/theme/theme.dart';

import 'shots_common.dart';

Future<void> main() async {
  final (state, ui) = await bootLumit();
  if (Platform.environment['LUMIT_SHOTS_OUT'] == null) {
    // ignore: avoid_print
    print('SKIPPED: set LUMIT_SHOTS_OUT, these are review artefacts, and the '
        'default destination is the manual.');
    exit(0);
  }
  final path = Platform.environment['LUMIT_SHOTS_PROJECT'];
  if (path == null || !File(path).existsSync()) {
    // ignore: avoid_print
    print('SKIPPED: set LUMIT_SHOTS_PROJECT to a .lum with its media on '
        'this machine.');
    exit(1);
  }
  Directory(shotsOut).createSync(recursive: true);

  ui.workspace.applyWorkspacePreset(WorkspacePreset.edit);
  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));
  await pause(2);
  // A 1920 by 1080 client area: the frame takes 16 across and 39 down.
  await sizeWindow(1936, 1119);
  await pause(2);

  await state.openProject(path);
  for (var i = 0; i < 120 && state.comps().isEmpty; i++) {
    await pause(0.5);
  }
  final comps = state.comps();
  if (comps.isEmpty) {
    // ignore: avoid_print
    print('the project opened with no compositions');
    await pause(2);
    exit(1);
  }
  // ignore: avoid_print
  print('opened: ${comps.length} comps, fronted ${ui.selectedComp?.internalid}');
  // The project's own saved state is the picture: the comp it was left on,
  // its frame, its selection and where the timeline was scrolled. Only a
  // named comp overrides that, and then a frame three quarters in.
  final wanted = Platform.environment['LUMIT_SHOTS_COMP']?.toLowerCase();
  var target = comps.first;
  if (wanted != null) {
    for (final candidate in comps) {
      if (candidate.$2.toLowerCase() == wanted) target = candidate;
    }
    ui.setSelectedComp(target.$1);
  }
  await pause(6);
  final comp = ui.selectedComp ?? target.$1;
  final fps = ui.model.fps;
  final frames = ui.model.durationFrames;
  var frame = ui.playheadFrame.value;
  if (wanted != null) {
    frame = (frames * 0.75).round();
    ui.scrubTo(frame);
    BridgeLayerEntry? withEffects;
    for (final entry in ui.model.heldLayers) {
      if (entry.layer.getEffects().isNotEmpty) {
        withEffects = entry;
        break;
      }
    }
    final chosen = withEffects ?? ui.model.heldLayers.first;
    ui.setSelection([chosen.layer]);
  }
  // The saved zoom came from a wider monitor, so at this width the lanes'
  // window can fall short of the playhead: centre it, keeping the zoom, and
  // front the comp again so the Timeline reads the view afresh.
  final id = comp.internalid.toString();
  final view = ui.compViews[id];
  final kept = List.of(ui.selectedLayers.value);
  if (view != null && frames > 0) {
    final visible = 1 / view.zoom;
    final start = frame / frames - visible / 2;
    final scroll = (start / (1 - visible)).clamp(0.0, 1.0);
    // Another comp that is already open, so no new tab appears; the view is
    // written while it is fronted, after the Timeline has saved its own,
    // and read back when this comp returns. The selection is kept by hand,
    // since fronting another comp lets it go.
    final others =
        ui.openComps.where((c) => c != comp.internalid).toList();
    if (others.isNotEmpty) {
      final other =
          comps.firstWhere((c) => c.$1.internalid == others.first);
      ui.setSelectedComp(other.$1);
      await pause(2);
      ui.compViews[id] = (frame: view.frame, zoom: view.zoom, scroll: scroll);
      ui.setSelectedComp(comp);
      await pause(3);
      ui.setSelection(kept);
      await pause(1);
    }
  }
  movePanel(ui.workspace.dock, Panel.effectControls.pane(),
      Panel.effectsAndPresets.pane(), DropPosition.stack);
  ui.workspace.touch();
  ui.frontPanel(Panel.effectControls);
  // The picture at this frame is rendered from real footage; give it time.
  await pause(14);

  Future<void> look(ThemeShape shape, LumitColorScheme scheme,
      {ToolBarPosition toolBar = ToolBarPosition.auto,
      ViewerBars bars = ViewerBars.auto,
      LanternRoom room = LanternRoom.auto}) async {
    ui.workspace.setShape(shape);
    ui.workspace.setScheme(scheme);
    ui.workspace.interface
      ..room = room
      ..toolBarPosition = toolBar
      ..viewerBars = bars;
    ui.workspace.recompose();
    ui.workspace.settingsChanged();
    await pause(3);
  }

  // ---- The edit, four ways ------------------------------------------------
  // Eight frames, eight schemes: a wide mix rather than the stock dark.
  await look(ThemeShape.studio, LumitColorScheme.mallowDark);
  await captureUi('1-studio-edit.png', scale: 2);
  await look(ThemeShape.desk, LumitColorScheme.vellumLight);
  await captureUi('2-desk-edit.png', scale: 2);
  await look(ThemeShape.lantern, LumitColorScheme.neonDark);
  await captureUi('3-lantern-edit.png', scale: 2);
  await look(ThemeShape.lantern, LumitColorScheme.canopyLight,
      toolBar: ToolBarPosition.left, bars: ViewerBars.deck);
  await captureUi('4-lantern-edit-rail.png', scale: 2);

  // ---- The mix, four ways -------------------------------------------------
  // The cut's own sound sits inside its 4K AVI clips, which draw no waveform,
  // so the mix rides on music laid into the comp instead: one row at -6 dB and
  // one track of two clips with a crossfade, the shape the manual's audio
  // timeline pictures are made with. The viewer keeps the cut's own picture,
  // on the cine shot that runs into the train POV.
  const mixFrame = 346;
  // The cut's top Adjustment lifts the whole comp two stops with a curve on
  // top, which suits the dark corridor the edit pictures sit on and blows this
  // brighter shot out, so both come off for the mix.
  for (final entry in ui.model.heldLayers) {
    if (entry.layer.getName() != 'Adjustment') continue;
    for (final fx in entry.layer.getEffects()) {
      final name = fx.getInfo().name;
      if (name == 'exposure' || name == 'curves') {
        entry.layer.setEffectEnabled(effect: fx, enabled: false);
      }
    }
  }
  BridgeRational secs(int s) => BridgeRational(num: s, den: 1);
  final music = state.project!.importFootage(path: '$fixtures/Music.wav');
  comp.addFootageLayer(footage: music, asSequence: false);
  comp.addFootageLayer(footage: music, asSequence: false);
  ui.model.refresh();
  await pause(2);
  final rows = comp.getLayers();
  final plain = rows[0];
  final track = rows[1];
  // The plain row runs under the playhead, turned down.
  plain.setSpan(
      span: BridgeSpan(
          inPoint: secs(6), outPoint: secs(16), startOffset: secs(0)));
  plain.setVolumeDb(value: const BridgeScalar.static_(-6));
  // The track: a clip trimmed to eight seconds with a fade at each end, and a
  // second laid two seconds into its tail so the overlap is a crossfade.
  track.setSpan(
      span: BridgeSpan(
          inPoint: secs(2), outPoint: secs(18), startOffset: secs(0)));
  track.convertToSequenced();
  var clips = track.getClips();
  track.trimClip(
      clip: clips.single.id,
      startFrame: clips.single.startFrame,
      endFrame: clips.single.startFrame + (fps * 8).round());
  clips = track.getClips();
  track.setClipFade(
    clip: clips.single.id,
    fadeIn:
        const BridgeClipFade(seconds: 1.5, shape: BridgeClipFadeShape.smooth()),
    fadeOut:
        const BridgeClipFade(seconds: 1, shape: BridgeClipFadeShape.fast()),
  );
  track.addClip(
      footage: music,
      atFrame: clips.single.endFrame - (fps * 2).round(),
      overlap: true);
  clips = track.getClips();
  track.setClipFade(
    clip: clips.last.id,
    fadeOut: const BridgeClipFade(seconds: 2, shape: BridgeClipFadeShape.slow()),
  );
  ui.model.refresh();
  ui.workspace.applyWorkspacePreset(WorkspacePreset.audio);
  comp.audioPrepare();
  ui.scrubTo(mixFrame);
  // The peaks are read off the music in the background.
  await pause(20);
  ui.setSelection([plain]);
  await pause(1);

  Future<void> mixing(String name) async {
    // Picking a layer fronts its controls, so the desk is fronted again here:
    // the Audio workspace's left column is the mixer.
    ui.frontPanel(Panel.mixer);
    await pause(0.5);
    // Play up to the frame the pictures are taken at, stop there, and capture
    // at once: the meters' peak holds are still lit and the frame is the same.
    ui.scrubTo(mixFrame - (fps * 2).round());
    await pause(1);
    await tapKey('viewer-play', settle: 0.2);
    for (var i = 0; i < 100 && ui.playheadFrame.value < mixFrame; i++) {
      await pause(0.05);
    }
    await tapKey('viewer-play', settle: 0.1);
    ui.scrubTo(mixFrame);
    await pause(0.4);
    await captureUi(name, scale: 2);
    await pause(1);
  }

  await look(ThemeShape.studio, LumitColorScheme.nocturneDark);
  await mixing('5-studio-audio.png');
  await look(ThemeShape.desk, LumitColorScheme.hearthDark);
  await mixing('6-desk-audio.png');
  await look(ThemeShape.lantern, LumitColorScheme.tavernDark);
  await mixing('7-lantern-audio.png');
  await look(ThemeShape.lantern, LumitColorScheme.arcaneDark,
      room: LanternRoom.night, toolBar: ToolBarPosition.left);
  await mixing('8-lantern-audio-night.png');

  // ignore: avoid_print
  print('shot ${target.$2} at frame $frame of $frames at $fps fps');
  // ignore: avoid_print
  print('comp on screen: ${ui.selectedComp?.internalid}');
  exit(0);
}
