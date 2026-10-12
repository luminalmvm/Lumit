// The door to a shared project that is kept online.
//
// A shared project can be kept on Lumit's server, sealed, so the people in
// it can open it and edit when nobody else is there. The server is spoken to
// over HTTPS and one WebSocket, and the engine has neither. So, as with
// Lumit's relay (`relay_door.dart`), the engine never dials it. It dials
// this door, on this machine, once for each project, and speaks frames. The
// door makes the calls those frames ask for and passes the answers back.
//
// The door carries and does not read. Whatever is stored was sealed before
// it got here, with a key the door is never shown. It looks at the few bytes
// in front of a frame that say where the frame goes, and no further.
//
// Two things matter more than the rest. Entries reach the engine in the
// order the server sent them, because that order is the project. And
// neither end is read faster than the other can take, so a slow line makes
// the engine wait and never makes the door hold more than a frame or two.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'account.dart';
import 'relay_door.dart';

/// The first line a caller says: which project, the access the server is
/// shown on every call, this machine's token, and its person's name, sealed.
/// Anything else is refused, and so is a line longer than the limit.
final RegExp _first = RegExp(r'^LUMIT-ONLINE 1 ([0-9a-f]{32}) ([0-9a-f]{64})'
    r' ([0-9]{1,20}) ([A-Za-z0-9_-]+)$');
const int _lineLimit = 1024;

// The kinds of frame. Those under 0x20 are the server's own and pass through
// as they are, but for a batch that is too big for one socket message. The
// rest are between the engine and the door.
const int _append = 0x01, _appendBlob = 0x02, _entry = 0x11, _entryBlob = 0x12;
const int _create = 0x20, _meta = 0x21, _getSnapshot = 0x22;
const int _putSnapshot = 0x23, _part = 0x24, _end = 0x25, _live = 0x26;
const int _ok = 0x30, _metaIs = 0x31, _partIs = 0x32, _endIs = 0x33;
const int _againIs = 0x34;
const int _lost = 0x3e, _failed = 0x3f;

/// The most a socket message may be, then a batch, a snapshot, and one part
/// of a snapshot. A length is held against its limit before any room is made
/// for what it measures.
const int _messageLimit = 512 << 10;
const int _batchLimit = 64 << 20;
const int _snapshotLimit = 95000000;
const int _partLimit = 1 << 20;

/// How long the server has to answer, and how long it may go quiet in the
/// middle of an answer.
const Duration _patience = Duration(seconds: 30);

/// How long a fetch that was cut goes on asking for the rest with nothing
/// more of it arriving, before the line is taken for gone.
const Duration _lineGone = Duration(minutes: 2);

/// How often the socket is asked whether it is still there. One that has
/// died without saying so is dropped after two of these.
const Duration _alive = Duration(seconds: 15);

/// How long a line that is being left has to finish what it was doing.
const Duration _leave = Duration(seconds: 2);

final Random _random = Random.secure();

String _hex(List<int> bytes) =>
    [for (final b in bytes) b.toRadixString(16).padLeft(2, '0')].join();

/// A number the engine wrote as eight bytes. None is anywhere near the top
/// bit, which Dart would read as a minus sign.
int _u64(Uint8List bytes, int at) {
  final number = ByteData.sublistView(bytes).getUint64(at, Endian.little);
  if (number < 0) throw const _Refused('malformed');
  return number;
}

/// The whole of an answer, when it is no longer than [limit].
Future<Uint8List> _whole(HttpClientResponse answer, int limit) async {
  final bytes = BytesBuilder(copy: false);
  await for (final chunk in answer.timeout(_patience)) {
    // Thrown from in here, which lets go of the rest of the answer too.
    if (answer.contentLength > limit || bytes.length + chunk.length > limit) {
      throw const CloudError('too_large');
    }
    bytes.add(chunk);
  }
  return bytes.takeBytes();
}

/// Something the engine said that the door will not carry. The engine is
/// told what, and the connection ends there, because nothing after it can
/// be read as frames.
class _Refused implements Exception {
  final String code;
  const _Refused(this.code);
}

class OnlineDoor {
  final AccountState account;

  /// Tells the engine which port the door is on, or null once it has shut
  /// for good (`share_online_door`).
  final void Function(int? port) tell;

  /// Where Lumit's server is.
  final String base;

