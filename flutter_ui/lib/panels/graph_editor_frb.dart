// The graph editor: the selected properties' animation as curves you shape,
// After Effects style (docs/07 §5).
//
// One full-height pane over the Timeline's own time axis — same ruler, same
// zoom, same horizontal scroll — with every selected property drawn as its own
// coloured curve (a two-axis property like Position contributes one curve per
// axis). Keyframes draw with interpolation-coded glyphs; selected keys show
// their bezier tangent handles, draggable per side, with `Alt` breaking and
// re-joining the two sides. The **value** lens plots value against time; the
// **speed** lens plots dv/dt, where each key is an in point and an out point
// that move independently, each with a single influence handle — the AE speed
// graph.
//
// **Zero bridge calls to draw** (K-184): the curves are evaluated by the Dart
// port of the engine's own cubic (graph_maths.dart, pinned together by
// docs/impl/keyframe-eval.md), and every scalar rides in on the comp read
// model. The bridge is only crossed when a gesture commits — one write per
// channel, batched per layer, so a drag stays one undo step per property.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart';
import 'package:lumit_flutter/state/comp_model.dart';
import 'package:lumit_flutter/state/preview_throttle.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/os_keys.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/drag_escape.dart';
import '../widgets/marquee.dart';
import 'easing_curve.dart';
import 'effect_param_row_frb.dart';
import 'graph_maths.dart';
import 'key_block.dart';
import 'layer_fold_frb.dart';
import 'timeline_extras_frb.dart';
import 'timeline_snap.dart';
import 'transform_rows_frb.dart';

/// A value drag in flight **in the layer area**, published for the graph pane to
/// draw (K-333).
///
/// The row stages its value in Dart and commits once on release (K-192), so the
/// read model — and therefore the curve — still holds the old one until the
/// pointer comes up. The pane cannot ask for it, because it is not in the
/// document; the row publishes it here instead, exactly as a bar drag publishes
/// its travel for the waveform lane (`BarDragPreview`, K-172). Null between
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

/// How wide a keyframe's grab target is, and a tangent handle's. Both are
/// bigger than the glyph they carry: these are small marks that must be
/// caught first time, and a miss on a handle is worse than a miss on empty
/// pane — it drops the key's selection and takes the handles away with it.
const double _keyGrab = 16;
const double _handleGrab = 18;

/// How wide a key's glyph is drawn, and how wide a **selected** one is: one
/// size step larger, the drawing's 7 in a 6 world (§6.2). The target around it
/// does not change with the selection — a mark that grew its own catch would
/// move the pointer's meaning under a still hand.
const double _keyGlyph = 10;
const double _selectedKeyGlyph = 12;

/// How thick a **transform box** edge's grab is (§6.2): a strip this wide,
/// centred on the edge and running its whole length.
///
/// Narrower than a key's own target ([_keyGrab]) on purpose. The box's edges
/// run through the very keys at its extremes, and the keys are drawn *over*
/// the box, so a key at a corner keeps every gesture it has and the edge is
/// grabbable everywhere else along its length (P5).
const double _boxGrab = 10;

