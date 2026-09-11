// What a signed release manifest has to hold up to.
//
// The updater's strongest claim is that a release was made by whoever holds the
// signing key, and a claim like that is only worth what its refusals are worth.
// So most of this file is about the ways a manifest can be wrong: signed by
// somebody else, signed and then edited, well-formed and about a different
// file, or simply absent.
//
// The key pair is generated in the test rather than checked in, and the pinned
// constant the shipped code reads cannot be reassigned, so the verification is
// driven through `verifyManifestWithKey` — the same code path with the key
// passed in, which is what `verifyManifest` calls once it has read the pinned
// one.

import 'dart:convert';

import 'package:cryptography/cryptography.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/release_signature.dart';

/// A plausible digest, for a fixture that does not care what it is.
final String aDigest = 'a' * 64;

/// A manifest for one file, as the release pipeline writes it.
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

  test('a manifest this key signed is believed, and names its files', () async {
    final bytes = manifestFor();
    final (manifest, trust) = await verifyManifestWithKey(
      bytes,
      await sign(bytes),
      publicKeyBase64,
    );
    expect(trust, ReleaseTrust.trusted);
    expect(manifest, isNotNull);
    expect(manifest!.version, '0.3.4');
    final asset = manifest.assetNamed('Lumit-0.3.4-setup.exe');
    expect(asset, isNotNull);
    expect(asset!.size, 4096);
    expect(asset.sha256, aDigest);
    // A file the manifest says nothing about is not found, rather than
    // defaulted to something.
    expect(manifest.assetNamed('something-else.exe'), isNull);
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

  test('a manifest edited after it was signed is refused', () async {
    final original = manifestFor(sha256: aDigest);
    final signature = await sign(original);

    // The shape the whole mechanism exists for: everything about the release is
    // the same except the digest of the file it points at.
    final tampered = manifestFor(sha256: 'b' * 64);
    final (manifest, trust) =
        await verifyManifestWithKey(tampered, signature, publicKeyBase64);
    expect(trust, ReleaseTrust.badSignature);
    expect(manifest, isNull);
  });

  test('a signature for another release is refused', () async {
    // What a replay looks like: a real signature, over a real manifest, for the
    // version before this one.
    final older = manifestFor(version: '0.3.3');
    final signature = await sign(older);
    final newer = manifestFor(version: '0.3.4');
    final (_, trust) =
        await verifyManifestWithKey(newer, signature, publicKeyBase64);
    expect(trust, ReleaseTrust.badSignature);
  });

  test('a signature that is not a signature is refused, never thrown', () async {
    final bytes = manifestFor();
    for (final rubbish in <String>[
      '',
      'not base64 at all !!',
      base64.encode(List<int>.filled(63, 0)), // one byte short
      base64.encode(List<int>.filled(65, 0)), // one byte over
      base64.encode(List<int>.filled(64, 0)), // right length, wrong value
    ]) {
      final (_, trust) =
          await verifyManifestWithKey(bytes, rubbish, publicKeyBase64);
      expect(trust, ReleaseTrust.badSignature, reason: 'for "$rubbish"');
    }
  });

  test('a pinned key that is not a key refuses everything', () async {
    final bytes = manifestFor();
    final signature = await sign(bytes);
    for (final bad in <String>[
      'not base64 !!',
      base64.encode(List<int>.filled(31, 0)),
      base64.encode(List<int>.filled(33, 0)),
    ]) {
      final (_, trust) = await verifyManifestWithKey(bytes, signature, bad);
      expect(trust, ReleaseTrust.badSignature, reason: 'for "$bad"');
    }
    // And an empty key is "no key pinned", which is a different answer: this
    // build simply cannot make the strong check.
    final (_, unpinned) = await verifyManifestWithKey(bytes, signature, '');
    expect(unpinned, ReleaseTrust.unsigned);
  });

  test('a signed manifest that is not a manifest is refused', () async {
    // Signed by the right key — so this is the case where the *publisher* got
    // it wrong, not an attacker. Still refused: a shape that is nearly right is
    // exactly the shape that gets believed by accident.
    for (final body in <Object>[
      'not json at all',
      jsonEncode({'assets': []}), // no version
      jsonEncode({'version': '', 'assets': []}), // empty version
      jsonEncode({'version': '1', 'assets': []}), // nothing in it
      jsonEncode({'version': '1'}), // no assets at all
      jsonEncode({
        'version': '1',
        'assets': [
          {'name': 'a', 'size': 1, 'sha256': 'short'}
        ]
      }),
      jsonEncode({
        'version': '1',
        'assets': [
          {'name': 'a', 'size': 1, 'sha256': 'z' * 64} // not hex
        ]
      }),
      jsonEncode({
        'version': '1',
        'assets': [
          {'name': 'a', 'size': 0, 'sha256': 'a' * 64} // nothing weighs nothing
        ]
      }),
      jsonEncode({
        'version': '1',
        'assets': [
          {'name': '', 'size': 1, 'sha256': 'a' * 64} // nameless
        ]
      }),
      jsonEncode({
        'version': '1',
        'assets': [
          {'name': 'a', 'size': '1', 'sha256': 'a' * 64} // a size as text
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

  test('a digest is read case-insensitively but kept lower case', () async {
    final bytes = manifestFor(sha256: 'A' * 64);
    final (manifest, trust) = await verifyManifestWithKey(
        bytes, await sign(bytes), publicKeyBase64);
    expect(trust, ReleaseTrust.trusted);
    expect(manifest!.assetNamed('Lumit-0.3.4-setup.exe')!.sha256, 'a' * 64);
  });

  test('the shipped build pins no key yet, and says so rather than pretending',
      () {
    // A guard, not a preference: if this ever fails because somebody pinned a
    // key, the right change is to delete this test — and to notice, at that
    // moment, that unsigned releases stop installing from then on.
    expect(updateSigningKey, isEmpty);
    expect(releaseSigningIsEnforced, isFalse);
  });
}
