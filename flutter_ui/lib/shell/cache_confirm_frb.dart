// Clearing the disk tier, with a question first.
//
// The other two tiers are cleared on a click and nothing is lost but time: RAM
// and VRAM frames cost one re-render each. The disk tier is different in kind —
// it holds files that may represent a night's rendering, it survives closing the
// application, and there is nothing to undo. So this is the one cache control
// that asks, and it says how much is about to go (docs/07-UI-SPEC.md §15).
//
// Shared by both places the tier can be emptied — the Settings page's Clear
// button and the status line's disk meter — so the two cannot drift into asking
// differently, or one of them not asking at all.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';

/// Ask, then clear. Returns true when frames were actually thrown away, so a
/// caller can refresh its readout; false when the user said no, or when there
/// was nothing on disk to clear in the first place (no dialogue then — a
/// question about deleting nothing is just noise).
Future<bool> confirmClearDiskCache(BuildContext context) async {
  final before = diskCacheStats();
  if (before.entries == BigInt.zero) return false;

  final confirmed = await showLumitModal<bool>(
    context: context,
    builder: (close) => _ConfirmClearDisk(
      entries: before.entries.toInt(),
      megabytes: (before.usedBytes.toInt() / (1 << 20)).round(),
      onChoose: close,
    ),
  );
  if (confirmed != true) return false;
  clearDiskCache();
  return true;
}

class _ConfirmClearDisk extends StatelessWidget {
  final int entries;
  final int megabytes;
  final ValueChanged<bool?> onChoose;

  const _ConfirmClearDisk({
    required this.entries,
    required this.megabytes,
    required this.onChoose,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      width: 380,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.all(10),
            child: Text(l10n.cacheDeleteTitle, style: t.bodyPrimary),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Text(
              '$entries frames, $megabytes MB. They are only a cache, so '
              'nothing in your project is lost — but they will have to be '
              'rendered again.',
              style: t.small.copyWith(color: t.textMuted),
            ),
          ),
          const SizedBox(height: 14),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                HouseButton(
                  key: const ValueKey('disk-clear-cancel'),
                  small: true,
                  onPressed: () => onChoose(false),
                  child: Text(l10n.cacheKeepThem, style: t.small),
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: const ValueKey('disk-clear-confirm'),
                  small: true,
                  // The window's default action: focused on open, so Enter
                  // confirms, and drawn with the accent edge that says so.
                  primary: true,
                  autofocus: true,
                  onPressed: () => onChoose(true),
                  // No style here: the filled action sets its own label
                  // (mono capitals, §7.1).
                  child: Text(l10n.delete),
                ),
              ],
            ),
          ),
          const SizedBox(height: 10),
        ],
      ),
    );
  }
}
