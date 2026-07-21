// The Viewer's CPU frame source (phase F2, docs/flutter-port/05 §F2).
//
// In plain terms: the Viewer needs actual pictures to show. This object works
// out WHICH footage the playhead is over, asks the engine bridge to decode that
// one frame to raw pixels, turns those pixels into a `ui.Image` Flutter can
// blit, and keeps a small cache so scrubbing back and forth is cheap. The
// Scopes panel reads the very same decoded pixels from here, so the trace always
// matches the picture on screen.
//
// TWO PATHS. When the engine bridge offers composited-comp rendering
// (`CompRenderBridge`, backed by lumit-bridge's headless renderer), this asks
// the engine for the WHOLE composited comp frame — every layer, transform,
// blend and effect, the same pixels the egui Viewer and the exporter produce
// (K-031, K-175). A missing layer inside the comp arrives already slated as
// colour bars within that frame, so the Viewer needs no separate slate on the
// comp path. When the render fails (no GPU adapter, an old library, or a
// transient error) this falls back, per frame, to the single-layer path below.
//
// SINGLE-LAYER FALLBACK. Without comp rendering, this previews only the
// *topmost visible footage layer* whose span covers the playhead, decoded
// straight, with no transform, blending or effects. A footage layer in the
// snapshot is matched to a footage item by name (the snapshot carries no source
// id here), and Retime is not in the snapshot, so the comp-frame → source-frame
// mapping is a straight offset (subtract in_frame). Both are noted where they
// bite; the comp path has neither limitation (the engine resolves everything).

import 'dart:collection';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';

import '../bridge/bridge.dart';
import '../state/app_state.dart';

/// What the Viewer previews this frame: a resolved footage [item] and the
/// [sourceFrame] within it (the comp frame minus the layer's in-point).
@immutable
class PreviewTarget {
  final BridgeItem item;
  final int sourceFrame;
  const PreviewTarget(this.item, this.sourceFrame);

  @override
  bool operator ==(Object other) =>
      other is PreviewTarget &&
      other.item.id == item.id &&
      other.sourceFrame == sourceFrame;

  @override
  int get hashCode => Object.hash(item.id, sourceFrame);
}

/// Find the footage item a footage [layer] references. The snapshot's layer
/// carries no item id (bridge v0.2), so it is matched to a footage item by name
/// — the honest F2 approximation. Searches nested folders. Null when no footage
/// item shares the layer's name.
BridgeItem? footageItemForLayer(BridgeLayer layer, List<BridgeItem> items) {
  BridgeItem? found;
  void walk(List<BridgeItem> xs) {
    for (final it in xs) {
      if (found != null) return;
      if (it.kind == BridgeItemKind.footage && it.name == layer.name) {
        found = it;
        return;
      }
      walk(it.children);
    }
  }

  walk(items);
  return found;
}

/// Resolve what the Viewer previews at [previewFrame]: the topmost VISIBLE
/// footage layer whose span covers the frame, mapped to its source item and
/// source frame. Pure, so the resolution rules are unit-tested without a bridge.
///
/// `layers` is top-first (index 0 = top), so the first covering match wins.
/// The span test mirrors the engine: in_frame ≤ frame < out_frame.
PreviewTarget? resolvePreview(
  BridgeComp? comp,
  int previewFrame,
  List<BridgeItem> items,
) {
  if (comp == null) return null;
  for (final layer in comp.layers) {
    if (layer.kind != BridgeLayerKind.footage) continue;
    if (!layer.switches.visible) continue;
    if (previewFrame < layer.inFrame || previewFrame >= layer.outFrame) continue;
    final item = footageItemForLayer(layer, items);
    if (item == null) continue;
    // Straight offset — Retime is not in the snapshot yet, so a retimed layer
    // previews as if played straight (noted in the checklist).
    return PreviewTarget(item, previewFrame - layer.inFrame);
  }
  return null;
}

