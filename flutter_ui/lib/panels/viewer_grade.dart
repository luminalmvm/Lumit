// The Viewer's grading tools: the reference a grade is held against, the
// matte view, the qualifier's pick circle and the colour chart's grid. The
// windows have a file of their own, viewer_grade_windows.dart.
//
// Each one reads and writes the grading panels' shared state (`ui.grade`), so
// a key, a button in the Grade panel or a still in the Gallery reaches the
// same tool. The compare and the chart cross no bridge at all: the reference
// is a picture drawn over the live one, as the snapshot is. The matte view is
// the engine's and preview only. A pick previews its rows while the drag runs
// and writes them as one undo step when it ends.

import 'dart:async';
import 'dart:io';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart' show kSecondaryButton;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/grade_groups.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/grade_engine.dart';
import '../state/grade_target.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart' show EffectStackEditor;
import 'grade_actions.dart' show gradeListOf, gradeScopeOf;
import 'grade_controls.dart' show GradeIconButton;
import 'timeline_extras_frb.dart' show showMenuAt;
import 'viewer_menus.dart' show viewerRow;
import 'viewer_view_cube.dart' show ViewLookTag;

const GradeEngine _engine = GradeEngine();

// --- The shot and its node ---------------------------------------------------

/// A clip's stack on the comp's clock, read once per document revision.
final Map<String, (int?, List<BridgeEffectInstanceInfo>)> _clipStacks = {};

/// [shot]'s own stack, its values on the comp's clock.
List<BridgeEffectInstanceInfo> gradeStackOf(LumitUiState ui, GradeShot shot) {
  final info = ui.model.byId(shot.layer.internallayerId)?.info;
  if (info == null) return const [];
  final clip = shot.clip;
  if (clip == null) return info.effects;
  final revision = ui.model.revision;
  final held = _clipStacks[shot.key];
  if (held != null && held.$1 == revision) return held.$2;
  List<BridgeEffectInstanceInfo> read;
  try {
    read = [
      for (final e in shot.layer.getClipEffects(clip: clip, onCompClock: true))
        e.getInfo()
    ];
  } catch (_) {
    read = const [];
  }
  _clipStacks[shot.key] = (revision, read);
  return read;
}

/// The list the current node's rows live in on [shot], when it is not the
/// shot's own stack: the group grade in view, or the shared node picked.
({UuidValue group, BridgeGradePart part})? gradeListOn(
        LumitUiState ui, GradeShot shot) =>
    shot.same(ui.gradeShot.value) ? gradeListOf(ui) : null;

/// A group's list or a shared node's, read once per document revision.
(int?, String, List<BridgeEffectInstanceInfo>)? _listRead;

List<BridgeEffectInstanceInfo> _listOf(
    LumitUiState ui, GradeShot shot, ({UuidValue group, BridgeGradePart part}) list) {
  final key = '${list.group}/${list.part.name}';
  final revision = ui.model.revision;
  final held = _listRead;
  if (held != null && held.$1 == revision && held.$2 == key) return held.$3;
  List<BridgeEffectInstanceInfo> read;
  try {
    read = [
      for (final h
          in shot.layer.getGradeGroupEffects(group: list.group, part_: list.part))
        h.getInfo()
    ];
  } catch (_) {
    read = const [];
  }
  _listRead = (revision, key, read);
  return read;
}

/// The node the Grade panel edits on [shot]: the one picked when it is in
/// the stack, else the last Correction, else none. While a group's grade is
/// in view the stack is that grade; a shared node is its one node.
BridgeEffectInstanceInfo? gradeNodeOf(LumitUiState ui, GradeShot shot) {
  final list = gradeListOn(ui, shot);
  final stack = list == null ? gradeStackOf(ui, shot) : _listOf(ui, shot, list);
  final want = ui.gradeNode.value;
  if (want != null) {
    for (final e in stack) {
      if (e.id == want) return e;
    }
    if (list != null && gradeScopeOf(ui) == null) return stack.firstOrNull;
  }
  return stack.where((e) => e.name == correctionName).lastOrNull;
}

/// Rows of one node written from the Viewer: previewed while a drag runs,
/// then the whole lot as one op on release.
class ViewerGradeWriter {
  final EffectStackEditor editor = EffectStackEditor();

  void _aim(LumitUiState ui, GradeShot shot) {
    final clip = shot.clip;
    final list = gradeListOn(ui, shot);
    editor.groupStack = list == null
        ? null
        : () =>
            shot.layer.getGradeGroupEffects(group: list.group, part_: list.part);
    editor.gradeList = list != null;
    editor.clipStack = clip == null || list != null
        ? null
        : () => shot.layer.getClipEffects(clip: clip, onCompClock: true);
  }

  void live(LumitUiState ui, CompositionReference comp, GradeShot shot,
      UuidValue node, Map<String, BridgeEffectValue> rows) {
    _aim(ui, shot);
    final frame = ui.playheadFrame.value;
    for (final e in rows.entries) {
      editor.live(comp, shot.layer, node, e.key, e.value,
          frame: frame, scale: ui.viewerScale);
    }
  }

