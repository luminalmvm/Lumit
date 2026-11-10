// The Animators section: a Text layer's letters moved one at a time.
//
// An animator names a set of per-letter offsets — a push, a turn, a size, a
// fade, a tint — and a **range selector** saying which letters they reach. The
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
    show TextAnimationPreset, reversedAnimator, textPresetAnimators;
import 'layer_fold_frb.dart' show animatorsPath;
import '../state/font_axes.dart';
import '../widgets/layer_picker.dart';
import 'timeline_extras_frb.dart' show showMenuAt;
import 'timeline_mask_rows_frb.dart' show RowButton;

/// Matches the Effect controls panel's own cell width, so the values line up
/// down the panel.
const double _cellWidth = 78;

/// The Animators section for [layer], or nothing when it is not a Text layer.
class TextAnimatorRowsFrb extends StatelessWidget {
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
  });

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<int>(
        // The font's axes come in from the engine a moment after the first
        // draw, and name the axis rows.
        valueListenable: fontAxesArrived,
        builder: (context, _, __) => _section(context),
      );

  Widget _section(BuildContext context) {
    final document = layer.getText();
    if (document == null) return const SizedBox.shrink();

    // One write path: read the document, change the animator list, write it
    // whole. The engine takes the whole document as one op — and when the list
    // crosses empty ↔ not-empty it moves the layer's anchor by the margin the
    // animated raster adds, in the same op, so the words do not jump.
    void write(List<BridgeTextAnimator> animators) {
      layer.setText(
        document: BridgeTextDocument(
          text: document.text,
          expression: document.expression,
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

    final rows = <Widget>[];
    for (var i = 0; i < document.animators.length; i++) {
      rows.addAll(_animatorRows(
        context,
        document.animators,
        i,
        write,
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

  /// One animator: its heading, its range selector, then its five property
  /// groups in the order they read — where the letter is, how it is turned,
  /// how big, how faded, what colour.
  List<Widget> _animatorRows(
    BuildContext context,
    List<BridgeTextAnimator> animators,
    int index,
    ValueChanged<List<BridgeTextAnimator>> write,
  ) {
    final t = ThemeScope.of(context).theme;
    final a = animators[index];

    void put(BridgeTextAnimator next) => write([
          for (var i = 0; i < animators.length; i++)
            if (i == index) next else animators[i],
        ]);
    void putSelector(BridgeRangeSelector next) =>
        put(animatorWith(a, selector: next));

    return [
      fxTwoColumnRow(
        context: context,
        name: Text(t.kickerCase(a.name),
            style: t.kickerOn, overflow: TextOverflow.ellipsis),
        control: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            // Add ▸ Property or Selector, as After Effects' Add beside an
            // animator.
            RowButton(
              key: ValueKey<String>('text-animator-add-to-$index'),
              label: l10n.shapeOpAdd,
              arrow: true,
              onTap: (at) async {
                final style = layer.getText()?.style;
                if (style != null) {
                  await fontAxesReady(style.family, style.face);
                }
                if (!context.mounted) return;
                showMenuAt<void>(
                  context: context,
                  position: at,
                  width: 170,
                  rows: (close) => animatorAddMenuRows(
                    close: () => close(null),
                    animators: animators,
                    index: index,
                    style: style,
                    solid: solid,
                    put: put,
                  ),
                );
              },
            ),
            const SizedBox(width: 6),
            if (animatorHasKeys(a))
              HouseButton(
                key: ValueKey<String>('text-animator-reverse-$index'),
                small: true,
                frameless: true,
                // Every animator of this name, as Tracking in lays two.
                onPressed: () => write([
                  for (final other in animators)
                    if (other.name == a.name)
                      reversedAnimator(other)
                    else
                      other,
                ]),
                child: Text(l10n.textAnimatorReverse),
              ),
            HouseButton(
              key: ValueKey<String>('text-animator-remove-$index'),
              small: true,
              frameless: true,
              onPressed: () => write([
                for (var i = 0; i < animators.length; i++)
                  if (i != index) animators[i],
              ]),
              child: Text(l10n.textAnimatorRemove),
            ),
          ],
        ),
      ),
      // Range picks letters by where they come in the words; Proximity by how
      // near they are to another layer, so dragging a null across the words
      // sweeps the change through them.
      _plainRow(
        context,
        l10n.textAnimatorSelector,
        BareDropdown<BridgeSelectorKind>(
          key: ValueKey<String>('text-animator-kind-$index'),
          value: a.selector.kind,
          options: BridgeSelectorKind.values,
          label: (k) => switch (k) {
            BridgeSelectorKind.range => l10n.textAnimatorSelectorRange,
            BridgeSelectorKind.proximity => l10n.textAnimatorSelectorProximity,
            BridgeSelectorKind.wiggly => l10n.textAnimatorSelectorWiggly,
          },
          onChanged: (k) => put(animatorWithSelectorKind(a, k)),
        ),
      ),
      if (a.selector.kind == BridgeSelectorKind.wiggly) ...[
        _numberRow(
            context,
            l10n.textAnimatorMaxAmount,
            'wiggly-max-$index',
            [a.selector.maxAmount],
            -100,
            100,
            100,
            (s) => putSelector(selectorWith(a.selector, maxAmount: s.first))),
        _numberRow(
            context,
            l10n.textAnimatorMinAmount,
            'wiggly-min-$index',
            [a.selector.minAmount],
            -100,
            100,
            0,
            (s) => putSelector(selectorWith(a.selector, minAmount: s.first))),
        _numberRow(
            context,
            l10n.textAnimatorWiggles,
            'wiggly-rate-$index',
            [a.selector.wiggles],
            0,
            1000,
            2,
            (s) => putSelector(selectorWith(a.selector, wiggles: s.first))),
        _numberRow(
            context,
            l10n.textAnimatorCorrelation,
            'wiggly-correlation-$index',
            [a.selector.correlation],
            0,
            100,
            50,
            (s) => putSelector(selectorWith(a.selector, correlation: s.first))),
        _plainRow(
          context,
          l10n.textAnimatorSeed,
          DragValueField(
            key: ValueKey<String>('wiggly-seed-$index'),
            value: a.selector.seed,
            min: 0,
            max: 99999,
            resetTo: 0,
            decimals: 0,
            onChanged: (v) => putSelector(selectorWith(a.selector,
                seed: v.round().clamp(0, 99999).toInt())),
          ),
        ),
      ],
      if (a.selector.kind == BridgeSelectorKind.proximity) ...[
        _plainRow(
          context,
          l10n.textAnimatorTarget,
          LayerPicker(
            key: ValueKey<String>('text-animator-target-$index'),
            layers: _layers(context),
            value: a.selector.target,
            empty: l10n.none,
            missing: l10n.missingLayer,
            // Any layer but the words themselves: what is measured is its
            // anchor, so a null, a light or a camera serves as well as a
            // picture.
            offered: (l) => l.layer.internallayerId != layer.internallayerId,
            onChanged: (picked) => putSelector(picked == null
                ? selectorWith(a.selector, clearTarget: true)
                : selectorWith(a.selector, target: picked)),
          ),
        ),
        _numberRow(
            context,
            l10n.textAnimatorRadius,
            'range-radius-$index',
            [a.selector.radius],
            0,
            100000,
            200,
            (s) => putSelector(selectorWith(a.selector, radius: s.first))),
        _plainRow(
          context,
          l10n.textAnimatorInvert,
          Align(
            alignment: Alignment.centerLeft,
            child: HouseCheckbox(
              key: ValueKey<String>('text-animator-invert-$index'),
              value: a.selector.invert,
              onChanged: (on) =>
                  putSelector(selectorWith(a.selector, invert: on)),
            ),
          ),
        ),
      ],
      // The range: which letters this animator reaches, in per cent of the
      // words. Offset is the one a cascade is keyed on, which is why it is a
      // number of its own rather than something the two ends share.
      if (a.selector.kind == BridgeSelectorKind.range) ...[
        _numberRow(
            context,
            l10n.textAnimatorRangeStart,
            'range-start-$index',
            [a.selector.start],
            -1000,
            1000,
            0,
            (s) => putSelector(selectorWith(a.selector, start: s.first))),
        _numberRow(
            context,
            l10n.textAnimatorRangeEnd,
            'range-end-$index',
            [a.selector.end],
            -1000,
            1000,
            100,
            (s) => putSelector(selectorWith(a.selector, end: s.first))),
        _numberRow(
            context,
            l10n.textAnimatorRangeOffset,
            'range-offset-$index',
            [a.selector.offset],
            -1000,
            1000,
            0,
            (s) => putSelector(selectorWith(a.selector, offset: s.first))),
      ],
      _plainRow(
        context,
        l10n.textAnimatorBasis,
        BareDropdown<BridgeSelectorBasis>(
          key: ValueKey<String>('text-animator-basis-$index'),
          value: a.selector.basis,
          options: BridgeSelectorBasis.values,
          label: (b) => switch (b) {
            BridgeSelectorBasis.characters => l10n.textAnimatorBasisCharacters,
            BridgeSelectorBasis.words => l10n.textAnimatorBasisWords,
          },
          onChanged: (b) => putSelector(selectorWith(a.selector, basis: b)),
        ),
      ),
      _plainRow(
        context,
        l10n.textAnimatorShape,
        BareDropdown<BridgeSelectorShape>(
          key: ValueKey<String>('text-animator-shape-$index'),
          value: a.selector.shape,
          options: BridgeSelectorShape.values,
          label: (s) => switch (s) {
            BridgeSelectorShape.square => l10n.textAnimatorShapeSquare,
            BridgeSelectorShape.ramp => l10n.textAnimatorShapeRamp,
            BridgeSelectorShape.smooth => l10n.textAnimatorShapeSmooth,
          },
          onChanged: (s) => putSelector(selectorWith(a.selector, shape: s)),
        ),
      ),
      // The property groups the animator shows: those Add ▸ Property put on
      // it and any whose numbers have been moved.
      if (animatorShowsProperty(a, AnimatorProperty.position))
        _numberRow(
            context,
            l10n.transformPosition,
            'anim-position-$index',
            [a.positionX, a.positionY],
            -100000,
            100000,
            0,
            (s) => put(animatorWith(a, positionX: s[0], positionY: s[1]))),
      if (solid && animatorShowsProperty(a, AnimatorProperty.positionZ))
        _numberRow(
            context,
            '${l10n.transformPosition} z',
            'anim-position-z-$index',
            [a.positionZ],
            -100000,
            100000,
            0,
            (s) => put(animatorWith(a, positionZ: s.first))),
      if (animatorShowsProperty(a, AnimatorProperty.rotation))
        _numberRow(
            context,
            l10n.transformRotation,
            'anim-rotation-$index',
            [a.rotation],
            -100000,
            100000,
            0,
            (s) => put(animatorWith(a, rotation: s.first))),
      if (solid && animatorShowsProperty(a, AnimatorProperty.rotationX))
        _numberRow(
            context,
            '${l10n.transformRotation} x',
            'anim-rotation-x-$index',
            [a.rotationX],
            -100000,
            100000,
            0,
            (s) => put(animatorWith(a, rotationX: s.first))),
      if (solid && animatorShowsProperty(a, AnimatorProperty.rotationY))
        _numberRow(
            context,
            '${l10n.transformRotation} y',
            'anim-rotation-y-$index',
            [a.rotationY],
            -100000,
            100000,
            0,
            (s) => put(animatorWith(a, rotationY: s.first))),
      if (animatorShowsProperty(a, AnimatorProperty.scale))
        _numberRow(
            context,
            l10n.transformScale,
            'anim-scale-$index',
            [a.scaleX, a.scaleY],
            -10000,
            10000,
            100,
            (s) => put(animatorWith(a, scaleX: s[0], scaleY: s[1]))),
      if (animatorShowsProperty(a, AnimatorProperty.opacity))
        _numberRow(
            context,
            l10n.transformOpacity,
            'anim-opacity-$index',
            [a.opacity],
            0,
            100,
            100,
            (s) => put(animatorWith(a, opacity: s.first))),
      // A fill **offset**, added to the layer's own colour in scene-linear —
      // so it can be negative, and so two animators tinting the same letter
      // add up. That is why it is three numbers rather than a swatch: a
      // colour picker has no way to say "a bit less red than the layer".
      if (animatorShowsProperty(a, AnimatorProperty.fill))
        _numberRow(
            context,
            l10n.textAnimatorFillOffset,
            'anim-fill-$index',
            [a.fillR, a.fillG, a.fillB],
            -10,
            10,
            0,
            (s) => put(animatorWith(a, fillR: s[0], fillG: s[1], fillB: s[2]))),
      // Variable font axes: where a reached letter is set on the font's own
      // axes, a place rather than an amount, so 900 is the heaviest weight
      // whatever the layer is set at.
      for (var k = 0; k < a.axes.length; k++)
        _axisRow(context, animators, index, k, put),
    ];
  }

  /// Every layer in the comp, for the Proximity target.
  List<BridgeLayerEntry> _layers(BuildContext context) =>
      Provider.of<LumitUiState>(context, listen: false).model.layers;

  /// One axis an animator moves the letters along: a keyable place, and a
  /// way to take it off.
  Widget _axisRow(
    BuildContext context,
    List<BridgeTextAnimator> animators,
    int index,
    int k,
    ValueChanged<BridgeTextAnimator> put,
  ) {
    final a = animators[index];
    final axis = a.axes[k];
    final style = layer.getText()?.style;
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
    ValueChanged<List<BridgeScalar>> write,
  ) {
    // The playhead is listened to here, per row, rather than by the panel above
    // — and only where a channel can actually move under it. A card that
    // redrew whole on every frame of a scrub is what made the playhead lag.
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;
    if (scalars.every((s) => s is BridgeScalar_Static)) {
      return _numberRowAt(context, label, keyName, scalars, min, max, resetTo,
          write, playhead.value);
    }
    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, at, _) => _numberRowAt(
          context, label, keyName, scalars, min, max, resetTo, write, at),
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
  ) {
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
    void commit(int axis, num value) => write([
          for (var i = 0; i < scalars.length; i++)
            if (i == axis)
              scalarWithValueAt(scalars[i], value.toDouble(), comp, at)
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
        name: Text(t.propertyCase(label),
            style: t.body, overflow: TextOverflow.ellipsis),
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
                    value: shown(scalars[i]),
                    min: min,
                    max: max,
                    resetTo: resetTo,
                    decimals: 1,
                    onChanged: (v) => commit(i, v),
                  ),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }

  /// A row whose control is a choice, not a curve — so a plain name, with no
  /// stopwatch to promise a keyframe that cannot exist.
  Widget _plainRow(BuildContext context, String label, Widget control) {
    final t = ThemeScope.of(context).theme;
    return fxTwoColumnRow(
      context: context,
      name: Text(t.propertyCase(label),
          style: t.body, overflow: TextOverflow.ellipsis),
      control: SizedBox(width: _cellWidth + 60, child: control),
    );
  }
}

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
          freshTextAnimator(
              l10n.textAnimatorDefaultName(document.animators.length + 1)),
      ],
    ),
  );
  return true;
}

