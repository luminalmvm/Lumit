// The workspace controller: everything the egui `Shell` persists (dock
// layout, colour scheme, shape, accent override, animation level, the
// settings structs), held in one ChangeNotifier and written to a JSON file —
// the Flutter counterpart of eframe's storage (docs/archive/flutter-port/03).

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:ui';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';

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

/// Where the user was in one composition when they last left it (K-624): the
/// playhead frame, the Timeline's magnification, and how far through its
/// scrollable range the lanes were scrolled (0 at the left, 1 at the right —
/// a fraction rather than a pixel offset so the view comes back to the same
/// stretch of time whatever width the panel has since been dragged to).
///
/// A record for the same reason [ViewerLook] is: three numbers with no
/// behaviour, compared by value.
typedef CompView = ({int frame, double zoom, double scroll});

/// A comp nobody has been in yet: frame one, fitted, at the left.
const CompView newCompView = (frame: 0, zoom: 1.0, scroll: 0.0);

/// Which of the Viewer's marks are drawn over one composition (K-416, K-689):
/// the proportional grid, the title/action safe rectangles, and the rulers
/// along the picture's top and left edges.
///
/// A record for [ViewerLook]'s reason — three switches with no behaviour,
/// compared by value, which is what [SavedSession]'s equality needs.
typedef ViewerOverlays = ({bool grid, bool safeAreas, bool rulers});

/// Nothing drawn: what a composition nobody has asked anything of shows.
const ViewerOverlays noViewerOverlays =
    (grid: false, safeAreas: false, rulers: false);

/// One guide dragged out of a ruler (K-689): where it sits in **comp pixels**,
/// and which way it runs.
///
/// [vertical] is the line's own direction, so a vertical guide is a constant
/// *x* and a horizontal one a constant *y* — the same reading Photoshop and
/// After Effects have taught. Comp pixels rather than fractions because that is
/// what the rulers count and what a layer's Position is measured in; a guide at
/// 960 is at 960 whatever the picture is magnified to.
typedef ViewerGuide = ({double at, bool vertical});

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

  /// The region of interest of each composition, by id (K-362, docs/07 §2.2
  /// item 7), as comp fractions `[u0, v0, u1, v1]`. Session state for the same
  /// reason the resolutions are: choosing which corner to work on is a way of
  /// working, not an edit, and it must never reach an export. Comps looking at
  /// the whole frame — the default — are simply absent.
  final Map<String, List<double>> regionsOfInterest;

  /// Which marks the Viewer draws over each composition, by id (K-416, K-689):
  /// the grid, the safe rectangles and the rulers.
  ///
  /// **Session state, and this is where K-416 said it would end up.** Which
  /// scaffolding you want over a shot is a way of looking at it rather than an
  /// edit to it: it never reaches an op, Ctrl+Z never undoes a tick, and no
  /// export has ever seen one. Comps with nothing drawn — the default — are
  /// simply absent.
  final Map<String, ViewerOverlays> viewerOverlays;

  /// The guides dragged out of each composition's rulers, by id (K-689), in
  /// comp pixels. Session state for the overlays' reason, and kept beside them
  /// because a guide is a mark over the picture like any other. Comps with no
  /// guides are simply absent.
  final Map<String, List<ViewerGuide>> guides;

  /// Where the user was in each composition, by id (K-624) — the playhead and
  /// the Timeline's view. Session state for the same reason the looks are:
  /// standing somewhere in a comp is not an edit to it, so it never reaches an
  /// op and Ctrl+Z never undoes a scrub. The comp fronted when the session was
  /// written is also in [frame], which stays the answer for a session written
  /// by a build that had none of this.
  final Map<String, CompView> compViews;

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
    this.regionsOfInterest = const {},
    this.viewerOverlays = const {},
    this.guides = const {},
    this.compViews = const {},
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
        'regions_of_interest': regionsOfInterest,
        'viewer_overlays': {
          for (final e in viewerOverlays.entries)
            e.key: {
              'grid': e.value.grid,
              'safe_areas': e.value.safeAreas,
              'rulers': e.value.rulers,
            },
        },
        'guides': {
          for (final e in guides.entries)
            e.key: [
              for (final g in e.value)
                {'at': g.at, 'vertical': g.vertical},
            ],
        },
        'comp_views': {
          for (final e in compViews.entries)
            e.key: {
              'frame': e.value.frame,
              'zoom': e.value.zoom,
              'scroll': e.value.scroll,
            },
        },
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
        regionsOfInterest: _regionsFromJson(j['regions_of_interest']),
        viewerOverlays: _overlaysFromJson(j['viewer_overlays']),
        guides: _guidesFromJson(j['guides']),
        compViews: _compViewsFromJson(j['comp_views']),
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
      mapEquals(other.viewerOverlays, viewerOverlays) &&
      other._guideKey == _guideKey &&
      other._regionKey == _regionKey &&
      mapEquals(other.compViews, compViews) &&
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
        _regionKey,
        Object.hashAll([
          for (final e in compViews.entries) Object.hash(e.key, e.value),
        ]),
        Object.hashAll([
          for (final e in viewerOverlays.entries) Object.hash(e.key, e.value),
        ]),
        _guideKey,
      );

  /// The guides compared (and hashed) as text, for [_regionKey]'s reason: a map
  /// of *lists* compares by identity, so two equal sets of guides would read as
  /// different and the session would rewrite itself on every frame.
  String get _guideKey => guides.isEmpty
      ? ''
      : jsonEncode({
          for (final e in guides.entries)
            e.key: [
              for (final g in e.value) [g.at, g.vertical],
            ],
        });

  /// The regions compared (and hashed) as text: a map of *lists* compares by
  /// identity under `mapEquals`, so two equal regions would read as different
  /// and the session would rewrite itself on every frame.
  String get _regionKey => regionsOfInterest.isEmpty
      ? ''
      : jsonEncode({
          for (final e in regionsOfInterest.entries) e.key: e.value,
        });
}

