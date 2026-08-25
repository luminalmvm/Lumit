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

  test('an existing .lum on the line is found', () {
    expect(projectPathFromArgs([real]), real);
  });

  test('flags and stray tokens around it are ignored', () {
    expect(projectPathFromArgs(['--verbose', real, 'other']), real);
  });

  test('extension match is case-insensitive', () {
    final upper = '${tmp.path}${Platform.pathSeparator}SHOT.LUM';
    File(upper).writeAsStringSync('');
    expect(projectPathFromArgs([upper]), upper);
  });

  test('a .lum that does not exist is not a project', () {
    expect(projectPathFromArgs(['${tmp.path}/missing.lum']), isNull);
  });

  test('a non-.lum file that exists is not a project', () {
    final other = '${tmp.path}${Platform.pathSeparator}notes.txt';
    File(other).writeAsStringSync('');
    expect(projectPathFromArgs([other]), isNull);
  });

  test('an empty line opens nothing', () {
    expect(projectPathFromArgs([]), isNull);
  });
}
