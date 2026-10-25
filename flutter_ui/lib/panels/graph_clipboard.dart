// The graph editor's keyframe clipboard (docs/07 §5.3): copy, the
// tab-separated text mirror, and paste. Split out of graph_editor_frb.dart,
// which re-exports it.

import 'package:flutter/services.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart';
import 'package:uuid/uuid.dart';

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

  /// A mask path's shapes as they were at the copy, kept by the engine as
  /// text.
  final String? shapes;
  const GraphClipChannel(this.source, this.keys,
      {this.staticValue, this.shapes});
}

/// [channel]'s [keys] for the clipboard. A mask path's shapes are taken now,
/// so an edit to the mask after the copy does not change what pastes.
GraphClipChannel _clipOf(GraphChannel channel, List<BridgeKeyframe> keys) {
  String? shapes;
  if (channel.isMaskPath) {
    try {
      shapes = channel.entry.layer.copyMaskPathKeys(
          id: channel.mask!.id, times: [for (final k in keys) k.time]);
    } catch (_) {
      // The mask has gone, so its shapes have nothing to paste.
    }
  }
  return GraphClipChannel(channel, keys, shapes: shapes);
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
    if (hit.isNotEmpty) copied.add(_clipOf(channel, hit));
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
  final full = [
    for (final channel in channels)
      if (channel.scalar case BridgeScalar_Static(:final field0))
        GraphClipChannel(channel, const [], staticValue: field0)
      else
        _clipOf(channel, channel.keys),
  ];
  graphKeyClipboard = full;

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
  graphKeyClipboard = full;
  return true;
}

