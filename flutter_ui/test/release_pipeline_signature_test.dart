// The release pipeline's signature, checked by the application's verifier.
//
// The two halves of this mechanism are written in different languages and run
// on different machines: `openssl pkeyutl -sign -rawin` on a release runner,
// and `package:cryptography`'s Ed25519 in the shipped application. Nothing in
// the type system connects them, and a disagreement about signature format or
// about which bytes are signed would only be discovered by a release that
// nobody could install.
//
// So the fixture below is a real manifest and a real signature, produced by the
// exact shell in .github/workflows/release.yml, checked here by the exact code
// the updater runs. Regenerating it is the two commands in
// docs/impl/release-signing.md; the values are otherwise meaningless and deliberately
// so — what is being tested is the agreement, not the contents.

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/release_signature.dart';

void main() {
  const manifest =
      r'''{"version":"0.3.4","assets":[{"name":"Lumit-0.3.4-setup.exe","size":16,"sha256":"8ebefffc1c2d7bc25ac5b5031fb4b7058ad7bbbbdeadd4dafb1e01491683713a"},{"name":"Lumit-0.3.4.dmg","size":10,"sha256":"353e5a66f4bd45ca35346ba8164c93ac594c2caf7f73e47aa9584110249f430c"}]}''';
  const signature = 'iOwSzbZPiQ0NSyXWfUjOEjQr18RIQrFoxzwSAdIw0Ii+Jb6FgKV5i1JhDAu2r26gkLnS5q4B8ldhHpRuBJjcDg==';
  const publicKey = 'zJgZfE9UoydEqGpi8KAyrEoe0Mo4po+9z6IzAioXpl4=';

  test('the release pipeline signs what the application verifies', () async {
    final (parsed, trust) = await verifyManifestWithKey(
      utf8.encode(manifest),
      signature,
      publicKey,
    );
    expect(trust, ReleaseTrust.trusted,
        reason: 'openssl and package:cryptography disagree about Ed25519');
    expect(parsed, isNotNull);
    expect(parsed!.version, '0.3.4');

    final asset = parsed.assetNamed('Lumit-0.3.4-setup.exe');
    expect(asset, isNotNull, reason: 'the manifest shell lost an asset');
    expect(asset!.size, 16);
    expect(asset.sha256,
        '8ebefffc1c2d7bc25ac5b5031fb4b7058ad7bbbbdeadd4dafb1e01491683713a');

    // Both files the shell walked, not just the first.
    expect(parsed.assets.length, 2);
  });

  test('one byte changed anywhere in that manifest breaks the signature',
      () async {
    final tampered = manifest.replaceFirst('"size":16', '"size":17');
    expect(tampered, isNot(manifest));
    final (_, trust) = await verifyManifestWithKey(
        utf8.encode(tampered), signature, publicKey);
    expect(trust, ReleaseTrust.badSignature);
  });
}
