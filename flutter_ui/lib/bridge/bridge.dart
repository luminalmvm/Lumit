// The Dart half of bridge v0 (docs/flutter-port/03-ARCHITECTURE.md "Bridge
// v0"): a thin dart:ffi wrapper over the `lumit_bridge` shared library. Dart
// calls the crate's C functions, each of which returns a Rust-owned UTF-8 JSON
// string; this side copies the string out, immediately frees it back to Rust,
// and decodes the JSON into typed Dart classes.
//
// The whole frontend must work WITHOUT the library present: `tryLoad` returns
// null when the `.dll` cannot be found or bound, and the app keeps its
// placeholder behaviour. Nothing here is imported into a code path that runs
// before a successful `tryLoad`, so the tests (which never load the library)
// stay green.

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

/// The kind of a project item, mirroring `lumit_core::model::ProjectItem`.
/// `unknown` covers a kind string a newer engine might add — drawn quietly
/// rather than crashing.
enum BridgeItemKind { footage, folder, composition, solid, unknown }

BridgeItemKind _kindOf(Object? raw) => switch (raw) {
      'footage' => BridgeItemKind.footage,
      'folder' => BridgeItemKind.folder,
      'composition' => BridgeItemKind.composition,
      'solid' => BridgeItemKind.solid,
      _ => BridgeItemKind.unknown,
    };

/// The kind of a composition layer, mirroring `lumit_core::model::LayerKind`.
/// `unknown` degrades a variant a newer engine might add.
enum BridgeLayerKind {
  footage,
  solid,
  precomp,
  text,
  camera,
  sequence,
  adjustment,
  unknown,
}

BridgeLayerKind _layerKindOf(Object? raw) => switch (raw) {
      'footage' => BridgeLayerKind.footage,
      'solid' => BridgeLayerKind.solid,
      'precomp' => BridgeLayerKind.precomp,
      'text' => BridgeLayerKind.text,
      'camera' => BridgeLayerKind.camera,
      'sequence' => BridgeLayerKind.sequence,
      'adjustment' => BridgeLayerKind.adjustment,
      _ => BridgeLayerKind.unknown,
    };

/// A footage item's probe status, mirroring the bridge's `status` field.
/// `unprobed` is the state before probing (or a `--no-default-features` build
/// that never probes); `unknown` degrades a status a newer engine might add.
enum BridgeMediaStatus { ok, missing, unprobed, failed, unknown }

BridgeMediaStatus _statusOf(Object? raw) => switch (raw) {
      'ok' => BridgeMediaStatus.ok,
      'missing' => BridgeMediaStatus.missing,
      'unprobed' => BridgeMediaStatus.unprobed,
      'failed' => BridgeMediaStatus.failed,
      _ => BridgeMediaStatus.unknown,
    };

int _asInt(Object? raw, [int fallback = 0]) =>
    raw is num ? raw.toInt() : fallback;

double _asDouble(Object? raw, [double fallback = 0]) =>
    raw is num ? raw.toDouble() : fallback;

/// An exact rational frame rate, `{num, den}` as the engine stores it (e.g.
/// 60000/1001). [fps] is the convenience double for display only.
class BridgeFps {
  final int num;
  final int den;

  const BridgeFps(this.num, this.den);

  double get fps => den == 0 ? 0 : num / den;

  factory BridgeFps.fromJson(Map<String, dynamic> m) =>
      BridgeFps(_asInt(m['num']), _asInt(m['den'], 1));
}

/// A layer's switches, mirroring `lumit_core::model::Switches` field-for-field.
/// The on-by-default switches (visible/audible/fx) default to true when a field
/// is absent, matching the model's serde defaults.
class BridgeSwitches {
  final bool visible;
  final bool audible;
  final bool locked;
  final bool threeD;
  final bool collapse;
  final bool fx;
  final bool solo;
  final bool motionBlur;

  const BridgeSwitches({
    required this.visible,
    required this.audible,
    required this.locked,
    required this.threeD,
    required this.collapse,
    required this.fx,
    required this.solo,
    required this.motionBlur,
  });

  factory BridgeSwitches.fromJson(Map<String, dynamic> m) => BridgeSwitches(
        visible: m['visible'] is bool ? m['visible'] as bool : true,
        audible: m['audible'] is bool ? m['audible'] as bool : true,
        locked: m['locked'] == true,
        threeD: m['three_d'] == true,
        collapse: m['collapse'] == true,
        fx: m['fx'] is bool ? m['fx'] as bool : true,
        solo: m['solo'] == true,
        motionBlur: m['motion_blur'] == true,
      );
}

/// One side of a keyframe's Bezier tangent (snapshot v4): its [speed] (the
/// tangent slope) and [influence] (how far the handle reaches, 0..1). Present on
/// a keyframe side only when its interpolation is `Bezier`.
class BridgeBezier {
  final double speed;
  final double influence;

  const BridgeBezier({required this.speed, required this.influence});

  factory BridgeBezier.fromJson(Map<String, dynamic> m) => BridgeBezier(
        speed: _asDouble(m['speed']),
        influence: _asDouble(m['influence']),
      );
}

/// One keyframe of a transform property (snapshot v3). `frame` is the comp frame
/// it lands on; `interpIn`/`interpOut` are the engine's `SideInterp` variant
/// names (`Hold`, `Linear`, `Bezier`). Snapshot v4 adds [bezierIn]/[bezierOut],
/// present on a side only when its interpolation is `Bezier`.
class BridgeKeyframe {
  final int frame;
  final double value;
  final String interpIn;
  final String interpOut;
  final BridgeBezier? bezierIn;
  final BridgeBezier? bezierOut;

  const BridgeKeyframe({
    required this.frame,
    required this.value,
    required this.interpIn,
    required this.interpOut,
    this.bezierIn,
    this.bezierOut,
  });

  factory BridgeKeyframe.fromJson(Map<String, dynamic> m) => BridgeKeyframe(
        frame: _asInt(m['frame']),
        value: _asDouble(m['value']),
        interpIn: m['interp_in'] is String ? m['interp_in'] as String : 'Linear',
        interpOut:
            m['interp_out'] is String ? m['interp_out'] as String : 'Linear',
        bezierIn: m['bezier_in'] is Map
            ? BridgeBezier.fromJson((m['bezier_in'] as Map).cast<String, dynamic>())
            : null,
        bezierOut: m['bezier_out'] is Map
            ? BridgeBezier.fromJson(
                (m['bezier_out'] as Map).cast<String, dynamic>())
            : null,
      );
}

/// One transform property's read-back (snapshot v3): its current [value] (the
/// static value, or the value at frame 0 when keyframed), whether it is
/// [animated], and — when animated — its [keys].
class BridgeTransformProperty {
  final double value;
  final bool animated;
  final List<BridgeKeyframe> keys;

  const BridgeTransformProperty({
    required this.value,
    required this.animated,
    required this.keys,
  });

  factory BridgeTransformProperty.fromJson(Map<String, dynamic> m) {
    final keys = <BridgeKeyframe>[];
    final rawKeys = m['keys'];
    if (rawKeys is List) {
      for (final k in rawKeys) {
        if (k is Map) keys.add(BridgeKeyframe.fromJson(k.cast<String, dynamic>()));
      }
    }
    return BridgeTransformProperty(
      value: _asDouble(m['value']),
      animated: m['animated'] == true,
      keys: keys,
    );
  }
}

/// A layer's whole transform read-back (snapshot v3): one
/// [BridgeTransformProperty] per snake_case property name (`anchor_x`…`opacity`).
class BridgeTransform {
  final Map<String, BridgeTransformProperty> properties;

  const BridgeTransform(this.properties);

  /// The property named [name] (e.g. `position_x`), or null if absent.
  BridgeTransformProperty? operator [](String name) => properties[name];

  factory BridgeTransform.fromJson(Map<String, dynamic> m) {
    final props = <String, BridgeTransformProperty>{};
    m.forEach((key, value) {
      if (value is Map) {
        props[key] =
            BridgeTransformProperty.fromJson(value.cast<String, dynamic>());
      }
    });
    return BridgeTransform(props);
  }
}

/// One effect parameter's read-back (snapshot v3). [kind] is a tag
/// (`scalar`/`colour`/`enum`/`bool`/`seed`/`point`/`file`/`layer`); [value] is
/// the decoded value (a double for scalar, a `List<double>` for colour, etc.),
/// or null for a kind the bridge does not yet surface.
/// A parameter's declared edit range (snapshot v5): the hard [min]/[max] (either
/// nullable), the soft [sliderMin]/[sliderMax] a drag rides between (Float only),
/// and the enum [options] (Choice only). Absent for kinds with no numeric range
/// (bool/seed/point/file/layer). The drag controls clamp to [min]/[max] and pace
/// their sensitivity by the slider span; before v5 these were unclamped.
class BridgeParamRange {
  final double? min;
  final double? max;
  final double? sliderMin;
  final double? sliderMax;
  final List<String> options;

  const BridgeParamRange({
    this.min,
    this.max,
    this.sliderMin,
    this.sliderMax,
    this.options = const [],
  });

  factory BridgeParamRange.fromJson(Map<String, dynamic> m) {
    final rawOptions = m['options'];
    return BridgeParamRange(
      min: m['min'] is num ? (m['min'] as num).toDouble() : null,
      max: m['max'] is num ? (m['max'] as num).toDouble() : null,
      sliderMin: m['slider_min'] is num
          ? (m['slider_min'] as num).toDouble()
          : null,
      sliderMax: m['slider_max'] is num
          ? (m['slider_max'] as num).toDouble()
          : null,
      options: rawOptions is List
          ? rawOptions.whereType<String>().toList()
          : const [],
    );
  }
}

class BridgeEffectParam {
  final String name;
  final String kind;
  final Object? value;

  /// The declared edit range (snapshot v5), or null for a param kind that has
  /// none (or an older library that does not carry it).
  final BridgeParamRange? range;

  /// Whether this parameter is keyframed (snapshot v0.9) — the stopwatch state
  /// for the Effect controls. False for a non-animatable kind or an older
  /// library that does not carry the flag.
  final bool animated;

  /// A scalar parameter's keyframes (snapshot v0.9), empty when not animated.
  final List<BridgeKeyframe> keys;

  /// A colour/point parameter's per-channel keyframes (snapshot v0.9), keyed by
  /// the channel name (`keys_r`/`keys_g`/`keys_b`/`keys_a` for a colour,
  /// `keys_x`/`keys_y` for a point). Empty for a scalar or non-animated param.
  final Map<String, List<BridgeKeyframe>> channelKeys;

  const BridgeEffectParam({
    required this.name,
    required this.kind,
    required this.value,
    this.range,
    this.animated = false,
    this.keys = const [],
    this.channelKeys = const {},
  });

  factory BridgeEffectParam.fromJson(Map<String, dynamic> m) {
    final rawRange = m['range'];
    List<BridgeKeyframe> parseKeys(Object? raw) {
      final out = <BridgeKeyframe>[];
      if (raw is List) {
        for (final k in raw) {
          if (k is Map) out.add(BridgeKeyframe.fromJson(k.cast<String, dynamic>()));
        }
      }
      return out;
    }

    final channelKeys = <String, List<BridgeKeyframe>>{};
    for (final field in const [
      'keys_r',
      'keys_g',
      'keys_b',
      'keys_a',
      'keys_x',
      'keys_y',
    ]) {
      final parsed = parseKeys(m[field]);
      if (parsed.isNotEmpty) channelKeys[field] = parsed;
    }
    return BridgeEffectParam(
      name: m['name'] is String ? m['name'] as String : '',
      kind: m['kind'] is String ? m['kind'] as String : 'unknown',
      value: m['value'],
      range: rawRange is Map
          ? BridgeParamRange.fromJson(rawRange.cast<String, dynamic>())
          : null,
      animated: m['animated'] == true,
      keys: parseKeys(m['keys']),
      channelKeys: channelKeys,
    );
  }
}

/// One effect instance in a layer's stack (snapshot v3). Snapshot v0.9 adds the
/// full effect identity — the [namespace] (`builtin`/`ofx`/`lfx`/`placeholder`)
/// and [version] alongside the match [name] — plus [sampleTemporally], so a
/// `.lumfx` preset round-trips byte-faithfully.
class BridgeEffect {
  final String id;
  final String name;
  final bool enabled;
  final List<BridgeEffectParam> params;

  /// The effect's namespace (snapshot v0.9), defaulting to `builtin` for an
  /// older library that does not carry it.
  final String namespace;

  /// The effect's schema version (snapshot v0.9); 0 for an older library.
  final int version;

  /// Whether a temporal re-render effect re-evaluates this effect at each
  /// sub-frame (snapshot v0.9); defaults true (the model default).
  final bool sampleTemporally;

  const BridgeEffect({
    required this.id,
    required this.name,
    required this.enabled,
    required this.params,
    this.namespace = 'builtin',
    this.version = 0,
    this.sampleTemporally = true,
  });

  factory BridgeEffect.fromJson(Map<String, dynamic> m) {
    final params = <BridgeEffectParam>[];
    final rawParams = m['params'];
    if (rawParams is List) {
      for (final p in rawParams) {
        if (p is Map) {
          params.add(BridgeEffectParam.fromJson(p.cast<String, dynamic>()));
        }
      }
    }
    return BridgeEffect(
      id: m['id'] is String ? m['id'] as String : '',
      name: m['name'] is String ? m['name'] as String : '',
      enabled: m['enabled'] == true,
      params: params,
      namespace: m['namespace'] is String ? m['namespace'] as String : 'builtin',
      version: _asInt(m['version']),
      sampleTemporally: m['sample_temporally'] != false,
    );
  }
}

/// One entry in the effect registry (`listEffects`): a stable [name] (the match
/// name an op takes), its sentence-case [label], and its [category] (a stable
/// machine key the Effects browser groups by, e.g. `blur_sharpen`) with its
/// sentence-case [categoryLabel] heading (snapshot v5). An older library carries
/// no category, so both default to empty (the browser then lists flat).
class BridgeEffectInfo {
  final String name;
  final String label;
  final String category;
  final String categoryLabel;

  const BridgeEffectInfo({
    required this.name,
    required this.label,
    this.category = '',
    this.categoryLabel = '',
  });

  factory BridgeEffectInfo.fromJson(Map<String, dynamic> m) => BridgeEffectInfo(
        name: m['name'] is String ? m['name'] as String : '',
        label: m['label'] is String ? m['label'] as String : '',
        category: m['category'] is String ? m['category'] as String : '',
        categoryLabel:
            m['category_label'] is String ? m['category_label'] as String : '',
      );
}

/// One rotating autosave beside a project (`listAutosaves`): its [slot] (1 =
/// newest) and absolute [path].
class BridgeAutosave {
  final int slot;
  final String path;

  const BridgeAutosave({required this.slot, required this.path});

  factory BridgeAutosave.fromJson(Map<String, dynamic> m) => BridgeAutosave(
        slot: _asInt(m['slot']),
        path: m['path'] is String ? m['path'] as String : '',
      );
}

/// A layer's matte (snapshot v4): the [source] layer id, the [channel]
/// (`alpha`/`luma`), whether it is [inverted], and the [sourceMode] (how the
/// matte samples its source: `none`/`masks`/`effects_and_masks`).
class BridgeMatte {
  final String source;
  final String channel;
  final bool inverted;
  final String sourceMode;

  const BridgeMatte({
    required this.source,
    required this.channel,
    required this.inverted,
    required this.sourceMode,
  });

  factory BridgeMatte.fromJson(Map<String, dynamic> m) => BridgeMatte(
        source: m['source'] is String ? m['source'] as String : '',
        channel: m['channel'] is String ? m['channel'] as String : 'alpha',
        inverted: m['inverted'] == true,
        sourceMode: m['source_mode'] is String
            ? m['source_mode'] as String
            : 'effects_and_masks',
      );
}

/// One boundary of a Retime store (snapshot v4): its local time as a comp
/// [tFrame] (and the durable [tSeconds]), the [sSeconds] source position it
/// pins, and whether it is [smooth] (edits keep the speed equal across it).
class BridgeRetimeBoundary {
  final int tFrame;
  final double tSeconds;
  final double sSeconds;
  final bool smooth;

  const BridgeRetimeBoundary({
    required this.tFrame,
    required this.tSeconds,
    required this.sSeconds,
    required this.smooth,
  });

  factory BridgeRetimeBoundary.fromJson(Map<String, dynamic> m) =>
      BridgeRetimeBoundary(
        tFrame: _asInt(m['t_frame']),
        tSeconds: _asDouble(m['t_seconds']),
        sSeconds: _asDouble(m['s_seconds']),
        smooth: m['smooth'] == true,
      );
}

/// One segment of a Retime store (snapshot v4). [kind] is `rate` or `map`. A
/// `rate` segment carries [v0]/[v1] (speeds, 1 = 100%) and an [ease] name
/// (`Linear`/`Slow`/`Fast`/`Smooth`/`Sharp`); a `map` segment carries
/// [m0]/[m1]/[b0]/[b1] (the cubic handle description). Absent fields are null.
class BridgeRetimeSegment {
  final String kind;
  final double? v0;
  final double? v1;
  final String? ease;
  final double? m0;
  final double? m1;
  final double? b0;
  final double? b1;

  const BridgeRetimeSegment({
    required this.kind,
    this.v0,
    this.v1,
    this.ease,
    this.m0,
    this.m1,
    this.b0,
    this.b1,
  });

  factory BridgeRetimeSegment.fromJson(Map<String, dynamic> m) {
    double? d(Object? raw) => raw is num ? raw.toDouble() : null;
    return BridgeRetimeSegment(
      kind: m['kind'] is String ? m['kind'] as String : 'rate',
      v0: d(m['v0']),
      v1: d(m['v1']),
      ease: m['ease'] is String ? m['ease'] as String : null,
      m0: d(m['m0']),
      m1: d(m['m1']),
      b0: d(m['b0']),
      b1: d(m['b1']),
    );
  }
}