  OnlineDoor(this.account, this.tell, {String? base})
      : base = base ?? cloudUrl;

  final HttpClient _http = HttpClient()
    ..connectionTimeout = const Duration(seconds: 10)
    ..userAgent = 'Lumit';

  ServerSocket? _server;

  /// The port the door is on, or null while it is shut.
  int? get port => _server?.port;

  /// Every connection the engine has through the door now.
  final Set<Socket> _callers = {};

  /// The last thing the engine was told had failed, as the server's code or
  /// the door's own. What tells a join that found nothing from one that
  /// could not ask.
  String? lastFailure;

  /// Open the door if it is not open, tell the engine where it is, and give
  /// back its port. It then stays open until [dispose]: the engine dials
  /// again by itself whenever it has lost its way, and has to find the door
  /// where it was. Safe to call before every share, every join and every
  /// saved copy that is opened again. Null when this machine would not give
  /// it a port.
  Future<int?> open() async {
    if (_server != null) return port;
    try {
      // This machine only. Nothing outside it can reach the door.
      final server = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
      _server = server;
      server.listen(_serve, onError: (_) {});
      tell(server.port);
      return server.port;
    } catch (_) {
      return null;
    }
  }

  /// Shut the door for good, and end every connection through it.
  Future<void> dispose() async {
    await _shut();
    for (final local in _callers.toList()) {
      local.destroy();
    }
    _http.close(force: true);
  }

  Future<void> _shut() async {
    final server = _server;
    _server = null;
    if (server == null) return;
    try {
      tell(null);
    } catch (_) {
      // The engine has gone first.
    }
    await server.close();
  }

  Future<void> _serve(Socket local) async {
    _callers.add(local);
    try {
      await _Caller(this, local).serve();
    } finally {
      _callers.remove(local);
    }
  }

  Uri _at(String id, [String rest = '', Map<String, String>? query]) =>
      Uri.parse('$base/v1/online/$id$rest').replace(queryParameters: query);

  /// What every call carries: the project's access, and the account's token
  /// whenever someone is signed in, which is how the server knows the host.
  Future<Map<String, String>> _headers(String access) async {
    final token = await account.accessToken();
    return {
      'X-Lumit-Online': access,
      if (token != null) HttpHeaders.authorizationHeader: 'Bearer $token',
    };
  }

  Future<HttpClientRequest> _open(String method, Uri at, String access,
      {int? length, ContentType? type, Map<String, String>? more}) async {
    final headers = {...await _headers(access), ...?more};
    final request = await _http.openUrl(method, at);
    request.followRedirects = false;
    headers.forEach(request.headers.set);
    if (length != null) {
      request.headers.contentType = type ?? ContentType.binary;
      request.contentLength = length;
    }
    return request;
  }

  /// One call to Lumit's server, and its answer when that is a yes. [body]
  /// is handed over before the answer is waited for, so a long one on a slow
  /// line is not mistaken for a server that has gone quiet.
  Future<HttpClientResponse> _send(String method, Uri at, String access,
      {List<int>? body, ContentType? type, Map<String, String>? more}) async {
    final request = await _open(method, at, access,
        length: body?.length, type: type, more: more);
    if (body != null) {
      request.add(body);
      await request.flush();
    }
    return _answered(request);
  }

  /// The answer to [request] when it is a yes. Otherwise this throws what
  /// the server refused with, or `offline` when it never answered.
  Future<HttpClientResponse> _answered(HttpClientRequest request) async {
    final HttpClientResponse answer;
    try {
      answer = await request.close().timeout(_patience);
    } catch (_) {
      request.abort();
      throw const CloudError('offline');
    }
    if (answer.statusCode ~/ 100 == 2) return answer;
    var code = 'unknown';
    try {
      final said = jsonDecode(utf8.decode(await _whole(answer, _messageLimit)));
      if (said is Map && said['error'] is String) {
        code = said['error'] as String;
      }
    } catch (_) {
      // Not JSON: an error page from something in between.
    }
    throw CloudError(code, answer.statusCode);
  }

