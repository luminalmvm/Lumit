// Review links: a composition exported, kept on Lumit's server, and watched
// in a browser by anyone with its address, who can comment on a frame.
//
// This holds the account's links, sends a composition as a new link or as a
// new version of one, and keeps the comments of the link being looked at in
// step over a socket. It speaks to the same server as [AccountState], as the
// signed-in account, and nothing here is on a rebuild path.
//
// A few things are remembered between runs, in a small file beside the
// profile's settings: which composition a link came from, how much of it has
// been seen, and which comments were made into markers. None of that goes in
// the project, which has no business knowing where it was sent.

import 'dart:async';
import 'dart:collection';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;
import 'dart:ui' show Offset;

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart' show Clipboard, ClipboardData;
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:uuid/uuid.dart';

import 'account.dart';
import 'comp_time.dart';
import 'profiles.dart';
import 'transfer.dart';
import 'workspace.dart';

int _int(Object? v) => v is num ? v.toInt() : 0;
int? _intOrNull(Object? v) => v is num ? v.toInt() : null;
String _text(Object? v) => v is String ? v : '';

/// One upload of a link's video.
class ReviewVersion {
  final int n, size, width, height, fpsNum, fpsDen, frames, created;

  /// The composition frame the video's first frame was exported from.
  final int startFrame;

  /// False while it is still being sent.
  final bool ready;
  final bool poster;

  /// The composition's name when it was sent, and what its sender said had
  /// changed.
  final String comp, note;

  /// The reviewers who have said yes to it, oldest first.
  final List<ReviewApproval> approvals;

  const ReviewVersion({
    required this.n,
    this.size = 0,
    this.width = 0,
    this.height = 0,
    this.fpsNum = 30,
    this.fpsDen = 1,
    this.frames = 0,
    this.startFrame = 0,
    this.created = 0,
    this.ready = true,
    this.poster = false,
    this.comp = '',
    this.note = '',
    this.approvals = const [],
  });

  /// The same version with [approvals] in place of its own.
  ReviewVersion approvedBy(List<ReviewApproval> approvals) => ReviewVersion(
        n: n,
        size: size,
        width: width,
        height: height,
        fpsNum: fpsNum,
        fpsDen: fpsDen,
        frames: frames,
        startFrame: startFrame,
        created: created,
        ready: ready,
        poster: poster,
        comp: comp,
        note: note,
        approvals: approvals,
      );

  factory ReviewVersion.from(Map<String, dynamic> j) => ReviewVersion(
        n: _int(j['n']),
        size: _int(j['size']),
        width: _int(j['width']),
        height: _int(j['height']),
        fpsNum: _int(j['fps_num']),
        fpsDen: _int(j['fps_den']),
        frames: _int(j['frames']),
        startFrame: _int(j['start_frame']),
        created: _int(j['created']),
        ready: j['state'] == 'ready',
        poster: j['poster'] == true,
        comp: _text(j['comp']),
        note: _text(j['note']),
        approvals: ReviewApproval.listFrom(j['approvals']),
      );
}

/// What is different in a composition from when its link's newest version
/// was sent, by name, the layers of the compositions it places among them.
class ReviewChanges {
  /// The names of the layers that are no longer the same, that are new,
  /// and that have gone.
  final List<String> changed = [], added = [], removed = [];

  /// Each effect that is new on a layer that was there before, or gone from
  /// one that still is: the effect, then the layer. An effect is called as
  /// the marks have it.
  final List<(String, String)> effectsAdded = [], effectsRemoved = [];

  bool get none =>
      changed.isEmpty &&
      added.isEmpty &&
      removed.isEmpty &&
      effectsAdded.isEmpty &&
      effectsRemoved.isEmpty;
}

/// A reviewer's yes to one version. It is not a comment: it has no frame
/// and nothing to resolve, and is in none of a link's counts.
class ReviewApproval {
  final String id, name;
  final int created;
  const ReviewApproval({required this.id, this.name = '', this.created = 0});

  /// A version's approvals as the server lists them, oldest first.
  static List<ReviewApproval> listFrom(Object? j) => [
        if (j is List)
          for (final a in j)
            if (a is Map<String, dynamic>)
              ReviewApproval(
                  id: _text(a['id']),
                  name: _text(a['name']),
                  created: _int(a['created'])),
      ];
}

/// A review link as its owner sees it.
class Review {
  final String id, url, title;

  /// What tells the page in a browser that its owner is looking.
  final String ownerKey;

  /// The username of the account that made it, or null when that account
  /// has not chosen one.
  final String? ownerUsername;

  /// Whether reviewers may comment, may attach a picture to a comment, and
  /// may download the file.
  final bool comments, pictures, downloads;

  /// Whether a password is set. The password itself never comes back.
  final bool password;

  /// Whether its maker is emailed about new comments on it. On until it is
  /// turned off, and only ever said to its maker.
  final bool mail;

  /// The id of its second address, which shows the video and nothing of the
  /// review, and the one version that address shows. Null until one is
  /// made, and only ever said to its maker.
  final String? clean;
  final int? cleanVersion;

  /// When it stops working and when it was last commented on, in seconds
  /// since 1970, or null for never.
  final int? expires, lastComment;
  final int updated, commentCount, openCount;

  /// How many likes it has, which the three counts above leave out.
  final int likeCount;

  /// The versions that are ready to watch, oldest first.
  final List<ReviewVersion> versions;

  const Review({
    required this.id,
    required this.url,
    required this.title,
    this.ownerKey = '',
    this.ownerUsername,
    this.comments = true,
    this.pictures = true,
    this.downloads = false,
    this.password = false,
    this.mail = true,
    this.clean,
    this.cleanVersion,
    this.expires,
    this.lastComment,
    this.updated = 0,
    this.commentCount = 0,
    this.openCount = 0,
    this.likeCount = 0,
    this.versions = const [],
  });

  factory Review.from(Map<String, dynamic> j) => Review(
        id: _text(j['id']),
        url: _text(j['url']),
        title: _text(j['title']),
        ownerKey: _text(j['owner_key']),
        ownerUsername: switch (j['owner']) {
          {'username': final String username} => username,
          _ => null,
        },
        comments: j['comments'] != false,
        pictures: j['pictures'] != false,
        downloads: j['downloads'] == true,
        password: j['password'] == true,
        mail: j['mail'] != false,
        clean: j['clean'] is String ? j['clean'] as String : null,
        cleanVersion: _intOrNull(j['clean_version']),
        expires: _intOrNull(j['expires']),
        lastComment: _intOrNull(j['last_comment']),
        updated: _int(j['updated']),
        commentCount: _int(j['comment_count']),
        openCount: _int(j['open_count']),
        likeCount: _int(j['like_count']),
        versions: [
          if (j['versions'] case final List<dynamic> all)
            for (final v in all)
              if (v is Map<String, dynamic> && v['state'] == 'ready')
                ReviewVersion.from(v),
        ],
      );

