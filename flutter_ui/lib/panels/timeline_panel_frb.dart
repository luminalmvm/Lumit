// The Timeline panel, on the flutter_rust_bridge API.
//
// Two columns side by side over one shared time axis: an **outline** on the
// left (layer number, label chip, name, switches, blend mode, parent) and a
// **layer area** on the right (the ruler, the playhead, one bar per layer, the
// work area and the markers). Everything draws from the comp read model
// (state/comp_model.dart, K-184); edits go out through the reference handles.
//
// **What is here.** Adding every layer kind, deleting, duplicating, reordering,
// the eight switches, blend mode, parenting, dragging and trimming a layer's
// bar, scrubbing the playhead, the work area and marker cues.
//
// The **Graph** button swaps the layer area for the graph editor
// (graph_editor_frb.dart), which shapes the selected layer's curves.
//
// **The one rule the drags follow.** A bar drag is a live *preview* of nothing —
// unlike an effect or transform drag there is no cheap render to show, because
// moving a layer in time changes what every frame contains. So a bar drag holds
// its offset in Dart and commits one `set_span` on release: one op, one undo
// step, even when the gesture moved the in point and the start offset together.

import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/l10n/engine_labels.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/splash.dart';
import '../state/comp_model.dart';
import '../state/comp_time.dart';
import '../state/dock.dart';
import '../state/drag_payloads.dart';
import '../state/timecode.dart';
import '../state/timeline_columns.dart';
import '../state/tools.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/marquee.dart';
import '../widgets/time_readout.dart';
// The ruler helpers moved with the ruler (shared with the graph editor); the
// re-export keeps their long-standing import path alive for their tests.
export 'timeline_extras_frb.dart' show rulerLabelStepSeconds, rulerLabelOf;

import 'placeholder.dart';
import 'easing_curve.dart';
import 'easing_editor.dart';
import 'graph_editor_frb.dart';
import 'graph_maths.dart';
import 'package:lumit_flutter/state/preview_throttle.dart';
import 'timeline_extras_frb.dart';
import 'sequence_view_frb.dart';
import 'timeline_razor.dart';
import 'effect_param_row_frb.dart';
import 'keyframe_controls_frb.dart';
import 'layer_fold_frb.dart';
import 'package:lumit_flutter/src/rust/api/retime.dart';
import 'flow_rows_frb.dart';
import 'fx_section.dart';
import '../widgets/smooth_zoom.dart';
import '../widgets/zoom_anchored_scroll.dart';
import 'timeline_snap.dart';
import 'waveform_frb.dart';
import 'timeline_timings.dart';
import 'transform_rows_frb.dart';

/// The blend-mode names, fetched once per session: the list is static for the
/// life of the process, and every outline row was re-fetching it per rebuild.
List<String>? _blendModes;

/// The engine's answers to "what does this curve read at this time",
/// remembered per (scalar, time) — the same bargain state/comp_time.dart
/// strikes for frame↔time: the engine still computes each answer, once,
/// rather than once per rebuild of every animated row (K-184). A freezed
/// scalar compares by value, so an edited curve is a new question here, never
/// a stale answer; the ceiling only stops a long session growing forever.
final Map<(BridgeScalar, BridgeRational), double> _scalarSamples = {};

double sampledScalar(BridgeScalar scalar, BridgeRational time) {
  if (_scalarSamples.length >= 8192) _scalarSamples.clear();
  return _scalarSamples[(scalar, time)] ??=
      sampleScalar(scalar: scalar, time: time);
}

/// One layer row's height.
const double _rowHeight = 22;

/// The outline's two header rows: the toolbar (timecode, search, the view
/// buttons) and the column-group header under it.
const double _toolbarHeight = 26;

/// The lane side's bottom bar (zoom, magnet, the horizontal scrollbar).
///
/// **The outline reserves the same height below its rows**, and that is not
/// decoration: the two halves are one table, and a viewport that is shorter on
/// one side can be scrolled further than the other. The lanes could run past the
/// outline's last row by exactly this bar's height, and the halves came apart at
/// the bottom of a long stack — reported as "the lane area can scroll up more
/// than the layer area". Reserving it keeps both viewports the same height,
/// which is what keeps `maxScrollExtent` the same on both.
const double _laneBottomBarHeight = 20;
const double _headerHeight = 20;

/// Half a keyframe diamond's width on a property's own lane.
const double _keyHalf = 4;

/// The same on a **shut layer's** row (§12A.1): half the scale, because these
/// are a summary of everything keyed inside the layer rather than the keys you
/// take hold of. Twirl the layer open and each property draws its own at full
/// size, where they can be dragged.
const double _summaryKeyHalf = _keyHalf / 2;

/// The two landscapes flanking the zoom slider (K-293). Painter-drawn, so
/// K-209's 16px floor — which is about an icon-set glyph's 1.5-unit stroke
/// falling on less than a pixel — does not apply: a filled shape has no stroke
/// to lose. They sit inside a 20px bar, and the pair has to differ plainly
/// enough to read as "less of this / more of this" at a glance.
const double _zoomGlyphSmall = 9;
const double _zoomGlyphLarge = 14;

/// The time ruler's height: the toolbar and column header stay inside the
/// outline (docs/07 §4.1), so the lane side gives their whole height to the
/// ruler — minus the cache bar tucked under it. That is what makes the ruler
/// **double height** (docs/15 §12A.1): the upper half is the clock, carrying
/// the labels, the ticks and the playhead's head, and the lower half carries
/// the markers and the work-area band. A taller bar is an easier playhead grab
/// as well, but the two rows are the point.
const double _rulerHeight =
    _toolbarHeight + _headerHeight - TimelineCacheBar.height;

/// How near the end of a bar counts as grabbing its edge to trim rather than its
/// middle to move.
const double _trimGrab = 8;

/// Which part of a bar [width] pixels wide a press at [dx] takes hold of.
///
/// Each trim zone is [_trimGrab] wide but never more than a third of the bar,
/// so a bar only a few frames long still keeps a middle to move by — without
/// the cap, a short bar was all edge and could not be dragged along the
/// timeline at all.
BarGrab barGrabAt(double dx, double width) {
  final edge = min(_trimGrab, width / 3);
  if (dx < edge) return BarGrab.trimIn;
  if (dx > width - edge) return BarGrab.trimOut;
  return BarGrab.move;
}

/// A layer drag in flight: the index lifted, and the index it would land on.
///
/// **Held by the panel and read by both halves of the table**, which is the
/// point (K-208). The outline owns the gesture — the name is the stack handle
/// — so when only it knew about the drag, only it could move: the lanes sat
/// still while their layers were being reordered beside them. One value, read
/// by the outline rows and the lane blocks alike, and the two halves slide as
/// one row because they are working from the same number.
class LayerDrag {
  final int from;
  final int to;
  const LayerDrag(this.from, this.to);

  @override
  bool operator ==(Object other) =>
      other is LayerDrag && other.from == from && other.to == to;

  @override
  int get hashCode => Object.hash(from, to);
}

/// One layer as **both halves of the table see it**: the rows it shows, the
/// room an open Sequence view wants, and the height those come to.
///
/// The outline and the lane area are still two widget trees, and they have to
/// be: they sit in two scroll views, and only the lane half rebuilds when the
/// zoom moves (K-293). What they must never be is two *opinions*. Each half
/// used to walk [layerFoldRows] for itself, test `open` for itself and look up
/// `sequenceExtra` for itself, and the table was level only because all three
/// pairs happened to agree — a layer could grow a row on one side and not the
/// other, and nothing would say so. Decided once here, the halves can differ in
/// what they draw but not in what a layer *is*.
class LayerRow {
  final BridgeLayerEntry entry;

  /// The layer's id as a string, which is the key everything else is filed
  /// under — worked out once rather than at each of the dozen places that
  /// wanted it.
  final String id;

  /// Twirled open — whether [foldRows] are **drawn**.
  final bool open;

  /// The rows this layer's fold-out has, open or shut.
  ///
  /// Held for a shut layer too, because snapping wants them: a keyframe is
  /// somewhere in time whether or not its row is on screen, and a key drag or
  /// a razor cut can land on it either way (`_snapTargets`, K-292). Everything
  /// that *draws* reads [drawnRows] instead — the one place that difference is
  /// written down, rather than an `open` test remembered at each of five call
  /// sites.
  final List<LayerFoldRow> foldRows;

  /// The rows the table draws under this layer: its fold-out when it is
  /// twirled open, nothing when it is not.
  List<LayerFoldRow> get drawnRows => open ? foldRows : const [];

  /// How much taller an open Sequence view makes this row (K-248), or null
  /// when the layer has no view open. **The outline reserves exactly the room
  /// the lanes draw the view in**, or every row below sits at a different
  /// height on the two sides.
  final double? sequenceExtra;

  /// Whether this layer's source carries sound, and whether it has a picture
  /// to show (K-435). What the switches column reads to draw only the switches
  /// this layer can actually use: no audible switch on a solid that has never
  /// made a sound, no visibility switch on a music track that has never shown
  /// anything.
  ///
  /// Carried here, decided once by the panel, because answering either means
  /// probing the media — the cost K-184 exists to keep out of a row's build.
  final bool hasAudio;
  final bool hasPicture;

  /// Every keyframe anywhere on this layer, for the diamonds its **own** row
  /// draws while it is shut (§12A.1). Empty while the layer is open: each
  /// property then draws its own on its own lane, and saying it twice would
  /// put a small diamond behind every large one.
  final List<BridgeKeyframe> summaryKeys;

  const LayerRow({
    required this.entry,
    required this.id,
    required this.open,
    required this.foldRows,
    this.summaryKeys = const [],
    required this.sequenceExtra,
    this.hasAudio = false,
    this.hasPicture = true,
  });

  /// This block's height: its own row, the rows it draws, and its open view.
  double get height =>
      _rowHeight * (1 + drawnRows.length) + (sequenceExtra ?? 0);
}

/// What a column group is called — in its header, and on the bottom bar's
/// toggle for it (K-448), which must name the same thing the header does.
String columnGroupLabel(TimelineGroup group) => switch (group) {
      TimelineGroup.switches => l10n.columnAv,
      TimelineGroup.identity => l10n.columnLayer,
      TimelineGroup.render => l10n.columnSwitches,
      TimelineGroup.compose => l10n.columnCompose,
      TimelineGroup.timings => l10n.tipRenderTime,
    };

/// Decide every layer's row, once for the whole panel. `flowParams` and
/// `volumeDb` are the panel's once-per-revision reads, riding down onto the
/// fold rows (K-184).
List<LayerRow> layerRows({
  required List<BridgeLayerEntry> layers,
  required Set<String> open,
  required Map<String, bool> hasAudio,
  Map<String, bool> hasPicture = const {},
  Map<String, double> sequenceExtra = const {},
  Map<String, BridgeFlowParams> flowParams = const {},
  Map<String, BridgeScalar> volumeDb = const {},
}) {
  final out = <LayerRow>[];
  for (final entry in layers) {
    final id = entry.layer.internallayerId.toString();
    out.add(LayerRow(
      entry: entry,
      id: id,
      open: open.contains(id),
      foldRows: layerFoldRows(
          entry: entry,
          open: open,
          hasAudio: hasAudio[id] ?? false,
          flowParams: flowParams[id],
          volumeDb: volumeDb[id]),
      // Only for a shut layer: an open one shows the real thing.
      summaryKeys: open.contains(id)
          ? const []
          : layerKeys(
              entry: entry,
              flowParams: flowParams[id],
              volumeDb: volumeDb[id],
            ),
      sequenceExtra: sequenceExtra[id],
      hasAudio: hasAudio[id] ?? false,
      // Until the probe has answered, a layer is assumed to have a picture:
      // the visibility switch is the one every layer but a music track uses,
      // so appearing and then going is far less startling than the reverse.
      hasPicture: hasPicture[id] ?? true,
    ));
  }
  return out;
}

/// How far the block at [index] slides while a drag is in flight, in pixels;
/// positive is down.
///
/// The lifted block travels the whole way to the slot it would take, and every
/// block it passes moves one lift's height the other way — so the stack reads
/// as already reordered before the drop, which is what makes a drop feel
/// decided rather than guessed at. Pure, so the maths both halves depend on is
/// tested without building a Timeline.
double layerDragShift(List<double> heights, LayerDrag? drag, int index) {
  if (drag == null || drag.from == drag.to) return 0;
  if (index < 0 || index >= heights.length) return 0;
  if (drag.from < 0 || drag.from >= heights.length) return 0;
  if (drag.to < 0 || drag.to >= heights.length) return 0;
  if (index == drag.from) {
    var travel = 0.0;
    if (drag.to > drag.from) {
      for (var i = drag.from + 1; i <= drag.to; i++) {
        travel += heights[i];
      }
      return travel;
    }
    for (var i = drag.to; i < drag.from; i++) {
      travel -= heights[i];
    }
    return travel;
  }
  final lifted = heights[drag.from];
  if (drag.to > drag.from) {
    return index > drag.from && index <= drag.to ? -lifted : 0;
  }
  return index >= drag.to && index < drag.from ? lifted : 0;
}

/// Which slot a drag is aiming at, from how far it has travelled.
///
/// [from] is the block lifted, [travel] how far the pointer has moved down the
/// stack since the lift in pixels (negative is up). Returns the index the block
/// would take if dropped now.
///
/// **Measured against the stack as it was when the drag began**, which is the
/// whole point. The rows on screen are slid out of the way while a drag is in
/// flight, so asking "which row is the pointer over?" asks about geometry the
/// drag itself is moving: each answer slides the rows, which changes the next
/// answer, and the block oscillates between two slots without the pointer
/// moving at all. Travel against the original heights cannot do that — it is
/// a function of the pointer alone.
///
/// The threshold is the midpoint of the block being passed, not its edge: an
/// edge means the slot flips the instant a single pixel of overlap appears,
/// which is the other half of the same jitter. Travelling back to where the
/// drag started therefore returns [from] exactly, so a cancelled-by-hand drag
/// leaves the stack alone.
int layerDragTarget(List<double> heights, int from, double travel) {
  if (from < 0 || from >= heights.length) return from;
  var to = from;
  if (travel > 0) {
    var passed = 0.0;
    for (var i = from + 1; i < heights.length; i++) {
      if (travel < passed + heights[i] / 2) break;
      passed += heights[i];
      to = i;
    }
  } else if (travel < 0) {
    var passed = 0.0;
    for (var i = from - 1; i >= 0; i--) {
      if (-travel < passed + heights[i] / 2) break;
      passed += heights[i];
      to = i;
    }
  }
  return to;
}

/// Which slot footage dropped from the Project panel takes, from how far down
/// the stack it landed.
///
/// [y] is measured from the top of the first block, so the caller subtracts the
/// pinned toolbar and header and adds the scroll. A drop in a block's top half
/// goes above it and one in its bottom half below, the same midpoint rule a
/// layer drag uses — and a drop past the last block lands at the bottom.
int layerDropSlot(List<double> heights, double y) {
  var top = 0.0;
  for (var i = 0; i < heights.length; i++) {
    if (y < top + heights[i] / 2) return i;
    top += heights[i];
  }
  return heights.length;
}

/// One layer's block, slid out of a dragged layer's way.
///
/// A transform, not a layout change: the rows keep their places, so a drag
/// never reflows the table under itself — and the same widget wraps the block
/// in the outline and the block in the lanes, which is what keeps them
/// together to the pixel.
class LayerDragSlide extends StatelessWidget {
  final ValueListenable<LayerDrag?> drag;
  final List<double> heights;
  final int index;
  final Widget child;

  const LayerDragSlide({
    super.key,
    required this.drag,
    required this.heights,
    required this.index,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    // The user's animation level, not a constant: at *None* the rows must
    // arrive without travelling at all (15-DESIGN §8), and a hard-coded
    // duration here would be one animation the setting could not reach.
    final duration = animationDuration(ThemeScope.of(context).animationLevel);
    return ValueListenableBuilder<LayerDrag?>(
      valueListenable: drag,
      child: child,
      builder: (context, value, child) {
        final height = index < heights.length ? heights[index] : 0.0;
        return AnimatedSlide(
          offset: height <= 0
              ? Offset.zero
              : Offset(0, layerDragShift(heights, value, index) / height),
          duration: duration,
          curve: Curves.easeOut,
          child: child,
        );
      },
    );
  }
}

class TimelinePanelFrb extends StatefulWidget {
  const TimelinePanelFrb({super.key});

