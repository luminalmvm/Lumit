// The toolbar's tools: what they are, which of them is armed, and how a
// keyboard chord picks one (docs/07 §1.7, K-216).
//
// **In plain terms.** A tool is the answer to "what does dragging in the Viewer
// do?" — nudge a layer about, pan the picture, draw a mask, cut a clip. Every
// editor has a strip of them under the menu, and picking one is the whole of
// what this file models: one value, held in one place, that the rest of the app
// reads. Tools that share a job are grouped the way After Effects groups them —
// all five shape tools sit under one button, and the button remembers which of
// the five you last used — so the strip stays short.
//
// Nothing here does any editing. Which tool is armed is a *state*, and the
// panels decide what to make of it; a tool whose behaviour is not built yet is
// still a real, selectable tool that simply changes nothing on the picture yet
// ([ToolMode.ready] says which are which, and the toolbar says so in the
// tooltip rather than hiding the button).

import 'package:flutter/painting.dart' show Color;
import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';

/// A cluster of tools that share one toolbar button, in the order the button's
/// flyout lists them.
///
/// A group with one member is just a button; a group with several is After
/// Effects' hidden-tools flyout — press and hold, or right-click, to see the
/// rest, and the shortcut cycles through them.
enum ToolGroup {
  select,
  hand,
  zoom,
  rotate,
  anchor,
  razor,
  shape,
  pen,
  type,
  paint,
  roto,
  puppet,
  camera,
}

/// One tool.
///
/// [ready] is honest bookkeeping rather than decoration: it says whether
/// choosing this tool changes what a drag does *today*. The toolbar draws the
/// unbuilt ones the same as the rest — they are the specified tool set, not a
/// wish list — and only its tooltip mentions that the behaviour is still to
/// come.
enum ToolMode {
  select(ToolGroup.select, LumitIcon.pointer, ready: true),
  hand(ToolGroup.hand, LumitIcon.move, ready: true),
  zoom(ToolGroup.zoom, LumitIcon.zoomIn, ready: true),
  rotate(ToolGroup.rotate, LumitIcon.rotate, ready: true),
  anchor(ToolGroup.anchor, LumitIcon.anchorPoint, ready: true),
  razor(ToolGroup.razor, LumitIcon.razor, ready: true),

  // The shape tools draw a mask on the selected layer, or a shape layer with
  // nothing selected — AE's rule, and the reason they are one group. Both
  // halves are built: the mask half since K-222, the shape layer since K-237.
  shapeRectangle(ToolGroup.shape, LumitIcon.rectangle, ready: true),
  shapeRoundedRectangle(ToolGroup.shape, LumitIcon.roundedRectangle,
      ready: true),
  shapeEllipse(ToolGroup.shape, LumitIcon.ellipse, ready: true),
  shapePolygon(ToolGroup.shape, LumitIcon.polygon, ready: true),
  shapeStar(ToolGroup.shape, LumitIcon.star, ready: true),

  // The Pen builds a mask path point by point (K-223). Its four siblings edit a
  // *finished* path, which is not built.
  pen(ToolGroup.pen, LumitIcon.pen, ready: true),
  penAddVertex(ToolGroup.pen, LumitIcon.vertexAdd),
  penDeleteVertex(ToolGroup.pen, LumitIcon.vertexDelete),
  penConvertVertex(ToolGroup.pen, LumitIcon.vertexConvert),
  penMaskFeather(ToolGroup.pen, LumitIcon.maskFeather),

  // Making and editing text layers on the picture (K-225). Vertical type would
  // need the engine to lay a line out downwards; it lays out one horizontal
  // line, so that member stays unbuilt.
  typeHorizontal(ToolGroup.type, LumitIcon.text, ready: true),
  typeVertical(ToolGroup.type, LumitIcon.textVertical),

  // Painting on a layer (K-227): the brush lays the fill colour down, the
  // eraser rubs through to transparent, and the clone stamp copies from an
  // Alt-clicked source elsewhere on the same layer.
  brush(ToolGroup.paint, LumitIcon.brush, ready: true),
  cloneStamp(ToolGroup.paint, LumitIcon.cloneStamp, ready: true),
  eraser(ToolGroup.paint, LumitIcon.eraser, ready: true),

