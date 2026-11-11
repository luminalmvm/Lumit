// Export rows: one file for each row of a graphics template's data.
//
// The rows are the Data window's selection, or every row. Each is queued as
// an export of its own, folded under one heading in the queue, or written as
// a still at the template's poster frame. The names, the faults and the
// export itself are the engine's; this asks once for each change of the
// pattern and draws the answer.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/template.dart';
import 'package:lumit_flutter/src/rust/api/template/copy.dart';
import 'package:lumit_flutter/src/rust/api/template/data.dart';
import 'package:lumit_flutter/src/rust/api/template/rows.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../main.dart';
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'export_dialog_frb.dart'
    show
        exportBodyPadding,
        exportDefaultFolder,
        exportLabelColumn,
        exportRowGap,
        exportTokenComp,
        exportTokenDate;
import 'export_queue_frb.dart' show showExportQueueFrb;
import 'status_line_frb.dart';

/// A selected copy of a template: its layer, its clip when it is one in Cut,
/// and what it says of itself.
typedef CopyHost = ({LayerReference layer, UuidValue? clip, BridgeCopy copy});

const double _width = 520;
const String _tokenRow = '{row}';

/// How long typing in the pattern settles before the names are asked again,
/// and how often a run of stills is asked how far it has got.
const Duration _settle = Duration(milliseconds: 150);
const Duration _poll = Duration(milliseconds: 200);

Future<void> showTemplateRowsExport({
  required BuildContext context,
  required CompositionReference comp,
  required String name,
  required BridgeData data,
  required BridgeTemplate? template,
  required List<int> rows,
  CopyHost? host,
  String? hostComp,
}) {
  String? project;
  try {
    project = context.read<LumitState>().project?.path();
  } catch (_) {
    // No project to ask, which leaves the folder to be chosen.
  }
  return showLumitModal<void>(
    context: context,
    id: 'template-rows-export',
    builder: (close) => _RowsExport(
      opener: context,
      comp: comp,
      name: name,
      data: data,
      template: template,
      rows: rows,
      host: host,
      hostComp: hostComp,
      projectPath: project,
      onClose: () => close(null),
    ),
  );
}

class _RowsExport extends StatefulWidget {
  final BuildContext opener;
  final CompositionReference comp;
  final String name;
  final BridgeData data;
  final BridgeTemplate? template;
  final List<int> rows;
  final CopyHost? host;
  final String? hostComp;
  final String? projectPath;
  final VoidCallback onClose;

  const _RowsExport({
    required this.opener,
    required this.comp,
    required this.name,
    required this.data,
    required this.template,
    required this.rows,
    required this.host,
    required this.hostComp,
    required this.projectPath,
    required this.onClose,
  });

  @override
  State<_RowsExport> createState() => _RowsExportState();
}

class _RowsExportState extends State<_RowsExport> {
  late final BridgeExportDefaults _defaults = exportDefaultsGet();
  late final List<BridgeExportPresetEntry> _presets = exportPresetList();
  final TextEditingController _pattern = TextEditingController();
  final FocusNode _patternFocus = FocusNode(debugLabel: 'rows-pattern');

  /// Whether the composition the copy sits in is exported, with the copy
  /// following each row, rather than the template on its own.
  bool _inHost = false;

  /// The preset, or null for a still of each row.
  String? _preset;
  String _folder = '';
  bool _skip = true;

  /// The rows with a fault, and the first names the queue will write.
  int _faulty = 0;
  List<String> _names = const [];

  Timer? _naming;
  Timer? _polling;
  BridgeStillsProgress? _progress;

  /// What the footer says once something has been tried: how many stills
  /// were written, or why nothing went.
  String? _outcome;
  bool _busy = false;

  /// The rows have gone to the queue, so a second Enter does nothing.
  bool _queued = false;

