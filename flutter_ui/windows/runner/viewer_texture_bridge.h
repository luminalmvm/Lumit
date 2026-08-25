// The in-runner zero-copy Viewer bridge (K-177).
//
// In plain terms: the Rust engine draws the Viewer's picture into a Windows
// shared GPU texture and hands Dart an OS "handle" naming it. Dart calls this
// small bridge (over the 'lumit/viewer_texture' method channel) to register that
// handle with Flutter's engine as an external GPU-surface texture; Flutter then
// samples it directly for the `Texture` widget — no pixel copy. `frameReady`
// tells the engine a new frame has been drawn.
//
// For the DXGI-shared-handle surface type Flutter's embedder opens the shared
// handle itself (on its own D3D11/ANGLE device), so this bridge holds NO D3D
// device of its own: it only forwards the handle inside a
// FlutterDesktopGpuSurfaceDescriptor and manages the texture-registrar
// lifecycle.
//
// The plumbing pattern — the descriptor shape, the
// kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle surface type, and the
// register / mark-frame-available dance — follows the MIT-licensed
// `flutter_wgpu_texture` package as a reference. We borrow the pattern, not its
// code (it owns its own renderer/scene architecture we do not want).
//
// NOTE: this file compiles only as part of `flutter build windows` on a real
// machine (the sandbox that authored it cannot run the Windows toolchain). It is
// written against the actual flutter_windows / texture-registrar headers.

#ifndef RUNNER_VIEWER_TEXTURE_BRIDGE_H_
#define RUNNER_VIEWER_TEXTURE_BRIDGE_H_

#include <flutter/method_channel.h>
#include <flutter/plugin_registrar_windows.h>
#include <flutter/standard_method_codec.h>
#include <flutter/texture_registrar.h>
#include <flutter_windows.h>

#include <atomic>
#include <cstdint>
#include <map>
#include <memory>

// Registers engine-created shared textures with Flutter and drives their
// frame-available notifications, over the 'lumit/viewer_texture' channel.
class ViewerTextureBridge {
 public:
  // Builds the bridge over the plugin registrar the engine hands out (see
  // FlutterWindow::OnCreate). The registrar and its messenger/texture-registrar
  // must outlive this object; it keeps the registrar wrapper alive itself.
  explicit ViewerTextureBridge(FlutterDesktopPluginRegistrarRef registrar_ref);
  ~ViewerTextureBridge();

  ViewerTextureBridge(const ViewerTextureBridge&) = delete;
  ViewerTextureBridge& operator=(const ViewerTextureBridge&) = delete;

 private:
  // One registered shared texture: the descriptor Flutter re-reads each frame
  // (kept alive here so the callback can return a stable pointer) and the
  // texture variant it was registered with (must outlive the registration).
  struct Entry {
    FlutterDesktopGpuSurfaceDescriptor descriptor;
    std::unique_ptr<flutter::TextureVariant> texture;
    // How many times Flutter has actually asked for this descriptor — that is,
    // how many times it has composited the texture.
    //
    // It exists because the failure mode here is silent and total: if the
    // embedder cannot open or draw the shared handle, it simply draws nothing,
    // reports no error, and the Viewer shows an empty panel for the rest of the
    // session while everything else carries on. A count of zero after several
    // frames have been announced is the only evidence available, so Dart asks
    // for it and falls back to copying pixels when it stays at zero.
    //
    // Atomic: the callback runs on the raster thread, the read on the platform
    // thread.
    std::atomic<uint64_t> presented{0};
  };

  void HandleMethodCall(
      const flutter::MethodCall<flutter::EncodableValue>& call,
      std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result);

  // Register the shared texture named by |handle| (an NT HANDLE value) with the
  // given size, returning its Flutter texture id (0 on failure).
  int64_t Register(uint64_t handle, uint32_t width, uint32_t height);
  bool MarkFrameAvailable(int64_t texture_id);
  // How many times Flutter has composited |texture_id| (see Entry::presented).
  uint64_t Presented(int64_t texture_id) const;
  void Unregister(int64_t texture_id);

  std::unique_ptr<flutter::PluginRegistrarWindows> registrar_;
  std::unique_ptr<flutter::MethodChannel<flutter::EncodableValue>> channel_;
  flutter::TextureRegistrar* textures_ = nullptr;  // owned by registrar_
  std::map<int64_t, std::unique_ptr<Entry>> entries_;
};

#endif  // RUNNER_VIEWER_TEXTURE_BRIDGE_H_
