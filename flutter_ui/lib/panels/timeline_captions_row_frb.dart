// A caption layer's open row in the Timeline: one block per caption under
// the layer's bar. Click picks one and moves the playhead to it, a drag on an
// end retimes it and on the middle moves it, written once on release.
//
// Nothing here asks the engine anything while drawing or dragging: the
// captions come in on a [CaptionTrack], and the bridge is crossed when a
// gesture or a menu row commits.

import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart' show UuidValue;

import '../l10n/strings.dart';
import '../state/caption_selection.dart';
import '../state/caption_tracks.dart';
import '../widgets/controls.dart';
import '../widgets/drag_escape.dart';
import '../widgets/escape_ladder.dart';
import 'timeline_captions_frb.dart';
import 'timeline_extras_frb.dart';
import 'timeline_outline_row_frb.dart' show showLayerRowMenu;
import 'timeline_snap.dart';

/// Comp frames as the engine's time, to a thousandth of a frame. Whole frames
/// come out exact.
BridgeRational _timeOf(double frame, int fpsNum, int fpsDen) {
  if (fpsNum <= 0) {
    return BridgeRational(num: (frame * 1000).round(), den: 1000);
  }
  final whole = frame.roundToDouble();
  if ((frame - whole).abs() < 1e-6) {
    return BridgeRational(num: whole.toInt() * fpsDen, den: fpsNum);
  }
  return BridgeRational(
      num: (frame * fpsDen * 1000).round(), den: fpsNum * 1000);
}

/// How near an edge counts as taking hold of it rather than the middle.
const double _edgeGrab = 6;

enum _Grab { body, start, end }

/// A caption layer's captions under its bar, while it is open.
class CaptionsRowFrb extends StatefulWidget {
  final BridgeLayerEntry entry;
  final CaptionTrack track;
  final TimelineAxis axis;
  final int fpsNum;
  final int fpsDen;

  /// The lanes' sideways scroll, so only the captions in view are drawn.
  final ScrollController? hScroll;

  /// What a drag can land on, the Timeline's own list, and whether the
  /// magnet is on.
  final List<SnapTarget> snapTargets;
  final bool magnet;

  /// The razor is armed, and where its cut at screen x lands.
  final bool razor;
  final double Function(double x)? razorFrameAt;

  final VoidCallback onSelect;
  final VoidCallback onClose;

  /// An edit went in; read the captions again.
  final VoidCallback onChanged;

  const CaptionsRowFrb({
    super.key,
    required this.entry,
    required this.track,
    required this.axis,
    required this.fpsNum,
    required this.fpsDen,
    this.hScroll,
    this.snapTargets = const [],
    this.magnet = true,
    this.razor = false,
    this.razorFrameAt,
    required this.onSelect,
    required this.onClose,
    required this.onChanged,
  });

  @override
  State<CaptionsRowFrb> createState() => _CaptionsRowFrbState();
}

class _CaptionsRowFrbState extends State<CaptionsRowFrb> {
  late final LumitUiState _ui;
  late final CaptionSelection _selection;
  late final Listenable _blocksRepaint;
  final DragEscape _escape = DragEscape();
  final DoubleTap _taps = DoubleTap();
  VoidCallback? _escapeRelease;

  /// Set while this row picks its layer, so that change is not read as
  /// something else being picked.
  bool _picking = false;

  /// The caption a Shift click runs from, by its place in the track.
  int? _anchor;

  /// Each caption's words laid out once, by its index and text.
  final Map<int, (String, TextPainter)> _words = {};
  TextStyle? _wordsStyle;

  /// The drag in flight: which caption, what it holds, where the pointer
  /// went down, how far it has travelled, and where that puts the caption.
  ({
    int at,
    _Grab grab,
    double downX,
    double dx,
    double start,
    double end
  })? _drag;
  SnapTarget? _caught;
  Iterable<SnapTarget> _targets = const [];
  double _downX = 0;
  MouseCursor _cursor = SystemMouseCursors.basic;

  UuidValue get _layerId => widget.entry.layer.internallayerId;
  bool get _locked => widget.entry.info.switches.locked;
  bool get _pickIsOurs => captionRowPick.value?.internallayerId == _layerId;

