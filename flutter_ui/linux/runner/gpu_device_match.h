// Which graphics card is which: the decisions the Linux runner makes about a
// machine with two of them.
//
// In plain terms: a laptop with two graphics cards gives the two halves of
// Lumit different ones. The engine asks Vulkan for the fast card and draws the
// Viewer's picture there. Flutter draws the window with whichever card the
// desktop gave it, which is the slow one. The picture is handed from one to the
// other without a copy (viewer_texture_bridge.h), and that only works when both
// are the same card, or two cards whose drivers know how to share. NVIDIA's
// doesn't. On an Intel + NVIDIA laptop the Intel driver took the hand-off and
// then killed the process on the first draw ("intel: the execbuf ioctl keeps
// returning ENOMEM"), from inside the driver, where nothing of Lumit's could
// catch it.
//
// Two decisions come out of that, and both are here:
//
//   * lumit_gpu_match and lumit_gpu_refuses_import: is the card the picture is
//     on the card Flutter is drawing with, and if not, can the two share it?
//     The runner asks before it imports anything, and refuses the texture with
//     a message when they can't (viewer_texture_bridge.cc).
//   * lumit_wants_nvidia_offload: should Flutter be asked to draw with the
//     NVIDIA card in the first place, so the two agree and nothing has to be
//     refused? The runner asks once, before GTK starts (gpu_offload.cc).
//
// **Why they are in a header of their own.** Everything else in the runner
// needs GTK, EGL and a real graphics card to run at all, so none of it can be
// tested where it is written. These two decisions are the part that can be
// wrong in a way nobody would see until a laptop crashed, and they are plain
// arithmetic on plain values. So they are kept free of every one of those
// dependencies (the C++ standard library and nothing else), and
// gpu_device_match_test.cc runs them on any machine with a compiler. CI builds
// and runs that test in the `flutter-linux` job.
//
// A card is named the way the kernel names it: by the major and minor numbers
// of its device node under /dev/dri. A card has up to two nodes, a *primary*
// one (/dev/dri/card0) that can also drive a display and a *render* one
// (/dev/dri/renderD128) that can only draw. They are different files with
// different numbers, so a render node is only ever compared with a render node
// and a primary with a primary.

#ifndef RUNNER_GPU_DEVICE_MATCH_H_
#define RUNNER_GPU_DEVICE_MATCH_H_

#include <cstdint>
#include <string>
#include <vector>

// The major number every DRM device node has (DRM_MAJOR in the kernel). A node
// with any other major is some other kind of device (NVIDIA's own /dev/nvidia0
// is 195) and says nothing about which /dev/dri card it is.
constexpr uint32_t kLumitDrmMajor = 226;

// One device node. `known` is false when the driver did not report it, and an
// unknown node is never equal to anything, including another unknown one.
struct LumitDrmNode {
  bool known = false;
  uint32_t dev_major = 0;
  uint32_t dev_minor = 0;
};

// The nodes of one card. Either may be unknown.
struct LumitGpuDevice {
  LumitDrmNode primary;
  LumitDrmNode render;
};

enum class LumitDrmNodeKind { kNone, kPrimary, kRender };

// Which kind of node a device number is, going by the ranges the kernel hands
// minors out in: 0-63 primary, 128-191 render. Anything else (the retired
// control nodes at 64-127, a major that is not DRM's) is not a node this code
// can compare, and is reported as none.
//
// Sorted by number rather than by which EGL query produced the path, because a
// driver is free to answer "which file is this display on" with either kind,
// and filing a render node under "primary" would make one card look like two.
inline LumitDrmNodeKind lumit_drm_node_kind(uint32_t dev_major,
                                            uint32_t dev_minor) {
  if (dev_major != kLumitDrmMajor) {
    return LumitDrmNodeKind::kNone;
  }
  if (dev_minor < 64) {
    return LumitDrmNodeKind::kPrimary;
  }
  if (dev_minor >= 128 && dev_minor < 192) {
    return LumitDrmNodeKind::kRender;
  }
  return LumitDrmNodeKind::kNone;
}

// Record a device number on `device` under the kind its number says it is.
// Does nothing for a number that is not a DRM node.
inline void lumit_gpu_device_add(LumitGpuDevice* device, uint32_t dev_major,
                                 uint32_t dev_minor) {
  switch (lumit_drm_node_kind(dev_major, dev_minor)) {
    case LumitDrmNodeKind::kPrimary:
      device->primary = {true, dev_major, dev_minor};
      break;
    case LumitDrmNodeKind::kRender:
      device->render = {true, dev_major, dev_minor};
      break;
    case LumitDrmNodeKind::kNone:
      break;
  }
}

