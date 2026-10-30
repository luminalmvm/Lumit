// Menus: the rows, the floating surface they sit on with its safe-triangle
// hover intent, and the right-click holder.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../icons/lumit_icon.dart';
import '../../icons/lumit_icons.dart';
import '../../l10n/strings.dart';
import '../../theme/style_sheet.dart';
import '../hover_intent.dart';
import 'base.dart';
import 'motion.dart';
import 'popups.dart';

/// The tick column of a menu row: the set's checkmark where the row is on, and
/// an empty slot of the same width where it is not, so a menu's names line up
/// whether anything in it is ticked or nothing is.
///
/// The mark is a glyph, not the character `✓`: a character is drawn by
/// whichever font has it, at whatever weight that font gives it, and sat beside
/// the set's own marks at three different weights across three menus. The glyph
/// takes the row's text colour like any other — [MenuRow] puts `bodyPrimary` on
/// its children — and `colour` is for the one caller that draws a row disabled
/// and needs the tick to go with it.
Widget menuTick(bool on, {Color? colour}) => SizedBox(
      width: 16,
      child: on
          ? LumitIcon(LumitIcons.tick,
              colour: colour, semanticLabel: l10n.menuRowTicked)
          : null,
    );

/// How far a menu's rows stand in from its edge. A row's corner is the
/// surface's corner less this, so the two curves run parallel.
const double menuInset = 6;

/// One row in a dropdown/menu popup.
class MenuRow extends StatefulWidget {
  final Widget child;
  final VoidCallback onPressed;
  final bool selected;

  /// Whether the row can be picked. One that cannot stays in the menu in the
  /// disabled colour, takes no highlight and no click, and the arrow keys
  /// pass over it.
  final bool enabled;

  /// **An option row leaves the menu open**, the way a checkbox row does — for
  /// the menus whose rows change the picture in front of you: the preview
  /// resolution, the playback mode. Flipping between them is comparing them,
  /// and a menu that shut after each choice made comparing two tiers a matter
  /// of reopening the menu between every look.
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

  /// Called when the pointer lands on the row, so a list can show what a row
  /// would do before it's picked, like the colour scheme list does.
  final VoidCallback? onEnter;

  const MenuRow({
    super.key,
    required this.child,
    required this.onPressed,
    this.selected = false,
    this.enabled = true,
    this.hoverId,
    this.onEnter,
  }) : option = false;

  /// A row that picks one of several and leaves the menu up — see [option].
  const MenuRow.option({
    super.key,
    required this.child,
    required this.onPressed,
    this.selected = false,
    this.enabled = true,
    this.hoverId,
    this.onEnter,
  }) : option = true;

  @override
  State<MenuRow> createState() => _MenuRowState();
}

class _MenuRowState extends State<MenuRow> {
  _FloatSurfaceState? _surface;

  /// The lit row, the surface's when the row is on one: the pointer and the
  /// arrow keys move one highlight between them.
  ValueNotifier<Object?>? _alone;
  ValueNotifier<Object?> get _lights =>
      _surface?._lit ?? (_alone ??= ValueNotifier<Object?>(null));

