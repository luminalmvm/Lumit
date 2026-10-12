import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  /// What macOS has asked Lumit to open, kept until Dart takes it.
  let opens = OpenRequests()

  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return true
  }

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }

  // A project double-clicked in Finder and a lumit: link clicked in a browser
  // both arrive here, as events: macOS puts neither on the command line the
  // way Windows and Linux do. That is so whether the click started Lumit or
  // found it running.
  override func application(_ application: NSApplication, open urls: [URL]) {
    opens.open(urls)
  }
}

/// Carries what macOS asks Lumit to open across to Dart.
///
/// Each request waits here until Dart takes it. Dart takes once as it starts,
/// and again whenever it is told there is something: the "open" message says
/// only that, and the answer to "take" is the requests themselves. Asked for
/// that way, none is lost to arriving at a moment nothing was listening.
///
/// The request Lumit was started by is the one that needs care. macOS delivers
/// it while the application is still launching, after the main queue has
/// already had a turn, so Dart can ask before it has come. An answer is
/// therefore held until launching has finished, which macOS says only once
/// that request has been handed over. Dart then knows at start-up whether it
/// was started to open something, as it does from its arguments elsewhere.
///
/// Everything here runs on the main thread, where AppKit delivers the events
/// and Flutter delivers the channel's calls.
final class OpenRequests {
  private var channel: FlutterMethodChannel?
  private var waiting: [String] = []
  private var launched = false

  /// Dart's "take", asked before launching had finished.
  private var asked: FlutterResult?

  init() {
    // Not the delegate's own method, which would have to pass the call up to
    // a FlutterAppDelegate that may or may not answer it.
    NotificationCenter.default.addObserver(
      forName: NSApplication.didFinishLaunchingNotification, object: nil, queue: nil
    ) { [weak self] _ in
      guard let self = self else { return }
      self.launched = true
      if let asked = self.asked {
        self.asked = nil
        self.take(asked)
      }
    }
  }

  /// Answer Dart on `controller`'s engine. Called by the window as it makes
  /// the engine, which is before any request can arrive.
  func attach(to controller: FlutterViewController) {
    let channel = FlutterMethodChannel(
      name: "lumit/open",
      binaryMessenger: controller.engine.binaryMessenger)
    channel.setMethodCallHandler { [weak self] call, result in
      guard let self = self, call.method == "take" else {
        result(FlutterMethodNotImplemented)
        return
      }
      if self.launched {
        self.take(result)
      } else {
        self.asked = result
      }
    }
    self.channel = channel
  }

  private func take(_ result: FlutterResult) {
    result(waiting)
    waiting = []
  }

  func open(_ urls: [URL]) {
    // A file as its path and a link as it was written, which is what each
    // would have been as an argument on another system.
    waiting += urls.map { $0.isFileURL ? $0.path : $0.absoluteString }
    channel?.invokeMethod("open", arguments: nil)
  }
}
