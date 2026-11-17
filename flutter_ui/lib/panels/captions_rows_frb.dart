// The Captions panel's rows: one caption each, its times, its words as they
// read and how fast they ask to be read.
//
// Every row's height is known before it is drawn (its words laid out at the
// panel's width, and whether the times sit side by side or one over the
// other), so the list can be a lazy one and still be scrolled straight to any
// caption of thousands.

import 'package:flutter/rendering.dart' show RenderParagraph;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/captions.dart';

import '../l10n/strings.dart';
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'caption_layers.dart';

/// One caption as the list draws it: the caption, where it sits in frames,
/// and its words cut into the lines it reads in.
class CaptionRow {
  final BridgeCue cue;
  final int start;
  final int end;
  final List<List<BridgeCueWord>> lines;

  /// Its words as typed: lines on lines of their own, no speaker label.
  final String text;

  CaptionRow._(this.cue, this.start, this.end, this.lines, this.text);

  factory CaptionRow.of(BridgeCue cue, (int, int) fps) {
    final lines = <List<BridgeCueWord>>[];
    if (cue.words.isEmpty) {
      for (final line in cue.text.split('\n')) {
        lines.add([BridgeCueWord(text: line, emphasis: false)]);
      }
    } else {
      final breaks = cue.breaks.toSet();
      for (var i = 0; i < cue.words.length; i++) {
        if (i == 0 || breaks.contains(i)) lines.add([]);
        lines.last.add(cue.words[i]);
      }
    }
    final text = [
      for (final line in lines) line.map((w) => w.text).join(' '),
    ].join('\n');
    return CaptionRow._(cue, captionFrame(cue.start!, fps),
        captionFrame(cue.end!, fps), lines, text);
  }

  int get index => cue.index;
}

/// The room a row's text and times take.
const double captionLineHeight = 15;
const double _pad = 5;

/// Every row's height, from its words laid out at the width they have, so
/// the list knows where each row is before it is drawn. Kept per caption
/// until its words change, and all of it until the width, face or text size
/// does.
class CaptionRowHeights {
  final Map<int, (String, double)> _known = {};
  double _width = -1;
  TextStyle? _style;
  TextScaler _scaler = TextScaler.noScaling;

  /// The words' column is [width] wide, in [style] at [scaler].
  void lay(double width, TextStyle style, TextScaler scaler) {
    if (width == _width && style == _style && scaler == _scaler) return;
    _known.clear();
    _width = width;
    _style = style;
    _scaler = scaler;
  }

  /// How tall [row] is: all of its words, or its two times when they are
  /// stacked, whichever is more. One being typed into gets room for three
  /// lines.
  double of(CaptionRow row,
      {required bool wide, bool typing = false, bool timing = false}) {
    final style = _style;
    var words = captionLineHeight;
    if (style != null && _width > 0) {
      final key = '${row.cue.label ?? ''}\n${row.text}';
      final known = _known[row.index];
      if (known != null && known.$1 == key) {
        words = known.$2;
      } else {
        final painter = TextPainter(
          text: captionRowSpan(row, style, label: style, accent: style),
          textDirection: TextDirection.ltr,
          textScaler: _scaler,
        )..layout(maxWidth: _width);
        words = painter.height.ceilToDouble();
        painter.dispose();
        _known[row.index] = (key, words);
      }
    }
    final least = [
      if (!wide) 2 * captionLineHeight,
      if (typing) 3 * captionLineHeight,
    ].fold(words, (a, b) => a > b ? a : b);
    return least + _pad * 2 + (typing || timing ? 6 : 0);
  }
}

/// A caption's words as its row draws them: the speaker label in [label],
/// emphasised words in [accent], a line to a line, and what a find has found
/// under [wash].
TextSpan captionRowSpan(CaptionRow row, TextStyle style,
    {required TextStyle label,
    required TextStyle accent,
    List<int> found = const [],
    int findLength = 0,
    Color? wash}) {
  final spans = <InlineSpan>[
    if (row.cue.label?.isNotEmpty ?? false)
      TextSpan(text: '${row.cue.label} ', style: label),
  ];
  var offset = 0;
  for (var i = 0; i < row.lines.length; i++) {
    if (i > 0) spans.add(TextSpan(text: '\n', style: style));
    for (var w = 0; w < row.lines[i].length; w++) {
      final word = row.lines[i][w];
      if (w > 0) {
        spans.add(TextSpan(text: ' ', style: style));
        offset++;
      }
      final base = word.emphasis ? accent : style;
      // A found stretch is washed letter by letter, so a find across two
      // words or inside one reads as it was typed.
      var run = StringBuffer();
      bool? washed;
      void flush() {
        if (run.isEmpty) return;
        spans.add(TextSpan(
            text: run.toString(),
            style:
                washed == true ? base.copyWith(backgroundColor: wash) : base));
        run = StringBuffer();
      }

      for (var c = 0; c < word.text.length; c++) {
        final at = offset + c;
        final hit = found.any((f) => at >= f && at < f + findLength);
        if (washed != null && hit != washed) flush();
        washed = hit;
        run.write(word.text[c]);
      }
      flush();
      offset += word.text.length;
    }
    offset++; // the line's end
  }
  return TextSpan(children: spans);
}