  /// The same link with [versions] in place of its own, or with [clean] as
  /// its second address and the version that shows.
  Review changed({List<ReviewVersion>? versions, (String?, int?)? clean}) =>
      Review(
        id: id,
        url: url,
        title: title,
        ownerKey: ownerKey,
        ownerUsername: ownerUsername,
        comments: comments,
        pictures: pictures,
        downloads: downloads,
        password: password,
        mail: mail,
        clean: clean == null ? this.clean : clean.$1,
        cleanVersion: clean == null ? cleanVersion : clean.$2,
        expires: expires,
        lastComment: lastComment,
        updated: updated,
        commentCount: commentCount,
        openCount: openCount,
        likeCount: likeCount,
        versions: versions ?? this.versions,
      );

  /// The second address, which is the first with another id at its end.
  String? get cleanUrl => clean == null
      ? null
      : '${url.substring(0, url.lastIndexOf('/') + 1)}$clean';

  /// Every approval on it, whichever version it is of.
  Iterable<ReviewApproval> get approvals =>
      versions.expand((v) => v.approvals);

  /// When something was last said on it, a comment or an approval.
  int get lastSaid => approvals.fold(
      lastComment ?? 0, (at, approval) => math.max(at, approval.created));

  /// The address that opens it in a browser as whoever made it.
  String get ownerUrl => ownerKey.isEmpty ? url : '$url#o=$ownerKey';

  /// The address that opens the page on [comment]'s version, at its frame.
  String commentUrl(ReviewComment comment) =>
      '$url?v=${comment.version}&t=${comment.frame}';

  /// The address that shows version [n] beside [before], the one it took
  /// over from, as whoever made the link.
  String compareUrl(int before, int n) =>
      '$url?compare=$before,$n${ownerKey.isEmpty ? '' : '#o=$ownerKey'}';

  /// Where a version's still is kept.
  String posterUrl(ReviewVersion version) =>
      '$cloudUrl/v1/reviews/$id/v/${version.n}/poster';

  /// Where the picture [comment] came with is kept.
  String pictureUrl(ReviewComment comment) =>
      '$cloudUrl/v1/reviews/$id/comments/${comment.id}/picture';
}

/// One mark drawn over a frame: a pen line through its points, or a line, an
/// arrow, a box or a pin. Every point is from 0 to 1 of the picture, x then
/// y, from its top left.
class ReviewStroke {
  /// `line`, `arrow`, `box` or `pin`, and null for the pen. A pin is one
  /// point and the other three are two.
  final String? shape;

  /// Its colour as 0xRRGGBB, and its width as a share of the picture's.
  final int colour;
  final double width;
  final List<Offset> points;

  const ReviewStroke({
    this.shape,
    this.colour = 0xffffff,
    this.width = 0.004,
    required this.points,
  });

  static ReviewStroke? from(Object? j) {
    if (j is! Map) return null;
    final points = [
      if (j['points'] case final List<dynamic> all)
        for (final p in all)
          if (p case [final num x, final num y, ...])
            Offset(x.toDouble(), y.toDouble()),
    ];
    if (points.isEmpty) return null;
    final colour = j['colour'];
    return ReviewStroke(
      shape: j['shape'] is String ? j['shape'] as String : null,
      colour: colour is String && colour.length == 7
          ? int.tryParse(colour.substring(1), radix: 16) ?? 0xffffff
          : 0xffffff,
      width: j['width'] is num ? (j['width'] as num).toDouble() : 0.004,
      points: points,
    );
  }
}

/// Somebody who opened a link in a browser: the name they gave, if they gave
/// one, the version they were on, and the first and last time.
class ReviewView {
  final String name;
  final int version, first, last;
  const ReviewView(
      {this.name = '', this.version = 1, this.first = 0, this.last = 0});
}

/// A comment on a frame of one version, a reply to one, or a like.
class ReviewComment {
  final String id, name, text;

  /// The comment this replies to, or null for one on a frame.
  final String? parent;
  final int version, created;

  /// The frame of the video it is on, and the last frame when it is a range.
  final int frame;
  final int? endFrame;

  /// Whether the link's owner wrote it, and whether it has been dealt with.
  final bool owner, resolved;

  /// What was drawn over the frame with it. Empty for most.
  final List<ReviewStroke> strokes;

  /// `like` for a mark with no words, and null for a comment. A like is
  /// never a note to act on: it is not resolved, replied to or made a marker.
  final String? reaction;

  /// How large the picture it came with is, in pixels, or null for none. A
  /// comment with one may have no words.
  final ({int w, int h})? picture;

  const ReviewComment({
    required this.id,
    required this.name,
    required this.text,
    this.parent,
    this.version = 1,
    this.created = 0,
    this.frame = 0,
    this.endFrame,
    this.owner = false,
    this.resolved = false,
    this.strokes = const [],
    this.reaction,
    this.picture,
  });

  bool get drawn => strokes.isNotEmpty;

  /// The place on the picture it is about: its pin, or failing that the head
  /// of an arrow, or the middle of a box.
  Offset? get point {
    ReviewStroke? of(String shape) =>
        strokes.where((s) => s.shape == shape).firstOrNull;
    if (of('pin') case final pin?) return pin.points.first;
    if (of('arrow') case final arrow?) return arrow.points.last;
    if (of('box') case final box?) {
      return (box.points.first + box.points.last) / 2;
    }
    return null;
  }

  factory ReviewComment.from(Map<String, dynamic> j) => ReviewComment(
        id: _text(j['id']),
        name: _text(j['name']),
        text: _text(j['text']),
        parent: j['parent'] is String ? j['parent'] as String : null,
        version: _int(j['version']),
        created: _int(j['created']),
        frame: _int(j['frame']),
        endFrame: _intOrNull(j['end_frame']),
        owner: j['owner'] == true,
        resolved: j['resolved'] == true,
        strokes: [
          if (j['drawing'] case {'strokes': final List<dynamic> all})
            for (final stroke in all)
              if (ReviewStroke.from(stroke) case final made?) made,
        ],
        reaction: j['reaction'] is String ? j['reaction'] as String : null,
        picture: switch (j['picture']) {
          {'w': final int w, 'h': final int h} when w > 0 && h > 0 =>
            (w: w, h: h),
          _ => null,
        },
      );
}

final RegExp _letterOrDigit = RegExp(r'[\p{L}\p{N}]', unicode: true);

/// Where [text] mentions somebody, in order: an `@` and one of [names] at
/// once after it, whatever its capitals, with no letter or digit before the
/// `@` or after the name. Where one name begins another, the longer is the
/// mention. [me] is the name of whoever is reading, to know their own by.
/// The page in the browser reads a mention the same way.
List<({int start, int end, bool me})> reviewMentions(
    String text, Iterable<String> names,
    {String me = ''}) {
  final lower = text.toLowerCase();
  // ponytail: the few letters whose small form is longer are not searched,
  // since a place in one string would not be the same place in the other.
  if (lower.length != text.length || !text.contains('@')) return const [];
  final known = {
    for (final name in names)
      if (name.trim().isNotEmpty) name.trim().toLowerCase(),
  }.toList()
    ..sort((a, b) => b.length.compareTo(a.length));
  bool word(int at) =>
      at >= 0 && at < lower.length && _letterOrDigit.hasMatch(lower[at]);
  final found = <({int start, int end, bool me})>[];
  for (var at = lower.indexOf('@'); at >= 0; at = lower.indexOf('@', at + 1)) {
    if (word(at - 1)) continue;
    for (final name in known) {
      final end = at + 1 + name.length;
      if (!lower.startsWith(name, at + 1) || word(end)) continue;
      found.add((start: at, end: end, me: name == me.trim().toLowerCase()));
      at = end - 1;
      break;
    }
  }
  return found;
}

