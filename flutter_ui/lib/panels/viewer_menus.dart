// The Viewer's right-click menus: the rows they are made of, and the picture's
// own menu.
//
// A row here calls the command the menu bar or the keymap already calls, and a
// picker on a bar opens these same rows, so the two can't say different things.

import 'dart:async';
import 'dart:io' show Directory, File, Platform, Process, ProcessResult;
import 'dart:typed_data';
import 'dart:ui' as dartui;

import 'package:flutter/services.dart' show Clipboard, ClipboardData;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/lib.dart' show F32Array4;
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart' show UuidValue;

import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/comp_settings_frb.dart' show showCompSettingsFrb;
import '../shell/menu_bar_frb.dart' show menuEntriesFlyout;
import '../shell/number_dialog_frb.dart' show askNumberFrb;
import '../state/file_dialogs.dart' show pickFrameSaveLocation;
import '../state/settings.dart';
import '../state/timecode.dart';
import '../state/viewer_view.dart';
import '../state/viewer_views.dart';
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart' show addMarkerFrb, showMenuAt, workAreaFrames;
import 'timeline_layer_menu_frb.dart' show layerMenuEntries;
import 'timeline_outline_row_frb.dart' show layerRowRename;
import 'viewer_bar.dart' show durationFramesOf;
import 'viewer_panel_frb.dart' show showViewerBackgroundPicker;
import 'viewer_rulers.dart' show viewerGuideComp, viewerRulerBand;
import 'viewer_strips.dart' show playbackSlows, viewerZoomSteps;

/// What only the panel knows, for the menus opened from its bars and picture.
/// One per view, kept up to date as the panel builds.
class ViewerMenuHost {
  late CompositionReference comp;
  late int viewId;
  double? zoom;
  double shownScale = 1;
  late ValueChanged<double?> onZoom;
  ViewerChannel channel = ViewerChannel.rgb;
  late ValueChanged<ViewerChannel> onChannel;
  F32Array4? background;
  Rect picture = Rect.zero;
  Size compSize = Size.zero;

  bool hasSnapshot = false;
  bool snapshotShown = false;
  late VoidCallback takeSnapshot;
  late VoidCallback clearSnapshot;
  late ValueChanged<bool> showSnapshot;

  /// Opens the picture's menu from wherever the right click landed. Set by the
  /// layer controls while they're up, because they know what's under the
  /// pointer. Every tool leaves a right click to them.
  void Function(TapUpDetails)? openPicture;
}

/// One row of a Viewer menu. Null [pick] draws it disabled. [ticked] is the tick
/// column, which every row in a Viewer menu keeps so the names share one edge.
Widget viewerRow(
  String text,
  VoidCallback? pick,
  VoidCallback close, {
  String? id,
  String? action,
  bool ticked = false,
}) =>
    MenuRow(
      key: id == null ? null : ValueKey<String>(id),
      enabled: pick != null,
      onPressed: () {
        close();
        pick?.call();
      },
      child: MenuLabel(text,
          action: action, ticked: ticked, enabled: pick != null),
    );

/// A row that opens a flyout, its name on the same edge as its neighbours.
Widget viewerSub(
  String text,
  VoidCallback close,
  List<Widget> Function(VoidCallback dismiss) rows, {
  String? id,
}) =>
    SubmenuRow(
      key: id == null ? null : ValueKey<String>(id),
      closeParent: close,
      submenu: (dismiss) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: rows(dismiss),
          ),
        ),
      ),
      child: MenuLabel(text, ticked: false),
    );

/// A right click on [child] opens [rows] at the pointer.
class ViewerRightClick extends StatelessWidget {
  final Widget child;
  final List<Widget> Function(VoidCallback close) rows;

  const ViewerRightClick({super.key, required this.rows, required this.child});

  @override
  Widget build(BuildContext context) => GestureDetector(
        behavior: HitTestBehavior.translucent,
        onSecondaryTapUp: (d) => showMenuAt<void>(
          context: context,
          position: d.globalPosition,
          rows: (close) => rows(() => close(null)),
        ),
        child: child,
      );
}

// --- Rows that more than one menu shares -------------------------------------