  /// What is stored at [at], a piece at a time as it arrives. Cut on its
  /// way, it asks for the rest from where it stopped, and goes on asking
  /// until nothing more has come for [_lineGone]. So a line that drops
  /// every few seconds carries a large thing once, and not its first part
  /// again and again.
  ///
  /// [began] is told of each answer before its pieces, and whether it
  /// carries on from the last one. When it does not, what came before is of
  /// something else and is thrown away: a newer snapshot has taken the place
  /// of the one that was coming. An empty piece is no bytes, and says only
  /// that the door is still asking.
  Stream<List<int>> _fetched(Uri at, String access,
      void Function(HttpClientResponse answer, bool rest) began) async* {
    var got = 0;
    String? which;
    var last = DateTime.now();
    while (true) {
      try {
        final answer = await _send('GET', at, access, more: {
          // As they are stored, or where to carry on from would mean nothing.
          HttpHeaders.acceptEncodingHeader: 'identity',
          if (got > 0) HttpHeaders.rangeHeader: 'bytes=$got-',
          if (got > 0 && which != null) 'X-Lumit-Snapshot': which,
        });
        final rest = answer.statusCode == HttpStatus.partialContent;
        final from = answer.headers.value(HttpHeaders.contentRangeHeader);
        if (rest && !(from ?? '').startsWith('bytes $got-')) {
          // Not the rest that was asked for, so nothing to join it to.
          answer.listen(null).cancel().ignore();
          throw const CloudError('unknown');
        }
        if (!rest) got = 0;
        which = answer.headers.value('x-lumit-snapshot');
        began(answer, rest);
        await for (final chunk in answer.timeout(_patience)) {
          got += chunk.length;
          last = DateTime.now();
          yield chunk;
        }
        return;
      } on CloudError catch (e) {
        // All of it was here, and only its end was lost.
        if (e.status == HttpStatus.requestedRangeNotSatisfiable && got > 0) {
          return;
        }
        if (!e.offline) rethrow;
      } catch (_) {
        // Cut on its way.
      }
      if (DateTime.now().difference(last) > _lineGone) {
        throw const CloudError('offline');
      }
      yield const <int>[];
      await Future<void>.delayed(const Duration(seconds: 1));
    }
  }
}

/// One caller at the door: one connection from the engine, for one project.
class _Caller {
  final OnlineDoor door;
  final Socket local;
  final StreamIterator<Uint8List> said;
  _Caller(this.door, this.local) : said = StreamIterator<Uint8List>(local);

  late final String id, access, token, name;

  /// What the engine has sent that no frame has used yet.
  Uint8List rest = Uint8List(0);

  /// The last frame said to the engine, done once the engine has taken it.
  Future<void> told = Future<void>.value();

  Future<void> serve() async {
    try {
      local.setOption(SocketOption.tcpNoDelay, true);
      // A write to an engine that has gone is how many of these end. It is
      // heard about where the write is waited on, and need not be here.
      local.done.ignore();
      final (line, after) = await RelayDoor.firstLine(said, _lineLimit);
      final named = _first.firstMatch(line);
      if (named == null) throw const _Refused('malformed');
      id = named[1]!;
      access = named[2]!;
      token = named[3]!;
      name = named[4]!;
      rest = after;
      for (var frame = await next(); frame != null; frame = await next()) {
        final payload = Uint8List.sublistView(frame, 1);
        switch (frame[0]) {
          case _create:
            await answer(() => create(payload));
          case _meta:
            await answer(meta);
          case _getSnapshot:
            await answer(getSnapshot);
          case _putSnapshot:
            await putSnapshot(payload);
          case _live:
            await live(payload);
          default:
            throw const _Refused('malformed');
        }
      }
    } on _Refused catch (e) {
      // Said, and this side closed, and then the engine is given a moment to
      // read that and go. A line shut over what the engine is still sending
      // takes the answer down with it.
      try {
        await fail(e.code)
            .then((_) => local.close())
            .then((_) => Future.doWhile(said.moveNext))
            .timeout(_leave);
      } catch (_) {
        // It went first, or is not listening.
      }
    } catch (_) {
      // Either end going is how every one of these finishes.
    } finally {
      await said.cancel();
      local.destroy();
    }
  }

  /// The next frame from the engine with its kind in front, as it would go
  /// down the socket, or null when the engine has closed. A frame longer
  /// than its kind may be is refused before any room is made for it.
  Future<Uint8List?> next() async {
    final head = await _take(Uint8List(5), 0);
    if (head == null) return null;
    final length = ByteData.sublistView(head).getUint32(1, Endian.little);
    final limit = switch (head[0]) {
      // ponytail: a batch is held whole on its way through, up to its
      // 64 MiB. Pour it into the upload as it comes if that ever shows.
      _append => 20 + _batchLimit,
      _part => _partLimit,
      // Everything else has to fit in one socket message.
      _ => _messageLimit - 1,
    };
    if (length > limit) throw const _Refused('too_large');
    return _take(Uint8List(1 + length)..[0] = head[0], 1);
  }

