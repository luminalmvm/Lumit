import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'package:lumit_flutter/state/win32.dart'
    if (dart.library.js_interop) 'package:lumit_flutter/state/win32_web.dart';

const Map<String, String> keyLayouts = {
  'us': r"`1234567890-=QWERTYUIOP[]\ASDFGHJKL;'\ZXCVBNM,./",
  'uk': r"`1234567890-=QWERTYUIOP[]#ASDFGHJKL;'\ZXCVBNM,./",
  'de': r'^1234567890ß´QWERTZUIOPü+#ASDFGHJKLöä<YXCVBNM,.-',
  'fr': r'''²&é"'(-è_çà)=AZERTYUIOP^$*QSDFGHJKLMù<WXCVBN,;:!''',
  'es': r"º1234567890'¡QWERTYUIOP`+çASDFGHJKLñ´<ZXCVBNM,.-",
  'it': r"\1234567890'ìQWERTYUIOPè+ùASDFGHJKLòà<ZXCVBNM,.-",
  'sv': r"§1234567890+´QWERTYUIOPå¨'ASDFGHJKLöä<ZXCVBNM,.-",
  'de-ch': r"§1234567890'^QWERTZUIOPü¨$ASDFGHJKLöä<YXCVBNM,.-",
  'ru': r'ё1234567890-=QWERTYUIOPхъ\ASDFGHJKLжэ\ZXCVBNMбю.',
};

const String _intl = 'IntlBackslash';

final List<String> _usNames = [
  for (var i = 0; i < 48; i++) i == 37 ? _intl : keyLayouts['us']![i],
];

const Set<String> _punctuation = {
  '`', '-', '=', '[', ']', r'\', ';', "'", ',', '.', '/',
};

const List<PhysicalKeyboardKey> _places = [
  PhysicalKeyboardKey.backquote,
  PhysicalKeyboardKey.digit1, PhysicalKeyboardKey.digit2,
  PhysicalKeyboardKey.digit3, PhysicalKeyboardKey.digit4,
  PhysicalKeyboardKey.digit5, PhysicalKeyboardKey.digit6,
  PhysicalKeyboardKey.digit7, PhysicalKeyboardKey.digit8,
  PhysicalKeyboardKey.digit9, PhysicalKeyboardKey.digit0,
  PhysicalKeyboardKey.minus, PhysicalKeyboardKey.equal,
  PhysicalKeyboardKey.keyQ, PhysicalKeyboardKey.keyW, PhysicalKeyboardKey.keyE,
  PhysicalKeyboardKey.keyR, PhysicalKeyboardKey.keyT, PhysicalKeyboardKey.keyY,
  PhysicalKeyboardKey.keyU, PhysicalKeyboardKey.keyI, PhysicalKeyboardKey.keyO,
  PhysicalKeyboardKey.keyP, PhysicalKeyboardKey.bracketLeft,
  PhysicalKeyboardKey.bracketRight, PhysicalKeyboardKey.backslash,
  PhysicalKeyboardKey.keyA, PhysicalKeyboardKey.keyS, PhysicalKeyboardKey.keyD,
  PhysicalKeyboardKey.keyF, PhysicalKeyboardKey.keyG, PhysicalKeyboardKey.keyH,
  PhysicalKeyboardKey.keyJ, PhysicalKeyboardKey.keyK, PhysicalKeyboardKey.keyL,
  PhysicalKeyboardKey.semicolon, PhysicalKeyboardKey.quote,
  PhysicalKeyboardKey.intlBackslash,
  PhysicalKeyboardKey.keyZ, PhysicalKeyboardKey.keyX, PhysicalKeyboardKey.keyC,
  PhysicalKeyboardKey.keyV, PhysicalKeyboardKey.keyB, PhysicalKeyboardKey.keyN,
  PhysicalKeyboardKey.keyM, PhysicalKeyboardKey.comma,
  PhysicalKeyboardKey.period, PhysicalKeyboardKey.slash,
];

final List<int> _scancodes = [
  0x29,
  for (var s = 0x02; s <= 0x0D; s++) s,
  for (var s = 0x10; s <= 0x1B; s++) s,
  0x2B,
  for (var s = 0x1E; s <= 0x28; s++) s,
  0x56,
  for (var s = 0x2C; s <= 0x35; s++) s,
];

final RegExp _letter = RegExp(r'^[A-Za-z]$');
final RegExp _readsAsItself = RegExp(r'^[A-Z0-9]$');

const MethodChannel _channel = MethodChannel('lumit/window');

final keyLayout = KeyLayout();

class KeyLayout extends ChangeNotifier {
  KeyLayout() {
    _adopt(keyLayouts['us']!);
  }

  String? override;

  int? windowsLayout;

  String _chars = '';
  List<String> _names = const [];

  bool get iso => _chars[37] != _chars[25];

  String? nameAt(PhysicalKeyboardKey key) {
    final i = _places.indexOf(key);
    final known = windowsLayout != null || keyLayouts.containsKey(override);
    return i < 0 || !known ? null : _names[i];
  }

  String nameFor(String usName) {
    final i = _usNames.indexOf(usName);
    return i < 0 ? usName : _names[i];
  }

  String? nameOfVirtualKey(int virtualKey) {
    final layout = windowsLayout;
    if (layout == null) return null;
    final i = _scancodes.indexOf(winScancode(virtualKey, layout));
    return i < 0 ? null : _names[i];
  }

  String legend(String name) {
    if (_readsAsItself.hasMatch(name)) return name;
    final i = _names.indexOf(name);
    if (i < 0) return name;
    final upper = _chars[i].toUpperCase();
    return upper.length == 1 ? upper : _chars[i];
  }

  Future<void> refresh() async {
    windowsLayout = await _askWindowsLayout();
    final layout = windowsLayout;
    final chars = keyLayouts[override] ??
        (layout == null ? null : winKeyLayout(layout, _scancodes)) ??
        keyLayouts['us']!;
    if (chars == _chars) return;
    _adopt(chars);
    notifyListeners();
  }

  static Future<int?> _askWindowsLayout() async {
    if (kIsWeb || defaultTargetPlatform != TargetPlatform.windows) return null;
    try {
      return await _channel.invokeMethod<int>('keyboardLayout');
    } catch (_) {
      return null;
    }
  }

  void _adopt(String chars) {
    final names = List<String?>.filled(_usNames.length, null);
    final taken = <String>{};
    void give(int i, String name) {
      names[i] = name;
      taken.add(name);
    }

    for (var i = 0; i < names.length; i++) {
      final upper = chars[i].toUpperCase();
      if (_letter.hasMatch(chars[i]) && !taken.contains(upper)) give(i, upper);
    }
    for (var i = 1; i <= 10; i++) {
      if (names[i] == null && !taken.contains(_usNames[i])) {
        give(i, _usNames[i]);
      }
    }
    for (var i = 0; i < names.length; i++) {
      final c = chars[i];
      if (names[i] == null &&
          !_punctuation.contains(_usNames[i]) &&
          _punctuation.contains(c) &&
          !taken.contains(c) &&
          chars.indexOf(c) == chars.lastIndexOf(c)) {
        give(i, c);
      }
    }
    for (var i = 0; i < names.length; i++) {
      if (names[i] == null && !taken.contains(_usNames[i])) {
        give(i, _usNames[i]);
      }
    }
    for (var i = 0; i < names.length; i++) {
      if (names[i] == null) {
        give(
            i,
            _usNames.firstWhere((n) => n != _intl && !taken.contains(n),
                orElse: () => _intl));
      }
    }
    _chars = chars;
    _names = [for (final name in names) name!];
  }
}
