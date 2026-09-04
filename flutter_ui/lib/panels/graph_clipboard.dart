// The graph editor's keyframe clipboard (docs/07 §5.3): copy, the
// tab-separated text mirror, and paste. Split out of graph_editor_frb.dart,
// which re-exports it.

import 'package:flutter/services.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart';

import '../l10n/strings.dart';
import 'effect_param_row_frb.dart';
import 'graph_channels.dart';
import 'graph_edits.dart';
import 'graph_maths.dart';
import 'layer_fold_frb.dart';

// ---------------------------------------------------------------------------
// The keyframe clipboard (docs/07 §5.3).
// ---------------------------------------------------------------------------

/// One copied channel: where it came from (for the AE text's property line)
/// and its keys with full easing fidelity.
///
/// A row with **no keyframes at all** copies too: it has a value, and
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
/// **Returns whether anything was taken**. It used to return nothing
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
              frame: keyFrame(k, fps),
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
/// of one that has none. What `Ctrl+C` does with property rows selected
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
      frames.add(keyFrame(k, fps));
    }
  }
  final sorted = frames.toList()..sort();
  final columns = axes.length == 1
      ? [unit]
      : [
          for (var i = 0; i < axes.length; i++)
            '${axisLetter(i).toUpperCase()} $unit'
        ];

  /// The key an axis has exactly on `frame`, if any — the one whose easing
  /// the row carries. A filled-in value has no key, and so no easing.
  BridgeKeyframe? keyAt(GraphClipChannel axis, double frame) {
    for (final k in axis.keys) {
      if ((keyFrame(k, fps) - frame).abs() < 1e-9) return k;
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
/// [project] is what makes a paste **one undo step**: a clipboard that
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

  // A value copied from a row with no keyframes pastes as a value: onto
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
        [for (final k in clip.keys) (keyFrame(k, fps), k)],
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
              for (final k in withKeyAt(
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
      for (final k in channel.keys) keyFrame(k, fps): k,
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