  @override
  State<TimelinePanelFrb> createState() => _TimelinePanelFrbState();
}

class _TimelinePanelFrbState extends State<TimelinePanelFrb>
    with SingleTickerProviderStateMixin {
  /// What is twirled open: layer ids, and the paths of the groups under them
  /// (`<layer>/transform`, `<layer>/effects/<effect>`, `<layer>/audio`). Held by
  /// the panel rather than by each row so the lane side can leave room for
  /// exactly the rows the outline draws — the two halves are one table, and a
  /// name that does not line up with its bar is worse than no fold-out at all.
  final Set<String> _open = {};

  /// Which layers' sources carry sound, by id. Cached because answering it
  /// probes the file with FFmpeg, which must never happen in a build — the same
  /// reason the Project panel caches missing media. Absent means "not asked
  /// yet", and a layer with no entry simply shows no Audio group until the
  /// answer arrives.
  final Map<String, bool> _hasAudio = {};

  /// Which layers have a picture to show, by id (K-435) — the mirror of
  /// [_hasAudio], cached for the same reason and filled in the same pass.
  /// A layer with no entry is assumed to have one: the visibility switch is
  /// the one nearly every layer uses.
  final Map<String, bool> _hasPicture = {};

  /// Each layer's waveform peaks, by id — the stretch of its source the lanes
  /// are currently showing, summarised to one bucket per pixel column (K-280).
  ///
  /// Refetched when the zoom or the scroll moves the window far enough to
  /// matter, which is what keeps the drawn detail level with the zoom instead
  /// of blocky. Peaks belong to the file, so the painter maps them through the
  /// live in/out/offset and a drag or a trim carries the transients with it
  /// without asking again (K-172).
  final Map<String, BridgeAudioPeaks> _peaks = {};

  /// What each layer's peaks were fetched for: the window, the bucket count
  /// and the wave style. Equal keys mean the answer in hand is still the right
  /// one, so nothing is asked again — a scroll of a few pixels rounds to the
  /// same key by design ([WaveformRequest]).
  final Map<String, String> _peakKeys = {};

  /// The lanes' viewport width as the last layout measured it. Written during
  /// build and never read to decide layout.
  ///
  /// Two things read it: how much audio a waveform lane is showing, which is
  /// what the peak window is worked out from (K-280); and the zoom, which
  /// needs the width at magnification 1 to know what a frame is worth in
  /// pixels *now* — it cannot ask the axis, because the axis is rebuilt from
  /// the zoom itself ([_laneFrames] is its other half, K-293).
  double _laneViewport = 0;

  /// Each Footage layer's source length in comp frames, by layer id. Cached
  /// for the same reason [_hasAudio] is: the answer comes from probing the
  /// file with FFmpeg, which must never happen in a build. Absent means "not
  /// asked yet"; a null value means the answer never came (no media feature,
  /// missing file), which leaves that layer's ends free.
  final Map<String, int?> _footageFrames = {};

  /// How far each layer's ends may be dragged, by layer id (K-211) — what the
  /// bars trim within and draw their corner marks from.
  Map<String, BarBounds> _barBounds = {};

  /// The document revision [_barBounds] was worked out at. A precomp's length
  /// and a layer's Retime are both one edit away from changing, so the bounds
  /// are taken again whenever the document moves — and never in between, which
  /// is what keeps a rebuild free of bridge calls (K-184).
  BigInt? _boundsRevision;

  /// How waveforms draw — Settings ▸ Interface ▸ Editing (K-280, K-285).
  WaveformStyle get _waveformStyle {
    final interface =
        Provider.of<LumitUiState>(context, listen: false).workspace.interface;
    return WaveformStyle(
      multiwave: interface.multiwaveWaveforms,
      fromBottom: interface.waveformsFromBottom,
    );
  }

  /// Fetch peaks for every layer whose Waveform twirl is open, over the stretch
  /// of audio the lanes are showing right now.
  ///
  /// **This is what makes the resolution follow the zoom.** The old lane asked
  /// once for 2 048 buckets across the whole source and kept them for the
  /// session, so zooming in stretched the same coarse summary until it was a
  /// staircase (K-172, superseded here). Now the window is the visible one and
  /// the buckets are the visible pixel columns, so a wave has as much detail as
  /// there is room to show, at any zoom.
  ///
  /// Called from the build *and* from the lanes' scroll, because scrolling
  /// moves the window without changing anything the panel rebuilds for. The
  /// request rounds itself off, so an ordinary scroll asks nothing new and only
  /// a real move sends a fetch.
  void _refreshPeaks(List<BridgeLayerEntry> layers) {
    final ui = _ui;
    if (ui == null) return;
    final frames = ui.model.durationFrames;
    final fps = ui.model.fps;
    final width = _laneViewport * _zoom;
    if (frames <= 0 || fps <= 0 || width <= 0 || _laneViewport <= 0) return;
    // Only the band split reaches the engine: where the wave sits is a
    // drawing decision, so toggling it repaints and fetches nothing.
    final multiwave = _waveformStyle.needsBands;
    // The comp seconds under the lanes' window, from the same mapping the axis
    // draws with.
    final maxOffset = max(0.0, width - _laneViewport);
    final offset =
        _hLane.hasClients ? _hLane.offset.clamp(0.0, maxOffset) : 0.0;
    final secondsPerPixel = frames / fps / width;
    final viewStart = offset * secondsPerPixel;
    final viewEnd = (offset + _laneViewport) * secondsPerPixel;

    for (final entry in layers) {
      final id = entry.layer.internallayerId.toString();
      if (!_open.contains(waveformPath(id))) {
        // A shut lane keeps nothing: the window it was fetched for is stale by
        // the time it opens again, and the memory is a whole track's summary.
        _peaks.remove(id);
        _peakKeys.remove(id);
        continue;
      }
      final span = entry.info.span;
      final startOffset = rationalSeconds(span.startOffset);
      // The layer's own source clock: comp time less where its source starts.
      final request = WaveformRequest.forView(
        startSeconds: viewStart - startOffset,
        endSeconds: viewEnd - startOffset,
        pixels: _laneViewport,
      );
      if (request == null) continue;
      // A retimed layer's buckets are taken through its Retime map (K-436), so
      // reshaping the map changes the answer without moving the window. Only a
      // retimed layer pays for that: an ordinary one keys on the window alone
      // and an edit anywhere else in the document asks for nothing.
      final retimed =
          entry.info.retime == null ? '' : '|${ui.model.heldRevision}';
      final key = '${request.key}|$multiwave$retimed';
      // Claimed before the fetch starts, so a rebuild mid-decode does not ask
      // twice for the same window.
      if (_peakKeys[id] == key) continue;
      _peakKeys[id] = key;
      entry.layer
          .audioPeaks(
        startSeconds: request.startSeconds,
        endSeconds: request.endSeconds,
        buckets: request.buckets,
        multiwave: multiwave,
      )
          .then((peaks) {
        // A later window may already have been asked for while this one was
        // decoding; the newest ask wins, so an old answer is dropped rather
        // than drawn over a lane that has moved on.
        if (!mounted || _peakKeys[id] != key) return;
        setState(() => _peaks[id] = peaks);
      });
    }
  }

  /// The lanes scrolled: the visible window moved, so the waveforms may want a
  /// finer summary of somewhere else. Nothing is rebuilt here — the fetch calls
  /// `setState` only when an answer actually arrives.
  void _onLaneScroll() {
    if (_peakKeys.isEmpty && _peaks.isEmpty) return;
    _refreshPeaks(_lastLayers);
  }

  /// The layers the last build drew, so a scroll can refresh their peaks
  /// without a rebuild to hand them over.
  List<BridgeLayerEntry> _lastLayers = const [];

  /// Work out how far every layer's ends may be dragged (K-211).
  ///
  /// Two costs, kept apart. A **footage** length means opening the file, so it
  /// is asked once per layer, off the build, and kept for the session — the
  /// same bargain [_refreshPeaks] strikes. Everything else is a cheap read that
  /// an edit can change (a precomp lengthened in its comp settings, Retime
  /// switched on), so the whole table is rebuilt when — and only when — the
  /// document revision moves.
  void _refreshBounds(CompModel model, int fpsNum, int fpsDen) {
    final layers = model.layers;
    for (final entry in layers) {
      final id = entry.layer.internallayerId.toString();
      if (entry.info.kind != BridgeLayerKind.footage ||
          _footageFrames.containsKey(id)) {
        continue;
      }
      // Claim the slot first, so a rebuild mid-probe does not probe twice.
      _footageFrames[id] = null;
      final ItemReference? source;
      try {
        source = entry.layer.getSourceItem();
      } catch (_) {
        continue;
      }
      if (source is! ItemReference_Footage) continue;
      source.field0.mediaInfo().then((info) {
        if (!mounted || info == null) return;
        setState(() {
          _footageFrames[id] = frameOfTime(info.duration, fpsNum, fpsDen);
          // The answer changes the bounds, and the document has not moved:
          // forget the revision so the next build works them out again.
          _boundsRevision = null;
        });
      });
    }

    final revision = model.revision;
    if (revision != null && revision == _boundsRevision) return;
    _boundsRevision = revision;
    _barBounds = {
      for (final entry in layers)
        entry.layer.internallayerId.toString():
            _boundsOf(entry, fpsNum, fpsDen),
    };
    // The Flow group's parameters and the Volume scalar, for the fold-out's
    // rows: neither is in the read model, so they are read here — once per
    // document revision, on the same bargain as the bounds — and ride down on
    // the rows rather than being asked for per rebuild (K-184).
    final flowParams = <String, BridgeFlowParams>{};
    final volumeDb = <String, BridgeScalar>{};
    for (final entry in layers) {
      final id = entry.layer.internallayerId.toString();
      try {
        if (entry.info.flow) flowParams[id] = entry.layer.getFlowParams();
        if (_hasAudio[id] ?? false) volumeDb[id] = entry.layer.getVolumeDb();
      } catch (_) {
        // A layer gone between the model read and this: its rows go too.
      }
    }
    _flowParams = flowParams;
    _volumeDb = volumeDb;
  }

  /// Per-layer answers the fold rows carry (K-184) — see [_refreshBounds].
  Map<String, BridgeFlowParams> _flowParams = {};
  Map<String, BridgeScalar> _volumeDb = {};

  /// The work area, held between document revisions — see the note in [_body].
  ({int start, int end, bool whole})? _workArea;
  BigInt? _workRevision;
  CompositionReference? _workComp;

  /// One layer's bounds, from what its kind can be asked cheaply.
  BarBounds _boundsOf(BridgeLayerEntry entry, int fpsNum, int fpsDen) {
    final info = entry.info;
    // Retime frees both ends (docs/04-RETIMING.md). One question now, not two:
    // K-249 left the Retime property as the only map, so the read model's own
    // field is the whole answer and the bar no longer crosses the bridge to
    // ask a second time.
    final retimed = info.retime != null;
    int? sourceFrames;
    try {
      switch (info.kind) {
        case BridgeLayerKind.footage:
          sourceFrames = _footageFrames[entry.layer.internallayerId.toString()];
        case BridgeLayerKind.precomp:
          final source = entry.layer.getSourceItem();
          if (source is ItemReference_Composition) {
            sourceFrames = frameOfTime(
                source.field0.getSettings().duration, fpsNum, fpsDen);
          }
        default:
          // Every generated kind: nothing to run out of, both ends free.
          sourceFrames = null;
      }
    } catch (_) {
      // A layer that has gone, or a source that cannot be read: free ends
      // rather than a bar pinned to a guess.
      return BarBounds.free;
    }
    return barBounds(
      startOffsetFrame: frameOfTime(info.span.startOffset, fpsNum, fpsDen),
      sourceFrames: sourceFrames,
      retimed: retimed,
    );
  }

  /// Twirl a fold open or shut. Shutting one drops the selection inside it
  /// (K-203): a selected property that is no longer on screen is a highlight
  /// with nowhere to sit, and it came back as soon as the fold reopened — on a
  /// layer the user had since stopped working on.
  void _toggle(String path) => setState(() {
        if (_open.remove(path)) {
          _dropSelectionUnder(path);
        } else {
          _open.add(path);
        }
      });

  /// Forget any selected property at or below [path], and any keyframes of
  /// theirs the marquee had caught.
  void _dropSelectionUnder(String path) {
    _selectedProperties.removeWhere((p) => p == path || isUnderPath(path, p));
    _laneKeySelection.removeWhere((id) {
      final hash = id.lastIndexOf('#');
      if (hash <= 0) return false;
      final row = id.substring(0, hash);
      return row == path || isUnderPath(path, row);
    });
    _graphKeySelection.clear();
  }

  /// Nothing selected: no layer, no properties, no keyframes (K-203).
  ///
  /// Clicking empty space in either half of the table is how you get here. An
  /// editor with no way *out* of a selection makes every following command
  /// ambiguous — Delete, U and the Retime chord all read the selection first,
  /// and until now the only way to change it was to pick something else.
  void _deselectAll(LumitUiState ui) {
    if (ui.selectedLayer.value == null &&
        _selectedProperties.isEmpty &&
        _laneKeySelection.isEmpty &&
        _graphKeySelection.isEmpty &&
        _highlighted == null) {
      return;
    }
    setState(() {
      // Clears the list as well as the primary — `_syncSelection` only follows
      // the primary the other way (one layer set becomes the whole selection),
      // so dropping it alone would leave the list holding what was let go.
      ui.clearSelection();
      _selectedProperties.clear();
      _laneKeySelection.clear();
      _graphKeySelection.clear();
      _highlighted = null;
    });
  }

  /// Select a layer by click: plain replaces, Ctrl toggles, Shift extends the
  /// range down the stack — the same three rules a property row follows
  /// ([_selectProperty]), because a selection that behaved one way for rows
  /// and another for layers would be two selections to learn.
  ///
  /// The list is the shell's (K-217), so this hands the work to
  /// [LumitUiState.setSelection] and [LumitUiState.toggleSelected] rather than
  /// keeping a second idea of what is selected: the Viewer's boxes, Delete and
  /// the split all read that one list.
  ///
  /// A layer's properties are not selected with it: a click on a layer's name
  /// means "this layer", and leaving a property of the layer before it lit is
  /// the highlight belonging to nothing on screen that K-203 went looking for.
  void _selectLayer(LumitUiState ui, LayerReference? layer,
          {List<BridgeLayerEntry> among = const []}) =>
      setState(() {
        if (ui.selectedLayer.value?.internallayerId != layer?.internallayerId) {
          // The one place this is decided, shared with the listener that
          // catches a selection made in the Viewer (K-275). The highlight goes
          // with the property selection it belongs to: left behind, the
          // previous layer's row stayed lit after a click on a different
          // layer.
          _dropLayerLocalSelection();
        }
        if (layer == null) {
          ui.clearSelection();
          return;
        }
        final keys = HardwareKeyboard.instance;
        if (keys.isControlPressed || keys.isMetaPressed) {
          ui.toggleSelected(layer);
          return;
        }
        final held = ui.selectedLayer.value;
        if (keys.isShiftPressed && held != null) {
          final a = among.indexWhere(
              (e) => e.layer.internallayerId == held.internallayerId);
          final b = among.indexWhere(
              (e) => e.layer.internallayerId == layer.internallayerId);
          if (a >= 0 && b >= 0) {
            // The clicked layer stays the primary — it is the one just asked
            // for, and everything that acts on one layer acts on that.
            ui.setSelection([
              layer,
              for (var i = a < b ? a : b; i <= (a < b ? b : a); i++)
                if (i != b) among[i].layer,
            ]);
            return;
          }
        }
        ui.setSelection([layer]);
      });

  /// Fill in any layer's has-audio and has-picture answers we do not have, off
  /// the build. Both come from probing the source, so both are asked once per
  /// layer and remembered (K-184, K-435).
  void _refreshAudio(List<BridgeLayerEntry> layers) {
    for (final entry in layers) {
      final id = entry.layer.internallayerId.toString();
      if (_hasAudio.containsKey(id)) continue;
      // Claim the slot first, so a rebuild mid-probe does not probe twice.
      _hasAudio[id] = false;
      // Asked here, beside the audio question, and never from a build: it is
      // synchronous across the seam, so a build asking it would probe on the
      // UI thread. The answer decides whether the row draws a visibility
      // switch at all, so it is wanted for every layer, not only footage.
      _hasPicture[id] = entry.layer.hasPicture();
      entry.layer.hasAudio().then((has) {
        if (!mounted || _hasAudio[id] == has) return;
        setState(() {
          _hasAudio[id] = has;
          // A layer with sound has a Volume scalar to fetch, and the document
          // has not moved: forget the revision so the next build reads it.
          _boundsRevision = null;
        });
      });
    }
  }

  String _search = '';

  /// The shy filter (docs/07 §4.2): while on, layers whose shy switch is set
  /// disappear from the list — not from the picture; shy never renders.
  bool _hideShy = false;

  /// The outline's column groups in their current order. Dragging a header
  /// group reorders them as a unit; session-lived, like the twirl state.
  List<TimelineGroup> _groupOrder = [...defaultGroupOrder];

  /// Each group's width. Dragging a header seam changes one of these and
  /// leaves the rest alone, so the outline grows by what the drag moved.
  Map<TimelineGroup, double> _groupWidths = {...defaultGroupWidths};

  /// The column groups the bottom bar has switched **off** (K-448, §12A.1),
  /// so the outline pares down to names and bars when the columns are not in
  /// use. Session-lived, like the order and the widths. The identity group is
  /// never in here: names and bars are what "pared down" means, and a table
  /// with no first column is a table of nothing.
  final Set<TimelineGroup> _hiddenGroups = <TimelineGroup>{};

  /// The groups a bottom-bar toggle offers, in the order the bar shows them.
  static const List<TimelineGroup> _toggleableGroups = [
    TimelineGroup.switches,
    TimelineGroup.render,
    TimelineGroup.compose,
  ];

  /// Widen (or narrow) one group, never below what its cells need.
  void _resizeGroup(TimelineGroup group, double delta) => setState(() {
        final next = ((_groupWidths[group] ?? 0) + delta)
            .clamp(minGroupWidth(group), 900.0);
        _groupWidths = {..._groupWidths, group: next};
      });

  /// The layer whose fold-out was last touched — drawn a shade dimmer than
  /// the selected layer, so "which layer do these rows belong to" has an
  /// answer at a glance without stealing the selection.
  String? _highlighted;

  /// The selected properties, as fold paths (`<layer>/effects/<fx>/<param>`),
  /// in selection order — clicking a property's name selects it, Ctrl+click
  /// toggles it, Shift+click extends the range, across layers (docs/07 §4.3,
  /// §5). Each is a coloured curve in the graph editor.
  final List<String> _selectedProperties = [];

  /// The graph editor's selected keyframes, as `channelId#index` — owned here
  /// so the bottom bar's buttons and the shortcuts act on the same set.
  final Set<String> _graphKeySelection = {};

  /// The Sequence layers whose view is open (K-248) — double-clicking one
  /// opens its clips and their speed envelope inside its own row.
  final Set<String> _sequenceOpen = {};

  /// How tall each open view's **graph** half is, by layer id, once its
  /// divider has been dragged. Absent means the default three rows.
  ///
  /// Only the graph resizes: the clip strip is where the cutting happens and
  /// it is sized for that, while how much room a speed curve wants depends
  /// entirely on how far the ramps go.
  final Map<String, double> _sequenceGraph = {};

  /// How much taller each open view makes its layer's row.
  Map<String, double> get _sequenceExtra => {
        for (final id in _sequenceOpen)
          // The clips' top row *is* the layer's own bar row, so opening adds
          // only the two under it — which is what keeps the layer's row
          // looking exactly as it did when the view is shut (K-248).
          id: sequenceClipExtra + (_sequenceGraph[id] ?? sequenceEnvelopeStrip),
      };

  /// Where each open sequence view sits down the table, as (top, bottom) in
  /// the rows' own coordinates — what the row seams skip over, so an open view
  /// reads as one cell rather than six ruled rows.
  List<(double, double)> _sequenceBlanks(List<LayerRow> rows) {
    final out = <(double, double)>[];
    var y = 0.0;
    for (final row in rows) {
      final extra = row.sequenceExtra;
      if (extra != null) {
        // **From the top of the layer's own row.** The view takes that row as
        // the first of its three clip rows, so a seam ruled under it would cut
        // the clip region in two — which is the one place the region must read
        // as a single block. The seams that *bound* the whole view are left
        // alone: the range is exclusive at both ends, so the layer still has a
        // line above it and the fold-out below still has one over it.
        out.add((y, y + _rowHeight + extra));
      }
      y += row.height;
    }
    return out;
  }

  /// Open or close a Sequence layer's view. Any other kind is left alone: a
  /// double-click on a Precomp already means "open the comp", and a layer with
  /// no clips has nothing to show.
  void _toggleSequenceView(BridgeLayerEntry entry) {
    if (entry.info.kind != BridgeLayerKind.sequence) return;
    final id = entry.layer.internallayerId.toString();
    setState(() {
      if (!_sequenceOpen.remove(id)) _sequenceOpen.add(id);
    });
  }

  /// Which reading of the curves the graph shows (docs/07 §5.1).
  GraphLens _graphLens = GraphLens.value;

  /// Auto-fit: the graph frames its curves vertically by itself; toggled off,
  /// the wheel pans and `Alt`+wheel zooms the value axis (docs/07 §5.3).
  bool _graphAutoFit = true;

  final GlobalKey<GraphEditorFrbState> _graphPane = GlobalKey();

  /// The property rows currently on screen, in display order — what a
  /// Shift+click range runs along. Rebuilt by every build.
  List<String> _visiblePropertyPaths = const [];

  /// Select [path] by click: plain replaces, Ctrl toggles, Shift extends from
  /// the last selected along the visible rows. Marks its layer either way.
  void _selectProperty(String path) => setState(() {
        final keys = HardwareKeyboard.instance;
        if (keys.isControlPressed || keys.isMetaPressed) {
          if (!_selectedProperties.remove(path)) _selectedProperties.add(path);
        } else if (keys.isShiftPressed && _selectedProperties.isNotEmpty) {
          final a = _visiblePropertyPaths.indexOf(_selectedProperties.last);
          final b = _visiblePropertyPaths.indexOf(path);
          if (a < 0 || b < 0) {
            if (!_selectedProperties.contains(path)) {
              _selectedProperties.add(path);
            }
          } else {
            for (var i = a < b ? a : b; i <= (a < b ? b : a); i++) {
              if (!_selectedProperties.contains(_visiblePropertyPaths[i])) {
                _selectedProperties.add(_visiblePropertyPaths[i]);
              }
            }
          }
        } else {
          _selectedProperties
            ..clear()
            ..add(path);
        }
        _graphKeySelection.clear();
        _highlighted = layerIdOfPath(path) ?? _highlighted;
        _openRetimeInItsDefaultLens(path);
        _publishEffectSelection();
        _publishPropertySelection();
      });

  /// The other direction: an effect picked in the Effect controls panel lights
  /// its row here (K-300). A no-op when the selection is already what this
  /// panel published, which is what keeps the two from bouncing.
  void _onEffectSelectionChanged() {
    if (!mounted) return;
    final ui = _ui!;
    final owner = ui.selectedEffectsLayer?.internallayerId.toString();
    final wanted = owner == null
        ? const <String>[]
        : [for (final id in ui.selectedEffects.value) effectPath(owner, '$id')];
    final held = [
      for (final path in _selectedProperties)
        if (effectIdOfPath(path) != null) path,
    ];
    if (held.length == wanted.length &&
        List.generate(held.length, (i) => held[i] == wanted[i])
            .every((same) => same)) {
      return;
    }
    setState(() {
      _selectedProperties
        ..clear()
        ..addAll(wanted);
      _graphKeySelection.clear();
      if (owner != null) _highlighted = owner;
    });
  }

  /// Hand the effect headings among the selected rows to the shell (K-300), so
  /// Copy — and the Effect controls panel, which lights the same effects —
  /// sees what was picked here.
  ///
  /// Derived from the row selection rather than kept beside it: the Timeline
  /// has one idea of what is selected, and an effect heading is a row in it.
  /// Rows on more than one layer resolve to the first layer with an effect
  /// picked, because a `.lumfx` document is one layer's stack.
  /// Tell the rest of the shell which property rows are picked (K-341), so the
  /// Viewer can outline the layer they belong to and show the points of a mask
  /// whose Path row is among them.
  void _publishPropertySelection() {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    ui.selectedProperties.value =
        List<String>.unmodifiable(_selectedProperties);
  }

  /// The Viewer asking for a row to be picked — a mask path it has just
  /// dragged, whose keyframe moved and whose row should therefore be the one
  /// showing.
  void _onSelectPropertyRequested() {
    if (!mounted) return;
    final path = _ui?.selectPropertyRequest.value;
    if (path == null) return;
    _ui!.selectPropertyRequest.value = null;
    if (_selectedProperties.length == 1 && _selectedProperties.first == path) {
      return;
    }
    setState(() {
      _selectedProperties
        ..clear()
        ..add(path);
      _graphKeySelection.clear();
      _highlighted = layerIdOfPath(path) ?? _highlighted;
    });
    _publishPropertySelection();
  }

  void _publishEffectSelection() {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    String? layerId;
    final picked = <UuidValue>[];
    for (final path in _selectedProperties) {
      final effect = effectIdOfPath(path);
      if (effect == null) continue;
      final owner = layerIdOfPath(path)!;
      layerId ??= owner;
      if (owner == layerId) picked.add(UuidValue.fromString(effect));
    }
    if (layerId == null) {
      ui.clearEffectSelection();
      return;
    }
    for (final entry in _lastLayers) {
      if (entry.layer.internallayerId.toString() != layerId) continue;
      // Stack order, not click order: the same rule the engine's copy follows.
      final stack = [for (final e in entry.info.effects) e.id];
      ui.setEffectSelection(
        entry.layer,
        [
          for (final id in stack)
            if (picked.contains(id)) id
        ],
      );
      return;
    }
    ui.clearEffectSelection();
  }

  /// Opening a **Retime** row lands in the lens the user asked for (K-246):
  /// with *Retime opens to Speed* on, the speed view — which in that mode
  /// is the Vegas envelope (K-247).
  ///
  /// Only on the way *in*, and only for a Retime: switching lens by hand
  /// afterwards must stick, and selecting Position must not drag the Retime
  /// preference onto it. Turn the preference off, reopen the row, and the Time
  /// view is back — which is the point of it being a preference, not a mode.
  void _openRetimeInItsDefaultLens(String path) {
    if (!_vegas(context)) return;
    final owner = layerIdOfPath(path);
    if (owner == null || path != retimePath(owner)) return;
    _graphLens = GraphLens.speed;
    _publishEasingClaim();
  }

  /// Settings ▸ Interface ▸ Editing ▸ *Video arrives as a Sequence layer*
  /// (K-246). Forwarded to the engine, which decides whether this particular
  /// media is something to cut.
  bool _videoAsSequence(BuildContext context) =>
      Provider.of<LumitUiState>(context, listen: false)
          .workspace
          .interface
          .videoAsSequenceLayer;

  /// Settings ▸ Interface ▸ Editing ▸ *Retime opens to Speed* (K-246).
  bool _vegas(BuildContext context) =>
      Provider.of<LumitUiState>(context, listen: false)
          .workspace
          .interface
          .retimeOpensToSpeed;

  /// Editing a value or keying a property selects it too (docs/07 §4.3) —
  /// quietly: an already-selected property stays where it is in the order.
  void _selectOnEdit(String path) {
    if (_selectedProperties.contains(path)) return;
    setState(() {
      _selectedProperties
        ..clear()
        ..add(path);
      _graphKeySelection.clear();
      _highlighted = layerIdOfPath(path) ?? _highlighted;
    });
    _publishPropertySelection();
  }

  /// The graph editor replaces the layer area rather than sitting beside it:
  /// the two want the same width, and a curve squeezed into half a panel is not
  /// a curve you can shape.
  bool _graph = false;

  /// Whether the razor is armed — which is now the *toolbar's* answer (K-220):
  /// the Razor tool (`C`) and this panel's own menu item are two doors into one
  /// state, because two razors that could disagree is one razor too many. The
  /// menu item arms and disarms the tool.
  bool _razorArmed(LumitUiState ui) => ui.tools.tool.group == ToolGroup.razor;

  void _toggleRazor(LumitUiState ui) => _razorArmed(ui)
      ? ui.tools.select(ToolMode.select)
      : ui.tools.select(ToolMode.razor);

  /// The toolbar's state, subscribed to once.
  ///
  /// The armed tool lives on its own notifier beside the rest of the shell's UI
  /// state, so watching `LumitUiState` does not hear about it: without this the
  /// lanes kept whatever the razor was when the panel last drew, and arming it
  /// from the toolbar — or from this panel's own menu — changed nothing until
  /// something else happened to rebuild.
  ToolsState? _boundTools;

  void _onToolChanged() {
    if (mounted) setState(() {});
  }

  void _bindTools(LumitUiState ui) {
    if (identical(_boundTools, ui.tools)) return;
    _boundTools?.removeListener(_onToolChanged);
    _boundTools = ui.tools..addListener(_onToolChanged);
  }

  /// Cut at [frame]: the layer that was clicked, or — with Shift — every layer
  /// that spans that moment (docs/07 §4.4).
  void _razorCutAt(
    LumitUiState ui,
    BridgeLayerEntry? clicked,
    int frame,
    VoidCallback onChanged,
  ) {
    final targets = razorTargets(
      ui.model.layers,
      frame,
      clicked: clicked,
      allLayers: HardwareKeyboard.instance.isShiftPressed,
    );
    if (razorCut(targets, frame)) onChanged();
  }

  /// `Ctrl+Shift+D`: cut every selected layer at the playhead (docs/07 §4.4).
  ///
  /// A command, not a tool — it does not care which tool is armed, and it cuts
  /// where the playhead is rather than where the pointer is. The rules are the
  /// razor's, and they are read from the razor rather than written a second
  /// time: [razorTargets] says what a cut at that frame can land on (strictly
  /// inside the layer), [razorCut] makes it, and a cut the engine refuses is
  /// silence.
  bool _splitSelectionAtPlayhead(LumitUiState ui) {
    final frame = ui.playheadFrame.value;
    final selected = ui.selectedLayerIds;
    final targets = [
      for (final entry in razorTargets(ui.model.layers, frame,
          clicked: null, allLayers: true))
        if (selected.contains(entry.layer.internallayerId)) entry,
    ];
    if (targets.isEmpty) return false;
    if (razorCut(targets, frame)) ui.model.refresh();
    return true;
  }

  /// `[` and `]`: move the selected layers so that end lands on the playhead;
  /// with `Alt`, trim that end to it instead (docs/07 §4.4).
  ///
  /// A move carries the layer's content with it and a trim does not, and
  /// neither may run past the source or turn a bar inside out. Those are the
  /// bar drag's rules, so they are read from the bar drag's own clamp rather
  /// than written a second time here — a key and a drag that disagreed about
  /// where a layer may end would be two different edits wearing one name.
  bool _moveOrTrimSelection(LumitUiState ui, String action) {
    final comp = ui.selectedComp;
    final selected = ui.selectedLayerIds;
    if (comp == null || selected.isEmpty) return false;

    final grab = switch (action) {
      'layer.trim.in' => BarGrab.trimIn,
      'layer.trim.out' => BarGrab.trimOut,
      _ => BarGrab.move,
    };
    final atIn = action == 'layer.move.in' || action == 'layer.trim.in';
    final frame = ui.playheadFrame.value;
    final (fpsNum, fpsDen) = ui.model.fpsExact;

    var changed = false;
    for (final entry in ui.model.layers) {
      if (!selected.contains(entry.layer.internallayerId)) continue;
      final span = entry.info.span;
      final inFrame = frameOfTime(span.inPoint, fpsNum, fpsDen);
      final outFrame = frameOfTime(span.outPoint, fpsNum, fpsDen);
      final delta = clampBarDelta(
        grab: grab,
        delta: frame - (atIn ? inFrame : outFrame),
        inFrame: inFrame,
        outFrame: outFrame,
        bounds: _barBounds[entry.layer.internallayerId.toString()] ??
            BarBounds.free,
      );
      if (delta == 0) continue;
      final newIn = inFrame + (grab == BarGrab.trimOut ? 0 : delta);
      final newOut = outFrame + (grab == BarGrab.trimIn ? 0 : delta);
      if (newOut <= newIn) continue;
      entry.layer.setSpan(
        span: BridgeSpan(
          inPoint: comp.timeOfFrame(frame: newIn),
          outPoint: comp.timeOfFrame(frame: newOut),
          // Moving carries the content with the bar, so time zero travels too.
          startOffset: grab == BarGrab.move
              ? comp.timeOfFrame(
                  frame: frameOfTime(span.startOffset, fpsNum, fpsDen) + delta)
              : span.startOffset,
        ),
      );
      changed = true;
    }
    if (changed) ui.model.refresh();
    return changed;
  }

  /// One reveal key: the selected layers open showing exactly what the key
  /// names, and pressing it again shuts them (docs/07 §4.3).
  ///
  /// AE's `P`, `S`, `R`, `T`, `A`, `E`, `M` and `Shift+L`. `U` is not one of
  /// these — it asks the engine what qualifies and has its own cycle
  /// ([_revealTap]); these know their row up front. `R` on a 3D layer reveals
  /// all three rotation rows, because the engine lists them as three groups.
  bool _reveal(LumitUiState ui, String action) {
    final selected = ui.selectedLayerIds;
    if (selected.isEmpty) return false;
    setState(() {
      for (final entry in ui.model.layers) {
        if (!selected.contains(entry.layer.internallayerId)) continue;
        final id = entry.layer.internallayerId.toString();
        final wanted = _revealPaths(id, entry, action);
        // Already showing this and nothing else: the key is a toggle, so the
        // second press shuts the layer rather than reopening what is open.
        final showing = _open.contains(id) &&
            wanted.every(_open.contains) &&
            !_open.any((p) => isUnderPath(id, p) && !wanted.contains(p));
        // Every reveal starts from the layer closed, so it shows what it says
        // rather than adding to whatever the last one left open.
        _open.removeWhere((p) => p == id || isUnderPath(id, p));
        _dropSelectionUnder(id);
        if (showing) continue;
        _open
          ..add(id)
          ..addAll(wanted);
      }
    });
    return true;
  }

  /// The FX console has planted a key and wants its row visible (K-326): open
  /// the layer and the named row, leaving whatever else is open alone.
  void _onRevealRequested() {
    final request = _ui?.revealPropertyRequest.value;
    if (request == null || !mounted) return;
    _ui!.revealPropertyRequest.value = null;
    final (layerId, action) = request;
    setState(() {
      for (final entry in _ui!.model.layers) {
        if (entry.layer.internallayerId != layerId) continue;
        final id = layerId.toString();
        _open
          ..add(id)
          ..addAll(_revealPaths(id, entry, action));
      }
    });
  }

  /// Which fold paths a reveal key opens under [id]. Empty means the layer's
  /// own row and nothing beneath it — what the Retime chord leaves behind, and
  /// what `E` or `M` come to on a layer with no effects or masks to show.
  List<String> _revealPaths(String id, BridgeLayerEntry entry, String action) {
    final axis = switch (action) {
      'reveal.position' => 'position',
      'reveal.scale' => 'scale',
      'reveal.rotation' => 'rotation',
      'reveal.opacity' => 'opacity',
      'reveal.anchor' => 'anchor',
      _ => null,
    };
    if (axis != null) {
      return [
        for (final group in transformGroups(threeD: entry.info.switches.threeD))
          if (group.axes.first.prop.name.startsWith(axis))
            transformGroupPath(id, group),
      ];
    }
    return switch (action) {
      'reveal.effects' =>
        entry.info.effects.isEmpty ? const [] : [effectsPath(id)],
      'reveal.masks' => entry.info.masks.isEmpty ? const [] : [masksPath(id)],
      'reveal.volume' => [audioPath(id)],
      _ => const [],
    };
  }

  /// The bar drag in flight, if any — a notifier rather than panel state so
  /// only the waveform lanes redraw as the pointer moves, not the whole table.
  final ValueNotifier<BarDragPreview?> _barDrag = ValueNotifier(null);

  /// The lane view's selected keyframes, as `rowId#index` (docs/07 §4.3) —
  /// what the marquee gathered. Session state, like the twirl set.
  final Set<String> _laneKeySelection = {};

  /// The layer drag in flight (K-208), read by both halves of the table. A
  /// notifier rather than panel state: a drag slides rows, and rebuilding the
  /// whole panel per pointer move to do it would cost the table its bridge
  /// budget (docs/13).
  final ValueNotifier<LayerDrag?> _layerDrag = ValueNotifier(null);

  /// The layer `Enter` has asked to rename (K-243). A notifier for the same
  /// reason the drag is one: only the row it names has anything to do, and
  /// rebuilding the whole table to tell it would be the panel's budget spent
  /// on a text field.
  final ValueNotifier<UuidValue?> _renameRequest = ValueNotifier(null);

  /// The outline's and the lanes' vertical scrolls, linked both ways so the
  /// two halves of the table stay one table; the lanes' side owns the visible
  /// scrollbar. In graph view the outline scrolls alone.
  final ScrollController _vOutline = ScrollController();
  final ScrollController _vLane = ScrollController();

  /// The lanes' horizontal scroll, once zoomed past fit.
  ///
  /// Anchored (K-293): a zoom hands it the frame to hold and where to hold it,
  /// and it makes the offset agree with the new width **during layout**, which
  /// is the only moment the two are known together. Jumping it from outside
  /// layout put the offset past the end of content that had not been laid out
  /// yet, and the spring back — with the scrollbar drawn from a position and a
  /// length that disagreed — is the twitching thumb a zoom drag showed.
  final ZoomAnchoredScrollController _hLane = ZoomAnchoredScrollController();

  /// Time zoom: 1 is fit-to-panel, and it **flies** rather than cutting
  /// (docs/07 §4.6). The Viewer's magnification has flown since K-218; this is
  /// the same helper, so the two read as one application rather than two.
  ///
  /// Only the **lane side** rebuilds when it moves: the whole panel used to,
  /// sixty times a second through a flight, which put the outline's every row —
  /// and the work-area and cache reads that come with it — inside the zoom
  /// (K-293). Nothing left of the seam depends on the zoom.
  late final SmoothZoom _zoomMotion;

  /// What the flight is holding still: the frame that was under the pointer (or
  /// the playhead, for the bottom bar's slider) and where on screen it was.
  /// Re-applied on **every tick**, because the content grows all through the
  /// flight — hold the offset still instead and the anchor slides out from
  /// under the cursor, which is the drift the Viewer's own note warns about.
  double _zoomAnchorFrame = 0;
  double _zoomAnchorViewportX = 0;

  double get _zoom => _zoomMotion.value;

  /// How many frames the lanes span — [_laneViewport]'s other half, recorded in
  /// build so the zoom can work out the pixels a frame is worth at any
  /// magnification without reading the axis it is itself rebuilding.
  int _laneFrames = 1;

  /// How much motion the shell is set to show, read in build where the theme
  /// scope is in reach — the same arrangement the Viewer's zoom uses.
  AnimationLevel _animationLevel = AnimationLevel.all;

  double get _perFrameNow => _laneFrames <= 0
      ? 0
      : max(0.0, _laneViewport * _zoomMotion.value - TimelineAxis.pad * 2) /
          _laneFrames;

  /// How many frames full zoom-in shows across the lanes (owner, 2026-08-06).
  ///
  /// A *count of frames*, not a magnification, because that is the thing the
  /// number means to a person: at the right-hand end of the slider you are
  /// looking at twenty frames, whether the composition is five seconds or ten
  /// minutes. The visible span is `frames / zoom` whatever the panel's width,
  /// so the ceiling that gives it is simply `frames / 20`.
  static const int _framesAtFullZoom = 20;

  double get _maxZoom => max(1.0, _laneFrames / _framesAtFullZoom.toDouble());

  /// The body the Project panel drops onto, so a drop can be measured against
  /// it: where in the stack the pointer let go is where the footage lands.
  final GlobalKey _dropArea = GlobalKey();

  /// Whether a dragged keyframe sticks to whole frames (docs/07 §4.5). On by
  /// default: landing between frames is the deliberate exception.
  bool _magnet = true;

  bool _syncingScroll = false;

  @override
  void initState() {
    super.initState();
    _zoomMotion = SmoothZoom(vsync: this, initial: 1, min: 1, max: 64)
      ..addListener(_onZoomTick);
    _vOutline.addListener(() => _followScroll(_vOutline, _vLane));
    _vLane.addListener(() => _followScroll(_vLane, _vOutline));
    // Scrolling sideways moves which stretch of audio the waveform lanes show,
    // and a summary is only as detailed as the window it was taken over
    // (K-280). Nothing else about the panel changes, so this listens rather
    // than the panel rebuilding on every scrolled pixel.
    _hLane.addListener(_onLaneScroll);
    HardwareKeyboard.instance.addHandler(_onKey);
    // Claim Delete for the finer selection this panel holds (K-234). The state
    // is kept, not looked up again: `dispose` runs after the element is
    // deactivated, where an ancestor lookup is no longer safe.
    _ui = Provider.of<LumitUiState>(context, listen: false);
    _ui!.deleteClaim = _deleteSelectedMasks;
    _ui!.copyClaim = _copySelectedKeys;
    _ui!.pasteClaim = _pasteKeysIntoSelection;
    _publishEasingClaim();
    // An effect can be picked in the Effect controls panel too (K-300), and one
    // selection means the row here lights up when it is.
    _ui!.selectedEffects.addListener(_onEffectSelectionChanged);
    // The selection can change from outside this panel — a click on the
    // picture in the Viewer is the everyday case (K-275). The property
    // selection, the graph's key selection and the row highlight all belong to
    // whichever layer was chosen, so they are cleared wherever the choosing
    // happened rather than only in this panel's own click path. Without this,
    // picking a different layer on the picture left the *previous* layer's
    // rows lit in the Timeline: two layers appearing chosen at once, which is
    // the ambiguity K-203 set out to remove.
    _primary = _ui!.selectedLayer.value?.internallayerId;
    _ui!.selectedLayer.addListener(_onPrimaryChanged);
    // Switching measuring off takes the render-time column away entirely, and
    // the outline is that much narrower for it — a layout change, so the panel
    // has to hear about it rather than only the cells inside the column.
    _ui!.renderTimings.addListener(_onTimingsChanged);
    // The FX console's Keyframe ring plants a key and then asks for its row to
    // be on screen (K-326). Ensure-open, not the reveal keys' toggle: showing
    // a row that is already showing must never hide it.
    _ui!.revealPropertyRequest.addListener(_onRevealRequested);
    _ui!.selectPropertyRequest.addListener(_onSelectPropertyRequested);
    // Merged **once**, not per build: a fresh `Listenable` every rebuild makes
    // every cache bar under it unsubscribe and resubscribe, which during a zoom
    // flight is sixty times a second for nothing (K-293).
    _cacheRevision = Listenable.merge([_ui!.frameArrived, _ui!.cacheChanged]);
  }

  /// When the render cache may have changed — a frame arrived, or the cache
  /// was cleared. Held, for the reason [didChangeDependencies] gives.
  Listenable? _cacheRevision;

  void _onTimingsChanged() {
    if (mounted) setState(() {});
  }

  /// The layer the panel's local selections belong to, so a change of primary
  /// can be told from a rebuild.
  UuidValue? _primary;

  void _onPrimaryChanged() {
    final now = _ui?.selectedLayer.value?.internallayerId;
    if (now == _primary) return;
    _primary = now;
    if (!mounted) return;
    setState(_dropLayerLocalSelection);
  }

  /// Everything the panel holds that belongs to one layer. Called whenever the
  /// primary changes, from here or from anywhere else.
  void _dropLayerLocalSelection() {
    _selectedProperties.clear();
    _graphKeySelection.clear();
    _laneKeySelection.clear();
    _highlighted = null;
  }

  /// The shell state this panel claimed Delete on, so the claim can be dropped
  /// again when the panel goes.
  LumitUiState? _ui;

  /// The graph editor's channels right now, resolved from the read model.
  List<GraphChannel> _channelsNow() {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return graphChannels(
        layers: ui.model.layers, selected: _selectedProperties);
  }

  /// The key selection the current view acts on: the graph's own, or the lane
  /// marquee's translated onto channels (a lane diamond stands for every axis
  /// of its row, so `row#i` fans out to each channel of that path).
  Set<String> _actionKeySelection(List<GraphChannel> channels) {
    if (_graph) return _graphKeySelection;
    final out = <String>{};
    for (final id in _laneKeySelection) {
      final hash = id.lastIndexOf('#');
      if (hash <= 0) continue;
      final path = id.substring(0, hash);
      final index = id.substring(hash + 1);
      for (final channel in channels) {
        if (channel.path == path) out.add('${channel.id}#$index');
      }
    }
    return out;
  }

  /// Set the selected keys' easing (the F9 family and the bottom bar's
  /// Linear / Bezier / Hold): both sides, or one for ease-in/ease-out.
  void _applyInterp(BridgeSideInterp side,
      {bool inSide = true, bool outSide = true}) {
    // In lane view the selection speaks in row paths, so the channels have to
    // cover those too, not only the selected properties.
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final paths = _graph
        ? _selectedProperties
        : {
            for (final id in _laneKeySelection)
              if (id.lastIndexOf('#') > 0) id.substring(0, id.lastIndexOf('#'))
          }.toList();
    final channels = graphChannels(layers: ui.model.layers, selected: paths);
    final selection = _actionKeySelection(channels);
    if (selection.isEmpty) return;
    applyInterpToSelection(
      channels: channels,
      selectedKeys: selection,
      side: side,
      inSide: inSide,
      outSide: outSide,
    );
    ui.model.refresh();
  }

  /// Stamp a shaped ease onto the selection — the easing editor's Apply.
  ///
  /// The selection is resolved the same way [_applyInterp] resolves it, so the
  /// bottom bar's one-click eases and a shaped one act on exactly the same
  /// keys. Where they differ is the unit: a side is stamped key by key, a curve
  /// span by span (`applyEasingToSelection`).
  ///
  /// Value lens only, and locked twice: the bottom bar hides the button in the
  /// speed lens, and this refuses the call. A shape is drawn against value
  /// travel, so stamping one from the speed lens would edit a graph the user is
  /// not looking at.
  /// The Easing… button: dock the Easing panel, or — with Settings ▸ Interface
  /// ▸ Editing ▸ *Shape eases in a popup* on — open the same editor over the
  /// footer (K-349).
  ///
  /// Docking rather than only focusing: the button is how the panel is
  /// discovered, and a button that does nothing because the panel is already in
  /// an arrangement the user cannot see is worse than one that opens it twice.
  /// `setPanelVisible` is a no-op when it is already there, so a second press
  /// only brings it to the front of its tab group.
  void _openEasing(BuildContext buttonContext) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    if (ui.workspace.interface.easingInPopup) {
      final box = buttonContext.findRenderObject() as RenderBox?;
      if (box == null) return;
      showEasingPopup(
        context: buttonContext,
        position: box.localToGlobal(Offset.zero),
        onApply: _applyEasing,
      );
      return;
    }
    setPanelVisible(ui.split, Panel.easing, true);
    activatePanelTab(ui.split, Panel.easing);
    ui.activePanel.value = Panel.easing;
    ui.workspace.touch();
  }

  /// What the Easing panel and the popup both press. Published to the shell as
  /// [LumitUiState.easingApply] while this panel can take a shape, so the panel
  /// can grey its Apply when it cannot (K-349).
  ///
  /// The popup is an overlay entry, so it outlives the panel that opened it —
  /// a re-dock while it is up would otherwise land an Apply on a dead State.
  void _applyEasing(EasingCurve curve) {
    if (!mounted) return;
    if (_graph && _graphLens == GraphLens.speed) return;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final paths = _graph
        ? _selectedProperties
        : {
            for (final id in _laneKeySelection)
              if (id.lastIndexOf('#') > 0) id.substring(0, id.lastIndexOf('#'))
          }.toList();
    final channels = graphChannels(layers: ui.model.layers, selected: paths);
    final selection = _actionKeySelection(channels);
    if (selection.isEmpty) return;
    applyEasingToSelection(
      channels: channels,
      selectedKeys: selection,
      curve: curve,
    );
    ui.model.refresh();
  }

  /// Publish — or withdraw — the shell's easing claim.
  ///
  /// Withdrawn in exactly the case [_applyEasing] refuses: the graph showing
  /// the speed lens. Called from the four places `_graph` and `_graphLens`
  /// move, and from [initState] and [dispose]; a notifier write inside a
  /// `setState` callback is fine, one during `build` would not be, which is why
  /// this is not simply read off in the tree.
  void _publishEasingClaim() {
    _ui?.easingApply.value =
        _graph && _graphLens == GraphLens.speed ? null : _applyEasing;
  }

  /// The Timeline's keyboard commands: `Shift+F3` toggles the graph, the F9
  /// family sets easing, `Ctrl+Shift+D` cuts the selection at the playhead,
  /// `F` re-frames the graph, `Ctrl+C`/`Ctrl+V` copy and
  /// paste keyframes, Delete removes the graph's selected keys. Registered on
  /// the hardware keyboard (panels do not hold focus); a focused text field
  /// keeps its keys.
  bool _onKey(KeyEvent event) {
    if (event is! KeyDownEvent || !mounted) return false;
    // A dialogue is up: its keys are its own (K-243). These commands are
    // registered on the hardware keyboard rather than on focus, so without
    // this the Pre-compose dialogue's `Enter` also renamed the layer behind it.
    if (lumitModalOpen) return false;
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return false;
    }
    // What this chord means in the Timeline — or in the graph editor while it
    // is open, which has bindings of its own (K-199). The engine answers;
    // nothing here compares keys at all since K-300 took the last two
    // comparisons out (copy and paste, now bound actions like everything else).
    final ui = Provider.of<LumitUiState>(context, listen: false);
    // This panel is one surface with two views, so a chord bound in either
    // context works in both — the view's own context first, the other as the
    // fallback. It used to fall back one way only (graph → timeline), which is
    // why the F9 family did nothing in lane view: easing is bound in the graph
    // context (docs/07 §15) while keyframes are selectable in both, so F9 over
    // the lanes looked up no action at all.
    final action = ui.keymap.actionFor(
          _graph ? BridgeKeyContext.graph : BridgeKeyContext.timeline,
          event,
        ) ??
        ui.keymap.actionFor(
          _graph ? BridgeKeyContext.timeline : BridgeKeyContext.graph,
          event,
        );

    if (action == 'graph.toggle') {
      setState(() => _graph = !_graph);
      _publishEasingClaim();
      return true;
    }
    if (action == 'reveal.animated') {
      return _revealTap();
    }
    if (action == 'reveal.audio') {
      return _revealAudioTap(ui);
    }
    // Enter renames the selected layer in place (docs/07 §15, K-243): the row
    // it names opens its own editor, which is why this sets a value rather
    // than reaching into a row. Only while this is the focused panel — the
    // Project panel and Effect controls answer the same key for their own
    // selections now (K-321), and two renames on one press is a mess.
    if (action == 'layer.rename') {
      // A different panel is focused: its own rename answers this key. No
      // panel focused yet falls to the Timeline, as it always did.
      final active = ui.activePanel.value;
      if (active != null && active != Panel.timeline) return false;
      final layer = ui.selectedLayer.value;
      if (layer == null) return false;
      _renameRequest.value = layer.internallayerId;
      return true;
    }
    if (action == 'layer.split') {
      return _splitSelectionAtPlayhead(ui);
    }
    if (action == 'layer.move.in' ||
        action == 'layer.move.out' ||
        action == 'layer.trim.in' ||
        action == 'layer.trim.out') {
      return _moveOrTrimSelection(ui, action!);
    }
    // The single-property reveals (docs/07 §4.3). `layer.retime.enable` is the
    // shell's command, not this panel's — it lands here only to *show* the row
    // the shell has just switched on, which is view state and so ours.
    if (action != null &&
        (action.startsWith('reveal.') || action == 'layer.retime.enable')) {
      return _reveal(ui, action);
    }
    if (action == 'graph.ease' ||
        action == 'graph.ease.in' ||
        action == 'graph.ease.out') {
      // Both sides, the way in, or the way out (docs/07 §5.3).
      _applyInterp(
        easyEase,
        inSide: action != 'graph.ease.out',
        outSide: action != 'graph.ease.in',
      );
      return true;
    }
    // Delete with a mask row selected is not handled here: every one of these
    // handlers runs, in registration order, so a `true` from this one would not
    // stop the shell's Delete removing the layer as well. The Timeline claims
    // the key through [LumitUiState.deleteClaim] instead, which the shell asks
    // *before* it deletes anything (K-234). Copy and paste are claimed the same
    // way (K-300): they used to be compared against `Ctrl+C`/`Ctrl+V` here,
    // which was fine while the shell had no copy of its own and became a
    // double action the moment it did.

    if (!_graph) return false;

