// The shape of one section in the Effect controls panel: a heading that twirls,
// and property rows under it laid out in two columns.
//
// **In plain terms.** The panel used to draw each effect as a separate bordered
// box, which made the stack read as a pile of unrelated cards. It is really one
// list — the same list the Timeline twirls open under a layer — so it is drawn
// as one now: a heading bar per section, a hairline under every row, names down
// the left and their controls down the right. The columns are not divided by
// anything visible; they line up because every row starts them at the same x
// (K-443): the stopwatch, then a keyframe-navigation slot that is reserved even
// when the property is not animated, then the label, then the control.
//
// **The heading row.** Left column: the section's own enable switch where it
// has one, the twirl, the name as a kicker. Right column, aligned with the
// values below it: the section's actions — Reset for an effect, and what the
// effect cost in the last measured frame. Hard against the right edge: the
// close mark, kept apart from the actions because removing is not an adjustment.
//
// **Round mode keeps its bubble** (K-092). Sharp draws the section edge to edge
// with hairlines, which is the After Effects reading; round wraps the same rows
// in the floating-card chrome, so the two shapes differ in chrome and not in
// layout.

import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/dashed_outline.dart';

/// **The fixed column edges** (K-443, docs/15 §12A.3). Every row in the panel
/// lays out on the same four x positions: the stopwatch, then a keyframe-
/// navigation slot that stays empty until the property is animated, then the
/// label, then the control. The label therefore never moves when a stopwatch is
/// switched on — which is exactly what it used to do, because the navigator
/// appeared *inside* the label's own space and shoved it right.
///
/// The widths come from what the controls need rather than from a mockup's
/// pixels: a glyph is 16px on its 16-unit grid (§5) and a [HouseButton] insets
/// its child by the 1px edge it always reserves, so one icon button measures
/// 18 — the stopwatch column, and 54 for the three the navigator holds.
const double fxStopwatchColumn = 18;

/// The reserved keyframe-navigation slot: previous key, add-or-remove key, next
/// key — three 16px glyphs in their 18px buttons, drawn only while the property
/// is animated and holding their ground when it is not.
const double fxKeyNavColumn = 54;

/// The stopwatch and navigation columns together — what every row reserves
/// before its label, animated or not.
const double fxKeyColumnWidth = fxStopwatchColumn + fxKeyNavColumn;

/// How wide the label column is. Long parameter names ellipsise rather than
/// pushing the control column, which is the point of it being fixed.
const double fxLabelColumnWidth = 88;

/// How wide the whole name side is — the keyframe columns, the gap, and the
/// label — so the controls stack into one column down the panel.
const double fxNameColumnWidth = fxKeyColumnWidth + 4 + fxLabelColumnWidth;

/// The width the **Timeline's** fold-out leaves for a stopwatch on a row that
/// has none. Its lanes are not the Effect controls panel's columns — they
/// answer to the render-switch column group — so it keeps the single narrow
/// gutter it always had.
const double fxKeyframeGutter = 18;

/// How tall one property row's content is — **every** row in the Effect
/// controls panel, whatever control it carries.
///
/// The panel used to let each row be as tall as its own tallest control, so a
/// tick-box row (14 px checkbox) came out four to eight pixels shorter than a
/// number row (22 px stopwatch strip) or a choice row (a dropdown's button
/// face), and a stack of parameters visibly stepped in and out. One fixed
/// height for the content box settles it, and the height is the tick-box row's
/// — the shortest of them, and the one the owner asked everything to match.
///
/// It is the *content* box: the section adds its own 2 px above and below and
/// the hairline under the row, so a row occupies 23 px on the panel.
///
/// Controls taller than this than sit inside it rather than pushing it out —
/// their padding is squeezed by the constraint, never their text. The one
/// control that had to give ground is the stopwatch button, whose 16 px icon
/// plus 2 px of padding would have spilled: it carries none now (see
/// `keyframe_controls_frb.dart`).
///
/// Not shared with the Timeline: its lanes have their own heights, and its
/// fold-out rows take the other branch of these row widgets entirely.
const double fxRowHeight = 18;

/// One twirl-open section: Source, Transform, or one effect.
class FxSection extends StatelessWidget {
  /// The section's own control, left of the name — an effect's enable switch.
  final Widget? leading;
  final String title;
  final bool open;
  final VoidCallback onToggle;

