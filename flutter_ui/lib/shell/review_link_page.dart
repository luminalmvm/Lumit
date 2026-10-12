// The Link page of the Previews window: what reviewers may do with a
// link, who has opened it, its versions, and the way to delete it.
//
// It asks the server one thing itself, as it opens: who has opened the link.
// Everything else it is handed, and says what was pressed.

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/external_links.dart';
import '../state/reviews.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'pro_window.dart';
import 'review_link_parts.dart';

const double _labelColumn = 110;

/// How many of the people who opened a link are listed before the rest are
/// only counted.
const int _viewsShown = 6;

/// The page of a link's own settings: what reviewers may do, who has opened
/// it, its versions, and the way to delete it.
class ReviewLinkPage extends StatefulWidget {
  final ReviewsState reviews;
  final Review review;

  /// Run something the server may refuse, and say why if it does.
  final Future<void> Function(Future<void> Function() call) run;

  /// Say what is wrong with something typed here.
  final ValueChanged<String> onRefused;
  final ValueChanged<ReviewVersion> onDeleteVersion;
  final VoidCallback onDeleteLink;

  /// Put an address on the clipboard and say so, and take the link's video
  /// only address away, after asking.
  final ValueChanged<String> onCopy;
  final VoidCallback onRemoveClean;

  const ReviewLinkPage({
    super.key,
    required this.reviews,
    required this.review,
    required this.run,
    required this.onRefused,
    required this.onDeleteVersion,
    required this.onDeleteLink,
    required this.onCopy,
    required this.onRemoveClean,
  });

  @override
  State<ReviewLinkPage> createState() => _ReviewLinkPageState();
}

class _ReviewLinkPageState extends State<ReviewLinkPage> {
  final _password = TextEditingController();

  /// Who has opened the link, or null until the server has said, and
  /// whether the whole of the list is shown and not only the start of it.
  List<ReviewView>? _views;
  bool _allViews = false;

  @override
  void initState() {
    super.initState();
    // The button beside the field says Set or Clear by what is typed.
    _password.addListener(() => setState(() {}));
    // Asked as the page opens, and not again while it is up.
    unawaited(widget.reviews.views(widget.review).then((views) {
      if (mounted) setState(() => _views = views);
    }, onError: (Object _) {
      // The rest of the page does not wait on it.
    }));
  }

  @override
  void dispose() {
    _password.dispose();
    super.dispose();
  }