  @override
  void initState() {
    super.initState();
    _ui = Provider.of<LumitUiState>(context, listen: false);
    _selection = _ui.captionSelection;
    _blocksRepaint = Listenable.merge([
      _selection,
      if (widget.hScroll != null) widget.hScroll!,
    ]);
    _ui.selectedLayers.addListener(_onLayersPicked);
    _escapeRelease = EscapeLadder.register(EscapeRung.selection, () {
      if (!_pickIsOurs) return false;
      _selection.choose(_layerId, const {});
      captionRowPick.value = null;
      return true;
    });
  }

  @override
  void dispose() {
    _ui.selectedLayers.removeListener(_onLayersPicked);
    _escapeRelease?.call();
    _escape.dispose();
    for (final held in _words.values) {
      held.$2.dispose();
    }
    super.dispose();
  }

  /// Anything else picked lets the captions go, for Delete's sake.
  void _onLayersPicked() {
    if (!_picking && _pickIsOurs) captionRowPick.value = null;
  }

  void _pickLayer() {
    _picking = true;
    widget.onSelect();
    _picking = false;
    captionRowPick.value = widget.entry.layer;
  }

  TextPainter _wordsOf(int at, TextStyle style) {
    final cue = widget.track.cues[at];
    if (_wordsStyle != style) {
      for (final held in _words.values) {
        held.$2.dispose();
      }
      _words.clear();
      _wordsStyle = style;
    }
    final held = _words[cue.index];
    if (held != null && held.$1 == cue.text) return held.$2;
    held?.$2.dispose();
    final painter = TextPainter(
      text: TextSpan(text: cue.text, style: style),
      textDirection: TextDirection.ltr,
      maxLines: 2,
    )..layout();
    _words[cue.index] = (cue.text, painter);
    return painter;
  }

  // --- Where the pointer is ------------------------------------------------

  /// The caption under screen x, by its place in the track, allowing a few
  /// pixels round one too thin to aim at.
  int? _cueAtX(double x) {
    final axis = widget.axis;
    final track = widget.track;
    final frame = axis.frameAtExact(x);
    final hit = track.at(frame);
    if (hit != null) return hit;
    if (axis.perFrame <= 0) return null;
    final reach = 3 / axis.perFrame;
    final i = track.firstFrom(frame - reach);
    for (var j = i; j < track.length && track.starts[j] <= frame + reach; j++) {
      if (track.ends[j] >= frame - reach) return j;
    }
    return null;
  }

  _Grab _grabAt(int at, double x) {
    final left = widget.axis.xOf(widget.track.starts[at]);
    final right = widget.axis.xOf(widget.track.ends[at]);
    final edge = math.min(_edgeGrab, (right - left) / 3);
    if (x < left + edge) return _Grab.start;
    if (x > right - edge) return _Grab.end;
    return _Grab.body;
  }

  void _hover(PointerHoverEvent event) {
    final x = event.localPosition.dx;
    final at = _cueAtX(x);
    final cursor = at == null || widget.razor || _locked
        ? SystemMouseCursors.basic
        : _grabAt(at, x) == _Grab.body
            ? SystemMouseCursors.grab
            : SystemMouseCursors.resizeLeftRight;
    if (cursor != _cursor) setState(() => _cursor = cursor);
  }

  // --- Picking -------------------------------------------------------------

  Set<int> get _picked =>
      _selection.layer == _layerId ? _selection.cues : const {};

  void _tap(TapUpDetails details) {
    final x = details.localPosition.dx;
    final at = _cueAtX(x);
    if (widget.razor) {
      if (_locked) return;
      final frame = widget.razorFrameAt?.call(x) ?? widget.axis.frameAtExact(x);
      _edit(() => widget.entry.layer
          .splitCueAt(at: _timeOf(frame, widget.fpsNum, widget.fpsDen)));
      return;
    }
    final twice = _taps.tap(at: details.localPosition, slop: kDoubleTapSlop);
    _pickLayer();
    if (at == null) {
      _selection.choose(_layerId, const {});
      // Double-clicking the room shuts it, the gesture that opened it.
      if (twice) widget.onClose();
      return;
    }
    final cue = widget.track.cues[at];
    if (twice) {
      _ui.captionSelection.requestTyping(_layerId, cue.index);
      return;
    }
    final keys = HardwareKeyboard.instance;
    final anchor = _anchor;
    if (keys.isShiftPressed && anchor != null && anchor < widget.track.length) {
      final (from, to) = anchor < at ? (anchor, at) : (at, anchor);
      _selection.choose(_layerId, {
        ..._picked,
        for (var i = from; i <= to; i++) widget.track.cues[i].index,
      });
      return;
    }
    if (keys.isControlPressed || keys.isMetaPressed) {
      final picked = {..._picked};
      if (!picked.remove(cue.index)) picked.add(cue.index);
      _selection.choose(_layerId, picked);
      _anchor = at;
      return;
    }
    _anchor = at;
    _selection.choose(_layerId, {cue.index});
    // Onto the first frame it shows on.
    _ui.scrubTo((widget.track.starts[at] - 1e-6).ceil());
  }

