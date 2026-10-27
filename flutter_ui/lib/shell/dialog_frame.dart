// The shape every popup shares: a title strip, an optional row of page tabs,
// label-left rows inside titled groups, and a footer carrying a factual line
// and the single filled action (§12A.4).
//
// **In plain terms.** Two dialogs are drawn in the approved mockups — New
// composition and Export — and they are drawn as the same object at two sizes.
// This file is that object: the pieces below are what both are assembled from,
// so a change to the pattern is one edit rather than two that drift.
//
// A dialog is built **in the window**, as an ordinary framework overlay. That
// is not a stopgap: when Flutter's windowing reaches the stable channel a
// `showDialog` becomes a real child window with no rewrite here.
//
// The measurements are the drawings' own, read off their computed styles and
// pinned by `export_metrics_test` / `comp_settings_metrics_test`. A value that
// disagrees with the drawing is a defect (§12A.6).

import 'package:flutter/widgets.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../theme/style_sheet.dart';
import '../widgets/controls.dart';

/// The title strip, over its hairline.
const double dialogTitleStrip = 30;

/// The close mark in it, at the size the drawings render it.
const double dialogCloseGlyph = 12;

/// The page-tab row under the title strip, over its own hairline, and the gap
/// between two tabs.
const double dialogTabRow = 26;
const double dialogTabGap = 2;

/// The footer: 10 above a 24px button and 10 below it, over a hairline.
const double dialogFooterHeight = 45;
const double dialogFooterButton = 24;
const double dialogFooterGap = 12;

/// The air between two **stacked** actions, and the band they add up to.
///
/// A footer whose actions will not fit on one line drops them to a column
/// instead of eliding their words — §12A.6's ladder, step 2, applied to a
/// footer. Three buttons come to 10 above, 24 · 8 · 24 · 8 · 24, and 10 below.
const double dialogFooterStackGap = 8;
const double dialogFooterPad = 10;

/// The inset the title strip, the footer and a body take from the dialog's
/// edges.
const double dialogPadding = 14;

/// A dropdown's closed face, a value well, and a plain button inside the body:
/// 22, the one height §12A.6 gives them in a dialog.
const double dialogControlHeight = 22;

/// A titled group: a hairline box with its kicker notched into the top edge,
/// and the air between two of them. The box's corner is the shape's
/// `sectionRadius`.
const double dialogGroupGap = 10;

/// The factual mono lines — the footer's summary, a group's own reading.
const double dialogMonoSize = 10;

/// The frame itself: the drawing's `surface_1` out to a hairline edge.
///
/// The edge is a foreground decoration so it is painted *over* the outermost
/// pixel rather than insetting everything by one — the drawing's 640 is the
/// room inside the frame, not the room inside the frame less its border.
class DialogFrame extends StatelessWidget {
  /// The frame's own width, or null to take the width it is given.
  ///
  /// Every dialog but one states a width, because a dialog is drawn to a
  /// measure. A **resizable** window is the exception: the window around it
  /// has the size, and a frame that fixed its own would win against the
  /// corner grip (the shape settings_window_frb.dart already uses).
  final double? width;
  final List<Widget> children;

  const DialogFrame({super.key, this.width, required this.children});

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    return SizedBox(
      width: width,
      child: dialogSurface(
        context,
        t,
        Entrance.content(
          spec: scope.motion.content,
          rise: scope.motion.contentRise,
          blur: scope.motion.contentBlur,
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: children,
          ),
        ),
      ),
    );
  }
}