  /// Actions in the value column, aligned with the controls below — Reset.
  final List<Widget> actions;

  /// Hard right — the close mark.
  final Widget? trailing;

  /// A right-click on the heading, with the pointer's global position — where
  /// the actions that are not worth a permanent button live (an effect's
  /// reordering, K-276). Null leaves the secondary click unclaimed.
  final void Function(Offset at)? onContextMenu;

  /// A click on the heading's name **picks this section** (K-300) — an effect
  /// is a thing that can be selected, copied and cut, and the click that says
  /// which one is the one on its name. Null (Source, Transform: sections that
  /// are not one of several) leaves the name doing what the twirl does, which
  /// is what the whole heading did before.
  final VoidCallback? onSelect;

  /// Drawn picked: the heading takes the selection fill, as a Timeline row
  /// does, so one effect chosen in either place reads the same in both.
  final bool selected;

  /// The twirl mark's own key — it is the only thing that folds a selectable
  /// section, so it is worth being able to point at.
  final Key? twirlKey;

  /// This section's place in its list, when the heading may be **dragged** to
  /// another place in it (docs/07 §6's drag-to-reorder). Null — Source,
  /// Transform, anything that does not sit in a reorderable stack — leaves the
  /// heading undraggable and accepting nothing.
  final int? dragIndex;

  /// A heading dropped on this one: the place it came from. Called only when
  /// [dragIndex] is set and the two differ.
  final void Function(int from)? onDropped;

  /// The rows under the heading, drawn only while [open].
  final List<Widget> rows;

  /// While true the heading's name is an inline editor instead of a label —
  /// how an effect is renamed (`Enter` on the selected effect, K-321).
  /// Sections that cannot be renamed (Source, Transform) never set it.
  final bool renaming;

  /// The rename's commit: the typed name, empty to clear back to the
  /// effect's own label. Called on Enter and on clicking away, the same
  /// contract every inline rename in the application has (K-243).
  final ValueChanged<String>? onRenamed;

  /// `Escape` while renaming: close the editor and keep the old name (K-323).
  final VoidCallback? onRenameCancelled;

  /// False while the effect is **bypassed**. The heading takes a dashed outline
  /// (docs/15 §5) and the rows below stop answering the pointer — but nothing
  /// fades, because the reason to look at a bypassed effect is to read what it
  /// was set to. Sections that cannot be switched off leave it true.
  final bool enabled;

  const FxSection({
    super.key,
    required this.title,
    required this.open,
    required this.onToggle,
    required this.rows,
    this.leading,
    this.actions = const [],
    this.trailing,
    this.onContextMenu,
    this.onSelect,
    this.selected = false,
    this.twirlKey,
    this.dragIndex,
    this.onDropped,
    this.renaming = false,
    this.onRenamed,
    this.onRenameCancelled,
    this.enabled = true,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final column = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        // Bypassed: the heading wears a dashed outline instead of the stack
        // being dimmed (docs/15 §5).
        enabled
            ? _draggableHeading(t)
            : DashedOutline(child: _draggableHeading(t)),
        if (open)
          for (final row in rows)
            Container(
              decoration: BoxDecoration(
                  border: Border(bottom: BorderSide(color: t.hairline))),
              padding: const EdgeInsets.fromLTRB(8, 2, 6, 2),
              child: row,
            ),
      ],
    );

