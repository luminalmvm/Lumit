// The shape of one section in the Effect controls panel: a heading that twirls,
// and property rows under it laid out in two columns.
//
// **In plain terms.** One list, the same list the Timeline twirls open under a
// layer: a heading bar per section, a hairline under every row, names down the
// left and their controls down the right. The columns are not divided by
// anything visible. They line up because every row starts them at the same x:
// the stopwatch, then the label, then the control.
//
// **The heading row.** The section's own enable switch where it has one, the
// twirl and the name, then whatever it has to say at the far end. What acts on
// the section comes up there under the pointer, and the rest is in its menu.
//
// **Round mode keeps its bubble**. Sharp draws the section edge to edge
// with hairlines, which is the After Effects reading; round wraps the same rows
// in the floating-card chrome, so the two shapes differ in chrome and not in
// layout.

import 'package:flutter/foundation.dart' show ValueListenable;
import 'package:flutter/widgets.dart';
import 'package:uuid/uuid.dart';

import 'package:lumit_flutter/src/rust/api/effect.dart' show BridgeKeyframe;

import '../icons/icons.dart';
import '../theme/theme.dart';
import '../widgets/share_marks.dart';
import '../widgets/controls.dart';
import '../widgets/dashed_outline.dart';
import 'fx_key_strip.dart';
import 'keyframe_controls_frb.dart' show KeyframeControlsFrb, editPickedKeys;

/// **The fixed column edges** (docs/15 §12A.3). Every row in the panel starts
/// on the same three x positions: the stopwatch, then the label, then the
/// control. A keyed row's navigator stands at the far end of the row, past the
/// control, so switching a stopwatch on moves nothing.
///
/// A glyph is 16px on its 16-unit grid (§5) and a [HouseButton] insets its child
/// by the 1px edge it always reserves, so one icon button measures 18.
const double fxStopwatchColumn = 18;

/// What every row reserves before its label.
const double fxKeyColumnWidth = fxStopwatchColumn;

/// How wide a row's link mark is: a 14px glyph with 4 either side.
const double fxLinkMarkWidth = 22;

/// The label column on a row with room to spare, wide enough that the longest
/// names in the catalogue read whole. Longer ones ellipsise rather than pushing
/// the control column, which is the point of it being fixed.
const double fxLabelColumnWidth = 142;

/// How wide the whole name side is, so the controls stack into one column down
/// the panel.
const double fxNameColumnWidth = fxKeyColumnWidth + 4 + fxLabelColumnWidth;

/// The room the control column keeps before the label starts taking the
/// panel's extra width: a well, its unit and its reset, or on a keyed row
/// its key diamond.
const double fxControlColumnWidth = 116;

/// How wide the label column is on a row [width] wide: what is left once the
/// control column has its room, between a floor that still reads a short name
/// and a ceiling that keeps a value within reach of its name in a wide panel.
double fxLabelWidthFor(double width) {
  if (!width.isFinite) return fxLabelColumnWidth;
  return (width - fxKeyColumnWidth - 10 - fxControlColumnWidth)
      .clamp(88.0, 200.0);
}

/// The label column of a row [width] wide. Beside a [compact] control a long
/// name widens it into the room the control leaves, and a short one keeps the
/// column's edge.
Widget _labelBox(double width, Widget name, double? compact) {
  final column = fxLabelWidthFor(width);
  final most = compact == null
      ? column
      : width - fxKeyColumnWidth - 10 - compact;
  if (most <= column) return SizedBox(width: column, child: name);
  return ConstrainedBox(
    constraints: BoxConstraints(minWidth: column, maxWidth: most),
    child: IntrinsicWidth(child: name),
  );
}

/// Says whether the pointer is over the row around it, for the marks that stay
/// quiet until then. A listenable, so a row lighting up repaints those marks and
/// nothing else on it.
class FxRowHover extends InheritedWidget {
  final ValueListenable<bool> over;
  const FxRowHover({super.key, required this.over, required super.child});

  @override
  bool updateShouldNotify(FxRowHover old) => old.over != over;
}

/// The pointer's place on one row, handed down through [FxRowHover].
class FxRowHoverRegion extends StatefulWidget {
  final Widget child;
  const FxRowHoverRegion({super.key, required this.child});

  @override
  State<FxRowHoverRegion> createState() => _FxRowHoverRegionState();
}

class _FxRowHoverRegionState extends State<FxRowHoverRegion> {
  final ValueNotifier<bool> _over = ValueNotifier(false);