/// A footage layer's Retime store (snapshot v4): whether reverse is allowed, the
/// frame [interpolation] policy (`nearest`/`blend`/`flow`), the [boundaries]
/// (n + 1) and the [segments] (n). Segment `i` spans `boundaries[i]..[i+1]`.
class BridgeRetime {
  final bool reverse;
  final String interpolation;
  final List<BridgeRetimeBoundary> boundaries;
  final List<BridgeRetimeSegment> segments;

  const BridgeRetime({
    required this.reverse,
    required this.interpolation,
    required this.boundaries,
    required this.segments,
  });

  factory BridgeRetime.fromJson(Map<String, dynamic> m) {
    final boundaries = <BridgeRetimeBoundary>[];
    final rawB = m['boundaries'];
    if (rawB is List) {
      for (final b in rawB) {
        if (b is Map) {
          boundaries
              .add(BridgeRetimeBoundary.fromJson(b.cast<String, dynamic>()));
        }
      }
    }
    final segments = <BridgeRetimeSegment>[];
    final rawS = m['segments'];
    if (rawS is List) {
      for (final s in rawS) {
        if (s is Map) {
          segments.add(BridgeRetimeSegment.fromJson(s.cast<String, dynamic>()));
        }
      }
    }
    return BridgeRetime(
      reverse: m['reverse'] == true,
      interpolation: m['interpolation'] is String
          ? m['interpolation'] as String
          : 'nearest',
      boundaries: boundaries,
      segments: segments,
    );
  }
}

/// A composition's motion-blur master (snapshot v4): the [enabled] master, the
/// shutter [angle] and [phase] in degrees, and the sub-frame [samples] count.
class BridgeMotionBlur {
  final bool enabled;
  final double angle;
  final double phase;
  final int samples;

  const BridgeMotionBlur({
    required this.enabled,
    required this.angle,
    required this.phase,
    required this.samples,
  });

  factory BridgeMotionBlur.fromJson(Map<String, dynamic> m) => BridgeMotionBlur(
        enabled: m['enabled'] == true,
        angle: _asDouble(m['shutter_angle']),
        phase: _asDouble(m['shutter_phase']),
        samples: _asInt(m['samples']),
      );
}

/// One entry in the blend-mode registry (`listBlendModes`): a stable [name] (the
/// serde variant name the op takes) and its sentence-case [label].
class BridgeBlendMode {
  final String name;
  final String label;

  const BridgeBlendMode({required this.name, required this.label});

  factory BridgeBlendMode.fromJson(Map<String, dynamic> m) => BridgeBlendMode(
        name: m['name'] is String ? m['name'] as String : '',
        label: m['label'] is String ? m['label'] as String : '',
      );
}

/// The dialogue fields a delivery preset stamps (`exportPreset`), mirroring
/// `ExportDialogState::apply`: the [codec] (`h264`/`hevc`), the delivery [size]
/// (`[w, h]`, or null for the comp's own size), the [bitrateMbps] as typed
/// (empty for the encoder's default quality), the [includeAudio] default, and
/// the suggested [defaultName].
class BridgeExportPreset {
  final String preset;
  final String codec;
  final List<int>? size;
  final String bitrateMbps;
  final bool includeAudio;
  final String defaultName;

  const BridgeExportPreset({
    required this.preset,
    required this.codec,
    required this.size,
    required this.bitrateMbps,
    required this.includeAudio,
    required this.defaultName,
  });

  factory BridgeExportPreset.fromJson(Map<String, dynamic> m) {
    List<int>? size;
    final rawSize = m['size'];
    if (rawSize is List && rawSize.length == 2) {
      size = [_asInt(rawSize[0]), _asInt(rawSize[1])];
    }
    return BridgeExportPreset(
      preset: m['preset'] is String ? m['preset'] as String : 'custom',
      codec: m['codec'] is String ? m['codec'] as String : 'h264',
      size: size,
      bitrateMbps: m['bitrate_mbps'] is String ? m['bitrate_mbps'] as String : '',
      includeAudio: m['include_audio'] != false,
      defaultName:
          m['default_name'] is String ? m['default_name'] as String : 'export.mp4',
    );
  }

  /// The default fields (no library / a parse failure): custom, comp size.
  static const idle = BridgeExportPreset(
    preset: 'custom',
    codec: 'h264',
    size: null,
    bitrateMbps: '',
    includeAudio: true,
    defaultName: 'export.mp4',
  );
}

/// The state of the one running export (`exportPoll`), mirroring the bridge's
/// poll reply. [state] is `idle`/`running`/`done`/`failed`; [frame]/[total] are
/// the progress counters; [encoder] is the encoder the ladder settled on (once
/// known); [path] is set on `done`; [error] is set on `failed`.
class BridgeExportState {
  final String state;
  final int frame;
  final int total;
  final String? encoder;
  final String? path;
  final String? error;

  const BridgeExportState({
    required this.state,
    this.frame = 0,
    this.total = 0,
    this.encoder,
    this.path,
    this.error,
  });

  bool get isRunning => state == 'running';
  bool get isDone => state == 'done';
  bool get isFailed => state == 'failed';

  /// The state before anything has run (or with no library).
  static const idle = BridgeExportState(state: 'idle');

  factory BridgeExportState.fromJson(Map<String, dynamic> m) => BridgeExportState(
        state: m['state'] is String ? m['state'] as String : 'idle',
        frame: _asInt(m['frame']),
        total: _asInt(m['total']),
        encoder: m['encoder'] is String ? m['encoder'] as String : null,
        path: m['path'] is String ? m['path'] as String : null,
        error: m['error'] is String ? m['error'] as String : null,
      );
}

/// One marker with its kind (snapshot v0.9): the comp [frame] it lands on, its
/// [kind] (`user`/`beat`/`chapter`), an optional beat [confidence] (0..1, only
/// on a beat marker), its [label] and, for a spanning marker, [durationFrames].
class BridgeMarker {
  final int frame;
  final String kind;
  final double? confidence;
  final String label;
  final int? durationFrames;

  const BridgeMarker({
    required this.frame,
    required this.kind,
    this.confidence,
    this.label = '',
    this.durationFrames,
  });

  bool get isBeat => kind == 'beat';

  factory BridgeMarker.fromJson(Map<String, dynamic> m) => BridgeMarker(
        frame: _asInt(m['frame']),
        kind: m['kind'] is String ? m['kind'] as String : 'user',
        confidence:
            m['confidence'] is num ? (m['confidence'] as num).toDouble() : null,
        label: m['label'] is String ? m['label'] as String : '',
        durationFrames:
            m['duration_frames'] is num ? _asInt(m['duration_frames']) : null,
      );
}

/// A text layer's document read-back (snapshot v0.9): the [content], the pixel
/// [size] at natural scale, and the scene-linear RGBA [fill].
class BridgeTextDocument {
  final String content;
  final double size;
  final List<double> fill;

  const BridgeTextDocument({
    required this.content,
    required this.size,
    required this.fill,
  });

  factory BridgeTextDocument.fromJson(Map<String, dynamic> m) {
    final rawFill = m['fill'];
    final fill = rawFill is List
        ? [for (final c in rawFill) _asDouble(c)]
        : const <double>[0, 0, 0, 1];
    return BridgeTextDocument(
      content: m['content'] is String ? m['content'] as String : '',
      size: _asDouble(m['size']),
      fill: fill,
    );
  }
}

/// One clip on a Sequence layer (snapshot v0.9). [id] is stable (ops address a
/// clip by it); [sourceKind] is `footage`/`comp` and [sourceId] names it;
/// [placeStartFrame]/[placeEndFrame] are the clip's span on the comp timeline,
/// with the source trim [sourceInSecs]/[sourceOutSecs] and the clip's own
/// [retime] carried in seconds.
class BridgeClip {
  final String id;
  final String sourceKind;
  final String sourceId;
  final double sourceInSecs;
  final double sourceOutSecs;
  final int placeStartFrame;
  final int placeEndFrame;
  final double placeStartSecs;
  final double placeDurationSecs;
  final BridgeRetime? retime;

  const BridgeClip({
    required this.id,
    required this.sourceKind,
    required this.sourceId,
    required this.sourceInSecs,
    required this.sourceOutSecs,
    required this.placeStartFrame,
    required this.placeEndFrame,
    required this.placeStartSecs,
    required this.placeDurationSecs,
    this.retime,
  });

  factory BridgeClip.fromJson(Map<String, dynamic> m) => BridgeClip(
        id: m['id'] is String ? m['id'] as String : '',
        sourceKind: m['source_kind'] is String ? m['source_kind'] as String : 'footage',
        sourceId: m['source_id'] is String ? m['source_id'] as String : '',
        sourceInSecs: _asDouble(m['source_in_secs']),
        sourceOutSecs: _asDouble(m['source_out_secs']),
        placeStartFrame: _asInt(m['place_start_frame']),
        placeEndFrame: _asInt(m['place_end_frame']),
        placeStartSecs: _asDouble(m['place_start_secs']),
        placeDurationSecs: _asDouble(m['place_duration_secs']),
        retime: m['retime'] is Map
            ? BridgeRetime.fromJson((m['retime'] as Map).cast<String, dynamic>())
            : null,
      );
}

/// The realtime preview tier (`playbackTier`/`resetRealtime`, ABI 9): the
/// current [tier] (1 = Full, 2 = Half, 3 = Third, 4 = Quarter) and its [scale]
/// (`1/tier`). In Auto resolution mode the Viewer renders the next frame at
/// [scale] and shows the tier; a manual resolution pick ignores it. [full] is
/// the idle default (Full, scale 1) — also what an older library reports.
class BridgePlaybackTier {
  final int tier;
  final double scale;

  const BridgePlaybackTier({required this.tier, required this.scale});

  static const full = BridgePlaybackTier(tier: 1, scale: 1.0);

  /// The tier name for the Viewer readout ("Full"/"Half"/"Third"/"Quarter").
  String get label => switch (tier) {
        1 => 'Full',
        2 => 'Half',
        3 => 'Third',
        4 => 'Quarter',
        _ => 'Full',
      };

  factory BridgePlaybackTier.fromJson(Map<String, dynamic> m) =>
      BridgePlaybackTier(
        tier: m['tier'] is num ? (m['tier'] as num).toInt() : 1,
        scale: m['scale'] is num ? (m['scale'] as num).toDouble() : 1.0,
      );
}

/// One composition layer as the Timeline reads it. `inFrame`/`outFrame` are comp
/// frames derived from the comp's own rate; `index` is the stack position
/// (0 = top). Snapshot v3 adds the [transform] read-back, the [effects] stack,
/// and the identity links ([sourceItemId], [sourceCompId], [colour]). Snapshot
/// v4 adds the [blendMode], [matte], [parent] columns and a footage layer's
/// [retime]. Snapshot v0.9 adds [startOffsetFrame]/[startOffsetSecs]/[inSecs]/
/// [outSecs] (the overrun-hatch ingredients), the asset read-back ([text],
/// [cameraZoom], [solidSize]) and a sequence layer's [clips].
class BridgeLayer {
  final String id;
  final int index;
  final String name;
  final BridgeLayerKind kind;
  final int inFrame;
  final int outFrame;
  final int label;
  final BridgeSwitches switches;

  /// The transform read-back (snapshot v3), or null for an older engine.
  final BridgeTransform? transform;

  /// The effect stack (snapshot v3); empty when the layer has no effects.
  final List<BridgeEffect> effects;

  /// A footage layer's source item id, else null.
  final String? sourceItemId;

  /// A precomp layer's source composition id, else null.
  final String? sourceCompId;

  /// A solid layer's scene-linear RGBA, else null.
  final List<double>? colour;

  /// The blend mode (serde variant name, e.g. `Normal`), or null for an older
  /// engine.
  final String? blendMode;

  /// The layer's matte (snapshot v4), or null when it has none.
  final BridgeMatte? matte;

  /// The transform parent layer id (snapshot v4), or null when unparented.
  final String? parent;

  /// A footage layer's Retime store (snapshot v4), or null when it plays at
  /// source rate.
  final BridgeRetime? retime;

  /// Where layer time 0 sits on the comp timeline, as a comp frame and in
  /// seconds (snapshot v0.9). With [inSecs]/[outSecs] these are the overrun HOLD
  /// hatch ingredients the frame-only read-back lacked.
  final int startOffsetFrame;
  final double startOffsetSecs;
  final double inSecs;
  final double outSecs;

  /// A text layer's document read-back (snapshot v0.9), else null.
  final BridgeTextDocument? text;

  /// A camera layer's zoom read-back as a property (snapshot v0.9), else null.
  final BridgeTransformProperty? cameraZoom;

  /// A solid layer's `[width, height]` (snapshot v0.9), else null.
  final List<int>? solidSize;

  /// A sequence layer's clips (snapshot v0.9); empty for other kinds.
  final List<BridgeClip> clips;

  const BridgeLayer({
    required this.id,
    required this.index,
    required this.name,
    required this.kind,
    required this.inFrame,
    required this.outFrame,
    required this.label,
    required this.switches,
    this.transform,
    this.effects = const [],
    this.sourceItemId,
    this.sourceCompId,
    this.colour,
    this.blendMode,
    this.matte,
    this.parent,
    this.retime,
    this.startOffsetFrame = 0,
    this.startOffsetSecs = 0,
    this.inSecs = 0,
    this.outSecs = 0,
    this.text,
    this.cameraZoom,
    this.solidSize,
    this.clips = const [],
  });

  factory BridgeLayer.fromJson(Map<String, dynamic> m) {
    final effects = <BridgeEffect>[];
    final rawEffects = m['effects'];
    if (rawEffects is List) {
      for (final e in rawEffects) {
        if (e is Map) effects.add(BridgeEffect.fromJson(e.cast<String, dynamic>()));
      }
    }
    List<double>? colour;
    final rawColour = m['colour'];
    if (rawColour is List) {
      colour = [for (final c in rawColour) _asDouble(c)];
    }
    return BridgeLayer(
      id: m['id'] is String ? m['id'] as String : '',
      index: _asInt(m['index']),
      name: m['name'] is String ? m['name'] as String : '',
      kind: _layerKindOf(m['kind']),
      inFrame: _asInt(m['in_frame']),
      outFrame: _asInt(m['out_frame']),
      label: _asInt(m['label']),
      switches: m['switches'] is Map
          ? BridgeSwitches.fromJson(
              (m['switches'] as Map).cast<String, dynamic>())
          : const BridgeSwitches(
              visible: true,
              audible: true,
              locked: false,
              threeD: false,
              collapse: false,
              fx: true,
              solo: false,
              motionBlur: false,
            ),
      transform: m['transform'] is Map
          ? BridgeTransform.fromJson(
              (m['transform'] as Map).cast<String, dynamic>())
          : null,
      effects: effects,
      sourceItemId: m['source_item_id'] is String
          ? m['source_item_id'] as String
          : null,
      sourceCompId: m['source_comp_id'] is String
          ? m['source_comp_id'] as String
          : null,
      colour: colour,
      blendMode: m['blend_mode'] is String ? m['blend_mode'] as String : null,
      matte: m['matte'] is Map
          ? BridgeMatte.fromJson((m['matte'] as Map).cast<String, dynamic>())
          : null,
      parent: m['parent'] is String ? m['parent'] as String : null,
      retime: m['retime'] is Map
          ? BridgeRetime.fromJson((m['retime'] as Map).cast<String, dynamic>())
          : null,
      startOffsetFrame: _asInt(m['start_offset_frame']),
      startOffsetSecs: _asDouble(m['start_offset_secs']),
      inSecs: _asDouble(m['in_secs']),
      outSecs: _asDouble(m['out_secs']),
      text: m['text'] is Map
          ? BridgeTextDocument.fromJson((m['text'] as Map).cast<String, dynamic>())
          : null,
      cameraZoom: m['camera'] is Map
          ? BridgeTransformProperty.fromJson(
              (m['camera'] as Map).cast<String, dynamic>())
          : null,
      solidSize: m['solid_size'] is List && (m['solid_size'] as List).length == 2
          ? [
              _asInt((m['solid_size'] as List)[0]),
              _asInt((m['solid_size'] as List)[1]),
            ]
          : null,
      clips: () {
        final out = <BridgeClip>[];
        final raw = m['clips'];
        if (raw is List) {
          for (final c in raw) {
            if (c is Map) out.add(BridgeClip.fromJson(c.cast<String, dynamic>()));
          }
        }
        return out;
      }(),
    );
  }
}

/// A composition's detail: size, frame rate, derived frame count, layers (top
/// first) and marker frames.
class BridgeComp {
  final int width;
  final int height;
  final BridgeFps fps;
  final int frameCount;
  final List<BridgeLayer> layers;
  final List<int> markers;

  /// The markers with their kind (snapshot v0.9): the same markers as [markers]
  /// but carrying `user`/`beat`/`chapter` and a beat's confidence, so beat
  /// markers can be drawn apart. Empty for an older library (fall back to
  /// [markers]).
  final List<BridgeMarker> markerDetails;

  /// The work area as `[inFrame, outFrame]` (snapshot v3), or null for the full
  /// comp — the preview/export span the B/N keys set.
  final List<int>? workArea;

  /// The comp motion-blur master (snapshot v4), or null for an older engine.
  final BridgeMotionBlur? motionBlur;

  const BridgeComp({
    required this.width,
    required this.height,
    required this.fps,
    required this.frameCount,
    required this.layers,
    required this.markers,
    this.markerDetails = const [],
    this.workArea,
    this.motionBlur,
  });

