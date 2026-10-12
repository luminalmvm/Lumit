// Asking for the NVIDIA card on a laptop with two graphics cards.
//
// In plain terms: on a laptop with an NVIDIA card beside an Intel or AMD one,
// the desktop gives a program the Intel or AMD card unless it asks otherwise.
// The engine does ask, through Vulkan, and gets the NVIDIA card. Flutter
// doesn't, so the Viewer's picture ends up on one card and the window on the
// other, and NVIDIA's driver can't share a picture with another card. The
// runner refuses that hand-off (viewer_texture_bridge.cc) and the Viewer shows
// a message in place of the picture.
//
// This is what makes the picture appear instead. NVIDIA's driver reads one
// environment variable, __NV_PRIME_RENDER_OFFLOAD, when a program opens its
// drawing context ("PRIME render offload"). With it set, Flutter's EGL context
// is made on the NVIDIA card, the same card the engine is on, and the hand-off
// is the ordinary one-card case. It has to be set before GTK starts, since that
// is when the context is made.
//
// It is only set on the machine that needs it, see lumit_wants_nvidia_offload
// in gpu_device_match.h. A driver that can't offload after all is meant to
// decline the display, so Mesa takes it as before and the worst case is the
// message. NVIDIA's Wayland code does exactly that. Its X11 code isn't public,
// so LUMIT_NO_PRIME_OFFLOAD is there for a machine where it doesn't.

#ifndef RUNNER_GPU_OFFLOAD_H_
#define RUNNER_GPU_OFFLOAD_H_

// Set __NV_PRIME_RENDER_OFFLOAD=1 for this process when the machine has an
// NVIDIA card run by NVIDIA's driver beside another card, and nobody has set
// the variable already. Call once, first thing in main(), before GTK.
//
// LUMIT_NO_PRIME_OFFLOAD=1 turns it off.
void lumit_request_gpu_offload();

#endif  // RUNNER_GPU_OFFLOAD_H_
