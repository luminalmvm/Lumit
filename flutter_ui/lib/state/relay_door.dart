// The door to Lumit's own relay.
//
// A relay is spoken to in plain lines over a plain connection, which is what
// lets anyone run one on whatever server they have. Lumit's own can't be
// reached that way: it sits behind a web address, where the only connections
// let in are web ones, and a host has to show the account it is signed in to.
//
// So the engine never dials it. It dials this door, on this machine, and says
// exactly what it would say to any relay. The door says the same thing to
// Lumit's relay over a WebSocket, adds the account's token for a host, and
// from then on passes bytes both ways without looking at them. They were
// encrypted with the invite's key before they got here.
//
// The door holds nothing between calls but the key a host's room was opened
// with, which is what lets that host, and nobody else, take a guest from it.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:lumit_flutter/src/rust/api/share.dart' as bridge_share;

import 'account.dart';

/// What every first line starts with, and how long one may be
/// (`lumit-relay`).
const String _call = 'LUMIT-RELAY 2 ';
const int _lineLimit = 128;

/// How long the relay has to answer a first line. The engine gives up on the
/// door after ten seconds, so there is nothing to gain by waiting longer.
const Duration _patience = Duration(seconds: 9);

/// How often the line to the relay is asked whether it is still there. One
/// that has died without saying so is dropped after two of these, and the
/// engine, hearing its connection go, tries again.
const Duration _alive = Duration(seconds: 15);

/// The most one door sends ahead of what the other has said it handed on.
/// Small enough that a full relay room holds little, and far more than an
/// edit ever is.
const int _window = 2 << 20;

class RelayDoor {
  final AccountState account;
  RelayDoor(this.account);

  ServerSocket? _server;

  /// The port the door is on, or null while it is shut.
  int? get port => _server?.port;

  /// For each room this machine hosts, the key its guests are taken with.
  final Map<String, String> _takeKeys = {};

  /// Open the door if it is not open, and tell the engine where it is. Safe
  /// to call before every share and every join. False when this machine
  /// would not give it a port, which leaves sharing as it was without one.
  Future<bool> open() async {
    if (_server != null) return true;
    try {
      // This machine only. Nothing outside it can reach the door.
      final server = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
      _server = server;
      server.listen(_serve, onError: (_) {});
      bridge_share.shareCloudDoor(port: server.port);
      return true;
    } catch (_) {
      return false;
    }
  }

  Future<void> close() async {
    final server = _server;
    _server = null;
    if (server == null) return;
    try {
      bridge_share.shareCloudDoor(port: null);
    } catch (_) {
      // The engine has gone first.
    }
    await server.close();
  }

  Uri _at(String path) {
    final base = Uri.parse(cloudUrl);
    return base.replace(
        scheme: base.scheme == 'http' ? 'ws' : 'wss', path: path);
  }

  /// One caller at the door: read which of the three it is, and be that to
  /// the relay.
  Future<void> _serve(Socket local) async {
    local.setOption(SocketOption.tcpNoDelay, true);
    final said = StreamIterator<Uint8List>(local);
    WebSocket? relay;
    try {
      final (line, rest) = await _firstLine(said);
      final words = line.startsWith(_call)
          ? line.substring(_call.length).split(' ')
          : const <String>[];
      final room = words.length > 1 ? words[1] : '';
      if (!RegExp(r'^[0-9a-fA-F]{16,64}$').hasMatch(room)) {
        return _refuse(local, 'NO');
      }
      switch (words[0]) {
        case 'HOST':
          final token = await account.accessToken();
          if (token == null) return _refuse(local, 'NO');
          try {
            relay = await WebSocket.connect('${_at('/v1/relay/$room/host')}',
                    headers: {'Authorization': 'Bearer $token'})
                .timeout(_patience);
          } on WebSocketException {
            // A room that is already somebody's, or an account without Pro.
            return _refuse(local, 'BUSY');
          }
          relay.pingInterval = _alive;
          await _host(room, local, said, rest, relay);
        case 'JOIN':
          relay = await WebSocket.connect('${_at('/v1/relay/$room/join')}')
              .timeout(_patience);
          relay.pingInterval = _alive;
          await _pass(local, said, rest, relay);
        case 'TAKE' when words.length > 3 && _takeKeys[room] != null:
          // The engine names the room's token before the guest. It was given
          // none by this door, which keeps the relay's own key for the room.
          // The key goes in a header. An address is what ends up in logs.
          relay = await WebSocket.connect(
                  '${_at('/v1/relay/$room/take/${words[3]}')}',
                  headers: {'X-Lumit-Take': _takeKeys[room]!})
              .timeout(_patience);
          relay.pingInterval = _alive;
          await _pass(local, said, rest, relay);
        default:
          return _refuse(local, 'NO');
      }
    } catch (_) {
      // Either end going is how every one of these finishes.
    } finally {
      await said.cancel();
      local.destroy();
      try {
        await relay?.close();
      } catch (_) {
        // Still being fed down a stalled line. The relay closes a pair
        // that has gone quiet, which this one now has.
      }
    }
  }

  void _refuse(Socket local, String word) {
    local.add(ascii.encode('$word\n'));
  }