// A node as the engine sent it over the method channel: two integers, with a
// negative one (the fallback for a missing argument) meaning "not reported".
inline LumitDrmNode lumit_drm_node_from_args(int64_t dev_major,
                                             int64_t dev_minor) {
  if (dev_major < 0 || dev_minor < 0 || dev_major > UINT32_MAX ||
      dev_minor > UINT32_MAX) {
    return {};
  }
  return {true, static_cast<uint32_t>(dev_major),
          static_cast<uint32_t>(dev_minor)};
}

enum class LumitGpuMatch {
  // One side or the other did not say which card it is on, or the two sides
  // named different kinds of node. Nothing can be concluded.
  kUnknown,
  kSame,
  kDifferent,
};

// Whether the card the engine rendered on (`renderer`) is the card Flutter is
// drawing with (`display`).
//
// Render nodes are compared when both sides have one, because that is the node
// both drivers use to draw and the one every current driver reports. Primary
// nodes are the fallback for a driver that only names its primary node. If
// there is no kind both sides reported, the answer is unknown.
inline LumitGpuMatch lumit_gpu_match(const LumitGpuDevice& renderer,
                                     const LumitGpuDevice& display) {
  const auto same = [](const LumitDrmNode& a, const LumitDrmNode& b) {
    return a.dev_major == b.dev_major && a.dev_minor == b.dev_minor;
  };
  if (renderer.render.known && display.render.known) {
    return same(renderer.render, display.render) ? LumitGpuMatch::kSame
                                                 : LumitGpuMatch::kDifferent;
  }
  if (renderer.primary.known && display.primary.known) {
    return same(renderer.primary, display.primary) ? LumitGpuMatch::kSame
                                                   : LumitGpuMatch::kDifferent;
  }
  return LumitGpuMatch::kUnknown;
}

// Whether the runner must refuse to import the engine's buffer.
//
// Two different cards is not enough on its own. The drivers in the kernel tree
// (i915, xe, amdgpu, nouveau) move a shared buffer into system memory when a
// second card asks for it, which is how a laptop with an Intel and an AMD card
// has always put the fast card's picture on the slow card's screen. NVIDIA's
// own driver doesn't do that for a buffer in video memory. The other card's
// import succeeds, and its first draw fails with ENOMEM, which Mesa's Intel
// driver answers by aborting the process.
//
// So the import is refused when the cards are known to differ and the buffer
// was made by NVIDIA's driver, or by a driver that couldn't be named
// (`exporter_driver` empty), since an unnamed driver might be NVIDIA's.
// Everything else is left as it was: the same card, a card nobody could
// identify, and two cards whose drivers are known to share.
//
// `allow_cross_gpu` is the LUMIT_ALLOW_CROSS_GPU_IMPORT escape hatch, for
// confirming on the laptop that crashed that this check is what stops it.
inline bool lumit_gpu_refuses_import(LumitGpuMatch match,
                                     const std::string& exporter_driver,
                                     bool allow_cross_gpu) {
  if (match != LumitGpuMatch::kDifferent || allow_cross_gpu) {
    return false;
  }
  return exporter_driver.empty() || exporter_driver == "nvidia";
}

// One graphics card as the PCI bus lists it: who made it, and which kernel
// driver has it ("i915", "amdgpu", "nouveau", "nvidia"), empty when none.
struct LumitPciGpu {
  uint32_t vendor = 0;
  std::string driver;
};

constexpr uint32_t kLumitPciVendorNvidia = 0x10de;

// Whether to ask NVIDIA's driver to draw Flutter on its card ("PRIME render
// offload"), decided before GTK starts.
//
// Yes only on the machine the crash happened on: an NVIDIA card run by
// NVIDIA's own driver, sitting beside a card from somebody else. Every other
// machine is left exactly as it was:
//
//   * one card, of any make: there is nothing to disagree about.
//   * NVIDIA under nouveau: that is a Mesa driver, and the variable this sets
//     is one only NVIDIA's own driver reads.
//   * `already_chosen`: somebody set the offload variable themselves, to either
//     value. Their choice stands.
//   * `opted_out`: LUMIT_NO_PRIME_OFFLOAD is set.
inline bool lumit_wants_nvidia_offload(const std::vector<LumitPciGpu>& gpus,
                                       bool already_chosen, bool opted_out) {
  if (already_chosen || opted_out) {
    return false;
  }
  bool nvidia = false;
  bool another = false;
  for (const LumitPciGpu& gpu : gpus) {
    if (gpu.vendor == kLumitPciVendorNvidia) {
      nvidia = nvidia || gpu.driver == "nvidia";
    } else {
      another = true;
    }
  }
  return nvidia && another;
}

#endif  // RUNNER_GPU_DEVICE_MATCH_H_
