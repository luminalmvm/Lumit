// The Timeline's shape rows: one piece of a shape layer's art, its strokes,
// its paints, and the value rows under them.
//
// Split out of timeline_panel_frb.dart.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/l10n/engine_labels.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../state/layer_bounds.dart' show shapeContentsRect;
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';
import 'text_animator_rows_frb.dart';
import 'package:lumit_flutter/state/preview_throttle.dart';
import 'keyframe_controls_frb.dart';
import 'layer_fold_frb.dart';
import 'timeline_mask_rows_frb.dart';

/// One piece of a shape layer's art in the Timeline (K-237), on the shared
/// [ItemOpacityRow]. The engine takes the whole contents list, so every
/// edit — and the drag's preview — is "the list, with this item changed".
class ShapeItemRow extends StatelessWidget {
  final LayerReference layer;
  final BridgeShapeItem item;
  final ValueColumn valueColumn;
  final VoidCallback onChanged;

  /// The composition, for the live preview a drag shows (K-239).
  final CompositionReference comp;

  const ShapeItemRow({super.key, 
    required this.layer,
    required this.item,
    required this.valueColumn,
    required this.onChanged,
    required this.comp,
  });

  static BridgeShapeItem _with(BridgeShapeItem i,
          {String? name, double? opacity}) =>
      shapeItemWith(i, name: name, opacity: opacity);

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
    return ItemOpacityRow(
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

/// One paint stroke in the Timeline (K-227), on the shared [ItemOpacityRow].
/// The engine takes the whole stroke, so every edit is "this stroke, with one
/// field changed" — and its name is not renamed here, so the row shows it
/// plain.
class StrokeRow extends StatelessWidget {
  final LayerReference layer;
  final BridgeStroke stroke;
  final ValueColumn valueColumn;
  final VoidCallback onChanged;

  /// The composition, for the live preview a drag shows (K-239).
  final CompositionReference comp;

  const StrokeRow({super.key, 
    required this.layer,
    required this.stroke,
    required this.valueColumn,
    required this.onChanged,
    required this.comp,
  });

  static BridgeStroke _with(BridgeStroke s, {double? opacity, int? blend}) =>
      BridgeStroke(
        id: s.id,
        name: s.name,
        points: s.points,
        colour: s.colour,
        width: s.width,
        hardness: s.hardness,
        shape: s.shape,
        opacity: opacity ?? s.opacity,
        start: s.start,
        end: s.end,
        mode: s.mode,
        blend: blend ?? s.blend,
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
    return ItemOpacityRow(
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
                if (s.id == stroke.id) _with(s, opacity: opacity) else s,
            ],
          );
        } catch (_) {
          // A preview is a courtesy; the drag carries on without it.
        }
      },
      onCommit: (opacity) {
        try {
          layer.setStroke(stroke: _with(stroke, opacity: opacity));
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
      // The layer blend list, on a stroke (K-550) — the same words, from the
      // same engine table, so a mark blends by the name it blends by
      // everywhere else.
      extra: _StrokeBlendPicker(
        layer: layer,
        stroke: stroke,
        onChanged: onChanged,
      ),
    );
  }
}

/// A stroke's blend mode (K-550), on the stroke's own Timeline row.
///
/// The engine's list, read once and held: `listBlendModes` is a table of
/// English words the engine owns, and every one of them has a translation
/// entry already because a layer's own picker shows the same list.
class _StrokeBlendPicker extends StatelessWidget {
  final LayerReference layer;
  final BridgeStroke stroke;
  final VoidCallback onChanged;

  const _StrokeBlendPicker({
    required this.layer,
    required this.stroke,
    required this.onChanged,
  });

  static List<String>? _modes;

