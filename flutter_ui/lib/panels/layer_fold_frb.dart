// What a layer shows when it is twirled open in the Timeline: the section
// headings, and the property rows under whichever of them are open.
//
// **One list, two halves.** The Timeline is a table: names on the left, lanes on
// the right, and a row of one has to be the same height as the row of the other
// or every bar drifts away from its own layer. So the fold-out is worked out
// *once*, as a list of rows, and each half walks the same list — the outline
// drawing each row's controls, the lane side leaving each row's height. Nothing
// has to be kept in step by hand because there is only one description of it.
//
// **The groups.** Transform always (every layer has one), Effects when the layer
// has any, Audio only when the layer's source actually carries sound
// (docs/07 §4.3), and Retime above them all when the layer has one. Masks are
// not built yet. A group is a heading with
// its own twirl, so opening a layer shows a tidy list of headings and you open
// only the one you want — which is what the spec asks for and what keeps a busy
// comp from becoming a wall of numbers.

import 'package:flutter/services.dart';

import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/retime.dart';

import 'effect_param_row_frb.dart';
import 'graph_maths.dart';
import 'transform_rows_frb.dart';

/// One row of a layer's fold-out.
sealed class LayerFoldRow {
  /// How far this row is indented — 1 for a section heading, 2 for a property
  /// under one, 3 for a parameter under an effect.
  final int depth;
  const LayerFoldRow(this.depth);
}

/// A section heading with its own twirl: Transform, Effects, an effect, Audio.
final class FoldGroupRow extends LayerFoldRow {
  /// Identifies this group in the open set. Built from ids, not labels, so two
  /// effects of the same kind on one layer open independently.
  final String path;
  final String label;
  final bool open;
  const FoldGroupRow({
    required this.path,
    required this.label,
    required this.open,
    required int depth,
  }) : super(depth);
}

/// One transform property group — Position, Scale, and so on — with the
/// layer's transform read once for the whole fold (K-183).
final class FoldTransformRow extends LayerFoldRow {
  final TransformGroup group;
  final BridgeTransform transform;
  const FoldTransformRow(this.group, this.transform, {required int depth})
      : super(depth);
}

/// One parameter of one effect — everything the row draws, from the read
/// model (K-184). Plain data; a write reads fresh instance handles at commit.
final class FoldEffectParamRow extends LayerFoldRow {
  final BridgeEffectInstanceInfo info;
  final BridgeParamInfo param;

  /// This parameter's current value, or null when the instance does not carry
  /// it (a schema newer than the saved document).
  final BridgeEffectValue? value;
  const FoldEffectParamRow(this.info, this.param, this.value,
      {required int depth})
      : super(depth);
}

/// The layer's Volume.
final class FoldVolumeRow extends LayerFoldRow {
  /// The Volume scalar, read once per document revision by the panel and
  /// carried here so the row draws without a bridge call (K-184). Null only
  /// for a caller that supplied none, which the panel never is.
  final BridgeScalar? scalar;
  const FoldVolumeRow({this.scalar, required int depth}) : super(depth);
}

/// The layer's Retime (K-197): source time in seconds, keyframable like any
/// other property. It sits above Transform rather than inside it, and only
/// appears on a layer that has been given one (Ctrl+Alt+T) — which is why the
/// scalar rides on the row: `null` retime means no row at all, so a row that
/// exists always has a curve to draw.
final class FoldRetimeRow extends LayerFoldRow {
  final BridgeScalar scalar;
  const FoldRetimeRow(this.scalar, {required int depth}) : super(depth);
}

/// One mask on the layer (K-222): its name, and the switches that decide how it
/// gates the picture.
final class FoldMaskRow extends LayerFoldRow {
  final BridgeMask mask;
  const FoldMaskRow(this.mask, {required int depth}) : super(depth);
}

