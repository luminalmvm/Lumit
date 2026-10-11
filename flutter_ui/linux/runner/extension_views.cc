#include "extension_views.h"

#include <webkit2/webkit2.h>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <map>
#include <string>
#include <vector>

namespace {

// The scheme an extension's own files are served under. The Dart side
// (extension_host.dart, extensionScheme) names the same one.
const char kScheme[] = "lumit-extension";

// The most pixels a picture of a page is sent with. A panel is far smaller.
const int64_t kPictureLimit = 64 * 1000 * 1000;

struct Views;

// One extension's page.
struct Page {
  Views* views = nullptr;
  int64_t number = 0;
  // What its own pages start with: lumit-extension://<id>/
  std::string origin;
  WebKitWebView* web = nullptr;
  // Where the panel is, in the window.
  GdkRectangle where = {0, 0, 1, 1};
};

struct Views {
  FlMethodChannel* channel = nullptr;
  GtkOverlay* overlay = nullptr;
  GtkWidget* flutter = nullptr;
  // Made with the first page, since it wants to know where to keep what
  // pages store.
  WebKitWebContext* context = nullptr;
  // Each extension's folder, by its id, for the scheme to serve from.
  std::map<std::string, std::string> folders;
  std::map<int64_t, Page*> pages;
};

// What a file is, by the end of its name, for the ones a page is made of.
// Anything else is asked of the system.
const char* known_type(const char* path) {
  static const struct {
    const char* end;
    const char* type;
  } kTypes[] = {
      {".html", "text/html"},       {".htm", "text/html"},
      {".css", "text/css"},         {".js", "text/javascript"},
      {".mjs", "text/javascript"},  {".json", "application/json"},
      {".map", "application/json"}, {".wasm", "application/wasm"},
      {".svg", "image/svg+xml"},
  };
  for (const auto& known : kTypes) {
    if (g_str_has_suffix(path, known.end)) {
      return known.type;
    }
  }
  return nullptr;
}

void refuse(WebKitURISchemeRequest* request) {
  g_autoptr(GError) error =
      g_error_new_literal(G_IO_ERROR, G_IO_ERROR_NOT_FOUND, "not found");
  webkit_uri_scheme_request_finish_error(request, error);
}

// Serve a file of an extension's to its own page.
void serve_cb(WebKitURISchemeRequest* request, gpointer user_data) {
  Views* self = static_cast<Views*>(user_data);
  g_autoptr(GUri) uri = g_uri_parse(webkit_uri_scheme_request_get_uri(request),
                                    G_URI_FLAGS_NONE, nullptr);
  const gchar* id = uri != nullptr ? g_uri_get_host(uri) : nullptr;
  const gchar* path = uri != nullptr ? g_uri_get_path(uri) : nullptr;
  if (id == nullptr || path == nullptr || strlen(path) < 2) {
    return refuse(request);
  }
  auto found = self->folders.find(id);
  if (found == self->folders.end()) {
    return refuse(request);
  }
  // Only for a page of the extension's own: one that has gone somewhere else
  // cannot read its files.
  WebKitWebView* web = webkit_uri_scheme_request_get_web_view(request);
  const gchar* at = web != nullptr ? webkit_web_view_get_uri(web) : nullptr;
  const std::string origin = std::string(kScheme) + "://" + id + "/";
  if (at == nullptr || !g_str_has_prefix(at, origin.c_str())) {
    return refuse(request);
  }
  // The file itself, with every link followed, has to be inside the folder.
  const std::string folder = found->second + "/";
  g_autofree gchar* joined =
      g_build_filename(found->second.c_str(), path + 1, nullptr);
  char* real = realpath(joined, nullptr);
  if (real == nullptr) {
    return refuse(request);
  }
  const std::string name = real;
  free(real);
  if (name.compare(0, folder.size(), folder) != 0 ||
      !g_file_test(name.c_str(), G_FILE_TEST_IS_REGULAR)) {
    return refuse(request);
  }
  g_autoptr(GFile) file = g_file_new_for_path(name.c_str());
  g_autoptr(GFileInfo) info = g_file_query_info(
      file,
      G_FILE_ATTRIBUTE_STANDARD_SIZE "," G_FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
      G_FILE_QUERY_INFO_NONE, nullptr, nullptr);
  g_autoptr(GFileInputStream) stream = g_file_read(file, nullptr, nullptr);
  if (info == nullptr || stream == nullptr) {
    return refuse(request);
  }
  g_autofree gchar* guessed = nullptr;
  const char* type = known_type(name.c_str());
  if (type == nullptr) {
    const char* content = g_file_info_get_content_type(info);
    guessed = content != nullptr ? g_content_type_get_mime_type(content)
                                 : nullptr;
    type = guessed != nullptr ? guessed : "application/octet-stream";
  }
  webkit_uri_scheme_request_finish(request, G_INPUT_STREAM(stream),
                                   g_file_info_get_size(info), type);
}

// One context for every page: the scheme, and where what pages store is
// kept. Each page still has its own storage, since each is served under its
// own name.
WebKitWebContext* make_context(Views* self, const char* data) {
  g_autofree gchar* cache = g_build_filename(data, "cache", nullptr);
  g_autoptr(WebKitWebsiteDataManager) manager =
      webkit_website_data_manager_new("base-data-directory", data,
                                      "base-cache-directory", cache, nullptr);
  WebKitWebContext* context =
      webkit_web_context_new_with_website_data_manager(manager);
  webkit_web_context_register_uri_scheme(context, kScheme, serve_cb, self,
                                         nullptr);
  // Served by the application, so as good as https: a page can use what the
  // web keeps for pages it trusts, and fetch its own files.
  WebKitSecurityManager* security =
      webkit_web_context_get_security_manager(context);
  webkit_security_manager_register_uri_scheme_as_secure(security, kScheme);
  webkit_security_manager_register_uri_scheme_as_cors_enabled(security,
                                                              kScheme);
  return context;
}

FlValue* about(Page* page) {
  FlValue* args = fl_value_new_map();
  fl_value_set_string_take(args, "view", fl_value_new_int(page->number));
  return args;
}

// A message from the page. WebKit gives the same handler to every frame and
// does not say which one spoke, so the Dart side checks the key each message
// carries. A page that has gone somewhere else is not listened to at all.
void message_cb(WebKitUserContentManager* manager,
                WebKitJavascriptResult* result,
                gpointer user_data) {
  Page* page = static_cast<Page*>(user_data);
  const gchar* at = webkit_web_view_get_uri(page->web);
  JSCValue* value = webkit_javascript_result_get_js_value(result);
  if (at == nullptr || !g_str_has_prefix(at, page->origin.c_str()) ||
      !jsc_value_is_string(value)) {
    return;
  }
  g_autofree gchar* text = jsc_value_to_string(value);
  g_autoptr(FlValue) args = about(page);
  fl_value_set_string_take(args, "text", fl_value_new_string(text));
  fl_method_channel_invoke_method(page->views->channel, "message", args,
                                  nullptr, nullptr, nullptr);
}

void uri_cb(WebKitWebView* web, GParamSpec* spec, gpointer user_data) {
  Page* page = static_cast<Page*>(user_data);
  const gchar* at = webkit_web_view_get_uri(web);
  if (at == nullptr) {
    return;
  }
  g_autoptr(FlValue) args = about(page);
  fl_value_set_string_take(args, "url", fl_value_new_string(at));
  fl_method_channel_invoke_method(page->views->channel, "url", args, nullptr,
                                  nullptr, nullptr);
}

// The overlay asks where each thing laid over it goes.
gboolean position_cb(GtkOverlay* overlay,
                     GtkWidget* widget,
                     GdkRectangle* allocation,
                     gpointer user_data) {
  Views* self = static_cast<Views*>(user_data);
  for (const auto& entry : self->pages) {
    if (GTK_WIDGET(entry.second->web) == widget) {
      *allocation = entry.second->where;
      return TRUE;
    }
  }
  return FALSE;
}

const char* text_of(FlValue* args, const char* key) {
  FlValue* value = fl_value_lookup_string(args, key);
  return value != nullptr && fl_value_get_type(value) == FL_VALUE_TYPE_STRING
             ? fl_value_get_string(value)
             : nullptr;
}

double number_of(FlValue* args, const char* key) {
  FlValue* value = fl_value_lookup_string(args, key);
  if (value != nullptr && fl_value_get_type(value) == FL_VALUE_TYPE_FLOAT) {
    return fl_value_get_float(value);
  }
  if (value != nullptr && fl_value_get_type(value) == FL_VALUE_TYPE_INT) {
    return static_cast<double>(fl_value_get_int(value));
  }
  return 0;
}

bool flag_of(FlValue* args, const char* key) {
  FlValue* value = fl_value_lookup_string(args, key);
  return value != nullptr && fl_value_get_type(value) == FL_VALUE_TYPE_BOOL &&
         fl_value_get_bool(value);
}

// The page `args` names, if it is one that was made.
Page* page_of(Views* self, FlValue* args) {
  FlValue* view = fl_value_lookup_string(args, "view");
  if (view == nullptr || fl_value_get_type(view) != FL_VALUE_TYPE_INT) {
    return nullptr;
  }
  auto found = self->pages.find(fl_value_get_int(view));
  return found == self->pages.end() ? nullptr : found->second;
}

bool create(Views* self, FlValue* args) {
  FlValue* view = fl_value_lookup_string(args, "view");
  const char* id = text_of(args, "id");
  const char* folder = text_of(args, "folder");
  const char* data = text_of(args, "data");
  const char* bootstrap = text_of(args, "bootstrap");
  if (view == nullptr || fl_value_get_type(view) != FL_VALUE_TYPE_INT ||
      id == nullptr || folder == nullptr || data == nullptr ||
      bootstrap == nullptr ||
      self->pages.count(fl_value_get_int(view)) != 0) {
    return false;
  }
  char* real = realpath(folder, nullptr);
  if (real == nullptr) {
    return false;
  }
  self->folders[id] = real;
  free(real);
  if (self->context == nullptr) {
    self->context = make_context(self, data);
  }

  Page* page = new Page();
  page->views = self;
  page->number = fl_value_get_int(view);
  page->origin = std::string(kScheme) + "://" + id + "/";

  // `window.lumit`, before any script of the page's own, and for the top
  // frame alone.
  g_autoptr(WebKitUserContentManager) manager =
      webkit_user_content_manager_new();
  WebKitUserScript* script = webkit_user_script_new(
      bootstrap, WEBKIT_USER_CONTENT_INJECT_TOP_FRAME,
      WEBKIT_USER_SCRIPT_INJECT_AT_DOCUMENT_START, nullptr, nullptr);
  webkit_user_content_manager_add_script(manager, script);
  webkit_user_script_unref(script);
  g_signal_connect(manager, "script-message-received::lumit",
                   G_CALLBACK(message_cb), page);
  webkit_user_content_manager_register_script_message_handler(manager,
                                                              "lumit");

  page->web = WEBKIT_WEB_VIEW(
      g_object_new(WEBKIT_TYPE_WEB_VIEW, "web-context", self->context,
                   "user-content-manager", manager, nullptr));
  WebKitSettings* settings = webkit_web_view_get_settings(page->web);
  // Drawn without the graphics card. A panel's page is small, and WebKit's
  // accelerated path draws nothing at all on some drivers.
  webkit_settings_set_hardware_acceleration_policy(
      settings, WEBKIT_HARDWARE_ACCELERATION_POLICY_NEVER);
  webkit_settings_set_enable_developer_extras(settings,
                                              flag_of(args, "inspect"));
  g_signal_connect(page->web, "notify::uri", G_CALLBACK(uri_cb), page);

  // Out of sight until the Dart side has said where the panel is.
  gtk_widget_set_no_show_all(GTK_WIDGET(page->web), TRUE);
  self->pages[page->number] = page;
  gtk_overlay_add_overlay(self->overlay, GTK_WIDGET(page->web));
  return true;
}

void place(Page* page, FlValue* args) {
  page->where.x = static_cast<int>(std::lround(number_of(args, "x")));
  page->where.y = static_cast<int>(std::lround(number_of(args, "y")));
  page->where.width =
      std::max(1, static_cast<int>(std::lround(number_of(args, "width"))));
  page->where.height =
      std::max(1, static_cast<int>(std::lround(number_of(args, "height"))));
  // The interface is drawn at a scale of the person's choosing, and the page
  // is drawn at the same one.
  const double scale = number_of(args, "scale");
  if (scale > 0 &&
      std::fabs(webkit_web_view_get_zoom_level(page->web) - scale) > 0.001) {
    webkit_web_view_set_zoom_level(page->web, scale);
  }
  GtkWidget* widget = GTK_WIDGET(page->web);
  gtk_widget_set_visible(widget, flag_of(args, "shown"));
  gtk_widget_queue_resize(widget);
}

void run(Page* page, const char* script) {
#if WEBKIT_CHECK_VERSION(2, 40, 0)
  webkit_web_view_evaluate_javascript(page->web, script, -1, nullptr, nullptr,
                                      nullptr, nullptr, nullptr);
#else
  webkit_web_view_run_javascript(page->web, script, nullptr, nullptr, nullptr);
#endif
}

// A picture of the page has been taken: answer the call that asked with its
// pixels, as RGBA.
void picture_cb(GObject* source, GAsyncResult* result, gpointer user_data) {
  g_autoptr(FlMethodCall) call = FL_METHOD_CALL(user_data);
  WebKitWebView* web = WEBKIT_WEB_VIEW(source);
  cairo_surface_t* taken =
      webkit_web_view_get_snapshot_finish(web, result, nullptr);
  if (taken == nullptr) {
    fl_method_call_respond_success(call, nullptr, nullptr);
    return;
  }
  // Whatever kind of surface it came as, read it as plain pixels of the
  // size it is on screen.
  const int factor = gtk_widget_get_scale_factor(GTK_WIDGET(web));
  int width = gtk_widget_get_allocated_width(GTK_WIDGET(web)) * factor;
  int height = gtk_widget_get_allocated_height(GTK_WIDGET(web)) * factor;
  if (cairo_surface_get_type(taken) == CAIRO_SURFACE_TYPE_IMAGE) {
    width = cairo_image_surface_get_width(taken);
    height = cairo_image_surface_get_height(taken);
  }
  if (width < 1 || height < 1 ||
      static_cast<int64_t>(width) * height > kPictureLimit) {
    cairo_surface_destroy(taken);
    fl_method_call_respond_success(call, nullptr, nullptr);
    return;
  }
  cairo_surface_t* image =
      cairo_image_surface_create(CAIRO_FORMAT_ARGB32, width, height);
  double scale_x = 1;
  double scale_y = 1;
  cairo_surface_get_device_scale(taken, &scale_x, &scale_y);
  cairo_surface_set_device_scale(image, scale_x, scale_y);
  cairo_t* painter = cairo_create(image);
  cairo_set_source_surface(painter, taken, 0, 0);
  cairo_paint(painter);
  cairo_destroy(painter);
  cairo_surface_destroy(taken);
  cairo_surface_flush(image);

  // Cairo keeps a pixel as one number, alpha at the top. Flutter reads four
  // bytes, red first.
  const unsigned char* rows = cairo_image_surface_get_data(image);
  const int stride = cairo_image_surface_get_stride(image);
  std::vector<uint8_t> pixels(static_cast<size_t>(width) * height * 4);
  for (int y = 0; y < height; y++) {
    const unsigned char* row = rows + static_cast<size_t>(y) * stride;
    uint8_t* out = pixels.data() + static_cast<size_t>(y) * width * 4;
    for (int x = 0; x < width; x++) {
      uint32_t pixel;
      memcpy(&pixel, row + x * 4, sizeof(pixel));
      out[x * 4 + 0] = (pixel >> 16) & 0xff;
      out[x * 4 + 1] = (pixel >> 8) & 0xff;
      out[x * 4 + 2] = pixel & 0xff;
      out[x * 4 + 3] = (pixel >> 24) & 0xff;
    }
  }
  cairo_surface_destroy(image);

  g_autoptr(FlValue) answer = fl_value_new_map();
  fl_value_set_string_take(answer, "width", fl_value_new_int(width));
  fl_value_set_string_take(answer, "height", fl_value_new_int(height));
  fl_value_set_string_take(
      answer, "pixels", fl_value_new_uint8_list(pixels.data(), pixels.size()));
  fl_method_call_respond_success(call, answer, nullptr);
}

void dispose(Views* self, Page* page) {
  g_signal_handlers_disconnect_by_data(page->web, page);
  g_signal_handlers_disconnect_by_data(
      webkit_web_view_get_user_content_manager(page->web), page);
  self->pages.erase(page->number);
  gtk_widget_destroy(GTK_WIDGET(page->web));
  delete page;
}

void method_call_cb(FlMethodChannel* channel,
                    FlMethodCall* call,
                    gpointer user_data) {
  Views* self = static_cast<Views*>(user_data);
  const gchar* method = fl_method_call_get_name(call);
  FlValue* args = fl_method_call_get_args(call);

  if (strcmp(method, "unfocus") == 0) {
    // A page that was typed in keeps the keyboard when the press that
    // follows lands on Flutter's own view.
    if (!gtk_widget_has_focus(self->flutter)) {
      gtk_widget_grab_focus(self->flutter);
    }
    fl_method_call_respond_success(call, nullptr, nullptr);
    return;
  }
  if (args == nullptr || fl_value_get_type(args) != FL_VALUE_TYPE_MAP) {
    fl_method_call_respond_error(call, "bad_args", "a map was expected",
                                 nullptr, nullptr);
    return;
  }
  if (strcmp(method, "create") == 0) {
    if (create(self, args)) {
      fl_method_call_respond_success(call, nullptr, nullptr);
    } else {
      fl_method_call_respond_error(call, "bad_args",
                                   "the page could not be made", nullptr,
                                   nullptr);
    }
    return;
  }
  Page* page = page_of(self, args);
  if (page == nullptr) {
    // Gone already, which is not worth an error: the panel closed.
    fl_method_call_respond_success(call, nullptr, nullptr);
    return;
  }
  if (strcmp(method, "place") == 0) {
    place(page, args);
  } else if (strcmp(method, "load") == 0) {
    const char* url = text_of(args, "url");
    if (url != nullptr) {
      webkit_web_view_load_uri(page->web, url);
    }
  } else if (strcmp(method, "run") == 0) {
    const char* script = text_of(args, "script");
    if (script != nullptr) {
      run(page, script);
    }
  } else if (strcmp(method, "picture") == 0) {
    // Answered when the picture has been taken.
    webkit_web_view_get_snapshot(
        page->web, WEBKIT_SNAPSHOT_REGION_VISIBLE,
        WEBKIT_SNAPSHOT_OPTIONS_NONE, nullptr, picture_cb, g_object_ref(call));
    return;
  } else if (strcmp(method, "dispose") == 0) {
    dispose(self, page);
  } else {
    fl_method_call_respond_not_implemented(call, nullptr);
    return;
  }
  fl_method_call_respond_success(call, nullptr, nullptr);
}

}  // namespace

void extension_views_register(FlBinaryMessenger* messenger,
                              GtkOverlay* overlay,
                              FlView* view) {
  // Lives as long as the window, which is as long as the application.
  Views* self = new Views();
  self->overlay = overlay;
  self->flutter = GTK_WIDGET(view);
  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  self->channel = fl_method_channel_new(messenger, "lumit/extension_views",
                                        FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(self->channel, method_call_cb,
                                            self, nullptr);
  g_signal_connect(overlay, "get-child-position", G_CALLBACK(position_cb),
                   self);
}
