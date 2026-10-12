#include "viewer_texture_bridge.h"

#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <GLES2/gl2ext.h>
#include <sys/stat.h>
#include <sys/sysmacros.h>
#include <unistd.h>

#include <atomic>
#include <cerrno>
#include <cstdint>
#include <cstring>
#include <mutex>
#include <string>

#ifdef GDK_WINDOWING_WAYLAND
#include <gdk/gdkwayland.h>
#endif
#ifdef GDK_WINDOWING_X11
#include <gdk/gdkx.h>
#endif

#include "gpu_device_match.h"

// ---------------------------------------------------------------------------
// Which graphics card Flutter is drawing with.
//
// In plain terms: on a laptop with two graphics cards the engine draws the
// Viewer's picture on one, and Flutter may be drawing the window with the
// other. The picture is handed over as a DMA-BUF, which is memory on one card,
// and a driver given memory from another card doesn't always say no. Mesa's
// Intel driver accepted an NVIDIA buffer and then aborted the whole process on
// the first draw. So before anything is imported, the runner finds out which
// card is behind Flutter's EGL display and compares it with the card the engine
// says the buffer is on. gpu_device_match.h holds the comparison, this is the
// part that has to ask EGL.
//
// EGL names the card by the path of its device node (EGL_EXT_device_query for
// the display's device, then EGL_EXT_device_drm_render_node and
// EGL_EXT_device_drm for the two files). A stat() of that path gives the
// numbers the kernel knows the card by, which are what Vulkan reported on the
// engine's side. A driver with none of these leaves the card unknown, and
// unknown is never a reason to refuse.
// ---------------------------------------------------------------------------

#ifndef EGL_DEVICE_EXT
#define EGL_DEVICE_EXT 0x322C
#endif
#ifndef EGL_DRM_DEVICE_FILE_EXT
#define EGL_DRM_DEVICE_FILE_EXT 0x3233
#endif
#ifndef EGL_DRM_RENDER_NODE_FILE_EXT
#define EGL_DRM_RENDER_NODE_FILE_EXT 0x3377
#endif

// The two query functions, spelt out here so the file doesn't depend on how
// new the system's eglext.h is.
typedef EGLBoolean (*LumitQueryDisplayAttrib)(EGLDisplay display,
                                              EGLint attribute,
                                              EGLAttrib* value);
typedef const char* (*LumitQueryDeviceString)(void* device, EGLint name);

// GError codes in the "lumit" domain. Zero is any ordinary import failure. The
// mismatch has a code of its own because Dart shows a message for it.
enum { kLumitErrorImport = 0, kLumitErrorGpuMismatch = 1 };

// What the runner found out about the card behind an EGL display: its nodes,
// and one line describing them for the log.
struct LumitDisplayGpu {
  LumitGpuDevice device;
  std::string description;
};

// Whether |name| is one of the space-separated words in |list|. A plain strstr
// isn't enough, since "EGL_EXT_device_drm" is also the start of
// "EGL_EXT_device_drm_render_node".
static bool HasExtension(const char* list, const char* name) {
  if (list == nullptr) {
    return false;
  }
  const size_t length = std::strlen(name);
  for (const char* at = std::strstr(list, name); at != nullptr;
       at = std::strstr(at + length, name)) {
    const bool starts = at == list || at[-1] == ' ';
    const bool ends = at[length] == ' ' || at[length] == '\0';
    if (starts && ends) {
      return true;
    }
  }
  return false;
}

// Add the device node at |path| to |gpu|, filed by its number, and to the
// description. A path that can't be read is described and otherwise ignored.
static void AddNode(LumitDisplayGpu* gpu, const char* path) {
  if (path == nullptr || path[0] == '\0') {
    return;
  }
  if (!gpu->description.empty()) {
    gpu->description += ", ";
  }
  gpu->description += path;
  struct stat info;
  if (stat(path, &info) != 0 || !S_ISCHR(info.st_mode)) {
    gpu->description += " (not readable)";
    return;
  }
  const uint32_t dev_major = major(info.st_rdev);
  const uint32_t dev_minor = minor(info.st_rdev);
  lumit_gpu_device_add(&gpu->device, dev_major, dev_minor);
  g_autofree gchar* numbers =
      g_strdup_printf(" (%u:%u)", dev_major, dev_minor);
  gpu->description += numbers;
}

