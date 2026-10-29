// The Flare window's right side: the selected element's name, its controls
// in twirls by group, and its colour.

part of 'flare_window.dart';

/// A scene-linear channel as the screen shows it.
double _srgb(double v) {
  v = v.clamp(0.0, 1.0);
  return v <= 0.0031308 ? 12.92 * v : 1.055 * math.pow(v, 1 / 2.4) - 0.055;
}

extension _Inspector on _FlareWindowState {
  Widget _inspector(LumitTheme t) {
    final e = _element;
    if (e == null) return const SizedBox.shrink();
    final kind = e['kind'] as String;
    final controls =
        _controls.putIfAbsent(kind, () => flareControls(kind: kind));
    final groups = <String>[];
    for (final c in controls) {
      if (!groups.contains(c.group)) groups.add(c.group);
    }
    Widget mark(String key, String tip, String icon, VoidCallback onTap) =>
        LumitTooltip(
          message: tip,
          child: HoverMark(
            builder: (context, lit) => GestureDetector(
              key: ValueKey(key),
              behavior: HitTestBehavior.opaque,
              onTap: onTap,
              child: SizedBox(
                width: 22,
                height: 22,
                child: Center(
                  child: glyph.LumitIcon(icon,
                      size: 11,
                      colour: Color.lerp(t.textMuted, t.textPrimary, lit)),
                ),
              ),
            ),
          ),
        );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 8, 6, 2),
          child: Row(
            children: [
              Expanded(
                child: Text(t.kickerCase(_kindLabel(kind)),
                    style: t.kicker, overflow: TextOverflow.ellipsis),
              ),
              mark('flare-duplicate', l10n.flareDuplicate, LumitIcons.group,
                  () => _duplicateElement(_selected)),
              mark('flare-delete', l10n.flareDelete, LumitIcons.delete,
                  () => _deleteElement(_selected)),
            ],
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 2, 12, 4),
          child: HouseTextField(
            key: const ValueKey('flare-name'),
            controller: _name,
            width: double.infinity,
            submitOnLostFocus: true,
            onSubmitted: _renameSelected,
          ),
        ),
        Expanded(
          child: ListView(
            padding: const EdgeInsets.fromLTRB(12, 4, 12, 10),
            children: [
              for (final g in groups)
                _twirl(t, g, [
                  for (final c in controls)
                    if (c.group == g) _controlRow(t, e, c),
                  if (g == 'Look') _colourRows(t, e),
                ]),
            ],
          ),
        ),
      ],
    );
  }

  Widget _twirl(LumitTheme t, String group, List<Widget> rows) {
    final open = !_closed.contains(group);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        GestureDetector(
          key: ValueKey('flare-group-$group'),
          behavior: HitTestBehavior.opaque,
          onTap: () => _redraw(() {
            open ? _closed.add(group) : _closed.remove(group);
          }),
          child: SizedBox(
            height: 26,
            child: Row(
              children: [
                glyph.LumitIcon(open ? LumitIcons.collapse : LumitIcons.expand,
                    size: 8, colour: t.textMuted),
                const SizedBox(width: 6),
                Text(engineLabel(group), style: t.bodyStrong),
              ],
            ),
          ),
        ),
        if (open) ...rows,
        const SizedBox(height: 4),
      ],
    );
  }

  Widget _row(LumitTheme t, String label, Widget control) => SizedBox(
        height: 28,
        child: Row(
          children: [
            SizedBox(
              width: _labelWidth,
              child: Padding(
                padding: const EdgeInsets.only(right: 8),
                child: Text(engineLabel(label),
                    style: t.body, overflow: TextOverflow.ellipsis),
              ),
            ),
            Expanded(child: control),
          ],
        ),
      );

  double _valueOf(Json e, BridgeFlareControl c) =>
      ((e['values'] as Json)[c.key] as num?)?.toDouble() ?? c.default_;

  void _set(Json e, BridgeFlareControl c, num v) => _edit(
      () => (e['values'] as Json)[c.key] = v.toDouble(),
      coalesce: c.key);

  Widget _controlRow(LumitTheme t, Json e, BridgeFlareControl c) {
    final v = _valueOf(e, c);
    final counter = c.edit == BridgeFlareEdit.counter;
    final span = c.max - c.min;
    final decimals = counter
        ? 0
        : span <= 10
            ? 2
            : span <= 100
                ? 1
                : 0;
    Widget field({bool signed = false, double? min, double? max}) => SizedBox(
          width: 58,
          child: DragValueField(
            key: ValueKey('flare-field-${c.key}'),
            value: v,
            min: min ?? c.hardMin,
            max: max ?? c.hardMax,
            decimals: decimals,
            suffix: c.suffix.isEmpty ? null : c.suffix,
            signed: signed,
            speed: counter ? 0.1 : span / 300,
            onChanged: (n) => _set(e, c, n),
          ),
        );
    switch (c.edit) {
      case BridgeFlareEdit.slider:
      case BridgeFlareEdit.counter:
        return _row(
          t,
          c.label,
          LayoutBuilder(
            builder: (context, box) => Row(
              children: [
                HouseSlider(
                  key: ValueKey('flare-slider-${c.key}'),
                  value: v.clamp(c.min, c.max),
                  min: c.min,
                  max: c.max,
                  step: counter ? 1 : null,
                  showValue: false,
                  width: math.max(40, box.maxWidth - 64),
                  onChanged: (n) => _set(e, c, counter ? n.round() : n),
                ),
                const SizedBox(width: 6),
                field(),
              ],
            ),
          ),
        );
      case BridgeFlareEdit.toggle:
        return _row(
          t,
          c.label,
          Align(
            alignment: Alignment.centerLeft,
            child: HouseCheckbox(
              key: ValueKey('flare-toggle-${c.key}'),
              value: v >= 0.5,
              onChanged: (on) => _set(e, c, on ? 1 : 0),
            ),
          ),
        );
      case BridgeFlareEdit.angle:
        return _row(
          t,
          c.label,
          Row(
            children: [
              AngleDial(
                key: ValueKey('flare-dial-${c.key}'),
                size: 22,
                degrees: v,
                onChanged: (n) => _set(e, c, n),
              ),
              const SizedBox(width: 8),
              field(signed: true),
            ],
          ),
        );
      case BridgeFlareEdit.choice:
        final index =
            v.round().clamp(0, math.max(0, c.options.length - 1)).toInt();
        if (c.options.length <= 3) {
          return _row(
            t,
            c.label,
            Align(
              alignment: Alignment.centerLeft,
              child: _segmented(
                t,
                keyPrefix: 'flare-choice-${c.key}',
                items: [
                  for (final (i, o) in c.options.indexed)
                    ('$i', engineLabel(o)),
                ],
                current: '$index',
                onPick: (k) => _set(e, c, int.parse(k)),
              ),
            ),
          );
        }
        return _row(
          t,
          c.label,
          Align(
            alignment: Alignment.centerLeft,
            child: BareDropdown<int>(
              key: ValueKey('flare-choice-${c.key}'),
              value: index,
              options: [for (var i = 0; i < c.options.length; i++) i],
              label: (i) => engineLabel(c.options[i]),
              onChanged: (i) => _set(e, c, i),
            ),
          ),
        );
    }
  }

  /// The colour mode, then its swatches: one for Solid, two for Gradient,
  /// a list with a plus for Mixed.
  Widget _colourRows(LumitTheme t, Json e) {
    final colour = e['colour'] as Json;
    final mode = (colour['mode'] as String?) ?? 'light';
    final colours = [
      for (final c in (colour['colours'] as List? ?? const []))
        [for (final n in c as List) (n as num).toDouble()],
    ];
    final count = switch (mode) {
      'solid' => 1,
      'gradient' => 2,
      'mixed' => math.max(1, colours.length),
      _ => 0,
    };
    while (colours.length < count) {
      colours.add([1, 1, 1]);
    }
    void write(List<List<double>> list, {String? coalesce}) => _edit(
        () => colour['colours'] = list,
        coalesce: coalesce);
    Widget swatch(int i) {
      final c = colours[i];
      final button = ColourSwatchButton(
        key: ValueKey('flare-swatch-$i'),
        colour: Color.from(
            alpha: 1, red: _srgb(c[0]), green: _srgb(c[1]), blue: _srgb(c[2])),
        initial: PickedColour(c[0], c[1], c[2]),
        scale: ColourScale.unit,
        max: 4,
        onPreview: (p) => write(
            [...colours]..[i] = [p.r, p.g, p.b],
            coalesce: 'colour$i'),
        onPicked: (p) => write(
            [...colours]..[i] = [p.r, p.g, p.b],
            coalesce: 'colour$i'),
      );
      if (mode != 'mixed' || colours.length <= 1) return button;
      return HouseContextMenu(
        itemBuilder: (close) => [
          MenuRow(
            onPressed: () {
              close();
              write([...colours]..removeAt(i));
            },
            child: Text(l10n.flareDelete),
          ),
        ],
        child: button,
      );
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // The five modes take the whole width on a line of their own: in
        // the control column they wrap.
        SizedBox(
          height: 24,
          child: Align(
            alignment: Alignment.centerLeft,
            child: Text(engineLabel('Colour'), style: t.body),
          ),
        ),
        Wrap(
          spacing: 2,
          runSpacing: 2,
          children: [
            for (final m in _modes)
              HouseButton(
                key: ValueKey('flare-colour-${m.key}'),
                small: true,
                frameless: m.key != mode,
                active: m.key == mode,
                padding: const EdgeInsets.symmetric(horizontal: 7),
                onPressed: () => _edit(() => colour['mode'] = m.key),
                child: Text(engineLabel(m.label),
                    style: t.small.copyWith(
                        color: m.key == mode ? t.textPrimary : null)),
              ),
          ],
        ),
        if (count > 0)
          Padding(
            padding: const EdgeInsets.only(top: 6, bottom: 4),
            child: Wrap(
              spacing: 4,
              runSpacing: 4,
              crossAxisAlignment: WrapCrossAlignment.center,
              children: [
                for (var i = 0; i < count; i++) swatch(i),
                if (mode == 'mixed')
                  HoverMark(
                    builder: (context, lit) => GestureDetector(
                      key: const ValueKey('flare-colour-add'),
                      behavior: HitTestBehavior.opaque,
                      onTap: () => write([...colours, [1, 1, 1]]),
                      child: Container(
                        width: 28,
                        height: 18,
                        decoration: BoxDecoration(
                          borderRadius:
                              BorderRadius.circular(t.tokens.controlRadius),
                          border: Border.all(color: swatchEdge(t, lit)),
                        ),
                        child: Center(
                          child: glyph.LumitIcon(LumitIcons.addEffect,
                              size: 8,
                              colour: Color.lerp(
                                  t.textMuted, t.textPrimary, lit)),
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}
