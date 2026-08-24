// The export dialog, rebuilt to its drawing (K-444, K-449, K-458, K-485).
//
// **The shape is the approved drawing's.** A frame of 640: a kicker title
// strip naming the composition, a row of section tabs, and a body of titled
// groups — Output, Composition, Time, Picture, Colour, Audio, Metadata — whose
// rows are a label in a fixed column with the control beside it, two to a line
// where the rows are short. The footer states the facts (frames, length, size,
// rate, an estimate of the file) and carries the two actions: *Add to queue*
// outlined, and EXPORT, the single filled action.
//
// **One page, and the tabs say where you are** (K-485). The dialog is a single
// scrolling page rather than six of them: the tab strip follows the section
// last touched or scrolled to, and clicking a tab scrolls its section into view
// when it is not fully visible and lights its box for a moment. A settings
// dialog where the same file's picture and sound are on different pages hides
// half of what an export is from the person deciding it.
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
// **The engine decides what a file can hold.** Every format carries a
// capability row (K-479), and a control the chosen format cannot honour is
// drawn **disabled** rather than left out or left live: an mp4 has no alpha
// channel and no sixteenth bit, a PNG sequence has no sound and no bitrate, a
// WAV has no picture at all. The two rows the engine cannot back at all —
// proxies and guide layers, each a document subsystem before it is an export
// setting — are disabled with a reason for the same reason: the drawing shows
// them, and honesty is a dead control with a name, not a missing one.
//
// **Nothing here crosses the bridge in `build`.** The capability row, the
// refusal, the crop and the bitrate are all the engine's answers, and all four
// are recomputed in `_edit` — the one place a field changes — so a rebuild
// costs nothing (the standing rebuild-path rule).

import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../main.dart';
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

/// The label column of the **right** column of a paired row.
///
/// The drawing gives every label 100 and then asks the frame-rate row for a
/// 150px dropdown *and* a 56px value well beside it, which is 212 of control in
/// a 173 column — the drawing overflows itself, and the well was the part that
/// lost. The owner's ruling is that the value box always fits and the right
/// column stays aligned, so the right column's controls extend **left** into
/// its label instead: 78 for the label, 195 for the control, one left edge and
/// one right edge down the whole column.
const double exportLabelColumnPaired = 78;

/// The air between the body's edges and its groups, and between two columns of
/// short rows.
const EdgeInsets exportBodyPadding = EdgeInsets.fromLTRB(14, 10, 14, 12);
const double exportColumnGap = 20;

/// The room the frame's own bands take: the title strip, the tab row and the
/// footer, plus a little air — what the body has to fit inside when the window
/// is short.
const double exportChromeHeight = 160;

const double exportButtonWidth = 72;
const double exportNumberWell = 56;
const double exportSizeWell = 64;

/// The preset row: a narrower list than a full-width control, then *Edit* and
/// *Save as…* beside it (the owner's ruling on the drawing's single button).
const double exportPresetDropdown = 95;
const double exportPresetEditButton = 48;

/// A crop inset's well — four of them and their T · L · B · R marks fit the
/// row beside the region tick and the final-size reading.
const double exportCropWell = 40;

/// The audio row's five faces, as the drawing measures them.
const double exportAudioSourceWidth = 110;
const double exportAudioRateWidth = 90;
const double exportAudioDepthWidth = 70;
const double exportAudioLayoutWidth = 80;
const double exportAudioBitRateWidth = 90;

/// The resample-quality face at the end of the Resize row, drawn dead in the
/// mockup itself: the export path has one resampler.
const double exportResampleWidth = 80;

/// How long a section's box stays lit after its tab is clicked.
const Duration exportSectionFlash = Duration(milliseconds: 600);

/// How far below the body's top edge a group's own top may be and still count
/// as the section you are looking at.
const double exportSpyBias = 24;

/// The AAC rates offered, bits per second. 320 leads because it is the
/// delivery-preset rate (docs/06 §7.5); the rest are the customary steps down.
const List<int> _audioRates = [320000, 256000, 192000, 128000];

/// What the export writes. *Still* is not here and will not be: a still is an
/// image sequence of one frame, which the span already says (K-485).
enum ExportOutputType { video, imageSequence, audioOnly }

/// A section of the page, and the tab that names it. The Composition group has
/// no tab of its own — it reads as part of Output, which is the group above it.
enum ExportSection { output, time, picture, colour, audio, metadata }

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

/// H.264 first because it is the delivery default; the sequences and the
/// sound-only containers after it, because they are the specialist choices.
List<_Format> get _formats => [
      _Format('h264', l10n.formatH264, 'mp4', l10n.formatMp4Picker,
          ExportOutputType.video),
      _Format('hevc', l10n.formatHevc, 'mp4', l10n.formatMp4Picker,
          ExportOutputType.video),
      _Format('png', l10n.formatPngSequence, 'png', l10n.formatPngPicker,
          ExportOutputType.imageSequence),
      _Format('tiff', l10n.formatTiffSequence, 'tiff', l10n.formatTiffPicker,
          ExportOutputType.imageSequence),
      _Format('m4a', l10n.formatM4a, 'm4a', l10n.formatM4aPicker,
          ExportOutputType.audioOnly),
      _Format('wav', l10n.formatWav, 'wav', l10n.formatWavPicker,
          ExportOutputType.audioOnly),
    ];

