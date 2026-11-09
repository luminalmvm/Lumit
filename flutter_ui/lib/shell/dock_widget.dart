// Renders the dock tree (state/dock.dart): weighted splits with draggable
// dividers, tab groups as pill tab bars (dock.rs::tab_ui styling), solo panes
// bare, and the Sharp/Round pane chrome. A tab drags to re-dock (dock.rs
// drag-to-redock, via egui_tiles): a ghost pill follows the cursor, the
// hovered pane shows a drop-zone preview, and release commits the move through
// movePanel. Every pane is a drop target, bare ones included. A pane standing
// alone has no tab, so its title lifts it instead (DockPaneHandle).

import 'dart:async' show Timer, unawaited;
import 'dart:math' show max, min;
import 'dart:ui' show ViewFocusEvent, ViewFocusState;

import 'package:flutter/foundation.dart' show defaultTargetPlatform;
import 'package:flutter/gestures.dart'
    show GestureBinding, PointerScrollEvent, PointerSignalEvent;
import 'package:flutter/rendering.dart' show RenderOffstage;
import 'package:flutter/services.dart' show HardwareKeyboard;
import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../main.dart' show LumitNotice, LumitState, LumitUiState;
import '../l10n/strings.dart';
import '../panels/viewer_strips.dart' show viewerPaneCorners;
import '../state/dock.dart';
import '../theme/style_sheet.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/edge_fade.dart';
import '../widgets/ui_scale.dart';
import 'pop_out.dart';
import 'window_frame.dart'
    show
        FloatingResizeEdges,
        FloatingTitleLine,
        FloatingWindowButtons,
        WindowDragArea,
        floatingButtonsWidth,
        showFloatingWindowMenu,
        windowFrameDrawn;

typedef PanelBuilder = Widget Function(BuildContext context, PaneId pane);

/// A pointer must travel this far before a press on a tab becomes a re-dock
/// drag rather than a click.
const double _dragSlop = 6.0;

/// **The width below which a panel stops giving way and starts sliding** (the
/// degradation ladder, step 5, and the "every panel declares a minimum" rule
/// that goes with it).
///
/// In plain terms: a panel shrinks by shedding — a word here, a column there —
/// but every panel reaches a width where there is nothing left to shed and the
/// only honest answers are to slide the content sideways or to paint outside
/// the box. This number is where that line is, and [PanelFloor] is what does
/// the sliding. Dragging a seam past it is refused outright, so the ordinary
/// way of making a panel narrow never reaches the scrolling case at all; the
/// scroll is for the window that is simply too small to hold the workspace.
///
/// The numbers are what each panel measurably needs once its own ladder has
/// run out — the width its narrowest useful arrangement lays out at without
/// overflowing — and `panel_width_sweep_test` is what keeps them honest.
double panelMinWidth(Panel panel) => switch (panel) {
      // Its columns give way on their own down to here, so nothing is cut at
      // the panel's edge above it (`projectMinWidth`).
      Panel.project => 180,
      // The picture itself will draw at any size, and both strips slide on
      // their own below the width they want, so the Viewer's floor is only
      // about there being a picture worth looking at.
      Panel.viewer => 200,
      // The outline's own columns, before a single lane of time.
      Panel.timeline => 220,
      // A parameter row: its name, its wells and its keyframe marks.
      Panel.effectControls => 240,
      // The toolbar folds into one mark and the canvas draws at any size.
      Panel.graph => 200,
      Panel.effectsAndPresets => 180,
      Panel.node => 200,
      Panel.scopes => 160,
      // A primary's name, its track and its number; the wheels stack under
      // this.
      Panel.grade => 200,
      Panel.hierarchy => 180,
      Panel.easing => 220,
      Panel.retime => 220,
      // Two strips and the Master: below that a desk is a list of names.
      Panel.mixer => 190,
      // The Beats section's widest row: a label, the BPM well, Tap and the
      // phase chips.
      Panel.audio => 260,
      // The Audio timeline's own outline row, measured cell by cell: 8 of pad,
      // three switch cells at 22, the gap, a 16 twirl, the 18 number, the
      // label dot and their four 8-px gaps, a name worth reading, the Spectral
      // chip and the 10 trailing inset. That comes to 278, and the board draws
      // the column at 300.
      Panel.audioTimeline => 300,
      // The track headers' column and a lane of time beside it.
      Panel.cutTimeline => 260,
      // A label and a value well side by side, which is every row they have.
      Panel.text => 220,
      Panel.paragraph => 220,
      Panel.expressions => 180,
      // The two rows' labels and controls, and the two buttons.
      Panel.stagger => 220,
      Panel.notes => 160,
      // One grid. The second goes under it when there is no room beside.
      Panel.align => 120,
      Panel.debug => 180,
      // A page lays itself out, and a timer or a sign-in form needs this much.
      Panel.extension => 240,
    };

/// The smallest width a whole dock subtree can be drawn at: a pane is its
/// panel's floor, a tab group is the widest of the panels stacked in it, and a
/// split is the sum of its children across, or the widest of them down.
double dockMinWidth(DockNode node) => switch (node) {
      DockPane(:final panel) => panelMinWidth(panel),
      DockTabs(:final children) => children.isEmpty
          ? 0
          : children
              .map((c) => panelMinWidth(c.panel))
              .reduce((a, b) => a > b ? a : b),
      DockSplit(:final axis, :final children) => children.isEmpty
          ? 0
          : axis == DockAxis.horizontal
              ? children.map(dockMinWidth).reduce((a, b) => a + b)
              : children.map(dockMinWidth).reduce((a, b) => a > b ? a : b),
    };

/// The height below which a stacked split stops shrinking a panel and takes
/// the room from a sibling instead. The timelines have one: their header,
/// their column header and six rows. So does the Audio panel: its meter, a
/// layer's rows and the Beats row, under its header and a shut panel's.
/// Everything else scales or scrolls.
double panelMinHeight(Panel panel) => switch (panel) {
      Panel.timeline || Panel.audioTimeline || Panel.cutTimeline => 250,
      Panel.audio => 350,
      _ => 0,
    };

/// [dockMinWidth]'s twin down the page: a split is the sum of its children
/// down, or the tallest of them across.
double dockMinHeight(DockNode node) => switch (node) {
      DockPane(:final panel) => panelMinHeight(panel),
      DockTabs(:final children) => children.isEmpty
          ? 0
          : children
              .map((c) => panelMinHeight(c.panel))
              .reduce((a, b) => a > b ? a : b),
      DockSplit(:final axis, :final children) => children.isEmpty
          ? 0
          : axis == DockAxis.vertical
              ? children.map(dockMinHeight).reduce((a, b) => a + b)
              : children.map(dockMinHeight).reduce((a, b) => a > b ? a : b),
    };

/// Holds [child] to [minWidth] and slides it sideways when the pane is
/// narrower than that (§12A.6's ladder, step 5).
///
/// In plain terms: rather than squeeze a panel into a box it cannot fit, give
/// it the box it needs and let the user push it left and right. Every docked
/// panel is wrapped in one of these, which is why a panel nobody has audited
/// for narrow widths still cannot paint outside its pane.
class PanelFloor extends StatelessWidget {
  final double minWidth;
  final Widget child;

  const PanelFloor({super.key, required this.minWidth, required this.child});

  @override
  Widget build(BuildContext context) => LayoutBuilder(
        builder: (context, box) {
          // Nothing to hold it to, or room to spare: the panel lays out
          // exactly as it always did, and this wrapper costs one LayoutBuilder.
          if (!box.hasBoundedWidth ||
              !box.hasBoundedHeight ||
              box.maxWidth >= minWidth) {
            return child;
          }
          // Faded at the edge that has more, so a cut panel says it slides.
          return EdgeFade(
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: SizedBox(
                width: minWidth,
                // The height has to be handed over as a number: a panel is a
                // Column with an Expanded in it, and inside a scroll view the
                // cross axis arrives loose.
                height: box.maxHeight,
                child: child,
              ),
            ),
          );
        },
      );
}

class DockWidget extends StatefulWidget {
  final DockSplit root;
  final PanelBuilder buildPanel;
  final VoidCallback onLayoutChanged;

  /// The pane filling the window, or null when the arrangement is showing
  /// normally (docs/07 §1.1's backtick, and the Viewer's cinema view).
  ///
  /// **Not part of the arrangement**: it is a way of looking at one for a
  /// moment, so nothing here is written down and a restart opens as it was.
  final ValueNotifier<PaneId?> maximised;

  /// The panel that last took a click — it wears the accent boundary so the
  /// keyboard's home is always visible (Shell::active_panel).
  /// A pane, not a panel, because two Viewers are two homes.
  final ValueNotifier<PaneId?> activePanel;

  /// Settings ▸ Interface ▸ Scale, which a popped-out window draws at too.
  final double uiScale;

  const DockWidget({
    super.key,
    required this.root,
    required this.buildPanel,
    required this.onLayoutChanged,
    required this.activePanel,
    required this.maximised,
    this.uiScale = 1.0,
  });

  @override
  State<DockWidget> createState() => _DockWidgetState();
}

/// What the keyboard asks of the dock, which is the one thing that knows
/// how big each panel is and where each window stands. Each answers whether
/// anything moved.
abstract final class DockCommands {
  static bool Function(PaneId pane) _float = (_) => false;
  static bool Function(PaneId pane) _dock = (_) => false;

  /// Move [pane] into a window of its own.
  static bool float(PaneId pane) => _float(pane);

  /// Put [pane] back in the main window.
  static bool dock(PaneId pane) => _dock(pane);
}

class _DockWidgetState extends State<DockWidget> with WidgetsBindingObserver {
  // One stable key per pane, used to hit-test the pane rects during a drag.
  // A pane keeps its key even while it is an inactive tab (unbuilt, so its
  // key resolves to no context and is skipped). Made on demand, because the
  // Viewer can be in the arrangement any number of times and the set is no
  // longer known up front.
  final Map<PaneId, GlobalKey> _paneKeys = {};

  GlobalKey _paneKey(PaneId pane) => _paneKeys.putIfAbsent(pane, GlobalKey.new);

  // A stack's headers are drop targets too, since a panel twirled shut has no
  // body to drop on.
  final Map<PaneId, GlobalKey> _headerKeys = {};

  GlobalKey _headerKey(PaneId pane) =>
      _headerKeys.putIfAbsent(pane, GlobalKey.new);
  late final _DragController _drag = _DragController(
    paneKey: _paneKey,
    headerKey: _headerKey,
    // Floating windows stand above the main one, the latest on top, so a
    // pointer over one that covers part of the main window is over it.
    panes: () => [
      for (final w in widget.root.windows.reversed) ...panesIn(w.root),
      ...mainPanesIn(widget.root),
    ],
    places: _places,
    onStart: _placeViews,
    onGhostShow: _showGhost,
    onGhostHide: _removeGhost,
    onCommit: _commitMove,
    onFloat: _float,
    onOutside: _outline,
  );

  /// The outline sent last, so a pointer standing still sends nothing.
  Rect? _outlined;

  void _outline(Rect? bounds) {
    if (bounds == _outlined) return;
    _outlined = bounds;
    unawaited(NativeWindows.ghost(bounds, _drag.theme?.accent));
  }

  /// The pill that follows a drag, one in each window, each drawing it only
  /// while the pointer is over its own window.
  final List<OverlayEntry> _ghosts = [];

  /// The floating windows on screen, by the arrangement's window id. Opened
  /// a frame after a window joins the arrangement and closed a frame after it
  /// leaves, so a view is never drawn into a window that is not there.
  final Map<int, _Host> _hosts = {};

  /// A frame has been asked for to open the windows still missing.
  bool _opening = false;

  /// The main window's view, which every other window's place is read
  /// against.
  int? _mainView;

  /// Where each of Lumit's views draws on the screen, by view id, as the
  /// runner said when a drag last started.
  final Map<int, _ViewPlace> _places = {};

  /// The last pane the keyboard was in, in each window: 0 for the main one.
  final Map<int, PaneId> _lastActive = {};

  /// How big a window opened from a menu is made, by window id: the size its
  /// panel had in the window it came from.
  final Map<int, Size> _sizeHints = {};