  @override
  void dispose() {
    _over.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => MouseRegion(
        onEnter: (_) => _over.value = true,
        onExit: (_) => _over.value = false,
        child: FxRowHover(over: _over, child: widget.child),
      );
}

/// A mark that rests at [rest] and comes up at once when the pointer is on its
/// row, or the keyboard is on the mark. It stays in place and answers the
/// pointer at rest, so nothing moves as it comes and goes.
class FxQuiet extends StatefulWidget {
  final Widget child;
  final double rest;
  const FxQuiet({super.key, required this.child, this.rest = 0});

  @override
  State<FxQuiet> createState() => _FxQuietState();
}

class _FxQuietState extends State<FxQuiet> {
  bool _focused = false;

  @override
  Widget build(BuildContext context) {
    final over = context.dependOnInheritedWidgetOfExactType<FxRowHover>()?.over;
    if (over == null) return widget.child;
    return Focus(
      canRequestFocus: false,
      skipTraversal: true,
      onFocusChange: (on) => setState(() => _focused = on),
      child: ValueListenableBuilder<bool>(
        valueListenable: over,
        child: widget.child,
        builder: (context, lit, child) =>
            Opacity(opacity: lit || _focused ? 1 : widget.rest, child: child),
      ),
    );
  }
}

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
/// height for the content box settles it.
///
/// It is the *content* box, which is why it is a function of the density
/// rather than one of its values: what §12A.6's table states is the height a
/// row **occupies**, and the section spends [_fxRowChrome] of that on its own
/// 2 px above, 2 px below and the hairline under the row. So the content box
/// is whatever is left — 22 of 27 under Regular, 21 of 26 under Compact.
///
/// Controls taller than this than sit inside it rather than pushing it out —
/// their padding is squeezed by the constraint, never their text. The one
/// control that had to give ground is the stopwatch button, whose 16 px icon
/// plus 2 px of padding would have spilled: it carries none now (see
/// `keyframe_controls_frb.dart`).
///
/// Not shared with the Timeline: its lanes have their own heights, and its
/// fold-out rows take the other branch of these row widgets entirely.
double fxRowHeight(LumitTheme t) => t.density.propertyRow - _fxRowChrome;

/// What a row spends on itself before its content gets any: 2 above, 2 below,
/// and the 1 px hairline beneath it. Declared once so the two numbers in
/// §12A.6's table and the box inside them cannot drift apart.
const double _fxRowChrome = 5;

/// A section heading's height (docs/15 §12A.6): **24**, fixed, so a
/// stack of effects steps at one pitch whatever each heading carries. Its
/// padding comes out of that, leaving 20 for the switch, the twirl, the kicker
/// and the heading's own actions.
const double fxHeadingHeight = 24;

/// The effect enable switch's **hit target** on a heading (owner, desk test).
///
/// [HouseCheckbox] draws a 9px mark inside a 14px box, which is the right
/// size for a settings page and too small to aim at in a stack of effect
/// headings — the one control in the panel a person reaches for over and over,
/// and the one they were missing. The mark keeps its language; the *target*
/// grows to fill the stopwatch column and the whole height of the heading, so
/// anywhere in that little block switches the effect off.
const double fxEnableHitWidth = fxStopwatchColumn;
const double fxEnableHitHeight = fxHeadingHeight - 4;

/// How much larger the mark itself is drawn here than the 14px checkbox's
/// default — 9px becomes 11, which is the modest step the owner asked for
/// beside a heading's capitals without becoming a different control.
const double fxEnableMarkScale = 11 / 9;

/// The width of [HouseToggle]'s own box, focus ring included: what Lantern's
/// heading scales down to fit the 18px enable slot.
const double fxEnableToggleWidth = 26;

/// What a click-then-drag across the enable switches is setting them **to**
/// (item 6.2). Carried as a drag payload rather than in a shared variable so
/// the switches the pointer passes over are found the way every other
/// cross-widget gesture in the application finds its target: a [Draggable] on
/// the switch the drag starts from, a [DragTarget] on each of the others.
///
/// Its own type, so the heading's own `Draggable<int>` reorder — which this
/// gesture sits inside — cannot see it and does not budge.
@immutable
class FxEnablePaint {
  /// The state the FIRST switch took, which every switch the pointer meets
  /// takes too. Not a flip: a run of mixed switches comes out even, and
  /// dragging back over one already set does nothing.
  final bool to;
  const FxEnablePaint(this.to);
}

/// **The enable switch's face**, on its own: the checkbox at
/// [fxEnableMarkScale] and nothing around it.
///
/// Shared because a second surface wears the same control. A node card in the
/// Graph panel carries an enable tick left of its name, and it is *this* tick —
/// the same mark at the same size, answering the same bypass — rather than a
/// lookalike drawn again on a canvas. The hit target and the paint-drag differ
/// between the two (a stack of headings has neighbours to paint; a canvas of
/// boxes does not), so only the face is shared.
Widget fxEnableMark({
  Key? key,
  required bool on,
  required ValueChanged<bool> onChanged,
}) =>
    Transform.scale(
      scale: fxEnableMarkScale,
      child: HouseCheckbox(key: key, value: on, onChanged: onChanged),
    );

/// One effect's enable switch, with the drag that paints its neighbours
/// (item 6.2, owner's desk test for the hit area).
///
/// The mark keeps its language at [fxEnableMarkScale]; the target is the
/// whole stopwatch column for the whole height of the heading. A click toggles
/// this one; a drag off it sets every switch it crosses to whatever this one
/// just became.
Widget fxEnableSwitch({
  required String id,
  required bool on,
  required ValueChanged<bool> onChanged,
}) =>
    DragTarget<FxEnablePaint>(
      // Answered on the way in and only on the way in, which is once per
      // switch the pointer crosses. False either way: nothing is being
      // dropped here — the crossing IS the act — and a target that accepted
      // would draw itself as a landing place.
      onWillAcceptWithDetails: (details) {
        if (details.data.to != on) onChanged(details.data.to);
        return false;
      },
      builder: (context, _, __) => Draggable<FxEnablePaint>(
        data: FxEnablePaint(!on),
        // Nothing rides under the pointer: this drag paints switches, it does
        // not carry anything to a destination.
        feedback: const SizedBox.shrink(),
        onDragStarted: () => onChanged(!on),
        child: GestureDetector(
          key: ValueKey<String>('fx-enabled-hit-$id'),
          behavior: HitTestBehavior.opaque,
          onTap: () => onChanged(!on),
          child: SizedBox(
            width: fxEnableHitWidth,
            height: fxEnableHitHeight,
            child: Center(
              // Lantern's heading carries a switch rather than a tick. It is
              // the house toggle scaled into the same 18px slot, so the fixed
              // columns under the heading do not move between shapes.
              child: ThemeScope.of(context).theme.shape == ThemeShape.lantern
                  ? Transform.scale(
                      scale: fxEnableHitWidth / fxEnableToggleWidth,
                      child: HouseToggle(
                        key: ValueKey<String>('fx-enabled-$id'),
                        value: on,
                        onChanged: onChanged,
                      ),
                    )
                  : fxEnableMark(
                      key: ValueKey<String>('fx-enabled-$id'),
                      on: on,
                      onChanged: onChanged,
                    ),
            ),
          ),
        ),
      ),
    );

/// How thick the line is that says where a dragged effect will land. Two
/// pixels, in the accent — the same weight and colour every other drop in the
/// application draws its feedback in (the Timeline's and the Project panel's
/// outlines), so a drop indicator is recognisably one.
const double fxDropLineWidth = 2;

/// What every surface that lights an effect draws its pick from: the layer the
/// picked effects are on, and the instances themselves in stack order.
///
/// Published as one value, the way the Timeline publishes its row selection,
/// because the layer is half the answer — an instance id on its own says
/// nothing about whose stack it sits in. The gate used to be each panel's own:
/// the Effect controls panel showed one layer and asked only about ids, and the
/// Timeline pasted the layer into every path itself. It is asked here instead,
/// once, for both of them.
@immutable
class FxSelection {
  const FxSelection({this.layer, this.effects = const []});