  void write(LumitState app, LumitUiState ui, GradeShot shot, UuidValue node,
      Map<String, BridgeEffectValue> rows) {
    _aim(ui, shot);
    editor.writeAll(shot.layer, node, rows);
    app.notifyDocumentChanged();
    ui.model.refresh();
  }

  void clear() => editor.clear();
}

// --- Reference compare ---------------------------------------------------------

/// The reference's picture, loaded once a reference is set: a still's PNG,
/// or a shot's frame at the comp's size.
class _ReferencePicture {
  static String? _key;
  static ui.Image? image;
  static final ValueNotifier<int> loaded = ValueNotifier(0);

  static String _keyOf(GradeReference r) =>
      r.stillPicture ?? '${r.shot?.key}@${r.frame}';

  static Future<void> load(CompositionReference comp, GradeReference? r) async {
    final key = r == null ? null : _keyOf(r);
    if (key == _key) return;
    _key = key;
    final old = image;
    image = null;
    loaded.value++;
    WidgetsBinding.instance.addPostFrameCallback((_) => old?.dispose());
    if (r == null) return;
    ui.Image? got;
    try {
      if (r.stillPicture case final png?) {
        final codec =
            await ui.instantiateImageCodec(await File(png).readAsBytes());
        got = (await codec.getNextFrame()).image;
      } else if (r.shot case final shot?) {
        final size = comp.getSize();
        final frame = await comp.shotThumbnail(
            layer: shot.layer.internallayerId,
            clip: shot.clip,
            frame: r.frame,
            maxEdge: math.max(size.width, size.height));
        if (frame != null) {
          final done = Completer<ui.Image>();
          ui.decodeImageFromPixels(frame.rgba, frame.width, frame.height,
              ui.PixelFormat.rgba8888, done.complete);
          got = await done.future;
        }
      }
    } catch (_) {
      // A still whose file has gone, or a shot that has: nothing to draw.
    }
    if (_key != key) {
      got?.dispose();
      return;
    }
    image = got;
    loaded.value++;
  }
}

/// The reference drawn against the live picture: right after the picture in
/// the stage, so a difference is taken against the picture alone and the
/// marks over it stay on top.
class ViewerReferenceLayer extends StatelessWidget {
  final LumitUiState uiState;
  final CompositionReference comp;
  final int viewId;
  final Rect fitted;

  const ViewerReferenceLayer({
    super.key,
    required this.uiState,
    required this.comp,
    required this.viewId,
    required this.fitted,
  });

  @override
  Widget build(BuildContext context) {
    final g = uiState.grade;
    return Positioned.fill(
      child: ListenableBuilder(
        listenable: Listenable.merge([
          g.reference,
          g.comparing,
          g.compareStyle,
          g.compareSwapped,
          g.compareAt,
          _ReferencePicture.loaded,
        ]),
        builder: (context, _) {
          final r = g.reference.value;
          // After the build: loading tells the builder it has started.
          scheduleMicrotask(() => _ReferencePicture.load(comp, r));
          final image = _ReferencePicture.image;
          // The active view of the comp being graded, and nowhere else.
          if (r == null ||
              !g.comparing.value ||
              image == null ||
              fitted.isEmpty ||
              viewId != uiState.activeViewId ||
              comp.internalid != uiState.selectedComp?.internalid) {
            return const SizedBox.shrink();
          }
          return LayoutBuilder(
              builder: (context, box) => _drawn(context, box, r, image));
        },
      ),
    );
  }

  Widget _drawn(
      BuildContext context, BoxConstraints box, GradeReference r, ui.Image image) {
    final g = uiState.grade;
    final t = ThemeScope.of(context).theme;
    final style = g.compareStyle.value;
    final swapped = g.compareSwapped.value;
    final at = g.compareAt.value;
    final size = Size(box.maxWidth, box.maxHeight);
    final across = style == GradeCompare.wipeAcross;
    final x = size.width * at, y = size.height * at;
    final all = Offset.zero & size;
    var rect = fitted;
    var clip = all;
    var blend = BlendMode.srcOver;
    var opacity = 1.0;
    switch (style) {
      case GradeCompare.wipe:
        clip = swapped
            ? Rect.fromLTRB(0, 0, x, size.height)
            : Rect.fromLTRB(x, 0, size.width, size.height);
      case GradeCompare.wipeAcross:
        clip = swapped
            ? Rect.fromLTRB(0, 0, size.width, y)
            : Rect.fromLTRB(0, y, size.width, size.height);
      case GradeCompare.split:
        // The same region of both, side by side: the reference moved along
        // by what the picture's own side shows.
        final shown = x - math.max(fitted.left, 0);
        rect = fitted.shift(Offset(swapped ? -shown : shown, 0));
        clip = swapped
            ? Rect.fromLTRB(0, 0, x, size.height)
            : Rect.fromLTRB(x, 0, size.width, size.height);
      case GradeCompare.difference:
        blend = BlendMode.difference;
      case GradeCompare.mix:
        opacity = at;
    }
    // The reference's side named, as a snapshot's is.
    final tagLeft = switch (style) {
      GradeCompare.wipe || GradeCompare.split => swapped ? null : x + 8,
      _ => null,
    };
    final tagRight = switch (style) {
      GradeCompare.wipe || GradeCompare.split =>
        swapped ? size.width - x + 8 : null,
      GradeCompare.wipeAcross || GradeCompare.difference || GradeCompare.mix =>
        8.0,
    };
    return Stack(
      children: [
        Positioned.fill(
          child: IgnorePointer(
            child: CustomPaint(
              key: const ValueKey('viewer-reference'),
              painter: _ReferencePainter(
                  image, rect, clip, blend, opacity, t.textPrimary),
            ),
          ),
        ),
        if (tagLeft != null)
          Positioned(left: tagLeft, bottom: 8, child: ViewLookTag(look: r.name)),
        if (tagLeft == null && tagRight != null)
          Positioned(
              right: tagRight,
              bottom: across && swapped ? null : 8,
              top: across && swapped ? 8 : null,
              child: ViewLookTag(look: r.name)),
        if (style != GradeCompare.difference)
          across
              ? Positioned(
                  left: 0,
                  right: 0,
                  top: y - _grab / 2,
                  height: _grab,
                  child: _seam(context, t, size.height, y, vertical: false))
              : Positioned(
                  top: 0,
                  bottom: 0,
                  left: x - _grab / 2,
                  width: _grab,
                  child: _seam(context, t, size.width, x, vertical: true)),
      ],
    );
  }

