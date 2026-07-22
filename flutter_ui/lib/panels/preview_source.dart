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
import 'viewer_texture_controller.dart';

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
/// It listens to the app's fine-grained [AppStateStub.playheadFrame] notifier
/// (and the big document notifier): whenever the playhead or the document
/// changes it re-resolves the preview and, if the wanted frame is not already
/// cached, asks the [FrameRenderer] for exactly one frame — the per-tick
/// throttle. The heavy render/decode runs OFF the UI isolate when an
/// [IsolateFrameRenderer] is supplied (the perf pass, K-176 — the UI thread
/// must never render a frame, docs/14 and K-017); a [SynchronousFrameRenderer]
/// keeps the old inline behaviour for tests and the placeholder build. Turning
/// the returned bytes into a `ui.Image` is always async, so a listener is
/// notified when the image lands. Latest-wins: at most one render is in flight,
/// and a newer wanted frame supersedes the queued one — the last real picture
/// stays on screen while a newer frame is in flight (never blank, the K-130
/// scope-hold idea applied to the Viewer).
class PreviewSource extends ChangeNotifier {
  final AppStateStub app;
  final FrameRenderer _renderer;

  /// Small most-recently-used cache of decoded frames (keyed `itemId@frame`).
  static const int _cacheLimit = 8;
  final LinkedHashMap<String, _CacheEntry> _cache = LinkedHashMap();

  PreviewTarget? _target;
  ui.Image? _image;
  DecodedFrame? _displayedFrame;
  int _generation = 0;

  /// The key of the render/decode (or its follow-on image decode) in flight, or
  /// null when nothing is pending. Enforces at-most-one-in-flight (latest-wins).
  String? _pendingKey;

  /// Set when the wanted frame changed while a render was in flight; a re-resolve
  /// runs once that render (and its image decode) completes — the "supersede the
  /// queued request" half of latest-wins.
  bool _wantedDirty = false;

  /// A monotonic request sequence handed to the renderer as its `generation`, so
  /// the worker can drop a superseded request at the isolate boundary too.
  int _seq = 0;
  bool _disposed = false;
  bool _compActive = false;

  /// The document epoch this source last resolved against (mirrors
  /// [AppStateStub.documentEpoch], bumped whenever an edit/undo/redo/open adopts
  /// a fresh snapshot). Every decoded frame in the LRU belongs to one epoch, and
  /// a bump makes them all stale: the engine's own rendered-frame cache
  /// invalidated in the same edit, so a Dart LRU keyed without the epoch would
  /// keep serving a pre-edit picture until it happened to fall out on its own —
  /// exactly the "preview does not live-update on edits" defect. The epoch is
  /// both dropped from the cache on a bump and folded into every cache key (belt
  /// and braces), so a reply banked under the old epoch can never satisfy a
  /// post-edit lookup. Pure playhead motion rides `playheadFrame` and never bumps
  /// the epoch, so scrubbing still hits the cache.
  int _lastEpoch = 0;

  // ---- Zero-copy shared-texture path (K-177) ----

  /// Owns the `lumit/viewer_texture` platform-channel registration. Created only
  /// when the renderer offers the shared path; null otherwise (and on every
  /// non-Windows or old build). A test injects a fake so no real runner is hit.
  late final ViewerTextureController? _textureController;

  /// True when the Viewer is showing the shared GPU texture (a `Texture` widget),
  /// not a read-back `ui.Image`.
  bool _sharedActive = false;
  int? _textureId;
  int? _sharedWidth;
  int? _sharedHeight;

  /// A throttled read-back render runs in parallel with the shared path purely
  /// to feed the Scopes their pixels (the texture path moves no pixels to the
  /// CPU). Capped at ~10 Hz so it never competes with the Viewer's own cadence.
  static const int _scopeThrottleMs = 100;
  int _lastScopeRenderMs = 0;
  bool _scopePending = false;