/// One field written into the container, in the order the section lists them.
/// The five classic fields are prefilled with their own names; a field of one's
/// own carries its key in a well of its own, because the key is FFmpeg's word
/// and not a translatable one.
class _MetaField {
  final String key;
  final String? label;
  final TextEditingController value;
  _MetaField(this.key, this.label, String initial)
      : value = TextEditingController(text: initial);
}

/// The classic fields, in the order docs/06 §7.4 lists them — FFmpeg's own
/// keys, because our own words here would write a field nothing reads.
List<(String, String)> get _standardMetadata => [
      ('title', l10n.metadataTitle),
      ('artist', l10n.metadataAuthor),
      ('copyright', l10n.metadataCopyright),
      ('comment', l10n.metadataComment),
      ('creation_time', l10n.metadataCreated),
    ];

Future<void> showExportDialogFrb({
  required BuildContext context,
  required CompositionReference comp,
  Future<String?> Function()? picker,
  List<double>? region,
}) {
  // The Viewer's region of interest, read here rather than inside the dialog:
  // the modal is an overlay, and what is above it in the tree is not what is
  // above the window that opened it.
  List<double>? roi = region;
  if (roi == null) {
    try {
      roi = context.read<LumitUiState>().regionOfInterest;
    } catch (_) {
      roi = null;
    }
  }
  return showLumitModal<void>(
    context: context,
    id: 'export',
    builder: (close) => _ExportDialog(
      comp: comp,
      picker: picker,
      region: roi,
      onClose: () => close(null),
    ),
  );
}

class _ExportDialog extends StatefulWidget {
  final CompositionReference comp;
  final Future<String?> Function()? picker;
  final List<double>? region;
  final VoidCallback onClose;

  const _ExportDialog({
    required this.comp,
    required this.picker,
    required this.region,
    required this.onClose,
  });

  @override
  State<_ExportDialog> createState() => _ExportDialogState();
}

class _ExportDialogState extends State<_ExportDialog> {
  // ---- where the reader is -------------------------------------------------

  ExportSection _section = ExportSection.output;
  ExportSection? _flash;
  Timer? _flashTimer;
  final ScrollController _scroll = ScrollController();
  final GlobalKey _bodyKey = GlobalKey();
  final Map<ExportSection, GlobalKey> _sectionKeys = {
    for (final section in ExportSection.values) section: GlobalKey(),
  };

  // ---- what is being asked for ---------------------------------------------

  String _preset = '';
  List<BridgeExportPresetEntry> _presets = const [];
  bool _naming = false;
  final TextEditingController _presetName = TextEditingController();

  _Format _format = _formats.first;
  int _bitrate = 0;
  int _peak = 0;
  bool _autoBitrate = true;
  double _fps = 60;
  bool _ownRate = false;
  _Span _span = _Span.workArea;
  int _rangeStart = 0;
  int _rangeEnd = 1;
  bool _audio = true;
  int _audioRate = _audioRates.first;
  String? _path;
  bool _openFolder = false;
  bool _makeANoise = false;
  String? _refused;

  /// The output size: a fraction of the comp's, or the pixels typed into the
  /// Resize row when it is ticked.
  int _divisor = 1;
  bool _resize = false;
  bool _lockAspect = true;
  int _resizeWidth = 1920;
  int _resizeHeight = 1080;

  /// The render settings (`RenderOptions`) and what the picture carries.
  int _quality = 1;
  bool _effects = true;
  bool _honourSolo = true;
  bool _diskCache = false;
  int _depth = 8;
  bool _alphaChannel = false;
  bool _straightAlpha = false;

  int _cropTop = 0;
  int _cropLeft = 0;
  int _cropBottom = 0;
  int _cropRight = 0;
  bool _useRegion = false;

  late final List<_MetaField> _metadata = [
    for (final (key, label) in _standardMetadata) _MetaField(key, label, ''),
  ];

