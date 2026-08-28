// The Timeline's lane area: the ruler, the playhead and one bar per layer,
// with the selected-key model the block tools work in.
//
// Split out of timeline_panel_frb.dart.

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';
import '../state/comp_time.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/marquee.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'key_block.dart';
import 'graph_editor_frb.dart';
import 'graph_maths.dart';
import 'timeline_extras_frb.dart';
import 'sequence_view_frb.dart';
import 'timeline_razor.dart';
import 'layer_fold_frb.dart';
import 'timeline_snap.dart';
import 'waveform_frb.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_bar_frb.dart';
import 'timeline_key_lane_frb.dart';
import 'timeline_key_block_frb.dart';

/// One selected keyframe, and where it sits: which row's curve it belongs to,
/// which key of that curve it is, what frame it reads and where its lane is in
/// the area's own pixels (K-458).
///
/// The block tools' unit of work. A stretch scales [frame]; Reverse mirrors
/// it; Stagger pushes it; the box and its badge measure the set of them; and
/// the commit groups them back by ([entry], [row]) into one write per row.
class SelectedKey {
  final BridgeLayerEntry entry;
  final LayerFoldRow row;
  final String rowId;
  final int index;
  final double frame;
  final double top;
  final double height;

  const SelectedKey({
    required this.entry,
    required this.row,
    required this.rowId,
    required this.index,
    required this.frame,
    required this.top,
    required this.height,
  });
}

/// Write a finished key gesture: every key [moved] holds re-timed to where it
/// has been seen to travel, the whole set **one undo step** (K-458).
///
/// One writer for the block stretch and for a lane key's drag (6.24), because
/// they are one act — a set of keys, and a rule saying where each of them
/// lands. Grouped by row before anything is written, so a row's keys move
/// together and the strictly-ascending check inside [moveLaneKeys] sees the
/// finished list rather than one key at a time.
///
/// Returns whether anything was written.
bool commitKeyGesture({
  required List<SelectedKey> places,
  required KeyStretch moved,
  required bool whole,
  required int fpsNum,
  required int fpsDen,
  required ProjectReference? project,
}) {
  final byRow = <String, (SelectedKey, Map<int, BridgeRational>)>{};
  for (final place in places) {
    if (!moved.keys.contains('${place.rowId}#${place.index}')) continue;
    final frame = moved.frameOf(place.frame, whole: whole);
    (byRow[place.rowId] ??= (place, {})).$2[place.index] =
        timeOfSubframe(frame, fpsNum, fpsDen);
  }
  if (byRow.isEmpty) return false;
  var changed = false;
  asOneUndoStep(project, () {
    for (final (place, times) in byRow.values) {
      if (moveLaneKeys(entry: place.entry, row: place.row, times: times)) {
        changed = true;
      }
    }
  });
  return changed;
}

/// Everything a drag on the Timeline can land on (docs/07 §4.5), built from
/// the read model and the memoised marker list — so it costs no bridge calls
/// (K-184).
///
/// One list for the whole panel: the lanes' keys and bars, the ruler's
/// work-area edges and markers, and the graph's key drags all reach for the
/// same things, and a target that only some of them can see would be a second
/// answer to "what is there".
List<SnapTarget> timelineSnapTargets({
  required List<LayerRow> rows,
  required CompositionReference comp,
  required int playheadFrame,
  required ({int start, int end, bool whole}) work,
  required double fps,
}) =>
    snapTargetsOf(
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
      playheadFrame: playheadFrame,
      work: work,
      fps: fps,
    );

/// The right column: the ruler, the playhead, and one bar per layer.
class LayerArea extends StatelessWidget {
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
  ///
  /// **Listened to, not handed down.** Clicking a property's name picks its
  /// keyframes with it (K-500 §2.1), so a click changes this as well as the
  /// outline's selection — and a panel-wide `setState` to say so redrew the
  /// ruler, the bars and every lane to fill a handful of diamonds. Each
  /// layer's lanes watch their own share of it instead ([_LayerKeys]), on the
  /// same rule the outline's blocks follow. Everything outside a build reads
  /// `.value`, which is where a gesture has always taken it from.
  final ValueListenable<Set<String>> selectedKeys;
  final ValueChanged<Set<String>> onKeysSelected;