    // Round mode keeps the bubble the sharp shape does without; the rows inside
    // are identical either way.
    if (t.tokens.cardRadius <= 0) return column;
    return Container(
      margin: const EdgeInsets.fromLTRB(6, 4, 6, 4),
      decoration: BoxDecoration(
        color: t.surface1,
        borderRadius: BorderRadius.circular(t.tokens.cardRadius),
        border: Border.all(color: t.hairline),
      ),
      clipBehavior: Clip.antiAlias,
      child: column,
    );
  }

  /// The heading, wrapped in the drag-and-drop that reorders the stack when
  /// this section has a place in one. Dragging the *name* is how a stack is
  /// reordered everywhere else in the application (layers in the Timeline,
  /// items in the Project panel), so an effect stack reorders the same way; the
  /// heading also stays a drop target, and the one under the pointer lights up
  /// so it is clear which place is being taken.
  Widget _draggableHeading(LumitTheme t) {
    final index = dragIndex;
    if (index == null || onDropped == null) return _heading(t);
    return DragTarget<int>(
      onWillAcceptWithDetails: (d) => d.data != index,
      onAcceptWithDetails: (d) => onDropped!(d.data),
      builder: (context, candidate, _) => Draggable<int>(
        data: index,
        // The pointer carries the effect's name and nothing else: a full-width
        // card under the cursor hides the stack it is being placed into.
        feedback: Container(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
          decoration: BoxDecoration(
            color: t.surface2,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(color: t.accent),
          ),
          child: Text(title, style: t.small),
        ),
        childWhenDragging: Opacity(opacity: 0.4, child: _heading(t)),
        child: candidate.isEmpty
            ? _heading(t)
            : DecoratedBox(
                decoration: BoxDecoration(
                  border: Border(top: BorderSide(color: t.accent, width: 2)),
                ),
                child: _heading(t),
              ),
      ),
    );
  }

  Widget _heading(LumitTheme t) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        // **The name picks the effect; only the twirl folds it** (K-300). A
        // click that both picked and collapsed took the parameters away at the
        // moment you said which effect you meant, which is the opposite of what
        // selecting one is for. A section that cannot be picked (Source,
        // Transform) twirls on its name as it always did.
        onTap: onSelect ?? onToggle,
        onSecondaryTapUp: onContextMenu == null
            ? null
            : (details) => onContextMenu!(details.globalPosition),
        child: Container(
          color: selected ? t.selectionFill : t.surface2,
          padding: const EdgeInsets.fromLTRB(8, 4, 6, 4),
          child: Row(
            children: [
              SizedBox(
                width: fxNameColumnWidth,
                child: Row(
                  children: [
                    // Enable switch, twirl, name — the order the redesign's
                    // heading reads in (K-443): what the effect *is doing*
                    // before what the heading does to the list under it. The
                    // switch sits centred in the stopwatch column so the two
                    // glyphs share an axis down the panel.
                    if (leading case final widget?) ...[
                      SizedBox(
                        width: fxStopwatchColumn,
                        child: Center(child: widget),
                      ),
                      const SizedBox(width: 4),
                    ],
                    GestureDetector(
                      key: twirlKey,
                      behavior: HitTestBehavior.opaque,
                      onTap: onToggle,
                      child: Padding(
                        // Room to aim at, now that it is the only way in.
                        padding: const EdgeInsets.symmetric(horizontal: 2),
                        child: lumitIcon(
                          open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                          size: iconSize,
                          color: open ? t.textPrimary : t.textMuted,
                        ),
                      ),
                    ),
                    const SizedBox(width: 2),
                    Expanded(
                      // **The section's name is a kicker** (§7.1): every
                      // container label is, and a properties section header is
                      // one. The capitals are the style rather than the string,
                      // so a renamed effect keeps whatever the owner typed.
                      child: renaming && onRenamed != null
                          ? _RenameField(
                              initial: title,
                              onDone: onRenamed!,
                              onCancel: onRenameCancelled ?? () {},
                            )
                          : Text(title.toUpperCase(),
                              style: t.kickerOn,
                              overflow: TextOverflow.ellipsis),
                    ),
                  ],
                ),
              ),
              Expanded(
                child: Row(children: actions),
              ),
              if (trailing case final widget?) widget,
            ],
          ),
        ),
      );
}

/// The heading's inline rename editor (K-321): opens with the current name
/// selected — a name is retyped far more often than amended — commits on
/// Enter or on clicking away, like every inline rename (K-243), and throws the
/// edit away on Escape (K-323).
class _RenameField extends StatefulWidget {
  final String initial;
  final ValueChanged<String> onDone;
  final VoidCallback onCancel;
  const _RenameField(
      {required this.initial, required this.onDone, required this.onCancel});

  @override
  State<_RenameField> createState() => _RenameFieldState();
}

