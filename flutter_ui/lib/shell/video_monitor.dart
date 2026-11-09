// The video monitor: the Viewer's picture alone, filling a display of its
// own, for a client to watch on a second screen.
//
// It draws the same textures the Viewer does, so it plays in step with it and
// costs no render of its own. It follows the view being worked in, and shows a
// wipe or a split when that view's pane is comparing two. Nothing else of the
// Viewer comes with it: no bars, rulers, guides, handles or tags.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../panels/viewer_compare.dart' show ViewerWipeClip;
import '../panels/viewer_picture.dart'
    if (dart.library.js_interop) '../panels/viewer_picture_web.dart';
import '../panels/viewer_stage.dart' show pictureChannelFilter;
import '../state/dock.dart' show PaneId;
import '../state/ui_state.dart';
import '../state/viewer_views.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import 'menu_bar_frb.dart' show MenuEntry;
import 'pop_out.dart';

/// Opens and closes the video monitor's window as
/// [LumitUiState.videoMonitor] says, around [child].
class VideoMonitorHost extends StatefulWidget {
  final Widget child;

  const VideoMonitorHost({super.key, required this.child});

  @override
  State<VideoMonitorHost> createState() => _VideoMonitorHostState();
}

class _VideoMonitorHostState extends State<VideoMonitorHost> {
  late final LumitUiState _ui = context.read<LumitUiState>();
  PopOutWindow? _window;

  /// The display the window was last put on.
  Rect? _placed;
  bool _opening = false;
  VoidCallback? _escapeRelease;

  @override
  void initState() {
    super.initState();
    _ui.videoMonitor.addListener(_sync);
    // The monitor covers a whole display, so Escape takes it down before it
    // reaches the selection.
    _escapeRelease = EscapeLadder.register(EscapeRung.dialog, () {
      if (_ui.videoMonitor.value == null) return false;
      _ui.setVideoMonitor(null);
      return true;
    });
    // The displays may have brought it back before this was built.
    WidgetsBinding.instance.addPostFrameCallback((_) => _sync());
  }

  @override
  void dispose() {
    _escapeRelease?.call();
    _ui.videoMonitor.removeListener(_sync);
    final window = _window;
    if (window != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) => window.close());
    }
    super.dispose();
  }

  void _sync() {
    if (!mounted || !popOutAvailable) return;
    final display = _ui.videoMonitor.value;
    final window = _window;
    if (display == null) {
      if (window == null) return;
      // The view leaves the tree first, and the window goes after it.
      setState(() => _window = null);
      _placed = null;
      WidgetsBinding.instance.addPostFrameCallback((_) => window.close());
      return;
    }
    if (window == null) {
      if (_opening) return;
      _opening = true;
      // Never inside a frame: making a window draws its first frame there
      // and then.
      Timer.run(_open);
      return;
    }
    if (display == _placed) return;
    _placed = display;
    if (window.handle != 0) NativeWindows.monitor(window.handle, display);
  }

  void _open() {
    _opening = false;
    final display = _ui.videoMonitor.value;
    if (!mounted || display == null || _window != null) return;
    final scale = [
      for (final d in _ui.displays.value)
        if (d.bounds == display) d.scale,
    ].firstOrNull;
    PopOutWindow? made;
    made = PopOutWindow.open(
      title: l10n.videoMonitorTitle,
      size: display.size / (scale ?? 1.0),
      // Closed by the system, or from the taskbar: the monitor is off.
      onClosed: () {
        if (made != null && identical(made, _window)) {
          _ui.setVideoMonitor(null);
        }
      },
    );
    final window = made;
    if (window == null) {
      _ui.setVideoMonitor(null);
      return;
    }
    setState(() => _window = window);
    _placed = display;
    _place(window);
  }

  Future<void> _place(PopOutWindow window) async {
    final handle = window.handle;
    if (handle == 0) return;
    // Quiet: a video monitor never takes the keyboard, not even as it opens.
    await NativeWindows.adopt(handle,
        ownTitle: false, onTop: false, quiet: true);
    final display = _placed;
    if (!identical(window, _window) || display == null) return;
    await NativeWindows.monitor(handle, display);
  }

  @override
  Widget build(BuildContext context) => ViewAnchor(
        view: _window?.view(const _MonitorPicture()),
        child: widget.child,
      );
}

/// What the monitor shows: the active view's picture on black, or the two
/// sides of its pane's compare.
class _MonitorPicture extends StatefulWidget {
  const _MonitorPicture();

  @override
  State<_MonitorPicture> createState() => _MonitorPictureState();
}

