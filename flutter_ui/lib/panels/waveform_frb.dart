// Drawing a waveform: the single wave, the multiwave stack, and the rule that
// decides which stretch of audio to ask the engine for (K-280).
//
// In plain terms: the engine hands back a *summary* of a stretch of sound —
// for each bucket, how far the signal swung down, how far it swung up, and how
// much energy it carried. This file turns that into pixels, and works out what
// to ask for in the first place.
//
// Two things make it more than a for-loop over buckets.
//
// **The resolution follows the zoom.** A summary is only ever as detailed as
// the window it was taken over, so a lane asks for the stretch it is actually
// showing, at one bucket per pixel column. Zoom in and it asks again over a
// shorter stretch, and the wave gains detail instead of growing blocky.
// [WaveformRequest] is that ask, and it deliberately rounds itself off so that
// nudging the scrollbar does not send a fresh request per pointer move.
//
// **The multiwave.** One wave says how loud a moment is and nothing about what
// is in it: a mastered track is a solid block whether it is a kick, a snare or
// a vocal. So the engine can split the sound into three bands and summarise
// each, and [WaveformPainter] draws all three **over one another in the same
// lane** — so a kick and a hi-hat are told apart inside one silhouette, at a
// row height where three separate lanes would each be six pixels tall and say
// nothing.
//
// Two rules make three overlaid waves readable rather than a pile.
//
// **Lightest at the back.** The bands are drawn treble first and bass last, so
// the pale end of the ramp sits behind and each darker band lands in front of
// it. A dark shape on a pale one reads as two shapes; the other way round the
// pale one swallows what is under it.
//
// **Each one a little higher.** Every band after the first is lifted a couple
// of pixels above the one behind it. Perfectly concentric waves hide each
// other wherever they happen to agree — which, for three bands of one sound,
// is most of the time; the offset keeps a sliver of each visible whatever the
// others are doing, the way a fanned hand of cards shows every card.
//
// Overlaid rather than stacked on purpose: the whole point is to see *inside*
// the wave you are already reading, not to read three small waves and add them
// up in your head. The single wave stays available in Settings for anyone who
// wants the plain picture, and is drawn exactly as it always was.
//
// **Where the wave sits** is a second, independent choice ([WaveformStyle]). A
// waveform is symmetrical about silence, so half of a centred one is a mirror
// of the other half and says nothing twice. Folding it onto the floor spends
// the whole row's height on the half that carries the information, which reads
// far better in a short row and is the shape most NLEs draw. Centred is the
// default because it is what the eye expects of a *wave*; from the bottom is
// there for anyone who would rather have the height.
//
// **How much room it has** is the last of it (K-437). A waveform lane in the
// Timeline is only ever open under its own **Waveform** twirl, and that twirl's
// own row is empty lane space — so the lane is drawn twice a row tall, standing
// on its own floor and reaching up through the row above. A centred wave then
// sits about the divider between the two, which is where silence actually is,
// and one rising from the floor gets both rows to rise through. Only the
// painting reaches up; the row is the height it always was, so nothing in the
// outline or the lane stack moves.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../theme/theme.dart';

/// The most buckets one request may ask for. Mirrors `MAX_PEAK_BUCKETS` on the
/// engine side — the engine clamps too, and asking for what it will not give
/// would mean a lane whose buckets and columns quietly disagree.
const int maxPeakBuckets = 4096;

/// How many pixel columns share one bucket. One each is the honest answer and
/// what this uses: the wave is drawn a column at a time, so a bucket per
/// column is exactly enough detail and no more.
const double pixelsPerBucket = 1;

/// A lane's ask: which stretch of a source to summarise, and how finely.
///
/// **Rounded on purpose.** The window is snapped outward to a grid a fraction
/// of its own width, and the bucket count to a power of two, so scrolling by a
/// pixel or nudging the zoom leaves the request identical and no new work is
/// started. Only a real move — a zoom step, a scroll of a fair part of the
/// view — changes it.
@immutable
class WaveformRequest {
  final double startSeconds;
  final double endSeconds;
  final int buckets;

  const WaveformRequest({
    required this.startSeconds,
    required this.endSeconds,
    required this.buckets,
  });

