// The direction a name the user gave reads in. The chrome is left to right, so
// an Arabic or Hebrew layer name cut short put its ellipsis beside its first
// word and lost its end without a mark. Read from the name's first letter that
// has a direction, so "Shot 010 – …" stays left to right whatever follows.

import 'package:flutter/widgets.dart';

/// Right to left when the name starts in a right-to-left script, otherwise
/// null so the text takes the chrome's own direction.
TextDirection? nameDirection(String name) {
  for (final c in name.runes) {
    final rtl = (c >= 0x0590 && c <= 0x08FF) ||
        (c >= 0xFB1D && c <= 0xFDFF) ||
        (c >= 0xFE70 && c <= 0xFEFF);
    if (rtl) return TextDirection.rtl;
    final ltr = (c >= 0x41 && c <= 0x5A) ||
        (c >= 0x61 && c <= 0x7A) ||
        (c >= 0xC0 && c < 0x0590) ||
        c >= 0x0900;
    if (ltr) return null;
  }
  return null;
}