  static const double _grab = 11;

  /// The divider and its grab strip. A drag moves it, a double click puts it
  /// back in the middle, a right click offers the reference's menu.
  Widget _seam(BuildContext context, LumitTheme t, double length, double at,
      {required bool vertical}) {
    final g = uiState.grade;
    void move(double by) {
      if (length <= 0) return;
      g.compareAt.value = ((at + by) / length).clamp(0.0, 1.0);
    }

    return MouseRegion(
      cursor: vertical
          ? SystemMouseCursors.resizeLeftRight
          : SystemMouseCursors.resizeUpDown,
      child: GestureDetector(
        key: const ValueKey('viewer-reference-seam'),
        behavior: HitTestBehavior.opaque,
        onHorizontalDragUpdate: vertical ? (d) => move(d.delta.dx) : null,
        onVerticalDragUpdate: vertical ? null : (d) => move(d.delta.dy),
        onDoubleTap: () => g.compareAt.value = 0.5,
        onSecondaryTapUp: (d) =>
            showReferenceMenu(context, uiState, d.globalPosition),
        child: Center(
          child: vertical
              ? Container(width: 1, color: t.accent)
              : Container(height: 1, color: t.accent),
        ),
      ),
    );
  }
}

class _ReferencePainter extends CustomPainter {
  final ui.Image image;
  final Rect rect, clip;
  final BlendMode blend;
  final double opacity;

  /// Any colour: only its alpha is used, for the mix.
  final Color ink;

  const _ReferencePainter(
      this.image, this.rect, this.clip, this.blend, this.opacity, this.ink);

  @override
  void paint(Canvas canvas, Size size) {
    canvas.save();
    canvas.clipRect(clip);
    canvas.drawImageRect(
      image,
      Rect.fromLTWH(0, 0, image.width.toDouble(), image.height.toDouble()),
      rect,
      Paint()
        ..blendMode = blend
        ..filterQuality = FilterQuality.medium
        ..color = ink.withValues(alpha: opacity.clamp(0.0, 1.0)),
    );
    canvas.restore();
  }

  @override
  bool shouldRepaint(_ReferencePainter old) =>
      old.image != image ||
      old.rect != rect ||
      old.clip != clip ||
      old.blend != blend ||
      old.opacity != opacity;
}

/// What the split-screen grid shows, as its chip and menus name it.
String gradeGridName(GradeGrid g) => switch (g) {
      GradeGrid.versions => l10n.gradeGridVersions,
      GradeGrid.stills => l10n.gradeGridStills,
      GradeGrid.shots => l10n.gradeGridShots,
    };

String gradeCompareName(GradeCompare c) => switch (c) {
      GradeCompare.wipe => l10n.gradeCompareWipe,
      GradeCompare.wipeAcross => l10n.gradeCompareWipeAcross,
      GradeCompare.split => l10n.gradeCompareSplit,
      GradeCompare.difference => l10n.gradeCompareDifference,
      GradeCompare.mix => l10n.gradeCompareMix,
    };

