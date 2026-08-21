// Lumit's icons: the Iconoir set (MIT), ported one-for-one from
// crates/lumit-ui/src/icons.rs (K-085). Same rules as the Rust side: every
// glyph is a real icon from one consistent set, emoji are banned, icons take
// the text colour of their state, and the motion-blur mark is drawn from the
// owner's artwork rather than looked up (Iconoir has no motion-blur glyph).

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:iconoir_flutter/regular/align_left.dart' as ic;
import 'package:iconoir_flutter/regular/arc_3d.dart' as ic;
import 'package:iconoir_flutter/regular/circle.dart' as ic;
import 'package:iconoir_flutter/regular/color_filter.dart' as ic;
import 'package:iconoir_flutter/regular/color_picker.dart' as ic;
import 'package:iconoir_flutter/regular/copy.dart' as ic;
import 'package:iconoir_flutter/regular/cube.dart' as ic;
import 'package:iconoir_flutter/regular/cursor_pointer.dart' as ic;
import 'package:iconoir_flutter/regular/design_nib.dart' as ic;
import 'package:iconoir_flutter/regular/design_pencil.dart' as ic;
import 'package:iconoir_flutter/regular/drag.dart' as ic;
import 'package:iconoir_flutter/regular/drag_hand_gesture.dart' as ic;
import 'package:iconoir_flutter/regular/ease_curve_control_points.dart' as ic;
import 'package:iconoir_flutter/regular/erase.dart' as ic;
import 'package:iconoir_flutter/regular/expand.dart' as ic;
import 'package:iconoir_flutter/regular/eye.dart' as ic;
import 'package:iconoir_flutter/regular/eye_closed.dart' as ic;
import 'package:iconoir_flutter/regular/fill_color.dart' as ic;
import 'package:iconoir_flutter/regular/flare.dart' as ic;
import 'package:iconoir_flutter/regular/folder.dart' as ic;
import 'package:iconoir_flutter/regular/frame.dart' as ic;
import 'package:iconoir_flutter/regular/fx.dart' as ic;
import 'package:iconoir_flutter/regular/globe.dart' as ic;
import 'package:iconoir_flutter/regular/hdr.dart' as ic;
import 'package:iconoir_flutter/regular/intersect.dart' as ic;
import 'package:iconoir_flutter/regular/keyframe.dart' as ic;
import 'package:iconoir_flutter/regular/keyframe_plus.dart' as ic;
import 'package:iconoir_flutter/regular/label.dart' as ic;
import 'package:iconoir_flutter/regular/link.dart' as ic;
import 'package:iconoir_flutter/regular/link_xmark.dart' as ic;
import 'package:iconoir_flutter/regular/lock.dart' as ic;
import 'package:iconoir_flutter/regular/lock_slash.dart' as ic;
import 'package:iconoir_flutter/regular/magic_wand.dart' as ic;
import 'package:iconoir_flutter/regular/magnet.dart' as ic;
import 'package:iconoir_flutter/regular/mask_square.dart' as ic;
import 'package:iconoir_flutter/regular/media_video.dart' as ic;
import 'package:iconoir_flutter/regular/minus_circle.dart' as ic;
import 'package:iconoir_flutter/regular/movie.dart' as ic;
import 'package:iconoir_flutter/regular/nav_arrow_down.dart' as ic;
import 'package:iconoir_flutter/regular/nav_arrow_left.dart' as ic;
import 'package:iconoir_flutter/regular/nav_arrow_right.dart' as ic;
import 'package:iconoir_flutter/regular/network.dart' as ic;
import 'package:iconoir_flutter/regular/path_arrow.dart' as ic;
import 'package:iconoir_flutter/regular/pause.dart' as ic;
import 'package:iconoir_flutter/regular/pentagon.dart' as ic;
import 'package:iconoir_flutter/regular/pin.dart' as ic;
import 'package:iconoir_flutter/regular/play.dart' as ic;
import 'package:iconoir_flutter/regular/plus_circle.dart' as ic;
import 'package:iconoir_flutter/regular/refresh_double.dart' as ic;
import 'package:iconoir_flutter/regular/rotate_camera_right.dart' as ic;
import 'package:iconoir_flutter/regular/scissor.dart' as ic;
import 'package:iconoir_flutter/regular/snow_flake.dart' as ic;
import 'package:iconoir_flutter/regular/sound_high.dart' as ic;
import 'package:iconoir_flutter/regular/sound_off.dart' as ic;
import 'package:iconoir_flutter/regular/square.dart' as ic;
import 'package:iconoir_flutter/regular/square_dashed.dart' as ic;
import 'package:iconoir_flutter/regular/star.dart' as ic;
import 'package:iconoir_flutter/regular/text.dart' as ic;
import 'package:iconoir_flutter/regular/timer.dart' as ic;
import 'package:iconoir_flutter/regular/type.dart' as ic;
import 'package:iconoir_flutter/regular/camera.dart' as ic;
import 'package:iconoir_flutter/regular/video_camera.dart' as ic;
import 'package:iconoir_flutter/regular/view_columns_3.dart' as ic;
import 'package:iconoir_flutter/regular/view_grid.dart' as ic;
import 'package:iconoir_flutter/regular/wind.dart' as ic;
import 'package:iconoir_flutter/regular/zoom_in.dart' as ic;
import 'package:iconoir_flutter/solid/keyframe.dart' as ics;

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
  nodes,
  footage,
  comp,
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
  keyframeFilled,
  stopwatch,
  twirlClosed,
  twirlOpen,
  collapse,
  flow,
  cube3d,
  magnet,
  eyedropper,
  reset,
  motionBlur,
  fx,

  /// The label-colour column's tag (docs/07 §4.2).
  label,

  /// Shy: hide-from-the-layer-list, and the master filter that honours it.
  /// Drawn, not looked up (Iconoir has no peek): lines standing above the
  /// list's baseline.
  shy,

  /// The shy mark's hidden state: the lines ducked down to a stub over the
  /// baseline — this layer is (or these layers are) hidden from the list.
  shyHidden,

  /// A filled circle — the solo switch's on state; [ellipse] is its off.
  circleFilled,

  /// A Null layer: an empty square crossed corner to corner, the mark After
  /// Effects puts on a null. Drawn, not looked up (Iconoir has no crosshair),
  /// and deliberately unlike [rectangle] and [solid], which are plain squares.
  nullLayer,

  /// A landscape — two hills under a sky — drawn small at one end of the
  /// Timeline's zoom slider and large at the other, the pair After Effects puts
  /// on its own zoom slider (owner, 2026-08-06). One shape at two sizes says
  /// "less of this / more of this" without needing a word.
  ///
  /// **Painter-drawn on purpose** (docs/15 §5 allows it, and K-209 requires
  /// it): the small end wants to be well under 16px, and an Iconoir glyph below
  /// 16px puts its 1.5-unit stroke on less than a whole pixel — the crunch a
  /// magnifying glass at 13px showed. A filled shape has no stroke to lose, so
  /// it stays clean at any size the bar has room for.
  zoomExtent,

  // --- The toolbar's tools (K-216, docs/07 §1.7). ---
  zoomIn,
  rotate,

  /// The anchor-point tool: a crosshair in a ring, the same mark the Viewer
  /// draws a layer's origin with. Painter-drawn — Iconoir has no crosshair, and
  /// the tool's whole job is that one mark.
  anchorPoint,
  razor,

  /// A square with its corners taken off. Painter-drawn: Iconoir's own square
  /// is [rectangle], and the two shape tools have to be told apart at 16px,
  /// which two lookups of the same glyph could not do.
  roundedRectangle,
  polygon,
  vertexAdd,
  vertexDelete,
  vertexConvert,
  maskFeather,
  textVertical,
  brush,
  cloneStamp,
  eraser,
  rotoBrush,
  refineEdge,
  puppetPin,
  puppetStarch,
  puppetOverlap,
  puppetBend,
  cameraOrbit,
  cameraPan,
  cameraDolly,

  /// The Viewer bar's layer-controls switch (K-217): a box with a handle on
  /// each corner — the mark it governs, drawn small. Painter-drawn, because
  /// what it depicts is Lumit's own gizmo rather than anything a general icon
  /// set has a glyph for.
  wireframe,

  /// The Viewer bar's exposure box (K-314): a camera iris. Painter-drawn —
  /// Iconoir has no aperture glyph, and exposure in stops is a camera idea, so
  /// the mark is the camera's.
  aperture,

  /// The Viewer bar's tone-map switch (K-314). Iconoir's HDR mark: what the
  /// toggle is about is the values above 1 that an ordinary display cannot
  /// show.
  toneMap,

  /// The Viewer bar's transparency-grid switch (K-411): the checkerboard
  /// itself, at icon size. Painter-drawn — Iconoir's grids are all wire
  /// lattices, and what this toggle draws is filled squares in alternation,
  /// which is the only thing that reads as "nothing is here".
  checkerboard,

  /// The Viewer bar's grid-and-guides menu (K-416): a wire lattice. It is
  /// deliberately the mark [checkerboard] declined — Iconoir's grids are drawn
  /// as outlined cells, which is exactly what an overlay grid *is*, where the
  /// transparency board is filled squares in alternation. The two sit beside
  /// each other on the bar and must not read as the same switch twice.
  grid,

  /// The Viewer bar's channel picker (K-411): three overlapping circles, the
  /// mark for a picture separated into its channels. Tinted by whichever one
  /// is being shown.
  channels,

  /// A matte: a shape cut out of a square. The channel picker's alpha face —
  /// alpha is not a colour, so it gets a mark rather than a tint. The same
  /// Iconoir glyph as [rotoBrush], named here for what it means on the bar
  /// (as [twirlClosed] and [nextKeyframe] already share an arrow).
  matte,
}

