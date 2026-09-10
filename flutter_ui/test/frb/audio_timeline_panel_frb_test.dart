// The Audio timeline panel over a real document (docs/impl/audio-timeline.md
// §6, plans 10 and 11): the tracks it lists, the faded picture row with its
// Detach audio button, the twirl opening on to Volume, the clips on a converted
// track with their trims, slides, cross-track moves, drops and razor, and the
// budget gates every timeline-shaped panel has to meet - idle costs nothing, a
// scrub moves the playhead's own layer and leaves the lanes alone, and a drag
// writes once, on release.

import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/audio_timeline_fades_frb.dart'
    show ClipFadePainter;
import 'package:lumit_flutter/panels/audio_timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/audio_timeline_rows_frb.dart'
    show audioTrackMaxRows, audioTrackMinRows;
import 'package:lumit_flutter/panels/timeline_extras_frb.dart'
    show clipFillAlpha, clipFillSelectedAlpha, workAreaFrames, workAreaWith;
import 'package:lumit_flutter/panels/timeline_panel_frb.dart'
    show TimelinePanelFrb;
import 'package:lumit_flutter/state/dock.dart' show Panel, PanelPane;
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/drag_payloads.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/widgets/controls.dart'
    show SubmenuRow, closeLumitPopups, lumitPopupOpen;

import 'frb_test_support.dart';

/// Counts widget rebuilds by name, from the framework's own log - the rebuild
/// budget file's own counter, kept here so this panel's gates do not depend on
/// the Timeline's fixture.
class _Rebuilds {
  final Map<String, int> byName = {};
  bool counting = false;
  DebugPrintCallback? _previous;

  void install() {
    _previous = debugPrint;
    debugPrint = (String? message, {int? wrapWidth}) {
      if (!counting || message == null) return;
      var line = message;
      final tail = line.lastIndexOf('): ');
      if (tail >= 0) line = line.substring(tail + 3);
      line = line.replaceFirst(RegExp(r'^(Building|Rebuilding)\s+'), '');
      final name = line.trim().split(RegExp(r'[\s(<{-]')).first;
      byName[name] = (byName[name] ?? 0) + 1;
    };
    debugPrintRebuildDirtyWidgets = true;
  }

  /// Both globals back where they were - `flutter_test` fails the test if a
  /// foundation debug variable is left set.
  void remove() {
    debugPrintRebuildDirtyWidgets = false;
    if (_previous != null) debugPrint = _previous!;
  }

  int get total => byName.values.fold(0, (a, b) => a + b);
  void reset() => byName.clear();

  String ranking() {
    final entries = byName.entries.toList()
      ..sort((a, b) => b.value.compareTo(a.value));
    return entries.take(15).map((e) => '${e.value}x ${e.key}').join('\n');
  }
}

/// Counts what crosses the bridge, by name. The budget file's own shape, kept
/// here so this panel's gate does not depend on the Timeline's fixture.
class _Calls extends BaseHandler {
  final Map<String, int> byName = {};
  bool counting = false;

  /// The calls that put a clip somewhere else. A drag is allowed one of these,
  /// on release (docs/impl/audio-timeline.md §6, plan 11).
  static const _writes = {
    'layer_reference_slide_clip',
    'layer_reference_move_clip',
    'layer_reference_trim_clip',
    'layer_reference_convert_to_sequenced',
  };

  int get writes => _writes.fold(0, (sum, name) => sum + (byName[name] ?? 0));

  void reset() => byName.clear();

  String ranking() {
    final entries = byName.entries.toList()
      ..sort((a, b) => b.value.compareTo(a.value));
    return entries.take(10).map((e) => '${e.value}x ${e.key}').join('\n');
  }

  void _tick(String name) {
    if (counting) byName[name] = (byName[name] ?? 0) + 1;
  }

  @override
  Future<S> executeNormal<S, E extends Object>(NormalTask<S, E> task) {
    _tick(task.constMeta.debugName);
    return super.executeNormal(task);
  }

  @override
  S executeSync<S, E extends Object, WireSyncType>(
      SyncTask<S, E, WireSyncType> task) {
    _tick(task.constMeta.debugName);
    return super.executeSync(task);
  }
}

