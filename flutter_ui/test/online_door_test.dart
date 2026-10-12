// The door to a project that is kept online, between a made-up engine and a
// made-up server.
//
// What this guards is people's work. An entry that reaches the engine out of
// order, or not at all, is an edit lost. So it checks that what goes in at
// one end comes out of the other whole and in order, and that nothing an
// engine or a server does takes the door down.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/account.dart';
import 'package:lumit_flutter/state/online_door.dart';

// The kinds of frame, as `online_door.dart` has them.
const append = 0x01, hello = 0x10, entry = 0x11, ack = 0x13, stale = 0x14;
const create = 0x20, meta = 0x21, getSnapshot = 0x22;
const putSnapshot = 0x23, part = 0x24, end = 0x25, live = 0x26;
const ok = 0x30, metaIs = 0x31, partIs = 0x32, endIs = 0x33;
const againIs = 0x34;
const lost = 0x3e, failed = 0x3f;

const mebibyte = 1 << 20;

Uint8List bytes(List<List<int>> parts) {
  final all = BytesBuilder();
  parts.forEach(all.add);
  return all.takeBytes();
}

Uint8List u32(int n) =>
    (ByteData(4)..setUint32(0, n, Endian.little)).buffer.asUint8List();

Uint8List u64(int n) =>
    (ByteData(8)..setUint64(0, n, Endian.little)).buffer.asUint8List();

int readU64(Uint8List from, [int at = 0]) =>
    ByteData.sublistView(from).getUint64(at, Endian.little);

/// Bytes nobody could mistake for another lot: a sealed thing, to the door.
Uint8List sealed(int length, int seed) {
  final random = Random(seed);
  return Uint8List.fromList(
      [for (var i = 0; i < length; i++) random.nextInt(256)]);
}