  /// The request for a lane showing `[start, end)` seconds across `pixels`
  /// pixels, padded either side so a small scroll stays inside what has
  /// already been fetched.
  ///
  /// Returns null when there is nothing to draw — a zero-width lane, or a
  /// window with no time in it.
  static WaveformRequest? forView({
    required double startSeconds,
    required double endSeconds,
    required double pixels,
  }) {
    if (!(endSeconds > startSeconds) || !(pixels > 0)) return null;
    final span = endSeconds - startSeconds;
    // Half a view either side: enough that ordinary scrolling never outruns
    // the fetched window, cheap enough that it is one request either way.
    final pad = span * 0.5;
    final grid = span * 0.25;
    // The start snaps to the grid and the *width* is a whole number of grid
    // steps — rather than snapping both ends, which would round two boundaries
    // and change the window twice as often for no more coverage.
    final from = ((startSeconds - pad) / grid).floorToDouble() * grid;
    final to = from + ((span + pad * 2) / grid).ceilToDouble() * grid;
    // The padded window is drawn over the same pixels, so it wants
    // proportionally more buckets to keep one per column.
    final wanted = pixels / pixelsPerBucket * (to - from) / span;
    final buckets = _roundUpToPowerOfTwo(wanted.ceil()).clamp(64, maxPeakBuckets);
    return WaveformRequest(startSeconds: from, endSeconds: to, buckets: buckets);
  }

  /// The key this request files its answer under: two requests with the same
  /// key would fetch the same summary, so the second is never sent.
  String get key => '${startSeconds.toStringAsFixed(4)}'
      '|${endSeconds.toStringAsFixed(4)}|$buckets';

  @override
  bool operator ==(Object other) =>
      other is WaveformRequest &&
      other.startSeconds == startSeconds &&
      other.endSeconds == endSeconds &&
      other.buckets == buckets;

  @override
  int get hashCode => Object.hash(startSeconds, endSeconds, buckets);
}

int _roundUpToPowerOfTwo(int n) {
  var p = 64;
  while (p < n && p < maxPeakBuckets) {
    p *= 2;
  }
  return p;
}

/// How a waveform is drawn — the two choices Settings offers, together,
/// because a painter wants them together and neither means much alone.
@immutable
class WaveformStyle {
  /// Draw the three-band stack rather than one full-range wave.
  final bool multiwave;

  /// Stand the wave on the floor of its row, rectified, instead of centring it
  /// about silence. Each column then reaches up by how far the signal swung
  /// *either* way, so the whole row's height carries signal rather than half
  /// of it mirroring the other half.
  final bool fromBottom;

  const WaveformStyle({this.multiwave = true, this.fromBottom = false});

  /// What the peaks are fetched for. Only [multiwave] reaches the engine —
  /// where the wave sits is a drawing decision, so switching it repaints
  /// without asking for anything.
  bool get needsBands => multiwave;

  @override
  bool operator ==(Object other) =>
      other is WaveformStyle &&
      other.multiwave == multiwave &&
      other.fromBottom == fromBottom;

  @override
  int get hashCode => Object.hash(multiwave, fromBottom);
}

/// The waveform of one span of audio, drawn a pixel column at a time.
///
/// The peaks carry their own clock — source seconds for a layer, clip-local
/// seconds for a Sequence clip — and the painter is told how to get from a
/// canvas x to that clock: `time(x) = originSeconds + x * secondsPerPixel`.
/// Both callers are straight lines in x, which is what lets a bar be dragged
/// or a clip slid with the wave following it and nothing refetched.
class WaveformPainter extends CustomPainter {
  final BridgeAudioPeaks? peaks;

  /// The peaks' own clock at canvas x = 0.
  final double originSeconds;
  final double secondsPerPixel;

  /// The columns to draw between — the visible part of the bar or clip.
  final double left;
  final double right;

  final WaveformColours colours;

  /// Where the wave sits and whether the bands are stacked.
  final WaveformStyle style;

  /// Vertical breathing room top and bottom, so a full-scale wave does not
  /// touch the row's edges.
  final double inset;

  /// How tall to draw, when that is taller than the row the painter sits in.
  /// The wave is anchored to the **bottom** of `size` and reaches up past its
  /// top; null means the row's own height, which is what a clip uses.
  ///
  /// The Timeline's lane passes twice a row here (K-437). A waveform row is
  /// only ever there under its own **Waveform** twirl, and that twirl's row is
  /// empty lane space — so the wave is given both rows and a centred one is
  /// drawn about the divider between them, which is the line silence actually
  /// sits on. Nothing about the layout moves: the row is the height it always
  /// was, and only the painting reaches above it.
  final double? height;

  const WaveformPainter({
    required this.peaks,
    required this.originSeconds,
    required this.secondsPerPixel,
    required this.left,
    required this.right,
    required this.colours,
    this.style = const WaveformStyle(),
    this.inset = 1,
    this.height,
  });

  /// The bands to draw, back to front: which slice of the answer each one is,
  /// and what colour it takes.
  ///
  /// Treble first and bass last — the band order reversed, so the palest end
  /// of the ramp sits behind and each darker band lands in front of it. A dark
  /// shape on a pale one reads as two shapes; the other way round the pale one
  /// swallows what is under it. Fixed by band rather than worked out from the
  /// colours, so the picture is the same whatever a theme makes of them.
  List<({int band, Color colour})> get _bandOrder =>
      switch (peaks?.bands ?? 0) {
        3 => [
            (band: 2, colour: colours.high),
            (band: 1, colour: colours.mid),
            (band: 0, colour: colours.low),
          ],
        _ => [(band: 0, colour: colours.rest)],
      };

