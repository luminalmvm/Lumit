// One transform property row inside a layer's open Transform twirl: the outline
// cell (stopwatch, ◄ ◆ ► navigator, name, value readout(s)) on the left and the
// keyframe lane on the right. Ported from the egui `prop_row`/`linked_pair_row`
// (crates/lumit-ui/src/shell/inspector/transform_rows.rs) and the lane glyph
// interaction (inspector/lane.rs), trimmed to the read-back the bridge gives us:
// editing values lives in Effect controls, so the readouts here are display-only.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../bridge/bridge.dart';
import '../../icons/icons.dart';
import '../../state/app_state.dart';
import '../../theme/theme.dart';
import '../../widgets/controls.dart';
import 'key_glyph.dart';
import 'key_nav.dart';
import 'lane_host.dart';
import 'lane_scale.dart';
import 'lane_selection.dart';
import 'layer_row.dart' show kRowHeight;
import 'property_rows.dart';

/// The left indent of a property row (past the layer/group twirl).
const double _kPropIndent = 30;

/// A single transform property row.
class PropertyRow extends StatefulWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  final PropRowSpec spec;
  final double outlineWidth;
  final LaneScale scale;
  final TimelineLaneHost host;

  const PropertyRow({
    super.key,
    required this.app,
    required this.compId,
    required this.layer,
    required this.spec,
    required this.outlineWidth,
    required this.scale,
    required this.host,
  });

  @override
  State<PropertyRow> createState() => _PropertyRowState();
}

class _PropertyRowState extends State<PropertyRow> {
  // The key grabbed at pointer-down, resolved so a drag knows what it moves.
  LaneKeyId? _grabbed;
  double _downLocalX = 0;

  AppStateStub get app => widget.app;
  BridgeLayer get layer => widget.layer;
  PropRowSpec get spec => widget.spec;
  LaneScale get scale => widget.scale;

  List<BridgeKeyframe> get _keys => rowKeys(layer.transform, spec);

  int get _playhead => app.previewFrame;

  double _laneX(int frame) => scale.xOfFrame(frame) - scale.trackLeft;

  /// The key under lane-local [localX], or null. Uses the drawn (drag-offset)
  /// position so a mid-drag grab still lands on the glyph.
  LaneKeyId? _hitKey(double localX) {
    LaneKeyId? best;
    var bestDist = 8.0;
    for (final k in _keys) {
      final id = LaneKeyId(layer.id, spec.primary, k.frame);
      final selected = widget.host.selectedKeys.contains(id);
      final shown = selected && widget.host.keyDragActive
          ? k.frame + widget.host.keyDragDelta
          : k.frame;
      final d = (_laneX(shown) - localX).abs();
      if (d <= bestDist) {
        bestDist = d;
        best = id;
      }
    }
    return best;
  }

  bool get _additive =>
      HardwareKeyboard.instance.isControlPressed ||
      HardwareKeyboard.instance.isShiftPressed ||
      HardwareKeyboard.instance.isMetaPressed;

  void _stopwatch() {
    final frame = _playhead;
    for (final name in spec.props) {
      app.togglePropertyAnimated(widget.compId, layer.id, name, frame);
    }
  }