/// A dialogue's box: `surface1` out to a hairline edge drawn over its outer
/// pixel, the float corner and shadow, with a style's `dialog` rules over it.
/// Shared by every dialogue and the Settings window.
Widget dialogSurface(BuildContext context, LumitTheme t, Widget child) {
  final style = partStyle(context, t, 'dialog');
  final radius = BorderRadius.circular(style.radius ?? t.tokens.floatRadius);
  return frosted(
    style,
    radius,
    Container(
      decoration: dress(
        BoxDecoration(
          color: t.surface1,
          borderRadius: BorderRadius.circular(t.tokens.floatRadius),
          boxShadow: t.floatShadow,
        ),
        PartStyle(
            background: style.background,
            radius: style.radius,
            shadows: style.shadows,
            blur: style.blur),
      ),
      foregroundDecoration: dress(
        BoxDecoration(
          borderRadius: BorderRadius.circular(t.tokens.floatRadius),
          border: Border.all(color: t.hairline),
        ),
        PartStyle(
            borderColour: style.borderColour,
            borderWidth: style.borderWidth,
            radius: style.radius),
      ),
      child: ClipRRect(
        borderRadius: radius,
        child: StylePart(name: 'dialog', child: child),
      ),
    ),
  );
}

/// The title strip: what this dialog is, what it is about, and the way out.
///
/// [subject] is the thing being acted on — the composition's name in the
/// Export dialog — and it reads in the quiet colour beside the kicker, because
/// it is the user's word rather than the application's.
///
/// **The subject takes the whole of the space and the close mark is pushed to
/// the far end of it** — the drawing's own `margin-left: auto` on the mark.
/// The strip used to hold a `Flexible` subject *and* a `Spacer`, which are two
/// flexible children of equal flex: a `Row` hands each of them half the free
/// space, a short composition name uses almost none of its half, and the
/// leftover is not given back — it falls to the end of the row, behind the
/// close mark. So the mark drifted inward by half of whatever the name did not
/// need and only reached the corner when the name was long enough to fill its
/// share. One flexible child cannot do that.
///
/// A shape whose titles sit centred (Lantern) puts the name and the subject
/// in the middle of the strip with the header dot before them, and the close
/// mark stays in its corner.
Widget dialogTitleBar(
  LumitTheme t, {
  required String title,
  String subject = '',
  required VoidCallback onClose,
  required String keyPrefix,
}) {
  final name = Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      if (t.tokens.headerDot)
        Container(
          key: ValueKey<String>('$keyPrefix-title-dot'),
          width: 6,
          height: 6,
          margin: const EdgeInsets.only(right: 6),
          decoration: BoxDecoration(color: t.accent, shape: BoxShape.circle),
        ),
      Text(t.kickerCase(title), style: t.kickerOn),
    ],
  );
  final about = Padding(
    padding: const EdgeInsets.only(left: 10),
    child: Text(
      subject,
      style: t.body.copyWith(color: t.textMuted),
      overflow: TextOverflow.ellipsis,
    ),
  );
  final close = LumitTooltip(
    message: l10n.close,
    child: HoverMark(
      swells: true,
      // From the glyph's own middle, which sits at the box's right.
      alignment: const Alignment(0.6, 0),
      builder: (context, lit) => GestureDetector(
        key: ValueKey<String>('$keyPrefix-close'),
        behavior: HitTestBehavior.opaque,
        onTap: onClose,
        child: SizedBox(
          // The mark is 12 wide at the drawing's inset; the extra 8 of
          // target hangs to its *left*, into the strip rather than into
          // the corner, so a comfortable click area costs the glyph no
          // part of the position the drawing gives it.
          width: dialogCloseGlyph + 8,
          height: dialogTitleStrip,
          child: Align(
            alignment: Alignment.centerRight,
            child: glyph.LumitIcon(
              LumitIcons.close,
              size: dialogCloseGlyph,
              colour: Color.lerp(t.textMuted, t.textPrimary, lit),
              semanticLabel: l10n.close,
            ),
          ),
        ),
      ),
    ),
  );
  return Container(
    key: ValueKey<String>('$keyPrefix-title-strip'),
    height: dialogTitleStrip + 1,
    decoration: BoxDecoration(
      color: t.surface2,
      border: Border(bottom: BorderSide(color: t.hairline)),
    ),
    // 14 either side, as the drawing computes it: the mark's own inset from
    // the corner is the strip's, not a smaller one of its own.
    padding: const EdgeInsets.symmetric(horizontal: dialogPadding),
    child: t.tokens.titleCentred
        ? Stack(
            alignment: Alignment.center,
            children: [
              Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  name,
                  if (subject.isNotEmpty) Flexible(child: about),
                ],
              ),
              Align(alignment: Alignment.centerRight, child: close),
            ],
          )
        : Row(
            children: [
              name,
              Expanded(
                  child: Align(alignment: Alignment.centerLeft, child: about)),
              close,
            ],
          ),
  );
}

