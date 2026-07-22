// The Effect controls panel (phase F4): the Transform property rows and the
// effect stack for the selected layer of the front composition, in the
// settings-card style.
//
// Snapshot v3 now carries current transform *values* (read-back), so the value
// boxes seed from `app.transformValueFor` (read-back first, session-edit
// fallback) rather than the old em-dash placeholder. Each transform row also
// carries the stopwatch (accent when animated) and the ◄ ◆ ► keyframe
// navigator, ported from the egui `keyframe_nav.rs` note-2.4 behaviour: ◄/►
// jump the playhead to the previous/next key, and ◆ adds a key at the playhead
// (or removes the one already there). Below Transform, one card per effect on
// the layer, with its parameter rows edited by kind.
//
// Every commit routes through the matching `app` op (setTransform,
// togglePropertyAnimated, addKeyframe/removeKeyframe, setEffectEnabled,
// removeEffect, setEffectParamScalar/_colour); each is one undo step.

import 'package:flutter/widgets.dart';

import '../bridge/bridge.dart';
import '../icons/icons.dart';
import '../state/app_state.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';

/// Fixed width of a value cell so the axis boxes line up down the group.
const double _cellWidth = 60.0;

/// The stable ids of the three-colour channel picker (K-143): an effect that
/// declares these three Colour params shows one grouped swatch row rather than
/// three separate colour rows.
const List<String> _channelColourIds = [
  'channel_colour_1',
  'channel_colour_2',
  'channel_colour_3',
];

class EffectControlsPanel extends StatelessWidget {
  final AppStateStub app;
  const EffectControlsPanel({super.key, required this.app});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ListenableBuilder(
      listenable: app,
      builder: (context, _) {
        final comp = app.frontComp;
        final compId = app.frontCompIdResolved;
        final selectedId = app.selectedLayer;
        BridgeLayer? layer;
        if (comp != null && selectedId != null) {
          for (final l in comp.layers) {
            if (l.id == selectedId) {
              layer = l;
              break;
            }
          }
        }
        if (layer == null || compId == null) {
          return Center(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 240),
              child: Text(
                'Select a layer to edit its transform and effects here.',
                style: t.small,
                textAlign: TextAlign.center,
              ),
            ),
          );
        }
        final resolvedLayer = layer;
        // The whole panel is a drop target for an effect dragged from the
        // Effects & presets panel — dropping applies it to this shown layer
        // through `addEffect` (the drag-an-effect-onto-a-layer gesture, docs/07
        // §7). The Timeline-row drop target awaits the timeline agent's seam.
        return DragTarget<EffectDragData>(
          onAcceptWithDetails: (details) =>
              app.addEffect(compId, resolvedLayer.id, details.data.effectName),
          builder: (context, candidate, rejected) => Container(
            decoration: candidate.isNotEmpty
                ? BoxDecoration(border: Border.all(color: t.accent, width: 1.5))
                : null,
            child: ListView(
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
              children: [
                _LayerTitle(layer: resolvedLayer),
                const SizedBox(height: 6),
                _TransformGroup(app: app, compId: compId, layer: resolvedLayer),
                // The kind-specific property group (Text / Solid / Camera),
                // between Transform and the effect stack — mirroring where the
                // egui inspector puts a layer's asset properties.
                ..._assetGroup(app, compId, resolvedLayer),
                const SizedBox(height: 10),
                _EffectStack(app: app, compId: compId, layer: resolvedLayer),
              ],
            ),
          ),
        );
      },
    );
  }
}

/// The selected layer's title line: type glyph + name.
class _LayerTitle extends StatelessWidget {
  final BridgeLayer layer;
  const _LayerTitle({required this.layer});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final (icon, tint) = _layerStyle(layer.kind, t);
    return Row(
      children: [
        lumitIcon(icon, size: 15, color: tint),
        const SizedBox(width: 6),
        Expanded(
          child: Text(layer.name, style: t.bodyPrimary, overflow: TextOverflow.ellipsis),
        ),
      ],
    );
  }
}

/// A titled surface holding a column of rows (Transform group; effect cards).
class _Card extends StatelessWidget {
  final String? title;
  final List<Widget> rows;
  const _Card({this.title, required this.rows});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final round = t.shape == ThemeShape.round;
    final surface = Container(
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
      decoration: BoxDecoration(
        color: t.surface2,
        borderRadius: round ? BorderRadius.circular(t.tokens.cardRadius) : null,
        border: round ? null : Border.all(color: t.hairline),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (var i = 0; i < rows.length; i++) ...[
            if (i > 0) Container(height: 1, color: t.hairline),
            rows[i],
          ],
        ],
      ),
    );
    if (title == null) return surface;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(title!, style: t.small),
        const SizedBox(height: 4),
        surface,
      ],
    );
  }
}

