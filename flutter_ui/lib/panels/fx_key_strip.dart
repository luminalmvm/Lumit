// The keyframe strip at the right of the Effect controls panel: a small
// timeline that spans just the selected layer, with a ruler, the playhead and
// one lane per parameter row, so a key can be seen and moved without leaving
// the panel.
//
// The strip is not one widget. Each row draws its own lane in a column the
// rows reserve at their right edge ([FxKeyLane], through `fxTwoColumnRow`),
// which is what keeps a lane exactly on its row through any scroll. The host
// ([FxKeyStripHost]) draws what runs across all of them: the ground, the
// ruler, the playhead and the divider, and holds the selection and the drag
// the lanes share. The lanes themselves are the Timeline's own [KeyLane], so
// a key is picked, dragged and dropped here exactly as it is there.
//
// In a panel too narrow for a column beside the rows the strip is stacked
// instead: the ruler runs the panel's width and each animated row carries its
// lane on a line of its own beneath it ([FxKeyStripScope.stackedLane]).
//
// The span the strip covers and the offset its keys are placed at are plain
// numbers handed to the host. A layer passes its in and out points and no
// offset; a clip inside a Sequence layer, whose keys are kept relative to the
// clip, would pass the clip's span and its start.

import 'dart:collection' show UnmodifiableSetView;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart'
    show PointerDeviceKind, kPrimaryButton, kTouchSlop;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart' show iconSize;
import '../icons/lumit_icon.dart';
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/comp_time.dart' show sampledScalar;
import '../state/dock.dart' show Panel;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'fx_section.dart';
import 'graph_maths.dart' show keyframeAmong;
import 'key_block.dart';
import 'layer_fold_frb.dart'
    show asOneUndoStep, laneKeyFrame, timeOfSubframe;
import 'timeline_extras_frb.dart'
    show
        PlayheadMarker,
        TimelineAxis,
        rulerLabelOf,
        rulerLabelStepSeconds,
        rulerMinorStepSeconds,
        showMenuAt;
import 'timeline_key_block_frb.dart' show LaneKeysPainter;
import 'timeline_key_lane_frb.dart' show KeyLane;
import 'timeline_metrics_frb.dart' show keyShapeOf, laneKeyHalf;

/// The narrowest the strip is let get, and what it opens at.
const double fxKeyStripMinWidth = 120;
const double fxKeyStripDefaultWidth = 220;

/// The ruler band's height above the lanes.
const double fxKeyStripRulerHeight = 20;

/// Below this panel width the strip no longer stands beside the rows: the
/// name column, a usable control column and the narrowest strip do not fit
/// side by side. It is stacked from here down, a lane under each animated
/// row.
const double fxKeyStripMinPanelWidth = 560;

/// How tall a stacked lane's line is, under its row.
const double fxKeyStripStackedLane = 16;

/// The strip's inset from the panel's right edge: the row padding every
/// section wears, and the card's margin and border under a round shape.
double fxKeyStripInset(LumitTheme t) =>
    10 + (t.tokens.cardRadius > 0 ? 7 : 0);

/// What one row told the host about its keys: the channels the lane draws the
/// lead of, and the write that commits all of them at once.
typedef _LaneEntry = ({
  List<BridgeScalar> scalars,
  ValueChanged<List<BridgeScalar>> onWrite,
});

/// The strip's geometry and shared state, carried down to every row.
///
/// [width] is how wide a lane is, zero while the strip is hidden. Beside the
/// rows it is also the column each row reserves. [stacked] says the lanes
/// are under their rows instead, the panel's width each, and [reveal] is how
/// far open the strip is, which a stacked lane takes as its height.
/// [spanStart] is the comp frame the strip's left edge stands on and
/// [axis] runs over the span's frames. [keyOffset] is added to every key's
/// own time before it is placed, so keys stored relative to something can be
/// shown on the comp's clock.
class FxKeyStripScope extends InheritedWidget {
  final double width;
  final bool stacked;
  final double reveal;
  final ValueListenable<int> playhead;
  final TimelineAxis axis;
  final int spanStart;
  final int keyOffset;
  final double fps;
  final int fpsNum;
  final int fpsDen;
  final ValueNotifier<Set<String>> selected;
  final ValueNotifier<KeyStretch?> stretch;
  final _FxKeyStripHostState _host;

  const FxKeyStripScope._({
    required this.width,
    required this.stacked,
    required this.reveal,
    required this.playhead,
    required this.axis,
    required this.spanStart,
    required this.keyOffset,
    required this.fps,
    required this.fpsNum,
    required this.fpsDen,
    required this.selected,
    required this.stretch,
    required _FxKeyStripHostState host,
    required super.child,
  }) : _host = host;