/// Where a send has got to.
enum SendStage { exporting, uploading, done, failed }

/// A composition on its way to becoming a link, for the window to draw.
class ReviewSend {
  SendStage stage;

  /// Frames exported of the frames there are, then bytes sent of the file.
  int done, total;

  /// Whether it is in the export queue behind something else.
  bool waiting;

  /// The link, once it has landed.
  Review? review;

  /// Why it failed: the server's code, or one of this file's own.
  String? failure;

  ReviewSend({
    this.stage = SendStage.exporting,
    this.done = 0,
    this.total = 0,
    this.waiting = false,
    this.review,
    this.failure,
  });
}

/// A send that stopped, and the code for why. `cancelled` is the person's own
/// doing and is not shown.
class _Stopped implements Exception {
  final String code;
  const _Stopped(this.code);
}

/// What a refusal is filed under: offline and too-many-tries by what they
/// are, anything else by the server's own word for it.
String reviewFailure(Object e) => switch (e) {
      _Stopped(:final code) => code,
      CloudError(offline: true) => 'offline',
      CloudError(status: 429) => 'slow_down',
      CloudError(:final code) => code,
      _ => 'unknown',
    };

/// The size a review is exported at: the composition's shape fitted inside
/// 1080p or 720p turned the way the composition is, never larger than it
/// was, and even both ways as H.264 needs.
(int, int) reviewSize(int width, int height, int edge) {
  final long = edge * 16 ~/ 9;
  final wide = width >= height;
  final scale = math.min(
      1.0,
      math.min((wide ? long : edge) / math.max(width, 1),
          (wide ? edge : long) / math.max(height, 1)));
  int even(double v) => math.max(2, (v / 2).round() * 2);
  return (even(width * scale), even(height * scale));
}

/// Whether [path] is a file this wrote on its way to the server, which the
/// status line has no reason to offer as a finished export.
bool isReviewExport(String path) => path.contains(_tempPrefix);
const String _tempPrefix = 'lumit-review-';

/// The label colour a commenter gets, the same every time from their name.
int commenterColour(String name) =>
    1 +
    name.trim().toLowerCase().codeUnits.fold(0, (h, c) => (h * 31 + c) % 8009) %
        8;

final RegExp _lineBreaks = RegExp(r'\s*[\r\n]+\s*');

/// A marker some comments come to, before the engine has given it a time.
typedef CommentMarker = ({
  int frame,
  int? duration,
  String label,
  int colour,
  List<String> ids,

  /// The layer it is put on, by its id, or null for the composition's own.
  String? layer,
});

/// The markers [comments] come to on a composition of [compFrames] frames,
/// for a video whose first frame was the composition's [startFrame].
///
/// Only open comments on a frame count. Replies, likes and resolved ones are
/// left out. [layers] is the id of the layer each comment points at, for
/// those that point at one: such a comment's marker is that layer's own, so
/// it moves when the layer does, and every other is the composition's. Each
/// holds one marker to a frame, so comments that share a frame and a place
/// share a marker, and one whose comments are all in [added] is skipped. A
/// marker's label is one line, so line breaks become spaces, and one for a
/// picture with no words is the name alone. The frame is the composition's
/// in both cases.
List<CommentMarker> commentMarkers(
  Iterable<ReviewComment> comments, {
  required int startFrame,
  required int compFrames,
  Set<String> added = const {},
  Map<String, String> layers = const {},
}) {
  final last = math.max(0, compFrames - 1);
  // By frame, then the composition's before any layer's.
  final byPlace = SplayTreeMap<(int, String), List<ReviewComment>>((a, b) =>
      a.$1 != b.$1 ? a.$1.compareTo(b.$1) : a.$2.compareTo(b.$2));
  for (final c in comments) {
    if (c.parent != null || c.resolved || c.reaction != null) continue;
    byPlace.putIfAbsent(
        ((startFrame + c.frame).clamp(0, last), layers[c.id] ?? ''),
        () => []).add(c);
  }
  return [
    for (final MapEntry(key: (frame, layer), value: group) in byPlace.entries)
      if (group.any((c) => !added.contains(c.id)))
        (
          frame: frame,
          // The longest range among them, cut off at the composition's end.
          duration: [
            for (final c in group)
              if (c.endFrame case final end? when end > c.frame)
                math.min(end - c.frame + 1, last + 1 - frame),
          ].fold<int?>(null, (a, b) => a == null || b > a ? b : a),
          label: [
            for (final c in group)
              [c.name, c.text.replaceAll(_lineBreaks, ' ').trim()]
                  .where((part) => part.isNotEmpty)
                  .join(': '),
          ].join(' · '),
          colour: commenterColour(group.first.name),
          ids: [for (final c in group) c.id],
          layer: layer.isEmpty ? null : layer,
        ),
  ];
}

/// The account's review links, the one being looked at, and the one on its
/// way up.
class ReviewsState extends ChangeNotifier {
  final AccountState account;
  final ProfilesState profiles;

  ReviewsState(this.account, this.profiles);

  /// Every link that has something to watch, newest activity first.
  List<Review> links = [];

  /// What the account's files come to on the server, and the most it may.
  int usedBytes = 0, limitBytes = 0;

  /// Whether the server has answered with the list since this account was
  /// signed in, and why the last call for it failed if it did.
  bool loaded = false;
  String? failure;

  /// The link whose comments are held, oldest first, and the ones that came
  /// in while it was being looked at.
  String? watching;
  List<ReviewComment> comments = [];
  final Set<String> arrived = {};

  /// How many times the line has changed the link on screen, for a list
  /// that was asked for before one of them to know it is behind.
  int _news = 0;

  /// Whether that link's comments are still being asked for, so that none
  /// yet is not taken for none at all.
  bool reading = false;

  /// The send under way, or the one that just ended.
  ReviewSend? sending;

  Timer? _beat, _retry, _settle;
  WebSocket? _live;
  HttpClient? _upload;
  bool _started = false, _gone = false, _cancel = false;
  String? _who;

  /// Begin talking to the server: follow the account, and look for new
  /// comments now and then. Not called by a test, which keeps a test run
  /// off the network.
  void start() {
    if (_started) return;
    _started = true;
    account.addListener(_accountChanged);
    _beat = Timer.periodic(const Duration(minutes: 5), (_) {
      if (links.isNotEmpty || _kept.isNotEmpty) unawaited(refresh());
    });
    _accountChanged();
  }

  /// Another account, or none: what was held belonged to the last one.
  void _accountChanged() {
    final who = account.account?.id;
    if (who == _who) return;
    _who = who;
    links = [];
    usedBytes = limitBytes = 0;
    loaded = false;
    failure = null;
    unawaited(watch(null));
    notifyListeners();
    // Only an account known to have links is asked about them unprompted, and
    // one this profile has not met, which is asked once.
    if (who != null && (_kept.isNotEmpty || !_known.contains(who))) {
      unawaited(refresh());
    }
  }