/// The reference's menu: show it or not, how, which side, and clear it.
void showReferenceMenu(BuildContext context, LumitUiState ui, Offset at) {
  final g = ui.grade;
  if (g.reference.value == null) return;
  showMenuAt<void>(
    context: context,
    position: at,
    rows: (closeWith) {
      void close() => closeWith(null);
      return [
        viewerRow(l10n.gradeCompareShow,
            () => g.comparing.value = !g.comparing.value, close,
            id: 'viewer-reference-show', ticked: g.comparing.value),
        const MenuDivider(),
        for (final c in GradeCompare.values)
          viewerRow(gradeCompareName(c), () {
            g.compareStyle.value = c;
            g.comparing.value = true;
          }, close,
              id: 'viewer-reference-${c.name}',
              ticked: g.compareStyle.value == c),
        const MenuDivider(),
        viewerRow(l10n.viewerMenuSwapSides,
            () => g.compareSwapped.value = !g.compareSwapped.value, close,
            id: 'viewer-reference-swap', ticked: g.compareSwapped.value),
        viewerRow(l10n.viewerMenuResetDivider,
            () => g.compareAt.value = 0.5, close,
            id: 'viewer-reference-recentre'),
        viewerRow(l10n.gradeCompareClear, () => g.compareWith(null), close,
            id: 'viewer-reference-clear'),
      ];
    },
  );
}

// --- The Viewer bar's chips ---------------------------------------------------

/// The reference and the matte view, named on the Viewer bar while either is
/// set. A click on the reference shows or hides it; on the matte view, puts
/// the picture back. Each right-clicks for its choices.
class ViewerGradeChips extends StatelessWidget {
  const ViewerGradeChips({super.key});

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final g = ui.grade;
    return ListenableBuilder(
      listenable: Listenable.merge(
          [g.reference, g.comparing, g.compareStyle, g.matte, g.grid]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        final r = g.reference.value;
        final matte = g.matte.value;
        final grid = g.grid.value;
        return Row(mainAxisSize: MainAxisSize.min, children: [
          if (grid != null)
            _chip(
              t,
              key: 'viewer-grid-chip',
              text: gradeGridName(grid),
              lit: true,
              onTap: () => g.grid.value = null,
              onMenu: (at) => showMenuAt<void>(
                context: context,
                position: at,
                rows: (closeWith) => [
                  for (final m in GradeGrid.values)
                    viewerRow(gradeGridName(m), () => g.grid.value = m,
                        () => closeWith(null),
                        id: 'viewer-grid-${m.name}', ticked: m == grid),
                ],
              ),
              trailing: GradeIconButton(
                key: const ValueKey('viewer-grid-chip-clear'),
                glyph: LumitIcons.close,
                tip: l10n.gradeGridClose,
                onPressed: () => g.grid.value = null,
              ),
            ),
          if (r != null)
            _chip(
              t,
              key: 'viewer-reference-chip',
              text: '${r.name} · ${gradeCompareName(g.compareStyle.value)}',
              lit: g.comparing.value,
              onTap: () => g.comparing.value = !g.comparing.value,
              onMenu: (at) => showReferenceMenu(context, ui, at),
              trailing: GradeIconButton(
                key: const ValueKey('viewer-reference-chip-clear'),
                glyph: LumitIcons.close,
                tip: l10n.gradeCompareClear,
                onPressed: () => g.compareWith(null),
              ),
            ),
          if (matte != GradeMatte.off)
            _chip(
              t,
              key: 'viewer-matte-chip',
              text: matte == GradeMatte.key
                  ? l10n.gradeMatteKey
                  : l10n.gradeHighlight,
              lit: true,
              onTap: () => g.matte.value = GradeMatte.off,
              onMenu: (at) => showMenuAt<void>(
                context: context,
                position: at,
                rows: (closeWith) => [
                  for (final m in GradeMatte.values)
                    viewerRow(
                        switch (m) {
                          GradeMatte.off => l10n.gradeMatteOff,
                          GradeMatte.highlight => l10n.gradeHighlight,
                          GradeMatte.key => l10n.gradeMatteKey,
                        },
                        () => g.matte.value = m,
                        () => closeWith(null),
                        id: 'viewer-matte-${m.name}',
                        ticked: m == matte),
                  const MenuDivider(),
                  // What Highlight shows around the key; picking one shows it.
                  for (final b in GradeMatteBackdrop.values)
                    viewerRow(
                        switch (b) {
                          GradeMatteBackdrop.grey => l10n.gradeBackdropGrey,
                          GradeMatteBackdrop.black => l10n.gradeBackdropBlack,
                          GradeMatteBackdrop.white => l10n.gradeBackdropWhite,
                          GradeMatteBackdrop.mono => l10n.gradeBackdropMono,
                        }, () {
                      g.matteBackdrop.value = b;
                      g.matte.value = GradeMatte.highlight;
                    }, () => closeWith(null),
                        id: 'viewer-matte-backdrop-${b.name}',
                        ticked: matte == GradeMatte.highlight &&
                            b == g.matteBackdrop.value),
                ],
              ),
            ),
        ]);
      },
    );
  }

  Widget _chip(LumitTheme t,
          {required String key,
          required String text,
          required bool lit,
          required VoidCallback onTap,
          required void Function(Offset) onMenu,
          Widget? trailing}) =>
      Padding(
        padding: const EdgeInsets.only(left: 6),
        child: GestureDetector(
          key: ValueKey<String>(key),
          behavior: HitTestBehavior.opaque,
          onTap: onTap,
          onSecondaryTapUp: (d) => onMenu(d.globalPosition),
          child: HoverMark(
            builder: (context, over) => Container(
              height: 20,
              constraints: const BoxConstraints(maxWidth: 180),
              padding: EdgeInsets.only(left: 6, right: trailing == null ? 6 : 0),
              decoration: BoxDecoration(
                color: Color.lerp(t.surface1, t.surface2, over),
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                border: Border.all(color: lit ? t.accent : t.hairline),
              ),
              child: Row(mainAxisSize: MainAxisSize.min, children: [
                Flexible(
                  child: Text(text,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: t.small.copyWith(
                          color: lit ? t.textPrimary : t.textMuted)),
                ),
                if (trailing != null) trailing,
              ]),
            ),
          ),
        ),
      );
}

