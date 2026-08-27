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

import 'dart:math' as math;

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
import 'timeline_extras_frb.dart' show showMenuAt;

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

/// One row: its label, the axes it edits, and — on the three properties that
/// have more than one axis — which pair it belongs to and how that pair is
/// being shown (K-571).
class TransformGroup {
  final String label;
  final List<TransformAxis> axes;

  /// The pair this row came out of, or null on Rotation and Opacity, which have
  /// nothing to separate. It is what the row's menu acts on.
  final BridgeTransformPair? pair;

  /// The pair's mode. [BridgeAxisMode.separated] means this row is one axis of
  /// it; [BridgeAxisMode.linked] means the row draws one box for two axes.
  final BridgeAxisMode mode;

  const TransformGroup(
    this.label,
    this.axes, {
    this.pair,
    this.mode = BridgeAxisMode.combined,
  });

  /// A linked row draws one box and carries the other axis with it.
  bool get isLinked => mode == BridgeAxisMode.linked && axes.length > 1;
}

/// The rows a layer shows, in order.
///
/// The 3D rows (Position z, Rotation x, Rotation y) appear only on a 3D layer: a
/// 2D layer showing controls that cannot do anything is worse than not showing
/// them. Exposed as a list rather than built inline because the Timeline has to
/// know *how many* rows a layer will take before it draws them — its lanes have
/// to leave exactly that much room or the bars stop lining up with the names.
///
/// **Separated axes are more rows, not different data** (K-571). The axes are
/// separate scalar properties in the document whatever the mode says; a
/// separated pair simply hands each of them a row of its own, with its own
/// stopwatch, its own lane and its own curve. Which is why every surface that
/// walks this list — the fold-out, the lanes, the graph editor, the Effect
/// controls card — follows without knowing the feature exists.
List<TransformGroup> transformGroups({
  required bool threeD,
  required BridgeAxisModes modes,
}) {
  List<TransformGroup> pairRows(
    BridgeTransformPair pair,
    String label,
    List<String> axisLabels,
    List<TransformAxis> axes,
  ) {
    final mode = switch (pair) {
      BridgeTransformPair.anchor => modes.anchor,
      BridgeTransformPair.position => modes.position,
      BridgeTransformPair.scale => modes.scale,
    };
    if (mode != BridgeAxisMode.separated) {
      return [TransformGroup(label, axes, pair: pair, mode: mode)];
    }
    return [
      for (var i = 0; i < axes.length; i++)
        TransformGroup(axisLabels[i], [axes[i]], pair: pair, mode: mode),
    ];
  }

  return [
    ...pairRows(
      BridgeTransformPair.anchor,
      l10n.transformAnchorPoint,
      [l10n.transformAnchorPointX, l10n.transformAnchorPointY],
      const [
        TransformAxis(BridgeTransformProp.anchorX),
        TransformAxis(BridgeTransformProp.anchorY),
      ],
    ),
    ...pairRows(
      BridgeTransformPair.position,
      l10n.transformPosition,
      [
        l10n.transformPositionX,
        l10n.transformPositionY,
        if (threeD) l10n.transformPositionZ,
      ],
      [
        const TransformAxis(BridgeTransformProp.positionX),
        const TransformAxis(BridgeTransformProp.positionY),
        if (threeD) const TransformAxis(BridgeTransformProp.positionZ),
      ],
    ),
    ...pairRows(
      BridgeTransformPair.scale,
      l10n.transformScale,
      [l10n.transformScaleX, l10n.transformScaleY],
      const [
        TransformAxis(BridgeTransformProp.scaleX, suffix: '%'),
        TransformAxis(BridgeTransformProp.scaleY, suffix: '%'),
      ],
    ),
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
}

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

  /// How each pair is shown (K-571) — which decides how many rows there are.
  final BridgeAxisModes axisModes;
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
    required this.axisModes,
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
        for (final group
            in transformGroups(threeD: threeD, modes: axisModes))
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

  /// The group this row came out of, drawn before its name on a **flat
  /// sheet** (K-499): the dope sheet lists `Transform · Position`, where the
  /// fold-out draws `Position` under the Transform twirl. Null everywhere the
  /// row sits inside its own group.
  final String? nameGroup;

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
    this.nameGroup,
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

  /// A linked row's y:x ratio, taken once per gesture (K-571). Once, because
  /// re-reading it every tick would read the value the tick before had just
  /// written — the ratio would hold trivially and the link would do nothing —
  /// and because sampling costs a crossing the drag does not need to pay
  /// sixty times a second.
  double? _linkRatio;

  /// The partner axis's value for `value` on the leading one, holding the
  /// ratio the pair already had. A leading axis sitting at zero has no ratio to
  /// keep, so the partner simply matches it.
  double _linked(BridgeTransformProp lead, BridgeTransformProp partner,
      double value, int frame) {
    final ratio = _linkRatio ??= () {
      final time = timeOfFrame(widget.comp, frame);
      final x = sampleScalar(scalar: read(widget.transform, lead), time: time);
      final y =
          sampleScalar(scalar: read(widget.transform, partner), time: time);
      return x == 0 ? 1.0 : y / x;
    }();
    return value * ratio;
  }

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
    // A linked row draws one box for two axes (K-571): the second follows the
    // first through [_ratio], so a box for it would be a box that can only ever
    // be told what it already knows. The stopwatch above still covers both.
    final cells = group.isLinked ? group.axes.sublist(0, 1) : group.axes;
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
    // and drawn by whichever layout the row takes. Right-clicking it opens the
    // axis menu (K-571) — on the name rather than the whole row, for the reason
    // the left click is on the name: grabbing a value box must never be a
    // gesture about the property's shape.
    final label = GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: widget.onLabelTap,
      onSecondaryTapUp: group.pair == null
          ? null
          : (details) => _axisMenu(context, details.globalPosition, group),
      child: Row(
        children: [
          ...flatGroupPrefix(t, widget.nameGroup),
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
            for (var i = 0; i < cells.length; i++) ...[
              if (i > 0) const SizedBox(width: 6),
              _cell(transform, cells[i], frame, group: group),
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
              width: _valueAreaWidth(col, cells.length),
              child: Row(
                children: [
                  for (var i = 0; i < cells.length; i++) ...[
                    if (i > 0) const SizedBox(width: 4),
                    Expanded(
                        child: _cell(transform, cells[i], frame,
                            fixed: false, group: group)),
                  ],
                ],
              ),
            ),
            SizedBox(
                width: col.width + col.rightInset -
                    _valueAreaWidth(col, cells.length)),
          ] else
            for (final axis in cells) ...[
              const SizedBox(width: 6),
              _cell(transform, axis, frame, group: group),
            ],
        ],
      ),
    );
    final height = widget.rowHeight;
    return height == null ? row : SizedBox(height: height, child: row);
  }

  /// How much room the row's value boxes take in the Timeline's fold-out.
  ///
  /// **The Modes column is a minimum, not a cage** (owner, desk test). One box
  /// fills the column exactly, as it always has. Two — an unlinked Scale, a
  /// Position — were being squeezed into half of it each, which is narrower
  /// than a value well can hold `100.0%`: the unit dropped to a second line
  /// inside a lane one line tall. A row of more than one box therefore asks
  /// for the width those same boxes are drawn at in the Effect controls panel,
  /// [transformCellWidth] each, and **runs on to the right** rather than
  /// squeezing — into room a property row has going spare anyway, since it
  /// draws no matte, no blend and no parent under those columns.
  ///
  /// Capped at what the outline actually holds ([ValueColumn.rightInset] is
  /// everything to the right of the Modes column), so a narrow outline gives
  /// back the squeeze rather than pushing the boxes off the panel.
  double _valueAreaWidth(ValueColumn col, int cells) {
    final wanted = cells * transformCellWidth + (cells - 1) * 4;
    if (wanted <= col.width) return col.width;
    return math.min(wanted, col.width + col.rightInset);
  }

  Widget _cell(BridgeTransform transform, TransformAxis axis, int frame,
      {bool fixed = true, required TransformGroup group}) {
    final scalar = read(transform, axis.prop);
    // The axis this box drags along with it, or null when it drags alone.
    final partner = group.isLinked ? group.axes[1].prop : null;

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
          onChanged: (v) => _commit(axis.prop, v.toDouble(), partner, frame),
          onChangeStart: () => _staged = transform,
          onChangeLive: (v) => _live(axis.prop, v.toDouble(), partner, frame),
          onChangeEnd: (v) => _commit(axis.prop, v.toDouble(), partner, frame),
          onDragCancel: () {
            _clearLive();
            _linkRatio = null;
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
        onCommit: (v) => _commitKeyed(axis.prop, scalar, v, frame, partner),
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
        onCommit: (v) => _commitKeyed(axis.prop, scalar, v, frame, partner),
        onLive: (v) => _liveKeyed(axis.prop, scalar, v, frame, partner),
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
  void _liveKeyed(BridgeTransformProp prop, BridgeScalar scalar, double value,
      int frame, BridgeTransformProp? partner) {
    rowValueDrag.value = RowValueDrag(
      layer: widget.layer.internallayerId.toString(),
      prop: prop.name,
      frame: frame,
      value: value,
    );
    var staged = writeScalar(
      widget.transform,
      prop,
      scalarWithValueAt(scalar, value, widget.comp, frame),
    );
    // A linked row previews both axes, or the release would move the picture
    // one last time on a drag that looked finished (K-571).
    if (partner != null) {
      staged = writeScalar(
        staged,
        partner,
        scalarWithValueAt(read(widget.transform, partner),
            _linked(prop, partner, value, frame), widget.comp, frame),
      );
    }
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
  /// there) — one op, one undo step. A linked row (K-571) writes its partner
  /// axis in the same breath, and `setTransforms` batches the pair so it stays
  /// one step.
  void _commitKeyed(BridgeTransformProp prop, BridgeScalar scalar, double value,
      int frame, BridgeTransformProp? partner) {
    // The write is the last word: a held preview tick after it would put the
    // provisional picture back, and the graph reads the document again.
    _throttle.cancel();
    _clearLive();
    rowValueDrag.value = null;
    final written = scalarWithValueAt(scalar, value, widget.comp, frame);
    if (partner == null) {
      widget.layer.setTransform(prop: prop, value: written);
    } else {
      widget.layer.setTransforms(props: [
        prop,
        partner
      ], values: [
        written,
        scalarWithValueAt(read(widget.transform, partner),
            _linked(prop, partner, value, frame), widget.comp, frame),
      ]);
    }
    _linkRatio = null;
    widget.onChanged();
  }

  /// A drag tick: hold the new value locally and render it, without committing.
  void _live(BridgeTransformProp prop, double value,
      [BridgeTransformProp? partner, int frame = 0]) {
    var staged = write(_staged ?? widget.transform, prop, value);
    if (partner != null) {
      staged = write(staged, partner, _linked(prop, partner, value, frame));
    }
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

  /// Release, or a typed value: one op for the one property that changed — or,
  /// on a linked row (K-571), one batch for the pair, which is still one step.
  void _commit(BridgeTransformProp prop, double value,
      [BridgeTransformProp? partner, int frame = 0]) {
    // The commit is the last word on this gesture: a held preview tick after it
    // would put the provisional picture back.
    _throttle.cancel();
    _clearLive();
    if (partner == null) {
      widget.layer.setTransform(prop: prop, value: BridgeScalar.static_(value));
    } else {
      widget.layer.setTransforms(props: [
        prop,
        partner
      ], values: [
        BridgeScalar.static_(value),
        BridgeScalar.static_(_linked(prop, partner, value, frame)),
      ]);
    }
    _linkRatio = null;
    setState(() => _staged = null);
    widget.onChanged();
  }

  /// The row's axis menu (K-571): tell the pair's axes apart, or put them back
  /// together. Scale carries the link as well, because a scale that has stopped
  /// being proportional is nearly always a mistake — so it is a state you leave
  /// on purpose rather than one you fall out of.
  ///
  /// Every entry commits one op or one batch, so each is one undo step.
  void _axisMenu(BuildContext context, Offset at, TransformGroup group) {
    final pair = group.pair;
    if (pair == null) return;
    final separated = group.mode == BridgeAxisMode.separated;
    void set(void Function(BridgeAxisMode?) close, BridgeAxisMode mode) {
      close(null);
      widget.layer.setAxisMode(pair: pair, mode: mode);
      widget.onChanged();
    }

    showMenuAt<BridgeAxisMode>(
      context: context,
      position: at,
      rows: (close) => [
        if (pair == BridgeTransformPair.scale && !separated)
          MenuRow(
            key: ValueKey<String>('${widget.keyPrefix}-axis-link'),
            onPressed: () => set(
                close,
                group.mode == BridgeAxisMode.linked
                    ? BridgeAxisMode.combined
                    : BridgeAxisMode.linked),
            child: Text(group.mode == BridgeAxisMode.linked
                ? l10n.transformUnlinkAxes
                : l10n.transformLinkAxes),
          ),
        MenuRow(
          key: ValueKey<String>('${widget.keyPrefix}-axis-separate'),
          onPressed: () => set(
              close,
              separated
                  ? (pair == BridgeTransformPair.scale
                      ? BridgeAxisMode.linked
                      : BridgeAxisMode.combined)
                  : BridgeAxisMode.separated),
          child: Text(separated
              ? l10n.transformCombineAxes
              : l10n.transformSeparateAxes),
        ),
      ],
    );
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