/// Which of a mask's animatable values a [FoldMaskValueRow] carries (K-340).
///
/// [path] is the shape itself: a value with no number, so its row carries a
/// stopwatch and diamonds but no field (K-339).
enum MaskValue { path, opacity, feather, expansion, vertexFeather }

/// One of a mask's values — its shape, opacity, feather or expansion — on a
/// row of its own under the mask (K-222, K-340).
///
/// A row rather than another control squeezed onto the mask's own row: the
/// value column holds one field, every other number in the fold-out has a row
/// with its name on it, and a property without a row of its own has nowhere to
/// put the stopwatch that animates it.
final class FoldMaskValueRow extends LayerFoldRow {
  final BridgeMask mask;
  final MaskValue value;

  /// Which vertex's own feather this row carries, for
  /// [MaskValue.vertexFeather] (K-445). `-1` on every other row, which have
  /// one value each and no point to belong to.
  final int vertex;
  const FoldMaskValueRow(this.mask, this.value,
      {required int depth, this.vertex = -1})
      : super(depth);
}

/// One piece of a shape layer's art (K-237): its name, its fill and its
/// outline — the row that makes a drawn shape editable after the fact.
final class FoldShapeRow extends LayerFoldRow {
  final BridgeShapeItem item;
  const FoldShapeRow(this.item, {required int depth}) : super(depth);
}

/// One paint stroke on the layer (K-227): its name, so a stroke can be found,
/// renamed and deleted after it was painted.
final class FoldStrokeRow extends LayerFoldRow {
  final BridgeStroke stroke;
  const FoldStrokeRow(this.stroke, {required int depth}) : super(depth);
}

/// One control of a footage layer's Flow group (K-088, K-331). Which control
/// is the [kind]; all of them read and write the whole group in one op, so a
/// row needs nothing but its own identity.
///
/// The Input rate is the one animatable member, so it is the one that carries a
/// scalar and draws diamonds on its lane.
final class FoldFlowRow extends LayerFoldRow {
  final FlowRowKind kind;

  /// The Input rate's curve; null on every other kind.
  final BridgeScalar? rate;

  /// The whole group's parameters, read once per document revision by the
  /// panel and carried here so the row draws without a bridge call (K-184).
  /// Null only for a caller that supplied none, which the panel never is.
  final BridgeFlowParams? params;
  const FoldFlowRow(this.kind, {this.rate, this.params, required int depth})
      : super(depth);
}

/// The controls of the Flow group, in the order they are shown.
///
/// Resolution first because it is the one that costs money, then the rate (what
/// frames flow works between), then how hard it looks, then what it does where
/// it cannot see.
enum FlowRowKind {
  resolution,
  inputRate,
  detail,
  smoothness,
  occlusion,
  fallback,
  hudGuard,
  always;

  /// The row's shown name — a getter rather than a stored constant so each
  /// read speaks the current language.
  String get label => switch (this) {
        resolution => l10n.flowResolution,
        inputRate => l10n.flowInputRate,
        detail => l10n.flowVectorDetail,
        smoothness => l10n.flowSmoothness,
        occlusion => l10n.flowOcclusion,
        fallback => l10n.flowFallback,
        hudGuard => l10n.flowHudGuard,
        always => l10n.flowAlwaysOn,
      };
}

/// The waveform lane (K-172): the outline names it, the lane side draws the
/// layer's source peaks through its live in/out/offset.
final class FoldWaveformRow extends LayerFoldRow {
  const FoldWaveformRow({required int depth}) : super(depth);
}