  /// The keys picked on [rowKey]'s lane, by index.
  Set<int> pickedOn(String rowKey) => {
        for (final id in selected.value)
          if (id.startsWith('$rowKey#'))
            if (int.tryParse(id.substring(rowKey.length + 1)) case final i?) i,
      };

  static FxKeyStripScope? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<FxKeyStripScope>();

  /// The column a row reserves for its lane: zero outside the strip, and
  /// zero while it is stacked, where a lane has a line of its own.
  static double widthOf(BuildContext context) {
    final scope = maybeOf(context);
    return scope == null || scope.stacked ? 0 : scope.width;
  }

  /// [lane] as the line a row carries beneath it while the strip is stacked,
  /// or null where the row has none to show: beside the rows, hidden, or a
  /// row that is not animated.
  static Widget? stackedLane(BuildContext context, Widget? lane) {
    final scope = maybeOf(context);
    if (scope == null || !scope.stacked || scope.width <= 0) return null;
    if (lane is! FxKeyLane || !lane.animated) return null;
    final t = ThemeScope.of(context).theme;
    return Align(
      alignment: Alignment.centerRight,
      child: SizedBox(
        width: scope.width,
        height: fxKeyStripStackedLane * scope.reveal,
        child: ClipRect(
          child: Stack(
            children: [
              // A step down from the panel, so the line reads as time.
              Positioned.fill(
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    color: t.surface0,
                    border: Border(top: BorderSide(color: t.hairline)),
                  ),
                ),
              ),
              Positioned.fill(child: lane),
              Positioned.fill(
                child: IgnorePointer(
                  child: ValueListenableBuilder<int>(
                    valueListenable: scope.playhead,
                    builder: (context, frame, _) {
                      final rel = frame - scope.spanStart;
                      if (rel < 0 || rel > scope.axis.frames) {
                        return const SizedBox.shrink();
                      }
                      return Stack(children: [
                        Positioned(
                          left: (scope.axis.xOf(rel) - 0.5)
                              .clamp(0.0, scope.width - 1),
                          top: 0,
                          bottom: 0,
                          width: 1,
                          child: ColoredBox(color: t.accent),
                        ),
                      ]);
                    },
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  /// How far a key's own frame is moved to land on the strip's axis.
  int get shift => keyOffset - spanStart;

  @override
  bool updateShouldNotify(FxKeyStripScope old) =>
      width != old.width ||
      stacked != old.stacked ||
      reveal != old.reveal ||
      spanStart != old.spanStart ||
      keyOffset != old.keyOffset ||
      axis.frames != old.axis.frames ||
      fps != old.fps;
}

/// The strip's host: the ground, the ruler, the playhead and the divider,
/// around [child], which is the panel's rows.
class FxKeyStripHost extends StatefulWidget {
  /// The comp frames the strip covers: where it starts and how many.
  final int spanStart;
  final int spanFrames;

  /// Added to every key's time before it is placed. Zero for a layer's own
  /// keys.
  final int keyOffset;
  final Widget child;

  const FxKeyStripHost({
    super.key,
    required this.spanStart,
    required this.spanFrames,
    this.keyOffset = 0,
    required this.child,
  });

  @override
  State<FxKeyStripHost> createState() => _FxKeyStripHostState();
}

class _FxKeyStripHostState extends State<FxKeyStripHost>
    with SingleTickerProviderStateMixin {
  /// The lanes on screen, by row key, so a Delete or a drag that reaches
  /// across rows can write each one through its own row.
  final Map<String, _LaneEntry> _lanes = {};

  /// The picked keys, as `rowKey#index`, and the view of them the lanes
  /// read. A view onto the live set rather than a copy, for the reason the
  /// Timeline keeps one: a key drag captures the selection the instant the
  /// pointer goes down, just after the press has added the key under it, and
  /// a copy taken at the last build would not hold it yet.
  final Set<String> _selection = {};
  late final ValueNotifier<Set<String>> _selected =
      ValueNotifier(UnmodifiableSetView(_selection));
  final ValueNotifier<KeyStretch?> _stretch = ValueNotifier(null);

  /// Tell the lanes to look at the selection again.
  void _publish() => _selected.value = UnmodifiableSetView(_selection);

  /// How far open the strip is, 0 to 1. Runs on the theme's reveal.
  late final AnimationController _open = AnimationController(vsync: this);

  /// The divider's travel while it is in hand, or null at rest.
  double? _dragging;

  LumitUiState? _ui;
  bool Function()? _priorDeleteClaim;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (identical(ui, _ui)) return;
    _unbind();
    _ui = ui;
    // Delete takes the picked keys while this panel is focused, chained in
    // front of the panel's own claim on its effects: a key is finer than the
    // effect it sits on, and nothing picked here falls through.
    _priorDeleteClaim = ui.deleteClaim;
    ui.deleteClaim = _deleteClaim;
  }

  void _unbind() {
    if (_ui?.deleteClaim == _deleteClaim) _ui!.deleteClaim = _priorDeleteClaim;
  }

  @override
  void dispose() {
    _unbind();
    _open.dispose();
    _selected.dispose();
    _stretch.dispose();
    super.dispose();
  }

  bool _deleteClaim() {
    final ui = _ui;
    if (!mounted ||
        ui == null ||
        ui.activePanel != Panel.effectControls ||
        _selection.isEmpty) {
      return _priorDeleteClaim?.call() ?? false;
    }
    return _deleteSelected() || (_priorDeleteClaim?.call() ?? false);
  }

  void _register(String rowKey, _LaneEntry entry) => _lanes[rowKey] = entry;
  void _unregister(String rowKey, _LaneEntry entry) {
    if (_lanes[rowKey] == entry) _lanes.remove(rowKey);
  }

  /// One key picked: alone, or toggled in and out when [additive], as the
  /// Timeline's lanes do it.
  void _select(String id, bool additive) {
    if (additive) {
      if (!_selection.remove(id)) _selection.add(id);
    } else {
      _selection
        ..clear()
        ..add(id);
    }
    _publish();
  }

  void _deselectAll() {
    if (_selection.isEmpty) return;
    _selection.clear();
    _publish();
  }

  /// The picked keys by row: each row's indices.
  Map<String, Set<int>> _byRow(Iterable<String> ids) {
    final out = <String, Set<int>>{};
    for (final id in ids) {
      final cut = id.lastIndexOf('#');
      final index = cut < 0 ? null : int.tryParse(id.substring(cut + 1));
      if (index == null) continue;
      (out[id.substring(0, cut)] ??= {}).add(index);
    }
    return out;
  }

  /// The release of a lane key's drag: every key it carried written where it
  /// travelled, one undo step. Each row rewrites all of its channels, so a
  /// point's two axes move together, and a row whose times would no longer
  /// ascend is left as it was.
  void _moveKeys(KeyStretch moved) {
    final scope = _scopeValues;
    final rows = _byRow(moved.keys);
    final project = Provider.of<LumitState>(context, listen: false).project;
    asOneUndoStep(project, () {
      for (final row in rows.entries) {
        final lane = _lanes[row.key];
        if (lane == null) continue;
        final next = <BridgeScalar>[];
        var ok = true;
        for (final scalar in lane.scalars) {
          if (scalar is! BridgeScalar_Keyframed) {
            next.add(scalar);
            continue;
          }
          final keys = scalar.field0;
          final retimed = [
            for (var i = 0; i < keys.length; i++)
              if (row.value.contains(i))
                BridgeKeyframe(
                  time: timeOfSubframe(
                      moved.frameOf(laneKeyFrame(keys[i], scope.fps),
                          whole: true),
                      scope.fpsNum,
                      scope.fpsDen),
                  value: keys[i].value,
                  interpIn: keys[i].interpIn,
                  interpOut: keys[i].interpOut,
                )
              else
                keys[i],
          ];
          for (var i = 1; i < retimed.length; i++) {
            if (laneKeyFrame(retimed[i], scope.fps) <=
                laneKeyFrame(retimed[i - 1], scope.fps)) {
              ok = false;
            }
          }
          next.add(BridgeScalar.keyframed(retimed));
        }
        if (ok) lane.onWrite(next);
      }
    });
  }

  /// Take the picked keys away, one undo step. A channel whose last key goes
  /// is left static at what that key held, as the graph's Delete leaves it.
  bool _deleteSelected() {
    final rows = _byRow(_selection);
    final project = Provider.of<LumitState>(context, listen: false).project;
    var changed = false;
    asOneUndoStep(project, () {
      for (final row in rows.entries) {
        final lane = _lanes[row.key];
        if (lane == null) continue;
        final next = <BridgeScalar>[];
        for (final scalar in lane.scalars) {
          if (scalar is! BridgeScalar_Keyframed) {
            next.add(scalar);
            continue;
          }
          final keys = scalar.field0;
          final rest = [
            for (var i = 0; i < keys.length; i++)
              if (!row.value.contains(i)) keys[i],
          ];
          if (rest.length == keys.length) {
            next.add(scalar);
            continue;
          }
          final last =
              keys[row.value.where((i) => i < keys.length).last].value;
          next.add(rest.isEmpty
              ? BridgeScalar.static_(last)
              : BridgeScalar.keyframed(rest));
        }
        lane.onWrite(next);
        changed = true;
      }
    });
    _selection.clear();
    _publish();
    return changed;
  }

  /// Plant a key on [rowKey] at comp frame [frame], holding what each channel
  /// already reads there, so nothing moves. Nothing on a frame already keyed.
  void _plantKey(String rowKey, int frame) {
    final lane = _lanes[rowKey];
    if (lane == null) return;
    final scope = _scopeValues;
    final time = timeOfSubframe(frame.toDouble(), scope.fpsNum, scope.fpsDen);
    final next = <BridgeScalar>[];
    for (final scalar in lane.scalars) {
      if (scalar is! BridgeScalar_Keyframed) return;
      final keys = scalar.field0;
      if (keys.any((k) => laneKeyFrame(k, scope.fps).round() == frame)) return;
      final added = [...keys, keyframeAmong(keys, time, sampledScalar(scalar, time))]
        ..sort((a, b) => laneKeyFrame(a, scope.fps)
            .compareTo(laneKeyFrame(b, scope.fps)));
      next.add(BridgeScalar.keyframed(added));
    }
    lane.onWrite(next);
  }

  /// The key's own menu, on a right-click.
  Future<void> _keyMenu(
      BuildContext context, String id, Offset position) async {
    if (!_selection.contains(id)) _select(id, false);
    final picked = await showMenuAt<String>(
      context: context,
      position: position,
      rows: (close) => [
        MenuRow(
          key: const ValueKey('fx-key-menu-delete'),
          onPressed: () => close('delete'),
          child: Text(l10n.deleteKey),
        ),
      ],
    );
    if (picked == 'delete' && mounted) _deleteSelected();
  }

  ({double fps, int fpsNum, int fpsDen}) get _scopeValues {
    final model = Provider.of<LumitUiState>(context, listen: false).model;
    final (num, den) = model.fpsExact;
    return (fps: model.fps, fpsNum: num, fpsDen: den);
  }

  /// Drive the strip open or shut on the theme's reveal.
  void _aim(bool open, ThemeScope scope) {
    final target = open ? 1.0 : 0.0;
    if (_open.value == target ||
        (_open.isAnimating && (_open.velocity > 0) == open)) {
      return;
    }
    final spec = scope.motion.reveal;
    if (spec.isStill) {
      _open.value = target;
      return;
    }
    _open.animateTo(target, duration: spec.duration, curve: spec.curve);
  }

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final scope = ThemeScope.of(context);
    return ListenableBuilder(
      listenable: ui.workspace,
      builder: (context, _) => LayoutBuilder(
        builder: (context, constraints) {
          final ws = ui.workspace;
          final room = constraints.maxWidth;
          final stacked = room < fxKeyStripMinPanelWidth;
          _aim(ws.fxKeyStrip, scope);
          // The widest the strip may be: the name column and a usable
          // control column keep theirs.
          final most = (room - fxNameColumnWidth - 200 - 2 * fxKeyStripInset(scope.theme))
              .clamp(fxKeyStripMinWidth, double.infinity);
          final full = ((_dragging == null
                      ? ws.fxKeyStripWidth
                      : ws.fxKeyStripWidth - _dragging!)
                  .clamp(fxKeyStripMinWidth, most))
              .toDouble();
          return AnimatedBuilder(
            animation: _open,
            builder: (context, _) => stacked
                ? _stack(context, ui,
                    room - 2 * fxKeyStripInset(scope.theme), _open.value)
                : _strip(context, ui, full, _open.value),
          );
        },
      ),
    );
  }

  /// The strip stacked for a narrow panel: the ruler across the panel, and
  /// under it the rows, each animated one carrying its lane beneath it. The
  /// lanes keep their width while the strip opens, and grow in height.
  Widget _stack(
      BuildContext context, LumitUiState ui, double full, double open) {
    final t = ThemeScope.of(context).theme;
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    final fps = ui.model.fps;
    final width = open <= 0 ? 0.0 : full.clamp(0.0, double.infinity);
    final axis = TimelineAxis(frames: widget.spanFrames, width: width);
    final rows = FxKeyStripScope._(
      width: width,
      stacked: true,
      reveal: open,
      playhead: ui.playheadFrame,
      axis: axis,
      spanStart: widget.spanStart,
      keyOffset: widget.keyOffset,
      fps: fps,
      fpsNum: fpsNum,
      fpsDen: fpsDen,
      selected: _selected,
      stretch: _stretch,
      host: this,
      child: widget.child,
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (width > 0)
          SizedBox(
            height: fxKeyStripRulerHeight * open,
            child: ClipRect(
              child: Opacity(
                opacity: open,
                child: OverflowBox(
                  alignment: Alignment.topCenter,
                  maxHeight: fxKeyStripRulerHeight,
                  minHeight: fxKeyStripRulerHeight,
                  child: Padding(
                    padding:
                        EdgeInsets.symmetric(horizontal: fxKeyStripInset(t)),
                    child: Stack(
                      children: [
                        Positioned.fill(
                          child: _FxRuler(
                            axis: axis,
                            spanStart: widget.spanStart,
                            fps: fps,
                            onSeek: (frame) => ui.scrubTo(frame),
                          ),
                        ),
                        Positioned.fill(
                          child: IgnorePointer(
                            child: _FxPlayhead(
                              playhead: ui.playheadFrame,
                              axis: axis,
                              spanStart: widget.spanStart,
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ),
        Expanded(child: rows),
      ],
    );
  }

  Widget _strip(
      BuildContext context, LumitUiState ui, double full, double open) {
    final t = ThemeScope.of(context).theme;
    final width = full * open;
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    final fps = ui.model.fps;
    final axis = TimelineAxis(frames: widget.spanFrames, width: width);
    final inset = fxKeyStripInset(t);
    final rulerHeight = fxKeyStripRulerHeight * open;
    final rows = FxKeyStripScope._(
      width: width,
      stacked: false,
      reveal: open,
      playhead: ui.playheadFrame,
      axis: axis,
      spanStart: widget.spanStart,
      keyOffset: widget.keyOffset,
      fps: fps,
      fpsNum: fpsNum,
      fpsDen: fpsDen,
      selected: _selected,
      stretch: _stretch,
      host: this,
      child: widget.child,
    );
    if (width <= 0) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [Expanded(child: rows)],
      );
    }
    return Stack(
      children: [
        // The lane ground, behind the rows and flush with the panel's edge:
        // a step down from the panel so the column reads as a strip of time
        // rather than as more panel.
        Positioned(
          right: 0,
          top: rulerHeight,
          bottom: 0,
          width: width + inset,
          child: IgnorePointer(
            child: Opacity(
              opacity: open,
              child: DecoratedBox(
                decoration: BoxDecoration(
                  color: t.surface0,
                  border: Border(left: BorderSide(color: t.hairline)),
                ),
              ),
            ),
          ),
        ),
        Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SizedBox(
              height: rulerHeight,
              child: Opacity(
                opacity: open,
                child: Align(
                  alignment: Alignment.centerRight,
                  child: SizedBox(
                    width: width + inset,
                    height: fxKeyStripRulerHeight,
                    child: _FxRuler(
                      axis: axis,
                      spanStart: widget.spanStart,
                      fps: fps,
                      onSeek: (frame) => ui.scrubTo(frame),
                    ),
                  ),
                ),
              ),
            ),
            Expanded(child: rows),
          ],
        ),
        // The playhead, through the ruler and every lane, on its own layer.
        Positioned(
          right: inset,
          top: 0,
          bottom: 0,
          width: width,
          child: IgnorePointer(
            child: Opacity(
              opacity: open,
              child: ClipRect(
                child: _FxPlayhead(
                  playhead: ui.playheadFrame,
                  axis: axis,
                  spanStart: widget.spanStart,
                ),
              ),
            ),
          ),
        ),
        // The divider on the strip's left edge: drag it to set the width.
        Positioned(
          right: inset + width - _seamGrab / 2,
          top: 0,
          bottom: 0,
          width: _seamGrab,
          child: _Seam(
            dragging: _dragging != null,
            onDrag: (dx) => setState(() => _dragging = (_dragging ?? 0) + dx),
            onRelease: () {
              final moved = _dragging;
              setState(() => _dragging = null);
              if (moved == null || moved == 0) return;
              ui.workspace.setFxKeyStripWidth(full);
            },
          ),
        ),
      ],
    );
  }
}

/// How wide the divider's hit target is, centred on the hairline it moves.
const double _seamGrab = 7;

/// The divider between the rows and the strip: the resize cursor, a grip in
/// the ruler band, and an accent line while it is in hand.
class _Seam extends StatelessWidget {
  final bool dragging;
  final ValueChanged<double> onDrag;
  final VoidCallback onRelease;

  const _Seam({
    required this.dragging,
    required this.onDrag,
    required this.onRelease,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return MouseRegion(
      cursor: SystemMouseCursors.resizeColumn,
      child: GestureDetector(
        key: const ValueKey('fx-key-strip-seam'),
        behavior: HitTestBehavior.opaque,
        onHorizontalDragUpdate: (d) => onDrag(d.delta.dx),
        onHorizontalDragEnd: (_) => onRelease(),
        onHorizontalDragCancel: onRelease,
        child: HoverMark(
          cursor: SystemMouseCursors.resizeColumn,
          builder: (context, lit) => Stack(
            children: [
              if (dragging)
                Positioned(
                  left: _seamGrab / 2 - 0.5,
                  top: 0,
                  bottom: 0,
                  width: 1,
                  child: ColoredBox(color: t.accent),
                ),
              // The grip: a short bar in the ruler band, brighter under the
              // pointer, which is what says the hairline below it moves.
              Positioned(
                left: _seamGrab / 2 - 1,
                top: (fxKeyStripRulerHeight - 10) / 2,
                width: 2,
                height: 10,
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    color: Color.lerp(t.hairlineStrong, t.textMuted, lit),
                    borderRadius: BorderRadius.circular(1),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The strip's ruler: sparse labels on the comp's clock over the layer's own
/// span, and a scrub surface.
class _FxRuler extends StatelessWidget {
  final TimelineAxis axis;
  final int spanStart;
  final double fps;
  final ValueChanged<int> onSeek;

  const _FxRuler({
    required this.axis,
    required this.spanStart,
    required this.fps,
    required this.onSeek,
  });

  void _seek(double x) => onSeek(
      spanStart + axis.frameAt(x).clamp(0, axis.frames < 0 ? 0 : axis.frames));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      key: const ValueKey('fx-key-strip-ruler'),
      behavior: HitTestBehavior.opaque,
      onTapDown: (d) => _seek(d.localPosition.dx),
      onHorizontalDragStart: (d) => _seek(d.localPosition.dx),
      onHorizontalDragUpdate: (d) => _seek(d.localPosition.dx),
      child: ClipRect(
        child: CustomPaint(
          painter: _FxRulerPainter(
            axis: axis,
            spanStart: spanStart,
            fps: fps,
            ground: t.surface0,
            hairline: t.hairline,
            tick: t.hairlineStrong,
            label: t.mono.copyWith(fontSize: 9, color: t.textMuted),
          ),
        ),
      ),
    );
  }
}

class _FxRulerPainter extends CustomPainter {
  final TimelineAxis axis;
  final int spanStart;
  final double fps;
  final Color ground;
  final Color hairline;
  final Color tick;
  final TextStyle label;

  const _FxRulerPainter({
    required this.axis,
    required this.spanStart,
    required this.fps,
    required this.ground,
    required this.hairline,
    required this.tick,
    required this.label,
  });

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = ground);
    final edge = Paint()
      ..color = hairline
      ..strokeWidth = 1;
    canvas.drawLine(Offset(0.5, 0), Offset(0.5, size.height), edge);
    canvas.drawLine(Offset(0, size.height - 0.5),
        Offset(size.width, size.height - 0.5), edge);
    if (axis.frames <= 0 || fps <= 0 || axis.perFrame <= 0) return;
    final pxPerSec = axis.perFrame * fps;
    final step = rulerLabelStepSeconds(pixelsPerSecond: pxPerSec);
    final minor = rulerMinorStepSeconds(
        pixelsPerSecond: pxPerSec, labelStep: step, fps: fps);
    final start = spanStart / fps;
    final end = (spanStart + axis.frames) / fps;
    final quiet = Paint()
      ..color = hairline
      ..strokeWidth = 1;
    final strong = Paint()
      ..color = tick
      ..strokeWidth = 1;
    double xOf(double seconds) => axis.xOf(seconds * fps - spanStart);
    // Ticks on the comp's own clock, so a layer starting at 2.3 s is
    // labelled 03s where that second falls on it, not 00s at its edge.
    if (minor < step) {
      for (var s = (start / minor).ceil() * minor; s <= end; s += minor) {
        final x = xOf(s);
        canvas.drawLine(Offset(x, size.height - 4), Offset(x, size.height), quiet);
      }
    }
    for (var s = (start / step).ceil() * step; s <= end; s += step) {
      final x = xOf(s);
      canvas.drawLine(Offset(x, size.height - 7), Offset(x, size.height), strong);
      final text = TextPainter(
        text: TextSpan(text: rulerLabelOf(s), style: label),
        textDirection: TextDirection.ltr,
      )..layout();
      text.paint(canvas, Offset(x + 3, 1));
    }
  }

  @override
  bool shouldRepaint(_FxRulerPainter old) =>
      old.spanStart != spanStart ||
      old.fps != fps ||
      old.ground != ground ||
      old.hairline != hairline ||
      old.tick != tick ||
      old.label != label ||
      old.axis.frames != axis.frames ||
      old.axis.width != axis.width;
}

/// The playhead over the strip, following the frame on its own layer. Beyond
/// the span it stands at the edge it is past, dimmed, with a mark pointing
/// the way it went.
class _FxPlayhead extends StatelessWidget {
  final ValueListenable<int> playhead;
  final TimelineAxis axis;
  final int spanStart;

  const _FxPlayhead({
    required this.playhead,
    required this.axis,
    required this.spanStart,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final half = PlayheadMarker.halfWidthFor(t.shape);
    final head = PlayheadMarker.headHeightFor(t.shape);
    return RepaintBoundary(
      child: ValueListenableBuilder<int>(
        valueListenable: playhead,
        builder: (context, frame, child) {
          final rel = frame - spanStart;
          final beyond = rel < 0 ? -1 : (rel > axis.frames ? 1 : 0);
          final x = axis.xOf(rel.clamp(0, axis.frames));
          return Stack(
            children: [
              Positioned(
                left: x - half,
                top: 0,
                bottom: 0,
                child: Opacity(opacity: beyond == 0 ? 1 : 0.45, child: child),
              ),
              if (beyond != 0)
                Positioned(
                  left: beyond < 0 ? x + half + 1 : x - half - 7,
                  top: (head - 6) / 2,
                  child: CustomPaint(
                    size: const Size(6, 6),
                    painter: _BeyondPainter(colour: t.accent, left: beyond < 0),
                  ),
                ),
            ],
          );
        },
        child: const PlayheadMarker(),
      ),
    );
  }
}

/// A small triangle pointing the way the playhead has gone.
class _BeyondPainter extends CustomPainter {
  final Color colour;
  final bool left;
  const _BeyondPainter({required this.colour, required this.left});

  @override
  void paint(Canvas canvas, Size size) {
    final path = left
        ? (Path()
          ..moveTo(size.width, 0)
          ..lineTo(0, size.height / 2)
          ..lineTo(size.width, size.height)
          ..close())
        : (Path()
          ..moveTo(0, 0)
          ..lineTo(size.width, size.height / 2)
          ..lineTo(0, size.height)
          ..close());
    canvas.drawPath(path, Paint()..color = colour);
  }

  @override
  bool shouldRepaint(_BeyondPainter old) =>
      old.colour != colour || old.left != left;
}

/// One row's lane in the strip: the Timeline's own [KeyLane] over the row's
/// lead channel, and the ground under it. Nothing at all outside the strip.
///
/// A row hands over the same channels and write its stopwatch uses, so a key
/// moved here is committed by the row exactly as a key added there is.
class FxKeyLane extends StatefulWidget {
  final String rowKey;
  final List<BridgeScalar> scalars;
  final ValueChanged<List<BridgeScalar>> onWrite;

  const FxKeyLane({
    super.key,
    required this.rowKey,
    required this.scalars,
    required this.onWrite,
  });

  /// Whether the row has keys to show, which is what gives it a line of its
  /// own while the strip is stacked.
  bool get animated =>
      scalars.isNotEmpty && scalars.first is BridgeScalar_Keyframed;

  @override
  State<FxKeyLane> createState() => _FxKeyLaneState();
}

class _FxKeyLaneState extends State<FxKeyLane> {
  _LaneEntry? _entry;
  _FxKeyStripHostState? _host;

  /// Where the last press landed, for telling a click from a drag.
  Offset? _down;
  int _downButtons = 0;

  void _tell(FxKeyStripScope scope) {
    final old = _entry;
    if (old != null) _host?._unregister(widget.rowKey, old);
    _entry = (scalars: widget.scalars, onWrite: widget.onWrite);
    _host = scope._host;
    _host!._register(widget.rowKey, _entry!);
  }

  @override
  void dispose() {
    final old = _entry;
    if (old != null) _host?._unregister(widget.rowKey, old);
    super.dispose();
  }

  List<BridgeKeyframe> get _keys => switch (widget.scalars.first) {
        BridgeScalar_Keyframed(:final field0) => field0,
        _ => const [],
      };

  /// A click on the lane's ground: `Ctrl` plants a key on an animated row at
  /// that frame, a plain click lets everything go, as the Timeline's ground
  /// does.
  void _tapGround(FxKeyStripScope scope, Offset local) {
    final keyboard = HardwareKeyboard.instance;
    if (!keyboard.isControlPressed && !keyboard.isMetaPressed) {
      scope._host._deselectAll();
      return;
    }
    if (_keys.isEmpty) return;
    final frame = scope.axis.frameAt(local.dx).clamp(0, scope.axis.frames) +
        scope.spanStart -
        scope.keyOffset;
    scope._host._plantKey(widget.rowKey, frame);
  }

  /// The key under [dx], within the grab a lane's own slots give, or null.
  int? _keyAt(FxKeyStripScope scope, double dx) {
    final keys = _keys;
    int? best;
    var span = 6.0 + 1e-9;
    for (var i = 0; i < keys.length; i++) {
      final d =
          (scope.axis.xOf(laneKeyFrame(keys[i], scope.fps) + scope.shift) - dx)
              .abs();
      if (d <= span) {
        span = d;
        best = i;
      }
    }
    return best;
  }

  /// A click on a key picks it. In the Timeline the diamond's own drag
  /// recogniser takes the click, being alone in the arena; here the panel's
  /// list and its ground both hold recognisers, and a drag that never moved
  /// loses to them on release. So the click is read off the pointer itself,
  /// and a press that travelled is left to the drag.
  void _clicked(FxKeyStripScope scope, PointerUpEvent e) {
    final down = _down;
    _down = null;
    if (down == null || _downButtons != kPrimaryButton) return;
    final slop = e.kind == PointerDeviceKind.mouse ? 4.0 : kTouchSlop;
    if ((e.localPosition - down).distance >= slop) return;
    final i = _keyAt(scope, down.dx);
    if (i == null) return;
    final keyboard = HardwareKeyboard.instance;
    scope._host._select(
        '${widget.rowKey}#$i',
        keyboard.isShiftPressed ||
            keyboard.isControlPressed ||
            keyboard.isMetaPressed);
  }

  @override
  Widget build(BuildContext context) {
    final scope = FxKeyStripScope.maybeOf(context);
    if (scope == null || scope.width <= 0) return const SizedBox.shrink();
    _tell(scope);
    final keys = _keys;
    return ValueListenableBuilder<Set<String>>(
      valueListenable: scope.selected,
      builder: (context, selected, _) => Stack(
        children: [
          Positioned.fill(
            child: GestureDetector(
              key: ValueKey<String>('fx-lane-${widget.rowKey}'),
              behavior: HitTestBehavior.opaque,
              onTapUp: (d) => _tapGround(scope, d.localPosition),
            ),
          ),
          if (keys.isNotEmpty)
            Positioned.fill(
              child: Listener(
                onPointerDown: (e) {
                  _down = e.localPosition;
                  _downButtons = e.buttons;
                },
                onPointerUp: (e) => _clicked(scope, e),
                onPointerCancel: (_) => _down = null,
                child: KeyLane(
                rowId: widget.rowKey,
                keys: keys,
                axis: scope.axis,
                fps: scope.fps,
                fpsNum: scope.fpsNum,
                fpsDen: scope.fpsDen,
                // ponytail: whole frames only. The Timeline's lanes also
                // land on markers and the playhead; hand snap targets in
                // here if a strip drag needs them.
                magnet: true,
                barShift: scope.shift,
                snapTargets: const [],
                selectedKeys: selected,
                stretch: scope.stretch,
                onSelectKey: (i, additive) =>
                    scope._host._select('${widget.rowKey}#$i', additive),
                onKeyMenu: (i, at) =>
                    scope._host._keyMenu(context, '${widget.rowKey}#$i', at),
                onMoveKeys: scope._host._moveKeys,
                onChanged: () {},
                ),
              ),
            ),
        ],
      ),
    );
  }
}

/// An effect heading's lane: every key the effect carries, as the small
/// marks a shut layer's row draws in the Timeline. A statement, not a target.
class FxKeySummaryLane extends StatelessWidget {
  final List<BridgeKeyframe> keys;
  const FxKeySummaryLane({super.key, required this.keys});

  @override
  Widget build(BuildContext context) {
    final scope = FxKeyStripScope.maybeOf(context);
    if (scope == null || scope.width <= 0 || keys.isEmpty) {
      return const SizedBox.shrink();
    }
    final t = ThemeScope.of(context).theme;
    return CustomPaint(
      painter: LaneKeysPainter(
        frames: [
          for (final k in keys) laneKeyFrame(k, scope.fps) + scope.shift,
        ],
        selected: const {},
        axis: scope.axis,
        colour: t.animated,
        chosen: t.textPrimary,
        half: laneKeyHalf / 2,
        shapes: [for (final k in keys) keyShapeOf(k)],
      ),
    );
  }
}

/// The header's switch for the strip. Lit while it is shown.
class FxKeyStripToggle extends StatelessWidget {
  const FxKeyStripToggle({super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ws = Provider.of<LumitUiState>(context, listen: false).workspace;
    return ListenableBuilder(
      listenable: ws,
      builder: (context, _) => LumitTooltip(
        message: ws.fxKeyStrip ? l10n.tipHideKeyStrip : l10n.tipShowKeyStrip,
        child: HouseButton(
          key: const ValueKey('fx-key-strip-toggle'),
          frameless: true,
          small: true,
          padding: const EdgeInsets.symmetric(horizontal: 3, vertical: 2),
          onPressed: () => ws.setFxKeyStrip(!ws.fxKeyStrip),
          child: LumitIcon(LumitIcons.addKey,
              size: iconSize,
              colour: ws.fxKeyStrip ? t.textPrimary : t.textMuted),
        ),
      ),
    );
  }
}