/// The magnification, as the header's picker and the View menu offer it.
List<Widget> viewerZoomRows(
  LumitUiState ui,
  ViewerMenuHost host,
  VoidCallback close,
) =>
    [
      viewerRow(l10n.menuFit, () => host.onZoom(null), close,
          id: 'viewer-menu-fit',
          action: ViewerZoomCommand.fit.action,
          ticked: host.zoom == null),
      viewerRow(l10n.menuZoomIn,
          () => ui.requestViewerZoom(ViewerZoomCommand.zoomIn), close,
          action: ViewerZoomCommand.zoomIn.action),
      viewerRow(l10n.menuZoomOut,
          () => ui.requestViewerZoom(ViewerZoomCommand.zoomOut), close,
          action: ViewerZoomCommand.zoomOut.action),
      const MenuDivider(),
      for (final step in viewerZoomSteps)
        if (step != null)
          viewerRow('${(step * 100).round()}%', () => host.onZoom(step), close,
              id: step == 1 ? 'viewer-menu-actual' : null,
              ticked: host.zoom == step),
    ];

/// The preview resolution, ticked. [viewId] for a view with its own, as a 3D
/// viewport has.
List<Widget> viewerResolutionRows(
  LumitUiState ui,
  VoidCallback close, {
  int? viewId,
}) =>
    [
      for (final resolution in PreviewResolution.values)
        viewerRow(resolution.title,
            () => ui.setPreviewResolution(resolution, viewId: viewId), close,
            action: resolution.action,
            ticked: resolution ==
                ui.previewResolutionOf(viewId ?? ui.activeViewId)),
    ];

String viewerChannelName(ViewerChannel c) => switch (c) {
      ViewerChannel.rgb => 'RGB',
      ViewerChannel.red => engineLabel('Red'),
      ViewerChannel.green => engineLabel('Green'),
      ViewerChannel.blue => engineLabel('Blue'),
      ViewerChannel.alpha => l10n.channelAlpha,
    };

/// The channel on view, ticked.
List<Widget> viewerChannelRows(ViewerMenuHost host, VoidCallback close) => [
      for (final c in ViewerChannel.values)
        viewerRow(viewerChannelName(c), () => host.onChannel(c), close,
            id: 'viewer-channel-${c.name}', ticked: c == host.channel),
    ];

/// Taking a snapshot, showing it, and letting it go.
List<Widget> viewerSnapshotRows(ViewerMenuHost host, VoidCallback close) => [
      viewerRow(l10n.tipViewerSnapshotTake, host.takeSnapshot, close,
          id: 'viewer-menu-snapshot-take'),
      viewerRow(
          l10n.viewerMenuSnapshotShow,
          host.hasSnapshot ? () => host.showSnapshot(!host.snapshotShown) : null,
          close,
          id: 'viewer-menu-snapshot-show',
          ticked: host.snapshotShown),
      viewerRow(l10n.viewerMenuSnapshotClear,
          host.hasSnapshot ? host.clearSnapshot : null, close,
          id: 'viewer-menu-snapshot-clear'),
    ];

/// How the views in this panel are laid out, and the two put together.
List<Widget> viewerLayoutRows(LumitUiState ui, VoidCallback close) => [
      viewerRow(l10n.menuOneView, () => ui.setViewerLayout(ViewLayout.one),
          close,
          action: 'viewer.layout.one',
          ticked: ui.currentViewerLayout == ViewLayout.one),
      viewerRow(l10n.menuTwoViewsAcross,
          () => ui.setViewerLayout(ViewLayout.twoAcross), close,
          action: 'viewer.layout.two',
          ticked: ui.currentViewerLayout == ViewLayout.twoAcross),
      viewerRow(l10n.menuTwoViewsDown,
          () => ui.setViewerLayout(ViewLayout.twoDown), close,
          ticked: ui.currentViewerLayout == ViewLayout.twoDown),
      viewerRow(
          l10n.menuFourViews, () => ui.setViewerLayout(ViewLayout.four), close,
          action: 'viewer.layout.four',
          ticked: ui.currentViewerLayout == ViewLayout.four),
    ];

