// Whatever draws an extension's page.
//
// The page is shown by the system's web view, and that is a different thing
// on each platform. On Windows it is WebView2, drawn into a texture. On
// macOS it is a WKWebView, which Flutter can place among its own widgets.
// On Linux it is a WebKitGTK view, which Flutter cannot: the runner lays it
// over the window where this widget is, and this widget tells it where that
// is.
//
// All three serve the extension's folder under a name of its own, give the
// page `window.lumit` before its own script runs, and carry messages both
// ways. Nothing here knows what the messages mean: they go to and from an
// [ExtensionPageLink], which the panel holds the other end of.

import 'dart:async';
import 'dart:convert';
import 'dart:io' show Platform;
import 'dart:math' show Random;
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/extensions.dart';
import 'package:webview_windows/webview_windows.dart';

import '../l10n/strings.dart';
import '../state/extension_host.dart';
import '../widgets/controls.dart';

/// The two ends of one page: what the panel is told, and what it can ask of
/// the page once it is up.
class ExtensionPageLink {
  ExtensionPageLink({
    required this.onMessage,
    required this.onUrl,
    required this.onReady,
    required this.onFailed,
  });

  /// A message the page sent.
  final void Function(Object? message) onMessage;

  /// The address the view is at, each time it changes.
  final void Function(String url) onUrl;

  /// The view is up and [post] and [load] can be used.
  final VoidCallback onReady;

  /// The view could not be put up, and why in the person's own language.
  final void Function(String why) onFailed;

  /// Send the page a message. Null until [onReady].
  void Function(Map<String, Object?> message)? post;

  /// Take the view to an address. Null until [onReady].
  void Function(String url)? load;
}

/// An extension's page, drawn by this platform's web view.
class ExtensionPage extends StatelessWidget {
  final BridgeExtension extension;
  final ExtensionPageLink link;

  const ExtensionPage({super.key, required this.extension, required this.link});

  @override
  Widget build(BuildContext context) {
    if (Platform.isWindows) return _WebView2Page(extension, link);
    if (Platform.isMacOS) return _AppKitPage(extension, link);
    return _GtkPage(extension, link);
  }
}

/// Set to 1 to let the page be inspected: Safari's Develop menu on macOS,
/// Inspect Element on the page's own menu on Linux.
bool get _inspectable =>
    Platform.environment['LUMIT_EXTENSION_DEVTOOLS'] == '1';

/// A key for one WebKit page, which nothing could guess.
String _newKey() {
  final random = Random.secure();
  return [
    for (var i = 0; i < 16; i++)
      random.nextInt(256).toRadixString(16).padLeft(2, '0'),
  ].join();
}

// --- Windows ------------------------------------------------------------

/// The one web view environment every extension's page shares, made the
/// first time a panel needs it. Each page still keeps its own storage: they
/// are served under different names.
Future<void>? _environment;

Future<void> _ensureEnvironment() => _environment ??= () async {
      try {
        await WebviewController.initializeEnvironment(
            userDataPath: extensionDataDir(id: 'webview'));
      } on PlatformException {
        // Made already, by a panel before a hot restart.
      }
    }();

class _WebView2Page extends StatefulWidget {
  final BridgeExtension extension;
  final ExtensionPageLink link;

  const _WebView2Page(this.extension, this.link);

  @override
  State<_WebView2Page> createState() => _WebView2PageState();
}

class _WebView2PageState extends State<_WebView2Page> {
  final WebviewController _view = WebviewController();
  final List<StreamSubscription<Object?>> _listening = [];
  bool _up = false;

  @override
  void initState() {
    super.initState();
    unawaited(_open());
  }