/// The size an icon draws at (15-DESIGN §5: 16px for panels, 20px for the
/// transport).
///
/// **These are not free numbers.** Every Iconoir glyph is drawn on a 24-unit
/// grid with a 1.5-unit stroke, so the stroke's width on screen is
/// `size / 24 * 1.5` — at 16 that is exactly one pixel at 100% display
/// scaling, and at 20 it is 1.25. The panel icons had been drawn at 10–13,
/// where the stroke comes to 0.63–0.81 of a pixel: there is no such pixel, so
/// the renderer spreads each line across two of them at partial strength and
/// the whole set reads as smeared and unevenly weighted. That is the "crunch",
/// and no amount of anti-aliasing fixes it — anti-aliasing is what is *doing*
/// it. The cure is drawing at a size whose stroke a pixel can hold.
const double iconSize = 16;
const double iconSizeTransport = 20;

/// The Iconoir grid every glyph is drawn on, and how wide its stroke is in
/// those units. Paired: the stroke's width on screen is
/// `size / _iconGridUnits * _iconStrokeUnits`, which is the whole of why the
/// sizes above are what they are.
const double _iconGridUnits = 24;
const double _iconStrokeUnits = 1.5;

/// Build `icon` at `size` in `color`. The motion-blur mark is drawn, not
/// looked up, exactly as in the Rust frontend.
Widget lumitIcon(LumitIcon icon, {required double size, required Color color}) {
  final painter = switch (icon) {
    LumitIcon.motionBlur => _GridIconPainter(color, _drawMotionBlur),
    LumitIcon.shy => _GridIconPainter(color, _drawShy),
    LumitIcon.shyHidden => _GridIconPainter(color, _drawShyHidden),
    LumitIcon.circleFilled => _GridIconPainter(color, _drawCircleFill),
    LumitIcon.nullLayer => _GridIconPainter(color, _drawNullLayer),
    LumitIcon.anchorPoint => _GridIconPainter(color, _drawAnchorPoint),
    LumitIcon.roundedRectangle =>
      _GridIconPainter(color, _drawRoundedRectangle),
    LumitIcon.wireframe => _GridIconPainter(color, _drawWireframe),
    LumitIcon.zoomExtent => ZoomExtentPainter(color),
    LumitIcon.aperture => _GridIconPainter(color, _drawAperture),
    LumitIcon.checkerboard => _GridIconPainter(color, _drawCheckerboard),
    _ => null,
  };
  if (painter != null) {
    return CustomPaint(size: Size.square(size), painter: painter);
  }
  return _CrispGlyph(size: size, child: _glyph(icon, color));
}

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