  PreviewSource(this.app,
      {FrameRenderer? renderer, ViewerTextureController? textureController})
      : _renderer = renderer ?? SynchronousFrameRenderer(app) {
    _textureController = textureController ??
        (_renderer.supportsSharedTexture ? ViewerTextureController() : null);
    _lastEpoch = app.documentEpoch;
    app.addListener(_onAppChanged);
    app.playheadFrame.addListener(_onAppChanged);
    // Let Settings → Clear cache empty this decoded-frame LRU too (the engine
    // cache and this Dart tier are two halves of the same thing, K-176).
    app.previewCacheClearer = clearDecodedCache;
    _resolveAndDecode();
  }

  /// Empty the decoded-frame LRU (the Dart-side tier of the rendered-frame
  /// cache), disposing each held image. The next frame re-decodes. Wired to
  /// Settings → Clear cache via [AppStateStub.previewCacheClearer].
  void clearDecodedCache() {
    for (final entry in _cache.values) {
      entry.image.dispose();
    }
    _cache.clear();
  }

  /// What the Viewer resolved this frame (null = no single-layer footage under
  /// the playhead, OR the composited-comp path is active — see [compActive]).
  PreviewTarget? get target => _target;

  /// True when the shown image is the WHOLE composited comp (the engine's
  /// headless render), not a single decoded footage layer. The Viewer drops the
  /// "single-layer" caveat from its placeholder wording on this path, and treats
  /// the frame as self-contained (any missing layer is slated inside it).
  bool get compActive => _compActive;

  /// Whether the renderer can composite comps at all (the dll carries the
  /// comp-render capability). False means an old library, not a failure.
  bool get compRenderSupported => _renderer.supportsCompRender;

  /// Whether the most recent comp render came back empty (no GPU adapter, or
  /// an engine-side render error) — cleared by the next successful comp
  /// frame. The Viewer's placeholder names this state.
  bool _compRenderFailed = false;
  bool get compRenderFailed => _compRenderFailed;

  /// True when the Viewer should show the shared GPU texture directly (the
  /// zero-copy path, K-177) rather than a read-back image. Set once the shared
  /// render lands and the platform channel has registered the texture.
  bool get sharedActive => _sharedActive;

  /// The external-texture id to show while [sharedActive], or null.
  int? get textureId => _textureId;

  /// The shared frame's aspect ratio (width ÷ height), so the Viewer can fit the
  /// `Texture` to the panel exactly as it fits a read-back image. Null until the
  /// first shared frame lands.
  double? get sharedAspect {
    final w = _sharedWidth;
    final h = _sharedHeight;
    if (w == null || h == null || h <= 0) return null;
    return w / h;
  }

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

  /// Publish [generation] to the engine as the newest wanted primary render, so a
  /// stale render already queued behind the renderer lock is skipped before it
  /// starts (engine-side cancellation, K-176). A fast atomic store on the UI
  /// isolate through the bridge's [RenderCancelBridge] capability; a no-op when
  /// the loaded library is older (the symbol is absent) or there is no bridge.
  /// Only the primary comp/shared render publishes — the throttled scope render
  /// uses a higher generation it does not publish, so it is never mistaken for a
  /// supersession of the picture the Viewer shows.
  void _publishGeneration(int generation) {
    final b = app.bridge;
    if (b is RenderCancelBridge && (b as RenderCancelBridge).supportsRenderCancel) {
      (b as RenderCancelBridge).renderCancelStale(generation);
    }
  }

  void _resolveAndDecode() {
    _syncEpoch();
    // Prefer the zero-copy shared-texture path (K-177); then the read-back
    // composited-comp path; only when both decline do we fall to the
    // single-layer decode. Each returns true when it owns this frame.
    if (_resolveAndDecodeShared()) return;
    if (_resolveAndDecodeComp()) return;
    _resolveAndDecodeSingleLayer();
  }

