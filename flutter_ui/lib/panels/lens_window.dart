// The lens window, shared by Lens flare and Depth of field pro: pick a bundled
// lens, one of your own or a file, change any of it, and keep it in the
// effect or save it to your lenses. Bundled lenses are built in, so an edit
// to one is always a new lens.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:uuid/uuid.dart';

import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/lens.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/dialog_frame.dart';
import '../state/file_dialogs.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'shader_editor.dart' show InstanceHome;

const double lensWindowWidth = 900;
const double _bodyHeight = 500;
const double _listWidth = 230;
const double _rowHeight = 22;
const double _drawingHeight = 150;

/// Press a lens effect's Edit lens… row. Answers whether [param] was it.
bool pressLensButton({
  required BuildContext context,
  required InstanceHome home,
  required UuidValue effect,
  required String param,
  required VoidCallback onApplied,
}) {
  if (param != 'edit_lens') return false;
  unawaited(() async {
    if (await showLensWindow(context: context, home: home, effect: effect) &&
        context.mounted) {
      onApplied();
    }
  }());
  return true;
}

/// What the window hands back: a bundled lens untouched, or a lens to keep.
class _Chosen {
  final int? library;
  final BridgeLens? lens;
  const _Chosen.library(int this.library) : lens = null;
  const _Chosen.kept(BridgeLens this.lens) : library = null;
}

/// Open the window on one effect and commit what it returns. Answers whether
/// anything changed.
Future<bool> showLensWindow({
  required BuildContext context,
  required InstanceHome home,
  required UuidValue effect,
}) async {
  BridgeLens? kept;
  var model = 0;
  var found = false;
  for (final instance in home.read()) {
    if (instance.id() != effect) continue;
    kept = instance.keptLens();
    if (instance.getValue(id: 'lens_model')
        case BridgeEffectValue_Choice(:final field0)) {
      model = field0;
    }
    found = true;
  }
  if (!found || !context.mounted) return false;
  final chosen = await showLumitModal<_Chosen>(
    context: context,
    id: 'lens-window',
    builder: (close) => _LensWindow(
      kept: kept,
      model: model,
      onUse: close,
      onCancel: () => close(null),
    ),
  );
  if (chosen == null) return false;
  final stack = home.read();
  for (final instance in stack) {
    if (instance.id() != effect) continue;
    if (chosen.library case final index?) {
      instance.setValue(
          id: 'lens_model', value: BridgeEffectValue.choice(index));
      instance.setKeptLens(lens: null);
    } else {
      instance.setKeptLens(lens: chosen.lens);
    }
    try {
      home.commit(stack);
    } catch (_) {
      return false;
    }
    return true;
  }
  return false;
}

class _LensWindow extends StatefulWidget {
  final BridgeLens? kept;
  final int model;
  final ValueChanged<_Chosen> onUse;
  final VoidCallback onCancel;

  const _LensWindow({
    required this.kept,
    required this.model,
    required this.onUse,
    required this.onCancel,
  });

  @override
  State<_LensWindow> createState() => _LensWindowState();
}

class _LensWindowState extends State<_LensWindow> {
  late final List<String> _library = libraryLensNames();
  List<BridgeLensFile> _yours = userLenses();
  final _name = TextEditingController();

  /// Which list entry is open: `lib:3`, `file:<path>` or `kept`.
  late String _source;
  BridgeLens? _draft;
  bool _edited = false;
  List<BridgeLensLine> _lines = const [];
  String? _said;

