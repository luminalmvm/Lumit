// A picture on the system clipboard, as files Lumit can import.
//
// Flutter's own clipboard reads text and nothing else, so the picture comes
// through the `pasteboard` plugin. Two shapes arrive and each is answered
// differently:
//
// * **Files**, copied in a file manager. They are imported where they are, so
//   the project points at the original and no copy is made;
// * **Pixels**, from a browser, a screenshot tool or an editor. There is no
//   file to point at, so the picture is written as a PNG, and that file is what
//   gets imported.
//
// A picture the plugin cannot read, or no plugin at all (a test), is the same
// as nothing on the clipboard.

import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:pasteboard/pasteboard.dart';

/// The picture formats Paste takes as files: the stills the engine reads and
/// GIF. Video, sound and models copied in a file manager are left alone.
const Set<String> _pictureExtensions = {
  'png', 'jpg', 'jpeg', 'tif', 'tiff', 'exr', 'tga', 'targa', 'dpx', 'bmp',
  'webp', 'ppm', 'pgm', 'pnm', 'pbm', 'hdr', 'gif',
};

/// The paths of [paths] that are pictures that exist, in the order given.
List<String> pictureFiles(Iterable<String> paths) => [
      for (final path in paths)
        if (_pictureExtensions.contains(_extensionOf(path)) &&
            File(path).existsSync())
          path,
    ];

String _extensionOf(String path) {
  final dot = path.lastIndexOf('.');
  final slash = path.lastIndexOf(RegExp(r'[/\\]'));
  return dot <= slash ? '' : path.substring(dot + 1).toLowerCase();
}

/// The picture files copied in a file manager, or none.
Future<List<String>> clipboardPictureFiles() async {
  try {
    return pictureFiles(await Pasteboard.files());
  } catch (_) {
    return const [];
  }
}

/// The picture the clipboard holds as pixels, as PNG, or null when it holds
/// none or what it holds cannot be read.
///
/// Windows hands the plugin's bytes over as a bitmap file, so they are decoded
/// and written again as PNG. Bytes that are PNG already go through untouched.
Future<Uint8List?> clipboardPicturePng() async {
  final Uint8List? raw;
  try {
    raw = await Pasteboard.image;
  } catch (_) {
    return null;
  }
  if (raw == null || raw.isEmpty) return null;
  if (_isPng(raw)) return raw;
  ui.Codec? codec;
  try {
    codec = await ui.instantiateImageCodec(raw);
    final frame = await codec.getNextFrame();
    try {
      final data =
          await frame.image.toByteData(format: ui.ImageByteFormat.png);
      return data?.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes);
    } finally {
      frame.image.dispose();
    }
  } catch (_) {
    return null;
  } finally {
    codec?.dispose();
  }
}

bool _isPng(Uint8List bytes) =>
    bytes.length > 8 &&
    bytes[0] == 0x89 &&
    bytes[1] == 0x50 &&
    bytes[2] == 0x4E &&
    bytes[3] == 0x47;

/// Write [png] under [home] as `<name>/<name>.png`, with the first [name] that
/// has no folder there yet, and answer the file's path.
///
/// Each picture is alone in a folder of its own. Import reads a numbered file
/// beside another with the same name as a run of frames and folds the two into
/// one image sequence, which is what `Pasted image 1.png` and `Pasted image
/// 2.png` in one folder would be taken for.
Future<String> savePastedPicture(
  Uint8List png, {
  required Directory home,
  required String Function(int number) name,
}) async {
  final sep = Platform.pathSeparator;
  for (var number = 1;; number++) {
    final stem = name(number);
    final folder = Directory('${home.path}$sep$stem');
    if (folder.existsSync()) continue;
    await folder.create(recursive: true);
    final file = File('${folder.path}$sep$stem.png');
    await file.writeAsBytes(png, flush: true);
    return file.path;
  }
}
