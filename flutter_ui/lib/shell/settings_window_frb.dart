// The Settings window, on the flutter_rust_bridge API.
//
// **The shape (K-465, superseding K-193's five pages).** The approved drawing
// frames it as a window 760×520: a kicker title strip carrying a search field
// and a close mark, a 160px sidebar of pages down the left, the page itself on
// the right, and a footer saying that changes apply immediately with Reset page
// and Close at its far end. A page is a stack of *sections* — a kicker, a rule
// above it, and rows under it — and a row is a label in a fixed 190px column
// with its control beside it. There are no cards and no help sentences: the
// drawing has room for neither, and what a setting does is said by its name.
//
// **The pages are the drawing's.** General, Appearance, Timeline, Viewer,
// Preview and cache, Shortcuts. The drawing lists three more — Audio, Autosave,
// Export — and they are not here, because there is nothing to put on them: the
// engine has no autosave interval, no audio device and no export defaults to
// offer yet, and an empty page is a promise the window cannot keep. They arrive
// with the settings they would hold.
//
// **What lives where.** Appearance is Dart's own: the theme is the frontend's
// and the engine has no opinion about it. Timeline and Viewer are working
// preferences, persisted in the workspace file. Preview and cache is mostly a
// readout of the engine with a button — the cache budgets are the numbers here
// that change engine behaviour, and even those are not part of the document, so
// nothing in this window is undoable.
//
// **Shortcuts (K-199)** is the one page that is not a settings form: it is a
// table of every shortcut, grouped by where it is live, with the action on the
// left and its chord on the right — click a chord and press the keys you want.
// It edits the engine's keymap, not a copy, so what the table shows is what the
// keyboard does. The title strip's search is its search too: on every other
// page the field hides the rows whose names do not match, and on this one it
// asks the engine the same question.

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart' show kDebugMode;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart';
import 'package:lumit_flutter/src/rust/api/system.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/file_dialogs.dart';
import '../state/keymap.dart';
import '../state/settings.dart';
import '../state/updates.dart';
import '../state/workspace.dart';
import '../theme/custom_theme.dart';
import '../theme/theme.dart';
import '../theme/theme_file.dart';
import '../widgets/controls.dart';
import '../widgets/theme_swatches.dart';
import 'about_window_frb.dart';
import 'cache_confirm_frb.dart';
import 'menu_bar_frb.dart';
import 'settings_rows.dart';
import 'theme_editor_frb.dart';
import 'theme_name_dialog.dart';
import 'update_dialog_frb.dart';

/// The smallest budget worth setting, in MiB. Below this the cache holds a
/// frame or two and costs more in bookkeeping than it saves.
const double _minBudgetMib = 64;

/// The ceiling when the machine will not say how much it has (K-194): every
/// platform but Windows, so far. Generous rather than clever — the engine
/// clamps to what it can actually allocate either way.
const double _unknownMemoryMib = 16384;

// ---- the drawing's measurements ---------------------------------------------
//
// Every one of these is read off the approved drawing's own computed styles,
// not chosen: `settings_metrics_test` pins them, and a value that disagrees
// with the drawing is a defect (§12A.6).

/// The size the window opens at: the drawing's own frame (K-465). The corner
/// grip takes it from here, and where it is left is remembered (K-242).
const Size settingsWindowSize = Size(760, 520);

/// Below this the sidebar and the widest setting row stop fitting side by side.
const Size settingsMinSize = Size(560, 380);

/// The title strip, and the footer under the page. §12A.4's dialog title strip
/// is 30; the footer is 8 above a 26px button and 8 below it, over a hairline.
const double settingsTitleStrip = 30;
const double settingsFooterHeight = 43;
const double settingsFooterButton = 26;

/// The sidebar, and one page's entry in it.
const double settingsSidebarWidth = 160;
const double settingsNavRow = 24;

/// The tick down the left edge of the page being shown, and the inset the
/// label gives up to it.
const double settingsNavTick = 2;

/// The search well in the title strip.
const double settingsSearchWidth = 174;
const double settingsSearchHeight = 20;

/// The close mark, at the size the drawing renders it (K-456).
const double settingsCloseGlyph = 12;

/// The two dropdown widths the drawing uses: a wide face for a phrase, a
/// narrow one for a single word.
const double _ddWide = 180;
const double _ddNarrow = 120;

/// The scale row's track and its well.
const double _scaleTrack = 160;
const double _scaleWell = 70;

/// One accent swatch.
const double _swatch = 14;

/// One page in the sidebar.
enum SettingsPage {
  general,
  appearance,
  timeline,
  viewer,
  previewAndCache,
  shortcuts;

  /// The name in the sidebar. A getter rather than a constructor argument
  /// because an enum constant is built once, at start-up, and the language can
  /// change after that.
  String get label => switch (this) {
        SettingsPage.general => l10n.settingsPageGeneral,
        SettingsPage.appearance => l10n.settingsPageAppearance,
        SettingsPage.timeline => l10n.panelTimeline,
        SettingsPage.viewer => l10n.panelViewer,
        SettingsPage.previewAndCache => l10n.settingsPagePreviewAndCache,
        SettingsPage.shortcuts => l10n.settingsPageShortcuts,
      };
}

Future<void> showSettingsWindowFrb(BuildContext context) =>
    showLumitModal<void>(
      context: context,
      id: 'settings',
      initialSize: settingsWindowSize,
      minSize: settingsMinSize,
      builder: (close) => _SettingsWindow(onClose: () => close(null)),
    );

class _SettingsWindow extends StatefulWidget {
  final VoidCallback onClose;
  const _SettingsWindow({required this.onClose});

  @override
  State<_SettingsWindow> createState() => _SettingsWindowState();
}

/// Whether a cache-location choice is this project's or the application's
/// (docs/07 §15). Interface-only: the engine stores the two in different places —
/// one in the document, one in the settings file — and this is the control that
/// says which the user means.
enum CacheScope { everywhere, thisProject }

class _SettingsWindowState extends State<_SettingsWindow> {
  SettingsPage _page = SettingsPage.general;

  /// What the title strip's search field holds. Empty shows everything.
  String _query = '';

  /// The Preview and cache page's engine readouts, captured in one sweep so
  /// `build()` never crosses the bridge (the standing rebuild-path rule —
  /// these were ~8 calls per rebuild, re-triggered by every switch
  /// `setState` anywhere in the window). Refreshed on page entry, once a
  /// second while the page is up, and by any control that changes what
  /// they report.
  ({
    BridgeCacheStats ram,
    BridgeVramCacheStats vram,
    BridgeDiskCacheStats disk,
    BridgePlaybackTier tier,
    BridgeMemoryReport? memory,
    BridgeProjectCacheLocation? own,
  })? _perf;
  Timer? _perfTimer;

  void _pollPerf() => _perf = (
        ram: cacheStats(),
        vram: vramCacheStats(),
        disk: diskCacheStats(),
        tier: playbackTier(),
        // Only read when it is going to be drawn: the report is a debug-build
        // instrument, and a release build should not be making the call at all.
        memory: kDebugMode ? memoryReport() : null,
        own: _project(context)?.cacheLocation(),
      );

  /// Front [page]. Preview and cache polls on entry and keeps a slow tick while
  /// it is up, so its readouts stay live without a bridge call in `build()`.
  void _showPage(SettingsPage page) {
    setState(() {
      _page = page;
      if (page == SettingsPage.previewAndCache) _pollPerf();
      // The keymap filters engine-side, so the shared search has to be handed
      // over when the page it belongs to comes forward.
      if (page == SettingsPage.shortcuts) _keymapState()?.query = _query;
    });
    if (page == SettingsPage.previewAndCache) {
      _perfTimer ??= Timer.periodic(
          const Duration(seconds: 1), (_) => setState(_pollPerf));
    } else {
      _perfTimer?.cancel();
      _perfTimer = null;
    }
  }

  KeymapState? _keymapState() =>
      Provider.of<LumitUiState>(context, listen: false).keymap;

  /// An engine-changing control on the Preview and cache page: apply, then read
  /// the page's numbers back in the same frame so the rows show the answer.
  void _perfEdit(VoidCallback apply) => setState(() {
        apply();
        _pollPerf();
      });

  // ---- the search ----------------------------------------------------------

  /// Whether a row called [title] survives the search. Rows are matched on
  /// their own names only: a search that also read the section kickers would
  /// keep every row under "Theme" for the word *theme*, which is a page, not a
  /// result.
  bool _matches(String title) =>
      _query.isEmpty || title.toLowerCase().contains(_query.toLowerCase());

