// The export dialog, rebuilt to its drawing (K-444, K-449, K-458).
//
// **The shape is the approved drawing's.** A frame of 640: a kicker title
// strip naming the composition, a row of page tabs, and a body of titled
// groups — Output, Composition, Time, Picture, Audio — whose rows are a label
// in a fixed 100px column with the control beside it, two to a line where the
// rows are short. The footer states the facts (frames, length, size, rate, an
// estimate of the file) and carries the two actions: *Add to queue* outlined,
// and EXPORT, the single filled action.
//
// **Nothing here polls.** The old dialog started an export and then watched it
// from this window; the drawing has no progress line, because progress belongs
// to the queue window and to the status strip. Both buttons queue the export —
// *Add to queue* leaves it waiting, EXPORT sets the queue running — and the
// queue window opens on top so the work is never started somewhere you cannot
// see it.
//
// **Every field defaults to the truth rather than to a number.** The frame rate
// starts as the composition's own; the span starts as the work area when one is
// set and the whole comp when not — the values a user would have typed anyway,
// already typed.
//
// **What the drawing asks for and the engine cannot yet give** is left out
// rather than drawn dead, which is the rule the Settings window's three missing
// pages already set (K-465): the render-settings rows (quality, effects, solo
// switches, proxies, guide layers, disk cache, colour depth), the picture's
// colour management (channels, alpha, depth, colour space), crop and the region
// of interest, the Still and Audio-only output types, and *Save as…* for a
// preset of one's own. Each is engine-first work, listed in docs/TODO.md; the
// rows are drawn and waiting.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';

import '../l10n/strings.dart';
import '../state/file_dialogs.dart';
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'export_queue_frb.dart';
import 'status_line_frb.dart';

/// The frame the drawing gives this dialog.
const double exportDialogWidth = 640;

/// The label column and the gap after it, and a row's own height — the
/// drawing's own, which are not the New composition dialog's (K-458: each
/// drawing measures itself).
const double exportLabelColumn = 100;
const double exportRowGap = 10;
const double exportRowHeight = 28;

/// The air between the body's edges and its groups, and between two columns of
/// short rows.
const EdgeInsets exportBodyPadding = EdgeInsets.fromLTRB(14, 10, 14, 12);
const double exportColumnGap = 20;

/// The two widths the drawing gives a control that does not fill its row: a
/// button beside a well, and a well holding a number.
/// The room the frame's own bands take: the title strip, the tab row and the
/// footer, plus a little air — what the body has to fit inside when the window
/// is short.
const double exportChromeHeight = 160;

const double exportButtonWidth = 72;
const double exportNumberWell = 56;
const double exportSizeWell = 64;

/// The delivery presets offered, with the empty string for a custom export —
/// exactly the names the engine's own preset table knows, because a preset the
/// engine has never heard of stamps nothing and looks broken.
const List<String> _presets = [
  '',
  'youtube_1080p60',
  'youtube_1440p60',
  'youtube_4k60',
  'vertical_1080p60',
];

/// The AAC rates offered, bits per second. 320 leads because it is the
/// delivery-preset rate (docs/06 §7.5); the rest are the customary steps down.
const List<int> _audioRates = [320000, 256000, 192000, 128000];

/// What the export writes: a video file, or one still per frame. The drawing
/// offers two more — Still and Audio only — which the engine cannot write yet.
enum ExportOutputType { video, imageSequence }

/// A page of the dialog. The drawing's Output page holds every group; the rest
/// front the one group they name, until the long tail behind them (colour
/// management, encoder options, metadata) exists to fill them out.
enum ExportPage { output, picture, time, audio }

/// How much of the composition to write.
enum _Span { workArea, wholeComp, custom }

/// One output format: the engine key, the reading label, and the file
/// extension its picker filters on.
class _Format {
  final String key;
  final String label;
  final String extension;
  final String pickerLabel;
  final ExportOutputType type;
  const _Format(
      this.key, this.label, this.extension, this.pickerLabel, this.type);
}