/// The keyframes a fold row shows as diamonds on its lane (docs/07 §4.3), or
/// empty for rows with nothing keyed. A multi-axis transform row reads its
/// lead axis: the axes key together, so one axis's times are the row's.
List<BridgeKeyframe> laneKeysOf(LayerFoldRow row) => switch (row) {
      FoldTransformRow(:final group, :final transform) => switch (
            read(transform, group.axes.first.prop)) {
          BridgeScalar_Keyframed(:final field0) => field0,
          BridgeScalar_Static() => const [],
          BridgeScalar_Expression() => const [],
        },
      FoldRetimeRow(:final scalar) => switch (scalar) {
          BridgeScalar_Keyframed(:final field0) => field0,
          BridgeScalar_Static() => const [],
          BridgeScalar_Expression() => const [],
        },
      FoldFlowRow(:final rate) => switch (rate) {
          BridgeScalar_Keyframed(:final field0) => field0,
          _ => const [],
        },
      FoldEffectParamRow(:final value) => switch (value) {
          BridgeEffectValue_Float(
            field0: BridgeScalar_Keyframed(:final field0)
          ) =>
            field0,
          _ => const [],
        },
      // A mask's numbers key like any other scalar; its **shape** keys as whole
      // paths, and those keys carry their own eases and a counted-up value
      // (K-344), so the lane draws their diamonds and the graph can draw the
      // rate the shape is changing at.
      FoldMaskValueRow(:final mask, :final value, :final vertex) =>
        value == MaskValue.path
            ? mask.pathKeys
            : switch (maskScalarOf(mask, value, vertex)) {
                BridgeScalar_Keyframed(:final field0) => field0,
                _ => const [],
              },
      _ => const [],
    };

/// What a mask's value row is called — shared by the row, the graph channel
/// and anything else that has to name one.
String maskValueLabel(MaskValue value, [int vertex = -1]) => switch (value) {
      MaskValue.path => l10n.maskPath,
      MaskValue.opacity => l10n.maskOpacity,
      MaskValue.feather => l10n.maskFeather,
      MaskValue.expansion => l10n.maskExpansion,
      // Counted from one, as the person drawing counts points.
      MaskValue.vertexFeather => l10n.maskVertexFeather(vertex + 1),
    };

/// Which of a mask's animatable numbers [value] names. The shape is not one of
/// them — it has no number — and asks for the still zero nobody reads, as does
/// a per-point feather naming a point the mask no longer has.
BridgeScalar maskScalarOf(BridgeMask mask, MaskValue value,
        [int vertex = -1]) =>
    switch (value) {
      MaskValue.opacity => mask.opacity,
      MaskValue.feather => mask.feather,
      MaskValue.expansion => mask.expansion,
      MaskValue.vertexFeather =>
        vertex >= 0 && vertex < mask.vertexFeather.length
            ? mask.vertexFeather[vertex]
            : const BridgeScalar.static_(0),
      MaskValue.path => const BridgeScalar.static_(0),
    };

/// [mask] with the one number [value] names replaced.
BridgeMask maskWithScalar(BridgeMask mask, MaskValue value, BridgeScalar to,
        [int vertex = -1]) =>
    BridgeMask(
      id: mask.id,
      name: mask.name,
      vertices: mask.vertices,
      closed: mask.closed,
      inverted: mask.inverted,
      opacity: value == MaskValue.opacity ? to : mask.opacity,
      mode: mask.mode,
      feather: value == MaskValue.feather ? to : mask.feather,
      vertexFeather: value == MaskValue.vertexFeather
          ? [
              for (var i = 0; i < mask.vertexFeather.length; i++)
                i == vertex ? to : mask.vertexFeather[i]
            ]
          : mask.vertexFeather,
      expansion: value == MaskValue.expansion ? to : mask.expansion,
      pathKeys: mask.pathKeys,
    );

/// A key's position on the comp's frame axis, computed Dart-side from its
/// exact time and the comp's rate so a paint never crosses the bridge for it.
///
/// Fractional on purpose: with the magnet off a key may sit *between* frames
/// (docs/07 §4.5), and it has to draw where it actually is.
double laneKeyFrame(BridgeKeyframe key, double fps) =>
    rationalSeconds(key.time) * fps;

