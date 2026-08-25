// A layer's Transform properties as editable rows — the one implementation of
// them, used by both panels that show them.
//
// The Effect controls panel wraps these in its Transform card; the Timeline
// twirls them open under a layer. They were the Effect controls card's private
// business first, and the Timeline's fold-out would have been a second copy of
// the same eleven properties, the same staging, and the same preview throttle —
// which is exactly the kind of copy that drifts. So they moved here whole.
//
// **What a row is.** Keyframe controls (the stopwatch and the ◄ ◆ ► navigator),
// a label, and one draggable value per axis. A property group that has more than
// one axis — Position is x and y — is *one* row with one stopwatch, because a
// control that says "Position" has to act on Position; the axes are separate
// properties underneath (which is what makes a per-axis curve possible) and are
// committed together as one op.
//
// **What a drag costs.** One undo step, not one per tick. A tick stages the new
// value locally and renders it through `renderFrameWithTransformPreview`, which
// patches a clone of the document engine-side and never touches the document;
// only the release commits. An animated property is not draggable at all —
// writing a static value over a curve would delete it — and says so instead.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../state/preview_throttle.dart';
import '../state/timeline_columns.dart';
import '../widgets/angle_dial.dart';
import '../widgets/controls.dart';
import 'graph_editor_frb.dart';
import 'fx_section.dart';
import 'keyframe_controls_frb.dart';

/// How wide one value cell is. Fixed rather than flexible so the columns line
/// up down the card, which is what makes a stack of numbers readable.
const double transformCellWidth = 74;

/// One axis of a transform row: which property it edits, and the display hints
/// that make its drag feel right.
class TransformAxis {
  final BridgeTransformProp prop;
  final String? suffix;
  final double min;
  final double max;
  final int decimals;
  final double speed;
  const TransformAxis(
    this.prop, {
    this.suffix,
    this.min = -100000,
    this.max = 100000,
    this.decimals = 1,
    this.speed = 1,
  });
}

/// One row: its label and the axes it edits.
class TransformGroup {
  final String label;
  final List<TransformAxis> axes;
  const TransformGroup(this.label, this.axes);
}

/// The rows a layer shows, in order.
///
/// The 3D rows (Position z, Rotation x, Rotation y) appear only on a 3D layer: a
/// 2D layer showing controls that cannot do anything is worse than not showing
/// them. Exposed as a list rather than built inline because the Timeline has to
/// know *how many* rows a layer will take before it draws them — its lanes have
/// to leave exactly that much room or the bars stop lining up with the names.
List<TransformGroup> transformGroups({required bool threeD}) => [
      TransformGroup(l10n.transformAnchorPoint, const [
        TransformAxis(BridgeTransformProp.anchorX),
        TransformAxis(BridgeTransformProp.anchorY),
      ]),
      TransformGroup(l10n.transformPosition, [
        const TransformAxis(BridgeTransformProp.positionX),
        const TransformAxis(BridgeTransformProp.positionY),
        if (threeD) const TransformAxis(BridgeTransformProp.positionZ),
      ]),
      TransformGroup(l10n.transformScale, const [
        TransformAxis(BridgeTransformProp.scaleX, suffix: '%'),
        TransformAxis(BridgeTransformProp.scaleY, suffix: '%'),
      ]),
      TransformGroup(l10n.transformRotation, const [
        TransformAxis(BridgeTransformProp.rotation, suffix: '°', speed: 0.5),
      ]),
      if (threeD) ...[
        TransformGroup(l10n.transformRotationX, const [
          TransformAxis(BridgeTransformProp.rotationX, suffix: '°', speed: 0.5),
        ]),
        TransformGroup(l10n.transformRotationY, const [
          TransformAxis(BridgeTransformProp.rotationY, suffix: '°', speed: 0.5),
        ]),
      ],
      TransformGroup(l10n.transformOpacity, [
        TransformAxis(BridgeTransformProp.opacity,
            suffix: '%', min: 0, max: 100, decimals: 0, speed: 0.5),
      ]),
    ];