  Future<void> _open() async {
    final extension = widget.extension;
    final link = widget.link;
    final origin = extensionOrigin(extension.id);
    try {
      if (await WebviewController.getWebViewVersion() == null) {
        return link.onFailed(l10n.extensionNoRuntime);
      }
      await _ensureEnvironment();
      await _view.initialize();
      if (!mounted) return;
      // No window of its own, and its files are its own: a page from
      // anywhere else cannot load them.
      await _view.setPopupWindowPolicy(WebviewPopupWindowPolicy.deny);
      await _view.addVirtualHostNameMapping(origin.substring(8),
          extension.folder, WebviewHostResourceAccessKind.deny);
      await _view.addScriptToExecuteOnDocumentCreated(
          extensionBootstrap(origin: origin, key: ''));
      _listening.add(_view.url.listen(link.onUrl));
      _listening.add(_view.webMessage.listen(link.onMessage, onError: (_) {}));
      link.post =
          (message) => unawaited(_view.postWebMessage(jsonEncode(message)));
      link.load = (url) => unawaited(_view.loadUrl(url));
      await _view.loadUrl('$origin/${extension.entry}');
      if (!mounted) return;
      setState(() => _up = true);
      link.onReady();
    } catch (_) {
      link.onFailed(l10n.extensionFailedHint);
    }
  }

  @override
  void dispose() {
    for (final subscription in _listening) {
      subscription.cancel();
    }
    _view.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => _up
      ? Webview(_view)
      : ColoredBox(color: ThemeScope.of(context).theme.surface1);
}

// --- macOS --------------------------------------------------------------

class _AppKitPage extends StatefulWidget {
  final BridgeExtension extension;
  final ExtensionPageLink link;

  const _AppKitPage(this.extension, this.link);

  @override
  State<_AppKitPage> createState() => _AppKitPageState();
}

class _AppKitPageState extends State<_AppKitPage> {
  final String _key = _newKey();
  late final String _origin = extensionOrigin(widget.extension.id);

  /// The view's own channel, named by the number Flutter gave the view.
  MethodChannel? _channel;

  @override
  void initState() {
    super.initState();
    GestureBinding.instance.pointerRouter.addGlobalRoute(_pressed);
  }

  /// A press Flutter was given landed outside the page, which keeps the
  /// keyboard until it is taken back.
  void _pressed(PointerEvent event) {
    if (event is PointerDownEvent) unawaited(_ask('unfocus'));
  }

  Future<void> _ask(String method, [Object? argument]) async {
    try {
      await _channel?.invokeMethod<void>(method, argument);
    } catch (_) {
      // The view has gone.
    }
  }

  void _created(int view) {
    final link = widget.link;
    final channel = MethodChannel('lumit/extension_view_$view');
    _channel = channel;
    channel.setMethodCallHandler((call) async {
      switch (call.method) {
        case 'message':
          final message = extensionUnwrap(call.arguments, _key);
          if (message != null) link.onMessage(message);
        case 'url':
          if (call.arguments case final String url) link.onUrl(url);
      }
    });
    link.post = (message) => unawaited(_ask('run', extensionDeliver(message)));
    link.load = (url) => unawaited(_ask('load', url));
    // Loaded from here and not as the view is made, so nothing the page
    // says is said before anyone is listening.
    unawaited(_ask('load', '$_origin/${widget.extension.entry}'));
    link.onReady();
  }

  @override
  void dispose() {
    GestureBinding.instance.pointerRouter.removeGlobalRoute(_pressed);
    _channel?.setMethodCallHandler(null);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AppKitView(
        viewType: 'lumit/extension_view',
        creationParams: <String, Object?>{
          'id': widget.extension.id,
          'folder': widget.extension.folder,
          'bootstrap': extensionBootstrap(origin: _origin, key: _key),
          'inspect': _inspectable,
        },
        creationParamsCodec: const StandardMessageCodec(),
        onPlatformViewCreated: _created,
      );
}

// --- Linux --------------------------------------------------------------

class _GtkPage extends StatefulWidget {
  final BridgeExtension extension;
  final ExtensionPageLink link;

  const _GtkPage(this.extension, this.link);

