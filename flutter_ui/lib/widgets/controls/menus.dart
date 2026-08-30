// Menus: the rows, the floating surface they sit on with its safe-triangle
// hover intent (K-318), and the right-click holder.

import 'dart:async';

import 'package:flutter/material.dart';

import '../../icons/lumit_icon.dart';
import '../../icons/lumit_icons.dart';
import '../../l10n/strings.dart';
import '../hover_intent.dart';
import 'base.dart';
import 'popups.dart';

/// The tick column of a menu row: the set's checkmark where the row is on, and
/// an empty slot of the same width where it is not, so a menu's names line up
/// whether anything in it is ticked or nothing is.
///
/// The mark is a glyph, not the character `✓` (K-440's tick): a character is
/// drawn by whichever font has it, at whatever weight that font gives it, and
/// sat beside the set's own marks at three different weights across three
/// menus. The glyph takes the row's text colour like any other — [MenuRow]
/// puts `bodyPrimary` on its children — and `colour` is for the one caller
/// that draws a row disabled and needs the tick to go with it.
Widget menuTick(bool on, {Color? colour}) => SizedBox(
      width: 16,
      child: on
          ? LumitIcon(LumitIcons.tick,
              colour: colour, semanticLabel: l10n.menuRowTicked)
          : null,
    );

/// One row in a dropdown/menu popup.
class MenuRow extends StatefulWidget {
  final Widget child;
  final VoidCallback onPressed;
  final bool selected;

  /// **An option row leaves the menu open** (K-671), the way a checkbox row
  /// does (K-520) — for the menus whose rows change the picture in front of
  /// you: the preview resolution, the playback mode. Flipping between them is
  /// comparing them, and a menu that shut after each choice made comparing two
  /// tiers a matter of reopening the menu between every look.
  ///
  /// The row runs its command and stays; the menu then goes when the pointer
  /// leaves it, on Escape, or on a click away. Only rows that pick one of
  /// several *and* show their answer immediately: an ordinary command row
  /// still closes, because closing is what doing something should do.
  final bool option;

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
  }) : option = false;

  /// A row that picks one of several and leaves the menu up — see [option].
  const MenuRow.option({
    super.key,
    required this.child,
    required this.onPressed,
    this.selected = false,
    this.hoverId,
  }) : option = true;

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
        onTap: () {
          widget.onPressed();
          // The row stayed, so the menu now needs a way out that is not a
          // click: the pointer leaving it (K-671).
          if (widget.option) surface?._optionPicked();
        },
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

  /// How to take this menu down once an option row has been picked and the
  /// pointer has left it (K-671). Null on a surface with no option rows — and
  /// on the menu bar's own lists, whose flyouts are navigated by moving off
  /// one surface and onto another, so "the pointer left" is not a dismissal
  /// there.
  final VoidCallback? onLeaveAfterOption;

  const FloatSurface({
    super.key,
    required this.child,
    this.width,
    this.onLeaveAfterOption,
  });

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

  /// An option row was pressed and left the menu up (K-671). From here the
  /// pointer leaving the surface is what takes it down.
  bool _leaveArmed = false;
  void _optionPicked() => _leaveArmed = true;

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
    final surface = Container(
      width: widget.width,
      padding: const EdgeInsets.all(6),
      decoration: BoxDecoration(
        color: t.surface3,
        borderRadius: BorderRadius.circular(t.tokens.floatRadius),
        border: Border.all(color: t.hairline, width: 1),
        boxShadow: t.floatShadow,
      ),
      child: widget.child,
    );
    return _MenuHoverScope(
      hovered: _hovered,
      state: this,
      child: widget.onLeaveAfterOption == null
          ? surface
          : MouseRegion(
              onExit: (_) {
                if (_leaveArmed) widget.onLeaveAfterOption!();
              },
              child: surface,
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
