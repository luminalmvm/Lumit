// The pressable controls: the house button and the three little state marks
// (checkbox, toggle, radio), which share one keyboard-activation shortcut set.

import 'dart:math' as math;

import 'package:flutter/gestures.dart' show kPrimaryButton;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../theme/motion.dart';
import '../../theme/theme.dart';
import '../../theme/style_sheet.dart';
import 'base.dart';
import 'motion.dart';

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

/// The house button, in the three faces docs/15-DESIGN.md gives it:
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
/// Hover and press lay a tint of the text colour over whichever face is set,
/// press twice as strong, since a step up the surface ramp is the ground's own
/// colour in some light schemes.
class HouseButton extends StatefulWidget {
  final Widget child;
  final VoidCallback? onPressed;
  final bool frameless;
  final bool small;
  final EdgeInsets? padding;

  /// The default action of the window it sits in — what `Enter` presses, and
  /// the one filled button the surface is allowed (§3.1).
  final bool primary;

  /// The closed face of a dropdown: `surface2` and a plain hairline rather
  /// than the secondary button's outline, so a picker reads as something to
  /// open rather than as an action to take.
  final bool dropdown;

  /// Take keyboard focus on first build — for the default button of a
  /// confirmation window, so `Enter` presses it the moment the window opens.
  /// Pair it with [primary] so what Enter will do is visible.
  final bool autofocus;

