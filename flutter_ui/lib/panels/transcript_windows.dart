// The Transcript panel's windows: Export transcript, Assemble from script,
// the speakers' names, and the lines a script had that nobody said.
//
// Each asks its questions and answers with a plain record; the panel turns
// the answer into the engine's call.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/cut/transcript.dart'
    show BridgeTranscriptFormat;

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/dialog_frame.dart';
import '../shell/question_window.dart';
import '../widgets/controls.dart';
import 'transcript_text.dart';

/// Which words a transcript file holds.
enum TranscriptRange { whole, selection, workArea }

class TranscriptExportChoice {
  final BridgeTranscriptFormat format;
  final bool speakers;
  final bool timecodes;
  final bool fillers;
  final TranscriptRange range;
  final int charsPerLine;
  final int lines;

  const TranscriptExportChoice({
    required this.format,
    required this.speakers,
    required this.timecodes,
    required this.fillers,
    required this.range,
    required this.charsPerLine,
    required this.lines,
  });
}

String _formatName(BridgeTranscriptFormat f) => switch (f) {
      BridgeTranscriptFormat.text => l10n.transcriptFormatText,
      BridgeTranscriptFormat.word => l10n.transcriptFormatWord,
      BridgeTranscriptFormat.richText => l10n.transcriptFormatRichText,
      BridgeTranscriptFormat.markdown => l10n.transcriptFormatMarkdown,
      BridgeTranscriptFormat.html => l10n.transcriptFormatHtml,
      BridgeTranscriptFormat.csv => l10n.transcriptFormatCsv,
      BridgeTranscriptFormat.subRip => l10n.transcriptFormatSubRip,
      BridgeTranscriptFormat.webVtt => l10n.transcriptFormatWebVtt,
    };

/// The file's extension for each kind.
String transcriptExtension(BridgeTranscriptFormat f) => switch (f) {
      BridgeTranscriptFormat.text => 'txt',
      BridgeTranscriptFormat.word => 'docx',
      BridgeTranscriptFormat.richText => 'rtf',
      BridgeTranscriptFormat.markdown => 'md',
      BridgeTranscriptFormat.html => 'html',
      BridgeTranscriptFormat.csv => 'csv',
      BridgeTranscriptFormat.subRip => 'srt',
      BridgeTranscriptFormat.webVtt => 'vtt',
    };

const double _labelColumn = 110;

/// Ask how to write the transcript. Null when the window was closed.
Future<TranscriptExportChoice?> askTranscriptExport(
  BuildContext context, {
  required bool hasSelection,
  required bool hasWorkArea,
}) {
  var format = BridgeTranscriptFormat.text;
  var speakers = true, timecodes = true, fillers = false;
  var range = TranscriptRange.whole;
  var charsPerLine = 42, lines = 2;
  return showLumitModal<TranscriptExportChoice>(
    context: context,
    id: 'transcript-export',
    minSize: const Size(360, 240),
    builder: (close) => StatefulBuilder(builder: (context, set) {
      final t = ThemeScope.of(context).theme;
      final subtitles = format == BridgeTranscriptFormat.subRip ||
          format == BridgeTranscriptFormat.webVtt;
      void go() => close(TranscriptExportChoice(
            format: format,
            speakers: speakers,
            timecodes: timecodes,
            fillers: fillers,
            range: range,
            charsPerLine: charsPerLine,
            lines: lines,
          ));
      Widget tick(String key, String label, bool on, ValueChanged<bool> flip) =>
          Padding(
            padding: const EdgeInsets.symmetric(vertical: 3),
            child: HouseCheckbox(
              key: ValueKey<String>(key),
              value: on,
              label: label,
              onChanged: (v) => set(() => flip(v)),
            ),
          );
      return dialogEnter(
        onEnter: go,
        child: DialogFrame(
          width: 380,
          children: [
            dialogTitleBar(t,
                title: l10n.transcriptExportTitle,
                onClose: () => close(null),
                keyPrefix: 'transcript-export'),
            Padding(
              padding: dialogBodyPadding,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                mainAxisSize: MainAxisSize.min,
                children: [
                  dialogRow(
                    t,
                    l10n.exportFormat,
                    BareDropdown<BridgeTranscriptFormat>(
                      key: const ValueKey('transcript-export-format'),
                      value: format,
                      options: BridgeTranscriptFormat.values,
                      label: _formatName,
                      onChanged: (f) => set(() => format = f),
                    ),
                    labelColumn: _labelColumn,
                  ),
                  dialogRow(
                    t,
                    l10n.transcriptExportRange,
                    BareDropdown<TranscriptRange>(
                      key: const ValueKey('transcript-export-range'),
                      value: range,
                      options: TranscriptRange.values,
                      label: (r) => switch (r) {
                        TranscriptRange.whole => l10n.transcriptRangeWhole,
                        TranscriptRange.selection =>
                          l10n.transcriptRangeSelection,
                        TranscriptRange.workArea =>
                          l10n.transcriptRangeWorkArea,
                      },
                      disabledReason: (r) => switch (r) {
                        TranscriptRange.selection when !hasSelection => '',
                        TranscriptRange.workArea when !hasWorkArea => '',
                        _ => null,
                      },
                      onChanged: (r) => set(() => range = r),
                    ),
                    labelColumn: _labelColumn,
                  ),
                  if (subtitles) ...[
                    dialogRow(
                      t,
                      l10n.transcriptCharsPerLine,
                      SizedBox(
                        width: 72,
                        child: DragValueField(
                          key: const ValueKey('transcript-export-chars'),
                          value: charsPerLine,
                          min: 16,
                          max: 120,
                          onChanged: (v) =>
                              set(() => charsPerLine = v.round()),
                        ),
                      ),
                      labelColumn: _labelColumn,
                    ),
                    dialogRow(
                      t,
                      l10n.transcriptSubtitleLines,
                      SizedBox(
                        width: 72,
                        child: DragValueField(
                          key: const ValueKey('transcript-export-lines'),
                          value: lines,
                          min: 1,
                          max: 4,
                          onChanged: (v) => set(() => lines = v.round()),
                        ),
                      ),
                      labelColumn: _labelColumn,
                    ),
                  ],
                  const SizedBox(height: 6),
                  tick('transcript-export-speakers',
                      l10n.transcriptExportSpeakerNames, speakers,
                      (v) => speakers = v),
                  tick('transcript-export-timecodes',
                      l10n.transcriptExportTimecodes, timecodes,
                      (v) => timecodes = v),
                  tick('transcript-export-fillers',
                      l10n.transcriptExportFillers, fillers, (v) => fillers = v),
                ],
              ),
            ),
            dialogFooter(t, keyPrefix: 'transcript-export', actions: [
              dialogPlainAction(t,
                  key: const ValueKey('transcript-export-cancel'),
                  label: l10n.cancel,
                  onPressed: () => close(null)),
              const SizedBox(width: 8),
              dialogFilledAction(t,
                  key: const ValueKey('transcript-export-go'),
                  label: l10n.transcriptExportAction,
                  onPressed: go),
            ]),
          ],
        ),
      );
    }),
  );
}