/// The kind-specific asset property group for the selected layer: a Text group
/// for text layers, a Solid group for solids, a Camera group for cameras.
/// Empty for every other kind. Values seed from the snapshot read-back (bridge
/// v0.9 carries text content/size/fill, a solid's size and a camera's zoom;
/// `app.textContentFor`/`solidSizeFor`/`cameraZoomFor` prefer it), falling back
/// to the session-edit maps only on an older library. Each group carries a
/// layer-keyed [ValueKey] so switching the selected layer re-seeds its editors.
List<Widget> _assetGroup(AppStateStub app, String compId, BridgeLayer layer) {
  final Widget? group = switch (layer.kind) {
    BridgeLayerKind.text => _TextGroup(
        key: ValueKey<String>('text-group-${layer.id}'),
        app: app,
        compId: compId,
        layer: layer),
    BridgeLayerKind.solid =>
      _SolidGroup(app: app, compId: compId, layer: layer),
    BridgeLayerKind.camera =>
      _CameraGroup(app: app, compId: compId, layer: layer),
    _ => null,
  };
  if (group == null) return const [];
  return [const SizedBox(height: 10), group];
}

/// The Text group: a multi-line content editor, a size box and a fill swatch,
/// committed together through `setTextContent`. The content seeds from the
/// snapshot read-back (`layer.text`, bridge v0.9), or the session edit / default
/// on an older library.
class _TextGroup extends StatefulWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  const _TextGroup({
    super.key,
    required this.app,
    required this.compId,
    required this.layer,
  });

  @override
  State<_TextGroup> createState() => _TextGroupState();
}

class _TextGroupState extends State<_TextGroup> {
  late final TextEditingController _content;
  final FocusNode _focus = FocusNode();

  TextContent get _held => widget.app.textContentFor(widget.layer.id);

  @override
  void initState() {
    super.initState();
    _content = TextEditingController(text: _held.text);
    // Commit the content when the multi-line editor loses focus.
    _focus.addListener(() {
      if (!_focus.hasFocus) _commit(text: _content.text);
    });
  }

