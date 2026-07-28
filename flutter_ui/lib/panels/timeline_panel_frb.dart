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

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../state/comp_model.dart';
import '../state/comp_time.dart';
import '../state/drag_payloads.dart';
import '../state/timecode.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/marquee.dart';
// The ruler helpers moved with the ruler (shared with the graph editor); the
// re-export keeps their long-standing import path alive for their tests.
export 'timeline_extras_frb.dart' show rulerLabelStepSeconds, rulerLabelOf;

import 'placeholder.dart';
import 'graph_editor_frb.dart';
import 'graph_maths.dart';
import 'timeline_extras_frb.dart';
import 'effect_param_row_frb.dart';
import 'keyframe_controls_frb.dart';
import 'layer_fold_frb.dart';
import 'transform_rows_frb.dart';

/// The blend-mode names, fetched once per session: the list is static for the
/// life of the process, and every outline row was re-fetching it per rebuild.
List<String>? _blendModes;

/// One layer row's height.
const double _rowHeight = 22;

/// The outline's two header rows: the toolbar (timecode, search, the view
/// buttons) and the column-group header under it.
const double _toolbarHeight = 26;
const double _headerHeight = 20;

/// The time ruler's height: the toolbar and column header stay inside the
/// outline (docs/07 §4.1), so the lane side gives their whole height to the
/// ruler — a taller bar is an easier playhead grab — minus the cache bar
/// tucked under it.
const double _rulerHeight =
    _toolbarHeight + _headerHeight - TimelineCacheBar.height;

/// How near the end of a bar counts as grabbing its edge to trim rather than its
/// middle to move.
const double _trimGrab = 6;

class TimelinePanelFrb extends StatefulWidget {
  const TimelinePanelFrb({super.key});

  @override
  State<TimelinePanelFrb> createState() => _TimelinePanelFrbState();
}

class _TimelinePanelFrbState extends State<TimelinePanelFrb> {
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

  /// Each layer's source waveform peaks, by id — fetched once when its
  /// Waveform twirl first opens (decoding a whole track is not work for a
  /// build), then good for the session: peaks belong to the file, so trims
  /// and drags never invalidate them (K-172).
  final Map<String, BridgeAudioPeaks> _peaks = {};

  /// One lane's worth of buckets: plenty for any panel width.
  static const int _peakBuckets = 2048;

  /// Fetch peaks for any layer whose Waveform twirl is open and unanswered.
  void _refreshPeaks(List<BridgeLayerEntry> layers) {
    for (final entry in layers) {
      final id = entry.layer.internallayerId.toString();
      if (!_open.contains(waveformPath(id)) || _peaks.containsKey(id)) {
        continue;
      }
      // Claim the slot first, so a rebuild mid-decode does not decode twice.
      _peaks[id] = BridgeAudioPeaks(durationSeconds: 0, pairs: Float32List(0));
      entry.layer.audioPeaks(buckets: _peakBuckets).then((peaks) {
        if (!mounted) return;
        setState(() => _peaks[id] = peaks);
      });
    }
  }

  void _toggle(String path) => setState(() {
        if (!_open.remove(path)) _open.add(path);
      });

  /// Fill in any layer's has-audio answer we do not have, off the build.
  void _refreshAudio(List<BridgeLayerEntry> layers) {
    for (final entry in layers) {
      final id = entry.layer.internallayerId.toString();
      if (_hasAudio.containsKey(id)) continue;
      // Claim the slot first, so a rebuild mid-probe does not probe twice.
      _hasAudio[id] = false;
      entry.layer.hasAudio().then((has) {
        if (!mounted || _hasAudio[id] == has) return;
        setState(() => _hasAudio[id] = has);
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
        final cut = path.indexOf('/');
        if (cut > 0) _highlighted = path.substring(0, cut);
      });

  /// Editing a value or keying a property selects it too (docs/07 §4.3) —
  /// quietly: an already-selected property stays where it is in the order.
  void _selectOnEdit(String path) {
    if (_selectedProperties.contains(path)) return;
    setState(() {
      _selectedProperties
        ..clear()
        ..add(path);
      _graphKeySelection.clear();
      final cut = path.indexOf('/');
      if (cut > 0) _highlighted = path.substring(0, cut);
    });
  }

  /// The graph editor replaces the layer area rather than sitting beside it:
  /// the two want the same width, and a curve squeezed into half a panel is not
  /// a curve you can shape.
  bool _graph = false;

  /// With the razor armed, a click on a bar cuts it rather than selecting it.
  /// Modal on purpose — it is how every editor does the tool, and it is the one
  /// gesture where "what does a click do here" has two answers.
  bool _razor = false;

  /// The bar drag in flight, if any — a notifier rather than panel state so
  /// only the waveform lanes redraw as the pointer moves, not the whole table.
  final ValueNotifier<BarDragPreview?> _barDrag = ValueNotifier(null);

  /// The lane view's selected keyframes, as `rowId#index` (docs/07 §4.3) —
  /// what the marquee gathered. Session state, like the twirl set.
  final Set<String> _laneKeySelection = {};

  /// The outline's and the lanes' vertical scrolls, linked both ways so the
  /// two halves of the table stay one table; the lanes' side owns the visible
  /// scrollbar. In graph view the outline scrolls alone.
  final ScrollController _vOutline = ScrollController();
  final ScrollController _vLane = ScrollController();

  /// The lanes' horizontal scroll, once zoomed past fit.
  final ScrollController _hLane = ScrollController();

  /// Time zoom: 1 is fit-to-panel; the bottom bar's − / + / Fit set it, and
  /// Ctrl+wheel zooms about the pointer.
  double _zoom = 1;

  /// Whether a dragged keyframe sticks to whole frames (docs/07 §4.5). On by
  /// default: landing between frames is the deliberate exception.
  bool _magnet = true;

  bool _syncingScroll = false;

  @override
  void initState() {
    super.initState();
    _vOutline.addListener(() => _followScroll(_vOutline, _vLane));
    _vLane.addListener(() => _followScroll(_vLane, _vOutline));
    HardwareKeyboard.instance.addHandler(_onKey);
  }

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

  /// The Timeline's keyboard commands: `Shift+F3` toggles the graph, the F9
  /// family sets easing, `F` re-frames the graph, `Ctrl+C`/`Ctrl+V` copy and
  /// paste keyframes, Delete removes the graph's selected keys. Registered on
  /// the hardware keyboard (panels do not hold focus); a focused text field
  /// keeps its keys.
  bool _onKey(KeyEvent event) {
    if (event is! KeyDownEvent || !mounted) return false;
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return false;
    }
    final keyboard = HardwareKeyboard.instance;
    final ctrl = keyboard.isControlPressed || keyboard.isMetaPressed;
    final shift = keyboard.isShiftPressed;
    final key = event.logicalKey;

    if (key == LogicalKeyboardKey.f3 && shift) {
      setState(() => _graph = !_graph);
      return true;
    }
    if (key == LogicalKeyboardKey.f9) {
      // F9 easy-eases both sides; Shift+F9 the way in; Ctrl+Shift+F9 the way
      // out (docs/07 §5.3).
      _applyInterp(easyEase, inSide: !(ctrl && shift), outSide: !shift || ctrl);
      return true;
    }
    // Copy and paste work wherever keyframes are selected — the lane view's
    // marquee catch as much as the graph's (K-196).
    if (ctrl && key == LogicalKeyboardKey.keyC) {
      final ui = Provider.of<LumitUiState>(context, listen: false);
      final comp = ui.selectedComp;
      if (comp == null) return false;
      final channels = _channelsNow();
      final selection = _actionKeySelection(channels);
      if (selection.isEmpty) return false;
      copySelectedKeys(
        comp: comp,
        channels: channels,
        selectedKeys: selection,
        fps: ui.model.fps,
      );
      return true;
    }
    if (ctrl && key == LogicalKeyboardKey.keyV) {
      final ui = Provider.of<LumitUiState>(context, listen: false);
      final channels = _channelsNow();
      if (channels.isEmpty) return false;
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

    if (!_graph) return false;

    if (key == LogicalKeyboardKey.keyF && !ctrl && !shift) {
      _graphPane.currentState?.fitNow();
      return true;
    }
    if ((key == LogicalKeyboardKey.delete ||
            key == LogicalKeyboardKey.backspace) &&
        _graphKeySelection.isNotEmpty) {
      _graphPane.currentState?.deleteSelectedKeys();
      return true;
    }
    return false;
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
    _barDrag.dispose();
    _vOutline.dispose();
    _vLane.dispose();
    _hLane.dispose();
    super.dispose();
  }

  /// A modified wheel over the lanes (docs/07 §4.6). Ctrl zooms time about the
  /// pointer — the frame under the cursor stays under it — and Shift scrolls
  /// sideways. A plain wheel is not touched here, so it still reaches the
  /// scrollable and moves the rows.
  void _wheel(PointerScrollEvent event, double contentX, double perFrame) {
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed) {
      final next = (event.scrollDelta.dy < 0 ? _zoom * 1.2 : _zoom / 1.2)
          .clamp(1.0, 64.0);
      if (next == _zoom) return;
      // Where the pointer sits in the viewport, and which frame is under it.
      final viewportX = contentX - (_hLane.hasClients ? _hLane.offset : 0);
      final frame = perFrame <= 0 ? 0.0 : contentX / perFrame;
      final grew = next / _zoom;
      setState(() => _zoom = next);
      // The axis is only that wide after this frame lays out, so the offset
      // that holds the frame still is restored once it has.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted || !_hLane.hasClients) return;
        _hLane.jumpTo((frame * perFrame * grew - viewportX)
            .clamp(0.0, _hLane.position.maxScrollExtent));
      });
      return;
    }
    if (keys.isShiftPressed && _hLane.hasClients) {
      _hLane.jumpTo((_hLane.offset + event.scrollDelta.dy)
          .clamp(0.0, _hLane.position.maxScrollExtent));
    }
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
    final t = ThemeScope.of(context).theme;
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
    _refreshPeaks(layers);

