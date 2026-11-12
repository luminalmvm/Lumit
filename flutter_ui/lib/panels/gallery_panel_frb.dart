// The Gallery: stills kept with the grade that made them, in albums every
// project sees.
//
// A still is a frame as the Viewer showed it and the shot's grade at the
// time, two files in the library's Stills folder. Grab still keeps the current
// shot at the playhead. A still is compared against in the Viewer, its grade
// laid on a shot in place of the one it has, or dragged onto a Shots tile to
// do the same. Holding Alt over a still shows its grade on the current shot
// for as long as it is held, and writes nothing.

import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/grade_lut.dart';
import 'package:lumit_flutter/src/rust/api/stills.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/settings_account.dart' show askBefore;
import '../shell/theme_name_dialog.dart';
import '../state/file_dialogs.dart';
import '../state/grade_target.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/edge_fade.dart';
import 'grade_actions.dart'
    show
        fileSafe,
        galleryStillsChanged,
        grabStill,
        shortClock,
        toggleGradeGrid;
import 'grade_clipboard.dart';
import 'grade_controls.dart';
import 'grade_memories.dart';
import 'timeline_extras_frb.dart' show DoubleTap;

/// The album new stills go to while the library has none.
String get _firstAlbum => l10n.galleryFirstAlbum;

/// [still] as something the Viewer can compare against.
GradeReference stillReference(BridgeStill still) => GradeReference.still(
    path: still.path, picture: still.pngPath, name: still.name);

class GalleryPanelFrb extends StatefulWidget {
  const GalleryPanelFrb({super.key});

  @override
  State<GalleryPanelFrb> createState() => _GalleryPanelFrbState();
}

class _GalleryPanelFrbState extends State<GalleryPanelFrb> {
  LumitUiState get ui => Provider.of<LumitUiState>(context, listen: false);
  LumitState get app => Provider.of<LumitState>(context, listen: false);

  List<String> _albums = const [];
  List<BridgeStill> _stills = const [];

  /// The still clicked last, which a Shift-click runs from, and every still
  /// picked with it.
  String? _picked;
  final Set<String> _picks = {};
  final DoubleTap _taps = DoubleTap();

  /// The still under the pointer, and whether its grade is being shown.
  BridgeStill? _hover;
  bool _previewing = false;

  @override
  void initState() {
    super.initState();
    galleryStillsChanged.addListener(_load);
    HardwareKeyboard.instance.addHandler(_onKey);
    _load();
  }

  @override
  void dispose() {
    galleryStillsChanged.removeListener(_load);
    HardwareKeyboard.instance.removeHandler(_onKey);
    if (_previewing) ui.requestFrame();
    super.dispose();
  }

  /// The album shown, which every Grab still keeps to.
  String? get _shownAlbum => ui.grade.album.value;
  set _shownAlbum(String? album) => ui.grade.album.value = album;

  String get _album => _shownAlbum ?? _albums.firstOrNull ?? _firstAlbum;

  Future<void> _load() async {
    List<String> albums;
    List<BridgeStill> stills;
    try {
      albums = await listStillAlbums();
      if (!mounted) return;
      final album = _shownAlbum != null && albums.contains(_shownAlbum)
          ? _shownAlbum!
          : albums.firstOrNull;
      _shownAlbum = album;
      stills = album == null ? const [] : await listStills(album: album);
    } catch (_) {
      // No library folder: nothing to list.
      albums = const [];
      stills = const [];
    }
    if (!mounted) return;
    setState(() {
      _albums = albums;
      _stills = stills;
    });
    _publish();
  }

  /// Pick [still]: alone, or with Ctrl toggled in the pick, or with Shift
  /// every still from the last one clicked.
  void _pick(BridgeStill still) {
    final keys = HardwareKeyboard.instance;
    setState(() {
      if (keys.isControlPressed || keys.isMetaPressed) {
        if (!_picks.remove(still.path)) _picks.add(still.path);
      } else if (keys.isShiftPressed && _picked != null) {
        final from = _stills.indexWhere((s) => s.path == _picked);
        final to = _stills.indexOf(still);
        if (from >= 0 && to >= 0) {
          final (a, b) = from < to ? (from, to) : (to, from);
          _picks
            ..clear()
            ..addAll([for (final s in _stills.sublist(a, b + 1)) s.path]);
        }
      } else {
        _picks
          ..clear()
          ..add(still.path);
      }
      _picked = still.path;
    });
    _publish();
  }