  // --- What is remembered between runs --------------------------------------

  String? _keptFor;
  Map<String, dynamic> _keptLinks = {};

  /// The accounts whose links this profile has been shown before.
  List<String> _known = [];

  File get _file => File('${Workspace.storeFile().parent.path}'
      '${Platform.pathSeparator}profiles${Platform.pathSeparator}'
      '${profiles.current.id}.reviews.json');

  /// What is remembered about each link, by its id, for the profile in use.
  Map<String, dynamic> get _kept {
    final profile = profiles.current.id;
    if (_keptFor == profile) return _keptLinks;
    _keptFor = profile;
    _keptLinks = {};
    _known = [];
    try {
      final j = jsonDecode(_file.readAsStringSync());
      if (j case {'links': final Map<String, dynamic> links}) {
        _keptLinks = links;
        _known = [
          if (j['accounts'] case final List<dynamic> all)
            for (final id in all)
              if (id is String) id,
        ];
      }
    } catch (_) {
      // No file yet, or one that will not read.
    }
    return _keptLinks;
  }

  Map<String, dynamic> _keptOf(String id) {
    final held = _kept[id];
    if (held is Map<String, dynamic>) return held;
    return _kept[id] = <String, dynamic>{};
  }

  void _save() {
    try {
      _file.parent.createSync(recursive: true);
      _file.writeAsStringSync(const JsonEncoder.withIndent('  ')
          .convert({'accounts': _known, 'links': _kept}));
    } catch (_) {
      // A disk that will not take it forgets what was seen, and nothing else.
    }
  }

  /// The composition a link was sent from, by its id and its name, and the
  /// project that was open. Null for a link made on another machine.
  ({String comp, String name, String project})? originOf(Review review) {
    final held = _kept[review.id];
    if (held is! Map || held['comp'] is! String) return null;
    return (
      comp: held['comp'] as String,
      name: _text(held['comp_name']),
      project: _text(held['project']),
    );
  }

  /// The link [comp] was last sent to, if it has been sent.
  Review? linkOf(CompositionReference comp) {
    final id = '${comp.internalid}';
    for (final review in links) {
      if (originOf(review)?.comp == id) return review;
    }
    return null;
  }

  /// How many comments and approvals on [review] have come in since it was
  /// last looked at.
  int unseenIn(Review review) {
    final held = _kept[review.id];
    // An approval is in none of the server's counts, so the ones that have
    // been looked at are remembered by what they are called.
    final known = held is Map ? held['approvals'] : null;
    final approved = review.approvals
        .where((a) => known is! List || !known.contains(a.id))
        .length;
    final last = review.lastComment;
    if (last == null) return approved;
    final seen = held is Map ? _int(held['seen']) : 0;
    final more =
        review.commentCount - (held is Map ? _int(held['count']) : 0);
    // The server tells the time in seconds, so one made in the same second
    // as the last one seen is only told apart by there being more of them.
    return approved + (last > seen || more > 0 ? math.max(1, more) : 0);
  }

  int get unseen => links.fold(0, (sum, review) => sum + unseenIn(review));

  /// Whether the status line has something to say: comments nobody has
  /// looked at, or a send that is under way or failed with the window shut.
  bool get wantsAttention =>
      unseen > 0 || (sending != null && sending!.stage != SendStage.done);

  /// The link with the newest comment or approval nobody has looked at.
  Review? get newestUnseen {
    Review? newest;
    for (final review in links) {
      if (unseenIn(review) == 0) continue;
      if (newest == null || review.lastSaid > newest.lastSaid) {
        newest = review;
      }
    }
    return newest;
  }

  void _see(Review review) {
    final held = _keptOf(review.id);
    final last = review.lastComment ?? 0;
    final approvals = [for (final a in review.approvals) a.id];
    if (held['seen'] == last &&
        held['count'] == review.commentCount &&
        listEquals(held['approvals'] as List?, approvals)) {
      return;
    }
    held['seen'] = last;
    held['count'] = review.commentCount;
    held['approvals'] = approvals;
    _save();
  }

  // --- The list -------------------------------------------------------------

  Review? link(String? id) {
    for (final review in links) {
      if (review.id == id) return review;
    }
    return null;
  }

  /// Ask the server for the account's links. A link with nothing ready to
  /// watch is one still on its way up, or one that never made it, and is
  /// left out.
  Future<void> refresh() async {
    final who = account.account?.id;
    if (who == null) return;
    final news = _news;
    try {
      final j = await account.call('GET', '/v1/reviews');
      if (_gone || account.account?.id != who) return;
      // What the line said about the link on screen while this was on its
      // way is newer than this answer, which was asked for before it.
      final ahead = news == _news ? null : link(watching);
      links = [
        if (j['reviews'] case final List<dynamic> all)
          for (final r in all)
            if (r is Map<String, dynamic>) Review.from(r),
      ].where((r) => r.versions.isNotEmpty).toList();
      if (ahead != null && link(ahead.id) != null) _put(ahead);
      if (j['usage'] case final Map<String, dynamic> usage) {
        usedBytes = _int(usage['bytes']);
        limitBytes = _int(usage['limit']);
      }
      loaded = true;
      failure = null;
      // A link that has gone takes what was remembered about it with it.
      final before = _kept.length;
      _kept.removeWhere((id, _) => link(id) == null);
      if (_kept.length != before) _save();
      // The first time a profile is shown an account's links, what is on
      // them already was said somewhere else, and is taken as seen.
      if (!_known.contains(who)) {
        _known.add(who);
        links.forEach(_see);
        _save();
      }
      // The one on screen is being looked at, so it is seen as it changes.
      if (link(watching) case final shown?) _see(shown);
    } on CloudError catch (e) {
      if (_gone) return;
      failure = reviewFailure(e);
    }
    notifyListeners();
  }

  void _put(Review review) {
    final at = links.indexWhere((r) => r.id == review.id);
    at < 0 ? links.insert(0, review) : links[at] = review;
  }

  // --- The link being looked at ---------------------------------------------

  /// Hold [id]'s comments and hear about new ones, or let go with null.
  Future<void> watch(String? id) async {
    if (id == watching) return;
    watching = id;
    // Each look has a number. A link left and come back to while its first
    // read was still out is two looks, and only the newer may hold the line.
    final look = ++_look;
    comments = [];
    arrived.clear();
    _retry?.cancel();
    unawaited(_live?.close());
    _live = null;
    reading = id != null;
    if (!_gone) notifyListeners();
    if (id == null) return;
    await _read(id);
    if (look == _look) unawaited(_listen(id, 0, look));
  }

  int _look = 0;

  Future<void> _read(String id) async {
    try {
      final j = await account.call('GET', '/v1/reviews/$id');
      if (_gone || watching != id) return;
      if (j['review'] case final Map<String, dynamic> review) {
        final fresh = Review.from(review);
        if (fresh.versions.isNotEmpty) {
          _put(fresh);
          _see(fresh);
        }
      }
      comments = [
        if (j['comments'] case final List<dynamic> all)
          for (final c in all)
            if (c is Map<String, dynamic>) ReviewComment.from(c),
      ];
      failure = null;
    } on CloudError catch (e) {
      if (_gone || watching != id) return;
      failure = reviewFailure(e);
      // Deleted or expired since the list was read, so the list is stale.
      if (e.code == 'not_found') unawaited(refresh());
    }
    reading = false;
    notifyListeners();
  }

