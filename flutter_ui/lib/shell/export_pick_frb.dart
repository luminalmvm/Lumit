// Add compositions: put several comps on the export queue at once.
//
// It opens on the comp in front, and the + under the list adds another from a
// dropdown of every comp, so a project with a lot of comps never becomes a
// long list. Each row has its own comp, preset and file. A new row takes the
// preset of the row above it, so a batch shares one preset unless a row is
// changed. Everything goes to one folder, named the way the export defaults
// name a file. Anything a preset doesn't cover is changed afterwards from the
// item's settings in the queue.
//
// The document is read once as the window opens and nothing in `build`
// crosses the bridge: the file names are worked out whenever a row changes.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../main.dart';
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'export_dialog_frb.dart';
import 'export_queue_frb.dart' show exportQueueListMax, exportQueueRow;
import 'status_line_frb.dart';

const double exportPickWidth = 600;
const double _compColumn = 180;
const double _presetColumn = 170;

Future<void> showExportPickFrb({
  required BuildContext context,
  List<(CompositionReference, String)>? comps,
  String? projectPath,
  CompositionReference? front,
}) {
  var found = comps;
  var project = projectPath;
  var current = front;
  if (found == null) {
    try {
      final app = context.read<LumitState>();
      found = app.comps();
      project ??= app.project?.path();
    } catch (_) {
      found = const [];
    }
  }
  try {
    current ??= context.read<LumitUiState>().selectedComp;
  } catch (_) {
    // No shell above, as in a widget test.
  }
  return showLumitModal<void>(
    context: context,
    id: 'export-pick',
    builder: (close) => _ExportPick(
      comps: found!,
      front: current,
      projectPath: project,
      onClose: () => close(null),
    ),
  );
}

/// One row: which comp, with which preset, to which file.
class _Pick {
  int comp;
  String preset;
  String file = '';
  _Pick(this.comp, this.preset);
}

class _ExportPick extends StatefulWidget {
  final List<(CompositionReference, String)> comps;
  final CompositionReference? front;
  final String? projectPath;
  final VoidCallback onClose;

  const _ExportPick({
    required this.comps,
    required this.front,
    required this.projectPath,
    required this.onClose,
  });

  @override
  State<_ExportPick> createState() => _ExportPickState();
}

class _ExportPickState extends State<_ExportPick> {
  late final BridgeExportDefaults _defaults = exportDefaultsGet();
  late final List<BridgeExportPresetEntry> _presets = exportPresetList();
  final List<_Pick> _rows = [];
  final ScrollController _scroll = ScrollController();

  /// Where every file goes. Empty until one is chosen when the defaults ask
  /// each time.
  String _folder = '';
  String? _refused;
  bool _busy = false;

