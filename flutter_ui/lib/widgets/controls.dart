// House controls, owned rather than Material (docs/archive/flutter-port/04): every
// colour and metric reads the theme, idle widgets are borderless, hover and
// press bring an edge back (the K-084 owner amendment).

import 'dart:async';

import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/services.dart';

import '../l10n/strings.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:lumit_flutter/widgets/autofill.dart';
import 'package:lumit_flutter/widgets/escape_ladder.dart';
import 'package:lumit_flutter/widgets/hover_intent.dart';
import 'package:lumit_flutter/widgets/time_readout.dart' show monoSlotWidth;

/// The devices whose drags mean "move this thing" — **the trackpad's
/// two-finger scroll deliberately excluded**.
///
/// A two-finger scroll on a Mac trackpad arrives as a pan *gesture*, not as the
/// wheel's pointer signal, so any pan recogniser laid over a scrollable area
/// wins it in the arena and the area cannot be scrolled at all: reported as "I
/// can't scroll the timeline with my trackpad", and invisible to anyone with a
/// mouse. Excluding the trackpad here costs nothing that a user wants — a
/// *click*-drag on a trackpad is an ordinary pointer drag and is unaffected —
/// and hands two-finger scrolling back to the scrollable underneath.
const Set<PointerDeviceKind> dragDevices = {
  PointerDeviceKind.mouse,
  PointerDeviceKind.touch,
  PointerDeviceKind.stylus,
  PointerDeviceKind.invertedStylus,
  PointerDeviceKind.unknown,
};

/// The focus node every house control that answers the keyboard holds
/// (buttons, checkboxes, value boxes). The global shortcut handler in
/// `main.dart` stands down while one of these has focus — the same courtesy
/// it pays a focused text field — so `Enter` or `Space` on a focused control
/// presses the control and never also runs a panel command underneath it
/// (K-319).
class ControlFocusNode extends FocusNode {
  ControlFocusNode({super.debugLabel});
}

/// Enter, numpad Enter and Space, once per press (never on key repeat) —
/// what "press the focused control" means for every house control.
const Map<ShortcutActivator, Intent> _activateShortcuts = {
  SingleActivator(LogicalKeyboardKey.enter, includeRepeats: false):
      ActivateIntent(),
  SingleActivator(LogicalKeyboardKey.numpadEnter, includeRepeats: false):
      ActivateIntent(),
  SingleActivator(LogicalKeyboardKey.space, includeRepeats: false):
      ActivateIntent(),
};

/// The theme + workspace scope: an InheritedWidget the whole tree reads.
class ThemeScope extends InheritedWidget {
  final LumitTheme theme;
  final AnimationLevel animationLevel;
  final bool showTooltips;

  const ThemeScope({
    super.key,
    required this.theme,
    required this.animationLevel,
    required this.showTooltips,
    required super.child,
  });

  static ThemeScope of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<ThemeScope>()!;

  @override
  bool updateShouldNotify(ThemeScope old) =>
      old.theme != theme ||
      old.animationLevel != animationLevel ||
      old.showTooltips != showTooltips;
}

/// The house button, in the three faces docs/15-DESIGN.md gives it (K-439):
///
/// - **Secondary** (the default): an outline and nothing else — a
///   `hairline_strong` border with the panel's own surface showing through, so
///   a resting panel keeps to its three greys however many buttons it carries
///   (§2.1). `frameless` drops even the outline.
/// - **Primary** ([primary]): THE single filled action on the surface — an
///   `accent` fill with the label at the far end of the ramp, in mono capitals
///   (§3.1's closed accent job list, §12A.4's dialog footer).
/// - **Dropdown** ([dropdown]): a `surface2` well-adjacent face with a plain
///   `hairline` border and no raised fill.
///
/// Hover and press are fill steps over whichever face is set — `surface3` then
/// `hairline_strong` (§2.3).
class HouseButton extends StatefulWidget {
  final Widget child;
  final VoidCallback? onPressed;
  final bool frameless;
  final bool small;
  final EdgeInsets? padding;

  /// The default action of the window it sits in — what `Enter` presses
  /// (K-243), and the one filled button the surface is allowed (§3.1).
  final bool primary;

  /// The closed face of a dropdown: `surface2` and a plain hairline rather
  /// than the secondary button's outline, so a picker reads as something to
  /// open rather than as an action to take.
  final bool dropdown;

  /// Take keyboard focus on first build — for the default button of a
  /// confirmation window, so `Enter` presses it the moment the window opens
  /// (K-319). Pair it with [primary] so what Enter will do is visible.
  final bool autofocus;

  /// The chosen one of a set — a tab, a mode chip, a segmented option.
  ///
  /// Under Round this is K-394's **filled accent pill**: `accent` fill and the
  /// label in `surface0`, the far end of the ramp from the text, which is the
  /// dark label on a dark scheme and the light one on a light scheme without
  /// either being spelled out twice. Under Sharp it stays the accent *tint*
  /// the tool bar already arms a tool with — the state contrast is the shape's
  /// difference, not a colour change (Sharp's geometry and treatment are
  /// untouched).
  ///
  /// [primary] is a different thing: that is what `Enter` would press, this is
  /// which of several is currently in force.
  final bool active;

  const HouseButton({
    super.key,
    required this.child,
    this.onPressed,
    this.frameless = false,
    this.small = false,
    this.padding,
    this.primary = false,
    this.dropdown = false,
    this.autofocus = false,
    this.active = false,
  });

  @override
  State<HouseButton> createState() => _HouseButtonState();
}

class _HouseButtonState extends State<HouseButton> {
  bool _hover = false;
  bool _down = false;
  bool _focused = false;
  final ControlFocusNode _focusNode = ControlFocusNode(debugLabel: 'button');

  @override
  void dispose() {
    _focusNode.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final enabled = widget.onPressed != null;
    Color? fill;
    Color? edge;
    // The label, when the fill under it decides what colour it has to be.
    Color? label;
    // The label's own style, for the one face that sets it: mono capitals.
    TextStyle? labelStyle;
    if (!enabled) {
      fill = widget.frameless ? null : t.surface2;
      // A dead primary drops the accent but keeps the shape of its word: the
      // Export button must not change case as the path field is filled in.
      if (widget.primary) labelStyle = t.kicker.copyWith(color: t.textDisabled);
    } else if (widget.primary) {
      // The single filled action (§3.1, §12A.4). Ahead of every other state:
      // what `Enter` presses must be findable at a glance whatever the pointer
      // is doing, so hover lifts the fill rather than replacing it.
      fill = _hover || _down ? t.accentHover : t.accent;
      label = t.surface0;
      labelStyle = t.kicker.copyWith(color: t.surface0);
    } else if (widget.active) {
      // Ahead of hover and press: which one is in force must not blink off
      // under the pointer. Hover lifts it to `accentHover` instead.
      final round = t.shape == ThemeShape.round;
      fill = round
          ? (_hover || _down ? t.accentHover : t.accent)
          : t.accent.withValues(alpha: _hover || _down ? 0.24 : 0.16);
      edge = round ? null : t.accent;
      if (round) label = t.surface0;
    } else if (_down) {
      fill = t.hairlineStrong;
      edge = t.hairlineStrong;
    } else if (_hover) {
      fill = t.surface3;
      edge = t.hairlineStrong;
    } else if (widget.dropdown) {
      fill = t.surface2;
      edge = _focused ? t.accent : t.hairline;
    } else {
      // Idle: an outline over the panel's own surface, not a raised fill —
      // a widget at rest adds no fourth grey to the panel (§2.1, §2.3).
      fill = null;
      edge = _focused
          ? t.accent
          : widget.frameless
              ? null
              : t.hairlineStrong;
    }
    final pad = widget.padding ??
        (widget.small
            ? const EdgeInsets.symmetric(horizontal: 5, vertical: 2)
            : const EdgeInsets.symmetric(horizontal: 8, vertical: 3));
    // Keyboard-reachable (docs/15 §9): Tab lands here in reading order,
    // Enter/Space press it, and the accent edge is the focus ring (§6.5).
    return FocusableActionDetector(
      focusNode: _focusNode,
      enabled: enabled,
      autofocus: widget.autofocus,
      mouseCursor: enabled ? SystemMouseCursors.click : MouseCursor.defer,
      shortcuts: _activateShortcuts,
      actions: <Type, Action<Intent>>{
        ActivateIntent: CallbackAction<ActivateIntent>(onInvoke: (_) {
          widget.onPressed?.call();
          return null;
        }),
      },
      onFocusChange: (has) => setState(() => _focused = has),
      onShowHoverHighlight: (over) => setState(() {
        _hover = over;
        if (!over) _down = false;
      }),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTapDown: enabled ? (_) => setState(() => _down = true) : null,
        onTapUp: enabled ? (_) => setState(() => _down = false) : null,
        onTapCancel: enabled ? () => setState(() => _down = false) : null,
        onTap: widget.onPressed,
        child: AnimatedContainer(
          duration: animationDuration(scope.animationLevel),
          padding: pad,
          decoration: BoxDecoration(
            color: fill,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            // Always a border, transparent when there is nothing to show. A
            // BoxDecoration's border insets its child, so appearing on hover
            // grew the control by 2 px each way and nudged everything beside
            // it — the whole row visibly shifting as the pointer crossed it.
            border:
                Border.all(color: edge ?? const Color(0x00000000), width: 1),
          ),
          child: DefaultTextStyle(
            // A dropdown's closed face reads in the secondary text colour —
            // the mockups' own (§12A.6); bright primary at 11px reads bold.
            style: enabled
                ? (labelStyle ??
                    (label == null
                        ? (widget.dropdown ? t.body : t.bodyPrimary)
                        : t.bodyPrimary.copyWith(color: label)))
                : (labelStyle ?? t.body.copyWith(color: t.textDisabled)),
            child: _centred(_capitalised(widget.child, widget.primary)),
          ),
        ),
      ),
    );
  }
}

/// The label, held in the middle of whatever box the button was given.
///
/// **Why this is needed at all.** A `Container`'s padding passes its own
/// constraints straight down, so a button handed a *height* — the dialog
/// footer states 24 (§12A.4), a bar states its own — hands the label a **tight**
/// box 2px shorter. A paragraph in a tight box stretches to fill it and then
/// paints its single line at the **top**, which is the "the text sits
/// off-centre" every dialog footer showed: the label's rectangle was centred
/// while its words were not.
///
/// **An `Align` with both factors set to 1** is the whole fix, and the factors
/// are the point: they make it shrink-wrap, so under the loose constraints
/// every other button gets it is exactly the size of the child and a button
/// that was sizing itself to its own words still does, to the pixel. Under a
/// stated height it is forced to that height instead and centres the label in
/// it. `Container(alignment:)` would have centred too — and also made every
/// button in a bounded row grow to that row's full height, which is a different
/// change to every bar in the application.
///
/// A plain `Column` would centre as well, and was tried: it *reports* an
/// overflow wherever a caller states a box **smaller** than its label, and the
/// Export dialog has one such caller today. That is worth finding on its own
/// account, but it is not this fix's business to start failing the suite over —
/// `Align` takes the too-small box the way the padding always silently did.
Widget _centred(Widget label) => Align(
      alignment: Alignment.center,
      widthFactor: 1,
      heightFactor: 1,
      child: label,
    );

/// The primary button's label in capitals, when it is a plain word.
///
/// Flutter has no text transform, and the capitals are a *style* rather than
/// part of the phrase — the arb file keeps one sentence-case key, translated
/// once — so the upper-casing happens here on the way to the screen. A child
/// that is anything but a bare [Text] (an icon, a row) is handed back
/// untouched.
Widget _capitalised(Widget child, bool on) =>
    on && child is Text && child.data != null
        ? Text(child.data!.toUpperCase(), style: child.style)
        : child;

/// One row in a dropdown/menu popup.
class MenuRow extends StatefulWidget {
  final Widget child;
  final VoidCallback onPressed;
  final bool selected;

  /// What this row calls itself in its surface's hover state, for the rows that
  /// have to know which of them the pointer is over. Defaults to the row's own
  /// state; [SubmenuRow] passes its own, because the flyout belongs to the
  /// submenu row rather than to the plain row it draws itself with.
  final Object? hoverId;

  const MenuRow({
    super.key,
    required this.child,
    required this.onPressed,
    this.selected = false,
    this.hoverId,
  });

  @override
  State<MenuRow> createState() => _MenuRowState();
}

class _MenuRowState extends State<MenuRow> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final fill = _hover
        ? t.surface4
        : widget.selected
            ? t.accent.withValues(alpha: 0.5)
            : null;
    final surface = FloatSurface._of(context);
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (e) {
        setState(() => _hover = true);
        // Tell the surface which row the pointer is on, so a submenu that is
        // out can take itself back when the pointer moves to another row. The
        // surface may *hold* the report briefly while the pointer is inside an
        // open flyout's safe triangle (K-318) — the highlight above is
        // immediate either way; only the flyout switch waits.
        surface?._hoverRow(widget.hoverId ?? this, e.position);
      },
      onHover: (e) => surface?._hoverMoved(widget.hoverId ?? this, e.position),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: widget.onPressed,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
          decoration: BoxDecoration(
            color: fill,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          ),
          child: DefaultTextStyle(style: t.bodyPrimary, child: widget.child),
        ),
      ),
    );
  }
}