/// A layer's transform rows, all of them.
///
/// The Effect controls card shows the whole set; the Timeline's fold-out draws
/// them one at a time (its lanes are per row), so the row itself is the widget
/// that carries the behaviour and this is a Column of them.
class TransformRowsFrb extends StatelessWidget {
  final CompositionReference comp;
  final LayerReference layer;

  /// The layer's transform and 3D flag, from the read model (K-184) — so
  /// drawing the rows costs no bridge calls.
  final BridgeTransform transform;
  final bool threeD;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  /// Prefixes every row's widget key, so the same rows in two panels do not
  /// collide when both are on screen.
  final String keyPrefix;

  /// A fixed height per row, for a caller that has to line something up beside
  /// them (the Timeline's lanes). Null lets each row take what it needs.
  final double? rowHeight;

  /// Padding inside each row.
  final EdgeInsets rowPadding;

  /// Lay the rows out as the Effect controls panel's two columns.
  final bool twoColumn;

  const TransformRowsFrb({
    super.key,
    required this.comp,
    required this.layer,
    required this.transform,
    required this.threeD,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
    this.keyPrefix = 'tf',
    this.rowHeight,
    this.rowPadding = const EdgeInsets.symmetric(vertical: 2),
    this.twoColumn = false,
  });

  /// One widget per transform row — for a caller that has to put each row in its
  /// own chrome (the Effect controls panel's hairline-separated rows).
  List<Widget> rows(BuildContext context) => [
        for (final group in transformGroups(threeD: threeD))
          TransformRowFrb(
            comp: comp,
            layer: layer,
            transform: transform,
            group: group,
            playheadFrame: playheadFrame,
            onSeek: onSeek,
            onChanged: onChanged,
            keyPrefix: keyPrefix,
            rowHeight: rowHeight,
            rowPadding: rowPadding,
            twoColumn: twoColumn,
          ),
      ];

  @override
  Widget build(BuildContext context) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: rows(context),
      );
}

/// One transform property group as a row.
class TransformRowFrb extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;

  /// The layer's transform as the owner last read it — one read shared by all
  /// the rows, rather than one crossing per row (K-183).
  final BridgeTransform transform;
  final TransformGroup group;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;
  final String keyPrefix;
  final double? rowHeight;
  final EdgeInsets rowPadding;

  /// When set (the Timeline's fold-out), the value cells share this fixed
  /// span — aligned to both its edges — instead of each taking
  /// [transformCellWidth], so the values sit exactly under the render-switch
  /// column group whatever order the groups are dragged into (docs/07 §4.3).
  final ValueColumn? valueColumn;

  /// Clicking the property's *name* selects it for the graph editor
  /// (docs/07 §4.3) — the name, not the whole row, so grabbing a value field
  /// or a stopwatch never re-aims the graph.
  final VoidCallback? onLabelTap;

  /// The property's graph line colours while it is selected — one per axis,
  /// so Position reads as its x and y strokes (docs/07 §5). The label takes
  /// the first; a multi-axis row shows one dot per axis beside it.
  final List<Color>? graphColours;

  /// Lay the row out as the Effect controls panel's two columns — name left,
  /// axes left-aligned in the rest. Ignored when [valueColumn] is set.
  final bool twoColumn;

  const TransformRowFrb({
    super.key,
    required this.comp,
    required this.layer,
    required this.transform,
    required this.group,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
    this.keyPrefix = 'tf',
    this.rowHeight,
    this.rowPadding = const EdgeInsets.symmetric(vertical: 2),
    this.valueColumn,
    this.onLabelTap,
    this.graphColours,
    this.twoColumn = false,
  });

  @override
  State<TransformRowFrb> createState() => _TransformRowFrbState();
}

class _TransformRowFrbState extends State<TransformRowFrb> {
  /// The transform being dragged, held only for the length of one drag, so the
  /// preview renders the other ten properties as the document has them.
  BridgeTransform? _staged;

  /// Bounded preview rate, holding rather than dropping the ticks in between,
  /// so the pointer's last position always reaches the picture.
  final PreviewThrottle _throttle = PreviewThrottle();

