// The Previews window: the exports sent out for comments, and what came
// back.
//
// One window that reads the account and shows what is true of it. Somebody
// without Lumit Pro sees the way to it. Somebody with it sees their links
// down the left and the chosen one on the right: its address, what reviewers
// may do, its versions and its comments, which can be jumped to, answered
// and put on the Timeline as markers. Sending a composition is the window's
// other face (review_send.dart).
//
// The server is [ReviewsState]'s business. It is asked things when the
// window opens or a control is pressed, never in a build.

import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/services.dart'
    show
        Clipboard,
        ClipboardData,
        KeyDownEvent,
        KeyUpEvent,
        LogicalKeyboardKey;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:provider/provider.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../panels/layer_fold_frb.dart' show asOneUndoStep;
import '../panels/viewer_stage.dart' show layerAtCompPoint;
import '../state/account.dart' show Friend;
import '../state/external_links.dart';
import '../state/reviews.dart';
import '../state/timecode.dart';
import '../state/workspace.dart' show Workspace;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';
import 'dialog_frame.dart';
import 'pro_window.dart';
import 'profile_chip.dart';
import 'review_drawing.dart';
import 'review_link_page.dart';
import 'review_link_parts.dart';
import 'review_send.dart';
import 'settings_account.dart' show askBefore;
import 'settings_window_frb.dart' show SettingsPage, showSettingsWindowFrb;

/// The window with its list in it, the list's own column, and how tall the
/// two stand before they scroll. The column is as wide as a row's second
/// line at its fullest: a version, an open count, Approved and Changed. The
/// height is nine and a half rows under the list's own margin, so a longer
/// list shows half of its tenth and never a sliver of one.
const double reviewWindowWidth = 820;
const double _listWidth = 280;
const double _listMargin = 8;
const double _bodyHeight = _listMargin + 9.5 * reviewLinkRowHeight;

/// The two pages of the chosen link: what people said, and the link itself.
enum _Page { comments, link }

/// Open the Previews window: on [link] when one is named, and straight on
/// sending [send] when a composition is.
Future<void> showReviewLinks(BuildContext context,
    {String? link, CompositionReference? send}) async {
  final ui = context.read<LumitUiState>();
  final app = context.read<LumitState>();
  final reviews = ui.reviews;
  // A send that failed while the window was shut is said once, as it opens.
  final ended = reviews.sending;
  final failure = ended?.stage == SendStage.failed ? ended?.failure : null;
  reviews.clearSend();
  await showLumitModal<void>(
    context: context,
    id: 'review-links',
    builder: (close) => _ReviewWindow(
      ui: ui,
      app: app,
      link: link,
      send: send,
      failure: failure,
      close: () => close(null),
    ),
  );
  // Nothing is being looked at now, and a send that has ended was seen.
  unawaited(reviews.watch(null));
  reviews.clearSend();
}

class _ReviewWindow extends StatefulWidget {
  final LumitUiState ui;
  final LumitState app;
  final String? link;
  final CompositionReference? send;

  /// Why the last send failed, when it did so with the window shut.
  final String? failure;
  final VoidCallback close;

  const _ReviewWindow({
    required this.ui,
    required this.app,
    required this.link,
    required this.send,
    required this.failure,
    required this.close,
  });

  @override
  State<_ReviewWindow> createState() => _ReviewWindowState();
}

class _ReviewWindowState extends State<_ReviewWindow> {
  final _reply = TextEditingController();
  final _listScroll = ScrollController();
  final _detailScroll = ScrollController();
  final _focus = FocusNode(debugLabel: 'review links');

  /// The comment, or line of likes, the arrow keys are on. Null while they
  /// are on the list of links, which is where they start.
  String? _at;

  /// Each listed comment's place in the tree, to bring it into view by.
  final _rowKeys = <String, GlobalKey>{};

  /// The link on the right, the version of it whose comments are shown (the
  /// newest when null), and whether resolved comments are shown with them.
  String? _chosen;
  int? _version;
  bool _all = false;
  _Page _page = _Page.comments;

  /// The composition being sent, while the window is on its other face.
  CompositionReference? _sendFor;

  /// The chosen link's composition, when the open project has it, and the
  /// link that was looked up for.
  CompositionReference? _comp;
  String? _compFor;

  /// How many markers Add to Timeline would make, and what that was counted
  /// from.
  int _pending = 0;
  Object? _counted;

  /// The layer under the place each comment points at, by the comment's id,
  /// which is the layer its marker goes on. Found with the count above.
  Map<String, BridgeLayerEntry> _layers = {};

  /// The links whose composition has been edited since their newest version
  /// went out.
  Set<String> _moved = {};

  /// The comment whose drawing is being shown, and its frame of the
  /// composition once the engine has drawn it.
  String? _drawingOf;
  ui.Image? _frame;

  /// How tall the drawing may stand: what the comments' own room has left
  /// under its comment's first lines, so the mark that put it there is still
  /// in view to put it away with.
  double _drawingRoom = 240;

  /// The link a new version was asked for from its own page.
  Review? _sendInto;

  String? _replyTo, _error;
  bool _asked = false;

  ReviewsState get _reviews => widget.ui.reviews;

