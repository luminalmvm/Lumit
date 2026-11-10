// The Transcript panel's page: the words as a Dart model, the paragraphs laid
// out and painted, and where a press lands.
//
// Nothing here calls the bridge. The panel builds a [TranscriptModel] from
// what the engine sends, and this file turns it into blocks of a scrolling
// list, each paragraph one [TextPainter] laid out once per width and text
// size, so a two-hour transcript scrolls at full rate and any word can be
// jumped to by its block's height.

import 'dart:math' as math;
import 'dart:ui' show BoxHeightStyle;

import 'package:flutter/widgets.dart';

/// What a token is. [gap] is a break: a stretch of the composition with no
/// speech in it, which stands as a line of its own.
enum TranscriptKind { word, pause, gap, removed }

/// Why a word counts as filler, or none. A retake is an earlier attempt at
/// a sentence the speaker said again.
enum TranscriptFiller { none, hesitation, phrase, repeat, retake }

class TranscriptSpeaker {
  final String id;
  final String name;
  const TranscriptSpeaker({required this.id, required this.name});
}

/// One thing on the page: a word, a pause between words, a break, or a word
/// that was cut out. Frames are the subject's own: the composition's for the
/// Cut composition, the item's for the source.
class TranscriptToken {
  final TranscriptKind kind;
  final String text;

  /// The stretch an edit takes.
  final int startFrame;
  final int endFrame;

  /// The stretch the word is actually heard over.
  final int heardStartFrame;
  final int heardEndFrame;

  /// Which footage item the word is in, by its place in the read and by its
  /// id, its index in that item's own transcript, the clip it is heard
  /// through, and who says it. Null for none.
  final int? item;
  final String? itemId;
  final int? index;
  final String? clip;
  final int? speaker;
  final double confidence;

  /// Whether a paragraph starts here.
  final bool paragraph;
  final TranscriptFiller filler;

  /// A filler too close to the words round it to cut cleanly.
  final bool tight;

  /// Whether an edit point sits just before this token.
  final bool editBefore;

  /// Whether a space goes before the text: not between the characters of a
  /// language written without them.
  final bool spaceBefore;

  const TranscriptToken({
    required this.kind,
    required this.text,
    required this.startFrame,
    required this.endFrame,
    required this.heardStartFrame,
    required this.heardEndFrame,
    this.item,
    this.itemId,
    this.index,
    this.clip,
    this.speaker,
    this.confidence = 1,
    this.paragraph = false,
    this.filler = TranscriptFiller.none,
    this.tight = false,
    this.editBefore = false,
    this.spaceBefore = true,
  });

  bool get isWord =>
      kind == TranscriptKind.word || kind == TranscriptKind.removed;
  bool get isFiller => filler != TranscriptFiller.none;
  int get frames => endFrame - startFrame;

  /// The same word across reads, by item and index, which an edit elsewhere
  /// does not move. A pause or a break has no index and goes by its frame.
  Object get identity =>
      itemId != null && index != null ? (itemId!, index!) : startFrame;

  /// How long a pause or a break is quiet for: the whole silence, from the
  /// word before to the word after, which the review lists and labels.
  int get silence => heardEndFrame - heardStartFrame;
}

class TranscriptModel {
  final List<TranscriptToken> tokens;
  final List<TranscriptSpeaker> speakers;
  final int fpsNum;
  final int fpsDen;
  final String language;

  const TranscriptModel({
    required this.tokens,
    required this.speakers,
    required this.fpsNum,
    required this.fpsDen,
    this.language = '',
  });

  static const empty =
      TranscriptModel(tokens: [], speakers: [], fpsNum: 60, fpsDen: 1);

  double secondsOf(int frames) =>
      fpsNum <= 0 ? 0 : frames * fpsDen / fpsNum;

  /// The words of [from] up to [to], as a line of text, spaced as the page
  /// shows them.
  String textOf(int from, int to) {
    final out = StringBuffer();
    for (var i = from; i < to && i < tokens.length; i++) {
      final t = tokens[i];
      if (!t.isWord) continue;
      if (out.isNotEmpty && t.spaceBefore) out.write(' ');
      out.write(t.text);
    }
    return out.toString();
  }