  @override
  void initState() {
    super.initState();
    _preset = _presets.any((p) => p.name == _defaults.preset)
        ? _defaults.preset
        : _presets.firstOrNull?.name;
    _folder = exportDefaultFolder(_defaults, projectPath: widget.projectPath);
    _pattern.text = _startingPattern();
    _pattern.addListener(_patternChanged);
    try {
      _faulty = widget.comp
          .checkDataRows(rows: widget.rows, pattern: _pattern.text)
          .faulty
          .length;
    } catch (_) {
      // The template went under the dialogue. Nothing is counted.
    }
    _askNames();
  }

  @override
  void dispose() {
    _naming?.cancel();
    _polling?.cancel();
    _pattern.dispose();
    _patternFocus.dispose();
    super.dispose();
  }

  /// `{<column>} {row}` for the column feeding the first text control, else
  /// `{comp} {row}`.
  String _startingPattern() {
    for (final c in widget.template?.controls ?? const <BridgeControl>[]) {
      if (c.kind != BridgeControlKind.text) continue;
      for (final column in widget.data.columns) {
        if (column.feeds == BridgeFeeds.control && column.control == c.id) {
          return '{${column.name}} $_tokenRow';
        }
      }
    }
    return '$exportTokenComp $_tokenRow';
  }

  void _patternChanged() {
    _naming?.cancel();
    _naming = Timer(_settle, () {
      if (mounted) setState(_askNames);
    });
  }

  void _askNames() {
    try {
      _names = widget.comp
          .checkDataRows(
              rows: widget.rows.take(3).toList(), pattern: _pattern.text)
          .names;
    } catch (_) {
      _names = const [];
    }
  }

  void _insert(String token) {
    final value = _pattern.value;
    final text = value.text;
    final at = value.selection.isValid
        ? value.selection
        : TextSelection.collapsed(offset: text.length);
    _pattern.value = TextEditingValue(
      text: text.replaceRange(at.start, at.end, token),
      selection: TextSelection.collapsed(offset: at.start + token.length),
    );
    _patternFocus.requestFocus();
  }

  int get _files => widget.rows.length - (_skip ? _faulty : 0);

  bool get _canGo =>
      !_busy &&
      !_queued &&
      _folder.isNotEmpty &&
      _pattern.text.trim().isNotEmpty &&
      _files > 0 &&
      _outcome == null;

  Future<void> _chooseFolder() async {
    final folder = await pickFolder();
    if (folder == null || !mounted) return;
    setState(() => _folder = folder);
  }

  Future<void> _export() async {
    if (!_canGo) return;
    final host = _inHost ? widget.host : null;
    final preset = _preset;
    if (preset == null) return _stills(host);
    final spec = exportPresetGet(name: preset);
    if (spec == null) {
      setState(() => _outcome = l10n.templateRowsFailed);
      return;
    }
    final int queued;
    try {
      queued = widget.comp.queueDataRows(
        spec: spec,
        rows: widget.rows,
        pattern: _pattern.text,
        folder: _folder,
        skipFaulty: _skip,
        host: host?.layer,
        hostClip: host?.clip,
        start: true,
      );
    } catch (_) {
      setState(() => _outcome = l10n.templateRowsFailed);
      return;
    }
    if (queued == 0) {
      setState(() => _outcome = l10n.templateRowsNone);
      return;
    }
    _queued = true;
    // Wake the status line, which polls only while an export is live.
    statusLineExportStarted.value++;
    widget.onClose();
    if (widget.opener.mounted) {
      unawaited(showExportQueueFrb(context: widget.opener));
    }
  }