  void _toggleKey(bool onKey) {
    final frame = _playhead;
    for (final name in spec.props) {
      if (onKey) {
        app.removeKeyframe(widget.compId, layer.id, name, frame);
      } else {
        final v = app.transformValueFor(layer.id, name) ?? 0;
        app.addKeyframe(widget.compId, layer.id, name, frame, v);
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final frames = [for (final k in _keys) k.frame];
    final targets = keyNavTargets(frames, _playhead);
    final animated = rowAnimated(layer.transform, spec);

    return SizedBox(
      height: kRowHeight,
      child: Row(
        children: [
          SizedBox(
            width: widget.outlineWidth,
            child: _outline(t, targets, animated),
          ),
          Expanded(
            child: Listener(
              onPointerDown: (e) => _downLocalX = e.localPosition.dx,
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTapUp: (d) {
                  final hit = _hitKey(d.localPosition.dx);
                  if (hit != null) {
                    widget.host.keyTap(hit, additive: _additive);
                  }
                },
                onSecondaryTapDown: (d) {
                  final hit = _hitKey(d.localPosition.dx);
                  if (hit != null) widget.host.keyRemove(hit);
                },
                onHorizontalDragStart: (_) {
                  final hit = _hitKey(_downLocalX);
                  _grabbed = hit;
                  if (hit == null) return;
                  if (!widget.host.selectedKeys.contains(hit) && !_additive) {
                    widget.host.keyTap(hit, additive: false);
                  }
                  widget.host.keyDragStart(hit, hit.frame);
                },
                onHorizontalDragUpdate: (d) {
                  if (_grabbed == null) return;
                  final f = scale
                      .frameOfX(d.localPosition.dx + scale.trackLeft)
                      .round();
                  widget.host.keyDragTo(f);
                },
                onHorizontalDragEnd: (_) {
                  if (_grabbed != null) {
                    widget.host.keyDragEnd();
                    _grabbed = null;
                  }
                },
                onHorizontalDragCancel: () => _grabbed = null,
                child: CustomPaint(
                  painter: _LanePainter(
                    keys: _keys,
                    layerId: layer.id,
                    property: spec.primary,
                    scale: scale,
                    selected: widget.host.selectedKeys,
                    dragActive: widget.host.keyDragActive,
                    dragDelta: widget.host.keyDragDelta,
                    accent: t.accent,
                    hot: t.textPrimary,
                    outline: t.surface0,
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

  Widget _outline(LumitTheme t, KeyNavTargets targets, bool animated) {
    final readouts = <Widget>[];
    for (final name in spec.props) {
      final v = app.transformValueFor(layer.id, name);
      readouts.add(Text(
        v == null ? '—' : v.toStringAsFixed(1),
        style: t.small.copyWith(color: t.textSecondary),
        maxLines: 1,
        overflow: TextOverflow.clip,
      ));
      if (name != spec.props.last) readouts.add(const SizedBox(width: 4));
    }
    return Container(
      padding: const EdgeInsets.only(left: _kPropIndent, right: 4),
      alignment: Alignment.centerLeft,
      child: Row(
        children: [
          _StopwatchButton(
            key: ValueKey('stopwatch:${layer.id}:${spec.primary}'),
            animated: animated,
            onTap: _stopwatch,
          ),
          const SizedBox(width: 2),
          _NavButton(
            key: ValueKey('nav-prev:${layer.id}:${spec.primary}'),
            icon: LumitIcon.prevKeyframe,
            enabled: targets.prev != null,
            onTap: () => app.goToFrame(targets.prev!),
          ),
          _NavButton(
            key: ValueKey('nav-toggle:${layer.id}:${spec.primary}'),
            icon: targets.onKey ? LumitIcon.keyframeFilled : LumitIcon.keyframe,
            enabled: true,
            accent: targets.onKey,
            onTap: () => _toggleKey(targets.onKey),
          ),
          _NavButton(
            key: ValueKey('nav-next:${layer.id}:${spec.primary}'),
            icon: LumitIcon.nextKeyframe,
            enabled: targets.next != null,
            onTap: () => app.goToFrame(targets.next!),
          ),
          const SizedBox(width: 6),
          Expanded(
            child: Text(
              spec.label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: t.small.copyWith(color: t.textMuted),
            ),
          ),
          const SizedBox(width: 4),
          ...readouts,
        ],
      ),
    );
  }
}

/// The drawn, clickable stopwatch (a filled dot when animated, a ring when
/// not) — egui's `stopwatch_button`, accent when the property is animated.
class _StopwatchButton extends StatelessWidget {
  final bool animated;
  final VoidCallback onTap;
  const _StopwatchButton({super.key, required this.animated, required this.onTap});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: SizedBox(
        width: 15,
        height: kRowHeight,
        child: Center(
          child: lumitIcon(
            LumitIcon.stopwatch,
            size: 12,
            color: animated ? t.accent : t.textMuted,
          ),
        ),
      ),
    );
  }
}

/// One navigator arrow / diamond. Dimmed and inert when [enabled] is false
/// (egui `add_enabled`); accent when it marks a key on the playhead.
class _NavButton extends StatelessWidget {
  final LumitIcon icon;
  final bool enabled;
  final bool accent;
  final VoidCallback onTap;
  const _NavButton({
    super.key,
    required this.icon,
    required this.enabled,
    required this.onTap,
    this.accent = false,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final colour = !enabled
        ? t.textDisabled
        : accent
            ? t.accent
            : t.textMuted;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: enabled ? onTap : null,
      child: SizedBox(
        width: 14,
        height: kRowHeight,
        child: Center(child: lumitIcon(icon, size: 11, color: colour)),
      ),
    );
  }
}

/// The keyframe lane painter: one interpolation-coded glyph per key, offset by
/// the live drag delta when selected and a drag is in flight.
class _LanePainter extends CustomPainter {
  final List<BridgeKeyframe> keys;
  final String layerId;
  final String property;
  final LaneScale scale;
  final Set<LaneKeyId> selected;
  final bool dragActive;
  final int dragDelta;
  final Color accent, hot, outline;

  _LanePainter({
    required this.keys,
    required this.layerId,
    required this.property,
    required this.scale,
    required this.selected,
    required this.dragActive,
    required this.dragDelta,
    required this.accent,
    required this.hot,
    required this.outline,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final cy = size.height / 2;
    canvas.save();
    canvas.clipRect(Offset.zero & size);
    for (final k in keys) {
      final id = LaneKeyId(layerId, property, k.frame);
      final isSel = selected.contains(id);
      final shown = isSel && dragActive ? k.frame + dragDelta : k.frame;
      final x = scale.xOfFrame(shown) - scale.trackLeft;
      if (x < -2 || x > size.width + 2) continue;
      drawKeyGlyph(
        canvas,
        Offset(x, cy),
        keyShapeOf(k),
        fill: isSel && dragActive ? hot : accent,
        outline: outline,
        selected: isSel,
        selectRing: accent,
      );
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(_LanePainter old) =>
      old.keys != keys ||
      old.selected != selected ||
      old.dragActive != dragActive ||
      old.dragDelta != dragDelta ||
      old.scale.pxPerFrame != scale.pxPerFrame ||
      old.scale.viewStartFrame != scale.viewStartFrame;
}