/// A fresh animator: the whole set of properties, every one of them at a value
/// that changes nothing until it is moved.
BridgeTextAnimator freshTextAnimator(String name) => BridgeTextAnimator(
      name: name,
      selector: const BridgeRangeSelector(
        start: BridgeScalar.static_(0),
        end: BridgeScalar.static_(100),
        offset: BridgeScalar.static_(0),
        basis: BridgeSelectorBasis.characters,
        shape: BridgeSelectorShape.square,
        kind: BridgeSelectorKind.range,
        radius: BridgeScalar.static_(200),
        invert: false,
        minAmount: BridgeScalar.static_(0),
        maxAmount: BridgeScalar.static_(100),
        wiggles: BridgeScalar.static_(2),
        correlation: BridgeScalar.static_(50),
        seed: 0,
      ),
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
      axes: const [],
      // Nothing listed yet: Add ▸ Property builds it up, as After Effects'
      // blank animator.
      properties: const [],
    );

/// Whether any of [a]'s rows are keyed.
bool animatorHasKeys(BridgeTextAnimator a) => [
      a.selector.start,
      a.selector.end,
      a.selector.offset,
      a.positionX,
      a.positionY,
      a.rotation,
      a.positionZ,
      a.rotationX,
      a.rotationY,
      a.scaleX,
      a.scaleY,
      a.opacity,
      a.fillR,
      a.fillG,
      a.fillB,
      a.selector.radius,
      for (final x in a.axes) x.value,
    ].any((s) => s is BridgeScalar_Keyframed);