  /// The frame stretches a selection covers, one per range.
  List<({int start, int end})> spansOf(List<(int, int)> ranges) => [
        for (final (from, to) in ranges)
          if (to > from)
            (start: tokens[from].startFrame, end: tokens[to - 1].endFrame),
      ];

  /// The words a filler review lists, by its switches.
  List<int> fillers({
    required bool hesitations,
    required bool phrases,
    required bool repeats,
    required bool retakes,
    required bool skipTight,
  }) =>
      [
        for (var i = 0; i < tokens.length; i++)
          if (tokens[i].kind == TranscriptKind.word &&
              switch (tokens[i].filler) {
                TranscriptFiller.none => false,
                TranscriptFiller.hesitation => hesitations,
                TranscriptFiller.phrase => phrases,
                TranscriptFiller.repeat => repeats,
                TranscriptFiller.retake => retakes,
              } &&
              !(skipTight && tokens[i].tight))
            i,
      ];

  /// The pauses whose silence is longer than [longerThanMs], as the engine's
  /// Shorten counts them.
  List<int> pauses(int longerThanMs) => [
        for (var i = 0; i < tokens.length; i++)
          if (tokens[i].kind == TranscriptKind.pause &&
              secondsOf(tokens[i].silence) * 1000 > longerThanMs)
            i,
      ];

  /// Every token [speaker] says.
  List<int> saidBy(int speaker) => [
        for (var i = 0; i < tokens.length; i++)
          if (tokens[i].speaker == speaker && tokens[i].kind != TranscriptKind.gap)
            i,
      ];

  /// The runs of words matching [query] in the text as the page shows it,
  /// case and punctuation folded, so a phrase matches across word edges and
  /// a language without spaces matches within its run of characters. With
  /// [whole], only matches that start and end on a word's edges.
  List<(int, int)> find(String query, {bool whole = false}) {
    final want = _fold(query).replaceAll(RegExp(r'\s+'), ' ').trim();
    if (want.isEmpty) return const [];
    final page = StringBuffer();
    final starts = List<int>.filled(tokens.length, -1);
    final ends = List<int>.filled(tokens.length, -1);
    // A pause or a break is a gap in the words either way, written once.
    var gap = false;
    for (var i = 0; i < tokens.length; i++) {
      final t = tokens[i];
      if (!t.isWord) {
        gap = true;
        continue;
      }
      if (page.isNotEmpty && (t.spaceBefore || gap)) page.write(' ');
      gap = false;
      starts[i] = page.length;
      page.write(_fold(t.text));
      ends[i] = page.length;
    }
    final text = page.toString();
    final found = <(int, int)>[];
    var at = text.indexOf(want);
    var token = 0;
    while (at >= 0) {
      final end = at + want.length;
      while (token < tokens.length && (ends[token] < 0 || ends[token] <= at)) {
        token++;
      }
      var last = token;
      for (var k = token; k < tokens.length; k++) {
        if (starts[k] >= 0 && starts[k] >= end) break;
        if (starts[k] >= 0) last = k;
      }
      if (token < tokens.length &&
          (!whole || (starts[token] == at && ends[last] == end)) &&
          (found.isEmpty || found.last != (token, last + 1))) {
        found.add((token, last + 1));
      }
      at = text.indexOf(want, at + 1);
    }
    return found;
  }

  static final _punctuation = RegExp(r'[^\p{L}\p{N}\s]', unicode: true);
  static String _fold(String s) =>
      s.toLowerCase().replaceAll(_punctuation, '').trim();
}

/// Which tokens are picked: whole tokens, in ranges, and where the caret
/// stands (before token [caret], or after the last when it equals the token
/// count). Several ranges are one selection.
class TranscriptSelection {
  final List<(int, int)> ranges;
  final int? caret;

  const TranscriptSelection({this.ranges = const [], this.caret});

  static const none = TranscriptSelection();

  bool get isEmpty => ranges.isEmpty;

  bool contains(int token) =>
      ranges.any((r) => token >= r.$1 && token < r.$2);

