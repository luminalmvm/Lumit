// The title bar reads 'Lumit' until the project has a home on disk, then
// 'Lumit - <file name>' without the extension (windowTitleFor in main.dart).

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart' show windowTitleFor;

void main() {
  test('a Windows path shows the file name without .lum', () {
    expect(windowTitleFor(r'C:\work\Shot 01.lum'), 'Lumit - Shot 01');
  });
}
