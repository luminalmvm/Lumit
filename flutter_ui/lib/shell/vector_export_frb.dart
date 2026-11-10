// The Lottie and SVG window: a composition written as a Lottie, a dotLottie
// or an animated SVG, with a player playing the exact bytes the export would
// write and a check of what won't come across, layer by layer.
//
// **Modeless.** It floats over the work, so a line in the check can be
// clicked to select its layer and the layer fixed while the window watches:
// every document change that touches the composition asks the engine for the
// export again, and the player swaps to the new file on the frame it was on.
//
// **The engine makes the file; this shows it.** `vector_preview` is the same
// call the export makes, in memory. Nothing here crosses the bridge in
// `build`: previews are asked for on open, on a settings change and on a
// document change, debounced, each with a rising epoch so a late answer to an
// old question is dropped.
//
// The settings and the path are the comp's remembered ones
// (`vector_settings`, `vector_path`), kept by the engine after each export.

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart' show revealInFolder;
import 'package:lumit_flutter/src/rust/api/state.dart' show ScopedChange;
import 'package:lumit_flutter/src/rust/api/vector.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../main.dart';
import '../panels/effect_param_row_frb.dart' show effectLabelOf;
import '../panels/viewer_panel_frb.dart' show viewerBackgroundColour;
import '../panels/viewer_strips.dart';
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/lottie_player.dart';
import '../widgets/web_player.dart'
    if (dart.library.js_interop) '../widgets/web_player_web.dart';
import 'dialog_frame.dart';
import 'export_queue_frb.dart' show fileSize;
import 'review_link_parts.dart' show reviewRowMenu;

part 'vector_export_check.dart';
part 'vector_export_player.dart';

/// The label column of the settings rows and the gap after it.
const double _labelColumn = 72;
const double _rowGap = 10;

/// How long a burst of edits settles before the export is asked for again.
const Duration _settle = Duration(milliseconds: 250);

/// The three formats, by the key the engine knows them by.
const List<String> _formats = ['lottie', 'dotlottie', 'svg'];

/// The picture qualities offered: lossless, then JPEG from best to smallest.
const List<int> _qualities = [0, 90, 75, 50];

/// The Lottie players the check judges against, in the engine's order.
const List<String> _lottiePlayers = [
  'web',
  'web_canvas',
  'dotlottie',
  'ios',
  'android',
  'flutter',
];

/// Open the window on the composition in front. Open already, it switches to
/// that composition instead: it is one window, bound to one comp at a time.
Future<void> showVectorExportFrb(BuildContext context) async {
  final ui = context.read<LumitUiState>();
  final comp = ui.selectedComp;
  if (comp == null) return;
  final open = _VectorWindowState._open;
  if (open != null) {
    open.switchTo(comp);
    return;
  }
  final app = context.read<LumitState>();
  await showLumitModal<void>(
    context: context,
    id: 'vector-export',
    modeless: true,
    initialSize: const Size(880, 600),
    minSize: const Size(640, 440),
    builder: (close) =>
        _VectorWindow(comp: comp, app: app, ui: ui, onClose: () => close(null)),
  );
}

class _VectorWindow extends StatefulWidget {
  final CompositionReference comp;
  final LumitState app;
  final LumitUiState ui;
  final VoidCallback onClose;

  const _VectorWindow({
    required this.comp,
    required this.app,
    required this.ui,
    required this.onClose,
  });

  @override
  State<_VectorWindow> createState() => _VectorWindowState();
}

/// A composition the title strip offers.
class _CompRow {
  final CompositionReference comp;
  final String name;
  final bool nodeGraph;
  const _CompRow(this.comp, this.name, this.nodeGraph);
}

