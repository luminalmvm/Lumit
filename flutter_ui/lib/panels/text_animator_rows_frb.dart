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

import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../widgets/controls.dart';
import 'fx_section.dart';
import 'keyframe_controls_frb.dart';

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

  const TextAnimatorRowsFrb({
    super.key,
    required this.layer,
    required this.onChanged,
    required this.comp,
    required this.playheadFrame,
    required this.onSeek,
    required this.open,
    required this.onToggle,
  });

  @override
  Widget build(BuildContext context) {
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
    rows.add(fxTwoColumnRow(
      context: context,
      name: const SizedBox.shrink(),
      control: HouseButton(
        key: const ValueKey('text-animator-add'),
        small: true,
        onPressed: () {
          if (addTextAnimator(layer)) onChanged();
        },
        child: Text(l10n.textAnimatorAdd),
      ),
    ));

    return FxSection(
      title: l10n.textAnimatorsSection,
      open: open,
      onToggle: onToggle,
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
        name: Text(a.name.toUpperCase(),
            style: t.kickerOn, overflow: TextOverflow.ellipsis),
        control: HouseButton(
          key: ValueKey<String>('text-animator-remove-$index'),
          small: true,
          frameless: true,
          onPressed: () => write([
            for (var i = 0; i < animators.length; i++)
              if (i != index) animators[i],
          ]),
          child: Text(l10n.textAnimatorRemove),
        ),
      ),
      // The range: which letters this animator reaches, in per cent of the
      // words. Offset is the one a cascade is keyed on, which is why it is a
      // number of its own rather than something the two ends share.
      _numberRow(context, l10n.textAnimatorRangeStart, 'range-start-$index',
          [a.selector.start], -1000, 1000,
          (s) => putSelector(selectorWith(a.selector, start: s.first))),
      _numberRow(context, l10n.textAnimatorRangeEnd, 'range-end-$index',
          [a.selector.end], -1000, 1000,
          (s) => putSelector(selectorWith(a.selector, end: s.first))),
      _numberRow(context, l10n.textAnimatorRangeOffset, 'range-offset-$index',
          [a.selector.offset], -1000, 1000,
          (s) => putSelector(selectorWith(a.selector, offset: s.first))),
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
          },
          onChanged: (s) => putSelector(selectorWith(a.selector, shape: s)),
        ),
      ),
      _numberRow(context, l10n.transformPosition, 'anim-position-$index',
          [a.positionX, a.positionY], -100000, 100000,
          (s) => put(animatorWith(a, positionX: s[0], positionY: s[1]))),
      _numberRow(context, l10n.transformRotation, 'anim-rotation-$index',
          [a.rotation], -100000, 100000,
          (s) => put(animatorWith(a, rotation: s.first))),
      _numberRow(context, l10n.transformScale, 'anim-scale-$index',
          [a.scaleX, a.scaleY], -10000, 10000,
          (s) => put(animatorWith(a, scaleX: s[0], scaleY: s[1]))),
      _numberRow(context, l10n.transformOpacity, 'anim-opacity-$index',
          [a.opacity], 0, 100,
          (s) => put(animatorWith(a, opacity: s.first))),
      // A fill **offset**, added to the layer's own colour in scene-linear —
      // so it can be negative, and so two animators tinting the same letter
      // add up. That is why it is three numbers rather than a swatch: a
      // colour picker has no way to say "a bit less red than the layer".
      _numberRow(context, l10n.textAnimatorFillOffset, 'anim-fill-$index',
          [a.fillR, a.fillG, a.fillB], -10, 10,
          (s) => put(animatorWith(a, fillR: s[0], fillG: s[1], fillB: s[2]))),
    ];
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
    ValueChanged<List<BridgeScalar>> write,
  ) {
    // The playhead is listened to here, per row, rather than by the panel above
    // — and only where a channel can actually move under it. A card that
    // redrew whole on every frame of a scrub is what made the playhead lag.
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;
    if (scalars.every((s) => s is BridgeScalar_Static)) {
      return _numberRowAt(
          context, label, keyName, scalars, min, max, write, playhead.value);
    }
    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, at, _) =>
          _numberRowAt(context, label, keyName, scalars, min, max, write, at),
    );
  }

  Widget _numberRowAt(
    BuildContext context,
    String label,
    String keyName,
    List<BridgeScalar> scalars,
    double min,
    double max,
    ValueChanged<List<BridgeScalar>> write,
    int at,
  ) {
    final t = ThemeScope.of(context).theme;
    // An animated channel shows what its curve reads at the playhead, sampled
    // engine-side — the same answer the render will use, rather than a second
    // interpolation living in the view.
    double shown(BridgeScalar s) => switch (s) {
          BridgeScalar_Static(:final field0) => field0,
          BridgeScalar_Keyframed() || BridgeScalar_Expression() =>
            sampledScalar(s, timeOfFrame(comp, at)),
        };
    void commit(int axis, num value) => write([
          for (var i = 0; i < scalars.length; i++)
            if (i == axis)
              scalarWithValueAt(scalars[i], value.toDouble(), comp, at)
            else
              scalars[i],
        ]);

    return fxTwoColumnRow(
      context: context,
      keyframeControls: KeyframeControlsFrb(
        scalars: scalars,
        onWrite: write,
        comp: comp,
        playheadFrame: at,
        onSeek: onSeek,
        rowKey: keyName,
        // These rows only ever draw in the Effect controls panel, on its fixed
        // columns.
        fixedColumns: true,
      ),
      name: Text(label, style: t.body, overflow: TextOverflow.ellipsis),
      control: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (var i = 0; i < scalars.length; i++) ...[
            if (i > 0) const SizedBox(width: 6),
            SizedBox(
              width: _cellWidth,
              child: DragValueField(
                key: ValueKey<String>('$keyName-$i'),
                value: shown(scalars[i]),
                min: min,
                max: max,
                decimals: 1,
                onChanged: (v) => commit(i, v),
              ),
            ),
          ],
        ],
      ),
    );
  }

  /// A row whose control is a choice, not a curve — so a plain name, with no
  /// stopwatch to promise a keyframe that cannot exist.
  Widget _plainRow(BuildContext context, String label, Widget control) {
    final t = ThemeScope.of(context).theme;
    return fxTwoColumnRow(
      context: context,
      name: Text(label, style: t.body, overflow: TextOverflow.ellipsis),
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
bool addTextAnimator(LayerReference layer) {
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
      animators: [
        ...document.animators,
        _freshAnimator(
            l10n.textAnimatorDefaultName(document.animators.length + 1)),
      ],
    ),
  );
  return true;
}

