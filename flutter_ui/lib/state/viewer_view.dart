// What the View menu, the keyboard and the command palette ask the Viewer for
// (docs/07-UI-SPEC.md §2.2, §15).
//
// In plain terms: the magnification and the preview resolution are two
// different things that both sound like "zoom", and this file is where the two
// words are kept apart.
//
// **Magnification** is how big the picture is drawn in the panel. It changes
// nothing about what the engine renders — it is display scaling, and the
// arithmetic behind it lives in `panels/viewer_zoom.dart`. The three commands
// here are the named jumps a menu row or a chord can ask for; the Viewer panel
// holds the actual magnification, so the shell *asks* rather than reaching into
// a panel that may not even be mounted.
//
// **Preview resolution** is how many pixels the engine is asked to make. Half
// renders a quarter of them, so a heavy composition previews in a quarter of
// the time and looks correspondingly coarser. It is a real raster reduction,
// not a display trick, and it MUST never reach the export (glossary §5).

import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart' show BridgeCameraView;

/// Which way the Viewer looks at a 3D composition (docs/impl/camera.md §6).
///
/// **Active camera** is the composition's own: what an export sees, and the
/// only view that is not a pose of the panel's own. The six fixed ones look at
/// the comp centre from far enough away to be orthographic to the pixel, and
/// the three custom ones start from a three-quarter view and keep whatever the
/// camera tools do to them until the comp is closed.
enum ViewerView {
  activeCamera,
  front,
  back,
  left,
  right,
  top,
  bottom,
  custom1,
  custom2,
  custom3;

  /// The view the engine builds this one's starting pose from, or null for the
  /// active camera, which has no pose of its own.
  BridgeCameraView? get fixed => switch (this) {
        ViewerView.activeCamera => null,
        ViewerView.front => BridgeCameraView.front,
        ViewerView.back => BridgeCameraView.back,
        ViewerView.left => BridgeCameraView.left,
        ViewerView.right => BridgeCameraView.right,
        ViewerView.top => BridgeCameraView.top,
        ViewerView.bottom => BridgeCameraView.bottom,
        ViewerView.custom1 ||
        ViewerView.custom2 ||
        ViewerView.custom3 =>
          BridgeCameraView.custom,
      };

  /// Whether the camera tools move this view. Only the custom three: a Front
  /// view that could be dragged off square would not be a front view.
  bool get movable =>
      this == ViewerView.custom1 ||
      this == ViewerView.custom2 ||
      this == ViewerView.custom3;

  /// What the picker's row reads.
  String get title => switch (this) {
        ViewerView.activeCamera => l10n.viewerViewActiveCamera,
        ViewerView.front => l10n.viewerViewFront,
        ViewerView.back => l10n.viewerViewBack,
        ViewerView.left => l10n.viewerViewLeft,
        ViewerView.right => l10n.viewerViewRight,
        ViewerView.top => l10n.viewerViewTop,
        ViewerView.bottom => l10n.viewerViewBottom,
        ViewerView.custom1 => l10n.viewerViewCustom1,
        ViewerView.custom2 => l10n.viewerViewCustom2,
        ViewerView.custom3 => l10n.viewerViewCustom3,
      };
}

/// Which channel a view shows (docs/07 §2.2 item 3): the three colours on
/// their own, or the alpha as the greyscale matte it is.
///
/// Per view rather than per composition, like the magnification beside it: two
/// views exist so they can be looked at differently, and reading the alpha in
/// one while the other shows the picture is exactly what a second view is for.
enum ViewerChannel { rgb, red, green, blue, alpha }

/// A named magnification the Viewer can be asked to take.
///
/// Not a number: "fit" is a *rule* (the whole picture in the panel) that has to
/// be re-resolved every time the panel is resized, and a step in or out means
/// "from wherever you are now", which only the Viewer knows.
enum ViewerZoomCommand {
  zoomIn,
  zoomOut,
  fit;

  /// The keymap action id this command answers (docs/07 §15).
  String get action => switch (this) {
        ViewerZoomCommand.zoomIn => 'viewer.zoom.in',
        ViewerZoomCommand.zoomOut => 'viewer.zoom.out',
        ViewerZoomCommand.fit => 'viewer.zoom.fit',
      };

  /// What the View menu's row reads.
  String get title => switch (this) {
        ViewerZoomCommand.zoomIn => l10n.menuZoomIn,
        ViewerZoomCommand.zoomOut => l10n.menuZoomOut,
        ViewerZoomCommand.fit => l10n.menuFit,
      };
}

/// The fraction of composition resolution a preview frame is rendered at
/// (docs/07 §2.2 item 2, §15).
///
/// **Auto is not "Full by another name", and the difference is the point.**
/// Auto renders only the pixels the current magnification can actually
/// display — a Viewer in a small panel decodes and composites small — while
/// Full means composition resolution whatever the panel is showing, which is
/// what you want when judging detail at 100 %. Earlier the tier called "Full"
/// was silently Auto, and there was no way to ask for the real thing.
///
/// **Full is the default**: what the picture is made of should not depend on
/// how wide the panel happens to be, and a soft first look at a shot is soft
/// for a reason the user cannot see. Auto is one dropdown away.
enum PreviewResolution {
  auto,
  full,
  half,
  third,
  quarter;

  /// The fraction of comp resolution this tier asks for, or null for Auto,
  /// which takes whatever the panel implies instead.
  double? get fraction => switch (this) {
        PreviewResolution.auto => null,
        PreviewResolution.full => 1.0,
        PreviewResolution.half => 0.5,
        PreviewResolution.third => 1.0 / 3.0,
        PreviewResolution.quarter => 0.25,
      };

  /// The scale a render request carries, given what the panel can show.
  ///
  /// A fixed tier is a real raster reduction and is taken as asked — Half
  /// renders a quarter of the pixels — so that what you see is the tier you
  /// chose rather than whichever of the two happened to be smaller.
  double scaleFor(double panelScale) => fraction ?? panelScale;

  /// The keymap action id this resolution answers, or null for the tiers
  /// with no chord of their own (docs/07 §15 names three).
  String? get action => switch (this) {
        PreviewResolution.full => 'viewer.res.full',
        PreviewResolution.half => 'viewer.res.half',
        PreviewResolution.quarter => 'viewer.res.quarter',
        _ => null,
      };

  /// What the View ▸ Resolution row and the bar dropdown read.
  String get title => switch (this) {
        PreviewResolution.auto => l10n.menuAuto,
        PreviewResolution.full => l10n.menuFull,
        PreviewResolution.half => l10n.menuHalf,
        PreviewResolution.third => l10n.resolutionThird,
        PreviewResolution.quarter => l10n.menuQuarter,
      };
}
