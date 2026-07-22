// The Timeline panel (phase F3, wave 2): a comp-tab strip, a layer-search box,
// the two-row time ruler with the work-area band, the per-layer outline + lane
// rows (with twirl-down transform property rows and their keyframe lanes), a
// horizontal pan scrollbar, and the bottom bar's zoom / magnet / graph-lens
// controls. When no composition is open it keeps the F0 placeholder centre.
// Pure geometry, the glyph coding, work-area hit-test, search filter and pan
// clamp live in panels/timeline/ and are unit-tested; this file is the widget
// composition and the session-only interaction state (twirls, selection, pan).

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../bridge/bridge.dart';
import '../icons/icons.dart';
import '../shell/dialogs.dart';
import '../state/app_state.dart';
import '../widgets/controls.dart';
import 'timeline/cache_bar.dart';
import 'timeline/comp_tabs.dart';
import 'timeline/graph_editor.dart';
import 'timeline/group_header.dart';
import 'timeline/keyframe_clipboard.dart';
import 'timeline/lane_context_menu.dart';
import 'timeline/lane_host.dart';
import 'timeline/lane_scale.dart';
import 'timeline/lane_selection.dart';
import 'timeline/layer_row.dart';
import 'timeline/property_row.dart';
import 'timeline/property_rows.dart';
import 'timeline/ruler.dart';
import 'timeline/search.dart';

/// The default outline-column width (px). The divider between the outline and
/// the lane drags it (session-only), and it degrades the switch cluster when
/// the panel is too narrow to hold the wider setting.
const double _kOutlineWidth = 260;
const double _kRulerHeight = 36;

class TimelinePanel extends StatelessWidget {
  final AppStateStub app;
  const TimelinePanel({super.key, required this.app});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ListenableBuilder(
      listenable: app,
      builder: (context, _) {
        final comp = app.frontComp;
        final compId = app.frontCompIdResolved;
        return Column(
          children: [
            CompTabStrip(app: app),
            Expanded(
              child: (comp != null && compId != null)
                  ? _TimelineBody(app: app, comp: comp, compId: compId)
                  : Center(
                      child: Text(
                        'Layer rows, lanes and the graph lens arrive in phase F3.',
                        style: t.small,
                      ),
                    ),
            ),
            _BottomBar(app: app),
          ],
        );
      },
    );
  }
}

/// The live timeline body when a comp is fronted: the search box + two-row
/// ruler on top, the layer rows below (scrolling vertically), a pan scrollbar,
/// and the playhead + work-area overlays. Holds the session-only interaction
/// state (twirls, lane selection, horizontal pan).
class _TimelineBody extends StatefulWidget {
  final AppStateStub app;
  final BridgeComp comp;
  final String compId;
  const _TimelineBody({
    required this.app,
    required this.comp,
    required this.compId,
  });

  @override
  State<_TimelineBody> createState() => _TimelineBodyState();
}