  /// A right-click on a lane key, by id and at the pointer in global
  /// coordinates — the panel opens the menu, because the rows it acts on
  /// reach past this lane (K-500 §2.1).
  final void Function(String id, Offset position) onKeyMenu;

  /// The block stretch in flight, written by the handle and read by every lane
  /// — the same arrangement the bar drag uses (K-208), and for the same
  /// reason: the keys being stretched are spread across rows in two scroll
  /// views, so a gesture only the handle knew about would move the box while
  /// the diamonds sat still.
  final ValueNotifier<KeyStretch?> stretch;

  /// The project the block tools commit against, for the undo group that makes
  /// a stretch across several rows one step (K-458). Null in a widget test with
  /// no project open, where [asOneUndoStep] simply runs the writes.
  final ProjectReference? project;

  /// The Ease popover asked for from the block's badge, at the badge's own
  /// position in global coordinates — the drawing anchors the popover to the
  /// selection, and the badge is the only part of the box that is a control.
  final ValueChanged<Offset> onEase;

  /// A click on empty lane space — no bar, no diamond, no drag. Everything
  /// lets go (K-203).
  final VoidCallback onDeselectAll;

  /// The work area in frames, read once by the panel (K-203).
  final ({int start, int end, bool whole}) work;

  /// The ruler's mid-drag span, handed straight up to the panel — see
  /// `_workPreview` there.
  final ValueChanged<({int start, int end, bool whole})?> onWorkPreview;

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

  /// Settings ▸ Interface ▸ Panels ▸ *Layer names on lane bars* (K-514), off
  /// by default. Read once by the panel and handed down, never looked up in a
  /// bar's own build.
  final bool barNames;

  const LayerArea({super.key, 
    required this.comp,
    required this.rows,
    this.barNames = false,
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
    required this.onKeyMenu,
    required this.stretch,
    required this.project,
    required this.onEase,
    required this.onDeselectAll,
    required this.work,
    required this.onWorkPreview,
    required this.layerDrag,
    required this.blockHeights,
    required this.fpsNum,
    required this.fpsDen,
    required this.magnet,
    required this.onWheel,
  });

