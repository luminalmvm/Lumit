// Also write: other files made from the same frames as the chosen one, each
// a video or a still sequence with a suffix on the chosen file's name.

part of 'export_dialog_frb.dart';

/// One file written beside the chosen one: its format key and its suffix.
class _Also {
  String codec;
  final TextEditingController suffix;
  _Also(this.codec, String suffix)
      : suffix = TextEditingController(text: suffix);
}

/// The most files the engine writes beside the chosen one.
const int _mostAlso = 3;

extension _AlsoRows on _ExportDialogState {
  /// The extra files as the spec carries them. An export sent to another
  /// computer leaves them out on the way.
  List<BridgeExportAlso> get _alsoSpec => [
        for (final a in _also)
          BridgeExportAlso(codec: a.codec, suffix: a.suffix.text.trim()),
      ];

  /// The formats the engine would write beside the chosen one as it stands,
  /// asked as each field changes rather than on a rebuild.
  void _recomputeAlso() {
    String checkWith(List<BridgeExportAlso> also) {
      _alsoProbe = also;
      try {
        return widget.comp.exportSpecCheck(spec: _spec);
      } finally {
        _alsoProbe = null;
      }
    }

    _alsoOffered = _on.isNotEmpty || checkWith(const []).isNotEmpty
        ? const []
        : [
            for (final f in _formats)
              if (checkWith([BridgeExportAlso(codec: f.key, suffix: '_')])
                  .isEmpty)
                f.key,
          ];
  }

  /// A suffix no other file of the same kind has.
  String _freeSuffix(String codec) {
    final ext = _extensionOf(codec);
    final taken = {
      if (_format.extension == ext) '',
      for (final a in _also)
        if (_extensionOf(a.codec) == ext) a.suffix.text.trim(),
    };
    var n = 1;
    var suffix = '';
    while (taken.contains(suffix)) {
      suffix = '_${++n}';
    }
    return suffix;
  }

  String _extensionOf(String codec) =>
      _formats.where((f) => f.key == codec).firstOrNull?.extension ?? codec;

  /// A row for each extra file, then one to add another while there is room.
  List<Widget> _alsoRows(LumitTheme t) {
    final remote = _on.isNotEmpty;
    final reason = remote ? l10n.tipExportAlsoRemote : null;
    return [
      for (final (i, a) in _also.indexed)
        _row(
          t,
          i == 0 ? l10n.exportAlsoWrite : '',
          _why(
            reason,
            Row(children: [
              Expanded(
                child: dialogDropdown<String>(
                  t,
                  id: 'export-also-format-$i',
                  value: a.codec,
                  options: {a.codec, ..._alsoOffered}.toList(),
                  label: exportFormatLabel,
                  onChanged: remote
                      ? null
                      : (codec) => _edit(() => a.codec = codec),
                ),
              ),
              const SizedBox(width: 6),
              HouseTextField(
                key: ValueKey('export-also-suffix-$i'),
                controller: a.suffix,
                width: exportAlsoSuffixWidth,
                hint: l10n.exportAlsoSuffix,
                fill: t.surface0,
                submitOnLostFocus: true,
                onSubmitted: (_) => _edit(() {}),
              ),
              const SizedBox(width: 6),
              _stripButton(t, 'export-also-remove-$i', l10n.exportAlsoRemove,
                  () => _edit(() => _also.removeAt(i).suffix.dispose())),
            ]),
          ),
          dimmed: remote,
        ),
      if (!remote && _also.length < _mostAlso && _alsoOffered.isNotEmpty)
        _row(
          t,
          _also.isEmpty ? l10n.exportAlsoWrite : '',
          Align(
            alignment: AlignmentDirectional.centerStart,
            child: _stripButton(
              t,
              'export-also-add',
              l10n.exportAlsoAdd,
              () => _edit(() {
                // A kind of file not written yet, where there is one.
                final written = {
                  _format.extension,
                  for (final a in _also) _extensionOf(a.codec),
                };
                final codec = _alsoOffered.firstWhere(
                    (c) => !written.contains(_extensionOf(c)),
                    orElse: () => _alsoOffered.first);
                _also.add(_Also(codec, _freeSuffix(codec)));
              }),
            ),
          ),
        ),
    ];
  }
}

/// The suffix field's width: room for a short word.
const double exportAlsoSuffixWidth = 96;
