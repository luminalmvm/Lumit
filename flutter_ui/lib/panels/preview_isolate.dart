// The render isolate (the perf pass, K-176 — the big one).
//
// In plain terms: rendering a whole composited comp and reading its pixels back
// is heavy work. Doing it on the UI isolate freezes the interface every frame
// of a scrub or playback (the "laggy af" report; docs/14 and K-017 say the UI
// thread must never render a frame). This file moves that work onto a long-lived
// background worker isolate.
//
// HOW THE ENGINE STATE STAYS SHARED. The worker opens its OWN
// `DynamicLibrary.open` of the SAME `lumit_bridge.dll` file. A DLL opened twice
// in one process shares one copy of its data, so both handles see the one engine
// state behind the bridge's process-wide `Mutex` (crates/lumit-bridge/src/
// state.rs: `static BRIDGE: OnceLock<Mutex<Bridge>>`, taken only for the
// duration of one call and never across a re-entrant call, so it cannot
// deadlock). That mutex is exactly what makes a render on the worker isolate
// safe while the UI isolate keeps driving document ops — the two serialise
// through the lock rather than racing. Only the read-only calls ride the worker
// — comp renders, footage decodes, scope traces and thumbnails (TF round 5:
// the latter two used to run on the UI isolate and froze the interface waiting
// on the render lock); document mutations stay on the UI isolate's handle.
//
// LATEST-WINS. Requests carry a monotonic `generation`. The worker answers each
// in order; a reply the [PreviewSource] no longer wants is simply dropped there
// (it only ever keeps one request outstanding). If the worker cannot be spawned
// or the library cannot be opened in it, the renderer degrades to the inline
// [SynchronousFrameRenderer] so the Viewer never goes dark — the required
// fallback when isolates are unavailable.

import 'dart:ffi';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import '../bridge/bridge.dart';
import '../state/app_state.dart';
import 'preview_source.dart';

// The two engine symbols the worker needs, plus the buffer free. Mirrors the
// private typedefs in bridge.dart (kept local so the worker is self-contained).
typedef _RenderC = Pointer<Uint8> Function(
    Pointer<Char>, Uint64, Float, Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);
typedef _RenderDart = Pointer<Uint8> Function(
    Pointer<Char>, int, double, Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);
// The ABI-8 generation-aware comp render (K-176): the worker prefers this so a
// superseded render is skipped engine-side before it starts. Same shape as
// `_Render*` with a u64 generation folded in ahead of the out-pointers.
typedef _RenderGenC = Pointer<Uint8> Function(Pointer<Char>, Uint64, Float,
    Uint64, Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);
typedef _RenderGenDart = Pointer<Uint8> Function(Pointer<Char>, int, double, int,
    Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);
typedef _DecodeC = Pointer<Uint8> Function(
    Pointer<Char>, Uint64, Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);
typedef _DecodeDart = Pointer<Uint8> Function(
    Pointer<Char>, int, Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);
typedef _FreeBufferC = Void Function(Pointer<Uint8>, Size);
typedef _FreeBufferDart = void Function(Pointer<Uint8>, int);
// The cached-thumbnail decode (ABI 8): like decode, but a u32 max-edge instead
// of a u64 frame. Served on the worker so a cold video thumbnail never blocks
// the UI isolate (TF round 5).
typedef _ThumbC = Pointer<Uint8> Function(
    Pointer<Char>, Uint32, Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);
typedef _ThumbDart = Pointer<Uint8> Function(
    Pointer<Char>, int, Pointer<Uint32>, Pointer<Uint32>, Pointer<Size>);
// The GPU scope pass (K-096 v1): kind + comp + frame/scale + five packed
// 0x00RRGGBB colours → the fixed 256×256 RGBA trace. Served on the worker for
// the same reason (even a cache-served trace waits on the render lock).
typedef _RenderScopeC = Pointer<Uint8> Function(Uint32, Pointer<Char>, Uint64,
    Float, Uint32, Uint32, Uint32, Uint32, Uint32, Pointer<Size>);
typedef _RenderScopeDart = Pointer<Uint8> Function(int, Pointer<Char>, int,
    double, int, int, int, int, int, Pointer<Size>);

