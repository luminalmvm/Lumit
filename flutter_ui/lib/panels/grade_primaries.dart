// The Grade panel's Wheels and Adjust pages: the primaries.
//
// Wheels shows Lift, Gamma, Gain and Offset three ways: as wheels, as bars,
// or as the log wheels (Shadow, Midtone, Highlight and Offset, with their
// ranges). A fourth mode, HDR, shows the zone wheels (grade_hdr.dart). A
// strip of the everyday numbers sits under the wheels. Adjust is every number
// as a slider, then the RGB mixer.

part of 'grade_panel_frb.dart';

/// How far a wheel's rim reaches, and how far its master travels, by row.
/// Gain multiplies from 1, so its master runs 0 to 2; the rest add from 0.
const Map<String, (double reach, double span)> _wheelRanges = {
  'lift': (0.5, 0.5),
  'gamma': (0.5, 0.5),
  'gain': (1.0, 1.0),
  'offset': (0.5, 0.5),
  'shadow': (0.5, 0.5),
  'midtone': (0.5, 0.5),
  'highlight': (0.5, 0.5),
};

const List<String> _wheelRows = [
  'lift',
  'gamma',
  'gain',
  'offset',
  'shadow',
  'midtone',
  'highlight',
  'low_range',
  'high_range',
];

/// The numbers under the wheels, Resolve's strip in Lumit's words.
const List<String> _stripRows = [
  'temperature',
  'tint',
  'contrast',
  'pivot',
  'saturation',
  'hue',
  'lum_mix',
];

const List<String> _adjustRows = [
  'exposure',
  'temperature',
  'tint',
  'contrast',
  'pivot',
  'saturation',
  'hue',
  'lum_mix',
  'colour_boost',
  'shadows',
  'highlights',
  'midtone_detail',
];

const List<String> _mixerRows = [
  'mixer_red',
  'mixer_green',
  'mixer_blue',
  'mixer_keep_luma',
];

const List<String> _channels = ['red', 'green', 'blue'];