/// H.264 first because it is the preset default; the sequences last because
/// they are the specialist choice.
List<_Format> get _formats => [
      _Format('h264', l10n.formatH264, 'mp4', l10n.formatMp4Picker,
          ExportOutputType.video),
      _Format('hevc', l10n.formatHevc, 'mp4', l10n.formatMp4Picker,
          ExportOutputType.video),
      _Format('png', l10n.formatPngSequence, 'png', l10n.formatPngPicker,
          ExportOutputType.imageSequence),
      _Format('tiff', l10n.formatTiffSequence, 'tiff', l10n.formatTiffPicker,
          ExportOutputType.imageSequence),
    ];

Future<void> showExportDialogFrb({
  required BuildContext context,
  required CompositionReference comp,
  Future<String?> Function()? picker,
}) =>
    showLumitModal<void>(
      context: context,
      id: 'export',
      builder: (close) => _ExportDialog(
        comp: comp,
        picker: picker,
        onClose: () => close(null),
      ),
    );

class _ExportDialog extends StatefulWidget {
  final CompositionReference comp;
  final Future<String?> Function()? picker;
  final VoidCallback onClose;

  const _ExportDialog({
    required this.comp,
    required this.picker,
    required this.onClose,
  });

  @override
  State<_ExportDialog> createState() => _ExportDialogState();
}

class _ExportDialogState extends State<_ExportDialog> {
  ExportPage _page = ExportPage.output;

  String _preset = '';
  _Format _format = _formats.first;
  int _bitrate = 0;
  double _fps = 60;
  bool _ownRate = false;
  _Span _span = _Span.workArea;
  int _rangeStart = 0;
  int _rangeEnd = 1;
  bool _audio = true;
  int _audioRate = _audioRates.first;
  String? _path;
  bool _openFolder = false;
  String? _refused;

  /// The output size: a fraction of the comp's, or the pixels typed into the
  /// Resize row when it is ticked.
  int _divisor = 1;
  bool _resize = false;
  bool _lockAspect = true;
  int _resizeWidth = 1920;
  int _resizeHeight = 1080;

  /// The comp's own facts, read once as the dialog opens — never in `build`,
  /// which must cross no bridge (the standing rebuild-path rule).
  String _compName = '';
  double _compFps = 60;
  int _compFrames = 1;
  int _compWidth = 1920;
  int _compHeight = 1080;
  int _workStart = 0;
  int _workEnd = 1;
  bool _hasWorkArea = false;

  @override
  void initState() {
    super.initState();
    // The dialog opens on the delivery preset rather than a blank Custom:
    // docs/06 §7.5 names YouTube 1080p60 the preset default, and a fresh
    // dialog showing "Custom" with a bit rate of 0 read as broken.
    _applyPreset('youtube_1080p60');
    try {
      final settings = widget.comp.getSettings();
      _compName = settings.name;
      _compWidth = settings.width;
      _compHeight = settings.height;
      _resizeWidth = settings.width;
      _resizeHeight = settings.height;
      _compFps = settings.fpsNum / settings.fpsDen;
      _fps = _compFps;
      _compFrames = widget.comp.durationFrames();
      final area = widget.comp.getWorkArea();
      if (area != null) {
        _hasWorkArea = true;
        _workStart = (area.inPoint.num / area.inPoint.den * _compFps).round();
        _workEnd = (area.outPoint.num / area.outPoint.den * _compFps)
            .round()
            .clamp(_workStart + 1, _compFrames);
      } else {
        _workStart = 0;
        _workEnd = _compFrames;
        _span = _Span.wholeComp;
      }
      _rangeStart = _workStart;
      _rangeEnd = _workEnd;
    } catch (_) {
      // A comp that cannot be read leaves the placeholder defaults; the export
      // itself will refuse with the engine's own words.
    }
  }

  // ---- what the fields add up to -------------------------------------------

  bool get _images => _format.type == ExportOutputType.imageSequence;

  /// The span in comp frames, end exclusive.
  (int, int) get _range => switch (_span) {
        _Span.workArea => (_workStart, _workEnd),
        _Span.wholeComp => (0, _compFrames),
        _Span.custom => (_rangeStart, _rangeEnd),
      };

  /// The size the file will be, in pixels: the comp's own divided by the
  /// resolution chosen, unless Resize says otherwise.
  (int, int) get _outputSize => _resize
      ? (_resizeWidth, _resizeHeight)
      : (
          (_compWidth / _divisor).round().clamp(1, 16384),
          (_compHeight / _divisor).round().clamp(1, 16384),
        );

