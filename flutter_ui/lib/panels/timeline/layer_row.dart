// One layer row: the outline cell (index, type glyph + colour tab, name, and the
// switch cluster with the F3 degradation order) on the left, and the lane clip
// bar (with trim/move drags) on the right. Ported from the egui per-layer row
// loop (crates/lumit-ui/src/shell/timeline/panel.rs), simplified to the F3
// surface — keyframe lanes, matte/blend/parent columns and the graph lens stay
// open.

import 'package:flutter/widgets.dart';

import '../../bridge/bridge.dart';
import '../../icons/icons.dart';
import '../../state/app_state.dart';
import '../../widgets/controls.dart';
import 'lane_scale.dart';
import 'layer_menu.dart';
import 'layer_style.dart';
import 'outline_layout.dart';

/// The row pitch (px) — the egui 22 px lane row.
const double kRowHeight = 22;

/// A single layer's row across the outline column and the lane.
class LayerRow extends StatefulWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  final int displayIndex;
  final double outlineWidth;
  final LaneScale scale;
  final double fps;
  final List<int> markers;

  /// Whether this layer's outline twirl is open (its property rows show below).
  final bool open;

  /// Toggle the outline twirl (the body owns the open set).
  final VoidCallback onToggleOpen;

  const LayerRow({
    super.key,
    required this.app,
    required this.compId,
    required this.layer,
    required this.displayIndex,
    required this.outlineWidth,
    required this.scale,
    required this.fps,
    required this.markers,
    required this.open,
    required this.onToggleOpen,
  });

  @override
  State<LayerRow> createState() => _LayerRowState();
}

enum _Mode { move, trimIn, trimOut }

class _Drag {
  final _Mode mode;
  final double grabFrame;
  final int origIn;
  final int origOut;
  int inFrame;
  int outFrame;
  _Drag(this.mode, this.grabFrame, this.origIn, this.origOut)
      : inFrame = origIn,
        outFrame = origOut;
}

class _LayerRowState extends State<LayerRow> {
  _Drag? _drag;

  // The lane-local x of the last pointer-down. The mode hit-test must use this,
  // not the drag-start position: touch slop moves the pointer ~18 px before a
  // horizontal drag is recognised, which would skip past the 6 px edge handles.
  double _downLocalX = 0;

  AppStateStub get app => widget.app;
  BridgeLayer get layer => widget.layer;
  LaneScale get scale => widget.scale;

  int _snap(int frame) => snapFrame(
        frame,
        fps: widget.fps,
        markers: widget.markers,
        snapping: app.snapping,
        pxPerFrame: scale.pxPerFrame,
      );

  void _onDragStart() {
    if (layer.switches.locked) return;
    final localX = _downLocalX;
    final leftX = scale.xOfFrame(layer.inFrame) - scale.trackLeft;
    final rightX = scale.xOfFrame(layer.outFrame) - scale.trackLeft;
    final grab = scale.frameOfX(localX + scale.trackLeft);
    _Mode? mode;
    if ((localX - leftX).abs() <= 6) {
      mode = _Mode.trimIn;
    } else if ((localX - rightX).abs() <= 6) {
      mode = _Mode.trimOut;
    } else if (localX > leftX && localX < rightX) {
      mode = _Mode.move;
    }
    if (mode == null) return;
    app.selectLayer(layer.id);
    setState(() => _drag = _Drag(mode!, grab, layer.inFrame, layer.outFrame));
  }

  void _onDragUpdate(double localX) {
    final drag = _drag;
    if (drag == null) return;
    final f = scale.frameOfX(localX + scale.trackLeft);
    setState(() {
      switch (drag.mode) {
        case _Mode.move:
          final delta = (f - drag.grabFrame).round();
          final len = drag.origOut - drag.origIn;
          final newIn = _snap((drag.origIn + delta).clamp(0, scale.frameCount));
          drag.inFrame = newIn;
          drag.outFrame = newIn + len;
        case _Mode.trimIn:
          drag.inFrame =
              _snap(f.round()).clamp(0, drag.origOut - 1);
          drag.outFrame = drag.origOut;
        case _Mode.trimOut:
          drag.inFrame = drag.origIn;
          drag.outFrame =
              _snap(f.round()).clamp(drag.origIn + 1, scale.frameCount);
      }
    });
  }