  /// Reconcile with the app's document epoch before resolving. On a bump (an
  /// edit/undo/redo/open adopted a new snapshot) every decoded frame is stale:
  /// drop the LRU so no pre-edit picture is served, and — when a render is
  /// already in flight — mark the wanted frame dirty so a fresh render is issued
  /// once that reply lands (its stale pixels are dropped, not shown). The image
  /// currently on screen is kept alive so the Viewer holds the last picture while
  /// the fresh render is issued (never blank). A no-op on pure playhead motion,
  /// which never bumps the epoch, so scrubbing still hits the cache.
  void _syncEpoch() {
    final epoch = app.documentEpoch;
    if (epoch == _lastEpoch) return;
    _lastEpoch = epoch;
    for (final entry in _cache.values) {
      // Keep the on-screen image alive (it is held, not blanked); free the rest.
      if (!identical(entry.image, _image)) entry.image.dispose();
    }
    _cache.clear();
    if (_pendingKey != null) _wantedDirty = true;
  }

  /// The zero-copy path: ask the engine to render the whole comp into a shared
  /// GPU texture and show it directly (no pixels cross). Returns true when this
  /// path owns the frame (a render kicked off, or one already in flight); false
  /// when it is not applicable at all (no front comp, no shared support, or the
  /// platform channel is unavailable). A render that comes back null, or a
  /// registration that fails, falls back to the read-back path from inside the
  /// async reply — holding the last picture, never blanking.
  bool _resolveAndDecodeShared() {
    final controller = _textureController;
    final compId = app.frontCompIdResolved;
    if (compId == null ||
        // The Settings kill-switch (round 3): a shared texture that registers
        // but presents nothing shows an empty Viewer with no error to catch,
        // so the read-back path must be reachable without a rebuild.
        !app.useSharedTexture ||
        !_renderer.supportsSharedTexture ||
        controller == null ||
        !controller.available) {
      return false;
    }
    final frame = app.previewFrame;

    // Latest-wins: at most one shared render in flight; a newer wanted frame
    // supersedes the queued one when this reply lands.
    if (_pendingKey != null) {
      _wantedDirty = true;
      return true;
    }
    _pendingKey = 'shared:$compId@$frame#$_lastEpoch';
    var settled = false;
    final gen = ++_seq;
    final epoch = _lastEpoch;
    _publishGeneration(gen);
    _renderer.requestShared(compId, frame, gen, (shared) async {
      settled = true;
      if (_disposed) return;
      _pendingKey = null;
      if (epoch != app.documentEpoch) {
        // An edit adopted a new snapshot while this rendered: the texture is
        // pre-edit. Hold the last picture and re-resolve for the new epoch.
        _drainWanted();
        return;
      }
      if (shared == null) {
        // The engine could not use the shared path this frame (no D3D12 adapter,
        // a transient error): fall back to the read-back path, holding the last
        // picture.
        _leaveShared();
        if (!_resolveAndDecodeComp()) _resolveAndDecodeSingleLayer();
        _drainWanted();
        return;
      }
      // The platform-conditional argument pack: on Linux the shared frame carries
      // the DMA-BUF fd + DRM metadata, on Windows just the handle. The controller
      // sends the right `register` payload; the channel name and lifecycle are the
      // same either way.
      final id = await controller.ensureRegistered(
        shared.handle,
        shared.width,
        shared.height,
        fd: shared.fd,
        stride: shared.stride,
        offset: shared.offset,
        fourcc: shared.fourcc,
        modifier: shared.modifier,
      );
      if (_disposed) return;
      if (id == null) {
        // The runner has no viewer-texture bridge (or registration failed): the
        // controller latches unavailable, so the shared path is skipped for the
        // rest of the session and we fall back now.
        _leaveShared();
        if (!_resolveAndDecodeComp()) _resolveAndDecodeSingleLayer();
        _drainWanted();
        return;
      }
      await controller.frameReady();
      if (_disposed) return;
      if (epoch != app.documentEpoch) {
        // An edit landed during registration/readiness: don't present a pre-edit
        // texture; re-resolve for the new epoch.
        _drainWanted();
        return;
      }
      _enterShared(id, shared.width, shared.height);
      _generation++;
      notifyListeners();
      // Keep the Scopes fed with pixels at a throttled cadence (the texture path
      // itself moves none to the CPU).
      _maybeScopeRender(compId, frame);
      _drainWanted();
    });
    if (!settled) {
      // A genuinely off-thread render is pending: keep the last texture/picture
      // on screen until it lands (never blank).
      notifyListeners();
    }
    return true;
  }

