#include "backdrop.h"

#include <cstring>

#ifdef GDK_WINDOWING_WAYLAND
#include <gdk/gdkwayland.h>
#include <wayland-client.h>
#endif
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif

namespace {

// The window's CSS class while it is see-through, which clears GTK's own
// background under Flutter.
constexpr const char* kSeeThrough = "lumit-see-through";

// What was last asked for, for the blur to follow the window's size.
gboolean g_on = FALSE;

#ifdef GDK_WINDOWING_WAYLAND
// KWin's blur (org_kde_kwin_blur, from plasma-wayland-protocols), written out
// as wayland-scanner would. It is three requests, too few to generate.
extern const wl_interface kBlur;
const wl_interface* kBlurTypes[] = {&kBlur, &wl_surface_interface,
                                    &wl_surface_interface,
                                    &wl_region_interface};
const wl_message kManagerRequests[] = {
    {"create", "no", kBlurTypes + 0},
    {"unset", "o", kBlurTypes + 2},
};
const wl_interface kManager = {"org_kde_kwin_blur_manager", 1, 2,
                               kManagerRequests, 0, nullptr};
const wl_message kBlurRequests[] = {
    {"commit", "", kBlurTypes},
    {"set_region", "?o", kBlurTypes + 3},
    {"release", "", kBlurTypes},
};
const wl_interface kBlur = {"org_kde_kwin_blur", 1, 3, kBlurRequests, 0,
                            nullptr};

struct Wayland {
  wl_compositor* compositor = nullptr;
  wl_proxy* manager = nullptr;
  // On the window's surface while it is blurred, and that surface. GTK
  // makes a new one when a hidden window is shown again.
  wl_proxy* blur = nullptr;
  wl_surface* blurred = nullptr;
};

void on_global(void* data,
               wl_registry* registry,
               uint32_t name,
               const char* interface,
               uint32_t version) {
  auto* found = static_cast<Wayland*>(data);
  if (strcmp(interface, wl_compositor_interface.name) == 0) {
    found->compositor = static_cast<wl_compositor*>(
        wl_registry_bind(registry, name, &wl_compositor_interface, 1));
  } else if (strcmp(interface, kManager.name) == 0) {
    found->manager =
        static_cast<wl_proxy*>(wl_registry_bind(registry, name, &kManager, 1));
  }
}

void on_global_remove(void*, wl_registry*, uint32_t) {}

const wl_registry_listener kRegistry = {on_global, on_global_remove};

// The compositor's globals, read once on a queue of our own so GTK's events
// are left alone.
Wayland* wayland(GdkDisplay* display) {
  static Wayland* found = nullptr;
  if (found != nullptr) {
    return found;
  }
  found = new Wayland();
  wl_display* wl = gdk_wayland_display_get_wl_display(display);
  wl_event_queue* queue = wl_display_create_queue(wl);
  auto* wrapped = static_cast<wl_display*>(wl_proxy_create_wrapper(wl));
  wl_proxy_set_queue(reinterpret_cast<wl_proxy*>(wrapped), queue);
  wl_registry* registry = wl_display_get_registry(wrapped);
  wl_proxy_wrapper_destroy(wrapped);
  wl_registry_add_listener(registry, &kRegistry, found);
  wl_display_roundtrip_queue(wl, queue);
  return found;
}

gboolean wayland_blur(GdkWindow* window, const GtkAllocation& box) {
  Wayland* w = wayland(gdk_window_get_display(window));
  wl_surface* surface = gdk_wayland_window_get_wl_surface(window);
  if (w->manager == nullptr || w->compositor == nullptr ||
      surface == nullptr) {
    return FALSE;
  }
  auto version = [](wl_proxy* proxy) { return wl_proxy_get_version(proxy); };
  const bool stale = w->blur != nullptr && w->blurred != surface;
  if (!g_on || stale) {
    if (w->blur != nullptr) {
      if (!stale) {
        wl_proxy_marshal_flags(w->manager, 1, nullptr, version(w->manager), 0,
                               surface);
      }
      wl_proxy_marshal_flags(w->blur, 2, nullptr, version(w->blur),
                             WL_MARSHAL_FLAG_DESTROY);
      w->blur = nullptr;
    }
    if (!g_on) {
      return FALSE;
    }
  }
  if (w->blur == nullptr) {
    w->blurred = surface;
    w->blur = wl_proxy_marshal_flags(w->manager, 0, &kBlur,
                                     version(w->manager), 0, nullptr, surface);
  }
  wl_region* region = wl_compositor_create_region(w->compositor);
  wl_region_add(region, box.x, box.y, box.width, box.height);
  wl_proxy_marshal_flags(w->blur, 1, nullptr, version(w->blur), 0, region);
  wl_proxy_marshal_flags(w->blur, 0, nullptr, version(w->blur), 0);
  wl_region_destroy(region);
  return TRUE;
}
#endif

#ifdef GDK_WINDOWING_X11
gboolean x11_blur(GdkWindow* window, const GtkAllocation& box) {
  GdkAtom atom = gdk_atom_intern_static_string("_KDE_NET_WM_BLUR_BEHIND_REGION");
  // KWin's blur says it is there with a property of this name on the root
  // window, not in _NET_SUPPORTED.
  GdkAtom type;
  gint format;
  gint length;
  guchar* data = nullptr;
  const gboolean there =
      gdk_property_get(gdk_screen_get_root_window(gdk_window_get_screen(window)),
                       atom, GDK_NONE, 0, 1, FALSE, &type, &format, &length,
                       &data);
  g_free(data);
  if (!there) {
    return FALSE;
  }
  if (!g_on) {
    gdk_property_delete(window, atom);
    return FALSE;
  }
  // Format 32 is read as longs.
  const long region[] = {box.x, box.y, box.width, box.height};
  gdk_property_change(window, atom, gdk_atom_intern_static_string("CARDINAL"),
                      32, GDK_PROP_MODE_REPLACE,
                      reinterpret_cast<const guchar*>(region), 4);
  return TRUE;
}
#endif

// Ask the compositor to blur behind [content], or to stop. Whether it does.
gboolean blur(GtkWidget* content) {
  GdkWindow* window = gtk_widget_get_window(gtk_widget_get_toplevel(content));
  if (window == nullptr) {
    return FALSE;
  }
  GtkAllocation box;
  gtk_widget_get_allocation(content, &box);
  GdkDisplay* display = gdk_window_get_display(window);
#ifdef GDK_WINDOWING_WAYLAND
  if (GDK_IS_WAYLAND_DISPLAY(display)) {
    return wayland_blur(window, box);
  }
#endif
#ifdef GDK_WINDOWING_X11
  if (GDK_IS_X11_DISPLAY(display)) {
    return x11_blur(window, box);
  }
#endif
  return FALSE;
}

void size_cb(GtkWidget* content, GdkRectangle*, gpointer) {
  if (g_on) {
    blur(content);
  }
}

}  // namespace