    if (action == 'graph.fit') {
      _graphPane.currentState?.fitNow();
      return true;
    }
    if (action == 'edit.delete.selection' && _graphKeySelection.isNotEmpty) {
      _graphPane.currentState?.deleteSelectedKeys();
      return true;
    }
    return false;
  }

  /// Delete every selected mask row, returning whether there was one (K-234).
  ///
  /// The shell's Delete calls this before it deletes the selected layers, so a
  /// picked mask row is what the key acts on — the mask sits *on* the selected
  /// layer, and deleting the layer instead is the opposite of what was asked.
  ///
  /// The same call the row's own context menu makes, so there is one way a mask
  /// is deleted. One op per mask, as deleting several layers is one op each.
  /// Copy claims the chord when keyframes are selected (K-300, K-196's copy
  /// under the claim the shell asks) — and when whole property *rows* are, with
  /// no individual keys picked, in which case it copies those rows: every key
  /// of an animated one, the plain value of one with no keyframes at all
  /// (K-301). With neither the chord falls through to the shell, which copies
  /// the picked effects or the layer.
  bool _copySelectedKeys() {
    if (!mounted) return false;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final comp = ui.selectedComp;
    if (comp == null) return false;
    final channels = _channelsNow();
    final selection = _actionKeySelection(channels);
    if (selection.isEmpty) {
      return copyChannels(comp: comp, channels: channels, fps: ui.model.fps);
    }
    copySelectedKeys(
      comp: comp,
      channels: channels,
      selectedKeys: selection,
      fps: ui.model.fps,
    );
    return true;
  }

  /// Paste claims it when there are channels to paste *into* and keyframes to
  /// paste — or when nothing else is on the clipboard at all, which is what
  /// leaves keyframe text copied out of another tool a way in.
  bool _pasteKeysIntoSelection() {
    if (!mounted) return false;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final channels = _channelsNow();
    if (channels.isEmpty) return false;
    if (graphKeyClipboard.isEmpty && !ui.clipboard.isEmpty) return false;
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    pasteKeysAtPlayhead(
      channels: channels,
      playheadFrame: ui.playheadFrame.value,
      fps: ui.model.fps,
      fpsNum: fpsNum,
      fpsDen: fpsDen,
    ).then((pasted) {
      if (pasted && mounted) ui.model.refresh();
    });
    return true;
  }

  bool _deleteSelectedMasks() {
    if (!mounted) return false;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    // Mask paths are `<layer>/masks/<mask>`, so the layer and the mask are both
    // read straight off the selection — no lookup table to keep in step.
    final wanted = <String, Set<String>>{};
    for (final path in _selectedProperties) {
      final layerId = layerIdOfPath(path);
      if (layerId == null) continue;
      if (!isUnderPath(masksPath(layerId), path)) continue;
      (wanted[layerId] ??= {})
          .add(path.substring(masksPath(layerId).length + 1));
    }
    if (wanted.isEmpty) return false;

    var deleted = false;
    for (final entry in ui.model.layers) {
      final ids = wanted[entry.layer.internallayerId.toString()];
      if (ids == null) continue;
      for (final mask in entry.info.masks) {
        if (!ids.contains(mask.id.toString())) continue;
        try {
          entry.layer.deleteMask(id: mask.id);
          deleted = true;
        } catch (_) {
          // Gone between the draw and the press; nothing left to delete.
        }
      }
    }
    if (!deleted) return false;
    // The rows are gone, so the highlight that pointed at them goes too.
    setState(() => _selectedProperties.removeWhere((path) {
          final owner = layerIdOfPath(path);
          return owner != null && isUnderPath(masksPath(owner), path);
        }));
    ui.model.refresh();
    return true;
  }

  /// When the last `U` was pressed, and how many times in a row — the AE reveal
  /// cycle (docs/07 §4.3). Three taps inside the window are three different
  /// commands, so the count is what tells them apart.
  DateTime? _lastReveal;
  int _revealTaps = 0;

  /// How long a second `U` still counts as the same gesture. AE's own window;
  /// long enough to type deliberately, short enough that a `U` a moment later
  /// starts again rather than collapsing what you just opened.
  static const Duration _revealWindow = Duration(milliseconds: 500);

  /// One press of the reveal key: `U` opens what is animated, `UU` what has
  /// been modified, `UUU` shuts the layer again.
  ///
  /// The *counting* is ours, because a multi-tap is a gesture like a
  /// double-click and gestures are the frontend's. Which groups qualify is the
  /// engine's, and it is asked afresh on each tap — the answer depends on the
  /// document, and the document may have changed between taps.
  bool _revealTap() {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    // With nothing selected the reveal is the whole composition's (K-203):
    // "show me what is animated" is a question about the comp as often as
    // about one layer, and refusing to answer it unless something was selected
    // made the commonest use of the key the one it did not serve.
    final selected = ui.selectedLayer.value;
    final layers = selected != null
        ? [selected]
        : [for (final entry in ui.model.layers) entry.layer];
    if (layers.isEmpty) return false;

    final now = DateTime.now();
    final last = _lastReveal;
    _revealTaps = (last != null && now.difference(last) <= _revealWindow)
        ? _revealTaps + 1
        : 1;
    _lastReveal = now;

    setState(() {
      // Every tap starts from the layers closed, so a reveal shows exactly
      // what it says rather than adding to whatever was already open.
      for (final layer in layers) {
        final id = layer.internallayerId.toString();
        _open.removeWhere((path) => path == id || isUnderPath(id, path));
        _dropSelectionUnder(id);
      }
      if (_revealTaps >= 3) {
        // UUU: shut, and the next U starts the cycle over.
        _revealTaps = 0;
        _lastReveal = null;
        return;
      }
      for (final layer in layers) {
        final id = layer.internallayerId.toString();
        final groups = layer.revealGroups(
          kind: _revealTaps == 1
              ? BridgeRevealKind.animated
              : BridgeRevealKind.modified,
        );
        // Nothing qualifies: leave the layer shut rather than opening it onto
        // a list of headings the reveal just said were empty.
        if (!groups.any) continue;
        _open.add(id);
        if (groups.transform) _open.add(transformPath(id));
        if (groups.effects.isNotEmpty) {
          _open.add(effectsPath(id));
          for (final fx in groups.effects) {
            _open.add(effectPath(id, fx));
          }
        }
        if (groups.audio) _open.add(audioPath(id));
        // Retime needs no path of its own: the row sits above Transform on
        // any open layer that has one, and `groups.any` already counts it, so
        // a layer whose only animation is its Retime opens for `U` here.
      }
    });
    return true;
  }

  /// When the last `L` was pressed, and how many times in a row (K-281) — the
  /// same shape as the `U` cycle, and for the same reason: three taps inside
  /// the window are three different commands.
  DateTime? _lastAudioReveal;
  int _audioRevealTaps = 0;

  /// One press of `L` on the selected layers: **L** opens their Audio group,
  /// **LL** opens the waveform lane inside it, **LLL** shuts them again
  /// (docs/07 §4.3).
  ///
  /// A layer with no sound is left alone rather than opened onto an Audio
  /// group it does not have — the same answer `M` gives a layer with no masks.
  /// Whether a layer carries sound is the cached probe the outline already
  /// uses, so this costs no bridge call.
  bool _revealAudioTap(LumitUiState ui) {
    final selected = ui.selectedLayerIds;
    if (selected.isEmpty) return false;
    final now = DateTime.now();
    final last = _lastAudioReveal;
    _audioRevealTaps = (last != null && now.difference(last) <= _revealWindow)
        ? _audioRevealTaps + 1
        : 1;
    _lastAudioReveal = now;

    setState(() {
      for (final entry in ui.model.layers) {
        if (!selected.contains(entry.layer.internallayerId)) continue;
        final id = entry.layer.internallayerId.toString();
        // Every tap starts from the layer closed, so the cycle shows exactly
        // what it says rather than adding to whatever was already open.
        _open.removeWhere((p) => p == id || isUnderPath(id, p));
        _dropSelectionUnder(id);
        if (_audioRevealTaps >= 3) continue;
        if (!(_hasAudio[id] ?? false)) continue;
        _open
          ..add(id)
          ..add(audioPath(id));
        if (_audioRevealTaps >= 2) _open.add(waveformPath(id));
      }
      if (_audioRevealTaps >= 3) {
        // LLL: shut, and the next L starts the cycle over.
        _audioRevealTaps = 0;
        _lastAudioReveal = null;
      }
    });
    return true;
  }

  /// Mirror one side's scroll onto the other, guarded against the echo.
  void _followScroll(ScrollController from, ScrollController to) {
    if (_syncingScroll || !from.hasClients || !to.hasClients) return;
    if ((to.offset - from.offset).abs() < 0.5) return;
    _syncingScroll = true;
    to.jumpTo(from.offset.clamp(0.0, to.position.maxScrollExtent));
    _syncingScroll = false;
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    _ui?.selectedLayer.removeListener(_onPrimaryChanged);
    _ui?.renderTimings.removeListener(_onTimingsChanged);
    _ui?.revealPropertyRequest.removeListener(_onRevealRequested);
    _ui?.selectPropertyRequest.removeListener(_onSelectPropertyRequested);
    if (_ui?.deleteClaim == _deleteSelectedMasks) _ui!.deleteClaim = null;
    if (_ui?.copyClaim == _copySelectedKeys) _ui!.copyClaim = null;
    if (_ui?.pasteClaim == _pasteKeysIntoSelection) _ui!.pasteClaim = null;
    if (_ui?.easingApply.value == _applyEasing) _ui!.easingApply.value = null;
    _ui?.selectedEffects.removeListener(_onEffectSelectionChanged);
    _boundTools?.removeListener(_onToolChanged);
    _zoomMotion.dispose();
    _barDrag.dispose();
    _layerDrag.dispose();
    _renameRequest.dispose();
    _vOutline.dispose();
    _vLane.dispose();
    _hLane.dispose();
    super.dispose();
  }

  /// A modified wheel over the lanes (docs/07 §4.6). Ctrl zooms time about the
  /// pointer — the frame under the cursor stays under it — and Shift scrolls
  /// sideways. A plain wheel is not touched here, so it still reaches the
  /// scrollable and moves the rows.
  void _wheel(PointerScrollEvent event, double contentX, TimelineAxis axis) {
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed) {
      // What to hold still, in the numbers that are true *now*: which frame is
      // under the pointer, and where on screen the pointer is. The flight
      // re-applies these every tick, so the frame under the cursor stays under
      // it for the whole zoom rather than only at its ends.
      _zoomAnchorViewportX = contentX - (_hLane.hasClients ? _hLane.offset : 0);
      _zoomAnchorFrame = axis.frameAtExact(contentX);
      _zoomMotion.nudge(
        event.scrollDelta.dy < 0 ? 1.2 : 1 / 1.2,
        duration: animationDuration(_animationLevel),
      );
      return;
    }
    if (keys.isShiftPressed && _hLane.hasClients) {
      _hLane.jumpTo((_hLane.offset + event.scrollDelta.dy)
          .clamp(0.0, _hLane.position.maxScrollExtent));
    }
  }

  /// Zoom from somewhere other than the pointer — the bottom bar's slider —
  /// holding the **playhead** still (owner, 2026-08-06; K-293).
  ///
  /// A slider has no pointer to zoom about, so something has to be chosen, and
  /// the playhead is what the editor is working at: After Effects zooms its own
  /// timeline about the current-time indicator, and the middle of the scrollbar
  /// — which this held before — is a place nobody was looking at. In view, the
  /// playhead keeps *exactly* the screen position it has, so nothing under the
  /// eye moves; off view, it is brought to the middle, because a zoom that
  /// magnifies about something you cannot see leaves you nowhere.
  ///
  /// [fly] is false while the slider is being **dragged**: the drag is already
  /// the motion, and flying towards a target the finger moves every few
  /// milliseconds meant the lanes trailed the handle by a whole flight and the
  /// flight restarted before it ever arrived. A tap on the track, or the
  /// wheel, is a discrete jump and still flies.
  void _setZoom(double z, {bool fly = true}) {
    // While the slider is being dragged the anchor was chosen once, at the
    // start of the gesture, and holds to the end (K-319). Re-measuring it on
    // every drag update read the scroll offset before layout had corrected it
    // for the zoom just applied — a fresh zoom against a stale offset — and
    // each update re-anchored somewhere slightly wrong, which is what made
    // the lanes ping around under a dragged slider. The measured-once anchor
    // is exact: the flight (and the drag) re-applies the same fixed point
    // every tick, which is the invariant the whole mechanism is built on.
    if (!_zoomAnchorHeld) _anchorOnPlayhead();
    _zoomMotion.goTo(z,
        duration: fly ? animationDuration(_animationLevel) : Duration.zero);
  }

  /// True while a slider drag holds the anchor fixed — see [_setZoom].
  bool _zoomAnchorHeld = false;

  /// The slider's drag began: choose the anchor now, and keep it for the
  /// whole gesture.
  void _zoomDragStart() {
    _anchorOnPlayhead();
    _zoomAnchorHeld = true;
  }

  void _zoomDragEnd() {
    _zoomAnchorHeld = false;
  }

  /// Point the flight's anchor at the playhead — held where it is if it is on
  /// screen, brought to the middle if it is not.
  ///
  /// The per-frame width is derived from the scroll position's own content
  /// extent when it has one — the same numbers `zoomAnchorOffset` applies the
  /// anchor with — so the point measured here is exactly the point the layout
  /// puts back. A width from anywhere else (the build-time viewport cache)
  /// disagrees by a little at every zoom, and the disagreement is a
  /// systematic drift that grows with magnification.
  void _anchorOnPlayhead() {
    final viewport =
        _hLane.hasClients ? _hLane.position.viewportDimension : _laneViewport;
    final offset = _hLane.hasClients ? _hLane.offset : 0.0;
    final perFrame = _hLane.hasClients && _laneFrames > 0
        ? max(
                0.0,
                _hLane.position.viewportDimension +
                    _hLane.position.maxScrollExtent -
                    TimelineAxis.pad * 2) /
            _laneFrames
        : _perFrameNow;
    final playhead = (_ui?.playheadFrame.value ?? 0).toDouble();
    _zoomAnchorFrame = playhead;
    final x = TimelineAxis.pad + playhead * perFrame - offset;
    _zoomAnchorViewportX =
        perFrame > 0 && x >= 0 && x <= viewport ? x : viewport / 2;
  }

  /// Whether a pull-back to the ceiling is already booked, so a run of builds
  /// books one rather than one each.
  bool _pullingBackZoom = false;

  /// Bring a zoom that is past the composition's ceiling back to it, once this
  /// frame has been painted. Called from build, which is why it defers: see the
  /// note at the call site.
  void _pullZoomBackToCeiling() {
    if (_pullingBackZoom || _zoomMotion.target <= _maxZoom) return;
    _pullingBackZoom = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _pullingBackZoom = false;
      if (mounted && _zoomMotion.target > _maxZoom) _setZoom(_maxZoom);
    });
  }

  /// Hand the anchor to the scroll, for the layout that follows this tick.
  ///
  /// Every tick, because the lanes are growing the whole way through the flight
  /// — hold the offset still instead and the anchor slides out from under the
  /// cursor. The scroll applies it while it is being laid out, so the offset and
  /// the width it belongs to are never out of step (see
  /// `widgets/zoom_anchored_scroll.dart`); this used to jump the controller from
  /// here, which is what made the scrollbar's thumb twitch through a zoom.
  ///
  /// **No `setState`.** The lane side listens to [_zoomMotion] itself, so a
  /// tick rebuilds the lanes and nothing else; calling `setState` here rebuilt
  /// the outline's every row, and the bridge reads that come with it, once per
  /// animation frame (K-293).
  void _onZoomTick() {
    if (_laneFrames <= 0) return;
    _hLane.hold(ZoomAnchor(
      frame: _zoomAnchorFrame,
      viewportX: _zoomAnchorViewportX,
      frames: _laneFrames,
      pad: TimelineAxis.pad,
    ));
  }

  /// Which layer index a Project-panel drop landed on. The stack starts below
  /// the pinned toolbar and column header and scrolls under them, so the drop
  /// is measured in stack space; the slot is then read back as an index into
  /// the whole comp, because the rows on screen may be a filtered subset.
  int _dropIndex(LumitUiState ui, List<BridgeLayerEntry> layers,
      List<double> heights, Offset global) {
    final box = _dropArea.currentContext?.findRenderObject();
    if (box is! RenderBox) return 0;
    final y = box.globalToLocal(global).dy -
        (_toolbarHeight + _headerHeight) +
        (_vLane.hasClients ? _vLane.offset : 0);
    final slot = layerDropSlot(heights, y);
    if (slot >= layers.length) return ui.model.layers.length;
    final id = layers[slot].layer.internallayerId;
    final at = ui.model.layers.indexWhere((e) => e.layer.internallayerId == id);
    return at < 0 ? 0 : at;
  }

  /// The gutter down the right of a scrollable half: a block level with that
  /// half's header (After Effects keeps the same reserved corner), then the
  /// scrollbar itself. Reserved whether or not a thumb shows, so the columns
  /// do not shift when the view changes.
  Widget _scrollGutter(
    LumitTheme t, {
    required ScrollController controller,
    required List<Widget> header,
    required bool showThumb,
  }) =>
      SizedBox(
        width: scrollGutterWidth,
        child: Column(
          children: [
            ...header,
            Expanded(
              child: showThumb
                  ? _GutterScrollbar(controller: controller)
                  : const SizedBox.expand(),
            ),
          ],
        ),
      );

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context);
    _bindTools(ui);
    final comp = ui.selectedComp;
    if (comp == null) {
      // Footage dropped with nothing open offers to make the composition it
      // would go in — the same gesture the Project panel's New composition
      // button takes, so "drag a clip in and start" works from either side
      // rather than dead-ending on a placeholder.
      return _EmptyTimelineDrop(state: Provider.of<LumitState>(context));
    }

    // Everything this panel draws comes from the read model (K-184): zero
    // bridge calls per rebuild. The ListenableBuilder repaints the panel when
    // the model refreshes — which happens once per committed change.
    return ListenableBuilder(
      listenable: ui.model,
      builder: (context, _) => _body(context, ui, comp),
    );
  }

  Widget _body(
      BuildContext context, LumitUiState ui, CompositionReference comp) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    // How much motion the shell shows, for the zoom's flight.
    _animationLevel = scope.animationLevel;
    // The columns actually drawn. The render-time column is only there while
    // something is being measured (K-276): switched off it takes no width, no
    // header and no cells — a column of blanks is not a column, and the outline
    // is short of room as it is. Everything downstream — the header, the rows,
    // the fold-out's value and render-time cells, the outline's own width —
    // works from these rather than from the stored order, so the geometry
    // follows in one place.
    final measuring = ui.renderTimings.measuring;
    // A group is drawn unless it is the render-time column with nothing being
    // measured, or its bottom-bar toggle is off (K-448). One test, one place.
    bool drawn(TimelineGroup group) =>
        (measuring || group != TimelineGroup.timings) &&
        !_hiddenGroups.contains(group);
    final groupOrder = [
      for (final group in _groupOrder)
        if (drawn(group)) group
    ];
    final groupWidths = {
      for (final entry in _groupWidths.entries)
        if (drawn(entry.key)) entry.key: entry.value
    };
    final frames = ui.model.durationFrames;
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    final needle = _search.trim().toLowerCase();
    final layers = [
      for (final e in ui.model.layers)
        if ((needle.isEmpty || e.info.name.toLowerCase().contains(needle)) &&
            !(_hideShy && e.info.switches.shy))
          e,
    ];
    _refreshAudio(layers);
    _lastLayers = layers;
    _refreshPeaks(layers);
    // Every layer, not the filtered list: a bar hidden by the search box still
    // has ends, and they must be known the moment it comes back.
    _refreshBounds(ui.model, fpsNum, fpsDen);

    // What each layer is, decided **once for the whole panel** and read by
    // everything below — the outline, the lanes, the drag maths and the row
    // seams alike. It used to be worked out four times over from the same
    // three inputs, once per reader.
    final rows = layerRows(
        layers: layers,
        open: _open,
        hasAudio: _hasAudio,
        hasPicture: _hasPicture,
        sequenceExtra: _sequenceExtra,
        flowParams: _flowParams,
        volumeDb: _volumeDb);

    // The property rows on screen, in display order — what a Shift+click
    // range runs along — and the graph channels the selection resolves to,
    // each with its stroke colour for the outline's labels to match.
    //
    // Headings are in the list too since K-300: an effect's heading is a row
    // that can be picked (and copied), so a Shift+click has to be able to run
    // over one. Waveforms are not a row anything selects.
    _visiblePropertyPaths = [
      for (final layer in rows)
        for (final row in layer.drawnRows)
          if (row is! FoldWaveformRow) foldRowPath(layer.id, row),
    ];
    final channels =
        graphChannels(layers: ui.model.layers, selected: _selectedProperties);
    // The work area, in frames, read once for the whole panel (K-203) — and
    // once per document *revision*, not per rebuild: `workAreaFrames` is two
    // to four bridge calls, and only an edit can change its answer (K-184).
    final revision = ui.model.revision;
    if (_workArea == null || revision != _workRevision || comp != _workComp) {
      _workRevision = revision;
      _workComp = comp;
      _workArea = workAreaFrames(comp);
    }
    final work = _workArea!;
    // The block heights, as a plain list. Still needed even though the rows
    // now carry their own height: a drag measures its travel against the
    // *stack* ([layerDragTarget]), a drop reads a slot out of it
    // ([layerDropSlot]) and [LayerDragSlide] slides one block by the ones it
    // passes — all three want every height, not this row's.
    final blockHeights = [for (final row in rows) row.height];
    final graphColours = <String, List<Color>>{};
    for (final channel in channels) {
      (graphColours[channel.path] ??= [])
          .add(t.curve[channel.colourIndex % t.curve.length]);
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        CompTabsFrb(
          state: Provider.of<LumitState>(context, listen: false),
          uiState: ui,
        ),
        Expanded(
          // Dropping footage from the Project panel adds it as a layer, and
          // dropping a composition nests it as a Precomp layer. The target
          // wraps the whole body — outline and layer area both — because
          // "onto the Timeline" is what the gesture means; asking the user to
          // hit one half of it would be a rule with no reason behind it.
          // `Object` with a filter, because a DragTarget accepts exactly one
          // payload type and this drop honestly takes two.
          child: DragTarget<Object>(
            onWillAcceptWithDetails: (details) =>
                details.data is FootageDragData || details.data is CompDragData,
            onAcceptWithDetails: (details) {
              // Where the pointer let go, not the top of the stack: dropping
              // between two layers means "here", and a stack that ignores it
              // has to be re-sorted by hand after every drop.
              final at = _dropIndex(ui, layers, blockHeights, details.offset);
              switch (details.data) {
                case FootageDragData(:final footage):
                  // Bottom-up, so a multi-item drop stacks in the order the
                  // panel listed them: each lands at the top of the stack.
                  for (final f in footage.reversed) {
                    comp.addFootageLayer(
                        footage: f, asSequence: _videoAsSequence(context));
                  }
                  ui.model.refresh();
                  // They went on at the top; walk them down to the drop, the
                  // bottom-most first so each one's slot is free when it moves.
                  // ponytail: one undo step per layer — a drop-at-index on the
                  // engine side would make it one, if that ever grates.
                  final fresh = [
                    for (var i = 0; i < footage.length; i++)
                      ui.model.layers[i].layer,
                  ];
                  for (var i = fresh.length - 1; i >= 0; i--) {
                    fresh[i].reorder(newIndex: BigInt.from(at + i));
                  }
                case CompDragData(comp: final dropped):
                  // A comp cannot nest into itself; the engine refuses and
                  // the drop simply does nothing.
                  try {
                    comp.addPrecompLayer(comp: dropped).reorder(
                          newIndex: BigInt.from(at),
                        );
                  } catch (_) {}
              }
              ui.model.refresh();
            },
            builder: (context, candidate, _) => Container(
              key: _dropArea,
              // A live outline while something is over it, so the drop is
              // visibly going to land rather than being taken on faith.
              foregroundDecoration: candidate.isEmpty
                  ? null
                  : BoxDecoration(
                      border: Border.all(
                          color: ThemeScope.of(context).theme.accent, width: 2),
                    ),
              child: LayoutBuilder(
                builder: (context, constraints) {
                  // A panel narrower than the outline's columns shows a
                  // horizontally-scrolling slice of them rather than the
                  // overflow stripe — the same answer the Timeline toolbar
                  // gives — keeping the lanes at least a working sliver.
                  // The outline is as wide as its groups make it, and counts
                  // its own scroll gutter so the columns keep their places
                  // when the view changes.
                  final outlineWidth = outlineWidthOf(groupWidths);
                  final outlineViewport = (constraints.maxWidth - 120)
                      .clamp(120.0, outlineWidth + scrollGutterWidth);
                  // The axis spans the lane viewport times the zoom: at 1 the
                  // whole comp fits the panel (the Viewer's fit-to-panel
                  // habit); zoomed in, the lanes scroll under the bottom
                  // bar's scrollbar.
                  final laneViewport = (constraints.maxWidth -
                          outlineViewport -
                          scrollGutterWidth)
                      .clamp(1.0, 1e6);
                  _laneFrames = frames;
                  // A different comp is a different ceiling; a zoom already
                  // past the new one is pulled back to it rather than left
                  // showing fewer frames than the slider's end promises.
                  //
                  // **After this frame, not during it.** The pull-back is a
                  // zoom like any other, and a zoom notifies its listeners —
                  // which with motion turned off in Settings happens the
                  // instant it is asked for, i.e. inside this build, which is
                  // `setState` during build and an outright crash.
                  _zoomMotion.max = _maxZoom;
                  _pullZoomBackToCeiling();
                  // How wide the lanes are is how many buckets a waveform wants
                  // (K-280). Measured here because this is where it is known;
                  // acted on after the frame, since a build must not start one.
                  //
                  // The axis and the work area's pixels are *not* worked out
                  // here any more: they belong to the zoom, and the zoom only
                  // rebuilds the lane side (K-293), so they are worked out
                  // inside that half's builder below.
                  if (_laneViewport != laneViewport) {
                    _laneViewport = laneViewport;
                    WidgetsBinding.instance.addPostFrameCallback((_) {
                      if (mounted) _refreshPeaks(_lastLayers);
                    });
                  }

                  // **Not** wrapped in a playhead listener. Every layer row and
                  // every bar used to rebuild each time the playhead moved —
                  // sixty times a second during playback, growing with the layer
                  // count, and asking the engine for each layer's name and span
                  // again every time. Only two things actually care where the
                  // playhead is: the line itself, and the razor (which reads it
                  // when clicked). Both listen for themselves now.
                  //
                  // Dragging never scrolls the timeline — the wheel, the
                  // trackpad and the scrollbars do (docs/07 §4.6). A drag on
                  // empty lane space is the keyframe marquee, and a scrollable
                  // competing for it in the gesture arena would win and eat the
                  // box.
                  //
                  // **The trackpad is the exception, and it has to be**: a
                  // two-finger scroll on a Mac arrives as a pan *gesture*, not
                  // as the wheel's pointer signal, so an empty `dragDevices`
                  // set — which is what this was — left the panel unscrollable
                  // by trackpad while the wheel worked perfectly. Nobody with a
                  // mouse could see it. Allowing exactly `trackpad` here scrolls
                  // on two fingers and still leaves a click-drag to the marquee
                  // (a click-drag is a pointer drag, not a pan-zoom); the
                  // editing recognisers over these surfaces exclude the
                  // trackpad in turn, so they cannot take it back
                  // (`dragDevices` in widgets/controls.dart).
                  return ScrollConfiguration(
                    behavior: ScrollConfiguration.of(context).copyWith(
                        dragDevices: const {PointerDeviceKind.trackpad},
                        scrollbars: false),
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        _outlineHalf(context, ui, comp,
                            rows: rows,
                            layers: layers,
                            blockHeights: blockHeights,
                            groupOrder: groupOrder,
                            groupWidths: groupWidths,
                            graphColours: graphColours,
                            outlineViewport: outlineViewport,
                            outlineWidth: outlineWidth),
                        Expanded(
                          // **Only this half rebuilds when the zoom moves**
                          // (K-293). The zoom is a `Listenable`, and the lane
                          // side listens to it here rather than the panel
                          // calling `setState`: nothing left of the seam — the
                          // toolbar, the column header, every outline row, and
                          // the work-area and fold reads that come with them —
                          // depends on the zoom, and rebuilding all of it once
                          // per animation frame is what made a dragged zoom
                          // slider crawl.
                          child: ListenableBuilder(
                            listenable: _zoomMotion,
                            builder: (context, _) {
                              // The axis spans the lane viewport times the
                              // zoom: at 1 the whole comp fits the panel (the
                              // Viewer's fit-to-panel habit); zoomed in, the
                              // lanes scroll under the bottom bar's scrollbar.
                              final axis = TimelineAxis(
                                  frames: frames, width: laneViewport * _zoom);
                              // Where the work area falls, read once and handed
                              // to the ruler, the lanes and the curves alike
                              // (K-203) — and null pixels when it covers the
                              // whole comp, which is when there is no
                              // out-of-range ground to wash.
                              final graphWork = work.whole
                                  ? null
                                  : (axis.xOf(work.start), axis.xOf(work.end));
                              return _graph
                                  ? _graphHalf(context, ui, comp,
                                      axis: axis,
                                      channels: channels,
                                      work: work,
                                      graphWork: graphWork,
                                      frames: frames,
                                      fpsNum: fpsNum,
                                      fpsDen: fpsDen)
                                  : _laneHalf(context, ui, comp,
                                      axis: axis,
                                      rows: rows,
                                      layers: layers,
                                      blockHeights: blockHeights,
                                      work: work,
                                      frames: frames,
                                      fpsNum: fpsNum,
                                      fpsDen: fpsDen);
                            },
                          ),
                        ),
                      ],
                    ),
                  );
                },
              ),
            ),
          ),
        ),
      ],
    );
  }

  /// The outline half of the table: the toolbar, the column header, the rows
  /// and their gutter — everything left of the seam, which the zoom never
  /// rebuilds (K-293).
  Widget _outlineHalf(
    BuildContext context,
    LumitUiState ui,
    CompositionReference comp, {
    required List<LayerRow> rows,
    required List<BridgeLayerEntry> layers,
    required List<double> blockHeights,
    required List<TimelineGroup> groupOrder,
    required Map<TimelineGroup, double> groupWidths,
    required Map<String, List<Color>> graphColours,
    required double outlineViewport,
    required double outlineWidth,
  }) {
    final t = ThemeScope.of(context).theme;
    return SizedBox(
      width: outlineViewport,
      // A column, to match the lane side's: rows, then a
      // block the height of the lane bottom bar, so both
      // halves give their rows the same viewport and scroll
      // the same distance ([_laneBottomBarHeight]).
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Expanded(
              child: Stack(
            children: [
              Row(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Expanded(
                    child: SingleChildScrollView(
                      scrollDirection: Axis.horizontal,
                      child: SizedBox(
                        width: outlineWidth,
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            // The toolbar and the column header live in
                            // the outline, not across the panel: the lane
                            // side gives their height to a taller, easier
                            // to grab time ruler (docs/07 §4.1).
                            _Toolbar(
                              comp: comp,
                              model: ui.model,
                              playhead: ui.playheadFrame,
                              onSeek: ui.scrubTo,
                              graph: _graph,
                              onToggleGraph: () => setState(() {
                                _graph = !_graph;
                                _publishEasingClaim();
                              }),
                              razor: _razorArmed(ui),
                              onToggleRazor: () => _toggleRazor(ui),
                              hideShy: _hideShy,
                              onToggleHideShy: () =>
                                  setState(() => _hideShy = !_hideShy),
                              onSearch: (v) => setState(() => _search = v),
                              onChanged: ui.model.refresh,
                            ),
                            _ColumnHeader(
                              order: groupOrder,
                              widths: groupWidths,
                              onResize: _resizeGroup,
                              onReorder: (dragged, target) => setState(
                                () => _groupOrder = reorderedGroups(
                                    _groupOrder, dragged, target),
                              ),
                            ),
                            // The rows scroll under the pinned toolbar
                            // and header, in step with the lanes.
                            Expanded(
                              // A click that misses every row
                              // deselects (K-203). Translucent
                              // and outermost, so a name, a
                              // switch or a property still
                              // wins its own tap in the arena
                              // and only the empty ground
                              // below the last layer reaches
                              // here.
                              child: GestureDetector(
                                key: const ValueKey('tl-outline-ground'),
                                behavior: HitTestBehavior.translucent,
                                onTap: () => _deselectAll(ui),
                                child: SingleChildScrollView(
                                  controller: _vOutline,
                                  child: _Outline(
                                    comp: comp,
                                    rows: rows,
                                    onOpenSequence: _toggleSequenceView,
                                    layerDrag: _layerDrag,
                                    renameRequest: _renameRequest,
                                    blockHeights: blockHeights,
                                    groupOrder: groupOrder,
                                    widths: groupWidths,
                                    selectedIds: ui.selectedLayerIds,
                                    highlighted: _highlighted,
                                    selectedProperties: _selectedProperties,
                                    graphColours: graphColours,
                                    onSelectProperty: _selectProperty,
                                    onEditProperty: _selectOnEdit,
                                    onToggle: _toggle,
                                    playheadFrame: ui.playheadFrame.value,
                                    onSeek: ui.scrubTo,
                                    onSelect: (l) =>
                                        _selectLayer(ui, l, among: layers),
                                    onHighlight: (id) =>
                                        setState(() => _highlighted = id),
                                    onChanged: ui.model.refresh,
                                  ),
                                ),
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                  // The outline's own gutter: a fixed block level
                  // with the toolbar and column header, then its
                  // thumb — which only shows in graph view, where
                  // the two halves scroll apart.
                  _scrollGutter(
                    t,
                    controller: _vOutline,
                    showThumb: _graph,
                    header: [
                      Container(height: _toolbarHeight, color: t.surface1),
                      Container(height: _headerHeight, color: t.surface2),
                    ],
                  ),
                ],
              ),
              // The row seams, over the columns *and* the
              // gutter so they meet the lane area's (K-192);
              // phased by the scroll so they travel with the
              // rows they separate.
              Positioned(
                top: _toolbarHeight + _headerHeight,
                left: 0,
                right: 0,
                bottom: 0,
                child: IgnorePointer(
                  child: AnimatedBuilder(
                    animation: _vOutline,
                    builder: (context, _) => CustomPaint(
                      painter: _RowDividerPainter(
                        step: _rowHeight,
                        colour: t.hairline,
                        phase: -((_positionOf(_vOutline)?.pixels ?? 0) %
                            _rowHeight),
                        // The grid here repeats from the
                        // panel's edge, so the blanks are
                        // carried up by however far the rows
                        // have scrolled.
                        blanks: [
                          for (final b in _sequenceBlanks(rows))
                            (
                              b.$1 - (_positionOf(_vOutline)?.pixels ?? 0),
                              b.$2 - (_positionOf(_vOutline)?.pixels ?? 0),
                            ),
                        ],
                      ),
                    ),
                  ),
                ),
              ),
            ],
          )),
          // The outline's own end of the bottom bar: the column-group
          // toggles, where the lane side carries the zoom and the scrollbar
          // (K-448). The block was already reserved to keep the two halves the
          // same height — it now has something in it.
          _ColumnToggles(
            groups: _toggleableGroups,
            hidden: _hiddenGroups,
            onToggle: (group) => setState(() {
              if (!_hiddenGroups.remove(group)) _hiddenGroups.add(group);
            }),
          ),
        ],
      ),
    );
  }

  /// The graph editor's half: the same ruler, zoom and horizontal scroll as
  /// the lane view, over one full-height pane of curves (docs/07 §5).
  Widget _graphHalf(
    BuildContext context,
    LumitUiState ui,
    CompositionReference comp, {
    required TimelineAxis axis,
    required List<GraphChannel> channels,
    required ({int start, int end, bool whole}) work,
    required (double, double)? graphWork,
    required int frames,
    required int fpsNum,
    required int fpsDen,
  }) {
    final t = ThemeScope.of(context).theme;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Expanded(
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  controller: _hLane,
                  child: SizedBox(
                    width: axis.width,
                    child: Stack(
                      children: [
                        Column(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            TimelineRuler(
                              comp: comp,
                              axis: axis,
                              fps: ui.model.fps,
                              height: _rulerHeight,
                              work: work,
                              onWorkArea: (span) {
                                comp.setWorkArea(span: span);
                                setState(() {});
                              },
                              onMarkersChanged: () => setState(() {}),
                              onSeek: (f) => ui.scrubTo(
                                  f.clamp(0, frames == 0 ? 0 : frames - 1)),
                            ),
                            CacheStrip(
                              comp: comp,
                              axis: axis,
                              revision: _cacheRevision!,
                              work: graphWork,
                            ),
                            Expanded(
                              child: Stack(
                                children: [
                                  // The same two-shade
                                  // ground the lanes
                                  // get (K-203): the
                                  // work area runs the
                                  // full height of
                                  // whichever view is
                                  // open, so the span
                                  // being delivered is
                                  // never only a mark
                                  // on the ruler.
                                  Positioned.fill(
                                    child: IgnorePointer(
                                      child: CustomPaint(
                                        painter: WorkAreaGroundPainter(
                                          startX: graphWork?.$1,
                                          endX: graphWork?.$2,
                                          // The same band the ruler hangs and
                                          // the lanes carry (§12A.2: nothing
                                          // about the work area changes on a
                                          // mode switch).
                                          inside: Color.alphaBlend(
                                              t.animated.withValues(
                                                  alpha: workAreaLaneFillAlpha),
                                              t.surface1),
                                          outside: t.timelineOutOfRange,
                                          edge: workAreaEdgeColour(t),
                                        ),
                                      ),
                                    ),
                                  ),
                                  GraphEditorFrb(
                                    key: _graphPane,
                                    comp: comp,
                                    hScroll: _hLane,
                                    channels: channels,
                                    axis: axis,
                                    frames: frames,
                                    fps: ui.model.fps,
                                    fpsNum: fpsNum,
                                    fpsDen: fpsDen,
                                    magnet: _magnet,
                                    lens: _graphLens,
                                    autoFit: _graphAutoFit,
                                    vegas: _vegas(context),
                                    penArmed:
                                        ui.tools.tool.group == ToolGroup.pen,
                                    selectedKeys: _graphKeySelection,
                                    onSelectionChanged: () => setState(() {}),
                                    onChanged: ui.model.refresh,
                                    onWheelTime: (e, x) => _wheel(e, x, axis),
                                  ),
                                ],
                              ),
                            ),
                          ],
                        ),
                        // The playhead, over the
                        // ruler and curves alike.
                        ValueListenableBuilder<int>(
                          valueListenable: ui.playheadFrame,
                          builder: (context, frame, child) => Positioned(
                            left: axis.xOf(frame) - PlayheadMarker.halfWidth,
                            top: 0,
                            bottom: 0,
                            child: child!,
                          ),
                          child: const PlayheadMarker(),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
              // The pane frames itself vertically
              // (or the wheel does); the gutter
              // block keeps the columns level
              // with the lane view's.
              _scrollGutter(
                t,
                controller: _vLane,
                showThumb: false,
                header: [
                  Container(
                    height: _rulerHeight + TimelineCacheBar.height,
                    color: t.surface2,
                  ),
                ],
              ),
            ],
          ),
        ),
        _LaneBottomBar(
          zoom: _zoomMotion.target,
          hScroll: _hLane,
          magnet: _magnet,
          onToggleMagnet: () => setState(() => _magnet = !_magnet),
          onZoom: _setZoom,
          onZoomLive: (z) => _setZoom(z, fly: false),
          onZoomDragStart: _zoomDragStart,
          onZoomDragEnd: _zoomDragEnd,
          maxZoom: _maxZoom,
          lens: _graphLens,
          onLens: (lens) => setState(() {
            _graphLens = lens;
            _publishEasingClaim();
          }),
          autoFit: _graphAutoFit,
          onToggleAutoFit: () => setState(() => _graphAutoFit = !_graphAutoFit),
          onInterp: (side) => _applyInterp(side),
          onOpenEasing: _openEasing,
        ),
      ],
    );
  }

  /// The lane half: the ruler, the cache bar, one bar per layer and the
  /// bottom bar.
  Widget _laneHalf(
    BuildContext context,
    LumitUiState ui,
    CompositionReference comp, {
    required TimelineAxis axis,
    required List<LayerRow> rows,
    required List<BridgeLayerEntry> layers,
    required List<double> blockHeights,
    required ({int start, int end, bool whole}) work,
    required int frames,
    required int fpsNum,
    required int fpsDen,
  }) {
    final t = ThemeScope.of(context).theme;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Expanded(
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  controller: _hLane,
                  child: SizedBox(
                    width: axis.width,
                    child: _LayerArea(
                      comp: comp,
                      rows: rows,
                      selectedIds: ui.selectedLayerIds,
                      layerDrag: _layerDrag,
                      blockHeights: blockHeights,
                      onOpenSequence: _toggleSequenceView,
                      onGraphHeight: (entry, h) => setState(() =>
                          _sequenceGraph[
                              entry.layer.internallayerId.toString()] = h),
                      sequenceBlanks: _sequenceBlanks(rows),
                      hScroll: _hLane,
                      onClipPreview: (entry, clip, keys) =>
                          comp.renderFrameWithClipRetime(
                        frame: BigInt.from(ui.playheadFrame.value),
                        scale: 1.0,
                        layer: entry.layer,
                        clip: clip.id,
                        retime: BridgeScalar.keyframed(keys),
                      ),
                      peaks: _peaks,
                      waveformStyle: _waveformStyle,
                      fps: ui.model.fps,
                      fpsNum: fpsNum,
                      fpsDen: fpsDen,
                      magnet: _magnet,
                      axis: axis,
                      playhead: ui.playheadFrame,
                      razor: _razorArmed(ui),
                      onRazor: (entry, frame) =>
                          _razorCutAt(ui, entry, frame, ui.model.refresh),
                      vScroll: _vLane,
                      selectedKeys: _laneKeySelection,
                      onDeselectAll: () => _deselectAll(ui),
                      work: work,
                      onKeysSelected: (keys) {
                        // Picking keyframes picks
                        // their properties too —
                        // every distinct one the
                        // box caught — so the
                        // outline and the graph
                        // show what was boxed
                        // (docs/07 §4.3).
                        setState(() {
                          _laneKeySelection
                            ..clear()
                            ..addAll(keys);
                          if (keys.isEmpty) {
                            return;
                          }
                          _selectedProperties.clear();
                          for (final id in keys) {
                            final path = id.substring(0, id.lastIndexOf('#'));
                            if (!_selectedProperties.contains(path)) {
                              _selectedProperties.add(path);
                            }
                          }
                          _highlighted =
                              layerIdOfPath(_selectedProperties.first) ??
                                  _highlighted;
                        });
                      },
                      onWheel: (e, x) => _wheel(e, x, axis),
                      onSeek: (f) =>
                          ui.scrubTo(f.clamp(0, frames == 0 ? 0 : frames - 1)),
                      onSelect: (l) => _selectLayer(ui, l, among: layers),
                      onChanged: ui.model.refresh,
                      cacheRevision: _cacheRevision!,
                      dragPreview: _barDrag,
                      bounds: _barBounds,
                    ),
                  ),
                ),
              ),
              // The lanes' thumb, pinned to the
              // viewport's right edge rather than
              // riding the scrolled content.
              _scrollGutter(
                t,
                controller: _vLane,
                showThumb: true,
                header: [
                  Container(
                    height: _rulerHeight + TimelineCacheBar.height,
                    color: t.surface2,
                  ),
                ],
              ),
            ],
          ),
        ),
        _LaneBottomBar(
          zoom: _zoomMotion.target,
          hScroll: _hLane,
          magnet: _magnet,
          onToggleMagnet: () => setState(() => _magnet = !_magnet),
          onZoom: _setZoom,
          onZoomLive: (z) => _setZoom(z, fly: false),
          onZoomDragStart: _zoomDragStart,
          onZoomDragEnd: _zoomDragEnd,
          maxZoom: _maxZoom,
        ),
      ],
    );
  }
}