  /// Held rather than read through the context on the way past, because
  /// [dispose] needs it after the context is no longer a place to look things
  /// up — and the notifier itself outlives every row.
  ///
  /// Filled in [initState] rather than lazily: a row that is never dragged
  /// would otherwise run the lookup for the first time *in* [dispose], where
  /// the element is already being taken down and an ancestor lookup throws.
  late final LumitUiState _ui;

  @override
  void initState() {
    super.initState();
    _ui = Provider.of<LumitUiState>(context, listen: false);
  }

  /// Tell the Viewer's boxes what this drag is doing. The picture is already
  /// previewed at `staged`; without this the wireframe drawn from the document
  /// sits still until the drag is let go (see [LumitUiState.liveTransforms]).
  void _publishLive(BridgeTransform staged) {
    _ui.liveTransforms.value = {widget.layer.internallayerId: staged};
  }

  /// The gesture is over: the document is the truth again.
  void _clearLive() {
    if (_ui.liveTransforms.value.isNotEmpty) {
      _ui.liveTransforms.value = const {};
    }
  }

  @override
  void dispose() {
    _throttle.cancel();
    // A drag cut short by the panel closing or the selection changing must not
    // leave the box frozen at a provisional value nothing will ever replace.
    // After the frame, not in it: dispose can run inside a build, and the
    // Viewer's boxes builder listens to this notifier — firing it mid-build
    // marks that builder dirty while it is building. The notifier belongs to
    // the shell, so it is safe to touch once this row has gone.
    final live = _ui.liveTransforms;
    if (live.value.isNotEmpty) {
      WidgetsBinding.instance
          .addPostFrameCallback((_) => live.value = const {});
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    // The live playhead: an animated cell shows (and edits) the value under
    // the playhead, so it must follow a scrub rather than hold the frame the
    // panel last drew at.
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;
    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) =>
          _row(_staged ?? widget.transform, widget.group, frame),
    );
  }

  Widget _row(BridgeTransform transform, TransformGroup group, int frame) {
    final t = ThemeScope.of(context).theme;
    // One stopwatch, every axis in the row — and one undo step, because
    // `setTransforms` commits them as a batch.
    final keyframes = KeyframeControlsFrb(
      scalars: [for (final axis in group.axes) read(transform, axis.prop)],
      comp: widget.comp,
      playheadFrame: widget.playheadFrame,
      onSeek: widget.onSeek,
      rowKey: '${widget.keyPrefix}-${group.axes.first.prop.name}',
      // The Effect controls panel's fixed columns (K-443); the Timeline's
      // fold-out draws the same rows against its own column group and keeps
      // the loose layout.
      fixedColumns: widget.twoColumn && widget.valueColumn == null,
      onWrite: (next) {
        widget.layer.setTransforms(
          props: [for (final axis in group.axes) axis.prop],
          values: next,
        );
        setState(() => _staged = null);
        widget.onChanged();
      },
    );

    // The name is the row's handle for the graph editor, so it is built once
    // and drawn by whichever layout the row takes.
    final label = GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: widget.onLabelTap,
      child: Row(
        children: [
          Flexible(
            child: Text(
              group.label,
              style: widget.graphColours?.isNotEmpty ?? false
                  ? t.body.copyWith(color: widget.graphColours!.first)
                  : t.body,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          // One dot per axis in its stroke colour, so a two-axis property names
          // both of its curves.
          if ((widget.graphColours?.length ?? 0) > 1)
            for (final colour in widget.graphColours!)
              Padding(
                padding: const EdgeInsets.only(left: 3),
                child: Container(
                  width: 5,
                  height: 5,
                  decoration: BoxDecoration(
                    color: colour,
                    borderRadius: BorderRadius.circular(3),
                  ),
                ),
              ),
        ],
      ),
    );

    if (widget.twoColumn && widget.valueColumn == null) {
      // No padding of its own: the Effect controls panel gives every row the
      // same fixed height ([fxRowHeight]), and padding on top of that would
      // eat into the room the controls sit in.
      final row = fxTwoColumnRow(
        context: context,
        name: label,
        keyframeControls: keyframes,
        control: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (var i = 0; i < group.axes.length; i++) ...[
              if (i > 0) const SizedBox(width: 6),
              _cell(transform, group.axes[i], frame),
            ],
          ],
        ),
      );
      final height = widget.rowHeight;
      return height == null ? row : SizedBox(height: height, child: row);
    }

    final row = Padding(
      padding: widget.rowPadding,
      child: Row(
        children: [
          keyframes,
          const SizedBox(width: 4),
          Expanded(child: label),
          if (widget.valueColumn case final col?) ...[
            SizedBox(
              width: col.width,
              child: Row(
                children: [
                  for (var i = 0; i < group.axes.length; i++) ...[
                    if (i > 0) const SizedBox(width: 4),
                    Expanded(
                        child: _cell(transform, group.axes[i], frame,
                            fixed: false)),
                  ],
                ],
              ),
            ),
            SizedBox(width: col.rightInset),
          ] else
            for (final axis in group.axes) ...[
              const SizedBox(width: 6),
              _cell(transform, axis, frame),
            ],
        ],
      ),
    );
    final height = widget.rowHeight;
    return height == null ? row : SizedBox(height: height, child: row);
  }

  Widget _cell(BridgeTransform transform, TransformAxis axis, int frame,
      {bool fixed = true}) {
    final scalar = read(transform, axis.prop);

    // An animated property stays editable (docs/07 §4.3): the field shows the
    // value under the playhead, and a change writes it into the key sitting
    // there — or plants a new one — never flattening the curve.

    if (scalar case BridgeScalar_Expression scalar) {
      return Flexible(
        child: EffectParamRowExpression(
            value: scalar,
            set: (value) {
              final field = (value as BridgeEffectValue_Float).field0;

              if (field is BridgeScalar_Expression) {
                _commitExpression(axis.prop, field.field0);
              }

              if (field is BridgeScalar_Static) {
                _commit(axis.prop, field.field0);
              }
            },
            setLive: (value) {
              _liveExpression(
                  axis.prop,
                  ((value as BridgeEffectValue_Float).field0
                          as BridgeScalar_Expression)
                      .field0);
            },
            comp: widget.comp,
            frame: widget.playheadFrame,
            layer: widget.layer),
      );
    }

    // A rotation shows its whole turns beside its degrees (docs/07 §6.1): 30°
    // and 390° are the same picture but not the same animation, and a single
    // box cannot say which of the two a key holds. The value written is still
    // the one angle — see `TurnsAndDegreesField`.
    final isRotation = axis.suffix == '°';

    if (scalar is! BridgeScalar_Keyframed) {
      final static_ = (scalar as BridgeScalar_Static).field0;
      if (isRotation) {
        return TurnsAndDegreesField(
          keyName: '${widget.keyPrefix}-${axis.prop.name}',
          degrees: static_,
          onChanged: (v) => _live(axis.prop, v),
          onCommit: (v) => _commit(axis.prop, v),
        );
      }
      return SizedBox(
        width: fixed ? transformCellWidth : null,
        child: DragValueField(
          key: ValueKey<String>('${widget.keyPrefix}-${axis.prop.name}'),
          value: static_,
          min: axis.min,
          max: axis.max,
          speed: axis.speed,
          decimals: axis.decimals,
          suffix: axis.suffix,
          onChanged: (v) => _commit(axis.prop, v.toDouble()),
          onChangeStart: () => _staged = transform,
          onChangeLive: (v) => _live(axis.prop, v.toDouble()),
          onChangeEnd: (v) => _commit(axis.prop, v.toDouble()),
          onDragCancel: () {
            _clearLive();
            setState(() => _staged = null);
          },
          setExpression: () {
            _commitExpression(axis.prop, static_.toString());
          },
        ),
      );
    }

    final sampled =
        sampleScalar(scalar: scalar, time: timeOfFrame(widget.comp, frame));
    // No live preview mid-drag: staging a keyframed transform through the
    // static-preview path would lie about the curve. The release commits one
    // op — the key at the playhead updated or planted.
    if (isRotation) {
      return TurnsAndDegreesField(
        keyName: '${widget.keyPrefix}-${axis.prop.name}',
        degrees: sampled,
        onCommit: (v) => _commitKeyed(axis.prop, scalar, v, frame),
      );
    }
    return SizedBox(
      width: fixed ? transformCellWidth : null,
      child: KeyedValueField(
        fieldKey: ValueKey<String>('${widget.keyPrefix}-${axis.prop.name}'),
        value: sampled,
        min: axis.min,
        max: axis.max,
        speed: axis.speed,
        decimals: axis.decimals,
        suffix: axis.suffix,
        onCommit: (v) => _commitKeyed(axis.prop, scalar, v, frame),
        onLive: (v) => _liveKeyed(axis.prop, scalar, v, frame),
        onStart: () => _keyOnDragStart(axis.prop, scalar, sampled, frame),
      ),
    );
  }

  /// The playhead has no key on this property and a drag is starting, so one is
  /// planted there holding the value already showing (K-333). Nothing moves —
  /// it is the same value — and the drag then has a key to carry, which is what
  /// makes it visible in the graph as it goes rather than only on release.
  void _keyOnDragStart(
      BridgeTransformProp prop, BridgeScalar scalar, double value, int frame) {
    if (scalar is! BridgeScalar_Keyframed) return;
    if (scalar.field0
        .any((k) => widget.comp.frameAtTime(time: k.time) == frame)) {
      return;
    }
    widget.layer.setTransform(
      prop: prop,
      value: scalarWithValueAt(scalar, value, widget.comp, frame),
    );
    widget.onChanged();
  }

  /// A tick of a drag on an *animated* property: render the curve the release
  /// will write — the key at the playhead moved, or a linear one planted there
  /// — without writing it (K-333). The same patched-clone door a static drag
  /// uses, carrying a whole animation instead of one number.
  void _liveKeyed(
      BridgeTransformProp prop, BridgeScalar scalar, double value, int frame) {
    rowValueDrag.value = RowValueDrag(
      layer: widget.layer.internallayerId.toString(),
      prop: prop.name,
      frame: frame,
      value: value,
    );
    final staged = writeScalar(
      widget.transform,
      prop,
      scalarWithValueAt(scalar, value, widget.comp, frame),
    );
    // The provisional truth, for anything drawing this layer. A keyed property
    // draws no box today — the boxes want a static value and this one is a
    // curve — so this changes nothing on screen yet; it is published because
    // the contract is "what the drag is doing", not "what the box can use".
    _publishLive(staged);
    final ui = _ui;
    _throttle.request(() => widget.comp.renderFrameWithTransformPreview(
          frame: BigInt.from(ui.playheadFrame.value),
          scale: ui.viewerScale,
          layer: widget.layer,
          transform: staged,
        ));
  }

  /// Write `value` into the animated property's key at `frame` (or plant one
  /// there) — one op, one undo step.
  void _commitKeyed(
      BridgeTransformProp prop, BridgeScalar scalar, double value, int frame) {
    // The write is the last word: a held preview tick after it would put the
    // provisional picture back, and the graph reads the document again.
    _throttle.cancel();
    _clearLive();
    rowValueDrag.value = null;
    widget.layer.setTransform(
      prop: prop,
      value: scalarWithValueAt(scalar, value, widget.comp, frame),
    );
    widget.onChanged();
  }

  /// A drag tick: hold the new value locally and render it, without committing.
  void _live(BridgeTransformProp prop, double value) {
    final staged = write(_staged ?? widget.transform, prop, value);
    setState(() => _staged = staged);
    // The wireframe follows the picture: both are drawn from this value until
    // the drag is let go.
    _publishLive(staged);

    final ui = _ui;
    _throttle.request(() => widget.comp.renderFrameWithTransformPreview(
          frame: BigInt.from(ui.playheadFrame.value),
          scale: ui.viewerScale,
          layer: widget.layer,
          transform: _staged ?? staged,
        ));
  }

  void _liveExpression(BridgeTransformProp prop, String value) {
    final staged = writeExpression(_staged ?? widget.transform, prop, value);
    setState(() => _staged = staged);
    _publishLive(staged);

    final ui = _ui;
    _throttle.request(() => widget.comp.renderFrameWithTransformPreview(
          frame: BigInt.from(ui.playheadFrame.value),
          scale: ui.viewerScale,
          layer: widget.layer,
          transform: _staged ?? staged,
        ));
  }

  /// Release, or a typed value: one op for the one property that changed.
  void _commit(BridgeTransformProp prop, double value) {
    // The commit is the last word on this gesture: a held preview tick after it
    // would put the provisional picture back.
    _throttle.cancel();
    _clearLive();
    widget.layer.setTransform(prop: prop, value: BridgeScalar.static_(value));
    setState(() => _staged = null);
    widget.onChanged();
  }

  void _commitExpression(BridgeTransformProp prop, String value) {
    // The commit is the last word on this gesture: a held preview tick after it
    // would put the provisional picture back.
    _throttle.cancel();
    _clearLive();
    widget.layer
        .setTransform(prop: prop, value: BridgeScalar.expression(value));
    setState(() => _staged = null);
    widget.onChanged();
  }
}