  /// The first and last token picked, or null with none.
  (int, int)? get bounds {
    if (ranges.isEmpty) return null;
    var lo = ranges.first.$1, hi = ranges.first.$2;
    for (final (a, b) in ranges) {
      lo = math.min(lo, a);
      hi = math.max(hi, b);
    }
    return (lo, hi);
  }

  int get count {
    var n = 0;
    for (final (a, b) in ranges) {
      n += b - a;
    }
    return n;
  }

  /// With [range] added, overlapping ranges joined, sorted.
  TranscriptSelection adding((int, int) range, {int? caret}) {
    final all = [...ranges, range]..sort((a, b) => a.$1.compareTo(b.$1));
    final joined = <(int, int)>[];
    for (final r in all) {
      if (r.$2 <= r.$1) continue;
      if (joined.isNotEmpty && r.$1 <= joined.last.$2) {
        joined.last = (joined.last.$1, math.max(joined.last.$2, r.$2));
      } else {
        joined.add(r);
      }
    }
    return TranscriptSelection(ranges: joined, caret: caret ?? this.caret);
  }

  /// A fresh selection of one range.
  static TranscriptSelection of(int a, int b, {int? caret}) =>
      TranscriptSelection(
          ranges: [(math.min(a, b), math.max(a, b))],
          caret: caret ?? math.max(a, b));
}

/// The type the page is set in, read off the theme by the panel.
class TranscriptStyles {
  final TextStyle word;
  final TextStyle filler;
  final TextStyle removed;
  final TextStyle uncertain;
  final TextStyle pauseLabel;
  final double size;

  const TranscriptStyles({
    required this.word,
    required this.filler,
    required this.removed,
    required this.uncertain,
    required this.pauseLabel,
    required this.size,
  });

  /// The line's pitch, about one and a half times the type.
  static const double lineHeight = 1.55;
}

sealed class TranscriptBlock {
  /// The first token this block holds, and one past its last.
  final int from;
  final int to;
  double get height;
  const TranscriptBlock(this.from, this.to);
}

/// A break: a line of its own saying how long the silence runs.
class TranscriptBreakBlock extends TranscriptBlock {
  @override
  final double height;
  const TranscriptBreakBlock(int token, this.height) : super(token, token + 1);
}

/// A run of words and pauses, with a heading when the speaker changed or a
/// minute has passed since the last one, laid out as one paragraph of text.
class TranscriptParagraph extends TranscriptBlock {
  /// Whether a heading line stands above it, and the speaker it names, or
  /// null for a heading with the timecode alone.
  final bool heading;
  final int? speaker;
  final double headingHeight;
  final TextPainter painter;

  /// Where each token's text starts and ends in the painter's characters,
  /// by token index less [from].
  final List<int> charStart;
  final List<int> charEnd;

  /// Which placeholder each pause is, by token index less [from], or -1.
  final List<int> placeholderOf;

  TranscriptParagraph({
    required int from,
    required int to,
    required this.heading,
    required this.speaker,
    required this.headingHeight,
    required this.painter,
    required this.charStart,
    required this.charEnd,
    required this.placeholderOf,
  }) : super(from, to);

  @override
  double get height => headingHeight + painter.height;

  /// The top of the text within the block.
  double get textTop => headingHeight;

  List<LineMetrics>? _lines;
  List<LineMetrics> get lines => _lines ??= painter.computeLineMetrics();

  /// The boxes token [token] is drawn in, in the block's own coordinates.
  /// [throughSpace] takes the gap after it too, so neighbours join up.
  List<Rect> rectsOf(int token, {bool throughSpace = false}) {
    final i = token - from;
    if (i < 0 || i >= charStart.length) return const [];
    final end = throughSpace && i + 1 < charStart.length
        ? charStart[i + 1]
        : charEnd[i];
    return _boxes(charStart[i], end);
  }

  List<Rect> _boxes(int start, int end) => [
        for (final box in _textBoxes(start, end)) box.toRect(),
      ];

  /// The text engine's boxes in the block's coordinates, each keeping the
  /// direction its run is written in.
  List<TextBox> _textBoxes(int start, int end) => [
        for (final b in painter.getBoxesForSelection(
            TextSelection(baseOffset: start, extentOffset: end),
            boxHeightStyle: BoxHeightStyle.max))
          TextBox.fromLTRBD(b.left, b.top + headingHeight, b.right,
              b.bottom + headingHeight, b.direction),
      ];

