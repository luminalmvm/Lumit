import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  // The zero-copy Viewer bridge (K-195): holds the 'lumit/viewer_texture'
  // channel and the IOSurface textures registered on it for as long as the
  // window's engine lives. See windows/runner/flutter_window.cpp for the
  // sibling. Only the main window draws the Viewer, so only it needs one.
  private var viewerTextureBridge: ViewerTextureBridge?

  /// The name AppKit files this window's frame under in the user's defaults —
  /// its own machine-local store, which is why nothing here writes a file the
  /// way the Windows runner does.
  private static let savedFrameName = "LumitMainWindow"

  /// Pin the renderer to Skia, as the Windows runner does with its typed
  /// `set_impeller_switch` (K-748, extending K-732 to the other two platforms).
  ///
  /// Flutter's newer renderer, Impeller, is measurably slower for the way Lumit
  /// draws (docs/impl/ui-performance.md 2.4/4.1/7.2). AppKit's embedder has no
  /// API for the choice, so it is made the way `flutter run
  /// --no-enable-impeller` makes it on every desktop platform: an engine switch
  /// read out of the environment before the engine starts. Called immediately
  /// before `FlutterViewController()`, which is what starts it.
  ///
  /// **Appended, never overwritten.** `flutter run` fills these same variables
  /// with the switches a debug session needs — the VM service port among them —
  /// so replacing the count would leave the tool unable to attach.
  private static func pinSkia() {
    let already =
      Int(ProcessInfo.processInfo.environment["FLUTTER_ENGINE_SWITCHES"] ?? "") ?? 0
    let count = max(already, 0) + 1
    setenv("FLUTTER_ENGINE_SWITCH_\(count)", "enable-impeller=false", 1)
    setenv("FLUTTER_ENGINE_SWITCHES", String(count), 1)
  }

  override func awakeFromNib() {
    Self.pinSkia()
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)
    viewerTextureBridge = ViewerTextureBridge(controller: flutterViewController)

    // Open where the last run left the window, and zoomed — macOS's word for
    // maximised — when there is nothing to go back to. `setFrameUsingName`
    // says so by returning false, and AppKit pulls a restored frame back onto
    // a screen that still exists by itself, so an unplugged display needs no
    // code here the way it does on Windows. The one difference from the
    // Windows runner (windows/runner/win32_window.cpp) is that AppKit saves
    // the frame but not the zoomed state, so a window closed zoomed reopens at
    // the frame it would have un-zoomed to. Left there until a macOS user asks
    // for the rest.
    if !setFrameUsingName(Self.savedFrameName) {
      zoom(nil)
    }
    setFrameAutosaveName(Self.savedFrameName)

    super.awakeFromNib()
  }
}