/// The page-tab row: kickers in a line, the one in force in the bright colour
/// over an accent rule — §3.1's one job for the accent in a list of names.
Widget dialogTabs<T>(
  LumitTheme t, {
  required List<(T, String)> tabs,
  required T current,
  required ValueChanged<T> onPick,
  required String keyPrefix,
}) =>
    Container(
      key: ValueKey<String>('$keyPrefix-tabs'),
      height: dialogTabRow + 1,
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      padding: const EdgeInsets.symmetric(horizontal: 10),
      child: Row(
        children: [
          for (final (page, label) in tabs)
            HoverMark(
              builder: (context, lit) => GestureDetector(
                key: ValueKey<String>('$keyPrefix-tab-$page'),
                behavior: HitTestBehavior.opaque,
                onTap: () => onPick(page),
                child: Container(
                  margin: const EdgeInsets.only(right: dialogTabGap),
                  padding: const EdgeInsets.fromLTRB(6, 6, 6, 5),
                  decoration: BoxDecoration(
                    border: Border(
                      bottom: BorderSide(
                        // Clear when it is not the page in force, and a faint
                        // rule under the pointer: the rule is always there,
                        // so nothing shifts by a pixel as the tab changes
                        // (§7.1).
                        color: page == current
                            ? t.accent
                            : t.hairlineStrong.withValues(alpha: lit),
                      ),
                    ),
                  ),
                  child: Text(
                    t.kickerCase(label),
                    // A page that is not in force lights towards the one
                    // that is.
                    style: page == current
                        ? t.kickerOn
                        : t.kicker.copyWith(
                            color: Color.lerp(
                                t.kicker.color, t.kickerOn.color, lit)),
                  ),
                ),
              ),
            ),
        ],
      ),
    );

/// A titled group: a hairline box with its name notched into the top edge.
///
/// The drawing puts a *box* round each group here rather than the rule the
/// Settings pages use — a dialog read once wants its areas fenced, a settings
/// page lived in wants them separated. Each drawing decides its own.
/// [highlighted] lights the box's edge for a moment — what a section tab does
/// to the section it jumps to on a dialog that scrolls rather than paging. The
/// accent is doing here exactly what it does in the tab strip: saying
/// *this is the one you asked for*, and only while you are asking.
///
/// [dimmed] is a whole group the chosen output has no use for — the Audio group
/// of a folder of stills, the picture groups of a sound file. Its name goes to
/// `text_disabled` and nothing inside it answers a click; the rows stay drawn
/// and legible, because being off is not being gone (docs/15 §5). Deaf but not
/// faded, the same idiom a driven parameter row wears.
Widget dialogGroup(
  LumitTheme t,
  String title,
  List<Widget> rows, {
  Key? key,
  bool highlighted = false,
  bool dimmed = false,
}) =>
    Padding(
      key: key,
      padding: const EdgeInsets.only(top: 8),
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Container(
            decoration: BoxDecoration(
              border: Border.all(color: highlighted ? t.accent : t.hairline),
              borderRadius: BorderRadius.circular(t.tokens.sectionRadius),
            ),
            padding: const EdgeInsets.fromLTRB(12, 8, 12, 6),
            child: IgnorePointer(
              ignoring: dimmed,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                mainAxisSize: MainAxisSize.min,
                children: rows,
              ),
            ),
          ),
          // The kicker sits *on* the edge, its own background cutting the line
          // behind it — the group names itself without spending a row on it.
          Positioned(
            top: -6,
            left: 10,
            child: Container(
              color: t.surface1,
              padding: const EdgeInsets.symmetric(horizontal: 4),
              child: Text(t.kickerCase(title),
                  style: t.kicker.copyWith(
                      color: dimmed ? t.textDisabled : t.textSecondary)),
            ),
          ),
        ],
      ),
    );

