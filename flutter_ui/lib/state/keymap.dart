// The keyboard: turning a real keypress into a chord, and asking the engine
// what that chord means here (docs/07-UI-SPEC.md §15).
//
// **What lives here and what does not.** This file knows how a Flutter
// `KeyEvent` spells itself — that `LogicalKeyboardKey.pageUp` is the key the
// keymap calls `PageUp`, and that the primary modifier is Cmd on a Mac and Ctrl
// everywhere else. That is platform knowledge, and it belongs on the platform's
// side of the seam. Everything after that is the engine's: which action a chord
// runs, whether the focused panel outranks the app-wide binding, what clashes
// with what, and what the keymap file says. `crates/lumit-bridge/src/api/keymap.rs`
// answers all of those and this file only asks.
//
// The keymap itself is kept by the engine for the session. Its *file* is ours,
// because a keymap is machine-local settings and the workspace file is where
// those live — [KeymapState] stores the engine's JSON verbatim and never looks
// inside it.

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart' show PointerScrollEvent;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart' show SingleActivator;
import 'package:flutter_rust_bridge/flutter_rust_bridge.dart';

import '../src/rust/api/keymap.dart';
import 'os_keys.dart';
import 'workspace.dart';

/// The keys the keymap names in words, by the logical key Flutter reports.
///
/// Only the ones whose Lumit name differs from what `keyLabel` would give, or
/// where `keyLabel` is unreliable across layouts. Letters, digits and
/// punctuation fall through to the label, which is already what the keymap
/// spells them as.
final Map<LogicalKeyboardKey, String> _namedKeys = {
  LogicalKeyboardKey.space: 'Space',
  LogicalKeyboardKey.pageUp: 'PageUp',
  LogicalKeyboardKey.pageDown: 'PageDown',
  LogicalKeyboardKey.home: 'Home',
  LogicalKeyboardKey.end: 'End',
  LogicalKeyboardKey.enter: 'Enter',
  LogicalKeyboardKey.numpadEnter: 'Enter',
  LogicalKeyboardKey.backspace: 'Backspace',
  LogicalKeyboardKey.delete: 'Delete',
  LogicalKeyboardKey.escape: 'Escape',
  LogicalKeyboardKey.tab: 'Tab',
  LogicalKeyboardKey.arrowUp: 'ArrowUp',
  LogicalKeyboardKey.arrowDown: 'ArrowDown',
  LogicalKeyboardKey.arrowLeft: 'ArrowLeft',
  LogicalKeyboardKey.arrowRight: 'ArrowRight',
  // The marker key is AE's numpad asterisk. On the main rows `*` is Shift+8,
  // which is a different chord and deliberately not folded into this one.
  LogicalKeyboardKey.numpadMultiply: '*',
  LogicalKeyboardKey.numpadAdd: '+',
  LogicalKeyboardKey.numpadSubtract: '-',
  LogicalKeyboardKey.numpadDecimal: '.',
};

/// The modifier keys themselves, which are never a chord on their own — holding
/// Shift is not a shortcut, it is half of one.
final Set<LogicalKeyboardKey> _modifierKeys = {
  LogicalKeyboardKey.shift,
  LogicalKeyboardKey.shiftLeft,
  LogicalKeyboardKey.shiftRight,
  LogicalKeyboardKey.control,
  LogicalKeyboardKey.controlLeft,
  LogicalKeyboardKey.controlRight,
  LogicalKeyboardKey.alt,
  LogicalKeyboardKey.altLeft,
  LogicalKeyboardKey.altRight,
  LogicalKeyboardKey.meta,
  LogicalKeyboardKey.metaLeft,
  LogicalKeyboardKey.metaRight,
  LogicalKeyboardKey.capsLock,
};

/// The keymap's name for one logical key, or null when it has none we can
/// spell — a dead key, or a media key no shortcut should claim.
String? keyName(LogicalKeyboardKey key) {
  if (_modifierKeys.contains(key)) return null;
  final named = _namedKeys[key];
  if (named != null) return named;
  // Function keys are F-and-a-number in both worlds.
  final label = key.keyLabel;
  if (label.isEmpty) return null;
  // A single letter is upper-cased, which is how the keymap stores it; anything
  // else (a digit, punctuation, "F9") is already in its own canonical form.
  return label.length == 1 ? label.toUpperCase() : label;
}

