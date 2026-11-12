// The Shots panel: every shot of the comp in time order, one graded picture
// each, the way a colourist walks a cut.
//
// A shot is a picture clip on a Sequence layer, or any other layer with a
// picture of its own. Clicking one makes it the current shot, which the Grade
// and Grade nodes panels follow: the layer is selected, a clip is picked as
// the Cut timeline picks it, and the playhead goes into the shot if it was
// outside. The pictures are the shot alone with its grade, made one at a time
// after the document changes, newest first, and only for tiles on screen.

import 'dart:async';
import 'dart:collection';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/grade_groups.dart';
import 'package:lumit_flutter/src/rust/api/grade_shots.dart';
import 'package:lumit_flutter/src/rust/api/grade_versions.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/settings_account.dart' show askBefore;
import '../shell/theme_name_dialog.dart';
import '../state/dock.dart';
import '../state/grade_target.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'grade_actions.dart';
import 'grade_clipboard.dart';
import 'grade_controls.dart' show GradeIconButton, GradeMenu, gradeInset;
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'param_row_menu.dart' show menuGroups;
import 'timeline_extras_frb.dart'
    show DoubleTap, Segment, SegmentOption, labelChips;

enum _Filter { all, graded, ungraded }

/// What a tile says beside its picture, read once per document revision for
/// the tiles on screen.
typedef _Meta = ({String? version, UuidValue? group});

class ShotsPanelFrb extends StatefulWidget {
  const ShotsPanelFrb({super.key});

  @override
  State<ShotsPanelFrb> createState() => _ShotsPanelFrbState();
}

class _ShotsPanelFrbState extends State<ShotsPanelFrb> {
  LumitUiState get uiState => Provider.of<LumitUiState>(context, listen: false);
  LumitState get app => Provider.of<LumitState>(context, listen: false);

  _Filter _filter = _Filter.all;

  /// The label colour shown, by its place in the palette; null shows every
  /// colour.
  int? _label;

  /// The shots picked beside the current one, by [GradeShot.key], and the
  /// one a Shift-click runs from.
  final Set<String> _picked = {};
  String? _anchor;

  final DoubleTap _double = DoubleTap();
  String? _lastTap;

  final ScrollController _scroll = ScrollController();

  /// The longest edge a picture is made at: enough for the Lightbox too.
  static const int _edge = 480;

  /// Pictures by shot, frame and grade, the newest last; a bounded number
  /// kept. A null is one asked for and not back yet.
  final LinkedHashMap<String, ui.Image?> _pictures = LinkedHashMap();
  static const int _kept = 160;

  /// Pictures wanted, the newest last, and whether one is being made.
  final LinkedHashMap<String, (BridgeShot, int)> _wanted = LinkedHashMap();
  bool _making = false;
  final ValueNotifier<int> _arrived = ValueNotifier(0);

  int? _metaRevision;
  final Map<String, _Meta> _meta = {};
  final Map<String, String> _signs = {};
  List<BridgeGradeGroup> _groups = const [];
  (Object?, int?, (double, double))? _aspectRead;

  /// Shots whose versions and group are wanted, read after the frame.
  final Map<String, BridgeShot> _metaWanted = {};

  /// The shots as last drawn, each tile's run along the strip and how many
  /// stand in a row, so the current shot can be brought into view.
  List<BridgeShot> _shownNow = const [];
  double _extent = 0;
  int _across = 1;

  late final LumitUiState _ui = uiState;

  @override
  void initState() {
    super.initState();
    _ui.selectedLayer.addListener(_dropStaleClip);
    _ui.grade.shot.addListener(_follow);
    _ui.maximisedPane.addListener(_leftLightbox);
  }

  @override
  void dispose() {
    _ui.selectedLayer.removeListener(_dropStaleClip);
    _ui.grade.shot.removeListener(_follow);
    _ui.maximisedPane.removeListener(_leftLightbox);
    _scroll.dispose();
    _arrived.dispose();
    for (final p in _pictures.values) {
      p?.dispose();
    }
    super.dispose();
  }