  /// The layer the picked effects are on, as its id in string form — what a
  /// row is keyed by on both sides of the Timeline's table.
  final String? layer;

  /// The picked instances, in stack order.
  final List<UuidValue> effects;

  /// Whether [effect] on [layer] is one of the picked ones.
  bool holds(String layer, UuidValue effect) =>
      this.layer == layer && effects.contains(effect);
}

/// Which effect a section is, against the selection it should follow — what a
/// heading needs to light itself.
///
/// The selection travels as a listenable rather than as a flag handed down the
/// build, because a flag can only be changed by rebuilding the card that hands
/// it down: picking an effect redrew the whole card, every parameter row in it
/// and every button on them to recolour one word. Null on the sections that
/// are not one of several (Source, Transform) and on the ones that take no
/// part in the selection (a layer style, a group's header effect).
@immutable
class FxPick {
  const FxPick({
    required this.selection,
    required this.layer,
    required this.effect,
  });

  final ValueListenable<FxSelection> selection;

  /// The layer this section's effect is on, as its id in string form.
  final String layer;
  final UuidValue effect;

  /// Whether this section is picked as the selection now stands.
  bool get picked => selection.value.holds(layer, effect);
}

/// One twirl-open section: Source, Transform, or one effect.
class FxSection extends StatelessWidget {
  /// The section's own control, left of the name — an effect's enable switch.
  final Widget? leading;
  final String title;
  final bool open;
  final VoidCallback onToggle;