  /// The footer's line: what pressing the button would produce.
  String get _summary {
    final (start, end) = _range;
    final frames = (end - start).clamp(0, 1 << 30);
    final rate = _fps <= 0 ? _compFps : _fps;
    final seconds = rate <= 0 ? 0.0 : frames / rate;
    final (width, height) = _outputSize;
    final line = l10n.exportSummary(
      '$frames',
      seconds.toStringAsFixed(1),
      '$width',
      '$height',
      _formatRate(rate),
    );
    // A bit rate the encoder chose for itself is not a number this dialog may
    // multiply out, so the estimate simply is not offered.
    if (_images || _bitrate <= 0) return line;
    final gigabytes = _bitrate * seconds / 8 / 1000;
    return '$line · ${l10n.exportSummarySize(gigabytes.toStringAsFixed(1))}';
  }

  BridgeExportSpec get _spec {
    final (start, end) = _range;
    final (width, height) = _outputSize;
    final own = width == _compWidth && height == _compHeight;
    return BridgeExportSpec(
      preset: _preset,
      codec: _format.key,
      // Zero means "the composition's own size", which is what the engine
      // would work out anyway — sending it back only when it differs keeps a
      // preset's own size from being overwritten by this dialog's arithmetic.
      width: own ? 0 : width,
      height: own ? 0 : height,
      bitrateMbps: _images ? 0 : _bitrate,
      fps: _fps,
      rangeStartFrame: start,
      rangeEndFrame: end,
      includeAudio: !_images && _audio,
      audioBitRate: _images ? 0 : _audioRate,
    );
  }