  @override
  void initState() {
    super.initState();
    if (widget.kept case final kept?) {
      _open('kept', kept);
    } else {
      _openLibrary(widget.model);
    }
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  void _open(String source, BridgeLens? lens) {
    setState(() {
      _source = source;
      _draft = lens;
      _edited = false;
      _said = null;
      _name.text = lens?.name ?? '';
      _lines = lens == null ? const [] : lensDrawing(lens: lens);
    });
  }

  void _openLibrary(int index) =>
      _open('lib:$index', libraryLens(index: index));

  Future<void> _openFile(String path) async {
    try {
      _open('file:$path', await readLens(path: path));
    } catch (_) {
      setState(() => _said = l10n.lensNotALens);
    }
  }

  Future<void> _import() async {
    final path = await pickEffectInputFile(['lens'], l10n.lensFileType);
    if (path == null || !mounted) return;
    await _openFile(path);
  }

  void _edit(BridgeLens lens) => setState(() {
        _draft = lens;
        _edited = true;
        _lines = lensDrawing(lens: lens);
      });

  BridgeLens _named(BridgeLens l) =>
      BridgeLens(name: _name.text.trim(), focalMm: l.focalMm, rows: l.rows);

  void _setRow(int i, BridgeLensRow row) {
    final l = _draft!;
    _edit(BridgeLens(
      name: l.name,
      focalMm: l.focalMm,
      rows: [
        for (final (j, r) in l.rows.indexed)
          if (j == i)
            row
          // One stop: ticking a row clears the others.
          else if (row.stop && r.stop)
            _with(r, stop: false)
          else
            r,
      ],
    ));
  }

  void _removeRow(int i) {
    final l = _draft!;
    if (l.rows.length <= 3) return;
    _edit(BridgeLens(
      name: l.name,
      focalMm: l.focalMm,
      rows: [...l.rows]..removeAt(i),
    ));
  }

  void _addRow() {
    final l = _draft!;
    final last = l.rows.last;
    _edit(BridgeLens(
      name: l.name,
      focalMm: l.focalMm,
      rows: [...l.rows, _with(last, stop: false)],
    ));
  }

  Future<void> _save() async {
    final draft = _draft;
    if (draft == null) return;
    final lens = _named(draft);
    final path = _source.startsWith('file:') &&
            _yours.any((f) => 'file:${f.path}' == _source)
        ? _source.substring(5)
        : null;
    try {
      final saved = await saveUserLens(lens: lens, path: path);
      if (!mounted) return;
      setState(() => _yours = userLenses());
      _open('file:$saved', lens);
    } catch (_) {
      if (mounted) setState(() => _said = l10n.lensNotSaved);
    }
  }

  void _use() {
    final draft = _draft;
    if (draft == null) return;
    final name = _name.text.trim();
    if (_source.startsWith('lib:') && !_edited && name == draft.name) {
      widget.onUse(_Chosen.library(int.parse(_source.substring(4))));
    } else {
      widget.onUse(_Chosen.kept(_named(draft)));
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      width: lensWindowWidth,
      children: [
        dialogTitleBar(t,
            title: l10n.lensWindowTitle,
            onClose: widget.onCancel,
            keyPrefix: 'lens'),
        SizedBox(
          height: _bodyHeight,
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              SizedBox(width: _listWidth, child: _list(t)),
              Container(width: 1, color: t.hairline),
              Expanded(child: _detail(t)),
            ],
          ),
        ),
        dialogFooter(
          t,
          summary: _said ?? '',
          keyPrefix: 'lens',
          actions: [
            dialogPlainAction(t,
                key: const ValueKey('lens-cancel'),
                label: l10n.cancel,
                onPressed: widget.onCancel),
            dialogPlainAction(t,
                key: const ValueKey('lens-save'),
                label: l10n.lensSave,
                onPressed: _draft == null ? null : _save),
            dialogFilledAction(t,
                key: const ValueKey('lens-use'),
                label: l10n.lensUse,
                onPressed: _draft == null ? null : _use),
          ],
        ),
      ],
    );
  }

  Widget _heading(LumitTheme t, String text) => Padding(
        padding: const EdgeInsets.fromLTRB(10, 10, 10, 4),
        child: Text(t.kickerCase(text), style: t.kicker),
      );

  Widget _entry(LumitTheme t, String source, String label, VoidCallback onTap) {
    final on = _source == source;
    return GestureDetector(
      key: ValueKey('lens-entry-$source'),
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: Container(
        height: _rowHeight,
        color: on ? t.selectionFill : null,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        alignment: Alignment.centerLeft,
        child: Text(label,
            style: on ? t.bodyPrimary : t.body,
            overflow: TextOverflow.ellipsis),
      ),
    );
  }

  Widget _list(LumitTheme t) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Expanded(
            child: ListView(
              children: [
                if (widget.kept case final kept?) ...[
                  _heading(t, l10n.lensKept),
                  _entry(t, 'kept', kept.name, () => _open('kept', kept)),
                ],
                _heading(t, l10n.lensBuiltIn),
                for (final (i, name) in _library.indexed)
                  _entry(t, 'lib:$i', name, () => _openLibrary(i)),
                if (_yours.isNotEmpty) _heading(t, l10n.lensYours),
                for (final f in _yours)
                  _entry(t, 'file:${f.path}', f.name, () => _openFile(f.path)),
              ],
            ),
          ),
          Container(
            decoration: BoxDecoration(
                border: Border(top: BorderSide(color: t.hairline))),
            padding: const EdgeInsets.all(8),
            alignment: Alignment.centerLeft,
            child: SizedBox(
              height: dialogControlHeight,
              child: HouseButton(
                key: const ValueKey('lens-import'),
                onPressed: _import,
                child: Text(l10n.lensImport),
              ),
            ),
          ),
        ],
      );

  Widget _detail(LumitTheme t) {
    final lens = _draft;
    if (lens == null) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.all(dialogPadding),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(children: [
            HouseTextField(
              key: const ValueKey('lens-name'),
              controller: _name,
              width: 260,
            ),
            const SizedBox(width: 16),
            Text(l10n.lensFocalLength, style: t.body),
            const SizedBox(width: 8),
            SizedBox(
              width: 72,
              child: DragValueField(
                value: lens.focalMm,
                min: 1,
                max: 2000,
                decimals: 2,
                suffix: 'mm',
                onChanged: (v) => _edit(BridgeLens(
                    name: lens.name, focalMm: v.toDouble(), rows: lens.rows)),
              ),
            ),
          ]),
          const SizedBox(height: 10),
          SizedBox(
            height: _drawingHeight,
            child: CustomPaint(
              painter: _LensPainter(
                lines: _lines,
                glass: t.surface3,
                edge: t.textMuted,
                stop: t.accent,
                axis: t.hairline,
              ),
            ),
          ),
          const SizedBox(height: 10),
          _tableHead(t),
          Expanded(
            child: ListView(
              children: [
                for (final (i, r) in lens.rows.indexed) _tableRow(t, i, r),
                Padding(
                  padding: const EdgeInsets.only(top: 6),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: SizedBox(
                      height: dialogControlHeight,
                      child: HouseButton(
                        key: const ValueKey('lens-add-surface'),
                        onPressed: _addRow,
                        child: Text(l10n.lensAddSurface),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  static const _widths = [90.0, 80.0, 72.0, 60.0, 80.0, 56.0, 40.0];

  Widget _cell(double width, Widget child) => Padding(
        padding: const EdgeInsets.only(right: 6),
        child: SizedBox(width: width, child: child),
      );

  Widget _tableHead(LumitTheme t) {
    final labels = [
      l10n.lensRadius,
      l10n.lensThickness,
      l10n.lensIndex,
      l10n.lensAbbe,
      l10n.lensDiameter,
      l10n.lensCoating,
      l10n.lensStop,
    ];
    return Padding(
      padding: const EdgeInsets.only(left: 26, bottom: 4),
      child: Row(children: [
        for (final (i, label) in labels.indexed)
          _cell(_widths[i],
              Text(label, style: t.small, overflow: TextOverflow.ellipsis)),
      ]),
    );
  }

  Widget _tableRow(LumitTheme t, int i, BridgeLensRow r) {
    Widget field(num value, num min, num max, int decimals,
            BridgeLensRow Function(double) apply) =>
        DragValueField(
          value: value,
          min: min,
          max: max,
          decimals: decimals,
          speed: 0.1,
          onChanged: (v) => _setRow(i, apply(v.toDouble())),
        );
    return SizedBox(
      height: _rowHeight + 2,
      child: Row(children: [
        SizedBox(
          width: 26,
          child: Text('${i + 1}', style: t.mono.copyWith(color: t.textMuted)),
        ),
        _cell(
            _widths[0],
            r.stop
                ? const SizedBox.shrink()
                : field(r.radiusMm, -100000, 100000, 3,
                    (v) => _with(r, radiusMm: v))),
        _cell(_widths[1],
            field(r.thicknessMm, 0, 10000, 3, (v) => _with(r, thicknessMm: v))),
        _cell(_widths[2], field(r.ior, 1, 3, 5, (v) => _with(r, ior: v))),
        _cell(_widths[3], field(r.abbe, 0, 120, 2, (v) => _with(r, abbe: v))),
        _cell(_widths[4],
            field(r.semiApMm, 0.01, 1000, 3, (v) => _with(r, semiApMm: v))),
        _cell(
            _widths[5], field(r.coating, 0, 8, 0, (v) => _with(r, coating: v))),
        _cell(
            _widths[6],
            HouseCheckbox(
              value: r.stop,
              onChanged: (on) => _setRow(i, _with(r, stop: on)),
            )),
        GestureDetector(
          key: ValueKey('lens-remove-$i'),
          behavior: HitTestBehavior.opaque,
          onTap: () => _removeRow(i),
          child: glyph.LumitIcon(LumitIcons.close,
              size: 10, colour: t.textMuted, semanticLabel: l10n.lensRemove),
        ),
      ]),
    );
  }
}

BridgeLensRow _with(
  BridgeLensRow r, {
  double? radiusMm,
  double? thicknessMm,
  double? ior,
  double? abbe,
  double? semiApMm,
  double? coating,
  bool? stop,
}) =>
    BridgeLensRow(
      radiusMm: radiusMm ?? r.radiusMm,
      thicknessMm: thicknessMm ?? r.thicknessMm,
      ior: ior ?? r.ior,
      abbe: abbe ?? r.abbe,
      semiApMm: semiApMm ?? r.semiApMm,
      coating: coating ?? r.coating,
      stop: stop ?? r.stop,
    );

/// The cross-section, fitted to the box: glass filled between its two
/// surfaces, the stop in the accent, the axis as a hairline.
class _LensPainter extends CustomPainter {
  final List<BridgeLensLine> lines;
  final Color glass, edge, stop, axis;

  const _LensPainter({
    required this.lines,
    required this.glass,
    required this.edge,
    required this.stop,
    required this.axis,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (lines.isEmpty) return;
    var (minZ, maxZ, maxY) = (double.infinity, -double.infinity, 0.0);
    for (final l in lines) {
      for (var i = 0; i + 1 < l.points.length; i += 2) {
        minZ = minZ < l.points[i] ? minZ : l.points[i];
        maxZ = maxZ > l.points[i] ? maxZ : l.points[i];
        final y = l.points[i + 1].abs();
        maxY = maxY > y ? maxY : y;
      }
    }
    final spanZ = (maxZ - minZ).clamp(1e-3, double.infinity);
    final scale = [
      (size.width - 8) / spanZ,
      (size.height - 8) / (2 * maxY.clamp(1e-3, double.infinity)),
    ].reduce((a, b) => a < b ? a : b);
    final ox = (size.width - spanZ * scale) / 2 - minZ * scale;
    final oy = size.height / 2;
    Offset at(List<double> p, int i) =>
        Offset(ox + p[i] * scale, oy - p[i + 1] * scale);
    Path path(BridgeLensLine l) {
      final p = Path()..moveTo(at(l.points, 0).dx, at(l.points, 0).dy);
      for (var i = 2; i + 1 < l.points.length; i += 2) {
        final o = at(l.points, i);
        p.lineTo(o.dx, o.dy);
      }
      return p;
    }

    canvas.drawLine(
        Offset(0, oy), Offset(size.width, oy), Paint()..color = axis);
    for (var i = 0; i + 1 < lines.length; i++) {
      if (!lines[i].glassAfter) continue;
      final a = lines[i].points, b = lines[i + 1].points;
      final fill = Path()..moveTo(at(a, 0).dx, at(a, 0).dy);
      for (var j = 2; j + 1 < a.length; j += 2) {
        final o = at(a, j);
        fill.lineTo(o.dx, o.dy);
      }
      for (var j = b.length - 2; j >= 0; j -= 2) {
        final o = at(b, j);
        fill.lineTo(o.dx, o.dy);
      }
      fill.close();
      canvas.drawPath(fill, Paint()..color = glass);
    }
    for (final l in lines) {
      canvas.drawPath(
        path(l),
        Paint()
          ..color = l.stop ? stop : edge
          ..style = PaintingStyle.stroke
          ..strokeWidth = l.stop ? 2 : 1,
      );
    }
  }

  @override
  bool shouldRepaint(_LensPainter old) =>
      old.lines != lines ||
      old.glass != glass ||
      old.edge != edge ||
      old.stop != stop ||
      old.axis != axis;
}