  /// The picked stills, in the album's order, for the Viewer's grid.
  void _publish() {
    _picks.removeWhere((p) => !_stills.any((s) => s.path == p));
    ui.grade.pickedStills.value = [
      for (final s in _stills)
        if (_picks.contains(s.path)) stillReference(s)
    ];
  }

  void _show(String album) {
    _shownAlbum = album;
    _load();
  }

  Future<void> _grab() async {
    final still = await grabStill(app, ui);
    if (still == null || !mounted) return;
    _shownAlbum = still.album;
    setState(() {
      _picked = still.path;
      _picks
        ..clear()
        ..add(still.path);
    });
  }

  /// The still's picture written where the user picks, as a PNG.
  Future<void> _exportPicture(BridgeStill still) async {
    final path = await pickExportSaveLocation(fileSafe(still.name),
        extension: 'png', label: l10n.formatPngPicker);
    if (path == null) return;
    try {
      await File(still.pngPath).copy(path);
    } catch (_) {
      app.postNotice(l10n.galleryExportFailed, error: true);
      return;
    }
    app.postNotice(l10n.galleryStillExported);
  }

  /// The still's grade baked into a `.cube` where the user picks.
  Future<void> _exportLut(BridgeStill still) async {
    final comp = ui.selectedComp;
    if (comp == null) return;
    final path = await pickLutSaveLocation(fileSafe(still.name));
    if (path == null) return;
    try {
      await comp.exportStillLut(
          still: still.path,
          name: still.name,
          path: path,
          size: 33,
          space: BridgeLutSpace.video);
    } catch (_) {
      app.postNotice(l10n.gradeLutFailed, error: true);
      return;
    }
    app.postNotice(l10n.gradeLutExported);
  }

  // --- Holding Alt ---

  bool _onKey(KeyEvent e) {
    if (e.logicalKey == LogicalKeyboardKey.altLeft ||
        e.logicalKey == LogicalKeyboardKey.altRight) {
      _preview();
    }
    return false;
  }

  /// The hovered still's grade on the current shot while Alt is down; the
  /// document's picture back the moment either stops.
  void _preview() {
    final still = _hover;
    final shot = ui.gradeShot.value;
    final comp = ui.selectedComp;
    final held = HardwareKeyboard.instance.isAltPressed;
    if (still == null || !held || shot == null || comp == null) {
      if (_previewing) {
        _previewing = false;
        ui.requestFrame();
      }
      return;
    }
    try {
      comp.renderFrameWithPreview(
        frame: ui.playheadFrame.value,
        scale: ui.viewerScale,
        layer: shot.layer,
        effects: shot.layer.stillGradePreview(clip: shot.clip, path: still.path),
      );
      _previewing = true;
    } catch (_) {
      // A still that has gone, or a clip with nothing in its stack.
    }
  }

  void _hoverOn(BridgeStill? still) {
    if (_hover?.path == still?.path) return;
    _hover = still;
    _preview();
  }

  // --- What a still does ---

  void _compare(BridgeStill still) => ui.grade.compareWith(stillReference(still));

  Future<void> _apply(BridgeStill still, {required bool picked}) async {
    final shot = ui.gradeShot.value;
    if (shot == null) return;
    String text;
    try {
      text = await readStillGrade(path: still.path);
    } catch (_) {
      return;
    }
    if (!mounted) return;
    layGrade(app, ui, text, name: 'Apply grade', to: picked ? null : [shot]);
  }

  Future<void> _rename(BridgeStill still) async {
    final name = await askThemeName(context,
        title: l10n.galleryRenameStill,
        suggested: still.name,
        confirm: l10n.rename,
        label: l10n.name);
    if (name == null || name.trim().isEmpty) return;
    try {
      final renamed = await renameStill(path: still.path, name: name.trim());
      _follow(still, renamed);
    } catch (_) {
      // A name taken, or a file that would not move.
    }
    _load();
  }

  Future<void> _move(BridgeStill still, String album) async {
    try {
      final moved = await moveStill(path: still.path, album: album);
      _follow(still, moved);
    } catch (_) {
      // Left where it was.
    }
    _load();
  }

