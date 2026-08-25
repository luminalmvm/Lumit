// The shape every popup shares: a title strip, an optional row of page tabs,
// label-left rows inside titled groups, and a footer carrying a factual line
// and the single filled action (§12A.4, K-444).
//
// **In plain terms.** Two dialogs are drawn in the approved mockups — New
// composition and Export — and they are drawn as the same object at two sizes.
// This file is that object: the pieces below are what both are assembled from,
// so a change to the pattern is one edit rather than two that drift.
//
// A dialog is built **in the window**, as an ordinary framework overlay. That
// is not a stopgap: when Flutter's windowing reaches the stable channel a
// `showDialog` becomes a real child window with no rewrite here (K-449).
//
// The measurements are the drawings' own, read off their computed styles and
// pinned by `export_metrics_test` / `comp_settings_metrics_test`. A value that
// disagrees with the drawing is a defect (§12A.6).

import 'package:flutter/widgets.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

/// The title strip, over its hairline.
const double dialogTitleStrip = 30;

/// The close mark in it, at the size the drawings render it (K-456).
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
/// footer (K-488). Three buttons come to 10 above, 24 · 8 · 24 · 8 · 24, and
/// 10 below.
const double dialogFooterStackGap = 8;
const double dialogFooterPad = 10;

/// The inset the title strip, the footer and a body take from the dialog's
/// edges.
const double dialogPadding = 14;

/// A dropdown's closed face, a value well, and a plain button inside the body:
/// 22, the one height §12A.6 gives them in a dialog.
const double dialogControlHeight = 22;

/// A titled group: a hairline box with its kicker notched into the top edge,
/// and the air between two of them.
const double dialogGroupGap = 10;
const double dialogGroupRadius = 2;

/// The factual mono lines — the footer's summary, a group's own reading.
const double dialogMonoSize = 10;

/// The frame itself: the drawing's `surface_1` out to a hairline edge.
///
/// The edge is a foreground decoration so it is painted *over* the outermost
/// pixel rather than insetting everything by one — the drawing's 640 is the
/// room inside the frame, not the room inside the frame less its border.
class DialogFrame extends StatelessWidget {
  final double width;
  final List<Widget> children;

  const DialogFrame({super.key, required this.width, required this.children});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return SizedBox(
      width: width,
      child: Container(
        decoration: BoxDecoration(
          color: t.surface1,
          borderRadius: BorderRadius.circular(t.tokens.floatRadius),
          boxShadow: t.floatShadow,
        ),
        foregroundDecoration: BoxDecoration(
          borderRadius: BorderRadius.circular(t.tokens.floatRadius),
          border: Border.all(color: t.hairline),
        ),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(t.tokens.floatRadius),
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
Widget dialogTitleBar(
  LumitTheme t, {
  required String title,
  String subject = '',
  required VoidCallback onClose,
  required String keyPrefix,
}) =>
    Container(
      key: ValueKey<String>('$keyPrefix-title-strip'),
      height: dialogTitleStrip + 1,
      decoration: BoxDecoration(
        color: t.surface2,
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      // 14 either side, as the drawing computes it — the mark's own inset from
      // the corner is the strip's, not a smaller one of its own.
      padding: const EdgeInsets.symmetric(horizontal: dialogPadding),
      child: Row(
        children: [
          Text(title.toUpperCase(), style: t.kickerOn),
          Expanded(
            child: Padding(
              padding: const EdgeInsets.only(left: 10),
              child: Align(
                alignment: Alignment.centerLeft,
                child: Text(
                  subject,
                  style: t.body.copyWith(color: t.textMuted),
                  overflow: TextOverflow.ellipsis,
                ),
              ),
            ),
          ),
          LumitTooltip(
            message: l10n.close,
            child: GestureDetector(
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
                    colour: t.textMuted,
                    semanticLabel: l10n.close,
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );

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
            GestureDetector(
              key: ValueKey<String>('$keyPrefix-tab-$page'),
              behavior: HitTestBehavior.opaque,
              onTap: () => onPick(page),
              child: Container(
                margin: const EdgeInsets.only(right: dialogTabGap),
                padding: const EdgeInsets.fromLTRB(6, 6, 6, 5),
                decoration: BoxDecoration(
                  border: Border(
                    bottom: BorderSide(
                      // Transparent when it is not the page in force: the
                      // rule is always there, so nothing shifts by a pixel as
                      // the tab changes (§7.1).
                      color:
                          page == current ? t.accent : const Color(0x00000000),
                    ),
                  ),
                ),
                child: Text(
                  label.toUpperCase(),
                  style: page == current ? t.kickerOn : t.kicker,
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
/// page lived in wants them separated (K-458: each drawing decides its own).
/// [highlighted] lights the box's edge for a moment — what a section tab does
/// to the section it jumps to on a dialog that scrolls rather than paging
/// (K-485). The accent is doing here exactly what it does in the tab strip:
/// saying *this is the one you asked for*, and only while you are asking.
Widget dialogGroup(
  LumitTheme t,
  String title,
  List<Widget> rows, {
  Key? key,
  bool highlighted = false,
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
              borderRadius: BorderRadius.circular(dialogGroupRadius),
            ),
            padding: const EdgeInsets.fromLTRB(12, 8, 12, 6),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: rows,
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
              child: Text(title.toUpperCase(),
                  style: t.kicker.copyWith(color: t.textSecondary)),
            ),
          ),
        ],
      ),
    );

/// One row: its name on the left in a fixed column, its control beside it.
///
/// [labelColumn] and [gap] are the drawing's, and they differ between dialogs
/// — Export computes 100 and 10 in a 640px frame, New composition 110 and 12
/// in a 520px one. Neither is "the" dialog row; each drawing measures its own
/// (K-458), which is why this takes them rather than fixing one.
Widget dialogRow(
  LumitTheme t,
  String label,
  Widget control, {
  required double labelColumn,
  double gap = 10,
  double minHeight = 28,
  Key? key,
}) =>
    ConstrainedBox(
      key: key,
      constraints: BoxConstraints(minHeight: minHeight),
      child: Row(
        children: [
          SizedBox(width: labelColumn, child: Text(label, style: t.body)),
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
/// for a dialog too narrow to seat them in a line — §12A.6's ladder step 2
/// (K-488). The order is unchanged, so the filled action is still last, which
/// in a column means the bottom.
Widget dialogFooter(
  LumitTheme t, {
  String summary = '',
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
                if (summary.isNotEmpty)
                  Flexible(
                    child: Text(
                      summary,
                      key: ValueKey<String>('$keyPrefix-summary'),
                      style: dialogMono(t),
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                const Spacer(),
                for (final action in actions) ...[
                  const SizedBox(width: dialogFooterGap),
                  SizedBox(height: dialogFooterButton, child: action),
                ],
              ],
            ),
    );

/// A factual line — the footer's summary, a group's own reading. Mono, small,
/// quiet: it reports rather than labels.
TextStyle dialogMono(LumitTheme t) =>
    t.mono.copyWith(fontSize: dialogMonoSize, color: t.textMuted);

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

  /// Why an option cannot be chosen (K-485: disabled, never hidden).
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
