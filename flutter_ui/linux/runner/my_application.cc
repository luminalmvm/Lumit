#include "my_application.h"

#include <dlfcn.h>
#include <glib/gstdio.h>
#include <flutter_linux/flutter_linux.h>
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif

#include "backdrop.h"
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
  // What the view sits in, which a see-through window blurs behind.
  GtkWidget* content;
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
  // Whether the window is maximised as it first shows: as it was when Lumit
  // last closed, and always the first time.
  gboolean start_maximised;
  // Waiting for things to be still for a moment, before writing down where
  // the window is and before telling Dart the displays changed.
  guint save_source;
  guint displays_source;
  // The outline of the window a dragged panel would make, and its colour.
  GtkWidget* ghost;
  guint32 ghost_colour;
  // The size the ghost was last cut to its frame at, without a compositor.
  gint ghost_width;
  gint ghost_height;
};

G_DEFINE_TYPE(MyApplication, my_application, GTK_TYPE_APPLICATION)

// The file whose being there makes Lumit one Lumit at its next start, so a
// second start hands its arguments over (forward_cb). Written while Lumit is
// kept running in the background, beside the settings as own-title-bar is.
static gchar* keep_running_file() {
  return g_build_filename(g_get_home_dir(), ".config", "lumit", "keep-running",
                          nullptr);
}

// Where the window was when Lumit last closed, beside the settings as
// own-title-bar is.
static gchar* window_file() {
  return g_build_filename(g_get_home_dir(), ".config", "lumit", "window.ini",
                          nullptr);
}

// A window's size and place as it stands when it is neither maximised, full
// screen, tiled nor minimised, which GTK does not keep. Followed for the main
// window and every floating window.
static const gchar kNormal[] = "lumit-normal";

// Marks a floating window hidden with the main window, to show again with it.
static const gchar kHidden[] = "lumit-hidden";

static gboolean normal_bounds_cb(GtkWidget* widget,
                                 GdkEvent* event,
                                 gpointer user_data) {
  auto* normal =
      static_cast<GdkRectangle*>(g_object_get_data(G_OBJECT(widget), kNormal));
  GdkWindow* surface = gtk_widget_get_window(widget);
  if (normal != nullptr && surface != nullptr &&
      !(gdk_window_get_state(surface) &
        (GDK_WINDOW_STATE_MAXIMIZED | GDK_WINDOW_STATE_FULLSCREEN |
         GDK_WINDOW_STATE_TILED | GDK_WINDOW_STATE_ICONIFIED))) {
    gtk_window_get_position(GTK_WINDOW(widget), &normal->x, &normal->y);
    gtk_window_get_size(GTK_WINDOW(widget), &normal->width, &normal->height);
  }
  return FALSE;
}

static void follow_normal_bounds(GtkWindow* window) {
  if (g_object_get_data(G_OBJECT(window), kNormal) != nullptr) {
    return;
  }
  g_object_set_data_full(G_OBJECT(window), kNormal, g_new0(GdkRectangle, 1),
                         g_free);
  g_signal_connect(window, "configure-event", G_CALLBACK(normal_bounds_cb),
                   nullptr);
}

// Write down where the window is, for the next start.
static void save_window(MyApplication* self) {
  g_autoptr(GKeyFile) memory = g_key_file_new();
  g_key_file_set_boolean(memory, "window", "maximised",
                         gtk_window_is_maximized(self->window));
  auto* normal = static_cast<GdkRectangle*>(
      g_object_get_data(G_OBJECT(self->window), kNormal));
  if (normal != nullptr && normal->width > 0 && normal->height > 0) {
    g_key_file_set_integer(memory, "window", "x", normal->x);
    g_key_file_set_integer(memory, "window", "y", normal->y);
    g_key_file_set_integer(memory, "window", "width", normal->width);
    g_key_file_set_integer(memory, "window", "height", normal->height);
  }
  g_autofree gchar* file = window_file();
  g_autofree gchar* folder = g_path_get_dirname(file);
  g_mkdir_with_parents(folder, 0700);
  g_key_file_save_to_file(memory, file, nullptr);
}

static gboolean save_window_cb(gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  self->save_source = 0;
  save_window(self);
  return G_SOURCE_REMOVE;
}

// Each move and resize of the main window, written down once it has been
// still for half a second.
static gboolean main_configure_cb(MyApplication* self) {
  if (self->save_source != 0) {
    g_source_remove(self->save_source);
  }
  self->save_source = g_timeout_add(500, save_window_cb, self);
  return FALSE;
}