  Object get _id => widget.hoverId ?? this;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final surface = FloatSurface._of(context);
    if (surface == _surface) return;
    _surface?._rows.remove(this);
    _surface = surface?.._rows.add(this);
  }

  @override
  void dispose() {
    _surface?._rows.remove(this);
    _alone?.dispose();
    super.dispose();
  }

  /// Where the row is on screen, top to bottom, or null while it is not
  /// laid out. The arrow keys walk the rows in this order.
  double? get _top {
    final box = context.findRenderObject();
    return box is RenderBox && box.attached && box.hasSize
        ? box.localToGlobal(Offset.zero).dy
        : null;
  }

  void _press() {
    widget.onPressed();
    // The row stayed, so the menu now needs a way out that is not a click:
    // the pointer leaving it.
    if (widget.option) _surface?._optionPicked();
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final surface = _surface;
    final enabled = widget.enabled;
    return MouseRegion(
      onEnter: (e) {
        _lights.value = enabled ? _id : null;
        // Tell the surface which row the pointer is on, so a submenu that is
        // out can take itself back when the pointer moves to another row. The
        // surface may *hold* the report briefly while the pointer is inside an
        // open flyout's safe triangle — the highlight above is immediate either
        // way; only the flyout switch waits.
        surface?._hoverRow(_id, e.position);
        if (enabled) widget.onEnter?.call();
      },
      onHover: (e) => surface?._hoverMoved(_id, e.position),
      onExit: (_) {
        if (_lights.value == _id) _lights.value = null;
      },
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: enabled ? _press : null,
        child: ValueListenableBuilder<Object?>(
          valueListenable: _lights,
          builder: (context, lit, _) {
            final on = enabled && lit == _id;
            // Over the selected row's own light fill rather than instead of
            // it, so the chosen row still reads as chosen under the pointer.
            // Clear rather than absent at rest, so the highlight has a colour
            // to fade back to.
            final tint = hoverTint(t, on ? 1 : 0);
            final fill = widget.selected
                ? Color.alphaBlend(tint, t.accent.withValues(alpha: 0.16))
                : tint;
            // The highlight arrives at once and trails off behind the pointer
            // (docs/15 §8): a menu is read by where the highlight is, so it
            // may never lag, and the fade is only ever on the row just left.
            return AnimatedContainer(
              // Lit by the keyboard counts as lit: the command palette's
              // highlight moves at once too, and only what it left fades.
              duration: on || widget.selected
                  ? Duration.zero
                  : scope.motion.rowTrail.duration,
              curve: scope.motion.rowTrail.curve,
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
              decoration: dress(
                BoxDecoration(
                  color: fill,
                  borderRadius: BorderRadius.circular(
                      math.max(0, t.tokens.floatRadius - menuInset)),
                ),
                partStyle(context, t, 'row', states: {
                  if (on) 'hover',
                  if (widget.selected) 'on',
                  if (!enabled) 'disabled',
                }),
              ),
              child: DefaultTextStyle(
                style: enabled
                    ? t.bodyPrimary
                    : t.bodyPrimary.copyWith(color: t.textDisabled),
                child: widget.child,
              ),
            );
          },
        ),
      ),
    );
  }
}

/// Whether an open menu is answering the key being pressed. The arrows and
/// Enter belong to the menu while one is up, so a panel behind it does not
/// act on them as well.
bool get menuTakesKey =>
    _FloatSurfaceState._up.any((s) => s._rows.isNotEmpty) &&
    HardwareKeyboard.instance.logicalKeysPressed.any(_menuKeys.contains);

final Set<LogicalKeyboardKey> _menuKeys = {
  LogicalKeyboardKey.arrowUp,
  LogicalKeyboardKey.arrowDown,
  LogicalKeyboardKey.arrowLeft,
  LogicalKeyboardKey.arrowRight,
  LogicalKeyboardKey.enter,
  LogicalKeyboardKey.numpadEnter,
};

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
  /// pointer has left it. Null on a surface with no option rows — and on the
  /// menu bar's own lists, whose flyouts are navigated by moving off one
  /// surface and onto another, so "the pointer left" is not a dismissal there.
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

  /// The row of the nearest floating surface that is lit, by the pointer or
  /// the arrow keys, named by its [MenuRow.hoverId]. Null outside a surface.
  static ValueNotifier<Object?>? litRow(BuildContext context) =>
      _of(context)?._lit;

  /// The surface state itself — the rows and [SubmenuRow] talk to it for the
  /// hover-intent gating.
  static _FloatSurfaceState? _of(BuildContext context) =>
      context.getInheritedWidgetOfExactType<_MenuHoverScope>()?.state;

  @override
  State<FloatSurface> createState() => _FloatSurfaceState();
}

class _FloatSurfaceState extends State<FloatSurface> {
  final _hovered = ValueNotifier<Object?>(null);

  /// The row that is lit. Unlike [_hovered] it is never held back, and the
  /// pointer leaving a row puts it out.
  final _lit = ValueNotifier<Object?>(null);

  // --- The keys ------------------------------------------------------------
  //
  // A menu holds no focus (see popups.dart), so Up, Down, Enter, Right and
  // Left are heard on the hardware keyboard, and only by the innermost menu
  // that is up. They move the same highlight the pointer does.

  /// The rows on this surface, for the arrow keys to walk.
  final Set<_MenuRowState> _rows = {};

