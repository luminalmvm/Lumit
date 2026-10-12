import Cocoa
import FlutterMacOS
import WebKit

// The macOS half of extension panels: the page of an extension, shown in a
// WKWebView that Flutter places among its own widgets. The siblings are
// `linux/runner/extension_views.cc` and, on Windows, the webview_windows
// plugin, and lib/panels/extension_page.dart is the Dart side of all three.
//
// A view is made when Flutter asks for the view type 'lumit/extension_view',
// and talks on a channel of its own, 'lumit/extension_view_<number>', which
// takes `load`, `run` and `unfocus` and sends `message` and `url`.
//
// The page's files are served from the extension's folder under
// lumit-extension://<id>/, and only to a page that is itself from there.
// What the page asks of Lumit arrives as text through the one message
// handler WebKit gives every frame, so only what the top frame of the
// extension's own page sent is passed on. What the messages mean is for the
// Dart side: nothing here reads them.

/// The scheme an extension's own files are served under. The Dart side
/// (extension_host.dart, extensionScheme) names the same one.
private let extensionScheme = "lumit-extension"

/// What a file is, by the end of its name, for the ones a page is made of.
/// Anything else is asked of the system.
private let mimeTypes: [String: String] = [
  "html": "text/html", "htm": "text/html", "css": "text/css",
  "js": "text/javascript", "mjs": "text/javascript", "json": "application/json",
  "map": "application/json", "wasm": "application/wasm", "svg": "image/svg+xml",
  "png": "image/png", "jpg": "image/jpeg", "jpeg": "image/jpeg", "gif": "image/gif",
  "webp": "image/webp", "avif": "image/avif", "ico": "image/x-icon",
  "woff": "font/woff", "woff2": "font/woff2", "ttf": "font/ttf", "otf": "font/otf",
  "mp4": "video/mp4", "webm": "video/webm", "mp3": "audio/mpeg", "wav": "audio/wav",
  "ogg": "audio/ogg", "txt": "text/plain", "xml": "application/xml",
]

/// Serves one extension's folder to its own page, and to nothing else.
private final class ExtensionFiles: NSObject, WKURLSchemeHandler {
  private let id: String
  private let folder: URL

  init(id: String, folder: String) {
    self.id = id
    self.folder = URL(fileURLWithPath: folder, isDirectory: true).resolvingSymlinksInPath()
  }

  /// The file `url` names, if it is one inside the folder.
  private func file(for url: URL) -> URL? {
    let path = url.path
    guard path.count > 1 else { return nil }
    let file = folder.appendingPathComponent(String(path.dropFirst())).resolvingSymlinksInPath()
    guard file.path.hasPrefix(folder.path + "/") else { return nil }
    var directory: ObjCBool = false
    guard FileManager.default.fileExists(atPath: file.path, isDirectory: &directory),
      !directory.boolValue
    else { return nil }
    return file
  }

  /// The bytes a `Range` header asks for out of `count`, or nil for one that
  /// asks for something else.
  private func span(_ header: String, of count: Int) -> Range<Int>? {
    guard header.hasPrefix("bytes="), count > 0 else { return nil }
    let ends = header.dropFirst(6).split(separator: "-", omittingEmptySubsequences: false)
    guard ends.count == 2 else { return nil }
    if ends[0].isEmpty {
      // The last so many bytes.
      guard let last = Int(ends[1]), last > 0 else { return nil }
      return max(0, count - last)..<count
    }
    guard let first = Int(ends[0]), first < count else { return nil }
    let last = ends[1].isEmpty ? count - 1 : min(Int(ends[1]) ?? -1, count - 1)
    guard last >= first else { return nil }
    return first..<(last + 1)
  }

