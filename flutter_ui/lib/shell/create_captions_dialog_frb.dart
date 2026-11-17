// Create captions (Layer ▸ Audio ▸ Create captions…) and File ▸ Import ▸
// Captions…: the two ways a caption layer is made.
//
// Create asks how the captions should read, closes, hears whatever the layer
// plays that has not been heard yet (the status line and the Captions panel
// show it going), then makes the caption layer at the top of the stack and
// fronts the Captions panel on it. The answers are kept for next time.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/captions.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/streams.dart';
import 'package:lumit_flutter/src/rust/api/transcript.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../panels/audio_streams_menu_frb.dart' show audioStreamLabel;
import '../panels/caption_layers.dart';
import '../panels/caption_styles.dart';
import '../state/file_dialogs.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'speech_install_dialog_frb.dart';

/// How the captions should read, and how the speech should be heard, as the
/// dialogue keeps them between uses.
class CaptionSettings {
  /// A language code, or empty to have it heard.
  final String language;

  /// Nought tells the voices apart, one is one speaker, more is that many.
  final int speakers;
  final String style;
  final BridgeCaptionLimits limits;
  final bool fillers;
  final String vocabulary;
  final bool translate;

  const CaptionSettings({
    required this.language,
    required this.speakers,
    required this.style,
    required this.limits,
    required this.fillers,
    required this.vocabulary,
    required this.translate,
  });

  /// The last answers, or the composition's usual ones for [language].
  factory CaptionSettings.remembered(
      Workspace workspace, CompositionReference comp) {
    final j = workspace.captionSettings;
    final language = j['language'] is String ? j['language'] as String : '';
    var usual = _usual(comp, language);
    int number(String key, int fallback) =>
        (j[key] as num?)?.toInt() ?? fallback;
    return CaptionSettings(
      language: language,
      speakers: number('speakers', 0),
      style:
          j['style'] is String ? j['style'] as String : CaptionStyle.plain.name,
      limits: usual.copyWith(
        charsPerLine: number('chars_per_line', usual.charsPerLine),
        lines: number('lines', usual.lines),
        words: number('words', usual.words),
        minDurationMs: number('shortest_ms', usual.minDurationMs),
        maxDurationMs: number('longest_ms', usual.maxDurationMs),
        gapFrames: number('gap_frames', usual.gapFrames),
        readingSpeed: number('reading_speed', usual.readingSpeed),
        speakerLabels:
            BridgeSpeakerLabels.values.asNameMap()[j['speaker_labels']] ??
                usual.speakerLabels,
      ),
      fillers: j['fillers'] == true,
      vocabulary: j['vocabulary'] is String ? j['vocabulary'] as String : '',
      translate: j['translate'] == true,
    );
  }

  Map<String, dynamic> toJson() => {
        'language': language,
        'speakers': speakers,
        'style': style,
        'chars_per_line': limits.charsPerLine,
        'lines': limits.lines,
        'words': limits.words,
        'shortest_ms': limits.minDurationMs,
        'longest_ms': limits.maxDurationMs,
        'gap_frames': limits.gapFrames,
        'reading_speed': limits.readingSpeed,
        'speaker_labels': limits.speakerLabels.name,
        'fillers': fillers,
        'vocabulary': vocabulary,
        'translate': translate,
      };

  /// What the engine is asked when the speech has to be heard first.
  BridgeTranscribeOptions transcribe({int? stream}) => BridgeTranscribeOptions(
        language: language,
        speakers: speakers,
        stream: stream,
        translate: translate,
        speakerName: l10n.captionsSpeakerName('{n}'),
        vocabulary: vocabulary,
      );
}

/// How the Create captions dialogue last asked for speech to be heard, for
/// hearing footage on its own, from the Project panel.
BridgeTranscribeOptions rememberedTranscribeOptions(Workspace workspace) {
  final j = workspace.captionSettings;
  return BridgeTranscribeOptions(
    language: j['language'] is String ? j['language'] as String : '',
    speakers: (j['speakers'] as num?)?.toInt() ?? 0,
    translate: j['translate'] == true,
    speakerName: l10n.captionsSpeakerName('{n}'),
    vocabulary: j['vocabulary'] is String ? j['vocabulary'] as String : '',
  );
}