  // --- Dragging ------------------------------------------------------------

  void _dragStart(DragStartDetails details) {
    final at = _cueAtX(_downX);
    if (at == null || _locked || widget.razor) return;
    final cue = widget.track.cues[at];
    _pickLayer();
    if (!_picked.contains(cue.index)) {
      _selection.choose(_layerId, {cue.index});
      _anchor = at;
    }
    final start = widget.track.starts[at];
    final end = widget.track.ends[at];
    // The Timeline's own targets, less this caption's ends, and the ends of
    // the captions in view.
    final edges = <SnapTarget>[];
    final (from, to) = _inView();
    for (var i = widget.track.firstFrom(from);
        i < widget.track.length && widget.track.starts[i] <= to;
        i++) {
      if (i == at) continue;
      edges
        ..add(SnapTarget(widget.track.starts[i], SnapKind.editPoint))
        ..add(SnapTarget(widget.track.ends[i], SnapKind.editPoint));
    }
    _targets = widget.snapTargets
        .where((s) => s.frame != start && s.frame != end)
        .followedBy(edges);
    setState(() => _drag = (
          at: at,
          grab: _grabAt(at, _downX),
          downX: _downX,
          dx: 0,
          start: start,
          end: end,
        ));
    _escape.begin(_abandon);
  }

  void _dragUpdate(DragUpdateDetails details) {
    final drag = _drag;
    if (drag == null || !_escape.running) return;
    final dx = drag.dx + details.delta.dx;
    final (start, end) = _placed(drag.at, drag.grab, dx);
    setState(() => _drag = (
          at: drag.at,
          grab: drag.grab,
          downX: drag.downX,
          dx: dx,
          start: start,
          end: end,
        ));
  }

  /// Where caption [at] lands with the pointer [dx] along: snapped, and held
  /// between its neighbours, which it never runs into.
  (double, double) _placed(int at, _Grab grab, double dx) {
    final track = widget.track;
    final axis = widget.axis;
    final start = track.starts[at];
    final end = track.ends[at];
    final raw = axis.framesOfPx(dx);
    final magnet = widget.magnet &&
        !snapSuspended(
            controlPressed: HardwareKeyboard.instance.isControlPressed);
    final low = at > 0 ? track.ends[at - 1] : 0.0;
    final high = at + 1 < track.length ? track.starts[at + 1] : double.infinity;
    // A caption is at least one frame long.
    const shortest = 1.0;
    SnapResult land(double frame) => snapFrame(
        frame: frame,
        targets: _targets,
        perFrame: axis.perFrame,
        magnet: magnet);
    switch (grab) {
      case _Grab.start:
        final to = land(start + raw);
        _caught = to.caught;
        return (to.frame.clamp(low, math.max(low, end - shortest)), end);
      case _Grab.end:
        final to = land(end + raw);
        _caught = to.caught;
        return (start, to.frame.clamp(math.min(high, start + shortest), high));
      case _Grab.body:
        var shift = magnet ? raw.roundToDouble() : raw;
        _caught = null;
        var best = double.infinity;
        for (final source in [start, end]) {
          final to = land(source + raw);
          final caught = to.caught;
          if (caught == null) continue;
          final px = ((caught.frame - (source + raw)) * axis.perFrame).abs();
          if (px >= best) continue;
          best = px;
          _caught = caught;
          shift = caught.frame - source;
        }
        final lo = low - start;
        final hi = high - end;
        if (lo <= hi) shift = shift.clamp(lo, hi);
        return (start + shift, end + shift);
    }
  }

