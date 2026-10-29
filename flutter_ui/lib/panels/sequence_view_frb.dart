// The sequence view: a Sequence layer's row, grown tall.
//
// Double-click a Sequence layer — its name in the outline, or its bar in the
// lanes — and the row opens *in place* rather than swapping the Timeline for
// another tab. Cutting is the reason: you cut against the beat you can see, so
// the music, the other layers and the ruler all have to stay on screen while
// you do it. The spec originally put this in a tab of its own; that clause no
// longer stands.
//
// **Six rows, three and three.** The clips get three rows' worth of height —
// enough to take hold of, cut, drag along the row and trim by the edges — and
// the speed envelope gets three below them. Everything under the layer moves
// down by the same six rows, which is what makes the view part of the table
// rather than a thing floating over it.
//
// The envelope is its own widget, in speed_envelope.dart.
//
// Zero bridge calls to draw: every clip and its map ride in on the comp read
// model. The bridge is crossed only when a gesture commits.

import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart' show UuidValue;

import '../l10n/strings.dart';
import '../shell/stretch_dialog_frb.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import 'graph_maths.dart';
import 'speed_envelope.dart';
import 'timeline_extras_frb.dart';
import 'waveform_frb.dart';

/// One row of the view's **own** grid — the rows opening a layer adds, which
/// are the view's to size and are the same at either density.
///
/// The layer's own row is not one of these: that one belongs to the table, and
/// so measures `t.density.laneRow`. Counting it here as well is what
/// used to leave the view a pixel shorter than the room the outline reserved
/// for it under Regular, and the halves went out of step below an open view.
const double sequenceRow = 22;

/// What opening actually adds to the layer's row: the two clip rows under the
/// bar, plus the graph.
///
/// The clips get three rows in all — **including the layer's own bar row**,
/// which is the top of them. Collapsed, that row is exactly the bar it always
/// was; opening adds the two below it and the clips spread across all three,
/// so nothing about the layer's own row changes shape as it opens.
const double sequenceClipExtra = sequenceRow * 2;
const double sequenceEnvelopeStrip = sequenceRow * 3;

/// How short and how tall the graph half may be dragged. The floor is one row
/// — below that a curve has nowhere to be — and the ceiling stops a view
/// swallowing the whole panel by accident.
const double sequenceGraphMin = sequenceRow;
const double sequenceGraphMax = sequenceRow * 16;

/// The sequence shape last copied, held for the session.
///
/// A shape is small text and belongs to no one layer — it is *for* carrying
/// between them — so it sits beside the view rather than inside any instance
/// of it, and survives a row being closed and another opened.
String? sequenceShapeClipboard;

/// How near an edge counts as grabbing it rather than the clip's body.
const double _edgeGrab = 7;

/// A Sequence layer's clips and their speed envelope, under its bar.
class SequenceViewFrb extends StatefulWidget {
  final BridgeLayerEntry entry;

  /// The comp the layer is in, which is who knows what footage a clip plays.
  /// Without it the envelope draws no every-frame guide.
  final CompositionReference? comp;
  final TimelineAxis axis;
  final double fps;
  final int fpsNum;
  final int fpsDen;

  /// Where the lane's viewport starts inside the scrolled content.
  ///
  /// The strip is as wide as the whole composition and lives inside the
  /// Timeline's horizontal scroll, so canvas x 0 is the *start of time*, not
  /// the left of the window — the axis labels were painted there and scrolled
  /// out of sight the moment the Timeline moved, leaving the reference lines
  /// with nothing naming them. The graph editor solved this the same way.
  final ScrollController? hScroll;

  /// How waveforms draw. Passed in rather than read here: the
  /// Timeline already reads the setting for its own lanes, and a clip and a
  /// layer disagreeing about it would be two answers to one question.
  final WaveformStyle style;

  /// Whether the razor is armed, and how to cut this layer at a frame — the
  /// open view stands in for the layer's bar, so it carries the bar's razor.
  final bool razor;
  final void Function(int frame)? onRazor;

  /// Where a cut at screen x lands, in comp frames — the same function the
  /// Timeline's blade line is drawn with, so a cut inside a sequence agrees
  /// with the mark above it (docs/07 §4.5). Null falls back to the axis's own
  /// rounding, which is what it always did.
  final double Function(double x)? razorFrameAt;

  /// Select this layer, and close the view — the bar's other duties, which
  /// the view takes on while it is standing in for it.
  final VoidCallback? onSelect;
  final VoidCallback? onClose;

