// Keeping a secret where the operating system keeps its own.
//
// A sign-in is kept by one long random token. Whoever holds it is that person
// until they sign out, so it does not go in the settings file, which is plain
// text that gets copied, backed up and pasted into bug reports. It goes where
// the system keeps passwords: Credential Manager on Windows, the keychain on
// macOS, and the keyring that `secret-tool` speaks to on Linux.
//
// A Linux machine without `secret-tool` falls back to a file only its owner
// can read. That is weaker, and it is what the machine offers.

import 'dart:io' hide Platform;

import 'package:lumit_flutter/state/win32.dart'
    if (dart.library.js_interop) 'package:lumit_flutter/state/win32_web.dart';
import 'package:lumit_flutter/state/platform.dart'
    if (dart.library.js_interop) 'package:lumit_flutter/state/platform_web.dart';

/// What every entry is filed under, so they can be told from anything else
/// the system holds.
const String _service = 'Lumit';

/// Somewhere other than the system's own store to keep secrets, set by tests
/// so that a test run never writes into the developer's real one.
Map<String, String>? secretStoreOverride;

/// Keep [secret] under [name], replacing what was there. False when the
/// system would not take it.
Future<bool> writeSecret(String name, String secret) async {
  final fake = secretStoreOverride;
  if (fake != null) {
    fake[name] = secret;
    return true;
  }
  try {
    if (Platform.isWindows) return winWrite(name, secret);
    if (Platform.isMacOS) return await _macWrite(name, secret);
    return await _linuxWrite(name, secret);
  } catch (_) {
    return false;
  }
}

/// The secret kept under [name], or null when there is none.
Future<String?> readSecret(String name) async {
  final fake = secretStoreOverride;
  if (fake != null) return fake[name];
  try {
    if (Platform.isWindows) return winRead(name);
    if (Platform.isMacOS) return await _macRead(name);
    return await _linuxRead(name);
  } catch (_) {
    return null;
  }
}

/// Forget the secret kept under [name]. Nothing there is not an error.
Future<void> deleteSecret(String name) async {
  final fake = secretStoreOverride;
  if (fake != null) {
    fake.remove(name);
    return;
  }
  try {
    if (Platform.isWindows) {
      winDelete(name);
    } else if (Platform.isMacOS) {
      await Process.run('security',
          ['delete-generic-password', '-s', _service, '-a', name]);
    } else {
      await _linuxDelete(name);
    }
  } catch (_) {
    // Nothing to forget.
  }
}

// --- macOS: the keychain ---------------------------------------------------

/// Written through `security -i`, which reads its commands from standard
/// input. As an argument the secret would be in every process listing on the
/// machine for as long as the command ran.
Future<bool> _macWrite(String name, String secret) async {
  final security = await Process.start('security', ['-i']);
  security.stdin.writeln('add-generic-password -U -s ${_quoted(_service)} '
      '-a ${_quoted(name)} -w ${_quoted(secret)}');
  await security.stdin.close();
  await security.stdout.drain<void>();
  return await security.exitCode == 0;
}

Future<String?> _macRead(String name) async {
  final found = await Process.run('security',
      ['find-generic-password', '-s', _service, '-a', name, '-w']);
  if (found.exitCode != 0) return null;
  final secret = (found.stdout as String).trim();
  return secret.isEmpty ? null : secret;
}

String _quoted(String text) =>
    '"${text.replaceAll(r'\', r'\\').replaceAll('"', r'\"')}"';

// --- Linux: the keyring, or a file -----------------------------------------

Future<bool> _linuxWrite(String name, String secret) async {
  try {
    // The secret goes down standard input, for the reason macOS gives.
    final tool = await Process.start('secret-tool',
        ['store', '--label=$_service', 'service', _service, 'account', name]);
    tool.stdin.write(secret);
    await tool.stdin.close();
    if (await tool.exitCode == 0) return true;
  } on ProcessException {
    // No secret-tool on this machine.
  }
  final file = _linuxFile(name);
  file.parent.createSync(recursive: true);
  file.writeAsStringSync(secret, flush: true);
  // Only its owner reads it. Nothing else here stands in for a keyring.
  await Process.run('chmod', ['600', file.path]);
  return true;
}

Future<String?> _linuxRead(String name) async {
  try {
    final found = await Process.run(
        'secret-tool', ['lookup', 'service', _service, 'account', name]);
    final secret = (found.stdout as String).trim();
    if (found.exitCode == 0 && secret.isNotEmpty) return secret;
  } on ProcessException {
    // No secret-tool on this machine.
  }
  final file = _linuxFile(name);
  return file.existsSync() ? file.readAsStringSync().trim() : null;
}

Future<void> _linuxDelete(String name) async {
  try {
    await Process.run(
        'secret-tool', ['clear', 'service', _service, 'account', name]);
  } on ProcessException {
    // No secret-tool on this machine.
  }
  final file = _linuxFile(name);
  if (file.existsSync()) file.deleteSync();
}

File _linuxFile(String name) {
  final home = Platform.environment['XDG_CONFIG_HOME'] ??
      '${Platform.environment['HOME'] ?? '.'}/.config';
  return File('$home/lumit/secrets/${Uri.encodeComponent(name)}');
}