  @override
  void initState() {
    super.initState();
    if (widget.failure case final failure?) _error = reviewErrorText(failure);
    _sendFor = widget.send;
    if (widget.ui.account.signedIn) {
      _asked = true;
      unawaited(_reviews.refresh());
    }
    final first = widget.link ??
        (_reviews.newestUnseen ?? _reviews.links.firstOrNull)?.id;
    if (first != null) _choose(first);
    _found();
    _reviews.addListener(_changed);
    widget.ui.account.addListener(_changed);
    // Escape with a reply half typed puts the reply away and leaves the
    // window up. The window's own claim is asked after this one.
    _escape = EscapeLadder.register(EscapeRung.gesture, () {
      if (_replyTo == null) return false;
      setState(() => _replyTo = null);
      _focus.requestFocus();
      return true;
    });
  }

  late final VoidCallback _escape;

  @override
  void dispose() {
    _reviews.removeListener(_changed);
    widget.ui.account.removeListener(_changed);
    _escape();
    _frame?.dispose();
    _reply.dispose();
    _listScroll.dispose();
    _detailScroll.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _changed() {
    if (!mounted) return;
    // Whoever signs in while the window is up is asked about once.
    final signedIn = widget.ui.account.signedIn;
    if (signedIn && !_asked) unawaited(_reviews.refresh());
    _asked = signedIn;
    // A send this window has no form for failed: back to the list, saying so.
    final send = _reviews.sending;
    if (_sendFor == null && send?.stage == SendStage.failed) {
      _error = reviewErrorText(send!.failure ?? '');
      _reviews.clearSend();
      return;
    }
    // A send refused for want of Lumit Pro is put away. The window says that
    // in the form's place, with the way to it.
    if (!widget.ui.account.pro && send?.stage == SendStage.failed) {
      _reviews.clearSend();
    }
    if (_reviews.link(_chosen) == null && _reviews.links.isNotEmpty) {
      _choose((_reviews.newestUnseen ?? _reviews.links.first).id);
    }
    _found();
    setState(() {});
  }

  void _choose(String id) {
    _chosen = id;
    _version = null;
    _page = _Page.comments;
    _replyTo = null;
    _drawingOf = null;
    _error = null;
    _at = null;
    _rowKeys.clear();
    if (_detailScroll.hasClients) _detailScroll.jumpTo(0);
    // After this turn, since it tells everyone listening and this may be
    // running while the window is first being built.
    scheduleMicrotask(() {
      if (mounted && _chosen == id) unawaited(_reviews.watch(id));
    });
  }

  ReviewVersion _versionOf(Review review) => review.versions
      .firstWhere((v) => v.n == _version, orElse: () => review.versions.last);

  /// Look the chosen link's composition up once it is known, find the layer
  /// each of its comments points at, and count what they would add to the
  /// Timeline. All of it asks the engine, so all of it is done here, when
  /// something changed, and kept for the build.
  void _found() {
    // Not while a composition is going up, which says so ten times a second.
    if (_reviews.sending == null) {
      try {
        _moved = {
          for (final link in _reviews.links)
            if (_compOf(link) case final comp?
                when _reviews.movedOn(link, comp))
              link.id,
        };
      } catch (_) {
        // A composition that has gone has not moved on.
      }
    }
    final review = _reviews.link(_chosen);
    if (review != null && _compFor != _chosen) {
      _compFor = _chosen;
      _comp = _compOf(review);
    }
    final comp = _comp;
    final counted = (_reviews.comments, _chosen, _version, comp);
    if (counted == _counted) return;
    _counted = counted;
    _pending = 0;
    _layers = {};
    if (review == null || comp == null) return;
    try {
      final version = _versionOf(review);
      final settings = comp.getSettings();
      final last = comp.durationFrames() - 1;
      for (final c in _reviews.comments) {
        final point = c.point;
        if (c.version != version.n || c.parent != null || point == null) {
          continue;
        }
        // The picture is the composition's own, so a share of one is the
        // same share of the other.
        final under = layerAtCompPoint(
            widget.ui,
            comp,
            (version.startFrame + c.frame).clamp(0, last < 0 ? 0 : last),
            Offset(point.dx * settings.width, point.dy * settings.height));
        if (under == null) continue;
        _layers[c.id] = under;
      }
      _pending = _reviews
          .pendingMarkers(review, version, comp, layers: _layers)
          .length;
    } catch (_) {
      // The composition has gone since it was found.
      _comp = null;
    }
  }

  /// The composition [review] was sent from, in the open project: by its id
  /// where this machine remembers it, else by the name it had.
  CompositionReference? _compOf(Review review) {
    final origin = _reviews.originOf(review);
    final name = origin == null || origin.name.isEmpty
        ? review.versions.last.comp
        : origin.name;
    CompositionReference? named;
    try {
      for (final (comp, called) in widget.app.comps()) {
        if ('${comp.internalid}' == origin?.comp) return comp;
        if (called == name) named ??= comp;
      }
    } catch (_) {
      // No project to look in.
    }
    return named;
  }

  /// What the Comments page lists for [version], in the order it was said:
  /// a comment on a frame, or the likes that landed in one second of the
  /// video. Resolved comments are among them only when all are asked for.
  List<Object> _listed(ReviewVersion version) {
    final perSecond =
        (version.fpsNum / math.max(1, version.fpsDen)).ceil().clamp(1, 1000);
    final likes = <int, List<ReviewComment>>{};
    final listed = <Object>[];
    for (final c in _reviews.comments) {
      if (c.version != version.n || c.parent != null) continue;
      if (c.reaction != null) {
        final second = (version.startFrame + c.frame) ~/ perSecond;
        final together = likes[second];
        together == null
            ? listed.add(likes[second] = [c])
            : together.add(c);
      } else if (_all || !c.resolved) {
        listed.add(c);
      }
    }
    return listed;
  }

  /// The comment a listed thing is, or the first like of a line of them.
  static ReviewComment _first(Object listed) =>
      listed is ReviewComment ? listed : (listed as List<ReviewComment>).first;

  /// Up and down move through the links, or through the chosen link's
  /// comments once Right or Enter has gone across to them. Left comes back.
  /// On a comment, Enter puts the playhead on its frame. Home and End go to
  /// the ends of whichever the keys are in.
  KeyEventResult _key(FocusNode node, KeyEvent event) {
    if (event is KeyUpEvent) return KeyEventResult.ignored;
    // A field being typed in keeps its keys.
    final typing = FocusManager.instance.primaryFocus?.context
            ?.findAncestorWidgetOfExactType<EditableText>() !=
        null;
    final review = _reviews.link(_chosen);
    if (typing || review == null) return KeyEventResult.ignored;
    final key = event.logicalKey;
    final version = _versionOf(review);
    final listed =
        _page == _Page.comments ? _listed(version) : const <Object>[];
    final links = _reviews.links;
    final onLinks = _at == null;
    if (key == LogicalKeyboardKey.arrowRight && onLinks) {
      if (listed.isNotEmpty) _onComment(_first(listed.first).id);
      return KeyEventResult.handled;
    }
    if (key == LogicalKeyboardKey.arrowLeft && !onLinks) {
      setState(() => _at = null);
      return KeyEventResult.handled;
    }
    final count = onLinks ? links.length : listed.length;
    final from = onLinks
        ? links.indexWhere((link) => link.id == _chosen)
        : listed.indexWhere((item) => _first(item).id == _at);
    final to = switch (key) {
      LogicalKeyboardKey.arrowUp => math.max(0, from - 1),
      LogicalKeyboardKey.arrowDown => math.min(count - 1, from + 1),
      LogicalKeyboardKey.home => 0,
      LogicalKeyboardKey.end => count - 1,
      _ => null,
    };
    if (to != null) {
      if (to != from && to >= 0) {
        if (onLinks) {
          setState(() => _choose(links[to].id));
          _revealLink(to);
        } else {
          _onComment(_first(listed[to]).id);
        }
      }
      return KeyEventResult.handled;
    }
    final enter = key == LogicalKeyboardKey.enter ||
        key == LogicalKeyboardKey.numpadEnter;
    if (!enter || event is! KeyDownEvent) return KeyEventResult.ignored;
    if (onLinks) {
      if (listed.isNotEmpty) _onComment(_first(listed.first).id);
    } else if (from >= 0) {
      _jump(version, _first(listed[from]));
    }
    return KeyEventResult.handled;
  }

  /// Put the keys on the comment called [id], and bring it into view.
  void _onComment(String id) {
    _focus.requestFocus();
    setState(() => _at = id);
    _reveal(id);
  }

  /// Scroll the comments as little as brings the one called [id] into view,
  /// once it has been laid out as it now is.
  void _reveal(String id) {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      final row = _rowKeys[id]?.currentContext;
      if (row == null || !mounted) return;
      // Its foot and then its head, so the whole of it shows when it fits.
      for (final policy in const [
        ScrollPositionAlignmentPolicy.keepVisibleAtEnd,
        ScrollPositionAlignmentPolicy.keepVisibleAtStart,
      ]) {
        unawaited(Scrollable.ensureVisible(row, alignmentPolicy: policy));
      }
    });
  }

  /// Scroll the list as little as brings the link at [index] into view.
  void _revealLink(int index) {
    if (!_listScroll.hasClients) return;
    final at = _listScroll.position;
    final top = _listMargin + index * reviewLinkRowHeight;
    final foot = top + reviewLinkRowHeight + _listMargin;
    final to = top - _listMargin < at.pixels
        ? top - _listMargin
        : foot > at.pixels + at.viewportDimension
            ? foot - at.viewportDimension
            : null;
    if (to != null) _listScroll.jumpTo(to.clamp(0.0, at.maxScrollExtent));
  }

  /// Ask before something that can't be taken back, and give the keys back
  /// to the window once it is answered.
  Future<bool> _ask(String question, String yes, {String? note}) async {
    final sure = await askBefore(context, question, yes, note: note);
    if (mounted) _focus.requestFocus();
    return sure && mounted;
  }

  /// Show the picture [comment] came with whole, over the window, until it
  /// or the room round it is pressed. It holds the keys meanwhile, so the
  /// arrows move nothing behind it.
  Future<void> _showPicture(Review review, ReviewComment comment) async {
    await showLumitModal<void>(
      context: context,
      builder: (close) => Focus(
        autofocus: true,
        child: ReviewPicture(review, comment,
            whole: true, onPressed: () => close(null)),
      ),
    );
    // Whole, it is large, so it is not kept once it is put away.
    unawaited(NetworkImage(review.pictureUrl(comment)).evict());
    if (mounted) _focus.requestFocus();
  }

  /// Where [comment] mentions somebody: whoever sent the link, or anyone
  /// who has commented on it or approved it.
  List<({int start, int end, bool me})> _mentions(
      Review review, ReviewComment comment) {
    if (!comment.text.contains('@')) return const [];
    final me = _reviews.account.account?.name ?? '';
    return reviewMentions(
        comment.text,
        [
          me,
          for (final c in _reviews.comments)
            if (c.reaction == null) c.name,
          for (final approval in review.approvals) approval.name,
        ],
        me: me);
  }

  /// Run something the server may refuse, and say why if it does.
  Future<void> _do(Future<void> Function() call) async {
    setState(() => _error = null);
    try {
      await call();
    } catch (e) {
      if (mounted) setState(() => _error = reviewErrorText(reviewFailure(e)));
    }
  }

  /// The composition frame a comment on [version] is on.
  int _frameOf(
      CompositionReference comp, ReviewVersion version, ReviewComment comment) {
    final last = comp.durationFrames() - 1;
    return (version.startFrame + comment.frame).clamp(0, last < 0 ? 0 : last);
  }

  /// Put the playhead on [comment]'s frame, in its own composition, and
  /// select the layer it points at when it points at one.
  void _jump(ReviewVersion version, ReviewComment comment) {
    final comp = _comp;
    if (comp == null) return;
    final ui = widget.ui;
    try {
      final frame = _frameOf(comp, version, comment);
      ui.selectedComp == comp
          ? ui.scrubTo(frame)
          : ui.setSelectedComp(comp, atFrame: frame);
      if (_layers[comment.id] case final under?) {
        ui.setSelection([under.layer]);
      }
    } catch (_) {
      // The composition has gone.
    }
  }

  /// Show what was drawn with [comment] over its frame, or put it away. The
  /// frame is the composition's as it is now, which the engine draws once
  /// and this keeps for as long as it is shown.
  void _toggleDrawing(ReviewVersion version, ReviewComment comment) {
    final shown = _drawingOf == comment.id ? null : comment.id;
    final was = _frame;
    setState(() {
      _drawingOf = shown;
      _frame = null;
      if (_detailScroll.hasClients) {
        _drawingRoom = (_detailScroll.position.viewportDimension - 76)
            .clamp(120.0, 240.0);
      }
    });
    // Once the frame that drew it has gone by.
    WidgetsBinding.instance.addPostFrameCallback((_) => was?.dispose());
    // And its comment is brought into the window with it.
    _reveal(comment.id);
    final comp = _comp;
    if (shown == null || comp == null) return;
    unawaited(() async {
      try {
        final image = await Workspace.compStill(
            comp, _frameOf(comp, version, comment),
            maxEdge: 960);
        if (!mounted || _drawingOf != shown || _frame != null) {
          image?.dispose();
          return;
        }
        setState(() => _frame = image);
      } catch (_) {
        // The strokes are shown without the frame.
      }
    }());
  }

  void _copy(String address) {
    unawaited(Clipboard.setData(ClipboardData(text: address)));
    widget.app.postNotice(l10n.reviewCopied);
  }

  /// Everything there is to do with [review] from its row in the list.
  void _linkMenu(BuildContext row, Review review, Offset at) {
    final comp = _moved.contains(review.id) ? _compOf(review) : null;
    reviewRowMenu(row, at, [
      ('copy-link', l10n.reviewCopyLink, () => _copy(review.url)),
      (
        'open',
        l10n.reviewOpenInBrowser,
        () => unawaited(openExternalLink(review.ownerUrl)),
      ),
      (
        'send-friend',
        l10n.reviewSendToFriend,
        () => unawaited(_pickFriend(row, at, review)),
      ),
      if (comp != null && widget.ui.account.pro)
        (
          'send-version',
          l10n.reviewSendNewVersion,
          () => setState(() {
                _sendFor = comp;
                _sendInto = review;
              }),
        ),
      null,
      (
        'delete-link',
        l10n.reviewDeleteLink,
        () => unawaited(_deleteLink(review)),
      ),
    ]);
  }

  /// A menu at [at] of the account's friends, to send [review] to. Without
  /// a username there are none, and the menu says so, with the way to the
  /// Account page where one is chosen and friends are added. Whoever made
  /// the link is left out: sending it back to them does nothing.
  Future<void> _pickFriend(BuildContext anchor, Offset at, Review review) async {
    final account = widget.ui.account;
    await account.loadFriends();
    if (!mounted || !anchor.mounted) return;
    final named = account.account?.username != null;
    final friends = named
        ? [
            for (final friend in account.friends.friends)
              if (friend.username != review.ownerUsername) friend,
          ]
        : const <Friend>[];
    reviewRowMenu(anchor, at, [
      if (!named)
        ('friends-username', l10n.friendsNeedUsername, null)
      else if (friends.isEmpty)
        ('friends-none', l10n.reviewNoFriends, null)
      else
        for (final friend in friends)
          (
            'friend-${friend.username}',
            friend.name,
            () => unawaited(_do(() async {
                  await account.sendToFriend(review.id, friend.username);
                  widget.app.postNotice(l10n.reviewSentToFriend(friend.name));
                })),
          ),
      if (friends.isEmpty) ...[
        null,
        (
          'friends-settings',
          l10n.profileAccountSettings,
          () => unawaited(showSettingsWindowFrb(context,
              initialPage: SettingsPage.account)),
        ),
      ],
    ]);
  }

  /// [_pickFriend], under the button that asked.
  void _pickFriendUnder(BuildContext button, Review review) {
    final box = button.findRenderObject()! as RenderBox;
    unawaited(_pickFriend(
        button, box.localToGlobal(Offset(0, box.size.height + 2)), review));
  }

  void _add(Review review, ReviewVersion version) {
    final comp = _comp;
    if (comp == null) return;
    try {
      // The composition's markers and each layer's are written apart.
      asOneUndoStep(widget.app.project, () {
        if (_reviews.addToTimeline(review, version, comp, layers: _layers) >
            0) {
          widget.app.notifyDocumentChanged();
        }
      });
    } catch (_) {
      _comp = null;
    }
    _counted = null;
    _found();
    setState(() {});
  }

  Future<void> _deleteLink(Review review) async {
    final sure = await _ask(l10n.reviewDeleteLinkAsk(review.title), l10n.delete,
        note: l10n.reviewDeleteLinkNote);
    if (sure) await _do(() => _reviews.deleteLink(review));
  }

  Future<void> _deleteVersion(Review review, ReviewVersion version) async {
    // The video only address goes with the version it shows.
    final sure = await _ask(
        l10n.reviewDeleteVersionAsk(version.n), l10n.delete,
        note: review.clean != null && review.cleanVersion == version.n
            ? l10n.reviewDeleteVersionCleanNote
            : l10n.reviewDeleteVersionNote);
    if (!sure) return;
    if (_version == version.n) _version = null;
    await _do(() => _reviews.deleteVersion(review, version));
  }

  Future<void> _deleteComment(Review review, ReviewComment comment) async {
    final replies = _reviews.comments.any((c) => c.parent == comment.id);
    final sure = await _ask(
        comment.reaction == null
            ? l10n.reviewDeleteCommentAsk
            : l10n.reviewDeleteLikeAsk,
        l10n.delete,
        note: replies ? l10n.reviewDeleteRepliesNote : null);
    if (sure) await _do(() => _reviews.deleteComment(review, comment));
  }

  Future<void> _removeClean(Review review) async {
    final sure = await _ask(l10n.reviewCleanRemoveAsk, l10n.reviewRemove,
        note: l10n.reviewCleanRemoveNote);
    if (sure) await _do(() => _reviews.removeClean(review));
  }

  Future<void> _removeApproval(Review review, ReviewApproval approval) async {
    final sure = await _ask(
        l10n.reviewRemoveApprovalAsk(approval.name), l10n.reviewRemove);
    if (sure) await _do(() => _reviews.removeApproval(review, approval));
  }

  void _sendReply(Review review, ReviewComment to) {
    final text = _reply.text.trim();
    setState(() => _replyTo = null);
    _focus.requestFocus();
    if (text.isEmpty) return;
    _reply.clear();
    unawaited(_do(() => _reviews.reply(review, to, text)));
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final send = _reviews.sending;
    final busy = send != null && send.stage != SendStage.failed;
    // Sending is what needs Lumit Pro. An account that had it once still has
    // its links, and can read and delete them. A send under way when Pro
    // ended stays in sight until it lands or is refused.
    if (!widget.ui.account.pro &&
        !busy &&
        (_reviews.links.isEmpty || _sendFor != null)) {
      return _offer(t);
    }
    if (_sendFor != null || busy) {
      return ReviewSendFace(
        ui: widget.ui,
        app: widget.app,
        comp: _sendFor,
        link: _sendInto,
        // Opened straight on the form, there is no list behind it to go
        // back to.
        onBack: widget.send != null
            ? widget.close
            : () {
                _reviews.clearSend();
                setState(() => _sendFor = _sendInto = null);
              },
        onClose: widget.close,
      );
    }
    return _reviews.links.isEmpty ? _none(t) : _links(t);
  }

  Widget _titleBar(LumitTheme t) => dialogTitleBar(
        t,
        title: l10n.reviewTitle,
        onClose: widget.close,
        keyPrefix: 'review',
      );

  /// The button that turns the window to its other face, for the composition
  /// in front. The one filled action here. Without Lumit Pro it is the way
  /// to that instead.
  Widget _sendButton(LumitTheme t) {
    final comp = widget.ui.selectedComp;
    return dialogFilledAction(
      t,
      key: const ValueKey('review-send'),
      label: l10n.menuSendForReview,
      onPressed: !widget.ui.account.pro
          ? () => unawaited(showProWindow(context))
          : comp == null
              ? null
              : () => setState(() {
                    _sendFor = comp;
                    _sendInto = null;
                  }),
    );
  }

  String? get _failure =>
      _error ??
      (_reviews.failure == null ? null : reviewErrorText(_reviews.failure!));

  // --- Without Pro, and with no links yet -----------------------------------

  Widget _offer(LumitTheme t) => DialogFrame(
        width: reviewNarrowWidth,
        children: [
          _titleBar(t),
          ProOffer(
            headline: l10n.reviewProHeadline,
            preview: widget.ui.account.config?.preview ?? false,
          ),
        ],
      );

  Widget _none(LumitTheme t) {
    final failure = _failure;
    return DialogFrame(
      width: reviewNarrowWidth,
      children: [
        _titleBar(t),
        Padding(
          padding: const EdgeInsets.fromLTRB(22, 28, 22, 22),
          child: !_reviews.loaded && failure == null
              ? const Center(
                  child: SizedBox(width: 20, height: 20, child: SyncRing()))
              : Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Text(
                      failure ?? l10n.reviewEmpty,
                      key: const ValueKey('review-empty'),
                      textAlign: TextAlign.center,
                      style: t.body.copyWith(
                          color: failure == null ? t.textSecondary : t.warning,
                          height: 1.45),
                    ),
                    const SizedBox(height: 20),
                    SizedBox(height: 30, child: _sendButton(t)),
                  ],
                ),
        ),
      ],
    );
  }

  // --- The links -------------------------------------------------------------

  Widget _links(LumitTheme t) {
    final review = _reviews.link(_chosen);
    return Focus(
      focusNode: _focus,
      autofocus: true,
      onKeyEvent: _key,
      child: DialogFrame(
        width: reviewWindowWidth,
        children: [
          _titleBar(t),
          SizedBox(
            height: _bodyHeight,
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                SizedBox(
                  width: _listWidth,
                  child: ReviewScroll(
                    controller: _listScroll,
                    ground: t.surface1,
                    child: Padding(
                      padding:
                          const EdgeInsets.symmetric(vertical: _listMargin),
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.stretch,
                        children: [
                          for (final link in _reviews.links)
                            Builder(
                              key: ValueKey<String>('review-row-${link.id}'),
                              builder: (row) => ReviewLinkRow(
                                link: link,
                                chosen: link.id == _chosen,
                                unseen: _reviews.unseenIn(link) > 0,
                                moved: _moved.contains(link.id),
                                // The keys come back to the list with it.
                                onPick: () {
                                  _focus.requestFocus();
                                  setState(() => link.id == _chosen
                                      ? _at = null
                                      : _choose(link.id));
                                },
                                onMenu: (at) => _linkMenu(row, link, at),
                              ),
                            ),
                        ],
                      ),
                    ),
                  ),
                ),
                Container(width: 1, color: t.hairline),
                Expanded(
                  child: review == null
                      ? const SizedBox.shrink()
                      : _detail(t, review),
                ),
              ],
            ),
          ),
          dialogFooter(
            t,
            // How much of the account's room on the server is taken.
            summary: _reviews.limitBytes <= 0
                ? ''
                : l10n.reviewStorage(writtenBytes(_reviews.usedBytes),
                    writtenBytes(_reviews.limitBytes)),
            keyPrefix: 'review',
            actions: [_sendButton(t)],
          ),
        ],
      ),
    );
  }

  // --- The chosen link -----------------------------------------------------

  /// The chosen link: its still and its address, which stay put, over the
  /// two pages that scroll under them. The comments are the page it opens on,
  /// since they are what the window is opened for.
  Widget _detail(LumitTheme t, Review review) {
    final version = _versionOf(review);
    final failure = _failure;
    // Comments on frames, which are what there is to filter and to add.
    final said = [
      for (final c in _reviews.comments)
        if (c.version == version.n && c.parent == null && c.reaction == null)
          c,
    ];
    // Nothing on this version at all: no comment, no like, nobody's yes.
    final bare = version.approvals.isEmpty &&
        !_reviews.comments.any((c) => c.version == version.n);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        ReviewStill(
          review: review,
          version: version,
          onVersion: (n) => setState(() {
            _version = n;
            _replyTo = _drawingOf = _at = null;
            _found();
          }),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 10),
          child: ReviewAddress(
              review: review,
              onSendToFriend: (button) => _pickFriendUnder(button, review)),
        ),
        if (failure != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
            child: Text(failure,
                key: const ValueKey('review-error'),
                style: t.small.copyWith(color: t.warning, height: 1.4)),
          ),
        // The composition has moved on since this went out, and the next
        // version is one press away.
        if (_moved.contains(review.id) && _comp != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 8),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                      l10n.reviewChangedSince(
                          l10n.reviewVersion(review.versions.last.n)),
                      key: const ValueKey('review-changed-since'),
                      style: t.small.copyWith(color: t.textSecondary)),
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: const ValueKey('review-send-version'),
                  small: true,
                  onPressed: !widget.ui.account.pro
                      ? () => unawaited(showProWindow(context))
                      : () => setState(() {
                            _sendFor = _comp;
                            _sendInto = review;
                          }),
                  child: Text(l10n.reviewSendNewVersion, style: t.small),
                ),
              ],
            ),
          ),
        dialogTabs<_Page>(
          t,
          tabs: [
            (_Page.comments, l10n.reviewComments),
            (_Page.link, l10n.reviewPageLink),
          ],
          current: _page,
          onPick: (page) {
            // A page opens at its top, wherever the other was scrolled to.
            if (_detailScroll.hasClients) _detailScroll.jumpTo(0);
            setState(() {
              _page = page;
              _at = null;
            });
          },
          keyPrefix: 'review',
        ),
        // What is shown and the way onto the Timeline stay put over the
        // comments, and are not there at all until there is one.
        if (_page == _Page.comments && said.isNotEmpty)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 10, 16, 0),
            child: _commentsHead(t, review, version, said),
          ),
        Expanded(
          child: _page == _Page.comments && bare
              // With nothing said and nobody's yes, the one line there is to
              // say stands in the middle of the room, once the server has
              // said there is nothing, and nothing scrolls.
              ? Center(
                  child: _reviews.reading
                      ? null
                      : Text(l10n.reviewNoComments,
                          key: const ValueKey('review-no-comments'),
                          style: t.small),
                )
              : ReviewScroll(
                  controller: _detailScroll,
                  ground: t.surface1,
                  child: Padding(
                    padding: const EdgeInsets.fromLTRB(16, 8, 16, 12),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: _page == _Page.comments
                          ? _commentsPage(t, review, version)
                          : [
                              ReviewLinkPage(
                                key: ValueKey<String>(
                                    'review-page-${review.id}'),
                                reviews: _reviews,
                                review: review,
                                run: _do,
                                onRefused: (why) =>
                                    setState(() => _error = why),
                                onDeleteVersion: (gone) =>
                                    unawaited(_deleteVersion(review, gone)),
                                onDeleteLink: () =>
                                    unawaited(_deleteLink(review)),
                                onCopy: _copy,
                                onRemoveClean: () =>
                                    unawaited(_removeClean(review)),
                              ),
                            ],
                    ),
                  ),
                ),
        ),
      ],
    );
  }

  List<Widget> _commentsPage(
      LumitTheme t, Review review, ReviewVersion version) {
    final all = [
      for (final c in _reviews.comments)
        if (c.version == version.n) c,
    ];
    final listed = _listed(version);
    // ponytail: every listed comment is built at once, which a few hundred
    // are fine with. A link at the server's ceiling of 2000 wants a list
    // that builds what is in view, and the keys a way to find a row in it.
    return [
      if (_comp == null && listed.isNotEmpty)
        Padding(
          padding: const EdgeInsets.only(bottom: 8),
          child: Text(l10n.reviewNoComp, style: t.small),
        ),
      // A reviewer's yes to this version, before what anyone said about it.
      if (version.approvals.isNotEmpty) ...[
        if (listed.isNotEmpty || _comp == null)
          Container(height: 1, color: t.hairline),
        ReviewApprovedRow(
          key: ValueKey<String>('review-approved-by-${version.n}'),
          approvals: version.approvals,
          onRemove: (approval) =>
              unawaited(_removeApproval(review, approval)),
        ),
      ],
      if (listed.isEmpty)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 20),
          child: Text(
            all.any((c) => c.parent == null && c.reaction == null)
                ? l10n.reviewNoOpenComments
                : l10n.reviewNoComments,
            key: const ValueKey('review-no-comments'),
            textAlign: TextAlign.center,
            style: t.small,
          ),
        ),
      for (final (index, item) in listed.indexed)
        if (item is ReviewComment)
          ..._thread(t, review, version, item, all, first: index == 0)
        else if (item is List<ReviewComment>) ...[
          if (index > 0 || version.approvals.isNotEmpty)
            Container(height: 1, color: t.hairline),
          KeyedSubtree(
            key: _rowKeys.putIfAbsent(item.first.id, GlobalKey.new),
            child: ReviewLikeRow(
              key: ValueKey<String>('review-like-${item.first.id}'),
              likes: item,
              arrived: item.any((c) => _reviews.arrived.contains(c.id)),
              chosen: _at == item.first.id,
              timecode: timecodeOfRate(version.startFrame + item.first.frame,
                  version.fpsNum, version.fpsDen),
              onChoose: () => _onComment(item.first.id),
              onJump: _comp == null ? null : () => _jump(version, item.first),
              onDelete: item.length > 1
                  ? null
                  : () => unawaited(_deleteComment(review, item.first)),
            ),
          ),
        ],
    ];
  }

  /// Which comments are listed, with how many there are of each, and the
  /// way onto the Timeline. [said] is the version's comments on frames.
  Widget _commentsHead(LumitTheme t, Review review, ReviewVersion version,
          List<ReviewComment> said) =>
      Row(
        children: [
          for (final all in const [false, true]) ...[
            HouseButton(
              key: ValueKey<String>('review-filter-${all ? 'all' : 'open'}'),
              small: true,
              active: _all == all,
              onPressed: () => setState(() {
                _all = all;
                _at = null;
              }),
              // The one in force takes the button's own ink, which on a
              // filled pill is not the muted one.
              child: Text(
                  '${all ? l10n.reviewFilterAll : l10n.reviewFilterOpen}  '
                  '${said.where((c) => all || !c.resolved).length}',
                  style: _all == all
                      ? TextStyle(fontSize: t.small.fontSize)
                      : t.small),
            ),
            const SizedBox(width: 4),
          ],
          const Spacer(),
          HouseButton(
            key: const ValueKey('review-add'),
            small: true,
            onPressed: _pending > 0 ? () => _add(review, version) : null,
            child: Text(l10n.reviewAddToTimeline, style: t.small),
          ),
        ],
      );

  /// One comment on a frame, the replies under it, and the field a reply is
  /// typed in while one is being written. [first] is whether nothing is
  /// listed over it, in which case there is no rule over it either.
  List<Widget> _thread(LumitTheme t, Review review, ReviewVersion version,
      ReviewComment top, List<ReviewComment> all,
      {required bool first}) {
    final frame = version.startFrame + top.frame;
    final end = top.endFrame;
    return [
      if (!first || version.approvals.isNotEmpty)
        Container(height: 1, color: t.hairline),
      KeyedSubtree(
        key: _rowKeys.putIfAbsent(top.id, GlobalKey.new),
        child: ReviewCommentRow(
          key: ValueKey<String>('review-comment-${top.id}'),
          review: review,
          comment: top,
          mentions: _mentions(review, top),
          onPicture: () => unawaited(_showPicture(review, top)),
          arrived: _reviews.arrived.contains(top.id),
          chosen: _at == top.id,
          timecode: timecodeOfRate(frame, version.fpsNum, version.fpsDen),
          until: end != null && end > top.frame
              ? timecodeOfRate(version.startFrame + end, version.fpsNum,
                  version.fpsDen)
              : null,
          layer: _layers[top.id]?.info.name,
          // What was drawn, over its frame, as large as the column and the
          // window's height leave room for.
          drawing: _drawingOf != top.id
              ? null
              : ConstrainedBox(
                  constraints: BoxConstraints(maxHeight: _drawingRoom),
                  child: AspectRatio(
                    aspectRatio: math.max(1, version.width) /
                        math.max(1, version.height),
                    child: ReviewDrawing(
                      strokes: top.strokes,
                      frame: _frame,
                      aspect: math.max(1, version.width) /
                          math.max(1, version.height),
                      pin: t.labelColour(commenterColour(top.name)),
                    ),
                  ),
                ),
          onChoose: () => _onComment(top.id),
          onJump: _comp == null ? null : () => _jump(version, top),
          onResolve: () => unawaited(
              _do(() => _reviews.resolve(review, top, !top.resolved))),
          onReply: () => setState(() {
            _reply.clear();
            _replyTo = top.id;
          }),
          onDrawing: () => _toggleDrawing(version, top),
          onCopyLink: () => _copy(review.commentUrl(top)),
          onDelete: () => unawaited(_deleteComment(review, top)),
        ),
      ),
      for (final reply in all)
        if (reply.parent == top.id)
          Padding(
            padding: const EdgeInsets.only(left: 14),
            child: ReviewCommentRow(
              key: ValueKey<String>('review-comment-${reply.id}'),
              review: review,
              comment: reply,
              mentions: _mentions(review, reply),
              onPicture: () => unawaited(_showPicture(review, reply)),
              arrived: _reviews.arrived.contains(reply.id),
              onDelete: () => unawaited(_deleteComment(review, reply)),
            ),
          ),
      if (_replyTo == top.id)
        Padding(
          padding: const EdgeInsets.fromLTRB(14, 0, 0, 8),
          child: SizedBox(
            height: dialogControlHeight,
            child: HouseTextField(
              key: const ValueKey('review-reply'),
              controller: _reply,
              width: double.infinity,
              padding: const EdgeInsets.symmetric(horizontal: 8),
              fill: t.surface0,
              autofocus: true,
              hint: l10n.reviewReply,
              onSubmitted: (_) => _sendReply(review, top),
            ),
          ),
        ),
    ];
  }
}