  List<TextBox> _tokenBoxes(int token) {
    final i = token - from;
    if (i < 0 || i >= charStart.length) return const [];
    return _textBoxes(charStart[i], charEnd[i]);
  }

  /// The boxes of tokens [a] up to [b] as one run, joined through the gaps.
  List<Rect> rectsOfRun(int a, int b) {
    final i = math.max(0, a - from), j = math.min(charStart.length, b - from);
    if (j <= i) return const [];
    return _boxes(charStart[i], charEnd[j - 1]);
  }

  /// Where a caret before [token] stands (after the last word when [token]
  /// is [to]): its x and the line's top and bottom.
  ({double x, double top, double bottom})? caretAt(int token) {
    final i = token - from;
    if (i < 0 || i > charStart.length || charStart.isEmpty) return null;
    final List<TextBox> boxes;
    final bool after;
    if (i == charStart.length) {
      boxes = _tokenBoxes(to - 1);
      after = true;
    } else {
      boxes = _tokenBoxes(token);
      after = false;
    }
    if (boxes.isEmpty) return null;
    final b = after ? boxes.last : boxes.first;
    // A word written right to left starts at its right edge.
    final rtl = b.direction == TextDirection.rtl;
    return (x: after != rtl ? b.right : b.left, top: b.top, bottom: b.bottom);
  }

  /// The token under [local], and whether the press was past its middle.
  /// Null off the text.
  ({int token, bool after})? hit(Offset local) {
    final at = local - Offset(0, headingHeight);
    if (at.dy < 0 || at.dy > painter.height) return null;
    final i = _tokenOfChar(_charAt(at));
    if (i == null) return null;
    final boxes = _tokenBoxes(from + i);
    if (boxes.isEmpty) return (token: from + i, after: false);
    final box = boxes.firstWhere(
        (b) => b.top <= local.dy && local.dy <= b.bottom,
        orElse: () => boxes.first);
    final mid = (box.left + box.right) / 2;
    return (
      token: from + i,
      after: box.direction == TextDirection.rtl
          ? local.dx < mid
          : local.dx > mid,
    );
  }

  /// The character under [at], in the painter's own coordinates. The text
  /// engine answers the nearest caret position, which past a glyph's middle
  /// is after it, so an upstream answer means the character before.
  int _charAt(Offset at) {
    final pos = painter.getPositionForOffset(at);
    return pos.affinity == TextAffinity.upstream && pos.offset > 0
        ? pos.offset - 1
        : pos.offset;
  }

  /// The token nearest [x] on line [line].
  int tokenOnLine(int line, double x) {
    final m = lines[line.clamp(0, lines.length - 1)];
    final y = m.baseline - m.ascent + m.height / 2;
    final i = _tokenOfChar(_charAt(Offset(x, y))) ?? charStart.length - 1;
    // Past the line's end the caret lands after its last word.
    final r = rectsOf(from + i);
    if (r.isNotEmpty && x > r.last.right && from + i + 1 < to) {
      final next = rectsOf(from + i + 1);
      if (next.isNotEmpty && next.first.top > r.last.top) return from + i + 1;
    }
    return from + i;
  }

  /// Which line the caret before [token] is on.
  int lineOf(int token) {
    final c = caretAt(token);
    if (c == null) return 0;
    final y = c.top - headingHeight + 1;
    for (var i = 0; i < lines.length; i++) {
      final m = lines[i];
      if (y < m.baseline - m.ascent + m.height) return i;
    }
    return lines.length - 1;
  }

  /// The first and last tokens on line [line].
  (int, int) lineTokens(int line) {
    var first = to, last = from;
    for (var i = 0; i < charStart.length; i++) {
      if (lineOf(from + i) == line) {
        first = math.min(first, from + i);
        last = math.max(last, from + i);
      }
    }
    return first > last ? (from, from) : (first, last);
  }