// Ask EGL which card is behind |display|. It needs an initialised display but
// no current context, so the platform thread can call it as well as populate.
static LumitDisplayGpu QueryDisplayGpu(EGLDisplay display) {
  LumitDisplayGpu gpu;
  if (display == EGL_NO_DISPLAY) {
    gpu.description = "no EGL display";
    return gpu;
  }
  static LumitQueryDisplayAttrib query_display =
      reinterpret_cast<LumitQueryDisplayAttrib>(
          eglGetProcAddress("eglQueryDisplayAttribEXT"));
  static LumitQueryDeviceString query_device_string =
      reinterpret_cast<LumitQueryDeviceString>(
          eglGetProcAddress("eglQueryDeviceStringEXT"));
  // Client extensions are asked of no display at all.
  const char* client = eglQueryString(EGL_NO_DISPLAY, EGL_EXTENSIONS);
  const bool can_query = query_display != nullptr &&
                         query_device_string != nullptr &&
                         (HasExtension(client, "EGL_EXT_device_query") ||
                          HasExtension(client, "EGL_EXT_device_base"));
  EGLAttrib device_attrib = 0;
  if (can_query && query_display(display, EGL_DEVICE_EXT, &device_attrib) &&
      device_attrib != 0) {
    void* device = reinterpret_cast<void*>(device_attrib);
    const char* extensions = query_device_string(device, EGL_EXTENSIONS);
    if (HasExtension(extensions, "EGL_EXT_device_drm_render_node")) {
      AddNode(&gpu,
              query_device_string(device, EGL_DRM_RENDER_NODE_FILE_EXT));
    }
    if (HasExtension(extensions, "EGL_EXT_device_drm")) {
      AddNode(&gpu, query_device_string(device, EGL_DRM_DEVICE_FILE_EXT));
    }
  }
  if (gpu.description.empty()) {
    gpu.description = "device not reported by EGL";
  }
  const char* vendor = eglQueryString(display, EGL_VENDOR);
  gpu.description += ", EGL vendor ";
  gpu.description += vendor != nullptr ? vendor : "unknown";
  return gpu;
}

// The last description printed, which `displayGpu` hands to Dart so the shell
// can put it in lumit-diagnostics.log. Written from the platform thread
// (register) and the raster thread (populate), so it needs the lock.
static std::mutex display_gpu_lock;
static std::string display_gpu_said;

// Print which card Flutter is on, once, and again only if the answer changes.
// Always printed, since which card each half of Lumit was given is the first
// question about a blank or crashing Viewer on a machine with two.
static void NoteDisplayGpu(const std::string& description) {
  std::lock_guard<std::mutex> hold(display_gpu_lock);
  if (description == display_gpu_said) {
    return;
  }
  display_gpu_said = description;
  g_printerr("lumit-runner: display GPU: %s\n", description.c_str());
}

static std::string DescribeNodes(const LumitGpuDevice& device) {
  std::string said;
  if (device.render.known) {
    g_autofree gchar* text = g_strdup_printf(
        "render node %u:%u", device.render.dev_major, device.render.dev_minor);
    said += text;
  }
  if (device.primary.known) {
    g_autofree gchar* text =
        g_strdup_printf("%sprimary node %u:%u", said.empty() ? "" : ", ",
                        device.primary.dev_major, device.primary.dev_minor);
    said += text;
  }
  return said.empty() ? "device not reported" : said;
}

// Which kernel driver has the card |device| names ("nvidia", "amdgpu", "i915"),
// or empty when sysfs won't say. Read from the node's own entry under
// /sys/dev/char, which a Flatpak sandbox can see.
static std::string KernelDriver(const LumitGpuDevice& device) {
  const LumitDrmNode& node = device.render.known ? device.render
                                                 : device.primary;
  if (!node.known) {
    return "";
  }
  g_autofree gchar* link = g_strdup_printf("/sys/dev/char/%u:%u/device/driver",
                                           node.dev_major, node.dev_minor);
  g_autofree gchar* target = g_file_read_link(link, nullptr);
  if (target == nullptr) {
    return "";
  }
  g_autofree gchar* name = g_path_get_basename(target);
  return name;
}