  rotoBrush(ToolGroup.roto, LumitIcon.rotoBrush),
  refineEdge(ToolGroup.roto, LumitIcon.refineEdge),

  puppetPosition(ToolGroup.puppet, LumitIcon.puppetPin),
  puppetStarch(ToolGroup.puppet, LumitIcon.puppetStarch),
  puppetOverlap(ToolGroup.puppet, LumitIcon.puppetOverlap),
  puppetBend(ToolGroup.puppet, LumitIcon.puppetBend),

  // Moving the composition's active camera by dragging on the picture
  // (K-229): orbit round what it is looking at, track across, dolly in.
  cameraOrbit(ToolGroup.camera, LumitIcon.cameraOrbit, ready: true),
  cameraPan(ToolGroup.camera, LumitIcon.cameraPan, ready: true),
  cameraDolly(ToolGroup.camera, LumitIcon.cameraDolly, ready: true);

  const ToolMode(this.group, this.icon, {this.ready = false});

  /// The toolbar button this tool lives under.
  final ToolGroup group;

  /// What it is called — in tooltips, in the flyout, and in the status line.
  ///
  /// A getter rather than a constructor argument: an enum constant is built
  /// once, when the program starts, and the interface language can change
  /// after that (K-303).
  String get label => switch (this) {
        ToolMode.select => l10n.toolSelect,
        ToolMode.hand => l10n.toolHand,
        ToolMode.zoom => l10n.toolZoom,
        ToolMode.rotate => l10n.toolRotate,
        ToolMode.anchor => l10n.toolAnchor,
        ToolMode.razor => l10n.toolRazor,
        ToolMode.shapeRectangle => l10n.toolShapeRectangle,
        ToolMode.shapeRoundedRectangle => l10n.toolShapeRoundedRectangle,
        ToolMode.shapeEllipse => l10n.toolShapeEllipse,
        ToolMode.shapePolygon => l10n.toolShapePolygon,
        ToolMode.shapeStar => l10n.toolShapeStar,
        ToolMode.pen => l10n.toolPen,
        ToolMode.penAddVertex => l10n.toolPenAddVertex,
        ToolMode.penDeleteVertex => l10n.toolPenDeleteVertex,
        ToolMode.penConvertVertex => l10n.toolPenConvertVertex,
        ToolMode.penMaskFeather => l10n.toolPenMaskFeather,
        ToolMode.typeHorizontal => l10n.toolTypeHorizontal,
        ToolMode.typeVertical => l10n.toolTypeVertical,
        ToolMode.brush => l10n.toolBrush,
        ToolMode.cloneStamp => l10n.toolCloneStamp,
        ToolMode.eraser => l10n.toolEraser,
        ToolMode.rotoBrush => l10n.toolRotoBrush,
        ToolMode.refineEdge => l10n.toolRefineEdge,
        ToolMode.puppetPosition => l10n.toolPuppetPosition,
        ToolMode.puppetStarch => l10n.toolPuppetStarch,
        ToolMode.puppetOverlap => l10n.toolPuppetOverlap,
        ToolMode.puppetBend => l10n.toolPuppetBend,
        ToolMode.cameraOrbit => l10n.toolCameraOrbit,
        ToolMode.cameraPan => l10n.toolCameraPan,
        ToolMode.cameraDolly => l10n.toolCameraDolly,
      };

  final LumitIcon icon;

  /// Whether arming it changes what a drag does yet.
  final bool ready;

  /// Every tool in [group], in declaration order — which is flyout order.
  static List<ToolMode> membersOf(ToolGroup group) =>
      ToolMode.values.where((m) => m.group == group).toList(growable: false);

  /// The members of [group] that do something today (K-228). Empty for a group
  /// nothing in which is built, which is what makes its button disabled.
  static List<ToolMode> builtMembersOf(ToolGroup group) =>
      membersOf(group).where((m) => m.ready).toList(growable: false);
}

