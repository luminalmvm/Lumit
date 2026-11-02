#ifndef RUNNER_BACKDROP_H_
#define RUNNER_BACKDROP_H_

#include <flutter_linux/flutter_linux.h>
#include <gtk/gtk.h>

// A see-through window, when the style asks for one
// (lib/state/window_backdrop.dart). The window is made see-through where
// Flutter leaves it clear, and the compositor is asked to blur what is behind
// it. KDE Plasma can, on Wayland and on X11. Other desktops have no way to be
// asked, and a see-through window that isn't blurred is hard to read, so the
// window stays solid there.

// Gives the window a visual with alpha, if the screen is composited. Before
// the window is realised, since a visual can't change after.
void backdrop_prepare(GtkWindow* window);

// "tint" or "blur" asks for it, and both blur here. Anything else is none.
// [content] is what Flutter draws in, so the blur covers it and not the
// shadow round the window. Answers whether the window is see-through now.
gboolean backdrop_set(GtkWindow* window,
                      GtkWidget* content,
                      FlView* view,
                      const gchar* kind);

#endif  // RUNNER_BACKDROP_H_