  /// Put the caption back where the drag found it.
  void _abandon() {
    if (!mounted) return;
    setState(() {
      _drag = null;
      _caught = null;
    });
  }

  /// One write for the whole drag, so one undo step.
  void _commit() {
    final drag = _drag;
    setState(() {
      _drag = null;
      _caught = null;
    });
    if (drag == null) return;
    final track = widget.track;
    if (drag.start == track.starts[drag.at] &&
        drag.end == track.ends[drag.at]) {
      return;
    }
    final cue = track.cues[drag.at];
    // An end left where it was keeps its own exact time.
    final start = drag.start == track.starts[drag.at]
        ? cue.start!
        : _timeOf(drag.start, widget.fpsNum, widget.fpsDen);
    final end = drag.end == track.ends[drag.at]
        ? cue.end!
        : _timeOf(drag.end, widget.fpsNum, widget.fpsDen);
    _edit(() =>
        widget.entry.layer.setCueSpan(cue: cue.index, start: start, end: end));
  }

  /// An edit on this layer's captions, said on the status line if it fails.
  bool _edit(void Function() call) {
    try {
      call();
    } catch (_) {
      Provider.of<LumitState>(context, listen: false)
          .postNotice(l10n.captionsEditFailed, error: true);
      return false;
    }
    widget.onChanged();
    return true;
  }

  // --- The menu ------------------------------------------------------------

  Future<void> _menu(TapDownDetails details) async {
    final at = _cueAtX(details.localPosition.dx);
    if (at == null) {
      _pickLayer();
      await showLayerRowMenu(context, _layerId, details.globalPosition);
      return;
    }
    final track = widget.track;
    final cue = track.cues[at];
    _pickLayer();
    if (!_picked.contains(cue.index)) {
      _selection.choose(_layerId, {cue.index});
      _anchor = at;
    }
    final playhead = _ui.playheadFrame.value.toDouble();
    final canSplit =
        track.starts[at] < playhead && playhead < track.ends[at] && !_locked;
    final next =
        at + 1 < track.length && track.cues[at + 1].index == cue.index + 1;
    final raised = track.raised(at);
    final picked = await showMenuAt<String>(
      context: context,
      position: details.globalPosition,
      rows: (close) {
        Widget row(String answer, String label, {bool enabled = true}) =>
            MenuRow(
              key: ValueKey<String>('tl-caption-menu-$answer'),
              enabled: enabled && !_locked,
              onPressed: () => close(answer),
              child:
                  MenuLabel(label, enabled: enabled && !_locked, ticked: false),
            );
        return [
          row('split', l10n.captionsSplit, enabled: canSplit),
          row('join', l10n.captionsJoinNext, enabled: next),
          row('raise',
              raised ? l10n.captionsPutDown : l10n.captionsRaise),
          const MenuDivider(),
          row('delete', l10n.delete),
        ];
      },
    );
    if (!mounted || picked == null) return;
    final layer = widget.entry.layer;
    switch (picked) {
      case 'split':
        _edit(() => layer.splitCueAt(
            at: _timeOf(playhead, widget.fpsNum, widget.fpsDen)));
      case 'join':
        _edit(() => layer.mergeCues(first: cue.index, count: 2));
        _selection.choose(_layerId, {cue.index});
      case 'raise':
        final which = _picked.contains(cue.index) ? _picked : {cue.index};
        _edit(() => layer.raiseCues(cues: which.toList(), raised: !raised));
      case 'delete':
        final gone = _picked.isEmpty ? {cue.index} : _picked;
        if (_edit(() => layer.removeCues(cues: gone.toList()))) {
          _selection.choose(_layerId, const {});
        }
    }
  }

  // --- Drawing -------------------------------------------------------------