class _TimelineBodyState extends State<_TimelineBody>
    implements TimelineLaneHost {
  /// Layer ids whose outline twirl is open (property rows shown).
  final Set<String> _open = {};

  /// The horizontal pan: the view's left-edge comp frame (0 = comp start).
  /// Persisted only for this session (widget state), as the brief asks.
  double _viewStart = 0;

  /// The outline-column width, dragged by the divider between the outline and
  /// the lane (egui persists it; session-only widget state here, like [_viewStart]).
  double _outlineWidth = _kOutlineWidth;

  /// Whether the lane time grid (vertical guide lines) is drawn — the empty-lane
  /// menu's "Show time grid" toggle, session-only lane state (egui's
  /// `TimelineGrid`, panel.rs:398).
  bool _showTimeGrid = false;

  /// The layer-search query (case-insensitive substring filter).
  final TextEditingController _searchController = TextEditingController();
  final FocusNode _searchFocus = FocusNode();
  String _search = '';

  // --- Lane keyframe selection + drag (TimelineLaneHost) ------------------
  final Set<LaneKeyId> _selectedKeys = {};
  int _dragGrabFrame = 0;
  int _dragDelta = 0;
  bool _dragActive = false;

  // --- Layer drag-reorder --------------------------------------------------
  /// A per-layer key so the body can measure each row's centre while reordering
  /// (mirroring egui's `layer_row_centers`).
  final Map<String, GlobalKey> _rowKeys = {};

  /// The layer being dragged to restack, and the pointer's last global Y, while
  /// a reorder drag is live (both null otherwise).
  String? _reorderId;
  double? _reorderGlobalY;

  /// A GlobalKey on the body's outer Stack so the insertion line can convert a
  /// global gap-Y into the Stack's local coordinates.
  final GlobalKey _stackKey = GlobalKey();

  GlobalKey _rowKey(String id) => _rowKeys.putIfAbsent(id, () => GlobalKey());

  AppStateStub get app => widget.app;

  /// The visible layers after the search filter (top-first) — the reorder
  /// target index and the build both read this so they agree.
  List<BridgeLayer> _visibleLayers() => [
        for (final l in widget.comp.layers)
          if (layerMatchesSearch(l.name, _search)) l,
      ];

  /// A layer row's centre Y in global coordinates, or null before it has laid
  /// out (a filtered-out or freshly added row).
  double? _rowCentreGlobalY(String id) {
    final ctx = _rowKeys[id]?.currentContext;
    final box = ctx?.findRenderObject() as RenderBox?;
    if (box == null || !box.hasSize) return null;
    return box.localToGlobal(box.size.center(Offset.zero)).dy;
  }

  void _reorderStart(String id) {
    app.selectLayer(id);
    setState(() {
      _reorderId = id;
      _reorderGlobalY = null;
    });
  }

  void _reorderUpdate(Offset globalPos) {
    setState(() => _reorderGlobalY = globalPos.dy);
  }

  void _reorderEnd() {
    final id = _reorderId;
    final gy = _reorderGlobalY;
    if (id != null && gy != null) {
      final layers = _visibleLayers();
      final old = layers.indexWhere((l) => l.id == id);
      // The target index counts the OTHER rows whose centre sits above the
      // release Y — exactly the lifted-out insert index egui's ReorderLayer
      // takes (panel.rs:1770).
      var target = 0;
      for (final l in layers) {
        if (l.id == id) continue;
        final c = _rowCentreGlobalY(l.id);
        if (c != null && c < gy) target++;
      }
      if (old != -1 && target != old) {
        app.reorderLayer(widget.compId, id, target);
      }
    }
    setState(() {
      _reorderId = null;
      _reorderGlobalY = null;
    });
  }

  void _reorderCancel() {
    setState(() {
      _reorderId = null;
      _reorderGlobalY = null;
    });
  }

  /// The insertion line's Y in the Stack's local coordinates while a reorder
  /// drag is live, or null when it cannot be placed yet.
  double? _insertionLineY(List<BridgeLayer> layers) {
    final id = _reorderId;
    final gy = _reorderGlobalY;
    if (id == null || gy == null) return null;
    final others = <double>[];
    for (final l in layers) {
      if (l.id == id) continue;
      final c = _rowCentreGlobalY(l.id);
      if (c != null) others.add(c);
    }
    if (others.isEmpty) return null;
    others.sort();
    final target = others.where((c) => c < gy).length;
    final double gapGlobalY;
    if (target == 0) {
      gapGlobalY = others.first - kRowHeight / 2;
    } else if (target >= others.length) {
      gapGlobalY = others.last + kRowHeight / 2;
    } else {
      gapGlobalY = (others[target - 1] + others[target]) / 2;
    }
    final stackBox = _stackKey.currentContext?.findRenderObject() as RenderBox?;
    if (stackBox == null) return null;
    return stackBox.globalToLocal(Offset(0, gapGlobalY)).dy;
  }

  @override
  void initState() {
    super.initState();
    // Install the keyframe copy/paste handlers the shell's Ctrl+C/V drive (the
    // selection lives here; the clipboard payload lives on the app state so it
    // survives this body being rebuilt).
    app.copyKeyframesHandler = _copySelectedKeyframes;
    app.pasteKeyframesHandler = _pasteKeyframes;
  }

  @override
  void dispose() {
    // Uninstall our handlers only if they are still ours (a newer body may have
    // replaced them).
    if (app.copyKeyframesHandler == _copySelectedKeyframes) {
      app.copyKeyframesHandler = null;
    }
    if (app.pasteKeyframesHandler == _pasteKeyframes) {
      app.pasteKeyframesHandler = null;
    }
    _searchController.dispose();
    _searchFocus.dispose();
    super.dispose();
  }

  /// Copy the selected lane keys into the app clipboard (egui note 2.2). A quiet
  /// no-op when nothing is selected.
  void _copySelectedKeyframes() {
    if (_selectedKeys.isEmpty) return;
    final clip = buildKeyframeClipboard(_selectedKeys, widget.comp);
    if (clip.isEmpty) return;
    app.keyframeClipboard = clip.encode();
    app.setNotice(_selectedKeys.length == 1
        ? '1 keyframe copied'
        : '${_selectedKeys.length} keyframes copied');
  }

  /// Paste the copied keys at the playhead: values in one batch per layer, then
  /// each eased key's shape restored, then the selection follows the paste.
  void _pasteKeyframes() {
    final clip = KeyframeClipboard.decode(app.keyframeClipboard);
    if (clip.isEmpty) return;
    final playhead = app.previewFrame;
    for (final layerId in clip.layerIds) {
      app.applyKeyframeBatch(
          widget.compId, layerId, pasteAddBatchJson(clip, layerId, playhead));
    }
    // Restore the easing the value-only batch could not carry.
    for (final k in clip.keys) {
      if (!k.eases) continue;
      app.setKeyframeInterp(
        widget.compId,
        k.layerId,
        k.property,
        playhead + k.frameOffset,
        k.interpIn,
        k.interpOut,
        speedIn: k.speedIn ?? 0,
        influenceIn: k.influenceIn ?? 1.0 / 3.0,
        speedOut: k.speedOut ?? 0,
        influenceOut: k.influenceOut ?? 1.0 / 3.0,
      );
    }
    setState(() {
      _selectedKeys
        ..clear()
        ..addAll(pastedKeyIds(clip, playhead));
    });
  }

  @override
  Set<LaneKeyId> get selectedKeys => _selectedKeys;
  @override
  bool get keyDragActive => _dragActive;
  @override
  int get keyDragDelta => _dragDelta;

  @override
  void keyTap(LaneKeyId key, {required bool additive}) {
    setState(() => laneSelectClick(_selectedKeys, key, additive: additive));
  }

  @override
  void keyDragStart(LaneKeyId grabbed, int grabFrame) {
    setState(() {
      if (!_selectedKeys.contains(grabbed)) {
        _selectedKeys
          ..clear()
          ..add(grabbed);
      }
      _dragGrabFrame = grabFrame;
      _dragDelta = 0;
      _dragActive = true;
    });
  }

  @override
  void keyDragTo(int frame) {
    // Clamp so no selected key slides before frame 0 (egui's `.max(0.0)`).
    var delta = frame - _dragGrabFrame;
    var minFrame = 1 << 30;
    for (final k in _selectedKeys) {
      if (k.frame < minFrame) minFrame = k.frame;
    }
    if (minFrame != 1 << 30 && minFrame + delta < 0) delta = -minFrame;
    setState(() => _dragDelta = delta);
  }

  @override
  void keyDragEnd() {
    final delta = _dragDelta;
    if (delta != 0) {
      // One shiftKeyframes per (layer, property) channel — a single undo step
      // each, exactly as egui commits its lane drag per property.
      final groups = groupKeysForShift(_selectedKeys);
      groups.forEach((channel, frames) {
        app.shiftKeyframes(widget.compId, channel.$1, channel.$2, frames, delta);
      });
      // The selection follows the moved keys to their new frames.
      final moved = <LaneKeyId>{
        for (final k in _selectedKeys)
          LaneKeyId(k.layerId, k.property, k.frame + delta),
      };
      _selectedKeys
        ..clear()
        ..addAll(moved);
    }
    setState(() {
      _dragActive = false;
      _dragDelta = 0;
    });
  }

  @override
  void keyRemove(LaneKeyId key) {
    _selectedKeys.remove(key);
    app.removeKeyframe(widget.compId, key.layerId, key.property, key.frame);
  }

  void _toggleOpen(String layerId) {
    setState(() {
      if (!_open.remove(layerId)) _open.add(layerId);
    });
  }

  /// Pan the view by [framesDelta], clamped to the comp ends.
  void _pan(double framesDelta, int frameCount, double zoom) {
    setState(() {
      _viewStart = LaneScale.clampViewStart(
        desired: _viewStart + framesDelta,
        frameCount: frameCount,
        zoom: zoom,
      );
    });
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LayoutBuilder(
      builder: (context, constraints) {
        final totalW = constraints.maxWidth;
        // The outline never swallows the lane; the dragged width is clamped so
        // the lane keeps at least 80 px, and the switch cluster degrades to suit.
        final outlineW =
            _outlineWidth.clamp(60.0, (totalW - 80).clamp(60.0, double.infinity));
        final trackLeft = outlineW.toDouble();
        final trackW = (totalW - outlineW - 8).clamp(40.0, double.infinity);
        final scale = LaneScale.fit(
          trackLeft: trackLeft,
          trackWidth: trackW.toDouble(),
          frameCount: widget.comp.frameCount,
          zoom: app.timelineZoom,
          desiredStartFrame: _viewStart,
        );
        final fps = widget.comp.fps.fps;
        final markers = widget.comp.markers;
        // The visible layers after the search filter (top-first).
        final layers = _visibleLayers();

        return Stack(
          key: _stackKey,
          children: [
            Column(
              children: [
                // Ruler band: the search box on the left over the outline, the
                // two-row ruler with the work-area band over the lane.
                SizedBox(
                  height: _kRulerHeight,
                  child: Row(
                    children: [
                      Container(
                        width: outlineW.toDouble(),
                        decoration: BoxDecoration(
                          color: t.surface1,
                          border: Border(
                            bottom: BorderSide(color: t.hairline, width: 1),
                          ),
                        ),
                        alignment: Alignment.centerLeft,
                        padding: const EdgeInsets.symmetric(horizontal: 6),
                        // The top row (egui's timeline header): the layer search
                        // and, at its right, the composition motion-blur master.
                        child: Row(
                          children: [
                            Expanded(
                              child: _SearchField(
                                controller: _searchController,
                                focus: _searchFocus,
                                onChanged: (v) => setState(() => _search = v),
                              ),
                            ),
                            const SizedBox(width: 4),
                            _MotionBlurMaster(app: app),
                          ],
                        ),
                      ),
                      Expanded(
                        child: Stack(
                          children: [
                            TimelineRuler(
                              app: app,
                              scale: scale,
                              fps: fps,
                              markers: markers,
                              markerDetails: widget.comp.markerDetails,
                              height: _kRulerHeight,
                            ),
                            // The warm-frame cache bar (RAM tier) along the
                            // ruler's bottom edge, over the ticks.
                            TimelineCacheBar(
                              app: app,
                              compId: widget.compId,
                              scale: scale,
                              height: _kRulerHeight,
                            ),
                            if (widget.comp.workArea != null)
                              _WorkAreaBand(
                                app: app,
                                compId: widget.compId,
                                scale: scale,
                                workArea: widget.comp.workArea!,
                                height: _kRulerHeight,
                              ),
                          ],
                        ),
                      ),
                    ],
                  ),
                ),
                // The lane area: the graph editor when the graph lens is on,
                // else the scrolling layer rows. The ruler above and the pan
                // scrollbar below stay put, and the graph shares this `scale`.
                // A footage item dragged from the Project panel drops here to
                // become a new layer (top of the stack, mirroring egui's
                // `add_footage_to_comp`).
                Expanded(
                  child: DragTarget<FootageDragData>(
                    onAcceptWithDetails: (d) =>
                        app.addFootageLayer(widget.compId, d.data.itemId),
                    builder: (context, candidate, rejected) => Stack(
                      children: [
                        Positioned.fill(
                          child: _laneArea(
                              scale, fps, markers, layers, outlineW),
                        ),
                        if (candidate.isNotEmpty)
                          Positioned.fill(
                            child: IgnorePointer(
                              child: Container(
                                color: t.accent.withValues(alpha: 0.08),
                              ),
                            ),
                          ),
                      ],
                    ),
                  ),
                ),
                if (scale.canPan)
                  _PanScrollbar(
                    outlineWidth: outlineW.toDouble(),
                    trackWidth: trackW.toDouble(),
                    frameCount: widget.comp.frameCount,
                    zoom: app.timelineZoom,
                    viewStart: _viewStart,
                    onPan: (start) => setState(() => _viewStart = start),
                  ),
              ],
            ),
            // The live insertion line while a layer is being drag-reordered,
            // drawn across the outline column at the gap it would drop into.
            if (_reorderId != null)
              Builder(builder: (_) {
                final y = _insertionLineY(layers);
                if (y == null) return const SizedBox.shrink();
                return Positioned(
                  left: 0,
                  width: outlineW.toDouble(),
                  top: y - 1,
                  child: IgnorePointer(
                    child: Container(height: 2, color: t.accent),
                  ),
                );
              }),
            // The outline/lane divider: drag it to resize the outline column
            // (session-only, egui persists it). Clamped so the lane keeps 80 px.
            Positioned(
              left: (outlineW - 3).toDouble(),
              top: 0,
              bottom: scale.canPan ? _PanScrollbar.height : 0,
              width: 6,
              child: MouseRegion(
                cursor: SystemMouseCursors.resizeLeftRight,
                child: GestureDetector(
                  behavior: HitTestBehavior.translucent,
                  onHorizontalDragUpdate: (d) => setState(() {
                    _outlineWidth = (_outlineWidth + d.delta.dx)
                        .clamp(60.0, (totalW - 80).clamp(60.0, double.infinity));
                  }),
                  child: const SizedBox.expand(),
                ),
              ),
            ),
            // The playhead line is the only thing that moves per frame, so it
            // alone watches the fine-grained playhead notifier — the ruler,
            // layer rows and lanes above stay outside this rebuild (perf pass:
            // scrubbing no longer rebuilds the rows subtree).
            ValueListenableBuilder<int>(
              valueListenable: app.playheadFrame,
              builder: (context, frame, _) {
                final playheadX = scale.xOfFrame(frame);
                final showPlayhead = playheadX >= trackLeft - 0.5 &&
                    playheadX <= trackLeft + trackW + 0.5;
                if (!showPlayhead) return const SizedBox.shrink();
                return Positioned(
                  left: playheadX,
                  top: 0,
                  bottom: scale.canPan ? _PanScrollbar.height : 0,
                  child: IgnorePointer(
                    child: Container(width: 1, color: t.accent),
                  ),
                );
              },
            ),
          ],
        );
      },
    );
  }

  /// The lane area's inner content (graph editor or the scrolling layer rows),
  /// factored out so the [DragTarget] wrapper stays legible.
  Widget _laneArea(
    LaneScale scale,
    double fps,
    List<int> markers,
    List<BridgeLayer> layers,
    double outlineW,
  ) {
    if (app.timelineGraphMode) {
      return GraphEditor(
        app: app,
        comp: widget.comp,
        compId: widget.compId,
        scale: scale,
      );
    }
    final t = ThemeScope.of(context).theme;
    // The lane background: the optional time grid, and a right-click target for
    // the empty-lane context menu. Drawn BEHIND the rows so a clip bar or a
    // keyframe (with its own handlers) wins the hit-test — the menu opens only
    // on empty lane space (egui adds the marquee bg before the rows, panel.rs:357).
    final background = Positioned.fill(
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onSecondaryTapDown: (d) {
          if (d.localPosition.dx >= scale.trackLeft) {
            _openLaneMenu(d.globalPosition);
          }
        },
        child: CustomPaint(
          painter: _TimeGridPainter(
            scale: scale,
            fps: fps,
            show: _showTimeGrid,
            line: t.hairline,
          ),
        ),
      ),
    );
    final rows = Positioned.fill(
      child: Listener(
        onPointerSignal: (e) {
          if (e is! PointerScrollEvent || !scale.canPan) return;
          final shift = HardwareKeyboard.instance.isShiftPressed;
          final dx = e.scrollDelta.dx != 0
              ? e.scrollDelta.dx
              : (shift ? e.scrollDelta.dy : 0);
          if (dx == 0) return;
          _pan(dx / scale.pxPerFrame, widget.comp.frameCount, app.timelineZoom);
        },
        child: SingleChildScrollView(
          child: Column(
            children: [
              for (var i = 0; i < layers.length; i++)
                ..._layerBlock(
                    layers[i], i, outlineW.toDouble(), scale, fps, markers),
            ],
          ),
        ),
      ),
    );
    return Stack(children: [background, rows]);
  }

  /// Open the empty-lane context menu at [globalPosition] (row 2). Comp settings
  /// routes to the shared dialogue; Reveal in project / grid / beats go through
  /// the app state and this body's own grid toggle.
  void _openLaneMenu(Offset globalPosition) {
    showLaneContextMenu(
      context: context,
      app: app,
      compId: widget.compId,
      showTimeGrid: _showTimeGrid,
      onToggleGrid: () => setState(() => _showTimeGrid = !_showTimeGrid),
      onCompositionSettings: () => showCompositionSettingsDialog(context, app),
      position: globalPosition,
    );
  }

  /// One layer's rows: the clip row, then — when its twirl is open — the
  /// Transform group header and one row per transform property.
  List<Widget> _layerBlock(
    BridgeLayer layer,
    int displayIndex,
    double outlineW,
    LaneScale scale,
    double fps,
    List<int> markers,
  ) {
    final open = _open.contains(layer.id);
    final rows = <Widget>[
      LayerRow(
        key: _rowKey(layer.id),
        app: app,
        compId: widget.compId,
        layer: layer,
        displayIndex: displayIndex,
        outlineWidth: outlineW,
        scale: scale,
        fps: fps,
        markers: markers,
        open: open,
        onToggleOpen: () => _toggleOpen(layer.id),
        onReorderStart: _reorderStart,
        onReorderUpdate: _reorderUpdate,
        onReorderEnd: _reorderEnd,
        onReorderCancel: _reorderCancel,
      ),
    ];
    if (open) {
      final threeD = layer.switches.threeD;
      final isCamera = layer.kind == BridgeLayerKind.camera;
      rows.add(GroupHeaderRow(
        key: ValueKey('group:${layer.id}:transform'),
        label: 'Transform',
        open: true,
        outlineWidth: outlineW,
        onTap: () => _toggleOpen(layer.id),
      ));
      for (final spec in transformRows(threeD: threeD, isCamera: isCamera)) {
        rows.add(PropertyRow(
          key: ValueKey('prop:${layer.id}:${spec.primary}'),
          app: app,
          compId: widget.compId,
          layer: layer,
          spec: spec,
          outlineWidth: outlineW,
          scale: scale,
          host: this,
        ));
      }
    }
    return rows;
  }
}