  /// How tall the graph half is right now, and where to report a drag of its
  /// divider. Only the graph resizes: the clip strip is sized for cutting,
  /// while how much room a speed curve wants depends on how far its ramps go.
  final double graphHeight;
  final ValueChanged<double>? onGraphHeight;

  /// Committed a gesture; the panel refreshes its read model.
  final VoidCallback onChanged;

  /// Show `clip` playing under a map it has not been given yet — the live
  /// drag, which never touches the document.
  final void Function(BridgeClip clip, List<BridgeKeyframe> keys)? onPreview;

  const SequenceViewFrb({
    super.key,
    required this.entry,
    this.comp,
    required this.axis,
    required this.fps,
    required this.fpsNum,
    required this.fpsDen,
    this.hScroll,
    this.style = const WaveformStyle(),
    this.razor = false,
    this.onRazor,
    this.razorFrameAt,
    this.onSelect,
    this.onClose,
    this.graphHeight = sequenceEnvelopeStrip,
    this.onGraphHeight,
    this.onPreview,
    required this.onChanged,
  });

  @override
  State<SequenceViewFrb> createState() => _SequenceViewFrbState();
}

/// The clip picked in an open sequence view, which Delete takes before the
/// layer it sits on. One at a time, across every open view.
final ValueNotifier<({LayerReference layer, UuidValue clip})?>
    sequenceClipPick = ValueNotifier(null);

class _SequenceViewFrbState extends State<SequenceViewFrb> {
  LumitUiState? _ui;
  VoidCallback? _escapeRelease;

  /// Set while this view picks a clip's layer, so the change it makes is not
  /// read as the pick being let go.
  bool _picking = false;

  bool get _pickIsOurs =>
      sequenceClipPick.value?.layer.internallayerId ==
      widget.entry.layer.internallayerId;

  @override
  void initState() {
    super.initState();
    _ui = Provider.of<LumitUiState>(context, listen: false);
    _ui!.selectedLayers.addListener(_onLayersPicked);
    sequenceClipPick.addListener(_onPick);
    _escapeRelease = EscapeLadder.register(EscapeRung.selection, () {
      if (!_pickIsOurs) return false;
      sequenceClipPick.value = null;
      return true;
    });
  }

  /// Anything else picked lets the clip go.
  void _onLayersPicked() {
    if (!_picking && _pickIsOurs) sequenceClipPick.value = null;
  }

  void _onPick() {
    if (mounted) setState(() {});
  }

  /// A click on a clip: its layer is picked, as before, and the clip itself,
  /// so Delete takes the clip and Effect controls shows its own stack.
  void _pickClip(BridgeClip clip) {
    _picking = true;
    widget.onSelect?.call();
    _picking = false;
    sequenceClipPick.value = (layer: widget.entry.layer, clip: clip.id);
    _ui?.cutClip.value = (layer: widget.entry.layer, clip: clip.id);
    // Delete is about the clip now, not an effect row picked before it.
    _ui?.clearEffectSelection();
  }
  /// The clip being dragged, what the gesture is doing to it, and how far it
  /// has travelled — so the drag previews and commits once, on release.
  ({BridgeClip clip, _Grab grab, double dx})? _drag;

  /// The first frame each clip shows, by clip id, decoded off the build.
  /// A null entry is one already asked for — claimed before the decode starts
  /// so a rebuild mid-flight does not ask twice.
  final Map<String, ui.Image?> _thumbs = {};

  /// Each clip's waveform peaks, by clip id — the sound *inside* the
  /// cut, which is what a beat-checked edit is actually aimed at (docs/09 §4).
  ///
  /// Bucketed in the clip's own placed time by the engine, so a ramped clip's
  /// transients land where they are heard, and so sliding the clip along the
  /// row carries the picture with it with nothing refetched.
  final Map<String, BridgeAudioPeaks> _peaks = {};

  /// What each clip's peaks were fetched for — the window, the buckets and the
  /// wave style. Equal keys mean the answer in hand still fits, so a rebuild
  /// asks nothing.
  final Map<String, String> _peakKeys = {};

  @override
  void dispose() {
    _ui?.selectedLayers.removeListener(_onLayersPicked);
    sequenceClipPick.removeListener(_onPick);
    _escapeRelease?.call();
    if (_pickIsOurs) sequenceClipPick.value = null;
    for (final image in _thumbs.values) {
      image?.dispose();
    }
    super.dispose();
  }