    // The property rows on screen, in display order — what a Shift+click
    // range runs along — and the graph channels the selection resolves to,
    // each with its stroke colour for the outline's labels to match.
    _visiblePropertyPaths = [
      for (final e in layers)
        if (_open.contains(e.layer.internallayerId.toString()))
          for (final row in layerFoldRows(
            entry: e,
            open: _open,
            hasAudio: _hasAudio[e.layer.internallayerId.toString()] ?? false,
          ))
            if (row is! FoldGroupRow && row is! FoldWaveformRow)
              foldRowPath(e.layer.internallayerId.toString(), row),
    ];
    final channels =
        graphChannels(layers: ui.model.layers, selected: _selectedProperties);
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
              switch (details.data) {
                case FootageDragData(:final footage):
                  // Bottom-up, so a multi-item drop stacks in the order the
                  // panel listed them: each lands at the top of the stack.
                  for (final f in footage.reversed) {
                    comp.addFootageLayer(footage: f);
                  }
                case CompDragData(comp: final dropped):
                  // A comp cannot nest into itself; the engine refuses and
                  // the drop simply does nothing.
                  try {
                    comp.addPrecompLayer(comp: dropped);
                  } catch (_) {}
              }
              ui.model.refresh();
            },
            builder: (context, candidate, _) => Container(
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
                  final outlineWidth = outlineWidthOf(_groupWidths);
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
                  final axis =
                      TimelineAxis(frames: frames, width: laneViewport * _zoom);

                  // **Not** wrapped in a playhead listener. Every layer row and
                  // every bar used to rebuild each time the playhead moved —
                  // sixty times a second during playback, growing with the layer
                  // count, and asking the engine for each layer's name and span
                  // again every time. Only two things actually care where the
                  // playhead is: the line itself, and the razor (which reads it
                  // when clicked). Both listen for themselves now.
                  //
                  // Dragging never scrolls the timeline — the wheel and the
                  // scrollbars do (docs/07 §4.6). A drag on empty lane space
                  // is the keyframe marquee, and a scrollable competing for
                  // it in the gesture arena would win and eat the box.
                  return ScrollConfiguration(
                    behavior: ScrollConfiguration.of(context)
                        .copyWith(dragDevices: const {}, scrollbars: false),
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        SizedBox(
                          width: outlineViewport,
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
                                          crossAxisAlignment:
                                              CrossAxisAlignment.stretch,
                                          children: [
                                            // The toolbar and the column header live in
                                            // the outline, not across the panel: the lane
                                            // side gives their height to a taller, easier
                                            // to grab time ruler (docs/07 §4.1).
                                            _Toolbar(
                                              comp: comp,
                                              model: ui.model,
                                              playhead: ui.playheadFrame,
                                              graph: _graph,
                                              onToggleGraph: () => setState(
                                                  () => _graph = !_graph),
                                              razor: _razor,
                                              onToggleRazor: () => setState(
                                                  () => _razor = !_razor),
                                              hideShy: _hideShy,
                                              onToggleHideShy: () => setState(
                                                  () => _hideShy = !_hideShy),
                                              onSearch: (v) =>
                                                  setState(() => _search = v),
                                              onChanged: ui.model.refresh,
                                            ),
                                            _ColumnHeader(
                                              order: _groupOrder,
                                              widths: _groupWidths,
                                              onResize: _resizeGroup,
                                              onReorder: (dragged, target) =>
                                                  setState(
                                                () => _groupOrder =
                                                    reorderedGroups(_groupOrder,
                                                        dragged, target),
                                              ),
                                            ),
                                            // The rows scroll under the pinned toolbar
                                            // and header, in step with the lanes.
                                            Expanded(
                                              child: SingleChildScrollView(
                                                controller: _vOutline,
                                                child: _Outline(
                                                  comp: comp,
                                                  layers: layers,
                                                  groupOrder: _groupOrder,
                                                  widths: _groupWidths,
                                                  selected:
                                                      ui.selectedLayer.value,
                                                  highlighted: _highlighted,
                                                  selectedProperties:
                                                      _selectedProperties,
                                                  graphColours: graphColours,
                                                  onSelectProperty:
                                                      _selectProperty,
                                                  onEditProperty: _selectOnEdit,
                                                  open: _open,
                                                  hasAudio: _hasAudio,
                                                  onToggle: _toggle,
                                                  playheadFrame:
                                                      ui.playheadFrame.value,
                                                  onSeek: (f) => ui
                                                      .playheadFrame.value = f,
                                                  onSelect: (l) => setState(() {
                                                    ui.selectedLayer.value = l;
                                                  }),
                                                  onHighlight: (id) => setState(
                                                      () => _highlighted = id),
                                                  onChanged: ui.model.refresh,
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
                                      Container(
                                          height: _toolbarHeight,
                                          color: t.surface1),
                                      Container(
                                          height: _headerHeight,
                                          color: t.surface2),
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
                                        phase:
                                            -((_positionOf(_vOutline)?.pixels ??
                                                    0) %
                                                _rowHeight),
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                            ],
                          ),
                        ),
                        Expanded(
                          child: _graph
                              // The graph editor: the same ruler, zoom and
                              // horizontal scroll as the lane view, over one
                              // full-height pane of curves (docs/07 §5).
                              ? Column(
                                  crossAxisAlignment:
                                      CrossAxisAlignment.stretch,
                                  children: [
                                    Expanded(
                                      child: Row(
                                        crossAxisAlignment:
                                            CrossAxisAlignment.stretch,
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
                                                      crossAxisAlignment:
                                                          CrossAxisAlignment
                                                              .stretch,
                                                      children: [
                                                        TimelineRuler(
                                                          comp: comp,
                                                          axis: axis,
                                                          fps: ui.model.fps,
                                                          height: _rulerHeight,
                                                          onSeek: (f) => ui
                                                                  .playheadFrame
                                                                  .value =
                                                              f.clamp(
                                                                  0,
                                                                  frames == 0
                                                                      ? 0
                                                                      : frames -
                                                                          1),
                                                        ),
                                                        TimelineCacheBar(
                                                          comp: comp,
                                                          axis: axis,
                                                          revision:
                                                              Listenable.merge([
                                                            ui.frameArrived,
                                                            ui.cacheChanged
                                                          ]),
                                                        ),
                                                        Expanded(
                                                          child: GraphEditorFrb(
                                                            key: _graphPane,
                                                            comp: comp,
                                                            channels: channels,
                                                            axis: axis,
                                                            frames: frames,
                                                            fps: ui.model.fps,
                                                            fpsNum: fpsNum,
                                                            fpsDen: fpsDen,
                                                            magnet: _magnet,
                                                            lens: _graphLens,
                                                            autoFit:
                                                                _graphAutoFit,
                                                            selectedKeys:
                                                                _graphKeySelection,
                                                            onSelectionChanged:
                                                                () => setState(
                                                                    () {}),
                                                            onChanged: ui
                                                                .model.refresh,
                                                            onWheelTime: (e,
                                                                    x) =>
                                                                _wheel(e, x,
                                                                    axis.perFrame),
                                                          ),
                                                        ),
                                                      ],
                                                    ),
                                                    // The playhead, over the
                                                    // ruler and curves alike.
                                                    ValueListenableBuilder<int>(
                                                      valueListenable:
                                                          ui.playheadFrame,
                                                      builder: (context, frame,
                                                              child) =>
                                                          Positioned(
                                                        left: axis.xOf(frame),
                                                        top: 0,
                                                        bottom: 0,
                                                        child: child!,
                                                      ),
                                                      child: IgnorePointer(
                                                        child: Container(
                                                            width: 1,
                                                            color: t.accent),
                                                      ),
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
                                                height: _rulerHeight +
                                                    TimelineCacheBar.height,
                                                color: t.surface2,
                                              ),
                                            ],
                                          ),
                                        ],
                                      ),
                                    ),
                                    _LaneBottomBar(
                                      zoom: _zoom,
                                      hScroll: _hLane,
                                      magnet: _magnet,
                                      onToggleMagnet: () =>
                                          setState(() => _magnet = !_magnet),
                                      onZoom: (z) => setState(
                                          () => _zoom = z.clamp(1.0, 64.0)),
                                      lens: _graphLens,
                                      onLens: (lens) =>
                                          setState(() => _graphLens = lens),
                                      autoFit: _graphAutoFit,
                                      onToggleAutoFit: () => setState(
                                          () => _graphAutoFit = !_graphAutoFit),
                                      onInterp: (side) => _applyInterp(side),
                                    ),
                                  ],
                                )
                              : Column(
                                  crossAxisAlignment:
                                      CrossAxisAlignment.stretch,
                                  children: [
                                    Expanded(
                                      child: Row(
                                        crossAxisAlignment:
                                            CrossAxisAlignment.stretch,
                                        children: [
                                          Expanded(
                                            child: SingleChildScrollView(
                                              scrollDirection: Axis.horizontal,
                                              controller: _hLane,
                                              child: SizedBox(
                                                width: axis.width,
                                                child: _LayerArea(
                                                  comp: comp,
                                                  layers: layers,
                                                  open: _open,
                                                  hasAudio: _hasAudio,
                                                  peaks: _peaks,
                                                  fps: ui.model.fps,
                                                  fpsNum: fpsNum,
                                                  fpsDen: fpsDen,
                                                  magnet: _magnet,
                                                  axis: axis,
                                                  playhead: ui.playheadFrame,
                                                  razor: _razor,
                                                  vScroll: _vLane,
                                                  selectedKeys:
                                                      _laneKeySelection,
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
                                                      if (keys.isEmpty) return;
                                                      _selectedProperties
                                                          .clear();
                                                      for (final id in keys) {
                                                        final path =
                                                            id.substring(
                                                                0,
                                                                id.lastIndexOf(
                                                                    '#'));
                                                        if (!_selectedProperties
                                                            .contains(path)) {
                                                          _selectedProperties
                                                              .add(path);
                                                        }
                                                      }
                                                      final first =
                                                          _selectedProperties
                                                              .first;
                                                      final cut =
                                                          first.indexOf('/');
                                                      if (cut > 0) {
                                                        _highlighted = first
                                                            .substring(0, cut);
                                                      }
                                                    });
                                                  },
                                                  onWheel: (e, x) => _wheel(
                                                      e, x, axis.perFrame),
                                                  onSeek: (f) =>
                                                      ui.playheadFrame.value =
                                                          f.clamp(
                                                              0,
                                                              frames == 0
                                                                  ? 0
                                                                  : frames - 1),
                                                  onSelect: (l) => setState(() {
                                                    ui.selectedLayer.value = l;
                                                  }),
                                                  onChanged: ui.model.refresh,
                                                  cacheRevision:
                                                      Listenable.merge([
                                                    ui.frameArrived,
                                                    ui.cacheChanged
                                                  ]),
                                                  dragPreview: _barDrag,
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
                                                height: _rulerHeight +
                                                    TimelineCacheBar.height,
                                                color: t.surface2,
                                              ),
                                            ],
                                          ),
                                        ],
                                      ),
                                    ),
                                    _LaneBottomBar(
                                      zoom: _zoom,
                                      hScroll: _hLane,
                                      magnet: _magnet,
                                      onToggleMagnet: () =>
                                          setState(() => _magnet = !_magnet),
                                      onZoom: (z) => setState(
                                          () => _zoom = z.clamp(1.0, 64.0)),
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
        child: const PlaceholderPanel(
          icon: LumitIcon.comp,
          title: 'Timeline',
          hint: 'Open a composition, or drop footage here to make one.',
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

  const _FoldRow({
    required this.comp,
    required this.layer,
    required this.row,
    required this.valueColumn,
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
    // Selection rides on the property's *name* (docs/07 §4.3): the label
    // taps inside the row widgets call [onSelectProperty]; a click on the
    // rest of the row — its fields, its empty space — selects nothing.
    return Container(
      height: _rowHeight,
      // Selected is the full surface; a row that merely *contains* the
      // selection — the effect heading over a picked parameter — is the
      // same at half strength, exactly as a layer row marks itself.
      decoration: BoxDecoration(
        color: selected
            ? t.surface2
            : contains
                ? t.surface2.withValues(alpha: 0.45)
                : null,
      ),
      padding: EdgeInsets.only(left: indent, right: 4),
      child: _control(context),
    );
  }

  Widget _control(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return switch (row) {
      FoldWaveformRow() => const SizedBox.shrink(),
      FoldGroupRow(:final path, :final label, :final open) => GestureDetector(
          key: ValueKey<String>('tl-group-$path'),
          behavior: HitTestBehavior.opaque,
          onTap: () => onToggle(path),
          child: Row(
            children: [
              lumitIcon(
                open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                size: 12,
                color: open ? t.textPrimary : t.textMuted,
              ),
              const SizedBox(width: 4),
              Flexible(
                child:
                    Text(label, style: t.body, overflow: TextOverflow.ellipsis),
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
      FoldVolumeRow() => _VolumeRow(
          comp: comp,
          layer: layer,
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
class _VolumeRow extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  const _VolumeRow({
    required this.comp,
    required this.layer,
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
    final scalar = widget.layer.getVolumeDb();
    final animated = scalar is BridgeScalar_Keyframed;
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;

    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) {
        final value = _staged ??
            (animated
                ? sampleScalar(
                    scalar: scalar, time: timeOfFrame(widget.comp, frame))
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
            Expanded(child: Text('Volume', style: t.body)),
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
/// and only exists while the layer has been given a Retime (Alt+Shift+T), so
/// unlike Volume its scalar arrives on the fold row rather than being read here
/// (K-184: no bridge calls while drawing).
class _RetimeRow extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final BridgeScalar scalar;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  const _RetimeRow({
    required this.comp,
    required this.layer,
    required this.scalar,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
  });

  @override
  State<_RetimeRow> createState() => _RetimeRowState();
}

class _RetimeRowState extends State<_RetimeRow> {
  /// The value under the pointer during a drag, held so the whole gesture is
  /// one undo step. No live preview: a retime drag changes which frame is
  /// decoded, and there is no preview path for that yet — the release commits
  /// and the viewer re-renders then.
  double? _staged;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final scalar = widget.scalar;
    final animated = scalar is BridgeScalar_Keyframed;
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;

    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) {
        final value = _staged ??
            (animated
                ? sampleScalar(
                    scalar: scalar, time: widget.comp.timeOfFrame(frame: frame))
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
            Expanded(child: Text('Retime', style: t.body)),
            SizedBox(
              width: widget.valueColumn.width,
              child: animated
                  ? KeyedValueField(
                      fieldKey: const ValueKey('tl-retime-seconds'),
                      value: value,
                      // The same open range a transform axis gets: a source
                      // time before zero or past the end simply holds the end
                      // frame (docs/04 §7), so clamping the field would only
                      // fight the drag.
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

/// The waveform lane's painter: the layer's source peaks, mapped through its
/// live in/out/offset so dragging or trimming the bar carries the transients
/// with it in realtime (K-172). One vertical min-max line per pixel column.
class _WaveformPainter extends CustomPainter {
  final BridgeAudioPeaks? peaks;

  /// The span as drawn — the document's frames plus any drag in flight.
  final int inFrame;
  final int outFrame;
  final double startOffsetSeconds;
  final TimelineAxis axis;
  final double fps;
  final Color colour;

  const _WaveformPainter({
    required this.peaks,
    required this.inFrame,
    required this.outFrame,
    required this.startOffsetSeconds,
    required this.axis,
    required this.fps,
    required this.colour,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final held = peaks;
    if (held == null || held.pairs.isEmpty || held.durationSeconds <= 0) {
      return;
    }
    final buckets = held.pairs.length ~/ 2;
    final startOffset = startOffsetSeconds;
    final left = axis.xOf(inFrame).clamp(0.0, size.width);
    final right = axis.xOf(outFrame).clamp(0.0, size.width);
    final mid = size.height / 2;
    // Half a pixel of breathing room top and bottom.
    final half = mid - 1;
    final paintLine = Paint()
      ..color = colour
      ..strokeWidth = 1;

    for (var x = left; x < right; x += 1) {
      // Fractional, straight off the axis mapping: frameAt rounds to whole
      // frames, which would staircase the waveform.
      final compSec = x / axis.width * axis.frames / fps;
      final srcSec = compSec - startOffset;
      if (srcSec < 0 || srcSec >= held.durationSeconds) continue;
      final bucket = (srcSec / held.durationSeconds * buckets)
          .floor()
          .clamp(0, buckets - 1);
      final lo = held.pairs[bucket * 2].clamp(-1.0, 1.0);
      final hi = held.pairs[bucket * 2 + 1].clamp(-1.0, 1.0);
      canvas.drawLine(
        Offset(x, mid - hi * half),
        Offset(x, mid - lo * half),
        paintLine,
      );
    }
  }

  @override
  bool shouldRepaint(_WaveformPainter old) =>
      old.peaks != peaks ||
      old.inFrame != inFrame ||
      old.outFrame != outFrame ||
      old.startOffsetSeconds != startOffsetSeconds ||
      old.fps != fps ||
      old.axis.frames != axis.frames ||
      old.axis.width != axis.width;

  /// A background painter's default is to absorb hits across its whole rect,
  /// which would eat the keyframe marquee underneath. The lane is a picture,
  /// not a control.
  @override
  bool? hitTest(Offset position) => false;
}

/// The outline's toolbar (docs/07 §4.1): the timecode and frame readouts, the
/// layer search, the master motion-blur and shy-filter buttons, the Lane and
/// Graph view buttons, and the ⋯ menu holding the layer/work-area/marker
/// commands the old full-width toolbar carried.
class _Toolbar extends StatelessWidget {
  final CompositionReference comp;

  /// The read model, for the master motion-blur state and the exact rate —
  /// no bridge calls in a build (K-184).
  final CompModel model;

  /// Listened to, not read: only the two readouts redraw as it moves.
  final ValueListenable<int> playhead;
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
    return Container(
      height: _toolbarHeight,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Row(
        children: [
          // The clock face and the frame count, both zero-based: frame 0 is
          // 00:00:00:00, so three seconds into a 24 fps comp reads f72.
          ValueListenableBuilder<int>(
            valueListenable: playhead,
            builder: (context, frame, _) => Row(
              children: [
                Text(
                  timecodeOfRate(frame, fpsNum, fpsDen),
                  key: const ValueKey('tl-timecode'),
                  style: t.mono,
                ),
                const SizedBox(width: 6),
                Text(
                  'f$frame',
                  key: const ValueKey('tl-frame'),
                  style: t.mono.copyWith(color: t.textMuted),
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
            tip: mbOn
                ? 'Master motion blur on — layers with their switch set blur'
                : 'Master motion blur: enable the shutter for this comp',
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
            tip: hideShy
                ? 'Shy layers hidden from this list'
                : 'Hide shy layers from this list',
            onPressed: onToggleHideShy,
          ),
          const SizedBox(width: 6),
          _iconButton(
            context,
            keyName: 'tl-view-lanes',
            icon: LumitIcon.timelineBars,
            on: !graph,
            tip: 'Lane view',
            onPressed: graph ? onToggleGraph : () {},
          ),
          _iconButton(
            context,
            // Keeps the key the old Graph toolbar button had, so the graph
            // editor's own tests and muscle memory both still find it.
            keyName: 'tl-graph',
            icon: LumitIcon.graphCurve,
            on: graph,
            tip: 'Graph view',
            onPressed: graph ? () {} : onToggleGraph,
          ),
          const SizedBox(width: 6),
          HouseButton(
            key: const ValueKey('tl-more'),
            small: true,
            frameless: true,
            onPressed: () => _showMoreMenu(context),
            child: Text('⋯', style: t.small),
          ),
        ],
      ),
    );
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
        child: lumitIcon(icon, size: 12, color: on ? t.accent : t.textMuted),
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
    final picked = await showLumitPopup<String>(
      context: context,
      position: box.localToGlobal(Offset(box.size.width - 190, 24)),
      builder: (close) => FloatSurface(
        width: 190,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            MenuRow(
                key: const ValueKey('tl-add-layer'),
                onPressed: () => close('new-layer'),
                child: const Text('New layer')),
            MenuRow(
                key: const ValueKey('tl-razor'),
                onPressed: () => close('razor'),
                child: Text(razor ? 'Disarm razor' : 'Arm razor',
                    style: razor ? t.body.copyWith(color: t.accent) : null)),
            MenuRow(
                key: const ValueKey('tl-work-in'),
                onPressed: () => close('work-in'),
                child: const Text('Work area starts here')),
            MenuRow(
                key: const ValueKey('tl-work-out'),
                onPressed: () => close('work-out'),
                child: const Text('Work area ends here')),
            MenuRow(
                key: const ValueKey('tl-clear-work-area'),
                onPressed: () => close('work-clear'),
                child: const Text('Clear work area')),
            MenuRow(
                key: const ValueKey('tl-markers'),
                onPressed: () => close('markers'),
                child: const Text('Markers')),
            MenuRow(
                key: const ValueKey('tl-detect-beats'),
                onPressed: () => close('beats'),
                child: const Text('Detect beats')),
          ],
        ),
      ),
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
            frame: playheadNow,
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
        // pipeline, says so by doing nothing rather than by an alarm.
        comp
            .detectBeats(sensitivityPercent: 50)
            .then((_) => onChanged(), onError: (_) {});
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

  String _labelOf(TimelineGroup group) => switch (group) {
        TimelineGroup.switches => 'A/V',
        TimelineGroup.identity => 'Layer',
        TimelineGroup.render => 'Switches',
        TimelineGroup.compose => 'Matte · Blend · Parent',
      };

  /// The header cells, in the same widths the rows use, so each icon stands
  /// over its column. Indicators only — clicking a header does nothing; the
  /// switches live on the rows (docs/07 §4.2). Each carries a hover hint
  /// naming its column.
  Widget _cells(LumitTheme t, TimelineGroup group, double width) {
    Widget icon(LumitIcon i, String tip) => LumitTooltip(
          message: tip,
          child: Center(child: lumitIcon(i, size: 13, color: t.textMuted)),
        );
    Widget cell(LumitIcon i, String tip) =>
        SizedBox(width: switchCellWidth, child: icon(i, tip));
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
            cell(LumitIcon.eye, 'Visible'),
            cell(LumitIcon.audio, 'Audible'),
            cell(LumitIcon.ellipse, 'Solo — render only this layer'),
            cell(LumitIcon.lock, 'Lock — hold the layer still'),
            cell(LumitIcon.shy, 'Shy — hidden while the shy filter is on'),
          ],
        ),
      TimelineGroup.identity => Row(
          children: [
            const SizedBox(width: 16), // the twirl column has no header icon
            SizedBox(width: 16, child: icon(LumitIcon.label, 'Label colour')),
            const SizedBox(width: 4),
            Expanded(
              child: Text('Layer',
                  style: t.small, overflow: TextOverflow.ellipsis),
            ),
          ],
        ),
      // The switches pack left in ordinary cells; the rest of the group's
      // span is the fold-out's value column, not spare icon room.
      TimelineGroup.render => Row(
          children: [
            cell(LumitIcon.flow, 'Flow · collapse on a Precomp'),
            cell(LumitIcon.fx, 'Effects on or off'),
            cell(LumitIcon.motionBlur, 'Motion blur'),
            cell(LumitIcon.cube3d, '3D layer'),
          ],
        ),
      TimelineGroup.compose => () {
          final (matte, blend, parent) = composeCellWidths(width);
          return Row(
            children: [
              title('Matte', 'Matte — the layer that gates this one', matte),
              const SizedBox(width: cellGap),
              title('Blend', 'Blend mode', blend),
              const SizedBox(width: cellGap),
              title('Parent', 'Parent — transforms follow this layer', parent),
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
  final picked = await showLumitPopup<String>(
    context: context,
    position: box.localToGlobal(Offset(0, box.size.height + 2)),
    builder: (close) => FloatSurface(
      width: 190,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final kind in [
            'Solid',
            'Text',
            'Camera',
            'Adjustment',
            'Null',
            'Sequence'
          ])
            MenuRow(onPressed: () => close(kind), child: Text(kind)),
        ],
      ),
    ),
  );
  switch (picked) {
    case 'Solid':
      comp.addSolidLayer();
    case 'Text':
      comp.addTextLayer();
    case 'Camera':
      comp.addCameraLayer();
    case 'Adjustment':
      comp.addAdjustmentLayer();
    case 'Null':
      comp.addNullObjectLayer();
    case 'Sequence':
      comp.addSequenceLayer();
    case _:
      return;
  }
  onChanged();
}

/// The left column: one row per layer, with its switches and columns.
class _Outline extends StatelessWidget {
  final CompositionReference comp;
  final List<BridgeLayerEntry> layers;

  /// The column groups in their current order and at their current widths
  /// (docs/07 §4.2) — rows draw their cells to match the header's.
  final List<TimelineGroup> groupOrder;
  final Map<TimelineGroup, double> widths;
  final LayerReference? selected;
  final String? highlighted;

  /// The selected properties' fold paths, in selection order: each is a
  /// curve in the graph, its row draws selected, and every row containing
  /// one highlights (docs/07 §4.3, §5).
  final List<String> selectedProperties;

  /// Each selected path's graph line colours, for tinting its label.
  final Map<String, List<Color>> graphColours;
  final ValueChanged<String> onSelectProperty;
  final ValueChanged<String> onEditProperty;
  final Set<String> open;
  final Map<String, bool> hasAudio;
  final ValueChanged<String> onToggle;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final ValueChanged<LayerReference> onSelect;
  final ValueChanged<String> onHighlight;
  final VoidCallback onChanged;

  const _Outline({
    required this.comp,
    required this.layers,
    required this.groupOrder,
    required this.widths,
    required this.selected,
    required this.highlighted,
    required this.selectedProperties,
    required this.graphColours,
    required this.onSelectProperty,
    required this.onEditProperty,
    required this.open,
    required this.hasAudio,
    required this.onToggle,
    required this.playheadFrame,
    required this.onSeek,
    required this.onSelect,
    required this.onHighlight,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final valueColumn = valueColumnFor(groupOrder, widths);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (var i = 0; i < layers.length; i++) ...[
          _OutlineRow(
            key: ValueKey<String>('tl-row-${layers[i].layer.internallayerId}'),
            comp: comp,
            entry: layers[i],
            layers: layers,
            groupOrder: groupOrder,
            widths: widths,
            index: i,
            count: layers.length,
            // A local compare, not a bridge call: both ids already sit here.
            selected:
                selected?.internallayerId == layers[i].layer.internallayerId,
            // A layer marks itself when its fold was last touched, and when
            // a selected property is one of its own (docs/07 §4.3).
            highlighted: highlighted ==
                    layers[i].layer.internallayerId.toString() ||
                selectedProperties.any((p) =>
                    isUnderPath(layers[i].layer.internallayerId.toString(), p)),
            open: open.contains(layers[i].layer.internallayerId.toString()),
            onToggleOpen: () =>
                onToggle(layers[i].layer.internallayerId.toString()),
            onSelect: () => onSelect(layers[i].layer),
            onChanged: onChanged,
          ),
          // The fold-out, from the same list the lanes leave room for.
          if (open.contains(layers[i].layer.internallayerId.toString()))
            for (final row in layerFoldRows(
              entry: layers[i],
              open: open,
              hasAudio:
                  hasAudio[layers[i].layer.internallayerId.toString()] ?? false,
            ))
              // A raw pointer listener, not a gesture: touching a sub-item
              // highlights its layer, and it must never fight the row's own
              // taps and drags for the gesture arena.
              Listener(
                onPointerDown: (_) =>
                    onHighlight(layers[i].layer.internallayerId.toString()),
                child: _FoldRow(
                  comp: comp,
                  layer: layers[i].layer,
                  row: row,
                  valueColumn: valueColumn,
                  baseIndent: identityStart(groupOrder, widths),
                  path: foldRowPath(
                      layers[i].layer.internallayerId.toString(), row),
                  selectedProperties: selectedProperties,
                  graphColours: graphColours,
                  onSelectProperty: onSelectProperty,
                  onEditProperty: onEditProperty,
                  playheadFrame: playheadFrame,
                  onSeek: onSeek,
                  onToggle: onToggle,
                  onChanged: onChanged,
                ),
              ),
        ],
      ],
    );
  }
}

class _OutlineRow extends StatefulWidget {
  final CompositionReference comp;
  final BridgeLayerEntry entry;

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
  final VoidCallback onToggleOpen;
  final VoidCallback onSelect;
  final VoidCallback onChanged;

  const _OutlineRow({
    super.key,
    required this.comp,
    required this.entry,
    required this.layers,
    required this.groupOrder,
    required this.widths,
    required this.index,
    required this.count,
    required this.selected,
    required this.highlighted,
    required this.open,
    required this.onToggleOpen,
    required this.onSelect,
    required this.onChanged,
  });

  @override
  State<_OutlineRow> createState() => _OutlineRowState();
}

class _OutlineRowState extends State<_OutlineRow> {
  /// The inline rename, entered by double-clicking the name.
  TextEditingController? _rename;

  LayerReference get layer => widget.entry.layer;
  int get index => widget.index;
  int get count => widget.count;

  @override
  void dispose() {
    _rename?.dispose();
    super.dispose();
  }

  void _commitRename() {
    final text = _rename?.text.trim() ?? '';
    setState(() {
      _rename?.dispose();
      _rename = null;
    });
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

    return DragTarget<int>(
      // A layer dragged by its name lands here: dropping on this row puts it
      // at this row's place in the stack (docs/07 §4.7).
      onWillAcceptWithDetails: (d) => d.data != index && !info.switches.locked,
      onAcceptWithDetails: (d) {
        widget.layers[d.data].layer.reorder(newIndex: BigInt.from(index));
        widget.onChanged();
      },
      builder: (context, candidate, _) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: widget.onSelect,
        onSecondaryTapDown: (d) => _showRowMenu(context, d.globalPosition),
        child: Container(
          // A line where the dragged layer would land, so the drop is
          // visibly going somewhere rather than being taken on faith.
          foregroundDecoration: candidate.isEmpty
              ? null
              : BoxDecoration(
                  border: Border(top: BorderSide(color: t.accent, width: 2)),
                ),
          child: _rowBody(context, t, info),
        ),
      ),
    );
  }

  Widget _rowBody(BuildContext context, LumitTheme t, BridgeLayerInfo info) {
    return Container(
        key: ValueKey<String>('tl-rowbody-${layer.internallayerId}'),
        height: _rowHeight,
        decoration: BoxDecoration(
          // Selected is the brighter of the two states; a highlight (this
          // layer's fold-out was last touched) is the same surface at half
          // strength, so they read apart at a glance.
          color: widget.selected
              ? t.surface2
              : widget.highlighted
                  ? t.surface2.withValues(alpha: 0.45)
                  : null,
          // One hairline under every row, both halves of the table (K-190),
          // drawn inside the box so the row height — and the lane beside it
          // — is unchanged.
          border: Border(bottom: BorderSide(color: t.hairline)),
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
                child: switch (widget.groupOrder[i]) {
                  TimelineGroup.switches => _switchCells(context, info),
                  TimelineGroup.identity => _identityCells(context, t, info),
                  TimelineGroup.render => _renderCells(context, info),
                  TimelineGroup.compose => _composeCells(context, t, info,
                      widget.widths[TimelineGroup.compose] ?? 0),
                },
              ),
            ],
          ],
        ));
  }

  /// Group 1: visibility · audio · solo · lock · shy. The first two swap
  /// their glyph when off — a closed eye, a muted speaker — rather than only
  /// dimming, so the off state reads at a glance.
  Widget _switchCells(BuildContext context, BridgeLayerInfo info) {
    final id = layer.internallayerId.toString();
    final switches = info.switches;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        _switch(context, id, 'visible', LumitIcon.eye, switches.visible,
            BridgeLayerSwitch.visible,
            offIcon: LumitIcon.eyeClosed,
            tip: switches.visible ? 'Visible — click to hide' : 'Hidden'),
        _switch(context, id, 'audible', LumitIcon.audio, switches.audible,
            BridgeLayerSwitch.audible,
            offIcon: LumitIcon.mute,
            tip: switches.audible ? 'Audible — click to mute' : 'Muted'),
        // A circle, hollow until soloed.
        _switch(context, id, 'solo', LumitIcon.circleFilled, switches.solo,
            BridgeLayerSwitch.solo,
            offIcon: LumitIcon.ellipse,
            tip: switches.solo
                ? 'Soloed — only soloed layers render'
                : 'Solo this layer'),
        _switch(context, id, 'locked', LumitIcon.lock, switches.locked,
            BridgeLayerSwitch.locked,
            offIcon: LumitIcon.unlock,
            tip: switches.locked
                ? 'Locked — no edits until unlocked'
                : 'Lock this layer'),
        _switch(context, id, 'shy', LumitIcon.shyHidden, switches.shy,
            BridgeLayerSwitch.shy,
            offIcon: LumitIcon.shy,
            tip: switches.shy
                ? 'Shy — hidden while the shy filter is on'
                : 'Mark shy'),
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
          message: widget.open ? 'Fold the properties away' : 'Properties',
          child: GestureDetector(
            key: ValueKey<String>('tl-twirl-$id'),
            behavior: HitTestBehavior.opaque,
            onTap: widget.onToggleOpen,
            child: SizedBox(
              width: 16,
              height: _rowHeight,
              child: Center(
                child: lumitIcon(
                  widget.open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                  size: 13,
                  color: widget.open ? t.textPrimary : t.textMuted,
                ),
              ),
            ),
          ),
        ),
        LumitTooltip(
          message: 'Label colour — recolours the bar too',
          child: _labelSwatch(context, t, id, info.label),
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
        // A raw listener selects on the DOWN, outside the gesture arena: the
        // rename's double-tap holds the arena open for its whole window, so
        // selecting through the row's tap made a plain click on the name
        // reach the Effect controls a third of a second late — the same lag
        // the Project panel's rows cure the same way.
        Expanded(
          child: Listener(
            onPointerDown: (event) {
              if (event.buttons == kPrimaryButton) widget.onSelect();
            },
            child: info.switches.locked
                ? _name(t, id, info)
                : Draggable<int>(
                    data: index,
                    axis: Axis.vertical,
                    feedback: _dragLabel(t, info.name),
                    childWhenDragging:
                        Opacity(opacity: 0.4, child: _name(t, id, info)),
                    child: _name(t, id, info),
                  ),
          ),
        ),
        const SizedBox(width: 4),
      ],
    );
  }

  /// Group 3: flow (collapse on a Precomp) · fx · motion blur · 3D, spread
  /// across the same span the fold-out's value cells use.
  ///
  /// The flow slot: optical flow has no per-layer engine backing yet
  /// (docs/TODO.md), so a Precomp layer shows its collapse switch there —
  /// the spec's flow-or-collapse cell (K-168) — and other kinds leave it
  /// empty rather than offering a control that cannot do anything.
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
          info.kind == BridgeLayerKind.precomp
              ? _switch(context, id, 'collapse', LumitIcon.collapse,
                  switches.collapse, BridgeLayerSwitch.collapse,
                  tip: 'Collapse transformations')
              : const SizedBox(width: switchCellWidth),
          _switch(context, id, 'fx', LumitIcon.fx, switches.fx,
              BridgeLayerSwitch.fx,
              tip: switches.fx
                  ? 'Effects render — click to bypass'
                  : 'Effects bypassed'),
          _switch(context, id, 'mb', LumitIcon.motionBlur, switches.motionBlur,
              BridgeLayerSwitch.motionBlur,
              tip: 'Motion blur — needs the comp master on'),
          _switch(context, id, '3d', LumitIcon.cube3d, switches.threeD,
              BridgeLayerSwitch.threeD,
              tip: '3D layer'),
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
          message: 'Matte — the layer that gates this one',
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
          message: 'Blend mode',
          child: _blendPicker(context, t, info.blend, blendWidth),
        ),
        const SizedBox(width: cellGap),
        LumitTooltip(
          message: 'Parent — transforms follow this layer',
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

  /// What rides under the pointer while a layer is being dragged up or down
  /// the stack.
  Widget _dragLabel(LumitTheme t, String name) => Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
        decoration: BoxDecoration(
          color: t.surface3,
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          border: Border.all(color: t.accent),
        ),
        child: Text(name, style: t.body),
      );

  /// The name, or the rename editor a double-click turns it into. Submitting
  /// commits; clicking anywhere else commits too (the field loses the row).
  /// A locked layer's name does not open the editor: lock means no edits.
  Widget _name(LumitTheme t, String id, BridgeLayerInfo info) {
    final editor = _rename;
    if (editor != null) {
      return HouseTextField(
        key: ValueKey<String>('tl-rename-$id'),
        controller: editor,
        autofocus: true,
        onSubmitted: (_) => _commitRename(),
      );
    }
    return GestureDetector(
      key: ValueKey<String>('tl-name-$id'),
      behavior: HitTestBehavior.opaque,
      onDoubleTap: info.switches.locked
          ? null
          : () => setState(() {
                _rename = TextEditingController(text: info.name);
              }),
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
                  for (var i = 0; i < 8; i++)
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
  Widget _switch(
    BuildContext context,
    String id,
    String name,
    LumitIcon icon,
    bool on,
    BridgeLayerSwitch which, {
    LumitIcon? offIcon,
    String? tip,
  }) {
    final t = ThemeScope.of(context).theme;
    final glyph = on || offIcon != null
        ? lumitIcon(on ? icon : offIcon!,
            size: 13, color: on ? t.textPrimary : t.textMuted)
        : lumitIcon(icon, size: 13, color: t.textDisabled);
    final cell = GestureDetector(
      key: ValueKey<String>('tl-$name-$id'),
      behavior: HitTestBehavior.opaque,
      onTap: () {
        layer.setSwitch(switch_: which, on_: !on);
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
            child: Center(child: glyph),
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
        label: (i) => modes[i],
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
    final picked = await showLumitPopup<String>(
      context: context,
      position: position,
      builder: (close) => FloatSurface(
        width: 190,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            MenuRow(
                onPressed: () => close('duplicate'),
                child: const Text('Duplicate')),
            if (!locked) ...[
              if (index > 0)
                MenuRow(
                    onPressed: () => close('up'),
                    child: const Text('Bring forward')),
              if (index < count - 1)
                MenuRow(
                    onPressed: () => close('down'),
                    child: const Text('Send backward')),
              MenuRow(
                  onPressed: () => close('delete'),
                  child: const Text('Delete')),
            ],
          ],
        ),
      ),
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
      case _:
        return;
    }
    widget.onChanged();
  }
}

/// The right column: the ruler, the playhead, and one bar per layer.
class _LayerArea extends StatelessWidget {
  final CompositionReference comp;
  final List<BridgeLayerEntry> layers;

  /// Which layers are twirled open in the outline. Read only to leave the same
  /// room their property rows take, so a bar never drifts away from its name.
  final Set<String> open;

  /// Which layers carry sound — passed through only so the row list this side
  /// builds is identical to the outline's.
  final Map<String, bool> hasAudio;

  /// Each layer's source peaks, for the waveform lanes.
  final Map<String, BridgeAudioPeaks> peaks;

  /// The comp's rate, mapping the lane's pixels onto source seconds.
  final double fps;
  final TimelineAxis axis;

  /// Listened to, not read: only the playhead line moves when it changes.
  final ValueListenable<int> playhead;
  final bool razor;
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

  /// The lanes' vertical scroll — the outline mirrors it, and the thumb in
  /// the gutter beside this area is the one the user grabs.
  final ScrollController vScroll;

  /// The marquee's keyframe selection, as `rowId#index`, and where a new box
  /// reports what it caught.
  final Set<String> selectedKeys;
  final ValueChanged<Set<String>> onKeysSelected;

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
    required this.layers,
    required this.open,
    required this.hasAudio,
    required this.peaks,
    required this.fps,
    required this.axis,
    required this.playhead,
    required this.razor,
    required this.onSeek,
    required this.onSelect,
    required this.onChanged,
    required this.cacheRevision,
    required this.dragPreview,
    required this.vScroll,
    required this.selectedKeys,
    required this.onKeysSelected,
    required this.fpsNum,
    required this.fpsDen,
    required this.magnet,
    required this.onWheel,
  });

  /// The fold rows the lanes leave room for, per layer — one walk shared by
  /// the lane column, the marquee's hit maths and the diamonds.
  List<LayerFoldRow> _rowsOf(BridgeLayerEntry entry) => layerFoldRows(
        entry: entry,
        open: open,
        hasAudio: hasAudio[entry.layer.internallayerId.toString()] ?? false,
      );

  /// Every keyframe the box caught, walking the same rows the lanes draw —
  /// y from the row stack, x from the key's frame on the axis.
  Set<String> _keysIn(Rect rect) {
    final caught = <String>{};
    var y = 0.0;
    for (final entry in layers) {
      final id = entry.layer.internallayerId.toString();
      y += _rowHeight; // the layer's own bar row
      if (!open.contains(id)) continue;
      for (final row in _rowsOf(entry)) {
        final rowTop = y;
        y += _rowHeight;
        if (rowTop + _rowHeight < rect.top || rowTop > rect.bottom) continue;
        final keys = laneKeysOf(row);
        for (var i = 0; i < keys.length; i++) {
          final x = axis.xOf(laneKeyFrame(keys[i], fps));
          if (x >= rect.left && x <= rect.right) {
            caught.add('${foldRowPath(id, row)}#$i');
          }
        }
      }
    }
    return caught;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Stack(
      children: [
        Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            TimelineRuler(
              comp: comp,
              axis: axis,
              fps: fps,
              height: _rulerHeight,
              onSeek: onSeek,
            ),
            // Directly under the ruler and above the lanes, which is where the
            // interface spec puts it (docs/07 §3.2).
            TimelineCacheBar(comp: comp, axis: axis, revision: cacheRevision),
            // The rows scroll under the pinned ruler, in step with the
            // outline; the thumb lives in the gutter beside this area, so it
            // stays pinned to the viewport's edge rather than riding the
            // horizontally-scrolled content (docs/07 §4.6).
            Expanded(
              child: SingleChildScrollView(
                controller: vScroll,
                // Innermost, so the pointer-signal resolver hands it the
                // wheel before the scrollables do — a modified wheel zooms or
                // pans instead of scrolling, and a plain one is left alone.
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
                    if (!keys.isControlPressed && !keys.isShiftPressed) return;
                    GestureBinding.instance.pointerSignalResolver
                        .register(event, (resolved) {
                      if (resolved is PointerScrollEvent) {
                        onWheel(resolved, resolved.localPosition.dx);
                      }
                    });
                  },
                  child: Stack(
                    children: [
                      // Behind the bars: dragging empty lane space boxes up
                      // keyframes (docs/07 §4.3); bars and key handles above
                      // still win their own gestures.
                      Positioned.fill(
                        child: MarqueeSelect(
                          key: const ValueKey('tl-lane-marquee'),
                          onSelect: (rect) => onKeysSelected(_keysIn(rect)),
                          onClear: () => onKeysSelected(const {}),
                        ),
                      ),
                      Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          for (final entry in layers) ...[
                            _Bar(
                              key: ValueKey<String>(
                                  'tl-bar-${entry.layer.internallayerId}'),
                              comp: comp,
                              entry: entry,
                              axis: axis,
                              razor: razor,
                              playheadFrame: () => playhead.value,
                              onSelect: () => onSelect(entry.layer),
                              onChanged: onChanged,
                              dragPreview: dragPreview,
                            ),
                            // One lane per fold-out row the outline shows,
                            // from the same list it builds: keyframe rows
                            // draw their diamonds, the waveform row its
                            // peaks (K-172), the rest leave their room.
                            if (open.contains(
                                entry.layer.internallayerId.toString()))
                              Column(
                                key: ValueKey<String>(
                                    'tl-lanes-${entry.layer.internallayerId}'),
                                children: [
                                  for (final row in _rowsOf(entry))
                                    SizedBox(
                                      height: _rowHeight,
                                      child: _lane(t, entry, row),
                                    ),
                                ],
                              ),
                          ],
                        ],
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
                            ),
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ],
        ),
        // The playhead rides above every bar so it is never hidden behind one,
        // and it is the only thing here that redraws when it moves.
        ValueListenableBuilder<int>(
          valueListenable: playhead,
          builder: (context, frame, child) => Positioned(
            left: axis.xOf(frame),
            top: 0,
            bottom: 0,
            child: child!,
          ),
          child: IgnorePointer(
            child: Container(width: 1, color: t.accent),
          ),
        ),
      ],
    );
  }

  /// One fold row's lane: diamonds for a keyed property, the waveform for
  /// the waveform row, empty room otherwise.
  Widget? _lane(LumitTheme t, BridgeLayerEntry entry, LayerFoldRow row) {
    final id = entry.layer.internallayerId.toString();
    if (row is FoldWaveformRow) {
      return ValueListenableBuilder<BarDragPreview?>(
        valueListenable: dragPreview,
        builder: (context, preview, _) {
          final p = preview?.layerId == id ? preview : null;
          final span = entry.info.span;
          return CustomPaint(
            key: ValueKey<String>('tl-wave-$id'),
            size: Size(axis.width, _rowHeight),
            painter: _WaveformPainter(
              peaks: peaks[id],
              inFrame: entry.info.inFrame.toInt() + (p?.deltaIn ?? 0),
              outFrame: entry.info.outFrame.toInt() + (p?.deltaOut ?? 0),
              startOffsetSeconds:
                  span.startOffset.num / span.startOffset.den.toDouble() +
                      (p?.offsetShift ?? 0) / fps,
              axis: axis,
              fps: fps,
              colour: t.accent,
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
      selectedKeys: selectedKeys,
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
  final Set<String> selectedKeys;
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
    required this.selectedKeys,
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

  /// Where key [i] draws — its own time, plus the drag in flight.
  double _frameOf(int i) {
    final base = laneKeyFrame(widget.keys[i], widget.fps);
    if (_dragging != i) return base;
    final perFrame = widget.axis.perFrame;
    final moved = perFrame <= 0 ? base : base + _deltaPx / perFrame;
    final clamped = moved.clamp(0.0, widget.axis.frames.toDouble());
    return widget.magnet ? clamped.roundToDouble() : clamped;
  }

  void _commit(int index) {
    final frame = _frameOf(index);
    setState(() {
      _dragging = null;
      _deltaPx = 0;
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
    return Stack(
      children: [
        Positioned.fill(
          child: CustomPaint(
            painter: _LaneKeysPainter(
              frames: [
                for (var i = 0; i < widget.keys.length; i++) _frameOf(i)
              ],
              selected: {
                for (var i = 0; i < widget.keys.length; i++)
                  if (widget.selectedKeys.contains('${widget.rowId}#$i')) i,
              },
              axis: widget.axis,
              colour: t.textPrimary,
              chosen: t.accent,
            ),
          ),
        ),
        for (var i = 0; i < widget.keys.length; i++)
          Positioned(
            left: widget.axis.xOf(_frameOf(i)) - 6,
            top: 0,
            width: 12,
            height: _rowHeight,
            child: MouseRegion(
              cursor: SystemMouseCursors.resizeLeftRight,
              child: GestureDetector(
                key: ValueKey<String>('tl-key-${widget.rowId}#$i'),
                behavior: HitTestBehavior.opaque,
                onHorizontalDragStart: (_) => setState(() {
                  _dragging = i;
                  _deltaPx = 0;
                }),
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

  /// How far the first seam sits above the top edge — the outline's overlay
  /// is pinned to the panel rather than to the scrolled rows, so it carries
  /// the scroll offset here instead.
  final double phase;

  const _RowDividerPainter({
    required this.step,
    required this.colour,
    this.phase = 0,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (step <= 0) return;
    final paint = Paint()
      ..color = colour
      ..strokeWidth = 1;
    for (var y = phase + step; y <= size.height; y += step) {
      if (y < 0) continue;
      canvas.drawLine(Offset(0, y - 0.5), Offset(size.width, y - 0.5), paint);
    }
  }

  @override
  bool shouldRepaint(_RowDividerPainter old) =>
      old.step != step || old.colour != colour || old.phase != phase;

  /// Never absorbs a pointer: a background painter's default would eat the
  /// gestures on the rows below it.
  @override
  bool? hitTest(Offset position) => false;
}

/// A lane's keyframe diamonds: one per key, the marquee's catch in accent.
class _LaneKeysPainter extends CustomPainter {
  /// Fractional, so a key placed between frames draws between them.
  final List<double> frames;
  final Set<int> selected;
  final TimelineAxis axis;
  final Color colour;
  final Color chosen;

  const _LaneKeysPainter({
    required this.frames,
    required this.selected,
    required this.axis,
    required this.colour,
    required this.chosen,
  });

  @override
  void paint(Canvas canvas, Size size) {
    const half = 4.0;
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
      old.axis.frames != axis.frames ||
      old.axis.width != axis.width;

  /// A background painter's default is to absorb hits across its whole rect,
  /// which would eat the keyframe marquee underneath (the diamonds are picked
  /// up by the box, not clicked).
  @override
  bool? hitTest(Offset position) => false;
}

/// The lanes' bottom bar (docs/07 §4.5-§4.6): − / + / Fit with the zoom read
/// out, the magnet, and the horizontal scrollbar that moves the zoomed view.
///
/// In graph view it also carries the graph's own commands (docs/07 §5.3):
/// Linear / Bezier / Hold for the selected keys, the value/speed lens
/// switch, and the auto-fit toggle.
class _LaneBottomBar extends StatelessWidget {
  final double zoom;
  final ScrollController hScroll;
  final ValueChanged<double> onZoom;
  final bool magnet;
  final VoidCallback onToggleMagnet;

  /// Set in graph view; null hides the graph commands (the lane view).
  final GraphLens? lens;
  final ValueChanged<GraphLens>? onLens;
  final bool autoFit;
  final VoidCallback? onToggleAutoFit;
  final ValueChanged<BridgeSideInterp>? onInterp;

  const _LaneBottomBar({
    required this.zoom,
    required this.hScroll,
    required this.onZoom,
    required this.magnet,
    required this.onToggleMagnet,
    this.lens,
    this.onLens,
    this.autoFit = true,
    this.onToggleAutoFit,
    this.onInterp,
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
      height: 20,
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
                            label: 'Linear',
                            tip:
                                'Selected keyframes: straight lines both sides',
                            on: false,
                            onPressed: () => onInterp
                                ?.call(const BridgeSideInterp.linear())),
                        _graphButton(t,
                            keyName: 'graph-interp-bezier',
                            label: 'Bezier',
                            tip:
                                'Selected keyframes: easy ease (F9) — handles appear',
                            on: false,
                            onPressed: () => onInterp?.call(easyEase)),
                        _graphButton(t,
                            keyName: 'graph-interp-hold',
                            label: 'Hold',
                            tip: 'Selected keyframes: hold until the next key',
                            on: false,
                            onPressed: () =>
                                onInterp?.call(const BridgeSideInterp.hold())),
                        const SizedBox(width: 6),
                        _graphButton(t,
                            keyName: 'graph-lens-value',
                            label: 'Value',
                            tip: 'Value graph — value against time',
                            on: lens == GraphLens.value,
                            onPressed: () => onLens?.call(GraphLens.value)),
                        _graphButton(t,
                            keyName: 'graph-lens-speed',
                            label: 'Speed',
                            tip: 'Speed graph — how fast the value changes',
                            on: lens == GraphLens.speed,
                            onPressed: () => onLens?.call(GraphLens.speed)),
                        const SizedBox(width: 6),
                        _graphButton(t,
                            keyName: 'graph-autofit',
                            label: 'Auto fit',
                            tip: autoFit
                                ? 'Auto fit on — the graph frames its curves; click for '
                                    'manual scroll (wheel pans, Alt+wheel zooms)'
                                : 'Auto fit off — the wheel pans and Alt+wheel zooms '
                                    'the value axis',
                            on: autoFit,
                            onPressed: () => onToggleAutoFit?.call()),
                        const SizedBox(width: 6),
                      ],
                      ...[
                        HouseButton(
                          key: const ValueKey('tl-zoom-out'),
                          small: true,
                          frameless: true,
                          onPressed: () => onZoom(zoom / 1.5),
                          child: Text('−', style: t.small),
                        ),
                        SizedBox(
                          width: 44,
                          child: Text('${(zoom * 100).round()}%',
                              key: const ValueKey('tl-zoom-label'),
                              style: t.small.copyWith(color: t.textMuted),
                              textAlign: TextAlign.center),
                        ),
                        HouseButton(
                          key: const ValueKey('tl-zoom-in'),
                          small: true,
                          frameless: true,
                          onPressed: () => onZoom(zoom * 1.5),
                          child: Text('+', style: t.small),
                        ),
                        HouseButton(
                          key: const ValueKey('tl-zoom-fit'),
                          small: true,
                          frameless: true,
                          onPressed: () => onZoom(1),
                          child: Text('Fit', style: t.small),
                        ),
                        const SizedBox(width: 6),
                        LumitTooltip(
                          message: magnet
                              ? 'Magnet on — dragged keyframes land on whole frames'
                              : 'Magnet off — keyframes may sit between frames',
                          child: HouseButton(
                            key: const ValueKey('tl-magnet'),
                            small: true,
                            frameless: true,
                            padding: const EdgeInsets.symmetric(
                                horizontal: 4, vertical: 2),
                            onPressed: onToggleMagnet,
                            child: lumitIcon(LumitIcon.magnet,
                                size: 13,
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

  /// Clicking (or grabbing) the bar selects its layer.
  final VoidCallback onSelect;
  final VoidCallback onChanged;

  /// Where the live preview is published, for the waveform lane to follow.
  final ValueNotifier<BarDragPreview?> dragPreview;

  const _Bar({
    super.key,
    required this.comp,
    required this.entry,
    required this.axis,
    required this.razor,
    required this.playheadFrame,
    required this.onSelect,
    required this.onChanged,
    required this.dragPreview,
  });

  @override
  State<_Bar> createState() => _BarState();
}

class _BarState extends State<_Bar> {
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

    // The bar fills the row's whole height rather than floating inside an
    // inset, so a layer reads as a solid band; the lane area's own hairline
    // overlay draws the row seam over it (K-190).
    return SizedBox(
      height: _rowHeight,
      child: Stack(
        children: [
          Positioned(
            left: left,
            width: width,
            top: 0,
            bottom: 0,
            // Selection on the raw DOWN, outside the gesture arena: the
            // bar's tap otherwise waits for the move/trim drag recognisers
            // to concede before the Effect controls learn the layer.
            child: Listener(
              onPointerDown: (event) {
                if (event.buttons == kPrimaryButton) widget.onSelect();
              },
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                // Armed razor: a click cuts the clip under the playhead rather
                // than starting a drag. A layer with no clip there says so
                // through the engine's calm error, which is nothing on screen —
                // the cut simply does not happen.
                onTap: widget.razor && !held
                    ? () {
                        try {
                          widget.entry.layer
                              .cutClipAt(frame: widget.playheadFrame());
                        } catch (_) {
                          return;
                        }
                        widget.onChanged();
                      }
                    // Selection already happened on the down; the tap has
                    // nothing left to do, but registering it keeps the click
                    // out of any parent recogniser's hands.
                    : () {},
                onHorizontalDragDown: widget.razor || held
                    ? null
                    : (d) => _downDx = d.localPosition.dx,
                onHorizontalDragStart: widget.razor || held
                    ? null
                    // No select here: every drag begins with the down, and the
                    // down already selected.
                    : (d) => setState(() {
                          _delta = 0;
                          _deltaPx = 0;
                          _grab = _downDx < _trimGrab
                              ? BarGrab.trimIn
                              : _downDx > width - _trimGrab
                                  ? BarGrab.trimOut
                                  : BarGrab.move;
                        }),
                onHorizontalDragUpdate: widget.razor || held
                    ? null
                    : (d) => setState(() {
                          _deltaPx += d.delta.dx;
                          _delta = widget.axis.frameAt(_deltaPx);
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
                    color: t.labelColour(info.label),
                    borderRadius: BorderRadius.circular(2),
                  ),
                  // A Sequence layer draws its clip splits, so the razor has
                  // something to aim at and a cut is visible once made.
                  child: Stack(
                    children: [
                      for (final clipFrame in info.clipFrames)
                        Positioned(
                          left: widget.axis.xOf(clipFrame.toInt()) - 0.5,
                          top: 0,
                          bottom: 0,
                          child: Container(width: 1, color: t.surface0),
                        ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ],
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
    final delta = _delta;
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
