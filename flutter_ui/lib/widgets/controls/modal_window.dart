// The modal window: the centred, movable, resizable surface a dialogue opens
// on, and the count that tells the panels to stand down while one is up.

import 'dart:async';

import 'package:flutter/material.dart';

import '../../state/workspace.dart';
import '../escape_ladder.dart';
import 'base.dart';

/// Show a positioned popup and complete with the value handed to `close`.
/// Clicking outside, or pressing Escape, dismisses with null.
/// A centred modal on the app Overlay, with a dimmed click-to-dismiss backdrop.
/// Completes with whatever `close` was given, or null when dismissed.
///
/// **Escape closes it from the ladder's dialogue rung** (K-319, K-575). It was
/// Flutter's own `DismissIntent` for a while, which did dismiss the window but
/// could not be ordered against anything: the focus path runs whatever the
/// hardware-keyboard handlers returned, so a drag being abandoned inside the
/// window shut the window as well. A claim on the ladder is the same dismissal
/// with a place in the queue. Closing with null is what a click on the scrim
/// gives, too.
///
/// The value-returning sibling of [showLumitPopup]. `dialogs.dart` has a private
/// `_showModal` that returns nothing, which is fine for a dialog that commits
/// through a callback but not for one whose caller needs to know whether anything
/// was applied — hence this, in the house-controls file where both can reach it.
///
/// The window is **movable** — dragging anywhere on it that no control claims
/// moves it — and, when [initialSize] is given, **resizable** from the grip in
/// its bottom-right corner. Give an [id] and where it was left is remembered in
/// the workspace store, so it opens where it was last put, this session and the
/// next (K-242). Windows without an id always open centred at their natural
/// size, which is what a one-question confirmation wants.
Future<T?> showLumitModal<T>({
  required BuildContext context,
  required Widget Function(void Function(T?) close) builder,
  String? id,
  Size? initialSize,
  Size minSize = const Size(320, 240),
}) {
  final overlay = Overlay.of(context);
  final completer = Completer<T?>();
  late OverlayEntry entry;
  void close(T? v) {
    if (completer.isCompleted) return;
    completer.complete(v);
    entry.remove();
  }

  entry = OverlayEntry(
    builder: (_) => Stack(
      children: [
        Positioned.fill(
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () => close(null),
            child: ColoredBox(
              color: ThemeScope.of(context).theme.scrim,
            ),
          ),
        ),
        _MovableWindow(
          id: id,
          initialSize: initialSize,
          minSize: minSize,
          onDismiss: () => close(null),
          child: builder(close),
        ),
      ],
    ),
  );
  overlay.insert(entry);
  return completer.future;
}

/// Where movable windows remember being left. The shell points this at the one
/// [Workspace] it loaded at start-up; it is null in a widget test, which simply
/// means a window there opens centred and forgets where it was dragged.
Workspace? modalPlacementStore;

int _openModals = 0;

/// Whether a modal window is up (K-243).
///
/// The panels register their keyboard commands on the hardware keyboard rather
/// than holding focus, so nothing about a dialogue being open stopped them
/// hearing a keypress meant for it: `Enter` in the Pre-compose dialogue was
/// also `Enter` in the Timeline, and renamed a layer behind the window instead
/// of pressing the button in front of it. A panel command is about the panel,
/// and while a modal is up the panel is not what is being used.
///
/// Counted by the windows themselves as they mount and unmount, rather than by
/// the open and close calls: a window can also leave by having the tree taken
/// down under it, and a count only the close path decremented would stick above
/// zero and leave the keyboard dead for the rest of the session.
bool get lumitModalOpen => _openModals > 0;

/// For a modal surface that is not a [_MovableWindow] — the FX console
/// (K-328) is the one today. Counted from `initState` and `dispose`, for the
/// reason the window count is: a surface can leave by having the tree taken
/// down under it.
void markModalMounted() => _openModals++;
void markModalUnmounted() => _openModals--;

/// A window that can be dragged around the app window and, when it has a size,
/// resized from its bottom-right corner.
///
/// It sits at the centre and carries an *offset* from there rather than an
/// absolute position: that way it needs to know nothing about how big it is to
/// open centred, and a placement saved on one monitor still opens on screen on
/// another. The offset is clamped so the middle of the window can never leave
/// the app window — drag it as far as you like, it is always grabbable again.
class _MovableWindow extends StatefulWidget {
  final String? id;
  final Size? initialSize;
  final Size minSize;
  final Widget child;

  /// What Escape means here, or null for a window that cannot be dismissed.
  final VoidCallback? onDismiss;

  const _MovableWindow({
    required this.id,
    required this.initialSize,
    required this.minSize,
    required this.child,
    this.onDismiss,
  });

  @override
  State<_MovableWindow> createState() => _MovableWindowState();
}

class _MovableWindowState extends State<_MovableWindow> {
  Offset _offset = Offset.zero;
  Size? _size;