  /// Windows the user has just made, which come up with the keyboard. Every
  /// other window opens quietly: one coming back at start, or with a project
  /// or a workspace, leaves the keyboard where it was.
  final Set<int> _madeNow = {};

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    widget.activePanel.addListener(_noteActive);
    NativeWindows.moved = _movedByRunner;
    DockCommands._float = _floatKey;
    DockCommands._dock = _dockKey;
    frontWindowOverlay = _frontOverlay;
    mainWindowOverlay = _mainOverlay;
    otherWindowOverlays = _otherOverlays;
  }

  /// The main window's own overlay.
  OverlayState? _mainOverlay() => mounted ? Overlay.maybeOf(context) : null;

  /// Every window's overlay but [own].
  List<OverlayState> _otherOverlays(OverlayState own) => [
        if (_mainOverlay() case final main? when main != own) main,
        for (final host in _hosts.values)
          if (host.overlay.currentState case final other? when other != own)
            other,
      ];

  /// The overlay of the floating window in front, when it can hold a
  /// dialogue: what the main window's dialogues open in meanwhile.
  OverlayState? _frontOverlay() {
    for (final host in _hosts.values) {
      if (!host.window.isActivated) continue;
      final overlay = host.overlay.currentState;
      final size = overlay == null ? null : MediaQuery.maybeSizeOf(overlay.context);
      return size != null && roomForDialogue(size) ? overlay : null;
    }
    return null;
  }

  bool _floatKey(PaneId pane) {
    if (!popOutAvailable || !canFloat(widget.root, pane)) return false;
    _float(pane);
    return true;
  }

  bool _dockKey(PaneId pane) {
    if (windowOf(widget.root, pane) == null) return false;
    _dockPane(pane);
    return true;
  }

  @override
  void didUpdateWidget(DockWidget old) {
    super.didUpdateWidget(old);
    if (!identical(old.activePanel, widget.activePanel)) {
      old.activePanel.removeListener(_noteActive);
      widget.activePanel.addListener(_noteActive);
    }
    // Another arrangement came in, a project's or a workspace's: a window it
    // shares with the one before keeps its native window, and goes where the
    // new arrangement says.
    if (!identical(old.root, widget.root)) _replace();
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    widget.activePanel.removeListener(_noteActive);
    if (NativeWindows.moved == _movedByRunner) NativeWindows.moved = null;
    if (DockCommands._float == _floatKey) DockCommands._float = (_) => false;
    if (DockCommands._dock == _dockKey) DockCommands._dock = (_) => false;
    if (frontWindowOverlay == _frontOverlay) frontWindowOverlay = null;
    if (mainWindowOverlay == _mainOverlay) mainWindowOverlay = null;
    if (otherWindowOverlays == _otherOverlays) otherWindowOverlays = null;
    for (final host in _hosts.values) {
      host.dispose();
      final window = host.window;
      WidgetsBinding.instance.addPostFrameCallback((_) => window.close());
    }
    _hosts.clear();
    _removeGhost();
    _drag.dispose();
    super.dispose();
  }

  DockWindow? _window(int id) {
    for (final w in widget.root.windows) {
      if (w.id == id) return w;
    }
    return null;
  }

  // --- Floating windows ---------------------------------------------------

  /// Open the windows the arrangement has and the screen has not, and close
  /// the ones it no longer has.
  void _syncWindows() {
    final wanted = {for (final w in widget.root.windows) w.id};
    for (final id in _hosts.keys.where((id) => !wanted.contains(id)).toList()) {
      final host = _hosts.remove(id)!..dispose();
      dismissModalsIn(host.overlay.currentState);
      WidgetsBinding.instance.addPostFrameCallback((_) => host.window.close());
    }
    // A window's title names its panels, which move in and out of it. Set
    // after the frame: the window tells its listeners, which build.
    for (final w in widget.root.windows) {
      final host = _hosts[w.id];
      final title = _windowTitle(w);
      if (host == null || host.title == title) continue;
      host.title = title;
      WidgetsBinding.instance
          .addPostFrameCallback((_) => host.window.setTitle(title));
    }
    if (_opening || wanted.every(_hosts.containsKey)) return;
    _opening = true;
    WidgetsBinding.instance.addPostFrameCallback((_) => _openMissing());
  }

  Future<void> _openMissing() async {
    try {
      if (!mounted) return;
      if (!popOutAvailable) {
        // Nowhere to open a window, a browser above all: the panels come back
        // into the window there is rather than going missing.
        if (widget.root.windows.isEmpty) return;
        setState(() => dockAllWindows(widget.root));
        widget.onLayoutChanged();
        return;
      }
      final displays = await NativeWindows.displays();
      final main = await NativeWindows.frame();
      var cascade = 0;
      for (final w in widget.root.windows.toList()) {
        if (!mounted) return;
        if (_hosts.containsKey(w.id) || !widget.root.windows.contains(w)) {
          continue;
        }
        _open(
            w,
            quiet: !_madeNow.remove(w.id),
            fitWindowFrame(w.frame,
                displays: displays,
                main: main,
                size: _sizeHints.remove(w.id),
                cascade: cascade++),
            main?.scale ?? 1.0);
      }
    } finally {
      _opening = false;
    }
    if (mounted) setState(() {});
  }

  /// Open [w]'s native window and put it at [frame]: frameless where Lumit
  /// draws the title line, above the main window, watched for moves. Placed
  /// before it is first drawn, so it never shows somewhere else first.
  ///
  /// [units] is how many of the system's units make one logical pixel, for
  /// the size the window is first made at.
  void _open(DockWindow w, FloatingFrame? frame, double units,
      {bool quiet = false}) {
    final window = PopOutWindow.open(
      title: _windowTitle(w),
      size: frame == null
          ? const Size(960, 600)
          : Size(frame.bounds.width / units, frame.bounds.height / units),
      onClosed: () => _closed(w.id),
    );
    if (window == null) {
      setState(() => dockWindow(widget.root, w.id));
      widget.onLayoutChanged();
      return;
    }
    final host = _Host(w.id, window)
      ..placed = frame
      ..title = _windowTitle(w);
    host.listener = () => _changed(w.id);
    window.changes.addListener(host.listener!);
    _hosts[w.id] = host;
    host.ready = _adopt(host, w, frame, quiet);
  }

  Future<void> _adopt(
      _Host host, DockWindow w, FloatingFrame? frame, bool quiet) async {
    final handle = host.window.handle;
    if (handle == 0) return;
    await NativeWindows.adopt(handle,
        ownTitle: windowFrameDrawn, onTop: w.onTop, quiet: quiet);
    if (!_hosts.containsKey(w.id)) return;
    // Placing it shows it there, with the keyboard left where it was.
    if (frame != null) {
      await NativeWindows.setFrame(handle, frame.bounds,
          maximised: frame.maximised);
      if (frame.fullScreen) host.window.setFullScreen(true);
    }
    // A window made just now by hand comes to the front with the keyboard.
    if (!quiet && _hosts.containsKey(w.id)) {
      if (frame?.maximised ?? false) {
        host.window.setMaximised(true);
      } else if (!(frame?.fullScreen ?? false)) {
        host.window.activate();
      }
    }
  }

  /// Every window the new arrangement shares with the old keeps its native
  /// window; the ones whose place differs move.
  void _replace() {
    for (final w in widget.root.windows) {
      final host = _hosts[w.id];
      if (host == null || host.placed == w.frame) continue;
      host.placed = w.frame;
      final frame = w.frame;
      if (frame == null) continue;
      host.ready = host.ready.then((_) async {
        final displays = await NativeWindows.displays();
        final main = await NativeWindows.frame();
        final fitted =
            fitWindowFrame(frame, displays: displays, main: main) ?? frame;
        if (!_hosts.containsKey(w.id)) return;
        host.window.setFullScreen(false);
        await NativeWindows.setFrame(host.window.handle, fitted.bounds,
            maximised: fitted.maximised);
        if (fitted.fullScreen) host.window.setFullScreen(true);
      });
    }
  }

  /// The system's own title for [w]: what its panels are called.
  String _windowTitle(DockWindow w) => l10n.floatingWindowTitle([
        for (final node in w.root.children) ..._frontTitles(node),
      ].join(', '));

  List<String> _frontTitles(DockNode node) => switch (node) {
        DockPane pane => [paneTitle(pane.id)],
        DockTabs tabs =>
          tabs.stacked ? [for (final c in tabs.children) paneTitle(c.id)]
              : [paneTitle(tabs.activePane.id)],
        DockSplit split => [
            for (final c in split.children) ..._frontTitles(c),
          ],
      };

  /// The system closed window [id], or its close button was pressed: its
  /// panels go back into the main window. Not while Lumit is quitting, when
  /// they stay where they are for the next start.
  void _closed(int id) {
    if (!mounted || windowsQuitting) return;
    if (_window(id) == null) return;
    // A dialogue up in it holds work, a filled-in export say: the window
    // stays until that is answered.
    if (modalOpenIn(_hosts[id]?.overlay.currentState)) return;
    setState(() => dockWindow(widget.root, id));
    widget.onLayoutChanged();
  }

  /// Window [id] was resized, put in front or sent back (and moved, on
  /// Linux).
  void _changed(int id) {
    final host = _hosts[id];
    if (host == null || !mounted) return;
    final active = host.window.isActivated;
    if (active && !host.active) _focusWindow(id);
    host.active = active;
    host.soon(() => _noteFrame(id));
  }

  /// The runner says a window it watches stopped moving.
  void _movedByRunner(int handle) {
    for (final host in _hosts.values) {
      if (host.window.handle == handle) {
        host.soon(() => _noteFrame(host.id));
        return;
      }
    }
  }

  /// Write down where window [id] is now, so it opens there next time.
  Future<void> _noteFrame(int id) async {
    final host = _hosts[id];
    if (host == null) return;
    final now = await NativeWindows.frame(host.window.handle);
    final w = _window(id);
    if (now == null || now.minimised || w == null || !mounted) return;
    // Full screen, the window stands where it stood before, as it will again
    // when it leaves full screen.
    final full = host.window.isFullScreen || now.fullScreen;
    final before = w.frame;
    final frame = FloatingFrame(
        full && before != null ? before.bounds : now.bounds,
        maximised: full ? before?.maximised ?? false : now.maximised,
        fullScreen: full,
        display: now.display);
    if (frame == w.frame) return;
    w.frame = frame;
    host.placed = frame;
    widget.onLayoutChanged();
  }

  /// Keep the last pane the keyboard was in, per window.
  void _noteActive() {
    final pane = widget.activePanel.value;
    if (pane == null) return;
    _lastActive[windowOf(widget.root, pane)?.id ?? 0] = pane;
  }

  /// Window [id] (0 for the main one) came to the front: the keyboard goes
  /// back to the pane it was in there.
  void _focusWindow(int id) {
    final roots = id == 0
        ? mainPanesIn(widget.root)
        : panesIn(_window(id)?.root ?? DockSplit(DockAxis.horizontal, [], []));
    if (roots.isEmpty) return;
    final current = widget.activePanel.value;
    if (current != null && roots.contains(current)) return;
    final last = _lastActive[id];
    widget.activePanel.value =
        last != null && roots.contains(last) ? last : roots.first;
  }

  @override
  void didChangeViewFocus(ViewFocusEvent event) {
    if (event.state != ViewFocusState.focused) return;
    if (event.viewId == _mainView) {
      _focusWindow(0);
      return;
    }
    for (final host in _hosts.values) {
      if (host.window.viewId == event.viewId) {
        _focusWindow(host.id);
        return;
      }
    }
  }

  /// Ask the runner where every view is, for a drag that may cross windows.
  Future<void> _placeViews() async {
    final main = _mainView;
    final asked = <int, Future<NativeFrame?>>{
      if (main != null) main: NativeWindows.frame(),
      for (final host in _hosts.values)
        host.window.viewId: NativeWindows.frame(host.window.handle),
    };
    _places.clear();
    for (final MapEntry(key: view, value: frame) in asked.entries) {
      final f = await frame;
      if (f != null) _places[view] = (client: f.client, scale: f.scale);
    }
  }

  /// Take [pane] out into a window of its own, at [bounds] when a drag let go
  /// somewhere, and otherwise over the main window at the size it had.
  /// [group] takes the whole group it is in.
  void _float(PaneId pane, {Rect? bounds, bool group = false}) {
    final size = _paneSize(pane);
    DockWindow? made;
    setState(() {
      final frame = bounds == null ? null : FloatingFrame(bounds);
      made = group
          ? floatGroup(widget.root, pane, frame: frame)
          : floatPane(widget.root, pane, frame: frame);
    });
    final window = made;
    if (window == null) return;
    if (size != null) _sizeHints[window.id] = size;
    _madeNow.add(window.id);
    widget.activePanel.value = pane;
    widget.onLayoutChanged();
  }

  /// [pane]'s size on screen, in logical pixels, or null when it is not
  /// drawn.
  Size? _paneSize(PaneId pane) {
    final box = _paneKey(pane).currentContext?.findRenderObject();
    if (box is! RenderBox || !box.attached || !box.hasSize) return null;
    return MatrixUtils.transformRect(
            box.getTransformTo(null), Offset.zero & box.size)
        .size;
  }

  /// Put [pane] back in the main window.
  void _dockPane(PaneId pane) {
    setState(() => dockPane(widget.root, pane));
    widget.activePanel.value = pane;
    widget.onLayoutChanged();
  }

  /// Put every panel of window [id] back in the main window.
  void _dockWindow(int id) {
    setState(() => dockWindow(widget.root, id));
    widget.onLayoutChanged();
  }

  /// Keep window [id] above other programs' windows, or not.
  void _setOnTop(int id, bool on) {
    final w = _window(id);
    final host = _hosts[id];
    if (w == null || host == null) return;
    w.onTop = on;
    unawaited(NativeWindows.setOnTop(host.window.handle, on));
    setState(() {});
    widget.onLayoutChanged();
  }

  void _showGhost() {
    _removeGhost();
    void insert(OverlayState? overlay, int? view) {
      if (overlay == null) return;
      final entry = OverlayEntry(builder: (_) => _GhostLayer(drag: _drag, view: view));
      overlay.insert(entry);
      _ghosts.add(entry);
    }

    insert(Overlay.maybeOf(context), _mainView);
    for (final host in _hosts.values) {
      insert(host.overlay.currentState, host.window.viewId);
    }
  }

  void _removeGhost() {
    for (final entry in _ghosts) {
      entry.remove();
    }
    _ghosts.clear();
  }

  void _commitMove(PaneId dragged, PaneId target, DropPosition pos) {
    setState(() => movePanel(widget.root, dragged, target, pos));
    widget.activePanel.value = dragged;
    widget.onLayoutChanged();
  }

  /// Drop one pane out of the arrangement, reached from the tab's own
  /// right-click menu. One pane, not the panel: closing the second Viewer
  /// leaves the first where it was.
  void _closePanel(PaneId pane) {
    setState(() => closePane(widget.root, pane));
    widget.onLayoutChanged();
  }

  /// Close every other pane of [keep]'s group.
  void _closeOthers(PaneId keep, DockTabs group) {
    setState(() {
      for (final c in group.children.toList()) {
        if (c.id != keep) closePane(widget.root, c.id);
      }
    });
    widget.onLayoutChanged();
  }

  /// Put [panel] into [beside]'s group, fronted. The Viewer, which may be in
  /// the arrangement more than once, comes as another pane of it.
  void _addPanel(Panel panel, {required PaneId beside}) {
    setState(() {
      final PaneId made;
      if (panel == Panel.viewer) {
        made = addPane(widget.root, panel);
      } else {
        setPanelVisible(widget.root, panel, true);
        made = panel.pane();
      }
      movePanel(widget.root, made, beside, DropPosition.stack);
      widget.activePanel.value = made;
    });
    widget.onLayoutChanged();
  }

  /// Give children i and j of [split] the same room, where both panels'
  /// minimums allow it. [extent] is the split's own width or height.
  void _equalise(DockSplit split, int i, int j, double extent) {
    final sum = split.shares[i] + split.shares[j];
    final each = sum / 2;
    if (split.axis == DockAxis.horizontal) {
      final total = split.shares.reduce((a, b) => a + b);
      final px = each / total * extent;
      if (px < dockMinWidth(split.children[i]) ||
          px < dockMinWidth(split.children[j])) {
        return;
      }
    }
    setState(() {
      split.shares[i] = each;
      split.shares[j] = each;
    });
    widget.onLayoutChanged();
  }

  @override
  Widget build(BuildContext context) {
    _mainView = View.maybeOf(context)?.viewId;
    _syncWindows();
    final t = ThemeScope.of(context).theme;
    return ViewAnchor(
      view: _hosts.isEmpty
          ? null
          : ViewCollection(views: [
              for (final w in widget.root.windows)
                if (_hosts[w.id] case final host?)
                  KeyedSubtree(
                    key: ValueKey<(String, int)>(('window', w.id)),
                    child: host.window.view(
                        _FloatingWindowView(dock: this, window: w, host: host)),
                  ),
            ]),
      child: _DockScope(
        dock: this,
        child: _dockArea(context, t, widget.root),
      ),
    );
  }

  /// One window's panels: [tree], or the pane in it filling the window when
  /// one has been asked for. A pane closed while it was maximised simply puts
  /// the arrangement back rather than showing nothing.
  Widget _dockArea(BuildContext context, LumitTheme t, DockSplit tree,
          {_Edge? edge}) =>
      ValueListenableBuilder<PaneId?>(
        valueListenable: widget.maximised,
        builder: (context, maximised, _) {
          final held = identical(tree, widget.root)
              ? mainPanesIn(tree)
              : panesIn(tree);
          return Container(
            // Cards stand on the room; flush panes sit on the canvas.
            color: t.tokens.roomed ? t.room : t.surface0,
            padding: EdgeInsets.all(t.tokens.windowInset),
            child: maximised != null && held.contains(maximised)
                ? _maximised(context, t, maximised, edge)
                : _buildNode(context, tree, edge: edge),
          );
        },
      );

  /// The body of floating window [w]: its panels. Where Lumit draws the
  /// window's frame, the line they head it with is the window's top line,
  /// and a line of the window's own goes above them only when the pane in the
  /// top right corner has no line to carry the buttons.
  Widget _floatingBody(BuildContext context, DockWindow w, _Host host) {
    final t = ThemeScope.of(context).theme;
    final _Edge? edge = windowFrameDrawn
        ? (
            host: host,
            corner: true,
            direction: Directionality.of(context),
            onMenu: (context, at) => _showWindowMenu(context, at, w.id),
          )
        : null;
    final area = Stack(children: [
      Positioned.fill(child: _dockArea(context, t, w.root, edge: edge)),
      _WindowNotice(host: host),
    ]);
    if (edge == null) return area;
    // Counted once the first frame is laid out, and until then taken as
    // shown, so a window never opens with a line it then loses.
    if (host.buttons.value == null) {
      WidgetsBinding.instance
          .addPostFrameCallback((_) => host.buttons.value ??= 0);
    }
    return FloatingResizeEdges(
      window: host.window.handle,
      child: ColoredBox(
        color: t.tokens.roomed ? t.room : t.surface0,
        child: ValueListenableBuilder<int?>(
          valueListenable: host.buttons,
          // Stretched, so the panels take the window's whole width rather
          // than what the notice over them asks for.
          builder: (context, shown, area) => Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (shown == 0)
                FloatingTitleLine(
                  theme: t,
                  window: host.window.handle,
                  onTop: w.onTop,
                  onMenu: edge.onMenu,
                ),
              Expanded(child: area!),
            ],
          ),
          child: area,
        ),
      ),
    );
  }

  /// A floating window's own menu, on its top line.
  void _showWindowMenu(BuildContext context, Offset at, int id) {
    final w = _window(id);
    final host = _hosts[id];
    if (w == null || host == null) return;
    showFloatingWindowMenu(
      context,
      at,
      window: host.window.handle,
      maximised: host.window.isMaximised,
      fullScreen: host.window.isFullScreen,
      onTop: w.onTop,
      onSetOnTop: (on) => _setOnTop(id, on),
      onFullScreen: (on) => host.window.setFullScreen(on),
      onDock: () => _dockWindow(id),
    );
  }

  /// The window filled with [pane]: its whole group when it is in one, so
  /// the tabs stay to switch between and to double-click back, and the pane
  /// on its own otherwise.
  Widget _maximised(BuildContext context, LumitTheme t, PaneId pane,
      [_Edge? edge]) {
    final group = groupOf(widget.root, pane);
    if (group != null) {
      return _buildNode(context, group, edge: edge);
    }
    return _onTop(
      edge,
      _PaneChrome(
        pane: pane,
        activePanel: widget.activePanel,
        drag: _drag,
        onClose: _closePanel,
        header: _bareTitle(t, pane),
        child: widget.buildPanel(context, pane),
      ),
    );
  }

  /// The title line a pane standing alone wears, in every shape: it names the
  /// pane, lifts it, and takes the right-click for its menu. A panel that
  /// draws a header of its own has no second one.
  Widget? _bareTitle(LumitTheme t, PaneId pane) {
    final panel = pane.panel;
    // These four draw a header line of their own.
    if (panel == Panel.viewer ||
        panel == Panel.timeline ||
        panel == Panel.audioTimeline ||
        panel == Panel.cutTimeline) {
      return null;
    }
    return _TitleLine(
      panel: panel.name,
      lifts: true,
      child: Text(
        t.kickerCase(paneTitle(pane)),
        key: const ValueKey('pane-title'),
        style: t.tokens.titleCentred ? t.kickerOn : t.kicker,
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
      ),
    );
  }

  /// [node], told where it stands against a floating window's top edge by
  /// [edge]. A stack's headers each twirl their panel, so a stack in the
  /// corner leaves the window a line of its own.
  Widget _buildNode(BuildContext context, DockNode node, {_Edge? edge}) =>
      switch (node) {
        DockPane pane => _onTop(
            edge,
            _PaneChrome(
              pane: pane.id,
              activePanel: widget.activePanel,
              drag: _drag,
              onClose: _closePanel,
              header: _bareTitle(ThemeScope.of(context).theme, pane.id),
              child: widget.buildPanel(context, pane.id),
            ),
          ),
        DockTabs() when node.stacked => _StackGroup(
            tabs: node,
            buildPanel: widget.buildPanel,
            activePanel: widget.activePanel,
            drag: _drag,
            onClose: _closePanel,
            onChanged: _groupChanged,
            onResized: () => setState(() {}),
          ),
        DockTabs() => _TabGroup(
            tabs: node,
            buildPanel: widget.buildPanel,
            activePanel: widget.activePanel,
            drag: _drag,
            onClose: _closePanel,
            onChanged: _groupChanged,
            edge: edge,
          ),
        DockSplit() => _buildSplit(context, node, edge: edge),
      };

  /// A group changed something of its own: the fronted tab, what is twirled
  /// open, a height, or whether it is a stack at all.
  void _groupChanged() {
    setState(() {});
    widget.onLayoutChanged();
  }

  Widget _buildSplit(BuildContext context, DockSplit split, {_Edge? edge}) {
    final t = ThemeScope.of(context).theme;
    final horizontal = split.axis == DockAxis.horizontal;
    final shown = [for (var i = 0; i < split.children.length; i++) i];
    return LayoutBuilder(
      builder: (context, box) {
        final seams =
            (shown.length - 1) * _Divider.extentFor(t.tokens.tileGap);
        final weights = _givenWay(split, shown,
            (horizontal ? box.maxWidth : box.maxHeight) - seams);
        final children =
            _splitChildren(context, t, split, shown, weights, edge);
        // Every child fills its slot across the split too. A stack with
        // every panel shut is no taller than its headers, and stood in the
        // middle of its slot.
        if (!horizontal) {
          return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: children);
        }
        return Row(
          // The arrangement is the one the user built, in every language:
          // the seam drags and the drop sides below are in screen terms.
          textDirection: TextDirection.ltr,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: children,
        );
      },
    );
  }

  /// The least a child of [split] can be drawn at along the split.
  double _least(DockSplit split, DockNode child) =>
      split.axis == DockAxis.horizontal
          ? dockMinWidth(child)
          : dockMinHeight(child);

  /// What each shown child of a split is drawn at along [room]: its share,
  /// except that a child smaller than its panels' least is grown to it, the
  /// room coming off the sibling with the most to spare above its own. Only
  /// the drawing gives way; the shares the user dragged stay as they were,
  /// and a bigger window puts them back.
  List<double> _givenWay(DockSplit split, List<int> shown, double room) {
    final total = shown.fold<double>(0, (sum, i) => sum + split.shares[i]);
    if (total <= 0 || !room.isFinite || room <= 0) {
      return [for (final i in shown) split.shares[i]];
    }
    final width = [for (final i in shown) split.shares[i] / total * room];
    final least = [for (final i in shown) _least(split, split.children[i])];
    for (var k = 0; k < width.length; k++) {
      var short = least[k] - width[k];
      while (short > 0.5) {
        var giver = -1;
        var spare = 0.5;
        for (var j = 0; j < width.length; j++) {
          if (j != k && width[j] - least[j] > spare) {
            giver = j;
            spare = width[j] - least[j];
          }
        }
        // Nothing left to give: the panel slides inside its pane instead.
        if (giver < 0) break;
        final take = short < spare ? short : spare;
        width[giver] -= take;
        width[k] += take;
        short -= take;
      }
    }
    return width;
  }

  /// A split's children and the seams between them, each child weighted by
  /// [weights] where given, else by its share.
  List<Widget> _splitChildren(BuildContext context, LumitTheme t,
      DockSplit split, List<int> shown, List<double>? weights,
      [_Edge? edge]) {
    final horizontal = split.axis == DockAxis.horizontal;
    // The buttons are in the corner the line ends at: the left one in a
    // right to left language.
    final last = edge?.direction == TextDirection.rtl ? 0 : shown.length - 1;
    final children = <Widget>[];
    for (final (k, i) in shown.indexed) {
      // Side by side, every child runs along the top edge. One above
      // another, only the first does.
      final _Edge? along = edge == null
          ? null
          : horizontal
              ? (
                  host: edge.host,
                  corner: edge.corner && k == last,
                  direction: edge.direction,
                  onMenu: edge.onMenu,
                )
              : k == 0
                  ? edge
                  : null;
      children.add(Expanded(
        // Flex is integer; scale the weight up to keep precision.
        flex: ((weights?[k] ?? split.shares[i]) * 10000)
            .round()
            .clamp(1, 1 << 30),
        child: _buildNode(context, split.children[i], edge: along),
      ));
      if (k < shown.length - 1) {
        final j = shown[k + 1];
        children.add(_Divider(
          horizontal: horizontal,
          gap: t.tokens.tileGap,
          onDrag: (delta, totalExtent) {
            setState(() {
              _resize(split, i, j, horizontal ? delta.dx : delta.dy,
                  totalExtent);
            });
          },
          onDragEnd: widget.onLayoutChanged,
          onReset: (extent) => _equalise(split, i, j, extent),
        ));
      }
    }
    return children;
  }

  /// Move the boundary between child i and child j, the next one drawn, by
  /// `deltaPx` of `totalExtent`.
  void _resize(
      DockSplit split, int i, int j, double deltaPx, double totalExtent) {
    if (totalExtent <= 0) return;
    final total = split.shares.reduce((a, b) => a + b);
    final deltaShare = deltaPx / totalExtent * total;
    const minShare = 0.05;
    final a = split.shares[i] + deltaShare;
    final b = split.shares[j] - deltaShare;
    if (a < minShare || b < minShare) return;
    // **The seam stops at the panels' own minimums.** Every panel declares a
    // minimum, and the dock will not shrink it past that, so the drag is
    // refused rather than clamped: a seam that slid on while the panel
    // underneath stopped moving was how a panel ended up smaller than
    // anything it could draw. Growing one already under its least is fine.
    final shrunk = deltaShare > 0 ? j : i;
    final shrunkPx = (deltaShare > 0 ? b : a) / total * totalExtent;
    if (shrunkPx < _least(split, split.children[shrunk])) return;
    split.shares[i] = a;
    split.shares[j] = b;
  }
}