/// The keymap action each group answers to (docs/07 §15, K-199). The engine
/// owns the chords; this only says which group an action arms, so rebinding a
/// tool in Settings → Keymap moves the shortcut and nothing here changes.
const Map<String, ToolGroup> toolActions = {
  'tool.select': ToolGroup.select,
  'tool.hand': ToolGroup.hand,
  'tool.zoom': ToolGroup.zoom,
  'tool.rotate': ToolGroup.rotate,
  'tool.anchor': ToolGroup.anchor,
  'tool.razor': ToolGroup.razor,
  'tool.shape': ToolGroup.shape,
  'tool.pen': ToolGroup.pen,
  'tool.type': ToolGroup.type,
  'tool.paint': ToolGroup.paint,
  'tool.roto': ToolGroup.roto,
  'tool.puppet': ToolGroup.puppet,
  'tool.camera': ToolGroup.camera,
};

/// A colour the toolbar holds, in the document's own scene-linear channels — the
/// same numbers a fill crosses the bridge as, so nothing is converted twice
/// (K-225).
@immutable
class ToolColour {
  final double r;
  final double g;
  final double b;

  const ToolColour(this.r, this.g, this.b);

  static const ToolColour white = ToolColour(1, 1, 1);
  static const ToolColour black = ToolColour(0, 0, 0);

  @override
  bool operator ==(Object other) =>
      other is ToolColour && other.r == r && other.g == g && other.b == b;

  @override
  int get hashCode => Object.hash(r, g, b);
}

/// A tool colour as something a canvas can paint with.
///
/// The document's colours are scene-linear and may sit outside 0..1 (an HDR
/// tint, a lift), which `Color` cannot hold — so they are clamped here, at the
/// one point where a number becomes a pixel on the overlay. This is a *preview*
/// of a colour, not the colour itself: what the engine finally draws is the
/// unclamped value.
Color colourOf(ToolColour c, {double opacity = 1}) => Color.from(
      alpha: opacity,
      red: c.r.clamp(0.0, 1.0),
      green: c.g.clamp(0.0, 1.0),
      blue: c.b.clamp(0.0, 1.0),
    );

/// Which tool is armed, which member each group would arm, and the toolbar's
/// own switches.
///
/// Session state, deliberately: which tool you had in your hand is not part of
/// the project (nothing about the document changes when you pick one) and not
/// part of the workspace either (a layout is where the panels are). It starts
/// on Selection every time, exactly as After Effects does.
class ToolsState extends ChangeNotifier {
  ToolMode _tool = ToolMode.select;

  /// The armed tool.
  ToolMode get tool => _tool;

  /// The member each group last had armed, so a group button keeps showing the
  /// variant you chose rather than snapping back to the first one.
  final Map<ToolGroup, ToolMode> _lastUsed = {};

  /// The **fill** the drawing tools use: the colour new text is set in (K-225),
  /// and the colour a shape layer's fill will take once there are shape layers.
  ToolColour _fill = ToolColour.white;
  ToolColour get fill => _fill;
  set fill(ToolColour value) {
    if (_fill == value) return;
    _fill = value;
    notifyListeners();
  }

  /// The fill as the bridge wants it. Opaque: a fill's transparency is the
  /// layer's Opacity, which is a transform property and animatable, rather than
  /// a fourth number hidden in a swatch.
  BridgeColourRgba get fillRgba =>
      BridgeColourRgba(r: _fill.r, g: _fill.g, b: _fill.b, a: 1);

  /// The point size new text is set at.
  double _textSize = 72;
  double get textSize => _textSize;
  set textSize(double value) {
    final next = value.clamp(1.0, 2000.0);
    if (_textSize == next) return;
    _textSize = next;
    notifyListeners();
  }

  /// The **stroke** a shape layer's art is outlined in, and how wide that
  /// outline is in layer pixels (K-237).
  ///
  /// Live since shape layers landed: a width of zero draws no outline, which is
  /// how a fill-only shape is made.
  ToolColour _stroke = ToolColour.black;
  ToolColour get stroke => _stroke;
  set stroke(ToolColour value) {
    if (_stroke == value) return;
    _stroke = value;
    notifyListeners();
  }

