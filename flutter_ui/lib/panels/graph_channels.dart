// The graph editor's channel model: what a curve is, where it came from, and
// the small helpers every part of the pane reads keys through. Split out of
// graph_editor_frb.dart, which re-exports it.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'effect_param_row_frb.dart';
import 'graph_maths.dart';
import 'layer_fold_frb.dart';
import 'text_animator_rows_frb.dart';
import 'transform_rows_frb.dart';

/// A value drag in flight **in the layer area**, published for the graph pane to
/// draw.
///
/// The row stages its value in Dart and commits once on release, so the
/// read model — and therefore the curve — still holds the old one until the
/// pointer comes up. The pane cannot ask for it, because it is not in the
/// document; the row publishes it here instead, exactly as a bar drag publishes
/// its travel for the waveform lane (`BarDragPreview`). Null between
/// gestures.
///
/// The layer plus one channel selector — a transform axis, an effect
/// parameter, or the Retime — is what the two sides have in common, so
/// dragging Position x leaves y where it is. An **unkeyed** property is drawn
/// at its new value and gains no diamond: the drag is not planting a key, and
/// a glyph would say it was.
final ValueNotifier<RowValueDrag?> rowValueDrag = ValueNotifier(null);

/// One tick of a layer-area value drag: which channel, and what it holds.
class RowValueDrag {
  final String layer;

  /// A transform axis (`BridgeTransformProp.name`), or null.
  final String? prop;

  /// An effect parameter, or nulls.
  final String? effectId;
  final String? paramId;

  /// The layer's Retime channel.
  final bool retime;

  final int frame;
  final double value;

  const RowValueDrag({
    required this.layer,
    this.prop,
    this.effectId,
    this.paramId,
    this.retime = false,
    required this.frame,
    required this.value,
  });

  /// Whether [channel] is the curve this drag is editing.
  bool matches(GraphChannel channel) {
    if (channel.entry.layer.internallayerId.toString() != layer) return false;
    if (prop != null) return channel.prop?.name == prop;
    if (effectId != null) {
      return channel.effect?.id.toString() == effectId &&
          channel.param?.id == paramId;
    }
    return retime && channel.retime;
  }
}

/// Which reading of the curve is on screen (docs/07 §5.1).
enum GraphLens { value, speed }

/// One animatable channel on the graph: a single scalar curve, where it came
/// from, and how to write it back.
class GraphChannel {
  /// The fold-row path of the property row this channel belongs to — the
  /// outline's selection speaks in these.
  final String path;

  /// Unique per curve: a two-axis property has one channel per axis.
  final String id;
  final String label;

  /// Its stroke: an index into the theme's `curve` palette, assigned in
  /// selection order so the outline can tint the row's text to match.
  final int colourIndex;
  final BridgeScalar scalar;
  final BridgeLayerEntry entry;

  /// Set for a transform channel; null for an effect parameter.
  final BridgeTransformProp? prop;

  /// Set for an effect parameter channel.
  final BridgeEffectInstanceInfo? effect;
  final BridgeParamInfo? param;

  /// True for the layer's Retime channel, which is neither a transform
  /// property nor an effect parameter but reads and writes like both.
  final bool retime;

  /// Set for one of a mask's values: the mask it belongs to, and which
  /// of its values this is.
  final BridgeMask? mask;
  final MaskValue? maskValue;

  /// Which point a per-point feather channel belongs to; `-1` on every
  /// other channel.
  final int maskVertex;

  /// Set for one of a Text layer's animator numbers: which animator in
  /// the layer's list, and which of its numbers this is.
  final int animator;
  final TextAnimatorValue? animatorValue;

  /// True for a mask's **shape**. A path has no value to plot, so what
  /// this channel carries is the interpolation parameter — counted up, one per
  /// key — and both lenses draw its *slope*: the rate the shape is changing
  /// at. That is the one honest curve a path has, and it is what After Effects
  /// draws for a mask path.
  bool get isMaskPath => maskValue == MaskValue.path;

  const GraphChannel({
    required this.path,
    required this.id,
    required this.label,
    required this.colourIndex,
    required this.scalar,
    required this.entry,
    this.prop,
    this.effect,
    this.param,
    this.retime = false,
    this.mask,
    this.maskValue,
    this.maskVertex = -1,
    this.animator = -1,
    this.animatorValue,
  });