  /// Enter shared-texture mode: the Viewer shows the `Texture` widget, and the
  /// read-back image/target/comp flags are cleared so no stale picture or slate
  /// shows over it.
  void _enterShared(int textureId, int width, int height) {
    _sharedActive = true;
    _compActive = false;
    _target = null;
    _textureId = textureId;
    _sharedWidth = width;
    _sharedHeight = height;
  }

  /// Leave shared-texture mode (a null render or a failed registration), so the
  /// read-back fallback owns the picture again.
  void _leaveShared() {
    _sharedActive = false;
    _textureId = null;
  }

  /// A throttled read-back comp render whose only job is to feed the Scopes their
  /// pixels while the shared texture drives the Viewer. Runs at most every
  /// [_scopeThrottleMs] (~10 Hz), on its own in-flight guard so it never blocks a
  /// shared render, and updates [displayedFrame] without touching the shown
  /// [image] (the picture is the texture, not this).
  void _maybeScopeRender(String compId, int frame) {
    if (_scopePending) return;
    final now = DateTime.now().millisecondsSinceEpoch;
    if (now - _lastScopeRenderMs < _scopeThrottleMs) return;
    _lastScopeRenderMs = now;
    _scopePending = true;
    _renderer.requestComp(compId, frame, 1.0, ++_seq, (decoded) {
      _scopePending = false;
      if (_disposed) return;
      if (decoded == null || decoded.width == 0 || decoded.height == 0) return;
      _displayedFrame = decoded;
      _generation++;
      notifyListeners();
    });
  }

