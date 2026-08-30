// The house text well: a single-line box with the house edge, an optional
// leading mark, and the expression autofill list.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../theme/theme.dart';
import '../autofill.dart';
import 'base.dart';
import 'buttons.dart';

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
  /// whose job is searching (§12A.1). Decorative by default: it takes no
  /// pointer, so a click on it still lands in the field behind it.
  final Widget? leading;

  /// Whether [leading] answers the pointer. A mark that only says what the
  /// field is for must not swallow a click meant for the text behind it; a
  /// leading that is itself a control — the Project panel's colour filter,
  /// which lives inside the search well — has to be clickable.
  final bool leadingInteractive;

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

  /// Many lines instead of one, filling the height the parent gives it —
  /// what a **code well** is (docs/impl/custom-shader.md CS3). Enter inserts a
  /// newline rather than submitting, and the text scrolls inside the box.
  ///
  /// The single-line well is still the default and still what every other
  /// caller gets: a well is one line unless somebody asks for a page.
  final bool multiline;

  /// The well's scroll position, owned by the caller — for a gutter that has
  /// to follow the text it numbers. Null and [EditableText] makes its own, as
  /// every single-line caller wants.
  final ScrollController? scrollController;

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
    this.multiline = false,
    this.scrollController,
    this.leading,
    this.leadingInteractive = false,
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
        widget.leadingInteractive,
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
                    // A page rather than a line: `expands` fills the height the
                    // caller gave the well, and the multiline keyboard type is
                    // what makes Enter a newline instead of a submission.
                    maxLines: widget.multiline ? null : 1,
                    expands: widget.multiline,
                    keyboardType: widget.multiline
                        ? TextInputType.multiline
                        : TextInputType.text,
                    scrollController: widget.scrollController,
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
  static Widget _withLeading(Widget? leading, bool interactive, Widget field) =>
      leading == null
          ? field
          : Row(children: [
              interactive ? leading : IgnorePointer(child: leading),
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