  @override
  State<_GtkPage> createState() => _GtkPageState();
}

/// Flutter on Linux cannot hold another toolkit's view among its widgets, so
/// the page's view lies over the window and this keeps it where the panel
/// is. That puts it over anything Flutter draws there too: a menu, a
/// dialogue, a panel being dragged. So while something is over the panel the
/// view is hidden and a picture of it stands in.
class _GtkPageState extends State<_GtkPage> {
  static const MethodChannel _channel = MethodChannel('lumit/extension_views');
  static final Map<int, _GtkPageState> _pages = {};
  static int _next = 1;
  static bool _hooked = false;

  /// Listen to the runner, and look at every page after every frame. Once,
  /// since neither can be taken back, and a frame with no page costs one
  /// empty loop.
  static void _hook() {
    if (_hooked) return;
    _hooked = true;
    _channel.setMethodCallHandler((call) async {
      final arguments = call.arguments;
      if (arguments is! Map) return;
      final page = _pages[arguments['view']];
      if (page == null) return;
      switch (call.method) {
        case 'message':
          final message = extensionUnwrap(arguments['text'], page._key);
          if (message != null) page.widget.link.onMessage(message);
        case 'url':
          if (arguments['url'] case final String url) {
            page.widget.link.onUrl(url);
          }
      }
    });
    WidgetsBinding.instance.addPersistentFrameCallback((_) {
      for (final page in _pages.values.toList()) {
        page._place();
      }
    });
    // A press Flutter was given landed outside every page, and a page
    // that was typed in keeps the keyboard until it is taken back.
    GestureBinding.instance.pointerRouter.addGlobalRoute((event) {
      if (event is PointerDownEvent && _pages.isNotEmpty) {
        unawaited(_ask('unfocus'));
      }
    });
  }

  static Future<T?> _ask<T>(String method, [Object? arguments]) async {
    try {
      return await _channel.invokeMethod<T>(method, arguments);
    } catch (_) {
      return null;
    }
  }

  final int _view = _next++;
  final String _key = _newKey();
  late final String _origin = extensionOrigin(widget.extension.id);

  /// The runner has made the view.
  bool _made = false;

  /// What the runner was last told of where the view is.
  Rect _sentRect = Rect.zero;
  double _sentScale = 1;
  bool _sentShown = false;

  /// Something Flutter draws is over the panel.
  bool _covered = false;

  /// The view is hidden for it, which waits for [_still] to be on screen.
  bool _hidden = false;

  /// A picture of the page as it was when it was covered.
  ui.Image? _still;

  /// Moves on each time [_covered] changes, so a picture that arrives late
  /// is known to be late.
  int _turn = 0;

  @override
  void initState() {
    super.initState();
    _hook();
    _pages[_view] = this;
    unawaited(_make());
  }

  Future<void> _make() async {
    final extension = widget.extension;
    final link = widget.link;
    try {
      await _channel.invokeMethod<void>('create', <String, Object?>{
        'view': _view,
        'id': extension.id,
        'folder': extension.folder,
        'data': extensionDataDir(id: 'webview'),
        'bootstrap': extensionBootstrap(origin: _origin, key: _key),
        'inspect': _inspectable,
      });
    } catch (_) {
      if (mounted) link.onFailed(l10n.extensionFailedHint);
      return;
    }
    if (!mounted) {
      unawaited(_ask('dispose', {'view': _view}));
      return;
    }
    _made = true;
    link.post = (message) => unawaited(
        _ask('run', {'view': _view, 'script': extensionDeliver(message)}));
    link.load = (url) => unawaited(_ask('load', {'view': _view, 'url': url}));
    link.load!('$_origin/${extension.entry}');
    link.onReady();
    // Nothing else may ask for a frame, and the view is placed after one.
    WidgetsBinding.instance.scheduleFrame();
  }

  /// Whether [box] is under a tab that is not the one in front.
  static bool _offstage(RenderObject box) {
    for (RenderObject? node = box; node != null; node = node.parent) {
      if (node is RenderOffstage && node.offstage) return true;
    }
    return false;
  }

