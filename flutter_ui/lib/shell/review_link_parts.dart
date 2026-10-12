// The pieces the Previews window is put together from: a link's still, a
// comment's row, a like's line, and the page of a link's own settings,
// viewers and versions.
//
// They draw what they are handed and say what was pressed. What is asked of
// the server stays with the window and [ReviewsState], but for who has
// opened a link, which the page that shows it asks for as it opens.

import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/services.dart' show Clipboard, ClipboardData;
import 'package:flutter/widgets.dart';
import 'package:intl/intl.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../state/external_links.dart';
import '../state/reviews.dart';
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'pro_window.dart';
import 'review_send.dart';

/// How long ago a moment was, in the words the page in the browser uses,
/// so a reviewer and an editor say the same thing: just now, then minutes,
/// then hours for a day, then the date, with its year when that is not this
/// one. Never a clock time by itself, which beside a video reads as a
/// length.
String reviewWhen(int seconds) {
  if (seconds <= 0) return '';
  final at = DateTime.fromMillisecondsSinceEpoch(seconds * 1000);
  final now = DateTime.now();
  final gone = now.difference(at);
  if (gone.inMinutes < 1) return l10n.reviewJustNow;
  if (gone.inHours < 1) return l10n.reviewMinutesAgo(gone.inMinutes);
  if (gone.inHours < 24) return l10n.reviewHoursAgo(gone.inHours);
  DateFormat as([String? tag]) =>
      at.year == now.year ? DateFormat.MMMd(tag) : DateFormat.yMMMd(tag);
  try {
    // The English here is British, which the bare tag does not say.
    final tag = l10n.localeName;
    return as(tag == 'en' ? 'en_GB' : tag).format(at);
  } catch (_) {
    return as().format(at);
  }
}

/// What a version is, in a line: its size, how long it runs, what it weighs.
String reviewFacts(ReviewVersion version) =>
    '${version.width} × ${version.height}'
    ' · ${timecodeOfRate(version.frames, version.fpsNum, version.fpsDen)}'
    ' · ${writtenBytes(version.size)}';

/// A small boxed word in mono: a version's name.
Widget reviewTag(LumitTheme t, String word) => Container(
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        border: Border.all(color: t.hairlineStrong),
      ),
      child: Text(word,
          style: t.mono.copyWith(fontSize: 9, height: 1.2, color: t.textMuted)),
    );

/// A commenter's dot, in the colour their markers take on the Timeline.
Widget reviewDot(LumitTheme t, String name) => Container(
      width: 6,
      height: 6,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: t.labelColour(commenterColour(name)),
      ),
    );

/// A timecode as a chip, pressed to put the playhead there. [until] is
/// where a range runs to. With no [onJump] it is only read.
Widget _timecode(LumitTheme t, String id, String timecode,
        {String? until, VoidCallback? onJump}) =>
    MouseRegion(
      cursor: onJump == null ? MouseCursor.defer : SystemMouseCursors.click,
      child: GestureDetector(
        key: ValueKey<String>('review-jump-$id'),
        behavior: HitTestBehavior.opaque,
        onTap: onJump,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 2),
          decoration: BoxDecoration(
            color: t.surface2,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(color: t.hairline),
          ),
          child: Text(
            // A range is said as the page in the browser says it, so a
            // reviewer's "the note from 13:10 to 15:20" can be found.
            until == null ? timecode : l10n.reviewRange(timecode, until),
            style: t.mono.copyWith(
                fontSize: 10,
                height: 1.2,
                color: onJump == null ? t.textMuted : t.textPrimary),
          ),
        ),
      ),
    );

/// A small frameless word that does something, for a row's own actions.
Widget reviewAction(
        LumitTheme t, String id, Widget child, VoidCallback onPressed) =>
    HouseButton(
      key: ValueKey<String>('review-$id'),
      small: true,
      frameless: true,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      onPressed: onPressed,
      child: child,
    );

/// How wide a row's menu is. It is a set width so that the menu can be put
/// where the whole of it fits.
const double _menuWidth = 200;

/// One thing in a row's menu: the end of its key, the words, and what they
/// do. With nothing to do it is listed and dimmed. Null in a menu's list is
/// a rule between two runs of them.
typedef ReviewMenuItem = (String, String, VoidCallback?);

