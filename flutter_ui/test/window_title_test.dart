// The title bar reads 'Lumit' until the project has a home on disk, then
// 'Lumit - <file name>' without the extension (windowTitleFor in main.dart).

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart' show windowTitleFor;

void main() {
  test('no path is plain Lumit', () {
    expect(windowTitleFor(null), 'Lumit');
    expect(windowTitleFor(''), 'Lumit');
  });

  test('a Windows path shows the file name without .lum', () {
    expect(windowTitleFor(r'C:\work\Shot 01.lum'), 'Lumit - Shot 01');
  });

  test('a POSIX path shows the file name without .lum', () {
    expect(windowTitleFor('/home/me/edits/promo.lum'), 'Lumit - promo');
  });

  test('the extension strips case-insensitively', () {
    expect(windowTitleFor(r'C:\work\FINAL.LUM'), 'Lumit - FINAL');
  });

  test('a dot in the name only loses the extension', () {
    expect(windowTitleFor('/x/v2.final.lum'), 'Lumit - v2.final');
  });
}
