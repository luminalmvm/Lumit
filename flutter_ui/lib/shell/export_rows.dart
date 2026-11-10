// Rows of the export dialog kept apart from it: a GIF's own, and what
// happens to the comp's caption layers.

part of 'export_dialog_frb.dart';

extension _Rows on _ExportDialogState {
  /// Dither, palette and plays, live only for a GIF.
  List<Widget> _gifRows(LumitTheme t, bool dim) {
    final why = _caps.gif ? null : l10n.tipExportFormatLimit;
    final forever = _gifPlays == 0;
    return [
      _columns(
        _row(
          t,
          l10n.exportGifDither,
          _why(
            why,
            dialogDropdown<int>(
              t,
              id: 'export-gif-dither',
              value: _gifDither,
              options: const [0, 1, 2],
              label: (d) => switch (d) {
                1 => l10n.exportGifDitherPattern,
                2 => l10n.exportGifDitherNone,
                _ => l10n.exportGifDitherDiffusion,
              },
              onChanged: _caps.gif ? (d) => _edit(() => _gifDither = d) : null,
            ),
          ),
          dimmed: dim,
        ),
        _row(
          t,
          l10n.exportGifPalette,
          _why(
            why,
            dialogDropdown<bool>(
              t,
              id: 'export-gif-palette',
              value: _gifPalettePerFrame,
              options: const [false, true],
              label: (perFrame) => perFrame
                  ? l10n.exportGifPalettePerFrame
                  : l10n.exportGifPaletteShared,
              onChanged: _caps.gif
                  ? (p) => _edit(() => _gifPalettePerFrame = p)
                  : null,
            ),
          ),
          labelColumn: exportLabelColumnPaired,
          dimmed: dim,
        ),
      ),
      _row(
        t,
        l10n.exportGifPlays,
        Row(children: [
          _why(
            why,
            _tick(
              t,
              'export-gif-forever',
              l10n.exportGifForever,
              forever,
              _caps.gif ? (on) => _edit(() => _gifPlays = on ? 0 : 1) : null,
            ),
          ),
          const SizedBox(width: 8),
          SizedBox(
            width: exportSizeWell,
            height: dialogControlHeight,
            child: forever || !_caps.gif
                ? _well(t, forever ? '—' : '$_gifPlays',
                    key: const ValueKey('export-gif-plays'),
                    tone: t.textDisabled)
                : DragValueField(
                    key: const ValueKey('export-gif-plays'),
                    value: _gifPlays,
                    min: 1,
                    max: 1000,
                    fill: t.surface0,
                    onChanged: (v) => _edit(() => _gifPlays = v.toInt()),
                  ),
          ),
        ]),
        dimmed: dim,
      ),
    ];
  }

  /// Whether the export writes caption files beside the video, which the
  /// Save dialogue never asked about.
  bool get _captionsBeside =>
      _captions == BridgeCaptionsOut.srt || _captions == BridgeCaptionsOut.vtt;

  /// Drawn into the picture, a file beside it, or inside the video file
  /// where the format can carry them.
  Widget _captionsRow(LumitTheme t, bool dim) {
    String? why(BridgeCaptionsOut c) =>
        c == BridgeCaptionsOut.embed && !_caps.subtitles
            ? l10n.tipExportFormatLimit
            : null;
    return _columns(
      _row(
        t,
        l10n.exportCaptions,
        _why(
          why(_captions),
          dialogDropdown<BridgeCaptionsOut>(
            t,
            id: 'export-captions',
            value: _captions,
            options: BridgeCaptionsOut.values,
            label: (c) => switch (c) {
              BridgeCaptionsOut.burn => l10n.exportCaptionsBurn,
              BridgeCaptionsOut.srt => l10n.exportCaptionsSrt,
              BridgeCaptionsOut.vtt => l10n.exportCaptionsVtt,
              BridgeCaptionsOut.embed => l10n.exportCaptionsEmbed,
            },
            disabledReason: why,
            onChanged: (c) => _edit(() => _captions = c),
          ),
        ),
        dimmed: dim,
      ),
      null,
    );
  }
}