  Uri _socket(String id) {
    final base = Uri.parse(cloudUrl);
    return base.replace(
        scheme: base.scheme == 'http' ? 'ws' : 'wss',
        path: '/v1/reviews/$id/live');
  }

  /// The line the server says new comments down. One that drops is tried
  /// again, a little later each time, for as long as the link is on screen.
  /// A link with no room for another line is asked instead, each time.
  Future<void> _listen(String id, int tries, int look) async {
    var joined = false;
    var refused = false;
    try {
      final token = await account.accessToken();
      final socket = await WebSocket.connect('${_socket(id)}', headers: {
        if (token != null) 'Authorization': 'Bearer $token',
      }).timeout(const Duration(seconds: 10));
      if (_gone || look != _look) {
        unawaited(socket.close());
        return;
      }
      _live = socket;
      joined = true;
      socket.pingInterval = const Duration(seconds: 20);
      // Whatever was said while the line was down.
      if (tries > 0) await _read(id);
      await for (final message in socket) {
        if (message is String) _heard(id, message);
      }
    } on WebSocketException {
      // The server answered and gave no line: the link has all it can hold.
      refused = !joined;
    } catch (_) {
      // Offline, or the line dropped.
    }
    if (_gone || look != _look) return;
    // The first try comes straight after a read, so there is nothing new.
    if (refused && tries > 0) await _read(id);
    if (_gone || look != _look) return;
    _retry = Timer(Duration(seconds: math.min(30, 2 << tries)),
        () => _listen(id, joined ? 1 : tries + 1, look));
  }

  void _heard(String id, String message) {
    if (_gone || watching != id) return;
    final Object? j;
    try {
      j = jsonDecode(message);
    } catch (_) {
      return;
    }
    if (j is! Map<String, dynamic>) return;
    final said = j['comment'];
    switch (j['type']) {
      case 'comment' when said is Map<String, dynamic>:
        final comment = ReviewComment.from(said);
        if (!_take(id, comment)) return;
        arrived.add(comment.id);
      case 'comment_changed' when said is Map<String, dynamic>:
        _swap(ReviewComment.from(said));
      case 'comment_gone':
        _drop(_text(j['id']));
      case 'approval':
        // That version's whole list as it now stands, put in place of the
        // one held. It is on screen as it changes, so it is seen.
        final held = link(id);
        if (held == null) return;
        final n = _int(j['version']);
        final approvals = ReviewApproval.listFrom(j['approvals']);
        final fresh = held.changed(versions: [
          for (final v in held.versions)
            v.n == n ? v.approvedBy(approvals) : v,
        ]);
        _put(fresh);
        _see(fresh);
        _news++;
      case 'version':
        break;
      default:
        return;
    }
    notifyListeners();
    // The list's own counts, once things have gone quiet.
    _settle?.cancel();
    _settle = Timer(const Duration(seconds: 2), refresh);
  }

  /// Hold a comment that has just been made, unless it is held already. It
  /// is on screen as it lands, so it counts as seen.
  bool _take(String id, ReviewComment comment) {
    if (watching != id || comments.any((c) => c.id == comment.id)) return false;
    comments = [...comments, comment];
    // A like is in none of the server's counts, so it is in none kept here.
    if (comment.reaction != null) return true;
    final held = _keptOf(id);
    held['seen'] = math.max(_int(held['seen']), comment.created);
    held['count'] = _int(held['count']) + 1;
    _save();
    return true;
  }

  void _swap(ReviewComment comment) => comments = [
        for (final c in comments) c.id == comment.id ? comment : c,
      ];

  void _drop(String id) => comments = [
        for (final c in comments)
          if (c.id != id && c.parent != id) c,
      ];

  // --- Answering comments and changing a link ------------------------------

  /// Reply to [to] as the account.
  Future<void> reply(Review review, ReviewComment to, String text) async {
    final j =
        await account.call('POST', '/v1/reviews/${review.id}/comments', body: {
      'version': to.version,
      'frame': to.frame,
      if (to.endFrame != null) 'end_frame': to.endFrame,
      'name': account.account?.name ?? '',
      'text': text.trim(),
      'parent': to.parent ?? to.id,
    });
    // The socket may have said it first, and then it is held already.
    if (j['comment'] case final Map<String, dynamic> made) {
      _take(review.id, ReviewComment.from(made));
    }
    notifyListeners();
  }

  Future<void> resolve(
      Review review, ReviewComment comment, bool resolved) async {
    final j = await account.call(
        'PATCH', '/v1/reviews/${review.id}/comments/${comment.id}',
        body: {'resolved': resolved});
    if (j.isNotEmpty) _swap(ReviewComment.from(j));
    notifyListeners();
  }

  Future<void> deleteComment(Review review, ReviewComment comment) async {
    await account.call(
        'DELETE', '/v1/reviews/${review.id}/comments/${comment.id}');
    _drop(comment.id);
    notifyListeners();
  }

  /// Take a reviewer's approval off [review], as whoever sent it may.
  Future<void> removeApproval(Review review, ReviewApproval approval) async {
    await account.call(
        'DELETE', '/v1/reviews/${review.id}/approvals/${approval.id}');
    final held = link(review.id);
    if (held != null) {
      final fresh = held.changed(versions: [
        for (final v in held.versions)
          v.approvedBy([
            for (final a in v.approvals)
              if (a.id != approval.id) a,
          ]),
      ]);
      _put(fresh);
      _see(fresh);
    }
    notifyListeners();
  }

  /// The address of [review] that shows the video and nothing of the
  /// review. It is made the first time it is asked for, on the version that
  /// is newest then, and stays on that one whatever is sent after. Asked
  /// for with [version], it is moved to that one.
  Future<String> cleanAddress(Review review, {int? version}) async {
    final j = await account.call('POST', '/v1/reviews/${review.id}/clean',
        body: version == null ? null : {'version': version});
    final clean = j['clean'];
    if (clean is! String || clean.isEmpty) throw const CloudError('unknown');
    return _setClean(review, clean, _intOrNull(j['version'])).cleanUrl!;
  }

  /// Take that address away. It stops working at once.
  Future<void> removeClean(Review review) async {
    await account.call('DELETE', '/v1/reviews/${review.id}/clean');
    _setClean(review, null, null);
  }

  Review _setClean(Review review, String? clean, int? version) {
    final fresh =
        (link(review.id) ?? review).changed(clean: (clean, version));
    if (link(review.id) != null) _put(fresh);
    notifyListeners();
    return fresh;
  }

  /// Change a link's settings: any of `comments`, `pictures`, `downloads`,
  /// `mail`, `password` and `expires`.
  Future<void> patch(Review review, Map<String, Object?> change) async {
    final j =
        await account.call('PATCH', '/v1/reviews/${review.id}', body: change);
    if (j.isNotEmpty) _put(Review.from(j));
    notifyListeners();
  }

