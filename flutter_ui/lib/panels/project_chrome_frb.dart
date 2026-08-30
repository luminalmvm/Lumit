// The Project panel's chrome — everything above and below the tree of items:
// the preview card, the search well and its colour chips, the column headings,
// the sideways scrollbar, and the bottom bar.
//
// **In plain terms**: the panel's furniture. None of it holds state of its
// own — each piece is handed what it draws and a callback for what a click
// means, so the panel stays the one place that knows what is selected, what is
// filtered and what is cached.
//
// The measurements here are the mockups' *computed* styles, as everywhere else
// in this panel: the browser's own resolved numbers rather than a reading of
// the CSS (K-450, K-454).

import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/drag_payloads.dart';
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'project_columns_frb.dart';

/// The preview card: 10px of padding round a 96×54 poster frame, with the
/// hairline under it counted in — 10 + 54 + 10 + 1.
const double projectPreviewHeight = 75;
const double _previewPad = 10;
const double _thumbWidth = 96;
const double _thumbHeight = 54;

/// The gap between the card's three text lines.
const double _previewLineGap = 3;

/// The search row: 8 above the well, 6 under it, and the well itself the 20 a
/// value well is everywhere (§12A.6).
const double projectSearchRowHeight = 34;
const double _searchPadTop = 8;
const double _searchPadBottom = 6;

/// The horizontal scrollbar strip under the tree: a 6px strip with a 4px track
/// inset 8px either side.
const double projectScrollStripHeight = 6;
const double _scrollTrackHeight = 4;
const double _scrollTrackInset = 8;

/// The bottom bar. **20, not the 18 a secondary row usually gets** — the
/// mockup renders it at 20 and K-454 makes the mockup's own breathing room the
/// default density, so §12A.6's table gained a project-panel line rather than
/// this bar being shaved to fit the old one.
const double projectFooterHeight = 20;
const double _footerPad = 10;
const double _footerGap = 12;
const double _footerIconGap = 5;

/// The bottom bar's two label trackings, in logical pixels at 9px type: the
/// new-item words sit at 0.08em, the count at 0.06em — both quieter than the
/// 0.12em a kicker carries, because neither is naming a container.
const double _footerLabelTracking = 0.72;
const double _footerCountTracking = 0.54;

/// The bottom bar's glyphs (K-456), drawn at the size its own mockup computed.
const double projectFooterIconSize = 13;

/// The colour-chip filter, now *inside* the search well at its left (K-634):
/// the mockup's six 6px dots, 3px apart. The strip's own standoff padding has
/// gone with the move — the well's own 6px inset stands the leading dot off
/// the edge, and `HouseTextField` puts the usual 5 between a leading mark and
/// the text after it.
const double _chipSize = 6;
const double _chipGap = 3;

/// The label chips the filter row offers, as palette indices, in the mockup's
/// own order — azure, mint, amber, violet, coral. The sixth chip is not a
/// colour: it is the neutral one that clears the filter, so the row can always
/// be got out of.
const List<int> projectFilterLabels = [1, 4, 2, 3, 8];

/// Where the preview card gives way (§12A.6's ladder, step 3). The two mockups
/// are the two ends of this ladder — the 360-wide artboard shows the card, the
/// 260-wide docked panel has already dropped it.
const double projectWidthForPreview = 340;

/// Where the bottom bar's words go (§12A.6's ladder, step 4: a toolbar sheds
/// rather than shrinks). **Two steps, because this bar carries three controls
/// where the mockup drew two.** Import's word goes first — it is the one the
/// mockup gives no place at all, so it is the one with least claim on the room
/// — and Folder's and Composition's go together below that, since those two
/// are the mockup's own and should read as a pair for as long as they fit.
const double _widthForImportLabel = 420;
const double _widthForFooterLabels = 380;

