#include "my_application.h"

#include <flutter_linux/flutter_linux.h>
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif

#include "flutter/generated_plugin_registrant.h"
#include "viewer_texture_bridge.h"

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

// Pin the renderer to Skia, as the Windows runner does with its typed
// `set_impeller_switch` (K-748, extending K-732 to the other two platforms).
//
// In plain terms: Flutter has two renderers, and the newer one — Impeller — is
// measurably slower for the way Lumit draws (docs/impl/ui-performance.md
// 2.4/4.1/7.2). Windows has a C++ API for choosing; the GTK embedder has none,
// so the choice is made the way `flutter run --no-enable-impeller` makes it on
// every desktop platform: an engine switch read out of the environment before
// the engine starts.
//
// **Appended, never overwritten.** `flutter run` fills these same variables
// with the switches a debug session needs — the VM service port among them —
// so replacing the count would leave the tool unable to attach. This reads what
// is already there and adds one more.
static void pin_skia(void) {
  const gchar* set = g_getenv("FLUTTER_ENGINE_SWITCHES");
  gint64 count = 0;
  if (set != nullptr) {
    count = g_ascii_strtoll(set, nullptr, 10);
    if (count < 0) {
      count = 0;
    }
  }
  count += 1;
  g_autofree gchar* key =
      g_strdup_printf("FLUTTER_ENGINE_SWITCH_%" G_GINT64_FORMAT, count);
  g_setenv(key, "enable-impeller=false", TRUE);
  g_autofree gchar* total = g_strdup_printf("%" G_GINT64_FORMAT, count);
  g_setenv("FLUTTER_ENGINE_SWITCHES", total, TRUE);
}

// Called when first Flutter frame received.
static void first_frame_cb(MyApplication* self, FlView* view) {
  GtkWidget* window = gtk_widget_get_toplevel(GTK_WIDGET(view));
  gtk_widget_show(window);
  // **Maximise here, not before the view exists** (K-749).
  //
  // Lumit opens maximised on all three platforms, but asking for it during
  // `activate` asked an unrealised window with no surface behind it. What the
  // window manager then did with the request and what the engine had been told
  // to render did not have to agree, and on at least one compositor they did
  // not: a 0.3.0 Flatpak log shows the renderer waiting for a 1920x1080 frame
  // against a 958x1078 surface — a half-tiled window — and timing out.
  //
  // By here the view is realised and the window is on screen, so the maximise
  // is an ordinary resize: the window manager grants a size, the engine is told
  // that size, and there is only one answer in play. The cost is that the
  // window is briefly its default size on the way up, which is what the Windows
  // runner's own SW_MAXIMIZE-at-show does too.
  gtk_window_maximize(GTK_WINDOW(window));
}

// Implements GApplication::activate.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));

  // Use a header bar when running in GNOME as this is the common style used
  // by applications and is the setup most users will be using (e.g. Ubuntu
  // desktop).
  // If running on X and not using GNOME then just use a traditional title bar
  // in case the window manager does more exotic layout, e.g. tiling.
  // If running on Wayland assume the header bar will work (may need changing
  // if future cases occur).
  gboolean use_header_bar = FALSE;
#ifdef GDK_WINDOWING_X11
  GdkScreen* screen = gtk_window_get_screen(window);
  if (GDK_IS_X11_SCREEN(screen)) {
    const gchar* wm_name = gdk_x11_screen_get_window_manager_name(screen);
    if (g_strcmp0(wm_name, "GNOME Shell") != 0) {
      use_header_bar = FALSE;
    }
  }