void backdrop_prepare(GtkWindow* window) {
  GdkScreen* screen = gtk_widget_get_screen(GTK_WIDGET(window));
  GdkVisual* visual = gdk_screen_get_rgba_visual(screen);
  if (visual != nullptr && gdk_screen_is_composited(screen)) {
    gtk_widget_set_visual(GTK_WIDGET(window), visual);
  }
  static gboolean styled = FALSE;
  if (!styled) {
    styled = TRUE;
    g_autoptr(GtkCssProvider) css = gtk_css_provider_new();
    gtk_css_provider_load_from_data(
        css, "window.lumit-see-through { background-color: transparent; }",
        -1, nullptr);
    gtk_style_context_add_provider_for_screen(
        screen, GTK_STYLE_PROVIDER(css),
        GTK_STYLE_PROVIDER_PRIORITY_APPLICATION);
  }
}

gboolean backdrop_set(GtkWindow* window,
                      GtkWidget* content,
                      FlView* view,
                      const gchar* kind) {
  GdkScreen* screen = gtk_widget_get_screen(GTK_WIDGET(window));
  const gboolean asked =
      g_strcmp0(kind, "tint") == 0 || g_strcmp0(kind, "blur") == 0;
  g_on = asked && gdk_screen_is_composited(screen) &&
         gtk_widget_get_visual(GTK_WIDGET(window)) ==
             gdk_screen_get_rgba_visual(screen);
  if (g_on && !blur(content)) {
    g_on = FALSE;
  }
  if (!g_on) {
    blur(content);
  }
  static gboolean following = FALSE;
  if (g_on && !following) {
    following = TRUE;
    g_signal_connect(content, "size-allocate", G_CALLBACK(size_cb), nullptr);
  }
  GtkStyleContext* style = gtk_widget_get_style_context(GTK_WIDGET(window));
  GdkRGBA clear = {0, 0, 0, g_on ? 0.0 : 1.0};
  fl_view_set_background_color(view, &clear);
  if (g_on) {
    gtk_style_context_add_class(style, kSeeThrough);
  } else {
    gtk_style_context_remove_class(style, kSeeThrough);
  }
  gtk_widget_queue_draw(GTK_WIDGET(window));
  return g_on;
}
