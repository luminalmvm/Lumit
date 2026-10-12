// The two-graphics-card decisions, tested without a graphics card.
//
// In plain terms: gpu_device_match.h decides whether the runner may take the
// engine's picture and whether to ask NVIDIA's driver to draw Flutter. Getting
// either wrong shows up as a crash, or as a Viewer switched off on a machine
// with nothing wrong with it, and only on hardware the people writing this do
// not have. So the decisions are checked here against the cases that matter,
// with the numbers from the laptop in the original report.
//
// This is a program, not a test-framework file: it has no dependency but the
// compiler, so it runs anywhere. CI builds and runs it in the `flutter-linux`
// job. By hand, from this directory:
//
//     c++ -std=c++17 -Wall -Wextra -Werror gpu_device_match_test.cc -o t && ./t
//
// It prints the first check that fails and exits non-zero, or prints one line
// and exits 0.

#include "gpu_device_match.h"

#include <cstdio>
#include <cstdlib>

namespace {

int failures = 0;

void Check(bool ok, const char* what) {
  if (!ok) {
    std::fprintf(stderr, "FAILED: %s\n", what);
    failures++;
  }
}

LumitGpuDevice Device(int64_t primary_minor, int64_t render_minor) {
  LumitGpuDevice device;
  if (primary_minor >= 0) {
    device.primary = lumit_drm_node_from_args(kLumitDrmMajor, primary_minor);
  }
  if (render_minor >= 0) {
    device.render = lumit_drm_node_from_args(kLumitDrmMajor, render_minor);
  }
  return device;
}

// The laptop in the report: Intel on card0/renderD128 drawing the window,
// NVIDIA on card1/renderD129 drawing the picture.
const LumitGpuDevice kIntel = Device(0, 128);
const LumitGpuDevice kNvidia = Device(1, 129);
const LumitGpuDevice kUnknown = Device(-1, -1);

void MatchTests() {
  Check(lumit_gpu_match(kNvidia, kNvidia) == LumitGpuMatch::kSame,
        "one card on both sides is the same card");
  Check(lumit_gpu_match(kNvidia, kIntel) == LumitGpuMatch::kDifferent,
        "the laptop in the report is two different cards");

  // Unknown on either side, or both, concludes nothing.
  Check(lumit_gpu_match(kUnknown, kIntel) == LumitGpuMatch::kUnknown,
        "an engine that does not name its card is unknown");
  Check(lumit_gpu_match(kNvidia, kUnknown) == LumitGpuMatch::kUnknown,
        "a display that does not name its card is unknown");
  Check(lumit_gpu_match(kUnknown, kUnknown) == LumitGpuMatch::kUnknown,
        "two unknowns are unknown, not the same card");

  // A render node is compared with a render node and a primary with a
  // primary. One card named by its render node on one side and its primary
  // node on the other must not read as two cards.
  const LumitGpuDevice render_only = Device(-1, 129);
  const LumitGpuDevice primary_only = Device(1, -1);
  Check(lumit_gpu_match(render_only, primary_only) == LumitGpuMatch::kUnknown,
        "a render node and a primary node are never compared");
  Check(lumit_gpu_match(primary_only, render_only) == LumitGpuMatch::kUnknown,
        "nor the other way round");

  // The primary node is the fallback when one side has no render node.
  Check(lumit_gpu_match(kNvidia, primary_only) == LumitGpuMatch::kSame,
        "primary nodes are compared when render nodes can't be");
  Check(lumit_gpu_match(kNvidia, Device(0, -1)) == LumitGpuMatch::kDifferent,
        "and they tell two cards apart");

  // Render nodes win when both kinds are there, so the answer does not depend
  // on which of the two a driver happens to report as well.
  Check(lumit_gpu_match(Device(-1, 129), kNvidia) == LumitGpuMatch::kSame,
        "render nodes are compared first");
}

void RefusalTests() {
  // The laptop in the report: two cards, the picture made by NVIDIA's driver.
  Check(lumit_gpu_refuses_import(LumitGpuMatch::kDifferent, "nvidia", false),
        "a buffer from NVIDIA's driver is refused on another card");
  // A driver nobody could name might be NVIDIA's.
  Check(lumit_gpu_refuses_import(LumitGpuMatch::kDifferent, "", false),
        "a buffer from an unnamed driver is refused on another card");

  // Two cards whose drivers share buffers, which worked before this check.
  Check(!lumit_gpu_refuses_import(LumitGpuMatch::kDifferent, "amdgpu", false),
        "an amdgpu buffer imports on another card");
  Check(!lumit_gpu_refuses_import(LumitGpuMatch::kDifferent, "i915", false),
        "an i915 buffer imports on another card");
  Check(!lumit_gpu_refuses_import(LumitGpuMatch::kDifferent, "nouveau", false),
        "a nouveau buffer imports on another card");

  // One card, or a card nobody could identify, is never refused.
  Check(!lumit_gpu_refuses_import(LumitGpuMatch::kSame, "nvidia", false),
        "one card imports, NVIDIA or not");
  Check(!lumit_gpu_refuses_import(LumitGpuMatch::kUnknown, "nvidia", false),
        "unknown keeps the old behaviour and imports");
  Check(!lumit_gpu_refuses_import(LumitGpuMatch::kUnknown, "", false),
        "unknown imports whoever made the buffer");

  Check(!lumit_gpu_refuses_import(LumitGpuMatch::kDifferent, "nvidia", true),
        "the escape hatch lets a cross-card import through");
}

void NodeTests() {
  Check(lumit_drm_node_kind(226, 0) == LumitDrmNodeKind::kPrimary,
        "card0 is a primary node");
  Check(lumit_drm_node_kind(226, 63) == LumitDrmNodeKind::kPrimary,
        "the last primary minor is a primary node");
  Check(lumit_drm_node_kind(226, 128) == LumitDrmNodeKind::kRender,
        "renderD128 is a render node");
  Check(lumit_drm_node_kind(226, 191) == LumitDrmNodeKind::kRender,
        "the last render minor is a render node");
  Check(lumit_drm_node_kind(226, 64) == LumitDrmNodeKind::kNone,
        "a control node is not compared");
  Check(lumit_drm_node_kind(226, 192) == LumitDrmNodeKind::kNone,
        "a minor past the render range is not compared");
  // /dev/nvidia0, which NVIDIA's EGL may name instead of a /dev/dri file.
  Check(lumit_drm_node_kind(195, 0) == LumitDrmNodeKind::kNone,
        "a device that is not a DRM node is not compared");

  // A path is filed by its number, whichever EGL query it came from.
  LumitGpuDevice device;
  lumit_gpu_device_add(&device, 226, 129);
  Check(device.render.known && !device.primary.known,
        "a render node number is filed as a render node");
  lumit_gpu_device_add(&device, 226, 1);
  Check(device.primary.known && device.primary.dev_minor == 1,
        "a primary node number is filed as a primary node");
  lumit_gpu_device_add(&device, 195, 0);
  Check(device.primary.dev_minor == 1 && device.render.dev_minor == 129,
        "a non-DRM number changes nothing");

  // A missing method-channel argument arrives as -1.
  Check(!lumit_drm_node_from_args(-1, -1).known,
        "a missing argument is an unknown node");
  Check(!lumit_drm_node_from_args(226, -1).known,
        "half a node is an unknown node");
  Check(!lumit_drm_node_from_args(int64_t{1} << 40, 0).known,
        "a number too large for a device number is an unknown node");
  Check(lumit_drm_node_from_args(226, 129).known, "a whole node is known");
}

void OffloadTests() {
  const LumitPciGpu intel{0x8086, "i915"};
  const LumitPciGpu amd{0x1002, "amdgpu"};
  const LumitPciGpu nvidia{kLumitPciVendorNvidia, "nvidia"};
  const LumitPciGpu nouveau{kLumitPciVendorNvidia, "nouveau"};
  const LumitPciGpu unbound{kLumitPciVendorNvidia, ""};

  Check(lumit_wants_nvidia_offload({intel, nvidia}, false, false),
        "Intel beside NVIDIA's driver asks for offload");
  Check(lumit_wants_nvidia_offload({nvidia, amd}, false, false),
        "AMD beside NVIDIA's driver asks for offload, in either order");

  // Single-card machines are untouched, whatever the card.
  Check(!lumit_wants_nvidia_offload({nvidia}, false, false),
        "NVIDIA alone is left alone");
  Check(!lumit_wants_nvidia_offload({intel}, false, false),
        "Intel alone is left alone");
  Check(!lumit_wants_nvidia_offload({}, false, false),
        "a machine that lists no card is left alone");
  Check(!lumit_wants_nvidia_offload({nvidia, nvidia}, false, false),
        "two NVIDIA cards have nothing to offload from");

  // The variable is one only NVIDIA's own driver reads.
  Check(!lumit_wants_nvidia_offload({intel, nouveau}, false, false),
        "NVIDIA under nouveau is left alone");
  Check(!lumit_wants_nvidia_offload({intel, unbound}, false, false),
        "an NVIDIA card with no driver is left alone");
  Check(!lumit_wants_nvidia_offload({intel, amd}, false, false),
        "two cards with no NVIDIA are left alone");

  // Somebody else has already decided.
  Check(!lumit_wants_nvidia_offload({intel, nvidia}, true, false),
        "an offload variable already set is respected");
  Check(!lumit_wants_nvidia_offload({intel, nvidia}, false, true),
        "the opt-out is respected");
}

}  // namespace

int main() {
  MatchTests();
  RefusalTests();
  NodeTests();
  OffloadTests();
  if (failures != 0) {
    std::fprintf(stderr, "gpu_device_match_test: %d check(s) failed\n",
                 failures);
    return EXIT_FAILURE;
  }
  std::printf("gpu_device_match_test: all checks passed\n");
  return EXIT_SUCCESS;
}
