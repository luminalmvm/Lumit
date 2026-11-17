// The Viewer's split-screen grid: the current shot's versions, the stills
// picked in the Gallery, or the picked shots, side by side in a grid over the
// Viewer, each named. Two stand side by side, or one over the other in a tall
// Viewer; three or four two by two, more three by three. Each cell keeps the
// picture's shape.
// A double-click lays the cell's grade on the current shot: the version made
// live, the still's grade, or the other shot's grade.
//
// The pictures are the ones already made for the Shots panel and the Gallery:
// a shot's still with its grade, a version's in its place, a still's PNG.

import 'dart:async';
import 'dart:io';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/grade_shots.dart';
import 'package:lumit_flutter/src/rust/api/grade_versions.dart';
import 'package:lumit_flutter/src/rust/api/state.dart' show BridgeRenderedFrame;
import 'package:provider/provider.dart';

import '../state/grade_target.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart' show ThemeScope;
import '../widgets/escape_ladder.dart';
import 'grade_actions.dart';
import 'grade_clipboard.dart';
import 'timeline_extras_frb.dart' show DoubleTap;
import 'viewer_view_cube.dart' show ViewLookTag;

/// One cell: what it shows, what it is called, whether it is the one in use,
/// and what a double-click does.
typedef _Cell = ({
  String key,
  String name,
  bool lit,
  Future<ui.Image?> Function() picture,
  void Function() apply,
});

class ViewerGradeGrid extends StatefulWidget {
  final LumitUiState uiState;
  final CompositionReference comp;
  final int viewId;
  final Rect fitted;

  const ViewerGradeGrid({
    super.key,
    required this.uiState,
    required this.comp,
    required this.viewId,
    required this.fitted,
  });

  @override
  State<ViewerGradeGrid> createState() => _ViewerGradeGridState();
}

class _ViewerGradeGridState extends State<ViewerGradeGrid> {
  /// Pictures by cell, kept while the grid is up; the old one stays until
  /// its replacement is in.
  final Map<String, (String, ui.Image)> _pictures = {};
  final Set<String> _asked = {};
  final DoubleTap _taps = DoubleTap();

  LumitUiState get _ui => widget.uiState;

  late final VoidCallback _escapeRelease;

  @override
  void initState() {
    super.initState();
    // Escape puts the picture back, as the chip's cross does: after a drag
    // or a menu, before a selection.
    _escapeRelease = EscapeLadder.register(EscapeRung.dialog, () {
      final grid = _ui.grade.grid;
      if (grid.value == null) return false;
      grid.value = null;
      _ui.requestFrame();
      return true;
    });
  }

  @override
  void dispose() {
    _escapeRelease();
    for (final p in _pictures.values) {
      p.$2.dispose();
    }
    super.dispose();
  }

  /// The most cells a grid shows.
  static const int _most = 9;

  List<_Cell> _cells(LumitState app) {
    final g = _ui.grade;
    final shot = _ui.gradeShot.value;
    final frame = _ui.playheadFrame.value;
    final comp = widget.comp;
    switch (g.grid.value) {
      case GradeGrid.versions:
        if (shot == null) return const [];
        List<BridgeGradeVersion> versions;
        try {
          versions = shot.layer.getGradeVersions(clip: shot.clip);
        } catch (_) {
          versions = const [];
        }
        return [
          for (final v in versions.take(_most))
            (
              key: 'version/${v.id}',
              name: v.name,
              lit: v.live,
              picture: () async => _decoded(await comp.shotVersionThumbnail(
                  layer: shot.layer.internallayerId,
                  clip: shot.clip,
                  version: v.id,
                  frame: frame,
                  maxEdge: _edge)),
              apply: () {
                try {
                  shot.layer.switchGradeVersion(clip: shot.clip, version: v.id);
                } catch (_) {
                  return;
                }
                app.notifyDocumentChanged();
                _ui.model.refresh();
              },
            ),
        ];
      case GradeGrid.stills:
        return [
          for (final r in g.pickedStills.value.take(_most))
            (
              key: 'still/${r.still}',
              name: r.name,
              lit: false,
              picture: () async {
                final codec = await ui.instantiateImageCodec(
                    await File(r.stillPicture!).readAsBytes(),
                    targetWidth: _edge);
                return (await codec.getNextFrame()).image;
              },
              apply: () {
                if (shot != null) {
                  applyStillGrade(app, _ui, r.still!, [shot]);
                }
              },
            ),
        ];
      case GradeGrid.shots:
        final all = compShots(_ui);
        final picked = gradeTargets(_ui);
        final numbers = {
          for (final (i, s) in all.indexed) shotOf(s).key: i + 1,
        };
        return [
          for (final s in all
              .where((s) => picked.any((p) => p.same(shotOf(s))))
              .take(_most))
            (
              key: 'shot/${shotOf(s).key}',
              name: '${numbers[shotOf(s).key]}  ${s.name}',
              lit: shot != null && shot.same(shotOf(s)),
              picture: () async => _decoded(await comp.shotThumbnail(
                  layer: s.layer.internallayerId,
                  clip: s.clip,
                  frame: _frameIn(s, frame),
                  maxEdge: _edge)),
              apply: () => _applyShot(app, s),
            ),
        ];
      case null:
        return const [];
    }
  }

  /// How big a cell's picture is made.
  static const int _edge = 640;

