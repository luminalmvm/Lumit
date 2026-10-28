// The Flare window's presets strip under the preview: Built in and Yours
// tabs, the group chips, and a row of thumbnails drawn by the preview call.

part of 'flare_window.dart';

/// Thumbnails by design JSON, kept for the session: a strip that scrolls
/// back never draws twice.
final Map<String, Future<ui.Image?>> _thumbs = {};

/// Thumbnails are drawn at twice the widest tile, on a 1080p stand-in: one
/// picture serves every comp and every window size.
const int _thumbPxWidth = 320;
const int _thumbPxHeight = 180;

Future<ui.Image?> _thumb(String json) => _thumbs.putIfAbsent(json, () async {
      final bytes = await flarePreview(
        designJson: json,
        width: _thumbPxWidth,
        height: _thumbPxHeight,
        lightX: _thumbPxWidth * 0.36,
        lightY: _thumbPxHeight * 0.4,
        pxScale: _thumbPxWidth / _fallbackComp.width,
      );
      return bytes.isEmpty ? null : _decode(bytes, _thumbPxWidth, _thumbPxHeight);
    });

extension _Presets on _FlareWindowState {
  Widget _presetsStrip(LumitTheme t) {
    final yoursAny = _presets.any((p) => !p.builtIn);
    final yours = yoursAny && (_yours ?? true);
    final groups = <String>[];
    for (final p in _presets) {
      if (p.builtIn && !groups.contains(p.group)) groups.add(p.group);
    }
    if (!groups.contains(_group)) _group = '';
    final shown = [
      for (final p in _presets)
        if (p.builtIn != yours && (yours || _group.isEmpty || p.group == _group))
          p,
    ];
    Widget tab(String key, String label, bool on, VoidCallback onTap) =>
        HouseButton(
          key: ValueKey(key),
          small: true,
          frameless: !on,
          active: on,
          padding: const EdgeInsets.symmetric(horizontal: 8),
          onPressed: onTap,
          child: Text(label,
              style: t.small.copyWith(color: on ? t.textPrimary : null)),
        );
    final tabs = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (yoursAny) ...[
          tab('flare-tab-yours', l10n.flareYours, yours,
              () => _redraw(() => _yours = true)),
          const SizedBox(width: 3),
          tab('flare-tab-built-in', l10n.flareBuiltIn, !yours,
              () => _redraw(() => _yours = false)),
        ] else
          Text(t.kickerCase(l10n.flareBuiltIn), style: t.kicker),
      ],
    );
    final chips = Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final g in ['', ...groups]) ...[
          if (g.isNotEmpty) const SizedBox(width: 3),
          tab(
              'flare-group-${g.isEmpty ? 'all' : g}',
              g.isEmpty ? l10n.flareAllGroups : engineLabel(g),
              g == _group,
              () => _redraw(() => _group = g)),
        ],
      ],
    );
    const gap = 6.0;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // The tabs at the left and the group chips at the right; a window
        // too narrow for both puts the chips on a line of their own.
        Wrap(
          alignment: WrapAlignment.spaceBetween,
          runSpacing: 4,
          children: [
            SizedBox(height: 22, child: tabs),
            if (!yours && groups.length > 1) SizedBox(height: 22, child: chips),
          ],
        ),
        const SizedBox(height: gap),
        // A grid that scrolls down: as many tiles across as the width takes,
        // each grown to fill its share.
        Expanded(
          child: LayoutBuilder(builder: (context, c) {
            final across = math.max(
                1, ((c.maxWidth + gap) / (_tileMaxWidth + gap)).ceil());
            return SingleChildScrollView(
              key: const ValueKey('flare-presets'),
              child: Column(
                children: [
                  for (var row = 0; row < shown.length; row += across)
                    Padding(
                      padding: EdgeInsets.only(top: row == 0 ? 0 : gap),
                      child: Row(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          for (var i = row; i < row + across; i++) ...[
                            if (i > row) const SizedBox(width: gap),
                            Expanded(
                              child: i < shown.length
                                  ? _presetTile(t, shown[i])
                                  : const SizedBox.shrink(),
                            ),
                          ],
                        ],
                      ),
                    ),
                ],
              ),
            );
          }),
        ),
      ],
    );
  }

  Widget _presetTile(LumitTheme t, BridgeFlarePreset p) {
    final current = p.name == _designName;
    final tile = HouseButton(
      key: ValueKey('flare-preset-${p.name}'),
      small: true,
      frameless: true,
      active: current,
      padding: const EdgeInsets.all(2),
      onPressed: () => _usePreset(p),
      child: SizedBox(
        width: double.infinity,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            ClipRRect(
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
              child: AspectRatio(
                aspectRatio: 16 / 9,
                child: FutureBuilder<ui.Image?>(
                  future: _thumb(p.json),
                  builder: (context, snap) => snap.data == null
                      ? ColoredBox(color: t.surface0)
                      : RawImage(image: snap.data, fit: BoxFit.fill),
                ),
              ),
            ),
            Padding(
              padding: const EdgeInsets.only(top: 3),
              child: Text(
                p.name,
                textAlign: TextAlign.center,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: t.caption
                    .copyWith(color: current ? t.accent : t.textSecondary),
              ),
            ),
          ],
        ),
      ),
    );
    if (p.builtIn) return tile;
    return HouseContextMenu(
      itemBuilder: (close) => [
        MenuRow(
          key: ValueKey('flare-preset-rename-${p.name}'),
          onPressed: () {
            close();
            unawaited(_renamePreset(p));
          },
          child: Text(l10n.flareRename),
        ),
        MenuRow(
          key: ValueKey('flare-preset-delete-${p.name}'),
          onPressed: () {
            close();
            unawaited(_deletePreset(p));
          },
          child: Text(l10n.flareDelete),
        ),
      ],
      child: tile,
    );
  }

  /// A click replaces the elements with the preset's, and takes its name.
  void _usePreset(BridgeFlarePreset p) {
    final design = jsonDecode(p.json) as Json;
    _edit(() {
      _design['elements'] = design['elements'] ?? <Json>[];
      _design['name'] = p.name;
      _selected = 0;
      _solo = null;
      _syncName();
    });
  }

  Future<void> _renamePreset(BridgeFlarePreset p) async {
    final name = await askThemeName(context,
        title: l10n.flareNameTitle, suggested: p.name);
    if (name == null || name == p.name || !mounted) return;
    try {
      final saved = await renameFlare(name: p.name, newName: name);
      if (!mounted) return;
      _redraw(() {
        if (_designName == p.name) _design['name'] = saved;
        _presets = flarePresets();
      });
    } catch (_) {
      if (mounted) _redraw(() => _said = l10n.flareNotSaved);
    }
  }

  Future<void> _deletePreset(BridgeFlarePreset p) async {
    try {
      await deleteFlare(name: p.name);
    } catch (_) {}
    if (mounted) _redraw(() => _presets = flarePresets());
  }
}