  factory BridgeComp.fromJson(Map<String, dynamic> m) {
    final layers = <BridgeLayer>[];
    final rawLayers = m['layers'];
    if (rawLayers is List) {
      for (final l in rawLayers) {
        if (l is Map) {
          layers.add(BridgeLayer.fromJson(l.cast<String, dynamic>()));
        }
      }
    }
    final markers = <int>[];
    final rawMarkers = m['markers'];
    if (rawMarkers is List) {
      for (final frame in rawMarkers) {
        if (frame is num) markers.add(frame.toInt());
      }
    }
    final markerDetails = <BridgeMarker>[];
    final rawDetails = m['marker_details'];
    if (rawDetails is List) {
      for (final d in rawDetails) {
        if (d is Map) {
          markerDetails.add(BridgeMarker.fromJson(d.cast<String, dynamic>()));
        }
      }
    }
    List<int>? workArea;
    final rawWorkArea = m['work_area'];
    if (rawWorkArea is List && rawWorkArea.length == 2) {
      workArea = [_asInt(rawWorkArea[0]), _asInt(rawWorkArea[1])];
    }
    return BridgeComp(
      width: _asInt(m['width']),
      height: _asInt(m['height']),
      fps: m['fps'] is Map
          ? BridgeFps.fromJson((m['fps'] as Map).cast<String, dynamic>())
          : const BridgeFps(0, 1),
      frameCount: _asInt(m['frame_count']),
      layers: layers,
      markers: markers,
      markerDetails: markerDetails,
      workArea: workArea,
      motionBlur: m['motion_blur'] is Map
          ? BridgeMotionBlur.fromJson(
              (m['motion_blur'] as Map).cast<String, dynamic>())
          : null,
    );
  }
}

/// A footage item's probed media metadata, present once its status is `ok`.
class BridgeMedia {
  final int durationFrames;
  final BridgeFps fps;
  final int width;
  final int height;
  final bool audio;

  const BridgeMedia({
    required this.durationFrames,
    required this.fps,
    required this.width,
    required this.height,
    required this.audio,
  });

  factory BridgeMedia.fromJson(Map<String, dynamic> m) => BridgeMedia(
        durationFrames: _asInt(m['duration_frames']),
        fps: m['fps'] is Map
            ? BridgeFps.fromJson((m['fps'] as Map).cast<String, dynamic>())
            : const BridgeFps(0, 1),
        width: _asInt(m['width']),
        height: _asInt(m['height']),
        audio: m['audio'] == true,
      );

  /// Whether the container has a video stream — mirrors the bridge probe's own
  /// `(0, 1, 0, 0)` sentinel for "no video" (`media.rs::probe_path`), so an
  /// audio-only file reads false here.
  bool get hasVideo => width > 0 && height > 0;
}

/// A decoded footage frame: tightly-packed straight (non-premultiplied) RGBA8,
/// `width * height * 4` bytes. The bytes are copied out of the engine's buffer,
/// which is freed immediately, so this owns its pixels.
class DecodedFrame {
  final int width;
  final int height;
  final Uint8List rgba;

  const DecodedFrame({
    required this.width,
    required this.height,
    required this.rgba,
  });
}

/// A frame that stayed on the GPU (the zero-copy Viewer path, K-177). No pixels
/// cross — the runner opens the shared resource and Flutter samples it directly.
/// The identity is stable across frames (the same texture is re-used) and changes
/// only when the composition is resized.
///
/// Two platform shapes share this one type:
/// - **Windows** (DXGI shared handle): [handle] names the texture; the DMA-BUF
///   fields are null.
/// - **Linux** (Vulkan DMA-BUF): [fd] is the exported file descriptor, with
///   [stride], [offset], [fourcc] and [modifier] describing the buffer; [handle]
///   is 0. [isDmabuf] distinguishes them.
class SharedFrame {
  final int handle;
  final int width;
  final int height;

  /// The exported DMA-BUF file descriptor (Linux), or null on Windows.
  final int? fd;

  /// The DMA-BUF row stride in bytes (Linux), else null.
  final int? stride;

  /// The DMA-BUF plane offset in bytes (Linux), else null.
  final int? offset;

  /// The DRM fourcc of the DMA-BUF (Linux), else null.
  final int? fourcc;

  /// The DRM format modifier of the DMA-BUF (Linux; 0 = linear), else null.
  final int? modifier;

  const SharedFrame({
    required this.handle,
    required this.width,
    required this.height,
    this.fd,
    this.stride,
    this.offset,
    this.fourcc,
    this.modifier,
  });

  /// True for the Linux DMA-BUF shape (an exported [fd] is present).
  bool get isDmabuf => fd != null;
}

/// One node in the Project panel tree. Folders carry nested [children]; every
/// other kind carries an empty list. A composition additionally carries [comp]
/// (its size/layers/markers); a footage item carries its probe [status] and,
/// once probed, its [media] metadata.
class BridgeItem {
  final String id;
  final String name;
  final BridgeItemKind kind;
  final List<BridgeItem> children;

  /// Present for compositions (snapshot v2), else null.
  final BridgeComp? comp;

  /// Present for footage items once probed cleanly, else null.
  final BridgeMedia? media;

  /// Present for footage items (the probe status), else null.
  final BridgeMediaStatus? status;

  const BridgeItem({
    required this.id,
    required this.name,
    required this.kind,
    required this.children,
    this.comp,
    this.media,
    this.status,
  });

  factory BridgeItem.fromJson(Map<String, dynamic> m) {
    final rawChildren = m['children'];
    final children = <BridgeItem>[];
    if (rawChildren is List) {
      for (final child in rawChildren) {
        if (child is Map) {
          children.add(BridgeItem.fromJson(child.cast<String, dynamic>()));
        }
      }
    }
    return BridgeItem(
      id: m['id'] is String ? m['id'] as String : '',
      name: m['name'] is String ? m['name'] as String : '',
      kind: _kindOf(m['kind']),
      children: children,
      comp: m['comp'] is Map
          ? BridgeComp.fromJson((m['comp'] as Map).cast<String, dynamic>())
          : null,
      media: m['media'] is Map
          ? BridgeMedia.fromJson((m['media'] as Map).cast<String, dynamic>())
          : null,
      status: m.containsKey('status') ? _statusOf(m['status']) : null,
    );
  }
}

/// A decoded document snapshot — the `{"ok":true, …}` reply shape.
class BridgeSnapshot {
  final List<BridgeItem> items;
  final bool canUndo;
  final bool canRedo;

  /// The loaded/last-saved project path, or null for an unsaved document.
  final String? path;

  const BridgeSnapshot({
    required this.items,
    required this.canUndo,
    required this.canRedo,
    required this.path,
  });

  factory BridgeSnapshot.fromJson(Map<String, dynamic> m) {
    final rawItems = m['items'];
    final items = <BridgeItem>[];
    if (rawItems is List) {
      for (final item in rawItems) {
        if (item is Map) {
          items.add(BridgeItem.fromJson(item.cast<String, dynamic>()));
        }
      }
    }
    return BridgeSnapshot(
      items: items,
      canUndo: m['can_undo'] == true,
      canRedo: m['can_redo'] == true,
      path: m['path'] is String ? m['path'] as String : null,
    );
  }
}

/// The result of one bridge call: a snapshot on success, or a calm error string
/// for the status line on failure. Parsing a malformed reply is itself an
/// error, never a throw.
class BridgeReply {
  final BridgeSnapshot? snapshot;
  final String? error;

  /// The segment-to-Rate fit's drift in seconds, when the reply carried one
  /// (the →Rate op injects it as an additive top-level field).
  final double? driftSeconds;

  const BridgeReply.ok(this.snapshot, {this.driftSeconds}) : error = null;
  const BridgeReply.err(this.error)
      : snapshot = null,
        driftSeconds = null;

  bool get ok => error == null;

  /// Decode a reply string. `{"ok":true,…}` yields a snapshot; `{"ok":false,
  /// "error":"…"}` yields the error; anything else is reported as malformed.
  factory BridgeReply.parse(String raw) {
    Object? decoded;
    try {
      decoded = jsonDecode(raw);
    } catch (_) {
      return const BridgeReply.err('bridge returned malformed JSON');
    }
    if (decoded is! Map) {
      return const BridgeReply.err('bridge returned malformed JSON');
    }
    final map = decoded.cast<String, dynamic>();
    if (map['ok'] == true) {
      final drift = map['drift'];
      return BridgeReply.ok(
        BridgeSnapshot.fromJson(map),
        driftSeconds: drift is num ? drift.toDouble() : null,
      );
    }
    final err = map['error'];
    return BridgeReply.err(err is String ? err : 'bridge error');
  }
}

// The C signatures. Strings cross as `Pointer<Char>`; the engine allocates the
// replies and frees them through `lumit_bridge_free_string`.
typedef _NoArgC = Pointer<Char> Function();
typedef _NoArgDart = Pointer<Char> Function();
typedef _StrArgC = Pointer<Char> Function(Pointer<Char>);
typedef _StrArgDart = Pointer<Char> Function(Pointer<Char>);
typedef _FreeC = Void Function(Pointer<Char>);
typedef _FreeDart = void Function(Pointer<Char>);

// Snapshot-v2 op signatures (mixed argument types).
typedef _SwitchC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Bool);
typedef _SwitchDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, bool);
typedef _SpanC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Int64);
typedef _SpanDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, int);
typedef _TransformC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Double);
typedef _TransformDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, double);
typedef _MarkerC = Pointer<Char> Function(Pointer<Char>, Int64);
typedef _MarkerDart = Pointer<Char> Function(Pointer<Char>, int);

// Bridge v0.3 op signatures.
typedef _Str2C = Pointer<Char> Function(Pointer<Char>, Pointer<Char>);
typedef _Str2Dart = Pointer<Char> Function(Pointer<Char>, Pointer<Char>);
typedef _Str3C = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>);
typedef _Str3Dart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>);
typedef _CompSettingsC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Uint32, Uint32, Int64, Int64, Int64);
typedef _CompSettingsDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, int, int, int, int, int);
typedef _KeyframeC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Int64, Double);
typedef _KeyframeDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, int, double);
typedef _ShiftC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>, Int64);
typedef _ShiftDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>, int);
typedef _WorkAreaC = Pointer<Char> Function(Pointer<Char>, Int64, Bool);
typedef _WorkAreaDart = Pointer<Char> Function(Pointer<Char>, int, bool);
typedef _Str3BoolC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Bool);
typedef _Str3BoolDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, bool);
typedef _ScalarParamC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>, Double);
typedef _ScalarParamDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>, double);
typedef _ColourParamC = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Pointer<Char>, Pointer<Char>, Double, Double, Double, Double);
typedef _ColourParamDart = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Pointer<Char>, Pointer<Char>, double, double, double, double);

// Bridge v0.4 op signatures.
typedef _Str2BoolC = Pointer<Char> Function(Pointer<Char>, Pointer<Char>, Bool);
typedef _Str2BoolDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, bool);
typedef _Str2DoubleC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Double);
typedef _Str2DoubleDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, double);
typedef _Str2IntC = Pointer<Char> Function(Pointer<Char>, Pointer<Char>, Int64);
typedef _Str2IntDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, int);
typedef _Str2Int2C = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Int64, Int64);
typedef _Str2Int2Dart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, int, int);
typedef _SegmentPresetC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Int64, Pointer<Char>);
typedef _SegmentPresetDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, int, Pointer<Char>);
typedef _MatteC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>, Bool);
typedef _MatteDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>, bool);
typedef _MotionBlurC = Pointer<Char> Function(
    Pointer<Char>, Bool, Double, Double, Uint32);
typedef _MotionBlurDart = Pointer<Char> Function(
    Pointer<Char>, bool, double, double, int);
typedef _KeyframeInterpC = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Pointer<Char>, Int64, Pointer<Char>, Pointer<Char>, Double, Double, Double,
    Double);
typedef _KeyframeInterpDart = Pointer<Char> Function(Pointer<Char>,
    Pointer<Char>, Pointer<Char>, int, Pointer<Char>, Pointer<Char>, double,
    double, double, double);

// Frame decode: a raw RGBA8 buffer with its size written into out-pointers.
typedef _DecodeC = Pointer<Uint8> Function(
    Pointer<Char>, Uint64, Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);
typedef _DecodeDart = Pointer<Uint8> Function(
    Pointer<Char>, int, Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);
typedef _FreeBufferC = Void Function(Pointer<Uint8>, Size);
typedef _FreeBufferDart = void Function(Pointer<Uint8>, int);

// Composited-comp render: like decode, but keyed by comp id with a scale, and
// returning the whole composited frame (every layer/transform/effect) rather
// than one raw footage layer.
typedef _RenderC = Pointer<Uint8> Function(Pointer<Char>, Uint64, Float,
    Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);
typedef _RenderDart = Pointer<Uint8> Function(
    Pointer<Char>, int, double, Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);

// Zero-copy shared-texture path (K-177): no buffer returned — the frame stays on
// the GPU. `shared_supported` reports whether this build offers it; `render_to_shared`
// renders comp `id` at `frame` into a shared texture and writes the NT handle +
// dimensions into the out-pointers, returning true on success.
typedef _SharedSupportedC = Bool Function();
typedef _SharedSupportedDart = bool Function();
typedef _RenderSharedC = Bool Function(Pointer<Char>, Uint64, Pointer<Uint64>,
    Pointer<Uint32>, Pointer<Uint32>);
typedef _RenderSharedDart = bool Function(
    Pointer<Char>, int, Pointer<Uint64>, Pointer<Uint32>, Pointer<Uint32>);
// The Linux DMA-BUF sibling (K-177): renders comp `id` at `frame` into a Vulkan
// image exported as a DMA-BUF, writing the fd + DRM metadata into the
// out-pointers (fd, width, height, stride, offset, fourcc, modifier), returning
// true on success. A separate export from the Windows one so the Windows ABI is
// untouched; Dart picks the entry point by platform.
typedef _RenderSharedDmabufC = Bool Function(
    Pointer<Char>,
    Uint64,
    Pointer<Int32>,
    Pointer<Uint32>,
    Pointer<Uint32>,
    Pointer<Uint32>,
    Pointer<Uint32>,
    Pointer<Uint32>,
    Pointer<Uint64>);
typedef _RenderSharedDmabufDart = bool Function(
    Pointer<Char>,
    int,
    Pointer<Int32>,
    Pointer<Uint32>,
    Pointer<Uint32>,
    Pointer<Uint32>,
    Pointer<Uint32>,
    Pointer<Uint32>,
    Pointer<Uint64>);

// GPU scope pass (K-096 v1): like the comp render, but keyed by a scope kind
// and five packed 0x00RRGGBB colours, returning the fixed 256×256 RGBA8 trace.
typedef _RenderScopeC = Pointer<Uint8> Function(Uint32, Pointer<Char>, Uint64,
    Float, Uint32, Uint32, Uint32, Uint32, Uint32, Pointer<Size>);
typedef _RenderScopeDart = Pointer<Uint8> Function(int, Pointer<Char>, int,
    double, int, int, int, int, int, Pointer<Size>);

// Bridge v0.8 (ABI 8): cache controls, render cancellation, thumbnails. The
// cache controls and render_cancel_stale take/return JSON like the other ops;
// set_cache_budget and render_cancel_stale take a single u64. thumbnail mirrors
// decode but takes a u32 max-edge instead of a u64 frame.
typedef _U64ArgC = Pointer<Char> Function(Uint64);
typedef _U64ArgDart = Pointer<Char> Function(int);
typedef _ThumbC = Pointer<Uint8> Function(
    Pointer<Char>, Uint32, Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);
typedef _ThumbDart = Pointer<Uint8> Function(
    Pointer<Char>, int, Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);

// Bridge v0.10 (audio playback): play takes the comp id + start seconds, seek
// a bare double; the per-tick clock poll writes through out-pointers and
// returns whether this build has an audio pipeline at all.
typedef _AudioPlayC = Pointer<Char> Function(Pointer<Char>, Double);
typedef _AudioPlayDart = Pointer<Char> Function(Pointer<Char>, double);
typedef _DoubleArgC = Pointer<Char> Function(Double);
typedef _DoubleArgDart = Pointer<Char> Function(double);
typedef _AudioClockC = Bool Function(
    Pointer<Double>, Pointer<Bool>, Pointer<Bool>);
typedef _AudioClockDart = bool Function(
    Pointer<Double>, Pointer<Bool>, Pointer<Bool>);

// Bridge v0.5 signatures.
typedef _TextContentC = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Pointer<Char>, Double, Double, Double, Double, Double);
typedef _TextContentDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, double, double, double, double,
    double);
typedef _SetSolidC = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Double, Double, Double, Double, Uint32, Uint32);
typedef _SetSolidDart = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    double, double, double, double, int, int);
typedef _EffectU32C = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>, Uint32);
typedef _EffectU32Dart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>, int);
typedef _EffectBoolC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>, Bool);
typedef _EffectBoolDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>, bool);
typedef _EffectPointC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>, Double, Double);
typedef _EffectPointDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>, double, double);
typedef _Str3IntC = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, Int64);
typedef _Str3IntDart = Pointer<Char> Function(
    Pointer<Char>, Pointer<Char>, Pointer<Char>, int);

// Bridge v0.9 signatures.
// Mask geometry: comp, layer, kind + a drag rect (x, y, w, h).
typedef _MaskGeomC = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Pointer<Char>, Double, Double, Double, Double);
typedef _MaskGeomDart = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Pointer<Char>, double, double, double, double);
// Effect-param keyframe toggle/remove: comp, layer, effect, param, channel,
// frame.
typedef _EffectKeyC = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Pointer<Char>, Pointer<Char>, Int64, Int64);
typedef _EffectKeyDart = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Pointer<Char>, Pointer<Char>, int, int);
// Effect-param add-keyframe: the above + a value.
typedef _EffectAddKeyC = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Pointer<Char>, Pointer<Char>, Int64, Int64, Double);
typedef _EffectAddKeyDart = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Pointer<Char>, Pointer<Char>, int, int, double);
// Effect-param shift: comp, layer, effect, param, channel, frames_json, delta.
typedef _EffectShiftC = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Pointer<Char>, Pointer<Char>, Int64, Pointer<Char>, Int64);
typedef _EffectShiftDart = Pointer<Char> Function(Pointer<Char>, Pointer<Char>,
    Pointer<Char>, Pointer<Char>, int, Pointer<Char>, int);
// Effect-param keyframe interp: comp, layer, effect, param, channel, frame,
// interp_in, interp_out, speed_in, influence_in, speed_out, influence_out.
typedef _EffectKeyInterpC = Pointer<Char> Function(
    Pointer<Char>,
    Pointer<Char>,
    Pointer<Char>,
    Pointer<Char>,
    Int64,
    Int64,
    Pointer<Char>,
    Pointer<Char>,
    Double,
    Double,
    Double,
    Double);