/// Where one of Lumit's views draws on the screen: its drawing area in the
/// system's units, and how many of those make one logical pixel.
typedef _ViewPlace = ({Rect client, double scale});

/// The live state of a re-dock drag, shared between the dragged source (a tab
/// pill), the ghost pill and every pane's drop preview, in every window.
/// It resolves the hovered pane and drop position by hit-testing the pointer
/// against the pane rects each update, because MouseRegion does not fire while
/// a pointer is captured by a drag.
///
/// The pointer stays with the window the drag began in, wherever it goes:
/// each system keeps a pressed mouse with the window it was pressed in. A pane
/// in another window is found by taking the pointer through the screen, with
/// the places the runner gave for each window as the drag began.
class _DragController extends ChangeNotifier {
  final GlobalKey Function(PaneId) paneKey;
  final GlobalKey Function(PaneId) headerKey;
  final Iterable<PaneId> Function() panes;

  /// Each view's place on the screen, by view id. Filled by [onStart].
  final Map<int, _ViewPlace> places;
  final Future<void> Function() onStart;
  final VoidCallback onGhostShow;
  final VoidCallback onGhostHide;
  final void Function(PaneId dragged, PaneId target, DropPosition pos) onCommit;

  /// Make a window of the dragged pane, at [bounds] in the system's units.
  final void Function(PaneId pane, {Rect? bounds, bool group}) onFloat;

  /// Where the outline of the window a drag would make goes, in the system's
  /// units, while the pointer is over none of Lumit's windows; null when it
  /// is over one, or the drag has ended.
  final void Function(Rect? bounds) onOutside;

  _DragController({
    required this.paneKey,
    required this.headerKey,
    required this.panes,
    required this.places,
    required this.onStart,
    required this.onGhostShow,
    required this.onGhostHide,
    required this.onCommit,
    required this.onFloat,
    required this.onOutside,
  });

  PaneId? dragged;
  LumitTheme? theme;

  /// The view the drag began in, and its size in logical pixels.
  int? view;
  Size viewSize = Size.zero;

  /// The pointer, in logical pixels of the view the drag began in.
  Offset pointer = Offset.zero;

  /// The dragged pane's size on screen, in logical pixels: what a window
  /// made where it is let go is given.
  Size? size;
  PaneId? hoveredPanel;
  DropPosition? dropPosition;

  /// Over none of Lumit's windows: letting go makes a window there.
  bool outside = false;

  /// Held down, the key that makes a window wherever the pane is let go:
  /// Ctrl, and Command on macOS.
  bool detach = false;

  /// A press that may become a drag: ask where every window is now, so the
  /// answer is in before the pointer has travelled far enough to start one.
  void prepare() {
    places.clear();
    _placing = onStart();
  }

  Future<void> _placing = Future<void>.value();

  void start(PaneId pane, Offset localPos, LumitTheme t,
      {int? view, Size viewSize = Size.zero, Size? size}) {
    dragged = pane;
    theme = t;
    this.view = view;
    this.viewSize = viewSize;
    this.size = size;
    pointer = localPos;
    _placing.then((_) {
      if (dragged != pane) return;
      _resolve();
      notifyListeners();
    });
    _resolve();
    onGhostShow();
    notifyListeners();
  }

  void update(Offset localPos) {
    pointer = localPos;
    _resolve();
    onOutside(outside ? newWindowBounds : null);
    notifyListeners();
  }

  void finish() {
    final dragged = this.dragged;
    final target = hoveredPanel;
    final pos = dropPosition;
    final apart = detach || outside;
    final bounds = newWindowBounds;
    _reset();
    onOutside(null);
    onGhostHide();
    notifyListeners();
    if (dragged == null) return;
    if (apart) {
      onFloat(dragged, bounds: bounds);
    } else if (target != null && pos != null) {
      onCommit(dragged, target, pos);
    }
  }

  void cancel() {
    _reset();
    onOutside(null);
    onGhostHide();
    notifyListeners();
  }

  void _reset() {
    dragged = null;
    theme = null;
    hoveredPanel = null;
    dropPosition = null;
    outside = false;
    detach = false;
  }