List<Widget> viewerCompareRows(LumitUiState ui, VoidCallback close) => [
      viewerRow(l10n.menuCompareOff, () => ui.setCompareMode(CompareMode.none),
          close,
          ticked: ui.currentCompare == CompareMode.none),
      viewerRow(l10n.menuCompareWipe, () => ui.setCompareMode(CompareMode.wipe),
          close,
          action: 'viewer.compare',
          ticked: ui.currentCompare == CompareMode.wipe),
      viewerRow(
          l10n.menuCompareSplit, () => ui.setCompareMode(CompareMode.split), close,
          ticked: ui.currentCompare == CompareMode.split),
    ];

/// What playback does at the end of the work area, ticked.
List<Widget> viewerLoopRows(LumitUiState ui, VoidCallback close) => [
      for (final (mode, text) in [
        (LoopMode.workArea, l10n.tipTransportLoopWorkArea),
        (LoopMode.pingPong, l10n.tipTransportLoopPingPong),
        (LoopMode.once, l10n.tipTransportLoopOnce),
      ])
        viewerRow(text, () => ui.workspace.setLoopMode(mode), close,
            id: 'viewer-loop-${mode.name}',
            ticked: ui.workspace.performance.loop == mode),
    ];

/// How fast the Viewer plays, ticked.
List<Widget> viewerSpeedRows(LumitUiState ui, VoidCallback close) => [
      for (final slow in playbackSlows)
        viewerRow(l10n.viewerSpeedTimes(1 / slow),
            () => ui.setPlaybackSlow(slow), close,
            ticked: ui.playbackSlow == slow),
    ];

/// An empty stretch of the Viewer's bars: how they're arranged.
List<Widget> viewerBarRows(LumitUiState ui, VoidCallback close) => [
      for (final (bars, text) in [
        (ViewerBars.auto, l10n.styleChoice),
        (ViewerBars.split, l10n.viewerBarsSplit),
        (ViewerBars.top, l10n.viewerBarsTop),
        (ViewerBars.bottom, l10n.viewerBarsBottom),
        (ViewerBars.deck, l10n.viewerBarsDeck),
      ])
        viewerRow(text, () {
          ui.workspace.interface.viewerBars = bars;
          ui.workspace.settingsChanged();
        }, close,
            id: 'viewer-bars-${bars.name}',
            ticked: ui.workspace.interface.viewerBars == bars),
    ];

/// The transport's menu: the moves its marks make, the places they don't reach,
/// and what playback does.
List<Widget> viewerTransportRows(
  LumitUiState ui,
  BridgeCompSettings settings,
  CompositionReference comp,
  ValueChanged<int> onSeek,
  VoidCallback close,
) {
  final playing = ui.playing.value;
  final last = durationFramesOf(settings) - 1;
  final frame = ui.playheadFrame.value;
  int workEdge(bool start) {
    try {
      final work = workAreaFrames(comp);
      return start ? work.start : work.end - 1;
    } catch (_) {
      return start ? 0 : last;
    }
  }

  return [
    viewerRow(playing ? l10n.tipTransportPause : l10n.tipTransportPlay,
        () => playing ? ui.stopPlayback() : ui.play(), close,
        id: 'viewer-menu-play', action: 'playback.toggle'),
    const MenuDivider(),
    viewerRow(l10n.tipTransportStart, () => onSeek(0), close,
        action: 'playback.comp.start'),
    viewerRow(l10n.viewerMenuWorkAreaStart, () => onSeek(workEdge(true)), close,
        action: 'playback.workarea.start'),
    viewerRow(l10n.tipTransportPrevious, () => onSeek(frame - 1), close,
        action: 'playback.frame.prev'),
    viewerRow(l10n.tipTransportNext, () => onSeek(frame + 1), close,
        action: 'playback.frame.next'),
    viewerRow(l10n.viewerMenuWorkAreaEnd, () => onSeek(workEdge(false)), close,
        action: 'playback.workarea.end'),
    viewerRow(l10n.tipTransportEnd, () => onSeek(last), close,
        action: 'playback.comp.end'),
    const MenuDivider(),
    viewerSub(l10n.viewerMenuLoop, close,
        (dismiss) => viewerLoopRows(ui, dismiss)),
    viewerSub(l10n.viewerSpeed, close,
        (dismiss) => viewerSpeedRows(ui, dismiss)),
    viewerRow(l10n.tipTransportMute, () => ui.setAudioMuted(!ui.audioMuted.value),
        close,
        ticked: ui.audioMuted.value),
  ];
}

