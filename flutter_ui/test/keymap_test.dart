// Turning a real keypress into chord text — the one half of the keymap that is
// the frontend's (docs/07-UI-SPEC.md §15).
//
// Pure Dart, no engine: what is under test here is whether Flutter's idea of a
// key and the keymap's idea of a key agree. They have to agree exactly, because
// the engine matches chords as strings — a `Space` that arrives as `space` is
// simply an unbound key, and the shortcut silently does nothing.

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/keymap.dart';

void main() {
  group('key names match what the keymap stores', () {
    test('the named keys use the keymap words, not Flutter debug labels', () {
      expect(keyName(LogicalKeyboardKey.space), 'Space');
      expect(keyName(LogicalKeyboardKey.pageUp), 'PageUp');
      expect(keyName(LogicalKeyboardKey.pageDown), 'PageDown');
      expect(keyName(LogicalKeyboardKey.arrowLeft), 'ArrowLeft');
      expect(keyName(LogicalKeyboardKey.arrowRight), 'ArrowRight');
      expect(keyName(LogicalKeyboardKey.home), 'Home');
      expect(keyName(LogicalKeyboardKey.end), 'End');
      expect(keyName(LogicalKeyboardKey.delete), 'Delete');
      expect(keyName(LogicalKeyboardKey.backspace), 'Backspace');
    });
  });

  group('chordText spells the modifiers in the engine order', () {
    /// `chordText` reads the *held* keys, so a test has to hold them rather
    /// than describe them — which is also how the real handler sees a chord.
    Future<String?> chordFor(
      WidgetTester tester,
      LogicalKeyboardKey key, {
      List<LogicalKeyboardKey> holding = const [],
    }) async {
      String? seen;
      for (final mod in holding) {
        await tester.sendKeyDownEvent(mod);
      }
      bool handler(KeyEvent event) {
        if (event is KeyDownEvent && event.logicalKey == key) {
          seen = chordText(event);
        }
        return false;
      }
      HardwareKeyboard.instance.addHandler(handler);
      await tester.sendKeyDownEvent(key);
      await tester.sendKeyUpEvent(key);
      HardwareKeyboard.instance.removeHandler(handler);
      for (final mod in holding.reversed) {
        await tester.sendKeyUpEvent(mod);
      }
      return seen;
    }

    /// The engine writes `Mod+Alt+Shift+Key` and parses in any order, but only
    /// one spelling round-trips through its own Display — so this is the one
    /// the frontend must produce, or a chord captured in Settings would not
    /// match the same keys pressed in anger.
    testWidgets('modifiers come out in Mod, Alt, Shift order', (tester) async {
      expect(
        await chordFor(tester, LogicalKeyboardKey.keyD,
            holding: [LogicalKeyboardKey.shiftLeft]),
        'Shift+D',
      );
      expect(
        await chordFor(tester, LogicalKeyboardKey.keyT, holding: [
          LogicalKeyboardKey.altLeft,
          LogicalKeyboardKey.shiftLeft,
        ]),
        'Alt+Shift+T',
      );
      expect(
        await chordFor(tester, LogicalKeyboardKey.keyT, holding: [
          LogicalKeyboardKey.controlLeft,
          LogicalKeyboardKey.altLeft,
        ]),
        'Mod+Alt+T',
        reason: 'Ctrl is the primary modifier off macOS',
      );
    });

    testWidgets('a modifier pressed alone yields no chord', (tester) async {
      expect(await chordFor(tester, LogicalKeyboardKey.shiftLeft), isNull);
    });
  });

  group('chords as macOS menu activators', () {
    test('the modifiers and the key come back out again', () {
      final a = activatorForChord('Mod+Shift+Z')!;
      expect(a.trigger, LogicalKeyboardKey.keyZ);
      expect(a.meta, isTrue, reason: 'Mod is Cmd on the only platform asking');
      expect(a.shift, isTrue);
      expect(a.alt, isFalse);

      final b = activatorForChord('Mod+Alt+;')!;
      expect(b.trigger, LogicalKeyboardKey.semicolon);
      expect(b.alt, isTrue);
    });
  });
}
