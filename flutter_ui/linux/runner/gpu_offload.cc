#include "gpu_offload.h"

#include <glib.h>

#include <string>
#include <vector>

#include "gpu_device_match.h"

// The variable NVIDIA's driver reads to draw a program on its card when another
// card drives the screen. It covers EGL and Vulkan on its own. GLX needs a
// second variable (__GLX_VENDOR_LIBRARY_NAME), which is left alone on purpose:
// Flutter's own context is EGL on both X11 and Wayland, and forcing GLX to
// NVIDIA stops the window opening at all on a machine that can't offload.
static const char kOffloadVariable[] = "__NV_PRIME_RENDER_OFFLOAD";

// Turns the request off, for a machine where it makes things worse.
static const char kOptOutVariable[] = "LUMIT_NO_PRIME_OFFLOAD";

// One hexadecimal number from a sysfs file such as "0x10de\n", or |fallback|
// when the file can't be read.
static guint64 ReadHex(const gchar* directory, const gchar* file,
                       guint64 fallback) {
  g_autofree gchar* path = g_build_filename(directory, file, nullptr);
  g_autofree gchar* text = nullptr;
  if (!g_file_get_contents(path, &text, nullptr, nullptr)) {
    return fallback;
  }
  return g_ascii_strtoull(text, nullptr, 16);
}

// Every graphics card on the PCI bus, with the kernel driver that has it.
//
// Read from /sys/bus/pci/devices, which is there inside a Flatpak sandbox too
// (/sys/module isn't, so the driver is taken from each card's own `driver`
// link). A card is any device whose class starts with 0x03, the display
// controllers: a laptop's NVIDIA card is usually 0x0302 and not the 0x0300 a
// desktop card is.
static std::vector<LumitPciGpu> ListPciGpus() {
  std::vector<LumitPciGpu> gpus;
  static const char kRoot[] = "/sys/bus/pci/devices";
  GDir* devices = g_dir_open(kRoot, 0, nullptr);
  if (devices == nullptr) {
    return gpus;
  }
  for (const gchar* name = g_dir_read_name(devices); name != nullptr;
       name = g_dir_read_name(devices)) {
    g_autofree gchar* device = g_build_filename(kRoot, name, nullptr);
    if ((ReadHex(device, "class", 0) >> 16) != 0x03) {
      continue;
    }
    LumitPciGpu gpu;
    gpu.vendor = static_cast<uint32_t>(ReadHex(device, "vendor", 0));
    g_autofree gchar* link = g_build_filename(device, "driver", nullptr);
    g_autofree gchar* target = g_file_read_link(link, nullptr);
    if (target != nullptr) {
      g_autofree gchar* driver = g_path_get_basename(target);
      gpu.driver = driver;
    }
    gpus.push_back(gpu);
  }
  g_dir_close(devices);
  return gpus;
}

void lumit_request_gpu_offload() {
  const std::vector<LumitPciGpu> gpus = ListPciGpus();
  // Asked first with nothing set, to find out whether this is the machine at
  // all. Every other machine returns here without a word.
  if (!lumit_wants_nvidia_offload(gpus, false, false)) {
    return;
  }
  const bool already_chosen = g_getenv(kOffloadVariable) != nullptr;
  const bool opted_out = g_getenv(kOptOutVariable) != nullptr;
  if (!lumit_wants_nvidia_offload(gpus, already_chosen, opted_out)) {
    g_printerr(
        "lumit-runner: NVIDIA card beside another card, PRIME render offload "
        "left as it is (%s)\n",
        opted_out ? "LUMIT_NO_PRIME_OFFLOAD is set"
                  : "__NV_PRIME_RENDER_OFFLOAD is already set");
    return;
  }
  g_setenv(kOffloadVariable, "1", TRUE);
  g_printerr(
      "lumit-runner: NVIDIA card beside another card, asking for PRIME render "
      "offload so Flutter draws on the card the renderer uses "
      "(LUMIT_NO_PRIME_OFFLOAD=1 turns this off)\n");
}