  Future<void> deleteLink(Review review) async {
    await account.call('DELETE', '/v1/reviews/${review.id}');
    links = [
      for (final r in links)
        if (r.id != review.id) r,
    ];
    if (_kept.remove(review.id) != null) _save();
    if (watching == review.id) unawaited(watch(null));
    notifyListeners();
    unawaited(refresh());
  }

  Future<void> deleteVersion(Review review, ReviewVersion version) async {
    await account.call('DELETE', '/v1/reviews/${review.id}/v/${version.n}');
    comments = [
      for (final c in comments)
        if (c.version != version.n) c,
    ];
    await refresh();
  }

  /// Who has opened [review] in a browser, newest first. Its maker is never
  /// among them.
  Future<List<ReviewView>> views(Review review) async {
    final j = await account.call('GET', '/v1/reviews/${review.id}/views');
    return [
      if (j['views'] case final List<dynamic> all)
        for (final v in all)
          if (v is Map<String, dynamic>)
            ReviewView(
              name: _text(v['name']),
              version: _int(v['version']),
              first: _int(v['first']),
              last: _int(v['last']),
            ),
    ];
  }

  // --- Whether a composition has moved on -----------------------------------

  final Map<String, (BigInt, String, Map<String, List<String>>)> _prints = {};

  /// The most layers whose marks are kept for a link. A composition with
  /// more is still known to have changed, and is not said how.
  static const int _mostLayers = 2000;

  /// What [comp] is made of, twice over.
  ///
  /// First a short string that is different once its picture could be: its
  /// size, rate and length, every layer as the project file would write it
  /// but for its markers, and the same for every composition it places.
  ///
  /// Then each of those layers' own mark, by the end of the layer's id: a
  /// short digest of everything the layer holds, its name, and for each of
  /// its effects the end of the effect's id and what it is called. That is
  /// what saying how it changed needs, and it is some dozens of bytes a
  /// layer. An effect is called by its sender's own name for it behind an
  /// equals sign, when it has one, and else by the name the engine knows.
  ///
  /// The project's own change count can't say this. It starts again each
  /// time a project is opened, and moves when anything in the project does.
  /// It is only used here to save working the answer out twice.
  // ponytail: a node graph composition's boxes are not read, so a change to
  // one goes unnoticed. Read them here if node graphs get sent for review.
  (String, Map<String, List<String>>) _made(CompositionReference comp) {
    final revision = comp.documentRevision();
    final held = _prints['${comp.internalid}'];
    if (held != null && held.$1 == revision) return (held.$2, held.$3);
    Digest? digest;
    final hasher = sha256.startChunkedConversion(
        ChunkedConversionSink<Digest>.withCallback((d) => digest = d.single));
    final marks = <String, List<String>>{};
    // An id is made from the time first, so its end is what tells two apart.
    String end(Object? id) {
      final whole = '$id';
      return whole.substring(math.max(0, whole.length - 12));
    }

    // A name is kept to be said in a line, so the start of it is enough.
    String name(Object? called) {
      final whole = '$called';
      return whole.substring(0, math.min(48, whole.length));
    }

    final read = <String>{};
    void add(CompositionReference c) {
      if (!read.add('${c.internalid}')) return;
      final s = c.getSettings();
      hasher.add(utf8.encode('${s.width}x${s.height} ${s.fpsNum}/${s.fpsDen} '
          '${c.durationFrames()} ${s.background.join(',')}\n'));
      for (final entry in c.getModel().layers) {
        final copied = entry.layer.copyLayer();
        try {
          final whole = jsonDecode(copied) as Map;
          final layer = whole['layer'] as Map;
          // A layer's markers are notes on the work and not the work, as
          // the composition's are. A comment added to the Timeline is no
          // edit.
          // ponytail: read and written again, a whole number past 2^63 is
          // rounded, so a change to its last digits alone goes unseen.
          layer.remove('markers');
          final bytes = utf8.encode(jsonEncode(whole));
          final effects = layer['effects'];
          marks[end(layer['id'])] = [
            '${sha1.convert(bytes)}'.substring(0, 12),
            name(layer['name']),
            if (effects is List)
              for (final effect in effects)
                if (effect is Map) ...[
                  end(effect['id']),
                  effect['custom_name'] is String
                      ? '=${name(effect['custom_name'])}'
                      : name((effect['effect'] as Map?)?['match_name']),
                ],
          ];
          hasher.add(bytes);
        } catch (_) {
          // A layer that can't be read this way is taken as it came, and
          // only left out of the marks.
          hasher.add(utf8.encode(copied));
        }
      }
      for (final placed in c.getFlow().uses) {
        add(placed.comp);
      }
    }

    add(comp);
    hasher.close();
    if (marks.length > _mostLayers) marks.clear();
    final made = '$digest';
    _prints['${comp.internalid}'] = (revision, made, marks);
    return (made, marks);
  }

  /// Whether [comp] is no longer what it was when [review]'s newest version
  /// was sent from it. False when that was not on this machine.
  bool movedOn(Review review, CompositionReference comp) {
    final held = _kept[review.id];
    if (held is! Map || held['sent'] is! String) return false;
    if (held['sent_n'] != review.versions.last.n) return false;
    return _made(comp).$1 != held['sent'];
  }

  /// What is different in [comp] from when [review]'s newest version was
  /// sent from it, by name. Null when that is not known: it was not sent
  /// from this machine, or before names were kept, or it has too many
  /// layers to keep them for.
  ReviewChanges? changedSince(Review review, CompositionReference comp) {
    final held = _kept[review.id];
    if (held is! Map || held['sent_n'] != review.versions.last.n) return null;
    final was = held['marks'], now = _made(comp).$2;
    if (was is! Map || was.isEmpty || now.isEmpty) return null;
    // A mark's effects, by the ends of their ids.
    Map<String, String> effects(List<dynamic> mark) => {
          for (var i = 2; i + 1 < mark.length; i += 2)
            '${mark[i]}': '${mark[i + 1]}',
        };
    final changes = ReviewChanges();
    for (final MapEntry(key: id, value: mark) in now.entries) {
      final old = was[id];
      if (old == null) {
        changes.added.add(mark[1]);
        continue;
      }
      if (old is! List || old.length < 2) return null;
      final had = effects(old), has = effects(mark);
      for (final MapEntry(:key, :value) in has.entries) {
        if (!had.containsKey(key)) changes.effectsAdded.add((value, mark[1]));
      }
      for (final MapEntry(:key, :value) in had.entries) {
        if (!has.containsKey(key)) {
          changes.effectsRemoved.add((value, mark[1]));
        }
      }
      // A layer an effect came to or left is said by the effect, and not
      // a second time as a layer that changed.
      if (had.length == has.length &&
          has.keys.every(had.containsKey) &&
          old[0] != mark[0]) {
        changes.changed.add(mark[1]);
      }
    }
    for (final MapEntry(key: id, value: old) in was.entries) {
      if (now.containsKey(id)) continue;
      if (old is! List || old.length < 2) return null;
      changes.removed.add('${old[1]}');
    }
    return changes;
  }

  // --- Comments onto the Timeline -------------------------------------------

