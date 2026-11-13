import Cocoa
import FlutterMacOS

class MainFlutterWindow: NSWindow {
  // The zero-copy Viewer bridge: holds the 'lumit/viewer_texture'
  // channel and the IOSurface textures registered on it for as long as the
  // window's engine lives. See windows/runner/flutter_window.cpp for the
  // sibling. Every window's views draw from the one engine, so one serves all.
  private var viewerTextureBridge: ViewerTextureBridge?

  /// The name AppKit files this window's frame under in the user's defaults —
  /// its own machine-local store, which is why nothing here writes a file the
  /// way the Windows runner does.
  private static let savedFrameName = "LumitMainWindow"

  /// Whether the window was zoomed, which the saved frame does not say.
  private static let savedZoomedName = "LumitMainWindowZoomed"

  /// The floating windows Dart has handed over (lib/shell/pop_out.dart), by
  /// handle, with what watches each one.
  private var adopted: [Int: [NSObjectProtocol]] = [:]

  /// Messages to Dart waiting for things to settle: by window handle, and 0
  /// for the displays.
  private var settling: [Int: DispatchWorkItem] = [:]

  /// The floating windows hidden with this one, front to back, to show again
  /// with it.
  private var hiddenFloating: [Int] = []

  /// Where each floating window last stood un-zoomed and not full screen,
  /// which AppKit does not say once it is either.
  private var normalFrames: [Int: NSRect] = [:]

  /// Quiet floating windows just shown, by handle, watched for a moment in
  /// case they take the keyboard again.
  private var quieting: [Int: NSObjectProtocol] = [:]

  /// The outline of the window a dragged panel would make, made once and
  /// kept, and the colour it was last drawn in.
  private var ghost: NSWindow?
  private var ghostColour: Int?

  /// Settings says closing the window keeps Lumit running in the menu bar
  /// (lib/state/background.dart).
  private var keepRunning = false

  /// Started at sign-in and not shown yet.
  private var startedHidden = false

  private var statusItem: NSStatusItem?
  private var windowChannel: FlutterMethodChannel?

  /// The start-up splash (lib/shell/splash_window.dart) while it is up: a
  /// window of its own, the shape of its picture.
  private var splash: NSWindow?

  /// Whether Dart has said start-up is over (`splashClose`). Until then this
  /// window is on screen but see-through, so Flutter keeps drawing into it
  /// while the splash is what shows.
  private var handedOver = false

  /// The style's see-through window (lib/state/window_backdrop.dart): the
  /// system's material under Flutter, which leaves its room clear over it.
  private var backdrop: NSVisualEffectView?

  /// "tint" is the desktop's colour, soft and still. "blur" is whatever is
  /// behind the window, blurred. Anything else is a solid window. Answers
  /// whether it is see-through, which it isn't with Reduce transparency on in
  /// System Settings.
  private func setBackdrop(_ args: [String: Any]) -> Bool {
    let kind = args["kind"] as? String ?? ""
    let on = (kind == "tint" || kind == "blur")
      && !NSWorkspace.shared.accessibilityDisplayShouldReduceTransparency
    guard let controller = contentViewController as? FlutterViewController,
      let content = contentView
    else { return false }
    if on {
      let effect = backdrop ?? NSVisualEffectView(frame: content.bounds)
      if backdrop == nil {
        effect.autoresizingMask = [.width, .height]
        effect.blendingMode = .behindWindow
        effect.state = .active
        content.superview?.addSubview(effect, positioned: .below, relativeTo: content)
        backdrop = effect
      }
      effect.frame = content.frame
      effect.material = kind == "tint" ? .underWindowBackground : .hudWindow
      effect.appearance = NSAppearance(
        named: args["dark"] as? Bool == false ? .aqua : .darkAqua)
      isOpaque = false
      backgroundColor = .clear
      controller.backgroundColor = .clear
    } else {
      backdrop?.removeFromSuperview()
      backdrop = nil
      isOpaque = true
      backgroundColor = .windowBackgroundColor
      controller.backgroundColor = nil
    }
    return on
  }