/// The clock's menu: the time as words, and the transport's own rows.
List<Widget> viewerClockRows(
  LumitUiState ui,
  BridgeCompSettings settings,
  CompositionReference comp,
  ValueChanged<int> onSeek,
  VoidCallback close,
) {
  final frame = ui.playheadFrame.value;
  return [
    viewerRow(
        l10n.viewerMenuCopyTime,
        () => Clipboard.setData(ClipboardData(
            text: timecodeOfRate(frame, settings.fpsNum, settings.fpsDen))),
        close,
        id: 'viewer-menu-copy-time'),
    viewerRow(
        l10n.viewerMenuCopyFrameNumber,
        () => Clipboard.setData(ClipboardData(text: '$frame')),
        close),
    const MenuDivider(),
    ...viewerTransportRows(ui, settings, comp, onSeek, close),
  ];
}

// --- The frame as a file -----------------------------------------------------

/// The composition's frame at the playhead, at its own full size, as the export
/// would make it. Null where there's no graphics adapter to draw it.
Future<dartui.Image?> _frameImage(
    CompositionReference comp, int frame, Size size) async {
  final rendered = await comp.thumbnail(
    frame: frame,
    maxEdge: size.longestSide.round(),
  );
  if (rendered == null) return null;
  final done = Completer<dartui.Image>();
  dartui.decodeImageFromPixels(
    rendered.rgba,
    rendered.width,
    rendered.height,
    dartui.PixelFormat.rgba8888,
    done.complete,
  );
  return done.future;
}

/// The name a saved frame is offered: the composition and the frame's time,
/// with nothing a file name can't hold.
String _frameFileName(String comp, String time) =>
    '$comp $time'.replaceAll(RegExp(r'[<>:"/\\|?*]'), '-');

/// Put the frame at the playhead on the system clipboard as a picture.
///
/// Flutter's clipboard only carries text. Each system ships a small tool that
/// takes a picture, so the frame goes through a PNG in the temp folder, and the
/// build needs nothing of its own for it.
Future<void> copyViewerFrame(
  BuildContext context,
  LumitUiState ui,
  ViewerMenuHost host,
) async {
  final state = Provider.of<LumitState>(context, listen: false);
  Directory? folder;
  var copied = false;
  try {
    final image =
        await _frameImage(host.comp, ui.playheadFrame.value, host.compSize);
    if (image != null) {
      final png = await image.toByteData(format: dartui.ImageByteFormat.png);
      image.dispose();
      folder = await Directory.systemTemp.createTemp('lumit-frame-');
      final file = File('${folder.path}${Platform.pathSeparator}frame.png');
      await file.writeAsBytes(Uint8List.sublistView(png!));
      copied = await _pictureToClipboard(file.path);
    }
  } catch (_) {
    // No tool, or nothing to draw: the notice below says it didn't happen.
  }
  try {
    await folder?.delete(recursive: true);
  } catch (_) {
    // Left in the temp folder, which the system clears.
  }
  state.postNotice(copied ? l10n.viewerFrameCopied : l10n.viewerFrameNotCopied);
}

/// The PNG at [path] on the system clipboard. False if the tool wasn't there or
/// refused it.
Future<bool> _pictureToClipboard(String path) async {
  final ProcessResult done;
  if (Platform.isWindows) {
    final quoted = path.replaceAll("'", "''");
    done = await Process.run('powershell', [
      '-NoProfile',
      '-NonInteractive',
      '-Sta',
      '-Command',
      'Add-Type -AssemblyName System.Windows.Forms,System.Drawing; '
          "\$i = [System.Drawing.Image]::FromFile('$quoted'); "
          '[System.Windows.Forms.Clipboard]::SetImage(\$i); \$i.Dispose()',
    ]);
  } else if (Platform.isMacOS) {
    done = await Process.run('osascript', [
      '-e',
      'on run argv',
      '-e',
      'set the clipboard to (read (POSIX file (item 1 of argv)) as «class PNGf»)',
      '-e',
      'end run',
      path,
    ]);
  } else {
    // These two stay running to serve the picture, so their pipes are sent
    // elsewhere or the wait here would never end.
    final tool = Platform.environment.containsKey('WAYLAND_DISPLAY')
        ? 'wl-copy --type image/png < "\$1"'
        : 'xclip -selection clipboard -t image/png -i "\$1"';
    done = await Process.run(
        'sh', ['-c', '$tool >/dev/null 2>&1', 'sh', path]);
  }
  return done.exitCode == 0;
}