typedef _EffectKeyInterpDart = Pointer<Char> Function(
    Pointer<Char>,
    Pointer<Char>,
    Pointer<Char>,
    Pointer<Char>,
    int,
    int,
    Pointer<Char>,
    Pointer<Char>,
    double,
    double,
    double,
    double);

/// The set of document operations the frontend drives the engine through. The
/// real implementation is [LumitBridge] (dart:ffi over the shared library); the
/// interface exists so tests can supply a fake without loading the library or
/// touching plugin channels — every method is a pure `String → BridgeReply`
/// call, so a fake is a handful of lines.
abstract class DocumentBridge {
  BridgeReply snapshot();
  BridgeReply newProject();
  BridgeReply undo();
  BridgeReply redo();
  BridgeReply openProject(String path);

  /// Save to [path]; an empty string saves to the loaded path (an error reply
  /// if the document has never been saved).
  BridgeReply saveProject(String path);
  BridgeReply newComposition(String name);

  /// Add a footage item referencing the media file at [path]. With the engine's
  /// `media` feature the item is probed and its metadata/status ride the
  /// returned snapshot.
  BridgeReply importFootage(String path);

  /// Flip a layer's switch through the real op (undoable). [switchName] is the
  /// model's own field name (`visible`, `audible`, `locked`, `solo`,
  /// `motion_blur`, `fx`, `three_d`, `collapse`).
  BridgeReply setLayerSwitch(
      String compId, String layerId, String switchName, bool value);

  /// Edit a layer's span relative to the playhead [frame]. [edit] is one of
  /// `move_in`, `move_out`, `trim_in`, `trim_out`.
  BridgeReply editLayerSpan(
      String compId, String layerId, String edit, int frame);

  /// Set one transform property to a static [value]. [property] is a snake_case
  /// `TransformProp` name (e.g. `position_x`, `rotation`, `opacity`).
  BridgeReply setTransform(
      String compId, String layerId, String property, double value);

  /// Drop a user marker on the composition timeline at [frame].
  BridgeReply addMarker(String compId, int frame);

  // --- Bridge v0.3: layer lifecycle -------------------------------------

  /// Add a Solid layer (a white, comp-sized solid asset) to [compId].
  BridgeReply addSolidLayer(String compId);

  /// Add a Text layer (the "Text" starter document) to [compId].
  BridgeReply addTextLayer(String compId);

  /// Add a Camera layer to [compId].
  BridgeReply addCameraLayer(String compId);

  /// Add an Adjustment layer to [compId].
  BridgeReply addAdjustmentLayer(String compId);

  /// Add an (empty) Sequence layer to [compId].
  BridgeReply addSequenceLayer(String compId);

  /// Place the project footage item [itemId] into [compId] as a new Footage
  /// layer (top of the stack; the media's own duration/size when it has probed,
  /// else the full comp). An error reply when [itemId] is not a footage item.
  BridgeReply addFootageLayer(String compId, String itemId);

  /// Reorder a layer within its composition to [newIndex] (0 = top). The op
  /// clamps an out-of-range index into range rather than failing.
  BridgeReply reorderLayer(String compId, String layerId, int newIndex);

  /// Delete a layer from its composition.
  BridgeReply deleteLayer(String compId, String layerId);

  /// Duplicate a layer (a copy above the original, with a fresh id).
  BridgeReply duplicateLayer(String compId, String layerId);

  // --- Bridge v0.3: comp settings ---------------------------------------

  /// Edit a composition's settings (name, size, rate, duration in frames) as
  /// one undo step; the background is preserved.
  BridgeReply setCompSettings(String compId, String name, int width, int height,
      int fpsNum, int fpsDen, int durationFrames);

  // --- Bridge v0.3: keyframes -------------------------------------------

  /// The stopwatch: toggle a transform property's animation at the playhead
  /// [frame] (seed a key on enable, collapse to static on disable).
  BridgeReply togglePropertyAnimated(
      String compId, String layerId, String property, int frame);

  /// Insert or replace a transform keyframe at [frame] with [value].
  BridgeReply addKeyframe(
      String compId, String layerId, String property, int frame, double value);

  /// Remove the transform keyframe at [frame] (collapses to static when it was
  /// the last key).
  BridgeReply removeKeyframe(
      String compId, String layerId, String property, int frame);

  /// Slide the transform keyframes at comp [frames] by [delta] frames.
  BridgeReply shiftKeyframes(String compId, String layerId, String property,
      List<int> frames, int delta);

  // --- Bridge v0.3: work area -------------------------------------------

  /// Set one work-area edge to the playhead [frame] ([isOut] picks the out
  /// edge).
  BridgeReply setWorkAreaEdge(String compId, int frame, bool isOut);

  // --- Bridge v0.3: effects ---------------------------------------------

  /// The built-in effect registry (`[{name, label}]`). Empty on any failure.
  List<BridgeEffectInfo> listEffects();

  /// Apply a built-in effect (by its match [effectName]) to a layer.
  BridgeReply addEffect(String compId, String layerId, String effectName);

  /// Remove an effect instance from a layer by its id.
  BridgeReply removeEffect(String compId, String layerId, String effectId);

  /// Enable or bypass an effect instance.
  BridgeReply setEffectEnabled(
      String compId, String layerId, String effectId, bool enabled);

  /// Set a scalar (Float) effect parameter to a static [value].
  BridgeReply setEffectParamScalar(String compId, String layerId,
      String effectId, String paramName, double value);

  /// Set a Colour effect parameter to a static scene-linear RGBA.
  BridgeReply setEffectParamColour(String compId, String layerId,
      String effectId, String paramName, double r, double g, double b, double a);

  // --- Bridge v0.4: keyframe interpolation ------------------------------

  /// Set the interpolation of the keyframe nearest [frame] on a transform
  /// [property]. [interpIn]/[interpOut] are `Hold`/`Linear`/`Bezier`; the
  /// `(speed, influence)` pairs apply only to a `Bezier` side.
  BridgeReply setKeyframeInterp(
      String compId,
      String layerId,
      String property,
      int frame,
      String interpIn,
      String interpOut,
      double speedIn,
      double influenceIn,
      double speedOut,
      double influenceOut);

  // --- Bridge v0.4: Retime ----------------------------------------------

  /// Enable or disable a footage layer's Retime (the Time stopwatch).
  BridgeReply setRetimeEnabled(String compId, String layerId, bool enabled);

  /// Set a footage layer's constant playback speed (percent; 100 clears it).
  BridgeReply setRetimeSpeed(String compId, String layerId, double speedPercent);

  /// Set the ease of the Retime segment at [frame] (`Lin`/`Slow`/`Fast`/`Smth`/
  /// `Shrp`).
  BridgeReply setSegmentPreset(
      String compId, String layerId, int frame, String ease);

  /// Convert the Map segment at [frame] to a Rate segment (the reply's snapshot
  /// carries an added `drift` field).
  BridgeReply segmentToRate(String compId, String layerId, int frame);

  /// Move the value-lens Retime boundary at [index] to comp [frame].
  BridgeReply dragBoundary(String compId, String layerId, int index, int frame);

  // --- Bridge v0.4: timeline columns ------------------------------------

  /// The blend-mode registry (`[{name, label}]`). Empty on any failure.
  List<BridgeBlendMode> listBlendModes();

  /// Set a layer's blend mode (the serde variant name, e.g. `Multiply`).
  BridgeReply setBlendMode(String compId, String layerId, String mode);

  /// Point a layer at another as its matte, or clear it when [source] is empty.
  /// [channel] is `alpha`/`luma`.
  BridgeReply setMatte(String compId, String layerId, String source,
      String channel, bool inverted);

  /// Point a layer at another as its transform parent, or clear it when
  /// [parent] is empty.
  BridgeReply setParent(String compId, String layerId, String parent);

  /// Set the comp's motion-blur master (enable, shutter angle/phase, samples).
  BridgeReply setMotionBlur(String compId, bool enabled, double shutterAngle,
      double shutterPhase, int samples);

  /// Add a starter mask shape (`rectangle`/`ellipse`/`star`) to a layer.
  BridgeReply addMask(String compId, String layerId, String kind);

  // --- Bridge v0.4: export ----------------------------------------------

  /// Resolve a delivery [presetName] into the dialogue fields it stamps plus its
  /// suggested file name. [compName] and [template] drive the filename tokens.
  BridgeExportPreset exportPreset(
      String presetName, String compName, String template);

  /// Start an export of [compId] to [outPath] with the dialogue-shaped
  /// [specJson]. `ok:false` "an export is already running" while one is in
  /// flight (queue on the Dart side).
  BridgeReply startExport(String compId, String specJson, String outPath);

  /// Poll the running export, draining its progress channel.
  BridgeExportState exportPoll();

  /// Ask the running export to cancel (a no-op when none is running).
  BridgeReply exportCancel();

  /// Decode one footage frame to RGBA8 (the F2 CPU path), or null on failure
  /// (missing/unreadable file, no engine library). The pixels are copied out of
  /// the engine buffer, which is freed immediately.
  DecodedFrame? decodeFrame(String itemId, int frame);
}

/// The composited-comp render capability, kept as its own interface (not part of
/// [DocumentBridge]) so the many `implements DocumentBridge` fakes across the
/// suite need no change: a bridge either offers this capability or it does not.
/// The real [LumitBridge] implements it; [PreviewSource] probes with an
/// `is CompRenderBridge` check and reads [supportsCompRender] to tell a new
/// engine (with or without a GPU adapter) from an old library that lacks the
/// symbol entirely.
abstract class CompRenderBridge {
  /// True when the loaded library exports the composited-comp render symbol.
  /// False for an older library — the discriminator that keeps such a build on
  /// the single-layer path. It stays true even on a machine with no GPU adapter
  /// (the symbol is present); there, [renderCompFrame] simply returns null.
  bool get supportsCompRender;

  /// Render the WHOLE composited comp [compId] at [frame] to RGBA8 — every
  /// layer, transform, blend and effect, the same pixels the egui Viewer and
  /// the exporter produce. [scale] of 1.0 is the comp's own resolution; a
  /// smaller positive value downsamples the output. Null on failure (unknown
  /// comp, no GPU adapter, a render error); a missing layer inside the comp
  /// arrives already slated as colour bars within the returned frame. The
  /// pixels are copied out of the engine buffer, which is freed immediately.
  DecodedFrame? renderCompFrame(String compId, int frame, double scale);
}

/// The transform-preview fast-path capability (ABI 11), kept as its own
/// interface for the same reason as [CompRenderBridge]: a bridge either offers
/// it or not, and the many `implements DocumentBridge` fakes need no change.
/// Lets a drag stage an in-memory-only edit and render just the current frame
/// under it, without the full commit path's undo entry, journal write and
/// whole-document JSON round-trip — see `AppStateStub.previewTransform`.
abstract class PreviewTransformBridge {
  /// True when the loaded library exports the preview-transform symbols.
  /// False for an older library — the discriminator that keeps such a build on
  /// the per-tick [DocumentBridge.setTransform] path it already had.
  bool get supportsPreviewTransform;

  /// Stage (or update) an in-memory preview of [layerId]'s [property] —
  /// no undo entry, no journal write, no snapshot round-trip. The reply is a
  /// tiny `{"ok":true}` ack, never a document snapshot.
  BridgeReply previewTransform(
      String compId, String layerId, String property, double value);

  /// Drop the active preview without committing (Escape / drag-cancel).
  void cancelTransformPreview();

  /// Render composition [compId] at [frame] under the active preview (if
  /// any) — the drag-preview sibling of [CompRenderBridge.renderCompFrame].
  /// Never served from or banked into the engine's rendered-frame cache, so a
  /// caller must not call this more than once per frame it actually wants.
  DecodedFrame? renderPreviewFrame(String compId, int frame, double scale);
}

/// The zero-copy Viewer capability (K-177), kept as its own interface for the
/// same reason as [CompRenderBridge]: a bridge either offers it or it does not,
/// and the many `implements DocumentBridge` fakes need no change. The real
/// [LumitBridge] implements it; it is offered only when the loaded `.dll` was
/// built with the `shared-texture` feature on Windows AND exports the symbols.
abstract class SharedTextureBridge {
  /// True when this build offers the shared-texture path — the symbols are bound
  /// AND the engine reports it was compiled with the feature on Windows. False
  /// for an older library, a non-Windows build, or a feature-less build; the
  /// Viewer then stays on the read-back path.
  bool get supportsSharedTexture;

  /// Render the whole composited comp [compId] at [frame] into the shared GPU
  /// texture, returning its NT [SharedFrame.handle] and dimensions — no pixels
  /// cross. Null on failure (no D3D12 adapter, an unknown comp, a transient
  /// error), which sends the Viewer back to the read-back path for that frame.
  SharedFrame? renderToShared(String compId, int frame);
}

/// The GPU scope-pass capability (K-096 v1), kept as its own interface for the
/// same reason as [CompRenderBridge]: a bridge either offers it or it does not,
/// and the many `implements DocumentBridge` fakes need no change. The real
/// [LumitBridge] implements it; the Scopes panel probes with an
/// `is ScopeTraceBridge` check and reads [supportsScopeTrace] to prefer the
/// engine trace over its CPU fallback.
abstract class ScopeTraceBridge {
  /// True when the loaded library exports `render_scope` (the engine computes
  /// the trace on the GPU). False for an older library — the Scopes panel then
  /// keeps computing the trace on the CPU from the shown frame.
  bool get supportsScopeTrace;

  /// Compute a scope trace for the frame the Viewer shows — comp [compId] at
  /// [frame], at the same [scale] — returning the 256×256 RGBA8 trace bytes, or
  /// null on failure (unknown comp, no adapter, an older library). [kind] is
  /// `0` luma / `1` RGB waveform / `2` vectorscope / `3` histogram; [bg],
  /// [trace], [red], [green], [blue] are the fixed scope colours packed
  /// `0x00RRGGBB`. The heavy binning runs on the GPU; only the tiny trace
  /// crosses the boundary. The pixels are copied out before the engine buffer is
  /// freed, so the returned bytes are owned.
  Uint8List? renderScope(int kind, String compId, int frame, double scale,
      int bg, int trace, int red, int green, int blue);
}

/// One reading of the engine's audio playback clock (ABI 10): where the sound
/// card's clock sits, whether it is running, and whether a comp's mix is
/// loaded at all. [none] is the no-audio state (an old library, no output
/// device, a silent comp) — the Viewer then keeps its wall clock.
class AudioClock {
  /// Seconds of audio consumed since load/seek — the playback master clock.
  final double seconds;

  /// Whether the engine is currently consuming samples.
  final bool playing;

  /// Whether a comp's mix is loaded in the engine. False keeps the Viewer on
  /// its wall-clock fallback.
  final bool loaded;

  const AudioClock(
      {required this.seconds, required this.playing, required this.loaded});

  static const none = AudioClock(seconds: 0, playing: false, loaded: false);
}

/// Comp audio playback (ABI 10, docs/09), kept as its own capability interface
/// for the same reason as [CompRenderBridge]: a bridge either offers it or it
/// does not, and the many `implements DocumentBridge` fakes need no change.
/// The real [LumitBridge] implements it; [AppStateStub] probes with an
/// `is AudioPlaybackBridge` check. The engine owns one audio engine for the
/// session: the sound card's clock is the playback master and the Viewer's
/// ticker chases [audioClock]. On a machine with no output device every call
/// stays calm and [audioClock] reads unloaded — playback simply has no sound.
abstract class AudioPlaybackBridge {
  /// True when the loaded library exports the audio playback symbols (ABI
  /// 10+). False for an older library — playback then stays on the wall clock.
  bool get supportsAudioPlayback;

  /// Build (or refresh) [compId]'s audio mix in the background — called after
  /// an edit while audio is loaded or playing, so mute/solo/move/trim/Volume
  /// edits are heard. Cheap when nothing changed (a signature no-op); a
  /// changed mix is swapped in mid-playback with the clock kept.
  void audioPrepare(String compId);

  /// Start playing [compId]'s audio from [startSeconds]. An already-loaded mix
  /// seeks and plays at once; otherwise it is prepared in the background and
  /// starts from [startSeconds] when it lands.
  void audioPlay(String compId, double startSeconds);

  /// Pause playback (the clock holds its position).
  void audioPause();

  /// Move the audio clock to [seconds] (a scrub; play state untouched).
  void audioSeek(double seconds);

  /// Stop: pause and rewind to the start.
  void audioStop();

  /// The playback clock — polled once per Viewer tick; allocation-free on the
  /// engine side. [AudioClock.none] when this build has no audio pipeline.
  AudioClock audioClock();
}

/// The rendered-frame cache's live stats (ABI 8, `cacheStats`): the bytes
/// [usedBytes] of the [budgetBytes] cap, the number of cached frames [entries],
/// and the lifetime [hits]/[misses]. The default (no library / a parse failure)
/// is an empty cache at a zero budget.
class BridgeCacheStats {
  final int usedBytes;
  final int budgetBytes;
  final int entries;
  final int hits;
  final int misses;

  const BridgeCacheStats({
    this.usedBytes = 0,
    this.budgetBytes = 0,
    this.entries = 0,
    this.hits = 0,
    this.misses = 0,
  });

  static const empty = BridgeCacheStats();

  factory BridgeCacheStats.fromJson(Map<String, dynamic> m) => BridgeCacheStats(
        usedBytes: _asInt(m['used_bytes']),
        budgetBytes: _asInt(m['budget_bytes']),
        entries: _asInt(m['entries']),
        hits: _asInt(m['hits']),
        misses: _asInt(m['misses']),
      );
}

/// The rendered-frame cache controls (ABI 8, K-176), kept as their own
/// capability so the many `implements DocumentBridge` fakes need no change. The
/// real [LumitBridge] implements it; a bridge either offers the symbols or it
/// does not. These back Settings → Performance (Clear cache, Memory budget).
abstract class CacheControlBridge {
  /// True when the loaded library exports the cache-control symbols (ABI 8+).
  bool get supportsCacheControl;

  /// Empty the rendered-frame cache now; returns the fresh stats.
  BridgeCacheStats clearCache();