/// The chord [event] spells, in the engine's canonical `Mod+Alt+Shift+Key`
/// form — or null when this keypress cannot be one (a modifier held alone, or
/// a key with no name).
///
/// `Mod` is the platform's primary modifier: Cmd on macOS, Ctrl elsewhere. The
/// keymap and its shareable file stay platform-neutral because of that
/// translation, so a keymap written on a Mac still reads on Windows.
String? chordText(KeyEvent event) {
  final name = keyName(event.logicalKey);
  if (name == null) return null;
  final keys = HardwareKeyboard.instance.logicalKeysPressed;
  bool held(LogicalKeyboardKey a, LogicalKeyboardKey b) =>
      keys.contains(a) || keys.contains(b);
  final primary = defaultTargetPlatform == TargetPlatform.macOS
      ? held(LogicalKeyboardKey.metaLeft, LogicalKeyboardKey.metaRight)
      : held(LogicalKeyboardKey.controlLeft, LogicalKeyboardKey.controlRight);
  final alt = held(LogicalKeyboardKey.altLeft, LogicalKeyboardKey.altRight);
  final shift =
      held(LogicalKeyboardKey.shiftLeft, LogicalKeyboardKey.shiftRight);
  return [
    if (primary) 'Mod',
    if (alt) 'Alt',
    if (shift) 'Shift',
    name,
  ].join('+');
}

/// The logical key each keymap name stands for — [_namedKeys] read backwards.
/// Where two keys share a name (`Enter` is both), the first declared wins, so
/// the main-row key is the one a menu shows.
final Map<String, LogicalKeyboardKey> _keysByName = {
  for (final e in _namedKeys.entries.toList().reversed) e.value: e.key,
};

/// A chord as macOS's own menu bar wants it: a [SingleActivator] for the native
/// `PlatformMenuItem` to draw beside its row.
///
/// Only the native menu needs this — everywhere else the keyboard is the
/// engine's business and a chord is text. `Mod` becomes Cmd, because macOS is
/// the only place this is asked. Null when the chord names a key we cannot
/// spell as a logical key, so a row shows no shortcut rather than the wrong one.
SingleActivator? activatorForChord(String chord) {
  if (chord.isEmpty) return null;
  // `Mod++` would split into an empty last part; the trailing `+` *is* the key.
  final keyText = chord.endsWith('+') ? '+' : chord.split('+').last;
  final mods = chord.substring(0, chord.length - keyText.length).split('+');
  final key = _keysByName[keyText] ??
      (keyText.length == 1
          ? LogicalKeyboardKey(keyText.toLowerCase().codeUnitAt(0))
          : null);
  if (key == null) return null;
  return SingleActivator(
    key,
    meta: mods.contains('Mod'),
    alt: mods.contains('Alt'),
    shift: mods.contains('Shift'),
  );
}

/// How a chord is *shown* on this machine: `Mod` becomes the symbol or word the
/// platform's own menus use, so a Windows user reads Ctrl and a Mac user reads
/// ⌘. The stored form never changes — only the reading of it.
///
/// The keys the keymap names in one word read the way their caps do: an arrow
/// is an arrow, and PageUp is Page Up.
String chordLabel(String chord) {
  if (chord.isEmpty) return '';
  final mac = defaultTargetPlatform == TargetPlatform.macOS;
  return chord
      .replaceAll('Mod+', mac ? '⌘' : 'Ctrl+')
      .replaceAll('Alt+', mac ? '⌥' : 'Alt+')
      .replaceAll('Shift+', mac ? '⇧' : 'Shift+')
      .replaceAll('ArrowLeft', '←')
      .replaceAll('ArrowRight', '→')
      .replaceAll('ArrowUp', '↑')
      .replaceAll('ArrowDown', '↓')
      .replaceAll('PageUp', 'Page Up')
      .replaceAll('PageDown', 'Page Down');
}

/// How far a wheel event moved, on whichever axis carries it. Some platforms
/// turn a Shift+wheel sideways, and the modifier can be any of the three.
double wheelDelta(PointerScrollEvent event) {
  final d = event.scrollDelta;
  return d.dy.abs() >= d.dx.abs() ? d.dy : d.dx;
}

/// The live keymap: the table Settings → Keymap draws, and the lookup every
/// keypress goes through.
///
/// Holds no bindings of its own — [groups] and [conflicts] are what the engine
/// last answered, refreshed whenever an edit changes them. Every edit here
/// writes through to the engine first and stores the result second, so the
/// keymap that dispatches and the keymap on disk cannot drift apart.
class KeymapState extends ChangeNotifier {
  KeymapState({Workspace? workspace}) : _workspace = workspace {
    _restore();
    refresh();
  }

