// The machine Lumit is running on.
//
// `dart:io`'s own `Platform` everywhere but the browser build, which imports
// platform_web.dart in this file's place: there `dart:io` is only an outline,
// and asking it anything throws.

export 'dart:io' show Platform;
