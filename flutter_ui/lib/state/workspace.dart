// The workspace controller: everything the egui `Shell` persists (dock
// layout, colour scheme, shape, accent override, animation level, the
// settings structs), held in one ChangeNotifier and written to a JSON file —
// the Flutter counterpart of eframe's storage (docs/archive/flutter-port/03).

import 'dart:convert';
import 'dart:io';
import 'dart:ui';

import 'package:flutter/foundation.dart';

import '../l10n/strings.dart';
import '../theme/custom_theme.dart';
import '../theme/theme.dart';
import 'dock.dart';
import 'settings.dart';

/// How the Viewer is looking at one composition (K-314): exposure in stops and
/// whether the tone map is engaged. A record rather than a class because it is
/// two numbers with no behaviour, and records compare by value — which is what
/// [SavedSession]'s equality needs.
typedef ViewerLook = ({double stops, bool toneMap});

/// Neither control engaged: the picture is the export.
const ViewerLook neutralLook = (stops: 0.0, toneMap: false);

/// The per-project session the egui shell restores on open (its `SavedSession`,
/// crates/lumit-ui/src/app_state/mod.rs): which compositions are open, which is
/// fronted, where the playhead sits, and which layer is selected. Ids are the
/// snapshot's own string ids (the Flutter port keys by them, not Uuid). Stale
/// ids are validated against the document on restore and fall back to defaults,
/// so a session that names a since-deleted comp/layer never crashes.
class SavedSession {
  final List<String> openComps;
  final String? activeComp;
  final int frame;
  final String? selectedLayer;

  /// The Viewer's exposure and tone map, per composition id (K-314). A way of
  /// *looking*, not an edit to the work: it rides in the session (and thus in
  /// the `.lum`'s `ui_state` blob, K-245) rather than in the document, so
  /// Ctrl+Z never undoes an exposure nudge and setting one does not make the
  /// project dirty. Comps looking at neutral are simply absent.
  final Map<String, ViewerLook> viewerLooks;

  /// The preview resolution of each composition, by id (K-357, docs/07 §2.2
  /// item 2), as the enum's name. Session state for the same reason the looks
  /// are: choosing how coarsely to preview a shot is a way of working on it,
  /// not an edit to it, and it must never reach an export (glossary §5).
  /// Comps previewing at Auto — the default — are simply absent.
  final Map<String, String> previewResolutions;

  /// How the panels were arranged for this project, as [DockSplit.toJson]
  /// (K-245) — the arrangement itself, not the name of a preset, because the
  /// sizes and positions a user drags to are the arrangement.
  ///
  /// Held as raw JSON rather than as a [DockSplit], because this class is
  /// compared to decide whether anything needs writing, and a dock tree is
  /// mutated in place as panels are dragged: the object would be equal to
  /// itself after every change. Raw JSON also means an arrangement naming a
  /// panel this build has never heard of survives being read and written back.
  final Map<String, dynamic>? dock;

  const SavedSession({
    this.openComps = const [],
    this.activeComp,
    this.frame = 0,
    this.selectedLayer,
    this.dock,
    this.viewerLooks = const {},
    this.previewResolutions = const {},
  });

  Map<String, dynamic> toJson() => {
        'open_comps': openComps,
        'active_comp': activeComp,
        'frame': frame,
        'selected_layer': selectedLayer,
        'dock': dock,
        'viewer_looks': {
          for (final e in viewerLooks.entries)
            e.key: {'stops': e.value.stops, 'tone_map': e.value.toneMap},
        },
        'preview_resolutions': previewResolutions,
      };

  /// The per-comp resolutions out of a session's JSON, dropping anything that
  /// is not a plain string — a name this build has never heard of is left for
  /// the caller to resolve, so a project from a newer build opens rather than
  /// failing.
  static Map<String, String> _resolutionsFromJson(Object? raw) {
    if (raw is! Map) return const {};
    return {
      for (final e in raw.entries)
        if (e.key is String && e.value is String)
          e.key as String: e.value as String,
    };
  }