/// The engine's fixed scope-trace side (256×256, matching `scopeGrid`).
const int _scopeSide = 256;
// Zero-copy shared-texture render (K-177): no buffer returned — the frame stays
// on the GPU; the reply carries only the NT handle and dimensions.
typedef _RenderSharedC = Bool Function(
    Pointer<Char>, Uint64, Pointer<Uint64>, Pointer<Uint32>, Pointer<Uint32>);
typedef _RenderSharedDart = bool Function(
    Pointer<Char>, int, Pointer<Uint64>, Pointer<Uint32>, Pointer<Uint32>);
// The Linux DMA-BUF sibling (K-177): the frame stays on the GPU as a DMA-BUF; the
// reply carries the exported fd + DRM metadata (fd, width, height, stride,
// offset, fourcc, modifier) instead of an NT handle.
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

/// What the isolate needs to boot: the port to hand its own receive port back
/// on, and the candidate library paths to open (the UI isolate resolved these).
class _WorkerInit {
  final SendPort mainPort;
  final List<String> libPaths;
  const _WorkerInit(this.mainPort, this.libPaths);
}

/// A [FrameRenderer] that runs the heavy render/decode on a worker isolate, with
/// an inline [SynchronousFrameRenderer] fallback for the spawn window and for a
/// machine where the worker cannot open the library.
class IsolateFrameRenderer implements FrameRenderer {
  final AppStateStub app;

  @override
  final bool supportsCompRender;

  @override
  final bool supportsSharedTexture;

  final SynchronousFrameRenderer _fallback;
  final List<String> _libPaths;

  final ReceivePort _fromWorker = ReceivePort();
  SendPort? _toWorker;
  Isolate? _isolate;
  bool _ready = false;
  bool _failed = false;
  bool _disposed = false;

  /// Callbacks awaiting a worker RGBA reply (comp/decode), keyed by generation.
  final Map<int, void Function(DecodedFrame?)> _awaiting = {};

  /// Callbacks awaiting a worker shared-texture reply, keyed by generation.
  final Map<int, void Function(SharedFrame?)> _awaitingShared = {};

  /// Requests raised before the worker's send port arrived (the spawn window).
  final List<void Function()> _startupQueue = [];

  IsolateFrameRenderer._(this.app, this.supportsCompRender,
      this.supportsSharedTexture, this._libPaths)
      : _fallback = SynchronousFrameRenderer(app) {
    _fromWorker.listen(_onWorkerMessage);
    _spawn();
  }

  /// Build a renderer for [app]'s loaded [LumitBridge], or null when the app has
  /// no real library to open in a worker (then the caller keeps the inline
  /// renderer). [supportsCompRender] is read from the UI-isolate bridge — a
  /// cheap symbol-presence check, safe to do on the UI thread.
  static IsolateFrameRenderer? tryCreate(AppStateStub app) {
    final bridge = app.bridge;
    if (bridge is! LumitBridge) return null;
    final paths = <String>[
      if (bridge.loadedPath != null) bridge.loadedPath!,
      ...LumitBridge.candidateLibraryPaths(),
    ];
    return IsolateFrameRenderer._(app, bridge.supportsCompRender,
        bridge.supportsSharedTexture, paths);
  }

  Future<void> _spawn() async {
    try {
      _isolate = await Isolate.spawn(
        _workerMain,
        _WorkerInit(_fromWorker.sendPort, _libPaths),
        debugName: 'lumit-render',
      );
    } catch (_) {
      // No worker: everything falls back to the inline renderer.
      _failed = true;
      final queued = List<void Function()>.from(_startupQueue);
      _startupQueue.clear();
      for (final run in queued) {
        run();
      }
    }
  }

