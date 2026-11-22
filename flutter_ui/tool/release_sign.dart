// Signs a release manifest for scripts/sign-release.ps1. The 32-byte Ed25519
// seed comes in as base64 on the first line of stdin, never as an argument.
//
//   dart run tool/release_sign.dart public
//   dart run tool/release_sign.dart sign <manifest>

import 'dart:convert';
import 'dart:io';

import 'package:cryptography/cryptography.dart';
import 'package:lumit_flutter/state/release_signature.dart';

Future<void> main(List<String> args) async {
  try {
    // Only the base64 characters, since Windows PowerShell may add a byte
    // order mark to what it pipes in.
    final line = stdin.readLineSync() ?? '';
    final seed = base64.decode(line.replaceAll(RegExp('[^A-Za-z0-9+/=]'), ''));
    if (seed.length != 32) throw const FormatException('not a 32-byte key');
    final pair = await Ed25519().newKeyPairFromSeed(seed);
    final public = base64.encode((await pair.extractPublicKey()).bytes);
    if (args.length == 1 && args[0] == 'public') {
      stdout.writeln(public);
    } else if (args.length == 2 && args[0] == 'sign') {
      final manifest = File(args[1]).readAsBytesSync();
      final signed = await Ed25519().sign(manifest, keyPair: pair);
      final signature = base64.encode(signed.bytes);
      final (_, trust) =
          await verifyManifestWithKey(manifest, signature, public);
      if (trust != ReleaseTrust.trusted) {
        throw FormatException('the signature does not check: ${trust.name}');
      }
      stdout.writeln(signature);
    } else {
      throw const FormatException('use: public, or sign <manifest>');
    }
  } catch (e) {
    stderr.writeln('release_sign: $e');
    exitCode = 1;
  }
}
