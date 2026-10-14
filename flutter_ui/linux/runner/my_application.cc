#include "my_application.h"

#include <flutter_linux/flutter_linux.h>

#include "extension_views.h"
#include "flutter/generated_plugin_registrant.h"
#include "viewer_texture_bridge.h"

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

// Called when first Flutter frame received.
static void first_frame_cb(MyApplication* self, FlView* view) {
  GtkWidget* window = gtk_widget_get_toplevel(GTK_WIDGET(view));
  gtk_widget_show(window);
  // **Maximise here, not before the view exists**.
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

// Hand the window to the window manager to be moved, as a press on a title bar
// would.
static void start_window_move(FlView* view, GtkWindow* window) {
  GdkDevice* pointer = gdk_seat_get_pointer(gdk_display_get_default_seat(
      gtk_widget_get_display(GTK_WIDGET(window))));
  gint x = 0, y = 0;
  gdk_device_get_position(pointer, nullptr, &x, &y);
  gtk_window_begin_move_drag(window, GDK_BUTTON_PRIMARY, x, y,
                             GDK_CURRENT_TIME);

  // The window manager keeps the pointer until the button comes up, so the
  // release never reaches Flutter. It would go on believing the button is
  // down and take the next click for the end of this drag, so it is told now.
  // The view listens for the mouse on an event box of its own.
  g_autoptr(GList) children = gtk_container_get_children(GTK_CONTAINER(view));
  for (GList* child = children; child != nullptr; child = child->next) {
    if (!GTK_IS_EVENT_BOX(child->data)) {
      continue;
    }
    GdkEvent* release = gdk_event_new(GDK_BUTTON_RELEASE);
    release->button.button = GDK_BUTTON_PRIMARY;
    gdk_event_set_device(release, pointer);
    gboolean handled = FALSE;
    g_signal_emit_by_name(child->data, "button-release-event", release,
                          &handled);
    gdk_event_free(release);
  }
}

// The window has no title bar, so Flutter draws the buttons one would hold
// and says here what was pressed (lib/shell/window_frame.dart). Every call
// answers whether the window is maximised, which is all the button's drawing
// needs to know. The answer goes first, since closing takes the engine with
// it.
static void window_method_cb(FlMethodChannel* channel,
                             FlMethodCall* call,
                             gpointer user_data) {
  FlView* view = FL_VIEW(user_data);
  GtkWindow* window = GTK_WINDOW(gtk_widget_get_toplevel(GTK_WIDGET(view)));
  const gboolean maximised = gtk_window_is_maximized(window);
  g_autoptr(FlValue) answer = fl_value_new_bool(maximised);
  fl_method_call_respond_success(call, answer, nullptr);

  const gchar* method = fl_method_call_get_name(call);
  if (g_strcmp0(method, "drag") == 0) {
    start_window_move(view, window);
  } else if (g_strcmp0(method, "minimise") == 0) {
    gtk_window_iconify(window);
  } else if (g_strcmp0(method, "maximise") == 0) {
    if (maximised) {
      gtk_window_unmaximize(window);
    } else {
      gtk_window_maximize(window);
    }
  } else if (g_strcmp0(method, "close") == 0) {
    gtk_window_close(window);
  }
}

// Implements GApplication::activate.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));

  // Lumit's own title bar is switched on in Settings, which leaves this file
  // beside the settings (lib/shell/window_frame.dart). Without it the window
  // manager's title bar stays.
  //
  // With it there is no title bar. Lumit's menu bar is the window's top line
  // and carries the window's buttons. A header bar that is never shown is how
  // GTK is told so: the window still gets the frame GTK draws for itself, with
  // its shadow and its resize edges, and nothing along the top.
  g_autofree gchar* own_title_bar = g_build_filename(
      g_get_home_dir(), ".config", "lumit", "own-title-bar", nullptr);
  if (g_file_test(own_title_bar, G_FILE_TEST_EXISTS)) {
    gtk_window_set_titlebar(window, gtk_header_bar_new());
  }
  gtk_window_set_title(window, "lumit_flutter");

  gtk_window_set_default_size(window, 1280, 720);
  // The smallest it can be made, as on Windows (windows/runner/
  // win32_window.cpp): room for the default workspace's narrowest panels.
  GdkGeometry smallest = {};
  smallest.min_width = 840;
  smallest.min_height = 560;
  gtk_window_set_geometry_hints(window, nullptr, &smallest, GDK_HINT_MIN_SIZE);

  // Lumit opens maximised, as it does on Windows (windows/runner/
  // win32_window.cpp) and macOS — but the asking happens in `first_frame_cb`,
  // once there is a window on screen to maximise. Only the default, though:
  // GTK has nothing like AppKit's frame autosave, so remembering the size and
  // position between runs would mean following configure-events and writing a
  // file by hand here.
  // ponytail: default only, add the remembering when a Linux user asks.

  g_autoptr(FlDartProject) project = fl_dart_project_new();
  // Pin the renderer to Skia, as the Windows runner does with its typed
  // `set_impeller_switch`.
  //
  // In plain terms: Flutter has two renderers, and the newer one — Impeller —
  // is measurably slower for the way Lumit draws (docs/impl/ui-performance.md
  // 2.4/4.1/7.2), and on Linux it never draws the Viewer's external texture at
  // all (issue #104). This says so on the project, which is a property the
  // embedder reads unconditionally.
  //
  // **Not the environment variables `flutter run` uses.** The earlier pin
  // appended to FLUTTER_ENGINE_SWITCHES, which is how the tool does it — and
  // the engine reads those only under `#ifndef FLUTTER_RELEASE`
  // (shell/platform/common/engine_switches.cc). A release build ignores them
  // entirely, so that pin held in `flutter run` and did nothing in the
  // shipped Flatpak. Flip this to TRUE the day a Flutter upgrade's re-run of
  // ui-performance 2.4's A/B shows Impeller clearing the mandate.
  fl_dart_project_set_enable_impeller(project, FALSE);
  fl_dart_project_set_dart_entrypoint_arguments(
      project, self->dart_entrypoint_arguments);

  FlView* view = fl_view_new(project);
  GdkRGBA background_color;
  // Background defaults to black, override it here if necessary, e.g. #00000000
  // for transparent.
  gdk_rgba_parse(&background_color, "#000000");
  fl_view_set_background_color(view, &background_color);
  gtk_widget_show(GTK_WIDGET(view));
  // The view is the base of an overlay, and not the window's own child, so
  // that an extension's page can be laid over it (extension_views.h). With
  // nothing laid over it the overlay is the view and nothing more.
  GtkWidget* overlay = gtk_overlay_new();
  gtk_widget_show(overlay);
  gtk_container_add(GTK_CONTAINER(overlay), GTK_WIDGET(view));
  gtk_container_add(GTK_CONTAINER(window), overlay);

  // Show the window when Flutter renders.
  // Requires the view to be realized so we can start rendering.
  g_signal_connect_swapped(view, "first-frame", G_CALLBACK(first_frame_cb),
                           self);
  gtk_widget_realize(GTK_WIDGET(view));

  fl_register_plugins(FL_PLUGIN_REGISTRY(view));

  // The zero-copy Viewer bridge: register the 'lumit/viewer_texture' channel
  // on the main engine so Dart can hand it engine-drawn DMA-BUF frames to
  // show as GL external textures — the Linux twin of the Windows runner's
  // ViewerTextureBridge (flutter_window.cpp OnCreate). Only the main window shows
  // the Viewer, so — exactly as on Windows — the bridge is registered here alone,
  // not on the popped-out panels below. A build of the engine `.so` without the
  // `shared-texture-linux` feature simply never calls the channel (Dart keeps the
  // read-back path), so this registration is inert then.
  FlEngine* engine = fl_view_get_engine(view);
  viewer_texture_bridge_register(fl_engine_get_binary_messenger(engine),
                                 fl_engine_get_texture_registrar(engine));
  extension_views_register(fl_engine_get_binary_messenger(engine),
                           GTK_OVERLAY(overlay), view);

  // Lives as long as the window, which is as long as the application.
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  FlMethodChannel* window_channel =
      fl_method_channel_new(fl_engine_get_binary_messenger(engine),
                            "lumit/window", FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(window_channel, window_method_cb,
                                            view, nullptr);

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