  @override
  void dispose() {
    _content.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _commit({String? text, double? size, List<double>? rgba}) {
    final content = _held.copyWith(text: text, size: size, rgba: rgba);
    widget.app.commitTextContent(widget.compId, widget.layer.id, content);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final held = _held;
    return _Card(
      title: 'Text',
      rows: [
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 6),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text('Content', style: t.small.copyWith(color: t.textSecondary)),
              const SizedBox(height: 4),
              LumitTooltip(
                message: 'The text this layer shows (multi-line)',
                child: Container(
                  constraints: const BoxConstraints(minHeight: 44),
                  padding:
                      const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
                  decoration: BoxDecoration(
                    color: t.surface0,
                    borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                    border: Border.all(
                        color: _focus.hasFocus ? t.accent : t.hairline),
                  ),
                  child: EditableText(
                    key: const ValueKey('text-content'),
                    controller: _content,
                    focusNode: _focus,
                    style: t.bodyPrimary,
                    maxLines: null,
                    cursorColor: t.accent,
                    backgroundCursorColor: t.surface2,
                    selectionColor: t.accent.withValues(alpha: 0.5),
                  ),
                ),
              ),
            ],
          ),
        ),
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: Row(
            children: [
              Expanded(child: Text('Size', style: t.bodyPrimary)),
              const SizedBox(width: 12),
              SizedBox(
                width: _cellWidth,
                child: DragValueField(
                  key: const ValueKey('text-size'),
                  value: held.size,
                  min: 1,
                  max: 2000,
                  speed: 0.5,
                  decimals: 0,
                  suffix: ' pt',
                  // The default point size (TextContent.initial.size).
                  resetTo: 72,
                  onChanged: (v) => _commit(size: v.toDouble()),
                ),
              ),
            ],
          ),
        ),
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: Row(
            children: [
              Expanded(child: Text('Fill', style: t.bodyPrimary)),
              const SizedBox(width: 12),
              _ColourSwatch(
                key: const ValueKey('text-fill'),
                rgba: held.rgba,
                onPicked: (r, g, b, a) => _commit(rgba: [r, g, b, a]),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

/// The Solid group: a colour swatch (seeding from the snapshot's `layer.colour`)
/// and a width×height size, committed together through `setSolid`. The size
/// seeds from the snapshot read-back (`layer.solidSize`, bridge v0.9), or the
/// session edit / comp size on an older library.
class _SolidGroup extends StatelessWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  const _SolidGroup({
    required this.app,
    required this.compId,
    required this.layer,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final rgba = app.solidColourFor(layer);
    final size = app.solidSizeFor(layer.id);
    return _Card(
      title: 'Solid',
      rows: [
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: Row(
            children: [
              Expanded(child: Text('Colour', style: t.bodyPrimary)),
              const SizedBox(width: 12),
              _ColourSwatch(
                key: const ValueKey('solid-colour'),
                rgba: rgba,
                onPicked: (r, g, b, a) =>
                    app.commitSolid(compId, layer.id, [r, g, b, a], size),
              ),
            ],
          ),
        ),
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: Row(
            children: [
              Expanded(child: Text('Size', style: t.bodyPrimary)),
              const SizedBox(width: 12),
              SizedBox(
                width: _cellWidth,
                child: DragValueField(
                  key: const ValueKey('solid-width'),
                  value: size.width,
                  min: 1,
                  max: 16384,
                  speed: 2,
                  onChanged: (v) => app.commitSolid(compId, layer.id, rgba,
                      SolidSize(v.round(), size.height)),
                ),
              ),
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 6),
                child: Text('×', style: t.small),
              ),
              SizedBox(
                width: _cellWidth,
                child: DragValueField(
                  key: const ValueKey('solid-height'),
                  value: size.height,
                  min: 1,
                  max: 16384,
                  speed: 2,
                  onChanged: (v) => app.commitSolid(compId, layer.id, rgba,
                      SolidSize(size.width, v.round())),
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

/// The Camera group: a zoom box, committed through `setCameraZoom`. The zoom
/// seeds from the snapshot read-back (`layer.cameraZoom`, bridge v0.9), or the
/// session edit / comp width on an older library.
class _CameraGroup extends StatelessWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  const _CameraGroup({
    required this.app,
    required this.compId,
    required this.layer,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final zoom = app.cameraZoomFor(layer.id);
    return _Card(
      title: 'Camera',
      rows: [
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 4),
          child: Row(
            children: [
              Expanded(child: Text('Zoom', style: t.bodyPrimary)),
              const SizedBox(width: 12),
              SizedBox(
                width: _cellWidth,
                child: DragValueField(
                  key: const ValueKey('camera-zoom'),
                  value: zoom,
                  min: 1,
                  max: 100000,
                  speed: 1,
                  decimals: 0,
                  suffix: ' px',
                  onChanged: (v) =>
                      app.commitCameraZoom(compId, layer.id, v.toDouble()),
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}

/// The Transform group card: a titled surface holding the property rows.
class _TransformGroup extends StatelessWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  const _TransformGroup({
    required this.app,
    required this.compId,
    required this.layer,
  });

  @override
  Widget build(BuildContext context) {
    final threeD = layer.switches.threeD || layer.kind == BridgeLayerKind.camera;

    final rows = <Widget>[
      _TransformRow(
        app: app,
        compId: compId,
        layer: layer,
        label: 'Anchor point',
        axes: const [_AxisSpec('anchor_x'), _AxisSpec('anchor_y')],
      ),
      _TransformRow(
        app: app,
        compId: compId,
        layer: layer,
        label: 'Position',
        axes: [
          const _AxisSpec('position_x'),
          const _AxisSpec('position_y'),
          if (threeD) const _AxisSpec('position_z'),
        ],
      ),
      _TransformRow(
        app: app,
        compId: compId,
        layer: layer,
        label: 'Scale',
        linkable: true,
        axes: const [
          _AxisSpec('scale_x', seed: 100, suffix: '%'),
          _AxisSpec('scale_y', seed: 100, suffix: '%'),
        ],
      ),
      _TransformRow(
        app: app,
        compId: compId,
        layer: layer,
        label: 'Rotation',
        axes: const [_AxisSpec('rotation', suffix: '°', speed: 0.5)],
      ),
      _TransformRow(
        app: app,
        compId: compId,
        layer: layer,
        label: 'Opacity',
        axes: const [
          _AxisSpec('opacity',
              seed: 100, suffix: '%', min: 0, max: 100, decimals: 0, speed: 0.5),
        ],
      ),
      if (threeD) ...[
        _TransformRow(
          app: app,
          compId: compId,
          layer: layer,
          label: 'Rotation x',
          axes: const [_AxisSpec('rotation_x', suffix: '°', speed: 0.5)],
        ),
        _TransformRow(
          app: app,
          compId: compId,
          layer: layer,
          label: 'Rotation y',
          axes: const [_AxisSpec('rotation_y', suffix: '°', speed: 0.5)],
        ),
      ],
    ];

    return _Card(title: 'Transform', rows: rows);
  }
}

/// One transform property's axis: its snake_case name, plus display hints.
class _AxisSpec {
  final String prop;
  final num seed;
  final String? suffix;
  final num min;
  final num max;
  final int decimals;
  final double speed;
  const _AxisSpec(
    this.prop, {
    this.seed = 0,
    this.suffix,
    this.min = -100000,
    this.max = 100000,
    this.decimals = 1,
    this.speed = 1,
  });
}

/// A transform property row: the stopwatch and (when animated) the ◄ ◆ ►
/// navigator on the left, the label, then one value cell per axis. Reads
/// current values from `app.transformValueFor` (read-back first, session-edit
/// fallback). Multi-axis rows (Anchor, Position, Scale) drive their stopwatch
/// and navigator across every axis at once; because the bridge keyframe ops are
/// per-property (there is no batch op), a linked add/remove issues one op per
/// axis (so it is more than one undo step — noted).
class _TransformRow extends StatefulWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  final String label;
  final List<_AxisSpec> axes;
  final bool linkable;
  const _TransformRow({
    required this.app,
    required this.compId,
    required this.layer,
    required this.label,
    required this.axes,
    this.linkable = false,
  });

  @override
  State<_TransformRow> createState() => _TransformRowState();
}

class _TransformRowState extends State<_TransformRow> {
  bool _linked = true;

  AppStateStub get _app => widget.app;
  String get _layerId => widget.layer.id;

  BridgeTransformProperty? _propInfo(String prop) =>
      widget.layer.transform?[prop];

  double _valueOf(_AxisSpec a) =>
      _app.transformValueFor(_layerId, a.prop) ?? a.seed.toDouble();

  /// The union of all this row's axes' keyframe frames, sorted.
  List<int> _keyFrames() {
    final frames = <int>{};
    for (final a in widget.axes) {
      final info = _propInfo(a.prop);
      if (info != null) {
        for (final k in info.keys) {
          frames.add(k.frame);
        }
      }
    }
    final list = frames.toList()..sort();
    return list;
  }

  bool get _animated =>
      widget.axes.any((a) => _propInfo(a.prop)?.animated ?? false);

  void _toggleStopwatch() {
    final frame = _app.previewFrame;
    for (final a in widget.axes) {
      _app.togglePropertyAnimated(widget.compId, _layerId, a.prop, frame);
    }
  }

  void _toggleKeyframe(bool onKey) {
    final frame = _app.previewFrame;
    for (final a in widget.axes) {
      if (onKey) {
        _app.removeKeyframe(widget.compId, _layerId, a.prop, frame);
      } else {
        _app.addKeyframe(widget.compId, _layerId, a.prop, frame, _valueOf(a));
      }
    }
  }

  /// Commit an edit to [a]. When the row is a linked Scale, editing one axis
  /// preserves the x:y ratio across the pair (now that current values read
  /// back); a zero base falls back to setting both axes equal.
  void _commit(_AxisSpec a, double value) {
    if (widget.linkable && _linked && widget.axes.length == 2) {
      final other = widget.axes.firstWhere((x) => x.prop != a.prop);
      final base = _valueOf(a);
      final otherBase = _valueOf(other);
      double otherValue;
      if (base.abs() < 1e-9) {
        otherValue = value; // no ratio to preserve — match the edited axis
      } else {
        otherValue = otherBase * (value / base);
      }
      _app.setTransform(widget.compId, _layerId, a.prop, value);
      _app.setTransform(widget.compId, _layerId, other.prop, otherValue);
      return;
    }
    _app.setTransform(widget.compId, _layerId, a.prop, value);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final animated = _animated;
    final frames = _keyFrames();

    final cells = <Widget>[];
    for (var i = 0; i < widget.axes.length; i++) {
      final a = widget.axes[i];
      if (i > 0) cells.add(const SizedBox(width: 4));
      if (widget.linkable && i == 1) {
        cells.add(_LinkToggle(
          linked: _linked,
          onTap: () => setState(() => _linked = !_linked),
        ));
        cells.add(const SizedBox(width: 4));
      }
      cells.add(SizedBox(
        width: _cellWidth,
        child: DragValueField(
          key: ValueKey<String>('axis-${a.prop}'),
          value: _valueOf(a),
          min: a.min,
          max: a.max,
          speed: a.speed,
          decimals: a.decimals,
          suffix: a.suffix,
          // The property's model default (0 for anchor/position/rotation, 100
          // for scale/opacity) — the right-click Reset target.
          resetTo: a.seed,
          onChanged: (v) => _commit(a, v.toDouble()),
        ),
      ));
    }

    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        children: [
          _StopwatchButton(
            key: ValueKey<String>('stopwatch-${widget.label}'),
            animated: animated,
            onTap: _toggleStopwatch,
          ),
          const SizedBox(width: 4),
          if (animated)
            // The ◄ ◆ ► navigator (and the diamond's add-vs-remove sense)
            // tracks where the playhead sits relative to this property's keys,
            // so it alone watches the fine-grained playhead notifier — the
            // panel above stays on the app notifier (perf pass).
            ValueListenableBuilder<int>(
              valueListenable: _app.playheadFrame,
              builder: (context, frame, _) {
                final prev =
                    frames.where((f) => f < frame).fold<int?>(null, (m, f) => f);
                final onKey = frames.contains(frame);
                final next = frames
                    .where((f) => f > frame)
                    .fold<int?>(null, (m, f) => m ?? f);
                return _KeyframeNavigator(
                  onKey: onKey,
                  onPrev: prev == null ? null : () => _app.goToFrame(prev),
                  onToggle: () => _toggleKeyframe(onKey),
                  onNext: next == null ? null : () => _app.goToFrame(next),
                );
              },
            )
          else
            const SizedBox(width: 4),
          const SizedBox(width: 6),
          Expanded(child: Text(widget.label, style: t.bodyPrimary)),
          const SizedBox(width: 12),
          ...cells,
        ],
      ),
    );
  }
}

/// The scale link toggle: accent link when linked, muted broken link when not.
class _LinkToggle extends StatelessWidget {
  final bool linked;
  final VoidCallback onTap;
  const _LinkToggle({required this.linked, required this.onTap});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: linked
          ? 'Unlink scale (edit x and y separately)'
          : 'Link scale (edit both axes together)',
      child: HouseButton(
        frameless: true,
        small: true,
        onPressed: onTap,
        child: lumitIcon(
          linked ? LumitIcon.link : LumitIcon.unlink,
          size: 13,
          color: linked ? t.accent : t.textMuted,
        ),
      ),
    );
  }
}

/// The stopwatch: accent when the property is animated, muted otherwise.
/// Clicking toggles animation at the playhead (`togglePropertyAnimated`).
class _StopwatchButton extends StatelessWidget {
  final bool animated;
  final VoidCallback onTap;
  const _StopwatchButton({super.key, required this.animated, required this.onTap});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: animated
          ? 'Remove animation (freeze the current value)'
          : 'Animate: keyframe at the playhead',
      child: HouseButton(
        frameless: true,
        small: true,
        onPressed: onTap,
        child: lumitIcon(
          LumitIcon.stopwatch,
          size: 13,
          color: animated ? t.accent : t.textMuted,
        ),
      ),
    );
  }
}

/// The shared ◄ ◆ ► keyframe navigator (egui `keyframe_navigator`, note 2.4):
/// ◄ jumps to the previous key, the diamond adds a key at the playhead (a
/// filled ◆ when one is already there — clicking then removes it), ► jumps to
/// the next. Prev/next dim when there is no key to jump to.
class _KeyframeNavigator extends StatelessWidget {
  final bool onKey;
  final VoidCallback? onPrev;
  final VoidCallback onToggle;
  final VoidCallback? onNext;
  const _KeyframeNavigator({
    required this.onKey,
    required this.onPrev,
    required this.onToggle,
    required this.onNext,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        LumitTooltip(
          message: 'Previous keyframe',
          child: HouseButton(
            frameless: true,
            small: true,
            onPressed: onPrev,
            child: lumitIcon(
              LumitIcon.prevKeyframe,
              size: 11,
              color: onPrev == null ? t.textDisabled : t.textSecondary,
            ),
          ),
        ),
        LumitTooltip(
          message: onKey ? 'Remove keyframe here' : 'Add keyframe here',
          child: HouseButton(
            key: const ValueKey('kf-diamond'),
            frameless: true,
            small: true,
            onPressed: onToggle,
            child: lumitIcon(
              onKey ? LumitIcon.keyframeFilled : LumitIcon.keyframe,
              size: 11,
              color: onKey ? t.accent : t.textSecondary,
            ),
          ),
        ),
        LumitTooltip(
          message: 'Next keyframe',
          child: HouseButton(
            frameless: true,
            small: true,
            onPressed: onNext,
            child: lumitIcon(
              LumitIcon.nextKeyframe,
              size: 11,
              color: onNext == null ? t.textDisabled : t.textSecondary,
            ),
          ),
        ),
      ],
    );
  }
}

/// The stopwatch + ◄ ◆ ► navigator for one animatable effect parameter, driving
/// the bridge v0.9 effect-param keyframe ops. [channels] is the param's channel
/// list (scalar → `[0]`, point → `[0,1]`, colour → `[0..3]`), each with a getter
/// for the value a new key takes; every op is issued per channel, so a
/// multi-channel key is more than one undo step (mirrors the linked transform
/// rows — noted).
class _FxKeyframeControls extends StatelessWidget {
  final AppStateStub app;
  final String compId;
  final String layerId;
  final String effectId;
  final BridgeEffectParam param;
  final List<(int, double Function())> channels;
  const _FxKeyframeControls({
    super.key,
    required this.app,
    required this.compId,
    required this.layerId,
    required this.effectId,
    required this.param,
    required this.channels,
  });

  /// The union of this param's key frames across every channel, sorted.
  List<int> _frames() {
    final frames = <int>{};
    for (final k in param.keys) {
      frames.add(k.frame);
    }
    for (final list in param.channelKeys.values) {
      for (final k in list) {
        frames.add(k.frame);
      }
    }
    final list = frames.toList()..sort();
    return list;
  }

  void _toggleStopwatch() {
    final frame = app.previewFrame;
    for (final (ch, _) in channels) {
      app.toggleEffectParamAnimated(
          compId, layerId, effectId, param.name, ch, frame);
    }
  }

  void _toggleKeyframe(bool onKey) {
    final frame = app.previewFrame;
    for (final (ch, valueOf) in channels) {
      if (onKey) {
        app.removeEffectParamKeyframe(
            compId, layerId, effectId, param.name, ch, frame);
      } else {
        app.addEffectParamKeyframe(
            compId, layerId, effectId, param.name, ch, frame, valueOf());
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final animated = param.animated;
    final frames = _frames();
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        _StopwatchButton(
          key: ValueKey<String>('fxstopwatch-$effectId-${param.name}'),
          animated: animated,
          onTap: _toggleStopwatch,
        ),
        if (animated)
          ValueListenableBuilder<int>(
            valueListenable: app.playheadFrame,
            builder: (context, frame, _) {
              final prev =
                  frames.where((f) => f < frame).fold<int?>(null, (m, f) => f);
              final onKey = frames.contains(frame);
              final next = frames
                  .where((f) => f > frame)
                  .fold<int?>(null, (m, f) => m ?? f);
              return _KeyframeNavigator(
                onKey: onKey,
                onPrev: prev == null ? null : () => app.goToFrame(prev),
                onToggle: () => _toggleKeyframe(onKey),
                onNext: next == null ? null : () => app.goToFrame(next),
              );
            },
          )
        else
          const SizedBox(width: 4),
      ],
    );
  }
}

/// The effect stack: one card per effect on the layer, in stack order.
class _EffectStack extends StatelessWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  const _EffectStack({
    required this.app,
    required this.compId,
    required this.layer,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    if (layer.effects.isEmpty) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text('Effects', style: t.small),
          const SizedBox(height: 4),
          Text(
            'No effects on this layer. Add one from the Effects & presets panel.',
            style: t.small.copyWith(color: t.textMuted),
          ),
        ],
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text('Effects', style: t.small),
        const SizedBox(height: 4),
        for (final e in layer.effects) ...[
          _EffectCard(app: app, compId: compId, layer: layer, effect: e),
          const SizedBox(height: 8),
        ],
      ],
    );
  }
}

/// One effect: a title bar (enabled checkbox, label, remove) over its parameter
/// rows.
class _EffectCard extends StatelessWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  final BridgeEffect effect;
  const _EffectCard({
    required this.app,
    required this.compId,
    required this.layer,
    required this.effect,
  });

  @override
  Widget build(BuildContext context) {
    final rows = <Widget>[
      _EffectTitleRow(app: app, compId: compId, layer: layer, effect: effect),
    ];
    // Fold the three-colour channel picker into one row when the effect
    // declares the channel_colour_1..3 Colour params (K-143); otherwise render
    // each parameter by its kind.
    final hasChannelPicker = effect.params.any((p) =>
        p.name == _channelColourIds[0] && p.kind == 'colour');
    for (final p in effect.params) {
      if (hasChannelPicker &&
          (p.name == _channelColourIds[1] || p.name == _channelColourIds[2])) {
        continue; // folded into the channel-picker row
      }
      if (hasChannelPicker && p.name == _channelColourIds[0]) {
        rows.add(_ChannelPickerRow(
            app: app, compId: compId, layer: layer, effect: effect));
        continue;
      }
      rows.add(_EffectParamRow(
          app: app, compId: compId, layer: layer, effect: effect, param: p));
    }
    return _Card(rows: rows);
  }
}

class _EffectTitleRow extends StatelessWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  final BridgeEffect effect;
  const _EffectTitleRow({
    required this.app,
    required this.compId,
    required this.layer,
    required this.effect,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final label = _effectLabel(app, effect);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        children: [
          LumitTooltip(
            message: effect.enabled ? 'Bypass this effect' : 'Enable this effect',
            child: HouseCheckbox(
              value: effect.enabled,
              onChanged: (v) =>
                  app.setEffectEnabled(compId, layer.id, effect.id, v),
            ),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              label,
              style: t.bodyPrimary.copyWith(
                color: effect.enabled ? t.textPrimary : t.textDisabled,
              ),
              overflow: TextOverflow.ellipsis,
            ),
          ),
          const SizedBox(width: 8),
          LumitTooltip(
            message: 'Remove this effect',
            child: HouseButton(
              frameless: true,
              small: true,
              onPressed: () => app.removeEffect(compId, layer.id, effect.id),
              child: Text(
                '×',
                style: t.bodyPrimary.copyWith(color: t.textMuted),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// One effect parameter, rendered by kind. Scalar and colour edit live; every
/// other kind (enum/bool/seed/point/…) shows its value read-only with a muted
/// tooltip, since the matching edit op is not in the bridge yet.
///
/// An animatable kind (scalar, colour, point) carries the same stopwatch + ◄ ◆ ►
/// navigator the transform rows do (bridge v0.9 effect-param keyframe ops): the
/// stopwatch toggles animation at the playhead, the diamond adds/removes a key,
/// ◄/► jump to the previous/next key. A multi-channel param (a point's x/y, a
/// colour's r/g/b/a) keys every channel at once (one op per channel), so the
/// navigator reads the union of the channels' key frames.
class _EffectParamRow extends StatelessWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  final BridgeEffect effect;
  final BridgeEffectParam param;
  const _EffectParamRow({
    required this.app,
    required this.compId,
    required this.layer,
    required this.effect,
    required this.param,
  });

  /// The keyframe channels for this param's kind, each with a getter for the
  /// value a new key takes at the playhead — null for a non-animatable kind
  /// (enum/bool/seed/file/layer), which shows no stopwatch.
  List<(int, double Function())>? _channels() {
    switch (param.kind) {
      case 'scalar':
        return [
          (0, () => param.value is num ? (param.value as num).toDouble() : 0.0),
        ];
      case 'point':
        final xy = _rgbaOf(param.value);
        return [(0, () => xy[0]), (1, () => xy[1])];
      case 'colour':
        final c = _rgbaOf(param.value);
        return [(0, () => c[0]), (1, () => c[1]), (2, () => c[2]), (3, () => c[3])];
      default:
        return null;
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final channels = _channels();
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        children: [
          if (channels != null) ...[
            _FxKeyframeControls(
              key: ValueKey<String>('fxkf-${effect.id}-${param.name}'),
              app: app,
              compId: compId,
              layerId: layer.id,
              effectId: effect.id,
              param: param,
              channels: channels,
            ),
            const SizedBox(width: 4),
          ],
          Expanded(child: Text(_paramLabel(param.name), style: t.bodyPrimary)),
          const SizedBox(width: 12),
          _paramControl(context, t),
        ],
      ),
    );
  }

  Widget _paramControl(BuildContext context, LumitTheme t) {
    switch (param.kind) {
      case 'scalar':
        // Snapshot v5 carries the declared range: the drag clamps to min/max and
        // paces its sensitivity by the slider span (a wide span drags coarser).
        // An older library (no range) falls back to an unclamped gentle drag.
        final v = param.value is num ? (param.value as num).toDouble() : 0.0;
        final range = param.range;
        final min = range?.min ?? -1000000;
        final max = range?.max ?? 1000000;
        final span = (range?.sliderMax ?? max) - (range?.sliderMin ?? min);
        final speed = range == null || span <= 0 ? 0.5 : (span / 200).abs();
        return SizedBox(
          width: _cellWidth,
          child: DragValueField(
            key: ValueKey<String>('fxparam-${effect.id}-${param.name}'),
            value: v,
            min: min,
            max: max,
            speed: speed <= 0 ? 0.5 : speed,
            decimals: 2,
            onChanged: (nv) => app.setEffectParamScalar(
                compId, layer.id, effect.id, param.name, nv.toDouble()),
          ),
        );
      case 'colour':
        // A swatch, with an eyedropper button that arms a Viewer sample.
        return Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            _EyedropperButton(
              key: ValueKey<String>('fxdropper-${effect.id}-${param.name}'),
              onTap: () => app.armEyedropper(EyedropperArm(
                compId: compId,
                layerId: layer.id,
                effectId: effect.id,
                paramName: param.name,
                alpha: _rgbaOf(param.value).length > 3
                    ? _rgbaOf(param.value)[3]
                    : 1.0,
              )),
            ),
            const SizedBox(width: 4),
            _ColourSwatch(
              key: ValueKey<String>('fxcolour-${effect.id}-${param.name}'),
              rgba: _rgbaOf(param.value),
              onPicked: (r, g, b, a) => app.setEffectParamColour(
                  compId, layer.id, effect.id, param.name, r, g, b, a),
            ),
          ],
        );
      case 'enum':
        // A Choice: the option index. The range carries the option labels
        // (snapshot v5); an older library shows the bare index read-only.
        final options = param.range?.options ?? const [];
        final index = param.value is num ? (param.value as num).toInt() : 0;
        if (options.isEmpty) return _readOnly(t);
        return BareDropdown<int>(
          key: ValueKey<String>('fxenum-${effect.id}-${param.name}'),
          value: index.clamp(0, options.length - 1),
          options: [for (var i = 0; i < options.length; i++) i],
          label: (i) => options[i],
          onChanged: (i) => app.setEffectParamChoice(
              compId, layer.id, effect.id, param.name, i),
        );
      case 'bool':
        return HouseCheckbox(
          key: ValueKey<String>('fxbool-${effect.id}-${param.name}'),
          value: param.value == true,
          onChanged: (v) => app.setEffectParamBool(
              compId, layer.id, effect.id, param.name, v),
        );
      case 'seed':
        final seed = param.value is num ? (param.value as num).toInt() : 0;
        return SizedBox(
          width: _cellWidth,
          child: DragValueField(
            key: ValueKey<String>('fxseed-${effect.id}-${param.name}'),
            value: seed,
            min: 0,
            max: 2147483647,
            speed: 1,
            onChanged: (v) => app.setEffectParamSeed(
                compId, layer.id, effect.id, param.name, v.round()),
          ),
        );
      case 'point':
        final xy = _rgbaOf(param.value); // tolerant [x, y]
        return Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            SizedBox(
              width: _cellWidth,
              child: DragValueField(
                key: ValueKey<String>('fxpointx-${effect.id}-${param.name}'),
                value: xy[0],
                min: -1000000,
                max: 1000000,
                speed: 0.5,
                decimals: 1,
                onChanged: (v) => app.setEffectParamPoint(compId, layer.id,
                    effect.id, param.name, v.toDouble(), xy[1]),
              ),
            ),
            const SizedBox(width: 4),
            SizedBox(
              width: _cellWidth,
              child: DragValueField(
                key: ValueKey<String>('fxpointy-${effect.id}-${param.name}'),
                value: xy[1],
                min: -1000000,
                max: 1000000,
                speed: 0.5,
                decimals: 1,
                onChanged: (v) => app.setEffectParamPoint(compId, layer.id,
                    effect.id, param.name, xy[0], v.toDouble()),
              ),
            ),
          ],
        );
      default:
        // file / layer: read-only until the matching bridge op lands. Show the
        // value honestly, do not fake edits.
        return _readOnly(t);
    }
  }

  /// A read-only value chip (a kind with no editor yet: file / layer).
  Widget _readOnly(LumitTheme t) => LumitTooltip(
        message: 'Edits arrive with the matching bridge op',
        child: Container(
          constraints: const BoxConstraints(maxWidth: 120),
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
          decoration: BoxDecoration(
            color: t.surface3,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          ),
          child: Text(
            _displayValue(param.value),
            style: t.body.copyWith(color: t.textSecondary),
            overflow: TextOverflow.ellipsis,
          ),
        ),
      );
}