  void _setPassword() {
    final typed = _password.text;
    if (typed.isEmpty && !widget.review.password) return;
    // It travels in a header, which takes plain characters and no others.
    if (typed.isNotEmpty && !RegExp(r'^[\x20-\x7e]{1,100}$').hasMatch(typed)) {
      widget.onRefused(l10n.reviewErrorPassword);
      return;
    }
    unawaited(widget.run(() async {
      await widget.reviews
          .patch(widget.review, {'password': typed.isEmpty ? null : typed});
      _password.clear();
    }));
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _reviewers(t),
        const SizedBox(height: 4),
        _seenBy(t),
        const SizedBox(height: 4),
        _versions(t),
        const SizedBox(height: 12),
        Align(
          alignment: Alignment.centerRight,
          child: HouseButton(
            key: const ValueKey('review-delete'),
            small: true,
            onPressed: widget.onDeleteLink,
            child: Text(l10n.reviewDeleteLink, style: t.small),
          ),
        ),
      ],
    );
  }

  Widget _reviewers(LumitTheme t) {
    final review = widget.review;
    // [live] is whether it has anything to decide. One that has not is off
    // and takes no press.
    Widget toggle(String id, String label, bool value, String field,
            {bool live = true}) =>
        dialogRow(
          t,
          label,
          Align(
            alignment: Alignment.centerLeft,
            child: IgnorePointer(
              ignoring: !live,
              child: Opacity(
                opacity: live ? 1 : 0.4,
                child: HouseToggle(
                  key: ValueKey<String>('review-$id'),
                  value: value && live,
                  onChanged: (on) => unawaited(widget
                      .run(() => widget.reviews.patch(review, {field: on}))),
                ),
              ),
            ),
          ),
          labelColumn: _labelColumn,
        );
    final expires = review.expires;
    final mailOn = widget.reviews.account.account?.reviewMail ?? true;
    return dialogGroup(t, l10n.reviewReviewers, [
      toggle('comments', l10n.reviewCanComment, review.comments, 'comments'),
      // A picture comes on a comment, so with comments off there are none.
      toggle('pictures', l10n.reviewCanAttach, review.pictures, 'pictures',
          live: review.comments),
      toggle(
          'downloads', l10n.reviewCanDownload, review.downloads, 'downloads'),
      // Mail to whoever sent it, about new comments on this link. With the
      // account's own switch off there is none to be had, and it says so.
      dialogRow(
        t,
        l10n.reviewMail,
        Row(
          children: [
            IgnorePointer(
              ignoring: !mailOn,
              child: Opacity(
                opacity: mailOn ? 1 : 0.4,
                child: HouseToggle(
                  key: const ValueKey('review-mail'),
                  value: review.mail,
                  onChanged: (on) => unawaited(widget
                      .run(() => widget.reviews.patch(review, {'mail': on}))),
                ),
              ),
            ),
            if (!mailOn) ...[
              const SizedBox(width: 8),
              Text(l10n.reviewMailOff,
                  key: const ValueKey('review-mail-off'), style: t.small),
            ],
          ],
        ),
        labelColumn: _labelColumn,
      ),
      // A second address that shows the video and nothing of the review,
      // for whoever should see the work and not what was said about it. The
      // first press makes it, and every press copies it.
      dialogRow(
        t,
        l10n.reviewClean,
        Row(
          children: [
            HouseButton(
              key: const ValueKey('review-clean-copy'),
              small: true,
              onPressed: () => unawaited(widget.run(() async => widget
                  .onCopy(await widget.reviews.cleanAddress(review)))),
              child: Text(l10n.reviewCopyLink, style: t.small),
            ),
            // The one version it shows, where the link has more than one.
            // It stays on that one until it is moved by hand.
            if (review.cleanVersion case final shows?
                when review.versions.length > 1) ...[
              const SizedBox(width: 8),
              KeyedSubtree(
                key: const ValueKey('review-clean-version'),
                child: reviewTag(t, l10n.reviewVersion(shows)),
              ),
              if (shows < review.versions.last.n)
                reviewAction(
                    t,
                    'clean-move',
                    Text(
                        l10n.reviewCleanMove(
                            l10n.reviewVersion(review.versions.last.n)),
                        style: t.small),
                    () => unawaited(widget.run(() => widget.reviews
                        .cleanAddress(review,
                            version: review.versions.last.n)))),
            ],
            if (review.clean != null)
              reviewAction(
                  t,
                  'clean-remove',
                  Text(l10n.reviewRemove, style: t.small),
                  widget.onRemoveClean),
          ],
        ),
        labelColumn: _labelColumn,
      ),
      dialogRow(
        t,
        l10n.sharePassword,
        Row(
          children: [
            Expanded(
              child: SizedBox(
                height: dialogControlHeight,
                child: HouseTextField(
                  key: const ValueKey('review-password'),
                  controller: _password,
                  width: double.infinity,
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                  fill: t.surface0,
                  obscure: true,
                  // Dots for one that is set. What it is never comes back.
                  hint: review.password ? '••••••••' : l10n.none,
                  onSubmitted: (_) => _setPassword(),
                ),
              ),
            ),
            const SizedBox(width: 8),
            HouseButton(
              key: const ValueKey('review-password-set'),
              small: true,
              onPressed: _password.text.isEmpty && !review.password
                  ? null
                  : _setPassword,
              child: Text(
                  _password.text.isEmpty && review.password
                      ? l10n.clear
                      : l10n.reviewPasswordSet,
                  style: t.small),
            ),
          ],
        ),
        labelColumn: _labelColumn,
      ),
      dialogRow(
        t,
        l10n.reviewExpires,
        Align(
          alignment: Alignment.centerLeft,
          // Nought is never, and minus one is the day already set, which
          // stays on the list as what is in force.
          child: dialogDropdown<int>(
            t,
            id: 'review-expires',
            value: expires == null ? 0 : -1,
            options: [if (expires != null) -1, 0, 1, 7, 30],
            label: (days) => switch (days) {
              -1 => writtenDate(
                  DateTime.fromMillisecondsSinceEpoch((expires ?? 0) * 1000)),
              0 => l10n.reviewExpiresNever,
              _ => l10n.reviewExpiresDays(days),
            },
            onChanged: (days) {
              if (days < 0) return;
              final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
              unawaited(widget.run(() => widget.reviews.patch(
                  review, {'expires': days == 0 ? null : now + days * 86400})));
            },
          ),
        ),
        labelColumn: _labelColumn,
      ),
    ]);
  }

  /// Who has opened the link in a browser, newest first: the name they gave
  /// if they gave one, the version they were on, and when they were last
  /// there. The first few are listed and the rest counted.
  Widget _seenBy(LumitTheme t) {
    final views = _views ?? const <ReviewView>[];
    return dialogGroup(
      t,
      views.isEmpty ? l10n.reviewSeenBy : l10n.reviewSeenByCount(views.length),
      key: const ValueKey('review-seen-by'),
      [
        if (views.isEmpty)
          ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 24),
            child: Align(
              alignment: Alignment.centerLeft,
              // Nothing is said until the server has, so the line does not
              // flash "nobody" at somebody with twenty viewers.
              child: Text(_views == null ? '' : l10n.reviewSeenByNobody,
                  style: t.small),
            ),
          ),
        for (final view in views.take(_allViews ? views.length : _viewsShown))
          ConstrainedBox(
            constraints: const BoxConstraints(minHeight: 24),
            child: Row(
              children: [
                Expanded(
                  child: Row(
                    children: [
                      // The dot their comments have, for one who gave a
                      // name, and its room kept for one who did not.
                      SizedBox(
                        width: 14,
                        child: view.name.isEmpty
                            ? null
                            : Align(
                                alignment: Alignment.centerLeft,
                                child: reviewDot(t, view.name)),
                      ),
                      Flexible(
                        child: FittedText(
                          view.name.isEmpty
                              ? l10n.reviewSeenBySomeone
                              : view.name,
                          view.name.isEmpty
                              ? t.body.copyWith(color: t.textMuted)
                              : t.body,
                        ),
                      ),
                      const SizedBox(width: 8),
                      // Said quietly: a column of boxed tags would outweigh
                      // the names.
                      Text(l10n.reviewVersion(view.version),
                          style: dialogMono(t)),
                    ],
                  ),
                ),
                Text(reviewWhen(view.last), style: t.small),
              ],
            ),
          ),
        // The rest of them, shown in place when asked for.
        if (views.length > _viewsShown && !_allViews)
          Align(
            alignment: Alignment.centerLeft,
            child: HouseButton(
              key: const ValueKey('review-seen-by-more'),
              small: true,
              frameless: true,
              padding: const EdgeInsets.only(right: 6),
              onPressed: () => setState(() => _allViews = true),
              child: Text(l10n.reviewSeenByMore(views.length - _viewsShown),
                  style: t.small),
            ),
          ),
      ],
    );
  }

  /// Every version, newest first: what it is, a tick where a reviewer has
  /// approved it, what its sender said had changed, when it went up, the way
  /// to see it beside the one before, and the way to delete it while there
  /// is another to fall back on.
  Widget _versions(LumitTheme t) {
    final review = widget.review;
    final versions = review.versions;
    return dialogGroup(t, l10n.reviewVersions, [
      for (final (index, v) in versions.indexed.toList().reversed)
        _version(t, review, v,
            before: index > 0 ? versions[index - 1] : null,
            only: versions.length == 1),
    ]);
  }

  /// One version's row. [before] is the one it took over from, which it can
  /// be seen beside, and [only] is whether it is the last one the link has,
  /// which stays.
  Widget _version(LumitTheme t, Review review, ReviewVersion v,
      {required ReviewVersion? before, required bool only}) {
    final compare = before == null
        ? null
        : () => unawaited(
            openExternalLink(review.compareUrl(before.n, v.n)));
    final delete = only ? null : () => widget.onDeleteVersion(v);
    return Builder(
      builder: (row) => GestureDetector(
        behavior: HitTestBehavior.translucent,
        // What the row's two words do, for a right click too.
        onSecondaryTapDown: compare == null && delete == null
            ? null
            : (press) => reviewRowMenu(row, press.globalPosition, [
                  if (compare != null)
                    ('compare', l10n.reviewCompare, compare),
                  if (compare != null && delete != null) null,
                  if (delete != null)
                    ('delete-version', l10n.reviewDeleteVersion, delete),
                ]),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            ConstrainedBox(
              constraints: const BoxConstraints(minHeight: 28),
              child: Row(
                children: [
                  SizedBox(
                    width: 40,
                    child: Align(
                      alignment: Alignment.centerLeft,
                      child: reviewTag(t, l10n.reviewVersion(v.n)),
                    ),
                  ),
                  Expanded(
                    child: Row(
                      children: [
                        Flexible(
                          child: Text(reviewFacts(v),
                              style: dialogMono(t),
                              overflow: TextOverflow.ellipsis),
                        ),
                        // After the facts, so they stay in line down the
                        // list whether a version is approved or not.
                        if (v.approvals.isNotEmpty) ...[
                          const SizedBox(width: 8),
                          LumitTooltip(
                            key: ValueKey<String>(
                                'review-version-approved-${v.n}'),
                            message: reviewApprovedBy(v.approvals),
                            // The word as well as the tick, as the list of
                            // links has it: a bare tick could mean anything.
                            child: Row(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                glyph.LumitIcon(LumitIcons.tick,
                                    size: 12, colour: t.success),
                                const SizedBox(width: 4),
                                Text(l10n.reviewApproved,
                                    style: t.small
                                        .copyWith(color: t.textSecondary)),
                              ],
                            ),
                          ),
                        ],
                      ],
                    ),
                  ),
                  Text(reviewWhen(v.created), style: t.small),
                  const SizedBox(width: 6),
                  // Beside the one it took over from, in the browser. The
                  // first version has none before it, and keeps the word's
                  // room so the columns stand in line down the list, while
                  // there is a list for them to stand in line down.
                  if (compare != null)
                    reviewAction(t, 'compare-${v.n}',
                        Text(l10n.reviewCompare, style: t.small), compare)
                  else if (!only)
                    Visibility.maintain(
                      visible: false,
                      child: reviewAction(t, 'compare-none',
                          Text(l10n.reviewCompare, style: t.small), () {}),
                    ),
                  if (delete != null)
                    reviewAction(
                        t,
                        'delete-version-${v.n}',
                        Text(l10n.reviewDeleteVersion, style: t.small),
                        delete),
                ],
              ),
            ),
            if (v.note.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(left: 40, bottom: 6),
                child: Text(v.note,
                    key: ValueKey<String>('review-note-${v.n}'),
                    style: t.small.copyWith(
                        color: t.textSecondary, height: 1.4)),
              ),
          ],
        ),
      ),
    );
  }
}