/// The Timeline with no composition open: the placeholder, and a drop target
/// over it.
///
/// Dropping footage here asks for the new comp's settings — opened on the
/// media's own size, rate and length — and each dropped item lands in it as a
/// layer; dropping a composition simply opens that one. Without this the
/// panel was a dead end: the drag lifted, showed its feedback, and dropped
/// into nothing.
class _EmptyTimelineDrop extends StatelessWidget {
  final LumitState state;
  const _EmptyTimelineDrop({required this.state});

  @override
  Widget build(BuildContext context) {
    return DragTarget<Object>(
      onWillAcceptWithDetails: (details) =>
          details.data is FootageDragData || details.data is CompDragData,
      onAcceptWithDetails: (details) async {
        switch (details.data) {
          case FootageDragData(:final footage):
            final comp = await state.newComposition(context, footage: footage);
            if (comp == null || !context.mounted) return;
            Provider.of<LumitUiState>(context, listen: false)
                .setSelectedComp(comp);
          case CompDragData(comp: final dropped):
            Provider.of<LumitUiState>(context, listen: false)
                .setSelectedComp(dropped);
        }
      },
      builder: (context, candidate, _) => Container(
        foregroundDecoration: candidate.isEmpty
            ? null
            : BoxDecoration(
                border: Border.all(
                    color: ThemeScope.of(context).theme.accent, width: 2),
              ),
        child: PlaceholderPanel(
          icon: LumitIcon.comp,
          title: l10n.panelTimeline,
          hint: l10n.timelineEmpty,
        ),
      ),
    );
  }
}

/// One row of a layer's fold-out, in the outline.
///
/// A heading draws its own twirl; a property row draws the same controls the
/// Effect controls panel does, at exactly one lane's height so the two halves of
/// the table stay in step.
class _FoldRow extends StatelessWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final LayerFoldRow row;

  /// Where the value cells go, so they line up under the render-switch group
  /// whatever order the groups are dragged into (docs/07 §4.3).
  final ValueColumn valueColumn;

  /// Where the render-time readout goes, so an effect's measured cost sits
  /// under the same header its layer's does (docs/13 §7.1).
  final ValueColumn timingsColumn;

  /// Where the identity group starts in the current order — the fold-out
  /// hangs off the layer's own twirl, so a group's twirl sits just inside it
  /// rather than at the row's far left.
  final double baseIndent;

  /// This row's path, and the selected properties' — the row draws itself
  /// selected when it is among them, and highlighted when a selection sits
  /// *under* it (an effect's heading while one of its parameters is picked).
  final String path;
  final List<String> selectedProperties;

  /// Each selected path's graph line colours, one per axis — the label text
  /// takes them so the outline names its curves (docs/07 §5).
  final Map<String, List<Color>> graphColours;
  final ValueChanged<String> onSelectProperty;

  /// Editing a value (or keying) selects the property too, without the
  /// click-gesture modifiers.
  final ValueChanged<String> onEditProperty;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final ValueChanged<String> onToggle;
  final VoidCallback onChanged;

  /// Whether the layer this row belongs to is locked (K-291). A locked layer's
  /// rows are still *read* — the numbers are what the document holds and the
  /// curves still draw — but nothing on them can be touched.
  final bool locked;

  const _FoldRow({
    required this.comp,
    required this.layer,
    required this.row,
    required this.valueColumn,
    required this.timingsColumn,
    required this.baseIndent,
    required this.path,
    required this.selectedProperties,
    required this.graphColours,
    required this.onSelectProperty,
    required this.onEditProperty,
    required this.playheadFrame,
    required this.onSeek,
    required this.onToggle,
    required this.onChanged,
    required this.locked,
  });

  @override
  Widget build(BuildContext context) {
    // Just inside the layer's twirl, then one step per level, so a parameter
    // sits under its effect and an effect under Effects.
    final indent = baseIndent + 8.0 + (row.depth - 1) * 12.0;

    // No per-row change listener: the whole panel repaints from the read model
    // when anything commits (K-184), so the numbers shown are the document's.
    final t = ThemeScope.of(context).theme;
    final selected = selectedProperties.contains(path);
    final contains =
        !selected && selectedProperties.any((p) => isUnderPath(path, p));
    // Selection rides on the property's *name* (docs/07 §4.3) — and on any
    // press that *acts* on the row (K-334): the stopwatch, the ◄ ◆ ►
    // navigator, a value drag. Touching a row's controls IS choosing it, and
    // before this a value drag on an unselected row moved a curve the graph
    // was not even showing. Pointer-down rather than tap, so the selection —
    // and with it the graph channel — exists before the first drag tick. A
    // modified press is left to the label's own Ctrl/Shift semantics, and a
    // group heading keeps its pick-and-twirl click (K-300).
    final picks = row is! FoldGroupRow && row is! FoldWaveformRow;
    // **And the row must WIN that press, not merely see it** (K-343). The
    // ground under the outline clears the selection on tap, and its comment
    // has always said "a switch or a property still wins its own tap in the
    // arena" — which was true only of rows carrying a gesture recogniser. A
    // `Listener` is not one: it watches pointers and never competes. So a mask
    // row lit up on the press and went out again on the release, when the
    // ground took the tap nothing had claimed. This claims it, for every
    // picking row, which is what makes them all behave alike.
    //
    // Empty `onTap`, because the selecting is done on pointer-down above:
    // being in the arena at all is the whole job. The row's own controls sit
    // inside and win their taps ahead of it.
    final row_ = Listener(
      // **The whole row takes the press, not just the parts with a widget in
      // them** (K-343). A `Listener` defers to its children by default, and a
      // property row is mostly empty space — so a click beside the label never
      // reached this at all, fell through to the outline behind, and *cleared*
      // the selection instead of making one. Worst on a mask's Path row, which
      // has no value field and so is almost all empty. A heading keeps
      // defer-to-child: its own detector owns the click (K-300).
      behavior: picks ? HitTestBehavior.opaque : HitTestBehavior.deferToChild,
      onPointerDown: !picks
          ? null
          : (_) {
              final keys = HardwareKeyboard.instance;
              if (keys.isControlPressed ||
                  keys.isMetaPressed ||
                  keys.isShiftPressed) {
                return;
              }
              onEditProperty(path);
            },
      child: Container(
        height: _rowHeight,
        // Selected is the full surface; a row that merely *contains* the
        // selection — the effect heading over a picked parameter — is the
        // same at half strength, exactly as a layer row marks itself.
        decoration: BoxDecoration(
          color: selected
              ? t.selectionFill
              : contains
                  ? t.selectionFill.withValues(alpha: 0.45)
                  : null,
        ),
        padding: EdgeInsets.only(left: indent, right: 4),
        // A locked layer's rows are read-only, not hidden (K-291): the numbers
        // are still the document's and the curves still draw, but nothing on the
        // row can be touched. The engine refuses the edit anyway — this is what
        // stops the interface offering a gesture that would only be refused.
        //
        // A *group* row is exempt: twirling one open is navigation, not editing,
        // and a locked layer that could not be looked inside would be worse than
        // one that can.
        child: locked && row is! FoldGroupRow && row is! FoldWaveformRow
            ? AbsorbPointer(
                child: Opacity(opacity: 0.5, child: _control(context)),
              )
            : _control(context),
      ),
    );
    return picks
        ? GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () {},
            child: row_,
          )
        : row_;
  }

  /// Copy the effect this heading names (K-275) — or, when it is one of
  /// several picked, all of them (K-300). The Timeline's half of the pair, the
  /// Effect controls panel's heading carrying the other.
  void _effectMenu(BuildContext context, Offset at, String effectId) {
    showMenuAt<void>(
      context: context,
      position: at,
      width: 190,
      rows: (close) => [
        MenuRow(
          key: ValueKey<String>('tl-fx-menu-copy-$effectId'),
          onPressed: () {
            close(null);
            final ui = Provider.of<LumitUiState>(context, listen: false);
            try {
              ui.copyEffectsToClipboard(layer.copyEffects(
                effects:
                    ui.effectsToCopy(layer, UuidValue.fromString(effectId)),
              ));
            } catch (_) {
              // The effect went away between the menu opening and this row
              // being chosen; the clipboard keeps whatever it had.
            }
          },
          child: Text(l10n.copyEffect),
        ),
      ],
    );
  }

  Widget _control(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return switch (row) {
      FoldWaveformRow() => const SizedBox.shrink(),
      FoldGroupRow(:final path, :final label, :final open) => GestureDetector(
          key: ValueKey<String>('tl-group-$path'),
          behavior: HitTestBehavior.opaque,
          // **A heading is picked as well as twirled** (K-300). Until this, a
          // click on one only twirled, so an effect could not be selected here
          // at all and Copy had nothing to take. A plain click still opens the
          // heading, because that is what it has always done and the fold is
          // how the outline is navigated; a *modified* click only picks, so
          // Ctrl- and Shift-clicking a run of effects does not flap every one
          // of them open on the way past.
          onTap: () {
            onSelectProperty(path);
            if (!isModifiedClick) onToggle(path);
          },
          // An *effect's* heading offers to copy the picked effects (K-275,
          // K-300). The other headings — Transform, Effects, Masks, Audio — are
          // groupings rather than things that can be copied, and
          // `effectIdOfPath` is what tells them apart: only an effect's path
          // carries an id.
          onSecondaryTapUp: effectIdOfPath(path) == null
              ? null
              : (details) => _effectMenu(
                    context,
                    details.globalPosition,
                    effectIdOfPath(path)!,
                  ),
          child: Row(
            children: [
              GestureDetector(
                key: ValueKey<String>('tl-twirl-$path'),
                behavior: HitTestBehavior.opaque,
                onTap: () => onToggle(path),
                child: SizedBox(
                  // Wider than the glyph: the twirl is now the only way to open
                  // a heading, so it has to be worth aiming at.
                  width: iconSize + 6,
                  child: glyph.LumitIcon(
                    open ? LumitIcons.collapse : LumitIcons.expand,
                    size: iconSize,
                    colour: open ? t.textPrimary : t.textMuted,
                  ),
                ),
              ),
              const SizedBox(width: 4),
              // An effect's own heading carries what that effect cost, in the
              // render-time column with the layer totals (docs/13 §7.1). Every
              // other heading — Transform, Effects, Audio — is a grouping
              // rather than a thing that renders, so it carries nothing.
              //
              // **Expanded, and no Spacer.** A `Flexible` label beside a
              // `Spacer` splits the free space between them, which put the
              // number halfway across the row instead of in the column: two
              // flex children share, they do not queue. One Expanded label
              // takes the space, and the cell that follows lands hard right —
              // where the layer rows' numbers are.
              if (effectIdOfPath(path) case final String effectId
                  when timingsColumn.width > 0) ...[
                Expanded(
                  child: Text(label,
                      style: t.body, overflow: TextOverflow.ellipsis),
                ),
                Padding(
                  padding: EdgeInsets.only(right: timingsColumn.rightInset),
                  child: SizedBox(
                    width: timingsColumn.width,
                    child: TimingsCell(effectId: effectId),
                  ),
                ),
              ] else
                Flexible(
                  child: Text(label,
                      style: t.body, overflow: TextOverflow.ellipsis),
                ),
            ],
          ),
        ),
      FoldTransformRow(:final group, :final transform) => TransformRowFrb(
          comp: comp,
          layer: layer,
          transform: transform,
          group: group,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
          keyPrefix: 'tl-tf',
          rowPadding: EdgeInsets.zero,
          valueColumn: valueColumn,
          onLabelTap: () => onSelectProperty(path),
          graphColours: graphColours[path],
        ),
      FoldEffectParamRow() => _TimelineParamRow(
          comp: comp,
          layer: layer,
          row: row as FoldEffectParamRow,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
          onLabelTap: () => onSelectProperty(path),
          graphColour: graphColours[path]?.firstOrNull,
        ),
      FoldFlowRow() => _FlowRow(
          comp: comp,
          layer: layer,
          row: row as FoldFlowRow,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
        ),
      FoldVolumeRow(:final scalar) => _VolumeRow(
          comp: comp,
          layer: layer,
          scalar: scalar,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
        ),
      FoldRetimeRow(:final scalar) => _RetimeRow(
          comp: comp,
          layer: layer,
          scalar: scalar,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: onChanged,
          onLabelTap: () => onSelectProperty(path),
        ),
      FoldMaskRow(:final mask) => _MaskRow(
          comp: comp,
          layer: layer,
          mask: mask,
          valueColumn: valueColumn,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
          onLabelTap: () => onSelectProperty(path),
        ),
      FoldMaskValueRow(:final mask, :final value) => _MaskValueRow(
          comp: comp,
          layer: layer,
          mask: mask,
          value: value,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
        ),
      FoldShapeRow(:final item) => _ShapeItemRow(
          comp: comp,
          layer: layer,
          item: item,
          valueColumn: valueColumn,
          onChanged: onChanged,
        ),
      FoldStrokeRow(:final stroke) => _StrokeRow(
          comp: comp,
          layer: layer,
          stroke: stroke,
          valueColumn: valueColumn,
          onChanged: onChanged,
        ),
    };
  }
}

/// One effect parameter in the Timeline. It owns the staging for its own drag,
/// which is all the state a single row needs — no stack is read to *display*:
/// the value rides in on the fold row from the read model (K-184), and a drag
/// in flight overlays its staged value on top.
class _TimelineParamRow extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final FoldEffectParamRow row;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;
  final VoidCallback? onLabelTap;
  final Color? graphColour;

  const _TimelineParamRow({
    required this.comp,
    required this.layer,
    required this.row,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
    this.onLabelTap,
    this.graphColour,
  });

  @override
  State<_TimelineParamRow> createState() => _TimelineParamRowState();
}

class _TimelineParamRowState extends State<_TimelineParamRow> {
  final EffectStackEditor _editor = EffectStackEditor();

  @override
  void dispose() {
    // The editor's preview throttle owns a timer; a row unmounted mid-drag
    // (a twirl shutting, a layer deleted) must not leave it ticking.
    _editor.clear();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final row = widget.row;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return EffectParamRowFrb(
      key: ValueKey<String>('tl-fx-${row.info.id}-${row.param.id}'),
      effectId: row.info.id,
      param: row.param,
      valueColumn: widget.valueColumn,
      // One lane tall, like every other fold row: the card's own vertical
      // padding on top of that clipped the fields.
      rowPadding: EdgeInsets.zero,
      // The staged value while a drag is in flight, the document's otherwise.
      value: _editor.stagedValue(row.info.id, row.param.id) ?? row.value,
      siblings: {for (final v in row.info.values) v.id: v.value},
      comp: widget.comp,
      ownerLayerId: widget.layer.internallayerId,
      ownerLayers: ui.model.layers,
      playheadFrame: widget.playheadFrame,
      onSeek: widget.onSeek,
      onLabelTap: widget.onLabelTap,
      graphColour: widget.graphColour,
      onWrite: (effect, param, value) {
        _editor.write(widget.layer, effect, param, value);
        setState(() {});
        widget.onChanged();
      },
      onLive: (effect, param, value) => setState(() {
        _editor.live(widget.comp, widget.layer, effect, param, value,
            frame: ui.playheadFrame.value, scale: ui.viewerScale);
      }),
    );
  }
}

/// The Audio group's one row: the layer's Volume, in dB.
/// One control of the Flow group in the Timeline fold-out (K-088, K-331).
///
/// Every kind but the Input rate writes the whole group in one op, so the row
/// needs no state of its own: read, change one field, write it back. The Input
/// rate is a keyframeable scalar, so it alone carries the stopwatch and the
/// navigator — the same shape the Retime and Volume rows use.
class _FlowRow extends StatelessWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final FoldFlowRow row;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  const _FlowRow({
    required this.comp,
    required this.layer,
    required this.row,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Read once per document revision by the panel, never here (K-184). The
    // fallback covers a caller that supplied none, which the panel never is.
    final p = row.params ?? layer.getFlowParams();

    void write(BridgeFlowParams next) {
      layer.setFlowParams(params: next);
      onChanged();
    }

    final control = switch (row.kind) {
      FlowRowKind.resolution =>
        _choice('flow-resolution', flowResolutionOptions, p.resolution, (v) {
          write(flowParamsWith(p, resolution: v));
        }),
      FlowRowKind.detail =>
        _choice('flow-detail', flowDetailOptions, p.detail, (v) {
          write(flowParamsWith(p, detail: v));
        }),
      FlowRowKind.occlusion =>
        _choice('flow-occlusion', flowOcclusionOptions, p.occlusion, (v) {
          write(flowParamsWith(p, occlusion: v));
        }),
      FlowRowKind.fallback =>
        _choice('flow-fallback', flowFallbackOptions, p.fallback, (v) {
          write(flowParamsWith(p, fallback: v));
        }),
      FlowRowKind.smoothness => SizedBox(
          width: valueColumn.width,
          child: DragValueField(
            key: const ValueKey('flow-smoothness'),
            value: p.smoothness,
            min: 0,
            max: 100,
            onChanged: (v) =>
                write(flowParamsWith(p, smoothness: v.toDouble())),
          ),
        ),
      FlowRowKind.hudGuard => HouseCheckbox(
          key: const ValueKey('flow-hud-guard'),
          value: p.hudGuard,
          onChanged: (v) => write(flowParamsWith(p, hudGuard: v)),
        ),
      FlowRowKind.always => HouseCheckbox(
          key: const ValueKey('flow-always'),
          value: p.always,
          onChanged: (v) => write(flowParamsWith(p, always: v)),
        ),
      FlowRowKind.inputRate => _inputRate(),
    };

    return Row(
      children: [
        if (row.kind == FlowRowKind.inputRate)
          KeyframeControlsFrb(
            scalars: [row.rate!],
            comp: comp,
            playheadFrame: playheadFrame,
            onSeek: onSeek,
            rowKey: 'tl-flow-rate',
            onWrite: (next) {
              layer.setFlowInputRate(value: next.single);
              onChanged();
            },
          )
        else
          const SizedBox(width: fxKeyframeGutter),
        const SizedBox(width: 4),
        Expanded(child: Text(row.kind.label, style: t.body)),
        SizedBox(width: valueColumn.width, child: control),
      ],
    );
  }

  Widget _choice(
    String keyName,
    List<String> options,
    int value,
    ValueChanged<int> onChanged,
  ) =>
      SizedBox(
        width: valueColumn.width,
        child: FlowChoice(
          keyName: keyName,
          options: options,
          value: value,
          onChanged: onChanged,
        ),
      );

  /// The conform rate: a typed value with the cadence presets beside it, and
  /// keyframes, so a cut that changes cadence partway can be followed.
  Widget _inputRate() {
    final rate = row.rate!;
    final shown = switch (rate) {
      BridgeScalar_Static(:final field0) => field0,
      // An expression is sampled engine-side too, so it needs no case of its
      // own here — `sampledScalar` is the one place either is evaluated.
      BridgeScalar_Keyframed() ||
      BridgeScalar_Expression() =>
        sampledScalar(rate, timeOfFrame(comp, playheadFrame)),
    };
    return FlowRateControl(
      shown: shown,
      fieldWidth: (valueColumn.width * 0.45).clamp(48, 90),
      gap: 4,
      onRate: (fps) {
        layer.setFlowInputRate(
          value: scalarWithValueAt(rate, fps, comp, playheadFrame),
        );
        onChanged();
      },
    );
  }
}

class _VolumeRow extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;

  /// The Volume scalar, read once per document revision by the panel and
  /// riding in on the fold row (K-184).
  final BridgeScalar? scalar;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  const _VolumeRow({
    required this.comp,
    required this.layer,
    required this.scalar,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
  });

  @override
  State<_VolumeRow> createState() => _VolumeRowState();
}

class _VolumeRowState extends State<_VolumeRow> {
  /// The value under the pointer during a drag. Unlike a transform or an effect
  /// there is no preview to render — sound is not redrawn — so a tick only holds
  /// the number and the release commits it.
  double? _staged;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // From the fold row, never a bridge call here (K-184); the fallback
    // covers a caller that supplied none, which the panel never is.
    final scalar = widget.scalar ?? widget.layer.getVolumeDb();
    final animated = scalar is BridgeScalar_Keyframed;
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;

    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) {
        final value = _staged ??
            (animated
                ? sampledScalar(scalar, timeOfFrame(widget.comp, frame))
                : (scalar as BridgeScalar_Static).field0);
        return Row(
          children: [
            KeyframeControlsFrb(
              scalars: [scalar],
              comp: widget.comp,
              playheadFrame: frame,
              onSeek: widget.onSeek,
              rowKey: 'tl-volume',
              onWrite: (next) {
                widget.layer.setVolumeDb(value: next.single);
                widget.onChanged();
              },
            ),
            const SizedBox(width: 4),
            Expanded(child: Text(l10n.volume, style: t.body)),
            SizedBox(
              width: widget.valueColumn.width,
              // Animated: the change lands in the key under the playhead (or
              // plants one) rather than flattening the curve, and the drag is
              // staged so the whole gesture is one undo step.
              child: animated
                  ? KeyedValueField(
                      fieldKey: const ValueKey('tl-volume-db'),
                      value: value,
                      min: -60,
                      max: 12,
                      decimals: 1,
                      suffix: ' dB',
                      speed: 0.2,
                      onCommit: (v) => _commitAt(scalar, v, frame),
                    )
                  : DragValueField(
                      key: const ValueKey('tl-volume-db'),
                      value: value,
                      // The engine's own range (docs/09 §6): silence to a
                      // +12 dB boost.
                      min: -60,
                      max: 12,
                      decimals: 1,
                      suffix: ' dB',
                      speed: 0.2,
                      onChanged: (v) => _commitAt(scalar, v, frame),
                      onChangeLive: (v) =>
                          setState(() => _staged = v.toDouble()),
                      onChangeEnd: (v) => _commitAt(scalar, v, frame),
                      onDragCancel: () => setState(() => _staged = null),
                    ),
            ),
            SizedBox(width: widget.valueColumn.rightInset),
          ],
        );
      },
    );
  }

  void _commitAt(BridgeScalar scalar, num value, int frame) {
    widget.layer.setVolumeDb(
      value: scalarWithValueAt(scalar, value.toDouble(), widget.comp, frame),
    );
    setState(() => _staged = null);
    widget.onChanged();
  }
}

/// The layer's Retime (K-197): which moment of the source, in seconds, the
/// layer shows at this point on its own timeline.
///
/// An ordinary property row — the same stopwatch, the same navigator, the same
/// lane diamonds and the same graph lanes as Position. It sits above Transform
/// and only exists while the layer has been given a Retime (Ctrl+Alt+T), so
/// unlike Volume its scalar arrives on the fold row rather than being read here
/// (K-184: no bridge calls while drawing).
/// [m] with one or two fields changed. The engine takes the whole mask, so
/// every edit and every preview here is "the mask, with this changed".
BridgeMask maskWith(
  BridgeMask m, {
  String? name,
  bool? inverted,
  BridgeScalar? opacity,
  BridgeMaskMode? mode,
  BridgeScalar? feather,
  BridgeScalar? expansion,
}) =>
    BridgeMask(
      id: m.id,
      name: name ?? m.name,
      vertices: m.vertices,
      closed: m.closed,
      inverted: inverted ?? m.inverted,
      opacity: opacity ?? m.opacity,
      mode: mode ?? m.mode,
      feather: feather ?? m.feather,
      expansion: expansion ?? m.expansion,
      // Where the shape's own keys are is the engine's to say; an edit here
      // never moves them (`set_mask` patches them back).
      pathKeys: m.pathKeys,
    );

/// What a mask mode is called on its dropdown.
String maskModeLabel(BridgeMaskMode mode) => switch (mode) {
      BridgeMaskMode.none => l10n.maskModeNone,
      BridgeMaskMode.add => l10n.maskModeAdd,
      BridgeMaskMode.subtract => l10n.maskModeSubtract,
      BridgeMaskMode.intersect => l10n.maskModeIntersect,
      BridgeMaskMode.difference => l10n.maskModeDifference,
    };

/// The inline rename shared by the mask row and the shape-item row.
///
/// In plain terms: a shape drawn with the ellipse tool arrives called
/// "Ellipse", which is the right name until it isn't — this is how it becomes
/// "left eye". The name is a label; a double-click (or the row menu's
/// **Rename**) turns it into a field; `Enter` or a click elsewhere keeps what
/// was typed; `Escape` throws it away. An empty name is refused, because a row
/// with no name is worse than a row named after its tool.
///
/// **Why not a single click.** A single tap on these names *selects* the row,
/// and selection is what `Delete` acts on (K-234), so the rename needs a
/// gesture of its own.
///
/// **Why not `onDoubleTap`.** A double-tap recogniser holds every single tap
/// back for the whole double-tap window while the arena waits to see whether a
/// second one is coming — the layer bar found that out beside the razor and
/// counts timestamps instead ([DoubleTap]). The same trade applies here, and
/// worse: selection arriving a third of a second after the click is the thing
/// `Delete` is waiting on. Two timestamps owe the arena nothing.
///
/// The commit is one write through the row's own `_write`, so it is one op and
/// one undo step, exactly as the opacity drag beside it is (K-234, K-240).
mixin _InlineRename<T extends StatefulWidget> on State<T> {
  TextEditingController? _editor;
  final DoubleTap _nameTaps = DoubleTap();

  /// What the row is called now, and how it writes a new name.
  String get renameCurrent;
  void renameCommit(String name);

  /// Open the editor on the current name. Safe to call twice; the second call
  /// leaves the edit in progress alone rather than restarting it.
  void startRename() {
    if (_editor != null) return;
    setState(() => _editor = TextEditingController(text: renameCurrent));
  }

  /// Close the editor, writing what was typed only when [keep].
  void _endRename({required bool keep}) {
    // Both ways out can land here for one edit — submitting and then losing
    // the pointer — and the row can be gone by the time the second arrives.
    if (!mounted || _editor == null) return;
    final text = _editor?.text.trim() ?? '';
    setState(() {
      _editor?.dispose();
      _editor = null;
    });
    if (!keep || text.isEmpty || text == renameCurrent) return;
    renameCommit(text);
  }

  @override
  void dispose() {
    _editor?.dispose();
    super.dispose();
  }

  /// The name cell: the label, or the editor once a rename has started.
  ///
  /// [onTap] still fires on the first tap and at once, so selection is never
  /// held up; the second tap inside the double-tap window opens the editor.
  Widget renameName({
    required String nameKey,
    required String editorKey,
    required TextStyle style,
    VoidCallback? onTap,
  }) {
    final editor = _editor;
    if (editor != null) {
      return Focus(
        // An ancestor of the field, so `Escape` reaches here after the field
        // has had its say: abandon the edit and keep the stored name.
        onKeyEvent: (_, event) {
          if (event is! KeyDownEvent ||
              event.logicalKey != LogicalKeyboardKey.escape) {
            return KeyEventResult.ignored;
          }
          _endRename(keep: false);
          return KeyEventResult.handled;
        },
        child: HouseTextField(
          key: ValueKey<String>(editorKey),
          controller: editor,
          autofocus: true,
          onSubmitted: (_) => _endRename(keep: true),
          // Clicking anywhere else finishes the edit and keeps what was typed,
          // the same as every other inline rename here (K-243).
          onTapOutside: () => _endRename(keep: true),
        ),
      );
    }
    return GestureDetector(
      key: ValueKey<String>(nameKey),
      behavior: HitTestBehavior.opaque,
      onTap: () {
        onTap?.call();
        if (_nameTaps.tap()) startRename();
      },
      child: Text(renameCurrent, style: style, overflow: TextOverflow.ellipsis),
    );
  }
}

/// One mask's row in the fold-out (K-222): its name, its mode, its invert
/// switch and its opacity. Its feather and its expansion are rows of their own
/// underneath, because the value column holds one field.
///
/// Read from the model, written through the layer's own handle — the same shape
/// as every other row here. Deleting a mask is on its right-click menu, and on
/// the Delete key once the row is selected; a button per mask on every row is a
/// row of ways to lose work by mistake.
///
/// The row is selectable like any other property (K-234): tapping its name
/// calls [onLabelTap], the outline highlights it, and Delete acts on it.
class _MaskRow extends StatefulWidget {
  final LayerReference layer;
  final BridgeMask mask;
  final ValueColumn valueColumn;

  final VoidCallback onChanged;
  final VoidCallback? onLabelTap;

  /// The composition, for the live preview a drag shows (K-240).
  final CompositionReference comp;

  const _MaskRow({
    required this.layer,
    required this.mask,
    required this.valueColumn,
    required this.onChanged,
    required this.comp,
    this.onLabelTap,
  });

  @override
  State<_MaskRow> createState() => _MaskRowState();
}

class _MaskRowState extends State<_MaskRow> with _InlineRename<_MaskRow> {
  @override
  String get renameCurrent => widget.mask.name;

  @override
  void renameCommit(String name) => _write(name: name);

  /// Write the mask back with one field changed. The engine takes the whole
  /// mask, so this is the only shape an edit has.
  void _write({String? name, bool? inverted, BridgeMaskMode? mode}) {
    try {
      widget.layer.setMask(
        mask: maskWith(widget.mask, name: name, inverted: inverted, mode: mode),
      );
      widget.onChanged();
    } catch (_) {
      // The mask or its layer went away between the draw and the click.
    }
  }

  @override
  Widget build(BuildContext context) {
    final mask = widget.mask;
    final valueColumn = widget.valueColumn;
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapUp: (details) => _menu(context, details.globalPosition),
      child: Row(
        children: [
          lumitIcon(LumitIcon.rectangle,
              size: iconSize, color: t.textSecondary),
          const SizedBox(width: 4),
          // The name is the row's handle, exactly as it is on a transform row:
          // tapping it selects the mask, and Delete then acts on it. A
          // double-click renames it in place, and so does the row menu.
          Expanded(
            child: renameName(
              nameKey: 'tl-mask-name-${mask.id}',
              editorKey: 'tl-mask-rename-${mask.id}',
              style: t.body,
              onTap: widget.onLabelTap,
            ),
          ),
          // **Both of the mask's own switches live in the value column**, where
          // every other row's control sits, rather than floating beside the
          // name: the invert mark and the mode picker are what the mask *is*,
          // and a control that sits in no column reads as belonging to nothing.
          SizedBox(
            width: valueColumn.width,
            child: Row(
              children: [
                LumitTooltip(
                  message: l10n.tipInvert,
                  child: HouseButton(
                    key: ValueKey<String>('tl-mask-invert-${mask.id}'),
                    small: true,
                    frameless: true,
                    onPressed: () => _write(inverted: !mask.inverted),
                    child: Text(
                      l10n.maskInvertMark,
                      style: t.small.copyWith(
                          color: mask.inverted ? t.accent : t.textMuted),
                    ),
                  ),
                ),
                const SizedBox(width: 6),
                // The rest of the cell, so a long mode name ellipsises rather
                // than pushing the row wider than its column — the same rule
                // the blend picker follows.
                Expanded(
                  child: LumitTooltip(
                    message: l10n.tipMaskMode,
                    child: BareDropdown<BridgeMaskMode>(
                      key: ValueKey<String>('tl-mask-mode-${mask.id}'),
                      value: mask.mode,
                      options: BridgeMaskMode.values,
                      label: maskModeLabel,
                      onChanged: (m) => _write(mode: m),
                    ),
                  ),
                ),
              ],
            ),
          ),
          SizedBox(width: valueColumn.rightInset),
        ],
      ),
    );
  }

  void _menu(BuildContext context, Offset at) {
    showMenuAt<void>(
      context: context,
      position: at,
      width: 160,
      rows: (close) => [
        MenuRow(
          key: ValueKey<String>('tl-mask-rename-menu-${widget.mask.id}'),
          onPressed: () {
            close(null);
            startRename();
          },
          // The same bare "Rename" the Project panel's row menu offers.
          child: Text(l10n.rename),
        ),
        MenuRow(
          key: ValueKey<String>('tl-mask-delete-${widget.mask.id}'),
          onPressed: () {
            close(null);
            try {
              widget.layer.deleteMask(id: widget.mask.id);
              widget.onChanged();
            } catch (_) {}
          },
          child: Text(l10n.deleteMask),
        ),
      ],
    );
  }
}

