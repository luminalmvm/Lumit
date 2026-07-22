// Beat detection off the UI isolate (TF round 5).
//
// In plain terms: detecting beats means the engine listens to the whole
// composition's audio and analyses it — seconds of solid work on a long piece,
// and it queues behind the same engine lock the Viewer's renders take. Calling
// it on the interface thread froze the window for the whole analysis. So the
// call runs here instead, in a short-lived worker `Isolate.run` spins up: the
// worker opens its OWN handle to the same `lumit_bridge` library (a library
// opened twice in one process shares one copy of its data, exactly as the
// render worker does — see preview_isolate.dart), makes the one blocking call,
// and hands the reply JSON string back. The detected markers are already
// committed into the shared engine document by then; the UI isolate just
// parses the reply and adopts the fresh snapshot as any edit op would.

import 'dart:ffi';

import 'package:ffi/ffi.dart';

// The two symbols the worker needs, mirroring bridge.dart's private typedefs
// (kept local so the worker is self-contained, like the render worker's).
typedef _DetectC = Pointer<Char> Function(Pointer<Char>, Int64);
typedef _DetectDart = Pointer<Char> Function(Pointer<Char>, int);
typedef _FreeC = Void Function(Pointer<Char>);
typedef _FreeDart = void Function(Pointer<Char>);

/// Run `lumit_bridge_detect_beats` for [compId] at [sensitivity] against the
/// first of [libPaths] that opens, returning the raw reply JSON (or a bridge-
/// shaped error reply when the library or its symbols are unavailable). Blocks
/// for the whole mixdown + analysis — call it inside `Isolate.run`, never on
/// the UI isolate.
String detectBeatsWithLibrary(
    List<String> libPaths, String compId, int sensitivity) {
  DynamicLibrary? lib;
  for (final path in libPaths) {
    try {
      lib = DynamicLibrary.open(path);
      break;
    } catch (_) {
      // Try the next candidate.
    }
  }
  if (lib == null) {
    return '{"ok":false,"error":"the engine library could not be opened for '
        'beat detection"}';
  }
  final _DetectDart detect;
  final _FreeDart freeString;
  try {
    detect =
        lib.lookupFunction<_DetectC, _DetectDart>('lumit_bridge_detect_beats');
    freeString =
        lib.lookupFunction<_FreeC, _FreeDart>('lumit_bridge_free_string');
  } catch (_) {
    return '{"ok":false,"error":"this engine build is missing beat '
        'detection"}';
  }
  final id = compId.toNativeUtf8();
  try {
    final ptr = detect(id.cast(), sensitivity);
    if (ptr == nullptr) {
      return '{"ok":false,"error":"bridge returned a null reply"}';
    }
    try {
      // Copy the reply out before freeing it back to Rust — the same contract
      // as the bridge's `_readReply`.
      return ptr.cast<Utf8>().toDartString();
    } finally {
      freeString(ptr);
    }
  } finally {
    malloc.free(id);
  }
}