  final Workspace? _workspace;

  List<BridgeKeymapGroup> _groups = const [];
  List<BridgeKeyConflict> _conflicts = const [];
  List<BridgeKeyShadow> _shadows = const [];
  List<BridgeWheelBinding> _wheel = const [];
  BridgeHandleModifier _breakHandles = BridgeHandleModifier.alt;

  /// The whole table, grouped by where each binding is live.
  List<BridgeKeymapGroup> get groups => _groups;

  /// Chords that could fire two actions at once. Empty in the shipped keymap;
  /// the settings page warns when a rebind makes one.
  List<BridgeKeyConflict> get conflicts => _conflicts;

  /// Chords a panel has taken over from an app-wide binding. Not
  /// clashes — the focused panel wins by a stated rule — but worth saying,
  /// because the app-wide meaning stops working in that one panel. The shipped
  /// keymap carries one on purpose (`L` in the Timeline).
  List<BridgeKeyShadow> get shadows => _shadows;

  /// Which modifier turns the scroll wheel into each of its actions, as the
  /// engine last answered.
  List<BridgeWheelBinding> get wheel => _wheel;

  /// Whether the modifier [action] answers to is held right now. Read from the
  /// cached rows, so a wheel event never waits on the engine.
  bool wheelHeld(BridgeWheelAction action) {
    final keys = HardwareKeyboard.instance;
    for (final row in _wheel) {
      if (row.action != action) continue;
      return switch (row.modifier) {
        BridgeWheelModifier.ctrl => keys.isControlPressed,
        BridgeWheelModifier.alt => altActuallyHeld(),
        BridgeWheelModifier.shift => keys.isShiftPressed,
      };
    }
    return false;
  }

  /// Give a wheel action a new modifier. Whatever shares its panel swaps.
  Future<void> setWheel(
      BridgeWheelAction action, BridgeWheelModifier modifier) async {
    _wheel = await keymapSetWheel(action: action, modifier: modifier);
    _store();
    notifyListeners();
  }

  /// The modifier that breaks or joins a keyframe's handles as a drag begins
  /// in the Graph editor.
  BridgeHandleModifier get breakHandles => _breakHandles;

  /// Whether that modifier is held right now.
  bool get breakHandlesHeld => switch (_breakHandles) {
        BridgeHandleModifier.alt => altActuallyHeld(),
        BridgeHandleModifier.ctrl => HardwareKeyboard.instance.isControlPressed,
      };

  Future<void> setBreakHandles(BridgeHandleModifier modifier) async {
    _breakHandles = await keymapSetBreakHandles(modifier: modifier);
    _store();
    notifyListeners();
  }

  /// The search text above the table. Held here rather than in the page so it
  /// survives the page being closed and reopened.
  String _query = '';
  String get query => _query;
  set query(String value) {
    if (_query == value) return;
    _query = value;
    notifyListeners();
  }

  /// The rows matching [query], or the whole table when it is empty. The
  /// filtering is the engine's — it matches the description, the id and the
  /// chord, and only it knows all three.
  List<BridgeKeymapGroup> get visibleGroups {
    if (_query.trim().isEmpty) return _groups;
    final hits = keymapSearch(query: _query.trim())
        .map((b) => '${b.context}/${b.action}')
        .toSet();
    return _groups
        .map((g) => BridgeKeymapGroup(
              context: g.context,
              label: g.label,
              bindings: g.bindings
                  .where((b) => hits.contains('${b.context}/${b.action}'))
                  .toList(),
            ))
        .where((g) => g.bindings.isNotEmpty)
        .toList();
  }

  /// What [event] does while [context] is the focused panel, or null for
  /// nothing bound. This is the dispatch path — one sync call per keypress.
  String? actionFor(BridgeKeyContext context, KeyEvent event) {
    final chord = chordText(event);
    if (chord == null) return null;
    return keymapLookup(context: context, chord: chord);
  }

  /// The chord [action] answers to, as this machine spells it — for a tooltip
  /// that teaches the shortcut (docs/07 §14: every icon control names itself
  /// and its current shortcut). Null when the action has no binding, so a
  /// caller shows the name alone rather than an empty bracket.
  ///
  /// Read from the table the engine last gave us rather than by asking per
  /// hover: this is drawn on every tooltip and the answer only changes when a
  /// rebind lands, which is exactly when [_adopt] refreshes it.
  String? chordFor(String action) {
    final raw = rawChordFor(action);
    return raw == null ? null : chordLabel(raw);
  }