  @override
  Widget build(BuildContext context) {
    final modes = _modes ??= listBlendModes();
    return SizedBox(
      width: 96,
      child: BareDropdown<int>(
        key: ValueKey<String>('tl-stroke-blend-${stroke.id}'),
        value: stroke.blend < modes.length ? stroke.blend : 0,
        options: [for (var i = 0; i < modes.length; i++) i],
        label: (i) => engineLabel(modes[i]),
        onChanged: (i) {
          try {
            layer.setStroke(stroke: StrokeRow._with(stroke, blend: i));
            onChanged();
          } catch (_) {
            // The stroke or its layer went away between the draw and the click.
          }
        },
      ),
    );
  }
}

/// A stroke's Start or End (K-549) — the pair that draws a stroke on.
///
/// The same shape as [MaskValueRow], and for the same reasons: the value is
/// staged and previewed through the paint preview while it is dragged, the
/// release commits one op, and an edit on an animated one lands on the key
/// under the playhead rather than flattening the curve.
class StrokeValueRow extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final BridgeStroke stroke;
  final StrokeValue value;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  const StrokeValueRow({super.key, 
    required this.comp,
    required this.layer,
    required this.stroke,
    required this.value,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
  });

  @override
  State<StrokeValueRow> createState() => _StrokeValueRowState();
}

class _StrokeValueRowState extends State<StrokeValueRow> {
  double? _staged;
  final PreviewThrottle _throttle = PreviewThrottle();

  BridgeScalar get _scalar => strokeScalarOf(widget.stroke, widget.value);

  String get _rowKey => 'tl-stroke-${widget.value.name}-${widget.stroke.id}';

  @override
  void dispose() {
    _throttle.cancel();
    super.dispose();
  }

  /// Show the value the drag is passing through without writing it (K-240).
  /// The whole stroke list is sent with this one number replaced, because
  /// paint is stored and committed as a whole list.
  void _preview(BridgeScalar v) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    _throttle.request(() {
      try {
        widget.comp.renderFrameWithPaintPreview(
          frame: BigInt.from(ui.playheadFrame.value),
          scale: ui.viewerScale,
          layer: widget.layer,
          strokes: [
            for (final s in widget.layer.getPaint())
              if (s.id == widget.stroke.id)
                strokeWithScalar(s, widget.value, v)
              else
                s,
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
      widget.layer
          .setStroke(stroke: strokeWithScalar(widget.stroke, widget.value, v));
      widget.onChanged();
    } catch (_) {
      // The stroke or its layer went away mid-drag.
    }
  }

  void _commitStatic(num v) => _write(BridgeScalar.static_(v.toDouble()));

  void _commitKeyed(double v) =>
      _write(scalarWithValueAt(_scalar, v, widget.comp, widget.playheadFrame));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Row(
      children: [
        KeyframeControlsFrb(
          scalars: [_scalar],
          onWrite: (s) => _write(s.first),
          comp: widget.comp,
          playheadFrame: widget.playheadFrame,
          onSeek: widget.onSeek,
          rowKey: _rowKey,
        ),
        const SizedBox(width: 4),
        Expanded(
          child: Text(strokeValueLabel(widget.value),
              style: t.body, overflow: TextOverflow.ellipsis),
        ),
        SizedBox(
          width: widget.valueColumn.width,
          child: Align(
            alignment: Alignment.centerLeft,
            child: SizedBox(width: 72, child: _field()),
          ),
        ),
        SizedBox(width: widget.valueColumn.rightInset),
      ],
    );
  }

  Widget _field() {
    final key = ValueKey<String>(_rowKey);
    final scalar = _scalar;
    if (scalar is! BridgeScalar_Keyframed) {
      final stored =
          _staged ?? (scalar is BridgeScalar_Static ? scalar.field0 : 0.0);
      return DragValueField(
        key: key,
        value: stored,
        min: 0,
        max: 100,
        decimals: 1,
        suffix: '%',
        onChanged: _commitStatic,
        onChangeLive: (v) {
          setState(() => _staged = v.toDouble());
          _preview(BridgeScalar.static_(v.toDouble()));
        },
        onChangeEnd: _commitStatic,
        onDragCancel: () {
          setState(() => _staged = null);
          _preview(scalar);
        },
      );
    }
    // Animated: the field shows what the curve reads at the playhead, and an
    // edit writes the key there.
    return KeyedValueField(
      fieldKey: key,
      value:
          sampledScalar(scalar, timeOfFrame(widget.comp, widget.playheadFrame)),
      min: 0,
      max: 100,
      decimals: 1,
      suffix: '%',
      onCommit: _commitKeyed,
    );
  }
}

/// One of an animator's numbers in the Timeline (K-609) — a range end, a push,
/// a fade — on the same shape of row a stroke's write-on uses.
///
/// **No live preview while it drags**, unlike the mask and stroke rows beside
/// it: a text document has no preview call of its own, so the value is staged
/// on the row and the picture catches up when the drag is let go. The op, the
/// undo step and the committed value are the same either way; only the
/// in-flight picture is.
class AnimatorValueRow extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final int index;
  final BridgeTextAnimator animator;
  final TextAnimatorValue value;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  const AnimatorValueRow({super.key, 
    required this.comp,
    required this.layer,
    required this.index,
    required this.animator,
    required this.value,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
  });

  @override
  State<AnimatorValueRow> createState() => _AnimatorValueRowState();
}

class _AnimatorValueRowState extends State<AnimatorValueRow> {
  double? _staged;