/// The floating popup surface every menu and dropdown shares: `surface3`
/// fill, hairline edge, the float radius and the real drop shadow.
///
/// It also carries the surface's hover state — which of its rows the pointer is
/// over — because opening a flyout is one row's business and closing it again is
/// every other row's (see [SubmenuRow]). Scoped to the surface, so the rows of a
/// flyout never disturb the menu the flyout came from.
class FloatSurface extends StatefulWidget {
  final Widget child;
  final double? width;
  const FloatSurface({super.key, required this.child, this.width});

  /// The row of the nearest floating surface the pointer is on, or null outside
  /// one, where no menu is being drawn.
  static ValueNotifier<Object?>? hoveredRow(BuildContext context) =>
      context.getInheritedWidgetOfExactType<_MenuHoverScope>()?.hovered;

  /// The surface state itself — the rows and [SubmenuRow] talk to it for the
  /// hover-intent gating (K-318).
  static _FloatSurfaceState? _of(BuildContext context) =>
      context.getInheritedWidgetOfExactType<_MenuHoverScope>()?.state;

  @override
  State<FloatSurface> createState() => _FloatSurfaceState();
}

class _FloatSurfaceState extends State<FloatSurface> {
  final _hovered = ValueNotifier<Object?>(null);

  // --- Safe-triangle hover intent (K-318) -------------------------------
  //
  // While a [SubmenuRow]'s flyout is out, it arms a guard here: the row it
  // belongs to, and the flyout's rectangle. A hover report from any *other*
  // row is then held back while the pointer is inside the triangle from where
  // it left the owner row to the flyout's near edge — that path crosses the
  // rows below on any diagonal, and switching on them would take the flyout
  // away from under a pointer that was plainly travelling to it. The held
  // report lands when the pointer leaves the triangle, or when it has sat
  // still over the other row for [menuHoverGrace].

  /// The submenu row whose flyout is out, or null when none is.
  Object? _guardOwner;

  /// That flyout's rectangle in global coordinates, once measured.
  Rect? _guardRect;

  /// Where the pointer last was over the owner row — the triangle's apex.
  Offset? _guardApex;

  /// The pointer's last reported position over any of this surface's rows —
  /// the apex a guard starts from when the flyout opens before the pointer
  /// has moved again.
  Offset? _lastPointer;

  Timer? _pendingSwitch;
  Object? _pendingRow;

  SafeTriangle? get _triangle {
    final rect = _guardRect;
    final apex = _guardApex;
    if (_guardOwner == null || rect == null || apex == null) return null;
    return SafeTriangle.towards(apex, rect);
  }

  void _hoverRow(Object row, Offset globalPos) {
    _lastPointer = globalPos;
    if (_hovered.value == row) {
      _cancelPending();
      return;
    }
    if (row == _guardOwner) {
      // Back on the owner row: the flyout is safe, and the apex follows.
      _guardApex = globalPos;
      _cancelPending();
      _hovered.value = row;
      return;
    }
    final triangle = _triangle;
    if (triangle != null && triangle.contains(globalPos)) {
      // On another row, but travelling towards the flyout: hold the switch.
      _pendingRow = row;
      _pendingSwitch?.cancel();
      _pendingSwitch = Timer(menuHoverGrace, () {
        final pending = _pendingRow;
        _cancelPending();
        if (pending != null) _hovered.value = pending;
      });
      return;
    }
    _cancelPending();
    _hovered.value = row;
    _syncDebugOverlay();
  }

  void _hoverMoved(Object row, Offset globalPos) {
    _lastPointer = globalPos;
    if (row == _guardOwner) {
      _guardApex = globalPos;
      _syncDebugOverlay();
      return;
    }
    if (_pendingRow != row) return;
    final triangle = _triangle;
    if (triangle == null || !triangle.contains(globalPos)) {
      // The pointer settled on this row rather than passing over it on the
      // way to the flyout — no reason to keep it waiting.
      _cancelPending();
      _hovered.value = row;
    }
    _syncDebugOverlay();
  }

  void _cancelPending() {
    _pendingSwitch?.cancel();
    _pendingSwitch = null;
    _pendingRow = null;
  }

  // --- The debug overlay (the Debug panel's "Safe hover triangles") ------
  //
  // Reads the guard, never writes to it: the drawing must not be able to
  // change what the guard decides, or it would be showing something other
  // than the thing under test.

  /// What the overlay draws, or null when there is nothing to draw. Held as a
  /// notifier so the apex can follow the pointer without rebuilding the menu.
  final ValueNotifier<(SafeTriangle, bool)?> _debugShape =
      ValueNotifier<(SafeTriangle, bool)?>(null);
  OverlayEntry? _debugEntry;

  void _syncDebugOverlay() {
    final wanted = debugShowSafeTriangles.value ? _triangle : null;
    _debugShape.value = wanted == null ? null : (wanted, _pendingRow != null);
    if (wanted != null && _debugEntry == null && mounted) {
      final t = ThemeScope.of(context).theme;
      _debugEntry = OverlayEntry(
        builder: (_) => Positioned.fill(
          child: IgnorePointer(
            child: ValueListenableBuilder<(SafeTriangle, bool)?>(
              valueListenable: _debugShape,
              builder: (_, shape, __) => CustomPaint(
                painter: shape == null
                    ? null
                    : _SafeTrianglePainter(
                        shape.$1,
                        // Amber while a row switch is actually being held
                        // back: that is the guard doing its job, and it is
                        // the moment worth seeing.
                        shape.$2 ? t.warning : t.accent,
                      ),
              ),
            ),
          ),
        ),
      );
      Overlay.of(context, rootOverlay: true).insert(_debugEntry!);
    } else if (wanted == null) {
      _removeDebugOverlay();
    }
  }

  void _removeDebugOverlay() {
    _debugEntry?.remove();
    _debugEntry = null;
  }

  /// A [SubmenuRow]'s flyout opened: guard the diagonal to [flyout]. The
  /// apex starts where the pointer last was — the pointer opened the flyout,
  /// so it is on the owner row even if it has not moved since — and follows
  /// it while it stays there.
  void _armFlyoutGuard(Object owner, Rect flyout) {
    _guardOwner = owner;
    _guardRect = flyout;
    _guardApex = _lastPointer ?? Offset(flyout.left, flyout.center.dy);
    _syncDebugOverlay();
  }

  /// The pointer reached the flyout: whatever switch was pending on the way
  /// is void, and the owner row stays the surface's hovered row.
  void _flyoutEntered(Object owner) {
    if (_guardOwner != owner) return;
    _cancelPending();
    _hovered.value = owner;
  }

  /// The flyout has gone; the guard goes with it.
  void _disarmFlyoutGuard(Object owner) {
    if (_guardOwner != owner) return;
    _guardOwner = null;
    _guardRect = null;
    _guardApex = null;
    _cancelPending();
    _syncDebugOverlay();
  }

  @override
  void dispose() {
    _pendingSwitch?.cancel();
    // The overlay lives in the Overlay, not under this widget, so a surface
    // disposed with it showing would leave the triangle on screen.
    _removeDebugOverlay();
    _debugShape.dispose();
    _hovered.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return _MenuHoverScope(
      hovered: _hovered,
      state: this,
      child: Container(
        width: widget.width,
        padding: const EdgeInsets.all(6),
        decoration: BoxDecoration(
          color: t.surface3,
          borderRadius: BorderRadius.circular(t.tokens.floatRadius),
          border: Border.all(color: t.hairline, width: 1),
          boxShadow: t.floatShadow,
        ),
        child: widget.child,
      ),
    );
  }
}

/// The safe triangle, drawn: a translucent fill so the menu underneath stays
/// readable, its edges, and a ring at the apex where the pointer left the
/// owner row. The overlay fills the window, so global coordinates are the
/// canvas's own.
class _SafeTrianglePainter extends CustomPainter {
  final SafeTriangle triangle;
  final Color colour;

  const _SafeTrianglePainter(this.triangle, this.colour);

  @override
  void paint(Canvas canvas, Size size) {
    final path = Path()
      ..moveTo(triangle.apex.dx, triangle.apex.dy)
      ..lineTo(triangle.cornerA.dx, triangle.cornerA.dy)
      ..lineTo(triangle.cornerB.dx, triangle.cornerB.dy)
      ..close();
    canvas.drawPath(path, Paint()..color = colour.withValues(alpha: 0.18));
    canvas.drawPath(
      path,
      Paint()
        ..color = colour
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
    canvas.drawCircle(
      triangle.apex,
      3,
      Paint()
        ..color = colour
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(_SafeTrianglePainter old) =>
      old.colour != colour ||
      old.triangle.apex != triangle.apex ||
      old.triangle.cornerA != triangle.cornerA ||
      old.triangle.cornerB != triangle.cornerB;
}

class _MenuHoverScope extends InheritedWidget {
  final ValueNotifier<Object?> hovered;
  final _FloatSurfaceState state;

  const _MenuHoverScope({
    required this.hovered,
    required this.state,
    required super.child,
  });

  // The notifier itself never changes; the rows listen to it directly.
  @override
  bool updateShouldNotify(_MenuHoverScope old) => false;
}

/// The closed face all three bare dropdowns share: the label and the caret.
///
/// Ellipsised rather than allowed to overflow: a dropdown sits in whatever
/// width its caller has, and a label longer than that is a layout error the
/// user sees as striped tape. `Flexible` keeps the button intrinsic-width when
/// there is room, so nothing that fits changes shape.
///
/// [face] replaces the label with a mark of the caller's own — the Viewer
/// bar's channel picker, whose answer is a tinted glyph rather than a word
/// (K-411). The caret is the same one either way, so an icon dropdown still
/// reads as a dropdown.
Widget dropdownFace(LumitTheme t, String label, {Widget? face}) =>
    LayoutBuilder(builder: (context, c) {
      // In a cell too tight for even the caret and its gap (a fold-out value
      // column at its minimum), the caret is the first thing to go — a
      // sliver of the word still says more than striped overflow tape.
      final caretFits = !c.hasBoundedWidth || c.maxWidth >= 20;
      return Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          face ?? Flexible(child: Text(label, overflow: TextOverflow.ellipsis)),
          if (caretFits) ...[
            // 6, the gap every `.dd` the mockups compute leaves between its
            // label and its caret.
            const SizedBox(width: 6),
            // A small quiet mark: the border already says this is a control,
            // so the caret only has to say which kind (§12A, no raised look).
            CustomPaint(
              size: const Size(7, 7),
              painter: _CaretPainter(t.textMuted),
            ),
          ],
        ],
      );
    });

/// The **in-row** dropdown's label size (§12A.6's table): the pickers that sit
/// inside a Timeline row — matte, blend and parent — carry a 10px label, as the
/// approved mockups draw them. Their *height* is a density token
/// (`DensityTokens.inRowPicker`, K-454) because it is one of the handful of
/// measurements the Compact setting moves; the label size is not, and never
/// will be — Compact takes room out of rows, never legibility out of words.
const double inRowDropdownTextSize = 10;

/// The closed face of every bare dropdown, in its two sizes.
///
/// **Vertical 1 rather than the button default's 3**, so a label's descenders
/// are not clipped in a property row. The sum is tighter than it looks: a row
/// gives the button 18, the decoration's border insets the child by 1 top and
/// bottom, and body text at 11 carries a line box of about 13.3 — so the
/// padding has 3.4 to spend and 3 does not fit. At 1 the label has 14 to sit in
/// and centres there with room for the tails on p, q and g. Horizontal is the
/// button's own, so nothing moves sideways.
///
/// **Both heights are stated rather than left to the text**, because the
/// mockups' measurements are measurements and not consequences: a face that
/// grew out of its own font drifted every time the type did. [dense] is the
/// in-row face — the pickers inside a Timeline row — and the other is every
/// dropdown in a panel row or a bar. Both come from the density tokens
/// (K-454), so the Compact setting moves them together.
///
/// **Horizontal 6, not the button's 8**: every `.dd` the mockups compute pads
/// its label by exactly 6 either side, in both sizes.
Widget dropdownButton({
  required LumitTheme t,
  required bool dense,
  required VoidCallback? onPressed,
  required Widget face,
}) =>
    SizedBox(
      height: dense ? t.density.inRowPicker : t.density.dropdownFace,
      child: HouseButton(
        padding: EdgeInsets.symmetric(horizontal: 6, vertical: dense ? 0 : 1),
        onPressed: onPressed,
        dropdown: true,
        child: dense
            ? DefaultTextStyle.merge(
                style: const TextStyle(fontSize: inRowDropdownTextSize),
                child: face,
              )
            : face,
      ),
    );

/// A dropdown drawn as a bare label + caret; the open list floats on the
/// standard menu surface (`bare_dropdown` in the Rust settings window).
class BareDropdown<T> extends StatelessWidget {
  final T value;
  final List<T> options;
  final String Function(T) label;

