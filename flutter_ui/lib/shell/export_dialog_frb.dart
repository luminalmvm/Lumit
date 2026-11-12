// The export dialog, rebuilt to its drawing.
//
// **The shape is the approved drawing's.** A frame of 640: a kicker title
// strip naming the composition, a row of section tabs, the preset strip
// (a preset sets and saves every section, so it is chrome over the whole page
// rather than a row inside Output, and *Set as default* there remembers the
// preset in force between sessions), and a body of titled groups — Output,
// Composition, Time, Picture, Colour, Audio, Metadata — whose rows are a label
// in a fixed column with the control beside it, two to a line where the rows
// are short. The footer states the facts (frames, length, size, rate, an
// estimate of the file) and carries the two actions: *Add to queue* outlined,
// and EXPORT, the single filled action.
//
// **One page, and the tabs say where you are.** The dialog is a single
// scrolling page rather than six of them: the tab strip follows the section
// last touched or scrolled to, and clicking a tab scrolls its section into
// view when it is not fully visible and lights its box for a moment. A
// settings dialog where the same file's picture and sound are on different
// pages hides half of what an export is from the person deciding it.
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
// capability row, and a control the chosen format cannot honour is drawn
// **disabled** rather than left out or left live: an mp4 has no alpha channel
// and no sixteenth bit, a PNG sequence has no sound and no bitrate, a WAV has
// no picture at all, an AAC file has no sample width and a still states no
// colour space.
//
// **Every row the drawing shows is now a real setting.** Proxies, guide
// layers, motion blur, Retime blend, the resampler, the colour space and the
// three sound faces were drawn dead while the engine had nowhere to put them.
// The settings were built and carried over the seam, so each one writes into
// the spec now. The one face still drawn dead is OCIO, which is genuinely not
// in this build.
//
// **Nothing here crosses the bridge in `build`.** The capability row, the
// refusal, the crop and the bitrate are all the engine's answers, and all four
// are recomputed in `_edit` — the one place a field changes — so a rebuild
// costs nothing (the standing rebuild-path rule).

import 'dart:async';
import 'dart:io' show Directory, File;
import 'dart:typed_data';

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/services.dart' show Clipboard, ClipboardData;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/colour.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart' show BridgeLayerKind;
import 'package:lumit_flutter/src/rust/api/mixer.dart' show BridgeLoudnessTarget;
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/share.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart' show revealInFolder;
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../main.dart';
import '../panels/loudness_commands.dart'
    show loudnessPresets, loudnessTargetLabel, presetTruePeakDb;
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../state/file_dialogs.dart';
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/middle_ellipsis.dart';
import 'dialog_frame.dart';
import 'export_queue_frb.dart';
import 'settings_window_frb.dart';
import 'status_line_frb.dart';

part 'export_rows.dart';
part 'export_formats.dart';

/// The frame the drawing gives this dialog.
const double exportDialogWidth = 640;

/// The label column and the gap after it, and a row's own height — the
/// drawing's own, which are not the New composition dialog's. Each drawing
/// measures itself.
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

/// The room the frame's own bands take: the title strip and the footer, plus a
/// little air: what the body has to fit
/// inside when the window is short.
const double exportChromeHeight = 116;

const double exportButtonWidth = 72;
const double exportNumberWell = 56;
const double exportSizeWell = 64;

/// The three destination policies the export defaults store knows, by the
/// engine's own names. Here rather than in the Settings window because the
/// dialogue is what *acts* on them and Settings only sets them, and two places
/// spelling `"project"` differently would seed the wrong folder in silence.
const String exportDestinationAsk = 'ask';
const String exportDestinationProject = 'project';
const String exportDestinationFolder = 'folder';

/// The folder the export defaults write to, or empty when they ask each time
/// or the project has never been saved.
String exportDefaultFolder(BridgeExportDefaults defaults,
    {String? projectPath}) {
  final cut = projectPath?.lastIndexOf(RegExp(r'[/\\]')) ?? -1;
  return switch (defaults.destination) {
    exportDestinationFolder => defaults.folder,
    exportDestinationProject when cut > 0 => projectPath!.substring(0, cut),
    _ => '',
  };
}

/// Where the export defaults put a file of [codec] made with [preset] from a
/// composition called [compName], or null when they ask each time.
///
/// *Beside the project* is null too when the project at [projectPath] has never
/// been saved and so has no folder to be beside: a policy that cannot be
/// honoured is not a reason to invent a folder. The engine builds the name,
/// since the template's tokens are its own.
String? exportDefaultPath(BridgeExportDefaults defaults,
    {required String preset,
    required String compName,
    required String codec,
    String? projectPath}) {
  final folder = exportDefaultFolder(defaults, projectPath: projectPath);
  if (folder.isEmpty) return null;
  final name = exportPreset(
    preset: preset,
    compName: compName,
    template: defaults.filenameTemplate,
  ).defaultName;
  if (name.isEmpty) return null;
  final extension = _formats
      .firstWhere((f) => f.key == codec, orElse: () => _formats.first)
      .extension;
  // In the separator the folder itself is written with: a Windows folder
  // keeps its backslashes.
  final file = name.replaceFirst(RegExp(r'\.[A-Za-z0-9]+$'), '.$extension');
  if (folder.endsWith('/') || folder.endsWith('\\')) return '$folder$file';
  return '$folder${folder.contains('\\') ? '\\' : '/'}$file';
}

/// The tokens a filename template may carry — the exporter's own, and the
/// whole list: it substitutes these three and nothing else. Syntax rather than
/// prose, which is why they are spelled here and not in the arb: a translated
/// `{comp}` would stop being a token.
const String exportTokenComp = '{comp}';
const String exportTokenDate = '{date}';

/// A crop inset's well — four of them and their T · L · B · R marks fit the
/// row beside the region tick and the final-size reading.
const double exportCropWell = 40;

/// The loudness and true peak wells.
const double exportLoudnessWell = 76;

/// The audio row's five faces, as the drawing measures them.
const double exportAudioSourceWidth = 110;
const double exportAudioRateWidth = 90;
const double exportAudioDepthWidth = 70;
const double exportAudioLayoutWidth = 80;
const double exportAudioBitRateWidth = 90;

/// The resample-quality face at the end of the Resize row.
const double exportResampleWidth = 80;

/// A field-of-one's-own's **name** well in the Metadata section, standing in
/// the room the classic rows give their label: the key is FFmpeg's own word,
/// typed, and it sits where the eye already looks for the name of a row.
const double exportMetadataNameWidth = exportLabelColumn;

/// The AAC rates offered, bits per second. 320 leads because it is the
/// delivery-preset rate (docs/06 §7.5); the rest are the customary steps down.
const List<int> _audioRates = [320000, 256000, 192000, 128000];

/// What the export writes. *Still* is not here and will not be: a still is an
/// image sequence of one frame, which the span already says.
enum ExportOutputType { video, imageSequence, audioOnly }

/// A part of the window that folds away, under what every export needs.
enum ExportSection { composition, time, picture, colour, audio, metadata }

/// Which folds stand open. For the session, not the window: somebody who works
/// in Audio finds it where they left it.
final Set<ExportSection> exportOpenSections = {};

/// How much of the composition to write.
enum _Span { workArea, wholeComp, custom }

/// One field written into the container, in the order the section lists them.
/// The five classic fields are prefilled with their own names; a field of one's
/// own carries its key in a well of its own, because the key is FFmpeg's word
/// and not a translatable one.
///
/// **A field of one's own is named by whoever added it.** Its key started as
/// `field_6`, `field_7` and stayed there, which wrote a container full of
/// fields named after nothing. The key is a plain string on the way to FFmpeg,
/// so it is typed like any other — [name] is that editor, and it is null for
/// the five classics, whose keys are the standard's and not ours to change.
class _MetaField {
  /// The classic five: a fixed key and a translated label above it.
  final String? fixedKey;
  final String? label;

  /// A field of one's own: its key, typed. Null for the classics.
  final TextEditingController? name;

  final TextEditingController value;

  _MetaField(String key, this.label, String initial)
      : fixedKey = key,
        name = null,
        value = TextEditingController(text: initial);

  /// A field of one's own, seeded with a key to start from.
  _MetaField.own(String key)
      : fixedKey = null,
        label = null,
        name = TextEditingController(text: key),
        value = TextEditingController(text: '');

  /// What goes into the container. Trimmed, because a key with a space either
  /// side of it is a different key to FFmpeg and to nobody else.
  String get key => (fixedKey ?? name?.text ?? '').trim();

  void dispose() {
    name?.dispose();
    value.dispose();
  }
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
  int? queued,
}) {
  // The Viewer's region of interest, read here rather than inside the dialog:
  // the modal is an overlay, and what is above it in the tree is not what is
  // above the window that opened it.
  List<double>? roi = region;
  BridgeColourSummary colour = noColourConfig;
  ProjectReference? project;
  try {
    final ui = context.read<LumitUiState>();
    roi ??= ui.regionOfInterest;
    // The project's colour config, off the held summary rather than the file:
    // its space names fill the Colour section's dropdown.
    colour = ui.colourSummary;
  } catch (_) {
    roi = region;
  }
  try {
    project = context.read<LumitState>().project;
  } catch (_) {
    project = null;
  }
  return showLumitModal<void>(
    context: context,
    id: 'export',
    builder: (close) => _ExportDialog(
      comp: comp,
      project: project,
      colour: colour,
      picker: picker,
      region: roi,
      queued: queued,
      onClose: () => close(null),
    ),
  );
}

class _ExportDialog extends StatefulWidget {
  final CompositionReference comp;

  /// The project, for the one colour question only it can answer: whether a
  /// named space can actually be delivered right now. Null where the dialog
  /// was raised without one, which leaves the config section off.
  final ProjectReference? project;

  /// The colour config's names as the shell holds them.
  final BridgeColourSummary colour;
  final Future<String?> Function()? picker;
  final List<double>? region;

