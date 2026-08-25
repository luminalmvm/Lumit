// What Copy put down, for Paste to pick up (K-275).
//
// **In plain terms.** Copying a layer or an effect asks the engine for it as
// text — the same document a project file is made of, so everything on it
// travels: keyframes, masks, paint, switches, the lot. This holds that text
// until something pastes it.
//
// **The tray, and the system clipboard beside it** (K-275, opened up by K-302).
// The payload is a Lumit document, so this tray holds it whole; but a copy that
// leaves *no* trace on the system clipboard is a copy the machine cannot see —
// paste into a text editor and nothing arrives, which reads exactly like Copy
// having done nothing at all. So every copy is **mirrored to the system
// clipboard as text**, and a paste that finds this tray empty reads the system
// clipboard and takes a Lumit document back off it. That is also what makes
// copying between two running Lumit windows work, which is what this file's
// note used to say was owed.
//
// The tray still comes first, because it is the exact text this session copied:
// no round trip, and nothing else on the machine can have overwritten it.

import 'dart:convert';

/// What kind of thing is on the clipboard, so Paste knows what to do with the
/// text rather than guessing from its shape.
enum ClipboardKind { layer, effects }

/// The one tray. Held by the shell state, read by the Edit menu.
class LumitClipboard {
  ClipboardKind? _kind;
  String? _text;

  /// What is on it, or null when nothing has been copied this session.
  ClipboardKind? get kind => _kind;

  bool get isEmpty => _text == null;

  /// The copied document, or null. Paired with [kind]: a caller reads both or
  /// neither, which is why they are not two fields to keep in step by hand.
  String? get text => _text;

  /// Put a layer document down (from `LayerReference.copyLayer`).
  void putLayer(String text) {
    _kind = ClipboardKind.layer;
    _text = text;
  }

  /// Put an effect document down (from `LayerReference.copyEffects`) — one
  /// effect or a whole stack; both are the same `.lumfx` shape, so both paste
  /// the same way.
  void putEffects(String text) {
    _kind = ClipboardKind.effects;
    _text = text;
  }

  void clear() {
    _kind = null;
    _text = null;
  }
}

/// What kind of Lumit document [text] is, or null when it is not one (K-302).
///
/// Sniffed rather than trusted: this is asked of whatever happens to be on the
/// *system* clipboard, which is most often a shopping list. A layer document
/// says so in `kind`; an effect document is the `.lumfx` preset shape, which
/// names its effects and has no `kind` — the same two shapes the engine's
/// `paste_layer` and `paste_effects` accept, so nothing is offered here that
/// they would then refuse.
ClipboardKind? lumitDocumentKind(String text) {
  final trimmed = text.trimLeft();
  if (!trimmed.startsWith('{')) return null;
  try {
    final decoded = jsonDecode(text);
    if (decoded is! Map<String, dynamic>) return null;
    if (decoded['kind'] == 'layer' && decoded['layer'] != null) {
      return ClipboardKind.layer;
    }
    if (decoded['effects'] is List) return ClipboardKind.effects;
    return null;
  } catch (_) {
    // Not JSON at all: ordinary text somebody copied somewhere else.
    return null;
  }
}