  func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
    let request = task.request
    // Only for a page of the extension's own: one that has gone somewhere
    // else cannot read its files.
    guard let url = request.url, url.host == id,
      let page = request.mainDocumentURL, page.scheme == extensionScheme, page.host == id,
      let file = file(for: url),
      let data = try? Data(contentsOf: file, options: .mappedIfSafe)
    else {
      task.didFailWithError(URLError(.fileDoesNotExist))
      return
    }
    var status = 200
    var body = data
    var headers = [
      "Content-Type": mimeTypes[file.pathExtension.lowercased()] ?? "application/octet-stream",
      "Accept-Ranges": "bytes",
      "Cache-Control": "no-cache",
    ]
    // Sound and video are read a piece at a time.
    if let range = request.value(forHTTPHeaderField: "Range"),
      let span = span(range, of: data.count)
    {
      status = 206
      body = data.subdata(in: span)
      headers["Content-Range"] = "bytes \(span.lowerBound)-\(span.upperBound - 1)/\(data.count)"
    }
    headers["Content-Length"] = String(body.count)
    guard
      let response = HTTPURLResponse(
        url: url, statusCode: status, httpVersion: "HTTP/1.1", headerFields: headers)
    else {
      task.didFailWithError(URLError(.cannotParseResponse))
      return
    }
    task.didReceive(response)
    task.didReceive(body)
    task.didFinish()
  }

  // Every request is answered before `start` returns, so there is never one
  // left to stop.
  func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}
}

/// Passes a page's messages on without keeping the view alive, which the
/// page's own controller would do if the view were its handler.
private final class ExtensionMessages: NSObject, WKScriptMessageHandler {
  weak var view: ExtensionWebView?

  func userContentController(
    _ controller: WKUserContentController, didReceive message: WKScriptMessage
  ) {
    view?.received(message)
  }
}

/// One extension's page.
private final class ExtensionWebView: WKWebView, WKUIDelegate {
  private var extensionId = ""
  private var channel: FlutterMethodChannel?
  private var watching: NSKeyValueObservation?
  private weak var flutter: FlutterViewController?

  static func make(
    number: Int64, arguments: [String: Any], messenger: FlutterBinaryMessenger,
    flutter: FlutterViewController?
  ) -> ExtensionWebView {
    let id = arguments["id"] as? String ?? ""
    let configuration = WKWebViewConfiguration()
    configuration.setURLSchemeHandler(
      ExtensionFiles(id: id, folder: arguments["folder"] as? String ?? ""),
      forURLScheme: extensionScheme)
    // `window.lumit`, before any script of the page's own, and for the top
    // frame alone.
    configuration.userContentController.addUserScript(
      WKUserScript(
        source: arguments["bootstrap"] as? String ?? "",
        injectionTime: .atDocumentStart, forMainFrameOnly: true))
    let messages = ExtensionMessages()
    configuration.userContentController.add(messages, name: "lumit")

    let view = ExtensionWebView(frame: .zero, configuration: configuration)
    messages.view = view
    view.extensionId = id
    view.flutter = flutter
    view.uiDelegate = view
    if #available(macOS 13.3, *) {
      view.isInspectable = arguments["inspect"] as? Bool ?? false
    }

