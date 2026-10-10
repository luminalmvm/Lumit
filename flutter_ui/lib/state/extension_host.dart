// What one extension's page can ask of Lumit, and what it is told.
//
// The page talks in messages: {id, method, params} in, and {id, ok, result}
// or {id, ok, error} back, with {event, data} whenever something it may know
// about happens. Every method belongs to a permission the extension's
// manifest asked for, and one it did not ask for is refused here, before
// anything is done. The page reaches nothing of Lumit any other way.
//
// Nothing in this file knows how the page is shown. The panel that shows it
// hands messages in through [ExtensionHost.handle] and takes them out
// through [ExtensionHost.post].

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data' show BytesBuilder;

import 'package:crypto/crypto.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/about_window_frb.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/extensions.dart';
import 'package:lumit_flutter/src/rust/api/share.dart';
import 'package:lumit_flutter/state/external_links.dart';
import 'package:lumit_flutter/state/share.dart';
import 'package:lumit_flutter/state/updates.dart' show versionFromBootLine;

/// The version of the messages below. Raised when one changes meaning, and
/// what `app.info` tells a page so it can tell an older Lumit from a newer.
const int extensionApiVersion = 1;

/// What a page's own files are served under where the web view is WebKit,
/// which serves a folder under a scheme of the application's and not under
/// an https name.
const String extensionScheme = 'lumit-extension';

/// The name a page's own files are served under. Each extension has its own,
/// so what one stores no other can read. An https name on Windows and the
/// extension's own scheme on macOS and Linux, since that is what each web
/// view can serve a folder as.
String extensionOrigin(String id) =>
    Platform.isWindows ? 'https://$id.lumit.example' : '$extensionScheme://$id';

/// The most of an answer [ExtensionHost] fetches for a page.
const int _fetchLimit = 8 << 20;

/// How often the export and a watched folder are looked at.
const Duration _exportBeat = Duration(milliseconds: 500);
const Duration _watchBeat = Duration(seconds: 2);

/// How long after an edit the page is told, so a drag is one event.
const Duration _editSettle = Duration(milliseconds: 400);

/// Why a call was refused, as the page sees it.
class ExtensionRefusal implements Exception {
  /// `not-allowed`, `unknown-method`, `bad-params` or `failed`.
  final String code;
  final String message;

  const ExtensionRefusal(this.code, this.message);
}

/// What the panel does for a host that needs a window: the Shared project
/// window, and a folder picker.
abstract class ExtensionSurface {
  /// Open the Shared project window, on the Join page with [invite] in it,
  /// or on the Share page with [relay] offered. The person presses the
  /// button: an extension never shares or joins by itself.
  void openShare({String? invite, String? relay});

  Future<String?> chooseFolder();
}

class ExtensionHost with WidgetsBindingObserver {
  final BridgeExtension extension;
  final LumitState app;
  final ExtensionSurface surface;

  /// The folders this extension may watch: the ones the person picked for
  /// it. Kept by whoever made the host, so they outlive it.
  final Set<String> approvedFolders;
  final void Function() onFoldersChanged;

  /// Where messages for the page go. Null until the page is up.
  void Function(Map<String, Object?> message)? post;

  ExtensionHost({
    required this.extension,
    required this.app,
    required this.surface,
    required this.approvedFolders,
    required this.onFoldersChanged,
  });

  /// A permission as a manifest spells it.
  static String _word(BridgeExtensionPermission permission) =>
      permission.name.replaceAll('_', '');

  bool _allowed(BridgeExtensionPermission permission) =>
      extension.permissions.contains(permission);

  StreamSubscription<Object?>? _edits;
  Timer? _editTimer;
  Timer? _exportTimer;
  Timer? _watchTimer;
  String _projectWas = '';
  String _shareWas = '';
  BridgeExportState _exportWas = const BridgeExportState.idle();

  /// The file the last export made, which is the one file a page may ask
  /// the fingerprint of.
  String? _exported;

  /// The folder being watched, the files seen in it by their last size, and
  /// the ones already brought in or there from the start.
  String? _watched;
  final Map<String, int> _settling = {};
  final Set<String> _known = {};

