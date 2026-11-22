// The Animators section: a Text layer's letters moved one at a time.
//
// An animator names a set of per-letter offsets — a push, a turn, a size, a
// fade, a tint, a box — and a **range selector** saying which letters they
// reach, or the spoken words when a caption hands them in. The
// selector hands every letter a weight and the offsets are applied times that
// weight, so keying the selector's Offset sweeps the range across the words and
// every letter is moved in its turn. That is the cascade, and nothing in this
// file knows it: the weights, the composition of two animators and the drawing
// are all the engine's (the thin-view rule).
//
// Every animator carries all five property groups, defaulted to values that
// change nothing, which is why there is no menu of properties to add them
// from one at a time. So Add animator gives the rows, and four of them are
// usually left alone.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../widgets/controls.dart';
import 'fx_key_strip.dart';
import 'fx_section.dart';
import 'keyframe_controls_frb.dart';
import 'param_row_menu.dart';
import 'text_animation_presets.dart'
    show TextAnimationPreset, textPresetAnimators;
import 'layer_fold_frb.dart' show animatorsPath;
import '../state/font_axes.dart';
import 'timeline_extras_frb.dart' show showMenuAt;
import 'timeline_mask_rows_frb.dart' show RowButton;
import 'text_animator_tools.dart';

export 'text_animator_tools.dart';

/// Matches the Effect controls panel's own cell width, so the values line up
/// down the panel.
const double _cellWidth = 78;

/// The Animators section for [layer], or nothing when it is not a Text layer.
class TextAnimatorRowsFrb extends StatefulWidget {
  final LayerReference layer;
  final VoidCallback onChanged;

  /// The comp and playhead: every number here is keyable, so every row carries
  /// the stopwatch and the ◄ ◆ ► navigator.
  final CompositionReference comp;
  final int playheadFrame;
  final ValueChanged<int> onSeek;

  /// Whether the section is twirled open, and how to toggle it — held by the
  /// panel so the open set survives a rebuild.
  final bool open;
  final VoidCallback onToggle;

  /// A solid layer's letters also move in depth and turn in 3D.
  final bool solid;

  /// Which panel the rows are in, so a rename opens only there.
  final String renameScope;

  const TextAnimatorRowsFrb({
    super.key,
    required this.layer,
    required this.onChanged,
    required this.comp,
    required this.playheadFrame,
    required this.onSeek,
    required this.open,
    required this.onToggle,
    this.solid = false,
    this.renameScope = 'fx',
  });

  @override
  State<TextAnimatorRowsFrb> createState() => _TextAnimatorRowsFrbState();
}

class _TextAnimatorRowsFrbState extends State<TextAnimatorRowsFrb> {
  /// The animators from the last read model that held the layer, null when it
  /// is not a Text layer. A layer of a comp no longer in front is not in the
  /// model, and keeps what was last read, as the panel's other sections do.
  List<BridgeTextAnimator>? _held;

  /// The layer's style, which names the font axes its animators move. Read
  /// once for each new set of animators with an axis on it, not on every draw.
  BridgeTextStyle? _style;
  List<BridgeTextAnimator>? _styleFor;

  @override
  Widget build(BuildContext context) {
    // Drawn from the read model. Only the style is asked of the engine.
    final entry = Provider.of<LumitUiState>(context, listen: false)
        .model
        .byId(widget.layer.internallayerId);
    if (entry != null) {
      _held = entry.info.kind == BridgeLayerKind.text
          ? entry.info.textAnimators
          : null;
    }
    final animators = _held;
    if (animators == null) return const SizedBox.shrink();
    if (!identical(animators, _styleFor) &&
        animators.any((a) => a.axes.isNotEmpty)) {
      _styleFor = animators;
      _style = widget.layer.getText()?.style;
    }
    return ValueListenableBuilder<int>(
      // The font's axes come in from the engine a moment after the first
      // draw, and name the axis rows.
      valueListenable: fontAxesArrived,
      builder: (context, _, __) =>
          _AnimatorSection(of: widget, animators: animators, style: _style),
    );
  }
}

/// The section itself, for the animators [TextAnimatorRowsFrb] holds.
class _AnimatorSection extends StatelessWidget {
  final TextAnimatorRowsFrb of;
  final List<BridgeTextAnimator> animators;
  final BridgeTextStyle? style;

  const _AnimatorSection(
      {required this.of, required this.animators, required this.style});

  LayerReference get layer => of.layer;
  VoidCallback get onChanged => of.onChanged;
  CompositionReference get comp => of.comp;
  int get playheadFrame => of.playheadFrame;
  ValueChanged<int> get onSeek => of.onSeek;
  bool get open => of.open;
  VoidCallback get onToggle => of.onToggle;
  bool get solid => of.solid;

  @override
  Widget build(BuildContext context) {
    // One write path: read the document, change the animator list, write it
    // whole. The engine takes the whole document as one op — and when the list
    // crosses empty ↔ not-empty it moves the layer's anchor by the margin the
    // animated raster adds, in the same op, so the words do not jump. Read
    // when the edit is made, never while drawing.
    void write(List<BridgeTextAnimator> animators) {
      final document = layer.getText();
      if (document == null) return;
      // What the panels show about each animator moves with it.
      followAnimators(Provider.of<LumitUiState>(context, listen: false), layer,
          document.animators, animators);
      layer.setText(
        document: BridgeTextDocument(
          text: document.text,
          expression: document.expression,
          keys: document.keys,
          size: document.size,
          fill: document.fill,
          path: document.path,
          pathOffset: document.pathOffset,
          style: document.style,
          paragraph: document.paragraph,
          animators: animators,
        ),
      );
      onChanged();
    }

    // Every row of every animator is a row of the section, so each keeps the
    // section's own spacing and hairline; a folded animator or a shut
    // Advanced leaves its rows out.
    return ListenableBuilder(
      listenable: Listenable.merge([advancedOpen, animatorFolded]),
      builder: (context, _) => _section(context, write),
    );
  }

  Widget _section(
      BuildContext context, ValueChanged<List<BridgeTextAnimator>> write) {
    final rows = <Widget>[];
    for (var i = 0; i < animators.length; i++) {
      rows.addAll(_animatorRows(
        context,
        animators,
        i,
        write,
        style,
      ));
    }
    return FxSection(
      title: l10n.textAnimatorsSection,
      scope: animatorsPath(layer.internallayerId.toString()),
      open: open,
      onToggle: onToggle,
      // Add animator on the section's own heading, as on the Timeline's
      // Animators row: it adds an animator, where an animator's Add adds to
      // that animator.
      actions: [
        Flexible(
            child: RowButton(
          key: const ValueKey('text-animator-add'),
          label: l10n.textAnimatorAdd,
          arrow: true,
          onTap: (at) async {
            final style = layer.getText()?.style;
            if (style != null) await fontAxesReady(style.family, style.face);
            if (!context.mounted) return;
            showMenuAt<void>(
              context: context,
              position: at,
              width: 190,
              rows: (close) => animateMenuRows(
                close: () => close(null),
                layer: layer,
                solid: solid,
                onChanged: onChanged,
                comp: comp,
                frame: Provider.of<LumitUiState>(context, listen: false)
                    .playheadFrame
                    .value,
              ),
            );
          },
        )),
      ],
      rows: rows,
    );
  }

  /// One animator: its heading, its own Amount and Link, its selectors top
  /// to bottom, then its property groups in the order they read — where the
  /// letter is, how it is turned, how big, how faded, what colour.
  List<Widget> _animatorRows(
    BuildContext context,
    List<BridgeTextAnimator> animators,
    int index,
    ValueChanged<List<BridgeTextAnimator>> write,
    BridgeTextStyle? style,
  ) {
    final a = animators[index];

    void put(BridgeTextAnimator next) => write([
          for (var i = 0; i < animators.length; i++)
            if (i == index) next else animators[i],
        ]);
    final others = [
      for (var i = 0; i < animators.length; i++)
        if (i != index) animators[i].name,
    ];

    return [
      AnimatorHeading(
        key: ValueKey<String>('text-animator-heading-$index'),
        layer: layer,
        comp: comp,
        animators: animators,
        index: index,
        write: write,
        solid: solid,
        renameScope: of.renameScope,
      ),
      // A folded animator shows its heading only.
      if (!animatorFolded.value.contains(animatorFoldKey(layer, index)))
        for (final row in <Widget>[
            _numberRow(
                context,
                l10n.textAnimatorAnimatorAmount,
                'anim-amount-$index',
                [a.amount],
                0,
                100,
                100,
                (s) => put(animatorWith(a, amount: s.first))),
            // Link: another animator's selectors pick these letters too.
            if (others.isNotEmpty || a.follow != null)
              _plainRow(
                context,
                l10n.textAnimatorLink,
                BareDropdown<String?>(
                  key: ValueKey<String>('text-animator-link-$index'),
                  value: a.follow,
                  options: [
                    null,
                    ...others.toSet(),
                    if (a.follow != null && !others.contains(a.follow))
                      a.follow,
                  ],
                  label: (name) => name ?? l10n.none,
                  onChanged: (name) => put(name == null
                      ? animatorWith(a, clearFollow: true)
                      : animatorWith(a, follow: name)),
                ),
              ),
            if (a.follow != null)
              _plainRow(
                context,
                l10n.textAnimatorLinkOffset,
                DragValueField(
                  key: ValueKey<String>('text-animator-link-offset-$index'),
                  value: a.followOffset,
                  min: -60,
                  max: 60,
                  decimals: 2,
                  speed: 0.01,
                  suffix: ' s',
                  resetTo: 0,
                  onChanged: (v) =>
                      put(animatorWith(a, followOffset: v.toDouble())),
                ),
              ),
          // A linked animator reads the other's selectors, so its own are
          // set aside, faint, rather than hidden.
          for (var k = 0; k < selectorCount(a); k++)
            for (final row in _selectorRows(context, a, index, k, put))
              a.follow == null ? row : Opacity(opacity: 0.45, child: row),
          ..._propertyRows(context, animators, index, put, style),
        ])
          // A switched-off animator keeps its rows, faint.
          a.enabled ? row : Opacity(opacity: 0.45, child: row),
    ];
  }

