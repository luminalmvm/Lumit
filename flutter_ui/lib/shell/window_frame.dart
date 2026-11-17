// The window's own buttons, and the handle it is moved by.
//
// # In plain terms
//
// On Windows Lumit's window has no title bar, and on Linux it has none once
// that is switched on in Settings. The menu bar is its top line, the three
// buttons a title bar would hold stand at that line's end (its left end in a
// right to left language, as Windows puts them), and the empty part of the
// line moves the window the way a title bar does. The
// runner does the moving and the closing (windows/runner/flutter_window.cpp,
// linux/runner/my_application.cc); this file draws the buttons and says what
// was pressed.
//
// The buttons are laid over the whole window rather than put in the menu bar,
// because the menu bar is not always there: the splash, the welcome screen and
// a start-up that failed are each the whole window, and a window with no way
// to close it is the worse fault.
//
// A floating window has no menu bar. The line its panels head it with is its
// top line instead, and the buttons stand at the end of the one in its top
// right corner (dock_widget.dart).
//
// macOS keeps its title bar. Its menus are in the system's bar, so Lumit has
// no line of its own there to put the buttons in.

import 'dart:io' hide Platform;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../icons/lumit_icon.dart';
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/background.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'package:lumit_flutter/state/platform.dart'
    if (dart.library.js_interop) 'package:lumit_flutter/state/platform_web.dart';

const MethodChannel _channel = MethodChannel('lumit/window');

/// Whether Lumit draws the window's buttons itself here. Asked of the target
/// platform, so a widget test draws none unless it says it is a desktop. In a
/// browser the window is the browser's.
bool get windowFrameDrawn =>
    !kIsWeb &&
    (defaultTargetPlatform == TargetPlatform.windows ||
        (defaultTargetPlatform == TargetPlatform.linux &&
            _startedWithOwnTitleBar));

/// On Linux, the file whose being there turns Lumit's own title bar on. A file
/// rather than a setting in the store, because the runner reads it before the
/// window is made and has nothing to read the store with.
File get _ownTitleBarFile => File('${Workspace.storeFile().parent.path}'
    '${Platform.pathSeparator}own-title-bar');

/// Whether this run started with Lumit's own title bar. Read once, since the
/// runner settled it when the window was made and a change waits for the next
/// start.
final bool _startedWithOwnTitleBar = _ownTitleBarFile.existsSync();

/// Whether Lumit opens with its own title bar on Linux, which Settings shows
/// and switches.
bool get linuxOwnTitleBar => _ownTitleBarFile.existsSync();

set linuxOwnTitleBar(bool on) {
  try {
    if (on) {
      _ownTitleBarFile.createSync(recursive: true);
    } else {
      _ownTitleBarFile.deleteSync();
    }
  } catch (_) {
    // A folder that would not take the file leaves the title bar as it was.
  }
}

/// One button's width, and so a third of what the menu bar leaves clear at its
/// right end.
const double _buttonWidth = 40;

/// The room the three buttons take at the right end of the top line, or none
/// where the system draws them.
double get windowButtonsWidth => windowFrameDrawn ? _buttonWidth * 3 : 0;

/// The same for a floating window, which has no minimise button.
double get floatingButtonsWidth => windowFrameDrawn ? _buttonWidth * 2 : 0;

/// Whether the main window is maximised, which decides the middle button's
/// drawing.
final ValueNotifier<bool> _maximised = ValueNotifier<bool>(false);

/// The same for each floating window, by its native handle.
final Map<int, ValueNotifier<bool>> _floatingMaximised = {};

ValueNotifier<bool> _maximisedOf(int? window) => window == null
    ? _maximised
    : _floatingMaximised.putIfAbsent(window, () => ValueNotifier(false));

/// Ask the runner for something, of the main window or the floating window
/// [window]. Every call answers whether that window is maximised, so asking
/// for nothing in particular is how that is read.
Future<void> _ask(String what, {int? window}) async {
  try {
    _maximisedOf(window).value = await _channel.invokeMethod<bool>(
            what, window == null ? null : {'window': window}) ??
        false;
  } catch (_) {
    // No runner listening, which is a test.
  }
}

/// A part of the top line that moves the window when dragged, and maximises
/// it or puts it back on a double click. [window] names a floating window,
/// and [onMenu] gives its right-click a menu of its own.
class WindowDragArea extends StatelessWidget {
  final Widget? child;
  final int? window;
  final void Function(BuildContext context, Offset at)? onMenu;