  int? _tokenOfChar(int offset) {
    if (charStart.isEmpty) return null;
    var lo = 0, hi = charStart.length - 1;
    while (lo < hi) {
      final mid = (lo + hi + 1) >> 1;
      if (charStart[mid] <= offset) {
        lo = mid;
      } else {
        hi = mid - 1;
      }
    }
    return lo;
  }
}

/// The page laid out at one width and type size.
class TranscriptLayout {
  final TranscriptModel model;
  final TranscriptStyles styles;
  final double width;
  final List<TranscriptBlock> blocks;

  /// Which block each token is in.
  final List<int> blockOfToken;

  /// Whether a pause chip carries its length, which widens it.
  final bool pauseLabels;

  TranscriptLayout._(this.model, this.styles, this.width, this.blocks,
      this.blockOfToken, this.pauseLabels);

  /// Each paragraph laid out once, at [width].
  factory TranscriptLayout.lay({
    required TranscriptModel model,
    required TranscriptStyles styles,
    required double width,
    required bool pauseLabels,
    required double headingHeight,
  }) {
    final tokens = model.tokens;
    final blocks = <TranscriptBlock>[];
    final blockOf = List<int>.filled(tokens.length, 0);
    final lineHeight = styles.size * TranscriptStyles.lineHeight;
    // A heading comes back after a minute without one, so a long turn still
    // has a timecode to find a place by.
    final minute = model.fpsDen <= 0
        ? 0
        : (60 * model.fpsNum / model.fpsDen).round();
    int? lastSpeaker;
    int? lastHeading;
    var i = 0;
    while (i < tokens.length) {
      final t = tokens[i];
      if (t.kind == TranscriptKind.gap) {
        blockOf[i] = blocks.length;
        blocks.add(TranscriptBreakBlock(i, lineHeight + 8));
        i++;
        continue;
      }
      // A paragraph runs to the next paragraph start, speaker change or
      // break. Pauses carry no speaker and so never start one.
      final speaker = _speakerFrom(tokens, i);
      var j = i + 1;
      while (j < tokens.length) {
        final n = tokens[j];
        if (n.kind == TranscriptKind.gap || n.paragraph) break;
        if (n.speaker != null && speaker != null && n.speaker != speaker) {
          break;
        }
        if (n.speaker != null && speaker == null) break;
        j++;
      }
      final at = t.heardStartFrame;
      final heading = (speaker != null && speaker != lastSpeaker) ||
          lastHeading == null ||
          at - lastHeading >= minute;
      if (speaker != null) lastSpeaker = speaker;
      if (heading) lastHeading = at;
      final paragraph = _lay(model, styles, width, i, j, heading, speaker,
          heading ? headingHeight : 0, pauseLabels);
      for (var k = i; k < j; k++) {
        blockOf[k] = blocks.length;
      }
      blocks.add(paragraph);
      i = j;
    }
    return TranscriptLayout._(model, styles, width, blocks, blockOf, pauseLabels);
  }

  static int? _speakerFrom(List<TranscriptToken> tokens, int i) {
    for (var k = i; k < tokens.length; k++) {
      if (tokens[k].kind == TranscriptKind.gap || (k > i && tokens[k].paragraph)) {
        return null;
      }
      if (tokens[k].speaker != null) return tokens[k].speaker;
    }
    return null;
  }

  /// How wide a pause's chip is for its length: a little, plus a little a
  /// second, within bounds.
  static double chipWidth(double seconds) =>
      (8 + 10 * seconds).clamp(10.0, 48.0);

  /// About the x-height of the type.
  static double chipHeight(double size) => (size * 0.55).roundToDouble();