// --- Matte view, pick and chart ---------------------------------------------

/// The grading tools that take the pointer, and the matte view kept in step
/// with the current node. Above the layer controls, so a drag meant for a pick
/// or a corner never moves a layer.
class ViewerGradeTools extends StatefulWidget {
  final LumitUiState uiState;
  final CompositionReference comp;
  final int viewId;
  final Rect fitted;
  final Size compSize;

  /// The windows overlay, built by the stage, which knows the layer's map.
  final Widget windows;

  const ViewerGradeTools({
    super.key,
    required this.uiState,
    required this.comp,
    required this.viewId,
    required this.fitted,
    required this.compSize,
    required this.windows,
  });

  @override
  State<ViewerGradeTools> createState() => _ViewerGradeToolsState();
}

class _ViewerGradeToolsState extends State<ViewerGradeTools> {
  LumitUiState get _ui => widget.uiState;
  GradeTarget get _g => _ui.grade;
  late final Listenable _matteInputs = Listenable.merge(
      [_g.matte, _g.matteBackdrop, _g.shot, _g.node, _g.scope, _ui.model]);

  @override
  void initState() {
    super.initState();
    _matteInputs.addListener(_syncMatte);
    WidgetsBinding.instance.addPostFrameCallback((_) => _syncMatte());
  }

  @override
  void dispose() {
    _matteInputs.removeListener(_syncMatte);
    _writer.clear();
    // A pick cut short still closes the step it opened.
    if (_pickAt != null) _project?.endUndoGroup();
    super.dispose();
  }

  ProjectReference? _project;

  bool get _active =>
      widget.viewId == _ui.activeViewId &&
      widget.comp.internalid == _ui.selectedComp?.internalid;

  // --- Matte view ---

  /// What the engine was last told, so an unchanged state asks nothing.
  (GradeMatte, GradeMatteBackdrop, String, UuidValue?)? _sent;
  GradeShot? _sentShot;

  /// The engine's matte view for this view, as `ui.grade.matte` and the
  /// current node say. A node with no key, or none, turns it off.
  void _syncMatte() {
    if (!mounted || !_active) return;
    final shot = _g.shot.value;
    final node = shot == null ? null : gradeNodeOf(_ui, shot);
    var mode = _g.matte.value;
    if (mode != GradeMatte.off && node?.name != correctionName) {
      // The listener comes straight back here with Off.
      _g.matte.value = GradeMatte.off;
      return;
    }
    final target = shot ?? _sentShot;
    if (target == null) return;
    if (shot == null) mode = GradeMatte.off;
    final backdrop = _g.matteBackdrop.value;
    final want = (mode, backdrop, target.key, node?.id);
    if (want == _sent) return;
    // Off on the old shot first when the shot has moved under a matte view.
    if (_sentShot != null && !_sentShot!.same(target) && _sent?.$1 != GradeMatte.off) {
      _engine.setMatteView(widget.comp, _sentShot!, null, GradeMatte.off,
          view: widget.viewId);
    }
    _engine.setMatteView(widget.comp, target, node?.id, mode,
        view: widget.viewId, backdrop: backdrop);
    _sent = want;
    _sentShot = target;
    _ui.requestFrame();
  }

  // --- Qualifier pick ---

  final ViewerGradeWriter _writer = ViewerGradeWriter();

  /// The drag in hand: where it started and its radius, screen pixels.
  Offset? _pickAt;
  double _pickRadius = 0;
  GradeShot? _pickShot;
  UuidValue? _pickNode;
  GradeRows? _pickRows;
  bool _pickAdded = false;
  bool _solving = false, _again = false, _released = false;
  GradeMatte _matteWas = GradeMatte.off;

  /// A 3D qualifier's stroke in hand, screen pixels: the drag draws a line
  /// rather than a circle while the node keys in 3D.
  List<Offset>? _stroke;

  /// How wide a 3D stroke reads either side of its line, screen pixels.
  static const double _strokeRadius = 6;

  static bool _keys3d(BridgeEffectInstanceInfo? node) =>
      node != null &&
      node.values.any((v) =>
          v.id == 'qual_mode' &&
          v.value is BridgeEffectValue_Choice &&
          (v.value as BridgeEffectValue_Choice).field0 == 4);