/// Save the frame at the playhead as a PNG.
Future<void> saveViewerFrame(
  BuildContext context,
  LumitUiState ui,
  ViewerMenuHost host,
) async {
  final state = Provider.of<LumitState>(context, listen: false);
  final frame = ui.playheadFrame.value;
  final settings = host.comp.getSettings();
  final name = _frameFileName(settings.name,
      timecodeOfRate(frame, settings.fpsNum, settings.fpsDen));
  final path = await pickFrameSaveLocation(name);
  if (path == null) return;
  try {
    final image = await _frameImage(host.comp, frame, host.compSize);
    if (image == null) throw StateError('no picture');
    final png = await image.toByteData(format: dartui.ImageByteFormat.png);
    image.dispose();
    await File(path).writeAsBytes(Uint8List.sublistView(png!));
    state.postNotice(l10n.viewerFrameSaved);
  } catch (_) {
    state.postNotice(l10n.viewerFrameNotSaved);
  }
}

// --- The picture's menu ------------------------------------------------------

/// The marks over the picture and what's behind it, ticked.
List<Widget> viewerShowRows(
  LumitUiState ui,
  ViewerMenuHost host,
  VoidCallback close,
) {
  final on = ui.viewerOverlays;
  final aids = ui.viewerAidsOf(host.viewId);
  final threeD = ui.viewPoseOf(host.viewId) != null ||
      ui.model.heldLayers.any((e) =>
          e.info.switches.threeD ||
          e.info.kind == BridgeLayerKind.camera ||
          e.info.kind == BridgeLayerKind.light);
  return [
    viewerRow(l10n.tipTransparencyGrid, () => ui.setViewerGrid(!ui.viewerGrid),
        close,
        id: 'viewer-menu-checkerboard', ticked: ui.viewerGrid),
    viewerRow(l10n.viewerOverlayGrid,
        () => ui.setViewerOverlays(grid: !on.grid), close,
        id: 'viewer-guides-grid', ticked: on.grid),
    viewerRow(l10n.viewerOverlaySafeAreas,
        () => ui.setViewerOverlays(safeAreas: !on.safeAreas), close,
        id: 'viewer-guides-safe', ticked: on.safeAreas),
    viewerRow(l10n.viewerOverlayRulers,
        () => ui.setViewerOverlays(rulers: !on.rulers), close,
        id: 'viewer-guides-rulers',
        action: 'viewer.rulers.toggle',
        ticked: on.rulers),
    viewerRow(l10n.viewerOverlayLayerControls,
        () => ui.setViewerLayerControls(!ui.viewerLayerControls), close,
        id: 'viewer-wireframes',
        action: 'viewer.controls.toggle',
        ticked: ui.viewerLayerControls),
    if (ui.model.isNodeGraph)
      viewerRow(l10n.viewerOverlayReach,
          () => ui.setViewerOverlays(reach: !on.reach), close,
          id: 'viewer-guides-reach', ticked: on.reach),
    if (threeD) ...[
      viewerRow(l10n.viewerOverlayGround,
          () => ui.setViewerAids(host.viewId, ground: !aids.ground), close,
          id: 'viewer-guides-ground', ticked: aids.ground),
      viewerRow(l10n.viewerOverlayAxes,
          () => ui.setViewerAids(host.viewId, axes: !aids.axes), close,
          id: 'viewer-guides-axes', ticked: aids.axes),
    ],
  ];
}