/// One of a mask's values on a row under it (K-222, K-340): its shape, its
/// opacity, its feather or its expansion.
///
/// **Every one of them animates, and animates the way everything else does.**
/// The row carries the same stopwatch and ◄ ◆ ► the transform and effect rows
/// carry, reads its value at the playhead, and writes an edit into the key
/// sitting there — so a mask is keyed with the same gesture as a position.
///
/// The **shape** is the exception in one respect only: a path has no number to
/// put in a field, so its row is a name, a stopwatch and its diamonds, and the
/// shape itself is edited where it is drawn (K-339).
///
/// The drag is staged and previewed exactly as it always was, so the whole
/// gesture is one op and one undo step (K-234, K-240).
///
/// The row has no label tap: the mask itself is what Delete acts on, and a
/// selectable value row under it would give Delete a path it cannot resolve to
/// a mask.
class _MaskValueRow extends StatefulWidget {
  final LayerReference layer;
  final CompositionReference comp;
  final BridgeMask mask;
  final MaskValue value;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  const _MaskValueRow({
    required this.layer,
    required this.comp,
    required this.mask,
    required this.value,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
  });

  @override
  State<_MaskValueRow> createState() => _MaskValueRowState();
}

class _MaskValueRowState extends State<_MaskValueRow> {
  double? _staged;
  final PreviewThrottle _throttle = PreviewThrottle();

  bool get _isPath => widget.value == MaskValue.path;

  /// This row's animation. The path has none of its own — its keys are whole
  /// shapes, not numbers — so [maskScalarOf] answers a still zero for it.
  BridgeScalar get _scalar => maskScalarOf(widget.mask, widget.value);

  /// What a drag on this row may ask for. Feather is a width, so it has no
  /// negative side; expansion grows one way and shrinks the other; opacity is
  /// a percentage.
  (double, double) get _range => switch (widget.value) {
        MaskValue.opacity => (0, 100),
        MaskValue.feather => (0, 1000),
        _ => (-1000, 1000),
      };

  int get _decimals => widget.value == MaskValue.opacity ? 0 : 1;

  String get _suffix => widget.value == MaskValue.opacity ? '%' : ' px';

  @override
  void dispose() {
    _throttle.cancel();
    super.dispose();
  }

  /// Show the value the drag is passing through without writing it (K-240).
  void _preview(BridgeScalar v) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    _throttle.request(() {
      try {
        widget.comp.renderFrameWithMaskPreview(
          frame: BigInt.from(ui.playheadFrame.value),
          scale: ui.viewerScale,
          layer: widget.layer,
          masks: [
            for (final m in widget.layer.getMasks())
              if (m.id == widget.mask.id)
                maskWithScalar(m, widget.value, v)
              else
                m,
          ],
        );
      } catch (_) {
        // A preview is a courtesy; the drag carries on without it.
      }
    });
  }

  void _write(BridgeScalar v) {
    setState(() => _staged = null);
    try {
      widget.layer.setMask(mask: maskWithScalar(widget.mask, widget.value, v));
      widget.onChanged();
    } catch (_) {
      // The mask or its layer went away mid-drag.
    }
  }

  /// A still value: the number typed or dragged becomes the value.
  void _commitStatic(num v) => _write(BridgeScalar.static_(v.toDouble()));

  /// An animated one: the edit lands on the key under the playhead, or plants
  /// one there — never flattening the curve (docs/07 §4.3).
  void _commitKeyed(double v) =>
      _write(scalarWithValueAt(_scalar, v, widget.comp, widget.playheadFrame));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Row(
      children: [
        if (_isPath)
          MaskPathKeyframesFrb(
            layer: widget.layer,
            mask: widget.mask,
            comp: widget.comp,
            playheadFrame: widget.playheadFrame,
            onSeek: widget.onSeek,
            onChanged: widget.onChanged,
          )
        else
          KeyframeControlsFrb(
            scalars: [_scalar],
            onWrite: (s) => _write(s.first),
            comp: widget.comp,
            playheadFrame: widget.playheadFrame,
            onSeek: widget.onSeek,
            rowKey: 'tl-mask-${widget.value.name}-${widget.mask.id}',
          ),
        const SizedBox(width: 4),
        Expanded(
          child: Text(maskValueLabel(widget.value),
              style: t.body, overflow: TextOverflow.ellipsis),
        ),
        // Left of the value column, exactly where an effect parameter's field
        // sits, so every number down an open layer forms one column.
        SizedBox(
          width: widget.valueColumn.width,
          child: _isPath
              ? const SizedBox.shrink()
              : Align(
                  alignment: Alignment.centerLeft,
                  child: SizedBox(width: 72, child: _field()),
                ),
        ),
        SizedBox(width: widget.valueColumn.rightInset),
      ],
    );
  }

  Widget _field() {
    final (min, max) = _range;
    final key =
        ValueKey<String>('tl-mask-${widget.value.name}-${widget.mask.id}');
    final scalar = _scalar;
    if (scalar is! BridgeScalar_Keyframed) {
      final stored =
          _staged ?? (scalar is BridgeScalar_Static ? scalar.field0 : 0.0);
      return DragValueField(
        key: key,
        value: stored,
        min: min,
        max: max,
        decimals: _decimals,
        suffix: _suffix,
        onChanged: _commitStatic,
        onChangeLive: (v) {
          setState(() => _staged = v.toDouble());
          _preview(BridgeScalar.static_(v.toDouble()));
        },
        onChangeEnd: _commitStatic,
        onDragCancel: () {
          setState(() => _staged = null);
          // Put the document's own value back on screen.
          _preview(scalar);
        },
      );
    }
    // Animated: the field shows what the curve reads at the playhead, and an
    // edit writes the key there. No live preview mid-drag — staging a keyed
    // value through the static preview would lie about the curve.
    return KeyedValueField(
      fieldKey: key,
      value:
          sampledScalar(scalar, timeOfFrame(widget.comp, widget.playheadFrame)),
      min: min,
      max: max,
      decimals: _decimals,
      suffix: _suffix,
      onCommit: _commitKeyed,
    );
  }
}

/// One named, deletable item with an opacity of its own — a piece of a shape
/// layer's art (K-237) or a paint stroke (K-227). The two rows were twins:
/// an icon, the name, the staged-and-previewed opacity drag, and the
/// right-click menu that deletes it. What differs — how a preview is asked
/// for, how an edit is written, whether the name renames — comes in as
/// callbacks from the two thin rows below.
///
/// The drag is staged and previewed like every other dragged value here: the
/// tick shows live and the release commits once, so a gesture is one op and
/// one undo step (K-238, K-239).
class _ItemOpacityRow extends StatefulWidget {
  final LumitIcon icon;
  final String name;

  /// The widget keys' stem: `<keyPrefix>-name-<id>` and so on, kept exactly
  /// as the two original rows spelt them.
  final String keyPrefix;
  final String id;
  final double opacity;
  final ValueColumn valueColumn;

  /// Render the picture with [opacity] in place of the stored one; called
  /// from inside the row's own throttle.
  final void Function(double opacity) onPreview;

  /// Commit [opacity] as one op.
  final void Function(double opacity) onCommit;

  /// Write a new name, or null when this kind's name is not renamed here —
  /// which also drops the menu's Rename row.
  final void Function(String name)? onRename;
  final VoidCallback onDelete;
  final String deleteLabel;

  const _ItemOpacityRow({
    required this.icon,
    required this.name,
    required this.keyPrefix,
    required this.id,
    required this.opacity,
    required this.valueColumn,
    required this.onPreview,
    required this.onCommit,
    this.onRename,
    required this.onDelete,
    required this.deleteLabel,
  });

  @override
  State<_ItemOpacityRow> createState() => _ItemOpacityRowState();
}

class _ItemOpacityRowState extends State<_ItemOpacityRow>
    with _InlineRename<_ItemOpacityRow> {
  /// The opacity a drag is part way through, or null when nothing is
  /// dragging. Without it the field committed on every tick, so one drag was
  /// a stack of ops and `Ctrl+Z` backed out a hair (K-238, K-239).
  double? _staged;

  final PreviewThrottle _throttle = PreviewThrottle();

  @override
  String get renameCurrent => widget.name;

  @override
  void renameCommit(String name) => widget.onRename?.call(name);

  @override
  void dispose() {
    _throttle.cancel();
    super.dispose();
  }

  void _preview(double opacity) =>
      _throttle.request(() => widget.onPreview(opacity));

  void _commitOpacity(num v) {
    setState(() => _staged = null);
    widget.onCommit(v.toDouble());
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapUp: (details) => _menu(context, details.globalPosition),
      child: Row(
        children: [
          lumitIcon(widget.icon, size: iconSize, color: t.textSecondary),
          const SizedBox(width: 4),
          // Named after the tool that drew it — and, where the kind supports
          // it, renamed here: a double-click on the name, or the row menu.
          Expanded(
            child: widget.onRename == null
                ? Text(widget.name,
                    style: t.body, overflow: TextOverflow.ellipsis)
                : renameName(
                    nameKey: '${widget.keyPrefix}-name-${widget.id}',
                    editorKey: '${widget.keyPrefix}-rename-${widget.id}',
                    style: t.body,
                  ),
          ),
          SizedBox(
            width: widget.valueColumn.width,
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                SizedBox(
                  width: 56,
                  // Staged and previewed, like every other dragged value
                  // here: the drag shows live and commits once on release,
                  // so it is one op and one undo step.
                  child: DragValueField(
                    key: ValueKey<String>(
                        '${widget.keyPrefix}-opacity-${widget.id}'),
                    value: _staged ?? widget.opacity,
                    min: 0,
                    max: 100,
                    suffix: '%',
                    onChanged: _commitOpacity,
                    onChangeLive: (v) {
                      setState(() => _staged = v.toDouble());
                      _preview(v.toDouble());
                    },
                    onChangeEnd: _commitOpacity,
                    onDragCancel: () {
                      setState(() => _staged = null);
                      // The picture is showing a value nobody committed; put
                      // the document's own back on screen.
                      _preview(widget.opacity);
                    },
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  void _menu(BuildContext context, Offset at) {
    showMenuAt<void>(
      context: context,
      position: at,
      width: 160,
      rows: (close) => [
        if (widget.onRename != null)
          MenuRow(
            key: ValueKey<String>(
                '${widget.keyPrefix}-rename-menu-${widget.id}'),
            onPressed: () {
              close(null);
              startRename();
            },
            child: Text(l10n.rename),
          ),
        MenuRow(
          key: ValueKey<String>('${widget.keyPrefix}-delete-${widget.id}'),
          onPressed: () {
            close(null);
            widget.onDelete();
          },
          child: Text(widget.deleteLabel),
        ),
      ],
    );
  }
}

/// One piece of a shape layer's art in the Timeline (K-237), on the shared
/// [_ItemOpacityRow]. The engine takes the whole contents list, so every
/// edit — and the drag's preview — is "the list, with this item changed".
class _ShapeItemRow extends StatelessWidget {
  final LayerReference layer;
  final BridgeShapeItem item;
  final ValueColumn valueColumn;
  final VoidCallback onChanged;

  /// The composition, for the live preview a drag shows (K-239).
  final CompositionReference comp;

  const _ShapeItemRow({
    required this.layer,
    required this.item,
    required this.valueColumn,
    required this.onChanged,
    required this.comp,
  });

  static BridgeShapeItem _with(BridgeShapeItem i,
          {String? name, double? opacity}) =>
      BridgeShapeItem(
        id: i.id,
        name: name ?? i.name,
        vertices: i.vertices,
        closed: i.closed,
        fill: i.fill,
        stroke: i.stroke,
        strokeWidth: i.strokeWidth,
        opacity: opacity ?? i.opacity,
      );

  /// Write the contents back with this item changed, or dropped.
  void _write({String? name, double? opacity, bool delete = false}) {
    try {
      layer.setShapeContents(contents: [
        for (final other in layer.getShapeContents())
          if (other.id != item.id)
            other
          else if (!delete)
            _with(other, name: name, opacity: opacity),
      ]);
      onChanged();
    } catch (_) {
      // The item or its layer went away between the draw and the click.
    }
  }

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return _ItemOpacityRow(
      icon: LumitIcon.rectangle,
      name: item.name,
      keyPrefix: 'tl-shape',
      id: item.id.toString(),
      opacity: item.opacity,
      valueColumn: valueColumn,
      // Show the opacity the drag is passing through without writing it
      // (K-239), exactly as the stroke row does.
      onPreview: (opacity) {
        try {
          comp.renderFrameWithShapePreview(
            frame: BigInt.from(ui.playheadFrame.value),
            scale: ui.viewerScale,
            layer: layer,
            contents: [
              for (final i in layer.getShapeContents())
                if (i.id == item.id) _with(i, opacity: opacity) else i,
            ],
          );
        } catch (_) {
          // A preview is a courtesy; the drag carries on without it.
        }
      },
      onCommit: (opacity) => _write(opacity: opacity),
      onRename: (name) => _write(name: name),
      onDelete: () => _write(delete: true),
      deleteLabel: l10n.deleteShape,
    );
  }
}

/// One paint stroke in the Timeline (K-227), on the shared [_ItemOpacityRow].
/// The engine takes the whole stroke, so every edit is "this stroke, with one
/// field changed" — and its name is not renamed here, so the row shows it
/// plain.
class _StrokeRow extends StatelessWidget {
  final LayerReference layer;
  final BridgeStroke stroke;
  final ValueColumn valueColumn;
  final VoidCallback onChanged;

  /// The composition, for the live preview a drag shows (K-239).
  final CompositionReference comp;

  const _StrokeRow({
    required this.layer,
    required this.stroke,
    required this.valueColumn,
    required this.onChanged,
    required this.comp,
  });

  static BridgeStroke _withOpacity(BridgeStroke s, double opacity) =>
      BridgeStroke(
        id: s.id,
        name: s.name,
        points: s.points,
        colour: s.colour,
        width: s.width,
        hardness: s.hardness,
        opacity: opacity,
        mode: s.mode,
        cloneOffsetX: s.cloneOffsetX,
        cloneOffsetY: s.cloneOffsetY,
      );

  /// The icon says which of the three tools made it, so a list of marks can
  /// be read at a glance.
  LumitIcon get _icon => switch (stroke.mode) {
        BridgePaintMode.erase => LumitIcon.eraser,
        BridgePaintMode.clone => LumitIcon.cloneStamp,
        BridgePaintMode.paint => LumitIcon.brush,
      };

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return _ItemOpacityRow(
      icon: _icon,
      name: stroke.name,
      keyPrefix: 'tl-stroke',
      id: stroke.id.toString(),
      opacity: stroke.opacity,
      valueColumn: valueColumn,
      // The *whole* stroke list is sent, with this one stroke's opacity
      // replaced, because paint is stored and committed as a whole list. A
      // preview shaped differently from the op would be a second description
      // of the same thing.
      onPreview: (opacity) {
        try {
          comp.renderFrameWithPaintPreview(
            frame: BigInt.from(ui.playheadFrame.value),
            scale: ui.viewerScale,
            layer: layer,
            strokes: [
              for (final s in layer.getPaint())
                if (s.id == stroke.id) _withOpacity(s, opacity) else s,
            ],
          );
        } catch (_) {
          // A preview is a courtesy; the drag carries on without it.
        }
      },
      onCommit: (opacity) {
        try {
          layer.setStroke(stroke: _withOpacity(stroke, opacity));
          onChanged();
        } catch (_) {
          // The stroke or its layer went away between the draw and the
          // click.
        }
      },
      onDelete: () {
        try {
          layer.deleteStroke(id: stroke.id);
          onChanged();
        } catch (_) {}
      },
      deleteLabel: l10n.deleteStroke,
    );
  }
}

class _RetimeRow extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final BridgeScalar scalar;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  /// Selects the channel, so its curve opens in the graph — the same handle
  /// every other property row's name is. Retime was built without one, which
  /// left it the one channel `graphChannels` could build and nobody could
  /// choose.
  final VoidCallback? onLabelTap;

  const _RetimeRow({
    required this.comp,
    required this.layer,
    required this.scalar,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
    this.onLabelTap,
  });

  @override
  State<_RetimeRow> createState() => _RetimeRowState();
}

class _RetimeRowState extends State<_RetimeRow> {
  /// The value under the pointer during a drag, held so the whole gesture is
  /// one undo step. The picture keeps up in the meantime: a retime drag decides
  /// which frame is decoded, so it previews through its own door
  /// (`renderFrameWithRetime`) rather than by re-compositing pixels already in
  /// hand — the one edit where watching it move is the whole point.
  double? _staged;

  final PreviewThrottle _preview = PreviewThrottle();

  @override
  void dispose() {
    _preview.cancel();
    super.dispose();
  }

  /// The footage's own rate, probed once when the row mounts. Null until the
  /// probe answers, or when the source is not footage (or carries no video
  /// stream) — the comp rate stands in then, so the clock is always usable.
  (int, int)? _sourceFps;

  @override
  void initState() {
    super.initState();
    _probeSourceFps();
  }

  Future<void> _probeSourceFps() async {
    final item = widget.layer.getSourceItem();
    if (item is! ItemReference_Footage) return;
    final info = await item.field0.mediaInfo();
    if (!mounted || info == null || info.fpsNum <= 0 || info.fpsDen <= 0) {
      return;
    }
    setState(() => _sourceFps = (info.fpsNum, info.fpsDen));
  }

  /// Whether this gesture already planted its key — one plant per drag.
  bool _planted = false;

  /// A drag tick: render the map the release will write, without writing it —
  /// and publish it, so the graph's Retime curve follows the drag (K-334).
  ///
  /// The first tick on a frame with **no key plants one** holding the value
  /// already showing (K-333's rule, K-336 for this row): nothing moves, and
  /// the preview then *replaces* a real key instead of inserting beside the
  /// document's — the aligned path the transform rows take.
  void _live(BridgeScalar scalar, double value, int frame) {
    if (!_planted &&
        scalar is BridgeScalar_Keyframed &&
        !scalar.field0
            .any((k) => widget.comp.frameAtTime(time: k.time) == frame)) {
      _planted = true;
      final held = sampleScalar(
          scalar: scalar, time: widget.comp.timeOfFrame(frame: frame));
      widget.layer.setRetimeProperty(
        value: scalarWithValueAt(scalar, held, widget.comp, frame),
      );
      widget.onChanged();
    }
    setState(() => _staged = value);
    rowValueDrag.value = RowValueDrag(
      layer: widget.layer.internallayerId.toString(),
      retime: true,
      frame: frame,
      value: value,
    );
    final ui = Provider.of<LumitUiState>(context, listen: false);
    _preview.request(() => widget.comp.renderFrameWithRetime(
          frame: BigInt.from(ui.playheadFrame.value),
          scale: ui.viewerScale,
          layer: widget.layer,
          retime: scalarWithValueAt(scalar, value, widget.comp, frame),
        ));
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final scalar = widget.scalar;
    final animated = scalar is BridgeScalar_Keyframed;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final playhead = ui.playheadFrame;
    // Which face the row wears (K-287): the clock by default, seconds for
    // anyone who asked for them in Settings ▸ Interface ▸ Editing.
    final seconds = ui.workspace.interface.retimeInSeconds;
    // The clock face counts *source* frames, so it runs at the footage's own
    // rate — 600 fps footage counts to :599 whatever the comp's rate says.
    final (fpsNum, fpsDen) = _sourceFps ?? ui.model.fpsExact;

    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) {
        final value = _staged ??
            (animated
                ? sampledScalar(scalar, timeOfFrame(widget.comp, frame))
                : (scalar as BridgeScalar_Static).field0);
        return Row(
          children: [
            KeyframeControlsFrb(
              scalars: [scalar],
              comp: widget.comp,
              playheadFrame: frame,
              onSeek: widget.onSeek,
              rowKey: 'tl-retime',
              onWrite: (next) {
                widget.layer.setRetimeProperty(value: next.single);
                widget.onChanged();
              },
            ),
            const SizedBox(width: 4),
            Expanded(
              child: GestureDetector(
                key: const ValueKey('tl-retime-name'),
                behavior: HitTestBehavior.opaque,
                onTap: widget.onLabelTap,
                child: Text(l10n.retime, style: t.body),
              ),
            ),
            SizedBox(
              width: widget.valueColumn.width,
              child: seconds
                  ? (animated
                      ? KeyedValueField(
                          fieldKey: const ValueKey('tl-retime-seconds'),
                          onLive: (v) => _live(scalar, v, frame),
                          value: value,
                          // The same open range a transform axis gets: a
                          // source time before zero or past the end simply
                          // holds the end frame (docs/04 §7), so clamping the
                          // field would only fight the drag.
                          min: -100000,
                          max: 100000,
                          decimals: 3,
                          suffix: ' s',
                          speed: 0.02,
                          onCommit: (v) => _commitAt(scalar, v, frame),
                        )
                      : DragValueField(
                          key: const ValueKey('tl-retime-seconds'),
                          value: value,
                          min: -100000,
                          max: 100000,
                          decimals: 3,
                          suffix: ' s',
                          speed: 0.02,
                          onChanged: (v) => _commitAt(scalar, v, frame),
                          onChangeLive: (v) =>
                              _live(scalar, v.toDouble(), frame),
                          onChangeEnd: (v) => _commitAt(scalar, v, frame),
                          onDragCancel: () => setState(() => _staged = null),
                        ))
                  // The clock face (K-287, realising K-075): which moment of
                  // the source is showing, written the way every other time in
                  // the editor is written. Dragged and typed in whole source
                  // frames — a timecode cannot say "between two frames", which
                  // is what the seconds setting is for.
                  : TimeReadout(
                      key: const ValueKey('tl-retime-seconds'),
                      frame: _frameOfSeconds(value, fpsNum, fpsDen),
                      format: (f) => timecodeOfRateSigned(f, fpsNum, fpsDen),
                      parse: (text) =>
                          framesOfTimecodeSigned(text, fpsNum, fpsDen),
                      widthChars: timecodeChars(fpsNum, fpsDen) + 1,
                      style: t.mono,
                      minFrame: -100000,
                      maxFrame: 100000,
                      draggable: true,
                      onDragLive: (f) => _live(
                          scalar, _secondsOfFrame(f, fpsNum, fpsDen), frame),
                      onCommit: (f) => _commitAt(
                          scalar, _secondsOfFrame(f, fpsNum, fpsDen), frame),
                      onDragCancel: () => setState(() => _staged = null),
                    ),
            ),
            SizedBox(width: widget.valueColumn.rightInset),
          ],
        );
      },
    );
  }

  /// A source time in seconds as a whole source frame, and back.
  ///
  /// At the footage's own rate where the source is footage whose rate is
  /// known; at the composition's rate until the probe answers, and for
  /// everything else.
  static int _frameOfSeconds(double seconds, int fpsNum, int fpsDen) {
    if (fpsDen <= 0 || fpsNum <= 0) return 0;
    return (seconds * fpsNum / fpsDen).round();
  }

  static double _secondsOfFrame(int frame, int fpsNum, int fpsDen) =>
      fpsNum <= 0 ? 0 : frame * (fpsDen <= 0 ? 1 : fpsDen) / fpsNum;

  void _commitAt(BridgeScalar scalar, num value, int frame) {
    // The write is the last word on the gesture: a held preview tick after it
    // would put the provisional picture back.
    _preview.cancel();
    rowValueDrag.value = null;
    _planted = false;
    widget.layer.setRetimeProperty(
      value: scalarWithValueAt(scalar, value.toDouble(), widget.comp, frame),
    );
    setState(() => _staged = null);
    widget.onChanged();
  }
}

/// The live preview of a bar drag in flight: how far each edge and the start
/// offset have moved, in frames. Published by the bar and read by the waveform
/// lane, so the transients travel with the bar rather than jumping on release
/// (K-172). Null between gestures.
class BarDragPreview {
  final String layerId;
  final int deltaIn;
  final int deltaOut;
  final int offsetShift;
  const BarDragPreview(
      this.layerId, this.deltaIn, this.deltaOut, this.offsetShift);
}

/// What a grab of [grab] moved by [delta] frames does to a layer's span.
/// Moving carries the content with the bar, so the start offset travels too;
/// a trim leaves the content where it is and moves one edge over it.
BarDragPreview barDragPreview(String layerId, BarGrab grab, int delta) =>
    switch (grab) {
      BarGrab.move => BarDragPreview(layerId, delta, delta, delta),
      BarGrab.trimIn => BarDragPreview(layerId, delta, 0, 0),
      BarGrab.trimOut => BarDragPreview(layerId, 0, delta, 0),
    };

/// How far a layer's ends may be dragged, in comp frames (K-211).
///
/// **In plain terms:** a Footage, audio or Precomp layer can only show what its
/// source actually holds, so its bar stops where the media does — its head
/// cannot be dragged earlier than the source's first frame, and its tail cannot
/// be dragged past its last. Every generated kind — Solid, Text, Adjustment,
/// Null, Camera, Sequence — has no such source, so both its ends are free and
/// it is whatever length the user drags it to. Switching **Retime** on frees
/// the ends too (docs/04-RETIMING.md): a retimed layer decides for itself which
/// source moment each of its own frames shows, so its length stops being the
/// source's business.
class BarBounds {
  /// The earliest frame the in point may be trimmed to; null = the head is free.
  final int? minIn;

  /// The latest frame the out point may be trimmed to; null = the tail is free.
  final int? maxOut;

  const BarBounds({this.minIn, this.maxOut});

  /// Both ends free: every generated kind, anything retimed, and any source
  /// whose length could not be read.
  static const BarBounds free = BarBounds();

  @override
  bool operator ==(Object other) =>
      other is BarBounds && other.minIn == minIn && other.maxOut == maxOut;

  @override
  int get hashCode => Object.hash(minIn, maxOut);
}

/// The bounds one layer's bar trims within.
///
/// [startOffsetFrame] is where the layer's own time zero sits on the comp
/// timeline, which is where its source's first frame shows; [sourceFrames] is
/// the source's length in comp frames, or null when the layer has no source of
/// its own — or when its length could not be read at all, which leaves the ends
/// free rather than pinning them to a guess (missing media must never silently
/// crop a layer).
BarBounds barBounds({
  required int startOffsetFrame,
  required int? sourceFrames,
  required bool retimed,
}) =>
    retimed || sourceFrames == null
        ? BarBounds.free
        : BarBounds(
            minIn: startOffsetFrame,
            maxOut: startOffsetFrame + sourceFrames,
          );

/// How far a grab of [grab] may actually travel when the gesture has moved
/// [delta] frames: inside the layer's source, and never far enough to turn the
/// bar inside out — a bar always keeps at least one frame.
///
/// A **move** is never clamped. Moving carries the start offset with the bar,
/// so a layer that sits inside its source stays inside it however far it
/// travels; only the two trims can run out of source.
///
/// A bound never drags an edge that is *already* outside it — a layer whose
/// Retime was switched off after being stretched keeps the length it has, and
/// its ends stay where the user left them until they are dragged back in.
int clampBarDelta({
  required BarGrab grab,
  required int delta,
  required int inFrame,
  required int outFrame,
  required BarBounds bounds,
}) {
  switch (grab) {
    case BarGrab.move:
      return delta;
    case BarGrab.trimIn:
      var want = inFrame + delta;
      final earliest = bounds.minIn;
      if (earliest != null) want = max(want, min(earliest, inFrame));
      return min(want, outFrame - 1) - inFrame;
    case BarGrab.trimOut:
      var want = outFrame + delta;
      final latest = bounds.maxOut;
      if (latest != null) want = min(want, max(latest, outFrame));
      return max(want, inFrame + 1) - outFrame;
  }
}

/// An exact time as a comp frame number, without asking the engine (K-184).
///
/// The same floor `FrameRate::frame_at` takes, in whole integers so a long
/// timeline cannot drift the way a double would: a time `num/den` seconds at
/// `fpsNum/fpsDen` frames a second is `num·fpsNum / (den·fpsDen)`, rounded
/// down — and down for negative times too, which is what a layer starting
/// before the comp needs.
int frameOfTime(BridgeRational time, int fpsNum, int fpsDen) {
  final den = time.den.toInt() * fpsDen;
  if (den <= 0) return 0;
  final scaled = time.num.toInt() * fpsNum;
  final quotient = scaled ~/ den;
  return scaled % den != 0 && scaled < 0 ? quotient - 1 : quotient;
}

/// The corner marks that say a bar has run out of source (K-211): a small
/// triangle in the top-left corner when the head is as early as its media
/// allows, and one in the top-right when the tail is as late. Drawn only on the
/// kinds that have a source to run out of, and never on a retimed layer, whose
/// ends are free.
class BarEndMarksPainter extends CustomPainter {
  final bool atIn;
  final bool atOut;
  final Color colour;

  /// The triangle's legs. Small enough to read as a corner cut on a 22px row
  /// rather than as a badge sitting on the bar.
  static const double leg = 5;

  const BarEndMarksPainter({
    required this.atIn,
    required this.atOut,
    required this.colour,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (size.width <= 0) return;
    // Never let the two marks meet in the middle of a very short bar: a bar
    // narrower than both legs draws marks scaled to fit it instead.
    final l = min(leg, size.width / 2);
    final paint = Paint()..color = colour;
    if (atIn) {
      canvas.drawPath(
        Path()
          ..moveTo(0, 0)
          ..lineTo(l, 0)
          ..lineTo(0, l)
          ..close(),
        paint,
      );
    }
    if (atOut) {
      canvas.drawPath(
        Path()
          ..moveTo(size.width, 0)
          ..lineTo(size.width - l, 0)
          ..lineTo(size.width, l)
          ..close(),
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(BarEndMarksPainter old) =>
      old.atIn != atIn || old.atOut != atOut || old.colour != colour;
}

/// The outline's toolbar (docs/07 §4.1, §12A.1): the timecode and frame
/// readouts at the far left, the layer search stretched across the middle as
/// an inset well, and the Layers / Graph mode segments at the far right — with
/// the master motion-blur and shy-filter buttons and the ⋯ menu (the
/// layer/work-area/marker commands the old full-width toolbar carried) between
/// the well and the segments.
class _Toolbar extends StatelessWidget {
  final CompositionReference comp;

  /// The read model, for the master motion-blur state and the exact rate —
  /// no bridge calls in a build (K-184).
  final CompModel model;

  /// Listened to, not read: only the two readouts redraw as it moves.
  final ValueListenable<int> playhead;

  /// Where a typed time goes — the same take-hold-of-the-playhead move a drag
  /// on the ruler makes, so typing a time also stops the transport.
  final ValueChanged<int> onSeek;

  final bool graph;
  final VoidCallback onToggleGraph;
  final bool razor;
  final VoidCallback onToggleRazor;
  final bool hideShy;
  final VoidCallback onToggleHideShy;
  final ValueChanged<String> onSearch;
  final VoidCallback onChanged;

  const _Toolbar({
    required this.comp,
    required this.model,
    required this.playhead,
    required this.onSeek,
    required this.graph,
    required this.onToggleGraph,
    required this.razor,
    required this.onToggleRazor,
    required this.hideShy,
    required this.onToggleHideShy,
    required this.onSearch,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final (fpsNum, fpsDen) = model.fpsExact;
    final mbOn = model.motionBlurEnabled;
    final lastFrame = model.durationFrames - 1;
    return Container(
      height: _toolbarHeight,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Row(
        children: [
          // The clock face and the frame count, both zero-based: frame 0 is
          // 00:00:00:00, so three seconds into a 24 fps comp reads f72.
          //
          // Both sit in slots wide enough for the longest thing they can say
          // and both can be typed into (K-287): a readout that resized itself
          // as it counted shoved the search field sideways through every
          // second of playback, and a time you can read is a time you should
          // be able to state. Anything outside the composition lands on its
          // nearest end.
          ValueListenableBuilder<int>(
            valueListenable: playhead,
            builder: (context, frame, _) => Row(
              children: [
                TimeReadout(
                  key: const ValueKey('tl-timecode'),
                  frame: frame,
                  format: (f) => timecodeOfRate(f, fpsNum, fpsDen),
                  widthChars: timecodeChars(fpsNum, fpsDen),
                  // The clock is the row's first fact and reads at full
                  // strength; the frame count beside it is the same moment
                  // said again, so it stays muted (§12A.1).
                  style: t.mono.copyWith(color: t.textPrimary),
                  parse: (text) => framesOfTimecode(text, fpsNum, fpsDen),
                  onCommit: onSeek,
                  minFrame: 0,
                  maxFrame: lastFrame,
                  tooltip: l10n.tipPlayheadTime,
                ),
                TimeReadout(
                  key: const ValueKey('tl-frame'),
                  frame: frame,
                  format: (f) => 'f$f',
                  // The `f`, the digits of the last frame, and one spare so a
                  // comp that grows past a power of ten does not start to
                  // twitch before the next rebuild.
                  widthChars: 2 + '${lastFrame < 0 ? 0 : lastFrame}'.length,
                  style: t.mono.copyWith(color: t.textMuted),
                  parse: _frameOfTyped,
                  onCommit: onSeek,
                  minFrame: 0,
                  maxFrame: lastFrame,
                  tooltip: l10n.tipFrameNumber,
                ),
              ],
            ),
          ),
          const SizedBox(width: 10),
          Expanded(child: LayerSearchFrb(onChanged: onSearch, width: 1e9)),
          const SizedBox(width: 8),
          _iconButton(
            context,
            keyName: 'tl-mb-master',
            icon: LumitIcon.motionBlur,
            on: mbOn,
            tip:
                mbOn ? l10n.tipMasterMotionBlurOn : l10n.tipMasterMotionBlurOff,
            onPressed: () {
              comp.setMotionBlurEnabled(on_: !mbOn);
              onChanged();
            },
          ),
          _iconButton(
            context,
            keyName: 'tl-hide-shy',
            icon: LumitIcon.shy,
            on: hideShy,
            tip: hideShy ? l10n.tipShyHidden : l10n.tipHideShy,
            onPressed: onToggleHideShy,
          ),
          HouseButton(
            key: const ValueKey('tl-more'),
            small: true,
            frameless: true,
            onPressed: () => _showMoreMenu(context),
            child: Text('⋯', style: t.small),
          ),
          const SizedBox(width: 6),
          // The two modes, at the far right of the row (§12A.1). Kicker
          // segments rather than icons: "Layers" and "Graph" are the names of
          // two shapes of the same panel, and a word says which one is in
          // force where two small glyphs made the reader guess.
          _modeTab(
            context,
            keyName: 'tl-view-lanes',
            label: l10n.timelineModeLayers,
            tip: l10n.tipLaneView,
            active: !graph,
            onPressed: graph ? onToggleGraph : () {},
          ),
          _modeTab(
            context,
            // Keeps the key the old Graph toolbar button had, so the graph
            // editor's own tests and muscle memory both still find it.
            keyName: 'tl-graph',
            label: l10n.timelineModeGraph,
            tip: l10n.tipGraphView,
            active: graph,
            onPressed: graph ? () {} : onToggleGraph,
          ),
        ],
      ),
    );
  }

  /// One of the two mode segments. The one in force wears the secondary
  /// button's outline and a `kickerOn` label; the other is frameless and
  /// muted. **No accent**: §3.1's accent list is closed, and a mode segment is
  /// not on it — which of the two is in force reads from the frame.
  Widget _modeTab(
    BuildContext context, {
    required String keyName,
    required String label,
    required String tip,
    required bool active,
    required VoidCallback onPressed,
  }) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: tip,
      child: HouseButton(
        key: ValueKey<String>(keyName),
        small: true,
        frameless: !active,
        padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
        onPressed: onPressed,
        child: Text(label.toUpperCase(), style: active ? t.kickerOn : t.kicker),
      ),
    );
  }

  /// A typed frame number, with or without the `f` the readout wears. Null for
  /// anything that is not a number at all, which leaves the readout alone.
  static int? _frameOfTyped(String text) {
    var trimmed = text.trim().toLowerCase();
    if (trimmed.startsWith('f')) trimmed = trimmed.substring(1);
    return int.tryParse(trimmed.trim());
  }

  Widget _iconButton(
    BuildContext context, {
    required String keyName,
    required LumitIcon icon,
    required bool on,
    required String tip,
    required VoidCallback onPressed,
  }) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: tip,
      child: HouseButton(
        key: ValueKey<String>(keyName),
        small: true,
        frameless: true,
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
        onPressed: onPressed,
        child:
            lumitIcon(icon, size: iconSize, color: on ? t.accent : t.textMuted),
      ),
    );
  }

  /// The commands that used to line the full-width toolbar, one menu deep:
  /// adding layers, the razor, the work area, markers and beat detection.
  Future<void> _showMoreMenu(BuildContext context) async {
    final t = ThemeScope.of(context).theme;
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    final playheadNow = playhead.value;
    final picked = await showMenuAt<String>(
      context: context,
      position: box.localToGlobal(Offset(box.size.width - 190, 24)),
      width: 190,
      rows: (close) => [
        MenuRow(
            key: const ValueKey('tl-add-layer'),
            onPressed: () => close('new-layer'),
            child: Text(l10n.newLayer)),
        MenuRow(
            key: const ValueKey('tl-razor'),
            onPressed: () => close('razor'),
            child: Text(razor ? l10n.disarmRazor : l10n.armRazor,
                style: razor ? t.body.copyWith(color: t.accent) : null)),
        MenuRow(
            key: const ValueKey('tl-work-in'),
            onPressed: () => close('work-in'),
            child: Text(l10n.workAreaStart)),
        MenuRow(
            key: const ValueKey('tl-work-out'),
            onPressed: () => close('work-out'),
            child: Text(l10n.workAreaEnd)),
        MenuRow(
            key: const ValueKey('tl-clear-work-area'),
            onPressed: () => close('work-clear'),
            child: Text(l10n.workAreaClear)),
        MenuRow(
            key: const ValueKey('tl-markers'),
            onPressed: () => close('markers'),
            child: Text(l10n.menuMarkers)),
        MenuRow(
            key: const ValueKey('tl-detect-beats'),
            onPressed: () => close('beats'),
            child: Text(l10n.menuDetectBeats)),
      ],
    );
    if (!context.mounted) return;
    switch (picked) {
      case 'new-layer':
        await _showLayerMenu(context, comp, onChanged);
      case 'razor':
        onToggleRazor();
      case 'work-in' || 'work-out':
        comp.setWorkArea(
          span: workAreaWith(
            comp: comp,
            current: comp.getWorkArea(),
            wanted: playheadNow,
            isStart: picked == 'work-in',
          ),
        );
        onChanged();
      case 'work-clear':
        comp.setWorkArea(span: null);
        onChanged();
      case 'markers':
        await showMarkerEditorFrb(
          context: context,
          comp: comp,
          playheadFrame: playheadNow,
        );
        onChanged();
      case 'beats':
        // Seconds-long on a long comp, so it runs off-thread and the markers
        // appear when it finishes; a comp with no audio, or a machine with no
        // pipeline, says so by doing nothing rather than by an alarm. The card
        // is up for those seconds so the silence is not mistaken for a command
        // that did not land, and it comes down either way.
        showBusyWhile(
          context.read<LumitState>().busy,
          l10n.detectingBeats,
          comp
              .detectBeats(sensitivityPercent: 50)
              .then<void>((_) => onChanged(), onError: (_) {}),
        );
      case _:
        return;
    }
  }
}

/// A controller's scroll position, or null when there is not exactly one
/// view attached.
///
/// `ScrollController.offset` and `.position` both assert on a controller with
/// two views, which happens for a frame whenever a rebuild inserts the new
/// scroll view before the old one detaches — a drop target lighting up over
/// the panel was enough to hit it.
ScrollPosition? _positionOf(ScrollController controller) =>
    controller.positions.length == 1 ? controller.positions.first : null;

/// A scrollbar for a scroll view that is somewhere else in the tree.
///
/// `RawScrollbar` learns where its scrollable is from `ScrollNotification`s
/// rising through *its own* subtree. Sat in a gutter beside the scroll view,
/// it receives none — so it never repainted and the thumb was simply
/// invisible (K-192). This listens to the controller instead, which is the
/// thing it actually needs to know about, and drags it directly.
class _GutterScrollbar extends StatelessWidget {
  final ScrollController controller;
  final Axis axis;
  const _GutterScrollbar({
    required this.controller,
    this.axis = Axis.vertical,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return AnimatedBuilder(
      animation: controller,
      builder: (context, _) {
        final position = _positionOf(controller);
        if (position == null || !position.hasContentDimensions) {
          return const SizedBox.expand();
        }
        final viewport = position.viewportDimension;
        final range = position.maxScrollExtent;
        // Nothing overflows: no thumb, and nothing to grab at.
        if (range <= 0.5 || viewport <= 0) return const SizedBox.expand();

        return LayoutBuilder(
          builder: (context, constraints) {
            final track = axis == Axis.vertical
                ? constraints.maxHeight
                : constraints.maxWidth;
            if (track <= 0) return const SizedBox.expand();
            final extent =
                (viewport / (viewport + range) * track).clamp(20.0, track);
            final travel = track - extent;
            final offset = travel <= 0 ? 0.0 : position.pixels / range * travel;

            void dragBy(double delta) {
              if (travel <= 0) return;
              controller.jumpTo(
                  (position.pixels + delta / travel * range).clamp(0.0, range));
            }

            final thumb = MouseRegion(
              cursor: SystemMouseCursors.grab,
              child: GestureDetector(
                key: const ValueKey('tl-gutter-thumb'),
                behavior: HitTestBehavior.opaque,
                onVerticalDragUpdate:
                    axis == Axis.vertical ? (d) => dragBy(d.delta.dy) : null,
                onHorizontalDragUpdate:
                    axis == Axis.horizontal ? (d) => dragBy(d.delta.dx) : null,
                child: Container(
                  margin: const EdgeInsets.all(3),
                  decoration: BoxDecoration(
                    color: t.hairlineStrong,
                    borderRadius: BorderRadius.circular(3),
                  ),
                ),
              ),
            );

            return Stack(
              children: [
                axis == Axis.vertical
                    ? Positioned(
                        top: offset,
                        left: 0,
                        right: 0,
                        height: extent,
                        child: thumb)
                    : Positioned(
                        left: offset,
                        top: 0,
                        bottom: 0,
                        width: extent,
                        child: thumb),
              ],
            );
          },
        );
      },
    );
  }
}

/// The seam between adjacent column groups, in a row: plain space of exactly
/// [groupDividerWidth]. The header's rule is enough to read the grouping by;
/// repeating it down every row of a tall stack is noise. The width matches
/// the header's seam so the two stay column-aligned.
const Widget _rowSeam = SizedBox(width: groupDividerWidth);

/// The header's seam: the hairline that names the grouping, and the handle
/// that resizes the group to its left (docs/07 §4.2). Everything else keeps
/// its width, so a drag here widens or narrows the whole outline.
class _GroupSeam extends StatelessWidget {
  final ValueChanged<double> onResize;
  const _GroupSeam({super.key, required this.onResize});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return MouseRegion(
      cursor: SystemMouseCursors.resizeColumn,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onHorizontalDragUpdate: (d) => onResize(d.delta.dx),
        child: SizedBox(
          width: groupDividerWidth,
          child: Center(
            child: Container(width: 1, height: 14, color: t.hairlineStrong),
          ),
        ),
      ),
    );
  }
}

/// The column-group header (docs/07 §4.2): one icon per column, grouped into
/// the four clusters, each cluster draggable as a unit to reorder them.
class _ColumnHeader extends StatelessWidget {
  final List<TimelineGroup> order;
  final Map<TimelineGroup, double> widths;
  final void Function(TimelineGroup dragged, TimelineGroup target) onReorder;

  /// A seam dragged: widen (or narrow) the group on its left by `delta`.
  final void Function(TimelineGroup group, double delta) onResize;

  const _ColumnHeader({
    required this.order,
    required this.widths,
    required this.onReorder,
    required this.onResize,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      height: _headerHeight,
      color: t.surface2,
      padding: const EdgeInsets.symmetric(horizontal: 4),
      child: Row(
        children: [
          for (var i = 0; i < order.length; i++) ...[
            // The seam resizes the group it follows, which is the one the eye
            // reads it as belonging to.
            if (i > 0)
              _GroupSeam(
                key: ValueKey<String>('tl-seam-${order[i - 1].name}'),
                onResize: (delta) => onResize(order[i - 1], delta),
              ),
            _draggable(context, t, order[i]),
          ],
        ],
      ),
    );
  }

  Widget _draggable(BuildContext context, LumitTheme t, TimelineGroup group) {
    final content = SizedBox(
      width: widths[group],
      child: _cells(t, group, widths[group] ?? 0),
    );
    return DragTarget<TimelineGroup>(
      onWillAcceptWithDetails: (d) => d.data != group,
      onAcceptWithDetails: (d) => onReorder(d.data, group),
      builder: (context, candidate, _) => Draggable<TimelineGroup>(
        key: ValueKey<String>('tl-colgroup-${group.name}'),
        data: group,
        feedback: Container(
          height: _headerHeight,
          padding: const EdgeInsets.symmetric(horizontal: 8),
          color: t.surface2,
          child: Center(
            child: Text(_labelOf(group), style: t.small),
          ),
        ),
        childWhenDragging: Opacity(opacity: 0.4, child: content),
        child: Container(
          color: candidate.isEmpty ? null : t.accent.withValues(alpha: 0.18),
          child: content,
        ),
      ),
    );
  }

  String _labelOf(TimelineGroup group) => columnGroupLabel(group);

  /// The header cells, in the same widths the rows use, so each icon stands
  /// over its column. Indicators only — clicking a header does nothing; the
  /// switches live on the rows (docs/07 §4.2). Each carries a hover hint
  /// naming its column.
  Widget _cells(LumitTheme t, TimelineGroup group, double width) {
    Widget icon(LumitIcon i, String tip) => LumitTooltip(
          message: tip,
          child:
              Center(child: lumitIcon(i, size: iconSize, color: t.textMuted)),
        );
    Widget cell(LumitIcon i, String tip) =>
        SizedBox(width: switchCellWidth, child: icon(i, tip));
    // The same legend drawn from Lumit's own set (K-440), for the columns the
    // set already has a mark for.
    Widget markCell(String mark, String tip) => SizedBox(
          width: switchCellWidth,
          child: LumitTooltip(
            message: tip,
            child: Center(
                child:
                    glyph.LumitIcon(mark, size: iconSize, colour: t.textMuted)),
          ),
        );
    // The compose titles carry the dropdown's own text inset, so each sits
    // directly over the text in the cell below it.
    Widget title(String text, String tip, double width) => SizedBox(
          width: width,
          child: LumitTooltip(
            message: tip,
            child: Align(
              alignment: Alignment.centerLeft,
              child: Padding(
                padding: const EdgeInsets.only(left: dropdownTextInset),
                child: Text(text, style: t.small),
              ),
            ),
          ),
        );
    return switch (group) {
      TimelineGroup.switches => Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            markCell(LumitIcons.visible, l10n.switchVisible),
            markCell(LumitIcons.audio, l10n.switchAudible),
            markCell(LumitIcons.solo, l10n.switchSolo),
            markCell(LumitIcons.lock, l10n.switchLock),
            cell(LumitIcon.shy, l10n.switchShy),
          ],
        ),
      TimelineGroup.identity => Row(
          children: [
            const SizedBox(width: 16), // the twirl column has no header icon
            SizedBox(
                width: 16, child: icon(LumitIcon.label, l10n.tipLabelColour)),
            const SizedBox(width: 4),
            Expanded(
              child: Text(l10n.columnLayer,
                  style: t.small, overflow: TextOverflow.ellipsis),
            ),
          ],
        ),
      // The switches pack left in ordinary cells; the rest of the group's
      // span is the fold-out's value column, not spare icon room.
      TimelineGroup.render => Row(
          children: [
            cell(LumitIcon.flow, l10n.switchFlow),
            cell(LumitIcon.fx, l10n.switchEffects),
            cell(LumitIcon.motionBlur, l10n.switchMotionBlur),
            cell(LumitIcon.cube3d, l10n.switchThreeD),
            cell(LumitIcon.aperture, l10n.switchAcceptsLights),
          ],
        ),
      // The render-time column's header is its switch — see timeline_timings.
      TimelineGroup.timings => const TimingsHeaderCell(),
      TimelineGroup.compose => () {
          final (matte, blend, parent) = composeCellWidths(width);
          return Row(
            children: [
              title(l10n.columnMatte, l10n.tipMatte, matte),
              const SizedBox(width: cellGap),
              title(l10n.columnBlend, l10n.tipBlendMode, blend),
              const SizedBox(width: cellGap),
              title(l10n.columnParent, l10n.tipParent, parent),
            ],
          );
        }(),
    };
  }
}

