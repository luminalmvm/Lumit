// The sequence view: a Sequence layer's row, grown tall (K-248).
//
// Double-click a Sequence layer — its name in the outline, or its bar in the
// lanes — and the row opens *in place* rather than swapping the Timeline for
// another tab. Cutting is the reason: you cut against the beat you can see, so
// the music, the other layers and the ruler all have to stay on screen while
// you do it. K-071 originally put this in a tab of its own; K-248 supersedes
// that clause.
//
// **Six rows, three and three.** The clips get three rows' worth of height —
// enough to take hold of, cut, drag along the row and trim by the edges — and
// the speed envelope gets three below them. Everything under the layer moves
// down by the same six rows, which is what makes the view part of the table
// rather than a thing floating over it.
//
// The envelope is the same editor as the graph's Vegas lens, over the same
// keyframes (K-247, K-249): a point per key, its height the playback speed in
// per cent, straight lines between. `Ctrl`-click or double-click the line
// plants a point; `Alt`-click lifts one. A clip that has never been retimed
// draws the flat 100% it is playing at, and the first edit gives it a real map.
//
// Zero bridge calls to draw: every clip and its map ride in on the comp read
// model (K-184). The bridge is crossed only when a gesture commits.

import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'graph_maths.dart';
import 'layer_fold_frb.dart';
import 'timeline_extras_frb.dart';
import 'waveform_frb.dart';

/// One timeline row's height — the unit the whole view is measured in, so it
/// lands on the table's own grid.
const double sequenceRow = 22;

/// The clips get three rows — **including the layer's own bar row**, which is
/// the top of them. Collapsed, that row is exactly the bar it always was;
/// opening adds the two below it and the clips spread across all three, so
/// nothing about the layer's own row changes shape as it opens (K-248).
const double sequenceClipStrip = sequenceRow * 3;