/// The eyedropper button beside a Colour parameter: arms a Viewer sample.
class _EyedropperButton extends StatelessWidget {
  final VoidCallback onTap;
  const _EyedropperButton({super.key, required this.onTap});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: 'Eyedropper: sample a colour from the Viewer',
      child: HouseButton(
        frameless: true,
        small: true,
        onPressed: onTap,
        child: lumitIcon(LumitIcon.eyedropper, size: 13, color: t.textMuted),
      ),
    );
  }
}

/// The three-colour channel picker (K-143): three swatches, each opening the
/// colour picker and committing its channel through `setEffectParamColour`.
class _ChannelPickerRow extends StatelessWidget {
  final AppStateStub app;
  final String compId;
  final BridgeLayer layer;
  final BridgeEffect effect;
  const _ChannelPickerRow({
    required this.app,
    required this.compId,
    required this.layer,
    required this.effect,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final swatches = <Widget>[];
    for (var i = 0; i < _channelColourIds.length; i++) {
      final id = _channelColourIds[i];
      BridgeEffectParam? param;
      for (final p in effect.params) {
        if (p.name == id) {
          param = p;
          break;
        }
      }
      if (param == null) continue;
      if (swatches.isNotEmpty) swatches.add(const SizedBox(width: 4));
      swatches.add(Text('${i + 1}', style: t.small.copyWith(color: t.textSecondary)));
      swatches.add(const SizedBox(width: 2));
      swatches.add(_ColourSwatch(
        key: ValueKey<String>('fxchannel-${effect.id}-$id'),
        rgba: _rgbaOf(param.value),
        onPicked: (r, g, b, a) => app.setEffectParamColour(
            compId, layer.id, effect.id, id, r, g, b, a),
      ));
    }
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        children: [
          Expanded(child: Text('Channels', style: t.bodyPrimary)),
          const SizedBox(width: 12),
          ...swatches,
        ],
      ),
    );
  }
}