// Whether the buffer the engine made on |renderer| may be imported on
// |display|. On a refusal, |detail| names both cards.
//
// Two cards whose drivers share buffers are let through with one line saying
// so. LUMIT_ALLOW_CROSS_GPU_IMPORT lets any mismatch through with a warning,
// see lumit_gpu_refuses_import for who that is for.
static bool GpuMayImport(EGLDisplay display, const LumitGpuDevice& renderer,
                         std::string* detail) {
  const LumitDisplayGpu gpu = QueryDisplayGpu(display);
  NoteDisplayGpu(gpu.description);
  const LumitGpuMatch match = lumit_gpu_match(renderer, gpu.device);
  if (match != LumitGpuMatch::kDifferent) {
    return true;
  }
  const std::string driver = KernelDriver(renderer);
  *detail = "the render GPU (" + DescribeNodes(renderer) + ", driver " +
            (driver.empty() ? "unknown" : driver) +
            ") is not the display GPU (" + gpu.description + ")";
  const bool allow = g_getenv("LUMIT_ALLOW_CROSS_GPU_IMPORT") != nullptr;
  if (lumit_gpu_refuses_import(match, driver, allow)) {
    return false;
  }
  static std::atomic_flag said = ATOMIC_FLAG_INIT;
  if (!said.test_and_set()) {
    if (allow) {
      g_warning(
          "lumit: %s; importing the Viewer texture anyway because "
          "LUMIT_ALLOW_CROSS_GPU_IMPORT is set",
          detail->c_str());
    } else {
      g_printerr(
          "lumit-runner: %s; importing the Viewer texture across the two, "
          "which this driver supports\n",
          detail->c_str());
    }
  }
  return true;
}

// The EGL display Flutter's embedder renders with, found the way the embedder
// finds it (fl_opengl_manager.cc): the platform display for GDK's own
// connection. EGL hands back the same display for the same connection however
// often it is asked, so this is the embedder's display and not a second one.
//
// This is for the platform thread, where no context is current and
// eglGetCurrentDisplay() has nothing to say. If the embedder ever finds its
// display some other way, this returns one that was never initialised, the
// query fails, the card is unknown and nothing is refused here. The check in
// populate asks the display that is actually current, so it still stands.
static EGLDisplay FlutterEglDisplay() {
  typedef EGLDisplay (*GetPlatformDisplay)(EGLenum platform,
                                           void* native_display,
                                           const EGLint* attributes);
  static GetPlatformDisplay get_platform_display =
      reinterpret_cast<GetPlatformDisplay>(
          eglGetProcAddress("eglGetPlatformDisplayEXT"));
  GdkDisplay* display = gdk_display_get_default();
  if (get_platform_display == nullptr || display == nullptr) {
    return EGL_NO_DISPLAY;
  }
#ifdef GDK_WINDOWING_WAYLAND
  if (GDK_IS_WAYLAND_DISPLAY(display)) {
    return get_platform_display(EGL_PLATFORM_WAYLAND_EXT,
                                gdk_wayland_display_get_wl_display(display),
                                nullptr);
  }
#endif
#ifdef GDK_WINDOWING_X11
  if (GDK_IS_X11_DISPLAY(display)) {
    return get_platform_display(EGL_PLATFORM_X11_EXT,
                                gdk_x11_display_get_xdisplay(display),
                                nullptr);
  }
#endif
  return EGL_NO_DISPLAY;
}

// ---------------------------------------------------------------------------
// The DMA-BUF-backed FlTextureGL subclass.
//
// One instance per registered engine texture. It holds the DMA-BUF metadata the
// engine reported; the EGLImage + GL texture are created lazily on the first
// `populate` (that is the only moment the GL context is guaranteed current, on
// the render thread). The engine re-uses one texture across frames, so this is
// imported once and then just re-sampled on each `frameReady`.
// ---------------------------------------------------------------------------