mixin _GradePrimaries on _GradeModel {
  GradeWheelMode get _wheelMode => ui.grade.wheelMode.value;

  /// Whether a white balance pick has opened its undo group.
  bool _neutralTouched = false;

  /// The HDR mode's zones and Global, in place of the four wheels.
  Widget _hdrBody(LumitTheme t, bool enabled);

  /// The rows each wheel mode shows, for its tab's dot.
  static List<String> _modeRows(GradeWheelMode m) => switch (m) {
        GradeWheelMode.log => const [
            'shadow',
            'midtone',
            'highlight',
            'offset',
            'low_range',
            'high_range'
          ],
        GradeWheelMode.hdr => _hdrRows,
        _ => const ['lift', 'gamma', 'gain', 'offset'],
      };

  Widget _wheelsPage(LumitTheme t, bool enabled) {
    final ids = switch (_wheelMode) {
      GradeWheelMode.log => const ['shadow', 'midtone', 'highlight', 'offset'],
      _ => const ['lift', 'gamma', 'gain', 'offset'],
    };
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(gradeInset, 6, gradeInset, 4),
          child: SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            child: Segment(children: [
              for (final m in GradeWheelMode.values)
                SegmentOption(
                  key: ValueKey<String>('grade-wheel-mode-${m.name}'),
                  active: m == _wheelMode,
                  onTap: () => ui.grade.wheelMode.value = m,
                  child: gradeDotted(
                    t,
                    Text(
                      switch (m) {
                        GradeWheelMode.wheels => l10n.gradeWheels,
                        GradeWheelMode.bars => l10n.gradeBars,
                        GradeWheelMode.log => l10n.gradeLog,
                        GradeWheelMode.hdr => l10n.gradeHdr,
                      },
                      style: t.small.copyWith(
                          color: m == _wheelMode
                              ? t.textPrimary
                              : t.textSecondary),
                    ),
                    enabled && _anyChanged(_modeRows(m)),
                  ),
                ),
            ]),
          ),
        ),
        if (_wheelMode == GradeWheelMode.hdr)
          _hdrBody(t, enabled)
        else
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: gradeInset),
            child: LayoutBuilder(builder: (context, box) {
              final cells = [
                for (final id in ids)
                  _wheelMode == GradeWheelMode.bars
                      ? _barCell(t, id, enabled)
                      : (double size) => _wheelCell(t, id, size, enabled),
              ];
              // Four across while each has room for its numbers, two by two,
              // then one a row.
              const gap = 8.0;
              final least = _cellMin(t);
              final across = [4, 2, 1].firstWhere(
                  (n) => (box.maxWidth - gap * (n - 1)) / n >= least,
                  orElse: () => 1);
              final cell = (box.maxWidth - gap * (across - 1)) / across;
              final size = (cell - 16).clamp(56.0, _wheelMax).toDouble();
              return Wrap(
                spacing: gap,
                runSpacing: 10,
                children: [
                  for (final c in cells)
                    SizedBox(width: cell, child: Center(child: c(size))),
                ],
              );
            }),
          ),
        // HDR has no strip: Adjust is a page of its own beside it.
        if (_wheelMode != GradeWheelMode.hdr) ...[
          const SizedBox(height: 6),
          _strip(t, enabled),
        ],
      ],
    );
  }

  /// How big a wheel may be: no taller than the pane can show with its
  /// numbers and the strip under it.
  double get _wheelMax {
    final room = _room;
    if (room == null) return 96;
    return (room.height - 130).clamp(56.0, 96.0).toDouble();
  }

  /// A wheel's Y R G B row: four bare numbers, each with its tick.
  static double _numbersWidth(LumitTheme t) =>
      4 * (monoSlotWidth(t.mono.copyWith(fontSize: barValueTextSize), 5) + 4) +
      3 * 2;

  /// The narrowest a wheel's cell goes before the wheels wrap.
  static double _cellMin(LumitTheme t) => math.max(112, _numbersWidth(t));

  /// The wheels page's width beside Adjust: four wheels in one row, each
  /// cell as wide as its wheel or its numbers, and the strip in one row.
  double _wheelsWidth(LumitTheme t) {
    final log = _wheelMode == GradeWheelMode.log;
    final strip = _stripRows.length + (log ? 2 : 0);
    return math.max(4 * math.max(_wheelMax + 16, _cellMin(t)) + 3 * 8,
            strip * (_stripField + 6) - 6) +
        2 * gradeInset;
  }

  /// How wide each of the strip's fields is.
  static const double _stripField = 62;

  static const double _columnMin = 340, _columnMax = 420, _columnGap = 8;

  static int _columnsIn(double width) =>
      math.max(1, ((width + _columnGap) / (_columnMin + _columnGap)).floor());

  /// [rows] down as many columns as the width holds, none wider than a row
  /// needs, so a wide pane reads as a band rather than lines across it.
  /// [last] takes a column of its own once there are three, and goes under
  /// the rest before that.
  Widget _flow(List<Widget> rows, {Widget? last}) =>
      LayoutBuilder(builder: (context, box) {
        final n = _columnsIn(box.maxWidth);
        final width =
            math.min(_columnMax, (box.maxWidth - _columnGap * (n - 1)) / n);
        final own = last != null && n >= 3;
        final per = math.max(1, (rows.length / (own ? n - 1 : n)).ceil());
        Widget column(List<Widget> c) => SizedBox(
              width: width,
              child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch, children: c),
            );
        final columns = [
          for (var i = 0; i < rows.length; i += per)
            column(rows.sublist(i, math.min(i + per, rows.length))),
          if (own) column([last]),
        ];
        final band = Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (var i = 0; i < columns.length; i++) ...[
              if (i > 0) const SizedBox(width: _columnGap),
              columns[i],
            ],
          ],
        );
        if (last == null || own) return band;
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [band, SizedBox(width: width, child: last)],
        );
      });

  /// One wheel with its name, its master number and its three channels.
  Widget _wheelCell(LumitTheme t, String id, double size, bool enabled) {
    final rgb = _colour(id);
    final neutral = _defaultColour(id)[0];
    final (reach, span) = _wheelRanges[id]!;
    final keyed = _keyed(id);
    return GradeMenu(
      rows: (close) => _menuFor(close, rows: [id], param: id),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          GradeWheel(
            key: ValueKey<String>('grade-wheel-$id'),
            rgb: rgb,
            neutral: neutral,
            reach: reach,
            masterSpan: span,
            size: size,
            enabled: enabled,
            keyed: keyed,
            onStart: _touchStart,
            onLive: (c) => _live(id, _colourFor(id, c)),
            onCommit: (c) => _write(id, (on) => _colourFor(id, c, on)),
            onCancel: _touchCancel,
          ),
          const SizedBox(height: 4),
          _wheelNumbers(t, id, rgb, enabled, keyed),
        ],
      ),
    );
  }

  /// One wheel's colour as four bars: the master, then red, green and blue.
  Widget Function(double) _barCell(LumitTheme t, String id, bool enabled) =>
      (size) {
        final rgb = _colour(id);
        final keyed = _keyed(id);
        final (lo, hi) = _colourRange(id);
        final neutral = _defaultColour(id)[0];
        final span = _wheelRanges[id]!.$2;
        return GradeMenu(
          rows: (close) => _menuFor(close, rows: [id], param: id),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              GradeBars(
                key: ValueKey<String>('grade-bars-$id'),
                rgb: rgb,
                neutral: neutral,
                span: span,
                min: lo,
                max: hi,
                height: size,
                enabled: enabled,
                keyed: keyed,
                colours: [
                  t.textSecondary,
                  for (final c in _channels)
                    curveChannelColour(c) ?? t.textSecondary,
                ],
                onStart: _touchStart,
                onLive: (c) => _live(id, _colourFor(id, c)),
                onCommit: (c) => _write(id, (on) => _colourFor(id, c, on)),
                onCancel: _touchCancel,
              ),
              const SizedBox(height: 4),
              _wheelNumbers(t, id, rgb, enabled, keyed),
            ],
          ),
        );
      };

  Widget _wheelNumbers(
      LumitTheme t, String id, List<double> rgb, bool enabled, bool keyed) {
    final changed = _changed(id);
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        // Each number gives way in a narrow cell rather than spill out.
        Text(
          _label(id),
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: t.small.copyWith(
            color: keyed
                ? t.animated
                : !enabled
                    ? t.textDisabled
                    : changed
                        ? t.textSecondary
                        : t.textMuted,
          ),
        ),
        // Y R G B, as a colourist reads them: the master, then each channel.
        Padding(
          padding: const EdgeInsets.only(top: 2),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              for (final c in const [null, 0, 1, 2]) ...[
                if (c != null) const SizedBox(width: 2),
                Flexible(child: _wheelNumber(id, rgb, c, enabled, keyed)),
              ],
            ],
          ),
        ),
      ],
    );
  }

  /// One of a wheel's numbers: a channel, or with [channel] null the master,
  /// which moves the three together as the bar does. Typed or dragged, it
  /// writes the whole colour, as the wheel does.
  Widget _wheelNumber(
      String id, List<double> rgb, int? channel, bool enabled, bool keyed) {
    final (lo, hi) = _colourRange(id);
    final neutral = _defaultColour(id)[0];
    final master = wheelMaster(rgb);
    List<double> next(num v) => channel == null
        ? [for (final c in rgb) c + v.toDouble() - master]
        : [for (var i = 0; i < 3; i++) i == channel ? v.toDouble() : rgb[i]];
    void commit(num v) => _write(id, (on) => _colourFor(id, next(v), on));
    final field = DragValueField(
      key: ValueKey<String>('grade-wheel-$id-${channel ?? 'master'}'),
      bare: true,
      value: channel == null ? master : rgb[channel],
      min: lo,
      max: hi,
      speed: (_wheelRanges[id]?.$2 ?? 1) / 100,
      decimals: 2,
      signed: neutral == 0,
      keyed: keyed,
      resetTo: neutral,
      onChangeStart: _touchStart,
      onChangeLive: (v) => _live(id, _colourFor(id, next(v))),
      onChangeEnd: commit,
      onDragCancel: _touchCancel,
      onChanged: (v) {
        _touchStart();
        commit(v);
      },
    );
    return IgnorePointer(
      ignoring: !enabled,
      child: Opacity(
        opacity: enabled ? 1 : 0.5,
        // A channel's own colour under its number says which it is; the
        // master's is the text's.
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 1, vertical: 1),
              child: field,
            ),
            Container(
              width: 12,
              height: 1.5,
              color: channel == null
                  ? ThemeScope.of(context).theme.textSecondary
                  : curveChannelColour(_channels[channel]),
            ),
          ],
        ),
      ),
    );
  }

  /// The everyday numbers as named fields in a row that wraps: a name above
  /// each number, which scrubs, types and double-clicks back like any field.
  Widget _strip(LumitTheme t, bool enabled) => Padding(
        padding: const EdgeInsets.symmetric(horizontal: gradeInset),
        child: Wrap(
          spacing: 6,
          runSpacing: 6,
          children: [
            if (_wheelMode == GradeWheelMode.log)
              for (final id in const ['low_range', 'high_range'])
                _namedField(t, id, enabled, decimals: 3),
            for (final id in _stripRows) _namedField(t, id, enabled),
          ],
        ),
      );

  Widget _namedField(LumitTheme t, String id, bool enabled,
      {double width = _stripField, String? label, int? decimals}) {
    final keyed = _keyed(id);
    final changed = _changed(id);
    final live = enabled && !_locked(id);
    final (lo, hi) = _hardRange(id);
    final (min, max) = _range(id);
    final neutral = _defaultFloat(id);
    return GradeMenu(
      rows: (close) => _menuFor(close, rows: [id], param: id),
      child: SizedBox(
        width: width,
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              label ?? _label(id),
              style: t.small.copyWith(
                  color: keyed
                      ? t.animated
                      : !live
                          ? t.textDisabled
                          : changed
                              ? t.textSecondary
                              : t.textMuted),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
            IgnorePointer(
              ignoring: !live,
              child: Opacity(
                opacity: live ? 1 : 0.5,
                child: DragValueField(
                  key: ValueKey<String>('grade-field-$id'),
                  value: _float(id),
                  min: lo,
                  max: hi,
                  speed: (max - min) / 200,
                  decimals: decimals ?? _decimals(id),
                  signed: neutral == 0 && min < 0,
                  keyed: keyed,
                  resetTo: neutral,
                  onChangeStart: _touchStart,
                  onChangeLive: (v) => _liveFloat(id, v.toDouble()),
                  onChangeEnd: (v) => _writeFloat(id, v.toDouble()),
                  onDragCancel: _touchCancel,
                  onChanged: (v) {
                    _touchStart();
                    _writeFloat(id, v.toDouble());
                  },
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// How many places a row's number shows: fine rows more.
  int _decimals(String id) {
    final (min, max) = _range(id);
    final span = max - min;
    return span <= 2
        ? 3
        : span <= 20
            ? 2
            : 1;
  }

  /// One number as a slider row, its menu and its reset.
  Widget _slider(LumitTheme t, String id, bool enabled,
      {int? decimals, Widget? trailing, List<Color>? tint, String? label}) {
    final (min, max) = _range(id);
    final (hardMin, hardMax) = _hardRange(id);
    final neutral = _defaultFloat(id);
    // The track sits its neutral value in the middle, however far the
    // effect's own range runs past it one way: an untouched row reads as
    // untouched. The number keeps the whole range.
    final centred = neutral > min && neutral < max;
    final span = math.min(neutral - min, max - neutral);
    // The callbacks run later, outside any [_within]: they keep its aim.
    final aside = _aside;
    void at(VoidCallback run) => _within(aside, run);
    return GradeSlider(
      key: ValueKey<String>('grade-row-$id'),
      label: label ?? _label(id),
      value: _float(id),
      min: centred ? neutral - span : min,
      max: centred ? neutral + span : max,
      fieldMin: hardMin,
      fieldMax: hardMax,
      neutral: neutral,
      decimals: decimals ?? _decimals(id),
      signed: neutral == 0 && min < 0,
      enabled: enabled && !_locked(id),
      keyed: _keyed(id),
      tint: tint,
      onStart: _touchStart,
      onLive: (v) => at(() => _liveFloat(id, v)),
      onCommit: (v) => at(() => _writeFloat(id, v)),
      onCancel: _touchCancel,
      menu: (close) =>
          _within(aside, () => _menuFor(close, rows: [id], param: id)),
      trailing: trailing ?? const SizedBox(width: 18),
    );
  }

  /// A switch row: its name and a tick, one undo step a press.
  Widget _toggle(LumitTheme t, String id, bool enabled, {String? label}) {
    final on = _bool(id);
    return GradeMenu(
      rows: (close) => _menuFor(close, rows: [id]),
      child: Padding(
        padding:
            const EdgeInsets.symmetric(horizontal: gradeInset, vertical: 3),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            HouseCheckbox(
              key: ValueKey<String>('grade-tick-$id'),
              value: on,
              onChanged: enabled
                  ? (v) => _set({id: BridgeEffectValue.bool(v)})
                  : null,
            ),
            const SizedBox(width: 6),
            Flexible(
              child: Text(label ?? _label(id),
                  style: t.small.copyWith(
                      color: !enabled
                          ? t.textDisabled
                          : _changed(id)
                              ? t.textSecondary
                              : t.textMuted),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis),
            ),
          ],
        ),
      ),
    );
  }

  Widget _adjustPage(LumitTheme t, bool enabled) {
    // Blue to orange and green to magenta, the way the rows push.
    final warm = [hsvToRgb(210, 0.45, 0.8), hsvToRgb(32, 0.55, 0.85)];
    final tint = [hsvToRgb(120, 0.45, 0.75), hsvToRgb(300, 0.45, 0.8)];
    return Padding(
      padding: const EdgeInsets.only(top: 4),
      child: _flow(
        [
          for (final id in _adjustRows)
            switch (id) {
              'temperature' => _slider(t, id, enabled,
                  tint: warm, trailing: _neutralDropper(enabled)),
              'tint' => _slider(t, id, enabled, tint: tint),
              'exposure' => _slider(t, id, enabled, label: l10n.fxExposure),
              _ => _slider(t, id, enabled),
            },
        ],
        last: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            GradeMenu(
              rows: (close) => _menuFor(close, rows: _mixerRows),
              child: GradeHeading(
                label: l10n.gradeMixer,
                changed: enabled && _anyChanged(_mixerRows),
                onReset: () => _reset(_mixerRows),
              ),
            ),
            _mixer(t, enabled),
            _toggle(t, 'mixer_keep_luma', enabled),
          ],
        ),
      ),
    );
  }

  /// The mixer as three rows of three: what each output channel takes from
  /// red, green and blue.
  Widget _mixer(LumitTheme t, bool enabled) => Padding(
        padding: const EdgeInsets.symmetric(horizontal: gradeInset),
        child: Column(
          children: [
            for (final id in const ['mixer_red', 'mixer_green', 'mixer_blue'])
              GradeMenu(
                rows: (close) => _menuFor(close, rows: [id], param: id),
                child: Padding(
                  padding: const EdgeInsets.symmetric(vertical: 1),
                  child: Row(
                    children: [
                      SizedBox(
                        width: 74,
                        child: Text(_label(id),
                            style: t.small.copyWith(
                                color: _keyed(id)
                                    ? t.animated
                                    : !enabled
                                        ? t.textDisabled
                                        : _changed(id)
                                            ? t.textSecondary
                                            : t.textMuted),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis),
                      ),
                      for (var i = 0; i < 3; i++)
                        Expanded(child: _mixerField(id, i, enabled)),
                    ],
                  ),
                ),
              ),
          ],
        ),
      );

  Widget _mixerField(String id, int channel, bool enabled,
      {bool signed = true}) {
    final rgb = _colour(id);
    final (lo, hi) = _colourRange(id);
    List<double> next(num v) =>
        [for (var i = 0; i < 3; i++) i == channel ? v.toDouble() : rgb[i]];
    void commit(num v) => _write(id, (on) => _colourFor(id, next(v), on));
    return IgnorePointer(
      ignoring: !enabled,
      child: Opacity(
        opacity: enabled ? 1 : 0.5,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            DragValueField(
              key: ValueKey<String>('grade-mixer-$id-$channel'),
              bare: true,
              value: rgb[channel],
              min: lo,
              max: hi,
              speed: 0.01,
              decimals: 2,
              signed: signed,
              keyed: _keyed(id),
              resetTo: _defaultColour(id)[channel],
              onChangeStart: _touchStart,
              onChangeLive: (v) => _live(id, _colourFor(id, next(v))),
              onChangeEnd: commit,
              onDragCancel: _touchCancel,
              onChanged: (v) {
                _touchStart();
                commit(v);
              },
            ),
            Container(
              width: 12,
              height: 1.5,
              color: curveChannelColour(_channels[channel]),
            ),
          ],
        ),
      ),
    );
  }

  // --- The neutral picker ------------------------------------------------------

  /// The number a pick reads the picture against is the document's, never a
  /// preview's: the window of pixels the dropper reads is cut from the frame
  /// the document draws, so the picture it sees has the rows as they stand.
  BridgeWhiteBalance _neutralFor(DropperSample s) => whiteBalanceFor(
        r: s.r,
        g: s.g,
        b: s.b,
        temperature: _float('temperature'),
        tint: _float('tint'),
      );

  void _neutralLive(DropperSample s) {
    final solved = _neutralFor(s);
    if (!_neutralTouched) {
      _touchStart();
      _neutralTouched = true;
    }
    _liveMany({
      'temperature': _floatFor('temperature', solved.temperature),
      'tint': _floatFor('tint', solved.tint),
    });
  }

  /// Both rows as one write, so one undo step takes the whole pick back.
  void _neutralPick(DropperSample s) {
    final solved = _neutralFor(s);
    if (!_neutralTouched) _touchStart();
    _neutralTouched = false;
    _writeMany({
      'temperature': (on) => _floatFor('temperature', solved.temperature, on),
      'tint': (on) => _floatFor('tint', solved.tint, on),
    });
  }

  /// Abandoned: the preview goes, and the node too if the pick brought it.
  void _neutralRevert() {
    if (!_neutralTouched) return;
    _neutralTouched = false;
    _touchCancel();
  }

  Widget _neutralDropper(bool enabled) {
    if (!enabled || _locked('temperature') || _locked('tint')) {
      return const SizedBox(width: 18);
    }
    return DropperButton(
      id: 'grade-neutral',
      tip: l10n.gradePickNeutral,
      arm: (ui) => ui.armDropper(DropperArm(
        id: 'grade-neutral',
        reads: DropperReads.colour,
        label: l10n.gradePickNeutral,
        onPreview: _neutralLive,
        onPick: _neutralPick,
        onRevert: _neutralRevert,
      )),
    );
  }
}