class _VectorWindowState extends State<_VectorWindow>
    with SingleTickerProviderStateMixin {
  /// The one window up, so opening again switches it rather than stacking.
  static _VectorWindowState? _open;

  late CompositionReference _comp;
  late BridgeCompSettings _compSettings;
  bool _hasWorkArea = false;
  List<_CompRow> _comps = const [];
  final Map<UuidValue, bool> _nodeGraphs = {};

  /// The other compositions ticked into the batch, by id.
  final Set<UuidValue> _ticked = {};

  late BridgeVectorSettings _settings;
  String? _path;

  /// The path was typed or browsed here, so a format change keeps it and
  /// swaps the extension rather than asking the engine for a fresh one.
  bool _pathOwn = false;
  final TextEditingController _pathField = TextEditingController();

  /// The last export that came back whole, and what the newest ask said when
  /// it did not. The player keeps the last good file through an error.
  BridgeVectorPreview? _preview;
  String? _error;
  bool _busy = false;

  /// One count for every window, so a window opened later never asks with a
  /// number the engine has already seen. It starts from the clock, so a
  /// restart of the interface over a running engine counts on from above.
  static int _epoch = DateTime.now().millisecondsSinceEpoch;

  /// Each format's file ending, asked of the engine once.
  late final Map<String, String> _endings;

  /// The ticked comps' own checks, by comp.
  final Map<UuidValue, List<BridgeVectorIssue>> _others = {};

  StreamSubscription<ScopedChange>? _changes;
  Timer? _debounce;
  final ScrollController _scroll = ScrollController();

  // The player's state is in vector_export_player.dart.
  _Player _player = _Player.lottie;
  _Ground _ground = _Ground.board;

  /// The stage's magnification, or null to fit.
  double? _zoom;
  late final AnimationController _clock =
      AnimationController(vsync: this, duration: const Duration(seconds: 1));
  bool _playing = false;
  int _frame = 0;

  /// The player keeps to the Timeline: it shows the playhead's frame, and
  /// moving it moves the playhead.
  bool _follow = false;

  /// The window is moving the playhead itself, so the change it hears back
  /// is its own.
  bool _driving = false;
  late final WebPlayerController _web = WebPlayerController(
    onFrame: (frame) {
      if (mounted && _playerShown == _Player.browser) {
        setState(() => _frame = frame);
      }
    },
    onEnded: () {
      if (mounted && _playerShown == _Player.browser && _playing) {
        setState(() => _playing = false);
      }
    },
  );

  /// The check's lines opened to show their places.
  final Set<String> _openLines = {};

  /// `setState` for the part files, whose extensions cannot call it.
  void _redraw(VoidCallback change) => setState(change);

  /// The window's own keyboard. A control's node, so the shell's shortcuts
  /// stand down while it is held, as they do for any focused control; a
  /// click beside the window gives it up, and a click in it takes it back.
  final ControlFocusNode _focus = ControlFocusNode(debugLabel: 'vector');

  /// A write that failed, said in the footer until the next try.
  String? _writeError;

  @override
  void initState() {
    super.initState();
    _open = this;
    _endings = {for (final f in _formats) f: vectorExtension(format: f)};
    _clock.addListener(_tick);
    // The clock has stopped by its last tick, so the last frame is set here.
    _clock.addStatusListener((status) {
      if (status == AnimationStatus.completed && _playing) {
        setState(() {
          _playing = false;
          _frame = _frames - 1;
        });
      }
    });
    _changes = widget.app.onChange.listen(_changed);
    widget.ui.playheadFrame.addListener(_onPlayhead);
    widget.ui.playing.addListener(_onShellPlaying);
    _bind(widget.comp);
  }

  @override
  void dispose() {
    if (identical(_open, this)) _open = null;
    widget.ui.playheadFrame.removeListener(_onPlayhead);
    widget.ui.playing.removeListener(_onShellPlaying);
    _changes?.cancel();
    _debounce?.cancel();
    _clock.dispose();
    _pathField.dispose();
    _scroll.dispose();
    _focus.dispose();
    super.dispose();
  }

  // ---- the composition ------------------------------------------------------

  /// Bind the window to [comp]: its remembered settings and path, its own
  /// facts for the frame rate list, and a fresh export.
  void _bind(CompositionReference comp) {
    _playing = false;
    _clock.stop();
    _web.pause();
    _comp = comp;
    if (_follow) widget.ui.setSelectedComp(comp);
    _compSettings = comp.getSettings();
    _hasWorkArea = comp.getWorkArea() != null;
    _settings = comp.vectorSettings();
    if (!_hasWorkArea && _settings.workArea) {
      _settings = _copy(_settings, workArea: false);
    }
    // A rate or quality the controls don't offer becomes the one they show,
    // so the export uses what the window says.
    if (!_rates.contains((_settings.fpsNum, _settings.fpsDen))) {
      _settings = _copy(_settings, fpsNum: 0, fpsDen: 0);
    }
    if (!_qualities.contains(_settings.imageQuality)) {
      _settings = _copy(_settings, imageQuality: 0);
    }
    _pathOwn = false;
    _path = _askPath();
    _pathField.text = _path ?? '';
    _ticked.remove(comp.internalid);
    _readComps();
    _preview = null;
    _error = null;
    _others.clear();
    _openLines.clear();
    _frame = 0;
    _clock.value = 0;
    unawaited(_ask());
  }

  /// Switch the window to [comp], which is what opening it again does.
  void switchTo(CompositionReference comp) {
    if (comp.internalid == _comp.internalid) return;
    setState(() => _bind(comp));
  }

  String? _askPath() {
    try {
      return _comp.vectorPath(format: _settings.format);
    } catch (_) {
      return null;
    }
  }

  /// Every composition in the project, in the Project panel's order.
  void _readComps() {
    final project = widget.app.project;
    if (project == null) return;
    final rows = <_CompRow>[];
    void walk(List<ItemReference> items) {
      for (final item in items) {
        if (item case ItemReference_Composition(:final field0)) {
          final graph = _nodeGraphs[field0.internalid] ??=
              field0.getModel().isNodeGraph;
          rows.add(_CompRow(field0, item.name(), graph));
        } else if (item case ItemReference_Folder(:final field0)) {
          walk(field0.getChildren());
        }
      }
    }

    walk(project.getItems());
    _comps = rows;
    _ticked.retainAll({for (final r in rows) r.comp.internalid});
  }

  String get _compName => _comps
      .where((r) => r.comp.internalid == _comp.internalid)
      .map((r) => r.name)
      .firstOrNull ??
      _compSettings.name;

  /// The compositions whose edits concern this window.
  Set<UuidValue> get _watched => {
        _comp.internalid,
        ..._ticked,
        ...?_preview?.issues.map((i) => i.comp.internalid),
      };

  void _changed(ScopedChange change) {
    if (!mounted) return;
    if (change.items) {
      setState(_readComps);
    }
    final layer = change.layer;
    final item = change.item;
    final touches = layer != null
        ? _watched.contains(layer.internalcompId)
        : item != null
            ? item is ItemReference_Composition &&
                _watched.contains(item.field0.internalid)
            : !change.items;
    if (!touches) return;
    _debounce?.cancel();
    _debounce = Timer(_settle, () {
      if (!mounted) return;
      _compSettings = _comp.getSettings();
      _hasWorkArea = _comp.getWorkArea() != null;
      unawaited(_ask());
    });
  }

  // ---- the export, in memory --------------------------------------------------

  /// Ask the engine for the export as the settings stand. A newer ask stops
  /// an older one on the engine's side and drops its answer here.
  Future<void> _ask() async {
    final epoch = ++_epoch;
    setState(() => _busy = true);
    try {
      final preview =
          await _comp.vectorPreview(settings: _settings, epoch: epoch);
      if (!mounted || epoch != _epoch) return;
      setState(() {
        _preview = preview;
        _error = null;
        _busy = false;
      });
      _show(preview);
    } catch (_) {
      if (!mounted || epoch != _epoch) return;
      setState(() {
        _error = l10n.vectorPreviewFailed;
        _busy = false;
      });
    }
    for (final row in _comps) {
      if (!_ticked.contains(row.comp.internalid)) continue;
      try {
        final other =
            await row.comp.vectorPreview(settings: _settings, epoch: epoch);
        if (!mounted || epoch != _epoch) return;
        setState(() => _others[row.comp.internalid] = other.issues);
      } catch (_) {
        if (!mounted || epoch != _epoch) return;
        setState(() => _others.remove(row.comp.internalid));
      }
    }
  }

  void _askSoon() {
    _debounce?.cancel();
    _debounce = Timer(_settle, () {
      if (mounted) unawaited(_ask());
    });
  }

  /// Change the settings, and ask again.
  void _edit(BridgeVectorSettings next) {
    setState(() => _settings = next);
    _askSoon();
  }

  void _setFormat(String format) {
    if (format == _settings.format) return;
    final before = _playerShown;
    setState(() {
      _settings = _copy(_settings, format: format);
      if (_pathOwn && _path != null) {
        _path = _withExtension(_path!, _endings[format]!);
      } else {
        _path = _askPath();
      }
      _pathField.text = _path ?? '';
    });
    _handOff(before);
    _askSoon();
  }

  void _setPath(String text) {
    final typed = text.trim();
    setState(() {
      _path = typed.isEmpty ? null : typed;
      _pathOwn = typed.isNotEmpty;
      _writeError = null;
    });
  }

  Future<String?> _browse() async {
    final ext = _endings[_settings.format]!;
    final current = _path;
    final slash = current?.lastIndexOf(RegExp(r'[/\\]')) ?? -1;
    final picked = await pickExportSaveLocation(
      current == null
          ? '${_fileName(_compName)}.$ext'
          : current.substring(slash + 1),
      extension: ext,
      label: _formatName(_settings.format),
      initialDirectory: slash > 0 ? current!.substring(0, slash) : null,
    );
    if (picked == null || !mounted) return null;
    setState(() {
      _path = picked;
      _pathOwn = true;
      _pathField.text = picked;
    });
    return picked;
  }

  // ---- the export, on disk -----------------------------------------------------

  /// The last export came back whole, so the file can be written.
  bool get _canExport => _preview != null && _error == null;

  Future<void> _export() async {
    final path = _path ??
        (kIsWeb
            ? '${_fileName(_compName)}.${_endings[_settings.format]}'
            : await _browse());
    if (path == null || !mounted) return;
    final written = <String>[];
    final taken = {path.toLowerCase()};
    try {
      written.addAll(await _comp.vectorExport(settings: _settings, path: path));
      for (final row in _comps) {
        if (!_ticked.contains(row.comp.internalid)) continue;
        written.addAll(await row.comp.vectorExport(
            settings: _settings, path: _pathFor(row, path, taken)));
      }
    } catch (_) {
      if (mounted) setState(() => _writeError = l10n.workspaceFileUnwritable);
      return;
    }
    // A page has no folders: each file is handed over as a download.
    if (kIsWeb) {
      for (final file in written) {
        await handOver(file);
      }
    }
    if (!mounted) return;
    setState(() => _writeError = null);
    final slash = path.lastIndexOf(RegExp(r'[/\\]'));
    final files = 1 + _ticked.length;
    widget.app.postNotice(
      files == 1
          ? l10n.vectorExported(path.substring(slash + 1))
          : l10n.vectorExportedMany(files),
      action: kIsWeb ? null : l10n.projectShowInFolder,
      onTap: kIsWeb ? null : () => revealInFolder(path: written.first),
    );
  }

  /// Where a ticked comp's file goes: in the first file's folder, named after
  /// the comp, and numbered when a file in the batch already has that name.
  /// [taken] holds the batch's paths so far, lower case.
  String _pathFor(_CompRow row, String first, Set<String> taken) {
    final slash = first.lastIndexOf(RegExp(r'[/\\]'));
    final stem = '${first.substring(0, slash + 1)}${_fileName(row.name)}';
    final ext = _endings[_settings.format];
    var path = '$stem.$ext';
    for (var n = 2; !taken.add(path.toLowerCase()); n++) {
      path = '$stem $n.$ext';
    }
    return path;
  }

  // ---- keys -------------------------------------------------------------------

  KeyEventResult _key(FocusNode node, KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    // A key typed into the path field is the field's.
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        focused.findAncestorWidgetOfExactType<EditableText>() != null) {
      return KeyEventResult.ignored;
    }
    final frames = _preview?.frames ?? 0;
    switch (event.logicalKey) {
      case LogicalKeyboardKey.space:
        if (event is KeyRepeatEvent) return KeyEventResult.handled;
        _togglePlay();
      case LogicalKeyboardKey.arrowLeft:
        _step(-1);
      case LogicalKeyboardKey.arrowRight:
        _step(1);
      case LogicalKeyboardKey.home:
        _seek(0, stop: true);
      case LogicalKeyboardKey.end:
        _seek(frames - 1, stop: true);
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        if (event is KeyRepeatEvent) return KeyEventResult.handled;
        if (_canExport) unawaited(_export());
      default:
        return KeyEventResult.ignored;
    }
    return KeyEventResult.handled;
  }

  // ---- the window -------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Focus(
      focusNode: _focus,
      autofocus: true,
      onKeyEvent: _key,
      child: TapRegion(
        onTapOutside: (_) => _focus.unfocus(),
        onTapInside: (_) {
          final held = FocusManager.instance.primaryFocus;
          if (held == null || !held.ancestors.contains(_focus)) {
            _focus.requestFocus();
          }
        },
        child: DialogFrame(
          children: [
            dialogTitleBar(
              t,
              title: l10n.vectorTitle,
              subjectWidget: _compPicker(context, t),
              onClose: widget.onClose,
              keyPrefix: 'vector',
            ),
            Expanded(
              child: LayoutBuilder(
                builder: (context, box) {
                  final right = (box.maxWidth * 0.44).clamp(300.0, 380.0);
                  return Row(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Expanded(child: _playerColumn(context, t)),
                      SizedBox(
                          width: right, child: _settingsColumn(context, t)),
                    ],
                  );
                },
              ),
            ),
            dialogFooter(
              t,
              keyPrefix: 'vector',
              summary: _footerLine(),
              actions: [
                dialogFilledAction(
                  t,
                  key: const ValueKey<String>('vector-export'),
                  label: _ticked.isEmpty
                      ? l10n.exportAction
                      : l10n.vectorExportCount(1 + _ticked.length),
                  onPressed: _canExport ? () => unawaited(_export()) : null,
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  /// The footer's facts, or the one calm line about what went wrong.
  String _footerLine() {
    final error = _writeError ?? _error;
    if (error != null && !_busy) return error;
    final p = _preview;
    if (p == null) return '';
    return l10n.vectorFacts(_busy ? '–' : fileSize(p.size), p.frames,
        _fpsText(p.fps), p.width, p.height);
  }

  /// The composition's name as a dropdown of every composition, each with a
  /// tick that adds it to the batch. Picking a name switches the window.
  Widget _compPicker(BuildContext context, LumitTheme t) => Align(
        alignment: AlignmentDirectional.centerStart,
        child: Builder(
          builder: (context) => KeyedSubtree(
            key: const ValueKey<String>('vector-comp-picker'),
            child: dropdownButton(
              t: t,
              dense: false,
              onPressed: () => _openCompPicker(context),
              face: dropdownFace(
                t,
                _ticked.isEmpty
                    ? _compName
                    : '$_compName · ${1 + _ticked.length}',
              ),
            ),
          ),
        ),
      );

  Future<void> _openCompPicker(BuildContext anchor) async {
    final box = anchor.findRenderObject()! as RenderBox;
    await showLumitPopup<void>(
      context: anchor,
      position: box.localToGlobal(Offset(0, box.size.height + 2)),
      builder: (close) => FloatSurface(
        child: StatefulBuilder(
          builder: (context, refresh) {
            final t = ThemeScope.of(context).theme;
            return IntrinsicWidth(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final row in _comps)
                    Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Padding(
                          padding: const EdgeInsetsDirectional.only(start: 8),
                          child: HouseCheckbox(
                            key: ValueKey<String>(
                                'vector-tick-${row.comp.internalid}'),
                            value: row.comp.internalid == _comp.internalid ||
                                _ticked.contains(row.comp.internalid),
                            onChanged: row.nodeGraph ||
                                    row.comp.internalid == _comp.internalid
                                ? null
                                : (on) {
                                    setState(() {
                                      if (on) {
                                        _ticked.add(row.comp.internalid);
                                      } else {
                                        _ticked.remove(row.comp.internalid);
                                        _others.remove(row.comp.internalid);
                                      }
                                    });
                                    refresh(() {});
                                    _askSoon();
                                  },
                          ),
                        ),
                        Expanded(
                          child: MenuRow(
                            key: ValueKey<String>(
                                'vector-comp-${row.comp.internalid}'),
                            enabled: !row.nodeGraph,
                            selected: row.comp.internalid == _comp.internalid,
                            onPressed: () {
                              close(null);
                              switchTo(row.comp);
                            },
                            child: MenuLabel(row.name, enabled: !row.nodeGraph),
                          ),
                        ),
                      ],
                    ),
                  if (_comps.isEmpty)
                    Padding(
                      padding: const EdgeInsets.all(8),
                      child: Text(_compName,
                          style: t.body.copyWith(color: t.textMuted)),
                    ),
                ],
              ),
            );
          },
        ),
      ),
    );
  }

  /// Output and Content over the check, scrolling as one column.
  Widget _settingsColumn(BuildContext context, LumitTheme t) =>
      dialogScroller(
        context,
        t,
        _scroll,
        child: ListView(
          key: const ValueKey<String>('vector-settings'),
          controller: _scroll,
          padding: const EdgeInsets.fromLTRB(0, 4, dialogPadding, 10),
          children: [
            dialogGroup(t, l10n.exportGroupOutput, [
              _row(t, l10n.exportFormat, _formatDropdown(t)),
              _row(
                t,
                l10n.vectorWriteTo,
                Row(children: [
                  Expanded(
                    child: HouseTextField(
                      key: const ValueKey<String>('vector-path'),
                      controller: _pathField,
                      hint: l10n.exportNotChosen,
                      width: double.infinity,
                      submitOnLostFocus: true,
                      onSubmitted: _setPath,
                    ),
                  ),
                  // A page has no folders, so only the name is chosen here.
                  if (!kIsWeb) ...[
                    const SizedBox(width: 6),
                    SizedBox(
                      height: dialogControlHeight,
                      child: HouseButton(
                        key: const ValueKey<String>('vector-browse'),
                        padding: const EdgeInsets.symmetric(horizontal: 8),
                        onPressed: () => unawaited(_browse()),
                        child: Text(l10n.exportBrowseEllipsis),
                      ),
                    ),
                  ],
                ]),
              ),
            ]),
            dialogGroup(t, l10n.vectorGroupContent, [
              _row(
                t,
                l10n.vectorText,
                dialogDropdown<bool>(
                  t,
                  id: 'vector-text',
                  value: _settings.textAsShapes,
                  options: const [true, false],
                  label: (shapes) =>
                      shapes ? l10n.vectorTextShapes : l10n.vectorTextText,
                  onChanged: _settings.format == 'svg'
                      ? null
                      : (shapes) =>
                          _edit(_copy(_settings, textAsShapes: shapes)),
                ),
              ),
              _row(
                t,
                l10n.vectorImages,
                Row(children: [
                  Expanded(
                    child: dialogDropdown<bool>(
                      t,
                      id: 'vector-images',
                      value: _settings.embedImages,
                      options: const [true, false],
                      label: (embed) => embed
                          ? l10n.vectorImagesEmbed
                          : l10n.vectorImagesBeside,
                      onChanged: _settings.format == 'dotlottie'
                          ? null
                          : (embed) =>
                              _edit(_copy(_settings, embedImages: embed)),
                    ),
                  ),
                  const SizedBox(width: 6),
                  Expanded(
                    child: dialogDropdown<int>(
                      t,
                      id: 'vector-quality',
                      value: _settings.imageQuality,
                      options: _qualities,
                      label: (q) => q == 0
                          ? l10n.vectorImagesLossless
                          : l10n.vectorImagesJpeg(q),
                      onChanged: (q) =>
                          _edit(_copy(_settings, imageQuality: q)),
                    ),
                  ),
                ]),
              ),
              _row(t, l10n.frameRate, _rateDropdown(t)),
              _row(
                t,
                l10n.exportSpan,
                dialogDropdown<bool>(
                  t,
                  id: 'vector-span',
                  value: _settings.workArea && _hasWorkArea,
                  options: [if (_hasWorkArea) true, false],
                  label: (work) => work
                      ? l10n.vectorSpanWorkArea
                      : l10n.exportSpanWholeComp,
                  onChanged: (work) => _edit(_copy(_settings, workArea: work)),
                ),
              ),
              _row(
                t,
                '',
                Align(
                  alignment: AlignmentDirectional.centerStart,
                  child: HouseCheckbox(
                    key: const ValueKey<String>('vector-minify'),
                    value: _settings.minify,
                    label: l10n.vectorMinify,
                    onChanged: (on) => _edit(_copy(_settings, minify: on)),
                  ),
                ),
              ),
            ]),
            dialogGroup(t, l10n.vectorGroupCheck, [
              if (_settings.format != 'svg')
                _row(t, l10n.vectorPlayers, _playersMenu(context, t)),
              ..._checkLines(context, t),
            ]),
          ],
        ),
      );

  Widget _row(LumitTheme t, String label, Widget control) =>
      dialogRow(t, label, control, labelColumn: _labelColumn, gap: _rowGap);

  /// Format, with each file extension dimmed beside its name.
  Widget _formatDropdown(LumitTheme t) => SizedBox(
        height: dialogControlHeight,
        child: BareDropdown<String>(
          key: const ValueKey<String>('vector-format'),
          value: _settings.format,
          options: _formats,
          label: _formatName,
          note: (f) => '.${_endings[f]}',
          face: Row(mainAxisSize: MainAxisSize.min, children: [
            Text(_formatName(_settings.format)),
            const SizedBox(width: 6),
            Text('.${_endings[_settings.format]}',
                style: TextStyle(color: t.textMuted)),
          ]),
          onChanged: _setFormat,
        ),
      );

  /// The comp's own rate first, then the usual ones, and the comp's own
  /// number among them when it is not one of those.
  List<(int, int)> get _rates {
    final own = (_compSettings.fpsNum, _compSettings.fpsDen);
    const usual = [(24, 1), (25, 1), (30, 1), (50, 1), (60, 1)];
    return [(0, 0), ...usual, if (!usual.contains(own)) own];
  }

  Widget _rateDropdown(LumitTheme t) {
    final own = (_compSettings.fpsNum, _compSettings.fpsDen);
    final options = _rates;
    var value = (_settings.fpsNum, _settings.fpsDen);
    if (!options.contains(value)) value = (0, 0);
    return dialogDropdown<(int, int)>(
      t,
      id: 'vector-rate',
      value: value,
      options: options,
      label: (r) => r == (0, 0)
          ? l10n.vectorRateComposition(_fpsText(own.$1 / own.$2))
          : _fpsText(r.$1 / r.$2),
      onChanged: (r) => _edit(_copy(_settings, fpsNum: r.$1, fpsDen: r.$2)),
    );
  }

  /// The players the check judges against: a menu of ticks that stays open
  /// while they are set.
  Widget _playersMenu(BuildContext context, LumitTheme t) {
    final ticked = _lottiePlayers.where(_settings.players.contains).length;
    return Align(
      alignment: AlignmentDirectional.centerStart,
      child: Builder(
        builder: (anchor) => SizedBox(
          height: dialogControlHeight,
          child: dropdownButton(
            t: t,
            dense: false,
            onPressed: () => _openPlayersMenu(anchor),
            face: dropdownFace(
              t,
              ticked == _lottiePlayers.length
                  ? l10n.vectorPlayersAll
                  : l10n.vectorPlayersSome(ticked, _lottiePlayers.length),
            ),
          ),
        ),
      ),
    );
  }

  Future<void> _openPlayersMenu(BuildContext anchor) async {
    final box = anchor.findRenderObject()! as RenderBox;
    await showLumitPopup<void>(
      context: anchor,
      position: box.localToGlobal(Offset(0, box.size.height + 2)),
      builder: (close) => FloatSurface(
        child: StatefulBuilder(
          builder: (context, refresh) => IntrinsicWidth(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                for (final player in _lottiePlayers)
                  MenuRow(
                    key: ValueKey<String>('vector-player-$player'),
                    onPressed: () {
                      final on = !_settings.players.contains(player);
                      final players = [
                        for (final p in _lottiePlayers)
                          if (p == player ? on : _settings.players.contains(p))
                            p,
                      ];
                      refresh(() {});
                      _edit(_copy(_settings, players: players));
                    },
                    child: MenuLabel(playerName(player),
                        ticked: _settings.players.contains(player)),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// A player's name for the reader, by the engine's key.
String playerName(String key) => switch (key) {
      'web' => l10n.vectorPlayerWeb,
      'web_canvas' => l10n.vectorPlayerWebCanvas,
      'dotlottie' => l10n.vectorPlayerDotLottie,
      'ios' => l10n.vectorPlayerIos,
      'android' => l10n.vectorPlayerAndroid,
      'flutter' => l10n.vectorPlayerFlutter,
      'chrome' => l10n.vectorPlayerChrome,
      'firefox' => l10n.vectorPlayerFirefox,
      'safari' => l10n.vectorPlayerSafari,
      _ => key,
    };

String _formatName(String format) => switch (format) {
      'dotlottie' => l10n.vectorFormatDotLottie,
      'svg' => l10n.vectorFormatSvg,
      _ => l10n.vectorFormatLottie,
    };

/// `29.97`, or `30` when it is whole.
String _fpsText(double fps) =>
    fps == fps.roundToDouble() ? fps.round().toString() : fps.toStringAsFixed(2);

String _withExtension(String path, String ext) {
  final slash = path.lastIndexOf(RegExp(r'[/\\]'));
  final dot = path.lastIndexOf('.');
  return dot > slash ? '${path.substring(0, dot)}.$ext' : '$path.$ext';
}

/// [name] as a file name: the characters no file system takes are dropped.
String _fileName(String name) {
  final clean = name.replaceAll(RegExp(r'[<>:"/\\|?*\x00-\x1f]'), '').trim();
  return clean.isEmpty ? 'animation' : clean;
}

BridgeVectorSettings _copy(
  BridgeVectorSettings s, {
  String? format,
  bool? textAsShapes,
  bool? embedImages,
  int? fpsNum,
  int? fpsDen,
  bool? workArea,
  bool? minify,
  List<String>? players,
  bool? loopPlayback,
  int? imageQuality,
}) =>
    BridgeVectorSettings(
      format: format ?? s.format,
      textAsShapes: textAsShapes ?? s.textAsShapes,
      embedImages: embedImages ?? s.embedImages,
      fpsNum: fpsNum ?? s.fpsNum,
      fpsDen: fpsDen ?? s.fpsDen,
      workArea: workArea ?? s.workArea,
      minify: minify ?? s.minify,
      players: players ?? s.players,
      loopPlayback: loopPlayback ?? s.loopPlayback,
      imageQuality: imageQuality ?? s.imageQuality,
    );