    let channel = FlutterMethodChannel(
      name: "lumit/extension_view_\(number)", binaryMessenger: messenger)
    channel.setMethodCallHandler { [weak view] call, result in
      view?.handle(call)
      result(nil)
    }
    view.channel = channel
    view.watching = view.observe(\.url, options: [.new]) { view, _ in
      if let url = view.url {
        view.channel?.invokeMethod("url", arguments: url.absoluteString)
      }
    }
    return view
  }

  deinit {
    channel?.setMethodCallHandler(nil)
  }

  private func handle(_ call: FlutterMethodCall) {
    switch call.method {
    case "load":
      if let text = call.arguments as? String, let url = URL(string: text) {
        load(URLRequest(url: url))
      }
    case "run":
      if let script = call.arguments as? String {
        evaluateJavaScript(script, completionHandler: nil)
      }
    case "unfocus":
      unfocus()
    default:
      break
    }
  }

  /// A message from the page. WebKit gives the same handler to every frame,
  /// so only the top frame of the extension's own page is listened to.
  fileprivate func received(_ message: WKScriptMessage) {
    let origin = message.frameInfo.securityOrigin
    guard message.frameInfo.isMainFrame, origin.`protocol` == extensionScheme,
      origin.host == extensionId, let text = message.body as? String
    else { return }
    channel?.invokeMethod("message", arguments: text)
  }

  /// Give the keyboard back to Flutter. A page that was typed in keeps it
  /// when the press that follows lands on Flutter's own view.
  private func unfocus() {
    guard let window, let holder = window.firstResponder as? NSView,
      holder.isDescendant(of: self)
    else { return }
    // The controller's view wraps the one Flutter draws in and takes keys
    // with.
    let host = flutter?.view
    window.makeFirstResponder(host?.subviews.first { $0.acceptsFirstResponder } ?? host)
  }

  // MARK: The page's own dialogues, which WebKit leaves to the application.

  /// A word AppKit already has in the person's language.
  private func word(_ key: String) -> String {
    Bundle(for: NSApplication.self).localizedString(forKey: key, value: key, table: nil)
  }

  private func ask(_ alert: NSAlert, then: @escaping (Bool) -> Void) {
    if let window {
      alert.beginSheetModal(for: window) { then($0 == .alertFirstButtonReturn) }
    } else {
      then(alert.runModal() == .alertFirstButtonReturn)
    }
  }

  func webView(
    _ webView: WKWebView, runJavaScriptAlertPanelWithMessage message: String,
    initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping () -> Void
  ) {
    let alert = NSAlert()
    alert.messageText = message
    alert.addButton(withTitle: word("OK"))
    ask(alert) { _ in completionHandler() }
  }

  func webView(
    _ webView: WKWebView, runJavaScriptConfirmPanelWithMessage message: String,
    initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping (Bool) -> Void
  ) {
    let alert = NSAlert()
    alert.messageText = message
    alert.addButton(withTitle: word("OK"))
    alert.addButton(withTitle: word("Cancel"))
    ask(alert, then: completionHandler)
  }

  func webView(
    _ webView: WKWebView, runJavaScriptTextInputPanelWithPrompt prompt: String,
    defaultText: String?, initiatedByFrame frame: WKFrameInfo,
    completionHandler: @escaping (String?) -> Void
  ) {
    let alert = NSAlert()
    alert.messageText = prompt
    alert.addButton(withTitle: word("OK"))
    alert.addButton(withTitle: word("Cancel"))
    let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
    field.stringValue = defaultText ?? ""
    alert.accessoryView = field
    ask(alert) { completionHandler($0 ? field.stringValue : nil) }
  }

  func webView(
    _ webView: WKWebView, runOpenPanelWith parameters: WKOpenPanelParameters,
    initiatedByFrame frame: WKFrameInfo, completionHandler: @escaping ([URL]?) -> Void
  ) {
    let panel = NSOpenPanel()
    panel.allowsMultipleSelection = parameters.allowsMultipleSelection
    panel.canChooseDirectories = false
    panel.begin { completionHandler($0 == .OK ? panel.urls : nil) }
  }
}

/// Makes an extension's page when Flutter asks for one.
final class ExtensionViewFactory: NSObject, FlutterPlatformViewFactory {
  private let messenger: FlutterBinaryMessenger
  private weak var flutter: FlutterViewController?

  /// Offer the view type on `controller`'s engine. Nothing is made until a
  /// panel shows an extension.
  static func register(with controller: FlutterViewController) {
    let registrar = controller.registrar(forPlugin: "LumitExtensionViews")
    registrar.register(
      ExtensionViewFactory(messenger: registrar.messenger, flutter: controller),
      withId: "lumit/extension_view")
  }

  private init(messenger: FlutterBinaryMessenger, flutter: FlutterViewController) {
    self.messenger = messenger
    self.flutter = flutter
  }

  func create(withViewIdentifier viewId: Int64, arguments args: Any?) -> NSView {
    ExtensionWebView.make(
      number: viewId, arguments: args as? [String: Any] ?? [:], messenger: messenger,
      flutter: flutter)
  }

  func createArgsCodec() -> (FlutterMessageCodec & NSObjectProtocol)? {
    FlutterStandardMessageCodec.sharedInstance()
  }
}