  /// Selector [k] of animator [index]: a heading once there is more than
  /// one, what it follows, its open rows, then its Advanced ones.
  List<Widget> _selectorRows(
    BuildContext context,
    BridgeTextAnimator a,
    int index,
    int k,
    ValueChanged<BridgeTextAnimator> put,
  ) {
    final t = ThemeScope.of(context).theme;
    final s = selectorAt(a, k);
    void putSelector(BridgeRangeSelector next) =>
        put(animatorWithSelectorAt(a, k, next));
    final tag = k == 0 ? '$index' : '$index-s$k';
    final advancedKey = '${layer.internallayerId}/$index/$k';
    final items = selectorItemsOf(s, k);
    Widget item(SelectorItem item, {double indent = animatorRowIndent}) {
      final idle = selectorItemIdle(s, item);
      final row = switch (item) {
        SelectorSettingItem(:final setting) => _plainRow(
            context,
            animatorSettingLabel(setting),
            animatorSettingControl(
              context,
              layer: layer,
              s: s,
              k: k,
              setting: setting,
              put: putSelector,
              keyName: '${_settingKey(setting, s)}-$tag',
            ),
            indent: indent),
        SelectorValueItem(:final value) => _numberRow(
            context,
            textAnimatorValueLabel(value, a),
            '${_valueKeys[value] ?? value.name}-$tag',
            [selectorScalarOf(s, value)],
            textAnimatorValueMin(value),
            textAnimatorValueMax(value),
            textAnimatorDefault(value),
            (v) => putSelector(selectorWithScalar(s, value, v.first)),
            decimals: textAnimatorValueDecimals(value),
            scale: value == TextAnimatorValue.staggerDelay
                ? delayScale(s, comp)
                : 1,
            trailing: value == TextAnimatorValue.staggerAmount ||
                    value == TextAnimatorValue.easeHigh
                ? SelectorCurvePreview(selector: s)
                : null,
            indent: indent,
          ),
      };
      return idle ? Opacity(opacity: 0.4, child: row) : row;
    }

    return [
      if (selectorCount(a) > 1)
        fxTwoColumnRow(
          context: context,
          name: Padding(
            padding:
                const EdgeInsetsDirectional.only(start: animatorRowIndent),
            child: Text(t.kickerCase(l10n.textAnimatorSelectorNumber(k + 1)),
                style: t.kicker, overflow: TextOverflow.ellipsis),
          ),
          control: k == 0
              ? const SizedBox.shrink()
              : Align(
                  alignment: Alignment.centerLeft,
                  child: HouseButton(
                    key: ValueKey<String>('text-animator-remove-selector-$tag'),
                    small: true,
                    frameless: true,
                    onPressed: () => put(animatorWithoutSelector(a, k)),
                    child: Text(l10n.textAnimatorRemoveSelector),
                  ),
                ),
        ),
      _plainRow(
        context,
        l10n.textAnimatorFollows,
        BareDropdown<BridgeSelectorMode>(
          key: ValueKey<String>('text-animator-mode-$tag'),
          value: s.mode,
          options: BridgeSelectorMode.values,
          label: (m) => switch (m) {
            BridgeSelectorMode.range => l10n.textAnimatorFollowsRange,
            BridgeSelectorMode.current => l10n.textAnimatorFollowsCurrent,
            BridgeSelectorMode.said => l10n.textAnimatorFollowsSaid,
            BridgeSelectorMode.unsaid => l10n.textAnimatorFollowsUnsaid,
            BridgeSelectorMode.emphasis => l10n.textAnimatorFollowsEmphasis,
          },
          onChanged: (m) => putSelector(selectorWith(s, mode: m)),
        ),
      ),
      // Spoken words take an ease instead of a range.
      if (s.mode != BridgeSelectorMode.range &&
          s.mode != BridgeSelectorMode.emphasis)
        _plainRow(
          context,
          l10n.textAnimatorEase,
          DragValueField(
            key: ValueKey<String>('text-animator-ease-$tag'),
            value: s.ease,
            min: 0,
            max: 10,
            decimals: 2,
            speed: 0.01,
            suffix: ' s',
            resetTo: 0,
            onChanged: (v) =>
                putSelector(selectorWith(s, ease: v.toDouble())),
          ),
        ),
      for (final i in items)
        if (!i.advanced) item(i),
      // Each Advanced row is a row of the section too, so it keeps the
      // section's spacing; the panel rebuilds when Advanced opens or shuts.
      if (items.any((i) => i.advanced)) ...[
        fxTwoColumnRow(
          context: context,
          name: Padding(
            padding:
                const EdgeInsetsDirectional.only(start: animatorRowIndent),
            child: GestureDetector(
              key: ValueKey<String>('text-animator-advanced-$tag'),
              behavior: HitTestBehavior.opaque,
              onTap: () => toggleAdvanced(advancedKey),
              child: Text(
                  '${advancedOpen.value.contains(advancedKey) ? '▾' : '▸'} '
                  '${t.propertyCase(l10n.textAnimatorAdvanced)}',
                  style: t.body,
                  overflow: TextOverflow.ellipsis),
            ),
          ),
          control: const SizedBox.shrink(),
        ),
        if (advancedOpen.value.contains(advancedKey))
          for (final i in items)
            if (i.advanced) item(i, indent: animatorRowIndent * 2),
      ],
    ];
  }