  const WindowDragArea({super.key, this.child, this.window, this.onMenu});

  @override
  Widget build(BuildContext context) {
    if (!windowFrameDrawn) return child ?? const SizedBox.shrink();
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onPanStart: (_) => _ask('drag', window: window),
      onDoubleTap: () => _ask('maximise', window: window),
      onSecondaryTapUp: (d) => (onMenu ?? _menu)(context, d.globalPosition),
      child: child,
    );
  }
}

/// What a title bar's right-click offers: the three buttons, by name.
void _menu(BuildContext context, Offset at) {
  showLumitPopup<void>(
    context: context,
    position: at,
    builder: (close) => FloatSurface(
      width: 160,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final (what, label) in [
            ('minimise', l10n.windowMinimise),
            (
              'maximise',
              _maximised.value ? l10n.windowRestore : l10n.windowMaximise
            ),
          ])
            MenuRow(
              key: ValueKey<String>('window-menu-$what'),
              onPressed: () {
                close(null);
                _ask(what);
              },
              child: Text(label),
            ),
          const MenuDivider(),
          MenuRow(
            key: const ValueKey('window-menu-close'),
            onPressed: () {
              close(null);
              _ask('close');
            },
            child: Text(l10n.close),
          ),
        ],
      ),
    ),
  );
}

/// A floating window's title line's right-click: the window's buttons by
/// name, full screen, keeping it above other programs, and its panels back
/// in the main window.
void showFloatingWindowMenu(
  BuildContext context,
  Offset at, {
  required int window,
  required bool maximised,
  required bool fullScreen,
  required bool onTop,
  required ValueChanged<bool> onSetOnTop,
  required ValueChanged<bool> onFullScreen,
  required VoidCallback onDock,
}) {
  showLumitPopup<void>(
    context: context,
    position: at,
    builder: (close) {
      MenuRow row(String key, Widget label, VoidCallback act) => MenuRow(
            key: ValueKey<String>('floating-menu-$key'),
            onPressed: () {
              close(null);
              act();
            },
            child: label,
          );
      return FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              row(
                  'maximise',
                  MenuLabel(
                      maximised ? l10n.windowRestore : l10n.windowMaximise,
                      ticked: false),
                  () => _ask('maximise', window: window)),
              row('full-screen',
                  MenuLabel(l10n.windowFullScreen, ticked: fullScreen),
                  () => onFullScreen(!fullScreen)),
              row('on-top', MenuLabel(l10n.windowKeepOnTop, ticked: onTop),
                  () => onSetOnTop(!onTop)),
              const MenuDivider(),
              row('dock', MenuLabel(l10n.windowPanelsToMain, ticked: false),
                  onDock),
              const MenuDivider(),
              row('close', MenuLabel(l10n.close, ticked: false),
                  () => _ask('close', window: window)),
            ],
          ),
        ),
      );
    },
  );
}

/// A floating window's buttons where Lumit draws them.
class FloatingWindowButtons extends StatelessWidget {
  final LumitTheme theme;

  /// The floating window, by its native handle.
  final int window;

  const FloatingWindowButtons(
      {super.key, required this.theme, required this.window});

  @override
  Widget build(BuildContext context) =>
      _WindowButtons(theme: theme, window: window);
}

/// A floating window's own top line, for when the pane in its top right
/// corner has no line to carry the buttons: the empty line moves the window,
/// and the buttons stand at its end.
class FloatingTitleLine extends StatelessWidget {
  final LumitTheme theme;

  /// The floating window, by its native handle.
  final int window;
  final bool onTop;
  final void Function(BuildContext context, Offset at) onMenu;

  const FloatingTitleLine({
    super.key,
    required this.theme,
    required this.window,
    required this.onTop,
    required this.onMenu,
  });

  @override
  Widget build(BuildContext context) => SizedBox(
        key: const ValueKey('floating-title-line'),
        height: theme.density.menuBar,
        child: Row(
          children: [
            Expanded(child: WindowDragArea(window: window, onMenu: onMenu)),
            _WindowButtons(theme: theme, window: window),
          ],
        ),
      );
}