/// The handle between two column headings: it resizes the column to its left,
/// and everything else keeps its width (docs/07 §4.2's rule for the Timeline
/// outline, which this mirrors). It is drawn *inside* the gap the rows already
/// carry between their cells, so adding it moves no column.
///
/// **Every boundary is drawn; only some of them take hold.** The seam is the
/// Timeline header's own treatment — a 1×10 `hairline_strong` rule centred in
/// the gap — and it stands at every column boundary, because it is what says
/// where one column ends and the next begins. What varies is whether it
/// resizes: beside a fixed-width column (items, fps, path) it is a plain rule
/// with no drag and no resize cursor, and beside a column with a width of its
/// own it is a handle.
///
/// It used to draw nothing at all where it could not drag, which left the
/// `items|size` and `fps|path` boundaries unmarked while their neighbours were
/// ruled — a header that looked half-finished rather than one that told you
/// which seams move.
class _ColumnSeam extends StatelessWidget {
  final ValueChanged<double>? onResize;
  const _ColumnSeam({super.key, required this.onResize});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final resize = onResize;
    if (resize == null) {
      return SizedBox(
        width: projectRowGap,
        child: Center(
          child: Container(width: 1, height: 10, color: t.hairlineStrong),
        ),
      );
    }
    return MouseRegion(
      cursor: SystemMouseCursors.resizeColumn,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onHorizontalDragUpdate: (d) => resize(d.delta.dx),
        child: SizedBox(
          width: projectRowGap,
          child: Center(
            child: Container(width: 1, height: 10, color: t.hairlineStrong),
          ),
        ),
      ),
    );
  }
}

/// The column a seam belongs to: the one drawn immediately before [column].
ProjectColumn _leftOf(ProjectColumns cols, ProjectColumn column) {
  final order = cols.visible;
  return order[order.indexOf(column) - 1];
}

/// The column headings. Kicker words, and the values below them are laid out
/// by the same [ProjectColumns.cells] call, so they cannot come apart. The
/// gaps between them are the drag handles that resize the columns.
Widget projectColumnHeader(
  LumitTheme t,
  ProjectColumns cols, {
  required void Function(ProjectColumn column, double delta) onResize,
}) =>
    Container(
      key: const ValueKey('project-column-header'),
      height: projectColumnHeaderHeight(t),
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      padding: const EdgeInsets.only(
          left: projectHeaderPadLeft, right: projectRowPadding),
      child: Row(
        children: [
          // Name is the flexible slot, and what it is left with is
          // [projectNameColumn] — see [ProjectColumns.laidOutWidth].
          Expanded(child: Text(l10n.name.toUpperCase(), style: t.kicker)),
          ...cols.cells(
            seam: (before) => _ColumnSeam(
              key: ValueKey<String>(
                  'project-seam-${_leftOf(cols, before).name}'),
              // The seam resizes the column it follows, which is the one the
              // eye reads it as belonging to — and nothing at all beside a
              // column that has no width of its own to change.
              onResize: projectColumnIsFixedWidth(_leftOf(cols, before))
                  ? null
                  : (delta) => onResize(_leftOf(cols, before), delta),
            ),
            items: l10n.projectColumnItems.toUpperCase(),
            size: l10n.projectColumnSize.toUpperCase(),
            fps: l10n.unitFps.toUpperCase(),
            path: l10n.projectColumnPath.toUpperCase(),
            style: t.kicker,
            // The heading is as quiet as its column: Path is context, not a
            // fact about the item, and the mockup hushes both together.
            pathStyle: t.kicker.copyWith(color: t.textDisabled),
          ),
        ],
      ),
    );

/// The search well. Inset on `surface_0` like every other value well (§2.1),
/// inside the 8/6 the mockup pads the row with.
Widget projectSearchRow(
  LumitTheme t, {
  required TextEditingController controller,
  required FocusNode focus,
  required int? labelFilter,
  required ValueChanged<int?> onFilter,
}) =>
    SizedBox(
      key: const ValueKey('project-search-row'),
      height: projectSearchRowHeight,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(projectRowPadding, _searchPadTop,
            projectRowPadding, _searchPadBottom),
        child: SizedBox(
          height: wellHeight,
          child: HouseTextField(
            key: const ValueKey('project-search'),
            controller: controller,
            focusNode: focus,
            width: double.infinity,
            // The well fills its row rather than floating inside it:
            // the mockup renders it exactly 20 tall, and the default
            // 3px above and below would burst that.
            padding: const EdgeInsets.symmetric(horizontal: 6),
            // `surface2`, not the well's usual recess: this well has
            // its own row to itself over the panel's `surface1`, and
            // the mockup computes it a shade lighter rather than a
            // shade darker.
            fill: t.surface2,
            hint: l10n.searchProject,
            // **The swatch filter lives inside the well** (K-634), where a
            // search field's leading mark goes — one control that narrows
            // the list rather than two beside each other, and the well is
            // the row's full width again. It is the one leading in the app
            // that answers the pointer, because it is a control rather than
            // a sign saying what the field is for.
            leading: _labelChips(t, labelFilter, onFilter),
            leadingInteractive: true,
          ),
        ),
      ),
    );

