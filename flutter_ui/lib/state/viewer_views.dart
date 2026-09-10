// The Viewer's views: what each picture on screen is showing, and how it is
// being looked at (docs/impl/multi-viewer.md §1).
//
// In plain terms: the Viewer used to be one picture of one composition, so
// "which composition" had one answer for the whole application. It has several
// now. A **view** is one picture surface. It is bound to one item, it can be
// locked so that opening something else does not steal it, and it carries its
// own magnification, channel, exposure and the rest. A **Viewer panel** holds
// one, two or four of them in a layout; the panel is what the dock knows about,
// and the views live inside it.
//
// Two files describe a view and this is the join between them. The workspace
// says which views exist and how they are laid out, because that is the panel
// arrangement and it is the user's. The project says what each view is showing
// and whether it is locked, because those are references to project content and
// a workspace is a file people send each other (docs/07 §1.5). The id below is
// what lets the two agree, so it is a uuid minted once and never an index: an
// index would silently re-point a lock at a different picture the moment a view
// was closed.

import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:uuid/uuid.dart';

import 'dock.dart';
import 'viewer_view.dart' show ViewerChannel;
import 'workspace.dart' show ViewerLook, neutralLook;

/// Which of the three display modes a view is in (docs/07 §2.1).
enum ViewMode {
  /// The rendered composite at the playhead.
  composition,

  /// A project footage item with its interpretation applied.
  footage,

  /// One layer's source before transform.
  layer,
}

/// How the views inside one Viewer panel are laid out (docs/07 §2, and After
/// Effects' own view-layout menu).
enum ViewLayout {
  one,
  twoAcross,
  twoDown,
  four;

  /// How many views this layout shows.
  int get views => switch (this) {
        ViewLayout.one => 1,
        ViewLayout.twoAcross => 2,
        ViewLayout.twoDown => 2,
        ViewLayout.four => 4,
      };

  bool get horizontal => this != ViewLayout.twoDown;
}

/// Which way a compare puts two views together (docs/07 §2, item 10 of the
/// multi-viewer note).
enum CompareMode {
  /// Off: the views are side by side in their own panes as usual.
  none,

  /// One picture, cut at the divider, both halves in register.
  wipe,

  /// The two pictures side by side inside one pane, each squeezed to its half.
  split,
}

/// One picture surface.
///
/// Everything here that is *how you are looking* belongs to the view: two
/// views exist so they can be looked at differently, and a per-composition
/// magnification would make a two-up compare impossible. What stays per
/// composition is what is true about the shot rather than about the pane it is
/// in: the preview resolution, the guides and the rulers (docs/07 §2.2 items 2
/// and 6, unchanged).
class ViewerSurface {
  /// Stable for the life of the view, minted when it is made, written into
  /// both the workspace and the project.
  final String id;

  /// The small number the engine knows this view by, on the render requests
  /// and the frames that come back (docs/impl/multi-viewer.md §2.1). Minted
  /// per open project; the uuid above is what persists.
  final int engineId;

  ViewMode mode;

  /// The composition this view shows, or the composition a layer view's layer
  /// belongs to. Null for a footage view and for a view showing nothing.
  String? compId;

  /// The footage item a footage view shows, or the layer a layer view shows.
  String? itemId;

  /// Locked to what it is showing: opening something else must not steal it
  /// (docs/07 §2.6).
  bool locked;

  /// Where a footage view stands in its **source**, counted in the item's own
  /// frames (docs/impl/multi-viewer.md §3.6). The transport belongs to the
  /// composition, so a view of a clip keeps its own place in it.
  ///
  /// Not written down anywhere: where a clip was left mid-scrub is not project
  /// content, and a view opens at its start.
  int sourceFrame = 0;

  // --- How this view is looking at it ------------------------------------