/// A floating window's edges and corners on Linux, where Lumit's own title
/// bar takes the system's frame away and the edges with it: a press on one
/// asks the window manager to resize from there. Elsewhere [child] as it is.
class FloatingResizeEdges extends StatelessWidget {
  final int window;
  final Widget child;

  const FloatingResizeEdges(
      {super.key, required this.window, required this.child});

  /// How far in from each side an edge reaches.
  static const double _reach = 5;

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform != TargetPlatform.linux || !windowFrameDrawn) {
      return child;
    }
    Widget edge(String which, MouseCursor cursor) => MouseRegion(
          cursor: cursor,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onPanStart: (_) => _channel.invokeMethod<void>(
                'beginResize', {'window': window, 'edge': which}).ignore(),
          ),
        );
    const r = _reach;
    return Stack(children: [
      child,
      Positioned(left: r, right: r, top: 0, height: r,
          child: edge('n', SystemMouseCursors.resizeUp)),
      Positioned(left: r, right: r, bottom: 0, height: r,
          child: edge('s', SystemMouseCursors.resizeDown)),
      Positioned(top: r, bottom: r, left: 0, width: r,
          child: edge('w', SystemMouseCursors.resizeLeft)),
      Positioned(top: r, bottom: r, right: 0, width: r,
          child: edge('e', SystemMouseCursors.resizeRight)),
      Positioned(left: 0, top: 0, width: r, height: r,
          child: edge('nw', SystemMouseCursors.resizeUpLeft)),
      Positioned(right: 0, top: 0, width: r, height: r,
          child: edge('ne', SystemMouseCursors.resizeUpRight)),
      Positioned(left: 0, bottom: 0, width: r, height: r,
          child: edge('sw', SystemMouseCursors.resizeDownLeft)),
      Positioned(right: 0, bottom: 0, width: r, height: r,
          child: edge('se', SystemMouseCursors.resizeDownRight)),
    ]);
  }
}

/// [child] with a strip along its top that moves the window, for a screen
/// that is the whole window and has no menu bar to move it by.
class WindowDragStrip extends StatelessWidget {
  final LumitTheme theme;
  final Widget child;

  const WindowDragStrip({super.key, required this.theme, required this.child});

  @override
  Widget build(BuildContext context) {
    if (!windowFrameDrawn) return child;
    return Stack(children: [
      child,
      PositionedDirectional(
        top: 0,
        start: 0,
        end: windowButtonsWidth,
        height: theme.density.menuBar,
        child: const WindowDragArea(),
      ),
    ]);
  }
}

/// [child] as the window, with the window's buttons over the end of its top
/// line.
class WindowFrame extends StatelessWidget {
  final LumitTheme theme;
  final Widget child;

  const WindowFrame({super.key, required this.theme, required this.child});

  @override
  Widget build(BuildContext context) {
    if (!windowFrameDrawn) return child;
    return Stack(children: [
      child,
      PositionedDirectional(
        top: 0,
        end: 0,
        height: theme.density.menuBar,
        child: _WindowButtons(theme: theme),
      ),
    ]);
  }
}

class _WindowButtons extends StatefulWidget {
  final LumitTheme theme;

  /// A floating window's native handle, or null for the main window.
  final int? window;

  const _WindowButtons({required this.theme, this.window});

  @override
  State<_WindowButtons> createState() => _WindowButtonsState();
}