  /// The property groups the animator shows: those Add ▸ Property put on it
  /// and any whose numbers have been moved, then its font axes.
  List<Widget> _propertyRows(
    BuildContext context,
    List<BridgeTextAnimator> animators,
    int index,
    ValueChanged<BridgeTextAnimator> put,
    BridgeTextStyle? style,
  ) {
    final a = animators[index];
    Widget row(AnimatorProperty p, String label, String key) {
      final values = p.numbers;
      return _numberRow(
          context,
          label,
          '$key-$index',
          [for (final v in values) textAnimatorScalarOf(a, v)],
          textAnimatorValueMin(values.first),
          textAnimatorValueMax(values.first),
          textAnimatorDefault(values.first),
          (s) {
            var next = a;
            for (var i = 0; i < values.length; i++) {
              next = animatorWithScalar(next, values[i], s[i]);
            }
            put(next);
          },
          decimals: textAnimatorValueDecimals(values.first));
    }

    return [
      for (final p in AnimatorProperty.values)
        if ((solid || !p.solidOnly) && animatorShowsProperty(a, p))
          switch (p) {
            AnimatorProperty.position =>
              row(p, l10n.transformPosition, 'anim-position'),
            AnimatorProperty.positionZ =>
              row(p, '${l10n.transformPosition} z', 'anim-position-z'),
            AnimatorProperty.rotation =>
              row(p, l10n.transformRotation, 'anim-rotation'),
            AnimatorProperty.rotationX =>
              row(p, '${l10n.transformRotation} x', 'anim-rotation-x'),
            AnimatorProperty.rotationY =>
              row(p, '${l10n.transformRotation} y', 'anim-rotation-y'),
            AnimatorProperty.scale => row(p, l10n.transformScale, 'anim-scale'),
            AnimatorProperty.opacity =>
              row(p, l10n.transformOpacity, 'anim-opacity'),
            // A fill **offset**, added to the layer's own colour in
            // scene-linear, so it can be negative and two animators add up.
            AnimatorProperty.fill =>
              row(p, l10n.textAnimatorFillOffset, 'anim-fill'),
            // A box behind the letters reached, in a colour of its own.
            AnimatorProperty.box => Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  _numberRow(
                      context,
                      l10n.textAnimatorBoxColour,
                      'anim-box-$index',
                      [a.boxR, a.boxG, a.boxB],
                      0,
                      10,
                      0,
                      (s) => put(animatorWith(a,
                          boxR: s[0], boxG: s[1], boxB: s[2]))),
                  _numberRow(
                      context,
                      l10n.textAnimatorBoxOpacity,
                      'anim-box-opacity-$index',
                      [a.boxOpacity],
                      0,
                      100,
                      0,
                      (s) => put(animatorWith(a, boxOpacity: s.first))),
                ],
              ),
            AnimatorProperty.skew => Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  _numberRow(
                      context,
                      l10n.textAnimatorSkew,
                      'anim-skew-$index',
                      [a.skew],
                      -85,
                      85,
                      0,
                      (s) => put(animatorWith(a, skew: s.first))),
                  _numberRow(
                      context,
                      l10n.textAnimatorSkewAxis,
                      'anim-skew-axis-$index',
                      [a.skewAxis],
                      -100000,
                      100000,
                      0,
                      (s) => put(animatorWith(a, skewAxis: s.first))),
                ],
              ),
            AnimatorProperty.tracking =>
              row(p, l10n.textAnimatorTracking, 'anim-tracking'),
            AnimatorProperty.blur => row(p, l10n.textAnimatorBlur, 'anim-blur'),
            AnimatorProperty.characterOffset => row(
                p, l10n.textAnimatorCharacterOffset, 'anim-character-offset'),
          },
      // Variable font axes: where a reached letter is set on the font's own
      // axes, a place rather than an amount.
      for (var k = 0; k < a.axes.length; k++)
        _axisRow(context, animators, index, k, put, style),
    ];
  }

  /// One axis an animator moves the letters along: a keyable place, and a
  /// way to take it off.
  Widget _axisRow(
    BuildContext context,
    List<BridgeTextAnimator> animators,
    int index,
    int k,
    ValueChanged<BridgeTextAnimator> put,
    BridgeTextStyle? style,
  ) {
    final a = animators[index];
    final axis = a.axes[k];
    final family = style?.family ?? '';
    final face = style?.face ?? '';
    final (lo, hi) = fontAxisRange(family, face, axis.tag);
    void putAxes(List<BridgeAnimatorAxis> axes) =>
        put(animatorWith(a, axes: axes));
    return Row(
      children: [
        Expanded(
          child: _numberRow(
              context,
              l10n.textAnimatorAxisLabel(fontAxisName(family, face, axis.tag)),
              'anim-axis-${axis.tag}-$index',
              [axis.value],
              lo,
              hi,
              fontAxisFace(family, face, axis.tag),
              (s) => putAxes([
                    for (var i = 0; i < a.axes.length; i++)
                      if (i == k)
                        BridgeAnimatorAxis(tag: axis.tag, value: s.first)
                      else
                        a.axes[i],
                  ])),
        ),
        HouseButton(
          key: ValueKey<String>('anim-axis-remove-${axis.tag}-$index'),
          small: true,
          frameless: true,
          onPressed: () => putAxes([
            for (var i = 0; i < a.axes.length; i++)
              if (i != k) a.axes[i],
          ]),
          child: const Text('×'),
        ),
      ],
    );
  }

  /// A keyable row: the stopwatch and navigator, the name, and one scrub field
  /// per channel — a two-axis property draws two, the fill offset three.
  Widget _numberRow(
    BuildContext context,
    String label,
    String keyName,
    List<BridgeScalar> scalars,
    double min,
    double max,
    double resetTo,
    ValueChanged<List<BridgeScalar>> write, {
    int decimals = 1,
    double scale = 1,
    Widget? trailing,
    double indent = animatorRowIndent,
  }) {
    // The playhead is listened to here, per row, rather than by the panel above
    // — and only where a channel can actually move under it. A card that
    // redrew whole on every frame of a scrub is what made the playhead lag.
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;
    if (scalars.every((s) => s is BridgeScalar_Static)) {
      return _numberRowAt(context, label, keyName, scalars, min, max, resetTo,
          write, playhead.value, decimals, scale, trailing, indent);
    }
    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, at, _) => _numberRowAt(context, label, keyName,
          scalars, min, max, resetTo, write, at, decimals, scale, trailing,
          indent),
    );
  }

  Widget _numberRowAt(
    BuildContext context,
    String label,
    String keyName,
    List<BridgeScalar> scalars,
    double min,
    double max,
    double resetTo,
    ValueChanged<List<BridgeScalar>> write,
    int at,
    int decimals, [
    double scale = 1,
    Widget? trailing,
    double indent = animatorRowIndent,
  ]) {
    final t = ThemeScope.of(context).theme;
    // An animated channel shows what its curve reads at the playhead, sampled
    // engine-side — the same answer the render will use, rather than a second
    // interpolation living in the view.
    double shown(BridgeScalar s) => switch (s) {
          BridgeScalar_Static(:final field0) => field0,
          BridgeScalar_Keyframed() ||
          BridgeScalar_Expression() =>
            sampledScalar(s, timeOfFrame(comp, at)),
        };
    // A Delay shown in frames is kept in seconds: shown times [scale],
    // written back divided by it.
    void commit(int axis, num value) => write([
          for (var i = 0; i < scalars.length; i++)
            if (i == axis)
              scalarWithValueAt(scalars[i], value.toDouble() / scale, comp, at)
            else
              scalars[i],
        ]);

    final keys = KeyframeControlsFrb(
      scalars: scalars,
      onWrite: write,
      comp: comp,
      onSeek: onSeek,
      rowKey: keyName,
      // These rows only ever draw in the Effect controls panel, on its fixed
      // columns.
      fixedColumns: true,
    );
    return ParamRowMenu(
      menu: scalarRowMenu(context, keys,
          defaults: List.filled(scalars.length, resetTo)),
      child: fxTwoColumnRow(
        context: context,
        keyframeControls: keys,
        lane: FxKeyLane(rowKey: keyName, scalars: scalars, onWrite: write),
        name: Padding(
          padding: EdgeInsetsDirectional.only(start: indent),
          child: Text(t.propertyCase(label),
              style: t.body, overflow: TextOverflow.ellipsis),
        ),
        control: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (var i = 0; i < scalars.length; i++) ...[
              if (i > 0) const SizedBox(width: 6),
              // Three values in a narrow panel give way, rather than run off it.
              Flexible(
                child: SizedBox(
                  width: _cellWidth,
                  child: DragValueField(
                    key: ValueKey<String>('$keyName-$i'),
                    value: shown(scalars[i]) * scale,
                    min: min * scale,
                    max: max * scale,
                    resetTo: resetTo * scale,
                    decimals: decimals,
                    onChanged: (v) => commit(i, v),
                  ),
                ),
              ),
            ],
            if (trailing != null) ...[const SizedBox(width: 6), trailing],
          ],
        ),
      ),
    );
  }

  /// A row whose control is a choice, not a curve — so a plain name, with no
  /// stopwatch to promise a keyframe that cannot exist.
  Widget _plainRow(BuildContext context, String label, Widget control,
      {double indent = animatorRowIndent}) {
    final t = ThemeScope.of(context).theme;
    return fxTwoColumnRow(
      context: context,
      name: Padding(
        padding: EdgeInsetsDirectional.only(start: indent),
        child: Text(t.propertyCase(label),
            style: t.body, overflow: TextOverflow.ellipsis),
      ),
      control: SizedBox(width: _cellWidth + 60, child: control),
    );
  }
}

/// The test keys the rows have always had, by number.
const _valueKeys = {
  TextAnimatorValue.rangeStart: 'range-start',
  TextAnimatorValue.rangeEnd: 'range-end',
  TextAnimatorValue.rangeOffset: 'range-offset',
  TextAnimatorValue.easeHigh: 'range-ease-high',
  TextAnimatorValue.easeLow: 'range-ease-low',
  TextAnimatorValue.radius: 'range-radius',
  TextAnimatorValue.maxAmount: 'wiggly-max',
  TextAnimatorValue.minAmount: 'wiggly-min',
  TextAnimatorValue.wiggles: 'wiggly-rate',
  TextAnimatorValue.correlation: 'wiggly-correlation',
  TextAnimatorValue.staggerAmount: 'stagger-amount',
  TextAnimatorValue.staggerDelay: 'stagger-delay',
  TextAnimatorValue.staggerDelayEase: 'stagger-delay-ease',
  TextAnimatorValue.staggerBounceAmplitude: 'stagger-bounce-amp',
  TextAnimatorValue.staggerBounceFrequency: 'stagger-bounce-freq',
  TextAnimatorValue.staggerBounceDecay: 'stagger-bounce-decay',
};

/// The least one of an animator's numbers is dragged to.
double textAnimatorValueMin(TextAnimatorValue v) => switch (v) {
      TextAnimatorValue.easeHigh ||
      TextAnimatorValue.easeLow ||
      TextAnimatorValue.staggerDelayEase ||
      TextAnimatorValue.minAmount ||
      TextAnimatorValue.maxAmount =>
        -100,
      TextAnimatorValue.staggerAmount => -200,
      TextAnimatorValue.rangeStart ||
      TextAnimatorValue.rangeEnd ||
      TextAnimatorValue.rangeOffset ||
      TextAnimatorValue.characterOffset =>
        -1000,
      TextAnimatorValue.skew => -85,
      TextAnimatorValue.scaleX || TextAnimatorValue.scaleY => -10000,
      TextAnimatorValue.fillR ||
      TextAnimatorValue.fillG ||
      TextAnimatorValue.fillB =>
        -10,
      TextAnimatorValue.positionX ||
      TextAnimatorValue.positionY ||
      TextAnimatorValue.positionZ ||
      TextAnimatorValue.rotation ||
      TextAnimatorValue.rotationX ||
      TextAnimatorValue.rotationY ||
      TextAnimatorValue.skewAxis ||
      TextAnimatorValue.tracking =>
        -100000,
      _ => 0,
    };