/// The layer-search box in the outline header (egui's top-row search): a bare
/// single-line field, its placeholder shown when empty.
class _SearchField extends StatelessWidget {
  final TextEditingController controller;
  final FocusNode focus;
  final ValueChanged<String> onChanged;
  const _SearchField({
    required this.controller,
    required this.focus,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      height: 22,
      padding: const EdgeInsets.symmetric(horizontal: 8),
      decoration: BoxDecoration(
        color: t.surface0,
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        border: Border.all(color: t.hairline),
      ),
      alignment: Alignment.centerLeft,
      child: Stack(
        alignment: Alignment.centerLeft,
        children: [
          if (controller.text.isEmpty)
            Text('Search layers', style: t.small.copyWith(color: t.textMuted)),
          EditableText(
            controller: controller,
            focusNode: focus,
            style: t.small.copyWith(color: t.textSecondary),
            cursorColor: t.accent,
            backgroundCursorColor: t.surface2,
            selectionColor: t.accent.withValues(alpha: 0.4),
            onChanged: onChanged,
            maxLines: 1,
          ),
        ],
      ),
    );
  }
}

/// The work-area band on the ruler: a filled strip in the success tint along
/// the ruler top between the in/out edges (mirroring the egui draw), dimmed
/// outside, with two draggable edge brackets that move the edge through
/// [AppStateStub.setWorkAreaEdge]. Its own local x=0 maps to the lane left.
class _WorkAreaBand extends StatelessWidget {
  final AppStateStub app;
  final String compId;
  final LaneScale scale;
  final List<int> workArea;
  final double height;
  const _WorkAreaBand({
    required this.app,
    required this.compId,
    required this.scale,
    required this.workArea,
    required this.height,
  });

