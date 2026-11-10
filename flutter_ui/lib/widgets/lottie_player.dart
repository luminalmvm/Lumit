// The Lottie and SVG window's first player: the `lottie` package playing the
// exact bytes the export would write, which is the player a Flutter app ships
// with. A dotLottie is read as the zip it is, pictures from its `i/` folder.
//
// The frame comes from outside, as a progress animation the window drives, so
// the transport, the frame counter and the picture cannot disagree. Merge
// paths stay off, as they are in the apps people ship.

import 'dart:async';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';
import 'package:lottie/lottie.dart';

/// [bytes] read as a Flutter app reads them with `Lottie.memory`: pictures
/// and fonts loaded. Each preview is a new file, so none is kept in the
/// package's cache.
Future<LottieComposition> _readLottie(Uint8List bytes, {required bool zip}) async {
  final provider = MemoryLottie(bytes,
      decoder: zip ? LottieComposition.decodeZip : null);
  try {
    return await provider.load();
  } finally {
    Lottie.cache.evict(provider);
  }
}

/// The frame at [progress] (0 at the first frame, as the player's own) as
/// the player draws it, as PNG bytes at the file's own size.
Future<Uint8List?> lottieFramePng(Uint8List bytes,
    {required bool zip, required double progress}) async {
  final composition = await _readLottie(bytes, zip: zip);
  final width = composition.bounds.width;
  final height = composition.bounds.height;
  if (width <= 0 || height <= 0) return null;
  final drawable = LottieDrawable(composition, frameRate: FrameRate.composition)
    ..setProgress(progress);
  final recorder = ui.PictureRecorder();
  drawable.draw(ui.Canvas(recorder),
      Rect.fromLTWH(0, 0, width.toDouble(), height.toDouble()),
      fit: BoxFit.fill);
  final image = await recorder.endRecording().toImage(width, height);
  try {
    final png = await image.toByteData(format: ui.ImageByteFormat.png);
    return png?.buffer.asUint8List();
  } finally {
    image.dispose();
  }
}

class LottiePlayer extends StatefulWidget {
  /// The file, as the export would write it.
  final Uint8List bytes;

  /// Whether [bytes] is a `.lottie` zip rather than a `.json`.
  final bool zip;

  /// Where the player is, 0 at the file's first frame and 1 at its last.
  final Animation<double> progress;

  const LottiePlayer({
    super.key,
    required this.bytes,
    required this.zip,
    required this.progress,
  });

  @override
  State<LottiePlayer> createState() => _LottiePlayerState();
}

class _LottiePlayerState extends State<LottiePlayer> {
  LottieComposition? _composition;

  /// Moves on with every file handed in, so a slow read of an old file
  /// cannot land over a newer one.
  int _read = 0;

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  @override
  void didUpdateWidget(LottiePlayer old) {
    super.didUpdateWidget(old);
    if (!identical(old.bytes, widget.bytes) || old.zip != widget.zip) {
      unawaited(_load());
    }
  }

  Future<void> _load() async {
    final read = ++_read;
    try {
      final composition = await _readLottie(widget.bytes, zip: widget.zip);
      if (!mounted || read != _read) return;
      setState(() => _composition = composition);
    } catch (_) {
      // A file the player can't read leaves the last good picture up.
    }
  }

  @override
  Widget build(BuildContext context) {
    final composition = _composition;
    if (composition == null) return const SizedBox.expand();
    return Lottie(
      composition: composition,
      controller: widget.progress,
      fit: BoxFit.contain,
      // Whole frames, the way the export counts them.
      frameRate: FrameRate.composition,
      options: LottieOptions(enableMergePaths: false),
    );
  }
}