  /// A clip this panel picked stops being picked when another layer is
  /// chosen elsewhere, as it would in the Cut timeline.
  void _dropStaleClip() {
    final ui = uiState;
    final clip = ui.cutClip.value;
    if (clip == null) return;
    if (ui.selectedLayer.value?.internallayerId != clip.layer.internallayerId) {
      ui.cutClip.value = null;
      ui.cutClips.value = const [];
    }
  }

  // --- Reads ---------------------------------------------------------------

  double _aspect() => _shape().$1;

  /// The comp's width in pixels, how big its picture is at its own size.
  double _compWidth() => _shape().$2;

  (double, double) _shape() {
    final comp = uiState.selectedComp;
    final revision = uiState.model.revision;
    final held = _aspectRead;
    if (held != null && held.$1 == comp?.internalid && held.$2 == revision) {
      return held.$3;
    }
    var shape = (16 / 9, 1920.0);
    try {
      final s = comp?.getSettings();
      if (s != null && s.height > 0 && s.width > 0) {
        shape = (
          (s.width / s.height).clamp(0.3, 4.0).toDouble(),
          s.width.toDouble()
        );
      }
    } catch (_) {
      // No comp; the usual shape.
    }
    _aspectRead = (comp?.internalid, revision, shape);
    return shape;
  }

  /// A new revision forgets what tiles said; they ask again as they draw.
  void _freshen() {
    final revision = uiState.model.revision;
    if (revision == _metaRevision) return;
    _metaRevision = revision;
    _signs.clear();
    // What tiles said stays up until the new answers are in.
    _metaWanted.clear();
    _metaStale = true;
  }

  bool _metaStale = false;

  /// What [s]'s tile says, from the last read; asked again after the frame
  /// when the document has moved on since.
  _Meta _metaOf(BridgeShot s) {
    final key = shotOf(s).key;
    final held = _meta[key];
    if (held == null || _metaStale) {
      if (_metaWanted.isEmpty) {
        WidgetsBinding.instance.addPostFrameCallback((_) => _readMeta());
      }
      _metaWanted[key] = s;
    }
    return held ?? (version: null, group: null);
  }

  void _readMeta() {
    if (!mounted || _metaWanted.isEmpty) return;
    if (_metaStale) {
      _metaStale = false;
      _meta.clear();
      try {
        _groups = app.project?.getGradeGroups() ?? const [];
      } catch (_) {
        _groups = const [];
      }
    }
    for (final MapEntry(:key, value: s) in _metaWanted.entries) {
      String? version;
      UuidValue? group;
      try {
        final versions = s.layer.getGradeVersions(clip: s.clip);
        if (versions.length > 1) {
          version = versions.where((v) => v.live).firstOrNull?.name;
        }
        group = s.layer.getGradeGroup(clip: s.clip);
      } catch (_) {
        // The shot went; it is gone from the list on the next read.
      }
      _meta[key] = (version: version, group: group);
    }
    _metaWanted.clear();
    setState(() {});
  }

  /// The pane put back, by Escape or the window's own control: the
  /// Lightbox goes with it.
  void _leftLightbox() {
    if (_ui.grade.lightbox.value &&
        _ui.maximisedPane.value != Panel.shots.pane()) {
      _ui.grade.lightbox.value = false;
    }
  }

