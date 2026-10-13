// The window's own buttons, and the handle it is moved by.
//
// # In plain terms
//
// On Windows Lumit's window has no title bar. The menu bar is its top line, the
// three buttons a title bar would hold stand at that line's right end, and the
// empty part of the line moves the window the way a title bar does. The runner
// does the moving and the closing (windows/runner/flutter_window.cpp); this
// file draws the buttons and says what was pressed.
//
// The buttons are laid over the whole window rather than put in the menu bar,
// because the menu bar is not always there: the splash, the welcome screen and
// a start-up that failed are each the whole window, and a window with no way
// to close it is the worse fault.
//
// macOS keeps its title bar. Its menus are in the system's bar, so Lumit has
// no line of its own there to put the buttons in. Linux keeps its title bar
// too, until its runner can do what the Windows one does.

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';

const MethodChannel _channel = MethodChannel('lumit/window');

/// Whether Lumit draws the window's buttons itself here. Asked of the target
/// platform, so a widget test draws none unless it says it is Windows.
bool get windowFrameDrawn => defaultTargetPlatform == TargetPlatform.windows;

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
              glyph: _Glyph.minimise,
              label: l10n.windowMinimise,
              onPressed: () => _ask('minimise'),
            ),
            // ponytail: resting on this button does not open Windows 11's
            // snap layouts. Windows shows them only over a button it finds by
            // its own hit test, which would move the button's hover and press
            // into the runner. Win+Z and dragging to a screen edge still snap.
            _WindowButton(
              theme: widget.theme,
              glyph: maximised ? _Glyph.restore : _Glyph.maximise,
              label: maximised ? l10n.windowRestore : l10n.windowMaximise,
              onPressed: () => _ask('maximise'),
            ),
            _WindowButton(
              theme: widget.theme,
              glyph: _Glyph.close,
              label: l10n.close,
              onPressed: () => _ask('close'),
            ),
          ],
        ),
      );
}

enum _Glyph { minimise, maximise, restore, close }

class _WindowButton extends StatefulWidget {
  final LumitTheme theme;
  final _Glyph glyph;
  final String label;
  final VoidCallback onPressed;

  const _WindowButton({
    required this.theme,
    required this.glyph,
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
    final closing = _hover && widget.glyph == _Glyph.close;
    return Semantics(
      button: true,
      label: widget.label,
      child: MouseRegion(
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: widget.onPressed,
          child: ColoredBox(
            color: closing
                ? t.error
                : _hover
                    ? t.surface3
                    : t.surface3.withValues(alpha: 0),
            child: SizedBox(
              width: _buttonWidth,
              child: CustomPaint(
                painter: _GlyphPainter(
                  widget.glyph,
                  closing
                      ? t.surface0
                      : _hover
                          ? t.textPrimary
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

/// The four marks, drawn as the system draws its own: hairlines in a box ten
/// pixels a side.
class _GlyphPainter extends CustomPainter {
  final _Glyph glyph;
  final Color colour;

  const _GlyphPainter(this.glyph, this.colour);

  @override
  void paint(Canvas canvas, Size size) {
    final pen = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    final box = Rect.fromCenter(
        center: size.center(Offset.zero), width: 10, height: 10);
    switch (glyph) {
      case _Glyph.minimise:
        canvas.drawLine(box.centerLeft, box.centerRight, pen);
      case _Glyph.maximise:
        canvas.drawRect(box, pen);
      case _Glyph.restore:
        // The window in front, and the top and right of the one behind it.
        canvas.drawRect(
            Rect.fromLTRB(box.left, box.top + 2, box.right - 2, box.bottom),
            pen);
        canvas.drawPath(
          Path()
            ..moveTo(box.left + 2, box.top + 2)
            ..lineTo(box.left + 2, box.top)
            ..lineTo(box.right, box.top)
            ..lineTo(box.right, box.bottom - 2)
            ..lineTo(box.right - 2, box.bottom - 2),
          pen,
        );
      case _Glyph.close:
        canvas.drawLine(box.topLeft, box.bottomRight, pen);
        canvas.drawLine(box.topRight, box.bottomLeft, pen);
    }
  }

  @override
  bool shouldRepaint(_GlyphPainter old) =>
      old.glyph != glyph || old.colour != colour;
}
