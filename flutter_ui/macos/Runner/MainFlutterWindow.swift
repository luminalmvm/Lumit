import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  // The zero-copy Viewer bridge (K-195): holds the 'lumit/viewer_texture'
  // channel and the IOSurface textures registered on it for as long as the
  // window's engine lives. See windows/runner/flutter_window.cpp for the
  // sibling. Only the main window draws the Viewer, so only it needs one.
  private var viewerTextureBridge: ViewerTextureBridge?

  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    RegisterGeneratedPlugins(registry: flutterViewController)
    viewerTextureBridge = ViewerTextureBridge(controller: flutterViewController)

    super.awakeFromNib()
  }
}
