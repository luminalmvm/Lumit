// Fetching something over the network, and checking that what arrived is what
// was published.
//
// # In plain terms
//
// Two things in Lumit come down from the internet, and both only ever on a
// click: a newer Lumit (`updates.dart`) and an addon (`addons.dart`). They want
// the same three steps, so the steps live here rather than twice over. Ask a
// URL for some text. Stream a large file to disk, saying how far it has got and
// stopping the moment it is asked to. Then check the file is the length the
// publisher said and carries the digest they published, before anything is done
// with it.
//
// Every one of these is handed to a service rather than reached for, so a test
// drives the whole sequence with no network and no real file of any size.

import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart' as crypto;
import 'package:lumit_flutter/l10n/strings.dart';

/// Asking a URL for text. Injected so a test can answer without a network.
typedef TextFetcher = Future<String> Function(Uri url);

/// Fetching a file. [onProgress] is called with bytes received and the total
/// expected; [cancelled] is asked as it goes and a true answer abandons the
/// download. Injected for the same reason.
typedef AssetDownloader = Future<void> Function(
  Uri url,
  File into, {
  required void Function(int received, int total) onProgress,
  required bool Function() cancelled,
});

/// How long to wait for the other end to say anything at all.
const Duration _connectTimeout = Duration(seconds: 10);

/// A file that is not the length it was published as.
const String verifyIncomplete = 'incomplete';

/// A file whose digest is not the one that was published.
const String verifyChecksum = 'checksum';

/// Read [url] as text, asking for [accept] where the server offers more than
/// one shape of answer.
///
/// Nothing is authenticated: everything Lumit reads this way is public, so
/// asking anonymously means Lumit never holds a token.
Future<String> fetchText(Uri url, {String? accept}) async {
  final client = HttpClient()..connectionTimeout = _connectTimeout;
  try {
    final request = await client.getUrl(url);
    request.headers.set(HttpHeaders.userAgentHeader, 'Lumit');
    if (accept != null) request.headers.set(HttpHeaders.acceptHeader, accept);
    final response = await request.close();
    if (response.statusCode != 200) {
      // Drained rather than dropped: an undrained response holds the socket.
      await response.drain<void>();
      throw HttpException(l10n.updateServerAnswered('${response.statusCode}'),
          uri: url);
    }
    return await response.transform(utf8.decoder).join();
  } finally {
    client.close(force: true);
  }
}

/// Stream a file to disk. Redirects are followed, which is how a release asset
/// reaches its CDN; the file is written as it arrives rather than held in
/// memory, because these are hundreds of megabytes.
Future<void> downloadAsset(
  Uri url,
  File into, {
  required void Function(int received, int total) onProgress,
  required bool Function() cancelled,
}) async {
  final client = HttpClient()..connectionTimeout = _connectTimeout;
  IOSink? sink;
  try {
    final request = await client.getUrl(url);
    request.headers.set(HttpHeaders.userAgentHeader, 'Lumit');
    final response = await request.close();
    if (response.statusCode != 200) {
      await response.drain<void>();
      throw HttpException(l10n.updateDownloadAnswered('${response.statusCode}'),
          uri: url);
    }
    final total = response.contentLength;
    var received = 0;
    sink = into.openWrite();
    await for (final chunk in response) {
      if (cancelled()) break;
      sink.add(chunk);
      received += chunk.length;
      onProgress(received, total);
    }
  } finally {
    await sink?.flush();
    await sink?.close();
    client.close(force: true);
  }
}

/// Check [file] against what it was published as: [size] bytes, and [sha256] as
/// a hex digest with or without a `sha256:` in front of it.
///
/// Returns null when the file is sound, or [verifyIncomplete] or
/// [verifyChecksum] for the caller to turn into its own sentence. This is the
/// gate that stands in front of every downloaded file: an installer is the most
/// dangerous file Lumit ever touches, and a model is the largest, so neither is
/// used until its length and, where one was published, its digest are exactly
/// what was promised. A size of zero or a null digest means nothing was
/// published to check against, and that half of the check is skipped.
///
/// The length is read synchronously on purpose: it keeps the whole sequence
/// free of real asynchronous IO except where a digest genuinely needs
/// streaming, which is what lets a widget test drive it from end to end
/// (`flutter_test` does not run real IO outside `runAsync`).
Future<String?> verifySha256(
  File file, {
  required String? sha256,
  required int size,
}) async {
  final length = file.lengthSync();
  if (size > 0 && length != size) return verifyIncomplete;
  if (sha256 == null) return null;
  final wanted =
      sha256.startsWith('sha256:') ? sha256.substring('sha256:'.length) : sha256;
  final digest = await crypto.sha256.bind(file.openRead()).first;
  if (digest.toString().toLowerCase() != wanted.toLowerCase()) {
    return verifyChecksum;
  }
  return null;
}