void main() {
  final calls = _Calls();

  setUpAll(() => initEngineForTests(handler: calls));

  /// A comp with one of each kind of track: a music layer, which is a track
  /// with a lane of its own, and a precomp that sounds, which is a picture as
  /// well and so is listed faded until its audio is detached.
  Future<({dynamic ui, CompositionReference comp, String music, String nested})>
      mount(WidgetTester tester) async {
    final p = freshProject();
    final wav = p.state.project!.importFootage(path: _toneWavFile());
    final inner = p.state.project!.newComposition(name: 'Music');
    inner.addFootageLayer(footage: wav, asSequence: false);
    final scene = p.state.project!.newComposition(name: 'Scene');
    scene.addFootageLayer(footage: wav, asSequence: false);
    final music = scene.getLayers().single.internallayerId.toString();
    scene.addPrecompLayer(comp: inner);
    final nested = [
      for (final layer in scene.getLayers())
        if (layer.internallayerId.toString() != music)
          layer.internallayerId.toString(),
    ].single;
    p.uiState.setSelectedComp(scene);
    p.uiState.model.refresh();

    tester.view.physicalSize = const Size(1280, 600);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(hostPanel(
      child: const AudioTimelinePanelFrb(),
      state: p.state,
      uiState: p.uiState,
      size: const Size(1280, 600),
    ));
    await tester.pump();
    // The audio and picture probes are real trips into FFmpeg.
    await settleFrb(tester, minRounds: 8);
    return (ui: p.uiState, comp: scene, music: music, nested: nested);
  }

  testWidgets('lists a track per audible layer, and fades the picture row',
      (tester) async {
    final p = await mount(tester);

    expect(find.text(l10n.panelAudioTimeline.toUpperCase()), findsOneWidget,
        reason: 'the tab strip names this panel, not the Timeline');
    expect(find.byKey(ValueKey<String>('atl-row-${p.music}')), findsOneWidget,
        reason: 'the music layer is a track');
    expect(find.byKey(ValueKey<String>('atl-lane-mode-${p.music}')),
        findsOneWidget,
        reason: 'a track wears the lane-mode chip at the right of its row');
    // The drawn box, not the hit area: the tap takes the row's height so the
    // chip is easy to hit, and the box takes only its text's.
    expect(
        tester
            .getSize(find
                .descendant(
                    of: find
                        .byKey(ValueKey<String>('atl-lane-mode-${p.music}')),
                    matching: find.byType(Container))
                .first)
            .height,
        lessThan(20),
        reason: 'the chip is one line of text tall, not the whole track');
    expect(find.byKey(ValueKey<String>('atl-wave-${p.music}')), findsOneWidget,
        reason: 'an unconverted Audio layer draws its own wave on the lane');

    expect(find.byKey(ValueKey<String>('atl-row-${p.nested}')), findsOneWidget,
        reason: 'a precomp that sounds is listed');
    expect(
        find.byKey(ValueKey<String>('atl-detach-${p.nested}')), findsOneWidget,
        reason:
            'a picture row wears Detach audio where a track wears its chip');
    expect(
        find.byKey(ValueKey<String>('atl-lane-mode-${p.nested}')), findsNothing,
        reason: 'and it wears one or the other, never both');

    expect(
        find.byKey(ValueKey<String>('tl-volume-band-${p.music}')), findsNothing,
        reason: "the track's Volume is not on this panel's lanes: it keeps "
            'its row under the twirl, and a clip wears its own gain line '
            'in its place');
  });

  testWidgets('the first edit marks the comp mixed, and only the first',
      (tester) async {
    final p = await mount(tester);
    expect(p.comp.soundMix(), isFalse,
        reason: 'looking at a comp writes nothing: the mark is an edit (§2)');

    // Every generated call goes through this handler, so counting is how a
    // second write is caught.
    const mark = 'composition_reference_set_sound_mix';
    calls
      ..reset()
      ..counting = true;
    for (var i = 0; i < 10; i++) {
      await tester.pump(const Duration(milliseconds: 16));
    }
    expect(calls.byName[mark] ?? 0, 0,
        reason: 'a comp on show was marked, which is an undo step for looking '
            'at it:\n${calls.ranking()}');

    // The mute switch, which is a write like any other made here.
    await tester.tap(find.byKey(ValueKey<String>('atl-audible-${p.music}')));
    await settleFrb(tester, minRounds: 3);
    expect(p.comp.soundMix(), isTrue,
        reason: 'the first edit made here is what marks the comp mixed');
    expect(calls.byName[mark] ?? 0, 1);

    await tester.tap(find.byKey(ValueKey<String>('atl-audible-${p.music}')));
    await settleFrb(tester, minRounds: 3);
    calls.counting = false;
    expect(calls.byName[mark] ?? 0, 1,
        reason: 'a comp already marked was marked again, which is an undo '
            'step per edit:\n${calls.ranking()}');
  });

  /// The ruler is a write road like the switches and the clips, so it ends
  /// where they end: a work area dragged here is an edit made here, and the
  /// first edit made here is what marks the comp (§2).
  testWidgets('a work-area drag on the ruler marks the comp', (tester) async {
    final p = await mount(tester);
    expect(p.comp.soundMix(), isFalse);

    // A span with room either side, so the handle stands out in the ruler
    // rather than hard against its end.
    p.comp.setWorkArea(
        span: workAreaWith(
            comp: p.comp,
            current: null,
            wanted: p.comp.durationFrames() ~/ 2,
            isStart: false));
    p.ui.model.refresh();
    await settleFrb(tester, minRounds: 2);
    final before = workAreaFrames(p.comp);

    final gesture = await tester.startGesture(
        tester.getCenter(find.byKey(const ValueKey('tl-work-end'))));
    await tester.pump();
    for (var i = 0; i < 6; i++) {
      await gesture.moveBy(const Offset(-12, 0));
      await tester.pump();
    }
    await gesture.up();
    await settleFrb(tester, minRounds: 3);

    expect(workAreaFrames(p.comp).end, lessThan(before.end),
        reason: 'the release wrote the span the drag asked for');
    expect(p.comp.soundMix(), isTrue,
        reason: 'a comp written to from this panel is a comp mixed here, '
            'whichever road did the writing');
  });

  testWidgets('the chrome strip reads the playhead and searches the tracks',
      (tester) async {
    final p = await mount(tester);
    expect(find.byKey(const ValueKey('atl-timecode')), findsOneWidget,
        reason: "the strip carries the playhead's own clock");
    expect(find.byKey(const ValueKey('atl-frame')), findsNothing,
        reason: 'the count and the length give their room to the search box, '
            'and the wide digits of the test face leave no way round it');
    expect(find.byKey(const ValueKey('atl-outline-seams')), findsOneWidget,
        reason: 'the outline rules the same rows the lanes do');
    // And rules them over the same ground: the chrome strip stands as tall as
    // the navigator's band and the chrome row together, then the column header
    // faces the ruler's lower row, so both viewports open at one height.
    final outline =
        tester.getRect(find.byKey(const ValueKey('atl-outline-blocks')));
    final lanes = tester.getRect(find.byKey(const ValueKey('atl-lane-blocks')));
    expect(outline.top, closeTo(lanes.top, 0.5),
        reason: 'the two halves spend the same height above their first '
            'track, or the seams rule rows the lanes have not reached');
    expect(find.byKey(const ValueKey('tl-view-lanes')), findsNothing,
        reason:
            'LAYERS and GRAPH are the layer Timeline\'s, not this panel\'s');

    final name = p.ui.model.layers
        .firstWhere((e) => e.layer.internallayerId.toString() == p.nested)
        .info
        .name;
    await tester.enterText(find.byKey(const ValueKey('atl-search')), name);
    await settleFrb(tester, minRounds: 2);
    expect(find.byKey(ValueKey<String>('atl-row-${p.nested}')), findsOneWidget,
        reason: 'the track the search names stays');
    expect(find.byKey(ValueKey<String>('atl-row-${p.music}')), findsNothing,
        reason: 'and one whose name does not carry it is not listed');

    await tester.enterText(find.byKey(const ValueKey('atl-search')), '');
    await settleFrb(tester, minRounds: 2);
    expect(find.byKey(ValueKey<String>('atl-row-${p.music}')), findsOneWidget,
        reason: 'clearing the box brings every track back');
  });

  /// The layer Convert to precomp leaves behind holds sound and nothing else,
  /// so it comes back here as a track of its own and not as a faded picture
  /// row (docs/impl/audio-timeline.md §2, §5).
  testWidgets('the packed mix comes back as a track', (tester) async {
    final p = await mount(tester);
    p.comp.setSoundMix(mixed: true);
    final mix = p.comp.precomposeSoundMix(name: 'Sound mix');
    p.ui.model.refresh();
    await settleFrb(tester, minRounds: 8);

    final id = mix.internallayerId.toString();
    expect(find.byKey(ValueKey<String>('atl-row-$id')), findsOneWidget,
        reason: 'the sound is all still there, so the row is listed');
    expect(find.byKey(ValueKey<String>('atl-lane-mode-$id')), findsOneWidget,
        reason: 'and it is worked on here, so it wears the lane-mode chip');
    expect(find.byKey(ValueKey<String>('atl-detach-$id')), findsNothing,
        reason: 'there is no picture on it to detach the sound from');
  });

  testWidgets('the twirl opens straight on to Volume', (tester) async {
    final p = await mount(tester);
    final volume =
        find.byKey(ValueKey<String>('atl-prop-${p.music}/audio/volume'));
    expect(volume, findsNothing, reason: 'a track opens shut');

    await tester.tap(find.byKey(ValueKey<String>('atl-twirl-${p.music}')));
    await settleFrb(tester, minRounds: 2);

    expect(volume, findsOneWidget,
        reason: 'no Audio heading stands over it, and the path is unchanged');
    expect(find.text('Audio'), findsNothing,
        reason: 'the Audio heading belongs to the layer Timeline');
  });

  testWidgets('a press on a track picks its layer and lights the name',
      (tester) async {
    final p = await mount(tester);
    final name = find.byKey(ValueKey<String>('atl-name-${p.music}'));
    final resting = tester.widget<Text>(name).style;

    await tester.tap(name);
    await tester.pump();
    expect(p.ui.selectedLayer.value?.internallayerId.toString(), p.music,
        reason: 'the press is the shell selection, so the Effect controls '
            'panel follows the track');
    expect(tester.widget<Text>(name).style, isNot(resting),
        reason: "a picked track's name reads at full strength");

    // A faded picture row is deaf: its sound is worked on once it has been
    // detached, and until then the row cannot be chosen at all.
    await tester.tap(find.byKey(ValueKey<String>('atl-name-${p.nested}')),
        warnIfMissed: false);
    await tester.pump();
    expect(p.ui.selectedLayer.value?.internallayerId.toString(), p.music,
        reason: 'the faded row took the press it should have ignored');
  });

  testWidgets('Enter renames the picked track', (tester) async {
    final p = await mount(tester);
    p.ui.activePane.value = Panel.audioTimeline.pane();
    await tester.tap(find.byKey(ValueKey<String>('atl-name-${p.music}')));
    await tester.pump();

    final editor = find.byKey(ValueKey<String>('atl-rename-${p.music}'));
    expect(editor, findsNothing, reason: 'one press is not a rename');

    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(editor, findsOneWidget,
        reason: 'Enter turns the name into the field the layer Timeline uses');

    await tester.enterText(editor, 'Bassline');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await settleFrb(tester, minRounds: 3);

    expect(
        p.ui.model.layers
            .firstWhere((e) => e.layer.internallayerId.toString() == p.music)
            .info
            .name,
        'Bassline',
        reason: 'what was typed is the layer\'s name');
    expect(editor, findsNothing, reason: 'and the editor shuts behind it');
  });

  testWidgets('the Effects heading stands empty and its glyph fills the rack',
      (tester) async {
    final p = await mount(tester);
    expect(
        p.ui.model.layers
            .firstWhere((e) => e.layer.internallayerId.toString() == p.music)
            .info
            .effects,
        isEmpty,
        reason: 'the track carries no plugin yet');

    await tester.tap(find.byKey(ValueKey<String>('atl-twirl-${p.music}')));
    await settleFrb(tester, minRounds: 2);
    expect(find.byKey(ValueKey<String>('atl-prop-${p.music}/effects')),
        findsOneWidget,
        reason: 'the heading stands whether or not the rack holds anything');

    await tester
        .tap(find.byKey(ValueKey<String>('atl-track-add-effect-${p.music}')));
    await tester.pump();
    expect(lumitPopupOpen, isTrue,
        reason: "the track's own catalogue drops from the glyph");
    closeLumitPopups();
    await tester.pump();
  });

  /// A track's height is its own (docs/impl/audio-timeline.md §6, plan 20):
  /// the drag on the outline row's bottom edge, the two halves following it
  /// together, and both ends of the range holding.
  testWidgets('a drag on the row edge sets the track height', (tester) async {
    final p = await mount(tester);
    final row = find.byKey(ValueKey<String>('atl-row-${p.music}'));
    final lane = find.byKey(ValueKey<String>('atl-wave-${p.music}'));
    final band = tester.getSize(row).height;
    final step = band / audioTrackMinRows;
    expect(tester.getSize(lane).height, closeTo(band, 0.5),
        reason: 'the two halves open at one height');

    final gesture = await tester.startGesture(tester
        .getCenter(find.byKey(ValueKey<String>('atl-resize-${p.music}'))));
    await gesture.moveBy(Offset(0, step + 2));
    await tester.pump();
    expect(tester.getSize(row).height, closeTo(band + step, 0.5),
        reason: 'a lane row of travel is a lane row of height');
    expect(tester.getSize(lane).height, closeTo(band + step, 0.5),
        reason: 'and the lane half is the same table, so it followed');

    await gesture.moveBy(Offset(0, step * 20));
    await tester.pump();
    expect(tester.getSize(row).height, closeTo(step * audioTrackMaxRows, 0.5),
        reason: 'eight lane rows is as tall as a track goes');
    expect(tester.getSize(lane).height, closeTo(step * audioTrackMaxRows, 0.5));

    await gesture.moveBy(Offset(0, -step * 20));
    await tester.pump();
    expect(tester.getSize(row).height, closeTo(step * audioTrackMinRows, 0.5),
        reason: 'and two is as short');
    await gesture.up();
    await tester.pump();
  });

  /// Plan 20: a drag ends where it ends, and what it left short of a whole
  /// lane row is not spent on the next one - the same rule the first drag is
  /// held to, over two gestures instead of one.
  testWidgets('a fresh drag on the row edge starts its travel at nothing',
      (tester) async {
    final p = await mount(tester);
    final row = find.byKey(ValueKey<String>('atl-row-${p.music}'));
    final edge = find.byKey(ValueKey<String>('atl-resize-${p.music}'));
    final band = tester.getSize(row).height;
    final step = band / audioTrackMinRows;

    final first = await tester.startGesture(tester.getCenter(edge));
    await first.moveBy(Offset(0, step * 0.9));
    await tester.pump();
    expect(tester.getSize(row).height, closeTo(band, 0.5),
        reason: 'nine tenths of a lane row is not a lane row');
    await first.up();
    await tester.pump();

    final second = await tester.startGesture(tester.getCenter(edge));
    await second.moveBy(Offset(0, step * 0.2));
    await tester.pump();
    expect(tester.getSize(row).height, closeTo(band, 0.5),
        reason: 'two tenths of a row of travel grew the track a whole row, so '
            'the drag before it banked its own');
    await second.up();
    await tester.pump();
  });

  /// A **short** comp with two converted tracks, one above the other.
  ///
  /// One second at the default rate, so half a second of tone covers half the
  /// lanes and a clip is wide enough to take hold of, read the header of, and
  /// drag on to the row below.
  ///
  /// [beside] mounts a second panel under this one, for the claims: the shell's
  /// key slots hold one callback each, and the point of the arbitration is that
  /// two panels can stand on screen without taking them from each other.
  Future<
          ({
            LumitUiState ui,
            LayerReference top,
            LayerReference bottom,
            FootageReference wav
          })>
      mountClips(WidgetTester tester,
          {Widget? beside,
          Size size = const Size(1280, 600),
          BridgeRational duration =
              const BridgeRational(num: 1, den: 1)}) async {
    final p = freshProject();
    final wav = p.state.project!.importFootage(path: _toneWavFile());
    final defaults = BridgeCompSettings.defaults();
    final scene = p.state.project!.newComposition(
      name: 'Scene',
      settings: BridgeCompSettings(
        name: 'Scene',
        width: defaults.width,
        height: defaults.height,
        fpsNum: defaults.fpsNum,
        fpsDen: defaults.fpsDen,
        duration: duration,
        background: defaults.background,
        shutterAngle: defaults.shutterAngle,
        motionBlurSamples: defaults.motionBlurSamples,
      ),
    );
    scene.addFootageLayer(footage: wav, asSequence: false);
    scene.addFootageLayer(footage: wav, asSequence: false);
    final layers = scene.getLayers();
    for (final layer in layers) {
      layer.convertToSequenced();
    }
    p.uiState.setSelectedComp(scene);
    p.uiState.model.refresh();

    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(hostPanel(
      child: beside == null
          ? const AudioTimelinePanelFrb()
          : Column(children: [
              const Expanded(child: AudioTimelinePanelFrb()),
              Expanded(child: beside),
            ]),
      state: p.state,
      uiState: p.uiState,
      size: size,
    ));
    await tester.pump();
    await settleFrb(tester, minRounds: 8);
    return (ui: p.uiState, top: layers.first, bottom: layers.last, wav: wav);
  }

  /// A point on a clip's box that is the clip's own: below the header strip,
  /// and clear of the gain line, which lies at the top of the box until it is
  /// pulled down.
  Offset onBody(WidgetTester tester, BridgeClip clip, {double fromRight = 0}) {
    final box =
        tester.getRect(find.byKey(ValueKey<String>('atl-clip-${clip.id}')));
    return Offset(
        fromRight > 0 ? box.right - fromRight : box.center.dx, box.bottom - 6);
  }

  /// **The chrome strip's room** (docs/impl/audio-timeline.md §5). The outline
  /// is 300 px whatever the comp is, and the readouts grow with it: a long
  /// comp's frame count and length will take the search box's room down to
  /// nothing if they are let, and the box overflows its own well. The box keeps
  /// its floor and the two readouts go instead.
  testWidgets('a long comp leaves the search box its room', (tester) async {
    final search = find.byKey(const ValueKey('atl-search'));
    // Fifty seconds at sixty, which is three thousand frames: the length the
    // readouts first grow enough on to squeeze the box out of its own well.
    await mountClips(tester, duration: const BridgeRational(num: 50, den: 1));
    expect(search, findsOneWidget,
        reason: 'the tracks are searched whatever the comp is');
    expect(tester.getSize(search).width,
        greaterThanOrEqualTo(audioTimelineSearchFloor),
        reason: 'and the box is never squeezed under the width it reads at');

    // And a hundred thousand frames, which is the widest the strip is ever
    // asked to carry.
    await mountClips(tester, duration: const BridgeRational(num: 5000, den: 3));
    expect(tester.getSize(search).width,
        greaterThanOrEqualTo(audioTimelineSearchFloor));
    expect(find.byKey(const ValueKey('atl-timecode')), findsOneWidget,
        reason: 'the clock is the last thing the strip gives up');
  });

  /// The rule the strip goes down, in numbers: a 300 px outline less its own
  /// insets, with the readouts a comp of a few thousand frames puts in it and
  /// the ones a comp of a hundred thousand does.
  test('the count and the length go before the search box does', () {
    expect(
        audioTimelineChromeCarriesCount(
            strip: 282, clock: 87, count: 50, length: 30),
        isTrue,
        reason: 'an ordinary comp says the time, the frame and the length');
    expect(
        audioTimelineChromeCarriesCount(
            strip: 282, clock: 87, count: 62, length: 42),
        isFalse,
        reason: 'a longer one has room for the readouts or the box, not both');
  });

  testWidgets('a converted track draws a box per clip, with its header',
      (tester) async {
    final p = await mountClips(tester);
    final clip = p.top.getClips().single;

    expect(find.byKey(ValueKey<String>('atl-clip-${clip.id}')), findsOneWidget);
    expect(find.byKey(ValueKey<String>('atl-clip-edge-${clip.id}')),
        findsOneWidget,
        reason: 'the solid leading edge says where this cut begins');
    expect(find.byKey(ValueKey<String>('atl-clip-name-${clip.id}')),
        findsOneWidget,
        reason: 'the header names what the clip plays');
    expect(find.byKey(ValueKey<String>('atl-clip-colour-${clip.id}')),
        findsOneWidget,
        reason: "the colour box opens the track's own label picker");
  });

  /// How strongly a clip's box is filled: the picked one is held stronger.
  double fillAlpha(WidgetTester tester, BridgeClip clip) {
    final box = tester.widget<Container>(find
        .descendant(
          of: find.byKey(ValueKey<String>('atl-clip-${clip.id}')),
          matching: find.byType(Container),
        )
        .first);
    return (box.decoration! as BoxDecoration).color!.a;
  }

  testWidgets('a click picks a clip and empty ground lets it go',
      (tester) async {
    final p = await mountClips(tester);
    final clip = p.top.getClips().single;
    final box =
        tester.getRect(find.byKey(ValueKey<String>('atl-clip-${clip.id}')));
    expect(fillAlpha(tester, clip), closeTo(clipFillAlpha, 0.001));

    await tester.tapAt(onBody(tester, clip));
    await tester.pump();
    expect(fillAlpha(tester, clip), closeTo(clipFillSelectedAlpha, 0.001),
        reason: 'a picked clip is its own colour held stronger');

    await tester.tapAt(Offset(box.right + 60, box.center.dy));
    await tester.pump();
    expect(fillAlpha(tester, clip), closeTo(clipFillAlpha, 0.001),
        reason: 'a click on empty ground means nothing is selected');
  });

  /// A press taken away is not a gesture, so what it claimed on Escape has to
  /// come off with it. The ladder stops at the first rung that answers, and a
  /// drag that is no longer in flight would answer over the picked clip.
  testWidgets('a cancelled press leaves Escape to the picked clip',
      (tester) async {
    final p = await mountClips(tester);
    final clip = p.top.getClips().single;

    for (final (what, at) in [
      ('the clip body', onBody(tester, clip)),
      (
        'the gain line',
        tester.getCenter(find.byKey(ValueKey<String>('atl-gain-${clip.id}')))
      ),
      (
        'the fade corner',
        tester.getCenter(find.byKey(ValueKey<String>('atl-fade-in-${clip.id}')))
      ),
    ]) {
      await tester.tapAt(onBody(tester, clip));
      await tester.pump();
      expect(fillAlpha(tester, clip), closeTo(clipFillSelectedAlpha, 0.001),
          reason: 'the clip has to be picked before Escape has anything to do');

      final gesture = await tester.startGesture(at);
      await tester.pump();
      await gesture.cancel();
      await tester.pump();

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      expect(fillAlpha(tester, clip), closeTo(clipFillAlpha, 0.001),
          reason: 'a cancelled press on $what kept Escape from the clip');
    }
  });

  testWidgets('a body drag slides the clip along its track', (tester) async {
    final p = await mountClips(tester);
    final before = p.top.getClips().single;

    final gesture = await tester.startGesture(onBody(tester, before));
    await tester.pump();
    await gesture.moveBy(const Offset(120, 0));
    await tester.pump();
    await gesture.up();
    await settleFrb(tester, minRounds: 3);

    final after = p.top.getClips().single;
    expect(after.startFrame, greaterThan(before.startFrame),
        reason: 'the clip did not move');
    expect(
        after.endFrame - after.startFrame, before.endFrame - before.startFrame,
        reason: 'a slide moves a clip, it does not stretch it');
  });

  /// Plan 17: while a drag is on, every part of the box travels with it. The
  /// fades are drawn by a layer stacked **over** the clip strip rather than
  /// inside it, so the ramps and the corner handles are what used to sit on the
  /// frames the document still held.
  testWidgets('a body drag carries the fades with the box', (tester) async {
    final p = await mountClips(tester);
    final clip = p.top.getClips().single;
    p.top.setClipFade(
      clip: clip.id,
      fadeIn:
          const BridgeClipFade(seconds: 0.1, shape: BridgeClipFadeShape.fast()),
    );
    p.ui.model.refresh();
    await settleFrb(tester, minRounds: 3);

    final box = find.byKey(ValueKey<String>('atl-clip-${clip.id}'));
    final before = tester.getRect(box);
    final perFrame = before.width / (clip.endFrame - clip.startFrame);

    final gesture = await tester.startGesture(onBody(tester, clip));
    await tester.pump();
    await gesture.moveBy(const Offset(120, 0));
    await tester.pump();

    final after = tester.getRect(box);
    final shift = ((after.left - before.left) / perFrame).round();
    expect(shift, greaterThan(0), reason: 'the box itself never moved');
    final painter = tester
        .widget<CustomPaint>(
            find.byKey(ValueKey<String>('atl-fades-${p.top.internallayerId}')))
        .painter! as ClipFadePainter;
    final ramp =
        painter.ramps.firstWhere((r) => r.clip == clip.id.toString() && r.into);
    expect(ramp.from, closeTo((clip.startFrame + shift).toDouble(), 0.01),
        reason: 'the ramp stayed on the frames the document still holds');
    expect(
        tester
            .getCenter(find.byKey(ValueKey<String>('atl-fade-out-${clip.id}')))
            .dx,
        closeTo(after.right, 2),
        reason: 'and the corner handle stayed at the committed edge');

    await gesture.up();
    await settleFrb(tester, minRounds: 3);
  });

  testWidgets('an edge drag trims the clip', (tester) async {
    final p = await mountClips(tester);
    final before = p.top.getClips().single;

    final gesture =
        await tester.startGesture(onBody(tester, before, fromRight: 4));
    await tester.pump();
    await gesture.moveBy(const Offset(-100, 0));
    await tester.pump();
    await gesture.up();
    await settleFrb(tester, minRounds: 3);

    final after = p.top.getClips().single;
    expect(after.startFrame, before.startFrame,
        reason: 'a trim at one end leaves the other where it was');
    expect(after.endFrame, lessThan(before.endFrame));
  });

  /// Plan 15: a head pulled back past the source's first sample plays
  /// silence until the sound arrives, so the box marks where that is and the
  /// head snaps to it on the way back.
  testWidgets('a head past the source start is marked, and snaps back to it',
      (tester) async {
    // A narrow panel, so the lane draws a frame in under sixteen pixels and
    // the magnet's eight are worth more than half of one: a drag can then stop
    // short of the sound and still be taken by it, which is the whole of what
    // the extra target does.
    final p = await mountClips(tester, size: const Size(900, 600));
    final untouched = p.bottom.getClips().single;
    final first = p.top.getClips().single;
    // Slid along the row, then pulled back out past its own beginning.
    p.top.slideClip(
        clip: first.id, toFrame: first.startFrame + 6, overlap: true);
    final slid = p.top.getClips().single;
    p.top.trimClip(clip: slid.id, startFrame: 0, endFrame: slid.endFrame);
    p.ui.model.refresh();
    await settleFrb(tester, minRounds: 3);

    final clip = p.top.getClips().single;
    final reach = clip.reachStartFrame;
    expect(reach, isNotNull);
    expect(reach, greaterThan(clip.startFrame),
        reason: 'the head is out in front of the sound it plays');
    expect(find.byKey(ValueKey<String>('atl-source-start-${clip.id}')),
        findsOneWidget,
        reason: 'the box marks the frame the sound starts on');
    expect(find.byKey(ValueKey<String>('atl-source-start-${untouched.id}')),
        findsNothing,
        reason: 'a clip that hides none of its sound is marked nowhere');

    // The head dragged back towards the sound, stopping short of it by more
    // than half a frame: the whole-frame rounding alone would leave it there,
    // and only the source's own target pulls it the rest of the way.
    final box =
        tester.getRect(find.byKey(ValueKey<String>('atl-clip-${clip.id}')));
    final perFrame = box.width / (clip.endFrame - clip.startFrame);
    final gesture =
        await tester.startGesture(Offset(box.left + 3, box.bottom - 6));
    await tester.pump();
    await gesture
        .moveBy(Offset(perFrame * ((reach! - clip.startFrame) - 0.7), 0));
    await tester.pump();
    await gesture.up();
    await settleFrb(tester, minRounds: 3);

    final after = p.top.getClips().single;
    expect(after.startFrame, reach,
        reason: 'the head landed where the pointer left it, not on the frame '
            'the sound starts on');
  });

  /// Plan 19: the head lands on the source's start and the tail on its end
  /// whichever way the edge is moving, so an end taken in comes back out on to
  /// the sound's own edge.
  testWidgets('a trimmed end comes back out on to the source', (tester) async {
    // The narrow panel again: a frame drawn in about ten pixels, so the
    // magnet's eight are worth more than half of one and a drag that stops
    // short of the source can only be finished by the source's own target.
    final p = await mountClips(tester, size: const Size(900, 600));
    final whole = p.top.getClips().single;
    // Trimmed in at both ends and then slid along the row, so neither reach
    // stands on a layer's own in or out point and nothing else is there to
    // catch the drag.
    p.top.trimClip(
        clip: whole.id,
        startFrame: whole.startFrame + 4,
        endFrame: whole.endFrame - 4);
    final trimmed = p.top.getClips().single;
    p.top.slideClip(
        clip: trimmed.id, toFrame: trimmed.startFrame + 10, overlap: true);
    p.ui.model.refresh();
    await settleFrb(tester, minRounds: 3);

    final clip = p.top.getClips().single;
    final reachIn = clip.reachStartFrame;
    final reachOut = clip.reachEndFrame;
    expect(reachIn, isNotNull);
    expect(reachOut, isNotNull,
        reason: 'the tail has no target without a reach at the source end');
    expect(reachIn, lessThan(clip.startFrame),
        reason: 'the head is trimmed in, so the sound starts before it');
    expect(reachOut, greaterThan(clip.endFrame));

    final box =
        tester.getRect(find.byKey(ValueKey<String>('atl-clip-${clip.id}')));
    final perFrame = box.width / (clip.endFrame - clip.startFrame);
    // Stopping six tenths of a frame short of the sound: the rounding to whole
    // frames alone leaves the edge a frame away from it.
    final head =
        await tester.startGesture(Offset(box.left + 3, box.bottom - 6));
    await tester.pump();
    await head
        .moveBy(Offset(perFrame * ((reachIn! - clip.startFrame) + 0.6), 0));
    await tester.pump();
    await head.up();
    await settleFrb(tester, minRounds: 3);

    final headOut = p.top.getClips().single;
    expect(headOut.startFrame, reachIn,
        reason: 'the head stopped short of the sound it plays');

    final tailBox =
        tester.getRect(find.byKey(ValueKey<String>('atl-clip-${headOut.id}')));
    final tail = await tester
        .startGesture(Offset(tailBox.right - 3, tailBox.bottom - 6));
    await tester.pump();
    await tail
        .moveBy(Offset(perFrame * ((reachOut! - headOut.endFrame) - 0.6), 0));
    await tester.pump();
    await tail.up();
    await settleFrb(tester, minRounds: 3);

    expect(p.top.getClips().single.endFrame, reachOut,
        reason: 'the tail stopped short of the end of the sound');
  });

  testWidgets('a corner drag fades the clip in', (tester) async {
    final p = await mountClips(tester);
    final before = p.top.getClips().single;
    expect(before.fadeIn.seconds, 0, reason: 'a fresh clip does not fade');

    final corner = find.byKey(ValueKey<String>('atl-fade-in-${before.id}'));
    expect(corner, findsOneWidget,
        reason: 'each top corner carries the handle that drags its fade');
    final gesture = await tester.startGesture(tester.getCenter(corner));
    await tester.pump();
    await gesture.moveBy(const Offset(90, 0));
    await tester.pump();
    expect(
        find.byKey(const ValueKey<String>('atl-fade-readout')), findsOneWidget,
        reason: 'the length of the fade shows while it is being dragged');
    await gesture.up();
    await settleFrb(tester, minRounds: 3);

    final after = p.top.getClips().single;
    expect(after.fadeIn.seconds, greaterThan(0),
        reason: 'the corner was dragged inward and no fade was written');
    expect(after.startFrame, before.startFrame,
        reason: 'a fade is not a trim: the clip is where it was');
  });

  /// Plan 18: the clip's own level, drawn across its box and taken hold of on
  /// the line. Written once, on release, and the fades rise to it.
  testWidgets('a drag on the gain line writes the clip gain once',
      (tester) async {
    final p = await mountClips(tester);
    final clip = p.top.getClips().single;
    expect(clip.gainDb, 0,
        reason: 'a fresh clip plays at the level it was cut at');
    expect(
        find.byKey(ValueKey<String>('tl-volume-band-${p.top.internallayerId}')),
        findsNothing,
        reason: 'a converted track has no band on its lane either');
    p.top.setClipFade(
      clip: clip.id,
      fadeIn:
          const BridgeClipFade(seconds: 0.2, shape: BridgeClipFadeShape.fast()),
    );
    p.ui.model.refresh();
    await settleFrb(tester, minRounds: 3);

    final lane =
        find.byKey(ValueKey<String>('atl-fades-${p.top.internallayerId}'));
    double rampTop() =>
        (tester.widget<CustomPaint>(lane).painter! as ClipFadePainter)
            .topOf(clip.id.toString(), tester.getSize(lane).height);
    final corner = find.byKey(ValueKey<String>('atl-fade-in-${clip.id}'));
    final wasTop = rampTop();
    final wasCorner = tester.getCenter(corner).dy;

    final line = find.byKey(ValueKey<String>('atl-gain-${clip.id}'));
    expect(line, findsOneWidget, reason: 'every clip wears its own gain line');
    const write = 'layer_reference_set_clip_gain';
    calls
      ..reset()
      ..counting = true;
    final gesture = await tester.startGesture(tester.getCenter(line));
    await tester.pump();
    for (var i = 0; i < 4; i++) {
      await gesture.moveBy(const Offset(0, 6));
      await tester.pump();
    }
    expect(calls.byName[write] ?? 0, 0,
        reason: 'a pointer move wrote the gain, so a drag is as many undo '
            'steps as it has pixels:\n${calls.ranking()}');
    expect(find.byKey(ValueKey<String>('atl-gain-readout-${clip.id}')),
        findsOneWidget,
        reason: 'the level shows beside the line while it is dragged');

    await gesture.up();
    await settleFrb(tester, minRounds: 3);
    calls.counting = false;
    expect(calls.byName[write] ?? 0, 1,
        reason: 'the drag is staged in Dart and written once, on release:\n'
            '${calls.ranking()}');
    expect(p.top.getClips().single.gainDb, lessThan(0),
        reason: 'the line was pulled down and the clip is no quieter');
    expect(rampTop(), greaterThan(wasTop),
        reason: 'the ramp still rises to the top of the box rather than to '
            "the clip's own gain line");
    expect(tester.getCenter(corner).dy, greaterThan(wasCorner),
        reason: 'and the corner handle came down on to the line with it');

    // The line takes every press that lands on it, so the clip's own menu has
    // to be reachable through it.
    final menu = await tester.startGesture(tester.getCenter(line),
        kind: PointerDeviceKind.mouse, buttons: kSecondaryMouseButton);
    await menu.up();
    await tester.pumpAndSettle();
    expect(find.byKey(ValueKey<String>('atl-clip-split-${clip.id}')),
        findsOneWidget,
        reason: 'a right click on the gain line found no way to the clip menu');
    closeLumitPopups();
    await tester.pumpAndSettle();
  });

  testWidgets('a corner on an unconverted layer converts it first',
      (tester) async {
    final p = await mount(tester);
    final entry = p.ui.model.layers
        .firstWhere((e) => e.layer.internallayerId.toString() == p.music);
    expect(entry.info.clips, isEmpty, reason: 'the row has not been cut yet');

    final gesture = await tester.startGesture(tester
        .getCenter(find.byKey(ValueKey<String>('atl-fade-in-${p.music}'))));
    await tester.pump();
    await gesture.moveBy(const Offset(60, 0));
    await tester.pump();
    await gesture.up();
    await settleFrb(tester, minRounds: 4);

    final clips = entry.layer.getClips() as List<BridgeClip>;
    expect(clips.length, 1, reason: 'the layer became the one clip it was');
    expect(clips.single.fadeIn.seconds, greaterThan(0),
        reason: 'and the fade the corner asked for was written on it');
  });

  testWidgets('a corner inside an overlap stands on the clip edge',
      (tester) async {
    final p = await mountClips(tester);
    final first = p.top.getClips().single;
    // A fade out, and then a clip laid across that end: the overlap is the
    // crossfade from here on, so the corner drags the edge and has to stand
    // on it rather than at seconds nothing reads any more.
    p.top.setClipFade(
      clip: first.id,
      fadeOut:
          const BridgeClipFade(seconds: 0.3, shape: BridgeClipFadeShape.fast()),
    );
    p.top.addClip(footage: p.wav, atFrame: first.endFrame - 4, overlap: true);
    p.ui.model.refresh();
    await settleFrb(tester, minRounds: 3);

    final box =
        tester.getRect(find.byKey(ValueKey<String>('atl-clip-${first.id}')));
    final handle = tester
        .getCenter(find.byKey(ValueKey<String>('atl-fade-out-${first.id}')));
    expect(handle.dx, closeTo(box.right, 2),
        reason: 'the handle was drawn at the seconds the clip stores, away '
            'from the join it drags');
  });

  testWidgets('a right click on a fade opens the fade menu', (tester) async {
    final p = await mountClips(tester);
    final clip = p.top.getClips().single;
    p.top.setClipFade(
      clip: clip.id,
      fadeIn:
          const BridgeClipFade(seconds: 0.3, shape: BridgeClipFadeShape.fast()),
    );
    p.ui.model.refresh();
    await settleFrb(tester, minRounds: 3);

    final box =
        tester.getRect(find.byKey(ValueKey<String>('atl-clip-${clip.id}')));
    // Inside the ramp, and below the header strip.
    final gesture = await tester.startGesture(
        Offset(box.left + 6, box.bottom - 6),
        kind: PointerDeviceKind.mouse,
        buttons: kSecondaryMouseButton);
    await gesture.up();
    await tester.pumpAndSettle();

    expect(lumitPopupOpen, isTrue);
    expect(find.byKey(ValueKey<String>('atl-fade-shape-fast-${clip.id}')),
        findsOneWidget,
        reason: 'the fade menu names the five shapes');
    expect(find.byKey(ValueKey<String>('atl-fade-custom-${clip.id}')),
        findsOneWidget,
        reason: 'and offers the editor after them');
    expect(
        find.byKey(ValueKey<String>('atl-clip-split-${clip.id}')), findsNothing,
        reason: 'a right click on a fade is about the fade, not the clip');
    closeLumitPopups();
    await tester.pumpAndSettle();
  });

  testWidgets('the razor cuts a clip where it is clicked', (tester) async {
    final p = await mountClips(tester);
    final clip = p.top.getClips().single;
    p.ui.tools.select(ToolMode.razor);
    await tester.pump();

    await tester.tapAt(onBody(tester, clip));
    await settleFrb(tester, minRounds: 3);

    expect(p.top.getClips().length, 2,
        reason: 'a razor click on a track makes an edit point, not two layers');
  });

  testWidgets('footage dropped on a track becomes a clip on it',
      (tester) async {
    final p = await mountClips(tester);
    final clip = p.top.getClips().single;
    final box =
        tester.getRect(find.byKey(ValueKey<String>('atl-clip-${clip.id}')));

    tester
        .widget<DragTarget<Object>>(find.byType(DragTarget<Object>).first)
        .onAcceptWithDetails!(DragTargetDetails<Object>(
      data: FootageDragData([p.wav], 'tone'),
      offset: Offset(box.right + 20, box.center.dy),
    ));
    await settleFrb(tester, minRounds: 6);

    expect(p.top.getClips().length, 2,
        reason: 'the drop landed on a track, so it is a clip and not a layer');
    expect(p.ui.model.layers.length, 2, reason: 'and no new layer was made');
  });

  testWidgets(
      'a drag that crosses a track moves the clip there, beside a '
      'marquee started on empty ground', (tester) async {
    final p = await mountClips(tester);
    final clip = p.top.getClips().single;
    final box =
        tester.getRect(find.byKey(ValueKey<String>('atl-clip-${clip.id}')));
    // The marquee is under the lanes and takes empty ground; a clip's own
    // pointer must still be the clip's.
    await tester.dragFrom(
        Offset(box.right + 40, box.center.dy), const Offset(80, 20));
    await tester.pump();

    final other = p.bottom.getClips().single;
    final target =
        tester.getRect(find.byKey(ValueKey<String>('atl-clip-${other.id}')));
    final from = onBody(tester, clip);
    final gesture = await tester.startGesture(from);
    await tester.pump();
    await gesture.moveBy(Offset(0, target.center.dy - from.dy));
    await tester.pump();
    await gesture.up();
    await settleFrb(tester, minRounds: 4);

    expect(p.top.getClips(), isEmpty,
        reason: 'the clip left the row it was on');
    expect(p.bottom.getClips().length, 2,
        reason: 'and landed on the one below, over what was already there');
  });

  testWidgets('the clip header bypasses its stack and offers audio plugins',
      (tester) async {
    final p = await mountClips(tester);
    final clip = p.top.getClips().single;
    expect(clip.fx, isTrue, reason: 'a clip is heard through its stack');

    await tester.tap(find.byKey(ValueKey<String>('atl-clip-fx-${clip.id}')));
    await settleFrb(tester, minRounds: 2);
    expect(p.top.getClips().single.fx, isFalse,
        reason: "the clip's own switch bypasses the clip's own stack");
    expect(fillAlpha(tester, clip), closeTo(clipFillAlpha, 0.001),
        reason: 'pressing a control is not also picking the clip under it');

    await tester
        .tap(find.byKey(ValueKey<String>('atl-clip-add-effect-${clip.id}')));
    await tester.pump();
    expect(lumitPopupOpen, isTrue,
        reason: 'the add-effect menu drops from the button');
    // The catalogue narrowed to what a clip can be heard through: the one
    // Audio heading, holding Lumit's own effects and whatever plugins this
    // machine has, and none of the picture families this build also carries.
    final sound = listEffects().where((e) => e.category == 'audio').toList();
    expect(sound, isNotEmpty, reason: 'the built-in suite is in the catalogue');
    expect(listEffects().where((e) => e.category != 'audio'), isNotEmpty,
        reason: 'the filter is only worth asserting if it left something out');
    expect(find.byType(SubmenuRow), findsOneWidget);
    expect(find.byKey(const ValueKey<String>('fx-category-audio')),
        findsOneWidget);

    // And behind it the effects themselves, by their sentence-case labels.
    await tester.tap(find.byKey(const ValueKey<String>('fx-category-audio')));
    await tester.pump();
    for (final label in ['Gain', 'Parametric EQ', 'Compressor', 'Reverb']) {
      expect(find.text(label), findsOneWidget, reason: '$label is offered');
    }
    expect(find.text('Gaussian blur'), findsNothing,
        reason: 'and nothing that draws pixels is');
    closeLumitPopups();
    await tester.pump();
  });

  testWidgets("a clip's twirl opens its own effect rows", (tester) async {
    final p = await mountClips(tester);
    p.top.addClipEffect(clip: p.top.getClips().single.id, name: 'blur');
    p.ui.model.refresh();
    await settleFrb(tester, minRounds: 2);
    final clip = p.top.getClips().single;
    final fx = clip.effects.single;
    final head = 'c:${clip.id}';
    final fxPath = '$head/effects/${fx.id}';

    expect(find.byKey(ValueKey<String>('atl-prop-$head')), findsNothing,
        reason: 'a clip opens shut');

    await tester.tap(find.byKey(ValueKey<String>('atl-clip-twirl-${clip.id}')));
    await settleFrb(tester, minRounds: 2);
    expect(find.byKey(ValueKey<String>('atl-prop-$head')), findsOneWidget,
        reason: "the drop-down opens on to the clip's name");
    expect(find.byKey(ValueKey<String>('atl-prop-$fxPath')), findsOneWidget,
        reason: 'one heading per effect on the clip');
    expect(find.byKey(ValueKey<String>('atl-twirl-${p.top.internallayerId}')),
        findsOneWidget,
        reason:
            "the track's own twirl is untouched: a clip answers for itself");

    await tester.tap(find.byKey(ValueKey<String>('tl-twirl-$fxPath')));
    await settleFrb(tester, minRounds: 2);
    expect(
        find.byKey(ValueKey<String>('atl-prop-$fxPath/radius')), findsOneWidget,
        reason: "the effect's parameters are rows like any other's");

    // The heading's own bypass tick, which is the only place a clip's effect
    // can be switched off: the write goes through the clip's instance.
    await tester.tap(find.byKey(ValueKey<String>('fx-enabled-hit-$fxPath')));
    await settleFrb(tester, minRounds: 2);
    expect(p.top.getClips().single.effects.single.enabled, isFalse);
  });

  testWidgets('with both timelines up, Delete reaches the one holding the keys',
      (tester) async {
    final p = await mountClips(tester, beside: const TimelinePanelFrb());
    final clip = p.top.getClips().single;
    await tester.tapAt(onBody(tester, clip));
    await tester.pump();

    // The layer Timeline mounted last and took the slot; the Audio timeline
    // takes it while it is the focused panel and gives it straight back.
    p.ui.activePane.value = Panel.timeline.pane();
    await tester.pump();
    expect(p.ui.deleteClaim?.call() ?? false, isFalse,
        reason:
            'the Timeline has no keys or masks picked, so it takes nothing');
    expect(p.top.getClips(), hasLength(1),
        reason: 'a blurred panel must not answer for the focused one');

    p.ui.activePane.value = Panel.audioTimeline.pane();
    await tester.pump();
    expect(p.ui.deleteClaim?.call() ?? false, isTrue);
    await settleFrb(tester, minRounds: 2);
    expect(p.top.getClips(), isEmpty,
        reason: 'the picked clip goes when this panel holds the keys');
  });

  group('Budgets', () {
    late _Rebuilds rebuilds;

    setUp(() => rebuilds = _Rebuilds()..install());
    tearDown(() => rebuilds.remove());

    int paints(WidgetTester tester, String key) {
      final boundary = tester.renderObject<RenderRepaintBoundary>(
          find.byKey(ValueKey<String>(key)).first);
      return boundary.debugSymmetricPaintCount +
          boundary.debugAsymmetricPaintCount;
    }

    testWidgets('idle: nothing rebuilds and no block repaints', (tester) async {
      await mount(tester);
      final lanesBefore = paints(tester, 'atl-lane-blocks');
      final outlineBefore = paints(tester, 'atl-outline-blocks');

      rebuilds
        ..reset()
        ..counting = true;
      for (var i = 0; i < 20; i++) {
        await tester.pump(const Duration(milliseconds: 16));
      }
      rebuilds
        ..counting = false
        ..remove();

      expect(paints(tester, 'atl-lane-blocks'), lanesBefore,
          reason: 'a lane block re-recorded with nothing happening to it');
      expect(paints(tester, 'atl-outline-blocks'), outlineBefore,
          reason: 'an outline block re-recorded with nothing happening to it');
      expect(rebuilds.total, 0,
          reason: 'something rebuilt itself at rest - a polling listener, or a '
              'ticker nobody stopped:\n${rebuilds.ranking()}');
    });

    testWidgets('a scrub repaints the playhead and not the lanes',
        (tester) async {
      final p = await mount(tester);
      // Twirled open, so the lanes carry rows and not only the tracks' own.
      await tester.tap(find.byKey(ValueKey<String>('atl-twirl-${p.music}')));
      await settleFrb(tester, minRounds: 2);
      // This one counts paints, and the framework refuses to end a test with a
      // foundation debug flag still set.
      rebuilds.remove();

      final lanesBefore = paints(tester, 'atl-lane-blocks');
      final headBefore = paints(tester, 'tl-playhead-layer');
      for (var frame = 1; frame <= 20; frame++) {
        p.ui.playheadFrame.value = frame;
        await tester.pump(const Duration(milliseconds: 16));
      }

      expect(paints(tester, 'atl-lane-blocks'), lanesBefore,
          reason: 'the lanes were redrawn for a playhead that moved over them');
      expect(paints(tester, 'tl-playhead-layer'), greaterThan(headBefore),
          reason: 'the playhead did not redraw, so nothing was measured');
    });

    testWidgets('a clip drag writes once, on release', (tester) async {
      rebuilds.remove(); // this one counts bridge calls, not rebuilds
      final p = await mountClips(tester);
      final before = p.top.getClips().single;

      final gesture = await tester.startGesture(onBody(tester, before));
      await tester.pump();
      calls
        ..reset()
        ..counting = true;
      for (var i = 0; i < 4; i++) {
        await gesture.moveBy(const Offset(30, 0));
        await tester.pump();
      }
      expect(calls.writes, 0,
          reason: 'a pointer move wrote to the document, so a drag is as many '
              'undo steps as it has frames:\n${calls.ranking()}');

      await gesture.up();
      await settleFrb(tester, minRounds: 3);
      calls.counting = false;
      expect(calls.writes, 1,
          reason: 'the whole drag is staged in Dart and written once, on '
              'release:\n${calls.ranking()}');
      expect(p.top.getClips().single.startFrame, greaterThan(before.startFrame),
          reason: 'and the clip actually moved');
    });
  });
}

/// A real, probeable WAV: half a second of 8 kHz mono square wave. Written
/// synchronously - an awaited async `dart:io` call in a `testWidgets` body
/// hangs the test outright.
String _toneWavFile() {
  final dir = Directory.systemTemp.createTempSync('lumit-audio-timeline');
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
