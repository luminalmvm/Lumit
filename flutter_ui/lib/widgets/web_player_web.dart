// The browser player in the web build: the desktop's page in an iframe of
// its own. A few lines here stand in for the extension page host's
// `window.lumit`, and the file goes over as a blob URL.

import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';
import 'dart:typed_data';

import 'package:flutter/services.dart' show rootBundle;
import 'package:flutter/widgets.dart';
import 'package:web/web.dart' as web;

import 'web_player_page.dart';

/// The bundled player script, listed in pubspec under `assets/players/`.
const String _playerScript = 'assets/players/lottie/lottie_svg.min.js';

/// The page's `window.lumit`: calls go up to this window and events come
/// down from it, both as JSON text.
const String _lumit = r'''<script>
window.lumit = (function () {
  var listeners = {};
  addEventListener('message', function (e) {
    if (e.source !== parent || typeof e.data !== 'string') return;
    var m = JSON.parse(e.data);
    (listeners[m.event] || []).forEach(function (fn) { fn(m.data); });
  });
  return {
    call: function (method, params) {
      parent.postMessage(JSON.stringify({ method: method, params: params || {} }), self.origin);
      return Promise.resolve(null);
    },
    on: function (event, fn) { (listeners[event] = listeners[event] || []).push(fn); }
  };
})();
</script>''';

/// What the window tells the page, and what the page tells it.
class WebPlayerController {
  WebPlayerController({required this.onFrame, this.onEnded});

  /// The frame the page is on.
  final ValueChanged<int> onFrame;

  /// A file that doesn't loop has played to its end.
  final VoidCallback? onEnded;

  _WebPlayerState? _page;
  Uint8List? _bytes;
  String _kind = 'lottie';
  double _fps = 30;
  int _frames = 1;
  bool _loop = true;
  String _ground = '';
  bool _playing = false;
  int _frame = 0;

  /// A new file: [kind] is `lottie` or `svg`. Played from [frame].
  void load({
    required Uint8List bytes,
    required String kind,
    required double fps,
    required int frames,
    required bool loop,
    required int frame,
  }) {
    _bytes = bytes;
    _kind = kind;
    _fps = fps;
    _frames = frames;
    _loop = loop;
    _frame = frame;
    _page?._load();
  }

  void play() {
    _playing = true;
    _page?._post({'event': 'play'});
  }

  void pause() {
    _playing = false;
    _page?._post({'event': 'pause'});
  }

  void seek(int frame) {
    _frame = frame;
    _page?._post({'event': 'seek', 'data': {'frame': frame}});
  }

  /// What the page shows behind the picture, as a CSS background.
  void setGround(String css) {
    _ground = css;
    _page?._post({'event': 'ground', 'data': {'css': css}});
  }

  void setLoop(bool on) {
    _loop = on;
    _page?._post({'event': 'loop', 'data': {'on': on}});
  }
}

class WebPlayer extends StatefulWidget {
  final WebPlayerController controller;

  const WebPlayer({super.key, required this.controller});

  @override
  State<WebPlayer> createState() => _WebPlayerState();
}

class _WebPlayerState extends State<WebPlayer> {
  /// The page's frame, once the view has made it.
  web.HTMLIFrameElement? _frame;
  late final JSFunction _listener = _message.toJS;

  /// The page has run its script and is listening.
  bool _up = false;

  /// The blob URL of the file the page was last given.
  String? _url;

  @override
  void initState() {
    super.initState();
    widget.controller._page = this;
    web.window.addEventListener('message', _listener);
  }

  @override
  void didUpdateWidget(WebPlayer old) {
    super.didUpdateWidget(old);
    if (!identical(old.controller, widget.controller)) {
      old.controller._page = null;
      widget.controller._page = this;
      if (_up) _load();
    }
  }

  /// The script goes into the page itself, so nothing in the iframe has a
  /// URL to resolve.
  Future<void> _prepare(web.HTMLIFrameElement frame) async {
    final script = await rootBundle.loadString(_playerScript);
    if (!mounted) return;
    frame.srcdoc = webPlayerPage
        .replaceFirst('<script src="lottie_svg.min.js"></script>',
            '$_lumit<script>$script</script>')
        .toJS;
  }

  @override
  void dispose() {
    if (identical(widget.controller._page, this)) {
      widget.controller._page = null;
    }
    web.window.removeEventListener('message', _listener);
    _revoke();
    super.dispose();
  }

  void _revoke() {
    final url = _url;
    if (url != null) web.URL.revokeObjectURL(url);
    _url = null;
  }

  void _post(Map<String, Object?> message) => _frame?.contentWindow
      ?.postMessage(jsonEncode(message).toJS, web.window.origin.toJS);

  /// Hand the page the controller's file, to show where the transport is.
  void _load() {
    final c = widget.controller;
    final bytes = c._bytes;
    if (bytes == null || !_up) return;
    _revoke();
    final url = web.URL.createObjectURL(web.Blob(<JSAny>[bytes.toJS].toJS));
    _url = url;
    _post({
      'event': 'load',
      'data': {
        'kind': c._kind,
        'path': url,
        'fps': c._fps,
        'frames': c._frames,
        'loop': c._loop,
        'ground': c._ground,
        'frame': c._frame,
        'playing': c._playing,
      },
    });
  }

  void _message(web.MessageEvent event) {
    final page = _frame?.contentWindow;
    if (page == null || !event.source.strictEquals(page).toDart) return;
    final data = event.data.dartify();
    if (data is! String) return;
    final Object? message;
    try {
      message = jsonDecode(data);
    } catch (_) {
      return;
    }
    if (message is! Map) return;
    final params = message['params'];
    switch (message['method']) {
      case 'up':
        _up = true;
        _load();
      case 'ended':
        // A file that has ended is stopped, so its next load stays still.
        widget.controller._playing = false;
        widget.controller.onEnded?.call();
      case 'frame':
        if (params is Map && params['frame'] is num) {
          widget.controller.onFrame((params['frame'] as num).toInt());
        }
    }
  }

  /// Flutter's own iframe view, which lets the element go with the view.
  @override
  Widget build(BuildContext context) => HtmlElementView.fromTagName(
        tagName: 'iframe',
        onElementCreated: (element) {
          final frame = element as web.HTMLIFrameElement;
          frame.style
            ..border = 'none'
            ..width = '100%'
            ..height = '100%';
          _frame = frame;
          unawaited(_prepare(frame));
        },
      );
}