/// The guides: adding one where the pointer is, and what the set does.
/// [stageAt] is the pointer in the stage, or null for the bar's own menu, which
/// is nowhere on the picture to add one at.
List<Widget> viewerGuideRows(
  LumitUiState ui,
  ViewerMenuHost host,
  VoidCallback close, {
  Offset? stageAt,
}) {
  void add(bool vertical, double screen) => ui.setGuides([
        ...ui.guides,
        (
          at: viewerGuideComp(screen,
              vertical: vertical,
              picture: host.picture,
              compSize: host.compSize),
          vertical: vertical,
        ),
      ]);
  final at = stageAt;
  return [
    if (at != null) ...[
      viewerRow(l10n.viewerMenuAddVerticalGuide, () => add(true, at.dx), close,
          id: 'viewer-menu-guide-vertical'),
      viewerRow(
          l10n.viewerMenuAddHorizontalGuide, () => add(false, at.dy), close,
          id: 'viewer-menu-guide-horizontal'),
    ],
    viewerRow(l10n.viewerOverlaySnap,
        () => ui.tools.snapping = !ui.tools.snapping, close,
        id: 'viewer-guides-snap', ticked: ui.tools.snapping),
    viewerRow(l10n.menuSnapToGrid,
        () => ui.tools.snapToGrid = !ui.tools.snapToGrid, close,
        action: 'viewer.snap.grid.toggle', ticked: ui.tools.snapToGrid),
    viewerRow(l10n.viewerMenuLockGuides, () {
      final on = ui.workspace.interface;
      on.viewerGuidesLocked = !on.viewerGuidesLocked;
      ui.workspace.settingsChanged();
    }, close,
        id: 'viewer-menu-guides-lock',
        ticked: ui.workspace.interface.viewerGuidesLocked),
    viewerRow(l10n.viewerOverlayClearGuides,
        ui.guides.isEmpty ? null : () => ui.setGuides(const []), close,
        id: 'viewer-guides-clear'),
  ];
}

/// A ruler's menu: a guide from this strip where the pointer is, the units, the
/// guides' own rows and the rulers' tick.
void showViewerRulerMenu(
  BuildContext context,
  LumitUiState ui,
  ViewerMenuHost host,
  bool vertical,
  TapUpDetails details,
) {
  void units(bool percent) {
    ui.workspace.interface.viewerRulerPercent = percent;
    ui.workspace.settingsChanged();
  }

  final percent = ui.workspace.interface.viewerRulerPercent;
  showMenuAt<void>(
    context: context,
    position: details.globalPosition,
    rows: (closeWith) {
      void close() => closeWith(null);
      final along = vertical ? details.localPosition.dy : details.localPosition.dx;
      return [
        // The strip counts one way and its guide runs the other: the left
        // strip gives the horizontal line through the pointer's height.
        viewerRow(
            vertical
                ? l10n.viewerMenuAddHorizontalGuide
                : l10n.viewerMenuAddVerticalGuide, () {
          // A strip's own frame starts where it does, so the left one is
          // measured from the band below the top one.
          final screen = vertical ? along + viewerRulerBand : along;
          ui.setGuides([
            ...ui.guides,
            (
              at: viewerGuideComp(screen,
                  vertical: !vertical,
                  picture: host.picture,
                  compSize: host.compSize),
              vertical: !vertical,
            ),
          ]);
        }, close, id: 'viewer-ruler-add'),
        const MenuDivider(),
        viewerRow(l10n.viewerMenuLockGuides, () {
          final on = ui.workspace.interface;
          on.viewerGuidesLocked = !on.viewerGuidesLocked;
          ui.workspace.settingsChanged();
        }, close,
            id: 'viewer-ruler-lock',
            ticked: ui.workspace.interface.viewerGuidesLocked),
        viewerRow(l10n.viewerOverlayClearGuides,
            ui.guides.isEmpty ? null : () => ui.setGuides(const []), close,
            id: 'viewer-ruler-clear'),
        const MenuDivider(),
        viewerSub(
          l10n.viewerMenuRulerUnits,
          close,
          (dismiss) => [
            viewerRow(l10n.viewerMenuRulerPixels, () => units(false), dismiss,
                id: 'viewer-ruler-pixels', ticked: !percent),
            viewerRow(l10n.viewerMenuRulerPercent, () => units(true), dismiss,
                id: 'viewer-ruler-percent', ticked: percent),
          ],
          id: 'viewer-ruler-units',
        ),
        viewerRow(
            l10n.viewerOverlayRulers,
            () => ui.setViewerOverlays(rulers: !ui.viewerOverlays.rulers),
            close,
            action: 'viewer.rulers.toggle',
            ticked: ui.viewerOverlays.rulers),
      ];
    },
  );
}

