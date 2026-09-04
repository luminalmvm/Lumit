// The graph editor's edit operations: how a gesture or a menu writes curves
// back through the bridge. Split out of graph_editor_frb.dart, which
// re-exports it.

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'easing_curve.dart';
import 'graph_channels.dart';
import 'graph_maths.dart';
import 'key_block.dart';
import 'layer_fold_frb.dart';
import 'text_animator_rows_frb.dart';
import 'transform_rows_frb.dart';

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
    } else if (channel.animatorValue case final value?) {
      // One number of one animator, written through the whole document —
      // which is the op, so two curves on one animator are two writes
      // and two undo steps, exactly as two on one mask are.
      writeTextAnimatorScalar(
        layer: channel.entry.layer,
        index: channel.animator,
        value: value,
        to: next,
      );
    } else if (channel.isMaskPath && channel.mask != null) {
      // A shape key holds a path, not a number, so only its time and its eases
      // can be written — which is exactly what a graph edit changes.
      channel.entry.layer.setMaskPathKeys(
        id: channel.mask!.id,
        keys: keysOf(next),
      );
    } else if (channel.mask case final mask?) {
      // A mask edit takes the whole mask, so there is nothing to batch per
      // property; two curves on one mask are two writes and two undo steps,
      // which is what `SetLayerMasks` costs until it grows a per-key op.
      channel.entry.layer.setMask(
        mask: maskWithScalar(mask, channel.maskValue!, next,
            channel.maskVertex),
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
/// **Why this exists.** A drag is one op on release, so between the
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
/// place to grab, not an edit (docs/07 §4.3, the lane gesture).
///
/// A channel with nothing keyed is left alone: the gesture is *"plant a key on
/// this keyed row"*, and turning a static property into an animated one is the
/// stopwatch's job, not a Ctrl-click's. A channel that already has a key on
/// that frame is left alone too, because two keys at one time is not a curve
/// the engine will take. A mask's **shape** channel is skipped: a path
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
    if (keys.any((k) => keyFrame(k, fps).round() == frame.round())) continue;
    edits[channel] = BridgeScalar.keyframed(withKeyAt(
        keys, frame, evaluateKeys(keys, seconds), fps, fpsNum, fpsDen));
  }
  if (edits.isEmpty) return false;
  commitChannelEdits(edits);
  return true;
}

/// Remove every key in [selectedKeys] from [channels] — the graph's Delete and
/// the lane key menu's *Delete key* are the same removal.
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
/// measure before they move anything.
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
        out.add(keyFrame(keys[i], fps));
      }
    }
  }
  return out;
}

/// Give every selected key a new time, [frameOf] deciding where each one goes
/// from where it is and which channel it is on.
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
      final was = keyFrame(keys[i], fps);
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
/// (the Keys mode bottom bar).
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
/// rather than together (the Ease popover's Stagger).
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