Widget _glyph(LumitIcon icon, Color color) => switch (icon) {
      LumitIcon.pointer => ic.CursorPointer(color: color),
      LumitIcon.move => ic.DragHandGesture(color: color),
      LumitIcon.rectangle => ic.Square(color: color),
      LumitIcon.ellipse => ic.Circle(color: color),
      LumitIcon.star => ic.Star(color: color),
      LumitIcon.pen => ic.DesignNib(color: color),
      LumitIcon.play => ic.Play(color: color),
      LumitIcon.pause => ic.Pause(color: color),
      LumitIcon.lock => ic.Lock(color: color),
      LumitIcon.unlock => ic.LockSlash(color: color),
      LumitIcon.link => ic.Link(color: color),
      LumitIcon.unlink => ic.LinkXmark(color: color),
      LumitIcon.folder => ic.Folder(color: color),
      LumitIcon.film => ic.Movie(color: color),
      LumitIcon.graphCurve => ic.EaseCurveControlPoints(color: color),
      LumitIcon.timelineBars => ic.AlignLeft(color: color),
      LumitIcon.nodes => ic.Network(color: color),
      LumitIcon.footage => ic.MediaVideo(color: color),
      LumitIcon.comp => ic.Frame(color: color),
      LumitIcon.solid => ic.FillColor(color: color),
      LumitIcon.sequence => ic.ViewColumns3(color: color),
      LumitIcon.text => ic.Text(color: color),
      LumitIcon.camera => ic.VideoCamera(color: color),
      LumitIcon.snapshot => ic.Camera(color: color),
      LumitIcon.eye => ic.Eye(color: color),
      LumitIcon.eyeClosed => ic.EyeClosed(color: color),
      LumitIcon.audio => ic.SoundHigh(color: color),
      LumitIcon.mute => ic.SoundOff(color: color),
      LumitIcon.prevKeyframe => ic.NavArrowLeft(color: color),
      LumitIcon.nextKeyframe => ic.NavArrowRight(color: color),
      LumitIcon.keyframeAdd => ic.KeyframePlus(color: color),
      LumitIcon.keyframe => ic.Keyframe(color: color),
      LumitIcon.keyframeFilled => ics.KeyframeSolid(color: color),
      LumitIcon.stopwatch => ic.Timer(color: color),
      LumitIcon.twirlClosed => ic.NavArrowRight(color: color),
      LumitIcon.twirlOpen => ic.NavArrowDown(color: color),
      LumitIcon.collapse => ic.Flare(color: color),
      LumitIcon.flow => ic.Wind(color: color),
      LumitIcon.cube3d => ic.Cube(color: color),
      LumitIcon.magnet => ic.Magnet(color: color),
      LumitIcon.eyedropper => ic.ColorPicker(color: color),
      LumitIcon.reset => ic.RefreshDouble(color: color),
      LumitIcon.motionBlur => const SizedBox.shrink(), // handled above
      LumitIcon.fx => ic.Fx(color: color),
      LumitIcon.label => ic.Label(color: color),
      // The toolbar's tools. Where Iconoir has no mark for a tool nobody but an
      // editor would name — a razor, a puppet pin — the nearest honest glyph
      // from the same family is used rather than a second family being brought
      // in for one icon (15-DESIGN §5: one set, no exceptions).
      LumitIcon.zoomIn => ic.ZoomIn(color: color),
      LumitIcon.rotate => ic.RotateCameraRight(color: color),
      LumitIcon.razor => ic.Scissor(color: color),
      LumitIcon.polygon => ic.Pentagon(color: color),
      LumitIcon.vertexAdd => ic.PlusCircle(color: color),
      LumitIcon.vertexDelete => ic.MinusCircle(color: color),
      LumitIcon.vertexConvert => ic.PathArrow(color: color),
      LumitIcon.maskFeather => ic.SquareDashed(color: color),
      LumitIcon.textVertical => ic.Type(color: color),
      LumitIcon.brush => ic.DesignPencil(color: color),
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
      LumitIcon.grid => ic.ViewGrid(color: color),
      LumitIcon.channels => ic.ColorFilter(color: color),
      LumitIcon.matte => ic.MaskSquare(color: color),
      // Painter-drawn, handled above.
      LumitIcon.checkerboard ||
      LumitIcon.aperture ||
      LumitIcon.shy ||
      LumitIcon.shyHidden ||
      LumitIcon.circleFilled ||
      LumitIcon.nullLayer ||
      LumitIcon.anchorPoint ||
      LumitIcon.roundedRectangle ||
      LumitIcon.wireframe ||
      LumitIcon.zoomExtent =>
        const SizedBox.shrink(),
    };