  /// The stretch of frames in view, from the lanes' scroll.
  (double, double) _inView() {
    final axis = widget.axis;
    final scroll = widget.hScroll;
    final laid = scroll != null &&
        scroll.hasClients &&
        scroll.position.hasContentDimensions;
    final left = laid ? scroll.offset : 0.0;
    final width = laid ? scroll.position.viewportDimension : axis.width;
    return (axis.frameAtExact(left), axis.frameAtExact(left + width));
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final drag = _drag;
    final label = t.labelColour(widget.entry.info.label);
    return MouseRegion(
      cursor: drag == null ? _cursor : SystemMouseCursors.grabbing,
      onHover: _hover,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        supportedDevices: dragDevices,
        onTapUp: _tap,
        onSecondaryTapDown: _menu,
        onHorizontalDragDown: (d) => _downX = d.localPosition.dx,
        onHorizontalDragStart: _dragStart,
        onHorizontalDragUpdate: _dragUpdate,
        onHorizontalDragEnd: (_) {
          if (_escape.end()) {
            _commit();
          } else {
            _abandon();
          }
        },
        onHorizontalDragCancel: () {
          _escape.end();
          _abandon();
        },
        child: Stack(
          children: [
            Positioned.fill(
              child: RepaintBoundary(
                child: CustomPaint(
                  key: ValueKey<String>(
                      'tl-captions-${widget.entry.layer.internallayerId}'),
                  painter: _BlocksPainter(
                    track: widget.track,
                    axis: widget.axis,
                    inView: _inView,
                    picked: () => _picked,
                    drag: drag == null
                        ? null
                        : (at: drag.at, start: drag.start, end: drag.end),
                    words: _wordsOf,
                    ground: t.surface1,
                    fill: label.withValues(alpha: clipFillAlpha),
                    edge: label,
                    // A picked caption is lit the way a picked bar is.
                    pickedFill: Color.lerp(label, t.textPrimary, 0.35)!
                        .withValues(alpha: clipFillSelectedAlpha),
                    seam: t.surface0,
                    text: t.small.copyWith(color: t.textPrimary, height: 1.2),
                    radius: clipRadius(t),
                    repaint: _blocksRepaint,
                  ),
                ),
              ),
            ),
            // The caption being said, marked as the playhead runs: its own
            // painter, so playback repaints a line and not every block.
            Positioned.fill(
              child: IgnorePointer(
                child: CustomPaint(
                  painter: _NowPainter(
                    track: widget.track,
                    axis: widget.axis,
                    playhead: _ui.playheadFrame,
                    drag: drag == null
                        ? null
                        : (at: drag.at, start: drag.start, end: drag.end),
                    accent: t.accent,
                    radius: clipRadius(t),
                  ),
                ),
              ),
            ),
            if (_caught case final caught?)
              Positioned(
                left: widget.axis.xOf(caught.frame) - 0.5,
                top: 0,
                bottom: 0,
                width: 1,
                child: IgnorePointer(child: ColoredBox(color: t.accent)),
              ),
            if (drag != null) _hint(drag),
          ],
        ),
      ),
    );
  }

  /// The pill beside the pointer while a caption is moved or retimed: where
  /// it starts and ends now, and how long it is, as a bar's drag says it.
  Widget _hint(
      ({
        int at,
        _Grab grab,
        double downX,
        double dx,
        double start,
        double end
      }) drag) {
    final from = drag.start.round();
    final to = drag.end.round();
    final text = l10n.timelineBarDragHint(from, to, to - from);
    final x = drag.downX + drag.dx;
    final pill = text.length * 5.0 + 8;
    return Positioned(
      key: const ValueKey<String>('tl-caption-drag-hint'),
      left: x + 8 + pill > widget.axis.width ? x - 8 - pill : x + 8,
      top: 1,
      child: HintPill(text: text),
    );
  }
}

typedef _Dragged = ({int at, double start, double end});

/// The captions in view, each a block with its words inside.
class _BlocksPainter extends CustomPainter {
  _BlocksPainter({
    required this.track,
    required this.axis,
    required this.inView,
    required this.picked,
    required this.drag,
    required this.words,
    required this.ground,
    required this.fill,
    required this.edge,
    required this.pickedFill,
    required this.seam,
    required this.text,
    required this.radius,
    super.repaint,
  });

  final CaptionTrack track;
  final TimelineAxis axis;
  final (double, double) Function() inView;
  final Set<int> Function() picked;
  final _Dragged? drag;
  final TextPainter Function(int at, TextStyle style) words;
  final Color ground;
  final Color fill;
  final Color edge;
  final Color pickedFill;
  final Color seam;
  final TextStyle text;
  final double radius;

