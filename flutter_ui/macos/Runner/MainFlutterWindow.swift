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

  /// Settings says closing the window keeps Lumit running in the menu bar
  /// (lib/state/background.dart).
  private var keepRunning = false

  /// Started at sign-in and not shown yet.
  private var startedHidden = false

  private var statusItem: NSStatusItem?
  private var windowChannel: FlutterMethodChannel?

  /// Kept running in the background, closing the window only hides it, and
  /// the Dock icon goes with it. Quit in the menu bar is an ordinary quit.
  override func performClose(_ sender: Any?) {
    guard keepRunning else { return super.performClose(sender) }
    orderOut(nil)
    NSApp.setActivationPolicy(.accessory)
    windowChannel?.invokeMethod("hidden", arguments: nil)
  }

  /// Show the window and bring Lumit to the front, from the menu bar, the
  /// Dock or a second start.
  @objc func reveal() {
    startedHidden = false
    NSApp.setActivationPolicy(.regular)
    makeKeyAndOrderFront(nil)
    NSApp.activate(ignoringOtherApps: true)
  }

  /// Start Lumit in the menu bar when the person signs in, with a LaunchAgent
  /// that passes --background, or stop doing so.
  private func setOpenAtSignIn(_ on: Bool) {
    let agents = FileManager.default.homeDirectoryForCurrentUser
      .appendingPathComponent("Library/LaunchAgents")
    let label = Bundle.main.bundleIdentifier ?? "io.github.luminalmvm.lumitFlutter"
    let plist = agents.appendingPathComponent("\(label).plist")
    guard on, let exe = Bundle.main.executablePath else {
      try? FileManager.default.removeItem(at: plist)
      return
    }
    try? FileManager.default.createDirectory(at: agents, withIntermediateDirectories: true)
    let agent: NSDictionary = [
      "Label": label, "ProgramArguments": [exe, "--background"], "RunAtLoad": true,
    ]
    try? agent.write(to: plist)
  }

  private func setBackground(_ args: [String: Any]) {
    keepRunning = args["keep"] as? Bool ?? false
    setOpenAtSignIn(keepRunning && args["signIn"] as? Bool == true)
    guard keepRunning else {
      if let item = statusItem {
        NSStatusBar.system.removeStatusItem(item)
        statusItem = nil
      }
      // Started at sign-in with nothing now to come back from: the window is
      // the only way back, so it shows.
      if startedHidden { reveal() }
      return
    }
    let item = statusItem ?? NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    if let icon = NSApp.applicationIconImage.copy() as? NSImage {
      icon.size = NSSize(width: 18, height: 18)
      item.button?.image = icon
    }
    let menu = NSMenu()
    menu.addItem(withTitle: args["open"] as? String ?? "", action: #selector(reveal), keyEquivalent: "")
      .target = self
    menu.addItem(
      withTitle: args["quit"] as? String ?? "",
      action: #selector(NSApplication.terminate(_:)), keyEquivalent: ""
    ).target = NSApp
    item.menu = menu
    statusItem = item
  }

  override func awakeFromNib() {
    let flutterViewController = FlutterViewController()
    let windowFrame = self.frame
    self.contentViewController = flutterViewController
    self.setFrame(windowFrame, display: true)

    // The smallest it can be made, as on Windows (windows/runner/
    // win32_window.cpp): room for the default workspace's narrowest panels.
    self.contentMinSize = NSSize(width: 840, height: 560)

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

    // macOS draws the window's buttons itself, so this channel only carries
    // what Settings says about running in the background, and Dart asking
    // for the window back.
    let window = FlutterMethodChannel(
      name: "lumit/window",
      binaryMessenger: flutterViewController.engine.binaryMessenger)
    window.setMethodCallHandler { [weak self] call, result in
      switch call.method {
      case "background":
        self?.setBackground(call.arguments as? [String: Any] ?? [:])
      case "show":
        self?.reveal()
      default:
        result(FlutterMethodNotImplemented)
        return
      }
      result(nil)
    }
    windowChannel = window

    // Started at sign-in, it waits in the menu bar with no window and no Dock
    // icon until it is opened.
    if CommandLine.arguments.contains("--background") {
      startedHidden = true
      NSApp.setActivationPolicy(.accessory)
      DispatchQueue.main.async { self.orderOut(nil) }
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
