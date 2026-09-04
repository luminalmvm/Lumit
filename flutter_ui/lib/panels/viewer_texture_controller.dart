// The Viewer's zero-copy texture lifecycle.
//
// In plain terms: the engine can draw the Viewer's picture straight into a piece
// of GPU memory that Flutter shows without any copy. The engine hands us an OS
// "handle" naming that memory — a shared-texture handle on Windows, an
// IOSurface id on macOS, a DMA-BUF descriptor on Linux; this object
// registers it with the platform's runner (over a small platform channel each
// runner implements),
// gets back a `textureId` the `Texture` widget shows, and tells the runner each
// time a new frame has been drawn. It re-registers when the handle or size
// changes (a comp resize). No pixels ever pass through this object.
//
// **There is nothing behind it.** Zero-copy is the Viewer's only transport: a
// texture path that does not reach the screen is a defect to be found and
// fixed, not routed around by a slower copy that vaguely works. So nothing here
// switches to an alternative — what it does instead is refuse to fail
// *silently*, which is what made issue #104 a hunt.
//
// The platform-channel shape (method names, the shared-handle surface type, the
// register/frame-available dance) follows the MIT-licensed `flutter_wgpu_texture`
// package as a reference for the embedder plumbing — we borrow the pattern, not
// the code.

import 'package:flutter/services.dart';

import '../state/faults.dart';

/// Owns the `lumit/viewer_texture` platform-channel registration for one Viewer.
/// A fake [MethodChannel] can be injected so tests drive it without the runner.
class ViewerTextureController {
  /// The channel every runner listens on (see
  /// `windows/runner/viewer_texture_bridge.cpp`,
  /// `linux/runner/viewer_texture_bridge.cc`,
  /// `macos/Runner/ViewerTextureBridge.swift`).
  static const String channelName = 'lumit/viewer_texture';

  final MethodChannel _channel;

  int? _textureId;
  int? _handle;
  int? _width;
  int? _height;
  // The DMA-BUF fd this texture was registered with (Linux), part of the identity
  // for the no-op-on-unchanged check. Null on Windows.
  int? _fd;

  /// False once a channel call reports the runner has no handler (an unwired or
  /// old build). Sticky for the session, because that answer cannot change: a
  /// channel with no handler on the other side will not grow one, and asking it
  /// again every frame buys nothing.
  bool _available = true;

  ViewerTextureController({MethodChannel? channel})
      : _channel = channel ?? const MethodChannel(channelName);

  /// The current external-texture id, or null before the first registration.
  int? get textureId => _textureId;

  /// True until the platform channel is found to be missing.
  bool get available => _available;

  /// Register (or re-register) the shared texture with the given [width]/[height],
  /// returning its `textureId`. The texture is named by [handle] on Windows (the
  /// DXGI shared handle) and macOS (the `IOSurfaceID`), or by the DMA-BUF fields
  /// on Linux — pass [fd] plus
  /// [stride], [offset], [fourcc] and [modifier] to send the DMA-BUF `register`
  /// payload instead of the handle one (the "platform-conditional argument pack";
  /// the channel name and lifecycle are identical). A no-op returning the existing
  /// id when the identity (handle-or-fd + size) is unchanged. Returns null — and
  /// latches [available] to false — when the runner has no handler for the
  /// channel.
  Future<int?> ensureRegistered(
    int handle,
    int width,
    int height, {
    int? fd,
    int? stride,
    int? offset,
    int? fourcc,
    int? modifier,
  }) async {
    if (!_available) return null;
    // Identity is the fd on Linux (DMA-BUF) or the handle on Windows, plus size.
    if (_textureId != null &&
        _handle == handle &&
        _fd == fd &&
        _width == width &&
        _height == height) {
      return _textureId;
    }
    // A registration for this same identity already in flight: wait on it
    // rather than starting a second one. Frames arrive faster than a platform
    // channel round trip, so without this a resize registers the new texture
    // once per frame that arrives while the first call is still out — every one
    // but the last leaked, and the Viewer flickered between them.
    final wanted = (handle, fd, width, height);
    if (_registering != null && _wanted == wanted) return _registering;
    final pending =
        _register(handle, width, height, fd, stride, offset, fourcc, modifier);
    _wanted = wanted;
    _registering = pending;
    try {
      return await pending;
    } finally {
      if (identical(_registering, pending)) {
        _registering = null;
        _wanted = null;
      }
    }
  }

  /// The registration in flight and the identity it is for — see
  /// [ensureRegistered].
  Future<int?>? _registering;
  (int, int?, int, int)? _wanted;

