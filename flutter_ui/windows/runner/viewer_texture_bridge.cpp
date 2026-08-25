#include "viewer_texture_bridge.h"

#include <string>
#include <variant>

namespace {

// Read a 64-bit unsigned value out of a method-call argument map. The standard
// codec encodes a Dart int as int32 when it fits and int64 otherwise, so both
// must be accepted — a shared-texture NT handle is a 64-bit pointer value, while
// width/height arrive as int32.
uint64_t GetU64(const flutter::EncodableMap* map, const char* key) {
  if (map == nullptr) {
    return 0;
  }
  auto it = map->find(flutter::EncodableValue(std::string(key)));
  if (it == map->end()) {
    return 0;
  }
  const flutter::EncodableValue& value = it->second;
  if (const auto* v64 = std::get_if<int64_t>(&value)) {
    return static_cast<uint64_t>(*v64);
  }
  if (const auto* v32 = std::get_if<int32_t>(&value)) {
    // Via uint32_t so a large width/height is not sign-extended.
    return static_cast<uint64_t>(static_cast<uint32_t>(*v32));
  }
  return 0;
}

}  // namespace

ViewerTextureBridge::ViewerTextureBridge(
    FlutterDesktopPluginRegistrarRef registrar_ref)
    : registrar_(
          std::make_unique<flutter::PluginRegistrarWindows>(registrar_ref)) {
  textures_ = registrar_->texture_registrar();
  channel_ = std::make_unique<flutter::MethodChannel<flutter::EncodableValue>>(
      registrar_->messenger(), "lumit/viewer_texture",
      &flutter::StandardMethodCodec::GetInstance());
  channel_->SetMethodCallHandler(
      [this](const flutter::MethodCall<flutter::EncodableValue>& call,
             std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>>
                 result) { HandleMethodCall(call, std::move(result)); });
}

ViewerTextureBridge::~ViewerTextureBridge() {
  // Unregister everything still live before the registrar goes away.
  for (auto& pair : entries_) {
    if (textures_ != nullptr) {
      textures_->UnregisterTexture(pair.first, nullptr);
    }
  }
  entries_.clear();
}

void ViewerTextureBridge::HandleMethodCall(
    const flutter::MethodCall<flutter::EncodableValue>& call,
    std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result) {
  const auto* args = std::get_if<flutter::EncodableMap>(call.arguments());

  if (call.method_name() == "register") {
    const uint64_t handle = GetU64(args, "handle");
    const uint32_t width = static_cast<uint32_t>(GetU64(args, "width"));
    const uint32_t height = static_cast<uint32_t>(GetU64(args, "height"));
    if (handle == 0 || width == 0 || height == 0) {
      result->Error("bad_args", "register needs handle, width and height");
      return;
    }
    const int64_t id = Register(handle, width, height);
    if (id == 0) {
      result->Error("register_failed", "could not register the shared texture");
      return;
    }
    result->Success(flutter::EncodableValue(id));
    return;
  }

  if (call.method_name() == "frameReady") {
    const int64_t id = static_cast<int64_t>(GetU64(args, "textureId"));
    MarkFrameAvailable(id);
    // Answer with how many times Flutter has actually drawn this texture, so
    // the caller can tell "registered and working" from "registered and
    // silently never drawn" — the two look identical otherwise.
    result->Success(
        flutter::EncodableValue(static_cast<int64_t>(Presented(id))));
    return;
  }

  if (call.method_name() == "unregister") {
    const int64_t id = static_cast<int64_t>(GetU64(args, "textureId"));
    Unregister(id);
    result->Success();
    return;
  }

  result->NotImplemented();
}

int64_t ViewerTextureBridge::Register(uint64_t handle, uint32_t width,
                                      uint32_t height) {
  if (textures_ == nullptr) {
    return 0;
  }
  auto entry = std::make_unique<Entry>();
  entry->descriptor = {};
  entry->descriptor.struct_size = sizeof(FlutterDesktopGpuSurfaceDescriptor);
  // For kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle, |handle| is the NT HANDLE
  // of the shared texture; the embedder opens it on its own D3D11/ANGLE device.
  entry->descriptor.handle =
      reinterpret_cast<void*>(static_cast<uintptr_t>(handle));
  entry->descriptor.width = width;
  entry->descriptor.height = height;
  entry->descriptor.visible_width = width;
  entry->descriptor.visible_height = height;
  // The engine's shared texture is DXGI_FORMAT_B8G8R8A8_UNORM holding the
  // display-encoded bytes (K-177). BGRA, because ANGLE — which opens this
  // handle inside the engine — matches share-handle surfaces against its own
  // B8G8R8A8 configs and silently declines RGBA: the texture registers, the
  // compositor asks for it every frame, and nothing appears.
  entry->descriptor.format = kFlutterDesktopPixelFormatBGRA8888;
  entry->descriptor.release_callback = nullptr;
  entry->descriptor.release_context = nullptr;

  // The callback returns a stable pointer to the descriptor held inside the
  // heap-allocated Entry; moving the unique_ptr into the map does not move the
  // Entry itself, so the pointer stays valid for the texture's lifetime. The
  // same reasoning lets it count its own invocations.
  Entry* entry_ptr = entry.get();
  entry->texture = std::make_unique<flutter::TextureVariant>(
      flutter::GpuSurfaceTexture(
          kFlutterDesktopGpuSurfaceTypeDxgiSharedHandle,
          [entry_ptr](size_t /*width*/, size_t /*height*/)
              -> const FlutterDesktopGpuSurfaceDescriptor* {
            entry_ptr->presented.fetch_add(1, std::memory_order_relaxed);
            return &entry_ptr->descriptor;
          }));

  const int64_t id = textures_->RegisterTexture(entry->texture.get());
  entries_[id] = std::move(entry);
  return id;
}

bool ViewerTextureBridge::MarkFrameAvailable(int64_t texture_id) {
  if (textures_ == nullptr || entries_.find(texture_id) == entries_.end()) {
    return false;
  }
  return textures_->MarkTextureFrameAvailable(texture_id);
}

uint64_t ViewerTextureBridge::Presented(int64_t texture_id) const {
  auto it = entries_.find(texture_id);
  if (it == entries_.end()) {
    return 0;
  }
  return it->second->presented.load(std::memory_order_relaxed);
}

void ViewerTextureBridge::Unregister(int64_t texture_id) {
  auto it = entries_.find(texture_id);
  if (it == entries_.end()) {
    return;
  }
  if (textures_ != nullptr) {
    textures_->UnregisterTexture(texture_id, nullptr);
  }
  entries_.erase(it);
}
