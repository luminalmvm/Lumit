// Window ▸ Assign shortcut to this workspace.
//
// In plain terms: press the keys you want, and from then on they bring this
// arrangement of panels back.
//
// **What the chord is actually bound to** is the workspace's *place on the
// strip*, not its name: the engine's keymap has nine actions,
// `workspace.switch.1` … `9`, and the strip counts the shipped presets first
// and the user's own after them, in name order. So a workspace renamed past
// one of its neighbours swaps chords with it. That is deliberate — it is what
// makes `Alt+Shift+7` reach the same place on the strip on every launch — and
// the dialogue says so rather than letting it be a surprise later.
//
// The capture is the Keymap page's, one layer smaller: the dialogue listens
// from the moment it opens, shows what it heard, and the Apply is what
// commits — so a chord pressed by accident can be pressed again instead of
// having to be undone.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart' show BridgeKeyContext;

import '../l10n/strings.dart';
import '../state/keymap.dart';
import '../widgets/controls.dart';

/// How many workspace slots the keyboard reaches: the engine registers
/// `workspace.switch.1` … `9` and no more, so a strip longer than nine runs
/// past the keys there are.
const int workspaceSlots = 9;

/// What Window ▸ Assign shortcut to this workspace does, or null — so the row
/// greys out — when the arrangement in force is not one of the first nine on
/// the strip.
///
/// The result is said in the status line rather than in a dialogue of its own:
/// binding a key is a quiet act, and the strip is where the answer shows.
VoidCallback? assignWorkspaceShortcutAction(
  BuildContext context,
  LumitState app,
  LumitUiState ui,
) {
  final workspace = ui.workspace;
  final slot = workspace.activeWorkspaceSlot;
  if (slot == null || slot > workspaceSlots) return null;
  final name = workspace.activeUserWorkspace ??
      workspace.activePreset?.title ??
      '';
  final action = 'workspace.switch.$slot';
  return () async {
    final chord = await showWorkspaceShortcutFrb(
      context: context,
      workspace: name,
      slot: slot,
      current: ui.keymap.chordFor(action),
    );
    if (chord == null) return;
    final refusal =
        await ui.keymap.rebind(BridgeKeyContext.global, action, chord);
    app.postNotice(
      refusal ?? l10n.workspaceShortcutSet(chordLabel(chord), name),
      error: refusal != null,
    );
  };
}

/// Ask for a chord for workspace slot [slot]. Completes with the chord in the
/// engine's own notation, or null when dismissed.
Future<String?> showWorkspaceShortcutFrb({
  required BuildContext context,
  required String workspace,
  required int slot,
  String? current,
}) =>
    showLumitModal<String>(
      context: context,
      builder: (close) => _ShortcutBody(
        workspace: workspace,
        slot: slot,
        current: current,
        onConfirm: close,
        onCancel: () => close(null),
      ),
    );

class _ShortcutBody extends StatefulWidget {
  final String workspace;
  final int slot;
  final String? current;
  final ValueChanged<String> onConfirm;
  final VoidCallback onCancel;

  const _ShortcutBody({
    required this.workspace,
    required this.slot,
    required this.current,
    required this.onConfirm,
    required this.onCancel,
  });

  @override
  State<_ShortcutBody> createState() => _ShortcutBodyState();
}

class _ShortcutBodyState extends State<_ShortcutBody> {
  late String? _chord = widget.current;

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_handler);
  }

  @override
  void dispose() {
    // The handler outlives the widget otherwise, and would go on swallowing
    // every keypress in the application.
    HardwareKeyboard.instance.removeHandler(_handler);
    super.dispose();
  }

  /// Take the next chord, and keep taking them until the dialogue is closed —
  /// pressing a second one is how you change your mind before applying.
  ///
  /// Escape and Enter are left alone: they are the dialogue's own two ways
  /// out, and a shortcut nobody can leave the dialogue to use would not be
  /// much of one.
  bool _handler(KeyEvent event) {
    if (event is! KeyDownEvent) return false;
    if (event.logicalKey == LogicalKeyboardKey.escape) return false;
    if (event.logicalKey == LogicalKeyboardKey.enter ||
        event.logicalKey == LogicalKeyboardKey.numpadEnter) {
      final chord = _chord;
      if (chord != null) {
        // After this frame: applying closes the dialogue, and closing it while
        // the keyboard is still delivering the press is how a handler ends up
        // running against a dead State.
        scheduleMicrotask(() => widget.onConfirm(chord));
      }
      return true;
    }
    // A modifier on its own is half a chord: keep listening rather than
    // recording `Ctrl` as a shortcut.
    final chord = chordText(event);
    if (chord == null) return true;
    setState(() => _chord = chord);
    return true;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final chord = _chord;
    return FloatSurface(
      width: 360,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.only(bottom: 10),
            child: Text(l10n.menuAssignWorkspaceShortcut, style: t.bodyPrimary),
          ),
          Container(
            key: const ValueKey('workspace-shortcut-chord'),
            padding: const EdgeInsets.symmetric(vertical: 10),
            alignment: Alignment.center,
            decoration: BoxDecoration(
              border: Border.all(color: t.hairlineStrong),
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            ),
            child: Text(
              chord == null ? l10n.keymapPressAShortcut : chordLabel(chord),
              style: chord == null
                  ? t.small.copyWith(color: t.textMuted)
                  : t.mono.copyWith(color: t.textPrimary),
            ),
          ),
          const SizedBox(height: 10),
          // The factual line the dialogue pattern asks for (§12A.4), and the
          // rule that would otherwise be a surprise said plainly.
          Text(
            l10n.workspaceShortcutSlot(widget.workspace, '${widget.slot}'),
            key: const ValueKey('workspace-shortcut-summary'),
            style: t.caption.copyWith(color: t.textMuted),
          ),
          const SizedBox(height: 16),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              HouseButton(
                key: const ValueKey('workspace-shortcut-confirm'),
                primary: true,
                onPressed: chord == null ? null : () => widget.onConfirm(chord),
                child: Text(l10n.apply),
              ),
              const SizedBox(width: 8),
              HouseButton(
                key: const ValueKey('workspace-shortcut-cancel'),
                onPressed: widget.onCancel,
                child: Text(l10n.cancel),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
