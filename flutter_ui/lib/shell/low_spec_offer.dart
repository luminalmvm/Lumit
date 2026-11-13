// Low-spec mode, offered once on a machine that looks too small for Lumit's
// defaults.
//
// After the first-run questions, on the first launch that finds little video
// memory or little memory. Asked once: the answer, either way, is kept, and
// the switch stays in Settings ▸ Performance.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/system.dart'
    show powerSaving, systemMemoryBytes, videoMemoryBytes;

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

/// Reads the power every few seconds for Low-spec mode on battery, and offers
/// that switch once, the first time Lumit finds itself on battery.
class BatteryWatch {
  final LumitUiState ui;
  final bool Function() ask;
  Timer? _timer;
  bool _asking = false;

  BatteryWatch(this.ui, {this.ask = powerSaving});

  /// Start reading, from a [context] that can show the offer.
  void start(BuildContext context) {
    _read(context);
    _timer = Timer.periodic(const Duration(seconds: 5), (_) => _read(context));
  }

  void stop() => _timer?.cancel();

  void _read(BuildContext context) {
    bool saving;
    try {
      saving = ask();
    } catch (_) {
      return;
    }
    final perf = ui.workspace.performance;
    if (saving &&
        !_asking &&
        !perf.lowSpecBatteryOffered &&
        !perf.lowSpecOnBattery &&
        !perf.lowSpecMode &&
        context.mounted) {
      _asking = true;
      unawaited(_offer(context).whenComplete(() => _asking = false));
      return;
    }
    ui.followBattery(saving);
  }

  /// Asked once: the answer, either way, is kept, and the switch stays in
  /// Settings ▸ Performance.
  Future<void> _offer(BuildContext context) async {
    final perf = ui.workspace.performance;
    perf.lowSpecBatteryOffered = true;
    ui.workspace.settingsChanged();
    final go = await showLumitModal<bool>(
      context: context,
      builder: (close) => QuestionWindow(
        title: l10n.lowSpecBatteryOfferTitle,
        body: l10n.lowSpecBatteryOfferBody,
        keyPrefix: 'low-spec-battery-offer',
        onClose: () => close(false),
        actions: [
          questionPlain('low-spec-battery-offer-no', l10n.lowSpecOfferNo,
              () => close(false)),
          questionFilled('low-spec-battery-offer-yes', l10n.lowSpecOfferYes,
              () => close(true)),
        ],
      ),
    );
    if (go == true) {
      ui.setLowSpecOnBattery(true);
      try {
        ui.followBattery(ask());
      } catch (_) {
        // The power could not be read; the next reading follows it.
      }
    }
  }
}