  static TranscriptParagraph _lay(
    TranscriptModel model,
    TranscriptStyles styles,
    double width,
    int from,
    int to,
    bool heading,
    int? speaker,
    double headingHeight,
    bool pauseLabels,
  ) {
    final tokens = model.tokens;
    final spans = <InlineSpan>[];
    final dims = <PlaceholderDimensions>[];
    final charStart = <int>[];
    final charEnd = <int>[];
    final placeholderOf = <int>[];
    var chars = 0;
    final gap = TextSpan(text: ' ', style: styles.word);
    for (var i = from; i < to; i++) {
      final t = tokens[i];
      // The gap goes before a token that asks for one, so a language
      // written without spaces runs its words together.
      if (i > from && (t.spaceBefore || t.kind == TranscriptKind.pause)) {
        spans.add(gap);
        chars += 1;
      }
      if (t.kind == TranscriptKind.pause) {
        final seconds = model.secondsOf(t.silence);
        var w = chipWidth(seconds);
        if (pauseLabels) {
          final label = TextPainter(
              text: TextSpan(text: pauseText(seconds), style: styles.pauseLabel),
              textDirection: TextDirection.ltr)
            ..layout();
          w = math.max(w, label.width + 8);
          label.dispose();
        }
        final size = Size(w, chipHeight(styles.size));
        spans.add(WidgetSpan(
            alignment: PlaceholderAlignment.middle,
            child: SizedBox.fromSize(size: size)));
        dims.add(PlaceholderDimensions(
            size: size, alignment: PlaceholderAlignment.middle));
        placeholderOf.add(dims.length - 1);
        charStart.add(chars);
        chars += 1;
        charEnd.add(chars);
      } else {
        final style = t.kind == TranscriptKind.removed
            ? styles.removed
            : t.isFiller
                ? styles.filler
                : t.confidence < 0.5
                    ? styles.uncertain
                    : styles.word;
        spans.add(TextSpan(text: t.text, style: style));
        placeholderOf.add(-1);
        charStart.add(chars);
        chars += t.text.length;
        charEnd.add(chars);
      }
    }
    final painter = TextPainter(
      text: TextSpan(style: styles.word, children: spans),
      textDirection: TextDirection.ltr,
    );
    if (dims.isNotEmpty) painter.setPlaceholderDimensions(dims);
    painter.layout(maxWidth: width);
    return TranscriptParagraph(
      from: from,
      to: to,
      heading: heading,
      speaker: speaker,
      headingHeight: headingHeight,
      painter: painter,
      charStart: charStart,
      charEnd: charEnd,
      placeholderOf: placeholderOf,
    );
  }

  /// A pause's or a break's length as the page writes it.
  static String pauseText(double seconds) => '${seconds.toStringAsFixed(1)} s';

  double heightOf(int block) => blocks[block].height;

  /// Where each block starts down the page, and the page's whole height
  /// after them, summed once.
  late final List<double> tops = () {
    final out = List<double>.filled(blocks.length + 1, 0);
    for (var i = 0; i < blocks.length; i++) {
      out[i + 1] = out[i] + blocks[i].height;
    }
    return out;
  }();

  /// How far down the page block [block] starts.
  double topOf(int block) => tops[block.clamp(0, blocks.length)];

  double get totalHeight => tops.last;

  /// The block at [y] down the page, or null off it.
  int? blockAt(double y) {
    if (blocks.isEmpty || y < 0 || y >= totalHeight) return null;
    var lo = 0, hi = blocks.length - 1;
    while (lo < hi) {
      final mid = (lo + hi + 1) >> 1;
      if (tops[mid] <= y) {
        lo = mid;
      } else {
        hi = mid - 1;
      }
    }
    return lo;
  }

  /// The token nearest a point on the page, for a drag that has left the
  /// words: above the page it is the first, below it the last.
  int nearest(double x, double y) {
    if (model.tokens.isEmpty) return 0;
    if (y < 0) return 0;
    if (y >= totalHeight) return model.tokens.length - 1;
    final b = blockAt(y)!;
    final block = blocks[b];
    if (block is! TranscriptParagraph) return block.from;
    final local = y - tops[b] - block.headingHeight;
    var line = block.lines.length - 1;
    for (var i = 0; i < block.lines.length; i++) {
      final m = block.lines[i];
      if (local < m.baseline - m.ascent + m.height) {
        line = i;
        break;
      }
    }
    return block.tokenOnLine(line, x).clamp(block.from, block.to - 1);
  }

  /// The paragraph holding [token], or null for a break or off the page.
  TranscriptParagraph? paragraphOf(int token) {
    if (token < 0 || token >= blockOfToken.length) return null;
    final block = blocks[blockOfToken[token]];
    return block is TranscriptParagraph ? block : null;
  }