BridgeCaptionLimits _usual(CompositionReference comp, String language) {
  try {
    return comp.captionLimitsFor(language: language);
  } catch (_) {
    return const BridgeCaptionLimits(
      charsPerLine: 42,
      lines: 2,
      words: 0,
      minDurationMs: 700,
      maxDurationMs: 7000,
      gapFrames: 2,
      readingSpeed: 17,
      splitOnSpeaker: true,
      speakerLabels: BridgeSpeakerLabels.off,
    );
  }
}

bool _heard(FootageReference item) {
  try {
    return item.hasTranscript();
  } catch (_) {
    return false;
  }
}

/// Make a caption layer from what [source] says: ask how, hear it where it
/// has not been heard, then make the layer and front the Captions panel on it.
Future<void> createCaptionsFrb(
    BuildContext context, LayerReference source) async {
  final app = context.read<LumitState>();
  final ui = context.read<LumitUiState>();
  final comp = ui.selectedComp;
  if (comp == null) return;
  List<FootageReference> heard;
  try {
    heard = source.heardItems();
  } catch (_) {
    heard = const [];
  }
  if (heard.isEmpty) {
    app.postNotice(l10n.captionsNoSound, error: true);
    return;
  }
  final unheard = [
    for (final item in heard)
      if (!_heard(item)) item,
  ];
  var languages = const <BridgeLanguage>[];
  var ready = const BridgeSpeechReady(
      runtime: false, pack: false, speakers: false, translates: false);
  var streams = const <BridgeAudioStream>[];
  try {
    languages = transcriptionLanguages();
    ready = speechReady();
    if (unheard.isNotEmpty) streams = unheard.first.audioStreams();
  } catch (_) {}
  String sourceName;
  try {
    sourceName = source.getName();
  } catch (_) {
    sourceName = '';
  }
  final answer = await showLumitModal<(CaptionSettings, int?)>(
    context: context,
    id: 'create-captions',
    builder: (close) => CreateCaptionsBody(
      subject: sourceName,
      start: CaptionSettings.remembered(ui.workspace, comp),
      usual: (language) => _usual(comp, language),
      workspace: ui.workspace,
      languages: languages,
      hears: unheard.isNotEmpty,
      ready: ready,
      streams: streams,
      onClose: close,
    ),
  );
  if (answer == null) return;
  final (settings, stream) = answer;
  ui.workspace.setCaptionSettings(settings.toJson());
  if (unheard.isNotEmpty) {
    if (app.transcription.running) {
      app.postNotice(l10n.captionsHearingBusy, error: true);
      return;
    }
    if (!context.mounted) return;
    final outcome = await transcribeWithPrompt(
        context, unheard, settings.transcribe(stream: stream));
    if (outcome == null || outcome.cancelled) return;
    if (!unheard.every(_heard)) {
      app.postNotice(l10n.captionsHearingFailed, error: true);
      return;
    }
  }
  var language = settings.language;
  if (language.isEmpty) {
    try {
      language = heard.first.transcript()?.language ?? '';
    } catch (_) {}
  }
  final named = languages.where((l) => l.code == language).firstOrNull?.name;
  final choice = captionLookChoice(ui.workspace, settings.style);
  try {
    final layer = comp.createCaptions(
      source: source,
      options: BridgeCaptionOptions(
        limits: settings.limits,
        name: named == null
            ? l10n.captionsLayerName
            : l10n.captionsLayerNamed(named),
        fillers: settings.fillers,
      ),
      look: choice.look(height: comp.getSettings().height),
    );
    _landed(app, ui, comp, layer);
  } catch (_) {
    app.postNotice(l10n.captionsCreateFailed, error: true);
  }
}

/// A caption layer just made in [comp]: picked, and the panel fronted on it.
void _landed(LumitState app, LumitUiState ui, CompositionReference comp,
    LayerReference layer) {
  noteCaptionLayer(layer);
  app.notifyDocumentChanged();
  ui.model.refresh();
  if (ui.selectedComp != comp) return;
  ui.setSelection([layer]);
  frontCaptionsPanel(ui);
}

