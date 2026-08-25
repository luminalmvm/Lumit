// The first-run screen: one question, asked once (K-246, docs/07 §13.1).
//
// On the very first launch — a machine with no settings file — Lumit asks how
// the user edits, and sets the two preferences of K-246 from the answer. That
// is the whole screen, plus the update tick along the bottom (K-296): a
// preference primer, not a tour, and not a wizard. Every setting it writes is
// an ordinary row in Settings afterwards — the editing pair under Interface ▸
// Editing, the tick under General ▸ Updates — so nothing here is a decision
// anybody is stuck with.
//
// It is deliberately plain for now. The four cards of docs/07 §13.1, each with
// a small image showing what the choice does, are the destination; the owner
// asked for the simple version first and the polish is in docs/TODO.md.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../state/workspace.dart';
import '../widgets/controls.dart';

/// What the screen comes back with: which editor, and whether Lumit should
/// keep an eye out for new versions (K-296). The tick is on the screen rather
/// than only in Settings because it is a decision about how Lumit behaves from
/// now on, which is exactly what this screen is for.
typedef FirstRunAnswer = ({bool? vegas, bool autoUpdate});

/// Show the screen if this machine has never answered it, and record the
/// answer. Does nothing at all on any later launch, so callers can call it
/// unconditionally at start-up.
Future<void> maybeShowFirstRunFrb(
    BuildContext context, Workspace workspace) async {
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
  }
}

class _FirstRun extends StatefulWidget {
  final ValueChanged<FirstRunAnswer?> onChoose;
  const _FirstRun({required this.onChoose});

  @override
  State<_FirstRun> createState() => _FirstRunState();
}

class _FirstRunState extends State<_FirstRun> {
  /// Ticked to begin with (K-296). Nothing is downloaded either way — this is
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
class _Choice extends StatelessWidget {
  final String id;
  final String title;
  final String blurb;
  final VoidCallback onTap;

  const _Choice({
    required this.id,
    required this.title,
    required this.blurb,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      key: ValueKey<String>(id),
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: Container(
        padding: const EdgeInsets.all(12),
        decoration: BoxDecoration(
          color: t.surface2,
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          border: Border.all(color: t.hairline, width: 1),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(title, style: t.bodyPrimary),
            const SizedBox(height: 6),
            Text(blurb, style: t.small.copyWith(color: t.textMuted)),
          ],
        ),
      ),
    );
  }
}