  /// One still for each row at the poster frame, with how far it has got
  /// beside a Stop.
  Future<void> _stills(CopyHost? host) async {
    setState(() {
      _busy = true;
      _progress = null;
    });
    _polling = Timer.periodic(_poll, (_) {
      if (!mounted) return;
      try {
        setState(() => _progress = dataStillsProgress());
      } catch (_) {
        // No answer this time. The next one may have it.
      }
    });
    String outcome;
    try {
      final written = await widget.comp.exportDataStills(
        rows: widget.rows,
        pattern: _pattern.text,
        folder: _folder,
        skipFaulty: _skip,
        host: host?.layer,
        hostClip: host?.clip,
        frame: widget.template?.poster ?? 0,
      );
      outcome = l10n.templateRowsWritten(written);
    } catch (_) {
      outcome = l10n.templateRowsFailed;
    }
    _polling?.cancel();
    if (!mounted) return;
    setState(() {
      _busy = false;
      _progress = null;
      _outcome = outcome;
    });
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final progress = _progress;
    return DialogFrame(
      width: _width,
      children: [
        dialogTitleBar(
          t,
          title: l10n.templateRowsTitle,
          subject: widget.name,
          onClose: widget.onClose,
          keyPrefix: 'template-rows',
        ),
        Padding(
          padding: exportBodyPadding,
          child: dialogEnter(
            onEnter: () => unawaited(_export()),
            child: IgnorePointer(
              ignoring: _busy,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  _row(t, l10n.templateRowsComposition, _composition(t)),
                  _row(t, l10n.templateRowsFileName, _fileName(t)),
                  if (_names.isNotEmpty)
                    Padding(
                      padding: const EdgeInsetsDirectional.only(
                          start: exportLabelColumn + exportRowGap, bottom: 4),
                      child: Text(
                        _names.join('   '),
                        key: const ValueKey('template-rows-names'),
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: dialogMono(t),
                      ),
                    ),
                  _row(t, l10n.templateRowsFolder, _folderRow(t)),
                  _row(
                    t,
                    l10n.exportPreset,
                    dialogDropdown<String?>(
                      t,
                      id: 'template-rows-preset',
                      value: _preset,
                      options: [for (final p in _presets) p.name, null],
                      label: (name) => name ?? l10n.templateRowsStill,
                      onChanged: (name) => setState(() => _preset = name),
                    ),
                  ),
                  if (_faulty > 0)
                    _row(t, l10n.templateRowsFaults, _faults(t)),
                ],
              ),
            ),
          ),
        ),
        dialogFooter(
          t,
          keyPrefix: 'template-rows',
          leading: progress == null
              ? null
              : Row(children: [
                  SizedBox(
                    width: 120,
                    child: HouseProgressBar(
                        fraction: progress.total == 0
                            ? 0
                            : progress.done / progress.total),
                  ),
                  const SizedBox(width: 8),
                  Text(
                      l10n.templateRowsWriting(
                          progress.done, progress.total),
                      style: dialogMono(t)),
                ]),
          summary: _outcome ?? (_busy ? '' : l10n.templateRowsFiles(_files)),
          actions: _busy
              ? [
                  dialogPlainAction(
                    t,
                    key: const ValueKey('template-rows-stop'),
                    label: l10n.templateRowsStop,
                    onPressed: cancelDataStills,
                  ),
                ]
              : _outcome != null
                  ? [
                      dialogFilledAction(
                        t,
                        key: const ValueKey('template-rows-close'),
                        label: l10n.close,
                        onPressed: widget.onClose,
                      ),
                    ]
                  : [
                      dialogPlainAction(
                        t,
                        key: const ValueKey('template-rows-cancel'),
                        label: l10n.cancel,
                        onPressed: widget.onClose,
                      ),
                      dialogFilledAction(
                        t,
                        key: const ValueKey('template-rows-export'),
                        label: l10n.exportAction,
                        onPressed: _canGo ? () => unawaited(_export()) : null,
                      ),
                    ],
        ),
      ],
    );
  }

  Widget _row(LumitTheme t, String label, Widget control) => dialogRow(
        t,
        label,
        control,
        labelColumn: exportLabelColumn,
        gap: exportRowGap,
      );

  /// The template on its own, or the composition the selected copy sits in.
  Widget _composition(LumitTheme t) {
    final hostComp = widget.hostComp;
    if (widget.host == null || hostComp == null) {
      return Text(widget.name,
          key: const ValueKey('template-rows-comp'),
          overflow: TextOverflow.ellipsis,
          style: t.bodyPrimary);
    }
    return dialogDropdown<bool>(
      t,
      id: 'template-rows-comp',
      value: _inHost,
      options: const [false, true],
      label: (inHost) =>
          inHost ? l10n.templateRowsInHost(hostComp) : widget.name,
      onChanged: (inHost) => setState(() => _inHost = inHost),
    );
  }

  /// The pattern, and the menu that puts a token in at the cursor.
  Widget _fileName(LumitTheme t) => Row(children: [
        Expanded(
          child: SizedBox(
            height: dialogControlHeight,
            child: HouseTextField(
              key: const ValueKey('template-rows-pattern'),
              controller: _pattern,
              focusNode: _patternFocus,
              width: double.infinity,
              style: t.mono.copyWith(color: t.textPrimary),
              onSubmitted: (_) => unawaited(_export()),
            ),
          ),
        ),
        const SizedBox(width: 6),
        LumitTooltip(
          message: l10n.templateRowsInsert,
          child: Builder(
            builder: (context) => SizedBox(
              width: dialogControlHeight,
              height: dialogControlHeight,
              child: HouseButton(
                key: const ValueKey('template-rows-insert'),
                padding: EdgeInsets.zero,
                onPressed: () {
                  final box = context.findRenderObject()! as RenderBox;
                  showMenuAt<void>(
                    context: context,
                    position:
                        box.localToGlobal(Offset(0, box.size.height + 2)),
                    rows: (close) {
                      Widget token(String id, String label, String token) =>
                          MenuRow(
                            key: ValueKey<String>('template-rows-token-$id'),
                            onPressed: () {
                              close(null);
                              _insert(token);
                            },
                            child: MenuLabel(label),
                          );
                      return [
                        for (final (i, column) in widget.data.columns.indexed)
                          token('$i', column.name, '{${column.name}}'),
                        if (widget.data.columns.isNotEmpty) const MenuDivider(),
                        token('row', l10n.templateRowsRowNumber, _tokenRow),
                        token('comp', l10n.templateRowsCompName,
                            exportTokenComp),
                        token('date', l10n.templateRowsDate, exportTokenDate),
                      ];
                    },
                  );
                },
                child: glyph.LumitIcon(LumitIcons.more,
                    size: 12, colour: t.textSecondary),
              ),
            ),
          ),
        ),
      ]);

  Widget _folderRow(LumitTheme t) => Row(children: [
        Expanded(
          child: Text(
            _folder.isEmpty ? l10n.exportNotChosen : _folder,
            key: const ValueKey('template-rows-folder'),
            style: dialogMono(t),
            overflow: TextOverflow.ellipsis,
          ),
        ),
        const SizedBox(width: 8),
        SizedBox(
          height: dialogControlHeight,
          child: HouseButton(
            key: const ValueKey('template-rows-browse'),
            padding: const EdgeInsets.symmetric(horizontal: 10),
            onPressed: _chooseFolder,
            child: Text(l10n.exportBrowseEllipsis, style: t.body),
          ),
        ),
      ]);

  /// How many rows have a fault, and whether they are left out or use the
  /// template's values where they will not read.
  Widget _faults(LumitTheme t) => Row(children: [
        Text(l10n.templateDataRows(_faulty),
            key: const ValueKey('template-rows-faulty'),
            style: t.mono.copyWith(color: t.error)),
        const SizedBox(width: 12),
        Flexible(
          child: _radio(t, 'template-rows-skip', l10n.templateRowsSkip, _skip,
              () => setState(() => _skip = true)),
        ),
        const SizedBox(width: 12),
        Flexible(
          child: _radio(t, 'template-rows-use', l10n.templateRowsUseTemplate,
              !_skip, () => setState(() => _skip = false)),
        ),
      ]);

  Widget _radio(LumitTheme t, String key, String label, bool selected,
          VoidCallback onPick) =>
      GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onPick,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            HouseRadio(
                key: ValueKey(key), selected: selected, onChanged: onPick),
            const SizedBox(width: 6),
            Flexible(
              child: Text(label,
                  overflow: TextOverflow.ellipsis, style: t.bodyPrimary),
            ),
          ],
        ),
      );
}