/// The per-comp views out of a session's JSON, keeping only entries that carry
/// a frame — a session from another build, or a hand-edited one, leaves the
/// comp at its default rather than stopping the project from opening. The zoom
/// is held at or above 1 (fit-to-panel is as far out as the Timeline goes) and
/// the scroll inside 0..1, so a nonsense number cannot strand the lanes
/// somewhere the user cannot scroll back from.
Map<String, CompView> _compViewsFromJson(Object? raw) {
  if (raw is! Map) return const {};
  final out = <String, CompView>{};
  for (final e in raw.entries) {
    final k = e.key;
    final v = e.value;
    if (k is! String || v is! Map || v['frame'] is! num) continue;
    final zoom = v['zoom'] is num ? (v['zoom'] as num).toDouble() : 1.0;
    final scroll = v['scroll'] is num ? (v['scroll'] as num).toDouble() : 0.0;
    if (!zoom.isFinite || !scroll.isFinite) continue;
    out[k] = (
      frame: max(0, (v['frame'] as num).toInt()),
      zoom: zoom < 1.0 ? 1.0 : zoom,
      scroll: scroll.clamp(0.0, 1.0),
    );
  }
  return out;
}

/// The per-comp overlays out of a session's JSON (K-689). A missing or
/// malformed switch reads as off, which is what a session written by a build
/// that had fewer of them gets: the marks it did ask for, and nothing invented.
Map<String, ViewerOverlays> _overlaysFromJson(Object? raw) {
  if (raw is! Map) return const {};
  final out = <String, ViewerOverlays>{};
  for (final e in raw.entries) {
    final k = e.key;
    final v = e.value;
    if (k is! String || v is! Map) continue;
    final overlays = (
      grid: v['grid'] == true,
      safeAreas: v['safe_areas'] == true,
      rulers: v['rulers'] == true,
    );
    if (overlays != noViewerOverlays) out[k] = overlays;
  }
  return out;
}

/// The per-comp guides out of a session's JSON (K-689), keeping only entries
/// that are a finite position — a hand-edited or truncated session leaves the
/// comp with the guides that did read, and an app that opens.
Map<String, List<ViewerGuide>> _guidesFromJson(Object? raw) {
  if (raw is! Map) return const {};
  final out = <String, List<ViewerGuide>>{};
  for (final e in raw.entries) {
    final k = e.key;
    final v = e.value;
    if (k is! String || v is! List) continue;
    final lines = <ViewerGuide>[
      for (final g in v)
        if (g is Map && g['at'] is num && (g['at'] as num).toDouble().isFinite)
          (at: (g['at'] as num).toDouble(), vertical: g['vertical'] == true),
    ];
    if (lines.isNotEmpty) out[k] = lines;
  }
  return out;
}

/// The per-comp regions out of a session's JSON, keeping only entries that are
/// four finite numbers. Anything else is no region, which is also what a
/// hand-edited or truncated session file gets: the whole frame, and an app that
/// opens.
Map<String, List<double>> _regionsFromJson(Object? raw) {
  if (raw is! Map) return const {};
  final out = <String, List<double>>{};
  for (final e in raw.entries) {
    final k = e.key;
    final v = e.value;
    if (k is! String || v is! List || v.length != 4) continue;
    final nums = [
      for (final n in v)
        if (n is num) n.toDouble()
    ];
    if (nums.length == 4 && nums.every((n) => n.isFinite)) out[k] = nums;
  }
  return out;
}