  BridgeScalar get _scalar =>
      textAnimatorScalarOf(widget.animator, widget.value);

  String get _rowKey => 'tl-anim-${widget.index}-${widget.value.name}';

  void _write(BridgeScalar v) {
    setState(() => _staged = null);
    try {
      writeTextAnimatorScalar(
        layer: widget.layer,
        index: widget.index,
        value: widget.value,
        to: v,
      );
      widget.onChanged();
    } catch (_) {
      // The animator or its layer went away mid-drag.
    }
  }

  void _commitStatic(num v) => _write(BridgeScalar.static_(v.toDouble()));

  void _commitKeyed(double v) =>
      _write(scalarWithValueAt(_scalar, v, widget.comp, widget.playheadFrame));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Row(
      children: [
        KeyframeControlsFrb(
          scalars: [_scalar],
          onWrite: (s) => _write(s.first),
          comp: widget.comp,
          playheadFrame: widget.playheadFrame,
          onSeek: widget.onSeek,
          rowKey: _rowKey,
        ),
        const SizedBox(width: 4),
        Expanded(
          child: Text(textAnimatorValueLabel(widget.value),
              style: t.body, overflow: TextOverflow.ellipsis),
        ),
        SizedBox(
          width: widget.valueColumn.width,
          child: Align(
            alignment: Alignment.centerLeft,
            child: SizedBox(width: 72, child: _field()),
          ),
        ),
        SizedBox(width: widget.valueColumn.rightInset),
      ],
    );
  }

  Widget _field() {
    final key = ValueKey<String>(_rowKey);
    final scalar = _scalar;
    if (scalar is! BridgeScalar_Keyframed) {
      final stored =
          _staged ?? (scalar is BridgeScalar_Static ? scalar.field0 : 0.0);
      return DragValueField(
        key: key,
        value: stored,
        min: -100000,
        max: 100000,
        decimals: 1,
        onChanged: _commitStatic,
        onChangeLive: (v) => setState(() => _staged = v.toDouble()),
        onChangeEnd: _commitStatic,
        onDragCancel: () => setState(() => _staged = null),
      );
    }
    // Animated: the field shows what the curve reads at the playhead, and an
    // edit writes the key there.
    return KeyedValueField(
      fieldKey: key,
      value:
          sampledScalar(scalar, timeOfFrame(widget.comp, widget.playheadFrame)),
      min: -100000,
      max: 100000,
      decimals: 1,
      onCommit: _commitKeyed,
    );
  }
}