  /// Kept running in the background, closing the window only hides it, and
  /// the floating windows and the Dock icon go with it. Quit in the menu bar
  /// is an ordinary quit.
  ///
  /// Otherwise, with a floating window open, closing this one quits.
  /// FlutterAppDelegate asks Dart first, which asks about unsaved work, and
  /// the floating windows stay where they are, so the next start opens them
  /// there. Closing them one by one first would put their panels back in here.
  override func performClose(_ sender: Any?) {
    guard keepRunning else {
      let floatingOpen = NSApp.windows.contains {
        $0 !== self && ($0.isVisible || $0.isMiniaturized)
          && $0.contentViewController is FlutterViewController
      }
      if floatingOpen {
        NSApp.terminate(nil)
        return
      }
      return super.performClose(sender)
    }
    orderOut(nil)
    hideFloating()
    NSApp.setActivationPolicy(.accessory)
    windowChannel?.invokeMethod("hidden", arguments: nil)
  }

  /// Show the window and bring Lumit to the front, from the menu bar, the
  /// Dock or a second start.
  @objc func reveal() {
    startedHidden = false
    alphaValue = 1
    ignoresMouseEvents = false
    NSApp.setActivationPolicy(.regular)
    makeKeyAndOrderFront(nil)
    showFloating()
    NSApp.activate(ignoringOtherApps: true)
  }

  /// Hide the floating windows on screen with this one, in the menu bar or
  /// the Dock.
  private func hideFloating() {
    let shown = adoptedOnScreen
    shown.forEach { $0.orderOut(nil) }
    hiddenFloating += shown.map { Self.handle(of: $0) }
  }

  /// Bring them back above this one, without the keyboard.
  private func showFloating() {
    for handle in hiddenFloating {
      floatingWindow(handle)?.order(.above, relativeTo: windowNumber)
    }
    hiddenFloating = []
  }

  /// A window's handle as Dart has it: the NSWindow's address
  /// (lib/shell/pop_out_handle.dart).
  private static func handle(of window: NSWindow) -> Int {
    Int(bitPattern: Unmanaged.passUnretained(window).toOpaque())
  }

  /// The floating window Dart names by `handle`, or nil once it has gone. The
  /// handle is only ever compared with the windows there are, never followed.
  private func floatingWindow(_ handle: Int) -> NSWindow? {
    NSApp.windows.first { $0 !== self && Self.handle(of: $0) == handle }
  }

  /// The adopted floating windows on screen on any Space, front to back.
  private var adoptedOnScreen: [NSWindow] {
    (NSWindow.windowNumbers(options: .allSpaces) ?? [])
      .compactMap { NSApp.window(withWindowNumber: $0.intValue) }
      .filter { adopted[Self.handle(of: $0)] != nil }
  }

  /// A rectangle as Dart has it, `[x, y, width, height]` down from the top
  /// of the primary display (lib/shell/pop_out.dart). AppKit's run up from
  /// its bottom.
  private static func flipped(_ rect: NSRect) -> [Double] {
    let top = NSScreen.screens.first?.frame.maxY ?? 0
    return [rect.minX, top - rect.maxY, rect.width, rect.height].map { Double($0) }
  }

  /// One of Dart's rectangles as AppKit's.
  private static func unflipped(_ raw: Any?) -> NSRect? {
    guard let n = raw as? [NSNumber], n.count == 4 else { return nil }
    let top = NSScreen.screens.first?.frame.maxY ?? 0
    let height = CGFloat(n[3].doubleValue)
    return NSRect(
      x: CGFloat(n[0].doubleValue), y: top - CGFloat(n[1].doubleValue) - height,
      width: CGFloat(n[2].doubleValue), height: height)
  }