/// The button review links put on the status line while there is something
/// to say: how many comments nobody has looked at, or a ring while a
/// composition is on its way up. It opens the window on the newest of them.
class ReviewChip extends StatelessWidget {
  const ReviewChip({super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final reviews = context.read<LumitUiState>().reviews;
    Widget swept(Widget child) => ShaderMask(
          blendMode: BlendMode.srcIn,
          shaderCallback: (bounds) => proSweep.createShader(bounds),
          child: child,
        );
    return ListenableBuilder(
      listenable: reviews,
      builder: (context, _) {
        final unseen = reviews.unseen;
        final stage = reviews.sending?.stage;
        return LumitTooltip(
          message: l10n.reviewTitle,
          child: HouseButton(
            key: const ValueKey('status-review'),
            frameless: true,
            small: true,
            padding: const EdgeInsets.symmetric(horizontal: 4),
            onPressed: () => unawaited(
                showReviewLinks(context, link: reviews.newestUnseen?.id)),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (stage == SendStage.exporting ||
                    stage == SendStage.uploading)
                  const SizedBox(width: 12, height: 12, child: SyncRing())
                else if (stage == SendStage.failed)
                  glyph.LumitIcon(LumitIcons.comment,
                      size: 14, colour: t.warning)
                else
                  swept(glyph.LumitIcon(LumitIcons.comment,
                      size: 14, colour: t.textPrimary)),
                // The sweep is the mark's. The count is said plainly.
                if (unseen > 0) ...[
                  const SizedBox(width: 4),
                  Text('$unseen',
                      key: const ValueKey('status-review-count'),
                      style: t.small.copyWith(
                          color: t.textPrimary, fontWeight: FontWeight.w500)),
                ],
              ],
            ),
          ),
        );
      },
    );
  }
}
