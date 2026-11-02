// Footage in the browser build. A page has no paths, so a file the user
// picks or drops is opened by flutter_ui/web/media.js, which reads it with
// the browser's own decoders and hands the engine what it found under a path
// of its own, `/web/<id>/<name>`. The import then goes on with that path as
// it would with one on a disk.

import 'dart:async';
import 'dart:js_interop';

import 'package:web/web.dart' as web;

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

/// Pick video and sound files and open each one, answering their paths.
// ponytail: a plain file input, until the browser build's own pickers land.
Future<List<String>> pickWebMedia() async {
  final input = web.HTMLInputElement()
    ..type = 'file'
    ..multiple = true
    ..accept = 'video/*,audio/*';
  final picked = Completer<void>();
  void done(web.Event _) {
    if (!picked.isCompleted) picked.complete();
  }

  input.onchange = done.toJS;
  input.oncancel = done.toJS;
  input.click();
  await picked.future;
  final files = input.files;
  final paths = <String>[];
  for (var i = 0; i < (files?.length ?? 0); i++) {
    final file = files!.item(i);
    if (file == null) continue;
    final path = await openWebMedia(file);
    if (path != null) paths.add(path);
  }
  return paths;
}