  /// The pointer on the screen, in the system's units, or null when the
  /// runner has not said where the drag's window is.
  Offset? get screenPoint {
    final place = places[view];
    return place == null ? null : place.client.topLeft + pointer * place.scale;
  }

  /// The pointer in [inView]'s logical pixels, or null when where that view
  /// is is not known.
  Offset? pointerIn(int? inView) {
    if (inView == view) return pointer;
    final place = places[inView];
    final screen = screenPoint;
    if (place == null || screen == null) return null;
    return (screen - place.client.topLeft) / place.scale;
  }

  /// Where a window made of the dragged pane goes, in the system's units:
  /// its title line under the pointer, its panel the size it was. Null when
  /// the screen is not known, and the window goes where a new one would.
  Rect? get newWindowBounds {
    final screen = screenPoint;
    final place = places[view];
    if (screen == null || place == null) return null;
    final s = place.scale;
    final content = size ?? const Size(640, 400);
    final grab = const Offset(48, 14) * s;
    return Rect.fromLTWH(screen.dx - grab.dx, screen.dy - grab.dy,
        content.width * s, (content.height + 32) * s);
  }

  /// Find the pane under the pointer and the drop position within it, in
  /// whichever window it is.
  void _resolve() {
    hoveredPanel = null;
    dropPosition = null;
    outside = false;
    final keys = HardwareKeyboard.instance;
    detach = defaultTargetPlatform == TargetPlatform.macOS
        ? keys.isMetaPressed
        : keys.isControlPressed;
    if (detach) return;
    for (final id in panes()) {
      final hit = _hit(paneKey(id));
      if (hit == null) continue;
      hoveredPanel = id;
      dropPosition = _positionIn(hit.rect, hit.point);
      return;
    }
    // A header in a stack joins the stack, under the panel it belongs to.
    for (final id in panes()) {
      if (_hit(headerKey(id)) == null) continue;
      hoveredPanel = id;
      dropPosition = DropPosition.stack;
      return;
    }
    outside = !_overAWindow();
  }

  /// [key]'s box and the pointer, both in logical pixels of the view the box
  /// is in, when the pointer is over it.
  ({Rect rect, Offset point})? _hit(GlobalKey key) {
    final context = key.currentContext;
    final box = context?.findRenderObject() as RenderBox?;
    if (context == null ||
        box == null ||
        !box.attached ||
        !box.hasSize ||
        // A hidden tab is built (to keep its panel state) but sits under an
        // offstage ancestor. It still resolves a render object and would
        // report a rect overlapping the visible pane, so skip it — only the
        // on-stage pane is a valid drop target.
        !_onStage(box)) {
      return null;
    }
    final point = pointerIn(_viewOf(context));
    if (point == null) return null;
    // Both corners, through every transform on the way: the interface is
    // drawn at a scale, so a box's own size is not its size on screen.
    final rect = MatrixUtils.transformRect(
        box.getTransformTo(null), Offset.zero & box.size);
    return rect.contains(point) ? (rect: rect, point: point) : null;
  }

  /// Whether the pointer is over one of Lumit's windows. With no places from
  /// the runner, only the drag's own window is known.
  bool _overAWindow() {
    final screen = screenPoint;
    if (screen == null) return (Offset.zero & viewSize).contains(pointer);
    return places.values.any((p) => p.client.contains(screen));
  }

  static int? _viewOf(BuildContext context) =>
      context.findAncestorWidgetOfExactType<View>()?.view.viewId;

  /// Whether [box] has no offstage ancestor — i.e. it is actually painted.
  static bool _onStage(RenderObject box) {
    RenderObject? node = box;
    while (node != null) {
      if (node is RenderOffstage && node.offstage) return false;
      final parent = node.parent;
      node = parent is RenderObject ? parent : null;
    }
    return true;
  }

  /// The inner ~50% of both axes is a stack; outside it, the nearest edge
  /// picks a side to split off.
  static DropPosition _positionIn(Rect rect, Offset p) {
    final fx = ((p.dx - rect.left) / rect.width).clamp(0.0, 1.0);
    final fy = ((p.dy - rect.top) / rect.height).clamp(0.0, 1.0);
    if ((fx - 0.5).abs() < 0.25 && (fy - 0.5).abs() < 0.25) {
      return DropPosition.stack;
    }
    final left = fx, right = 1 - fx, top = fy, bottom = 1 - fy;
    final nearest = [left, right, top, bottom].reduce((a, b) => a < b ? a : b);
    if (nearest == left) return DropPosition.left;
    if (nearest == right) return DropPosition.right;
    if (nearest == top) return DropPosition.above;
    return DropPosition.below;
  }
}

/// Turns a press-and-drag on its child into a re-dock drag: a press that
/// travels past the slop starts the drag and drives the controller, leaving a
/// plain tap (a tab click) to the child's own gesture handling. Uses a raw
/// Listener so it stays out of the gesture arena and never fights the tab
/// strip's horizontal scroll.
class _DragSource extends StatefulWidget {
  final PaneId pane;
  final _DragController drag;
  final Widget child;

  const _DragSource({
    required this.pane,
    required this.drag,
    required this.child,
  });

  @override
  State<_DragSource> createState() => _DragSourceState();
}

class _DragSourceState extends State<_DragSource> {
  Offset? _downAt;
  bool _dragging = false;

  /// Theme snapshot taken at pointer-down, when the element is certainly
  /// live. A press elsewhere can rebuild the dock (the active-panel edge)
  /// and deactivate this element while the pointer stays captured, so the
  /// move handler must never look up inherited widgets through `context`.
  LumitTheme? _theme;

  /// The view pressed in and its size, taken at pointer-down for the same
  /// reason.
  int? _view;
  Size _viewSize = Size.zero;

  @override
  Widget build(BuildContext context) {
    return Listener(
      onPointerDown: (e) {
        _downAt = e.position;
        _dragging = false;
        _theme = ThemeScope.of(context).theme;
        widget.drag.prepare();
        final view = context.findAncestorWidgetOfExactType<View>()?.view;
        _view = view?.viewId;
        _viewSize = view == null
            ? Size.zero
            : view.physicalSize / view.devicePixelRatio;
      },
      onPointerMove: (e) {
        final theme = _theme;
        if (_downAt == null || theme == null) return;
        if (!_dragging) {
          if ((e.position - _downAt!).distance < _dragSlop) return;
          _dragging = true;
          widget.drag.start(widget.pane, e.position, theme,
              view: _view, viewSize: _viewSize, size: _paneSize());
        } else {
          widget.drag.update(e.position);
        }
      },
      onPointerUp: (e) {
        if (_dragging) widget.drag.finish();
        _dragging = false;
        _downAt = null;
      },
      onPointerCancel: (e) {
        if (_dragging) widget.drag.cancel();
        _dragging = false;
        _downAt = null;
      },
      child: widget.child,
    );
  }

  /// The dragged pane's size on screen, in logical pixels.
  Size? _paneSize() {
    final box =
        widget.drag.paneKey(widget.pane).currentContext?.findRenderObject();
    if (box is! RenderBox || !box.attached || !box.hasSize) return null;
    return MatrixUtils.transformRect(
            box.getTransformTo(null), Offset.zero & box.size)
        .size;
  }
}

/// The ghost pill that follows the cursor during a drag, on one window's
/// overlay. Each window has one, and draws it only while the pointer is over
/// that window. With the key down that makes a window of it, the window it
/// would make is drawn round the pill.
class _GhostLayer extends StatelessWidget {
  final _DragController drag;

  /// The view this layer is drawn in.
  final int? view;
  const _GhostLayer({required this.drag, required this.view});

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
        animation: drag,
        builder: (context, _) {
          final pane = drag.dragged;
          final t = drag.theme;
          final inView = drag.pointerIn(view);
          if (pane == null || t == null || inView == null) {
            return const SizedBox.shrink();
          }
          // The pointer is a window coordinate and the pill is placed in the
          // overlay's own space; at any UI scale but 100% those differ, and
          // the pill would trail the pointer by the difference.
          final at = overlayLocal(context, inView);
          final size = drag.size;
          // The window it would make, from its corners in the view's own
          // pixels: the same rectangle [newWindowBounds] puts on the screen.
          final corner = inView - const Offset(48, 14);
          final window = size == null
              ? null
              : Rect.fromPoints(
                  overlayLocal(context, corner),
                  overlayLocal(context,
                      corner + Offset(size.width, size.height + 32)));
          return Stack(
            children: [
              if (drag.detach && window != null)
                Positioned.fromRect(
                  rect: window,
                  child: IgnorePointer(
                    child: Entrance(
                      spec: ThemeScope.of(context).motion.dropZone,
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          color: t.accent.withValues(alpha: 0.12),
                          borderRadius:
                              BorderRadius.circular(t.tokens.cardRadius),
                          border: Border.all(color: t.accent),
                        ),
                      ),
                    ),
                  ),
                ),
              Positioned(
                left: at.dx + 10,
                top: at.dy + 8,
                child: IgnorePointer(
                  // Picked up rather than cut in: the pill comes up from a
                  // little small as the tab leaves its strip (docs/15 §8).
                  child: Entrance(
                    spec: ThemeScope.of(context).motion.lift,
                    scale: 0.92,
                    alignment: Alignment.topLeft,
                    child: _GhostPill(title: paneTitle(pane), theme: t),
                  ),
                ),
              ),
            ],
          );
        },
      );
}

/// The floating tab pill, styled like the active pill it was lifted from.
class _GhostPill extends StatelessWidget {
  final String title;
  final LumitTheme theme;
  const _GhostPill({required this.title, required this.theme});

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 3),
      decoration: BoxDecoration(
        color: theme.surface1,
        borderRadius: BorderRadius.circular(theme.tokens.controlRadius),
        border: Border.all(color: theme.accent, width: 1),
        boxShadow: theme.floatShadow,
      ),
      child: Text(theme.kickerCase(title), style: theme.kickerOn),
    );
  }
}

/// The 6px accent dot at the left corner of a card's title line.
///
/// **Decorative and static.** It never blinks, never fills and never means
/// anything. It is not a status light, and no state may be routed through it.
class _HeaderDot extends StatelessWidget {
  final Color colour;
  const _HeaderDot({required this.colour});

  @override
  Widget build(BuildContext context) => Container(
        width: 6,
        height: 6,
        decoration: BoxDecoration(color: colour, shape: BoxShape.circle),
      );
}

class _Divider extends StatefulWidget {
  final bool horizontal;
  final double gap;
  final void Function(Offset delta, double totalExtent) onDrag;

  /// The drag let go, so the new sizes are saved once rather than on every
  /// move.
  final VoidCallback onDragEnd;

  /// Give the two sides the same room again: a double-click, or the seam's
  /// one menu row. Told the split's own extent, as [onDrag] is.
  final void Function(double totalExtent) onReset;

  const _Divider({
    super.key,
    required this.horizontal,
    required this.gap,
    required this.onDrag,
    required this.onDragEnd,
    required this.onReset,
  });

  /// How far apart a seam holds the panes either side: the token's gap, or a
  /// comfortable 7 px to grab where the gap is a hairline.
  static double extentFor(double gap) => gap < 7.0 ? 7.0 : gap;

  @override
  State<_Divider> createState() => _DividerState();
}

class _DividerState extends State<_Divider> {
  bool _hover = false;
  bool _dragging = false;

  /// When the seam was last clicked, for the second click of a double one.
  DateTime? _lastTap;

  /// The split's own width or height: the row or column the seam sits in.
  double _extent() {
    final parent = context.findAncestorRenderObjectOfType<RenderBox>()?.size;
    if (parent == null) return 0;
    return widget.horizontal ? parent.width : parent.height;
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final step =
        _hover || _dragging ? scope.motion.hoverIn : scope.motion.hoverOut;
    // Flush panes (Studio and Desk): hairline-toned gap, brighter on hover
    // and drag. Cards: the room shows between them with no hairline, a
    // hairline on hover, accent while dragging (dock.rs::resize_stroke).
    final sharp = !t.tokens.roomed;
    // Desk's seams are the chassis showing between plates: the ground.
    final idle = sharp
        ? (t.shape == ThemeShape.desk ? t.surface0 : t.surface2)
        : t.room;
    final colour = _dragging
        ? (sharp ? t.textPrimary : t.accent)
        : _hover
            ? (sharp ? t.textPrimary : t.hairlineStrong)
            : idle;
    // The visible gap keeps the token width; the hit area is padded to a
    // comfortable 7 px so a 1 px hairline is still grabbable.
    final hit = _Divider.extentFor(widget.gap);
    // **The whole seam is painted, not just the hairline in it** (owner, desk
    // test). The hit padding is layout, so the panes stand `hit` apart whatever
    // the token says — and only `gap` of that was ever painted, leaving 3 px of
    // nothing either side of the hairline. Nothing composites onto whatever
    // happens to be underneath, and under Sharp that is the dock's `surface_0`
    // — the same value the Graph panel grounds its canvas in, so the seam
    // between the small Viewer and the Node panel read as a slot with the graph
    // showing through it. Every split is this one widget, so every seam had it.
    //
    // The ground goes edge to edge; the hairline stays [gap] wide inside it, so
    // hover and drag still light a hairline rather than the whole band.
    return MouseRegion(
      cursor: widget.horizontal
          ? SystemMouseCursors.resizeColumn
          : SystemMouseCursors.resizeRow,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        // Timed by hand, as a tab's double-click is: a double-tap recogniser
        // keeps a timer running after every click.
        onTap: () {
          final now = DateTime.now();
          final again = _lastTap != null &&
              now.difference(_lastTap!) < const Duration(milliseconds: 350);
          _lastTap = again ? null : now;
          if (again) widget.onReset(_extent());
        },
        onSecondaryTapUp: (d) => showLumitPopup<void>(
          context: context,
          position: d.globalPosition,
          builder: (close) => FloatSurface(
            child: IntrinsicWidth(
              child: MenuRow(
                key: const ValueKey('seam-menu-reset'),
                onPressed: () {
                  close(null);
                  widget.onReset(_extent());
                },
                child: MenuLabel(l10n.resetSplit),
              ),
            ),
          ),
        ),
        onPanStart: (_) => setState(() => _dragging = true),
        onPanEnd: (_) {
          setState(() => _dragging = false);
          widget.onDragEnd();
        },
        onPanCancel: () => setState(() => _dragging = false),
        onPanUpdate: (d) => widget.onDrag(d.delta, _extent()),
        child: Container(
          width: widget.horizontal ? hit : null,
          height: widget.horizontal ? null : hit,
          color: idle,
          child: Center(
            child: AnimatedContainer(
              duration: step.duration,
              curve: step.curve,
              width: widget.horizontal ? widget.gap : null,
              height: widget.horizontal ? null : widget.gap,
              color: colour,
            ),
          ),
        ),
      ),
    );
  }
}

