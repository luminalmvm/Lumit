// The Lottie and SVG window's browser player: the system's web view, which
// is the only thing that plays an animated SVG, and where most Lottie files
// play. It is the extension page host showing a folder of its own: an
// index.html, the bundled lottie-web, and the current file. The transport
// stays Lumit's: play, pause, seek and loop go to the page as events, and the
// page reports its frame back.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/services.dart' show rootBundle;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/extensions.dart';

import '../panels/extension_page.dart';
import '../widgets/controls.dart';
import 'web_player_page.dart';

/// The bundled player script, listed in pubspec under `assets/players/`.
const String _playerScript = 'assets/players/lottie/lottie_svg.min.js';

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
  /// The folder the page is served from, made once the script is copied.
  Directory? _folder;
  late final ExtensionPageLink _link = ExtensionPageLink(
    onMessage: _message,
    onUrl: (_) {},
    onReady: () {},
    onFailed: (why) => setState(() => _failed = why),
  );
  String? _failed;

  /// The page has run its script and is listening.
  bool _up = false;

  /// Counts the files written, so each is served under a new name and the
  /// page never shows a cached one, and the name of the last.
  int _written = 0;
  String? _served;

  @override
  void initState() {
    super.initState();
    widget.controller._page = this;
    unawaited(_prepare());
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

  Future<void> _prepare() async {
    final folder = Directory.systemTemp.createTempSync('lumit-player');
    final script = await rootBundle.load(_playerScript);
    if (!mounted) {
      folder.deleteSync(recursive: true);
      return;
    }
    File('${folder.path}/lottie_svg.min.js')
        .writeAsBytesSync(script.buffer.asUint8List());
    File('${folder.path}/index.html').writeAsStringSync(webPlayerPage);
    setState(() => _folder = folder);
  }

  @override
  void dispose() {
    if (identical(widget.controller._page, this)) {
      widget.controller._page = null;
    }
    try {
      _folder?.deleteSync(recursive: true);
    } catch (_) {
      // The web view may still hold a file; the system clears its temp.
    }
    super.dispose();
  }

  void _post(Map<String, Object?> message) => _link.post?.call(message);

  /// Write the controller's file beside the page and tell the page to show
  /// it where the transport is.
  void _load() {
    final folder = _folder;
    final c = widget.controller;
    final bytes = c._bytes;
    if (folder == null || bytes == null || !_up) return;
    final name = 'a${++_written}.${c._kind == 'svg' ? 'svg' : 'json'}';
    File('${folder.path}/$name').writeAsBytesSync(bytes);
    if (_served != null) {
      try {
        File('${folder.path}/$_served').deleteSync();
      } catch (_) {
        // The web view may still hold it. The folder goes with the player.
      }
    }
    _served = name;
    _post({
      'event': 'load',
      'data': {
        'kind': c._kind,
        'path': name,
        'fps': c._fps,
        'frames': c._frames,
        'loop': c._loop,
        'ground': c._ground,
        'frame': c._frame,
        'playing': c._playing,
      },
    });
  }

  void _message(Object? message) {
    if (message is String) {
      try {
        message = jsonDecode(message);
      } catch (_) {
        return;
      }
    }
    if (message is! Map) return;
    final id = message['id'];
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
    // Every call is answered, so the page's own list of waiting calls
    // stays empty across a long session.
    if (id is num && id != 0) _post({'id': id, 'ok': true, 'result': null});
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final folder = _folder;
    if (_failed != null) {
      return Center(
          child: Text(_failed!, style: t.small.copyWith(color: t.textMuted)));
    }
    if (folder == null) return const SizedBox.expand();
    return ExtensionPage(
      extension: BridgeExtension(
        id: 'lumit-player',
        name: '',
        version: '',
        summary: '',
        author: '',
        homepage: '',
        folder: folder.path,
        panel: true,
        entry: 'index.html',
        permissions: const [],
        hosts: const [],
        broken: false,
        effects: const [],
        native: false,
      ),
      link: _link,
    );
  }
}
