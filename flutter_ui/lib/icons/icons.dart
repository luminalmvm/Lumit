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
// **The set owes nothing now** (K-440's list, closed): the deep tools (puppet,
// roto, vertex, camera navigation), the star and solid marks, the label tag,
// the snap magnet, tone map, the node panel's mark and the filled key were the
// last stand-ins, and each is drawn. Iconoir is gone from the app with them.
// Four marks stay painter-drawn on purpose — the Null layer, the rounded
// rectangle, the Viewer's layer-controls box and the zoom slider's hills — and
// each says below why a glyph would be the worse drawing.

import 'package:flutter/widgets.dart';

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

  /// The star shape tool: five points against [polygon]'s five flat sides.
  /// The two tools sit beside each other, so the count is what tells them
  /// apart.
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

  /// The node panel's mark: two boxes and the wire between them. The set's
  /// other graph glyphs are its *actions* (auto-wire, heal, frame all); this
  /// one is the panel.
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

  /// A Solid: a swatch — a frame with one flat block in it. Deliberately not
  /// `Still`, which is a photograph in a frame and would say "an image file"
  /// over a layer that is one flat colour.
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

  /// A filled keyframe diamond: [keyframe]'s shape, filled. §5 bans a filled
  /// twin of an outlined idea as a *second meaning*; this is one meaning in a
  /// second state, and the Timeline draws the pair side by side.
  keyframeFilled,
  stopwatch,
  twirlClosed,
  twirlOpen,

  /// Collapse transformations / continuously rasterise: two arrows closing on
  /// one line. The set's `Collapse` is the twirl triangle [twirlOpen] draws,
  /// which is a different switch entirely, so this has a glyph of its own.
  collapse,
  flow,
  cube3d,

  /// The Timeline's snap magnet: a horseshoe.
  magnet,
  eyedropper,
  reset,
  motionBlur,

  /// The effects mark: the layer switch, the Effect controls' empty states and
  /// its header. All of them mean "effects", so all of them draw the set's
  /// `Effects switch`. The set's `Add effect` is a bare plus and belongs on a
  /// button that adds one, which is a different call and not this member.
  fx,

  /// The label-colour column's tag (docs/07 §4.2).
  label,

  /// Shy: hide-from-the-layer-list, and the master filter that honours it.
  shy,

  /// The shy mark's hidden state — this layer is (or these layers are) hidden
  /// from the list.
  shyHidden,

  /// The solo switch's on state; [ellipse] is its off.
  circleFilled,

  /// A Null layer: an empty square crossed corner to corner, the mark After
  /// Effects puts on a null.
  ///
  /// **Painter-drawn on purpose**: it is drawn with a 2-unit mitred stroke and
  /// butt caps, which is not the set's weight and not its joins — the corners
  /// have to come to a point for the square to read as a transform box rather
  /// than as [rectangle] with a cross in it. A glyph of the set could not do
  /// that without breaking the one grammar §5 keeps.
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

  /// The rotate tool: a box with a turn over it. Deliberately not a bare
  /// circular arrow, which is what [reset] draws.
  rotate,

  /// The anchor-point tool: a crosshair in a ring, the same mark the Viewer
  /// draws a layer's origin with.
  anchorPoint,
  razor,

  /// A square with its corners taken off.
  ///
  /// **Painter-drawn on purpose**: the corner radius is a quarter of the side
  /// *at the size it is asked for*, so the pair with [rectangle] stays legible
  /// as two shape tools at the 13 the Project panel uses and the 16 the toolbar
  /// does. A fixed radius baked into a 16-unit path rounds away to nothing at
  /// the small end and the two tools become one mark.
  roundedRectangle,
  polygon,

  /// The pen group's vertex tools: one curve with a vertex on it, and then a
  /// plus, a minus, or the pair of tangent handles the convert puts there.
  vertexAdd,
  vertexDelete,
  vertexConvert,

  /// Mask feather: a hard edge inside a soft one. The set's `Mask` is the mask
  /// *tool* — a frame with a shape in it — and feather is about the edge alone,
  /// so it drops the frame.
  maskFeather,

  /// Vertical type: [text]'s T on its side, crossbar to the left.
  textVertical,
  brush,

  /// The paint group's other tools, and the roto pair. The set's `Paint` is
  /// the brush alone; these four are a stamp, an eraser, the brush over a
  /// dashed selection, and an edge with hair coming off it.
  cloneStamp,
  eraser,
  rotoBrush,
  refineEdge,

  /// The puppet tools: a pin, a braced region, one shape passing in front of
  /// another, and a line bent off the dashed one it would have followed.
  puppetPin,
  puppetStarch,
  puppetOverlap,
  puppetBend,

  /// The camera navigation tools: a body with a ring round it, the four ways
  /// in the plane, and a view cone with an arrow down its axis. The set's
  /// `Camera` is the camera *layer*, which is none of those.
  cameraOrbit,
  cameraPan,
  cameraDolly,

  /// The Viewer bar's layer-controls switch (K-217): a box with a handle on
  /// each corner — the mark it governs, drawn small.
  ///
  /// **Painter-drawn on purpose**: the handles are filled squares whose size is
  /// fixed against the box, not against the stroke, so the switch keeps looking
  /// like the gizmo it turns on at every size the bar renders it. In a stroked
  /// glyph the handles and the box would share one weight and the mark would
  /// read as a grid.
  wireframe,

  /// The Viewer bar's exposure box (K-314).
  aperture,

  /// The Viewer bar's tone-map switch (K-314): the transfer curve against the
  /// dashed line of doing nothing. The gap between them is the whole of what
  /// the switch is for — the values above 1 an ordinary display cannot show.
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
/// **These are not free numbers.** A glyph is drawn on a grid with a 1.5-unit
/// stroke, so on a 24-unit grid the stroke's width on screen is
/// `size / 24 * 1.5` — at 16 that is exactly one pixel at 100% display
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