  /// Decode a clip's opening frame into the cache. Off the build, because it
  /// opens the media — and once per clip, keyed by the moment it opens on, so
  /// cutting or re-speeding a clip fetches the frame it now starts on.
  void _wantThumb(BridgeClip clip) {
    final key = '${clip.id}@${clip.startFrame}@${clip.retimed}';
    if (_thumbs.containsKey(key)) return;
    _thumbs[key] = null;
    widget.entry.layer
        .clipThumbnail(clip: clip.id, maxEdge: 96, along: null)
        .then((frame) {
      if (!mounted || frame == null || frame.width == 0) return;
      ui.decodeImageFromPixels(
        frame.rgba,
        frame.width,
        frame.height,
        ui.PixelFormat.rgba8888,
        (image) {
          if (!mounted) {
            image.dispose();
            return;
          }
          setState(() => _thumbs[key] = image);
        },
      );
    });
  }

  /// Ask for the waveform of the part of `clip` that is on screen, at one
  /// bucket per pixel column of it — so a clip's wave gains detail as the
  /// Timeline zooms in, rather than stretching the summary it was given first.
  ///
  /// Off the build, and claimed before the fetch starts, exactly as the
  /// thumbnails are: the first ask for a file decodes it.
  void _wantPeaks(BridgeClip clip, double left, double width) {
    final id = clip.id.toString();
    final axis = widget.axis;
    if (axis.perFrame <= 0 || widget.fps <= 0 || width <= 0) return;
    final secondsPerPixel = 1 / (axis.perFrame * widget.fps);
    // The visible slice of this clip, in the clip's own placed clock.
    final scroll = widget.hScroll;
    final viewLeft = scroll != null && scroll.hasClients ? scroll.offset : 0.0;
    final viewWidth = scroll != null && scroll.hasClients
        ? scroll.position.viewportDimension
        : axis.width;
    final from = math.max(left, viewLeft);
    final to = math.min(left + width, viewLeft + viewWidth);
    if (!(to > from)) return;
    // Clip-local placed seconds start at the clip's own place_start, which is
    // the clock `clipAudioPeaks` buckets in.
    final localStart = rationalSeconds(clip.placeStart);
    final request = WaveformRequest.forView(
      startSeconds: localStart + (from - left) * secondsPerPixel,
      endSeconds: localStart + (to - left) * secondsPerPixel,
      pixels: to - from,
    );
    if (request == null) return;
    // The trim and the map are part of the key: both change which source
    // moments the buckets stand for, and neither moves the clip's box.
    final key = '${request.key}|${clip.startFrame}|${clip.endFrame}'
        '|${clip.retimed}|${widget.style.needsBands}';
    if (_peakKeys[id] == key) return;
    _peakKeys[id] = key;
    widget.entry.layer
        .clipAudioPeaks(
      clip: clip.id,
      startSeconds: request.startSeconds,
      endSeconds: request.endSeconds,
      buckets: request.buckets,
      multiwave: widget.style.needsBands,
      stereo: false,
    )
        .then((peaks) {
      if (!mounted || _peakKeys[id] != key) return;
      setState(() => _peaks[id] = peaks);
    });
  }

  /// Double-clicks on the envelope and the clip strip, spotted without a
  /// recogniser in the way of the single click that selects — the same trap
  /// the bar and the graph pane both hit ([DoubleTap]).
  final DoubleTap _envelopeTaps = DoubleTap();
  final DoubleTap _stripTaps = DoubleTap();

  List<BridgeClip> get _clips => widget.entry.info.clips;

  double _xOf(int frame) => widget.axis.xOf(frame);