  static int _frameIn(BridgeShot s, int frame) =>
      frame >= s.startFrame && frame < s.endFrame
          ? frame
          : (s.startFrame + s.endFrame) ~/ 2;

  /// [from]'s grade in place of the current shot's.
  void _applyShot(LumitState app, BridgeShot from) {
    final shot = _ui.gradeShot.value;
    if (shot == null || shot.same(shotOf(from))) return;
    final ids = shotGradeIds(_ui, shotOf(from));
    try {
      layGrade(app, _ui, from.layer.copyEffects(effects: ids),
          name: 'Apply grade', to: [shot]);
    } catch (_) {
      // The shot went.
    }
  }

  static Future<ui.Image?> _decoded(BridgeRenderedFrame? got) async {
    if (got == null || got.width == 0) return null;
    final done = Completer<ui.Image>();
    ui.decodeImageFromPixels(got.rgba, got.width, got.height,
        ui.PixelFormat.rgba8888, done.complete);
    return done.future;
  }

  /// [cell]'s picture for what the document is now, asked for once.
  ui.Image? _pictureOf(_Cell cell, String sign) {
    final held = _pictures[cell.key];
    final want = '${cell.key}#$sign';
    if (held?.$1 != want && _asked.add(want)) {
      unawaited(() async {
        ui.Image? got;
        try {
          got = await cell.picture();
        } catch (_) {
          got = null;
        }
        _asked.remove(want);
        if (!mounted || got == null) {
          got?.dispose();
          return;
        }
        final old = _pictures[cell.key];
        _pictures[cell.key] = (want, got);
        setState(() {});
        WidgetsBinding.instance.addPostFrameCallback((_) => old?.$2.dispose());
      }());
    }
    return held?.$2;
  }

  @override
  Widget build(BuildContext context) {
    final g = _ui.grade;
    return Positioned.fill(
      child: ListenableBuilder(
        listenable: Listenable.merge(
            [g.grid, g.pickedStills, g.shot, _ui.model, _ui.playheadFrame]),
        builder: (context, _) {
          final fitted = widget.fitted;
          if (g.grid.value == null ||
              fitted.isEmpty ||
              widget.viewId != _ui.activeViewId ||
              widget.comp.internalid != _ui.selectedComp?.internalid) {
            return const SizedBox.shrink();
          }
          final app = Provider.of<LumitState>(context, listen: false);
          final cells = _cells(app);
          final t = ThemeScope.of(context).theme;
          // Grades and versions change with the document, stills don't.
          final sign = '${_ui.model.revision}@${_ui.playheadFrame.value}';
          return LayoutBuilder(builder: (context, box) {
            final area = (Offset.zero & box.biggest).deflate(_gap);
            final aspect = fitted.width / fitted.height;
            final (across, down, cell) = _grid(cells.length, area.size, aspect);
            final left = area.left +
                (area.width - across * cell.width - (across - 1) * _gap) / 2;
            // Centred on the rows the cells fill.
            final rows = math.min(down, (cells.length / across).ceil());
            final top = area.top +
                (area.height - rows * cell.height - (rows - 1) * _gap) / 2;
            return Stack(
              children: [
                // The picture under the grid is put out of sight.
                Positioned.fill(child: ColoredBox(color: t.surface0)),
                for (final (i, c) in cells.indexed)
                  Positioned(
                    left: left + (i % across) * (cell.width + _gap),
                    top: top + (i ~/ across) * (cell.height + _gap),
                    width: cell.width,
                    height: cell.height,
                    child: _cell(t, c, sign),
                  ),
              ],
            );
          });
        },
      ),
    );
  }

  static const double _gap = 4;

  /// How [count] cells of the picture's [aspect] sit in [room]: how many
  /// across and down, and each cell's size. Two take whichever way round
  /// draws them bigger.
  static (int, int, Size) _grid(int count, Size room, double aspect) {
    Size fit(int across, int down) {
      final w = (room.width - _gap * (across - 1)) / across;
      final h = (room.height - _gap * (down - 1)) / down;
      final cw = math.max(0.0, math.min(w, h * aspect));
      return Size(cw, cw / aspect);
    }

    final (across, down) = switch (count) {
      <= 1 => (1, 1),
      2 => fit(2, 1).width >= fit(1, 2).width ? (2, 1) : (1, 2),
      <= 4 => (2, 2),
      _ => (3, 3),
    };
    return (across, down, fit(across, down));
  }

  Widget _cell(LumitTheme t, _Cell c, String sign) {
    final image = _pictureOf(c, c.key.startsWith('still/') ? '' : sign);
    return GestureDetector(
      key: ValueKey<String>('viewer-grid-${c.key}'),
      behavior: HitTestBehavior.opaque,
      onTapDown: (d) {
        if (_taps.tap(at: d.globalPosition, slop: 8)) c.apply();
      },
      child: Container(
        decoration: BoxDecoration(
          color: t.surface1,
          border: Border.all(
              color: c.lit ? t.accent : t.hairline, width: c.lit ? 1.5 : 1),
        ),
        child: Stack(
          fit: StackFit.expand,
          children: [
            if (image != null) RawImage(image: image, fit: BoxFit.contain),
            Positioned(
              left: 6,
              bottom: 6,
              right: 6,
              child: Align(
                alignment: Alignment.bottomLeft,
                child: ViewLookTag(look: c.name),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