/// The grid the painter-drawn marks below are laid out on, and how wide a
/// stroke is in those units. Paired: the stroke's width on screen is
/// `size / _iconGridUnits * _iconStrokeUnits`, which is the whole of why the
/// sizes above are what they are. Twenty-four rather than the set's sixteen
/// because these four coordinate systems predate the set and there is nothing
/// to gain from restating them.
const double _iconGridUnits = 24;
const double _iconStrokeUnits = 1.5;

/// Build `icon` at `size` in `color`.
///
/// Two ways down: Lumit's own glyph, which is every member but four, and the
/// painter-drawn mark for the four that are Lumit's own artwork and say above
/// why a glyph would be the worse drawing.
Widget lumitIcon(LumitIcon icon, {required double size, required Color color}) {
  final own = _ownGlyph(icon);
  if (own != null) {
    return glyph.LumitIcon(own, size: size, colour: color);
  }
  return CustomPaint(
    size: Size.square(size),
    painter: switch (icon) {
      LumitIcon.roundedRectangle =>
        _GridIconPainter(color, _drawRoundedRectangle),
      LumitIcon.wireframe => _GridIconPainter(color, _drawWireframe),
      LumitIcon.zoomExtent => ZoomExtentPainter(color),
      // The Null layer, and the one place an unmapped member can land: a
      // member added without a glyph draws the mark that says "nothing here",
      // which is wrong but visible, rather than an empty box that is not.
      _ => _GridIconPainter(color, _drawNullLayer),
    },
  );
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
      LumitIcon.solid => LumitIcons.solid,
      LumitIcon.nodes => LumitIcons.nodes,
      // The toolbar's remaining tools.
      LumitIcon.star => LumitIcons.star,
      LumitIcon.rotate => LumitIcons.rotate,
      LumitIcon.textVertical => LumitIcons.verticalType,
      LumitIcon.vertexAdd => LumitIcons.vertexAdd,
      LumitIcon.vertexDelete => LumitIcons.vertexDelete,
      LumitIcon.vertexConvert => LumitIcons.vertexConvert,
      LumitIcon.maskFeather => LumitIcons.maskFeather,
      LumitIcon.cloneStamp => LumitIcons.cloneStamp,
      LumitIcon.eraser => LumitIcons.eraser,
      LumitIcon.rotoBrush => LumitIcons.rotoBrush,
      LumitIcon.refineEdge => LumitIcons.refineEdge,
      LumitIcon.puppetPin => LumitIcons.puppetPin,
      LumitIcon.puppetStarch => LumitIcons.puppetStarch,
      LumitIcon.puppetOverlap => LumitIcons.puppetOverlap,
      LumitIcon.puppetBend => LumitIcons.puppetBend,
      LumitIcon.cameraOrbit => LumitIcons.cameraOrbit,
      LumitIcon.cameraPan => LumitIcons.cameraPan,
      LumitIcon.cameraDolly => LumitIcons.cameraDolly,
      // Switches and columns.
      LumitIcon.collapse => LumitIcons.collapseTransformations,
      LumitIcon.label => LumitIcons.label,
      LumitIcon.magnet => LumitIcons.snap,
      LumitIcon.keyframeFilled => LumitIcons.selectedKey,
      LumitIcon.toneMap => LumitIcons.toneMap,
      _ => null,
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