  /// The token a caret lands on moving one line up or down from before
  /// [token], keeping [x]. Null at the page's ends.
  int? lineStep(int token, double x, {required bool down}) {
    if (model.tokens.isEmpty) return null;
    final at = token.clamp(0, model.tokens.length - 1);
    final atEnd = token >= model.tokens.length;
    var block = blockOfToken[atEnd ? model.tokens.length - 1 : at];
    var here = blocks[block];
    var line = here is TranscriptParagraph
        ? here.lineOf(atEnd ? here.to : token)
        : 0;
    while (true) {
      if (here is TranscriptParagraph) {
        final next = down ? line + 1 : line - 1;
        if (next >= 0 && next < here.lines.length) {
          return here.tokenOnLine(next, x);
        }
      }
      block += down ? 1 : -1;
      if (block < 0 || block >= blocks.length) return null;
      here = blocks[block];
      if (here is TranscriptParagraph) {
        line = down ? -1 : here.lines.length;
      } else {
        return here.from;
      }
    }
  }

  void dispose() {
    for (final b in blocks) {
      if (b is TranscriptParagraph) b.painter.dispose();
    }
  }
}

/// What the painter lights up over the words.
class TranscriptMarks {
  final TranscriptSelection selection;

  /// The word the playhead is on.
  final int? current;

  /// The results of a search or a review, and the one in hand.
  final Set<int> found;
  final int? foundCurrent;

  /// The words of the clips picked in the Cut timeline.
  final Set<String> pickedClips;

  /// A drop caret while words are dragged: before this token.
  final int? drop;

  /// Whether the caret is drawn this blink.
  final bool caretOn;

  const TranscriptMarks({
    this.selection = TranscriptSelection.none,
    this.current,
    this.found = const {},
    this.foundCurrent,
    this.pickedClips = const {},
    this.drop,
    this.caretOn = true,
  });
}

/// The colours the page is painted in, from the theme.
class TranscriptPaint {
  final Color accent;
  final Color selectionFill;
  final Color marker;
  final Color surface3;
  final Color hairlineStrong;
  final double radius;

  const TranscriptPaint({
    required this.accent,
    required this.selectionFill,
    required this.marker,
    required this.surface3,
    required this.hairlineStrong,
    required this.radius,
  });
}

/// One paragraph: its fills, its words, its chips, its carets.
class TranscriptParagraphPainter extends CustomPainter {
  final TranscriptParagraph paragraph;
  final TranscriptModel model;
  final TranscriptMarks marks;
  final TranscriptPaint colours;
  final TextStyle pauseLabel;
  final bool pauseLabels;