class _RenameFieldState extends State<_RenameField> {
  late final TextEditingController _controller = TextEditingController(
    text: widget.initial,
  )..selection = TextSelection(
      baseOffset: 0,
      extentOffset: widget.initial.length,
    );

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => HouseTextField(
        key: const ValueKey('fx-rename-field'),
        controller: _controller,
        width: fxNameColumnWidth - 40,
        autofocus: true,
        submitOnLostFocus: true,
        onSubmitted: widget.onDone,
        onCancelled: widget.onCancel,
      );
}

/// One property row on the panel's fixed columns (K-443): the keyframe slot,
/// the [name], and the [control], each starting at the same x on every row.
///
/// [keyframeControls] sits in a slot of [fxKeyColumnWidth] whose space is
/// reserved even when there are none, so a row that cannot animate lines up
/// with one that can — and a row whose stopwatch has just been switched on does
/// not shuffle its own label sideways.
///
/// [name] is a widget rather than a string because a name is not only text: it
/// is the row's handle for the graph editor (docs/07 §4.3) — tappable, tinted to
/// its curve's colour while selected, and carrying a dot per axis on a
/// multi-axis property. The row that owns the property builds it once and hands
/// the same widget to whichever layout it draws, so the two cannot drift.
Widget fxTwoColumnRow({
  required BuildContext context,
  required Widget name,
  Widget? keyframeControls,
  required Widget control,
}) =>
    SizedBox(
      height: fxRowHeight,
      child: Row(
        children: [
          SizedBox(
            width: fxKeyColumnWidth,
            child: keyframeControls == null
                ? null
                : Align(
                    alignment: Alignment.centerLeft, child: keyframeControls),
          ),
          const SizedBox(width: 4),
          SizedBox(width: fxLabelColumnWidth, child: name),
          Expanded(
            child: Align(alignment: Alignment.centerLeft, child: control),
          ),
        ],
      ),
    );

/// A parameter group's own twirl inside a section (P4, K-145): the sub-heading
/// an effect tucks its advanced controls behind — Bokeh's Depth map, Shake's
/// Per-axis wobble, Matte key's Screen matte.
///
/// **Why it is a row and not a nested section.** The panel is one list
/// (docs/07 §6), and a group is a fold *within* a section, not a section of its
/// own: it keeps the same hairline, the same name column and the same padding
/// as the rows around it, and differs only by a twirl and a heavier label. A
/// nested [FxSection] would bring its own heading bar and — in round mode — its
/// own card, which would read as an effect inside an effect.
///
/// Its twirl sits in the stopwatch column and its label starts at the label
/// column's x, so the fold reads as belonging to the rows beneath it and the
/// panel keeps one straight label edge from top to bottom (K-443). The label is
/// a kicker, as every container label is (§7.1).
Widget fxGroupHeaderRow(
  BuildContext context, {
  required String label,
  required bool open,
  required VoidCallback onToggle,
  Key? key,
}) {
  final t = ThemeScope.of(context).theme;
  return GestureDetector(
    key: key,
    behavior: HitTestBehavior.opaque,
    onTap: onToggle,
    child: SizedBox(
      height: fxRowHeight,
      child: Row(
        children: [
          SizedBox(
            width: fxKeyColumnWidth,
            child: Align(
              alignment: Alignment.centerLeft,
              child: Padding(
                padding: const EdgeInsets.only(left: 1),
                child: lumitIcon(
                  open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                  size: iconSize,
                  color: open ? t.textPrimary : t.textMuted,
                ),
              ),
            ),
          ),
          const SizedBox(width: 4),
          SizedBox(
            width: fxLabelColumnWidth,
            child: Text(
              label.toUpperCase(),
              style: open ? t.kickerOn : t.kicker,
              overflow: TextOverflow.ellipsis,
            ),
          ),
          const Expanded(child: SizedBox.shrink()),
        ],
      ),
    ),
  );
}

/// A section heading's text action — Reset. Sits in the value column, so it
/// reads as an action *on* the values rather than on the panel.
Widget fxTextAction(
  BuildContext context, {
  required String label,
  required String tip,
  required String keyName,
  required VoidCallback onPressed,
}) {
  final t = ThemeScope.of(context).theme;
  return LumitTooltip(
    message: tip,
    child: HouseButton(
      key: ValueKey<String>(keyName),
      frameless: true,
      small: true,
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
      onPressed: onPressed,
      child: Text(label, style: t.small.copyWith(color: t.textMuted)),
    ),
  );
}
