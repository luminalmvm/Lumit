// Turning a real keypress into chord text — the one half of the keymap that is
// the frontend's (docs/07-UI-SPEC.md §15, K-199).
//
// Pure Dart, no engine: what is under test here is whether Flutter's idea of a
// key and the keymap's idea of a key agree. They have to agree exactly, because
// the engine matches chords as strings — a `Space` that arrives as `space` is
// simply an unbound key, and the shortcut silently does nothing.

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/keymap.dart';

void main() {
  group('key names match what the keymap stores', () {
    test('letters come through upper-cased, as the keymap spells them', () {
      expect(keyName(LogicalKeyboardKey.keyD), 'D');
      expect(keyName(LogicalKeyboardKey.keyU), 'U');
    });

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

    test('function keys and digits are already in their own form', () {
      expect(keyName(LogicalKeyboardKey.f9), 'F9');
      expect(keyName(LogicalKeyboardKey.digit1), '1');
    });

    /// A modifier alone is half a chord. Returning a name for it would let the
    /// settings page bind "Shift" to something, and then Shift would run a
    /// command every time it was used to type a capital letter.
    test('a modifier on its own has no name', () {
      for (final key in [
        LogicalKeyboardKey.shiftLeft,
        LogicalKeyboardKey.controlLeft,
        LogicalKeyboardKey.altRight,
        LogicalKeyboardKey.metaLeft,
      ]) {
        expect(keyName(key), isNull, reason: '$key is not a chord by itself');
      }
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

    testWidgets('a bare key is just its name', (tester) async {
      expect(await chordFor(tester, LogicalKeyboardKey.space), 'Space');
    });

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

  group('chordLabel reads the chord in the platform`s own words', () {
    test('off macOS the primary modifier is Ctrl', () {
      debugDefaultTargetPlatformOverride = TargetPlatform.windows;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      expect(chordLabel('Mod+Shift+P'), 'Ctrl+Shift+P');
      expect(chordLabel('Space'), 'Space');
    });

    test('on macOS it is the symbols a Mac user reads', () {
      debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      expect(chordLabel('Mod+Alt+T'), '⌘⌥T');
    });

    test('an unbound row has nothing to read', () {
      expect(chordLabel(''), '');
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

    test('the named keys and the awkward ones survive', () {
      expect(activatorForChord('Space')!.trigger, LogicalKeyboardKey.space);
      expect(activatorForChord('Shift+PageDown')!.trigger,
          LogicalKeyboardKey.pageDown);
      // A chord whose key *is* the separator must not split into nothing. It
      // is the numpad key, as `*` is for markers — the main row's `+` is
      // Shift+= and a different chord.
      expect(activatorForChord('Mod++')!.trigger, LogicalKeyboardKey.numpadAdd);
    });

    test('a chord we cannot spell shows nothing rather than the wrong thing',
        () {
      expect(activatorForChord(''), isNull);
      expect(activatorForChord('Mod+NotAKey'), isNull);
    });
  });
}