G_DECLARE_FINAL_TYPE(LumitDmabufTexture, lumit_dmabuf_texture, LUMIT,
                     DMABUF_TEXTURE, FlTextureGL)

#ifndef GL_TEXTURE_EXTERNAL_OES
#define GL_TEXTURE_EXTERNAL_OES 0x8D65
#endif

struct _LumitDmabufTexture {
  FlTextureGL parent_instance;

  // Our own duplicate of the DMA-BUF the engine exported. The engine's Rust side
  // owns the descriptor it sent and closes it itself, so we `dup()` on
  // registration and close only our copy — exactly one owner per side, no double
  // close.
  int fd;
  uint32_t width;
  uint32_t height;
  uint32_t stride;
  uint32_t offset;
  uint32_t fourcc;
  uint64_t modifier;
  // Which card the engine says the buffer is on, compared with Flutter's own
  // before the import. Unknown when the engine's driver didn't say.
  LumitGpuDevice renderer_gpu;

  // Lazily created on the first populate (render thread, GL context current).
  gboolean created;
  gboolean failed;
  // Why the import failed, for `frameReady` to hand to Dart, since nothing
  // reaches Dart from populate itself. |refusal_detail| is written once on the
  // raster thread before |refusal| is stored, and read on the platform thread
  // only after |refusal| is seen non-zero, which is what makes the plain
  // pointer safe to share. Zero is no failure, otherwise a kLumitError code
  // plus one.
  gchar* refusal_detail;
  std::atomic<int> refusal;
  GLuint gl_texture;
  GLenum target;
  EGLImageKHR egl_image;
  // Bumped on the raster thread in `populate`, read on the platform thread in
  // `frameReady`, so it must be atomic (the Windows sibling does the same).
  std::atomic<uint64_t> presented;
};

G_DEFINE_TYPE(LumitDmabufTexture, lumit_dmabuf_texture, fl_texture_gl_get_type())