  /// What stands in the value column at all times, aligned with the controls
  /// below: a solved camera's link badge, a plug-in's own page.
  final List<Widget> actions;

  /// Hard right, in the place [hoverActions] come up: what the section has to
  /// say while the pointer is elsewhere, an effect's cost in the last frame.
  final Widget? status;

  /// What acts on the section, hard right, up while the pointer is on the
  /// heading: Reset, the close mark. Each is in the heading's menu as well.
  final List<Widget> hoverActions;

  /// A right-click on the heading, with the pointer's global position — where
  /// the actions that are not worth a permanent button live (an effect's
  /// reordering). Null leaves the secondary click unclaimed.
  final void Function(Offset at)? onContextMenu;

  /// A click on the heading's name **picks this section** — an effect
  /// is a thing that can be selected, copied and cut, and the click that says
  /// which one is the one on its name. Null (Source, Transform: sections that
  /// are not one of several) leaves the name doing what the twirl does, which
  /// is what the whole heading did before.
  final VoidCallback? onSelect;

  /// Drawn picked: the heading takes the selection fill, as a Timeline row
  /// does, so one effect chosen in either place reads the same in both.
  ///
  /// The selection is followed here rather than read off a flag, so a pick
  /// recolours the heading and rebuilds nothing above it.
  final FxPick? pick;

  /// The colours of the other people in a shared project who have this
  /// section picked, or a row inside it in hand. The heading carries a bar
  /// of them. Empty whenever nobody has. A section that names its own rows
  /// passes it. One that gives a [scope] has it worked out.
  final List<int> theirs;

  /// What this section's rows are called in a shared project, for a section
  /// whose rows have no names of their own: each is `scope/#n`, by its place
  /// in [rows]. Where the Timeline has a group for the same rows this is its
  /// path, so someone working in that group there marks this heading. Null
  /// for a section that names each row itself, or takes no part.
  final String? scope;

  /// The twirl mark's own key — it is the only thing that folds a selectable
  /// section, so it is worth being able to point at.
  final Key? twirlKey;

  /// Which thing in its list this section is, when the heading may be
  /// **dragged** to another place in it (docs/07 §6's drag-to-reorder) — an
  /// effect's own id. Null — Source, Transform, anything that does not sit in
  /// a reorderable stack — leaves the heading undraggable and accepting
  /// nothing.
  ///
  /// The thing rather than its position, because a card outlives the
  /// positions around it: removing an effect above this one moves it down the
  /// stack without the heading being rebuilt.
  final String? dragKey;

  /// A heading dropped on this one: the [dragKey] it came from. Called only
  /// when [dragKey] is set and the two differ.
  final void Function(String from)? onDropped;

  /// Whether the effect dragged from [from] lands below this heading, which is
  /// which edge the drop line is drawn on. Asked while the pointer is over
  /// this heading, so the answer is read from the list as it is now.
  final bool Function(String from)? landsBelow;

  /// The rows under the heading, drawn only while [open].
  final List<Widget> rows;

  /// While true the heading's name is an inline editor instead of a label —
  /// how an effect is renamed (`Enter` on the selected effect).
  /// Sections that cannot be renamed (Source, Transform) never set it.
  final bool renaming;

  /// The rename's commit: the typed name, empty to clear back to the
  /// effect's own label. Called on Enter and on clicking away, the same
  /// contract every inline rename in the application has.
  final ValueChanged<String>? onRenamed;

  /// `Escape` while renaming: close the editor and keep the old name.
  final VoidCallback? onRenameCancelled;

  /// False while the effect is **bypassed**. The heading takes a dashed
  /// outline and the rows below dim with it, still readable and still live,
  /// so what it was set to can be read and changed. Sections that cannot be
  /// switched off leave it true.
  final bool enabled;

  /// Every key under this heading, for the keyframe strip's summary lane on
  /// the heading while the section is shut. Empty on a section with none, or
  /// one that does not say.
  final List<BridgeKeyframe> summaryKeys;