  void _onDragEnd() {
    final drag = _drag;
    if (drag == null) return;
    switch (drag.mode) {
      // A pure move is one op: SpanEdit::MoveIn lands the in point on the
      // target frame and shifts out + start_offset with it, length preserved
      // (lumit-core ops.rs). No trim, so no second call is needed.
      case _Mode.move:
        if (drag.inFrame != drag.origIn) {
          app.editLayerSpan(widget.compId, layer.id, 'move_in', drag.inFrame);
        }
      case _Mode.trimIn:
        if (drag.inFrame != drag.origIn) {
          app.editLayerSpan(widget.compId, layer.id, 'trim_in', drag.inFrame);
        }
      case _Mode.trimOut:
        if (drag.outFrame != drag.origOut) {
          app.editLayerSpan(widget.compId, layer.id, 'trim_out', drag.outFrame);
        }
    }
    setState(() => _drag = null);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final style = layerTypeStyle(layer.kind, t);
    final selected = app.selectedLayer == layer.id;
    return SizedBox(
      height: kRowHeight,
      child: Row(
        children: [
          SizedBox(
            width: widget.outlineWidth,
            child: _OutlineCell(
              app: app,
              compId: widget.compId,
              layer: layer,
              displayIndex: widget.displayIndex,
              style: style,
              selected: selected,
              width: widget.outlineWidth,
              open: widget.open,
              onToggleOpen: widget.onToggleOpen,
            ),
          ),
          Expanded(
            child: Listener(
              onPointerDown: (e) => _downLocalX = e.localPosition.dx,
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTapDown: (d) {
                  final leftX = scale.xOfFrame(layer.inFrame) - scale.trackLeft;
                  final rightX =
                      scale.xOfFrame(layer.outFrame) - scale.trackLeft;
                  if (d.localPosition.dx >= leftX &&
                      d.localPosition.dx <= rightX) {
                    app.selectLayer(layer.id);
                  }
                },
                onHorizontalDragStart: (_) => _onDragStart(),
                onHorizontalDragUpdate: (d) => _onDragUpdate(d.localPosition.dx),
                onHorizontalDragEnd: (_) => _onDragEnd(),
                onHorizontalDragCancel: () => setState(() => _drag = null),
                child: CustomPaint(
                  painter: _LaneBarPainter(
                    inFrame: _drag?.inFrame ?? layer.inFrame,
                    outFrame: _drag?.outFrame ?? layer.outFrame,
                    scale: scale,
                    typeColour: style.colour,
                    fill: t.surface3,
                    edge: t.hairlineStrong,
                    accent: t.accent,
                    selected: selected,
                  ),
                  child: const SizedBox.expand(),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// The outline (left) cell: colour tab, index, glyph, name, switch cluster —
/// with the switch cluster degrading as [width] shrinks.
class _OutlineCell extends StatelessWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  final int displayIndex;
  final ({LumitIcon icon, Color colour}) style;
  final bool selected;
  final double width;
  final bool open;
  final VoidCallback onToggleOpen;

  const _OutlineCell({
    required this.app,
    required this.compId,
    required this.layer,
    required this.displayIndex,
    required this.style,
    required this.selected,
    required this.width,
    required this.open,
    required this.onToggleOpen,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final canAudio = layerCanCarryAudio(layer.kind);
    final isPrecomp = layer.kind == BridgeLayerKind.precomp;
    final cols = chooseColumns(width, canAudio: canAudio, isPrecomp: isPrecomp);
    final sw = layer.switches;
    final dimmed = !sw.visible;
    final nameColour = dimmed ? t.textMuted : t.textSecondary;

    Widget switchIcon(
      LumitIcon icon,
      bool on,
      String field,
      Color onColour, {
      LumitIcon? offIcon,
    }) =>
        _SwitchButton(
          key: ValueKey('sw:${layer.id}:$field'),
          icon: on ? icon : (offIcon ?? icon),
          colour: on ? onColour : t.textMuted,
          onTap: () =>
              app.setLayerSwitch(compId, layer.id, field, !_switchValue(field)),
        );

    final cluster = <Widget>[
      if (cols.eye)
        switchIcon(LumitIcon.eye, sw.visible, 'visible', t.textSecondary,
            offIcon: LumitIcon.eyeClosed),
      if (cols.speaker)
        switchIcon(LumitIcon.audio, sw.audible, 'audible', t.textSecondary,
            offIcon: LumitIcon.mute),
      if (cols.solo)
        _SoloButton(
          key: ValueKey('sw:${layer.id}:solo'),
          app: app,
          compId: compId,
          layer: layer,
        ),
      if (cols.lock)
        switchIcon(LumitIcon.lock, sw.locked, 'locked', t.accent,
            offIcon: LumitIcon.unlock),
      if (cols.fx) switchIcon(LumitIcon.fx, sw.fx, 'fx', t.accent),
      if (cols.motionBlur)
        switchIcon(LumitIcon.motionBlur, sw.motionBlur, 'motion_blur', t.accent),
      if (cols.threeD)
        switchIcon(LumitIcon.cube3d, sw.threeD, 'three_d', t.accent),
      if (cols.collapse)
        switchIcon(LumitIcon.collapse, sw.collapse, 'collapse', t.accent),
    ];

    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: () => app.selectLayer(layer.id),
      onSecondaryTapDown: (d) => showLayerContextMenu(
        context: context,
        app: app,
        compId: compId,
        layer: layer,
        position: d.globalPosition,
      ),
      child: Container(
        color: selected ? t.surface2 : null,
        child: Row(
          children: [
            // 3 px left-edge colour tab in the layer-type colour.
            Container(width: 3, color: style.colour),
            // The disclosure twirl: open reveals the layer's property rows.
            GestureDetector(
              key: ValueKey('twirl:${layer.id}'),
              behavior: HitTestBehavior.opaque,
              onTap: onToggleOpen,
              child: SizedBox(
                width: 15,
                height: kRowHeight,
                child: Center(
                  child: lumitIcon(
                    open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                    size: 11,
                    color: t.textMuted,
                  ),
                ),
              ),
            ),
            const SizedBox(width: 1),
            if (cols.index)
              Padding(
                padding: const EdgeInsets.only(right: 4),
                child: Text('${displayIndex + 1}', style: t.small),
              ),
            lumitIcon(style.icon, size: 13, color: style.colour),
            const SizedBox(width: 5),
            Expanded(
              child: Text(
                layer.name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: t.small.copyWith(color: nameColour),
              ),
            ),
            for (final w in cluster)
              Padding(padding: const EdgeInsets.only(left: 1), child: w),
            const SizedBox(width: 4),
          ],
        ),
      ),
    );
  }

  bool _switchValue(String field) => switch (field) {
        'visible' => layer.switches.visible,
        'audible' => layer.switches.audible,
        'locked' => layer.switches.locked,
        'fx' => layer.switches.fx,
        'motion_blur' => layer.switches.motionBlur,
        'three_d' => layer.switches.threeD,
        'collapse' => layer.switches.collapse,
        'solo' => layer.switches.solo,
        _ => false,
      };
}

/// A 16 px tappable switch glyph.
class _SwitchButton extends StatelessWidget {
  final LumitIcon icon;
  final Color colour;
  final VoidCallback onTap;
  const _SwitchButton({
    super.key,
    required this.icon,
    required this.colour,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: SizedBox(
          width: 16,
          height: kRowHeight,
          child: Center(child: lumitIcon(icon, size: 12, color: colour)),
        ),
      );
}

/// Solo has no Iconoir glyph; the egui frontend labels its column "S", so the
/// switch is a themed "S" that lights accent when soloed.
class _SoloButton extends StatelessWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  const _SoloButton({
    super.key,
    required this.app,
    required this.compId,
    required this.layer,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final on = layer.switches.solo;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: () => app.setLayerSwitch(compId, layer.id, 'solo', !on),
      child: SizedBox(
        width: 16,
        height: kRowHeight,
        child: Center(
          child: Text(
            'S',
            style: t.small.copyWith(
              color: on ? t.accent : t.textMuted,
              fontWeight: FontWeight.w600,
            ),
          ),
        ),
      ),
    );
  }
}

/// The clip bar: a tonal wash of the layer-type colour over the neutral fill,
/// a 3 px type tab on the left edge, a hairline edge (accent when selected).
class _LaneBarPainter extends CustomPainter {
  final int inFrame;
  final int outFrame;
  final LaneScale scale;
  final Color typeColour, fill, edge, accent;
  final bool selected;

  _LaneBarPainter({
    required this.inFrame,
    required this.outFrame,
    required this.scale,
    required this.typeColour,
    required this.fill,
    required this.edge,
    required this.accent,
    required this.selected,
  });

  double _lx(num frame) => scale.xOfFrame(frame) - scale.trackLeft;

  @override
  void paint(Canvas canvas, Size size) {
    final left = _lx(inFrame);
    final right = _lx(outFrame);
    if (right <= 0 || left >= size.width) return;
    final rect = Rect.fromLTRB(
      left,
      2,
      right,
      size.height - 2,
    );
    final rrect = RRect.fromRectAndRadius(rect, const Radius.circular(3));
    canvas.save();
    canvas.clipRect(Offset.zero & size);
    canvas.drawRRect(rrect, Paint()..color = fill);
    canvas.drawRRect(
      rrect,
      Paint()..color = typeColour.withValues(alpha: 0.13),
    );
    // 3 px type tab on the left edge.
    canvas.drawRect(
      Rect.fromLTRB(rect.left, rect.top, rect.left + 3, rect.bottom),
      Paint()..color = typeColour,
    );
    canvas.drawRRect(
      rrect,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = selected ? 1.5 : 1
        ..color = selected ? accent : edge,
    );
    canvas.restore();
  }

  @override
  bool shouldRepaint(_LaneBarPainter old) =>
      old.inFrame != inFrame ||
      old.outFrame != outFrame ||
      old.selected != selected ||
      old.scale.pxPerFrame != scale.pxPerFrame ||
      old.scale.viewStartFrame != scale.viewStartFrame;
}