/// The title line a pane wears: a tab group's strip of tabs, or a bare pane's
/// own name. Studio and Desk draw it as a strip of `surface2` above the body
/// (docs/15-DESIGN.md §2.1, "faint surfaces: tab bars, bottom bars, panel
/// headers"), the words at the left. Lantern draws it inside the card on
/// `surface1`, with the accent dot at the left corner and the words centred.
class _TitleLine extends StatelessWidget {
  final Widget child;

  /// The panel it heads, or the fronted one of a group.
  final String panel;

  /// Whether the line lifts its pane, as a bare pane's title does.
  final bool lifts;

  const _TitleLine(
      {required this.panel, required this.child, this.lifts = false});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final roomed = t.tokens.roomed;
    final dot = t.tokens.headerDot;
    return dockTopLine(context, (room) {
      final words = Padding(
        padding: EdgeInsets.symmetric(horizontal: roomed ? 12 : 10) + room,
        child: Row(
          children: [
            if (dot) ...[
              _HeaderDot(colour: t.accent),
              const SizedBox(width: 8),
            ],
            Expanded(
              child: t.tokens.titleCentred
                  ? Center(child: child)
                  : Align(
                      alignment: AlignmentDirectional.centerStart,
                      child: child),
            ),
            // The dot's own width again, so the words centre on the line
            // rather than on what is left of it.
            if (dot) const SizedBox(width: 14),
          ],
        ),
      );
      return Container(
        key: const ValueKey('pane-title-line'),
        height: t.density.headerStrip,
        decoration: dress(
            BoxDecoration(color: roomed ? t.surface1 : t.surface2),
            partStyle(context, t, 'header', inside: panel)),
        child: lifts
            ? DockPaneHandle(child: words)
            : DockWindowGround(child: words),
      );
    });
  }
}

/// The card a roomed pane stands in: `surface1`, the card radius and shadow,
/// clipped. A flush shape hands the child back as it is.
Widget _card(BuildContext context, LumitTheme t, String? panel, Widget child) {
  if (!t.tokens.roomed) return child;
  final style = partStyle(context, t, 'pane', inside: panel);
  final decoration = dress(
    BoxDecoration(
      color: t.surface1,
      borderRadius: BorderRadius.circular(t.tokens.cardRadius),
      boxShadow: t.tokens.cardShadow,
    ),
    style.fill,
  );
  final radius = decoration.borderRadius as BorderRadius?;
  return frosted(
    style,
    radius,
    Container(
      decoration: decoration,
      // Over the content, so the header's fill can't cover it on the curve.
      foregroundDecoration: style.edge(radius),
      clipBehavior: Clip.antiAlias,
      child: insetShadows(style, radius, child),
    ),
  );
}

/// A tab group: the title line of tabs plus the active pane's body. Under
/// Lantern the whole group is one card and the tabs sit on its title line.
class _TabGroup extends StatelessWidget {
  final DockTabs tabs;
  final PanelBuilder buildPanel;
  final VoidCallback onChanged;
  final ValueNotifier<PaneId?> activePanel;
  final _DragController drag;

  /// Take one pane out of the arrangement, for the tab's right-click menu.
  final void Function(PaneId) onClose;

  /// Where it stands against a floating window's top edge, which its strip
  /// of tabs runs along when it does.
  final _Edge? edge;

  const _TabGroup({
    required this.tabs,
    required this.buildPanel,
    required this.onChanged,
    required this.activePanel,
    required this.drag,
    required this.onClose,
    this.edge,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final shown = [for (var i = 0; i < tabs.children.length; i++) i];
    final front = tabs.active.clamp(0, tabs.children.length - 1);
    final active = tabs.children[front];

    return _card(
      context,
      t,
      active.id.panel.name,
      Column(
        children: [
          _onTop(
            edge,
            _TabStrip(
              tabs: tabs,
              shown: shown,
              front: front,
              drag: drag,
              onClose: onClose,
              onChanged: onChanged,
            ),
          ),
          // Two requirements meet here, each once lost to the other. Panel
          // state, scroll offsets, twirl-downs, survives a tab switch, so a
          // hidden tab's subtree stays MOUNTED (the TF round 5 fix, pinned by
          // dock_panel_state_test). And a hidden tab is never BUILT (Airyzz's
          // "dont build invisible panels", restored after a merge overwrote
          // it): not at all before it is first shown, and not again while
          // hidden, _KeepAlivePane returns the same built instance, and an
          // identical child short-circuits Flutter's rebuild, so the dock
          // rebuilding sixty times a second never reaches a hidden panel.
          Expanded(
            child: Stack(
              children: [
                for (final i in shown)
                  _paneBody(context, tabs.children[i].id, i == front),
              ],
            ),
          ),
        ],
      ),
    );
  }

  /// One tab's body. Keyed per panel so reordering the tabs never
  /// cross-matches one panel's State onto another.
  Widget _paneBody(BuildContext context, PaneId pane, bool visible) {
    return KeyedSubtree(
      key: ValueKey(pane),
      child: _KeepAlivePane(
        visible: visible,
        builder: (context) => _PaneChrome(
          pane: pane,
          activePanel: activePanel,
          drag: drag,
          inCard: true,
          child: buildPanel(context, pane),
        ),
      ),
    );
  }
}

/// A tab's width less its name, in each shape: what [_TabPill] puts round the
/// word.
double _tabChrome(LumitTheme t) {
  // The always-there border, one either side.
  const border = 2.0;
  if (t.tokens.actionRadius == ShapeTokens.stadium) {
    return t.tokens.pillInset * 2 + 16 + border;
  }
  // Desk's 10 of margin either side, or the flush shapes' 2 of margin and 8
  // of padding.
  return 20 + border;
}

/// A tab group's strip. A group narrower than its tabs first takes width from
/// the tabs behind the fronted one, each name fading where it is cut, down to
/// its first word and two letters of the next. Past that the strip scrolls:
/// the wheel moves it, and a mark at its end lists every tab of the group.
/// The whole line, its margins included, takes the group's right-click.
class _TabStrip extends StatefulWidget {
  final DockTabs tabs;

  /// The tabs not popped out, by index into the group.
  final List<int> shown;
  final int front;
  final _DragController drag;
  final void Function(PaneId) onClose;
  final VoidCallback onChanged;

  const _TabStrip({
    required this.tabs,
    required this.shown,
    required this.front,
    required this.drag,
    required this.onClose,
    required this.onChanged,
  });

  @override
  State<_TabStrip> createState() => _TabStripState();
}

class _TabStripState extends State<_TabStrip> {
  final ScrollController _scroll = ScrollController();

  /// Whether the strip holds more than it shows, as its last layout found.
  /// Worked out from the names' widths rather than from the scroll view, so
  /// the mark that appears cannot change the answer that put it there.
  bool _cut = false;

  /// Each name's width as measured, by its words and style, so a dock
  /// redrawing every frame measures nothing twice.
  final Map<(String, TextStyle, TextScaler), double> _words = {};

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  /// The plain wheel moves the strip sideways, as it would a list.
  void _wheel(PointerSignalEvent event) {
    if (event is! PointerScrollEvent || !_scroll.hasClients) return;
    final position = _scroll.position;
    if (position.maxScrollExtent <= 0 || event.scrollDelta.dx != 0) return;
    GestureBinding.instance.pointerSignalResolver.register(event, (_) {
      _scroll.jumpTo((position.pixels + event.scrollDelta.dy)
          .clamp(0.0, position.maxScrollExtent));
    });
  }

  void _front(int i) {
    widget.tabs.active = i;
    widget.onChanged();
  }

  double _word(LumitTheme t, TextStyle ambient, int i, TextScaler scaler) =>
      _measure(t, ambient, i, scaler,
          t.kickerCase(paneTitle(widget.tabs.children[i].id)));

  /// The least of tab [i]'s name drawn before the strip scrolls: its first
  /// word and two letters of the next, clear of the fade after them, so no
  /// two tabs cut to the same few letters. A one-word name stays whole.
  double _least(LumitTheme t, TextStyle ambient, int i, TextScaler scaler) {
    final name = t.kickerCase(paneTitle(widget.tabs.children[i].id));
    final parts = name.split(' ');
    if (parts.length < 2) return _word(t, ambient, i, scaler);
    final kept = '${parts[0]} ${parts[1].substring(0, min(2, parts[1].length))}';
    if (kept.length >= name.length) return _word(t, ambient, i, scaler);
    // The fade covers an ellipsis' width at the cut.
    return _measure(t, ambient, i, scaler, '$kept…');
  }

  double _measure(LumitTheme t, TextStyle ambient, int i, TextScaler scaler,
      String words) {
    // Over the ambient style, as the tab's own Text is drawn.
    final style = ambient.merge(i == widget.front ? t.kickerOn : t.kicker);
    return _words.putIfAbsent((words, style, scaler), () {
      final painter = TextPainter(
        text: TextSpan(text: words, style: style),
        textDirection: TextDirection.ltr,
        textScaler: scaler,
        maxLines: 1,
      )..layout();
      final width = painter.width;
      painter.dispose();
      return width;
    });
  }

  /// How wide each tab behind the front one may draw its name, by index, in
  /// [room], and whether even then the strip is cut. The narrowest keep
  /// their names and the rest share what is left alike, none below its
  /// [_least]; empty when every tab fits whole.
  (Map<int, double>, bool) _fit(
      LumitTheme t, TextStyle ambient, double room, TextScaler scaler) {
    final words = {
      for (final i in widget.shown) i: _word(t, ambient, i, scaler)
    };
    final rest = [
      for (final i in widget.shown)
        if (i != widget.front) i
    ]..sort((a, b) => words[a]!.compareTo(words[b]!));
    // A pixel short, so rounding never tips a strip that fits into scrolling.
    var left = room -
        widget.shown.length * _tabChrome(t) -
        (words[widget.front] ?? 0) -
        1;
    if (rest.fold<double>(0, (sum, i) => sum + words[i]!) <= left) {
      return (const {}, false);
    }
    final least = {
      for (final i in rest) i: min(words[i]!, _least(t, ambient, i, scaler))
    };
    if (rest.fold<double>(0, (sum, i) => sum + least[i]!) > left) {
      return (
        {
          for (final i in rest)
            if (words[i]! > least[i]!) i: least[i]!,
        },
        true
      );
    }
    // Each name draws at the cap, or whole if shorter, or its least if
    // longer: the widest cap that fits, found by halving.
    double drawn(int i, double cap) => min(words[i]!, max(least[i]!, cap));
    var lo = 0.0;
    var hi = rest.fold<double>(0, (m, i) => max(m, words[i]!));
    for (var k = 0; k < 20; k++) {
      final cap = (lo + hi) / 2;
      if (rest.fold<double>(0, (sum, i) => sum + drawn(i, cap)) <= left) {
        lo = cap;
      } else {
        hi = cap;
      }
    }
    return (
      {
        for (final i in rest)
          if (words[i]! > drawn(i, lo)) i: drawn(i, lo),
      },
      false
    );
  }

  /// Every tab of the group under the mark, the fronted one ticked.
  void _list(BuildContext context) {
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    showLumitPopup<void>(
      context: context,
      position: box.localToGlobal(Offset(0, box.size.height)),
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: _tabRows(widget.tabs, widget.shown, widget.front,
                () => close(null), _front),
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final active = widget.tabs.children[widget.front];
    final scaler = MediaQuery.textScalerOf(context);
    void menu(BuildContext context, Offset at, PaneId pane) => _showPaneMenu(
          context,
          at,
          pane: pane,
          group: widget.tabs,
          onClose: widget.onClose,
          onGroupChanged: widget.onChanged,
          listTabs: _cut,
        );
    // The empty end of the strip and its margins answer for the fronted
    // tab, so Add panel is there whether or not the pointer finds a tab.
    return GestureDetector(
      behavior: HitTestBehavior.translucent,
      onSecondaryTapUp: (d) => menu(context, d.globalPosition, active.id),
      child: _TitleLine(
        panel: active.id.panel.name,
        child: LayoutBuilder(
          builder: (context, box) {
            final ambient = DefaultTextStyle.of(context).style;
            var (caps, cut) = _fit(t, ambient, box.maxWidth, scaler);
            // Cut even at a few letters a name: the strip scrolls, and the
            // mark that lists the rest takes its room from the strip.
            if (cut) {
              caps = _fit(t, ambient, box.maxWidth - _tabMarkWidth, scaler).$1;
            }
            _cut = cut;
            final strip = Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final i in widget.shown)
                  _TabPill(
                    pane: widget.tabs.children[i].id,
                    title: paneTitle(widget.tabs.children[i].id),
                    active: i == widget.front,
                    word: caps[i],
                    drag: widget.drag,
                    onMenu: menu,
                    onPressed: () => _front(i),
                  ),
              ],
            );
            return SizedBox(
              width: double.infinity,
              child: Align(
                alignment: t.tokens.titleCentred
                    ? Alignment.center
                    : AlignmentDirectional.centerStart,
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Flexible(
                      child: Listener(
                        onPointerSignal: _wheel,
                        child: EdgeFade(
                          // A strip of tabs never stretches at its ends, on
                          // any system.
                          child: ScrollConfiguration(
                            behavior: ScrollConfiguration.of(context)
                                .copyWith(overscroll: false),
                            child: SingleChildScrollView(
                              controller: _scroll,
                              scrollDirection: Axis.horizontal,
                              child: strip,
                            ),
                          ),
                        ),
                      ),
                    ),
                    if (cut)
                      OverflowMark(
                        buttonKey: const ValueKey('dock-tabs-more'),
                        tooltip: l10n.tipDockMoreTabs,
                        width: _tabMarkWidth,
                        height: t.density.headerStrip,
                        onOpen: _list,
                      ),
                  ],
                ),
              ),
            );
          },
        ),
      ),
    );
  }
}

/// The mark at a cut strip's end.
const double _tabMarkWidth = 24;

/// A row for each of [shown], the fronted one ticked, which fronts its tab.
List<Widget> _tabRows(DockTabs tabs, List<int> shown, int front,
        VoidCallback close, void Function(int) pick) =>
    [
      for (final i in shown)
        MenuRow(
          key: ValueKey<String>('dock-tabs-${tabs.children[i].id.panel.name}'),
          onPressed: () {
            close();
            pick(i);
          },
          child: MenuLabel(paneTitle(tabs.children[i].id), ticked: i == front),
        ),
    ];

/// A tab body that is mounted always, built only while visible.
///
/// While visible it rebuilds normally. While hidden it returns the widget
/// instance it last built — Flutter skips rebuilding an identical child, so
/// no build work cascades into hidden panels (their own listeners still fire;
/// that is each panel's business) — and a tab never yet shown builds nothing
/// at all. The subtree stays in the tree offstage with its ticker paused, so
/// State (scroll, twirls, search text) survives the flip either way.
class _KeepAlivePane extends StatefulWidget {
  final bool visible;
  final WidgetBuilder builder;
  const _KeepAlivePane({required this.visible, required this.builder});