  /// Start telling the page what it may know. Called once the page is up.
  void start() {
    WidgetsBinding.instance.addObserver(this);
    app.addListener(_projectMaybeChanged);
    app.share.roster.addListener(_shareMaybeChanged);
    _projectWas = _projectKey();
    _shareWas = _shareKey();
    if (_allowed(BridgeExtensionPermission.activity)) {
      _edits = app.onChange.listen((_) {
        _editTimer ??= Timer(_editSettle, _tellEdit);
      });
    }
    if (_allowed(BridgeExtensionPermission.export_)) {
      _exportWas = exportPoll();
      _exportTimer = Timer.periodic(_exportBeat, (_) => _pollExport());
    }
  }

  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    app.removeListener(_projectMaybeChanged);
    app.share.roster.removeListener(_shareMaybeChanged);
    _edits?.cancel();
    _editTimer?.cancel();
    _exportTimer?.cancel();
    _watchTimer?.cancel();
    post = null;
  }

  void _emit(String event, [Map<String, Object?> data = const {}]) =>
      post?.call({'event': event, 'data': data});

  // --- What the page is told --------------------------------------------

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (!_allowed(BridgeExtensionPermission.activity)) return;
    _emit('activity.focus', {'focused': state == AppLifecycleState.resumed});
  }

  Map<String, Object?> _projectInfo() {
    final project = app.project;
    String? path;
    var dirty = false;
    try {
      path = project?.path();
      dirty = project?.isDirty() ?? false;
    } catch (_) {
      // The project closed between the question and the answer.
    }
    final file = path?.split(RegExp(r'[\\/]')).last;
    return {
      'open': project != null,
      // The file's name and never where it is.
      'name': file,
      'saved': path != null,
      'dirty': dirty,
    };
  }

  String _projectKey() => jsonEncode(_projectInfo());

  void _projectMaybeChanged() {
    if (!_allowed(BridgeExtensionPermission.project)) return;
    final now = _projectKey();
    if (now == _projectWas) return;
    _projectWas = now;
    _emit('project.changed', _projectInfo());
  }

  void _tellEdit() {
    _editTimer = null;
    String? step;
    try {
      final entries = app.project?.historyEntries() ?? const [];
      step = entries.where((e) => !e.undone).lastOrNull?.name;
    } catch (_) {
      // No project to read a history from.
    }
    _emit('activity.edit', {
      'step': step,
      'at': DateTime.now().millisecondsSinceEpoch,
    });
    _projectMaybeChanged();
  }

  Map<String, Object?> _exportInfo(BridgeExportState state) => switch (state) {
        BridgeExportState_Idle() => {'state': 'idle'},
        BridgeExportState_Running(:final frame, :final total) => {
            'state': 'running',
            'frame': frame.toInt(),
            'total': total.toInt(),
          },
        BridgeExportState_Done(:final path) => {'state': 'done', 'path': path},
        BridgeExportState_Failed(:final error) => {
            'state': 'failed',
            'error': error,
          },
      };

  void _pollExport() {
    final BridgeExportState now;
    try {
      now = exportPoll();
    } catch (_) {
      return;
    }
    final was = _exportWas;
    if (now == was) return;
    _exportWas = now;
    switch (now) {
      case BridgeExportState_Running():
        _emit(
            was is BridgeExportState_Running
                ? 'export.progress'
                : 'export.started',
            _exportInfo(now));
      case BridgeExportState_Done(:final path):
        _exported = path;
        _emit('export.finished', _exportInfo(now));
      case BridgeExportState_Failed():
        _emit('export.failed', _exportInfo(now));
      case BridgeExportState_Idle():
        break;
    }
  }

  Map<String, Object?> _shareInfo() {
    final share = app.share;
    return {
      'role': share.role.name,
      'away': share.away,
      'people': [
        for (final person in share.people)
          {'id': person.id, 'name': person.name, 'me': person.me},
      ],
      // Whether someone far away can get in, for a host.
      'open': share.reach is BridgeShareReach_Open ||
          share.relayed == BridgeShareRelayed.open,
    };
  }

  String _shareKey() => jsonEncode(_shareInfo());

  void _shareMaybeChanged() {
    if (!_allowed(BridgeExtensionPermission.share)) return;
    final now = _shareKey();
    if (now == _shareWas) return;
    _shareWas = now;
    _emit('share.changed', _shareInfo());
  }

  void _pollWatch() {
    final folder = _watched;
    if (folder == null) return;
    final List<FileSystemEntity> listed;
    try {
      listed = Directory(folder).listSync(followLinks: false);
    } catch (_) {
      return;
    }
    final ready = <String>[];
    for (final entity in listed) {
      if (entity is! File || _known.contains(entity.path)) continue;
      final int size;
      try {
        size = entity.lengthSync();
      } catch (_) {
        continue;
      }
      // A file still being written grows between two looks. One that has
      // stopped is brought in.
      if (size > 0 && _settling[entity.path] == size) {
        ready.add(entity.path);
      } else {
        _settling[entity.path] = size;
      }
    }
    if (ready.isEmpty || app.project == null) return;
    for (final path in ready) {
      _known.add(path);
      _settling.remove(path);
    }
    unawaited(app.importFootagePaths(ready).then((_) {
      for (final path in ready) {
        _emit('import.added', {'path': path});
      }
    }));
  }

  void _watch(String folder) {
    _watched = folder;
    _settling.clear();
    _known.clear();
    try {
      // What is there already is not new.
      for (final entity in Directory(folder).listSync(followLinks: false)) {
        if (entity is File) _known.add(entity.path);
      }
    } catch (_) {
      // Nothing is listed, so everything in it will be new.
    }
    _watchTimer ??= Timer.periodic(_watchBeat, (_) => _pollWatch());
  }

  // --- What the page asks -----------------------------------------------

  /// One message from the page. Anything that is not a call is dropped.
  Future<void> handle(Object? message) async {
    if (message is! Map) return;
    final id = message['id'];
    final method = message['method'];
    if (id is! int || method is! String) return;
    final params = message['params'];
    try {
      final result = await _call(
          method, params is Map ? params : const <Object?, Object?>{});
      post?.call({'id': id, 'ok': true, 'result': result});
    } on ExtensionRefusal catch (refusal) {
      post?.call({
        'id': id,
        'ok': false,
        'error': {'code': refusal.code, 'message': refusal.message},
      });
    } catch (_) {
      post?.call({
        'id': id,
        'ok': false,
        'error': {'code': 'failed', 'message': 'Lumit could not do that'},
      });
    }
  }

  void _need(BridgeExtensionPermission permission) {
    if (_allowed(permission)) return;
    throw ExtensionRefusal('not-allowed',
        'this extension did not ask for the ${_word(permission)} permission');
  }

  String _text(Map<Object?, Object?> params, String name) {
    final value = params[name];
    if (value is String && value.isNotEmpty) return value;
    throw ExtensionRefusal('bad-params', '$name is missing');
  }

  Future<Object?> _call(String method, Map<Object?, Object?> params) async {
    switch (method) {
      case 'app.info':
        return {
          'api': extensionApiVersion,
          'lumit': versionFromBootLine(lumitVersion()),
          'extension': {'id': extension.id, 'version': extension.version},
          'permissions': [for (final p in extension.permissions) _word(p)],
          'hosts': extension.hosts,
        };
      case 'app.notice':
        app.postNotice('${extension.name}: ${_text(params, 'text')}');
        return null;
      case 'app.openExternal':
        final url = _allowedUrl(_text(params, 'url'));
        return launchInDefaultBrowser(url.toString());
      case 'net.fetch':
        return _fetch(params);

      case 'project.info':
        _need(BridgeExtensionPermission.project);
        return _projectInfo();

      case 'export.state':
        _need(BridgeExtensionPermission.export_);
        return _exportInfo(_exportWas);
      case 'export.fingerprint':
        _need(BridgeExtensionPermission.export_);
        return _fingerprint();

      case 'import.files':
        _need(BridgeExtensionPermission.import_);
        return _import(params);
      case 'import.watch':
        _need(BridgeExtensionPermission.import_);
        return _startWatch(params);
      case 'import.unwatch':
        _need(BridgeExtensionPermission.import_);
        _watched = null;
        _watchTimer?.cancel();
        _watchTimer = null;
        return null;

      case 'share.state':
        _need(BridgeExtensionPermission.share);
        return _shareInfo();
      case 'share.link':
        _need(BridgeExtensionPermission.share);
        return app.share.role == ShareRole.host ? app.share.link() : null;
      case 'share.start':
        _need(BridgeExtensionPermission.share);
        final relay = params['relay'];
        surface.openShare(relay: relay is String ? relay : null);
        return null;
      case 'share.join':
        _need(BridgeExtensionPermission.share);
        surface.openShare(invite: _text(params, 'link'));
        return null;
      case 'share.stop':
        _need(BridgeExtensionPermission.share);
        app.stopSharing();
        return null;
    }
    throw ExtensionRefusal('unknown-method', 'Lumit has no $method');
  }

  /// [text] as an address this extension's manifest lets it reach: https,
  /// and a site it named or one under it.
  Uri _allowedUrl(String text) {
    final url = Uri.tryParse(text);
    final host = url?.host.toLowerCase() ?? '';
    final named = extension.hosts
        .map((h) => h.toLowerCase())
        .any((h) => host == h || host.endsWith('.$h'));
    if (url == null || url.scheme != 'https' || !named) {
      throw const ExtensionRefusal('not-allowed',
          'that address is not https on a site this extension named');
    }
    return url;
  }

  /// Fetch for the page, from the sites its manifest named and no others.
  /// A redirect is handed back rather than followed, so it cannot lead
  /// somewhere the manifest did not name.
  Future<Object?> _fetch(Map<Object?, Object?> params) async {
    final url = _allowedUrl(_text(params, 'url'));
    final method = params['method'];
    final headers = params['headers'];
    final body = params['body'];
    final client = HttpClient()
      ..connectionTimeout = const Duration(seconds: 15);
    try {
      final request = await client.openUrl(
          method is String ? method.toUpperCase() : 'GET', url);
      request.followRedirects = false;
      if (headers is Map) {
        for (final MapEntry(:key, :value) in headers.entries) {
          if (key is String && value is String) request.headers.set(key, value);
        }
      }
      if (body is String) request.add(utf8.encode(body));
      final response =
          await request.close().timeout(const Duration(seconds: 30));
      final bytes = BytesBuilder(copy: false);
      await for (final chunk in response.timeout(const Duration(seconds: 30))) {
        bytes.add(chunk);
        if (bytes.length > _fetchLimit) {
          throw const ExtensionRefusal(
              'failed', 'the answer was longer than 8 MB');
        }
      }
      final answered = <String, String>{};
      response.headers.forEach((name, values) {
        answered[name] = values.join(', ');
      });
      return {
        'status': response.statusCode,
        'headers': answered,
        'body': utf8.decode(bytes.takeBytes(), allowMalformed: true),
      };
    } on ExtensionRefusal {
      rethrow;
    } catch (_) {
      throw const ExtensionRefusal('failed', 'that site could not be reached');
    } finally {
      client.close(force: true);
    }
  }

  /// The SHA-256 of the file the last export made, and nothing else's.
  Future<Object?> _fingerprint() async {
    final path = _exported;
    if (path == null) {
      throw const ExtensionRefusal(
          'failed', 'no export has finished since this panel opened');
    }
    final file = File(path);
    final digest = await sha256.bind(file.openRead()).first;
    return {
      'path': path,
      'bytes': await file.length(),
      'sha256': digest.toString(),
    };
  }

  Future<Object?> _import(Map<Object?, Object?> params) async {
    final asked = params['paths'];
    final paths = [
      if (asked is List)
        for (final path in asked)
          if (path is String && File(path).existsSync()) path,
    ];
    if (paths.isEmpty || app.project == null) {
      throw const ExtensionRefusal(
          'bad-params', 'paths names no file, or no project is open');
    }
    await app.importFootagePaths(paths);
    return {'imported': paths.length};
  }

  /// Watch a folder and bring in what appears in it. With no folder named
  /// the person picks one. A folder named has to be one they picked for
  /// this extension before.
  Future<Object?> _startWatch(Map<Object?, Object?> params) async {
    var folder = params['folder'];
    if (folder is! String) {
      folder = await surface.chooseFolder();
      if (folder is! String) return {'folder': null};
      approvedFolders.add(folder);
      onFoldersChanged();
    } else if (!approvedFolders.contains(folder)) {
      throw const ExtensionRefusal('not-allowed',
          'that folder was not picked for this extension. Call import.watch with no folder to ask');
    }
    if (!Directory(folder).existsSync()) {
      throw const ExtensionRefusal('failed', 'that folder is not there');
    }
    _watch(folder);
    return {'folder': folder};
  }
}