  /// Null disables the control — the closed face still names the value, drawn
  /// in [HouseButton]'s own disabled style, and opens nothing. For a choice
  /// something else is currently making (the Viewer's resolution while
  /// adaptive playback picks the tier itself).
  final ValueChanged<T>? onChanged;

  /// The heading an option sits under, or null for none. Options keep their
  /// given order; a heading is drawn each time the answer changes, so a list
  /// that is already grouped needs nothing else, and one that is not gets no
  /// headings rather than a scrambled list.
  final String? Function(T)? group;

  /// A mark to show instead of the value's name on the closed face. The menu
  /// still lists [label]'s words, so nothing is lost by showing a glyph — see
  /// [dropdownFace].
  final Widget? face;

  /// The in-row face: 16 tall with a 10px label, for a picker that sits inside
  /// a Timeline row rather than in a dialog or a bar (§12A.6, K-451).
  final bool dense;

  /// Why an option cannot be chosen, or null where it can — K-485's
  /// disabled-not-hidden rule inside a list. The row stays in the menu, drawn
  /// quiet, with the reason on hover; a list that removed it would leave the
  /// reader hunting for a name they know exists.
  final String? Function(T)? disabledReason;

  const BareDropdown({
    super.key,
    required this.value,
    required this.options,
    required this.label,
    required this.onChanged,
    this.group,
    this.face,
    this.dense = false,
    this.disabledReason,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return dropdownButton(
      t: t,
      dense: dense,
      onPressed: onChanged == null ? null : () => _open(context, t),
      face: dropdownFace(t, label(value), face: face),
    );
  }

  Future<void> _open(BuildContext context, LumitTheme t) async {
    final box = context.findRenderObject()! as RenderBox;
    final origin = box.localToGlobal(Offset.zero);
    // A one-item list rather than the value itself. The popup answers null when
    // it is dismissed, so for an option list that *contains* null — "System
    // default" on the Audio page, "Follow the machine" on General — choosing
    // that option and closing the menu were the same answer, and the option
    // could never be picked at all. Boxing keeps the two apart.
    final picked = await showLumitPopup<List<T>>(
      context: context,
      position: origin + Offset(0, box.size.height + 2),
      // IntrinsicWidth bounds the stretch: a float in the overlay has
      // unbounded width, and a stretched Column inside one otherwise
      // forces an infinite width (the settings-dropdown crash).
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (var i = 0; i < options.length; i++) ...[
                if (group != null &&
                    group!(options[i]) != null &&
                    (i == 0 || group!(options[i - 1]) != group!(options[i])))
                  Padding(
                    padding: EdgeInsets.fromLTRB(10, i == 0 ? 6 : 10, 10, 2),
                    child: Text(
                      group!(options[i])!,
                      style: t.small.copyWith(color: t.textMuted),
                    ),
                  ),
                if (disabledReason?.call(options[i]) case final why?)
                  LumitTooltip(
                    message: why,
                    child: MenuRow(
                      selected: options[i] == value,
                      onPressed: () {},
                      child: Text(label(options[i]),
                          style: TextStyle(color: t.textDisabled)),
                    ),
                  )
                else
                  MenuRow(
                    selected: options[i] == value,
                    onPressed: () => close([options[i]]),
                    child: Text(label(options[i])),
                  ),
              ],
            ],
          ),
        ),
      ),
    );
    if (picked != null) onChanged!(picked.single);
  }
}

/// Options at or above this count get [BareSearchDropdown] instead of the
/// plain [BareDropdown] (K-262). A plain dropdown builds every row eagerly
/// inside an `IntrinsicWidth`, which walks all of them twice — fine for the
/// handful of options every parameter has today — and fatal for the
/// K-262-era Lens flare library, whose 1299 rows took the app down in
/// layout. The flare is a curated twenty since K-264; the guard stays.
const int searchableOptionThreshold = 40;

/// A dropdown for long option lists: a search field over a **lazily built**
/// list, with the group headings drawn inline (K-262).
///
/// The list is a `ListView.builder` inside a bounded box, so only the rows
/// on screen are ever built no matter how many options there are — the
/// difference between a thousand-row list being a feature and a crash.
class BareSearchDropdown extends StatelessWidget {
  final int value;
  final List<String> options;
  final ValueChanged<int> onChanged;

  /// The heading an option sits under, or null for none.
  final String? Function(String)? group;

  /// Placeholder for the search field — what the user is looking for. Null
  /// takes the plain word "Search", which is what most callers want.
  final String? hint;

  const BareSearchDropdown({
    super.key,
    required this.value,
    required this.options,
    required this.onChanged,
    this.group,
    this.hint,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final label = value >= 0 && value < options.length ? options[value] : '—';
    return HouseButton(
      // Vertical 1 rather than the button default's 3, so a label's descenders
      // are not clipped in a property row. The sum is tighter than it looks: a
      // row gives the button 18, the decoration's border insets the child by 1
      // top and bottom, and body text at 11 carries a line box of about 13.3 —
      // so the padding has 3.4 to spend and 3 does not fit. At 1 the label has
      // 14 to sit in and centres there with room for the tails on p, q and g.
      // Shrinking the text instead would not have done it: 10 still asks for
      // 12.1, which clears 3 by nothing at all. Horizontal is the button's own,
      // so nothing moves sideways.
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 1),
      onPressed: () async {
        final box = context.findRenderObject()! as RenderBox;
        final origin = box.localToGlobal(Offset.zero);
        final picked = await showLumitPopup<int>(
          context: context,
          position: origin + Offset(0, box.size.height + 2),
          builder: (close) => FloatSurface(
            child: _SearchPickerBody(
              value: value,
              options: options,
              group: group,
              hint: hint ?? l10n.search,
              onPick: close,
            ),
          ),
        );
        if (picked != null) onChanged(picked);
      },
      dropdown: true,
      child: dropdownFace(t, label),
    );
  }
}

/// One row of the picker's flattened list: a heading, or an option.
class _PickerEntry {
  final String? heading;
  final int? optionIndex;
  const _PickerEntry.heading(this.heading) : optionIndex = null;
  const _PickerEntry.option(this.optionIndex) : heading = null;
}

class _SearchPickerBody extends StatefulWidget {
  final int value;
  final List<String> options;
  final String? Function(String)? group;
  final String hint;
  final void Function(int?) onPick;

  const _SearchPickerBody({
    required this.value,
    required this.options,
    required this.group,
    required this.hint,
    required this.onPick,
  });

  @override
  State<_SearchPickerBody> createState() => _SearchPickerBodyState();
}

class _SearchPickerBodyState extends State<_SearchPickerBody> {
  final TextEditingController _query = TextEditingController();
  late List<_PickerEntry> _entries = _build('');

  @override
  void dispose() {
    _query.dispose();
    super.dispose();
  }

  /// The visible rows for a query: every option whose label contains it
  /// (case-insensitively, all terms), with a heading each time the group
  /// changes. Flattened so the list builder stays lazy.
  List<_PickerEntry> _build(String query) {
    final terms =
        query.toLowerCase().split(' ').where((w) => w.isNotEmpty).toList();
    final out = <_PickerEntry>[];
    String? lastGroup;
    for (var i = 0; i < widget.options.length; i++) {
      final label = widget.options[i];
      final lower = label.toLowerCase();
      if (terms.any((w) => !lower.contains(w))) continue;
      final g = widget.group?.call(label);
      if (g != null && g != lastGroup) {
        out.add(_PickerEntry.heading(g));
        lastGroup = g;
      }
      out.add(_PickerEntry.option(i));
    }
    return out;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // A fixed box: the popup's own scroll view would otherwise give the
    // list unbounded height, and an unbounded ListView cannot be lazy.
    return SizedBox(
      width: 300,
      height: 380,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 2, 4, 6),
            child: HouseTextField(
              controller: _query,
              width: 288,
              autofocus: true,
              hint: widget.hint,
              onSubmitted: (_) {
                // Enter takes the only match, which is what a search that
                // has narrowed to one thing means.
                final only = _entries.where((e) => e.optionIndex != null);
                if (only.length == 1) widget.onPick(only.first.optionIndex);
              },
            ),
          ),
          Expanded(
            child: _entries.isEmpty
                ? Center(
                    child: Text(
                      l10n.noMatches,
                      style: t.small.copyWith(color: t.textMuted),
                    ),
                  )
                : ListView.builder(
                    primary: false,
                    itemCount: _entries.length,
                    itemBuilder: (context, i) {
                      final e = _entries[i];
                      final heading = e.heading;
                      if (heading != null) {
                        return Padding(
                          padding:
                              EdgeInsets.fromLTRB(10, i == 0 ? 2 : 8, 10, 2),
                          child: Text(
                            heading,
                            style: t.small.copyWith(color: t.textMuted),
                          ),
                        );
                      }
                      final idx = e.optionIndex!;
                      return MenuRow(
                        selected: idx == widget.value,
                        onPressed: () => widget.onPick(idx),
                        child: Text(
                          widget.options[idx],
                          overflow: TextOverflow.ellipsis,
                        ),
                      );
                    },
                  ),
          ),
        ],
      ),
    );
  }

  @override
  void initState() {
    super.initState();
    _query.addListener(() {
      setState(() => _entries = _build(_query.text));
    });
  }
}

/// A [BareDropdown] whose option list is built only when the menu opens.
///
/// For pickers whose options are bridge reads (the Timeline's parent picker):
/// the resting button then costs nothing per rebuild, and the reads happen
/// once per click instead of once per rebuild.
class BareLazyDropdown<T> extends StatelessWidget {
  /// What the closed button shows.
  final String label;

  /// The options, as (value, label) pairs — called when the menu opens.
  final List<(T, String)> Function() options;
  final ValueChanged<T> onChanged;

  /// The in-row face: 16 tall with a 10px label, for a picker that sits inside
  /// a Timeline row rather than in a dialog or a bar (§12A.6, K-451).
  final bool dense;

  const BareLazyDropdown({
    super.key,
    required this.label,
    required this.options,
    required this.onChanged,
    this.dense = false,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return dropdownButton(
      t: t,
      dense: dense,
      onPressed: () async {
        final box = context.findRenderObject()! as RenderBox;
        final origin = box.localToGlobal(Offset.zero);
        final built = options();
        final picked = await showLumitPopup<(T,)>(
          context: context,
          position: origin + Offset(0, box.size.height + 2),
          builder: (close) => FloatSurface(
            child: IntrinsicWidth(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final (value, optionLabel) in built)
                    MenuRow(
                      selected: optionLabel == label,
                      // Wrapped in a record so a null value survives the
                      // popup's null-means-dismissed contract.
                      onPressed: () => close((value,)),
                      child: Text(optionLabel),
                    ),
                ],
              ),
            ),
          ),
        );
        if (picked != null) onChanged(picked.$1);
      },
      face: dropdownFace(t, label),
    );
  }
}

class _CaretPainter extends CustomPainter {
  final Color color;
  const _CaretPainter(this.color);
  @override
  void paint(Canvas canvas, Size size) {
    final p = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5
      ..strokeCap = StrokeCap.round;
    final w = size.width, h = size.height;
    canvas.drawLine(Offset(w * 0.2, h * 0.35), Offset(w * 0.5, h * 0.65), p);
    canvas.drawLine(Offset(w * 0.5, h * 0.65), Offset(w * 0.8, h * 0.35), p);
  }

  @override
  bool shouldRepaint(_CaretPainter old) => old.color != color;
}

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

/// A single-line text box in the house style. The dialogs each grew their own
/// copy of this; it belongs here.
class HouseTextField extends StatefulWidget {
  final TextEditingController controller;
  final double width;
  final ValueChanged<String>? onSubmitted;
  final bool submitOnLostFocus;

  /// A pointer went down somewhere that is not this field. What an inline
  /// rename commits on: clicking away is a person finishing the edit, and a
  /// field that kept what was typed only when `Enter` was pressed threw the
  /// work away for everyone who clicks instead (K-243).
  final VoidCallback? onTapOutside;

  /// `Escape`: throw the edit away and close the editor, keeping what the
  /// thing was called before. The counterpart to [onSubmitted] — every other
  /// way out of an inline rename *commits* (Enter, clicking away, K-243), so
  /// without this there is no way to change your mind, and Escape fell through
  /// to the modal dismissal that an inline editor has no modal for.
  ///
  /// Handled on the field's own focus node, ahead of the shortcut system, so
  /// it cannot be swallowed by `EditableText`'s own `DismissIntent` handling.
  final VoidCallback? onCancelled;
  final TextStyle? style;
  final ExpressionAutofillGenerator? autofill;

  /// Grab focus on first build — for fields that appear in response to a
  /// gesture (an inline rename), where a second click to focus would be
  /// asking the user to say it twice.
  final bool autofocus;

