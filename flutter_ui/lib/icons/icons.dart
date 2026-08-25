// Lumit's icons, as the rest of the app still asks for them: a [LumitIcon]
// enum with a size and a colour.
//
// **What it draws is now Lumit's own set** (docs/15-DESIGN.md §5, K-440):
// every member that has a glyph in `lumit_icons.dart` renders that glyph
// through [glyph.LumitIcon], exactly as a call site of the new set would. The
// enum stays so the twenty-odd call sites need not change while the panels are
// rebuilt one at a time — it is a name for a glyph, no longer a lookup into a
// second icon family.
//
// The members the new set has no word for yet — the deep tools (puppet, roto,
// vertex, camera navigation), the star and solid marks, the fx switch, the
// label tag, the snap magnet, tone map, the node panel's mark — keep the
// Iconoir glyph or the painter-drawn mark they had (K-085), so nothing on
// screen is a glyph that means something else. They are the list of drawings
// the set still owes.

import 'package:flutter/widgets.dart';
import 'package:iconoir_flutter/regular/arc_3d.dart' as ic;
import 'package:iconoir_flutter/regular/copy.dart' as ic;
import 'package:iconoir_flutter/regular/drag.dart' as ic;
import 'package:iconoir_flutter/regular/erase.dart' as ic;
import 'package:iconoir_flutter/regular/expand.dart' as ic;
import 'package:iconoir_flutter/regular/fill_color.dart' as ic;
import 'package:iconoir_flutter/regular/flare.dart' as ic;
import 'package:iconoir_flutter/regular/globe.dart' as ic;
import 'package:iconoir_flutter/regular/hdr.dart' as ic;
import 'package:iconoir_flutter/regular/intersect.dart' as ic;
import 'package:iconoir_flutter/regular/label.dart' as ic;
import 'package:iconoir_flutter/regular/magic_wand.dart' as ic;
import 'package:iconoir_flutter/regular/magnet.dart' as ic;
import 'package:iconoir_flutter/regular/mask_square.dart' as ic;
import 'package:iconoir_flutter/regular/minus_circle.dart' as ic;
import 'package:iconoir_flutter/regular/network.dart' as ic;
import 'package:iconoir_flutter/regular/path_arrow.dart' as ic;
import 'package:iconoir_flutter/regular/pin.dart' as ic;
import 'package:iconoir_flutter/regular/plus_circle.dart' as ic;
import 'package:iconoir_flutter/regular/rotate_camera_right.dart' as ic;
import 'package:iconoir_flutter/regular/snow_flake.dart' as ic;
import 'package:iconoir_flutter/regular/square_dashed.dart' as ic;
import 'package:iconoir_flutter/regular/star.dart' as ic;
import 'package:iconoir_flutter/regular/type.dart' as ic;
import 'package:iconoir_flutter/solid/keyframe.dart' as ics;

// Prefixed: the widget that draws one glyph of the new set is also called
// `LumitIcon`, and the enum below owns that name here.
import 'lumit_icon.dart' as glyph;
import 'lumit_icons.dart';

/// One icon.
///
/// The first 44 variants are the Rust `Icon` enum's, name for name. The tool
/// marks after them (K-216) are this frontend's own: the archived egui shell has
/// no toolbar to draw them, so there is no Rust counterpart to keep in step.
enum LumitIcon {
  pointer,
  move,
  rectangle,
  ellipse,

  /// The star shape tool. **No glyph in the new set yet** — Iconoir's star
  /// stands in, because the set's only many-sided shape is [polygon] and the
  /// two tools sit beside each other.
  star,
  pen,
  play,
  pause,
  lock,
  unlock,
  link,
  unlink,
  folder,
  film,
  graphCurve,
  timelineBars,

  /// The node panel's mark. **No glyph in the new set yet**: the set's graph
  /// glyphs are its actions (auto-wire, heal, frame all), not the panel.
  nodes,
  footage,
  comp,

  /// A sound file in the Project panel — the set's own speaker-with-waves,
  /// which is a *file* mark, not the layer switch [audio] draws.
  audioFile,

  /// The Project panel's bottom-bar controls: bring a file in, and make a
  /// composition. Both carry the plus the mockup gives a "new this" button.
  import,
  newComposition,