/// The exact time of a (possibly fractional) frame position — what a lane key
/// drag commits.
///
/// Quantised to a thousandth of a frame and built from the comp's exact rate,
/// so the time stays rational (docs/14 §2): at 29.97 a whole frame is exactly
/// 1001/30000 s and half of one is exactly 1001/60000, never a rounded double.
BridgeRational timeOfSubframe(double frame, int fpsNum, int fpsDen) {
  final milliframes = (frame * 1000).round();
  return BridgeRational(
    num: milliframes * fpsDen,
    den: 1000 * (fpsNum == 0 ? 1 : fpsNum),
  );
}

/// Move a lane row's keyframe [index] to [time], as ONE op — one undo step
/// for the whole drag.
///
/// A transform row moves the key on *every* axis it covers: the axes key
/// together, so the row's one diamond stands for all of their keys. Refused
/// (returning false, changing nothing) when the move would land on or past a
/// neighbour — two keys cannot share a time, and the engine refuses a curve
/// whose times do not strictly ascend.
bool moveLaneKey({
  required BridgeLayerEntry entry,
  required LayerFoldRow row,
  required int index,
  required BridgeRational time,
}) {
  final target = rationalSeconds(time);

  List<BridgeKeyframe>? moved(List<BridgeKeyframe> keys) {
    if (index >= keys.length) return null;
    for (var i = 0; i < keys.length; i++) {
      if (i == index) continue;
      final other = rationalSeconds(keys[i].time);
      if (i < index ? other >= target : other <= target) return null;
    }
    return [
      for (var i = 0; i < keys.length; i++)
        if (i == index)
          BridgeKeyframe(
            time: time,
            value: keys[i].value,
            interpIn: keys[i].interpIn,
            interpOut: keys[i].interpOut,
          )
        else
          keys[i],
    ];
  }

  switch (row) {
    case FoldTransformRow(:final group, :final transform):
      final props = <BridgeTransformProp>[];
      final values = <BridgeScalar>[];
      for (final axis in group.axes) {
        final scalar = read(transform, axis.prop);
        if (scalar is! BridgeScalar_Keyframed) return false;
        final next = moved(scalar.field0);
        // Every axis or none: a half-applied move would leave the row's axes
        // keyed at different times, which is not a row any more.
        if (next == null) return false;
        props.add(axis.prop);
        values.add(BridgeScalar.keyframed(next));
      }
      if (props.isEmpty) return false;
      entry.layer.setTransforms(props: props, values: values);
      return true;

    case FoldEffectParamRow(:final info, :final param):
      final stack = entry.layer.getEffects();
      for (final instance in stack) {
        if (instance.id() != info.id) continue;
        final value = instance.getValue(id: param.id);
        if (value is! BridgeEffectValue_Float) return false;
        final scalar = value.field0;
        if (scalar is! BridgeScalar_Keyframed) return false;
        final next = moved(scalar.field0);
        if (next == null) return false;
        instance.setValue(
          id: param.id,
          value: BridgeEffectValue.float(BridgeScalar.keyframed(next)),
        );
        entry.layer.setEffects(effects: stack);
        return true;
      }
      return false;

    case FoldRetimeRow(:final scalar):
      if (scalar is! BridgeScalar_Keyframed) return false;
      final next = moved(scalar.field0);
      if (next == null) return false;
      entry.layer.setRetimeProperty(value: BridgeScalar.keyframed(next));
      return true;

    case FoldMaskValueRow(:final mask, :final value, :final vertex):
      if (value == MaskValue.path) {
        // A path key is a whole shape, so the engine moves it rather than the
        // frontend rebuilding a list of them (K-340).
        if (index >= mask.pathKeys.length) return false;
        return entry.layer.moveMaskPathKey(
          id: mask.id,
          from: mask.pathKeys[index].time,
          to: time,
        );
      }
      final scalar = maskScalarOf(mask, value, vertex);
      if (scalar is! BridgeScalar_Keyframed) return false;
      final next = moved(scalar.field0);
      if (next == null) return false;
      entry.layer.setMask(
        mask: maskWithScalar(mask, value, BridgeScalar.keyframed(next), vertex),
        at: null,
      );
      return true;

    case _:
      return false;
  }
}