/// One of a shape item's animatable numbers in the Timeline (K-551) — its Trim
/// start, end or offset — on the same shape of row a stroke's write-on uses.
///
/// The whole contents list is sent for every write, because a shape layer's art
/// is stored and committed as a whole list (K-237): a preview shaped differently
/// from the op would be a second description of the same thing.
class ShapeValueRow extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final BridgeShapeItem item;
  final ShapeValue value;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  const ShapeValueRow({super.key, 
    required this.comp,
    required this.layer,
    required this.item,
    required this.value,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
  });

  @override
  State<ShapeValueRow> createState() => _ShapeValueRowState();
}

class _ShapeValueRowState extends State<ShapeValueRow> {
  double? _staged;
  final PreviewThrottle _throttle = PreviewThrottle();

  BridgeScalar get _scalar => shapeScalarOf(widget.item, widget.value);

  String get _rowKey => 'tl-shape-${widget.value.name}-${widget.item.id}';

  /// The trim's two ends are a per cent of the path's own length; its offset is
  /// degrees, because degrees go round; the dashes are lengths in layer pixels.
  bool get _isPath => widget.value == ShapeValue.path;

  (double, double, String) get _units => switch (widget.value) {
        // The shape has no number to drag, so no range and no unit: its row is
        // the stopwatch and its diamonds (K-606).
        ShapeValue.path => (0, 0, ''),
        // Out or in, in layer pixels.
        ShapeValue.offsetPath => (-1000, 1000, ' px'),
        // Where the ramp starts and ends, in the art's own coordinates.
        ShapeValue.gradientStartX ||
        ShapeValue.gradientStartY ||
        ShapeValue.gradientEndX ||
        ShapeValue.gradientEndY =>
          (-10000, 10000, ' px'),
        ShapeValue.trimStart || ShapeValue.trimEnd => (0, 100, '%'),
        ShapeValue.trimOffset => (-3600, 3600, '°'),
        ShapeValue.dash || ShapeValue.gap => (0, 1000, ' px'),
        ShapeValue.dashOffset => (-1000, 1000, ' px'),
        // The repeater: the count and which copy the original is are whole
        // things, and the step is read in the units the layer's own transform
        // is — pixels, degrees, per cent.
        ShapeValue.repeatCopies => (1, maxShapeCopies, ''),
        ShapeValue.repeatOffset => (-maxShapeCopies, maxShapeCopies, ''),
        ShapeValue.repeatAnchorX ||
        ShapeValue.repeatAnchorY ||
        ShapeValue.repeatPositionX ||
        ShapeValue.repeatPositionY =>
          (-10000, 10000, ' px'),
        ShapeValue.repeatRotation => (-3600, 3600, '°'),
        ShapeValue.repeatScale => (-1000, 1000, '%'),
        ShapeValue.repeatStartOpacity || ShapeValue.repeatEndOpacity => (
            0,
            100,
            '%'
          ),
      };
  double get _min => _units.$1;
  double get _max => _units.$2;
  String get _suffix => _units.$3;

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
        widget.comp.renderFrameWithShapePreview(
          frame: BigInt.from(ui.playheadFrame.value),
          scale: ui.viewerScale,
          layer: widget.layer,
          contents: [
            for (final i in widget.layer.getShapeContents())
              if (i.id == widget.item.id)
                shapeWithScalar(i, widget.value, v)
              else
                i,
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
      widget.layer.setShapeContents(
        contents: [
          for (final i in widget.layer.getShapeContents())
            if (i.id == widget.item.id)
              shapeWithScalar(i, widget.value, v)
            else
              i,
        ],
        // Not a shape edit: a keyed path is carried through untouched (K-606).
        at: null,
      );
      widget.onChanged();
    } catch (_) {
      // The item or its layer went away mid-drag.
    }
  }

  void _commitStatic(num v) => _write(BridgeScalar.static_(v.toDouble()));