  @override
  void paint(Canvas canvas, Size size) {
    final held = peaks;
    if (held == null || held.buckets == 0 || held.values.isEmpty) return;
    if (!(held.endSeconds > held.startSeconds)) return;
    final from = math.max(0.0, left);
    final to = math.min(size.width, right);
    if (!(to > from)) return;

    final bands = _bandOrder;
    // One lane, whichever this is: the stack is drawn *through* the wave, not
    // beside it.
    final stacked = bands.length > 1;
    // The band the wave is drawn in: as tall as [height] asks for, standing on
    // the bottom of the row and reaching up from there. Equal to the row
    // itself unless a lane has borrowed the empty row above it.
    final tall = height ?? size.height;
    final top = size.height - tall;
    // Centred about silence, or stood on the floor of the band. Standing on the
    // floor spends the whole height on one rectified half, so `reach` is twice
    // what a centred wave's is.
    final baseline = style.fromBottom ? top + tall - inset : top + tall / 2;
    // How far each band sits above the one behind it, and how much room that
    // costs the wave itself. Proportional to the band so a tall clip fans wider
    // than a 22 px lane, and capped at both ends: below a pixel the offset is
    // invisible, above a few it eats more amplitude than it earns.
    final step = stacked ? (tall * 0.08).clamp(1.0, 4.0) : 0.0;
    final fan = step * (bands.length - 1);
    final reach = math.max(
      0.5,
      (style.fromBottom ? tall - inset * 2 : tall / 2 - inset) - fan,
    );
    final buckets = held.buckets;
    final span = held.endSeconds - held.startSeconds;

    for (var drawn = 0; drawn < bands.length; drawn++) {
      final band = bands[drawn].band;
      final colour = bands[drawn].colour;
      // Back to front, each a little higher than the last.
      final floor = baseline - step * drawn;
      // The single wave keeps its softened envelope and its solid energy core
      // — the shape people already read. A band in the stack is drawn solid
      // and coreless instead: three softened envelopes over one another blend
      // into a wash, where three solid ones let the brightest reach through.
      final body = Paint()
        ..color = stacked ? colour : colour.withValues(alpha: colour.a * 0.8)
        ..strokeWidth = 1;
      final core = Paint()
        ..color = colour
        ..strokeWidth = 1;

      for (var x = from.floorToDouble(); x < to; x += 1) {
        final seconds = originSeconds + (x + 0.5) * secondsPerPixel;
        final at = (seconds - held.startSeconds) / span * buckets;
        if (at < 0 || at >= buckets) continue;
        final bucket = at.floor();
        final base = 3 * (band * buckets + bucket);
        if (base + 2 >= held.values.length) continue;
        final lo = held.values[base].clamp(-1.0, 1.0);
        final hi = held.values[base + 1].clamp(-1.0, 1.0);
        final rms = held.values[base + 2].clamp(0.0, 1.0);
        if (lo == 0 && hi == 0 && rms == 0) continue;
        if (style.fromBottom) {
          // Rectified: the column reaches up by how far the signal swung
          // either way, whichever was further.
          final amp = math.max(hi.abs(), lo.abs());
          canvas.drawLine(
            Offset(x + 0.5, floor),
            Offset(x + 0.5, floor - amp * reach),
            body,
          );
        } else {
          canvas.drawLine(
            Offset(x + 0.5, floor - hi * reach),
            Offset(x + 0.5, floor - lo * reach),
            body,
          );
        }
        // The energy inside the envelope: what tells a sustained note from a
        // spike that happens to reach the same height. The stack says that
        // with its own brightness, so only the single wave draws it.
        if (!stacked && rms > 0) {
          canvas.drawLine(
            Offset(x + 0.5, floor - rms * reach),
            Offset(x + 0.5, style.fromBottom ? floor : floor + rms * reach),
            core,
          );
        }
      }
    }
  }

  @override
  bool shouldRepaint(WaveformPainter old) =>
      !identical(old.peaks, peaks) ||
      old.originSeconds != originSeconds ||
      old.secondsPerPixel != secondsPerPixel ||
      old.left != left ||
      old.right != right ||
      old.colours != colours ||
      old.style != style ||
      old.inset != inset ||
      old.height != height;

  /// A background painter's default is to absorb hits across its whole rect,
  /// which would eat the keyframe marquee underneath. The lane is a picture,
  /// not a control.
  @override
  bool? hitTest(Offset position) => false;
}
