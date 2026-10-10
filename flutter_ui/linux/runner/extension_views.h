// The Linux half of extension panels: the page of an extension, shown in a
// WebKitGTK view.
//
// In plain terms: an extension's panel is a web page, and Flutter on Linux
// cannot hold another toolkit's widget among its own. So the page's view is
// laid over the window, in a GtkOverlay that has Flutter's view as its base,
// and the Dart side (lib/panels/extension_page.dart) says where the panel is
// after every frame that moves it. While something Flutter draws is over the
// panel the Dart side asks for a picture of the page, shows that, and has the
// view hidden, since a view on top would cover a menu or a dialogue.
//
// One channel, 'lumit/extension_views', for every page, each named by a
// number the Dart side picks:
//
//   create   {view, id, folder, data, bootstrap, inspect}
//   place    {view, x, y, width, height, scale, shown}
//   load     {view, url}
//   run      {view, script}
//   picture  {view}  ->  {width, height, pixels}, RGBA
//   dispose  {view}
//   unfocus          give the keyboard back to Flutter's view
//
// and two it sends: message {view, text} and url {view, url}.
//
// The page's files are served from the extension's folder under
// lumit-extension://<id>/, and only to a page that is itself from there.
// The siblings are macos/Runner/ExtensionViews.swift and, on Windows, the
// webview_windows plugin.

#ifndef RUNNER_EXTENSION_VIEWS_H_
#define RUNNER_EXTENSION_VIEWS_H_

#include <flutter_linux/flutter_linux.h>
#include <gtk/gtk.h>

G_BEGIN_DECLS

// Register the 'lumit/extension_views' channel on `messenger`. Pages are laid
// over `overlay`, whose base is `view`. All three live as long as the window.
void extension_views_register(FlBinaryMessenger* messenger,
                              GtkOverlay* overlay,
                              FlView* view);

G_END_DECLS

#endif  // RUNNER_EXTENSION_VIEWS_H_