  /// Fill [into], from [from] on, with what the engine says next. Null when
  /// it closes first.
  Future<Uint8List?> _take(Uint8List into, int from) async {
    for (var at = from; at < into.length;) {
      if (rest.isEmpty) {
        if (!await said.moveNext()) return null;
        rest = said.current;
      }
      final some = min(into.length - at, rest.length);
      into.setRange(at, at + some, rest);
      rest = Uint8List.sublistView(rest, some);
      at += some;
    }
    return into;
  }

  /// One frame to the engine. Each waits for the one before it: a socket
  /// takes nothing more while it is still handing on the last, and frames
  /// from the two directions must not cut into each other.
  Future<void> say(int kind,
      [List<int> payload = const [], List<int> more = const []]) {
    return told = told.then((_) {
      local.add((ByteData(5)
            ..setUint8(0, kind)
            ..setUint32(1, payload.length + more.length, Endian.little))
          .buffer
          .asUint8List());
      local.add(payload);
      local.add(more);
      // Until this machine has taken them, which is what holds a fast
      // server back for a slow engine.
      return local.flush();
    });
  }

  Future<void> fail(String code) {
    door.lastFailure = code;
    return say(_failed, utf8.encode(jsonEncode({'error': code})));
  }

  /// One exchange with the server. Whatever goes wrong with it is said to
  /// the engine, and the connection carries on with its frames in step.
  Future<void> answer(Future<void> Function() exchange) async {
    try {
      await exchange();
    } on _Refused {
      rethrow;
    } catch (e) {
      await fail(e is CloudError ? e.code : 'offline');
    }
  }

  /// Put the project online. The engine says what it is and gives the seal
  /// key, wrapped. The access is added here, from the first line.
  Future<void> create(Uint8List payload) async {
    final Object? asked;
    try {
      asked = jsonDecode(utf8.decode(payload));
    } on FormatException {
      throw const _Refused('malformed');
    }
    if (asked is! Map) throw const _Refused('malformed');
    // Only an account can keep a project online, so there is nothing to ask
    // the server without one.
    if (await door.account.accessToken() == null) return fail('signed_out');
    final made = await door._send('POST', door._at(id), access,
        body: utf8.encode(jsonEncode({...asked, 'access': access})),
        type: ContentType.json);
    await made.drain<void>();
    await say(_ok);
  }

  Future<void> meta() async {
    final found =
        await door._send('GET', door._at(id, '', {'token': token}), access);
    await say(_metaIs, await _whole(found, _messageLimit));
  }

  /// The snapshot, handed to the engine a part at a time as it arrives. The
  /// engine keeps the parts until the end is said, and then opens the whole.
  /// Cut on its way, the rest is asked for and the parts carry on
  /// ([OnlineDoor._fetched]). If a newer snapshot has taken its place by
  /// then, the engine is told to throw away what it has and is sent the
  /// newer from its start. If the line is gone for good, it is told the
  /// fetch failed in place of the end.
  Future<void> getSnapshot() async {
    var seq = 0, size = 0, clean = false;
    final parts =
        door._fetched(door._at(id, '/snapshot'), access, (found, rest) {
      final at = int.tryParse(found.headers.value('x-lumit-seq') ?? '');
      if (at == null || at < 0) {
        // Nothing says where in the order it was taken, so it is no use.
        found.listen(null).cancel().ignore();
        throw const CloudError('unknown');
      }
      (seq, clean) = (at, !rest);
    });
    await for (final chunk in parts) {
      if (clean) {
        if (size > 0) await say(_againIs);
        (size, clean) = (0, false);
      }
      // Thrown from in here, which lets go of the rest of the answer too.
      if ((size += chunk.length) > _snapshotLimit) {
        throw const CloudError('too_large');
      }
      // An empty one is said too. It is the door still asking, and it keeps
      // the engine from taking the door for gone while the line is.
      for (var at = 0; at == 0 || at < chunk.length; at += _partLimit) {
        await say(
            _partIs, chunk.sublist(at, min(chunk.length, at + _partLimit)));
      }
    }
    await say(_endIs,
        (ByteData(8)..setUint64(0, seq, Endian.little)).buffer.asUint8List());
  }

