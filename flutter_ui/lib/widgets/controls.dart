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
import 'package:lumit_flutter/widgets/hover_intent.dart';

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

/// A borderless hover-reactive button: idle `surface3` fill (or nothing when
/// `frameless`), hover `surface4` + strong hairline, press strong fill +
/// accent edge — the egui widget-state table.
class HouseButton extends StatefulWidget {
  final Widget child;
  final VoidCallback? onPressed;
  final bool frameless;
  final bool small;
  final EdgeInsets? padding;

  /// The default action of the window it sits in — what `Enter` presses
  /// (K-243). Drawn with the accent edge it would otherwise only get under the
  /// pointer, which is what docs/15 §2 keeps the one accent for.
  final bool primary;

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
    if (!enabled) {
      fill = widget.frameless ? null : t.surface2;
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
      edge = t.accent;
    } else if (_hover) {
      fill = t.surface4;
      edge = t.hairlineStrong;
    } else {
      fill = widget.frameless ? null : t.surface3;
      if (widget.primary || _focused) edge = t.accent;
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
            style: enabled
                ? (label == null
                    ? t.bodyPrimary
                    : t.bodyPrimary.copyWith(color: label))
                : t.body.copyWith(color: t.textDisabled),
            child: widget.child,
          ),
        ),
      ),
    );
  }
}

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
Widget _dropdownFace(LumitTheme t, String label, {Widget? face}) => Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        face ?? Flexible(child: Text(label, overflow: TextOverflow.ellipsis)),
        const SizedBox(width: 4),
        CustomPaint(
          size: const Size(9, 9),
          painter: _CaretPainter(t.textSecondary),
        ),
      ],
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
  /// [_dropdownFace].
  final Widget? face;

  const BareDropdown({
    super.key,
    required this.value,
    required this.options,
    required this.label,
    required this.onChanged,
    this.group,
    this.face,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
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
      onPressed: onChanged == null ? null : () => _open(context, t),
      child: _dropdownFace(t, label(value), face: face),
    );
  }

  Future<void> _open(BuildContext context, LumitTheme t) async {
    final box = context.findRenderObject()! as RenderBox;
    final origin = box.localToGlobal(Offset.zero);
    final picked = await showLumitPopup<T>(
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
                    (i == 0 ||
                        group!(options[i - 1]) != group!(options[i])))
                  Padding(
                    padding:
                        EdgeInsets.fromLTRB(10, i == 0 ? 6 : 10, 10, 2),
                    child: Text(
                      group!(options[i])!,
                      style: t.small.copyWith(color: t.textMuted),
                    ),
                  ),
                MenuRow(
                  selected: options[i] == value,
                  onPressed: () => close(options[i]),
                  child: Text(label(options[i])),
                ),
              ],
            ],
          ),
        ),
      ),
    );
    if (picked != null) onChanged!(picked);
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
      child: _dropdownFace(t, label),
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

  const BareLazyDropdown({
    super.key,
    required this.label,
    required this.options,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
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
      child: _dropdownFace(t, label),
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
/// **Escape is Flutter's own `DismissIntent`, not another key handler** (K-319).
/// `WidgetsApp` already binds Escape to it above everything, so the window only
/// has to say what dismissing *means* — an `Actions` entry that closes with
/// null, the same answer a click on the scrim gives. The comment that used to
/// sit here claimed Escape worked "via the route": it did not, because this is
/// an `OverlayEntry` and not a route, so nothing was listening and Escape did
/// nothing in every dialogue in the application.
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
    builder: (_) => Actions(
      // Escape. `WidgetsApp` binds it to DismissIntent above the whole tree,
      // so this only has to say what dismissing means here.
      actions: <Type, Action<Intent>>{
        DismissIntent: CallbackAction<DismissIntent>(
          onInvoke: (_) {
            close(null);
            return null;
          },
        ),
      },
      child: Stack(
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
            child: builder(close),
          ),
        ],
      ),
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

  const _MovableWindow({
    required this.id,
    required this.initialSize,
    required this.minSize,
    required this.child,
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
    super.dispose();
  }

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

  const HouseTextField({
    super.key,
    required this.controller,
    this.width = 200,
    this.onSubmitted,
    this.submitOnLostFocus = false,
    this.onTapOutside,
    this.onCancelled,
    this.autofill,
    this.autofocus = false,
    this.focusNode,
    this.style,
    this.hint,
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

    return Container(
      width: widget.width,
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 3),
      decoration: BoxDecoration(
        color: t.surface0,
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        border: Border.all(color: t.hairline),
      ),
      child: Stack(
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
    );
  }

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

Future<T?> showLumitPopup<T>({
  required BuildContext context,
  required Offset position,
  required Widget Function(void Function(T?) close) builder,
  // Whether what is underneath still feels the pointer while this popup is up.
  // Menus want it — hovering another heading or another row is how a menu is
  // navigated — and nothing else does: a dropdown that let the panel behind it
  // light up under the pointer would be answering to a click it will not get.
  bool hoverThrough = false,
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
              onTap: () => close(null),
              onSecondaryTap: () => close(null),
            ),
          ),
          Positioned.fill(
            child: CustomSingleChildLayout(
              delegate: _PopupLayout(position),
              // Scrolls only when it has to: a shorter popup shrink-wraps and
              // behaves exactly as before.
              child: SingleChildScrollView(child: builder(close)),
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
        child: Container(
          width: 14,
          height: 14,
          decoration: BoxDecoration(
            color: widget.value ? t.accent : t.surface3,
            borderRadius: BorderRadius.circular(3),
            border: Border.all(
                color: _focused
                    ? t.accent
                    : (widget.value ? t.accent : t.hairlineStrong),
                width: _focused ? 1.5 : 1),
          ),
          child: widget.value
              ? CustomPaint(painter: _TickPainter(t.surface0))
              : null,
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

class _TickPainter extends CustomPainter {
  final Color color;
  const _TickPainter(this.color);
  @override
  void paint(Canvas canvas, Size size) {
    final p = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.6
      ..strokeCap = StrokeCap.round;
    final path = Path()
      ..moveTo(size.width * 0.22, size.height * 0.52)
      ..lineTo(size.width * 0.44, size.height * 0.74)
      ..lineTo(size.width * 0.8, size.height * 0.28);
    canvas.drawPath(path, p);
  }

  @override
  bool shouldRepaint(_TickPainter old) => old.color != color;
}

/// How much a scrub tick is worth right now, from the modifier keys — the
/// After Effects convention: Shift makes a drag coarse (×10), Ctrl makes it
/// fine (×0.1), and nothing held is ×1. Sampled inside the drag handler on
/// every update, so pressing or releasing a modifier mid-drag takes effect at
/// once.
double scrubFactor() => HardwareKeyboard.instance.isShiftPressed
    ? 10
    : HardwareKeyboard.instance.isControlPressed
        ? 0.1
        : 1;

/// egui's DragValue: drag horizontally to adjust, click to type, right-click
/// for Reset / Copy / Paste (egui's built-in drag-value menu). [resetTo] is the
/// field's known default — Reset appears only when a call site supplies one.
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

  /// The resting background. Defaults to `surface3`, which reads as a field on
  /// a panel — but a dialogue's own surface *is* surface3, so a field there has
  /// to be darker to look like something you can type into. Only the resting
  /// colour: hover stays the standard lift, so the affordance is unchanged.
  final Color? fill;
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
    this.fill,
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

class _DragValueFieldState extends State<DragValueField>
    implements TextSelectionGestureDetectorBuilderDelegate {
  bool _editing = false;
  bool _hover = false;
  bool _focused = false;
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

  String _format(num v) {
    var s = _plain(v);
    // `toStringAsFixed` already carries a minus; only the plus has to be put
    // back, and only where the reading is signed.
    if (widget.signed && !s.startsWith('-')) s = '+$s';
    return widget.suffix == null ? s : '$s${widget.suffix}';
  }

  void _commitText() {
    final raw = _controller.text.replaceAll(widget.suffix ?? '', '').trim();
    final parsed = num.tryParse(raw);
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
                  final parsed = raw == null ? null : num.tryParse(raw);
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
      return SizedBox(
        width: 72,
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: t.surface0,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(color: t.accent),
          ),
          // The selection gestures, so a press puts the caret down and a drag
          // highlights — without this the editor took keys but a drag over the
          // text selected nothing (K-319).
          child: TextSelectionGestureDetectorBuilder(delegate: this)
              .buildGestureDetector(
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
              child: EditableText(
                key: textFieldKey,
                controller: _controller,
                focusNode: _focus,
                style: t.bodyPrimary,
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
      onFocusChange: (has) => setState(() => _focused = has),
      mouseCursor: SystemMouseCursors.resizeLeftRight,
      onShowHoverHighlight: (over) => setState(() => _hover = over),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: _beginEdit,
        onSecondaryTapDown: (d) => _contextMenu(context, d.globalPosition),
        onHorizontalDragStart: (_) {
          _dragAccum = 0;
          _lastDragValue = null;
          widget.onChangeStart?.call();
        },
        onHorizontalDragUpdate: (d) {
          final factor = scrubFactor();
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
          widget.onDragCancel?.call();
        },
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
          decoration: BoxDecoration(
            color: _hover ? t.surface4 : (widget.fill ?? t.surface3),
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            // Reserved even when not hovered — see HouseButton above. The
            // accent edge while keyboard-focused is the focus ring (§6.5).
            border: Border.all(
                color: _focused
                    ? t.accent
                    : (_hover ? t.hairlineStrong : const Color(0x00000000)),
                width: 1),
          ),
          child: Text(_format(widget.value), style: t.bodyPrimary),
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
                track: t.surface0,
                fill: t.accent,
                knob: t.textPrimary,
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
