// Footage in the browser build. A page has no paths, so a file the user
// picks or drops is opened by flutter_ui/web/media.js, which reads it with
// the browser's own decoders and hands the engine what it found under a path
// of its own, `/web/<id>/<name>`. The import then goes on with that path as
// it would with one on a disk.

import 'dart:async';
import 'dart:js_interop';


@JS('lumitMedia.open')
external JSPromise<JSString?> _open(JSAny file);

/// Open [file], a browser `File`, and answer the path the engine knows it by,
/// or null when the browser can decode none of it.
Future<String?> openWebMedia(Object file) async {
  try {
    return (await _open(file as JSAny).toDart)?.toDart;
  } catch (_) {
    return null;
  }
}