/// Make a caption layer in the fronted composition from a subtitle file, in
/// the style last chosen. [path] skips the picker.
Future<void> importCaptionsFrb(BuildContext context, {String? path}) async {
  final app = context.read<LumitState>();
  final ui = context.read<LumitUiState>();
  final comp = ui.selectedComp;
  if (comp == null) return;
  final picked = path ?? await pickCaptionsToOpen();
  if (picked == null) return;
  final file = picked.split(RegExp(r'[\\/]')).last;
  final dot = file.lastIndexOf('.');
  final choice = captionLookChoice(
      ui.workspace, ui.workspace.captionSettings['style'] as String?);
  try {
    final layer = comp.importCaptions(
      path: picked,
      name: dot > 0 ? file.substring(0, dot) : file,
      look: choice.look(height: comp.getSettings().height),
    );
    _landed(app, ui, comp, layer);
  } catch (_) {
    app.postNotice(l10n.captionsImportFailed(file), error: true);
  }
}

/// The dialogue's body. The answers come back with the audio stream picked,
/// which is the file's and not kept.
class CreateCaptionsBody extends StatefulWidget {
  final String subject;
  final CaptionSettings start;
  final BridgeCaptionLimits Function(String language) usual;
  final Workspace workspace;
  final List<BridgeLanguage> languages;

  /// Some of what the layer plays has not been heard, so the questions about
  /// hearing it are asked.
  final bool hears;
  final BridgeSpeechReady ready;
  final List<BridgeAudioStream> streams;
  final void Function((CaptionSettings, int?)?) onClose;

  const CreateCaptionsBody({
    super.key,
    required this.subject,
    required this.start,
    required this.usual,
    required this.workspace,
    required this.languages,
    required this.hears,
    required this.ready,
    required this.streams,
    required this.onClose,
  });

  @override
  State<CreateCaptionsBody> createState() => _CreateCaptionsBodyState();
}

class _CreateCaptionsBodyState extends State<CreateCaptionsBody> {
  late String _language = widget.start.language;
  late int _speakers = widget.ready.speakers || widget.start.speakers != 0
      ? widget.start.speakers
      : 1;
  late String _style = widget.start.style;
  late BridgeCaptionLimits _limits = widget.start.limits;
  late bool _fillers = widget.start.fillers;
  late bool _translate = widget.start.translate && widget.ready.translates;
  late final TextEditingController _vocabulary =
      TextEditingController(text: widget.start.vocabulary);
  late int? _stream =
      widget.streams.length > 1 ? widget.streams.first.place : null;
  bool _more = false;

  static const double _labels = 132;

  @override
  void dispose() {
    _vocabulary.dispose();
    super.dispose();
  }

  void _create() => widget.onClose((
        CaptionSettings(
          language: _language,
          speakers: _speakers,
          style: _style,
          limits: _limits,
          fillers: _fillers,
          vocabulary: _vocabulary.text.trim(),
          translate: _translate,
        ),
        _stream,
      ));

  /// A new language brings its own line length and reading speed.
  void _pickLanguage(String language) {
    final usual = widget.usual(language);
    setState(() {
      _language = language;
      _limits = _limits.copyWith(
          charsPerLine: usual.charsPerLine, readingSpeed: usual.readingSpeed);
    });
  }

  /// One word reads a word at a time on one line, and moving off it goes
  /// back to the language's usual.
  void _pickStyle(CaptionLookChoice choice) {
    final was = captionLookChoice(widget.workspace, _style);
    setState(() {
      _style = choice.key;
      if (choice.single) {
        _limits = _limits.copyWith(words: 1, lines: 1);
      } else if (was.single) {
        final usual = widget.usual(_language);
        _limits = _limits.copyWith(words: usual.words, lines: usual.lines);
      }
    });
  }