/// The colour-swatch filter, inside the search well at its left (§12A.3a,
/// K-634). Five palette dots and a neutral one: tapping a colour narrows the
/// tree to the items wearing it — a folder's own tag included, so a colour
/// finds everything filed under a folder of that colour (K-567) — and tapping
/// the neutral chip, or the held colour again, shows everything.
///
/// The held chip is marked by a ring rather than by growing, so the row does
/// not change width as the filter is used (§12A.5: nothing changes the
/// resting state).
Widget _labelChips(
        LumitTheme t, int? labelFilter, ValueChanged<int?> onFilter) =>
    Row(
      key: const ValueKey('project-label-chips'),
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final (i, label) in [...projectFilterLabels, null].indexed)
          Padding(
            // Between the chips, not before the first one: the well's own
            // 6 of inset is what stands the leading dot off the edge.
            padding: EdgeInsets.only(left: i == 0 ? 0 : _chipGap),
            child: LumitTooltip(
              message: label == null
                  ? l10n.tipShowEverything
                  : l10n.tipFilterByLabel,
              child: GestureDetector(
                key: ValueKey<String>('project-label-chip-${label ?? 'none'}'),
                behavior: HitTestBehavior.opaque,
                onTap: () => onFilter(label == labelFilter ? null : label),
                child: Container(
                  width: _chipSize,
                  height: _chipSize,
                  decoration: BoxDecoration(
                    color: label == null ? t.surface4 : t.labelColour(label),
                    shape: BoxShape.circle,
                    border: labelFilter == label && label != null
                        ? Border.all(color: t.textPrimary)
                        : null,
                  ),
                ),
              ),
            ),
          ),
      ],
    );

/// The horizontal scrollbar under the tree: a 4px track inset 8 either side,
/// with a thumb as wide a share of it as the view is of the content. It is
/// full width — and so says nothing — until the width ladder's last step
/// actually bites.
Widget projectScrollStrip(LumitTheme t, ScrollController hScroll) => SizedBox(
      key: const ValueKey('project-scroll-strip'),
      height: projectScrollStripHeight,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: _scrollTrackInset),
        child: Align(
          alignment: Alignment.topCenter,
          child: Container(
            height: _scrollTrackHeight,
            decoration: BoxDecoration(
              color: t.surface2,
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            ),
            child: AnimatedBuilder(
              animation: hScroll,
              builder: (context, _) {
                // `maxScrollExtent` is only an answer once the list has been
                // laid out — on the very first build there is nothing to
                // measure yet, and the thumb simply fills its track.
                final position = hScroll.hasClients ? hScroll.position : null;
                final p = position != null && position.hasContentDimensions
                    ? position
                    : null;
                final extent = p == null || p.maxScrollExtent <= 0
                    ? 1.0
                    : p.viewportDimension /
                        (p.viewportDimension + p.maxScrollExtent);
                final at = p == null || p.maxScrollExtent <= 0
                    ? 0.0
                    : (p.pixels / p.maxScrollExtent).clamp(0.0, 1.0);
                return Align(
                  // -1 is hard left, 1 is hard right: the thumb travels the
                  // track's leftover room exactly as the view travels the
                  // content's.
                  alignment: Alignment(at * 2 - 1, 0),
                  child: FractionallySizedBox(
                    widthFactor: extent,
                    child: Container(
                      decoration: BoxDecoration(
                        color: t.surface4,
                        borderRadius:
                            BorderRadius.circular(t.tokens.controlRadius),
                      ),
                    ),
                  ),
                );
              },
            ),
          ),
        ),
      ),
    );