  /// The stroke as the bridge wants it, for a shape layer's outline (K-237).
  BridgeColourRgba get strokeRgba =>
      BridgeColourRgba(r: _stroke.r, g: _stroke.g, b: _stroke.b, a: 1);

  double _strokeWidth = 2;
  double get strokeWidth => _strokeWidth;
  set strokeWidth(double value) {
    final next = value.clamp(0.0, 1000.0);
    if (_strokeWidth == next) return;
    _strokeWidth = next;
    notifyListeners();
  }

  /// The **brush**: how wide a paint stroke is in layer pixels, how hard its
  /// edge is and how opaque the mark it leaves (K-227). Its own settings rather
  /// than the shape tools' stroke, because a brush is a different thing that
  /// happens to have a width — and because these three are live while the
  /// stroke pair is not.
  double _brushSize = 20;
  double get brushSize => _brushSize;
  set brushSize(double value) {
    final next = value.clamp(1.0, 2000.0);
    if (_brushSize == next) return;
    _brushSize = next;
    notifyListeners();
  }

  /// 0 is a brush that fades all the way from its centre, 100 a hard edge.
  double _brushHardness = 80;
  double get brushHardness => _brushHardness;
  set brushHardness(double value) {
    final next = value.clamp(0.0, 100.0);
    if (_brushHardness == next) return;
    _brushHardness = next;
    notifyListeners();
  }

  double _brushOpacity = 100;
  double get brushOpacity => _brushOpacity;
  set brushOpacity(double value) {
    final next = value.clamp(0.0, 100.0);
    if (_brushOpacity == next) return;
    _brushOpacity = next;
    notifyListeners();
  }

  /// Which member of [group] its button currently stands for.
  ///
  /// The first *built* one where there is one, so a group with a working tool
  /// under a not-yet-built first member (the Pen's editing siblings, vertical
  /// type) opens on the one that works.
  ToolMode memberOf(ToolGroup group) =>
      _lastUsed[group] ??
      (ToolMode.builtMembersOf(group).firstOrNull ??
          ToolMode.membersOf(group).first);

  /// Arm [tool], if it is a tool that does anything (K-228).
  ///
  /// A tool whose behaviour is not built cannot be armed — by click, by flyout
  /// or by chord. It stays on the strip, drawn disabled, because the tool set
  /// *is* the specification and a missing button teaches the wrong shape of the
  /// application; but arming one would have handed the user a pointer that does
  /// nothing, which is worse than a button that visibly cannot be pressed.
  void select(ToolMode tool) {
    if (!tool.ready) return;
    _lastUsed[tool.group] = tool;
    if (_tool == tool) return;
    _tool = tool;
    notifyListeners();
  }

  /// Arm [group] the way pressing its button does: the member it last had.
  void selectGroup(ToolGroup group) => select(memberOf(group));

  /// Arm [group] the way pressing its *chord* does.
  ///
  /// The AE rule, and the reason this is not the same as [selectGroup]: the
  /// first press arms the group's remembered member, and pressing again while
  /// that group is already armed steps to the next member and round. So `Q`
  /// walks rectangle → rounded rectangle → ellipse → polygon → star → rectangle
  /// without ever opening the flyout.
  void cycleGroup(ToolGroup group) {
    // Only the built ones are in the walk (K-228): a chord that stepped onto a
    // tool that does nothing would be a chord that appears to do nothing.
    final members = ToolMode.builtMembersOf(group);
    if (members.isEmpty) return;
    if (_tool.group != group || members.length == 1) {
      selectGroup(group);
      return;
    }
    final at = members.indexOf(_tool);
    select(members[at < 0 ? 0 : (at + 1) % members.length]);
  }

  /// Run a keymap action if it is one of the toolbar's, and say whether it was.
  bool handleAction(String action) {
    final group = toolActions[action];
    if (group == null) return false;
    cycleGroup(group);
    return true;
  }
}