/// What a page is given before any of its own script runs: `window.lumit`,
/// with `call` to ask and `on` to listen. [origin] is the page's own, and a
/// page anywhere else is given nothing.
///
/// The two web views carry messages differently. WebView2 has a channel of
/// its own each way. WebKit hands a page one function to send text with, the
/// same one to every frame on the page, so each message carries [key], which
/// only the page's own frame was told, and an answer comes back as a call to
/// a function this leaves on the window.
String extensionBootstrap({required String origin, required String key}) =>
    _bootstrap
        .replaceFirst('__ORIGIN__', jsonEncode(origin))
        .replaceFirst('__KEY__', jsonEncode(key));

/// A message for a WebKit page, as the script that hands it over.
String extensionDeliver(Map<String, Object?> message) =>
    'window.__lumitDeliver&&window.__lumitDeliver(${jsonEncode(message)})';

/// What a WebKit page sent, or null when it does not carry [key].
Object? extensionUnwrap(Object? text, String key) {
  if (text is! String) return null;
  try {
    final sent = jsonDecode(text);
    if (sent is Map && sent['key'] == key) return sent['message'];
  } catch (_) {
    // Not something the bootstrap wrote.
  }
  return null;
}

const String _bootstrap = r'''
(() => {
  if (window.lumit || location.protocol + '//' + location.host !== __ORIGIN__) return;
  const webview2 = window.chrome && window.chrome.webview;
  const webkit = window.webkit && window.webkit.messageHandlers
    && window.webkit.messageHandlers.lumit;
  if (!webview2 && !webkit) return;
  const waiting = new Map();
  const listeners = new Map();
  let next = 1;
  const receive = (message) => {
    if (!message || typeof message !== 'object') return;
    if (typeof message.event === 'string') {
      for (const listener of listeners.get(message.event) || []) {
        try { listener(message.data); } catch (error) { console.error(error); }
      }
      return;
    }
    const call = waiting.get(message.id);
    if (!call) return;
    waiting.delete(message.id);
    if (message.ok) {
      call.resolve(message.result);
    } else {
      const error = new Error(message.error && message.error.message || 'failed');
      error.code = message.error && message.error.code || 'failed';
      call.reject(error);
    }
  };
  let send;
  if (webview2) {
    webview2.addEventListener('message', (event) => receive(event.data));
    send = (message) => webview2.postMessage(message);
  } else {
    Object.defineProperty(window, '__lumitDeliver', { value: receive });
    send = (message) => webkit.postMessage(JSON.stringify({ key: __KEY__, message }));
  }
  window.lumit = Object.freeze({
    call(method, params) {
      return new Promise((resolve, reject) => {
        const id = next++;
        waiting.set(id, { resolve, reject });
        send({ id, method, params: params || {} });
      });
    },
    on(event, listener) {
      if (!listeners.has(event)) listeners.set(event, new Set());
      listeners.get(event).add(listener);
      return () => listeners.get(event).delete(listener);
    },
  });
  window.dispatchEvent(new Event('lumitready'));
})();
''';