/// A fold row's stable path — its id for selection, for the lane's keyframes,
/// and for working out what contains it.
///
/// Hierarchical on purpose, sharing its prefixes with [FoldGroupRow.path]:
/// selecting `<layer>/effects/<effect>/<param>` is what tells the outline to
/// highlight that effect's heading and that layer's row (docs/07 §4.3), and
/// `startsWith` is the whole of the "is this my ancestor" test.
String foldRowPath(String layerId, LayerFoldRow row) => switch (row) {
      FoldGroupRow(:final path) => path,
      FoldTransformRow(:final group) => transformGroupPath(layerId, group),
      FoldEffectParamRow(:final info, :final param) =>
        '${effectPath(layerId, info.id.toString())}/${param.id}',
      FoldVolumeRow() => '${audioPath(layerId)}/volume',
      FoldRetimeRow() => retimePath(layerId),
      FoldFlowRow(:final kind) => '${flowPath(layerId)}/${kind.name}',
      FoldWaveformRow() => waveformPath(layerId),
      FoldMaskRow(:final mask) => '${masksPath(layerId)}/${mask.id}',
      FoldMaskValueRow(:final mask, :final value, :final vertex) =>
        '${masksPath(layerId)}/${mask.id}/${value.name}'
            '${vertex < 0 ? '' : '/$vertex'}',
      FoldStrokeRow(:final stroke) => '${paintPath(layerId)}/${stroke.id}',
      FoldShapeRow(:final item) => '${contentsPath(layerId)}/${item.id}',
    };

/// Whether [path] sits under [ancestor] — a property under its group, a
/// parameter under its effect, anything under its layer.
bool isUnderPath(String ancestor, String path) =>
    ancestor.isNotEmpty && path.startsWith('$ancestor/');

/// The layer id a fold path belongs to — everything before the first `/` —
/// or null for a bare layer id, which sits under no layer but itself.
String? layerIdOfPath(String path) {
  final cut = path.indexOf('/');
  return cut > 0 ? path.substring(0, cut) : null;
}

/// The path of a layer's Retime row.
String retimePath(String layerId) => '$layerId/retime';

/// The path of a layer's Flow group in the open set.
String flowPath(String layerId) => '$layerId/flow';

/// The path of a layer's Transform group in the open set.
String transformPath(String layerId) => '$layerId/transform';

/// The path of one Transform row — Position, Scale, Rotation and the rest.
///
/// Named after the group's first axis rather than its label, because the label
/// is what the row *says* and the axis is what it *is*: renaming "Anchor point"
/// would otherwise quietly unbind the `A` key from the row it reveals.
String transformGroupPath(String layerId, TransformGroup group) =>
    '${transformPath(layerId)}/${group.axes.first.prop.name}';

/// The path of a layer's Effects group.
String effectsPath(String layerId) => '$layerId/effects';

/// The path of one effect within the Effects group.
String effectPath(String layerId, String effectId) =>
    '$layerId/effects/$effectId';

/// The effect instance a fold path names, or null when the path is not one
/// effect's heading (it is the Effects group itself, one parameter under an
/// effect, or something else entirely). Used by the render-time indicator to
/// put an effect's measured cost on its own row (docs/13 §7.1), and by the
/// Timeline's heading menu to know which rows can be copied from (K-275).
/// Whether a click is carrying one of the selection modifiers — Ctrl (Cmd) or
/// Shift. A heading twirls on a plain click and only *picks* on a modified one
/// (K-300): a Shift-click running over a stack of effects must not open every
/// heading it passes.
bool get isModifiedClick =>
    HardwareKeyboard.instance.isControlPressed ||
    HardwareKeyboard.instance.isMetaPressed ||
    HardwareKeyboard.instance.isShiftPressed;

String? effectIdOfPath(String path) {
  final parts = path.split('/');
  if (parts.length != 3 || parts[1] != 'effects') return null;
  return parts[2];
}

