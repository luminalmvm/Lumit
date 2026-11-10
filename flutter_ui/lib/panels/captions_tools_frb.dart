// What the Captions panel's bar opens: the style grid, the limits, the
// subtitle files and the rest of what a caption layer can be asked.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/captions.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/transcript.dart';

import '../l10n/strings.dart';
import '../shell/create_captions_dialog_frb.dart'
    show importCaptionsFrb, speakerLabelsName;
import '../shell/dialog_frame.dart';
import '../shell/menu_bar_frb.dart' show MenuEntry;
import '../shell/theme_name_dialog.dart';
import '../state/file_dialogs.dart';
import '../state/text_documents.dart';
import '../widgets/controls.dart';
import 'caption_styles.dart';
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'timeline_extras_frb.dart' show showMenuAt;

/// The window point just under the control [context] belongs to, where a
/// popover hangs from it.
Offset popupBelow(BuildContext context) {
  final box = context.findRenderObject() as RenderBox?;
  if (box == null || !box.hasSize) return Offset.zero;
  return box.localToGlobal(Offset(0, box.size.height + 2));
}

/// New limits for caption [layer], and a box as tall as its lines. With
/// [rebuild] every caption is cut again from its words. One undo step.
void setCaptionLimits(LumitState app, LayerReference layer,
    BridgeCaptionLimits was, BridgeCaptionLimits limits,
    {bool rebuild = false}) {
  asOneUndoStep(app.project, () {
    layer.setCaptionLimits(limits: limits, rebuild: rebuild);
    if (limits.lines == was.lines) return;
    final document = layer.getText();
    if (document == null) return;
    final leading = document.style.leading ?? document.size * 1.2;
    layer.setText(
        document: document.copyWith(
            paragraph: document.paragraph
                .copyWith(boxHeight: leading * limits.lines)));
  }, name: 'Edit captions');
}

/// The style grid: every look as a tile, the one the layer wears outlined.
/// A pick restyles the layer at once and the grid stays up, so looks can be
/// tried one after another.
Future<void> showCaptionStylePopover(
  BuildContext context, {
  required LumitState app,
  required LumitUiState ui,
  required CompositionReference comp,
  required LayerReference layer,
  required VoidCallback onEdited,
}) {
  final height = comp.getSettings().height;
  String? worn() {
    try {
      final document = layer.getText();
      return document == null
          ? null
          : captionLookKeyOf(document, captionLookChoices(ui.workspace),
              height: height);
    } catch (_) {
      return null;
    }
  }

  var wearing = worn();
  return showLumitPopup<void>(
    context: context,
    position: popupBelow(context),
    builder: (close) => StatefulBuilder(
      builder: (context, setState) {
        final t = ThemeScope.of(context).theme;
        void pick(CaptionLookChoice choice) {
          try {
            applyCaptionLook(app.project, comp, layer, choice);
            onEdited();
          } catch (_) {
            app.postNotice(l10n.captionsEditFailed, error: true);
          }
          setState(() => wearing = worn());
        }

        Future<void> save() async {
          var name = l10n.captionsStyleName(1);
          final taken = {
            for (final saved in savedCaptionStyles(ui.workspace)) saved.name
          };
          for (var n = 2; taken.contains(name); n++) {
            name = l10n.captionsStyleName(n);
          }
          final chosen = await askThemeName(context,
              title: l10n.captionsSaveStyleTitle,
              suggested: name,
              confirm: l10n.save);
          if (chosen == null || chosen.trim().isEmpty) return;
          saveCaptionStyle(ui.workspace, chosen.trim(), layer);
          setState(() => wearing = worn());
        }

        void menu(SavedCaptionStyle saved, Offset at) => showMenuAt<void>(
              context: context,
              position: at,
              rows: (closeMenu) => [
                MenuRow(
                  key: const ValueKey('caption-style-rename'),
                  onPressed: () async {
                    closeMenu(null);
                    final to = await askThemeName(context,
                        title: l10n.rename,
                        suggested: saved.name,
                        confirm: l10n.rename);
                    if (to == null || to.trim().isEmpty) return;
                    renameCaptionStyle(ui.workspace, saved.name, to.trim());
                    setState(() => wearing = worn());
                  },
                  child: Text(l10n.renameEllipsis),
                ),
                MenuRow(
                  key: const ValueKey('caption-style-delete'),
                  onPressed: () {
                    closeMenu(null);
                    deleteCaptionStyle(ui.workspace, saved.name);
                    setState(() => wearing = worn());
                  },
                  child: Text(l10n.delete),
                ),
              ],
            );

        return FloatSurface(
          width: 352,
          child: Padding(
            padding: const EdgeInsets.fromLTRB(10, 10, 10, 6),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Wrap(
                  spacing: 8,
                  runSpacing: 8,
                  children: [
                    for (final choice in captionLookChoices(ui.workspace))
                      CaptionLookTile(
                        key: ValueKey<String>('caption-style-${choice.key}'),
                        choice: choice,
                        width: 76,
                        selected: choice.key == wearing,
                        onPressed: () => pick(choice),
                        onMenu: choice.saved == null
                            ? null
                            : (at) => menu(choice.saved!, at),
                      ),
                  ],
                ),
                const SizedBox(height: 8),
                Container(height: 1, color: t.hairline),
                const SizedBox(height: 6),
                Align(
                  alignment: AlignmentDirectional.centerStart,
                  child: HouseButton(
                    key: const ValueKey('caption-style-save'),
                    small: true,
                    frameless: true,
                    onPressed: () => unawaited(save()),
                    child: Text(l10n.captionsSaveStyle, style: t.small),
                  ),
                ),
              ],
            ),
          ),
        );
      },
    ),
  );
}

