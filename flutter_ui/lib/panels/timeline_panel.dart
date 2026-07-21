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
import '../state/app_state.dart';
import '../widgets/controls.dart';
import 'timeline/comp_tabs.dart';
import 'timeline/group_header.dart';
import 'timeline/lane_host.dart';
import 'timeline/lane_scale.dart';
import 'timeline/lane_selection.dart';
import 'timeline/layer_row.dart';
import 'timeline/property_row.dart';
import 'timeline/property_rows.dart';
import 'timeline/ruler.dart';
import 'timeline/search.dart';

/// The fixed outline-column width (px). Resizable later; F3 pins it at 260 and
/// degrades the switch cluster when the panel is too narrow to hold it.
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

  /// The layer-search query (case-insensitive substring filter).
  final TextEditingController _searchController = TextEditingController();
  final FocusNode _searchFocus = FocusNode();
  String _search = '';

  // --- Lane keyframe selection + drag (TimelineLaneHost) ------------------
  final Set<LaneKeyId> _selectedKeys = {};
  int _dragGrabFrame = 0;
  int _dragDelta = 0;
  bool _dragActive = false;

  AppStateStub get app => widget.app;

  @override
  void dispose() {
    _searchController.dispose();
    _searchFocus.dispose();
    super.dispose();
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
        // The outline never swallows the lane; below ~340 px it shrinks so the
        // lane keeps at least 80 px, and the switch cluster degrades to suit.
        final outlineW =
            _kOutlineWidth.clamp(60.0, (totalW - 80).clamp(60.0, _kOutlineWidth));
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
        final layers = [
          for (final l in widget.comp.layers)
            if (layerMatchesSearch(l.name, _search)) l,
        ];

        final playheadX = scale.xOfFrame(app.previewFrame);
        final showPlayhead =
            playheadX >= trackLeft - 0.5 && playheadX <= trackLeft + trackW + 0.5;

        return Stack(
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
                        child: _SearchField(
                          controller: _searchController,
                          focus: _searchFocus,
                          onChanged: (v) => setState(() => _search = v),
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
                Expanded(
                  child: Listener(
                    onPointerSignal: (e) {
                      if (e is! PointerScrollEvent || !scale.canPan) return;
                      final shift = HardwareKeyboard.instance.isShiftPressed;
                      final dx = e.scrollDelta.dx != 0
                          ? e.scrollDelta.dx
                          : (shift ? e.scrollDelta.dy : 0);
                      if (dx == 0) return;
                      _pan(dx / scale.pxPerFrame, widget.comp.frameCount,
                          app.timelineZoom);
                    },
                    child: SingleChildScrollView(
                      child: Column(
                        children: [
                          for (var i = 0; i < layers.length; i++)
                            ..._layerBlock(layers[i], i, outlineW.toDouble(),
                                scale, fps, markers),
                        ],
                      ),
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
            if (showPlayhead)
              Positioned(
                left: playheadX,
                top: 0,
                bottom: scale.canPan ? _PanScrollbar.height : 0,
                child: IgnorePointer(
                  child: Container(width: 1, color: t.accent),
                ),
              ),
          ],
        );
      },
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
        key: ValueKey(layer.id),
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
          const Spacer(),
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