/// The extension one of the user's own workspaces is written under (docs/07
/// §1.4). Lumit's own rather than a plain `.json`, so the picker can offer just
/// workspaces — the same reasoning as the shared theme's.
const String workspaceFileExtension = 'lumworkspace';

/// What the file says it is. Checked on read, so a stray `.json` renamed to
/// `.lumworkspace` is refused rather than half-loaded.
const String workspaceFileFormat = 'lumit-workspace';

/// One of the user's own saved arrangements (docs/07 §1.4): a name and the
/// panel tree under it.
///
/// **Stored per user, never in the project** — one human-readable file each in
/// `workspaces/` beside the settings, so a workspace can be sent to somebody.
/// The stored file and the exported file are the same document, which is what
/// makes *Export* a copy rather than a second format to keep in step.
///
/// The tree is held as raw JSON for the same reason [SavedSession.dock] is: a
/// dock tree is mutated in place as panels are dragged, so a parsed one would
/// quietly change under the name that saved it.
class UserWorkspace {
  final String name;
  final Map<String, dynamic> dock;

  const UserWorkspace(this.name, this.dock);

  Map<String, dynamic> toJson() => {
        'format': workspaceFileFormat,
        'version': 1,
        'name': name,
        'dock': dock,
      };

  /// The document as text — indented, with a trailing newline, because this is
  /// a file people are meant to be able to read and diff.
  String encode() =>
      '${const JsonEncoder.withIndent('  ').convert(toJson())}\n';

  /// Read one back, or null when [raw] is not a workspace at all. Never
  /// throws: picking the wrong file is a normal thing to do, and the caller
  /// says so in a sentence rather than falling over.
  static UserWorkspace? fromJson(Object? raw) {
    if (raw is! Map) return null;
    final format = raw['format'];
    if (format is String && format != workspaceFileFormat) return null;
    final name = raw['name'];
    final dock = raw['dock'];
    if (name is! String || name.trim().isEmpty || dock is! Map) return null;
    final tree = dock.cast<String, dynamic>();
    // An arrangement that does not parse is not an arrangement. Checked here
    // rather than on use, so a broken file is refused at the door instead of
    // becoming a workspace that does nothing when it is picked.
    try {
      if (DockNode.fromJson(tree) is! DockSplit) return null;
    } catch (_) {
      // `DockNode.fromJson` throws on a node it does not recognise, which a
      // hand-edited or truncated file is full of. Refused, not raised.
      return null;
    }
    return UserWorkspace(name.trim(), tree);
  }
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

  /// The effects and presets the owner has starred in Effects & presets
  /// (owner, desk test), by the panel's own key — an effect's match name, or
  /// `preset:` and the preset's name.
  ///
  /// Here rather than in the panel because a favourite is a preference, not a
  /// view state: it has to survive the panel being closed, the workspace being
  /// switched and the application being restarted, which is exactly what this
  /// file is for. Opaque keys, so the panel decides what may be starred
  /// without this file learning about effect schemas.
  final Set<String> favouriteEffects = <String>{};

  bool isFavouriteEffect(String key) => favouriteEffects.contains(key);

  /// Star it, or take the star off again.
  void toggleFavouriteEffect(String key) {
    if (!favouriteEffects.remove(key)) favouriteEffects.add(key);
    settingsChanged();
  }

  /// Whether an effect's own graph — Levels' histogram, a Curves channel —
  /// draws entirely in the theme's colours (owner, desk test). Off by default,
  /// and for the same reason the scopes toggle is: a red curve should be red.
  /// With it off the *channel* views take the standard R, G and B, and only
  /// Master takes the theme colour; with it on the whole graph is themed.
  bool themedEffectGraphs = false;

  /// The side of a Curves plot, in logical pixels — the graph-size option After
  /// Effects offers as small, medium and large (item 6.32). A preference rather
  /// than a view state, for the reason the favourites are: the size someone
  /// grades at is theirs, and having to set it again every session is the
  /// annoyance the option exists to remove.
  double curvePlotSize = 150;

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

  /// Whether the welcome screen opens on launch (K-481).
  ///
  /// On by default. Off is for somebody who always starts from the same
  /// project, or simply does not want to be asked: Lumit then opens straight
  /// into the shell, whose Viewer offers the same three ways to start until
  /// something is displayed, so turning this off never hides a choice.
  bool showWelcomeOnLaunch = true;

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

