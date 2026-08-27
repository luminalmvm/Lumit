// The pressable controls: the house button and the three little state marks
// (checkbox, toggle, radio), which share one keyboard-activation shortcut set.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../theme/theme.dart';
import 'base.dart';

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
    final onChanged = widget.onChanged;
    // The mockups' checkbox (K-450): a 9px outlined box whose checked state is
    // a 5px block of the primary text colour — no accent, which also puts the
    // control inside §3.1's accent discipline. The 14px container stays as the
    // hit target around the smaller mark; the focus ring reads in the animated
    // token, like a focused value well.
    final mark = SizedBox(
      width: 14,
      height: 14,
      child: Center(
        child: Container(
          width: 9,
          height: 9,
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(1),
            border: Border.all(
                color: onChanged == null
                    ? t.textDisabled
                    : _focused
                        ? t.animated
                        : t.textMuted,
                width: _focused && onChanged != null ? 1.5 : 1),
          ),
          child: widget.value
              ? Center(
                  child: Container(
                    width: 5,
                    height: 5,
                    color: onChanged == null ? t.textDisabled : t.textPrimary,
                  ),
                )
              : null,
        ),
      ),
    );
    // Disabled: no gesture, no focus, no shortcut — just the reading.
    if (onChanged == null) return mark;
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
        child: mark,
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
