// What win32.dart answers, for the browser build, which has no `dart:ffi`
// and no Windows to ask.

String abiName() => 'web_wasm32';

bool altPhysicallyDown() => false;

bool lumitInFront() => true;

bool winWrite(String name, String secret) => false;

String? winRead(String name) => null;

void winDelete(String name) {}