/// The limits a caption layer's captions are cut to, and the button that cuts
/// them again. A change is kept at once; the captions follow it on Make again.
Future<void> showCaptionLimitsPopover(
  BuildContext context, {
  required LumitState app,
  required LayerReference layer,
  required BridgeCaptionLimits limits,
  required VoidCallback onEdited,
}) {
  var held = limits;
  return showLumitPopup<void>(
    context: context,
    position: popupBelow(context),
    builder: (close) => StatefulBuilder(builder: (context, setState) {
      final t = ThemeScope.of(context).theme;
      void keep(BridgeCaptionLimits next, {bool rebuild = false}) {
        try {
          setCaptionLimits(app, layer, held, next, rebuild: rebuild);
          onEdited();
        } catch (_) {
          app.postNotice(l10n.captionsEditFailed, error: true);
        }
        setState(() => held = next);
      }

      Widget row(String label, Widget control) =>
          dialogRow(t, label, control, labelColumn: 128, minHeight: 26);
      Widget number(String id, num value, num min, num max,
              ValueChanged<num> onChanged,
              {int decimals = 0, String? suffix, String? zero}) =>
          Align(
            alignment: AlignmentDirectional.centerStart,
            child: SizedBox(
              width: 76,
              height: dialogControlHeight,
              child: DragValueField(
                key: ValueKey<String>(id),
                value: value,
                min: min,
                max: max,
                decimals: decimals,
                speed: decimals > 0 ? 0.1 : 1,
                suffix: suffix == null ? null : ' $suffix',
                zeroLabel: zero,
                onChanged: onChanged,
              ),
            ),
          );
      const words = [0, 1, 2, 3, 4, 5, 6, 8, 10];
      final seconds = l10n.unitSymbolSeconds;
      return FloatSurface(
        width: 260,
        child: Padding(
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 8),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              row(
                l10n.captionsLines,
                dialogDropdown<int>(t,
                    id: 'caption-limits-lines',
                    value: held.lines.clamp(1, 3),
                    options: const [1, 2, 3],
                    label: (n) => '$n',
                    onChanged: (n) => keep(held.copyWith(lines: n))),
              ),
              row(
                l10n.captionsCharsPerLine,
                number('caption-limits-chars', held.charsPerLine, 8, 80,
                    (v) => keep(held.copyWith(charsPerLine: v.round()))),
              ),
              row(
                l10n.captionsWords,
                dialogDropdown<int>(t,
                    id: 'caption-limits-words',
                    value: words.contains(held.words) ? held.words : 0,
                    options: words,
                    label: (n) => n == 0 ? l10n.captionsWordsAny : '$n',
                    onChanged: (n) => keep(held.copyWith(words: n))),
              ),
              row(
                l10n.captionsGap,
                number('caption-limits-gap', held.gapFrames, 0, 60,
                    (v) => keep(held.copyWith(gapFrames: v.round())),
                    suffix: l10n.unitSymbolFramesShort),
              ),
              row(
                l10n.captionsShortest,
                number(
                    'caption-limits-shortest',
                    held.minDurationMs / 1000,
                    0.1,
                    10,
                    (v) =>
                        keep(held.copyWith(minDurationMs: (v * 1000).round())),
                    decimals: 1,
                    suffix: seconds),
              ),
              row(
                l10n.captionsLongest,
                number(
                    'caption-limits-longest',
                    held.maxDurationMs / 1000,
                    1,
                    20,
                    (v) =>
                        keep(held.copyWith(maxDurationMs: (v * 1000).round())),
                    decimals: 1,
                    suffix: seconds),
              ),
              row(
                l10n.captionsReadingSpeed,
                number('caption-limits-reading', held.readingSpeed, 0, 40,
                    (v) => keep(held.copyWith(readingSpeed: v.round())),
                    suffix: l10n.captionsUnitCps, zero: l10n.captionsOff),
              ),
              const SizedBox(height: 6),
              Align(
                alignment: AlignmentDirectional.centerEnd,
                child: HouseButton(
                  key: const ValueKey('caption-limits-again'),
                  small: true,
                  onPressed: () => keep(held, rebuild: true),
                  child: Text(l10n.captionsMakeAgain),
                ),
              ),
            ],
          ),
        ),
      );
    }),
  );
}