  /// Ask the engine for the whole composited comp frame. Returns true when this
  /// path owns the frame (a cache hit, a render kicked off, or one already in
  /// flight); false only when the comp path is not applicable at all (no front
  /// comp, or a renderer without the comp-render capability). A render that
  /// COMES BACK null (no GPU adapter, a transient failure) falls back to the
  /// single-layer path from inside the async reply, holding the last picture.
  /// When it owns the frame there is no single-layer [target] (the comp frame
  /// carries any missing-layer slate itself) and [compActive] is set.
  bool _resolveAndDecodeComp() {
    final compId = app.frontCompIdResolved;
    if (compId == null || !_renderer.supportsCompRender) return false;
    final frame = app.previewFrame;
    // The resolution picker's downsample factor: the render (and the engine
    // frame cache, which keys on scale) both honour it, so Half/Third/Quarter
    // actually render fewer pixels and warm their own per-scale cache entries.
    // Under Auto it is the realtime controller's live tier scale (K-171).
    final scale = app.effectivePreviewScale;
    final key = 'comp:$compId@$frame@$scale#$_lastEpoch';

    final cached = _cache[key];
    if (cached != null) {
      final wasComp = _compActive;
      _enterComp();
      _touch(key, cached);
      // A frame we can serve from the Dart LRU is warm in the engine cache too
      // (the RAM tier the timeline cache bar draws).
      app.noteFrameWarmed(compId, frame);
      final changed = !identical(_image, cached.image);
      _image = cached.image;
      _displayedFrame = cached.frame;
      if (changed) _generation++;
      // Notify on a new picture, or when we have just switched into comp mode
      // (so the Viewer drops any single-layer slate it was showing).
      if (changed || !wasComp) notifyListeners();
      return true;
    }

    // Something already in flight: remember to re-resolve when it lands, and
    // hold the last picture. The comp path still owns the frame.
    if (_pendingKey != null) {
      _wantedDirty = true;
      return true;
    }

    // Issue the composited-comp render off the UI isolate. Latest-wins: this is
    // the only request in flight until its reply arrives.
    _pendingKey = key;
    var settled = false;
    final gen = ++_seq;
    final epoch = _lastEpoch;
    _publishGeneration(gen);
    _renderer.requestComp(compId, frame, scale, gen, (rendered) {
      settled = true;
      if (_disposed) return;
      _pendingKey = null;
      if (epoch != app.documentEpoch) {
        // A superseding edit adopted a new snapshot while this frame rendered:
        // its pixels are pre-edit and must never be shown, banked, or marked
        // warm. Hold the last picture; the drain re-resolves for the new epoch.
        _drainWanted();
        return;
      }
      if (rendered == null || rendered.width == 0 || rendered.height == 0) {
        // The engine could not composite this frame: fall back to single-layer,
        // holding the last picture. Remember the failure so the Viewer's
        // placeholder can NAME it instead of promising future work (the
        // desk-test round 3 requirement).
        if (!_compRenderFailed) {
          _compRenderFailed = true;
          notifyListeners();
        }
        _resolveAndDecodeSingleLayer();
        _drainWanted();
        return;
      }
      // The engine has rendered and cached this frame (the RAM tier).
      _compRenderFailed = false;
      app.noteFrameWarmed(compId, frame);
      _enterComp();
      _startImageDecode(key, rendered, epoch);
      _drainWanted();
    });
    if (!settled) {
      // An off-thread render is genuinely pending: enter comp mode and keep the
      // last picture on screen until the composited frame lands (never blank).
      _enterComp();
      notifyListeners();
    }
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

    final key = '${target.item.id}@${target.sourceFrame}#$_lastEpoch';
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

    // At most one render/decode is in flight: hold the last picture and
    // re-resolve when the in-flight one completes.
    if (_pendingKey != null) {
      _wantedDirty = true;
      return;
    }

    // Decode the one footage frame off the UI isolate.
    _pendingKey = key;
    final epoch = _lastEpoch;
    _renderer.requestDecode(target.item.id, target.sourceFrame, ++_seq,
        (decoded) {
      if (_disposed) return;
      _pendingKey = null;
      if (epoch != app.documentEpoch) {
        // A relink (or any edit) bumped the epoch while this decoded: the pixels
        // are pre-edit. Hold the last picture; the drain re-resolves fresh.
        _drainWanted();
        return;
      }
      if (decoded == null || decoded.width == 0 || decoded.height == 0) {
        // Decode failed / not ready: hold the last picture/trace rather than
        // blanking (the file is fine — this frame just isn't ready).
        _drainWanted();
        return;
      }
      _startImageDecode(key, decoded, epoch);
      _drainWanted();
    });
  }

  /// Re-resolve when a frame changed while a render was in flight, once nothing
  /// is pending — the "supersede the queued request" half of latest-wins.
  void _drainWanted() {
    if (_wantedDirty && _pendingKey == null && !_disposed) {
      _wantedDirty = false;
      _resolveAndDecode();
    }
  }

  /// Enter composited-comp mode: no single-layer target (the comp frame is
  /// self-contained), and the "single-layer" wording is dropped in the Viewer.
  void _enterComp() {
    _target = null;
    _compActive = true;
  }

