// What page.dart does, for the browser build.

import 'dart:js_interop';
import 'dart:js_interop_unsafe';

/// Lumit has drawn: the page fades its loading card
/// (`lumitReady` in web/flutter_bootstrap.js).
void pageReady() {
  if (globalContext.has('lumitReady')) globalContext.callMethod('lumitReady'.toJS);
}