class TranscriptAssembleChoice {
  final String script;

  /// Every transcribed item, or only the Project panel's selected ones.
  final bool allFootage;

  /// A new composition, or the fronted one at the playhead.
  final bool intoNew;
  final String name;

  const TranscriptAssembleChoice({
    required this.script,
    required this.allFootage,
    required this.intoNew,
    required this.name,
  });
}

/// Ask for a script and where its lines go. [openScript] picks and reads a
/// file, answering its name and text, or null. Null when the window was
/// closed or the script was empty.
Future<TranscriptAssembleChoice?> askTranscriptAssemble(
  BuildContext context, {
  required Future<(String, String)?> Function() openScript,
  required bool hasSelectedItems,
  required bool hasComposition,
}) {
  final script = TextEditingController();
  final name = TextEditingController(text: l10n.transcriptAssembleDefaultName);
  var allFootage = true;
  var intoNew = true;
  return showLumitModal<TranscriptAssembleChoice>(
    context: context,
    id: 'transcript-assemble',
    minSize: const Size(380, 320),
    builder: (close) => StatefulBuilder(builder: (context, set) {
      final t = ThemeScope.of(context).theme;
      final ready = script.text.trim().isNotEmpty;
      void go() {
        if (!ready) return;
        close(TranscriptAssembleChoice(
          script: script.text,
          allFootage: allFootage,
          intoNew: intoNew,
          name: name.text.trim().isEmpty
              ? l10n.transcriptAssembleDefaultName
              : name.text.trim(),
        ));
      }

      return DialogFrame(
        width: 440,
        children: [
          dialogTitleBar(t,
              title: l10n.transcriptAssembleTitle,
              onClose: () => close(null),
              keyPrefix: 'transcript-assemble'),
          Padding(
            padding: dialogBodyPadding,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: [
                SizedBox(
                  height: 180,
                  child: HouseTextField(
                    key: const ValueKey('transcript-assemble-script'),
                    controller: script,
                    width: double.infinity,
                    multiline: true,
                    topAlign: true,
                    autofocus: true,
                    onSubmitted: (_) => set(() {}),
                  ),
                ),
                const SizedBox(height: 6),
                Row(children: [
                  HouseButton(
                    key: const ValueKey('transcript-assemble-open'),
                    small: true,
                    onPressed: () async {
                      final read = await openScript();
                      if (read == null) return;
                      set(() {
                        script.text = read.$2;
                        name.text = read.$1;
                      });
                    },
                    child: Text(l10n.transcriptAssembleOpen),
                  ),
                ]),
                const SizedBox(height: 8),
                dialogRow(
                  t,
                  l10n.transcriptAssembleSearch,
                  BareDropdown<bool>(
                    key: const ValueKey('transcript-assemble-search'),
                    value: allFootage,
                    options: const [true, false],
                    label: (all) => all
                        ? l10n.transcriptAssembleAllFootage
                        : l10n.transcriptAssembleSelectedItems,
                    disabledReason: (all) =>
                        !all && !hasSelectedItems ? '' : null,
                    onChanged: (v) => set(() => allFootage = v),
                  ),
                  labelColumn: _labelColumn,
                ),
                dialogRow(
                  t,
                  l10n.transcriptAssembleInto,
                  BareDropdown<bool>(
                    key: const ValueKey('transcript-assemble-into'),
                    value: intoNew,
                    options: const [true, false],
                    label: (fresh) => fresh
                        ? l10n.newComposition
                        : l10n.transcriptAssembleThisComposition,
                    disabledReason: (fresh) =>
                        !fresh && !hasComposition ? '' : null,
                    onChanged: (v) => set(() => intoNew = v),
                  ),
                  labelColumn: _labelColumn,
                ),
                if (intoNew)
                  dialogRow(
                    t,
                    l10n.name,
                    HouseTextField(
                      key: const ValueKey('transcript-assemble-name'),
                      controller: name,
                      width: double.infinity,
                      onSubmitted: (_) => go(),
                    ),
                    labelColumn: _labelColumn,
                  ),
              ],
            ),
          ),
          dialogFooter(t, keyPrefix: 'transcript-assemble', actions: [
            dialogPlainAction(t,
                key: const ValueKey('transcript-assemble-cancel'),
                label: l10n.cancel,
                onPressed: () => close(null)),
            const SizedBox(width: 8),
            // Listens to the script so the button wakes as text arrives.
            ListenableBuilder(
              listenable: script,
              builder: (context, _) => dialogFilledAction(t,
                  key: const ValueKey('transcript-assemble-go'),
                  label: l10n.transcriptAssembleAction,
                  onPressed: script.text.trim().isEmpty ? null : go),
            ),
          ]),
        ],
      );
    }),
  ).whenComplete(() {
    script.dispose();
    name.dispose();
  });
}