  /// The looks out of a session's JSON, dropping any entry that is not the
  /// shape this build writes — a project from another build must open, looking
  /// neutral, rather than fail to open.
  static Map<String, ViewerLook> _looksFromJson(Object? raw) {
    if (raw is! Map) return const {};
    return {
      for (final e in raw.entries)
        if (e.key is String &&
            e.value is Map &&
            (e.value as Map)['stops'] is num)
          e.key as String: (
            stops: ((e.value as Map)['stops'] as num).toDouble(),
            toneMap: (e.value as Map)['tone_map'] == true,
          ),
    };
  }

  factory SavedSession.fromJson(Map<String, dynamic> j) => SavedSession(
        openComps: j['open_comps'] is List
            ? [
                for (final c in j['open_comps'] as List)
                  if (c is String) c
              ]
            : const [],
        activeComp:
            j['active_comp'] is String ? j['active_comp'] as String : null,
        frame: j['frame'] is num ? (j['frame'] as num).toInt() : 0,
        selectedLayer: j['selected_layer'] is String
            ? j['selected_layer'] as String
            : null,
        dock: j['dock'] is Map
            ? (j['dock'] as Map).cast<String, dynamic>()
            : null,
        viewerLooks: _looksFromJson(j['viewer_looks']),
        previewResolutions: _resolutionsFromJson(j['preview_resolutions']),
      );

  /// The arrangement compared by value. Encoding is the cheap deep compare
  /// here: both sides are built key-by-key in the same order by [toJson], so
  /// equal trees encode identically.
  String get _dockKey => dock == null ? '' : jsonEncode(dock);

  @override
  bool operator ==(Object other) =>
      other is SavedSession &&
      other.activeComp == activeComp &&
      other.frame == frame &&
      other.selectedLayer == selectedLayer &&
      other._dockKey == _dockKey &&
      mapEquals(other.viewerLooks, viewerLooks) &&
      mapEquals(other.previewResolutions, previewResolutions) &&
      listEquals(other.openComps, openComps);

  @override
  int get hashCode => Object.hash(
        activeComp,
        frame,
        selectedLayer,
        _dockKey,
        Object.hashAll(openComps),
        Object.hashAll([
          for (final e in viewerLooks.entries) Object.hash(e.key, e.value),
        ]),
        Object.hashAll([
          for (final e in previewResolutions.entries)
            Object.hash(e.key, e.value),
        ]),
      );
}

/// Where a floating window was left (K-242): how far it was dragged from the
/// centre of the app window, and how big it was made when it is one of the
/// resizable ones. Stored as an offset from centre rather than as a corner
/// position so a window opened on a smaller monitor than it was left on still
/// lands somewhere sensible.
class WindowPlacement {
  final Offset offset;
  final Size? size;

  const WindowPlacement(this.offset, this.size);

  Map<String, dynamic> toJson() => {
        'dx': offset.dx,
        'dy': offset.dy,
        if (size != null) 'w': size!.width,
        if (size != null) 'h': size!.height,
      };