  /// The markers the open comments on [version] would add to [comp]: every
  /// one whose marker is not on the composition or one of its layers now.
  /// Asking the composition, not only the memory, is what lets an undone add
  /// be done again. [layers] is the layer each comment points at, for those
  /// that point at one that is still there.
  List<CommentMarker> pendingMarkers(
      Review review, ReviewVersion version, CompositionReference comp,
      {Map<String, BridgeLayerEntry> layers = const {}}) {
    final held = _kept[review.id];
    final marked = held is Map ? held['marked'] : null;
    final present = {
      for (final m in markersOf(comp)) '${m.id}',
      for (final entry in comp.getModel().layers)
        for (final m in entry.info.markers) '${m.marker.id}',
    };
    return commentMarkers(
      comments.where((c) => c.version == version.n),
      startFrame: version.startFrame,
      compFrames: comp.durationFrames(),
      layers: {
        for (final MapEntry(:key, :value) in layers.entries)
          key: '${value.layer.internallayerId}',
      },
      added: {
        if (marked is Map)
          for (final MapEntry(:key, :value) in marked.entries)
            if (present.contains(value)) '$key',
      },
    );
  }

  /// Add those markers, and say how many there were: to the layer a comment
  /// points at, as that layer's own, and else to [comp]. A marker already on
  /// one of their frames gives way, as it does to any marker put where it
  /// is. Beats keep their own band and stay. It writes once to the
  /// composition and once to each layer, so the caller makes it one undo
  /// step.
  int addToTimeline(
      Review review, ReviewVersion version, CompositionReference comp,
      {Map<String, BridgeLayerEntry> layers = const {}}) {
    final fresh = pendingMarkers(review, version, comp, layers: layers);
    if (fresh.isEmpty) return 0;
    final held = _keptOf(review.id);
    final marked = held['marked'] is Map<String, dynamic>
        ? held['marked'] as Map<String, dynamic>
        : held['marked'] = <String, dynamic>{};
    final byId = {
      for (final entry in layers.values)
        '${entry.layer.internallayerId}': entry.layer,
    };
    // What goes where: a layer's under its id and the composition's under
    // null, each by the frame it lands on in that list's own time.
    final made = <String?, Map<int, BridgeMarker>>{};
    for (final m in fresh) {
      // A layer's own time starts where the layer does.
      final frame = switch (byId[m.layer]) {
        final layer? =>
          m.frame - frameAtTime(comp, layer.getSpan().startOffset),
        null => m.frame,
      };
      final id = UuidValue.fromString(const Uuid().v4());
      (made[m.layer] ??= {})[frame] = BridgeMarker(
        id: id,
        time: timeOfFrame(comp, frame),
        label: m.label,
        colour: m.colour,
        durationFrames: m.duration,
        isBeat: false,
      );
      for (final comment in m.ids) {
        marked[comment] = '$id';
      }
    }
    for (final MapEntry(key: id, value: put) in made.entries) {
      final layer = byId[id];
      final all = [
        for (final m in layer == null ? markersOf(comp) : layer.getMarkers())
          if (m.isBeat || !put.containsKey(frameAtTime(comp, m.time))) m,
        ...put.values,
      ]
        // In time order, a beat before a flag on its frame. A flag is as
        // wide as its words and the ruler draws the list as it stands, so
        // one listed after a marker to its right would lie over it and
        // take its presses.
        ..sort((a, b) {
          final by =
              frameAtTime(comp, a.time).compareTo(frameAtTime(comp, b.time));
          return by != 0 ? by : (b.isBeat ? 1 : 0) - (a.isBeat ? 1 : 0);
        });
      if (layer == null) {
        writeMarkers(comp, all);
      } else {
        layer.setMarkers(markers: all);
      }
    }
    _save();
    notifyListeners();
    return fresh.length;
  }

  // --- Sending --------------------------------------------------------------

  /// Export [comp] from [start] up to [end] and send it: as a new link called
  /// [title], or as the next version of [into].
  ///
  /// [edge] is 1080 or 720. [comments] is whether reviewers may comment on a
  /// new link. [note] is what changed, said in a line. [project] is the open
  /// project's path, to remember beside the composition. How it is going is
  /// on [sending], and nothing is thrown.
  Future<void> send(
    CompositionReference comp, {
    Review? into,
    required String title,
    required int edge,
    required int start,
    required int end,
    bool comments = true,
    String note = '',
    String project = '',
  }) async {
    if (sending != null &&
        sending!.stage != SendStage.done &&
        sending!.stage != SendStage.failed) {
      return;
    }
    final send = sending = ReviewSend(total: end - start);
    _cancel = false;
    notifyListeners();
    Directory? folder;
    // The link and version the server has made room for, until it is whole.
    (String, int, bool)? begun;
    try {
      // A full account is refused here, before a long export and not after.
      if (limitBytes > 0 && usedBytes >= limitBytes) {
        throw const _Stopped('storage_full');
      }
      final settings = comp.getSettings();
      // The composition as it goes out, which the export takes its copy of
      // now. An edit made while it is on its way up is one made since.
      final (sent, marks) = _made(comp);
      final (width, height) = reviewSize(settings.width, settings.height, edge);
      final spec = BridgeExportSpec(
        preset: '',
        codec: 'h264',
        width: width,
        height: height,
        bitrateMbps: edge >= 1080 ? 6 : 3,
        peakMbps: 0,
        bitrateAuto: false,
        fps: 0,
        rangeStartFrame: start,
        rangeEndFrame: end,
        includeAudio: true,
        audioBitRate: 160000,
        depth: 8,
        alphaChannel: false,
        straightAlpha: false,
        colourSpace: '',
        cropTop: 0,
        cropLeft: 0,
        cropBottom: 0,
        cropRight: 0,
        useRegionOfInterest: false,
        region: Float64List(0),
        metadata: const [],
        qualityDivisor: 1,
        diskCacheReadOnly: false,
        effects: true,
        honourSolo: true,
        makeANoise: false,
        openFolder: false,
      );
      if (end <= start || comp.exportSpecCheck(spec: spec).isNotEmpty) {
        throw const _Stopped('export_refused');
      }
      // The engine writes the file and does not make the folder.
      folder = Directory.systemTemp.createTempSync(_tempPrefix);
      final file = File('${folder.path}${Platform.pathSeparator}review.mp4');
      await _export(comp, spec, file.path, send);
      if (!file.existsSync()) throw const _Stopped('export_failed');

      // ponytail: the size is only known once the export is done, so one too
      // big for the server is refused after it was made. Work it out from
      // the bit rate first if long compositions make that hurt.
      final size = file.lengthSync();
      send
        ..stage = SendStage.uploading
        ..waiting = false
        ..done = 0
        ..total = size;
      notifyListeners();
      final made = await account.call('POST', '/v1/reviews', body: {
        'review': into?.id,
        'title': title.trim(),
        'comp': settings.name,
        'note': note.trim().substring(0, math.min(note.trim().length, 200)),
        'size': size,
        'width': width,
        'height': height,
        'fps_num': settings.fpsNum,
        'fps_den': settings.fpsDen,
        'frames': end - start,
        'start_frame': start,
      });
      final id = _text((made['review'] as Map?)?['id']);
      final n = _int(made['version']);
      begun = (id, n, into == null);
      _stopIfCancelled();
      // Closed to comments before anyone has the address, not after.
      if (into == null && !comments) {
        await account
            .call('PATCH', '/v1/reviews/$id', body: {'comments': false});
      }
      final parts = await _sendParts(file, id, n, _int(made['part_size']),
          _int(made['parts']), send);
      await _sendPoster(comp, start, id, n);
      _stopIfCancelled();
      final review = Review.from(await account.call(
          'POST', '/v1/reviews/$id/v/$n/finish',
          body: {'parts': parts}));
      begun = null;

      _keptOf(id)
        ..['comp'] = '${comp.internalid}'
        ..['comp_name'] = settings.name
        ..['project'] = project
        ..['sent'] = sent
        ..['sent_n'] = n
        ..['marks'] = marks;
      _put(review);
      _see(review);
      _save();
      send
        ..stage = SendStage.done
        ..review = review;
      unawaited(Clipboard.setData(ClipboardData(text: review.url))
          .catchError((Object _) {}));
      unawaited(refresh());
    } catch (e) {
      final why = _cancel ? 'cancelled' : reviewFailure(e);
      if (why == 'cancelled') {
        sending = null;
      } else {
        send
          ..stage = SendStage.failed
          ..failure = why;
      }
      // The server is holding room for a file that is not coming.
      if (begun case (final id, final n, final whole)?) {
        unawaited(account
            .call('DELETE', whole ? '/v1/reviews/$id' : '/v1/reviews/$id/v/$n')
            .then((_) {}, onError: (Object _) {}));
      }
    } finally {
      _upload?.close(force: true);
      _upload = null;
      if (folder != null) unawaited(_tidy(folder));
      if (!_gone) notifyListeners();
    }
  }