/// [animator] with one channel replaced — the generated struct is immutable and
/// every field required, so there is one copy helper rather than eleven
/// constructor calls spread through the rows.
BridgeTextAnimator animatorWith(
  BridgeTextAnimator a, {
  String? name,
  BridgeRangeSelector? selector,
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
  List<BridgeAnimatorAxis>? axes,
  List<String>? properties,
}) =>
    BridgeTextAnimator(
      name: name ?? a.name,
      selector: selector ?? a.selector,
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
  // A Proximity selector's reach, a Wiggly selector's four, then the font
  // axes in the animator's order.
  radius,
  minAmount,
  maxAmount,
  wiggles,
  correlation,
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

/// The numbers [a] has rows for: its selector's own (a Range's three, a
/// Proximity's radius, a Wiggly's four), the property groups it shows ([solid]
/// adds the 3D ones) and one per font axis it moves.
List<TextAnimatorValue> textAnimatorValuesOf(BridgeTextAnimator a,
    {required bool solid}) {
  final kind = a.selector.kind;
  return [
    for (final value in TextAnimatorValue.values)
      if (switch (value) {
        TextAnimatorValue.rangeStart ||
        TextAnimatorValue.rangeEnd ||
        TextAnimatorValue.rangeOffset =>
          kind == BridgeSelectorKind.range,
        TextAnimatorValue.radius => kind == BridgeSelectorKind.proximity,
        TextAnimatorValue.minAmount ||
        TextAnimatorValue.maxAmount ||
        TextAnimatorValue.wiggles ||
        TextAnimatorValue.correlation =>
          kind == BridgeSelectorKind.wiggly,
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
  positionZ('position_z'),
  rotationX('rotation_x'),
  rotationY('rotation_y');

  /// How the document names it.
  final String key;
  const AnimatorProperty(this.key);

  /// Only a solid layer's letters move in depth and turn in 3D.
  bool get solidOnly =>
      this == positionZ || this == rotationX || this == rotationY;

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
        positionZ => const [TextAnimatorValue.positionZ],
        rotationX => const [TextAnimatorValue.rotationX],
        rotationY => const [TextAnimatorValue.rotationY],
      };

  /// What the Add ▸ Property menu calls it.
  String get label => switch (this) {
        position => l10n.transformPosition,
        rotation => l10n.transformRotation,
        scale => l10n.transformScale,
        opacity => l10n.transformOpacity,
        fill => l10n.textAnimatorFillOffset,
        positionZ => '${l10n.transformPosition} z',
        rotationX => '${l10n.transformRotation} x',
        rotationY => '${l10n.transformRotation} y',
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
  if (listed == null || listed.contains(p.key)) return true;
  return p.numbers.any((v) =>
      textAnimatorScalarOf(a, v) !=
      BridgeScalar.static_(textAnimatorDefault(v)));
}

/// [a] listing [p] too.
BridgeTextAnimator animatorWithProperty(
    BridgeTextAnimator a, AnimatorProperty p) {
  final listed = a.properties;
  if (listed == null || listed.contains(p.key)) return a;
  return animatorWith(a, properties: [...listed, p.key]);
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
  final listed =
      a.properties ?? [for (final other in AnimatorProperty.values) other.key];
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
      _ when textAnimatorAxisIndex(value) != null =>
        l10n.textAnimatorAxisLabel(fontAxisName('', '', _axisTag(a, value))),
      TextAnimatorValue.rangeStart => l10n.textAnimatorRangeStart,
      TextAnimatorValue.rangeEnd => l10n.textAnimatorRangeEnd,
      TextAnimatorValue.rangeOffset => l10n.textAnimatorRangeOffset,
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
      TextAnimatorValue.maxAmount ||
      TextAnimatorValue.rangeEnd ||
      TextAnimatorValue.scaleX ||
      TextAnimatorValue.scaleY ||
      TextAnimatorValue.opacity =>
        100,
      _ => 0,
    };

/// Which of [a]'s numbers [value] names.
BridgeScalar textAnimatorScalarOf(
        BridgeTextAnimator a, TextAnimatorValue value) =>
    switch (value) {
      TextAnimatorValue.rangeStart => a.selector.start,
      TextAnimatorValue.rangeEnd => a.selector.end,
      TextAnimatorValue.rangeOffset => a.selector.offset,
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
      TextAnimatorValue.radius => a.selector.radius,
      TextAnimatorValue.minAmount => a.selector.minAmount,
      TextAnimatorValue.maxAmount => a.selector.maxAmount,
      TextAnimatorValue.wiggles => a.selector.wiggles,
      TextAnimatorValue.correlation => a.selector.correlation,
      _ => switch (textAnimatorAxisIndex(value)) {
          final i? when i < a.axes.length => a.axes[i].value,
          _ => const BridgeScalar.static_(0),
        },
    };

/// [a] with the one number [value] names replaced.
BridgeTextAnimator animatorWithScalar(
        BridgeTextAnimator a, TextAnimatorValue value, BridgeScalar to) =>
    switch (value) {
      TextAnimatorValue.rangeStart =>
        animatorWith(a, selector: selectorWith(a.selector, start: to)),
      TextAnimatorValue.rangeEnd =>
        animatorWith(a, selector: selectorWith(a.selector, end: to)),
      TextAnimatorValue.rangeOffset =>
        animatorWith(a, selector: selectorWith(a.selector, offset: to)),
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
      TextAnimatorValue.radius =>
        animatorWith(a, selector: selectorWith(a.selector, radius: to)),
      TextAnimatorValue.minAmount =>
        animatorWith(a, selector: selectorWith(a.selector, minAmount: to)),
      TextAnimatorValue.maxAmount =>
        animatorWith(a, selector: selectorWith(a.selector, maxAmount: to)),
      TextAnimatorValue.wiggles =>
        animatorWith(a, selector: selectorWith(a.selector, wiggles: to)),
      TextAnimatorValue.correlation =>
        animatorWith(a, selector: selectorWith(a.selector, correlation: to)),
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
}) {
  final document = layer.getText();
  if (document == null || index >= document.animators.length) return;
  layer.setText(
    document: BridgeTextDocument(
      text: document.text,
      expression: document.expression,
      size: document.size,
      fill: document.fill,
      path: document.path,
      pathOffset: document.pathOffset,
      style: document.style,
      paragraph: document.paragraph,
      animators: [
        for (var i = 0; i < document.animators.length; i++)
          if (i == index)
            animatorWithScalar(document.animators[i], value, to)
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
  BridgeSelectorKind? kind,
  UuidValue? target,
  bool clearTarget = false,
  BridgeScalar? radius,
  bool? invert,
  BridgeScalar? minAmount,
  BridgeScalar? maxAmount,
  BridgeScalar? wiggles,
  BridgeScalar? correlation,
  int? seed,
}) =>
    BridgeRangeSelector(
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
      kind: kind ?? s.kind,
      target: clearTarget ? null : (target ?? s.target),
      radius: radius ?? s.radius,
      invert: invert ?? s.invert,
    );

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
              for (final (kind, label) in [
                (BridgeSelectorKind.range, l10n.textAnimatorSelectorRange),
                (
                  BridgeSelectorKind.proximity,
                  l10n.textAnimatorSelectorProximity
                ),
                (BridgeSelectorKind.wiggly, l10n.textAnimatorSelectorWiggly),
              ])
                MenuRow(
                  key: ValueKey<String>(
                      'text-animator-selector-${kind.name}-$index'),
                  enabled: a.selector.kind != kind,
                  onPressed: () {
                    dismiss();
                    put(animatorWithSelectorKind(a, kind));
                  },
                  child: Text(label),
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
  BridgeTextAnimator fresh() => freshTextAnimator(
      l10n.textAnimatorDefaultName(document.animators.length + 1));
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
          onPressed: () => add([animatorWithProperty(fresh(), p)]),
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
                      animatorWith(fresh(), axes: [
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
    ],
  ];
}

/// [layer]'s animators with [index] taken off, or all of them when [index] is
/// null: what Delete does on an animator's heading and on Animators.
void removeTextAnimators(LayerReference layer, {int? index}) {
  final document = layer.getText();
  if (document == null || document.animators.isEmpty) return;
  layer.setText(
    document: BridgeTextDocument(
      text: document.text,
      expression: document.expression,
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