  /// The queue item being changed, when the dialogue was opened from its row
  /// in the queue. It opens on that item's settings and saves back to it.
  final int? queued;
  final VoidCallback onClose;

  const _ExportDialog({
    required this.comp,
    required this.project,
    required this.colour,
    required this.picker,
    required this.region,
    this.queued,
    required this.onClose,
  });

  @override
  State<_ExportDialog> createState() => _ExportDialogState();
}

class _ExportDialogState extends State<_ExportDialog> {
  final ScrollController _scroll = ScrollController();

  // ---- what is being asked for ---------------------------------------------

  String _preset = '';
  List<BridgeExportPresetEntry> _presets = const [];
  bool _naming = false;
  final TextEditingController _presetName = TextEditingController();

  /// The preset the defaults store names, so the strip can show whether what
  /// is in force is already the default. Empty when nothing was saved.
  String _defaultPreset = '';

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

  /// The three sound settings: the rate the mix is written at, the width of
  /// one written sample, and how many channels the file carries. Their
  /// defaults are what every export has always written.
  int _audioSampleRate = 48000;
  int _audioDepth = 16;
  int _audioChannels = 2;

  /// The rates the engine will take — three, and its own list rather than one
  /// repeated here. Read once, because `build` crosses no bridge.
  List<int> _sampleRates = const [48000];

  /// What the mix is brought to: null for off, [_targetPick] for the comp's
  /// loudness target, [_customPick] for [_customLufs], or a preset in LUFS.
  /// [_truePeakDb] is the ceiling it is held under.
  Object? _loudnessPick;
  double _customLufs = loudnessPresets.first;
  double _truePeakDb = presetTruePeakDb;
  static const Object _targetPick = #target;
  static const Object _customPick = #custom;

  /// A file of each bus's sound beside the main one.
  bool _stems = false;

  /// The comp's loudness target and whether it has a bus, read once.
  BridgeLoudnessTarget? _compTarget;
  bool _hasBus = false;

  /// The settings came from a preset of one's own or a queued export, so
  /// their loudness, Off included, stands over the comp's target. A built-in
  /// names no loudness, and the target stands.
  bool _ownLoudness = false;

  String? _path;

  /// The path the system's Save dialogue handed back, which has already asked
  /// about writing over a file. Any other path is asked about here.
  String? _picked;
  bool _openFolder = false;
  bool _makeANoise = false;
  String? _refused;

  /// The footage this composition uses that this computer has no original
  /// of, in a shared project. Empty everywhere else.
  List<BridgeFootageLack> _lacking = const [];

  /// The 3D models this comp draws that can't be read right now.
  List<String> _unread = const [];

  /// What the export does about [_lacking]: one of the three below, or the
  /// number of the person whose computer does the export instead.
  int _footage = _footageAsItIs;
  static const int _footageAsItIs = -1;
  static const int _footageFrames = -2;
  static const int _footageOriginals = -3;

  /// The footage is on its way, or someone else's computer is exporting,
  /// and this is what the footer says about it meanwhile.
  String? _waiting;
  Timer? _waitTimer;

  /// Whether [_refused] is the addon one, which is the only refusal with
  /// somewhere to send the reader: the Addons page.
  bool _refusedForAddon = false;

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

  /// A GIF's own choices: the dither (0 diffusion, 1 pattern, 2 none), a
  /// palette for each frame, and how many times it plays, 0 for forever.
  int _gifDither = 0;
  bool _gifPalettePerFrame = false;
  int _gifPlays = 0;

  /// Deliver from the proxies rather than the originals, and deliver the guide
  /// layers too. Both are off by default whatever the project is set to: a
  /// proxy switched on to work with is not a picture anybody meant to ship.
  bool _useProxies = false;
  bool _renderGuides = false;
  BridgeCaptionsOut _captions = BridgeCaptionsOut.burn;
  bool _hasCaptions = false;

  /// The Time overrides. Motion blur has three answers — the composition's
  /// own, on for every checked layer, off for all of them — and Retime blend
  /// has two, because Lumit has no comp-wide blending master for a third to
  /// speak to.
  int _motionBlur = 0;
  int _retimeBlend = 0;

  /// The filter a resized frame is resampled with. The empty string is
  /// *Fast* — bilinear, what every Lumit export has always used — and it
  /// stays the default, because changing it would silently alter what every
  /// stored preset writes.
  String _resample = '';

  /// The colour space the file is written and tagged in, as the engine's own
  /// stored name. Empty is sRGB / Rec. 709, which is a pass-through.
  String _colourSpace = '';