  /// Bring the current shot into view when it changes from elsewhere: a
  /// key, the Timeline, the Cut timeline.
  void _follow() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final current = _ui.grade.shot.value;
      if (!mounted || current == null || !_scroll.hasClients) return;
      final at = _shownNow.indexWhere((s) => shotOf(s).same(current));
      if (at < 0 || _extent <= 0) return;
      final p = _scroll.position;
      final start = at ~/ _across * _extent;
      final end = start + _extent + 16;
      if (start < p.pixels) {
        _scroll.jumpTo(start.clamp(0.0, p.maxScrollExtent));
      } else if (end > p.pixels + p.viewportDimension) {
        _scroll.jumpTo(
            (end - p.viewportDimension).clamp(0.0, p.maxScrollExtent));
      }
    });
  }

  /// What the shot's picture depends on here: its stack's effects and
  /// values. A change anywhere else leaves the picture as it is.
  String _sign(BridgeShot s) {
    final shot = shotOf(s);
    return _signs.putIfAbsent(shot.key, () {
      final stack = shotStackOf(uiState, shot) ?? const [];
      return Object.hashAll([
        for (final e in stack) ...[
          e.name,
          e.enabled,
          for (final v in e.values) '${v.id}=${v.value}',
        ]
      ]).toString();
    });
  }

  static int _frameOf(BridgeShot s) => (s.startFrame + s.endFrame) ~/ 2;

  String _pictureKey(BridgeShot s) =>
      '${shotOf(s).key}@${_frameOf(s)}#${_sign(s)}';

  void _want(String key, BridgeShot s) {
    if (_pictures.containsKey(key) || _wanted.containsKey(key)) return;
    _wanted.remove(key);
    _wanted[key] = (s, _frameOf(s));
    // Only the newest few: a fast scroll leaves the rest behind.
    while (_wanted.length > 48) {
      _wanted.remove(_wanted.keys.first);
    }
    if (!_making) {
      _making = true;
      WidgetsBinding.instance.addPostFrameCallback((_) => _make());
    }
  }

  /// Make the wanted pictures one at a time, the newest first.
  Future<void> _make() async {
    while (mounted && _wanted.isNotEmpty) {
      final key = _wanted.keys.last;
      final (s, frame) = _wanted.remove(key)!;
      final CompositionReference? comp = uiState.selectedComp;
      if (comp == null) break;
      _pictures[key] = null;
      ui.Image? image;
      try {
        final got = await comp.shotThumbnail(
            layer: s.layer.internallayerId,
            clip: s.clip,
            frame: frame,
            maxEdge: _edge);
        if (got != null && got.width > 0) {
          final done = Completer<ui.Image>();
          ui.decodeImageFromPixels(got.rgba, got.width, got.height,
              ui.PixelFormat.rgba8888, done.complete);
          image = await done.future;
        }
      } catch (_) {
        image = null;
      }
      if (!mounted) {
        image?.dispose();
        return;
      }
      _keep(key, s, image);
    }
    _making = false;
  }

  /// Keep [image] for [key], letting go of the shot's older pictures and
  /// of the oldest beyond what is kept.
  void _keep(String key, BridgeShot s, ui.Image? image) {
    final prefix = '${shotOf(s).key}@';
    final old = [
      for (final k in _pictures.keys)
        if (k != key && k.startsWith(prefix)) k
    ];
    // The old picture stays until the new one is in, so a tile never blinks.
    if (image != null) {
      for (final k in old) {
        _pictures.remove(k)?.dispose();
      }
    }
    _pictures.remove(key);
    _pictures[key] = image;
    while (_pictures.length > _kept) {
      _pictures.remove(_pictures.keys.first)?.dispose();
    }
    _arrived.value++;
  }

  /// The newest picture of [s] there is, the one for its grade now or an
  /// older one while that is made.
  ui.Image? _pictureOf(BridgeShot s) {
    final key = _pictureKey(s);
    if (_pictures[key] case final image?) return image;
    _want(key, s);
    final prefix = '${shotOf(s).key}@';
    for (final e in _pictures.entries) {
      if (e.key.startsWith(prefix) && e.value != null) return e.value;
    }
    return null;
  }

  // --- Picking -------------------------------------------------------------

  List<BridgeShot> _shown(List<BridgeShot> all) => [
        for (final s in all)
          if ((_label == null || s.label == _label) &&
              switch (_filter) {
                _Filter.all => true,
                _Filter.graded => s.graded,
                _Filter.ungraded => !s.graded,
              })
            s
      ];

  void _tap(BridgeShot s, List<BridgeShot> shown) {
    final key = shotOf(s).key;
    final keys = HardwareKeyboard.instance;
    final ui = uiState;
    final current = ui.gradeShot.value;
    if (keys.isControlPressed || keys.isMetaPressed) {
      // Ctrl toggles a shot in the pick; the current shot stays current.
      if (current != null && shotOf(s).same(current)) return;
      setState(() {
        if (!_picked.remove(key)) _picked.add(key);
      });
      final cur =
          shown.where((x) => current != null && shotOf(x).same(current));
      if (cur.isNotEmpty) _publish(cur.first, shown);
      return;
    }
    if (keys.isShiftPressed && _anchor != null) {
      final from = shown.indexWhere((x) => shotOf(x).key == _anchor);
      final to = shown.indexOf(s);
      if (from >= 0 && to >= 0) {
        final (a, b) = from < to ? (from, to) : (to, from);
        setState(() {
          _picked
            ..clear()
            ..addAll([for (final x in shown.sublist(a, b + 1)) shotOf(x).key]);
        });
        _publish(s, shown);
        return;
      }
    }
    final second = _double.tap() && _lastTap == key;
    _lastTap = key;
    setState(() {
      _picked.clear();
      _anchor = key;
    });
    _publish(s, shown);
    if (second) _reveal(s);
  }

  void _publish(BridgeShot current, List<BridgeShot> shown) {
    final picked = [
      for (final s in shown)
        if (_picked.contains(shotOf(s).key) &&
            (s.clip == null) == (current.clip == null))
          s
    ];
    pickShots(uiState, current, picked);
  }

  /// Bring the shot up in the Cut timeline for a clip when there is one in
  /// the arrangement, else in the Timeline.
  void _reveal(BridgeShot s) {
    final ui = uiState;
    final panel = s.clip != null && panelVisible(ui.split, Panel.cutTimeline)
        ? Panel.cutTimeline
        : Panel.timeline;
    setPanelVisible(ui.split, panel, true);
    activatePanelTab(ui.split, panel);
    ui.activePane.value = panel.pane();
    ui.workspace.touch();
  }

  /// The shots a menu row acts on: the pressed one, and the others picked
  /// with it when it is one of them.
  List<BridgeShot> _actedOn(BridgeShot s, List<BridgeShot> shown) {
    final current = uiState.gradeShot.value;
    final inPick = _picked.contains(shotOf(s).key) ||
        (current != null && shotOf(s).same(current));
    if (!inPick) return [s];
    return [
      s,
      for (final x in shown)
        if (x != s &&
            (_picked.contains(shotOf(x).key) ||
                (current != null && shotOf(x).same(current))))
          x
    ];
  }

  // --- Menu ----------------------------------------------------------------

  MenuRow _row(String key, String label, VoidCallback close, VoidCallback run,
          {bool enabled = true, bool? ticked, String? action}) =>
      MenuRow(
        key: ValueKey<String>(key),
        enabled: enabled,
        onPressed: () {
          close();
          run();
        },
        child:
            MenuLabel(label, ticked: ticked, enabled: enabled, action: action),
      );

  void _tell() {
    app.notifyDocumentChanged();
    uiState.model.refresh();
  }

  void _edit(String name, void Function() run) {
    asOneUndoStep(app.project, () {
      try {
        run();
      } catch (_) {
        // A locked layer or a shot that went: the document is as it was.
      }
    }, name: name);
    _tell();
  }

  /// The pressed shot's menu. Its rows act on the pressed shot, and on the
  /// others picked with it, without changing which shot is current.
  List<Widget> _menu(VoidCallback close, BridgeShot s, List<BridgeShot> all,
      List<BridgeShot> shown) {
    final ui = uiState;
    final current = ui.gradeShot.value;
    final shot = shotOf(s);
    final acted = [for (final x in _actedOn(s, shown)) shotOf(x)];
    final at = all.indexOf(s);
    final graded = shotGradeIds(ui, shot).isNotEmpty;
    final reference = current != null && !current.same(shot)
        ? all.where((x) => shotOf(x).same(current)).firstOrNull
        : null;
    // The reference is read where the playhead is, if it is in that shot.
    int frameIn(BridgeShot r) {
      final f = ui.playheadFrame.value;
      return f >= r.startFrame && f < r.endFrame
          ? f
          : (r.startFrame + r.endFrame) ~/ 2;
    }

    return menuGroups([
      [
        _row('shots-copy', l10n.gradeCopy, close, () => copyShotGrade(ui, shot),
            enabled: graded, action: 'grade.copy'),
        _row('shots-paste', l10n.gradePaste, close,
            () => layGrade(app, ui, ui.clipboard.text!, to: acted),
            enabled: canPasteGrade(ui), action: 'grade.paste'),
        _row('shots-previous', l10n.gradeApplyPrevious, close,
            () => applyPreviousGrade(app, ui, before: s, onto: acted),
            enabled: at > 0, action: 'grade.apply.previous'),
        _row('shots-second', l10n.gradeApplySecond, close,
            () => applyPreviousGrade(app, ui, before: s, onto: acted, back: 2),
            enabled: at > 1, action: 'grade.apply.second'),
        _row('shots-reset', l10n.gradeResetAll, close,
            () => resetGrade(app, ui, onto: acted),
            enabled: graded),
      ],
      [
        SubmenuRow(
          key: const ValueKey<String>('shots-versions'),
          closeParent: close,
          submenu: (dismiss) => FloatSurface(
            child: IntrinsicWidth(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: _versionRows(dismiss, shot),
              ),
            ),
          ),
          child: MenuLabel(l10n.gradeVersions),
        ),
        SubmenuRow(
          key: const ValueKey<String>('shots-group'),
          closeParent: close,
          submenu: (dismiss) => FloatSurface(
            child: IntrinsicWidth(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: _groupRows(dismiss, _actedOn(s, shown)),
              ),
            ),
          ),
          child: MenuLabel(l10n.gradeGroup),
        ),
      ],
      [
        _row('shots-match', l10n.gradeMatchToCurrent, close,
            () => matchShotTo(
                app, ui, s, shotOf(reference!), frameIn(reference)),
            enabled: reference != null),
        _row('shots-still', l10n.gradeGrabStill, close,
            () => grabStill(app, ui, of: s),
            action: 'grade.still.grab'),
        _row('shots-lut', l10n.gradeExportLutEllipsis, close,
            () => exportShotLut(app, ui, shot: shot),
            enabled: graded),
      ],
      [
        _row('shots-grid', l10n.gradeGridShots, close,
            () => toggleGradeGrid(ui, GradeGrid.shots),
            enabled: gradeTargets(ui).length > 1,
            ticked: ui.grade.grid.value == GradeGrid.shots),
        _row('shots-lightbox', l10n.gradeLightbox, close,
            () => toggleLightbox(ui),
            ticked: ui.grade.lightbox.value, action: 'grade.lightbox'),
      ],
    ]);
  }

  List<Widget> _versionRows(VoidCallback close, GradeShot shot) {
    List<BridgeGradeVersion> versions;
    try {
      versions = shot.layer.getGradeVersions(clip: shot.clip);
    } catch (_) {
      versions = const [];
    }
    final live = versions.where((v) => v.live).firstOrNull;
    void run(void Function() edit) {
      try {
        edit();
      } catch (_) {
        // The shot went, or refused.
      }
      _tell();
    }

    return menuGroups([
      [
        for (final v in versions)
          _row('shots-version-${v.id}', v.name, close,
              () => run(() => shot.layer
                  .switchGradeVersion(clip: shot.clip, version: v.id)),
              ticked: v.live),
      ],
      [
        _row('shots-version-new', l10n.gradeVersionNew, close,
            () => run(() =>
                shot.layer.addGradeVersion(clip: shot.clip, copy: false)),
            ticked: versions.isEmpty ? null : false),
        _row('shots-version-copy', l10n.gradeVersionDuplicate, close,
            () => run(() =>
                shot.layer.addGradeVersion(clip: shot.clip, copy: true)),
            ticked: versions.isEmpty ? null : false),
        if (live != null)
          _row('shots-version-rename', l10n.renameEllipsis, close,
              () => _renameVersion(shot, live),
              ticked: false),
        if (live != null)
          _row('shots-version-delete', l10n.gradeVersionDelete, close,
              () => run(() => shot.layer
                  .deleteGradeVersion(clip: shot.clip, version: live.id)),
              ticked: false),
      ],
    ]);
  }

  Future<void> _renameVersion(GradeShot shot, BridgeGradeVersion v) async {
    final name = await askThemeName(context,
        title: l10n.gradeVersionRenameTitle,
        suggested: v.name,
        confirm: l10n.rename,
        label: l10n.name);
    if (name == null || name.trim().isEmpty || !mounted) return;
    try {
      shot.layer
          .renameGradeVersion(
              clip: shot.clip, version: v.id, name: name.trim());
    } catch (_) {
      return;
    }
    _tell();
  }

  List<Widget> _groupRows(VoidCallback close, List<BridgeShot> shots) {
    final project = app.project;
    final comp = uiState.selectedComp?.internalid;
    if (project == null || comp == null) return const [];
    final groups = _groups;
    final inGroups = {for (final s in shots) _metaOf(s).group};
    final one = inGroups.length == 1 ? inGroups.single : null;
    return menuGroups([
      [
        _row('shots-group-new', l10n.gradeGroupNew, close,
            () => _edit('New grade group', () {
                  project.createGradeGroup(shots: [
                    for (final s in shots)
                      BridgeShotId(
                          comp: comp,
                          layer: s.layer.internallayerId,
                          clip: s.clip),
                  ]);
                }),
            ticked: groups.isEmpty ? null : false),
      ],
      [
        for (final g in groups)
          _row('shots-group-${g.id}', g.name, close,
              () => _edit('Join grade group', () {
                    for (final s in shots) {
                      s.layer.setGradeGroup(clip: s.clip, group: g.id);
                    }
                  }),
              ticked: one == g.id),
      ],
      [
        if (groups.where((g) => g.id == one).firstOrNull case final g?) ...[
          _row('shots-group-rename', l10n.gradeGroupRename, close,
              () => _renameGroup(g),
              ticked: false),
          _row('shots-group-delete', l10n.gradeGroupDelete, close,
              () => _deleteGroup(g),
              ticked: false),
        ],
        if (inGroups.any((g) => g != null))
          _row('shots-group-leave', l10n.gradeGroupLeave, close,
              () => _edit('Leave grade group', () {
                    for (final s in shots) {
                      s.layer.setGradeGroup(clip: s.clip, group: null);
                    }
                  }),
              ticked: groups.isEmpty ? null : false),
      ],
    ]);
  }

  Future<void> _renameGroup(BridgeGradeGroup g) async {
    final name = await askThemeName(context,
        title: l10n.gradeGroupRenameTitle,
        suggested: g.name,
        confirm: l10n.rename,
        label: l10n.name);
    final project = app.project;
    if (name == null || name.trim().isEmpty || project == null || !mounted) {
      return;
    }
    try {
      project.renameGradeGroup(group: g.id, name: name.trim());
    } catch (_) {
      return;
    }
    _metaStale = true;
    _tell();
  }

  Future<void> _deleteGroup(BridgeGradeGroup g) async {
    if (!await askBefore(
        context, l10n.gradeGroupDeleteAsk(g.name), l10n.delete)) {
      return;
    }
    final project = app.project;
    if (project == null || !mounted) return;
    try {
      project.deleteGradeGroup(group: g.id);
    } catch (_) {
      return;
    }
    // A group's grade on view goes with it.
    uiState.grade.scope.value = GradeScope.shot;
    _metaStale = true;
    _tell();
  }

  // --- Drawing -------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final ui = uiState;
    return ListenableBuilder(
      listenable:
          Listenable.merge([ui.model, ui.grade.shot, ui.grade.lightbox]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        _freshen();
        final all = compShots(ui);
        final shown = _shown(all);
        final numbers = {
          for (final (i, s) in all.indexed) shotOf(s).key: i + 1,
        };
        // Picks of shots that went are let go.
        _picked.removeWhere((k) => !numbers.containsKey(k));
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _header(t, all.length),
            Expanded(
              child: Container(
                color: t.surface0,
                child: LayoutBuilder(
                  builder: (context, box) =>
                      _strip(t, box.biggest, all, shown, numbers),
                ),
              ),
            ),
          ],
        );
      },
    );
  }

  Widget _header(LumitTheme t, int count) => Container(
        height: 26,
        color: t.surface1,
        padding: const EdgeInsets.only(left: gradeInset, right: 4),
        child: Row(
          children: [
            Expanded(
              child: Text(count == 0 ? '' : l10n.gradeShotCount(count),
                  style: t.small.copyWith(color: t.textMuted),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis),
            ),
            Flexible(
              flex: 3,
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                reverse: true,
                child: Row(mainAxisSize: MainAxisSize.min, children: [
                  Segment(children: [
                  for (final f in _Filter.values)
                    SegmentOption(
                      key: ValueKey<String>('shots-filter-${f.name}'),
                      active: f == _filter,
                      onTap: () => setState(() => _filter = f),
                      child: Text(
                          switch (f) {
                            _Filter.all => l10n.gradeFilterAll,
                            _Filter.graded => l10n.gradeFilterGraded,
                            _Filter.ungraded => l10n.gradeFilterUngraded,
                          },
                          style: t.small.copyWith(
                              color: f == _filter
                                  ? t.textPrimary
                                  : t.textSecondary)),
                    ),
                  ]),
                  const SizedBox(width: 4),
                  _colourFilter(t),
                  const SizedBox(width: 2),
                  GradeIconButton(
                    key: const ValueKey('shots-lightbox'),
                    glyph: LumitIcons.grid,
                    tip: l10n.gradeLightbox,
                    active: uiState.grade.lightbox.value,
                    onPressed: () => toggleLightbox(uiState),
                  ),
                ]),
              ),
            ),
          ],
        ),
      );

  /// The label colour filter: a chip in the colour shown, hollow while every
  /// colour shows. It offers the Timeline's own chips.
  Widget _colourFilter(LumitTheme t) => Builder(builder: (context) {
        final label = _label;
        return LumitTooltip(
          message: l10n.gradeFilterColour,
          child: HouseButton(
            key: const ValueKey('shots-colour'),
            frameless: true,
            small: true,
            active: label != null,
            padding: const EdgeInsets.all(4),
            onPressed: () {
              final box = context.findRenderObject()! as RenderBox;
              GradeMenu.show(
                context,
                box.localToGlobal(Offset(0, box.size.height + 2)),
                (close) => [
                  _row('shots-colour-any', l10n.gradeFilterAnyColour, close,
                      () => setState(() => _label = null),
                      ticked: label == null),
                  labelChips(t,
                      keyPrefix: 'shots-colour',
                      selected: label,
                      onPick: (i) {
                        close();
                        setState(() => _label = i);
                      }),
                ],
              );
            },
            child: Container(
              width: 12,
              height: 12,
              decoration: BoxDecoration(
                color: label == null ? null : t.labelColour(label),
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                border: label == null
                    ? Border.all(color: t.textMuted)
                    : null,
              ),
            ),
          ),
        );
      });

  Widget _strip(LumitTheme t, Size pane, List<BridgeShot> all,
      List<BridgeShot> shown, Map<String, int> numbers) {
    const pad = 8.0, gap = 6.0, line = 18.0;
    final aspect = _aspect();
    if (uiState.grade.lightbox.value) {
      return _lightbox(t, pane, all, shown, numbers, aspect);
    }
    _across = 1;
    final down = pane.height > pane.width;
    // Across, the picture is as tall as the pane allows; down, as wide.
    final double pictureW, pictureH;
    if (down) {
      pictureW = (pane.width - pad * 2).clamp(40.0, 360.0);
      pictureH = pictureW / aspect;
    } else {
      pictureH = (pane.height - pad * 2 - line).clamp(24.0, 200.0);
      pictureW = pictureH * aspect;
    }
    final extent = down ? pictureH + line + gap : pictureW + gap;
    _shownNow = shown;
    _extent = extent;
    final current = uiState.gradeShot.value;
    return ListView.builder(
      key: const ValueKey('shots-strip'),
      controller: _scroll,
      primary: false,
      scrollDirection: down ? Axis.vertical : Axis.horizontal,
      padding: const EdgeInsets.all(pad),
      itemExtent: extent,
      itemCount: shown.length,
      itemBuilder: (context, i) {
        final s = shown[i];
        final shot = shotOf(s);
        return Padding(
          padding: down
              ? const EdgeInsets.only(bottom: gap)
              : const EdgeInsets.only(right: gap),
          // A pane shorter than a tile clips it rather than squeezing it.
          child: ClipRect(
            child: OverflowBox(
              alignment: Alignment.topLeft,
              minHeight: 0,
              maxHeight: double.infinity,
              child: Align(
                alignment: Alignment.topLeft,
                child: SizedBox(
                  width: pictureW,
                  child: _tile(t, s, numbers[shot.key] ?? i + 1, pictureH,
                      line, current != null && shot.same(current),
                      _picked.contains(shot.key), all, shown),
                ),
              ),
            ),
          ),
        );
      },
    );
  }

  /// The Lightbox: every shot as a large picture, row after row in time
  /// order, with the strip's numbers, picks and menus.
  Widget _lightbox(LumitTheme t, Size pane, List<BridgeShot> all,
      List<BridgeShot> shown, Map<String, int> numbers, double aspect) {
    // About four across a full-width window, and never wider than the
    // picture at its own size.
    const pad = 12.0, gap = 10.0, line = 18.0, least = 420.0;
    final room = pane.width - pad * 2;
    final across = math.max(
        math.max(1, ((room + gap) / (least + gap)).floor()),
        ((room + gap) / (_compWidth() + gap)).ceil());
    final pictureW = (room - gap * (across - 1)) / across;
    final pictureH = pictureW / aspect;
    _shownNow = shown;
    _extent = pictureH + line + gap;
    _across = across;
    final current = uiState.gradeShot.value;
    return GridView.builder(
      key: const ValueKey('shots-lightbox-grid'),
      controller: _scroll,
      primary: false,
      padding: const EdgeInsets.all(pad),
      gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
        crossAxisCount: across,
        crossAxisSpacing: gap,
        mainAxisSpacing: gap,
        mainAxisExtent: pictureH + line,
      ),
      itemCount: shown.length,
      itemBuilder: (context, i) {
        final s = shown[i];
        final shot = shotOf(s);
        return _tile(t, s, numbers[shot.key] ?? i + 1, pictureH, line,
            current != null && shot.same(current), _picked.contains(shot.key),
            all, shown);
      },
    );
  }

  Widget _tile(LumitTheme t, BridgeShot s, int number, double pictureH,
      double line, bool current, bool picked, List<BridgeShot> all,
      List<BridgeShot> shown) {
    final meta = _metaOf(s);
    final groupAt =
        meta.group == null ? -1 : _groups.indexWhere((g) => g.id == meta.group);
    return DragTarget<GradeStillDrag>(
      onAcceptWithDetails: (d) =>
          applyStillGrade(app, uiState, d.data.path, [shotOf(s)]),
      builder: (context, over, _) => GradeMenu(
        rows: (close) => _menu(close, s, all, shown),
        child: GestureDetector(
          key: ValueKey<String>('shots-tile-${shotOf(s).key}'),
          behavior: HitTestBehavior.opaque,
          onTap: () => _tap(s, shown),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Container(
                height: pictureH,
                decoration: BoxDecoration(
                  color: t.surface2,
                  borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                  border: Border.all(
                      color: over.isNotEmpty || current
                          ? t.accent
                          : picked
                              ? t.hairlineStrong
                              : t.hairline,
                      width: current || over.isNotEmpty ? 2 : 1),
                ),
                clipBehavior: Clip.antiAlias,
                child: Stack(
                  fit: StackFit.expand,
                  children: [
                    ValueListenableBuilder<int>(
                      valueListenable: _arrived,
                      builder: (context, _, __) {
                        final image = _pictureOf(s);
                        return image == null
                            ? const SizedBox.shrink()
                            : RawImage(image: image, fit: BoxFit.cover);
                      },
                    ),
                    Positioned(
                      left: 3,
                      top: 3,
                      child: Container(
                        padding: const EdgeInsets.symmetric(horizontal: 3),
                        decoration: BoxDecoration(
                          color: t.surface1,
                          borderRadius:
                              BorderRadius.circular(t.tokens.actionRadius),
                          border: Border.all(
                              color: s.graded ? t.accent : t.hairline),
                        ),
                        child: Text('$number',
                            style: t.small.copyWith(color: t.textSecondary)),
                      ),
                    ),
                    if (groupAt >= 0)
                      Positioned(
                        right: 4,
                        top: 4,
                        child: Container(
                          width: 7,
                          height: 7,
                          decoration: BoxDecoration(
                            color: t.personColour(groupAt + 1),
                            shape: BoxShape.circle,
                          ),
                        ),
                      ),
                  ],
                ),
              ),
              // The name, and the version after it when there is a choice.
              SizedBox(
                height: line,
                child: Row(
                  children: [
                    Flexible(
                      child: Text(s.name,
                          style: t.small.copyWith(
                              color:
                                  current ? t.textPrimary : t.textSecondary),
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis),
                    ),
                    if (meta.version case final v?) ...[
                      const SizedBox(width: 6),
                      Flexible(
                        child: Text(v,
                            style: t.small.copyWith(color: t.textMuted),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis),
                      ),
                    ],
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
