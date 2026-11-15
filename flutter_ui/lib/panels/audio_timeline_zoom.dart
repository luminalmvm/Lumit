// How far in the Audio timeline zooms, and which summary a wave asks for once
// it is that far in.

import 'dart:math' show max, min;
import 'dart:typed_data';

import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/layer/clip_peaks.dart';

import 'waveform_frb.dart';

/// The engine's finest summary block, in samples. Closer in than a block a
/// column, a wave's peaks are read off the samples themselves.
const double _finestBlock = 256;

/// How many samples full zoom-in shows across the lanes: few enough that each
/// stands a few pixels from the next, where the wave draws its samples.
const double _samplesAtFullZoom = 256;

/// The widest the lanes are laid out, in pixels. Past this the drawing's
/// own arithmetic cannot place a column to the nearest pixel.
const double _widestLanes = 8388608;

/// The layer Timeline's own full zoom, in frames across, which a long comp
/// still reaches however wide that lays it out.
const double _framesAtTimelineZoom = 20;

/// The deepest zoom for [frames] frames at [fps] across [viewport] pixels:
/// single samples apart, as far as the lanes' width allows, and never
/// shallower than the layer Timeline's.
double audioMaxZoom(
    {required int frames, required double fps, required double viewport}) {
  if (frames <= 0) return 1;
  final timeline = frames / _framesAtTimelineZoom;
  if (fps <= 0 || viewport <= 0) return max(1, timeline);
  final samples = frames / (fps * _samplesAtFullZoom / waveSampleRate);
  return max(1, max(timeline, min(samples, _widestLanes / viewport)));
}

/// Whether a wave drawn at [secondsPerPixel] can show the three-band stack.
/// Closer in, the bands are the filtered signal itself, three thin wavering
/// lines rather than a picture of the sound, so the full-range wave is drawn.
bool waveBandsResolve(double secondsPerPixel) =>
    secondsPerPixel * waveSampleRate >= _finestBlock;

/// [request]'s buckets, never more than it holds samples. Closer in than a
/// sample a bucket most buckets catch none, and the wave is drawn as a comb.
int waveBuckets(WaveformRequest request) => min(request.buckets,
    max(1, ((request.endSeconds - request.startSeconds) * waveSampleRate).floor()));

/// An uncut track's wave over [request], drawn at [secondsPerPixel]: the
/// stack while it resolves, else the full wave, and the samples beside it
/// once they stand apart.
Future<BridgeAudioPeaks> trackPeaks(LayerReference layer,
        WaveformRequest request, double secondsPerPixel) =>
    layer
        .audioPeaks(
          startSeconds: request.startSeconds,
          endSeconds: request.endSeconds,
          buckets: waveBuckets(request),
          multiwave: waveBandsResolve(secondsPerPixel),
        )
        .then((peaks) => withSamples(peaks, secondsPerPixel, request,
            (a, b) => layer.audioSamples(startSeconds: a, endSeconds: b)));

/// A mix's [peaks], which come with no samples, read as samples once one is
/// wider than a pixel: each bucket then holds one sample, and the middle of
/// its two extremes is it. Drawn as the line a file's samples are, where a
/// summary's columns would stand no taller than a hairline and vanish.
Future<BridgeAudioPeaks> mixAsSamples(BridgeAudioPeaks peaks,
        WaveformRequest request, double secondsPerPixel) =>
    withSamples(peaks, secondsPerPixel, request, (a, b) async {
      final n = peaks.buckets;
      final rows = peaks.channels == 2 ? 2 : 1;
      final values = Float32List(n * rows);
      for (var c = 0; c < rows; c++) {
        for (var i = 0; i < n; i++) {
          final at = 3 * (c * peaks.bands * n + i);
          if (at + 1 >= peaks.values.length) break;
          values[c * n + i] = (peaks.values[at] + peaks.values[at + 1]) / 2;
        }
      }
      return BridgeAudioSamples(
          startSeconds: peaks.startSeconds,
          endSeconds: peaks.endSeconds,
          frames: n,
          channels: rows,
          values: values);
    });