/// One row: its name on the left in a fixed column, its control beside it.
///
/// [labelColumn] and [gap] are the drawing's, and they differ between dialogs
/// — Export computes 100 and 10 in a 640px frame, New composition 110 and 12
/// in a 520px one. Neither is "the" dialog row; each drawing measures its own,
/// which is why this takes them rather than fixing one.
Widget dialogRow(
  LumitTheme t,
  String label,
  Widget control, {
  required double labelColumn,
  double gap = 10,
  double minHeight = 28,
  Key? key,
  bool dimmed = false,
}) =>
    ConstrainedBox(
      key: key,
      constraints: BoxConstraints(minHeight: minHeight),
      child: Row(
        children: [
          SizedBox(
              width: labelColumn,
              child: Text(label,
                  style: dimmed
                      ? t.body.copyWith(color: t.textDisabled)
                      : t.body)),
          SizedBox(width: gap),
          Expanded(child: control),
        ],
      ),
    );

/// The footer: a factual line, then the actions, the filled one last.
///
/// The single filled action is the ceiling, not the floor — a dialog with
/// nothing to commit carries outlined buttons and no fill (§12A.4).
///
/// [stacked] drops the actions to a column, each at the footer's full width,
/// for a dialog too narrow to seat them in a line — §12A.6's ladder step 2. The
/// order is unchanged, so the filled action is still last, which in a column
/// means the bottom.
///
/// [leading] stands at the far edge in the line's place: an action kept
/// apart from the way out, such as one that ends something for everybody.
Widget dialogFooter(
  LumitTheme t, {
  String summary = '',
  Widget? leading,
  required List<Widget> actions,
  required String keyPrefix,
  bool stacked = false,
}) =>
    Container(
      key: ValueKey<String>('$keyPrefix-footer'),
      height: stacked ? null : dialogFooterHeight,
      decoration: BoxDecoration(
        color: t.surface2,
        border: Border(top: BorderSide(color: t.hairline)),
      ),
      padding: stacked
          ? const EdgeInsets.fromLTRB(
              dialogPadding, dialogFooterPad, dialogPadding, dialogFooterPad)
          : const EdgeInsets.symmetric(horizontal: dialogPadding),
      child: stacked
          ? Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (summary.isNotEmpty)
                  Padding(
                    padding:
                        const EdgeInsets.only(bottom: dialogFooterStackGap),
                    child: Text(
                      summary,
                      key: ValueKey<String>('$keyPrefix-summary'),
                      style: dialogMono(t),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                for (final (index, action) in actions.indexed) ...[
                  if (index > 0) const SizedBox(height: dialogFooterStackGap),
                  SizedBox(height: dialogFooterButton, child: action),
                ],
              ],
            )
          : Row(
              children: [
                // The line takes all the room, which stands the actions at
                // the far end whatever its length. Sharing the room with a
                // spacer left them adrift in the middle under a short line.
                Expanded(
                  child: leading != null
                      ? Align(
                          alignment: Alignment.centerLeft,
                          child: SizedBox(
                              height: dialogFooterButton, child: leading),
                        )
                      : summary.isEmpty
                      ? const SizedBox.shrink()
                      : Text(
                          summary,
                          key: ValueKey<String>('$keyPrefix-summary'),
                          style: dialogMono(t),
                          overflow: TextOverflow.ellipsis,
                        ),
                ),
                for (final action in actions) ...[
                  const SizedBox(width: dialogFooterGap),
                  SizedBox(height: dialogFooterButton, child: action),
                ],
              ],
            ),
    );