/// The most one of an animator's numbers is dragged to.
double textAnimatorValueMax(TextAnimatorValue v) => switch (v) {
      TextAnimatorValue.easeHigh ||
      TextAnimatorValue.easeLow ||
      TextAnimatorValue.staggerDelayEase ||
      TextAnimatorValue.minAmount ||
      TextAnimatorValue.maxAmount ||
      TextAnimatorValue.correlation ||
      TextAnimatorValue.opacity ||
      TextAnimatorValue.boxOpacity ||
      TextAnimatorValue.animatorAmount =>
        100,
      TextAnimatorValue.staggerAmount => 200,
      TextAnimatorValue.staggerDelay => 10,
      TextAnimatorValue.staggerBounceAmplitude ||
      TextAnimatorValue.staggerBounceFrequency ||
      TextAnimatorValue.staggerBounceDecay =>
        20,
      TextAnimatorValue.rangeStart ||
      TextAnimatorValue.rangeEnd ||
      TextAnimatorValue.rangeOffset ||
      TextAnimatorValue.wiggles ||
      TextAnimatorValue.characterOffset ||
      TextAnimatorValue.blur =>
        1000,
      TextAnimatorValue.skew => 85,
      TextAnimatorValue.scaleX || TextAnimatorValue.scaleY => 10000,
      TextAnimatorValue.fillR ||
      TextAnimatorValue.fillG ||
      TextAnimatorValue.fillB ||
      TextAnimatorValue.boxR ||
      TextAnimatorValue.boxG ||
      TextAnimatorValue.boxB =>
        10,
      _ => 100000,
    };

/// How many decimals one of an animator's numbers shows.
int textAnimatorValueDecimals(TextAnimatorValue v) => switch (v) {
      TextAnimatorValue.staggerDelay ||
      TextAnimatorValue.staggerBounceAmplitude ||
      TextAnimatorValue.staggerBounceFrequency ||
      TextAnimatorValue.staggerBounceDecay =>
        2,
      TextAnimatorValue.characterOffset => 0,
      _ => 1,
    };

/// Give a Text layer one more animator: the card's *Add animator*
/// button, and Animation ▸ Animate text, which must mean the same thing.
///
/// The whole document is rewritten because that is the only shape the engine
/// takes — and adding the **first** animator moves the layer's anchor in the
/// same op, so the words do not jump. Returns whether one was added: a layer
/// that is not Type has no document to add to, and says so by doing nothing.
/// [adding] is the animators to add, a blank one if there are none.
bool addTextAnimator(LayerReference layer,
    {List<BridgeTextAnimator> adding = const []}) {
  final document = layer.getText();
  if (document == null) return false;
  layer.setText(
    document: BridgeTextDocument(
      text: document.text,
      expression: document.expression,
      keys: document.keys,
      size: document.size,
      fill: document.fill,
      path: document.path,
      pathOffset: document.pathOffset,
      style: document.style,
      paragraph: document.paragraph,
      animators: [
        ...document.animators,
        ...adding,
        if (adding.isEmpty)
          freshTextAnimator(animatorNameFor(document.animators)),
      ],
    ),
  );
  return true;
}

/// A new animator's name, the [n]th, with what it was added for after it
/// in brackets: "Animator 1 (Position)". Renamed, it is whatever is typed.
/// Numbered after the [existing] ones, and never a name one of them has: one
/// taken out leaves its number free, and the next is numbered past it.
String animatorNameFor(List<BridgeTextAnimator> existing, [String? what]) {
  final taken = {for (final a in existing) a.name};
  for (var n = existing.length + 1;; n++) {
    final base = l10n.textAnimatorDefaultName(n);
    final name = what == null ? base : '$base ($what)';
    if (!taken.contains(name)) return name;
  }
}

/// A fresh animator: the whole set of properties, every one of them at a value
/// that changes nothing until it is moved.
BridgeTextAnimator freshTextAnimator(String name) => BridgeTextAnimator(
      name: name,
      selector: freshSelector(),
      selectors: const [],
      enabled: true,
      solo: false,
      amount: const BridgeScalar.static_(100),
      followOffset: 0,
      skew: const BridgeScalar.static_(0),
      skewAxis: const BridgeScalar.static_(0),
      tracking: const BridgeScalar.static_(0),
      blur: const BridgeScalar.static_(0),
      characterOffset: const BridgeScalar.static_(0),
      positionX: const BridgeScalar.static_(0),
      positionY: const BridgeScalar.static_(0),
      rotation: const BridgeScalar.static_(0),
      positionZ: const BridgeScalar.static_(0),
      rotationX: const BridgeScalar.static_(0),
      rotationY: const BridgeScalar.static_(0),
      scaleX: const BridgeScalar.static_(100),
      scaleY: const BridgeScalar.static_(100),
      opacity: const BridgeScalar.static_(100),
      fillR: const BridgeScalar.static_(0),
      fillG: const BridgeScalar.static_(0),
      fillB: const BridgeScalar.static_(0),
      boxOpacity: const BridgeScalar.static_(0),
      boxR: BridgeScalar.static_(textAnimatorDefault(TextAnimatorValue.boxR)),
      boxG: BridgeScalar.static_(textAnimatorDefault(TextAnimatorValue.boxG)),
      boxB: BridgeScalar.static_(textAnimatorDefault(TextAnimatorValue.boxB)),
      axes: const [],
      // Nothing listed yet: Add ▸ Property builds it up, as After Effects'
      // blank animator.
      properties: const [],
    );

/// Whether any of [a]'s rows are keyed, any selector's included.
bool animatorHasKeys(BridgeTextAnimator a) => firstKeySeconds([a]) != null;

/// [animator] with one channel replaced — the generated struct is immutable and
/// every field required, so there is one copy helper rather than eleven
/// constructor calls spread through the rows.
BridgeTextAnimator animatorWith(
  BridgeTextAnimator a, {
  String? name,
  BridgeRangeSelector? selector,
  List<BridgeRangeSelector>? selectors,
  bool? enabled,
  bool? solo,
  BridgeScalar? amount,
  String? follow,
  bool clearFollow = false,
  double? followOffset,
  BridgeScalar? skew,
  BridgeScalar? skewAxis,
  BridgeScalar? tracking,
  BridgeScalar? blur,
  BridgeScalar? characterOffset,
  BridgeScalar? positionX,
  BridgeScalar? positionY,
  BridgeScalar? rotation,
  BridgeScalar? positionZ,
  BridgeScalar? rotationX,
  BridgeScalar? rotationY,
  BridgeScalar? scaleX,
  BridgeScalar? scaleY,
  BridgeScalar? opacity,
  BridgeScalar? fillR,
  BridgeScalar? fillG,
  BridgeScalar? fillB,
  BridgeScalar? boxOpacity,
  BridgeScalar? boxR,
  BridgeScalar? boxG,
  BridgeScalar? boxB,
  List<BridgeAnimatorAxis>? axes,
  List<String>? properties,
}) =>
    BridgeTextAnimator(
      name: name ?? a.name,
      selector: selector ?? a.selector,
      selectors: selectors ?? a.selectors,
      enabled: enabled ?? a.enabled,
      solo: solo ?? a.solo,
      amount: amount ?? a.amount,
      follow: clearFollow ? null : (follow ?? a.follow),
      followOffset: followOffset ?? a.followOffset,
      skew: skew ?? a.skew,
      skewAxis: skewAxis ?? a.skewAxis,
      tracking: tracking ?? a.tracking,
      blur: blur ?? a.blur,
      characterOffset: characterOffset ?? a.characterOffset,
      positionX: positionX ?? a.positionX,
      positionY: positionY ?? a.positionY,
      rotation: rotation ?? a.rotation,
      positionZ: positionZ ?? a.positionZ,
      rotationX: rotationX ?? a.rotationX,
      rotationY: rotationY ?? a.rotationY,
      scaleX: scaleX ?? a.scaleX,
      scaleY: scaleY ?? a.scaleY,
      opacity: opacity ?? a.opacity,
      fillR: fillR ?? a.fillR,
      fillG: fillG ?? a.fillG,
      fillB: fillB ?? a.fillB,
      boxOpacity: boxOpacity ?? a.boxOpacity,
      boxR: boxR ?? a.boxR,
      boxG: boxG ?? a.boxG,
      boxB: boxB ?? a.boxB,
      axes: axes ?? a.axes,
      properties: properties ?? a.properties,
    );