  const FxSection({
    super.key,
    required this.title,
    required this.open,
    required this.onToggle,
    required this.rows,
    this.leading,
    this.actions = const [],
    this.status,
    this.hoverActions = const [],
    this.onContextMenu,
    this.onSelect,
    this.pick,
    this.theirs = const [],
    this.scope,
    this.twirlKey,
    this.dragKey,
    this.onDropped,
    this.landsBelow,
    this.renaming = false,
    this.onRenamed,
    this.onRenameCancelled,
    this.enabled = true,
    this.summaryKeys = const [],
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Bypassed: the heading wears a dashed outline and its rows dim.
    final heading = enabled
        ? _draggableHeading(t)
        : DashedOutline(child: _draggableHeading(t));
    // Asked for only by a section whose rows go by their place in it, so
    // every other section builds without looking.
    final share = scope == null ? null : ShareRows.maybeOf(context);
    final named = share != null && share.active ? share : null;
    final holders = named == null ? theirs : named.under(scope!);
    final column = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (holders.isNotEmpty)
          DecoratedBox(
            position: DecorationPosition.foreground,
            decoration: TheirBar(
              [for (final c in holders) t.personColour(c)],
              tint: 0.12,
            ),
            child: heading,
          )
        else
          heading,
        OpenReveal(
          open: open,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (var i = 0; i < rows.length; i++)
                Container(
                  decoration: BoxDecoration(
                      border: Border(bottom: BorderSide(color: t.hairline))),
                  padding: const EdgeInsets.fromLTRB(10, 2, 10, 2),
                  child: Opacity(
                    opacity: enabled ? 1 : 0.5,
                    child: named == null
                        ? rows[i]
                        : named.row('$scope/#$i', rows[i]),
                  ),
                ),
            ],
          ),
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
      // The edge again over the content, which would otherwise cover it where
      // the corner curves.
      foregroundDecoration: BoxDecoration(
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
  /// items in the Project panel), so an effect stack reorders the same way.
  ///
  /// **The drop indicator** (owner, desk test). The heading used to take a
  /// line along its top edge whichever way the drag came from, which said the
  /// wrong thing half the time: dropping an effect from *above* onto this one
  /// puts it after this one, and a line drawn above it pointed at the gap it
  /// was not going into. The line is now drawn on the edge the effect will
  /// actually land against — under the heading when it is travelling down the
  /// stack, over it when it is travelling up — so it reads as the insertion
  /// point rather than as a highlight on the row.
  Widget _draggableHeading(LumitTheme t) {
    final key = dragKey;
    if (key == null || onDropped == null) return _heading(t);
    return DragTarget<String>(
      onWillAcceptWithDetails: (d) => d.data != key,
      onAcceptWithDetails: (d) => onDropped!(d.data),
      builder: (context, candidate, _) {
        // Which side the gap is on: a heading dragged from above this one
        // lands below it, and one from below lands above it.
        final from = candidate.isEmpty ? null : candidate.first;
        final line = BorderSide(color: t.accent, width: fxDropLineWidth);
        return Draggable<String>(
          data: key,
          // The pointer carries the effect's name and nothing else: a
          // full-width card under the cursor hides the stack it is being
          // placed into.
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
          child: from == null
              ? _heading(t)
              : DecoratedBox(
                  key: const ValueKey('fx-drop-line'),
                  // Foreground, so the line sits over the heading's own fill
                  // rather than being painted under it.
                  position: DecorationPosition.foreground,
                  decoration: BoxDecoration(
                    border: landsBelow?.call(from) ?? false
                        ? Border(bottom: line)
                        : Border(top: line),
                  ),
                  child: _heading(t),
                ),
        );
      },
    );
  }

  Widget _heading(LumitTheme t) {
    // Built once, above the fill and passed through it: recolouring a heading
    // that has just been picked must not rebuild what is inside it.
    final names = Row(
      children: [
        // Enable switch, twirl, name: what the effect *is doing* before what
        // the heading does to the list under it. The switch sits centred in
        // the stopwatch column so the two glyphs share an axis down the panel.
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
          child: HoverMark(
            builder: (context, lit) => Padding(
              // Room to aim at, now that it is the only way in.
              padding: const EdgeInsets.symmetric(horizontal: 2),
              child: TwirlTurn(
                open: open,
                child: lumitIcon(
                  open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                  size: iconSize,
                  color: open
                      ? t.textPrimary
                      : Color.lerp(t.textMuted, t.textPrimary, lit)!,
                ),
              ),
            ),
          ),
        ),
        const SizedBox(width: 2),
        Expanded(
          // **The section's name is a kicker** (§7.1): every container label
          // is, and a properties section header is one. The capitals are the
          // style rather than the string, so a renamed effect keeps what was
          // typed.
          child: renaming && onRenamed != null
              ? FxRenameField(
                  initial: title,
                  onDone: onRenamed!,
                  onCancel: onRenameCancelled ?? () {},
                )
              : Text(t.kickerCase(title),
                  // Bypassed, the name drops to muted with the fill.
                  style: enabled ? t.kickerOn : t.kicker,
                  overflow: TextOverflow.ellipsis),
        ),
      ],
    );
    final content = Row(
      children: [
        if (actions.isEmpty)
          Expanded(child: names)
        else
          // The actions start where the rows' controls do, at every width.
          Expanded(
            child: LayoutBuilder(
              builder: (context, box) => Row(children: [
                SizedBox(
                    width: fxKeyColumnWidth + 4 + fxLabelWidthFor(box.maxWidth),
                    child: names),
                Expanded(child: Row(children: actions)),
              ]),
            ),
          ),
        // The status and the actions share one place, and swap at once: the
        // name keeps its room, and nothing beside them moves.
        if (status != null || hoverActions.isNotEmpty)
          Stack(
            alignment: AlignmentDirectional.centerEnd,
            children: [
              if (hoverActions.isNotEmpty)
                FxQuiet(
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: hoverActions,
                  ),
                ),
              if (status case final widget?) _FxRestOnly(child: widget),
            ],
          ),
        // The keyframe strip's column, kept on the heading too so the close
        // mark stands clear of it. Shut, it carries the section's keys.
        Builder(
          builder: (context) {
            final strip = FxKeyStripScope.widthOf(context);
            if (strip <= 0) return const SizedBox.shrink();
            return SizedBox(
              width: strip,
              height: fxHeadingHeight - 4,
              child: open ? null : FxKeySummaryLane(keys: summaryKeys),
            );
          },
        ),
      ],
    );
    final picked = pick;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      // **The name picks the effect; only the twirl folds it**. A
      // click that both picked and collapsed took the parameters away at the
      // moment you said which effect you meant, which is the opposite of what
      // selecting one is for. A section that cannot be picked (Source,
      // Transform) twirls on its name as it always did.
      onTap: onSelect ?? onToggle,
      onSecondaryTapUp: onContextMenu == null
          ? null
          : (details) => onContextMenu!(details.globalPosition),
      // The heading lifts a step under the pointer: it is the thing to press
      // to pick the effect, and looked like a title.
      child: FxRowHoverRegion(
        child: HoverMark(
          builder: (context, hover) => picked == null
              ? _headingBox(t, false, content, hover)
              : _FxPickedFill(
                  pick: picked,
                  box: (lit) => _headingBox(t, lit, content, hover),
                ),
        ),
      ),
    );
  }