  /// The field's focus, owned by the caller — for a caller that has to steer
  /// it after build (the FX console keeps its field focused for its whole
  /// life, K-328). Null and the field makes and disposes its own, as every
  /// other caller wants.
  final FocusNode? focusNode;

  /// Muted placeholder shown while the field is empty — what the field is
  /// *for*, on fields whose surroundings do not already say.
  final String? hint;

  /// A mark inside the well, before the text — the search glyph on a field
  /// whose job is searching (§12A.1). Decorative: it takes no pointer, so a
  /// click on it still lands in the field behind it.
  final Widget? leading;

  /// Which end of the well the text sits at. The default reads from the start,
  /// which is what a name or a search term wants; a **number** reads from the
  /// right, so the digits of one line up with the digits of the next — the
  /// drawings right-align every numeric well they draw (the composition's
  /// frame rate, its size, its shutter angle).
  final TextAlign textAlign;

  /// The well's own inset. Overridden by the one caller that has to fit a
  /// **secondary row** (K-451: 18 px — the Timeline's timecode/search/mode
  /// row), where the default 3 px above and below would burst it.
  final EdgeInsets padding;

  /// The well's fill, for the two grounds the mockups actually draw. The
  /// default `surface0` is the recess every well takes (§2.1) — the Timeline's
  /// layer search, the ease popup's fields, an inline rename. A search well
  /// that sits *on* `surface1` with nothing else in its row takes `surface2`
  /// instead, which is the Project panel's (K-454: the manifests decide, and
  /// they disagree about this one on purpose — a well over a busy row has to
  /// sink, a well alone in its own row only has to be a well).
  final Color? fill;

  const HouseTextField({
    super.key,
    required this.controller,
    this.width = 200,
    this.padding = const EdgeInsets.symmetric(horizontal: 6, vertical: 3),
    this.fill,
    this.onSubmitted,
    this.submitOnLostFocus = false,
    this.onTapOutside,
    this.onCancelled,
    this.autofill,
    this.autofocus = false,
    this.focusNode,
    this.style,
    this.hint,
    this.leading,
    this.textAlign = TextAlign.start,
  });

  @override
  State<HouseTextField> createState() => _HouseTextFieldState();
}

class _HouseTextFieldState extends State<HouseTextField>
    implements TextSelectionGestureDetectorBuilderDelegate {
  late FocusNode _focus;
  final GlobalKey<EditableTextState> textFieldKey = GlobalKey();
  final layerLink = LayerLink();
  OverlayEntry? _overlay;

  @override
  void initState() {
    super.initState();
    _focus = widget.focusNode ?? FocusNode();
    _focus.onKeyEvent = onKeyEvent;
    // The hint draws only while empty, so emptiness changing must redraw.
    widget.controller.addListener(_changed);
    // And the edge answers focus, so taking or losing it must redraw too.
    _focus.addListener(_redraw);
  }

  void _redraw() {
    if (mounted) setState(() {});
  }

  List<dynamic> suggestions = List.empty();
  int? highlightedSuggestion;

  void _changed() {
    if (widget.autofill == null) {
      setState(() {});
      return;
    }

    setState(() {
      suggestions = widget.autofill!.getSuggestions(
          widget.controller.text, widget.controller.selection.baseOffset);
    });

    if (suggestions.isEmpty) {
      setState(() {
        highlightedSuggestion = null;
      });
      hideOverlay();
    } else {
      showOverlay();
    }
  }

  KeyEventResult onKeyEvent(FocusNode node, KeyEvent event) {
    // Escape first, and before the shortcut system sees it: an inline rename
    // is not a modal, so `DismissIntent` finds nothing to dismiss and the
    // editor used to sit there with no way out but committing.
    if (event is KeyDownEvent &&
        event.logicalKey == LogicalKeyboardKey.escape &&
        widget.onCancelled != null) {
      widget.onCancelled!();
      return KeyEventResult.handled;
    }
    if (suggestions.isNotEmpty) {
      if (event is! KeyDownEvent) {
        return KeyEventResult.ignored;
      }

      if (event.logicalKey == LogicalKeyboardKey.tab) {
        setState(() {
          if (highlightedSuggestion == null) {
            highlightedSuggestion = 0;
          } else {
            highlightedSuggestion =
                (highlightedSuggestion! + 1) % suggestions.length;
          }

          showOverlay();
        });
        return KeyEventResult.handled;
      }

      if (event.logicalKey == LogicalKeyboardKey.enter) {
        if (highlightedSuggestion != null) {
          setState(() {
            widget.autofill!.applySuggestion(
                suggestions[highlightedSuggestion!], widget.controller);

            highlightedSuggestion = null;
          });

          WidgetsBinding.instance.addPostFrameCallback((_) {
            textFieldKey.currentState!.bringIntoView(
                TextPosition(offset: widget.controller.selection.baseOffset));
          });

          hideOverlay();
          return KeyEventResult.handled;
        }
      }
    }

    return KeyEventResult.ignored;
  }

  void showOverlay() {
    if (_overlay != null) {
      hideOverlay();
    }

    final t = ThemeScope.of(context);
    _overlay?.remove();
    _overlay = null;
    _overlay = OverlayEntry(
      canSizeOverlay: true,
      builder: (c) {
        return Stack(
          children: [
            Material(
              // Fully transparent: the completion list draws its own surface
              // below, and Material is here only for the text style and ink.
              // Spelled as a zero colour rather than the Material palette's
              // named constant, which is a hex by another route and so is
              // refused by the design-token lint (docs/15-DESIGN.md §4.1).
              color: const Color(0x00000000),
              child: ThemeScope(
                  theme: t.theme,
                  animationLevel: t.animationLevel,
                  showTooltips: t.showTooltips,
                  child: CompositedTransformFollower(
                    link: layerLink,
                    offset: const Offset(-5, 16),
                    child: Container(
                      decoration: BoxDecoration(
                          color: t.theme.surface0,
                          border: BoxBorder.fromLTRB(
                              left: BorderSide(color: t.theme.selectionFill),
                              right: BorderSide(color: t.theme.selectionFill),
                              bottom: BorderSide(color: t.theme.selectionFill)),
                          borderRadius: t.theme.shape == ThemeShape.round
                              ? BorderRadius.only(
                                  bottomLeft: Radius.circular(8),
                                  bottomRight: Radius.circular(8))
                              : null),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          for (int i = 0; i < suggestions.length; i++)
                            HouseButton(
                              frameless: i != highlightedSuggestion,
                              onPressed: () {},
                              child: widget.autofill?.buildSuggestion(
                                      suggestions[i], t.theme) ??
                                  Text(suggestions[i].word),
                            )
                        ],
                      ),
                    ),
                  )),
            ),
          ],
        );
      },
    );

    Overlay.of(context, rootOverlay: true).insert(_overlay!);
  }

  void hideOverlay() {
    _overlay?.remove();
    _overlay = null;
  }

  @override
  void dispose() {
    widget.controller.removeListener(_changed);
    _focus.removeListener(_redraw);
    // The completion list is an OverlayEntry, which lives in the Overlay rather
    // than under this widget — so it outlives the field that opened it unless
    // it is taken down here, and a field disposed with suggestions showing
    // leaves them on screen over whatever comes next.
    hideOverlay();
    if (widget.focusNode == null) {
      _focus.dispose();
    } else {
      // A borrowed node goes back the way it came: handler detached, life
      // still the caller's.
      _focus.onKeyEvent = null;
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final hint = widget.hint;
    final leading = widget.leading;

    return Container(
      width: widget.width,
      padding: widget.padding,
      // Fill the height the caller gives (a well is its stated height, not
      // its text's): with an alignment the box expands to bounded
      // constraints instead of shrink-wrapping the 11px line — the project
      // panel's 20px search well rendered 16 without this.
      alignment: Alignment.centerLeft,
      decoration: BoxDecoration(
        color: widget.fill ?? t.surface0,
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        // `animated`, not `accent`: a focused well is the one focus that means
        // "you are about to change a value" (§3.1, §6.5), and the drawings
        // draw the focused well's edge in that token. [DragValueField] has
        // answered focus this way all along; a well you type into rather than
        // scrub had simply never answered at all.
        border: Border.all(color: _focus.hasFocus ? t.animated : t.hairline),
      ),
      child: _withLeading(
        leading,
        Stack(
          children: [
            if (hint != null && widget.controller.text.isEmpty)
              Text(hint, style: t.body.copyWith(color: t.textMuted)),
            // Focus on the *down* stroke, not the resolved tap: a press that
            // slides straight into a drag is someone selecting text in one
            // motion, and the field must already be theirs when the drag's
            // highlight starts (K-319).
            Listener(
              onPointerDown: (_) {
                if (!_focus.hasFocus) _focus.requestFocus();
              },
              child: TextSelectionGestureDetectorBuilder(delegate: this)
                  .buildGestureDetector(
                child: CompositedTransformTarget(
                  link: layerLink,
                  child: EditableText(
                    key: textFieldKey,
                    controller: widget.controller,
                    focusNode: _focus,
                    autofocus: widget.autofocus,
                    style: widget.style ?? t.bodyPrimary,
                    textAlign: widget.textAlign,
                    cursorColor: t.accent,
                    backgroundCursorColor: t.surface2,
                    selectionColor: t.accent.withValues(alpha: 0.5),
                    onSubmitted: widget.onSubmitted,
                    selectionControls: desktopTextSelectionHandleControls,
                    onTapOutside: (event) {
                      if (widget.submitOnLostFocus) {
                        widget.onSubmitted?.call(widget.controller.text);
                      }
                      // K-243: clicking away is a person finishing the edit, so an
                      // inline rename commits on it rather than throwing the work
                      // away for everyone who does not press Enter.
                      widget.onTapOutside?.call();
                      _focus.unfocus();
                      hideOverlay();
                    },
                  ),
                ),
              ),
            )
          ],
        ),
      ),
    );
  }

  /// The well's contents, with [leading] before them when there is one.
  static Widget _withLeading(Widget? leading, Widget field) => leading == null
      ? field
      : Row(children: [
          IgnorePointer(child: leading),
          const SizedBox(width: 5),
          Expanded(child: field),
        ]);

  @override
  GlobalKey<EditableTextState> get editableTextKey => textFieldKey;

  @override
  bool get forcePressEnabled => false;

  @override
  bool get selectionEnabled => true;
}

/// A menu row that opens a submenu beside it (K-194).
///
/// The parent menu stays open underneath while the submenu is up — closing it
/// first would take this row's `BuildContext` with it, and the overlay the
/// submenu needs is reached *through* that context. Picking something in the
/// submenu dismisses both.
class SubmenuRow extends StatefulWidget {
  final Widget child;

  /// Closes the menu this row belongs to.
  final VoidCallback closeParent;

  /// Builds the submenu's surface. `dismiss` closes the submenu *and* the
  /// parent, which is what picking an item means.
  final Widget Function(VoidCallback dismiss) submenu;

  const SubmenuRow({
    super.key,
    required this.child,
    required this.closeParent,
    required this.submenu,
  });

  @override
  State<SubmenuRow> createState() => _SubmenuRowState();
}

class _SubmenuRowState extends State<SubmenuRow> {
  ValueNotifier<Object?>? _hovered;

  /// True from the moment the flyout is asked for; [_close] arrives a frame
  /// later, when the overlay builds it.
  bool _out = false;
  VoidCallback? _close;

  /// The open flyout's surface, measured after it builds — what the parent
  /// surface's safe triangle points at (K-318).
  final GlobalKey _flyoutKey = GlobalKey();

  /// The parent surface this row sits on, resolved in build. Held as a state
  /// reference so the flyout's callbacks and dispose can reach it without a
  /// context lookup mid-teardown.
  _FloatSurfaceState? _surface;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    _surface = FloatSurface._of(context);
    final hovered = FloatSurface.hoveredRow(context);
    if (hovered != _hovered) {
      _hovered?.removeListener(_hoverMoved);
      _hovered = hovered?..addListener(_hoverMoved);
    }
    return MenuRow(
      hoverId: this,
      onPressed: _open,
      child: Row(
        children: [
          Expanded(child: widget.child),
          Text('›', style: t.body.copyWith(color: t.textMuted)),
        ],
      ),
    );
  }

  void _hoverMoved() {
    if (_hovered?.value == this) {
      _open();
    } else {
      _out = false;
      _close?.call();
      _close = null;
    }
  }

  /// Hand the flyout's measured rectangle to the parent surface, so its
  /// hover-intent guard knows what the pointer is travelling towards.
  void _armGuard() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!_out) return;
      final box = _flyoutKey.currentContext?.findRenderObject();
      if (box is! RenderBox || !box.attached) return;
      _surface?._armFlyoutGuard(
          this, box.localToGlobal(Offset.zero) & box.size);
    });
  }

  void _open() {
    if (_out) return;
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    // Beside the row, overlapping it slightly, the way a flyout sits.
    final at = box.localToGlobal(Offset(box.size.width - 6, -4));
    _out = true;
    showLumitPopup<void>(
      context: context,
      position: at,
      // So the menu underneath still feels the pointer: moving to another row
      // is what takes this flyout back.
      hoverThrough: true,
      builder: (close) {
        _close = () => close(null);
        _armGuard();
        return MouseRegion(
          // Reaching the flyout settles the intent: any switch pending on the
          // rows crossed on the way is void.
          onEnter: (_) => _surface?._flyoutEntered(this),
          child: KeyedSubtree(
            key: _flyoutKey,
            child: widget.submenu(() {
              close(null);
              widget.closeParent();
            }),
          ),
        );
      },
    ).then((_) {
      _out = false;
      _close = null;
      _surface?._disarmFlyoutGuard(this);
    });
  }

  @override
  void dispose() {
    _hovered?.removeListener(_hoverMoved);
    // The menu this row belongs to has gone (another heading took over, say);
    // its flyout goes with it rather than being left behind. After the frame,
    // because removing an overlay entry sets the overlay's state and this is
    // the middle of a tear-down.
    final close = _close;
    if (close != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) => close());
    }
    super.dispose();
  }
}

