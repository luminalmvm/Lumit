#include "my_application.h"

#include <dlfcn.h>
#include <glib/gstdio.h>
#include <flutter_linux/flutter_linux.h>

#include "extension_views.h"
#include "flutter/generated_plugin_registrant.h"
#include "viewer_texture_bridge.h"

// The tray, from libayatana-appindicator (or the older libappindicator it
// came from), opened while Lumit runs so nothing new is needed to build or
// start it. A desktop with no tray, GNOME as it ships, shows no icon, and
// opening Lumit again brings the window back.
typedef void* (*IndicatorNew)(const gchar* id, const gchar* icon,
                              int category);
typedef void (*IndicatorSetStatus)(void* indicator, int status);
typedef void (*IndicatorSetMenu)(void* indicator, GtkMenu* menu);

struct _MyApplication {
  GtkApplication parent_instance;
  char** dart_entrypoint_arguments;
  GtkWindow* window;
  FlView* view;
  FlMethodChannel* window_channel;
  FlMethodChannel* open_channel;
  // Started at sign-in and not shown yet.
  gboolean start_hidden;
  // Settings says closing the window keeps Lumit running in the tray.
  gboolean keep_running;
  // What a second start passed on, until Dart takes it (lumit/open).
  GPtrArray* open_waiting;
  void* indicator;
  IndicatorSetStatus indicator_set_status;
  GtkWidget* tray_open;
  GtkWidget* tray_quit;
  // The start-up splash's window and picture while it is up. The main window
  // waits for it to go.
  GtkWidget* splash;
  cairo_surface_t* splash_picture;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

// The file whose being there makes Lumit one Lumit at its next start, so a
// second start hands its arguments over (forward_cb). Written while Lumit is
// kept running in the background, beside the settings as own-title-bar is.
static gchar* keep_running_file() {
  return g_build_filename(g_get_home_dir(), ".config", "lumit", "keep-running",
                          nullptr);
}

// Start Lumit in the tray when the person signs in, with an autostart entry
// that passes --background, or stop doing so.
static void set_open_at_sign_in(gboolean on) {
  g_autofree gchar* entry = g_build_filename(
      g_get_home_dir(), ".config", "autostart", "lumit.desktop", nullptr);
  if (!on) {
    g_remove(entry);
    return;
  }
  g_autofree gchar* exec = nullptr;
  if (g_file_test("/.flatpak-info", G_FILE_TEST_EXISTS)) {
    exec = g_strdup("flatpak run " APPLICATION_ID " --background");
  } else {
    g_autofree gchar* self = g_file_read_link("/proc/self/exe", nullptr);
    if (self == nullptr) {
      return;
    }
    exec = g_strdup_printf("\"%s\" --background", self);
  }
  g_autofree gchar* text = g_strdup_printf(
      "[Desktop Entry]\nType=Application\nName=Lumit\nExec=%s\n"
      "X-GNOME-Autostart-enabled=true\n",
      exec);
  g_autofree gchar* folder = g_path_get_dirname(entry);
  g_mkdir_with_parents(folder, 0700);
  g_file_set_contents(entry, text, -1, nullptr);
}

// Show the window and bring it to the front, from the tray or a second start.
static void reveal(MyApplication* self) {
  self->start_hidden = FALSE;
  gtk_widget_show(GTK_WIDGET(self->window));
  gtk_window_present(self->window);
}

static void tray_quit_cb(MyApplication* self) {
  fl_method_channel_invoke_method(self->window_channel, "quit", nullptr,
                                  nullptr, nullptr, nullptr);
}

// Put Lumit's icon in the tray, or take it away.
static void show_tray(MyApplication* self, gboolean shown, const gchar* open,
                      const gchar* quit) {
  if (self->indicator == nullptr) {
    if (!shown) {
      return;
    }
    void* library = dlopen("libayatana-appindicator3.so.1", RTLD_LAZY);
    if (library == nullptr) {
      library = dlopen("libappindicator3.so.1", RTLD_LAZY);
    }
    if (library == nullptr) {
      return;
    }
    auto make = reinterpret_cast<IndicatorNew>(
        dlsym(library, "app_indicator_new"));
    auto set_menu = reinterpret_cast<IndicatorSetMenu>(
        dlsym(library, "app_indicator_set_menu"));
    self->indicator_set_status = reinterpret_cast<IndicatorSetStatus>(
        dlsym(library, "app_indicator_set_status"));
    if (make == nullptr || set_menu == nullptr ||
        self->indicator_set_status == nullptr) {
      return;
    }
    GtkWidget* menu = gtk_menu_new();
    self->tray_open = gtk_menu_item_new_with_label("");
    self->tray_quit = gtk_menu_item_new_with_label("");
    gtk_menu_shell_append(GTK_MENU_SHELL(menu), self->tray_open);
    gtk_menu_shell_append(GTK_MENU_SHELL(menu), self->tray_quit);
    g_signal_connect_swapped(self->tray_open, "activate", G_CALLBACK(reveal),
                             self);
    g_signal_connect_swapped(self->tray_quit, "activate",
                             G_CALLBACK(tray_quit_cb), self);
    gtk_widget_show_all(menu);
    // The icon the desktop entry names, which a Flatpak calls by its id.
    const gchar* icon =
        g_file_test("/.flatpak-info", G_FILE_TEST_EXISTS) ? APPLICATION_ID
                                                          : "lumit";
    // Category 0 is an application's status, the ordinary kind.
    self->indicator = make(APPLICATION_ID, icon, 0);
    set_menu(self->indicator, GTK_MENU(menu));
  }
  gtk_menu_item_set_label(GTK_MENU_ITEM(self->tray_open), open);
  gtk_menu_item_set_label(GTK_MENU_ITEM(self->tray_quit), quit);
  // 1 shows the icon, 0 hides it.
  self->indicator_set_status(self->indicator, shown ? 1 : 0);
}

// Kept running in the background, closing the window only hides it. Connected
// before the view's own handler, which would take the close for a quit. Quit
// in the tray goes through Dart instead.
static gboolean window_delete_cb(GtkWidget* window, GdkEvent* event,
                                 MyApplication* self) {
  if (!self->keep_running) {
    return FALSE;
  }
  gtk_widget_hide(window);
  fl_method_channel_invoke_method(self->window_channel, "hidden", nullptr,
                                  nullptr, nullptr, nullptr);
  return TRUE;
}

// A second start's arguments, handed over by GApplication
// (my_application_local_command_line).
static void forward_cb(GSimpleAction* action,
                       GVariant* parameter,
                       gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  g_autofree const gchar** asked = g_variant_get_strv(parameter, nullptr);
  gboolean background = FALSE;
  const guint before = self->open_waiting->len;
  for (const gchar** argument = asked; *argument != nullptr; argument++) {
    if (g_strcmp0(*argument, "--background") == 0) {
      background = TRUE;
    } else if (**argument != '\0') {
      g_ptr_array_add(self->open_waiting, g_strdup(*argument));
    }
  }
  // Another start at sign-in has nothing to show.
  if (!background && self->window != nullptr) {
    reveal(self);
  }
  if (self->open_waiting->len > before && self->open_channel != nullptr) {
    fl_method_channel_invoke_method(self->open_channel, "open", nullptr,
                                    nullptr, nullptr, nullptr);
  }
}

// Dart taking what a second start passed on, as macOS's runner does with
// what the Finder asks for.
static void open_method_cb(FlMethodChannel* channel,
                           FlMethodCall* call,
                           gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  if (g_strcmp0(fl_method_call_get_name(call), "take") != 0) {
    fl_method_call_respond_not_implemented(call, nullptr);
    return;
  }
  g_autoptr(FlValue) asked = fl_value_new_list();
  for (guint i = 0; i < self->open_waiting->len; i++) {
    fl_value_append_take(asked, fl_value_new_string(static_cast<const gchar*>(
                                    g_ptr_array_index(self->open_waiting, i))));
  }
  g_ptr_array_set_size(self->open_waiting, 0);
  fl_method_call_respond_success(call, asked, nullptr);
}

// Called when first Flutter frame received.
static void first_frame_cb(MyApplication* self, FlView* view) {
  GtkWidget* window = gtk_widget_get_toplevel(GTK_WIDGET(view));
  // With the splash up, the window shows and is maximised when the splash goes
  // (close_splash).
  if (self->splash != nullptr) {
    return;
  }
  // Started at sign-in, it waits in the tray until it is opened.
  if (!self->start_hidden) {
    gtk_widget_show(window);
  }
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

// The start-up splash (lib/shell/splash_window.dart) is a window of its own,
// the shape of its picture where the desktop can show see-through windows.
// Without a compositor it is the picture on Lumit's dark ground.
static gboolean splash_draw_cb(GtkWidget* widget, cairo_t* cr,
                               MyApplication* self) {
  if (gdk_screen_is_composited(gtk_widget_get_screen(widget))) {
    cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
    cairo_set_source_rgba(cr, 0, 0, 0, 0);
    cairo_paint(cr);
    cairo_set_operator(cr, CAIRO_OPERATOR_OVER);
  } else {
    cairo_set_source_rgb(cr, 0x0b / 255.0, 0x0c / 255.0, 0x0e / 255.0);
    cairo_paint(cr);
  }
  if (self->splash_picture != nullptr) {
    cairo_set_source_surface(cr, self->splash_picture, 0, 0);
    cairo_paint(cr);
  }
  return TRUE;
}

// Any press on the splash moves it.
static gboolean splash_press_cb(GtkWidget* widget, GdkEventButton* event,
                                gpointer user_data) {
  if (event->button == GDK_BUTTON_PRIMARY) {
    gtk_window_begin_move_drag(GTK_WINDOW(widget), event->button,
                               static_cast<gint>(event->x_root),
                               static_cast<gint>(event->y_root), event->time);
  }
  return TRUE;
}

// The primary monitor's scale, which the splash is drawn at.
static gint splash_scale() {
  GdkDisplay* display = gdk_display_get_default();
  GdkMonitor* monitor = gdk_display_get_primary_monitor(display);
  if (monitor == nullptr) {
    monitor = gdk_display_get_monitor(display, 0);
  }
  return monitor != nullptr ? gdk_monitor_get_scale_factor(monitor) : 1;
}

// Put the splash on screen, or give it new pixels: premultiplied RGBA,
// "width" by "height", in "bytes". False if it could not.
static gboolean show_splash(MyApplication* self, FlValue* args) {
  if (self->start_hidden || fl_value_get_type(args) != FL_VALUE_TYPE_MAP) {
    return FALSE;
  }
  FlValue* width_value = fl_value_lookup_string(args, "width");
  FlValue* height_value = fl_value_lookup_string(args, "height");
  FlValue* bytes = fl_value_lookup_string(args, "bytes");
  if (width_value == nullptr || height_value == nullptr || bytes == nullptr ||
      fl_value_get_type(width_value) != FL_VALUE_TYPE_INT ||
      fl_value_get_type(height_value) != FL_VALUE_TYPE_INT ||
      fl_value_get_type(bytes) != FL_VALUE_TYPE_UINT8_LIST) {
    return FALSE;
  }
  const int64_t width = fl_value_get_int(width_value);
  const int64_t height = fl_value_get_int(height_value);
  if (width <= 0 || height <= 0 || width > 16384 || height > 16384 ||
      static_cast<int64_t>(fl_value_get_length(bytes)) < width * height * 4) {
    return FALSE;
  }
  cairo_surface_t* picture = cairo_image_surface_create(
      CAIRO_FORMAT_ARGB32, static_cast<int>(width), static_cast<int>(height));
  if (cairo_surface_status(picture) != CAIRO_STATUS_SUCCESS) {
    cairo_surface_destroy(picture);
    return FALSE;
  }
  // Cairo keeps each pixel as one native-endian ARGB word, premultiplied.
  cairo_surface_flush(picture);
  const uint8_t* rgba = fl_value_get_uint8_list(bytes);
  unsigned char* data = cairo_image_surface_get_data(picture);
  const int stride = cairo_image_surface_get_stride(picture);
  for (int64_t y = 0; y < height; y++) {
    auto* row = reinterpret_cast<uint32_t*>(data + y * stride);
    for (int64_t x = 0; x < width; x++) {
      const uint8_t* in = rgba + (y * width + x) * 4;
      row[x] = (static_cast<uint32_t>(in[3]) << 24) |
               (static_cast<uint32_t>(in[0]) << 16) |
               (static_cast<uint32_t>(in[1]) << 8) | in[2];
    }
  }
  cairo_surface_mark_dirty(picture);
  const gint scale = splash_scale();
  cairo_surface_set_device_scale(picture, scale, scale);
  if (self->splash_picture != nullptr) {
    cairo_surface_destroy(self->splash_picture);
  }
  self->splash_picture = picture;

  if (self->splash != nullptr) {
    gtk_widget_queue_draw(self->splash);
    return TRUE;
  }
  GtkWidget* splash = gtk_window_new(GTK_WINDOW_TOPLEVEL);
  gtk_window_set_title(GTK_WINDOW(splash), "Lumit");
  gtk_window_set_decorated(GTK_WINDOW(splash), FALSE);
  gtk_window_set_resizable(GTK_WINDOW(splash), FALSE);
  gtk_widget_set_app_paintable(splash, TRUE);
  GdkScreen* screen = gtk_widget_get_screen(splash);
  GdkVisual* visual = gdk_screen_get_rgba_visual(screen);
  if (visual != nullptr && gdk_screen_is_composited(screen)) {
    gtk_widget_set_visual(splash, visual);
  }
  gtk_window_set_default_size(GTK_WINDOW(splash),
                              static_cast<gint>(width / scale),
                              static_cast<gint>(height / scale));
  gtk_window_set_position(GTK_WINDOW(splash), GTK_WIN_POS_CENTER);
  gtk_widget_add_events(splash, GDK_BUTTON_PRESS_MASK);
  g_signal_connect(splash, "draw", G_CALLBACK(splash_draw_cb), self);
  g_signal_connect(splash, "button-press-event", G_CALLBACK(splash_press_cb),
                   nullptr);
  self->splash = splash;
  gtk_widget_show(splash);
  return TRUE;
}

// Take the splash down. The main window shows, and is maximised, in its
// place: the first frame came and went while the splash was up.
static void close_splash(MyApplication* self) {
  const gboolean was_up = self->splash != nullptr;
  if (self->splash != nullptr) {
    gtk_widget_destroy(self->splash);
    self->splash = nullptr;
  }
  if (self->splash_picture != nullptr) {
    cairo_surface_destroy(self->splash_picture);
    self->splash_picture = nullptr;
  }
  if (self->window == nullptr) {
    return;
  }
  if (!self->start_hidden &&
      !gtk_widget_get_visible(GTK_WIDGET(self->window))) {
    gtk_widget_show(GTK_WIDGET(self->window));
    gtk_window_present(self->window);
  }
  if (was_up) {
    gtk_window_maximize(self->window);
  }
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
  MyApplication* self = MY_APPLICATION(user_data);
  const gchar* asked = fl_method_call_get_name(call);
  // The splash's three have answers of their own.
  if (g_strcmp0(asked, "splashScale") == 0) {
    g_autoptr(FlValue) scale =
        self->start_hidden ? fl_value_new_null()
                           : fl_value_new_float(splash_scale());
    fl_method_call_respond_success(call, scale, nullptr);
    return;
  }
  if (g_strcmp0(asked, "splash") == 0) {
    g_autoptr(FlValue) shown =
        fl_value_new_bool(show_splash(self, fl_method_call_get_args(call)));
    fl_method_call_respond_success(call, shown, nullptr);
    return;
  }
  if (g_strcmp0(asked, "splashClose") == 0) {
    fl_method_call_respond_success(call, nullptr, nullptr);
    close_splash(self);
    return;
  }
  GtkWindow* window = self->window;
  FlView* view = self->view;
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
  } else if (g_strcmp0(method, "background") == 0) {
    // Settings: whether closing the window keeps Lumit running in the tray
    // (lib/state/background.dart), and the tray menu's words.
    FlValue* args = fl_method_call_get_args(call);
    auto value = [args](const gchar* key, FlValueType type) -> FlValue* {
      if (fl_value_get_type(args) != FL_VALUE_TYPE_MAP) {
        return nullptr;
      }
      FlValue* found = fl_value_lookup_string(args, key);
      return found != nullptr && fl_value_get_type(found) == type ? found
                                                                  : nullptr;
    };
    auto text = [&value](const gchar* key) {
      FlValue* found = value(key, FL_VALUE_TYPE_STRING);
      return found != nullptr ? fl_value_get_string(found) : "";
    };
    FlValue* keep = value("keep", FL_VALUE_TYPE_BOOL);
    self->keep_running = keep != nullptr && fl_value_get_bool(keep);
    g_autofree gchar* marker = keep_running_file();
    if (self->keep_running) {
      g_autofree gchar* folder = g_path_get_dirname(marker);
      g_mkdir_with_parents(folder, 0700);
      g_file_set_contents(marker, "", 0, nullptr);
    } else {
      g_remove(marker);
    }
    show_tray(self, self->keep_running, text("open"), text("quit"));
    FlValue* sign_in = value("signIn", FL_VALUE_TYPE_BOOL);
    set_open_at_sign_in(self->keep_running && sign_in != nullptr &&
                        fl_value_get_bool(sign_in));
    // Started at sign-in with nothing now to come back from: the window is
    // the only way back, so it shows.
    if (!self->keep_running && self->start_hidden) {
      reveal(self);
    }
  } else if (g_strcmp0(method, "show") == 0) {
    reveal(self);
  }
}

// Implements GApplication::activate.
static void my_application_activate(GApplication* application) {
  MyApplication* self = MY_APPLICATION(application);
  GtkWindow* window =
      GTK_WINDOW(gtk_application_window_new(GTK_APPLICATION(application)));
  self->window = window;
  // Before the view connects its own, so this one is asked first.
  g_signal_connect(window, "delete-event", G_CALLBACK(window_delete_cb), self);

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
  self->view = view;
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

  // desktop_drop asks a drop for the Flatpak's file transfer key first, and
  // GTK has ended the drop by the time it reads it, so nothing arrives. The
  // plain list of files works, and the Flatpak can read where footage is kept.
  GtkTargetList* drop_targets = gtk_target_list_new(nullptr, 0);
  gtk_target_list_add(drop_targets,
                      gdk_atom_intern_static_string("text/uri-list"), 0, 0);
  gtk_drag_dest_set_target_list(GTK_WIDGET(view), drop_targets);
  gtk_target_list_unref(drop_targets);

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

  // These live as long as the window, which is as long as the application.
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  self->window_channel =
      fl_method_channel_new(fl_engine_get_binary_messenger(engine),
                            "lumit/window", FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(self->window_channel,
                                            window_method_cb, self, nullptr);
  self->open_channel =
      fl_method_channel_new(fl_engine_get_binary_messenger(engine),
                            "lumit/open", FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(self->open_channel, open_method_cb,
                                            self, nullptr);

  gtk_widget_grab_focus(GTK_WIDGET(view));
}

// Implements GApplication::local_command_line.
static gboolean my_application_local_command_line(GApplication* application,
                                                  gchar*** arguments,
                                                  int* exit_status) {
  MyApplication* self = MY_APPLICATION(application);
  // Strip out the first argument as it is the binary name.
  self->dart_entrypoint_arguments = g_strdupv(*arguments + 1);
  // Started at sign-in, to sit in the tray until it is opened.
  self->start_hidden = g_strv_contains(
      const_cast<const gchar* const*>(self->dart_entrypoint_arguments),
      "--background");

  g_autoptr(GError) error = nullptr;
  gboolean registered = g_application_register(application, nullptr, &error);
  // Kept running in the background, Lumit asks to be the one Lumit. Where
  // that can't be had, with no session bus for instance, it starts as a Lumit
  // of its own, as it does with the setting off.
  if (!registered && !(g_application_get_flags(application) &
                       G_APPLICATION_NON_UNIQUE)) {
    g_clear_error(&error);
    g_application_set_flags(application, G_APPLICATION_NON_UNIQUE);
    registered = g_application_register(application, nullptr, &error);
  }
  if (!registered) {
    g_warning("Failed to register: %s", error->message);
    *exit_status = 1;
    return TRUE;
  }

  // A Lumit kept running in the background has this one's name. It is the
  // one that shows itself and opens what was asked for.
  if (g_application_get_is_remote(application)) {
    g_action_group_activate_action(
        G_ACTION_GROUP(application), "forward",
        g_variant_new_strv(
            const_cast<const gchar* const*>(self->dart_entrypoint_arguments),
            -1));
    *exit_status = 0;
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
  g_clear_pointer(&self->open_waiting, g_ptr_array_unref);
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

static void my_application_init(MyApplication* self) {
  self->open_waiting = g_ptr_array_new_with_free_func(g_free);
  // What a second start calls on this one, when this one is kept running in
  // the background and so holds the name.
  g_autoptr(GSimpleAction) forward =
      g_simple_action_new("forward", G_VARIANT_TYPE_STRING_ARRAY);
  g_signal_connect(forward, "activate", G_CALLBACK(forward_cb), self);
  g_action_map_add_action(G_ACTION_MAP(self), G_ACTION(forward));
}

MyApplication* my_application_new() {
  // Set the program name to the application ID, which helps various systems
  // like GTK and desktop environments map this running application to its
  // corresponding .desktop file. This ensures better integration by allowing
  // the application to be recognized beyond its binary name.
  g_set_prgname(APPLICATION_ID);

  // Kept running in the background, Lumit is one Lumit and a second start
  // hands its arguments to it. Otherwise every start is a Lumit of its own,
  // as it always was.
  g_autofree gchar* keep_running = keep_running_file();
  const GApplicationFlags flags =
      g_file_test(keep_running, G_FILE_TEST_EXISTS)
          ? static_cast<GApplicationFlags>(0)
          : G_APPLICATION_NON_UNIQUE;
  return MY_APPLICATION(g_object_new(my_application_get_type(),
                                     "application-id", APPLICATION_ID, "flags",
                                     flags, nullptr));
}