#endif
  if (use_header_bar) {
    GtkHeaderBar* header_bar = GTK_HEADER_BAR(gtk_header_bar_new());
    gtk_widget_show(GTK_WIDGET(header_bar));
    gtk_header_bar_set_title(header_bar, "lumit_flutter");
    gtk_header_bar_set_show_close_button(header_bar, TRUE);
    gtk_window_set_titlebar(window, GTK_WIDGET(header_bar));
  } else {
    gtk_window_set_title(window, "lumit_flutter");
  }

  gtk_window_set_default_size(window, 1280, 720);

  // Lumit opens maximised, as it does on Windows (windows/runner/
  // win32_window.cpp) and macOS — but the asking happens in `first_frame_cb`,
  // once there is a window on screen to maximise (K-749). Only the default,
  // though: GTK has nothing like AppKit's frame autosave, so remembering the
  // size and position between runs would mean following configure-events and
  // writing a file by hand here.
  // ponytail: default only, add the remembering when a Linux user asks.

  // Before the engine exists, which is what `fl_view_new` below creates.
  pin_skia();

  g_autoptr(FlDartProject) project = fl_dart_project_new();
  fl_dart_project_set_dart_entrypoint_arguments(
      project, self->dart_entrypoint_arguments);

  FlView* view = fl_view_new(project);
  GdkRGBA background_color;
  // Background defaults to black, override it here if necessary, e.g. #00000000
  // for transparent.
  gdk_rgba_parse(&background_color, "#000000");
  fl_view_set_background_color(view, &background_color);
  gtk_widget_show(GTK_WIDGET(view));
  gtk_container_add(GTK_CONTAINER(window), GTK_WIDGET(view));

  // Show the window when Flutter renders.
  // Requires the view to be realized so we can start rendering.
  g_signal_connect_swapped(view, "first-frame", G_CALLBACK(first_frame_cb),
                           self);
  gtk_widget_realize(GTK_WIDGET(view));

  fl_register_plugins(FL_PLUGIN_REGISTRY(view));

  // The zero-copy Viewer bridge (K-177): register the 'lumit/viewer_texture'
  // channel on the main engine so Dart can hand it engine-drawn DMA-BUF frames to
  // show as GL external textures — the Linux twin of the Windows runner's
  // ViewerTextureBridge (flutter_window.cpp OnCreate). Only the main window shows
  // the Viewer, so — exactly as on Windows — the bridge is registered here alone,
  // not on the popped-out panels below. A build of the engine `.so` without the
  // `shared-texture-linux` feature simply never calls the channel (Dart keeps the
  // read-back path), so this registration is inert then.
  FlEngine* engine = fl_view_get_engine(view);
  viewer_texture_bridge_register(fl_engine_get_binary_messenger(engine),
                                 fl_engine_get_texture_registrar(engine));

  gtk_widget_grab_focus(GTK_WIDGET(view));
}

// Implements GApplication::local_command_line.
static gboolean my_application_local_command_line(GApplication* application,
                                                  gchar*** arguments,
                                                  int* exit_status) {
  MyApplication* self = MY_APPLICATION(application);
  // Strip out the first argument as it is the binary name.
  self->dart_entrypoint_arguments = g_strdupv(*arguments + 1);

  g_autoptr(GError) error = nullptr;
  if (!g_application_register(application, nullptr, &error)) {
    g_warning("Failed to register: %s", error->message);
    *exit_status = 1;
    return TRUE;
  }

  g_application_activate(application);
  *exit_status = 0;

  return TRUE;
}

// Implements GApplication::startup.
static void my_application_startup(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application startup.

  G_APPLICATION_CLASS(my_application_parent_class)->startup(application);
}

// Implements GApplication::shutdown.
static void my_application_shutdown(GApplication* application) {
  // MyApplication* self = MY_APPLICATION(object);

  // Perform any actions required at application shutdown.

  G_APPLICATION_CLASS(my_application_parent_class)->shutdown(application);
}

// Implements GObject::dispose.
static void my_application_dispose(GObject* object) {
  MyApplication* self = MY_APPLICATION(object);
  g_clear_pointer(&self->dart_entrypoint_arguments, g_strfreev);
  G_OBJECT_CLASS(my_application_parent_class)->dispose(object);
}

static void my_application_class_init(MyApplicationClass* klass) {
  G_APPLICATION_CLASS(klass)->activate = my_application_activate;
  G_APPLICATION_CLASS(klass)->local_command_line =
      my_application_local_command_line;
  G_APPLICATION_CLASS(klass)->startup = my_application_startup;
  G_APPLICATION_CLASS(klass)->shutdown = my_application_shutdown;
  G_OBJECT_CLASS(klass)->dispose = my_application_dispose;
}

static void my_application_init(MyApplication* self) {}

MyApplication* my_application_new() {
  // Set the program name to the application ID, which helps various systems
  // like GTK and desktop environments map this running application to its
  // corresponding .desktop file. This ensures better integration by allowing
  // the application to be recognized beyond its binary name.
  g_set_prgname(APPLICATION_ID);

  return MY_APPLICATION(g_object_new(my_application_get_type(),
                                     "application-id", APPLICATION_ID, "flags",
                                     G_APPLICATION_NON_UNIQUE, nullptr));
}
