// The export a saved preset makes of a composition, as the export dialogue's
// Export button would make it with that preset chosen. For what a phone asks
// for.

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';

/// The dialogue's fields as [stored], the preset called [preset], fills
/// them, over frames [start] to [end] of a composition set as [settings]. Its
/// own ticks are off, the Viewer's region is not in it, and it sends the
/// size the frame comes out at unless that is the composition's own.
BridgeExportSpec presetExportSpec({
  required String preset,
  required BridgeExportSpec stored,
  required BridgeCompSettings settings,
  required int start,
  required int end,
}) {
  BridgeExportSpec spec(int width, int height) => BridgeExportSpec(
        preset: preset,
        codec: stored.codec,
        width: width,
        height: height,
        bitrateMbps: stored.bitrateMbps,
        peakMbps: stored.peakMbps,
        bitrateAuto: stored.bitrateAuto,
        fps: stored.fps,
        rangeStartFrame: start,
        rangeEndFrame: end,
        includeAudio: stored.includeAudio,
        audioBitRate: stored.audioBitRate,
        audioRate: stored.audioRate,
        audioDepth: stored.audioDepth,
        audioChannels: stored.audioChannels,
        depth: stored.depth,
        alphaChannel: stored.alphaChannel,
        straightAlpha: stored.straightAlpha,
        colourSpace: stored.colourSpace,
        resample: stored.resample,
        cropTop: stored.cropTop,
        cropLeft: stored.cropLeft,
        cropBottom: stored.cropBottom,
        cropRight: stored.cropRight,
        useRegionOfInterest: false,
        region: Float64List(0),
        metadata: const [],
        qualityDivisor: stored.qualityDivisor,
        diskCacheReadOnly: stored.diskCacheReadOnly,
        effects: stored.effects,
        honourSolo: stored.honourSolo,
        renderGuides: stored.renderGuides,
        captions: stored.captions,
        motionBlur: stored.motionBlur,
        retimeBlend: stored.retimeBlend,
        useProxies: stored.useProxies,
        makeANoise: false,
        openFolder: false,
        gifDither: stored.gifDither,
        gifPalettePerFrame: stored.gifPalettePerFrame,
        gifPlays: stored.gifPlays,
      );
  var (width, height) = (stored.width, stored.height);
  if (width <= 0 || height <= 0) {
    final crop = exportCropFor(
        spec: spec(0, 0),
        compWidth: settings.width,
        compHeight: settings.height);
    final divisor = stored.qualityDivisor.clamp(1, 4);
    width = (crop.width / divisor).round().clamp(1, 16384);
    height = (crop.height / divisor).round().clamp(1, 16384);
  }
  final own = width == settings.width && height == settings.height;
  return spec(own ? 0 : width, own ? 0 : height);
}