  void _onWorkerMessage(Object? message) {
    if (message is SendPort) {
      _toWorker = message;
      _ready = true;
      final queued = List<void Function()>.from(_startupQueue);
      _startupQueue.clear();
      for (final run in queued) {
        run();
      }
      return;
    }
    if (message is! List) return;
    // A shared-texture reply is tagged; an RGBA (comp/decode) reply is a bare
    // 4-tuple `[generation, width, height, ttd]`. The shared reply carries both
    // shapes: the Windows handle and the Linux DMA-BUF fields (fd -1 / handle 0
    // for the shape not in use).
    if (message.isNotEmpty && message[0] == 'shared' && message.length == 10) {
      final generation = message[1] as int;
      final handle = message[2] as int;
      final width = message[3] as int;
      final height = message[4] as int;
      final fd = message[5] as int;
      final stride = message[6] as int;
      final offset = message[7] as int;
      final fourcc = message[8] as int;
      final modifier = message[9] as int;
      final onFrame = _awaitingShared.remove(generation);
      if (onFrame == null) return; // superseded/unknown — drop it
      if (width > 0 && height > 0 && (handle != 0 || fd >= 0)) {
        onFrame(SharedFrame(
          handle: handle,
          width: width,
          height: height,
          fd: fd >= 0 ? fd : null,
          stride: fd >= 0 ? stride : null,
          offset: fd >= 0 ? offset : null,
          fourcc: fd >= 0 ? fourcc : null,
          modifier: fd >= 0 ? modifier : null,
        ));
      } else {
        onFrame(null);
      }
      return;
    }
    if (message.length == 4) {
      final generation = message[0] as int;
      final width = message[1] as int;
      final height = message[2] as int;
      final ttd = message[3];
      final onFrame = _awaiting.remove(generation);
      if (onFrame == null) return; // superseded/unknown — drop it
      if (ttd is TransferableTypedData && width > 0 && height > 0) {
        onFrame(DecodedFrame(
            width: width, height: height, rgba: ttd.materialize().asUint8List()));
      } else {
        onFrame(null);
      }
    }
  }

  void _dispatch(int generation, List<Object?> wire,
      void Function(DecodedFrame?) onFrame) {
    if (_disposed) {
      onFrame(null);
      return;
    }
    if (_failed) {
      // Route through the inline fallback (comp vs decode by the leading tag).
      _runFallback(wire, onFrame);
      return;
    }
    if (!_ready) {
      _startupQueue.add(() => _dispatch(generation, wire, onFrame));
      return;
    }
    _awaiting[generation] = onFrame;
    _toWorker!.send(wire);
  }

  void _runFallback(List<Object?> wire, void Function(DecodedFrame?) onFrame) {
    switch (wire[0]) {
      case 'comp':
        _fallback.requestComp(wire[1] as String, wire[2] as int,
            wire[3] as double, wire[4] as int, onFrame);
      case 'thumb':
        _fallback.requestThumbnail(
            wire[1] as String, wire[2] as int, wire[4] as int, onFrame);
      case 'scope':
        // The trace bytes ride the shared DecodedFrame reply shape (the
        // requestScopeTrace adapter unwraps them again).
        _fallback.requestScopeTrace(
            wire[5] as int,
            wire[1] as String,
            wire[2] as int,
            wire[3] as double,
            wire[6] as int,
            wire[7] as int,
            wire[8] as int,
            wire[9] as int,
            wire[10] as int,
            wire[4] as int,
            (bytes) => onFrame(bytes == null
                ? null
                : DecodedFrame(
                    width: _scopeSide, height: _scopeSide, rgba: bytes)));
      default:
        _fallback.requestDecode(
            wire[1] as String, wire[2] as int, wire[4] as int, onFrame);
    }
  }

  /// The shared-texture sibling of [_dispatch] — its reply is a
  /// [SharedFrame], not pixels, so it uses its own awaiting map.
  void _dispatchShared(int generation, List<Object?> wire,
      void Function(SharedFrame?) onFrame) {
    if (_disposed) {
      onFrame(null);
      return;
    }
    if (_failed) {
      _fallback.requestShared(
          wire[1] as String, wire[2] as int, wire[4] as int, onFrame);
      return;
    }
    if (!_ready) {
      _startupQueue.add(() => _dispatchShared(generation, wire, onFrame));
      return;
    }
    _awaitingShared[generation] = onFrame;
    _toWorker!.send(wire);
  }

  @override
  void requestComp(String compId, int frame, double scale, int generation,
      void Function(DecodedFrame?) onFrame) {
    _dispatch(generation, ['comp', compId, frame, scale, generation], onFrame);
  }

  @override
  void requestShared(String compId, int frame, int generation,
      void Function(SharedFrame?) onFrame) {
    _dispatchShared(
        generation, ['shared', compId, frame, 0.0, generation], onFrame);
  }

  @override
  void requestDecode(String itemId, int frame, int generation,
      void Function(DecodedFrame?) onFrame) {
    _dispatch(generation, ['decode', itemId, frame, 1.0, generation], onFrame);
  }