// Whether |widget|, one of GTK's windows, is a floating window: one of
// Flutter's own, made from Dart (lib/shell/pop_out.dart), which holds its
// view as its only child. The main window holds its view in an overlay.
static gboolean is_floating(MyApplication* self, gpointer widget) {
  return widget != self->window &&
         FL_IS_VIEW(gtk_bin_get_child(GTK_BIN(widget)));
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
  // The floating windows hidden with it come back with it, first and without
  // the keyboard, so the main window is the one that has it.
  g_autoptr(GList) windows = gtk_window_list_toplevels();
  for (GList* window = windows; window != nullptr; window = window->next) {
    if (is_floating(self, window->data) &&
        g_object_steal_data(G_OBJECT(window->data), kHidden) != nullptr) {
      GtkWindow* floating = GTK_WINDOW(window->data);
      const gboolean focus = gtk_window_get_focus_on_map(floating);
      gtk_window_set_focus_on_map(floating, FALSE);
      gtk_widget_show(GTK_WIDGET(floating));
      gtk_window_set_focus_on_map(floating, focus);
    }
  }
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

// Kept running in the background, closing the window only hides it, and the
// floating windows with it. Connected before the view's own handler, which
// would take the close for a quit. Quit in the tray goes through Dart instead.
//
// Quitting, the floating windows are left alone. Closing one puts its panels
// back in the main window, which would lose the arrangement for the next
// start, so they stay open while Dart asks about unsaved work and go when the
// process ends.
static gboolean window_delete_cb(GtkWidget* window, GdkEvent* event,
                                 MyApplication* self) {
  save_window(self);
  if (!self->keep_running) {
    return FALSE;
  }
  gtk_widget_hide(window);
  g_autoptr(GList) windows = gtk_window_list_toplevels();
  for (GList* other = windows; other != nullptr; other = other->next) {
    if (is_floating(self, other->data) &&
        gtk_widget_get_visible(GTK_WIDGET(other->data))) {
      g_object_set_data(G_OBJECT(other->data), kHidden, GINT_TO_POINTER(1));
      gtk_widget_hide(GTK_WIDGET(other->data));
    }
  }
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
  if (self->start_maximised) {
    gtk_window_maximize(GTK_WINDOW(window));
  }
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

// Take the splash down. The main window shows, and is maximised when it
// starts so, in its place: the first frame came and went while the splash was
// up.
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
  if (was_up && self->start_maximised) {
    gtk_window_maximize(self->window);
  }
}

// Hand the window to the window manager to be moved, as a press on a title bar
// would, or resized from |edge|, a GdkWindowEdge, as a press on that edge of
// its frame would. Less than zero moves it.
static void start_window_drag(FlView* view, GtkWindow* window, gint edge) {
  GdkDevice* pointer = gdk_seat_get_pointer(gdk_display_get_default_seat(
      gtk_widget_get_display(GTK_WIDGET(window))));
  gint x = 0, y = 0;
  gdk_device_get_position(pointer, nullptr, &x, &y);
  if (edge < 0) {
    gtk_window_begin_move_drag(window, GDK_BUTTON_PRIMARY, x, y,
                               GDK_CURRENT_TIME);
  } else {
    gtk_window_begin_resize_drag(window, static_cast<GdkWindowEdge>(edge),
                                 GDK_BUTTON_PRIMARY, x, y, GDK_CURRENT_TIME);
  }

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

// The ghost's colour, 0xAARRGGBB, faint across it and solid round its edge.
static gboolean ghost_draw_cb(GtkWidget* widget,
                              cairo_t* cr,
                              MyApplication* self) {
  const guint32 colour = self->ghost_colour;
  const double alpha = (colour >> 24) / 255.0;
  const double red = ((colour >> 16) & 0xff) / 255.0;
  const double green = ((colour >> 8) & 0xff) / 255.0;
  const double blue = (colour & 0xff) / 255.0;
  cairo_set_operator(cr, CAIRO_OPERATOR_SOURCE);
  cairo_set_source_rgba(cr, red, green, blue, alpha * 0.18);
  cairo_paint(cr);
  cairo_set_operator(cr, CAIRO_OPERATOR_OVER);
  cairo_set_source_rgba(cr, red, green, blue, alpha);
  cairo_set_line_width(cr, 2);
  cairo_rectangle(cr, 1, 1, gtk_widget_get_allocated_width(widget) - 2,
                  gtk_widget_get_allocated_height(widget) - 2);
  cairo_stroke(cr);
  return TRUE;
}

// The value at |key| in a call's arguments when it is of |type|, or null.
static FlValue* argument(FlValue* args, const gchar* key, FlValueType type) {
  if (fl_value_get_type(args) != FL_VALUE_TYPE_MAP) {
    return nullptr;
  }
  FlValue* found = fl_value_lookup_string(args, key);
  return found != nullptr && fl_value_get_type(found) == type ? found
                                                              : nullptr;
}

static gboolean flag(FlValue* args, const gchar* key) {
  FlValue* found = argument(args, key, FL_VALUE_TYPE_BOOL);
  return found != nullptr && fl_value_get_bool(found);
}

// A rectangle Dart sent at |key| as [x, y, width, height], into |rect|. False
// when there is no such thing there.
static gboolean rect_argument(FlValue* args,
                              const gchar* key,
                              GdkRectangle* rect) {
  FlValue* list = argument(args, key, FL_VALUE_TYPE_LIST);
  if (list == nullptr || fl_value_get_length(list) != 4) {
    return FALSE;
  }
  gint numbers[4];
  for (size_t i = 0; i < 4; i++) {
    FlValue* number = fl_value_get_list_value(list, i);
    if (fl_value_get_type(number) != FL_VALUE_TYPE_FLOAT) {
      return FALSE;
    }
    // Far past any screen, or not a number at all.
    const double n = fl_value_get_float(number);
    if (!(n > -1e6 && n < 1e6)) {
      return FALSE;
    }
    numbers[i] = static_cast<gint>(n);
  }
  *rect = {numbers[0], numbers[1], numbers[2], numbers[3]};
  return rect->width > 0 && rect->height > 0;
}

// A rectangle as Dart reads one: [x, y, width, height].
static FlValue* rect_value(const GdkRectangle& rect) {
  FlValue* list = fl_value_new_list();
  const gint numbers[] = {rect.x, rect.y, rect.width, rect.height};
  for (gint n : numbers) {
    fl_value_append_take(list, fl_value_new_float(n));
  }
  return list;
}

// The window a call is about: the floating window it names by its GtkWindow's
// address, or the main window when it names none. Null when the one it names
// has gone, so only a window GTK still lists is ever touched.
static GtkWindow* named_window(MyApplication* self, FlValue* args) {
  if (fl_value_get_type(args) != FL_VALUE_TYPE_MAP ||
      fl_value_lookup_string(args, "window") == nullptr) {
    return self->window;
  }
  FlValue* named = argument(args, "window", FL_VALUE_TYPE_INT);
  if (named == nullptr) {
    return nullptr;
  }
  g_autoptr(GList) windows = gtk_window_list_toplevels();
  for (GList* window = windows; window != nullptr; window = window->next) {
    if (reinterpret_cast<intptr_t>(window->data) == fl_value_get_int(named) &&
        is_floating(self, window->data)) {
      return GTK_WINDOW(window->data);
    }
  }
  return nullptr;
}

// Where |window| is, as lib/shell/pop_out.dart's NativeFrame reads it, in
// GDK's own units. Wayland tells no window where it is, so there it is at 0, 0.
static FlValue* frame_value(GtkWindow* window, FlView* view) {
  GtkWidget* widget = GTK_WIDGET(window);
  GdkWindow* surface = gtk_widget_get_window(widget);
  const gint state = surface != nullptr ? gdk_window_get_state(surface) : 0;
  GdkRectangle bounds = {};
  gtk_window_get_position(window, &bounds.x, &bounds.y);
  gtk_window_get_size(window, &bounds.width, &bounds.height);
  auto* normal =
      static_cast<GdkRectangle*>(g_object_get_data(G_OBJECT(window), kNormal));
  if (normal != nullptr && normal->width > 0 && normal->height > 0 &&
      (state & (GDK_WINDOW_STATE_MAXIMIZED | GDK_WINDOW_STATE_FULLSCREEN |
                GDK_WINDOW_STATE_TILED | GDK_WINDOW_STATE_ICONIFIED))) {
    bounds = *normal;
  }
  // Where Flutter draws: the view, inside whatever GTK draws around it.
  gint x = 0, y = 0, left = 0, top = 0;
  gtk_widget_translate_coordinates(GTK_WIDGET(view), widget, 0, 0, &x, &y);
  if (surface != nullptr) {
    gdk_window_get_origin(surface, &left, &top);
  }
  const GdkRectangle client = {
      left + x, top + y, gtk_widget_get_allocated_width(GTK_WIDGET(view)),
      gtk_widget_get_allocated_height(GTK_WIDGET(view))};
  FlValue* frame = fl_value_new_map();
  fl_value_set_string_take(frame, "bounds", rect_value(bounds));
  fl_value_set_string_take(frame, "maximised",
                           fl_value_new_bool(gtk_window_is_maximized(window)));
  fl_value_set_string_take(
      frame, "minimised",
      fl_value_new_bool((state & GDK_WINDOW_STATE_ICONIFIED) != 0));
  fl_value_set_string_take(
      frame, "fullScreen",
      fl_value_new_bool((state & GDK_WINDOW_STATE_FULLSCREEN) != 0));
  fl_value_set_string_take(frame, "client", rect_value(client));
  fl_value_set_string_take(frame, "scale", fl_value_new_float(1.0));
  // Only X11 says where a window is. Told, so Dart never takes two windows
  // both answering the corner of the screen as one over the other.
  fl_value_set_string_take(
      frame, "placed",
      fl_value_new_bool(GDK_IS_X11_DISPLAY(gtk_widget_get_display(widget))));
  GdkMonitor* monitor =
      surface != nullptr ? gdk_display_get_monitor_at_window(
                               gtk_widget_get_display(widget), surface)
                         : nullptr;
  if (monitor != nullptr) {
    GdkRectangle display, work;
    gdk_monitor_get_geometry(monitor, &display);
    gdk_monitor_get_workarea(monitor, &work);
    fl_value_set_string_take(frame, "display", rect_value(display));
    fl_value_set_string_take(frame, "work", rect_value(work));
  }
  return frame;
}

// Every display, as lib/shell/pop_out.dart's NativeDisplay reads it.
static FlValue* displays_value(GdkDisplay* display) {
  // Wayland names no monitor primary, and Dart wants one, so it is the first.
  GdkMonitor* primary = gdk_display_get_primary_monitor(display);
  if (primary == nullptr) {
    primary = gdk_display_get_monitor(display, 0);
  }
  FlValue* displays = fl_value_new_list();
  for (gint i = 0; i < gdk_display_get_n_monitors(display); i++) {
    GdkMonitor* monitor = gdk_display_get_monitor(display, i);
    GdkRectangle bounds, work;
    gdk_monitor_get_geometry(monitor, &bounds);
    gdk_monitor_get_workarea(monitor, &work);
    // The monitor's own name, such as "DELL U2720Q", where it gives one.
    const gchar* maker = gdk_monitor_get_manufacturer(monitor);
    const gchar* model = gdk_monitor_get_model(monitor);
    g_autofree gchar* name =
        g_strdup_printf("%s %s", maker != nullptr ? maker : "",
                        model != nullptr ? model : "");
    FlValue* one = fl_value_new_map();
    fl_value_set_string_take(one, "name",
                             fl_value_new_string(g_strstrip(name)));
    fl_value_set_string_take(one, "bounds", rect_value(bounds));
    fl_value_set_string_take(one, "work", rect_value(work));
    fl_value_set_string_take(one, "scale", fl_value_new_float(1.0));
    fl_value_set_string_take(one, "primary",
                             fl_value_new_bool(monitor == primary));
    fl_value_append_take(displays, one);
  }
  return displays;
}

// The window has no title bar, so Flutter draws the buttons one would hold
// and says here what was pressed (lib/shell/window_frame.dart). Every call
// answers whether the window is maximised, which is all the button's drawing
// needs to know. The answer goes first, since closing takes the engine with
// it.
//
// The floating windows' title lines say so here too, naming their window, and
// the floating windows are placed from here (lib/shell/pop_out.dart).
static void window_method_cb(FlMethodChannel* channel,
                             FlMethodCall* call,
                             gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  const gchar* asked = fl_method_call_get_name(call);
  FlValue* args = fl_method_call_get_args(call);
  // The splash's three have answers of their own.
  if (g_strcmp0(asked, "splashScale") == 0) {
    g_autoptr(FlValue) scale =
        self->start_hidden ? fl_value_new_null()
                           : fl_value_new_float(splash_scale());
    fl_method_call_respond_success(call, scale, nullptr);
    return;
  }
  if (g_strcmp0(asked, "splash") == 0) {
    g_autoptr(FlValue) shown = fl_value_new_bool(show_splash(self, args));
    fl_method_call_respond_success(call, shown, nullptr);
    return;
  }
  if (g_strcmp0(asked, "splashClose") == 0) {
    fl_method_call_respond_success(call, nullptr, nullptr);
    close_splash(self);
    return;
  }
  // The style's see-through window (backdrop.cc).
  if (g_strcmp0(asked, "backdrop") == 0) {
    FlValue* kind = argument(args, "kind", FL_VALUE_TYPE_STRING);
    g_autoptr(FlValue) showing = fl_value_new_bool(
        backdrop_set(self->window, self->content, self->view,
                     kind != nullptr ? fl_value_get_string(kind) : ""));
    fl_method_call_respond_success(call, showing, nullptr);
    return;
  }
  // The outline of the window a dragged panel would make, at "rect", or
  // gone without one (lib/shell/pop_out.dart). Wayland tells no window where
  // it is, so there is none there.
  if (g_strcmp0(asked, "ghost") == 0) {
    fl_method_call_respond_success(call, nullptr, nullptr);
#ifdef GDK_WINDOWING_X11
    const gboolean placeable = GDK_IS_X11_DISPLAY(gdk_display_get_default());
#else
    const gboolean placeable = FALSE;
#endif
    GdkRectangle rect;
    if (!placeable || !rect_argument(args, "rect", &rect)) {
      if (self->ghost != nullptr) {
        gtk_widget_hide(self->ghost);
      }
      return;
    }
    if (self->ghost == nullptr) {
      GtkWidget* ghost = gtk_window_new(GTK_WINDOW_POPUP);
      GdkVisual* visual =
          gdk_screen_get_rgba_visual(gtk_widget_get_screen(ghost));
      if (visual != nullptr) {
        gtk_widget_set_visual(ghost, visual);
      }
      gtk_widget_set_app_paintable(ghost, TRUE);
      gtk_window_set_accept_focus(GTK_WINDOW(ghost), FALSE);
      gtk_window_set_keep_above(GTK_WINDOW(ghost), TRUE);
      gtk_window_set_skip_taskbar_hint(GTK_WINDOW(ghost), TRUE);
      gtk_window_set_skip_pager_hint(GTK_WINDOW(ghost), TRUE);
      g_signal_connect(ghost, "draw", G_CALLBACK(ghost_draw_cb), self);
      // The pointer goes through it to whatever is underneath.
      gtk_widget_realize(ghost);
      cairo_region_t* nowhere = cairo_region_create();
      gtk_widget_input_shape_combine_region(ghost, nowhere);
      cairo_region_destroy(nowhere);
      self->ghost = ghost;
    }
    FlValue* colour = argument(args, "colour", FL_VALUE_TYPE_INT);
    if (colour != nullptr &&
        static_cast<guint32>(fl_value_get_int(colour)) != self->ghost_colour) {
      self->ghost_colour = static_cast<guint32>(fl_value_get_int(colour));
      gtk_widget_queue_draw(self->ghost);
    }
    gtk_window_move(GTK_WINDOW(self->ghost), rect.x, rect.y);
    gtk_window_resize(GTK_WINDOW(self->ghost), rect.width, rect.height);
    // Without a compositor nothing shows through it, so it is cut down to
    // its frame and never covers the desktop.
    if (!gdk_screen_is_composited(gtk_widget_get_screen(self->ghost)) &&
        (rect.width != self->ghost_width ||
         rect.height != self->ghost_height)) {
      const cairo_rectangle_int_t outer = {0, 0, rect.width, rect.height};
      const cairo_rectangle_int_t inner = {2, 2, rect.width - 4,
                                           rect.height - 4};
      cairo_region_t* frame = cairo_region_create_rectangle(&outer);
      if (inner.width > 0 && inner.height > 0) {
        cairo_region_subtract_rectangle(frame, &inner);
      }
      gtk_widget_shape_combine_region(self->ghost, frame);
      cairo_region_destroy(frame);
      self->ghost_width = rect.width;
      self->ghost_height = rect.height;
    }
    gtk_widget_show_all(self->ghost);
    return;
  }
  GtkWindow* window = named_window(self, args);
  // A floating window that has gone already: nothing to do to it.
  if (window == nullptr) {
    g_autoptr(FlValue) gone = fl_value_new_bool(FALSE);
    fl_method_call_respond_success(call, gone, nullptr);
    return;
  }
  const gboolean floating = window != self->window;
  FlView* view =
      floating ? FL_VIEW(gtk_bin_get_child(GTK_BIN(window))) : self->view;
  if (g_strcmp0(asked, "frame") == 0) {
    g_autoptr(FlValue) frame = frame_value(window, view);
    fl_method_call_respond_success(call, frame, nullptr);
    return;
  }
  if (g_strcmp0(asked, "displays") == 0) {
    g_autoptr(FlValue) displays =
        displays_value(gtk_widget_get_display(GTK_WIDGET(window)));
    fl_method_call_respond_success(call, displays, nullptr);
    return;
  }
  // The four that place a floating window answer nothing, and do nothing to
  // the main window.
  if (g_strcmp0(asked, "adopt") == 0) {
    fl_method_call_respond_success(call, nullptr, nullptr);
    if (floating) {
      // Above the main window and minimised with it, with no taskbar entry of
      // its own. Where Lumit draws the title line the system's goes, and the
      // window is resized from edges Lumit draws (beginResize).
      gtk_window_set_transient_for(window, self->window);
      gtk_window_set_skip_taskbar_hint(window, TRUE);
      gtk_window_set_keep_above(window, flag(args, "onTop"));
      gtk_window_set_decorated(window, !flag(args, "title"));
      // One reopened at start leaves the keyboard with the main window. It
      // is first shown on its first frame, after this.
      gtk_window_set_focus_on_map(window, !flag(args, "quiet"));
      follow_normal_bounds(window);
    }
    return;
  }
  if (g_strcmp0(asked, "setFrame") == 0) {
    fl_method_call_respond_success(call, nullptr, nullptr);
    GdkRectangle bounds;
    if (floating && rect_argument(args, "bounds", &bounds)) {
      // Never presented, so the keyboard stays where it is. Wayland keeps
      // where windows go to itself and takes only the size.
      const gboolean maximised = flag(args, "maximised");
      if (!maximised) {
        gtk_window_unmaximize(window);
      }
      gtk_window_move(window, bounds.x, bounds.y);
      gtk_window_resize(window, bounds.width, bounds.height);
      // What it comes back to from maximised, which it may never have been
      // shown at.
      auto* normal = static_cast<GdkRectangle*>(
          g_object_get_data(G_OBJECT(window), kNormal));
      if (normal != nullptr) {
        *normal = bounds;
      }
      if (maximised) {
        gtk_window_maximize(window);
      }
    }
    return;
  }
  if (g_strcmp0(asked, "onTop") == 0) {
    fl_method_call_respond_success(call, nullptr, nullptr);
    if (floating) {
      gtk_window_set_keep_above(window, flag(args, "on"));
    }
    return;
  }
  if (g_strcmp0(asked, "monitor") == 0) {
    fl_method_call_respond_success(call, nullptr, nullptr);
    GdkRectangle display;
    if (floating && rect_argument(args, "display", &display)) {
      // A video monitor: no frame, full screen on that display, or the one
      // nearest it, and never taking the keyboard.
      GdkDisplay* gdk_display = gtk_widget_get_display(GTK_WIDGET(window));
      GdkMonitor* chosen = gdk_display_get_monitor_at_point(
          gdk_display, display.x + display.width / 2,
          display.y + display.height / 2);
      gint index = 0;
      for (gint i = 0; i < gdk_display_get_n_monitors(gdk_display); i++) {
        if (gdk_display_get_monitor(gdk_display, i) == chosen) {
          index = i;
        }
      }
      gtk_window_set_decorated(window, FALSE);
      gtk_window_set_accept_focus(window, FALSE);
      gtk_window_set_focus_on_map(window, FALSE);
      gtk_window_set_keep_above(window, TRUE);
      gtk_window_fullscreen_on_monitor(window, gtk_window_get_screen(window),
                                       index);
      // It may have had the keyboard already, so it goes back.
      if (gtk_window_is_active(window)) {
        gtk_window_present(self->window);
      }
    }
    return;
  }
  const gboolean maximised = gtk_window_is_maximized(window);
  g_autoptr(FlValue) answer = fl_value_new_bool(maximised);
  fl_method_call_respond_success(call, answer, nullptr);

  if (g_strcmp0(asked, "drag") == 0) {
    start_window_drag(view, window, -1);
  } else if (g_strcmp0(asked, "beginResize") == 0) {
    // A floating window without the system's frame, resized from an edge
    // Lumit draws (lib/shell/window_frame.dart). In GdkWindowEdge's order.
    static const gchar* const edges[] = {"nw", "n",  "ne", "w",
                                         "e",  "sw", "s",  "se"};
    FlValue* edge = argument(args, "edge", FL_VALUE_TYPE_STRING);
    for (guint i = 0; edge != nullptr && i < G_N_ELEMENTS(edges); i++) {
      if (g_strcmp0(fl_value_get_string(edge), edges[i]) == 0) {
        start_window_drag(view, window, static_cast<gint>(i));
      }
    }
  } else if (g_strcmp0(asked, "minimise") == 0) {
    gtk_window_iconify(window);
  } else if (g_strcmp0(asked, "maximise") == 0) {
    if (maximised) {
      gtk_window_unmaximize(window);
    } else {
      gtk_window_maximize(window);
    }
  } else if (g_strcmp0(asked, "close") == 0) {
    gtk_window_close(window);
  } else if (g_strcmp0(asked, "background") == 0) {
    // Settings: whether closing the window keeps Lumit running in the tray
    // (lib/state/background.dart), and the tray menu's words.
    auto text = [args](const gchar* key) {
      FlValue* found = argument(args, key, FL_VALUE_TYPE_STRING);
      return found != nullptr ? fl_value_get_string(found) : "";
    };
    self->keep_running = flag(args, "keep");
    g_autofree gchar* marker = keep_running_file();
    if (self->keep_running) {
      g_autofree gchar* folder = g_path_get_dirname(marker);
      g_mkdir_with_parents(folder, 0700);
      g_file_set_contents(marker, "", 0, nullptr);
    } else {
      g_remove(marker);
    }
    show_tray(self, self->keep_running, text("open"), text("quit"));
    set_open_at_sign_in(self->keep_running && flag(args, "signIn"));
    // Started at sign-in with nothing now to come back from: the window is
    // the only way back, so it shows.
    if (!self->keep_running && self->start_hidden) {
      reveal(self);
    }
  } else if (g_strcmp0(asked, "show") == 0) {
    reveal(self);
  } else if (g_strcmp0(asked, "notify") == 0) {
    // An export ended while Lumit wasn't in front (lib/state/background.dart).
    // Inside a Flatpak it goes through the notification portal by itself. A
    // click runs app.reveal.
    auto text = [args](const gchar* key) -> const gchar* {
      FlValue* found = argument(args, key, FL_VALUE_TYPE_STRING);
      return found != nullptr ? fl_value_get_string(found) : "";
    };
    g_autoptr(GNotification) notification = g_notification_new(text("title"));
    if (text("body")[0] != '\0') {
      g_notification_set_body(notification, text("body"));
    }
    g_notification_set_default_action(notification, "app.reveal");
    g_application_send_notification(G_APPLICATION(self), nullptr,
                                    notification);
  }
}

// A display came, went or changed. Dart is told once things have been still
// for half a second, as a monitor plugged in comes in several steps.
static gboolean displays_settled_cb(gpointer user_data) {
  MyApplication* self = MY_APPLICATION(user_data);
  self->displays_source = 0;
  fl_method_channel_invoke_method(self->window_channel, "displaysChanged",
                                  nullptr, nullptr, nullptr, nullptr);
  return G_SOURCE_REMOVE;
}

static void displays_changed_cb(MyApplication* self) {
  if (self->displays_source != 0) {
    g_source_remove(self->displays_source);
  }
  self->displays_source = g_timeout_add(500, displays_settled_cb, self);
}

// The compositor started or stopped, so a see-through window may have to go
// solid, or may come back.
static void composited_cb(MyApplication* self) {
  if (self->window_channel != nullptr) {
    fl_method_channel_invoke_method(self->window_channel, "backdrop", nullptr,
                                    nullptr, nullptr, nullptr);
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
  gtk_window_set_title(window, "Lumit");

  // Lumit opens as it was when it last closed (save_window), or maximised the
  // first time, as it does on Windows (windows/runner/win32_window.cpp) and
  // macOS. The maximising happens in `first_frame_cb`, once there is a window
  // on screen to maximise. A place on a monitor that has gone since is left
  // to the window manager, and so is every place on Wayland, which keeps
  // where windows go to itself.
  g_autoptr(GKeyFile) memory = g_key_file_new();
  g_autofree gchar* memory_file = window_file();
  self->start_maximised = TRUE;
  if (g_key_file_load_from_file(memory, memory_file, G_KEY_FILE_NONE,
                                nullptr)) {
    self->start_maximised =
        g_key_file_get_boolean(memory, "window", "maximised", nullptr);
  }
  const gint width = g_key_file_get_integer(memory, "window", "width", nullptr);
  const gint height =
      g_key_file_get_integer(memory, "window", "height", nullptr);
  const gboolean sized = width > 0 && height > 0;
  gtk_window_set_default_size(window, sized ? width : 1280,
                              sized ? height : 720);
#ifdef GDK_WINDOWING_X11
  GdkDisplay* display = gtk_widget_get_display(GTK_WIDGET(window));
  const gint x = g_key_file_get_integer(memory, "window", "x", nullptr);
  const gint y = g_key_file_get_integer(memory, "window", "y", nullptr);
  // The middle of its top edge, which is where it is taken hold of.
  const gint middle = x + width / 2;
  for (gint i = 0; sized && GDK_IS_X11_DISPLAY(display) &&
                   i < gdk_display_get_n_monitors(display);
       i++) {
    GdkRectangle monitor;
    gdk_monitor_get_geometry(gdk_display_get_monitor(display, i), &monitor);
    if (middle >= monitor.x && middle < monitor.x + monitor.width &&
        y >= monitor.y && y < monitor.y + monitor.height) {
      gtk_window_move(window, x, y);
      break;
    }
  }
#endif
  follow_normal_bounds(window);
  g_signal_connect_swapped(window, "configure-event",
                           G_CALLBACK(main_configure_cb), self);
  // The smallest it can be made, as on Windows (windows/runner/
  // win32_window.cpp): room for the default workspace's narrowest panels.
  GdkGeometry smallest = {};
  smallest.min_width = 840;
  smallest.min_height = 560;
  gtk_window_set_geometry_hints(window, nullptr, &smallest, GDK_HINT_MIN_SIZE);
  // So a style can make it see-through later. Dart asks again when the
  // compositor starts or stops.
  backdrop_prepare(window);
  g_signal_connect_swapped(gtk_widget_get_screen(GTK_WIDGET(window)),
                           "composited-changed",
                           G_CALLBACK(composited_cb), self);
  // Dart brings back the arrangement last used with the displays there are
  // now (lib/state/ui_state.dart).
  g_signal_connect_swapped(gdk_display_get_default(), "monitor-added",
                           G_CALLBACK(displays_changed_cb), self);
  g_signal_connect_swapped(gdk_display_get_default(), "monitor-removed",
                           G_CALLBACK(displays_changed_cb), self);
  g_signal_connect_swapped(gdk_screen_get_default(), "monitors-changed",
                           G_CALLBACK(displays_changed_cb), self);

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
  self->content = overlay;

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
  // ViewerTextureBridge (flutter_window.cpp OnCreate). Every window's
  // views draw from the one engine, so the bridge is registered here once,
  // exactly as on Windows. A build of the engine `.so` without the
  // `shared-texture-linux` feature simply never calls the channel (Dart keeps the
  // read-back path), so this registration is inert then.
  FlEngine* engine = fl_view_get_engine(view);
  viewer_texture_bridge_register(fl_engine_get_binary_messenger(engine),
                                 fl_engine_get_texture_registrar(engine));
  // WebKitGTK is the machine's own and only extension panels use it, so it is
  // loaded and not linked. Without it a panel says it couldn't load and the
  // rest of Lumit runs (#693). RTLD_NOW so a WebKitGTK that is too old fails
  // here and not mid-call. Never closed, since GTypes can't be unregistered.
  void* extension_views = dlopen("liblumit_extension_views.so",
                                 RTLD_NOW | RTLD_LOCAL);
  auto register_extension_views =
      extension_views == nullptr
          ? nullptr
          : reinterpret_cast<decltype(&extension_views_register)>(
                dlsym(extension_views, "extension_views_register"));
  if (register_extension_views != nullptr) {
    register_extension_views(fl_engine_get_binary_messenger(engine),
                             GTK_OVERLAY(overlay), view);
  } else {
    const char* error = dlerror();
    g_warning("Extension panels are off: %s",
              error != nullptr ? error : "unknown error");
  }

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

// Whether this start was asked to install an extension package, a style, a
// preset, a theme or a graphics template.
static gboolean asks_for_package(gchar** arguments) {
  for (gchar** argument = arguments; *argument != nullptr; argument++) {
    g_autofree gchar* lower = g_ascii_strdown(*argument, -1);
    if (g_str_has_suffix(lower, ".lfx") || g_str_has_suffix(lower, ".lumstyle") ||
        g_str_has_suffix(lower, ".lumfx") || g_str_has_suffix(lower, ".lumtheme") ||
        g_str_has_suffix(lower, ".lumt")) {
      return TRUE;
    }
  }
  return FALSE;
}

// Whether another Lumit already has Lumit's name on the session bus.
static gboolean lumit_is_open() {
  g_autoptr(GDBusConnection) bus =
      g_bus_get_sync(G_BUS_TYPE_SESSION, nullptr, nullptr);
  if (bus == nullptr) {
    return FALSE;
  }
  g_autoptr(GVariant) reply = g_dbus_connection_call_sync(
      bus, "org.freedesktop.DBus", "/org/freedesktop/DBus",
      "org.freedesktop.DBus", "NameHasOwner",
      g_variant_new("(s)", APPLICATION_ID), G_VARIANT_TYPE("(b)"),
      G_DBUS_CALL_FLAGS_NONE, -1, nullptr, nullptr);
  gboolean open = FALSE;
  if (reply != nullptr) {
    g_variant_get(reply, "(b)", &open);
  }
  return open;
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

  // A Lumit of its own still takes Lumit's name when no other Lumit has it,
  // so a start asked to install an extension package can find it. That start
  // asks for the name too, and hands the package over when it is taken.
  if ((g_application_get_flags(application) & G_APPLICATION_NON_UNIQUE) &&
      (asks_for_package(self->dart_entrypoint_arguments) || !lumit_is_open())) {
    g_application_set_flags(application, static_cast<GApplicationFlags>(0));
  }

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

  // A Lumit kept running in the background, or any open Lumit when this start
  // brings a package, has this one's name. It is the one that shows itself
  // and opens what was asked for.
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
  g_clear_handle_id(&self->save_source, g_source_remove);
  g_clear_handle_id(&self->displays_source, g_source_remove);
  g_clear_pointer(&self->ghost, gtk_widget_destroy);
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
  // What a click on one of Lumit's notifications runs.
  g_autoptr(GSimpleAction) shown = g_simple_action_new("reveal", nullptr);
  g_signal_connect_swapped(shown, "activate", G_CALLBACK(reveal), self);
  g_action_map_add_action(G_ACTION_MAP(self), G_ACTION(shown));
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