/// The one shell behind every painter-drawn mark. Each mark used to be its
/// own [CustomPainter] class repeating the same shell — hold the colour,
/// scale the 24-unit grid onto the canvas, repaint when the colour changes —
/// around a dozen lines of actual drawing. The shell is kept once here; what
/// to draw comes in as a top-level function, and comparing those tear-offs in
/// [shouldRepaint] is also what repaints a swap of one mark for another (the
/// shy mark's two states are two functions rather than a flag).
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

/// The exposure box's mark (K-314): a camera iris, on the same 24×24 grid and
/// at the same 1.5-unit stroke as every Iconoir glyph, so it sits in the bar at
/// the weight of the icons either side of it.
///
/// Six blades, drawn as six chords of the ring at 60° apart. Each chord runs
/// between two points on the circle a third of the way round from each other,
/// which is what gives the iris its hexagonal opening without any of the lines
/// meeting at the centre.
void _drawAperture(Canvas canvas, Size size, double s, Color color) {
  final centre = Offset(size.width / 2, size.height / 2);
  final radius = 9.0 * s;
  final paint = Paint()
    ..color = color
    ..style = PaintingStyle.stroke
    ..strokeWidth = _iconStrokeUnits * s
    ..strokeCap = StrokeCap.round;
  canvas.drawCircle(centre, radius, paint);
  Offset on(double turns) =>
      centre +
      Offset(math.cos(turns * 2 * math.pi), math.sin(turns * 2 * math.pi)) *
          radius;
  for (var blade = 0; blade < 6; blade++) {
    canvas.drawLine(on(blade / 6), on((blade + 2) / 6), paint);
  }
}