  @override
  State<_KeepAlivePane> createState() => _KeepAlivePaneState();
}

class _KeepAlivePaneState extends State<_KeepAlivePane> {
  Widget? _built;

  @override
  Widget build(BuildContext context) {
    if (widget.visible) _built = widget.builder(context);
    final built = _built;
    if (built == null) return const SizedBox.shrink();
    return Offstage(
      offstage: !widget.visible,
      child: TickerMode(enabled: widget.visible, child: built),
    );
  }
}

class _TabPill extends StatefulWidget {
  final PaneId pane;
  final String title;
  final bool active;
  final VoidCallback onPressed;
  final _DragController drag;

  /// How wide the name may draw, where the strip is short of room, or null
  /// for all of it.
  final double? word;

  /// The tab's right-click menu, opened at a point.
  final void Function(BuildContext context, Offset at, PaneId pane) onMenu;

  const _TabPill({
    required this.pane,
    required this.title,
    required this.active,
    required this.onPressed,
    required this.drag,
    required this.onMenu,
    this.word,
  });

  @override
  State<_TabPill> createState() => _TabPillState();
}

class _TabPillState extends State<_TabPill> {
  bool _hover = false;

  /// When the tab was last clicked, for the second click of a double one.
  DateTime? _lastTap;

  /// A click fronts the tab at once, and a second within the double-click
  /// time fills the window with it, or puts the arrangement back. Timed by
  /// hand rather than with a double-tap recogniser, which would hold every
  /// single click back while it waited for a second.
  void _tap() {
    final now = DateTime.now();
    final again = _lastTap != null &&
        now.difference(_lastTap!) < const Duration(milliseconds: 350);
    _lastTap = again ? null : now;
    if (!again) {
      widget.onPressed();
      return;
    }
    final maximised = _DockScope.of(context)?.widget.maximised;
    if (maximised == null) return;
    maximised.value = maximised.value == widget.pane ? null : widget.pane;
  }

  @override
  void initState() {
    super.initState();
    _reveal();
  }

  @override
  void didUpdateWidget(_TabPill old) {
    super.didUpdateWidget(old);
    if (widget.active && !old.active) _reveal();
  }

  /// The fronted tab brings itself into view, so a strip narrower than its
  /// tabs never hides the one that is open.
  void _reveal() {
    if (!widget.active) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) Scrollable.ensureVisible(context, alignment: 0.5);
    });
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    // The fronted tab's fill takes the mark's time. A hover fill comes up
    // quickly and leaves a little slower (docs/15 §8).
    final step = widget.active
        ? scope.motion.mark
        : _hover
            ? scope.motion.hoverIn
            : scope.motion.hoverOut;
    // A filled pill marks the fronted tab only where actions are capsules.
    final round = t.tokens.actionRadius == ShapeTokens.stadium;
    // Desk draws the mockup's tab: a bare lowercase word on the 24 line, the
    // fronted one over a 2px accent rule, no fill and no box on hover.
    final ruled = t.shape == ThemeShape.desk;
    final Color fill;
    final Color textColour;
    if (widget.active) {
      // Lantern fills the fronted pill with `surface2`. **The flush shapes
      // draw no box at all**: the mockups' `.kick.on` is bare text on the
      // strip's own grey, transparent fill, no border, and which tab is
      // fronted reads from the word brightening to `text_primary` alone. It
      // had worn an accent outline, which spends the accent on a resting
      // state and makes the strip's one lit tab look like a control to press.
      fill = round ? t.surface2 : const Color(0x00000000);
      textColour = t.textPrimary;
    } else if (_hover) {
      fill = ruled ? const Color(0x00000000) : t.surface3;
      textColour = t.textPrimary;
    } else {
      fill = const Color(0x00000000);
      textColour = t.textMuted;
    }
    // Always a border, transparent unless hover has something to show: a
    // border insets its child, so letting one appear would shrink the pill by
    // 2 px and shuffle every tab beside it as the pointer crossed the strip.
    final border = Border.all(
      color: _hover && !widget.active && !ruled
          ? t.hairlineStrong
          : const Color(0x00000000),
      width: 1,
    );
    // A panel's name is a container label, so it is a kicker (§7.1): one size,
    // one weight, capitals applied here rather than in the arb file. Which tab
    // is fronted reads from the colour and the accent tick alone — never from
    // a bigger or heavier word, which would shuffle the strip every time the
    // front tab changed.
    final style =
        (widget.active ? t.kickerOn : t.kicker).copyWith(color: textColour);
    final word = widget.word;
    // A name cut short fades where it stops, and says the whole of itself on
    // hover.
    final label = word == null
        ? Text(t.kickerCase(widget.title), style: style)
        : LumitTooltip(
            message: widget.title,
            child: SizedBox(
              width: word,
              child: Text(t.kickerCase(widget.title),
                  style: style,
                  maxLines: 1,
                  softWrap: false,
                  overflow: TextOverflow.fade),
            ),
          );
    // The fill stands off the tab's slot by the shape's pill inset on every
    // side, its corner the outer one less the inset, so the margin is the
    // same the whole way round. Desk's rule spans the word alone, so the word
    // takes the mockup's 10 of air as margin rather than padding, and the
    // box under the rule is the word's own.
    // Only the fill and its edge are eased, so the pill keeps one size and one
    // place while the colour under the word changes.
    final pill = TweenAnimationBuilder<Decoration>(
      tween: DecorationTween(
        end: dress(
          BoxDecoration(
            color: fill,
            borderRadius: BorderRadius.circular(round
                ? t.tokens.actionRadius - t.tokens.pillInset
                : t.tokens.actionRadius),
            border: border,
          ),
          partStyle(context, t, 'tab',
              inside: widget.pane.panel.name,
              states: {if (widget.active) 'on', if (_hover) 'hover'}),
        ),
      ),
      duration: step.duration,
      curve: step.curve,
      child: label,
      builder: (context, decoration, label) => Container(
        key: ValueKey<String>('dock-tab-fill-${widget.pane.panel.name}'),
        margin: round
            ? EdgeInsets.all(t.tokens.pillInset)
            : ruled
                ? const EdgeInsets.symmetric(horizontal: 10)
                : const EdgeInsets.symmetric(horizontal: 2, vertical: 3),
        padding:
            ruled ? EdgeInsets.zero : const EdgeInsets.symmetric(horizontal: 8),
        alignment: Alignment.center,
        decoration: decoration,
        // Painted over the box, so the rule insets nothing and the fronted
        // word sits on the same line as its neighbours.
        foregroundDecoration: ruled && widget.active
            ? BoxDecoration(
                border: Border(bottom: BorderSide(color: t.accent, width: 2)))
            : null,
        child: label,
      ),
    );
    return _DragSource(
      pane: widget.pane,
      drag: widget.drag,
      child: MouseRegion(
        key: ValueKey<String>('dock-tab-${widget.pane.panel.name}'),
        // The arrow, as a tab is on Windows.
        cursor: MouseCursor.defer,
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          onTap: _tap,
          onSecondaryTapUp: (d) =>
              widget.onMenu(context, d.globalPosition, widget.pane),
          // While this pill is the dragged one, it paints nothing but keeps
          // its footprint — egui leaves the gap while the ghost floats free.
          child: AnimatedBuilder(
            animation: widget.drag,
            builder: (context, child) => Opacity(
              opacity: widget.drag.dragged == widget.pane ? 0.0 : 1.0,
              child: child,
            ),
            child: pill,
          ),
        ),
      ),
    );
  }
}

/// A panel's right-click menu, on its tab, its header in a stack, its own
/// title when it stands alone, and the empty end of its strip: the panel's
/// own commands (close, close the others, fill the window), where it lives
/// (a window of its own, its group in one, back in the main window), what to
/// add beside it, how the group it is in is drawn, and the way back to the
/// workspace. A pane standing alone has no group, so those rows go.
void _showPaneMenu(
  BuildContext context,
  Offset position, {
  required PaneId pane,
  required void Function(PaneId) onClose,
  DockTabs? group,
  VoidCallback? onGroupChanged,
  bool listTabs = false,
}) {
  final dock = _DockScope.of(context);
  final root = dock?.widget.root;
  // Null outside the shell (a bare widget test), where there is no workspace
  // to reset.
  final ui = Provider.of<LumitUiState?>(context, listen: false);
  final others = group == null || dock == null
      ? const <PaneId>[]
      : [
          for (final c in group.children)
            if (c.id != pane) c.id,
        ];
  final maximised = dock?.widget.maximised;
  final windows = popOutAvailable && root != null;
  final floating = root == null ? null : windowOf(root, pane);
  // The whole group goes when something is left behind it.
  final groupFloats = windows &&
      group != null &&
      group.children.length > 1 &&
      (floating == null
          ? mainPanesIn(root).length > group.children.length
          : panesIn(floating.root).length > group.children.length);
  // A strip too narrow for its tabs lists them first, as its mark does.
  final shown = !listTabs || group == null
      ? const <int>[]
      : [for (var i = 0; i < group.children.length; i++) i];
  showLumitPopup<void>(
    context: context,
    position: position,
    builder: (close) => FloatSurface(
      child: IntrinsicWidth(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (shown.length > 1) ...[
              ..._tabRows(group!, shown, group.active, () => close(null), (i) {
                group.active = i;
                onGroupChanged?.call();
              }),
              const MenuDivider(),
            ],
            MenuRow(
              key: const ValueKey('tab-menu-close'),
              onPressed: () {
                close(null);
                onClose(pane);
              },
              child: MenuLabel(l10n.closePanel, ticked: false),
            ),
            if (others.isNotEmpty)
              MenuRow(
                key: const ValueKey('tab-menu-close-others'),
                onPressed: () {
                  close(null);
                  dock!._closeOthers(pane, group!);
                },
                child: MenuLabel(l10n.closeOtherPanels, ticked: false),
              ),
            if (maximised != null)
              MenuRow(
                key: const ValueKey('tab-menu-maximise'),
                onPressed: () {
                  close(null);
                  maximised.value = maximised.value == pane ? null : pane;
                },
                child: MenuLabel(l10n.maximisePanel,
                    action: 'panel.maximise',
                    ticked: maximised.value == pane),
              ),
            if (windows) ...[
              const MenuDivider(),
              MenuRow(
                key: const ValueKey('tab-menu-pop-out'),
                // The main window keeps one panel, so its last does not go.
                enabled: canFloat(root, pane),
                onPressed: () {
                  close(null);
                  dock!._float(pane);
                },
                child: MenuLabel(l10n.panelToNewWindow,
                    action: 'panel.float', ticked: false),
              ),
              if (groupFloats)
                MenuRow(
                  key: const ValueKey('tab-menu-pop-out-group'),
                  onPressed: () {
                    close(null);
                    dock!._float(pane, group: true);
                  },
                  child: MenuLabel(l10n.groupToNewWindow, ticked: false),
                ),
              if (floating != null)
                MenuRow(
                  key: const ValueKey('tab-menu-dock'),
                  onPressed: () {
                    close(null);
                    dock!._dockPane(pane);
                  },
                  child: MenuLabel(l10n.panelToMainWindow,
                      action: 'panel.dock', ticked: false),
                ),
              if (floating != null)
                MenuRow(
                  key: const ValueKey('tab-menu-on-top'),
                  onPressed: () {
                    close(null);
                    dock!._setOnTop(floating.id, !floating.onTop);
                  },
                  child: MenuLabel(l10n.windowKeepOnTop,
                      ticked: floating.onTop),
                ),
            ],
            if (dock != null) ...[
              const MenuDivider(),
              SubmenuRow(
                key: const ValueKey('tab-menu-add'),
                closeParent: () => close(null),
                submenu: (dismiss) => _addPanelMenu(dock, pane, dismiss),
                child: MenuLabel(l10n.addPanel, ticked: false),
              ),
            ],
            if (group != null) ...[
              const MenuDivider(),
              MenuRow(
                key: const ValueKey('tab-menu-stacked'),
                onPressed: () {
                  close(null);
                  group.stacked = !group.stacked;
                  // A stack with nothing open is a column of headers, so the
                  // panel that was in front opens with it.
                  if (group.stacked && group.open.isEmpty) {
                    group.setOpen(group.activePane.id, true);
                  }
                  onGroupChanged?.call();
                },
                child: MenuLabel(l10n.panelGroupStacked, ticked: group.stacked),
              ),
              if (group.stacked)
                MenuRow(
                  key: const ValueKey('tab-menu-solo'),
                  onPressed: () {
                    close(null);
                    group.solo = !group.solo;
                    // Turning it on keeps the panel that was clicked and
                    // shuts the rest, which is what the setting now means.
                    if (group.solo && group.isOpen(pane)) {
                      group.setOpen(pane, true);
                    } else if (group.solo && group.open.length > 1) {
                      group.setOpen(group.open.first, true);
                    }
                    onGroupChanged?.call();
                  },
                  child: MenuLabel(l10n.panelGroupSolo, ticked: group.solo),
                ),
            ],
            if (ui != null) ...[
              const MenuDivider(),
              MenuRow(
                key: const ValueKey('tab-menu-reset'),
                onPressed: () {
                  close(null);
                  ui.resetLayout();
                },
                child: MenuLabel(l10n.menuResetWorkspace, ticked: false),
              ),
            ],
          ],
        ),
      ),
    ),
  );
}

/// Every panel in the groups the workspaces fall into: the Edit five, colour,
/// nodes, sound, the Cut timeline, type, and the rest. Window ▸ Panels and
/// Add panel both list them so. An extension's panel is listed by the
/// extension; the debug panel once Help has switched it on in [root].
List<List<Panel>> panelGroups(DockNode root) {
  const named = [
    [
      Panel.project,
      Panel.viewer,
      Panel.timeline,
      Panel.effectControls,
      Panel.effectsAndPresets,
    ],
    [Panel.scopes, Panel.grade],
    [Panel.graph, Panel.node],
    [Panel.mixer, Panel.audio, Panel.audioTimeline],
    [Panel.cutTimeline],
    [Panel.text, Panel.paragraph],
  ];
  final placed = {for (final group in named) ...group, Panel.extension};
  if (!panelVisible(root, Panel.debug)) placed.add(Panel.debug);
  return [
    ...named,
    [
      for (final panel in Panel.values)
        if (!placed.contains(panel)) panel
    ],
  ];
}