class _WindowButtonsState extends State<_WindowButtons>
    with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _ask('state', window: widget.window);
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  // A window cannot be maximised or put back without changing size, so this
  // catches every way it happens: the button, a snap, the keyboard.
  @override
  void didChangeMetrics() {
    _ask('state', window: widget.window);
    _place();
  }

  final _maximiseKey = GlobalKey();
  Map<String, int>? _placed;

  /// Tell the runner where the maximise button is, in physical pixels, when
  /// that changes. Windows 11 opens its snap layouts over the button its own
  /// hit test finds, and the runner answers that test.
  void _place() => WidgetsBinding.instance.addPostFrameCallback((_) {
        final box =
            _maximiseKey.currentContext?.findRenderObject() as RenderBox?;
        if (!mounted || box == null || !box.attached) return;
        final rect = MatrixUtils.transformRect(
            box.getTransformTo(null), Offset.zero & box.size);
        final scale = View.of(context).devicePixelRatio;
        final placed = {
          'left': (rect.left * scale).round(),
          'top': (rect.top * scale).round(),
          'right': (rect.right * scale).round(),
          'bottom': (rect.bottom * scale).round(),
          if (widget.window case final window?) 'window': window,
        };
        if (mapEquals(placed, _placed)) return;
        _placed = placed;
        _channel.invokeMethod<bool>('maximiseButton', placed).ignore();
      });

  @override
  Widget build(BuildContext context) {
    _place();
    final window = widget.window;
    final maximisedState = _maximisedOf(window);
    return ListenableBuilder(
      listenable: Listenable.merge([
        maximisedState,
        window == null ? maximiseHovered : floatingMaximiseHovered,
      ]),
      builder: (context, _) {
        final maximised = maximisedState.value;
        final hovered = window == null
            ? maximiseHovered.value
            : floatingMaximiseHovered.value == window;
        return Row(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // A floating window is minimised with the main one, and has no
            // place of its own on the taskbar to go to.
            if (window == null)
              _WindowButton(
                theme: widget.theme,
                mark: _Mark.windowMinimise,
                label: l10n.windowMinimise,
                onPressed: () => _ask('minimise', window: window),
              ),
            // On Windows the runner takes this button's pointer and press,
            // and says when it is hovered.
            _WindowButton(
              key: _maximiseKey,
              theme: widget.theme,
              mark: maximised ? _Mark.windowRestore : _Mark.windowMaximise,
              label: maximised ? l10n.windowRestore : l10n.windowMaximise,
              hovered: hovered,
              onPressed: () => _ask('maximise', window: window),
            ),
            _WindowButton(
              theme: widget.theme,
              mark: _Mark.windowClose,
              label: l10n.close,
              onPressed: () => _ask('close', window: window),
            ),
          ],
        );
      },
    );
  }
}

/// The four marks. Each style draws its own (tool/icons/glyphs.json, Window): Studio in the
/// set's ordinary hand, Desk square and mitred, Lantern smaller and rounder.
enum _Mark {
  windowMinimise,
  windowMaximise,
  windowRestore,
  windowClose;

  String glyph(ThemeShape shape) => switch (shape) {
        ThemeShape.studio => const [
            LumitIcons.windowMinimise,
            LumitIcons.windowMaximise,
            LumitIcons.windowRestore,
            LumitIcons.close,
          ],
        ThemeShape.desk => const [
            LumitIcons.windowMinimiseSquare,
            LumitIcons.windowMaximiseSquare,
            LumitIcons.windowRestoreSquare,
            LumitIcons.windowCloseSquare,
          ],
        ThemeShape.lantern => const [
            LumitIcons.windowMinimiseRound,
            LumitIcons.windowMaximiseRound,
            LumitIcons.windowRestoreRound,
            LumitIcons.windowCloseRound,
          ],
      }[index];
}

class _WindowButton extends StatefulWidget {
  final LumitTheme theme;
  final _Mark mark;
  final String label;
  final VoidCallback onPressed;

  /// Lit from outside, for a pointer Flutter does not see.
  final bool hovered;

  const _WindowButton({
    super.key,
    required this.theme,
    required this.mark,
    required this.label,
    required this.onPressed,
    this.hovered = false,
  });

  @override
  State<_WindowButton> createState() => _WindowButtonState();
}

class _WindowButtonState extends State<_WindowButton> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = widget.theme;
    final hover = _hover || widget.hovered;
    // Close warns in the theme's own error colour, as the system's goes red.
    final closing = hover && widget.mark == _Mark.windowClose;
    // Lantern lights a round plate under the pointer, as it does for every
    // action. The flat styles light the button edge to edge.
    final round = t.tokens.roomed;
    return Semantics(
      button: true,
      label: widget.label,
      child: MouseRegion(
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: widget.onPressed,
          child: Container(
            width: _buttonWidth,
            padding: round
                ? const EdgeInsets.symmetric(horizontal: 4, vertical: 3)
                : null,
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: closing
                    ? t.error
                    : hover
                        ? t.surface3
                        : null,
                borderRadius:
                    round ? BorderRadius.circular(t.tokens.actionRadius) : null,
              ),
              child: Center(
                child: LumitIcon(
                  widget.mark.glyph(t.shape),
                  // At rest on Lantern the button stands on the room, so its
                  // ink is the room's.
                  colour: closing
                      ? t.surface0
                      : hover
                          ? t.textPrimary
                          : round
                              ? t.roomInk
                              : t.textSecondary,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
