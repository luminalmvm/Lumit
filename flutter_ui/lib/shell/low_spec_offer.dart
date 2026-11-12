// Low-spec mode, offered once on a machine that looks too small for Lumit's
// defaults.
//
// After the first-run questions, on the first launch that finds little video
// memory or little memory. Asked once: the answer, either way, is kept, and
// the switch stays in Settings ▸ Performance.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/system.dart'
    show systemMemoryBytes, videoMemoryBytes;

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'question_window.dart';

/// Below this much video memory a machine is offered Low-spec mode.
const int lowSpecVideoMemory = 3 << 30;

/// Below this much memory a machine is offered Low-spec mode.
const int lowSpecMemory = 12 << 30;

/// Whether a machine with `vram` bytes of video memory and `ram` of memory is
/// small enough to be offered Low-spec mode. Nought is "not known" and counts
/// for nothing.
bool looksLowSpec({required int vram, required int ram}) =>
    (vram > 0 && vram < lowSpecVideoMemory) || (ram > 0 && ram < lowSpecMemory);

/// Offer Low-spec mode once, on a machine that looks small.
Future<void> maybeOfferLowSpec(BuildContext context, LumitUiState ui) async {
  final perf = ui.workspace.performance;
  if (perf.lowSpecMode || perf.lowSpecOffered) return;
  if (!looksLowSpec(vram: videoMemoryBytes(), ram: systemMemoryBytes())) {
    return;
  }
  perf.lowSpecOffered = true;
  ui.workspace.settingsChanged();
  final go = await showLumitModal<bool>(
    context: context,
    builder: (close) => QuestionWindow(
      title: l10n.lowSpecOfferTitle,
      body: l10n.lowSpecOfferBody,
      keyPrefix: 'low-spec-offer',
      onClose: () => close(false),
      actions: [
        questionPlain('low-spec-offer-no', l10n.lowSpecOfferNo,
            () => close(false)),
        questionFilled('low-spec-offer-yes', l10n.lowSpecOfferYes,
            () => close(true)),
      ],
    ),
  );
  if (go == true) ui.setLowSpecMode(true);
}