/// The speakers by name, each renamed in its field. [rename] is called as a
/// field is left or Enter pressed with a changed name, and [find] with a
/// speaker's place once the window has closed to show their words.
Future<void> showTranscriptSpeakers(
  BuildContext context, {
  required List<TranscriptSpeaker> speakers,
  required void Function(TranscriptSpeaker speaker, String name) rename,
  required ValueChanged<int> find,
}) {
  final fields = [
    for (final s in speakers) TextEditingController(text: s.name),
  ];
  // The names as last committed, so a second commit of one does nothing.
  final names = [for (final s in speakers) s.name];
  return showLumitModal<void>(
    context: context,
    minSize: const Size(260, 120),
    builder: (close) => Builder(builder: (context) {
      final t = ThemeScope.of(context).theme;
      void commit(int i) {
        final name = fields[i].text.trim();
        if (name.isNotEmpty && name != names[i]) {
          rename(speakers[i], name);
          names[i] = name;
        }
      }

      return QuestionWindow(
        title: l10n.transcriptSpeakersTitle,
        keyPrefix: 'transcript-speakers',
        width: 300,
        onClose: () => close(null),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            for (var i = 0; i < speakers.length; i++)
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 3),
                child: Row(children: [
                  Container(
                    width: 8,
                    height: 8,
                    decoration: BoxDecoration(
                        color: t.labelColour(i + 1), shape: BoxShape.circle),
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: HouseTextField(
                      key: ValueKey<String>('transcript-speaker-$i'),
                      controller: fields[i],
                      width: double.infinity,
                      submitOnLostFocus: true,
                      onSubmitted: (_) => commit(i),
                    ),
                  ),
                  const SizedBox(width: 4),
                  LumitTooltip(
                    message: l10n.transcriptFind,
                    child: HouseButton(
                      key: ValueKey<String>('transcript-speaker-find-$i'),
                      small: true,
                      frameless: true,
                      padding: const EdgeInsets.symmetric(horizontal: 4),
                      onPressed: () {
                        close(null);
                        find(i);
                      },
                      child: glyph.LumitIcon(LumitIcons.search,
                          size: 12, colour: t.textSecondary),
                    ),
                  ),
                ]),
              ),
          ],
        ),
        actions: [
          questionFilled('transcript-speakers-done', l10n.done, () {
            for (var i = 0; i < speakers.length; i++) {
              commit(i);
            }
            close(null);
          }),
        ],
      );
    }),
  ).whenComplete(() {
    for (final f in fields) {
      f.dispose();
    }
  });
}

/// The script's lines that were found in no footage.
Future<void> showLinesNotFound(BuildContext context, List<String> lines) =>
    showLumitModal<void>(
      context: context,
      minSize: const Size(280, 120),
      builder: (close) => Builder(builder: (context) {
        final t = ThemeScope.of(context).theme;
        return QuestionWindow(
          title: l10n.transcriptLinesNotFound,
          keyPrefix: 'transcript-missing',
          onClose: () => close(null),
          content: ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: 240),
            child: SingleChildScrollView(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  for (final line in lines)
                    Padding(
                      padding: const EdgeInsets.symmetric(vertical: 2),
                      child: Text(line, style: t.body),
                    ),
                ],
              ),
            ),
          ),
          actions: [
            questionFilled(
                'transcript-missing-close', l10n.close, () => close(null)),
          ],
        );
      }),
    );
