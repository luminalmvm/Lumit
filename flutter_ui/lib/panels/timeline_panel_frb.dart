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
//
// **What is left in this file** is the panel and the one state that holds the
// whole of it: what is twirled open, what is selected, the zoom and the two
// scroll positions, the keyboard, and the three build methods the halves are
// laid out in. Everything it draws with lives beside it, a file per piece —
// the shared numbers in timeline_metrics_frb.dart, then the toolbar, the
// outline and its row, the fold rows, the lane area, one lane, the key block,
// the bar and the bottom bar. This file re-exports the lot, so every name that
// was reachable through it before the split still is.

import 'dart:collection';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show exportFrb;
import '../shell/precompose_dialog_frb.dart' show showPrecomposeDialogFrb;
import '../state/comp_model.dart';
import '../state/dock.dart';
import '../state/drag_payloads.dart';
import '../state/settings.dart';
import '../state/timeline_columns.dart';
import '../state/tools.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
// The ruler helpers moved with the ruler (shared with the graph editor); the
// re-export keeps their long-standing import path alive for their tests.
export 'timeline_extras_frb.dart' show rulerLabelStepSeconds, rulerLabelOf;

import 'timeline_metrics_frb.dart';
import 'timeline_layer_rows_frb.dart';
import 'timeline_bar_frb.dart';
import 'timeline_toolbar_frb.dart';
import 'timeline_outline_frb.dart';
import 'timeline_lane_area_frb.dart';
import 'timeline_key_lane_frb.dart';
import 'timeline_lane_bottom_bar_frb.dart';

import 'package:lumit_flutter/src/rust/api/project.dart';
import 'ease_popover.dart';
import 'easing_curve.dart';
import 'key_block.dart';
import 'easing_editor.dart';
import 'graph_editor_frb.dart';
import 'graph_maths.dart';
import 'graph_panel.dart' show DrivenParam, drivenParamsOf;
import 'timeline_extras_frb.dart';
import 'timeline_navigator.dart';
import 'sequence_view_frb.dart';
import 'spectral_lane_frb.dart';
import 'timeline_razor.dart';
import 'layer_fold_frb.dart';
import 'package:lumit_flutter/src/rust/api/retime.dart';
import '../widgets/smooth_zoom.dart';
import '../widgets/zoom_anchored_scroll.dart';
import 'waveform_frb.dart';
import 'timeline_group_row_frb.dart';
import 'transform_rows_frb.dart';