  /// The engine's four answers, read whenever a field changes and never in
  /// `build`.
  BridgeFormatCaps _caps = BridgeFormatCaps(
    video: true,
    audio: true,
    alpha: false,
    depths: Uint32List.fromList(const [8]),
    bitRate: true,
    audioBitRate: true,
    metadata: true,
  );
  String _check = '';
  BridgeCrop? _crop;
  int _bitrateBps = 0;

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
    _presets = exportPresetList();
    // The dialog opens on the first built-in — *Master*, the composition's own
    // frame at a worked-out bitrate — rather than a blank Custom: a fresh
    // dialog showing "Custom" with a bit rate of 0 read as broken.
    if (_presets.isNotEmpty) _applyPreset(_presets.first.name);
    _recompute();
    _scroll.addListener(_spy);
  }

  @override
  void dispose() {
    _flashTimer?.cancel();
    _scroll.dispose();
    _presetName.dispose();
    for (final field in _metadata) {
      field.value.dispose();
    }
    super.dispose();
  }

  // ---- what the fields add up to -------------------------------------------

  bool get _images => _format.type == ExportOutputType.imageSequence;

  /// The span in comp frames, end exclusive.
  (int, int) get _range => switch (_span) {
        _Span.workArea => (_workStart, _workEnd),
        _Span.wholeComp => (0, _compFrames),
        _Span.custom => (_rangeStart, _rangeEnd),
      };

  /// The size the file will be, in pixels: the comp's own (less whatever the
  /// crop takes off) divided by the resolution chosen, unless Resize says
  /// otherwise.
  (int, int) get _outputSize {
    if (_resize) return (_resizeWidth, _resizeHeight);
    final crop = _crop;
    final width = crop?.width ?? _compWidth;
    final height = crop?.height ?? _compHeight;
    return (
      (width / _divisor).round().clamp(1, 16384),
      (height / _divisor).round().clamp(1, 16384),
    );
  }

  double get _rate => _fps <= 0 ? _compFps : _fps;

  /// The footer's line: what pressing the button would produce.
  String get _summary {
    final (start, end) = _range;
    final frames = (end - start).clamp(0, 1 << 30);
    final seconds = _rate <= 0 ? 0.0 : frames / _rate;
    final (width, height) = _outputSize;
    final line = l10n.exportSummary(
      '$frames',
      seconds.toStringAsFixed(1),
      '$width',
      '$height',
      _formatRate(_rate),
    );
    // A bit rate the encoder chose for itself is not a number this dialog may
    // multiply out, so the estimate simply is not offered — a file with a
    // picture whose rate nobody set has no size anyone can state.
    if (_caps.video && _bitrateBps <= 0) return line;
    final bits = (_caps.video ? _bitrateBps : 0) +
        (_audio && _caps.audio ? _audioRate : 0);
    if (bits <= 0) return line;
    final gigabytes = bits * seconds / 8 / 1e9;
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
      bitrateMbps: _bitrate,
      peakMbps: _peak,
      bitrateAuto: _autoBitrate,
      fps: _fps,
      rangeStartFrame: start,
      rangeEndFrame: end,
      includeAudio: _audio,
      audioBitRate: _audioRate,
      depth: _depth,
      alphaChannel: _alphaChannel,
      straightAlpha: _straightAlpha,
      colourSpace: '',
      cropTop: _cropTop,
      cropLeft: _cropLeft,
      cropBottom: _cropBottom,
      cropRight: _cropRight,
      useRegionOfInterest: _useRegion,
      region: Float64List.fromList(widget.region ?? const []),
      metadata: [
        for (final field in _metadata)
          if (field.value.text.trim().isNotEmpty)
            BridgeMetadataField(key: field.key, value: field.value.text.trim()),
      ],
      qualityDivisor: _quality,
      diskCacheReadOnly: _diskCache,
      effects: _effects,
      honourSolo: _honourSolo,
      makeANoise: _makeANoise,
      openFolder: _openFolder,
    );
  }

  /// Every field change comes through here: the state is written, and then the
  /// four answers only the engine can give are read again — the capability row
  /// for the chosen format, whatever it refuses, the crop it resolves and the
  /// bitrate it will run at. Doing it here rather than in `build` is what keeps
  /// the rebuild path free of bridge calls.
  void _edit(VoidCallback change) {
    setState(() {
      change();
      _recompute();
    });
  }

  void _recompute() {
    _caps = exportFormatCaps(codec: _format.key);
    final spec = _spec;
    _crop = exportCropFor(
      spec: spec,
      compWidth: _compWidth,
      compHeight: _compHeight,
    );
    final (width, height) = _outputSize;
    _bitrateBps = exportResolvedBitrate(
            spec: spec, width: width, height: height, fps: _rate)
        .toInt();
    _check = exportSpecCheck(spec: spec);
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
        dialogTabs<ExportSection>(
          t,
          tabs: [
            (ExportSection.output, l10n.exportGroupOutput),
            (ExportSection.time, l10n.exportGroupTime),
            (ExportSection.picture, l10n.exportGroupPicture),
            (ExportSection.colour, l10n.exportGroupColour),
            (ExportSection.audio, l10n.exportGroupAudio),
            (ExportSection.metadata, l10n.exportGroupMetadata),
          ],
          current: _section,
          onPick: _goToSection,
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
            key: _bodyKey,
            controller: _scroll,
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
          summary: _refused ?? (_check.isNotEmpty ? _check : _summary),
          keyPrefix: 'export',
          actions: [
            HouseButton(
              key: const ValueKey('export-add-to-queue'),
              onPressed: _canQueue ? () => _queue(start: false) : null,
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
              onPressed: _canQueue ? () => _queue(start: true) : null,
              child: Text(l10n.exportAction),
            ),
          ],
        ),
      ],
    );
  }

  /// Somewhere to write, and a spec the format will actually carry. A refusal
  /// stands in the footer where the summary was, so the reason is read before
  /// the button is missed.
  bool get _canQueue => _path != null && _check.isEmpty;

  /// The page, in the order it reads. Composition follows Output and belongs to
  /// its tab; every other group is a section of its own.
  List<Widget> _groups(LumitTheme t) => [
        _section_(ExportSection.output, _outputGroup(t)),
        _compositionGroup(t),
        _section_(ExportSection.time, _timeGroup(t)),
        _section_(ExportSection.picture, _pictureGroup(t)),
        _section_(ExportSection.colour, _colourGroup(t)),
        _section_(ExportSection.audio, _audioGroup(t)),
        _section_(ExportSection.metadata, _metadataGroup(t)),
      ];

  /// One section of the page: it carries the key the tab scrolls to, and it
  /// claims the tab strip the moment anything inside it is touched.
  Widget _section_(ExportSection section, Widget group) => KeyedSubtree(
        key: _sectionKeys[section],
        child: Listener(
          onPointerDown: (_) {
            if (_section != section) setState(() => _section = section);
          },
          child: group,
        ),
      );

  // ---- where the reader is -------------------------------------------------

  /// The tab strip follows the page: the section in force is the last one whose
  /// top has passed the body's own top edge.
  void _spy() {
    final body = _bodyKey.currentContext?.findRenderObject();
    if (body is! RenderBox) return;
    final top = body.localToGlobal(Offset.zero).dy;
    ExportSection current = ExportSection.values.first;
    for (final section in ExportSection.values) {
      final box = _sectionKeys[section]?.currentContext?.findRenderObject();
      if (box is! RenderBox) continue;
      if (box.localToGlobal(Offset.zero).dy - top <= exportSpyBias) {
        current = section;
      }
    }
    if (current != _section) setState(() => _section = current);
  }

  /// Clicking a tab: scroll its section into view when it is not already fully
  /// visible, and light its box for a moment so the eye lands on it.
  void _goToSection(ExportSection section) {
    setState(() {
      _section = section;
      _flash = section;
    });
    _flashTimer?.cancel();
    _flashTimer = Timer(exportSectionFlash, () {
      if (mounted) setState(() => _flash = null);
    });
    final target = _sectionKeys[section]?.currentContext;
    if (target == null || _fullyVisible(target)) return;
    Scrollable.ensureVisible(
      target,
      duration: const Duration(milliseconds: 180),
      curve: Curves.easeOut,
    );
  }

  /// Whether a section's whole box is inside the body's window. A section
  /// already in front of the reader is not worth moving the page for.
  bool _fullyVisible(BuildContext target) {
    final body = _bodyKey.currentContext?.findRenderObject();
    final box = target.findRenderObject();
    if (body is! RenderBox || box is! RenderBox) return true;
    final top = body.localToGlobal(Offset.zero).dy;
    final boxTop = box.localToGlobal(Offset.zero).dy;
    return boxTop >= top - 0.5 &&
        boxTop + box.size.height <= top + body.size.height + 0.5;
  }

  // ---- the groups ----------------------------------------------------------

  Widget _outputGroup(LumitTheme t) => _group(
        t,
        ExportSection.output,
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
              Row(children: [
                SizedBox(
                  width: exportPresetDropdown,
                  height: dialogControlHeight,
                  child: BareDropdown<String>(
                    key: const ValueKey('export-preset'),
                    value: _preset,
                    options: ['', ..._presets.map((p) => p.name)],
                    label: (p) => p.isEmpty ? l10n.custom : p,
                    onChanged: _applyPreset,
                  ),
                ),
                const SizedBox(width: 6),
                SizedBox(
                  width: exportPresetEditButton,
                  height: dialogControlHeight,
                  child: HouseButton(
                    key: const ValueKey('export-preset-edit'),
                    onPressed: _editPreset,
                    child: Text(l10n.exportPresetEdit, style: t.body),
                  ),
                ),
                const SizedBox(width: 6),
                Expanded(
                  child: SizedBox(
                    height: dialogControlHeight,
                    child: HouseButton(
                      key: const ValueKey('export-preset-save-as'),
                      onPressed: () => _nameAPreset(''),
                      child: Text(l10n.exportPresetSaveAs, style: t.body),
                    ),
                  ),
                ),
              ]),
              labelColumn: exportLabelColumnPaired,
            ),
          ),
          if (_naming) _presetNameRow(t),
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
          // The drawing's dropdown is two ticks instead (K-485): *when done* is
          // two independent answers, and a long export left running wants the
          // sound and the folder both.
          _row(
            t,
            l10n.exportWhenDone,
            Row(children: [
              _tick(t, 'export-make-a-noise', l10n.exportMakeANoise,
                  _makeANoise, (on) => _edit(() => _makeANoise = on)),
              const SizedBox(width: 16),
              _tick(t, 'export-open-folder', l10n.exportOpenFolder, _openFolder,
                  (on) => _edit(() => _openFolder = on)),
            ]),
          ),
        ],
      );

  /// Naming a preset: the one row *Edit* and *Save as…* both open, because
  /// saving over a name and renaming into it are the same act (the store
  /// replaces a preset of that name in its own row).
  Widget _presetNameRow(LumitTheme t) => _row(
        t,
        l10n.exportPresetName,
        Row(children: [
          HouseTextField(
            key: const ValueKey('export-preset-name'),
            controller: _presetName,
            width: 180,
            autofocus: true,
            fill: t.surface0,
            onSubmitted: (_) => _savePreset(),
          ),
          const SizedBox(width: 6),
          SizedBox(
            height: dialogControlHeight,
            child: HouseButton(
              key: const ValueKey('export-preset-save'),
              onPressed: _savePreset,
              child: Text(l10n.save, style: t.body),
            ),
          ),
          const SizedBox(width: 6),
          if (_presets.any((p) => p.name == _preset && !p.readOnly))
            SizedBox(
              height: dialogControlHeight,
              child: HouseButton(
                key: const ValueKey('export-preset-delete'),
                onPressed: _deletePreset,
                child: Text(l10n.delete, style: t.body),
              ),
            ),
          const SizedBox(width: 6),
          SizedBox(
            height: dialogControlHeight,
            child: HouseButton(
              key: const ValueKey('export-preset-cancel'),
              onPressed: () => setState(() => _naming = false),
              child: Text(l10n.cancel, style: t.body),
            ),
          ),
        ]),
      );

  Widget _compositionGroup(LumitTheme t) => _group(
        t,
        ExportSection.output,
        l10n.exportGroupComposition,
        [
          _columns(
            _row(
              t,
              l10n.exportQuality,
              dialogDropdown<int>(
                t,
                id: 'export-quality',
                value: _quality,
                options: const [1, 2, 3, 4],
                label: _qualityLabel,
                onChanged: (q) => _edit(() => _quality = q),
              ),
            ),
            _row(
              t,
              l10n.exportEffects,
              dialogDropdown<bool>(
                t,
                id: 'export-effects',
                value: _effects,
                options: const [true, false],
                label: (on) =>
                    on ? l10n.exportCurrentSettings : l10n.exportAllOff,
                onChanged: (on) => _edit(() => _effects = on),
              ),
              labelColumn: exportLabelColumnPaired,
            ),
          ),
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
                onChanged: (d) => _edit(() => _divisor = d),
              ),
            ),
            _row(
              t,
              l10n.exportSoloSwitches,
              dialogDropdown<bool>(
                t,
                id: 'export-solo',
                value: _honourSolo,
                options: const [true, false],
                label: (on) =>
                    on ? l10n.exportCurrentSettings : l10n.exportAllOff,
                onChanged: (on) => _edit(() => _honourSolo = on),
              ),
              labelColumn: exportLabelColumnPaired,
            ),
          ),
          // Proxies and guide layers are document subsystems before they are
          // export settings, and neither exists yet (K-479). The drawing shows
          // them, so they are drawn — dead, named, and with a reason.
          _columns(
            _row(
              t,
              l10n.exportProxies,
              _dead(t, 'export-proxies', l10n.exportProxiesNone,
                  l10n.tipExportNeedsProxies),
            ),
            _row(
              t,
              l10n.exportGuideLayers,
              _dead(t, 'export-guide-layers', l10n.exportAllOff,
                  l10n.tipExportNeedsGuideLayers),
              labelColumn: exportLabelColumnPaired,
            ),
          ),
          _columns(
            _row(
              t,
              l10n.exportDiskCache,
              dialogDropdown<bool>(
                t,
                id: 'export-disk-cache',
                value: _diskCache,
                options: const [false, true],
                label: (on) =>
                    on ? l10n.exportDiskCacheReadOnly : l10n.exportDiskCacheOff,
                onChanged: (on) => _edit(() => _diskCache = on),
              ),
            ),
            _row(
              t,
              l10n.exportColourDepth,
              _depthDropdown(t, 'export-colour-depth'),
              labelColumn: exportLabelColumnPaired,
            ),
          ),
        ],
      );

  Widget _timeGroup(LumitTheme t) {
    final (start, end) = _range;
    return _group(
      t,
      ExportSection.time,
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
              onChanged: (span) => _edit(() {
                _span = span;
                if (span == _Span.custom) {
                  _rangeStart = start;
                  _rangeEnd = end;
                }
              }),
            ),
          ),
          // The value well is a fixed 56 and the list beside it takes whatever
          // the row has left, so the rate is always readable and the column's
          // controls keep one left edge and one right (K-485).
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
                  onChanged: (own) => _edit(() {
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
                        onChanged: (v) => _edit(() => _fps = v.toDouble()),
                      )
                    : _well(t, _formatRate(_compFps),
                        key: const ValueKey('export-fps'),
                        tone: t.textDisabled),
              ),
            ]),
            labelColumn: exportLabelColumnPaired,
          ),
        ),
        // Both are comp-wide settings the export path has no override for: an
        // export renders what the composition renders. Drawn dead rather than
        // live, for the same reason proxies are.
        _columns(
          _row(
            t,
            l10n.exportMotionBlur,
            _dead(t, 'export-motion-blur', l10n.exportOnForCheckedLayers,
                l10n.tipExportCompSetting),
          ),
          _row(
            t,
            l10n.exportRetimeBlend,
            _dead(t, 'export-retime-blend', l10n.exportOnForCheckedLayers,
                l10n.tipExportCompSetting),
            labelColumn: exportLabelColumnPaired,
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
                  onChanged: (v) => _edit(() {
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
                  onChanged: (v) => _edit(() {
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
            _timecode(start, _rate),
            _timecode(end, _rate),
            _timecode(end - start, _rate),
          ),
          key: const ValueKey('export-time-reading'),
        ),
      ],
    );
  }

  Widget _pictureGroup(LumitTheme t) {
    final (width, height) = _outputSize;
    final crop = _crop;
    return _group(
      t,
      ExportSection.picture,
      l10n.exportGroupPicture,
      [
        _columns(
          _row(
            t,
            l10n.exportChannels,
            dialogDropdown<bool>(
              t,
              id: 'export-channels',
              value: _alphaChannel,
              options: const [false, true],
              label: (alpha) =>
                  alpha ? l10n.exportChannelsRgbAlpha : l10n.exportChannelsRgb,
              onChanged:
                  _caps.alpha ? (a) => _edit(() => _alphaChannel = a) : null,
            ),
          ),
          _row(
            t,
            l10n.exportAlpha,
            dialogDropdown<bool>(
              t,
              id: 'export-alpha',
              value: _straightAlpha,
              options: const [false, true],
              label: (straight) => straight
                  ? l10n.exportAlphaStraight
                  : l10n.exportAlphaPremultiplied,
              onChanged: _caps.alpha && _alphaChannel
                  ? (s) => _edit(() => _straightAlpha = s)
                  : null,
            ),
            labelColumn: exportLabelColumnPaired,
          ),
        ),
        _columns(
          _row(t, l10n.exportDepth, _depthDropdown(t, 'export-depth')),
          _row(
            t,
            l10n.exportBitRate,
            Row(children: [
              _tick(
                t,
                'export-bitrate-auto',
                l10n.exportBitRateAuto,
                _autoBitrate,
                _caps.bitRate ? (on) => _edit(() => _autoBitrate = on) : null,
              ),
              const SizedBox(width: 8),
              Expanded(
                child: SizedBox(
                  height: dialogControlHeight,
                  child: _autoBitrate || !_caps.bitRate
                      ? _well(
                          t,
                          _bitrateBps > 0
                              ? '${(_bitrateBps / 1e6).round()} Mb/s'
                              : '—',
                          key: const ValueKey('export-bitrate'),
                          tone: t.textDisabled,
                        )
                      : DragValueField(
                          key: const ValueKey('export-bitrate'),
                          value: _bitrate,
                          min: 0,
                          max: 400,
                          suffix: _bitrate == 0 ? null : ' Mb/s',
                          fill: t.surface0,
                          onChanged: (v) => _edit(() {
                            _bitrate = v.toInt();
                            // A rate of one's own has no preset peak behind it.
                            _peak = 0;
                          }),
                        ),
                ),
              ),
            ]),
            labelColumn: exportLabelColumnPaired,
          ),
        ),
        _row(
          t,
          l10n.exportResize,
          Row(children: [
            HouseCheckbox(
              key: const ValueKey('export-resize'),
              value: _resize,
              onChanged: (on) => _edit(() => _resize = on),
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
              onChanged: (on) => _edit(() => _lockAspect = on),
            ),
            const SizedBox(width: 6),
            Flexible(
              child: Text(l10n.exportLockAspect,
                  style: t.body.copyWith(color: t.textMuted),
                  overflow: TextOverflow.ellipsis),
            ),
            const Spacer(),
            SizedBox(
              width: exportResampleWidth,
              height: dialogControlHeight,
              child: _dead(t, 'export-resample', l10n.exportResampleHigh,
                  l10n.tipExportOneResampler),
            ),
          ]),
        ),
        _row(
          t,
          l10n.exportCrop,
          Row(children: [
            _cropWell(t, 'top', l10n.exportCropTop, _cropTop,
                (v) => _cropTop = v, _compHeight),
            _cropWell(t, 'left', l10n.exportCropLeft, _cropLeft,
                (v) => _cropLeft = v, _compWidth),
            _cropWell(t, 'bottom', l10n.exportCropBottom, _cropBottom,
                (v) => _cropBottom = v, _compHeight),
            _cropWell(t, 'right', l10n.exportCropRight, _cropRight,
                (v) => _cropRight = v, _compWidth),
            const SizedBox(width: 8),
            HouseCheckbox(
              key: const ValueKey('export-use-region'),
              value: _useRegion,
              onChanged: (on) => _edit(() => _useRegion = on),
            ),
            const SizedBox(width: 6),
            Flexible(
              child: Text(l10n.exportUseRegionOfInterest,
                  style: t.body.copyWith(color: t.textMuted),
                  overflow: TextOverflow.ellipsis),
            ),
            const SizedBox(width: 8),
            Flexible(
              child: Align(
                alignment: Alignment.centerRight,
                child: Text(
                  l10n.exportFinalSize('$width', '$height'),
                  key: const ValueKey('export-final-size'),
                  style: dialogMono(t),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ),
          ]),
        ),
        if (_images)
          _reading(
            t,
            l10n.exportImageSequenceNote(_format.extension.toUpperCase()),
          ),
        if (crop != null &&
            (crop.width < _compWidth || crop.height < _compHeight))
          _reading(
            t,
            l10n.exportCropReading('${crop.width}', '${crop.height}'),
            key: const ValueKey('export-crop-reading'),
          ),
      ],
    );
  }

  /// Colour: the one transform this build performs, and the honest shape of the
  /// one it does not. An OCIO output space is modelled all the way through
  /// (`ColourSpace::Ocio`) and refused before a frame is rendered, so the row
  /// names it and leaves it dead rather than pretending the choice is there.
  Widget _colourGroup(LumitTheme t) => _group(
        t,
        ExportSection.colour,
        l10n.exportGroupColour,
        [
          _row(
            t,
            l10n.exportColourSpace,
            dialogDropdown<String>(
              t,
              id: 'export-colour-space',
              value: '',
              options: const [''],
              label: (_) => l10n.exportColourSpaceSrgb,
              onChanged: (_) {},
            ),
          ),
          _row(
            t,
            l10n.exportColourManagement,
            _dead(
                t, 'export-ocio', l10n.exportColourOcio, l10n.tipExportAfterV1),
          ),
          _reading(t, l10n.exportColourNote),
        ],
      );

  Widget _audioGroup(LumitTheme t) => _group(
        t,
        ExportSection.audio,
        l10n.exportGroupAudio,
        [
          _row(
            t,
            l10n.exportGroupAudio,
            Row(children: [
              SizedBox(
                width: exportAudioSourceWidth,
                height: dialogControlHeight,
                child: BareDropdown<bool>(
                  key: const ValueKey('export-audio'),
                  value: _audio,
                  options: const [true, false],
                  label: (on) =>
                      on ? l10n.exportAudioAuto : l10n.exportAudioOff,
                  onChanged:
                      _caps.audio ? (on) => _edit(() => _audio = on) : null,
                ),
              ),
              const SizedBox(width: 6),
              // The engine mixes every export at 48 kHz, sixteen bits, stereo
              // (`EXPORT_AUDIO_RATE`): three readings in the drawing's own
              // faces, dead because there is nothing to choose.
              SizedBox(
                width: exportAudioRateWidth,
                height: dialogControlHeight,
                child: _dead(t, 'export-audio-sample-rate', l10n.exportAudio48k,
                    l10n.tipExportOneMix),
              ),
              const SizedBox(width: 6),
              SizedBox(
                width: exportAudioDepthWidth,
                height: dialogControlHeight,
                child: _dead(t, 'export-audio-depth', l10n.exportAudio16Bit,
                    l10n.tipExportOneMix),
              ),
              const SizedBox(width: 6),
              SizedBox(
                width: exportAudioLayoutWidth,
                height: dialogControlHeight,
                child: _dead(t, 'export-audio-layout', l10n.exportAudioStereo,
                    l10n.tipExportOneMix),
              ),
              const Spacer(),
              SizedBox(
                width: exportAudioBitRateWidth,
                height: dialogControlHeight,
                child: BareDropdown<int>(
                  key: const ValueKey('export-audio-rate'),
                  value: _audioRate,
                  options: _audioRates,
                  label: (r) => '${r ~/ 1000} kb/s',
                  onChanged: _caps.audioBitRate && _audio
                      ? (r) => _edit(() => _audioRate = r)
                      : null,
                ),
              ),
            ]),
          ),
        ],
      );

  /// Metadata: an ordered key/value set, because the order lands in the file
  /// and an export is deterministic. The five classic fields lead, named; a
  /// field of one's own carries FFmpeg's own key in a well beside its value.
  Widget _metadataGroup(LumitTheme t) => _group(
        t,
        ExportSection.metadata,
        l10n.exportGroupMetadata,
        [
          for (final (index, field) in _metadata.indexed)
            _row(
              t,
              field.label ?? '',
              Row(children: [
                if (field.label == null) ...[
                  _well(t, field.key, tone: t.textMuted),
                  const SizedBox(width: 6),
                ],
                Expanded(
                  child: HouseTextField(
                    key: ValueKey<String>('export-metadata-$index'),
                    controller: field.value,
                    width: double.infinity,
                    fill: t.surface0,
                    onSubmitted: (_) => _edit(() {}),
                    submitOnLostFocus: true,
                  ),
                ),
                const SizedBox(width: 6),
                SizedBox(
                  height: dialogControlHeight,
                  child: HouseButton(
                    key: ValueKey<String>('export-metadata-remove-$index'),
                    onPressed: _caps.metadata
                        ? () => _edit(() {
                              _metadata.removeAt(index).value.dispose();
                            })
                        : null,
                    child: Text(l10n.exportMetadataRemove, style: t.body),
                  ),
                ),
              ]),
            ),
          _row(
            t,
            '',
            Row(children: [
              SizedBox(
                height: dialogControlHeight,
                child: HouseButton(
                  key: const ValueKey('export-metadata-add'),
                  onPressed: _caps.metadata
                      ? () => _edit(() => _metadata.add(_MetaField(
                          'field_${_metadata.length + 1}', null, '')))
                      : null,
                  child: Text(l10n.exportMetadataAdd, style: t.body),
                ),
              ),
            ]),
          ),
          if (!_caps.metadata) _reading(t, l10n.exportMetadataUnsupported),
        ],
      );

  // ---- the pieces a row is made of -----------------------------------------

  /// A titled group that knows which section it belongs to, so a tab can light
  /// it briefly when it is jumped to.
  Widget _group(
    LumitTheme t,
    ExportSection section,
    String title,
    List<Widget> rows,
  ) =>
      dialogGroup(
        t,
        title,
        rows,
        key: ValueKey<String>('export-group-${title.toLowerCase()}'),
        highlighted: _flash == section,
      );

  Widget _row(LumitTheme t, String label, Widget control,
          {double labelColumn = exportLabelColumn}) =>
      dialogRow(
        t,
        label,
        control,
        labelColumn: labelColumn,
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

  /// A tick with its word beside it.
  Widget _tick(LumitTheme t, String id, String label, bool value,
          ValueChanged<bool>? onChanged) =>
      Row(mainAxisSize: MainAxisSize.min, children: [
        HouseCheckbox(
          key: ValueKey<String>(id),
          value: value,
          onChanged: onChanged ?? (_) {},
        ),
        const SizedBox(width: 6),
        Text(label,
            style: t.body.copyWith(
                color: onChanged == null ? t.textDisabled : t.textMuted)),
      ]);

  /// A control the engine cannot back: the drawing's own face, drawn dead, with
  /// a reason on hover. Not absent — the drawing shows it — and not live, which
  /// would be a switch that wrote nothing.
  Widget _dead(LumitTheme t, String id, String label, String reason) =>
      LumitTooltip(
        message: reason,
        child: dialogDropdown<String>(
          t,
          id: id,
          value: label,
          options: [label],
          label: (v) => v,
          onChanged: null,
        ),
      );

  /// The colour depth, offered only where the format carries more than one.
  Widget _depthDropdown(LumitTheme t, String id) => dialogDropdown<int>(
        t,
        id: id,
        value: _depth,
        options: _caps.depths.isEmpty
            ? [_depth]
            : _caps.depths.map((d) => d.toInt()).toList(),
        label: (d) => d >= 16 ? l10n.exportDepth16 : l10n.exportDepth8,
        onChanged:
            _caps.depths.length > 1 ? (d) => _edit(() => _depth = d) : null,
      );

  /// One crop inset: its mark and its well, in pixels at composition size
  /// (K-419).
  Widget _cropWell(LumitTheme t, String id, String mark, int value,
          ValueChanged<int> set, int limit) =>
      Row(mainAxisSize: MainAxisSize.min, children: [
        Padding(
          padding: const EdgeInsets.only(right: 4),
          child: Text(mark, style: dialogMono(t)),
        ),
        SizedBox(
          width: exportCropWell,
          height: dialogControlHeight,
          child: DragValueField(
            key: ValueKey<String>('export-crop-$id'),
            value: value,
            min: 0,
            max: (limit - 1).clamp(0, 16384),
            fill: t.surface0,
            onChanged: (v) => _edit(() => set(v.toInt())),
          ),
        ),
        const SizedBox(width: 6),
      ]);

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
            ExportOutputType.audioOnly =>
              l10n.exportTypeAudioOnly.toUpperCase(),
          },
          style: on ? t.kickerOn : t.kicker,
        ),
      ),
    );
  }

  String _qualityLabel(int divisor) => switch (divisor) {
        1 => l10n.exportQualityFull,
        2 => l10n.exportQualityHalf,
        3 => l10n.exportQualityThird,
        _ => l10n.exportQualityQuarter,
      };

  // ---- what the controls do ------------------------------------------------

  void _setFormat(_Format format) => _edit(() {
        _format = format;
        // A chosen path keeps its stem but not its extension — `shot.mp4` as a
        // PNG sequence would write `shot.mp4.00001`-shaped nonsense.
        final path = _path;
        if (path != null) {
          _path = path.replaceFirst(
              RegExp(r'\.[A-Za-z0-9]+$'), '.${format.extension}');
        }
      });

  /// A preset fills every field it names — it is the whole settings payload,
  /// not a stamp on three of them (K-479's preset store).
  void _applyPreset(String name) {
    if (name.isEmpty) {
      _edit(() => _preset = '');
      return;
    }
    final stored = exportPresetGet(name: name);
    if (stored == null) {
      _edit(() => _preset = name);
      return;
    }
    _edit(() {
      _preset = name;
      _format = _formats.firstWhere(
        (f) => f.key == stored.codec,
        orElse: () => _formats.first,
      );
      if (stored.width > 0 && stored.height > 0) {
        _resize = true;
        _resizeWidth = stored.width;
        _resizeHeight = stored.height;
      }
      _bitrate = stored.bitrateMbps;
      _peak = stored.peakMbps;
      _autoBitrate = stored.bitrateAuto;
      if (stored.fps > 0) {
        _ownRate = true;
        _fps = stored.fps;
      }
      _audio = stored.includeAudio;
      _audioRate = stored.audioBitRate.toInt();
      _depth = stored.depth;
      _alphaChannel = stored.alphaChannel;
      _straightAlpha = stored.straightAlpha;
      _cropTop = stored.cropTop;
      _cropLeft = stored.cropLeft;
      _cropBottom = stored.cropBottom;
      _cropRight = stored.cropRight;
      _quality = stored.qualityDivisor;
      _diskCache = stored.diskCacheReadOnly;
      _effects = stored.effects;
      _honourSolo = stored.honourSolo;
    });
  }

  /// *Edit* names the preset in force, so saving replaces it. A built-in is
  /// read-only and says so rather than opening a field that cannot be used.
  void _editPreset() {
    final entry = _presets.where((p) => p.name == _preset).firstOrNull;
    if (entry != null && entry.readOnly) {
      setState(() => _refused = l10n.exportPresetReadOnly(entry.name));
      return;
    }
    _nameAPreset(_preset);
  }

  void _nameAPreset(String name) => setState(() {
        _refused = null;
        _naming = true;
        _presetName.text = name;
      });

  void _savePreset() {
    final name = _presetName.text.trim();
    if (name.isEmpty) return;
    try {
      exportPresetSave(name: name, spec: _spec);
    } catch (error) {
      setState(() => _refused = '$error');
      return;
    }
    setState(() {
      _naming = false;
      _refused = null;
      _presets = exportPresetList();
      _preset = name;
    });
  }

  void _deletePreset() {
    try {
      exportPresetDelete(name: _preset);
    } catch (error) {
      setState(() => _refused = '$error');
      return;
    }
    setState(() {
      _naming = false;
      _refused = null;
      _presets = exportPresetList();
      _preset = '';
    });
  }

  /// Editing one side of the output size, carrying the other with it when the
  /// aspect is locked.
  void _setResize({int? width, int? height}) => _edit(() {
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
    if (path != null) _edit(() => _path = path);
  }

  /// Queue the export, and show the queue. A refusal — a spec the encoder will
  /// not take — is shown in the footer where the summary was, rather than
  /// swallowed.
  void _queue({required bool start}) {
    final path = _path;
    if (path == null) return;
    setState(() => _refused = null);
    try {
      widget.comp.queueExport(spec: _spec, path: path, start: start);
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