  /// Set the cache's RAM budget in bytes; returns the fresh stats.
  BridgeCacheStats setCacheBudget(int bytes);

  /// The cache's current stats.
  BridgeCacheStats cacheStats();
}

/// The latest-wins render-cancellation control (ABI 8, K-176): the UI isolate
/// marks every render generation below [generation] as superseded so a stale
/// comp render queued behind the renderer lock is skipped before it starts. A
/// fast atomic store, safe to call on the UI isolate. Its own capability so the
/// fakes need no change.
abstract class RenderCancelBridge {
  /// True when the loaded library exports `render_cancel_stale` (ABI 8+).
  bool get supportsRenderCancel;

  /// Publish [generation] as the newest wanted render; lower ones are stale.
  void renderCancelStale(int generation);
}

/// The Project-panel thumbnail path (ABI 8): decode + downscale a footage item's
/// representative frame once, caching it engine-side. Its own capability so the
/// many `implements DocumentBridge` fakes need no change; a panel builds its UI
/// against this and a fake supplies a synthetic thumbnail.
abstract class ThumbnailBridge {
  /// True when the loaded library exports `thumbnail` (ABI 8+).
  bool get supportsThumbnail;

  /// A thumbnail of footage [itemId] whose longer edge is at most [maxEdge],
  /// or null on failure (unknown/non-footage item, missing/unreadable file, an
  /// older library). The pixels are copied out of the engine buffer, which is
  /// freed immediately, so the returned frame owns them.
  DecodedFrame? thumbnail(String itemId, int maxEdge);
}

/// The bridge v0.5 edit ops, kept as their own capability interface (like
/// [CompRenderBridge]) so the many `implements DocumentBridge` fakes across the
/// suite need no change: a bridge either offers this capability or it does not.
/// The real [LumitBridge] implements it; [AppStateStub] probes with an
/// `is EditOpsBridge` check and routes its pass-throughs here, surfacing a calm
/// "engine build without edit ops" notice when the loaded library is too old.
abstract class EditOpsBridge {
  // Razor (sequence layers).
  BridgeReply cutClipAtPlayhead(String compId, String layerId, int frame);
  BridgeReply deleteClipAtPlayhead(String compId, String layerId, int frame);

  // Beats.
  BridgeReply detectBeats(String compId, int sensitivity);
  BridgeReply clearBeatMarkers(String compId);

  // Project-item ops.
  BridgeReply deleteItem(String itemId);
  BridgeReply renameItem(String itemId, String name);
  BridgeReply moveToRoot(String itemId);
  BridgeReply relink(String itemId, String path);

  // Layer-identity ops.
  BridgeReply renameLayer(String compId, String layerId, String name);
  BridgeReply convertToSequenced(String compId, String layerId);
  BridgeReply trimToSourceEnd(String compId, String layerId);

  // Retime setters.
  BridgeReply setRetimeReverse(String compId, String layerId, bool reverse);
  BridgeReply setRetimeInterpolation(
      String compId, String layerId, String interp);

  // Asset-property ops.
  BridgeReply setTextContent(String compId, String layerId, String text,
      double size, double r, double g, double b, double a);
  BridgeReply setSolid(String compId, String layerId, double r, double g,
      double b, double a, int width, int height);
  BridgeReply setCameraZoom(String compId, String layerId, double zoom);

  // Extra effect-param setters + reorder + the linked-keyframe batch.
  BridgeReply setEffectParamChoice(
      String compId, String layerId, String effectId, String paramName, int index);
  BridgeReply setEffectParamBool(String compId, String layerId, String effectId,
      String paramName, bool value);
  BridgeReply setEffectParamSeed(String compId, String layerId, String effectId,
      String paramName, int seed);
  BridgeReply setEffectParamPoint(String compId, String layerId, String effectId,
      String paramName, double x, double y);
  BridgeReply reorderEffect(
      String compId, String layerId, String effectId, int newIndex);
  BridgeReply applyKeyframeBatch(String compId, String layerId, String opsJson);

  // Recovery + boot log.
  BridgeReply autosave(String path, int keep);
  List<BridgeAutosave> listAutosaves(String path);
  BridgeReply restoreJournal(String path);
  List<String> bootLog();

  // Bridge v0.9: mask geometry, effect-param keyframes, presets, realtime tier.
  BridgeReply addMaskGeometry(String compId, String layerId, String kind,
      double x, double y, double w, double h);
  BridgeReply toggleEffectParamAnimated(String compId, String layerId,
      String effectId, String paramName, int channel, int frame);
  BridgeReply addEffectParamKeyframe(String compId, String layerId,
      String effectId, String paramName, int channel, int frame, double value);
  BridgeReply removeEffectParamKeyframe(String compId, String layerId,
      String effectId, String paramName, int channel, int frame);
  BridgeReply shiftEffectParamKeyframes(String compId, String layerId,
      String effectId, String paramName, int channel, String framesJson,
      int delta);
  BridgeReply setEffectParamKeyframeInterp(
      String compId,
      String layerId,
      String effectId,
      String paramName,
      int channel,
      int frame,
      String interpIn,
      String interpOut,
      double speedIn,
      double influenceIn,
      double speedOut,
      double influenceOut);
  BridgeReply saveEffectPreset(String compId, String layerId, String name);
  BridgeReply loadEffectPreset(String compId, String layerId, String text);
  BridgePlaybackTier playbackTier();
  BridgePlaybackTier resetRealtime();
}

/// The `.lumfx` preset serialiser (bridge v0.9): `save_effect_preset` returns
/// the stack as `.lumfx` JSON (not a snapshot), so it is its own capability
/// interface — the real [LumitBridge] implements it, and a fake can supply the
/// JSON so the Dart-side save-to-file flow is testable without the library.
abstract class PresetJsonBridge {
  /// The `.lumfx` JSON for a layer's effect stack, or null on failure / an older
  /// library. The Dart side writes it to a file it picked.
  String? saveEffectPresetJson(String compId, String layerId, String name);
}