  void _commitKeyed(double v) =>
      _write(scalarWithValueAt(_scalar, v, widget.comp, widget.playheadFrame));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Row(
      children: [
        if (_isPath)
          PathKeyframesFrb(
            keys: widget.item.pathKeys,
            rowKey: _rowKey,
            onToggleKey: (time) => widget.layer
                .toggleShapePathKey(id: widget.item.id, time: time),
            onClear: (time) => widget.layer
                .clearShapePathKeys(id: widget.item.id, time: time),
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
            rowKey: _rowKey,
          ),
        const SizedBox(width: 4),
        Expanded(
          child: Text(shapeValueLabel(widget.value),
              style: t.body, overflow: TextOverflow.ellipsis),
        ),
        SizedBox(
          width: widget.valueColumn.width,
          child: Align(
            alignment: Alignment.centerLeft,
            // A shape is not a number, so its row has no value field — the
            // drawing tools are where it is edited.
            child: _isPath
                ? const SizedBox.shrink()
                : SizedBox(width: 72, child: _field()),
          ),
        ),
        SizedBox(width: widget.valueColumn.rightInset),
      ],
    );
  }

  Widget _field() {
    final key = ValueKey<String>(_rowKey);
    final scalar = _scalar;
    if (scalar is! BridgeScalar_Keyframed) {
      final stored =
          _staged ?? (scalar is BridgeScalar_Static ? scalar.field0 : 0.0);
      return DragValueField(
        key: key,
        value: stored,
        min: _min,
        max: _max,
        decimals: 1,
        suffix: _suffix,
        onChanged: _commitStatic,
        onChangeLive: (v) {
          setState(() => _staged = v.toDouble());
          _preview(BridgeScalar.static_(v.toDouble()));
        },
        onChangeEnd: _commitStatic,
        onDragCancel: () {
          setState(() => _staged = null);
          _preview(scalar);
        },
      );
    }
    // Animated: the field shows what the curve reads at the playhead, and an
    // edit writes the key there.
    return KeyedValueField(
      fieldKey: key,
      value:
          sampledScalar(scalar, timeOfFrame(widget.comp, widget.playheadFrame)),
      min: _min,
      max: _max,
      decimals: 1,
      suffix: _suffix,
      onCommit: _commitKeyed,
    );
  }
}

/// A shape item's fill colour, its gradient choice, or the gradient's second
/// colour (K-555). None of the three is a number, so this row carries a swatch
/// or a dropdown where the others carry a value field — and no stopwatch,
/// because none of them keys.
class ShapePaintRow extends StatelessWidget {
  final LayerReference layer;
  final BridgeShapeItem item;
  final ShapePaint which;
  final ValueColumn valueColumn;
  final VoidCallback onChanged;

  const ShapePaintRow({super.key, 
    required this.layer,
    required this.item,
    required this.which,
    required this.valueColumn,
    required this.onChanged,
  });

  /// The whole list back with this item changed — how every shape edit is
  /// written (K-283), so this is one op and one undo step.
  void _write(BridgeShapeItem Function(BridgeShapeItem) change) {
    try {
      layer.setShapeContents(
        contents: [
          for (final other in layer.getShapeContents())
            if (other.id == item.id) change(other) else other,
        ],
        // Not a shape edit: a keyed path is carried through untouched (K-606).
        at: null,
      );
      onChanged();
    } catch (_) {
      // The item or its layer went away between the draw and the click.
    }
  }

  /// The far end of the ramp before anybody has picked one: black, which is
  /// what the engine draws for a gradient with no second colour.
  static const _defaultEnd = BridgeColourRgba(r: 0, g: 0, b: 0, a: 1);

  Color _shown(BridgeColourRgba c) => documentColour(
        (c.r.clamp(0.0, 1.0) * 255).round(),
        (c.g.clamp(0.0, 1.0) * 255).round(),
        (c.b.clamp(0.0, 1.0) * 255).round(),
        255,
      );

  BridgeColourRgba _picked(PickedColour p, double alpha) =>
      BridgeColourRgba(r: p.r, g: p.g, b: p.b, a: alpha);