/// One cache slot: the blit-ready image and the raw pixels behind it (so the
/// Scopes can read exactly the frame the Viewer is showing, even on a cache
/// hit where no fresh decode happens).
class _CacheEntry {
  final ui.Image image;
  final DecodedFrame frame;
  const _CacheEntry(this.image, this.frame);
}

/// The shared CPU frame source. Lives on [AppStateStub] so the Viewer and the
/// Scopes panel read the same decoded pixels through one notifier.
///
/// It listens to the app: whenever the playhead (or document) changes it
/// re-resolves the preview and, if the wanted frame is not already cached,
/// decodes exactly one frame — the per-tick throttle. Decoding is synchronous
/// (the bridge FFI call), but turning bytes into a `ui.Image` is async, so a
/// listener is notified when the image lands.
class PreviewSource extends ChangeNotifier {
  final AppStateStub app;

  /// Small most-recently-used cache of decoded frames (keyed `itemId@frame`).
  static const int _cacheLimit = 8;
  final LinkedHashMap<String, _CacheEntry> _cache = LinkedHashMap();

  PreviewTarget? _target;
  ui.Image? _image;
  DecodedFrame? _displayedFrame;
  int _generation = 0;
  String? _pendingKey;
  bool _disposed = false;
  bool _compActive = false;

  PreviewSource(this.app) {
    app.addListener(_onAppChanged);
    _resolveAndDecode();
  }

  /// What the Viewer resolved this frame (null = no single-layer footage under
  /// the playhead, OR the composited-comp path is active — see [compActive]).
  PreviewTarget? get target => _target;

  /// True when the shown image is the WHOLE composited comp (the engine's
  /// headless render), not a single decoded footage layer. The Viewer drops the
  /// "single-layer" caveat from its placeholder wording on this path, and treats
  /// the frame as self-contained (any missing layer is slated inside it).
  bool get compActive => _compActive;

  /// The blit-ready image for the current frame, or null when there is nothing
  /// decoded to show (slate, placeholder, or an in-flight decode).
  ui.Image? get image => _image;

  /// The raw pixels the Viewer is currently showing — the Scopes panel reads
  /// this. Held across a momentarily-unavailable frame (K-130) rather than
  /// nulled, so a scrub or a not-yet-decoded frame keeps the last real trace.
  DecodedFrame? get displayedFrame => _displayedFrame;

  /// Bumped each time a new frame is shown, so the Scopes panel knows when to
  /// rebuild its trace without diffing pixel buffers.
  int get generation => _generation;

  void _onAppChanged() {
    if (_disposed) return;
    _resolveAndDecode();
  }

  void _resolveAndDecode() {
    // Prefer the composited-comp path; only when it declines this frame (engine
    // can't render it) do we fall back to the single-layer decode below.
    if (_resolveAndDecodeComp()) return;
    _resolveAndDecodeSingleLayer();
  }

  /// Ask the engine for the whole composited comp frame. Returns true when this
  /// path owns the frame (a cache hit, a decode kicked off, or a decode already
  /// in flight); false when the engine cannot render it, so the caller falls
  /// back to the single-layer path. When it owns the frame there is no
  /// single-layer [target] (the comp frame carries any missing-layer slate
  /// itself) and [compActive] is set.
  bool _resolveAndDecodeComp() {
    final bridge = app.bridge;
    final compId = app.frontCompIdResolved;
    // A bridge without the comp-render capability, or no front comp: decline.
    // (CompRenderBridge is a separate interface, not a subtype of DocumentBridge,
    // so bind it through an explicit cast the `is!` guard has already made safe.)
    if (compId == null || bridge is! CompRenderBridge) return false;
    final render = bridge as CompRenderBridge;
    if (!render.supportsCompRender) return false;
    final frame = app.previewFrame;
    final key = 'comp:$compId@$frame';

    final cached = _cache[key];
    if (cached != null) {
      final wasComp = _compActive;
      _enterComp();
      _touch(key, cached);
      final changed = !identical(_image, cached.image);
      _image = cached.image;
      _displayedFrame = cached.frame;
      if (changed) _generation++;
      // Notify on a new picture, or when we have just switched into comp mode
      // (so the Viewer drops any single-layer slate it was showing).
      if (changed || !wasComp) notifyListeners();
      return true;
    }

    // A decode already in flight for this comp frame: this path still owns it.
    if (_pendingKey == key) return true;

    final rendered = render.renderCompFrame(compId, frame, 1.0);
    if (rendered == null || rendered.width == 0 || rendered.height == 0) {
      // The engine could not composite this frame (no adapter, or a transient
      // failure): let the single-layer path try, holding the last picture.
      return false;
    }
    _enterComp();
    _startImageDecode(key, rendered);
    // The last picture stays on screen until the comp image lands; notify so the
    // Viewer reflects the mode change (target cleared) immediately.
    notifyListeners();
    return true;
  }

