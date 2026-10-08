// The first-run screen: one question, asked once (docs/07 §13.1).
//
// On the very first launch — a machine with no settings file — Lumit asks how
// the user edits, and sets the two editing preferences from the answer. The
// After Effects answer loads the After Effects shortcuts with them. That
// is the whole screen, plus the update tick along the bottom: a preference
// primer, not a tour, and not a wizard. Every setting it writes is an ordinary
// row in Settings afterwards — the editing pair under Interface ▸ Editing, the
// tick under General ▸ Updates — so nothing here is a decision anybody is
// stuck with.
//
// It is deliberately plain for now. The four cards of docs/07 §13.1, each with
// a small image showing what the choice does, are the destination; the owner
// asked for the simple version first and the polish is in docs/TODO.md.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';

import '../l10n/strings.dart';
import '../state/keymap.dart';
import '../state/workspace.dart';
import '../widgets/controls.dart';

/// What the screen comes back with: which editor, and whether Lumit should
/// keep an eye out for new versions. The tick is on the screen rather than
/// only in Settings because it is a decision about how Lumit behaves from now
/// on, which is exactly what this screen is for.
typedef FirstRunAnswer = ({bool? vegas, bool autoUpdate});

/// Show the screen if this machine has never answered it, and record the
/// answer. Does nothing at all on any later launch, so callers can call it
/// unconditionally at start-up.
///
/// [keymap] is what the After Effects answer loads its shortcuts into.
Future<void> maybeShowFirstRunFrb(BuildContext context, Workspace workspace,
    {KeymapState? keymap}) async {
  if (workspace.firstRunDone) return;
  final answer = await showLumitModal<FirstRunAnswer>(
    context: context,
    initialSize: const Size(560, 380),
    minSize: const Size(460, 320),
    builder: (close) => _FirstRun(onChoose: close),
  );
  // A null answer is a click on the scrim, which is the same as Skip: the
  // question has been put, so it is not put again, and the defaults stand —
  // the After Effects shape, with update checks on.
  workspace.setAutoUpdate(answer?.autoUpdate ?? true);
  final vegas = answer?.vegas;
  if (vegas == null) {
    workspace.skipFirstRun();
  } else {
    workspace.setEditingStyle(vegas: vegas);
    // The card says After Effects, so its keys come too: the preset the
    // After Effects button on Settings > Shortcuts loads.
    if (!vegas) await keymap?.loadPreset(BridgeKeymapPreset.afterEffects);
  }
}

class _FirstRun extends StatefulWidget {
  final ValueChanged<FirstRunAnswer?> onChoose;
  const _FirstRun({required this.onChoose});

  @override
  State<_FirstRun> createState() => _FirstRunState();
}

class _FirstRunState extends State<_FirstRun> {
  /// Ticked to begin with. Nothing is downloaded either way — this is
  /// permission to look, not permission to fetch.
  bool _autoUpdate = true;

  /// Answer with both halves at once: the editor, and the update tick as it
  /// stands when the choice is made.
  void _answer(bool? vegas) =>
      widget.onChoose((vegas: vegas, autoUpdate: _autoUpdate));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      child: SizedBox.expand(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(14, 12, 14, 4),
              child: Text(l10n.firstRunTitle, style: t.bodyPrimary),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(14, 0, 14, 12),
              child: Text(
                l10n.firstRunBlurb,
                style: t.small.copyWith(color: t.textMuted),
              ),
            ),
            Expanded(
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 14),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Expanded(
                      child: _Choice(
                        id: 'first-run-ae',
                        title: l10n.keymapAfterEffects,
                        blurb: l10n.firstRunAfterEffects,
                        note: l10n.firstRunAfterEffectsKeys,
                        onTap: () => _answer(false),
                      ),
                    ),
                    const SizedBox(width: 10),
                    Expanded(
                      child: _Choice(
                        id: 'first-run-vegas',
                        title: l10n.firstRunVegasName,
                        blurb: l10n.firstRunVegas,
                        onTap: () => _answer(true),
                      ),
                    ),
                  ],
                ),
              ),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
              child: Row(
                children: [
                  // The update tick sits beside Skip rather than in a section
                  // of its own: it is a second, much smaller question, and
                  // giving it a heading would suggest the two are equals.
                  HouseCheckbox(
                    key: const ValueKey('first-run-auto-update'),
                    value: _autoUpdate,
                    onChanged: (on) => setState(() => _autoUpdate = on),
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      l10n.firstRunAutoUpdate,
                      style: t.small.copyWith(color: t.textMuted),
                    ),
                  ),
                  const SizedBox(width: 12),
                  HouseButton(
                    key: const ValueKey('first-run-skip'),
                    small: true,
                    frameless: true,
                    onPressed: () => _answer(null),
                    child: Text(l10n.skip, style: t.small),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// One answer: a tall card that is entirely the button, because the blurb is
/// as much a part of the choice as the name at the top of it.
class _Choice extends StatefulWidget {
  final String id;
  final String title;
  final String blurb;

  /// One more line under the blurb, for something else the answer does.
  final String? note;
  final VoidCallback onTap;

  const _Choice({
    required this.id,
    required this.title,
    required this.blurb,
    this.note,
    required this.onTap,
  });

  @override
  State<_Choice> createState() => _ChoiceState();
}

class _ChoiceState extends State<_Choice> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        key: ValueKey<String>(widget.id),
        behavior: HitTestBehavior.opaque,
        onTap: widget.onTap,
        child: Container(
          padding: const EdgeInsets.all(12),
          decoration: BoxDecoration(
            // The welcome screen's cards: under the pointer the fill and the
            // hairline both come up. Two steps of fill here, because the
            // window these sit on is itself the step between.
            color: _hover ? t.surface4 : t.surface2,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(
                color: _hover ? t.hairlineStrong : t.hairline, width: 1),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(widget.title, style: t.bodyPrimary),
              const SizedBox(height: 6),
              Text(widget.blurb, style: t.small.copyWith(color: t.textMuted)),
              if (widget.note case final note?) ...[
                const SizedBox(height: 6),
                Text(note, style: t.small.copyWith(color: t.textMuted)),
              ],
            ],
          ),
        ),
      ),
    );
  }
}
