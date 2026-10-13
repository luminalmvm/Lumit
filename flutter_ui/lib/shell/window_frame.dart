// The window's own buttons, and the handle it is moved by.
//
// # In plain terms
//
// On Windows and Linux Lumit's window has no title bar. The menu bar is its top
// line, the three buttons a title bar would hold stand at that line's right
// end, and the empty part of the line moves the window the way a title bar
// does. The runner does the moving and the closing (windows/runner/
// flutter_window.cpp, linux/runner/my_application.cc); this file draws the
// buttons and says what was pressed.
//
// The buttons are laid over the whole window rather than put in the menu bar,
// because the menu bar is not always there: the splash, the welcome screen and
// a start-up that failed are each the whole window, and a window with no way
// to close it is the worse fault.
//
// macOS keeps its title bar. Its menus are in the system's bar, so Lumit has
// no line of its own there to put the buttons in.

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../icons/icon_style.dart';
import '../icons/lumit_icon.dart';
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';

const MethodChannel _channel = MethodChannel('lumit/window');

/// Whether Lumit draws the window's buttons itself here. Asked of the target
/// platform, so a widget test draws none unless it says it is a desktop.
bool get windowFrameDrawn =>
    defaultTargetPlatform == TargetPlatform.windows ||
    defaultTargetPlatform == TargetPlatform.linux;

/// One button's width, and so a third of what the menu bar leaves clear at its
/// right end.
const double _buttonWidth = 40;

/// The room the three buttons take at the right end of the top line, or none
/// where the system draws them.
double get windowButtonsWidth => windowFrameDrawn ? _buttonWidth * 3 : 0;

/// Whether the window is maximised, which decides the middle button's drawing.
final ValueNotifier<bool> _maximised = ValueNotifier<bool>(false);

/// Ask the runner for something. Every call answers whether the window is
/// maximised, so asking for nothing in particular is how that is read.
Future<void> _ask(String what) async {
  try {
    _maximised.value = await _channel.invokeMethod<bool>(what) ?? false;
  } catch (_) {
    // No runner listening, which is a test.
  }
}

/// A part of the top line that moves the window when dragged, and maximises
/// it or puts it back on a double click.
class WindowDragArea extends StatelessWidget {
  final Widget? child;

  const WindowDragArea({super.key, this.child});

  @override
  Widget build(BuildContext context) {
    if (!windowFrameDrawn) return child ?? const SizedBox.shrink();
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onPanStart: (_) => _ask('drag'),
      onDoubleTap: () => _ask('maximise'),
      child: child,
    );
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
      Positioned(
        top: 0,
        left: 0,
        right: windowButtonsWidth,
        height: theme.density.menuBar,
        child: const WindowDragArea(),
      ),
    ]);
  }
}

/// [child] as the window, with the window's buttons over its top right corner.
class WindowFrame extends StatelessWidget {
  final LumitTheme theme;
  final Widget child;

  const WindowFrame({super.key, required this.theme, required this.child});

  @override
  Widget build(BuildContext context) {
    if (!windowFrameDrawn) return child;
    return Stack(children: [
      child,
      Positioned(
        top: 0,
        right: 0,
        height: theme.density.menuBar,
        child: _WindowButtons(theme: theme),
      ),
    ]);
  }
}

class _WindowButtons extends StatefulWidget {
  final LumitTheme theme;

  const _WindowButtons({required this.theme});

  @override
  State<_WindowButtons> createState() => _WindowButtonsState();
}

class _WindowButtonsState extends State<_WindowButtons>
    with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _ask('state');
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  // A window cannot be maximised or put back without changing size, so this
  // catches every way it happens: the button, a snap, the keyboard.
  @override
  void didChangeMetrics() => _ask('state');

  @override
  Widget build(BuildContext context) => ValueListenableBuilder<bool>(
        valueListenable: _maximised,
        builder: (context, maximised, _) => Row(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _WindowButton(
              theme: widget.theme,
              mark: _Mark.windowMinimise,
              label: l10n.windowMinimise,
              onPressed: () => _ask('minimise'),
            ),
            // ponytail: resting on this button does not open Windows 11's
            // snap layouts. Windows shows them only over a button it finds by
            // its own hit test, which would move the button's hover and press
            // into the runner. Win+Z and dragging to a screen edge still snap.
            _WindowButton(
              theme: widget.theme,
              mark: maximised ? _Mark.windowRestore : _Mark.windowMaximise,
              label: maximised ? l10n.windowRestore : l10n.windowMaximise,
              onPressed: () => _ask('maximise'),
            ),
            _WindowButton(
              theme: widget.theme,
              mark: _Mark.windowClose,
              label: l10n.close,
              onPressed: () => _ask('close'),
            ),
          ],
        ),
      );
}

/// The four marks, named as a person's own icon folder names them
/// (icons/icon_style.dart).
///
/// Each style draws its own (tool/icons/glyphs.json, Window): Studio in the
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

  const _WindowButton({
    required this.theme,
    required this.mark,
    required this.label,
    required this.onPressed,
  });

  @override
  State<_WindowButton> createState() => _WindowButtonState();
}

class _WindowButtonState extends State<_WindowButton> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = widget.theme;
    // Close warns in the theme's own error colour, as the system's goes red.
    final closing = _hover && widget.mark == _Mark.windowClose;
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
                    : _hover
                        ? t.surface3
                        : null,
                borderRadius:
                    round ? BorderRadius.circular(t.tokens.actionRadius) : null,
              ),
              child: Center(
                child: LumitIcon(
                  IconStyle.resolve(
                      widget.mark.name, widget.mark.glyph(t.shape), t),
                  // At rest on Lantern the button stands on the room, so its
                  // ink is the room's.
                  colour: closing
                      ? t.surface0
                      : _hover
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