  /// A row, or nothing when the search has hidden it.
  Widget? _row(LumitTheme t, String title, Widget control,
          {String description = ''}) =>
      _matches(title) ? settingsRow(t, title, description, control) : null;

  /// A switch row — the drawing's pill, not a checkbox: the same answer, in the
  /// shape the Settings drawing gives it.
  Widget? _flag(
    LumitTheme t,
    String key,
    String title, {
    required bool value,
    required ValueChanged<bool> set,
  }) =>
      _row(
        t,
        title,
        HouseToggle(
          key: ValueKey<String>(key),
          value: value,
          onChanged: (on) => setState(() => set(on)),
        ),
      );

  /// A dropdown at the drawing's height, in one of its two widths.
  Widget _dropdown<T>({
    required String key,
    required T value,
    required List<T> options,
    required String Function(T) label,
    required ValueChanged<T> onChanged,
    String? Function(T)? group,
    double width = _ddNarrow,
  }) =>
      SizedBox(
        width: width,
        height: settingsControlHeight,
        child: BareDropdown<T>(
          key: ValueKey<String>(key),
          value: value,
          options: options,
          label: label,
          group: group,
          onChanged: onChanged,
        ),
      );

  /// A page: its sections, in order, with the ones the search emptied dropped
  /// and the first survivor left without a rule above it.
  List<Widget> _sections(
    LumitTheme t,
    List<(String, List<Widget?>)> sections,
  ) {
    final out = <Widget>[];
    for (final (title, rows) in sections) {
      final kept = rows.whereType<Widget>().toList();
      if (kept.isEmpty) continue;
      out.add(settingsSection(t, title, kept, first: out.isEmpty));
    }
    return out;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = Provider.of<LumitUiState>(context);

    // No width or height of its own: the window frame around it is what has the
    // size, so the corner grip can change it (K-242).
    //
    // Not a `FloatSurface`: that is the *menu* surface — `surface3` with 6px of
    // padding round its rows — and the drawing gives this window the page's own
    // `surface1` right out to a hairline edge. The edge is a foreground
    // decoration so that it is painted *over* the outermost pixel rather than
    // insetting everything by one: the drawing's 760 is the room inside the
    // frame, not the room inside the frame less its border.
    return Container(
      decoration: BoxDecoration(
        color: t.surface1,
        borderRadius: BorderRadius.circular(t.tokens.floatRadius),
        boxShadow: t.floatShadow,
      ),
      foregroundDecoration: BoxDecoration(
        borderRadius: BorderRadius.circular(t.tokens.floatRadius),
        border: Border.all(color: t.hairline),
      ),
      child: ClipRRect(
        borderRadius: BorderRadius.circular(t.tokens.floatRadius),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _titleStrip(t),
            Expanded(
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  SizedBox(
                    width: settingsSidebarWidth,
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        Expanded(child: _sidebar(t)),
                        Container(width: 1, color: t.hairline),
                      ],
                    ),
                  ),
                  Expanded(child: _pageBody(t, ui)),
                ],
              ),
            ),
            _footer(t, ui),
          ],
        ),
      ),
    );
  }

  // ---- the frame -----------------------------------------------------------

  Widget _titleStrip(LumitTheme t) => Container(
        key: const ValueKey('settings-title-strip'),
        height: settingsTitleStrip + 1,
        decoration: BoxDecoration(
          color: t.surface2,
          border: Border(bottom: BorderSide(color: t.hairline)),
        ),
        padding: const EdgeInsets.symmetric(horizontal: 14),
        child: Row(
          children: [
            Text(l10n.settingsTitle.toUpperCase(), style: t.kickerOn),
            const Spacer(),
            SizedBox(
              width: settingsSearchWidth,
              height: settingsSearchHeight,
              child: HouseTextField(
                key: const ValueKey('settings-search'),
                controller: _searchController(),
                width: double.infinity,
                // The well fills the row rather than floating in it: the
                // drawing renders it exactly 20 tall, and the default 3px above
                // and below would burst that.
                padding: const EdgeInsets.symmetric(horizontal: 6),
                // `surface2`, the strip's own ground: this well sits on the
                // title strip rather than in a panel, and the drawing computes
                // it the same shade rather than a recess.
                fill: t.surface2,
                hint: l10n.searchSettings,
              ),
            ),
            const SizedBox(width: 12),
            _closeMark(t),
          ],
        ),
      );

  Widget _closeMark(LumitTheme t) => LumitTooltip(
        message: l10n.close,
        child: GestureDetector(
          key: const ValueKey('settings-close'),
          behavior: HitTestBehavior.opaque,
          onTap: widget.onClose,
          child: SizedBox(
            width: settingsCloseGlyph + 8,
            height: settingsTitleStrip,
            child: Center(
              child: glyph.LumitIcon(
                LumitIcons.close,
                size: settingsCloseGlyph,
                colour: t.textMuted,
                semanticLabel: l10n.close,
              ),
            ),
          ),
        ),
      );

  Widget _sidebar(LumitTheme t) => Padding(
        padding: const EdgeInsets.only(top: 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (final page in SettingsPage.values)
              GestureDetector(
                key: ValueKey<String>('settings-page-${page.name}'),
                behavior: HitTestBehavior.opaque,
                onTap: () => _showPage(page),
                child: Container(
                  height: settingsNavRow,
                  alignment: Alignment.centerLeft,
                  padding: EdgeInsets.only(
                      left: _page == page ? 14 - settingsNavTick : 14,
                      right: 14),
                  decoration: BoxDecoration(
                    color: _page == page ? t.surface2 : null,
                    // The tick, not a fill: the page in force is marked by an
                    // accent edge down its left, which is the one job §3.1
                    // leaves the accent on a list of names.
                    border: _page == page
                        ? Border(
                            left: BorderSide(
                                color: t.accent, width: settingsNavTick))
                        : null,
                  ),
                  child: Text(page.label,
                      style: _page == page ? t.bodyPrimary : t.body),
                ),
              ),
          ],
        ),
      );

  Widget _footer(LumitTheme t, LumitUiState ui) => Container(
        key: const ValueKey('settings-footer'),
        height: settingsFooterHeight,
        decoration: BoxDecoration(
          color: t.surface2,
          border: Border(top: BorderSide(color: t.hairline)),
        ),
        padding: const EdgeInsets.symmetric(horizontal: 14),
        child: Row(
          children: [
            Text(
              l10n.settingsChangesApplyImmediately,
              // A kicker's face at a kicker's size, but neither shouted nor
              // tracked as wide: the drawing sets this line in sentence case at
              // half the tracking, because it is a sentence and not a label.
              style: t.kicker.copyWith(letterSpacing: 0.54),
            ),
            const Spacer(),
            SizedBox(
              height: settingsFooterButton,
              child: HouseButton(
                key: const ValueKey('settings-reset-page'),
                padding: const EdgeInsets.symmetric(horizontal: 12),
                onPressed: () => _resetPage(ui),
                child: Text(l10n.settingsResetPage),
              ),
            ),
            const SizedBox(width: 12),
            SizedBox(
              height: settingsFooterButton,
              child: HouseButton(
                key: const ValueKey('settings-close-button'),
                padding: const EdgeInsets.symmetric(horizontal: 12),
                onPressed: widget.onClose,
                child: Text(l10n.close),
              ),
            ),
          ],
        ),
      );

  Widget _pageBody(LumitTheme t, LumitUiState ui) {
    final sections = switch (_page) {
      SettingsPage.general => _general(t, ui),
      SettingsPage.appearance => _appearance(t, ui),
      SettingsPage.timeline => _timeline(t, ui),
      SettingsPage.viewer => _viewer(t, ui),
      SettingsPage.previewAndCache => _performance(t, ui),
      SettingsPage.shortcuts => _keymap(t, ui),
    };
    return RawScrollbar(
      controller: _scroll,
      thumbVisibility: true,
      trackVisibility: true,
      thickness: 6,
      radius: const Radius.circular(3),
      thumbColor: t.surface4,
      trackColor: t.surface2,
      trackRadius: const Radius.circular(3),
      padding: const EdgeInsets.fromLTRB(0, 8, 2, 8),
      child: ListView(
        key: ValueKey<String>('settings-body-${_page.name}'),
        controller: _scroll,
        padding: EdgeInsets.zero,
        children: [
          ...sections,
          if (sections.isEmpty)
            Padding(
              padding: const EdgeInsets.all(16),
              child: Text(l10n.settingsNoMatches,
                  key: const ValueKey('settings-no-matches'),
                  style: t.small.copyWith(color: t.textMuted)),
            ),
          const SizedBox(height: 8),
        ],
      ),
    );
  }

  /// Put the page back the way it ships. What "the way it ships" means is
  /// knowable in Dart for every page but Preview and cache, where the cache
  /// budgets are the engine's own defaults and it offers no way to ask for
  /// them back — so that page resets the two things it can, the playback mode
  /// and the quality tier, and leaves the budgets where the user put them.
  void _resetPage(LumitUiState ui) {
    final workspace = ui.workspace;
    final settings = workspace.interface;
    final shipped = InterfaceSettings();
    switch (_page) {
      case SettingsPage.general:
        workspace.setAutoUpdate(true);
        ui.setLanguage(null);
      case SettingsPage.appearance:
        workspace.setScheme(LumitColorScheme.dark);
        workspace.setAccent(null);
        ui.setShape(ThemeShape.sharp);
        workspace.setAnimationLevel(AnimationLevel.all);
        workspace.setThemedScopes(false);
        workspace.setThemedViewerSurround(false);
        settings.uiScale = shipped.uiScale;
        settings.showTooltips = shipped.showTooltips;
        settings.multiwaveWaveforms = shipped.multiwaveWaveforms;
        settings.waveformsFromBottom = shipped.waveformsFromBottom;
        settings.compact = shipped.compact;
        workspace.recompose();
        workspace.save();
      case SettingsPage.timeline:
        settings.retimeOpensToSpeed = shipped.retimeOpensToSpeed;
        settings.retimeInSeconds = shipped.retimeInSeconds;
        settings.videoAsSequenceLayer = shipped.videoAsSequenceLayer;
        settings.pasteLayersAtOriginalTime = shipped.pasteLayersAtOriginalTime;
        settings.playheadStaysOnStop = shipped.playheadStaysOnStop;
        settings.transformInEffectControls = shipped.transformInEffectControls;
        settings.easingInPopup = shipped.easingInPopup;
        workspace.settingsChanged();
      case SettingsPage.viewer:
        workspace.setSmoothZoomedViewer(false);
        settings.showToneMap = shipped.showToneMap;
        workspace.settingsChanged();
        ui.pushViewerLook();
      case SettingsPage.previewAndCache:
        _perfEdit(() {
          workspace.performance.playback = PlaybackMode.adaptive;
          workspace.settingsChanged();
          resetRealtime();
        });
      case SettingsPage.shortcuts:
        unawaited(_resetKeymap(ui.keymap));
    }
    setState(() {});
  }

  Future<void> _resetKeymap(KeymapState km) async {
    await km.loadPreset(BridgeKeymapPreset.lumit);
    if (mounted) setState(() {});
  }

  // ---- the pages -----------------------------------------------------------

  List<Widget> _general(LumitTheme t, LumitUiState ui) => _sections(t, [
        (
          l10n.settingsGroupDisplay,
          [
            _row(
              t,
              l10n.settingsLanguage,
              _dropdown<String?>(
                key: 'settings-language',
                // Null first: following the machine is the default, and the one
                // choice that is not a language in the list.
                value: ui.workspace.interface.language,
                options: [null, ...languageNames.keys],
                width: _ddWide,
                // Each language names itself, so this list reads the same
                // whichever language Lumit is currently in — somebody who picked
                // one they cannot read can still find their way back.
                label: (tag) => tag == null
                    ? l10n.languageFollowSystem
                    : languageNames[tag]!,
                onChanged: (tag) => setState(() => ui.setLanguage(tag)),
              ),
            ),
          ],
        ),
        (
          l10n.settingsGroupWorkspace,
          [
            // Off means Lumit opens straight into the shell (K-481); the
            // Viewer offers the same three ways to start until something is
            // displayed, so nothing is hidden by turning this off.
            _flag(t, 'settings-welcome-on-launch', l10n.settingsWelcomeOnLaunch,
                value: ui.workspace.showWelcomeOnLaunch,
                set: ui.workspace.setShowWelcomeOnLaunch),
            _row(
              t,
              l10n.settingsPanelLayout,
              HouseButton(
                key: const ValueKey('settings-reset-workspace'),
                small: true,
                onPressed: () => setState(ui.resetLayout),
                child: Text(l10n.menuResetWorkspace, style: t.small),
              ),
            ),
          ],
        ),
        // The same updater the Help menu drives, seen from the other side
        // (K-296): one service, two views, so they can never disagree about
        // whether a check is running or an update is waiting.
        (
          l10n.settingsGroupUpdates,
          [
            _flag(t, 'settings-auto-update', l10n.settingsAutomaticUpdates,
                value: ui.workspace.autoUpdate,
                set: ui.workspace.setAutoUpdate),
            // The whole row watches the service, not just its button: the line
            // under it is the part that says what was found, and a stale
            // sentence beside a live button would be worse than either alone.
            if (_matches(l10n.settingsThisVersion))
              ListenableBuilder(
                listenable: ui.updates,
                builder: (context, _) => settingsRow(
                  t,
                  l10n.settingsThisVersion,
                  _updateStatusLine(ui),
                  HouseButton(
                    key: const ValueKey('settings-check-updates'),
                    small: true,
                    onPressed: ui.updates.busy
                        ? null
                        : () => pressUpdateRow(
                              context,
                              updates: ui.updates,
                              notice: context.read<LumitState>().postNotice,
                              projectIsDirty: () =>
                                  context
                                      .read<LumitState>()
                                      .project
                                      ?.isDirty() ??
                                  false,
                              saveProject: () => saveProjectFrb(
                                  context.read<LumitState>(), ui),
                            ),
                    child: Text(ui.updates.menuLabel, style: t.small),
                  ),
                ),
              ),
          ],
        ),
      ]);

  /// What this build is, read over the bridge once: the row rebuilds with
  /// every update-service notification, and the installed version cannot
  /// change under a running process.
  late final String _installed = lumitProductVersion();

  /// The line under "This version": what is installed, and what the last check
  /// made of it. Rebuilt with the row, so it follows the service too.
  String _updateStatusLine(LumitUiState ui) => switch (ui.updates.stage) {
        UpdateStage.upToDate => l10n.updateUpToDate(_installed),
        UpdateStage.available =>
          l10n.updateAvailable(_installed, '${ui.updates.release?.version}'),
        UpdateStage.ready =>
          l10n.updateReady(_installed, '${ui.updates.release?.version}'),
        UpdateStage.failed =>
          '$_installed. ${ui.updates.failure ?? l10n.updateCheckDidNotFinish}',
        _ => _installed,
      };

  List<Widget> _appearance(LumitTheme t, LumitUiState ui) {
    final settings = ui.workspace.interface;
    return _sections(t, [
      (
        l10n.settingsGroupTheme,
        [
          _row(
            t,
            l10n.settingsColourScheme,
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                _dropdown<ThemeChoice>(
                  key: 'settings-scheme',
                  value: ui.workspace.themeChoice,
                  options: ui.workspace.themeChoices,
                  width: _ddWide,
                  label: (c) => c.label,
                  // Dark, Light, then the user's own (K-202): seven built-ins
                  // and a growing list of custom themes is a long flat menu,
                  // and light/dark is the first thing anyone is choosing by.
                  group: (c) => c.group,
                  onChanged: (c) => setState(() => ui.workspace.choose(c)),
                ),
                const SizedBox(width: 8),
                // What the selection actually looks like, beside its name
                // (K-298). The drawing leaves the rest of the row empty and
                // this is what it is for.
                ThemeSwatchStrip(
                  key: const ValueKey('settings-theme-swatches'),
                  theme: ui.workspace.theme,
                ),
              ],
            ),
          ),
          // Scheme, then the two rows about *your* theme, then the two dials
          // that tune whichever one is in force. The order is the order the
          // work happens in: pick a scheme, make it yours, then adjust it —
          // accent and shape used to sit above the rows that create the thing
          // they adjust.
          _row(
            t,
            l10n.settingsCustomColours,
            HouseButton(
              key: const ValueKey('settings-customise'),
              small: true,
              onPressed: () async {
                await showThemeEditorFrb(context, ui);
                if (mounted) setState(() {});
              },
              child: Text(l10n.customiseEllipsis, style: t.small),
            ),
            description: ui.workspace.customThemeName == null
                ? ''
                : l10n.settingsHelpEditingTheme(
                    '${ui.workspace.customThemeName}'),
          ),
          _row(t, l10n.settingsYourThemes, _themeShelf(t, ui),
              description: _themeMessage ?? ''),
          _row(t, l10n.settingsAccent, _accentSwatches(t, ui)),
          _row(t, l10n.settingsShape, _shapeChips(t, ui)),
        ],
      ),
      (
        l10n.settingsPageInterface,
        [
          _row(t, l10n.settingsScale, _scaleRow(t, ui)),
          // On or off, and nothing in between: a tooltip is a name, never a
          // lesson (K-440, docs/07 §13.2), so there is no longer form to
          // choose between. The switch is the whole setting, and off means
          // no tooltip anywhere — `LumitTooltip` reads it from the theme
          // scope and hands back the bare control.
          _flag(t, 'settings-tooltips', l10n.settingsTooltips,
              value: settings.showTooltips, set: (on) {
            settings.showTooltips = on;
            ui.workspace.settingsChanged();
          }),
          _row(
            t,
            l10n.settingsMotion,
            _dropdown<AnimationLevel>(
              key: 'settings-animation',
              value: ui.workspace.animationLevel,
              options: AnimationLevel.values,
              label: (a) => switch (a) {
                AnimationLevel.all => l10n.motionFull,
                AnimationLevel.minimal => l10n.motionMinimal,
                AnimationLevel.none => l10n.none,
              },
              onChanged: (a) =>
                  setState(() => ui.workspace.setAnimationLevel(a)),
            ),
          ),
          // `recompose`, not `settingsChanged`: density lives on the built
          // theme (K-454), so the theme has to be rebuilt before anything is
          // told to redraw. `recompose` notifies; `save` is still ours to call.
          _flag(t, 'settings-compact', l10n.settingsCompact,
              value: settings.compact, set: (on) {
            settings.compact = on;
            ui.workspace.recompose();
            ui.workspace.save();
          }),
        ],
      ),
      (
        l10n.settingsGroupViewer,
        [
          _flag(t, 'settings-themed-scopes', l10n.settingsScopesUseThemeColour,
              value: ui.workspace.themedScopes,
              set: ui.workspace.setThemedScopes),
          _row(
            t,
            l10n.settingsSurround,
            _dropdown<bool>(
              key: 'settings-themed-surround',
              value: ui.workspace.themedViewerSurround,
              options: const [false, true],
              label: (themed) =>
                  themed ? l10n.surroundThemeColour : l10n.surroundNeutral,
              onChanged: (themed) =>
                  setState(() => ui.workspace.setThemedViewerSurround(themed)),
            ),
          ),
          // How the Viewer's chrome is arranged round the picture (K-448's
          // choice, K-466's drawing). Appearance rather than the Viewer page:
          // the Viewer page is about the *image*, and this is about where the
          // chrome round it sits.
          _row(
            t,
            l10n.settingsViewerBars,
            _dropdown<ViewerBars>(
              key: 'settings-viewer-bars',
              value: settings.viewerBars,
              options: ViewerBars.values,
              label: (bars) => switch (bars) {
                ViewerBars.split => l10n.viewerBarsSplit,
                ViewerBars.top => l10n.viewerBarsTop,
                ViewerBars.bottom => l10n.viewerBarsBottom,
              },
              width: _ddWide,
              onChanged: (bars) => setState(() {
                settings.viewerBars = bars;
                ui.workspace.settingsChanged();
              }),
            ),
          ),
        ],
      ),
      (
        l10n.settingsGroupWaveforms,
        [
          _row(
            t,
            l10n.settingsStyle,
            _dropdown<bool>(
              key: 'settings-multiwave',
              value: settings.multiwaveWaveforms,
              options: const [true, false],
              label: (stack) =>
                  stack ? l10n.waveformFrequency : l10n.waveformPlain,
              onChanged: (stack) => setState(() {
                settings.multiwaveWaveforms = stack;
                ui.workspace.settingsChanged();
              }),
            ),
          ),
          _row(
            t,
            l10n.settingsAnchor,
            _dropdown<bool>(
              key: 'settings-waveform-from-bottom',
              value: settings.waveformsFromBottom,
              options: const [false, true],
              label: (bottom) =>
                  bottom ? l10n.waveformBottom : l10n.waveformCentre,
              onChanged: (bottom) => setState(() {
                settings.waveformsFromBottom = bottom;
                ui.workspace.settingsChanged();
              }),
            ),
          ),
        ],
      ),
    ]);
  }

  /// Sharp or Round, as the drawing draws it: two kicker chips side by side,
  /// the one in force outlined. Not an accent fill — §3.1 spends the accent on
  /// the tick beside the page name, and a second accent in the same window
  /// would make neither of them mean anything.
  Widget _shapeChips(LumitTheme t, LumitUiState ui) => Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final shape in ThemeShape.values) ...[
            if (shape != ThemeShape.values.first) const SizedBox(width: 2),
            SizedBox(
              height: settingsControlHeight,
              child: HouseButton(
                key: ValueKey<String>('settings-shape-${shape.name}'),
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
                frameless: ui.shape != shape,
                onPressed: () => setState(() => ui.setShape(shape)),
                child: Text(
                  (shape == ThemeShape.sharp
                          ? l10n.cornersSharp
                          : l10n.cornersRound)
                      .toUpperCase(),
                  style: ui.shape == shape ? t.kickerOn : t.kicker,
                ),
              ),
            ),
          ],
        ],
      );

  /// The five one-click accents, and the hex of whatever the accent actually
  /// is — which is not always one of the five, because the theme editor can
  /// set any colour at all and a custom theme carries its own.
  Widget _accentSwatches(LumitTheme t, LumitUiState ui) => Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final colour in LumitTheme.accentPresets) ...[
            if (colour != LumitTheme.accentPresets.first)
              const SizedBox(width: 4),
            GestureDetector(
              key: ValueKey<String>('settings-accent-${_hex(colour)}'),
              behavior: HitTestBehavior.opaque,
              onTap: () => setState(() => ui.workspace.setAccent(colour)),
              child: Container(
                width: _swatch,
                height: _swatch,
                decoration: BoxDecoration(
                  color: colour,
                  borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                  // The chosen one is ringed, so which of the five is in force
                  // is readable without reading the hex beside them.
                  border: _hex(colour) == _hex(t.accent)
                      ? Border.all(color: t.textPrimary)
                      : null,
                ),
              ),
            ),
          ],
          const SizedBox(width: 6),
          Text(
            _hex(t.accent),
            key: const ValueKey('settings-accent-hex'),
            style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
          ),
        ],
      );

  /// A colour as the drawing writes it: `#e05a72`. The channels are read as
  /// the workspace file reads them, so what is shown here and what is stored
  /// there can never drift apart by a rounding.
  static String _hex(Color c) {
    String pair(double channel) =>
        (channel * 255).round().toRadixString(16).padLeft(2, '0');
    return '#${pair(c.r)}${pair(c.g)}${pair(c.b)}';
  }

  /// The interface scale: a track, the number in a well, its unit, and the
  /// note that says when it lands. **On release**, not while dragging: the
  /// scale rebuilds and re-lays out the whole application, and doing that on
  /// every tick of a drag is a slideshow.
  Widget _scaleRow(LumitTheme t, LumitUiState ui) {
    final settings = ui.workspace.interface;
    void set(double percent) => setState(() {
          settings.uiScale = percent / 100;
          ui.workspace.settingsChanged();
        });
    final unit = t.mono.copyWith(fontSize: 10, color: t.textMuted);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        HouseSlider(
          key: const ValueKey('settings-ui-scale'),
          value: settings.uiScale * 100,
          min: 75,
          max: 200,
          step: 5,
          width: _scaleTrack,
          showValue: false,
          commitOnRelease: true,
          onChanged: set,
        ),
        const SizedBox(width: 8),
        SizedBox(
          width: _scaleWell,
          height: settingsControlHeight,
          child: DragValueField(
            key: const ValueKey('settings-ui-scale-value'),
            value: (settings.uiScale * 100).round(),
            min: 75,
            max: 200,
            decimals: 0,
            onChanged: (v) => set(v.toDouble()),
          ),
        ),
        const SizedBox(width: 8),
        Text(l10n.unitSymbolPercent, style: unit),
        const SizedBox(width: 8),
        // The one thing in the row that may be shortened: the window is
        // resizable, and at the drawing's own width this row fills the column
        // to the pixel.
        Flexible(
          child: Text(l10n.settingsAppliesOnRelease,
              style: unit, overflow: TextOverflow.ellipsis),
        ),
      ],
    );
  }

  // ---- Your themes (K-298) -------------------------------------------------

  /// What the last theme import, export or rename said. Kept beside the
  /// buttons like the keymap page's message, and for the same reason: a file
  /// that would not read is a fact about the file, not an emergency.
  String? _themeMessage;

  /// Everything you can do to a theme that is not changing one of its colours,
  /// as one row's worth of buttons: five verbs about the same thing, where five
  /// rows saying "Duplicate", "Rename" and so on would be a list of buttons
  /// pretending to be settings.
  Widget _themeShelf(LumitTheme t, LumitUiState ui) {
    final workspace = ui.workspace;
    final custom = workspace.customThemeName;
    return Wrap(
      spacing: 6,
      runSpacing: 6,
      children: [
        HouseButton(
          key: const ValueKey('settings-theme-duplicate'),
          small: true,
          onPressed: () => setState(() {
            final name = workspace.duplicateActiveTheme();
            _themeMessage = l10n.themeCopiedTo(name);
          }),
          child: Text(l10n.menuDuplicate, style: t.small),
        ),
        HouseButton(
          key: const ValueKey('settings-theme-rename'),
          small: true,
          // Only one of the user's own can be renamed: a built-in
          // scheme's name is Lumit's, not the user's, and renaming it
          // would leave two people describing different Darks.
          onPressed: custom == null ? null : () => _renameTheme(ui),
          child: Text(l10n.renameEllipsis, style: t.small),
        ),
        HouseButton(
          key: const ValueKey('settings-theme-delete'),
          small: true,
          frameless: true,
          onPressed: custom == null
              ? null
              : () => setState(() {
                    workspace.deleteCustomTheme(custom);
                    _themeMessage = l10n.themeDeleted(custom);
                  }),
          child: Text(l10n.delete, style: t.small),
        ),
        HouseButton(
          key: const ValueKey('settings-theme-import'),
          small: true,
          onPressed: () => _importTheme(ui),
          child: Text(l10n.menuImport, style: t.small),
        ),
        HouseButton(
          key: const ValueKey('settings-theme-export'),
          small: true,
          onPressed: () => _exportTheme(ui),
          child: Text(l10n.menuExport, style: t.small),
        ),
      ],
    );
  }

  /// Rename the selected theme. The workspace decides the name it lands under,
  /// so a clash with another of the user's own is numbered rather than refused.
  Future<void> _renameTheme(LumitUiState ui) async {
    final workspace = ui.workspace;
    final from = workspace.customThemeName;
    if (from == null) return;
    final asked = await askThemeName(context,
        title: l10n.themeRenameTitle, suggested: from, confirm: l10n.rename);
    if (asked == null || !mounted) return;
    final now = workspace.renameCustomTheme(from, asked);
    setState(() => _themeMessage = now == null || now == from
        ? null
        : now == asked.trim()
            ? l10n.themeRenamedTo(now)
            : l10n.themeNameTaken(asked.trim(), now));
  }

  /// Read a theme file and take it in under a name nothing else holds — an
  /// import never overwrites one of the user's own.
  Future<void> _importTheme(LumitUiState ui) async {
    final path = await pickThemeToOpen();
    if (path == null) return;
    String text;
    try {
      text = await File(path).readAsString();
    } catch (e) {
      if (mounted) {
        setState(() => _themeMessage = l10n.keymapFileUnreadable);
      }
      return;
    }
    final read = readThemeFile(text);
    if (!mounted) return;
    final theme = read.theme;
    if (theme == null) {
      setState(() => _themeMessage = read.refusal);
      return;
    }
    final wanted = theme.name.trim();
    final name = ui.workspace.importCustomTheme(theme);
    setState(() => _themeMessage = name == wanted
        ? l10n.themeImported(name)
        : l10n.themeImportedRenamed(wanted, name));
  }

  /// Write the theme in use out as a file. Offered from a built-in scheme too:
  /// what is exported is the colours on screen, and "the stock dark with my
  /// accent" is a perfectly good thing to send somebody.
  Future<void> _exportTheme(LumitUiState ui) async {
    final workspace = ui.workspace;
    final name = workspace.customThemeName ?? workspace.themeChoice.label;
    final path = await pickThemeSaveLocation(themeFileName(name));
    if (path == null) return;
    try {
      await File(path)
          .writeAsString(encodeThemeFile(CustomTheme.from(name, ui.theme)));
      if (mounted) setState(() => _themeMessage = l10n.themeExported(name));
    } catch (e) {
      if (mounted) {
        setState(() => _themeMessage = l10n.keymapFileUnwritable);
      }
    }
  }

  /// The Timeline page: how an edit behaves, and what the animation panels
  /// show. The two the first-run screen sets (K-246), plus the transport's one
  /// (K-254). They sit here as ordinary rows, and independently of each
  /// other: the screen offers its pair together, but somebody who wants
  /// Vegas ramps and After Effects imports is exactly the split docs/07
  /// §13.1 expects to be common.
  List<Widget> _timeline(LumitTheme t, LumitUiState ui) {
    final settings = ui.workspace.interface;
    void changed() => ui.workspace.settingsChanged();
    return _sections(t, [
      (
        l10n.settingsGroupEditing,
        [
          _flag(
              t, 'settings-retime-speed-lens', l10n.settingsRetimeOpensToSpeed,
              value: settings.retimeOpensToSpeed, set: (on) {
            settings.retimeOpensToSpeed = on;
            changed();
          }),
          _flag(t, 'settings-retime-in-seconds',
              l10n.settingsRetimeValuesInSeconds,
              value: settings.retimeInSeconds, set: (on) {
            settings.retimeInSeconds = on;
            changed();
          }),
          _flag(t, 'settings-video-as-sequence',
              l10n.settingsVideoArrivesAsASequence,
              value: settings.videoAsSequenceLayer, set: (on) {
            settings.videoAsSequenceLayer = on;
            changed();
          }),
          _flag(t, 'settings-paste-at-original-time',
              l10n.settingsPasteLayersAtTheirOriginal,
              value: settings.pasteLayersAtOriginalTime, set: (on) {
            settings.pasteLayersAtOriginalTime = on;
            changed();
          }),
          _flag(t, 'settings-playhead-stays',
              l10n.settingsPlayheadStaysWherePlaybackStopped,
              value: settings.playheadStaysOnStop, set: (on) {
            settings.playheadStaysOnStop = on;
            changed();
          }),
        ],
      ),
      (
        l10n.settingsGroupPanels,
        [
          _flag(t, 'settings-transform-in-fx',
              l10n.settingsTransformInEffectControls,
              value: settings.transformInEffectControls, set: (on) {
            settings.transformInEffectControls = on;
            changed();
          }),
          _flag(t, 'settings-easing-in-popup', l10n.settingsShapeEasesInAPopup,
              value: settings.easingInPopup, set: (on) {
            settings.easingInPopup = on;
            changed();
          }),
        ],
      ),
    ]);
  }

  /// The Viewer page: how the picture is shown. What the Viewer *looks* like —
  /// its surround, its scopes' colours — is Appearance's, one section down
  /// from the theme it follows; these two are about the image itself.
  List<Widget> _viewer(LumitTheme t, LumitUiState ui) {
    final settings = ui.workspace.interface;
    return _sections(t, [
      (
        l10n.settingsGroupPicture,
        [
          _flag(t, 'settings-smooth-zoomed-viewer',
              l10n.settingsSmoothThePictureWhenZoomed,
              value: ui.workspace.smoothZoomedViewer,
              set: ui.workspace.setSmoothZoomedViewer),
          _flag(t, 'settings-show-tone-map', l10n.settingsShowTheToneMapButton,
              value: settings.showToneMap, set: (on) {
            settings.showToneMap = on;
            ui.workspace.settingsChanged();
            // Turning it off disengages the tone map as well as hiding the
            // button, so the picture has to be asked for again — the look the
            // Viewer is now reading is not the one the engine was given.
            ui.pushViewerLook();
          }),
        ],
      ),
    ]);
  }

  // ---- Shortcuts (K-199) ---------------------------------------------------

  /// Every shortcut, grouped by where it is live. The table is the engine's —
  /// this walks what `keymap_groups` answered and draws it. The search is the
  /// title strip's, handed to the engine as it is typed.
  List<Widget> _keymap(LumitTheme t, LumitUiState ui) {
    final km = ui.keymap;
    final groups = km.visibleGroups;
    return [
      // Where the whole table comes from: one of the two presets, or a file.
      // Two rows rather than five buttons in a heap, so each says what it is.
      settingsSection(
          t,
          l10n.settingsGroupKeymap,
          [
            settingsRow(
              t,
              l10n.settingsPreset,
              '',
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  HouseButton(
                    key: const ValueKey('keymap-preset-lumit'),
                    small: true,
                    onPressed: () async {
                      await km.loadPreset(BridgeKeymapPreset.lumit);
                      if (mounted) setState(() {});
                    },
                    child: Text(l10n.keymapLumitDefault, style: t.small),
                  ),
                  const SizedBox(width: 6),
                  HouseButton(
                    key: const ValueKey('keymap-preset-ae'),
                    small: true,
                    onPressed: () async {
                      await km.loadPreset(BridgeKeymapPreset.afterEffects);
                      if (mounted) setState(() {});
                    },
                    child: Text(l10n.keymapAfterEffects, style: t.small),
                  ),
                ],
              ),
            ),
            // What the last import said, when it had something to say. Kept beside
            // the buttons rather than thrown as a dialogue: a keymap that would not
            // read is a fact about the file, not an emergency.
            settingsRow(
              t,
              l10n.settingsKeymapFile,
              _keymapMessage ?? '',
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  HouseButton(
                    key: const ValueKey('keymap-import'),
                    small: true,
                    onPressed: () => _importKeymap(km),
                    child: Text(l10n.menuImport, style: t.small),
                  ),
                  const SizedBox(width: 6),
                  HouseButton(
                    key: const ValueKey('keymap-export'),
                    small: true,
                    onPressed: () => _exportKeymap(km),
                    child: Text(l10n.menuExport, style: t.small),
                  ),
                ],
              ),
            ),
          ],
          first: true),
      // The clash warning. Present only when there is one, because a banner
      // that is always there is a banner nobody reads.
      if (km.conflicts.isNotEmpty)
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 10, 16, 0),
          child: Container(
            key: const ValueKey('keymap-conflicts'),
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            decoration: BoxDecoration(
              color: t.surface1,
              border: Border.all(color: t.warning),
              borderRadius: BorderRadius.circular(t.tokens.floatRadius),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  l10n.keymapClashGlobalCount(km.conflicts.length),
                  style: t.body,
                ),
                for (final clash in km.conflicts)
                  Padding(
                    padding: const EdgeInsets.only(top: 2),
                    child: Text(
                      '${chordLabel(clash.chord)} — ${clash.actions.join(', ')}',
                      style: t.small.copyWith(color: t.textMuted),
                    ),
                  ),
              ],
            ),
          ),
        ),
      // Panels that have taken a chord over from an app-wide one (K-281). Not
      // a warning — nothing is ambiguous, the focused panel simply wins — so
      // it is a quiet note rather than a bordered banner. It is said at all
      // because the app-wide meaning does stop working in that one panel, and
      // finding that out by pressing the key is worse than reading it here.
      if (km.shadows.isNotEmpty)
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 10, 16, 0),
          child: Column(
            key: const ValueKey('keymap-shadows'),
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                l10n.keymapClashPanelCount(km.shadows.length),
                style: t.small.copyWith(color: t.textMuted),
              ),
              for (final shadow in km.shadows)
                Padding(
                  padding: const EdgeInsets.only(top: 2),
                  child: Text(
                    l10n.keymapShadowLine(chordLabel(shadow.chord),
                        shadow.action, shadow.context, shadow.shadowed),
                    style: t.small.copyWith(color: t.textMuted),
                  ),
                ),
            ],
          ),
        ),
      if (groups.isEmpty)
        Padding(
          padding: const EdgeInsets.all(16),
          child: Text(l10n.keymapNoMatches,
              style: t.small.copyWith(color: t.textMuted)),
        ),
      for (final group in groups)
        settingsSection(t, group.label, [
          for (final binding in group.bindings)
            settingsRow(
              t,
              binding.description,
              '',
              _ChordCell(
                key: ValueKey('keymap-chord-${binding.context.name}-'
                    '${binding.action}'),
                binding: binding,
                keymap: km,
                onChanged: () {
                  if (mounted) setState(() {});
                },
              ),
            ),
        ]),
    ];
  }

  /// What the last import or export said, shown under the buttons.
  String? _keymapMessage;

  /// Read a keymap file and hand it to the engine, which refuses it whole if it
  /// is not one — so a wrong file costs nothing but the message.
  Future<void> _importKeymap(KeymapState km) async {
    final path = await pickKeymapToOpen();
    if (path == null) return;
    String text;
    try {
      text = await File(path).readAsString();
    } catch (e) {
      if (mounted) {
        setState(() => _keymapMessage = l10n.keymapFileUnreadable);
      }
      return;
    }
    final refusal = await km.fromJson(text);
    if (!mounted) return;
    setState(() => _keymapMessage = refusal ?? l10n.keymapImported);
  }

  /// Write the keymap out as the shareable file docs/07 §15 promises.
  Future<void> _exportKeymap(KeymapState km) async {
    final path = await pickKeymapSaveLocation();
    if (path == null) return;
    try {
      await File(path).writeAsString(km.toJson());
      if (mounted) setState(() => _keymapMessage = l10n.keymapExported);
    } catch (e) {
      if (mounted) {
        setState(() => _keymapMessage = l10n.keymapFileUnwritable);
      }
    }
  }

  /// One controller for the search box, kept across rebuilds so typing does
  /// not reset the cursor, and released with the window.
  TextEditingController? _search;
  final ScrollController _scroll = ScrollController();

  @override
  void dispose() {
    _perfTimer?.cancel();
    _search?.dispose();
    _scroll.dispose();
    super.dispose();
  }

  TextEditingController _searchController() {
    final existing = _search;
    if (existing != null) return existing;
    final created = TextEditingController(text: _query)
      ..addListener(() {
        _query = _search?.text ?? '';
        // The keymap filters engine-side against its own query, so the shared
        // field has to hand it over rather than filtering the rows itself.
        _keymapState()?.query = _query;
        if (mounted) setState(() {});
      });
    _search = created;
    return created;
  }

  List<Widget> _performance(LumitTheme t, LumitUiState ui) {
    // Filled by [_showPage] and refreshed by the page's slow tick — never
    // read here, because this method is a rebuild path.
    final perf = _perf!;
    final stats = perf.ram;
    final vram = perf.vram;
    final tier = perf.tier;
    final memory = perf.memory;

    return _sections(t, [
      (
        l10n.settingsGroupPlayback,
        [
          _row(
            t,
            l10n.settingsWhenTheMachineCannotKeep,
            _dropdown<PlaybackMode>(
              key: 'settings-playback-mode',
              value: ui.workspace.performance.playback,
              options: PlaybackMode.values,
              width: _ddWide,
              label: (m) => m == PlaybackMode.adaptive
                  ? l10n.playbackAdaptive
                  : l10n.playbackEveryFrame,
              onChanged: (m) => setState(() {
                ui.workspace.performance.playback = m;
                ui.workspace.settingsChanged();
              }),
            ),
          ),
          _row(
            t,
            l10n.settingsQualityTier,
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(_tierLabel(tier.tier),
                    key: const ValueKey('settings-tier'), style: t.small),
                const SizedBox(width: 8),
                LumitTooltip(
                  message: l10n.tipResetQualityTier,
                  child: HouseButton(
                    key: const ValueKey('settings-tier-reset'),
                    small: true,
                    onPressed: () => _perfEdit(resetRealtime),
                    child: Text(l10n.reset, style: t.small),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
      (
        l10n.settingsGroupRenderedFrameCache,
        [
          _budgetRow(
            t,
            key: 'settings-cache-budget',
            bytes: stats.budgetBytes.toInt(),
            ceilingMib: _systemMib,
            onSet: (bytes) => _perfEdit(() {
              setCacheBudget(bytes: bytes);
              ui.workspace.setCacheBudgetBytes(bytes.toInt());
            }),
          ),
          _row(
            t,
            l10n.settingsInUse,
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  l10n.settingsUsedMbIn(
                      _mib(stats.usedBytes.toInt()), '${stats.entries}'),
                  key: const ValueKey('settings-cache-used'),
                  style: t.small,
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: const ValueKey('settings-cache-clear'),
                  small: true,
                  onPressed: () => _perfEdit(clearCache),
                  child: Text(l10n.clear, style: t.small),
                ),
              ],
            ),
            description: l10n.settingsHelpCacheInUse('${stats.hits}',
                '${stats.hits + stats.misses}', '${stats.compDecodes}'),
          ),
        ],
      ),
      (
        l10n.settingsGroupPreviewCacheOnTheGraphics,
        [
          _budgetRow(
            t,
            key: 'settings-vram-budget',
            bytes: vram.budgetBytes.toInt(),
            ceilingMib: _vramMib,
            onSet: (bytes) => _perfEdit(() {
              setVramCacheBudget(bytes: bytes);
              ui.workspace.setVramBudgetBytes(bytes.toInt());
            }),
          ),
          _row(
            t,
            l10n.settingsInUse,
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  l10n.settingsUsedMbIn(
                      _mib(vram.usedBytes.toInt()), '${vram.entries}'),
                  key: const ValueKey('settings-vram-used'),
                  style: t.small,
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: const ValueKey('settings-vram-clear'),
                  small: true,
                  onPressed: () => _perfEdit(clearVramCache),
                  child: Text(l10n.clear, style: t.small),
                ),
              ],
            ),
          ),
        ],
      ),
      _diskCache(t, ui),
      // Where the memory has gone (K-294). Last on the page, under the tiers
      // it weighs: each section above reports one store, and this one reports
      // the whole process and what none of them accounts for.
      //
      // **Debug builds only** (owner, 2026-08-06). It is an instrument for
      // hunting a fault, not a setting: a shipped editor asking its user to
      // interpret live texture counts has handed them the engineering rather
      // than the tool. `kDebugMode` is false in both profile and release
      // builds, so what ships is the page without it.
      (
        l10n.settingsGroupMemory,
        memory == null
            ? <Widget?>[]
            : [
                _row(
                  t,
                  l10n.settingsThisProcess,
                  Text(
                    memory.processBytes == BigInt.zero
                        ? l10n.settingsMemoryNotKnown
                        : _bytes(memory.processBytes),
                    key: const ValueKey('settings-memory-process'),
                    style: t.small,
                  ),
                ),
                _row(
                  t,
                  l10n.settingsNotHeldByAnyCache,
                  Text(
                    memory.processBytes == BigInt.zero
                        ? '—'
                        : _bytes(memory.unaccountedBytes),
                    key: const ValueKey('settings-memory-unaccounted'),
                    style: t.small,
                  ),
                ),
                _row(
                  t,
                  l10n.settingsHeldByTheGraphicsDriver,
                  Text(
                    l10n.settingsMemoryTexturesBuffers(
                        '${memory.gpuTextures}', '${memory.gpuBuffers}'),
                    key: const ValueKey('settings-memory-gpu'),
                    style: t.small,
                  ),
                ),
                // The byte figures are Vulkan and D3D12 only, so the row is not
                // drawn at all on a Mac rather than printing two zeroes and
                // inviting the reader to draw a conclusion from them.
                if (memory.gpuReservedBytes != BigInt.zero)
                  _row(
                    t,
                    l10n.settingsGraphicsMemoryReserved,
                    Text(
                      l10n.settingsMemoryReservedInUse(
                          _bytes(memory.gpuReservedBytes),
                          _bytes(memory.gpuAllocatedBytes)),
                      key: const ValueKey('settings-memory-gpu-bytes'),
                      style: t.small,
                    ),
                  ),
                _row(
                  t,
                  l10n.settingsOpenMediaDecoders,
                  Text(
                    '${memory.openDecoders}',
                    key: const ValueKey('settings-memory-decoders'),
                    style: t.small,
                  ),
                ),
                _row(
                  t,
                  l10n.settingsFramesWaitingToBeWritten,
                  Text(
                    '${memory.parkQueueFrames}',
                    key: const ValueKey('settings-memory-parks'),
                    style: t.small,
                  ),
                ),
              ],
      ),
    ]);
  }

  /// The disk tier (docs/06 §5.4, docs/07 §15): its budget, where it lives, and
  /// what it holds. The bottom of the three-tier cache and the only one that
  /// outlives the session, which is why it has a folder at all.
  (String, List<Widget?>) _diskCache(LumitTheme t, LumitUiState ui) {
    final disk = _perf!.disk;
    // What this project says, if it says anything: a project's own choice
    // overrides the application's, so it is what the controls should show.
    final own = _perf!.own;
    final scope = own == null ? CacheScope.everywhere : CacheScope.thisProject;
    final where = own?.location ??
        cacheLocationFromName(ui.workspace.performance.diskCacheLocation ??
            BridgeCacheLocation.appData.name);
    return (
      l10n.settingsGroupFramesParkedOnDisk,
      [
        _budgetRow(
          t,
          key: 'settings-disk-budget',
          bytes: disk.budgetBytes.toInt(),
          ceilingMib: _diskCeilingMib,
          onSet: (bytes) => _perfEdit(() {
            setDiskCacheBudget(bytes: bytes);
            ui.workspace.setDiskBudgetBytes(bytes.toInt());
          }),
        ),
        _row(
          t,
          l10n.settingsWhere,
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              SizedBox(
                width: _ddWide,
                height: settingsControlHeight,
                child: BareDropdown<BridgeCacheLocation>(
                  key: const ValueKey('settings-disk-location'),
                  value: where,
                  options: BridgeCacheLocation.values,
                  label: _locationLabel,
                  onChanged: (l) => _setLocation(ui, l, scope),
                ),
              ),
              if (where == BridgeCacheLocation.custom) ...[
                const SizedBox(width: 8),
                LumitTooltip(
                  message: l10n.tipChooseCacheFolder,
                  child: HouseButton(
                    key: const ValueKey('settings-disk-folder'),
                    small: true,
                    onPressed: () => _pickCacheFolder(ui, scope),
                    child: Text(l10n.chooseEllipsis, style: t.small),
                  ),
                ),
              ],
            ],
          ),
          description:
              disk.root.isEmpty ? l10n.settingsHelpNowhereToPark : disk.root,
        ),
        _row(
          t,
          l10n.settingsAppliesTo,
          _dropdown<CacheScope>(
            key: 'settings-disk-scope',
            value: scope,
            options: CacheScope.values,
            width: _ddWide,
            label: (s) => switch (s) {
              CacheScope.everywhere => l10n.scopeEverything,
              CacheScope.thisProject => l10n.scopeThisProject,
            },
            onChanged: (s) => _setScope(ui, s, where),
          ),
        ),
        _row(
          t,
          l10n.settingsInUse,
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Text(
                l10n.settingsUsedMbIn(
                    _mib(disk.usedBytes.toInt()), '${disk.entries}'),
                key: const ValueKey('settings-disk-used'),
                style: t.small,
              ),
              const SizedBox(width: 8),
              HouseButton(
                key: const ValueKey('settings-disk-clear'),
                small: true,
                onPressed: () async {
                  final cleared = await confirmClearDiskCache(context);
                  if (cleared && mounted) _perfEdit(() {});
                },
                child: Text(l10n.clear, style: t.small),
              ),
            ],
          ),
        ),
      ]
    );
  }

  /// The open project, or null before one exists — read through the provider
  /// rather than held, since the settings window outlives no project.
  ProjectReference? _project(BuildContext context) =>
      Provider.of<LumitState>(context, listen: false).project;

  static String _locationLabel(BridgeCacheLocation l) => switch (l) {
        BridgeCacheLocation.appData => l10n.cacheLocationWithLumit,
        BridgeCacheLocation.besideProject => l10n.cacheLocationBesideProject,
        BridgeCacheLocation.custom => l10n.cacheLocationChosenFolder,
      };

  /// Point the cache somewhere, at whichever scope is in force. The project's own
  /// choice is an op (undoable, saved in the `.lum`); the application's is a
  /// setting. Same control, same three options — only the store differs.
  void _setLocation(
    LumitUiState ui,
    BridgeCacheLocation location,
    CacheScope scope,
  ) {
    final folder = scope == CacheScope.thisProject
        ? (_project(context)?.cacheLocation()?.folder ?? '')
        : (ui.workspace.performance.diskCacheFolder ?? '');
    _perfEdit(() {
      if (scope == CacheScope.thisProject) {
        _project(context)?.setCacheLocation(
          location:
              BridgeProjectCacheLocation(location: location, folder: folder),
        );
      } else {
        // Choosing the custom option without a folder yet leaves the tier where
        // it is; the engine says so by keeping its default, and the Choose…
        // button appears beside the dropdown.
        setDiskCacheLocation(location: location, folder: folder);
        ui.workspace.setDiskCacheLocation(
            location.name, folder.isEmpty ? null : folder);
      }
    });
  }

  /// Switch between "this project decides" and "the application decides".
  /// Turning it off clears the project's override rather than copying the
  /// application's answer into it, so the project follows along afterwards.
  void _setScope(LumitUiState ui, CacheScope scope, BridgeCacheLocation where) {
    _perfEdit(() {
      switch (scope) {
        case CacheScope.thisProject:
          _project(context)?.setCacheLocation(
            location: BridgeProjectCacheLocation(
              location: where,
              folder: ui.workspace.performance.diskCacheFolder ?? '',
            ),
          );
        case CacheScope.everywhere:
          _project(context)?.setCacheLocation(location: null);
      }
    });
  }

  Future<void> _pickCacheFolder(LumitUiState ui, CacheScope scope) async {
    final folder = await pickFolder();
    if (folder == null || !mounted) return;
    _perfEdit(() {
      if (scope == CacheScope.thisProject) {
        _project(context)?.setCacheLocation(
          location: BridgeProjectCacheLocation(
              location: BridgeCacheLocation.custom, folder: folder),
        );
      } else {
        setDiskCacheLocation(
            location: BridgeCacheLocation.custom, folder: folder);
        ui.workspace
            .setDiskCacheLocation(BridgeCacheLocation.custom.name, folder);
      }
    });
  }

  /// The ceiling for the disk budget. Free disk space is not something the
  /// engine reports yet (K-194 covers memory only), so the field is generous
  /// rather than guessed at: 500 GB, which no cache should reach and no user
  /// should be stopped short of.
  static const double _diskCeilingMib = 500 * 1024;

  // ---- the shapes every page is built from ---------------------------------

  /// A cache budget: type a number of megabytes, or drag it, up to what the
  /// machine actually has (K-194).
  ///
  /// A typed box rather than a pick from a fixed list — the old dropdown could
  /// not say "3 GB on a 32 GB machine", and its options were a guess at what
  /// hardware would show up.
  Widget? _budgetRow(
    LumitTheme t, {
    required String key,
    required int bytes,
    required double ceilingMib,
    required ValueChanged<BigInt> onSet,
  }) =>
      _row(
        t,
        l10n.settingsBudget,
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            SizedBox(
              width: 110,
              height: settingsControlHeight,
              child: DragValueField(
                key: ValueKey<String>(key),
                value: (bytes >> 20).toDouble(),
                min: _minBudgetMib,
                max: ceilingMib,
                // A megabyte a pixel is far too fine on a 32 GB ceiling.
                speed: 16,
                decimals: 0,
                onChanged: (mib) => onSet(BigInt.from(mib.round()) << 20),
              ),
            ),
            const SizedBox(width: 8),
            Text(l10n.unitMb,
                style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
          ],
        ),
      );

  /// What the machine has, in MiB, falling back to a documented ceiling when
  /// it will not say. Installed RAM is answered on all three desktops
  /// (K-204); video memory is Windows-only so far, so that is the one that
  /// still falls back off Windows. Read once per run — the machine's memory
  /// does not change under a process, and as getters these were a bridge
  /// call per rebuild.
  static final double _systemMib = _mibOf(systemMemoryBytes());
  static final double _vramMib = _mibOf(videoMemoryBytes());

  static double _mibOf(BigInt bytes) {
    final mib = (bytes >> 20).toDouble();
    return mib <= 0 ? _unknownMemoryMib : mib;
  }

  /// Bytes as a person reads them — MB up to a gigabyte, GB above, one
  /// decimal so 85.4 GB does not print as 85.
  static String _bytes(BigInt bytes) {
    final b = bytes.toDouble();
    if (b >= 1 << 30) {
      return '${(b / (1 << 30)).toStringAsFixed(1)} ${l10n.unitGb}';
    }
    return '${(b / (1 << 20)).toStringAsFixed(0)} ${l10n.unitMb}';
  }

  static String _mib(int bytes) => (bytes / (1 << 20)).toStringAsFixed(0);

  static String _tierLabel(int tier) => switch (tier) {
        1 => l10n.menuFull,
        2 => l10n.menuHalf,
        3 => l10n.resolutionThird,
        _ => l10n.menuQuarter,
      };
}

