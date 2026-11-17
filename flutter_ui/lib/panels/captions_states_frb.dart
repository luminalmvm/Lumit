// The Captions panel when it has no captions to list: nothing made yet, or
// the speech still being heard. And the line over the list while it is.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/transcript.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/create_captions_dialog_frb.dart';
import '../shell/speech_install_dialog_frb.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'expressions_panel_frb.dart' show panelBar;
import 'placeholder.dart';

/// No caption layer yet: make one from the selected layer's speech, when it
/// has some, or from a file.
Widget captionsEmpty(BuildContext context, LumitUiState ui,
        {required bool canCreate}) =>
    PlaceholderPanel(
      key: const ValueKey('captions-empty'),
      icon: LumitIcon.text,
      title: l10n.panelCaptions,
      hint: '',
      action: Wrap(
        spacing: 8,
        runSpacing: 6,
        alignment: WrapAlignment.center,
        children: [
          HouseButton(
            key: const ValueKey('captions-empty-create'),
            small: true,
            onPressed: canCreate
                ? () {
                    final layer = ui.selectedLayer.value;
                    if (layer != null) {
                      unawaited(createCaptionsFrb(context, layer));
                    }
                  }
                : null,
            child: Text(l10n.captionsCreate),
          ),
          HouseButton(
            key: const ValueKey('captions-empty-import'),
            small: true,
            onPressed: () => unawaited(importCaptionsFrb(context)),
            child: Text(l10n.captionsImportEllipsis),
          ),
        ],
      ),
    );

/// What a transcription is doing, in the words the status line uses, then
/// how fast and how much longer.
(String, String) _says(LumitState app, BridgeTranscribeEvent event) {
  final step =
      transcriptionStep(event, footageItemName(app.project, event.item));
  final rest = [
    if (transcriptionSpeed(event) case final speed?) speed,
    if (transcriptionTimeLeft(event) case final left?) left,
  ].join(' · ');
  return (step, rest);
}

/// The speech being heard, with nothing to list yet.
Widget captionsRunning(LumitState app, BridgeTranscribeEvent event) {
  final (step, rest) = _says(app, event);
  return PlaceholderPanel(
    key: const ValueKey('captions-running'),
    icon: LumitIcon.text,
    title: step,
    hint: rest,
    action: SizedBox(
      width: 180,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          // The whole width, or the track shrinks to the fill and always
          // reads full.
          SizedBox(
            width: double.infinity,
            child: HouseProgressBar(fraction: event.fraction),
          ),
          const SizedBox(height: 8),
          HouseButton(
            key: const ValueKey('captions-running-cancel'),
            small: true,
            onPressed: app.transcription.cancel,
            child: Text(l10n.cancel),
          ),
        ],
      ),
    ),
  );
}

/// The speech being heard while captions are listed: one line over them.
Widget captionsStrip(
    LumitTheme t, LumitState app, BridgeTranscribeEvent event) {
  final (step, rest) = _says(app, event);
  return Container(
    key: const ValueKey('captions-strip'),
    height: 24,
    padding: const EdgeInsets.symmetric(horizontal: 8),
    decoration: BoxDecoration(
      border: Border(bottom: BorderSide(color: t.hairline)),
    ),
    child: Row(
      children: [
        Expanded(
          child: Text(rest.isEmpty ? step : '$step · $rest',
              maxLines: 1, overflow: TextOverflow.ellipsis, style: t.small),
        ),
        const SizedBox(width: 8),
        SizedBox(width: 64, child: HouseProgressBar(fraction: event.fraction)),
        const SizedBox(width: 4),
        HouseButton(
          small: true,
          frameless: true,
          onPressed: app.transcription.cancel,
          child: Text(l10n.cancel, style: t.small),
        ),
      ],
    ),
  );
}

/// A mark on the panel's bar, greyed when it can do nothing, with its chord
/// in the tip when it has one. [onPressed] is handed the mark's own context,
/// for a popover to hang from.
Widget captionsMark(LumitTheme t, String key, String tip, String icon,
        void Function(BuildContext)? onPressed,
        {String? action, bool active = false}) =>
    LumitTooltip(
      message: tip,
      action: action,
      child: Builder(
        builder: (context) => HouseButton(
          key: ValueKey<String>(key),
          small: true,
          frameless: !active,
          active: active,
          padding: const EdgeInsets.all(1),
          onPressed: onPressed == null ? null : () => onPressed(context),
          child: glyph.LumitIcon(icon,
              size: 16,
              colour: onPressed == null
                  ? t.textDisabled
                  : active
                      ? t.textPrimary
                      : t.textMuted),
        ),
      ),
    );

/// The row under the find field that replaces what it found: in the caption
/// stepped to, or in every caption.
Widget captionsReplaceBar(
  LumitTheme t, {
  required TextEditingController replaceWith,
  required bool enabled,
  required VoidCallback onReplace,
  required VoidCallback onReplaceAll,
  required bool matchCase,
  required bool wholeWord,
  required VoidCallback onMatchCase,
  required VoidCallback onWholeWord,
}) =>
    Container(
      height: 26,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border(top: BorderSide(color: t.hairline)),
      ),
      child: panelBar(
        least: 260,
        child: Row(
          children: [
            Expanded(
              child: HouseTextField(
                key: const ValueKey('captions-replace-with'),
                controller: replaceWith,
                hint: l10n.captionsReplaceWith,
                width: double.infinity,
                onSubmitted: (_) => onReplace(),
              ),
            ),
            const SizedBox(width: 6),
            HouseButton(
              key: const ValueKey('captions-replace-one'),
              small: true,
              onPressed: enabled ? onReplace : null,
              child: Text(l10n.captionsReplace),
            ),
            const SizedBox(width: 4),
            HouseButton(
              key: const ValueKey('captions-replace-all'),
              small: true,
              onPressed: enabled ? onReplaceAll : null,
              child: Text(l10n.captionsReplaceAll),
            ),
            const SizedBox(width: 6),
            captionsMark(t, 'captions-match-case', l10n.captionsMatchCase,
                LumitIcons.allCaps, (_) => onMatchCase(),
                active: matchCase),
            captionsMark(t, 'captions-whole-word', l10n.captionsWholeWord,
                LumitIcons.wholeWord, (_) => onWholeWord(),
                active: wholeWord),
          ],
        ),
      ),
    );