  const TranscriptParagraphPainter({
    required this.paragraph,
    required this.model,
    required this.marks,
    required this.colours,
    required this.pauseLabel,
    required this.pauseLabels,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final p = paragraph;
    final radius = Radius.circular(colours.radius);
    void fill(List<Rect> rects, Color colour,
        {double inset = 0, bool stroke = false}) {
      final paint = Paint()..color = colour;
      if (stroke) {
        paint
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1;
      }
      for (final r in _joined(rects)) {
        canvas.drawRRect(
            RRect.fromRectAndRadius(r.inflate(inset), radius), paint);
      }
    }

    // The clips picked in the Cut timeline, faintest and underneath.
    if (marks.pickedClips.isNotEmpty) {
      var run = -1;
      for (var i = p.from; i <= p.to; i++) {
        final on = i < p.to &&
            model.tokens[i].clip != null &&
            marks.pickedClips.contains(model.tokens[i].clip);
        if (on && run < 0) run = i;
        if (!on && run >= 0) {
          fill(p.rectsOfRun(run, i), colours.surface3);
          run = -1;
        }
      }
    }
    // Search results and review items, the one in hand stronger and
    // outlined.
    for (final i in marks.found) {
      if (i < p.from || i >= p.to) continue;
      final current = i == marks.foundCurrent;
      fill(p.rectsOf(i), colours.marker.withValues(alpha: current ? 0.45 : 0.2),
          inset: 1);
      if (current) fill(p.rectsOf(i), colours.accent, inset: 0.5, stroke: true);
    }
    // The selection, joined through the gaps between neighbours.
    for (final (a, b) in marks.selection.ranges) {
      final lo = math.max(a, p.from), hi = math.min(b, p.to);
      if (hi > lo) fill(p.rectsOfRun(lo, hi), colours.selectionFill);
    }
    // The word the playhead is on.
    if (marks.current case final c? when c >= p.from && c < p.to) {
      fill(p.rectsOf(c), colours.accent.withValues(alpha: 0.42), inset: 1);
    }

    p.painter.paint(canvas, Offset(0, p.headingHeight));

    // Pause chips at their placeholders.
    final boxes = p.painter.inlinePlaceholderBoxes ?? const [];
    final chip = Paint()..color = colours.surface3;
    for (var i = 0; i < p.placeholderOf.length; i++) {
      final k = p.placeholderOf[i];
      if (k < 0 || k >= boxes.length) continue;
      final r = boxes[k].toRect().shift(Offset(0, p.headingHeight));
      canvas.drawRRect(
          RRect.fromRectAndRadius(r, Radius.circular(r.height / 2)), chip);
      if (pauseLabels) {
        final seconds = model.secondsOf(model.tokens[p.from + i].silence);
        final label = TextPainter(
            text: TextSpan(
                text: TranscriptLayout.pauseText(seconds), style: pauseLabel),
            textDirection: TextDirection.ltr)
          ..layout();
        label.paint(
            canvas,
            Offset(r.center.dx - label.width / 2,
                r.center.dy - label.height / 2));
        label.dispose();
      }
    }
    // Edit points: a hairline between the words, x-height tall.
    final edit = Paint()
      ..color = colours.hairlineStrong
      ..strokeWidth = 1;
    final xHeight = TranscriptLayout.chipHeight(model.tokens.isEmpty
        ? 13
        : (p.painter.text?.style?.fontSize ?? 13));
    for (var i = p.from; i < p.to; i++) {
      if (!model.tokens[i].editBefore || i == p.from) continue;
      // Removed words share one edit point: the mark stands before the
      // first of them.
      if (model.tokens[i].kind == TranscriptKind.removed &&
          model.tokens[i - 1].kind == TranscriptKind.removed) {
        continue;
      }
      final rects = p.rectsOf(i);
      if (rects.isEmpty) continue;
      final r = rects.first;
      final x = (r.left - 2).roundToDouble() + 0.5;
      final mid = r.center.dy;
      canvas.drawLine(
          Offset(x, mid - xHeight / 2), Offset(x, mid + xHeight / 2), edit);
    }
    // The caret, and the drop caret while words are in hand.
    void caret(int token, Color colour) {
      final c = p.caretAt(token);
      if (c == null) return;
      final x = c.x.roundToDouble();
      canvas.drawRect(
          Rect.fromLTRB(x - 0.5, c.top + 1, x + 1, c.bottom - 1),
          Paint()..color = colour);
    }

    final drop = marks.drop;
    final c = marks.selection.caret;
    if (drop != null && drop >= p.from && drop <= p.to) {
      caret(drop, colours.accent);
    } else if (marks.caretOn && c != null && c >= p.from && c <= p.to) {
      // A caret at a paragraph's end is the next paragraph's start too, and
      // the later paragraph draws it. Only the page's end is drawn here.
      if (c < p.to || p.to == model.tokens.length) caret(c, colours.accent);
    }
  }

  /// Boxes on one line that touch or nearly touch, as one. The text engine
  /// hands a run back a box per style run, so struck-through words came as
  /// one box each.
  static List<Rect> _joined(List<Rect> rects) {
    final out = <Rect>[];
    for (final r in rects) {
      if (out.isNotEmpty &&
          (out.last.top - r.top).abs() < 1 &&
          r.left - out.last.right < 4) {
        out.last = Rect.fromLTRB(math.min(out.last.left, r.left), out.last.top,
            math.max(out.last.right, r.right), math.max(out.last.bottom, r.bottom));
      } else {
        out.add(r);
      }
    }
    return out;
  }

  @override
  bool shouldRepaint(TranscriptParagraphPainter old) =>
      old.paragraph != paragraph ||
      old.marks != marks ||
      old.colours != colours ||
      old.pauseLabels != pauseLabels;
}