/// The scrollbar Settings has, for a list long enough to be travelled by it:
/// a track down the side of [child], a thumb that drags, and a press on the
/// track that pages. [top] starts it lower, under something held over the
/// list.
///
/// The strip laid over the gutter is what takes the drag. A scrollbar's own
/// thumb answers a drag too, but one that starts anywhere else on the gutter
/// falls through to the window and carries it off.
Widget dialogGutter(
    BuildContext context, LumitTheme t, ScrollController controller,
    {double top = 0, required Widget child}) {
  const thickness = 6.0, inset = 2.0, ends = 8.0;
  return Stack(
    children: [
      RawScrollbar(
        controller: controller,
        thumbVisibility: true,
        trackVisibility: true,
        thickness: thickness,
        radius: const Radius.circular(3),
        thumbColor: t.surface4,
        trackColor: t.surface2,
        trackRadius: const Radius.circular(3),
        padding: EdgeInsets.fromLTRB(0, top + ends, inset, ends),
        child: ScrollConfiguration(
          behavior:
              ScrollConfiguration.of(context).copyWith(scrollbars: false),
          child: child,
        ),
      ),
      Positioned(
        right: inset,
        top: top + ends,
        bottom: ends,
        width: thickness,
        child: GestureDetector(
          behavior: HitTestBehavior.translucent,
          onVerticalDragUpdate: (drag) {
            if (!controller.hasClients) return;
            final position = controller.position;
            final viewport = position.viewportDimension;
            final range = position.maxScrollExtent;
            if (viewport <= 0 || range <= 0) return;
            // A pixel of gutter is worth as much of the list as a pixel of
            // thumb, so the thumb keeps up with the pointer.
            controller.jumpTo((position.pixels +
                    drag.delta.dy * (viewport + range) / viewport)
                .clamp(0.0, range));
          },
        ),
      ),
    ],
  );
}

/// A footer's filled action. With nothing to do it keeps its box, a hairline
/// round its dimmed word, so the row of buttons holds its shape as the
/// action comes and goes.
Widget dialogFilledAction(
  LumitTheme t, {
  required Key key,
  required String label,
  required VoidCallback? onPressed,
  bool autofocus = false,
}) =>
    _boxedWhenDead(
      t,
      dead: onPressed == null,
      HouseButton(
        key: key,
        primary: true,
        frameless: onPressed == null,
        autofocus: autofocus,
        padding: const EdgeInsets.symmetric(horizontal: 16),
        onPressed: onPressed,
        child: Text(label),
      ),
    );

/// A footer's outlined action, which keeps its box the same way: a footer
/// whose buttons all go dead for a moment, while something is fetched, does
/// not change shape as it does.
Widget dialogPlainAction(
  LumitTheme t, {
  required Key key,
  required String label,
  required VoidCallback? onPressed,
}) =>
    _boxedWhenDead(
      t,
      dead: onPressed == null,
      HouseButton(
        key: key,
        frameless: onPressed == null,
        padding: const EdgeInsets.symmetric(horizontal: 12),
        onPressed: onPressed,
        child: Text(label),
      ),
    );

Widget _boxedWhenDead(LumitTheme t, Widget button, {required bool dead}) =>
    dead
        ? DecoratedBox(
            position: DecorationPosition.foreground,
            decoration: BoxDecoration(
              border: Border.all(color: t.hairline),
              borderRadius: BorderRadius.circular(t.tokens.actionRadius),
            ),
            child: button,
          )
        : button;

/// A factual line — the footer's summary, a group's own reading. Mono, small,
/// quiet: it reports rather than labels.
TextStyle dialogMono(LumitTheme t) =>
    t.mono.copyWith(fontSize: dialogMonoSize, color: t.textMuted);