/// Which of an animator's animatable numbers a row carries.
///
/// The three range numbers first, because they decide *which letters* the rest
/// reach, then the five property groups in the order they read: where the
/// letter is, how it is turned, how big, how faded, what colour.
enum TextAnimatorValue {
  rangeStart,
  rangeEnd,
  rangeOffset,
  easeHigh,
  easeLow,
  positionX,
  positionY,
  positionZ,
  rotation,
  rotationX,
  rotationY,
  scaleX,
  scaleY,
  opacity,
  fillR,
  fillG,
  fillB,
  boxOpacity,
  boxR,
  boxG,
  boxB,
  // A Proximity selector's reach, a Wiggly selector's four, then the font
  // axes in the animator's order.
  radius,
  minAmount,
  maxAmount,
  wiggles,
  correlation,
  staggerAmount,
  staggerDelay,
  staggerDelayEase,
  staggerBounceAmplitude,
  staggerBounceFrequency,
  staggerBounceDecay,
  // The whole animator's Amount, then the properties After Effects has that
  // came later.
  animatorAmount,
  skew,
  skewAxis,
  tracking,
  blur,
  characterOffset,
  axis0,
  axis1,
  axis2,
  axis3,
  axis4,
  axis5,
  axis6,
  axis7,
}

/// The font axes' slots, in order.
const _axisValues = [
  TextAnimatorValue.axis0,
  TextAnimatorValue.axis1,
  TextAnimatorValue.axis2,
  TextAnimatorValue.axis3,
  TextAnimatorValue.axis4,
  TextAnimatorValue.axis5,
  TextAnimatorValue.axis6,
  TextAnimatorValue.axis7,
];

/// Which of [a]'s font axes [value] is, or null for any other number.
int? textAnimatorAxisIndex(TextAnimatorValue value) {
  final i = _axisValues.indexOf(value);
  return i < 0 ? null : i;
}

/// Whether [value] is one of a selector's own numbers rather than the
/// animator's.
bool isSelectorValue(TextAnimatorValue value) => switch (value) {
      TextAnimatorValue.rangeStart ||
      TextAnimatorValue.rangeEnd ||
      TextAnimatorValue.rangeOffset ||
      TextAnimatorValue.easeHigh ||
      TextAnimatorValue.easeLow ||
      TextAnimatorValue.radius ||
      TextAnimatorValue.minAmount ||
      TextAnimatorValue.maxAmount ||
      TextAnimatorValue.wiggles ||
      TextAnimatorValue.correlation ||
      TextAnimatorValue.staggerAmount ||
      TextAnimatorValue.staggerDelay ||
      TextAnimatorValue.staggerDelayEase ||
      TextAnimatorValue.staggerBounceAmplitude ||
      TextAnimatorValue.staggerBounceFrequency ||
      TextAnimatorValue.staggerBounceDecay =>
        true,
      _ => false,
    };

/// The animator's own numbers [a] has rows for, after its selectors': its
/// Amount, the property groups it shows ([solid] adds the 3D ones) and one
/// per font axis it moves. A selector's are [selectorItemsOf]'s.
List<TextAnimatorValue> textAnimatorValuesOf(BridgeTextAnimator a,
    {required bool solid}) {
  return [
    for (final value in TextAnimatorValue.values)
      if (switch (value) {
        _ when isSelectorValue(value) => false,
        TextAnimatorValue.animatorAmount => true,
        _ when textAnimatorAxisIndex(value) != null =>
          textAnimatorAxisIndex(value)! < a.axes.length,
        _ => switch (animatorPropertyOf(value)) {
            final p? => (solid || !p.solidOnly) && animatorShowsProperty(a, p),
            null => true,
          },
      })
        value,
  ];
}

/// A group of an animator's numbers that Add ▸ Property adds as one, the way
/// After Effects lists them.
enum AnimatorProperty {
  position('position'),
  rotation('rotation'),
  scale('scale'),
  opacity('opacity'),
  fill('fill'),
  box('box'),
  positionZ('position_z'),
  rotationX('rotation_x'),
  rotationY('rotation_y'),
  skew('skew'),
  tracking('tracking'),
  blur('blur'),
  characterOffset('character_offset');

  /// How the document names it.
  final String key;
  const AnimatorProperty(this.key);

  /// Only a solid layer's letters move in depth and turn in 3D.
  bool get solidOnly =>
      this == positionZ || this == rotationX || this == rotationY;

  /// A group that came after the list did: an animator from before the list
  /// shows it only once it is added or moved, so it shows no more than it
  /// did.
  bool get late =>
      this == skew ||
      this == tracking ||
      this == blur ||
      this == characterOffset;

  /// The numbers in the group.
  List<TextAnimatorValue> get numbers => switch (this) {
        position => const [
            TextAnimatorValue.positionX,
            TextAnimatorValue.positionY
          ],
        rotation => const [TextAnimatorValue.rotation],
        scale => const [TextAnimatorValue.scaleX, TextAnimatorValue.scaleY],
        opacity => const [TextAnimatorValue.opacity],
        fill => const [
            TextAnimatorValue.fillR,
            TextAnimatorValue.fillG,
            TextAnimatorValue.fillB
          ],
        box => const [
            TextAnimatorValue.boxOpacity,
            TextAnimatorValue.boxR,
            TextAnimatorValue.boxG,
            TextAnimatorValue.boxB
          ],
        positionZ => const [TextAnimatorValue.positionZ],
        rotationX => const [TextAnimatorValue.rotationX],
        rotationY => const [TextAnimatorValue.rotationY],
        skew => const [TextAnimatorValue.skew, TextAnimatorValue.skewAxis],
        tracking => const [TextAnimatorValue.tracking],
        blur => const [TextAnimatorValue.blur],
        characterOffset => const [TextAnimatorValue.characterOffset],
      };

  /// What the Add ▸ Property menu calls it.
  String get label => switch (this) {
        position => l10n.transformPosition,
        rotation => l10n.transformRotation,
        scale => l10n.transformScale,
        opacity => l10n.transformOpacity,
        fill => l10n.textAnimatorFillOffset,
        box => l10n.textAnimatorBox,
        positionZ => '${l10n.transformPosition} z',
        rotationX => '${l10n.transformRotation} x',
        rotationY => '${l10n.transformRotation} y',
        skew => l10n.textAnimatorSkew,
        tracking => l10n.textAnimatorTracking,
        blur => l10n.textAnimatorBlur,
        characterOffset => l10n.textAnimatorCharacterOffset,
      };
}

/// The group [value] belongs to, or null for a selector's number or an axis.
AnimatorProperty? animatorPropertyOf(TextAnimatorValue value) {
  for (final p in AnimatorProperty.values) {
    if (p.numbers.contains(value)) return p;
  }
  return null;
}

/// Whether [a] lists [p]: every group on an animator from before the list,
/// one Add ▸ Property put there, and one whose numbers have been moved.
bool animatorShowsProperty(BridgeTextAnimator a, AnimatorProperty p) {
  final listed = a.properties;
  if (listed == null && !p.late || (listed?.contains(p.key) ?? false)) {
    return true;
  }
  return p.numbers.any((v) =>
      textAnimatorScalarOf(a, v) !=
      BridgeScalar.static_(textAnimatorDefault(v)));
}

/// [a] listing [p] too.
BridgeTextAnimator animatorWithProperty(
    BridgeTextAnimator a, AnimatorProperty p) {
  final listed = a.properties;
  if (listed?.contains(p.key) ?? false) return a;
  // An animator from before the list lists every older group already.
  if (listed == null && !p.late) return a;
  return animatorWith(a, properties: [
    ...listed ??
        [
          for (final other in AnimatorProperty.values)
            if (!other.late) other.key
        ],
    p.key,
  ]);
}

/// [a] without [p]: its numbers back where they change nothing, and off the
/// list. An animator from before the list starts one with the rest of its
/// groups.
BridgeTextAnimator animatorWithoutProperty(
    BridgeTextAnimator a, AnimatorProperty p) {
  var out = a;
  for (final v in p.numbers) {
    out = animatorWithScalar(
        out, v, BridgeScalar.static_(textAnimatorDefault(v)));
  }
  final listed = a.properties ??
      [
        for (final other in AnimatorProperty.values)
          if (!other.late) other.key
      ];
  return animatorWith(out, properties: [
    for (final k in listed)
      if (k != p.key) k
  ]);
}

/// The numbers only a solid layer's letters can use.
const solidAnimatorValues = {
  TextAnimatorValue.positionZ,
  TextAnimatorValue.rotationX,
  TextAnimatorValue.rotationY,
};