/// The property line and columns for a transform property's copied axes.
LumitClipGroup _transformClipGroup(
    GraphChannel lead, List<GraphClipChannel> axes, double fps) {
  final (name, unit) = switch (lead.prop!) {
    BridgeTransformProp.anchorX ||
    BridgeTransformProp.anchorY ||
    BridgeTransformProp.anchorZ =>
      (l10n.transformAnchorPoint, l10n.unitPixels),
    BridgeTransformProp.positionX ||
    BridgeTransformProp.positionY ||
    BridgeTransformProp.positionZ =>
      (l10n.transformPosition, l10n.unitPixels),
    BridgeTransformProp.scaleX ||
    BridgeTransformProp.scaleY ||
    BridgeTransformProp.scaleZ =>
      (l10n.transformScale, l10n.unitPercent),
    BridgeTransformProp.orientationX ||
    BridgeTransformProp.orientationY ||
    BridgeTransformProp.orientationZ =>
      (l10n.transformOrientation, l10n.unitDegrees),
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
    BridgeTransformProp.poiX ||
    BridgeTransformProp.poiY ||
    BridgeTransformProp.poiZ =>
      (l10n.transformPointOfInterest, l10n.unitPixels),
    BridgeTransformProp.zoom => (l10n.sourceZoom, l10n.unitPixels),
    BridgeTransformProp.focusDistance => (
        l10n.cameraFocusDistance,
        l10n.unitPixels
      ),
    BridgeTransformProp.aperture => (l10n.cameraAperture, l10n.unitPixels),
    BridgeTransformProp.blurLevel => (
        l10n.cameraBlurLevel,
        l10n.unitPercent
      ),
    BridgeTransformProp.lightIntensity => (
        l10n.lightIntensity,
        l10n.unitPercent
      ),
    BridgeTransformProp.lightColourR ||
    BridgeTransformProp.lightColourG ||
    BridgeTransformProp.lightColourB =>
      (l10n.lightColour, ''),
    BridgeTransformProp.coneAngle => (l10n.lightConeAngle, l10n.unitDegrees),
    BridgeTransformProp.coneFeather => (
        l10n.lightConeFeather,
        l10n.unitPercent
      ),
    BridgeTransformProp.lightRadius => (l10n.lightRadius, l10n.unitPixels),
    BridgeTransformProp.falloffDistance => (
        l10n.lightFalloffDistance,
        l10n.unitPixels
      ),
    BridgeTransformProp.lightWidth ||
    BridgeTransformProp.lightHeight =>
      (l10n.size, l10n.unitPixels),
    BridgeTransformProp.ambient => (l10n.materialAmbient, l10n.unitPercent),
    BridgeTransformProp.diffuse => (l10n.materialDiffuse, l10n.unitPercent),
    BridgeTransformProp.specular => (
        l10n.materialSpecular,
        l10n.unitPercent
      ),
    BridgeTransformProp.shininess => (
        l10n.materialShininess,
        l10n.unitPercent
      ),
    BridgeTransformProp.metal => (l10n.materialMetal, l10n.unitPercent),
    BridgeTransformProp.shadowDarkness => (
        l10n.lightShadowDarkness,
        l10n.unitPercent
      ),
    BridgeTransformProp.shadowDiffusion => (
        l10n.lightShadowDiffusion,
        l10n.unitPixels
      ),
    BridgeTransformProp.lightTransmission => (
        l10n.materialLightTransmission,
        l10n.unitPercent
      ),
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
///
/// [onPasted] hears where the pasted keys ended up, so they can be selected.
Future<bool> pasteKeysAtPlayhead({
  required List<GraphChannel> channels,
  required int playheadFrame,
  required double fps,
  required int fpsNum,
  required int fpsDen,
  ProjectReference? project,
  void Function(List<PastedKey> pasted)? onPasted,
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

  final paste = _Paste(
    clips: graphKeyClipboard.isEmpty ? null : graphKeyClipboard,
    sources: sources,
    statics: statics,
    playheadFrame: playheadFrame,
    fps: fps,
    fpsNum: fpsNum,
    fpsDen: fpsDen,
  );
  if (!paste.any) return false;
  for (var i = 0; i < channels.length && i < sources.length; i++) {
    paste.put(channels[i], i);
  }
  return paste.commit(project, onPasted);
}

/// Where one pasted key ended up: its row's path, its curve's id, and its
/// place among that curve's keys.
typedef PastedKey = ({String path, String id, int index});

/// Paste the in-app clipboard onto whole layers, for when layers are picked
/// and no property rows are. Each copied property lands on the same property
/// of every one of [targets], keeping its timing, the earliest key on the
/// playhead.
///
/// Masks and shape items are matched by name, then by place. An effect is
/// matched by kind: the one at the same place among the target's effects of
/// that kind, or else the first. A target with none gets a copy of the effect
/// the keys came off, settings and all, and the keys go on that. Anything a
/// target does not have is passed over. One undo step.
bool pasteKeysOntoLayers({
  required List<BridgeLayerEntry> targets,
  required int playheadFrame,
  required double fps,
  required int fpsNum,
  required int fpsDen,
  ProjectReference? project,
  void Function(List<PastedKey> pasted)? onPasted,
}) {
  final clips = graphKeyClipboard;
  if (clips.isEmpty || targets.isEmpty) return false;
  final paste = _Paste(
    clips: clips,
    sources: [
      for (final clip in clips) [for (final k in clip.keys) (keyFrame(k, fps), k)],
    ],
    statics: [for (final clip in clips) clip.staticValue],
    playheadFrame: playheadFrame,
    fps: fps,
    fpsNum: fpsNum,
    fpsDen: fpsDen,
  );
  if (!paste.any) return false;
  return paste.commit(project, onPasted, before: () {
    for (final target in targets) {
      _matchOnLayer(paste, clips, target);
    }
  });
}

/// Aim each of [clips] at the same property on [target], adding the effects
/// it lacks first.
void _matchOnLayer(
    _Paste paste, List<GraphClipChannel> clips, BridgeLayerEntry target) {
  var entry = target;
  final to = target.layer.internallayerId.toString();
  // A copied effect's, mask's or shape item's id, to the target's own.
  final swaps = <String, String?>{};
  final fresh = <String>{};
  final seen = <String>{};
  // The effects to add, by the layer they came off.
  final missing = <String, (BridgeLayerEntry, List<UuidValue>)>{};
  for (final clip in clips) {
    final source = clip.source;
    final info = source.entry.info;
    final from = source.entry.layer.internallayerId.toString();
    if (source.effect case final fx? when seen.add('${fx.id}')) {
      final style = isUnderPath(stylesPath(from), source.path);
      List<String> ofKind(List<BridgeEffectInstanceInfo> list) => [
            for (final e in list)
              if (e.name == fx.name) '${e.id}'
          ];
      final theirs = ofKind(style ? info.styles : info.effects);
      final ours = ofKind(style ? entry.info.styles : entry.info.effects);
      final nth = theirs.indexOf('${fx.id}');
      if (ours.isNotEmpty) {
        swaps['${fx.id}'] = nth >= 0 && nth < ours.length ? ours[nth] : ours[0];
      } else if (!style) {
        (missing[from] ??= (source.entry, [])).$2.add(fx.id);
      }
    } else if (source.mask case final mask?) {
      swaps['${mask.id}'] ??= _sameItem(
          [for (final m in info.masks) (m.name, '${m.id}')],
          [for (final m in entry.info.masks) (m.name, '${m.id}')],
          '${mask.id}');
    } else if (source.shape case final item?) {
      swaps['${item.id}'] ??= _sameItem(
          [for (final s in info.shapeContents) (s.name, '${s.id}')],
          [for (final s in entry.info.shapeContents) (s.name, '${s.id}')],
          '${item.id}');
    }
  }

  // The copies, settings and all, each with an id of its own. The copy comes
  // out in stack order, so the new effects are the last ones, in that order.
  for (final (source, ids) in missing.values) {
    final order = [for (final e in source.info.effects) '${e.id}'];
    ids.sort((a, b) => order.indexOf('$a').compareTo(order.indexOf('$b')));
    final had = entry.info.effects.length;
    try {
      target.layer.loadPreset(
          text: source.layer.copyEffects(effects: ids), clip: null);
      entry = BridgeLayerEntry(layer: target.layer, info: target.layer.getInfo());
    } catch (_) {
      // The effect has gone since the copy, so its keys have nowhere to go.
      continue;
    }
    paste.added = true;
    final added = entry.info.effects.skip(had).toList();
    if (added.length != ids.length) continue;
    for (var i = 0; i < ids.length; i++) {
      swaps['${ids[i]}'] = '${added[i].id}';
      fresh.add('${added[i].id}');
    }
  }

  for (var i = 0; i < clips.length; i++) {
    final source = clips[i].source;
    final from = source.entry.layer.internallayerId.toString();
    String swap(String s) {
      var out = '$to${s.substring(from.length)}';
      swaps.forEach((a, b) {
        if (b != null) out = out.replaceFirst('/$a/', '/$b/');
      });
      return out;
    }

    // A still mask has no path curve to find, so its shape is aimed at
    // directly.
    if (source.isMaskPath) {
      final id = swaps['${source.mask!.id}'];
      final mask = entry.info.masks.where((m) => '${m.id}' == id).firstOrNull;
      if (mask != null) {
        paste.putPath(target.layer, swap(source.path), mask.id, mask.pathKeys, i);
      }
      continue;
    }
    final id = swap(source.id);
    final channel = graphChannels(layers: [entry], selected: [swap(source.path)])
        .where((c) => c.id == id)
        .firstOrNull;
    if (channel == null) continue;
    paste.put(channel, i, fresh: fresh.contains('${channel.effect?.id}'));
  }
}

/// The id in [ours] of the item [id] names in [theirs]: the one with the same
/// name, or else the one in the same place. Each item is a (name, id) pair.
String? _sameItem(
    List<(String, String)> theirs, List<(String, String)> ours, String id) {
  final at = theirs.indexWhere((t) => t.$2 == id);
  if (at < 0) return null;
  for (final (name, ourId) in ours) {
    if (name == theirs[at].$1) return ourId;
  }
  return at < ours.length ? ours[at].$2 : null;
}

/// One paste in the making: the copied keys as comp frames, how far they
/// shift, and the writes gathered target by target until [commit].
class _Paste {
  /// The in-app clips the sources came from, or null for keyframe text.
  final List<GraphClipChannel>? clips;
  final List<List<(double, BridgeKeyframe)>> sources;
  final List<double?> statics;
  final int playheadFrame;
  final double fps;
  final int fpsNum;
  final int fpsDen;

  /// Whether there is anything to paste at all.
  late final bool any;
  late final double shift;

  final _edits = <GraphChannel, BridgeScalar>{};

  /// Mask shapes to copy across: the layer, the row, the mask, its keys
  /// before the paste, and the clip.
  final _paths =
      <(LayerReference, String, UuidValue, List<BridgeKeyframe>, int)>[];

  /// The frames put down on each row, to select afterwards.
  final _landed = <PastedKey>[];

  /// Whether an effect was added on the way.
  bool added = false;

  _Paste({
    required this.clips,
    required this.sources,
    required this.statics,
    required this.playheadFrame,
    required this.fps,
    required this.fpsNum,
    required this.fpsDen,
  }) {
    var earliest = double.infinity;
    for (final source in sources) {
      for (final (frame, _) in source) {
        if (frame < earliest) earliest = frame;
      }
    }
    // Values only: nothing has a time, so there is no shift to work out and
    // the paste is not about the playhead at all.
    any = earliest.isFinite || statics.any((v) => v != null);
    shift = earliest.isFinite ? playheadFrame - earliest : 0.0;
  }

  /// Clip [i] onto [channel]. A [fresh] channel is on an effect this paste
  /// added, whose copied curve the pasted keys replace rather than join.
  void put(GraphChannel channel, int i, {bool fresh = false}) {
    // A shape is not a number, so it only goes onto another shape.
    final clip = clips?[i];
    if (channel.isPath || (clip?.source.isPath ?? false)) {
      if (channel.isMaskPath && clip != null && clip.source.isMaskPath) {
        putPath(channel.entry.layer, channel.path, channel.mask!.id,
            channel.keys, i);
      }
      return;
    }
    final value = i < statics.length ? statics[i] : null;
    if (value != null) {
      final next = channel.isStatic
          ? BridgeScalar.static_(value)
          : BridgeScalar.keyframed(withKeyAt(channel.keys,
              playheadFrame.toDouble(), value, fps, fpsNum, fpsDen));
      _edits[channel] = next;
      if (!channel.isStatic) {
        _land(channel.path, channel.id, keysOf(next), {playheadFrame * 1.0});
      }
      return;
    }
    // Merge on frames: a pasted key replaces one already at its frame — two
    // keys at one time is not a curve the engine will take.
    final merged = <double, BridgeKeyframe>{
      if (!fresh)
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
    final keys = [for (final f in frames) merged[f]!];
    _edits[channel] = BridgeScalar.keyframed(keys);
    _land(channel.path, channel.id, keys,
        {for (final (frame, _) in sources[i]) frame + shift});
  }

  /// Clip [i]'s mask shapes onto [mask] of [layer], whose row is [path].
  void putPath(LayerReference layer, String path, UuidValue mask,
          List<BridgeKeyframe> keys, int i) =>
      _paths.add((layer, path, mask, keys, i));

  void _land(
      String path, String id, List<BridgeKeyframe> keys, Set<double> frames) {
    for (var k = 0; k < keys.length; k++) {
      final f = keyFrame(keys[k], fps);
      if (frames.any((p) => (p - f).abs() < 1e-3)) {
        _landed.add((path: path, id: id, index: k));
      }
    }
  }

  /// Write everything, after [before], as one undo step. Says whether
  /// anything was pasted, and tells [onPasted] where.
  bool commit(ProjectReference? project,
      void Function(List<PastedKey> pasted)? onPasted,
      {void Function()? before}) {
    var pathed = false;
    asOneUndoStep(project, () {
      before?.call();
      if (_edits.isNotEmpty) commitChannelEdits(_edits);
      for (final (layer, path, mask, had, i) in _paths) {
        final clip = clips![i];
        final shapes = clip.shapes;
        if (shapes == null) continue;
        final at = [for (final (f, _) in sources[i]) f + shift];
        try {
          if (!layer.pasteMaskPathKeys(
            id: mask,
            copied: shapes,
            times: [for (final k in clip.keys) k.time],
            to: [for (final f in at) timeOfSubframe(f, fpsNum, fpsDen)],
          )) {
            continue;
          }
        } catch (_) {
          // The mask it was going onto has gone.
          continue;
        }
        pathed = true;
        // The keys it now has: the ones it had, less any a pasted one
        // replaced, and the pasted ones.
        bool pasted(double f) => at.any((p) => (p - f).abs() < 1e-3);
        final frames = [
          for (final k in had)
            if (!pasted(keyFrame(k, fps))) keyFrame(k, fps),
          ...at,
        ]..sort();
        for (var k = 0; k < frames.length; k++) {
          if (pasted(frames[k])) {
            _landed.add((path: path, id: path, index: k));
          }
        }
      }
    });
    if (_edits.isEmpty && !pathed && !added) return false;
    onPasted?.call(_landed);
    return true;
  }
}

// ---------------------------------------------------------------------------
// Copy ease and Paste ease: the easing of keys, without their values or times.
// ---------------------------------------------------------------------------

/// What Copy ease took: each copied key's way in and way out, in time order.
List<(BridgeSideInterp, BridgeSideInterp)> easeClipboard = const [];

/// Copy ease: the sides of the selected keys on the first channel that has
/// any selected. Returns whether anything was taken.
bool copySelectedEase({
  required List<GraphChannel> channels,
  required Set<String> selectedKeys,
}) {
  for (final channel in channels) {
    final keys = channel.keys;
    final picked = [
      for (var i = 0; i < keys.length; i++)
        if (selectedKeys.contains('${channel.id}#$i'))
          (keys[i].interpIn, keys[i].interpOut),
    ];
    if (picked.isEmpty) continue;
    easeClipboard = picked;
    return true;
  }
  return false;
}

/// Paste ease: the copied sides onto each channel's selected keys, in time
/// order, as one undo step. One copied key goes onto every key; a longer run
/// repeats span by span, so a copied pair shapes every selected pair.
bool pasteEaseOnSelection({
  required ProjectReference? project,
  required List<GraphChannel> channels,
  required Set<String> selectedKeys,
}) {
  final copied = easeClipboard;
  if (copied.isEmpty) return false;
  final period = copied.length - 1;
  final edits = <GraphChannel, BridgeScalar>{};
  for (final channel in channels) {
    final keys = channel.keys;
    final picked = [
      for (var i = 0; i < keys.length; i++)
        if (selectedKeys.contains('${channel.id}#$i')) i,
    ];
    if (picked.isEmpty) continue;
    final next = [...keys];
    for (var p = 0; p < picked.length; p++) {
      // Which copied key lends this one its way in and its way out.
      final into = period == 0 || p == 0 ? 0 : (p - 1) % period + 1;
      final out = period == 0 || p == picked.length - 1 ? into : p % period;
      final key = keys[picked[p]];
      next[picked[p]] = BridgeKeyframe(
        time: key.time,
        value: key.value,
        interpIn: copied[into].$1,
        interpOut: copied[out].$2,
      );
    }
    edits[channel] = BridgeScalar.keyframed(next);
  }
  if (edits.isEmpty) return false;
  asOneUndoStep(project, () => commitChannelEdits(edits));
  return true;
}