  @override
  void initState() {
    super.initState();
    _openModals++;
    final dismiss = widget.onDismiss;
    if (dismiss != null) {
      _escapeRelease = EscapeLadder.register(EscapeRung.dialog, () {
        dismiss();
        return true;
      });
    }
    _size = widget.initialSize;
    final id = widget.id;
    final saved = id == null ? null : modalPlacementStore?.windowPlacements[id];
    if (saved != null) {
      _offset = saved.offset;
      // A fixed-size window keeps its natural size however big it was when the
      // placement was written — only a resizable one takes a size back.
      if (widget.initialSize != null && saved.size != null) _size = saved.size;
    }
  }

  @override
  void dispose() {
    _openModals--;
    _escapeRelease?.call();
    _escapeRelease = null;
    super.dispose();
  }

  /// How to stand down from the ladder. Held here rather than beside the call
  /// that opened the window, for the reason the modal count is: a window can
  /// leave by having the tree taken down under it, and a claim only the close
  /// path released would go on eating Escape for the rest of the session.
  VoidCallback? _escapeRelease;

  void _remember() {
    final id = widget.id;
    if (id == null) return;
    modalPlacementStore?.rememberWindow(id, WindowPlacement(_offset, _size));
  }

  /// Keep the middle of the window inside the app window, so however far it is
  /// dragged there is always something left to grab.
  Offset _clampOffset(Offset o, BoxConstraints box) => Offset(
        o.dx.clamp(-box.maxWidth / 2, box.maxWidth / 2),
        o.dy.clamp(-box.maxHeight / 2, box.maxHeight / 2),
      );

  Size? _clampSize(Size? s, BoxConstraints box) => s == null
      ? null
      : Size(
          s.width.clamp(widget.minSize.width, box.maxWidth),
          s.height.clamp(widget.minSize.height, box.maxHeight),
        );

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LayoutBuilder(
      builder: (context, box) {
        // The gesture handlers accumulate onto the *state*, never onto these:
        // several pointer moves can arrive between two frames, and every one of
        // them would read the same stale value from this build and the window
        // would move a fraction of the distance dragged.
        final offset = _clampOffset(_offset, box);
        final size = _clampSize(_size, box);

        return Center(
          child: Transform.translate(
            offset: offset,
            // The grip is a *sibling* of the window, not something inside it.
            // Nested drag detectors both join the gesture arena for a pointer
            // that lands on the inner one and neither ends up moving anything;
            // as siblings the topmost — the grip — takes the corner and the
            // window takes everywhere else.
            child: Stack(
              children: [
                GestureDetector(
                  // Anything with its own drag — a slider, a scrolling list, a
                  // text selection — wins the gesture over this, so dragging a
                  // control still does what the control does and dragging the
                  // window's own chrome moves the window.
                  onPanUpdate: (d) => setState(
                    () => _offset = _clampOffset(_offset + d.delta, box),
                  ),
                  onPanEnd: (_) => _remember(),
                  child: SizedBox(
                    width: size?.width,
                    height: size?.height,
                    // Its own focus scope, so Tab cycles inside the window
                    // rather than wandering into the panels behind it, and
                    // reading order — left to right, then top to bottom —
                    // rather than widget-tree order, which nests columns
                    // inside rows and visits them in whatever order the
                    // layout code happened to compose them (K-319).
                    child: FocusScope(
                      child: FocusTraversalGroup(
                        policy: ReadingOrderTraversalPolicy(),
                        child: widget.child,
                      ),
                    ),
                  ),
                ),
                if (size != null)
                  Positioned(
                    right: 0,
                    bottom: 0,
                    child: MouseRegion(
                      cursor: SystemMouseCursors.resizeDownRight,
                      child: GestureDetector(
                        key: const ValueKey('window-resize-grip'),
                        behavior: HitTestBehavior.opaque,
                        onPanUpdate: (d) => setState(() {
                          final was = _clampSize(_size, box)!;
                          final now = _clampSize(
                            Size(
                              was.width + d.delta.dx,
                              was.height + d.delta.dy,
                            ),
                            box,
                          )!;
                          // The window is anchored at its centre, so growing it
                          // by one pixel to the right means moving it half a
                          // pixel right for the left edge to stay put.
                          _offset = _clampOffset(
                            _offset +
                                Offset((now.width - was.width) / 2,
                                    (now.height - was.height) / 2),
                            box,
                          );
                          _size = now;
                        }),
                        onPanEnd: (_) => _remember(),
                        child: CustomPaint(
                          size: const Size(14, 14),
                          painter: _GripPainter(t.hairline),
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
        );
      },
    );
  }
}

/// The three short diagonals that say "drag this corner".
class _GripPainter extends CustomPainter {
  final Color color;
  const _GripPainter(this.color);

  @override
  void paint(Canvas canvas, Size size) {
    final p = Paint()
      ..color = color
      ..strokeWidth = 1
      ..strokeCap = StrokeCap.round;
    for (final inset in [3.0, 6.5, 10.0]) {
      canvas.drawLine(
        Offset(size.width - 2, size.height - inset),
        Offset(size.width - inset, size.height - 2),
        p,
      );
    }
  }

  @override
  bool shouldRepaint(_GripPainter old) => old.color != color;
}
