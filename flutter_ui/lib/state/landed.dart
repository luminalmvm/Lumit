// A still from the engine, which the browser build hands over a moment after
// it is first asked for.

import 'package:flutter/foundation.dart' show kIsWeb;

/// [ask], and in the browser build ask again while it answers null. A picture
/// there comes off the graphics card between turns, so the first ask starts
/// it and a later one collects it. Gives up after two seconds.
Future<T?> landed<T>(Future<T?> Function() ask) async {
  var got = await ask();
  for (var i = 0; kIsWeb && got == null && i < 40; i++) {
    await Future<void>.delayed(const Duration(milliseconds: 50));
    got = await ask();
  }
  return got;
}