/// A row's own menu, for a press at [at] on the screen inside [row]. It
/// opens under the press and towards the middle of the row, so one asked for
/// at the row's far end stays inside the window.
void reviewRowMenu(BuildContext row, Offset at, List<ReviewMenuItem?> items) {
  final box = row.findRenderObject()! as RenderBox;
  final press = box.globalToLocal(at);
  final room = box.size.width - _menuWidth;
  final left = press.dx <= room
      ? press.dx
      : press.dx >= _menuWidth
          ? press.dx - _menuWidth
          : room.clamp(0.0, press.dx);
  final t = ThemeScope.of(row).theme;
  unawaited(showMenuAt<void>(
    context: row,
    position: box.localToGlobal(Offset(left, press.dy)),
    width: _menuWidth,
    rows: (close) => [
      for (final item in items)
        if (item case (final id, final words, final run)?)
          MenuRow(
            key: ValueKey<String>('review-menu-$id'),
            onPressed: () {
              close(null);
              run?.call();
            },
            child: Text(words,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: run == null
                    ? t.body.copyWith(color: t.textDisabled)
                    : null),
          )
        else
          Container(
            height: 1,
            margin: const EdgeInsets.symmetric(vertical: 4),
            color: t.hairline,
          ),
    ],
  ));
}

/// The three dots at the end of a row, which open its menu under themselves.
Widget _more(LumitTheme t, String id, void Function(Offset at) menu) =>
    Builder(
      builder: (context) => LumitTooltip(
        message: l10n.reviewMore,
        child: reviewAction(
          t,
          'more-$id',
          glyph.LumitIcon(LumitIcons.more,
              size: 12,
              colour: t.textSecondary,
              semanticLabel: l10n.reviewMore),
          () {
            final box = context.findRenderObject()! as RenderBox;
            menu(box.localToGlobal(
                Offset(box.size.width, box.size.height + 2)));
          },
        ),
      ),
    );

/// The mark of the row the arrow keys are on: the accent edge the chosen
/// link has, standing in the margin beside it.
Widget _chosenMark(LumitTheme t, bool chosen, Widget row) => !chosen
    ? row
    : Stack(
        clipBehavior: Clip.none,
        children: [
          row,
          Positioned(
            left: -10,
            top: 5,
            bottom: 7,
            width: 2,
            child: ColoredBox(color: t.accent),
          ),
        ],
      );

/// A column that scrolls under the house thumb. An edge with more past it
/// fades into [ground], so a row cut by it reads as more to come and not as
/// a fault.
class ReviewScroll extends StatefulWidget {
  final ScrollController controller;
  final Color ground;
  final Widget child;

  const ReviewScroll({
    super.key,
    required this.controller,
    required this.ground,
    required this.child,
  });

  @override
  State<ReviewScroll> createState() => _ReviewScrollState();
}

class _ReviewScrollState extends State<ReviewScroll> {
  bool _above = false, _below = false;

  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_read);
  }

  @override
  void didUpdateWidget(ReviewScroll old) {
    super.didUpdateWidget(old);
    if (old.controller == widget.controller) return;
    old.controller.removeListener(_read);
    widget.controller.addListener(_read);
  }

  @override
  void dispose() {
    widget.controller.removeListener(_read);
    super.dispose();
  }

  /// Which edges have more past them, as the column is laid out now.
  void _read() {
    if (!mounted || !widget.controller.hasClients) return;
    final at = widget.controller.position;
    final above = at.pixels > 0.5;
    final below = at.pixels < at.maxScrollExtent - 0.5;
    if (above == _above && below == _below) return;
    setState(() {
      _above = above;
      _below = below;
    });
  }

  Widget _fade({required bool top}) => Positioned(
        left: 0,
        // Clear of the thumb, which is what says how much more there is.
        right: 8,
        top: top ? 0 : null,
        bottom: top ? null : 0,
        height: 16,
        child: IgnorePointer(
          child: DecoratedBox(
            decoration: BoxDecoration(
              gradient: LinearGradient(
                begin: top ? Alignment.topCenter : Alignment.bottomCenter,
                end: top ? Alignment.bottomCenter : Alignment.topCenter,
                colors: [widget.ground, widget.ground.withValues(alpha: 0)],
              ),
            ),
          ),
        ),
      );

  @override
  Widget build(BuildContext context) {
    // Whether it scrolls is only known once it is laid out.
    WidgetsBinding.instance.addPostFrameCallback((_) => _read());
    return Stack(
      children: [
        dialogScroller(
          context,
          ThemeScope.of(context).theme,
          widget.controller,
          child: SingleChildScrollView(
              controller: widget.controller, child: widget.child),
        ),
        if (_above) _fade(top: true),
        if (_below) _fade(top: false),
      ],
    );
  }
}