/// The loaded `lumit_bridge` library, bound to typed calls. Construct with
/// [tryLoad]; a null result means the app runs on its placeholders.
class LumitBridge
    implements
        DocumentBridge,
        CompRenderBridge,
        PreviewTransformBridge,
        SharedTextureBridge,
        CacheControlBridge,
        RenderCancelBridge,
        ThumbnailBridge,
        ScopeTraceBridge,
        EditOpsBridge,
        PresetJsonBridge,
        AudioPlaybackBridge {
  final _NoArgDart _version;
  final _NoArgDart _newProject;
  final _StrArgDart _openProject;
  final _StrArgDart _saveProject;
  final _NoArgDart _snapshot;
  final _StrArgDart _newComposition;
  final _StrArgDart _importFootage;
  final _NoArgDart _undo;
  final _NoArgDart _redo;
  final _SwitchDart _setLayerSwitch;
  final _SpanDart _editLayerSpan;
  final _TransformDart _setTransform;
  final _MarkerDart _addMarker;
  final _StrArgDart _addSolidLayer;
  final _StrArgDart _addTextLayer;
  final _StrArgDart _addCameraLayer;
  final _StrArgDart _addAdjustmentLayer;
  final _StrArgDart _addSequenceLayer;
  final _Str2Dart _addFootageLayer;
  final _Str2IntDart _reorderLayer;
  final _Str2Dart _deleteLayer;
  final _Str2Dart _duplicateLayer;
  final _CompSettingsDart _setCompSettings;
  final _SpanDart _togglePropertyAnimated;
  final _KeyframeDart _addKeyframe;
  final _SpanDart _removeKeyframe;
  final _ShiftDart _shiftKeyframes;
  final _WorkAreaDart _setWorkAreaEdge;
  final _NoArgDart _listEffects;
  final _Str3Dart _addEffect;
  final _Str3Dart _removeEffect;
  final _Str3BoolDart _setEffectEnabled;
  final _ScalarParamDart _setEffectParamScalar;
  final _ColourParamDart _setEffectParamColour;
  // Bridge v0.4.
  final _KeyframeInterpDart _setKeyframeInterp;
  final _Str2BoolDart _setRetimeEnabled;
  final _Str2DoubleDart _setRetimeSpeed;
  final _SegmentPresetDart _setSegmentPreset;
  final _Str2IntDart _segmentToRate;
  final _Str2Int2Dart _dragBoundary;
  final _NoArgDart _listBlendModes;
  final _Str3Dart _setBlendMode;
  final _MatteDart _setMatte;
  final _Str3Dart _setParent;
  final _MotionBlurDart _setMotionBlur;
  final _Str3Dart _addMask;
  final _Str3Dart _exportPreset;
  final _Str3Dart _startExport;
  final _NoArgDart _exportPoll;
  final _NoArgDart _exportCancel;
  final _DecodeDart _decodeFrame;
  // Bridge v0.5.
  final _Str2IntDart _cutClipAtPlayhead;
  final _Str2IntDart _deleteClipAtPlayhead;
  final _MarkerDart _detectBeats;
  final _StrArgDart _clearBeatMarkers;
  final _StrArgDart _deleteItem;
  final _Str2Dart _renameItem;
  final _StrArgDart _moveToRoot;
  final _Str2Dart _relink;
  final _Str3Dart _renameLayer;
  final _Str2Dart _convertToSequenced;
  final _Str2Dart _trimToSourceEnd;
  final _Str2BoolDart _setRetimeReverse;
  final _Str3Dart _setRetimeInterpolation;
  final _MarkerDart _autosave;
  final _StrArgDart _listAutosaves;
  final _StrArgDart _restoreJournal;
  final _NoArgDart _bootLog;
  final _TextContentDart _setTextContent;
  final _SetSolidDart _setSolid;
  final _Str2DoubleDart _setCameraZoom;
  final _EffectU32Dart _setEffectParamChoice;
  final _EffectBoolDart _setEffectParamBool;
  final _EffectU32Dart _setEffectParamSeed;
  final _EffectPointDart _setEffectParamPoint;
  final _Str3IntDart _reorderEffect;
  final _Str3Dart _applyKeyframeBatch;
  // Bridge v0.9. Bound defensively (an older `.dll` lacks these): each stays
  // null and its method degrades to an unsupported reply.
  _MaskGeomDart? _addMaskGeometry;
  _EffectKeyDart? _toggleEffectParamAnimated;
  _EffectAddKeyDart? _addEffectParamKeyframe;
  _EffectKeyDart? _removeEffectParamKeyframe;
  _EffectShiftDart? _shiftEffectParamKeyframes;
  _EffectKeyInterpDart? _setEffectParamKeyframeInterp;
  _Str3Dart? _saveEffectPreset;
  _Str3Dart? _loadEffectPreset;
  _NoArgDart? _playbackTier;
  _NoArgDart? _resetRealtime;

  /// Bound only when the loaded library exports it. An older `.dll` (predating
  /// the composited-comp path) lacks the symbol; rather than failing the whole
  /// load, this stays null and [renderCompFrame] returns null, so the Viewer
  /// keeps its single-layer path. Non-final because it is looked up defensively
  /// in the constructor body, not the initializer list.
  _RenderDart? _renderCompFrame;

  /// The transform-preview fast-path symbols (ABI 11). Bound defensively like
  /// [_renderCompFrame], all three together in one block (they only make
  /// sense as a set): an older `.dll` lacks them, so [supportsPreviewTransform]
  /// is false and the Effect Controls panel keeps its per-tick
  /// [DocumentBridge.setTransform] path.
  _TransformDart? _previewTransform;
  _NoArgDart? _cancelTransformPreview;
  _RenderDart? _renderPreviewFrame;

  /// The zero-copy shared-texture symbols (K-177). Bound defensively like
  /// [_renderCompFrame]: an older `.dll` lacks them, and both stay null then, so
  /// [supportsSharedTexture] is false and the Viewer keeps the read-back path.
  _SharedSupportedDart? _sharedSupported;
  _RenderSharedDart? _renderToShared;
  // The Linux DMA-BUF render entry point (K-177), bound defensively alongside the
  // Windows one. On Linux [renderToShared] prefers this; on Windows it is null and
  // the handle path is used.
  _RenderSharedDmabufDart? _renderToSharedDmabuf;

  /// The GPU scope-pass symbol (K-096 v1). Bound defensively like
  /// [_renderCompFrame]: an older `.dll` lacks it, so it stays null and
  /// [supportsScopeTrace] is false, keeping the Scopes panel on its CPU path.
  _RenderScopeDart? _renderScope;

  /// The ABI-8 cache-control, render-cancellation and thumbnail symbols. Bound
  /// defensively: an older `.dll` lacks them, so each capability reports itself
  /// off and its methods degrade (empty stats / no-op / null thumbnail).
  _NoArgDart? _clearCache;
  _U64ArgDart? _setCacheBudget;
  _NoArgDart? _cacheStats;
  _U64ArgDart? _renderCancelStale;
  _ThumbDart? _thumbnail;

  /// The ABI-10 audio playback symbols, bound as a group (all or none): an
  /// older `.dll` lacks them, so [supportsAudioPlayback] reports false and
  /// playback stays on the wall clock.
  _StrArgDart? _audioPrepare;
  _AudioPlayDart? _audioPlay;
  _NoArgDart? _audioPause;
  _DoubleArgDart? _audioSeek;
  _NoArgDart? _audioStop;
  _AudioClockDart? _audioClock;

  /// Out-pointers for the per-tick clock poll, allocated once so polling the
  /// clock every Viewer tick allocates nothing. Session-lifetime (the library
  /// itself is never unloaded), so they are deliberately never freed.
  Pointer<Double>? _audioClockSecs;
  Pointer<Bool>? _audioClockPlaying;
  Pointer<Bool>? _audioClockLoaded;

  final _FreeDart _freeString;
  final _FreeBufferDart _freeBuffer;

  LumitBridge._(DynamicLibrary lib)
      : _version = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_version',
        ),
        _newProject = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_new_project',
        ),
        _openProject = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_open_project',
        ),
        _saveProject = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_save_project',
        ),
        _snapshot = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_snapshot',
        ),
        _newComposition = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_new_composition',
        ),
        _importFootage = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_import_footage',
        ),
        _undo = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_undo',
        ),
        _redo = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_redo',
        ),
        _setLayerSwitch = lib.lookupFunction<_SwitchC, _SwitchDart>(
          'lumit_bridge_set_layer_switch',
        ),
        _editLayerSpan = lib.lookupFunction<_SpanC, _SpanDart>(
          'lumit_bridge_edit_layer_span',
        ),
        _setTransform = lib.lookupFunction<_TransformC, _TransformDart>(
          'lumit_bridge_set_transform',
        ),
        _addMarker = lib.lookupFunction<_MarkerC, _MarkerDart>(
          'lumit_bridge_add_marker',
        ),
        _addSolidLayer = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_add_solid_layer',
        ),
        _addTextLayer = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_add_text_layer',
        ),
        _addCameraLayer = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_add_camera_layer',
        ),
        _addAdjustmentLayer = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_add_adjustment_layer',
        ),
        _addSequenceLayer = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_add_sequence_layer',
        ),
        _addFootageLayer = lib.lookupFunction<_Str2C, _Str2Dart>(
          'lumit_bridge_add_footage_layer',
        ),
        _reorderLayer = lib.lookupFunction<_Str2IntC, _Str2IntDart>(
          'lumit_bridge_reorder_layer',
        ),
        _deleteLayer = lib.lookupFunction<_Str2C, _Str2Dart>(
          'lumit_bridge_delete_layer',
        ),
        _duplicateLayer = lib.lookupFunction<_Str2C, _Str2Dart>(
          'lumit_bridge_duplicate_layer',
        ),
        _setCompSettings =
            lib.lookupFunction<_CompSettingsC, _CompSettingsDart>(
          'lumit_bridge_set_comp_settings',
        ),
        _togglePropertyAnimated = lib.lookupFunction<_SpanC, _SpanDart>(
          'lumit_bridge_toggle_property_animated',
        ),
        _addKeyframe = lib.lookupFunction<_KeyframeC, _KeyframeDart>(
          'lumit_bridge_add_keyframe',
        ),
        _removeKeyframe = lib.lookupFunction<_SpanC, _SpanDart>(
          'lumit_bridge_remove_keyframe',
        ),
        _shiftKeyframes = lib.lookupFunction<_ShiftC, _ShiftDart>(
          'lumit_bridge_shift_keyframes',
        ),
        _setWorkAreaEdge = lib.lookupFunction<_WorkAreaC, _WorkAreaDart>(
          'lumit_bridge_set_work_area_edge',
        ),
        _listEffects = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_list_effects',
        ),
        _addEffect = lib.lookupFunction<_Str3C, _Str3Dart>(
          'lumit_bridge_add_effect',
        ),
        _removeEffect = lib.lookupFunction<_Str3C, _Str3Dart>(
          'lumit_bridge_remove_effect',
        ),
        _setEffectEnabled = lib.lookupFunction<_Str3BoolC, _Str3BoolDart>(
          'lumit_bridge_set_effect_enabled',
        ),
        _setEffectParamScalar =
            lib.lookupFunction<_ScalarParamC, _ScalarParamDart>(
          'lumit_bridge_set_effect_param_scalar',
        ),
        _setEffectParamColour =
            lib.lookupFunction<_ColourParamC, _ColourParamDart>(
          'lumit_bridge_set_effect_param_colour',
        ),
        _setKeyframeInterp =
            lib.lookupFunction<_KeyframeInterpC, _KeyframeInterpDart>(
          'lumit_bridge_set_keyframe_interp',
        ),
        _setRetimeEnabled = lib.lookupFunction<_Str2BoolC, _Str2BoolDart>(
          'lumit_bridge_set_retime_enabled',
        ),
        _setRetimeSpeed = lib.lookupFunction<_Str2DoubleC, _Str2DoubleDart>(
          'lumit_bridge_set_retime_speed',
        ),
        _setSegmentPreset =
            lib.lookupFunction<_SegmentPresetC, _SegmentPresetDart>(
          'lumit_bridge_set_segment_preset',
        ),
        _segmentToRate = lib.lookupFunction<_Str2IntC, _Str2IntDart>(
          'lumit_bridge_segment_to_rate',
        ),
        _dragBoundary = lib.lookupFunction<_Str2Int2C, _Str2Int2Dart>(
          'lumit_bridge_drag_boundary',
        ),
        _listBlendModes = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_list_blend_modes',
        ),
        _setBlendMode = lib.lookupFunction<_Str3C, _Str3Dart>(
          'lumit_bridge_set_blend_mode',
        ),
        _setMatte = lib.lookupFunction<_MatteC, _MatteDart>(
          'lumit_bridge_set_matte',
        ),
        _setParent = lib.lookupFunction<_Str3C, _Str3Dart>(
          'lumit_bridge_set_parent',
        ),
        _setMotionBlur = lib.lookupFunction<_MotionBlurC, _MotionBlurDart>(
          'lumit_bridge_set_motion_blur',
        ),
        _addMask = lib.lookupFunction<_Str3C, _Str3Dart>(
          'lumit_bridge_add_mask',
        ),
        _exportPreset = lib.lookupFunction<_Str3C, _Str3Dart>(
          'lumit_bridge_export_preset',
        ),
        _startExport = lib.lookupFunction<_Str3C, _Str3Dart>(
          'lumit_bridge_start_export',
        ),
        _exportPoll = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_export_poll',
        ),
        _exportCancel = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_export_cancel',
        ),
        _decodeFrame = lib.lookupFunction<_DecodeC, _DecodeDart>(
          'lumit_bridge_decode_frame',
        ),
        _cutClipAtPlayhead = lib.lookupFunction<_Str2IntC, _Str2IntDart>(
          'lumit_bridge_cut_clip_at_playhead',
        ),
        _deleteClipAtPlayhead = lib.lookupFunction<_Str2IntC, _Str2IntDart>(
          'lumit_bridge_delete_clip_at_playhead',
        ),
        _detectBeats = lib.lookupFunction<_MarkerC, _MarkerDart>(
          'lumit_bridge_detect_beats',
        ),
        _clearBeatMarkers = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_clear_beat_markers',
        ),
        _deleteItem = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_delete_item',
        ),
        _renameItem = lib.lookupFunction<_Str2C, _Str2Dart>(
          'lumit_bridge_rename_item',
        ),
        _moveToRoot = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_move_to_root',
        ),
        _relink = lib.lookupFunction<_Str2C, _Str2Dart>(
          'lumit_bridge_relink',
        ),
        _renameLayer = lib.lookupFunction<_Str3C, _Str3Dart>(
          'lumit_bridge_rename_layer',
        ),
        _convertToSequenced = lib.lookupFunction<_Str2C, _Str2Dart>(
          'lumit_bridge_convert_to_sequenced',
        ),
        _trimToSourceEnd = lib.lookupFunction<_Str2C, _Str2Dart>(
          'lumit_bridge_trim_to_source_end',
        ),
        _setRetimeReverse = lib.lookupFunction<_Str2BoolC, _Str2BoolDart>(
          'lumit_bridge_set_retime_reverse',
        ),
        _setRetimeInterpolation = lib.lookupFunction<_Str3C, _Str3Dart>(
          'lumit_bridge_set_retime_interpolation',
        ),
        _autosave = lib.lookupFunction<_MarkerC, _MarkerDart>(
          'lumit_bridge_autosave',
        ),
        _listAutosaves = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_list_autosaves',
        ),
        _restoreJournal = lib.lookupFunction<_StrArgC, _StrArgDart>(
          'lumit_bridge_restore_journal',
        ),
        _bootLog = lib.lookupFunction<_NoArgC, _NoArgDart>(
          'lumit_bridge_boot_log',
        ),
        _setTextContent = lib.lookupFunction<_TextContentC, _TextContentDart>(
          'lumit_bridge_set_text_content',
        ),
        _setSolid = lib.lookupFunction<_SetSolidC, _SetSolidDart>(
          'lumit_bridge_set_solid',
        ),
        _setCameraZoom = lib.lookupFunction<_Str2DoubleC, _Str2DoubleDart>(
          'lumit_bridge_set_camera_zoom',
        ),
        _setEffectParamChoice =
            lib.lookupFunction<_EffectU32C, _EffectU32Dart>(
          'lumit_bridge_set_effect_param_choice',
        ),
        _setEffectParamBool =
            lib.lookupFunction<_EffectBoolC, _EffectBoolDart>(
          'lumit_bridge_set_effect_param_bool',
        ),
        _setEffectParamSeed =
            lib.lookupFunction<_EffectU32C, _EffectU32Dart>(
          'lumit_bridge_set_effect_param_seed',
        ),
        _setEffectParamPoint =
            lib.lookupFunction<_EffectPointC, _EffectPointDart>(
          'lumit_bridge_set_effect_param_point',
        ),
        _reorderEffect = lib.lookupFunction<_Str3IntC, _Str3IntDart>(
          'lumit_bridge_reorder_effect',
        ),
        _applyKeyframeBatch = lib.lookupFunction<_Str3C, _Str3Dart>(
          'lumit_bridge_apply_keyframe_batch',
        ),
        _freeString = lib.lookupFunction<_FreeC, _FreeDart>(
          'lumit_bridge_free_string',
        ),
        _freeBuffer = lib.lookupFunction<_FreeBufferC, _FreeBufferDart>(
          'lumit_bridge_free_buffer',
        ) {
    // The composited-comp render symbol is optional: an older library omits it,
    // and the frontend must still load and run on its single-layer path. Bind it
    // defensively so a missing symbol leaves [_renderCompFrame] null rather than
    // throwing out of [tryLoad].
    try {
      _renderCompFrame = lib.lookupFunction<_RenderC, _RenderDart>(
        'lumit_bridge_render_comp_frame',
      );
    } catch (_) {
      _renderCompFrame = null;
    }
    // The transform-preview fast-path symbols (ABI 11) are optional together —
    // an older library omits all three, and only make sense as a set — so bind
    // them in one block and leave the capability off if any is missing.
    try {
      _previewTransform = lib.lookupFunction<_TransformC, _TransformDart>(
        'lumit_bridge_preview_transform',
      );
      _cancelTransformPreview = lib.lookupFunction<_NoArgC, _NoArgDart>(
        'lumit_bridge_cancel_transform_preview',
      );
      _renderPreviewFrame = lib.lookupFunction<_RenderC, _RenderDart>(
        'lumit_bridge_render_comp_frame_preview',
      );
    } catch (_) {
      _previewTransform = null;
      _cancelTransformPreview = null;
      _renderPreviewFrame = null;
    }
    // The shared-texture symbols are likewise optional (K-177): an older library
    // omits them, so bind defensively and leave the capability off if either is
    // missing.
    try {
      _sharedSupported = lib.lookupFunction<_SharedSupportedC,
          _SharedSupportedDart>('lumit_bridge_shared_supported');
      _renderToShared = lib.lookupFunction<_RenderSharedC, _RenderSharedDart>(
        'lumit_bridge_render_to_shared',
      );
    } catch (_) {
      _sharedSupported = null;
      _renderToShared = null;
    }
    // The Linux DMA-BUF render entry point is a newer, separate symbol — bind it
    // independently so a library that has the Windows shared symbol but not this
    // one (or vice versa) still binds what it can.
    try {
      _renderToSharedDmabuf =
          lib.lookupFunction<_RenderSharedDmabufC, _RenderSharedDmabufDart>(
        'lumit_bridge_render_to_shared_dmabuf',
      );
    } catch (_) {
      _renderToSharedDmabuf = null;
    }
    // The ABI-8 cache/cancel/thumbnail symbols are optional (an older library
    // omits them). Bind each independently so a partial upgrade still offers
    // what it can.
    try {
      _clearCache = lib.lookupFunction<_NoArgC, _NoArgDart>(
        'lumit_bridge_clear_cache',
      );
      _setCacheBudget = lib.lookupFunction<_U64ArgC, _U64ArgDart>(
        'lumit_bridge_set_cache_budget',
      );
      _cacheStats = lib.lookupFunction<_NoArgC, _NoArgDart>(
        'lumit_bridge_cache_stats',
      );
    } catch (_) {
      _clearCache = null;
      _setCacheBudget = null;
      _cacheStats = null;
    }
    try {
      _renderCancelStale = lib.lookupFunction<_U64ArgC, _U64ArgDart>(
        'lumit_bridge_render_cancel_stale',
      );
    } catch (_) {
      _renderCancelStale = null;
    }
    try {
      _thumbnail = lib.lookupFunction<_ThumbC, _ThumbDart>(
        'lumit_bridge_thumbnail',
      );
    } catch (_) {
      _thumbnail = null;
    }
    // The GPU scope-pass symbol (K-096 v1) is optional: an older library omits
    // it, so bind defensively and keep the Scopes panel on its CPU trace then.
    try {
      _renderScope = lib.lookupFunction<_RenderScopeC, _RenderScopeDart>(
        'lumit_bridge_render_scope',
      );
    } catch (_) {
      _renderScope = null;
    }
    // The ABI-9 symbols (mask geometry, effect-param keyframes, presets, the
    // realtime tier readout) are optional: an older library omits them, so bind
    // each group defensively and leave the method degrading to an unsupported
    // reply when the symbol is missing.
    try {
      _addMaskGeometry = lib.lookupFunction<_MaskGeomC, _MaskGeomDart>(
        'lumit_bridge_add_mask_geometry',
      );
    } catch (_) {
      _addMaskGeometry = null;
    }
    try {
      _toggleEffectParamAnimated =
          lib.lookupFunction<_EffectKeyC, _EffectKeyDart>(
        'lumit_bridge_toggle_effect_param_animated',
      );
      _addEffectParamKeyframe =
          lib.lookupFunction<_EffectAddKeyC, _EffectAddKeyDart>(
        'lumit_bridge_add_effect_param_keyframe',
      );
      _removeEffectParamKeyframe =
          lib.lookupFunction<_EffectKeyC, _EffectKeyDart>(
        'lumit_bridge_remove_effect_param_keyframe',
      );
      _shiftEffectParamKeyframes =
          lib.lookupFunction<_EffectShiftC, _EffectShiftDart>(
        'lumit_bridge_shift_effect_param_keyframes',
      );
      _setEffectParamKeyframeInterp =
          lib.lookupFunction<_EffectKeyInterpC, _EffectKeyInterpDart>(
        'lumit_bridge_set_effect_param_keyframe_interp',
      );
    } catch (_) {
      _toggleEffectParamAnimated = null;
      _addEffectParamKeyframe = null;
      _removeEffectParamKeyframe = null;
      _shiftEffectParamKeyframes = null;
      _setEffectParamKeyframeInterp = null;
    }
    try {
      _saveEffectPreset = lib.lookupFunction<_Str3C, _Str3Dart>(
        'lumit_bridge_save_effect_preset',
      );
      _loadEffectPreset = lib.lookupFunction<_Str3C, _Str3Dart>(
        'lumit_bridge_load_effect_preset',
      );
    } catch (_) {
      _saveEffectPreset = null;
      _loadEffectPreset = null;
    }
    try {
      _playbackTier = lib.lookupFunction<_NoArgC, _NoArgDart>(
        'lumit_bridge_playback_tier',
      );
      _resetRealtime = lib.lookupFunction<_NoArgC, _NoArgDart>(
        'lumit_bridge_reset_realtime',
      );
    } catch (_) {
      _playbackTier = null;
      _resetRealtime = null;
    }
    // The ABI-10 audio playback symbols (docs/09): bound as a group so a
    // partially-matching library reports the capability off rather than half
    // working. The clock's out-pointers are allocated once here, so the
    // per-tick poll allocates nothing.
    try {
      _audioPrepare = lib.lookupFunction<_StrArgC, _StrArgDart>(
        'lumit_bridge_audio_prepare',
      );
      _audioPlay = lib.lookupFunction<_AudioPlayC, _AudioPlayDart>(
        'lumit_bridge_audio_play',
      );
      _audioPause = lib.lookupFunction<_NoArgC, _NoArgDart>(
        'lumit_bridge_audio_pause',
      );
      _audioSeek = lib.lookupFunction<_DoubleArgC, _DoubleArgDart>(
        'lumit_bridge_audio_seek',
      );
      _audioStop = lib.lookupFunction<_NoArgC, _NoArgDart>(
        'lumit_bridge_audio_stop',
      );
      _audioClock = lib.lookupFunction<_AudioClockC, _AudioClockDart>(
        'lumit_bridge_audio_clock',
      );
      _audioClockSecs = malloc<Double>();
      _audioClockPlaying = malloc<Bool>();
      _audioClockLoaded = malloc<Bool>();
    } catch (_) {
      _audioPrepare = null;
      _audioPlay = null;
      _audioPause = null;
      _audioSeek = null;
      _audioStop = null;
      _audioClock = null;
    }
  }

  /// The filesystem path the library was actually opened from, when a candidate
  /// path (not the bare OS-resolved name) succeeded. The render isolate opens
  /// its OWN handle to the same file — same process, so the same engine state
  /// behind the bridge's process-wide `Mutex` — so it needs this exact path.
  /// Null when the bare name resolved through the OS loader's search path
  /// (the worker then tries the bare name too).
  String? loadedPath;

  /// Load the library and bind it, or return null if it cannot be found or a
  /// symbol is missing. Never throws — a failure is just "run on placeholders".
  static LumitBridge? tryLoad() {
    for (final candidate in _candidatePaths()) {
      try {
        final lib = DynamicLibrary.open(candidate);
        final bridge = LumitBridge._(lib);
        bridge.loadedPath = candidate;
        return bridge;
      } catch (_) {
        // Try the next candidate.
      }
    }
    return null;
  }

  /// The candidate library paths, in load order — exposed so the render isolate
  /// can open the same `lumit_bridge.dll` in its own worker.
  static List<String> candidateLibraryPaths() => _candidatePaths();

  /// Where the library might live, in the order the runner should try:
  /// beside the executable first (the shipped layout), then the Cargo debug
  /// output relative to the working directory (the developer layout), then the
  /// bare name so the OS loader's own search path gets a turn.
  static List<String> _candidatePaths() {
    // The shared library's platform base name: `lumit_bridge.dll` on Windows
    // (cdylib), `liblumit_bridge.so` on Linux, `liblumit_bridge.dylib` on macOS
    // (the `lib` prefix Cargo gives a cdylib on both Unix targets). The search
    // ORDER below is identical on every platform, so Windows behaviour is
    // byte-for-byte what it was.
    final name = Platform.isWindows
        ? 'lumit_bridge.dll'
        : Platform.isMacOS
            ? 'liblumit_bridge.dylib'
            : 'liblumit_bridge.so';
    final paths = <String>[];
    try {
      final exeDir = File(Platform.resolvedExecutable).parent.path;
      paths.add('$exeDir${Platform.pathSeparator}$name');
    } catch (_) {
      // resolvedExecutable can be unavailable in some hosts; skip it.
    }
    final cwd = Directory.current.path;
    final sep = Platform.pathSeparator;
    // Release before debug: the shipped build instruction is
    // `cargo build -p lumit-bridge --release --features shared-texture`, so a
    // machine with both prefers the optimised library.
    for (final profile in ['release', 'debug']) {
      paths.add('$cwd$sep..$sep..$sep..${sep}target$sep$profile$sep$name');
      paths.add('$cwd$sep..${sep}target$sep$profile$sep$name');
    }
    paths.add(name);
    return paths;
  }

  /// `{"version":"…","abi":1,"ok":true}` as the raw decoded map, or null if the
  /// reply is malformed. Used for a boot-time handshake / log line.
  Map<String, dynamic>? version() {
    final raw = _callNoArg(_version);
    try {
      final decoded = jsonDecode(raw);
      return decoded is Map ? decoded.cast<String, dynamic>() : null;
    } catch (_) {
      return null;
    }
  }

  @override
  BridgeReply snapshot() => BridgeReply.parse(_callNoArg(_snapshot));
  @override
  BridgeReply newProject() => BridgeReply.parse(_callNoArg(_newProject));
  @override
  BridgeReply undo() => BridgeReply.parse(_callNoArg(_undo));
  @override
  BridgeReply redo() => BridgeReply.parse(_callNoArg(_redo));

  @override
  BridgeReply openProject(String path) =>
      BridgeReply.parse(_callStrArg(_openProject, path));

  /// Save to [path]; an empty string saves to the loaded path (an error reply
  /// if the document has never been saved).
  @override
  BridgeReply saveProject(String path) =>
      BridgeReply.parse(_callStrArg(_saveProject, path));

  @override
  BridgeReply newComposition(String name) =>
      BridgeReply.parse(_callStrArg(_newComposition, name));

  @override
  BridgeReply importFootage(String path) =>
      BridgeReply.parse(_callStrArg(_importFootage, path));

  @override
  BridgeReply setLayerSwitch(
    String compId,
    String layerId,
    String switchName,
    bool value,
  ) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final s = switchName.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(
        _setLayerSwitch(c.cast(), l.cast(), s.cast(), value),
      ));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(s);
    }
  }

  @override
  BridgeReply editLayerSpan(
    String compId,
    String layerId,
    String edit,
    int frame,
  ) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final e = edit.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(
        _editLayerSpan(c.cast(), l.cast(), e.cast(), frame),
      ));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(e);
    }
  }

  @override
  BridgeReply setTransform(
    String compId,
    String layerId,
    String property,
    double value,
  ) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final p = property.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(
        _setTransform(c.cast(), l.cast(), p.cast(), value),
      ));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(p);
    }
  }

  @override
  bool get supportsPreviewTransform =>
      _previewTransform != null &&
      _cancelTransformPreview != null &&
      _renderPreviewFrame != null;

  @override
  BridgeReply previewTransform(
    String compId,
    String layerId,
    String property,
    double value,
  ) {
    final fn = _previewTransform;
    if (fn == null) {
      return const BridgeReply.err('preview transform unsupported');
    }
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final p = property.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(fn(c.cast(), l.cast(), p.cast(), value)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(p);
    }
  }

  @override
  void cancelTransformPreview() {
    final fn = _cancelTransformPreview;
    if (fn == null) return;
    _readReply(fn()); // a tiny stateless ack — nothing to adopt
  }

  @override
  BridgeReply addMarker(String compId, int frame) {
    final c = compId.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(_addMarker(c.cast(), frame)));
    } finally {
      malloc.free(c);
    }
  }

  // --- Bridge v0.3 --------------------------------------------------------

  /// Call a one-comp-id op, freeing the argument after the reply is copied.
  BridgeReply _compArgOp(_StrArgDart fn, String compId) {
    final c = compId.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(fn(c.cast())));
    } finally {
      malloc.free(c);
    }
  }

  /// Call a two-string op, freeing both arguments afterwards.
  BridgeReply _twoStrOp(_Str2Dart fn, String a, String b) {
    final pa = a.toNativeUtf8();
    final pb = b.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(fn(pa.cast(), pb.cast())));
    } finally {
      malloc.free(pa);
      malloc.free(pb);
    }
  }

  @override
  BridgeReply addSolidLayer(String compId) => _compArgOp(_addSolidLayer, compId);
  @override
  BridgeReply addTextLayer(String compId) => _compArgOp(_addTextLayer, compId);
  @override
  BridgeReply addCameraLayer(String compId) =>
      _compArgOp(_addCameraLayer, compId);
  @override
  BridgeReply addAdjustmentLayer(String compId) =>
      _compArgOp(_addAdjustmentLayer, compId);
  @override
  BridgeReply addSequenceLayer(String compId) =>
      _compArgOp(_addSequenceLayer, compId);

  @override
  BridgeReply addFootageLayer(String compId, String itemId) =>
      _twoStrOp(_addFootageLayer, compId, itemId);

  @override
  BridgeReply reorderLayer(String compId, String layerId, int newIndex) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_reorderLayer(c.cast(), l.cast(), newIndex)));
    } finally {
      malloc.free(c);
      malloc.free(l);
    }
  }

  @override
  BridgeReply deleteLayer(String compId, String layerId) =>
      _twoStrOp(_deleteLayer, compId, layerId);
  @override
  BridgeReply duplicateLayer(String compId, String layerId) =>
      _twoStrOp(_duplicateLayer, compId, layerId);

  @override
  BridgeReply setCompSettings(String compId, String name, int width, int height,
      int fpsNum, int fpsDen, int durationFrames) {
    final c = compId.toNativeUtf8();
    final n = name.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(_setCompSettings(
          c.cast(), n.cast(), width, height, fpsNum, fpsDen, durationFrames)));
    } finally {
      malloc.free(c);
      malloc.free(n);
    }
  }

  @override
  BridgeReply togglePropertyAnimated(
      String compId, String layerId, String property, int frame) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final p = property.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(
          _togglePropertyAnimated(c.cast(), l.cast(), p.cast(), frame)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(p);
    }
  }

  @override
  BridgeReply addKeyframe(
      String compId, String layerId, String property, int frame, double value) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final p = property.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(
          _addKeyframe(c.cast(), l.cast(), p.cast(), frame, value)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(p);
    }
  }

  @override
  BridgeReply removeKeyframe(
      String compId, String layerId, String property, int frame) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final p = property.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_removeKeyframe(c.cast(), l.cast(), p.cast(), frame)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(p);
    }
  }

  @override
  BridgeReply shiftKeyframes(String compId, String layerId, String property,
      List<int> frames, int delta) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final p = property.toNativeUtf8();
    final f = jsonEncode(frames).toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(
          _shiftKeyframes(c.cast(), l.cast(), p.cast(), f.cast(), delta)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(p);
      malloc.free(f);
    }
  }

  @override
  BridgeReply setWorkAreaEdge(String compId, int frame, bool isOut) {
    final c = compId.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_setWorkAreaEdge(c.cast(), frame, isOut)));
    } finally {
      malloc.free(c);
    }
  }

  @override
  List<BridgeEffectInfo> listEffects() {
    final raw = _readReply(_listEffects());
    try {
      final decoded = jsonDecode(raw);
      if (decoded is! Map || decoded['ok'] != true) return const [];
      final effects = decoded['effects'];
      if (effects is! List) return const [];
      return [
        for (final e in effects)
          if (e is Map) BridgeEffectInfo.fromJson(e.cast<String, dynamic>()),
      ];
    } catch (_) {
      return const [];
    }
  }

  @override
  BridgeReply addEffect(String compId, String layerId, String effectName) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final e = effectName.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_addEffect(c.cast(), l.cast(), e.cast())));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(e);
    }
  }

  @override
  BridgeReply removeEffect(String compId, String layerId, String effectId) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final e = effectId.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_removeEffect(c.cast(), l.cast(), e.cast())));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(e);
    }
  }

  @override
  BridgeReply setEffectEnabled(
      String compId, String layerId, String effectId, bool enabled) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final e = effectId.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(
          _setEffectEnabled(c.cast(), l.cast(), e.cast(), enabled)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(e);
    }
  }

  @override
  BridgeReply setEffectParamScalar(String compId, String layerId,
      String effectId, String paramName, double value) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final e = effectId.toNativeUtf8();
    final p = paramName.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(
          _setEffectParamScalar(c.cast(), l.cast(), e.cast(), p.cast(), value)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(e);
      malloc.free(p);
    }
  }

  @override
  BridgeReply setEffectParamColour(String compId, String layerId,
      String effectId, String paramName, double r, double g, double b,
      double a) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final e = effectId.toNativeUtf8();
    final p = paramName.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(_setEffectParamColour(
          c.cast(), l.cast(), e.cast(), p.cast(), r, g, b, a)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(e);
      malloc.free(p);
    }
  }

  // --- Bridge v0.4 --------------------------------------------------------

  @override
  BridgeReply setKeyframeInterp(
      String compId,
      String layerId,
      String property,
      int frame,
      String interpIn,
      String interpOut,
      double speedIn,
      double influenceIn,
      double speedOut,
      double influenceOut) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final p = property.toNativeUtf8();
    final ii = interpIn.toNativeUtf8();
    final io = interpOut.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(_setKeyframeInterp(c.cast(), l.cast(),
          p.cast(), frame, ii.cast(), io.cast(), speedIn, influenceIn, speedOut,
          influenceOut)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(p);
      malloc.free(ii);
      malloc.free(io);
    }
  }

  @override
  BridgeReply setRetimeEnabled(String compId, String layerId, bool enabled) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_setRetimeEnabled(c.cast(), l.cast(), enabled)));
    } finally {
      malloc.free(c);
      malloc.free(l);
    }
  }

  @override
  BridgeReply setRetimeSpeed(
      String compId, String layerId, double speedPercent) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_setRetimeSpeed(c.cast(), l.cast(), speedPercent)));
    } finally {
      malloc.free(c);
      malloc.free(l);
    }
  }

  @override
  BridgeReply setSegmentPreset(
      String compId, String layerId, int frame, String ease) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final e = ease.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_setSegmentPreset(c.cast(), l.cast(), frame, e.cast())));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(e);
    }
  }

  @override
  BridgeReply segmentToRate(String compId, String layerId, int frame) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_segmentToRate(c.cast(), l.cast(), frame)));
    } finally {
      malloc.free(c);
      malloc.free(l);
    }
  }

  @override
  BridgeReply dragBoundary(
      String compId, String layerId, int index, int frame) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_dragBoundary(c.cast(), l.cast(), index, frame)));
    } finally {
      malloc.free(c);
      malloc.free(l);
    }
  }

  @override
  List<BridgeBlendMode> listBlendModes() {
    final raw = _readReply(_listBlendModes());
    try {
      final decoded = jsonDecode(raw);
      if (decoded is! Map || decoded['ok'] != true) return const [];
      final modes = decoded['blend_modes'];
      if (modes is! List) return const [];
      return [
        for (final m in modes)
          if (m is Map) BridgeBlendMode.fromJson(m.cast<String, dynamic>()),
      ];
    } catch (_) {
      return const [];
    }
  }

  @override
  BridgeReply setBlendMode(String compId, String layerId, String mode) =>
      _threeStrOp(_setBlendMode, compId, layerId, mode);

  @override
  BridgeReply setMatte(String compId, String layerId, String source,
      String channel, bool inverted) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final s = source.toNativeUtf8();
    final ch = channel.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(
          _setMatte(c.cast(), l.cast(), s.cast(), ch.cast(), inverted)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(s);
      malloc.free(ch);
    }
  }

  @override
  BridgeReply setParent(String compId, String layerId, String parent) =>
      _threeStrOp(_setParent, compId, layerId, parent);

  @override
  BridgeReply setMotionBlur(String compId, bool enabled, double shutterAngle,
      double shutterPhase, int samples) {
    final c = compId.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(_setMotionBlur(
          c.cast(), enabled, shutterAngle, shutterPhase, samples)));
    } finally {
      malloc.free(c);
    }
  }

  @override
  BridgeReply addMask(String compId, String layerId, String kind) =>
      _threeStrOp(_addMask, compId, layerId, kind);

  @override
  BridgeExportPreset exportPreset(
      String presetName, String compName, String template) {
    final p = presetName.toNativeUtf8();
    final c = compName.toNativeUtf8();
    final t = template.toNativeUtf8();
    try {
      final raw = _readReply(_exportPreset(p.cast(), c.cast(), t.cast()));
      final decoded = jsonDecode(raw);
      if (decoded is Map && decoded['ok'] == true) {
        return BridgeExportPreset.fromJson(decoded.cast<String, dynamic>());
      }
      return BridgeExportPreset.idle;
    } catch (_) {
      return BridgeExportPreset.idle;
    } finally {
      malloc.free(p);
      malloc.free(c);
      malloc.free(t);
    }
  }

  @override
  BridgeReply startExport(String compId, String specJson, String outPath) =>
      _threeStrOp(_startExport, compId, specJson, outPath);

  @override
  BridgeExportState exportPoll() {
    final raw = _readReply(_exportPoll());
    try {
      final decoded = jsonDecode(raw);
      if (decoded is Map && decoded['ok'] == true) {
        return BridgeExportState.fromJson(decoded.cast<String, dynamic>());
      }
      return BridgeExportState.idle;
    } catch (_) {
      return BridgeExportState.idle;
    }
  }

  @override
  BridgeReply exportCancel() => BridgeReply.parse(_callNoArg(_exportCancel));

  /// Call a three-string op, freeing all three arguments afterwards.
  BridgeReply _threeStrOp(_Str3Dart fn, String a, String b, String c) {
    final pa = a.toNativeUtf8();
    final pb = b.toNativeUtf8();
    final pc = c.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(fn(pa.cast(), pb.cast(), pc.cast())));
    } finally {
      malloc.free(pa);
      malloc.free(pb);
      malloc.free(pc);
    }
  }

  @override
  DecodedFrame? decodeFrame(String itemId, int frame) {
    final id = itemId.toNativeUtf8();
    final outW = malloc<Uint32>();
    final outH = malloc<Uint32>();
    final outLen = malloc<Size>();
    try {
      final ptr = _decodeFrame(id.cast(), frame, outW, outH, outLen);
      if (ptr == nullptr) return null;
      final len = outLen.value;
      try {
        // Copy the pixels out before the buffer is freed back to Rust.
        final rgba = Uint8List.fromList(ptr.asTypedList(len));
        return DecodedFrame(
          width: outW.value,
          height: outH.value,
          rgba: rgba,
        );
      } finally {
        _freeBuffer(ptr, len);
      }
    } finally {
      malloc.free(id);
      malloc.free(outW);
      malloc.free(outH);
      malloc.free(outLen);
    }
  }

  @override
  bool get supportsCompRender => _renderCompFrame != null;

  @override
  DecodedFrame? renderCompFrame(String compId, int frame, double scale) {
    final render = _renderCompFrame;
    if (render == null) return null; // old library without the symbol
    final id = compId.toNativeUtf8();
    final outW = malloc<Uint32>();
    final outH = malloc<Uint32>();
    final outLen = malloc<Size>();
    try {
      final ptr = render(id.cast(), frame, scale, outW, outH, outLen);
      if (ptr == nullptr) return null;
      final len = outLen.value;
      try {
        // Copy the pixels out before the buffer is freed back to Rust — the same
        // contract as decodeFrame (one boxed slice, freed as a whole).
        final rgba = Uint8List.fromList(ptr.asTypedList(len));
        return DecodedFrame(
          width: outW.value,
          height: outH.value,
          rgba: rgba,
        );
      } finally {
        _freeBuffer(ptr, len);
      }
    } finally {
      malloc.free(id);
      malloc.free(outW);
      malloc.free(outH);
      malloc.free(outLen);
    }
  }

  @override
  DecodedFrame? renderPreviewFrame(String compId, int frame, double scale) {
    final render = _renderPreviewFrame;
    if (render == null) return null; // old library, or unsupported — see supportsPreviewTransform
    final id = compId.toNativeUtf8();
    final outW = malloc<Uint32>();
    final outH = malloc<Uint32>();
    final outLen = malloc<Size>();
    try {
      final ptr = render(id.cast(), frame, scale, outW, outH, outLen);
      if (ptr == nullptr) return null;
      final len = outLen.value;
      try {
        // Same copy-then-free contract as renderCompFrame/decodeFrame.
        final rgba = Uint8List.fromList(ptr.asTypedList(len));
        return DecodedFrame(
          width: outW.value,
          height: outH.value,
          rgba: rgba,
        );
      } finally {
        _freeBuffer(ptr, len);
      }
    } finally {
      malloc.free(id);
      malloc.free(outW);
      malloc.free(outH);
      malloc.free(outLen);
    }
  }

  @override
  bool get supportsSharedTexture {
    final supported = _sharedSupported;
    // Both the presence of the symbol AND the engine's own answer must agree:
    // an old library has no symbol; a feature-less or unsupported-platform build
    // has the symbol but answers false. The per-platform render entry point must
    // also be bound — the DMA-BUF one on Linux, the shared-handle one elsewhere.
    if (supported == null || !supported()) return false;
    return Platform.isLinux
        ? _renderToSharedDmabuf != null
        : _renderToShared != null;
  }

  @override
  SharedFrame? renderToShared(String compId, int frame) {
    // Linux renders into a Vulkan image exported as a DMA-BUF; every other
    // platform uses the DXGI shared-handle path. The Windows ABI is untouched —
    // the two are separate engine exports and the platform picks between them.
    return Platform.isLinux
        ? _renderSharedViaDmabuf(compId, frame)
        : _renderToSharedHandle(compId, frame);
  }

  /// The Windows/DXGI shared-handle render (K-177): comp `id` at `frame` into the
  /// shared texture, returning its NT handle and size. Null on failure.
  SharedFrame? _renderToSharedHandle(String compId, int frame) {
    final render = _renderToShared;
    if (render == null) return null; // old library without the symbol
    final id = compId.toNativeUtf8();
    final outHandle = malloc<Uint64>();
    final outW = malloc<Uint32>();
    final outH = malloc<Uint32>();
    try {
      final ok = render(id.cast(), frame, outHandle, outW, outH);
      if (!ok || outHandle.value == 0 || outW.value == 0 || outH.value == 0) {
        return null;
      }
      return SharedFrame(
        handle: outHandle.value,
        width: outW.value,
        height: outH.value,
      );
    } finally {
      malloc.free(id);
      malloc.free(outHandle);
      malloc.free(outW);
      malloc.free(outH);
    }
  }

  /// The Linux DMA-BUF render (K-177): comp `id` at `frame` into the exported
  /// image, returning its fd + DRM metadata as a [SharedFrame]. Null on failure
  /// (no capable Vulkan adapter, missing extensions, a transient error), which
  /// sends the Viewer back to the read-back path for that frame.
  SharedFrame? _renderSharedViaDmabuf(String compId, int frame) {
    final render = _renderToSharedDmabuf;
    if (render == null) return null; // library without the DMA-BUF symbol
    final id = compId.toNativeUtf8();
    final outFd = malloc<Int32>();
    final outW = malloc<Uint32>();
    final outH = malloc<Uint32>();
    final outStride = malloc<Uint32>();
    final outOffset = malloc<Uint32>();
    final outFourcc = malloc<Uint32>();
    final outModifier = malloc<Uint64>();
    try {
      final ok = render(id.cast(), frame, outFd, outW, outH, outStride,
          outOffset, outFourcc, outModifier);
      if (!ok || outFd.value < 0 || outW.value == 0 || outH.value == 0) {
        return null;
      }
      return SharedFrame(
        handle: 0,
        width: outW.value,
        height: outH.value,
        fd: outFd.value,
        stride: outStride.value,
        offset: outOffset.value,
        fourcc: outFourcc.value,
        modifier: outModifier.value,
      );
    } finally {
      malloc.free(id);
      malloc.free(outFd);
      malloc.free(outW);
      malloc.free(outH);
      malloc.free(outStride);
      malloc.free(outOffset);
      malloc.free(outFourcc);
      malloc.free(outModifier);
    }
  }

  @override
  bool get supportsScopeTrace => _renderScope != null;

  @override
  Uint8List? renderScope(int kind, String compId, int frame, double scale,
      int bg, int trace, int red, int green, int blue) {
    final fn = _renderScope;
    if (fn == null) return null; // old library without the symbol
    final id = compId.toNativeUtf8();
    final outLen = malloc<Size>();
    try {
      final ptr =
          fn(kind, id.cast(), frame, scale, bg, trace, red, green, blue, outLen);
      if (ptr == nullptr) return null;
      final len = outLen.value;
      try {
        // Copy the trace out before the buffer is freed back to Rust — the same
        // contract as renderCompFrame (one boxed slice, freed as a whole).
        return Uint8List.fromList(ptr.asTypedList(len));
      } finally {
        _freeBuffer(ptr, len);
      }
    } finally {
      malloc.free(id);
      malloc.free(outLen);
    }
  }

  // --- Bridge v0.8: cache controls, render cancellation, thumbnails ------

  @override
  bool get supportsCacheControl =>
      _clearCache != null && _setCacheBudget != null && _cacheStats != null;

  @override
  BridgeCacheStats clearCache() {
    final fn = _clearCache;
    if (fn == null) return BridgeCacheStats.empty;
    return _parseCacheStats(_callNoArg(fn));
  }

  @override
  BridgeCacheStats setCacheBudget(int bytes) {
    final fn = _setCacheBudget;
    if (fn == null) return BridgeCacheStats.empty;
    return _parseCacheStats(_readReply(fn(bytes)));
  }

  @override
  BridgeCacheStats cacheStats() {
    final fn = _cacheStats;
    if (fn == null) return BridgeCacheStats.empty;
    return _parseCacheStats(_callNoArg(fn));
  }

  BridgeCacheStats _parseCacheStats(String raw) {
    try {
      final decoded = jsonDecode(raw);
      if (decoded is Map && decoded['ok'] == true) {
        return BridgeCacheStats.fromJson(decoded.cast<String, dynamic>());
      }
    } catch (_) {}
    return BridgeCacheStats.empty;
  }

  @override
  bool get supportsRenderCancel => _renderCancelStale != null;

  @override
  void renderCancelStale(int generation) {
    final fn = _renderCancelStale;
    if (fn == null) return;
    // Frees the small {"ok":true} reply; the effect is the engine-side atomic.
    _readReply(fn(generation));
  }

  @override
  bool get supportsThumbnail => _thumbnail != null;

  @override
  DecodedFrame? thumbnail(String itemId, int maxEdge) {
    final fn = _thumbnail;
    if (fn == null) return null;
    final id = itemId.toNativeUtf8();
    final outW = malloc<Uint32>();
    final outH = malloc<Uint32>();
    final outLen = malloc<Size>();
    try {
      final ptr = fn(id.cast(), maxEdge, outW, outH, outLen);
      if (ptr == nullptr) return null;
      final len = outLen.value;
      try {
        // Copy the pixels out before the buffer is freed back to Rust — the same
        // contract as decodeFrame (one boxed slice, freed as a whole).
        final rgba = Uint8List.fromList(ptr.asTypedList(len));
        return DecodedFrame(width: outW.value, height: outH.value, rgba: rgba);
      } finally {
        _freeBuffer(ptr, len);
      }
    } finally {
      malloc.free(id);
      malloc.free(outW);
      malloc.free(outH);
      malloc.free(outLen);
    }
  }

  // --- Bridge v0.10: comp audio playback (docs/09) ------------------------

  @override
  bool get supportsAudioPlayback =>
      _audioPrepare != null &&
      _audioPlay != null &&
      _audioPause != null &&
      _audioSeek != null &&
      _audioStop != null &&
      _audioClock != null;

  @override
  void audioPrepare(String compId) {
    final fn = _audioPrepare;
    if (fn == null) return;
    // The reply is a bare {"ok":true} (or a calm error already surfaced by
    // the op that caused the edit); free it and move on.
    _callStrArg(fn, compId);
  }

  @override
  void audioPlay(String compId, double startSeconds) {
    final fn = _audioPlay;
    if (fn == null) return;
    final id = compId.toNativeUtf8();
    try {
      _readReply(fn(id.cast(), startSeconds));
    } finally {
      malloc.free(id);
    }
  }

  @override
  void audioPause() {
    final fn = _audioPause;
    if (fn == null) return;
    _callNoArg(fn);
  }

  @override
  void audioSeek(double seconds) {
    final fn = _audioSeek;
    if (fn == null) return;
    _readReply(fn(seconds));
  }

  @override
  void audioStop() {
    final fn = _audioStop;
    if (fn == null) return;
    _callNoArg(fn);
  }

  @override
  AudioClock audioClock() {
    final fn = _audioClock;
    final secs = _audioClockSecs;
    final playing = _audioClockPlaying;
    final loaded = _audioClockLoaded;
    if (fn == null || secs == null || playing == null || loaded == null) {
      return AudioClock.none;
    }
    // The one per-tick call: no allocation on either side of the boundary.
    if (!fn(secs, playing, loaded)) return AudioClock.none;
    return AudioClock(
      seconds: secs.value,
      playing: playing.value,
      loaded: loaded.value,
    );
  }

  // --- Bridge v0.5 ops ---------------------------------------------------

  /// Call a one-string op, freeing the argument afterwards.
  BridgeReply _oneStrOp(_StrArgDart fn, String a) =>
      BridgeReply.parse(_callStrArg(fn, a));

  /// Call a (comp, layer, int) op, freeing the two strings afterwards.
  BridgeReply _twoStrIntOp(_Str2IntDart fn, String a, String b, int n) {
    final pa = a.toNativeUtf8();
    final pb = b.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(fn(pa.cast(), pb.cast(), n)));
    } finally {
      malloc.free(pa);
      malloc.free(pb);
    }
  }

  @override
  BridgeReply cutClipAtPlayhead(String compId, String layerId, int frame) =>
      _twoStrIntOp(_cutClipAtPlayhead, compId, layerId, frame);

  @override
  BridgeReply deleteClipAtPlayhead(String compId, String layerId, int frame) =>
      _twoStrIntOp(_deleteClipAtPlayhead, compId, layerId, frame);

  @override
  BridgeReply detectBeats(String compId, int sensitivity) {
    final p = compId.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(_detectBeats(p.cast(), sensitivity)));
    } finally {
      malloc.free(p);
    }
  }

  @override
  BridgeReply clearBeatMarkers(String compId) =>
      _oneStrOp(_clearBeatMarkers, compId);

  @override
  BridgeReply deleteItem(String itemId) => _oneStrOp(_deleteItem, itemId);

  @override
  BridgeReply renameItem(String itemId, String name) =>
      _twoStrOp(_renameItem, itemId, name);

  @override
  BridgeReply moveToRoot(String itemId) => _oneStrOp(_moveToRoot, itemId);

  @override
  BridgeReply relink(String itemId, String path) =>
      _twoStrOp(_relink, itemId, path);

  @override
  BridgeReply renameLayer(String compId, String layerId, String name) =>
      _threeStrOp(_renameLayer, compId, layerId, name);

  @override
  BridgeReply convertToSequenced(String compId, String layerId) =>
      _twoStrOp(_convertToSequenced, compId, layerId);

  @override
  BridgeReply trimToSourceEnd(String compId, String layerId) =>
      _twoStrOp(_trimToSourceEnd, compId, layerId);

  @override
  BridgeReply setRetimeReverse(String compId, String layerId, bool reverse) {
    final pa = compId.toNativeUtf8();
    final pb = layerId.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_setRetimeReverse(pa.cast(), pb.cast(), reverse)));
    } finally {
      malloc.free(pa);
      malloc.free(pb);
    }
  }

  @override
  BridgeReply setRetimeInterpolation(
          String compId, String layerId, String interp) =>
      _threeStrOp(_setRetimeInterpolation, compId, layerId, interp);

  @override
  BridgeReply setTextContent(String compId, String layerId, String text,
      double size, double r, double g, double b, double a) {
    final pc = compId.toNativeUtf8();
    final pl = layerId.toNativeUtf8();
    final pt = text.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(_setTextContent(
          pc.cast(), pl.cast(), pt.cast(), size, r, g, b, a)));
    } finally {
      malloc.free(pc);
      malloc.free(pl);
      malloc.free(pt);
    }
  }

  @override
  BridgeReply setSolid(String compId, String layerId, double r, double g,
      double b, double a, int width, int height) {
    final pc = compId.toNativeUtf8();
    final pl = layerId.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(
          _setSolid(pc.cast(), pl.cast(), r, g, b, a, width, height)));
    } finally {
      malloc.free(pc);
      malloc.free(pl);
    }
  }

  @override
  BridgeReply setCameraZoom(String compId, String layerId, double zoom) {
    final pc = compId.toNativeUtf8();
    final pl = layerId.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_setCameraZoom(pc.cast(), pl.cast(), zoom)));
    } finally {
      malloc.free(pc);
      malloc.free(pl);
    }
  }

  /// Call a (comp, layer, effect, param, extra) effect-param op, freeing the
  /// four strings afterwards. [invoke] receives the four native pointers.
  BridgeReply _effectParamOp(
      String compId,
      String layerId,
      String effectId,
      String paramName,
      Pointer<Char> Function(
              Pointer<Char>, Pointer<Char>, Pointer<Char>, Pointer<Char>)
          invoke) {
    final pc = compId.toNativeUtf8();
    final pl = layerId.toNativeUtf8();
    final pe = effectId.toNativeUtf8();
    final pp = paramName.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(invoke(pc.cast(), pl.cast(), pe.cast(), pp.cast())));
    } finally {
      malloc.free(pc);
      malloc.free(pl);
      malloc.free(pe);
      malloc.free(pp);
    }
  }

  @override
  BridgeReply setEffectParamChoice(String compId, String layerId,
          String effectId, String paramName, int index) =>
      _effectParamOp(compId, layerId, effectId, paramName,
          (c, l, e, p) => _setEffectParamChoice(c, l, e, p, index));

  @override
  BridgeReply setEffectParamBool(String compId, String layerId, String effectId,
          String paramName, bool value) =>
      _effectParamOp(compId, layerId, effectId, paramName,
          (c, l, e, p) => _setEffectParamBool(c, l, e, p, value));

  @override
  BridgeReply setEffectParamSeed(String compId, String layerId, String effectId,
          String paramName, int seed) =>
      _effectParamOp(compId, layerId, effectId, paramName,
          (c, l, e, p) => _setEffectParamSeed(c, l, e, p, seed));

  @override
  BridgeReply setEffectParamPoint(String compId, String layerId,
          String effectId, String paramName, double x, double y) =>
      _effectParamOp(compId, layerId, effectId, paramName,
          (c, l, e, p) => _setEffectParamPoint(c, l, e, p, x, y));

  @override
  BridgeReply reorderEffect(
      String compId, String layerId, String effectId, int newIndex) {
    final pc = compId.toNativeUtf8();
    final pl = layerId.toNativeUtf8();
    final pe = effectId.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(_reorderEffect(pc.cast(), pl.cast(), pe.cast(), newIndex)));
    } finally {
      malloc.free(pc);
      malloc.free(pl);
      malloc.free(pe);
    }
  }

  @override
  BridgeReply applyKeyframeBatch(
          String compId, String layerId, String opsJson) =>
      _threeStrOp(_applyKeyframeBatch, compId, layerId, opsJson);

  @override
  BridgeReply autosave(String path, int keep) {
    final p = path.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(_autosave(p.cast(), keep)));
    } finally {
      malloc.free(p);
    }
  }

  @override
  List<BridgeAutosave> listAutosaves(String path) {
    final raw = _callStrArg(_listAutosaves, path);
    try {
      final decoded = jsonDecode(raw);
      if (decoded is Map && decoded['ok'] == true) {
        final list = decoded['autosaves'];
        if (list is List) {
          return list
              .whereType<Map>()
              .map((m) => BridgeAutosave.fromJson(m.cast<String, dynamic>()))
              .toList();
        }
      }
    } catch (_) {}
    return const [];
  }

  @override
  BridgeReply restoreJournal(String path) =>
      BridgeReply.parse(_callStrArg(_restoreJournal, path));

  @override
  List<String> bootLog() {
    final raw = _callNoArg(_bootLog);
    try {
      final decoded = jsonDecode(raw);
      if (decoded is Map && decoded['ok'] == true) {
        final lines = decoded['lines'];
        if (lines is List) return lines.whereType<String>().toList();
      }
    } catch (_) {}
    return const [];
  }

  // ---- Bridge v0.9 ----

  @override
  BridgeReply addMaskGeometry(String compId, String layerId, String kind,
      double x, double y, double w, double h) {
    final fn = _addMaskGeometry;
    if (fn == null) return const BridgeReply.err('library lacks add mask geometry');
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final k = kind.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(fn(c.cast(), l.cast(), k.cast(), x, y, w, h)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(k);
    }
  }

  @override
  BridgeReply toggleEffectParamAnimated(String compId, String layerId,
      String effectId, String paramName, int channel, int frame) {
    final fn = _toggleEffectParamAnimated;
    if (fn == null) return const BridgeReply.err('library lacks effect keyframing');
    return _effectKeyOp(fn, compId, layerId, effectId, paramName, channel, frame);
  }

  @override
  BridgeReply removeEffectParamKeyframe(String compId, String layerId,
      String effectId, String paramName, int channel, int frame) {
    final fn = _removeEffectParamKeyframe;
    if (fn == null) return const BridgeReply.err('library lacks effect keyframing');
    return _effectKeyOp(fn, compId, layerId, effectId, paramName, channel, frame);
  }

  BridgeReply _effectKeyOp(_EffectKeyDart fn, String compId, String layerId,
      String effectId, String paramName, int channel, int frame) {
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final e = effectId.toNativeUtf8();
    final p = paramName.toNativeUtf8();
    try {
      return BridgeReply.parse(
          _readReply(fn(c.cast(), l.cast(), e.cast(), p.cast(), channel, frame)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(e);
      malloc.free(p);
    }
  }

  @override
  BridgeReply addEffectParamKeyframe(String compId, String layerId,
      String effectId, String paramName, int channel, int frame, double value) {
    final fn = _addEffectParamKeyframe;
    if (fn == null) return const BridgeReply.err('library lacks effect keyframing');
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final e = effectId.toNativeUtf8();
    final p = paramName.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(
          fn(c.cast(), l.cast(), e.cast(), p.cast(), channel, frame, value)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(e);
      malloc.free(p);
    }
  }

  @override
  BridgeReply shiftEffectParamKeyframes(String compId, String layerId,
      String effectId, String paramName, int channel, String framesJson,
      int delta) {
    final fn = _shiftEffectParamKeyframes;
    if (fn == null) return const BridgeReply.err('library lacks effect keyframing');
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final e = effectId.toNativeUtf8();
    final p = paramName.toNativeUtf8();
    final f = framesJson.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(
          fn(c.cast(), l.cast(), e.cast(), p.cast(), channel, f.cast(), delta)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(e);
      malloc.free(p);
      malloc.free(f);
    }
  }

  @override
  BridgeReply setEffectParamKeyframeInterp(
      String compId,
      String layerId,
      String effectId,
      String paramName,
      int channel,
      int frame,
      String interpIn,
      String interpOut,
      double speedIn,
      double influenceIn,
      double speedOut,
      double influenceOut) {
    final fn = _setEffectParamKeyframeInterp;
    if (fn == null) return const BridgeReply.err('library lacks effect keyframing');
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final e = effectId.toNativeUtf8();
    final p = paramName.toNativeUtf8();
    final ii = interpIn.toNativeUtf8();
    final io = interpOut.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(fn(c.cast(), l.cast(), e.cast(),
          p.cast(), channel, frame, ii.cast(), io.cast(), speedIn, influenceIn,
          speedOut, influenceOut)));
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(e);
      malloc.free(p);
      malloc.free(ii);
      malloc.free(io);
    }
  }

  @override
  BridgeReply saveEffectPreset(String compId, String layerId, String name) {
    final fn = _saveEffectPreset;
    if (fn == null) return const BridgeReply.err('library lacks effect presets');
    // Save returns `{ok, preset}` (not a snapshot); the caller reads `preset`
    // off the raw reply via [saveEffectPresetJson]. Here we surface ok/err.
    return _threeStrOptional(fn, compId, layerId, name);
  }

  /// The `.lumfx` JSON text from a save-preset reply, or null on failure — the
  /// Dart side writes it to a file it picked. Separate from [saveEffectPreset]
  /// because the preset text is not a snapshot.
  @override
  String? saveEffectPresetJson(String compId, String layerId, String name) {
    final fn = _saveEffectPreset;
    if (fn == null) return null;
    final c = compId.toNativeUtf8();
    final l = layerId.toNativeUtf8();
    final n = name.toNativeUtf8();
    try {
      final raw = _readReply(fn(c.cast(), l.cast(), n.cast()));
      final decoded = jsonDecode(raw);
      if (decoded is Map && decoded['ok'] == true && decoded['preset'] is String) {
        return decoded['preset'] as String;
      }
    } catch (_) {
    } finally {
      malloc.free(c);
      malloc.free(l);
      malloc.free(n);
    }
    return null;
  }

  @override
  BridgeReply loadEffectPreset(String compId, String layerId, String text) {
    final fn = _loadEffectPreset;
    if (fn == null) return const BridgeReply.err('library lacks effect presets');
    return _threeStrOptional(fn, compId, layerId, text);
  }

  BridgeReply _threeStrOptional(_Str3Dart fn, String a, String b, String c) {
    final ap = a.toNativeUtf8();
    final bp = b.toNativeUtf8();
    final cp = c.toNativeUtf8();
    try {
      return BridgeReply.parse(_readReply(fn(ap.cast(), bp.cast(), cp.cast())));
    } finally {
      malloc.free(ap);
      malloc.free(bp);
      malloc.free(cp);
    }
  }

  @override
  BridgePlaybackTier playbackTier() {
    final fn = _playbackTier;
    if (fn == null) return BridgePlaybackTier.full;
    return _parsePlaybackTier(_callNoArg(fn));
  }

  @override
  BridgePlaybackTier resetRealtime() {
    final fn = _resetRealtime;
    if (fn == null) return BridgePlaybackTier.full;
    return _parsePlaybackTier(_callNoArg(fn));
  }

  BridgePlaybackTier _parsePlaybackTier(String raw) {
    try {
      final decoded = jsonDecode(raw);
      if (decoded is Map && decoded['ok'] == true) {
        return BridgePlaybackTier.fromJson(decoded.cast<String, dynamic>());
      }
    } catch (_) {}
    return BridgePlaybackTier.full;
  }

  // Copy a reply string out of the engine-owned pointer, then free it back to
  // Rust. The copy must happen before the free, so `toDartString` runs inside
  // the try and the free in the finally.
  String _readReply(Pointer<Char> ptr) {
    if (ptr == nullptr) {
      return '{"ok":false,"error":"bridge returned a null reply"}';
    }
    try {
      return ptr.cast<Utf8>().toDartString();
    } finally {
      _freeString(ptr);
    }
  }

  String _callNoArg(_NoArgDart fn) => _readReply(fn());

  String _callStrArg(_StrArgDart fn, String arg) {
    final argPtr = arg.toNativeUtf8();
    try {
      return _readReply(fn(argPtr.cast<Char>()));
    } finally {
      malloc.free(argPtr);
    }
  }
}