  @override
  void requestScopeTrace(int kind, String compId, int frame, double scale,
      int bg, int trace, int red, int green, int blue, int generation,
      void Function(Uint8List?) onTrace) {
    // The trace rides the shared RGBA reply (generation-keyed), so the plumbing
    // — startup queue, fallback, latest-wins drop — is the comp render's.
    _dispatch(
        generation,
        ['scope', compId, frame, scale, generation, kind, bg, trace, red, green, blue],
        (frame) => onTrace(frame?.rgba));
  }

  @override
  void requestThumbnail(String itemId, int maxEdge, int generation,
      void Function(DecodedFrame?) onFrame) {
    _dispatch(generation, ['thumb', itemId, maxEdge, 1.0, generation], onFrame);
  }

  @override
  void dispose() {
    _disposed = true;
    _awaiting.clear();
    _awaitingShared.clear();
    _startupQueue.clear();
    _fromWorker.close();
    _isolate?.kill(priority: Isolate.immediate);
    _isolate = null;
  }
}

// ---------------------------------------------------------------------------
// The worker isolate.
// ---------------------------------------------------------------------------

/// The worker entrypoint: open the library, then service render/decode requests
/// off the UI isolate, replying with the pixels wrapped in a
/// [TransferableTypedData] (a zero-copy hand-off back to the UI isolate).
void _workerMain(_WorkerInit init) {
  final recv = ReceivePort();
  init.mainPort.send(recv.sendPort);

  DynamicLibrary? lib;
  for (final path in init.libPaths) {
    try {
      lib = DynamicLibrary.open(path);
      break;
    } catch (_) {
      // Try the next candidate.
    }
  }

  if (lib == null) {
    // No library in the worker: answer null to everything so the UI isolate's
    // renderer keeps its last picture rather than hanging on a lost request.
    // Every wire shape (5-tuple, and the 11-entry scope one) carries its
    // generation at index 4.
    recv.listen((message) {
      if (message is! List || message.length < 5) return;
      final generation = message[4] as int;
      if (message[0] == 'shared') {
        init.mainPort.send(_emptySharedReply(generation));
      } else {
        init.mainPort.send([generation, 0, 0, null]);
      }
    });
    return;
  }

  _RenderDart? render;
  _RenderGenDart? renderGen;
  _DecodeDart? decode;
  _RenderSharedDart? renderShared;
  _RenderSharedDmabufDart? renderSharedDmabuf;
  _FreeBufferDart? freeBuffer;
  try {
    render = lib.lookupFunction<_RenderC, _RenderDart>(
        'lumit_bridge_render_comp_frame');
  } catch (_) {
    render = null;
  }
  try {
    renderGen = lib.lookupFunction<_RenderGenC, _RenderGenDart>(
        'lumit_bridge_render_comp_frame_gen');
  } catch (_) {
    renderGen = null;
  }
  try {
    decode =
        lib.lookupFunction<_DecodeC, _DecodeDart>('lumit_bridge_decode_frame');
  } catch (_) {
    decode = null;
  }
  try {
    renderShared = lib.lookupFunction<_RenderSharedC, _RenderSharedDart>(
        'lumit_bridge_render_to_shared');
  } catch (_) {
    renderShared = null;
  }
  try {
    renderSharedDmabuf =
        lib.lookupFunction<_RenderSharedDmabufC, _RenderSharedDmabufDart>(
            'lumit_bridge_render_to_shared_dmabuf');
  } catch (_) {
    renderSharedDmabuf = null;
  }
  try {
    freeBuffer = lib.lookupFunction<_FreeBufferC, _FreeBufferDart>(
        'lumit_bridge_free_buffer');
  } catch (_) {
    freeBuffer = null;
  }
  // The ABI-8 thumbnail and the K-096 scope pass are optional symbols (an older
  // library omits them); a missing one simply answers null, and the UI isolate
  // falls back (glyph / CPU trace).
  _ThumbDart? thumbnail;
  try {
    thumbnail =
        lib.lookupFunction<_ThumbC, _ThumbDart>('lumit_bridge_thumbnail');
  } catch (_) {
    thumbnail = null;
  }
  _RenderScopeDart? renderScope;
  try {
    renderScope = lib.lookupFunction<_RenderScopeC, _RenderScopeDart>(
        'lumit_bridge_render_scope');
  } catch (_) {
    renderScope = null;
  }

  recv.listen((message) {
    if (message is! List || message.length < 5) return;
    final kind = message[0] as String;
    final id = message[1] as String;
    final frame = message[2] as int;
    final scale = message[3] as double;
    final generation = message[4] as int;

    if (kind == 'scope') {
      if (message.length != 11) return;
      final reply = _scopeOne(renderScope, freeBuffer, id, frame, scale,
          message[5] as int, message[6] as int, message[7] as int,
          message[8] as int, message[9] as int, message[10] as int);
      init.mainPort.send([generation, reply.$1, reply.$2, reply.$3]);
      return;
    }
    if (message.length != 5) return;
    if (kind == 'shared') {
      init.mainPort
          .send(_renderShared(renderShared, renderSharedDmabuf, id, frame, generation));
      return;
    }
    final reply = switch (kind) {
      'comp' =>
        _renderOne(render, renderGen, freeBuffer, id, frame, scale, generation),
      // On the thumb wire the `frame` slot carries the max edge.
      'thumb' => _thumbOne(thumbnail, freeBuffer, id, frame),
      _ => _decodeOne(decode, freeBuffer, id, frame),
    };
    init.mainPort.send([generation, reply.$1, reply.$2, reply.$3]);
  });
}