  /// The frames a drag has travelled, snapped to whole frames — the row edits
  /// in frames, like everything else on the timeline.
  int _draggedFrames(double dx) {
    final perFrame = widget.axis.perFrame;
    return perFrame <= 0 ? 0 : (dx / perFrame).round();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SizedBox(
          // All three rows, the layer's own bar row included: the view stands
          // in for the bar while it is open, so the clips are one region
          // rather than a strip under a bar with a seam between them. The top
          // one is the table's row and takes the density's height; the two
          // below are the view's own.
          height: t.density.laneRow + sequenceClipExtra,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            // The razor cuts here, since there is no bar to aim at while the
            // view is open — the same command on the same layer, aimed at
            // where it was clicked (docs/07 §4.4).
            onTapUp: (d) {
              if (widget.razor) {
                widget.onRazor?.call(
                  widget.razorFrameAt?.call(d.localPosition.dx).round() ??
                      widget.axis.frameAt(d.localPosition.dx),
                );
                return;
              }
              widget.onSelect?.call();
              // Double-clicking the region shuts it again, the same gesture
              // that opened it.
              if (_stripTaps.tap()) widget.onClose?.call();
            },
            child: Stack(
              children: [
                // The region's own ground, so three rows of it read as one
                // block rather than as empty lane.
                Positioned.fill(
                  child: IgnorePointer(
                    child: ColoredBox(color: t.surface1),
                  ),
                ),
                // Each clip's untrimmed reach, behind the clips (§12A.1):
                // what shows past a clip's ends is exactly the material
                // trimmed away, the same outline a trimmed *layer* draws
                // around its bar.
                for (final c in _clips)
                  if (_clipGhost(t, c) case final ghost?) ghost,
                for (final c in _clips) _clip(t, c),
                // Where two clips overlap, the join is a crossfade, and the
                // overlap wears the board's opposed-fades pair, with
                // a handle at either end — dragging one adjusts the fade by
                // trimming the clip whose ramp it is.
                ..._crossfades(t),
                // Where the clips end and the graph begins. Drawn *inside*
                // the clip region rather than as a row of its own: a
                // separator with a height of its own makes the view one pixel
                // taller than the outline reserved for it, and the two halves
                // of the Timeline go out of step over a hairline.
                Positioned(
                  left: 0,
                  right: 0,
                  bottom: 0,
                  child: IgnorePointer(
                    child: Container(height: 1, color: t.hairline),
                  ),
                ),
              ],
            ),
          ),
        ),
        SizedBox(
          height: (widget.graphHeight - _dividerHeight)
              .clamp(sequenceGraphMin, sequenceGraphMax),
          child: SpeedEnvelope(
            entry: widget.entry,
            comp: widget.comp,
            axis: widget.axis,
            hScroll: widget.hScroll,
            fps: widget.fps,
            fpsNum: widget.fpsNum,
            fpsDen: widget.fpsDen,
            onChanged: widget.onChanged,
            onPreview: widget.onPreview,
            onTapped: _envelopeTaps.tap,
          ),
        ),
        // The divider, at the very bottom of the view: drag it to give the
        // graph more or less room. Only here, and only on a Sequence layer —
        // every other row's height is the table's business, not the user's.
        _GraphDivider(
          height: widget.graphHeight,
          onHeight: (h) => widget.onGraphHeight?.call(h),
        ),
      ],
    );
  }

  /// A clip's span as drawn right now — the document's frames plus whatever
  /// a drag in flight has done to them, the same arithmetic [_clip] draws
  /// its box with. The crossfades read this so a fade follows the edge being
  /// dragged rather than jumping on release.
  (int, int) _drawnSpan(BridgeClip clip) {
    final drag = _drag;
    final moving = drag != null && drag.clip.id == clip.id;
    final shift = moving ? _draggedFrames(drag.dx) : 0;
    return (
      clip.startFrame.toInt() + (moving && drag.grab != _Grab.end ? shift : 0),
      clip.endFrame.toInt() + (moving && drag.grab != _Grab.start ? shift : 0),
    );
  }

  /// The joins where clips overlap, each drawn as the opposed-fades pair with
  /// a handle at either end (the AudioWorkspace board).
  ///
  /// The overlap **is** the crossfade — the mixer ramps the outgoing clip out
  /// and the incoming one in across exactly this span — so the drawing is
  /// derived, never stored: sliding or trimming either clip moves the fade
  /// because it moves the overlap. A butt cut has no overlap and gets no
  /// ramps, which is what a hard cut on the beat is for; the fade is *made*
  /// by trimming an end past the join, which the edge grabs already do.
  List<Widget> _crossfades(LumitTheme t) {
    final ordered = [..._clips]
      ..sort((a, b) => a.startFrame.compareTo(b.startFrame));
    final out = <Widget>[];
    for (var i = 0; i + 1 < ordered.length; i++) {
      final outgoing = ordered[i];
      final incoming = ordered[i + 1];
      final (_, outEnd) = _drawnSpan(outgoing);
      final (inStart, _) = _drawnSpan(incoming);
      if (outEnd <= inStart) continue;
      final left = _xOf(inStart);
      final width = (_xOf(outEnd) - left).clamp(1.0, double.infinity);
      out.add(Positioned(
        key: ValueKey<String>('seq-crossfade-${outgoing.id}'),
        left: left,
        width: width,
        top: 2,
        bottom: 2,
        child: IgnorePointer(
          child: CustomPaint(painter: _CrossfadePainter(line: t.accent)),
        ),
      ));
      // The two handles: each end of the fade is the end of one clip's ramp,
      // so each drags that clip's own edge — the same trim the clip's edge
      // grab commits, aimed by a target that stays grabbable when the two
      // edges sit a few pixels apart.
      out.add(_fadeHandle(
        t,
        key: 'seq-fade-in-${incoming.id}',
        clip: incoming,
        grab: _Grab.start,
        x: left,
      ));
      out.add(_fadeHandle(
        t,
        key: 'seq-fade-out-${outgoing.id}',
        clip: outgoing,
        grab: _Grab.end,
        x: left + width,
      ));
    }
    return out;
  }

  Widget _fadeHandle(
    LumitTheme t, {
    required String key,
    required BridgeClip clip,
    required _Grab grab,
    required double x,
  }) =>
      Positioned(
        key: ValueKey<String>(key),
        left: x - 6,
        top: 0,
        width: 12,
        height: 14,
        child: MouseRegion(
          cursor: SystemMouseCursors.resizeLeftRight,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            supportedDevices: dragDevices,
            onHorizontalDragStart: (_) =>
                setState(() => _drag = (clip: clip, grab: grab, dx: 0)),
            onHorizontalDragUpdate: (d) => setState(() {
              final held = _drag;
              if (held != null) {
                _drag =
                    (clip: held.clip, grab: held.grab, dx: held.dx + d.delta.dx);
              }
            }),
            onHorizontalDragEnd: (_) => _commitDrag(),
            onHorizontalDragCancel: () => setState(() => _drag = null),
            child: Center(
              child: Transform.rotate(
                angle: math.pi / 4,
                child: Container(
                  width: 5,
                  height: 5,
                  decoration: BoxDecoration(
                    border: Border.all(color: t.textSecondary),
                    color: t.surface1,
                  ),
                ),
              ),
            ),
          ),
        ),
      );

  /// Where this clip's **whole source** would sit if none of it had been
  /// trimmed away (docs/15 §12A.1) — the clip-level twin of the outline
  /// a trimmed layer bar draws.
  ///
  /// `null` when there is nothing to draw, and the engine decides most of
  /// those: a **retimed** clip has no reach at all, because its map decides
  /// which source moment each of its frames shows; a source whose length could
  /// not be read has none either, rather than one pinned to a guess. The last
  /// case is this side's: a clip already showing all of its source needs no
  /// outline, because there is nothing trimmed away to outline.
  ///
  /// The reach travels with a move, exactly as the layer bar's does: sliding a
  /// clip along the row carries the material it can show with it, and an
  /// outline that stayed behind made a clip at its limit look as though it had
  /// left the limit.
  Widget? _clipGhost(LumitTheme t, BridgeClip clip) {
    final reachStart = clip.reachStartFrame;
    final reachEnd = clip.reachEndFrame;
    if (reachStart == null || reachEnd == null) return null;

    final drag = _drag;
    final moving = drag != null && drag.clip.id == clip.id;
    // Only a whole-clip move carries the source with it. Trimming an edge
    // moves the box over material that stays put, which is the whole point of
    // the outline.
    final shift =
        moving && drag.grab == _Grab.body ? _draggedFrames(drag.dx) : 0;
    final start = reachStart.toInt() + shift;
    final end = reachEnd.toInt() + shift;
    final clipStart = clip.startFrame.toInt() +
        (moving && drag.grab != _Grab.end ? _draggedFrames(drag.dx) : 0);
    final clipEnd = clip.endFrame.toInt() +
        (moving && drag.grab != _Grab.start ? _draggedFrames(drag.dx) : 0);
    if (clipStart <= start && clipEnd >= end) return null;

    final left = _xOf(start);
    return Positioned(
      key: ValueKey<String>('seq-clip-ghost-${clip.id}'),
      left: left,
      width: (_xOf(end) - left).clamp(1.0, double.infinity),
      top: 2,
      bottom: 2,
      child: IgnorePointer(
        child: Container(
          decoration: BoxDecoration(
            // A hairline and nothing inside it, the layer bar's own treatment
            // (§12A.1): the outline says how far this same clip could still be
            // pulled, and a fill would read as a second, dimmer clip sitting
            // behind it.
            border: Border.all(
              color: t
                  .labelColour(widget.entry.info.label)
                  .withValues(alpha: 0.25),
              width: 1,
            ),
            // The clip's own ends, because this *is* the clip drawn as far as
            // its source goes.
            borderRadius: BorderRadius.circular(clipRadius(t)),
          ),
        ),
      ),
    );
  }

  /// One clip: a box where it sits, saying how fast it plays. Its body drags
  /// it along the row and its edges trim it.
  Widget _clip(LumitTheme t, BridgeClip clip) {
    _wantThumb(clip);
    final drag = _drag;
    final moving = drag != null && drag.clip.id == clip.id;
    final shift = moving ? _draggedFrames(drag.dx) : 0;
    final start = clip.startFrame.toInt() +
        (moving && drag.grab != _Grab.end ? shift : 0);
    final end = clip.endFrame.toInt() +
        (moving && drag.grab != _Grab.start ? shift : 0);
    final left = _xOf(start);
    final width = (_xOf(end) - left).clamp(2.0, double.infinity);
    _wantPeaks(clip, left, width);
    // The clip's own placed clock at its left edge. Sliding the whole clip
    // moves box and content together, so this does not change and the wave
    // rides along; dragging the *start* edge moves the box over content that
    // stays put, so the origin travels with it and the wave holds still until
    // the trim commits and the peaks are asked for again.
    final originSeconds = rationalSeconds(clip.placeStart) +
        (moving && drag.grab == _Grab.start ? shift / widget.fps : 0);
    final picked = _pickIsOurs && sequenceClipPick.value?.clip == clip.id;
    final edge = picked ? t.accent : t.surface0;

    return Positioned(
      key: ValueKey<String>('seq-clip-${clip.id}'),
      left: left,
      width: width,
      top: 2,
      bottom: 2,
      child: MouseRegion(
        cursor: SystemMouseCursors.resizeLeftRight,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onSecondaryTapDown: (d) => _clipMenu(clip, d.globalPosition),
          onTapUp: widget.razor ? null : (_) => _pickClip(clip),
          supportedDevices: dragDevices,
          onHorizontalDragStart: (d) => setState(() {
            final where = d.localPosition.dx;
            _drag = (
              clip: clip,
              grab: where < _edgeGrab
                  ? _Grab.start
                  : where > width - _edgeGrab
                      ? _Grab.end
                      : _Grab.body,
              dx: 0,
            );
          }),
          onHorizontalDragUpdate: (d) => setState(() {
            final held = _drag;
            if (held != null) {
              _drag = (
                clip: held.clip,
                grab: held.grab,
                dx: held.dx + d.delta.dx,
              );
            }
          }),
          onHorizontalDragEnd: (_) => _commitDrag(),
          onHorizontalDragCancel: () => setState(() => _drag = null),
          child: Container(
            decoration: BoxDecoration(
              // Desaturated, exactly as a layer bar is (§12A.1): the label's
              // own colour thinned over the ground, with the solid leading
              // edge below carrying it at full strength. Computed from the
              // token, so a recoloured layer recolours its clips too.
              color: t
                  .labelColour(widget.entry.info.label)
                  .withValues(alpha: clipFillAlpha),
              border: Border.all(color: edge, width: picked ? 2 : 1),
              // Stadium ends under Round (§12.1), clamped to half the
              // clip's own height by the control radius sentinel. **The hit
              // rect stays rectangular** — the gesture detector is outside
              // this box and reads localPosition.dx across the full width, so
              // the edge-grab zones are the ones they always were, curve or
              // no curve. Deliberate: the ends are the smallest targets here.
              borderRadius: BorderRadius.circular(clipRadius(t)),
            ),
            // The edge again over the content, which would otherwise cover it where
            // the corner curves.
            foregroundDecoration: BoxDecoration(
              border: Border.all(color: edge, width: picked ? 2 : 1),
              borderRadius: BorderRadius.circular(clipRadius(t)),
            ),
            alignment: Alignment.center,
            clipBehavior: Clip.hardEdge,
            child: Stack(
              children: [
                // The clip's leading edge: where this cut begins, at full
                // strength, so a run of clips reads as a run of beginnings.
                Positioned(
                  key: ValueKey<String>('seq-clip-edge-${clip.id}'),
                  left: 0,
                  top: 0,
                  bottom: 0,
                  width: clipEdgeWidth,
                  child:
                      ColoredBox(color: t.labelColour(widget.entry.info.label)),
                ),
                // The clip's own sound, under its label and thumbnail: a cut
                // is aimed at what you can see, and on a Sequence layer what
                // you are cutting is the clip (docs/09 §4). Drawn behind
                // everything so the speed readout stays legible over it.
                if (_peaks[clip.id.toString()] case final peaks?)
                  Positioned.fill(
                    child: CustomPaint(
                      key: ValueKey<String>('seq-wave-${clip.id}'),
                      painter: WaveformPainter(
                        peaks: peaks,
                        // Canvas x 0 is the clip's own left edge, and its
                        // placed clock starts at `place_start` — so a slid
                        // clip carries its wave with it and nothing refetches.
                        originSeconds: originSeconds,
                        secondsPerPixel:
                            widget.axis.perFrame <= 0 || widget.fps <= 0
                                ? 0.0
                                : 1 / (widget.axis.perFrame * widget.fps),
                        left: 0,
                        right: width,
                        colours: t.waveform,
                        style: widget.style,
                      ),
                    ),
                  ),
                // The frame this clip opens on, so a row of cuts can be told
                // apart at a glance rather than by their timings.
                //
                // **And nothing beside it** (§6.27). The thumbnail used to
                // share its row with the clip's speed, printed as a percentage
                // or as the word "ramp" — a second reading of the very thing
                // the envelope draws directly underneath, in the space a
                // picture of the shot was meant to fill. The envelope is
                // untouched: it is where a clip's speed is read.
                if (_thumbs['${clip.id}@${clip.startFrame}@${clip.retimed}']
                    case final image?)
                  Padding(
                    padding: const EdgeInsets.all(1),
                    child: RawImage(image: image, fit: BoxFit.contain),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// What can be done to one clip. Right-click is where a clip's own
  /// operations live: it is the one gesture that is unambiguously *about this
  /// clip* and cannot be confused with cutting, moving or trimming it.
  Future<void> _clipMenu(BridgeClip clip, Offset at) async {
    final picked = await showMenuAt<String>(
      context: context,
      position: at,
      width: 190,
      rows: (close) => [
        MenuRow(
            onPressed: () => close('copy-clip'),
            child: Text(l10n.clipCopyShape)),
        MenuRow(
            onPressed: () => close('copy-row'),
            child: Text(l10n.clipCopyRowShape)),
        MenuRow(
            onPressed: () => close('paste'), child: Text(l10n.clipPasteShape)),
        // The numeric speed entry (docs/04 §12.1's `retime.set_segment_speed`),
        // beside the reset that was already here. No duration well: a clip's
        // place and length are fixed by the covenant, so its speed
        // decides what it plays rather than how long it runs.
        MenuRow(
            key: ValueKey<String>('seq-clip-speed-${clip.id}'),
            onPressed: () => close('speed'),
            child: Text(l10n.menuClipSpeed)),
        MenuRow(
            onPressed: () => close('reset'), child: Text(l10n.clipResetSpeed)),
        MenuRow(onPressed: () => close('delete'), child: Text(l10n.clipDelete)),
      ],
    );
    if (!mounted || picked == null) return;
    switch (picked) {
      case 'delete':
        // A gap, not a closed row: what follows keeps the beat it was cut to.
        widget.entry.layer.deleteClip(clip: clip.id);
      case 'reset':
        widget.entry.layer
            .setClipSpeed(clip: clip.id, percent: 100, endPercent: 100);
      case 'speed':
        final percent = await showStretchDialogFrb(
          context: context,
          durationFrames: null,
          fps: widget.fps,
        );
        if (percent == null || !mounted) return;
        // Constant, not a ramp: the dialogue asks one number, and a ramp is
        // what the envelope beneath the clips is for.
        widget.entry.layer
            .setClipSpeed(clip: clip.id, percent: percent, endPercent: percent);
      // The shape — where the cuts fall and how each piece is ramped — with
      // no media in it, so pasting it onto a depth pass cuts and ramps that
      // pass to match without touching what it plays.
      case 'copy-clip':
        sequenceShapeClipboard =
            widget.entry.layer.copySequenceShape(clip: clip.id);
      case 'copy-row':
        sequenceShapeClipboard = widget.entry.layer.copySequenceShape();
      case 'paste':
        final shape = sequenceShapeClipboard;
        if (shape != null) {
          widget.entry.layer.pasteSequenceShape(text: shape);
        }
    }
    widget.onChanged();
  }

  /// Write the drag: a body grab slides the clip, an edge grab trims it.
  void _commitDrag() {
    final drag = _drag;
    setState(() => _drag = null);
    if (drag == null) return;
    final shift = _draggedFrames(drag.dx);
    if (shift == 0) return;
    final layer = widget.entry.layer;
    switch (drag.grab) {
      case _Grab.body:
        layer.slideClip(
          clip: drag.clip.id,
          toFrame: drag.clip.startFrame + shift,
          nudgeTicks: 0,
          // A slide on a picture row overwrites what it lands on. Overlapping
          // by a slide is the audio row's own rule, and a dissolve on a
          // picture row is made by its own command.
          overlap: false,
        );
      case _Grab.start:
        layer.trimClip(
          clip: drag.clip.id,
          startFrame: drag.clip.startFrame + shift,
          endFrame: drag.clip.endFrame,
        );
      case _Grab.end:
        layer.trimClip(
          clip: drag.clip.id,
          startFrame: drag.clip.startFrame,
          endFrame: drag.clip.endFrame + shift,
        );
    }
    widget.onChanged();
  }
}

/// What a clip drag has hold of.
enum _Grab { body, start, end }

/// The opposed-fades pair over one overlap: the outgoing clip's ramp falls,
/// the incoming one's rises — the board's own X across the join. Straight
/// lines, as the board draws them; the mixer's ramps underneath are
/// equal-power quarter-sines, and the picture's job is to say *where the
/// fade is*, which the crossing does.
class _CrossfadePainter extends CustomPainter {
  final Color line;

  const _CrossfadePainter({required this.line});

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = line
      ..strokeWidth = 1.2
      ..style = PaintingStyle.stroke;
    canvas.drawLine(Offset(0, 1), Offset(size.width, size.height - 1), paint);
    canvas.drawLine(Offset(0, size.height - 1), Offset(size.width, 1), paint);
  }

  @override
  bool shouldRepaint(_CrossfadePainter old) => old.line != line;
}

/// How much of the view's height the divider itself takes.
const double _dividerHeight = 5;

/// The grab bar along the bottom of a sequence view.
///
/// **Tracks the pointer exactly, and lands on whole rows.** The raw travel is
/// accumulated here and the *height* is snapped, rather than snapping each
/// delta — snapped deltas quietly lose the remainder on every frame, so the
/// divider drifts away from the mouse over a long drag. And it has to land on
/// rows: the lane's seams are ruled on the table's grid, so a view half a row
/// out puts a hairline through the middle of every layer below it.
class _GraphDivider extends StatefulWidget {
  final double height;
  final ValueChanged<double> onHeight;
  const _GraphDivider({required this.height, required this.onHeight});

  @override
  State<_GraphDivider> createState() => _GraphDividerState();
}

class _GraphDividerState extends State<_GraphDivider> {
  /// The height the gesture began at, and everything the pointer has
  /// travelled since — kept raw, so the snap is of the total.
  double? _from;
  double _travelled = 0;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return MouseRegion(
      cursor: SystemMouseCursors.resizeUpDown,
      child: GestureDetector(
        key: const ValueKey('seq-graph-divider'),
        behavior: HitTestBehavior.opaque,
        supportedDevices: dragDevices,
        onVerticalDragStart: (_) {
          _from = widget.height;
          _travelled = 0;
        },
        onVerticalDragUpdate: (d) {
          final from = _from ?? widget.height;
          _travelled += d.delta.dy;
          final wanted =
              (from + _travelled).clamp(sequenceGraphMin, sequenceGraphMax);
          // **A whole number of rows, divider included.** The divider lives
          // inside the graph's height rather than on top of it: adding its
          // five pixels to a row multiple made the layer's whole block five
          // pixels off the table's grid, so every row below an open view sat
          // slightly out of its own seams.
          final rows = (wanted / sequenceRow).round().clamp(1, 16);
          widget.onHeight(rows * sequenceRow);
        },
        onVerticalDragEnd: (_) => _from = null,
        onVerticalDragCancel: () => _from = null,
        child: SizedBox(
          height: _dividerHeight,
          child: Center(
            child: Container(
              height: 1,
              margin: const EdgeInsets.symmetric(horizontal: 40),
              color: t.hairline,
            ),
          ),
        ),
      ),
    );
  }
}