/// One property out of a transform.
BridgeScalar read(BridgeTransform tf, BridgeTransformProp prop) =>
    switch (prop) {
      BridgeTransformProp.anchorX => tf.anchorX,
      BridgeTransformProp.anchorY => tf.anchorY,
      BridgeTransformProp.positionX => tf.positionX,
      BridgeTransformProp.positionY => tf.positionY,
      BridgeTransformProp.positionZ => tf.positionZ,
      BridgeTransformProp.scaleX => tf.scaleX,
      BridgeTransformProp.scaleY => tf.scaleY,
      BridgeTransformProp.rotation => tf.rotation,
      BridgeTransformProp.rotationX => tf.rotationX,
      BridgeTransformProp.rotationY => tf.rotationY,
      BridgeTransformProp.opacity => tf.opacity,
    };

/// A copy of `tf` with one property replaced — what the preview renders.
///
/// Rebuilt field by field because the generated type has no `copyWith`: it is a
/// plain data class across the seam, which is the point of it.
BridgeTransform write(
        BridgeTransform tf, BridgeTransformProp prop, double value) =>
    writeScalar(tf, prop, BridgeScalar.static_(value));

/// A copy of `tf` with one property's whole animation replaced — what a graph
/// drag previews, where the provisional value is a curve rather than a number.
BridgeTransform writeScalar(
    BridgeTransform tf, BridgeTransformProp prop, BridgeScalar replacement) {
  BridgeScalar pick(BridgeTransformProp p, BridgeScalar current) =>
      p == prop ? replacement : current;

  return BridgeTransform(
    anchorX: pick(BridgeTransformProp.anchorX, tf.anchorX),
    anchorY: pick(BridgeTransformProp.anchorY, tf.anchorY),
    positionX: pick(BridgeTransformProp.positionX, tf.positionX),
    positionY: pick(BridgeTransformProp.positionY, tf.positionY),
    positionZ: pick(BridgeTransformProp.positionZ, tf.positionZ),
    scaleX: pick(BridgeTransformProp.scaleX, tf.scaleX),
    scaleY: pick(BridgeTransformProp.scaleY, tf.scaleY),
    rotation: pick(BridgeTransformProp.rotation, tf.rotation),
    rotationX: pick(BridgeTransformProp.rotationX, tf.rotationX),
    rotationY: pick(BridgeTransformProp.rotationY, tf.rotationY),
    opacity: pick(BridgeTransformProp.opacity, tf.opacity),
  );
}

BridgeTransform writeExpression(
        BridgeTransform tf, BridgeTransformProp prop, String expression) =>
    writeScalar(tf, prop, BridgeScalar.expression(expression));
