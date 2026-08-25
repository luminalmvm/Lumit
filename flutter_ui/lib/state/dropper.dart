// The dropper: what is armed, and the arithmetic on the pixels it reads.
//
// **In plain terms.** The dropper is the little pipette beside a parameter.
// Click it and the tool arms: the Viewer grows a magnifier that follows the
// pointer and shows the pixels under it, hugely enlarged, and the next click on
// the picture lifts a value from them. What that value *is* depends on what
// armed it — a colour for a colour swatch, a depth for a depth-of-field focal
// point — which is why this file talks about "what is being read" rather than
// about colour.
//
// **Which pixel grid all of this is in.** The engine's reply says which raster
// it cut from, and that is the only grid the window can be indexed in: the
// picture read is often a reduced-resolution preview, so its pixels are not the
// composition's. Every position here is therefore a pixel of the *reply's* own
// raster, arrived at from a fraction of the picture (which is what the Viewer
// actually knows: where the pointer is inside the drawn image). Mixing the two
// grids is what made the magnifier show one repeated edge pixel — a flat colour
// where the picture should be.
//
// **Why a window rather than nine pixels.** The engine answers a read with a
// whole square of the picture — 129 pixels a side — and the magnifier's own 9×9
// grid is cut out of it here, in Dart. The pointer can then move, and the sample
// size change, without asking the engine anything at all: a sweep across the
// picture costs a handful of reads rather than one per mouse move. A new read is
// needed only when the pointer approaches the window's edge, the playhead moves,
// an edit lands, or a different layer is being read.
//
// Nothing here draws anything or crosses the bridge. It holds the armed state
// and does the sums on the pixels, so both are testable without a running
// engine — the widget in widgets/dropper_overlay.dart is the part with pixels
// on screen.

import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';

/// What a pick is being read *for*. The magnifier looks the same in every case
/// — it is the caption and the committed value that differ.
enum DropperReads {
  /// The averaged colour of the region, in scene-linear RGB.
  colour,

  /// A 0–1 depth: the region's luma, read off a depth pass. What the
  /// depth-of-field Focus takes (docs/08 §3.22).
  depth,

  /// The pixel's own position in the composition, for an x/y parameter pair.
  position,
}

/// One armed dropper: what is being picked, where the pixels come from, and
/// what to do with the answer.
///
/// The commit is a callback rather than a description of the target because the
/// thing being edited already knows how to edit itself: the parameter row hands
/// over a closure that writes its own value through the ordinary undoable path.
/// Naming the target instead would mean re-resolving a layer, an effect and a
/// parameter index on the far side of the picture, which is the arrangement the
/// egui build had and the source of its "target has since moved" silence.
@immutable
class DropperArm {
  /// Which control armed it — `fx-<effect>-<param>`, say. Only ever compared,
  /// so that the button that armed the tool can show itself lit while it is.
  final String id;

  final DropperReads reads;

  /// What is being picked, for the magnifier's caption: the parameter's own
  /// name, as the user sees it in the panel ("Key colour", "Focus").
  final String label;

  /// Read this layer *alone* rather than the composite — a depth pass, which is
  /// usually hidden and so is nowhere to be seen in the composite. Null samples
  /// the picture as shown.
  final LayerReference? sampleLayer;

  /// The name of that layer, so the caption can say where the numbers are
  /// coming from rather than showing a colour nobody is picking.
  final String? sampleLayerName;

  /// Write the picked value. Called once, on the click that picks.
  final void Function(DropperSample sample) onPick;

  const DropperArm({
    required this.id,
    required this.reads,
    required this.label,
    required this.onPick,
    this.sampleLayer,
    this.sampleLayerName,
  });
}

/// What one pick lifted off the picture.
@immutable
class DropperSample {
  /// The region's average colour in **scene-linear** RGB, each 0–1 — the space
  /// a Colour parameter stores, so it is written straight through.
  final double r, g, b;

  /// The region's average Rec. 709 luma in linear light, 0–1: the depth proxy.
  final double depth;

  /// Where the centre pixel sits, in the raster the window was read from (the
  /// reply's `width`/`height`) — which is the composition's own grid only when
  /// the picture was read at full resolution.
  final int x, y;

  /// The same position as a fraction of the picture (pixel centre over the
  /// reply's own raster) — resolution-independent, which is what a position
  /// pick writes into an x/y parameter pair (docs/07 §6.1).
  final double xFrac, yFrac;

  /// How many pixels a side were averaged.
  final int region;

  const DropperSample({
    required this.r,
    required this.g,
    required this.b,
    required this.depth,
    required this.x,
    required this.y,
    this.xFrac = 0,
    this.yFrac = 0,
    required this.region,
  });
}

/// The sample sizes Shift+scroll steps through (docs/07 §6.1). Odd throughout, so
/// there is always a single centre pixel; 1 is the default, meaning "this pixel
/// and no other".
const List<int> dropperRegions = [1, 3, 5, 7, 9];

/// The magnifier's grid, and so the largest region: nine pixels a side. The
/// region can never exceed what is shown.
const int dropperGrid = 9;

/// How much of the picture one read brings back: 129 pixels a side, 66 KiB.
///
/// Must match the engine's own cap (`worker_thread::MAX_WINDOW`), which clamps
/// it anyway — the reply says how big it actually is, and nothing here assumes.
const int dropperWindow = 129;

