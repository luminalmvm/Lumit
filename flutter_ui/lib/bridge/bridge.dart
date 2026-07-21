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

/// One keyframe of a transform property (snapshot v3). `frame` is the comp frame
/// it lands on; `interpIn`/`interpOut` are the engine's `SideInterp` variant
/// names (`Hold`, `Linear`, `Bezier`).
class BridgeKeyframe {
  final int frame;
  final double value;
  final String interpIn;
  final String interpOut;

  const BridgeKeyframe({
    required this.frame,
    required this.value,
    required this.interpIn,
    required this.interpOut,
  });

  factory BridgeKeyframe.fromJson(Map<String, dynamic> m) => BridgeKeyframe(
        frame: _asInt(m['frame']),
        value: _asDouble(m['value']),
        interpIn: m['interp_in'] is String ? m['interp_in'] as String : 'Linear',
        interpOut:
            m['interp_out'] is String ? m['interp_out'] as String : 'Linear',
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
class BridgeEffectParam {
  final String name;
  final String kind;
  final Object? value;

  const BridgeEffectParam({
    required this.name,
    required this.kind,
    required this.value,
  });

  factory BridgeEffectParam.fromJson(Map<String, dynamic> m) =>
      BridgeEffectParam(
        name: m['name'] is String ? m['name'] as String : '',
        kind: m['kind'] is String ? m['kind'] as String : 'unknown',
        value: m['value'],
      );
}

/// One effect instance in a layer's stack (snapshot v3).
class BridgeEffect {
  final String id;
  final String name;
  final bool enabled;
  final List<BridgeEffectParam> params;

  const BridgeEffect({
    required this.id,
    required this.name,
    required this.enabled,
    required this.params,
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
    );
  }
}

/// One entry in the effect registry (`listEffects`): a stable [name] (the match
/// name an op takes) and its sentence-case [label].
class BridgeEffectInfo {
  final String name;
  final String label;

  const BridgeEffectInfo({required this.name, required this.label});

  factory BridgeEffectInfo.fromJson(Map<String, dynamic> m) => BridgeEffectInfo(
        name: m['name'] is String ? m['name'] as String : '',
        label: m['label'] is String ? m['label'] as String : '',
      );
}

/// One composition layer as the Timeline reads it. `inFrame`/`outFrame` are comp
/// frames derived from the comp's own rate; `index` is the stack position
/// (0 = top). Snapshot v3 adds the [transform] read-back, the [effects] stack,
/// and the identity links ([sourceItemId], [sourceCompId], [colour]).
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

  /// The work area as `[inFrame, outFrame]` (snapshot v3), or null for the full
  /// comp — the preview/export span the B/N keys set.
  final List<int>? workArea;

  const BridgeComp({
    required this.width,
    required this.height,
    required this.fps,
    required this.frameCount,
    required this.layers,
    required this.markers,
    this.workArea,
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
      workArea: workArea,
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

  const BridgeReply.ok(this.snapshot) : error = null;
  const BridgeReply.err(this.error) : snapshot = null;

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
      return BridgeReply.ok(BridgeSnapshot.fromJson(map));
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

/// The loaded `lumit_bridge` library, bound to typed calls. Construct with
/// [tryLoad]; a null result means the app runs on its placeholders.
class LumitBridge implements DocumentBridge, CompRenderBridge {
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
  final _DecodeDart _decodeFrame;

  /// Bound only when the loaded library exports it. An older `.dll` (predating
  /// the composited-comp path) lacks the symbol; rather than failing the whole
  /// load, this stays null and [renderCompFrame] returns null, so the Viewer
  /// keeps its single-layer path. Non-final because it is looked up defensively
  /// in the constructor body, not the initializer list.
  _RenderDart? _renderCompFrame;
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
        _decodeFrame = lib.lookupFunction<_DecodeC, _DecodeDart>(
          'lumit_bridge_decode_frame',
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
  }

  /// Load the library and bind it, or return null if it cannot be found or a
  /// symbol is missing. Never throws — a failure is just "run on placeholders".
  static LumitBridge? tryLoad() {
    for (final candidate in _candidatePaths()) {
      try {
        final lib = DynamicLibrary.open(candidate);
        return LumitBridge._(lib);
      } catch (_) {
        // Try the next candidate.
      }
    }
    return null;
  }

  /// Where the library might live, in the order the runner should try:
  /// beside the executable first (the shipped layout), then the Cargo debug
  /// output relative to the working directory (the developer layout), then the
  /// bare name so the OS loader's own search path gets a turn.
  static List<String> _candidatePaths() {
    const name = 'lumit_bridge.dll';
    final paths = <String>[];
    try {
      final exeDir = File(Platform.resolvedExecutable).parent.path;
      paths.add('$exeDir${Platform.pathSeparator}$name');
    } catch (_) {
      // resolvedExecutable can be unavailable in some hosts; skip it.
    }
    final cwd = Directory.current.path;
    final sep = Platform.pathSeparator;
    paths.add('$cwd$sep..$sep..$sep..${sep}target${sep}debug$sep$name');
    paths.add('$cwd$sep..${sep}target${sep}debug$sep$name');
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