/// A colour swatch that opens the house colour picker and hands the chosen
/// scene-linear RGBA back to [onPicked]. The parameter's channels are
/// scene-linear 0..1; the picker edits them straight through, the same
/// convention the egui colour rows use (gamma is not re-applied).
class _ColourSwatch extends StatelessWidget {
  final List<double> rgba;
  final void Function(double r, double g, double b, double a) onPicked;
  const _ColourSwatch({super.key, required this.rgba, required this.onPicked});

  Color get _asColour {
    int ch(double f) => (f.clamp(0.0, 1.0) * 255).round();
    return documentColour(ch(rgba[0]), ch(rgba[1]), ch(rgba[2]), 255);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: () async {
        final box = context.findRenderObject()! as RenderBox;
        final origin = box.localToGlobal(Offset(0, box.size.height + 4));
        final picked = await showColourPicker(
          context: context,
          position: origin,
          initial: _asColour,
        );
        if (picked != null) {
          final a = rgba.length > 3 ? rgba[3] : 1.0;
          onPicked(picked.r, picked.g, picked.b, a);
        }
      },
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        child: Container(
          width: 28,
          height: 18,
          decoration: BoxDecoration(
            color: _asColour,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(color: t.hairlineStrong),
          ),
        ),
      ),
    );
  }
}