/// What opening actually adds to the row: the two clip rows under the bar,
/// plus the graph.
const double sequenceClipExtra = sequenceClipStrip - sequenceRow;
const double sequenceEnvelopeStrip = sequenceRow * 3;
const double sequenceViewHeight = sequenceClipStrip + sequenceEnvelopeStrip;

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

  /// How waveforms draw (K-280, K-285). Passed in rather than read here: the
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
  /// drag, which never touches the document (K-247).
  final void Function(BridgeClip clip, List<BridgeKeyframe> keys)? onPreview;

  const SequenceViewFrb({
    super.key,
    required this.entry,
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

class _SequenceViewFrbState extends State<SequenceViewFrb> {
  /// The clip being dragged, what the gesture is doing to it, and how far it
  /// has travelled — so the drag previews and commits once, on release.
  ({BridgeClip clip, _Grab grab, double dx})? _drag;

  /// The first frame each clip shows, by clip id, decoded off the build.
  /// A null entry is one already asked for — claimed before the decode starts
  /// so a rebuild mid-flight does not ask twice.
  final Map<String, ui.Image?> _thumbs = {};

  /// Each clip's waveform peaks, by clip id (K-280) — the sound *inside* the
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
    widget.entry.layer.clipThumbnail(clip: clip.id, maxEdge: 96).then((frame) {
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
  /// Timeline zooms in, rather than stretching the summary it was given first
  /// (K-280).
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
          // rather than a strip under a bar with a seam between them.
          height: sequenceClipStrip,
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
                for (final c in _clips) _clip(t, c),
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
          child: _EnvelopeStrip(
            entry: widget.entry,
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
    final speed = clip.speedPercent;
    _wantPeaks(clip, left, width);
    // The clip's own placed clock at its left edge. Sliding the whole clip
    // moves box and content together, so this does not change and the wave
    // rides along; dragging the *start* edge moves the box over content that
    // stays put, so the origin travels with it and the wave holds still until
    // the trim commits and the peaks are asked for again.
    final originSeconds = rationalSeconds(clip.placeStart) +
        (moving && drag.grab == _Grab.start ? shift / widget.fps : 0);

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
              color: t.labelColour(widget.entry.info.label),
              border: Border.all(color: t.surface0, width: 1),
              // Stadium ends under Round (K-394, §12.1), clamped to half the
              // clip's own height by the control radius sentinel. **The hit
              // rect stays rectangular** — the gesture detector is outside
              // this box and reads localPosition.dx across the full width, so
              // the edge-grab zones are the ones they always were, curve or
              // no curve. Deliberate: the ends are the smallest targets here.
              borderRadius: BorderRadius.circular(
                  t.shape == ThemeShape.round ? t.tokens.controlRadius : 2),
            ),
            alignment: Alignment.center,
            clipBehavior: Clip.hardEdge,
            child: Stack(
              children: [
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
                Row(
                  children: [
                    // The frame this clip opens on, so a row of cuts can be told
                    // apart at a glance rather than by their timings (K-248).
                    if (_thumbs['${clip.id}@${clip.startFrame}@${clip.retimed}']
                        case final image?)
                      Padding(
                        padding: const EdgeInsets.all(1),
                        child: RawImage(image: image, fit: BoxFit.contain),
                      ),
                    Expanded(
                      child: Center(
                        child: ClipRect(
                          child: Text(
                            // A ramp has no single number to show, and printing one would
                            // be a lie about a curve — the envelope below reads it.
                            speed == null ? l10n.clipRamp : '${speed.round()}%',
                            style: t.small.copyWith(color: t.textPrimary),
                            overflow: TextOverflow.clip,
                            softWrap: false,
                          ),
                        ),
                      ),
                    ),
                  ],
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
      // The shape — where the cuts fall and how each piece is ramped — with
      // no media in it, so pasting it onto a depth pass cuts and ramps that
      // pass to match without touching what it plays (K-248).
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

/// The speed envelope: every clip's map drawn as points and straight lines,
/// against an axis that grows to hold whatever the curves reach.
class _EnvelopeStrip extends StatefulWidget {
  final BridgeLayerEntry entry;
  final TimelineAxis axis;
  final ScrollController? hScroll;
  final double fps;
  final int fpsNum;
  final int fpsDen;
  final VoidCallback onChanged;
  final void Function(BridgeClip clip, List<BridgeKeyframe> keys)? onPreview;

  /// Reports a click and answers whether it was the second of a double.
  final bool Function() onTapped;

  const _EnvelopeStrip({
    required this.entry,
    required this.axis,
    this.hScroll,
    required this.fps,
    required this.fpsNum,
    required this.fpsDen,
    required this.onChanged,
    this.onPreview,
    required this.onTapped,
  });

  @override
  State<_EnvelopeStrip> createState() => _EnvelopeStripState();
}

class _EnvelopeStripState extends State<_EnvelopeStrip> {
  /// The point under the pointer while a drag runs: which clip, which key,
  /// the speed it is being asked for, and how far it has been carried in
  /// time. A point moves both ways — *when* a ramp reaches a speed matters as
  /// much as what the speed is.
  ({BridgeClip clip, int index, double speed, double dx})? _drag;

  /// The speed the grabbed point had when the gesture began, and how far the
  /// pointer has travelled since.
  ///
  /// **The drag is relative, not absolute.** Reading the speed straight off
  /// the pointer's height teleports the point to the cursor the instant it is
  /// grabbed — the whole clip's width is the grab target, so the pointer is
  /// rarely at the point's own height — and then the change over a gesture is
  /// not proportional to the travel at all. Travel in, travel out.
  double _grabbedAt = 0;
  double _travelled = 0;

  /// Where the press landed, and whether it has moved since — a press that
  /// never moves is a click, and the two are told apart here because raw
  /// pointer events do not do it for you.
  Offset? _downAt;
  bool _moving = false;

  /// The selection box in flight, and the points it has caught, as
  /// `clipId#index`.
  ///
  /// A press begins a box unless it lands **on a line** — within reach of the
  /// curve at that x, which is a generous target and the one gesture that
  /// obviously means "take hold of this". Everywhere else in the strip is
  /// empty space, so it is free to select across.
  Rect? _box;
  final Set<String> _selected = {};

  /// How near a clip's own line counts as grabbing it rather than starting a
  /// selection box.
  static const double _lineGrab = 10;

  List<BridgeClip> get _clips => widget.entry.info.clips;

  /// A clip's envelope keys — the map it actually plays by, which the engine
  /// hands over whether or not the clip has one of its own.
  ///
  /// Nothing is constructed here on purpose. This used to fabricate the flat
  /// line for an un-retimed clip and started it at source **zero**, which is
  /// true only of a clip nobody has cut: every clip after a cut begins part
  /// way into its media, so ramping one sent it back to the top of the file
  /// and it played the same frame or two throughout.
  List<BridgeKeyframe> _keysOf(BridgeClip clip) => keysOf(clip.retime);

  /// The axis as it was when the drag began, held for the whole gesture.
  ///
  /// **Without this the drag runs away.** The range grows to hold whatever a
  /// point reaches, so a point dragged past the floor widened the axis, which
  /// stretched what the next pixel of travel was worth, which pushed the point
  /// further still — the value ran off exponentially for a steady hand. A
  /// gesture has to keep the scale it started on; the axis reframes when the
  /// pointer is let go.
  (double, double)? _frozen;

  /// The range the strip draws over: the documented default, grown to hold
  /// every point on every clip (K-247).
  (double, double) get _range {
    final frozen = _frozen;
    if (frozen != null) return frozen;
    final (lo, hi) = fitEnvelopeRange([for (final c in _clips) _keysOf(c)]);
    // A little air past whatever the curves reach, so a point at the extreme
    // is not drawn half outside its own row. The *default* range carries its
    // own headroom (K-250), so an ordinary clip needs none added.
    const air = 8.0;
    final (dlo, dhi) = envelopeDefaultRange;
    return (lo < dlo ? lo - air : dlo, hi > dhi ? hi + air : dhi);
  }

  double _y(double speed, double height) {
    final (lo, hi) = _range;
    final span = (hi - lo).abs() < 1e-9 ? 1.0 : hi - lo;
    return height - (speed - lo) / span * height;
  }

  /// Where a clip's key sits on the comp's own clock, in x pixels.
  double _xOfKey(BridgeClip clip, BridgeKeyframe key) =>
      widget.axis.xOf(clip.startFrame.toInt() +
          (rationalSeconds(key.time) * widget.fps).round());

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LayoutBuilder(
      builder: (context, box) {
        final height = box.maxHeight;
        // **Every child keyed.** Flutter matches a Stack's children by
        // position when they are not, so the readout appearing mid-drag took
        // the slot before the gesture detector and rebuilt its element from
        // scratch — taking with it the recogniser holding the drag, which
        // ended the gesture the instant the readout showed up. The same trap
        // K-212 records against the trimmed-layer ghost.
        return Stack(
          children: [
            Positioned.fill(
              key: const ValueKey('seq-envelope-curves'),
              child: IgnorePointer(
                // Clipped to the strip's own bounds. The axis is frozen for
                // the length of a gesture, so a point dragged past the top or
                // the bottom would otherwise be drawn outside this row and
                // over the layers below it — the picture reframes when the
                // pointer is let go and the freeze lifts.
                child: ClipRect(
                  child: CustomPaint(
                    painter: _EnvelopePainter(
                      lanes: [
                        for (final c in _clips)
                          (
                            clip: c,
                            keys: _shown(c),
                          ),
                      ],
                      xOfKey: _xOfKey,
                      y: (s) => _y(s, height),
                      chosen: t.accent,
                      selected: _selected,
                      range: _range,
                      line: t.hairline,
                      curve: t.curve.first,
                      label: t.small.copyWith(color: t.textMuted),
                      viewportLeft: (widget.hScroll?.hasClients ?? false)
                          ? widget.hScroll!.offset
                          : 0,
                    ),
                  ),
                ),
              ),
            ),
            // What the drag is setting, beside the point it has hold of — a
            // speed is a number you are aiming at, and reading it off the
            // height of a dot against an axis that reframes as you drag is
            // not aiming.
            if (_drag case final held?) _readout(t, held, height),
            if (_box case final box?)
              Positioned(
                key: const ValueKey('seq-envelope-box'),
                left: box.left,
                top: box.top,
                width: box.width,
                height: box.height,
                child: IgnorePointer(
                  child: DecoratedBox(
                    decoration: BoxDecoration(
                      color: t.accent.withValues(alpha: 0.12),
                      border: Border.all(color: t.accent),
                    ),
                  ),
                ),
              ),
            // Last, so it is on top, and keyed, so it keeps its element
            // however the children above it come and go.
            // The line itself: click to plant a point, drag one to re-speed.
            Positioned.fill(
              key: const ValueKey('seq-envelope-gestures'),
              // **Raw pointer events, not a gesture.** A point moves in both
              // directions, and a pan recogniser that wants both axes is in
              // the arena against the lane's vertical scroll *and* its
              // horizontal one — which win, so the drag died the moment it
              // had travelled far enough for them to claim it and had to be
              // started again. A `Listener` is not in the arena at all, so
              // the first drag is the one that works.
              child: Listener(
                key: const ValueKey('seq-envelope'),
                behavior: HitTestBehavior.opaque,
                onPointerDown: (e) {
                  _downAt = e.localPosition;
                  if (_onALine(e.localPosition, height)) {
                    _startDrag(e.localPosition, height);
                  } else {
                    setState(() => _box =
                        Rect.fromPoints(e.localPosition, e.localPosition));
                  }
                },
                onPointerMove: (e) => setState(() {
                  final from = _downAt;
                  if (_box != null && from != null) {
                    _moving = true;
                    _box = Rect.fromPoints(from, e.localPosition);
                    return;
                  }
                  final held = _drag;
                  if (held == null) return;
                  _moving = true;
                  _travelled += e.delta.dy;
                  final (lo, hi) = _range;
                  final span = (hi - lo).abs() < 1e-9 ? 1.0 : hi - lo;
                  _drag = (
                    clip: held.clip,
                    index: held.index,
                    // Down is slower: the axis runs fast at the top.
                    speed: _grabbedAt -
                        _travelled / (height <= 0 ? 1 : height) * span,
                    dx: held.dx + e.delta.dx,
                  );
                  // The picture follows the point. A retime decides *which*
                  // frame is decoded, so this is the one drag where nothing
                  // moves on screen until it is asked to.
                  widget.onPreview?.call(held.clip, _shown(held.clip));
                }),
                onPointerUp: (e) {
                  setState(() => _frozen = null);
                  final box = _box;
                  if (box != null) {
                    setState(() {
                      _box = null;
                      if (_moving) _catch(box, height);
                    });
                  } else if (!_moving) {
                    // A press that never moved is a click: plant a point, lift
                    // one, or nothing at all.
                    setState(() => _drag = null);
                    _tap(_downAt ?? e.localPosition, height);
                  } else {
                    _commit();
                  }
                  _moving = false;
                  _downAt = null;
                },
                onPointerCancel: (_) => setState(() {
                  _drag = null;
                  _moving = false;
                  _frozen = null;
                  _box = null;
                }),
              ),
            ),
          ],
        );
      },
    );
  }

  /// The speed the drag is asking for, floating beside the point.
  Widget _readout(
      LumitTheme t,
      ({BridgeClip clip, int index, double speed, double dx}) held,
      double height) {
    final keys = _shown(held.clip);
    final index = held.index.clamp(0, keys.length - 1);
    final x = _xOfKey(held.clip, keys[index]);
    final y = _y(envelopeSpeeds(keys)[index], height);
    return Positioned(
      key: const ValueKey('seq-envelope-readout'),
      left: x + 8,
      top: (y - 9).clamp(0.0, height - 18),
      child: IgnorePointer(
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
          decoration: BoxDecoration(
            color: t.surface3,
            border: Border.all(color: t.hairline),
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          ),
          child: Text('${held.speed.round()}%',
              style: t.small.copyWith(color: t.textPrimary)),
        ),
      ),
    );
  }

  /// A clip's keys with the drag in flight applied, so the line follows the
  /// pointer rather than jumping on release.
  List<BridgeKeyframe> _shown(BridgeClip clip) {
    final keys = _keysOf(clip);
    final held = _drag;
    if (held == null) return keys;
    if (held.clip.id != clip.id) {
      // Another clip the selection reaches into: its caught points move too.
      return _selected.any((s) => s.startsWith('${clip.id}#'))
          ? _withSpeed(clip, keys, held.index, held.speed)
          : keys;
    }
    return _moved(clip, _withSpeed(clip, keys, held.index, held.speed),
        held.index, held.dx);
  }

  /// [keys] with the dragged one carried along in time, snapped to whole
  /// frames and held strictly between its neighbours.
  ///
  /// The two ends stay put: they are the clip's own edges, and a clip's
  /// length is trimmed on the clip, never on its speed curve.
  List<BridgeKeyframe> _moved(
      BridgeClip clip, List<BridgeKeyframe> keys, int index, double dx) {
    if (index <= 0 || index >= keys.length - 1) return keys;
    final perFrame = widget.axis.perFrame;
    if (perFrame <= 0) return keys;
    final frames = (dx / perFrame).round();
    if (frames == 0) return keys;
    final fps = widget.fps <= 0 ? 1.0 : widget.fps;
    final wanted = rationalSeconds(keys[index].time) + frames / fps;
    // One frame of daylight either side, so two keys can never share a time.
    final lo = rationalSeconds(keys[index - 1].time) + 1 / fps;
    final hi = rationalSeconds(keys[index + 1].time) - 1 / fps;
    if (lo >= hi) return keys;
    final at = wanted.clamp(lo, hi);
    // Through `moveEnvelopePoint`, which re-integrates: keeping the stored
    // tangents while the span's length changes is what bent a straight line.
    return moveEnvelopePoint(
        keys, index, timeOfSubframe(at * fps, widget.fpsNum, widget.fpsDen));
  }

  /// [keys] with the drag applied — one point, or the whole line.
  ///
  /// **A clip nobody has retimed moves as one level.** Its envelope is the two
  /// implied ends of a flat 100%, and dragging one of those alone would tilt
  /// the line into a ramp nobody asked for: the obvious reading of dragging a
  /// flat line is "this clip plays at that speed", which is also what Vegas's
  /// first envelope point does. Plant a point and the line has a shape worth
  /// keeping, so from then on a drag moves only the point it has hold of.
  List<BridgeKeyframe> _withSpeed(
      BridgeClip clip, List<BridgeKeyframe> keys, int index, double speed) {
    if (!clip.retimed) {
      return envelopeToKeys(keys, [for (final _ in keys) speed]);
    }
    // A caught set moves together, by the same amount the dragged point
    // moved. Its own points keep whatever spread they had — a selection is
    // for shifting a shape, not for flattening it.
    final held = _drag;
    if (held != null && _selected.contains('${held.clip.id}#${held.index}')) {
      final by = speed - _grabbedAt;
      final speeds = envelopeSpeeds(keys);
      var touched = false;
      for (var i = 0; i < speeds.length; i++) {
        if (_selected.contains('${clip.id}#$i')) {
          speeds[i] += by;
          touched = true;
        }
      }
      if (touched) return envelopeToKeys(keys, speeds);
    }
    return setEnvelopeSpeed(keys, index, speed);
  }

  /// Whether [local] is near enough to a clip's own line to mean "take hold
  /// of this" rather than "start a box here".
  ///
  /// Measured against the line at that x, not against the nearest point: a
  /// clip's line is what the eye follows, and asking for a direct hit on a
  /// 7px dot would make re-speeding a test of aim.
  bool _onALine(Offset local, double height) {
    final clip = _clipAt(local.dx);
    if (clip == null) return false;
    final keys = _keysOf(clip);
    final speeds = envelopeSpeeds(keys);
    for (var i = 0; i + 1 < keys.length; i++) {
      final x0 = _xOfKey(clip, keys[i]);
      final x1 = _xOfKey(clip, keys[i + 1]);
      if (local.dx < x0 - _lineGrab || local.dx > x1 + _lineGrab) continue;
      final f = x1 > x0 ? ((local.dx - x0) / (x1 - x0)).clamp(0.0, 1.0) : 0.0;
      final at = _y(speeds[i] + (speeds[i + 1] - speeds[i]) * f, height);
      if ((local.dy - at).abs() <= _lineGrab) return true;
    }
    return false;
  }

  /// Take every point inside [box] into the selection. `Shift` adds to what
  /// was already there; a plain box replaces it, and an empty one clears.
  void _catch(Rect box, double height) {
    if (!HardwareKeyboard.instance.isShiftPressed) _selected.clear();
    for (final clip in _clips) {
      final keys = _keysOf(clip);
      final speeds = envelopeSpeeds(keys);
      for (var i = 0; i < keys.length; i++) {
        final at = Offset(_xOfKey(clip, keys[i]), _y(speeds[i], height));
        if (box.contains(at)) _selected.add('${clip.id}#$i');
      }
    }
  }

  /// The envelope point [local] means: the nearest one within reach, or —
  /// failing that — the nearest point of whichever clip the pointer is over.
  ///
  /// The fallback is what makes the band usable. A point is a 7px dot on a
  /// 2px line; asking for a direct hit on one would make re-speeding a clip a
  /// test of aim, when the obvious reading of "drag anywhere on this clip's
  /// line" is never ambiguous — a clip's points are its own.
  ({BridgeClip clip, int index})? _nearestPoint(Offset local, double height) {
    ({BridgeClip clip, int index})? best;
    var bestD = 14.0;
    for (final clip in _clips) {
      final keys = _keysOf(clip);
      final speeds = envelopeSpeeds(keys);
      for (var i = 0; i < keys.length; i++) {
        final p = Offset(_xOfKey(clip, keys[i]), _y(speeds[i], height));
        final d = (p - local).distance;
        if (d < bestD) {
          bestD = d;
          best = (clip: clip, index: i);
        }
      }
    }
    if (best != null) return best;

    final over = _clipAt(local.dx);
    if (over == null) return null;
    final keys = _keysOf(over);
    var nearest = 0;
    var nearestD = double.infinity;
    for (var i = 0; i < keys.length; i++) {
      final d = (_xOfKey(over, keys[i]) - local.dx).abs();
      if (d < nearestD) {
        nearestD = d;
        nearest = i;
      }
    }
    return (clip: over, index: nearest);
  }

  /// The clip whose span covers [x] pixels.
  BridgeClip? _clipAt(double x) {
    for (final c in _clips) {
      final left = widget.axis.xOf(c.startFrame.toInt());
      final right = widget.axis.xOf(c.endFrame.toInt());
      if (x >= left && x < right) return c;
    }
    return null;
  }

  void _startDrag(Offset local, double height) {
    final found = _nearestPoint(local, height);
    if (found == null) return;
    final at = envelopeSpeeds(_keysOf(found.clip))[found.index];
    setState(() {
      _frozen = _range;
      _grabbedAt = at;
      _travelled = 0;
      _drag = (clip: found.clip, index: found.index, speed: at, dx: 0);
    });
  }

  /// `Ctrl`-click or double-click the line to plant a point; `Alt`-click one
  /// to lift it — the same gestures the graph editor uses, so nothing new has
  /// to be learnt for the strip.
  void _tap(Offset local, double height) {
    final doubled = widget.onTapped();
    final keys = HardwareKeyboard.instance;
    final found = _nearestPoint(local, height);

    if (found != null && keys.isAltPressed) {
      final all = _keysOf(found.clip);
      if (all.length <= 2) return; // never below the two ends
      _write(found.clip, [
        for (var i = 0; i < all.length; i++)
          if (i != found.index) all[i],
      ]);
      return;
    }
    if (!doubled && !keys.isControlPressed) return;

    final clip = _clipAt(local.dx);
    if (clip == null) return;
    final all = _keysOf(clip);
    final at = (widget.axis.frameAt(local.dx) - clip.startFrame) / widget.fps;
    final speeds = envelopeSpeeds(all);
    var index = all.length;
    for (var i = 0; i < all.length; i++) {
      if (at < rationalSeconds(all[i].time)) {
        index = i;
        break;
      }
    }
    if (index == 0 || index == all.length) return; // only between the ends
    final t0 = rationalSeconds(all[index - 1].time);
    final t1 = rationalSeconds(all[index].time);
    final f = t1 > t0 ? (at - t0) / (t1 - t0) : 0.0;
    final planted = speeds[index - 1] + (speeds[index] - speeds[index - 1]) * f;
    final grown = [...all]..insert(
        index,
        BridgeKeyframe(
          time: timeOfSubframe(at * widget.fps, widget.fpsNum, widget.fpsDen),
          value: 0,
          interpIn: const BridgeSideInterp.linear(),
          interpOut: const BridgeSideInterp.linear(),
        ));
    _write(clip, envelopeToKeys(grown, [...speeds]..insert(index, planted)));
  }

  void _commit() {
    final held = _drag;
    if (held == null) return;
    // Every clip the selection reaches into, not just the one under the
    // pointer: a box drawn across two clips moves the points in both.
    final touched = _selected.contains('${held.clip.id}#${held.index}')
        ? _clips
            .where((c) => _selected.any((s) => s.startsWith('${c.id}#')))
            .toList()
        : [held.clip];
    for (final clip in touched) {
      final keys = clip.id == held.clip.id
          ? _moved(
              clip,
              _withSpeed(clip, _keysOf(clip), held.index, held.speed),
              held.index,
              held.dx,
            )
          : _withSpeed(clip, _keysOf(clip), held.index, held.speed);
      _write(clip, keys);
    }
    setState(() => _drag = null);
  }

  void _write(BridgeClip clip, List<BridgeKeyframe> keys) {
    widget.entry.layer.setClipRetime(
      clip: clip.id,
      value: BridgeScalar.keyframed(keys),
    );
    widget.onChanged();
  }
}

/// The envelope's furniture and its lines: the 100% reference every clip is
/// measured against, the zero line that says where backwards begins, and each
/// clip's own straight run of points.
class _EnvelopePainter extends CustomPainter {
  final List<({BridgeClip clip, List<BridgeKeyframe> keys})> lanes;
  final double Function(BridgeClip, BridgeKeyframe) xOfKey;
  final double Function(double) y;
  final (double, double) range;
  final Color line;
  final Color curve;

  /// The accent a caught point takes, and which points are caught.
  final Color chosen;
  final Set<String> selected;
  final TextStyle label;

  /// Where the viewport's left edge sits in the canvas's own coordinates.
  final double viewportLeft;

  const _EnvelopePainter({
    required this.lanes,
    required this.xOfKey,
    required this.y,
    required this.range,
    required this.line,
    required this.curve,
    required this.chosen,
    required this.selected,
    required this.label,
    required this.viewportLeft,
  });

  @override
  void paint(Canvas canvas, Size size) {
    for (final (speed, text) in [(100.0, '100%'), (0.0, '0')]) {
      final at = y(speed);
      if (at < 0 || at > size.height) continue;
      // Dotted, so the graph's own reference lines never read as the row
      // seams that rule the rest of the table — solid, they were the same
      // mark meaning two different things.
      final paint = Paint()
        ..color = line
        ..strokeWidth = 1;
      for (var x = 0.0; x < size.width; x += 6) {
        canvas.drawLine(
            Offset(x, at), Offset((x + 3).clamp(0, size.width), at), paint);
      }
      final painter = TextPainter(
        text: TextSpan(text: text, style: label),
        textDirection: TextDirection.ltr,
      )..layout();
      // At the window's edge, not the content's: a label at canvas x 0 is at
      // the start of time and scrolls away the moment the lane moves.
      painter.paint(canvas, Offset(viewportLeft + 2, at - painter.height));
    }

    final stroke = Paint()
      ..color = curve
      ..strokeWidth = 2
      ..style = PaintingStyle.stroke;
    for (final lane in lanes) {
      final speeds = envelopeSpeeds(lane.keys);
      final points = [
        for (var i = 0; i < lane.keys.length; i++)
          Offset(xOfKey(lane.clip, lane.keys[i]), y(speeds[i])),
      ];
      if (points.length < 2) continue;
      final path = Path()..moveTo(points.first.dx, points.first.dy);
      for (final p in points.skip(1)) {
        path.lineTo(p.dx, p.dy);
      }
      canvas.drawPath(path, stroke);
      for (var i = 0; i < points.length; i++) {
        final caught = selected.contains('${lane.clip.id}#$i');
        canvas.drawCircle(
          points[i],
          caught ? 4.5 : 3.5,
          Paint()..color = caught ? chosen : curve,
        );
      }
    }
  }

  @override
  bool shouldRepaint(_EnvelopePainter old) =>
      old.lanes != lanes ||
      old.range != range ||
      old.curve != curve ||
      old.selected != selected ||
      old.viewportLeft != viewportLeft;
}