/// The transparency-grid switch's mark (K-411): the checkerboard the toggle
/// puts behind transparent pixels, on the same 24-unit grid.
///
/// A 4×4 board of 4-unit squares inside a 1.5-unit outline, filled in
/// alternation. Painter-drawn rather than looked up: Iconoir's grid marks are
/// lattices of lines, and a lattice does not read as "nothing is here" — the
/// alternation is the whole meaning, and it needs filled cells to carry it.
void _drawCheckerboard(Canvas canvas, Size size, double s, Color color) {
  Offset at(double x, double y) => Offset(x * s, y * s);
  final fill = Paint()..color = color;
  for (var row = 0; row < 4; row++) {
    for (var col = 0; col < 4; col++) {
      if ((row + col).isEven) continue;
      canvas.drawRect(
        Rect.fromPoints(
          at(4 + col * 4, 4 + row * 4),
          at(8 + col * 4, 8 + row * 4),
        ),
        fill,
      );
    }
  }
  canvas.drawRect(
    Rect.fromPoints(at(4, 4), at(20, 20)),
    Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = _iconStrokeUnits * s
      ..strokeJoin = StrokeJoin.miter,
  );
}

/// The motion-blur mark: a ring with speed streaks running into it, from the
/// owner's artwork on a 24×24 grid — coordinates identical to the Rust
/// `draw_motion_blur` so the two frontends paint the same mark.
void _drawMotionBlur(Canvas canvas, Size size, double s, Color color) {
  final origin = Offset(size.width / 2 - 12.0 * s, size.height / 2 - 12.0 * s);
  Offset at(double x, double y) => origin + Offset(x * s, y * s);
  final paint = Paint()
    ..color = color
    ..style = PaintingStyle.stroke
    ..strokeWidth = 2.0 * s
    ..strokeCap = StrokeCap.butt;
  // The ring: a 2-unit stroke on a 4-unit radius, centred at (17, 12).
  canvas.drawCircle(at(17, 12), 4.0 * s, paint);
  // The streaks; two rows broken by a shorter dash further left, which is
  // what makes the mark read as motion rather than a plain arrow.
  const rows = [
    (4.0, 14.0, 8.0),
    (10.0, 13.0, 12.0),
    (8.0, 14.0, 16.0),
    (3.0, 7.0, 12.0),
    (4.0, 5.0, 16.0),
  ];
  for (final (x1, x2, y) in rows) {
    canvas.drawLine(at(x1, y), at(x2, y), paint);
  }
}