// Import the DMA-BUF into an EGLImage and bind it to a fresh GL texture. Runs on
// the render thread with the Flutter GL context current (called from populate).
// Returns FALSE and sets |error| on failure, so the engine drops the frame and
// Dart falls back to the read-back path.
static gboolean lumit_dmabuf_texture_create(LumitDmabufTexture* self,
                                            GError** error) {
  static PFNEGLCREATEIMAGEKHRPROC create_image =
      reinterpret_cast<PFNEGLCREATEIMAGEKHRPROC>(
          eglGetProcAddress("eglCreateImageKHR"));
  static PFNGLEGLIMAGETARGETTEXTURE2DOESPROC image_target_texture =
      reinterpret_cast<PFNGLEGLIMAGETARGETTEXTURE2DOESPROC>(
          eglGetProcAddress("glEGLImageTargetTexture2DOES"));
  if (create_image == nullptr || image_target_texture == nullptr) {
    g_set_error(error, g_quark_from_static_string("lumit"), 0,
                "EGL dma-buf import extensions are unavailable");
    return FALSE;
  }

  EGLDisplay display = eglGetCurrentDisplay();
  if (display == EGL_NO_DISPLAY) {
    g_set_error(error, g_quark_from_static_string("lumit"), 0,
                "no current EGL display in the populate callback");
    return FALSE;
  }

  // Which driver Flutter is drawing with, in its own words, once a run. The
  // "display GPU" line says which card, this says whose driver.
  static std::atomic_flag named_renderer = ATOMIC_FLAG_INIT;
  if (!named_renderer.test_and_set()) {
    const GLubyte* renderer = glGetString(GL_RENDERER);
    g_printerr("lumit-runner: Flutter GL renderer: %s\n",
               renderer != nullptr ? reinterpret_cast<const char*>(renderer)
                                   : "unknown");
  }

  // Before the import, not after. Registration already made this check from
  // the platform thread, against the display it could find there. This one
  // asks the display that is current right now, which is the one about to do
  // the importing. A buffer from another card is refused with no EGLImage
  // made, because the driver that takes it may not survive drawing it.
  std::string mismatch;
  if (!GpuMayImport(display, self->renderer_gpu, &mismatch)) {
    g_warning("lumit: %s; refusing the zero-copy Viewer texture",
              mismatch.c_str());
    g_set_error(error, g_quark_from_static_string("lumit"),
                kLumitErrorGpuMismatch, "%s", mismatch.c_str());
    return FALSE;
  }

  // The EGL_LINUX_DMA_BUF_EXT attribute list (mirrors the reference plugin).
  //
  // The modifier attributes are stated whenever the driver understands them.
  // Nvidia's driver needs them even for a linear buffer — without an explicit
  // modifier it guesses, and the import silently produces a black texture. But
  // the attributes are *only legal* with EGL_EXT_image_dma_buf_import_modifiers;
  // a driver without that extension rejects the whole import with
  // EGL_BAD_PARAMETER. So we ask the display which it is, once, and build the
  // list accordingly.
#ifndef EGL_IMAGE_PRESERVED_KHR
#define EGL_IMAGE_PRESERVED_KHR 0x30D2
#endif

  const char* extensions = eglQueryString(display, EGL_EXTENSIONS);
  const gboolean has_modifiers =
      extensions != nullptr &&
      std::strstr(extensions, "EGL_EXT_image_dma_buf_import_modifiers") !=
          nullptr;

  EGLint attribs[30];
  int i = 0;
  attribs[i++] = EGL_LINUX_DRM_FOURCC_EXT;
  attribs[i++] = static_cast<EGLint>(self->fourcc);
  attribs[i++] = EGL_WIDTH;
  attribs[i++] = static_cast<EGLint>(self->width);
  attribs[i++] = EGL_HEIGHT;
  attribs[i++] = static_cast<EGLint>(self->height);
  attribs[i++] = EGL_DMA_BUF_PLANE0_FD_EXT;
  attribs[i++] = self->fd;
  attribs[i++] = EGL_DMA_BUF_PLANE0_OFFSET_EXT;
  attribs[i++] = static_cast<EGLint>(self->offset);
  attribs[i++] = EGL_DMA_BUF_PLANE0_PITCH_EXT;
  attribs[i++] = static_cast<EGLint>(self->stride);
  if (has_modifiers) {
    attribs[i++] = EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT;
    attribs[i++] = static_cast<EGLint>(self->modifier & 0xFFFFFFFF);
    attribs[i++] = EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT;
    attribs[i++] = static_cast<EGLint>(self->modifier >> 32);
  }
  attribs[i++] = EGL_IMAGE_PRESERVED_KHR;
  attribs[i++] = EGL_TRUE;
  attribs[i++] = EGL_NONE;

  EGLImageKHR image = create_image(display, EGL_NO_CONTEXT,
                                   EGL_LINUX_DMA_BUF_EXT, nullptr, attribs);
  if (image == EGL_NO_IMAGE_KHR) {
    EGLint err = eglGetError();
    g_set_error(error, g_quark_from_static_string("lumit"), 0,
                "eglCreateImageKHR failed for the dma-buf (err=0x%x)", err);
    return FALSE;
  }

  // Clear any existing GL errors before calling image_target_texture
  while (glGetError() != GL_NO_ERROR) {}

  GLuint texture = 0;
  glGenTextures(1, &texture);
  GLenum target = GL_TEXTURE_2D;

  glBindTexture(GL_TEXTURE_2D, texture);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
  glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
  image_target_texture(GL_TEXTURE_2D, image);
  if (glGetError() != GL_NO_ERROR) {
    while (glGetError() != GL_NO_ERROR) {}
    target = GL_TEXTURE_EXTERNAL_OES;
    glBindTexture(target, texture);
    glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
    image_target_texture(target, image);
  }
  glBindTexture(target, 0);

  self->target = target;
  self->egl_image = image;
  self->gl_texture = texture;
  return TRUE;
}