  /// The same binding in the engine's own spelling (`Mod+Shift+Z`), for the one
  /// caller that needs the parts rather than the reading: the macOS native menu,
  /// which wants a [SingleActivator].
  String? rawChordFor(String action) {
    for (final group in _groups) {
      for (final binding in group.bindings) {
        if (binding.action == action && binding.chord.isNotEmpty) {
          return binding.chord;
        }
      }
    }
    return null;
  }

  Map<String, List<BridgeKeyBinding>> _byChord = const {};

  /// Everything [chord] runs, in every context. The table shows one chord a
  /// row, so the keyboard picture reads this to light a key for an action's
  /// second chord too.
  List<BridgeKeyBinding> bindingsFor(String chord) =>
      _byChord[chord] ?? const [];

  /// An empty search is every binding the engine holds.
  void _indexChords() {
    final byChord = <String, List<BridgeKeyBinding>>{};
    for (final binding in keymapSearch(query: '')) {
      (byChord[binding.chord] ??= []).add(binding);
    }
    _byChord = byChord;
  }

  /// Re-read the table, the conflicts and the shadows from the engine.
  void refresh() {
    _groups = keymapGroups();
    _conflicts = keymapConflicts();
    _shadows = keymapShadows();
    _wheel = keymapWheel();
    _breakHandles = keymapBreakHandles();
    _indexChords();
    notifyListeners();
  }

  void _adopt(List<BridgeKeymapGroup> groups) {
    _groups = groups;
    _conflicts = keymapConflicts();
    _shadows = keymapShadows();
    _wheel = keymapWheel();
    _breakHandles = keymapBreakHandles();
    _indexChords();
    _store();
    notifyListeners();
  }

  /// Point an action at a new chord. Returns null on success, or the engine's
  /// own words when the text was not a chord — which the page shows rather
  /// than swallowing.
  Future<String?> rebind(
      BridgeKeyContext context, String action, String chord) async {
    try {
      _adopt(await keymapRebind(
          context: context, action: action, chord: chord));
      return null;
    } on AnyhowException catch (e) {
      return e.message;
    }
  }

  /// Leave an action with no chord at all.
  Future<void> unbind(BridgeKeyContext context, String action) async {
    _adopt(await keymapUnbind(context: context, action: action));
  }

  /// Put one row back to what the shipped keymap gives it.
  Future<void> resetBinding(BridgeKeyContext context, String action) async {
    _adopt(await keymapResetBinding(context: context, action: action));
  }

  /// Replace the whole keymap with a shipped preset.
  Future<void> loadPreset(BridgeKeymapPreset preset) async {
    _adopt(await keymapLoadPreset(preset: preset));
  }

  /// The whole keymap as text, for "Export keymap…". The same format the
  /// workspace stores, so a file a user shares is a file that restores.
  String toJson() => keymapToJson();

  /// Take a keymap from text — an imported file, or the stored blob. Returns
  /// null on success or the engine's words when the text is not a keymap.
  Future<String?> fromJson(String json) async {
    try {
      _adopt(await keymapFromJson(json: json));
      return null;
    } on AnyhowException catch (e) {
      return e.message;
    }
  }

  /// Take the shortcuts from an After Effects shortcut file. Returns how many
  /// actions took their keys from it, or null when the engine refused the
  /// text and left the keymap alone.
  Future<int?> importAfterEffects(String text) async {
    try {
      final result = await keymapImportAfterEffects(text: text);
      _adopt(result.groups);
      return result.actions;
    } catch (_) {
      return null;
    }
  }

  /// Hand the engine the workspace's shortcuts again, after a profile or a
  /// sync put different ones there. None stored is the shipped defaults.
  Future<void> restore() async {
    final stored = _workspace?.keymapJson;
    if (stored == null || stored.isEmpty) {
      await loadPreset(BridgeKeymapPreset.lumit);
    } else {
      await fromJson(stored);
    }
  }

  /// Hand the engine whatever the workspace stored, if anything. A blob that
  /// no longer parses is ignored rather than fatal: the session starts on the
  /// shipped defaults, which is a working keyboard, and the next edit rewrites
  /// the store.
  void _restore() {
    final stored = _workspace?.keymapJson;
    if (stored == null || stored.isEmpty) return;
    try {
      keymapFromJson(json: stored);
    } catch (_) {
      // Defaults it is.
    }
  }

  void _store() {
    final workspace = _workspace;
    if (workspace == null) return;
    workspace.setKeymapJson(keymapToJson());
  }
}