/// One guide's menu: where it is, and taking it away.
void showViewerGuideMenu(
  BuildContext context,
  LumitUiState ui,
  ViewerMenuHost host,
  int index,
  TapUpDetails details,
) {
  final guides = ui.guides;
  if (index >= guides.length) return;
  final guide = guides[index];
  showMenuAt<void>(
    context: context,
    position: details.globalPosition,
    rows: (closeWith) {
      void close() => closeWith(null);
      return [
        viewerRow(l10n.viewerMenuGuidePosition, () async {
          final span =
              guide.vertical ? host.compSize.width : host.compSize.height;
          final asked = await askNumberFrb(
            context: context,
            title: l10n.viewerMenuGuidePosition,
            value: guide.at,
            min: 0,
            max: span,
          );
          if (asked == null || index >= ui.guides.length) return;
          ui.setGuides([
            for (var i = 0; i < ui.guides.length; i++)
              if (i == index)
                (at: asked, vertical: guide.vertical)
              else
                ui.guides[i],
          ]);
        }, close, id: 'viewer-guide-position'),
        const MenuDivider(),
        viewerRow(l10n.viewerMenuLockGuides, () {
          final on = ui.workspace.interface;
          on.viewerGuidesLocked = !on.viewerGuidesLocked;
          ui.workspace.settingsChanged();
        }, close,
            id: 'viewer-guide-lock',
            ticked: ui.workspace.interface.viewerGuidesLocked),
        viewerRow(l10n.viewerOverlayClearGuides, () => ui.setGuides(const []),
            close,
            id: 'viewer-guide-clear'),
        const MenuDivider(),
        viewerRow(l10n.viewerMenuDeleteGuide, () {
          ui.setGuides([
            for (var i = 0; i < ui.guides.length; i++)
              if (i != index) ui.guides[i],
          ]);
        }, close, id: 'viewer-guide-delete'),
      ];
    },
  );
}

/// The region of interest: sweeping one out, or letting go of the one in force.
Widget viewerRegionRow(LumitUiState ui, VoidCallback close) {
  final region = ui.regionOfInterest != null;
  return viewerRow(
      region ? l10n.viewerOverlayClearRegion : l10n.viewerOverlayRegion,
      () => region
          ? ui.setRegionOfInterest(null)
          : ui.armingRegion = !ui.armingRegion,
      close,
      id: 'viewer-region',
      ticked: region || ui.armingRegion);
}

/// The composition's own two rows.
List<Widget> viewerCompositionRows(
  BuildContext context,
  LumitUiState ui,
  ViewerMenuHost host,
  VoidCallback close,
  Offset at,
) =>
    [
      viewerRow(l10n.viewerOverlayBackground, () {
        showViewerBackgroundPicker(
          context: context,
          comp: host.comp,
          background: host.background,
          position: at,
        );
      }, close, id: 'viewer-background'),
      viewerRow(l10n.viewerMenuCompSettings, () async {
        final state = Provider.of<LumitState>(context, listen: false);
        final applied =
            await showCompSettingsFrb(context: context, comp: host.comp);
        if (applied) state.notifyDocumentChanged();
      }, close, id: 'viewer-menu-comp-settings'),
    ];