  double _laneX(int frame) => scale.xOfFrame(frame) - scale.trackLeft;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final inX = _laneX(workArea[0]);
    final outX = _laneX(workArea[1]);
    return Stack(
      children: [
        IgnorePointer(
          child: CustomPaint(
            size: Size(scale.trackWidth, height),
            painter: _WorkAreaPainter(
              inX: inX,
              outX: outX,
              band: t.success,
              dim: t.surface0.withValues(alpha: 0.35),
            ),
          ),
        ),
        _EdgeHandle(
          edgeX: inX,
          height: height,
          scale: scale,
          onFrame: (f) => app.setWorkAreaEdge(compId, f, false),
        ),
        _EdgeHandle(
          edgeX: outX,
          height: height,
          scale: scale,
          onFrame: (f) => app.setWorkAreaEdge(compId, f, true),
        ),
      ],
    );
  }
}

class _WorkAreaPainter extends CustomPainter {
  final double inX, outX;
  final Color band, dim;
  _WorkAreaPainter({
    required this.inX,
    required this.outX,
    required this.band,
    required this.dim,
  });

  @override
  void paint(Canvas canvas, Size size) {
    // Dim the regions outside the work area.
    final dimPaint = Paint()..color = dim;
    if (inX > 0) {
      canvas.drawRect(
          Rect.fromLTRB(0, 0, inX.clamp(0, size.width), size.height), dimPaint);
    }
    if (outX < size.width) {
      canvas.drawRect(
          Rect.fromLTRB(outX.clamp(0, size.width), 0, size.width, size.height),
          dimPaint);
    }
    // The success strip along the top between the edges.
    final l = inX.clamp(0.0, size.width);
    final r = outX.clamp(0.0, size.width);
    if (r > l) {
      canvas.drawRect(Rect.fromLTRB(l, 0, r, 4), Paint()..color = band);
    }
    // Edge brackets.
    final edge = Paint()
      ..color = band
      ..strokeWidth = 1.5;
    for (final x in [inX, outX]) {
      if (x >= -1 && x <= size.width + 1) {
        canvas.drawLine(Offset(x, 0), Offset(x, size.height), edge);
      }
    }
  }