/// The bottom bar: the new-item controls at the left, the count at the right.
///
/// **Import lives here** although the mockup draws only Folder and
/// Composition — it is a command the panel has always carried and the mockup
/// gave it no other home, so it takes the mockup's own icon-and-kicker shape
/// rather than being dropped. The count's `n missing ·` half is the
/// "show only missing" filter, which the mockup likewise has no row for.
///
/// [hasProject] is what decides whether the proxies switch is drawn at all —
/// with no document open there is nothing project-wide to govern.
Widget projectFooter(
  LumitTheme t, {
  required int items,
  required int missing,
  required double width,
  required bool hasProject,
  required bool useProxies,
  required bool missingOnly,
  required VoidCallback onToggleMissing,
  required VoidCallback onImport,
  required VoidCallback onNewFolder,
  required void Function(List<FootageReference> footage) onNewComposition,
  required VoidCallback onToggleProxies,
}) {
  final active = missingOnly && missing > 0;
  final labels = width >= _widthForFooterLabels;
  final count = t.kicker.copyWith(letterSpacing: _footerCountTracking);
  return Container(
    key: const ValueKey('project-footer'),
    height: projectFooterHeight,
    color: t.surface2,
    padding: const EdgeInsets.symmetric(horizontal: _footerPad),
    child: Row(
      children: [
        LumitTooltip(
          message: l10n.importFootage,
          child: _footerAction(
            t,
            key: const ValueKey('project-import'),
            icon: LumitIcon.import,
            label:
                width >= _widthForImportLabel ? l10n.projectFooterImport : null,
            onPressed: onImport,
          ),
        ),
        const SizedBox(width: _footerGap),
        LumitTooltip(
          message: l10n.newFolder,
          child: _footerAction(
            t,
            key: const ValueKey('project-new-folder'),
            icon: LumitIcon.folder,
            label: labels ? l10n.projectFooterFolder : null,
            onPressed: onNewFolder,
          ),
        ),
        const SizedBox(width: _footerGap),
        // Footage dropped here makes a comp that matches it (docs/07 §3.1)
        // — the same dialog the button opens, with the media's own size,
        // rate and length already filled in, and every dropped item landing
        // in the finished comp as a layer.
        DragTarget<FootageDragData>(
          onAcceptWithDetails: (d) => onNewComposition(d.data.footage),
          builder: (context, candidate, _) => Container(
            foregroundDecoration: candidate.isEmpty
                ? null
                : BoxDecoration(border: Border.all(color: t.accent)),
            child: LumitTooltip(
              message: l10n.newComposition,
              child: _footerAction(
                t,
                key: const ValueKey('project-new-comp'),
                icon: LumitIcon.newComposition,
                label: labels ? l10n.projectFooterComposition : null,
                onPressed: () => onNewComposition(const []),
              ),
            ),
          ),
        ),
        // **The project-wide proxies switch** (K-501, docs/07 §3.3). It sits
        // on the bottom bar after a divider, apart from the new-item
        // controls, for the reason the Timeline's own comp-wide toggles do
        // (§12A.1): a switch that governs the whole document reads apart
        // from the commands that make things.
        //
        // Drawn like the controls beside it — the set's own mark with a
        // kicker word after it — so it sheds the word and keeps the mark as
        // the panel narrows (§12A.6's step 4), rather than vanishing at the
        // width where a word-only control would have to. Its two strengths
        // are the switch conventions': `text_primary` on, `text_muted` off,
        // and never the accent (§3.1's list is closed).
        if (hasProject) ...[
          Container(
              width: 1,
              height: 10,
              color: t.hairline,
              margin: const EdgeInsets.symmetric(horizontal: _footerIconGap)),
          Builder(builder: (context) {
            final ink = useProxies ? t.textPrimary : t.textMuted;
            return LumitTooltip(
              message:
                  useProxies ? l10n.tipUseProxiesOn : l10n.tipUseProxiesOff,
              child: GestureDetector(
                key: const ValueKey('project-use-proxies'),
                behavior: HitTestBehavior.opaque,
                onTap: onToggleProxies,
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    glyph.LumitIcon(LumitIcons.proxy,
                        size: projectFooterIconSize, colour: ink),
                    if (labels) ...[
                      const SizedBox(width: _footerIconGap),
                      Text(
                        l10n.projectFooterProxies.toUpperCase(),
                        style: t.kicker.copyWith(
                            letterSpacing: _footerLabelTracking, color: ink),
                      ),
                    ],
                  ],
                ),
              ),
            );
          }),
        ],
        const Spacer(),
        // The missing half reads *before* the total, so the bar ends on the
        // item count (the owner's order). The two stay separate strings: a
        // translator sees each phrase whole, and the order is the layout's
        // business rather than something spliced into a sentence.
        // Flexible like the total beside it: both halves are the bar's
        // truncating text (§12A.6's step 1), and the missing half used to be
        // fixed — which meant a narrow bar carrying a broken item overflowed
        // rather than shortening.
        if (missing > 0)
          Flexible(
              child: LumitTooltip(
            message: active ? l10n.tipShowEverything : l10n.tipMissingOnly,
            child: GestureDetector(
              key: const ValueKey('missing-toggle'),
              behavior: HitTestBehavior.opaque,
              onTap: onToggleMissing,
              child: Text(
                '${l10n.projectMissingCount(missing)} · ',
                style: count.copyWith(color: active ? t.warning : t.textMuted),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          )),
        // The count never pushes the bar wider than the panel: it is the
        // flexible text the width ladder's first step truncates (§12A.6).
        Flexible(
          child: Text(l10n.projectItemCount(items),
              style: count, maxLines: 1, overflow: TextOverflow.ellipsis),
        ),
      ],
    ),
  );
}