static gboolean lumit_dmabuf_texture_populate(FlTextureGL* texture,
                                              uint32_t* target, uint32_t* name,
                                              uint32_t* width, uint32_t* height,
                                              GError** error) {
  LumitDmabufTexture* self = LUMIT_DMABUF_TEXTURE(texture);
  if (self->failed) {
    g_set_error(error, g_quark_from_static_string("lumit"), 0,
                "dma-buf texture import failed earlier");
    return FALSE;
  }
  if (!self->created) {
    if (!lumit_dmabuf_texture_create(self, error)) {
      self->failed = TRUE;
      // Leave the reason where `frameReady` can find it. Detail first, flag
      // second, see the fields' comment.
      const gboolean said = error != nullptr && *error != nullptr;
      self->refusal_detail =
          g_strdup(said ? (*error)->message : "the dma-buf import failed");
      self->refusal.store((said ? (*error)->code : kLumitErrorImport) + 1,
                          std::memory_order_release);
      return FALSE;
    }
    self->created = TRUE;
  }
  self->presented.fetch_add(1, std::memory_order_relaxed);
  *target = self->target;
  *name = self->gl_texture;
  *width = self->width;
  *height = self->height;
  return TRUE;
}

static void lumit_dmabuf_texture_dispose(GObject* object) {
  LumitDmabufTexture* self = LUMIT_DMABUF_TEXTURE(object);
  if (self->egl_image != nullptr) {
    static PFNEGLDESTROYIMAGEKHRPROC destroy_image =
        reinterpret_cast<PFNEGLDESTROYIMAGEKHRPROC>(
            eglGetProcAddress("eglDestroyImageKHR"));
    EGLDisplay display = eglGetCurrentDisplay();
    if (destroy_image != nullptr && display != EGL_NO_DISPLAY) {
      destroy_image(display, self->egl_image);
    }
    self->egl_image = nullptr;
  }
  if (self->gl_texture != 0) {
    glDeleteTextures(1, &self->gl_texture);
    self->gl_texture = 0;
  }
  if (self->fd >= 0) {
    close(self->fd);
    self->fd = -1;
  }
  g_clear_pointer(&self->refusal_detail, g_free);
  G_OBJECT_CLASS(lumit_dmabuf_texture_parent_class)->dispose(object);
}

static void lumit_dmabuf_texture_class_init(LumitDmabufTextureClass* klass) {
  FL_TEXTURE_GL_CLASS(klass)->populate = lumit_dmabuf_texture_populate;
  G_OBJECT_CLASS(klass)->dispose = lumit_dmabuf_texture_dispose;
}

static void lumit_dmabuf_texture_init(LumitDmabufTexture* self) {
  self->fd = -1;
  self->created = FALSE;
  self->failed = FALSE;
  self->refusal_detail = nullptr;
  self->refusal.store(0, std::memory_order_relaxed);
  self->presented.store(0, std::memory_order_relaxed);
}

static LumitDmabufTexture* lumit_dmabuf_texture_new(int fd, uint32_t width,
                                                    uint32_t height,
                                                    uint32_t stride,
                                                    uint32_t offset,
                                                    uint32_t fourcc,
                                                    uint64_t modifier,
                                                    LumitGpuDevice renderer) {
  LumitDmabufTexture* self = LUMIT_DMABUF_TEXTURE(
      g_object_new(lumit_dmabuf_texture_get_type(), nullptr));
  // Our own copy of the descriptor; see the `fd` field's comment. A failed
  // `dup` (descriptor table full, or the engine's fd already gone) must not be
  // ignored: the texture would be silently dead for the whole session.
  self->fd = dup(fd);
  if (self->fd < 0) {
    g_warning(
        "lumit: dup() of the dma-buf descriptor failed (%s); the zero-copy "
        "Viewer texture cannot be registered",
        g_strerror(errno));
    g_object_unref(self);
    return nullptr;
  }
  self->width = width;
  self->height = height;
  self->stride = stride;
  self->offset = offset;
  self->fourcc = fourcc;
  self->modifier = modifier;
  self->renderer_gpu = renderer;
  return self;
}

// ---------------------------------------------------------------------------
// The method-channel bridge state.
// ---------------------------------------------------------------------------

typedef struct {
  FlTextureRegistrar* registrar;  // engine-owned, outlives us
  // texture id (int64) -> FlTexture* (borrowed; the registrar holds the ref).
  GHashTable* textures;
} ViewerTextureBridge;