/// The path of a layer's Masks group.
String masksPath(String layerId) => '$layerId/masks';

/// The path of a shape layer's Contents group.
String contentsPath(String layerId) => '$layerId/contents';

/// The path of a layer's Paint group.
String paintPath(String layerId) => '$layerId/paint';

/// The path of a layer's Audio group.
String audioPath(String layerId) => '$layerId/audio';

/// The path of the Waveform twirl inside the Audio group.
String waveformPath(String layerId) => '$layerId/audio/waveform';

/// The rows to draw under an open layer, in order.
///
/// `hasAudio` is passed in rather than asked for here because answering it means
/// probing the file with FFmpeg, which is not work for a build — the Timeline
/// caches it per layer, exactly as the Project panel caches missing media.
/// `flowParams` and `volumeDb` are passed in for the same reason at a smaller
/// scale: neither is in the read model, so the panel reads them once per
/// document revision and the rows carry them (K-184).
List<LayerFoldRow> layerFoldRows({
  required BridgeLayerEntry entry,
  required Set<String> open,
  required bool hasAudio,
  BridgeFlowParams? flowParams,
  BridgeScalar? volumeDb,
}) {
  final id = entry.layer.internallayerId.toString();
  final info = entry.info;
  final rows = <LayerFoldRow>[];

  // A reveal key (`P`, `S`, `R`, `T`, `A`) leaves exactly one Transform row
  // open and the group itself shut — "show me Position" means Position, not
  // Position among five others. That is a *solo*, and it is read here rather
  // than passed in because the lanes build their rows from this same list and
  // must leave room for the same ones (docs/07 §4.3).
  final transformOpen = open.contains(transformPath(id));
  final groups = transformGroups(threeD: info.switches.threeD);
  final soloed = !transformOpen &&
      groups.any((g) => open.contains(transformGroupPath(id, g)));

  // Retime first, above everything (docs/07 §4.3): it decides *which* frame of
  // the source the rest of the fold-out then transforms. A layer that has not
  // been given one shows no row rather than a dead control — and it stands
  // down while a solo is in force, for the same reason the other four rows do.
  if (info.retime case final retime? when !soloed) {
    rows.add(FoldRetimeRow(retime, depth: 1));
  }

  // Flow above Transform and below Retime, which is the order the picture is
  // built in: the retime picks a moment, flow decides what is *shown* at a
  // moment between two frames, and the transform then places the result. Only
  // on a layer whose flow switch is on — an empty heading is a promise the row
  // cannot keep (K-088).
  if (info.flow && !soloed) {
    final flowOpen = open.contains(flowPath(id));
    rows.add(FoldGroupRow(
      path: flowPath(id),
      label: l10n.flowSection,
      open: flowOpen,
      depth: 1,
    ));
    if (flowOpen) {
      for (final kind in FlowRowKind.values) {
        rows.add(FoldFlowRow(
          kind,
          rate: kind == FlowRowKind.inputRate ? info.flowInputRate : null,
          params: flowParams,
          depth: 2,
        ));
      }
    }
  }

  rows.add(FoldGroupRow(
    path: transformPath(id),
    label: l10n.transformSection,
    open: transformOpen,
    depth: 1,
  ));
  for (final group in groups) {
    if (transformOpen || open.contains(transformGroupPath(id, group))) {
      rows.add(FoldTransformRow(group, info.transform, depth: 2));
    }
  }

  // Contents first of the three: a shape layer's art *is* its picture, so it
  // comes before the masks that gate that picture and the effects that process
  // it (K-237, docs/06 render order).
  if (info.shapeContents.isNotEmpty) {
    final contentsOpen = open.contains(contentsPath(id));
    rows.add(FoldGroupRow(
      path: contentsPath(id),
      label: l10n.foldContents,
      open: contentsOpen,
      depth: 1,
    ));
    if (contentsOpen) {
      for (final item in info.shapeContents) {
        rows.add(FoldShapeRow(item, depth: 2));
      }
    }
  }

  // Masks, above Effects because that is the order they are applied in: a mask
  // gates the layer's alpha *before* its effects run (docs/06 render order), so
  // the fold-out reads top to bottom the way the picture is built. Like
  // Effects, the heading appears only once there is something under it — an
  // empty heading is a promise the row cannot keep.
  if (info.masks.isNotEmpty) {
    final masksOpen = open.contains(masksPath(id));
    rows.add(FoldGroupRow(
      path: masksPath(id),
      label: l10n.foldMasks,
      open: masksOpen,
      depth: 1,
    ));
    if (masksOpen) {
      for (final mask in info.masks) {
        rows.add(FoldMaskRow(mask, depth: 2));
        // Its values sit under it, the way an effect's parameters sit under
        // the effect — shape first, because it is what the mask *is*, then the
        // numbers in the order they apply.
        for (final value in MaskValue.values) {
          if (value == MaskValue.vertexFeather) continue;
          rows.add(FoldMaskValueRow(mask, value, depth: 3));
        }
        // The per-point widths, under the one width they vary from (K-445),
        // and only once the mask actually carries them: a mask feathered the
        // ordinary way shows the four rows it always did.
        for (var i = 0; i < mask.vertexFeather.length; i++) {
          rows.add(FoldMaskValueRow(mask, MaskValue.vertexFeather,
              depth: 3, vertex: i));
        }
      }
    }
  }

  // Paint, between Masks and Effects, because that is where it happens: strokes
  // are stamped into the layer's own pixels, which the masks then gate and the
  // effects then process (K-227, docs/06 render order).
  if (info.paint.isNotEmpty) {
    final paintOpen = open.contains(paintPath(id));
    rows.add(FoldGroupRow(
      path: paintPath(id),
      label: l10n.foldPaint,
      open: paintOpen,
      depth: 1,
    ));
    if (paintOpen) {
      for (final stroke in info.paint) {
        rows.add(FoldStrokeRow(stroke, depth: 2));
      }
    }
  }

  // Effects appear only once there are some: an empty heading is a promise the
  // row cannot keep.
  if (info.effects.isNotEmpty) {
    final effectsOpen = open.contains(effectsPath(id));
    rows.add(FoldGroupRow(
      path: effectsPath(id),
      label: l10n.workspaceEffects,
      open: effectsOpen,
      depth: 1,
    ));
    if (effectsOpen) {
      for (final fx in info.effects) {
        final path = effectPath(id, fx.id.toString());
        final effectOpen = open.contains(path);
        rows.add(FoldGroupRow(
          path: path,
          // The user's own name where one is set (K-321), so the fold-out
          // and the Effect controls read the same.
          label: fx.customName ?? effectLabelOf(fx.name),
          open: effectOpen,
          depth: 2,
        ));
        if (effectOpen) {
          final values = {for (final v in fx.values) v.id: v.value};
          for (final param in cachedListParameters(fx.name)) {
            rows.add(FoldEffectParamRow(fx, param, values[param.id], depth: 3));
          }
        }
      }
    }
  }

  if (hasAudio) {
    final audioOpen = open.contains(audioPath(id));
    rows.add(FoldGroupRow(
      path: audioPath(id),
      label: l10n.workspaceAudio,
      open: audioOpen,
      depth: 1,
    ));
    if (audioOpen) {
      rows.add(FoldVolumeRow(scalar: volumeDb, depth: 2));
      // The waveform behind its own twirl (K-172), so a busy comp only pays
      // for the lanes actually being looked at.
      final waveOpen = open.contains(waveformPath(id));
      rows.add(FoldGroupRow(
        path: waveformPath(id),
        label: l10n.foldWaveform,
        open: waveOpen,
        depth: 2,
      ));
      if (waveOpen) rows.add(const FoldWaveformRow(depth: 3));
    }
  }

  return rows;
}