  /// The heading's box, around contents built above it: the fill, the fixed
  /// height and the padding. [hover] is how far the pointer has lit it.
  Widget _headingBox(LumitTheme t, bool lit, Widget content, double hover) =>
      Container(
        height: fxHeadingHeight,
        // Bypassed and unpicked, the heading has no fill at all: the dashed
        // outline is left standing on the panel's own ground, so the effect
        // reads as absent rather than as another live row.
        color: lit
            ? t.selectionFill
            : enabled
                ? Color.alphaBlend(hoverTint(t, hover), t.surface2)
                : hover > 0
                    ? hoverTint(t, hover)
                    : null,
        padding: const EdgeInsets.fromLTRB(10, 2, 10, 2),
        child: content,
      );
}

/// A reading that gives its place to the actions under the pointer.
class _FxRestOnly extends StatelessWidget {
  final Widget child;
  const _FxRestOnly({required this.child});

  @override
  Widget build(BuildContext context) {
    final over = context.dependOnInheritedWidgetOfExactType<FxRowHover>()?.over;
    final reading = IgnorePointer(child: child);
    if (over == null) return reading;
    return ValueListenableBuilder<bool>(
      valueListenable: over,
      child: reading,
      builder: (context, lit, child) =>
          Opacity(opacity: lit ? 0 : 1, child: child),
    );
  }
}

/// The heading's fill, and the only thing a pick redraws.
///
/// It follows the selection itself and asks the shared gate its one question,
/// so the answer for one heading cannot drag the others along: the selection is
/// a single list for the whole stack, and a section that read it in its own
/// build redrew every card in the panel whenever any card was picked. The
/// heading's contents are built above this and handed through unchanged, so
/// what a pick rebuilds is the box around them.
class _FxPickedFill extends StatefulWidget {
  const _FxPickedFill({required this.pick, required this.box});

