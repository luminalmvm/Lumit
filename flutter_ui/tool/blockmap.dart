
import 'dart:io';

import 'package:lumit_flutter/state/blockmap.dart';

void main(List<String> files) {
  for (final path in files) {
    File(blockmapNameFor(path))
        .writeAsStringSync(encodeBlockmap(piecesOf(File(path))));
  }
}
