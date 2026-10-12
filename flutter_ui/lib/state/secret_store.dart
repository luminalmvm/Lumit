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

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

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
    if (Platform.isWindows) return _winWrite(name, secret);
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
    if (Platform.isWindows) return _winRead(name);
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
      _winDelete(name);
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

// --- Windows: Credential Manager ------------------------------------------

/// `CREDENTIALW`, as `wincred.h` lays it out.
final class _Credential extends Struct {
  @Uint32()
  external int flags;
  @Uint32()
  external int type;
  external Pointer<Utf16> targetName;
  external Pointer<Utf16> comment;
  @Uint32()
  external int lastWrittenLow;
  @Uint32()
  external int lastWrittenHigh;
  @Uint32()
  external int blobSize;
  external Pointer<Uint8> blob;
  @Uint32()
  external int persist;
  @Uint32()
  external int attributeCount;
  external Pointer<Void> attributes;
  external Pointer<Utf16> targetAlias;
  external Pointer<Utf16> userName;
}

/// `CRED_TYPE_GENERIC`, and `CRED_PERSIST_LOCAL_MACHINE`: kept for this
/// person on this machine, and not carried to another by a roaming profile.
const int _generic = 1;
const int _thisMachine = 2;

final DynamicLibrary _advapi = DynamicLibrary.open('advapi32.dll');

final int Function(Pointer<_Credential>, int) _credWrite = _advapi.lookupFunction<
    Int32 Function(Pointer<_Credential>, Uint32),
    int Function(Pointer<_Credential>, int)>('CredWriteW');

final int Function(Pointer<Utf16>, int, int, Pointer<Pointer<_Credential>>)
    _credRead = _advapi.lookupFunction<
        Int32 Function(
            Pointer<Utf16>, Uint32, Uint32, Pointer<Pointer<_Credential>>),
        int Function(Pointer<Utf16>, int, int,
            Pointer<Pointer<_Credential>>)>('CredReadW');

final int Function(Pointer<Utf16>, int, int) _credDelete =
    _advapi.lookupFunction<Int32 Function(Pointer<Utf16>, Uint32, Uint32),
        int Function(Pointer<Utf16>, int, int)>('CredDeleteW');

final void Function(Pointer<Void>) _credFree = _advapi.lookupFunction<
    Void Function(Pointer<Void>), void Function(Pointer<Void>)>('CredFree');

String _target(String name) => '$_service/$name';

bool _winWrite(String name, String secret) {
  final bytes = utf8.encode(secret);
  final target = _target(name).toNativeUtf16();
  final user = name.toNativeUtf16();
  final blob = calloc<Uint8>(bytes.length);
  final credential = calloc<_Credential>();
  try {
    blob.asTypedList(bytes.length).setAll(0, bytes);
    credential.ref
      ..type = _generic
      ..targetName = target
      ..blobSize = bytes.length
      ..blob = blob
      ..persist = _thisMachine
      ..userName = user;
    return _credWrite(credential, 0) != 0;
  } finally {
    // Not left lying in freed memory for the next allocation to find.
    blob.asTypedList(bytes.length).fillRange(0, bytes.length, 0);
    calloc
      ..free(blob)
      ..free(credential)
      ..free(target)
      ..free(user);
  }
}

String? _winRead(String name) {
  final target = _target(name).toNativeUtf16();
  final found = calloc<Pointer<_Credential>>();
  try {
    if (_credRead(target, _generic, 0, found) == 0) return null;
    final credential = found.value.ref;
    final secret = utf8.decode(
        credential.blob.asTypedList(credential.blobSize),
        allowMalformed: true);
    _credFree(found.value.cast());
    return secret;
  } finally {
    calloc
      ..free(target)
      ..free(found);
  }
}

void _winDelete(String name) {
  final target = _target(name).toNativeUtf16();
  try {
    _credDelete(target, _generic, 0);
  } finally {
    calloc.free(target);
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