/// Whether the window in hand still has every pixel the magnifier needs around
/// `(x, y)` — the pixel under the pointer, in the window's own raster.
///
/// The half-grid margin is what makes a read last: the pointer may travel to
/// within four pixels of the window's edge before another is needed. Edge
/// repeats mean a window against the picture's border still covers points
/// beyond it, so no border case is needed here either.
bool windowCovers(BridgeSampledPixels window, int x, int y) {
  final half = window.window ~/ 2;
  final reach = dropperGrid ~/ 2;
  return (x - reach) >= window.x - half &&
      (x + reach) <= window.x + half &&
      (y - reach) >= window.y - half &&
      (y + reach) <= window.y + half;
}

/// The pixel of the reply's raster at the fraction `(u, v)` of the picture.
///
/// The one place a point in the picture becomes a pixel, and it goes through
/// the reply's **own** `width`/`height` — never the composition's, which is a
/// different grid whenever the Viewer is showing a reduced-resolution preview.
(int, int) windowPixelAt(BridgeSampledPixels window, double u, double v) => (
      (u.clamp(0.0, 1.0) * window.width).floor().clamp(0, window.width - 1),
      (v.clamp(0.0, 1.0) * window.height).floor().clamp(0, window.height - 1),
    );

/// One pixel of a window, by its position in the reply's raster, or null when
/// that position lies outside the window.
///
/// **Null rather than clamped to the edge.** The window already carries the
/// picture's own edge repeats, so a pixel beyond the picture's border is inside
/// it and answers normally; a position outside the *window* means the caller is
/// asking in the wrong grid or has outrun what it holds, and a clamp would
/// answer that with a plausible colour — which is exactly how a whole magnifier
/// of one repeated edge pixel went unnoticed. Blank cells say it instead.
({int r, int g, int b})? windowPixel(BridgeSampledPixels window, int x, int y) {
  final half = window.window ~/ 2;
  final col = x - (window.x - half);
  final row = y - (window.y - half);
  if (col < 0 || row < 0 || col >= window.window || row >= window.window) {
    return null;
  }
  final i = (row * window.window + col) * 4;
  if (i + 2 >= window.rgba.length) return null;
  return (r: window.rgba[i], g: window.rgba[i + 1], b: window.rgba[i + 2]);
}

/// The next region up or down the [dropperRegions] ladder. `steps` is signed —
/// one Shift+scroll notch either way — and the ends hold rather than wrap: a
/// picker that jumped from 9×9 back to 1×1 on one extra notch would lose the
/// size the user had settled on.
int nextDropperRegion(int current, int steps) {
  final at = dropperRegions.indexOf(current);
  final from = at == -1 ? 0 : at;
  return dropperRegions[(from + steps).clamp(0, dropperRegions.length - 1)];
}

/// One sRGB byte as scene-linear light, 0–1 (IEC 61966-2-1).
///
/// The patch arrives as display-ready sRGB — the bytes the picture is made of —
/// and a Colour parameter is scene-linear, so every average is taken *after*
/// this. Averaging the bytes instead would make a region of one white pixel and
/// three black ones mid-grey rather than the quarter-light it actually is.
double srgbDecode(int byte) {
  final v = byte.clamp(0, 255) / 255.0;
  return v <= 0.04045 ? v / 12.92 : math.pow((v + 0.055) / 1.055, 2.4).toDouble();
}

/// Scene-linear light back to an sRGB byte — the inverse of [srgbDecode], for
/// showing a linear value as a swatch.
int srgbEncode(double linear) {
  final v = linear.clamp(0.0, 1.0);
  final e = v <= 0.0031308
      ? v * 12.92
      : 1.055 * math.pow(v, 1 / 2.4).toDouble() - 0.055;
  return (e * 255).round().clamp(0, 255);
}

/// Average the `region × region` pixels around `(x, y)` of a window, and say
/// what they mean.
///
/// `(x, y)` is the pixel under the pointer, in the reply's own raster — not the
/// window's centre, which is only where the pointer *was* when the window was
/// read. A region wider than the magnifier's grid is clamped rather than taken.
DropperSample sampleFromWindow(
  BridgeSampledPixels window,
  int region,
  int x,
  int y,
) {
  final n = region.clamp(1, dropperGrid);
  final half = (n - 1) ~/ 2;
  var sr = 0.0, sg = 0.0, sb = 0.0;
  var count = 0;
  for (var dy = -half; dy <= half; dy++) {
    for (var dx = -half; dx <= half; dx++) {
      final px = windowPixel(window, x + dx, y + dy);
      if (px == null) continue;
      sr += srgbDecode(px.r);
      sg += srgbDecode(px.g);
      sb += srgbDecode(px.b);
      count++;
    }
  }
  final xFrac = (x + 0.5) / math.max(window.width, 1);
  final yFrac = (y + 0.5) / math.max(window.height, 1);
  if (count == 0) {
    return DropperSample(
        r: 0, g: 0, b: 0, depth: 0, x: x, y: y, xFrac: xFrac, yFrac: yFrac, region: n);
  }
  final r = sr / count, g = sg / count, b = sb / count;
  return DropperSample(
    r: r,
    g: g,
    b: b,
    // Rec. 709 luma in linear light: a grey depth pass has luma equal to its
    // own value, which is the number the effect reads.
    depth: (0.2126 * r + 0.7152 * g + 0.0722 * b).clamp(0.0, 1.0),
    x: x,
    y: y,
    xFrac: xFrac,
    yFrac: yFrac,
    region: n,
  );
}