  /// Null means "fit", which is a rule rather than a number and has to be
  /// re-resolved every time the pane is resized.
  double? magnification;
  double panX;
  double panY;
  ViewerChannel channel;
  bool transparencyBoard;
  ViewerLook look;

  /// The region of interest as comp fractions `[u0, v0, u1, v1]`, or null for
  /// the whole frame.
  List<double>? region;

  /// The OCIO display and view, or null for the built-in transform.
  List<String>? colourView;
  bool wireframe;
  bool layerControls;

  ViewerSurface({
    required this.id,
    required this.engineId,
    this.mode = ViewMode.composition,
    this.compId,
    this.itemId,
    this.locked = false,
    this.magnification,
    this.panX = 0,
    this.panY = 0,
    this.channel = ViewerChannel.rgb,
    this.transparencyBoard = false,
    this.look = neutralLook,
    this.region,
    this.colourView,
    this.wireframe = false,
    this.layerControls = true,
  });

  /// What the project remembers about this view. The display half rides in the
  /// workspace beside the layout; this is the half that is a reference to
  /// project content.
  Map<String, dynamic> toJson() => {
        'id': id,
        'mode': mode.name,
        if (compId != null) 'comp': compId,
        if (itemId != null) 'item': itemId,
        if (locked) 'locked': true,
        if (magnification != null) 'zoom': magnification,
        if (panX != 0) 'pan_x': panX,
        if (panY != 0) 'pan_y': panY,
        if (channel != ViewerChannel.rgb) 'channel': channel.name,
        if (transparencyBoard) 'board': true,
        if (look != neutralLook)
          'look': {'stops': look.stops, 'tone_map': look.toneMap},
        if (region != null) 'region': region,
        if (colourView != null) 'colour_view': colourView,
        if (wireframe) 'wireframe': true,
        if (!layerControls) 'layer_controls': false,
      };

  /// Read one back. Anything malformed reads as its default rather than
  /// failing the load: a project written by another build must open.
  static ViewerSurface? fromJson(Map<String, dynamic> j, int engineId) {
    final id = j['id'];
    if (id is! String || id.isEmpty) return null;
    final look = j['look'];
    return ViewerSurface(
      id: id,
      engineId: engineId,
      mode: ViewMode.values.asNameMap()[j['mode']] ?? ViewMode.composition,
      compId: j['comp'] is String ? j['comp'] as String : null,
      itemId: j['item'] is String ? j['item'] as String : null,
      locked: j['locked'] == true,
      magnification: _finite(j['zoom']),
      panX: _finite(j['pan_x']) ?? 0,
      panY: _finite(j['pan_y']) ?? 0,
      channel:
          ViewerChannel.values.asNameMap()[j['channel']] ?? ViewerChannel.rgb,
      transparencyBoard: j['board'] == true,
      look: look is Map
          ? (
              stops: _finite(look['stops']) ?? 0.0,
              toneMap: look['tone_map'] == true,
            )
          : neutralLook,
      region: _fractions(j['region']),
      colourView: _pair(j['colour_view']),
      wireframe: j['wireframe'] == true,
      layerControls: j['layer_controls'] != false,
    );
  }

  static double? _finite(Object? raw) =>
      raw is num && raw.isFinite ? raw.toDouble() : null;

  static List<double>? _fractions(Object? raw) {
    if (raw is! List || raw.length != 4) return null;
    final out = <double>[];
    for (final v in raw) {
      final f = _finite(v);
      if (f == null) return null;
      out.add(f);
    }
    return out;
  }

  static List<String>? _pair(Object? raw) =>
      raw is List && raw.length == 2 && raw.every((e) => e is String)
          ? [raw[0] as String, raw[1] as String]
          : null;
}

/// Every view, which panes hold them, and which one is active.
///
/// A [ChangeNotifier] rather than part of `LumitUiState`'s own notification,
/// because fronting a view is a change the panels that follow it care about and
/// nothing else does.
class ViewerViews extends ChangeNotifier {
  static const _uuid = Uuid();