  /// The surfaces up in a popup, innermost last. A surface drawn inside a
  /// page rather than a popup is not one of them.
  static final List<_FloatSurfaceState> _up = [];
  bool _listed = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_listed || !inLumitPopup(context)) return;
    _listed = true;
    _up.add(this);
    // Removed and added, as the escape ladder does: the binding empties its
    // handlers between widget tests.
    HardwareKeyboard.instance
      ..removeHandler(_onKey)
      ..addHandler(_onKey);
    FocusManager.instance
      ..removeEarlyKeyEventHandler(_keepFromFocus)
      ..addEarlyKeyEventHandler(_keepFromFocus);
  }

  /// The last key a menu took. The focused widget hears every key the
  /// hardware keyboard does, so this one is kept from it, or Down in a
  /// search box would move its cursor as well as the highlight.
  static KeyEvent? _taken;

  static bool _onKey(KeyEvent event) {
    final took = event is! KeyUpEvent &&
        _up.isNotEmpty &&
        _up.last._key(event.logicalKey);
    _taken = took ? event : null;
    return took;
  }

  static KeyEventResult _keepFromFocus(KeyEvent event) =>
      identical(event, _taken) ? KeyEventResult.handled : KeyEventResult.ignored;

  /// Whether a text box has the keys. Up and Down still move the highlight,
  /// as in the search picker, and the rest are the box's.
  static bool _typing() {
    final focused = FocusManager.instance.primaryFocus?.context;
    return focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null);
  }

  bool _key(LogicalKeyboardKey key) {
    final rows = [
      for (final r in _rows)
        if (r.widget.enabled && r._top != null) r
    ]..sort((a, b) => a._top!.compareTo(b._top!));
    if (rows.isEmpty) return false;
    final at = rows.indexWhere((r) => r._id == _lit.value);
    if (key == LogicalKeyboardKey.arrowDown ||
        key == LogicalKeyboardKey.arrowUp) {
      final down = key == LogicalKeyboardKey.arrowDown;
      final next = rows[at < 0
          ? (down ? 0 : rows.length - 1)
          : (at + (down ? 1 : -1)) % rows.length];
      _lit.value = next._id;
      Scrollable.ensureVisible(
        next.context,
        alignmentPolicy: down
            ? ScrollPositionAlignmentPolicy.keepVisibleAtEnd
            : ScrollPositionAlignmentPolicy.keepVisibleAtStart,
      );
      return true;
    }
    if (_typing()) return false;
    if (key == LogicalKeyboardKey.arrowLeft) return closeLumitFlyout(context);
    if (at < 0) return false;
    final row = rows[at];
    final sub = row.widget.hoverId;
    if (sub is _SubmenuRowState &&
        (key == LogicalKeyboardKey.arrowRight ||
            key == LogicalKeyboardKey.enter ||
            key == LogicalKeyboardKey.numpadEnter)) {
      sub._open();
      // Its first row lit once it is up, so Down is not needed to get in.
      WidgetsBinding.instance.addPostFrameCallback((_) {
        final inner = _up.lastOrNull;
        if (inner != null && inner != this && inner._lit.value == null) {
          inner._key(LogicalKeyboardKey.arrowDown);
        }
      });
      return true;
    }
    if (key == LogicalKeyboardKey.enter ||
        key == LogicalKeyboardKey.numpadEnter) {
      row._press();
      return true;
    }
    return false;
  }

  // --- Safe-triangle hover intent ---------------------------------------
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

  /// An option row was pressed and left the menu up. From here the pointer
  /// leaving the surface is what takes it down.
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
    _lit.dispose();
    if (_listed) {
      _up.remove(this);
      if (_up.isEmpty) {
        HardwareKeyboard.instance.removeHandler(_onKey);
        FocusManager.instance.removeEarlyKeyEventHandler(_keepFromFocus);
      }
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final dressed = partStyle(context, t, 'menu');
    final surface = frosted(dressed, BorderRadius.circular(dressed.radius ?? t.tokens.floatRadius), Container(
      width: widget.width,
      padding: const EdgeInsets.all(menuInset),
      decoration: dress(
        BoxDecoration(
          color: t.surface3,
          borderRadius: BorderRadius.circular(t.tokens.floatRadius),
          border: Border.all(color: t.hairline, width: 1),
          boxShadow: t.floatShadow,
        ),
        dressed,
      ),
      child: Entrance.content(
        spec: scope.motion.content,
        rise: scope.motion.contentRise,
        blur: scope.motion.contentBlur,
        child: StylePart(name: 'menu', child: widget.child),
      ),
    ));
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

/// A menu row that opens a submenu beside it.
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
  /// surface's safe triangle points at.
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
          LumitIcon(LumitIcons.expand, colour: t.textMuted),
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