/// A fresh animator: the whole set of properties, every one of them at a value
/// that changes nothing until it is moved.
BridgeTextAnimator _freshAnimator(String name) => BridgeTextAnimator(
      name: name,
      selector: const BridgeRangeSelector(
        start: BridgeScalar.static_(0),
        end: BridgeScalar.static_(100),
        offset: BridgeScalar.static_(0),
        basis: BridgeSelectorBasis.characters,
        shape: BridgeSelectorShape.square,
      ),
      positionX: const BridgeScalar.static_(0),
      positionY: const BridgeScalar.static_(0),
      rotation: const BridgeScalar.static_(0),
      scaleX: const BridgeScalar.static_(100),
      scaleY: const BridgeScalar.static_(100),
      opacity: const BridgeScalar.static_(100),
      fillR: const BridgeScalar.static_(0),
      fillG: const BridgeScalar.static_(0),
      fillB: const BridgeScalar.static_(0),
    );

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
  BridgeScalar? scaleX,
  BridgeScalar? scaleY,
  BridgeScalar? opacity,
  BridgeScalar? fillR,
  BridgeScalar? fillG,
  BridgeScalar? fillB,
}) =>
    BridgeTextAnimator(
      name: name ?? a.name,
      selector: selector ?? a.selector,
      positionX: positionX ?? a.positionX,
      positionY: positionY ?? a.positionY,
      rotation: rotation ?? a.rotation,
      scaleX: scaleX ?? a.scaleX,
      scaleY: scaleY ?? a.scaleY,
      opacity: opacity ?? a.opacity,
      fillR: fillR ?? a.fillR,
      fillG: fillG ?? a.fillG,
      fillB: fillB ?? a.fillB,
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
  rotation,
  scaleX,
  scaleY,
  opacity,
  fillR,
  fillG,
  fillB,
}

/// What one of an animator's numbers is called — shared by the Timeline row,
/// the graph channel and anything else that has to name one.
String textAnimatorValueLabel(TextAnimatorValue value) => switch (value) {
      TextAnimatorValue.rangeStart => l10n.textAnimatorRangeStart,
      TextAnimatorValue.rangeEnd => l10n.textAnimatorRangeEnd,
      TextAnimatorValue.rangeOffset => l10n.textAnimatorRangeOffset,
      TextAnimatorValue.positionX => '${l10n.transformPosition} x',
      TextAnimatorValue.positionY => '${l10n.transformPosition} y',
      TextAnimatorValue.rotation => l10n.transformRotation,
      TextAnimatorValue.scaleX => '${l10n.transformScale} x',
      TextAnimatorValue.scaleY => '${l10n.transformScale} y',
      TextAnimatorValue.opacity => l10n.transformOpacity,
      TextAnimatorValue.fillR => '${l10n.textAnimatorFillOffset} r',
      TextAnimatorValue.fillG => '${l10n.textAnimatorFillOffset} g',
      TextAnimatorValue.fillB => '${l10n.textAnimatorFillOffset} b',
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
      TextAnimatorValue.rotation => a.rotation,
      TextAnimatorValue.scaleX => a.scaleX,
      TextAnimatorValue.scaleY => a.scaleY,
      TextAnimatorValue.opacity => a.opacity,
      TextAnimatorValue.fillR => a.fillR,
      TextAnimatorValue.fillG => a.fillG,
      TextAnimatorValue.fillB => a.fillB,
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
      TextAnimatorValue.rotation => animatorWith(a, rotation: to),
      TextAnimatorValue.scaleX => animatorWith(a, scaleX: to),
      TextAnimatorValue.scaleY => animatorWith(a, scaleY: to),
      TextAnimatorValue.opacity => animatorWith(a, opacity: to),
      TextAnimatorValue.fillR => animatorWith(a, fillR: to),
      TextAnimatorValue.fillG => animatorWith(a, fillG: to),
      TextAnimatorValue.fillB => animatorWith(a, fillB: to),
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
}) =>
    BridgeRangeSelector(
      start: start ?? s.start,
      end: end ?? s.end,
      offset: offset ?? s.offset,
      basis: basis ?? s.basis,
      shape: shape ?? s.shape,
    );