/// Which of [row]'s words is drawn under [global] in [paragraph], by its place
/// in the caption's words, or null off the words.
int? captionWordAt(RenderParagraph paragraph, CaptionRow row, Offset global) {
  if (row.cue.words.isEmpty || !paragraph.hasSize) return null;
  final local = paragraph.globalToLocal(global);
  final label = row.cue.label;
  var at = label?.isNotEmpty ?? false ? label!.length + 1 : 0;
  var index = 0;
  for (final line in row.lines) {
    for (final word in line) {
      final boxes = paragraph.getBoxesForSelection(
          TextSelection(baseOffset: at, extentOffset: at + word.text.length));
      if (boxes.any((b) => b.toRect().inflate(1).contains(local))) {
        return index;
      }
      at += word.text.length + 1; // the space, or the line's end
      index++;
    }
  }
  return null;
}

/// The times' face: mono, small and quiet.
TextStyle captionTimeStyle(LumitTheme t) =>
    t.mono.copyWith(fontSize: 10, color: t.textMuted, height: 1.5);

/// Where a find term falls in [text], as start offsets.
List<int> captionMatches(String text, String find,
    {required bool matchCase, required bool wholeWord}) {
  if (find.isEmpty) return const [];
  final hay = matchCase ? text : text.toLowerCase();
  final needle = matchCase ? find : find.toLowerCase();
  bool letter(int at) =>
      at >= 0 &&
      at < text.length &&
      RegExp(r'[\p{L}\p{N}]', unicode: true).hasMatch(text[at]);
  final out = <int>[];
  var from = 0;
  while (true) {
    final at = hay.indexOf(needle, from);
    if (at < 0) break;
    if (!wholeWord || (!letter(at - 1) && !letter(at + needle.length))) {
      out.add(at);
    }
    from = at + needle.length;
  }
  return out;
}

/// One caption's row.
class CaptionRowView extends StatefulWidget {
  final CaptionRow row;
  final (int, int) fps;
  final int startFrame;
  final bool wide;
  final double timeWidth;
  final bool selected;
  final bool current;

  /// Above this reading speed the figure turns amber. Nought: never.
  final int readingSpeed;
  final String find;
  final bool matchCase;
  final bool wholeWord;

  /// The field typed into in place of the words, when this row is being
  /// typed into, and the one for a time being changed.
  final Widget? typing;
  final Widget? timing;

  /// Which time [timing] stands in for: the end, or the start.
  final bool timingEnd;
  final void Function(TapDownDetails) onTapDown;
  final VoidCallback onDoubleTap;
  final void Function(bool end) onTimeDoubleTap;

  /// A right press at [at], over word [word] of the caption's words or off
  /// them.
  final void Function(Offset at, int? word) onMenu;

  const CaptionRowView({
    super.key,
    required this.row,
    required this.fps,
    required this.startFrame,
    required this.wide,
    required this.timeWidth,
    required this.selected,
    required this.current,
    required this.readingSpeed,
    required this.find,
    required this.matchCase,
    required this.wholeWord,
    required this.onTapDown,
    required this.onDoubleTap,
    required this.onTimeDoubleTap,
    required this.onMenu,
    this.typing,
    this.timing,
    this.timingEnd = false,
  });

  @override
  State<CaptionRowView> createState() => _CaptionRowViewState();
}

class _CaptionRowViewState extends State<CaptionRowView> {
  /// The words as drawn, to find the one under a right press.
  final GlobalKey _words = GlobalKey();

  String _time(int frame) => timecodeOfRate(
      frame + widget.startFrame, widget.fps.$1, widget.fps.$2);

  int? _wordAt(Offset global) {
    final paragraph = _words.currentContext?.findRenderObject();
    return paragraph is RenderParagraph
        ? captionWordAt(paragraph, widget.row, global)
        : null;
  }