  /// Screen to comp pixels.
  double get _scale => widget.fitted.width <= 0
      ? 1
      : widget.compSize.width / widget.fitted.width;
  Offset _toComp(Offset p) => (p - widget.fitted.topLeft) * _scale;
  Offset _toScreen(Offset c) => widget.fitted.topLeft + c / _scale;

  void _pickStart(Offset at) {
    final shot = _g.shot.value;
    final project = context.read<LumitState>().project;
    if (shot == null || project == null || _pickAt != null) return;
    _project = project;
    var node = gradeNodeOf(_ui, shot);
    project.beginUndoGroup();
    project.nameUndoGroup(name: 'Grade');
    _pickAdded = false;
    if (node == null || node.name != correctionName) {
      // A shot with no Correction gets one after its grade, in the same step;
      // a group's grade in view takes it at its end.
      final after = gradeNodesOf(gradeStackOf(_ui, shot)).lastOrNull?.id;
      final scope = gradeScopeOf(_ui);
      final made = scope != null
          ? _addToGroup(shot, scope)
          : _engine.addNode(shot, after, GradeNodeKind.serial);
      if (made == null) {
        project.endUndoGroup();
        return;
      }
      _pickAdded = true;
      _g.setNode(made);
      _ui.model.refresh();
    }
    node = gradeNodeOf(_ui, shot);
    if (node == null) {
      project.endUndoGroup();
      return;
    }
    _matteWas = _g.matte.value;
    if (_matteWas == GradeMatte.off) _g.matte.value = GradeMatte.highlight;
    final stroke = _keys3d(node);
    setState(() {
      _pickAt = at;
      _pickRadius = 0;
      _pickShot = shot;
      _pickNode = node!.id;
      _pickRows = null;
      _released = false;
      _stroke = stroke ? [at] : null;
    });
    unawaited(_solve());
  }

  /// A Correction at the end of a group's grade, as one with the step open.
  UuidValue? _addToGroup(
      GradeShot shot, ({UuidValue group, BridgeGradePart part}) scope) {
    try {
      shot.layer.addGradeGroupEffect(
          group: scope.group, part_: scope.part, name: correctionName);
      return shot.layer
          .getGradeGroupEffects(group: scope.group, part_: scope.part)
          .lastOrNull
          ?.id();
    } catch (_) {
      return null;
    }
  }

  void _pickMove(Offset at) {
    final from = _pickAt;
    if (from == null) return;
    final stroke = _stroke;
    if (stroke != null) {
      if ((at - stroke.last).distance < 3) return;
      setState(() => _stroke = [...stroke, at]);
    } else {
      setState(() => _pickRadius = (at - from).distance);
    }
    unawaited(_solve());
  }

  void _pickEnd() {
    if (_pickAt == null) return;
    _released = true;
    if (!_solving) unawaited(_solve());
  }

  /// One solve at a time: a move while one runs asks again once it lands.
  Future<void> _solve() async {
    if (_solving) {
      _again = true;
      return;
    }
    _solving = true;
    do {
      _again = false;
      final at = _pickAt, shot = _pickShot, node = _pickNode;
      if (at == null || shot == null || node == null) break;
      final circle = GradeCircle(
          _toComp(at), math.max(_pickRadius, _tapRadius) * _scale);
      final stroke = _stroke;
      final mode = _g.pick.value ?? GradePick.pick;
      GradeRows? rows;
      try {
        rows = stroke != null
            ? await _engine.qualifier3dFromStroke(
                widget.comp,
                shot,
                _ui.playheadFrame.value,
                node,
                [for (final p in stroke) _toComp(p)],
                _strokeRadius * _scale,
                mode)
            : await _engine.qualifierFromPick(
                widget.comp, shot, _ui.playheadFrame.value, node, circle, mode);
      } catch (_) {
        rows = null;
      }
      if (!mounted || _pickNode != node) break;
      if (rows != null && rows.isNotEmpty) {
        _pickRows = rows;
        _writer.live(_ui, widget.comp, shot, node, rows);
      }
    } while (_again);
    _solving = false;
    if (_released && mounted) _pickFinish();
  }

  void _pickFinish() {
    final shot = _pickShot, node = _pickNode, rows = _pickRows;
    final project = context.read<LumitState>().project;
    if (shot != null && node != null && rows != null && rows.isNotEmpty) {
      _writer.write(context.read<LumitState>(), _ui, shot, node, rows);
      project?.endUndoGroup();
      if (_stroke case final stroke?) {
        final mode = _g.pick.value ?? GradePick.pick;
        _g.addStroke(
            node,
            GradeStroke([for (final p in stroke) _toComp(p)],
                drop: mode == GradePick.subtract),
            fresh: mode == GradePick.pick);
      }
    } else {
      _writer.clear();
      project?.endUndoGroup();
      if (_pickAdded) {
        project?.undo();
        context.read<LumitState>().notifyDocumentChanged();
        _ui.model.refresh();
      }
      _ui.requestFrame();
    }
    if (_matteWas == GradeMatte.off) _g.matte.value = GradeMatte.off;
    setState(() {
      _pickAt = null;
      _pickShot = null;
      _pickNode = null;
      _pickRows = null;
      _stroke = null;
    });
  }