/// Who has approved a version, in a line: one name, two, or one and how
/// many more. Never asked about an empty list.
String reviewApprovedBy(List<ReviewApproval> all) => switch (all.length) {
      1 => l10n.reviewApprovedBy(all.first.name),
      2 => l10n.reviewApprovedByTwo(all.first.name, all.last.name),
      _ => l10n.reviewApprovedByMore(all.first.name, all.length - 1),
    };

/// A version's still, filling the box it is given. Until it has loaded, and
/// for a version that has none, the box holds a soft wash instead of a hole.
class ReviewPoster extends StatelessWidget {
  final Review review;
  final ReviewVersion version;
  const ReviewPoster(this.review, this.version, {super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final empty = CustomPaint(painter: ProWashPainter(t.surface2, 0.2));
    if (!version.poster) return empty;
    return Image.network(
      review.posterUrl(version),
      // With its maker's key, so a link with a password still shows them the
      // still.
      headers: {'X-Lumit-Owner': review.ownerKey},
      fit: BoxFit.cover,
      gaplessPlayback: true,
      errorBuilder: (_, __, ___) => empty,
      frameBuilder: (_, child, frame, __) => frame == null ? empty : child,
    );
  }
}

/// The picture a comment came with: small on the comment's row, or [whole],
/// as large as it is where the app's window has the room. Until it has
/// loaded the box holds a soft wash, as a still's does.
class ReviewPicture extends StatelessWidget {
  final Review review;
  final ReviewComment comment;
  final bool whole;
  final VoidCallback? onPressed;

  const ReviewPicture(this.review, this.comment,
      {super.key, this.whole = false, this.onPressed});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final (:w, :h) = comment.picture!;
    final window = MediaQuery.sizeOf(context);
    final (wide, tall) =
        whole ? (window.width - 96, window.height - 96) : (160.0, 72.0);
    // Never drawn larger than it is.
    final scale = math.min(1.0, math.min(wide / w, tall / h));
    final empty = CustomPaint(painter: ProWashPainter(t.surface2, 0.2));
    final radius = BorderRadius.circular(t.tokens.controlRadius);
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        key: ValueKey<String>('review-picture-${comment.id}'),
        onTap: onPressed,
        child: Container(
          width: w * scale,
          height: h * scale,
          clipBehavior: Clip.antiAlias,
          // A ground of its own, for a picture that is partly clear, and an
          // edge, for one as dark as what it lies on.
          decoration: BoxDecoration(color: t.surface2, borderRadius: radius),
          foregroundDecoration: BoxDecoration(
            borderRadius: radius,
            border: Border.all(color: t.hairline),
          ),
          child: Image.network(
            review.pictureUrl(comment),
            // With its maker's key, as the still is asked for.
            headers: {'X-Lumit-Owner': review.ownerKey},
            // The server keeps the one file, so on a row it is read no
            // larger than it is drawn. Whole, it is read as it is, and
            // whoever shows it lets it go again.
            // ponytail: every picture on the listed comments is asked for
            // at once, up to the 200 a link may hold. Ask as each comes
            // into view if links grow that many.
            cacheWidth: whole
                ? null
                : (w * scale * MediaQuery.devicePixelRatioOf(context)).ceil(),
            fit: BoxFit.fill,
            filterQuality: FilterQuality.medium,
            gaplessPlayback: true,
            semanticLabel: l10n.reviewShowPicture,
            errorBuilder: (_, __, ___) => empty,
            frameBuilder: (_, child, frame, __) =>
                frame == null ? empty : child,
          ),
        ),
      ),
    );
  }
}

/// How tall a link's row in the list stands.
const double reviewLinkRowHeight = 56;