/// A parameter value's scene-linear RGBA, tolerant of `[r,g,b]` or `[r,g,b,a]`.
List<double> _rgbaOf(Object? value) {
  if (value is List) {
    final out = [for (final e in value) e is num ? e.toDouble() : 0.0];
    while (out.length < 4) {
      out.add(out.length == 3 ? 1.0 : 0.0);
    }
    return out;
  }
  return const [0, 0, 0, 1];
}

/// A read-only display string for a non-editable parameter value.
String _displayValue(Object? value) {
  if (value == null) return '—';
  if (value is bool) return value ? 'On' : 'Off';
  if (value is List) {
    return value
        .map((e) => e is num ? _trim(e.toDouble()) : e.toString())
        .join(', ');
  }
  if (value is num) return _trim(value.toDouble());
  return value.toString();
}

String _trim(double v) {
  final s = v.toStringAsFixed(2);
  return s.endsWith('.00') ? v.round().toString() : s;
}

/// A parameter's display label from its stable name: `blur_radius` → "Blur
/// radius" (the registry does not carry per-param labels).
String _paramLabel(String name) {
  final words = name.split('_').where((w) => w.isNotEmpty).toList();
  if (words.isEmpty) return name;
  final first = words.first;
  final head = first.isEmpty
      ? first
      : '${first[0].toUpperCase()}${first.substring(1)}';
  return [head, ...words.skip(1)].join(' ');
}

/// An effect's display label: its registry label when known, else its match
/// name (the snapshot carries the match name in `effect.name`).
String _effectLabel(AppStateStub app, BridgeEffect effect) {
  for (final info in app.listEffects()) {
    if (info.name == effect.name) return info.label;
  }
  return _paramLabel(effect.name);
}

/// The icon and tint for a layer kind, mirroring the egui `layer_type_style`.
(LumitIcon, Color) _layerStyle(BridgeLayerKind kind, LumitTheme t) =>
    switch (kind) {
      BridgeLayerKind.footage => (LumitIcon.footage, t.layer.footage),
      BridgeLayerKind.sequence => (LumitIcon.sequence, t.layer.sequence),
      BridgeLayerKind.precomp => (LumitIcon.comp, t.layer.precomp),
      BridgeLayerKind.solid => (LumitIcon.solid, t.layer.solid),
      BridgeLayerKind.text => (LumitIcon.text, t.layer.text),
      BridgeLayerKind.camera => (LumitIcon.camera, t.layer.camera),
      BridgeLayerKind.adjustment => (LumitIcon.solid, t.layer.solid),
      BridgeLayerKind.unknown => (LumitIcon.footage, t.textMuted),
    };