  /// A Solid. **No glyph in the new set yet** — its project marks are folder,
  /// composition, footage, still, sequence and audio, and none of those is a
  /// solid colour: `Still` is a photograph in a frame, which would say
  /// "an image file" over a layer that is one flat colour.
  ///
  /// An Adjustment layer used to share this mark; it has its own now
  /// ([adjustment]).
  solid,
  sequence,
  text,
  camera,

  /// The still camera on the Ctrl+Space console's snapshot button (K-324) —
  /// a *photo*, distinct from [camera], which is the camera layer.
  snapshot,
  eye,
  eyeClosed,
  audio,
  mute,
  prevKeyframe,
  nextKeyframe,
  keyframeAdd,
  keyframe,

  /// A filled keyframe diamond. **No glyph in the new set yet, and by design**:
  /// §5 bans a filled variant of an outlined idea, so a selected key wants a
  /// state rather than a second mark. Iconoir's solid diamond stands in.
  keyframeFilled,
  stopwatch,
  twirlClosed,
  twirlOpen,

  /// Collapse transformations / continuously rasterise. **No glyph in the new
  /// set yet** — the set's `Collapse` is the twirl triangle [twirlOpen] draws,
  /// which is a different switch entirely.
  collapse,
  flow,
  cube3d,

  /// The Timeline's snap magnet. **No glyph in the new set yet.**
  magnet,
  eyedropper,
  reset,
  motionBlur,

  /// The effects switch, and the add-effect button. **No glyph in the new set
  /// yet**: `Add effect` is a bare plus, which is right on the button and wrong
  /// on a layer's switch, and one member draws both.
  fx,

  /// The label-colour column's tag (docs/07 §4.2). **No glyph in the new set
  /// yet.**
  label,

  /// Shy: hide-from-the-layer-list, and the master filter that honours it.
  shy,

  /// The shy mark's hidden state — this layer is (or these layers are) hidden
  /// from the list.
  shyHidden,

  /// The solo switch's on state; [ellipse] is its off.
  circleFilled,

  /// A Null layer: an empty square crossed corner to corner, the mark After
  /// Effects puts on a null. **No glyph in the new set yet**, so it stays
  /// painter-drawn, and deliberately unlike [rectangle] and [solid], which are
  /// plain squares.
  nullLayer,

  /// A landscape — two hills under a sky — drawn small at one end of the
  /// Timeline's zoom slider and large at the other, the pair After Effects puts
  /// on its own zoom slider (owner, 2026-08-06). One shape at two sizes says
  /// "less of this / more of this" without needing a word.
  ///
  /// **Painter-drawn on purpose** (docs/15 §5 allows it, and K-209 requires
  /// it): the small end wants to be well under 16px, and a stroked glyph below
  /// 16px puts its 1.5-unit stroke on less than a whole pixel — the crunch a
  /// magnifying glass at 13px showed. A filled shape has no stroke to lose, so
  /// it stays clean at any size the bar has room for.
  zoomExtent,

  // --- The toolbar's tools (K-216, docs/07 §1.7). ---
  zoomIn,

  /// The rotate tool. **No glyph in the new set yet.**
  rotate,

  /// The anchor-point tool: a crosshair in a ring, the same mark the Viewer
  /// draws a layer's origin with.
  anchorPoint,
  razor,

  /// A square with its corners taken off. **No glyph in the new set yet**:
  /// its only square is [rectangle], and the two shape tools have to be told
  /// apart at 16px, which two lookups of the same glyph could not do — so this
  /// one stays painter-drawn.
  roundedRectangle,
  polygon,

  /// The pen group's vertex tools. **No glyphs in the new set yet.**
  vertexAdd,
  vertexDelete,
  vertexConvert,

  /// Mask feather. **No glyph in the new set yet** — the set's `Mask` is the
  /// mask *tool*, not the feather that follows it.
  maskFeather,

  /// Vertical type. **No glyph in the new set yet.**
  textVertical,
  brush,

  /// The paint group's other tools, and the roto pair. **No glyphs in the new
  /// set yet** — the set's `Paint` covers the brush alone.
  cloneStamp,
  eraser,
  rotoBrush,
  refineEdge,

  /// The puppet tools. **No glyphs in the new set yet.**
  puppetPin,
  puppetStarch,
  puppetOverlap,
  puppetBend,

  /// The camera navigation tools. **No glyphs in the new set yet** — the set's
  /// `Camera` is the camera layer, not orbit, pan and dolly.
  cameraOrbit,
  cameraPan,
  cameraDolly,