class _MonitorPictureState extends State<_MonitorPicture> {
  /// Each composition's size, as the Viewer asks for it, held until the
  /// document changes so a rebuild asks the engine nothing.
  final Map<String, Size?> _sizes = {};
  int? _sizesAt;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = context.read<LumitUiState>();
    return MouseRegion(
      cursor: SystemMouseCursors.none,
      child: ColoredBox(
        color: t.videoMonitorGround,
        // The engine's answers arrive on the state, and a view fronted,
        // laid out or compared on the views.
        child: ListenableBuilder(
          listenable: Listenable.merge([ui, ui.views]),
          builder: (context, _) => _content(t, ui),
        ),
      ),
    );
  }

  Widget _content(LumitTheme t, LumitUiState ui) {
    final views = ui.views;
    final active = views.active;
    if (active == null) return const SizedBox.expand();
    PaneId? pane;
    for (final e in views.paneViews.entries) {
      if (e.value.contains(active.id)) pane = e.key;
    }
    final mode = pane == null ? CompareMode.none : views.compareOf(pane);
    final pair = [
      for (final id in views.paneViews[pane] ?? const <String>[])
        if (views.byId(id) case final view?) view,
    ];
    // A snapshot lives in the Viewer panel, so its compare shows the live
    // picture alone.
    if (pane == null || !mode.pairsViews || pair.length < 2) {
      return _fitted(ui, active);
    }
    final at = views.dividerOf(pane);
    final swapped = views.paneSwapped.contains(pane);
    if (mode == CompareMode.split) {
      final (first, second) =
          swapped ? (pair[1], pair[0]) : (pair[0], pair[1]);
      return LayoutBuilder(builder: (context, box) {
        final x = box.maxWidth * at;
        return Stack(children: [
          Positioned(
              left: 0, top: 0, bottom: 0, width: x, child: _fitted(ui, first)),
          Positioned(
            left: x,
            top: 0,
            bottom: 0,
            width: box.maxWidth - x,
            child: _fitted(ui, second),
          ),
        ]);
      });
    }
    // A wipe: both in the first one's frame, so the two are in register.
    return _fitted(
      ui,
      pair[0],
      LayoutBuilder(builder: (context, box) {
        final x = box.maxWidth * at;
        return Stack(fit: StackFit.expand, children: [
          _picture(ui, pair[0]),
          ClipRect(
            clipper: ViewerWipeClip(x, swapped: swapped),
            child: _fitted(ui, pair[1]),
          ),
          Positioned(
            left: x - 0.5,
            top: 0,
            bottom: 0,
            width: 1,
            child: ColoredBox(color: t.hairline),
          ),
        ]);
      }),
    );
  }

  /// [view]'s picture, or [child] in its place, as large as fits and in the
  /// picture's own shape.
  Widget _fitted(LumitUiState ui, ViewerSurface view, [Widget? child]) {
    final size = _sizeOf(ui, view);
    if (size == null) return const SizedBox.expand();
    return Center(
      child: AspectRatio(
        aspectRatio: size.width / size.height,
        child: child ?? _picture(ui, view),
      ),
    );
  }

  /// The texture [view] is drawing, in the channel it is shown in. Listened
  /// to rather than kept, because the engine can hand the view a new one.
  Widget _picture(LumitUiState ui, ViewerSurface view) =>
      ValueListenableBuilder<int?>(
        valueListenable: ui.textureOf(view.engineId),
        builder: (context, id, _) => id == null
            ? const SizedBox.expand()
            : pictureChannelFilter(
                view.mode == ViewMode.footage
                    ? view.channel
                    : ui.views.optionsFor(view).channel,
                viewerPicture(id)),
      );

  /// The picture's size as the Viewer has it: the file's for a footage view,
  /// the canvas for a 3D viewport looking through a view of its own, and the
  /// composition's otherwise. Null while it is not known.
  Size? _sizeOf(LumitUiState ui, ViewerSurface view) {
    if (view.mode == ViewMode.footage) {
      final footage = ui.footageOf(view);
      final facts = footage == null ? null : ui.itemFacts(footage);
      if (facts == null || facts.width <= 0 || facts.height <= 0) return null;
      return Size(facts.width.toDouble(), facts.height.toDouble());
    }
    if (ui.isViewport(view.engineId) && ui.viewPoseOf(view.engineId) != null) {
      final (w, h) = view.canvas;
      if (w > 0 && h > 0) return Size(w.toDouble(), h.toDouble());
    }
    final comp = ui.compFor(view) ?? ui.selectedComp;
    if (comp == null) return null;
    final revision = ui.model.heldRevision;
    if (revision != _sizesAt) {
      _sizes.clear();
      _sizesAt = revision;
    }
    return _sizes.putIfAbsent('${comp.internalid}', () {
      try {
        final size = comp.getSize();
        if (size.width == 0 || size.height == 0) return null;
        return Size(size.width.toDouble(), size.height.toDouble());
      } catch (_) {
        // Deleted a moment ago.
        return null;
      }
    });
  }
}

/// The Window menu's Video monitor rows: Off, then each display, the one it
/// is on ticked.
List<MenuEntry> videoMonitorMenu(LumitUiState ui) {
  final on = ui.videoMonitor.value;
  final rows = [
    MenuEntry(l10n.videoMonitorOff, () => ui.setVideoMonitor(null),
        checked: on == null),
  ];
  // Two displays of the same make share a name, so the later ones are
  // numbered.
  final seen = <String, int>{};
  for (final (i, d) in ui.displays.value.indexed) {
    final name = d.name.isEmpty ? l10n.displayNumbered(i + 1) : d.name;
    final count = seen[name] = (seen[name] ?? 0) + 1;
    rows.add(MenuEntry(count == 1 ? name : '$name $count',
        () => ui.setVideoMonitor(d.bounds),
        checked: d.bounds == on));
  }
  return rows;
}
