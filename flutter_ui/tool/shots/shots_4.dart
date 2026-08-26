// Manual screenshots, sweep 4: the tools, and what they make.
//
// toolbar · toolbar-flyout · viewer-bar · text-layer · shape-layer · mask ·
// matte · paint · camera
//
// The comp is staged through the real engine and then worked the way a person
// would — the flyout is right-clicked open, the Type tool is clicked into the
// words on the picture, the Timeline's twirls are pressed. Each shot that adds
// something to the document takes it away again afterwards, so the next one is
// photographing what its caption says and not the leftovers of the last.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1   # PowerShell; LUMIT_SHOTS=1 elsewhere
//   flutter run -d windows -t tool/shots/shots_4.dart
//
// A first pass with `LUMIT_SHOTS_NOCROP=1` and `LUMIT_SHOTS_OUT` set writes
// whole windows somewhere harmless, for checking what the crops are aimed at.

import 'dart:io';
import 'dart:typed_data';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_shapes.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

import 'shots_common.dart';

UuidValue _id() => UuidValue.fromString(const Uuid().v4());

Future<void> main() async {
  final (state, ui) = await bootLumit();
  final project = state.project!;

  final comp = project.newComposition(
    name: 'Opening titles',
    settings: BridgeCompSettings(
      name: 'Opening titles',
      width: 1920,
      height: 1080,
      fpsNum: 25,
      fpsDen: 1,
      duration: BridgeRational(num: 10, den: 1),
      background: F32Array4(Float32List.fromList([0, 0, 0, 1])),
      shutterAngle: 180,
      motionBlurSamples: 16,
    ),
  );

  // Bottom of the stack upwards, the running order of a title sequence.
  comp.addFootageLayer(
    footage: project.importFootage(path: '$fixtures/Music.wav'),
    asSequence: false,
  );
  for (final file in ['Gameplay.mp4', 'Title card.mp4']) {
    comp.addFootageLayer(
      footage: project.importFootage(path: '$fixtures/$file'),
      asSequence: false,
    );
  }
  final title = comp.addTextLayer();
  title.setText(
    document: const BridgeTextDocument(animators: [], pathOffset: BridgeScalar.static_(0), 
      text: 'Northern lights',
      size: 140,
      fill: BridgeColourRgba(r: 1, g: 1, b: 1, a: 1),
    ),
  );
  // The anchor is the middle of the line, so a long line runs off to the right
  // of the starter document's centre unless Position is pulled back.
  title.setTransforms(props: const [
    BridgeTransformProp.positionX,
    BridgeTransformProp.positionY,
  ], values: const [
    BridgeScalar.static_(490),
    BridgeScalar.static_(840),
  ]);

  final layers = comp.getLayers();
  for (final (index, name)
      in ['Title', 'Title card', 'Gameplay', 'Music'].indexed) {
    layers[index].rename(name: name);
  }
  final card = layers[1];
  final gameplay = layers[2];
  card.setTransform(
    prop: BridgeTransformProp.opacity,
    value: const BridgeScalar.static_(55),
  );

  ui.setSelectedComp(comp);
  ui.playheadFrame.value = 48;
  ui.setSelection([title]);

  // The sweeps photograph the shell; the welcome screen has its own sweep.
  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));
  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(6);

  final titleId = title.internallayerId.toString();
  final cardId = card.internallayerId.toString();
  final gameplayId = gameplay.internallayerId.toString();

  /// Where the picture sits on screen — the Viewer's own picture area, with the
  /// composition fitted into the middle of it. Worked out the way the Viewer
  /// works it out, so a click lands where the picture really is.
  ///
  /// The area is measured off the stage rather than taken as the panel less a
  /// 26px bar: under Round the bar is parted from the picture by the tile gap
  /// and sits below it, so the arithmetic that fitted Sharp is short by the gap.
  Rect stage() {
    final area = boxOf('viewer-stage')!;
    final size = comp.getSize();
    final scale = math.min(area.width / size.width, area.height / size.height);
    final drawn = Size(size.width * scale, size.height * scale);
    return Rect.fromLTWH(
      area.left + (area.width - drawn.width) / 2,
      area.top + (area.height - drawn.height) / 2,
      drawn.width,
      drawn.height,
    );
  }

  /// A point of the composition, in window coordinates.
  Offset onPicture(double x, double y) {
    final fitted = stage();
    final size = comp.getSize();
    return Offset(
      fitted.left + x / size.width * fitted.width,
      fitted.top + y / size.height * fitted.height,
    );
  }

  /// Move the splitter between the upper band and the Timeline.
  Future<void> timelineShare(double share) async {
    ui.workspace.dock.shares[0] = 1 - share;
    ui.workspace.dock.shares[1] = share;
    ui.workspace.touch();
    await pause(2);
  }

  /// The Viewer panel, bar and all.
  Rect viewerBox() => boxOfType(ViewerPanelFrb)!;

  // ---- Shot: the toolbar --------------------------------------------------
  // The brush in hand, so the strip shows both halves of what it is: the tools
  // on the left with one of them lit, and the armed tool's own options — the
  // colour it lays down, its size, hardness and opacity — beside them.
  ui.tools.select(ToolMode.brush);
  await pause(1.5);
  final firstTool = boxOf('tool-select')!;
  // Cut after the tool options rather than at the window's edge: the strip runs
  // the full width, and half of it is the empty space between the options and
  // the workspace buttons. A 45:1 picture is scaled down by the page until
  // nothing on it can be read. Clamped to the window all the same — a crop one
  // pixel wider than the picture is a crop ffmpeg refuses outright.
  final windowWidth = shotRootKey.currentContext!.size!.width;
  await captureUi(
    'toolbar.png',
    scale: 3,
    crop: Rect.fromLTRB(
      firstTool.left - 4,
      firstTool.top - 5,
      math.min(boxOf('tool-camera')!.right + 350, windowWidth - 1),
      firstTool.bottom + 5,
    ),
  );

  // ---- Shot: a shape group's flyout ---------------------------------------
  // Right-clicked open, which is one of the two ways it opens (the other is a
  // press and hold). The pointer is back on the strip's own default first, so
  // what is lit beside the flyout is the Selection tool rather than whatever
  // the shot before this one had in hand.
  ui.tools.select(ToolMode.select);
  await pause(1);
  await rightTapKey('tool-shape');
  final shapeButton = boxOf('tool-shape')!;
  final lastMember = boxOf('tool-flyout-shapeStar');
  await captureUi(
    'toolbar-flyout.png',
    scale: 3,
    crop: Rect.fromLTRB(
      shapeButton.left - 120,
      shapeButton.top - 5,
      shapeButton.left + 240,
      (lastMember?.bottom ?? shapeButton.bottom + 200) + 8,
    ),
  );
  // Off the flyout again, and back to the pointer.
  await tapAt(Offset(shapeButton.left - 200, shapeButton.top - 5), settle: 0.8);
  ui.tools.select(ToolMode.select);
  await pause(1);

  // ---- Shot: the Viewer bar -----------------------------------------------
  // The bar's own box, with a margin: under Round it is a strip of its own with
  // a rounded edge and a shadow, and a crop taken from the zoom button's height
  // would shave both off.
  final barBox = boxOf('viewer-bar')!;
  await captureUi(
    'viewer-bar.png',
    scale: 3,
    crop: barBox.inflate(6),
  );

  // ---- Shot: a text layer being edited in the Viewer -----------------------
  // The Type tool clicked into the words already on the picture, which is what
  // puts a caret in them. The toolbar's options change with the tool, so the
  // crop reaches up to the strip: the fill and the point size the edit is
  // using are part of what "being edited" looks like.
  ui.tools.select(ToolMode.typeHorizontal);
  await pause(1.2);
  await tapAt(onPicture(430, 830), settle: 1.6);
  await captureUi(
    'text-layer.png',
    scale: 2,
    crop: Rect.fromLTRB(viewerBox().left, boxOf('tool-select')!.top - 5,
        viewerBox().right, viewerBox().bottom),
  );
  ui.tools.select(ToolMode.select);
  await pause(1.5);

  // ---- Shot: a shape layer's contents in the Timeline ----------------------
  // A badge behind the words: a star with a fill and a stroke, and a bar under
  // it — two pieces of art on one layer, which is what a Contents heading is
  // for.
  final badge = comp.addShapeLayer(name: 'Badge', contents: [
    BridgeShapeItem(
      id: _id(),
      name: 'Star',
      vertices: shapePath(
        tool: ToolMode.shapeStar,
        from: (1380, 180),
        to: (1700, 500),
      ),
      closed: true,
      fill: const BridgeColourRgba(r: 0.98, g: 0.78, b: 0.24, a: 1),
      stroke: const BridgeColourRgba(r: 0.09, g: 0.10, b: 0.13, a: 1),
      strokeWidth: 6,
      opacity: 100,
      trimStart: const BridgeScalar.static_(0),
      trimEnd: const BridgeScalar.static_(100),
      trimOffset: const BridgeScalar.static_(0),
      dashes: const [],
      dashOffset: const BridgeScalar.static_(0),
      gradient: 0,
      gradientColour: null,
      gradientStartX: const BridgeScalar.static_(0),
      gradientStartY: const BridgeScalar.static_(0),
      gradientEndX: const BridgeScalar.static_(0),
      gradientEndY: const BridgeScalar.static_(0),
      combine: 0,
      pathKeys: const [],
      offsetAmount: const BridgeScalar.static_(0),
      repeatCopies: const BridgeScalar.static_(1),
      repeatOffset: const BridgeScalar.static_(0),
      repeatAnchorX: const BridgeScalar.static_(0),
      repeatAnchorY: const BridgeScalar.static_(0),
      repeatPositionX: const BridgeScalar.static_(0),
      repeatPositionY: const BridgeScalar.static_(0),
      repeatRotation: const BridgeScalar.static_(0),
      repeatScale: const BridgeScalar.static_(100),
      repeatStartOpacity: const BridgeScalar.static_(100),
      repeatEndOpacity: const BridgeScalar.static_(100),
    ),
    BridgeShapeItem(
      id: _id(),
      name: 'Underline',
      vertices: shapePath(
        tool: ToolMode.shapeRectangle,
        from: (1380, 540),
        to: (1700, 566),
      ),
      closed: true,
      fill: const BridgeColourRgba(r: 0.98, g: 0.78, b: 0.24, a: 1),
      stroke: null,
      strokeWidth: 0,
      opacity: 100,
      trimStart: const BridgeScalar.static_(0),
      trimEnd: const BridgeScalar.static_(100),
      trimOffset: const BridgeScalar.static_(0),
      dashes: const [],
      dashOffset: const BridgeScalar.static_(0),
      gradient: 0,
      gradientColour: null,
      gradientStartX: const BridgeScalar.static_(0),
      gradientStartY: const BridgeScalar.static_(0),
      gradientEndX: const BridgeScalar.static_(0),
      gradientEndY: const BridgeScalar.static_(0),
      combine: 0,
      pathKeys: const [],
      offsetAmount: const BridgeScalar.static_(0),
      repeatCopies: const BridgeScalar.static_(1),
      repeatOffset: const BridgeScalar.static_(0),
      repeatAnchorX: const BridgeScalar.static_(0),
      repeatAnchorY: const BridgeScalar.static_(0),
      repeatPositionX: const BridgeScalar.static_(0),
      repeatPositionY: const BridgeScalar.static_(0),
      repeatRotation: const BridgeScalar.static_(0),
      repeatScale: const BridgeScalar.static_(100),
      repeatStartOpacity: const BridgeScalar.static_(100),
      repeatEndOpacity: const BridgeScalar.static_(100),
    ),
  ]);
  ui.model.refresh();
  ui.setSelection([badge]);
  await timelineShare(0.42);
  final badgeId = badge.internallayerId.toString();
  await tapKey('tl-twirl-$badgeId');
  await tapKey('tl-twirl-${contentsPath(badgeId)}');
  await pause(1.5);
  await captureUi(
    'shape-layer.png',
    scale: 2,
    crop: Rect.fromLTRB(
      2,
      boxOf('tl-ruler')!.top,
      boxOf('tl-ruler')!.left + 420,
      (boxOf('tl-rowbody-$titleId') ?? boxOf('tl-rowbody-$badgeId')!).bottom,
    ),
  );
  await tapKey('tl-twirl-$badgeId');
  await timelineShare(0.32);

  // ---- Shot: a mask on a layer, gating its alpha ---------------------------
  // An ellipse over the gameplay, softened — the vignette somebody would draw
  // by hand. The layers above it come off so the picture the mask is cutting
  // is the one on screen, and the mask's own layer is selected, because that is
  // what draws the path and its feather over the picture.
  card.setSwitch(switch_: BridgeLayerSwitch.visible, on_: false);
  title.setSwitch(switch_: BridgeLayerSwitch.visible, on_: false);
  badge.setSwitch(switch_: BridgeLayerSwitch.visible, on_: false);
  final maskId = _id();
  gameplay.addMask(
    mask: BridgeMask(
      id: maskId,
      name: 'Ellipse',
      vertices: shapePath(
        tool: ToolMode.shapeEllipse,
        from: (300, 120),
        to: (1620, 960),
      ),
      closed: true,
      inverted: false,
      opacity: const BridgeScalar.static_(100),
      mode: BridgeMaskMode.add,
      feather: const BridgeScalar.static_(90),
      vertexFeather: const [],
      expansion: const BridgeScalar.static_(0),
      pathKeys: const [],
    ),
  );
  ui.model.refresh();
  ui.setSelection([gameplay]);
  await pause(4);
  await captureUi(
    'mask.png',
    scale: 2,
    crop: boxOf('viewer-stage')!,
  );
  gameplay.deleteMask(id: maskId);
  card.setSwitch(switch_: BridgeLayerSwitch.visible, on_: true);
  title.setSwitch(switch_: BridgeLayerSwitch.visible, on_: true);
  ui.model.refresh();
  await pause(2);

  // ---- Shot: a layer matted by the alpha of another ------------------------
  // The gameplay showing through the words: the Title layer's alpha gates it,
  // which is the oldest trick in a title sequence and exactly what the column
  // says. The matte source itself comes off — a matte layer always does, which
  // is why the words come out of the footage rather than in their own white —
  // and so does the card, so what is left on the picture is the matted layer
  // and nothing else.
  card.setSwitch(switch_: BridgeLayerSwitch.visible, on_: false);
  title.setSwitch(switch_: BridgeLayerSwitch.visible, on_: false);
  gameplay.setMatte(
    matte:
        BridgeMatte(layer: title.internallayerId, luma: false, inverted: false),
  );
  ui.model.refresh();
  ui.setSelection([gameplay]);
  await pause(4);
  await captureUi(
    'matte.png',
    scale: 2,
    crop: Rect.fromLTRB(
      2,
      viewerBox().top,
      viewerBox().right,
      boxOf('tl-rowbody-$gameplayId')!.bottom + 8,
    ),
  );
  gameplay.setMatte(matte: null);
  card.setSwitch(switch_: BridgeLayerSwitch.visible, on_: true);
  title.setSwitch(switch_: BridgeLayerSwitch.visible, on_: true);
  ui.model.refresh();
  await pause(2);

  // ---- Shot: brush strokes on a layer --------------------------------------
  // Marks made with the brush, and the Paint heading the Timeline grows for
  // them. Sent as strokes rather than dragged, because a drag over the picture
  // makes exactly this and nothing about the row would differ.
  card.setSwitch(switch_: BridgeLayerSwitch.visible, on_: false);
  title.setSwitch(switch_: BridgeLayerSwitch.visible, on_: false);
  final strokes = [
    (
      'Brush 1',
      const BridgeColourRgba(r: 0.98, g: 0.78, b: 0.24, a: 1),
      [
        (520.0, 700.0),
        (640.0, 620.0),
        (780.0, 660.0),
        (920.0, 560.0),
        (1080.0, 610.0),
        (1240.0, 520.0),
      ]
    ),
    (
      'Brush 2',
      const BridgeColourRgba(r: 0.35, g: 0.72, b: 0.98, a: 1),
      [
        (560.0, 860.0),
        (700.0, 830.0),
        (860.0, 852.0),
        (1020.0, 812.0),
        (1180.0, 840.0),
      ]
    ),
  ];
  for (final (name, colour, points) in strokes) {
    gameplay.addStroke(
      stroke: BridgeStroke(
        id: _id(),
        name: name,
        points: [
          for (final (x, y) in points) BridgeStrokePoint(x: x, y: y, pressure: 1),
        ],
        colour: colour,
        width: 34,
        hardness: 0.8,
        shape: BridgeBrushShape.round,
        opacity: 100,
        start: const BridgeScalar.static_(0),
        end: const BridgeScalar.static_(100),
        mode: BridgePaintMode.paint,
        blend: 0,
        cloneOffsetX: 0,
        cloneOffsetY: 0,
      ),
    );
  }
  ui.model.refresh();
  ui.setSelection([gameplay]);
  ui.tools.select(ToolMode.brush);
  await timelineShare(0.40);
  await tapKey('tl-twirl-$gameplayId');
  await tapKey('tl-twirl-${paintPath(gameplayId)}');
  await pause(4);
  await captureUi(
    'paint.png',
    scale: 2,
    crop: Rect.fromLTRB(
      2,
      viewerBox().top,
      viewerBox().right,
      boxOf('tl-ruler')!.bottom + 240,
    ),
  );
  await tapKey('tl-twirl-$gameplayId');
  for (final stroke in gameplay.getPaint()) {
    gameplay.deleteStroke(id: stroke.id);
  }
  ui.tools.select(ToolMode.select);
  await timelineShare(0.32);
  card.setSwitch(switch_: BridgeLayerSwitch.visible, on_: true);
  title.setSwitch(switch_: BridgeLayerSwitch.visible, on_: true);
  badge.setSwitch(switch_: BridgeLayerSwitch.visible, on_: true);
  ui.model.refresh();
  await pause(2);

  // ---- Shot: a camera layer over 3D layers --------------------------------
  // Three picture layers with their 3D switches on, scaled down and set apart
  // in depth so each reads as its own plane, and a camera looking at them from
  // off to one side. Spreading them is the point: layers left stacked at the
  // same place and the same depth hide one another however the camera is
  // turned, and the shot would say nothing about depth at all.
  //
  // The words come off. A 2D layer is drawn flat over the whole picture, so a
  // line of type across the front of a shot about depth reads as a mistake.
  title.setSwitch(switch_: BridgeLayerSwitch.visible, on_: false);
  card.setTransform(
      prop: BridgeTransformProp.opacity,
      value: const BridgeScalar.static_(100));
  for (final (layer, x, y, z, scale) in [
    (gameplay, 640.0, 620.0, 0.0, 52.0),
    (card, 1240.0, 430.0, -520.0, 52.0),
    (badge, 860.0, 470.0, -950.0, 46.0),
  ]) {
    layer.setSwitch(switch_: BridgeLayerSwitch.threeD, on_: true);
    layer.setTransforms(props: const [
      BridgeTransformProp.positionX,
      BridgeTransformProp.positionY,
      BridgeTransformProp.positionZ,
      BridgeTransformProp.scaleX,
      BridgeTransformProp.scaleY,
    ], values: [
      BridgeScalar.static_(x),
      BridgeScalar.static_(y),
      BridgeScalar.static_(z),
      BridgeScalar.static_(scale),
      BridgeScalar.static_(scale),
    ]);
  }
  final camera = comp.addCameraLayer();
  camera.rename(name: 'Camera');
  camera.setTransforms(props: const [
    BridgeTransformProp.positionX,
    BridgeTransformProp.positionY,
    BridgeTransformProp.rotationY,
    BridgeTransformProp.rotationX,
  ], values: const [
    BridgeScalar.static_(1180),
    BridgeScalar.static_(420),
    BridgeScalar.static_(-16),
    BridgeScalar.static_(6),
  ]);
  ui.model.refresh();
  ui.setSelection([camera]);
  await pause(5);
  await captureUi(
    'camera.png',
    scale: 2,
    crop: Rect.fromLTRB(
      2,
      viewerBox().top,
      viewerBox().right,
      boxOf('tl-rowbody-$cardId')!.bottom + 8,
    ),
  );

  exit(0);
}