  /// The Viewer bar's layer-controls switch (K-217): a box with a handle on
  /// each corner — the mark it governs, drawn small. **No glyph in the new set
  /// yet**, so it stays painter-drawn: what it depicts is Lumit's own gizmo.
  wireframe,

  /// The Viewer bar's exposure box (K-314).
  aperture,

  /// The Viewer bar's tone-map switch (K-314). **No glyph in the new set yet**;
  /// Iconoir's HDR mark stands in — what the toggle is about is the values
  /// above 1 that an ordinary display cannot show.
  toneMap,

  /// The Viewer bar's transparency-grid switch (K-411): the checkerboard
  /// itself, at icon size.
  checkerboard,

  /// The Viewer bar's grid-and-guides menu (K-416): a wire lattice. It is
  /// deliberately the mark [checkerboard] declined — an overlay grid is
  /// outlined cells, where the transparency board is filled squares in
  /// alternation. The two sit beside each other on the bar and must not read as
  /// the same switch twice.
  grid,

  /// The Viewer bar's channel picker (K-411): three overlapping circles, the
  /// mark for a picture separated into its channels. Tinted by whichever one
  /// is being shown.
  channels,

  /// A matte: a shape cut out of a square. The channel picker's alpha face —
  /// alpha is not a colour, so it gets a mark rather than a tint.
  matte,

  /// The transport's four steps beside [play] and [pause] (K-466). They had
  /// been the characters `|◀ ◀ ▶ ▶|`, drawn in the body face at whatever size
  /// the font happened to give them; the approved drawing puts a glyph on each
  /// one and the set has carried all four since the icon pass.
  toStart,
  previousFrame,
  nextFrame,
  toEnd,

  /// An Adjustment layer: the set's half-filled circle, the mark for "this
  /// changes what is under it". It had been drawn as [solid], because the set
  /// was read as owing a drawing here — but the drawing was already in the
  /// set, unused, and a solid colour is not what an adjustment layer is.
  adjustment,
}

/// The size an icon draws at (15-DESIGN §5: 16px for panels, 20px for the
/// transport).
///
/// **These are not free numbers.** Every glyph is drawn on a grid with a
/// 1.5-unit stroke, so on the 24-unit Iconoir grid the stroke's width on screen
/// is `size / 24 * 1.5` — at 16 that is exactly one pixel at 100% display
/// scaling, and at 20 it is 1.25. The panel icons had been drawn at 10–13,
/// where the stroke comes to 0.63–0.81 of a pixel: there is no such pixel, so
/// the renderer spreads each line across two of them at partial strength and
/// the whole set reads as smeared and unevenly weighted. That is the "crunch",
/// and no amount of anti-aliasing fixes it — anti-aliasing is what is *doing*
/// it. The cure is drawing at a size whose stroke a pixel can hold. Lumit's own
/// set is drawn on a 16-unit grid instead, which is why this is the default.
///
/// **It is a default, not a law** (K-456, superseding K-209's fixed 16): a
/// panel whose approved mockup computes a smaller glyph renders it at the
/// mockup's size and passes that size here — the Project panel's 13 in a row
/// and 14 on its bottom bar. The stroke softens a little there, and that is
/// the mockups' own look.
const double iconSize = 16;
const double iconSizeTransport = 20;

/// The Iconoir grid its glyphs are drawn on, and how wide their stroke is in
/// those units. Paired: the stroke's width on screen is
/// `size / _iconGridUnits * _iconStrokeUnits`, which is the whole of why the
/// sizes above are what they are.
const double _iconGridUnits = 24;
const double _iconStrokeUnits = 1.5;

/// Build `icon` at `size` in `color`.
///
/// Three ways down, in order: Lumit's own glyph where the set has one, the
/// painter-drawn mark where the icon is Lumit's own artwork, and the Iconoir
/// glyph where the set still owes a drawing.
Widget lumitIcon(LumitIcon icon, {required double size, required Color color}) {
  final own = _ownGlyph(icon);
  if (own != null) {
    return glyph.LumitIcon(own, size: size, colour: color);
  }
  final painter = switch (icon) {
    LumitIcon.nullLayer => _GridIconPainter(color, _drawNullLayer),
    LumitIcon.roundedRectangle =>
      _GridIconPainter(color, _drawRoundedRectangle),
    LumitIcon.wireframe => _GridIconPainter(color, _drawWireframe),
    LumitIcon.zoomExtent => ZoomExtentPainter(color),
    _ => null,
  };
  if (painter != null) {
    return CustomPaint(size: Size.square(size), painter: painter);
  }
  return _CrispGlyph(size: size, child: _iconoirGlyph(icon, color));
}