  @override
  Widget build(BuildContext context) {
    final w = widget;
    final row = w.row;
    final t = ThemeScope.of(context).theme;
    final timeStyle = captionTimeStyle(t);
    Widget time(bool end) {
      if (w.timing != null && w.timingEnd == end) {
        return SizedBox(width: w.timeWidth + 8, child: w.timing);
      }
      return GestureDetector(
        behavior: HitTestBehavior.opaque,
        onDoubleTap: () => w.onTimeDoubleTap(end),
        child: SizedBox(
          width: w.timeWidth + 8,
          height: captionLineHeight,
          child: Text(_time(end ? row.end : row.start),
              key: ValueKey<String>(
                  'caption-${end ? 'out' : 'in'}-${row.index}'),
              maxLines: 1,
              style:
                  end ? timeStyle.copyWith(color: t.textDisabled) : timeStyle),
        ),
      );
    }

    final times = w.wide
        ? Row(mainAxisSize: MainAxisSize.min, children: [
            time(false),
            const SizedBox(width: 6),
            time(true),
          ])
        : Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [time(false), time(true)],
          );

    final fast = w.readingSpeed > 0 && row.cue.readingSpeed > w.readingSpeed;
    final speed = LumitTooltip(
      message: l10n.captionsReadingSpeedTip(row.cue.readingSpeed.round()),
      child: SizedBox(
        width: 22,
        child: Text(
          '${row.cue.readingSpeed.round()}',
          key: ValueKey<String>('caption-cps-${row.index}'),
          textAlign: TextAlign.end,
          maxLines: 1,
          style: timeStyle.copyWith(color: fast ? t.warning : t.textMuted),
        ),
      ),
    );

    return GestureDetector(
      key: ValueKey<String>('caption-row-${row.index}'),
      behavior: HitTestBehavior.opaque,
      onTapDown: w.onTapDown,
      onDoubleTap: w.onDoubleTap,
      onSecondaryTapDown: (d) =>
          w.onMenu(d.globalPosition, _wordAt(d.globalPosition)),
      child: HoverFill(
        faint: true,
        child: Container(
          color: w.selected ? t.selectionFill : null,
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              // The caption being said, marked down its edge.
              Container(
                width: 2,
                margin: const EdgeInsets.symmetric(vertical: 2),
                color: w.current ? t.accent : null,
              ),
              const SizedBox(width: 6),
              Padding(
                padding: const EdgeInsets.symmetric(vertical: _pad),
                child: times,
              ),
              const SizedBox(width: 2),
              Expanded(
                child: Padding(
                  padding: EdgeInsets.symmetric(
                      vertical: w.typing != null ? _pad - 2 : _pad),
                  child: w.typing ?? _wordsOf(context, t),
                ),
              ),
              const SizedBox(width: 8),
              Padding(
                padding: const EdgeInsets.symmetric(vertical: _pad),
                child: speed,
              ),
              const SizedBox(width: 8),
            ],
          ),
        ),
      ),
    );
  }

  /// The words as they read, whole: the row is as tall as they are
  /// ([CaptionRowHeights] measured the same span at the same width).
  Widget _wordsOf(BuildContext context, LumitTheme t) {
    final style = captionWordStyle(t);
    return RichText(
      key: _words,
      textScaler: MediaQuery.textScalerOf(context),
      text: captionRowSpan(widget.row, style,
          label: style.copyWith(color: t.textMuted),
          accent: style.copyWith(color: t.accent),
          found: captionMatches(widget.row.text, widget.find,
              matchCase: widget.matchCase, wholeWord: widget.wholeWord),
          findLength: widget.find.length,
          wash: t.accent.withValues(alpha: 0.28)),
    );
  }
}

/// The words' face, in a row and in the field a caption is typed into.
TextStyle captionWordStyle(LumitTheme t) =>
    t.body.copyWith(color: t.textPrimary, height: 15 / 11);

/// The field a caption is typed into: Enter keeps it, Shift+Enter starts a
/// new line, Escape puts it back.
class CaptionTypingField extends StatelessWidget {
  final TextEditingController controller;
  final FocusNode focusNode;
  final VoidCallback onCommit;
  final VoidCallback onCancel;
  const CaptionTypingField({
    super.key,
    required this.controller,
    required this.focusNode,
    required this.onCommit,
    required this.onCancel,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Focus(
      canRequestFocus: false,
      skipTraversal: true,
      onKeyEvent: (_, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        final enter = event.logicalKey == LogicalKeyboardKey.enter ||
            event.logicalKey == LogicalKeyboardKey.numpadEnter;
        if (enter && !HardwareKeyboard.instance.isShiftPressed) {
          onCommit();
          return KeyEventResult.handled;
        }
        return KeyEventResult.ignored;
      },
      child: HouseTextField(
        key: const ValueKey('caption-typing'),
        controller: controller,
        focusNode: focusNode,
        multiline: true,
        topAlign: true,
        autofocus: true,
        style: captionWordStyle(t),
        width: double.infinity,
        onTapOutside: onCommit,
        onCancelled: onCancel,
      ),
    );
  }
}