  /// The hard bounds the engine will clamp a written value to, where this
  /// channel has any (docs/08 §1.2) — either side open on its own.
  ///
  /// Read so the curve drawn *during* a drag can agree with where the key will
  /// actually come to rest: the commit and the preview pixels have always
  /// clamped, and a line that overshot on the way was the drawing disagreeing
  /// with both. A transform, a mask value and the Retime are unbounded here,
  /// which is the honest answer — their limits are not a parameter's range.
  (double?, double?) get hardBounds => switch (param?.kind) {
        BridgeParamKind_Float(:final hardMin, :final hardMax) => (
            hardMin,
            hardMax
          ),
        BridgeParamKind_Int(:final hardMin, :final hardMax) => (
            hardMin?.toDouble(),
            hardMax?.toDouble()
          ),
        // A closed range is the travel *and* the bound — that is what closed
        // means.
        BridgeParamKind_Slider(:final min, :final max) => (min, max),
        _ => (null, null),
      };

  /// [value] held inside this channel's hard bounds.
  double clampToBounds(double value) {
    final (low, high) = hardBounds;
    if (low != null && value < low) return low;
    if (high != null && value > high) return high;
    return value;
  }

  /// The curve's value at `t` seconds as the pane should **draw** it: the
  /// engine's own evaluation, held inside the hard range.
  ///
  /// Clamping the dragged key was only half of M28. A cubic span between two
  /// in-range keys bulges past both of them, so the line still crossed a bound
  /// the parameter cannot hold — most visibly while a key was being pushed
  /// against one, which is where it was reported. docs/08 §1.2 is plain that a
  /// hard range MUST NOT be exceeded, so a line drawn outside one is drawing a
  /// value that does not exist.
  double drawnValueAt(List<BridgeKeyframe> keys, double t) =>
      clampToBounds(evaluateKeys(keys, t));

  List<BridgeKeyframe> get keys => keysOf(scalar);
  bool get isStatic => scalar is BridgeScalar_Static;
  double get staticValue => switch (scalar) {
        BridgeScalar_Static(:final field0) => field0,
        BridgeScalar_Keyframed() => 0,
        BridgeScalar_Expression() => 0,
      };
}