/// One popup on the chain: the handle that takes it back down again.
class _PopupHandle {
  _PopupHandle(this.dismiss);

  /// Closes this popup with nothing picked, exactly as clicking away does.
  final VoidCallback dismiss;
}

/// **The one chain of open popups, outermost first** (K-519).
///
/// Every menu, dropdown, picker and flyout in the application goes through
/// [showLumitPopup], and each call used to push an overlay entry of its own
/// with a click-away barrier of its own and no idea the others existed. Move
/// the pointer quickly across the menu bar, the Add-effect list and a picker
/// and you could end up with three menus on screen at once, each wanting its
/// own click to dismiss.
///
/// This list is the missing authority. A popup raised from a context *inside*
/// an open popup — a submenu flying out of a menu row — extends the chain;
/// one raised from anywhere else starts a new chain, which closes whatever was
/// up. One click on a barrier, or one Escape, takes the whole chain.
final List<_PopupHandle> _popupChain = [];

/// Close every popup at [depth] and deeper, innermost first.
void _truncatePopups(int depth) {
  while (_popupChain.length > depth) {
    _popupChain.removeLast().dismiss();
  }
  if (_popupChain.isEmpty) {
    _popupEscapeRelease?.call();
    _popupEscapeRelease = null;
  }
}

/// Take down every open menu, dropdown and flyout.
void closeLumitPopups() => _truncatePopups(0);

/// Whether any popup is up. The menus are not a modal — panels keep their
/// keyboard — so this is for tests and for Escape, not for gating commands.
bool get lumitPopupOpen => _popupChain.isNotEmpty;

/// Escape while a chain is up dismisses all of it.
///
/// The ladder's popup rung (widgets/escape_ladder.dart), not Flutter's own
/// [DismissIntent], because a menu holds no focus: the intent would be
/// dispatched at whatever widget the focus was on when the menu opened, which
/// is outside the popup's subtree, so an `Actions` entry there would never be
/// reached. Registered only while a chain is open, so nothing else pays for it
/// — and below the gesture rung, so a menu open over a drag in flight is not
/// what one press takes.
VoidCallback? _popupEscapeRelease;

bool _popupEscape() {
  if (_popupChain.isEmpty) return false;
  closeLumitPopups();
  return true;
}

/// Marks a popup's own subtree, so a popup raised from inside one knows it is
/// a flyout of that popup rather than a new chain.
class _PopupScope extends InheritedWidget {
  final int depth;
  const _PopupScope({required this.depth, required super.child});

  @override
  bool updateShouldNotify(_PopupScope old) => old.depth != depth;
}

/// A window point in the coordinate space of the overlay that is going to draw
/// it (K-560).
///
/// **In plain terms.** A control says where it is with `localToGlobal`, which
/// answers in window pixels. A popup is not laid out in the window, though — it
/// is laid out inside an [Overlay], and the two only agree while nothing
/// between them moves or resizes the picture. The UI scale does exactly that
/// (widgets/ui_scale.dart), so at 125% a menu opened halfway down the window
/// was placed a quarter of the way further down again. Asking the overlay's own
/// box to convert the point undoes whatever is between the two, at any scale,
/// and is the identity when there is nothing.
///
/// An overlay with no box yet — nothing has laid it out — leaves the point
/// alone, which is the old behaviour and always right at 1×.
Offset overlayLocal(BuildContext context, Offset windowPoint) {
  final box = Overlay.of(context).context.findRenderObject();
  return box is RenderBox && box.hasSize
      ? box.globalToLocal(windowPoint)
      : windowPoint;
}

Future<T?> showLumitPopup<T>({
  required BuildContext context,
  // Where the popup is anchored, in **window** coordinates — what a control's
  // `localToGlobal` hands back. It is converted into the overlay's own space
  // here (K-560), once, so no call site has to know what is between them.
  required Offset position,
  required Widget Function(void Function(T?) close) builder,
  // Whether what is underneath still feels the pointer while this popup is up.
  // Menus want it — hovering another heading or another row is how a menu is
  // navigated — and nothing else does: a dropdown that let the panel behind it
  // light up under the pointer would be answering to a click it will not get.
  bool hoverThrough = false,
}) {
  final overlay = Overlay.of(context);
  final anchor = overlayLocal(context, position);
  final completer = Completer<T?>();
  late OverlayEntry entry;
  late _PopupHandle handle;
  void close(T? v) {
    if (completer.isCompleted) return;
    completer.complete(v);
    entry.remove();
    // Anything this popup itself opened goes with it.
    final at = _popupChain.indexOf(handle);
    if (at >= 0) _truncatePopups(at);
  }

  // Where this popup sits on the chain: one deeper than the popup it was
  // raised from, or the root when it was raised from outside every popup —
  // in which case the chain that was up is dismissed first.
  final parentDepth =
      context.getInheritedWidgetOfExactType<_PopupScope>()?.depth ?? -1;
  _truncatePopups(parentDepth + 1);
  final depth = _popupChain.length;
  handle = _PopupHandle(() => close(null));
  _popupChain.add(handle);
  if (_popupChain.length == 1) {
    _popupEscapeRelease = EscapeLadder.register(EscapeRung.popup, _popupEscape);
  }

  entry = OverlayEntry(
    // LayoutBuilder, not MediaQuery: what matters is the room the overlay
    // actually has, and the two disagree wherever a MediaQuery has been
    // overridden. A popup taller than that room would run off the bottom of the
    // window with its last rows unreachable — so it is capped at the space
    // below its own top edge, and scrolls inside that if it needs to.
    builder: (_) => LayoutBuilder(
      builder: (context, constraints) => Stack(
        children: [
          Positioned.fill(
            // Translucent still takes the click — it is above whatever it
            // covers, so it wins the gesture arena — but lets hover through to
            // the menu bar and to the menu this one flew out of.
            child: GestureDetector(
              behavior: hoverThrough
                  ? HitTestBehavior.translucent
                  : HitTestBehavior.opaque,
              // One click away is one dismissal: the whole chain goes, not
              // just the popup whose barrier caught the click (K-519).
              onTap: closeLumitPopups,
              onSecondaryTap: closeLumitPopups,
            ),
          ),
          Positioned.fill(
            child: CustomSingleChildLayout(
              delegate: _PopupLayout(anchor),
              // Scrolls only when it has to: a shorter popup shrink-wraps and
              // behaves exactly as before.
              child: SingleChildScrollView(
                child: _PopupScope(depth: depth, child: builder(close)),
              ),
            ),
          ),
        ],
      ),
    ),
  );
  overlay.insert(entry);
  return completer.future;
}

/// Places a popup at its anchor, then pulls it back on screen if it would hang
/// off an edge.
///
/// Anchoring alone was enough while every popup opened from the top of the
/// window. A control near the bottom — the Viewer's transport, now that it sits
/// under the picture — opens a list that would run off the bottom entirely, so
/// the whole thing is shifted up until it fits. The same applies sideways for a
/// control near the right edge.
class _PopupLayout extends SingleChildLayoutDelegate {
  final Offset anchor;
  const _PopupLayout(this.anchor);

  @override
  BoxConstraints getConstraintsForChild(BoxConstraints constraints) =>
      BoxConstraints.loose(Size(
        constraints.maxWidth,
        // Never taller than the room above *or* below the anchor, whichever the
        // popup ends up using — the larger of the two is the most it can need.
        (constraints.maxHeight - 16).clamp(80.0, double.infinity),
      ));

  @override
  Offset getPositionForChild(Size size, Size childSize) => Offset(
        anchor.dx.clamp(0.0, (size.width - childSize.width).clamp(0.0, 1e6)),
        anchor.dy.clamp(0.0, (size.height - childSize.height).clamp(0.0, 1e6)),
      );

  @override
  bool shouldRelayout(_PopupLayout old) => old.anchor != anchor;
}

/// A 14 px themed checkbox.
class HouseCheckbox extends StatefulWidget {
  final bool value;
  final ValueChanged<bool> onChanged;
  const HouseCheckbox(
      {super.key, required this.value, required this.onChanged});

  @override
  State<HouseCheckbox> createState() => _HouseCheckboxState();
}

class _HouseCheckboxState extends State<HouseCheckbox> {
  bool _focused = false;
  final ControlFocusNode _focusNode = ControlFocusNode(debugLabel: 'checkbox');

  @override
  void dispose() {
    _focusNode.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FocusableActionDetector(
      focusNode: _focusNode,
      shortcuts: _activateShortcuts,
      actions: <Type, Action<Intent>>{
        ActivateIntent: CallbackAction<ActivateIntent>(onInvoke: (_) {
          widget.onChanged(!widget.value);
          return null;
        }),
      },
      onFocusChange: (has) => setState(() => _focused = has),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => widget.onChanged(!widget.value),
        // The mockups' checkbox (K-450): a 9px outlined box whose checked
        // state is a 5px block of the primary text colour — no accent, which
        // also puts the control inside §3.1's accent discipline. The 14px
        // container stays as the hit target around the smaller mark; the
        // focus ring reads in the animated token, like a focused value well.
        child: SizedBox(
          width: 14,
          height: 14,
          child: Center(
            child: Container(
              width: 9,
              height: 9,
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(1),
                border: Border.all(
                    color: _focused ? t.animated : t.textMuted,
                    width: _focused ? 1.5 : 1),
              ),
              child: widget.value
                  ? Center(
                      child: Container(
                        width: 5,
                        height: 5,
                        color: t.textPrimary,
                      ),
                    )
                  : null,
            ),
          ),
        ),
      ),
    );
  }
}

/// The pill switch the approved **dialog** drawings put on a plain on/off
/// setting: a 22×12 track with an 8px knob that slides from one end to the
/// other.
///
/// In plain terms, this is the switch you flick — the thing a phone's settings
/// screen uses — as opposed to [HouseCheckbox], the little box you tick. Both
/// say the same thing; which one a surface draws is the drawing's business,
/// and the Settings window's drawing asks for this.
///
/// On is `animated`, the amber the drawing computes, **not** the accent:
/// §3.1's accent discipline gives the accent to the single filled action and
/// to focus, and a page of switches would spend it a dozen times over. Off is
/// the same `hairline_strong` rule every inert edge takes.
class HouseToggle extends StatefulWidget {
  final bool value;
  final ValueChanged<bool> onChanged;
  const HouseToggle({super.key, required this.value, required this.onChanged});

  @override
  State<HouseToggle> createState() => _HouseToggleState();
}

class _HouseToggleState extends State<HouseToggle> {
  bool _focused = false;
  final ControlFocusNode _focusNode = ControlFocusNode(debugLabel: 'toggle');

  /// The track, and the knob that runs inside it. The drawing's own numbers.
  static const double _width = 22;
  static const double _height = 12;
  static const double _knob = 8;
  static const double _inset = 2;