  /// Every keyframe the box caught, walking the same rows the lanes draw —
  /// y from the row stack, x from the key's frame on the axis.
  ///
  /// The height comes off the row itself rather than from a theme lookup: this
  /// runs from a drag, outside any build, and a row already knows what it
  /// measures (K-454).
  Set<String> _keysIn(Rect rect) {
    final caught = <String>{};
    var y = 0.0;
    for (final layer in rows) {
      final step = layer.rowHeight;
      y += step; // the layer's own bar row
      // **And the room an open Sequence view took** (K-248, §4.4). The view
      // sits between the layer's own row and its fold-out, so a walk that
      // stepped only by row heights put every row below one adrift by the
      // view's extra height — the box caught keys the user had not drawn it
      // round, and the block box that reads the same walk drew itself over
      // the wrong rows.
      y += layer.sequenceExtra ?? 0;
      for (final row in layer.drawnRows) {
        final rowTop = y;
        y += step;
        if (rowTop + step < rect.top || rowTop > rect.bottom) continue;
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

  /// The keyed lane row [y] pixels down the area, and its path.
  ///
  /// Null over a layer's own row, over an open Sequence view, over a row with
  /// nothing keyed, and over the ground below the last layer — everywhere, in
  /// other words, that has no property to plant a key on.
  ({LayerRow layer, LayerFoldRow row, String rowId})? _rowAt(double y) {
    var top = 0.0;
    for (final layer in rows) {
      final step = layer.rowHeight;
      top += step; // the layer's own bar row
      top += layer.sequenceExtra ?? 0;
      for (final row in layer.drawnRows) {
        if (y >= top && y < top + step) {
          if (laneKeysOf(row).isEmpty) return null;
          return (layer: layer, row: row, rowId: foldRowPath(layer.id, row));
        }
        top += step;
      }
    }
    return null;
  }

  /// A click on lane ground: `Ctrl` plants a key on the keyed row under the
  /// pointer at that time (docs/07 §4.3, K-500 §2.1), a plain click lets
  /// everything go (K-203).
  ///
  /// The new key takes the value the curve already reads there, so planting
  /// one moves nothing — it is a place to grab. A two-axis row keys both axes,
  /// because one lane diamond stands for the whole row.
  void _tapGround(Offset local) {
    final keyboard = HardwareKeyboard.instance;
    if (!keyboard.isControlPressed && !keyboard.isMetaPressed) {
      onDeselectAll();
      return;
    }
    final at = _rowAt(local.dy);
    if (at == null) return;
    final frame = magnet
        ? axis.frameAt(local.dx).toDouble()
        : axis.frameAtExact(local.dx);
    final planted = plantKeyOnChannels(
      channels: graphChannels(layers: [at.layer.entry], selected: [at.rowId]),
      frame: frame,
      fps: fps,
      fpsNum: fpsNum,
      fpsDen: fpsDen,
    );
    if (planted) onChanged();
  }

  /// One key picked: alone, or toggled in and out when [additive] (K-500 §2.1).
  ///
  /// One implementation, because a click has more than one way in now: the
  /// diamond's own gesture, and a click on the block handle standing over it
  /// (§2.1's carve-out, closed here).
  void _selectKey(String id, bool additive) {
    // A copy, never the live set: `onKeysSelected` clears it before it reads
    // what it was handed.
    final next = <String>{...selectedKeys.value};
    if (additive) {
      if (!next.remove(id)) next.add(id);
    } else {
      next
        ..clear()
        ..add(id);
    }
    onKeysSelected(next);
  }

  /// The release of a lane key's drag: every key it carried written where it
  /// travelled, one undo step (6.24).
  ///
  /// Done here rather than in the lane because the selection reaches across
  /// rows and layers, and a lane knows only its own — the same reason the
  /// block stretch commits here.
  void _moveHeldKeys(KeyStretch moved) {
    if (commitKeyGesture(
      places: _selectedKeyPlaces(),
      moved: moved,
      whole: magnet &&
          !snapSuspended(
              controlPressed: HardwareKeyboard.instance.isControlPressed),
      fpsNum: fpsNum,
      fpsDen: fpsDen,
      project: project,
    )) {
      onChanged();
    }
  }

  /// Every **selected** key, with where it sits: the one walk the block box,
  /// its badge, the stretch commit, Reverse and Stagger all read.
  ///
  /// One walk rather than five, because the five have to agree — a badge that
  /// counted keys one way while the stretch moved them another would be two
  /// descriptions of one block, and the disagreement would only show up as a
  /// box that no longer fits what it holds.
  ///
  /// Ordered top to bottom, which is the order Stagger's *top down* means.
  List<SelectedKey> _selectedKeyPlaces() {
    final held = selectedKeys.value;
    final out = <SelectedKey>[];
    var y = 0.0;
    for (final layer in rows) {
      final step = layer.rowHeight;
      y += step; // the layer's own bar row
      y += layer.sequenceExtra ?? 0; // and an open Sequence view's room (§4.4)
      for (final row in layer.drawnRows) {
        final rowTop = y;
        y += step;
        final rowId = foldRowPath(layer.id, row);
        final keys = laneKeysOf(row);
        for (var i = 0; i < keys.length; i++) {
          if (!held.contains('$rowId#$i')) continue;
          out.add(SelectedKey(
            entry: layer.entry,
            row: row,
            rowId: rowId,
            index: i,
            frame: laneKeyFrame(keys[i], fps),
            top: rowTop,
            height: step,
          ));
        }
      }
    }
    return out;
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
                // The toolbar and column header stay inside the outline
                // (docs/07 §4.1), so the lane side gives their whole height to
                // the ruler — which is exactly what `density.ruler` is: the
                // token carries that derivation now, rather than this file
                // adding two constants up (K-454, docs/15 §12A.6). The cache
                // bar is drawn *inside* it, on the ruler's floor over the
                // work-area band, rather than taking three pixels of its own
                // beneath it. That is what makes the ruler **double height**
                // (docs/15 §12A.1): the upper half is the clock, carrying the
                // labels, the ticks and the playhead's head, and the lower
                // half carries the markers, the work-area band and the cache.
                // A taller bar is an easier playhead grab as well, but the two
                // rows are the point.
                TimelineRuler(
                  comp: comp,
                  axis: axis,
                  fps: fps,
                  height: t.density.ruler,
                  work: work,
                  onSeek: onSeek,
                  onWorkArea: (span) {
                    comp.setWorkArea(span: span);
                    onChanged();
                  },
                  onWorkPreview: onWorkPreview,
                  onMarkersChanged: onChanged,
                  // The work-area edges and the markers snap to the same
                  // shared list the keys and the bars do (docs/07 §4.5).
                  snapTargets: snap,
                  magnet: magnet,
                  // On the ruler's floor, over the work-area band, which is
                  // where the interface spec puts it (docs/07 §3.2, §12A.1).
                  cache: TimelineCacheBar(
                      comp: comp, axis: axis, revision: cacheRevision),
                ),
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
                                    compStartX: axis.xOf(0),
                                    compEndX: axis.xOf(axis.frames),
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
                                // **Additive with `Shift` or `Ctrl`** held when
                                // the drag began (K-500 §2.1): the box adds to
                                // what was already in hand rather than
                                // replacing it, which is how a selection is
                                // built up out of rows that are not next to
                                // each other. The graph's box has always done
                                // this; the lanes were replace-only.
                                onSelect: (rect, additive) => onKeysSelected(
                                    additive
                                        ? {
                                            ...selectedKeys.value,
                                            ..._keysIn(rect)
                                          }
                                        : _keysIn(rect)),
                                // A click that caught nothing is a click on empty
                                // lane space, which is the deselect gesture: the
                                // bars and the key handles above take their own
                                // taps, so only the ground reaches here. With
                                // `Ctrl` it plants a key on the keyed row it
                                // landed on instead (docs/07 §4.3), which is why
                                // the click is reported by position.
                                onClear: onDeselectAll,
                                onTapAt: _tapGround,
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
                                            Bar(
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
                                              snapTargets: snap,
                                              magnet: magnet,
                                              showName: barNames,
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
                                              // Whatever the row grew, less
                                              // the two clip rows: the
                                              // rest is the graph's, so
                                              // the view fills exactly the
                                              // room the outline left.
                                              graphHeight:
                                                  rows[i].sequenceExtra! -
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
                                          // **The key selection is listened
                                          // to here, one layer at a time** —
                                          // the seam that keeps a click on a
                                          // property's name off the layers it
                                          // did not touch.
                                          if (rows[i].open)
                                            _LayerKeys(
                                              keys: selectedKeys,
                                              layerId: rows[i].id,
                                              builder: (context) => Column(
                                                key: ValueKey<String>(
                                                    'tl-lanes-${rows[i].id}'),
                                                children: [
                                                  for (final row
                                                      in rows[i].drawnRows)
                                                    SizedBox(
                                                      height: t.density.laneRow,
                                                      child: _lane(
                                                          t,
                                                          rows[i].entry,
                                                          row,
                                                          snap),
                                                    ),
                                                ],
                                              ),
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
                                      compStartX: axis.xOf(0),
                                      compEndX: axis.xOf(axis.frames),
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
                                  painter: RowDividerPainter(
                                    step: t.density.laneRow,
                                    colour: t.hairline,
                                    blanks: sequenceBlanks,
                                  ),
                                ),
                              ),
                            ),
                            // The block-selection box, over the keys it holds
                            // and over the seams that cross it (K-458): it is
                            // the one thing here that describes the *whole*
                            // selection, so anything drawn on top of it would
                            // be drawn on top of the answer. It is also the one
                            // thing here that cannot be gated per layer, for
                            // the same reason — so it listens whole, being a
                            // single overlay.
                            Positioned.fill(
                                child: ValueListenableBuilder<Set<String>>(
                                    valueListenable: selectedKeys,
                                    builder: (context, _, __) =>
                                        KeyBlockOverlay(
                              places: _selectedKeyPlaces(),
                              axis: axis,
                              stretch: stretch,
                              magnet: magnet,
                              snapTargets: snap,
                              fpsNum: fpsNum,
                              fpsDen: fpsDen,
                              project: project,
                              onEase: onEase,
                              onChanged: onChanged,
                              onSelectKey: _selectKey,
                              onKeyMenu: onKeyMenu,
                            ))),
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
  List<SnapTarget> _snapTargets() => timelineSnapTargets(
        rows: rows,
        comp: comp,
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
            size: Size(axis.width, t.density.laneRow),
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
              height: t.density.laneRow * 2,
            ),
          );
        },
      );
    }
    final rowKeys = laneKeysOf(row);
    if (rowKeys.isEmpty) return null;
    final rowId = foldRowPath(id, row);
    // The diamonds travel with a bar being moved, live (§6.26) — the same
    // reading the waveform above already takes from the same preview, and for
    // the same reason: a key that jumps only on release was never seen to
    // move. A trim leaves them where they are, which is what the release
    // writes too, since keys cross on the comp's clock by way of the layer's
    // start offset (K-213) and only a move carries that offset with it.
    return ValueListenableBuilder<BarDragPreview?>(
      valueListenable: dragPreview,
      builder: (context, preview, _) => KeyLane(
        key: ValueKey<String>('tl-keys-$rowId'),
        entry: entry,
        row: row,
        rowId: rowId,
        keys: rowKeys,
        axis: axis,
        fps: fps,
        fpsNum: fpsNum,
        fpsDen: fpsDen,
        magnet: magnet,
        barShift: keyShiftOf(preview, id),
        snapTargets: snapTargets,
        // The **whole** selection, not this layer's share of it: a lane draws
        // only its own diamonds from it, but a drag started here carries every
        // key in hand, and those sit on rows this lane cannot see. What the
        // per-layer gate above decides is when to rebuild, not what to hand
        // over.
        selectedKeys: selectedKeys.value,
        stretch: stretch,
        onKeyMenu: (index, position) => onKeyMenu('$rowId#$index', position),
        onSelectKey: (index, additive) => _selectKey('$rowId#$index', additive),
        onMoveKeys: _moveHeldKeys,
        onChanged: onChanged,
      ),
    );
  }
}