  /// Why each of the config's spaces cannot be delivered, for the ones that
  /// cannot: disabled, never hidden. Worked out once as the dialog opens — the
  /// config cannot change while it is up, and asking per rebuild would cross
  /// the bridge once per name per frame.
  final Map<String, String> _colourRefused = {};

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
    audio24Bit: false,
    colourSpaces: const [''],
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
      _hasCaptions = widget.comp.hasCaptions();
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
      _compTarget = widget.comp.loudnessTarget();
      _hasBus = widget.comp
          .getLayers()
          .any((l) => l.getInfo().kind == BridgeLayerKind.bus);
    } catch (_) {
      // A comp that cannot be read leaves the placeholder defaults; the export
      // itself will refuse with the engine's own words.
    }
    _sampleRates = exportAudioRates().map((r) => r.toInt()).toList();
    // Which of the config's spaces this project can actually deliver. A space
    // whose chain does not bake — a 3D look-up table asked for backwards, say —
    // is listed and dead rather than missing, so the reader is not left
    // hunting for a name they know the config has.
    final project = widget.project;
    if (project != null) {
      for (final space in widget.colour.spaces) {
        try {
          if (!project.canDeliverColourSpace(name: space)) {
            _colourRefused[space] = l10n.tipExportColourSpaceUnavailable;
          }
        } catch (_) {
          _colourRefused[space] = l10n.tipExportColourSpaceUnavailable;
        }
      }
    }
    _presets = exportPresetList();
    // What the last *Set as default* — or Settings ▸ Export — asked for.
    // Read once here: this is the one moment the store is consulted, and
    // `build` crosses no bridge.
    final defaults = exportDefaultsGet();
    _defaultPreset = defaults.preset;
    // The dialog opens on the named default, else on the first built-in —
    // *Master*, the composition's own frame at a worked-out bitrate — rather
    // than a blank Custom: a fresh dialog showing "Custom" with a bit rate of 0
    // read as broken. A default naming a preset that has since been deleted
    // falls through to the same place rather than opening on nothing.
    final named = _presets.any((p) => p.name == _defaultPreset);
    if (named) {
      _applyPreset(_defaultPreset);
    } else if (_presets.isNotEmpty) {
      _applyPreset(_presets.first.name);
    }
    // The format is remembered separately from the preset: the same settings in
    // a different container is a thing people want, and applying the preset
    // first means an unset codec changes nothing.
    if (defaults.codec.isNotEmpty) {
      final format = _formats.where((f) => f.key == defaults.codec).firstOrNull;
      if (format != null) _format = format;
    }
    // A comp with a loudness target is exported to it unless the preset says
    // otherwise.
    if (_compTarget case final target?
        when _loudnessPick == null && !_ownLoudness) {
      _loudnessPick = _targetPick;
      _truePeakDb = target.truePeakDb;
    }
    _seedDestination(defaults);
    _recompute();
    try {
      _lacking = widget.comp.shareLacking();
      _unread = widget.comp.unreadModels();
    } catch (_) {
      // No engine to ask, as in a widget test.
    }
    // The frames it reads, where everything can be had from someone: the
    // least to send that still delivers at full quality.
    if (_lacking.isNotEmpty && _lacking.every((l) => l.holders.isNotEmpty)) {
      _footage = _footageFrames;
    }
    final queued = widget.queued;
    final spec = queued == null ? null : exportQueueSpec(id: queued);
    final item =
        exportQueueList().where((item) => item.id == queued).firstOrNull;
    _failedItem = item?.state is BridgeExportQueueState_Failed;
    if (spec != null && item != null) _reopen(spec, item.path);
  }

  /// Whether the queued item it opened on had failed. Its settings cannot be
  /// changed in place, so it is queued again with the new ones.
  bool _failedItem = false;

  /// Open on a queued item's settings. Its footage was settled when it was
  /// queued, so that group goes.
  void _reopen(BridgeExportSpec spec, String path) {
    _lacking = const [];
    _takeSpec(spec, exact: true);
    _edit(() {
      _ownRate = spec.fps > 0 && (spec.fps - _compFps).abs() > 0.0005;
      _fps = spec.fps > 0 ? spec.fps : _compFps;
      _resize = spec.width > 0 && spec.height > 0;
      final (start, end) = (spec.rangeStartFrame, spec.rangeEndFrame);
      if (start < 0 || end <= start) {
        _span = _hasWorkArea ? _Span.workArea : _Span.wholeComp;
      } else if (_hasWorkArea && start == _workStart && end == _workEnd) {
        _span = _Span.workArea;
      } else if (start == 0 && end == _compFrames) {
        _span = _Span.wholeComp;
      } else {
        _span = _Span.custom;
        _rangeStart = start;
        _rangeEnd = end;
      }
      _useRegion = spec.useRegionOfInterest;
      for (final field in spec.metadata) {
        var target = _metadata.where((m) => m.key == field.key).firstOrNull;
        if (target == null) {
          target = _MetaField.own(field.key);
          _metadata.add(target);
        }
        target.value.text = field.value;
      }
      _openFolder = spec.openFolder;
      _makeANoise = spec.makeANoise;
      _path = _picked = _queuedPath = path;
    });
  }

  /// Where the queued item wrote when the dialogue opened on it.
  String? _queuedPath;

  @override
  void dispose() {
    _waitTimer?.cancel();
    _scroll.dispose();
    _presetName.dispose();
    for (final field in _metadata) {
      field.dispose();
    }
    super.dispose();
  }

  // ---- what the fields add up to -------------------------------------------

  bool get _images => _format.type == ExportOutputType.imageSequence;

  /// Whether the chosen format writes a picture at all. A sound file does not,
  /// so the whole Picture section is drawn **disabled** (docs/15 §12A.3d's
  /// rule, widened from the row to the section): there was a line under the
  /// rows saying the section did nothing, and a dead control says it better
  /// and in less room.
  bool get _picture => _caps.video || _images;

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

  /// The rate the file is written at: the one chosen, held to the fastest the
  /// format plays.
  double get _rate {
    final rate = _fps <= 0 ? _compFps : _fps;
    final most = _caps.maxFps;
    return most > 0 && rate > most ? most.toDouble() : rate;
  }

  /// The footer's line: what pressing the button would produce.
  String get _summary {
    final (start, end) = _range;
    final span = (end - start).clamp(0, 1 << 30);
    final seconds = _compFps <= 0 ? 0.0 : span / _compFps;
    // The span's own frames at the rate the file is written, counted as the
    // engine counts them.
    final frames =
        _rate == _compFps ? span : (seconds * _rate).round().clamp(1, 1 << 30);
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
    // Megabytes under a gigabyte, and nothing at all under a megabyte: an
    // estimate of nought is no estimate.
    final megabytes = bits * seconds / 8 / 1e6;
    if (megabytes < 1) return line;
    final size = megabytes >= 1000
        ? l10n.exportSummarySize((megabytes / 1000).toStringAsFixed(1))
        : l10n.exportSummarySizeMb('${megabytes.round()}');
    return '$line · $size';
  }

  /// The loudness the dialogue asks for, null for none.
  BridgeExportLoudness? get _loudness {
    final pick = _loudnessPick;
    final lufs = pick is double
        ? pick
        : pick == _targetPick
            ? _compTarget?.lufs
            : pick == _customPick
                ? _customLufs
                : null;
    return lufs == null
        ? null
        : BridgeExportLoudness(lufs: lufs, truePeakDb: _truePeakDb);
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
      audioRate: _audioSampleRate,
      audioDepth: _audioDepth,
      audioChannels: _audioChannels,
      depth: _depth,
      alphaChannel: _alphaChannel,
      straightAlpha: _straightAlpha,
      colourSpace: _colourSpace,
      resample: _resample,
      renderGuides: _renderGuides,
      captions: _captions,
      motionBlur: _motionBlur,
      retimeBlend: _retimeBlend,
      useProxies: _useProxies,
      cropTop: _cropTop,
      cropLeft: _cropLeft,
      cropBottom: _cropBottom,
      cropRight: _cropRight,
      useRegionOfInterest: _useRegion,
      region: Float64List.fromList(widget.region ?? const []),
      metadata: [
        for (final field in _metadata)
          // Both halves, or neither: a value under no name has nowhere to go,
          // and a name over no value writes an empty field.
          if (field.key.isNotEmpty && field.value.text.trim().isNotEmpty)
            BridgeMetadataField(key: field.key, value: field.value.text.trim()),
      ],
      qualityDivisor: _quality,
      diskCacheReadOnly: _diskCache,
      effects: _effects,
      honourSolo: _honourSolo,
      makeANoise: _makeANoise,
      openFolder: _openFolder,
      gifDither: _gifDither,
      gifPalettePerFrame: _gifPalettePerFrame,
      gifPlays: _gifPlays,
      loudness: _loudness,
      stems: _stems,
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
    // Asked of the composition, not free-standing: whether a colour space can
    // be delivered is a question about this project's colour config.
    _check = widget.comp.exportSpecCheck(spec: spec);
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
        // Vertical metrics never squish (§12A.6): when the window is too short
        // for every group, the body scrolls rather than the rows shrinking.
        ConstrainedBox(
          constraints: BoxConstraints(
            maxHeight: (MediaQuery.sizeOf(context).height - exportChromeHeight)
                .clamp(exportRowHeight, 4000),
          ),
          child: SingleChildScrollView(
            controller: _scroll,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (_lacking.isNotEmpty)
                  Padding(
                    padding: exportBodyPadding,
                    child: _footageGroup(t),
                  ),
                _essentials(t),
                _compositionGroup(t),
                _timeGroup(t),
                _pictureGroup(t),
                _colourGroup(t),
                _audioGroup(t),
                _metadataGroup(t),
              ],
            ),
          ),
        ),
        dialogFooter(
          t,
          summary:
              _waiting ?? _refused ?? (_check.isNotEmpty ? _check : _summary),
          keyPrefix: 'export',
          actions: [
            // While footage is on its way there is one thing to press.
            if (_waiting != null)
              HouseButton(
                key: const ValueKey('export-footage-cancel'),
                onPressed: _stopWaiting,
                padding: const EdgeInsets.symmetric(horizontal: 12),
                child: Text(l10n.cancel),
              ),
            // Only beside the one refusal that has somewhere to go: the page
            // where the missing addon is installed.
            if (_refusedForAddon)
              HouseButton(
                key: const ValueKey('export-open-addons'),
                onPressed: () => unawaited(showSettingsWindowFrb(context,
                    initialPage: SettingsPage.addons)),
                padding: const EdgeInsets.symmetric(horizontal: 12),
                child: Text(l10n.openAddons),
              ),
            // Opened from the queue, the one thing to do is keep the changes,
            // or for a row that failed, queue it again with them.
            if (widget.queued != null)
              HouseButton(
                key: const ValueKey('export-save-queued'),
                primary: true,
                autofocus: true,
                padding: const EdgeInsets.symmetric(horizontal: 16),
                onPressed: _canQueue ? _saveQueued : null,
                child: Text(_failedItem ? l10n.exportQueueAgain : l10n.save),
              )
            else ...[
              HouseButton(
                key: const ValueKey('export-add-to-queue'),
                onPressed: _canQueue ? () => _queue(start: false) : null,
                padding: const EdgeInsets.symmetric(horizontal: 12),
                child: Text(l10n.exportAddToQueue),
              ),
              HouseButton(
                key: const ValueKey('export-start'),
                // The window's default action: Enter exports unless a field is
                // being typed in.
                primary: true,
                autofocus: true,
                padding: const EdgeInsets.symmetric(horizontal: 16),
                onPressed: _canQueue ? () => _queue(start: true) : null,
                child: Text(l10n.exportAction),
              ),
            ],
          ],
        ),
      ],
    );
  }

  /// A spec the format will actually carry. A refusal stands in the footer
  /// where the summary was, so the reason is read before the button is missed.
  /// Somewhere to write is not asked for here: with none chosen yet, pressing
  /// the button asks.
  bool get _canQueue => _check.isEmpty && _waiting == null;

  /// What a shut fold says it holds.
  String _foldReading(ExportSection section) {
    final (width, height) = _outputSize;
    final (start, end) = _range;
    return switch (section) {
      ExportSection.composition => [
          _qualityLabel(_quality),
          l10n.exportReadingSize('$width', '$height'),
        ],
      ExportSection.time => [
          switch (_span) {
            _Span.workArea => l10n.exportSpanWorkArea('$_workStart', '$_workEnd'),
            _Span.wholeComp => l10n.exportSpanWholeComp,
            _Span.custom => l10n.custom,
          },
          '${_formatRate(_rate)} ${l10n.unitFps}',
          _timecode(end - start, _compFps),
        ],
      ExportSection.picture => [
          _alphaChannel ? l10n.exportChannelsRgbAlpha : l10n.exportChannelsRgb,
          _depth >= 16 ? l10n.exportDepth16 : l10n.exportDepth8,
          if (_caps.video && _bitrateBps > 0)
            l10n.exportReadingVideoRate('${(_bitrateBps / 1e6).round()}'),
        ],
      ExportSection.colour => [_colourSpaceLabel(_colourSpace)],
      ExportSection.audio => _audio && _caps.audio
          ? [
              _sampleRateLabel(_audioSampleRate),
              _audioChannels == 1 ? l10n.exportAudioMono : l10n.exportAudioStereo,
              if (_caps.audioBitRate)
                l10n.exportReadingAudioRate('${_audioRate ~/ 1000}'),
              if (_loudness case final loudness?)
                loudnessTargetLabel(loudness.lufs),
            ]
          : [l10n.off],
      // Nothing to say until somebody has written something in it.
      ExportSection.metadata => [
          for (final field in _metadata)
            if (field.value.text.isNotEmpty) field.value.text,
        ],
    }.join(' · ');
  }

  // ---- the groups ----------------------------------------------------------

  /// What every export needs, on show with nothing to open: the preset, what
  /// to write, where, and what to do after.
  Widget _essentials(LumitTheme t) => Padding(
        key: const ValueKey('export-preset-strip'),
        padding: const EdgeInsets.symmetric(
            horizontal: dialogPadding, vertical: 8),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _row(
              t,
              l10n.exportPreset,
              Row(children: [
                Expanded(
                  child: GestureDetector(
                    onSecondaryTapDown: (d) => showMenuAt<void>(
                      context: context,
                      position: d.globalPosition,
                      rows: (close) =>
                          _presetRows(() => close(null), saveAs: true),
                    ),
                    child: SizedBox(
                      height: dialogControlHeight,
                      child: BareDropdown<String>(
                        key: const ValueKey('export-preset'),
                        value: _preset,
                        options: ['', ..._presets.map((p) => p.name)],
                        label: (p) => p.isEmpty ? l10n.custom : p,
                        onChanged: _applyPreset,
                      ),
                    ),
                  ),
                ),
                const SizedBox(width: 6),
                _stripButton(t, 'export-preset-save-as',
                    l10n.exportPresetSaveAs, () => _nameAPreset('')),
                const SizedBox(width: 6),
                _presetMore(t),
              ]),
            ),
            // Naming a preset: the one line *Edit* and *Save as…* both open,
            // because saving over a name and renaming into it are the same act
            // (the store replaces a preset of that name in its own row).
            if (_naming)
              _row(
                t,
                l10n.exportPresetName,
                Row(children: [
                  Expanded(
                    child: HouseTextField(
                      key: const ValueKey('export-preset-name'),
                      controller: _presetName,
                      width: double.infinity,
                      autofocus: true,
                      fill: t.surface0,
                      onSubmitted: (_) => _savePreset(),
                    ),
                  ),
                  const SizedBox(width: 6),
                  _stripButton(t, 'export-preset-save', l10n.save, _savePreset),
                  if (_presets.any((p) => p.name == _preset && !p.readOnly)) ...[
                    const SizedBox(width: 6),
                    _stripButton(
                        t, 'export-preset-delete', l10n.delete, _deletePreset),
                  ],
                  const SizedBox(width: 6),
                  _stripButton(t, 'export-preset-cancel', l10n.cancel,
                      () => setState(() => _naming = false)),
                ]),
              ),
            _row(
              t,
              l10n.exportFormat,
              Row(children: [
                for (final type in ExportOutputType.values)
                  if (_formats.any((f) => f.type == type)) _typeChip(t, type),
                const SizedBox(width: 6),
                Expanded(
                  child: dialogDropdown<_Format>(
                    t,
                    id: 'export-format',
                    value: _format,
                    options:
                        _formats.where((f) => f.type == _format.type).toList(),
                    label: (f) => f.label,
                    onChanged: _setFormat,
                  ),
                ),
              ]),
            ),
            _row(
              t,
              l10n.exportWriteTo,
              Row(children: [
                Expanded(
                  child: GestureDetector(
                    onSecondaryTapDown: (d) => _pathMenu(d.globalPosition),
                    child: _well(
                      t,
                      _path == null ? l10n.exportNotChosen : _leaf(_path!),
                      key: const ValueKey('export-path'),
                      tone: t.textMuted,
                      tip: _path,
                    ),
                  ),
                ),
                // A page has no folders: the browser asks where as it starts.
                if (!kIsWeb) ...[
                  const SizedBox(width: 6),
                  // A floor rather than a width: a longer word for Browse grows
                  // the button instead of wrapping.
                  ConstrainedBox(
                    constraints: const BoxConstraints(
                        minWidth: exportButtonWidth,
                        minHeight: dialogControlHeight,
                        maxHeight: dialogControlHeight),
                    child: HouseButton(
                      key: const ValueKey('export-choose'),
                      onPressed: _choose,
                      child: Text(l10n.exportBrowseEllipsis, style: t.body),
                    ),
                  ),
                ],
              ]),
            ),
            // *When done* is two independent answers: a long export left
            // running wants the sound and the folder both.
            _row(
              t,
              l10n.exportWhenDone,
              Row(children: [
                _tick(t, 'export-make-a-noise', l10n.exportMakeANoise,
                    _makeANoise, (on) => _edit(() => _makeANoise = on)),
                const SizedBox(width: 16),
                _tick(t, 'export-open-folder', l10n.exportOpenFolder,
                    _openFolder, (on) => _edit(() => _openFolder = on)),
              ]),
            ),
          ],
        ),
      );

  /// The preset's quieter acts, behind the three dots.
  Widget _presetMore(LumitTheme t) => LumitTooltip(
        message: l10n.exportPresetMore,
        child: Builder(
          builder: (context) => SizedBox(
            width: dialogControlHeight,
            height: dialogControlHeight,
            child: HouseButton(
              key: const ValueKey('export-preset-more'),
              padding: EdgeInsets.zero,
              onPressed: () {
                final box = context.findRenderObject()! as RenderBox;
                showMenuAt<void>(
                  context: context,
                  position: box.localToGlobal(Offset(0, box.size.height + 2)),
                  rows: (close) => _presetRows(() => close(null)),
                );
              },
              child: glyph.LumitIcon(LumitIcons.more,
                  size: 12, colour: t.textSecondary),
            ),
          ),
        ),
      );

  /// What can be done with the preset in force: the three dots' rows, and
  /// Save as… before them on the preset's own right-click, where the button
  /// beside it is not in reach. The default wears a tick, as a choice does.
  List<Widget> _presetRows(VoidCallback close, {bool saveAs = false}) {
    // Greyed for a shipped preset, which is not yours to change or delete.
    final own = _presets.any((p) => p.name == _preset && !p.readOnly);
    return [
      if (saveAs)
        MenuRow(
          key: const ValueKey('export-preset-save-as-row'),
          onPressed: () {
            close();
            _nameAPreset('');
          },
          child: MenuLabel(l10n.exportPresetSaveAs, ticked: false),
        ),
      MenuRow(
        key: const ValueKey('export-preset-edit'),
        enabled: own,
        onPressed: () {
          close();
          _nameAPreset(_preset);
        },
        child: MenuLabel(l10n.exportPresetEdit, ticked: false, enabled: own),
      ),
      MenuRow(
        key: const ValueKey('export-preset-set-default'),
        onPressed: () {
          close();
          _setAsDefault();
        },
        child: MenuLabel(l10n.exportPresetSetDefault,
            ticked: _preset == _defaultPreset),
      ),
      const MenuDivider(),
      MenuRow(
        key: const ValueKey('export-preset-delete-row'),
        enabled: own,
        onPressed: () {
          close();
          _deletePreset();
        },
        child: MenuLabel(l10n.delete, ticked: false, enabled: own),
      ),
    ];
  }

  /// The file's right-click: its path to the clipboard, another place, and
  /// the folder it goes in.
  void _pathMenu(Offset at) {
    final path = _path;
    final folder = path == null || kIsWeb ? null : File(path).parent.path;
    final folderThere = folder != null && Directory(folder).existsSync();
    showMenuAt<void>(
      context: context,
      position: at,
      rows: (close) => [
        MenuRow(
          key: const ValueKey('export-path-copy'),
          enabled: path != null,
          onPressed: () {
            close(null);
            Clipboard.setData(ClipboardData(text: path!));
          },
          child: Text(l10n.welcomeCopyPath),
        ),
        if (!kIsWeb) ...[
          MenuRow(
            key: const ValueKey('export-path-browse'),
            onPressed: () {
              close(null);
              _choose();
            },
            child: Text(l10n.exportBrowseEllipsis),
          ),
          // The file itself once it is there, else the folder it will be in.
          MenuRow(
            key: const ValueKey('export-path-folder'),
            enabled: folderThere,
            onPressed: () {
              close(null);
              if (!revealInFolder(path: path!)) revealInFolder(path: folder!);
            },
            child: Text(l10n.projectShowInFolder),
          ),
        ],
      ],
    );
  }

  /// A strip button at its content width — §12A.4's 12 either side of an
  /// outlined label, which is the whole of the clipping fix.
  Widget _stripButton(
          LumitTheme t, String id, String label, VoidCallback onPressed,
          {bool active = false}) =>
      SizedBox(
        height: dialogControlHeight,
        child: HouseButton(
          key: ValueKey<String>(id),
          padding: const EdgeInsets.symmetric(horizontal: 12),
          active: active,
          onPressed: onPressed,
          child: Text(label, style: t.body),
        ),
      );

  /// How the composition is drawn on the way out — and the sound-only case it
  /// has nothing to say in either. Every row here is a statement about the
  /// picture, so a format with no picture leaves the whole group dead, exactly
  /// as Picture and Colour are (the owner's ruling: there is no image in an
  /// audio-only export to alter). The engine agrees rather than being taken on
  /// trust: `ExportSpec::render_options` hands a sound-only export the
  /// defaults, so a solo switch left ignored here cannot decide what a `.wav`
  /// contains.
  Widget _compositionGroup(LumitTheme t) {
    final dim = !_picture;
    return _group(
      t,
      ExportSection.composition,
      l10n.exportGroupComposition,
      [
        if (_unread.isNotEmpty)
          _warning(t, l10n.exportModelsUnread(_unread.join(', ')),
              key: const ValueKey('export-models-unread')),
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
            dimmed: dim,
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
            dimmed: dim,
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
            dimmed: dim,
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
            dimmed: dim,
          ),
        ),
        // Both default to the delivery answer rather than to the project's
        // own: an export reads the originals and leaves the guide layers out,
        // whatever is switched on to work with.
        _columns(
          _row(
            t,
            l10n.exportProxies,
            dialogDropdown<bool>(
              t,
              id: 'export-proxies',
              value: _useProxies,
              options: const [false, true],
              label: (on) =>
                  on ? l10n.exportProxiesAll : l10n.exportProxiesNone,
              onChanged: (on) => _edit(() => _useProxies = on),
            ),
            dimmed: dim,
          ),
          _row(
            t,
            l10n.exportGuideLayers,
            dialogDropdown<bool>(
              t,
              id: 'export-guide-layers',
              value: _renderGuides,
              options: const [false, true],
              label: (on) =>
                  on ? l10n.exportCurrentSettings : l10n.exportAllOff,
              onChanged: (on) => _edit(() => _renderGuides = on),
            ),
            labelColumn: exportLabelColumnPaired,
            dimmed: dim,
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
            dimmed: dim,
          ),
          _row(
            t,
            l10n.exportColourDepth,
            _depthDropdown(t, 'export-colour-depth'),
            labelColumn: exportLabelColumnPaired,
            dimmed: dim,
          ),
        ),
        if (_hasCaptions) _captionsRow(t, dim),
      ],
      dimmed: dim,
    );
  }

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
          // controls keep one left edge and one right.
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
                        value: _rate,
                        min: 1,
                        max: _caps.maxFps > 0 ? _caps.maxFps.toDouble() : 240,
                        decimals: 2,
                        resetTo: _compFps,
                        fill: t.surface0,
                        onChanged: (v) => _edit(() => _fps = v.toDouble()),
                      )
                    : _well(t, _formatRate(_rate),
                        key: const ValueKey('export-fps'),
                        tone: t.textDisabled),
              ),
            ]),
            labelColumn: exportLabelColumnPaired,
          ),
        ),
        // Both are overrides on the export's own throwaway snapshot. Motion
        // blur has three answers because blur passes two gates — the
        // composition's master switch and the layer's own check; Retime blend
        // has two, because a layer's Nearest/Blend/Flow choice *is* its check
        // and there is no comp-wide master for a third answer to speak to.
        _columns(
          _row(
            t,
            l10n.exportMotionBlur,
            dialogDropdown<int>(
              t,
              id: 'export-motion-blur',
              value: _motionBlur,
              options: const [0, 1, 2],
              label: (v) => switch (v) {
                0 => l10n.exportCurrentSettings,
                1 => l10n.exportOnForCheckedLayers,
                _ => l10n.exportOffForAllLayers,
              },
              onChanged: (v) => _edit(() => _motionBlur = v),
            ),
          ),
          _row(
            t,
            l10n.exportRetimeBlend,
            dialogDropdown<int>(
              t,
              id: 'export-retime-blend',
              value: _retimeBlend,
              options: const [0, 1],
              label: (v) => v == 0
                  ? l10n.exportCurrentSettings
                  : l10n.exportOffForAllLayers,
              onChanged: (v) => _edit(() => _retimeBlend = v),
            ),
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
            _timecode(start, _compFps),
            _timecode(end, _compFps),
            _timecode(end - start, _compFps),
          ),
          key: const ValueKey('export-time-reading'),
        ),
      ],
    );
  }

  /// Picture, and the sound-only case it has nothing to say in: a format with
  /// no picture leaves every row here dead, so the whole group states that
  /// rather than each row stating it alone.
  Widget _pictureGroup(LumitTheme t) {
    final (width, height) = _outputSize;
    final crop = _crop;
    final dim = !_picture;
    return _group(
      t,
      ExportSection.picture,
      l10n.exportGroupPicture,
      [
        _columns(
          _row(
            t,
            l10n.exportChannels,
            _why(
              _caps.alpha ? null : l10n.tipExportFormatLimit,
              dialogDropdown<bool>(
                t,
                id: 'export-channels',
                value: _alphaChannel,
                options: const [false, true],
                label: (alpha) => alpha
                    ? l10n.exportChannelsRgbAlpha
                    : l10n.exportChannelsRgb,
                onChanged:
                    _caps.alpha ? (a) => _edit(() => _alphaChannel = a) : null,
              ),
            ),
            dimmed: dim,
          ),
          _row(
            t,
            l10n.exportAlpha,
            _why(
              !_caps.alpha || !_caps.alphaMode
                  ? l10n.tipExportFormatLimit
                  : _alphaChannel
                      ? null
                      : l10n.tipExportNeedsAlpha,
              dialogDropdown<bool>(
                t,
                id: 'export-alpha',
                // A GIF always writes straight, so it says so.
                value: _caps.alpha && !_caps.alphaMode || _straightAlpha,
                options: const [false, true],
                label: (straight) => straight
                    ? l10n.exportAlphaStraight
                    : l10n.exportAlphaPremultiplied,
                onChanged: _caps.alpha && _caps.alphaMode && _alphaChannel
                    ? (s) => _edit(() => _straightAlpha = s)
                    : null,
              ),
            ),
            labelColumn: exportLabelColumnPaired,
            dimmed: dim,
          ),
        ),
        _columns(
          _row(t, l10n.exportDepth, _depthDropdown(t, 'export-depth'),
              dimmed: dim),
          _row(
            t,
            l10n.exportBitRate,
            Row(children: [
              _why(
                _caps.bitRate ? null : l10n.tipExportFormatLimit,
                _tick(
                  t,
                  'export-bitrate-auto',
                  l10n.exportBitRateAuto,
                  _autoBitrate,
                  _caps.bitRate ? (on) => _edit(() => _autoBitrate = on) : null,
                ),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: SizedBox(
                  height: dialogControlHeight,
                  child: _autoBitrate || !_caps.bitRate
                      ? _well(
                          t,
                          _bitrateBps > 0
                              ? l10n.exportReadingVideoRate(
                                  '${(_bitrateBps / 1e6).round()}')
                              : '—',
                          key: const ValueKey('export-bitrate'),
                          tone: t.textDisabled,
                        )
                      : DragValueField(
                          key: const ValueKey('export-bitrate'),
                          value: _bitrate,
                          min: 0,
                          max: 400,
                          // The unit alone: the reading with no number in it.
                          suffix: _bitrate == 0
                              ? null
                              : l10n.exportReadingVideoRate(''),
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
            dimmed: dim,
          ),
        ),
        if (_caps.gif) ..._gifRows(t, dim),
        _row(
          t,
          l10n.exportResize,
          Row(children: [
            HouseCheckbox(
              key: const ValueKey('export-resize'),
              value: _resize,
              onChanged: _picture ? (on) => _edit(() => _resize = on) : null,
            ),
            const SizedBox(width: 6),
            SizedBox(
              width: exportSizeWell,
              height: dialogControlHeight,
              child: _resize && _picture
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
              child: _resize && _picture
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
            Flexible(
              child: HouseCheckbox(
                key: const ValueKey('export-lock-aspect'),
                value: _lockAspect,
                label: l10n.exportLockAspect,
                onChanged:
                    _picture ? (on) => _edit(() => _lockAspect = on) : null,
              ),
            ),
            const Spacer(),
            // *Fast* is bilinear and *High* is Lanczos-3 widened by the shrink
            // factor. Fast leads because it is what every export has always
            // written, not because it is better.
            SizedBox(
              width: exportResampleWidth,
              height: dialogControlHeight,
              child: BareDropdown<String>(
                key: const ValueKey('export-resample'),
                value: _resample,
                options: const ['', 'high'],
                label: (v) => v == 'high'
                    ? l10n.exportResampleHigh
                    : l10n.exportResampleFast,
                onChanged:
                    _caps.video ? (v) => _edit(() => _resample = v) : null,
              ),
            ),
          ]),
          dimmed: dim,
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
            Flexible(
              child: HouseCheckbox(
                key: const ValueKey('export-use-region'),
                value: _useRegion,
                label: l10n.exportUseRegionOfInterest,
                onChanged:
                    _picture ? (on) => _edit(() => _useRegion = on) : null,
              ),
            ),
            const SizedBox(width: 8),
            Flexible(
              child: Align(
                alignment: AlignmentDirectional.centerEnd,
                child: Text(
                  l10n.exportFinalSize('$width', '$height'),
                  key: const ValueKey('export-final-size'),
                  style: dialogMono(t)
                      .copyWith(color: _picture ? null : t.textDisabled),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ),
          ]),
          dimmed: dim,
        ),
        // Not while the group is dead: a reading of a frame nobody is writing
        // is the "this does nothing" line the disabled group already states.
        if (!dim &&
            crop != null &&
            (crop.width < _compWidth || crop.height < _compHeight))
          _reading(
            t,
            l10n.exportCropReading('${crop.width}', '${crop.height}'),
            key: const ValueKey('export-crop-reading'),
          ),
      ],
      dimmed: dim,
    );
  }

  /// Colour: the five built-in spaces this build transforms to, and — once the
  /// project names a colour config — every output space that config offers.
  /// The list a format offers is the format's own: a container that can state
  /// its colour carries all five built-ins, and a still — which states none —
  /// is offered only the space an untagged file is universally taken to be. A
  /// config's space rides on top of that list whatever the container can
  /// state, because an OCIO export is written untagged by design
  /// (docs/impl/ocio.md §5.2); what refuses it is the config, not the format,
  /// and `_colourRefused` is where that answer is.
  Widget _colourGroup(LumitTheme t) {
    final dim = !_picture;
    return _group(
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
            value: _colourSpace,
            options: _colourOptions,
            label: _colourSpaceLabel,
            // The config's names are the user's own words and never go through
            // the label table; the heading over them is ours.
            group: (v) => widget.colour.spaces.contains(v) &&
                    !_caps.colourSpaces.contains(v)
                ? l10n.colourSpaceFromConfig
                : null,
            disabledReason: (v) => _colourRefused[v],
            onChanged: _colourOptions.length > 1
                ? (v) => _edit(() => _colourSpace = v)
                : null,
          ),
          dimmed: dim,
        ),
        _row(
          t,
          l10n.exportColourManagement,
          _dead(
            t,
            'export-ocio',
            widget.colour.path.isEmpty
                ? l10n.exportColourOcio
                : widget.colour.path,
            l10n.tipExportColourManagedBy,
          ),
          dimmed: dim,
        ),
        // A file written through a config's transform carries no reliable
        // primaries or transfer to state, so it is written untagged rather
        // than mis-tagged (docs/impl/ocio.md §5.2). Said here, where the
        // choice is made, since a file without a tag can come back looking
        // wrong.
        if (_colourFromConfig && _caps.colourSpaces.isNotEmpty)
          _warning(t, l10n.exportColourConfigUntagged,
              key: const ValueKey('export-colour-tagging')),
      ],
      dimmed: dim,
    );
  }

  /// The dropdown's list: the format's own built-in spaces, then the loaded
  /// config's, and always the stored value even where neither offers it — a
  /// preset naming a space this format cannot state must still read back as
  /// what it says, not as something else.
  List<String> get _colourOptions => [
        if (_caps.colourSpaces.isEmpty) _colourSpace,
        ..._caps.colourSpaces,
        // Only where the container carries a picture at all: a sound file has
        // no colour to write, and its empty capability row says so.
        if (_caps.colourSpaces.isNotEmpty)
          for (final space in widget.colour.spaces)
            if (!_caps.colourSpaces.contains(space)) space,
      ];

  /// Whether the chosen space came out of the project's colour config rather
  /// than the built-in family — which decides whether the file can be tagged
  /// at all. A built-in the *format* cannot state is a different matter, and
  /// the format's own reading says that.
  bool get _colourFromConfig =>
      widget.colour.loaded && widget.colour.spaces.contains(_colourSpace);

  /// A stored colour-space name in the words the row shows. An OCIO config's
  /// space is shown as it arrived — it is the user's own word, like a codec
  /// name, and needs no table of ours.
  String _colourSpaceLabel(String stored) => switch (stored) {
        '' => l10n.exportColourSpaceSrgb,
        'linear' => l10n.exportColourSpaceLinear,
        'rec709' => l10n.exportColourSpaceRec709,
        'rec2020' => l10n.exportColourSpaceRec2020,
        'display-p3' => l10n.exportColourSpaceDisplayP3,
        _ => stored,
      };

  /// Sound, and the mute case: a folder of stills carries none, so the group
  /// goes dead entire rather than offering four rows that write nothing.
  Widget _audioGroup(LumitTheme t) {
    final dim = !_caps.audio;
    // Why the settings after the first list are dead while it reads Off.
    final off = _audio ? null : l10n.tipExportAudioOff;
    final live = _caps.audio && _audio;
    return _group(
      t,
      ExportSection.audio,
      l10n.exportGroupAudio,
      [
        _row(
          t,
          l10n.exportAudioRowLabel,
          Row(children: [
            SizedBox(
              width: exportAudioSourceWidth,
              height: dialogControlHeight,
              child: BareDropdown<bool>(
                key: const ValueKey('export-audio'),
                value: _audio,
                options: const [true, false],
                label: (on) => on ? l10n.exportAudioAuto : l10n.exportAudioOff,
                onChanged:
                    _caps.audio ? (on) => _edit(() => _audio = on) : null,
              ),
            ),
            const SizedBox(width: 6),
            // The three sound settings. The rate resamples at the source
            // rather than after the mix; the width means something only for
            // the uncompressed forms, so AAC offers sixteen alone rather than
            // accepting a setting it cannot honour; and mono is the finished
            // stereo mix folded down, once.
            SizedBox(
              width: exportAudioRateWidth,
              height: dialogControlHeight,
              child: _why(
                off,
                BareDropdown<int>(
                  key: const ValueKey('export-audio-sample-rate'),
                  value: _audioSampleRate,
                  options: _sampleRates,
                  label: _sampleRateLabel,
                  onChanged: _caps.audio && _audio
                      ? (r) => _edit(() => _audioSampleRate = r)
                      : null,
                ),
              ),
            ),
            const SizedBox(width: 6),
            SizedBox(
              width: exportAudioDepthWidth,
              height: dialogControlHeight,
              child: _why(
                off ?? (_caps.audio24Bit ? null : l10n.tipExportFormatLimit),
                BareDropdown<int>(
                  key: const ValueKey('export-audio-depth'),
                  value: _audioDepth,
                  options: _caps.audio24Bit ? const [16, 24] : const [16],
                  label: (d) =>
                      d >= 24 ? l10n.exportAudio24Bit : l10n.exportAudio16Bit,
                  onChanged: _caps.audio && _caps.audio24Bit && _audio
                      ? (d) => _edit(() => _audioDepth = d)
                      : null,
                ),
              ),
            ),
            const SizedBox(width: 6),
            SizedBox(
              width: exportAudioLayoutWidth,
              height: dialogControlHeight,
              child: _why(
                off,
                BareDropdown<int>(
                  key: const ValueKey('export-audio-layout'),
                  value: _audioChannels,
                  options: const [2, 1],
                  label: (c) =>
                      c == 1 ? l10n.exportAudioMono : l10n.exportAudioStereo,
                  onChanged: _caps.audio && _audio
                      ? (c) => _edit(() => _audioChannels = c)
                      : null,
                ),
              ),
            ),
            const Spacer(),
            SizedBox(
              width: exportAudioBitRateWidth,
              height: dialogControlHeight,
              child: _why(
                off ??
                    (_caps.audioBitRate ? null : l10n.tipExportFormatLimit),
                BareDropdown<int>(
                  key: const ValueKey('export-audio-rate'),
                  value: _audioRate,
                  options: _audioRates,
                  label: (r) => l10n.exportReadingAudioRate('${r ~/ 1000}'),
                  onChanged: _caps.audioBitRate && _audio
                      ? (r) => _edit(() => _audioRate = r)
                      : null,
                ),
              ),
            ),
          ]),
          dimmed: dim,
        ),
        _columns(
          _row(
            t,
            l10n.exportLoudness,
            Row(children: [
              Expanded(
                child: SizedBox(
                  height: dialogControlHeight,
                  child: _why(
                    off,
                    BareDropdown<Object?>(
                      key: const ValueKey('export-loudness'),
                      value: _loudnessPick,
                      options: [
                        null,
                        if (_compTarget != null) _targetPick,
                        ...loudnessPresets,
                        _customPick,
                      ],
                      label: _loudnessLabel,
                      onChanged: live ? _pickLoudness : null,
                    ),
                  ),
                ),
              ),
              if (_loudnessPick == _customPick) ...[
                const SizedBox(width: 6),
                SizedBox(
                  width: exportLoudnessWell,
                  height: dialogControlHeight,
                  child: live
                      ? DragValueField(
                          key: const ValueKey('export-loudness-lufs'),
                          value: _customLufs,
                          min: -40,
                          max: -5,
                          decimals: 1,
                          fill: t.surface0,
                          onChanged: (v) =>
                              _edit(() => _customLufs = v.toDouble()),
                        )
                      : _well(t, _customLufs.toStringAsFixed(1),
                          key: const ValueKey('export-loudness-lufs'),
                          tone: t.textDisabled),
                ),
              ],
            ]),
            dimmed: dim,
          ),
          _loudness == null
              ? null
              : _row(
                  t,
                  l10n.exportTruePeak,
                  Align(
                    alignment: AlignmentDirectional.centerStart,
                    child: SizedBox(
                      width: exportLoudnessWell,
                      height: dialogControlHeight,
                      child: live
                          ? DragValueField(
                              key: const ValueKey('export-true-peak'),
                              value: _truePeakDb,
                              min: -9,
                              max: 0,
                              decimals: 1,
                              suffix: ' ${l10n.unitSymbolDbtp}',
                              resetTo: presetTruePeakDb,
                              fill: t.surface0,
                              onChanged: (v) =>
                                  _edit(() => _truePeakDb = v.toDouble()),
                            )
                          : _well(
                              t,
                              '${_truePeakDb.toStringAsFixed(1)} '
                              '${l10n.unitSymbolDbtp}',
                              key: const ValueKey('export-true-peak'),
                              tone: t.textDisabled),
                    ),
                  ),
                  labelColumn: exportLabelColumnPaired,
                  dimmed: dim,
                ),
        ),
        if (_hasBus)
          _row(
            t,
            l10n.exportStems,
            _why(
              off,
              _tick(t, 'export-stems', l10n.exportStemsEach, _stems,
                  live ? (on) => _edit(() => _stems = on) : null),
            ),
            dimmed: dim,
          ),
      ],
      dimmed: dim,
    );
  }

  /// A loudness choice as the list reads it.
  String _loudnessLabel(Object? pick) => switch (pick) {
        final double lufs => loudnessTargetLabel(lufs),
        _targetPick => l10n.exportLoudnessTarget(
            loudnessTargetLabel(_compTarget?.lufs ?? 0)),
        _customPick => l10n.custom,
        _ => l10n.off,
      };

  /// A loudness picked off the list. The comp's target brings its own true
  /// peak with it.
  void _pickLoudness(Object? pick) => _edit(() {
        _loudnessPick = pick;
        if (_compTarget case final target? when pick == _targetPick) {
          _truePeakDb = target.truePeakDb;
        }
      });

  /// Metadata: an ordered key/value set, because the order lands in the file
  /// and an export is deterministic. The five classic fields lead, named; a
  /// field of one's own carries FFmpeg's own key in a well beside its value.
  /// A container with nowhere to keep metadata leaves the whole group dead,
  /// the same way a sound file leaves Picture dead.
  Widget _metadataGroup(LumitTheme t) {
    final dim = !_caps.metadata;
    return _group(
      t,
      ExportSection.metadata,
      l10n.exportGroupMetadata,
      [
        for (final (index, field) in _metadata.indexed)
          _row(
            t,
            field.label ?? '',
            dimmed: dim,
            Row(children: [
              if (field.name case final name?) ...[
                // The key, typed: FFmpeg's own word, and the reason the row
                // has no label above it.
                SizedBox(
                  width: exportMetadataNameWidth,
                  child: _caps.metadata
                      ? HouseTextField(
                          key: ValueKey<String>('export-metadata-name-$index'),
                          controller: name,
                          width: exportMetadataNameWidth,
                          fill: t.surface0,
                          onSubmitted: (_) => _edit(() {}),
                          submitOnLostFocus: true,
                        )
                      : _well(t, name.text,
                          key: ValueKey<String>('export-metadata-name-$index'),
                          tone: t.textDisabled),
                ),
                const SizedBox(width: 6),
              ],
              Expanded(
                // A container with nowhere to keep metadata gets the reading
                // and not the field: the disabled face is the whole message,
                // where a line under the rows saying so was one more thing
                // to read.
                child: _caps.metadata
                    ? HouseTextField(
                        key: ValueKey<String>('export-metadata-$index'),
                        controller: field.value,
                        width: double.infinity,
                        fill: t.surface0,
                        onSubmitted: (_) => _edit(() {}),
                        submitOnLostFocus: true,
                      )
                    : _well(t, field.value.text,
                        key: ValueKey<String>('export-metadata-$index'),
                        tone: t.textDisabled),
              ),
              const SizedBox(width: 6),
              SizedBox(
                height: dialogControlHeight,
                child: HouseButton(
                  key: ValueKey<String>('export-metadata-remove-$index'),
                  onPressed: _caps.metadata
                      ? () => _edit(() {
                            _metadata.removeAt(index).dispose();
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
          dimmed: dim,
          Row(children: [
            SizedBox(
              height: dialogControlHeight,
              child: HouseButton(
                key: const ValueKey('export-metadata-add'),
                onPressed: _caps.metadata
                    ? () => _edit(() => _metadata
                        .add(_MetaField.own('field_${_metadata.length + 1}')))
                    : null,
                child: Text(l10n.exportMetadataAdd, style: t.body),
              ),
            ),
          ]),
        ),
      ],
      dimmed: dim,
    );
  }

  // ---- the pieces a row is made of -----------------------------------------

  /// A titled group that knows which section it belongs to, so a tab can light
  /// it briefly when it is jumped to.
  Widget _group(
    LumitTheme t,
    ExportSection section,
    String title,
    List<Widget> rows, {
    bool dimmed = false,
  }) {
    // A warning in a shut fold would never be read, so the fold is held
    // open, and its twirl does nothing until the warning is.
    final held = section == ExportSection.composition && _unread.isNotEmpty;
    final open = held || exportOpenSections.contains(section);
    return _why(
      // A dead group answers no pointer, so it says why once for all its
      // rows. Only three things leave a group dead, one per kind of group.
      !dimmed
          ? null
          : switch (section) {
              ExportSection.audio => l10n.tipExportNoSound,
              ExportSection.metadata => l10n.tipExportNoMetadata,
              _ => l10n.tipExportNoPicture,
            },
      dialogFold(
        t,
        title: title,
        reading: dimmed ? '' : _foldReading(section),
        readingAt: exportLabelColumn + exportRowGap,
        open: open,
        dimmed: dimmed,
        onToggle: held
            ? null
            : () => setState(() {
                  open
                      ? exportOpenSections.remove(section)
                      : exportOpenSections.add(section);
                }),
        rows: rows,
        key: ValueKey<String>('export-group-${section.name}'),
      ),
    );
  }

  Widget _row(LumitTheme t, String label, Widget control,
          {double labelColumn = exportLabelColumn, bool dimmed = false}) =>
      dialogRow(
        t,
        label,
        control,
        labelColumn: labelColumn,
        gap: exportRowGap,
        minHeight: exportRowHeight,
        dimmed: dimmed,
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
      HouseCheckbox(
        key: ValueKey<String>(id),
        value: value,
        label: label,
        onChanged: onChanged,
      );

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

  /// A control with the reason it is dead on hover, or the control alone when
  /// it is live. A greyed list with no reason reads as a fault. The reasons
  /// are two words each, which is all a tooltip is allowed here.
  Widget _why(String? reason, Widget control) => reason == null
      ? control
      : LumitTooltip(message: reason, child: control);

  /// The colour depth, offered only where the format carries more than one.
  Widget _depthDropdown(LumitTheme t, String id) => _why(
        _caps.depths.length > 1 ? null : l10n.tipExportFormatLimit,
        dialogDropdown<int>(
          t,
          id: id,
          value: _depth,
          options: _caps.depths.isEmpty
              ? [_depth]
              : _caps.depths.map((d) => d.toInt()).toList(),
          // OpenEXR carries floats and nothing else, so its 16 is half floats
          // rather than sixteen-bit codes and it offers a 32 the others do
          // not. Naming those by their depth alone keeps the row honest
          // without a second word for what is already an established
          // abbreviation.
          label: (d) => _format.key == 'exr'
              ? l10n.bitsPerChannel('$d')
              : (d >= 16 ? l10n.exportDepth16 : l10n.exportDepth8),
          onChanged:
              _caps.depths.length > 1 ? (d) => _edit(() => _depth = d) : null,
        ),
      );

  /// One crop inset: its mark and its well, in pixels at composition size.
  Widget _cropWell(LumitTheme t, String id, String mark, int value,
          ValueChanged<int> set, int limit) =>
      Row(mainAxisSize: MainAxisSize.min, children: [
        Padding(
          padding: const EdgeInsetsDirectional.only(end: 4),
          child: Text(mark,
              style: dialogMono(t)
                  .copyWith(color: _picture ? null : t.textDisabled)),
        ),
        SizedBox(
          width: exportCropWell,
          height: dialogControlHeight,
          // A sound file has nothing to crop, so the well is a reading rather
          // than a control — the same dead face the auto bitrate wears.
          child: _picture
              ? DragValueField(
                  key: ValueKey<String>('export-crop-$id'),
                  value: value,
                  min: 0,
                  max: (limit - 1).clamp(0, 16384),
                  fill: t.surface0,
                  onChanged: (v) => _edit(() => set(v.toInt())),
                )
              : _well(t, '$value',
                  key: ValueKey<String>('export-crop-$id'),
                  tone: t.textDisabled),
        ),
        const SizedBox(width: 6),
      ]);

  /// A recessed box holding something the user did not type — a chosen path,
  /// or a number this row is not offering to change.
  Widget _well(LumitTheme t, String text,
          {Key? key, Color? tone, String? tip}) =>
      Container(
        key: key,
        height: dialogControlHeight,
        alignment: AlignmentDirectional.centerStart,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        decoration: BoxDecoration(
          color: t.surface0,
          border: Border.all(color: t.hairline),
          borderRadius: BorderRadius.circular(t.tokens.sectionRadius),
        ),
        child: MiddleEllipsis(
          text,
          style: t.mono.copyWith(fontSize: 11, color: tone ?? t.textPrimary),
          tip: tip,
        ),
      );

  /// What to do about footage this computer has no original of: the choices
  /// that can be had, the least to send first.
  Widget _footageGroup(LumitTheme t) {
    final share = context.read<LumitState>().share;
    final everyone = _lacking.every((l) => l.holders.isNotEmpty);
    // The people who have every one of them, and so could do the export.
    final able = [
      if (everyone)
        for (final person in share.others)
          if (_lacking.every((l) => l.holders.contains(person.id))) person.id,
    ];
    final bytes = _lacking.fold<int>(0, (sum, l) => sum + l.bytes.toInt());
    final megabytes = (bytes / (1 << 20)).ceil();
    return dialogGroup(t, l10n.exportFootageGroup, [
      _row(
        t,
        l10n.exportFootage,
        dialogDropdown<int>(
          t,
          id: 'export-footage',
          value: _footage,
          options: [
            if (everyone) ...[_footageFrames, _footageOriginals],
            ...able,
            _footageAsItIs,
          ],
          label: (choice) => switch (choice) {
            _footageFrames => l10n.exportFootageFrames,
            _footageOriginals => l10n.exportFootageOriginals(megabytes),
            _footageAsItIs => l10n.exportFootageStandIns,
            final person => l10n.exportFootageOn(
                share.nameOf(person) ?? l10n.shareDefaultName),
          },
          onChanged: _waiting != null
              ? null
              : (choice) => setState(() => _footage = choice),
        ),
      ),
      _warning(t, l10n.exportFootageCount(_lacking.length)),
    ]);
  }

  /// Give up waiting on footage or on someone else's export. What has
  /// arrived is kept, and an export already asked of someone carries on
  /// there: its file still comes back.
  void _stopWaiting() {
    _waitTimer?.cancel();
    _waitTimer = null;
    if (_footage < 0) context.read<LumitState>().share.cancelFetch();
    if (mounted) setState(() => _waiting = null);
  }

  /// Get the footage first, or hand the export to someone who has it, and
  /// say in the footer how that is going until it is done.
  void _queueWithFootage(String path, {required bool start}) {
    final app = context.read<LumitState>();
    try {
      if (_footage >= 0) {
        widget.comp.shareAskExport(to: _footage, spec: _spec, path: path);
      } else {
        widget.comp.shareFetchExport(
            spec: _spec,
            path: path,
            parts: _footage == _footageFrames,
            start: start);
      }
    } catch (error) {
      setState(() => _refused = '$error');
      return;
    }
    setState(() {
      _refused = null;
      _waiting = l10n.exportFootageStarting;
    });
    _waitTimer?.cancel();
    _waitTimer = Timer.periodic(const Duration(milliseconds: 300),
        (_) => _watchFootage(app, start: start));
  }

  void _watchFootage(LumitState app, {required bool start}) {
    if (!mounted) return;
    int percent(int done, int total) =>
        total == 0 ? 0 : (done * 100 / total).round();
    void failed(String why) {
      _waitTimer?.cancel();
      setState(() {
        _waiting = null;
        _refused = why;
      });
    }

    final String text;
    if (_footage >= 0) {
      final who = app.share.nameOf(_footage) ?? l10n.shareDefaultName;
      switch (app.share.asking()) {
        case BridgeShareAsking_Idle() || BridgeShareAsking_Waiting():
          text = l10n.exportRemoteWaiting(who);
        case BridgeShareAsking_Running(:final frame, :final total):
          text = l10n.exportRemoteRunning(who, frame.toInt(), total.toInt());
        case BridgeShareAsking_Fetching(:final done, :final total):
          text = l10n.exportRemoteFetching(percent(done, total));
        case BridgeShareAsking_Done(:final path):
          _waitTimer?.cancel();
          app.postNotice(l10n.exportRemoteDone(_leaf(path)));
          return widget.onClose();
        case BridgeShareAsking_Failed(:final why):
          return failed(switch (why) {
            'refused' => l10n.exportRemoteRefused(who),
            'lacking' => l10n.exportRemoteLacking(who),
            _ => l10n.exportRemoteStopped(who),
          });
      }
    } else {
      switch (app.share.fetching()) {
        case BridgeShareFetching_Idle():
          text = l10n.exportFootageStarting;
        case BridgeShareFetching_Working(:final done, :final total):
          // Nothing has crossed while the others' computers are still
          // making what was asked for.
          text = total == 0
              ? l10n.exportFootageMaking
              : l10n.exportFootageFetching(percent(done, total));
        case BridgeShareFetching_Queued():
          _waitTimer?.cancel();
          if (start) statusLineExportStarted.value++;
          showExportQueueFrb(context: context);
          return widget.onClose();
        case BridgeShareFetching_Failed():
          return failed(l10n.exportFootageFailed);
      }
    }
    if (text != _waiting) setState(() => _waiting = text);
  }

  /// A factual line under a group's rows, in the label column's own indent.
  Widget _reading(LumitTheme t, String text, {Key? key}) => Padding(
        padding: const EdgeInsetsDirectional.only(
            start: exportLabelColumn + exportRowGap, top: 2, bottom: 4),
        child: Text(text, key: key, style: dialogMono(t)),
      );

  /// A line the user has to act on, where a reading would go.
  Widget _warning(LumitTheme t, String text, {Key? key}) => Padding(
        padding: const EdgeInsetsDirectional.only(
            start: exportLabelColumn + exportRowGap, top: 2, bottom: 4),
        child: Text(text, key: key, style: t.small),
      );

  /// One of the output types, as the drawing's chip: the chosen one outlined
  /// and bright, the rest bare and quiet — no accent, which §3.1 keeps for the
  /// filled action in the footer.
  Widget _typeChip(LumitTheme t, ExportOutputType type) {
    final on = _format.type == type;
    return HoverMark(
      builder: (context, lit) => GestureDetector(
        key: ValueKey<String>('export-type-${type.name}'),
        behavior: HitTestBehavior.opaque,
        onTap: () => _setFormat(_formats.firstWhere((f) => f.type == type)),
        child: Container(
          margin: const EdgeInsetsDirectional.only(end: dialogTabGap),
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(t.tokens.sectionRadius),
            border: Border.all(
                color: on ? t.hairlineStrong : const Color(0x00000000)),
          ),
          child: Text(
            t.kickerCase(switch (type) {
              ExportOutputType.video => l10n.exportTypeVideo,
              ExportOutputType.imageSequence => l10n.exportTypeImageSequence,
              ExportOutputType.audioOnly => l10n.exportTypeAudioOnly,
            }),
            style: on
                ? t.kickerOn
                : t.kicker.copyWith(
                    color: Color.lerp(t.kicker.color, t.textPrimary, lit)),
          ),
        ),
      ),
    );
  }

  /// A sample rate in the drawing's own face — three fixed readings rather
  /// than a formatted number, so the list reads as one column of kHz.
  String _sampleRateLabel(int hz) => switch (hz) {
        44100 => l10n.exportAudio44k,
        96000 => l10n.exportAudio96k,
        _ => l10n.exportAudio48k,
      };

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
  /// not a stamp on three of them.
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
    _takeSpec(stored);
  }

  /// Fill every field a preset carries from [stored], its name included.
  /// [exact] for a queued export's own settings, which name their loudness.
  void _takeSpec(BridgeExportSpec stored, {bool exact = false}) {
    _edit(() {
      _preset = stored.preset;
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
      // A preset written before these fields existed carries the defaults the
      // engine fills in, so applying one is never a way to lose a setting.
      _audioSampleRate = stored.audioRate == 0 ? 48000 : stored.audioRate;
      _audioDepth = stored.audioDepth >= 24 ? 24 : 16;
      _audioChannels = stored.audioChannels == 1 ? 1 : 2;
      final loudness = stored.loudness;
      _ownLoudness = exact ||
          _presets.any((p) => p.name == stored.preset && !p.readOnly);
      final target = _compTarget;
      _loudnessPick = switch (loudness?.lufs) {
        null when target != null && !_ownLoudness => _targetPick,
        null => null,
        final lufs when lufs == _compTarget?.lufs => _targetPick,
        final lufs when loudnessPresets.contains(lufs) => lufs,
        _ => _customPick,
      };
      if (loudness != null) {
        _truePeakDb = loudness.truePeakDb;
        if (_loudnessPick == _customPick) _customLufs = loudness.lufs;
      } else if (target != null && _loudnessPick == _targetPick) {
        _truePeakDb = target.truePeakDb;
      }
      _stems = stored.stems;
      _colourSpace = stored.colourSpace;
      _resample = stored.resample;
      _renderGuides = stored.renderGuides;
      _captions = stored.captions;
      _motionBlur = stored.motionBlur;
      _retimeBlend = stored.retimeBlend;
      _useProxies = stored.useProxies;
      _depth = stored.depth;
      _alphaChannel = stored.alphaChannel;
      _straightAlpha = stored.straightAlpha;
      _gifDither = stored.gifDither;
      _gifPalettePerFrame = stored.gifPalettePerFrame;
      _gifPlays = stored.gifPlays;
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

  /// Fill in the destination the defaults store asks for, so a person who
  /// always writes to the same place opens the dialog with a file name already
  /// in the Destination row.
  ///
  /// *Ask* leaves it empty, which is what the dialog has always done, and so
  /// does *beside the project* for a project never saved.
  void _seedDestination(BridgeExportDefaults defaults) {
    String? project;
    try {
      project = widget.project?.path();
    } catch (_) {
      // Closed meanwhile: no folder to be beside.
    }
    _path = exportDefaultPath(defaults,
            preset: _preset,
            compName: _compName,
            codec: _format.key,
            projectPath: project) ??
        _path;
    // A page has no folders, so only the name is chosen here.
    if (kIsWeb) {
      _path ??= exportPreset(
              preset: _preset,
              compName: _compName,
              template: defaults.filenameTemplate)
          .defaultName
          .replaceFirst(RegExp(r'\.[A-Za-z0-9]+$'), '.${_format.extension}');
    }
  }

  /// *Set as default*: remember the preset and the format in force as what the
  /// dialog opens on. The filename template and the destination policy are
  /// left exactly as they were — those are Settings ▸ Export's rows, and this
  /// button must not quietly answer questions it does not ask.
  void _setAsDefault() {
    final stored = exportDefaultsGet();
    try {
      exportDefaultsSet(
        defaults: BridgeExportDefaults(
          preset: _preset,
          codec: _format.key,
          filenameTemplate: stored.filenameTemplate,
          destination: stored.destination,
          folder: stored.folder,
        ),
      );
    } catch (error) {
      setState(() => _refused = '$error');
      return;
    }
    setState(() {
      _refused = null;
      _defaultPreset = _preset;
    });
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
            numbered: _format.type == ExportOutputType.imageSequence,
          );
    if (path != null) _edit(() => _path = _picked = path);
  }

  /// Queue the export, and show the queue. A refusal — a spec the encoder will
  /// not take — is shown in the footer where the summary was, rather than
  /// swallowed.
  Future<void> _queue({required bool start}) async {
    if (_path == null) await _choose();
    if (!mounted) return;
    var path = _path;
    if (path == null) return;
    // A sequence's frames were never in the Save dialogue's question, so its
    // first frame is always asked about.
    if ((path != _picked || _images || _captionsBeside) &&
        exportPathTaken(path: path, codec: _format.key, captions: _captions)) {
      final keep = await askLumitChoice<bool>(
        context: context,
        title: l10n.exportOverwriteTitle,
        question: l10n.exportOverwriteQuestion(_leaf(path)),
        answers: [
          (l10n.exportOverwrite, false),
          (l10n.exportKeepBoth, true),
        ],
      );
      if (keep == null || !mounted) return;
      if (keep) {
        path = exportFreePath(
            path: path, codec: _format.key, captions: _captions);
        setState(() => _path = path);
      }
    }
    setState(() {
      _refused = null;
      _refusedForAddon = false;
    });
    if (_lacking.isNotEmpty && _footage != _footageAsItIs) {
      return _queueWithFootage(path, start: start);
    }
    try {
      widget.comp.queueExport(spec: _spec, path: path, start: start);
    } catch (error) {
      // Which refusal this is, is read off the engine rather than off the
      // error: a `BridgeError` reaches Dart as an opaque handle with nothing
      // readable on it. The pre-flight turns an export away for one reason a
      // person can act on, a layer whose Flow engine names a model this machine
      // cannot run (docs/impl/addons.md §6.3), so the question put back to the
      // engine is the one the pre-flight asked, over this composition. Asking
      // the machine instead would call every blank path and every lost comp an
      // addon problem, on the usual machine with no addons installed.
      final addon = widget.comp.addonNeeded();
      setState(() {
        _refusedForAddon = addon;
        _refused = addon ? l10n.exportAddonMissing : '$error';
      });
      return;
    }
    // Wake the status line, which polls only while an export is live.
    if (start) statusLineExportStarted.value++;
    // The queue goes up **before** this dialogue comes down. Closing first
    // deactivates this element, and looking the Overlay up off a deactivated
    // context is the error that filled the screen after an export.
    showExportQueueFrb(context: context);
    widget.onClose();
  }

  /// Give the queued item these settings. A file it would newly write over is
  /// asked about first, as when it was queued.
  Future<void> _saveQueued() async {
    var path = _path;
    final id = widget.queued;
    if (path == null || id == null) return;
    if (path != _queuedPath &&
        (path != _picked || _images || _captionsBeside) &&
        exportPathTaken(path: path, codec: _format.key, captions: _captions)) {
      final keep = await askLumitChoice<bool>(
        context: context,
        title: l10n.exportOverwriteTitle,
        question: l10n.exportOverwriteQuestion(_leaf(path)),
        answers: [
          (l10n.exportOverwrite, false),
          (l10n.exportKeepBoth, true),
        ],
      );
      if (keep == null || !mounted) return;
      if (keep) {
        path = exportFreePath(
            path: path, codec: _format.key, captions: _captions);
      }
    }
    if (_failedItem) {
      try {
        widget.comp.queueExport(spec: _spec, path: path, start: false);
      } catch (error) {
        setState(() => _refused = '$error');
        return;
      }
      exportQueueRemove(id: id);
      widget.onClose();
      return;
    }
    try {
      exportQueueUpdate(id: id, spec: _spec, path: path);
    } catch (_) {
      // It started, or went, while the dialogue was open.
      setState(() => _refused = l10n.exportQueueNotWaiting);
      return;
    }
    widget.onClose();
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
