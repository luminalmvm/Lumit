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
  test('performance defaults are the shipped ones', () {
    final p = PerformanceSettings();
    // Every frame, not adaptive (K-670): a fresh install shows the picture it
    // was asked for rather than a softened one.
    expect(p.playback, PlaybackMode.everyFrame);
  });

  test('interface defaults are a no-op for existing installs', () {
    final i = InterfaceSettings();
    expect(i.uiScale, 1.0);
    expect(i.showTooltips, isTrue);
    // The Vegas pair (K-246) defaults off: the shipped behaviour is the
    // After Effects one, and a new setting must never change an editor on
    // somebody who has not asked for it.
    expect(i.retimeOpensToSpeed, isFalse);
    expect(i.videoAsSequenceLayer, isFalse);
    // The Retime row reads as a clock by default (K-287); seconds are the
    // deviation, not the shipped state.
    expect(i.retimeInSeconds, isFalse);
  });

  test('a settings file written before the Vegas pair loads as After Effects',
      () {
    final i = InterfaceSettings.fromJson(const {'ui_scale': 1.25});
    expect(i.retimeOpensToSpeed, isFalse);
    expect(i.videoAsSequenceLayer, isFalse);
    expect(i.retimeInSeconds, isFalse);
  });

  /// K-560's one migration. `ui_scale` held the whole scale; the user's own
  /// factor is now `ui_scale_user`, drawn over the ×1.1 presentation baseline.
  /// A file from before the rebase must come back the same *size* it was —
  /// which means a smaller number, exactly once, and never again.
  group('the scale rebase (K-560)', () {
    test('a stored scale from before the baseline divides by it once', () {
      final old = InterfaceSettings.fromJson(const {'ui_scale': 1.0});
      expect(old.uiScale, closeTo(1 / uiScaleBaseline, 1e-9));
      // The size on screen is what has to be unchanged, and it is.
      expect(effectiveUiScale(old.uiScale), closeTo(1.0, 1e-9));

      // Written back and read again, it does not divide a second time.
      final again = InterfaceSettings.fromJson(old.toJson());
      expect(again.uiScale, closeTo(old.uiScale, 1e-9));
    });

    test('a settings file with neither key is the shipped 100%', () {
      final fresh = InterfaceSettings.fromJson(const {});
      expect(fresh.uiScale, closeTo(1.0, 1e-9));
      expect(effectiveUiScale(fresh.uiScale), closeTo(uiScaleBaseline, 1e-9));
    });

    test('the slider reads the user factor, not what is drawn', () {
      // What the Settings row shows is `uiScale * 100` — 100% for a new user,
      // whose interface is meanwhile drawn at 110%.
      expect((InterfaceSettings().uiScale * 100).round(), 100);
      expect((InterfaceSettings.fromJson(const {'ui_scale_user': 1.25}).uiScale *
              100)
          .round(),
          125);
    });
  });

  /// The tone map button is asked for (K-314): hidden by default, and hidden
  /// for a settings file written before the field existed.
  test('the tone map button is hidden unless a settings file asks for it', () {
    expect(InterfaceSettings().showToneMap, isFalse);
    expect(InterfaceSettings.fromJson(const {'ui_scale': 1.25}).showToneMap,
        isFalse);
    final on = InterfaceSettings()..showToneMap = true;
    expect(InterfaceSettings.fromJson(on.toJson()).showToneMap, isTrue);
  });

  /// K-349. The panel is the default, so the flag is the *deviation* — same
  /// shape as the playhead and waveform preferences above.
  test('the easing editor is a panel unless a settings file asks otherwise',
      () {
    expect(InterfaceSettings().easingInPopup, isFalse);
    expect(InterfaceSettings.fromJson(const {'ui_scale': 1.25}).easingInPopup,
        isFalse,
        reason: 'a file written before the field existed gets the panel');
    final popup = InterfaceSettings()..easingInPopup = true;
    expect(InterfaceSettings.fromJson(popup.toJson()).easingInPopup, isTrue);
  });

  /// K-454. Regular is the default and Compact is the deviation, so a
  /// settings file written by a build that only ever drew the tight rows
  /// opens roomy — the mockups' room is the decision, not an opt-in.
  test('rows are roomy unless a settings file asks for Compact', () {
    expect(InterfaceSettings().compact, isFalse);
    expect(
        InterfaceSettings.fromJson(const {'ui_scale': 1.25}).compact, isFalse,
        reason: 'a file written before the field existed gets the roomy rows');
    final tight = InterfaceSettings()..compact = true;
    expect(InterfaceSettings.fromJson(tight.toJson()).compact, isTrue);
  });

  /// K-514, the owner's explicit default: the mockups draw a layer's name on
  /// every bar in the lane area, and the ruling from desktop testing is that
  /// the editor should not — the outline is already a column of exactly those
  /// names. Off unless a settings file asks, including for every file written
  /// by a build that always drew them.
  test('bars carry no layer name unless a settings file asks for one', () {
    expect(InterfaceSettings().layerNamesOnBars, isFalse);
    expect(
        InterfaceSettings.fromJson(const {'ui_scale': 1.25}).layerNamesOnBars,
        isFalse,
        reason: 'a file written before the field existed loses the labels');
    final named = InterfaceSettings()..layerNamesOnBars = true;
    expect(InterfaceSettings.fromJson(named.toJson()).layerNamesOnBars, isTrue);
  });

  /// K-448's arrangement choice, settled by K-466's drawing. Split is the
  /// default because it is what the drawing draws; the other two gather the
  /// chrome into one strip. Stored by name, so a reordered enum cannot
  /// silently rearrange somebody's Viewer.
  test("the Viewer's bars are split unless a settings file says otherwise", () {
    expect(InterfaceSettings().viewerBars, ViewerBars.split);
    expect(InterfaceSettings.fromJson(const {'ui_scale': 1.25}).viewerBars,
        ViewerBars.split,
        reason: "a file written before the field existed gets the drawing's");
    expect(
        InterfaceSettings.fromJson(const {'viewer_bars': 'nonsense'})
            .viewerBars,
        ViewerBars.split,
        reason: 'and so does a name no build of Lumit ever wrote');
    for (final bars in ViewerBars.values) {
      final chosen = InterfaceSettings()..viewerBars = bars;
      expect(chosen.toJson()['viewer_bars'], bars.name);
      expect(InterfaceSettings.fromJson(chosen.toJson()).viewerBars, bars);
    }
  });

  /// The two densities are the two the design doc's §12A.6 table lists, and
  /// Regular is the one a theme carries when nobody has chosen.
  test('the density tokens are the table, and Regular is the default', () {
    expect(LumitTheme.dark().density, DensityTokens.regular);
    expect(DensityTokens.of(false), DensityTokens.regular);
    expect(DensityTokens.of(true), DensityTokens.compact);

    expect(DensityTokens.regular.laneRow, 23);
    expect(DensityTokens.regular.secondaryRow, 19);
    expect(DensityTokens.regular.inRowPicker, 18);
    expect(DensityTokens.regular.dropdownFace, 20);
    expect(DensityTokens.regular.propertyRow, 27);

    expect(DensityTokens.compact.laneRow, 22);
    expect(DensityTokens.compact.secondaryRow, 18);
    expect(DensityTokens.compact.inRowPicker, 16);
    expect(DensityTokens.compact.dropdownFace, 18);
    expect(DensityTokens.compact.propertyRow, 26);
  });

  /// K-512, the owner's ruling from desktop testing: the Timeline's own two
  /// chrome rows are no longer plain secondary rows. Regular grew them — the
  /// row that is aimed at all day most — and states a height for the controls
  /// standing in them, so the hit targets grew with the row. **Compact keeps
  /// exactly what it drew**, which is the half of this that a test has to
  /// hold: a ruling about Regular that quietly moved Compact would be a
  /// ruling about both.
  test('the Timeline chrome grew under Regular and stood still under Compact',
      () {
    expect(DensityTokens.regular.timelineChromeRow, 24);
    expect(DensityTokens.regular.timelineHeaderRow, 23);
    expect(DensityTokens.regular.timelineChromeControl, 20);

    expect(DensityTokens.compact.timelineChromeRow, 18);
    expect(DensityTokens.compact.timelineHeaderRow, 18);
    expect(DensityTokens.compact.timelineChromeControl, isNull,
        reason: 'Compact states nothing, so every control measures itself');

    // Derived, never declared: the ruler is what the outline's two chrome
    // rows cost, which is what makes the Timeline's halves meet. Grow either
    // row and the ruler grows with it.
    expect(DensityTokens.regular.ruler, 47);
    expect(DensityTokens.compact.ruler, 36);
    for (final d in [DensityTokens.regular, DensityTokens.compact]) {
      expect(d.ruler, d.timelineChromeRow + d.timelineHeaderRow);
    }
  });

  /// The rebuild's own guard (K-465): the Settings window was taken apart and
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
      compact: true,
      viewerBars: ViewerBars.bottom,
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
    expect(back.compact, isTrue);
    expect(back.viewerBars, ViewerBars.bottom);
    // Every field is one of the above: a new one added without a line here is
    // a setting nothing checks survives the file.
    expect(all.toJson().keys.length, 17);
  });

  test('the Retime seconds preference round-trips', () {
    final i = InterfaceSettings(retimeInSeconds: true);
    expect(InterfaceSettings.fromJson(i.toJson()).retimeInSeconds, isTrue);
  });

  /// The returning playhead is the *new* default (K-254), so unlike the Vegas
  /// pair it does not defer to what a settings file leaves out — an install
  /// that predates the field adopts the new behaviour rather than being pinned
  /// to the old one by its own silence.
  test('the playhead returns on stop unless a settings file says otherwise',
      () {
    expect(InterfaceSettings().playheadStaysOnStop, isFalse);
    expect(
        InterfaceSettings.fromJson(const {'ui_scale': 1.25})
            .playheadStaysOnStop,
        isFalse);
    final on = InterfaceSettings()..playheadStaysOnStop = true;
    expect(InterfaceSettings.fromJson(on.toJson()).playheadStaysOnStop, isTrue);
  });

  test('the Vegas pair survives a settings round-trip', () {
    final i = InterfaceSettings()
      ..retimeOpensToSpeed = true
      ..videoAsSequenceLayer = true;
    final back = InterfaceSettings.fromJson(i.toJson());
    expect(back.retimeOpensToSpeed, isTrue);
    expect(back.videoAsSequenceLayer, isTrue);
  });

  test('the Vegas answer sets both preferences and answers the question', () {
    Workspace.storeOverride = _scratchStore('vegas');
    final ws = Workspace()..setEditingStyle(vegas: true);
    expect(ws.interface.retimeOpensToSpeed, isTrue);
    expect(ws.interface.videoAsSequenceLayer, isTrue);
    expect(ws.firstRunDone, isTrue);
    Workspace.storeOverride = null;
  });

  test('the After Effects answer leaves both off, and is still an answer', () {
    Workspace.storeOverride = _scratchStore('ae');
    final ws = Workspace()..setEditingStyle(vegas: false);
    expect(ws.interface.retimeOpensToSpeed, isFalse);
    expect(ws.interface.videoAsSequenceLayer, isFalse);
    // The point of the flag: answering "After Effects" must not leave the
    // screen asking again on the next launch just because nothing changed.
    expect(ws.firstRunDone, isTrue);
    Workspace.storeOverride = null;
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

  // Automatic update checks (K-296): on by default, for a fresh install and
  // for a settings file written before the setting existed alike.
  test('update checks default to on and survive a restart', () {
    expect(Workspace().autoUpdate, isTrue);
    expect(
        (Workspace()..applyJson(<String, dynamic>{'ui_scale': 1.0})).autoUpdate,
        isTrue,
        reason: 'a file that predates the setting still gets the default');

    final path = _scratchStore('auto-update');
    File(path).parent.createSync(recursive: true);
    if (File(path).existsSync()) File(path).deleteSync();
    Workspace.storeOverride = path;

    (Workspace()..load()).setAutoUpdate(false);
    expect((Workspace()..load()).autoUpdate, isFalse);
    Workspace.storeOverride = null;
  });

  // The welcome screen on launch (K-481): on by default, off survives the
  // restart it is about — a setting that forgot itself would be a poor joke.
  test('the welcome screen defaults to on and survives a restart', () {
    expect(Workspace().showWelcomeOnLaunch, isTrue);
    expect(
        (Workspace()..applyJson(<String, dynamic>{'ui_scale': 1.0}))
            .showWelcomeOnLaunch,
        isTrue,
        reason: 'a file that predates the setting still gets the default');

    final path = _scratchStore('welcome-on-launch');
    File(path).parent.createSync(recursive: true);
    if (File(path).existsSync()) File(path).deleteSync();
    Workspace.storeOverride = path;

    (Workspace()..load()).setShowWelcomeOnLaunch(false);
    expect((Workspace()..load()).showWelcomeOnLaunch, isFalse);
    (Workspace()..load()).setShowWelcomeOnLaunch(true);
    expect((Workspace()..load()).showWelcomeOnLaunch, isTrue);
    Workspace.storeOverride = null;
  });

  test('when the last update check happened is remembered', () {
    final path = _scratchStore('update-check');
    File(path).parent.createSync(recursive: true);
    if (File(path).existsSync()) File(path).deleteSync();
    Workspace.storeOverride = path;

    expect((Workspace()..load()).lastUpdateCheckMs, 0);
    (Workspace()..load()).rememberUpdateCheck(1234567);
    expect((Workspace()..load()).lastUpdateCheckMs, 1234567);
    Workspace.storeOverride = null;
  });

  test('an unknown playback name falls back to the shipped default', () {
    final p = PerformanceSettings.fromJson(const {'playback': 'warp-speed'});
    expect(p.playback, PerformanceSettings().playback);
    // A stored name this build *does* know is untouched by the default change.
    expect(
        PerformanceSettings.fromJson(const {'playback': 'adaptive'}).playback,
        PlaybackMode.adaptive);
  });

  // The bug: the budgets were live engine state with nothing behind them, so
  // they reset on every launch while every other setting survived.
  test('cache budgets survive a settings round-trip', () {
    final p = PerformanceSettings()
      ..cacheBudgetBytes = 3 * 1024 * 1024 * 1024
      ..vramBudgetBytes = 2 * 1024 * 1024 * 1024;
    final back = PerformanceSettings.fromJson(p.toJson());
    expect(back.cacheBudgetBytes, 3 * 1024 * 1024 * 1024);
    expect(back.vramBudgetBytes, 2 * 1024 * 1024 * 1024);
  });

  test('untouched budgets stay absent rather than freezing a default', () {
    final json = PerformanceSettings().toJson();
    expect(json.containsKey('cache_budget_bytes'), isFalse);
    expect(json.containsKey('vram_budget_bytes'), isFalse);
    expect(PerformanceSettings.fromJson(json).cacheBudgetBytes, isNull);
  });

  test('a nonsense budget in the file loads as absent', () {
    final p = PerformanceSettings.fromJson(const {
      'playback': 'adaptive',
      'cache_budget_bytes': 'lots',
      'vram_budget_bytes': -1,
    });
    expect(p.cacheBudgetBytes, isNull);
    expect(p.vramBudgetBytes, isNull);
  });

  test('workspace JSON round-trips appearance and settings', () {
    final ws = Workspace();
    ws.colorScheme = LumitColorScheme.gruvboxDark;
    ws.themeShape = ThemeShape.round;
    ws.accentOverride = const Color(0xff804060);
    ws.animationLevel = AnimationLevel.minimal;
    ws.performance.playback = PlaybackMode.everyFrame;
    ws.lastProjectPath = 'C:/edit/last.lum';
    ws.recompose();

    final j = ws.toJson();
    final back = Workspace()..applyJson(Map<String, dynamic>.from(j));
    expect(back.colorScheme, LumitColorScheme.gruvboxDark);
    expect(back.lastProjectPath, 'C:/edit/last.lum');
    expect(back.themeShape, ThemeShape.round);
    expect(back.animationLevel, AnimationLevel.minimal);
    expect(back.performance.playback, PlaybackMode.everyFrame);
    expect((back.accentOverride!.r * 255).round(), 0x80);
    // The rebuilt theme carries the override and the shape tokens.
    expect(back.theme.tokens, ShapeTokens.round);
    expect((back.theme.accent.r * 255).round(), 0x80);
  });

  /// **The output device is settings, not project data** (docs/09 §3.1): a
  /// sound card is a property of the machine, so the settings file carries it
  /// and hands it to the engine on the next launch. Absent means follow the
  /// machine, which is what every file written before the Audio page existed
  /// was already doing.
  test('the chosen audio output round-trips and defaults to the machine', () {
    expect(Workspace().audioDevice, isNull);
    expect(
        (Workspace()..applyJson(<String, dynamic>{'ui_scale': 1.0}))
            .audioDevice,
        isNull,
        reason: 'a file written before this field existed follows the machine');

    final ws = Workspace()..audioDevice = 'Headset Earphone (Arctis 7)';
    final back = Workspace()..applyJson(Map<String, dynamic>.from(ws.toJson()));
    expect(back.audioDevice, 'Headset Earphone (Arctis 7)');

    // An empty id is the engine's word for the system default, not a device
    // name — a hand-edited file carrying one must read as "follow the machine"
    // rather than as a device called nothing.
    expect(
        (Workspace()..applyJson(<String, dynamic>{'audio_device': ''}))
            .audioDevice,
        isNull);
  });

  /// **The autosave cadence is settings, not project data** (K-587, docs/10
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

  /// **Effect graphs use theme colour** (owner, desk test). Off by shipped
  /// default — a Levels histogram's red hump and a Curves Red tab draw in red,
  /// and only Master takes the theme's own colour — and it round-trips, so the
  /// switch is not forgotten at the next launch.
  test('the effect-graph colour preference defaults off and round-trips', () {
    expect(Workspace().themedEffectGraphs, isFalse,
        reason: 'a new setting never changes the editor on somebody who has '
            'not asked for it');
    expect(
        (Workspace()..applyJson(<String, dynamic>{'ui_scale': 1.0}))
            .themedEffectGraphs,
        isFalse,
        reason: 'and a file written before it existed reads as off');

    final ws = Workspace()..themedEffectGraphs = true;
    expect(
        (Workspace()..applyJson(Map<String, dynamic>.from(ws.toJson())))
            .themedEffectGraphs,
        isTrue);
  });

  /// **Favourites are a preference, not a view state** (owner, desk test):
  /// they outlive the panel, the workspace and the session, so they travel in
  /// this file rather than in the widget that draws the stars.
  test('starred effects and presets round-trip', () {
    final fresh = Workspace();
    expect(fresh.favouriteEffects, isEmpty);
    expect(fresh.isFavouriteEffect('blur'), isFalse);

    final ws = Workspace();
    ws.favouriteEffects.addAll(['blur', 'preset:Soft glow']);
    final back = Workspace()..applyJson(Map<String, dynamic>.from(ws.toJson()));
    expect(back.isFavouriteEffect('blur'), isTrue);
    expect(back.isFavouriteEffect('preset:Soft glow'), isTrue,
        reason: 'a preset is starred under its own prefixed key, so it cannot '
            'collide with an effect of the same name');
    expect(back.isFavouriteEffect('vignette'), isFalse);

    // A file written before favourites existed simply has none.
    expect(
        (Workspace()..applyJson(<String, dynamic>{'ui_scale': 1.0}))
            .favouriteEffects,
        isEmpty);
  });
}
