// A file on its way to Lumit's server.
//
// A review link's video goes that way, in the pieces the server asks for.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;

import 'account.dart';

/// [bytes] at no more than [limit] bytes a second, which is asked for each
/// chunk, so a limit changed part of the way takes effect at once. Nought is
/// as fast as the line goes.
Stream<List<int>> paced(Stream<List<int>> bytes, int Function() limit) async* {
  final clock = Stopwatch()..start();
  await for (final chunk in bytes) {
    yield chunk;
    final rate = limit();
    if (rate > 0) {
      // How long this chunk ought to have taken, less how long it did.
      final due = Duration(microseconds: chunk.length * 1000000 ~/ rate) -
          clock.elapsed;
      if (due > Duration.zero) await Future<void>.delayed(due);
    }
    clock.reset();
  }
}

/// Send [file], [size] bytes long, in the [count] pieces of [partSize] the
/// server asked for, and come back with what it called each. A piece that
/// fails is tried three more times.
///
/// [at] is where a piece goes, and [headers] what goes with it, asked for on
/// each try because a token runs out. [onProgress] hears how many bytes have
/// gone, ten times a second at most. [check] throws once the send has been
/// stopped. [limit] is the most bytes a second, as [paced] has it.
Future<List<Map<String, Object>>> sendParts(
  HttpClient http,
  File file, {
  required int size,
  required int partSize,
  required int count,
  required Uri Function(int part) at,
  required Future<Map<String, String>> Function() headers,
  required void Function(int done) onProgress,
  required void Function() check,
  int Function()? limit,
}) async {
  if (partSize <= 0 || count <= 0) throw const CloudError('unknown');
  final parts = <Map<String, Object>>[];
  var told = DateTime.now();
  for (var part = 1; part <= count; part++) {
    final from = (part - 1) * partSize;
    final to = math.min(size, from + partSize);
    for (var tries = 0;; tries++) {
      check();
      var done = from;
      onProgress(done);
      try {
        final request = await http.openUrl('PUT', at(part));
        (await headers()).forEach(request.headers.set);
        request.headers.contentType = ContentType.binary;
        request.contentLength = to - from;
        final Stream<List<int>> read = file.openRead(from, to).map((chunk) {
          done += chunk.length;
          // Ten times a second is as often as a bar needs to move.
          final now = DateTime.now();
          if (now.difference(told).inMilliseconds >= 100) {
            told = now;
            onProgress(done);
          }
          return chunk;
        });
        await request.addStream(limit == null ? read : paced(read, limit));
        final response = await request.close();
        final answer = await utf8.decodeStream(response);
        final Object? j = answer.isEmpty ? null : jsonDecode(answer);
        if (response.statusCode ~/ 100 != 2) {
          final code = j is Map && j['error'] is String ? j['error'] : null;
          throw CloudError(
              code is String ? code : 'unknown', response.statusCode);
        }
        final etag = j is Map && j['etag'] is String ? j['etag'] as String : '';
        parts.add({'part': part, 'etag': etag});
        break;
      } catch (e) {
        check();
        // A refusal is an answer. Anything else is the line, and worth
        // another go: a dropped connection, a busy server, a stale token.
        final refused = e is CloudError &&
            e.status >= 400 &&
            e.status < 500 &&
            !const [401, 408, 429].contains(e.status);
        if (refused) rethrow;
        if (tries >= 3) {
          throw e is CloudError ? e : const CloudError('offline');
        }
        await Future<void>.delayed(Duration(seconds: 1 << tries));
      }
    }
  }
  onProgress(size);
  return parts;
}