  Widget _number(
    LumitTheme t, {
    required String id,
    required num value,
    required num min,
    required num max,
    required ValueChanged<num> onChanged,
    int decimals = 0,
    String? suffix,
    String? zero,
  }) =>
      Align(
        alignment: AlignmentDirectional.centerStart,
        child: SizedBox(
          width: 84,
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

  Widget _check(String id, bool value, ValueChanged<bool> onChanged) => Align(
        alignment: AlignmentDirectional.centerStart,
        child: HouseCheckbox(
            key: ValueKey<String>(id), value: value, onChanged: onChanged),
      );

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final w = widget;
    Widget row(String label, Widget control) =>
        dialogRow(t, label, control, labelColumn: _labels);
    final choices = captionLookChoices(w.workspace);
    final speakers = [if (w.ready.speakers) 0, 1, 2, 3, 4, 5, 6];
    const words = [0, 1, 2, 3, 4, 5, 6, 8, 10];
    final seconds = l10n.unitSymbolSeconds;

    final fold = <Widget>[
      row(
        l10n.captionsWords,
        Align(
            alignment: AlignmentDirectional.centerStart,
            child: dialogDropdown<int>(t,
                id: 'captions-words',
                width: 84,
                value: words.contains(_limits.words) ? _limits.words : 0,
                options: words,
                label: (n) => n == 0 ? l10n.captionsWordsAny : '$n',
                onChanged: (n) =>
                    setState(() => _limits = _limits.copyWith(words: n)))),
      ),
      row(
        l10n.captionsGap,
        _number(t,
            id: 'captions-gap',
            value: _limits.gapFrames,
            min: 0,
            max: 60,
            suffix: l10n.unitSymbolFramesShort,
            onChanged: (v) => setState(
                () => _limits = _limits.copyWith(gapFrames: v.round()))),
      ),
      row(
        l10n.captionsShortest,
        _number(t,
            id: 'captions-shortest',
            value: _limits.minDurationMs / 1000,
            min: 0.1,
            max: 10,
            decimals: 1,
            suffix: seconds,
            onChanged: (v) => setState(() =>
                _limits = _limits.copyWith(minDurationMs: (v * 1000).round()))),
      ),
      row(
        l10n.captionsLongest,
        _number(t,
            id: 'captions-longest',
            value: _limits.maxDurationMs / 1000,
            min: 1,
            max: 20,
            decimals: 1,
            suffix: seconds,
            onChanged: (v) => setState(() =>
                _limits = _limits.copyWith(maxDurationMs: (v * 1000).round()))),
      ),
      row(
        l10n.captionsReadingSpeed,
        _number(t,
            id: 'captions-reading-speed',
            value: _limits.readingSpeed,
            min: 0,
            max: 40,
            suffix: l10n.captionsUnitCps,
            zero: l10n.captionsOff,
            onChanged: (v) => setState(
                () => _limits = _limits.copyWith(readingSpeed: v.round()))),
      ),
      row(
        l10n.captionsSpeakerNames,
        Align(
            alignment: AlignmentDirectional.centerStart,
            child: dialogDropdown<BridgeSpeakerLabels>(t,
                id: 'captions-speaker-names',
                width: 140,
                value: _limits.speakerLabels,
                options: BridgeSpeakerLabels.values,
                label: speakerLabelsName,
                onChanged: (v) => setState(
                    () => _limits = _limits.copyWith(speakerLabels: v)))),
      ),
      row(
          l10n.captionsFillers,
          _check('captions-fillers', _fillers,
              (v) => setState(() => _fillers = v))),
      if (w.hears)
        row(
          l10n.captionsVocabulary,
          HouseTextField(
            key: const ValueKey('captions-vocabulary'),
            controller: _vocabulary,
            width: double.infinity,
          ),
        ),
      if (w.hears && w.ready.translates)
        row(
            l10n.captionsTranslate,
            _check('captions-translate', _translate,
                (v) => setState(() => _translate = v))),
    ];

    return dialogEnter(
      onEnter: _create,
      child: DialogFrame(
        width: 480,
        children: [
          dialogTitleBar(t,
              title: l10n.captionsCreateTitle,
              subject: w.subject,
              onClose: () => w.onClose(null),
              keyPrefix: 'create-captions'),
          Padding(
            padding: dialogBodyPadding,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (w.hears) ...[
                  row(
                    l10n.captionsLanguage,
                    dialogDropdown<String>(t,
                        id: 'captions-language',
                        value: _language,
                        options: [
                          '',
                          for (final l in w.languages) l.code,
                        ],
                        label: (code) => code.isEmpty
                            ? l10n.captionsDetect
                            : w.languages
                                    .where((l) => l.code == code)
                                    .firstOrNull
                                    ?.name ??
                                code,
                        onChanged: _pickLanguage),
                  ),
                  row(
                    l10n.captionsSpeakers,
                    Align(
                        alignment: AlignmentDirectional.centerStart,
                        child: dialogDropdown<int>(t,
                            id: 'captions-speakers',
                            width: 140,
                            value: speakers.contains(_speakers) ? _speakers : 1,
                            options: speakers,
                            label: (n) => switch (n) {
                                  0 => l10n.captionsDetect,
                                  1 => l10n.captionsSpeakersOne,
                                  _ => '$n',
                                },
                            onChanged: (n) => setState(() => _speakers = n))),
                  ),
                  if (w.streams.length > 1)
                    row(
                      l10n.captionsAudioStream,
                      dialogDropdown<int>(t,
                          id: 'captions-stream',
                          value: _stream ?? w.streams.first.place,
                          options: [for (final s in w.streams) s.place],
                          label: (place) => audioStreamLabel(
                              w.streams.firstWhere((s) => s.place == place)),
                          onChanged: (p) => setState(() => _stream = p)),
                    ),
                ],
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 6),
                  child: Row(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      SizedBox(
                        width: _labels,
                        child: Padding(
                          padding: const EdgeInsets.only(top: 2),
                          child: Text(l10n.captionsStyle, style: t.body),
                        ),
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: Wrap(
                          spacing: 6,
                          runSpacing: 6,
                          children: [
                            for (final choice in choices)
                              CaptionLookTile(
                                key: ValueKey<String>(
                                    'captions-style-${choice.key}'),
                                choice: choice,
                                width: 72,
                                selected: choice.key == _style,
                                onPressed: () => _pickStyle(choice),
                              ),
                          ],
                        ),
                      ),
                    ],
                  ),
                ),
                row(
                  l10n.captionsLines,
                  Align(
                      alignment: AlignmentDirectional.centerStart,
                      child: dialogDropdown<int>(t,
                          id: 'captions-lines',
                          width: 84,
                          value: _limits.lines.clamp(1, 2),
                          options: const [1, 2],
                          label: (n) => '$n',
                          onChanged: (n) => setState(
                              () => _limits = _limits.copyWith(lines: n)))),
                ),
                row(
                  l10n.captionsCharsPerLine,
                  _number(t,
                      id: 'captions-chars',
                      value: _limits.charsPerLine,
                      min: 8,
                      max: 80,
                      onChanged: (v) => setState(() =>
                          _limits = _limits.copyWith(charsPerLine: v.round()))),
                ),
              ],
            ),
          ),
          dialogFold(
            t,
            key: const ValueKey('create-captions-more'),
            title: l10n.captionsMore,
            open: _more,
            onToggle: () => setState(() => _more = !_more),
            readingAt: _labels + 10,
            rows: fold,
          ),
          dialogFooter(
            t,
            keyPrefix: 'create-captions',
            actions: [
              dialogPlainAction(t,
                  key: const ValueKey('create-captions-cancel'),
                  label: l10n.cancel,
                  onPressed: () => w.onClose(null)),
              dialogFilledAction(t,
                  key: const ValueKey('create-captions-go'),
                  label: l10n.captionsCreate,
                  onPressed: _create),
            ],
          ),
        ],
      ),
    );
  }
}

/// What a choice of speaker labels is called.
String speakerLabelsName(BridgeSpeakerLabels labels) => switch (labels) {
      BridgeSpeakerLabels.off => l10n.captionsOff,
      BridgeSpeakerLabels.names => l10n.captionsSpeakerNamesNames,
      BridgeSpeakerLabels.dashes => l10n.captionsSpeakerNamesDashes,
    };