// The parts this file was split into (the split rule, K-007): every name they
// hold was reachable through this file before the split, and still is.
export 'timeline_metrics_frb.dart';
export 'timeline_layer_rows_frb.dart';
export 'timeline_mask_rows_frb.dart';
export 'timeline_shape_rows_frb.dart';
export 'timeline_retime_row_frb.dart';
export 'timeline_bar_frb.dart';
export 'timeline_toolbar_frb.dart';
export 'timeline_outline_frb.dart';
export 'timeline_outline_row_frb.dart';
export 'timeline_lane_area_frb.dart';
export 'timeline_key_lane_frb.dart';
export 'timeline_key_block_frb.dart';
export 'timeline_lane_bottom_bar_frb.dart';

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

  /// Which **layer groups** are folded shut (K-702), by group id. Session
  /// state, held beside [_open] and for the same reason: a fold changes how
  /// many rows the table has, and the outline and the lanes have to leave room
  /// for exactly the same ones. Not document state — whether a band is twirled
  /// open is no more part of the composition than whether a layer is.
  final Set<String> _foldedGroups = {};

  /// Whether [id]'s twirl is down.
  ///
  /// One set, since K-529: the flat-sheet twirl set went with Keys mode and
  /// with the graph's own outline — both views that opened every layer by
  /// default, and neither of which exists now. Both remaining views are the
  /// Layers outline, where shut-by-default is the right answer.
  bool _isOpen(String id) => _open.contains(id);

  /// Open or shut one twirl. The paths reach below the layer
  /// (`<layer>/transform` and the rest).
  ///
  /// Exactly this path: shutting a group leaves what was open *inside* it
  /// remembered, so twirling it back down finds it as it was.
  void _setOpen(String path, bool open) {
    // Whatever this path belongs to is being twirled by hand or by another
    // reveal, so it stops answering the last single `U` (K-622).
    _revealed.remove(path.split('/').first);
    if (open) {
      _open.add(path);
    } else {
      _open.remove(path);
    }
  }

  /// Shut [id] and forget everything opened inside it — what a reveal key does
  /// before it opens the rows it names, so a reveal shows what it says rather
  /// than adding to whatever the last one left open.
  void _shutLayerDeep(String id) {
    _revealed.remove(id);
    _open.removeWhere((p) => p == id || isUnderPath(id, p));
  }

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

  /// Each spectral-mode layer's spectrogram window (K-699), and what it was
  /// fetched for — the peaks' own bargain, for the other picture. A layer
  /// holds one or the other, never both: the mode decides which fetch runs,
  /// and the loser's entry is dropped so a long session does not keep two
  /// summaries of every lane it has looked at.
  final Map<String, BridgeSpectrogram> _spectra = {};
  final Map<String, String> _spectraKeys = {};

  /// A lane-mode chip changed (K-699): fetch what the new mode needs. No
  /// rebuild here — the chips and the lanes listen to [laneModes] themselves,
  /// which is what keeps the toggle off the rest of the table.
  void _onLaneMode() {
    if (!mounted) return;
    _refreshPeaks(_lastLayers);
  }

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

  /// Whether a layer's name is written along its bar — Settings ▸ Interface ▸
  /// Panels, off by default (K-514). Read here, once per build of the panel,
  /// and handed down to the bars.
  bool get _barNames => Provider.of<LumitUiState>(context, listen: false)
      .workspace
      .interface
      .layerNamesOnBars;

  /// What the chrome says — Settings ▸ Appearance ▸ Interface ▸ *Chrome
  /// labels* (K-440), read once per build of the panel and handed down.
  ///
  /// Held in a field rather than looked up by each toggle: the column toggles
  /// rebuild on every hover in the bar, and a settings lookup per toggle per
  /// hover is exactly the pattern K-184 keeps out of the paint path.
  ChromeLabels _chromeLabels = ChromeLabels.icons;

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
    // What a layer with no chip choice of its own shows (K-699) — read here,
    // once, rather than in every waveform row's build (K-184).
    laneModes.stackDefault = multiwave;
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
        _spectra.remove(id);
        _spectraKeys.remove(id);
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
      // The lane's mode decides which picture is fetched (K-699): the
      // spectrogram in spectral mode, the peaks otherwise — never both.
      final mode = laneModes.of(id);
      if (mode == LaneMode.spectral) {
        _peaks.remove(id);
        _peakKeys.remove(id);
        final key = '${request.key}$retimed';
        if (_spectraKeys[id] == key) continue;
        _spectraKeys[id] = key;
        entry.layer
            .audioSpectrogram(
          startSeconds: request.startSeconds,
          endSeconds: request.endSeconds,
          columns: request.buckets,
        )
            .then((grid) {
          if (!mounted || _spectraKeys[id] != key) return;
          setState(() => _spectra[id] = grid);
        });
        continue;
      }
      _spectra.remove(id);
      _spectraKeys.remove(id);
      final bands = mode == LaneMode.stack;
      final key = '${request.key}|$bands$retimed';
      // Claimed before the fetch starts, so a rebuild mid-decode does not ask
      // twice for the same window.
      if (_peakKeys[id] == key) continue;
      _peakKeys[id] = key;
      entry.layer
          .audioPeaks(
        startSeconds: request.startSeconds,
        endSeconds: request.endSeconds,
        buckets: request.buckets,
        multiwave: bands,
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
    if (_peakKeys.isEmpty && _peaks.isEmpty && _spectraKeys.isEmpty) return;
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
    final driven = <String, Map<String, DrivenParam>>{};
    for (final entry in layers) {
      final id = entry.layer.internallayerId.toString();
      try {
        if (entry.info.flow) flowParams[id] = entry.layer.getFlowParams();
        // Off the read model (K-680), where the Flow rate has always been: it
        // was a `get_volume_db` per sounding layer on every document revision.
        if (_hasAudio[id] ?? false) volumeDb[id] = entry.info.volumeDb;
        // Which parameters a wire is deciding (K-471, K-627), so a fold-out
        // row draws its *driven* mark where its stopwatch would be. Only a
        // layer with an effect stack can have one, and the answer rides down
        // on the row rather than being asked for per rebuild.
        //
        // **And only a layer with a wire in it at all** (K-680). Reading the
        // graph is the most expensive question on this walk, and it was asked
        // of every layer carrying effects on every document revision — 49
        // crossings and 17 ms per click on the owner's project — to hear "no
        // wires" from every one of them. The model says so for nothing.
        if (entry.info.wired && entry.info.effects.isNotEmpty) {
          final wired = drivenParamsOf(entry.layer);
          if (wired.isNotEmpty) driven[id] = wired;
        }
      } catch (_) {
        // A layer gone between the model read and this: its rows go too.
      }
    }
    _flowParams = flowParams;
    _volumeDb = volumeDb;
    _driven = driven;
  }

  /// Per-layer answers the fold rows carry (K-184) — see [_refreshBounds].
  Map<String, BridgeFlowParams> _flowParams = {};
  Map<String, BridgeScalar> _volumeDb = {};
  Map<String, Map<String, DrivenParam>> _driven = {};

  /// The work area, held between document revisions — see the note in [_body].
  ({int start, int end, bool whole})? _workArea;

  /// The ruler's staged span while a work-area edge is mid-drag: substituted
  /// for the document's below, so the lane and graph highlights move with the
  /// hand while the write still lands once, on release.
  ///
  /// **A notifier, not a field the panel rebuilds on** (K-626's pattern). The
  /// grounds that draw the band listen to it for themselves ([WorkAreaGround]),
  /// so a pointer move repaints three washes instead of rebuilding the whole
  /// Timeline — the outline, every row, the lanes, the key counts and the snap
  /// targets — which is what made an edge drag crawl on a real project. Its
  /// value is still read at build time, so a rebuild that happens mid-drag for
  /// its own reasons draws the staged span rather than jumping back.
  final ValueNotifier<({int start, int end, bool whole})?> _workPreview =
      ValueNotifier(null);
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
          // Off the read model (K-680), already at *this* comp's rate. It was
          // a `get_source_item` and a `get_settings` per precomp layer, run
          // again on every document revision — the two calls this walk cost
          // that the engine's own model walk already had in hand
          // (docs/impl/ui-performance.md §4.5).
          sourceFrames = info.sourceFrames;
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
  ///
  /// **A twirl on a selected row moves every selected row with it** (§6.4, the
  /// rule the Effect controls' twirls follow): five layers picked out and one
  /// of their twirls clicked opens all five, and they all take the clicked
  /// row's new state, so a mixed set comes out even rather than inverted row
  /// by row. A twirl on a row that is *not* in the selection is still about
  /// that row alone — clicking something unselected has never meant "and the
  /// selection too".
  void _toggle(String path) => setState(() {
        final opening = !_isOpen(path);
        for (final row in rowsTwirledWith(path, _twirlSelection())) {
          _setOpen(row, opening);
          if (!opening) _dropSelectionUnder(row);
        }
      });

  /// Every row a twirl could act on: the selected layers and the selected
  /// properties, as the paths [_open] is keyed by.
  Set<String> _twirlSelection() => {
        for (final id in _ui?.selectedLayerIds ?? const <UuidValue>{})
          id.toString(),
        ..._selectedProperties,
      };

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
  ///
  /// **Published, not `setState`** (docs/impl/ui-performance.md §4.4). A layer
  /// click used to redraw the whole panel to move one row's lit state: measured
  /// at one 39–67 ms build frame on the owner's comp, which is four frames of
  /// the 8.3 ms budget for a click that changes the shading of two blocks. The
  /// rows and the bars listen for their own slice of [TimelineSelection]
  /// instead, so what redraws is the layer let go of and the layer taken up.
  /// Graph mode is the exception the property paths already carve out: the
  /// picked properties *are* its curves, and dropping them gives it a new
  /// picture to draw.
  /// Everything a **layer group's** two rows can be asked to do (K-702), built
  /// once per build and handed to both halves so the header and its combined
  /// bar act on one set rather than two that agree.
  ///
  /// Every one of them is a forward: the engine decides what grouping means,
  /// what a broadcast switch does and how far a drag may travel, and the panel
  /// only says which group and how much. The single exception is the fold
  /// itself, which is session state and has no business in the document.
  GroupActions _groupActions(LumitUiState ui, CompositionReference comp) =>
      GroupActions(
        onToggleFold: (id) => setState(() {
          if (!_foldedGroups.remove(id)) _foldedGroups.add(id);
        }),
        // Choosing the header chooses the band, which is what makes every
        // command that runs over the selection — the stack drag included —
        // reach the whole group without a second road for groups.
        onSelect: (g) {
          final byId = {
            for (final e in ui.model.layers) e.layer.internallayerId: e.layer,
          };
          ui.setSelection([
            for (final m in g.members)
              if (byId[m] != null) byId[m]!,
          ]);
          // Published, not `setState` — [_selectLayer]'s own rule (WP-2): the
          // rows and bars listen for their slice of [TimelineSelection], so
          // what redraws is the band lit and the layers let go. The header
          // click used to rebuild the whole panel on top of the publish,
          // which is why choosing a group was "far slower than selecting any
          // other layer" (owner, 2026-08-31). Graph mode keeps the rebuild
          // the layer path keeps: the picked properties are its picture.
          _publishRowSelection();
          _publishLaneKeys();
          if (_graph) setState(() {});
        },
        onRename: (g, name) {
          comp.setGroupName(group: g.id, name: name);
          ui.model.refresh();
        },
        onLabel: (g, label) {
          comp.setGroupLabel(group: g.id, label: label);
          ui.model.refresh();
        },
        onSwitch: (g, which, on) {
          comp.setGroupSwitch(group: g.id, switch_: which, on_: on);
          ui.model.refresh();
        },
        onUngroup: (g) {
          comp.ungroup(group: g.id);
          _foldedGroups.remove(g.id.toString());
          ui.model.refresh();
        },
        // The heavy fold, from the light one's own menu: the group's members
        // handed to the precompose dialogue that already existed, so there is
        // one implementation of "pack these into a comp" rather than a
        // group-shaped copy of one.
        onPrecompose: (g) {
          final byId = {
            for (final e in ui.model.layers) e.layer.internallayerId: e.layer,
          };
          final layers = [
            for (final m in g.members)
              if (byId[m] != null) byId[m]!,
          ];
          if (layers.isEmpty) return;
          showPrecomposeDialogFrb(
            context: context,
            comp: comp,
            selectedLayers: layers,
            ui: ui,
            workspace: ui.workspace,
          );
        },
        onShift: (g, delta) {
          comp.shiftGroup(group: g.id, delta: delta);
          ui.model.refresh();
        },
      );

  void _selectLayer(LumitUiState ui, LayerReference? layer,
      {List<BridgeLayerEntry> among = const []}) {
    _aimLayerSelection(ui, layer, among);
    _publishRowSelection();
    _publishLaneKeys();
    if (_graph) setState(() {});
  }

  /// The click rules themselves, so [_selectLayer] can say what happens after
  /// them without a closure around every early return.
  void _aimLayerSelection(
      LumitUiState ui, LayerReference? layer, List<BridgeLayerEntry> among) {
    if (ui.selectedLayer.value?.internallayerId != layer?.internallayerId) {
      // The one place this is decided, shared with the listener that catches a
      // selection made in the Viewer (K-275). The highlight goes with the
      // property selection it belongs to: left behind, the previous layer's
      // row stayed lit after a click on a different layer.
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
      final a = among
          .indexWhere((e) => e.layer.internallayerId == held.internallayerId);
      final b = among
          .indexWhere((e) => e.layer.internallayerId == layer.internallayerId);
      if (a >= 0 && b >= 0) {
        // The clicked layer stays the primary — it is the one just asked for,
        // and everything that acts on one layer acts on that.
        ui.setSelection([
          layer,
          for (var i = a < b ? a : b; i <= (a < b ? b : a); i++)
            if (i != b) among[i].layer,
        ]);
        return;
      }
    }
    ui.setSelection([layer]);
  }

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
  /// **Each toggle hides the columns its own word names** (owner, desktop
  /// testing). Switches is the A/V cluster, Modes is the render cluster, and
  /// Parent is the parent picker — which used to be the whole compose cluster,
  /// so pressing Parent took the matte and the blend with it. The matte and
  /// the blend have no toggle of their own: the mockup's bottom bar carries
  /// these three words and no more.
  static const List<TimelineGroup> _toggleableGroups = [
    TimelineGroup.switches,
    TimelineGroup.render,
    TimelineGroup.parent,
  ];

  /// The width a seam drag is currently showing, or null when no seam is being
  /// dragged (T4). **The outline follows the hand**: the rows redraw at this
  /// width as the drag moves — which is what "the Layers column updates live"
  /// asks for, and what makes a switch column's hide ladder legible, since the
  /// cells going away are the whole of what the drag does.
  ///
  /// A notifier rather than `setState` because only the outline half depends on
  /// a column width. The panel's own rebuild — every lane, every bar, every
  /// waveform — is what made this drag lag when it was tried before, and none
  /// of that is left of the seam.
  final ValueNotifier<MapEntry<TimelineGroup, double>?> _liveResize =
      ValueNotifier(null);

  /// Where a seam drag of [delta] would put [group]: never past what its cells
  /// need or can use, and settled on the nearest whole cell or on the width it
  /// shipped at ([snapGroupWidth]).
  double _resizedWidth(TimelineGroup group, double delta) =>
      snapGroupWidth(group, (_groupWidths[group] ?? 0) + delta);

  /// Widen (or narrow) one group — and not at all for a fixed-width group (the
  /// render-time readout, sized for its own number).
  void _resizeGroup(TimelineGroup group, double delta) => setState(() {
        _liveResize.value = null;
        if (groupIsFixedWidth(group)) return;
        _groupWidths = {..._groupWidths, group: _resizedWidth(group, delta)};
      });

  /// The same width, mid-drag: published for the outline to draw and not
  /// written to [_groupWidths] until the hand lets go, so a cancelled drag
  /// leaves the column where it was.
  void _liveResizeGroup(TimelineGroup group, double? delta) =>
      _liveResize.value = delta == null || groupIsFixedWidth(group)
          ? null
          : MapEntry(group, _resizedWidth(group, delta));

  /// The drawn widths with a seam drag's live width in place of the stored
  /// one. The matte toggles' room is added back the same way the build adds
  /// it, so a dragged compose column keeps its two pickers in step (K-463).
  Map<TimelineGroup, double> _liveWidths(Map<TimelineGroup, double> widths,
          MapEntry<TimelineGroup, double>? live, bool matteToggles) =>
      live == null || !widths.containsKey(live.key)
          ? widths
          : {
              ...widths,
              live.key: live.value +
                  (live.key == TimelineGroup.compose && matteToggles
                      ? matteToggleWidth
                      : 0),
            };

  /// The layer whose fold-out was last touched — drawn a shade dimmer than
  /// the selected layer, so "which layer do these rows belong to" has an
  /// answer at a glance without stealing the selection.
  String? _highlighted;

  /// The selected properties, as fold paths (`<layer>/effects/<fx>/<param>`),
  /// in selection order — clicking a property's name selects it, Ctrl+click
  /// toggles it, Shift+click extends the range, across layers (docs/07 §4.3,
  /// §5). Each is a coloured curve in the graph editor.
  final List<String> _selectedProperties = [];

  /// The same two answers, published for the rows to listen to
  /// ([TimelineSelection]). Kept beside the fields rather than replacing them
  /// because every rule in this panel reads and edits the list in place; this
  /// is the snapshot the outline watches, and it is what makes a click repaint
  /// the rows whose selectedness changed instead of the whole Timeline.
  final ValueNotifier<TimelineSelection> _rowSelection =
      ValueNotifier(const TimelineSelection());

  /// Each selected path's graph line colours, keyed by path.
  Map<String, List<Color>> _colourOfChannels(List<GraphChannel> channels) {
    final t = ThemeScope.of(context).theme;
    final out = <String, List<Color>>{};
    for (final channel in channels) {
      (out[channel.path] ??= [])
          .add(t.curve[channel.colourIndex % t.curve.length]);
    }
    return out;
  }

  /// Hand the rows the selection as it now stands. Silent when nothing they
  /// draw has changed, so a publish that says the same thing costs no repaint.
  void _publishRowSelection([Map<String, List<Color>>? colours]) {
    final next = TimelineSelection(
      // The shell's list (K-217) as the rows read it: strings, because that is
      // what a block is keyed by on both sides of the table.
      layers: {
        for (final layer in _ui?.selectedLayers.value ?? const [])
          layer.internallayerId.toString(),
      },
      properties: List<String>.unmodifiable(_selectedProperties),
      highlighted: _highlighted,
      colours: colours ?? _colourOfChannels(_channelsNow()),
    );
    final held = _rowSelection.value;
    if (held.highlighted == next.highlighted &&
        setEquals(held.layers, next.layers) &&
        listEquals(held.properties, next.properties) &&
        _sameColours(held.colours, next.colours)) {
      return;
    }
    _rowSelection.value = next;
  }

  static bool _sameColours(
      Map<String, List<Color>> a, Map<String, List<Color>> b) {
    if (a.length != b.length) return false;
    for (final entry in a.entries) {
      if (!listEquals(entry.value, b[entry.key])) return false;
    }
    return true;
  }

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
        out.add((y, y + row.rowHeight + extra));
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

  /// How many keyframes each of those rows draws, by path. Rebuilt by every
  /// build, beside [_visiblePropertyPaths] and from the same rows.
  Map<String, int> _visibleKeyCounts = const {};

  /// Every lane key id belonging to [path] — `path#0`, `path#1`, and so on.
  ///
  /// A row's keys are numbered by their place in the row's own list, which is
  /// the same numbering the marquee, the lanes and the block tools all use, so
  /// a selection made from a name and one made from a box are the same thing.
  Set<String> _keysOfProperty(String path) => {
        for (var i = 0; i < (_visibleKeyCounts[path] ?? 0); i++) '$path#$i',
      };

  /// Select [path] by click: plain replaces, Ctrl toggles, Shift extends from
  /// the last selected along the visible rows. Marks its layer either way.
  ///
  /// **A property's name is its row's "select all"** (K-500 §2.1): picking a
  /// row picks its keyframes too, so the block box, the Ease popover and the
  /// F9 family act on the row that was just clicked without the user drawing a
  /// box round diamonds they can already see are all of them. Each of the three
  /// click rules carries its keys the same way it carries its rows — Ctrl
  /// toggles the row's keys in and out of the standing selection, Shift takes
  /// the keys of every row the run passes over.
  ///
  /// The stopwatch and the value well are not this gesture: they animate and
  /// they edit, and neither re-aims the selection (K-196).
  /// **Published, not `setState`.** A click on a name lights a row and fills
  /// that row's diamonds, and nothing else on screen can say anything about
  /// either — so the two halves of the table listen for their own share and
  /// redraw that (measured: 1144 widgets down to a fraction of it). Graph view
  /// is the exception, as it is for an effect picked over in the Effect
  /// controls panel: the picked properties *are* its curves, so it has a new
  /// picture to draw.
  void _selectProperty(String path) {
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed || keys.isMetaPressed) {
      if (_selectedProperties.remove(path)) {
        _laneKeySelection.removeAll(_keysOfProperty(path));
      } else {
        _selectedProperties.add(path);
        _laneKeySelection.addAll(_keysOfProperty(path));
      }
    } else if (keys.isShiftPressed && _selectedProperties.isNotEmpty) {
      final a = _visiblePropertyPaths.indexOf(_selectedProperties.last);
      final b = _visiblePropertyPaths.indexOf(path);
      if (a < 0 || b < 0) {
        if (!_selectedProperties.contains(path)) {
          _selectedProperties.add(path);
        }
        _laneKeySelection.addAll(_keysOfProperty(path));
      } else {
        for (var i = a < b ? a : b; i <= (a < b ? b : a); i++) {
          if (!_selectedProperties.contains(_visiblePropertyPaths[i])) {
            _selectedProperties.add(_visiblePropertyPaths[i]);
          }
          _laneKeySelection.addAll(_keysOfProperty(_visiblePropertyPaths[i]));
        }
      }
    } else {
      _selectedProperties
        ..clear()
        ..add(path);
      _laneKeySelection
        ..clear()
        ..addAll(_keysOfProperty(path));
    }
    _graphKeySelection.clear();
    _highlighted = layerIdOfPath(path) ?? _highlighted;
    _openRetimeInItsDefaultLens(path);
    _publishEffectSelection();
    _publishPropertySelection();
    _publishRowSelection();
    _publishLaneKeys();
    if (_graph) setState(() {});
  }

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
    _selectedProperties
      ..clear()
      ..addAll(wanted);
    _graphKeySelection.clear();
    if (owner != null) _highlighted = owner;
    // **Published, not `setState`.** An effect picked over in the Effect
    // controls panel changes which rows are lit and nothing else, so the rows
    // whose lighting changed are what redraws (measured: 858 widgets down to a
    // handful). Graph view is the exception — the picked properties *are* its
    // curves, so it has a new picture to draw.
    _publishRowSelection();
    if (_graph) setState(() {});
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
  ///
  /// Published rather than `setState` for the same reason [_selectProperty] is,
  /// and it matters here as much: a plain click on a property's name *is* this
  /// gesture first — the press picks the row before the tap does (K-334) — so
  /// leaving it a panel-wide redraw would have left the click costing one
  /// anyway.
  void _selectOnEdit(String path) {
    if (_selectedProperties.contains(path)) return;
    _selectedProperties
      ..clear()
      ..add(path);
    _graphKeySelection.clear();
    _highlighted = layerIdOfPath(path) ?? _highlighted;
    _publishPropertySelection();
    _publishRowSelection();
    if (_graph) setState(() {});
  }

  /// Which of the two views is up (K-529, §12A.1).
  ///
  /// One field rather than a flag per mode: two booleans can say "both at
  /// once", and a state that cannot be drawn is a state that will one day be
  /// reached. The graph editor replaces the layer area rather than sitting
  /// beside it — it wants the same width, and a curve squeezed into half a
  /// panel is not a curve you can shape.
  TimelineMode _mode = TimelineMode.layers;

  /// Graph mode, as everything downstream of the swap has always asked it.
  bool get _graph => _mode == TimelineMode.graph;

  /// Switch views. The easing claim follows, because which panel owns the
  /// selected keys' easing depends on which view is up.
  void _setMode(TimelineMode mode) {
    if (mode == _mode) return;
    setState(() => _mode = mode);
    _publishEasingClaim();
  }

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
        final showing = _isOpen(id) &&
            wanted.every(_open.contains) &&
            !_open.any((p) => isUnderPath(id, p) && !wanted.contains(p));
        // Every reveal starts from the layer closed, so it shows what it says
        // rather than adding to whatever the last one left open.
        _shutLayerDeep(id);
        _dropSelectionUnder(id);
        if (showing) continue;
        _setOpen(id, true);
        _open.addAll(wanted);
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
        _setOpen(id, true);
        _open.addAll(_revealPaths(id, entry, action));
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
        for (final group in transformGroups(
            threeD: entry.info.switches.threeD, modes: entry.info.axisModes))
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

  /// What a move drag starting on a selected bar carries (K-720): every
  /// **unlocked** selected layer — a locked one sits still, the way it sits
  /// out a switch batch — and the earliest in frame among them, the wall the
  /// whole set stops at. Asked from the drag's start handler, never from a
  /// build, so it reads the selection of the moment the hand closed.
  SelectionMove _selectionMove(LumitUiState ui) {
    final picked = ui.selectedLayerIds;
    final ids = <UuidValue>[];
    int? minIn;
    for (final entry in ui.model.layers) {
      if (!picked.contains(entry.layer.internallayerId)) continue;
      if (entry.info.switches.locked) continue;
      ids.add(entry.layer.internallayerId);
      final inFrame = entry.info.inFrame.toInt();
      if (minIn == null || inFrame < minIn) minIn = inFrame;
    }
    return SelectionMove(ids, minIn ?? 0);
  }

  /// The block stretch in flight (K-458), on the same terms and for the same
  /// reason: the box and every lane it crosses have to move together, and only
  /// they need to repaint while a handle is being dragged.
  final ValueNotifier<KeyStretch?> _keyStretch = ValueNotifier(null);

  /// The lane view's selected keyframes, as `rowId#index` (docs/07 §4.3) —
  /// what the marquee gathered. Session state, like the twirl set.
  final Set<String> _laneKeySelection = {};

  /// The same set, published for the lanes to listen to — the lane half's
  /// [_rowSelection], and there for the same reason. Kept beside the field
  /// rather than replacing it because every rule in this panel edits the set
  /// in place.
  ///
  /// What it carries is a **view onto the live set**, not a copy of it: a key
  /// drag captures the selection the instant the pointer goes down, after the
  /// press has just added the key under it, and a copy taken at the last build
  /// would be the selection as it stood before that press. So the lanes read
  /// through to the set itself, and this only says *when* to look again.
  late final ValueNotifier<Set<String>> _laneKeys =
      ValueNotifier(UnmodifiableSetView(_laneKeySelection));

  /// What was last published, so a publish that says the same thing can be
  /// told from one that does not — a view onto the live set cannot compare
  /// itself against the set it is a view of.
  Set<String> _laneKeysPublished = const {};

  /// Hand the lanes the key selection as it now stands. Silent when it says
  /// the same thing, so a publish that changes nothing costs no repaint. A
  /// fresh view each time, because a notifier holding the value it already has
  /// tells nobody anything.
  void _publishLaneKeys() {
    if (setEquals(_laneKeysPublished, _laneKeySelection)) return;
    _laneKeysPublished = Set<String>.of(_laneKeySelection);
    _laneKeys.value = UnmodifiableSetView(_laneKeySelection);
  }

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

  /// The **Animated filter** (K-441, 6.43): with it on the outline lists only
  /// the rows that carry keyframes, across every layer; off, the twirl-down
  /// lists come back exactly as they were.
  ///
  /// Off by default, because Layers mode is the layer stack first and a filter
  /// on at rest is a panel that appears to be missing rows. The strip's answer
  /// covers the whole comp and stays on until it is switched off; a single `U`
  /// asks the same question of the layers it revealed and lets go of them the
  /// moment anything else touches their twirls ([_revealed]).
  bool _animatedOnly = false;

  /// Layers a reveal opened onto **the rows it named alone**, and which rule
  /// each was opened by (K-622, K-684).
  ///
  /// The reveal cycle used to open a layer's *groups* — Transform, the effects,
  /// Audio — and a group opens whole, so revealing one keyed Intensity also
  /// unrolled every other parameter on that effect and every transform property
  /// beside a keyed one. `U` names the property, not the heading, so it filters
  /// the rows the way the Animated strip does and stops at the keys. The menu's
  /// three Reveal rows are the same machinery under a wider rule each, which is
  /// why the filter is stored per layer rather than the layer merely marked.
  ///
  /// Dropped per layer by [_setOpen] and [_shutLayerDeep], which is every twirl
  /// a hand or another reveal key can turn: a layer someone has started opening
  /// by hand is no longer showing the answer to a `U`, and going on filtering it
  /// would make the caret look broken.
  final Map<String, RevealFilter> _revealed = {};

  /// The comp's size when the last **modified** reveal was run, kept for the
  /// builds after it: that reveal asks whether a layer has been moved, and
  /// unmoved means the middle of the comp — a fact about the document that a
  /// build may not go and ask for (K-681).
  double _revealCompWidth = 0;
  double _revealCompHeight = 0;

  bool _syncingScroll = false;

  /// The zoom `\` came away from, so pressing it again goes back there. Null
  /// until it has been used, and dropped by any other zoom — going back to a
  /// magnification the user has since left is not what "the previous zoom"
  /// means.
  double? _zoomBeforeFit;

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
    // A lane-mode chip flipping to spectral needs data the peaks fetch never
    // asked for (K-699). Fetch only: the lanes and chips listen to the store
    // themselves, so the toggle repaints them without this panel rebuilding.
    laneModes.addListener(_onLaneMode);
    HardwareKeyboard.instance.addHandler(_onKey);
    // Claim Delete for the finer selection this panel holds (K-234). The state
    // is kept, not looked up again: `dispose` runs after the element is
    // deactivated, where an ancestor lookup is no longer safe.
    _ui = Provider.of<LumitUiState>(context, listen: false);
    _ui!.deleteClaim = _deleteClaim;
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
    _ui!.selectedLayers.addListener(_onLayerSelectionChanged);
    // Switching measuring off takes the render-time column away entirely, and
    // the outline is that much narrower for it — a layout change, so the panel
    // has to hear about it rather than only the cells inside the column.
    _ui!.renderTimings.addListener(_onTimingsChanged);
    // The FX console's Keyframe ring plants a key and then asks for its row to
    // be on screen (K-326). Ensure-open, not the reveal keys' toggle: showing
    // a row that is already showing must never hide it.
    _ui!.revealPropertyRequest.addListener(_onRevealRequested);
    // Animation ▸ Reveal … (K-684): the menu says which reveal, the panel runs
    // it over the selection, exactly as `U` does.
    _ui!.revealFilterRequest.addListener(_onRevealFilterRequested);
    _ui!.selectPropertyRequest.addListener(_onSelectPropertyRequested);
    // Merged **once**, not per build: a fresh `Listenable` every rebuild makes
    // every cache bar under it unsubscribe and resubscribe, which during a zoom
    // flight is sixty times a second for nothing (K-293).
    _cacheRevision = Listenable.merge([_ui!.frameArrived, _ui!.cacheChanged]);
    // Edge-follow: the playhead stays on screen while the transport runs
    // (docs/07 §4.6).
    _ui!.playheadFrame.addListener(_edgeFollow);
  }

  /// Keep the playhead in view during playback (docs/07 §4.6).
  ///
  /// **Page-flip**: when the playhead leaves the viewport the lanes jump so it
  /// lands back at the left edge, and the next page plays out under a still
  /// picture. A view that scrolled a pixel per frame instead would put the
  /// whole timeline in motion for the whole of playback, which is the harder
  /// thing to read — and it is the flip that After Effects' own default does.
  ///
  /// **Only while playing.** Scrubbing stops the transport (K-254), so this
  /// never fights a hand: the spec's "MUST NOT recentre while the user is
  /// dragging anything" is kept by the transport being off whenever anything
  /// is being dragged.
  void _edgeFollow() {
    final ui = _ui;
    if (ui == null || !ui.playing.value || !mounted) return;
    final position = positionOf(_hLane);
    if (position == null || position.maxScrollExtent <= 0) return;
    final viewport = position.viewportDimension;
    final span = viewport + position.maxScrollExtent - TimelineAxis.pad * 2;
    if (_laneFrames <= 0 || span <= 0) return;
    final x = TimelineAxis.pad + ui.playheadFrame.value * span / _laneFrames;
    final at = x - position.pixels;
    if (at >= 0 && at <= viewport) return;
    // The playhead to the left edge, one padding in so its head is whole.
    _hLane.jumpTo((x - TimelineAxis.pad).clamp(0.0, position.maxScrollExtent));
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
    _dropLayerLocalSelection();
    // Published rather than `setState`, for [_selectLayer]'s reason: this is
    // the same click arriving from somewhere else — the Viewer's picture, a
    // keyboard command — and it lights the same two blocks.
    _publishRowSelection();
    _publishLaneKeys();
    if (_graph) setState(() {});
  }

  /// The selection changed without the primary moving: a select-all, a
  /// Ctrl+click adding a second layer, a Viewer marquee. The rows still have to
  /// hear it, and the primary's listener above cannot say so.
  void _onLayerSelectionChanged() {
    if (mounted) _publishRowSelection();
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

  /// The property paths the current key selection covers, **top to bottom**.
  ///
  /// In graph view that is simply the picked properties. In the lanes and in
  /// Keys mode it is read off the key selection itself, which the marquee fills
  /// by walking the rows in the order they are drawn — so the order survives,
  /// and Stagger's "top down" means what the outline shows.
  List<String> _selectionPaths() => _graph
      ? _selectedProperties
      : {
          for (final id in _laneKeySelection)
            if (id.lastIndexOf('#') > 0) id.substring(0, id.lastIndexOf('#'))
        }.toList();

  /// The channels those paths resolve to, against the read model as it stands.
  ///
  /// Re-resolved rather than remembered: an edit replaces a property's whole
  /// animation, so a channel held across one carries the curve as it was.
  List<GraphChannel> _selectionChannels(LumitUiState ui) =>
      graphChannels(layers: ui.model.layers, selected: _selectionPaths());

  /// The channels a **key command** acts on: the rows the key selection sits
  /// on where there is one, and the picked properties otherwise.
  ///
  /// One resolution for every key command (K-529). Copy and Paste used to ask
  /// [_channelsNow] instead — the *picked properties*, which is a different
  /// question in the lane views, where the selection speaks in row paths. The
  /// two answers agree while nothing has changed the property selection since
  /// the keys were picked, and part ways the moment anything has: editing a
  /// value on another row, picking an effect in the Effect controls panel, or
  /// any other route that re-aims [_selectedProperties] without touching
  /// [_laneKeySelection]. Copy then looked for the selected keys among
  /// channels they were not on, found none, and — this is the part that made
  /// it read as broken — **claimed the chord anyway**, leaving whatever was
  /// copied last on the clipboard for Paste to put down again.
  ///
  /// The interpolation buttons, the tangent modes and the Ease popover have
  /// always resolved it this way ([_selectionChannels]); Copy and Paste
  /// now do too, which is why they behave the way the rest of the strip does.
  List<GraphChannel> _commandChannels(LumitUiState ui) {
    final paths = _selectionPaths();
    if (paths.isEmpty) return _channelsNow();
    return graphChannels(layers: ui.model.layers, selected: paths);
  }

  /// The project the block tools group their writes into one undo step against
  /// (K-458). Null in a widget test with no project open.
  ProjectReference? get _project =>
      Provider.of<LumitState>(context, listen: false).project;

  /// How many keys the selection holds, as the badge and the Ease popover count
  /// them — one per *row* diamond, not one per axis, which is what the user
  /// picked up.
  int get _selectedKeyCount =>
      _graph ? _graphKeySelection.length : _laneKeySelection.length;

  /// A new lane key selection — from a box, a click, or a property's name.
  ///
  /// Picking keyframes picks their **properties** too, every distinct one the
  /// selection touches, so the outline and the graph show what is in hand
  /// (docs/07 §4.3). An empty selection leaves the properties alone: letting
  /// go of the keys is not letting go of the rows they sat on.
  void _onLaneKeysSelected(Set<String> keys) {
    setState(() {
      _laneKeySelection
        ..clear()
        ..addAll(keys);
      if (keys.isEmpty) return;
      _selectedProperties.clear();
      for (final id in keys) {
        final path = id.substring(0, id.lastIndexOf('#'));
        if (!_selectedProperties.contains(path)) {
          _selectedProperties.add(path);
        }
      }
      _highlighted = layerIdOfPath(_selectedProperties.first) ?? _highlighted;
    });
  }

  /// Remove the selected keys — the lane key menu's *Delete key* (K-500 §2.1),
  /// the same removal the graph's Delete makes, and what `Delete` itself does
  /// with lane keys in hand (6.6).
  ///
  /// One undo step however many rows it reaches across: a block deleted in one
  /// gesture comes back in one (K-458). Returns whether anything went, which is
  /// what lets the Delete claim answer honestly.
  bool _deleteSelectedKeys() {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final channels = _selectionChannels(ui);
    final selection = _actionKeySelection(channels);
    if (selection.isEmpty) return false;
    asOneUndoStep(
        _project,
        () => deleteKeysFromChannels(
            channels: channels, selectedKeys: selection));
    setState(_laneKeySelection.clear);
    ui.model.refresh();
    return true;
  }

  /// What `Delete` acts on in this panel, finest selection first (K-234).
  ///
  /// **Keyframes before masks before layers.** The shell asks this before it
  /// deletes anything, so a rung that answers `true` is a rung that has already
  /// done the work — and each rung is a selection strictly *inside* the one
  /// below it, which is what makes the order the only sane one: picking a
  /// keyframe and pressing Delete must not cost the layer it sits on.
  ///
  /// **Graph mode's own keys are the top rung.** They were answered on the
  /// hardware keyboard instead, which does not claim anything: every handler
  /// runs on every key, so deleting a graph key also let the shell delete the
  /// layer it belonged to.
  bool _deleteClaim() =>
      (_graph && _graphKeySelection.isNotEmpty && _deleteGraphKeys()) ||
      (!_graph && _laneKeySelection.isNotEmpty && _deleteSelectedKeys()) ||
      _deleteSelectedMasks();

  /// Delete the keys picked in the graph pane, if it is there to ask.
  bool _deleteGraphKeys() {
    final pane = _graphPane.currentState;
    if (pane == null) return false;
    pane.deleteSelectedKeys();
    return true;
  }

  /// The right-click menu on a lane keyframe (K-500 §2.1): the graph key's own
  /// menu — Linear / Easy ease / Hold / Delete key — plus *Ease…*, which opens
  /// the popover on whatever is selected.
  ///
  /// A right-click on an **unselected** key selects it first, so the menu acts
  /// on the thing that was clicked; on a selected one it acts on the whole
  /// selection, which is what makes it the block's menu as well as one key's.
  void _laneKeyMenu(String id, Offset position) {
    if (!_laneKeySelection.contains(id)) _onLaneKeysSelected({id});
    showMenuAt<void>(
      context: context,
      position: position,
      width: 170,
      rows: (close) => [
        MenuRow(
          key: const ValueKey('tl-key-menu-linear'),
          onPressed: () {
            close(null);
            _applyInterp(const BridgeSideInterp.linear());
          },
          child: Text(l10n.easeLinear),
        ),
        MenuRow(
          key: const ValueKey('tl-key-menu-ease'),
          onPressed: () {
            close(null);
            _applyInterp(easyEase);
          },
          child: Text(l10n.easeEasy),
        ),
        MenuRow(
          key: const ValueKey('tl-key-menu-hold'),
          onPressed: () {
            close(null);
            _applyInterp(const BridgeSideInterp.hold());
          },
          child: Text(l10n.easeHold),
        ),
        MenuRow(
          key: const ValueKey('tl-key-menu-shape'),
          onPressed: () {
            close(null);
            _openEasePopover(position);
          },
          child: Text(l10n.keyMenuEase),
        ),
        MenuRow(
          key: const ValueKey('tl-key-menu-delete'),
          onPressed: () {
            close(null);
            _deleteSelectedKeys();
          },
          child: Text(l10n.deleteKey),
        ),
      ],
    );
  }

  /// Open the Ease popover on the selection, anchored at [position] (K-458).
  ///
  /// Reached from the block badge, which sits where the drawing puts the
  /// popover, and from the Keys bottom bar's Ease. Nothing here shapes a curve
  /// — the popover hands back a shape and a stagger, and both land through the
  /// machinery the Easing panel and the graph editor already use.
  void _openEasePopover(Offset position) {
    if (_selectedKeyCount == 0) return;
    showEasePopover(
      context: context,
      position: position,
      count: _selectedKeyCount,
      onOpenGraph: () => _setMode(TimelineMode.graph),
      onApply: _applyEaseRequest,
    );
  }

  /// One press of the popover's Apply: the shape onto every span the selection
  /// covers, then the stagger — **one undo step for the pair**.
  ///
  /// The shape first, and the channels re-read between the two. A shape changes
  /// no key's *time*, so applying it cannot move an index out from under the
  /// selection; a stagger does move times, and running it first would leave the
  /// ease writing the times back as they were. Between them the read model is
  /// freshened, because the second write must build on the first rather than on
  /// the curves as they were before either.
  void _applyEaseRequest(EaseRequest request) {
    if (!mounted) return;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    asOneUndoStep(_project, () {
      var channels = _selectionChannels(ui);
      var selection = _actionKeySelection(channels);
      if (selection.isEmpty) return;
      applyEasingToSelection(
        channels: channels,
        selectedKeys: selection,
        curve: request.curve,
      );
      if (request.stagger == 0) return;
      ui.model.refresh();
      channels = _selectionChannels(ui);
      selection = _actionKeySelection(channels);
      staggerSelection(
        channels: channels,
        selectedKeys: selection,
        order: _selectionPaths(),
        step: request.stagger,
        direction: request.order,
        fps: ui.model.fps,
        fpsNum: fpsNum,
        fpsDen: fpsDen,
      );
    });
    ui.model.refresh();
  }

  /// Set the selected keys' easing (the F9 family and the bottom bar's
  /// Linear / Bezier / Hold): both sides, or one for ease-in/ease-out.
  void _applyInterp(BridgeSideInterp side,
      {bool inSide = true, bool outSide = true}) {
    // In lane view the selection speaks in row paths, so the channels have to
    // cover those too, not only the selected properties.
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final channels = _selectionChannels(ui);
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

  /// Put the selected keys' sides into a tangent mode — the bottom bar's
  /// Tangents Auto / Clamp / Free (§6.3). The selection is resolved exactly as
  /// [_applyInterp] resolves it, so the two runs of chips act on one set of
  /// keys.
  void _applyTangentMode(TangentMode mode) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final channels = _selectionChannels(ui);
    final selection = _actionKeySelection(channels);
    if (selection.isEmpty) return;
    applyTangentModeToSelection(
      channels: channels,
      selectedKeys: selection,
      mode: mode,
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
    final channels = _selectionChannels(ui);
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
      // The key it always was: it puts the graph up, and takes it down again
      // to the layers. Keys mode has no shortcut of its own — its tab is the
      // switch — so pressing this from the dope sheet opens the graph, which
      // is what "show the graph" has always meant.
      _setMode(_graph ? TimelineMode.layers : TimelineMode.graph);
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
    // The zoom keys (docs/07 §4.6): `=` in, `-` out, `\` between the whole
    // composition and wherever the zoom was before. They hold the **playhead**
    // still, because a key press has no pointer to zoom about — the same
    // answer the bottom bar's slider gives (K-293).
    if (action == 'timeline.zoom.in' || action == 'timeline.zoom.out') {
      _zoomBeforeFit = null;
      _setZoom(zoomNudged(_zoomMotion.target,
          inward: action == 'timeline.zoom.in', maxZoom: _maxZoom));
      return true;
    }
    if (action == 'timeline.zoom.fit') {
      // A toggle, the way After Effects' own `\` is: away from the whole comp
      // and back to it, keeping the magnification it left.
      final was = _zoomBeforeFit;
      if (_zoomMotion.target > 1) {
        _zoomBeforeFit = _zoomMotion.target;
        _setZoom(1);
      } else {
        _zoomBeforeFit = null;
        _setZoom((was ?? 1).clamp(1.0, _maxZoom));
      }
      return true;
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
    // *before* it deletes anything (K-234) — and the graph pane's own keys are
    // on that claim for the same reason. Copy and paste are claimed the same
    // way (K-300): they used to be compared against `Ctrl+C`/`Ctrl+V` here,
    // which was fine while the shell had no copy of its own and became a
    // double action the moment it did.

    if (!_graph) return false;

    if (action == 'graph.fit') {
      _graphPane.currentState?.fitNow();
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
    final channels = _commandChannels(ui);
    final selection = _actionKeySelection(channels);
    if (selection.isEmpty) {
      return copyChannels(comp: comp, channels: channels, fps: ui.model.fps);
    }
    // The answer, not an assumption: a copy that took nothing must not claim
    // the chord, or `Ctrl+C` is swallowed and the clipboard still holds
    // whatever was copied before — which is the shape "Copy does nothing" took
    // on the owner's desktop.
    return copySelectedKeys(
      comp: comp,
      channels: channels,
      selectedKeys: selection,
      fps: ui.model.fps,
    );
  }

  /// Paste claims it when there are channels to paste *into* and keyframes to
  /// paste — or when nothing else is on the clipboard at all, which is what
  /// leaves keyframe text copied out of another tool a way in.
  bool _pasteKeysIntoSelection() {
    if (!mounted) return false;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    // The same channels Copy took them from, resolved the same way — so a
    // copy and the paste that follows it are looking at one list.
    final channels = _commandChannels(ui);
    if (channels.isEmpty) return false;
    if (graphKeyClipboard.isEmpty && !ui.clipboard.isEmpty) return false;
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    pasteKeysAtPlayhead(
      channels: channels,
      playheadFrame: ui.playheadFrame.value,
      fps: ui.model.fps,
      fpsNum: fpsNum,
      fpsDen: fpsDen,
      // One press, one step, however many layers the clipboard reaches.
      project: _project,
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

  /// One press of the reveal key: `U` opens the **keyed rows**, `UU` opens
  /// every modified property whole, `UUU` shuts the layer again.
  ///
  /// The *counting* is ours, because a multi-tap is a gesture like a
  /// double-click and gestures are the frontend's. Which groups qualify is the
  /// engine's, and it is asked afresh on each tap — the answer depends on the
  /// document, and the document may have changed between taps.
  ///
  /// **The first tap stops at the keys** (K-622). It used to open the *groups*
  /// holding animation, and a group opens whole: one keyed Intensity unrolled
  /// the whole of Glow, and one keyed Position unrolled every transform
  /// property beside it, so "reveal animated properties" reliably showed rows
  /// with nothing on them. It now filters to the keyed rows themselves, keeping
  /// the headings above them — the Animated strip's own arithmetic
  /// ([animatedFoldRows]), asked of these layers rather than of the comp.
  /// `UU` is where everything still opens.
  bool _revealTap() {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    // With nothing selected the reveal is the whole composition's (K-203):
    // "show me what is animated" is a question about the comp as often as
    // about one layer, and refusing to answer it unless something was selected
    // made the commonest use of the key the one it did not serve.
    //
    // **The whole selection, not its primary** (K-523): reading
    // `selectedLayer` meant `Ctrl+A` then `U` revealed the top layer alone,
    // and looked like a dead key whenever that one layer carried no keys.
    final selected = ui.selectedLayers.value;
    final layers = selected.isNotEmpty
        ? selected
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
        _shutLayerDeep(id);
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
        _setOpen(id, true);
        // `U`: the keyed rows and the headings over them, and nothing else.
        // The group paths below are what `UU` opens — a heading opened by path
        // shows everything under it, which is exactly what the first tap is
        // not for.
        if (_revealTaps == 1) {
          _revealed[id] = RevealFilter.keyframed;
          continue;
        }
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

  /// **Animation ▸ Reveal properties with keyframes / with animation / all
  /// modified properties** (K-684): the same filtered opening a single `U`
  /// does, over the same layers, under a wider rule each.
  ///
  /// The menu is *not* the cycle. A menu row is chosen deliberately and names
  /// what it does, so it says it once: no tap counting, and no third press that
  /// shuts what the first two opened. Which layers is the `U` answer — the
  /// selection, or the whole comp when nothing is selected (K-203, K-523).
  ///
  /// **The rows decide whether a layer opens**, not a separate question to the
  /// engine: the reveal is built here, so asking the engine "does anything
  /// qualify" could disagree with what the filter then keeps, and a layer
  /// opened onto no rows is the empty fold-out K-622 set out to remove.
  void _revealFiltered(LumitUiState ui, RevealFilter filter) {
    final selected = ui.selectedLayers.value;
    final ids = {
      for (final layer in selected) layer.internallayerId.toString()
    };
    final entries = ids.isEmpty
        ? ui.model.layers
        : [
            for (final entry in ui.model.layers)
              if (ids.contains(entry.layer.internallayerId.toString())) entry
          ];
    if (entries.isEmpty) return;
    // Where an unmoved layer sits, for the one filter that asks. Read here
    // rather than in the build that follows, which may ask the engine nothing.
    if (filter == RevealFilter.modified) {
      final settings = ui.selectedComp?.getSettings();
      _revealCompWidth = settings?.width.toDouble() ?? 0;
      _revealCompHeight = settings?.height.toDouble() ?? 0;
    }
    setState(() {
      for (final entry in entries) {
        final id = entry.layer.internallayerId.toString();
        // Every reveal starts from the layers closed, so it shows exactly what
        // it says rather than adding to whatever was already open.
        _shutLayerDeep(id);
        _dropSelectionUnder(id);
      }
      // Built the way the panel is about to build them, so "does this layer
      // show anything" is answered by the rows themselves. Heights are not
      // read from this pass, only which rows survived the filter.
      final rows = layerRows(
        layers: entries,
        open: _open,
        rowHeight: 1,
        hasAudio: _hasAudio,
        hasPicture: _hasPicture,
        sequenceExtra: _sequenceExtra,
        flowParams: _flowParams,
        volumeDb: _volumeDb,
        driven: _driven,
        reveal: {
          for (final entry in entries)
            entry.layer.internallayerId.toString(): filter
        },
        compWidth: _revealCompWidth,
        compHeight: _revealCompHeight,
      );
      for (final row in rows) {
        // Nothing qualifies: leave the layer shut rather than opening it onto
        // a list of headings the reveal just said were empty.
        if (row.foldRows.isEmpty) continue;
        // In this order: `_setOpen` drops whatever reveal the layer was under,
        // which is exactly what a hand on the twirl should do and exactly what
        // this must not do to the reveal it is in the middle of setting.
        _setOpen(row.id, true);
        _revealed[row.id] = filter;
      }
    });
  }

  /// The menu asked for one of the three reveals (K-684).
  void _onRevealFilterRequested() {
    if (!mounted) return;
    _revealFiltered(_ui!, _ui!.revealFilter);
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
        _shutLayerDeep(id);
        _dropSelectionUnder(id);
        if (_audioRevealTaps >= 3) continue;
        if (!(_hasAudio[id] ?? false)) continue;
        _setOpen(id, true);
        _open.add(audioPath(id));
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
    laneModes.removeListener(_onLaneMode);
    HardwareKeyboard.instance.removeHandler(_onKey);
    _ui?.selectedLayer.removeListener(_onPrimaryChanged);
    _ui?.selectedLayers.removeListener(_onLayerSelectionChanged);
    _ui?.renderTimings.removeListener(_onTimingsChanged);
    _ui?.revealPropertyRequest.removeListener(_onRevealRequested);
    _ui?.revealFilterRequest.removeListener(_onRevealFilterRequested);
    _ui?.selectPropertyRequest.removeListener(_onSelectPropertyRequested);
    if (_ui?.deleteClaim == _deleteClaim) _ui!.deleteClaim = null;
    if (_ui?.copyClaim == _copySelectedKeys) _ui!.copyClaim = null;
    if (_ui?.pasteClaim == _pasteKeysIntoSelection) _ui!.pasteClaim = null;
    if (_ui?.easingApply.value == _applyEasing) _ui!.easingApply.value = null;
    _ui?.selectedEffects.removeListener(_onEffectSelectionChanged);
    _ui?.playheadFrame.removeListener(_edgeFollow);
    _boundTools?.removeListener(_onToolChanged);
    _zoomMotion.dispose();
    _barDrag.dispose();
    _layerDrag.dispose();
    _renameRequest.dispose();
    _liveResize.dispose();
    _rowSelection.dispose();
    _laneKeys.dispose();
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

  /// The navigator asked for a window: `start` frames from the left, `span`
  /// frames across (T5).
  ///
  /// The strip draws the view and names what it wants; turning that into a
  /// magnification and an offset is this panel's job, because this panel owns
  /// both. The window's **left edge is the anchor** and is held for the length
  /// of the gesture ([_zoomAnchorHeld]), which is what makes dragging the
  /// window's right-hand end zoom about its left-hand one: the frame the eye is
  /// on stays exactly where it is while the span changes around it. The strip
  /// swaps the two ends over for the other handle by naming a different start.
  ///
  /// A pan asks for the same span it already had, and a zoom that is not
  /// changing does not notify — so there is no tick to carry the anchor into
  /// layout, and the offset is jumped straight to the frame instead.
  void _navigateTo(double start, double span) {
    if (_laneFrames <= 0 || span <= 0) return;
    _zoomAnchorFrame = start;
    _zoomAnchorViewportX = 0;
    _zoomAnchorHeld = true;
    final want = (_laneFrames / span).clamp(1.0, _maxZoom);
    if ((want - _zoomMotion.target).abs() > 1e-9) {
      _setZoom(want, fly: false);
    } else {
      _scrollFrameToLeftEdge(start);
    }
  }

  /// Put [frame] at the lanes' left edge, at the width the lanes have now.
  void _scrollFrameToLeftEdge(double frame) {
    final position = positionOf(_hLane);
    if (position == null || _laneFrames <= 0) return;
    final span = position.viewportDimension +
        position.maxScrollExtent -
        TimelineAxis.pad * 2;
    if (span <= 0) return;
    _hLane.jumpTo((TimelineAxis.pad + frame * span / _laneFrames)
        .clamp(0.0, position.maxScrollExtent));
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

  /// The comp the zoom and the scroll offset currently belong to, so a change
  /// of front can be told from an ordinary rebuild.
  UuidValue? _shownComp;

  /// This panel's half of "a composition remembers where you were" (K-624):
  /// the magnification and how far the lanes are scrolled. The shell holds the
  /// other half, the playhead, because that is not the Timeline's alone.
  ///
  /// Called from `build`, which is the one moment both comps are known and
  /// nothing has moved yet: the controllers still hold the outgoing comp's
  /// view, so it is written down exactly as it was left. Putting the incoming
  /// one back has to wait for layout — twice. The zoom's ceiling and the
  /// lanes' scrollable range are both worked out from the new comp's length,
  /// which is not known until it has been laid out once; and the zoom itself
  /// changes that range again, so the offset can only be trusted after the
  /// layout the zoom causes.
  void _noticeFrontedComp(LumitUiState ui, CompositionReference? comp) {
    final now = comp?.internalid;
    if (now == _shownComp) return;
    final was = _shownComp;
    _shownComp = now;
    if (was != null) {
      final position = positionOf(_hLane);
      final extent = position?.maxScrollExtent ?? 0;
      ui.rememberCompView(
        was.toString(),
        zoom: _zoomMotion.target,
        // A fraction of the scrollable range rather than a pixel offset: the
        // panel may be a different width when the user comes back, and it is
        // the stretch of time they were looking at that they want back.
        scroll: extent > 0 ? (position!.pixels / extent).clamp(0.0, 1.0) : 0.0,
      );
    }
    if (now == null) return;
    final view = ui.compViews[now.toString()];
    if (view == null) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || _shownComp != now) return;
      // Not "the zoom before a fit": that belonged to the comp just left.
      _zoomBeforeFit = null;
      _setZoom(view.zoom.clamp(1.0, _maxZoom), fly: false);
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted || _shownComp != now) return;
        final position = positionOf(_hLane);
        if (position == null) return;
        // The zoom's own anchor has been spent by the layout just done; let
        // go of any that is somehow still pending rather than have it pull
        // the offset back off the restored one.
        _hLane.release();
        _hLane.jumpTo(view.scroll * position.maxScrollExtent);
      });
    });
  }

  /// Which layer index a Project-panel drop landed on. The stack starts below
  /// the pinned toolbar and column header and scrolls under them, so the drop
  /// is measured in stack space; the slot is then read back as an index into
  /// the whole comp, because the rows on screen may be a filtered subset.
  ///
  /// [density] is handed in rather than read from a context: this is geometry,
  /// called from a drop callback, and the panel already has the theme in hand
  /// where the gesture is wired up.
  int _dropIndex(LumitUiState ui, List<BridgeLayerEntry> layers,
      List<double> heights, Offset global, DensityTokens density) {
    final box = _dropArea.currentContext?.findRenderObject();
    if (box is! RenderBox) return 0;
    // Everything above the stack: the navigator strip and its hairline (T5),
    // then the two chrome rows — the same pair the lane side spends on its
    // ruler, which is exactly what `density.ruler` is. Both are taken off, or
    // a drop lands a row above where it was let go.
    final y = box.globalToLocal(global).dy -
        TimelineNavigator.band -
        density.ruler +
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
                  ? GutterScrollbar(controller: controller)
                  : const SizedBox.expand(),
            ),
          ],
        ),
      );

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context);
    _bindTools(ui);
    // Read once, into a field, so the toggles that consume it do not each
    // look it up as the pointer moves over the bar.
    _chromeLabels = ui.workspace.interface.chromeLabels;
    final comp = ui.selectedComp;
    _noticeFrontedComp(ui, comp);
    if (comp == null) {
      // Footage dropped with nothing open offers to make the composition it
      // would go in — the same gesture the Project panel's New composition
      // button takes, so "drag a clip in and start" works from either side
      // rather than dead-ending on a placeholder.
      return EmptyTimelineDrop(state: Provider.of<LumitState>(context));
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
    final frames = ui.model.durationFrames;
    final (fpsNum, fpsDen) = ui.model.fpsExact;
    final needle = _search.trim().toLowerCase();
    // Where the comp's groups land on its rows (K-702): the header each
    // carrier layer draws, and the members a shut fold takes off the list —
    // both from one walk, so the two halves cannot disagree about how many
    // rows there are.
    final folds =
        groupFolds(groups: ui.model.groups, folded: _foldedGroups);
    final layers = [
      for (final e in ui.model.layers)
        if ((needle.isEmpty || e.info.name.toLowerCase().contains(needle)) &&
            !(_hideShy && e.info.switches.shy) &&
            !folds.hidden.contains(e.layer.internallayerId.toString()))
          e,
    ];
    // Whether the matte column is carrying its two mode toggles' room: it does
    // while some visible row has a matte set, and not otherwise (K-463) — a
    // comp with no mattes would else read as a 28px hole between every matte
    // face and the blend column beside it. Read off the list already in hand,
    // once for the panel, and handed to the header and the rows alike so the
    // two cannot disagree.
    final anyMatte = layers.any((e) => e.info.matte != null);
    final groupWidths = {
      for (final entry in _groupWidths.entries)
        if (drawn(entry.key))
          entry.key: entry.value +
              (entry.key == TimelineGroup.compose && anyMatte
                  ? matteToggleWidth
                  : 0),
    };
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
    //
    // **One answer for both views** since K-529: the graph's own filtered
    // outline is gone and its outline is the Layers outline, identical — so
    // there is no second row model to keep in step with this one.
    final rows = layerRows(
        layers: layers,
        open: _open,
        rowHeight: t.density.laneRow,
        hasAudio: _hasAudio,
        hasPicture: _hasPicture,
        sequenceExtra: _sequenceExtra,
        flowParams: _flowParams,
        volumeDb: _volumeDb,
        driven: _driven,
        // The strip filters the whole comp; a reveal filters only the layers
        // it opened, by the rule it opened them with (K-622, K-684).
        reveal: _animatedOnly ? everyLayerKeyframed : _revealed,
        groupHeaders: folds.headers,
        compWidth: _revealCompWidth,
        compHeight: _revealCompHeight);

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
    // How many diamonds each of those rows draws, so a click on a property's
    // *name* can take its keys with it (K-500 §2.1) without walking the model
    // again from a callback that has no rows to hand. Read off the same rows
    // the lanes draw from, so the two cannot disagree about how many keys a
    // row has.
    _visibleKeyCounts = {
      for (final layer in rows)
        for (final row in layer.drawnRows)
          if (row is! FoldWaveformRow)
            foldRowPath(layer.id, row): laneKeysOf(row).length,
    };
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
    final work = _workPreview.value ?? _workArea!;
    // The block heights, as a plain list. Still needed even though the rows
    // now carry their own height: a drag measures its travel against the
    // *stack* ([layerDragTarget]), a drop reads a slot out of it
    // ([layerDropSlot]) and [LayerDragSlide] slides one block by the ones it
    // passes — all three want every height, not this row's.
    final blockHeights = [for (final row in rows) row.height];
    final graphColours = _colourOfChannels(channels);
    // The rows read the selection off the notifier, so it has to be current
    // before they build. Marking a descendant dirty from here is legal — it is
    // inside this build's own scope — and harmless, since the subtree is about
    // to be built anyway.
    _publishRowSelection(graphColours);
    // And the same for the lanes, so every rule that still edits the key set
    // inside a `setState` reaches them without saying so itself.
    _publishLaneKeys();

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        CompTabsFrb(
          state: Provider.of<LumitState>(context, listen: false),
          uiState: ui,
          // The File menu's own command, not a second route to the dialog:
          // one function decides what "export" means (K-303's reason applies
          // to commands as much as to strings).
          onExport: () => exportFrb(context),
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
              final at = _dropIndex(
                  ui, layers, blockHeights, details.offset, t.density);
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
                  // ponytail: every call here is its own committed op, so the
                  // ceiling is two undo steps per file — the add, then the
                  // walk down to the drop. A ten-file drop takes twenty
                  // presses of Ctrl-Z to put back, and the middle of that
                  // sequence shows the layers at the top of the stack, which
                  // is a position the user never asked for. The trigger is any
                  // multi-select drop of more than two or three files followed
                  // by an undo — reachable the first time somebody drags a
                  // folder's worth of footage in. The fix is engine-side: an
                  // add-at-index op, so one drop is one step.
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
                  final columnsWidth = outlineWidthOf(groupWidths);
                  final outlineViewport = (constraints.maxWidth - 120)
                      .clamp(120.0, columnsWidth + scrollGutterWidth);
                  final outlineWidth = columnsWidth;
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
                        // **The outline redraws under a seam drag** (T4): while
                        // a seam is in hand this half is built at the live
                        // width, so the Layers column's names widen with the
                        // gesture and a switch column's cells go away as the
                        // seam passes them. Only this half listens — nothing
                        // right of the seam depends on a column width, and
                        // rebuilding the lanes and bars per pointer move is
                        // what made this drag lag when it was tried before.
                        ValueListenableBuilder<
                            MapEntry<TimelineGroup, double>?>(
                          valueListenable: _liveResize,
                          builder: (context, live, _) {
                            final widths =
                                _liveWidths(groupWidths, live, anyMatte);
                            return _outlineHalf(context, ui, comp,
                                rows: rows,
                                layers: layers,
                                blockHeights: blockHeights,
                                groupOrder: groupOrder,
                                groupWidths: widths,
                                matteToggles: anyMatte,
                                graphColours: graphColours,
                                outlineViewport: outlineViewport,
                                channels: channels,
                                outlineWidth: live == null
                                    ? outlineWidth
                                    : outlineWidthOf(widths));
                          },
                        ),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              // The time navigator (T5, K-648): the
                              // whole comp as a strip, with the slice
                              // the lanes are showing drawn on it.
                              //
                              // **Over the lane area alone** (K-682, the
                              // owner's ruling): it spanned the whole
                              // panel and stood blank over the outline,
                              // which read as a sliver of dead ground
                              // above the timecode row. That row is now
                              // taller by exactly this band, so the two
                              // halves still spend the same height above
                              // their first layer row. Outside the
                              // zoom's ListenableBuilder below — the
                              // strip listens to the zoom, the scroll
                              // and the playhead for itself.
                              TimelineNavigator(
                                trailing: scrollGutterWidth,
                                frames: frames,
                                zoom: _zoomMotion,
                                hScroll: _hLane,
                                playhead: ui.playheadFrame,
                                onWindow: _navigateTo,
                                onWindowEnd: _zoomDragEnd,
                              ),
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
                                        frames: frames,
                                        width: laneViewport * _zoom);
                                    return _graph
                                        ? _graphHalf(context, ui, comp,
                                            axis: axis,
                                            channels: channels,
                                            rows: rows,
                                            work: work,
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

    /// Whether the compose group's width is carrying the matte mode toggles'
    /// room (K-463) — the header and every row split the column by the same
    /// answer, so they line up either way.
    required bool matteToggles,
    required Map<String, List<Color>> graphColours,

    /// The curves on the pane, so the Key readout row can name the one key in
    /// hand (§3.3).
    required List<GraphChannel> channels,
    required double outlineViewport,
    required double outlineWidth,
  }) {
    final t = ThemeScope.of(context).theme;
    return SizedBox(
      width: outlineViewport,
      // A column, to match the lane side's: rows, then a
      // block the height of the lane bottom bar, so both
      // halves give their rows the same viewport and scroll
      // the same distance.
      //
      // **That reservation is not decoration**: the two
      // halves are one table, and a viewport shorter on one
      // side can be scrolled further than the other. The
      // lanes could run past the outline's last row by
      // exactly the bottom bar's height, and the halves came
      // apart at the bottom of a long stack — reported as
      // "the lane area can scroll up more than the layer
      // area". Reserving it keeps both viewports the same
      // height, which is what keeps `maxScrollExtent` the
      // same on both.
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
                            Toolbar(
                              model: ui.model,
                              playhead: ui.playheadFrame,
                              onSeek: ui.scrubTo,
                              mode: _mode,
                              onMode: _setMode,
                              onSearch: (v) => setState(() => _search = v),
                            ),
                            // The second row of the outline: the column
                            // headers, in both views (K-529 — the graph's
                            // own filter row went with its own outline).
                            ColumnHeader(
                              order: groupOrder,
                              widths: groupWidths,
                              matteToggles: matteToggles,
                              onResize: _resizeGroup,
                              onResizeLive: _liveResizeGroup,
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
                                // The viewport's own height, which is what
                                // the row list windows itself against.
                                child: LayoutBuilder(
                                    builder: (context, box) =>
                                        SingleChildScrollView(
                                          controller: _vOutline,
                                          // **One outline, in both views** (K-529):
                                          // Graph mode's colour-ticked filtered list
                                          // is gone and the graph shows exactly this,
                                          // so a property is picked the same way
                                          // wherever the panel is looked at.
                                          child: Outline(
                                            comp: comp,
                                            rows: rows,
                                            vScroll: _vOutline,
                                            viewport: box.maxHeight,
                                            onOpenSequence: _toggleSequenceView,
                                            layerDrag: _layerDrag,
                                            renameRequest: _renameRequest,
                                            blockHeights: blockHeights,
                                            groupOrder: groupOrder,
                                            widths: groupWidths,
                                            matteToggles: matteToggles,
                                            groupActions:
                                                _groupActions(ui, comp),
                                            selection: _rowSelection,
                                            onSelectProperty: _selectProperty,
                                            onEditProperty: _selectOnEdit,
                                            onToggle: _toggle,
                                            playheadFrame:
                                                ui.playheadFrame.value,
                                            onSeek: ui.scrubTo,
                                            onSelect: (l) => _selectLayer(ui, l,
                                                among: layers),
                                            // The dimmer mark that follows the fold-out
                                            // last touched: one row's shade, so one
                                            // row's repaint.
                                            onHighlight: (id) {
                                              _highlighted = id;
                                              _publishRowSelection();
                                            },
                                            onChanged: ui.model.refresh,
                                          ),
                                        )),
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
                      // The toolbar's block carries the navigator band the
                      // toolbar itself does (K-682), or the gutter's thumb
                      // would start a band above the rows it scrolls.
                      Container(
                          height: t.density.timelineChromeRow +
                              TimelineNavigator.band,
                          color: t.surface1),
                      Container(
                          height: t.density.timelineHeaderRow,
                          color: t.surface1),
                    ],
                  ),
                ],
              ),
              // The row seams, over the columns *and* the
              // gutter so they meet the lane area's (K-192);
              // phased by the scroll so they travel with the
              // rows they separate.
              Positioned(
                // Below the outline's own two chrome rows — the first of
                // which carries the navigator's band (K-682) — level with
                // the foot of the lane side's ruler.
                top: TimelineNavigator.band + t.density.ruler,
                left: 0,
                right: 0,
                bottom: 0,
                child: IgnorePointer(
                  child: AnimatedBuilder(
                    animation: _vOutline,
                    builder: (context, _) => CustomPaint(
                      painter: RowDividerPainter(
                        step: t.density.laneRow,
                        colour: t.hairline,
                        phase: -((positionOf(_vOutline)?.pixels ?? 0) %
                            t.density.laneRow),
                        // The grid here repeats from the
                        // panel's edge, so the blanks are
                        // carried up by however far the rows
                        // have scrolled.
                        blanks: [
                          for (final b in _sequenceBlanks(rows))
                            (
                              b.$1 - (positionOf(_vOutline)?.pixels ?? 0),
                              b.$2 - (positionOf(_vOutline)?.pixels ?? 0),
                            ),
                        ],
                      ),
                    ),
                  ),
                ),
              ),
            ],
          )),
          // The Key readout row, while exactly one key is in hand (§3.3).
          if (_graph)
            KeyReadoutRow(
              channels: channels,
              selectedKeys: _graphKeySelection,
              fps: ui.model.fps,
              onChanged: ui.model.refresh,
            ),
          // The outline's own end of the bottom bar: the key commands and the
          // column-group toggles, where the lane side carries the zoom and the
          // scrollbar (K-448). The block was already reserved to keep the two
          // halves the same height — it now has something in it.
          //
          // One row, two runs. The strip is loose, so at any ordinary outline
          // width it takes exactly the room its buttons need and the toggles
          // keep the rest; squeezed, neither run overflows — each scrolls
          // inside its own share.
          Row(children: [
            Flexible(
              child: KeyCommandStrip(
                // The keyframe strip (K-458) in Layers, the graph's own
                // commands in graph view — the same seven or ten buttons that
                // stood on the lane bar, in the same order.
                strip: !_graph,
                lens: _graph ? _graphLens : null,
                onLens: (lens) => setState(() {
                  _graphLens = lens;
                  _publishEasingClaim();
                }),
                autoFit: _graphAutoFit,
                onToggleAutoFit: () =>
                    setState(() => _graphAutoFit = !_graphAutoFit),
                onInterp: (side) => _applyInterp(side),
                onTangentMode: _applyTangentMode,
                onOpenEasing: _openEasing,
                onEaseBlock: (buttonContext) {
                  final box = buttonContext.findRenderObject();
                  if (box is! RenderBox) return;
                  _openEasePopover(box.localToGlobal(Offset.zero));
                },
              ),
            ),
            Expanded(
                child: ColumnToggles(
              groups: _toggleableGroups,
              labels: _chromeLabels,
              hidden: _hiddenGroups,
              onToggle: (group) => setState(() {
                if (!_hiddenGroups.remove(group)) _hiddenGroups.add(group);
              }),
              animatedOnly: _animatedOnly,
              onToggleAnimated: () =>
                  setState(() => _animatedOnly = !_animatedOnly),
              comp: comp,
              model: ui.model,
              playhead: ui.playheadFrame,
              razor: _razorArmed(ui),
              onToggleRazor: () => _toggleRazor(ui),
              hideShy: _hideShy,
              onToggleHideShy: () => setState(() => _hideShy = !_hideShy),
              onChanged: ui.model.refresh,
            )),
          ]),
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
    required List<LayerRow> rows,
    required ({int start, int end, bool whole}) work,
    required int frames,
    required int fpsNum,
    required int fpsDen,
  }) {
    final t = ThemeScope.of(context).theme;
    // The one shared target list (docs/07 §4.5), for the ruler's edges and
    // markers and for the pane's own key drags. Built from the read model, so
    // it costs no bridge calls (K-184).
    final snap = timelineSnapTargets(
      rows: rows,
      comp: comp,
      playheadFrame: ui.playheadFrame.value,
      work: work,
      fps: ui.model.fps,
    );
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
                              height: t.density.ruler,
                              work: work,
                              onWorkArea: (span) {
                                comp.setWorkArea(span: span);
                                setState(() {});
                              },
                              // No `setState`: the grounds listen for
                              // themselves, so an edge drag repaints the band
                              // and rebuilds nothing (K-626's pattern).
                              onWorkPreview: (span) =>
                                  _workPreview.value = span,
                              onMarkersChanged: () => setState(() {}),
                              // The graph shares the ruler, so it shares the
                              // ruler's snapping (docs/07 §4.5).
                              snapTargets: snap,
                              magnet: _magnet,
                              onSeek: (f) => ui.scrubTo(
                                  f.clamp(0, frames == 0 ? 0 : frames - 1)),
                              cache: TimelineCacheBar(
                                comp: comp,
                                axis: axis,
                                revision: _cacheRevision!,
                              ),
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
                                  WorkAreaGround(
                                    key: const ValueKey<String>(
                                        'tl-graph-ground'),
                                    preview: _workPreview,
                                    committed: work,
                                    axis: axis,
                                    // The same band the ruler hangs and the
                                    // lanes carry (§12A.2: nothing about the
                                    // work area changes on a mode switch).
                                    inside: Color.alphaBlend(
                                        t.animated.withValues(
                                            alpha: workAreaLaneFillAlpha),
                                        t.surface1),
                                    outside: t.timelineOutOfRange,
                                    edge: workAreaEdgeColour(t),
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
                                    snapTargets: snap,
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
                        // ruler and curves alike —
                        // on its own layer, so the
                        // curves are not redrawn
                        // for it.
                        PlayheadOverlay(
                          playhead: ui.playheadFrame,
                          xOf: axis.xOf,
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
                    height: t.density.ruler,
                    color: t.timelineOutOfRange,
                  ),
                ],
              ),
            ],
          ),
        ),
        LaneBottomBar(
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

  /// The lane half: the ruler, the cache bar, one bar per layer and the
  /// bottom bar — and, in Keys mode, the same everything with the dope
  /// sheet's rows where the bars were (K-455).
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
                    child: LayerArea(
                      comp: comp,
                      rows: rows,
                      barNames: _barNames,
                      selection: _rowSelection,
                      layerDrag: _layerDrag,
                      blockHeights: blockHeights,
                      groupActions: _groupActions(ui, comp),
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
                      spectra: _spectra,
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
                      selectedKeys: _laneKeys,
                      stretch: _keyStretch,
                      project: Provider.of<LumitState>(context, listen: false)
                          .project,
                      onEase: _openEasePopover,
                      onDeselectAll: () => _deselectAll(ui),
                      work: work,
                      onWorkPreview: (span) => _workPreview.value = span,
                      workPreview: _workPreview,
                      onKeysSelected: _onLaneKeysSelected,
                      onKeyMenu: _laneKeyMenu,
                      onWheel: (e, x) => _wheel(e, x, axis),
                      onSeek: (f) =>
                          ui.scrubTo(f.clamp(0, frames == 0 ? 0 : frames - 1)),
                      onSelect: (l) => _selectLayer(ui, l, among: layers),
                      onChanged: ui.model.refresh,
                      cacheRevision: _cacheRevision!,
                      dragPreview: _barDrag,
                      bounds: _barBounds,
                      selectionMove: () => _selectionMove(ui),
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
                    height: t.density.ruler,
                    color: t.timelineOutOfRange,
                  ),
                ],
              ),
            ],
          ),
        ),
        LaneBottomBar(
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