/// The shy mark, on a 24×24 grid — two draw functions for its two states,
/// which is also how the shell knows to repaint when one swaps for the other.
///
/// Not hidden: two lines standing over the list's long baseline.
void _drawShy(Canvas canvas, Size size, double s, Color color) =>
    _drawShyMark(canvas, s, color, hidden: false);

/// Hidden: just a stub ducked close over the baseline — the layers have
/// dropped out of the list.
void _drawShyHidden(Canvas canvas, Size size, double s, Color color) =>
    _drawShyMark(canvas, s, color, hidden: true);

void _drawShyMark(Canvas canvas, double s, Color color,
    {required bool hidden}) {
  Offset at(double x, double y) => Offset(x * s, y * s);
  final paint = Paint()
    ..color = color
    ..strokeWidth = 2.0 * s
    ..strokeCap = StrokeCap.round;
  // The baseline: the layer list itself.
  canvas.drawLine(at(4, 19), at(20, 19), paint);
  if (hidden) {
    canvas.drawLine(at(9, 13), at(15, 13), paint);
  } else {
    canvas.drawLine(at(6, 12), at(18, 12), paint);
    canvas.drawLine(at(9, 5), at(15, 5), paint);
  }
}

/// A filled circle: the solo switch's on state. The only mark that ignores the
/// grid scale — its radius is a fraction of the widget, not a unit count.
void _drawCircleFill(Canvas canvas, Size size, double s, Color color) {
  canvas.drawCircle(
    size.center(Offset.zero),
    size.shortestSide * 0.32,
    Paint()..color = color,
  );
}

/// The zoom slider's two ends: a landscape — two hills, the taller one behind —
/// on the same 24×24 grid, drawn small at the left end and large at the right.
///
/// **Filled, and drawn rather than looked up.** The pair only says "less / more"
/// if the two are plainly different sizes, and the small one has to be well
/// under 16px for that; an Iconoir glyph there would put its 1.5-unit stroke on
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

/// The Null layer's mark, on the same 24×24 grid as the other drawn marks: an
/// empty square crossed corner to corner. A Null has no pixels, so the square
/// stands for the transform box and the cross says there is nothing in it.
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

/// The anchor-point tool's mark, on the same 24×24 grid: a ring with a cross
/// through it — the origin crosshair the Viewer draws on the selected layer,
/// which is exactly what the tool moves.
void _drawAnchorPoint(Canvas canvas, Size size, double s, Color color) {
  Offset at(double x, double y) => Offset(x * s, y * s);
  final paint = Paint()
    ..color = color
    ..style = PaintingStyle.stroke
    ..strokeWidth = _iconStrokeUnits * s
    ..strokeCap = StrokeCap.butt;
  canvas.drawCircle(at(12, 12), 5.0 * s, paint);
  // The arms reach past the ring, so the centre reads as a point being aimed
  // at rather than a circle with a plus in it.
  canvas.drawLine(at(12, 3), at(12, 21), paint);
  canvas.drawLine(at(3, 12), at(21, 12), paint);
}

/// The rounded-rectangle shape tool's mark: the same square as [LumitIcon.rectangle]
/// with its corners taken off, so the pair reads as two members of one family
/// at 16px.
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
/// the gizmo the switch shows and hides, on the same 24×24 grid.
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
