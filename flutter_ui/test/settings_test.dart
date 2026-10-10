// Settings defaults must be a no-op for existing installs, and the workspace
// JSON must round-trip.

import 'dart:io';
import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/theme.dart';

/// A settings file of this test's own, never the developer's real one — every
/// setter calls `save()`, and the store is machine state a test must not reach.
String _scratchStore(String name) =>
    '${Directory.systemTemp.path}${Platform.pathSeparator}'
    'lumit-test-$name${Platform.pathSeparator}workspace.json';

void main() {
  /// The scale rebase's one migration. `ui_scale` held the whole scale; the
  /// user's own factor is now `ui_scale_user`, drawn over the ×1.1
  /// presentation baseline. A file from before the rebase must come back the
  /// same *size* it was — which means a smaller number, exactly once, and
  /// never again.
  group('the scale rebase', () {
    test('a stored scale from before the baseline divides by it once', () {
      final old = InterfaceSettings.fromJson(const {'ui_scale': 1.0});
      expect(old.uiScale, closeTo(1 / uiScaleBaseline, 1e-9));
      // The size on screen is what has to be unchanged, and it is.
      expect(effectiveUiScale(old.uiScale), closeTo(1.0, 1e-9));

      // Written back and read again, it does not divide a second time.
      final again = InterfaceSettings.fromJson(old.toJson());
      expect(again.uiScale, closeTo(old.uiScale, 1e-9));
    });
  });

  /// The rebuild's own guard: the Settings window was taken apart and
  /// put back to a new drawing, and the one thing that must not have happened
  /// is a setting quietly going missing. Every field of the interface settings
  /// is moved off its default here and read back, so a field dropped from the
  /// form — or from `toJson` — fails this rather than being noticed by a user
  /// whose preference stopped surviving a restart.
  test('every interface setting survives the file', () {
    final all = InterfaceSettings(
      language: 'de',
      uiScale: 1.25,
      showTooltips: false,
      transformInEffectControls: true,
      retimeOpensToSpeed: true,
      retimeInSeconds: true,
      videoAsSequenceLayer: true,
      playheadStaysOnStop: true,
      pasteLayersAtOriginalTime: true,
      multiwaveWaveforms: false,
      waveformsFromBottom: true,
      showToneMap: true,
      easingInPopup: true,
      layerNamesOnBars: true,
      projectThumbnails: true,
      timelineMinimap: true,
      compact: true,
      viewerBars: ViewerBars.bottom,
      toolBarPosition: ToolBarPosition.left,
      rangeSliders: false,
      rightClickOpensNodeSearch: false,
      tabOpensNodeSearch: false,
      shiftAOpensNodeSearch: false,
      room: LanternRoom.night,
      labelCase: LabelCase.lower,
    );
    final back = InterfaceSettings.fromJson(all.toJson());
    expect(back.language, 'de');
    expect(back.uiScale, 1.25);
    expect(back.showTooltips, isFalse);
    expect(back.transformInEffectControls, isTrue);
    expect(back.retimeOpensToSpeed, isTrue);
    expect(back.retimeInSeconds, isTrue);
    expect(back.videoAsSequenceLayer, isTrue);
    expect(back.playheadStaysOnStop, isTrue);
    expect(back.pasteLayersAtOriginalTime, isTrue);
    expect(back.multiwaveWaveforms, isFalse);
    expect(back.waveformsFromBottom, isTrue);
    expect(back.showToneMap, isTrue);
    expect(back.easingInPopup, isTrue);
    expect(back.layerNamesOnBars, isTrue);
    expect(back.projectThumbnails, isTrue);
    expect(back.timelineMinimap, isTrue);
    expect(back.compact, isTrue);
    expect(back.viewerBars, ViewerBars.bottom);
    expect(back.toolBarPosition, ToolBarPosition.left);
    expect(back.rangeSliders, isFalse);
    expect(back.rightClickOpensNodeSearch, isFalse);
    expect(back.tabOpensNodeSearch, isFalse);
    expect(back.shiftAOpensNodeSearch, isFalse);
    expect(back.room, LanternRoom.night);
    expect(back.labelCase, LabelCase.lower);
    // Every field is one of the above: a new one added without a line here is
    // a setting nothing checks survives the file.
    expect(all.toJson().keys.length, 28);
  });

  // The whole first-run rule in two lines: no file means ask, a file means
  // do not. Anything else — a `Workspace` built by a test, a corrupt file —
  // counts as "do not", or the screen appears where it has no business.
  test('only a missing settings file counts as a first run', () {
    expect(Workspace().firstRunDone, isTrue,
        reason: 'a Workspace built directly is not a first run');

    final missing = _scratchStore('missing');
    File(missing).parent.createSync(recursive: true);
    if (File(missing).existsSync()) File(missing).deleteSync();
    Workspace.storeOverride = missing;
    expect((Workspace()..load()).firstRunDone, isFalse);

    final corrupt = _scratchStore('corrupt');
    File(corrupt).parent.createSync(recursive: true);
    File(corrupt).writeAsStringSync('{ this is not json');
    Workspace.storeOverride = corrupt;
    expect((Workspace()..load()).firstRunDone, isTrue,
        reason: 'a corrupt file belongs to somebody who already uses Lumit');

    Workspace.storeOverride = null;
  });

  test('an answered first run survives a restart', () {
    final path = _scratchStore('restart');
    File(path).parent.createSync(recursive: true);
    if (File(path).existsSync()) File(path).deleteSync();
    Workspace.storeOverride = path;

    final first = Workspace()..load();
    expect(first.firstRunDone, isFalse);
    first.setEditingStyle(vegas: true);

    final second = Workspace()..load();
    expect(second.firstRunDone, isTrue);
    expect(second.interface.retimeOpensToSpeed, isTrue);
    Workspace.storeOverride = null;
  });

  test('adaptive chosen since the reset survives the next launch', () {
    final ws = Workspace()..performance.playback = PlaybackMode.adaptive;
    final back = Workspace()..applyJson(Map<String, dynamic>.from(ws.toJson()));
    expect(back.performance.playback, PlaybackMode.adaptive);
  });

  test('workspace JSON round-trips appearance and settings', () {
    final ws = Workspace();
    ws.colorScheme = LumitColorScheme.gruvboxDark;
    ws.themeShape = ThemeShape.lantern;
    ws.accentOverride = const Color(0xff804060);
    ws.animationLevel = AnimationLevel.minimal;
    ws.performance.playback = PlaybackMode.everyFrame;
    ws.lastProjectPath = 'C:/edit/last.lum';
    ws.shareName = 'Ada';
    ws.shareHosted['a-project'] = '47856/00ff';
    ws.shareOutside = true;
    ws.recompose();

    final j = ws.toJson();
    final back = Workspace()..applyJson(Map<String, dynamic>.from(j));
    expect(back.colorScheme, LumitColorScheme.gruvboxDark);
    expect(back.lastProjectPath, 'C:/edit/last.lum');
    expect(back.shareName, 'Ada');
    expect(back.shareHosted, {'a-project': '47856/00ff'});
    expect(back.shareOutside, isTrue);
    expect(back.themeShape, ThemeShape.lantern);
    expect(back.animationLevel, AnimationLevel.minimal);
    expect(back.performance.playback, PlaybackMode.everyFrame);
    expect((back.accentOverride!.r * 255).round(), 0x80);
    // The rebuilt theme carries the override and the shape tokens.
    expect(back.theme.tokens, ShapeTokens.lantern);
    expect((back.theme.accent.r * 255).round(), 0x80);
  });

  /// **The autosave cadence is settings, not project data** (docs/10
  /// §4): how often this machine copies your work is a property of the machine.
  /// Zero minutes is off and must survive the round trip as zero — a file that
  /// read it back as the default would turn autosave on again behind the user.
  test('the autosave cadence round-trips, including off', () {
    expect(Workspace().autosaveMinutes, 5);
    expect(Workspace().autosaveKeep, 5);
    expect(
        (Workspace()..applyJson(<String, dynamic>{'ui_scale': 1.0}))
            .autosaveMinutes,
        5,
        reason: 'a file written before this field existed gets the default');

    final ws = Workspace()..setAutosave(0, 12);
    final back = Workspace()..applyJson(Map<String, dynamic>.from(ws.toJson()));
    expect(back.autosaveMinutes, 0, reason: 'off stays off');
    expect(back.autosaveKeep, 12);

    // A hand-edited file cannot ask for a negative interval or for no copies
    // at all: one is meaningless and the other is a rotation with nothing in
    // it. Both are clamped rather than refused.
    final edited = Workspace()
      ..applyJson(<String, dynamic>{'autosave_minutes': -3, 'autosave_keep': 0});
    expect(edited.autosaveMinutes, 0);
    expect(edited.autosaveKeep, 1);
  });
}