  /// Where a ramp nobody has aimed should start and end: down the art's own
  /// box for a linear one, out from its middle for a radial one. A gradient
  /// that read as one flat colour the moment it was switched on would look
  /// broken rather than unaimed.
  BridgeShapeItem _aimed(BridgeShapeItem i, int kind) {
    final aimed = i.gradientStartX != const BridgeScalar.static_(0) ||
        i.gradientEndX != const BridgeScalar.static_(0) ||
        i.gradientStartY != const BridgeScalar.static_(0) ||
        i.gradientEndY != const BridgeScalar.static_(0);
    if (kind == 0 || aimed) return shapeItemWith(i, gradient: kind);
    final box = shapeContentsRect([i]);
    if (box == null) return shapeItemWith(i, gradient: kind);
    final (start, end) = kind == 2
        ? (box.center, Offset(box.right, box.center.dy))
        : (Offset(box.center.dx, box.top), Offset(box.center.dx, box.bottom));
    return shapeItemWith(
      i,
      gradient: kind,
      gradientStartX: BridgeScalar.static_(start.dx),
      gradientStartY: BridgeScalar.static_(start.dy),
      gradientEndX: BridgeScalar.static_(end.dx),
      gradientEndY: BridgeScalar.static_(end.dy),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final key = 'tl-shape-${which.name}-${item.id}';
    return Row(
      children: [
        // The width the stopwatch and its gap take on every other row, so the
        // labels line up down the fold.
        const SizedBox(width: 24),
        Expanded(
          child: Text(shapePaintLabel(which),
              style: t.body, overflow: TextOverflow.ellipsis),
        ),
        SizedBox(
          width: valueColumn.width,
          child: Align(
            alignment: Alignment.centerLeft,
            child: switch (which) {
              // Apart, or one of the four ways two paths combine (K-605).
              ShapePaint.combine => SizedBox(
                  width: 96,
                  child: BareDropdown<int>(
                    key: ValueKey<String>(key),
                    value: item.combine <= 4 ? item.combine : 0,
                    options: const [0, 1, 2, 3, 4],
                    label: (k) => switch (k) {
                      1 => l10n.shapeCombineUnion,
                      2 => l10n.shapeCombineSubtract,
                      3 => l10n.shapeCombineIntersect,
                      4 => l10n.shapeCombineExclude,
                      _ => l10n.shapeCombineApart,
                    },
                    onChanged: (k) => _write((i) => shapeItemWith(i, combine: k)),
                  ),
                ),
              ShapePaint.gradient => SizedBox(
                  width: 96,
                  child: BareDropdown<int>(
                    key: ValueKey<String>(key),
                    value: item.gradient <= 2 ? item.gradient : 0,
                    options: const [0, 1, 2],
                    label: (k) => switch (k) {
                      1 => l10n.shapeGradientLinear,
                      2 => l10n.shapeGradientRadial,
                      _ => l10n.shapeGradientFlat,
                    },
                    onChanged: (k) => _write((i) => _aimed(i, k)),
                  ),
                ),
              ShapePaint.fill => ColourSwatchButton(
                  key: ValueKey<String>(key),
                  colour: _shown(item.fill ?? _defaultEnd),
                  // A shape's colours are scene-linear, so the picker counts
                  // in 0—1 rather than in bytes.
                  scale: ColourScale.unit,
                  onPicked: (p) => _write((i) =>
                      shapeItemWith(i, fill: _picked(p, i.fill?.a ?? 1))),
                ),
              ShapePaint.gradientColour => ColourSwatchButton(
                  key: ValueKey<String>(key),
                  colour: _shown(item.gradientColour ?? _defaultEnd),
                  scale: ColourScale.unit,
                  onPicked: (p) => _write((i) => shapeItemWith(i,
                      gradientColour: _picked(p, i.gradientColour?.a ?? 1))),
                ),
            },
          ),
        ),
        SizedBox(width: valueColumn.rightInset),
      ],
    );
  }
}