/// The picture's menu. [layerRows] is what the layer controls know about the
/// layers under the pointer: empty when there are none, as under a tool that
/// has no use for them.
void showViewerPictureMenu({
  required BuildContext context,
  required LumitUiState ui,
  required ViewerMenuHost host,
  required Offset at,
  required Offset stageAt,
  List<Widget> Function(VoidCallback close) layerRows = _noRows,
}) {
  showMenuAt<void>(
    context: context,
    position: at,
    rows: (closeWith) {
      void close() => closeWith(null);
      final top = layerRows(close);
      return [
        if (top.isNotEmpty) ...[...top, const MenuDivider()],
        viewerRow(l10n.menuAddMarkerAtPlayhead,
            () => addMarkerFrb(host.comp, frame: ui.playheadFrame.value), close,
            id: 'viewer-menu-marker', action: 'marker.add'),
        viewerSub(l10n.viewerMenuGuides, close,
            (dismiss) => viewerGuideRows(ui, host, dismiss, stageAt: stageAt),
            id: 'viewer-menu-guides'),
        const MenuDivider(),
        viewerRow(l10n.viewerMenuCopyFrame,
            () => copyViewerFrame(context, ui, host), close,
            id: 'viewer-menu-copy-frame'),
        viewerRow(l10n.viewerMenuSaveFrame,
            () => saveViewerFrame(context, ui, host), close,
            id: 'viewer-menu-save-frame'),
        viewerSub(l10n.viewerMenuSnapshot, close,
            (dismiss) => viewerSnapshotRows(host, dismiss),
            id: 'viewer-menu-snapshot'),
        const MenuDivider(),
        viewerSub(l10n.viewerMenuZoom, close,
            (dismiss) => viewerZoomRows(ui, host, dismiss),
            id: 'viewer-menu-zoom'),
        viewerSub(l10n.menuResolution, close,
            (dismiss) => viewerResolutionRows(ui, dismiss, viewId: host.viewId)),
        viewerSub(l10n.tipViewerChannel, close,
            (dismiss) => viewerChannelRows(host, dismiss),
            id: 'viewer-menu-channel'),
        viewerSub(l10n.viewerMenuShow, close,
            (dismiss) => viewerShowRows(ui, host, dismiss),
            id: 'viewer-menu-show'),
        const MenuDivider(),
        viewerSub(l10n.menuViewLayout, close,
            (dismiss) => viewerLayoutRows(ui, dismiss)),
        viewerSub(l10n.menuCompare, close,
            (dismiss) => viewerCompareRows(ui, dismiss)),
        viewerRegionRow(ui, close),
        const MenuDivider(),
        ...viewerCompositionRows(context, ui, host, close, at),
      ];
    },
  );
}

List<Widget> _noRows(VoidCallback close) => const [];

/// What a layer's row in a picture menu opens: the layer menu a row in the
/// Timeline has, for the layer [id], acting on the selection as it does there.
Widget viewerLayerFlyout(
    BuildContext context, VoidCallback dismiss, UuidValue id) {
  final app = Provider.of<LumitState>(context, listen: false);
  final ui = Provider.of<LumitUiState>(context, listen: false);
  final entry =
      ui.model.layers.where((e) => e.layer.internallayerId == id).firstOrNull;
  return menuEntriesFlyout(
    entry == null
        ? const []
        : layerMenuEntries(context, app, ui, entry,
            onRename: layerRowRename(id)),
    dismiss,
  );
}

/// The source view's menu: the marks, placing what they hold, and letting it go.
List<Widget> viewerSourceRows(
  LumitUiState ui,
  ViewerSurface view,
  VoidCallback close,
) {
  final canPlace = ui.selectedComp != null;
  final marked = view.sourceIn != null || view.sourceOut != null;
  return [
    viewerRow(l10n.sourceMarkIn, () => ui.markSource(view, markIn: true), close,
        id: 'source-menu-in', action: 'cut.mark.in'),
    viewerRow(l10n.sourceMarkOut, () => ui.markSource(view, markIn: false), close,
        id: 'source-menu-out', action: 'cut.mark.out'),
    viewerRow(
        l10n.viewerMenuClearMarks,
        marked
            ? () {
                view.sourceIn = null;
                view.sourceOut = null;
                ui.views.touch();
              }
            : null,
        close,
        id: 'source-menu-clear'),
    const MenuDivider(),
    viewerRow(l10n.sourceInsert,
        canPlace ? () => ui.placeSource(view, insert: true) : null, close,
        id: 'source-menu-insert', action: 'cut.insert'),
    viewerRow(l10n.sourceOverwrite,
        canPlace ? () => ui.placeSource(view, insert: false) : null, close,
        id: 'source-menu-overwrite', action: 'cut.overwrite'),
    const MenuDivider(),
    viewerRow(l10n.viewerMenuCloseSource, () {
      view.itemId = null;
      view.sourceIn = null;
      view.sourceOut = null;
      ui.views.touch();
    }, close, id: 'source-menu-close'),
  ];
}