  /// What a click without a drag reads, screen pixels.
  static const double _tapRadius = 4;

  Widget _pickLayer(LumitTheme t) {
    final at = _pickAt;
    final shot = _g.shot.value;
    final node = shot == null ? null : gradeNodeOf(_ui, shot);
    // A 3D key's strokes so far, faint, and the one being drawn.
    final kept = _keys3d(node)
        ? [
            for (final s in _g.strokes.value[node!.id] ?? const <GradeStroke>[])
              (drop: s.drop, points: [for (final p in s.points) _toScreen(p)]),
          ]
        : null;
    final Widget drawn = kept != null || _stroke != null
        ? CustomPaint(
            key: const ValueKey('viewer-grade-strokes'),
            painter: _StrokePainter(
              kept ?? const [],
              _stroke,
              keep: t.accent,
              drop: t.error,
              under: t.surface0,
            ),
            child: const SizedBox.expand(),
          )
        : at == null
            ? const SizedBox.expand()
            : CustomPaint(
                painter: _CirclePainter(at, math.max(_pickRadius, _tapRadius),
                    t.accent, t.surface0),
              );
    return Positioned.fill(
      child: MouseRegion(
        cursor: SystemMouseCursors.precise,
        child: Listener(
          // A right click puts the tool down.
          onPointerDown: (e) {
            if (e.buttons == kSecondaryButton) _g.pick.value = null;
          },
          child: GestureDetector(
            key: const ValueKey('viewer-grade-pick'),
            behavior: HitTestBehavior.opaque,
            onPanStart: (d) => _pickStart(d.localPosition),
            onPanUpdate: (d) => _pickMove(d.localPosition),
            onPanEnd: (_) => _pickEnd(),
            onPanCancel: _pickEnd,
            onTapUp: (d) {
              _pickStart(d.localPosition);
              _pickEnd();
            },
            child: drawn,
          ),
        ),
      ),
    );
  }

  // --- Chart ---

  /// Which corner a drag holds, 0 to 3 from the top left round clockwise,
  /// or 4 for the whole chart.
  int? _corner;

  /// A centred chart the shape of a ColorChecker, half the picture across.
  List<Offset> _chartDefault() {
    final w = widget.compSize.width, h = widget.compSize.height;
    final cw = w * 0.5, ch = math.min(cw * 2 / 3, h * 0.8);
    final c = Offset(w / 2, h / 2);
    return [
      c + Offset(-cw / 2, -ch / 2),
      c + Offset(cw / 2, -ch / 2),
      c + Offset(cw / 2, ch / 2),
      c + Offset(-cw / 2, ch / 2),
    ];
  }

  Widget _chartLayer(LumitTheme t) {
    var corners = _g.chartCorners.value;
    if (corners == null) {
      corners = _chartDefault();
      final placed = corners;
      // Placed after the frame: a notifier may not change mid-build.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (_g.chartCorners.value == null) _g.chartCorners.value = placed;
      });
    }
    final shown = [for (final c in corners) _toScreen(c)];
    const grip = 14.0;
    void drag(int which, Offset by) {
      final held = _g.chartCorners.value ?? _chartDefault();
      final d = by * _scale;
      _g.chartCorners.value = [
        for (var i = 0; i < 4; i++)
          which == 4 || which == i ? held[i] + d : held[i],
      ];
    }

    return Positioned.fill(
      child: Stack(children: [
        Positioned.fill(
          child: IgnorePointer(
            child: CustomPaint(
              key: const ValueKey('viewer-grade-chart'),
              painter: _ChartPainter(shown, t.accent, t.surface0),
            ),
          ),
        ),
        // The middle moves the whole chart.
        Positioned.fromRect(
          rect: Rect.fromPoints(
              Offset.lerp(shown[0], shown[2], 0.4)!,
              Offset.lerp(shown[0], shown[2], 0.6)!),
          child: MouseRegion(
            cursor: SystemMouseCursors.move,
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onPanUpdate: (d) => drag(4, d.delta),
            ),
          ),
        ),
        for (var i = 0; i < 4; i++)
          Positioned(
            key: ValueKey<String>('viewer-grade-chart-corner-$i'),
            left: shown[i].dx - grip / 2,
            top: shown[i].dy - grip / 2,
            width: grip,
            height: grip,
            child: MouseRegion(
              cursor: SystemMouseCursors.precise,
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onPanStart: (_) => setState(() => _corner = i),
                onPanUpdate: (d) => drag(i, d.delta),
                onPanEnd: (_) => setState(() => _corner = null),
                child: CustomPaint(
                  painter: _HandlePainter(t.accent, t.surface0,
                      lit: _corner == i),
                ),
              ),
            ),
          ),
      ]),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Positioned.fill(
      child: ListenableBuilder(
        listenable: Listenable.merge([
          _g.pick,
          _g.chart,
          _g.chartCorners,
          _g.shot,
          _g.node,
          _g.scope,
          _g.strokes,
          _ui.model,
          _ui.views,
        ]),
        builder: (context, _) {
          if (!_active || widget.fitted.isEmpty || _g.shot.value == null) {
            return const SizedBox.shrink();
          }
          return Stack(children: [
            widget.windows,
            if (_g.chart.value) _chartLayer(t),
            if (_g.pick.value != null || _pickAt != null) _pickLayer(t),
          ]);
        },
      ),
    );
  }
}

