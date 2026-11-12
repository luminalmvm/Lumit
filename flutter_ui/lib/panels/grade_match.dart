// The Grade panel's Match and Look pages.
//
// Match sets the node from the picture: a balance, a match to another shot or
// a still, or a colour chart. Look is the LUT node, the saved looks, and the
// grade written out as a LUT for another application.

part of 'grade_panel_frb.dart';

const List<int> _lutSizes = [17, 33, 65];

mixin _GradeMatch on _GradeModel, _GradePrimaries {
  /// What Match to matches the shot to, when not the Viewer's reference.
  GradeReference? _matchTo;

  /// A solver in hand, so its buttons stand down until it answers.
  bool _solving = false;

  /// What Match chart found last, before and after, for the shot it ran on.
  (String, double, double)? _chartError;

  int _lutSize = 33;
  BridgeLutSpace _lutSpace = BridgeLutSpace.video;

  /// The look each shot is wearing and the grade it brought, by shot. A look
  /// isn't kept in the document, so this lasts until the grade is touched or
  /// its effects change.
  final Map<String, (String, List<UuidValue>)> _worn = {};

  // --- Match -------------------------------------------------------------------

  /// The comp's shots, read once per document revision.
  (int?, List<BridgeShot>)? _shotsRead;

  List<BridgeShot> get _shots {
    final comp = ui.selectedComp;
    if (comp == null) return const [];
    final revision = ui.model.revision;
    final held = _shotsRead;
    if (held != null && held.$1 == revision) return held.$2;
    List<BridgeShot> read;
    try {
      read = comp.shots();
    } catch (_) {
      read = const [];
    }
    _shotsRead = (revision, read);
    return read;
  }

  /// Run [solve] and write the rows it answers into the node as one step,
  /// the node it needs added in the same step.
  Future<void> _solveInto(
      Future<GradeRows?> Function(
              CompositionReference comp, GradeShot shot, int frame, UuidValue node)
          solve) async {
    final shot = _shot;
    final comp = ui.selectedComp;
    if (shot == null || comp == null || _solving) return;
    _touchStart();
    final node = _ensureOn(shot);
    if (node == null) {
      _touchCancel();
      return;
    }
    setState(() => _solving = true);
    GradeRows? rows;
    try {
      rows = await solve(comp, shot, ui.playheadFrame.value, node);
    } catch (_) {
      rows = null;
    }
    if (!mounted) return;
    setState(() => _solving = false);
    if (rows == null || rows.isEmpty) {
      _touchCancel();
      return;
    }
    _writeMany({for (final e in rows.entries) e.key: (_) => e.value},
        all: false);
  }

  Widget _matchPage(LumitTheme t, bool enabled) {
    final ready = enabled && _GradeModel._engine.solversReady && !_solving;
    final shot = _shot;
    final viewer = ui.grade.reference.value;
    final options = <GradeReference>[
      if (viewer != null) viewer,
      for (final s in _shots)
        if (shot == null || !GradeShot(s.layer, s.clip).same(shot))
          GradeReference.shot(GradeShot(s.layer, s.clip),
              frame: s.startFrame, name: s.name),
    ];
    final picked = _matchTo != null &&
            options.any((o) => _sameReference(o, _matchTo!))
        ? options.firstWhere((o) => _sameReference(o, _matchTo!))
        : options.firstOrNull;
    final error = _chartError;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SizedBox(height: 6),
        _actionRow(t, l10n.gradeBalance, [
          HouseButton(
            key: const ValueKey('grade-auto-balance'),
            small: true,
            onPressed: ready
                ? () => _solveInto((c, s, f, n) =>
                    _GradeModel._engine.autoBalance(c, s, f, n))
                : null,
            child: Text(l10n.gradeAutoBalance, style: t.small),
          ),
        ]),
        _actionRow(t, l10n.gradeMatchTo, [
          _capped(
            BareDropdown<GradeReference?>(
              key: const ValueKey('grade-match-to'),
              value: picked,
              options: options.isEmpty ? const [null] : options,
              label: (o) => o?.name ?? l10n.gradeNoShots,
              onChanged: enabled && options.isNotEmpty
                  ? (o) => setState(() => _matchTo = o)
                  : null,
            ),
          ),
          HouseButton(
            key: const ValueKey('grade-match'),
            small: true,
            onPressed: ready && picked != null
                ? () => _solveInto((c, s, f, n) =>
                    _GradeModel._engine.matchShot(c, s, f, n, picked))
                : null,
            child: Text(l10n.gradeMatch, style: t.small),
          ),
          HouseButton(
            key: const ValueKey('grade-compare'),
            small: true,
            onPressed: enabled && picked != null
                ? () => ui.grade.compareWith(picked)
                : null,
            child: Text(l10n.gradeCompare, style: t.small),
          ),
        ]),
        _actionRow(t, l10n.gradeChart, [
          HouseCheckbox(
            key: const ValueKey('grade-chart-show'),
            value: ui.grade.chart.value,
            label: l10n.gradeShowChart,
            onChanged:
                enabled ? (v) => setState(() => ui.grade.chart.value = v) : null,
          ),
          HouseButton(
            key: const ValueKey('grade-match-chart'),
            small: true,
            // The corners as they are when pressed: the Viewer moves them.
            onPressed: ready && ui.grade.chart.value
                ? () {
                    final at = ui.grade.chartCorners.value;
                    if (at != null) _matchChart(at);
                  }
                : null,
            child: Text(l10n.gradeMatchChart, style: t.small),
          ),
        ]),
        if (error != null && shot != null && error.$1 == shot.key)
          Padding(
            padding: const EdgeInsets.fromLTRB(gradeInset + 74, 0, 4, 4),
            child: Text(
              l10n.gradeChartError(
                  error.$2.toStringAsFixed(1), error.$3.toStringAsFixed(1)),
              style: t.small.copyWith(color: t.textMuted),
            ),
          ),
      ],
    );
  }

  static bool _sameReference(GradeReference a, GradeReference b) =>
      a.still != null ? a.still == b.still : a.shot?.same(b.shot) ?? false;

  /// The chart's rows, written as one step, and the error it leaves.
  Future<void> _matchChart(List<Offset> corners) async {
    final shot = _shot;
    GradeChartResult? found;
    await _solveInto((c, s, f, n) async {
      found = await _GradeModel._engine.matchChart(c, s, f, n, corners);
      return found?.rows;
    });
    final result = found;
    if (!mounted || result == null || shot == null) return;
    setState(() =>
        _chartError = (shot.key, result.errorBefore, result.errorAfter));
  }

  /// A labelled line of controls, as a slider row lays out its name. The
  /// controls wrap under one another in a narrow pane.
  Widget _actionRow(LumitTheme t, String label, List<Widget> children) =>
      Padding(
        padding: const EdgeInsets.fromLTRB(gradeInset, 2, 4, 4),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Padding(
              padding: const EdgeInsets.only(top: 3),
              child: SizedBox(
                width: 74,
                child: Text(label,
                    style: t.small.copyWith(color: t.textMuted),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis),
              ),
            ),
            Expanded(
              child: Wrap(
                spacing: 6,
                runSpacing: 4,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: children,
              ),
            ),
          ],
        ),
      );

  /// A control in an [_actionRow] that names something of any length.
  static Widget _capped(Widget child) =>
      ConstrainedBox(constraints: const BoxConstraints(maxWidth: 180), child: child);

  // --- Look --------------------------------------------------------------------

  Widget _lookPage(LumitTheme t, bool enabled) {
    final shot = _shot;
    final key = shot?.key;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SizedBox(height: 6),
        _actionRow(t, l10n.gradeLooks, [
          _capped(
            LookPicker(
              key: const ValueKey('grade-look'),
              enabled: enabled,
              current: _wornLook,
              onSave: enabled && canCopyGrade(ui) ? _saveLook : null,
              onPick: _pickLook,
            ),
          ),
        ]),
        ..._within('lut', () => _lutRows(t, enabled)),
        _actionRow(t, l10n.gradeExportLut, [
          BareDropdown<int>(
            key: const ValueKey('grade-lut-size'),
            value: _lutSize,
            options: _lutSizes,
            label: (n) => '$n',
            onChanged: enabled ? (n) => setState(() => _lutSize = n) : null,
          ),
          _capped(
            BareDropdown<BridgeLutSpace>(
              key: const ValueKey('grade-lut-space'),
              value: _lutSpace,
              options: BridgeLutSpace.values,
              label: (s) => switch (s) {
                BridgeLutSpace.video => l10n.gradeSpaceVideo,
                BridgeLutSpace.log => l10n.gradeSpaceLog,
                BridgeLutSpace.srgb => 'sRGB',
              },
              onChanged: enabled ? (s) => setState(() => _lutSpace = s) : null,
            ),
          ),
          HouseButton(
            key: const ValueKey('grade-export-lut'),
            small: true,
            onPressed: enabled && key != null && _anyPresent ? _exportLut : null,
            child: Text(l10n.menuExport, style: t.small),
          ),
        ]),
        _actionRow(t, l10n.gradeCdl, [
          HouseButton(
            key: const ValueKey('grade-cdl-import'),
            small: true,
            onPressed: enabled
                ? () => importCdlFile(context, app, ui,
                    after: _sharedRef?.id ?? _node?.id)
                : null,
            child: Text(l10n.menuImport, style: t.small),
          ),
          HouseButton(
            key: const ValueKey('grade-cdl-export'),
            small: true,
            onPressed: switch (_node) {
              final n? when enabled && cdlExportable(n.name) => () =>
                  exportCdlFile(app, ui, n.id),
              _ => null,
            },
            child: Text(l10n.menuExport, style: t.small),
          ),
        ]),
      ],
    );
  }

  /// The LUT node's file and its mix. Read and written on the shot's first
  /// LUT effect, which a chosen file adds after the grade.
  List<Widget> _lutRows(LumitTheme t, bool enabled) {
    final paths = switch (_value('file')) {
      BridgeEffectValue_File(:final field0) => field0.paths,
      _ => const <String>[],
    };
    final set = paths.isNotEmpty;
    final name = set ? paths.first.split(RegExp(r'[/\\]')).last : null;
    final filter = switch (_kind('file')) {
      BridgeParamKind_File(:final filterName) => engineLabel(filterName),
      _ => l10n.fxLut,
    };
    return [
      _actionRow(t, l10n.fxLut, [
        _capped(
          LumitTooltip(
            message: set ? paths.first : l10n.chooseA(filter),
            child: HouseButton(
              key: const ValueKey('grade-lut-file'),
              small: true,
              onPressed: !enabled ? null : _pickLut,
              child: Text(
                name ?? l10n.chooseEllipsis,
                style: t.small,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ),
        ),
        if (set)
          GradeIconButton(
            key: const ValueKey('grade-lut-clear'),
            glyph: LumitIcons.close,
            tip: l10n.clear,
            onPressed: () => _within('lut', () => _set({
                  'file': const BridgeEffectValue.file(BridgeFileParam(
                      paths: [], index: BridgeScalar.static_(0)))
                })),
          ),
      ]),
      if (set) _slider(t, 'mix', enabled, decimals: 0),
    ];
  }

  Future<void> _pickLut() async {
    final (extensions, label) = _within(
        'lut',
        () => switch (_kind('file')) {
              BridgeParamKind_File(:final filter, :final filterName) => (
                  filter,
                  filterName
                ),
              _ => (const <String>['cube'], 'LUT'),
            });
    final path = await pickEffectInputFile(extensions, label);
    if (path == null || !mounted) return;
    _within(
        'lut',
        () => _set({
              'file': BridgeEffectValue.file(BridgeFileParam(
                  paths: [path], index: const BridgeScalar.static_(0)))
            }));
  }

  /// The look on the shot, while its grade is still the one the look brought.
  String? get _wornLook {
    final shot = _shot;
    final worn = shot == null ? null : _worn[shot.key];
    if (shot == null || worn == null) return null;
    final ids = shotGradeIds(ui, shot);
    final same = ids.length == worn.$2.length &&
        Iterable.generate(ids.length).every((i) => ids[i] == worn.$2[i]);
    return same ? worn.$1 : null;
  }

  /// A look takes the place of the grade on every picked shot.
  Future<void> _pickLook(BridgePresetInfo look) async {
    final String text;
    try {
      text = await readEffectPreset(path: look.path);
    } catch (_) {
      return;
    }
    if (!mounted) return;
    final ui = this.ui;
    final targets = gradeTargets(ui);
    layGrade(app, ui, text, name: 'Apply look');
    ui.grade.setNode(null);
    if (!mounted) return;
    setState(() {
      for (final s in targets) {
        _worn[s.key] = (look.name, shotGradeIds(ui, s));
      }
    });
  }

  /// Write the shot's grade as a look, and show it as the look it wears.
  Future<void> _saveLook() async {
    final shot = _shot;
    if (shot == null) return;
    final ids = shotGradeIds(ui, shot);
    final name = await saveShotLook(context, app, ui, shot);
    if (name == null || !mounted) return;
    setState(() => _worn[shot.key] = (name, ids));
  }

  Future<void> _exportLut() =>
      exportShotLut(app, ui, size: _lutSize, space: _lutSpace);
}
