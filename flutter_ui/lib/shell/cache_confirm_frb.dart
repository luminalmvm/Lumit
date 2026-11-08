// Clearing the disk tier, with a question first.
//
// The other two tiers are cleared on a click and nothing is lost but time: RAM
// and VRAM frames cost one re-render each. The disk tier is different in kind:
// it holds files that may represent a night's rendering, it survives closing the
// application, and there is nothing to undo. So this is the one cache control
// that asks, and it says how much is about to go (docs/07-UI-SPEC.md §15).
//
// Shared by both places the tier can be emptied, the Settings page's Clear
// button and the status line's disk meter, so the two cannot drift into asking
// differently, or one of them not asking at all.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'question_window.dart';

/// Ask, then clear. Returns true when frames were actually thrown away, so a
/// caller can refresh its readout; false when the user said no, or when there
/// was nothing on disk to clear in the first place (no dialogue then: a
/// question about deleting nothing is just noise).
Future<bool> confirmClearDiskCache(BuildContext context) async {
  final before = diskCacheStats();
  if (before.entries == 0) return false;

  final confirmed = await showLumitModal<bool>(
    context: context,
    builder: (close) => QuestionWindow(
      title: l10n.cacheDeleteTitle,
      summary: l10n.cacheDeleteFacts(before.entries.toInt(),
          (before.usedBytes.toInt() / (1 << 20)).round()),
      keyPrefix: 'disk-clear',
      onClose: () => close(false),
      actions: [
        questionPlain(
            'disk-clear-cancel', l10n.cacheKeepThem, () => close(false)),
        questionFilled('disk-clear-confirm', l10n.delete, () => close(true)),
      ],
    ),
  );
  if (confirmed != true) return false;
  clearDiskCache();
  return true;
}