/// The glyph from Lumit's own set that this icon means, or null where the set
/// has no word for it yet (docs/15 §5's table is the mapping).
String? _ownGlyph(LumitIcon icon) => switch (icon) {
      // Tools.
      LumitIcon.pointer => LumitIcons.select,
      LumitIcon.move => LumitIcons.pan,
      LumitIcon.zoomIn => LumitIcons.zoom,
      LumitIcon.rectangle => LumitIcons.rectangle,
      LumitIcon.ellipse => LumitIcons.ellipse,
      LumitIcon.polygon => LumitIcons.polygon,
      LumitIcon.pen => LumitIcons.pen,
      LumitIcon.text => LumitIcons.text,
      LumitIcon.razor => LumitIcons.razor,
      LumitIcon.anchorPoint => LumitIcons.anchor,
      LumitIcon.camera => LumitIcons.camera,
      LumitIcon.brush => LumitIcons.paint,
      LumitIcon.eyedropper => LumitIcons.eyedropper,
      // Layer switches.
      LumitIcon.eye => LumitIcons.visible,
      LumitIcon.eyeClosed => LumitIcons.hidden,
      LumitIcon.audio => LumitIcons.audio,
      LumitIcon.mute => LumitIcons.muted,
      LumitIcon.circleFilled => LumitIcons.solo,
      LumitIcon.lock => LumitIcons.lock,
      LumitIcon.unlock => LumitIcons.unlocked,
      LumitIcon.shy => LumitIcons.shy,
      LumitIcon.shyHidden => LumitIcons.shyOn,
      LumitIcon.motionBlur => LumitIcons.motionBlur,
      LumitIcon.flow => LumitIcons.flow,
      LumitIcon.cube3d => LumitIcons.threeD,
      LumitIcon.matte => LumitIcons.matte,
      // The twirl: shut points the way it will open, open points down.
      LumitIcon.twirlClosed => LumitIcons.expand,
      LumitIcon.twirlOpen => LumitIcons.collapse,
      // Transport.
      LumitIcon.play => LumitIcons.play,
      LumitIcon.pause => LumitIcons.pause,
      LumitIcon.toStart => LumitIcons.toStart,
      LumitIcon.previousFrame => LumitIcons.previousFrame,
      LumitIcon.nextFrame => LumitIcons.nextFrame,
      LumitIcon.toEnd => LumitIcons.toEnd,
      // Timeline and graph.
      LumitIcon.timelineBars => LumitIcons.layers,
      LumitIcon.graphCurve => LumitIcons.scopes,
      // Keyframes and values.
      LumitIcon.stopwatch => LumitIcons.stopwatch,
      LumitIcon.fx => LumitIcons.effectsSwitch,
      LumitIcon.prevKeyframe => LumitIcons.previousKey,
      LumitIcon.nextKeyframe => LumitIcons.nextKey,
      LumitIcon.keyframeAdd => LumitIcons.addKey,
      LumitIcon.keyframe => LumitIcons.animated,
      LumitIcon.reset => LumitIcons.reset,
      LumitIcon.link => LumitIcons.link,
      LumitIcon.unlink => LumitIcons.unlink,
      // The Viewer bar.
      LumitIcon.checkerboard => LumitIcons.transparency,
      LumitIcon.grid => LumitIcons.grid,
      LumitIcon.channels => LumitIcons.channels,
      LumitIcon.aperture => LumitIcons.exposure,
      LumitIcon.snapshot => LumitIcons.snapshot,
      // The Project panel.
      LumitIcon.folder => LumitIcons.folder,
      LumitIcon.comp => LumitIcons.composition,
      LumitIcon.audioFile => LumitIcons.audioFile,
      LumitIcon.import => LumitIcons.import,
      LumitIcon.newComposition => LumitIcons.newComposition,
      LumitIcon.footage || LumitIcon.film => LumitIcons.footage,
      LumitIcon.sequence => LumitIcons.sequence,
      LumitIcon.adjustment => LumitIcons.adjustment,
      _ => null,
    };