/// One row's chord cell: what runs this action, and the way to change it.
///
/// Click it and it listens for the next chord you press — the keypress is
/// swallowed while it does, so binding `Ctrl+S` does not also save. Escape
/// leaves it alone; Backspace or Delete clears the binding. It shows *every*
/// chord an action has, because an action can have two (K-198) and one the
/// table did not draw would be a key that works with nothing on screen to say
/// so.
class _ChordCell extends StatefulWidget {
  final BridgeKeyBinding binding;
  final KeymapState keymap;
  final VoidCallback onChanged;

  const _ChordCell({
    super.key,
    required this.binding,
    required this.keymap,
    required this.onChanged,
  });

  @override
  State<_ChordCell> createState() => _ChordCellState();
}

class _ChordCellState extends State<_ChordCell> {
  bool _listening = false;
  String? _refusal;

  /// Take the next chord as this row's binding. Runs on every key event while
  /// listening, and always reports the event handled so nothing else acts on
  /// the keys being bound.
  ///
  /// Listening stops the moment a chord *arrives*, not when the engine answers:
  /// the engine call is a round trip, and a cell that went on saying "press a
  /// shortcut" until it came back would invite a second press that bound the
  /// wrong key. A refusal comes back into the cell as a message beside it.
  Future<bool> _capture(KeyEvent event) async {
    if (event is! KeyDownEvent) return true;
    if (event.logicalKey == LogicalKeyboardKey.escape) {
      _stopListening();
      return true;
    }
    if (event.logicalKey == LogicalKeyboardKey.backspace ||
        event.logicalKey == LogicalKeyboardKey.delete) {
      _stopListening();
      await widget.keymap.unbind(widget.binding.context, widget.binding.action);
      widget.onChanged();
      return true;
    }
    final chord = chordText(event);
    // A modifier on its own is half a chord: keep listening rather than
    // binding Shift to something.
    if (chord == null) return true;
    _stopListening();
    final refusal = await widget.keymap
        .rebind(widget.binding.context, widget.binding.action, chord);
    if (mounted) setState(() => _refusal = refusal);
    widget.onChanged();
    return true;
  }

