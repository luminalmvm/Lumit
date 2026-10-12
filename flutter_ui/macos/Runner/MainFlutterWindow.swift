import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  // The zero-copy Viewer bridge: holds the 'lumit/viewer_texture'
  // channel and the IOSurface textures registered on it for as long as the
  // window's engine lives. See windows/runner/flutter_window.cpp for the
  // sibling. Only the main window draws the Viewer, so only it needs one.
  private var viewerTextureBridge: ViewerTextureBridge?

  /// The name AppKit files this window's frame under in the user's defaults —
  /// its own machine-local store, which is why nothing here writes a file the
  /// way the Windows runner does.
  private static let savedFrameName = "LumitMainWindow"

  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)
    viewerTextureBridge = ViewerTextureBridge(controller: flutterViewController)
    // A double-clicked project or a clicked invite link reaches Dart through
    // the application delegate, which needs this engine to say so on.
    (NSApp.delegate as? AppDelegate)?.opens.attach(to: flutterViewController)
    // An extension's page is a web view Flutter places in its panel.
    ExtensionViewFactory.register(with: flutterViewController)
    // A nightly may wear the regular icon (lib/state/app_icon.dart): a path
    // shows that picture in the Dock, and none puts the bundle's own back. It
    // lasts as long as Lumit runs; the Finder always draws the bundle's.
    FlutterMethodChannel(
      name: "lumit/app_icon",
      binaryMessenger: flutterViewController.engine.binaryMessenger
    ).setMethodCallHandler { call, result in
      NSApp.applicationIconImage = (call.arguments as? String).flatMap {
        NSImage(contentsOfFile: $0)
      }
      result(nil)
    }

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