  /// Whether anything Flutter draws is over the panel: a menu, a dialogue,
  /// a tooltip or a dragged panel, which all go in the shell's overlay
  /// above the one entry the shell itself is, or something of the shell's
  /// own that takes the pointer where the panel is.
  bool _coveredAt(RenderBox box, Offset centre) {
    var entries = 0;
    Overlay.maybeOf(context)
        ?.context
        .findRenderObject()
        ?.visitChildren((_) => entries++);
    if (entries > 1) return true;
    final result = HitTestResult();
    WidgetsBinding.instance
        .hitTestInView(result, centre, View.of(context).viewId);
    return !result.path.any((entry) => entry.target == box);
  }

  /// Tell the runner where the view is now, if that has changed. Called
  /// after every frame, since anything that moves the panel draws one.
  void _place() {
    if (!_made || !mounted) return;
    final box = context.findRenderObject();
    var rect = Rect.zero;
    var scale = 1.0;
    var shown = false;
    if (box is RenderBox && box.attached && box.hasSize && !_offstage(box)) {
      rect = MatrixUtils.transformRect(
          box.getTransformTo(null), Offset.zero & box.size);
      shown = rect.width >= 1 && rect.height >= 1;
      // The interface is drawn at a scale of the person's choosing, and the
      // page is drawn at the same one.
      if (shown) scale = rect.width / box.size.width;
      _cover(shown && _coveredAt(box, rect.center));
    }
    shown = shown && !_hidden;
    if (rect == _sentRect && scale == _sentScale && shown == _sentShown) {
      return;
    }
    _sentRect = rect;
    _sentScale = scale;
    _sentShown = shown;
    final placed = _ask('place', <String, Object?>{
      'view': _view,
      'x': rect.left,
      'y': rect.top,
      'width': rect.width,
      'height': rect.height,
      'scale': scale,
      'shown': shown,
    });
    // The picture goes once the view is back, and not a frame before.
    if (shown && _still != null) {
      final turn = _turn;
      unawaited(placed.then((_) {
        if (mounted && turn == _turn) _setStill(null);
      }));
    }
  }

  void _setStill(ui.Image? still) {
    final old = _still;
    setState(() => _still = still);
    old?.dispose();
  }

  void _cover(bool covered) {
    if (covered == _covered) return;
    _covered = covered;
    final turn = ++_turn;
    if (!covered) {
      _hidden = false;
      return;
    }
    unawaited(_picture().then((still) {
      if (!mounted || turn != _turn) {
        still?.dispose();
        return;
      }
      if (still != null) _setStill(still);
      // Hidden once the picture has been drawn, so the panel is never
      // empty in between.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted || turn != _turn) return;
        _hidden = true;
        WidgetsBinding.instance.scheduleFrame();
      });
      WidgetsBinding.instance.scheduleFrame();
    }));
  }

  /// A picture of the page as it is now, or null when the runner has none.
  Future<ui.Image?> _picture() async {
    final shot = await _ask<Map<Object?, Object?>>('picture', {'view': _view});
    final pixels = shot?['pixels'];
    final width = shot?['width'];
    final height = shot?['height'];
    if (pixels is! Uint8List || width is! int || height is! int) return null;
    if (width < 1 || height < 1 || pixels.length != width * height * 4) {
      return null;
    }
    final done = Completer<ui.Image>();
    ui.decodeImageFromPixels(
        pixels, width, height, ui.PixelFormat.rgba8888, done.complete);
    return done.future;
  }

  @override
  void dispose() {
    _pages.remove(_view);
    if (_made) unawaited(_ask('dispose', {'view': _view}));
    _still?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Takes the pointer itself, which is how [_coveredAt] tells the panel
    // from something lying over it.
    return Listener(
      behavior: HitTestBehavior.opaque,
      child: ColoredBox(
        color: t.surface1,
        child: SizedBox.expand(
          child: _still == null
              ? null
              : RawImage(image: _still, fit: BoxFit.fill),
        ),
      ),
    );
  }
}