  @override
  void dispose() {
    // The handler outlives the widget otherwise: a row scrolled out of the
    // lazy list mid-capture would go on swallowing every keypress in the app.
    if (_listening) HardwareKeyboard.instance.removeHandler(_handler);
    super.dispose();
  }

  bool _handler(KeyEvent event) {
    unawaited(_capture(event));
    return true;
  }

  void _startListening() {
    if (_listening) return;
    HardwareKeyboard.instance.addHandler(_handler);
    setState(() {
      _listening = true;
      _refusal = null;
    });
  }

  void _stopListening() {
    if (!_listening) return;
    HardwareKeyboard.instance.removeHandler(_handler);
    if (mounted) {
      setState(() => _listening = false);
    } else {
      _listening = false;
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final chord = widget.binding.chord;
    final label = _listening
        ? l10n.keymapPressAShortcut
        : chord.isEmpty
            ? l10n.keymapNotSet
            : chordLabel(chord);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (_refusal != null)
          Padding(
            padding: const EdgeInsets.only(right: 6),
            child: Text(_refusal!, style: t.small.copyWith(color: t.error)),
          ),
        GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: _startListening,
          child: Container(
            constraints: const BoxConstraints(minWidth: 110),
            height: settingsControlHeight,
            alignment: Alignment.center,
            padding: const EdgeInsets.symmetric(horizontal: 8),
            decoration: BoxDecoration(
              color: _listening ? t.accent : t.surface2,
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            ),
            child: Text(
              label,
              textAlign: TextAlign.center,
              style: chord.isEmpty && !_listening
                  ? t.small.copyWith(color: t.textMuted)
                  : t.small,
            ),
          ),
        ),
        const SizedBox(width: 6),
        HouseButton(
          small: true,
          onPressed: () async {
            await widget.keymap
                .resetBinding(widget.binding.context, widget.binding.action);
            widget.onChanged();
          },
          child: Text(l10n.reset, style: t.small),
        ),
      ],
    );
  }
}