  /// A still that moved on disk is still the Viewer's reference.
  void _follow(BridgeStill was, BridgeStill now) {
    if (_picked == was.path) _picked = now.path;
    if (_picks.remove(was.path)) _picks.add(now.path);
    final r = ui.grade.reference.value;
    if (r?.still == was.path) ui.grade.reference.value = stillReference(now);
  }

  Future<void> _delete(BridgeStill still) async {
    if (!await askBefore(context, l10n.galleryDeleteStillAsk(still.name),
        l10n.delete)) {
      return;
    }
    try {
      await deleteStill(path: still.path);
    } catch (_) {
      // Already gone.
    }
    if (ui.grade.reference.value?.still == still.path) {
      ui.grade.compareWith(null);
    }
    _load();
  }

  Future<void> _newAlbum() async {
    final name = await askThemeName(context,
        title: l10n.galleryNewAlbum,
        suggested: l10n.galleryAlbumN(_albums.length + 1),
        confirm: l10n.create,
        label: l10n.name);
    if (name == null || name.trim().isEmpty) return;
    try {
      _shownAlbum = await makeStillAlbum(name: name.trim());
    } catch (_) {
      // A name taken.
    }
    _load();
  }

  Future<void> _renameAlbum() async {
    final album = _album;
    if (!_albums.contains(album)) return;
    final name = await askThemeName(context,
        title: l10n.galleryRenameAlbum,
        suggested: album,
        confirm: l10n.rename,
        label: l10n.name);
    if (name == null || name.trim().isEmpty) return;
    try {
      _shownAlbum = await renameStillAlbum(album: album, name: name.trim());
    } catch (_) {
      // A name taken.
    }
    _load();
  }

  Future<void> _deleteAlbum() async {
    final album = _album;
    if (!_albums.contains(album)) return;
    if (!await askBefore(
        context, l10n.galleryDeleteAlbumAsk(album), l10n.delete)) {
      return;
    }
    try {
      await deleteStillAlbum(album: album);
    } catch (_) {
      // Already gone.
    }
    _shownAlbum = null;
    _load();
  }

