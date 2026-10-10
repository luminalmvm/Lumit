// The command line can name a .lum to open (the installer's file association
// passes the document path as an argument). projectPathFromArgs picks it out:
// the first existing .lum on the line, nothing else.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart' show projectPathFromArgs;

void main() {
  late Directory tmp;
  late String real;

  setUp(() {
    tmp = Directory.systemTemp.createTempSync('lumit_args');
    real = '${tmp.path}${Platform.pathSeparator}shot.lum';
    File(real).writeAsStringSync('');
  });

  tearDown(() => tmp.deleteSync(recursive: true));

  test('flags and stray tokens around it are ignored', () {
    expect(projectPathFromArgs(['--verbose', real, 'other']), real);
    // A Linux desktop hands the same document over as a file: address.
    expect(projectPathFromArgs([Uri.file(real).toString()]), real);
  });
}