  static WindowPlacement? fromJson(Map<String, dynamic> j) {
    final dx = j['dx'], dy = j['dy'];
    if (dx is! num || dy is! num) return null;
    final w = j['w'], h = j['h'];
    return WindowPlacement(
      Offset(dx.toDouble(), dy.toDouble()),
      w is num && h is num ? Size(w.toDouble(), h.toDouble()) : null,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is WindowPlacement && other.offset == offset && other.size == size;

  @override
  int get hashCode => Object.hash(offset, size);
}

class Workspace extends ChangeNotifier {
  DockSplit dock = defaultLayout();
  LumitColorScheme colorScheme = LumitColorScheme.dark;
  ThemeShape themeShape = ThemeShape.sharp;
  Color? accentOverride;
  AnimationLevel animationLevel = AnimationLevel.all;

  /// The themes the user has made (K-202), in the order they were saved.
  List<CustomTheme> customThemes = [];

  /// The custom theme in use, by name, or null for the built-in
  /// [colorScheme]. Two fields rather than one because a built-in scheme is
  /// still what a custom theme is *built over*, and because a name that no
  /// longer matches a saved theme has to fall back to something — which it
  /// does, silently, to the built-in that is always there.
  String? customThemeName;

  /// The custom theme in use, or null when a built-in scheme is selected or
  /// the named one has been deleted.
  CustomTheme? get activeCustomTheme {
    final name = customThemeName;
    if (name == null) return null;
    for (final theme in customThemes) {
      if (theme.name == name) return theme;
    }
    return null;
  }

  /// Whether the Scopes panel draws in the theme's colours rather than the
  /// standard broadcast set (K-202). Off by default: a scope is read on a
  /// near-black graticule whatever the chrome, which is the same
  /// grading-accuracy reasoning that keeps the Viewer surround neutral
  /// (docs/15-DESIGN §8, §2.1). On, because it does look good.
  bool themedScopes = false;

  /// Whether the Viewer's surround takes the theme's own surface rather than
  /// the neutral grey (K-203). Off by default, and for the same reason the
  /// scopes toggle is: a grade cannot be judged against a tinted surround
  /// (docs/15-DESIGN §2.1/§11). Offered anyway, because a neutral rectangle in
  /// the middle of a themed shell is a thing people want to turn off.
  bool themedViewerSurround = false;

  /// Whether the Viewer smooths the picture when it is zoomed past 1:1. Off,
  /// so a magnified pixel is a square and what is on screen is what is in the
  /// frame — the reason to zoom in is usually to look at the pixels. On,
  /// Flutter's bilinear filtering blends them, which is gentler on the eye
  /// when the zoom is being used to frame rather than to inspect.
  bool smoothZoomedViewer = false;

  /// Working preferences for the Pre-compose dialogue (Ctrl+Shift+C).
  /// Default: Move attributes = true, Adjust duration = true, Open new comp = false.
  /// If changed by the user, saved straight to the workspace store.
  bool precomposeMoveAttributes = true;
  bool precomposeAdjustDuration = true;
  bool precomposeOpenNewComp = false;

  PerformanceSettings performance = PerformanceSettings();
  InterfaceSettings interface = InterfaceSettings();

  /// Whether Lumit looks for a newer version on launch (K-296).
  ///
  /// On by default, and offered on the setup screen as well as in Settings: an
  /// editor that quietly falls years behind is how people end up reporting bugs
  /// that were fixed long ago. It is a *look*, not a download — the installer
  /// is only fetched when the user asks for it, so leaving this on never costs
  /// anybody a surprise few hundred megabytes.
  bool autoUpdate = true;

  /// When the last update check finished, in milliseconds since the epoch.
  /// Zero means never. Kept so six launches in a morning ask GitHub once.
  int lastUpdateCheckMs = 0;

  /// Whether the first-run screen has had its answer (K-246, docs/07 §13.1).
  ///
  /// **True unless [load] finds no settings file.** Only loading can tell a
  /// genuine first run from a `Workspace` built for some other reason — every
  /// widget test constructs one, and a default of false would put the screen
  /// over the whole suite. A file that predates this field also answers true:
  /// an existing settings file means an existing user, and asking them where
  /// they came from after months of work would be absurd.
  bool firstRunDone = true;

  /// Take the Vegas answer, or the After Effects one, from the first-run screen
  /// (K-246). Both settings move together here and separately in Settings —
  /// this is the pair the screen offers, not a mode the rest of the code reads.
  /// Marks the screen answered, so it is asked exactly once.
  void setEditingStyle({required bool vegas}) {
    interface.retimeOpensToSpeed = vegas;
    interface.videoAsSequenceLayer = vegas;
    firstRunDone = true;
    settingsChanged();
  }

  /// Dismiss the first-run screen without changing anything (skip = Lumit
  /// defaults, docs/07 §13.1). Recorded so it is not asked again.
  void skipFirstRun() {
    firstRunDone = true;
    settingsChanged();
  }

  /// The keymap as the engine last serialised it, stored verbatim and never
  /// read here (docs/07 §15, K-199). A keymap is machine-local settings and
  /// this is the machine-local settings file; the *rules* stay in Rust, so what
  /// sits in this field is an opaque blob the frontend only ferries. Null until
  /// the user changes a binding, which is what makes the shipped defaults the
  /// default rather than a copy of them written out at first launch.
  String? keymapJson;

  /// Store the engine's keymap text and write the file. Called after every
  /// binding change, which is rare and cheap — the file is a few kilobytes and
  /// a keymap edit is a deliberate act, not something that happens per frame.
  void setKeymapJson(String json) {
    keymapJson = json;
    save();
  }

  /// Remember the rendered-frame cache budget the user just set, so the next
  /// launch asks the engine for the same one. The engine holds the live budget
  /// but has no store behind it, so without this the number resets on restart
  /// while every other setting survives. Written straight away for the same
  /// reason the keymap is: it is a deliberate act, not a per-frame event.
  void setCacheBudgetBytes(int bytes) {
    performance.cacheBudgetBytes = bytes;
    save();
  }

  /// As [setCacheBudgetBytes], for the graphics card's preview cache.
  void setVramBudgetBytes(int bytes) {
    performance.vramBudgetBytes = bytes;
    save();
  }

  /// As [setCacheBudgetBytes], for the frames parked on disk.
  void setDiskBudgetBytes(int bytes) {
    performance.diskBudgetBytes = bytes;
    save();
  }

  /// Remember where the disk cache should live: the engine's own location name,
  /// and the folder for the custom one (null for the other two).
  void setDiskCacheLocation(String location, String? folder) {
    performance.diskCacheLocation = location;
    performance.diskCacheFolder = folder;
    save();
  }

  /// The project last opened or saved with a path, restored on the next launch
  /// (the egui frontend reopens the last project the same way). Null until a
  /// project has been opened or saved to a file. This is only the *file*;
  /// [sessions] carries the per-project session beside it.
  String? lastProjectPath;

  /// Per-project sessions keyed by project file path — the Flutter counterpart
  /// of the egui shell's `SavedSession` map, restored when a project reopens.
  final Map<String, SavedSession> sessions = {};

  LumitTheme _theme = LumitTheme.dark();
  LumitTheme get theme => _theme;

  Workspace() {
    recompose();
  }

  /// Rebuild the theme from the current appearance fields — the single funnel
  /// every Appearance control uses (`Shell::recompose`).
  /// A theme shown but not saved — the customise window's live preview
  /// (K-202). It overrides everything while set, and clearing it recomposes
  /// from the stored selection, so a discarded edit leaves nothing behind.
  LumitTheme? _preview;

  void previewTheme(LumitTheme theme) {
    _preview = theme;
    _theme = theme;
    notifyListeners();
  }

  void clearPreview() {
    _preview = null;
    recompose();
  }

  void recompose() {
    if (_preview != null) {
      _theme = _preview!;
      notifyListeners();
      return;
    }
    final custom = activeCustomTheme;
    if (custom != null) {
      // A custom theme carries its own accent among its colours, so the
      // accent override does not apply on top — it would silently overwrite
      // a choice the user made in the editor.
      _theme = custom.build(themeShape);
    } else {
      _theme = LumitTheme.forScheme(
        colorScheme,
        themeShape,
        accentOverride: accentOverride,
      );
    }
    notifyListeners();
  }

  /// One entry in the theme picker: a built-in scheme, or one of the user's.
  List<ThemeChoice> get themeChoices => [
        for (final s in LumitColorScheme.values)
          if (s.mode == ThemeMode2.dark) ThemeChoice.builtIn(s),
        for (final s in LumitColorScheme.values)
          if (s.mode == ThemeMode2.light) ThemeChoice.builtIn(s),
        for (final t in customThemes) ThemeChoice.custom(t.name),
      ];

  /// What the picker shows as selected. A custom theme whose name no longer
  /// matches anything saved falls back to the built-in underneath it.
  ThemeChoice get themeChoice => activeCustomTheme != null
      ? ThemeChoice.custom(activeCustomTheme!.name)
      : ThemeChoice.builtIn(colorScheme);

  void choose(ThemeChoice choice) {
    final custom = choice.customName;
    if (custom != null) {
      setCustomTheme(custom);
    } else {
      setScheme(choice.scheme!);
    }
  }

  /// Select a built-in scheme, leaving any custom theme behind.
  void setScheme(LumitColorScheme s) {
    colorScheme = s;
    customThemeName = null;
    recompose();
    save();
  }

  /// Select one of the user's themes by name.
  void setCustomTheme(String name) {
    customThemeName = name;
    recompose();
    save();
  }

  /// Save a custom theme — replacing one of the same name, else appending —
  /// and select it. What the editor's Save does.
  void saveCustomTheme(CustomTheme theme) {
    final at = customThemes.indexWhere((t) => t.name == theme.name);
    if (at >= 0) {
      customThemes[at] = theme;
    } else {
      customThemes.add(theme);
    }
    customThemeName = theme.name;
    recompose();
    save();
  }

  /// A name no saved theme holds: [wanted] itself when it is free, else the
  /// same with a number after it. Two themes cannot share a name — the name
  /// *is* the identity, both in the picker and in the workspace file — so
  /// every route that adds one comes through here rather than overwriting
  /// somebody's work by accident (K-298).
  String availableThemeName(String wanted) {
    final base = wanted.trim().isEmpty ? l10n.themeUnnamed : wanted.trim();
    var tried = base;
    for (var n = 2; customThemes.any((t) => t.name == tried); n++) {
      tried = '$base $n';
    }
    return tried;
  }

  /// Copy the theme in use into one of the user's own, and select it (K-298).
  /// Returns the name it landed under.
  ///
  /// Works from a built-in scheme as well as from a custom theme: "start from
  /// this one and change a few things" is the same wish either way, and it is
  /// how a built-in becomes editable without the editor having to ask for a
  /// name first.
  String duplicateActiveTheme() {
    final name = availableThemeName('${themeChoice.label} copy');
    saveCustomTheme(CustomTheme.from(name, theme));
    return name;
  }

  /// Take an imported theme in and select it (K-298). Returns the name it
  /// landed under, which differs from the file's when a theme already had it —
  /// an import never overwrites one of the user's own.
  String importCustomTheme(CustomTheme imported) {
    final name = availableThemeName(imported.name);
    saveCustomTheme(imported.renamed(name));
    return name;
  }

  /// Rename one of the user's themes, keeping its place in the list and the
  /// selection on it. Returns the name it now has — [to] when that was free,
  /// else [to] with a number after it — or null when [from] is not a saved
  /// theme.
  String? renameCustomTheme(String from, String to) {
    final at = customThemes.indexWhere((t) => t.name == from);
    if (at < 0) return null;
    final wanted = to.trim();
    if (wanted.isEmpty || wanted == from) return from;
    final name = availableThemeName(wanted);
    customThemes[at] = customThemes[at].renamed(name);
    if (customThemeName == from) customThemeName = name;
    recompose();
    save();
    return name;
  }

  /// Forget a custom theme. Selecting it afterwards is impossible, so a
  /// session using it falls back to its built-in scheme.
  void deleteCustomTheme(String name) {
    customThemes.removeWhere((t) => t.name == name);
    if (customThemeName == name) customThemeName = null;
    recompose();
    save();
  }

  /// Turn automatic update checks on or off (K-296). Written straight out: it
  /// is one boolean, and a setting that did not survive the restart it is about
  /// would be a poor joke.
  void setAutoUpdate(bool on) {
    autoUpdate = on;
    settingsChanged();
  }

  /// Record that a check has just happened, so the next launch does not repeat
  /// it. Saved without notifying — nothing on screen reads this.
  void rememberUpdateCheck(int atMillis) {
    lastUpdateCheckMs = atMillis;
    save();
  }

  void setThemedScopes(bool on) {
    themedScopes = on;
    settingsChanged();
  }

  void setThemedViewerSurround(bool on) {
    themedViewerSurround = on;
    settingsChanged();
  }

  void setSmoothZoomedViewer(bool on) {
    smoothZoomedViewer = on;
    settingsChanged();
  }

  void setPrecomposeSettings({
    required bool moveAttributes,
    required bool adjustDuration,
    required bool openNewComp,
  }) {
    precomposeMoveAttributes = moveAttributes;
    precomposeAdjustDuration = adjustDuration;
    precomposeOpenNewComp = openNewComp;
    settingsChanged();
  }

  void setShape(ThemeShape s) {
    themeShape = s;
    recompose();
    save();
  }

  void setAccent(Color? c) {
    accentOverride = c;
    recompose();
    save();
  }

  void setAnimationLevel(AnimationLevel a) {
    animationLevel = a;
    settingsChanged();
  }

  /// A setting was edited in place: persist it and tell everything drawing
  /// from it. The one notify-and-save funnel — [interface] and [performance]
  /// edits call it directly, and the boolean setters above fold into it.
  void settingsChanged() {
    notifyListeners();
    save();
  }

  void resetWorkspaceLayout() {
    dock = defaultLayout();
    // The default arrangement is Edit's (see `presetLayout`), so the strip
    // ticks Edit rather than nothing after a reset.
    activePreset = WorkspacePreset.edit;
    notifyListeners();
    save();
  }

  /// Rearrange to one of the four shipped presets (docs/07 §1.6). Only the
  /// arrangement changes: no panel closes, reloads or re-evaluates anything.
  void applyWorkspacePreset(WorkspacePreset preset) {
    dock = presetLayout(preset);
    activePreset = preset;
    notifyListeners();
    save();
  }

  /// Which shipped preset the arrangement was last set to, for the toolbar's
  /// workspace strip to tick (docs/07 §1.4).
  ///
  /// Session-only, and not part of the stored layout: what persists is the
  /// arrangement itself, which the user is free to drag about afterwards — so
  /// on the next launch the strip shows no preset ticked rather than claiming
  /// one the panels may no longer match.
  WorkspacePreset? activePreset;

  void touch() => settingsChanged();

  /// Remember the file a project was just opened from or saved to, so the next
  /// launch can reopen it. Persisted immediately; no theme rebuild is needed, so
  /// this does not notify listeners.
  void rememberProject(String path) {
    lastProjectPath = path;
    recentProjects
      ..remove(path)
      ..insert(0, path);
    if (recentProjects.length > maxRecentProjects) {
      recentProjects.removeRange(maxRecentProjects, recentProjects.length);
    }
    save();
  }

  /// The projects opened or saved most recently, newest first — File ▸ Open
  /// recent. Paths only: whether the file is still there is not asked until
  /// someone picks it, because a network drive that is slow to answer must not
  /// hold up a menu opening.
  final List<String> recentProjects = [];

  /// How many the list keeps. Ten is the length every editor settled on: long
  /// enough to reach last week's work, short enough to read at a glance.
  static const int maxRecentProjects = 10;

  /// Remember [session] for the project at [path], persisted immediately so the
  /// next open restores it. A no-op write when the session is unchanged, so the
  /// piggybacked [save] does not churn the store on every identical update.
  void rememberSession(String path, SavedSession session) {
    if (sessions[path] == session) return;
    sessions[path] = session;
    save();
  }

  /// The saved session for the project at [path], or null when none is stored.
  SavedSession? sessionFor(String path) => sessions[path];

  /// Where each floating window was left, keyed by the window's id (K-242).
  final Map<String, WindowPlacement> windowPlacements = {};

  /// Remember where a window was dragged or resized to. Written straight away
  /// like the other deliberate acts here — moving a window happens once in a
  /// while, not per frame — and a no-op when nothing actually changed.
  void rememberWindow(String id, WindowPlacement placement) {
    if (windowPlacements[id] == placement) return;
    windowPlacements[id] = placement;
    save();
  }

  // --- Persistence ---------------------------------------------------------

  /// Somewhere other than the real settings file to read and write — set by
  /// the test harness, null in the application.
  ///
  /// **Why this exists.** A test builds a `Workspace` and any setter on it
  /// calls [save]; without a redirect that wrote *defaults* straight over the
  /// developer's own `%APPDATA%` file, so every `flutter test` run silently
  /// reset their settings. The store is machine state, and a test run must
  /// not be able to reach it.
  static String? storeOverride;

  /// `%APPDATA%\lumit\flutter-workspace.json` on Windows; a dotfolder
  /// fallback elsewhere. No plugin needed, and nothing machine-specific ever
  /// enters the repository.
  static File storeFile() {
    final override = storeOverride;
    if (override != null) return File(override);
    final base = Platform.environment['APPDATA'] ??
        '${Platform.environment['HOME'] ?? '.'}/.config';
    return File('$base${Platform.pathSeparator}lumit'
        '${Platform.pathSeparator}flutter-workspace.json');
  }

  Map<String, dynamic> toJson() => {
        'version': 1,
        'dock': dock.toJson(),
        'color_scheme': colorScheme.name,
        'theme_shape': themeShape.name,
        'accent_override': accentOverride == null
            ? null
            : [
                (accentOverride!.r * 255).round(),
                (accentOverride!.g * 255).round(),
                (accentOverride!.b * 255).round(),
              ],
        'animation_level': animationLevel.name,
        'performance': performance.toJson(),
        'interface': interface.toJson(),
        'first_run_done': firstRunDone,
        'auto_update': autoUpdate,
        'last_update_check_ms': lastUpdateCheckMs,
        'keymap': keymapJson,
        'custom_themes': [for (final t in customThemes) t.toJson()],
        'custom_theme': customThemeName,
        'themed_scopes': themedScopes,
        'themed_viewer_surround': themedViewerSurround,
        'smooth_zoomed_viewer': smoothZoomedViewer,
        'precompose_move_attributes': precomposeMoveAttributes,
        'precompose_adjust_duration': precomposeAdjustDuration,
        'precompose_open_new_comp': precomposeOpenNewComp,
        'last_project_path': lastProjectPath,
        'recent_projects': recentProjects,
        'sessions': {
          for (final e in sessions.entries) e.key: e.value.toJson(),
        },
        'windows': {
          for (final e in windowPlacements.entries) e.key: e.value.toJson(),
        },
      };

  void applyJson(Map<String, dynamic> j) {
    final d = j['dock'];
    if (d is Map<String, dynamic>) {
      final parsed = DockNode.fromJson(d);
      if (parsed is DockSplit) dock = parsed;
    }
    colorScheme = LumitColorScheme.values.asNameMap()[j['color_scheme']] ??
        LumitColorScheme.dark;
    themeShape =
        ThemeShape.values.asNameMap()[j['theme_shape']] ?? ThemeShape.sharp;
    final acc = j['accent_override'];
    accentOverride = acc is List && acc.length == 3
        ? Color.fromARGB(0xff, acc[0] as int, acc[1] as int, acc[2] as int)
        : null;
    animationLevel = AnimationLevel.values.asNameMap()[j['animation_level']] ??
        AnimationLevel.all;
    if (j['performance'] is Map<String, dynamic>) {
      performance = PerformanceSettings.fromJson(j['performance']);
    }
    if (j['interface'] is Map<String, dynamic>) {
      interface = InterfaceSettings.fromJson(j['interface']);
    }
    // Absent means an existing user, not a new one — see the field.
    firstRunDone = j['first_run_done'] as bool? ?? true;
    // Absent means a settings file written before there were updates to check
    // for; the default is on, and an existing user gets the same offer a new
    // one does.
    autoUpdate = j['auto_update'] as bool? ?? true;
    lastUpdateCheckMs = j['last_update_check_ms'] as int? ?? 0;
    keymapJson = j['keymap'] is String ? j['keymap'] as String : null;
    customThemes = [];
    final rawThemes = j['custom_themes'];
    if (rawThemes is List) {
      for (final entry in rawThemes) {
        if (entry is Map) {
          final theme = CustomTheme.fromJson(entry.cast<String, dynamic>());
          if (theme != null) customThemes.add(theme);
        }
      }
    }
    customThemeName =
        j['custom_theme'] is String ? j['custom_theme'] as String : null;
    themedScopes = j['themed_scopes'] == true;
    themedViewerSurround = j['themed_viewer_surround'] == true;
    smoothZoomedViewer = j['smooth_zoomed_viewer'] == true;
    precomposeMoveAttributes = j['precompose_move_attributes'] as bool? ?? true;
    precomposeAdjustDuration = j['precompose_adjust_duration'] as bool? ?? true;
    precomposeOpenNewComp = j['precompose_open_new_comp'] as bool? ?? false;
    lastProjectPath = j['last_project_path'] is String
        ? j['last_project_path'] as String
        : null;
    recentProjects.clear();
    final rawRecent = j['recent_projects'];
    if (rawRecent is List) {
      recentProjects.addAll(rawRecent.whereType<String>());
    }
    sessions.clear();
    final rawSessions = j['sessions'];
    if (rawSessions is Map) {
      rawSessions.forEach((key, value) {
        if (key is String && value is Map) {
          sessions[key] = SavedSession.fromJson(value.cast<String, dynamic>());
        }
      });
    }
    windowPlacements.clear();
    final rawWindows = j['windows'];
    if (rawWindows is Map) {
      rawWindows.forEach((key, value) {
        if (key is String && value is Map) {
          final p = WindowPlacement.fromJson(value.cast<String, dynamic>());
          if (p != null) windowPlacements[key] = p;
        }
      });
    }
    // The left group always opens on Project (activate_panel_tab at start-up).
    activatePanelTab(dock, Panel.project);
    recompose();
  }

  void load() {
    try {
      final f = storeFile();
      if (!f.existsSync()) {
        // Nothing on file for this machine: that, and only that, is a first
        // run (K-246). A corrupt file below is *not* — it belongs to somebody
        // who has used Lumit already, and losing their settings is enough of
        // an insult without being asked to introduce themselves again.
        firstRunDone = false;
        return;
      }
      final j = jsonDecode(f.readAsStringSync());
      if (j is Map<String, dynamic>) applyJson(j);
    } catch (_) {
      // A corrupt store falls back to defaults — never a crash.
    }
  }

  void save() {
    try {
      final f = storeFile();
      f.parent.createSync(recursive: true);
      f.writeAsStringSync(const JsonEncoder.withIndent('  ').convert(toJson()));
    } catch (_) {
      // Persistence is best-effort; the session keeps working without it.
    }
  }
}

/// A choice in the theme picker: one of the built-in schemes, or one of the
/// user's own themes by name (K-202).
class ThemeChoice {
  final LumitColorScheme? scheme;
  final String? customName;

  const ThemeChoice.builtIn(LumitColorScheme this.scheme) : customName = null;
  const ThemeChoice.custom(String this.customName) : scheme = null;

  String get label => scheme?.label ?? customName!;

  /// The heading this choice sits under. Light and dark first because that is
  /// what anyone is choosing by; the user's own last, because they are theirs.
  String get group => scheme == null
      ? l10n.custom
      : scheme!.mode == ThemeMode2.light
          ? l10n.schemeLight
          : l10n.schemeDark;

  @override
  bool operator ==(Object other) =>
      other is ThemeChoice &&
      other.scheme == scheme &&
      other.customName == customName;

  @override
  int get hashCode => Object.hash(scheme, customName);
}