// Read an integer field from the method-call argument map; returns |fallback|
// when absent or not an int. The standard codec encodes small ints as int32 and
// larger ones as int64, both of which fl_value_get_int handles.
static int64_t GetInt(FlValue* args, const char* key, int64_t fallback) {
  if (args == nullptr || fl_value_get_type(args) != FL_VALUE_TYPE_MAP) {
    return fallback;
  }
  FlValue* v = fl_value_lookup_string(args, key);
  if (v == nullptr || fl_value_get_type(v) != FL_VALUE_TYPE_INT) {
    return fallback;
  }
  return fl_value_get_int(v);
}

static void handle_register(ViewerTextureBridge* bridge, FlValue* args,
                            FlMethodCall* call) {
  int fd = static_cast<int>(GetInt(args, "fd", -1));
  uint32_t width = static_cast<uint32_t>(GetInt(args, "width", 0));
  uint32_t height = static_cast<uint32_t>(GetInt(args, "height", 0));
  uint32_t stride = static_cast<uint32_t>(GetInt(args, "stride", 0));
  uint32_t offset = static_cast<uint32_t>(GetInt(args, "offset", 0));
  uint32_t fourcc = static_cast<uint32_t>(GetInt(args, "fourcc", 0));
  uint64_t modifier = static_cast<uint64_t>(GetInt(args, "modifier", 0));
  if (fd < 0 || width == 0 || height == 0) {
    fl_method_call_respond_error(call, "bad_args",
                                 "register needs fd, width and height", nullptr,
                                 nullptr);
    return;
  }

  // Which card the engine drew on, when its driver said. A missing argument
  // reads as -1, which is unknown and compares with nothing.
  LumitGpuDevice renderer_gpu;
  renderer_gpu.render = lumit_drm_node_from_args(
      GetInt(args, "renderMajor", -1), GetInt(args, "renderMinor", -1));
  renderer_gpu.primary = lumit_drm_node_from_args(
      GetInt(args, "primaryMajor", -1), GetInt(args, "primaryMinor", -1));

  // Refused here, before a texture exists, when the two cards are known to
  // differ. Answering the registration is the only way Dart hears at once. A
  // failure inside populate is on the raster thread and is only collected by
  // the next `frameReady`, and a still picture has no next frame. populate
  // makes the same check again against the display that is actually current
  // (see lumit_dmabuf_texture_create).
  std::string mismatch;
  if (!GpuMayImport(FlutterEglDisplay(), renderer_gpu, &mismatch)) {
    g_warning("lumit: %s; refusing the zero-copy Viewer texture",
              mismatch.c_str());
    fl_method_call_respond_error(call, "gpu_mismatch", mismatch.c_str(),
                                 nullptr, nullptr);
    return;
  }

  LumitDmabufTexture* texture = lumit_dmabuf_texture_new(
      fd, width, height, stride, offset, fourcc, modifier, renderer_gpu);
  if (texture == nullptr) {
    fl_method_call_respond_error(call, "dup_failed",
                                 "could not duplicate the dma-buf descriptor",
                                 nullptr, nullptr);
    return;
  }
  fl_texture_registrar_register_texture(bridge->registrar,
                                        FL_TEXTURE(texture));
  int64_t id = fl_texture_get_id(FL_TEXTURE(texture));
  // The registrar holds the strong ref; keep a borrowed pointer for
  // frameReady/unregister and drop our construction ref.
  g_hash_table_insert(bridge->textures, GINT_TO_POINTER(id), texture);
  g_object_unref(texture);

  g_autoptr(FlMethodResponse) response =
      FL_METHOD_RESPONSE(fl_method_success_response_new(fl_value_new_int(id)));
  fl_method_call_respond(call, response, nullptr);
}

