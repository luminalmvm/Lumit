// The foundations every house control stands on: which pointer devices count
// as a drag, the focus node they all hold, and the theme scope they all read.

import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/material.dart';

import '../../theme/theme.dart';

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