Future<void> _showLayerMenu(
  BuildContext context,
  CompositionReference comp,
  VoidCallback onChanged,
) async {
  final box = context.findRenderObject();
  if (box is! RenderBox) return;
  final picked = await showMenuAt<VoidCallback>(
    context: context,
    position: box.localToGlobal(Offset(0, box.size.height + 2)),
    width: 190,
    rows: (close) => [
      // The row carries what it does, not a word to switch on: the label is
      // translated (K-303) and would no longer match an English case.
      for (final (label, add) in <(String, VoidCallback)>[
        (l10n.menuSolid, comp.addSolidLayer),
        (l10n.menuText, comp.addTextLayer),
        (l10n.menuCamera, comp.addCameraLayer),
        (l10n.menuPointLight, () => comp.addLightLayer(kind: 0)),
        (l10n.menuSpotLight, () => comp.addLightLayer(kind: 1)),
        (l10n.menuAreaLight, () => comp.addLightLayer(kind: 2)),
        (l10n.menuAdjustment, comp.addAdjustmentLayer),
        (l10n.menuNull, comp.addNullLayer),
        (l10n.menuSequence, comp.addSequenceLayer),
      ])
        MenuRow(onPressed: () => close(add), child: Text(label)),
    ],
  );
  if (picked == null) return;
  picked();
  onChanged();
}

/// The left column: one row per layer, with its switches and columns.
class _Outline extends StatelessWidget {
  final CompositionReference comp;

  /// The layers as the panel decided them — the same [LayerRow] list the lane
  /// area draws from, so a row's fold-out, its open Sequence view and its
  /// height are one answer rather than two that agree.
  final List<LayerRow> rows;

  /// The column groups in their current order and at their current widths
  /// (docs/07 §4.2) — rows draw their cells to match the header's.
  final List<TimelineGroup> groupOrder;
  final Map<TimelineGroup, double> widths;

  /// The whole selection as ids (K-217), worked out once by the panel: a row
  /// asking "am I selected?" is then one set lookup rather than a walk of the
  /// list per row per paint.
  final Set<UuidValue> selectedIds;
  final String? highlighted;

  /// The selected properties' fold paths, in selection order: each is a
  /// curve in the graph, its row draws selected, and every row containing
  /// one highlights (docs/07 §4.3, §5).
  final List<String> selectedProperties;

  /// Each selected path's graph line colours, for tinting its label.
  final Map<String, List<Color>> graphColours;
  final ValueChanged<String> onSelectProperty;
  final ValueChanged<String> onEditProperty;

  /// Open or close a Sequence layer's view (K-248).
  final void Function(BridgeLayerEntry entry)? onOpenSequence;
  final ValueChanged<String> onToggle;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final ValueChanged<LayerReference> onSelect;
  final ValueChanged<String> onHighlight;
  final VoidCallback onChanged;

  /// The drag in flight and the block heights it slides by — the panel's, so
  /// the lanes are working from the same two values (K-208).
  final ValueNotifier<LayerDrag?> layerDrag;
  final List<double> blockHeights;

  /// The layer `Enter` has just asked to rename (K-243).
  final ValueNotifier<UuidValue?> renameRequest;

  const _Outline({
    required this.comp,
    required this.rows,
    required this.groupOrder,
    required this.widths,
    required this.selectedIds,
    required this.highlighted,
    required this.selectedProperties,
    required this.graphColours,
    required this.onSelectProperty,
    required this.onEditProperty,
    this.onOpenSequence,
    required this.onToggle,
    required this.playheadFrame,
    required this.onSeek,
    required this.onSelect,
    required this.onHighlight,
    required this.onChanged,
    required this.layerDrag,
    required this.blockHeights,
    required this.renameRequest,
  });

  @override
  Widget build(BuildContext context) {
    // The column geometry is the same for every row, so it is worked out once
    // here rather than once per fold row of every layer.
    final valueColumn = valueColumnFor(groupOrder, widths);
    final timingsColumn = timingsColumnFor(groupOrder, widths);
    final baseIndent = identityStart(groupOrder, widths);
    // The layer entries, for the parent picker's menu — every layer is on
    // offer as a parent, and they come from the row list rather than from a
    // second list handed in beside it.
    final layers = [for (final row in rows) row.entry];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (var i = 0; i < rows.length; i++)
          LayerDragSlide(
            drag: layerDrag,
            heights: blockHeights,
            index: i,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                _OutlineRow(
                  key: ValueKey<String>('tl-row-${rows[i].id}'),
                  comp: comp,
                  entry: rows[i].entry,
                  onOpenSequence: () => onOpenSequence?.call(rows[i].entry),
                  layers: layers,
                  groupOrder: groupOrder,
                  widths: widths,
                  index: i,
                  count: rows.length,
                  // A local compare, not a bridge call: both ids already sit here.
                  selected:
                      selectedIds.contains(rows[i].entry.layer.internallayerId),
                  // A layer marks itself when its fold was last touched, and when
                  // a selected property is one of its own (docs/07 §4.3).
                  highlighted: highlighted == rows[i].id ||
                      selectedProperties.any((p) => isUnderPath(rows[i].id, p)),
                  open: rows[i].open,
                  hasAudio: rows[i].hasAudio,
                  hasPicture: rows[i].hasPicture,
                  onToggleOpen: () => onToggle(rows[i].id),
                  onSelect: () => onSelect(rows[i].entry.layer),
                  onChanged: onChanged,
                  layerDrag: layerDrag,
                  renameRequest: renameRequest,
                  blockHeights: blockHeights,
                ),
                // The room the lanes draw an open sequence view in (K-248). The
                // outline has nothing to put here — the clips and their envelope are
                // the lane's to draw — but it must leave exactly the same gap, or
                // every row below this one sits at a different height on the two
                // sides of the Timeline and the halves stop lining up. Both sides
                // ask the same [LayerRow], so the gap and the view cannot be
                // opened by one half and not the other.
                if (rows[i].sequenceExtra != null)
                  SizedBox(
                    key: ValueKey<String>('tl-seq-room-${rows[i].id}'),
                    height: rows[i].sequenceExtra,
                  ),
                // The fold-out, from the same list the lanes leave room for.
                for (final row in rows[i].drawnRows)
                  // A raw pointer listener, not a gesture: touching a sub-item
                  // highlights its layer, and it must never fight the row's own
                  // taps and drags for the gesture arena.
                  Listener(
                    onPointerDown: (_) => onHighlight(rows[i].id),
                    child: _FoldRow(
                      comp: comp,
                      layer: rows[i].entry.layer,
                      row: row,
                      valueColumn: valueColumn,
                      timingsColumn: timingsColumn,
                      baseIndent: baseIndent,
                      path: foldRowPath(rows[i].id, row),
                      selectedProperties: selectedProperties,
                      graphColours: graphColours,
                      onSelectProperty: onSelectProperty,
                      onEditProperty: onEditProperty,
                      playheadFrame: playheadFrame,
                      onSeek: onSeek,
                      onToggle: onToggle,
                      onChanged: onChanged,
                      locked: rows[i].entry.info.switches.locked,
                    ),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}

class _OutlineRow extends StatefulWidget {
  final CompositionReference comp;
  final BridgeLayerEntry entry;

  /// Open or close this layer's sequence view (K-248) — what a double-click
  /// on a Sequence layer means, where on other kinds it opens the source.
  final VoidCallback? onOpenSequence;

  /// Every layer in the comp, for the parent picker's menu — from the same
  /// read model, so offering them costs nothing.
  final List<BridgeLayerEntry> layers;

  /// The column groups in their current order, and their current widths
  /// (docs/07 §4.2).
  final List<TimelineGroup> groupOrder;
  final Map<TimelineGroup, double> widths;
  final int index;
  final int count;
  final bool selected;

  /// A sub-item of this layer was last touched — drawn a shade dimmer than
  /// selection, so the two states read apart at a glance.
  final bool highlighted;
  final bool open;

  /// What this layer can do (K-435), so the switches column offers only that:
  /// no audible switch where there is no sound, no visibility switch where
  /// there is no picture. Passed down from the panel — probing for either
  /// answer must never happen in a row's build.
  final bool hasAudio;
  final bool hasPicture;
  final VoidCallback onToggleOpen;
  final VoidCallback onSelect;
  final VoidCallback onChanged;

  /// The panel's drag state: this row is where the gesture is made — the name
  /// is the stack handle — and setting it here is what lets the lanes beside
  /// the outline move with it (K-208).
  final ValueNotifier<LayerDrag?> layerDrag;

  /// The layer the panel has just been asked to rename (`Enter`, K-243), or
  /// null. A notifier rather than a rebuild because only the one row it names
  /// has anything to do about it.
  final ValueNotifier<UuidValue?> renameRequest;

  /// Every block's height, as the stack stood when the panel last built —
  /// what a drag's travel is measured against, so the answer does not depend
  /// on rows the drag is itself moving.
  final List<double> blockHeights;

  const _OutlineRow({
    super.key,
    required this.comp,
    required this.entry,
    this.onOpenSequence,
    required this.layers,
    required this.groupOrder,
    required this.widths,
    required this.index,
    required this.count,
    required this.selected,
    required this.highlighted,
    required this.open,
    this.hasAudio = false,
    this.hasPicture = true,
    required this.onToggleOpen,
    required this.onSelect,
    required this.onChanged,
    required this.layerDrag,
    required this.renameRequest,
    required this.blockHeights,
  });

  @override
  State<_OutlineRow> createState() => _OutlineRowState();
}

class _OutlineRowState extends State<_OutlineRow> {
  /// The inline rename, entered with `Enter` on the selected layer.
  TextEditingController? _rename;

  /// How far this row has been dragged since the lift, in pixels down.
  ///
  /// Accumulated from the gesture's own deltas rather than read back off the
  /// widget's position, because the widget is being slid by the drag: its
  /// position is an output of this number, so reading it back would be the
  /// loop the travel measure exists to break.
  double _dragTravel = 0;

  /// Put the layer where the drag says, and let the rows go.
  ///
  /// A drop that lands where it started is not a reorder — it is the user
  /// changing their mind, and it must cost nothing. Committing it anyway
  /// wrote an undo step for a stack that had not moved.
  void _commitDrag() {
    final drag = widget.layerDrag.value;
    widget.layerDrag.value = null;
    if (drag == null || drag.from == drag.to) return;
    widget.layers[drag.from].layer.reorder(newIndex: BigInt.from(drag.to));
    widget.onChanged();
  }

  LayerReference get layer => widget.entry.layer;
  int get index => widget.index;
  int get count => widget.count;

  @override
  void initState() {
    super.initState();
    widget.renameRequest.addListener(_maybeRename);
  }

  @override
  void dispose() {
    widget.renameRequest.removeListener(_maybeRename);
    _rename?.dispose();
    super.dispose();
  }

  /// `Enter` on the selected layer names this row: open the editor on it.
  /// A locked layer keeps its name, the same as it did when a double-click was
  /// what opened the editor — lock means no edits.
  void _maybeRename() {
    if (!mounted || _rename != null) return;
    if (widget.renameRequest.value != layer.internallayerId) return;
    if (widget.entry.info.switches.locked) return;
    setState(
        () => _rename = TextEditingController(text: widget.entry.info.name));
  }

  /// Escape: shut the editor and rename nothing (K-323). Shares the closing
  /// half of [_commitRename] — the write is the only difference between them.
  void _cancelRename() {
    if (!mounted || _rename == null) return;
    setState(() {
      _rename?.dispose();
      _rename = null;
    });
    if (widget.renameRequest.value == layer.internallayerId) {
      widget.renameRequest.value = null;
    }
  }

  void _commitRename() {
    // Both ways out of the editor can land here for one edit — submitting and
    // then losing the pointer — and the row can be gone by the time the second
    // arrives. Either way there is nothing left to commit.
    if (!mounted || _rename == null) return;
    final text = _rename?.text.trim() ?? '';
    setState(() {
      _rename?.dispose();
      _rename = null;
    });
    // Clear the request this row answered, so pressing Enter again on the same
    // layer opens the editor a second time rather than seeing no change.
    if (widget.renameRequest.value == layer.internallayerId) {
      widget.renameRequest.value = null;
    }
    if (text.isEmpty || text == widget.entry.info.name) return;
    layer.rename(name: text);
    widget.onChanged();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // ZERO bridge calls: everything this row draws is in the read model
    // (K-184).
    final info = widget.entry.info;

    // Selection happens on the DOWN, for the whole row, outside the gesture
    // arena — the reason the name has always done it that way (see the note by
    // the name cell) applies to every other cell too, and the row's tap used to
    // do it a *second* time on the way up. Two calls per click is invisible for
    // a plain click and exactly wrong for a Ctrl+click, which toggled the layer
    // in and straight back out again.
    return Listener(
      onPointerDown: (event) {
        if (_claimed) {
          _claimed = false;
          return;
        }
        if (event.buttons == kPrimaryButton) widget.onSelect();
      },
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        // A tap that does nothing, so that nothing is what happens: the empty
        // ground behind these rows deselects on tap (K-203), and a row that
        // entered no tap into the arena let the ground win and throw away the
        // selection the pointer-down had just made.
        onTap: () {},
        onSecondaryTapDown: (d) => _showRowMenu(context, d.globalPosition),
        child: Container(
          // No drop line: the rows themselves move to where they would land,
          // so a line marking the same slot said it twice.
          child: _rowBody(context, t, info),
        ),
      ),
    );
  }

  /// Set by a control on its way down, so the row above it leaves the
  /// selection alone: pressing a layer's eye, or opening its properties, is
  /// not choosing the layer. The gesture arena used to settle this by itself,
  /// and cannot now that the row selects from a raw listener outside it.
  ///
  /// Cleared by the very next pointer-down the row sees, which is this same
  /// one — Flutter hands a pointer to the innermost target first, so the
  /// control always sets this before the row reads it.
  bool _claimed = false;

  /// Mark [child]'s clicks as the control's own, not the row's.
  Widget _ownClick(Widget child) =>
      Listener(onPointerDown: (_) => _claimed = true, child: child);

  Widget _rowBody(BuildContext context, LumitTheme t, BridgeLayerInfo info) {
    return Container(
        key: ValueKey<String>('tl-rowbody-${layer.internallayerId}'),
        height: _rowHeight,
        decoration: BoxDecoration(
          // Selected is the brighter of the two states; a highlight (this
          // layer's fold-out was last touched) is the same surface at half
          // strength, so they read apart at a glance.
          color: widget.selected
              ? t.selectionFill
              : widget.highlighted
                  ? t.selectionFill.withValues(alpha: 0.45)
                  : null,
          // No seam of its own: K-192's overlay draws the seams for the whole
          // outline, and a border here drew a *second* line a fraction of a
          // pixel from it — the overlay is phased by the scroll offset, which
          // a trackpad leaves fractional, so the two lines pulled apart as the
          // table scrolled and the outline's rows read a hair taller than the
          // lanes beside them.
        ),
        padding: const EdgeInsets.symmetric(horizontal: 4),
        child: Row(
          children: [
            // The cells come in the four column groups, in whatever order
            // the header's drag has put them and at whatever width its seams
            // have been dragged to (docs/07 §4.2).
            for (var i = 0; i < widget.groupOrder.length; i++) ...[
              if (i > 0) _rowSeam,
              SizedBox(
                width: widget.widths[widget.groupOrder[i]],
                // Only the identity group is the layer itself — its name and
                // its number are what you click to choose it. The other three
                // are controls: hiding a layer, or picking its blend mode, is
                // not choosing it, and those cells have never selected.
                child: switch (widget.groupOrder[i]) {
                  TimelineGroup.identity => _identityCells(context, t, info),
                  TimelineGroup.switches =>
                    _ownClick(_switchCells(context, info)),
                  TimelineGroup.render =>
                    _ownClick(_renderCells(context, info)),
                  TimelineGroup.compose => _ownClick(_composeCells(context, t,
                      info, widget.widths[TimelineGroup.compose] ?? 0)),
                  // What this layer's own picture cost in the last measured
                  // frame (docs/13 §7.1). A readout, not a control: it neither
                  // selects the layer nor claims the click.
                  TimelineGroup.timings => TimingsCell(
                      layerId: layer.internallayerId.toString(),
                    ),
                },
              ),
            ],
          ],
        ));
  }

  /// Group 1: visibility · audio · solo · lock · shy. The first two swap
  /// their glyph when off — a closed eye, a muted speaker — rather than only
  /// dimming, so the off state reads at a glance.
  ///
  /// **Only what the layer can do** (K-435). The eye is drawn for a layer with
  /// a picture, the speaker for a layer with sound — so an Audio layer has no
  /// eye, and a solid, a title, a shape or an image-only clip has no speaker.
  /// A control that does nothing when clicked is worse than no control: you
  /// have to click it to find out. Each keeps its cell's width either way, so
  /// the switches stay in their columns down the stack and the ones a row does
  /// have sit where the eye reads for them.
  Widget _switchCells(BuildContext context, BridgeLayerInfo info) {
    final id = layer.internallayerId.toString();
    final switches = info.switches;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (widget.hasPicture)
          _switch(context, id, 'visible', null, switches.visible,
              BridgeLayerSwitch.visible,
              mark: LumitIcons.visible,
              offMark: LumitIcons.hidden,
              tip: switches.visible ? l10n.switchVisible : l10n.switchHidden)
        else
          const SizedBox(width: switchCellWidth, height: _rowHeight),
        if (widget.hasAudio)
          _switch(context, id, 'audible', null, switches.audible,
              BridgeLayerSwitch.audible,
              mark: LumitIcons.audio,
              offMark: LumitIcons.muted,
              tip: switches.audible ? l10n.switchAudible : l10n.switchMuted)
        else
          const SizedBox(width: switchCellWidth, height: _rowHeight),
        // A ringed dot, dimmed until soloed — the set has one solo mark, so
        // this pair is told apart by strength rather than by shape.
        _switch(
            context, id, 'solo', null, switches.solo, BridgeLayerSwitch.solo,
            mark: LumitIcons.solo,
            offMark: LumitIcons.solo,
            tip: switches.solo ? l10n.switchSoloed : l10n.switchSolo),
        _switch(context, id, 'locked', null, switches.locked,
            BridgeLayerSwitch.locked,
            mark: LumitIcons.lock,
            offMark: LumitIcons.unlocked,
            tip: switches.locked ? l10n.switchLocked : l10n.switchLock),
        _switch(context, id, 'shy', LumitIcon.shyHidden, switches.shy,
            BridgeLayerSwitch.shy,
            offIcon: LumitIcon.shy,
            tip: switches.shy ? l10n.switchShy : l10n.switchMarkShy),
      ],
    );
  }