  /// Store a snapshot. The engine says where in the order it was taken and
  /// how long it is, then sends it in parts, and each part goes on to the
  /// server before the next is read.
  ///
  /// A snapshot that moves the project forward to a newer Lumit says which
  /// after the length, as JSON: `{"version": "...", "schema": "..."}`. The
  /// server is told in two headers, and takes it only from the host.
  Future<void> putSnapshot(Uint8List payload) async {
    if (payload.length < 16) throw const _Refused('malformed');
    final seq = _u64(payload, 0);
    var left = _u64(payload, 8);
    if (left > _snapshotLimit) throw const _Refused('too_large');
    Map<String, String>? moved;
    if (payload.length > 16) {
      final Object? to;
      try {
        to = jsonDecode(utf8.decode(Uint8List.sublistView(payload, 16)));
      } on FormatException {
        throw const _Refused('malformed');
      }
      final version = to is Map ? to['version'] : null;
      final schema = to is Map ? to['schema'] : null;
      // What the server reads a version as. Anything else is no header.
      final word = RegExp(r'^[\x21-\x7e]{1,40}$');
      if (version is! String ||
          schema is! String ||
          !word.hasMatch(version) ||
          !word.hasMatch(schema)) {
        throw const _Refused('malformed');
      }
      moved = {'X-Lumit-Version': version, 'X-Lumit-Schema': schema};
    }
    HttpClientRequest? request;
    try {
      request = await door._open(
          'PUT', door._at(id, '/snapshot', {'seq': '$seq'}), access,
          length: left, more: moved);
    } catch (_) {
      // Not reached. Said once the parts are in.
    }
    // The engine sends its parts whatever becomes of them. They are read to
    // the end even when the server has stopped taking them, which keeps the
    // frames in step for whatever the engine asks next.
    try {
      while (true) {
        final frame = await next();
        if (frame == null) throw const SocketException('gone');
        if (frame[0] == _end) break;
        if (frame[0] != _part || (left -= frame.length - 1) < 0) {
          throw const _Refused('malformed');
        }
        try {
          request?.add(Uint8List.sublistView(frame, 1));
          // Until the line has taken it. A slow one holds the engine back,
          // and the snapshot is never here whole.
          await request?.flush();
        } catch (_) {
          request?.abort();
          request = null;
        }
      }
      if (left != 0) throw const _Refused('malformed');
    } catch (_) {
      request?.abort();
      rethrow;
    }
    final sent = request;
    await answer(() async {
      if (sent == null) throw const CloudError('offline');
      await (await door._answered(sent)).drain<void>();
      await say(_ok);
    });
  }