  /// Stop the send under way, wherever it has got to.
  void cancelSend() {
    _cancel = true;
    _upload?.close(force: true);
  }

  /// Put a send that has ended away, so the window can start another.
  void clearSend() {
    final stage = sending?.stage;
    if (stage != SendStage.done && stage != SendStage.failed) return;
    sending = null;
    notifyListeners();
  }

  void _stopIfCancelled() {
    if (_cancel || _gone) throw const _Stopped('cancelled');
  }

  /// Run the export and come back when the file is written.
  ///
  /// It goes through the export queue, where it has a row of its own to ask
  /// after and waits its turn behind anything already running. The one time
  /// it does not is when the queue holds items nobody has started: starting
  /// one item starts them all, so then it is exported on its own instead and
  /// the queue is left as it was.
  Future<void> _export(CompositionReference comp, BridgeExportSpec spec,
      String path, ReviewSend send) async {
    bool isRunning(BridgeExportQueueItem row) =>
        row.state is BridgeExportQueueState_Running;
    final queue = exportQueueList();
    final parked = !queue.any(isRunning) &&
        queue.any((row) => row.state is BridgeExportQueueState_Waiting);
    int? row;
    try {
      if (parked) {
        comp.startExport(spec: spec, path: path);
      } else {
        row = comp.queueExport(spec: spec, path: path, start: true);
      }
    } catch (_) {
      // Something else is exporting, or an addon the composition needs is
      // missing. The engine's refusal has no words to read.
      throw const _Stopped('export_refused');
    }
    var finished = false;
    try {
      while (true) {
        await Future<void>.delayed(const Duration(milliseconds: 250));
        _stopIfCancelled();
        // Asking is also what turns the queue over.
        final rows = exportQueueList();
        if (row == null) {
          switch (exportPoll()) {
            case BridgeExportState_Running(:final frame, :final total):
              send
                ..done = frame.toInt()
                ..total = total.toInt();
            case BridgeExportState_Done():
              finished = true;
              return;
            case _:
              finished = true;
              throw const _Stopped('export_failed');
          }
        } else {
          switch (rows.where((r) => r.id == row).firstOrNull?.state) {
            case BridgeExportQueueState_Waiting():
              send.waiting = true;
            case BridgeExportQueueState_Running(:final frame, :final total):
              send
                ..waiting = false
                ..done = frame.toInt()
                ..total = total.toInt();
            case BridgeExportQueueState_Done():
              return;
            case BridgeExportQueueState_Failed():
              throw const _Stopped('export_failed');
            // Taken out of the queue window by hand.
            case null:
              throw const _Stopped('cancelled');
          }
        }
        notifyListeners();
      }
    } finally {
      if (row != null) {
        // Out of the person's queue either way. One still running is
        // stopped first, and the engine deletes what it had written.
        exportQueueRemove(id: row);
      } else if (!finished && !exportQueueList().any(isRunning)) {
        exportCancel();
      }
    }
  }

  /// Send the file in the pieces the server asked for, and come back with
  /// what it called each. A piece that fails is tried three more times.
  Future<List<Map<String, Object>>> _sendParts(File file, String id, int n,
      int partSize, int count, ReviewSend send) async {
    final http = _upload = HttpClient()
      ..connectionTimeout = const Duration(seconds: 10)
      ..userAgent = 'Lumit';
    return sendParts(
      http,
      file,
      size: send.total,
      partSize: partSize,
      count: count,
      at: (part) => Uri.parse('$cloudUrl/v1/reviews/$id/v/$n/parts/$part'),
      headers: () async {
        final token = await account.accessToken();
        if (token == null) throw const CloudError('signed_out', 401);
        return {HttpHeaders.authorizationHeader: 'Bearer $token'};
      },
      onProgress: (done) {
        send.done = done;
        notifyListeners();
      },
      check: _stopIfCancelled,
    );
  }

  /// A still of the first frame, for the page and for the list here. A link
  /// without one is fine, so nothing that goes wrong with it stops the send.
  Future<void> _sendPoster(
      CompositionReference comp, int frame, String id, int n) async {
    try {
      final png = await Workspace.compStillPng(comp, frame, maxEdge: 960);
      final token = await account.accessToken();
      // The server takes three megabytes and no more.
      if (png == null || png.length > 3 << 20 || token == null) return;
      final request = await _upload!
          .openUrl('PUT', Uri.parse('$cloudUrl/v1/reviews/$id/v/$n/poster'));
      request.headers
        ..set(HttpHeaders.authorizationHeader, 'Bearer $token')
        ..contentType = ContentType('image', 'png');
      request.contentLength = png.length;
      request.add(png);
      await (await request.close()).drain<void>();
    } catch (_) {
      // No still.
    }
  }

  /// Delete the folder the export went to. The encoder may still have the
  /// file open for a moment after a cancel, so this keeps trying for a bit.
  Future<void> _tidy(Directory folder) async {
    for (var tries = 0; tries < 20; tries++) {
      try {
        if (folder.existsSync()) folder.deleteSync(recursive: true);
        return;
      } catch (_) {
        await Future<void>.delayed(const Duration(milliseconds: 500));
      }
    }
  }

  @override
  void dispose() {
    _gone = true;
    _cancel = true;
    _beat?.cancel();
    _retry?.cancel();
    _settle?.cancel();
    unawaited(_live?.close());
    _upload?.close(force: true);
    if (_started) account.removeListener(_accountChanged);
    super.dispose();
  }
}
