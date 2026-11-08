// The formats an export can be written in.

part of 'export_dialog_frb.dart';

/// One output format: the engine key, the reading label, and the file
/// extension its picker filters on.
class _Format {
  final String key;
  final String label;
  final String extension;
  final String pickerLabel;
  final ExportOutputType type;
  const _Format(
      this.key, this.label, this.extension, this.pickerLabel, this.type);
}

/// The formats on offer. The browser build offers those its page has an
/// encoder for, and a browser with none still opens the dialog, on a format
/// the engine then refuses.
List<_Format> get _formats {
  if (!kIsWeb) return _desktopFormats;
  final offered = [
    for (final f in [..._desktopFormats, ..._webFormats])
      if (_offered.contains(f.key)) f,
  ];
  return offered.isEmpty ? _desktopFormats.take(1).toList() : offered;
}

/// The keys the page can write, asked once rather than on each rebuild.
final Set<String> _offered = {
  for (final f in [..._desktopFormats, ..._webFormats])
    if (exportFormatCaps(codec: f.key).video) f.key,
};

/// Only the browser build writes WebM. It has no Save dialogue of its own to
/// name the type in, so the label stands in.
List<_Format> get _webFormats => [
      _Format('vp9', l10n.formatVp9, 'webm', l10n.formatVp9,
          ExportOutputType.video),
      _Format('av1', l10n.formatAv1, 'webm', l10n.formatAv1,
          ExportOutputType.video),
    ];

/// H.264 first because it is the delivery default; the sequences and the
/// sound-only containers after it, because they are the specialist choices.
List<_Format> get _desktopFormats => [
      _Format('h264', l10n.formatH264, 'mp4', l10n.formatMp4Picker,
          ExportOutputType.video),
      _Format('hevc', l10n.formatHevc, 'mp4', l10n.formatMp4Picker,
          ExportOutputType.video),
      _Format('prores_proxy', l10n.formatProRes422Proxy, 'mov',
          l10n.formatMovPicker, ExportOutputType.video),
      _Format('prores_lt', l10n.formatProRes422Lt, 'mov',
          l10n.formatMovPicker, ExportOutputType.video),
      _Format('prores_422', l10n.formatProRes422, 'mov',
          l10n.formatMovPicker, ExportOutputType.video),
      _Format('prores_hq', l10n.formatProRes422Hq, 'mov',
          l10n.formatMovPicker, ExportOutputType.video),
      _Format('prores_4444', l10n.formatProRes4444, 'mov',
          l10n.formatMovPicker, ExportOutputType.video),
      _Format('prores_4444xq', l10n.formatProRes4444Xq, 'mov',
          l10n.formatMovPicker, ExportOutputType.video),
      _Format('gif', l10n.formatGif, 'gif', l10n.formatGifPicker,
          ExportOutputType.video),
      _Format('png', l10n.formatPngSequence, 'png', l10n.formatPngPicker,
          ExportOutputType.imageSequence),
      _Format('tiff', l10n.formatTiffSequence, 'tiff', l10n.formatTiffPicker,
          ExportOutputType.imageSequence),
      _Format('exr', l10n.formatExrSequence, 'exr', l10n.formatExrPicker,
          ExportOutputType.imageSequence),
      _Format('m4a', l10n.formatM4a, 'm4a', l10n.formatM4aPicker,
          ExportOutputType.audioOnly),
      _Format('wav', l10n.formatWav, 'wav', l10n.formatWavPicker,
          ExportOutputType.audioOnly),
    ];

/// The reading name of the format [key], or the key itself for one this build
/// does not offer.
String exportFormatLabel(String key) =>
    _formats.where((f) => f.key == key).firstOrNull?.label ?? key;