bool same(List<int> a, List<int> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

/// One batch of one edit, as the engine appends it.
Uint8List batch(int base, Uint8List sealed) =>
    bytes([u64(base), u64(base + 100), u32(1), sealed]);

const a = 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const b = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

/// The first line for project [id]. Its access is the id said twice, so a
/// call that reached the wrong project would be turned away.
String line(String id, {int token = 7}) =>
    'LUMIT-ONLINE 1 $id $id$id $token ${base64Url.encode(sealed(48, 1))}';

final asked = utf8.encode(jsonEncode(
    {'protocol': 1, 'version': '0.6.0', 'schema': '12', 'wrap': 'ab' * 60}));

String error(Uint8List payload) =>
    (jsonDecode(utf8.decode(payload)) as Map)['error'] as String;

/// An account that is signed in when it has a token, with no server asked.
class _Account extends AccountState {
  final String? token;
  _Account(this.token);

  @override
  Future<String?> accessToken() async => token;
}

/// The engine's end of one connection to the door.
class _Engine {
  final Socket socket;
  final StreamIterator<Uint8List> _said;
  Uint8List _rest = Uint8List(0);
  _Engine(this.socket) : _said = StreamIterator<Uint8List>(socket);

  static Future<_Engine> dial(OnlineDoor door, String line) async {
    final socket = await Socket.connect(
        InternetAddress.loopbackIPv4, (await door.open())!);
    socket.done.ignore();
    socket.add(ascii.encode('$line\n'));
    return _Engine(socket);
  }

  void send(int kind, [List<int> payload = const []]) =>
      socket.add(bytes([
        [kind],
        u32(payload.length),
        payload
      ]));

  /// The next frame the door says, or null once it has closed.
  Future<(int, Uint8List)?> next() async {
    final head = await _take(5);
    if (head == null) return null;
    final payload =
        await _take(ByteData.sublistView(head).getUint32(1, Endian.little));
    return payload == null ? null : (head[0], payload);
  }

  /// The payload of the next frame, which has to be of [kind].
  Future<Uint8List> take(int kind) async {
    final frame = await next();
    expect(frame?.$1, kind);
    return frame!.$2;
  }

  /// The next entry: where it sits in the order, and what was sealed.
  Future<(int, Uint8List)> nextEntry() async {
    final payload = await take(entry);
    return (readU64(payload), Uint8List.sublistView(payload, 16));
  }

  Future<Uint8List?> _take(int count) async {
    final taken = Uint8List(count);
    for (var at = 0; at < count;) {
      if (_rest.isEmpty) {
        if (!await _said.moveNext().timeout(const Duration(seconds: 20))) {
          return null;
        }
        _rest = _said.current;
      }
      final some = min(count - at, _rest.length);
      taken.setRange(at, at + some, _rest);
      _rest = Uint8List.sublistView(_rest, some);
      at += some;
    }
    return taken;
  }

  Future<void> close() async {
    await _said.cancel();
    socket.destroy();
  }
}

class _Project {
  final String access, host, wrap;
  _Project(this.access, this.host, this.wrap);

  int head = 0;
  int? snapshotSeq;
  Uint8List? snapshot;

  /// The stored snapshot's own name, which a fetch that was cut gives back
  /// to ask for the rest of it.
  String snapshotName = 'snapshot/first';

  /// The Lumit version and schema the last snapshot said take the project
  /// over, each as its header had it.
  (String?, String?)? moved;

  /// Where each entry sits in the order, then its sealed batch, or the name
  /// of the blob that holds it.
  final entries = <(int, Uint8List, bool)>[];
  final blobs = <String, Uint8List>{};
  final sockets = <WebSocket>[];
}

/// Just enough of Lumit's server for the door to speak to: the calls it
/// makes and the socket, with the append that only goes in at the head.
class _Server {
  final HttpServer http;
  _Server(this.http) {
    http.listen(_answer);
  }

  static Future<_Server> start() async =>
      _Server(await HttpServer.bind(InternetAddress.loopbackIPv4, 0));

  String get base => 'http://127.0.0.1:${http.port}';

  final projects = <String, _Project>{};

  /// Every call but the socket: what it was, the length it said its body
  /// had, the access it showed and who it said it was from.
  final calls = <(String, int, String?, String?)>[];

  /// How long a blob takes to fetch.
  Duration slowBlobs = Duration.zero;

  /// How many bytes of a snapshot or a blob go out before the line is cut,
  /// for each of the next fetches, and what happens at the server just
  /// after a cut. Then where each fetch asked to start from.
  final cuts = <int>[];
  void Function()? onCut;
  final from = <int>[];

  /// Sealed bytes on their way out, from wherever the fetch asked to carry
  /// on, and cut short when a cut is waiting. [name] is the snapshot's: the
  /// rest of one is only given to a fetch that names it.
  Future<void> _bytes(HttpRequest request, Uint8List whole,
      [String? name]) async {
    final asked = RegExp(r'^bytes=(\d+)-$')
        .firstMatch(request.headers.value('range') ?? '');
    final rest = asked != null &&
        (name == null || request.headers.value('x-lumit-snapshot') == name);
    final start = rest ? int.parse(asked[1]!) : 0;
    from.add(start);
    final answer = request.response;
    if (name != null) answer.headers.set('X-Lumit-Snapshot', name);
    if (rest) {
      answer.statusCode = HttpStatus.partialContent;
      answer.headers.set(HttpHeaders.contentRangeHeader,
          'bytes $start-${whole.length - 1}/${whole.length}');
    }
    final body = Uint8List.sublistView(whole, start);
    answer.contentLength = body.length;
    final cut = cuts.isEmpty ? body.length : cuts.removeAt(0);
    if (cut >= body.length) {
      answer.add(body);
      return answer.close();
    }
    // Some of it, and then the line goes.
    final line = await answer.detachSocket();
    line.add(Uint8List.sublistView(body, 0, cut));
    await line.flush();
    onCut?.call();
    line.destroy();
  }

  Future<void> _answer(HttpRequest request) async {
    final at = request.uri.pathSegments;
    final id = at[2];
    final what = [request.method, ...at.skip(3)].join(' ');
    final access = request.headers.value('x-lumit-online');
    final bearer = request.headers.value('authorization');
    final query = request.uri.queryParameters;

    Future<void> reply(int status, {Object? json, List<int>? body}) {
      request.response.statusCode = status;
      if (json != null) request.response.write(jsonEncode(json));
      if (body != null) request.response.add(body);
      return request.response.close();
    }

    if (what == 'GET live') {
      final project = projects[id];
      if (project == null || access != project.access) {
        return reply(404, json: {'error': 'not_found'});
      }
      return _live(project, await WebSocketTransformer.upgrade(request),
          int.parse(query['after']!));
    }
    final body = bytes(await request.toList());
    calls.add((what, request.contentLength, access, bearer));
    if (what == 'POST') {
      final made = jsonDecode(utf8.decode(body)) as Map;
      if (bearer == null) return reply(401, json: {'error': 'signed_out'});
      if (projects.containsKey(id)) {
        return reply(409, json: {'error': 'exists'});
      }
      projects[id] =
          _Project(made['access'] as String, bearer, made['wrap'] as String);
      return reply(201);
    }
    final project = projects[id];
    if (project == null || access != project.access) {
      return reply(404, json: {'error': 'not_found'});
    }
    switch (what) {
      case 'GET':
        return reply(200, json: {
          'wrap': project.wrap,
          'snapshot': project.snapshotSeq,
          'head': project.head,
          'host': bearer == project.host,
          'token': query['token'],
        });
      case 'PUT snapshot':
        project.snapshot = body;
        project.snapshotSeq = int.parse(query['seq']!);
        project.moved = (
          request.headers.value('x-lumit-version'),
          request.headers.value('x-lumit-schema')
        );
        return reply(204);
      case 'GET snapshot':
        request.response.headers.set('X-Lumit-Seq', '${project.snapshotSeq}');
        return _bytes(request, project.snapshot!, project.snapshotName);
    }
    if (what.startsWith('PUT blobs ')) {
      project.blobs[at[4]] = body;
      return reply(204);
    }
    if (what.startsWith('GET blobs ') && project.blobs[at[4]] != null) {
      await Future<void>.delayed(slowBlobs);
      return _bytes(request, project.blobs[at[4]]!);
    }
    return reply(404, json: {'error': 'not_found'});
  }

  void _live(_Project project, WebSocket socket, int after) {
    final seat = project.sockets.length + 1;
    Uint8List told(int seq, int from, Uint8List what, bool blob) =>
        bytes([
          [blob ? 0x12 : 0x11],
          u64(seq),
          u32(1),
          u32(from),
          what
        ]);
    socket.add(bytes([
      [hello],
      u32(seat),
      [seat],
      u64(project.head),
      u64(0),
      [1]
    ]));
    for (final (seq, what, blob) in project.entries) {
      if (seq > after) socket.add(told(seq, 0xffffffff, what, blob));
    }
    project.sockets.add(socket);
    socket.listen((message) {
      final said = message as Uint8List;
      if (said[0] != 0x01 && said[0] != 0x02) return;
      if (readU64(said, 1) != project.head) {
        return socket.add(bytes([
          [stale],
          u64(project.head)
        ]));
      }
      final blob = said[0] == 0x02;
      final what = Uint8List.sublistView(said, 21);
      project.entries.add((++project.head, what, blob));
      socket.add(bytes([
        [ack],
        u64(project.head)
      ]));
      for (final other in project.sockets) {
        if (other != socket) other.add(told(project.head, seat, what, blob));
      }
    }, onDone: () => project.sockets.remove(socket));
  }
}

void main() {
  late _Server server;
  final doors = <OnlineDoor>[];

  OnlineDoor door({String? signedIn, void Function(int?)? tell}) {
    final door =
        OnlineDoor(_Account(signedIn), tell ?? (_) {}, base: server.base);
    doors.add(door);
    return door;
  }

  setUp(() async => server = await _Server.start());

  tearDown(() async {
    for (final door in doors) {
      await door.dispose();
    }
    doors.clear();
    await server.http.close(force: true);
  });

  test('a project is made, given a snapshot, and goes live', () async {
    final told = <int?>[];
    final host =
        await _Engine.dial(door(signedIn: 'mack', tell: told.add), line(a));
    host.send(create, asked);
    await host.take(ok);
    final made = server.projects[a]!;
    expect((made.access, made.host, made.wrap),
        ('$a$a', 'Bearer mack', 'ab' * 60));

    host.send(meta);
    final found = jsonDecode(utf8.decode(await host.take(metaIs))) as Map;
    expect((found['snapshot'], found['head'], found['host'], found['token']),
        (null, 0, true, '7'));

    // Two and a half parts, so the last one is short.
    final snapshot = sealed(5 * mebibyte ~/ 2, 2);
    host.send(putSnapshot, bytes([u64(0), u64(snapshot.length)]));
    for (var at = 0; at < snapshot.length; at += mebibyte) {
      host.send(
          part,
          Uint8List.sublistView(
              snapshot, at, min(snapshot.length, at + mebibyte)));
    }
    host.send(end);
    await host.take(ok);
    expect(made.snapshotSeq, 0);
    expect(same(made.snapshot!, snapshot), isTrue);
    // The server was told how long it was before any of it came, which is
    // what lets it be poured straight into storage.
    expect(server.calls.last.$2, snapshot.length);

    host.send(live, u64(0));
    await host.take(hello);
    expect(server.calls.map((call) => call.$3), everyElement('$a$a'));

    // The door outlives its last caller, so an engine that dials again finds
    // it where it was. It goes when Lumit does, and tells the engine it has.
    await host.close();
    await Future<void>.delayed(const Duration(milliseconds: 50));
    expect(told, [isNotNull]);
    await doors.single.dispose();
    expect(told, [isNotNull, null]);
  });

  test('a snapshot that moves the project forward says which Lumit takes it '
      'over', () async {
    final host = await _Engine.dial(door(signedIn: 'mack'), line(a));
    host.send(create, asked);
    await host.take(ok);
    final made = server.projects[a]!;
    final snapshot = sealed(100, 3);
    void put(List<int> said) {
      host.send(putSnapshot, bytes([u64(0), u64(snapshot.length), said]));
      host.send(part, snapshot);
      host.send(end);
    }

    // Any other snapshot says nothing, and the server is told nothing.
    put([]);
    await host.take(ok);
    expect(made.moved, (null, null));

    put(utf8.encode(jsonEncode({'version': '0.7.0', 'schema': '0.4.0'})));
    await host.take(ok);
    expect(made.moved, ('0.7.0', '0.4.0'));
    // From the host, which is the only one the server takes it from.
    expect(server.calls.last.$4, 'Bearer mack');

    // Half of it is none of it, and the server is not asked.
    put(utf8.encode(jsonEncode({'version': '0.8.0'})));
    expect(error(await host.take(failed)), 'malformed');
    expect(made.moved, ('0.7.0', '0.4.0'));
  });

  test('someone who comes later gets the snapshot whole, then every entry in '
      'order, a big one among them', () async {
    final host = await _Engine.dial(door(signedIn: 'mack'), line(a));
    host.send(create, asked);
    await host.take(ok);
    final snapshot = sealed(3 * mebibyte, 3);
    host.send(putSnapshot, bytes([u64(0), u64(snapshot.length)]));
    for (var at = 0; at < snapshot.length; at += mebibyte) {
      host.send(part, Uint8List.sublistView(snapshot, at, at + mebibyte));
    }
    host.send(end);
    await host.take(ok);
    host.send(live, u64(0));
    await host.take(hello);

    // Three edits, the second too big for one socket message.
    final edits = [
      sealed(300, 4),
      sealed(600 << 10, 5),
      sealed(300, 6),
      sealed(300, 7),
      sealed(300, 8),
      sealed(700 << 10, 9),
      sealed(300, 10),
    ];
    for (var i = 0; i < 3; i++) {
      host.send(append, batch(i, edits[i]));
      expect(readU64(await host.take(ack)), i + 1);
    }
    final kept = server.projects[a]!;
    expect(kept.entries.map((entry) => entry.$3), [false, true, false]);
    expect(same(kept.blobs.values.single, edits[1]), isTrue);

    // A guest, through a door of its own, with nobody signed in. The blob
    // is slow to fetch, so the entry after it is at the door first and has
    // to wait there.
    server.slowBlobs = const Duration(milliseconds: 300);
    final guest = await _Engine.dial(door(), line(a, token: 8));
    guest.send(meta);
    final found = jsonDecode(utf8.decode(await guest.take(metaIs))) as Map;
    expect((found['snapshot'], found['head'], found['host']), (0, 3, false));
    guest.send(getSnapshot);
    final fetched = BytesBuilder();
    while (true) {
      final (kind, payload) = (await guest.next())!;
      if (kind == endIs) {
        expect(readU64(payload), 0);
        break;
      }
      expect(kind, partIs);
      expect(payload.length, lessThanOrEqualTo(mebibyte));
      fetched.add(payload);
    }
    expect(same(fetched.takeBytes(), snapshot), isTrue);
    guest.send(live, u64(0));
    await guest.take(hello);
    for (var i = 0; i < 3; i++) {
      final (seq, what) = await guest.nextEntry();
      expect(seq, i + 1);
      expect(same(what, edits[i]), isTrue);
    }

    // Both there at once. An append on a head that has moved is told so,
    // after the entry that moved it, and goes in on the new one.
    host.send(append, batch(3, edits[3]));
    expect(readU64(await host.take(ack)), 4);
    guest.send(append, batch(3, edits[4]));
    expect(same((await guest.nextEntry()).$2, edits[3]), isTrue);
    expect(readU64(await guest.take(stale)), 4);
    guest.send(append, batch(4, edits[4]));
    expect(readU64(await guest.take(ack)), 5);
    expect(same((await host.nextEntry()).$2, edits[4]), isTrue);

    // A big one and a small one straight after it arrive in that order.
    host.send(append, batch(5, edits[5]));
    expect(readU64(await host.take(ack)), 6);
    host.send(append, batch(6, edits[6]));
    expect(readU64(await host.take(ack)), 7);
    for (final i in [5, 6]) {
      final (seq, what) = await guest.nextEntry();
      expect(seq, i + 1);
      expect(same(what, edits[i]), isTrue);
    }
  });

  test('the server letting go of the socket is said as lost', () async {
    final one = door(signedIn: 'mack');
    final host = await _Engine.dial(one, line(a));
    host.send(create, asked);
    await host.take(ok);
    host.send(live, u64(0));
    await host.take(hello);
    // Closed the way a machine that is behind the snapshot is.
    await server.projects[a]!.sockets.single.close(4409);
    expect(await host.take(lost), [0x39, 0x11]);
    await host.close();

    // The engine dials again and is answered. Then the server goes
    // altogether, and that is an answer too.
    final again = await _Engine.dial(one, line(a));
    again.send(meta);
    await again.take(metaIs);
    await server.http.close(force: true);
    again.send(meta);
    expect(error(await again.take(failed)), 'offline');
  });

  test('a wrong first line and a frame over its cap are refused, and the '
      'door stays', () async {
    final one = door();
    final wrong = await _Engine.dial(one, 'LUMIT-RELAY 1 HOST $a');
    expect(error(await wrong.take(failed)), 'malformed');
    expect(await wrong.next(), isNull);

    // A batch one byte longer than a batch may be. Only its length is sent,
    // which is all the door needs to refuse it.
    final vast = await _Engine.dial(one, line(a));
    vast.socket.add(bytes([
      [append],
      u32(20 + 64 * mebibyte + 1)
    ]));
    expect(error(await vast.take(failed)), 'too_large');
    expect(await vast.next(), isNull);

    // What the server refuses is passed on, and the connection goes on to
    // the next thing asked. So does the door.
    final nobody = await _Engine.dial(one, line(a));
    nobody.send(meta);
    expect(error(await nobody.take(failed)), 'not_found');
    nobody.send(create, asked);
    expect(error(await nobody.take(failed)), 'signed_out');
    nobody.send(live, u64(0));
    expect(error(await nobody.take(failed)), 'not_found');
    nobody.send(meta);
    expect(error(await nobody.take(failed)), 'not_found');
  });

  test('two projects through one door do not cross', () async {
    final one = door(signedIn: 'mack');
    final first = await _Engine.dial(one, line(a, token: 1));
    final second = await _Engine.dial(one, line(b, token: 2));
    for (final engine in [first, second]) {
      engine.send(create, asked);
      await engine.take(ok);
      engine.send(live, u64(0));
      await engine.take(hello);
    }
    final watching = await _Engine.dial(one, line(a, token: 3));
    watching.send(live, u64(0));
    await watching.take(hello);

    final edits = [sealed(200, 11), sealed(200, 12), sealed(200, 13)];
    first.send(append, batch(0, edits[0]));
    second.send(append, batch(0, edits[1]));
    expect(readU64(await first.take(ack)), 1);
    expect(readU64(await second.take(ack)), 1);
    first.send(append, batch(1, edits[2]));
    expect(readU64(await first.take(ack)), 2);

    // The other person in the first project hears both of its edits and
    // nothing of the second project's.
    expect(same((await watching.nextEntry()).$2, edits[0]), isTrue);
    expect(same((await watching.nextEntry()).$2, edits[2]), isTrue);
    expect(server.projects[a]!.entries.length, 2);
    expect(same(server.projects[b]!.entries.single.$2, edits[1]), isTrue);
  });

  test('a snapshot or a big batch cut on its way carries on from where it '
      'stopped, and a snapshot replaced meanwhile starts clean', () async {
    final host = await _Engine.dial(door(signedIn: 'mack'), line(a));
    host.send(create, asked);
    await host.take(ok);
    Future<void> store(Uint8List snapshot, int seq) async {
      host.send(putSnapshot, bytes([u64(seq), u64(snapshot.length)]));
      host.send(part, snapshot);
      host.send(end);
      await host.take(ok);
    }

    final first = sealed(900 << 10, 21);
    await store(first, 0);
    final made = server.projects[a]!;

    // What the engine ends up holding of a fetch: the snapshot, where in
    // the order it was taken, and how often it was told to start again.
    Future<(Uint8List, int, int)> fetch(_Engine engine) async {
      engine.send(getSnapshot);
      final all = BytesBuilder();
      var again = 0;
      while (true) {
        final (kind, payload) = (await engine.next())!;
        if (kind == endIs) return (all.takeBytes(), readU64(payload), again);
        if (kind == againIs) {
          all.clear();
          again++;
        } else {
          expect(kind, partIs);
          all.add(payload);
        }
      }
    }

    // Cut twice. Each time the door asks for the rest, and the engine gets
    // every byte once.
    final guest = await _Engine.dial(door(), line(a, token: 8));
    server.cuts.addAll([300 << 10, 200 << 10]);
    var (got, seq, again) = await fetch(guest);
    expect((same(got, first), seq, again), (true, 0, 0));
    expect(server.from, [0, 300 << 10, 500 << 10]);

    // Cut, and a newer snapshot has taken its place before the rest is
    // asked for. The engine is told to start again and gets the newer whole.
    final second = sealed(400 << 10, 22);
    server
      ..from.clear()
      ..cuts.add(100 << 10)
      ..onCut = () {
        made
          ..snapshot = second
          ..snapshotSeq = 5
          ..snapshotName = 'snapshot/second';
      };
    (got, seq, again) = await fetch(guest);
    expect((same(got, second), seq, again), (true, 5, 1));
    expect(server.from, [0, 0]);

    // A big batch on its way down, cut the same way.
    host.send(live, u64(5));
    await host.take(hello);
    made.head = 5;
    final big = sealed(700 << 10, 23);
    host.send(append, batch(5, big));
    expect(readU64(await host.take(ack)), 6);
    server
      ..from.clear()
      ..onCut = null
      ..cuts.add(250 << 10);
    guest.send(live, u64(5));
    await guest.take(hello);
    final (at, what) = await guest.nextEntry();
    expect((at, same(what, big)), (6, true));
    expect(server.from, [0, 250 << 10]);
  });
}
