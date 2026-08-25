// The in-runner zero-copy Viewer bridge for Linux (K-177).
//
// In plain terms: the Rust engine draws the Viewer's picture into a Vulkan image
// and exports it as a DMA-BUF (a file descriptor naming GPU memory). Dart calls
// this small bridge (over the same 'lumit/viewer_texture' method channel the
// Windows runner uses) to register that fd with Flutter's GTK embedder as an
// external GL texture; Flutter then samples it directly for the `Texture` widget
// — no pixel copy. `frameReady` tells the engine a new frame has been drawn.
//
// The bridge subclasses FlTextureGL. On each `populate` (called on the render
// thread with the GL context current) it imports the DMA-BUF into an EGLImage
// (EGL_EXT_image_dma_buf_import) and binds it to a GL texture with
// glEGLImageTargetTexture2DOES, then returns that texture to the engine.
//
// The channel protocol mirrors the Windows plugin exactly — the same
// 'lumit/viewer_texture' channel with `register` / `frameReady` / `unregister` —
// but `register` carries the DMA-BUF fields {fd, width, height, stride, offset,
// fourcc, modifier} instead of a single NT handle.
//
// The plumbing pattern (the EGL_LINUX_DMA_BUF_EXT attribute list, the
// glEGLImageTargetTexture2DOES bind, the FlTextureGL populate contract) follows
// the MIT-licensed `flutter_wgpu_texture` package as a reference. We borrow the
// pattern, not its code.
//
// NOTE: this file compiles only as part of `flutter build linux` on a real Linux
// machine (the sandbox that authored it cannot run the GTK/EGL toolchain). It is
// written against the actual flutter_linux / EGL / GLES headers.

#ifndef RUNNER_VIEWER_TEXTURE_BRIDGE_H_
#define RUNNER_VIEWER_TEXTURE_BRIDGE_H_

#include <flutter_linux/flutter_linux.h>

G_BEGIN_DECLS

// Register the 'lumit/viewer_texture' channel on `messenger`, importing engine
// DMA-BUF frames as GL external textures through `registrar`. Both must outlive
// the application (they are owned by the FlView/engine). Safe to call once per
// Flutter engine (the main window and each popped-out panel).
void viewer_texture_bridge_register(FlBinaryMessenger* messenger,
                                    FlTextureRegistrar* registrar);

G_END_DECLS

#endif  // RUNNER_VIEWER_TEXTURE_BRIDGE_H_
