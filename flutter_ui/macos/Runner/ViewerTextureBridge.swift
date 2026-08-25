import CoreVideo
import FlutterMacOS
import Foundation
import IOSurface

// The macOS half of the zero-copy Viewer (K-195) — the sibling of
// `windows/runner/viewer_texture_bridge.cpp` and
// `linux/runner/viewer_texture_bridge.cc`, on the same 'lumit/viewer_texture'
// method channel with the same `register` / `frameReady` / `unregister` methods.
//
// In plain terms: the engine draws the Viewer's picture into a piece of graphics
// memory called an IOSurface and sends us the number that names it. We look that
// number up, wrap the surface in a CVPixelBuffer — a wrapper, not a copy — and
// register it with Flutter as an external texture. Flutter then draws that
// memory directly. No pixel is copied anywhere on this side.
//
// The `register` payload is the Windows one (`handle`, `width`, `height`),
// because the two are genuinely the same shape: one opaque integer naming a
// surface, plus its size. Here the integer is an IOSurfaceID.

/// One registered surface. `copyPixelBuffer` is called by the engine's raster
/// thread whenever it draws the texture, which is also how we count draws.
private final class LumitSurfaceTexture: NSObject, FlutterTexture {
  private let pixelBuffer: CVPixelBuffer
  private let counter = NSLock()
  private var drawn: Int = 0

  init?(surfaceID: IOSurfaceID) {
    // IOSurfaceLookup gives us a +1 reference to the same surface the engine
    // created in this process; the CVPixelBuffer then retains it.
    guard let surface = IOSurfaceLookup(surfaceID) else { return nil }
    var unmanaged: Unmanaged<CVPixelBuffer>?
    let status = CVPixelBufferCreateWithIOSurface(
      kCFAllocatorDefault, surface, nil, &unmanaged)
    guard status == kCVReturnSuccess, let unmanaged else { return nil }
    self.pixelBuffer = unmanaged.takeRetainedValue()
    super.init()
  }

  /// How many times Flutter has actually drawn this texture. Registration
  /// succeeding proves nothing — an embedder that cannot use the surface simply
  /// draws nothing and says nothing — so the Dart side watches this count and
  /// gives up on the path if it stays at zero.
  var drawCount: Int {
    counter.lock()
    defer { counter.unlock() }
    return drawn
  }

  func copyPixelBuffer() -> Unmanaged<CVPixelBuffer>? {
    counter.lock()
    drawn += 1
    counter.unlock()
    // The engine releases what it takes, so hand it a retained reference.
    return Unmanaged.passRetained(pixelBuffer)
  }
}

/// Owns the channel and the live textures for one Flutter engine.
final class ViewerTextureBridge {
  private let registry: FlutterTextureRegistry
  private let channel: FlutterMethodChannel
  private var textures: [Int64: LumitSurfaceTexture] = [:]

  /// Register the channel on `controller`'s engine. Held by the window for the
  /// engine's lifetime.
  init(controller: FlutterViewController) {
    // FlutterEngine itself conforms to FlutterTextureRegistry on macOS — there
    // is no separate registrar property to reach for, unlike the Windows and
    // Linux embedders.
    self.registry = controller.engine
    self.channel = FlutterMethodChannel(
      name: "lumit/viewer_texture",
      binaryMessenger: controller.engine.binaryMessenger)
    channel.setMethodCallHandler { [weak self] call, result in
      self?.handle(call, result: result)
    }
  }

  private func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    let args = call.arguments as? [String: Any]
    switch call.method {
    case "register":
      // The standard codec gives us NSNumber for every int, whatever its width.
      let handle = (args?["handle"] as? NSNumber)?.uint32Value ?? 0
      let width = (args?["width"] as? NSNumber)?.intValue ?? 0
      let height = (args?["height"] as? NSNumber)?.intValue ?? 0
      guard handle != 0, width > 0, height > 0 else {
        result(
          FlutterError(
            code: "bad_args", message: "register needs handle, width and height",
            details: nil))
        return
      }
      guard let texture = LumitSurfaceTexture(surfaceID: handle) else {
        result(
          FlutterError(
            code: "register_failed",
            message: "could not look up IOSurface \(handle)", details: nil))
        return
      }
      let id = registry.register(texture)
      textures[id] = texture
      result(id)
    case "frameReady":
      let id = (args?["textureId"] as? NSNumber)?.int64Value ?? 0
      guard let texture = textures[id] else {
        result(0)
        return
      }
      registry.textureFrameAvailable(id)
      // Answer with how many times Flutter has actually drawn this texture, so
      // the caller can tell "registered and working" from "registered and
      // silently never drawn" — the two look identical otherwise.
      result(texture.drawCount)
    case "unregister":
      let id = (args?["textureId"] as? NSNumber)?.int64Value ?? 0
      if textures.removeValue(forKey: id) != nil {
        registry.unregisterTexture(id)
      }
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}