  final FxPick pick;

  /// The heading, drawn lit or unlit.
  final Widget Function(bool lit) box;

  @override
  State<_FxPickedFill> createState() => _FxPickedFillState();
}

class _FxPickedFillState extends State<_FxPickedFill> {
  late bool _lit = widget.pick.picked;

  @override
  void initState() {
    super.initState();
    widget.pick.selection.addListener(_follow);
  }

  @override
  void didUpdateWidget(covariant _FxPickedFill old) {
    super.didUpdateWidget(old);
    if (old.pick.selection != widget.pick.selection) {
      old.pick.selection.removeListener(_follow);
      widget.pick.selection.addListener(_follow);
    }
    // The section at this place in the stack may be a different effect now —
    // an undo, a reorder, a delete — so the answer is taken afresh while the
    // panel is rebuilding anyway.
    _lit = widget.pick.picked;
  }

  @override
  void dispose() {
    widget.pick.selection.removeListener(_follow);
    super.dispose();
  }

  void _follow() {
    final next = widget.pick.picked;
    if (next == _lit) return;
    setState(() => _lit = next);
  }

  @override
  Widget build(BuildContext context) => widget.box(_lit);
}

/// The heading's inline rename editor: opens with the current name
/// selected — a name is retyped far more often than amended — commits on
/// Enter or on clicking away, like every inline rename, and throws the
/// edit away on Escape.
class FxRenameField extends StatefulWidget {
  final String initial;
  final ValueChanged<String> onDone;
  final VoidCallback onCancel;
  const FxRenameField(
      {super.key,
      required this.initial,
      required this.onDone,
      required this.onCancel});

  @override
  State<FxRenameField> createState() => _FxRenameFieldState();
}

