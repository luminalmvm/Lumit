// The popup chain (K-519): one authority over every menu, dropdown, picker and
// flyout on screen, the layout that keeps them inside the window, and the
// tooltip that is placed by the same delegate.

import 'dart:async';

import 'package:flutter/gestures.dart' show PointerEnterEvent;
import 'package:flutter/material.dart';

import '../escape_ladder.dart';
import 'base.dart';

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