/// What one of an animator's numbers is called — shared by the Timeline row,
/// the graph channel and anything else that has to name one.
String textAnimatorValueLabel(TextAnimatorValue value,
        [BridgeTextAnimator? a]) =>
    switch (value) {
      TextAnimatorValue.radius => l10n.textAnimatorRadius,
      TextAnimatorValue.minAmount => l10n.textAnimatorMinAmount,
      TextAnimatorValue.maxAmount => l10n.textAnimatorMaxAmount,
      TextAnimatorValue.wiggles => l10n.textAnimatorWiggles,
      TextAnimatorValue.correlation => l10n.textAnimatorCorrelation,
      TextAnimatorValue.staggerAmount => l10n.textAnimatorAmount,
      TextAnimatorValue.staggerDelay => l10n.textAnimatorDelay,
      TextAnimatorValue.staggerDelayEase => l10n.textAnimatorDelayEase,
      TextAnimatorValue.staggerBounceAmplitude =>
        l10n.textAnimatorBounceAmplitude,
      TextAnimatorValue.staggerBounceFrequency =>
        l10n.textAnimatorBounceFrequency,
      TextAnimatorValue.staggerBounceDecay => l10n.textAnimatorBounceDecay,
      TextAnimatorValue.animatorAmount => l10n.textAnimatorAnimatorAmount,
      TextAnimatorValue.skew => l10n.textAnimatorSkew,
      TextAnimatorValue.skewAxis => l10n.textAnimatorSkewAxis,
      TextAnimatorValue.tracking => l10n.textAnimatorTracking,
      TextAnimatorValue.blur => l10n.textAnimatorBlur,
      TextAnimatorValue.characterOffset => l10n.textAnimatorCharacterOffset,
      _ when textAnimatorAxisIndex(value) != null =>
        l10n.textAnimatorAxisLabel(fontAxisName('', '', _axisTag(a, value))),
      TextAnimatorValue.rangeStart => l10n.textAnimatorRangeStart,
      TextAnimatorValue.rangeEnd => l10n.textAnimatorRangeEnd,
      TextAnimatorValue.rangeOffset => l10n.textAnimatorRangeOffset,
      TextAnimatorValue.easeHigh => l10n.textAnimatorEaseHigh,
      TextAnimatorValue.easeLow => l10n.textAnimatorEaseLow,
      TextAnimatorValue.positionX => '${l10n.transformPosition} x',
      TextAnimatorValue.positionY => '${l10n.transformPosition} y',
      TextAnimatorValue.positionZ => '${l10n.transformPosition} z',
      TextAnimatorValue.rotation => l10n.transformRotation,
      TextAnimatorValue.rotationX => '${l10n.transformRotation} x',
      TextAnimatorValue.rotationY => '${l10n.transformRotation} y',
      TextAnimatorValue.scaleX => '${l10n.transformScale} x',
      TextAnimatorValue.scaleY => '${l10n.transformScale} y',
      TextAnimatorValue.opacity => l10n.transformOpacity,
      TextAnimatorValue.fillR => '${l10n.textAnimatorFillOffset} r',
      TextAnimatorValue.fillG => '${l10n.textAnimatorFillOffset} g',
      TextAnimatorValue.fillB => '${l10n.textAnimatorFillOffset} b',
      TextAnimatorValue.boxOpacity => l10n.textAnimatorBoxOpacity,
      TextAnimatorValue.boxR => '${l10n.textAnimatorBoxColour} r',
      TextAnimatorValue.boxG => '${l10n.textAnimatorBoxColour} g',
      TextAnimatorValue.boxB => '${l10n.textAnimatorBoxColour} b',
      _ => value.name,
    };

/// The tag of the font axis in [value]'s slot of [a], or the slot's number
/// when [a] is not known.
String _axisTag(BridgeTextAnimator? a, TextAnimatorValue value) {
  final i = textAnimatorAxisIndex(value) ?? 0;
  return a != null && i < a.axes.length ? a.axes[i].tag : '${i + 1}';
}

/// What one of an animator's numbers is when it changes nothing, for Reset.
double textAnimatorDefault(TextAnimatorValue value) => switch (value) {
      TextAnimatorValue.radius => 200,
      TextAnimatorValue.wiggles => 2,
      TextAnimatorValue.correlation => 50,
      TextAnimatorValue.staggerAmount => 100,
      TextAnimatorValue.staggerDelay => 0.1,
      TextAnimatorValue.staggerBounceFrequency => 2,
      TextAnimatorValue.staggerBounceDecay => 3,
      TextAnimatorValue.animatorAmount ||
      TextAnimatorValue.maxAmount ||
      TextAnimatorValue.rangeEnd ||
      TextAnimatorValue.scaleX ||
      TextAnimatorValue.scaleY ||
      TextAnimatorValue.opacity =>
        100,
      // The engine's soft yellow.
      TextAnimatorValue.boxR => 1,
      TextAnimatorValue.boxG => 0.75,
      TextAnimatorValue.boxB => 0.15,
      _ => 0,
    };

/// Which of selector [s]'s numbers [value] names; a number that is not a
/// selector's reads nought.
BridgeScalar selectorScalarOf(
        BridgeRangeSelector s, TextAnimatorValue value) =>
    switch (value) {
      TextAnimatorValue.rangeStart => s.start,
      TextAnimatorValue.rangeEnd => s.end,
      TextAnimatorValue.rangeOffset => s.offset,
      TextAnimatorValue.easeHigh => s.easeHigh,
      TextAnimatorValue.easeLow => s.easeLow,
      TextAnimatorValue.radius => s.radius,
      TextAnimatorValue.minAmount => s.minAmount,
      TextAnimatorValue.maxAmount => s.maxAmount,
      TextAnimatorValue.wiggles => s.wiggles,
      TextAnimatorValue.correlation => s.correlation,
      TextAnimatorValue.staggerAmount => s.amount,
      TextAnimatorValue.staggerDelay => s.delay,
      TextAnimatorValue.staggerDelayEase => s.delayEase,
      TextAnimatorValue.staggerBounceAmplitude => s.bounceAmplitude,
      TextAnimatorValue.staggerBounceFrequency => s.bounceFrequency,
      TextAnimatorValue.staggerBounceDecay => s.bounceDecay,
      _ => const BridgeScalar.static_(0),
    };

/// [s] with the number [value] names replaced.
BridgeRangeSelector selectorWithScalar(
        BridgeRangeSelector s, TextAnimatorValue value, BridgeScalar to) =>
    switch (value) {
      TextAnimatorValue.rangeStart => selectorWith(s, start: to),
      TextAnimatorValue.rangeEnd => selectorWith(s, end: to),
      TextAnimatorValue.rangeOffset => selectorWith(s, offset: to),
      TextAnimatorValue.easeHigh => selectorWith(s, easeHigh: to),
      TextAnimatorValue.easeLow => selectorWith(s, easeLow: to),
      TextAnimatorValue.radius => selectorWith(s, radius: to),
      TextAnimatorValue.minAmount => selectorWith(s, minAmount: to),
      TextAnimatorValue.maxAmount => selectorWith(s, maxAmount: to),
      TextAnimatorValue.wiggles => selectorWith(s, wiggles: to),
      TextAnimatorValue.correlation => selectorWith(s, correlation: to),
      TextAnimatorValue.staggerAmount => selectorWith(s, amount: to),
      TextAnimatorValue.staggerDelay => selectorWith(s, delay: to),
      TextAnimatorValue.staggerDelayEase => selectorWith(s, delayEase: to),
      TextAnimatorValue.staggerBounceAmplitude =>
        selectorWith(s, bounceAmplitude: to),
      TextAnimatorValue.staggerBounceFrequency =>
        selectorWith(s, bounceFrequency: to),
      TextAnimatorValue.staggerBounceDecay => selectorWith(s, bounceDecay: to),
      _ => s,
    };

/// Which of [a]'s numbers [value] names; a selector's number is read off
/// its selector [selector], the first by default.
BridgeScalar textAnimatorScalarOf(
        BridgeTextAnimator a, TextAnimatorValue value,
        {int selector = 0}) =>
    switch (value) {
      _ when isSelectorValue(value) =>
        selectorScalarOf(selectorAt(a, selector), value),
      TextAnimatorValue.animatorAmount => a.amount,
      TextAnimatorValue.skew => a.skew,
      TextAnimatorValue.skewAxis => a.skewAxis,
      TextAnimatorValue.tracking => a.tracking,
      TextAnimatorValue.blur => a.blur,
      TextAnimatorValue.characterOffset => a.characterOffset,
      TextAnimatorValue.positionX => a.positionX,
      TextAnimatorValue.positionY => a.positionY,
      TextAnimatorValue.positionZ => a.positionZ,
      TextAnimatorValue.rotation => a.rotation,
      TextAnimatorValue.rotationX => a.rotationX,
      TextAnimatorValue.rotationY => a.rotationY,
      TextAnimatorValue.scaleX => a.scaleX,
      TextAnimatorValue.scaleY => a.scaleY,
      TextAnimatorValue.opacity => a.opacity,
      TextAnimatorValue.fillR => a.fillR,
      TextAnimatorValue.fillG => a.fillG,
      TextAnimatorValue.fillB => a.fillB,
      TextAnimatorValue.boxOpacity => a.boxOpacity,
      TextAnimatorValue.boxR => a.boxR,
      TextAnimatorValue.boxG => a.boxG,
      TextAnimatorValue.boxB => a.boxB,
      _ => switch (textAnimatorAxisIndex(value)) {
          final i? when i < a.axes.length => a.axes[i].value,
          _ => const BridgeScalar.static_(0),
        },
    };