/// One link in the window's list: its still, its title and how long ago
/// something was last said on it, and under those what state it is in.
class ReviewLinkRow extends StatelessWidget {
  final Review link;

  /// Whether it is the one on the right, whether it has comments nobody has
  /// looked at, and whether its composition has been edited since it went
  /// out.
  final bool chosen, unseen, moved;
  final VoidCallback onPick;

  /// Asked for its menu, with where on the screen the press was.
  final ValueChanged<Offset> onMenu;

  const ReviewLinkRow({
    super.key,
    required this.link,
    required this.chosen,
    required this.unseen,
    required this.moved,
    required this.onPick,
    required this.onMenu,
  });

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final newest = link.versions.last;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onPick,
        onSecondaryTapDown: (press) => onMenu(press.globalPosition),
        // The same mark the Settings window gives the page in force: an
        // accent edge, and the padding gives up what the edge takes.
        child: AnimatedContainer(
          duration: scope.motion.mark.duration,
          height: reviewLinkRowHeight,
          padding: EdgeInsets.only(left: chosen ? 10 : 12, right: 12),
          decoration: BoxDecoration(
            color: chosen ? t.surface2 : null,
            border: chosen
                ? Border(left: BorderSide(color: t.accent, width: 2))
                : null,
          ),
          child: Row(
            children: [
              ClipRRect(
                borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                child: SizedBox(
                    width: 64, height: 36, child: ReviewPoster(link, newest)),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Column(
                  mainAxisAlignment: MainAxisAlignment.center,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    // What it is, and when something was last said on it.
                    Row(
                      children: [
                        Expanded(
                          child: FittedText(
                              link.title, chosen ? t.bodyPrimary : t.body),
                        ),
                        if (unseen)
                          Container(
                            key: ValueKey<String>('review-unseen-${link.id}'),
                            width: 6,
                            height: 6,
                            margin: const EdgeInsets.only(left: 8),
                            decoration: BoxDecoration(
                                shape: BoxShape.circle, color: t.textPrimary),
                          ),
                        const SizedBox(width: 8),
                        Text(
                            reviewWhen(link.lastSaid > 0
                                ? link.lastSaid
                                : link.updated),
                            style: t.small),
                      ],
                    ),
                    const SizedBox(height: 4),
                    // The state it is in, on one line. In a language whose
                    // words are longer, what does not fit goes whole and
                    // the last of it first: it wraps onto a line that is
                    // not shown.
                    SizedBox(
                      height: 16,
                      child: Wrap(
                        clipBehavior: Clip.hardEdge,
                        spacing: 8,
                        runSpacing: 16,
                        crossAxisAlignment: WrapCrossAlignment.center,
                        children: [
                          if (link.versions.length > 1)
                            reviewTag(t, l10n.reviewVersion(newest.n)),
                          if (link.openCount > 0)
                            Text(l10n.reviewOpenCount(link.openCount),
                                style:
                                    t.small.copyWith(color: t.textSecondary)),
                          // A reviewer has said yes to the newest version.
                          if (newest.approvals.isNotEmpty)
                            Row(
                              key: ValueKey<String>(
                                  'review-approved-${link.id}'),
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                glyph.LumitIcon(LumitIcons.tick,
                                    size: 11, colour: t.success),
                                const SizedBox(width: 3),
                                Text(l10n.reviewApproved,
                                    style: t.small
                                        .copyWith(color: t.textSecondary)),
                              ],
                            ),
                          if (moved)
                            Text(l10n.reviewChanged,
                                key: ValueKey<String>(
                                    'review-changed-${link.id}'),
                                style: t.small),
                        ],
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The chosen link's still, wide, falling away into the window's own surface
/// under its title and what the version is, with the way to another version
/// once there is one.
class ReviewStill extends StatelessWidget {
  final Review review;
  final ReviewVersion version;
  final ValueChanged<int> onVersion;

  const ReviewStill({
    super.key,
    required this.review,
    required this.version,
    required this.onVersion,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // As tall as the title and the line under it and no taller. The still
    // gives them its colour to stand on and has no room of its own: with
    // nothing in it but a tint, room over the title reads as a gap.
    return ClipRect(
      child: Stack(
        children: [
          // Only its colour, as the page has behind its picture. A strip
          // cut across a frame shows whatever lay there, and on a sparse
          // frame that looks like a fault.
          Positioned.fill(
            child: ImageFiltered(
              imageFilter: ui.ImageFilter.blur(
                  sigmaX: 36, sigmaY: 36, tileMode: TileMode.clamp),
              child: ReviewPoster(review, version),
            ),
          ),
          Positioned.fill(
            child: DecoratedBox(
              decoration: BoxDecoration(
                gradient: LinearGradient(
                  begin: Alignment.topCenter,
                  end: Alignment.bottomCenter,
                  stops: const [0, 1],
                  colors: [t.surface1.withValues(alpha: 0.6), t.surface1],
                ),
              ),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 16, 16, 2),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: [
                Expanded(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      FittedText(
                          key: const ValueKey('review-heading'),
                          review.title,
                          t.heading.copyWith(height: 1.2),
                          lines: 2),
                      const SizedBox(height: 4),
                      Text(reviewFacts(version),
                          style:
                              dialogMono(t).copyWith(color: t.textSecondary)),
                    ],
                  ),
                ),
                if (review.versions.length > 1) ...[
                  const SizedBox(width: 12),
                  dialogDropdown<int>(
                    t,
                    id: 'review-version',
                    width: 64,
                    value: version.n,
                    options: [for (final v in review.versions.reversed) v.n],
                    label: l10n.reviewVersion,
                    onChanged: onVersion,
                  ),
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// A link's address, with the way to copy it and the way to open it.
class ReviewAddress extends StatefulWidget {
  final Review review;
  const ReviewAddress({super.key, required this.review});

  @override
  State<ReviewAddress> createState() => _ReviewAddressState();
}

class _ReviewAddressState extends State<ReviewAddress> {
  bool _copied = false;
  Timer? _copiedFor;

  @override
  void dispose() {
    _copiedFor?.cancel();
    super.dispose();
  }

  void _copy() {
    unawaited(Clipboard.setData(ClipboardData(text: widget.review.url)));
    _copiedFor?.cancel();
    _copiedFor = Timer(const Duration(milliseconds: 1400), () {
      if (mounted) setState(() => _copied = false);
    });
    setState(() => _copied = true);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    Widget action(String id, VoidCallback onPressed, Widget child) => Padding(
          padding: const EdgeInsets.only(left: 8),
          child: SizedBox(
            height: 26,
            child: HouseButton(
              key: ValueKey<String>('review-$id'),
              padding: const EdgeInsets.symmetric(horizontal: 12),
              onPressed: onPressed,
              child: child,
            ),
          ),
        );
    return Row(
      children: [
        Expanded(child: reviewLinkWell(t, widget.review.url)),
        action(
          'copy',
          _copy,
          // The word keeps its room while the tick stands in for it, so the
          // row does not shift.
          Stack(
            alignment: Alignment.center,
            children: [
              Opacity(opacity: _copied ? 0 : 1, child: Text(l10n.reviewCopy)),
              if (_copied)
                glyph.LumitIcon(LumitIcons.tick, size: 14, colour: t.success),
            ],
          ),
        ),
        // As whoever made it, which is what the key after the address says.
        action(
            'open',
            () => unawaited(openExternalLink(widget.review.ownerUrl)),
            Text(l10n.reviewOpen)),
      ],
    );
  }
}

/// One comment: who, where on the composition and on which layer, and what
/// they said. Its actions stand where the time was while the pointer is on
/// it, and every one of them is in its menu too.
class ReviewCommentRow extends StatefulWidget {
  /// The link it is on, which is where its picture is asked for.
  final Review review;
  final ReviewComment comment;

  /// Where its words mention somebody, as [reviewMentions] has it.
  final List<({int start, int end, bool me})> mentions;

  /// Whether it came in while the window was open, and so plays in, and
  /// whether it is the one the arrow keys are on.
  final bool arrived, chosen;

  /// Where it is, as the composition's own timecode, and where it runs to
  /// when it is a range. Null on a reply.
  final String? timecode, until;

  /// The layer under the place it points at, when it points at one.
  final String? layer;

  /// What was drawn with it, over the frame it was drawn on, while that is
  /// being shown.
  final Widget? drawing;

  /// Null when the composition is not in the open project.
  final VoidCallback? onJump;
  final VoidCallback? onChoose, onResolve, onReply, onDrawing, onCopyLink;

  /// Show its picture whole.
  final VoidCallback? onPicture;
  final VoidCallback onDelete;

  const ReviewCommentRow({
    super.key,
    required this.review,
    required this.comment,
    this.mentions = const [],
    required this.arrived,
    this.chosen = false,
    this.timecode,
    this.until,
    this.layer,
    this.drawing,
    this.onJump,
    this.onChoose,
    this.onResolve,
    this.onReply,
    this.onDrawing,
    this.onCopyLink,
    this.onPicture,
    required this.onDelete,
  });

  @override
  State<ReviewCommentRow> createState() => _ReviewCommentRowState();
}

class _ReviewCommentRowState extends State<ReviewCommentRow> {
  bool _hover = false;

  /// Everything there is to do with the comment, at [at] on the screen.
  void _menu(Offset at) {
    final c = widget.comment;
    reviewRowMenu(context, at, [
      if (widget.timecode != null)
        ('jump', l10n.reviewGoToFrame, widget.onJump),
      if (c.drawn && widget.onDrawing != null)
        (
          'drawing',
          widget.drawing == null
              ? l10n.reviewShowDrawing
              : l10n.reviewHideDrawing,
          widget.onDrawing,
        ),
      if (c.picture != null && widget.onPicture != null)
        ('picture', l10n.reviewShowPicture, widget.onPicture),
      if (widget.onResolve case final resolve?)
        (
          'resolve',
          c.resolved ? l10n.reviewReopen : l10n.reviewResolve,
          resolve,
        ),
      if (widget.onReply case final reply?) ('reply', l10n.reviewReply, reply),
      if (widget.onCopyLink case final copy?)
        ('copy', l10n.reviewCopyCommentLink, copy),
      if (widget.timecode != null) null,
      ('delete', l10n.delete, widget.onDelete),
    ]);
  }

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final c = widget.comment;
    final top = widget.timecode != null;
    final row = MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.translucent,
        onTap: widget.onChoose,
        onSecondaryTapDown: (press) => _menu(press.globalPosition),
        child: Padding(
          padding: const EdgeInsets.only(top: 6, bottom: 8),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SizedBox(
                height: 20,
                child: Row(
                  children: [
                    reviewDot(t, c.name),
                    const SizedBox(width: 8),
                    ConstrainedBox(
                      constraints: const BoxConstraints(maxWidth: 140),
                      child: FittedText(
                          c.name,
                          t.bodyStrong.copyWith(
                              color:
                                  c.resolved ? t.textMuted : t.textPrimary)),
                    ),
                    if (widget.timecode case final timecode?) ...[
                      const SizedBox(width: 8),
                      _timecode(t, c.id, timecode,
                          until: widget.until, onJump: widget.onJump),
                    ],
                    // What is left of the line, with its marks at the start
                    // of it, so the time stands at the far end whatever
                    // there is to say.
                    Expanded(
                      child: Row(
                        children: [
                          if (widget.layer case final layer?) ...[
                            const SizedBox(width: 6),
                            Flexible(child: _layer(t, layer)),
                          ],
                          if (c.drawn) ...[
                            const SizedBox(width: 4),
                            _pen(t),
                          ],
                          if (c.resolved) ...[
                            const SizedBox(width: 6),
                            glyph.LumitIcon(LumitIcons.tick,
                                size: 12, colour: t.success),
                          ],
                        ],
                      ),
                    ),
                    const SizedBox(width: 8),
                    if (!_hover)
                      Text(reviewWhen(c.created), style: t.small)
                    else ...[
                      if (widget.onResolve case final resolve?)
                        reviewAction(
                            t,
                            'resolve-${c.id}',
                            Text(
                                c.resolved
                                    ? l10n.reviewReopen
                                    : l10n.reviewResolve,
                                style: t.small),
                            resolve),
                      if (widget.onReply case final reply?)
                        reviewAction(t, 'reply-${c.id}',
                            Text(l10n.reviewReply, style: t.small), reply),
                      // A comment on a frame has more than there is room
                      // for. A reply has only the one thing to offer.
                      if (top)
                        _more(t, c.id, _menu)
                      else
                        reviewAction(t, 'delete-${c.id}',
                            Text(l10n.delete, style: t.small), widget.onDelete),
                    ],
                  ],
                ),
              ),
              if (c.text.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.only(left: 14, top: 2),
                  child: Text.rich(_words(t),
                      style: t.body.copyWith(
                          color: c.resolved ? t.textMuted : t.textSecondary,
                          height: 1.45)),
                ),
              if (c.picture != null)
                Padding(
                  padding: const EdgeInsets.only(left: 14, top: 6, bottom: 2),
                  child: ReviewPicture(widget.review, c,
                      onPressed: widget.onPicture),
                ),
              if (widget.drawing case final drawing?)
                Padding(
                  padding: const EdgeInsets.only(left: 14, top: 8, bottom: 2),
                  child: drawing,
                ),
            ],
          ),
        ),
      ),
    );
    final marked = _chosenMark(t, widget.chosen, row);
    if (!widget.arrived) return marked;
    return Entrance(
        spec: scope.motion.reveal,
        rise: scope.motion.revealRise,
        child: marked);
  }

  /// What was said, with each mention drawn as one: its name in the accent,
  /// and on a wash of it where it is the link's own sender who is meant.
  InlineSpan _words(LumitTheme t) {
    final c = widget.comment;
    final style = TextStyle(
        color: c.resolved ? t.textMuted : t.accent,
        fontWeight: t.bodyStrong.fontWeight);
    final spans = <InlineSpan>[];
    var at = 0;
    for (final m in widget.mentions) {
      final name = c.text.substring(m.start, m.end);
      spans
        ..add(TextSpan(text: c.text.substring(at, m.start)))
        ..add(!m.me
            ? TextSpan(text: name, style: style)
            : WidgetSpan(
                alignment: PlaceholderAlignment.baseline,
                baseline: TextBaseline.alphabetic,
                child: Container(
                  padding: const EdgeInsets.symmetric(horizontal: 3),
                  decoration: BoxDecoration(
                    color: style.color!.withValues(alpha: 0.14),
                    borderRadius:
                        BorderRadius.circular(t.tokens.controlRadius),
                  ),
                  child: Text(name, style: t.body.merge(style)),
                ),
              ));
      at = m.end;
    }
    return TextSpan(children: [...spans, TextSpan(text: c.text.substring(at))]);
  }

  /// The layer the comment points at: its glyph and its name, kept quiet,
  /// since it is where to look and not something to press.
  Widget _layer(LumitTheme t, String name) => Row(
        key: ValueKey<String>('review-layer-${widget.comment.id}'),
        mainAxisSize: MainAxisSize.min,
        children: [
          glyph.LumitIcon(LumitIcons.layers, size: 11, colour: t.textMuted),
          const SizedBox(width: 3),
          Flexible(
            child: FittedText(
                name, t.small.copyWith(color: t.textSecondary)),
          ),
        ],
      );

  /// The mark that says something was drawn, pressed to see it over its
  /// frame and again to put it away.
  Widget _pen(LumitTheme t) {
    final shown = widget.drawing != null;
    return LumitTooltip(
      message: l10n.reviewDrawing,
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        child: GestureDetector(
          key: ValueKey<String>('review-drawing-${widget.comment.id}'),
          behavior: HitTestBehavior.opaque,
          onTap: widget.onDrawing,
          child: Container(
            width: 20,
            height: 18,
            alignment: Alignment.center,
            decoration: BoxDecoration(
              color: shown ? t.surface2 : null,
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
              border: Border.all(
                  color: shown ? t.hairlineStrong : t.surface1),
            ),
            child: glyph.LumitIcon(LumitIcons.paint,
                size: 12,
                colour: shown ? t.textPrimary : t.textMuted,
                semanticLabel: l10n.reviewDrawing),
          ),
        ),
      ),
    );
  }
}

/// The likes that landed in one second of the video, as one quiet line: who,
/// and where. A like is not a note, so it has nothing to resolve or answer.
class ReviewLikeRow extends StatefulWidget {
  /// Oldest first, and never empty.
  final List<ReviewComment> likes;
  final String timecode;

  /// Whether one of them came in while the window was open, and whether the
  /// line is the one the arrow keys are on.
  final bool arrived, chosen;

  /// Null when the composition is not in the open project.
  final VoidCallback? onJump;
  final VoidCallback? onChoose;

  /// Null for more than one, which there is no one thing to delete of.
  final VoidCallback? onDelete;

  const ReviewLikeRow({
    super.key,
    required this.likes,
    required this.timecode,
    required this.arrived,
    this.chosen = false,
    this.onJump,
    this.onChoose,
    this.onDelete,
  });

  @override
  State<ReviewLikeRow> createState() => _ReviewLikeRowState();
}

class _ReviewLikeRowState extends State<ReviewLikeRow> {
  bool _hover = false;

  void _menu(Offset at) => reviewRowMenu(context, at, [
        ('jump', l10n.reviewGoToFrame, widget.onJump),
        if (widget.onDelete case final delete?) ...[
          null,
          ('delete', l10n.delete, delete),
        ],
      ]);

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final first = widget.likes.first;
    final words = switch (widget.likes.length) {
      1 => l10n.reviewLiked(first.name),
      2 => l10n.reviewLikedTwo(first.name, widget.likes.last.name),
      final many => l10n.reviewLikedMore(first.name, many - 1),
    };
    final row = MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.translucent,
        onTap: widget.onChoose,
        onSecondaryTapDown: (press) => _menu(press.globalPosition),
        child: SizedBox(
          height: 28,
          child: Row(
            children: [
              reviewDot(t, first.name),
              const SizedBox(width: 8),
              Expanded(
                child: Row(
                  children: [
                    Flexible(
                      child: FittedText(
                          words, t.small.copyWith(color: t.textSecondary)),
                    ),
                    const SizedBox(width: 8),
                    _timecode(t, first.id, widget.timecode,
                        onJump: widget.onJump),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              if (_hover)
                _more(t, first.id, _menu)
              else
                Text(reviewWhen(widget.likes.last.created), style: t.small),
            ],
          ),
        ),
      ),
    );
    final marked = _chosenMark(t, widget.chosen, row);
    if (!widget.arrived) return marked;
    return Entrance(
        spec: scope.motion.reveal,
        rise: scope.motion.revealRise,
        child: marked);
  }
}

/// Who has approved the version being looked at, as one quiet line above
/// its comments. The way to take one off again stands where the time was
/// while the pointer is on it, and in its menu.
class ReviewApprovedRow extends StatefulWidget {
  /// Oldest first, and never empty.
  final List<ReviewApproval> approvals;
  final ValueChanged<ReviewApproval> onRemove;

  const ReviewApprovedRow({
    super.key,
    required this.approvals,
    required this.onRemove,
  });

  @override
  State<ReviewApprovedRow> createState() => _ReviewApprovedRowState();
}

class _ReviewApprovedRowState extends State<ReviewApprovedRow> {
  bool _hover = false;

  void _menu(Offset at) => reviewRowMenu(context, at, [
        for (final approval in widget.approvals)
          (
            'unapprove-${approval.id}',
            l10n.reviewRemoveApproval(approval.name),
            () => widget.onRemove(approval),
          ),
      ]);

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final all = widget.approvals;
    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.translucent,
        onSecondaryTapDown: (press) => _menu(press.globalPosition),
        child: SizedBox(
          height: 28,
          child: Row(
            children: [
              // As wide as a commenter's dot and its gap, so the words line
              // up with the comments under them.
              SizedBox(
                width: 14,
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: glyph.LumitIcon(LumitIcons.tick,
                      size: 11, colour: t.success),
                ),
              ),
              Expanded(
                child: FittedText(reviewApprovedBy(all),
                    t.small.copyWith(color: t.textSecondary)),
              ),
              const SizedBox(width: 8),
              if (!_hover)
                Text(reviewWhen(all.last.created), style: t.small)
              else
                // One approval is the one to remove. With more, the word
                // opens the list of them under itself.
                Builder(
                  builder: (word) => reviewAction(
                    t,
                    'unapprove',
                    Text(l10n.reviewRemove, style: t.small),
                    () {
                      if (all.length == 1) return widget.onRemove(all.single);
                      final box = word.findRenderObject()! as RenderBox;
                      _menu(box.localToGlobal(
                          Offset(box.size.width, box.size.height + 2)));
                    },
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}