  /// The output the user chose to hear Lumit through, by the engine's own id,
  /// or null to follow whatever the machine plays through (K-586, docs/09 §3.1).
  ///
  /// A sound card is a property of the machine, not of the work, so it lives in
  /// the settings file rather than in a project — and it is stored by id rather
  /// than by position in the list, because a device unplugged and plugged back
  /// in does not come back in the same place. The frontend never interprets it:
  /// it hands the id to the engine on boot and on every change, and the engine
  /// decides what to do when the device is not there any more.
  String? audioDevice;

  /// Choose the output and write the file. A deliberate act, like the keymap,
  /// so it is saved straight away rather than batched.
  void setAudioDevice(String? id) {
    audioDevice = id;
    save();
  }

  /// How often Lumit writes a spare copy of every open project, in minutes,
  /// and how many copies it keeps (docs/10 §4). Zero minutes is off.
  ///
  /// Application settings rather than project data — how often this machine
  /// copies your work is a property of the machine — so they live here and are
  /// handed to the engine at boot and on every change. The timer itself is the
  /// engine's, because the document is: the frontend never decides when a copy
  /// is due, and never asks whether one is.
  int autosaveMinutes = 5;
  int autosaveKeep = 5;

  /// Set the cadence and write the file. A deliberate act, like the keymap.
  void setAutosave(int minutes, int keep) {
    autosaveMinutes = minutes;
    autosaveKeep = keep;
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
    // Density rides on every theme this method can build, preview included:
    // it is a setting about rows rather than about colours, so no colour
    // choice — a scheme, a custom theme, a live preview — gets to lose it.
    final density = DensityTokens.of(interface.compact);
    if (_preview != null) {
      _theme = _preview!.copyWith(density: density);
      notifyListeners();
      return;
    }
    final custom = activeCustomTheme;
    if (custom != null) {
      // A custom theme carries its own accent among its colours, so the
      // accent override does not apply on top — it would silently overwrite
      // a choice the user made in the editor.
      _theme = custom.build(themeShape).copyWith(density: density);
    } else {
      _theme = LumitTheme.forScheme(
        colorScheme,
        themeShape,
        accentOverride: accentOverride,
      ).copyWith(density: density);
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

  /// Turn the welcome screen on or off for the next launch (K-481).
  void setShowWelcomeOnLaunch(bool on) {
    showWelcomeOnLaunch = on;
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

  void setThemedEffectGraphs(bool on) {
    themedEffectGraphs = on;
    settingsChanged();
  }

  void setCurvePlotSize(double pixels) {
    curvePlotSize = pixels;
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
    activeUserWorkspace = null;
    notifyListeners();
    save();
  }

  /// Rearrange to one of the four shipped presets (docs/07 §1.6). Only the
  /// arrangement changes: no panel closes, reloads or re-evaluates anything.
  void applyWorkspacePreset(WorkspacePreset preset) {
    dock = presetLayout(preset);
    activePreset = preset;
    activeUserWorkspace = null;
    notifyListeners();
    save();
    presetApplied.value++;
  }

  /// Bumped every time a shipped preset is applied — pressing the strip's word
  /// a second time included. Its own notifier rather than [notifyListeners]
  /// because this class notifies for a theme edit too, and a listener acting
  /// on "the user asked for this arrangement" must not fire on those; and
  /// because re-applying the preset already in force notifies nobody else.
  ///
  /// The Timeline is the listener (K-728): the Audio arrangement's board draws
  /// the sound lanes open, which a dock tree cannot say.
  final ValueNotifier<int> presetApplied = ValueNotifier<int>(0);

  /// Which shipped preset the arrangement was last set to, for the toolbar's
  /// workspace strip to tick (docs/07 §1.4).
  ///
  /// Session-only, and not part of the stored layout: what persists is the
  /// arrangement itself, which the user is free to drag about afterwards — so
  /// on the next launch the strip shows no preset ticked rather than claiming
  /// one the panels may no longer match.
  WorkspacePreset? activePreset;

  // --- The user's own workspaces (docs/07 §1.4) ----------------------------

  /// The arrangements the user has saved, in name order — which is the order
  /// the strip lists them in, after the shipped presets.
  ///
  /// Name order rather than the order they were saved in, because the store is
  /// a folder of files: an insertion order would have to be a number written
  /// into each of them, kept in step across rename, delete and import, to
  /// answer a question the alphabet already answers. Name order also means
  /// `Alt+Shift+7` reaches the same workspace on the next launch.
  final List<UserWorkspace> userWorkspaces = [];

  /// Which of the user's own the arrangement was last set to, for the strip to
  /// tick and for a drag to be written back to.
  ///
  /// Session-only, exactly as [activePreset] is and for the same reason: what
  /// persists is the arrangement itself, which the user is free to drag about,
  /// so a name ticked after a restart could claim a layout the panels no
  /// longer match.
  String? activeUserWorkspace;

  /// The folder the user's workspaces live in — `%APPDATA%\lumit\workspaces`
  /// in the application, a folder under [storeOverride] in a test.
  static Directory userWorkspaceDir() => Directory(
        '${storeFile().parent.path}${Platform.pathSeparator}workspaces',
      );

  /// The file [name] is kept in. The name is escaped rather than replaced, so
  /// two workspaces whose names differ only in punctuation cannot land on one
  /// file — and an ordinary name still reads as itself in the folder.
  static File userWorkspaceFile(String name) => File(
        '${userWorkspaceDir().path}${Platform.pathSeparator}'
        '${Uri.encodeComponent(name).replaceAll('%20', ' ')}'
        '.$workspaceFileExtension',
      );

  /// Read the folder. Best-effort throughout: one unreadable file costs its
  /// workspace, never the launch.
  void loadUserWorkspaces() {
    userWorkspaces.clear();
    try {
      final dir = userWorkspaceDir();
      if (!dir.existsSync()) return;
      for (final entry in dir.listSync()) {
        if (entry is! File) continue;
        try {
          final read = UserWorkspace.fromJson(
            jsonDecode(entry.readAsStringSync()),
          );
          if (read != null && !userWorkspaces.any((w) => w.name == read.name)) {
            userWorkspaces.add(read);
          }
        } catch (_) {}
      }
      _sortUserWorkspaces();
    } catch (_) {}
  }

  void _sortUserWorkspaces() =>
      userWorkspaces.sort((a, b) => a.name.compareTo(b.name));

  static void _writeUserWorkspace(UserWorkspace w) {
    try {
      final f = userWorkspaceFile(w.name);
      f.parent.createSync(recursive: true);
      f.writeAsStringSync(w.encode());
    } catch (_) {}
  }

  /// A name none of the user's workspaces holds: [wanted] when it is free,
  /// else the same with a number after it. The name is the identity here — the
  /// strip shows it and the store files by it — so every route that adds one
  /// comes through here rather than overwriting somebody's arrangement.
  String availableWorkspaceName(String wanted) {
    final base = wanted.trim();
    var tried = base;
    for (var n = 2; userWorkspaces.any((w) => w.name == tried); n++) {
      tried = '$base $n';
    }
    return tried;
  }

  /// Save the arrangement on screen under a name of the user's own (*Save as
  /// new workspace…*) and switch to it. Returns the name it landed under,
  /// which differs from [wanted] when that was taken.
  String saveWorkspaceAs(String wanted) {
    final saved = UserWorkspace(availableWorkspaceName(wanted), dock.toJson());
    userWorkspaces.add(saved);
    _sortUserWorkspaces();
    _writeUserWorkspace(saved);
    activePreset = null;
    activeUserWorkspace = saved.name;
    notifyListeners();
    return saved.name;
  }

  /// Rearrange to one of the user's own. Only the arrangement changes, exactly
  /// as for a preset: nothing closes, reloads or re-evaluates.
  void applyUserWorkspace(String name) {
    final at = userWorkspaces.indexWhere((w) => w.name == name);
    if (at < 0) return;
    final parsed = DockNode.fromJson(userWorkspaces[at].dock);
    if (parsed is! DockSplit) return;
    dock = parsed;
    activePreset = null;
    activeUserWorkspace = name;
    notifyListeners();
    save();
  }

  /// Rename one of the user's own, keeping the selection on it. Returns the
  /// name it now has — [to] when that was free, else [to] with a number after
  /// it — or null when [from] is not one of them.
  String? renameUserWorkspace(String from, String to) {
    final at = userWorkspaces.indexWhere((w) => w.name == from);
    if (at < 0) return null;
    final wanted = to.trim();
    if (wanted.isEmpty || wanted == from) return from;
    final renamed =
        UserWorkspace(availableWorkspaceName(wanted), userWorkspaces[at].dock);
    userWorkspaces[at] = renamed;
    _sortUserWorkspaces();
    _deleteUserWorkspaceFile(from);
    _writeUserWorkspace(renamed);
    if (activeUserWorkspace == from) activeUserWorkspace = renamed.name;
    notifyListeners();
    return renamed.name;
  }

  /// Forget one of the user's own. The arrangement on screen stays as it is —
  /// deleting the name it was saved under is not a reason to move the panels.
  void deleteUserWorkspace(String name) {
    if (!userWorkspaces.any((w) => w.name == name)) return;
    userWorkspaces.removeWhere((w) => w.name == name);
    _deleteUserWorkspaceFile(name);
    if (activeUserWorkspace == name) activeUserWorkspace = null;
    notifyListeners();
  }

  static void _deleteUserWorkspaceFile(String name) {
    try {
      final f = userWorkspaceFile(name);
      if (f.existsSync()) f.deleteSync();
    } catch (_) {}
  }

  /// Take an imported workspace in. Returns the name it landed under, which
  /// differs from the file's when one of the user's own already had it — an
  /// import never overwrites an arrangement they made.
  String importUserWorkspace(UserWorkspace imported) {
    final landed =
        UserWorkspace(availableWorkspaceName(imported.name), imported.dock);
    userWorkspaces.add(landed);
    _sortUserWorkspaces();
    _writeUserWorkspace(landed);
    notifyListeners();
    return landed.name;
  }

  /// The workspace in strip slot [slot], counting the shipped presets first
  /// and the user's own after them — what `Alt+Shift+1…9` switches by (docs/07
  /// §1.4, §15). Answers whether there was one, so a chord pointing past the
  /// end of the strip falls through to whatever else wants it rather than
  /// appearing to do nothing.
  bool switchToWorkspaceSlot(int slot) {
    const presets = WorkspacePreset.values;
    if (slot < 1) return false;
    if (slot <= presets.length) {
      applyWorkspacePreset(presets[slot - 1]);
      return true;
    }
    final at = slot - presets.length - 1;
    if (at >= userWorkspaces.length) return false;
    applyUserWorkspace(userWorkspaces[at].name);
    return true;
  }

  /// Which strip slot the arrangement in force occupies, or null when it is
  /// not one of the workspaces on the strip — the inverse of
  /// [switchToWorkspaceSlot], and what Window ▸ Assign shortcut binds.
  ///
  /// **The chord follows the slot, not the name** (K-574). Slots count the
  /// presets first and then the user's own in name order, so a workspace
  /// renamed past one of its neighbours changes slot with it. That is what
  /// makes `Alt+Shift+7` reach the same *place* on the strip every launch, and
  /// it is what the dialogue says out loud.
  int? get activeWorkspaceSlot {
    const presets = WorkspacePreset.values;
    if (activePreset case final preset?) return presets.indexOf(preset) + 1;
    final active = activeUserWorkspace;
    if (active == null) return null;
    final at = userWorkspaces.indexWhere((w) => w.name == active);
    return at < 0 ? null : presets.length + at + 1;
  }

  /// The arrangement moved: write it back to the user workspace in force, if
  /// there is one (docs/07 §1.4 — layout changes persist automatically to the
  /// active workspace). A no-op under a preset, whose factory layout is not the
  /// user's to overwrite.
  void rememberActiveWorkspaceLayout() {
    final active = activeUserWorkspace;
    if (active == null) return;
    final at = userWorkspaces.indexWhere((w) => w.name == active);
    if (at < 0) return;
    final updated = UserWorkspace(active, dock.toJson());
    userWorkspaces[at] = updated;
    _writeUserWorkspace(updated);
  }

  void touch() {
    rememberActiveWorkspaceLayout();
    settingsChanged();
  }

  /// Remember the file a project was just opened from or saved to, so the next
  /// launch can reopen it. Persisted immediately; no theme rebuild is needed, so
  /// this does not notify listeners.
  void rememberProject(String path) {
    lastProjectPath = path;
    recentProjects
      ..remove(path)
      ..insert(0, path);
    _recentOpened[path] = DateTime.now().toIso8601String();
    if (recentProjects.length > maxRecentProjects) {
      recentProjects.removeRange(maxRecentProjects, recentProjects.length);
    }
    _pruneRecentOpened();
    save();
  }

  /// The projects opened or saved most recently, newest first — File ▸ Open
  /// recent, and the welcome screen's list. Paths only: whether the file is
  /// still there is not asked until someone picks it, because a network drive
  /// that is slow to answer must not hold up a menu opening.
  final List<String> recentProjects = [];

  /// How many the list keeps. Ten is the length every editor settled on: long
  /// enough to reach last week's work, short enough to read at a glance.
  static const int maxRecentProjects = 10;

  /// When each remembered project was last opened **here**, ISO-8601 by path.
  ///
  /// Kept beside the list rather than inside it so nothing that reads
  /// [recentProjects] has to change. It is this machine's own record of when
  /// the user last had the project open — not the file's timestamp, which would
  /// cost a `stat` per row on a network drive to answer a question about
  /// *their* work rather than the file's.
  final Map<String, String> _recentOpened = {};

  /// When [path] was last opened here, or null for a project remembered before
  /// this was recorded (or never opened on this machine).
  DateTime? recentOpenedAt(String path) =>
      DateTime.tryParse(_recentOpened[path] ?? '');

  /// Forget one project — the × at the end of its row on the welcome screen.
  /// No question asked: the file is untouched and File ▸ Open brings it back.
  void forgetProject(String path) {
    if (!recentProjects.remove(path)) return;
    _recentOpened.remove(path);
    notifyListeners();
    save();
  }

  /// Forget the lot — the Clear beside the welcome screen's Recent heading.
  void clearRecentProjects() {
    if (recentProjects.isEmpty) return;
    recentProjects.clear();
    _recentOpened.clear();
    notifyListeners();
    save();
  }

  /// Drop the stamps of projects that have fallen off the end of the list, so
  /// the store does not grow a timestamp for every project ever opened.
  void _pruneRecentOpened() =>
      _recentOpened.removeWhere((path, _) => !recentProjects.contains(path));

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
        'show_welcome_on_launch': showWelcomeOnLaunch,
        'last_update_check_ms': lastUpdateCheckMs,
        'keymap': keymapJson,
        'audio_device': audioDevice,
        'autosave_minutes': autosaveMinutes,
        'autosave_keep': autosaveKeep,
        'custom_themes': [for (final t in customThemes) t.toJson()],
        'custom_theme': customThemeName,
        'themed_scopes': themedScopes,
        'favourite_effects': favouriteEffects.toList()..sort(),
        'themed_effect_graphs': themedEffectGraphs,
        'curve_plot_size': curvePlotSize,
        'themed_viewer_surround': themedViewerSurround,
        'smooth_zoomed_viewer': smoothZoomedViewer,
        'precompose_move_attributes': precomposeMoveAttributes,
        'precompose_adjust_duration': precomposeAdjustDuration,
        'precompose_open_new_comp': precomposeOpenNewComp,
        'last_project_path': lastProjectPath,
        'recent_projects': recentProjects,
        'recent_opened': _recentOpened,
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
    // Absent means a settings file written before the screen could be turned
    // off, and the answer for those is the same as for a new user: show it.
    showWelcomeOnLaunch = j['show_welcome_on_launch'] as bool? ?? true;
    lastUpdateCheckMs = j['last_update_check_ms'] as int? ?? 0;
    keymapJson = j['keymap'] is String ? j['keymap'] as String : null;
    // Absent, or empty from a hand-edited file, means follow the machine —
    // which is what every settings file written before this field existed was
    // already doing.
    final device = j['audio_device'];
    audioDevice = device is String && device.isNotEmpty ? device : null;
    // Absent means the shipped cadence, which is what a file written before the
    // Autosave page existed was already getting. Zero minutes is off and is
    // kept; a negative number is a hand-edited file and reads as off too, since
    // there is no sense in which it could mean anything else.
    autosaveMinutes = (j['autosave_minutes'] as int? ?? 5).clamp(0, 240);
    autosaveKeep = (j['autosave_keep'] as int? ?? 5).clamp(1, 50);
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
    favouriteEffects
      ..clear()
      ..addAll([
        if (j['favourite_effects'] case final List<dynamic> starred)
          for (final key in starred)
            if (key is String) key,
      ]);
    themedEffectGraphs = j['themed_effect_graphs'] == true;
    // Absent means a file written before the size could be chosen: medium.
    curvePlotSize = (j['curve_plot_size'] as num?)?.toDouble() ?? 150;
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
    // Absent for a settings file written before the welcome screen recorded
    // these: the dates simply read blank until each project is next opened.
    _recentOpened.clear();
    final rawOpened = j['recent_opened'];
    if (rawOpened is Map) {
      rawOpened.forEach((key, value) {
        if (key is String && value is String) _recentOpened[key] = value;
      });
      _pruneRecentOpened();
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
    // The user's own workspaces are their own files beside the settings, so
    // they are read whether or not there is a settings file to read.
    loadUserWorkspaces();
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

  // --- Project thumbnails ---------------------------------------------------
  //
  // The picture of a project as it looked when it was last saved (K-468), shown
  // on the welcome screen's recent rows.
  //
  // **It lives beside the settings file, not inside the `.lum`.** It is this
  // machine's snapshot of the user's own work, rewritten on every save, and a
  // project handed to somebody else has no business carrying a still of the
  // sender's screen into their copy. Beside the store also means the test
  // redirect above covers it for nothing: a `flutter test` run writes its
  // thumbnails into the same scratch folder its settings go to.

  /// How many pixels across a project thumbnail is captured at, whichever road
  /// takes it.
  ///
  /// The welcome row draws it 64 wide, so this is that at 200 % and no more: a
  /// picture nobody will ever see at full size is bytes on somebody's disk,
  /// milliseconds on every save, and — on the engine's road — pixels crossing
  /// the bridge that nothing will look at.
  static const int projectThumbnailPixels = 128;

  /// The folder the thumbnails live in — `%APPDATA%\lumit\thumbnails` in the
  /// application, the scratch folder under [storeOverride] in a test.
  static Directory thumbnailDir() => Directory(
        '${storeFile().parent.path}${Platform.pathSeparator}thumbnails',
      );

  /// The key a project's thumbnail is filed under: a digest of its path, so no
  /// part of a user's folder names reaches the file system here and two
  /// projects both called `Untitled.lum` keep their own picture.
  ///
  /// The path is folded to one spelling first — back-slashes forward, and lower
  /// case on Windows, where `C:\Work\a.lum` and `c:/work/A.LUM` are one file —
  /// so a project saved through the picker and the same project listed on a
  /// recent row land on the same key rather than on two.
  static String thumbnailKey(String path) {
    final forward = path.replaceAll('\\', '/');
    final folded = Platform.isWindows ? forward.toLowerCase() : forward;
    return sha1.convert(utf8.encode(folded)).toString();
  }

  /// Where the project at [path] keeps its thumbnail. The file may not exist:
  /// a project not saved since the feature landed, or one whose picture failed
  /// to capture, simply has none, and the row draws its placeholder.
  static File thumbnailFile(String path) => File(
        '${thumbnailDir().path}${Platform.pathSeparator}'
        '${thumbnailKey(path)}.png',
      );

  /// File [png] as the project's thumbnail, replacing whatever was there.
  ///
  /// Best-effort, like every other write in this class: a read-only appdata
  /// folder or a full disk costs the user a placeholder on one row, and must
  /// never cost them a save.
  static void writeThumbnail(String path, Uint8List png) {
    try {
      final f = thumbnailFile(path);
      f.parent.createSync(recursive: true);
      f.writeAsBytesSync(png);
    } catch (_) {}
  }

  /// The engine's own picture of [comp] at [frame] as a PNG — the road that
  /// needs no Viewer.
  ///
  /// K-468 filed the picture by photographing the Viewer widget, because a
  /// composition frame had no way of reaching Dart as pixels at all. It has one
  /// now (`CompositionReference.thumbnail`), and this is the seam every save
  /// road falls back to when there is no Viewer to photograph: a headless save,
  /// an After Effects conversion, a workspace with the panel closed, a project
  /// being opened for the first time since it grew a picture.
  ///
  /// **A field rather than a function**, so a test can put its own picture here:
  /// no widget test has a graphics adapter to render one on, and the wiring is
  /// what those tests are about.
  static Future<Uint8List?> Function(CompositionReference comp, int frame)
      compThumbnailPng = _compThumbnailPng;

  static Future<Uint8List?> _compThumbnailPng(
      CompositionReference comp, int frame) async {
    final still = await comp.thumbnail(
        frame: BigInt.from(frame), maxEdge: projectThumbnailPixels);
    if (still == null || still.width == 0 || still.height == 0) return null;
    // RGBA in, PNG out, both through `dart:ui` — the same encoder the Viewer's
    // own photograph goes through, so the two roads file the same kind of file.
    final decoded = Completer<Image>();
    decodeImageFromPixels(still.rgba, still.width, still.height,
        PixelFormat.rgba8888, decoded.complete);
    final image = await decoded.future;
    try {
      final png = await image.toByteData(format: ImageByteFormat.png);
      return png?.buffer.asUint8List();
    } finally {
      image.dispose();
    }
  }

  /// File the engine's picture of [comp] at [frame] as the thumbnail of the
  /// project at [path]. Best-effort throughout: a machine with no graphics
  /// adapter, a comp that will not draw and an unwritable folder all cost one
  /// row its picture, which is a state the row is built for.
  static Future<void> fileCompThumbnail(String path, CompositionReference comp,
      {int frame = 0}) async {
    try {
      final png = await compThumbnailPng(comp, frame);
      if (png != null) writeThumbnail(path, png);
    } catch (_) {}
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