/// Rebuilds one layer's lanes only when *that layer's* selected keyframes
/// change — the lane half's `_LayerBlock`, and the same rule.
///
/// A `ValueListenableBuilder` here would not do: the selection is one set for
/// the whole table, so it changes for every layer at once and every layer's
/// lanes would redraw to fill one row's diamonds. The ids carry the row path,
/// and a row path starts with its layer's id, so a layer's own share is a
/// filter on that. The share is what this **compares**; what the lanes are
/// handed is still the whole selection, because a key drag started on one row
/// carries keys on rows this layer does not own.
class _LayerKeys extends StatefulWidget {
  const _LayerKeys({
    required this.keys,
    required this.layerId,
    required this.builder,
  });

  final ValueListenable<Set<String>> keys;
  final String layerId;
  final WidgetBuilder builder;

  @override
  State<_LayerKeys> createState() => _LayerKeysState();
}

class _LayerKeysState extends State<_LayerKeys> {
  late Set<String> _mine = _read();

  Set<String> _read() => {
        for (final id in widget.keys.value)
          if (isUnderPath(widget.layerId, id)) id,
      };

  @override
  void initState() {
    super.initState();
    widget.keys.addListener(_follow);
  }

  @override
  void didUpdateWidget(covariant _LayerKeys old) {
    super.didUpdateWidget(old);
    if (old.keys != widget.keys) {
      old.keys.removeListener(_follow);
      widget.keys.addListener(_follow);
    }
    // The area is rebuilding anyway — an edit, a zoom — so this is the free
    // moment to take the current share rather than hold one the notifier has
    // moved past.
    _mine = _read();
  }

  @override
  void dispose() {
    widget.keys.removeListener(_follow);
    super.dispose();
  }

  void _follow() {
    final next = _read();
    if (setEquals(next, _mine)) return;
    setState(() => _mine = next);
  }

  @override
  Widget build(BuildContext context) => widget.builder(context);
}