  /// Turn a decoded RGBA buffer into a blit-ready image asynchronously, cache it
  /// under [key], and show it when it lands. Shared by both paths, so their
  /// caching, LRU and in-flight guard behave identically. [epoch] is the document
  /// epoch the source frame was rendered under: if the document has moved on by
  /// the time the image lands, the frame is pre-edit and is dropped (never banked
  /// under a stale key, never shown) — the last picture is held while the drain
  /// re-resolves.
  void _startImageDecode(String key, DecodedFrame decoded, int epoch) {
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
        if (epoch != app.documentEpoch) {
          img.dispose();
          _drainWanted();
          return;
        }
        _put(key, _CacheEntry(img, decoded));
        _image = img;
        _displayedFrame = decoded;
        _generation++;
        notifyListeners();
        // A newer frame may have been wanted while the image decoded; pick it up.
        _drainWanted();
      },
    );
  }

  void _touch(String key, _CacheEntry entry) {
    _cache.remove(key);
    _cache[key] = entry;
  }

  // --- Forwarders onto the renderer seam (the perf pass, TF round 5) ------
  //
  // The Scopes panel, the Project-panel thumbnails and the eyedropper reach
  // the OFF-THREAD renderer through these, so their engine calls never block
  // the UI isolate behind the render lock. Each rides its own caller-side
  // guard and its own generation; none of them touches [_pendingKey], so a
  // scope trace or a thumbnail can never delay the Viewer's own picture.

  /// Forward a scope-trace request to the renderer (the K-096 GPU pass, served
  /// off the UI isolate by the worker). The panel holds its own latest-wins
  /// guard; the generation here is only the worker's reply key and is never
  /// published as a wanted primary render (same as the throttled scope render).
  void requestScopeTrace(int kind, String compId, int frame, double scale,
      int bg, int trace, int red, int green, int blue,
      void Function(Uint8List?) onTrace) {
    _renderer.requestScopeTrace(
        kind, compId, frame, scale, bg, trace, red, green, blue, ++_seq,
        onTrace);
  }

  /// Forward a thumbnail decode to the renderer (off the UI isolate on the
  /// worker; the engine caches it, so repeats are cheap). The Project panel's
  /// rows hold their own epoch guard.
  void requestThumbnail(
      String itemId, int maxEdge, void Function(DecodedFrame?) onFrame) {
    _renderer.requestThumbnail(itemId, maxEdge, ++_seq, onFrame);
  }

  /// A one-off full-scale comp readback for the eyedropper, off the UI isolate.
  /// The eyedropper prefers [displayedFrame] (pixels already read back); this is
  /// only the fallback when no CPU frame exists yet (the shared-texture path
  /// before its first throttled readback). Its own request — never
  /// [_pendingKey] — so it cannot delay the Viewer.
  void requestSampleFrame(void Function(DecodedFrame?) onFrame) {
    final compId = app.frontCompIdResolved;
    if (compId == null || !_renderer.supportsCompRender) {
      onFrame(null);
      return;
    }
    _renderer.requestComp(compId, app.previewFrame, 1.0, ++_seq, (frame) {
      if (_disposed) return;
      onFrame(frame);
    });
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
    app.playheadFrame.removeListener(_onAppChanged);
    if (identical(app.previewCacheClearer, clearDecodedCache)) {
      app.previewCacheClearer = null;
    }
    _textureController?.dispose();
    _renderer.dispose();
    for (final entry in _cache.values) {
      entry.image.dispose();
    }
    _cache.clear();
    super.dispose();
  }
}

/// The seam the [PreviewSource] renders through. Implementations perform the
/// heavy engine render/decode; a [SynchronousFrameRenderer] does it inline on
/// the UI isolate (tests, the placeholder build), an [IsolateFrameRenderer]
/// hands it to a long-lived worker isolate so the UI thread never blocks on a
/// full comp render + readback (the perf pass, K-176). Every method is a
/// request→callback so the isolate implementation can honour latest-wins.
abstract class FrameRenderer {
  /// Whether the whole-comp render path is available (mirrors
  /// [CompRenderBridge.supportsCompRender]). Cheap and safe on the UI isolate —
  /// it is only a symbol-presence check, never a render.
  bool get supportsCompRender;

  /// Whether the zero-copy shared-texture path is available (mirrors
  /// [SharedTextureBridge.supportsSharedTexture], K-177). Only a symbol/flag
  /// check, safe on the UI isolate.
  bool get supportsSharedTexture;

  /// Render the whole composited comp [compId] at [frame], scaled by [scale].
  /// [generation] tags the request so a stale reply can be dropped. [onFrame]
  /// receives the decoded RGBA frame, or null when the engine cannot composite
  /// it (no adapter / a transient failure).
  void requestComp(String compId, int frame, double scale, int generation,
      void Function(DecodedFrame?) onFrame);