Widget _footerAction(
  LumitTheme t, {
  required Key key,
  required LumitIcon icon,
  // Null once the bar is too narrow to spell the word out: §12A.6's ladder
  // step 4 says a toolbar sheds rather than shrinks, and an icon that is
  // already the mockup's own is what a shed word leaves behind.
  required String? label,
  required VoidCallback onPressed,
}) =>
    GestureDetector(
      key: key,
      behavior: HitTestBehavior.opaque,
      onTap: onPressed,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          lumitIcon(icon, size: projectFooterIconSize, color: t.textMuted),
          if (label != null) ...[
            const SizedBox(width: _footerIconGap),
            Text(
              label.toUpperCase(),
              style: t.kicker.copyWith(letterSpacing: _footerLabelTracking),
            ),
          ],
        ],
      ),
    );

/// The picked item's readout (docs/07 §3.1): poster frame, name, and the
/// item's own vital statistics. Always present at a fixed height, so the tree
/// below never jumps when the selection changes; with nothing picked it is
/// simply quiet.
Widget projectPreviewCard(
  LumitTheme t, {
  required ItemReference? item,

  /// The item's name, read by the panel's own walk — which is where the
  /// calm-on-a-deleted-item guard lives, so the card never reads it again.
  required String name,
  required bool missing,
  required ui.Image? thumb,
  required BridgeMediaInfo? info,
}) =>
    Container(
      key: const ValueKey('project-preview-card'),
      height: projectPreviewHeight,
      decoration: BoxDecoration(
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      padding: const EdgeInsets.all(_previewPad),
      child: item == null
          ? const SizedBox.expand()
          : _previewContent(t, item, name, missing, thumb, info),
    );

Widget _previewContent(
  LumitTheme t,
  ItemReference item,
  String name,
  bool missing,
  ui.Image? image,
  BridgeMediaInfo? info,
) {
  final type = switch (item) {
    ItemReference_Footage() => l10n.projectTypeFootage,
    ItemReference_Folder() => l10n.projectTypeFolder,
    ItemReference_Composition() => l10n.projectTypeComposition,
    ItemReference_Solid() => l10n.projectTypeSolid,
  };

  Widget? thumb;
  if (item case ItemReference_Footage() when !missing) {
    // The picture comes straight from the RAM cache the walk prefilled, so
    // switching the selection redraws it in the same frame.
    thumb = SizedBox(
      width: _thumbWidth,
      height: _thumbHeight,
      child: image == null
          ? Center(
              child: lumitIcon(LumitIcon.footage,
                  size: iconSize, color: t.textMuted))
          : ClipRRect(
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
              child: Container(
                color: t.surface0,
                child: RawImage(image: image, fit: BoxFit.contain),
              ),
            ),
    );
  }

  return Row(
    key: const ValueKey('project-info-header'),
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      if (thumb != null) ...[thumb, const SizedBox(width: _previewPad)],
      Expanded(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(name, style: t.bodyPrimary, overflow: TextOverflow.ellipsis),
            const SizedBox(height: _previewLineGap),
            _previewFacts(t, item, missing, info),
            const SizedBox(height: _previewLineGap),
            // The card's second fact line: the mockup's `H.264 · 48 kHz
            // stereo`. Codec names are the file's own words, not ours, so
            // they are printed as the container declares them. With nothing
            // to say — a folder, a solid, a file that will not probe — it
            // falls back to the kind of thing this is, which is what the
            // card said before the codec crossed.
            Text(
              _previewCodecs(item, missing, info) ?? type,
              key: const ValueKey('project-info-codec'),
              style: projectMetaStyle(t),
              overflow: TextOverflow.ellipsis,
            ),
          ],
        ),
      ),
    ],
  );
}