  @override
  void initState() {
    super.initState();
    _folder = exportDefaultFolder(_defaults, projectPath: widget.projectPath);
    if (widget.comps.isEmpty) return;
    final front = widget.comps.indexWhere(
        (c) => c.$1.internalid == widget.front?.internalid);
    // The preset the dialogue would open on: the named default, else the
    // first built-in.
    final preset = _presets.any((p) => p.name == _defaults.preset)
        ? _defaults.preset
        : (_presets.firstOrNull?.name ?? '');
    _rows.add(_Pick(front < 0 ? 0 : front, preset));
    _name(_rows.first);
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  /// The row's settings: the preset as stored, with its own name on it.
  BridgeExportSpec? _spec(_Pick row) => exportPresetGet(name: row.preset);

  String _compName(int index) => widget.comps[index].$2;

  /// Work out the file a row writes. Called when its comp, its preset or the
  /// folder changes, never in `build`.
  void _name(_Pick row) {
    final codec = _spec(row)?.codec ?? 'h264';
    final path = _folder.isEmpty
        ? null
        : exportDefaultPath(
            BridgeExportDefaults(
              preset: _defaults.preset,
              codec: _defaults.codec,
              // With no template every comp would be called the preset's
              // own name, so a batch names each file after its comp.
              filenameTemplate: _defaults.filenameTemplate.trim().isEmpty
                  ? exportTokenComp
                  : _defaults.filenameTemplate,
              destination: exportDestinationFolder,
              folder: _folder,
            ),
            preset: row.preset,
            compName: _compName(row.comp),
            codec: codec);
    row.file = path ?? '';
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final canGo = !_busy && _rows.isNotEmpty && _folder.isNotEmpty;
    return DialogFrame(
      width: exportPickWidth,
      children: [
        dialogTitleBar(
          t,
          title: l10n.exportPickTitle,
          onClose: widget.onClose,
          keyPrefix: 'export-pick',
        ),
        Padding(
          padding: exportBodyPadding,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              dialogRow(
                t,
                l10n.exportWriteTo,
                Row(children: [
                  Expanded(
                    child: Text(
                      _folder.isEmpty ? l10n.exportNotChosen : _folder,
                      key: const ValueKey('export-pick-folder'),
                      style: dialogMono(t),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  const SizedBox(width: 8),
                  SizedBox(
                    height: dialogControlHeight,
                    child: HouseButton(
                      key: const ValueKey('export-pick-browse'),
                      padding: const EdgeInsets.symmetric(horizontal: 10),
                      onPressed: _busy ? null : _chooseFolder,
                      child: Text(l10n.exportBrowse, style: t.body),
                    ),
                  ),
                ]),
                labelColumn: exportLabelColumn,
                gap: exportRowGap,
              ),
              const SizedBox(height: 6),
              if (widget.comps.isEmpty)
                SizedBox(
                  height: exportQueueRow,
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: Text(l10n.exportPickNone,
                        style: t.body.copyWith(color: t.textMuted)),
                  ),
                )
              else ...[
                _header(t),
                ConstrainedBox(
                  constraints:
                      const BoxConstraints(maxHeight: exportQueueListMax),
                  child: dialogScroller(
                    context,
                    t,
                    _scroll,
                    child: ListView.builder(
                      controller: _scroll,
                      shrinkWrap: true,
                      itemCount: _rows.length,
                      itemBuilder: (context, i) => _row(t, _rows[i], i),
                    ),
                  ),
                ),
                const SizedBox(height: 4),
                Align(alignment: Alignment.centerLeft, child: _add(t)),
              ],
            ],
          ),
        ),
        dialogFooter(
          t,
          summary: _refused ?? l10n.exportPickSummary(_rows.length),
          keyPrefix: 'export-pick',
          actions: [
            HouseButton(
              key: const ValueKey('export-pick-cancel'),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              onPressed: widget.onClose,
              child: Text(l10n.cancel),
            ),
            dialogPlainAction(
              t,
              key: const ValueKey('export-pick-add'),
              label: l10n.exportAddToQueue,
              onPressed: canGo ? () => _queue(start: false) : null,
            ),
            dialogFilledAction(
              t,
              key: const ValueKey('export-pick-start'),
              label: l10n.exportAction,
              onPressed: canGo ? () => _queue(start: true) : null,
            ),
          ],
        ),
      ],
    );
  }

  /// The columns' names.
  Widget _header(LumitTheme t) => SizedBox(
        height: exportQueueRow,
        child: Row(children: [
          SizedBox(
            width: _compColumn,
            child: Text(l10n.exportPickComposition, style: t.kicker),
          ),
          const SizedBox(width: 10),
          SizedBox(
            width: _presetColumn,
            child: Text(l10n.exportPreset, style: t.kicker),
          ),
          const SizedBox(width: 10),
          Expanded(child: Text(l10n.exportPickFile, style: t.kicker)),
        ]),
      );

  Widget _row(LumitTheme t, _Pick row, int index) => SizedBox(
        key: ValueKey<String>('export-pick-row-$index'),
        height: exportQueueRow,
        child: Row(children: [
          dialogDropdown<int>(
            t,
            id: 'export-pick-comp-$index',
            width: _compColumn,
            value: row.comp,
            options: [for (var i = 0; i < widget.comps.length; i++) i],
            label: _compName,
            onChanged: (comp) => setState(() {
              row.comp = comp;
              _name(row);
            }),
          ),
          const SizedBox(width: 10),
          dialogDropdown<String>(
            t,
            id: 'export-pick-preset-$index',
            width: _presetColumn,
            value: row.preset,
            options: [for (final p in _presets) p.name],
            label: (name) => name,
            onChanged: (name) => setState(() {
              row.preset = name;
              _name(row);
            }),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Text(
              row.file.isEmpty ? '' : _leaf(row.file),
              style: dialogMono(t),
              overflow: TextOverflow.ellipsis,
            ),
          ),
          // The last row stays, so there is always something to export.
          if (_rows.length > 1)
            LumitTooltip(
              message: l10n.exportQueueRemove,
              child: HoverMark(
                builder: (context, lit) => GestureDetector(
                  key: ValueKey<String>('export-pick-remove-$index'),
                  behavior: HitTestBehavior.opaque,
                  onTap: () => setState(() => _rows.removeAt(index)),
                  child: SizedBox(
                    width: dialogCloseGlyph + 8,
                    height: exportQueueRow,
                    child: Center(
                      child: glyph.LumitIcon(
                        LumitIcons.close,
                        size: dialogCloseGlyph,
                        colour: Color.lerp(t.textMuted, t.textPrimary, lit),
                        semanticLabel: l10n.exportQueueRemove,
                      ),
                    ),
                  ),
                ),
              ),
            ),
        ]),
      );