class _FxRenameFieldState extends State<FxRenameField> {
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

/// One property row on the panel's fixed columns: the stopwatch, the [name],
/// and the [control], each starting at the same x on every row.
///
/// [keyframeControls] sits in a slot of [fxKeyColumnWidth] whose space is
/// reserved even when there are none, so a row that cannot animate lines up
/// with one that can. A keyed row's navigator takes the far end of the row, past
/// the control, so nothing on it moves when its stopwatch is switched on.
///
/// [name] is a widget rather than a string because a name is not only text: it
/// is the row's handle for the graph editor (docs/07 §4.3) — tappable, tinted to
/// its curve's colour while selected, and carrying a dot per axis on a
/// multi-axis property. The row that owns the property builds it once and hands
/// the same widget to whichever layout it draws, so the two cannot drift.
///
/// [grows] lets the row be taller than one line, for a control that drops
/// part of itself onto a second line when the panel is narrow.
///
/// [lane] is the row's lane in the keyframe strip ([FxKeyLane]), drawn in the
/// column every row reserves at its right edge while the strip is shown. A
/// row with nothing to key leaves the column empty, so the rows above and
/// below still line up on it.
///
/// [rowEnd] stands where a navigator would, on a row with none.
///
/// [compactControl] is how wide a small control is, a checkbox, on a row
/// whose label may then take what the control column has spare.
///
/// [controlNeeds] is how wide the control has to stay for its number to read
/// whole. The marks past it give way first: the link mark, then the key
/// steps either side of the diamond, then the diamond.
Widget fxTwoColumnRow({
  required BuildContext context,
  required Widget name,
  Widget? keyframeControls,
  required Widget control,
  bool grows = false,
  Widget? lane,
  Widget? rowEnd,
  Widget? trailing,
  double? compactControl,
  double? controlNeeds,
}) {
  final height = fxRowHeight(ThemeScope.of(context).theme);
  final strip = FxKeyStripScope.widthOf(context);
  // In a narrow panel the lane has a line of its own under an animated row.
  final under = FxKeyStripScope.stackedLane(context, lane);
  // A value edit on a row with keys picked in the strip moves them all.
  final picks = FxKeyStripScope.maybeOf(context);
  if (lane is FxKeyLane && picks != null && picks.width > 0) {
    control = ValueEditScope(
      run: (typed, edit) =>
          editPickedKeys(picks.pickedOn(lane.rowKey), typed, edit),
      child: control,
    );
  }
  final keys = keyframeControls;
  final keyed = keys is KeyframeControlsFrb && keys.animated ? keys : null;
  final row = ConstrainedBox(
    constraints: grows
        ? BoxConstraints(minHeight: height)
        : BoxConstraints.tightFor(height: height),
    child: Row(
      children: [
        Expanded(
          child: LayoutBuilder(builder: (context, constraints) {
            final spare = controlNeeds == null
                ? double.infinity
                : constraints.maxWidth -
                    fxKeyColumnWidth -
                    10 -
                    fxLabelWidthFor(constraints.maxWidth) -
                    controlNeeds;
            final steps = keyed == null ? 0.0 : 3 * fxStopwatchColumn;
            final mark =
                spare >= steps + fxLinkMarkWidth ? trailing : null;
            final end = keyed == null
                ? rowEnd
                : spare >= steps
                    ? keyed.navigator()
                    : spare >= fxStopwatchColumn
                        ? keyed.navigator(steps: false)
                        : null;
            return Row(
              children: [
                SizedBox(
                  width: fxKeyColumnWidth,
                  child: keyframeControls == null
                      ? null
                      : Align(
                          alignment: AlignmentDirectional.centerStart,
                          child: keyframeControls),
                ),
                const SizedBox(width: 4),
                _labelBox(constraints.maxWidth, name, compactControl),
                // A long name ends short of its value rather than against it.
                Expanded(
                  child: Padding(
                    padding: const EdgeInsetsDirectional.only(start: 6),
                    child: Align(
                        alignment: AlignmentDirectional.centerStart,
                        child: control),
                  ),
                ),
                if (end != null) end,
                // The row's last thing, after the key navigator: the link
                // mark, laid out always and shown on hover.
                if (mark != null) mark,
              ],
            );
          }),
        ),
        if (strip > 0) SizedBox(width: strip, height: height, child: lane),
      ],
    ),
  );
  final hovered = FxRowHoverRegion(child: row);
  if (under == null) return hovered;
  return Column(
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [hovered, under],
  );
}

/// A parameter group's own twirl inside a section (P4): the sub-heading
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
/// panel keeps one straight label edge from top to bottom. The label is
/// a kicker, as every container label is (§7.1).
///
/// [summary] is what a shut fold says about the rows inside it, drawn muted
/// in the value column. Without one the label has the whole row to itself.
Widget fxGroupHeaderRow(
  BuildContext context, {
  required String label,
  required bool open,
  required VoidCallback onToggle,
  String? summary,
  Key? key,
  void Function(Offset at)? onContextMenu,
}) {
  final t = ThemeScope.of(context).theme;
  return GestureDetector(
    key: key,
    behavior: HitTestBehavior.opaque,
    onTap: onToggle,
    onSecondaryTapUp:
        onContextMenu == null ? null : (d) => onContextMenu(d.globalPosition),
    child: HoverMark(builder: (context, lit) {
      // Shut, the name and the twirl light towards how they read open.
      final ink = open
          ? t.textPrimary
          : Color.lerp(t.textMuted, t.textPrimary, lit)!;
      final name = Text(
        t.kickerCase(label),
        style: open
            ? t.kickerOn
            : t.kicker.copyWith(
                color: Color.lerp(t.kicker.color, t.kickerOn.color, lit)),
        overflow: TextOverflow.ellipsis,
      );
      // The keyframe strip's column is kept clear here too, so the rows
      // under this fold line up with the ones above it.
      final strip = FxKeyStripScope.widthOf(context);
      return Container(
        height: fxRowHeight(t),
        margin: EdgeInsetsDirectional.only(end: strip),
        child: LayoutBuilder(
          builder: (context, constraints) => Row(
            children: [
              SizedBox(
                width: fxKeyColumnWidth,
                child: Align(
                  alignment: AlignmentDirectional.centerStart,
                  child: Padding(
                    padding: const EdgeInsetsDirectional.only(start: 1),
                    child: TwirlTurn(
                      open: open,
                      child: lumitIcon(
                        open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                        size: iconSize,
                        color: ink,
                      ),
                    ),
                  ),
                ),
              ),
              const SizedBox(width: 4),
              if (summary == null)
                Expanded(child: name)
              else ...[
                SizedBox(
                    width: fxLabelWidthFor(constraints.maxWidth),
                    child: name),
                // On the same edge as the controls in the rows around it.
                const SizedBox(width: 6),
                Expanded(
                  child: Text(
                    summary,
                    style: t.small.copyWith(color: t.textMuted),
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ],
            ],
          ),
        ),
      );
    }),
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
      // A little word beside a value is plain mono, not body text (§7.1).
      child:
          Text(t.propertyCase(label),
              style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
    ),
  );
}