  final List<ViewerSurface> views = [];

  /// Which views each Viewer pane holds, in the order they are laid out, and
  /// how. Workspace state: it is the panel arrangement.
  final Map<PaneId, List<String>> paneViews = {};
  final Map<PaneId, ViewLayout> paneLayouts = {};
  final Map<PaneId, CompareMode> paneCompare = {};

  /// Where the divider sits in a compare, as a fraction across the pane.
  final Map<PaneId, double> paneDivider = {};

  String? _activeId;

  /// The view the transport plays, when one has been named (docs/07, "always
  /// preview this view"). Null means whichever view is active.
  String? alwaysPreviewId;

  /// Share view options across views, as After Effects has it. With it on,
  /// every view reads the active view's way of looking rather than its own.
  bool shareViewOptions = false;

  /// The next engine id to mint. Small integers, one per view, per open
  /// project; never reused, so a frame in flight for a closed view can never
  /// be mistaken for one belonging to a new view in its place.
  int _nextEngineId = 0;

  /// A view that has just been closed, for the caller to tell the engine
  /// about. Drained by [takeClosed].
  final List<int> _closed = [];

  ViewerSurface? byId(String? id) {
    if (id == null) return null;
    for (final v in views) {
      if (v.id == id) return v;
    }
    return null;
  }

  ViewerSurface? get active => byId(_activeId) ?? (views.isEmpty ? null : views.first);

  String? get activeId => active?.id;

  /// The view the transport plays: the named one, or the active one — but
  /// never a view of a file, which has no transport of its own and must not
  /// have a composition painted over it. With a footage view active the
  /// picture goes to the first composition view instead, which is what the
  /// transport has always meant (docs/impl/multi-viewer.md §3.3).
  ///
  /// ponytail: with no composition view on screen at all it falls back to the
  /// named one, because the engine needs a view to publish into and there is
  /// nowhere better. Give playback somewhere to go if that ever shows.
  ViewerSurface? get previewing {
    final named = byId(alwaysPreviewId) ?? active;
    if (named == null || named.mode == ViewMode.composition) return named;
    for (final view in views) {
      if (view.mode == ViewMode.composition) return view;
    }
    return named;
  }

  /// The engine ids of views closed since this was last asked, so the caller
  /// can hand their textures back.
  List<int> takeClosed() {
    final out = List.of(_closed);
    _closed.clear();
    return out;
  }

  /// The views one pane shows, in layout order, made on demand: a Viewer pane
  /// that has never been laid out is one view showing whatever is fronted.
  List<ViewerSurface> forPane(PaneId pane) {
    final ids = paneViews[pane] ??= [];
    final layout = paneLayouts[pane] ?? ViewLayout.one;
    // Trim ids naming views that have gone, then top up to the layout.
    ids.removeWhere((id) => byId(id) == null);
    while (ids.length > layout.views) {
      _forget(ids.removeLast());
    }
    while (ids.length < layout.views) {
      // A new view starts on whatever the pane's first view is showing, which
      // is what makes splitting a pane immediately useful.
      final seed = ids.isEmpty ? active : byId(ids.first);
      ids.add(_make(seed).id);
    }
    return [for (final id in ids) byId(id)!];
  }

  ViewLayout layoutOf(PaneId pane) => paneLayouts[pane] ?? ViewLayout.one;

  void setLayout(PaneId pane, ViewLayout layout) {
    if (layoutOf(pane) == layout) return;
    paneLayouts[pane] = layout;
    forPane(pane);
    notifyListeners();
  }

  CompareMode compareOf(PaneId pane) => paneCompare[pane] ?? CompareMode.none;

  void setCompare(PaneId pane, CompareMode mode) {
    if (compareOf(pane) == mode) return;
    // A compare needs two pictures to put together.
    if (mode != CompareMode.none && layoutOf(pane).views < 2) {
      paneLayouts[pane] = ViewLayout.twoAcross;
      forPane(pane);
    }
    paneCompare[pane] = mode;
    notifyListeners();
  }