  /// Group 2: twirl · label chip · layer number · name.
  Widget _identityCells(
      BuildContext context, LumitTheme t, BridgeLayerInfo info) {
    final id = layer.internallayerId.toString();
    return Row(
      children: [
        // The twirl: the layer's properties, where AE puts them. Its own
        // gesture, so opening a layer does not also select it — you often
        // want to look at one layer's values while another is selected.
        LumitTooltip(
          message: widget.open ? l10n.tipHideProperties : l10n.tipProperties,
          child: _ownClick(GestureDetector(
            key: ValueKey<String>('tl-twirl-$id'),
            behavior: HitTestBehavior.opaque,
            onTap: widget.onToggleOpen,
            child: SizedBox(
              width: 16,
              height: _rowHeight,
              child: Center(
                child: glyph.LumitIcon(
                  widget.open ? LumitIcons.collapse : LumitIcons.expand,
                  size: iconSize,
                  colour: widget.open ? t.textPrimary : t.textMuted,
                ),
              ),
            ),
          )),
        ),
        LumitTooltip(
          message: l10n.tipLabelColour,
          child: _ownClick(_labelSwatch(context, t, id, info.label)),
        ),
        const SizedBox(width: 4),
        SizedBox(
          width: 20,
          child:
              Text('${index + 1}', style: t.small.copyWith(color: t.textMuted)),
        ),
        // The name is also the stack handle: drag it up or down to reorder
        // the layer (docs/07 §4.7). A locked layer holds its place.
        //
        // Selection is the row's, on the pointer down — the rename's
        // double-tap holds the gesture arena open for its whole window, so
        // selecting through a tap made a plain click on the name reach the
        // Effect controls a third of a second late.
        //
        // The drag itself: a plain vertical gesture, not a `Draggable`.
        //
        // A `Draggable` carries a floating copy of the thing being moved,
        // which is why this used to show a little name label under the
        // pointer while the real row stayed behind. Both halves of the
        // table already slide (K-208), so the stack shows the move
        // truthfully on its own — the label was a second, worse answer to
        // a question already answered, and the row it named did not move.
        // The row travels; nothing floats.
        Expanded(
          child: info.switches.locked
              ? _name(t, id, info)
              : GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  supportedDevices: dragDevices,
                  onVerticalDragStart: (_) {
                    _dragTravel = 0;
                    widget.layerDrag.value = LayerDrag(index, index);
                  },
                  onVerticalDragUpdate: (d) {
                    _dragTravel += d.delta.dy;
                    final to = layerDragTarget(
                        widget.blockHeights, index, _dragTravel);
                    final drag = widget.layerDrag.value;
                    if (drag?.to == to && drag?.from == index) return;
                    widget.layerDrag.value = LayerDrag(index, to);
                  },
                  onVerticalDragEnd: (_) => _commitDrag(),
                  onVerticalDragCancel: () => widget.layerDrag.value = null,
                  child: _name(t, id, info),
                ),
        ),
        const SizedBox(width: 4),
      ],
    );
  }

  /// Group 3: flow (collapse on a Precomp) · fx · motion blur · 3D, spread
  /// across the same span the fold-out's value cells use.
  ///
  /// The flow slot is the spec's flow-or-collapse cell (K-168): a Precomp shows
  /// its collapse switch there, **footage shows its Flow switch** (K-088/K-331),
  /// and other kinds leave it empty rather than offering a control that cannot
  /// do anything.
  Widget _renderCells(BuildContext context, BridgeLayerInfo info) {
    final id = layer.internallayerId.toString();
    final switches = info.switches;
    return SizedBox(
      width: renderGroupWidth,
      child: Row(
        children: [
          // Packed left in ordinary switch cells, exactly as group 1 is: the
          // group's remaining span belongs to the fold-out's value column,
          // not to spreading four icons across it.
          if (info.kind == BridgeLayerKind.precomp)
            _switch(context, id, 'collapse', LumitIcon.collapse,
                switches.collapse, BridgeLayerSwitch.collapse,
                tip: l10n.tipCollapseTransformations)
          else if (info.kind == BridgeLayerKind.footage)
            // The Flow cell: shaped exactly like a switch but writing the
            // layer's interpolation policy rather than a `BridgeLayerSwitch`,
            // because that is what flow *is* underneath (K-088: "the option
            // surfaces the policy").
            _switch(context, id, 'flow', LumitIcon.flow, info.flow, null,
                tip: info.flow ? l10n.tipFlowOn : l10n.tipFlowOff, onTap: () {
              layer.setFlowEnabled(on_: !info.flow);
              widget.onChanged();
            })
          else
            const SizedBox(width: switchCellWidth),
          _switch(context, id, 'fx', LumitIcon.fx, switches.fx,
              BridgeLayerSwitch.fx,
              tip: switches.fx
                  ? l10n.switchEffectsOn
                  : l10n.switchEffectsBypassed),
          _switch(context, id, 'mb', LumitIcon.motionBlur, switches.motionBlur,
              BridgeLayerSwitch.motionBlur,
              tip: l10n.switchMotionBlur),
          _switch(context, id, '3d', LumitIcon.cube3d, switches.threeD,
              BridgeLayerSwitch.threeD,
              tip: l10n.switchThreeD),
          // Accepts lights (K-361). The light's own icon, because that is what
          // the switch is about; it does nothing in a comp with no lights, so
          // it costs a glance rather than a decision.
          _switch(context, id, 'lit', LumitIcon.aperture,
              switches.acceptsLights, BridgeLayerSwitch.acceptsLights,
              tip: switches.acceptsLights
                  ? l10n.switchAcceptsLightsOn
                  : l10n.switchAcceptsLightsOff),
        ],
      ),
    );
  }

  /// Group 4: matte · blend · parent, sharing the group's width so dragging
  /// it wider widens the pickers rather than leaving space beside them.
  Widget _composeCells(
      BuildContext context, LumitTheme t, BridgeLayerInfo info, double width) {
    final (matteWidth, blendWidth, parentWidth) = composeCellWidths(width);
    return Row(
      children: [
        LumitTooltip(
          message: l10n.tipMatte,
          child: MattePickerFrb(
            layer: layer,
            info: info,
            all: widget.layers,
            width: matteWidth,
            onChanged: widget.onChanged,
          ),
        ),
        const SizedBox(width: cellGap),
        LumitTooltip(
          message: l10n.tipBlendMode,
          child: _blendPicker(context, t, info.blend, blendWidth),
        ),
        const SizedBox(width: cellGap),
        LumitTooltip(
          message: l10n.tipParent,
          child: ParentPickerFrb(
            layer: layer,
            info: info,
            all: widget.layers,
            width: parentWidth,
            onChanged: widget.onChanged,
          ),
        ),
      ],
    );
  }

  /// The comp a Precomp layer draws, if it is still in the document.
  CompositionReference? _sourceComp() {
    try {
      final source = layer.getSourceItem();
      return source is ItemReference_Composition ? source.field0 : null;
    } catch (_) {
      // A layer that has gone: nothing to open, and never a crash.
      return null;
    }
  }

  /// Double-clicking a layer opens it (K-243). A **Sequence** layer opens its
  /// own view in place — its clips and their speed envelope, inside its row
  /// (K-248) — because cutting is done against the beat you can see, so the
  /// music and the ruler have to stay on screen. A Precomp opens the comp it
  /// draws, the way it does in the Project panel and the Hierarchy; every
  /// other kind will open in a Viewer of its own once there is one to open,
  /// and until then does nothing. It no longer renames — `Enter` does that.
  void _openLayer() {
    if (widget.entry.info.kind == BridgeLayerKind.sequence) {
      widget.onOpenSequence?.call();
      return;
    }
    final comp = _sourceComp();
    if (comp == null) return;
    Provider.of<LumitUiState>(context, listen: false).setSelectedComp(comp);
  }

  /// The name, or the rename editor `Enter` turns it into. Submitting commits;
  /// clicking anywhere else commits too (the field loses the row). A locked
  /// layer's name does not open the editor: lock means no edits.
  Widget _name(LumitTheme t, String id, BridgeLayerInfo info) {
    final editor = _rename;
    if (editor != null) {
      return HouseTextField(
        key: ValueKey<String>('tl-rename-$id'),
        controller: editor,
        autofocus: true,
        onSubmitted: (_) => _commitRename(),
        // Clicking anywhere else finishes the edit and keeps what was typed.
        // It used to leave the field open and lose the change (K-243).
        onTapOutside: _commitRename,
        onCancelled: _cancelRename,
      );
    }
    return GestureDetector(
      key: ValueKey<String>('tl-name-$id'),
      behavior: HitTestBehavior.opaque,
      onDoubleTap: _openLayer,
      child: SizedBox(
        height: _rowHeight,
        child: Align(
          alignment: Alignment.centerLeft,
          child:
              Text(info.name, style: t.body, overflow: TextOverflow.ellipsis),
        ),
      ),
    );
  }

  /// The layer's label colour (TL2): a chip that opens the eight-colour
  /// picker. The palette is the theme's own, so no colour literal lives here.
  Widget _labelSwatch(
      BuildContext context, LumitTheme t, String id, int label) {
    return GestureDetector(
      key: ValueKey<String>('tl-label-$id'),
      behavior: HitTestBehavior.opaque,
      onTapDown: (d) async {
        final picked = await showLumitPopup<int>(
          context: context,
          position: d.globalPosition,
          builder: (close) => FloatSurface(
            child: Padding(
              padding: const EdgeInsets.all(6),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  for (var i = 0; i < LumitTheme.labelCount; i++)
                    GestureDetector(
                      key: ValueKey<String>('tl-label-chip-$i'),
                      onTap: () => close(i),
                      child: Container(
                        width: 14,
                        height: 14,
                        margin: const EdgeInsets.all(2),
                        decoration: BoxDecoration(
                          color: t.labelColour(i),
                          borderRadius:
                              BorderRadius.circular(t.tokens.controlRadius),
                        ),
                      ),
                    ),
                ],
              ),
            ),
          ),
        );
        if (picked == null) return;
        layer.setLabel(label: picked);
        widget.onChanged();
      },
      child: SizedBox(
        width: 16,
        height: _rowHeight,
        child: Center(
          child: Container(
            width: 10,
            height: 10,
            decoration: BoxDecoration(
              color: t.labelColour(label),
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            ),
          ),
        ),
      ),
    );
  }

  /// One switch cell: the icon in a small outlined box, so the click targets
  /// read as buttons rather than loose glyphs. With an [offIcon] the glyph
  /// itself flips (closed eye, muted speaker, hollow circle) and keeps full
  /// strength either way; without one the off state dims, as before.
  /// [onTap] replaces the default `set_switch` write for a cell that only
  /// wears the switch's clothes — the Flow cell, whose write is the layer's
  /// interpolation policy — in which case [which] may be null.
  Widget _switch(
    BuildContext context,
    String id,
    String name,
    LumitIcon? icon,
    bool on,
    BridgeLayerSwitch? which, {
    LumitIcon? offIcon,
    // Lumit's own set (K-440), where it has the mark: [mark]/[offMark] take
    // the place of [icon]/[offIcon] and are drawn from lumit_icons.dart. The
    // Iconoir pair stays for the switches the set has no glyph for yet, so
    // this cell can be ported one column at a time.
    String? mark,
    String? offMark,
    String? tip,
    VoidCallback? onTap,
  }) {
    final t = ThemeScope.of(context).theme;
    final ink = on || offIcon != null || offMark != null
        ? (on ? t.textPrimary : t.textMuted)
        : t.textDisabled;
    final Widget face = mark != null
        ? glyph.LumitIcon(on || offMark == null ? mark : offMark,
            size: iconSize, colour: ink)
        : lumitIcon(on || offIcon == null ? icon! : offIcon,
            size: iconSize, color: ink);
    final cell = GestureDetector(
      key: ValueKey<String>('tl-$name-$id'),
      behavior: HitTestBehavior.opaque,
      onTap: onTap ??
          () {
            layer.setSwitch(switch_: which!, on_: !on);
            widget.onChanged();
          },
      child: SizedBox(
        width: switchCellWidth,
        height: _rowHeight,
        child: Center(
          child: Container(
            width: 18,
            height: 18,
            decoration: BoxDecoration(
              color: t.surface0,
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
              border: Border.all(color: t.hairline),
            ),
            child: Center(child: face),
          ),
        ),
      ),
    );
    return tip == null ? cell : LumitTooltip(message: tip, child: cell);
  }

  Widget _blendPicker(
      BuildContext context, LumitTheme t, int current, double width) {
    final modes = _blendModes ??= listBlendModes();
    // The cell's share of its group: a dropdown that overflows its cell is a
    // layout error, not a cosmetic one, and the label ellipsises to fit.
    return SizedBox(
      width: width,
      child: BareDropdown<int>(
        key: ValueKey<String>('tl-blend-${layer.internallayerId}'),
        value: current < modes.length ? current : 0,
        options: [for (var i = 0; i < modes.length; i++) i],
        label: (i) => engineLabel(modes[i]),
        onChanged: (i) {
          layer.setBlend(index: i);
          widget.onChanged();
        },
      ),
    );
  }

  Future<void> _showRowMenu(BuildContext context, Offset position) async {
    // A locked layer keeps Duplicate — copying is not editing — but its own
    // order and existence are held still until it is unlocked.
    final locked = widget.entry.info.switches.locked;
    final picked = await showMenuAt<String>(
      context: context,
      position: position,
      width: 190,
      rows: (close) => [
        MenuRow(
            onPressed: () => close('duplicate'),
            child: Text(l10n.menuDuplicate)),
        if (!locked) ...[
          if (index > 0)
            MenuRow(
                onPressed: () => close('up'), child: Text(l10n.bringForward)),
          if (index < count - 1)
            MenuRow(
                onPressed: () => close('down'), child: Text(l10n.sendBackward)),
          // In and out of the clip-editing surface, for anyone. The Vegas
          // preference decides what an *import* becomes (K-246), never
          // what a layer is allowed to be — and coming back out is
          // offered wherever going in is, so a user who tries it can
          // change their mind.
          if (widget.entry.info.kind == BridgeLayerKind.footage)
            MenuRow(
                key: const ValueKey('tl-row-to-sequence'),
                onPressed: () => close('to-sequence'),
                child: Text(l10n.menuConvertToSequenceLayer)),
          if (widget.entry.info.kind == BridgeLayerKind.sequence)
            MenuRow(
                key: const ValueKey('tl-row-from-sequence'),
                onPressed: () => close('from-sequence'),
                child: Text(l10n.menuConvertToFootageLayer)),
        ],
        // The shape — the cuts, the gaps and the ramps, with no media in
        // it — from the layer itself, so carrying a cut onto a depth pass
        // never needs either row opened first (K-248). Offered on a locked
        // layer too: copying is not editing.
        if (widget.entry.info.kind == BridgeLayerKind.sequence) ...[
          MenuRow(
              key: const ValueKey('tl-row-copy-shape'),
              onPressed: () => close('copy-shape'),
              child: Text(l10n.copySequenceShape)),
          if (!locked && sequenceShapeClipboard != null)
            MenuRow(
                key: const ValueKey('tl-row-paste-shape'),
                onPressed: () => close('paste-shape'),
                child: Text(l10n.pasteSequenceShape)),
        ],
        if (!locked) ...[
          MenuRow(onPressed: () => close('delete'), child: Text(l10n.delete)),
        ],
        // Only when there is something to clear. A layer carries markers
        // when a composition was dropped in with some (K-254); most layers
        // have none and should not be offered a command that does nothing.
        if (!locked && widget.entry.info.markers.isNotEmpty)
          MenuRow(
              key: const ValueKey('tl-row-clear-markers'),
              onPressed: () => close('clear-markers'),
              child: Text(l10n.deleteAllMarkers)),
      ],
    );
    switch (picked) {
      case 'duplicate':
        layer.duplicate();
      case 'up':
        layer.reorder(newIndex: BigInt.from(index - 1));
      case 'down':
        layer.reorder(newIndex: BigInt.from(index + 1));
      case 'delete':
        layer.delete();
      case 'clear-markers':
        layer.setMarkers(markers: const []);
      case 'to-sequence':
        layer.convertToSequenced();
      case 'from-sequence':
        // A row of several clips refuses: which one the layer would become is
        // the user's decision, not the command's, and the engine says so.
        try {
          layer.convertFromSequenced();
        } catch (_) {
          return;
        }
      case 'copy-shape':
        try {
          sequenceShapeClipboard = layer.copySequenceShape();
        } catch (_) {}
        return; // nothing changed in the document
      case 'paste-shape':
        final shape = sequenceShapeClipboard;
        if (shape == null) return;
        try {
          layer.pasteSequenceShape(text: shape);
        } catch (_) {
          return;
        }
      case _:
        return;
    }
    widget.onChanged();
  }
}

/// The right column: the ruler, the playhead, and one bar per layer.
class _LayerArea extends StatelessWidget {
  final CompositionReference comp;

  /// The layers as the panel decided them — the same [LayerRow] list the
  /// outline draws from. Which rows a layer shows, whether its Sequence view
  /// is open and how tall the block is are read here rather than worked out
  /// again, so this half cannot come to a different answer from the other.
  final List<LayerRow> rows;

  /// The selection as ids, the same set the outline draws from (K-217) — a bar
  /// outlines when its name row is lit, so the two halves of the table never
  /// disagree about what is chosen.
  final Set<UuidValue> selectedIds;

  /// Open or close a Sequence layer's view (K-248).
  final void Function(BridgeLayerEntry entry)? onOpenSequence;

  /// The graph half of a sequence view has been dragged to a new height.
  final void Function(BridgeLayerEntry entry, double height)? onGraphHeight;

  /// A clip's envelope is being dragged: show it under the map it has not
  /// been given yet (K-247).
  final void Function(
          BridgeLayerEntry entry, BridgeClip clip, List<BridgeKeyframe> keys)?
      onClipPreview;

  /// The lane's horizontal scroll, so an open view's axis labels can sit at
  /// the window's edge rather than at the start of time.
  final ScrollController? hScroll;

  /// Where the open sequence views sit, so the row seams skip them (K-248).
  final List<(double, double)> sequenceBlanks;

  /// Each layer's source peaks, for the waveform lanes.
  final Map<String, BridgeAudioPeaks> peaks;

  /// How waveforms draw (K-280, K-285) — the lanes' own answer, handed down so
  /// an open Sequence view's clips agree with it.
  final WaveformStyle waveformStyle;

  /// The comp's rate, mapping the lane's pixels onto source seconds.
  final double fps;
  final TimelineAxis axis;

  /// Listened to, not read: only the playhead line moves when it changes.
  final ValueListenable<int> playhead;
  final bool razor;

  /// A razor click on a bar: which layer, and the frame under the pointer.
  final void Function(BridgeLayerEntry entry, int frame) onRazor;
  final ValueChanged<int> onSeek;

  /// Clicking a bar is clicking the layer: the lane side selects too.
  final ValueChanged<LayerReference> onSelect;
  final VoidCallback onChanged;

  /// Fires when something may have changed which frames are held — a frame
  /// arriving or the idle fill banking one — so the bar repaints then rather
  /// than polling the cache on every frame it draws.
  final Listenable cacheRevision;

  /// The bar drag in flight, written by the bars and read by the waveform
  /// lanes, so the peaks move with the gesture rather than on release.
  final ValueNotifier<BarDragPreview?> dragPreview;

  /// How far each layer's ends may be dragged, by layer id (K-211). A layer
  /// with no entry has free ends — the honest answer while a source length is
  /// still being read.
  final Map<String, BarBounds> bounds;

  /// The lanes' vertical scroll — the outline mirrors it, and the thumb in
  /// the gutter beside this area is the one the user grabs.
  final ScrollController vScroll;

  /// The marquee's keyframe selection, as `rowId#index`, and where a new box
  /// reports what it caught.
  final Set<String> selectedKeys;
  final ValueChanged<Set<String>> onKeysSelected;

  /// A click on empty lane space — no bar, no diamond, no drag. Everything
  /// lets go (K-203).
  final VoidCallback onDeselectAll;

  /// The work area in frames, read once by the panel (K-203).
  final ({int start, int end, bool whole}) work;

  /// The layer drag in flight, and the block heights it slides by — the
  /// outline makes the gesture, and these are what let this side move with it
  /// rather than sit still while its layers are reordered (K-208).
  final ValueNotifier<LayerDrag?> layerDrag;
  final List<double> blockHeights;

  /// The comp's exact rate, for the times a key drag commits.
  final int fpsNum;
  final int fpsDen;

  /// Whether a dragged keyframe sticks to whole frames (docs/07 §4.5).
  final bool magnet;

  /// A wheel over the lanes, with the pointer's position in *content* space
  /// (so the zoom can hold the frame under the cursor still). Plain wheels
  /// are left alone, so they still reach the scrollable.
  final void Function(PointerScrollEvent event, double contentX) onWheel;

  const _LayerArea({
    required this.comp,
    required this.rows,
    required this.selectedIds,
    this.onOpenSequence,
    this.onGraphHeight,
    this.sequenceBlanks = const [],
    this.hScroll,
    this.onClipPreview,
    required this.peaks,
    required this.waveformStyle,
    required this.fps,
    required this.axis,
    required this.playhead,
    required this.razor,
    required this.onRazor,
    required this.onSeek,
    required this.onSelect,
    required this.onChanged,
    required this.cacheRevision,
    required this.dragPreview,
    required this.bounds,
    required this.vScroll,
    required this.selectedKeys,
    required this.onKeysSelected,
    required this.onDeselectAll,
    required this.work,
    required this.layerDrag,
    required this.blockHeights,
    required this.fpsNum,
    required this.fpsDen,
    required this.magnet,
    required this.onWheel,
  });