  @override
  bool shouldRepaint(_WorkAreaPainter old) =>
      old.inX != inX || old.outX != outX;
}

/// A draggable work-area edge: a narrow hit strip centred on the edge that
/// slides it to the frame under the pointer (only re-firing when the rounded
/// frame changes, so a drag is one op per frame crossed, not per pixel).
class _EdgeHandle extends StatefulWidget {
  final double edgeX;
  final double height;
  final LaneScale scale;
  final ValueChanged<int> onFrame;
  const _EdgeHandle({
    required this.edgeX,
    required this.height,
    required this.scale,
    required this.onFrame,
  });

  @override
  State<_EdgeHandle> createState() => _EdgeHandleState();
}

class _EdgeHandleState extends State<_EdgeHandle> {
  static const double _w = 12;
  int? _last;

  void _emit(double localX) {
    final laneX = widget.edgeX - _w / 2 + localX;
    final f = widget.scale
        .frameOfX(laneX + widget.scale.trackLeft)
        .round()
        .clamp(0, widget.scale.frameCount);
    if (f != _last) {
      _last = f;
      widget.onFrame(f);
    }
  }

  @override
  Widget build(BuildContext context) {
    return Positioned(
      left: widget.edgeX - _w / 2,
      top: 0,
      width: _w,
      height: widget.height,
      child: MouseRegion(
        cursor: SystemMouseCursors.resizeLeftRight,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onHorizontalDragStart: (_) => _last = null,
          onHorizontalDragUpdate: (d) => _emit(d.localPosition.dx),
          child: const SizedBox.expand(),
        ),
      ),
    );
  }
}