/// Where captions sit, moved from where the layer puts them, in the layer's
/// own pixels. The fields read the first of [cues]; a change moves every one
/// of them on that axis, kept when the drag lets go.
Future<void> showCaptionPositionPopover(
  BuildContext context, {
  required Offset position,
  required LumitState app,
  required LayerReference layer,
  required List<BridgeCue> cues,
  required VoidCallback onEdited,
}) {
  final moves = {for (final cue in cues) cue.index: (cue.offsetX, cue.offsetY)};
  var shown = (cues.first.offsetX, cues.first.offsetY);
  return showLumitPopup<void>(
    context: context,
    position: position,
    builder: (close) => StatefulBuilder(builder: (context, setState) {
      final t = ThemeScope.of(context).theme;
      void move(bool y, double to, {required bool keep}) {
        setState(() => shown = y ? (shown.$1, to) : (to, shown.$2));
        if (!keep) return;
        try {
          asOneUndoStep(app.project, () {
            for (final cue in moves.keys.toList()) {
              final (x0, y0) = moves[cue]!;
              final at = y ? (x0, to) : (to, y0);
              layer.setCueOffset(cue: cue, x: at.$1, y: at.$2);
              moves[cue] = at;
            }
          }, name: 'Edit captions');
          onEdited();
        } catch (_) {
          app.postNotice(l10n.captionsEditFailed, error: true);
        }
      }

      Widget field(String label, bool y) => Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Text(label, style: t.small.copyWith(color: t.textMuted)),
                const SizedBox(height: 2),
                DragValueField(
                  key: ValueKey<String>('caption-position-${y ? 'y' : 'x'}'),
                  value: y ? shown.$2 : shown.$1,
                  min: -10000,
                  max: 10000,
                  suffix: ' ${l10n.unitSymbolPx}',
                  resetTo: 0,
                  onChangeLive: (v) => move(y, v.toDouble(), keep: false),
                  onChangeEnd: (v) => move(y, v.toDouble(), keep: true),
                  onChanged: (v) => move(y, v.toDouble(), keep: true),
                ),
              ],
            ),
          );
      return FloatSurface(
        width: 184,
        child: Padding(
          padding: const EdgeInsets.all(8),
          child: Row(children: [
            field(l10n.captionsPositionX, false),
            const SizedBox(width: 6),
            field(l10n.captionsPositionY, true),
          ]),
        ),
      );
    }),
  );
}

/// Export ▸: the captions as a subtitle file, on the composition's clock.
List<MenuEntry> captionExportRows(
    LumitState app, LayerReference layer, String name) {
  Future<void> write(BridgeCaptionFormat format, String extension) async {
    final path = await pickCaptionsSaveLocation(name, extension);
    if (path == null) return;
    try {
      await layer.exportCaptions(format: format, path: path);
      await handOver(path);
      app.postNotice(l10n.captionsExported(path.split(RegExp(r'[\\/]')).last));
    } catch (_) {
      app.postNotice(l10n.captionsExportFailed, error: true);
    }
  }

  return [
    MenuEntry(l10n.captionsFormatSrt,
        () => unawaited(write(BridgeCaptionFormat.srt, 'srt'))),
    MenuEntry(l10n.captionsFormatVtt,
        () => unawaited(write(BridgeCaptionFormat.vtt, 'vtt'))),
    MenuEntry(l10n.captionsFormatTtml,
        () => unawaited(write(BridgeCaptionFormat.ttml, 'ttml'))),
    MenuEntry(l10n.captionsFormatText,
        () => unawaited(write(BridgeCaptionFormat.txt, 'txt'))),
  ];
}

/// The bar's "…": import, the captions' language, how speakers show, and
/// cutting the captions again.
List<MenuEntry> captionMoreRows(BuildContext context, LumitState app,
    LayerReference layer, BridgeCaptions captions, VoidCallback onEdited) {
  void edit(void Function() run) {
    try {
      run();
      onEdited();
    } catch (_) {
      app.postNotice(l10n.captionsEditFailed, error: true);
    }
  }

  var languages = const <BridgeLanguage>[];
  try {
    languages = transcriptionLanguages();
  } catch (_) {}
  return [
    MenuEntry(l10n.captionsImportEllipsis,
        () => unawaited(importCaptionsFrb(context))),
    MenuEntry.divider(),
    if (languages.isNotEmpty)
      MenuEntry.submenu(l10n.captionsLanguage, [
        for (final language in languages)
          MenuEntry(
            language.name,
            () => edit(() => layer.setCaptionLanguage(language: language.code)),
            checked: language.code == captions.language,
          ),
      ]),
    MenuEntry.submenu(l10n.captionsSpeakerNames, [
      for (final labels in BridgeSpeakerLabels.values)
        MenuEntry(
          speakerLabelsName(labels),
          () => edit(() => layer.setCaptionLimits(
              limits: captions.limits.copyWith(speakerLabels: labels),
              rebuild: true)),
          checked: labels == captions.limits.speakerLabels,
        ),
    ]),
    MenuEntry.divider(),
    MenuEntry(
      l10n.captionsMakeAgain,
      () => edit(
          () => layer.setCaptionLimits(limits: captions.limits, rebuild: true)),
    ),
  ];
}