/// A 3D key's strokes: the ones already drawn on the node faintly, kept in
/// the accent and dropped in the error colour, and the stroke in hand.
class _StrokePainter extends CustomPainter {
  final List<({bool drop, List<Offset> points})> kept;
  final List<Offset>? live;
  final Color keep, drop, under;

  const _StrokePainter(this.kept, this.live,
      {required this.keep, required this.drop, required this.under});

  static Path _path(List<Offset> points) {
    final path = Path()..moveTo(points.first.dx, points.first.dy);
    for (final p in points.skip(1)) {
      path.lineTo(p.dx, p.dy);
    }
    return path;
  }

  void _line(Canvas canvas, List<Offset> points, Color colour, double width) {
    if (points.isEmpty) return;
    final paint = Paint()
      ..style = PaintingStyle.stroke
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round
      ..strokeWidth = width
      ..color = colour;
    if (points.length == 1) {
      canvas.drawCircle(points.first, width / 2,
          Paint()..color = colour);
      return;
    }
    canvas.drawPath(_path(points), paint);
  }

  @override
  void paint(Canvas canvas, Size size) {
    for (final s in kept) {
      _line(canvas, s.points, (s.drop ? drop : keep).withValues(alpha: 0.55),
          2);
    }
    if (live case final points?) {
      _line(canvas, points, under.withValues(alpha: 0.6), 5);
      _line(canvas, points, keep, 2);
    }
  }

  @override
  bool shouldRepaint(_StrokePainter old) => true;
}

/// A pick's circle: the samples it reads.
class _CirclePainter extends CustomPainter {
  final Offset at;
  final double radius;
  final Color line, under;
  const _CirclePainter(this.at, this.radius, this.line, this.under);

  @override
  void paint(Canvas canvas, Size size) {
    final stroke = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 3
      ..color = under.withValues(alpha: 0.6);
    canvas.drawCircle(at, radius, stroke);
    canvas.drawCircle(
        at,
        radius,
        stroke
          ..strokeWidth = 1.2
          ..color = line);
    canvas.drawCircle(at, 1.5, Paint()..color = line);
  }

  @override
  bool shouldRepaint(_CirclePainter old) =>
      old.at != at || old.radius != radius || old.line != line;
}

/// The chart's outline, its 6 by 4 grid, and the middle of each patch, which
/// is where the solver reads.
class _ChartPainter extends CustomPainter {
  final List<Offset> corners;
  final Color line, under;
  const _ChartPainter(this.corners, this.line, this.under);

  Offset _at(double u, double v) => Offset.lerp(
      Offset.lerp(corners[0], corners[1], u)!,
      Offset.lerp(corners[3], corners[2], u)!,
      v)!;

  @override
  void paint(Canvas canvas, Size size) {
    final thin = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1
      ..color = line.withValues(alpha: 0.7);
    final shade = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 3
      ..color = under.withValues(alpha: 0.5);
    final outline = Path()..addPolygon(corners, true);
    canvas.drawPath(outline, shade);
    canvas.drawPath(outline, thin..strokeWidth = 1.2);
    thin.strokeWidth = 0.8;
    for (var i = 1; i < 6; i++) {
      canvas.drawLine(_at(i / 6, 0), _at(i / 6, 1), thin);
    }
    for (var j = 1; j < 4; j++) {
      canvas.drawLine(_at(0, j / 4), _at(1, j / 4), thin);
    }
    final patch = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1
      ..color = line;
    for (var j = 0; j < 4; j++) {
      for (var i = 0; i < 6; i++) {
        final (u0, u1) = ((i + 0.25) / 6, (i + 0.75) / 6);
        final (v0, v1) = ((j + 0.25) / 4, (j + 0.75) / 4);
        canvas.drawPath(
            Path()
              ..addPolygon(
                  [_at(u0, v0), _at(u1, v0), _at(u1, v1), _at(u0, v1)], true),
            patch);
      }
    }
  }

  @override
  bool shouldRepaint(_ChartPainter old) =>
      old.line != line || old.corners.join() != corners.join();
}

/// A square grip on a handle, filled while held.
class _HandlePainter extends CustomPainter {
  final Color line, under;
  final bool lit;
  const _HandlePainter(this.line, this.under, {this.lit = false});

  @override
  void paint(Canvas canvas, Size size) {
    final r = Rect.fromCenter(
        center: size.center(Offset.zero), width: 7, height: 7);
    canvas.drawRect(r, Paint()..color = lit ? line : under);
    canvas.drawRect(
        r,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.2
          ..color = line);
  }

  @override
  bool shouldRepaint(_HandlePainter old) =>
      old.lit != lit || old.line != line || old.under != under;
}