static void handle_frame_ready(ViewerTextureBridge* bridge, FlValue* args,
                               FlMethodCall* call) {
  int64_t id = GetInt(args, "textureId", 0);
  gpointer texture = g_hash_table_lookup(bridge->textures, GINT_TO_POINTER(id));
  int64_t presented = 0;
  if (texture != nullptr) {
    // An import that failed is told to Dart here as an error, instead of as a
    // draw count that never moves. The mismatch has its own code because the
    // Viewer shows a message for it, anything else is "import_failed".
    LumitDmabufTexture* dmabuf = LUMIT_DMABUF_TEXTURE(texture);
    const int refusal = dmabuf->refusal.load(std::memory_order_acquire);
    if (refusal != 0) {
      fl_method_call_respond_error(
          call,
          refusal - 1 == kLumitErrorGpuMismatch ? "gpu_mismatch"
                                                : "import_failed",
          dmabuf->refusal_detail, nullptr, nullptr);
      return;
    }
    fl_texture_registrar_mark_texture_frame_available(
        bridge->registrar, FL_TEXTURE(texture));
    presented = static_cast<int64_t>(
        LUMIT_DMABUF_TEXTURE(texture)->presented.load(
            std::memory_order_relaxed));
  }
  g_autoptr(FlMethodResponse) response =
      FL_METHOD_RESPONSE(fl_method_success_response_new(fl_value_new_int(presented)));
  fl_method_call_respond(call, response, nullptr);
}

static void handle_unregister(ViewerTextureBridge* bridge, FlValue* args,
                              FlMethodCall* call) {
  int64_t id = GetInt(args, "textureId", 0);
  gpointer texture = g_hash_table_lookup(bridge->textures, GINT_TO_POINTER(id));
  if (texture != nullptr) {
    fl_texture_registrar_unregister_texture(bridge->registrar,
                                            FL_TEXTURE(texture));
    g_hash_table_remove(bridge->textures, GINT_TO_POINTER(id));
  }
  g_autoptr(FlMethodResponse) response =
      FL_METHOD_RESPONSE(fl_method_success_response_new(fl_value_new_null()));
  fl_method_call_respond(call, response, nullptr);
}

// Which card Flutter is drawing with, as the line printed to stderr, or null
// before anything has asked. Dart writes it to lumit-diagnostics.log once a
// run, beside the engine's own "graphics adapter" line.
static void handle_display_gpu(FlMethodCall* call) {
  std::string said;
  {
    std::lock_guard<std::mutex> hold(display_gpu_lock);
    said = display_gpu_said;
  }
  g_autoptr(FlMethodResponse) response = FL_METHOD_RESPONSE(
      fl_method_success_response_new(said.empty()
                                         ? fl_value_new_null()
                                         : fl_value_new_string(said.c_str())));
  fl_method_call_respond(call, response, nullptr);
}

static void method_call_cb(FlMethodChannel* channel, FlMethodCall* call,
                           gpointer user_data) {
  ViewerTextureBridge* bridge = static_cast<ViewerTextureBridge*>(user_data);
  const gchar* method = fl_method_call_get_name(call);
  FlValue* args = fl_method_call_get_args(call);

  if (g_strcmp0(method, "register") == 0) {
    handle_register(bridge, args, call);
  } else if (g_strcmp0(method, "frameReady") == 0) {
    handle_frame_ready(bridge, args, call);
  } else if (g_strcmp0(method, "unregister") == 0) {
    handle_unregister(bridge, args, call);
  } else if (g_strcmp0(method, "displayGpu") == 0) {
    handle_display_gpu(call);
  } else {
    g_autoptr(FlMethodResponse) response =
        FL_METHOD_RESPONSE(fl_method_not_implemented_response_new());
    fl_method_call_respond(call, response, nullptr);
  }
}

void viewer_texture_bridge_register(FlBinaryMessenger* messenger,
                                    FlTextureRegistrar* registrar) {
  ViewerTextureBridge* bridge = g_new0(ViewerTextureBridge, 1);
  bridge->registrar = registrar;
  bridge->textures = g_hash_table_new(g_direct_hash, g_direct_equal);

  g_autoptr(FlStandardMethodCodec) codec = fl_standard_method_codec_new();
  // Leaked deliberately: the channel lives for the whole engine, like the
  // registrar it drives (the app owns a single one per Flutter engine).
  FlMethodChannel* channel = fl_method_channel_new(
      messenger, "lumit/viewer_texture", FL_METHOD_CODEC(codec));
  fl_method_channel_set_method_call_handler(channel, method_call_cb, bridge,
                                            nullptr);
}
