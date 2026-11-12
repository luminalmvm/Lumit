// The Grade panel's Qualifier page: which part of the picture the node
// grades, picked by hue, saturation and luma, by channel, or by luma alone,
// then cleaned up.

part of 'grade_panel_frb.dart';

const List<String> _finesseRows = [
  'qual_denoise',
  'qual_clean_black',
  'qual_clean_white',
  'qual_black_clip',
  'qual_white_clip',
  'qual_blur',
  'qual_grow',
];

mixin _GradeQualifier on _GradeModel, _GradePrimaries {
  int get _qualMode => _choice('qual_mode');

  Widget _qualifierPage(LumitTheme t, bool enabled) {
    final mode = _qualMode;
    final options = switch (_kind('qual_mode')) {
      BridgeParamKind_Choice(:final options) => options,
      _ => const <String>[],
    };
    final grey = [hsvToRgb(0, 0, 0.05), hsvToRgb(0, 0, 0.95)];
    final bars = [
      if (mode == 1) ...[
        _hueBar(t, enabled),
        _bandBar(t, enabled, l10n.gradeSaturation, 'qual_sat_low',
            'qual_sat_high', 'qual_sat_soft',
            use: 'qual_use_sat',
            colours: [hsvToRgb(30, 0, 0.6), hsvToRgb(30, 0.75, 0.85)]),
        _bandBar(t, enabled, l10n.gradeLuma, 'qual_lum_low', 'qual_lum_high',
            'qual_lum_soft',
            use: 'qual_use_lum', colours: grey),
      ],
      if (mode == 2)
        for (final (c, hue, name) in [
          ('red', 0.0, l10n.gradeRed),
          ('green', 120.0, l10n.gradeGreen),
          ('blue', 240.0, l10n.gradeBlue),
        ])
          _bandBar(t, enabled, name, 'qual_${c}_low', 'qual_${c}_high',
              'qual_rgb_soft',
              colours: [hsvToRgb(hue, 1, 0.05), hsvToRgb(hue, 0.8, 0.9)]),
      if (mode == 3)
        _bandBar(t, enabled, l10n.gradeLuma, 'qual_lum_low', 'qual_lum_high',
            'qual_lum_soft',
            colours: grey),
      if (mode == 4) ...[
        _samplesRow(t, enabled, drop: false),
        _samplesRow(t, enabled, drop: true),
        const SizedBox(height: 2),
        _flow([
          _slider(t, 'qual_3d_size', enabled, decimals: 1),
          _slider(t, 'qual_3d_soft', enabled, decimals: 1),
        ]),
      ],
    ];
    final finesse = [
      GradeMenu(
        rows: (close) => _menuFor(close, rows: _finesseRows),
        child: GradeHeading(
          label: l10n.gradeFinesse,
          changed: enabled && _anyChanged(_finesseRows),
          onReset: () => _reset(_finesseRows),
        ),
      ),
      _flow([
        for (final id in _finesseRows) _slider(t, id, enabled, decimals: 1),
        _toggle(t, 'qual_invert', enabled),
      ]),
    ];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(gradeInset, 6, 4, 6),
          child: Row(
            children: [
              Flexible(
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Segment(children: [
                    for (var i = 0; i < options.length; i++)
                      SegmentOption(
                        key: ValueKey<String>('grade-qual-mode-$i'),
                        active: i == mode,
                        onTap: enabled && i != mode
                            ? () => _set({'qual_mode': BridgeEffectValue.choice(i)})
                            : () {},
                        child: Text(engineLabel(options[i]),
                            style: t.small.copyWith(
                                color: i == mode
                                    ? t.textPrimary
                                    : t.textSecondary)),
                      ),
                  ]),
                ),
              ),
              const SizedBox(width: 8),
              for (final pick in GradePick.values) _pickButton(pick, enabled),
            ],
          ),
        ),
        if (mode != 0)
          // The ranges and the finesse side by side once both have room.
          LayoutBuilder(builder: (context, box) {
            if (box.maxWidth < 1000) {
              return Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [...bars, ...finesse],
              );
            }
            return Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Expanded(
                  flex: 3,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: bars,
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  flex: 2,
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: finesse,
                  ),
                ),
              ],
            );
          }),
      ],
    );
  }

  /// The 3D key's kept samples, or its dropped ones: how many, each one's
  /// colour, and Clear.
  Widget _samplesRow(LumitTheme t, bool enabled, {required bool drop}) {
    final id = drop ? 'qual_3d_drop' : 'qual_3d_keep';
    final luma = '${id}_luma';
    final points = switch (_value(id)) {
      BridgeEffectValue_Shape(:final field0) => field0,
      _ => const <Float32List>[],
    };
    final lumas = switch (_value(luma)) {
      BridgeEffectValue_Shape(:final field0) => field0,
      _ => const <Float32List>[],
    };
    // A sample is Cb and Cr doubled, its luma beside it: back to a colour
    // in video gamma for its swatch.
    Color swatch(int i) {
      final p = points[i];
      final y = i < lumas.length && lumas[i].isNotEmpty ? lumas[i][0] : 0.5;
      final cb = p[0] / 2, cr = p[1] / 2;
      final r = y + 1.5748 * cr, b = y + 1.8556 * cb;
      final g = (y - 0.2126 * r - 0.0722 * b) / 0.7152;
      return Color.from(
          alpha: 1,
          red: r.clamp(0.0, 1.0),
          green: g.clamp(0.0, 1.0),
          blue: b.clamp(0.0, 1.0));
    }

    return GradeMenu(
      rows: (close) => _menuFor(close, rows: [id, luma]),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(gradeInset, 3, gradeInset, 3),
        child: Row(
          children: [
            SizedBox(
              width: 74,
              child: Text(drop ? l10n.gradeQualDrop : l10n.gradeQualKeep,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: t.small.copyWith(
                      color: !enabled
                          ? t.textDisabled
                          : points.isEmpty
                              ? t.textMuted
                              : t.textSecondary)),
            ),
            SizedBox(
              width: 24,
              child: Text('${points.length}',
                  key: ValueKey<String>('grade-qual-3d-count-$id'),
                  style: t.small.copyWith(
                      color: enabled ? t.textSecondary : t.textDisabled)),
            ),
            Expanded(
              child: ClipRect(
                child: Wrap(
                  spacing: 2,
                  runSpacing: 2,
                  children: [
                    for (var i = 0; i < points.length; i++)
                      if (points[i].length >= 2)
                        Container(
                          width: 10,
                          height: 10,
                          decoration: BoxDecoration(
                            color: swatch(i),
                            border: Border.all(color: t.hairlineStrong),
                            borderRadius: BorderRadius.circular(2),
                          ),
                        ),
                  ],
                ),
              ),
            ),
            HouseButton(
              key: ValueKey<String>('grade-qual-3d-clear-$id'),
              small: true,
              frameless: true,
              onPressed: enabled && points.isNotEmpty
                  ? () => _clearSamples(drop: drop)
                  : null,
              child: Text(l10n.gradeQualClear, style: t.small),
            ),
          ],
        ),
      ),
    );
  }

  /// The kept samples, or the dropped ones, gone as one step, and the
  /// strokes that made them with them.
  void _clearSamples({required bool drop}) {
    final id = drop ? 'qual_3d_drop' : 'qual_3d_keep';
    final node = _node;
    _set({
      id: const BridgeEffectValue.shape([]),
      '${id}_luma': const BridgeEffectValue.shape([]),
    });
    if (node != null) ui.grade.clearStrokes(node.id, drop: drop);
  }

  /// A pick tool: arms the picture, where a drag takes the samples it reads.
  /// The solver turns them into ranges.
  Widget _pickButton(GradePick pick, bool enabled) {
    final ready = enabled && _GradeModel._engine.solversReady;
    final armed = ui.grade.pick.value == pick;
    return GradeIconButton(
      key: ValueKey<String>('grade-qual-${pick.name}'),
      glyph: switch (pick) {
        GradePick.pick => LumitIcons.eyedropper,
        GradePick.add => LumitIcons.pickAdd,
        GradePick.subtract => LumitIcons.pickSubtract,
      },
      tip: switch (pick) {
        GradePick.pick => l10n.gradePick,
        GradePick.add => l10n.gradePickAdd,
        GradePick.subtract => l10n.gradePickSubtract,
      },
      active: armed,
      onPressed: !ready ? null : () => _armPick(armed ? null : pick),
    );
  }

  /// The tool on the picture, or none. A drag on the Viewer draws the
  /// circle it reads.
  void _armPick(GradePick? pick) => setState(() => ui.grade.pick.value = pick);

  /// Highlight: the Viewer shows where the node's key is, grey elsewhere. A
  /// right-click offers the key in black and white.
  Widget _highlightButton(bool enabled) {
    final ready = enabled && _GradeModel._engine.matteReady;
    final mode = ui.grade.matte.value;
    return GradeMenu(
      rows: (close) => !ready
          ? const []
          : [
              for (final m in GradeMatte.values)
                _row(
                    'grade-matte-${m.name}',
                    switch (m) {
                      GradeMatte.off => l10n.gradeMatteOff,
                      GradeMatte.highlight => l10n.gradeHighlight,
                      GradeMatte.key => l10n.gradeMatteKey,
                    },
                    close,
                    () => _setMatte(m),
                    ticked: m == mode),
            ],
      child: GradeIconButton(
        key: const ValueKey('grade-highlight'),
        glyph: LumitIcons.matte,
        tip: l10n.gradeHighlight,
        active: mode != GradeMatte.off,
        onPressed: !ready
            ? null
            : () => _setMatte(
                mode == GradeMatte.off ? GradeMatte.highlight : GradeMatte.off),
      ),
    );
  }

  /// The Viewer keeps the engine's matte view in step with this.
  void _setMatte(GradeMatte mode) => ui.grade.matte.value = mode;

  /// A bar's name, with its on and off tick where the qualifier can leave it
  /// out.
  Widget _barLabel(LumitTheme t, String label, bool enabled, String? use,
      List<String> rows) {
    final changed = _anyChanged(rows);
    final text = Text(label,
        style: t.small.copyWith(
            color: !enabled
                ? t.textDisabled
                : changed
                    ? t.textSecondary
                    : t.textMuted),
        maxLines: 1,
        overflow: TextOverflow.ellipsis);
    return SizedBox(
      width: 74,
      child: use == null
          ? text
          : Row(
              children: [
                HouseCheckbox(
                  key: ValueKey<String>('grade-tick-$use'),
                  value: _bool(use),
                  onChanged: enabled
                      ? (v) => _set({use: BridgeEffectValue.bool(v)})
                      : null,
                ),
                const SizedBox(width: 4),
                Flexible(child: text),
              ],
            ),
    );
  }

  /// The hue range: centre, width, softness and symmetry, in degrees and
  /// per cent, drawn as edges on the hue circle.
  Widget _hueBar(LumitTheme t, bool enabled) {
    const rows = ['qual_hue', 'qual_hue_width', 'qual_hue_soft', 'qual_hue_sym'];
    final c = _float('qual_hue');
    final w = _float('qual_hue_width');
    final s = _float('qual_hue_soft');
    final sym = _float('qual_hue_sym') / 100;
    final range = (
      low: (c - w / 2 * (1 - sym)) / 360,
      high: (c + w / 2 * (1 + sym)) / 360,
      soft: s / 360,
    );
    Map<String, double> rowsOf(GradeRange r) {
      // An edge moved keeps the centre where it is; the whole range moved
      // takes it along. Width and symmetry come from the two half widths.
      final moved = (r.high - r.low - (range.high - range.low)).abs() < 1e-9;
      final centre = moved ? c + (r.low - range.low) * 360 : c;
      final lo = (centre - r.low * 360).clamp(0.0, 360.0);
      final hi = (r.high * 360 - centre).clamp(0.0, 360.0);
      final width = (lo + hi).clamp(0.0, 360.0);
      return {
        'qual_hue': ((centre + 180) % 360) - 180,
        'qual_hue_width': width,
        'qual_hue_soft': (r.soft * 360).clamp(0.0, 180.0),
        'qual_hue_sym': width <= 0 ? 0 : ((hi - lo) / width * 100).clamp(-100.0, 100.0),
      };
    }

    return _barRow(
      t,
      enabled,
      name: l10n.gradeHue,
      label: _barLabel(t, l10n.gradeHue, enabled, 'qual_use_hue', rows),
      rows: rows,
      bar: GradeRangeBar(
        key: const ValueKey('grade-qual-hue-bar'),
        range: range,
        periodic: true,
        colours: [
          for (var i = 0; i <= 6; i++) hsvToRgb((i % 6) * 60.0, 0.6, 0.85)
        ],
        stops: [...gradeVectorHues, 1.0],
        enabled: enabled,
        on: _bool('qual_use_hue'),
        keyed: rows.any(_keyed),
        onStart: _touchStart,
        onLive: (r) => _liveMany({
          for (final e in rowsOf(r).entries) e.key: _floatFor(e.key, e.value)
        }),
        onCommit: (r) => _writeMany({
          for (final e in rowsOf(r).entries)
            e.key: (on) => _floatFor(e.key, e.value, on)
        }),
        onCancel: _touchCancel,
      ),
    );
  }

  /// A low and high band in per cent with its softness: saturation, luma,
  /// or one channel.
  Widget _bandBar(LumitTheme t, bool enabled, String label, String low,
      String high, String soft,
      {String? use, required List<Color> colours}) {
    final rows = [low, high, soft];
    final range = (
      low: _float(low) / 100,
      high: _float(high) / 100,
      soft: _float(soft) / 100,
    );
    Map<String, double> rowsOf(GradeRange r) => {
          low: (r.low * 100).clamp(0.0, 100.0),
          high: (r.high * 100).clamp(0.0, 100.0),
          soft: (r.soft * 100).clamp(0.0, 100.0),
        };
    return _barRow(
      t,
      enabled,
      name: label,
      label: _barLabel(t, label, enabled, use, rows),
      rows: rows,
      bar: GradeRangeBar(
        key: ValueKey<String>('grade-qual-bar-$low'),
        range: range,
        colours: colours,
        enabled: enabled,
        on: use == null || _bool(use),
        keyed: rows.any(_keyed),
        onStart: _touchStart,
        onLive: (r) => _liveMany({
          for (final e in rowsOf(r).entries) e.key: _floatFor(e.key, e.value)
        }),
        onCommit: (r) => _writeMany({
          for (final e in rowsOf(r).entries)
            e.key: (on) => _floatFor(e.key, e.value, on)
        }),
        onCancel: _touchCancel,
      ),
    );
  }

  /// A bar with its name beside it and its numbers after it, or under it
  /// when the bar would be left too short.
  Widget _barRow(LumitTheme t, bool enabled,
      {required String name,
      required Widget label,
      required List<String> rows,
      required Widget bar}) {
    // The bar already says what it ranges over: under Saturation, the
    // field for Saturation low reads Low.
    String short(String id) {
      final full = _label(id);
      if (!full.startsWith('$name ')) return full;
      final rest = full.substring(name.length + 1);
      return rest.isEmpty ? full : rest[0].toUpperCase() + rest.substring(1);
    }

    final fields = Wrap(
      spacing: 6,
      runSpacing: 4,
      children: [
        for (final id in rows)
          _namedField(t, id, enabled, width: 56, label: short(id)),
      ],
    );
    return GradeMenu(
      rows: (close) => _menuFor(close, rows: rows),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(gradeInset, 2, gradeInset, 6),
        child: LayoutBuilder(builder: (context, box) {
          final after = rows.length * 62.0;
          if (box.maxWidth >= 74 + 320 + 12 + after) {
            return Row(
              children: [
                label,
                Expanded(child: bar),
                const SizedBox(width: 12),
                SizedBox(width: after, child: fields),
              ],
            );
          }
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Row(children: [label, Expanded(child: bar)]),
              Padding(padding: const EdgeInsets.only(left: 74), child: fields),
            ],
          );
        }),
      ),
    );
  }
}