  /// The + under the list: a dropdown of every comp, and the one picked
  /// becomes a new row with the preset of the row above it.
  Widget _add(LumitTheme t) => LumitTooltip(
        message: l10n.exportPickAdd,
        child: SizedBox(
          width: dialogControlHeight + 6,
          height: dialogControlHeight,
          child: BareDropdown<int>(
            key: const ValueKey('export-pick-more'),
            // No comp is the + itself, so none is marked in the list.
            value: -1,
            options: [for (var i = 0; i < widget.comps.length; i++) i],
            label: (i) => i < 0 ? '' : _compName(i),
            face: Text('+', style: t.body),
            onChanged: _busy
                ? null
                : (comp) => setState(() {
                      final row = _Pick(comp, _rows.lastOrNull?.preset ?? '');
                      _name(row);
                      _rows.add(row);
                    }),
          ),
        ),
      );

  Future<void> _chooseFolder() async {
    final folder = await pickFolder();
    if (folder == null || !mounted) return;
    setState(() {
      _folder = folder;
      for (final row in _rows) {
        _name(row);
      }
    });
  }

  /// Queue every row. Files already there are asked about once for the lot,
  /// and two rows that would write the same file never do: the second is
  /// given the next free name.
  Future<void> _queue({required bool start}) async {
    final rows = [for (final r in _rows) if (r.file.isNotEmpty) r];
    if (rows.isEmpty) return;
    final codecs = {for (final r in rows) r: _spec(r)?.codec ?? 'h264'};
    final taken = [
      for (final r in rows)
        if (exportPathTaken(path: r.file, codec: codecs[r]!)) r
    ];
    var keepBoth = false;
    if (taken.isNotEmpty) {
      final keep = await askLumitChoice<bool>(
        context: context,
        title: l10n.exportOverwriteTitle,
        question: taken.length == 1
            ? l10n.exportOverwriteQuestion(_leaf(taken.first.file))
            : l10n.exportPickTaken(taken.length),
        answers: [
          (l10n.exportOverwrite, false),
          (l10n.exportKeepBoth, true),
        ],
      );
      if (keep == null || !mounted) return;
      keepBoth = keep;
    }
    setState(() => _busy = true);
    final queued = <String>{};
    final failed = <_Pick>[];
    for (final row in rows) {
      final spec = _spec(row);
      if (spec == null) {
        failed.add(row);
        continue;
      }
      var path = row.file;
      if ((keepBoth || queued.contains(path)) &&
          exportPathTaken(path: path, codec: spec.codec)) {
        path = exportFreePath(path: path, codec: spec.codec);
      }
      try {
        // Held until the last one is in, so the queue runs them in the
        // order the list shows.
        widget.comps[row.comp].$1
            .queueExport(spec: spec, path: path, start: false);
        queued.add(path);
      } catch (_) {
        failed.add(row);
      }
    }
    if (start && queued.isNotEmpty) {
      exportQueueStart();
      // Wake the status line, which polls only while an export is live.
      statusLineExportStarted.value++;
    }
    if (!mounted) return;
    if (failed.isEmpty) return widget.onClose();
    // What went in leaves the list, so pressing again only tries the rest.
    setState(() {
      _busy = false;
      _rows.retainWhere(failed.contains);
      _refused = l10n.exportPickRefused(
          failed.map((r) => _compName(r.comp)).join(', '));
    });
  }

  static String _leaf(String path) => path.split(RegExp(r'[/\\]')).last;
}