  Future<int?> _register(
    int handle,
    int width,
    int height,
    int? fd,
    int? stride,
    int? offset,
    int? fourcc,
    int? modifier,
  ) async {
    // The texture on screen right now. It stays registered — and so keeps
    // drawing the last good frame — until its replacement is ready: unregister
    // first and the Viewer has nothing to draw for the length of a platform
    // round trip, which is the blank flash a resize or a tier change used to
    // show.
    final previous = _textureId;
    try {
      final args = fd != null
          ? <String, Object?>{
              'fd': fd,
              'width': width,
              'height': height,
              'stride': stride ?? 0,
              'offset': offset ?? 0,
              'fourcc': fourcc ?? 0,
              'modifier': modifier ?? 0,
            }
          : <String, Object?>{
              'handle': handle,
              'width': width,
              'height': height,
            };
      final id = await _channel.invokeMethod<int>('register', args);
      _textureId = id;
      _announced = 0;
      _drawn = 0;
      _handle = handle;
      _fd = fd;
      _width = width;
      _height = height;
      // Now that the replacement is registered, let the old one go (the Linux
      // runner closes its fd here).
      if (previous != null && previous != id) {
        try {
          await _channel
              .invokeMethod<void>('unregister', {'textureId': previous});
        } catch (_) {
          // The old texture is already gone as far as we are concerned.
        }
      }
      return id;
    } on MissingPluginException {
      _available = false;
      return null;
    } catch (err) {
      // The runner has a handler and it refused: a bad descriptor, a driver
      // that will not import it, a size it cannot make. There is nowhere to go
      // and no point announcing frames into a texture that does not
      // exist, so the path latches off — but it says so on the way, because a
      // Viewer that goes blank without a word is what issue #104 cost a week.
      _available = false;
      if (!_reported) {
        _reported = true;
        recordFault('the Viewer texture could not be registered: $err',
            StackTrace.current);
      }
      return null;
    }
  }

  /// Tell the runner a fresh frame has been drawn into the registered texture,
  /// so Flutter re-samples it. A no-op when nothing is registered. A transient
  /// failure is swallowed (one skipped frame), but a missing handler latches
  /// [available] off.
  /// How many frames we have announced since registering, and how many times
  /// Flutter has actually drawn the texture in reply.
  ///
  /// **The failure these exist to catch.** If the embedder cannot open or
  /// composite the shared handle it does not fail — it draws nothing, says
  /// nothing, and the Viewer shows an empty panel for the whole session while
  /// the playhead runs and every other panel updates. Registration succeeding
  /// tells you nothing, because that is exactly what it does when it is about to
  /// silently ignore the texture. That is the shape of issue #104: Flutter 3.47
  /// started Linux on Impeller, Impeller's GLES surface never came back for the
  /// descriptor, and every log line said the frames were rendering.
  ///
  /// So: announce a few frames, then check whether any of them were drawn. If
  /// none were, **write it to the diagnostics file** — the one a bug report is
  /// asked for — and carry on announcing. Carrying on is deliberate: with no
  /// second transport to move to, switching this one off can only
  /// guarantee a dead Viewer where a recovering one was still possible.
  int _announced = 0;
  int _drawn = 0;

  /// Whether this path has already written its record — either the never-drawn
  /// one below or a refused registration above. Once a session: the never-drawn
  /// condition holds on every frame after the twelfth, and a diagnostics file
  /// filled with one line per frame is a file nobody reads.
  bool _reported = false;

  /// Frames to allow before deciding. Enough that a slow first composite or a
  /// window that has not been painted yet is not mistaken for a broken path.
  static const int _graceFrames = 12;

  /// True once the texture path has been seen to fail this way.
  bool get neverDrawn => _announced >= _graceFrames && _drawn == 0;

  /// The raw counters, for the integration test that hunts the silent-failure
  /// case on a real window — they mean nothing to production code.
  int get debugAnnounced => _announced;
  int get debugDrawn => _drawn;

  Future<void> frameReady() async {
    final id = _textureId;
    if (!_available || id == null) return;
    try {
      final drawn =
          await _channel.invokeMethod<int>('frameReady', {'textureId': id});
      _announced++;
      _drawn = drawn ?? _drawn;
      if (neverDrawn && !_reported) {
        _reported = true;
        recordFault(
            'the Viewer texture was registered and announced $_announced times '
            'and drawn none — the runner has it and the embedder is not '
            'compositing it',
            StackTrace.current);
      }
    } on MissingPluginException {
      _available = false;
    } catch (_) {
      // Keep the texture; a failed mark just skips this frame's repaint.
    }
  }

  /// Unregister the texture and forget it. Safe to call more than once.
  Future<void> dispose() async {
    final id = _textureId;
    _textureId = null;
    _handle = null;
    _fd = null;
    _width = null;
    _height = null;
    if (id == null) return;
    try {
      await _channel.invokeMethod<void>('unregister', {'textureId': id});
    } catch (_) {
      // Nothing to do on shutdown.
    }
  }
}
