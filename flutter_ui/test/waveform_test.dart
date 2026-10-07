// What a waveform lane asks for, and what it draws.
//
// No engine here: the request rule and the painter are both plain arithmetic
// over data the bridge hands over, and both are the parts that decide whether
// a zoomed-in wave gains detail or turns into a staircase.

import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' show PointMode;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/waveform_frb.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/theme/theme.dart';

/// Peaks shaped as the bridge returns them: `bands × buckets` triples.
BridgeAudioPeaks peaks({
  required double start,
  required double end,
  required int bands,
  required List<double> values,
}) =>
    BridgeAudioPeaks(
      durationSeconds: 10,
      startSeconds: start,
      endSeconds: end,
      bands: bands,
      buckets: values.length ~/ (3 * bands),
      values: Float32List.fromList(values.map((v) => v.toDouble()).toList()),
    );

/// One full-scale bucket, repeated.
List<double> loud(int buckets) =>
    [for (var i = 0; i < buckets; i++) ...[-1.0, 1.0, 0.5]];

void main() {
  group('what a lane asks for', () {
    test('a zoomed-in view asks for a shorter window, not more buckets',
        (() {
      // The same 800-pixel lane, showing ten seconds and then one.
      final wide = WaveformRequest.forView(
          startSeconds: 0, endSeconds: 10, pixels: 800)!;
      final close = WaveformRequest.forView(
          startSeconds: 4, endSeconds: 5, pixels: 800)!;
      expect(close.endSeconds - close.startSeconds,
          lessThan(wide.endSeconds - wide.startSeconds));
      // Roughly a bucket per pixel either way — the detail comes from the
      // window shrinking, which is exactly what the old fixed-bucket lane
      // could not do.
      expect(wide.buckets, greaterThanOrEqualTo(800));
      expect(close.buckets, greaterThanOrEqualTo(800));
      // And the close view has far more buckets per second of audio.
      final wideDensity = wide.buckets / (wide.endSeconds - wide.startSeconds);
      final closeDensity =
          close.buckets / (close.endSeconds - close.startSeconds);
      expect(closeDensity, greaterThan(wideDensity * 5));
    }));

    test('the bucket count never exceeds what the engine will give', () {
      final r = WaveformRequest.forView(
          startSeconds: 0, endSeconds: 1, pixels: 100000)!;
      expect(r.buckets, lessThanOrEqualTo(maxPeakBuckets));
    });
  });

  group('what a lane draws', () {
    /// A canvas that records the strokes rather than rasterising them: what a
    /// waveform *is* is a column of lines per bucket, so counting those says
    /// more than counting pixels, and it says it in milliseconds.
    const colours = WaveformColours(
      rest: Color(0xff5d8a96),
      low: Color(0xff4a7f9c),
      mid: Color(0xff6fa48c),
      high: Color(0xffaef3e7),
    );

    /// The envelope is drawn at reduced opacity over the same hue as its core,
    /// so a band is told by its colour, not by its alpha. Compared loosely:
    /// a `Paint` keeps its colour as 32-bit floats, so a channel comes back a
    /// few millionths from the double it went in as.
    bool sameHue(Color a, Color b) =>
        (a.r - b.r).abs() < 1e-4 &&
        (a.g - b.g).abs() < 1e-4 &&
        (a.b - b.b).abs() < 1e-4;

    List<_Stroke> strokes(WaveformPainter painter, Size size) {
      final canvas = _RecordingCanvas();
      painter.paint(canvas, size);
      return canvas.lines;
    }

    test('a single wave draws one lane, centred', () {
      final painter = WaveformPainter(
        peaks: peaks(start: 0, end: 1, bands: 1, values: loud(32)),
        originSeconds: 0,
        secondsPerPixel: 1 / 32,
        left: 0,
        right: 32,
        colours: colours,
      );
      final lines = strokes(painter, const Size(32, 32));
      expect(lines, isNotEmpty);
      expect(lines.every((l) => sameHue(l.colour, colours.rest)), isTrue,
          reason: 'the plain wave draws in the waveform colour, not the accent');
      // Every stroke straddles the middle of the one lane it has.
      for (final line in lines) {
        expect(line.a.dy, lessThanOrEqualTo(16));
        expect(line.b.dy, greaterThanOrEqualTo(16));
      }
    });

    /// The stack is drawn *through* the wave, not beside it: every band shares
    /// one lane, so what you read is one silhouette with its inside showing
    /// rather than three small waveforms boxed into thirds.
    test('a multiwave stack shares one lane', () {
      final painter = WaveformPainter(
        peaks: peaks(
          start: 0,
          end: 1,
          bands: 3,
          values: [...loud(8), ...loud(8), ...loud(8)],
        ),
        originSeconds: 0,
        secondsPerPixel: 1 / 24,
        left: 0,
        right: 24,
        colours: colours,
      );
      final lines = strokes(painter, const Size(24, 30));
      expect(lines, isNotEmpty);
      // All three bands present, and every one of them spanning most of the
      // lane rather than a third of it.
      for (final c in [colours.low, colours.mid, colours.high]) {
        final band = lines.where((l) => sameHue(l.colour, c));
        expect(band, isNotEmpty);
        final top = band.map((l) => math.min(l.a.dy, l.b.dy)).reduce(math.min);
        final bottom =
            band.map((l) => math.max(l.a.dy, l.b.dy)).reduce(math.max);
        expect(bottom - top, greaterThan(30 * 0.5),
            reason: 'a band uses the lane, not a third of it');
      }
    });

    test('a wave stops where its bar does', () {
      final painter = WaveformPainter(
        peaks: peaks(start: 0, end: 1, bands: 1, values: loud(32)),
        originSeconds: 0,
        secondsPerPixel: 1 / 32,
        // The bar covers only the right half of the canvas.
        left: 16,
        right: 32,
        colours: colours,
      );
      for (final line in strokes(painter, const Size(32, 16))) {
        expect(line.a.dx, greaterThanOrEqualTo(16));
      }
    });

    /// The lane is the widest thing on the table and the Audio workspace opens
    /// ten of them. A line at a time was a draw call a column a band —
    /// tens of thousands a frame — so a band goes down in one call now, and
    /// the wave it draws is the wave it always drew.
    test('a band is drawn in one call, however many columns it has', () {
      final canvas = _RecordingCanvas();
      WaveformPainter(
        peaks: peaks(
          start: 0,
          end: 1,
          bands: 3,
          values: [...loud(600), ...loud(600), ...loud(600)],
        ),
        originSeconds: 0,
        secondsPerPixel: 1 / 600,
        left: 0,
        right: 600,
        colours: colours,
      ).paint(canvas, const Size(600, 30));
      expect(canvas.lines.length, greaterThan(1500),
          reason: 'every column of every band is still drawn');
      expect(canvas.calls, 3, reason: 'one call a band');
    });

    test('no peaks, or empty peaks, draw nothing at all', () {
      for (final held in [
        null,
        peaks(start: 0, end: 1, bands: 1, values: const []),
      ]) {
        final painter = WaveformPainter(
          peaks: held,
          originSeconds: 0,
          secondsPerPixel: 1 / 32,
          left: 0,
          right: 32,
          colours: colours,
        );
        expect(strokes(painter, const Size(32, 16)), isEmpty);
      }
    });
  });
}

/// One recorded stroke.
class _Stroke {
  final Offset a;
  final Offset b;
  final Color colour;
  const _Stroke(this.a, this.b, this.colour);
}

/// A [Canvas] that keeps the lines instead of drawing them.
class _RecordingCanvas implements Canvas {
  final List<_Stroke> lines = [];

  /// How many times the painter asked the canvas for anything — the number
  /// the batching is about.
  int calls = 0;

  @override
  void drawLine(Offset p1, Offset p2, Paint paint) {
    calls++;
    lines.add(_Stroke(p1, p2, paint.color));
  }

  /// The painter batches a band's columns into one call; a pair of points is
  /// the line it used to draw one at a time, so the recorded list reads the
  /// same either way.
  @override
  void drawRawPoints(PointMode pointMode, Float32List points, Paint paint) {
    calls++;
    for (var i = 0; i + 3 < points.length; i += 4) {
      lines.add(_Stroke(Offset(points[i], points[i + 1]),
          Offset(points[i + 2], points[i + 3]), paint.color));
    }
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => null;
}