/// The panels not on screen, each a row that puts it into [beside]'s group,
/// in Window ▸ Panels' groups. The Viewer is always offered: a second one is
/// another pane of it.
Widget _addPanelMenu(_DockWidgetState dock, PaneId beside, VoidCallback dismiss) {
  final present = panelsIn(dock.widget.root).toSet();
  final groups = [
    for (final group in panelGroups(dock.widget.root))
      [
        for (final panel in group)
          if (panel != Panel.debug &&
              (panel == Panel.viewer || !present.contains(panel)))
            panel,
      ],
  ].where((group) => group.isNotEmpty).toList();
  return FloatSurface(
    width: 220,
    child: Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final (i, group) in groups.indexed) ...[
          if (i > 0) const MenuDivider(),
          for (final panel in group)
            MenuRow(
              key: ValueKey<String>('tab-menu-add-${panel.name}'),
              onPressed: () {
                dismiss();
                dock._addPanel(panel, beside: beside);
              },
              child: MenuLabel(panel.title),
            ),
        ],
      ],
    ),
  );
}

/// The least an open panel in a stack is drawn at, in logical pixels. Below
/// it a divider stops, the same as a seam stops at a panel's own minimum.
const double _stackMinBody = 48;

/// A group drawn as a stack: every panel under its own header, twirled open or
/// shut, the open ones sharing the height between them.
///
/// Every body stays mounted whether it is open or not, for the reason a hidden
/// tab does: a panel's scroll, twirls and half-typed text survive being shut.
class _StackGroup extends StatelessWidget {
  final DockTabs tabs;
  final PanelBuilder buildPanel;
  final VoidCallback onChanged;

  /// A seam is being dragged: redraw, and save when it lets go.
  final VoidCallback onResized;
  final ValueNotifier<PaneId?> activePanel;
  final _DragController drag;
  final void Function(PaneId) onClose;

  const _StackGroup({
    required this.tabs,
    required this.buildPanel,
    required this.onChanged,
    required this.onResized,
    required this.activePanel,
    required this.drag,
    required this.onClose,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final panes = [for (final c in tabs.children) c.id];
    final open = [for (final p in panes) if (tabs.isOpen(p)) p];
    final hit = t.tokens.tileGap < 7.0 ? 7.0 : t.tokens.tileGap;
    final total = open.fold<double>(0, (sum, p) => sum + tabs.shareOf(p));

    return _card(
      context,
      t,
      null,
      Container(
        // What shows under the headers when every panel is shut.
        color: t.surface1,
        child: LayoutBuilder(
          builder: (context, constraints) {
            // What the open panels share: the stack less its headers and the
            // seams between the open ones.
            final shared = (constraints.maxHeight -
                    panes.length * t.density.headerStrip -
                    (open.isEmpty ? 0 : open.length - 1) * hit)
                .clamp(0.0, double.infinity);
            final children = <Widget>[];
            for (final (i, pane) in panes.indexed) {
              final isOpen = tabs.isOpen(pane);
              // A line over the header when an open panel ends right above it
              // with no seam of its own, as a shut header's line does below.
              final above = i == 0 ? null : panes[i - 1];
              final lineAbove = above != null &&
                  tabs.isOpen(above) &&
                  open.indexOf(above) == open.length - 1;
              children.add(_StackHeader(
                key: ValueKey<(String, PaneId)>(('stack-header', pane)),
                pane: pane,
                open: isOpen,
                lineAbove: lineAbove,
                drag: drag,
                onToggle: () {
                  tabs.setOpen(pane, !isOpen);
                  if (!isOpen) {
                    tabs.active = tabs.children.indexWhere((c) => c.id == pane);
                  }
                  onChanged();
                },
                onMenu: (context, at) => _showPaneMenu(
                  context,
                  at,
                  pane: pane,
                  group: tabs,
                  onClose: onClose,
                  onGroupChanged: onChanged,
                ),
              ));
              // Always the same widget in the same slot, open or shut, so the
              // panel's State is never thrown away by a twirl. Shut, it is no
              // height at all and its body is offstage. Clipped to its share,
              // so a list's last row stops at the header below rather than
              // running under it.
              children.add(SizedBox(
                key: ValueKey<PaneId>(pane),
                height: isOpen ? shared * tabs.shareOf(pane) / total : 0,
                child: ClipRect(
                  child: _KeepAlivePane(
                    visible: isOpen,
                    builder: (context) => _PaneChrome(
                      pane: pane,
                      activePanel: activePanel,
                      drag: drag,
                      inCard: true,
                      child: buildPanel(context, pane),
                    ),
                  ),
                ),
              ));
              // A seam under every open panel with another open one below it.
              final next = open.indexOf(pane) + 1;
              if (isOpen && next < open.length) {
                final below = open[next];
                children.add(_Divider(
                  key: ValueKey<(String, PaneId)>(('stack-seam', pane)),
                  horizontal: false,
                  gap: t.tokens.tileGap,
                  onDrag: (delta, _) {
                    if (shared <= 0) return;
                    final move = delta.dy / shared * total;
                    final a = tabs.shareOf(pane) + move;
                    final b = tabs.shareOf(below) - move;
                    final least = _stackMinBody / shared * total;
                    if (a < least || b < least) return;
                    tabs.shares[pane] = a;
                    tabs.shares[below] = b;
                    onResized();
                  },
                  onDragEnd: onChanged,
                  onReset: (_) {
                    final each =
                        (tabs.shareOf(pane) + tabs.shareOf(below)) / 2;
                    tabs.shares[pane] = each;
                    tabs.shares[below] = each;
                    onChanged();
                  },
                ));
              }
            }
            // A stack shorter than its own headers clips the last of them
            // rather than overflowing.
            return ClipRect(
              child: SingleChildScrollView(
                physics: const NeverScrollableScrollPhysics(),
                child: Column(children: children),
              ),
            );
          },
        ),
      ),
    );
  }
}

/// One panel's header in a stack: a twirl and its name. A click twirls it, a
/// drag re-docks it, a right-click opens the panel's menu.
class _StackHeader extends StatefulWidget {
  final PaneId pane;
  final bool open;
  final bool lineAbove;
  final _DragController drag;
  final VoidCallback onToggle;
  final void Function(BuildContext context, Offset position) onMenu;

  const _StackHeader({
    super.key,
    required this.pane,
    required this.open,
    required this.lineAbove,
    required this.drag,
    required this.onToggle,
    required this.onMenu,
  });

  @override
  State<_StackHeader> createState() => _StackHeaderState();
}

class _StackHeaderState extends State<_StackHeader> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ink = widget.open || _hover ? t.textPrimary : t.textMuted;
    final strip = Container(
      key: widget.drag.headerKey(widget.pane),
      height: t.density.headerStrip,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      decoration: BoxDecoration(
        color: t.tokens.roomed ? t.surface1 : t.surface2,
        border: Border(
          top: widget.lineAbove
              ? BorderSide(color: t.hairline)
              : BorderSide.none,
          bottom: BorderSide(color: t.hairline),
        ),
      ),
      child: Row(
        children: [
          lumitIcon(
            widget.open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
            size: iconSize,
            color: ink,
          ),
          const SizedBox(width: 4),
          Expanded(
            child: Text(
              t.kickerCase(paneTitle(widget.pane)),
              style: (widget.open ? t.kickerOn : t.kicker).copyWith(color: ink),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ],
      ),
    );
    return _DragSource(
      pane: widget.pane,
      drag: widget.drag,
      child: MouseRegion(
        key: ValueKey<String>('dock-stack-${widget.pane.panel.name}'),
        cursor: SystemMouseCursors.click,
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: widget.onToggle,
          onSecondaryTapUp: (d) => widget.onMenu(context, d.globalPosition),
          // A panel being dropped on this header is shown by the header
          // lighting, since a shut panel has no body to draw the preview over.
          child: AnimatedBuilder(
            animation: widget.drag,
            builder: (context, child) {
              final target = widget.drag.dragged != null &&
                  widget.drag.dragged != widget.pane &&
                  widget.drag.hoveredPanel == widget.pane &&
                  widget.drag.dropPosition == DropPosition.stack;
              return DecoratedBox(
                position: DecorationPosition.foreground,
                decoration: BoxDecoration(
                  color: t.accent.withValues(alpha: target ? 0.22 : 0),
                ),
                child: child,
              );
            },
            child: strip,
          ),
        ),
      ),
    );
  }
}

/// The pane body chrome: Sharp draws edge-to-edge on `surface1`; Round wraps
/// the content in a rounded, shadowed, padded card (dock.rs::pane_ui). Any
/// click inside makes this the active panel, which wears the accent boundary
/// (Shell::active_panel). A live re-dock drag paints the drop-zone preview
/// over the hovered pane.
class _PaneChrome extends StatelessWidget {
  final PaneId pane;
  final ValueNotifier<PaneId?> activePanel;
  final _DragController drag;
  final Widget child;

  /// The title line drawn above the body, for a pane standing alone.
  final Widget? header;

  /// Whether a tab group's card already wraps this pane, so it draws no card
  /// of its own and its accent boundary rounds only the bottom corners.
  final bool inCard;

  /// Take this pane out of the arrangement, for a pane standing alone. Null
  /// in a group, where the tab or the stack header closes it.
  final void Function(PaneId)? onClose;

  const _PaneChrome({
    required this.pane,
    required this.activePanel,
    required this.drag,
    required this.child,
    this.header,
    this.inCard = false,
    this.onClose,
  });

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final round = t.tokens.roomed && !inCard;
    final onRoom = round && pane.panel == Panel.viewer;
    // Whether this pane is itself a card, with a card's corners and shadow.
    final card = round && !onRoom;
    final corner = Radius.circular(t.tokens.cardRadius);
    final edgeRadius = onRoom
        ? viewerPaneCorners(context, t)
        : round
            ? BorderRadius.all(corner)
            : t.tokens.roomed
                ? BorderRadius.vertical(bottom: corner)
                : null;
    final body = Stack(
      children: [
        // **Nothing paints outside its pane**, step 5 of the
        // degradation ladder: below the panel's declared minimum the
        // content keeps that width and slides. Wrapped here rather
        // than in each panel so a panel that has not thought about
        // narrow widths still cannot overflow, and so there is one
        // place to read the rule.
        PanelFloor(
          minWidth: panelMinWidth(pane.panel),
          // Its own layer, so a panel redrawing every frame while playing
          // repaints itself and not the window around it.
          child: StylePart(
              name: pane.panel.name, child: RepaintBoundary(child: child)),
        ),
        Positioned.fill(child: _DropPreview(pane: pane, drag: drag)),
      ],
    );
    return ValueListenableBuilder<PaneId?>(
      valueListenable: activePanel,
      builder: (context, active, _) {
        // Inside a group's card, the card wears the style's edge.
        final style = onRoom
            ? PartStyle.none
            : partStyle(context, t, 'pane',
                inside: pane.panel.name, states: {if (active == pane) 'active'});
        final edged = inCard && t.tokens.roomed ? style.fill : style;
        return Listener(
        // The whole pane by name, for the tour and the screenshot sweeps.
        key: ValueKey<String>('dock-pane-${pane.panel.name}'),
        // Any press claims focus for this panel, before the content handles
        // the event (the egui edge follows the last click the same way).
        onPointerDown: (_) => activePanel.value = pane,
        child: GestureDetector(
          behavior: HitTestBehavior.translucent,
          // Animated for the boundary alone: the accent edge comes up on the
          // pane that took the click and fades off the one that lost it. The
          // content is the same widget either way, so it is not rebuilt.
          child: frosted(
            edged,
            edged.radius != null
                ? BorderRadius.circular(edged.radius!)
                : card
                    ? BorderRadius.circular(t.tokens.cardRadius)
                    : null,
            AnimatedContainer(
            key: drag.paneKey(pane),
            duration: scope.motion.mark.duration,
            curve: scope.motion.mark.curve,
            decoration: _paneDecoration(context, t, onRoom, card, style),
            // The accent boundary paints over the content's edge, like the
            // egui overlay stroke at Order::Middle. It is ALWAYS supplied — an
            // inactive pane wears a fully transparent border of the same width,
            // so the composed widget chain (the internal DecoratedBox) keeps a
            // constant shape across the active flip. A null-vs-non-null
            // foregroundDecoration would add or remove a layer, forcing Flutter
            // to discard the pane's Element subtree and rebuild it with fresh
            // State — losing scroll offsets and the gesture recogniser the
            // activating pointer-down had just armed (the first-click-drag bug).
            //
            // A style's own edge is drawn here too, for the same reason: over
            // the content, a fill inside can't cover it where the corner curves.
            // The keyboard's home wears a quiet accent edge: at full
            // strength it boxed the Viewer in the Cut arrangement like a
            // selection, and the edge is a mark, not a choice.
            foregroundDecoration: BoxDecoration(
              border: Border.all(
                color: active == pane
                    ? t.accent.withValues(alpha: 0.55)
                    : edged.borderWidth == 0
                        ? t.accent.withValues(alpha: 0)
                        : edged.borderColour ?? t.accent.withValues(alpha: 0),
                width: edged.borderWidth == null || edged.borderWidth == 0
                    ? 1
                    : edged.borderWidth!,
              ),
              borderRadius: edged.radius == null
                  ? edgeRadius
                  : BorderRadius.circular(edged.radius!),
            ),
            padding: round ? EdgeInsets.all(t.tokens.cardPadding) : null,
            // A card is cut to its corners. A pane on the room is not a card:
            // its tiles are, and each has corners of its own. Cutting the
            // pane to the card radius took a second, wider corner off the
            // outer half of the Viewer's pills (a 16 corner off a capsule of
            // 14) and cut their shadows off at the pane's edge.
            clipBehavior: card ? Clip.antiAlias : Clip.none,
            // Always there, alone or in a group, so a pane dragged from one
            // to the other keeps the same shape under its key and its State.
            child: _PaneScope(
              pane: pane,
              drag: drag,
              onClose: onClose,
              child: header == null
                  ? body
                  : Column(children: [header!, Expanded(child: body)]),
            ),
          ),
          ),
        ),
      );
      },
    );
  }
}

extension on _PaneChrome {
  /// The pane's own box, with the style's fill over it. Its edge is drawn
  /// over the content instead. Inside a group's card the card already wears
  /// the style, so the pane only clears its fill.
  BoxDecoration _paneDecoration(BuildContext context, LumitTheme t,
      bool onRoom, bool card, PartStyle style) {
    final own = BoxDecoration(
      color: onRoom ? t.room : t.surface1,
      borderRadius: card ? BorderRadius.circular(t.tokens.cardRadius) : null,
      boxShadow: card ? t.tokens.cardShadow : null,
    );
    if (onRoom) return own;
    if (inCard && t.tokens.roomed) {
      return style.background == null
          ? own
          : own.copyWith(color: const Color(0x00000000));
    }
    return dress(own, style.fill);
  }
}

/// What a pane hands down to its panel, so the panel's own title can lift it.
class _PaneScope extends InheritedWidget {
  final PaneId pane;
  final _DragController drag;

  /// Null in a group, where there is a tab to drag instead.
  final void Function(PaneId)? onClose;