  /// The chosen one of a set — a tab, a mode chip, a segmented option.
  ///
  /// Each shape says it its own way. Lantern fills the pill with the accent
  /// and puts the label in `text_primary` on it. Desk keeps the accent tint
  /// and adds a 2px accent index on the leading edge, the mark on a switch.
  /// Studio stays the accent tint the tool bar already arms a tool with, in
  /// an accent outline. Whatever the shape, the fill stands off the button's
  /// edge by `ShapeTokens.pillInset` on every side.
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
    // The label's own style, for the one face that sets it: mono capitals,
    // at the size of every other button's label.
    TextStyle? labelStyle;
    final capitals = t.kicker.copyWith(
        fontSize: t.body.fontSize,
        letterSpacing:
            t.kicker.letterSpacing! * t.body.fontSize! / t.kicker.fontSize!);
    // Desk's 2px accent index on the leading edge of the control in force.
    var index = false;
    // Desk: a button is a flat plate with a shade line under it, and a
    // dropdown an underlined word; nothing is boxed.
    final plate = t.shape == ThemeShape.desk &&
        enabled &&
        !widget.primary &&
        !widget.active &&
        !widget.frameless;
    // The fill an accent face takes in the hand: a step past its hover.
    Color pressedAccent() => Color.alphaBlend(hoverTint(t, 2), t.accentHover);
    if (!enabled) {
      // No fill and a plain hairline, so a dead button reads lighter than a
      // live one rather than heavier.
      edge = widget.frameless ? null : t.hairline;
      // A dead primary drops the accent but keeps the shape of its word: the
      // Export button must not change case as the path field is filled in.
      if (widget.primary) labelStyle = capitals.copyWith(color: t.textDisabled);
    } else if (widget.primary) {
      // The single filled action (§3.1, §12A.4). Ahead of every other state:
      // what `Enter` presses must be findable at a glance whatever the pointer
      // is doing, so hover lifts the fill rather than replacing it.
      fill = _down
          ? pressedAccent()
          : _hover
              ? t.accentHover
              : t.accent;
      label = accentInk(t);
      labelStyle = capitals.copyWith(color: label);
    } else if (widget.active) {
      // Ahead of hover and press: which one is in force must not blink off
      // under the pointer. Hover lifts it to `accentHover` instead.
      final lifted = _hover || _down;
      switch (t.shape) {
        case ThemeShape.studio:
          fill = t.accent.withValues(alpha: lifted ? 0.24 : 0.16);
          edge = t.accent;
        case ThemeShape.desk:
          fill = t.accent.withValues(alpha: lifted ? 0.24 : 0.16);
          index = true;
        case ThemeShape.lantern:
          fill = _down
              ? pressedAccent()
              : lifted
                  ? t.accentHover
                  : t.accent;
          label = accentInk(t);
      }
    } else if (_down || _hover) {
      // Over the face's own ground where it has one, the dropdown's well and
      // Desk's plate, so hover lightens what is there.
      final ground = (widget.dropdown && !plate) || (plate && !widget.dropdown)
          ? t.surface2
          : null;
      final tint = hoverTint(t, _down ? 2 : 1);
      fill = ground == null ? tint : Color.alphaBlend(tint, ground);
      edge = t.hairlineStrong;
    } else if (plate) {
      fill = widget.dropdown ? null : t.surface2;
      edge = t.hairlineStrong;
    } else if (widget.dropdown) {
      fill = t.surface2;
      edge = t.hairline;
    } else {
      // Idle: an outline over the panel's own surface, not a raised fill —
      // a widget at rest adds no fourth grey to the panel (§2.1, §2.3).
      edge = widget.frameless ? null : t.hairlineStrong;
    }
    // The focus ring, over whatever face is drawn. In the label's colour on
    // an accent fill, where an accent ring would vanish.
    final ring = _focused && enabled
        ? (widget.primary || (widget.active && t.shape == ThemeShape.lantern)
            ? accentInk(t)
            : t.accent)
        : null;
    final pad = widget.padding ??
        (widget.small
            ? const EdgeInsets.symmetric(horizontal: 5, vertical: 2)
            : const EdgeInsets.symmetric(horizontal: 8, vertical: 3));
    // The fill of the one in force stands off the button's own edge by the
    // shape's pill inset, the same margin the whole way round, so a filled
    // segment never sits flush with the pill it is in. The inset is taken out
    // of the padding, so the label and the button's size do not move when the
    // state flips; Studio and Desk have an inset of 0 and draw as before.
    final inset = widget.active && enabled ? t.tokens.pillInset : 0.0;
    final margin = EdgeInsets.all(inset);
    final innerPad =
        (pad - margin).clamp(EdgeInsets.zero, EdgeInsetsGeometry.infinity);
    // Every face here is an action's, a stadium under Lantern: a dropdown's
    // closed face is a button, and only its list keeps the control corner.
    // An inset fill takes the outer radius minus the inset, so a stadium stays
    // a stadium and a rounded corner keeps its margin at the corner too.
    final radius = BorderRadius.circular(
        math.max(0, t.tokens.actionRadius - inset).toDouble());
    // The fill comes up quickly and leaves a little slower (docs/15 §8).
    final step = _hover || _down ? scope.motion.hoverIn : scope.motion.hoverOut;
    // A press gives a little under the finger, at Full only: Desk's
    // controls stop dead, and Minimal keeps no manner.
    final gives = scope.animationLevel == AnimationLevel.all &&
        t.shape != ThemeShape.desk;
    final dressed = partStyle(context, t, 'button', classes: {
      if (widget.primary) 'primary',
      if (widget.dropdown) 'dropdown',
    }, states: {
      if (_hover) 'hover',
      if (_down) 'pressed',
      if (widget.active) 'on',
      if (!enabled) 'disabled',
      if (_focused) 'focus',
    });
    if (dressed.colour != null) {
      label = dressed.colour;
      labelStyle = labelStyle?.copyWith(color: label);
    }
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
      // The press shows from the moment the pointer goes down. A tap's own
      // down waits out the drag recogniser in a dialogue or a list, so a
      // quick click there showed no press at all.
      child: Listener(
        onPointerDown: enabled
            ? (e) {
                if (e.buttons & kPrimaryButton != 0) {
                  setState(() => _down = true);
                }
              }
            : null,
        onPointerUp: enabled ? (_) => setState(() => _down = false) : null,
        onPointerCancel:
            enabled ? (_) => setState(() => _down = false) : null,
        child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: widget.onPressed,
        child: AnimatedScale(
        scale: gives && _down ? 0.97 : 1,
        duration: scope.motion.hoverIn.duration,
        curve: scope.motion.hoverIn.curve,
        child: Padding(
          padding: margin,
          child: frosted(dressed, radius, insetShadows(dressed, radius, AnimatedContainer(
          duration: step.duration,
          curve: step.curve,
          padding: innerPad,
          decoration: dress(BoxDecoration(
            color: fill,
            // A one-sided border cannot carry a radius, and Desk's is nought.
            borderRadius: plate ? null : radius,
            // Always a border, transparent when there is nothing to show. A
            // BoxDecoration's border insets its child, so appearing on hover
            // grew the control by 2 px each way and nudged everything beside
            // it — the whole row visibly shifting as the pointer crossed it.
            border: plate
                ? Border(
                    bottom: BorderSide(
                        color: edge ?? const Color(0x00000000), width: 1))
                : Border.all(
                    color: edge ?? const Color(0x00000000), width: 1),
          ), dressed),
          // The index and the focus ring are painted over the box, so they
          // inset nothing and the control is the same size as its neighbours.
          foregroundDecoration: ring != null
              ? BoxDecoration(
                  borderRadius: radius, border: Border.all(color: ring))
              : index
                  ? BoxDecoration(
                      border:
                          Border(left: BorderSide(color: t.accent, width: 2)))
                  : null,
          child: DefaultTextStyle(
            // A dropdown's closed face reads in the secondary text colour —
            // the mockups' own (§12A.6); bright primary at 11px reads bold.
            style: enabled
                ? (labelStyle ??
                    (label == null
                        ? (widget.dropdown ? t.body : t.bodyPrimary)
                        : t.bodyPrimary.copyWith(color: label)))
                : (labelStyle ?? t.body.copyWith(color: t.textDisabled)),
            child: _centred(_capitalised(t, widget.child, widget.primary)),
          ),
        ))),
        ),
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

/// The primary button's label cased for the shape, when it is a plain word.
///
/// Flutter has no text transform, and the case is a *style* rather than part
/// of the phrase, so the arb file keeps one sentence-case key, translated
/// once, and [LumitTheme.kickerCase] sets it on the way to the screen. A child
/// that is anything but a bare [Text] (an icon, a row) is handed back
/// untouched.
Widget _capitalised(LumitTheme t, Widget child, bool on) =>
    on && child is Text && child.data != null
        ? Text(t.kickerCase(child.data!), style: child.style)
        : child;

/// The word on an accent fill: whichever of `text_primary` and `surface_0`
/// stands out more against the accent. Dark words on the default spruce, or
/// on any dark accent, made a live button look switched off.
Color accentInk(LumitTheme t) {
  final fill = t.accent.computeLuminance() + 0.05;
  double contrast(Color c) {
    final ink = c.computeLuminance() + 0.05;
    return ink > fill ? ink / fill : fill / ink;
  }

  return contrast(t.textPrimary) >= contrast(t.surface0)
      ? t.textPrimary
      : t.surface0;
}

/// A 14 px themed checkbox.
///
/// **A null [onChanged] is disabled**, the way every other house control says
/// it (docs/15 §12A.3d): the box takes no click and no key, it is out of the
/// focus order, and it draws in `text_disabled` — a tick you can read and
/// cannot move. Call sites used to hand it an empty callback instead, which
/// made a box that looked live, took the click, and did nothing with it.
class HouseCheckbox extends StatefulWidget {
  final bool value;
  final ValueChanged<bool>? onChanged;

  /// The words beside the box, which a click on ticks it too.
  final String? label;
  const HouseCheckbox(
      {super.key, required this.value, required this.onChanged, this.label});

  @override
  State<HouseCheckbox> createState() => _HouseCheckboxState();
}

class _HouseCheckboxState extends State<HouseCheckbox> {
  bool _focused = false;
  final ControlFocusNode _focusNode = ControlFocusNode(debugLabel: 'checkbox');

  /// How many times the box has been ticked while on screen. The block pops
  /// in when somebody ticks it and not when a window full of ticked boxes
  /// opens, so a box that mounts already ticked counts nought and stays still.
  int _ticked = 0;

  @override
  void didUpdateWidget(HouseCheckbox old) {
    super.didUpdateWidget(old);
    if (widget.value && !old.value) _ticked++;
  }

  @override
  void dispose() {
    _focusNode.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final onChanged = widget.onChanged;
    // The mockups' checkbox: a 9px outlined box whose checked state is a 5px
    // block of the primary text colour — no accent, which also puts the control
    // inside §3.1's accent discipline. The 14px container stays as the hit
    // target around the smaller mark; the focus ring reads in the animated
    // token, like a focused value well. Under the pointer the edge lights, by
    // [lit] of the way, which is what says the box can be ticked.
    Widget mark(double lit) => SizedBox(
      width: 14,
      height: 14,
      child: Center(
        child: Container(
          width: 9,
          height: 9,
          decoration: BoxDecoration(
            // Half a well's corner, as the box is half a well's height: 1
            // under Studio and Desk, 3.5 under Lantern.
            borderRadius: BorderRadius.circular(t.tokens.wellRadius / 2),
            border: Border.all(
                color: onChanged == null
                    ? t.textDisabled
                    : _focused
                        ? t.animated
                        : Color.lerp(t.textMuted, t.textPrimary, lit)!,
                width: _focused && onChanged != null ? 1.5 : 1),
          ),
          child: widget.value
              ? Center(
                  child: Entrance(
                    key: ValueKey<int>(_ticked),
                    spec: _ticked == 0 ? MotionSpec.still : scope.motion.pop,
                    scale: scope.motion.popScale,
                    child: Container(
                      width: 5,
                      height: 5,
                      color:
                          onChanged == null ? t.textDisabled : t.textPrimary,
                    ),
                  ),
                )
              : null,
        ),
      ),
    );
    // The box, with its words after it inside the same hit area. One line of
    // text with the box in it, so it fits a row of any width and ends in an
    // ellipsis in a narrow one.
    Widget labelled(double lit) {
      final label = widget.label;
      if (label == null) return mark(lit);
      return Text.rich(
        TextSpan(children: [
          WidgetSpan(
            alignment: PlaceholderAlignment.middle,
            child: Padding(
                padding: const EdgeInsets.only(right: 6), child: mark(lit)),
          ),
          TextSpan(text: label),
        ]),
        maxLines: 1,
        overflow: TextOverflow.ellipsis,
        style: t.body
            .copyWith(color: onChanged == null ? t.textDisabled : t.textMuted),
      );
    }

    // Disabled: no gesture, no focus, no shortcut — just the reading.
    if (onChanged == null) return labelled(0);
    return FocusableActionDetector(
      focusNode: _focusNode,
      shortcuts: _activateShortcuts,
      actions: <Type, Action<Intent>>{
        ActivateIntent: CallbackAction<ActivateIntent>(onInvoke: (_) {
          onChanged(!widget.value);
          return null;
        }),
      },
      onFocusChange: (has) => setState(() => _focused = has),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => onChanged(!widget.value),
        child: HoverMark(builder: (context, lit) => labelled(lit)),
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
        child: HoverMark(
          builder: (context, lit) => Container(
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
            // The track only changes colour, so it eases plainly whatever the
            // knob does: a curve that lands long would ask for a colour past
            // the one it is heading to.
            duration: scope.motion.toggle.duration,
            curve: Curves.easeOut,
            width: _width,
            height: _height,
            decoration: BoxDecoration(
              // Lifted a little under the pointer, whichever way it is set.
              color: widget.value
                  ? Color.lerp(t.animated, t.textPrimary, 0.25 * lit)
                  : Color.lerp(t.hairlineStrong, t.textMuted, 0.6 * lit),
              borderRadius: BorderRadius.circular(_height / 2),
            ),
            // The knob slides as paint alone, so nothing is laid out again
            // while it crosses.
            alignment: Alignment.centerLeft,
            padding: const EdgeInsets.only(left: _inset),
            child: AnimatedSlide(
              duration: scope.motion.toggle.duration,
              curve: scope.motion.toggle.curve,
              offset: Offset(
                  widget.value ? (_width - _knob - 2 * _inset) / _knob : 0, 0),
              child: Container(
                width: _knob,
                height: _knob,
                decoration: BoxDecoration(
                  color: t.surface0,
                  borderRadius: BorderRadius.circular(_knob / 2),
                ),
              ),
            ),
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
///
/// Drawn as the checkbox is: chosen is a dot in the primary text colour, not
/// the accent, and focus is the `animated` edge.
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

  /// How many times this one has been chosen while on screen, for the same
  /// reason [HouseCheckbox] counts its ticks.
  int _chosen = 0;

  @override
  void didUpdateWidget(HouseRadio old) {
    super.didUpdateWidget(old);
    if (widget.selected && !old.selected) _chosen++;
  }

  @override
  void dispose() {
    _focusNode.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    Color ring(double lit) => !widget.enabled
        ? t.textDisabled
        : _focused
            ? t.animated
            : Color.lerp(t.textMuted, t.textPrimary, lit)!;

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
        child: HoverMark(
          cursor: widget.enabled ? SystemMouseCursors.click : MouseCursor.defer,
          builder: (context, lit) => Container(
          width: 14,
          height: 14,
          decoration: BoxDecoration(
            color: t.surface3,
            shape: BoxShape.circle,
            border: Border.all(color: ring(lit), width: 1.5),
          ),
          alignment: Alignment.center,
          child: widget.selected
              ? Entrance(
                  key: ValueKey<int>(_chosen),
                  spec: _chosen == 0 ? MotionSpec.still : scope.motion.pop,
                  scale: scope.motion.popScale,
                  child: Container(
                    width: 6,
                    height: 6,
                    decoration: BoxDecoration(
                      color: widget.enabled ? t.textPrimary : t.textDisabled,
                      shape: BoxShape.circle,
                    ),
                  ),
                )
              : null,
          ),
        ),
      ),
    );
  }
}