/// [a] with the one number [value] names replaced, a selector's on its
/// selector [selector].
BridgeTextAnimator animatorWithScalar(
        BridgeTextAnimator a, TextAnimatorValue value, BridgeScalar to,
        {int selector = 0}) =>
    switch (value) {
      _ when isSelectorValue(value) => animatorWithSelectorAt(a, selector,
          selectorWithScalar(selectorAt(a, selector), value, to)),
      TextAnimatorValue.animatorAmount => animatorWith(a, amount: to),
      TextAnimatorValue.skew => animatorWith(a, skew: to),
      TextAnimatorValue.skewAxis => animatorWith(a, skewAxis: to),
      TextAnimatorValue.tracking => animatorWith(a, tracking: to),
      TextAnimatorValue.blur => animatorWith(a, blur: to),
      TextAnimatorValue.characterOffset =>
        animatorWith(a, characterOffset: to),
      TextAnimatorValue.positionX => animatorWith(a, positionX: to),
      TextAnimatorValue.positionY => animatorWith(a, positionY: to),
      TextAnimatorValue.positionZ => animatorWith(a, positionZ: to),
      TextAnimatorValue.rotation => animatorWith(a, rotation: to),
      TextAnimatorValue.rotationX => animatorWith(a, rotationX: to),
      TextAnimatorValue.rotationY => animatorWith(a, rotationY: to),
      TextAnimatorValue.scaleX => animatorWith(a, scaleX: to),
      TextAnimatorValue.scaleY => animatorWith(a, scaleY: to),
      TextAnimatorValue.opacity => animatorWith(a, opacity: to),
      TextAnimatorValue.fillR => animatorWith(a, fillR: to),
      TextAnimatorValue.fillG => animatorWith(a, fillG: to),
      TextAnimatorValue.fillB => animatorWith(a, fillB: to),
      TextAnimatorValue.boxOpacity => animatorWith(a, boxOpacity: to),
      TextAnimatorValue.boxR => animatorWith(a, boxR: to),
      TextAnimatorValue.boxG => animatorWith(a, boxG: to),
      TextAnimatorValue.boxB => animatorWith(a, boxB: to),
      _ => switch (textAnimatorAxisIndex(value)) {
          final i? when i < a.axes.length => animatorWith(a, axes: [
              for (var k = 0; k < a.axes.length; k++)
                if (k == i)
                  BridgeAnimatorAxis(tag: a.axes[k].tag, value: to)
                else
                  a.axes[k],
            ]),
          _ => a,
        },
    };

/// Write one of [layer]'s animators back with one number replaced — the write
/// path every row outside the Animators section shares.
///
/// The whole document goes, because that is the op: reading it here rather
/// than carrying it on the row is deliberate, since this only ever runs on a
/// commit and never while anything is being drawn.
void writeTextAnimatorScalar({
  required LayerReference layer,
  required int index,
  required TextAnimatorValue value,
  required BridgeScalar to,
  int selector = 0,
}) {
  final document = layer.getText();
  if (document == null || index >= document.animators.length) return;
  layer.setText(
    document: BridgeTextDocument(
      text: document.text,
      expression: document.expression,
      keys: document.keys,
      size: document.size,
      fill: document.fill,
      path: document.path,
      pathOffset: document.pathOffset,
      style: document.style,
      paragraph: document.paragraph,
      animators: [
        for (var i = 0; i < document.animators.length; i++)
          if (i == index)
            animatorWithScalar(document.animators[i], value, to,
                selector: selector)
          else
            document.animators[i],
      ],
    ),
  );
}

/// [selector] with one field replaced, for the same reason.
BridgeRangeSelector selectorWith(
  BridgeRangeSelector s, {
  BridgeScalar? start,
  BridgeScalar? end,
  BridgeScalar? offset,
  BridgeSelectorBasis? basis,
  BridgeSelectorShape? shape,
  BridgeSelectorMode? mode,
  double? ease,
  BridgeSelectorKind? kind,
  UuidValue? target,
  bool clearTarget = false,
  BridgeScalar? radius,
  bool? invert,
  BridgeScalar? easeHigh,
  BridgeScalar? easeLow,
  BridgeScalar? amount,
  BridgeScalar? delay,
  BridgeScalar? delayEase,
  BridgeStaggerOrder? order,
  BridgeScalar? bounceAmplitude,
  BridgeScalar? bounceFrequency,
  BridgeScalar? bounceDecay,
  BridgeScalar? minAmount,
  BridgeScalar? maxAmount,
  BridgeScalar? wiggles,
  BridgeScalar? correlation,
  int? seed,
  BridgeSelectorCombine? combine,
  BridgeSelectorUnits? units,
  bool? randomize,
  bool? delayFrames,
  bool? total,
  BridgeStaggerRepeat? repeat,
  bool? bounceLast,
}) =>
    BridgeRangeSelector(
      combine: combine ?? s.combine,
      units: units ?? s.units,
      randomize: randomize ?? s.randomize,
      delayFrames: delayFrames ?? s.delayFrames,
      total: total ?? s.total,
      repeat: repeat ?? s.repeat,
      bounceLast: bounceLast ?? s.bounceLast,
      minAmount: minAmount ?? s.minAmount,
      maxAmount: maxAmount ?? s.maxAmount,
      wiggles: wiggles ?? s.wiggles,
      correlation: correlation ?? s.correlation,
      seed: seed ?? s.seed,
      start: start ?? s.start,
      end: end ?? s.end,
      offset: offset ?? s.offset,
      basis: basis ?? s.basis,
      shape: shape ?? s.shape,
      mode: mode ?? s.mode,
      ease: ease ?? s.ease,
      kind: kind ?? s.kind,
      target: clearTarget ? null : (target ?? s.target),
      radius: radius ?? s.radius,
      invert: invert ?? s.invert,
      easeHigh: easeHigh ?? s.easeHigh,
      easeLow: easeLow ?? s.easeLow,
      amount: amount ?? s.amount,
      delay: delay ?? s.delay,
      delayEase: delayEase ?? s.delayEase,
      order: order ?? s.order,
      bounceAmplitude: bounceAmplitude ?? s.bounceAmplitude,
      bounceFrequency: bounceFrequency ?? s.bounceFrequency,
      bounceDecay: bounceDecay ?? s.bounceDecay,
    );

/// What a Stagger selector's order is called.
String staggerOrderLabel(BridgeStaggerOrder o) => switch (o) {
      BridgeStaggerOrder.forward => l10n.textAnimatorOrderForward,
      BridgeStaggerOrder.backward => l10n.textAnimatorOrderBackward,
      BridgeStaggerOrder.fromCentre => l10n.textAnimatorOrderFromCentre,
      BridgeStaggerOrder.fromEdges => l10n.textAnimatorOrderFromEdges,
      BridgeStaggerOrder.random => l10n.textAnimatorOrderRandom,
    };

/// How many font axes one Text layer's animators can move, as many as After
/// Effects allows. The engine drops a ninth.
const maxAnimatedFontAxes = 8;

/// [a] with its selector made [kind]. A Proximity selector starts Smooth
/// where it was Square, since a square edge switches each letter at once
/// as the target passes it.
BridgeTextAnimator animatorWithSelectorKind(
    BridgeTextAnimator a, BridgeSelectorKind kind) {
  final square = a.selector.shape == BridgeSelectorShape.square;
  return animatorWith(a,
      selector: selectorWith(a.selector,
          kind: kind,
          shape: kind == BridgeSelectorKind.proximity && square
              ? BridgeSelectorShape.smooth
              : null));
}

/// The rows of an animator's Add menu, After Effects' Add beside an
/// animator: **Property** lists the groups it doesn't show yet and the font's
/// axes it doesn't move, **Selector** what picks its letters.
List<Widget> animatorAddMenuRows({
  required VoidCallback close,
  required List<BridgeTextAnimator> animators,
  required int index,
  required BridgeTextStyle? style,
  required bool solid,
  required ValueChanged<BridgeTextAnimator> put,
}) {
  final a = animators[index];
  final family = style?.family ?? '';
  final face = style?.face ?? '';
  final moved = {
    for (final other in animators)
      for (final x in other.axes) x.tag,
  };
  final axes = [
    for (final axis in shownFontAxes(family, face))
      if (!a.axes.any((x) => x.tag == axis.tag) &&
          (moved.contains(axis.tag) || moved.length < maxAnimatedFontAxes))
        axis,
  ];
  final properties = [
    for (final p in AnimatorProperty.values)
      if ((solid || !p.solidOnly) && !animatorShowsProperty(a, p)) p,
  ];
  Widget submenu(
          String key, String label, List<Widget> Function(VoidCallback) rows) =>
      SubmenuRow(
        key: ValueKey<String>(key),
        closeParent: close,
        submenu: (dismiss) => FloatSurface(
          width: 170,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: rows(dismiss),
          ),
        ),
        child: Text(label),
      );
  return [
    submenu(
        'text-animator-add-property-$index',
        l10n.textAnimatorProperty,
        (dismiss) => [
              for (final p in properties)
                MenuRow(
                  key: ValueKey<String>('text-animator-add-${p.key}-$index'),
                  onPressed: () {
                    dismiss();
                    put(animatorWithProperty(a, p));
                  },
                  child: Text(p.label),
                ),
              if (properties.isNotEmpty && axes.isNotEmpty) const MenuDivider(),
              // A new axis starts where the face already sets the letters, so
              // it changes nothing until it is moved.
              for (final axis in axes)
                MenuRow(
                  key: ValueKey<String>(
                      'text-animator-add-axis-${axis.tag}-$index'),
                  onPressed: () {
                    dismiss();
                    put(animatorWith(a, axes: [
                      ...a.axes,
                      BridgeAnimatorAxis(
                          tag: axis.tag,
                          value: BridgeScalar.static_(axis.face)),
                    ]));
                  },
                  child: Text(l10n.textAnimatorAxisLabel(axis.name)),
                ),
              if (properties.isEmpty && axes.isEmpty)
                MenuRow(
                  enabled: false,
                  onPressed: () {},
                  child: Text(moved.length >= maxAnimatedFontAxes
                      ? l10n.textAnimatorAxesFull
                      : l10n.none),
                ),
            ]),
    submenu(
        'text-animator-add-selector-$index',
        l10n.textAnimatorSelector,
        (dismiss) => [
              // Another selector under the ones the animator has, as After
              // Effects' Add ▸ Selector; its Mode says how it meets them.
              for (final kind in BridgeSelectorKind.values)
                MenuRow(
                  key: ValueKey<String>(
                      'text-animator-selector-${kind.name}-$index'),
                  onPressed: () {
                    dismiss();
                    put(animatorWith(a,
                        selectors: [...a.selectors, freshSelector(kind)]));
                  },
                  child: Text(selectorKindLabel(kind)),
                ),
            ]),
  ];
}