  /// The single-layer fallback: the topmost visible footage layer under the
  /// playhead, decoded straight. Unchanged behaviour from before the comp path.
  void _resolveAndDecodeSingleLayer() {
    _compActive = false;
    final comp = app.frontComp;
    final target =
        resolvePreview(comp, app.previewFrame, app.snapshot?.items ?? const []);
    _target = target;

    // No footage, or footage that shows a slate rather than a picture: leave the
    // last decoded frame in place for the Scopes to hold, and let the Viewer
    // draw the slate/placeholder. Clear the live image so the Viewer stops
    // blitting a stale picture over a slate.
    if (target == null ||
        target.item.status == BridgeMediaStatus.missing ||
        target.item.status == BridgeMediaStatus.failed) {
      if (_image != null) {
        _image = null;
        notifyListeners();
      }
      return;
    }

    final key = '${target.item.id}@${target.sourceFrame}';
    final cached = _cache[key];
    if (cached != null) {
      _touch(key, cached);
      final changed = !identical(_image, cached.image);
      _image = cached.image;
      _displayedFrame = cached.frame;
      if (changed) {
        _generation++;
        notifyListeners();
      }
      return;
    }

    // At most one decode is in flight for a given key.
    if (_pendingKey == key) return;

    final decoded = app.decodeFrame(target.item.id, target.sourceFrame);
    if (decoded == null) {
      // Decode failed for this frame: hold the last picture/trace rather than
      // blanking (the file is fine — this frame just isn't ready).
      return;
    }
    if (decoded.width == 0 || decoded.height == 0) return;

    _startImageDecode(key, decoded);
  }

  /// Enter composited-comp mode: no single-layer target (the comp frame is
  /// self-contained), and the "single-layer" wording is dropped in the Viewer.
  void _enterComp() {
    _target = null;
    _compActive = true;
  }

  /// Turn a decoded RGBA buffer into a blit-ready image asynchronously, cache it
  /// under [key], and show it when it lands. Shared by both paths, so their
  /// caching, LRU and in-flight guard behave identically.
  void _startImageDecode(String key, DecodedFrame decoded) {
    _pendingKey = key;
    ui.decodeImageFromPixels(
      decoded.rgba,
      decoded.width,
      decoded.height,
      ui.PixelFormat.rgba8888,
      (img) {
        if (_disposed) {
          img.dispose();
          return;
        }
        _pendingKey = null;
        _put(key, _CacheEntry(img, decoded));
        _image = img;
        _displayedFrame = decoded;
        _generation++;
        notifyListeners();
      },
    );
  }

  void _touch(String key, _CacheEntry entry) {
    _cache.remove(key);
    _cache[key] = entry;
  }

  void _put(String key, _CacheEntry entry) {
    _cache.remove(key);
    _cache[key] = entry;
    while (_cache.length > _cacheLimit) {
      final oldest = _cache.keys.first;
      final evicted = _cache.remove(oldest);
      // Free the GPU/CPU image behind the evicted entry; its bytes go with it.
      evicted?.image.dispose();
    }
  }

  @override
  void dispose() {
    _disposed = true;
    app.removeListener(_onAppChanged);
    for (final entry in _cache.values) {
      entry.image.dispose();
    }
    _cache.clear();
    super.dispose();
  }
}