  /// Narrower than this, a block has no room for words.
  static const double _wordsFrom = 18;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = ground);
    final (from, to) = inView();
    final chosen = picked();
    const top = 2.0;
    final bottom = size.height - 2;
    final body = Paint();
    final lead = Paint()..color = edge;
    final line = Paint()..color = seam;
    // How far right the blocks drawn so far reach. Zoomed out, captions
    // thinner than a pixel are drawn as one run rather than piled up.
    var reach = double.negativeInfinity;
    var before = double.negativeInfinity;
    final held = drag;
    for (var i = track.firstFrom(from);
        i < track.length && track.starts[i] <= to;
        i++) {
      final moving = held != null && held.at == i;
      final start = moving ? held.start : track.starts[i];
      final end = moving ? held.end : track.ends[i];
      final left = axis.xOf(start);
      final right = axis.xOf(end);
      body.color = chosen.contains(track.cues[i].index) ? pickedFill : fill;
      if (right - left < 3 && !moving) {
        final l = math.max(left, reach);
        final r = math.max(right, left + 1);
        if (r > l) canvas.drawRect(Rect.fromLTRB(l, top, r, bottom), body);
        reach = math.max(reach, r);
        before = left;
        continue;
      }
      final box = Rect.fromLTRB(left, top, right, bottom);
      final shape = RRect.fromRectAndRadius(box, Radius.circular(radius));
      canvas.drawRRect(shape, body);
      reach = math.max(reach, right);
      // Where one caption runs straight into the next, a seam of the ground
      // keeps them two.
      if (!moving &&
          i > 0 &&
          (track.starts[i] - track.ends[i - 1]).abs() < 1e-6 &&
          left - before >= captionSeamRoom) {
        canvas.drawRect(Rect.fromLTWH(left, top, 1, box.height), line);
      }
      before = left;
      canvas.save();
      canvas.clipRRect(shape);
      canvas.drawRect(
          Rect.fromLTWH(left, top, clipEdgeWidth, box.height), lead);
      // Raised to the top of the picture: its top edge at full strength.
      if (track.raised(i)) {
        canvas.drawRect(Rect.fromLTWH(left, top, box.width, 2), lead);
      }
      if (box.width >= _wordsFrom) {
        // Clear of the far end, so the words never run into the next one.
        canvas.clipRect(Rect.fromLTRB(left, top, right - 3, bottom));
        words(i, text).paint(canvas, Offset(left + clipEdgeWidth + 4, top + 3));
      }
      canvas.restore();
    }
  }

  @override
  bool shouldRepaint(_BlocksPainter old) =>
      old.track != track ||
      old.drag != drag ||
      old.axis.perFrame != axis.perFrame ||
      old.axis.inset != axis.inset ||
      old.fill != fill ||
      old.ground != ground ||
      old.text != text;

  @override
  bool? hitTest(Offset position) => false;
}

/// The caption under the playhead, ringed in the accent.
class _NowPainter extends CustomPainter {
  _NowPainter({
    required this.track,
    required this.axis,
    required this.playhead,
    required this.drag,
    required this.accent,
    required this.radius,
  }) : super(repaint: playhead);

  final CaptionTrack track;
  final TimelineAxis axis;
  final ValueListenable<int> playhead;
  final _Dragged? drag;
  final Color accent;
  final double radius;

  @override
  void paint(Canvas canvas, Size size) {
    final i = track.at(playhead.value.toDouble());
    if (i == null) return;
    final held = drag;
    final moving = held != null && held.at == i;
    final left = axis.xOf(moving ? held.start : track.starts[i]);
    final right =
        math.max(left + 1, axis.xOf(moving ? held.end : track.ends[i]));
    final box = Rect.fromLTRB(left, 2, right, size.height - 2);
    final paint = Paint()..color = accent;
    canvas.drawRect(
        Rect.fromLTWH(left, box.top, clipEdgeWidth, box.height), paint);
    if (box.width < 4) return;
    canvas.drawRRect(
        RRect.fromRectAndRadius(box.deflate(1), Radius.circular(radius)),
        paint
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2);
  }

  @override
  bool shouldRepaint(_NowPainter old) =>
      old.track != track ||
      old.drag != drag ||
      old.accent != accent ||
      old.axis.perFrame != axis.perFrame;

  @override
  bool? hitTest(Offset position) => false;
}