  /// The socket. From here the server's own messages pass both ways as
  /// frames, until one end goes.
  Future<void> live(Uint8List payload) async {
    if (payload.length != 8) throw const _Refused('malformed');
    final at = door._at(id, '/live',
        {'token': token, 'after': '${_u64(payload, 0)}', 'name': name});
    final WebSocket socket;
    try {
      // Nothing is squeezed on the way: what goes down it is sealed, and
      // sealed bytes do not get any smaller.
      final opening = WebSocket.connect(
          '${at.replace(scheme: at.scheme == 'http' ? 'ws' : 'wss')}',
          headers: await door._headers(access),
          compression: CompressionOptions.compressionOff);
      socket = await opening.timeout(_patience, onTimeout: () {
        // Given up on. If it opens after all it would sit at the server as
        // someone who is not there, so it is closed when it does.
        opening.then((opened) => opened.close()).ignore();
        throw TimeoutException(null);
      });
    } on WebSocketException catch (e) {
      // The server answered and would not open one. No project by that
      // name and the wrong access are the same answer.
      return fail(e.httpStatusCode == 404 ? 'not_found' : 'unknown');
    } catch (_) {
      return fail('offline');
    }
    socket.pingInterval = _alive;
    final heard = StreamIterator<dynamic>(socket);
    // What goes to the server. Its sink takes one stream at a time and says
    // when it is full by pausing this one, which is when the engine stops
    // being read.
    Completer<void>? flowing;
    final out = StreamController<dynamic>(
      onPause: () => flowing = Completer<void>(),
      onResume: () {
        flowing?.complete();
        flowing = null;
      },
    );
    var lost = false, gone = false;

    Future<void> up() async {
      for (var frame = await next(); frame != null; frame = await next()) {
        if (frame[0] >= _create) throw const _Refused('malformed');
        // Told already. It is read on only until the engine closes.
        if (lost) continue;
        var message = frame;
        if (frame[0] == _append && frame.length > _messageLimit) {
          final stored = await store(frame);
          if (stored == null) continue;
          message = stored;
        }
        await flowing?.future;
        if (!lost) out.add(message);
      }
      gone = true;
    }

    Future<void> down() async {
      // One at a time, each said to the engine before the next is taken
      // from the socket. A batch that has to be fetched holds back what
      // came after it, which is what keeps the order.
      while (await heard.moveNext()) {
        final message = heard.current;
        if (message is! Uint8List) continue;
        // Anything Lumit's server would not say ends the socket.
        // ponytail: room has been made for a message before its length can
        // be seen here, so this trusts the server not to send a vast one.
        // Reading the socket's frames by hand would let it be refused first.
        if (message.isEmpty ||
            message.length > _messageLimit ||
            message[0] >= _create) {
          return;
        }
        if (message[0] != _entryBlob) {
          await say(message[0], Uint8List.sublistView(message, 1));
          continue;
        }
        if (message.length != 33) return;
        final Uint8List sealed;
        // Nothing is read from the socket while the batch is fetched, so its
        // answer to being asked whether it is there would go unread, and a
        // fetch of half a minute on a slow line would drop a socket that had
        // nothing wrong with it. The fetch is what shows the line is there
        // meanwhile, and it gives up by itself when the line is not. Cut on
        // its way, it asks for the rest. If the socket went with the line,
        // that is found out once the batch is with the engine.
        socket.pingInterval = null;
        try {
          final blob = _hex(Uint8List.sublistView(message, 17));
          final bytes = BytesBuilder(copy: false);
          final pieces = door._fetched(door._at(id, '/blobs/$blob'), access,
              (_, rest) => rest ? null : bytes.clear());
          await for (final piece in pieces) {
            if (bytes.length + piece.length > _batchLimit) {
              throw const CloudError('too_large');
            }
            bytes.add(piece);
          }
          sealed = bytes.takeBytes();
        } catch (_) {
          // It can't be skipped, so the socket is given up. The engine asks
          // again from the last entry it has.
          return;
        }
        socket.pingInterval = _alive;
        await say(_entry, Uint8List.sublistView(message, 1, 17), sealed);
      }
    }

    final ups = up(), downs = down(), sending = socket.addStream(out.stream);
    final int code;
    try {
      await Future.any([ups, downs, sending]);
    } finally {
      lost = true;
      flowing?.complete();
      await heard.cancel();
      code = socket.closeCode ?? 0;
      // The server's sink is let go of before it is closed: a sink that is
      // still being fed can't be. A line that has stalled never says it
      // has finished, so this does not wait on one for long.
      try {
        await Future.wait([out.close(), sending]).timeout(_leave);
      } catch (_) {
        // Stalled, or already gone.
      }
      try {
        await socket.close().timeout(_leave);
      } catch (_) {
        // Still being fed down a stalled line. It stops answering when it
        // is asked whether it is there, and is dropped for that.
      }
    }
    // Whatever was on its way to the engine is said first, so that this is
    // the last thing it hears. Then the engine closes, and dials again when
    // it wants to.
    await downs;
    if (gone) return;
    await say(_lost, [code & 0xff, code >> 8]);
    await ups;
  }

  /// A batch too big for one socket message. It is kept on the server by
  /// itself, under a name made up here, and appended by that name. Null
  /// when the server would not keep it, which the engine has been told.
  Future<Uint8List?> store(Uint8List frame) async {
    final blob = [for (var i = 0; i < 16; i++) _random.nextInt(256)];
    var kept = false;
    await answer(() async {
      final put = await door._send(
          'PUT', door._at(id, '/blobs/${_hex(blob)}'), access,
          body: Uint8List.sublistView(frame, 21));
      await put.drain<void>();
      kept = true;
    });
    if (!kept) return null;
    return Uint8List(37)
      ..[0] = _appendBlob
      ..setRange(1, 21, frame, 1)
      ..setRange(21, 37, blob);
  }
}