  /// The first line a caller says, and whatever came after it in the same
  /// read, which belongs to the other end.
  Future<(String, Uint8List)> _firstLine(StreamIterator<Uint8List> said) async {
    final line = BytesBuilder(copy: false);
    while (await said.moveNext().timeout(_patience)) {
      final chunk = said.current;
      final end = chunk.indexOf(0x0a);
      if (end < 0) {
        line.add(chunk);
        if (line.length > _lineLimit) break;
        continue;
      }
      line.add(Uint8List.sublistView(chunk, 0, end));
      if (line.length > _lineLimit) break;
      return (
        ascii.decode(line.takeBytes(), allowInvalid: true).trim(),
        Uint8List.sublistView(chunk, end + 1),
      );
    }
    throw const SocketException('no first line');
  }

  /// A host's room. The relay says `OK` and the key guests are taken with,
  /// then `GUEST n` each time someone comes. The engine says `PING` every few
  /// seconds, which the relay answers without waking anything.
  Future<void> _host(String room, Socket local, StreamIterator<Uint8List> said,
      Uint8List rest, WebSocket relay) async {
    final heard = StreamIterator<dynamic>(relay);
    String? key;
    try {
      if (!await heard.moveNext().timeout(_patience)) return;
      final opened = heard.current;
      if (opened is! String || !opened.startsWith('OK ')) {
        return _refuse(local, 'NO');
      }
      key = opened.substring(3).trim();
      _takeKeys[room] = key;
      local.add(ascii.encode('OK\n'));
      // The engine's pings, onward. Their words are short, so a read is
      // whole lines or near enough that the relay does not mind.
      final pings = () async {
        var chunk = rest;
        do {
          for (final _ in ascii
              .decode(chunk, allowInvalid: true)
              .split('\n')
              .where((line) => line.trim() == 'PING')) {
            relay.add('PING');
          }
          if (!await said.moveNext()) break;
          chunk = said.current;
        } while (true);
      }();
      final guests = () async {
        while (await heard.moveNext()) {
          final word = heard.current;
          if (word is String && word.startsWith('GUEST ')) {
            local.add(ascii.encode('$word\n'));
          }
        }
      }();
      // Whichever end goes first takes the room with it.
      await Future.any([pings, guests]);
    } finally {
      // Only its own key. A host that came back for the same room before
      // this connection had finished going has already left a newer one.
      if (_takeKeys[room] == key) _takeKeys.remove(room);
      await heard.cancel();
    }
  }

  /// A guest asking for its host, or a host taking a guest: the relay says
  /// `OK` once the two are joined, and from then on it is bytes both ways.
  ///
  /// Nothing between the two doors holds a sender back by itself: the relay
  /// takes whatever it is sent and keeps it until the other end has read it.
  /// A clip sent to someone on a slow line would pile up there. So each door
  /// tells the other how much it has handed on (`ACK n`), and a door stops
  /// reading from its own side while more than [_window] bytes are out with
  /// no word back. The engine then waits, as it would on any slow line.
  Future<void> _pass(Socket local, StreamIterator<Uint8List> said,
      Uint8List rest, WebSocket relay) async {
    final heard = StreamIterator<dynamic>(relay);
    // What goes to the relay: bytes, and this door's own acks between them.
    // The relay's sink takes one stream at a time, so both go down this one.
    Completer<void>? flowing, room;
    final out = StreamController<dynamic>(
      onPause: () => flowing = Completer<void>(),
      onResume: () {
        flowing?.complete();
        flowing = null;
      },
    );
    var sent = 0, acked = 0, received = 0, told = 0;
    Future<void>? sending;
    try {
      // A guest waits here until its host takes it, which the relay gives
      // fifteen seconds.
      if (!await heard.moveNext().timeout(const Duration(seconds: 20))) return;
      if (heard.current != 'OK') return _refuse(local, 'NONE');
      local.add(ascii.encode('OK\n'));

      Future<void> up() async {
        Future<void> put(Uint8List chunk) async {
          while (sent - acked > _window) {
            await (room = Completer<void>()).future;
          }
          await flowing?.future;
          out.add(chunk);
          sent += chunk.length;
        }

        if (rest.isNotEmpty) await put(rest);
        while (await said.moveNext()) {
          await put(said.current);
        }
      }

      Future<void> down() async {
        while (await heard.moveNext()) {
          final message = heard.current;
          if (message is String) {
            final so = message.startsWith('ACK ')
                ? int.tryParse(message.substring(4))
                : null;
            if (so != null && so > acked) {
              acked = so;
              room?.complete();
              room = null;
            }
            continue;
          }
          final bytes = message as List<int>;
          local.add(bytes);
          // Until this machine has taken them, which is what is being
          // counted.
          await local.flush();
          received += bytes.length;
          if (received - told >= _window ~/ 4) {
            told = received;
            out.add('ACK $received');
          }
        }
      }

      sending = relay.addStream(out.stream);
      await Future.any([up(), down(), sending]);
    } finally {
      room?.complete();
      flowing?.complete();
      await heard.cancel();
      // The relay's sink is let go of before anyone closes it: a sink that
      // is still being fed can't be closed. A line that has stalled never
      // says it has finished, so this does not wait on one for long.
      if (sending == null) {
        unawaited(out.close());
      } else {
        try {
          await Future.wait([out.close(), sending])
              .timeout(const Duration(seconds: 2));
        } catch (_) {
          // Stalled, or already gone.
        }
      }
    }
  }
}
