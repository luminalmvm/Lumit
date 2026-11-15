// Hearing speech: starting a transcription, the small dialogue that installs
// the speech model when it is missing, and the words every progress readout
// shares.
//
// The model is two addons: the runtime every model runs on, and a speech
// pack. The catalogue may offer more than one pack; the larger hears better
// and is offered first.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/transcript.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/addons.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'settings_rows.dart' show byteSize;

/// Hear [items], asking to install the speech model first when it is missing.
/// What [TranscriptionState.start] returns: null when nothing ran.
Future<BridgeTranscribeOutcome?> transcribeWithPrompt(BuildContext context,
        List<FootageReference> items, BridgeTranscribeOptions options) =>
    context
        .read<LumitState>()
        .transcription
        .start(items, options, install: () => showSpeechInstallDialog(context));

/// Offer the runtime and a speech pack, and install what is picked. True once
/// both are in place.
Future<bool> showSpeechInstallDialog(BuildContext context) async {
  final service = context.read<LumitUiState>().addons;
  final done = await showLumitModal<bool>(
    context: context,
    builder: (close) => SpeechInstallBody(service: service, onClose: close),
  );
  return done ?? false;
}

/// Whether [service] has a speech pack installed.
bool hasSpeechPack(AddonService service) =>
    service.packs.any((pack) => pack.task == 'speech' && !pack.broken);

/// The dialogue's body, apart from where it is shown so a test can hand it a
/// service of its own.
class SpeechInstallBody extends StatefulWidget {
  final AddonService service;
  final void Function(bool installed) onClose;
  const SpeechInstallBody(
      {super.key, required this.service, required this.onClose});

  @override
  State<SpeechInstallBody> createState() => _SpeechInstallBodyState();
}

class _SpeechInstallBodyState extends State<SpeechInstallBody> {
  AddonService get _service => widget.service;

  /// The pack picked, by id.
  String? _pack;

  /// An install was asked for and is under way, runtime first.
  bool _installing = false;

  @override
  void initState() {
    super.initState();
    _service.addListener(_heard);
    _service.refresh();
    if (_service.entries.isEmpty) unawaited(_service.check());
  }

  @override
  void dispose() {
    _service.removeListener(_heard);
    super.dispose();
  }

  void _heard() {
    if (mounted) setState(() {});
  }

  bool get _runtimeMissing => _service.runtimeInstalled == null;
  bool get _packMissing => !hasSpeechPack(_service);

  /// The speech packs on offer, the largest first.
  List<Addon> get _packs => [
        for (final entry in _service.entries)
          if (entry.kind == AddonKind.model && entry.task == 'speech') entry,
      ]..sort((a, b) => b.sizeBytes.compareTo(a.sizeBytes));

  Addon? get _picked {
    final packs = _packs;
    for (final pack in packs) {
      if (pack.id == _pack) return pack;
    }
    return packs.firstOrNull;
  }

  /// What Install fetches: the runtime when it is missing, the pack picked
  /// when there is none.
  List<Addon> get _wanted => [
        if (_runtimeMissing)
          if (_service.runtimeOffered case final runtime?) runtime,
        if (_packMissing)
          if (_picked case final pack?) pack,
      ];

  bool get _ready => !_runtimeMissing && !_packMissing;

  Future<void> _install() async {
    setState(() => _installing = true);
    for (final addon in _wanted) {
      await _service.install(addon.id);
      if (!mounted) return;
      if (_service.stage != AddonStage.done) {
        setState(() => _installing = false);
        return;
      }
    }
    if (!mounted) return;
    setState(() => _installing = false);
    if (_ready) widget.onClose(true);
  }

  void _cancel() {
    if (_service.busy) _service.cancel();
    widget.onClose(false);
  }

  String _failure(AddonFailure why) => switch (why) {
        AddonFailure.network => l10n.settingsAddonsFailedNetwork,
        AddonFailure.catalogue => l10n.settingsAddonsFailedCatalogue,
        AddonFailure.manifest => l10n.settingsAddonsFailedManifest,
        AddonFailure.unsupported => l10n.settingsAddonsFailedUnsupported,
        AddonFailure.incomplete => l10n.settingsAddonsFailedIncomplete,
        AddonFailure.checksum => l10n.settingsAddonsFailedChecksum,
        AddonFailure.busy => l10n.settingsAddonsFailedBusy,
        AddonFailure.refused => l10n.settingsAddonsFailedRefused,
        AddonFailure.removeRefused => l10n.settingsAddonsFailedRemove,
      };

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final checking = _service.stage == AddonStage.checking;
    final runtime = _service.runtimeOffered;
    final packs = _packs;
    final picked = _picked;
    final wanted = _wanted;
    final whole = wanted.fold<int>(0, (sum, a) => sum + a.sizeBytes);
    final failure = _service.failure;
    final busy = _installing || _service.busy;
    final working = _service.working;
    Addon? coming;
    for (final addon in wanted) {
      if (addon.id == working) coming = addon;
    }

    Widget size(int bytes) => Text(byteSize(bytes), style: dialogMono(t));
    final none = !checking &&
        ((_packMissing && packs.isEmpty) ||
            (_runtimeMissing && runtime == null));