/// A glyph, nudged onto the pixel grid.
///
/// A stroke straddles the line it is drawn along — half its width each side.
/// Iconoir's paths run along whole units of the 24-grid, so a one-pixel stroke
/// lands centred on a pixel *boundary* and comes out as two half-lit pixels
/// instead of one lit one: a grey, doubled line rather than a crisp one. Half
/// a pixel across puts those strokes back on pixel centres, which is the whole
/// difference for every horizontal and vertical line in the set — most of it,
/// since these are interface icons.
///
/// Only when the stroke is an odd number of device pixels: at 2px (a 200%
/// display, or the 20px transport at 150%) the line already covers whole
/// pixels and moving it would be the thing that blurred it. Curves and
/// diagonals are unaffected either way.
///
/// **Iconoir's glyphs only.** Lumit's own set carries the offset in the
/// drawings themselves (its coordinates sit on half units of a 16-unit grid,
/// §5), so nudging those again would take them back off the pixel centres they
/// are already on.
///
/// Best effort at a UI scale other than 1: the scale multiplies in above this
/// widget, so the nudge lands near, rather than exactly on, half a pixel.
class _CrispGlyph extends StatelessWidget {
  final double size;
  final Widget child;

  const _CrispGlyph({required this.size, required this.child});

  @override
  Widget build(BuildContext context) {
    final ratio = MediaQuery.maybeDevicePixelRatioOf(context) ?? 1.0;
    final strokeDevicePixels =
        (size / _iconGridUnits * _iconStrokeUnits * ratio).round();
    final nudge = strokeDevicePixels.isOdd ? 0.5 / ratio : 0.0;
    return SizedBox(
      width: size,
      height: size,
      child: Transform.translate(
        offset: Offset(nudge, nudge),
        child: child,
      ),
    );
  }
}

/// The stand-ins: the icons Lumit's own set has no glyph for yet, still drawn
/// from Iconoir (K-085). Every one of these is a drawing the set owes; none of
/// them is a glyph of the set pressed into a meaning it does not have.
Widget _iconoirGlyph(LumitIcon icon, Color color) => switch (icon) {
      LumitIcon.star => ic.Star(color: color),
      LumitIcon.nodes => ic.Network(color: color),
      LumitIcon.solid => ic.FillColor(color: color),
      LumitIcon.keyframeFilled => ics.KeyframeSolid(color: color),
      LumitIcon.collapse => ic.Flare(color: color),
      LumitIcon.magnet => ic.Magnet(color: color),
      LumitIcon.label => ic.Label(color: color),
      LumitIcon.rotate => ic.RotateCameraRight(color: color),
      LumitIcon.vertexAdd => ic.PlusCircle(color: color),
      LumitIcon.vertexDelete => ic.MinusCircle(color: color),
      LumitIcon.vertexConvert => ic.PathArrow(color: color),
      LumitIcon.maskFeather => ic.SquareDashed(color: color),
      LumitIcon.textVertical => ic.Type(color: color),
      LumitIcon.cloneStamp => ic.Copy(color: color),
      LumitIcon.eraser => ic.Erase(color: color),
      LumitIcon.rotoBrush => ic.MaskSquare(color: color),
      LumitIcon.refineEdge => ic.MagicWand(color: color),
      LumitIcon.puppetPin => ic.Pin(color: color),
      LumitIcon.puppetStarch => ic.SnowFlake(color: color),
      LumitIcon.puppetOverlap => ic.Intersect(color: color),
      LumitIcon.puppetBend => ic.Arc3d(color: color),
      LumitIcon.cameraOrbit => ic.Globe(color: color),
      LumitIcon.cameraPan => ic.Drag(color: color),
      LumitIcon.cameraDolly => ic.Expand(color: color),
      LumitIcon.toneMap => ic.Hdr(color: color),
      // Everything else is drawn above, from Lumit's own set or by a painter.
      _ => const SizedBox.shrink(),
    };

/// The one shell behind every painter-drawn mark. Each mark used to be its
/// own [CustomPainter] class repeating the same shell — hold the colour,
/// scale the 24-unit grid onto the canvas, repaint when the colour changes —
/// around a dozen lines of actual drawing. The shell is kept once here; what
/// to draw comes in as a top-level function, and comparing those tear-offs in
/// [shouldRepaint] is also what repaints a swap of one mark for another.
class _GridIconPainter extends CustomPainter {
  final Color color;

  /// What to draw. `s` is the size on this canvas of one unit of the 24-unit
  /// grid every mark's coordinates are given in.
  final void Function(Canvas canvas, Size size, double s, Color color) draw;

  const _GridIconPainter(this.color, this.draw);