/// The empty shared reply (nothing rendered): handle 0, fd -1. Length matches a
/// real reply so the UI-isolate parser reads a consistent shape.
List<Object?> _emptySharedReply(int generation) =>
    ['shared', generation, 0, 0, 0, -1, 0, 0, 0, 0];

/// Render one comp into the shared GPU texture on the worker, returning the wire
/// reply `['shared', generation, handle, width, height, fd, stride, offset,
/// fourcc, modifier]`. No buffer to free: the frame stays on the GPU (K-177).
/// The Linux DMA-BUF export is preferred when its symbol is present and succeeds
/// (Windows builds answer false for it); otherwise the Windows shared-handle
/// export is tried. On Windows the DMA-BUF fields carry `fd -1`; on Linux the
/// handle carries `0`.
List<Object?> _renderShared(_RenderSharedDart? renderShared,
    _RenderSharedDmabufDart? renderSharedDmabuf, String compId, int frame,
    int generation) {
  // Prefer the DMA-BUF export (Linux). It returns false off Linux / without the
  // feature, so this falls through to the handle export there.
  if (renderSharedDmabuf != null) {
    final id = compId.toNativeUtf8();
    final outFd = malloc<Int32>();
    final outW = malloc<Uint32>();
    final outH = malloc<Uint32>();
    final outStride = malloc<Uint32>();
    final outOffset = malloc<Uint32>();
    final outFourcc = malloc<Uint32>();
    final outModifier = malloc<Uint64>();
    try {
      final ok = renderSharedDmabuf(id.cast(), frame, outFd, outW, outH,
          outStride, outOffset, outFourcc, outModifier);
      if (ok && outFd.value >= 0 && outW.value > 0 && outH.value > 0) {
        return [
          'shared',
          generation,
          0,
          outW.value,
          outH.value,
          outFd.value,
          outStride.value,
          outOffset.value,
          outFourcc.value,
          outModifier.value,
        ];
      }
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

  if (renderShared == null) return _emptySharedReply(generation);
  final id = compId.toNativeUtf8();
  final outHandle = malloc<Uint64>();
  final outW = malloc<Uint32>();
  final outH = malloc<Uint32>();
  try {
    final ok = renderShared(id.cast(), frame, outHandle, outW, outH);
    if (!ok || outHandle.value == 0) return _emptySharedReply(generation);
    return [
      'shared',
      generation,
      outHandle.value,
      outW.value,
      outH.value,
      -1,
      0,
      0,
      0,
      0,
    ];
  } finally {
    malloc.free(id);
    malloc.free(outHandle);
    malloc.free(outW);
    malloc.free(outH);
  }
}

/// Run one comp render on the worker; returns `(width, height, ttd?)`. Prefers
/// the generation-aware entry point (`render_comp_frame_gen`) so a superseded
/// render is skipped engine-side (K-176); falls back to the plain render when an
/// older library lacks the symbol.
(int, int, TransferableTypedData?) _renderOne(
    _RenderDart? render,
    _RenderGenDart? renderGen,
    _FreeBufferDart? freeBuffer,
    String compId,
    int frame,
    double scale,
    int generation) {
  if ((render == null && renderGen == null) || freeBuffer == null) {
    return (0, 0, null);
  }
  final id = compId.toNativeUtf8();
  final outW = malloc<Uint32>();
  final outH = malloc<Uint32>();
  final outLen = malloc<Size>();
  try {
    final ptr = renderGen != null
        ? renderGen(id.cast(), frame, scale, generation, outW, outH, outLen)
        : render!(id.cast(), frame, scale, outW, outH, outLen);
    if (ptr == nullptr) return (0, 0, null);
    final len = outLen.value;
    try {
      final bytes = Uint8List.fromList(ptr.asTypedList(len));
      return (outW.value, outH.value, TransferableTypedData.fromList([bytes]));
    } finally {
      freeBuffer(ptr, len);
    }
  } finally {
    malloc.free(id);
    malloc.free(outW);
    malloc.free(outH);
    malloc.free(outLen);
  }
}

/// Decode one cached thumbnail on the worker; returns `(width, height, ttd?)`.
/// Mirrors [_decodeOne] with the ABI-8 max-edge argument in the frame slot.
(int, int, TransferableTypedData?) _thumbOne(_ThumbDart? thumbnail,
    _FreeBufferDart? freeBuffer, String itemId, int maxEdge) {
  if (thumbnail == null || freeBuffer == null) return (0, 0, null);
  final id = itemId.toNativeUtf8();
  final outW = malloc<Uint32>();
  final outH = malloc<Uint32>();
  final outLen = malloc<Size>();
  try {
    final ptr = thumbnail(id.cast(), maxEdge, outW, outH, outLen);
    if (ptr == nullptr) return (0, 0, null);
    final len = outLen.value;
    try {
      final bytes = Uint8List.fromList(ptr.asTypedList(len));
      return (outW.value, outH.value, TransferableTypedData.fromList([bytes]));
    } finally {
      freeBuffer(ptr, len);
    }
  } finally {
    malloc.free(id);
    malloc.free(outW);
    malloc.free(outH);
    malloc.free(outLen);
  }
}

/// Compute one scope trace on the worker (the K-096 GPU pass); returns
/// `(side, side, ttd?)` — the trace is the engine's fixed 256×256.
(int, int, TransferableTypedData?) _scopeOne(
    _RenderScopeDart? renderScope,
    _FreeBufferDart? freeBuffer,
    String compId,
    int frame,
    double scale,
    int kind,
    int bg,
    int trace,
    int red,
    int green,
    int blue) {
  if (renderScope == null || freeBuffer == null) return (0, 0, null);
  final id = compId.toNativeUtf8();
  final outLen = malloc<Size>();
  try {
    final ptr = renderScope(
        kind, id.cast(), frame, scale, bg, trace, red, green, blue, outLen);
    if (ptr == nullptr) return (0, 0, null);
    final len = outLen.value;
    try {
      final bytes = Uint8List.fromList(ptr.asTypedList(len));
      return (_scopeSide, _scopeSide, TransferableTypedData.fromList([bytes]));
    } finally {
      freeBuffer(ptr, len);
    }
  } finally {
    malloc.free(id);
    malloc.free(outLen);
  }
}

/// Decode one footage frame on the worker; returns `(width, height, ttd?)`.
(int, int, TransferableTypedData?) _decodeOne(_DecodeDart? decode,
    _FreeBufferDart? freeBuffer, String itemId, int frame) {
  if (decode == null || freeBuffer == null) return (0, 0, null);
  final id = itemId.toNativeUtf8();
  final outW = malloc<Uint32>();
  final outH = malloc<Uint32>();
  final outLen = malloc<Size>();
  try {
    final ptr = decode(id.cast(), frame, outW, outH, outLen);
    if (ptr == nullptr) return (0, 0, null);
    final len = outLen.value;
    try {
      final bytes = Uint8List.fromList(ptr.asTypedList(len));
      return (outW.value, outH.value, TransferableTypedData.fromList([bytes]));
    } finally {
      freeBuffer(ptr, len);
    }
  } finally {
    malloc.free(id);
    malloc.free(outW);
    malloc.free(outH);
    malloc.free(outLen);
  }
}