  /// Render the whole composited comp [compId] at [frame] into a shared GPU
  /// texture (K-177). [onFrame] receives the shared frame's NT handle and
  /// dimensions, or null when the engine cannot use the shared path this frame
  /// (no D3D12 adapter, a transient error) — the Viewer then falls back.
  void requestShared(String compId, int frame, int generation,
      void Function(SharedFrame?) onFrame);

  /// Decode one footage frame [frame] of item [itemId] (the single-layer
  /// fallback). [onFrame] receives the frame, or null on failure.
  void requestDecode(String itemId, int frame, int generation,
      void Function(DecodedFrame?) onFrame);

  /// Compute the engine scope trace for comp [compId] at [frame]/[scale] (the
  /// K-096 GPU pass — [kind] and the packed colours mirror
  /// [ScopeTraceBridge.renderScope]). [onTrace] receives the 256×256 RGBA trace
  /// bytes, or null when the engine declines. Off the UI isolate on the worker
  /// implementation: even a cache-served trace waits on the engine's render
  /// lock, which the worker may hold for a whole uncached comp render — the
  /// "Scopes panel freezes the interface" defect.
  void requestScopeTrace(int kind, String compId, int frame, double scale,
      int bg, int trace, int red, int green, int blue, int generation,
      void Function(Uint8List?) onTrace);

  /// Decode the cached thumbnail of footage [itemId] (longer edge at most
  /// [maxEdge]) — [ThumbnailBridge.thumbnail] through the same seam, so a cold
  /// video thumbnail decode never runs on the UI isolate. [onFrame] receives
  /// the frame, or null without the capability / on failure.
  void requestThumbnail(String itemId, int maxEdge, int generation,
      void Function(DecodedFrame?) onFrame);

  /// Release any worker/isolate the renderer owns.
  void dispose();
}

/// The inline renderer: it calls the bridge on the UI isolate and invokes the
/// callback synchronously, so widget tests stay deterministic (the fake bridge
/// answers within the same call stack, exactly as before the perf pass). Used
/// whenever no [IsolateFrameRenderer] is injected.
class SynchronousFrameRenderer implements FrameRenderer {
  final AppStateStub app;
  SynchronousFrameRenderer(this.app);

  @override
  bool get supportsCompRender {
    final b = app.bridge;
    return b is CompRenderBridge && (b as CompRenderBridge).supportsCompRender;
  }

  @override
  bool get supportsSharedTexture {
    final b = app.bridge;
    return b is SharedTextureBridge &&
        (b as SharedTextureBridge).supportsSharedTexture;
  }

  @override
  void requestComp(String compId, int frame, double scale, int generation,
      void Function(DecodedFrame?) onFrame) {
    final b = app.bridge;
    onFrame(b is CompRenderBridge
        ? (b as CompRenderBridge).renderCompFrame(compId, frame, scale)
        : null);
  }

  @override
  void requestShared(String compId, int frame, int generation,
      void Function(SharedFrame?) onFrame) {
    final b = app.bridge;
    onFrame(b is SharedTextureBridge
        ? (b as SharedTextureBridge).renderToShared(compId, frame)
        : null);
  }

  @override
  void requestDecode(String itemId, int frame, int generation,
      void Function(DecodedFrame?) onFrame) {
    onFrame(app.decodeFrame(itemId, frame));
  }

  @override
  void requestScopeTrace(int kind, String compId, int frame, double scale,
      int bg, int trace, int red, int green, int blue, int generation,
      void Function(Uint8List?) onTrace) {
    final b = app.bridge;
    onTrace(b is ScopeTraceBridge && (b as ScopeTraceBridge).supportsScopeTrace
        ? (b as ScopeTraceBridge)
            .renderScope(kind, compId, frame, scale, bg, trace, red, green, blue)
        : null);
  }

  @override
  void requestThumbnail(String itemId, int maxEdge, int generation,
      void Function(DecodedFrame?) onFrame) {
    onFrame(app.thumbnail(itemId, maxEdge));
  }

  @override
  void dispose() {}
}