/// The channels the selected property paths resolve to, in selection order —
/// entirely from the read model, so building them costs no bridge calls.
///
/// A transform row yields one channel per axis (Position → x and y, the AE
/// red/green pair); a float effect parameter yields one. Volume is not in the
/// read model (one of its deliberate exceptions) and is skipped — docs/TODO.md.
List<GraphChannel> graphChannels({
  required List<BridgeLayerEntry> layers,
  required List<String> selected,
}) {
  final out = <GraphChannel>[];
  for (final path in selected) {
    final cut = path.indexOf('/');
    if (cut <= 0) continue;
    final layerId = path.substring(0, cut);
    BridgeLayerEntry? entry;
    for (final e in layers) {
      if (e.layer.internallayerId.toString() == layerId) {
        entry = e;
        break;
      }
    }
    if (entry == null) continue;

    // Retime: one channel, source time in seconds. An ordinary curve
    // here — the lens, the handles and the interp buttons all treat it as one.
    if (path == retimePath(layerId)) {
      if (entry.info.retime case final scalar?) {
        out.add(GraphChannel(
          path: path,
          id: path,
          label: '${entry.info.name} · Retime',
          colourIndex: out.length,
          scalar: scalar,
          entry: entry,
          retime: true,
        ));
      }
      continue;
    }

    if (path.startsWith('${transformPath(layerId)}/')) {
      final lead = path.substring(path.lastIndexOf('/') + 1);
      for (final group in transformGroups(
          threeD: entry.info.switches.threeD, modes: entry.info.axisModes)) {
        if (group.axes.first.prop.name != lead) continue;
        for (final axis in group.axes) {
          out.add(GraphChannel(
            path: path,
            id: '$path@${axis.prop.name}',
            label: group.axes.length == 1
                ? '${entry.info.name} · ${group.label}'
                : '${entry.info.name} · ${group.label} ${axisLetter(group.axes.indexOf(axis))}',
            colourIndex: out.length,
            scalar: read(entry.info.transform, axis.prop),
            entry: entry,
            prop: axis.prop,
          ));
        }
        break;
      }
      continue;
    }

    // An effect's parameters, and a **layer style's** on the same road:
    // a style is an effect instance in a second list, so its curve is drawn,
    // edited and coloured by this branch rather than by a copy of it.
    final styles = path.startsWith('${stylesPath(layerId)}/');
    if (styles || path.startsWith('${effectsPath(layerId)}/')) {
      final head = styles ? stylesPath(layerId) : effectsPath(layerId);
      final rest = path.substring(head.length + 1);
      final slash = rest.indexOf('/');
      if (slash <= 0) continue;
      final effectId = rest.substring(0, slash);
      final paramId = rest.substring(slash + 1);
      for (final fx in styles ? entry.info.styles : entry.info.effects) {
        if (fx.id.toString() != effectId) continue;
        for (final param in cachedListParameters(fx.name)) {
          if (param.id != paramId) continue;
          // The kind is the control, not the storage: a Slider, an Int and an
          // Angle all cross the bridge as one Float scalar, and any of them
          // keyed is a curve (docs/08 §1.2). So the test is on the value, not
          // the kind. Naming kinds here dropped Slider once and Angle after it.
          BridgeScalar? scalar;
          for (final v in fx.values) {
            if (v.id == param.id && v.value is BridgeEffectValue_Float) {
              scalar = (v.value as BridgeEffectValue_Float).field0;
            }
          }
          if (scalar == null) continue;
          out.add(GraphChannel(
            path: path,
            id: path,
            label:
                '${entry.info.name} · ${effectLabelOf(fx.name)} · ${param.label}',
            colourIndex: out.length,
            scalar: scalar,
            entry: entry,
            effect: fx,
            param: param,
          ));
        }
      }
      continue;
    }

    // A Text layer's animator numbers. They come off the read model
    // like a mask's do, so a curve here costs no bridge call to draw.
    if (path.startsWith('${animatorsPath(layerId)}/')) {
      final rest = path.substring(animatorsPath(layerId).length + 1);
      final slash = rest.indexOf('/');
      if (slash <= 0) continue;
      final index = int.tryParse(rest.substring(0, slash));
      final valueName = rest.substring(slash + 1);
      if (index == null || index >= entry.info.textAnimators.length) continue;
      final value = TextAnimatorValue.values
          .where((v) => v.name == valueName)
          .firstOrNull;
      if (value == null) continue;
      final animator = entry.info.textAnimators[index];
      out.add(GraphChannel(
        path: path,
        id: path,
        label: '${entry.info.name} · ${animator.name} · '
            '${textAnimatorValueLabel(value)}',
        colourIndex: out.length,
        scalar: textAnimatorScalarOf(animator, value),
        entry: entry,
        animator: index,
        animatorValue: value,
      ));
      continue;
    }

    // A mask's numbers. Its shape is deliberately absent: a path has no
    // value axis to draw against, so it keeps its lane diamonds and no curve.
    if (path.startsWith('${masksPath(layerId)}/')) {
      final rest = path.substring(masksPath(layerId).length + 1);
      final slash = rest.indexOf('/');
      if (slash <= 0) continue;
      final maskId = rest.substring(0, slash);
      // A per-point feather row's path carries the point after its name:
      // `.../vertexFeather/3`.
      final rawValue = rest.substring(slash + 1);
      final tail = rawValue.indexOf('/');
      final valueName = tail < 0 ? rawValue : rawValue.substring(0, tail);
      final vertex =
          tail < 0 ? -1 : int.tryParse(rawValue.substring(tail + 1)) ?? -1;
      for (final mask in entry.info.masks) {
        if (mask.id.toString() != maskId) continue;
        final value =
            MaskValue.values.where((v) => v.name == valueName).firstOrNull;
        if (value == null) break;
        // The shape's channel carries its keys as the counted-up interpolation
        // parameter; a still shape has none and draws nothing.
        if (value == MaskValue.path && mask.pathKeys.isEmpty) break;
        out.add(GraphChannel(
          path: path,
          id: path,
          label: '${entry.info.name} · ${mask.name} · '
              '${maskValueLabel(value, vertex)}',
          colourIndex: out.length,
          scalar: value == MaskValue.path
              ? BridgeScalar.keyframed(mask.pathKeys)
              : maskScalarOf(mask, value, vertex),
          entry: entry,
          mask: mask,
          maskValue: value,
          maskVertex: vertex,
        ));
        break;
      }
    }
  }
  return out;
}

String axisLetter(int i) => switch (i) { 0 => 'x', 1 => 'y', _ => 'z' };

/// [keys] with a key of [value] at [frame] — replacing the one already there,
/// because two keys at one time is not a curve the engine will take.
List<BridgeKeyframe> withKeyAt(
  List<BridgeKeyframe> keys,
  double frame,
  double value,
  double fps,
  int fpsNum,
  int fpsDen,
) {
  final merged = <double, BridgeKeyframe>{
    for (final k in keys) keyFrame(k, fps): k,
  };
  merged[frame] = keyframeAmong(
    keys,
    timeOfSubframe(frame, fpsNum, fpsDen),
    value,
  );
  final frames = merged.keys.toList()..sort();
  return [for (final f in frames) merged[f]!];
}

/// A key's position on the frame axis, fractional (a key may sit between
/// frames with the magnet off).
double keyFrame(BridgeKeyframe key, double fps) =>
    rationalSeconds(key.time) * fps;

/// A number as this pane's readouts write it: whole numbers plain, everything
/// else to two places — the same hand the dope sheet's own numbers are set in.
String graphNumberText(double v) =>
    v == v.roundToDouble() ? v.round().toString() : v.toStringAsFixed(2);