  @override
  void dispose() {
    _focusNode.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    return FocusableActionDetector(
      focusNode: _focusNode,
      shortcuts: _activateShortcuts,
      actions: <Type, Action<Intent>>{
        ActivateIntent: CallbackAction<ActivateIntent>(onInvoke: (_) {
          widget.onChanged(!widget.value);
          return null;
        }),
      },
      onFocusChange: (has) => setState(() => _focused = has),
      mouseCursor: SystemMouseCursors.click,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => widget.onChanged(!widget.value),
        // The focus ring is drawn on a box *around* the pill rather than on
        // the pill itself, so taking focus never moves the switch by a pixel —
        // and the 26×16 box is a kinder hit target than a 22×12 one.
        child: Container(
          width: _width + 4,
          height: _height + 4,
          alignment: Alignment.center,
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(_height / 2 + 2),
            border: Border.all(
              color: _focused ? t.accent : const Color(0x00000000),
            ),
          ),
          child: AnimatedContainer(
            duration: animationDuration(scope.animationLevel),
            width: _width,
            height: _height,
            decoration: BoxDecoration(
              color: widget.value ? t.animated : t.hairlineStrong,
              borderRadius: BorderRadius.circular(_height / 2),
            ),
            child: Stack(
              children: [
                AnimatedPositioned(
                  duration: animationDuration(scope.animationLevel),
                  left: widget.value ? _width - _knob - _inset : _inset,
                  top: _inset,
                  child: Container(
                    width: _knob,
                    height: _knob,
                    decoration: BoxDecoration(
                      color: t.surface0,
                      borderRadius: BorderRadius.circular(_knob / 2),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// One of a set of choices, where the set is exclusive — the dot beside a
/// sentence. [HouseCheckbox] is the independent one; this is the one that says
/// "this, and therefore not that". Disabled it still shows which way the
/// choice fell, dimmed, rather than going blank.
class HouseRadio extends StatefulWidget {
  final bool selected;
  final bool enabled;
  final VoidCallback? onChanged;

  const HouseRadio({
    super.key,
    required this.selected,
    this.enabled = true,
    this.onChanged,
  });

  @override
  State<HouseRadio> createState() => _HouseRadioState();
}

class _HouseRadioState extends State<HouseRadio> {
  bool _focused = false;
  final ControlFocusNode _focusNode = ControlFocusNode(debugLabel: 'radio');

  @override
  void dispose() {
    _focusNode.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final borderColor = !widget.enabled
        ? t.textMuted.withValues(alpha: 0.4)
        : (_focused || widget.selected ? t.accent : t.hairlineStrong);

    return FocusableActionDetector(
      focusNode: _focusNode,
      enabled: widget.enabled,
      shortcuts: _activateShortcuts,
      actions: <Type, Action<Intent>>{
        ActivateIntent: CallbackAction<ActivateIntent>(onInvoke: (_) {
          widget.onChanged?.call();
          return null;
        }),
      },
      onFocusChange: (has) => setState(() => _focused = has),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: widget.enabled ? widget.onChanged : null,
        child: Container(
          width: 14,
          height: 14,
          decoration: BoxDecoration(
            color: t.surface3,
            shape: BoxShape.circle,
            border: Border.all(color: borderColor, width: 1.5),
          ),
          alignment: Alignment.center,
          child: widget.selected
              ? Container(
                  width: 6,
                  height: 6,
                  decoration: BoxDecoration(
                    color: widget.enabled ? t.accent : t.textMuted,
                    shape: BoxShape.circle,
                  ),
                )
              : null,
        ),
      ),
    );
  }
}

/// The **modifier ladder** a value scrub runs on, coarsest first — the study's
/// four rungs (`Caddis study/notes-editor-ux.md` §3, docs/impl
/// /timeline-interaction.md polish 27), which are After Effects' own two with
/// a finer one under them: `Shift` ×10, nothing held ×1, `Ctrl` ×0.1, `Alt`
/// ×0.01.
///
/// Ordered, because the list is both the arithmetic and the chip's drawing:
/// [ScrubLadder] boxes whichever rung [scrubFactor] is answering.
const List<double> scrubLadder = [10, 1, 0.1, 0.01];

/// How much a scrub tick is worth right now, from the modifier keys — the
/// [scrubLadder]'s four rungs. Sampled inside the drag handler on every
/// update, so pressing or releasing a modifier mid-drag takes effect at once.
///
/// Coarse beats fine where two are held at once: `Shift` first, then `Alt`,
/// then `Ctrl`. A ladder needs one answer, and the one the hand meant is the
/// one it pressed on purpose — which, with two held, cannot be told apart, so
/// the order is fixed here rather than guessed.
double scrubFactor() => HardwareKeyboard.instance.isShiftPressed
    ? 10
    : HardwareKeyboard.instance.isAltPressed
        ? 0.01
        : HardwareKeyboard.instance.isControlPressed
            ? 0.1
            : 1;

/// The floating **sensitivity ladder** shown while a value scrub runs (polish
/// 27, study §3): all four rungs at once, the one in force boxed, so the
/// modifier that makes a drag fine is learned by using the field rather than
/// by reading the manual.
///
/// Transient and local (P1): [DragValueField] puts it up on the pointer's way
/// down and takes it down on release, and the resting panel keeps every pixel
/// it had.
class ScrubLadder extends StatelessWidget {
  /// What [scrubFactor] answers right now — which rung is boxed.
  final double factor;

  const ScrubLadder({super.key, required this.factor});

  /// A rung's label. Ordered as [scrubLadder] is.
  static List<String> get labels => [
        l10n.scrubLadderShift,
        l10n.scrubLadderBase,
        l10n.scrubLadderCtrl,
        l10n.scrubLadderAlt,
      ];

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // The hint pill's own ground and face (§4.2): every readout a gesture
    // summons in this application is 8px mono on `surface_4`.
    return IgnorePointer(
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 2, vertical: 2),
        decoration: BoxDecoration(
          color: t.surface4,
          borderRadius: BorderRadius.circular(2),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            // Coarsest on the right, finest on the left: the chip reads the
            // way the drag does, and the rungs sit in the order the study
            // draws them (ALT · CTRL · BASE · SHIFT).
            for (var i = scrubLadder.length - 1; i >= 0; i--)
              Container(
                margin: const EdgeInsets.symmetric(horizontal: 1),
                padding: const EdgeInsets.symmetric(horizontal: 3),
                decoration: BoxDecoration(
                  // The box is the whole mark: no fill, no colour beyond the
                  // one selection speaks in (P4).
                  border: Border.all(
                    color: factor == scrubLadder[i]
                        ? t.textPrimary
                        : const Color(0x00000000),
                  ),
                  borderRadius: BorderRadius.circular(2),
                ),
                child: Text(
                  labels[i],
                  style: t.mono.copyWith(
                    fontSize: 8,
                    color:
                        factor == scrubLadder[i] ? t.textPrimary : t.textMuted,
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// A value well's height in a panel: **20** (K-451, docs/15 §12A.6). Fixed
/// rather than grown from the number inside it, because the mockups' heights
/// are canonical and a well that measured its own font drifted with the face.
/// Dialog wells are 22 and set their own; they do not come through here yet.
const double wellHeight = 20;

/// The number inside a well: **11px mono**, the size the approved mockups
/// compute for every `.well` they draw (§7.1's mono row, K-454). It had been
/// 13, which is a size the mockups use nowhere and which crowded the well's
/// 20 from the inside.
const double wellTextSize = 11;

/// The number on a **bar**, where the drawing gives it no well: 10px mono, the
/// Viewer bottom bar's own `+0.0` (K-466). A bar reading is an aside beside the
/// picture, and it is set a size down from a panel's editable value.
const double barValueTextSize = 10;

/// The **value well** (docs/15-DESIGN.md §2.1/§3.1, K-439): drag horizontally
/// to adjust, click to type, right-click for Reset / Copy / Paste.
///
/// In plain terms, a number you can edit is drawn as a *recess* rather than as
/// a raised box — a `surface0` fill, darker than the panel around it, inside a
/// hairline. The well is what says "editable", so a resting panel keeps to its
/// three greys however many numbers it carries, and no colour has to be spent
/// saying it. The number itself is mono at [wellTextSize] (§7.1's absolute
/// rule, and its property-value row) and turns `accent` while it is actually
/// being dragged, `animated` when the property is keyed ([keyed]).
///
/// [resetTo] is the field's known default — Reset appears only when a call site
/// supplies one.
class DragValueField extends StatefulWidget {
  final num value;
  final num min;
  final num max;
  final double speed;
  final int decimals;
  final String? suffix;
  final num? resetTo;

  /// Whether a positive value is shown with its `+`.
  ///
  /// For a field whose zero is a *middle* rather than a floor — the Viewer's
  /// exposure in stops (K-314), which reads `+1.4` and `-2.3` — so the sign is
  /// part of the reading and the number does not appear to jump width when it
  /// crosses zero. Display only: what is typed, copied and pasted is the plain
  /// number, and `+1.4` parses as readily as `1.4`.
  final bool signed;

  /// The property this well edits has keyframes on it, so the number rests in
  /// `animated` rather than `text_primary` (§3.1). A live drag still wins: a
  /// value in hand is `accent` whether or not it is keyed.
  final bool keyed;

  /// The well's own fill, for the rare ground `surface0` cannot sit on. It is
  /// the inset every well now takes by default (§2.1), so a call site has no
  /// reason to pass anything.
  final Color? fill;

  /// Drawn **bare**: no inset, no hairline, the number alone at a bar's own
  /// 10px in `text_secondary` (K-466).
  ///
  /// One caller, and it is a measurement rather than a taste: the approved
  /// Viewer drawing sets the exposure as a plain `.mono` span on the bottom
  /// bar, with no background and no border, where every other editable number
  /// in the application rests in a well. A 20px well in a 22px bar would leave
  /// a pixel of ground above and below it and read as the bar's own edge.
  /// Everything else about the field is unchanged — the scrub, the modifier
  /// ladder, click-to-type, the context menu — and the drag and focus colours
  /// still speak, through the number rather than through an edge it has not
  /// got.
  final bool bare;
  final ValueChanged<num> onChanged;

  /// Fired once when a drag begins. Optional — a caller with nothing to do at
  /// drag-start (the common case) simply omits it.
  final VoidCallback? onChangeStart;

  /// Fired with the live value on every accumulated drag tick, in place of
  /// [onChanged], when supplied (a live-preview fast path — see
  /// [onChangeEnd]). Falls back to [onChanged] when null, so every existing
  /// call site behaves exactly as before.
  final ValueChanged<num>? onChangeLive;

  /// Fired once, with the final value, when a drag ends (mouse-up). Falls
  /// back to [onChanged] when null. Reset/Copy/Paste and the text-edit commit
  /// always call [onChanged] directly and never this — they are already
  /// one-shot edits, not a drag.
  final ValueChanged<num>? onChangeEnd;

  /// Fired when a drag is cancelled (a gesture cancel, or a released drag
  /// that never crossed one [speed] increment — so nothing was ever ticked).
  final VoidCallback? onDragCancel;

  const DragValueField({
    super.key,
    required this.value,
    required this.min,
    required this.max,
    required this.onChanged,
    this.speed = 1,
    this.decimals = 0,
    this.suffix,
    this.resetTo,
    this.signed = false,
    this.keyed = false,
    this.fill,
    this.bare = false,
    this.onChangeStart,
    this.onChangeLive,
    this.onChangeEnd,
    this.onDragCancel,
    this.setExpression,
  });

  /// Offered in the value's context menu when the property can take one.
  /// Absent (and the menu entry with it) for a field that cannot.
  final VoidCallback? setExpression;

  @override
  State<DragValueField> createState() => _DragValueFieldState();
}

/// What a value well makes of what was typed into it: a number, or a sum
/// (Caddis A3). `(1920-100)*0.5` commits 910.
///
/// **In plain terms.** Half of what a person types into a size or a position
/// is arithmetic they did in their head first — half of this, that less the
/// margin, twice the frame. The well now does the sum instead, over `+ - * /`,
/// brackets and a leading minus, with multiplication binding tighter than
/// addition the way it does on paper.
///
/// A plain number is read by [num.tryParse] exactly as before, so nothing
/// about typing a number changes — including the forms the parser knows and
/// this one deliberately does not, such as `1e3`. Anything that is not a sum
/// this understands, and any division by zero, comes back null, which is the
/// answer a well already had for text it could not read: keep the value it
/// has, say nothing, punish nobody.
num? parseNumberField(String text) {
  final plain = num.tryParse(text.trim());
  if (plain != null) return plain;
  return _Arithmetic(text.replaceAll(' ', '')).parse();
}

/// A recursive descent over the six symbols a value well needs, which is the
/// whole grammar — no dependency for four small methods.
class _Arithmetic {
  _Arithmetic(this._src);
  final String _src;
  int _at = 0;

  num? parse() {
    final value = _sum();
    // Trailing rubbish (`3+4x`) is not a sum with a tail, it is a mistake; and
    // a division by zero arrives here as an infinity or a NaN.
    if (value == null || _at != _src.length || !value.isFinite) return null;
    return value;
  }

  double? _sum() {
    var left = _product();
    while (left != null && _at < _src.length) {
      final op = _src[_at];
      if (op != '+' && op != '-') break;
      _at++;
      final right = _product();
      if (right == null) return null;
      left = op == '+' ? left + right : left - right;
    }
    return left;
  }

  double? _product() {
    var left = _atom();
    while (left != null && _at < _src.length) {
      final op = _src[_at];
      if (op != '*' && op != '/') break;
      _at++;
      final right = _atom();
      if (right == null) return null;
      left = op == '*' ? left * right : left / right;
    }
    return left;
  }

  double? _atom() {
    if (_at >= _src.length) return null;
    final c = _src[_at];
    if (c == '-' || c == '+') {
      _at++;
      final v = _atom();
      return v == null ? null : (c == '-' ? -v : v);
    }
    if (c == '(') {
      _at++;
      final v = _sum();
      if (v == null || _at >= _src.length || _src[_at] != ')') return null;
      _at++;
      return v;
    }
    final start = _at;
    while (_at < _src.length && _isNumberChar(_src.codeUnitAt(_at))) {
      _at++;
    }
    return _at == start ? null : double.tryParse(_src.substring(start, _at));
  }

  /// A digit or the decimal point. `double.tryParse` says whether what they
  /// spell is actually a number — `1.2.3` is not, and comes back null.
  static bool _isNumberChar(int unit) =>
      (unit >= 0x30 && unit <= 0x39) || unit == 0x2e;
}

class _DragValueFieldState extends State<DragValueField>
    implements TextSelectionGestureDetectorBuilderDelegate {
  bool _editing = false;
  bool _hover = false;
  bool _focused = false;

  /// A scrub is under the pointer right now — the one thing that turns the
  /// number `accent` (§3.1). Transient and local, like all feedback (§12A.5):
  /// it goes the moment the pointer lifts and leaves no trace behind.
  bool _dragging = false;
  double _dragAccum = 0;

  /// The last value ticked this drag (via [onChangeLive]/[onChanged]), or
  /// null before the first tick / after a commit or cancel. Distinguishes "a
  /// released drag that ticked at least once" (commit the last value) from "a
  /// released drag that never crossed one [DragValueField.speed] increment"
  /// (nothing to commit — a no-op cancel, which still opens the editor: the
  /// press was a click that wobbled, not a scrub).
  num? _lastDragValue;
  late TextEditingController _controller;
  late final FocusNode _focus = FocusNode(onKeyEvent: _onEditorKey);

  /// The floating [ScrubLadder], up only while a drag runs (polish 27).
  ///
  /// An overlay entry rather than a child of this field: the chip is bigger
  /// than the well it belongs to and every well in the application sits in a
  /// row that would either clip it or make room for it, and making room is a
  /// resting-state change (P1). Placed from the field's rect taken once at the
  /// down — a field does not move while it is being scrubbed — so a pointer
  /// move costs the overlay nothing.
  OverlayEntry? _ladder;

  /// Which rung is boxed, read afresh whenever a modifier goes down or up.
  /// A notifier rather than `setState`, so a modifier pressed mid-drag
  /// repaints the chip alone and not the panel behind it.
  final ValueNotifier<double> _factor = ValueNotifier<double>(1);

  /// Modifiers pressed and released **without the pointer moving** still change
  /// what the next pixel is worth, so the chip cannot wait for a drag update to
  /// find out. Never handles the key: it only looks.
  bool _ladderKey(KeyEvent event) {
    _factor.value = scrubFactor();
    return false;
  }

  void _showLadder() {
    final overlay = Overlay.maybeOf(context, rootOverlay: true);
    final box = context.findRenderObject();
    final overlayBox = overlay?.context.findRenderObject();
    // No overlay above this field (a bare widget-test host) simply means no
    // chip: the scrub itself is unaffected.
    if (overlay == null ||
        box is! RenderBox ||
        !box.hasSize ||
        overlayBox is! RenderBox) {
      return;
    }
    final top =
        box.localToGlobal(Offset(box.size.width / 2, 0), ancestor: overlayBox);
    final scope = ThemeScope.of(context);
    _factor.value = scrubFactor();
    _ladder = OverlayEntry(
      builder: (_) => Positioned(
        left: top.dx,
        top: top.dy - 4,
        // Centred over the field and sitting just above it, wherever that
        // leaves it: the chip is placed by its own bottom-centre, so a field
        // at the left edge of the window does not push it off screen.
        child: FractionalTranslation(
          translation: const Offset(-0.5, -1),
          // The overlay is above this field's own ThemeScope, so the chip is
          // handed the scope again on its way in.
          child: ThemeScope(
            theme: scope.theme,
            animationLevel: scope.animationLevel,
            showTooltips: scope.showTooltips,
            child: ValueListenableBuilder<double>(
              valueListenable: _factor,
              builder: (_, factor, __) => ScrubLadder(factor: factor),
            ),
          ),
        ),
      ),
    );
    overlay.insert(_ladder!);
    HardwareKeyboard.instance.addHandler(_ladderKey);
  }

  void _hideLadder() {
    HardwareKeyboard.instance.removeHandler(_ladderKey);
    _ladder?.remove();
    _ladder = null;
  }

  /// `Escape` in the open editor: shut it and keep the value the field had
  /// (K-323). Every other way out commits — Enter, Tab, clicking away — so
  /// without this a half-typed number had no way back.
  ///
  /// Clearing `_editing` first matters: the focus listener below commits on
  /// focus loss, and closing the editor is what loses it.
  KeyEventResult _onEditorKey(FocusNode node, KeyEvent event) {
    if (event is KeyDownEvent &&
        event.logicalKey == LogicalKeyboardKey.escape &&
        _editing) {
      setState(() => _editing = false);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  /// The idle box's focus — how Tab reaches the field, and what `Enter`
  /// opens the editor from (K-319).
  final ControlFocusNode _idleFocus = ControlFocusNode(debugLabel: 'value');

  /// The open editor, for the selection gestures: pressing in it puts the
  /// caret down and dragging highlights, the way any text box works.
  final GlobalKey<EditableTextState> textFieldKey = GlobalKey();

  @override
  GlobalKey<EditableTextState> get editableTextKey => textFieldKey;

  @override
  bool get forcePressEnabled => false;

  @override
  bool get selectionEnabled => true;

  @override
  void initState() {
    super.initState();
    _controller = TextEditingController();
    _focus.addListener(() {
      if (!_focus.hasFocus && _editing) _commitText();
    });
  }

  @override
  void dispose() {
    // The chip lives in the Overlay rather than under this field, so a field
    // disposed mid-drag would leave it on screen over whatever came next.
    _hideLadder();
    _factor.dispose();
    _controller.dispose();
    _focus.dispose();
    _idleFocus.dispose();
    super.dispose();
  }

  /// Open the text editor with the whole value selected — a value box is
  /// retyped far more often than it is amended, and a selected value means
  /// the first keystroke replaces it (K-319).
  void _beginEdit() {
    setState(() {
      _editing = true;
      final text = widget.decimals == 0
          ? widget.value.round().toString()
          : widget.value.toDouble().toStringAsFixed(widget.decimals);
      _controller.text = text;
      _controller.selection =
          TextSelection(baseOffset: 0, extentOffset: text.length);
    });
    _focus.requestFocus();
  }

  /// The face both states are set in: the resting reading and the open
  /// editor, so neither can drift from the other.
  TextStyle _valueStyle(LumitTheme t) =>
      t.mono.copyWith(fontSize: widget.bare ? barValueTextSize : wellTextSize);

  /// How wide the resting face draws — the reading's own width plus the well's
  /// padding and its edge.
  ///
  /// The reading is monospaced, so a character count is a width; [monoSlotWidth]
  /// is the same measurement the readouts use, and caches by face and length.
  double _restingWidth(LumitTheme t) =>
      monoSlotWidth(_valueStyle(t), _format(widget.value).length) +
      (widget.bare ? 0 : 12) +
      2;

  String _format(num v) {
    var s = _plain(v);
    // `toStringAsFixed` already carries a minus; only the plus has to be put
    // back, and only where the reading is signed.
    if (widget.signed && !s.startsWith('-')) s = '+$s';
    return widget.suffix == null ? s : '$s${widget.suffix}';
  }

  void _commitText() {
    final raw = _controller.text.replaceAll(widget.suffix ?? '', '').trim();
    final parsed = parseNumberField(raw);
    if (parsed != null) {
      widget.onChanged(parsed.clamp(widget.min, widget.max));
    }
    setState(() => _editing = false);
  }

  /// The plain numeric string (no suffix) — what Copy puts on the clipboard and
  /// what Paste parses back, so a value round-trips between fields.
  String _plain(num v) => widget.decimals == 0
      ? v.round().toString()
      : v.toDouble().toStringAsFixed(widget.decimals);

  /// The egui drag-value right-click menu: Reset (when a default is known),
  /// Copy and Paste, over the system clipboard with the field's own clamp.
  void _contextMenu(BuildContext context, Offset globalPos) {
    showLumitPopup<void>(
      context: context,
      position: globalPos,
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (widget.resetTo != null)
                MenuRow(
                  onPressed: () {
                    close(null);
                    widget.onChanged(
                        widget.resetTo!.clamp(widget.min, widget.max));
                  },
                  child: Text(l10n.reset),
                ),
              MenuRow(
                onPressed: () {
                  close(null);
                  Clipboard.setData(ClipboardData(text: _plain(widget.value)));
                },
                child: Text(l10n.menuCopy),
              ),
              MenuRow(
                onPressed: () async {
                  close(null);
                  final data = await Clipboard.getData(Clipboard.kTextPlain);
                  final raw =
                      data?.text?.replaceAll(widget.suffix ?? '', '').trim();
                  final parsed = raw == null ? null : parseNumberField(raw);
                  if (parsed != null) {
                    widget.onChanged(parsed.clamp(widget.min, widget.max));
                  }
                },
                child: Text(l10n.menuPaste),
              ),
              // Only where the property can actually hold one, so the menu on
              // a field that cannot never offers it.
              if (widget.setExpression != null)
                MenuRow(
                  onPressed: () {
                    close(null);
                    widget.setExpression?.call();
                  },
                  child: Text(l10n.setExpression),
                ),
            ],
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    if (_editing) {
      // **The editor is the resting face, with a caret in it.** Same box, same
      // padding, same border, same type, same right-hand anchor — because
      // anything else moves the number under the pointer that just clicked it,
      // which the owner read as jarring and was right to. It used to be a
      // fixed 72-wide box with the text against its *left* edge, so clicking
      // a well both resized the box and threw the digits across it.
      return SizedBox(
        width: _restingWidth(t),
        height: widget.bare ? null : wellHeight,
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: widget.bare ? null : widget.fill ?? t.surface0,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            // `animated`, not `accent`: the focused value field is the one
            // focus that means "you are about to change a value" (§3.1). Drawn
            // at the resting face's own width so the edge does not move either.
            border: Border.all(color: t.animated, width: 1),
          ),
          // The selection gestures, so a press puts the caret down and a drag
          // highlights — without this the editor took keys but a drag over the
          // text selected nothing (K-319).
          child: TextSelectionGestureDetectorBuilder(delegate: this)
              .buildGestureDetector(
            child: Padding(
              // The resting face's 6 of padding **plus its 1px edge**: a
              // `Container`'s decoration insets its child by the border it
              // draws and a `DecoratedBox` does not, so the 7 here is what
              // puts the two readings on exactly the same pixel.
              padding: const EdgeInsets.symmetric(horizontal: 7),
              child: EditableText(
                key: textFieldKey,
                controller: _controller,
                focusNode: _focus,
                // Mono while focused too — the number must not change width
                // between reading it and typing over it (§7.1) — the same
                // size as the resting number, so nothing reflows on the click.
                style: _valueStyle(t).copyWith(color: t.textPrimary),
                // The resting face is right-anchored, so the editor is too: the
                // digits stay where they were even though the reading loses its
                // sign or its unit on the way into the field.
                textAlign: TextAlign.right,
                cursorColor: t.accent,
                backgroundCursorColor: t.surface2,
                selectionColor: t.accent.withValues(alpha: 0.5),
                selectionControls: desktopTextSelectionHandleControls,
                onSubmitted: (_) => _commitText(),
              ),
            ),
          ),
        ),
      );
    }
    return FocusableActionDetector(
      focusNode: _idleFocus,
      // Enter only, no Space: this is a number box, and `Enter` opening the
      // editor is what Tab-and-type needs (K-319).
      shortcuts: const {
        SingleActivator(LogicalKeyboardKey.enter, includeRepeats: false):
            ActivateIntent(),
        SingleActivator(LogicalKeyboardKey.numpadEnter, includeRepeats: false):
            ActivateIntent(),
      },
      actions: <Type, Action<Intent>>{
        ActivateIntent: CallbackAction<ActivateIntent>(onInvoke: (_) {
          _beginEdit();
          return null;
        }),
      },
      onFocusChange: (has) {
        setState(() => _focused = has);
        // **Tab arrives ready to type** (§12A.3, K-529). The only way this box
        // takes focus is keyboard traversal — a click opens the editor
        // directly — and a value well reached by Tab is one about to be
        // retyped, so it opens its editor at once. `_beginEdit` is the call
        // that already selects the whole value, which is the half the owner
        // read as missing: the hop worked, the first keystroke appended.
        if (has && !_editing) _beginEdit();
      },
      mouseCursor: SystemMouseCursors.resizeLeftRight,
      onShowHoverHighlight: (over) => setState(() => _hover = over),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: _beginEdit,
        onSecondaryTapDown: (d) => _contextMenu(context, d.globalPosition),
        onHorizontalDragStart: (_) {
          _dragAccum = 0;
          _lastDragValue = null;
          setState(() => _dragging = true);
          _showLadder();
          widget.onChangeStart?.call();
        },
        onHorizontalDragUpdate: (d) {
          final factor = scrubFactor();
          _factor.value = factor;
          _dragAccum += d.delta.dx * widget.speed * factor;
          if (_dragAccum.abs() >= widget.speed * factor) {
            // The drag runs from its own last tick, not from `widget.value`:
            // pointer events arrive faster than rebuilds, and a base read
            // from the stale prop dropped every chunk but the frame's last —
            // a fast drag lost most of its travel.
            final next = ((_lastDragValue ?? widget.value) + _dragAccum)
                .clamp(widget.min, widget.max);
            _dragAccum = 0;
            _lastDragValue = next;
            (widget.onChangeLive ?? widget.onChanged)(next);
          }
        },
        onHorizontalDragEnd: (_) {
          final v = _lastDragValue;
          _lastDragValue = null;
          _hideLadder();
          setState(() => _dragging = false);
          if (v != null) {
            (widget.onChangeEnd ?? widget.onChanged)(v);
          } else {
            // Never crossed one speed-increment: nothing was ticked, so the
            // press was a click that wobbled a few pixels, not a scrub. It
            // cancels as a drag — and then does what the click meant, which
            // is open the editor (K-319). Before this, a click that moved
            // at all did nothing, and value boxes felt like they swallowed
            // clicks.
            widget.onDragCancel?.call();
            _beginEdit();
          }
        },
        onHorizontalDragCancel: () {
          _lastDragValue = null;
          _hideLadder();
          setState(() => _dragging = false);
          widget.onDragCancel?.call();
        },
        child: Container(
          height: widget.bare ? null : wellHeight,
          padding: widget.bare
              ? EdgeInsets.zero
              : const EdgeInsets.symmetric(horizontal: 6),
          decoration: BoxDecoration(
            // The inset stays the inset in every state: a well does not lift
            // under the pointer, because then it would stop being a recess
            // (§2.1). Hover and scrub speak through the edge instead.
            color: widget.bare ? null : widget.fill ?? t.surface0,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(
                color: widget.bare
                    ? const Color(0x00000000)
                    : _dragging
                        ? t.accent
                        // The one focus ring that is `animated` rather than
                        // `accent`: it means "you are about to change a value"
                        // (§3.1, §6.5).
                        : _focused
                            ? t.animated
                            : _hover
                                ? t.hairlineStrong
                                : t.hairline,
                width: 1),
          ),
          child: Align(
            alignment: Alignment.centerRight,
            widthFactor: 1,
            child: Text(
              _format(widget.value),
              textAlign: TextAlign.right,
              style: _valueStyle(t).copyWith(
                color: _dragging
                    ? t.accent
                    : widget.keyed
                        ? t.animated
                        // A bare number has no well to say "editable", so it
                        // rests where the drawing puts it — a bar's own
                        // secondary reading rather than the well's primary.
                        : widget.bare
                            ? (_hover || _focused
                                ? t.textPrimary
                                : t.textSecondary)
                            : t.textPrimary,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A thin themed slider. `commitOnRelease` reproduces the UI-scale rule
/// (K-117): the dragged value shows live but `onChanged` fires on release.
class HouseSlider extends StatefulWidget {
  final double value;
  final double min;
  final double max;
  final double? step;
  final int decimals;
  final String? suffix;
  final bool commitOnRelease;

  /// How wide the track is drawn. The default suits a settings row; a control
  /// in a toolbar wants less.
  final double width;

  /// Whether the number is drawn beside the track.
  ///
  /// Off for a slider whose value is already said elsewhere — the Timeline's
  /// zoom says it in a tooltip, and a readout repeating it would cost the
  /// bottom bar room it does not have.
  final bool showValue;
  final ValueChanged<double> onChanged;

  /// Called instead of [onChanged] while the handle is being **dragged**, for
  /// a control whose live value costs something the committed one does not —
  /// the Timeline's zoom applies a drag at once and only flies for a tap
  /// (K-293). Unset, a drag reports through [onChanged] as it always did.
  final ValueChanged<double>? onChangeLive;

  /// Fired once when a drag begins, before the first [onChangeLive] — for a
  /// caller that fixes something at the start of the gesture and holds it to
  /// the end (the Timeline's zoom anchors on the playhead *once* per drag,
  /// K-319). Omitted by callers with nothing to fix.
  final VoidCallback? onChangeStart;

  /// Fired once when a drag ends, after the last tick.
  final VoidCallback? onChangeEnd;

  const HouseSlider({
    super.key,
    required this.value,
    required this.min,
    required this.max,
    required this.onChanged,
    this.step,
    this.decimals = 2,
    this.suffix,
    this.commitOnRelease = false,
    this.width = 140,
    this.showValue = true,
    this.onChangeLive,
    this.onChangeStart,
    this.onChangeEnd,
  });

  @override
  State<HouseSlider> createState() => _HouseSliderState();
}

class _HouseSliderState extends State<HouseSlider> {
  double? _pending;

  double get _shown => _pending ?? widget.value;

  double _fromDx(double dx, double width) {
    var v =
        widget.min + (dx / width).clamp(0.0, 1.0) * (widget.max - widget.min);
    final s = widget.step;
    if (s != null && s > 0) v = (v / s).round() * s;
    return v.clamp(widget.min, widget.max).toDouble();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final width = widget.width;
    final frac =
        ((_shown - widget.min) / (widget.max - widget.min)).clamp(0.0, 1.0);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapUp: (d) => widget.onChanged(_fromDx(d.localPosition.dx, width)),
          onHorizontalDragStart: (_) => widget.onChangeStart?.call(),
          onHorizontalDragUpdate: (d) {
            final v = _fromDx(d.localPosition.dx, width);
            if (widget.commitOnRelease) {
              setState(() => _pending = v);
              // Held back from the *document*, not from the picture: a caller
              // with a live channel (an effect parameter's preview render)
              // still sees every tick, and only the release commits. Without
              // this the two options were exclusive, and a slider could either
              // preview or commit once, never both.
              widget.onChangeLive?.call(v);
            } else {
              (widget.onChangeLive ?? widget.onChanged)(v);
            }
          },
          onHorizontalDragEnd: (_) {
            if (_pending != null) {
              widget.onChanged(_pending!);
              setState(() => _pending = null);
            }
            widget.onChangeEnd?.call();
          },
          onHorizontalDragCancel: () => widget.onChangeEnd?.call(),
          child: SizedBox(
            width: width,
            height: 16,
            child: CustomPaint(
              painter: _SliderPainter(
                // The mockups' own track and knob: a `hairline_strong` rule
                // with a `text_secondary` handle on it. The track had been a
                // `surface0` recess, which spends a fourth grey on a groove
                // two pixels tall (§2.1), and the knob a `text_primary` dot,
                // which read brighter than the value it points at.
                track: t.hairlineStrong,
                fill: t.accent,
                knob: t.textSecondary,
                frac: frac,
              ),
            ),
          ),
        ),
        if (widget.showValue) ...[
          const SizedBox(width: 8),
          Text(
            '${_shown.toStringAsFixed(widget.decimals)}${widget.suffix ?? ''}',
            style: t.bodyPrimary,
          ),
        ],
      ],
    );
  }
}

class _SliderPainter extends CustomPainter {
  final Color track, fill, knob;
  final double frac;
  const _SliderPainter({
    required this.track,
    required this.fill,
    required this.knob,
    required this.frac,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final y = size.height / 2;
    final trackRect = RRect.fromRectAndRadius(
      Rect.fromLTWH(0, y - 2, size.width, 4),
      const Radius.circular(2),
    );
    canvas.drawRRect(trackRect, Paint()..color = track);
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromLTWH(0, y - 2, size.width * frac, 4),
        const Radius.circular(2),
      ),
      Paint()..color = fill,
    );
    canvas.drawCircle(Offset(size.width * frac, y), 5, Paint()..color = knob);
  }

  @override
  bool shouldRepaint(_SliderPainter old) =>
      old.frac != frac || old.fill != fill || old.track != track;
}

/// A tooltip that honours Settings → Interface → Show tooltips app-wide —
/// the one thing Flutter's own Tooltip cannot do.
class LumitTooltip extends StatelessWidget {
  final String message;
  final Widget child;
  const LumitTooltip({super.key, required this.message, required this.child});

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    if (!scope.showTooltips) return child;
    return _HoverTip(message: message, child: child);
  }
}

class _HoverTip extends StatefulWidget {
  final String message;
  final Widget child;
  const _HoverTip({required this.message, required this.child});

  @override
  State<_HoverTip> createState() => _HoverTipState();
}

class _HoverTipState extends State<_HoverTip> {
  OverlayEntry? _entry;

  /// The pending show, so leaving cancels it.
  ///
  /// Without this a tooltip could appear *after* the pointer had already gone:
  /// the delay ran to completion regardless, and the `onExit` that should have
  /// stopped it had come and gone while nothing was showing yet. The tip then
  /// stuck on screen with no pointer left to leave and dismiss it — hovering
  /// the control again would clear it, and moving off would bring it back,
  /// which is the loop this was stuck in.
  Timer? _pending;

  void _show(PointerEnterEvent e) {
    _pending?.cancel();
    _pending = Timer(const Duration(milliseconds: 500), _present);
  }

  void _present() {
    if (!mounted || _entry != null) return;
    final box = context.findRenderObject() as RenderBox?;
    if (box == null || !box.attached) return;
    final origin = box.localToGlobal(Offset(0, box.size.height + 4));
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    _entry = OverlayEntry(
      builder: (_) => Positioned.fill(
        child: IgnorePointer(
          // Pulled back on screen when it would hang off an edge — a control
          // near the bottom (the Viewer's transport) would otherwise tip below
          // the window entirely.
          child: CustomSingleChildLayout(
            delegate: _PopupLayout(origin),
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
              decoration: BoxDecoration(
                color: t.surface3,
                borderRadius: BorderRadius.circular(t.tokens.floatRadius),
                border: Border.all(color: t.hairline),
                boxShadow: t.floatShadow,
              ),
              child: Text(widget.message, style: t.body),
            ),
          ),
        ),
      ),
    );
    Overlay.of(context).insert(_entry!);
  }

  void _hide() {
    _pending?.cancel();
    _pending = null;
    _entry?.remove();
    _entry = null;
  }

  @override
  void dispose() {
    _hide();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => MouseRegion(
        onEnter: _show,
        onExit: (_) => _hide(),
        child: widget.child,
      );
}

/// A right-click menu holder: wraps [child] and floats [itemBuilder]'s rows
/// at the pointer on a secondary tap.
class HouseContextMenu extends StatelessWidget {
  const HouseContextMenu({this.child, this.itemBuilder, super.key});
  final Widget? child;
  final List<MenuRow> Function(void Function() close)? itemBuilder;

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      behavior: HitTestBehavior.translucent,
      onSecondaryTapDown: (d) => _contextMenu(context, d.globalPosition),
      child: child,
    );
  }

  void _contextMenu(BuildContext context, Offset globalPos) {
    showLumitPopup<void>(
      context: context,
      position: globalPos,
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              ...(itemBuilder?.call(() => close(())) ?? []),
            ],
          ),
        ),
      ),
    );
  }
}

/// The house progress bar: a fraction of accent fill on a `surface3` track.
/// One shape for the status line's export and cache meters and the update
/// download, which had each hand-rolled their own.
class HouseProgressBar extends StatelessWidget {
  final double fraction;
  final double height;
  const HouseProgressBar({super.key, required this.fraction, this.height = 4});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final radius = BorderRadius.circular(height / 2);
    return Container(
      height: height,
      decoration: BoxDecoration(color: t.surface3, borderRadius: radius),
      child: FractionallySizedBox(
        alignment: Alignment.centerLeft,
        widthFactor: fraction.clamp(0.0, 1.0),
        child: Container(
          decoration: BoxDecoration(color: t.accent, borderRadius: radius),
        ),
      ),
    );
  }
}

/// The live readout a gesture carries with it: a small `surface4` pill of 8px
/// mono, drawn beside the thing being moved and gone the moment it is let go
/// (docs/impl/timeline-interaction.md P1, §4.2/§6.2).
///
/// In plain terms: while you drag a keyframe, a tiny label rides next to the
/// pointer saying what frame and value it has reached, so you do not have to
/// look away at a readout somewhere else. It never appears at rest.
///
/// The same shape as the key block's badge — one pill, one size, wherever the
/// Timeline says a number under the hand.
class HintPill extends StatelessWidget {
  final String text;
  const HintPill({super.key, required this.text});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return IgnorePointer(
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
        decoration: BoxDecoration(
          color: t.surface4,
          borderRadius: BorderRadius.circular(2),
        ),
        child: Text(
          text,
          style: t.mono.copyWith(fontSize: 8, color: t.textPrimary),
        ),
      ),
    );
  }
}