  double dividerOf(PaneId pane) => paneDivider[pane] ?? 0.5;

  void setDivider(PaneId pane, double at) {
    paneDivider[pane] = at.clamp(0.02, 0.98);
    notifyListeners();
  }

  /// Make a view, copying what `seed` was showing when there is one.
  ViewerSurface _make(ViewerSurface? seed) {
    final made = ViewerSurface(
      id: _uuid.v4(),
      engineId: _nextEngineId++,
      mode: seed?.mode ?? ViewMode.composition,
      compId: seed?.compId,
      itemId: seed?.itemId,
    );
    views.add(made);
    return made;
  }

  /// Drop a view and remember to tell the engine.
  void _forget(String id) {
    final view = byId(id);
    if (view == null) return;
    views.remove(view);
    _closed.add(view.engineId);
    if (_activeId == id) _activeId = views.isEmpty ? null : views.first.id;
    if (alwaysPreviewId == id) alwaysPreviewId = null;
  }

  /// Every view a pane holds goes when the pane does.
  void forgetPane(PaneId pane) {
    for (final id in paneViews.remove(pane) ?? const <String>[]) {
      _forget(id);
    }
    paneLayouts.remove(pane);
    paneCompare.remove(pane);
    paneDivider.remove(pane);
    notifyListeners();
  }

  /// Front a view. Everything that follows the active view follows this.
  bool front(String id) {
    if (_activeId == id || byId(id) == null) return false;
    _activeId = id;
    notifyListeners();
    return true;
  }

  /// The next or previous view in the order they were made, for the keyboard.
  void cycle(int step) {
    if (views.length < 2) return;
    final at = views.indexWhere((v) => v.id == activeId);
    final next = (at + step) % views.length;
    front(views[next < 0 ? next + views.length : next].id);
  }

  /// Where an opened item lands (docs/impl/multi-viewer.md §1.5): the active
  /// view if it is unlocked, else the most recently active unlocked view, else
  /// a new view in a pane whose layout has room.
  ///
  /// **Null when every view is locked and no layout has room**, which is the
  /// caller's cue to open another Viewer panel — the only answer that still
  /// shows what was opened, and what After Effects does.
  ViewerSurface? viewForOpening(PaneId? anyViewerPane) {
    final current = active;
    if (current != null && !current.locked) return current;
    for (final v in views) {
      if (!v.locked) return v;
    }
    if (anyViewerPane == null) return null;
    final ids = paneViews[anyViewerPane] ??= [];
    if (ids.length >= layoutOf(anyViewerPane).views) return null;
    final made = _make(current);
    ids.add(made.id);
    return made;
  }

  /// The way of looking a view is drawing with, which is its own unless the
  /// share switch is on, in which case it is the active view's.
  ViewerSurface optionsFor(ViewerSurface view) =>
      shareViewOptions ? (active ?? view) : view;

  /// Announce a change to what a view is showing or how.
  void touch() => notifyListeners();

  // --- Persistence -------------------------------------------------------

  /// The project's half: what each view shows and whether it is locked.
  List<Map<String, dynamic>> toProjectJson() =>
      [for (final v in views) v.toJson()];

  /// The workspace's half: which pane holds which views, and how.
  Map<String, dynamic> toWorkspaceJson() => {
        'panes': {
          for (final e in paneViews.entries) _paneKey(e.key): e.value,
        },
        'layouts': {
          for (final e in paneLayouts.entries) _paneKey(e.key): e.value.name,
        },
        'compare': {
          for (final e in paneCompare.entries)
            if (e.value != CompareMode.none) _paneKey(e.key): e.value.name,
        },
        'dividers': {
          for (final e in paneDivider.entries) _paneKey(e.key): e.value,
        },
        if (_activeId != null) 'active': _activeId,
        if (alwaysPreviewId != null) 'always_preview': alwaysPreviewId,
        if (shareViewOptions) 'share_options': true,
      };