/// How wide the **value gutter** is: the drawing's 34px strip down the right
/// of the graph, on a translucent ground, where every value label lives
/// (§12A.2 — "value labels live in a fixed right-hand gutter, never on the
/// curve").
const double graphGutterWidth = 34;

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

  /// True for the layer's Retime channel (K-197), which is neither a transform
  /// property nor an effect parameter but reads and writes like both.
  final bool retime;

  /// Set for one of a mask's values (K-340): the mask it belongs to, and which
  /// of its values this is.
  final BridgeMask? mask;
  final MaskValue? maskValue;

  /// True for a mask's **shape** (K-344). A path has no value to plot, so what
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
  });

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
/// read model (K-184's deliberate exceptions) and is skipped — docs/TODO.md.
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

    // Retime (K-197): one channel, source time in seconds. An ordinary curve
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
      for (final group in transformGroups(threeD: entry.info.switches.threeD)) {
        if (group.axes.first.prop.name != lead) continue;
        for (final axis in group.axes) {
          out.add(GraphChannel(
            path: path,
            id: '$path@${axis.prop.name}',
            label: group.axes.length == 1
                ? '${entry.info.name} · ${group.label}'
                : '${entry.info.name} · ${group.label} ${_axisLetter(group.axes.indexOf(axis))}',
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

    if (path.startsWith('${effectsPath(layerId)}/')) {
      final rest = path.substring(effectsPath(layerId).length + 1);
      final slash = rest.indexOf('/');
      if (slash <= 0) continue;
      final effectId = rest.substring(0, slash);
      final paramId = rest.substring(slash + 1);
      for (final fx in entry.info.effects) {
        if (fx.id.toString() != effectId) continue;
        for (final param in cachedListParameters(fx.name)) {
          if (param.id != paramId) continue;
          // A Slider is a Float inside a closed range (K-414): the kind is the
          // control, not the storage, so it keeps every float affordance —
          // docs/08 §1.2 names the graph editor among them.
          if (param.kind is! BridgeParamKind_Float &&
              param.kind is! BridgeParamKind_Slider) {
            continue;
          }
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

    // A mask's numbers (K-340). Its shape is deliberately absent: a path has no
    // value axis to draw against, so it keeps its lane diamonds and no curve.
    if (path.startsWith('${masksPath(layerId)}/')) {
      final rest = path.substring(masksPath(layerId).length + 1);
      final slash = rest.indexOf('/');
      if (slash <= 0) continue;
      final maskId = rest.substring(0, slash);
      final valueName = rest.substring(slash + 1);
      for (final mask in entry.info.masks) {
        if (mask.id.toString() != maskId) continue;
        final value =
            MaskValue.values.where((v) => v.name == valueName).firstOrNull;
        if (value == null) break;
        // The shape's channel carries its keys as the counted-up interpolation
        // parameter (K-344); a still shape has none and draws nothing.
        if (value == MaskValue.path && mask.pathKeys.isEmpty) break;
        out.add(GraphChannel(
          path: path,
          id: path,
          label: '${entry.info.name} · ${mask.name} · ${maskValueLabel(value)}',
          colourIndex: out.length,
          scalar: value == MaskValue.path
              ? BridgeScalar.keyframed(mask.pathKeys)
              : maskScalarOf(mask, value),
          entry: entry,
          mask: mask,
          maskValue: value,
        ));
        break;
      }
    }
  }
  return out;
}

String _axisLetter(int i) => switch (i) { 0 => 'x', 1 => 'y', _ => 'z' };

/// Commit new scalars for a set of channels in the fewest ops: one
/// `setTransforms` batch per layer for its transform channels (one undo step),
/// one staged `setEffects` per layer for its effect channels.
void commitChannelEdits(Map<GraphChannel, BridgeScalar> edits) {
  // Transform channels, grouped by layer.
  final transforms = <String,
      (LayerReference, List<BridgeTransformProp>, List<BridgeScalar>)>{};
  final effects =
      <String, (LayerReference, Map<String, Map<String, BridgeScalar>>)>{};
  edits.forEach((channel, next) {
    final layerId = channel.entry.layer.internallayerId.toString();
    if (channel.prop != null) {
      final slot = transforms[layerId] ??= (channel.entry.layer, [], []);
      slot.$2.add(channel.prop!);
      slot.$3.add(next);
    } else if (channel.retime) {
      // One Retime per layer, so there is nothing to batch: the write is
      // already one op and therefore one undo step.
      channel.entry.layer.setRetimeProperty(value: next);
    } else if (channel.effect != null && channel.param != null) {
      final slot = effects[layerId] ??= (channel.entry.layer, {});
      (slot.$2[channel.effect!.id.toString()] ??= {})[channel.param!.id] = next;
    } else if (channel.isMaskPath && channel.mask != null) {
      // A shape key holds a path, not a number, so only its time and its eases
      // can be written — which is exactly what a graph edit changes (K-344).
      channel.entry.layer.setMaskPathKeys(
        id: channel.mask!.id,
        keys: keysOf(next),
      );
    } else if (channel.mask case final mask?) {
      // A mask edit takes the whole mask, so there is nothing to batch per
      // property; two curves on one mask are two writes and two undo steps,
      // which is what `SetLayerMasks` costs until it grows a per-key op.
      channel.entry.layer.setMask(
        mask: maskWithScalar(mask, channel.maskValue!, next),
        at: null,
      );
    }
  });
  for (final (layer, props, values) in transforms.values) {
    layer.setTransforms(props: props, values: values);
  }
  for (final (layer, byEffect) in effects.values) {
    final staged = layer.getEffects();
    for (final instance in staged) {
      final wanted = byEffect[instance.id().toString()];
      if (wanted == null) continue;
      wanted.forEach((paramId, scalar) {
        instance.setValue(id: paramId, value: BridgeEffectValue.float(scalar));
      });
    }
    layer.setEffects(effects: staged);
  }
}

/// Show the edits a gesture is *about* to make, without making them: the same
/// scalars [commitChannelEdits] writes on release, rendered through the
/// engine's patched clone.
///
/// **Why this exists.** A drag is one op on release (K-192), so between the
/// first move and the mouse-up the document still holds the old curve and the
/// Viewer still shows it. On a transform or an effect that is merely awkward;
/// on a **Retime** it is the whole edit — you are choosing which frame of the
/// source to land on, by eye, against a picture that will not move until you
/// let go. Every other live drag in the editor already previews (transform
/// rows, effect rows, paint, masks, clip envelopes); the graph is where curves
/// are actually shaped, and it was the one place that did not.
///
/// **One layer, one kind, per gesture.** A preview request patches a single
/// layer's single state (see `RenderCompRequestWithPreview`), so this previews
/// the grabbed channel's layer and the channels that patch the same way; a
/// selection spanning several layers or a transform *and* an effect at once
/// shows the rest on release, as it always did. That is what a gesture is in
/// practice: one property of one layer.
void previewChannelEdits({
  required CompositionReference comp,
  required Map<GraphChannel, BridgeScalar> edits,
  required int frame,
  required double scale,
}) {
  if (edits.isEmpty) return;
  final lead = edits.keys.first;
  final layer = lead.entry.layer;
  final layerId = layer.internallayerId.toString();
  bool sameLayer(GraphChannel c) =>
      c.entry.layer.internallayerId.toString() == layerId;
  final bigFrame = BigInt.from(frame);

  if (lead.retime) {
    comp.renderFrameWithRetime(
      frame: bigFrame,
      scale: scale,
      layer: layer,
      retime: edits[lead]!,
    );
    return;
  }

  if (lead.prop != null) {
    var transform = layer.getTransform();
    edits.forEach((channel, next) {
      if (!sameLayer(channel) || channel.prop == null) return;
      transform = writeScalar(transform, channel.prop!, next);
    });
    comp.renderFrameWithTransformPreview(
      frame: bigFrame,
      scale: scale,
      layer: layer,
      transform: transform,
    );
    return;
  }

  if (lead.effect == null || lead.param == null) return;
  final staged = layer.getEffects();
  for (final instance in staged) {
    edits.forEach((channel, next) {
      if (!sameLayer(channel) ||
          channel.effect == null ||
          channel.param == null) {
        return;
      }
      if (channel.effect!.id.toString() != instance.id().toString()) return;
      instance.setValue(
          id: channel.param!.id, value: BridgeEffectValue.float(next));
    });
  }
  comp.renderFrameWithPreview(
    frame: bigFrame,
    scale: scale,
    layer: layer,
    effects: staged,
  );
}

/// [keys] with a key of [value] at [frame] — replacing the one already there,
/// because two keys at one time is not a curve the engine will take (K-301).
List<BridgeKeyframe> _withKeyAt(
  List<BridgeKeyframe> keys,
  double frame,
  double value,
  double fps,
  int fpsNum,
  int fpsDen,
) {
  final merged = <double, BridgeKeyframe>{
    for (final k in keys) _keyFrame(k, fps): k,
  };
  merged[frame] = BridgeKeyframe(
    time: timeOfSubframe(frame, fpsNum, fpsDen),
    value: value,
    interpIn: const BridgeSideInterp.linear(),
    interpOut: const BridgeSideInterp.linear(),
  );
  final frames = merged.keys.toList()..sort();
  return [for (final f in frames) merged[f]!];
}

/// A key's position on the frame axis, fractional (a key may sit between
/// frames with the magnet off).
double _keyFrame(BridgeKeyframe key, double fps) =>
    rationalSeconds(key.time) * fps;

/// A number as this pane's readouts write it: whole numbers plain, everything
/// else to two places — the same hand the dope sheet's own numbers are set in.
String graphNumberText(double v) =>
    v == v.roundToDouble() ? v.round().toString() : v.toStringAsFixed(2);

/// One side of `keys[index]` as a bezier reaching [percent] of its span, at
/// the speed the side already reads.
///
/// The tangent handle's own commit, reached by typing instead of by dragging
/// — the graph's Key readout row writes influence with it, and so does the
/// numeric-entry popover (§3.3, §6.2). A side that was linear becomes a
/// bezier that looks exactly as it did, which is the only way to give a
/// straight side a reach at all.
BridgeSideInterp sideWithInfluence(
        List<BridgeKeyframe> keys, int index, bool isOut, double percent) =>
    BridgeSideInterp.bezier(BridgeBezierSide(
      speed: sideSpeedAtKey(keys, index, isOut: isOut),
      influence: (percent / 100).clamp(1e-3, 1.0).toDouble(),
    ));

/// Set one or both sides of every selected key to [side] — the F9 family and
/// the bottom bar's Linear / Bezier / Hold buttons. `inSide`/`outSide` pick
/// which sides change (ease-in touches only the in side, and so on).
void applyInterpToSelection({
  required List<GraphChannel> channels,
  required Set<String> selectedKeys,
  required BridgeSideInterp side,
  bool inSide = true,
  bool outSide = true,
}) {
  final edits = <GraphChannel, BridgeScalar>{};
  for (final channel in channels) {
    final keys = channel.keys;
    var touched = false;
    final next = <BridgeKeyframe>[];
    for (var i = 0; i < keys.length; i++) {
      if (selectedKeys.contains('${channel.id}#$i')) {
        touched = true;
        next.add(BridgeKeyframe(
          time: keys[i].time,
          value: keys[i].value,
          interpIn: inSide ? side : keys[i].interpIn,
          interpOut: outSide ? side : keys[i].interpOut,
        ));
      } else {
        next.add(keys[i]);
      }
    }
    if (touched) edits[channel] = BridgeScalar.keyframed(next);
  }
  if (edits.isNotEmpty) commitChannelEdits(edits);
}

/// Put both sides of every selected key into [mode] — the bottom bar's
/// Tangents Auto / Clamp / Free (docs/impl/timeline-interaction.md §6.3).
///
/// The mode is stored **per side**, and this sets both, because the strip's
/// unit is the key: a side is aimed one at a time by dragging its handle,
/// which is also what takes it back to Free. The ease each side is carrying
/// travels inside the automatic side ([withTangentMode]), so a round trip out
/// to Auto and back hands the custom ease over unchanged.
void applyTangentModeToSelection({
  required List<GraphChannel> channels,
  required Set<String> selectedKeys,
  required TangentMode mode,
}) {
  final edits = <GraphChannel, BridgeScalar>{};
  for (final channel in channels) {
    final keys = channel.keys;
    var touched = false;
    final next = <BridgeKeyframe>[];
    for (var i = 0; i < keys.length; i++) {
      if (!selectedKeys.contains('${channel.id}#$i')) {
        next.add(keys[i]);
        continue;
      }
      touched = true;
      next.add(BridgeKeyframe(
        time: keys[i].time,
        value: keys[i].value,
        interpIn: withTangentMode(keys[i].interpIn, mode),
        interpOut: withTangentMode(keys[i].interpOut, mode),
      ));
    }
    if (touched) edits[channel] = BridgeScalar.keyframed(next);
  }
  if (edits.isNotEmpty) commitChannelEdits(edits);
}

/// Plant a key at [frame] on every channel here, each taking the value its own
/// curve already reads there — so the picture does not move. Adding a key is a
/// place to grab, not an edit (docs/07 §4.3, K-500 §2.1's lane gesture).
///
/// A channel with nothing keyed is left alone: the gesture is *"plant a key on
/// this keyed row"*, and turning a static property into an animated one is the
/// stopwatch's job, not a Ctrl-click's. A channel that already has a key on
/// that frame is left alone too, because two keys at one time is not a curve
/// the engine will take (K-301). A mask's **shape** channel is skipped: a path
/// key holds a whole path, which the mask's own control plants.
///
/// Returns whether anything was written. One call, so a two-axis row's key
/// lands on both axes in one op — one undo step, as a lane diamond is one key.
bool plantKeyOnChannels({
  required List<GraphChannel> channels,
  required double frame,
  required double fps,
  required int fpsNum,
  required int fpsDen,
}) {
  final seconds = frame / (fps <= 0 ? 1 : fps);
  final edits = <GraphChannel, BridgeScalar>{};
  for (final channel in channels) {
    if (channel.isMaskPath) continue;
    final keys = channel.keys;
    if (keys.isEmpty) continue;
    if (keys.any((k) => _keyFrame(k, fps).round() == frame.round())) continue;
    edits[channel] = BridgeScalar.keyframed(_withKeyAt(
        keys, frame, evaluateKeys(keys, seconds), fps, fpsNum, fpsDen));
  }
  if (edits.isEmpty) return false;
  commitChannelEdits(edits);
  return true;
}

/// Remove every key in [selectedKeys] from [channels] — the graph's Delete and
/// the lane key menu's *Delete key* are the same removal (K-500 §2.1).
///
/// The last key of a curve leaves a static value holding what it held: a
/// property that has lost its animation still has to read something.
///
/// Returns whether anything was written.
bool deleteKeysFromChannels({
  required List<GraphChannel> channels,
  required Set<String> selectedKeys,
}) {
  final edits = <GraphChannel, BridgeScalar>{};
  for (final channel in channels) {
    final keys = channel.keys;
    final rest = <BridgeKeyframe>[];
    var removed = false;
    double? lastRemoved;
    for (var i = 0; i < keys.length; i++) {
      if (selectedKeys.contains('${channel.id}#$i')) {
        removed = true;
        lastRemoved = keys[i].value;
      } else {
        rest.add(keys[i]);
      }
    }
    if (!removed) continue;
    edits[channel] = rest.isEmpty
        ? BridgeScalar.static_(lastRemoved ?? 0)
        : BridgeScalar.keyframed(rest);
  }
  if (edits.isEmpty) return false;
  commitChannelEdits(edits);
  return true;
}

/// Every selected key's frame, across every channel — what the block tools
/// measure before they move anything (K-458).
///
/// The span a Reverse mirrors within is the *selection's*, not each channel's:
/// three rows selected together are one block, and mirroring each row inside
/// its own extent would slide the rows apart rather than turn the block round.
List<double> selectedKeyFrames({
  required List<GraphChannel> channels,
  required Set<String> selectedKeys,
  required double fps,
}) {
  final out = <double>[];
  for (final channel in channels) {
    final keys = channel.keys;
    for (var i = 0; i < keys.length; i++) {
      if (selectedKeys.contains('${channel.id}#$i')) {
        out.add(_keyFrame(keys[i], fps));
      }
    }
  }
  return out;
}

/// Give every selected key a new time, [frameOf] deciding where each one goes
/// from where it is and which channel it is on (K-458).
///
/// The shared body of Reverse and of the Ease popover's Stagger. Each channel's
/// list is rebuilt whole and **re-sorted**, because a move can change the order
/// keys come in — that is the point of a reverse — and the engine refuses a
/// curve whose times do not strictly ascend. A channel whose result would put
/// two keys on the same time is left alone rather than written wrong, and a
/// channel is written only if something on it actually moved.
///
/// [swapSides] mirrors each moved key's in and out interpolation as well as its
/// time. Reverse wants it: a key that eased slowly *out* of itself is, played
/// backwards, a key that eases slowly *into* itself, and a reverse that left the
/// sides alone would turn the times round while leaving the motion's shape
/// pointing the way it was. Stagger does not: a stagger is a shift, and a shift
/// changes nothing about how the movement runs.
void moveSelectedKeys({
  required List<GraphChannel> channels,
  required Set<String> selectedKeys,
  required double fps,
  required int fpsNum,
  required int fpsDen,
  required double Function(GraphChannel channel, double frame) frameOf,
  bool swapSides = false,
}) {
  final edits = <GraphChannel, BridgeScalar>{};
  for (final channel in channels) {
    final keys = channel.keys;
    var touched = false;
    final next = <BridgeKeyframe>[];
    for (var i = 0; i < keys.length; i++) {
      if (!selectedKeys.contains('${channel.id}#$i')) {
        next.add(keys[i]);
        continue;
      }
      final was = _keyFrame(keys[i], fps);
      final now = frameOf(channel, was);
      if (now == was && !swapSides) {
        next.add(keys[i]);
        continue;
      }
      touched = true;
      next.add(BridgeKeyframe(
        time: timeOfSubframe(now, fpsNum, fpsDen),
        value: keys[i].value,
        interpIn: swapSides ? keys[i].interpOut : keys[i].interpIn,
        interpOut: swapSides ? keys[i].interpIn : keys[i].interpOut,
      ));
    }
    if (!touched) continue;
    next.sort(
        (a, b) => rationalSeconds(a.time).compareTo(rationalSeconds(b.time)));
    // Two keys landing on one time is a curve the engine must refuse, so the
    // channel keeps what it had rather than being written into a state that
    // cannot be saved.
    var clash = false;
    for (var i = 1; i < next.length; i++) {
      if (rationalSeconds(next[i].time) <= rationalSeconds(next[i - 1].time)) {
        clash = true;
        break;
      }
    }
    if (!clash) edits[channel] = BridgeScalar.keyframed(next);
  }
  if (edits.isNotEmpty) commitChannelEdits(edits);
}

/// Reverse the selection in time: the block plays backwards where it stands
/// (K-458, the Keys mode bottom bar).
///
/// Each key's new time is its old one reflected through the middle of the
/// block, so the earliest becomes the latest and the whole run stays exactly
/// where it was on the Timeline. **The value travels with its key** — this
/// re-times keys, it does not shuffle values under fixed times — and each key's
/// two eases swap, because the side that was leaving is now the side arriving.
///
/// Wrap the call in [asOneUndoStep]: a selection spanning two layers is two
/// writes, and Reverse is one press.
void reverseSelection({
  required List<GraphChannel> channels,
  required Set<String> selectedKeys,
  required double fps,
  required int fpsNum,
  required int fpsDen,
}) {
  final frames = selectedKeyFrames(
      channels: channels, selectedKeys: selectedKeys, fps: fps);
  if (frames.length < 2) return;
  var lo = frames.first;
  var hi = frames.first;
  for (final f in frames) {
    if (f < lo) lo = f;
    if (f > hi) hi = f;
  }
  final sum = lo + hi;
  moveSelectedKeys(
    channels: channels,
    selectedKeys: selectedKeys,
    fps: fps,
    fpsNum: fpsNum,
    fpsDen: fpsDen,
    swapSides: true,
    frameOf: (_, frame) => sum - frame,
  );
}

/// Fan the selection out in time: each row's keys pushed [step] frames further
/// than the row before it, so a run of properties arrives one after another
/// rather than together (K-458, the Ease popover's Stagger).
///
/// [order] is the list of property paths, top to bottom as the outline lists
/// them — a channel's rank is where its own path sits in it, so the two axes of
/// one Position stagger *together*, which is what makes them still one row.
void staggerSelection({
  required List<GraphChannel> channels,
  required Set<String> selectedKeys,
  required List<String> order,
  required double step,
  required StaggerOrder direction,
  required double fps,
  required int fpsNum,
  required int fpsDen,
}) {
  if (step == 0 || order.length < 2) return;
  moveSelectedKeys(
    channels: channels,
    selectedKeys: selectedKeys,
    fps: fps,
    fpsNum: fpsNum,
    fpsDen: fpsDen,
    frameOf: (channel, frame) {
      final rank = order.indexOf(channel.path);
      if (rank < 0) return frame;
      return staggeredFrame(frame,
          rank: rank, rows: order.length, step: step, order: direction);
    },
  );
}

/// Stamp one normalised [curve] onto every **span** the selection covers — the
/// easing editor's Apply.
///
/// A shape describes the travel *between* two keys, so the unit of work here is
/// a span rather than a key: a span takes the curve when both of its ends are
/// selected. Selecting a run of keys therefore eases the whole run, and
/// selecting a lone key does nothing, having named no travel.
///
/// Each span converts the shape separately, against its own chord slope
/// ([EasingCurve.sidesFor]) — the same drawn ease over a 400-pixel move and a
/// 40-pixel one stores different speeds, and must, or only one of them would
/// look like the shape that was drawn. A key in the middle of a run takes its
/// in-side from the span behind it and its out-side from the span ahead.
void applyEasingToSelection({
  required List<GraphChannel> channels,
  required Set<String> selectedKeys,
  required EasingCurve curve,
}) {
  final edits = <GraphChannel, BridgeScalar>{};
  for (final channel in channels) {
    final keys = channel.keys;
    final next = [...keys];
    var touched = false;
    for (var i = 0; i + 1 < keys.length; i++) {
      if (!selectedKeys.contains('${channel.id}#$i') ||
          !selectedKeys.contains('${channel.id}#${i + 1}')) {
        continue;
      }
      final t1 = rationalSeconds(keys[i].time);
      final t2 = rationalSeconds(keys[i + 1].time);
      final dt = t2 - t1;
      // Two keys on the same frame have no travel to shape, and dividing by
      // that gap is how a curve becomes infinities. Leave the pair alone.
      if (dt <= 0) continue;
      final sides = curve.sidesFor((keys[i + 1].value - keys[i].value) / dt);
      next[i] = BridgeKeyframe(
        time: next[i].time,
        value: next[i].value,
        interpIn: next[i].interpIn,
        interpOut: sides.out,
      );
      next[i + 1] = BridgeKeyframe(
        time: next[i + 1].time,
        value: next[i + 1].value,
        interpIn: sides.inTo,
        interpOut: next[i + 1].interpOut,
      );
      touched = true;
    }
    if (touched) edits[channel] = BridgeScalar.keyframed(next);
  }
  if (edits.isNotEmpty) commitChannelEdits(edits);
}

// ---------------------------------------------------------------------------
// The keyframe clipboard (docs/07 §5.3, K-196).
// ---------------------------------------------------------------------------

/// One copied channel: where it came from (for the AE text's property line)
/// and its keys with full easing fidelity.
///
/// A row with **no keyframes at all** copies too (K-301): it has a value, and
/// a value is the thing being copied. Such a clip carries [staticValue] and no
/// keys, and pastes as a value rather than as a curve.
class GraphClipChannel {
  final GraphChannel source;
  final List<BridgeKeyframe> keys;
  final double? staticValue;
  const GraphClipChannel(this.source, this.keys, {this.staticValue});
}

/// The in-app keyframe clipboard: full fidelity, and the one a paste prefers.
/// Module-level so it survives panel rebuilds and pastes across layers.
List<GraphClipChannel> graphKeyClipboard = const [];

/// The running build's version, for the clipboard header — taken from the
/// engine's own boot log (`lumit-bridge 0.1.0`), so there is one source of
/// truth for it. Asked once per session; a copy is a gesture, not a paint.
String? _version;
String lumitVersion() {
  final held = _version;
  if (held != null) return held;
  try {
    final first = bootLog().firstOrNull ?? '';
    final parts = first.trim().split(RegExp(r'\s+'));
    return _version = parts.length > 1 ? parts.last : 'unknown';
  } catch (_) {
    return _version = 'unknown';
  }
}

/// Copy the selected keys. The in-app clipboard keeps everything; the system
/// clipboard gets the tab-separated keyframe table (docs/07 §5.3) — values
/// *and* easing — so a copied ramp can be scripted, inspected, or carried into
/// another tool.
///
/// **Returns whether anything was taken** (K-529). It used to return nothing
/// at all, so a caller could not tell a copy that captured a curve from one
/// that captured nothing — and the Timeline's caller reported success either
/// way, which swallowed `Ctrl+C` and left the previous copy on the clipboard
/// for the next Paste to put down. A copy that took nothing says so, and the
/// chord falls through to whatever else the selection offers.
bool copySelectedKeys({
  required CompositionReference comp,
  required List<GraphChannel> channels,
  required Set<String> selectedKeys,
  required double fps,
}) {
  final copied = <GraphClipChannel>[];
  for (final channel in channels) {
    final keys = channel.keys;
    final hit = [
      for (var i = 0; i < keys.length; i++)
        if (selectedKeys.contains('${channel.id}#$i')) keys[i],
    ];
    if (hit.isNotEmpty) copied.add(GraphClipChannel(channel, hit));
  }
  if (copied.isEmpty) return false;
  graphKeyClipboard = copied;

  // The text mirror. The axes of one property fold into a single group with an
  // X/Y[/Z] column each, over the union of their key frames.
  final settings = comp.getSettings();
  final groups = <LumitClipGroup>[];
  final done = <GraphClipChannel>{};
  for (final clip in copied) {
    if (done.contains(clip)) continue;
    final prop = clip.source.prop;
    if (prop != null) {
      final siblings = [
        for (final other in copied)
          if (other.source.path == clip.source.path) other,
      ];
      done.addAll(siblings);
      groups.add(_transformClipGroup(clip.source, siblings, fps));
    } else {
      done.add(clip);
      groups.add(LumitClipGroup(
        property: [
          l10n.workspaceEffects,
          effectLabelOf(clip.source.effect?.name ?? ''),
          clip.source.param?.label ?? '',
        ],
        columns: [l10n.clipboardValueColumn],
        rows: [
          for (final k in clip.keys)
            LumitClipRow(
              frame: _keyFrame(k, fps),
              values: [k.value],
              eases: [(k.interpIn, k.interpOut)],
            ),
        ],
      ));
    }
  }
  Clipboard.setData(ClipboardData(
    text: lumitClipboardText(
      version: lumitVersion(),
      fps: fps,
      width: settings.width.toInt(),
      height: settings.height.toInt(),
      groups: groups,
    ),
  ));
  return true;
}

/// Copy **whole rows** — every key of an animated channel, or the plain value
/// of one that has none (K-301). What `Ctrl+C` does with property rows selected
/// and no individual keyframes picked.
///
/// A row that is not animated still has a value, and that value is what a user
/// selecting the row and pressing Copy is asking for; before this the chord
/// found no keys, gave up, and quietly copied the whole layer instead.
///
/// Returns whether anything was copied.
bool copyChannels({
  required CompositionReference comp,
  required List<GraphChannel> channels,
  required double fps,
}) {
  if (channels.isEmpty) return false;
  graphKeyClipboard = [
    for (final channel in channels)
      if (channel.scalar case BridgeScalar_Static(:final field0))
        GraphClipChannel(channel, const [], staticValue: field0)
      else
        GraphClipChannel(channel, channel.keys),
  ];

  // The system clipboard gets the keyframe table for whatever is animated, and
  // — when nothing is — the plain numbers, tab-joined, which is what a value
  // copied out of Lumit is useful as anywhere else (it is also exactly what a
  // value field's own right-click Copy writes).
  final animated = [
    for (final clip in graphKeyClipboard)
      if (clip.keys.isNotEmpty) clip,
  ];
  if (animated.isEmpty) {
    Clipboard.setData(ClipboardData(
      text: graphKeyClipboard.map((c) => '${c.staticValue}').join('\t'),
    ));
    return true;
  }
  copySelectedKeys(
    comp: comp,
    channels: [for (final clip in animated) clip.source],
    selectedKeys: {
      for (final clip in animated)
        for (var i = 0; i < clip.keys.length; i++) '${clip.source.id}#$i',
    },
    fps: fps,
  );
  // `copySelectedKeys` has just replaced the in-app clipboard with the animated
  // rows alone; put the full set — static rows included — back.
  graphKeyClipboard = [
    for (final channel in channels)
      if (channel.scalar case BridgeScalar_Static(:final field0))
        GraphClipChannel(channel, const [], staticValue: field0)
      else
        GraphClipChannel(channel, channel.keys),
  ];
  return true;
}

/// The property line and columns for a transform property's copied axes.
LumitClipGroup _transformClipGroup(
    GraphChannel lead, List<GraphClipChannel> axes, double fps) {
  final (name, unit) = switch (lead.prop!) {
    BridgeTransformProp.anchorX || BridgeTransformProp.anchorY => (
        l10n.transformAnchorPoint,
        l10n.unitPixels
      ),
    BridgeTransformProp.positionX ||
    BridgeTransformProp.positionY ||
    BridgeTransformProp.positionZ =>
      (l10n.transformPosition, l10n.unitPixels),
    BridgeTransformProp.scaleX || BridgeTransformProp.scaleY => (
        l10n.transformScale,
        l10n.unitPercent
      ),
    BridgeTransformProp.rotation => (l10n.transformRotation, l10n.unitDegrees),
    BridgeTransformProp.rotationX => (
        l10n.transformRotationX,
        l10n.unitDegrees
      ),
    BridgeTransformProp.rotationY => (
        l10n.transformRotationY,
        l10n.unitDegrees
      ),
    BridgeTransformProp.opacity => (l10n.transformOpacity, l10n.unitPercent),
  };
  // The union of the axes' key frames: an axis with no key on some frame
  // contributes the value its curve reads there, so every row is complete.
  final frames = <double>{};
  for (final axis in axes) {
    for (final k in axis.keys) {
      frames.add(_keyFrame(k, fps));
    }
  }
  final sorted = frames.toList()..sort();
  final columns = axes.length == 1
      ? [unit]
      : [
          for (var i = 0; i < axes.length; i++)
            '${_axisLetter(i).toUpperCase()} $unit'
        ];

  /// The key an axis has exactly on `frame`, if any — the one whose easing
  /// the row carries. A filled-in value has no key, and so no easing.
  BridgeKeyframe? keyAt(GraphClipChannel axis, double frame) {
    for (final k in axis.keys) {
      if ((_keyFrame(k, fps) - frame).abs() < 1e-9) return k;
    }
    return null;
  }

  return LumitClipGroup(
    property: [l10n.transformSection, name],
    columns: columns,
    rows: [
      for (final f in sorted)
        LumitClipRow(
          frame: f,
          values: [
            for (final axis in axes)
              keyAt(axis, f)?.value ??
                  evaluateKeys(axis.keys, f / (fps <= 0 ? 1 : fps)),
          ],
          eases: [
            for (final axis in axes)
              switch (keyAt(axis, f)) {
                final BridgeKeyframe k => (k.interpIn, k.interpOut),
                _ => (
                    const BridgeSideInterp.linear(),
                    const BridgeSideInterp.linear()
                  ),
              },
          ],
        ),
    ],
  );
}

/// Paste the clipboard into the currently selected channels, the earliest key
/// landing on the playhead. The in-app clipboard pastes first; failing that,
/// keyframe text on the system clipboard is parsed — with its easing when it
/// carries any. Channels are matched in order.
///
/// [project] is what makes a paste **one undo step** (K-458): a clipboard that
/// came off three properties on two layers writes one op per layer per kind,
/// and one press of Ctrl+V is one press. Optional, so a caller with no project
/// to hand — a widget test — pastes exactly as it always did.
Future<bool> pasteKeysAtPlayhead({
  required List<GraphChannel> channels,
  required int playheadFrame,
  required double fps,
  required int fpsNum,
  required int fpsDen,
  ProjectReference? project,
}) async {
  if (channels.isEmpty) return false;

  // A value copied from a row with no keyframes pastes as a value (K-301): onto
  // a target that is not animated it simply replaces the number, and onto one
  // that is it sets a key at the playhead — which is what "put this value here"
  // means on a row that already moves.
  final statics = <double?>[
    for (final clip in graphKeyClipboard) clip.staticValue,
  ];

  // (channel keys to merge in) per target channel, times as comp frames.
  var sources = <List<(double, BridgeKeyframe)>>[];
  if (graphKeyClipboard.isNotEmpty) {
    sources = [
      for (final clip in graphKeyClipboard)
        [for (final k in clip.keys) (_keyFrame(k, fps), k)],
    ];
  } else {
    final text = (await Clipboard.getData(Clipboard.kTextPlain))?.text;
    final parsed = text == null ? null : parseClipboardText(text);
    if (parsed == null) return false;
    // The table's frames are in whatever rate it was written at; carry them
    // across as real time rather than as frame numbers.
    for (final group in parsed.groups) {
      final columns = group.rows.isEmpty ? 0 : group.rows.first.values.length;
      for (var c = 0; c < columns; c++) {
        sources.add([
          for (final row in group.rows)
            if (c < row.values.length)
              (
                row.frame / parsed.fps * fps,
                BridgeKeyframe(
                  // Placeholder time; rewritten with the shift below.
                  time: const BridgeRational(num: 0, den: 1),
                  value: row.values[c],
                  interpIn: c < row.eases.length
                      ? row.eases[c].$1
                      : const BridgeSideInterp.linear(),
                  interpOut: c < row.eases.length
                      ? row.eases[c].$2
                      : const BridgeSideInterp.linear(),
                ),
              ),
        ]);
      }
    }
  }
  if (sources.isEmpty) return false;

  var earliest = double.infinity;
  for (final source in sources) {
    for (final (frame, _) in source) {
      if (frame < earliest) earliest = frame;
    }
  }
  // Values only: nothing has a time, so there is no shift to work out and the
  // paste is not about the playhead at all.
  if (!earliest.isFinite && statics.every((v) => v == null)) return false;
  final shift = earliest.isFinite ? playheadFrame - earliest : 0.0;

  final edits = <GraphChannel, BridgeScalar>{};
  for (var i = 0; i < channels.length && i < sources.length; i++) {
    final channel = channels[i];
    final value = i < statics.length ? statics[i] : null;
    if (value != null) {
      edits[channel] = channel.isStatic
          ? BridgeScalar.static_(value)
          : BridgeScalar.keyframed([
              for (final k in _withKeyAt(
                channel.keys,
                playheadFrame.toDouble(),
                value,
                fps,
                fpsNum,
                fpsDen,
              ))
                k,
            ]);
      continue;
    }
    // Merge on frames: a pasted key replaces one already at its frame — two
    // keys at one time is not a curve the engine will take.
    final merged = <double, BridgeKeyframe>{
      for (final k in channel.keys) _keyFrame(k, fps): k,
    };
    for (final (frame, key) in sources[i]) {
      final at = frame + shift;
      merged[at] = BridgeKeyframe(
        time: timeOfSubframe(at, fpsNum, fpsDen),
        value: key.value,
        interpIn: key.interpIn,
        interpOut: key.interpOut,
      );
    }
    final frames = merged.keys.toList()..sort();
    edits[channel] =
        BridgeScalar.keyframed([for (final f in frames) merged[f]!]);
  }
  if (edits.isEmpty) return false;
  asOneUndoStep(project, () => commitChannelEdits(edits));
  return true;
}

// ---------------------------------------------------------------------------
// The pane.
// ---------------------------------------------------------------------------

class GraphEditorFrb extends StatefulWidget {
  final CompositionReference comp;
  final List<GraphChannel> channels;
  final TimelineAxis axis;

  /// The Timeline's horizontal scroll controller, so the value axis can be
  /// pinned to the viewport rather than to the start of time. Optional: a test
  /// that builds the pane alone has no scroll view around it.
  final ScrollController? hScroll;
  final int frames;
  final double fps;
  final int fpsNum;
  final int fpsDen;
  final bool magnet;

  /// The Timeline's shared snap targets (docs/07 §4.5). A key drag on this
  /// pane reaches for the same landmarks a lane key's does. Empty leaves it
  /// snapping to whole frames alone, which is what a pane built on its own in
  /// a test wants.
  final List<SnapTarget> snapTargets;

  final GraphLens lens;

  /// Auto-fit: the vertical range follows the curves (docs/07 §5.3). Off, the
  /// range is the user's — the wheel pans it and `Alt`+wheel zooms it.
  final bool autoFit;

  /// Whether the Pen tool is armed on the toolbar (docs/07 §1.7).
  ///
  /// With it in hand the graph plants and lifts keys on a single click — the
  /// same thing the Pen does to a path, done to a curve. Everything it offers
  /// is reachable without it (double-click, `Ctrl`-click, `Alt`-click), so
  /// nobody has to hold a tool to edit a curve.
  final bool penArmed;

  /// Settings ▸ Interface ▸ Editing ▸ *Retime opens to Velocity* (K-246).
  ///
  /// On, a **Retime** channel's speed view becomes the Vegas envelope of
  /// K-247: one point per key whose height is the playback speed in per cent,
  /// straight lines between them, and the frames after a dragged point
  /// re-integrated. Off — and for every channel that is not a Retime, in
  /// either mode — the speed view is the ordinary two-sided derivative graph
  /// with its in and out dots. Nothing about the value view changes either way.
  final bool vegas;

  /// The selected keys, as `channelId#index` — owned by the Timeline panel so
  /// the bottom bar and the shortcuts act on the same set.
  final Set<String> selectedKeys;
  final VoidCallback onSelectionChanged;
  final VoidCallback onChanged;

  /// `Ctrl`/`Shift` wheels go to the panel: time zoom about the pointer and
  /// horizontal scroll are the Timeline's own, shared with the lane view.
  final void Function(PointerScrollEvent event, double contentX) onWheelTime;

  const GraphEditorFrb({
    super.key,
    required this.comp,
    required this.channels,
    required this.axis,
    this.hScroll,
    required this.frames,
    required this.fps,
    required this.fpsNum,
    required this.fpsDen,
    required this.magnet,
    this.snapTargets = const [],
    required this.lens,
    required this.autoFit,
    this.vegas = false,
    this.penArmed = false,
    required this.selectedKeys,
    required this.onSelectionChanged,
    required this.onChanged,
    required this.onWheelTime,
  });

  @override
  State<GraphEditorFrb> createState() => GraphEditorFrbState();
}

/// A key drag in flight: which key was grabbed and how far the gesture has
/// moved, applied to every selected key for the preview and committed once.
/// A key drag in flight. The gesture's travel is kept **raw**, and the
/// `Shift` constraint is applied where it is read rather than where it is
/// accumulated — so the axis can change as the pointer moves, and letting
/// `Shift` go mid-drag restores the full travel instead of losing whatever
/// was suppressed while it was held.
class _KeyDrag {
  final String grabbedId;

  /// Everything the pointer has travelled, `Shift` or no `Shift`.
  double rawDx = 0;
  double rawDy = 0;

  _KeyDrag(this.grabbedId);

  /// Which way a `Shift`-constrained drag is going: whichever axis the
  /// pointer has travelled further along **in pixels**.
  ///
  /// Pixels, not values: the two axes carry different units at different
  /// zooms — seconds against source-seconds, or per cent — so comparing the
  /// numbers themselves would make the constraint depend on how far the graph
  /// happens to be zoomed rather than on the gesture the hand made.
  bool get _horizontal => rawDx.abs() >= rawDy.abs();

  double get dxPx =>
      !HardwareKeyboard.instance.isShiftPressed || _horizontal ? rawDx : 0;
  double get dyPx =>
      !HardwareKeyboard.instance.isShiftPressed || !_horizontal ? rawDy : 0;
}

/// Where a gesture in flight puts a selected key: given the channel it is on
/// and the frame and value it rests at, the frame and value it should be
/// **drawn** at and, on release, written to.
///
/// One shape for every move on this pane, so that the preview and the commit
/// cannot drift apart: the shown keys are built from it ([_withMove]) and so
/// are the edits ([_editsFor]). A key drag adds the same delta to every key;
/// the transform box scales each key by its distance from an anchor; both are
/// the same function seen from different ends.
typedef _KeyMove = (double frame, double value) Function(
    GraphChannel channel, double frame, double value);

/// A drag on the **selection transform box** (§6.2, docs/07 §5.3): which of
/// the box's two axes the grabbed handle scales, which edge is in hand, and
/// how far the pointer has taken it.
///
/// The box spans the selected keys in time and in value; a handle scales the
/// selection about the **opposite** edge, so the edge you are not holding is
/// the one that stays put — a stretch rather than a move, exactly as a lane
/// block's handle behaves (K-458).
class _BoxDrag {
  /// Which axis this edge scales: the left and right edges scale **time**, the
  /// top and bottom edges scale **value**. One axis each, so the box answers
  /// "how long" and "how far" as two separate questions — which is how the
  /// study describes the gesture, and what keeps the anchor unambiguous.
  final bool time;

  /// Whether the edge in hand is the earlier frame / the higher value, so the
  /// other end of the axis can be the anchor.
  final bool start;

  /// The end that stays put and the end that was grabbed — frames on the time
  /// axis, **pixels** on the value axis (see [scaledAbout]).
  final double anchor;
  final double from;

  /// Everything the pointer has travelled, `Shift` or no `Shift` — the same
  /// bargain [_KeyDrag] strikes, so letting the modifier go mid-drag gives the
  /// suppressed travel back rather than losing it.
  double rawDx = 0;
  double rawDy = 0;

  _BoxDrag({
    required this.time,
    required this.start,
    required this.anchor,
    required this.from,
  });

  /// The travel along the axis this edge scales. The other axis is ignored
  /// outright: an edge is a one-dimensional grab, and following the pointer
  /// sideways as well would make the box a move as well as a scale.
  double get travel => time ? rawDx : rawDy;

  /// `Shift` on this box is the Caddis modifier: it **rounds what the scale
  /// lands on** — whole frames in time, whole numbers in value — with the
  /// readout pill saying live what those numbers are. (The axis lock the same
  /// sentence of the study describes belongs to the box's other gesture, the
  /// *slide*, which is the key drag `Shift` already constrains — see
  /// [_KeyDrag].)
  ///
  /// `Ctrl` is deliberately not the taper docs/07 §5.3 once named: it already
  /// suspends the magnet on every other drag in this panel, and the taper has
  /// no drawing, no recorded arithmetic and no second gesture asking for it.
  /// See docs/impl/timeline-interaction.md §6.2.
  static bool get shiftHeld => HardwareKeyboard.instance.isShiftPressed;
}

/// A tangent-handle drag in flight (value lens), or a speed-dot/influence
/// drag (speed lens).
class _HandleDrag {
  final GraphChannel channel;
  final int index;
  final bool isOut;

  /// Whether the other side follows: joined by default, `Alt` at drag start
  /// flips it — held apart if they were together, re-joined if they were
  /// apart. False at once when the other side has no span to reach into.
  final bool mirrored;
  double speed;
  double influence;

  /// The partner side's provisional easing while the drag runs, so the curve
  /// and both handles move together rather than the other side jumping on
  /// release.
  double partnerSpeed;
  double partnerInfluence;

  /// How long the partner handle was **on screen**, in pixels, when the drag
  /// began. A joined partner keeps that pixel length however the dragged side
  /// swings: a handle's *value* length is meaningless to the eye — what the
  /// user sees is its length in the panel, and it must not appear to grow
  /// when the pair rotates toward vertical.
  final double partnerLenPx;

  /// Speed lens only: this is a keyframe dot rather than an influence handle
  /// — it drags the key's time sideways and that side's speed vertically.
  final bool dotOnly;

  /// The vertical range and pane height the gesture is running under — frozen
  /// with it, so the commit can record the handles' lengths against the same
  /// scale it drew them at.
  final (double, double) range;
  final double height;

  /// Pixels the dot has travelled sideways (speed lens dot drags only),
  /// before the `Shift` constraint — see [dxPx].
  double rawDx = 0;

  /// The pointer's vertical travel in pixels, and the speed the dot sat at
  /// when the gesture began. Together they let `Shift` hold the speed exactly
  /// where it started while the key moves in time.
  double rawDy = 0;
  double startSpeed = 0;

  /// Which way a `Shift`-constrained dot drag is going: whichever axis the
  /// pointer has travelled further along in **pixels**, so the choice follows
  /// the gesture rather than the zoom (see [_KeyDrag]).
  bool get _horizontal => rawDx.abs() >= rawDy.abs();

  bool get _constrained => HardwareKeyboard.instance.isShiftPressed;

  /// Sideways travel, held at zero while `Shift` makes this a vertical drag.
  double get dxPx => !_constrained || _horizontal ? rawDx : 0;

  /// The speed to draw and to commit: the pointer's, or the one the gesture
  /// started at while `Shift` makes this a horizontal drag.
  double get shownSpeed => _constrained && _horizontal ? startSpeed : speed;

  _HandleDrag({
    required this.channel,
    required this.index,
    required this.isOut,
    required this.mirrored,
    required this.speed,
    required this.influence,
    required this.partnerSpeed,
    required this.partnerInfluence,
    required this.partnerLenPx,
    required this.range,
    required this.height,
    this.dotOnly = false,
  });
}

class GraphEditorFrbState extends State<GraphEditorFrb> {
  _KeyDrag? _keyDrag;

  /// A drag on the selection transform box, or null between them (§6.2).
  _BoxDrag? _boxDrag;

  /// `Escape` abandons the drag in flight and writes nothing (P3, §8's gap
  /// 19): the box scale, the key drag and the handle drag alike, on the one
  /// shared mechanism the lanes already use.
  final DragEscape _escape = DragEscape();

  /// When a key was last clicked, and which one — for spotting the
  /// double-click that opens the numeric fields (§6.2's numeric entry).
  ///
  /// Counted with [DoubleTap] rather than by registering `onDoubleTap` beside
  /// the key's own tap, for the reason the pane's own double-click is counted
  /// this way: a double-tap recogniser makes Flutter hold every *single* tap
  /// back until its timer expires, so selecting a key — the commonest thing
  /// anyone does here — would gain a visible delay.
  final _keyTap = DoubleTap();
  String? _keyTapId;

  /// How often a drag in flight may ask the engine for a frame of the values it
  /// is about to write (see [previewChannelEdits]).
  final PreviewThrottle _preview = PreviewThrottle();

  @override
  void initState() {
    super.initState();
    rowValueDrag.addListener(_rowDragChanged);
  }

  void _rowDragChanged() {
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    rowValueDrag.removeListener(_rowDragChanged);
    _preview.cancel();
    _escape.dispose();
    super.dispose();
  }

  /// When and where the pane was last clicked, for spotting a double-click
  /// (see [_tapPane]).
  final _paneTap = DoubleTap();
  _HandleDrag? _handleDrag;

  /// The vertical range on screen while a gesture is in flight — held still
  /// so the curve being dragged does not re-frame itself under the pointer.
  (double, double)? _frozen;

  /// The user's own range per lens, once auto-fit is off.
  final Map<GraphLens, (double, double)> _manual = {};

  /// The last range a build computed — what manual mode starts from, and what
  /// the wheel handlers scale.
  (double, double) _lastRange = (0, 1);
  Size _paneSize = Size.zero;

  /// How long each tangent handle was last drawn, in pixels, by channel, key
  /// time and side.
  ///
  /// **Why this is remembered rather than measured.** A handle's length on
  /// screen comes from its reach in *time*, and a tangent swung near vertical
  /// has almost none — its length there is carried almost entirely by its
  /// speed instead. Measuring the length back out of a stored ease is exact in
  /// theory and lossy in practice at that extreme, so a partner mirrored while
  /// near-vertical could come back a different length than it went in. Keeping
  /// the number means a handle is the length you last left it, and swinging
  /// the pair out to vertical and back returns it unchanged.
  ///
  /// Keyed by the keyframe's *time*, not its index, so it belongs to the key
  /// rather than to a position in a list; a key moved in time simply falls
  /// back to its measured length, which is right there anyway. The scales it
  /// was measured under ride along, because a pixel length means nothing after
  /// the view has zoomed or re-framed — a remembered number read under a
  /// different scale is quietly discarded rather than shrinking the handle.
  final Map<String, ({double lenPx, double xScale, double yScale})>
      _handleLenPx = {};

  String _handleLenKey(GraphChannel channel, BridgeKeyframe key, bool isOut) =>
      '${channel.id}#${key.time.num}/${key.time.den}-${isOut ? 'out' : 'in'}';

  /// Pixels per second, and pixels per unit of value, as the pane stands.
  (double, double) _scales((double, double) range, double height) {
    final span =
        (range.$2 - range.$1).abs() < 1e-12 ? 1.0 : range.$2 - range.$1;
    return (
      widget.axis.perFrame * (widget.fps <= 0 ? 1 : widget.fps),
      height / span,
    );
  }

  /// The length a side is drawn at, from its stored ease.
  double _measuredLength(GraphChannel channel, int index, bool isOut,
      (double, double) range, double height) {
    final keys = channel.keys;
    final key = keys[index];
    final end = _sideEndpoint(keys, index, isOut);
    final keyPx = Offset(
        _xOfSeconds(rationalSeconds(key.time)), _yOf(key.value, range, height));
    return (Offset(_xOfSeconds(end.time), _yOf(end.value, range, height)) -
            keyPx)
        .distance;
  }

  /// A side's length on screen: the one remembered for it if it was taken
  /// under the scales in force now, else the one its stored ease draws it at.
  double _handleLength(GraphChannel channel, int index, bool isOut,
      (double, double) range, double height) {
    final held =
        _handleLenPx[_handleLenKey(channel, channel.keys[index], isOut)];
    final (xScale, yScale) = _scales(range, height);
    if (held != null &&
        (held.xScale - xScale).abs() < 1e-6 * (1 + xScale.abs()) &&
        (held.yScale - yScale).abs() < 1e-6 * (1 + yScale.abs())) {
      return held.lenPx;
    }
    return _measuredLength(channel, index, isOut, range, height);
  }

  void _rememberLength(GraphChannel channel, BridgeKeyframe key, bool isOut,
      double lenPx, (double, double) range, double height) {
    final (xScale, yScale) = _scales(range, height);
    _handleLenPx[_handleLenKey(channel, key, isOut)] =
        (lenPx: lenPx, xScale: xScale, yScale: yScale);
  }

  /// Re-frame the curves now (`F`, docs/07 §5.3): in manual mode the fitted
  /// range becomes the manual one; in auto mode the next build fits anyway.
  void fitNow() => setState(() => _manual.remove(widget.lens));

  /// Delete the selected keys — the Timeline's Delete shortcut.
  void deleteSelectedKeys() => _deleteSelection();

  List<List<BridgeKeyframe>> get _channelKeys =>
      [for (final c in widget.channels) c.keys];

  /// The stretch of time actually on screen, in seconds — or null when the pane
  /// is not inside a scroll view (a test builds it alone) and everything is.
  (double, double)? get _visibleSeconds {
    final c = widget.hScroll;
    if (c == null || !c.hasClients) return null;
    final width = c.position.viewportDimension;
    if (width <= 0) return null;
    return (_secondsOfX(c.offset), _secondsOfX(c.offset + width));
  }

  /// Each channel's keys as the fit should see them: **only the ones on
  /// screen**, plus what the curve reads at each edge of the view.
  ///
  /// Auto-fit frames the curves, and "the curves" means the part of them you
  /// are looking at. Fitting over every key regardless left the vertical
  /// framing fixed however far the time axis was zoomed in — zoom into a
  /// quiet stretch of a curve that spikes somewhere off-screen and the pane
  /// still made room for the spike, so the part under the pointer stayed a
  /// flat line (K-333).
  ///
  /// The edge samples are what stop a span *between* two keys from framing on
  /// nothing: zoomed between them there is no key in view at all, and the
  /// value there is the whole of what the view shows.
  List<List<BridgeKeyframe>> get _visibleChannelKeys {
    final window = _visibleSeconds;
    if (window == null) return _channelKeys;
    final (t0, t1) = window;
    return [
      for (final channel in widget.channels)
        () {
          final keys = channel.keys;
          if (keys.isEmpty) return keys;
          final shown = [
            for (final k in keys)
              if (rationalSeconds(k.time) >= t0 &&
                  rationalSeconds(k.time) <= t1)
                k,
          ];
          return [
            ...shown,
            for (final t in [t0, t1])
              BridgeKeyframe(
                time: timeOfSubframe(
                    t * widget.fps, widget.fpsNum, widget.fpsDen),
                value: evaluateKeys(keys, t),
                interpIn: const BridgeSideInterp.linear(),
                interpOut: const BridgeSideInterp.linear(),
              ),
          ];
        }(),
    ];
  }

  /// Which reading [channel] draws in. Everything follows the view's lens
  /// except a mask's **shape** (K-344), which has no value to plot and so
  /// draws its rate of change in both.
  GraphLens lensOf(GraphChannel channel) =>
      channel.isMaskPath ? GraphLens.speed : widget.lens;

  /// Whether [channel] draws as the Vegas speed envelope right now (K-247) —
  /// a Retime, in the speed view, with the preference on.
  bool isEnvelope(GraphChannel channel) =>
      widget.vegas && channel.retime && widget.lens == GraphLens.speed;

  bool get _anyEnvelope => widget.channels.any(isEnvelope);

  (double, double) _fitRange() {
    // Only shapes on screen means only speeds on screen (K-344).
    final allPaths = widget.channels.isNotEmpty &&
        widget.channels.every((c) => c.isMaskPath);
    if (widget.lens == GraphLens.value && !allPaths) {
      return fitValueRange(
        _visibleChannelKeys,
        [
          for (final c in widget.channels)
            if (c.isStatic) c.staticValue,
        ],
      );
    }
    if (!_anyEnvelope) return fitSpeedRange(_visibleChannelKeys);
    // An envelope brings its own floor and ceiling (100% down to −25%), which
    // the ordinary speed fit has no business inventing. Any other channel
    // selected alongside is framed as before and the two ranges are unioned —
    // the speed view has always put unlike units on one axis (px/s beside
    // %/s), so this is not a new compromise, just a wider one.
    final envelope = fitEnvelopeRange([
      for (final c in widget.channels)
        if (isEnvelope(c)) c.keys
    ]);
    final others = [
      for (final c in widget.channels)
        if (!isEnvelope(c)) c.keys
    ];
    if (others.isEmpty) return envelope;
    final rest = fitSpeedRange(others);
    return (
      envelope.$1 < rest.$1 ? envelope.$1 : rest.$1,
      envelope.$2 > rest.$2 ? envelope.$2 : rest.$2,
    );
  }

  /// The Timeline's horizontal scroll offset, or zero before the view has been
  /// laid out (and in tests, which build the pane on its own).
  double get _viewportLeft {
    final c = widget.hScroll;
    return c != null && c.hasClients ? c.offset : 0;
  }

  /// How wide that viewport is. Falls back to the pane's own width for a test
  /// that builds the pane with no scroll view around it — where the canvas
  /// *is* the viewport.
  double _viewportWidth(double paneWidth) {
    final c = widget.hScroll;
    return c != null && c.hasClients ? c.position.viewportDimension : paneWidth;
  }

  (double, double) _range() {
    final frozen = _frozen;
    if (frozen != null) return frozen;
    if (!widget.autoFit) {
      return _manual[widget.lens] ??= _fitRange();
    }
    return _fitRange();
  }

  double _yOf(double v, (double, double) range, double height) {
    final (lo, hi) = range;
    final span = (hi - lo).abs() < 1e-12 ? 1.0 : hi - lo;
    return height - (v - lo) / span * height;
  }

  double _valueAt(double y, (double, double) range, double height) {
    final (lo, hi) = range;
    final span = (hi - lo).abs() < 1e-12 ? 1.0 : hi - lo;
    return lo + (height - y) / (height <= 0 ? 1 : height) * span;
  }

  /// A key's y in the current lens: its value, one side's speed, or — as an
  /// envelope point — its playback speed in per cent.
  double _keyY(
      GraphChannel channel, int index, (double, double) range, double height,
      {required bool isOut}) {
    // Through [_shownKeys], never the document's keys, in every lens: the
    // drag previews — a row drag's published value, a handle's provisional
    // sides — live in the shown list, and the diamond has to sit on the curve
    // that is actually being drawn (K-334, K-336).
    final shown = _shownKeys(channel);
    if (index >= shown.length) return 0;
    if (lensOf(channel) == GraphLens.value) {
      return _yOf(shown[index].value, range, height);
    }
    if (isEnvelope(channel)) {
      return _yOf(envelopeSpeeds(shown)[index], range, height);
    }
    return _yOf(sideSpeedAtKey(shown, index, isOut: isOut), range, height);
  }

  // --- wheel ---------------------------------------------------------------

  void _wheel(PointerScrollEvent event) {
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed || keys.isShiftPressed) {
      widget.onWheelTime(event, event.localPosition.dx);
      return;
    }
    // The vertical axis is the user's only once auto-fit is off.
    if (widget.autoFit || _paneSize.height <= 0) return;
    final range = _manual[widget.lens] ??= _lastRange;
    final (lo, hi) = range;
    final span = hi - lo;
    if (altActuallyHeld()) {
      // Zoom about the pointer: the value under the cursor stays put. The
      // anchor is clamped to the pane, because the pointer signal is reported
      // against a listener that is taller than the graph — an anchor from
      // outside it zooms about a value nowhere near the curve, which is how a
      // few ticks turned into a range of millions.
      final at = _valueAt(
        event.localPosition.dy.clamp(0.0, _paneSize.height),
        range,
        _paneSize.height,
      );
      final factor = event.scrollDelta.dy < 0 ? 1 / 1.2 : 1.2;
      setState(() => _manual[widget.lens] =
          _sane((at - (at - lo) * factor, at + (hi - at) * factor)));
      return;
    }
    // Wheel down moves the *content* up, as scrolling does everywhere: the
    // window onto the values travels the other way to the wheel.
    final pan = -event.scrollDelta.dy / _paneSize.height * span;
    setState(() => _manual[widget.lens] = _sane((lo + pan, hi + pan)));
  }

  /// A vertical range the pane can actually draw: finite, the right way up, and
  /// within a thousandfold of the range auto-fit would choose.
  ///
  /// **Why there has to be a floor and a ceiling.** Alt+wheel multiplies the
  /// span by 1.2 a tick, so half a second of scrolling is a range hundreds of
  /// times the curve and a few seconds is millions. Nothing refuses it, and
  /// nothing about the pane then looks broken — it looks *dead*: the curve is
  /// far outside the window, a pan of one wheel notch moves it by a fraction of
  /// a span nobody can see, and only another Alt+wheel — being multiplicative —
  /// can climb back. That is the "no other scroll works until I press Alt
  /// again" report, and it was never the Alt key at all (K-333).
  (double, double) _sane((double, double) range) {
    final fit = _fitRange();
    final reference = (fit.$2 - fit.$1).abs();
    final limit = reference.isFinite && reference > 1e-9 ? reference : 1.0;
    var (lo, hi) = range;
    if (!lo.isFinite || !hi.isFinite || hi <= lo) return fit;
    final span = (hi - lo).clamp(limit / 1000, limit * 1000);
    final middle = (lo + hi) / 2;
    return (middle - span / 2, middle + span / 2);
  }

  // --- selection -----------------------------------------------------------

  bool get _addToSelection =>
      HardwareKeyboard.instance.isShiftPressed ||
      HardwareKeyboard.instance.isControlPressed;

  void _selectKey(String id, {bool toggle = false}) {
    if (toggle && widget.selectedKeys.contains(id)) {
      widget.selectedKeys.remove(id);
    } else if (_addToSelection) {
      widget.selectedKeys.add(id);
    } else {
      widget.selectedKeys
        ..clear()
        ..add(id);
    }
    widget.onSelectionChanged();
  }

  /// [additive] is read by the marquee at the drag's *start* (K-500 §2.1) —
  /// the modifier decides the gesture when it begins, not when it ends.
  void _applyMarquee(
      Rect rect, (double, double) range, double height, bool additive) {
    if (!additive) widget.selectedKeys.clear();
    for (final channel in widget.channels) {
      final keys = channel.keys;
      for (var i = 0; i < keys.length; i++) {
        final x = widget.axis.xOf(_keyFrame(keys[i], widget.fps));
        final hit = widget.lens == GraphLens.value
            ? rect.contains(
                Offset(x, _keyY(channel, i, range, height, isOut: true)))
            : rect.contains(
                    Offset(x, _keyY(channel, i, range, height, isOut: true))) ||
                rect.contains(
                    Offset(x, _keyY(channel, i, range, height, isOut: false)));
        if (hit) widget.selectedKeys.add('${channel.id}#$i');
      }
    }
    widget.onSelectionChanged();
  }

  /// A plain click on empty pane clears the selection; `Ctrl`+click plants a
  /// key on the curve under the pointer (docs/07 §4.3's lane gesture, read
  /// through the graph).
  void _tapPane(Offset local, (double, double) range, double height) {
    if (HardwareKeyboard.instance.isControlPressed || widget.penArmed) {
      _addKeyAt(local, range, height);
      return;
    }
    // The second click of a double-click plants a key on the curve.
    //
    // Counted with [DoubleTap] rather than an `onDoubleTap` beside the pane's
    // own gesture, because the pane reports taps through `onTapUp`, which
    // claims the gesture arena the moment the first tap lifts — a double-tap
    // recogniser next to it never gets to form, so the gesture simply never
    // fired. Timestamps do the same job with none of the arena's opinions.
    if (_paneTap.tap(at: local, slop: _keyGrab)) {
      _addKeyAt(local, range, height);
      return;
    }
    if (widget.selectedKeys.isEmpty) return;
    widget.selectedKeys.clear();
    widget.onSelectionChanged();
  }

  /// Plant a key on the curve under the pointer, without changing its shape.
  ///
  /// Works in **either lens**: the key's value is the curve's own value at
  /// that moment, so the picture does not move — adding a point is a place to
  /// grab, not an edit. In the Vegas envelope the new point also takes the
  /// speed the envelope already reads there, so the straight line it sits on
  /// stays straight (K-247).
  void _addKeyAt(Offset local, (double, double) range, double height) {
    final frame = widget.magnet
        ? widget.axis.frameAt(local.dx).toDouble()
        : widget.axis.frameAtExact(local.dx);
    final seconds = frame / (widget.fps <= 0 ? 1 : widget.fps);

    // The curve nearest the pointer vertically takes the key — measured in
    // whichever reading is on screen, so the speed view picks by the speed
    // curve the user is actually looking at.
    GraphChannel? nearest;
    var best = 12.0;
    for (final channel in widget.channels) {
      final keys = channel.keys;
      final double drawn;
      if (lensOf(channel) == GraphLens.value) {
        drawn = channel.isStatic
            ? channel.staticValue
            : evaluateKeys(keys, seconds);
      } else {
        drawn = channel.isStatic
            ? 0
            : evaluateKeysSpeed(keys, seconds) *
                (isEnvelope(channel) ? 100 : 1);
      }
      final d = (_yOf(drawn, range, height) - local.dy).abs();
      if (d < best) {
        best = d;
        nearest = channel;
      }
    }
    final channel = nearest;
    if (channel == null) return;

    final keys = channel.keys;
    final taken = {for (final k in keys) _keyFrame(k, widget.fps).round()};
    if (taken.contains(frame.round())) return;
    final time = timeOfSubframe(frame, widget.fpsNum, widget.fpsDen);

    // An envelope keeps its shape by construction: give the new point the
    // speed the line already has there and re-integrate through it.
    if (isEnvelope(channel) && keys.length >= 2) {
      final speeds = envelopeSpeeds(keys);
      var at = keys.length;
      for (var i = 0; i < keys.length; i++) {
        if (seconds < rationalSeconds(keys[i].time)) {
          at = i;
          break;
        }
      }
      final double planted;
      if (at == 0) {
        planted = speeds.first;
      } else if (at == keys.length) {
        planted = speeds.last;
      } else {
        final t0 = rationalSeconds(keys[at - 1].time);
        final t1 = rationalSeconds(keys[at].time);
        final f = t1 > t0 ? (seconds - t0) / (t1 - t0) : 0.0;
        planted = speeds[at - 1] + (speeds[at] - speeds[at - 1]) * f;
      }
      final grown = [...keys]..insert(
          at,
          BridgeKeyframe(
            time: time,
            value: 0,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          ));
      final withSpeed = [...speeds]..insert(at, planted);
      commitChannelEdits({
        channel: BridgeScalar.keyframed(envelopeToKeys(grown, withSpeed)),
      });
      widget.onChanged();
      return;
    }

    final value =
        channel.isStatic ? channel.staticValue : evaluateKeys(keys, seconds);
    final next = [
      ...keys,
      BridgeKeyframe(
        time: time,
        value: value,
        interpIn: const BridgeSideInterp.linear(),
        interpOut: const BridgeSideInterp.linear(),
      ),
    ]..sort(
        (a, b) => rationalSeconds(a.time).compareTo(rationalSeconds(b.time)));
    commitChannelEdits({channel: BridgeScalar.keyframed(next)});
    widget.onChanged();
  }

  /// Take one key off [channel], keeping every other key exactly as it is.
  ///
  /// The counterpart of [_addKeyAt]: `Alt`-click or double-click a key, or
  /// click one with the Pen armed. A channel's last key is refused rather than
  /// leaving a keyframed property with nothing in it.
  void _removeKey(GraphChannel channel, int index) {
    final keys = channel.keys;
    if (index < 0 || index >= keys.length || keys.length <= 1) return;
    final next = [
      for (var i = 0; i < keys.length; i++)
        if (i != index) keys[i],
    ];
    widget.selectedKeys.removeWhere((id) => id.startsWith('${channel.id}#'));
    commitChannelEdits({channel: BridgeScalar.keyframed(next)});
    widget.onSelectionChanged();
    widget.onChanged();
  }

  // --- key drags -----------------------------------------------------------

  void _startKeyDrag(String id) {
    if (!widget.selectedKeys.contains(id)) _selectKey(id);
    setState(() {
      _keyDrag = _KeyDrag(id);
      _frozen = _range();
    });
    _escape.begin(_abandonDrag);
  }

  /// Put the pane back where the gesture found it and write nothing — what
  /// `Escape` does to any drag in flight (P3).
  ///
  /// The provisional geometry *is* the drag object, so dropping it is the
  /// whole revert: the curve, the glyphs, the handles and the box all derive
  /// from [_shownKeys], which reads the document again the moment there is no
  /// move to fold in.
  void _abandonDrag() {
    _preview.cancel();
    if (!mounted) return;
    setState(() {
      _keyDrag = null;
      _boxDrag = null;
      _handleDrag = null;
      _frozen = null;
    });
  }

  void _commitKeyDrag((double, double) range, double height) {
    final drag = _keyDrag;
    final commit = _escape.end();
    setState(() {
      _keyDrag = null;
      _frozen = null;
    });
    if (!commit || drag == null || (drag.dxPx == 0 && drag.dyPx == 0)) return;
    // The commit is the last word on this gesture: a preview tick still held
    // would put the provisional picture back on top of it.
    _preview.cancel();
    final (edits, newSelection) = _keyDragEdits(drag, range, height);
    if (edits.isEmpty) return;
    commitChannelEdits(edits);
    widget.selectedKeys
      ..clear()
      ..addAll(newSelection);
    widget.onSelectionChanged();
    widget.onChanged();
  }

  /// What a key drag would write, and the selection it would leave.
  (Map<GraphChannel, BridgeScalar>, Set<String>) _keyDragEdits(
          _KeyDrag drag, (double, double) range, double height) =>
      _editsFor(_keyDragMove(drag, range, height));

  /// Where the **grabbed** key sits in the document, or null when the drag's
  /// id no longer names a key (a channel list rebuilt under it).
  double? _grabbedFrame(_KeyDrag drag) {
    final cut = drag.grabbedId.lastIndexOf('#');
    if (cut < 0) return null;
    final id = drag.grabbedId.substring(0, cut);
    final index = int.tryParse(drag.grabbedId.substring(cut + 1));
    if (index == null) return null;
    for (final channel in widget.channels) {
      if (channel.id != id) continue;
      if (index < 0 || index >= channel.keys.length) return null;
      return _keyFrame(channel.keys[index], widget.fps);
    }
    return null;
  }

  /// A key drag's sideways travel, taken to the nearest **time landmark**
  /// while the magnet is on (§4.5): markers (beat markers among them), the
  /// playhead, the work-area edges, layer ends and edit points — but **not
  /// other keyframes**, which are everywhere on this pane and would make every
  /// drag sticky against the very things it is rearranging.
  ///
  /// Decided from the grabbed key alone and then applied to the whole
  /// selection, so a run of keys keeps its shape rather than each key finding
  /// its own target. `Ctrl` suspends it, as everywhere else.
  ///
  /// Pure, so the preview, the commit and the capture indicator all read one
  /// answer — which is why it can be called from `build`.
  ({double frames, SnapTarget? caught}) _snappedKeyTravel(_KeyDrag drag) {
    final perFrame = widget.axis.perFrame;
    final raw = perFrame <= 0 ? 0.0 : drag.dxPx / perFrame;
    final base = _grabbedFrame(drag);
    if (!widget.magnet ||
        base == null ||
        perFrame <= 0 ||
        snapSuspended(
            controlPressed: HardwareKeyboard.instance.isControlPressed)) {
      return (frames: raw, caught: null);
    }
    final snapped = snapFrame(
      frame: base + raw,
      targets: widget.snapTargets.where((s) => s.kind != SnapKind.keyframe),
      perFrame: perFrame,
      magnet: true,
    );
    final caught = snapped.caught;
    if (caught == null) return (frames: raw, caught: null);
    return (frames: caught.frame - base, caught: caught);
  }

  /// A key drag's move: the same delta on every selected key — sideways in
  /// frames, and (in the value lens) up or down in the units of whichever
  /// range that key's own curve is drawn against.
  _KeyMove _keyDragMove(_KeyDrag drag, (double, double) range, double height) {
    final dFrames = _snappedKeyTravel(drag).frames;
    return (channel, frame, value) {
      // Per channel, because Normalise gives each curve its own range: the
      // same travel in pixels is a different travel in value on each of them,
      // and it is each curve's own scale the hand was working against.
      final own = range;
      final span = (own.$2 - own.$1).abs() < 1e-12 ? 1.0 : own.$2 - own.$1;
      final dValue = widget.lens == GraphLens.value && height > 0
          ? -drag.dyPx / height * span
          : 0.0;
      var moved = (frame + dFrames).clamp(0.0, widget.frames.toDouble());
      if (widget.magnet) moved = moved.roundToDouble();
      return (moved, value + dValue);
    };
  }

  /// What [move] would write, and the selection it would leave: every selected
  /// key put where the gesture puts it, per channel. Read twice — once per
  /// preview tick and once by the release — so the picture during the drag is
  /// made of exactly the values the commit will write.
  (Map<GraphChannel, BridgeScalar>, Set<String>) _editsFor(_KeyMove move) {
    final edits = <GraphChannel, BridgeScalar>{};
    final newSelection = <String>{};
    for (final channel in widget.channels) {
      final keys = channel.keys;
      final movedIdx = <int>{};
      for (var i = 0; i < keys.length; i++) {
        if (widget.selectedKeys.contains('${channel.id}#$i')) movedIdx.add(i);
      }
      if (movedIdx.isEmpty) continue;

      // Every key with its (possibly moved) frame; a moved key may cross an
      // unmoved one — the list re-sorts, exactly as AE lets keys pass each
      // other — but two keys may not share a frame, which refuses the channel.
      final placed = <(double frame, BridgeKeyframe key, bool moved)>[];
      for (var i = 0; i < keys.length; i++) {
        final base = _keyFrame(keys[i], widget.fps);
        if (!movedIdx.contains(i)) {
          placed.add((base, keys[i], false));
          continue;
        }
        final (frame, value) = move(channel, base, keys[i].value);
        placed.add((
          frame,
          BridgeKeyframe(
            time: timeOfSubframe(frame, widget.fpsNum, widget.fpsDen),
            value: value,
            interpIn: keys[i].interpIn,
            interpOut: keys[i].interpOut,
          ),
          true,
        ));
      }
      placed.sort((a, b) => a.$1.compareTo(b.$1));
      var collides = false;
      for (var i = 0; i + 1 < placed.length; i++) {
        if ((placed[i].$1 - placed[i + 1].$1).abs() < 1e-9) collides = true;
      }
      if (collides) {
        // The gesture stops at the wall: this channel keeps what it had, and
        // its keys stay selected where they were.
        for (final i in movedIdx) {
          newSelection.add('${channel.id}#$i');
        }
        continue;
      }
      edits[channel] = BridgeScalar.keyframed([for (final p in placed) p.$2]);
      for (var i = 0; i < placed.length; i++) {
        if (placed[i].$3) newSelection.add('${channel.id}#$i');
      }
    }
    return (edits, newSelection);
  }

  /// A drag tick: render the values the release will write, without writing
  /// them. Throttled, and coalescing — see [PreviewThrottle].
  void _previewDrag(Map<GraphChannel, BridgeScalar> edits) {
    if (edits.isEmpty) return;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    _preview.request(() => previewChannelEdits(
          comp: widget.comp,
          edits: edits,
          frame: ui.playheadFrame.value,
          scale: ui.viewerScale,
        ));
  }

  /// Where key [index] of [channel] draws, with the drag in flight applied.
  Offset _keyPoint(
      GraphChannel channel, int index, (double, double) range, double height,
      {required bool isOut}) {
    // ONE list for both coordinates. A row drag on a frame with no key shows
    // a curve one key longer than the document's, and reading x from the
    // document while y read the preview drew every diamond past the insertion
    // with one key's x and another's y — glyphs floating off the curve until
    // release (K-336). The same list the glyph loop iterates, so the index can
    // never cross lists.
    final shown = _shownKeys(channel);
    if (index >= shown.length) return Offset.zero;
    final key = shown[index];
    // The gesture's travel is already in [shown] — folded in by [_withMove]
    // so that the handles and the curve move with the glyph rather than after
    // it — so there is nothing to add here.
    var x = widget.axis.xOf(_keyFrame(key, widget.fps));
    var y = _keyY(channel, index, range, height, isOut: isOut);
    // A speed-lens dot in flight: sideways under the pointer, and the side
    // being dragged sits at the speed the pointer is asking for.
    final dot = _handleDrag;
    if (dot != null &&
        dot.dotOnly &&
        dot.channel.id == channel.id &&
        dot.index == index) {
      x += dot.dxPx;
      if (dot.isOut == isOut) {
        y = _yOf(dot.shownSpeed, range, height);
      }
    }
    return Offset(x, y);
  }

  // --- the selection transform box (§6.2) ----------------------------------

  /// Where the box stands: the selected keys' extent on screen, the gesture in
  /// flight already folded in, with the frames its badge counts.
  ///
  /// Null unless there is a block to draw — **two or more** selected keys in
  /// the **value** lens. One key is a key: it has its own drag and its own
  /// readout pill, and a box round it would say "0 f". The speed lens draws a
  /// key as two dots with a speed each, which is a different reading and not a
  /// value extent to scale.
  ({Rect rect, int count, double first, double last})? _boxOf(
      (double, double) range, double height) {
    if (widget.lens != GraphLens.value) return null;
    var left = double.infinity, right = -double.infinity;
    var top = double.infinity, bottom = -double.infinity;
    var first = double.infinity, last = -double.infinity;
    var count = 0;
    for (final channel in widget.channels) {
      final keys = _shownKeys(channel);
      for (var i = 0; i < keys.length; i++) {
        if (!widget.selectedKeys.contains('${channel.id}#$i')) continue;
        final point = _keyPoint(channel, i, range, height, isOut: true);
        final frame = _keyFrame(keys[i], widget.fps);
        count++;
        if (point.dx < left) left = point.dx;
        if (point.dx > right) right = point.dx;
        if (point.dy < top) top = point.dy;
        if (point.dy > bottom) bottom = point.dy;
        if (frame < first) first = frame;
        if (frame > last) last = frame;
      }
    }
    if (!KeyBlock.isBlock(count) || !left.isFinite || !top.isFinite) {
      return null;
    }
    return (
      rect: Rect.fromLTRB(left, top, right, bottom),
      count: count,
      first: first,
      last: last,
    );
  }

  /// The box's scale, as the move it makes to every key it holds.
  ///
  /// **About the opposite edge**: the anchor is the edge the hand is *not*
  /// holding, so a key's distance from it is what gets multiplied — which is
  /// what makes this a scale rather than a slide (docs/07 §5.3, Caddis §2.1).
  /// Time is scaled in frames; value is scaled in **pixels**, so that under
  /// Normalise — where every curve has its own range — the whole selection
  /// still scales by the one amount the hand asked for.
  _KeyMove _boxMove(_BoxDrag box, (double, double) range, double height) {
    final perFrame = widget.axis.perFrame;
    final round = _BoxDrag.shiftHeld;
    if (box.time) {
      // Clamped exactly as a lane stretch is: a box pulled through its own
      // anchor would turn the selection back to front in time, which is
      // Reverse's job and not a scale's.
      final to = clampStretch(
        anchor: box.anchor,
        from: box.from,
        to: box.from + (perFrame <= 0 ? 0 : box.travel / perFrame),
      );
      return (channel, frame, value) {
        var f =
            scaledAbout(anchor: box.anchor, from: box.from, to: to, at: frame);
        if (widget.magnet || round) f = f.roundToDouble();
        return (f.clamp(0.0, widget.frames.toDouble()), value);
      };
    }
    final to = box.from + box.travel;
    return (channel, frame, value) {
      if (height <= 0) return (frame, value);
      // Value is scaled in **pixels**, so that under Normalise — where every
      // curve is drawn against its own range — the whole selection still
      // scales by the one amount the hand asked for.
      final own = range;
      var v = _valueAt(
        scaledAbout(
            anchor: box.anchor,
            from: box.from,
            to: to,
            at: _yOf(value, own, height)),
        own,
        height,
      );
      // `Shift` snaps what the scale lands on to whole numbers — the Caddis
      // behaviour the readout pill reports live.
      if (round) v = v.roundToDouble();
      return (frame, v);
    };
  }

  void _startBoxDrag(
    ({Rect rect, int count, double first, double last}) box, {
    required bool time,
    required bool start,
  }) {
    setState(() {
      _boxDrag = _BoxDrag(
        time: time,
        start: start,
        // The value axis runs the other way on screen: the *top* of the box is
        // the largest value, so a value edge and its anchor are read off the
        // rectangle rather than off the values.
        anchor: time
            ? (start ? box.last : box.first)
            : (start ? box.rect.bottom : box.rect.top),
        from: time
            ? (start ? box.first : box.last)
            : (start ? box.rect.top : box.rect.bottom),
      );
      _frozen = _range();
    });
    _escape.begin(_abandonDrag);
  }

  void _commitBoxDrag((double, double) range, double height) {
    final box = _boxDrag;
    final commit = _escape.end();
    setState(() {
      _boxDrag = null;
      _frozen = null;
    });
    if (!commit || box == null || box.travel == 0) return;
    // The commit is the last word on this gesture: a preview tick still held
    // would put the provisional picture back on top of it.
    _preview.cancel();
    final (edits, newSelection) = _editsFor(_boxMove(box, range, height));
    if (edits.isEmpty) return;
    commitChannelEdits(edits);
    widget.selectedKeys
      ..clear()
      ..addAll(newSelection);
    widget.onSelectionChanged();
    widget.onChanged();
  }

  // --- handle drags --------------------------------------------------------

  /// The neighbour a side's handle reaches toward, or null at the ends.
  BridgeKeyframe? _neighbour(
          List<BridgeKeyframe> keys, int index, bool isOut) =>
      isOut
          ? (index + 1 < keys.length ? keys[index + 1] : null)
          : (index > 0 ? keys[index - 1] : null);

  /// Seconds ↔ pixels on the time axis, so handle geometry can be worked out
  /// where the user actually sees it: on screen.
  double _xOfSeconds(double t) => widget.axis.xOf(t * widget.fps);
  double _secondsOfX(double x) =>
      widget.axis.frameAtExact(x) / (widget.fps <= 0 ? 1 : widget.fps);

  /// A side's handle endpoint in (seconds, value) whatever its interpolation —
  /// a bezier side's own, or where a linear side's *would* be. Used to measure
  /// the partner's on-screen length before a drag starts.
  ({double time, double value}) _sideEndpoint(
      List<BridgeKeyframe> keys, int index, bool isOut) {
    final key = keys[index];
    final nb = _neighbour(keys, index, isOut);
    final side = isOut ? key.interpOut : key.interpIn;
    return handleEndpoint(
      keyTime: rationalSeconds(key.time),
      keyValue: key.value,
      neighbourTime:
          nb == null ? rationalSeconds(key.time) : rationalSeconds(nb.time),
      isOut: isOut,
      speed: sideSpeedAtKey(keys, index, isOut: isOut),
      influence: sideInfluence(side),
    );
  }

  void _startHandleDrag(GraphChannel channel, int index, bool isOut,
      bool dotOnly, (double, double) range, double height) {
    final keys = channel.keys;
    final key = keys[index];
    final side = isOut ? key.interpOut : key.interpIn;
    final other = isOut ? key.interpIn : key.interpOut;
    // Joined when both sides have a tangent and it moves at the same speed;
    // `Alt` held as the drag begins flips it — break them apart, or join them
    // back. A side with no span on the other flank has nothing to join to.
    // A pair of automatic sides is joined by construction: the neighbours give
    // both of them the same aim.
    bool eased(BridgeSideInterp s) =>
        s is BridgeSideInterp_Bezier || s is BridgeSideInterp_Auto;
    final joined = eased(side) &&
        eased(other) &&
        (sideSpeedAtKey(keys, index, isOut: isOut) -
                    sideSpeedAtKey(keys, index, isOut: !isOut))
                .abs() <
            1e-9;
    final alt = altActuallyHeld();
    final hasOther = _neighbour(keys, index, !isOut) != null;
    final speed = sideSpeedAtKey(keys, index, isOut: isOut);

    setState(() {
      _handleDrag = _HandleDrag(
        channel: channel,
        index: index,
        isOut: isOut,
        mirrored: hasOther && (alt ? !joined : joined),
        speed: speed,
        influence: sideInfluence(side),
        partnerSpeed: sideSpeedAtKey(keys, index, isOut: !isOut),
        partnerInfluence: sideInfluence(other),
        partnerLenPx: _handleLength(channel, index, !isOut, range, height),
        range: range,
        height: height,
        dotOnly: dotOnly,
      );
      _frozen = _range();
    });
    _escape.begin(_abandonDrag);
  }

  void _updateHandleDrag(Offset local, (double, double) range, double height,
      {double dx = 0, double dy = 0}) {
    final drag = _handleDrag;
    if (drag == null) return;
    final keys = drag.channel.keys;
    final key = keys[drag.index];
    final nb = _neighbour(keys, drag.index, drag.isOut);
    final keyTime = rationalSeconds(key.time);
    final pointerTime = _secondsOfX(local.dx);
    final pointerValue = _valueAt(local.dy, range, height);

    setState(() {
      if (isEnvelope(drag.channel)) {
        // An envelope point: its height is the playback speed in per cent,
        // straight off the axis, and it carries the key sideways like any
        // speed dot. There is no influence to set — an envelope's sides are
        // always the chord (K-247), which is what keeps its lines straight.
        drag.speed = pointerValue;
        if (drag.dotOnly) {
          drag.rawDx += dx;
          drag.rawDy += dy;
        }
        return;
      }
      if (widget.lens == GraphLens.speed) {
        // The speed lens: a dot's height IS that side's speed and its sideways
        // travel moves the keyframe in time; an influence handle's reach sets
        // the influence.
        drag.speed = pointerValue;
        if (drag.dotOnly) {
          drag.rawDx += dx;
          drag.rawDy += dy;
        } else if (nb != null) {
          final dt = (rationalSeconds(nb.time) - keyTime).abs();
          if (dt > 1e-9) {
            drag.influence =
                ((drag.isOut ? pointerTime - keyTime : keyTime - pointerTime) /
                        dt)
                    .clamp(1e-3, 1.0)
                    .toDouble();
          }
        }
        return;
      }

      if (nb == null) return;
      // `Shift` lays the handle flat (K-333): the value is held at the key's
      // own, so the tangent leaves it horizontally — the ease-out-to-nothing
      // every editor spells this way. A joined partner is mirrored from the
      // dragged side, so it comes flat with it and the pair reads as one
      // straight line through the key.
      final flat = HardwareKeyboard.instance.isShiftPressed;
      final r = handleFromDrag(
        keyTime: keyTime,
        keyValue: key.value,
        neighbourTime: rationalSeconds(nb.time),
        isOut: drag.isOut,
        dragTime: pointerTime,
        dragValue: flat ? key.value : pointerValue,
      );
      drag.speed = r.speed;
      drag.influence = r.influence;
      _mirrorPartner(drag, key, r.speed, r.influence, range, height);
    });
    // The shaped curve, exactly as the release will commit it (K-192): an ease
    // or an envelope point changes which source moment every frame between two
    // keys reads, so it is as much a picture edit as moving the key itself.
    //
    // **Only where the picture can actually differ** (K-529, owner: a handle
    // drag cost calls by the hundred per second wherever it was made). An ease
    // changes the values *between* two keys and nothing outside them, so with
    // the playhead outside that span every one of those renders came back with
    // the frame already on screen. A key drag is not like this — it moves the
    // key, so the span it changes moves with the pointer — which is why the
    // guard sits here rather than inside [_previewDrag].
    if (!_handleDragShowsAtPlayhead(drag)) return;
    _previewDrag(
        {drag.channel: BridgeScalar.keyframed(_shownKeys(drag.channel))});
  }

  /// Whether the frame on screen is one this handle drag can change: the span
  /// between the dragged key and its neighbour, widened to the partner's span
  /// when the two sides are joined and so swing together.
  ///
  /// An envelope point is the exception and always answers true: dragging one
  /// re-integrates every frame after it (K-247), so the change is not bounded
  /// by a span at all.
  bool _handleDragShowsAtPlayhead(_HandleDrag drag) {
    if (isEnvelope(drag.channel) || drag.dotOnly) return true;
    final keys = drag.channel.keys;
    if (drag.index < 0 || drag.index >= keys.length) return true;
    final key = keys[drag.index];
    var lo = rationalSeconds(key.time);
    var hi = lo;
    for (final isOut in drag.mirrored ? const [true, false] : [drag.isOut]) {
      final nb = _neighbour(keys, drag.index, isOut);
      if (nb == null) continue;
      final at = rationalSeconds(nb.time);
      if (at < lo) lo = at;
      if (at > hi) hi = at;
    }
    if (hi <= lo) return false;
    final fps = widget.fps <= 0 ? 1.0 : widget.fps;
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame.value /
            fps;
    return playhead >= lo && playhead <= hi;
  }

  /// Swing the joined partner opposite the dragged handle, keeping the pixel
  /// length it had when the gesture began.
  ///
  /// The whole calculation is in **screen** space: the two handles read as one
  /// straight line through the key, and staying straight means being opposite
  /// *as drawn* — the value axis and the time axis have different units and
  /// their own zooms, so mirroring in value space would bend the line and
  /// stretch the partner as the pair swings toward vertical.
  void _mirrorPartner(_HandleDrag drag, BridgeKeyframe key, double speed,
      double influence, (double, double) range, double height) {
    if (!drag.mirrored) return;
    final keys = drag.channel.keys;
    final partnerNb = _neighbour(keys, drag.index, !drag.isOut);
    if (partnerNb == null) return;
    final keyTime = rationalSeconds(key.time);
    final nb = _neighbour(keys, drag.index, drag.isOut);
    if (nb == null) return;

    // Where the dragged handle actually ended up (its reach is clamped inside
    // the span), so the partner is opposite what is drawn, not opposite the
    // raw pointer.
    final e = handleEndpoint(
      keyTime: keyTime,
      keyValue: key.value,
      neighbourTime: rationalSeconds(nb.time),
      isOut: drag.isOut,
      speed: speed,
      influence: influence,
    );
    final keyPx = Offset(_xOfSeconds(keyTime), _yOf(key.value, range, height));
    final direction =
        Offset(_xOfSeconds(e.time), _yOf(e.value, range, height)) - keyPx;
    final length = direction.distance;
    if (length < 1e-6) return;
    // The partner keeps the pixel length it began the gesture with, whatever
    // the pair's angle: what the eye reads is length on screen, and a handle
    // that grew as the tangent swung would be the thing this exists to avoid.
    final partnerPx = keyPx - direction / length * drag.partnerLenPx;

    final pr = handleFromDrag(
      keyTime: keyTime,
      keyValue: key.value,
      neighbourTime: rationalSeconds(partnerNb.time),
      isOut: !drag.isOut,
      dragTime: _secondsOfX(partnerPx.dx),
      dragValue: _valueAt(partnerPx.dy, range, height),
    );
    drag.partnerSpeed = pr.speed;
    drag.partnerInfluence = pr.influence;
  }

  void _commitHandleDrag() {
    final drag = _handleDrag;
    final commit = _escape.end();
    setState(() {
      _handleDrag = null;
      _frozen = null;
    });
    if (!commit || drag == null) return;
    // As in [_commitKeyDrag]: the write is the last word, so no held preview
    // tick may land after it.
    _preview.cancel();
    final shown = _keysWithHandleDrag(drag, drag.channel.keys);

    // Both sides keep the length they were left at: the dragged one wherever
    // the pointer put it, the partner exactly the length it started with. Held
    // against the scale they were drawn under, so a later zoom re-measures
    // rather than shrinking them.
    if (widget.lens == GraphLens.value && !drag.dotOnly) {
      final key = shown[drag.index];
      final keyPx = Offset(_xOfSeconds(rationalSeconds(key.time)),
          _yOf(key.value, drag.range, drag.height));
      final end = _sideEndpoint(shown, drag.index, drag.isOut);
      _rememberLength(
        drag.channel,
        key,
        drag.isOut,
        (Offset(_xOfSeconds(end.time),
                    _yOf(end.value, drag.range, drag.height)) -
                keyPx)
            .distance,
        drag.range,
        drag.height,
      );
      if (drag.mirrored) {
        _rememberLength(drag.channel, key, !drag.isOut, drag.partnerLenPx,
            drag.range, drag.height);
      }
    }

    // A speed-lens dot also carries the key sideways: commit the move with the
    // same rules a key drag follows — whole frames with the magnet on, and no
    // two keys sharing a frame.
    if (drag.dotOnly && drag.dxPx != 0) {
      final moved = _keysWithDotTimeMove(drag, shown);
      if (moved == null) {
        // The move collided; the easing still stands where the key already is.
        commitChannelEdits({drag.channel: BridgeScalar.keyframed(shown)});
        widget.onChanged();
        return;
      }
      commitChannelEdits({drag.channel: BridgeScalar.keyframed(moved)});
      widget.onChanged();
      return;
    }
    commitChannelEdits({drag.channel: BridgeScalar.keyframed(shown)});
    widget.onChanged();
  }

  /// [keys] with a speed-lens dot's sideways travel applied to its keyframe,
  /// or null when the move would land on a neighbour.
  List<BridgeKeyframe>? _keysWithDotTimeMove(
      _HandleDrag drag, List<BridgeKeyframe> keys) {
    final perFrame = widget.axis.perFrame;
    if (perFrame <= 0) return null;
    final base = _keyFrame(keys[drag.index], widget.fps);
    var frame =
        (base + drag.dxPx / perFrame).clamp(0.0, widget.frames.toDouble());
    if (widget.magnet) frame = frame.roundToDouble();
    if ((frame - base).abs() < 1e-9) return null;
    for (var i = 0; i < keys.length; i++) {
      if (i == drag.index) continue;
      if ((_keyFrame(keys[i], widget.fps) - frame).abs() < 1e-9) return null;
    }
    final at = timeOfSubframe(frame, widget.fpsNum, widget.fpsDen);
    if (isEnvelope(drag.channel)) {
      return moveEnvelopePoint(keys, drag.index, at);
    }
    // An **envelope** point keeps its speed and re-integrates. A key's stored
    // tangent is a speed; its span's chord is an average. Move a key in time
    // and the chord changes while the tangent stays put, so a span that was
    // straight stops being straight and the graph starts describing playback
    // the points do not say. Everywhere else a key keeps its value and its
    // easing, which is what moving a keyframe has always meant.
    if (isEnvelope(drag.channel)) {
      return moveEnvelopePoint(keys, drag.index, at);
    }
    final moved = [
      for (var i = 0; i < keys.length; i++)
        if (i == drag.index)
          BridgeKeyframe(
            time: at,
            value: keys[i].value,
            interpIn: keys[i].interpIn,
            interpOut: keys[i].interpOut,
          )
        else
          keys[i],
    ]..sort(
        (a, b) => rationalSeconds(a.time).compareTo(rationalSeconds(b.time)));
    return moved;
  }

  /// [keys] with the drag's provisional easing written into its key — both
  /// sides when they are joined, so the curve, the handle and its partner all
  /// move together while the pointer is down.
  List<BridgeKeyframe> _keysWithHandleDrag(
      _HandleDrag drag, List<BridgeKeyframe> keys) {
    // An envelope point sets a speed and the source positions after it follow
    // (K-247). Every keyframe *time* stays exactly put, so a beat already
    // synced stays synced — the covenant this whole feature is built around.
    if (isEnvelope(drag.channel)) {
      return setEnvelopeSpeed(keys, drag.index, drag.speed);
    }
    final dragged = BridgeSideInterp.bezier(
        BridgeBezierSide(speed: drag.speed, influence: drag.influence));
    final partner = drag.mirrored
        ? BridgeSideInterp.bezier(BridgeBezierSide(
            speed: drag.partnerSpeed, influence: drag.partnerInfluence))
        : null;
    return [
      for (var i = 0; i < keys.length; i++)
        if (i == drag.index)
          BridgeKeyframe(
            time: keys[i].time,
            value: keys[i].value,
            interpIn: drag.isOut ? (partner ?? keys[i].interpIn) : dragged,
            interpOut: drag.isOut ? dragged : (partner ?? keys[i].interpOut),
          )
        else
          keys[i],
    ];
  }

  /// [keys] with the row drag's value written into the key at its frame —
  /// matched to the **nearest half frame**, never by float equality (K-336).
  ///
  /// `_withKeyAt` merged by exact double frame, and a key's frame comes back
  /// through rational-to-float maths: frame 50 at 60 fps reads 49.999…, which
  /// is not 50.0, so the drag's key was *inserted beside* the document's
  /// instead of replacing it. One extra key shifts every later index, and the
  /// glyphs read position by index — the dragged key drew at the next key's
  /// place and everything after it sat one key off, until the release rebuilt
  /// from the document and it all snapped back. Same length in, same length
  /// out (or +1 when the playhead truly has no key), so the glyph indexes hold.
  List<BridgeKeyframe> _keysWithRowDrag(
      List<BridgeKeyframe> keys, RowValueDrag row) {
    final out = <BridgeKeyframe>[];
    var replaced = false;
    for (final k in keys) {
      if (!replaced && (_keyFrame(k, widget.fps) - row.frame).abs() < 0.5) {
        out.add(BridgeKeyframe(
          time: k.time,
          value: row.value,
          interpIn: k.interpIn,
          interpOut: k.interpOut,
        ));
        replaced = true;
      } else {
        out.add(k);
      }
    }
    if (replaced) return out;
    // The playhead genuinely sits between keys and the plant has not landed
    // yet: show the key the release will write, in order.
    final planted = BridgeKeyframe(
      time: timeOfSubframe(row.frame.toDouble(), widget.fpsNum, widget.fpsDen),
      value: row.value,
      interpIn: const BridgeSideInterp.linear(),
      interpOut: const BridgeSideInterp.linear(),
    );
    final at = out.indexWhere((k) => _keyFrame(k, widget.fps) > row.frame);
    if (at < 0) {
      out.add(planted);
    } else {
      out.insert(at, planted);
    }
    return out;
  }

  /// The keys as every reader should see them — the drag in flight folded in,
  /// so the curve, the glyphs, the tangent endpoints and the lines between
  /// them all derive from **one** moved list.
  ///
  /// `painting` is what tells a *row* drag's provisional value from the keys
  /// that really exist: the curve is drawn through it, the diamonds are not, so
  /// dragging an unkeyed value moves the line without appearing to key it.
  List<BridgeKeyframe> _shownKeys(GraphChannel channel,
          {bool painting = false}) =>
      _withMove(channel, _keysWithSideDrags(channel, painting: painting));

  /// The move a gesture in flight is making, or null when none is — the key
  /// drag's delta, or the transform box's scale.
  ///
  /// Only one of the two ever runs: a box handle is a different target from a
  /// key glyph, and a pointer is in one place.
  _KeyMove? _activeMove() {
    final range = _range();
    final height = _paneSize.height;
    final box = _boxDrag;
    if (box != null) return _boxMove(box, range, height);
    final drag = _keyDrag;
    if (drag == null || (drag.dxPx == 0 && drag.dyPx == 0)) return null;
    return _keyDragMove(drag, range, height);
  }

  /// The gesture in flight, applied to the keys it holds.
  ///
  /// **Why it lives here rather than in [_keyPoint].** The delta used to be
  /// added to the glyph's coordinates alone, so a dragged key moved while
  /// [_handleEndpointFor] and [_HandlesPainter] went on reading the document's
  /// unmoved key: the handle line stretched from the travelling glyph to a
  /// stranded dot, and the dot never moved at all. Folding the same move into
  /// the shown keys moves the key *and* everything measured from it — the
  /// curve, the handles, and the transform box's own edges — because they are
  /// all measured from this one list.
  ///
  /// The order is left alone. A key dragged past its neighbour keeps its index
  /// for the length of the gesture — the ids in the selection, the glyph loop
  /// and the handles are all indexed into this list, and re-sorting mid-drag
  /// would re-aim them at each other's keys. The release sorts (and refuses a
  /// collision) in [_editsFor].
  List<BridgeKeyframe> _withMove(
      GraphChannel channel, List<BridgeKeyframe> keys) {
    final move = _activeMove();
    if (move == null) return keys;
    List<BridgeKeyframe>? moved;
    for (var i = 0; i < keys.length; i++) {
      if (!widget.selectedKeys.contains('${channel.id}#$i')) continue;
      moved ??= [...keys];
      final key = keys[i];
      // The commit's own arithmetic, so with the magnet on the key draws on
      // the whole frame it will land on (K-333) and a key taken off either end
      // of the composition draws where it will actually come to rest.
      final (frame, value) =
          move(channel, _keyFrame(key, widget.fps), key.value);
      moved[i] = BridgeKeyframe(
        time: timeOfSubframe(frame, widget.fpsNum, widget.fpsDen),
        value: value,
        interpIn: key.interpIn,
        interpOut: key.interpOut,
      );
    }
    return moved ?? keys;
  }

  /// The keys with the *other* drags' provisional geometry swapped in: a row
  /// drag's published value, and a handle drag's shaped sides.
  List<BridgeKeyframe> _keysWithSideDrags(GraphChannel channel,
      {bool painting = false}) {
    final row = rowValueDrag.value;
    if (row != null && row.matches(channel)) {
      final keys = channel.keys;
      if (keys.isNotEmpty) {
        return _keysWithRowDrag(keys, row);
      }
      // Not keyed: one point carries the whole flat line to its new height, and
      // only for the drawing.
      if (painting) {
        return _withKeyAt(const [], row.frame.toDouble(), row.value, widget.fps,
            widget.fpsNum, widget.fpsDen);
      }
    }
    final drag = _handleDrag;
    if (drag == null || drag.channel.id != channel.id) return channel.keys;
    final shaped = _keysWithHandleDrag(drag, channel.keys);
    // A dot carries its keyframe sideways as well as up, and the curve has to
    // go with it: the dot is drawn at the pointer either way, so a curve left
    // at the committed time simply comes adrift from the dot until the gesture
    // ends. The same move the release will commit, applied to the preview.
    if (!drag.dotOnly || drag.dxPx == 0) return shaped;
    return _keysWithDotTimeMove(drag, shaped) ?? shaped;
  }

  // --- numeric entry (docs/07 §5.3) ----------------------------------------

  /// Double-clicking a key opens its exact frame, value and influences as
  /// fields (§6.2's numeric entry; the Key readout row of §3.3 is the same
  /// numbers, kept in the outline for the single-selection case).
  Future<void> _openKeyFields(
      GraphChannel channel, int index, Offset position) async {
    final keys = channel.keys;
    if (index < 0 || index >= keys.length) return;
    final key = keys[index];
    final frame = _keyFrame(key, widget.fps);
    // The frame field is bounded by the key's neighbours: a typed time that
    // crossed one would re-sort the curve under the popover, which is holding
    // an index into it. Everything else about crossing keys stays the drag's
    // business, where the list is rebuilt on release.
    final gap = widget.magnet ? 1.0 : 0.01;
    var lower = index > 0 ? _keyFrame(keys[index - 1], widget.fps) + gap : 0.0;
    var upper = index + 1 < keys.length
        ? _keyFrame(keys[index + 1], widget.fps) - gap
        : widget.frames.toDouble();
    if (upper < lower) {
      lower = frame;
      upper = frame;
    }
    await showKeyFieldsPopover(
      context: context,
      position: position,
      frame: frame,
      value: key.value,
      inPercent: (sideInfluence(key.interpIn) * 100).roundToDouble(),
      outPercent: (sideInfluence(key.interpOut) * 100).roundToDouble(),
      minFrame: lower,
      maxFrame: upper,
      onApply: (frame, value, inPercent, outPercent) => _applyKeyFields(
        channel.id,
        index,
        frame: frame,
        value: value,
        inPercent: inPercent,
        outPercent: outPercent,
      ),
    );
  }

  /// Write what the numeric fields hold onto one key.
  ///
  /// The channel is looked up **by id at the moment of the write**, never held
  /// by the popover: a channel is a snapshot of the read model, and a box that
  /// kept the one it opened with would write its second edit on top of a curve
  /// that no longer exists.
  void _applyKeyFields(
    String channelId,
    int index, {
    required double frame,
    required double value,
    required int inPercent,
    required int outPercent,
  }) {
    for (final channel in widget.channels) {
      if (channel.id != channelId) continue;
      final keys = channel.keys;
      if (index < 0 || index >= keys.length) return;
      final key = keys[index];
      final f = frame.clamp(0.0, widget.frames.toDouble());
      for (var i = 0; i < keys.length; i++) {
        if (i != index && (_keyFrame(keys[i], widget.fps) - f).abs() < 1e-9) {
          return;
        }
      }
      // A side is rewritten only when its own number changed: giving a side an
      // influence turns a linear one into a bezier at its current speed, and
      // typing a *frame* must not quietly do that to both sides of the key.
      var interpIn = key.interpIn;
      var interpOut = key.interpOut;
      if ((sideInfluence(key.interpIn) * 100).round() != inPercent) {
        interpIn = sideWithInfluence(keys, index, false, inPercent.toDouble());
      }
      if ((sideInfluence(key.interpOut) * 100).round() != outPercent) {
        interpOut = sideWithInfluence(keys, index, true, outPercent.toDouble());
      }
      commitChannelEdits({
        channel: BridgeScalar.keyframed([
          for (var i = 0; i < keys.length; i++)
            if (i == index)
              BridgeKeyframe(
                time: timeOfSubframe(f, widget.fpsNum, widget.fpsDen),
                value: value,
                interpIn: interpIn,
                interpOut: interpOut,
              )
            else
              keys[i],
        ]),
      });
      widget.onChanged();
      return;
    }
  }

  // --- menus ---------------------------------------------------------------

  Future<void> _showKeyMenu(
      GraphChannel channel, int index, Offset position) async {
    final picked = await showLumitPopup<String>(
      context: context,
      position: position,
      builder: (close) => FloatSurface(
        width: 170,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            MenuRow(
                onPressed: () => close('linear'), child: Text(l10n.easeLinear)),
            MenuRow(onPressed: () => close('ease'), child: Text(l10n.easeEasy)),
            MenuRow(onPressed: () => close('hold'), child: Text(l10n.easeHold)),
            MenuRow(
                onPressed: () => close('delete'), child: Text(l10n.deleteKey)),
          ],
        ),
      ),
    );
    if (picked == null) return;
    final id = '${channel.id}#$index';
    if (picked == 'delete') {
      _deleteSelection(fallback: id);
      return;
    }
    final targets =
        widget.selectedKeys.contains(id) ? widget.selectedKeys : {id};
    applyInterpToSelection(
      channels: widget.channels,
      selectedKeys: targets,
      side: switch (picked) {
        'linear' => const BridgeSideInterp.linear(),
        'hold' => const BridgeSideInterp.hold(),
        _ => easyEase,
      },
    );
    widget.onChanged();
  }

  /// Delete the selected keys (or [fallback] when none) — the last key of a
  /// curve leaves a static value holding what it held.
  void _deleteSelection({String? fallback}) {
    final targets = widget.selectedKeys.isNotEmpty
        ? widget.selectedKeys
        : {if (fallback != null) fallback};
    final wrote = deleteKeysFromChannels(
        channels: widget.channels, selectedKeys: targets);
    if (!wrote) return;
    widget.selectedKeys.clear();
    widget.onSelectionChanged();
    widget.onChanged();
  }

  // --- build ---------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;

    // An empty graph is still a graph. It used to be replaced outright by a
    // line of text, which took the wheel handler, the grid, the value axis and
    // the horizontal scrollbar with it: with nothing selected you could not
    // Ctrl-scroll to zoom, could not pan, and had no axis to read — the pane
    // only became a pane once it had something in it. The empty range is a
    // real range (`fitValueRange` answers 0..1 for no data), so everything
    // below works with no channels; the message is drawn over the top instead.
    return LayoutBuilder(
      builder: (context, constraints) {
        final height = constraints.maxHeight;
        final range = _range();
        _lastRange = range;
        _paneSize = Size(constraints.maxWidth, height);

        return Listener(
          // Claimed through the resolver, not handled outright: the pane sits
          // inside the Timeline's horizontal scroll view, which registers for
          // the same wheel event and would *also* act on it — scrolling the
          // curves sideways while this handler zoomed or panned them. The
          // resolver gives one event to exactly one handler, and the innermost
          // registrant (this one) wins.
          onPointerSignal: (event) {
            if (event is! PointerScrollEvent) return;
            GestureBinding.instance.pointerSignalResolver.register(event,
                (resolved) {
              if (resolved is PointerScrollEvent) _wheel(resolved);
            });
          },
          child: Stack(
            clipBehavior: Clip.hardEdge,
            children: [
              // Repainted as the Timeline scrolls, not merely as it rebuilds:
              // scrolling moves this pane without rebuilding it, and the value
              // labels are pinned to the viewport, so they have to be redrawn
              // at the new edge. The listener is the scroll controller the
              // Timeline already owns.
              Positioned.fill(
                child: AnimatedBuilder(
                  animation: widget.hScroll ?? const AlwaysStoppedAnimation(0),
                  // Clipped to the pane. The vertical range is frozen while a
                  // drag runs, so a key taken past the top or the bottom would
                  // otherwise be drawn outside the graph and over whatever
                  // sits beside it; the framing catches up when the drag ends.
                  builder: (context, _) => ClipRect(
                    child: CustomPaint(
                      painter: _GraphPainter(
                        channels: widget.channels,
                        shownKeys: [
                          for (final c in widget.channels)
                            _shownKeys(c, painting: true)
                        ],
                        lens: widget.lens,
                        axis: widget.axis,
                        fps: widget.fps,
                        range: range,
                        palette: t.curve,
                        comp: Provider.of<LumitUiState>(context, listen: false)
                            .model,
                        grid: t.hairline,
                        // The axis numbers are numbers, so they are set in the
                        // mono face at the mockup's own 8 — Hanken at 10 read
                        // as body text ruling the graph.
                        label: t.mono.copyWith(fontSize: 8, color: t.textMuted),
                        viewportLeft: _viewportLeft,
                        viewportWidth: _viewportWidth(constraints.maxWidth),
                        gutterFill: t.surface0.withValues(alpha: 0.85),
                        vegas: widget.vegas,
                      ),
                    ),
                  ),
                ),
              ),
              Positioned.fill(
                child: MarqueeSelect(
                  key: const ValueKey('graph-marquee'),
                  onSelect: (rect, additive) =>
                      _applyMarquee(rect, range, height, additive),
                  onTapAt: (local) => _tapPane(local, range, height),
                  onClear: () {},
                ),
              ),
              // The selection transform box (§6.2). Above the marquee, so its
              // edges take their own drags; **below** the keys and handles,
              // so a key sitting at an extreme of the box keeps every gesture
              // it answers everywhere else (P5).
              ..._transformBox(t, range, height, constraints.maxWidth),
              // The tangent handles (or speed influence handles), above the
              // marquee so they win their own gestures.
              Positioned.fill(
                child: IgnorePointer(
                  child: CustomPaint(
                    key: const ValueKey<String>('graph-handle-lines'),
                    painter: _HandlesPainter(
                      state: this,
                      range: range,
                      height: height,
                      // `text_primary`, the colour of everything selected —
                      // never the accent, and never `warning`, which has no
                      // job on this pane (K-439, §6.1).
                      colour: t.textPrimary,
                    ),
                  ),
                ),
              ),
              // Which of the two wins where they overlap depends on the lens,
              // and they *do* overlap: a handle's reach is a fraction of the
              // gap to the next key, so on a long composition it sits within
              // a few pixels of its own key.
              //
              // Value lens: the handle is on top. It is the finer gesture, the
              // key is grabbable everywhere else along the curve, and a miss
              // that drops the selection also takes the handles away.
              // Speed lens: the dot is on top — it is the keyframe itself, and
              // its influence bar runs out sideways from underneath it.
              if (widget.lens == GraphLens.value) ...[
                ..._keyHandles(t, range, height),
                ..._tangentHandles(range, height),
              ] else ...[
                ..._tangentHandles(range, height),
                ..._keyHandles(t, range, height),
              ],
              // What a key drag has landed on, marked while it holds it —
              // the same hairline the lanes and the ruler draw (docs/07 §4.5).
              ..._snapCapture(t),
              // The live readout, beside the key in hand (§6.2).
              ..._valueHint(range, height, constraints.maxWidth),
              // Over the live pane rather than instead of it, so the grid and
              // the axis stay readable behind the invitation to fill them.
              if (widget.channels.isEmpty)
                Positioned.fill(
                  child: IgnorePointer(
                    child: Center(
                      child: Text(
                        l10n.graphEditorEmpty,
                        style: t.small,
                        textAlign: TextAlign.center,
                      ),
                    ),
                  ),
                ),
            ],
          ),
        );
      },
    );
  }

  /// The **selection transform box** (§6.2, docs/07 §5.3): the same hairline
  /// box the lanes draw round a block of keys, spanning the selection in time
  /// *and* value, with a grab on each of its four edges.
  ///
  /// An edge scales one axis about the opposite edge — left and right scale
  /// time, top and bottom scale value. `Shift` snaps what the scale lands on
  /// to whole frames and whole numbers, the readout pill saying live what
  /// those are. One undo step, and `Escape` puts it all back (P3).
  ///
  /// **No corner grabs.** The box's corners stand exactly on the selection's
  /// extreme keys — with two keys selected they *are* those keys — so a corner
  /// grab would either swallow the key's own drag or sit unreachable beneath
  /// it (P5). The two axes are scaled in two gestures instead, which is the
  /// same arithmetic and costs one extra drag.
  List<Widget> _transformBox(
      LumitTheme t, (double, double) range, double height, double paneWidth) {
    final box = _boxOf(range, height);
    if (box == null) return const [];
    final r = box.rect;
    final live = _boxDrag;
    return [
      // The box itself takes no pointer: it covers the keys it holds, and one
      // that ate their clicks would make a selected key the one key that
      // cannot be picked up again. Its edges are the separate grabs below.
      Positioned(
        key: const ValueKey<String>('graph-transform-box'),
        left: r.left,
        top: r.top,
        width: r.width,
        height: r.height,
        child: IgnorePointer(
          child: DecoratedBox(
            decoration: BoxDecoration(border: Border.all(color: t.textPrimary)),
          ),
        ),
      ),
      _boxHandle(box,
          name: 'graph-box-left',
          time: true,
          start: true,
          range: range,
          height: height),
      _boxHandle(box,
          name: 'graph-box-right',
          time: true,
          start: false,
          range: range,
          height: height),
      _boxHandle(box,
          name: 'graph-box-top',
          time: false,
          start: true,
          range: range,
          height: height),
      _boxHandle(box,
          name: 'graph-box-bottom',
          time: false,
          start: false,
          range: range,
          height: height),
      // The block's badge, in the lanes' own words: how many keys it holds and
      // how many frames it spans. It reads the box as it stands, so a scale in
      // flight reports the span the release will write.
      Positioned(
        key: const ValueKey<String>('graph-box-badge'),
        left: r.right + 6,
        top: r.top - 1,
        child: IgnorePointer(
          child: Container(
            padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
            decoration: BoxDecoration(
              color: t.surface4,
              borderRadius: BorderRadius.circular(2),
            ),
            child: Text(
              l10n.keyBlockBadge(
                  box.count,
                  KeyBlock(first: box.first, last: box.last, count: box.count)
                      .spanFrames),
              style: t.mono.copyWith(fontSize: 8, color: t.textPrimary),
            ),
          ),
        ),
      ),
      // The live readout, under the hand while the scale runs and gone on
      // release (P1). It reads the frames the box now spans and the values its
      // top and bottom now stand at — against the pane's own value axis, which
      // is the axis the gutter's numbers label.
      if (live != null)
        Positioned(
          key: const ValueKey<String>('graph-box-hint'),
          left: r.left + 8 + 132 > paneWidth ? r.left - 8 - 132 : r.left + 8,
          top: r.bottom + 4,
          child: IgnorePointer(
            child: HintPill(
              text: l10n.graphBoxHint(
                box.first.round(),
                box.last.round(),
                graphNumberText(_valueAt(r.bottom, range, height)),
                graphNumberText(_valueAt(r.top, range, height)),
              ),
            ),
          ),
        ),
    ];
  }

  /// One edge of the box, as a strip wide enough to aim at.
  Widget _boxHandle(
    ({Rect rect, int count, double first, double last}) box, {
    required String name,
    required bool time,
    required bool start,
    required (double, double) range,
    required double height,
  }) {
    final r = box.rect;
    // The strip's length never falls below a grab's own width: a selection
    // whose keys all hold the same value has a box with no height, and an edge
    // nobody can put a pointer on is an edge that is not there (P5).
    final w = time ? _boxGrab : (r.width < _boxGrab ? _boxGrab : r.width);
    final h = time ? (r.height < _boxGrab ? _boxGrab : r.height) : _boxGrab;
    return Positioned(
      key: ValueKey<String>(name),
      left: time
          ? (start ? r.left : r.right) - _boxGrab / 2
          : r.center.dx - w / 2,
      top: time
          ? r.center.dy - h / 2
          : (start ? r.top : r.bottom) - _boxGrab / 2,
      width: w,
      height: h,
      child: MouseRegion(
        // The cursor says which way this grab pulls before the button goes
        // down (P2).
        cursor: time
            ? SystemMouseCursors.resizeLeftRight
            : SystemMouseCursors.resizeUpDown,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          supportedDevices: dragDevices,
          // From the down rather than from the slop, as the lanes' own block
          // handles are: a precision grab that starts a pointer's width behind
          // the cursor stays behind it for the whole gesture.
          dragStartBehavior: DragStartBehavior.down,
          onPanStart: (_) => _startBoxDrag(box, time: time, start: start),
          onPanUpdate: (d) {
            if (!_escape.running) return;
            setState(() {
              _boxDrag
                ?..rawDx += d.delta.dx
                ..rawDy += d.delta.dy;
            });
            final live = _boxDrag;
            if (live != null) {
              _previewDrag(_editsFor(_boxMove(live, range, height)).$1);
            }
          },
          onPanEnd: (_) => _commitBoxDrag(range, height),
          onPanCancel: () {
            _escape.end();
            _abandonDrag();
          },
        ),
      ),
    );
  }

  /// The drawing's **value hint pill**, riding beside the one key in hand:
  /// `f<frame> · <value> · <in> / <out> %` (§6.2).
  ///
  /// Drawn from the selection, so it is transient by construction: it arrives
  /// with the key being worked on, follows a drag live — the key it reads is
  /// the *shown* key, travel and all — and leaves when the selection does (P1).
  /// One key only: with a block in hand the block's own badge is the readout.
  /// The capture line: where a key drag's snap has taken it, drawn full height
  /// while the target holds the drag (docs/07 §4.5) and gone on release.
  List<Widget> _snapCapture(LumitTheme t) {
    final drag = _keyDrag;
    if (drag == null || widget.lens != GraphLens.value) return const [];
    final caught = _snappedKeyTravel(drag).caught;
    if (caught == null) return const [];
    return [
      Positioned(
        key: const ValueKey<String>('graph-snap-caught'),
        left: widget.axis.xOf(caught.frame) - 0.5,
        top: 0,
        bottom: 0,
        width: 1,
        child: IgnorePointer(child: ColoredBox(color: t.accent)),
      ),
    ];
  }

  List<Widget> _valueHint(
      (double, double) range, double height, double paneWidth) {
    if (widget.lens != GraphLens.value || widget.selectedKeys.length != 1) {
      return const [];
    }
    final id = widget.selectedKeys.first;
    final hash = id.lastIndexOf('#');
    if (hash < 0) return const [];
    final index = int.tryParse(id.substring(hash + 1));
    if (index == null) return const [];
    final channelId = id.substring(0, hash);
    for (final channel in widget.channels) {
      if (channel.id != channelId) continue;
      final keys = _shownKeys(channel);
      if (index < 0 || index >= keys.length) return const [];
      final key = keys[index];
      final point = _keyPoint(channel, index, range, height, isOut: true);
      final value = key.value;
      // Room for the pill beside the key, or on its other side where the key
      // has run out of pane — a readout clipped by the edge is no readout.
      const width = 96.0;
      final left = point.dx + 8 + width > paneWidth
          ? point.dx - 8 - width
          : point.dx + 8;
      return [
        Positioned(
          key: const ValueKey<String>('graph-value-hint'),
          left: left,
          top: point.dy + 6,
          child: HintPill(
            text: l10n.graphKeyHint(
              _keyFrame(key, widget.fps).round(),
              graphNumberText(value),
              (sideInfluence(key.interpIn) * 100).round(),
              (sideInfluence(key.interpOut) * 100).round(),
            ),
          ),
        ),
      ];
    }
    return const [];
  }

  /// The grabbable key glyphs.
  List<Widget> _keyHandles(
      LumitTheme t, (double, double) range, double height) {
    final out = <Widget>[];
    for (final channel in widget.channels) {
      final keys = channel.keys;
      for (var i = 0; i < keys.length; i++) {
        final id = '${channel.id}#$i';
        final chosen = widget.selectedKeys.contains(id);
        final sides = widget.lens == GraphLens.value || isEnvelope(channel)
            // The value view, and the Vegas envelope: one point per key. The
            // envelope's whole idea is that a key has *a* speed rather than
            // two one-sided ones (K-247), so a second dot would be a second
            // answer to a question with one.
            ? const [true]
            // Speed lens: an in dot and an out dot, moved independently —
            // the ends have only the side that exists.
            : [
                if (i > 0) false,
                if (i + 1 < keys.length || keys.length == 1) true,
              ];
        for (final isOut in sides) {
          final point = _keyPoint(channel, i, range, height, isOut: isOut);
          out.add(Positioned(
            left: point.dx - _keyGrab / 2,
            top: point.dy - _keyGrab / 2,
            child: GestureDetector(
              key: ValueKey<String>(widget.lens == GraphLens.value
                  ? 'graph-key-$id'
                  : 'graph-key-$id-${isOut ? 'out' : 'in'}'),
              behavior: HitTestBehavior.opaque,
              // `Alt`-click lifts the key, and so does a click with the Pen
              // armed — the counterpart of planting one on the curve. A plain
              // click still selects.
              //
              // Deliberately **not** `onDoubleTap`. Registering one makes
              // Flutter hold every single tap back until the double-tap timer
              // expires, so selecting a key — the commonest thing anyone does
              // here — would gain a visible delay. Double-click stays the
              // gesture for *planting* a key on empty curve, where there is no
              // competing single click to slow down.
              onTapUp: (d) {
                if (altActuallyHeld() || widget.penArmed) {
                  _keyTapId = null;
                  _removeKey(channel, i);
                  return;
                }
                // **Double-click opens the numeric fields** (docs/07 §5.3):
                // the same key twice inside the double-tap window, counted by
                // timestamps so the first click still selects at once.
                final again = _keyTap.tap() && _keyTapId == id;
                _keyTapId = id;
                if (again) {
                  _keyTapId = null;
                  _openKeyFields(channel, i, d.globalPosition);
                  return;
                }
                _selectKey(id,
                    toggle: HardwareKeyboard.instance.isControlPressed);
              },
              onSecondaryTapDown: (d) =>
                  _showKeyMenu(channel, i, d.globalPosition),
              supportedDevices: dragDevices,
              onPanStart: (_) {
                if (widget.lens == GraphLens.value) {
                  _startKeyDrag(id);
                } else {
                  // A speed dot: this side's speed vertically, the keyframe's
                  // time sideways. It does not select on the way — changing
                  // the selection mid-gesture rebuilds the handles out from
                  // under the recogniser and the drag dies on its first move.
                  _startHandleDrag(channel, i, isOut, true, range, height);
                }
              },
              onPanUpdate: (d) {
                if (widget.lens == GraphLens.value) {
                  setState(() {
                    _keyDrag
                      ?..rawDx += d.delta.dx
                      ..rawDy += d.delta.dy;
                  });
                  final drag = _keyDrag;
                  if (drag != null) {
                    _previewDrag(_keyDragEdits(drag, range, height).$1);
                  }
                } else {
                  final box = context.findRenderObject();
                  if (box is RenderBox) {
                    _updateHandleDrag(
                        box.globalToLocal(d.globalPosition), range, height,
                        dx: d.delta.dx);
                  }
                }
              },
              onPanEnd: (_) {
                if (widget.lens == GraphLens.value) {
                  _commitKeyDrag(range, height);
                } else {
                  _commitHandleDrag();
                }
              },
              onPanCancel: () {
                _preview.cancel();
                setState(() {
                  _keyDrag = null;
                  _handleDrag = null;
                  _frozen = null;
                });
              },
              // The glyph is small; the target around it is not (see
              // [_keyGrab]).
              //
              // **No cursor of its own** (owner, desktop testing): hovering a
              // key used to swap the pointer for the move cursor, so crossing
              // a curve full of keys made the cursor flicker between two
              // shapes on the way to wherever the hand was actually going.
              // The mark under the pointer already says a key is there, and
              // it brightens to say so. The *drag* cursors stay — a handle
              // swings up and down, a box edge scales — because those are
              // gestures with one direction to promise.
              child: SizedBox(
                width: _keyGrab,
                height: _keyGrab,
                child: Center(
                  child: SizedBox(
                    // A selected key draws one size step larger — the
                    // drawing's 7 in a 6 world — so the catch reads at a
                    // glance without a second colour beyond `text_primary`.
                    width: chosen ? _selectedKeyGlyph : _keyGlyph,
                    height: chosen ? _selectedKeyGlyph : _keyGlyph,
                    child: CustomPaint(
                      painter: _KeyGlyphPainter(
                        key_: keys[i],
                        // Selected is `text_primary` — the one colour
                        // selection speaks in. Not the accent: its jobs are
                        // the playhead, the one filled button and the active
                        // tab tick, and nothing else (K-439).
                        colour: chosen
                            ? t.textPrimary
                            : t.curve[channel.colourIndex % t.curve.length],
                        speedDot: widget.lens == GraphLens.speed,
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ));
        }
      }
    }
    return out;
  }

  /// The draggable tangent endpoints for selected keys.
  // The theme is not asked for here: the ring paints itself from it
  // ([_HandleRing]), so the dot's colours live where the dot is drawn.
  List<Widget> _tangentHandles((double, double) range, double height) {
    final out = <Widget>[];
    for (final channel in widget.channels) {
      final keys = _shownKeys(channel);
      for (var i = 0; i < keys.length; i++) {
        if (!widget.selectedKeys.contains('${channel.id}#$i')) continue;
        for (final isOut in const [true, false]) {
          final e = _handleEndpointFor(channel, keys, i, isOut);
          if (e == null) continue;
          final point = Offset(
            widget.axis.xOf(e.$1 * widget.fps),
            _yOf(e.$2, range, height),
          );
          // A handle's reach is a fraction of the gap to the next key, so on a
          // long composition both handles sit a few pixels from their key —
          // and from each other. A fixed target would make which one you get a
          // coin toss, so it never grows past the reach itself: the two stay
          // tellable apart however tight the curve, and the key underneath
          // keeps whatever is left.
          final reach =
              (point - _keyPoint(channel, i, range, height, isOut: true))
                  .distance;
          final grab = reach.clamp(9.0, _handleGrab);
          out.add(Positioned(
            left: point.dx - grab / 2,
            top: point.dy - grab / 2,
            child: GestureDetector(
              key: ValueKey<String>(
                  'graph-handle-${channel.id}#$i-${isOut ? 'out' : 'in'}'),
              behavior: HitTestBehavior.opaque,
              supportedDevices: dragDevices,
              onPanStart: (_) =>
                  _startHandleDrag(channel, i, isOut, false, range, height),
              onPanUpdate: (d) {
                final box = context.findRenderObject();
                if (box is RenderBox) {
                  _updateHandleDrag(
                      box.globalToLocal(d.globalPosition), range, height);
                }
              },
              onPanEnd: (_) => _commitHandleDrag(),
              onPanCancel: () {
                _preview.cancel();
                setState(() {
                  _handleDrag = null;
                  _frozen = null;
                });
              },
              // A generous target around a small dot: a handle that takes two
              // attempts to grab loses the keyframe's selection on the miss.
              child: _HandleRing(size: grab),
            ),
          ));
        }
      }
    }
    return out;
  }

  /// A selected key's handle endpoint in (seconds, y-value) for the current
  /// lens, or null where the side has no span to reach into.
  (double, double)? _handleEndpointFor(
      GraphChannel channel, List<BridgeKeyframe> keys, int index, bool isOut) {
    final nb = _neighbour(keys, index, isOut);
    if (nb == null) return null;
    final key = keys[index];
    final side = isOut ? key.interpOut : key.interpIn;
    if (widget.lens == GraphLens.value) {
      // Handles belong to eased sides; a linear side has none to show. An
      // automatic side has one — its neighbours simply choose where it points,
      // and dragging it is how the side is taken back to Free.
      if (side is! BridgeSideInterp_Bezier && side is! BridgeSideInterp_Auto) {
        return null;
      }
      final e = handleEndpoint(
        keyTime: rationalSeconds(key.time),
        keyValue: key.value,
        neighbourTime: rationalSeconds(nb.time),
        isOut: isOut,
        speed: sideSpeedAtKey(keys, index, isOut: isOut),
        influence: sideInfluence(side),
      );
      return (e.time, e.value);
    }
    // An envelope point has no influence handle: its sides are always the
    // chord, which is what keeps the lines between points straight (K-247).
    if (isEnvelope(channel)) return null;
    // Speed lens: the influence handle reaches horizontally from the dot.
    final keyTime = rationalSeconds(key.time);
    final dt = (rationalSeconds(nb.time) - keyTime).abs();
    final speed = sideSpeedAtKey(keys, index, isOut: isOut);
    final reach = sideInfluence(side) * dt;
    return (isOut ? keyTime + reach : keyTime - reach, speed);
  }
}

// ---------------------------------------------------------------------------
// Numeric entry (docs/07 §5.3).
// ---------------------------------------------------------------------------

/// The popover's width and its label column — the Ease popover's own two
/// measures, so the two small boxes the graph opens read as one family.
const double _keyFieldsWidth = 168;
const double _keyFieldsLabel = 46;

/// Open the **numeric entry** box on one keyframe: its exact frame, its exact
/// value, and how far each of its two eases reaches.
///
/// [onApply] is called with all four numbers on every change, so the caller
/// writes one key rather than four separate edits, and the box stays up — the
/// point of typing numbers is usually to type more than one. It carries no
/// buttons for the same reason every value well in the application carries
/// none: a field commits what it holds.
Future<void> showKeyFieldsPopover({
  required BuildContext context,
  required Offset position,
  required double frame,
  required double value,
  required double inPercent,
  required double outPercent,
  required double minFrame,
  required double maxFrame,
  required void Function(
          double frame, double value, int inPercent, int outPercent)
      onApply,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => _KeyFieldsPopover(
        frame: frame,
        value: value,
        inPercent: inPercent,
        outPercent: outPercent,
        minFrame: minFrame,
        maxFrame: maxFrame,
        onApply: onApply,
      ),
    );

class _KeyFieldsPopover extends StatefulWidget {
  final double frame;
  final double value;
  final double inPercent;
  final double outPercent;
  final double minFrame;
  final double maxFrame;
  final void Function(double frame, double value, int inPercent, int outPercent)
      onApply;

  const _KeyFieldsPopover({
    required this.frame,
    required this.value,
    required this.inPercent,
    required this.outPercent,
    required this.minFrame,
    required this.maxFrame,
    required this.onApply,
  });

  @override
  State<_KeyFieldsPopover> createState() => _KeyFieldsPopoverState();
}

class _KeyFieldsPopoverState extends State<_KeyFieldsPopover> {
  /// The four numbers as the box holds them. Kept here rather than read back
  /// off the key, because the box outlives the channel snapshot it opened on
  /// (see `_applyKeyFields`): what it shows is what was typed into it.
  late double _frame = widget.frame;
  late double _value = widget.value;
  late double _in = widget.inPercent;
  late double _out = widget.outPercent;

  void _apply() => widget.onApply(_frame, _value, _in.round(), _out.round());

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      width: _keyFieldsWidth,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _row(t, l10n.graphKeyFrameField, 'graph-fields-frame', _frame,
              min: widget.minFrame,
              max: widget.maxFrame,
              decimals: 0, set: (v) {
            _frame = v;
            _apply();
          }),
          _row(t, l10n.graphKeyValueField, 'graph-fields-value', _value,
              min: -100000, max: 100000, decimals: 2, set: (v) {
            _value = v;
            _apply();
          }),
          _row(t, l10n.graphEaseIn, 'graph-fields-in', _in,
              min: 0,
              max: 100,
              decimals: 0,
              suffix: l10n.unitSymbolPercent, set: (v) {
            _in = v;
            _apply();
          }),
          _row(t, l10n.graphEaseOut, 'graph-fields-out', _out,
              min: 0,
              max: 100,
              decimals: 0,
              suffix: l10n.unitSymbolPercent, set: (v) {
            _out = v;
            _apply();
          }),
        ],
      ),
    );
  }

  Widget _row(
    LumitTheme t,
    String label,
    String key,
    double value, {
    required num min,
    required num max,
    required int decimals,
    String? suffix,
    required ValueChanged<double> set,
  }) =>
      Padding(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
        child: Row(
          children: [
            SizedBox(
              width: _keyFieldsLabel,
              child: Text(label,
                  style: t.body, maxLines: 1, overflow: TextOverflow.ellipsis),
            ),
            const SizedBox(width: 6),
            Expanded(
              child: DragValueField(
                key: ValueKey<String>(key),
                value: value,
                min: min,
                max: max,
                decimals: decimals,
                suffix: suffix,
                keyed: true,
                onChanged: (v) => setState(() => set(v.toDouble())),
              ),
            ),
          ],
        ),
      );
}

// ---------------------------------------------------------------------------
// Painters.
// ---------------------------------------------------------------------------

/// The grid, the value-axis labels, and every channel's curve.
class _GraphPainter extends CustomPainter {
  final List<GraphChannel> channels;
  final List<List<BridgeKeyframe>> shownKeys;
  final GraphLens lens;
  final TimelineAxis axis;
  final double fps;

  /// The shared range — what the grid lines and the gutter's numbers say.
  final (double, double) range;

  final List<Color> palette;
  final Color grid;
  final TextStyle label;
  final CompModel comp;

  /// Where the viewport's left edge sits in the canvas's own coordinates.
  ///
  /// The pane is as wide as the whole comp and lives inside the Timeline's
  /// horizontal scroll view, so canvas x 0 is the *start of time*, not the
  /// left of the window. The value labels were painted there and scrolled out
  /// of sight the moment the Timeline moved, leaving the grid lines with
  /// nothing naming them. Painting at the viewport's edge keeps the axis
  /// readable wherever the view is and at whatever zoom.
  final double viewportLeft;

  /// How wide that viewport is, so the gutter can be pinned to its **right**
  /// edge.
  final double viewportWidth;

  /// The gutter's translucent ground.
  final Color gutterFill;

  /// Whether Retime channels draw as the Vegas envelope (K-247) — which puts
  /// their curve on the axis in **per cent** rather than in source seconds per
  /// second, so it lands on the points drawn over it.
  final bool vegas;

  const _GraphPainter({
    required this.channels,
    required this.shownKeys,
    required this.lens,
    required this.axis,
    required this.fps,
    required this.range,
    required this.palette,
    required this.grid,
    required this.label,
    required this.comp,
    required this.viewportLeft,
    required this.viewportWidth,
    required this.gutterFill,
    this.vegas = false,
  });

  double _yOf(double v, (double, double) range, Size size) {
    final (lo, hi) = range;
    final span = (hi - lo).abs() < 1e-12 ? 1.0 : hi - lo;
    return size.height - (v - lo) / span * size.height;
  }

  @override
  void paint(Canvas canvas, Size size) {
    _paintGrid(canvas, size);
    final f = fps <= 0 ? 1.0 : fps;
    for (var c = 0; c < channels.length; c++) {
      final channel = channels[c];
      final keys = shownKeys[c];
      // A Retime drawn as the Vegas envelope reads in per cent, so its curve
      // is scaled onto the same axis as its points (K-247).
      final envelope = vegas && channel.retime && lens == GraphLens.speed;
      final speedScale = envelope ? 100.0 : 1.0;
      // **A shape draws its rate of change in both lenses** (K-344): a path has
      // no value to plot, so the value view would otherwise be an empty pane
      // for a property that is plainly animating.
      final chLens = channel.isMaskPath ? GraphLens.speed : lens;
      final paint = Paint()
        ..color = palette[channel.colourIndex % palette.length]
        ..strokeWidth = 1.4
        ..style = PaintingStyle.stroke;

      if (channel.scalar is! BridgeScalar_Expression) {
        if (channel.isStatic || keys.isEmpty) {
          // A static property is a flat line of its value (a flat 0 as speed).
          final y = _yOf(
              chLens == GraphLens.value ? channel.staticValue : 0, range, size);
          canvas.drawLine(Offset(0, y), Offset(size.width, y), paint);
          continue;
        }
        if (keys.length == 1) {
          final y = _yOf(
              chLens == GraphLens.value ? keys.first.value : 0, range, size);
          canvas.drawLine(Offset(0, y), Offset(size.width, y), paint);
          continue;
        }
      }

      final path = Path();
      const step = 2.5;
      var first = true;
      if (channel.scalar case BridgeScalar_Expression _) {
        // An expression has no keys to walk, so the curve is sampled from the
        // engine across the visible span — the same evaluator the renderer
        // uses, so the drawn line matches the motion that will be rendered.
        final startSeconds = axis.frameAtExact(0) / f;
        final endSeconds = axis.frameAtExact(size.width) / f;
        final start = timeOfSubframe(
            startSeconds * f, comp.fpsExact.$1, comp.fpsExact.$2);
        final end =
            timeOfSubframe(endSeconds * f, comp.fpsExact.$1, comp.fpsExact.$2);

        const samples = 500;
        final result = sampleScalarRangeWithContext(
            scalar: channel.scalar,
            layer: channel.entry.layer,
            start: start,
            end: end,
            samples: samples);

        for (var i = 0; i < result.length; i++) {
          final x = size.width * (i.toDouble() / samples.toDouble());
          final point = Offset(x, _yOf(result[i], range, size));
          if (first) {
            path.moveTo(point.dx, point.dy);
            first = false;
          } else {
            path.lineTo(point.dx, point.dy);
          }
        }
      } else {
        for (var x = 0.0; x <= size.width; x += step) {
          final seconds = axis.frameAtExact(x) / f;
          final v = chLens == GraphLens.value
              ? evaluateKeys(keys, seconds)
              : evaluateKeysSpeed(keys, seconds) * speedScale;
          final point = Offset(x, _yOf(v, range, size));
          if (first) {
            path.moveTo(point.dx, point.dy);
            first = false;
          } else {
            path.lineTo(point.dx, point.dy);
          }
        }
      }
      canvas.drawPath(path, paint);

      // Speed lens: the vertical join at each key, where in and out speed
      // step — drawn faint so a discontinuity reads as one key, not two.
      if (lens == GraphLens.speed && !envelope) {
        final joinPaint = Paint()
          ..color = paint.color.withValues(alpha: 0.35)
          ..strokeWidth = 1;
        for (var i = 0; i < keys.length; i++) {
          final x = axis.xOf(_keyFrame(keys[i], f));
          final yIn = _yOf(sideSpeedAtKey(keys, i, isOut: false), range, size);
          final yOut = _yOf(sideSpeedAtKey(keys, i, isOut: true), range, size);
          if ((yIn - yOut).abs() > 1) {
            canvas.drawLine(Offset(x, yIn), Offset(x, yOut), joinPaint);
          }
        }
      }
    }
    _paintGutter(canvas, size);
  }

  /// The values the grid rules at: a nice step whose lines sit at least ~36 px
  /// apart. Empty when the range is not a range at all.
  List<double> _gridValues(Size size) {
    final (lo, hi) = range;
    final span = (hi - lo).abs() < 1e-12 ? 1.0 : hi - lo;
    final rawStep = span / (size.height / 36).clamp(1, 12);
    final magnitude = _pow10((rawStep.abs()).clamp(1e-12, double.infinity));
    var step = magnitude;
    for (final m in const [1.0, 2.0, 5.0, 10.0]) {
      if (magnitude * m >= rawStep) {
        step = magnitude * m;
        break;
      }
    }
    if (!step.isFinite || step <= 0) return const [];
    final out = <double>[];
    for (var v = (lo / step).ceilToDouble() * step; v <= hi; v += step) {
      out.add(v);
      // A range and a step that disagree by a rounding error would otherwise
      // rule a line per pixel for ever.
      if (out.length > 64) break;
    }
    return out;
  }

  void _paintGrid(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = grid
      ..strokeWidth = 1;
    for (final v in _gridValues(size)) {
      final y = _yOf(v, range, size);
      canvas.drawLine(Offset(0, y), Offset(size.width, y), paint);
    }
  }

  /// The value labels, in the **fixed right-hand gutter** the drawing gives
  /// them (§12A.2, §6.3): a [graphGutterWidth] strip of translucent ground at
  /// the right edge of the *viewport*, with each grid line's number sitting on
  /// it.
  ///
  /// The viewport, not the canvas: the pane is as wide as the whole
  /// composition and lives inside the Timeline's horizontal scroll view, so a
  /// gutter pinned to the canvas would be off screen at every zoom but one.
  /// Drawn last, over the curves — the drawing's own translucent strip, which
  /// lets a curve be seen running under its own numbers.
  void _paintGutter(Canvas canvas, Size size) {
    final left = viewportLeft + viewportWidth - graphGutterWidth;
    canvas.drawRect(
      Rect.fromLTWH(left, 0, graphGutterWidth, size.height),
      Paint()..color = gutterFill,
    );
    for (final v in _gridValues(size)) {
      final y = _yOf(v, range, size);
      final text = TextPainter(
        text: TextSpan(
            text: (v.abs() >= 100 || v == v.roundToDouble()
                ? v.round().toString()
                : v.toStringAsFixed(1)),
            style: label),
        textDirection: TextDirection.ltr,
      )..layout();
      text.paint(canvas,
          Offset(left + 4, (y - text.height - 1).clamp(0, size.height - 12)));
    }
  }

  /// The power of ten at or just under `v` (0.03 → 0.01, 30 → 10).
  static double _pow10(double v) {
    var p = 1.0;
    while (p > v) {
      p /= 10;
    }
    while (p * 10 <= v) {
      p *= 10;
    }
    return p;
  }

  @override
  bool shouldRepaint(_GraphPainter old) => true;

  /// A picture, not a control: hits fall through to the marquee.
  @override
  bool? hitTest(Offset position) => false;
}

/// The lines from selected keys to their tangent (or influence) endpoints:
/// the drawing's dashed hairline, 2 on and 2 off, in `text_primary`.
///
/// Dashed rather than solid because the line is scaffolding — it says which dot
/// belongs to which key, and it must not be mistaken for a curve. The dashes
/// are stepped along the line here rather than cut out of a path's metrics: it
/// is one straight segment, so the arithmetic is a lerp.
class _HandlesPainter extends CustomPainter {
  final GraphEditorFrbState state;
  final (double, double) range;
  final double height;
  final Color colour;

  const _HandlesPainter({
    required this.state,
    required this.range,
    required this.height,
    required this.colour,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = colour.withValues(alpha: 0.8)
      ..strokeWidth = 1;
    final widget = state.widget;
    for (final channel in widget.channels) {
      final keys = state._shownKeys(channel);
      for (var i = 0; i < keys.length; i++) {
        if (!widget.selectedKeys.contains('${channel.id}#$i')) continue;
        for (final isOut in const [true, false]) {
          final e = state._handleEndpointFor(channel, keys, i, isOut);
          if (e == null) continue;
          final from = widget.lens == GraphLens.value
              ? state._keyPoint(channel, i, range, height, isOut: true)
              : state._keyPoint(channel, i, range, height, isOut: isOut);
          final to = Offset(
            widget.axis.xOf(e.$1 * widget.fps),
            state._yOf(e.$2, range, height),
          );
          _dashed(canvas, from, to, paint);
        }
      }
    }
  }

  /// 2 on, 2 off, from [from] to [to].
  static void _dashed(Canvas canvas, Offset from, Offset to, Paint paint) {
    const dash = 2.0;
    const gap = 2.0;
    final length = (to - from).distance;
    if (length <= 0) return;
    for (var at = 0.0; at < length; at += dash + gap) {
      final end = (at + dash).clamp(0.0, length);
      canvas.drawLine(
        Offset.lerp(from, to, at / length)!,
        Offset.lerp(from, to, end / length)!,
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(_HandlesPainter old) => true;

  @override
  bool? hitTest(Offset position) => false;
}

/// A keyframe's glyph, coded by interpolation: diamond for linear, circle for
/// an eased (bezier) key, square for hold — the same coding the lanes will
/// learn (docs/07 §4.3). On the speed lens every dot is a circle.
class _KeyGlyphPainter extends CustomPainter {
  final BridgeKeyframe key_;
  final Color colour;
  final bool speedDot;
  const _KeyGlyphPainter({
    required this.key_,
    required this.colour,
    required this.speedDot,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()..color = colour;
    final half = size.width / 2;
    // An automatic side is an eased side: it has a tangent, so its key is a
    // circle like any other eased one.
    if (speedDot ||
        key_.interpIn is BridgeSideInterp_Bezier ||
        key_.interpOut is BridgeSideInterp_Bezier ||
        key_.interpIn is BridgeSideInterp_Auto ||
        key_.interpOut is BridgeSideInterp_Auto) {
      canvas.drawCircle(Offset(half, half), half - 1, paint);
      return;
    }
    if (key_.interpOut is BridgeSideInterp_Hold) {
      canvas.drawRect(
          Rect.fromLTWH(1, 1, size.width - 2, size.height - 2), paint);
      return;
    }
    canvas.drawPath(
      Path()
        ..moveTo(half, 0)
        ..lineTo(size.width, half)
        ..lineTo(half, size.height)
        ..lineTo(0, half)
        ..close(),
      paint,
    );
  }

  @override
  bool shouldRepaint(_KeyGlyphPainter old) =>
      old.colour != colour ||
      old.speedDot != speedDot ||
      old.key_.interpIn != key_.interpIn ||
      old.key_.interpOut != key_.interpOut;
}

/// A tangent endpoint's dot: the drawing's **hollow ring** — a `text_primary`
/// stroke round a hole punched in the pane's own ground, so the curve running
/// under it does not read as running through it.
class _HandleDotPainter extends CustomPainter {
  final Color colour;
  final Color fill;

  /// The pointer is over the ring's target: the stroke comes up to full
  /// strength, and goes back down when it leaves (P1 — nothing at rest).
  final bool hovered;

  const _HandleDotPainter(
      {required this.colour, required this.fill, this.hovered = false});

  @override
  void paint(Canvas canvas, Size size) {
    final centre = Offset(size.width / 2, size.height / 2);
    final r = size.width / 2 - 1;
    canvas.drawCircle(centre, r, Paint()..color = fill);
    canvas.drawCircle(
      centre,
      r,
      Paint()
        ..color = hovered ? colour : colour.withValues(alpha: 0.8)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(_HandleDotPainter old) =>
      old.colour != colour || old.fill != fill || old.hovered != hovered;
}

/// A tangent handle's dot with its own hover state, so brightening one ring
/// repaints that ring rather than rebuilding the pane under the pointer.
class _HandleRing extends StatefulWidget {
  final double size;
  const _HandleRing({required this.size});

  @override
  State<_HandleRing> createState() => _HandleRingState();
}

class _HandleRingState extends State<_HandleRing> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return MouseRegion(
      // A handle swings: the drag that matters is the vertical one, and the
      // cursor says so before the button goes down (P2).
      cursor: SystemMouseCursors.resizeUpDown,
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: SizedBox(
        width: widget.size,
        height: widget.size,
        child: Center(
          child: SizedBox(
            width: 8,
            height: 8,
            child: CustomPaint(
              painter: _HandleDotPainter(
                colour: t.textPrimary,
                fill: t.surface0,
                hovered: _hovered,
              ),
            ),
          ),
        ),
      ),
    );
  }
}