/// A thin thumb down the side of [child], which scrolls with [controller].
/// It is there for as long as there is more than fits, so a list that goes
/// on says so. It only shows where things are: a drag on a window moves the
/// window. [top] starts the thumb lower, under something held over the
/// list.
Widget dialogScroller(
        BuildContext context, LumitTheme t, ScrollController controller,
        {double top = 0, required Widget child}) =>
    RawScrollbar(
      controller: controller,
      thumbVisibility: true,
      interactive: false,
      thickness: 3,
      radius: const Radius.circular(2),
      thumbColor: t.surface4,
      padding: EdgeInsets.fromLTRB(2, top + 2, 2, 2),
      child: ScrollConfiguration(
        behavior: ScrollConfiguration.of(context).copyWith(scrollbars: false),
        child: child,
      ),
    );

/// A dropdown at the drawing's height, in the width the row gives it.
Widget dialogDropdown<T>(
  LumitTheme t, {
  required String id,
  required T value,
  required List<T> options,
  required String Function(T) label,
  required ValueChanged<T>? onChanged,
  double? width,

  /// A heading over an option, for a list that comes in named runs — the
  /// export's colour spaces, where the config's own names sit under one.
  String? Function(T)? group,

  /// Why an option cannot be chosen: disabled, never hidden.
  String? Function(T)? disabledReason,
}) =>
    SizedBox(
      width: width,
      height: dialogControlHeight,
      child: BareDropdown<T>(
        key: ValueKey<String>(id),
        value: value,
        options: options,
        label: label,
        onChanged: onChanged,
        group: group,
        disabledReason: disabledReason,
      ),
    );

/// Text that says the whole of itself in a tooltip when there was not the
/// room to show it all.
class FittedText extends StatelessWidget {
  final String text;
  final TextStyle style;

  /// How many lines it may run to before it is cut.
  final int lines;
  const FittedText(this.text, this.style, {super.key, this.lines = 1});

  @override
  Widget build(BuildContext context) => LayoutBuilder(
        builder: (context, box) {
          final line = Text(text,
              style: style, maxLines: lines, overflow: TextOverflow.ellipsis);
          final painter = TextPainter(
            text: TextSpan(text: text, style: style),
            maxLines: lines,
            textDirection: Directionality.of(context),
            textScaler: MediaQuery.textScalerOf(context),
          )..layout(maxWidth: box.maxWidth);
          final cut = painter.didExceedMaxLines;
          painter.dispose();
          // The tip wraps: a name of any length on one line would lie
          // across whatever is beside it.
          return cut
              ? LumitTooltip(message: text, maxWidth: 320, child: line)
              : line;
        },
      );
}

/// A short question with a button per answer, beside Cancel: the shape the
/// unsaved-changes window has. The last answer is the default, focused on
/// open. Cancel, Escape and the close mark answer null. [stacked] puts the
/// buttons in a column, for answers too long to sit in a row.
Future<T?> askLumitChoice<T>({
  required BuildContext context,
  required String title,
  required String question,
  required List<(String, T)> answers,
  bool stacked = false,
}) =>
    showLumitModal<T>(
      context: context,
      builder: (close) => Builder(builder: (context) {
        final t = ThemeScope.of(context).theme;
        return DialogFrame(
          width: 360,
          children: [
            dialogTitleBar(t,
                title: title, onClose: () => close(null), keyPrefix: 'choice'),
            Padding(
              padding: const EdgeInsets.all(dialogPadding),
              child: Text(question, style: t.body),
            ),
            dialogFooter(
              t,
              keyPrefix: 'choice',
              stacked: stacked,
              actions: [
                HouseButton(
                  key: const ValueKey('choice-cancel'),
                  onPressed: () => close(null),
                  child: Text(l10n.cancel),
                ),
                for (final (i, (label, value)) in answers.indexed)
                  HouseButton(
                    key: ValueKey('choice-$i'),
                    primary: i == answers.length - 1,
                    autofocus: i == answers.length - 1,
                    onPressed: () => close(value),
                    child: Text(label),
                  ),
              ],
            ),
          ],
        );
      }),
    );
