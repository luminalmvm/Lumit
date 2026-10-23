# webview_windows

Version 0.4.0 of [webview_windows](https://github.com/jnschulze/flutter-webview-windows)
by Niklas Schulze, under its BSD licence (`LICENSE`). Kept here so Lumit can change it.

Lumit's changes:

- `limitRequests`, which holds a page to its own files and a list of hosts through
  `AddWebResourceRequestedFilterWithRequestSourceKinds`, and serves its files itself
  with a Content-Security-Policy. A virtual host name mapping would serve them first and
  without it. Lumit's extension panel uses it.
- The WebView2 SDK is 1.0.2903.40, for that call, and is unpacked under a folder named
  after its version so a build folder made with the old one picks it up.