  // --- Building ---

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ListenableBuilder(
      listenable:
          Listenable.merge([ui.grade.shot, ui.grade.reference, ui.model]),
      builder: (context, _) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _header(t),
          _memories(t),
          Expanded(
            child: Container(
              color: t.surface0,
              child: LayoutBuilder(builder: (context, box) => _grid(t, box)),
            ),
          ),
        ],
      ),
    );
  }

  Widget _header(LumitTheme t) {
    final canGrab = ui.gradeShot.value != null && ui.selectedComp != null;
    return Container(
      height: 26,
      color: t.surface1,
      padding: const EdgeInsets.only(left: 4, right: 4),
      child: Row(
        children: [
          Flexible(
            child: Builder(
              builder: (context) => HouseButton(
                key: const ValueKey('gallery-album'),
                frameless: true,
                small: true,
                onPressed: () {
                  final box = context.findRenderObject()! as RenderBox;
                  GradeMenu.show(context,
                      box.localToGlobal(Offset(0, box.size.height + 2)),
                      _albumRows);
                },
                child: Text(
                  _album,
                  style: t.small.copyWith(color: t.textSecondary),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ),
          ),
          const Spacer(),
          GradeIconButton(
            key: const ValueKey('gallery-grab'),
            glyph: LumitIcons.snapshot,
            tip: l10n.galleryGrab,
            onPressed: canGrab ? _grab : null,
          ),
        ],
      ),
    );
  }

  MenuRow _row(String key, String label, VoidCallback close, VoidCallback? run,
          {bool? ticked}) =>
      MenuRow(
        key: ValueKey<String>(key),
        enabled: run != null,
        onPressed: () {
          close();
          run?.call();
        },
        child: MenuLabel(label, ticked: ticked, enabled: run != null),
      );

  List<Widget> _albumRows(VoidCallback close) {
    final real = _albums.contains(_album);
    return [
      for (final a in _albums.isEmpty ? [_firstAlbum] : _albums)
        _row('gallery-album-$a', a, close, () => _show(a), ticked: a == _album),
      const MenuDivider(),
      _row('gallery-album-new', l10n.galleryNewAlbumRow, close, _newAlbum),
      _row('gallery-album-rename', l10n.galleryRenameAlbumRow, close,
          real ? _renameAlbum : null),
      _row('gallery-album-delete', l10n.galleryDeleteAlbum, close,
          real ? _deleteAlbum : null),
    ];
  }

  List<Widget> _stillRows(BridgeStill s, VoidCallback close) {
    final shot = ui.gradeShot.value;
    final many = gradeTargets(ui).length > 1;
    final others = [
      for (final a in _albums)
        if (a != s.album) a
    ];
    return [
      _row('gallery-apply', l10n.galleryApply, close,
          shot == null ? null : () => _apply(s, picked: false)),
      _row('gallery-apply-picked', l10n.galleryApplyPicked, close,
          shot != null && many ? () => _apply(s, picked: true) : null),
      _row('gallery-compare', l10n.galleryCompare, close, () => _compare(s)),
      _row(
          'gallery-grid',
          l10n.gradeGridStills,
          close,
          _picks.length > 1
              ? () => toggleGradeGrid(ui, GradeGrid.stills)
              : null,
          ticked: ui.grade.grid.value == GradeGrid.stills),
      const MenuDivider(),
      _row('gallery-export-still', l10n.galleryExportStill, close,
          () => _exportPicture(s)),
      _row('gallery-export-lut', l10n.galleryExportLut, close,
          ui.selectedComp == null ? null : () => _exportLut(s)),
      const MenuDivider(),
      _row('gallery-rename', l10n.renameEllipsis, close, () => _rename(s)),
      for (final a in others)
        _row('gallery-move-$a', l10n.galleryMoveTo(a), close, () => _move(s, a)),
      _row('gallery-delete', l10n.delete, close, () => _delete(s)),
    ];
  }

  /// Where a still came from, as far as its name doesn't already say: a
  /// still named by its shot and time has nothing to add.
  static String _sourceOf(BridgeStill s) => [
        if (s.shot.isNotEmpty && !s.name.contains(s.shot)) s.shot,
        if (!s.name.contains(shortClock(s.timecode))) s.timecode,
      ].join(' · ');

  static const double _tileMin = 128;
  static const double _gap = 6;

  // --- Memories ---

  /// The eight memories as a strip of slots: a filled one shows the shot it
  /// was kept from. A click lays it on the current shot; a right-click keeps
  /// the current grade in it, or empties it.
  Widget _memories(LumitTheme t) => Container(
        color: t.surface0,
        padding: const EdgeInsets.fromLTRB(_gap, _gap, _gap, 0),
        child: LayoutBuilder(builder: (context, box) {
          const gap = 4.0;
          final slot = ((box.maxWidth - gap * (gradeMemorySlots - 1)) /
                  gradeMemorySlots)
              .clamp(8.0, 72.0);
          return Row(
            children: [
              for (var n = 1; n <= gradeMemorySlots; n++) ...[
                if (n > 1) const SizedBox(width: gap),
                _memory(t, n, slot),
              ],
            ],
          );
        }),
      );

  Widget _memory(LumitTheme t, int n, double width) {
    final memory = gradeMemory(app, ui, n);
    final bytes = memory == null ? null : gradeMemoryPicture(memory);
    final shot = ui.gradeShot.value;
    final graded = shot != null && shotGradeIds(ui, shot).isNotEmpty;
    return GradeMenu(
      rows: (close) => [
        _row('gallery-memory-apply', l10n.galleryMemoryApply, close,
            memory != null && shot != null
                ? () => applyMemory(app, ui, n)
                : null),
        _row('gallery-memory-store', l10n.galleryMemoryStore, close,
            graded ? () => storeMemory(app, ui, n) : null),
        _row('gallery-memory-clear', l10n.galleryMemoryClear, close,
            memory != null ? () => clearMemory(app, ui, n) : null),
      ],
      child: GestureDetector(
        key: ValueKey<String>('gallery-memory-$n'),
        behavior: HitTestBehavior.opaque,
        onTap: memory != null && shot != null
            ? () => applyMemory(app, ui, n)
            : null,
        child: HoverMark(
          builder: (context, over) => Container(
            width: width,
            // Never shorter than its number in a narrow pane.
            height: width * 9 / 16 < 18 ? 18 : width * 9 / 16,
            decoration: BoxDecoration(
              color: memory == null
                  ? Color.lerp(t.surface0, t.surface1, over)
                  : t.surface2,
              borderRadius: BorderRadius.circular(t.tokens.actionRadius),
              border: Border.all(
                  color: memory != null && over > 0.5
                      ? t.hairlineStrong
                      : t.hairline),
            ),
            clipBehavior: Clip.antiAlias,
            child: Stack(
              fit: StackFit.expand,
              children: [
                if (bytes != null)
                  Image.memory(bytes,
                      fit: BoxFit.cover,
                      gaplessPlayback: true,
                      errorBuilder: (context, _, __) =>
                          const SizedBox.shrink()),
                Align(
                  alignment: Alignment.topLeft,
                  child: Container(
                    margin: const EdgeInsets.all(2),
                    padding: const EdgeInsets.symmetric(horizontal: 2),
                    decoration: memory == null
                        ? null
                        : BoxDecoration(
                            color: t.surface1,
                            borderRadius:
                                BorderRadius.circular(t.tokens.actionRadius),
                          ),
                    child: Text('$n',
                        style: t.caption.copyWith(
                            color: memory == null
                                ? t.textDisabled
                                : t.textSecondary)),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _grid(LumitTheme t, BoxConstraints box) {
    if (_stills.isEmpty) return const SizedBox.expand();
    final across =
        ((box.maxWidth - _gap) / (_tileMin + _gap)).floor().clamp(1, 64);
    final tile = (box.maxWidth - _gap * (across + 1)) / across;
    final picture = tile * 9 / 16;
    return MouseRegion(
      onExit: (_) => _hoverOn(null),
      child: EdgeFade(
        axis: Axis.vertical,
        child: GridView.builder(
          primary: false,
          padding: const EdgeInsets.all(_gap),
          gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
            crossAxisCount: across,
            mainAxisSpacing: _gap,
            crossAxisSpacing: _gap,
            mainAxisExtent:
                picture + (_stills.any((s) => _sourceOf(s).isNotEmpty) ? 36 : 22),
          ),
          itemCount: _stills.length,
          itemBuilder: (context, i) =>
              _tile(t, _stills[i], tile, picture),
        ),
      ),
    );
  }

  Widget _tile(LumitTheme t, BridgeStill s, double width, double picture) {
    final picked = _picks.contains(s.path);
    final compared = ui.grade.reference.value?.still == s.path;
    final dpr = MediaQuery.devicePixelRatioOf(context);
    final thumb = SizedBox(
      width: width,
      height: picture,
      child: Container(
        decoration: BoxDecoration(
          color: t.surface1,
          border: Border.all(
              color: picked
                  ? t.accent
                  : compared
                      ? t.hairlineStrong
                      : t.hairline,
              width: picked ? 1.5 : 1),
        ),
        padding: const EdgeInsets.all(1),
        child: Image.file(
          File(s.pngPath),
          key: ValueKey<String>(s.pngPath),
          fit: BoxFit.contain,
          cacheWidth: (width * dpr).round().clamp(16, 1024),
          gaplessPlayback: true,
          errorBuilder: (context, _, __) => const SizedBox.expand(),
        ),
      ),
    );
    return Draggable<GradeStillDrag>(
      key: ValueKey<String>('gallery-still-${s.path}'),
      data: GradeStillDrag(s.path),
      dragAnchorStrategy: pointerDragAnchorStrategy,
      feedback: FloatSurface(
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
          child: Text(s.name, style: t.small),
        ),
      ),
      child: MouseRegion(
        onEnter: (_) => _hoverOn(s),
        onHover: (_) => _hoverOn(s),
        child: GradeMenu(
          rows: (close) {
            // A right-click on a still outside the pick picks it alone.
            if (!_picks.contains(s.path)) _pick(s);
            return _stillRows(s, close);
          },
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTapDown: (d) {
              _pick(s);
              if (_taps.tap(at: d.globalPosition, slop: 8)) _compare(s);
            },
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                thumb,
                const SizedBox(height: 3),
                Row(children: [
                  if (s.label != 0) ...[
                    Container(
                      width: 6,
                      height: 6,
                      decoration: BoxDecoration(
                        color: t.labelColour(s.label),
                        shape: BoxShape.circle,
                      ),
                    ),
                    const SizedBox(width: 4),
                  ],
                  Expanded(
                    child: Text(s.name,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: t.small.copyWith(
                            color: picked ? t.textPrimary : t.textSecondary)),
                  ),
                ]),
                if (_sourceOf(s) case final source when source.isNotEmpty)
                  Text(
                    source,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: t.caption.copyWith(color: t.textMuted),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
