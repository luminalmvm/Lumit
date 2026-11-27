// What a signed release manifest has to hold up to.
//
// The updater's strongest claim is that a release was made by whoever holds the
// signing key, and a claim like that is only worth what its refusals are worth.
// So most of this file is about the ways a manifest can be wrong: signed by
// somebody else, signed and then edited, well-formed and about a different
// file, or simply absent.
//
// The key pair is generated in the test rather than checked in, and passed to
// `verifyManifestWithKey`, the same call the updater makes with the pinned
// key.

import 'dart:convert';

import 'package:cryptography/cryptography.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/release_signature.dart';

final String aDigest = 'a' * 64;

List<int> manifestFor({
  String version = '0.3.4',
  String name = 'Lumit-0.3.4-setup.exe',
  int size = 4096,
  String? sha256,
}) =>
    utf8.encode(jsonEncode({
      'version': version,
      'assets': [
        {'name': name, 'size': size, 'sha256': sha256 ?? aDigest},
      ],
    }));

void main() {
  late SimpleKeyPair pair;
  late String publicKeyBase64;

  setUp(() async {
    pair = await Ed25519().newKeyPair();
    final public = await pair.extractPublicKey();
    publicKeyBase64 = base64.encode(public.bytes);
  });

  Future<String> sign(List<int> bytes) async {
    final signature = await Ed25519().sign(bytes, keyPair: pair);
    return base64.encode(signature.bytes);
  }

  test('a manifest edited after it was signed is refused', () async {
    final original = manifestFor(sha256: aDigest);
    final signature = await sign(original);

    final tampered = manifestFor(sha256: 'b' * 64);
    final (manifest, trust) =
        await verifyManifestWithKey(tampered, signature, publicKeyBase64);
    expect(trust, ReleaseTrust.badSignature);
    expect(manifest, isNull);
  });

  test('a signature that is not a signature is refused, never thrown', () async {
    final bytes = manifestFor();
    for (final rubbish in <String>[
      '',
      'not base64 at all !!',
      base64.encode(List<int>.filled(63, 0)),
      base64.encode(List<int>.filled(65, 0)),
      base64.encode(List<int>.filled(64, 0)),
    ]) {
      final (_, trust) =
          await verifyManifestWithKey(bytes, rubbish, publicKeyBase64);
      expect(trust, ReleaseTrust.badSignature, reason: 'for "$rubbish"');
    }
  });

  test('a signed manifest that is not a manifest is refused', () async {
    // Signed by the right key — so this is the case where the *publisher* got
    // it wrong, not an attacker. Still refused: a shape that is nearly right is
    // exactly the shape that gets believed by accident.
    for (final body in <Object>[
      'not json at all',
      jsonEncode({'assets': []}),
      jsonEncode({'version': '', 'assets': []}),
      jsonEncode({'version': '1', 'assets': []}),
      jsonEncode({'version': '1'}),
      jsonEncode({
        'version': '1',
        'assets': [
          {'name': 'a', 'size': 1, 'sha256': 'short'}
        ]
      }),
      jsonEncode({
        'version': '1',
        'assets': [
          {'name': 'a', 'size': 1, 'sha256': 'z' * 64}
        ]
      }),
      jsonEncode({
        'version': '1',
        'assets': [
          {'name': 'a', 'size': 0, 'sha256': 'a' * 64}
        ]
      }),
      jsonEncode({
        'version': '1',
        'assets': [
          {'name': '', 'size': 1, 'sha256': 'a' * 64}
        ]
      }),
      jsonEncode({
        'version': '1',
        'assets': [
          {'name': 'a', 'size': '1', 'sha256': 'a' * 64}
        ]
      }),
    ]) {
      final bytes = utf8.encode(body as String);
      final (manifest, trust) =
          await verifyManifestWithKey(bytes, await sign(bytes), publicKeyBase64);
      expect(trust, ReleaseTrust.malformed, reason: 'for $body');
      expect(manifest, isNull);
    }
  });

  test('a manifest larger than the cap is refused without being parsed',
      () async {
    final huge = utf8.encode(jsonEncode({
      'version': '1',
      'assets': [
        {'name': 'x' * (releaseManifestMaxBytes + 1), 'size': 1, 'sha256': 'a' * 64}
      ]
    }));
    expect(huge.length, greaterThan(releaseManifestMaxBytes));
    final (_, trust) =
        await verifyManifestWithKey(huge, await sign(huge), publicKeyBase64);
    expect(trust, ReleaseTrust.malformed);
  });

  test('a manifest somebody else signed is refused', () async {
    final bytes = manifestFor();
    final impostor = await Ed25519().newKeyPair();
    final signature = await Ed25519().sign(bytes, keyPair: impostor);

    final (manifest, trust) = await verifyManifestWithKey(
      bytes,
      base64.encode(signature.bytes),
      publicKeyBase64,
    );
    expect(trust, ReleaseTrust.badSignature);
    expect(manifest, isNull);
  });

  // A pin that is not a key refuses every update, and only users find out.
  test('every pinned key is a 32-byte public key', () {
    expect(updateSigningKeys, isNotEmpty);
    for (final key in updateSigningKeys) {
      expect(base64.decode(key), hasLength(32), reason: key);
    }
  });
}
