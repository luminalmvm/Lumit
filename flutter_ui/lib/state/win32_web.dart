
import 'package:web/web.dart' as web;

String abiName() => 'web_wasm32';

bool altPhysicallyDown() => false;

bool lumitInFront() => true;

bool winWrite(String name, String secret) => false;

String? winRead(String name) => null;

void winDelete(String name) {}

bool openInTab(String url) {
  web.window.open(url, '_blank', 'noopener');
  return true;
}