  const _PaneScope({
    required this.pane,
    required this.drag,
    required this.onClose,
    required super.child,
  });

  @override
  bool updateShouldNotify(_PaneScope old) =>
      old.pane != pane || old.drag != drag || old.onClose != onClose;
}

/// The part of a panel's own header that lifts its pane.
///
/// A pane standing alone has no tab, so its title does the tab's job: a drag
/// re-docks it and a right-click offers Close panel. In a tab group or a
/// stack the tab already does both, and this is its child and nothing more.
/// On a floating window's top line the words alone lift the pane, and the
/// empty line round them moves the window.
class DockPaneHandle extends StatelessWidget {
  final Widget child;
  const DockPaneHandle({super.key, required this.child});

  @override
  Widget build(BuildContext context) {
    final scope = context.dependOnInheritedWidgetOfExactType<_PaneScope>();
    final onClose = scope?.onClose;
    if (scope == null || onClose == null) return child;
    final onTop = _WindowTop.of(context) != null;
    final handle = _DragSource(
      pane: scope.pane,
      drag: scope.drag,
      child: GestureDetector(
        // The whole of the handle, so the empty strip beside the word lifts
        // the pane as well as the word does.
        behavior:
            onTop ? HitTestBehavior.deferToChild : HitTestBehavior.opaque,
        onSecondaryTapUp: (d) => _showPaneMenu(
          context,
          d.globalPosition,
          pane: scope.pane,
          onClose: onClose,
        ),
        child: child,
      ),
    );
    return DockWindowGround(child: handle);
  }
}

/// Where a node of a floating window stands against the window's top edge:
/// which window, whether in the corner the window's buttons stand in, the
/// way the window reads, which decides that corner, and the window's menu.
typedef _Edge = ({
  _Host host,
  bool corner,
  TextDirection direction,
  void Function(BuildContext context, Offset at) onMenu,
});

/// Told to the panes along a floating window's top edge, where Lumit draws
/// the window's frame. Their line is the window's top line: the empty part
/// moves the window, as a title bar does, and the pane in the corner shows
/// the window's buttons at its end.
class _WindowTop extends InheritedWidget {
  final _Edge? edge;
  const _WindowTop({required this.edge, required super.child});

  static _Edge? of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<_WindowTop>()?.edge;

  @override
  bool updateShouldNotify(_WindowTop old) =>
      old.edge?.host != edge?.host || old.edge?.corner != edge?.corner;
}

Widget _onTop(_Edge? edge, Widget child) =>
    edge == null ? child : _WindowTop(edge: edge, child: child);

/// [child] with no part in a floating window's top line. A panel showing
/// several pictures side by side, each with lines of its own, puts each in
/// one, and the window keeps a line of its own.
class DockOffTopLine extends StatelessWidget {
  final Widget child;
  const DockOffTopLine({super.key, required this.child});

  @override
  Widget build(BuildContext context) => _WindowTop(edge: null, child: child);
}

/// [child] on a floating window's top line, where its pane heads one: the
/// empty line round it moves the window, a double click maximises it and a
/// right click opens its menu. Elsewhere [child] as it is.
class DockWindowGround extends StatelessWidget {
  final Widget child;
  const DockWindowGround({super.key, required this.child});

  @override
  Widget build(BuildContext context) {
    final edge = _WindowTop.of(context);
    if (edge == null) return child;
    return Stack(fit: StackFit.passthrough, children: [
      Positioned.fill(
        child: WindowDragArea(
            window: edge.host.window.handle, onMenu: edge.onMenu),
      ),
      child,
    ]);
  }
}

/// A top line a panel draws for itself, made by [line] with [room] added to
/// its padding: where the pane heads a floating window's top right corner,
/// the window's buttons stand in that room. Elsewhere there is none.
///
/// The room is on the side the window reads to, which a panel held left to
/// right in every language cannot tell from its own direction.
Widget dockTopLine(
    BuildContext context, Widget Function(EdgeInsets room) line) {
  final edge = _WindowTop.of(context);
  if (edge == null || !edge.corner) return line(EdgeInsets.zero);
  final left = edge.direction == TextDirection.rtl;
  final room = floatingButtonsWidth;
  return Stack(fit: StackFit.passthrough, children: [
    line(left ? EdgeInsets.only(left: room) : EdgeInsets.only(right: room)),
    Positioned(
      top: 0,
      bottom: 0,
      left: left ? 0 : null,
      right: left ? null : 0,
      child: const DockWindowButtons(),
    ),
  ]);
}

/// A floating window's buttons, in the top line of the pane in its top
/// right corner. Where no such line shows them, the window draws a line of
/// its own for them.
class DockWindowButtons extends StatefulWidget {
  const DockWindowButtons({super.key});

  @override
  State<DockWindowButtons> createState() => _DockWindowButtonsState();
}

class _DockWindowButtonsState extends State<DockWindowButtons> {
  /// The window these are counted as shown in.
  _Host? _counted;

  /// Count these as shown in [host]'s window, or in none. After the frame,
  /// since the window's body rebuilds on the count.
  void _count(_Host? host) {
    final from = _counted;
    if (identical(host, from)) return;
    _counted = host;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (from != null) from.buttons.value = (from.buttons.value ?? 1) - 1;
      if (host != null) host.buttons.value = (host.buttons.value ?? 0) + 1;
    });
  }

  @override
  void dispose() {
    _count(null);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final edge = _WindowTop.of(context);
    final corner = edge != null && edge.corner ? edge : null;
    _count(corner?.host);
    if (corner == null) return const SizedBox.shrink();
    return GestureDetector(
      // As on the empty line: a right click opens the window's menu.
      onSecondaryTapUp: (d) => corner.onMenu(context, d.globalPosition),
      child: Directionality(
        textDirection: corner.direction,
        child: FloatingWindowButtons(
            theme: ThemeScope.of(context).theme,
            window: corner.host.window.handle),
      ),
    );
  }
}

/// The translucent accent region shown over the pane the pointer hovers while
/// a re-dock drag is live: the whole pane for a stack, the near half for an
/// edge split.
class _DropPreview extends StatelessWidget {
  final PaneId pane;
  final _DragController drag;
  const _DropPreview({required this.pane, required this.drag});

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
        animation: drag,
        builder: (context, _) {
          if (drag.dragged == null ||
              drag.hoveredPanel != pane ||
              drag.dropPosition == null) {
            return const SizedBox.shrink();
          }
          final scope = ThemeScope.of(context);
          final zone = scope.motion.dropZone;
          return IgnorePointer(
            // Fades in on the pane the pointer has reached, then glides
            // between that pane's five places as the pointer moves over it,
            // so the eye follows one region rather than a series of cuts
            // (docs/15 §8). At *Minimal* it fades in and then cuts from place
            // to place, since nothing travels there; at *None* both are
            // immediate.
            child: Entrance(
              spec: zone,
              child: TweenAnimationBuilder<Rect?>(
                tween: RectTween(end: _dropRegion(drag.dropPosition!)),
                duration:
                    scope.motion.carries ? zone.duration : Duration.zero,
                curve: zone.curve,
                builder: (context, region, _) => CustomPaint(
                  painter: _DropPainter(
                      region: region!, accent: scope.theme.accent),
                ),
              ),
            ),
          );
        },
      );
}

/// Where a drop at [pos] lands, as a fraction of the pane: the whole of it for
/// a stack, the near half for an edge split.
Rect _dropRegion(DropPosition pos) => switch (pos) {
      DropPosition.stack => const Rect.fromLTWH(0, 0, 1, 1),
      DropPosition.left => const Rect.fromLTWH(0, 0, 0.5, 1),
      DropPosition.right => const Rect.fromLTWH(0.5, 0, 0.5, 1),
      DropPosition.above => const Rect.fromLTWH(0, 0, 1, 0.5),
      DropPosition.below => const Rect.fromLTWH(0, 0.5, 1, 0.5),
    };

class _DropPainter extends CustomPainter {
  /// The region to light, as a fraction of the pane ([_dropRegion]).
  final Rect region;
  final Color accent;
  const _DropPainter({required this.region, required this.accent});

  @override
  void paint(Canvas canvas, Size size) {
    final region = Rect.fromLTRB(
      this.region.left * size.width,
      this.region.top * size.height,
      this.region.right * size.width,
      this.region.bottom * size.height,
    );
    canvas.drawRect(region, Paint()..color = accent.withValues(alpha: 0.35));
    canvas.drawRect(
      region,
      Paint()
        ..color = accent
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(_DropPainter old) =>
      old.region != region || old.accent != accent;
}

/// **The bare-pane corner grip is gone** (owner review, 2026-08-24).
///
/// A solo pane used to carry a 16px dot-grid square in its top-right corner
/// (dock.rs::paint_bare_pane_grip), which dragged the pane's panel exactly like
/// a tab. It read as a control on every panel that had one, and on the Viewer it
/// sat over the right-hand end of that panel's own header strip — so the one
/// dock affordance drawn on the picture was also the one covering a picker.
///
/// What carries re-docking now: a **tab pill** drags its panel (`_TabPill`),
/// every pane is still a **drop target** (`_DropPreview`), and Window →
/// Workspace holds the presets, the reset and the per-panel toggles. A pane
/// that is alone in its slot is lifted by its own title (`DockPaneHandle`),
/// which sits in the panel's header strip rather than floating over its content.

/// The dock itself, for the menus of the panes in it: what they close, add
/// and fill the window with.
class _DockScope extends InheritedWidget {
  final _DockWidgetState dock;
  const _DockScope({required this.dock, required super.child});

  static _DockWidgetState? of(BuildContext context) =>
      context.getInheritedWidgetOfExactType<_DockScope>()?.dock;

  @override
  bool updateShouldNotify(_DockScope old) => old.dock != dock;
}

/// A floating window on screen: its native window, and what the dock keeps
/// about it while it is up.
class _Host {
  final int id;
  final PopOutWindow window;

  /// The window's own overlay, which its menus, dialogues and the drag's
  /// pill open in.
  final GlobalKey<OverlayState> overlay = GlobalKey<OverlayState>();

  /// Where it was last put or last seen, so a new arrangement moves it only
  /// when its place differs.
  FloatingFrame? placed;

  /// In front, as last seen.
  bool active = false;

  /// The system's title for it, as last set.
  String? title;

  /// Done once the runner has taken the window on and placed it.
  Future<void> ready = Future<void>.value();

  VoidCallback? listener;
  Timer? _settle;

  /// How many of its panes' top lines show the window's buttons, once its
  /// first frame is laid out. None, and the window draws a line of its own
  /// for them.
  final ValueNotifier<int?> buttons = ValueNotifier<int?>(null);

  _Host(this.id, this.window);

  /// Run [act] once the window has been still for a moment: a drag across
  /// the screen is one move, not a hundred.
  void soon(VoidCallback act) {
    _settle?.cancel();
    _settle = Timer(const Duration(milliseconds: 350), act);
  }

  void dispose() {
    _settle?.cancel();
    final listener = this.listener;
    if (listener != null) window.changes.removeListener(listener);
  }
}

/// The main window's latest notice, at the foot of a floating window that is
/// in front as it is said, since the status line is out of sight there. It
/// goes after a few seconds, or with its close mark, and one that stays waits
/// for the close.
class _WindowNotice extends StatefulWidget {
  final _Host host;
  const _WindowNotice({required this.host});

  @override
  State<_WindowNotice> createState() => _WindowNoticeState();
}

class _WindowNoticeState extends State<_WindowNotice> {
  ValueNotifier<LumitNotice?>? _notices;
  LumitNotice? _shown;
  Timer? _away;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final notices = Provider.of<LumitState?>(context, listen: false)?.notice;
    if (identical(notices, _notices)) return;
    _notices?.removeListener(_said);
    _notices = notices?..addListener(_said);
  }

  @override
  void dispose() {
    _away?.cancel();
    _notices?.removeListener(_said);
    super.dispose();
  }

  void _said() {
    final notice = _notices?.value;
    _away?.cancel();
    if (notice == null || !widget.host.window.isActivated) {
      if (_shown != null) setState(() => _shown = null);
      return;
    }
    setState(() => _shown = notice);
    if (!notice.sticky) {
      _away = Timer(const Duration(seconds: 5), () {
        if (mounted) setState(() => _shown = null);
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final notice = _shown;
    if (notice == null) return const SizedBox.shrink();
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    return Positioned(
      left: 12,
      right: 12,
      bottom: 12,
      child: Align(
        alignment: AlignmentDirectional.bottomStart,
        child: Entrance(
          key: ObjectKey(notice),
          spec: scope.motion.reveal,
          rise: scope.motion.revealRise,
          child: FloatSurface(
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Flexible(
                    child: GestureDetector(
                      onTap: notice.onTap,
                      child: Text(
                        notice.message,
                        key: const ValueKey('floating-notice'),
                        style: t.small.copyWith(
                            color: notice.error ? t.warning : t.textPrimary),
                        overflow: TextOverflow.ellipsis,
                      ),
                    ),
                  ),
                  if (notice.action case final action?) ...[
                    const SizedBox(width: 4),
                    HouseButton(
                      small: true,
                      frameless: true,
                      onPressed: notice.onTap,
                      child: Text(action, style: t.small),
                    ),
                  ],
                  const SizedBox(width: 4),
                  HouseButton(
                    key: const ValueKey('floating-notice-close'),
                    small: true,
                    frameless: true,
                    onPressed: () => setState(() => _shown = null),
                    child:
                        Text('×', style: t.small.copyWith(color: t.textMuted)),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A floating window's content: the window's arrangement, drawn by the same
/// dock as the main window's.
///
/// It sits in the tree under the dock, so its panels see the same state and
/// theme they do there. Its window is drawn apart from the main one, though,
/// so it brings its own scale, and its own overlay for menus, dialogues and
/// tooltips to open in.
class _FloatingWindowView extends StatefulWidget {
  final _DockWidgetState dock;
  final DockWindow window;
  final _Host host;

  const _FloatingWindowView({
    required this.dock,
    required this.window,
    required this.host,
  });

  @override
  State<_FloatingWindowView> createState() => _FloatingWindowViewState();
}

class _FloatingWindowViewState extends State<_FloatingWindowView> {
  late final OverlayEntry _body = OverlayEntry(
      builder: (context) =>
          widget.dock._floatingBody(context, widget.window, widget.host));

  @override
  void didUpdateWidget(_FloatingWindowView old) {
    super.didUpdateWidget(old);
    // The dock rebuilt: the entry is not a child of this widget, so it is
    // told to build again rather than finding out.
    _body.markNeedsBuild();
  }

  @override
  Widget build(BuildContext context) => _DockScope(
        dock: widget.dock,
        child: UiScaleView(
          scale: widget.dock.widget.uiScale,
          child: Overlay(key: widget.host.overlay, initialEntries: [_body]),
        ),
      );
}