/// The card's first fact line: what this item can truthfully state. The
/// length reads as `HH:MM:SS:FF` timecode at the item's own rate — the same
/// clock face the Viewer shows — never as a bare frame count.
Widget _previewFacts(
    LumitTheme t, ItemReference item, bool missing, BridgeMediaInfo? info) {
  String? line;
  switch (item) {
    case ItemReference_Footage():
      if (missing) {
        return Text(l10n.projectItemMissing,
            style: projectMetaStyle(t).copyWith(color: t.warning));
      }
      if (info != null) {
        final fps = info.fpsDen == 0 ? 0.0 : info.fpsNum / info.fpsDen;
        final seconds = info.duration.den == 0
            ? 0.0
            : info.duration.num / info.duration.den;
        final frames = (seconds * fps).round();
        final size = '${info.width}×${info.height}';
        line = switch (info) {
          // No picture at all — the probe's answer, not an inference from a
          // width of zero. Sound has no frames worth counting, so its last
          // field is milliseconds rather than a frame number.
          _ when info.videoCodec == null =>
            '${l10n.projectInfoAudio} · ${timecodeOfSecondsMs(seconds)}',
          // A picture that does not run has no rate and no length: the two
          // numbers a still would print are one frame and one frame's worth
          // of time, which say nothing and imply motion.
          _ when info.isStill => '$size · ${l10n.projectInfoStill}',
          _ => '$size · ${fps.toStringAsFixed(2)} ${l10n.unitFps}'
              ' · ${timecodeOfRate(frames, info.fpsNum, info.fpsDen)}',
        };
      }
    case ItemReference_Composition(:final field0):
      final s = field0.getSettings();
      final fps = s.fpsDen == 0 ? 0.0 : s.fpsNum / s.fpsDen;
      final frames = field0.durationFrames();
      line = '${s.width}×${s.height} · '
          '${fps.toStringAsFixed(2)} ${l10n.unitFps}'
          ' · ${timecodeOfRate(frames, s.fpsNum, s.fpsDen)}';
    case ItemReference_Folder(:final field0):
      line = l10n.projectItemCount(field0.getChildren().length);
    case ItemReference_Solid():
      break;
  }
  if (line == null) return const SizedBox.shrink();
  return Text(line,
      key: const ValueKey('project-info-line'),
      style: projectMetaStyle(t),
      overflow: TextOverflow.ellipsis);
}

/// The card's second fact line for footage: what the container is made of.
/// `null` when there is nothing truthful to say, which is every other kind
/// of item and any file that has not probed.
String? _previewCodecs(
    ItemReference item, bool missing, BridgeMediaInfo? info) {
  if (item is! ItemReference_Footage || missing) return null;
  if (info == null) return null;
  final parts = [
    if (info.videoCodec != null) info.videoCodec!,
    if (info.audioCodec != null && info.sampleRate > 0)
      '${projectSampleRateText(info.sampleRate)} '
          '${projectChannelText(info.channels)}',
  ];
  return parts.isEmpty ? null : parts.join(' · ');
}