/// The rows of a Text layer's **Add animator** menu, After Effects' Animate:
/// each one adds an animator showing that property, or moving that font axis.
/// A blank one first, with nothing on it to start.
List<Widget> animateMenuRows({
  required VoidCallback close,
  required LayerReference layer,
  required bool solid,
  required VoidCallback onChanged,
  CompositionReference? comp,
  int frame = 0,
}) {
  final document = layer.getText();
  if (document == null) return const [];
  final style = document.style;
  final moved = {
    for (final other in document.animators)
      for (final x in other.axes) x.tag,
  };
  final axes = [
    for (final axis in shownFontAxes(style.family, style.face))
      if (moved.contains(axis.tag) || moved.length < maxAnimatedFontAxes) axis,
  ];
  // Named after what it was added for: "Animator 1 (Position)".
  BridgeTextAnimator fresh([String? what]) => freshTextAnimator(
      animatorNameFor(document.animators, what));
  void add(List<BridgeTextAnimator> animators) {
    close();
    if (addTextAnimator(layer, adding: animators)) onChanged();
  }

  return [
    MenuRow(
      key: const ValueKey<String>('text-animate-blank'),
      onPressed: () => add([fresh()]),
      child: Text(l10n.textAnimatorBlank),
    ),
    const MenuDivider(),
    for (final p in AnimatorProperty.values)
      if (solid || !p.solidOnly)
        MenuRow(
          key: ValueKey<String>('text-animate-${p.key}'),
          onPressed: () => add([animatorWithProperty(fresh(p.label), p)]),
          child: Text(p.label),
        ),
    // The font's axes, one animator moving each, in a menu of their own: a
    // font can have a dozen.
    if (axes.isNotEmpty)
      SubmenuRow(
        key: const ValueKey<String>('text-animate-axes'),
        closeParent: close,
        submenu: (dismiss) => FloatSurface(
          width: 190,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final axis in axes)
                MenuRow(
                  key: ValueKey<String>('text-animate-axis-${axis.tag}'),
                  onPressed: () {
                    dismiss();
                    add([
                      animatorWith(fresh(axis.name), axes: [
                        BridgeAnimatorAxis(
                            tag: axis.tag,
                            value: BridgeScalar.static_(axis.face)),
                      ])
                    ]);
                  },
                  child: Text(l10n.textAnimatorAxisLabel(axis.name)),
                ),
            ],
          ),
        ),
        child: Text(l10n.textFontAxes),
      ),
    // The presets, a second of keys from the playhead, as Animate text has.
    if (comp != null) ...[
      const MenuDivider(),
      for (final preset in TextAnimationPreset.values)
        MenuRow(
          key: ValueKey<String>('text-animate-preset-${preset.name}'),
          onPressed: () => add(textPresetAnimators(preset, comp, frame)),
          child: Text(preset.title),
        ),
      // The user's own, saved from an animator's More menu, beside the
      // built-in ones; their keys start at the playhead.
      if (animatorPresets() case final saved when saved.isNotEmpty)
        SubmenuRow(
          key: const ValueKey<String>('text-animate-my-presets'),
          closeParent: close,
          submenu: (dismiss) => FloatSurface(
            width: 190,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                for (final (name, file) in saved)
                  MenuRow(
                    key: ValueKey<String>('text-animate-my-preset-$name'),
                    onPressed: () async {
                      dismiss();
                      final loaded = await loadAnimatorPreset(file, comp, frame);
                      if (loaded.isNotEmpty) add(loaded);
                    },
                    child: Text(name),
                  ),
              ],
            ),
          ),
          child: Text(l10n.textAnimatorMyPresets),
        ),
    ],
    // What Copy animator kept, from this layer or another.
    if (copiedAnimators.isNotEmpty) ...[
      const MenuDivider(),
      MenuRow(
        key: const ValueKey<String>('text-animate-paste'),
        onPressed: () => add([
          for (final a in copiedAnimators)
            animatorWith(a,
                name: freeAnimatorName(document.animators, a.name)),
        ]),
        child: Text(l10n.textAnimatorPaste),
      ),
    ],
  ];
}

/// [layer]'s animators with [index] taken off, or all of them when [index] is
/// null: what Delete does on an animator's heading and on Animators.
void removeTextAnimators(LayerReference layer,
    {int? index, LumitUiState? ui}) {
  final document = layer.getText();
  if (document == null || document.animators.isEmpty) return;
  followAnimators(ui, layer, document.animators, [
    if (index != null)
      for (var i = 0; i < document.animators.length; i++)
        if (i != index) document.animators[i],
  ]);
  layer.setText(
    document: BridgeTextDocument(
      text: document.text,
      expression: document.expression,
      keys: document.keys,
      size: document.size,
      fill: document.fill,
      path: document.path,
      pathOffset: document.pathOffset,
      style: document.style,
      paragraph: document.paragraph,
      animators: [
        if (index != null)
          for (var i = 0; i < document.animators.length; i++)
            if (i != index) document.animators[i],
      ],
    ),
  );
}

/// [layer]'s animator [index] without the number [value] names: a font axis
/// taken off, a property group put back and off the list. A selector's own
/// numbers stay, since an animator is never without a selector. Returns
/// whether anything went.
bool removeTextAnimatorValue(
    LayerReference layer, int index, TextAnimatorValue value) {
  final document = layer.getText();
  if (document == null || index >= document.animators.length) return false;
  final a = document.animators[index];
  final BridgeTextAnimator next;
  if (textAnimatorAxisIndex(value) case final k? when k < a.axes.length) {
    next = animatorWith(a, axes: [
      for (var i = 0; i < a.axes.length; i++)
        if (i != k) a.axes[i],
    ]);
  } else if (animatorPropertyOf(value) case final p?) {
    next = animatorWithoutProperty(a, p);
  } else {
    return false;
  }
  layer.setText(
    document: BridgeTextDocument(
      text: document.text,
      expression: document.expression,
      keys: document.keys,
      size: document.size,
      fill: document.fill,
      path: document.path,
      pathOffset: document.pathOffset,
      style: document.style,
      paragraph: document.paragraph,
      animators: [
        for (var i = 0; i < document.animators.length; i++)
          if (i == index) next else document.animators[i],
      ],
    ),
  );
  return true;
}

/// Write [layer]'s animator [index] back as [next], the rest of the document
/// as it is.
void writeTextAnimator(
    LayerReference layer, int index, BridgeTextAnimator next) {
  final document = layer.getText();
  if (document == null || index >= document.animators.length) return;
  layer.setText(
    document: BridgeTextDocument(
      text: document.text,
      expression: document.expression,
      keys: document.keys,
      size: document.size,
      fill: document.fill,
      path: document.path,
      pathOffset: document.pathOffset,
      style: document.style,
      paragraph: document.paragraph,
      animators: [
        for (var i = 0; i < document.animators.length; i++)
          if (i == index) next else document.animators[i],
      ],
    ),
  );
}

/// The test key a selector choice's control has always had.
String _settingKey(AnimatorSetting setting, BridgeRangeSelector s) =>
    switch (setting) {
      AnimatorSetting.selector => 'text-animator-kind',
      AnimatorSetting.mode => 'text-animator-subtract',
      AnimatorSetting.order => 'stagger-order',
      AnimatorSetting.seed when s.kind == BridgeSelectorKind.wiggly =>
        'wiggly-seed',
      AnimatorSetting.seed when s.kind == BridgeSelectorKind.stagger =>
        'stagger-seed',
      _ => 'text-animator-${setting.name}',
    };
