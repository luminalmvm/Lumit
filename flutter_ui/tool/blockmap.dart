// Writes the blockmap the updater patches from beside each file it is given.
// The release pipeline runs it on the Windows installer and the macOS zip.
//
//   dart run tool/blockmap.dart <file>...

import 'dart:io';

import 'package:lumit_flutter/state/blockmap.dart';

void main(List<String> files) {
  for (final path in files) {
    File(blockmapNameFor(path))
        .writeAsStringSync(encodeBlockmap(piecesOf(File(path))));
  }
}