  // ---- the frame -----------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: exportDialogWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.exportAction,
          subject: _compName,
          onClose: widget.onClose,
          keyPrefix: 'export',
        ),
        dialogTabs<ExportPage>(
          t,
          tabs: [
            (ExportPage.output, l10n.exportGroupOutput),
            (ExportPage.picture, l10n.exportGroupPicture),
            (ExportPage.time, l10n.exportGroupTime),
            (ExportPage.audio, l10n.exportGroupAudio),
          ],
          current: _page,
          onPick: (page) => setState(() => _page = page),
          keyPrefix: 'export',
        ),
        // Vertical metrics never squish (§12A.6): when the window is too short
        // for every group, the body scrolls rather than the rows shrinking.
        ConstrainedBox(
          constraints: BoxConstraints(
            maxHeight: (MediaQuery.sizeOf(context).height - exportChromeHeight)
                .clamp(exportRowHeight, 4000),
          ),
          child: SingleChildScrollView(
            child: Padding(
              padding: exportBodyPadding,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                // The drawing sets 10 between groups, and each group holds 8 of
                // its own above it for the kicker notched into its top edge.
                children: [
                  for (final (index, group) in _groups(t).indexed) ...[
                    if (index > 0) const SizedBox(height: dialogGroupGap),
                    group,
                  ],
                ],
              ),
            ),
          ),
        ),
        dialogFooter(
          t,
          summary: _refused ?? _summary,
          keyPrefix: 'export',
          actions: [
            HouseButton(
              key: const ValueKey('export-add-to-queue'),
              onPressed: _path == null ? null : () => _queue(start: false),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              child: Text(l10n.exportAddToQueue),
            ),
            HouseButton(
              key: const ValueKey('export-start'),
              // The window's default action (K-319): Enter exports unless a
              // field is being typed in.
              primary: true,
              autofocus: true,
              padding: const EdgeInsets.symmetric(horizontal: 16),
              onPressed: _path == null ? null : () => _queue(start: true),
              child: Text(l10n.exportAction),
            ),
          ],
        ),
      ],
    );
  }

  /// The groups this page shows. Output holds them all, as the drawing draws
  /// it; every other tab fronts the one it names.
  List<Widget> _groups(LumitTheme t) => switch (_page) {
        ExportPage.output => [
            _outputGroup(t),
            _compositionGroup(t),
            _timeGroup(t),
            _pictureGroup(t),
            _audioGroup(t),
          ],
        ExportPage.picture => [_pictureGroup(t)],
        ExportPage.time => [_timeGroup(t)],
        ExportPage.audio => [_audioGroup(t)],
      };

  // ---- the groups ----------------------------------------------------------

  Widget _outputGroup(LumitTheme t) => dialogGroup(
        t,
        l10n.exportGroupOutput,
        [
          _row(
            t,
            l10n.exportType,
            Row(children: [
              for (final type in ExportOutputType.values) _typeChip(t, type),
            ]),
          ),
          _columns(
            _row(
              t,
              l10n.exportFormat,
              dialogDropdown<_Format>(
                t,
                id: 'export-format',
                value: _format,
                options: _formats.where((f) => f.type == _format.type).toList(),
                label: (f) => f.label,
                onChanged: _setFormat,
              ),
            ),
            _row(
              t,
              l10n.exportPreset,
              dialogDropdown<String>(
                t,
                id: 'export-preset',
                value: _preset,
                options: _presets,
                label: (p) => p.isEmpty ? l10n.custom : p,
                onChanged: _applyPreset,
              ),
            ),
          ),
          _row(
            t,
            l10n.exportWriteTo,
            Row(children: [
              Expanded(
                child: _well(
                  t,
                  _path == null ? l10n.exportNotChosen : _leaf(_path!),
                  key: const ValueKey('export-path'),
                  tone: t.textMuted,
                ),
              ),
              const SizedBox(width: 6),
              SizedBox(
                width: exportButtonWidth,
                height: dialogControlHeight,
                child: HouseButton(
                  key: const ValueKey('export-choose'),
                  onPressed: _choose,
                  child: Text(l10n.exportBrowse, style: t.body),
                ),
              ),
            ]),
          ),
          _row(
            t,
            l10n.exportWhenDone,
            Row(children: [
              HouseCheckbox(
                key: const ValueKey('export-open-folder'),
                value: _openFolder,
                onChanged: (on) => setState(() => _openFolder = on),
              ),
              const SizedBox(width: 6),
              Text(l10n.exportOpenFolder, style: t.body),
            ]),
          ),
        ],
        key: const ValueKey('export-group-output'),
      );

  Widget _compositionGroup(LumitTheme t) => dialogGroup(
        t,
        l10n.exportGroupComposition,
        [
          _columns(
            _row(
              t,
              l10n.exportResolution,
              dialogDropdown<int>(
                t,
                id: 'export-resolution',
                value: _divisor,
                options: const [1, 2, 3, 4],
                label: (d) => d == 1
                    ? l10n.exportResolutionFull('$_compWidth', '$_compHeight')
                    : l10n.exportResolutionFraction(
                        '$d',
                        '${(_compWidth / d).round()}',
                        '${(_compHeight / d).round()}',
                      ),
                onChanged: (d) => setState(() => _divisor = d),
              ),
            ),
            null,
          ),
        ],
        key: const ValueKey('export-group-composition'),
      );

  Widget _timeGroup(LumitTheme t) {
    final (start, end) = _range;
    final rate = _fps <= 0 ? _compFps : _fps;
    return dialogGroup(
      t,
      l10n.exportGroupTime,
      [
        _columns(
          _row(
            t,
            l10n.exportSpan,
            dialogDropdown<_Span>(
              t,
              id: 'export-span',
              value: _span,
              options: [
                if (_hasWorkArea) _Span.workArea,
                _Span.wholeComp,
                _Span.custom,
              ],
              label: (span) => switch (span) {
                _Span.workArea =>
                  l10n.exportSpanWorkArea('$_workStart', '$_workEnd'),
                _Span.wholeComp => l10n.exportSpanWholeComp,
                _Span.custom => l10n.custom,
              },
              onChanged: (span) => setState(() {
                _span = span;
                if (span == _Span.custom) {
                  _rangeStart = start;
                  _rangeEnd = end;
                }
              }),
            ),
          ),
          _row(
            t,
            l10n.frameRate,
            Row(children: [
              Expanded(
                child: dialogDropdown<bool>(
                  t,
                  id: 'export-rate-source',
                  value: _ownRate,
                  options: const [false, true],
                  label: (own) => own
                      ? l10n.custom
                      : l10n.exportRateComposition(_formatRate(_compFps)),
                  onChanged: (own) => setState(() {
                    _ownRate = own;
                    if (!own) _fps = _compFps;
                  }),
                ),
              ),
              const SizedBox(width: 6),
              SizedBox(
                width: exportNumberWell,
                height: dialogControlHeight,
                // The comp's own rate is a *reading* until a rate of one's own
                // is chosen: a well that could be dragged while the list beside
                // it says "Composition" would be two answers to one question.
                child: _ownRate
                    ? DragValueField(
                        key: const ValueKey('export-fps'),
                        value: _fps,
                        min: 1,
                        max: 240,
                        decimals: 2,
                        resetTo: _compFps,
                        fill: t.surface0,
                        onChanged: (v) => setState(() => _fps = v.toDouble()),
                      )
                    : _well(t, _formatRate(_compFps),
                        key: const ValueKey('export-fps'),
                        tone: t.textDisabled),
              ),
            ]),
          ),
        ),
        if (_span == _Span.custom)
          _row(
            t,
            l10n.exportSpanFrames,
            Row(children: [
              SizedBox(
                width: exportNumberWell,
                height: dialogControlHeight,
                child: DragValueField(
                  key: const ValueKey('export-range-start'),
                  value: _rangeStart,
                  min: 0,
                  max: _compFrames - 1,
                  fill: t.surface0,
                  onChanged: (v) => setState(() {
                    _rangeStart = v.toInt();
                    if (_rangeEnd <= _rangeStart) _rangeEnd = _rangeStart + 1;
                  }),
                ),
              ),
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 6),
                child: Text('–', style: dialogMono(t)),
              ),
              SizedBox(
                width: exportNumberWell,
                height: dialogControlHeight,
                child: DragValueField(
                  key: const ValueKey('export-range-end'),
                  value: _rangeEnd,
                  min: 1,
                  max: _compFrames,
                  fill: t.surface0,
                  onChanged: (v) => setState(() {
                    _rangeEnd = v.toInt().clamp(1, _compFrames);
                    if (_rangeStart >= _rangeEnd) _rangeStart = _rangeEnd - 1;
                  }),
                ),
              ),
            ]),
          ),
        _reading(
          t,
          l10n.exportTimeReading(
            _timecode(start, rate),
            _timecode(end, rate),
            _timecode(end - start, rate),
          ),
          key: const ValueKey('export-time-reading'),
        ),
      ],
      key: const ValueKey('export-group-time'),
    );
  }

  Widget _pictureGroup(LumitTheme t) {
    final (width, height) = _outputSize;
    return dialogGroup(
      t,
      l10n.exportGroupPicture,
      [
        _row(
          t,
          l10n.exportResize,
          Row(children: [
            HouseCheckbox(
              key: const ValueKey('export-resize'),
              value: _resize,
              onChanged: (on) => setState(() => _resize = on),
            ),
            const SizedBox(width: 6),
            SizedBox(
              width: exportSizeWell,
              height: dialogControlHeight,
              child: _resize
                  ? DragValueField(
                      key: const ValueKey('export-resize-width'),
                      value: _resizeWidth,
                      min: 16,
                      max: 16384,
                      fill: t.surface0,
                      onChanged: (v) => _setResize(width: v.toInt()),
                    )
                  : _well(t, '$_resizeWidth',
                      key: const ValueKey('export-resize-width'),
                      tone: t.textDisabled),
            ),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 6),
              child: Text('×', style: dialogMono(t)),
            ),
            SizedBox(
              width: exportSizeWell,
              height: dialogControlHeight,
              child: _resize
                  ? DragValueField(
                      key: const ValueKey('export-resize-height'),
                      value: _resizeHeight,
                      min: 16,
                      max: 16384,
                      fill: t.surface0,
                      onChanged: (v) => _setResize(height: v.toInt()),
                    )
                  : _well(t, '$_resizeHeight',
                      key: const ValueKey('export-resize-height'),
                      tone: t.textDisabled),
            ),
            const SizedBox(width: 8),
            HouseCheckbox(
              key: const ValueKey('export-lock-aspect'),
              value: _lockAspect,
              onChanged: (on) => setState(() => _lockAspect = on),
            ),
            const SizedBox(width: 6),
            Flexible(
              child: Text(l10n.exportLockAspect,
                  style: t.body.copyWith(color: t.textMuted),
                  overflow: TextOverflow.ellipsis),
            ),
            const Spacer(),
            Flexible(
              child: Text(
                l10n.exportFinalSize('$width', '$height'),
                key: const ValueKey('export-final-size'),
                style: dialogMono(t),
                overflow: TextOverflow.ellipsis,
                textAlign: TextAlign.right,
              ),
            ),
          ]),
        ),
        if (!_images)
          _row(
            t,
            l10n.exportBitRate,
            Row(children: [
              SizedBox(
                width: 90,
                height: dialogControlHeight,
                child: DragValueField(
                  key: const ValueKey('export-bitrate'),
                  value: _bitrate,
                  min: 0,
                  max: 400,
                  suffix: _bitrate == 0 ? null : ' Mb/s',
                  fill: t.surface0,
                  onChanged: (v) => setState(() => _bitrate = v.toInt()),
                ),
              ),
            ]),
          ),
        if (_images)
          _reading(
            t,
            l10n.exportImageSequenceNote(_format.extension.toUpperCase()),
          ),
      ],
      key: const ValueKey('export-group-picture'),
    );
  }

  Widget _audioGroup(LumitTheme t) => dialogGroup(
        t,
        l10n.exportGroupAudio,
        [
          _row(
            t,
            l10n.exportGroupAudio,
            Row(children: [
              SizedBox(
                width: 110,
                height: dialogControlHeight,
                child: BareDropdown<bool>(
                  key: const ValueKey('export-audio'),
                  value: _audio,
                  options: const [true, false],
                  label: (on) => on ? l10n.exportAudioOn : l10n.exportAudioOff,
                  onChanged:
                      _images ? null : (on) => setState(() => _audio = on),
                ),
              ),
              const SizedBox(width: 6),
              SizedBox(
                width: 90,
                height: dialogControlHeight,
                child: BareDropdown<int>(
                  key: const ValueKey('export-audio-rate'),
                  value: _audioRate,
                  options: _audioRates,
                  label: (r) => '${r ~/ 1000} kb/s',
                  onChanged: _images || !_audio
                      ? null
                      : (r) => setState(() => _audioRate = r),
                ),
              ),
              const Spacer(),
              // The engine mixes every export at 48 kHz stereo
              // (`EXPORT_AUDIO_RATE`), so this is a reading, not a choice: the
              // drawing's three further faces would be controls over nothing.
              Text(l10n.exportAudioReading, style: dialogMono(t)),
            ]),
          ),
        ],
        key: const ValueKey('export-group-audio'),
      );

  // ---- the pieces a row is made of -----------------------------------------

  Widget _row(LumitTheme t, String label, Widget control) => dialogRow(
        t,
        label,
        control,
        labelColumn: exportLabelColumn,
        gap: exportRowGap,
        minHeight: exportRowHeight,
      );

  /// Two short rows side by side, as the drawing sets them.
  Widget _columns(Widget left, Widget? right) => Row(
        children: [
          Expanded(child: left),
          const SizedBox(width: exportColumnGap),
          Expanded(child: right ?? const SizedBox.shrink()),
        ],
      );

  /// A recessed box holding something the user did not type — a chosen path,
  /// or a number this row is not offering to change.
  Widget _well(LumitTheme t, String text, {Key? key, Color? tone}) => Container(
        key: key,
        height: dialogControlHeight,
        alignment: Alignment.centerLeft,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        decoration: BoxDecoration(
          color: t.surface0,
          border: Border.all(color: t.hairline),
          borderRadius: BorderRadius.circular(dialogGroupRadius),
        ),
        child: Text(
          text,
          style: t.mono.copyWith(fontSize: 11, color: tone ?? t.textPrimary),
          overflow: TextOverflow.ellipsis,
        ),
      );

  /// A factual line under a group's rows, in the label column's own indent.
  Widget _reading(LumitTheme t, String text, {Key? key}) => Padding(
        padding: const EdgeInsets.only(
            left: exportLabelColumn + exportRowGap, top: 2, bottom: 4),
        child: Text(text, key: key, style: dialogMono(t)),
      );

  /// One of the output types, as the drawing's chip: the chosen one outlined
  /// and bright, the rest bare and quiet — no accent, which §3.1 keeps for the
  /// filled action in the footer.
  Widget _typeChip(LumitTheme t, ExportOutputType type) {
    final on = _format.type == type;
    return GestureDetector(
      key: ValueKey<String>('export-type-${type.name}'),
      behavior: HitTestBehavior.opaque,
      onTap: () => _setFormat(_formats.firstWhere((f) => f.type == type)),
      child: Container(
        margin: const EdgeInsets.only(right: dialogTabGap),
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(dialogGroupRadius),
          border: Border.all(
              color: on ? t.hairlineStrong : const Color(0x00000000)),
        ),
        child: Text(
          switch (type) {
            ExportOutputType.video => l10n.exportTypeVideo.toUpperCase(),
            ExportOutputType.imageSequence =>
              l10n.exportTypeImageSequence.toUpperCase(),
          },
          style: on ? t.kickerOn : t.kicker,
        ),
      ),
    );
  }

  // ---- what the controls do ------------------------------------------------

  void _setFormat(_Format format) => setState(() {
        _format = format;
        // A chosen path keeps its stem but not its extension — `shot.mp4` as a
        // PNG sequence would write `shot.mp4.00001`-shaped nonsense.
        final path = _path;
        if (path != null) {
          _path = path.replaceFirst(
              RegExp(r'\.[A-Za-z0-9]+$'), '.${format.extension}');
        }
      });

  /// A preset stamps the fields the engine says it stamps — codec, bit rate —
  /// and leaves the rate and span alone: how much to export is the user's,
  /// whatever the delivery target.
  void _applyPreset(String preset) {
    setState(() => _preset = preset);
    if (preset.isEmpty) return;
    final stamp = exportPreset(preset: preset, compName: '', template: '');
    setState(() {
      _format = _formats.firstWhere(
        (f) => f.key == stamp.codec,
        orElse: () => _formats.first,
      );
      _bitrate = stamp.bitrateMbps;
    });
  }

  /// Editing one side of the output size, carrying the other with it when the
  /// aspect is locked.
  void _setResize({int? width, int? height}) => setState(() {
        if (width != null) {
          final ratio = _resizeHeight / _resizeWidth;
          _resizeWidth = width;
          if (_lockAspect) {
            _resizeHeight = (width * ratio).round().clamp(16, 16384);
          }
        }
        if (height != null) {
          final ratio = _resizeWidth / _resizeHeight;
          _resizeHeight = height;
          if (_lockAspect) {
            _resizeWidth = (height * ratio).round().clamp(16, 16384);
          }
        }
      });

  Future<void> _choose() async {
    final picker = widget.picker;
    final path = picker != null
        ? await picker()
        : await pickExportSaveLocation(
            'export.${_format.extension}',
            extension: _format.extension,
            label: _format.pickerLabel,
          );
    if (path != null) setState(() => _path = path);
  }

  /// Queue the export, and show the queue. A refusal — a spec the encoder will
  /// not take — is shown in the footer where the summary was, rather than
  /// swallowed.
  void _queue({required bool start}) {
    final path = _path;
    if (path == null) return;
    setState(() => _refused = null);
    try {
      widget.comp.queueExport(
        spec: _spec,
        path: path,
        start: start,
        openFolder: _openFolder,
      );
    } catch (error) {
      setState(() => _refused = '$error');
      return;
    }
    // Wake the status line, which polls only while an export is live.
    if (start) statusLineExportStarted.value++;
    final context = this.context;
    widget.onClose();
    showExportQueueFrb(context: context);
  }

  String _timecode(int frames, double rate) =>
      timecodeOfRate(frames, (rate * 1000).round(), 1000);

  static String _leaf(String path) => path.split(RegExp(r'[/\\]')).last;
}

/// A rate as one number: `60`, `23.976` — trailing zeros dropped, so the
/// ordinary rates read as ordinary numbers.
String _formatRate(double rate) {
  final whole = rate.roundToDouble();
  if ((rate - whole).abs() < 0.0005) return '${whole.toInt()}';
  return rate
      .toStringAsFixed(3)
      .replaceFirst(RegExp(r'0+$'), '')
      .replaceFirst(RegExp(r'\.$'), '');
}
