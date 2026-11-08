// What win32.dart answers, for the browser build, which has no `dart:ffi`
// and no Windows to ask.

import 'package:web/web.dart' as web;

String abiName() => 'web_wasm32';

bool altPhysicallyDown() => false;

bool lumitInFront() => true;

bool winWrite(String name, String secret) => false;

String? winRead(String name) => null;

void winDelete(String name) {}

/// Open [url] in a tab of its own, which is how a page follows a link.
bool openInTab(String url) {
  web.window.open(url, '_blank', 'noopener');
  return true;
}