/// The horizontal pan scrollbar under the lane, shown when the view is zoomed
/// past fit: a thumb whose width is the visible fraction, dragged to pan; the
/// ruler and lanes follow through the shared [LaneScale].
class _PanScrollbar extends StatelessWidget {
  static const double height = 12;

  final double outlineWidth;
  final double trackWidth;
  final int frameCount;
  final double zoom;
  final double viewStart;
  final ValueChanged<double> onPan;

  const _PanScrollbar({
    required this.outlineWidth,
    required this.trackWidth,
    required this.frameCount,
    required this.zoom,
    required this.viewStart,
    required this.onPan,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final frames = frameCount < 1 ? 1 : frameCount;
    final visible = frames / zoom;
    final thumbFrac = (visible / frames).clamp(0.0, 1.0);
    final startFrac = (viewStart / frames).clamp(0.0, 1.0 - thumbFrac);
    final thumbW = thumbFrac * trackWidth;

    void onDrag(double dx) {
      // Convert a thumb move in px back to a view-start in frames.
      final start = LaneScale.clampViewStart(
        desired: (startFrac * frames) + dx / trackWidth * frames,
        frameCount: frameCount,
        zoom: zoom,
      );
      onPan(start);
    }

    return SizedBox(
      height: height,
      child: Row(
        children: [
          SizedBox(width: outlineWidth),
          SizedBox(
            width: trackWidth,
            child: Stack(
              children: [
                Positioned.fill(child: Container(color: t.surface1)),
                Positioned(
                  left: startFrac * trackWidth,
                  top: 2,
                  bottom: 2,
                  width: thumbW.clamp(20.0, trackWidth),
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onHorizontalDragUpdate: (d) => onDrag(d.delta.dx),
                    child: Container(
                      decoration: BoxDecoration(
                        color: t.hairlineStrong,
                        borderRadius: BorderRadius.circular(4),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// The bottom bar: zoom (− + and readout), the magnet snap toggle, and the graph
/// lens toggle — the same controls the F0 skeleton carried, kept correct.
class _BottomBar extends StatelessWidget {
  final AppStateStub app;
  const _BottomBar({required this.app});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      height: 24,
      color: t.surface2,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Row(
        children: [
          // The leading cluster scrolls horizontally rather than overflowing
          // when the panel is narrow, so adding the motion-blur master never
          // clips the graph toggle off the end.
          Expanded(
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
          HouseButton(
            frameless: true,
            small: true,
            onPressed: () => app.zoomTimeline(1.4),
            child: Text('+', style: t.bodyPrimary),
          ),
          HouseButton(
            frameless: true,
            small: true,
            onPressed: () => app.zoomTimeline(1 / 1.4),
            child: Text('−', style: t.bodyPrimary),
          ),
          HouseButton(
            frameless: true,
            small: true,
            onPressed: app.zoomTimelineFit,
            child: Text('Fit', style: t.small),
          ),
          const SizedBox(width: 6),
          Text('${(app.timelineZoom * 100).round()}%', style: t.small),
          const SizedBox(width: 10),
          LumitTooltip(
            message: 'Snapping',
            child: HouseButton(
              frameless: true,
              small: true,
              onPressed: () {
                app.snapping = !app.snapping;
                app.setNotice(app.snapping ? 'snapping on' : 'snapping off');
              },
              child: lumitIcon(
                LumitIcon.magnet,
                size: 13,
                color: app.snapping ? t.accent : t.textMuted,
              ),
            ),
          ),
                ],
              ),
            ),
          ),
          LumitTooltip(
            message: 'Graph editor (Shift+F3)',
            child: HouseButton(
              frameless: true,
              small: true,
              onPressed: app.toggleGraphMode,
              child: lumitIcon(
                LumitIcon.graphCurve,
                size: 13,
                color: app.timelineGraphMode ? t.accent : t.textMuted,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// The lane time grid: faint vertical guide lines at the ruler's labelled
/// second ticks, drawn behind the rows when "Show time grid" is on (egui's
/// `TimelineGrid::Time`). Draws nothing when [show] is false.
class _TimeGridPainter extends CustomPainter {
  final LaneScale scale;
  final double fps;
  final bool show;
  final Color line;

  _TimeGridPainter({
    required this.scale,
    required this.fps,
    required this.show,
    required this.line,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (!show || fps <= 0) return;
    final pxPerSecond = scale.pxPerFrame * fps;
    if (pxPerSecond <= 0) return;
    final spec = chooseTicks(pxPerSecond);
    final viewStartSeconds = scale.viewStartFrame / fps;
    final viewEndSeconds = viewStartSeconds + scale.trackWidth / pxPerSecond;
    final durationSeconds = scale.frameCount / fps;
    final paint = Paint()
      ..color = line
      ..strokeWidth = 0.5;
    var s =
        (viewStartSeconds / spec.secondsPerLabel).floor() * spec.secondsPerLabel;
    while (s <= viewEndSeconds && s <= durationSeconds + 1e-6) {
      if (s >= -1e-6) {
        final x = scale.xOfFrame(s * fps);
        if (x >= scale.trackLeft - 0.5 &&
            x <= scale.trackLeft + scale.trackWidth + 0.5) {
          canvas.drawLine(Offset(x, 0), Offset(x, size.height), paint);
        }
      }
      s += spec.secondsPerLabel;
    }
  }

  @override
  bool shouldRepaint(_TimeGridPainter old) =>
      old.show != show ||
      old.fps != fps ||
      old.scale.pxPerFrame != scale.pxPerFrame ||
      old.scale.viewStartFrame != scale.viewStartFrame ||
      old.scale.trackWidth != scale.trackWidth;
}

/// The composition motion-blur master (T9/T22): the comp-wide enable that the
/// per-layer motion-blur switches need. It sits in the Timeline's top row
/// (egui's home for it), at the right of the layer-search box. Toggling flips
/// only the master enable, preserving the comp's shutter angle/phase/samples.
class _MotionBlurMaster extends StatelessWidget {
  final AppStateStub app;
  const _MotionBlurMaster({required this.app});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final comp = app.frontComp;
    final compId = app.frontCompIdResolved;
    if (comp == null || compId == null) return const SizedBox.shrink();
    // Older engines carry no master read-back; default to sensible shutter
    // values so the first enable is well-formed (180° / 0° / 16 samples).
    final mb = comp.motionBlur ??
        const BridgeMotionBlur(
            enabled: false, angle: 180, phase: 0, samples: 16);
    return LumitTooltip(
      message: 'Composition motion blur (master)',
      child: HouseButton(
        key: const ValueKey('mb-master'),
        frameless: true,
        small: true,
        onPressed: () => app.setMotionBlur(
            compId, !mb.enabled, mb.angle, mb.phase, mb.samples),
        child: lumitIcon(
          LumitIcon.motionBlur,
          size: 13,
          color: mb.enabled ? t.accent : t.textMuted,
        ),
      ),
    );
  }
}