  /// Every keyframe the box caught, walking the same rows the lanes draw —
  /// y from the row stack, x from the key's frame on the axis.
  Set<String> _keysIn(Rect rect) {
    final caught = <String>{};
    var y = 0.0;
    for (final layer in rows) {
      y += _rowHeight; // the layer's own bar row
      for (final row in layer.drawnRows) {
        final rowTop = y;
        y += _rowHeight;
        if (rowTop + _rowHeight < rect.top || rowTop > rect.bottom) continue;
        final keys = laneKeysOf(row);
        for (var i = 0; i < keys.length; i++) {
          final x = axis.xOf(laneKeyFrame(keys[i], fps));
          if (x >= rect.left && x <= rect.right) {
            caught.add('${foldRowPath(layer.id, row)}#$i');
          }
        }
      }
    }
    return caught;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Where the work area falls in this area's own pixels, or null when it
    // covers the whole comp — in which case there is no out-of-range ground to
    // wash and the strip stays one colour.
    final workAreaPixels =
        work.whole ? null : (axis.xOf(work.start), axis.xOf(work.end));
    // Gathered once for the whole area, not once per lane (docs/07 §4.5).
    final snap = _snapTargets();
    // Where a razor cut lands, as a frame — the *one* answer the blade's line
    // and the cut itself both read, so the mark cannot stand anywhere but
    // where the edge bites. The cut was always quantised (`frameAt` rounds);
    // it is the line that used to follow the pointer between frames.
    double razorFrameAt(double x) => snapFrame(
          frame: axis.frameAtExact(x),
          targets: snap,
          perFrame: axis.perFrame,
          magnet: magnet &&
              !snapSuspended(
                  controlPressed: HardwareKeyboard.instance.isControlPressed),
        )
            // A cut is a clip boundary, and a clip boundary is a whole frame —
            // so even a snap onto a target that sits between frames (a keyframe
            // may) lands on one. Rounding here rather than at the cut is what
            // keeps the drawn line and the edge exactly the same place.
            .frame
            .roundToDouble();
    // The blade pointer and the line that says where the cut lands (K-220).
    // Round the whole area rather than inside a bar: the line spans every row,
    // and a pointer clipped to one bar would vanish at its edges. Inert — and
    // free — while the razor is not armed.
    return RazorOverlay(
        active: razor,
        snapX: (x) => axis.xOf(razorFrameAt(x)),
        mark: t.textPrimary,
        outline: t.surface0,
        child: Stack(
          children: [
            Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                TimelineRuler(
                  comp: comp,
                  axis: axis,
                  fps: fps,
                  height: _rulerHeight,
                  work: work,
                  onSeek: onSeek,
                  onWorkArea: (span) {
                    comp.setWorkArea(span: span);
                    onChanged();
                  },
                  onMarkersChanged: onChanged,
                ),
                // Directly under the ruler and above the lanes, which is where the
                // interface spec puts it (docs/07 §3.2).
                CacheStrip(
                    comp: comp,
                    axis: axis,
                    revision: cacheRevision,
                    work: workAreaPixels),
                // The rows scroll under the pinned ruler, in step with the
                // outline; the thumb lives in the gutter beside this area, so it
                // stays pinned to the viewport's edge rather than riding the
                // horizontally-scrolled content (docs/07 §4.6).
                Expanded(
                    // The rows are given at least the viewport's height, so the
                    // ground, the row seams and the marquee carry on below the last
                    // layer rather than stopping at it: a lane area that ran out of
                    // rows half way down read as a hole in the table, and a click in
                    // that hole reached nothing to deselect against.
                    child: LayoutBuilder(
                  builder: (context, box) => SingleChildScrollView(
                    controller: vScroll,
                    child: ConstrainedBox(
                      constraints: BoxConstraints(minHeight: box.maxHeight),
                      // Innermost, so the pointer-signal resolver hands it
                      // the wheel before the scrollables do — a modified
                      // wheel zooms or pans instead of scrolling, and a
                      // plain one is left alone.
                      child: Listener(
                        // A *modified* wheel is claimed through the resolver, so the
                        // scroll views around this one cannot act on the same event
                        // as well — a Ctrl+wheel zoom that also scrolled the lanes
                        // sideways is what an unclaimed signal looks like. A plain
                        // wheel is deliberately left unregistered: it belongs to the
                        // scrollable, which is what moves the rows (docs/07 §4.6).
                        onPointerSignal: (event) {
                          if (event is! PointerScrollEvent) return;
                          final keys = HardwareKeyboard.instance;
                          if (!keys.isControlPressed && !keys.isShiftPressed) {
                            return;
                          }
                          GestureBinding.instance.pointerSignalResolver
                              .register(event, (resolved) {
                            if (resolved is PointerScrollEvent) {
                              onWheel(resolved, resolved.localPosition.dx);
                            }
                          });
                        },
                        child: Stack(
                          children: [
                            // The ground, in two shades (K-202): the work area keeps
                            // the panel's own surface, and everything outside it is
                            // washed a step darker. Without it the lane area was one
                            // long strip at a single value, which left a selected
                            // row almost nothing to stand out against — and left the
                            // span you are actually delivering invisible below the
                            // ruler.
                            Positioned.fill(
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: WorkAreaGroundPainter(
                                    startX: workAreaPixels?.$1,
                                    endX: workAreaPixels?.$2,
                                    // The lower reach of the one band the
                                    // ruler starts (§12A.1) — behind the bars,
                                    // the keys and the marquee, because it is
                                    // the ground they stand on.
                                    inside: Color.alphaBlend(
                                        t.animated.withValues(
                                            alpha: workAreaLaneFillAlpha),
                                        t.surface1),
                                    outside: t.timelineOutOfRange,
                                    edge: workAreaEdgeColour(t),
                                  ),
                                ),
                              ),
                            ),
                            // Behind the bars: dragging empty lane space boxes up
                            // keyframes (docs/07 §4.3); bars and key handles above
                            // still win their own gestures.
                            Positioned.fill(
                              child: MarqueeSelect(
                                key: const ValueKey('tl-lane-marquee'),
                                onSelect: (rect) =>
                                    onKeysSelected(_keysIn(rect)),
                                // A click that caught nothing is a click on empty
                                // lane space, which is the deselect gesture: the
                                // bars and the key handles above take their own
                                // taps, so only the ground reaches here.
                                onClear: onDeselectAll,
                              ),
                            ),
                            Column(
                              crossAxisAlignment: CrossAxisAlignment.stretch,
                              children: [
                                for (var i = 0; i < rows.length; i++)
                                  // The block slides by the same rule and
                                  // the same heights the outline's does, so
                                  // a layer dragged up the stack takes its
                                  // bar and its lanes with it (K-208).
                                  LayerDragSlide(
                                    drag: layerDrag,
                                    heights: blockHeights,
                                    index: i,
                                    child: Container(
                                      // One outline around the layer's own
                                      // bar row and everything its open
                                      // view added, so it reads as one
                                      // region belonging to one layer
                                      // rather than as loose strips that
                                      // happen to sit under it (K-248).
                                      // A *foreground* decoration: a
                                      // border in the ordinary one insets
                                      // its child, which made the lane's
                                      // block two pixels taller than the
                                      // outline reserved and put the two
                                      // halves back out of step. This
                                      // paints over the content and
                                      // occupies no layout at all.
                                      foregroundDecoration:
                                          rows[i].sequenceExtra != null
                                              ? BoxDecoration(
                                                  border: Border.all(
                                                    color: t.accent
                                                        .withValues(alpha: 0.5),
                                                  ),
                                                  borderRadius:
                                                      BorderRadius.circular(3),
                                                )
                                              : null,
                                      child: Column(
                                        crossAxisAlignment:
                                            CrossAxisAlignment.stretch,
                                        children: [
                                          // An open sequence view takes the
                                          // layer's own bar row as the top
                                          // of its clip area, so the bar
                                          // itself stands down: three rows
                                          // of clips is one region, and a
                                          // bar drawn across the first of
                                          // them would put a seam through
                                          // the middle of it (K-248).
                                          if (rows[i].sequenceExtra == null)
                                            _Bar(
                                              key: ValueKey<String>(
                                                  'tl-bar-${rows[i].id}'),
                                              comp: comp,
                                              entry: rows[i].entry,
                                              axis: axis,
                                              razor: razor,
                                              selected: selectedIds.contains(
                                                  rows[i]
                                                      .entry
                                                      .layer
                                                      .internallayerId),
                                              playheadFrame: () =>
                                                  playhead.value,
                                              onRazor: (frame) =>
                                                  onRazor(rows[i].entry, frame),
                                              razorFrameAt: razorFrameAt,
                                              onSelect: () =>
                                                  onSelect(rows[i].entry.layer),
                                              onOpenSequence: rows[i]
                                                          .entry
                                                          .info
                                                          .kind ==
                                                      BridgeLayerKind.sequence
                                                  ? () => onOpenSequence
                                                      ?.call(rows[i].entry)
                                                  : null,
                                              onChanged: onChanged,
                                              dragPreview: dragPreview,
                                              bounds: bounds[rows[i].id] ??
                                                  BarBounds.free,
                                              summaryKeys: rows[i].summaryKeys,
                                              fps: fps,
                                            ),
                                          // A Sequence layer's own clips and
                                          // their speed envelope, in the room
                                          // the row grew for them (K-248) —
                                          // the same `sequenceExtra` the
                                          // outline left the gap for, so the
                                          // view and its room are one answer.
                                          if (rows[i].sequenceExtra != null)
                                            SequenceViewFrb(
                                              key: ValueKey<String>(
                                                  'tl-seq-${rows[i].id}'),
                                              entry: rows[i].entry,
                                              axis: axis,
                                              fps: fps,
                                              fpsNum: fpsNum,
                                              fpsDen: fpsDen,
                                              hScroll: hScroll,
                                              style: waveformStyle,
                                              razor: razor,
                                              onRazor: (frame) =>
                                                  onRazor(rows[i].entry, frame),
                                              razorFrameAt: razorFrameAt,
                                              onSelect: () =>
                                                  onSelect(rows[i].entry.layer),
                                              onClose: () => onOpenSequence
                                                  ?.call(rows[i].entry),
                                              graphHeight:
                                                  (rows[i].sequenceExtra ??
                                                          sequenceViewHeight) -
                                                      sequenceClipExtra,
                                              onGraphHeight: (h) =>
                                                  onGraphHeight?.call(
                                                      rows[i].entry, h),
                                              onPreview: (clip, keys) =>
                                                  onClipPreview?.call(
                                                      rows[i].entry,
                                                      clip,
                                                      keys),
                                              onChanged: onChanged,
                                            ),
                                          // One lane per fold-out row the outline shows,
                                          // from the same list it builds: keyframe rows
                                          // draw their diamonds, the waveform row its
                                          // peaks (K-172), the rest leave their room.
                                          if (rows[i].open)
                                            Column(
                                              key: ValueKey<String>(
                                                  'tl-lanes-${rows[i].id}'),
                                              children: [
                                                for (final row
                                                    in rows[i].drawnRows)
                                                  SizedBox(
                                                    height: _rowHeight,
                                                    child: _lane(
                                                        t,
                                                        rows[i].entry,
                                                        row,
                                                        snap),
                                                  ),
                                              ],
                                            ),
                                        ],
                                      ),
                                    ),
                                  ),
                              ],
                            ),
                            // The same wash again, over the bars this time: under
                            // them it was invisible along any row that had a layer
                            // in it, which is exactly the rows being looked at. Kept
                            // light, so what is out of range is dimmed rather than
                            // hidden.
                            if (workAreaPixels != null)
                              Positioned.fill(
                                child: IgnorePointer(
                                  child: CustomPaint(
                                    painter: WorkAreaGroundPainter(
                                      startX: workAreaPixels.$1,
                                      endX: workAreaPixels.$2,
                                      inside: t.surface1.withValues(alpha: 0),
                                      outside: t.timelineOutOfRange
                                          .withValues(alpha: 0.55),
                                    ),
                                  ),
                                ),
                              ),
                            // The row hairlines, over everything and touching
                            // nothing (K-190): they run the full width of the lane
                            // area so the eye can track a row across the table,
                            // and they are drawn rather than given to each row as
                            // a border because a decorated box absorbs pointers —
                            // which would eat the marquee underneath.
                            Positioned.fill(
                              child: IgnorePointer(
                                child: CustomPaint(
                                  painter: _RowDividerPainter(
                                    step: _rowHeight,
                                    colour: t.hairline,
                                    blanks: sequenceBlanks,
                                  ),
                                ),
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                )),
              ],
            ),
            // The playhead rides above every bar so it is never hidden behind one,
            // and it is the only thing here that redraws when it moves.
            ValueListenableBuilder<int>(
              valueListenable: playhead,
              builder: (context, frame, child) => Positioned(
                left: axis.xOf(frame) - PlayheadMarker.halfWidth,
                top: 0,
                bottom: 0,
                child: child!,
              ),
              child: const PlayheadMarker(),
            ),
          ],
        ));
  }

  /// One fold row's lane: diamonds for a keyed property, the waveform for
  /// the waveform row, empty room otherwise.
  /// Everything a lane key can land on (docs/07 §4.5), built once for the
  /// panel from the read model and the memoised marker list — so it costs no
  /// bridge calls, and no lane pays for another lane's targets.
  List<SnapTarget> _snapTargets() => snapTargetsOf(
        layers: [for (final row in rows) row.entry],
        compMarkers: markersOf(comp),
        keyRows: [
          for (final layer in rows)
            for (final row in layer.foldRows)
              (
                rowId: foldRowPath(layer.id, row),
                frames: [
                  for (final k in laneKeysOf(row)) laneKeyFrame(k, fps),
                ],
              ),
        ],
        playheadFrame: playhead.value,
        work: work,
        fps: fps,
      );

  Widget? _lane(LumitTheme t, BridgeLayerEntry entry, LayerFoldRow row,
      List<SnapTarget> snapTargets) {
    final id = entry.layer.internallayerId.toString();
    if (row is FoldWaveformRow) {
      return ValueListenableBuilder<BarDragPreview?>(
        valueListenable: dragPreview,
        builder: (context, preview, _) {
          final p = preview?.layerId == id ? preview : null;
          final span = entry.info.span;
          // The span as drawn — the document's frames plus any drag in flight —
          // and where its source starts, so a bar being dragged or trimmed
          // carries its transients with it in realtime (K-172).
          final inFrame = entry.info.inFrame.toInt() + (p?.deltaIn ?? 0);
          final outFrame = entry.info.outFrame.toInt() + (p?.deltaOut ?? 0);
          final startOffset =
              rationalSeconds(span.startOffset) + (p?.offsetShift ?? 0) / fps;
          final secondsPerPixel =
              axis.perFrame <= 0 || fps <= 0 ? 0.0 : 1 / (axis.perFrame * fps);
          return CustomPaint(
            key: ValueKey<String>('tl-wave-$id'),
            size: Size(axis.width, _rowHeight),
            painter: WaveformPainter(
              peaks: peaks[id],
              // Canvas x 0 is the axis's left padding, comp time 0 sits a
              // padding's width in, and the source's own clock runs from there
              // less wherever the layer starts it.
              originSeconds: -startOffset - TimelineAxis.pad * secondsPerPixel,
              secondsPerPixel: secondsPerPixel,
              left: axis.xOf(inFrame),
              right: axis.xOf(outFrame),
              colours: t.waveform,
              style: waveformStyle,
              // Both rows (K-437): the lane's own, and the empty one belonging
              // to the **Waveform** twirl directly above it. A centred wave
              // then sits on the divider between the two rather than inside
              // half of one, and a wave rising from the floor has the pair to
              // rise through. The paint reaches up; the row does not grow, so
              // the outline and the lanes stay level.
              height: _rowHeight * 2,
            ),
          );
        },
      );
    }
    final keys = laneKeysOf(row);
    if (keys.isEmpty) return null;
    final rowId = foldRowPath(id, row);
    return _KeyLane(
      key: ValueKey<String>('tl-keys-$rowId'),
      entry: entry,
      row: row,
      rowId: rowId,
      keys: keys,
      axis: axis,
      fps: fps,
      fpsNum: fpsNum,
      fpsDen: fpsDen,
      magnet: magnet,
      snapTargets: snapTargets,
      selectedKeys: selectedKeys,
      onSelectKey: (index, additive) {
        final id = '$rowId#$index';
        // A copy, never the live set: `onKeysSelected` clears it before it
        // reads what it was handed.
        final next = <String>{...selectedKeys};
        if (additive) {
          if (!next.remove(id)) next.add(id);
        } else {
          next
            ..clear()
            ..add(id);
        }
        onKeysSelected(next);
      },
      onChanged: onChanged,
    );
  }
}

/// One keyed property's lane: its keyframes as diamonds, each draggable in
/// time.
///
/// With the magnet on, a drag lands on whole frames; with it off the key may
/// sit *between* frames (docs/07 §4.5) — the times are exact rationals either
/// way. The gesture holds its offset in Dart and commits once on release, so
/// a drag is one undo step; a move onto a neighbour is refused and the key
/// simply stays where it was.
class _KeyLane extends StatefulWidget {
  final BridgeLayerEntry entry;
  final LayerFoldRow row;
  final String rowId;
  final List<BridgeKeyframe> keys;
  final TimelineAxis axis;
  final double fps;
  final int fpsNum;
  final int fpsDen;
  final bool magnet;

  /// Everything on the Timeline this lane's keys may land on (docs/07 §4.5),
  /// gathered once for the panel and handed down — the list is the same for
  /// every lane, so building it per lane would be the same work many times.
  /// This lane's own keys are already left out of it.
  final List<SnapTarget> snapTargets;
  final Set<String> selectedKeys;

  /// Click a diamond to select it — the second way into the key selection the
  /// F9 family and the easing buttons act on, beside the marquee. Additive
  /// (Shift, Ctrl) toggles one in or out of the catch.
  final void Function(int index, bool additive) onSelectKey;
  final VoidCallback onChanged;

  const _KeyLane({
    super.key,
    required this.entry,
    required this.row,
    required this.rowId,
    required this.keys,
    required this.axis,
    required this.fps,
    required this.fpsNum,
    required this.fpsDen,
    required this.magnet,
    required this.snapTargets,
    required this.selectedKeys,
    required this.onSelectKey,
    required this.onChanged,
  });

  @override
  State<_KeyLane> createState() => _KeyLaneState();
}

class _KeyLaneState extends State<_KeyLane> {
  int? _dragging;

  /// Pixels the gesture has moved. The frame offset is always derived from
  /// this running total rather than summed per event, for the same reason the
  /// bar drag does it: per-event rounding reads as mouse acceleration.
  double _deltaPx = 0;

  /// What the drag in flight last landed on, so the capture can be drawn. The
  /// spec requires the target to be indicated at the moment it takes the drag —
  /// without it a key that jumps reads as a fault rather than a service.
  SnapTarget? _caught;

  /// Where key [i] draws — its own time, plus the drag in flight, snapped.
  double _frameOf(int i) {
    final base = laneKeyFrame(widget.keys[i], widget.fps);
    if (_dragging != i) return base;
    final perFrame = widget.axis.perFrame;
    final moved = perFrame <= 0 ? base : base + _deltaPx / perFrame;
    final clamped = moved.clamp(0.0, widget.axis.frames.toDouble());
    final own = {
      for (final k in widget.keys) laneKeyFrame(k, widget.fps),
    };
    final snapped = snapFrame(
      frame: clamped,
      // This lane's own keys are dropped: a key snapping to itself would be
      // pinned where it started, and a neighbour already on the same frame is
      // not a place worth being taken to either.
      targets: widget.snapTargets
          .where((t) => t.kind != SnapKind.keyframe || !own.contains(t.frame)),
      perFrame: perFrame,
      // `Ctrl` held suspends snapping for as long as it is held, which is the
      // way out when the wanted place is exactly where a snap will not allow.
      magnet: widget.magnet &&
          !snapSuspended(
              controlPressed: HardwareKeyboard.instance.isControlPressed),
    );
    _caught = snapped.caught;
    return snapped.frame;
  }

  void _commit(int index) {
    final frame = _frameOf(index);
    setState(() {
      _dragging = null;
      _deltaPx = 0;
      _caught = null;
    });
    if (frame == laneKeyFrame(widget.keys[index], widget.fps)) return;
    final moved = moveLaneKey(
      entry: widget.entry,
      row: widget.row,
      index: index,
      time: timeOfSubframe(frame, widget.fpsNum, widget.fpsDen),
    );
    if (moved) widget.onChanged();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Worked out once for the build: [_frameOf] is where the snap is decided
    // and where [_caught] is set, so asking it twice per key would answer the
    // same question twice and leave the indicator depending on which of the
    // two calls ran last.
    final frames = [for (var i = 0; i < widget.keys.length; i++) _frameOf(i)];
    final caught = _caught;
    // **Every child of this Stack carries a key**, and the keys stay the same
    // whether or not a snap has been caught.
    //
    // Without them the drag died the moment a snap first took it, which read as
    // "a lane key can only be dragged one frame, and dragging again puts it
    // back". A child appearing part-way down an unkeyed list makes Flutter pair
    // each new child with the *old* child in that slot — the indicator was
    // matched to the first diamond, the first diamond to the second, and so on —
    // so the diamonds' gesture detectors were torn down and rebuilt mid-gesture.
    // A recogniser destroyed while it holds a pointer ends its drag, which
    // committed the two or three pixels travelled so far and left the rest of
    // the gesture doing nothing. Keyed, each child is matched to itself, the
    // detector holding the pointer lives, and the drag runs to the release.
    return Stack(
      children: [
        Positioned.fill(
          key: const ValueKey<String>('tl-lane-diamonds'),
          child: CustomPaint(
            painter: _LaneKeysPainter(
              frames: frames,
              selected: {
                for (var i = 0; i < widget.keys.length; i++)
                  if (widget.selectedKeys.contains('${widget.rowId}#$i')) i,
              },
              axis: widget.axis,
              colour: t.animated,
              chosen: t.textPrimary,
            ),
          ),
        ),
        // What the drag landed on, marked while it holds it (docs/07 §4.5:
        // the snapped-to target MUST be indicated at the moment of capture).
        if (caught != null)
          Positioned(
            key: const ValueKey<String>('tl-lane-snap-caught'),
            left: widget.axis.xOf(caught.frame) - 0.5,
            top: 0,
            bottom: 0,
            width: 1,
            child: IgnorePointer(
              child: ColoredBox(color: t.accent),
            ),
          ),
        for (var i = 0; i < widget.keys.length; i++)
          Positioned(
            key: ValueKey<String>('tl-key-slot-${widget.rowId}#$i'),
            left: widget.axis.xOf(frames[i]) - 6,
            top: 0,
            width: 12,
            height: _rowHeight,
            child: MouseRegion(
              cursor: SystemMouseCursors.resizeLeftRight,
              child: GestureDetector(
                key: ValueKey<String>('tl-key-${widget.rowId}#$i'),
                behavior: HitTestBehavior.opaque,
                // Touching a diamond selects it, and a drag is a touch that
                // went somewhere — so the drag's own start is where selection
                // belongs. This recognizer is alone in the arena, which means
                // it wins on release even when the pointer never moved: one
                // callback covers the click and the drag, and no second
                // recognizer competes for the sub-pixel-per-frame movements a
                // lane drag is made of. Without a per-key selection only the
                // marquee could fill the lane catch, so easing one key from
                // the lanes (F9, the bottom bar's buttons) had nothing to act
                // on and looked like it did nothing.
                supportedDevices: dragDevices,
                onHorizontalDragStart: (_) {
                  final keyboard = HardwareKeyboard.instance;
                  widget.onSelectKey(
                    i,
                    keyboard.isShiftPressed ||
                        keyboard.isControlPressed ||
                        keyboard.isMetaPressed,
                  );
                  setState(() {
                    _dragging = i;
                    _deltaPx = 0;
                  });
                },
                onHorizontalDragUpdate: (d) =>
                    setState(() => _deltaPx += d.delta.dx),
                onHorizontalDragEnd: (_) => _commit(i),
                onHorizontalDragCancel: () => setState(() {
                  _dragging = null;
                  _deltaPx = 0;
                }),
              ),
            ),
          ),
      ],
    );
  }
}

/// The lane area's row seams: one hairline per row, the full width of the
/// area (K-190).
///
/// Drawn as one overlay rather than given to each row as a border because a
/// decorated box absorbs pointers — a border per row would quietly eat the
/// keyframe marquee under it — and because the bars fill their whole row, so
/// the seam has to land on top of them to be seen at all.
class _RowDividerPainter extends CustomPainter {
  final double step;
  final Color colour;

  /// Vertical stretches to leave alone, as (top, bottom) pairs.
  ///
  /// An open sequence view is one table cell, not six rows of one — ruling it
  /// into rows drew lines through the clips and straight across the middle of
  /// the speed envelope, which read as the graph having been chopped up
  /// (K-248).
  final List<(double, double)> blanks;

  /// How far the first seam sits above the top edge — the outline's overlay
  /// is pinned to the panel rather than to the scrolled rows, so it carries
  /// the scroll offset here instead.
  final double phase;

  const _RowDividerPainter({
    required this.step,
    required this.colour,
    this.phase = 0,
    this.blanks = const [],
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (step <= 0) return;
    final paint = Paint()
      ..color = colour
      ..strokeWidth = 1;
    for (var y = phase + step; y <= size.height; y += step) {
      if (y < 0) continue;
      // Strictly inside a blank, so the seams that *bound* an open view stay:
      // the row still has a top and a bottom, it simply has no rules through
      // its middle.
      if (blanks.any((b) => y > b.$1 + 0.5 && y < b.$2 - 0.5)) continue;
      canvas.drawLine(Offset(0, y - 0.5), Offset(size.width, y - 0.5), paint);
    }
  }

  /// The blanks are compared **by value**, not by identity: they are rebuilt
  /// fresh on every build, so an identity test said "changed" every time and
  /// both overlays repainted whatever had actually moved (K-293). The list is
  /// one entry per open sequence view, so comparing it is nothing.
  @override
  bool shouldRepaint(_RowDividerPainter old) =>
      old.step != step ||
      old.colour != colour ||
      old.phase != phase ||
      !listEquals(old.blanks, blanks);

  /// Never absorbs a pointer: a background painter's default would eat the
  /// gestures on the rows below it.
  @override
  bool? hitTest(Offset position) => false;
}

/// A lane's keyframe diamonds: one per key, in `animated` (§3.1) — the token
/// that means "this is animated or in hand" — and `text_primary` for the ones
/// the marquee has hold of. Neither is `accent`: the accent's job list is the
/// playhead, the one filled button and the active tab tick, and nothing else.
class _LaneKeysPainter extends CustomPainter {
  /// Fractional, so a key placed between frames draws between them.
  final List<double> frames;
  final Set<int> selected;
  final TimelineAxis axis;
  final Color colour;
  final Color chosen;

  /// Half a diamond's width. [_keyHalf] on a property's own lane; half of
  /// that on a shut layer's row, where the diamonds are a summary rather than
  /// the things you drag (§12A.1).
  final double half;

  const _LaneKeysPainter({
    required this.frames,
    required this.selected,
    required this.axis,
    required this.colour,
    required this.chosen,
    this.half = _keyHalf,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final mid = size.height / 2;
    for (var i = 0; i < frames.length; i++) {
      final x = axis.xOf(frames[i]);
      canvas.drawPath(
        Path()
          ..moveTo(x, mid - half)
          ..lineTo(x + half, mid)
          ..lineTo(x, mid + half)
          ..lineTo(x - half, mid)
          ..close(),
        Paint()..color = selected.contains(i) ? chosen : colour,
      );
    }
  }

  @override
  bool shouldRepaint(_LaneKeysPainter old) =>
      !listEquals(old.frames, frames) ||
      !setEquals(old.selected, selected) ||
      old.colour != colour ||
      old.chosen != chosen ||
      old.half != half ||
      old.axis.frames != axis.frames ||
      old.axis.width != axis.width;

  /// A background painter's default is to absorb hits across its whole rect,
  /// which would eat the keyframe marquee underneath (the diamonds are picked
  /// up by the box, not clicked).
  @override
  bool? hitTest(Offset position) => false;
}

/// The outline's end of the bottom bar (K-448, §12A.1): one kicker per
/// column group, lit while that group is drawn.
///
/// Kickers rather than buttons because these name *containers* (§7.1) — they
/// are the same words the column headers carry, and clicking one takes its
/// columns away so the outline pares down to names and bars. Nothing here
/// touches the document: it is what this panel shows, and it lives as long as
/// the session does.
class _ColumnToggles extends StatelessWidget {
  final List<TimelineGroup> groups;
  final Set<TimelineGroup> hidden;
  final ValueChanged<TimelineGroup> onToggle;

  const _ColumnToggles({
    required this.groups,
    required this.hidden,
    required this.onToggle,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      height: _laneBottomBarHeight,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 8),
      // Scrolls sideways when the outline is narrow — the same answer the
      // toolbar and the lane bar give; an overflow stripe is a layout fault.
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: Row(
          children: [
            for (final group in groups) ...[
              LumitTooltip(
                message: l10n.tipToggleColumns(columnGroupLabel(group)),
                child: HouseButton(
                  key: ValueKey<String>('tl-column-${group.name}'),
                  small: true,
                  frameless: true,
                  padding:
                      const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
                  onPressed: () => onToggle(group),
                  child: Text(
                    columnGroupLabel(group).toUpperCase(),
                    style: hidden.contains(group) ? t.kicker : t.kickerOn,
                  ),
                ),
              ),
              const SizedBox(width: 4),
            ],
          ],
        ),
      ),
    );
  }
}

/// The lanes' bottom bar (docs/07 §4.5-§4.6): − / + / Fit with the zoom read
/// out, the magnet, and the horizontal scrollbar that moves the zoomed view.
///
/// In graph view it also carries the graph's own commands (docs/07 §5.3):
/// Linear / Bezier / Hold for the selected keys, the value/speed lens
/// switch, and the auto-fit toggle.
class _LaneBottomBar extends StatelessWidget {
  /// Where the zoom is *going*, not where the flight has reached — so the
  /// handle sits under the finger that put it there rather than trailing the
  /// animation by a flight's length (K-293).
  final double zoom;

  /// The far end of the slider: the zoom at which the lanes show
  /// [_TimelinePanelFrbState._framesAtFullZoom] frames.
  final double maxZoom;
  final ScrollController hScroll;

  /// A zoom asked for in one step — a tap on the track — which flies.
  final ValueChanged<double> onZoom;

  /// A zoom asked for continuously, while the handle is dragged. The drag is
  /// the motion, so this one arrives at once.
  final ValueChanged<double> onZoomLive;

  /// The drag's ends, so the panel can anchor once per gesture (K-319).
  final VoidCallback? onZoomDragStart;
  final VoidCallback? onZoomDragEnd;
  final bool magnet;
  final VoidCallback onToggleMagnet;

  /// Set in graph view; null hides the graph commands (the lane view).
  final GraphLens? lens;
  final ValueChanged<GraphLens>? onLens;
  final bool autoFit;
  final VoidCallback? onToggleAutoFit;
  final ValueChanged<BridgeSideInterp>? onInterp;

  /// The Easing… button pressed, with the button's own context so a popup can
  /// be anchored to it. Whether that is a popup or a docked panel is the
  /// panel's decision, not this bar's (K-349).
  final ValueChanged<BuildContext>? onOpenEasing;

  const _LaneBottomBar({
    required this.zoom,
    required this.maxZoom,
    required this.hScroll,
    required this.onZoom,
    required this.onZoomLive,
    this.onZoomDragStart,
    this.onZoomDragEnd,
    required this.magnet,
    required this.onToggleMagnet,
    this.lens,
    this.onLens,
    this.autoFit = true,
    this.onToggleAutoFit,
    this.onInterp,
    this.onOpenEasing,
  });

  Widget _graphButton(
    LumitTheme t, {
    required String keyName,
    required String label,
    required String tip,
    required bool on,
    required VoidCallback onPressed,
  }) =>
      LumitTooltip(
        message: tip,
        child: HouseButton(
          key: ValueKey<String>(keyName),
          small: true,
          frameless: true,
          padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
          onPressed: onPressed,
          child: Text(label,
              style: TextStyle(
                  color: on ? t.accent : t.textMuted,
                  fontSize: t.small.fontSize)),
        ),
      );

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      height: _laneBottomBarHeight,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 4),
      child: LayoutBuilder(
        builder: (context, constraints) {
          // The buttons scroll sideways when the panel is narrow — the same
          // answer the Timeline toolbar gives; an overflow stripe is a
          // layout fault. The scrollbar keeps its share of the bar whatever
          // the buttons need.
          final buttonRoom =
              (constraints.maxWidth - 120).clamp(0.0, constraints.maxWidth);
          return Row(
            children: [
              ConstrainedBox(
                constraints: BoxConstraints(maxWidth: buttonRoom),
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Row(
                    children: [
                      if (lens != null) ...[
                        // The selected keys' easing, one click each — the F9 family's
                        // buttons (docs/07 §5.3).
                        _graphButton(t,
                            keyName: 'graph-interp-linear',
                            label: l10n.easeLinear,
                            tip: l10n.tipLinearKeyframes,
                            on: false,
                            onPressed: () => onInterp
                                ?.call(const BridgeSideInterp.linear())),
                        _graphButton(t,
                            keyName: 'graph-interp-bezier',
                            label: l10n.easeBezier,
                            tip: l10n.tipEasyEase,
                            on: false,
                            onPressed: () => onInterp?.call(easyEase)),
                        _graphButton(t,
                            keyName: 'graph-interp-hold',
                            label: l10n.easeHold,
                            tip: l10n.tipHoldKeyframes,
                            on: false,
                            onPressed: () =>
                                onInterp?.call(const BridgeSideInterp.hold())),
                        // The shaped ease, one step along from the one-click
                        // three: same selection, a curve instead of a constant.
                        // Its own Builder so the popup can find where this
                        // button is; the popup layout slides it up into view.
                        //
                        // Value lens only. The box draws a shape against the
                        // value's own travel, so a curve stamped while the
                        // speed lens is up would land on the value graph — a
                        // change the user cannot see in the view they drew it
                        // in. The one-click three above stay in both lenses: a
                        // side's interp means the same thing either way.
                        if (lens == GraphLens.value)
                          Builder(
                            builder: (buttonContext) => _graphButton(t,
                                keyName: 'graph-interp-easing',
                                label: l10n.easeCustom,
                                tip: l10n.tipEasingEditor,
                                on: false,
                                onPressed: () =>
                                    onOpenEasing?.call(buttonContext)),
                          ),
                        const SizedBox(width: 6),
                        _graphButton(t,
                            keyName: 'graph-lens-value',
                            label: l10n.clipboardValueColumn,
                            tip: l10n.tipValueGraph,
                            on: lens == GraphLens.value,
                            onPressed: () => onLens?.call(GraphLens.value)),
                        _graphButton(t,
                            keyName: 'graph-lens-speed',
                            label: l10n.graphSpeed,
                            tip: l10n.tipSpeedGraph,
                            on: lens == GraphLens.speed,
                            onPressed: () => onLens?.call(GraphLens.speed)),
                        const SizedBox(width: 6),
                        _graphButton(t,
                            keyName: 'graph-autofit',
                            label: l10n.graphAutoFit,
                            tip: autoFit
                                ? l10n.tipAutoFitOn
                                : l10n.tipAutoFitOff,
                            on: autoFit,
                            onPressed: () => onToggleAutoFit?.call()),
                        const SizedBox(width: 6),
                      ],
                      ...[
                        // The zoom, as a slider between a small landscape and
                        // a large one (owner, 2026-08-06) — the pair After
                        // Effects flanks its own zoom slider with. The far left
                        // is the whole composition; the far right is twenty
                        // frames across the lanes, whatever the comp's length.
                        // It replaced − / + / Fit: the two ends *are* Fit and
                        // full zoom, and a slider says where you are between
                        // them, which three buttons never did.
                        //
                        // Painter-drawn and small, both deliberately: the pair
                        // only says "less / more" if the sizes plainly differ,
                        // and an Iconoir glyph under 16px crunches (K-209), so
                        // these are filled shapes with no stroke to lose.
                        lumitIcon(LumitIcon.zoomExtent,
                            size: _zoomGlyphSmall, color: t.textMuted),
                        const SizedBox(width: 4),
                        LumitTooltip(
                          message:
                              l10n.tipZoomPercent('${(zoom * 100).round()}'),
                          child: HouseSlider(
                            key: const ValueKey('tl-zoom-slider'),
                            // The slider runs on the *logarithm* of the zoom,
                            // so equal travel buys equal ratio — the same
                            // reason the flight interpolates that way. A linear
                            // one would spend nine tenths of its length in the
                            // last few frames of a long comp.
                            value: zoomSliderPosition(zoom, maxZoom),
                            min: 0,
                            max: 1,
                            width: 96,
                            showValue: false,
                            // Dragged, the zoom follows the finger with no
                            // flight; tapped, it flies to where the track was
                            // clicked (K-293). The drag's ends bracket the
                            // gesture so the panel anchors once (K-319).
                            onChangeStart: onZoomDragStart,
                            onChangeEnd: onZoomDragEnd,
                            onChangeLive: (t) =>
                                onZoomLive(zoomForSliderPosition(t, maxZoom)),
                            onChanged: (t) =>
                                onZoom(zoomForSliderPosition(t, maxZoom)),
                          ),
                        ),
                        const SizedBox(width: 4),
                        lumitIcon(LumitIcon.zoomExtent,
                            size: _zoomGlyphLarge, color: t.textMuted),
                        const SizedBox(width: 6),
                        LumitTooltip(
                          message: magnet ? l10n.tipSnapOn : l10n.tipSnapOff,
                          child: HouseButton(
                            key: const ValueKey('tl-magnet'),
                            small: true,
                            frameless: true,
                            padding: const EdgeInsets.symmetric(
                                horizontal: 4, vertical: 2),
                            onPressed: onToggleMagnet,
                            child: lumitIcon(LumitIcon.magnet,
                                size: iconSize,
                                color: magnet ? t.accent : t.textMuted),
                          ),
                        ),
                        const SizedBox(width: 6),
                      ],
                    ],
                  ),
                ),
              ),
              Expanded(
                child: _GutterScrollbar(
                  controller: hScroll,
                  axis: Axis.horizontal,
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}

/// One layer's bar: drag its middle to move it, its ends to trim.
class _Bar extends StatefulWidget {
  final CompositionReference comp;
  final BridgeLayerEntry entry;
  final TimelineAxis axis;
  final bool razor;

  /// Read when the razor is clicked, not captured when the bar is built.
  final int Function() playheadFrame;

  /// A razor click on this bar, at the frame under the pointer (K-220) — the
  /// panel decides what that cuts, because Shift cuts layers this bar knows
  /// nothing about.
  final void Function(int frame) onRazor;

  /// Where a cut at screen x lands, in comp frames — the same function the
  /// blade's line is drawn with, so the two cannot disagree (docs/07 §4.5).
  final double Function(double x) razorFrameAt;

  /// Clicking (or grabbing) the bar selects its layer.
  final VoidCallback onSelect;

  /// Double-clicking a Sequence layer's bar opens its view, the same as
  /// double-clicking its name (K-248): the clips are what you came for, and
  /// the bar is where you were already looking.
  final VoidCallback? onOpenSequence;
  final VoidCallback onChanged;

  /// Where the live preview is published, for the waveform lane to follow.
  final ValueNotifier<BarDragPreview?> dragPreview;

  /// How far this layer's ends may be dragged (K-211). [BarBounds.free] for
  /// every kind that has no source to run out of.
  final BarBounds bounds;

  /// Whether this layer is in the selection. The bar is the only mark a
  /// selected layer has on the lane side, and with several chosen at once
  /// (K-217) the outline's lit rows are off the side of the panel.
  final bool selected;

  /// Every key on the layer, drawn on its row at half scale while it is shut
  /// (§12A.1) — a summary, not a target: they are not draggable here, because
  /// several properties keyed on one frame are several keys under one diamond.
  /// Twirl the layer open and each property's lane draws its own.
  final List<BridgeKeyframe> summaryKeys;

  /// The comp's rate, to place [summaryKeys] on the frame axis.
  final double fps;

  const _Bar({
    super.key,
    required this.comp,
    required this.entry,
    required this.axis,
    required this.razor,
    required this.selected,
    required this.playheadFrame,
    required this.onRazor,
    required this.razorFrameAt,
    required this.onSelect,
    this.onOpenSequence,
    required this.onChanged,
    required this.dragPreview,
    required this.bounds,
    this.summaryKeys = const [],
    required this.fps,
  });

  @override
  State<_Bar> createState() => _BarState();
}

class _BarState extends State<_Bar> {
  /// Spots a double-click without putting a recogniser in the razor’s way.
  final DoubleTap _barTaps = DoubleTap();

  /// Frames the gesture has moved so far, held here rather than committed.
  ///
  /// A bar drag has no cheap preview to show — moving a layer in time changes
  /// what every frame contains — so the bar moves in Dart and the document
  /// learns about it once, on release.
  int _delta = 0;

  /// Pixels the gesture has moved so far. The frame delta is always derived
  /// from this running total: rounding each pointer event's own delta to
  /// frames and summing those threw the sub-frame remainders away, so a slow
  /// drag moved less than the pointer and a fast one more — which reads as
  /// mouse acceleration.
  double _deltaPx = 0;
  BarGrab? _grab;

  /// Where the pointer went DOWN, deciding edge-trim versus move. Down, not
  /// drag-start: a drag's start position is where the slop was exceeded,
  /// which read a fast edge grab as a grab of the middle.
  double _downDx = 0;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final round = t.shape == ThemeShape.round;
    // ZERO bridge calls (K-184): the span already mapped to comp frames, the
    // kind, and the clip split positions all ride in on the read model.
    final info = widget.entry.info;
    final inFrame = info.inFrame;
    final outFrame = info.outFrame;

    // A locked layer's bar is a fact, not a handle: no move, no trim, no cut
    // — clicking it still selects, so the lock switch stays reachable.
    final held = info.switches.locked;

    final (drawIn, drawOut) = switch (_grab) {
      BarGrab.move => (inFrame + _delta, outFrame + _delta),
      BarGrab.trimIn => (inFrame + _delta, outFrame),
      BarGrab.trimOut => (inFrame, outFrame + _delta),
      null => (inFrame, outFrame),
    };

    final left = widget.axis.xOf(drawIn);
    final width = (widget.axis.xOf(drawOut) - left).clamp(2.0, 1e6);

    // The source's reach travels with a move: sliding a layer along the
    // timeline carries its start offset, so the media it can show moves with
    // it. Without this the marks and the ghost stayed behind while the bar
    // went, and a bar at its limit looked as though it had left the limit.
    final shift = _grab == BarGrab.move ? _delta : 0;
    final minIn =
        widget.bounds.minIn == null ? null : widget.bounds.minIn! + shift;
    final maxOut =
        widget.bounds.maxOut == null ? null : widget.bounds.maxOut! + shift;
    // Where the untrimmed source would reach (K-212): drawn behind the bar, so
    // what shows past each end is exactly the material trimmed away. Only when
    // there is something to show — a bar filling its source draws no ghost.
    final ghost = (minIn != null && maxOut != null) &&
            (drawIn > minIn || drawOut < maxOut)
        ? (widget.axis.xOf(minIn), widget.axis.xOf(maxOut))
        : null;

    // The bar fills the row's whole height rather than floating inside an
    // inset, so a layer reads as a solid band; the lane area's own hairline
    // overlay draws the row seam over it (K-190).
    return SizedBox(
      height: _rowHeight,
      // **Both children are keyed.** The ghost comes and goes as the bar is
      // trimmed, and without keys the children were matched by position: the
      // ghost appearing took the bar's slot, so the bar's element — and with it
      // the gesture recogniser holding the drag — was rebuilt from scratch
      // mid-gesture. The bar moved by the first update's frames and then went
      // dead, which is what "dragging a footage edge only moves one frame"
      // was. Keys keep each child matched to its own element however many
      // there are.
      child: Stack(
        children: [
          if (ghost != null)
            Positioned(
              key: ValueKey<String>(
                  'tl-bar-ghost-${widget.entry.layer.internallayerId}'),
              left: ghost.$1,
              width: (ghost.$2 - ghost.$1).clamp(1.0, 1e6),
              top: 0,
              bottom: 0,
              child: IgnorePointer(
                child: Container(
                  decoration: BoxDecoration(
                    // A hairline and nothing inside it (§12A.1): the outline
                    // says how far this same clip *could* still be pulled, and
                    // a fill would read as a second, dimmer object sitting
                    // behind the bar rather than as the bar's own reach.
                    border: Border.all(
                      color: t.labelColour(info.label).withValues(alpha: 0.25),
                      width: 1,
                    ),
                    // Follows the bar's own ends: this *is* the bar, drawn as
                    // far as its source goes, and a rectangle round a capsule
                    // would read as a second object rather than the same one.
                    borderRadius: BorderRadius.circular(
                        round ? t.tokens.controlRadius : 2),
                  ),
                ),
              ),
            ),
          Positioned(
            key: ValueKey<String>(
                'tl-bar-body-${widget.entry.layer.internallayerId}'),
            left: left,
            width: width,
            top: 0,
            bottom: 0,
            // Selection on the raw DOWN, outside the gesture arena: the
            // bar's tap otherwise waits for the move/trim drag recognisers
            // to concede before the Effect controls learn the layer.
            child: Listener(
              onPointerDown: (event) {
                if (event.buttons != kPrimaryButton) return;
                widget.onSelect();
                // A Sequence layer's bar opens its view on a double-click, the
                // same as its name does (K-248) — counted here rather than
                // with an `onDoubleTap` below, because a double-tap recogniser
                // beside the razor's `onTapUp` makes the arena hold every
                // single tap back, and the razor stops cutting ([DoubleTap]).
                final open = widget.onOpenSequence;
                if (open != null && _barTaps.tap()) open();
              },
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                // Armed razor: a click cuts this layer **where it was clicked**
                // rather than starting a drag (docs/07 §4.4). At the playhead
                // is what Cut-at-playhead is for; a razor's whole point is that
                // the cut lands under the blade. A layer with nothing cuttable
                // there says so through the engine's calm error, which is
                // nothing on screen — the cut simply does not happen.
                onTapUp: widget.razor && !held
                    ? (details) => widget.onRazor(
                          widget
                              .razorFrameAt(left + details.localPosition.dx)
                              .round(),
                        )
                    : null,
                // Selection already happened on the down; the tap has nothing
                // left to do, but registering it keeps the click out of any
                // parent recogniser's hands.
                onTap: widget.razor && !held ? null : () {},
                onHorizontalDragDown: widget.razor || held
                    ? null
                    : (d) => _downDx = d.localPosition.dx,
                supportedDevices: dragDevices,
                onHorizontalDragStart: widget.razor || held
                    ? null
                    // No select here: every drag begins with the down, and the
                    // down already selected.
                    : (d) => setState(() {
                          _delta = 0;
                          _deltaPx = 0;
                          _grab = barGrabAt(_downDx, width);
                        }),
                onHorizontalDragUpdate: widget.razor || held
                    ? null
                    : (d) => setState(() {
                          _deltaPx += d.delta.dx;
                          // The pointer keeps travelling; the bar does not.
                          // Held against the source's ends (K-211) and against
                          // itself, so a trim can neither run past the media
                          // nor turn the bar inside out — and dragging back
                          // picks the edge up again from where it stuck.
                          _delta = clampBarDelta(
                            grab: _grab ?? BarGrab.move,
                            // A *travel*, not a place: the axis's end padding
                            // must not be taken off it.
                            delta: widget.axis.framesOfPx(_deltaPx).round(),
                            inFrame: inFrame,
                            outFrame: outFrame,
                            bounds: widget.bounds,
                          );
                          _publishPreview();
                        }),
                onHorizontalDragEnd: widget.razor || held
                    ? null
                    : (_) => _commit(inFrame, outFrame),
                onHorizontalDragCancel: widget.razor || held
                    ? null
                    : () => setState(() {
                          _delta = 0;
                          _deltaPx = 0;
                          _grab = null;
                          widget.dragPreview.value = null;
                        }),
                child: Container(
                  key: ValueKey<String>(
                      'tl-bar-fill-${widget.entry.layer.internallayerId}'),
                  decoration: BoxDecoration(
                    // The layer's label colour (K-188): the same chip the
                    // outline swatch shows, so recolouring a layer recolours
                    // its bar — and each kind starts on its own colour.
                    // **Desaturated** under the redesign (§12A.1): the fill is
                    // that colour at [clipFillAlpha] over the lane's ground,
                    // computed from the token rather than picked, so a lane
                    // full of layers reads organised rather than carnival. The
                    // solid leading edge below carries the full colour.
                    // Selected bars brighten that fill rather than growing an
                    // outline: the hue still says which layer this is, and a
                    // lighter bar reads at a glance where a 1px box did not.
                    color: widget.selected
                        ? Color.lerp(
                                t.labelColour(info.label), t.textPrimary, 0.35)!
                            .withValues(alpha: clipFillSelectedAlpha)
                        : t
                            .labelColour(info.label)
                            .withValues(alpha: clipFillAlpha),
                    // Stadium ends under Round (K-394, §12.1) — the control
                    // radius is the sentinel that clamps to half the bar's own
                    // height. **The bar's HIT rect is unchanged and stays
                    // rectangular**: a BoxDecoration's radius paints, it does
                    // not hit-test, so [barGrabAt] still reads dx across the
                    // full width and the trim zones keep exactly the grab area
                    // they had. That is deliberate — a curved end would take
                    // pixels off the corner of a target already only 8 px wide.
                    borderRadius: BorderRadius.circular(
                        round ? t.tokens.controlRadius : 2),
                  ),
                  child: Stack(
                    children: [
                      // The leading edge (§12A.1): 2px of the full colour at
                      // the bar's start, so a desaturated fill still lands with
                      // a snap and a row of bars reads as a row of beginnings.
                      Positioned(
                        key: ValueKey<String>(
                            'tl-bar-edge-${widget.entry.layer.internallayerId}'),
                        left: 0,
                        top: 0,
                        bottom: 0,
                        width: clipEdgeWidth,
                        child: IgnorePointer(
                          child: ColoredBox(color: t.labelColour(info.label)),
                        ),
                      ),
                      // The layer's name, on its bar (§6.1): mono, quiet
                      // enough to sit under the marks and the waveform, clear
                      // of the leading edge.
                      Positioned(
                        left: clipEdgeWidth + 4,
                        right: 2,
                        top: 0,
                        bottom: 0,
                        child: IgnorePointer(
                          child: Align(
                            alignment: Alignment.centerLeft,
                            child: Text(
                              info.name,
                              key: ValueKey<String>(
                                  'tl-bar-name-${widget.entry.layer.internallayerId}'),
                              style: t.mono.copyWith(
                                fontSize: 11,
                                color: t.textPrimary
                                    .withValues(alpha: clipNameAlpha),
                              ),
                              maxLines: 1,
                              overflow: TextOverflow.clip,
                              softWrap: false,
                            ),
                          ),
                        ),
                      ),
                      // A Sequence layer's bar stays a plain bar: the clips and
                      // their edit points are the sequence view's to draw, and
                      // split lines up here only said the same thing twice
                      // (K-248). What the bar does show is where its clips are
                      // *not* — the gaps, faint, the way a trimmed footage
                      // layer shows the source it is not using (K-212).
                      if (info.kind == BridgeLayerKind.sequence)
                        Positioned.fill(
                          child: IgnorePointer(
                            child: CustomPaint(
                              painter: SequenceGapsPainter(
                                clips: info.clips,
                                axis: widget.axis,
                                left: left,
                                ink: t.surface0,
                              ),
                            ),
                          ),
                        ),
                      // The two trim zones say so under the pointer: a bar
                      // whose ends can be taken hold of should not have to be
                      // discovered by trial. Inside the gesture detector, not
                      // over it, so hovering never costs the drag its events.
                      if (!held && !widget.razor) ...[
                        _trimCursor(width, left: true),
                        _trimCursor(width, left: false),
                      ],
                      // The corner marks: this bar is as long as its source
                      // allows in that direction (K-211).
                      Positioned.fill(
                        child: IgnorePointer(
                          child: CustomPaint(
                            key: ValueKey<String>(
                                'tl-bar-ends-${widget.entry.layer.internallayerId}'),
                            painter: BarEndMarksPainter(
                              atIn: minIn != null && drawIn <= minIn,
                              atOut: maxOut != null && drawOut >= maxOut,
                              // The same ink the clip splits use, so the bar
                              // keeps one vocabulary of marks.
                              colour: t.surface0,
                            ),
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
          // What is keyed inside a shut layer, at half scale (§12A.1) — so
          // the stack says where the animation is without every layer having
          // to be twirled open. They travel with a move, because they belong
          // to the layer.
          if (widget.summaryKeys.isNotEmpty)
            Positioned.fill(
              key: ValueKey<String>(
                  'tl-bar-keys-${widget.entry.layer.internallayerId}'),
              child: IgnorePointer(
                child: CustomPaint(
                  painter: _LaneKeysPainter(
                    frames: [
                      for (final k in widget.summaryKeys)
                        laneKeyFrame(k, widget.fps) + shift,
                    ],
                    selected: const {},
                    axis: widget.axis,
                    colour: t.animated,
                    chosen: t.textPrimary,
                    half: _summaryKeyHalf,
                  ),
                ),
              ),
            ),
          // The layer's own markers (K-254), over the bar so they take the
          // pointer ahead of it — a flag is a much smaller target than a bar,
          // and a right-click meant for one must not open the bar's menu.
          // They travel with a move, because they are part of the layer.
          for (final m in info.markers)
            Positioned(
              left: widget.axis.xOf(m.frame.toInt() + shift) -
                  MarkerFlag.width / 2,
              bottom: 0,
              child: MouseRegion(
                cursor: SystemMouseCursors.click,
                child: GestureDetector(
                  key: ValueKey<String>('tl-layer-marker-${m.marker.id}'),
                  behavior: HitTestBehavior.opaque,
                  onSecondaryTapUp: (d) =>
                      _markerMenu(context, m.marker, d.globalPosition),
                  // A left click on a flag is a click on its layer, which is
                  // what the bar under it would have done.
                  onTap: widget.onSelect,
                  child: MarkerFlag(
                    label: m.marker.label,
                    fill: t.marker,
                    pill: t.surface4,
                    text: markerLabelStyle(t),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }

  /// The right-click menu on a marker sitting on a layer's bar — the shared
  /// marker menu, with Delete all on it.
  ///
  /// Deleting here touches **this layer's** list and nothing else. A layer's
  /// markers are its own copy of whatever composition was dropped in, so a
  /// delete cannot reach into that comp — or into the other places it is used
  /// (K-254).
  void _markerMenu(BuildContext context, BridgeMarker marker, Offset at) {
    showMarkerMenuFrb(
      context: context,
      position: at,
      marker: marker,
      markers: () => [for (final m in widget.entry.info.markers) m.marker],
      write: (markers) {
        widget.entry.layer.setMarkers(markers: markers);
        widget.onChanged();
      },
      deleteAll: true,
      keyPrefix: 'tl-layer-marker',
    );
  }

  /// One end's hover strip: the pointer becomes the horizontal resize arrow
  /// over exactly the width [barGrabAt] treats as that end.
  Widget _trimCursor(double width, {required bool left}) {
    final edge = min(_trimGrab, width / 3);
    return Positioned(
      left: left ? 0 : null,
      right: left ? null : 0,
      top: 0,
      bottom: 0,
      width: edge,
      child: const MouseRegion(
        cursor: SystemMouseCursors.resizeLeftRight,
        child: SizedBox.expand(),
      ),
    );
  }

  /// Publish where the gesture has the bar right now, for the waveform lane.
  void _publishPreview() {
    final grab = _grab;
    if (grab == null) return;
    widget.dragPreview.value = barDragPreview(
        widget.entry.layer.internallayerId.toString(), grab, _delta);
  }

  /// One `set_span` for the whole gesture, so a move that shifted the in point
  /// and the start offset together is a single undo step.
  void _commit(int inFrame, int outFrame) {
    final grab = _grab;
    // Clamped once more on the way out: a source length that arrived from its
    // probe part-way through the gesture only reaches the bar on the next
    // build, and what is committed must obey the bounds in force at release.
    final delta = grab == null
        ? 0
        : clampBarDelta(
            grab: grab,
            delta: _delta,
            inFrame: inFrame,
            outFrame: outFrame,
            bounds: widget.bounds,
          );
    setState(() {
      _delta = 0;
      _deltaPx = 0;
      _grab = null;
    });
    widget.dragPreview.value = null;
    if (grab == null || delta == 0) return;

    final span = widget.entry.info.span;
    var newIn = inFrame;
    var newOut = outFrame;
    var offsetShift = 0;
    switch (grab) {
      case BarGrab.move:
        newIn += delta;
        newOut += delta;
        // Moving carries the content with the bar, so time 0 travels too.
        offsetShift = delta;
      case BarGrab.trimIn:
        newIn += delta;
      case BarGrab.trimOut:
        newOut += delta;
    }
    // A bar cannot be trimmed past itself; the op refuses it, and refusing here
    // first means the gesture simply stops rather than raising.
    if (newOut <= newIn) return;

    widget.entry.layer.setSpan(
      span: BridgeSpan(
        inPoint: widget.comp.timeOfFrame(frame: newIn),
        outPoint: widget.comp.timeOfFrame(frame: newOut),
        startOffset: offsetShift == 0
            ? span.startOffset
            : widget.comp.timeOfFrame(
                frame: widget.comp.frameAtTime(time: span.startOffset) +
                    offsetShift,
              ),
      ),
    );
    widget.onChanged();
  }
}

/// Which part of a bar a drag grabbed: its middle, or one of its two ends.
enum BarGrab { move, trimIn, trimOut }