    final rows = <Widget>[
      if (_runtimeMissing && runtime != null)
        dialogRow(
          t,
          runtime.name.isNotEmpty ? runtime.name : l10n.settingsAddonsRuntime,
          Align(
            alignment: AlignmentDirectional.centerEnd,
            child: size(runtime.sizeBytes),
          ),
          labelColumn: 220,
          key: const ValueKey('speech-install-runtime'),
        ),
      if (_packMissing)
        for (final pack in packs)
          _packRow(t, pack, selected: pack.id == picked?.id, enabled: !busy),
      if (none)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 6),
          child: Text(l10n.speechInstallNoneOffered,
              key: const ValueKey('speech-install-none'), style: t.body),
        ),
      if (checking)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 6),
          child: Text(l10n.speechInstallChecking, style: t.small),
        ),
      if (coming != null) ...[
        const SizedBox(height: 10),
        Row(
          children: [
            Expanded(
              child: Text(
                coming.name.isEmpty ? coming.id : coming.name,
                overflow: TextOverflow.ellipsis,
                style: t.small,
              ),
            ),
            const SizedBox(width: 8),
            Text('${(_service.fraction * 100).floor()}%', style: dialogMono(t)),
          ],
        ),
        const SizedBox(height: 4),
        HouseProgressBar(
            key: const ValueKey('speech-install-progress'),
            fraction: _service.fraction),
      ],
    ];

    return dialogEnter(
      onEnter: () {
        if (!busy && wanted.isNotEmpty) unawaited(_install());
      },
      child: DialogFrame(
        width: 420,
        children: [
          dialogTitleBar(t,
              title: l10n.speechInstallTitle,
              onClose: _cancel,
              keyPrefix: 'speech-install'),
          Padding(
            padding: dialogBodyPadding,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: rows,
            ),
          ),
          dialogFooter(
            t,
            keyPrefix: 'speech-install',
            summary: failure != null
                ? _failure(failure)
                : whole > 0
                    ? byteSize(whole)
                    : '',
            actions: [
              dialogPlainAction(t,
                  key: const ValueKey('speech-install-cancel'),
                  label: none ? l10n.close : l10n.cancel,
                  onPressed: _cancel),
              // Nothing to install, nothing to press.
              if (!none)
                dialogFilledAction(t,
                    key: const ValueKey('speech-install-go'),
                    label: l10n.settingsAddonsInstall,
                    onPressed: busy ||
                            wanted.isEmpty ||
                            (_packMissing && picked == null)
                        ? null
                        : () => unawaited(_install())),
            ],
          ),
        ],
      ),
    );
  }

  /// One speech pack to choose: its name and what it weighs, and the terms
  /// its training carries when the catalogue gives any.
  Widget _packRow(LumitTheme t, Addon pack,
          {required bool selected, required bool enabled}) =>
      HoverMark(
        key: ValueKey<String>('speech-install-pack-${pack.id}'),
        cursor: enabled ? SystemMouseCursors.click : MouseCursor.defer,
        builder: (context, lit) => GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: enabled ? () => setState(() => _pack = pack.id) : null,
          child: Padding(
            padding: const EdgeInsets.symmetric(vertical: 4),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                HouseRadio(
                  selected: selected,
                  enabled: enabled,
                  onChanged: () => setState(() => _pack = pack.id),
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(pack.name.isEmpty ? pack.id : pack.name,
                          style: t.body.copyWith(
                              color: Color.lerp(
                                  t.body.color, t.textPrimary, lit))),
                      if (pack.notes.isNotEmpty)
                        Text(pack.notes, style: t.small),
                    ],
                  ),
                ),
                const SizedBox(width: 8),
                Text(byteSize(pack.sizeBytes), style: dialogMono(t)),
              ],
            ),
          ),
        ),
      );
}

// --- The words a running transcription is shown in -----------------------------

/// The name of the footage item [id] in [project], or empty when it has gone.
String footageItemName(ProjectReference? project, String id) {
  String? walk(List<ItemReference> items) {
    for (final item in items) {
      switch (item) {
        case ItemReference_Footage(:final field0)
            when field0.internalid.toString() == id:
          return item.name();
        case ItemReference_Folder(:final field0):
          final found = walk(field0.getChildren());
          if (found != null) return found;
        case _:
          break;
      }
    }
    return null;
  }

  try {
    return walk(project?.getItems() ?? const []) ?? '';
  } catch (_) {
    return '';
  }
}

/// What a transcription is doing, in a few words: [name] is the file.
String transcriptionStep(BridgeTranscribeEvent event, String name) =>
    switch (event.stage) {
      BridgeTranscribeStage.loading => l10n.transcribeLoading,
      BridgeTranscribeStage.speakers => l10n.transcribeSpeakers(name),
      _ => l10n.transcribeListening(name),
    };

/// How long it should take from here, once that is known.
String? transcriptionTimeLeft(BridgeTranscribeEvent event) {
  final seconds = event.remaining;
  if (seconds <= 0 || event.fraction <= 0) return null;
  final minutes = (seconds / 60).round();
  if (minutes < 1) return l10n.exportLeftUnderMinute;
  return minutes < 120
      ? l10n.exportLeftMinutes('$minutes')
      : l10n.exportLeftHours('${(minutes / 60).round()}');
}

/// How many times faster than it plays the sound is being heard, once known.
String? transcriptionSpeed(BridgeTranscribeEvent event) => event.speed > 0
    ? l10n.transcribeSpeed(event.speed.toStringAsFixed(1))
    : null;