  /// Do `act` once nothing has changed for `delay` seconds, so a drag across
  /// the screen is one message to Dart, not a hundred.
  private func whenStill(_ key: Int, after delay: Double, _ act: @escaping () -> Void) {
    settling[key]?.cancel()
    let work = DispatchWorkItem { [weak self] in
      self?.settling[key] = nil
      act()
    }
    settling[key] = work
    DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: work)
  }

  /// The engine makes each window key as it shows it, which a window reopened
  /// at start or a video monitor must not keep. The keyboard comes back here
  /// now, and once more if the window takes it again in the next second,
  /// before its first frame.
  private func quieten(_ floating: NSWindow, _ handle: Int) {
    guard isVisible else { return }
    makeKey()
    guard quieting[handle] == nil else { return }
    quieting[handle] = NotificationCenter.default.addObserver(
      forName: NSWindow.didBecomeKeyNotification, object: floating, queue: .main
    ) { [weak self] _ in
      guard let self = self, let watch = self.quieting.removeValue(forKey: handle) else { return }
      NotificationCenter.default.removeObserver(watch)
      self.makeKey()
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
      if let watch = self?.quieting.removeValue(forKey: handle) {
        NotificationCenter.default.removeObserver(watch)
      }
    }
  }

  /// Floating windows and displays (lib/shell/pop_out.dart). A call may name
  /// a floating window by its handle, and one naming a window that has gone
  /// answers false. A call naming none is about this window.
  private func answerWindows(_ call: FlutterMethodCall, _ result: FlutterResult) {
    let args = call.arguments as? [String: Any] ?? [:]
    let handle = args["window"] as? Int
    let floating = handle.flatMap { floatingWindow($0) }
    if handle != nil && floating == nil {
      return result(false)
    }
    let window = floating ?? self
    switch call.method {
    case "adopt":
      if let floating = floating, let handle = handle {
        floating.level = args["onTop"] as? Bool == true ? .floating : .normal
        // It can sit beside this window when this one is full screen.
        floating.collectionBehavior.insert(.fullScreenAuxiliary)
        if adopted[handle] == nil {
          if !floating.isZoomed && !floating.styleMask.contains(.fullScreen) {
            normalFrames[handle] = floating.frame
          }
          let center = NotificationCenter.default
          var watching: [NSObjectProtocol] = []
          for name in [NSWindow.didMoveNotification, NSWindow.didResizeNotification] {
            watching.append(
              center.addObserver(forName: name, object: floating, queue: .main) {
                [weak self, weak floating] _ in
                self?.whenStill(handle, after: 0.3) {
                  guard let self = self, let floating = floating else { return }
                  // Kept once still, so never a frame from partway through
                  // zooming or going full screen.
                  if !floating.isZoomed && !floating.styleMask.contains(.fullScreen) {
                    self.normalFrames[handle] = floating.frame
                  }
                  self.windowChannel?.invokeMethod("windowMoved", arguments: ["window": handle])
                }
              })
          }
          watching.append(
            center.addObserver(
              forName: NSWindow.willCloseNotification, object: floating, queue: .main
            ) { [weak self] _ in
              guard let self = self else { return }
              self.adopted.removeValue(forKey: handle)?.forEach {
                NotificationCenter.default.removeObserver($0)
              }
              self.settling.removeValue(forKey: handle)?.cancel()
              self.hiddenFloating.removeAll { $0 == handle }
              self.normalFrames[handle] = nil
            })
          adopted[handle] = watching
        }
        if args["quiet"] as? Bool == true {
          quieten(floating, handle)
        }
      }
      result(nil)
    case "frame":
      // Zoomed or full screen, a floating window answers where it last stood
      // as neither. The main window has none kept, so it answers its frame.
      let fullScreen = window.styleMask.contains(.fullScreen)
      var bounds = window.frame
      if window.isZoomed || fullScreen, let handle = handle, let normal = normalFrames[handle] {
        bounds = normal
      }
      var answer: [String: Any] = [
        "bounds": Self.flipped(bounds),
        "maximised": window.isZoomed,
        "minimised": window.isMiniaturized,
        "fullScreen": fullScreen,
        "client": Self.flipped(window.contentRect(forFrameRect: window.frame)),
        "scale": 1.0,
      ]
      if let screen = window.screen {
        answer["display"] = Self.flipped(screen.frame)
        answer["work"] = Self.flipped(screen.visibleFrame)
      }
      result(answer)
    case "setFrame":
      // Placed as it is un-zoomed, then zoomed or not. Nothing here makes it
      // key, so the keyboard stays where it was.
      if let floating = floating, let handle = handle,
        let bounds = Self.unflipped(args["bounds"])
      {
        let zoomed = args["maximised"] as? Bool == true
        if floating.isZoomed && !zoomed {
          floating.zoom(nil)
        }
        floating.setFrame(bounds, display: true)
        normalFrames[handle] = bounds
        if zoomed && !floating.isZoomed {
          floating.zoom(nil)
        }
      }
      result(nil)
    case "onTop":
      floating?.level = args["on"] as? Bool == true ? .floating : .normal
      result(nil)
    case "monitor":
      // A video monitor: no frame, over the display's menu bar, on every
      // Space and beside full screen. Borderless, it cannot become key again,
      // and the keyboard the engine gave it as it showed comes back here.
      if let floating = floating, let handle = handle,
        let display = Self.unflipped(args["display"])
      {
        floating.styleMask = [.borderless]
        floating.level = .statusBar
        floating.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        floating.hasShadow = false
        floating.setFrame(display, display: true)
        floating.orderFrontRegardless()
        quieten(floating, handle)
      }
      result(nil)
    case "displays":
      result(
        NSScreen.screens.enumerated().map { index, screen in
          [
            "name": screen.localizedName,
            "bounds": Self.flipped(screen.frame),
            "work": Self.flipped(screen.visibleFrame),
            "scale": 1.0,
            "primary": index == 0,
          ] as [String: Any]
        })
    case "ghost":
      // Above everything, and the pointer and the keyboard pass straight
      // through it. The layer draws the fill and the edge at any size.
      guard let rect = Self.unflipped(args["rect"]) else {
        ghost?.orderOut(nil)
        return result(nil)
      }
      let outline =
        ghost
        ?? NSWindow(contentRect: rect, styleMask: .borderless, backing: .buffered, defer: false)
      if ghost == nil {
        outline.isReleasedWhenClosed = false
        outline.ignoresMouseEvents = true
        outline.isOpaque = false
        outline.backgroundColor = .clear
        outline.hasShadow = false
        outline.level = .popUpMenu
        outline.collectionBehavior = [.canJoinAllSpaces, .transient, .ignoresCycle]
        outline.contentView?.wantsLayer = true
        ghost = outline
      }
      let colour = args["colour"] as? Int ?? 0
      if colour != ghostColour, let layer = outline.contentView?.layer {
        // 0xAARRGGBB, drawn as the Windows runner draws it: the edge solid
        // and the fill at about 18%, whatever the colour's own alpha.
        let edge = NSColor(
          srgbRed: CGFloat((colour >> 16) & 0xFF) / 255,
          green: CGFloat((colour >> 8) & 0xFF) / 255,
          blue: CGFloat(colour & 0xFF) / 255, alpha: 1)
        layer.borderWidth = 2
        layer.borderColor = edge.cgColor
        layer.backgroundColor = edge.withAlphaComponent(0.18).cgColor
        ghostColour = colour
      }
      if outline.frame != rect {
        outline.setFrame(rect, display: true)
      }
      outline.orderFrontRegardless()
      result(nil)
    case "minimise":
      window.miniaturize(nil)
      result(window.isZoomed)
    case "maximise":
      window.zoom(nil)
      result(window.isZoomed)
    case "close":
      // A floating window's close button asks the engine's delegate, which
      // asks Dart and says no. performClose would beep at that no, and at a
      // monitor, which has no close button, so the delegate is asked here.
      if floating != nil {
        _ = window.delegate?.windowShouldClose?(window)
      } else {
        performClose(nil)
      }
      result(window.isZoomed)
    case "drag", "state", "maximiseButton":
      // The system's title bar stays on macOS, and moves the window itself.
      result(window.isZoomed)
    case "beginResize":
      result(nil)
    default:
      result(FlutterMethodNotImplemented)
    }
  }

  /// The primary screen, which the splash is centred on.
  private var splashScreen: NSScreen? { NSScreen.screens.first ?? NSScreen.main }

  /// Put the splash on screen, or give it new pixels: premultiplied RGBA,
  /// "width" by "height", in "bytes". False if it could not.
  private func showSplash(_ args: [String: Any]) -> Bool {
    guard !startedHidden, !handedOver,
      let width = args["width"] as? Int, let height = args["height"] as? Int,
      width > 0, height > 0,
      let data = (args["bytes"] as? FlutterStandardTypedData)?.data,
      data.count >= width * height * 4,
      let provider = CGDataProvider(data: data as CFData),
      let space = CGColorSpace(name: CGColorSpace.sRGB),
      let image = CGImage(
        width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32,
        bytesPerRow: width * 4, space: space,
        bitmapInfo: CGBitmapInfo(
          rawValue: CGImageAlphaInfo.premultipliedLast.rawValue
            | CGBitmapInfo.byteOrder32Big.rawValue),
        provider: provider, decode: nil, shouldInterpolate: true,
        intent: .defaultIntent)
    else { return false }
    let scale = splashScreen?.backingScaleFactor ?? 1
    let size = NSSize(width: CGFloat(width) / scale, height: CGFloat(height) / scale)
    let window: NSWindow
    if let up = splash {
      window = up
    } else {
      window = NSWindow(
        contentRect: NSRect(origin: .zero, size: size), styleMask: .borderless,
        backing: .buffered, defer: false)
      window.isOpaque = false
      window.backgroundColor = .clear
      window.hasShadow = false
      window.isReleasedWhenClosed = false
      window.isMovableByWindowBackground = true
      let view = NSView(frame: NSRect(origin: .zero, size: size))
      view.wantsLayer = true
      window.contentView = view
      if let frame = splashScreen?.visibleFrame {
        window.setFrameOrigin(
          NSPoint(x: frame.midX - size.width / 2, y: frame.midY - size.height / 2))
      }
      splash = window
    }
    window.contentView?.layer?.contentsScale = scale
    window.contentView?.layer?.contents = image
    if !window.isVisible {
      window.orderFrontRegardless()
    }
    return true
  }

  /// Take the splash down and show this window in its place.
  private func handOver() {
    handedOver = true
    splash?.orderOut(nil)
    splash = nil
    if !startedHidden {
      reveal()
    }
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

    // macOS draws the window's buttons itself, so this channel carries what
    // Settings says about running in the background, Dart asking for the
    // window back, and the floating windows and displays.
    let window = FlutterMethodChannel(
      name: "lumit/window",
      binaryMessenger: flutterViewController.engine.binaryMessenger)
    window.setMethodCallHandler { [weak self] call, result in
      switch call.method {
      case "splashScale":
        let hidden = self?.startedHidden ?? true
        result(hidden ? nil : Double(self?.splashScreen?.backingScaleFactor ?? 1))
        return
      case "splash":
        result(self?.showSplash(call.arguments as? [String: Any] ?? [:]) ?? false)
        return
      case "splashClose":
        self?.handOver()
      case "background":
        self?.setBackground(call.arguments as? [String: Any] ?? [:])
      case "show":
        self?.reveal()
      case "close":
        self?.performClose(nil)
      case "backdrop":
        result(self?.setBackdrop(call.arguments as? [String: Any] ?? [:]) ?? false)
        return
      default:
        if let self = self {
          self.answerWindows(call, result)
        } else {
          result(nil)
        }
        return
      }
      result(nil)
    }
    windowChannel = window
    // The floating windows stay above this one, as owned windows do on
    // Windows: each time this one comes forward, they are put back in front
    // of it, in the order they were in, without taking the keyboard.
    for name in [NSWindow.didBecomeKeyNotification, NSWindow.didBecomeMainNotification] {
      NotificationCenter.default.addObserver(forName: name, object: self, queue: .main) {
        [weak self] _ in
        guard let self = self else { return }
        for floating in self.adoptedOnScreen where floating.isOnActiveSpace {
          floating.order(.above, relativeTo: self.windowNumber)
        }
      }
    }
    // Minimised, it takes the floating windows into the Dock with it.
    NotificationCenter.default.addObserver(
      forName: NSWindow.didMiniaturizeNotification, object: self, queue: .main
    ) { [weak self] _ in
      self?.hideFloating()
    }
    NotificationCenter.default.addObserver(
      forName: NSWindow.didDeminiaturizeNotification, object: self, queue: .main
    ) { [weak self] _ in
      self?.showFloating()
    }
    // A display plugged in, unplugged or rearranged, told once it settles.
    NotificationCenter.default.addObserver(
      forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main
    ) { [weak self] _ in
      self?.whenStill(0, after: 0.5) {
        self?.windowChannel?.invokeMethod("displaysChanged", arguments: nil)
      }
    }
    // Reduce transparency switched in System Settings: Dart asks for the
    // backdrop again and learns whether it shows.
    NSWorkspace.shared.notificationCenter.addObserver(
      forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification,
      object: nil, queue: .main
    ) { [weak self] _ in
      self?.windowChannel?.invokeMethod("backdrop", arguments: nil)
    }

    // Started at sign-in, it waits in the menu bar with no window and no Dock
    // icon until it is opened.
    if CommandLine.arguments.contains("--background") {
      startedHidden = true
      NSApp.setActivationPolicy(.accessory)
      DispatchQueue.main.async { self.orderOut(nil) }
    }

    // See-through until Dart hands over, which it does once the first frame is
    // drawn, splash or no splash. On screen rather than hidden, so Flutter has
    // a window to draw that frame into. If start-up never gets that far, the
    // window shows anyway after ten seconds, as the Windows runner's does, so
    // there is something to close.
    alphaValue = 0
    ignoresMouseEvents = true
    DispatchQueue.main.asyncAfter(deadline: .now() + 10) { [weak self] in
      guard let self = self, !self.handedOver, self.splash == nil, !self.startedHidden
      else { return }
      self.reveal()
    }

    // Open where the last run left the window, and zoomed — macOS's word for
    // maximised — when there is nothing to go back to. `setFrameUsingName`
    // says so by returning false, and AppKit pulls a restored frame back onto
    // a screen that still exists by itself, so an unplugged display needs no
    // code here the way it does on Windows. AppKit saves the frame but not
    // whether it was zoomed, so that is kept beside it each time the window
    // changes size, and a window closed zoomed opens zoomed, as it does on
    // Windows (windows/runner/win32_window.cpp).
    if !setFrameUsingName(Self.savedFrameName)
      || (UserDefaults.standard.bool(forKey: Self.savedZoomedName) && !isZoomed)
    {
      zoom(nil)
    }
    setFrameAutosaveName(Self.savedFrameName)
    // Full screen is not zoomed or un-zoomed, so it is left out.
    NotificationCenter.default.addObserver(
      forName: NSWindow.didResizeNotification, object: self, queue: .main
    ) { [weak self] _ in
      guard let self = self, !self.styleMask.contains(.fullScreen) else { return }
      UserDefaults.standard.set(self.isZoomed, forKey: Self.savedZoomedName)
    }

    super.awakeFromNib()
  }
}