  static String _paneKey(PaneId pane) => '${pane.panel.name}:${pane.instance}';

  static PaneId? _paneFromKey(String key) {
    final at = key.lastIndexOf(':');
    if (at <= 0) return null;
    final panel = Panel.values.asNameMap()[key.substring(0, at)];
    final instance = int.tryParse(key.substring(at + 1));
    if (panel == null || instance == null || instance < 0) return null;
    return (panel: panel, instance: instance);
  }

  /// Put both halves back. The project's half comes first, because the
  /// workspace's names the views it lays out.
  ///
  /// **A view named in one file and not the other is dropped**, which is what
  /// keeps a workspace shareable: a lock is a reference to project content, so
  /// a workspace someone sent you can lay out views without carrying anybody
  /// else's locks (docs/07 §1.5).
  void restore(Object? project, Object? workspace) {
    views.clear();
    paneViews.clear();
    paneLayouts.clear();
    paneCompare.clear();
    paneDivider.clear();
    _activeId = null;
    alwaysPreviewId = null;
    shareViewOptions = false;
    _nextEngineId = 0;

    if (project is List) {
      for (final raw in project) {
        if (raw is! Map) continue;
        final view = ViewerSurface.fromJson(
          raw.cast<String, dynamic>(),
          _nextEngineId,
        );
        if (view == null) continue;
        _nextEngineId++;
        views.add(view);
      }
    }
    if (workspace is Map) _restoreLayout(workspace);

    // A view the project described but no pane lays out is not on screen and
    // has nothing to draw into. Dropped rather than kept invisible — and this
    // runs even with no workspace half at all, which is what a project opened
    // against an arrangement that has never held a Viewer means.
    final laidOut = {for (final ids in paneViews.values) ...ids};
    views.removeWhere((v) => !laidOut.contains(v.id));
    if (byId(_activeId) == null) {
      _activeId = views.isEmpty ? null : views.first.id;
    }
    if (byId(alwaysPreviewId) == null) alwaysPreviewId = null;
    _nextEngineId = views.fold(0, (n, v) => math.max(n, v.engineId + 1));
  }

  void _restoreLayout(Map<dynamic, dynamic> workspace) {
    final panes = workspace['panes'];
    if (panes is Map) {
      for (final e in panes.entries) {
        final pane = _paneFromKey('${e.key}');
        if (pane == null || e.value is! List) continue;
        paneViews[pane] = [
          for (final id in e.value as List)
            if (id is String && byId(id) != null) id,
        ];
      }
    }
    final layouts = workspace['layouts'];
    if (layouts is Map) {
      for (final e in layouts.entries) {
        final pane = _paneFromKey('${e.key}');
        final layout = ViewLayout.values.asNameMap()['${e.value}'];
        if (pane != null && layout != null) paneLayouts[pane] = layout;
      }
    }
    final compare = workspace['compare'];
    if (compare is Map) {
      for (final e in compare.entries) {
        final pane = _paneFromKey('${e.key}');
        final mode = CompareMode.values.asNameMap()['${e.value}'];
        if (pane != null && mode != null) paneCompare[pane] = mode;
      }
    }
    final dividers = workspace['dividers'];
    if (dividers is Map) {
      for (final e in dividers.entries) {
        final pane = _paneFromKey('${e.key}');
        final at = e.value;
        if (pane != null && at is num && at.isFinite) {
          paneDivider[pane] = at.toDouble().clamp(0.02, 0.98);
        }
      }
    }
    final active = workspace['active'];
    if (active is String && byId(active) != null) _activeId = active;
    final preview = workspace['always_preview'];
    if (preview is String && byId(preview) != null) alwaysPreviewId = preview;
    shareViewOptions = workspace['share_options'] == true;
  }
}