  @override
  void paint(Canvas canvas, Size size) =>
      draw(canvas, size, size.shortestSide / _iconGridUnits, color);

  @override
  bool shouldRepaint(_GridIconPainter old) =>
      old.color != color || old.draw != draw;
}

/// The zoom slider's two ends: a landscape — two hills, the taller one behind —
/// on a 24×24 grid, drawn small at the left end and large at the right.
///
/// **Filled, and drawn rather than looked up.** The pair only says "less / more"
/// if the two are plainly different sizes, and the small one has to be well
/// under 16px for that; a stroked glyph there would put its 1.5-unit stroke on
/// a fraction of a pixel and crunch (docs/15 §5, K-209). A filled silhouette has
/// no stroke to lose, so it reads at 9px as cleanly as at 14.
void _drawZoomExtent(Canvas canvas, Size size, double s, Color color) {
  Offset at(double x, double y) => Offset(x * s, y * s);
  final paint = Paint()
    ..color = color
    ..style = PaintingStyle.fill;
  // The far hill first, so the near one overlaps it and the two read as
  // depth rather than as one jagged shape.
  canvas.drawPath(
    Path()
      ..moveTo(at(9, 20).dx, at(9, 20).dy)
      ..lineTo(at(15, 6).dx, at(15, 6).dy)
      ..lineTo(at(22, 20).dx, at(22, 20).dy)
      ..close(),
    paint,
  );
  canvas.drawPath(
    Path()
      ..moveTo(at(2, 20).dx, at(2, 20).dy)
      ..lineTo(at(8, 11).dx, at(8, 11).dy)
      ..lineTo(at(14, 20).dx, at(14, 20).dy)
      ..close(),
    paint,
  );
}

/// The landscape's named type, kept: the Timeline panel's test tells the
/// slider's two ends apart from every other mark by
/// `painter is ZoomExtentPainter`.
class ZoomExtentPainter extends _GridIconPainter {
  const ZoomExtentPainter(Color color) : super(color, _drawZoomExtent);
}

/// The Null layer's mark, on a 24×24 grid: an empty square crossed corner to
/// corner. A Null has no pixels, so the square stands for the transform box and
/// the cross says there is nothing in it.
void _drawNullLayer(Canvas canvas, Size size, double s, Color color) {
  Offset at(double x, double y) => Offset(x * s, y * s);
  final paint = Paint()
    ..color = color
    ..style = PaintingStyle.stroke
    ..strokeWidth = 2.0 * s
    ..strokeJoin = StrokeJoin.miter
    ..strokeCap = StrokeCap.butt;
  canvas.drawRect(Rect.fromPoints(at(4, 4), at(20, 20)), paint);
  canvas.drawLine(at(4, 4), at(20, 20), paint);
  canvas.drawLine(at(20, 4), at(4, 20), paint);
}

/// The rounded-rectangle shape tool's mark: the same square as
/// [LumitIcon.rectangle] with its corners taken off, so the pair reads as two
/// members of one family at 16px.
void _drawRoundedRectangle(Canvas canvas, Size size, double s, Color color) {
  final paint = Paint()
    ..color = color
    ..style = PaintingStyle.stroke
    ..strokeWidth = _iconStrokeUnits * s
    ..strokeJoin = StrokeJoin.round;
  canvas.drawRRect(
    RRect.fromRectAndRadius(
      Rect.fromLTRB(4 * s, 4 * s, 20 * s, 20 * s),
      // A quarter of the side, not three-eighths: at 6 the corners ate so
      // much of each edge that the mark read as a circle with flats on it
      // rather than as a square with its corners taken off.
      Radius.circular(4 * s),
    ),
    paint,
  );
}

/// The layer-controls switch: a box with a small filled square at each corner —
/// the gizmo the switch shows and hides, on a 24×24 grid.
void _drawWireframe(Canvas canvas, Size size, double s, Color color) {
  Offset at(double x, double y) => Offset(x * s, y * s);
  canvas.drawRect(
    Rect.fromPoints(at(5, 5), at(19, 19)),
    Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = _iconStrokeUnits * s,
  );
  final handle = Paint()..color = color;
  for (final (x, y) in const [
    (5.0, 5.0),
    (19.0, 5.0),
    (19.0, 19.0),
    (5.0, 19.0)
  ]) {
    canvas.drawRect(
      Rect.fromCenter(center: at(x, y), width: 4 * s, height: 4 * s),
      handle,
    );
  }
}
