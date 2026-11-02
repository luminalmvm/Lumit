// The few things Lumit asks of the operating system directly, through
// `dart:ffi`. The browser build has no `dart:ffi`, so it imports
// win32_web.dart in this file's place, which answers the same questions.

import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

/// This machine's operating system and processor, as `windows_x64`.
String abiName() => Abi.current().toString();

/// A tab is the browser build's; see win32_web.dart.
bool openInTab(String url) => false;

// --- Alt, from the keyboard itself ----------------------------------------

const int _vkMenu = 0x12;

typedef _GetAsyncKeyStateC = Int16 Function(Int32);
typedef _GetAsyncKeyStateD = int Function(int);

_GetAsyncKeyStateD? _getAsyncKeyState;

/// Whether Alt is physically down right now, on Windows.
bool altPhysicallyDown() {
  _getAsyncKeyState ??= DynamicLibrary.open('user32.dll')
      .lookupFunction<_GetAsyncKeyStateC, _GetAsyncKeyStateD>(
          'GetAsyncKeyState');
  return (_getAsyncKeyState!(_vkMenu) & 0x8000) != 0;
}

// --- The window in front ---------------------------------------------------

typedef _ForegroundC = IntPtr Function();
typedef _ForegroundD = int Function();
typedef _OwnerC = Uint32 Function(IntPtr window, Pointer<Uint32> process);
typedef _OwnerD = int Function(int window, Pointer<Uint32> process);

final DynamicLibrary _user32 = DynamicLibrary.open('user32.dll');
final _foreground =
    _user32.lookupFunction<_ForegroundC, _ForegroundD>('GetForegroundWindow');
final _owner =
    _user32.lookupFunction<_OwnerC, _OwnerD>('GetWindowThreadProcessId');

/// Whether the window in front is one of Lumit's, on Windows. A page is
/// inside Lumit's window, so a press in one leaves it in front.
bool lumitInFront() {
  final window = _foreground();
  if (window == 0) return false;
  final process = calloc<Uint32>();
  try {
    _owner(window, process);
    return process.value == pid;
  } finally {
    calloc.free(process);
  }
}

// --- Credential Manager --------------------------------------------------

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

final int Function(Pointer<_Credential>, int) _credWrite =
    _advapi.lookupFunction<Int32 Function(Pointer<_Credential>, Uint32),
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

String _target(String name) => 'Lumit/$name';

bool winWrite(String name, String secret) {
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

String? winRead(String name) {
  final target = _target(name).toNativeUtf16();
  final found = calloc<Pointer<_Credential>>();
  try {
    if (_credRead(target, _generic, 0, found) == 0) return null;
    final credential = found.value.ref;
    final secret = utf8.decode(credential.blob.asTypedList(credential.blobSize),
        allowMalformed: true);
    _credFree(found.value.cast());
    return secret;
  } finally {
    calloc
      ..free(target)
      ..free(found);
  }
}

void winDelete(String name) {
  final target = _target(name).toNativeUtf16();
  try {
    _credDelete(target, _generic, 0);
  } finally {
    calloc.free(target);
  }
}
