// The Transcript panel: the words of the fronted composition, or of the
// source view's footage, as a page. Picking words picks their clips in the
// Cut timeline and moves the playhead; deleting words cuts them out.
//
// The engine says what a word, a pause or a break is and where every edit
// lands. This file holds the interaction: which words are picked, where the
// caret is, what is scrolled into view, and which review is open.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/foundation.dart' show setEquals, visibleForTesting;
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart'
    show
        Clipboard,
        ClipboardData,
        HardwareKeyboard,
        KeyEvent,
        KeyRepeatEvent,
        KeyUpEvent,
        LogicalKeyboardKey;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart' show LumitUiState;
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/cut.dart';
import 'package:lumit_flutter/src/rust/api/cut/transcript.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart' show BridgeRational;
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/transcript.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart' show LumitIcon;
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/profile_chip.dart' show askProfileName;
import '../shell/splash.dart' show showBusyWhile;
import '../shell/speech_install_dialog_frb.dart' show transcribeWithPrompt;
import '../state/app_state.dart';
import '../state/dock.dart';
import '../state/file_dialogs.dart';
import '../state/settings.dart';
import '../state/timecode.dart';
import '../state/viewer_views.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/drag_escape.dart';
import '../widgets/escape_ladder.dart';
import 'placeholder.dart';
import 'timeline_extras_frb.dart' show showMenuAt;
import 'timeline_toolbar_frb.dart' show timelineChromeControl;
import 'transcript_find.dart';
import 'transcript_text.dart';
import 'transcript_windows.dart';

part 'transcript_keys.dart';

/// The engine's "none" for an index.
const int _none = 0xFFFFFFFF;

/// The page's side padding, as the Notes panel's, and the widest a line
/// gets.
const double _pagePad = 10;
const double _pageWidth = 680;
const double _headingHeight = 22;

/// The panel's model from the engine's, with pauses left out when they are
/// not shown.
TranscriptModel _modelOf(BridgeCutTranscript b, {required bool showPauses}) {
  int? index(int v) => v == _none ? null : v;
  return TranscriptModel(
    tokens: [
      for (final s in b.tokens)
        if (showPauses || s.kind != BridgeSpokenKind.pause)
          TranscriptToken(
            kind: switch (s.kind) {
              BridgeSpokenKind.word => TranscriptKind.word,
              BridgeSpokenKind.pause => TranscriptKind.pause,
              BridgeSpokenKind.break_ => TranscriptKind.gap,
              BridgeSpokenKind.removed => TranscriptKind.removed,
            },
            text: s.text,
            startFrame: s.startFrame,
            endFrame: s.endFrame,
            heardStartFrame: s.heardStartFrame,
            heardEndFrame: s.heardEndFrame,
            item: index(s.item),
            itemId: s.item < b.items.length
                ? b.items[s.item].internalid.toString()
                : null,
            index: index(s.index),
            clip: s.clip != _none && s.clip < b.clips.length
                ? b.clips[s.clip]
                : null,
            speaker: index(s.speaker),
            confidence: s.confidence,
            paragraph: s.paragraph,
            filler: switch (s.filler) {
              BridgeFiller.none => TranscriptFiller.none,
              BridgeFiller.hesitation => TranscriptFiller.hesitation,
              BridgeFiller.phrase => TranscriptFiller.phrase,
              BridgeFiller.repeat => TranscriptFiller.repeat,
              BridgeFiller.retake => TranscriptFiller.retake,
            },
            tight: s.tight,
            editBefore: s.editBefore,
            spaceBefore: s.spaceBefore,
          ),
    ],
    speakers: [
      for (final s in b.speakers) TranscriptSpeaker(id: s.id, name: s.name),
    ],
    fpsNum: b.fpsNum,
    fpsDen: b.fpsDen,
    language: b.language,
  );
}

class TranscriptPanelFrb extends StatefulWidget {
  const TranscriptPanelFrb({super.key, this.initialReview});

  /// A results bar open from the first frame, for the width sweep.
  @visibleForTesting
  final TranscriptFindMode? initialReview;

  @override
  State<TranscriptPanelFrb> createState() => _TranscriptPanelFrbState();
}

class _TranscriptPanelFrbState extends State<TranscriptPanelFrb> {
  late final LumitState _app = Provider.of<LumitState>(context, listen: false);
  late final LumitUiState _ui =
      Provider.of<LumitUiState>(context, listen: false);

  InterfaceSettings get _s => _ui.workspace.interface;

  /// Whether the page shows the source view's footage rather than the
  /// composition. Follows whichever side of the Viewer was fronted last, and
  /// the strip's switch.
  bool _source = false;
  ViewMode? _lastViewMode;

  /// What the engine last sent, and the page made of it.
  BridgeCutTranscript? _bridge;
  TranscriptModel _model = TranscriptModel.empty;
  bool _reading = false;
  bool _failed = false;

  /// The read in flight. An answer older than the newest asked for is dropped.
  int _readSeq = 0;
  String? _readStamp;

  /// What the page last read: the side and the item or composition.
  String? _subject;

  /// Bumped as each transcription ends, so a page with no composition open
  /// still reads again once the words land.
  int _jobsDone = 0;

  /// Bumped as the document changes, which with no composition open is the
  /// only sign the source's words were edited.
  int _appChanges = 0;
  void _onApp() {
    _appChanges++;
    _read();
  }

  TranscriptLayout? _layout;
  LumitTheme? _layoutTheme;
  double _layoutSize = 0;
  bool _layoutPauseLabels = false;

  TranscriptSelection _selection = TranscriptSelection.none;

  /// Where a drag started, and whether Ctrl is adding a range to the ones
  /// already picked.
  int? _anchor;
  TranscriptSelection _before = TranscriptSelection.none;
  bool _selecting = false;

  /// The x a caret keeps as it moves up and down the lines.
  double? _goalX;

  /// Where the caret goes once the page is read again after an edit.
  int? _caretFrame;

  /// The word the playhead is on.
  int? _current;

  /// Whether the playhead is moving because the page asked, which the page
  /// does not follow.
  bool _ownSeek = false;

  /// How many of following's own scrolls are running.
  int _following = 0;

  /// The token a drag selection last reached, and the clips last asked of
  /// the Cut timeline.
  int? _lastHit;
  Set<String> _sentClips = const {};

  /// A press on picked words that may become a drag of them.
  Offset? _pressAt;
  bool _dragging = false;
  int? _drop;
  final DragEscape _drag = DragEscape();

  /// Clicks on one token in quick succession: two pick the word, three the
  /// paragraph.
  int _taps = 0;
  int? _tapToken;
  Duration _tapAt = Duration.zero;

  /// The caret's blink.
  Timer? _blink;
  bool _caretOn = true;

  /// Bumped for everything that repaints the words without laying them out
  /// again: the playhead, the selection, the caret's blink.
  final ValueNotifier<int> _repaint = ValueNotifier(0);
  void _touch() => _repaint.value++;

  final TranscriptFind _find = TranscriptFind();

  /// The heard footage with no transcript, by name and id, and the set the
  /// row's close mark hid, which stays hidden until the set changes.
  List<String> _untranscribedNames = const [];
  Set<String> _untranscribedIds = const {};
  Set<String>? _hiddenUntranscribed;

  /// The page scrolling itself while a drag sits near its top or bottom
  /// edge, and the last move to carry on from after each step.
  Timer? _edgeScroll;
  double _edgeSpeed = 0;
  PointerMoveEvent? _lastMove;
  final ScrollController _scroll = ScrollController();
  bool _followPaused = false;

  /// An inline correction over the selected words, and the words it
  /// replaces, taken when it opened.
  TextEditingController? _correct;
  ({FootageReference item, int at, int remove, String was})? _correctRun;
  final FocusNode _correctFocus = FocusNode();

  VoidCallback? _escapeRelease;
  bool _claimed = false;
  bool Function()? _heldDelete, _heldCopy, _heldCut, _heldPaste;

  @override
  void initState() {
    super.initState();
    _escapeRelease = EscapeLadder.register(EscapeRung.selection, _escapeClaim);
    _app.addListener(_onApp);
    _ui.model.addListener(_read);
    _ui.addListener(_read);
    _ui.views.addListener(_onViews);
    _ui.workspace.addListener(_onSettings);
    _ui.playheadFrame.addListener(_onPlayhead);
    _ui.playing.addListener(_onPlaying);
    _ui.sourcePlaying.addListener(_onPlaying);
    _ui.cutClips.addListener(_touch);
    _ui.activePane.addListener(_onActivePanel);
    _ui.selectAllRequest.addListener(_onSelectAll);
    _ui.panelSearchRequest.addListener(_onSearchRequest);
    _job.addListener(_onJob);
    _find.addListener(_onFind);
    if (widget.initialReview case final mode?) _find.show(mode);
    _find.query.addListener(_onQuery);
    _scroll.addListener(_onScroll);
    _correctFocus.addListener(() {
      if (!_correctFocus.hasFocus && _correct != null) _endCorrect(commit: true);
    });
    HardwareKeyboard.instance.addHandler(_onKey);
    _onViews();
    _onActivePanel();
    _read();
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    _escapeRelease?.call();
    _drag.dispose();
    _edgeScroll?.cancel();
    _blink?.cancel();
    _app.removeListener(_onApp);
    _ui.model.removeListener(_read);
    _ui.removeListener(_read);
    _ui.views.removeListener(_onViews);
    _ui.workspace.removeListener(_onSettings);
    _ui.playheadFrame.removeListener(_onPlayhead);
    _ui.playing.removeListener(_onPlaying);
    _ui.sourcePlaying.removeListener(_onPlaying);
    _ui.cutClips.removeListener(_touch);
    _ui.activePane.removeListener(_onActivePanel);
    _ui.selectAllRequest.removeListener(_onSelectAll);
    _ui.panelSearchRequest.removeListener(_onSearchRequest);
    _job.removeListener(_onJob);
    _releaseKeys();
    // After the frame: the Cut timeline must not be told to repaint while
    // the tree is being taken down.
    final ui = _ui;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      ui.transcriptSpan.value = null;
    });
    _find.dispose();
    _scroll.dispose();
    _correct?.dispose();
    _correctFocus.dispose();
    _layout?.dispose();
    _repaint.dispose();
    super.dispose();
  }

  // ------------------------------------------------------------- the subject

  CompositionReference? get _comp => _ui.selectedComp;
  ViewerSurface? get _sourceView => _ui.sourceView;
  FootageReference? get _footage {
    final view = _sourceView;
    return view == null ? null : _ui.footageOf(view);
  }

  /// What the page's clock reads at its first frame, and whether it counts
  /// the drop-frame way: the composition's Start timecode, or the file's own
  /// as the source view reads it.
  (int, bool) get _clockStart {
    if (!_source) return (_ui.model.startFrame, false);
    final footage = _footage;
    final facts = footage == null ? null : _ui.itemFacts(footage);
    return (facts?.startFrame.toInt() ?? 0, facts?.startDropFrame ?? false);
  }

  /// The Viewer fronted its other side: the page goes with it.
  void _onViews() {
    final mode = _ui.views.active?.mode;
    if (mode != _lastViewMode) {
      _lastViewMode = mode;
      final source = mode == ViewMode.footage && _sourceView != null;
      if (source != _source) {
        _source = source;
        _selection = TranscriptSelection.none;
      }
    }
    if (_source && _sourceView == null) _source = false;
    _read();
    if (_source) _onPlayhead();
  }

  void _setSource(bool source) {
    if (source == _source) return;
    setState(() {
      _source = source;
      _selection = TranscriptSelection.none;
    });
    _publishSelection();
    _read();
  }

  BridgeSpokenLook get _look => BridgeSpokenLook(
      pauseMs: _s.transcriptPauseMs, showRemoved: _s.transcriptShowRemoved);

  /// Read the page again when its subject, the document or the look moved.
  /// The last page stays up until the new one arrives.
  Future<void> _read() async {
    if (!mounted) return;
    final comp = _comp;
    final footage = _source ? _footage : null;
    int revision;
    try {
      revision = comp?.documentRevision() ?? _appChanges;
    } catch (_) {
      revision = 0;
    }
    final subject = '$_source/${_source ? footage?.internalid : comp?.internalid}';
    if (subject != _subject) {
      // Another page: what was picked, the caret and what failed belong to
      // the last.
      _subject = subject;
      _failed = false;
      _selection = TranscriptSelection.none;
      _caretFrame = null;
    }
    final stamp = '$subject/$revision/$_jobsDone/${_s.transcriptPauseMs}/'
        '${_s.transcriptShowRemoved}/${_s.transcriptShowPauses}';
    if (stamp == _readStamp) return;
    _readStamp = stamp;
    final seq = ++_readSeq;
    if ((_source && footage == null) || (!_source && comp == null)) {
      _adopt(null);
      return;
    }
    setState(() => _reading = true);
    BridgeCutTranscript? got;
    try {
      got = _source
          ? await footage!.sourceTranscript(look: _look)
          : await comp!.cutTranscript(look: _look);
    } catch (_) {
      got = null;
    }
    if (!mounted || seq != _readSeq) return;
    _adopt(got);
  }

  void _adopt(BridgeCutTranscript? b) {
    // An edit's own caret lets the selection go; anything else keeps its
    // stretches to find again, with the words each starts and ends on. The
    // caret's frame is taken from this page, before the next replaces it.
    final picked = _caretFrame == null
        ? [
            for (final r in _selection.ranges)
              if (r.$2 > r.$1)
                (
                  start: _model.tokens[r.$1].startFrame,
                  end: _model.tokens[r.$2 - 1].endFrame,
                  words: _wordEnds(r),
                ),
          ]
        : const <({int start, int end, (Object, Object)? words})>[];
    final frame = _caretFrame ??
        (_selection.caret == null ? null : _frameOfCaret(_selection.caret!));
    _bridge = b;
    final missing = b?.untranscribed ?? const <FootageReference>[];
    _untranscribedIds = {for (final f in missing) f.internalid.toString()};
    _untranscribedNames = [
      for (final f in missing)
        () {
          try {
            return ItemReference.footage(f).name();
          } catch (_) {
            return '';
          }
        }(),
    ];
    _model = b == null
        ? TranscriptModel.empty
        : _modelOf(b, showPauses: _s.transcriptShowPauses);
    _layout?.dispose();
    _layout = null;
    // The words moved under the selection: each range finds its frames
    // again and keeps only while it starts and ends on the same words, or
    // holds no words either side, and the caret finds its frame, or the
    // place an edit left.
    _caretFrame = null;
    var next = TranscriptSelection(caret: _tokenAtFrame(frame));
    for (final s in picked) {
      final from = _tokenAtFrame(s.start), to = _tokenAtFrame(s.end);
      if (from != null &&
          to != null &&
          to > from &&
          _wordEnds((from, to)) == s.words) {
        next = next.adding((from, to));
      }
    }
    _selection = next;
    _anchor = null;
    _selecting = false;
    _pressAt = null;
    _dragging = false;
    _drop = null;
    _drag.end();
    _stopEdgeScroll();
    _find.refresh(_model, _s);
    if (mounted) setState(() => _reading = false);
    // The clips under a carried selection are not picked again: the Cut
    // timeline's own pick stands.
    _publishSelection(pick: false);
    _onPlayhead();
  }

  /// The first and last word of [range] by identity, or null when it holds
  /// only pauses and breaks.
  (Object, Object)? _wordEnds((int, int) range) {
    Object? first, last;
    for (var i = range.$1; i < range.$2; i++) {
      final t = _model.tokens[i];
      if (!t.isWord) continue;
      first ??= t.identity;
      last = t.identity;
    }
    return first == null ? null : (first, last!);
  }

  int? _frameOfCaret(int caret) {
    final tokens = _model.tokens;
    if (tokens.isEmpty) return null;
    return caret >= tokens.length
        ? tokens.last.endFrame
        : tokens[caret.clamp(0, tokens.length - 1)].startFrame;
  }

  /// The first token starting at or after [frame].
  int? _tokenAtFrame(int? frame) {
    if (frame == null || _model.tokens.isEmpty) return null;
    final tokens = _model.tokens;
    for (var i = 0; i < tokens.length; i++) {
      if (tokens[i].startFrame >= frame) return i;
    }
    return tokens.length;
  }

  void _onSettings() {
    if (!mounted) return;
    _find.refresh(_model, _s);
    setState(() {});
    _read();
  }

  // ------------------------------------------------------------ the playhead

  int get _playhead =>
      _source ? (_sourceView?.sourceFrame ?? 0) : _ui.playheadFrame.value;

  void _onPlayhead() {
    final at = _playhead;
    final tokens = _model.tokens;
    int? found;
    // The word heard at the playhead: the last one starting at or before it.
    var lo = 0, hi = tokens.length - 1;
    while (lo <= hi) {
      final mid = (lo + hi) >> 1;
      if (tokens[mid].startFrame <= at) {
        found = mid;
        lo = mid + 1;
      } else {
        hi = mid - 1;
      }
    }
    if (found != null && tokens[found].endFrame <= at) found = null;
    if (found == _current) return;
    _current = found;
    _touch();
    if (found != null && !_ownSeek && _s.transcriptFollow && !_followPaused) {
      _follow(found);
    }
  }

  /// Whether either side is playing: the composition, or the source view.
  bool get _playing => _ui.playing.value || _ui.sourcePlaying.value;

  void _onPlaying() {
    // Playing again lifts the pause a scroll put on following.
    if (_playing) _followPaused = false;
  }

  /// Keep the word being played about a third of the way down.
  void _follow(int token) {
    final layout = _layout;
    if (layout == null || !_scroll.hasClients) return;
    final paragraph = layout.paragraphOf(token);
    if (paragraph == null) return;
    final rects = paragraph.rectsOf(token);
    if (rects.isEmpty) return;
    final y = layout.topOf(layout.blockOfToken[token]) + rects.first.top;
    final view = _scroll.position.viewportDimension;
    final shown = y - _scroll.offset;
    // Only once it strays from the band around the third, so playback does
    // not scroll on every word.
    if (shown > view * 0.2 && shown < view * 0.5) return;
    final to = (y - view / 3).clamp(0.0, _scroll.position.maxScrollExtent);
    final motion = ThemeScope.of(context).motion;
    _following++;
    if (motion.isStill || !_playing) {
      _scroll.jumpTo(to);
      _following--;
    } else {
      _scroll
          .animateTo(to,
              duration: const Duration(milliseconds: 180), curve: Curves.easeOut)
          .whenComplete(() => _following--);
    }
  }

  void _onScroll() {
    // Any other scroll while playing stops following until play next
    // starts, the page seeks, or the switch is pressed.
    if (_following == 0 && _playing) _followPaused = true;
  }

  void _seekTo(int frame) {
    _followPaused = false;
    _ownSeek = true;
    try {
      if (_source) {
        // A seek stops the source as it stops the composition, or its clock
        // would carry the playhead straight back.
        if (_ui.sourcePlaying.value) _ui.sourcePlayToggle?.call();
        final view = _sourceView;
        if (view != null) _ui.seekFootageView(view, math.max(0, frame));
      } else {
        _ui.scrubTo(math.max(0, frame));
      }
    } finally {
      _ownSeek = false;
    }
  }

  // ----------------------------------------------------------- the selection

  void _select(TranscriptSelection next, {bool seek = false}) {
    _selection = next;
    _goalX = null;
    _touch();
    _publishSelection();
    _restartBlink();
    if (!seek) return;
    if (next.caret case final c?) {
      final tokens = _model.tokens;
      if (c < tokens.length) {
        final t = tokens[c];
        _seekTo(t.isWord ? t.heardStartFrame : t.startFrame);
      } else if (tokens.isNotEmpty) {
        _seekTo(tokens.last.endFrame);
      }
    }
  }

  /// [next], with [anchor] as where a Shift+click extends from, or none.
  void _selectAt(TranscriptSelection next, int? anchor, {bool seek = false}) {
    _anchor = anchor;
    _select(next, seek: seek);
  }

  /// The selection's stretch and its clips, for the Cut timeline. Without
  /// [pick] the clips are noted but not asked to be picked.
  void _publishSelection({bool pick = true}) {
    if (_source) {
      _ui.transcriptSpan.value = null;
      return;
    }
    final spans = _model.spansOf(_selection.ranges);
    _ui.transcriptSpan.value = spans.isEmpty
        ? null
        : (
            start: spans.map((s) => s.start).reduce(math.min),
            end: spans.map((s) => s.end).reduce(math.max),
          );
    final clips = <String>{
      for (final (a, b) in _selection.ranges)
        for (var i = a; i < b; i++)
          if (_model.tokens[i].clip case final c?) c,
    };
    if (pick && clips.isNotEmpty && !setEquals(clips, _sentClips)) {
      _ui.cutPickRequest.value = clips;
    }
    _sentClips = clips;
  }

  /// The frame stretches the selection covers, or the token at the caret's
  /// side when nothing is picked.
  List<BridgeFrameSpan> _spans({bool before = false}) {
    var ranges = _selection.ranges;
    if (ranges.isEmpty) {
      final c = _selection.caret;
      if (c == null) return const [];
      final i = before ? c - 1 : c;
      if (i < 0 || i >= _model.tokens.length) return const [];
      ranges = [(i, i + 1)];
    }
    return [
      for (final s in _model.spansOf(ranges))
        BridgeFrameSpan(startFrame: s.start, endFrame: s.end),
    ];
  }

  bool _escapeClaim() {
    if (!mounted || _ui.activePanel != Panel.transcript) return false;
    if (_correct != null) {
      _endCorrect(commit: false);
      return true;
    }
    if (!_selection.isEmpty) {
      _selectAt(TranscriptSelection(caret: _selection.caret), null);
      return true;
    }
    if (_find.isOpen) {
      _find.close();
      return true;
    }
    return false;
  }

  void _onSelectAll() {
    if (!mounted || !_ui.selectAllRequestIsFor(Panel.transcript)) return;
    if (_model.tokens.isEmpty) return;
    _anchor = 0;
    _select(TranscriptSelection.of(0, _model.tokens.length));
  }

  void _onSearchRequest() {
    if (!mounted || !_ui.searchRequestIsFor(Panel.transcript)) return;
    _find.show(TranscriptFindMode.search);
    _find.refresh(_model, _s);
    WidgetsBinding.instance
        .addPostFrameCallback((_) => _find.queryFocus.requestFocus());
  }

  /// The result the bar last stood on, by its first word, so a page read
  /// again under an open bar leaves the selection alone while the result in
  /// hand is the same.
  Object? _shownResult;

  void _onFind() {
    if (!mounted) return;
    setState(() {});
    // The result in hand is picked and the playhead goes to it.
    final range = _find.currentRange;
    if (range == null || !_find.isOpen) {
      _shownResult = null;
      return;
    }
    final key = _model.tokens[range.$1].identity;
    if (key == _shownResult) return;
    _shownResult = key;
    _anchor = null;
    _select(TranscriptSelection.of(range.$1, range.$2), seek: true);
    // After the next layout, as an edit has just let the last one go.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_find.currentRange case final r? when mounted) _revealToken(r.$1);
    });
  }

  /// The search text last seen, as caret moves in the well notify too.
  String _queryWas = '';

  void _onQuery() {
    final text = _find.query.text;
    // New text is a search, whichever review was open.
    if (text != _queryWas &&
        text.isNotEmpty &&
        _find.mode != TranscriptFindMode.search) {
      _find.show(TranscriptFindMode.search);
    }
    _queryWas = text;
    _find.refresh(_model, _s);
  }

  /// Scroll [token] into view if it is off the page. With [sought] the
  /// playhead went there too, so the scroll is the page's own and following
  /// carries on.
  void _revealToken(int token, {bool sought = true}) {
    final layout = _layout;
    if (layout == null || !_scroll.hasClients) return;
    final block = layout.blockOfToken.isEmpty
        ? null
        : layout.blockOfToken[token.clamp(0, layout.blockOfToken.length - 1)];
    if (block == null) return;
    final top = layout.topOf(block);
    final bottom = top + layout.heightOf(block);
    final view = _scroll.position.viewportDimension;
    if (top >= _scroll.offset && bottom <= _scroll.offset + view) return;
    if (sought) _following++;
    _scroll.jumpTo((top - view / 3).clamp(0.0, _scroll.position.maxScrollExtent));
    if (sought) _following--;
  }

  // ----------------------------------------------------------------- the keys

  void _onActivePanel() {
    if (_ui.activePanel == Panel.transcript) {
      _claimKeys();
      _restartBlink();
    } else {
      _releaseKeys();
      _blink?.cancel();
      _blink = null;
      if (!_caretOn) {
        _caretOn = true;
        _touch();
      }
    }
  }

  void _restartBlink() {
    _blink?.cancel();
    _caretOn = true;
    if (_ui.activePanel != Panel.transcript || _selection.caret == null) {
      _blink = null;
      return;
    }
    _blink = Timer.periodic(const Duration(milliseconds: 500), (_) {
      _caretOn = !_caretOn;
      _touch();
    });
  }

  void _claimKeys() {
    if (_claimed) return;
    _claimed = true;
    // A slot still holding the panel's own claim keeps what it held.
    if (_ui.deleteClaim != _deleteClaim) _heldDelete = _ui.deleteClaim;
    if (_ui.copyClaim != _copyClaim) _heldCopy = _ui.copyClaim;
    if (_ui.cutClaim != _cutClaim) _heldCut = _ui.cutClaim;
    if (_ui.pasteClaim != _pasteClaim) _heldPaste = _ui.pasteClaim;
    _ui.deleteClaim = _deleteClaim;
    _ui.copyClaim = _copyClaim;
    _ui.cutClaim = _cutClaim;
    _ui.pasteClaim = _pasteClaim;
  }

  void _releaseKeys() {
    if (!_claimed) return;
    _claimed = false;
    if (_ui.deleteClaim == _deleteClaim) _ui.deleteClaim = _heldDelete;
    if (_ui.copyClaim == _copyClaim) _ui.copyClaim = _heldCopy;
    if (_ui.cutClaim == _cutClaim) _ui.cutClaim = _heldCut;
    if (_ui.pasteClaim == _pasteClaim) _ui.pasteClaim = _heldPaste;
  }

  bool get _keysAreOurs => mounted && _ui.activePanel == Panel.transcript;

  /// A key that is not the panel's goes to the claim held beneath it. Two
  /// panels can hold each other's, so a question already passing stops.
  bool _passing = false;
  bool _pass(bool Function()? held) {
    if (_passing || held == null) return false;
    _passing = true;
    try {
      return held();
    } finally {
      _passing = false;
    }
  }

  bool _deleteClaim() {
    if (!_keysAreOurs) return _pass(_heldDelete);
    _remove(ripple: true);
    return true;
  }

  // The chords stay the panel's while it is active, whatever is picked, so
  // they never fall through to the layers underneath.
  bool _copyClaim() {
    if (!_keysAreOurs) return _pass(_heldCopy);
    if (!_selection.isEmpty) _source ? _copyText() : _copy();
    return true;
  }

  bool _cutClaim() {
    if (!_keysAreOurs) return _pass(_heldCut);
    if (!_source && !_selection.isEmpty && _copy()) {
      _remove(ripple: true, name: 'Delete words');
    }
    return true;
  }

  bool _pasteClaim() {
    if (!_keysAreOurs) return _pass(_heldPaste);
    if (!_source) _paste();
    return true;
  }

  /// The panel's keys, in transcript_keys.dart. Held here so the handler
  /// removed is the one added.
  bool _onKey(KeyEvent event) => _key(event);

  void _openFind(TranscriptFindMode mode) {
    if (_find.mode == mode) {
      _find.close();
      return;
    }
    _find.show(mode);
    _find.refresh(_model, _s);
  }

  void _stepFind(int by) {
    if (!_find.isOpen) {
      _find.show(TranscriptFindMode.search);
      _find.refresh(_model, _s);
    }
    // Asked for, so the result is taken again even when it is the same one.
    _shownResult = null;
    _find.step(_model, by);
  }

  // ---------------------------------------------------------------- the edits

  /// One engine call named in the History list, said on the status line when
  /// refused, and told to everyone when it went through. Nothing is sent
  /// while a page is being read, as its frames would be the last page's.
  bool _edit(String name, BridgeCutResult Function() call) {
    final project = _app.project;
    if (project == null || _reading) return false;
    BridgeCutResult result;
    project.beginUndoGroup();
    project.nameUndoGroup(name: name);
    try {
      result = call();
    } catch (_) {
      return false;
    } finally {
      project.endUndoGroup();
    }
    _ui.reportCut(result);
    if (result != BridgeCutResult.done) return false;
    _afterWrite();
    return true;
  }

  /// A project-level write with no result to report.
  bool _write(String name, void Function() call) {
    final project = _app.project;
    if (project == null || _reading) return false;
    project.beginUndoGroup();
    project.nameUndoGroup(name: name);
    try {
      call();
    } catch (_) {
      return false;
    } finally {
      project.endUndoGroup();
    }
    _afterWrite();
    return true;
  }

  void _afterWrite() {
    _ui.model.refresh();
    _app.notifyDocumentChanged();
  }

  bool get _ripple => !_s.transcriptLift;

  /// Delete or lift the selected words, or the one beside the caret.
  void _remove({required bool ripple, bool before = false, String? name}) {
    if (_source) return;
    final comp = _comp;
    // A word said wholly under another has no frames of its own to take.
    final spans = [
      for (final s in _spans(before: before))
        if (s.endFrame > s.startFrame) s,
    ];
    if (comp == null || spans.isEmpty) return;
    _caretFrame = spans.map((s) => s.startFrame).reduce(math.min);
    final done = _edit(name ?? (ripple ? 'Delete words' : 'Lift words'),
        () => comp.cutTranscriptRemove(spans: spans, ripple: ripple));
    if (done) {
      _seekTo(_caretFrame!);
    } else {
      _caretFrame = null;
    }
  }

  bool _copy() {
    final comp = _comp;
    final spans = _spans();
    if (comp == null || spans.isEmpty) return false;
    try {
      final result = comp.cutTranscriptCopy(spans: spans);
      _ui.reportCut(result);
      if (result != BridgeCutResult.done) return false;
      // The words' text goes to the system clipboard too.
      unawaited(_textToClipboard(spans));
      return true;
    } catch (_) {
      return false;
    }
  }

  void _paste() {
    final comp = _comp;
    if (comp == null) return;
    final c = _selection.caret;
    final at = (c == null ? null : _frameOfCaret(c)) ?? _playhead;
    int? end;
    final done = _edit('Paste words', () {
      final pasted = comp.cutTranscriptPaste(atFrame: at);
      end = pasted.endFrame;
      return pasted.result;
    });
    if (done && end != null) {
      _caretFrame = end;
      _seekTo(end!);
    }
  }

  /// Drop the dragged words before [drop], moving or copying them.
  void _moveTo(int drop, {required bool copy}) {
    final comp = _comp;
    final spans = _spans();
    if (comp == null || spans.isEmpty) return;
    final to = _frameOfCaret(drop);
    if (to == null) return;
    int? end;
    final done = _edit(copy ? 'Paste words' : 'Move words', () {
      final moved = comp.cutTranscriptMove(spans: spans, toFrame: to, copy: copy);
      end = moved.endFrame;
      return moved.result;
    });
    if (done && end != null) {
      // The playhead to where the words now start.
      _caretFrame = end;
      _seekTo(end! -
          spans.fold(0, (n, s) => n + s.endFrame - s.startFrame));
    }
  }

  /// The removed words picked, by clip and item, back into the composition.
  void _restore() {
    final comp = _comp;
    final b = _bridge;
    if (comp == null || b == null || _source) return;
    final tokens = _model.tokens;
    final runs = <(String clip, int item, int from, int to)>[];
    for (final (a, c) in _selection.ranges) {
      for (var i = a; i < c; i++) {
        final t = tokens[i];
        if (t.kind != TranscriptKind.removed ||
            t.clip == null ||
            t.item == null ||
            t.index == null) {
          continue;
        }
        if (runs.isNotEmpty &&
            runs.last.$1 == t.clip &&
            runs.last.$2 == t.item &&
            runs.last.$4 + 1 == t.index) {
          runs.last = (runs.last.$1, runs.last.$2, runs.last.$3, t.index!);
        } else {
          runs.add((t.clip!, t.item!, t.index!, t.index!));
        }
      }
    }
    if (runs.isEmpty) return;
    _edit(
        'Restore words',
        () => comp.cutTranscriptRestore(runs: [
              for (final (clip, item, from, to) in runs)
                BridgeRemovedRun(
                    clip: clip, item: b.items[item], from: from, to: to),
            ]));
  }

  void _mark() {
    final comp = _comp;
    final spans = _spans();
    if (comp == null || spans.isEmpty || _source) return;
    _edit('Mark as work area', () {
      comp.cutTranscriptMark(
          span: BridgeFrameSpan(
              startFrame: spans.map((s) => s.startFrame).reduce(math.min),
              endFrame: spans.map((s) => s.endFrame).reduce(math.max)));
      return BridgeCutResult.done;
    });
  }

  bool get _hasRemoved => _selection.ranges.any((r) {
        for (var i = r.$1; i < r.$2; i++) {
          if (_model.tokens[i].kind == TranscriptKind.removed) return true;
        }
        return false;
      });

  /// The selected words as one item's run, or null when they span items,
  /// several ranges or none.
  ({int item, int from, int to, int first, int last})? get _oneRun {
    if (_selection.ranges.length != 1) return null;
    final (a, b) = _selection.ranges.single;
    int? item, from, to;
    for (var i = a; i < b; i++) {
      final t = _model.tokens[i];
      if (!t.isWord) continue;
      if (t.item == null || t.index == null) return null;
      // One item's words, one after another.
      if (item != null && (t.item != item || t.index != to! + 1)) return null;
      item = t.item;
      from ??= t.index;
      to = t.index;
    }
    if (item == null) return null;
    return (item: item, from: from!, to: to!, first: a, last: b);
  }

  void _startCorrect() {
    final run = _oneRun;
    final b = _bridge;
    if (run == null || b == null) return;
    final was = _model.textOf(run.first, run.last);
    _correctRun = (
      item: b.items[run.item],
      at: run.from,
      remove: run.to - run.from + 1,
      was: was,
    );
    _correct?.dispose();
    _correct = TextEditingController(text: was);
    _correct!.selection =
        TextSelection(baseOffset: 0, extentOffset: _correct!.text.length);
    setState(() {});
    WidgetsBinding.instance
        .addPostFrameCallback((_) => _correctFocus.requestFocus());
  }

  void _endCorrect({required bool commit}) {
    final field = _correct;
    final run = _correctRun;
    if (field == null) return;
    final text = field.text.trim();
    _correct = null;
    _correctRun = null;
    // The field is still up until the next frame.
    WidgetsBinding.instance.addPostFrameCallback((_) => field.dispose());
    if (mounted) setState(() {});
    if (!commit || run == null || text == run.was) return;
    _write(
        'Correct words',
        () => _app.project!.correctWords(edits: [
              BridgeWordsEdit(
                item: run.item,
                at: run.at,
                remove: run.remove,
                texts: text.split(RegExp(r'\s+')).where((w) => w.isNotEmpty).toList(),
              ),
            ]));
  }

  /// The selected words' runs, one per item and stretch of indices.
  List<BridgeWordRun> _wordRuns() {
    final b = _bridge;
    if (b == null) return const [];
    final runs = <BridgeWordRun>[];
    for (final (a, c) in _selection.ranges) {
      int? item, from, to;
      void flush() {
        if (item != null) {
          runs.add(BridgeWordRun(item: b.items[item!], from: from!, to: to!));
        }
        item = null;
      }

      for (var i = a; i < c; i++) {
        final t = _model.tokens[i];
        if (!t.isWord || t.item == null || t.index == null) continue;
        if (item != null && (t.item != item || t.index != to! + 1)) flush();
        if (item == null) {
          item = t.item;
          from = t.index;
        }
        to = t.index;
      }
      flush();
    }
    return runs;
  }

  void _setSpeaker(String? id) {
    final runs = _wordRuns();
    if (runs.isEmpty) return;
    _write('Change speaker',
        () => _app.project!.setSpokenSpeaker(runs: runs, speaker: id));
  }

  /// A speaker nobody has yet: asked for by name, added to the first
  /// selected word's item, and given the selected words, as one step.
  Future<void> _newSpeaker() async {
    final name = await askProfileName(context,
        title: l10n.transcriptSpeakerName, confirm: l10n.apply, hint: l10n.name);
    if (name == null || name.trim().isEmpty || !mounted) return;
    final runs = _wordRuns();
    if (runs.isEmpty) return;
    _write('Change speaker', () {
      final id = runs.first.item.addSpeaker(name: name.trim());
      _app.project!.setSpokenSpeaker(runs: runs, speaker: id);
    });
  }

  void _renameSpeaker(TranscriptSpeaker speaker, String name) => _write(
      'Rename speaker',
      () => _app.project!.renameSpeaker(speaker: speaker.id, name: name));

  /// Transcribe the heard footage with no transcript, or all of it again.
  Future<void> _transcribe({bool again = false}) async {
    final b = _bridge;
    if (b == null) return;
    final items = again ? [...b.items, ...b.untranscribed] : b.untranscribed;
    if (items.isEmpty) return;
    setState(() => _failed = false);
    final outcome = await transcribeWithPrompt(
        context,
        items,
        BridgeTranscribeOptions(
          language: _language,
          speakers: _speakerCount,
          translate: false,
          speakerName: l10n.captionsSpeakerName('{n}'),
        ));
    // Nothing landed and something went wrong: the failure line. A job
    // turned down at the install prompt is not a failure.
    if (mounted && outcome != null && outcome.done == 0 && outcome.failed > 0) {
      setState(() => _failed = true);
    }
  }

  /// The running job's latest step, null when idle.
  ValueNotifier<BridgeTranscribeEvent?> get _job => _app.transcription.progress;

  void _onJob() {
    final event = _job.value;
    if (event == null) {
      _jobsDone++;
      _read();
    } else if (event.stage == BridgeTranscribeStage.failed && !_failed) {
      setState(() => _failed = true);
    }
  }

  /// What a step says, for the bar: how far, the stage's word, and whether
  /// it stopped badly.
  ({double fraction, String stage, bool failed})? _jobView(
      BridgeTranscribeEvent? event) {
    if (event == null) return null;
    final stage = switch (event.stage) {
      BridgeTranscribeStage.loading => l10n.transcriptStageLoading,
      BridgeTranscribeStage.listening => l10n.transcriptStageListening,
      BridgeTranscribeStage.speakers => l10n.transcriptStageSpeakers,
      // A cancelled job is let go before it is shown.
      BridgeTranscribeStage.done || BridgeTranscribeStage.cancelled => l10n.done,
      BridgeTranscribeStage.failed => l10n.transcriptFailed,
    };
    return (
      fraction: event.fraction,
      stage: stage,
      failed: event.stage == BridgeTranscribeStage.failed,
    );
  }

  /// The languages the speech model can hear, by code and name. Empty is
  /// Detect alone.
  late final List<(String, String)> _languages = [
    for (final l in transcriptionLanguages()) (l.code, l.name),
  ];

  /// The language and speaker count the Transcribe button asks for. Empty
  /// and zero leave both to the job.
  String _language = '';
  int _speakerCount = 0;

  // ------------------------------------------------------- the source's words

  /// The selected words' stretches as the item's own seconds.
  List<BridgeSourceSpan> _sourceSpans() => [
        for (final s in _model.spansOf(_selection.ranges))
          BridgeSourceSpan(
            start: BridgeRational(num: s.start * _model.fpsDen, den: _model.fpsNum),
            end: BridgeRational(num: s.end * _model.fpsDen, den: _model.fpsNum),
          ),
      ];

  /// [spans] of the source into [comp] at [at], one after another.
  bool _placeInto(CompositionReference comp, FootageReference footage,
      List<BridgeSourceSpan> spans, int at,
      {required bool insert, required String name}) {
    if (spans.isEmpty || _reading) return false;
    final fronted = comp.internalid == _comp?.internalid;
    int? end;
    final done = _ui.placeLikeSource(
      name: name,
      (target, linked, withPicture, arrival) {
        final placed = comp.cutPlaceWords(
          // The target is a layer of the fronted composition.
          target: fronted ? target : null,
          footage: footage,
          spans: spans,
          atFrame: at,
          insert: insert,
          linked: linked,
          withPicture: withPicture,
          arrival: arrival,
        );
        end = placed.endFrame;
        return placed.result;
      },
    );
    if (done && end != null && fronted) _ui.scrubTo(end!);
    return done;
  }

  void _placeWords({required bool insert}) {
    final comp = _comp;
    final footage = _footage;
    if (!_source || comp == null || footage == null) return;
    _placeInto(comp, footage, _sourceSpans(), _ui.playheadFrame.value,
        insert: insert, name: insert ? 'Insert words' : 'Overwrite with words');
  }

  /// After the composition's last clip.
  void _appendWords() {
    final comp = _comp;
    final footage = _footage;
    if (!_source || comp == null || footage == null) return;
    var end = 0;
    for (final entry in _ui.model.layers) {
      for (final clip in entry.info.clips) {
        end = math.max(end, clip.endFrame);
      }
    }
    _placeInto(comp, footage, _sourceSpans(), end,
        insert: true, name: 'Append words');
  }

  /// A composition sized like the footage, with the selected words in it,
  /// as one step, fronted once they are in.
  void _newCompositionFrom() {
    final footage = _footage;
    final project = _app.project;
    // Taken first: fronting the composition lets the selection go.
    final spans = _sourceSpans();
    if (!_source || footage == null || project == null || spans.isEmpty) {
      return;
    }
    final facts = _ui.itemFacts(footage);
    final name = ItemReference.footage(footage).name();
    CompositionReference? made;
    _write('Insert words', () {
      final comp = project.newComposition(name: name);
      if (facts != null && facts.width > 0 && facts.height > 0) {
        final was = comp.getSettings();
        comp.setSettings(
          settings: BridgeCompSettings(
            name: was.name,
            width: facts.width,
            height: facts.height,
            fpsNum: facts.fpsNum > 0 ? facts.fpsNum : was.fpsNum,
            fpsDen: facts.fpsDen > 0 ? facts.fpsDen : was.fpsDen,
            duration: was.duration,
            background: was.background,
            shutterAngle: was.shutterAngle,
            motionBlurSamples: was.motionBlurSamples,
            motionBlurLimit: was.motionBlurLimit,
          ),
        );
      }
      made = comp;
      _placeInto(comp, footage, spans, 0, insert: true, name: 'Insert words');
    });
    if (made case final comp?) _ui.setSelectedComp(comp);
  }

  // ------------------------------------------------------- the results bar

  TranscriptFindActions get _findActions => TranscriptFindActions(
        previous: () => _stepFind(-1),
        next: () => _stepFind(1),
        close: _find.close,
        delete: () => _deleteResults(current: true),
        deleteAll: () => _deleteResults(current: false),
        selectAll: () {
          if (_find.results.isEmpty) return;
          var all = TranscriptSelection.none;
          for (final r in _find.results) {
            all = all.adding(r);
          }
          _anchor = null;
          _select(TranscriptSelection(ranges: all.ranges, caret: all.bounds?.$2));
        },
        keep: () {
          final r = _find.currentRange;
          if (r == null) return;
          _find.kept.add(_model.tokens[r.$1].identity);
          _find.refresh(_model, _s);
        },
        shorten: () => _shortenPauses(current: true),
        shortenAll: () => _shortenPauses(current: false),
        replace: () => _replace(current: true),
        replaceAll: () => _replace(current: false),
        setLift: (lift) => _toggle((s) => s.transcriptLift = lift),
        setFiller: (kind, on) => _toggle((s) => switch (kind) {
              TranscriptFiller.hesitation => s.transcriptHesitations = on,
              TranscriptFiller.phrase => s.transcriptPhrases = on,
              TranscriptFiller.repeat => s.transcriptRepeats = on,
              TranscriptFiller.retake => s.transcriptRetakes = on,
              TranscriptFiller.none => on,
            }),
        setSkipTight: (on) => _toggle((s) => s.transcriptSkipTight = on),
        setLongerThanMs: (ms) => _toggle((s) => s.transcriptLongerThanMs = ms),
        setShortenToMs: (ms) => _toggle((s) => s.transcriptShortenToMs = ms),
        setSpeaker: (i) {
          _find.speaker = i;
          _find.refresh(_model, _s);
        },
      );

  void _toggle(void Function(InterfaceSettings s) flip) {
    flip(_s);
    _ui.workspace.settingsChanged();
  }

  List<(int, int)> get _actedOn => _find.currentRange == null
      ? const []
      : [_find.currentRange!];

  void _deleteResults({required bool current}) {
    final comp = _comp;
    if (comp == null || _source) return;
    // Removed words, and words said wholly under another, have no frames to
    // take out: a result of them alone is stepped over.
    final found = current ? _actedOn : _find.results;
    final ranges = [
      for (final r in found)
        if (_model.tokens.sublist(r.$1, r.$2).any((t) => t.frames > 0)) r,
    ];
    if (ranges.isEmpty) {
      if (current && found.isNotEmpty) _stepFind(1);
      return;
    }
    final spans = [
      for (final s in _model.spansOf(ranges))
        BridgeFrameSpan(startFrame: s.start, endFrame: s.end),
    ];
    final name = switch (_find.mode) {
      TranscriptFindMode.fillers => 'Delete filler words',
      _ => _ripple ? 'Delete words' : 'Lift words',
    };
    _caretFrame = spans.first.startFrame;
    if (!_edit(name,
        () => comp.cutTranscriptRemove(spans: spans, ripple: _ripple))) {
      _caretFrame = null;
    }
  }

  void _shortenPauses({required bool current}) {
    final comp = _comp;
    if (comp == null || _source) return;
    // The pauses the bar lists, so what is cut is what was shown.
    final only = [
      for (final s in _model.spansOf(current ? _actedOn : _find.results))
        BridgeFrameSpan(startFrame: s.start, endFrame: s.end),
    ];
    if (only.isEmpty) return;
    _edit(
        'Shorten pauses',
        () => comp.cutShortenPauses(
              look: _look,
              longerThanMs: _s.transcriptLongerThanMs,
              keepMs: _s.transcriptShortenToMs,
              only: only,
              ripple: _ripple,
            ));
  }

  /// The found words retyped as the replacement, one undo step.
  void _replace({required bool current}) {
    final b = _bridge;
    if (b == null) return;
    final texts = _find.replace.text
        .trim()
        .split(RegExp(r'\s+'))
        .where((w) => w.isNotEmpty)
        .toList();
    // Only whole words are retyped, each once however often it is used.
    final whole = _model.find(_find.query.text, whole: true).toSet();
    final ranges = (current ? _actedOn : _find.results).where(whole.contains);
    final edits = <BridgeWordsEdit>[];
    final seen = <(int, int)>{};
    // Matches can overlap and edits must not: one that starts inside the
    // last one taken is passed over.
    var taken = 0;
    for (final (a, c) in ranges) {
      if (a < taken) continue;
      int? item, from, to;
      for (var i = a; i < c; i++) {
        final t = _model.tokens[i];
        if (!t.isWord || t.item == null || t.index == null) continue;
        if (item != null && (t.item != item || t.index != to! + 1)) {
          item = null;
          break;
        }
        item = t.item;
        from ??= t.index;
        to = t.index;
      }
      if (item == null || !seen.add((item, from!))) continue;
      edits.add(BridgeWordsEdit(
          item: b.items[item], at: from, remove: to! - from + 1, texts: texts));
      taken = c;
    }
    if (edits.isEmpty) return;
    _write('Correct words', () => _app.project!.correctWords(edits: edits));
  }

  // --------------------------------------------------------------- the menus

  void _wordMenu(Offset global, int? token) {
    if (token != null && !_selection.contains(token)) {
      _selectAt(TranscriptSelection.of(token, token + 1, caret: token), token);
    }
    final some = !_selection.isEmpty;
    final run = _oneRun;
    showMenuAt<void>(
      context: context,
      position: global,
      width: 230,
      rows: (close) {
        Widget row(String key, String label, VoidCallback? go,
                {String? action}) =>
            MenuRow(
              key: ValueKey<String>(key),
              enabled: go != null,
              onPressed: () {
                close(null);
                go?.call();
              },
              child: MenuLabel(label, action: action, enabled: go != null),
            );
        return [
          if (!_source) ...[
            row('transcript-menu-cut', l10n.menuCut,
                some ? () => _cutClaim() : null,
                action: 'edit.cut'),
            row('transcript-menu-copy', l10n.menuCopy, some ? _copy : null,
                action: 'edit.copy'),
            row('transcript-menu-paste', l10n.menuPaste, _paste,
                action: 'edit.paste'),
            row('transcript-menu-delete', l10n.delete,
                some ? () => _remove(ripple: true) : null,
                action: 'transcript.extract'),
            row('transcript-menu-lift', l10n.transcriptLift,
                some ? () => _remove(ripple: false, name: 'Lift words') : null,
                action: 'transcript.lift'),
            const MenuDivider(),
          ],
          row('transcript-menu-correct', l10n.transcriptCorrect,
              run != null ? _startCorrect : null,
              action: 'transcript.correct'),
          SubmenuRow(
            key: const ValueKey('transcript-menu-speaker'),
            closeParent: () => close(null),
            submenu: (dismiss) => FloatSurface(
              width: 190,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final (i, s) in _model.speakers.indexed)
                    MenuRow.option(
                      key: ValueKey<String>('transcript-menu-speaker-$i'),
                      onPressed: () {
                        dismiss();
                        _setSpeaker(s.id);
                      },
                      child: MenuLabel(s.name, ticked: _selectedSpeaker == i),
                    ),
                  if (_model.speakers.isNotEmpty) const MenuDivider(),
                  MenuRow(
                    key: const ValueKey('transcript-menu-speaker-new'),
                    onPressed: () {
                      dismiss();
                      _newSpeaker();
                    },
                    child: MenuLabel(l10n.transcriptNewSpeaker),
                  ),
                ],
              ),
            ),
            child: MenuLabel(l10n.transcriptSpeaker, enabled: some),
          ),
          if (!_source)
            row('transcript-menu-mark', l10n.transcriptMarkWorkArea,
                some ? _mark : null,
                action: 'transcript.mark'),
          row('transcript-menu-paragraph', l10n.transcriptSelectParagraph,
              _selectParagraph),
          row('transcript-menu-copy-text', l10n.transcriptCopyAsText,
              some ? _copyText : null),
          if (_hasRemoved)
            row('transcript-menu-restore', l10n.transcriptRestore, _restore,
                action: 'transcript.restore'),
          if (_source) ...[
            const MenuDivider(),
            row('transcript-menu-insert', l10n.sourceInsert,
                some ? () => _placeWords(insert: true) : null,
                action: 'transcript.insert'),
            row('transcript-menu-overwrite', l10n.sourceOverwrite,
                some ? () => _placeWords(insert: false) : null,
                action: 'transcript.overwrite'),
            row('transcript-menu-append', l10n.transcriptAppend,
                some ? _appendWords : null),
            row('transcript-menu-new-comp', l10n.transcriptNewCompositionFrom,
                some ? _newCompositionFrom : null),
          ],
        ];
      },
    );
  }

  /// The speaker every selected word has, or null.
  int? get _selectedSpeaker {
    int? who;
    for (final (a, b) in _selection.ranges) {
      for (var i = a; i < b; i++) {
        final s = _model.tokens[i].speaker;
        if (s == null) continue;
        if (who != null && who != s) return null;
        who = s;
      }
    }
    return who;
  }

  void _selectParagraph() {
    final layout = _layout;
    final caret = _selection.caret ?? _selection.bounds?.$1;
    if (layout == null || caret == null || _model.tokens.isEmpty) return;
    final p = layout.paragraphOf(caret.clamp(0, _model.tokens.length - 1));
    if (p == null) return;
    _selectAt(TranscriptSelection.of(p.from, p.to), p.from);
  }

  Future<void> _copyText() => _textToClipboard(_spans());

  Future<void> _copyAllText() => _textToClipboard(_model.tokens.isEmpty
      ? const []
      : [
          BridgeFrameSpan(
              startFrame: _model.tokens.first.startFrame,
              endFrame: _model.tokens.last.endFrame),
        ]);

  /// The words in [spans], as the engine writes them, on the system clipboard.
  Future<void> _textToClipboard(List<BridgeFrameSpan> spans) async {
    if (spans.isEmpty) return;
    String text;
    try {
      text = _source
          ? await _footage!.sourceTranscriptText(look: _look, spans: spans)
          : await _comp!.cutTranscriptText(look: _look, spans: spans);
    } catch (_) {
      return;
    }
    await Clipboard.setData(ClipboardData(text: text));
  }

  void _setTextSize(double size) =>
      _toggle((s) => s.transcriptTextSize = size.clamp(10, 22).roundToDouble());

  void _moreMenu(Offset global, {bool folded = false}) {
    // The pause length as it is dragged, set when the drag lets go.
    num? pauseLive;
    showMenuAt<void>(
      context: context,
      position: global,
      width: 240,
      rows: (close) {
        Widget row(String key, String label, VoidCallback go,
                {bool? ticked, String? action}) =>
            MenuRow(
              key: ValueKey<String>(key),
              onPressed: () {
                close(null);
                go();
              },
              child: MenuLabel(label, ticked: ticked, action: action),
            );
        return [
          // A strip too tight for its buttons keeps them here instead.
          if (folded) ...[
            row('transcript-more-fillers',
                '${l10n.transcriptFillers} $_fillerCount',
                () => _openFind(TranscriptFindMode.fillers),
                ticked: _find.mode == TranscriptFindMode.fillers,
                action: 'transcript.fillers'),
            row('transcript-more-pauses',
                '${l10n.transcriptPauses} $_pauseCount',
                () => _openFind(TranscriptFindMode.pauses),
                ticked: _find.mode == TranscriptFindMode.pauses,
                action: 'transcript.pauses'),
            row('transcript-more-follow', l10n.transcriptFollow, _toggleFollow,
                ticked: _s.transcriptFollow),
            const MenuDivider(),
          ],
          row('transcript-more-removed', l10n.transcriptShowRemoved,
              () => _toggle((s) => s.transcriptShowRemoved = !s.transcriptShowRemoved),
              ticked: _s.transcriptShowRemoved),
          row('transcript-more-show-pauses', l10n.transcriptShowPauses,
              () => _toggle((s) => s.transcriptShowPauses = !s.transcriptShowPauses),
              ticked: _s.transcriptShowPauses),
          Padding(
            padding: const EdgeInsets.fromLTRB(10, 2, 10, 2),
            child: Row(children: [
              Expanded(child: Text(l10n.transcriptPauseLength)),
              SizedBox(
                width: 64,
                child: StatefulBuilder(
                  builder: (context, set) => DragValueField(
                    key: const ValueKey('transcript-more-pause-length'),
                    value: pauseLive ?? _s.transcriptPauseMs / 1000,
                    min: 0.1,
                    max: 3,
                    speed: 0.01,
                    decimals: 1,
                    suffix: ' s',
                    onChangeLive: (v) => set(() => pauseLive = v),
                    onChanged: (v) {
                      pauseLive = null;
                      _toggle((s) => s.transcriptPauseMs = (v * 1000).round());
                    },
                  ),
                ),
              ),
            ]),
          ),
          SubmenuRow(
            key: const ValueKey('transcript-more-size'),
            closeParent: () => close(null),
            submenu: (dismiss) => FloatSurface(
              width: 160,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  for (final (key, label, size) in [
                    ('smaller', l10n.transcriptTextSmaller,
                        _s.transcriptTextSize - 1),
                    ('larger', l10n.transcriptTextLarger,
                        _s.transcriptTextSize + 1),
                    ('reset', l10n.reset, 13.0),
                  ])
                    MenuRow(
                      key: ValueKey<String>('transcript-more-size-$key'),
                      onPressed: () {
                        dismiss();
                        _setTextSize(size);
                      },
                      child: MenuLabel(label),
                    ),
                ],
              ),
            ),
            child: MenuLabel(l10n.transcriptTextSize),
          ),
          const MenuDivider(),
          row('transcript-more-speakers', l10n.transcriptSpeakersMenu,
              () => showTranscriptSpeakers(context,
                  speakers: _model.speakers,
                  rename: _renameSpeaker,
                  find: _findSpeaker)),
          row('transcript-more-transcribe', l10n.transcriptTranscribeMissing,
              _transcribe),
          row('transcript-more-again', l10n.transcriptTranscribeAgain,
              () => _transcribe(again: true)),
          const MenuDivider(),
          row('transcript-more-assemble', l10n.transcriptAssembleMenu,
              _assemble),
          row('transcript-more-export', l10n.transcriptExportMenu, _export),
          row('transcript-more-copy-all', l10n.transcriptCopyAllText,
              _copyAllText),
        ];
      },
    );
  }

  /// The speakers review, on speaker [i].
  void _findSpeaker(int i) {
    _find.speaker = i;
    _find.show(TranscriptFindMode.speakers);
    _find.refresh(_model, _s);
  }

  // ------------------------------------------------------------- the windows

  Future<void> _export() async {
    final comp = _comp;
    final footage = _footage;
    if (_source ? footage == null : comp == null) return;
    final work = _source ? null : comp!.getWorkArea();
    final choice = await askTranscriptExport(context,
        hasSelection: !_selection.isEmpty, hasWorkArea: work != null);
    if (choice == null || !mounted) return;
    final tokens = _model.tokens;
    var start = 0;
    var end = tokens.isEmpty ? 0 : tokens.last.endFrame;
    switch (choice.range) {
      case TranscriptRange.whole:
        if (!_source) end = math.max(end, comp!.durationFrames());
      case TranscriptRange.selection:
        final spans = _model.spansOf(_selection.ranges);
        if (spans.isNotEmpty) {
          start = spans.map((s) => s.start).reduce(math.min);
          end = spans.map((s) => s.end).reduce(math.max);
        }
      case TranscriptRange.workArea:
        if (work != null) {
          start = comp!.frameAtTime(time: work.inPoint);
          end = comp.frameAtTime(time: work.outPoint);
        }
    }
    final name = _source
        ? ItemReference.footage(footage!).name()
        : comp!.getSettings().name;
    final ext = transcriptExtension(choice.format);
    final path = await pickExportSaveLocation(name,
        extension: ext, label: ext.toUpperCase());
    if (path == null || !mounted) return;
    final how = BridgeTranscriptExport(
      format: choice.format,
      speakers: choice.speakers,
      timecodes: choice.timecodes,
      fillers: choice.fillers,
      startFrame: start,
      endFrame: end,
      charsPerLine: choice.charsPerLine,
      lines: choice.lines,
    );
    try {
      if (_source) {
        await footage!.sourceTranscriptExport(path: path, look: _look, how: how);
      } else {
        await comp!.cutTranscriptExport(path: path, look: _look, how: how);
      }
    } catch (e) {
      if (mounted) _app.postNotice(l10n.exportFailed(e.toString()), error: true);
    }
  }

  Future<void> _assemble() async {
    final project = _app.project;
    if (project == null) return;
    final picked = _ui.selectedProjectItem.value;
    final pickedFootage = picked is ItemReference_Footage ? picked.field0 : null;
    final choice = await askTranscriptAssemble(
      context,
      openScript: () async {
        final path = await pickTranscriptScript();
        if (path == null) return null;
        try {
          final stem = path.split(RegExp(r'[\\/]')).last;
          final dot = stem.lastIndexOf('.');
          return (dot > 0 ? stem.substring(0, dot) : stem,
              await readScriptFile(path: path));
        } catch (_) {
          return null;
        }
      },
      hasSelectedItems: pickedFootage != null,
      hasComposition: _comp != null,
    );
    if (choice == null || !mounted) return;
    final footage = choice.allFootage || pickedFootage == null
        ? [
            for (final item in project.getItems())
              if (item is ItemReference_Footage) item.field0,
          ]
        : [pickedFootage];
    final into = choice.intoNew ? null : _comp;
    BridgeAssembled made;
    try {
      // The card holds every key and click while the lines are found, so
      // nothing changes before they are laid down.
      final finding =
          project.findScript(script: choice.script, footage: footage);
      await showBusyWhile(_app.busy, l10n.transcriptAssembleTitle, finding);
      final found = await finding;
      if (!mounted) return;
      made = project.assembleFromScript(
        found: found,
        into: into,
        atFrame: _ui.playheadFrame.value,
        name: choice.name,
      );
    } catch (_) {
      return;
    }
    _ui.reportCut(made.result);
    if (made.result != BridgeCutResult.done) return;
    _afterWrite();
    if (made.comp case final comp?) _ui.setSelectedComp(comp);
    if (made.missing.isNotEmpty) await showLinesNotFound(context, made.missing);
  }

  // -------------------------------------------------------------- the pointer

  /// The page's coordinates for a pointer position in the list.
  Offset _pageOffset(Offset local) =>
      Offset(local.dx - _pagePad, local.dy + _scroll.offset - _pagePad);

  ({int token, bool after})? _hitAt(Offset local) {
    final layout = _layout;
    if (layout == null) return null;
    final at = _pageOffset(local);
    // Right of the text is the scrollbar's.
    if (at.dx > layout.width) return null;
    final b = layout.blockAt(at.dy);
    if (b == null) return null;
    final block = layout.blocks[b];
    if (block is! TranscriptParagraph) {
      return (token: block.from, after: false);
    }
    return block.hit(Offset(at.dx, at.dy - layout.topOf(b)));
  }

  void _onPointerDown(PointerDownEvent e) {
    final layout = _layout;
    if (layout != null && _pageOffset(e.localPosition).dx > layout.width) {
      return;
    }
    if (_correct != null) {
      // A press in the correction field is the field's own. Its box is
      // only there to ask while the field is up.
      final at = _correctFocus.context;
      final field = at != null && at.mounted ? at.findRenderObject() : null;
      if (field is RenderBox &&
          field.hasSize &&
          (Offset.zero & field.size)
              .inflate(8)
              .contains(field.globalToLocal(e.position))) {
        return;
      }
      _endCorrect(commit: true);
    }
    final hit = _hitAt(e.localPosition);
    if (e.buttons == kSecondaryMouseButton) {
      _wordMenu(e.position, hit?.token);
      return;
    }
    if (e.buttons != kPrimaryMouseButton) return;
    final keys = HardwareKeyboard.instance;
    final shift = keys.isShiftPressed;
    final ctrl = keys.isControlPressed || keys.isMetaPressed;
    if (hit == null) {
      if (!shift && !ctrl) {
        _selectAt(TranscriptSelection(caret: _selection.caret), null);
      }
      return;
    }
    final token = hit.token;
    // Moves inside the pressed token leave what the press picked alone.
    _lastHit = token;
    if (shift) {
      final from = _anchor ?? _selection.caret ?? token;
      final end = token < from ? token : token + 1;
      _anchor = from;
      _select(TranscriptSelection.of(from, end, caret: end), seek: true);
      _selecting = true;
      _before = TranscriptSelection.none;
      return;
    }
    final now = e.timeStamp;
    _taps = token == _tapToken && now - _tapAt < const Duration(milliseconds: 400)
        ? _taps + 1
        : 1;
    _tapToken = token;
    _tapAt = now;
    // A first press on picked words may be the start of dragging them.
    if (!ctrl && _taps == 1 && _selection.contains(token)) {
      _pressAt = e.position;
      return;
    }
    _anchor = token;
    _before = ctrl ? _selection : TranscriptSelection.none;
    _selecting = true;
    if (_taps >= 3) {
      final p = _layout?.paragraphOf(token);
      if (p != null) {
        // The caret ends the paragraph, and the playhead goes to its start.
        _select(_before.adding((p.from, p.to), caret: p.to));
        final first = _model.tokens[p.from];
        _seekTo(first.isWord ? first.heardStartFrame : first.startFrame);
      }
    } else if (_taps == 2 || ctrl) {
      _select(_before.adding((token, token + 1), caret: token), seek: true);
    } else {
      _select(TranscriptSelection(caret: token), seek: true);
    }
  }

  void _onPointerMove(PointerMoveEvent e) {
    _lastMove = e;
    if (_dragging || _selecting) _edgeScrollFor(e.localPosition.dy);
    if (_pressAt case final pressed?) {
      if (!_dragging) {
        if ((e.position - pressed).distance < 4) return;
        _dragging = true;
        _drag.begin(() {
          _drop = null;
          _dragging = false;
          _pressAt = null;
          _stopEdgeScroll();
          _touch();
        });
      }
      if (!_drag.running) return;
      final hit = _hitAt(e.localPosition);
      final layout = _layout;
      int? drop;
      if (hit != null) {
        drop = hit.after ? hit.token + 1 : hit.token;
      } else if (layout != null) {
        final at = _pageOffset(e.localPosition);
        drop = layout.nearest(at.dx, at.dy);
      }
      // Inside the picked words, or just after the only run of them, they
      // would land where they are.
      final d = drop;
      final ranges = _selection.ranges;
      if (d != null &&
          (_selection.contains(d) ||
              (ranges.length == 1 && d == ranges.single.$2))) {
        drop = null;
      }
      if (drop != _drop) {
        _drop = drop;
        _touch();
      }
      return;
    }
    if (!_selecting || _anchor == null) return;
    final layout = _layout;
    if (layout == null) return;
    final at = _pageOffset(e.localPosition);
    final hit = _hitAt(e.localPosition)?.token ?? layout.nearest(at.dx, at.dy);
    if (hit == _lastHit) return;
    _lastHit = hit;
    final a = _anchor!;
    final range = hit >= a ? (a, hit + 1) : (hit, a + 1);
    _select(_before.adding(range, caret: hit >= a ? hit + 1 : hit));
  }

  /// Near the page's top or bottom edge a drag scrolls it, faster the
  /// closer, so words can go further than one screen.
  void _edgeScrollFor(double y) {
    if (!_scroll.hasClients) return;
    const edge = 28.0;
    final view = _scroll.position.viewportDimension;
    _edgeSpeed = y < edge
        ? -(edge - y) / edge * 14
        : y > view - edge
            ? (y - (view - edge)) / edge * 14
            : 0;
    if (_edgeSpeed == 0) {
      _stopEdgeScroll();
      return;
    }
    _edgeScroll ??= Timer.periodic(const Duration(milliseconds: 16), (_) {
      if (!mounted || !_scroll.hasClients) return _stopEdgeScroll();
      final to = (_scroll.offset + _edgeSpeed)
          .clamp(0.0, _scroll.position.maxScrollExtent);
      if (to == _scroll.offset) return;
      _scroll.jumpTo(to);
      // The pointer has not moved, but the words under it have.
      if (_lastMove case final last?) _onPointerMove(last);
    });
  }

  void _stopEdgeScroll() {
    _edgeScroll?.cancel();
    _edgeScroll = null;
    _edgeSpeed = 0;
  }

  void _onPointerUp(PointerUpEvent e) {
    _stopEdgeScroll();
    if (_pressAt != null) {
      final drop = _drop;
      final live = _drag.end();
      if (_dragging && live && drop != null) {
        _moveTo(drop, copy: HardwareKeyboard.instance.isAltPressed);
      } else if (!_dragging) {
        // A click on picked words collapses the pick to a caret there.
        final hit = _hitAt(e.localPosition);
        if (hit != null) {
          _selectAt(TranscriptSelection(caret: hit.token), hit.token,
              seek: true);
        }
      }
      _pressAt = null;
      _dragging = false;
      _drop = null;
      _touch();
    }
    _selecting = false;
  }

  void _onPointerSignal(PointerSignalEvent e) {
    if (e is! PointerScrollEvent) return;
    final keys = HardwareKeyboard.instance;
    if (!keys.isControlPressed && !keys.isMetaPressed) return;
    GestureBinding.instance.pointerSignalResolver.register(e, (e) {
      final by = (e as PointerScrollEvent).scrollDelta.dy < 0 ? 1 : -1;
      _setTextSize(_s.transcriptTextSize + by);
    });
  }

  // ------------------------------------------------------------------ the page

  TranscriptLayout _layoutFor(LumitTheme t, double width) {
    final size = _s.transcriptTextSize;
    final pauseLabels = _find.mode == TranscriptFindMode.pauses;
    final have = _layout;
    if (have != null &&
        have.width == width &&
        _layoutTheme == t &&
        _layoutSize == size &&
        _layoutPauseLabels == pauseLabels) {
      return have;
    }
    have?.dispose();
    final word = t.bodyPrimary
        .copyWith(fontSize: size, height: TranscriptStyles.lineHeight);
    final styles = TranscriptStyles(
      word: word,
      filler: word.copyWith(
          color: t.textMuted,
          decoration: TextDecoration.underline,
          decorationStyle: TextDecorationStyle.dashed,
          decorationColor: t.textMuted),
      removed: word.copyWith(
          color: t.textDisabled,
          decoration: TextDecoration.lineThrough,
          decorationColor: t.textDisabled),
      uncertain: word.copyWith(
          decoration: TextDecoration.underline,
          decorationStyle: TextDecorationStyle.dotted,
          decorationColor: t.textMuted),
      pauseLabel: t.mono.copyWith(fontSize: 9, color: t.textSecondary),
      size: size,
    );
    _layoutTheme = t;
    _layoutSize = size;
    _layoutPauseLabels = pauseLabels;
    return _layout = TranscriptLayout.lay(
      model: _model,
      styles: styles,
      width: width,
      pauseLabels: pauseLabels,
      headingHeight: _headingHeight,
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final hasSubject = _source ? _footage != null : _comp != null;
    return Container(
      color: t.surface0,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _strip(t),
          if (_find.isOpen)
            Entrance(
              key: ValueKey<String>('transcript-find-${_find.mode!.name}'),
              spec: ThemeScope.of(context).motion.reveal,
              rise: -ThemeScope.of(context).motion.revealRise,
              alignment: Alignment.topCenter,
              child: TranscriptFindBar(
                find: _find,
                model: _model,
                settings: _s,
                actions: _findActions,
              ),
            ),
          ValueListenableBuilder<BridgeTranscribeEvent?>(
            valueListenable: _job,
            builder: (context, event, _) {
              final view = _jobView(event);
              return view == null
                  ? const SizedBox.shrink()
                  : _progressRow(t, view);
            },
          ),
          if (_untranscribedNames.isNotEmpty &&
              !_nothingRead &&
              !setEquals(_hiddenUntranscribed, _untranscribedIds))
            _untranscribedRow(t),
          Expanded(child: hasSubject ? _body(t) : _placeholder(t)),
        ],
      ),
    );
  }

  Widget _placeholder(LumitTheme t, {String? title, Widget? action}) =>
      PlaceholderPanel(
        key: const ValueKey('transcript-empty'),
        icon: LumitIcon.text,
        title: title ?? l10n.timelineEmpty,
        hint: '',
        action: action,
      );

  /// No transcript has been read: there are no tokens, or only the breaks
  /// of footage nobody has transcribed.
  bool get _nothingRead =>
      _model.tokens.isEmpty || (_bridge?.items.isEmpty ?? false);

  Widget _body(LumitTheme t) {
    final b = _bridge;
    if (_nothingRead) {
      if (_reading && b == null) return const SizedBox.shrink();
      if (_failed) {
        return _placeholder(t,
            title: l10n.transcriptFailed,
            action: HouseButton(
              key: const ValueKey('transcript-try-again'),
              small: true,
              onPressed: _transcribe,
              child: Text(l10n.transcriptTryAgain),
            ));
      }
      if (b == null || (b.items.isEmpty && b.untranscribed.isEmpty)) {
        return _placeholder(t, title: l10n.mixerNoSound);
      }
      if (b.untranscribed.isNotEmpty) {
        return _placeholder(t,
            title: l10n.transcriptNotTranscribed, action: _transcribeRow(t));
      }
      if (b.items.isNotEmpty) {
        return Center(
          child: Text(l10n.transcriptNoSpeech,
              key: const ValueKey('transcript-no-speech'),
              style: t.body.copyWith(color: t.textMuted)),
        );
      }
    }
    return _page(t);
  }

  /// The Transcribe button with the language and speaker count beside it.
  Widget _transcribeRow(LumitTheme t) => Wrap(
        alignment: WrapAlignment.center,
        crossAxisAlignment: WrapCrossAlignment.center,
        spacing: 8,
        runSpacing: 6,
        children: [
          HouseButton(
            key: const ValueKey('transcript-transcribe'),
            small: true,
            primary: true,
            onPressed: _transcribe,
            child: Text(l10n.transcriptTranscribe),
          ),
          BareDropdown<String>(
            key: const ValueKey('transcript-language'),
            value: _language,
            options: ['', for (final (code, _) in _languages) code],
            label: (code) => code.isEmpty
                ? l10n.transcriptDetectLanguage
                : _languages
                    .firstWhere((l) => l.$1 == code, orElse: () => (code, code))
                    .$2,
            dense: true,
            onChanged: (code) => setState(() => _language = code),
          ),
          BareDropdown<int>(
            key: const ValueKey('transcript-speaker-count'),
            value: _speakerCount,
            options: const [0, 1, 2, 3, 4],
            label: (n) => switch (n) {
              1 => l10n.transcriptSpeakersOne,
              2 => l10n.transcriptSpeakersTwo,
              3 => l10n.transcriptSpeakersThree,
              4 => l10n.transcriptSpeakersFour,
              _ => l10n.transcriptDetectSpeakers,
            },
            dense: true,
            onChanged: (n) => setState(() => _speakerCount = n),
          ),
        ],
      );

  /// A thin bar under the strip while a job runs: how far, the stage's word
  /// and the percentage, and Cancel.
  Widget _progressRow(
      LumitTheme t, ({double fraction, String stage, bool failed}) view) {
    return Container(
      key: const ValueKey('transcript-progress'),
      height: t.density.secondaryRow,
      padding: const EdgeInsets.symmetric(horizontal: _pagePad),
      decoration:
          BoxDecoration(border: Border(bottom: BorderSide(color: t.hairline))),
      child: Row(children: [
        Expanded(child: HouseProgressBar(fraction: view.fraction, height: 2)),
        const SizedBox(width: 8),
        Text(
          '${view.stage} ${(view.fraction * 100).round()}%',
          style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
        ),
        const SizedBox(width: 4),
        HouseButton(
          key: const ValueKey('transcript-cancel'),
          small: true,
          frameless: true,
          onPressed: _cancelTranscription,
          child: Text(l10n.cancel),
        ),
      ]),
    );
  }

  void _cancelTranscription() => _app.transcription.cancel();

  /// One file by name, several by count, with Transcribe and a close mark
  /// that hides the row until the set of files changes.
  Widget _untranscribedRow(LumitTheme t) => Container(
        key: const ValueKey('transcript-untranscribed'),
        height: t.density.secondaryRow,
        padding: const EdgeInsets.only(left: _pagePad, right: 4),
        decoration: BoxDecoration(
            border: Border(bottom: BorderSide(color: t.hairline))),
        child: Row(children: [
          Expanded(
            child: Text(
                _untranscribedNames.length == 1
                    ? l10n.transcriptNotTranscribedNamed(_untranscribedNames.single)
                    : l10n.transcriptNotTranscribedCount(
                        _untranscribedNames.length),
                style: t.body,
                overflow: TextOverflow.ellipsis),
          ),
          HouseButton(
            key: const ValueKey('transcript-untranscribed-go'),
            small: true,
            frameless: true,
            onPressed: _transcribe,
            child: Text(l10n.transcriptTranscribe),
          ),
          const SizedBox(width: 2),
          LumitTooltip(
            message: l10n.close,
            child: HouseButton(
              key: const ValueKey('transcript-untranscribed-close'),
              small: true,
              frameless: true,
              padding: const EdgeInsets.symmetric(horizontal: 4),
              onPressed: () =>
                  setState(() => _hiddenUntranscribed = _untranscribedIds),
              child: glyph.LumitIcon(LumitIcons.close,
                  size: 12, colour: t.textSecondary),
            ),
          ),
        ]),
      );

  Widget _page(LumitTheme t) => LayoutBuilder(builder: (context, box) {
        final width = math.min(_pageWidth, box.maxWidth - 2 * _pagePad);
        if (width <= 0) return const SizedBox.shrink();
        final layout = _layoutFor(t, width);
        final colours = TranscriptPaint(
          accent: t.accent,
          selectionFill: t.selectionFill,
          marker: t.marker,
          surface3: t.surface3,
          hairlineStrong: t.hairlineStrong,
          radius: t.tokens.controlRadius,
        );
        return Listener(
          onPointerDown: _onPointerDown,
          onPointerMove: _onPointerMove,
          onPointerUp: _onPointerUp,
          onPointerCancel: (_) {
            _stopEdgeScroll();
            _selecting = false;
            _pressAt = null;
            _dragging = false;
            _drop = null;
            _drag.end();
            _touch();
          },
          behavior: HitTestBehavior.opaque,
          child: MouseRegion(
            cursor: SystemMouseCursors.text,
            child: ListenableBuilder(
              listenable: _repaint,
              builder: (context, _) {
                final marks = TranscriptMarks(
                  selection: _selection,
                  current: _current,
                  found: _find.isOpen ? _find.foundTokens : const {},
                  foundCurrent: _find.isOpen ? _find.foundCurrentToken : null,
                  pickedClips: _source
                      ? const {}
                      : {
                          for (final c in _ui.cutClips.value)
                            c.clip.toString()
                        },
                  drop: _dragging ? _drop : null,
                  caretOn: _caretOn,
                );
                return ListView.builder(
                  key: const ValueKey('transcript-page'),
                  controller: _scroll,
                  padding: const EdgeInsets.symmetric(vertical: _pagePad),
                  itemCount: layout.blocks.length,
                  itemExtentBuilder: (i, _) =>
                      i < layout.blocks.length ? layout.heightOf(i) : null,
                  // Inside the list, so Ctrl and the wheel reach the text size
                  // before the list scrolls.
                  itemBuilder: (context, i) => Listener(
                    onPointerSignal: _onPointerSignal,
                    // The whole row, not only where its block paints.
                    behavior: HitTestBehavior.opaque,
                    child: Padding(
                      padding: const EdgeInsets.only(left: _pagePad),
                      child: Align(
                        alignment: Alignment.topLeft,
                        child: SizedBox(
                          width: width,
                          height: layout.heightOf(i),
                          child: _block(t, layout, i, marks, colours),
                        ),
                      ),
                    ),
                  ),
                );
              },
            ),
          ),
        );
      });

  Widget _block(LumitTheme t, TranscriptLayout layout, int i,
      TranscriptMarks marks, TranscriptPaint colours) {
    final block = layout.blocks[i];
    if (block is TranscriptBreakBlock) {
      final seconds = _model.secondsOf(_model.tokens[block.from].silence);
      final picked = marks.selection.contains(block.from);
      return Container(
        key: ValueKey<String>('transcript-break-${block.from}'),
        color: picked ? t.selectionFill : null,
        child: Row(children: [
          Expanded(child: Container(height: 1, color: t.hairline)),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8),
            child: Text(TranscriptLayout.pauseText(seconds),
                style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
          ),
          Expanded(child: Container(height: 1, color: t.hairline)),
        ]),
      );
    }
    final p = block as TranscriptParagraph;
    final boxes = p.painter.inlinePlaceholderBoxes ?? const [];
    return Stack(
      key: ValueKey<String>('transcript-paragraph-${p.from}'),
      clipBehavior: Clip.none,
      children: [
        Positioned.fill(
          child: CustomPaint(
            painter: TranscriptParagraphPainter(
              paragraph: p,
              model: _model,
              marks: marks,
              colours: colours,
              pauseLabel: layout.styles.pauseLabel,
              pauseLabels: layout.pauseLabels,
            ),
          ),
        ),
        if (p.heading) _heading(t, p),
        // Each pause chip says how long it is on hover.
        for (var k = 0; k < p.placeholderOf.length; k++)
          if (p.placeholderOf[k] >= 0 && p.placeholderOf[k] < boxes.length)
            Positioned.fromRect(
              rect: boxes[p.placeholderOf[k]]
                  .toRect()
                  .shift(Offset(0, p.headingHeight)),
              child: LumitTooltip(
                message: TranscriptLayout.pauseText(
                    _model.secondsOf(_model.tokens[p.from + k].silence)),
                child: const SizedBox.expand(),
              ),
            ),
        if (_correct case final field?)
          if (_oneRun case final run? when run.first >= p.from && run.first < p.to)
            _correctField(t, p, run.first, field),
      ],
    );
  }

  /// The speaker's dot and name, when the paragraph names one, and the
  /// timecode of its first word.
  Widget _heading(LumitTheme t, TranscriptParagraph p) {
    final who = p.speaker;
    final name = who != null && who < _model.speakers.length
        ? _model.speakers[who].name
        : '';
    final first = _model.tokens[p.from];
    final start = _clockStart;
    return Positioned(
      left: 0,
      top: 0,
      right: 0,
      height: p.headingHeight,
      child: Row(children: [
        if (who != null) ...[
          Container(
            width: 7,
            height: 7,
            decoration: BoxDecoration(
                color: t.labelColour(who + 1), shape: BoxShape.circle),
          ),
          const SizedBox(width: 7),
          GestureDetector(
            onDoubleTap: () async {
              final speaker = _model.speakers[who];
              final renamed = await askProfileName(context,
                  title: l10n.rename, suggested: name, confirm: l10n.rename);
              if (renamed != null && renamed.trim().isNotEmpty) {
                _renameSpeaker(speaker, renamed.trim());
              }
            },
            child: Text(t.kickerCase(name), style: t.kicker),
          ),
          const SizedBox(width: 10),
        ],
        Text(
          timecodeOfRate(first.heardStartFrame + start.$1, _model.fpsNum,
              _model.fpsDen,
              dropFrame: start.$2),
          style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
        ),
      ]),
    );
  }

  /// The correction field over the first line of the words being retyped.
  Widget _correctField(LumitTheme t, TranscriptParagraph p, int first,
      TextEditingController field) {
    final rects = p.rectsOf(first);
    if (rects.isEmpty) return const SizedBox.shrink();
    final r = rects.first;
    return Positioned(
      left: r.left - 4,
      top: r.top - 2,
      width: math.max(160, p.painter.width - r.left + 4),
      child: HouseTextField(
        key: const ValueKey('transcript-correct'),
        controller: field,
        focusNode: _correctFocus,
        width: double.infinity,
        style: t.bodyPrimary.copyWith(fontSize: _s.transcriptTextSize),
        onSubmitted: (_) => _endCorrect(commit: true),
        onCancelled: () => _endCorrect(commit: false),
      ),
    );
  }

  // ---------------------------------------------------------------- the strip

  int get _fillerCount => _model
      .fillers(
        hesitations: _s.transcriptHesitations,
        phrases: _s.transcriptPhrases,
        repeats: _s.transcriptRepeats,
        retakes: _s.transcriptRetakes,
        skipTight: _s.transcriptSkipTight,
      )
      .length;

  int get _pauseCount => _model.pauses(_s.transcriptLongerThanMs).length;

  void _toggleFollow() {
    _followPaused = false;
    _toggle((s) => s.transcriptFollow = !s.transcriptFollow);
    if (_s.transcriptFollow && _current != null) _follow(_current!);
  }

  /// The strip: the subject as two marks, the search well taking the room,
  /// the two reviews with their counts, Follow, and the menu mark. Below
  /// 360 the reviews and Follow move into the menu, so the strip never
  /// grows a second mark.
  Widget _strip(LumitTheme t) => LayoutBuilder(builder: (context, box) {
        final cell =
            t.density.timelineChromeControl ?? t.density.timelineChromeRow;
        final wide = box.maxWidth >= 360;
        Widget mark(String key, String glyphName, String tip,
                VoidCallback? press,
                {bool on = false, String? action}) =>
            timelineChromeControl(
              t,
              LumitTooltip(
                message: tip,
                action: action,
                child: SizedBox(
                  width: 26,
                  height: cell,
                  child: HouseButton(
                    key: ValueKey<String>(key),
                    frameless: !on,
                    active: on,
                    padding: EdgeInsets.zero,
                    onPressed: press,
                    child: Center(
                      child: glyph.LumitIcon(glyphName,
                          colour: press == null
                              ? t.textDisabled
                              : on
                                  ? t.textPrimary
                                  : t.textSecondary),
                    ),
                  ),
                ),
              ),
            );
        Widget text(String key, String label, VoidCallback? press,
            {bool on = false, String? action, String? tip}) {
          final button = HouseButton(
            key: ValueKey<String>(key),
            small: true,
            frameless: !on,
            active: on,
            padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
            onPressed: press,
            child: Text(t.kickerCase(label), style: on ? t.kickerOn : t.kicker),
          );
          return timelineChromeControl(
              t,
              tip == null
                  ? button
                  : LumitTooltip(message: tip, action: action, child: button));
        }

        return Container(
          height: t.density.timelineChromeRow,
          color: t.surface1,
          padding: const EdgeInsets.symmetric(horizontal: 8),
          child: Row(children: [
            mark('transcript-composition', LumitIcons.composition,
                l10n.transcriptComposition, () => _setSource(false),
                on: !_source),
            const SizedBox(width: 2),
            mark('transcript-source', LumitIcons.footage, l10n.transcriptSource,
                _sourceView == null ? null : () => _setSource(true),
                on: _source),
            const SizedBox(width: 8),
            Expanded(
              child: timelineChromeControl(
                t,
                HouseTextField(
                  key: const ValueKey('transcript-search'),
                  controller: _find.query,
                  focusNode: _find.queryFocus,
                  width: double.infinity,
                  hint: l10n.search,
                  leading: glyph.LumitIcon(LumitIcons.search,
                      size: 12, colour: t.textMuted),
                  onSubmitted: (_) => _stepFind(
                      HardwareKeyboard.instance.isShiftPressed ? -1 : 1),
                ),
              ),
            ),
            if (wide) ...[
              ListenableBuilder(
                listenable: _repaint,
                builder: (context, _) {
                  final ranges = _selection.ranges;
                  if (ranges.length < 2) return const SizedBox.shrink();
                  final length = _model
                      .spansOf(ranges)
                      .fold(0, (n, s) => n + s.end - s.start);
                  return Padding(
                    padding: const EdgeInsets.only(left: 8),
                    child: Text(
                      l10n.transcriptRangesSummary(ranges.length,
                          timecodeOfRate(length, _model.fpsNum, _model.fpsDen)),
                      key: const ValueKey('transcript-ranges'),
                      style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
                    ),
                  );
                },
              ),
              const SizedBox(width: 8),
              text('transcript-fillers',
                  '${l10n.transcriptFillers} $_fillerCount',
                  () => _openFind(TranscriptFindMode.fillers),
                  on: _find.mode == TranscriptFindMode.fillers,
                  action: 'transcript.fillers',
                  tip: l10n.transcriptFillers),
              const SizedBox(width: 2),
              text('transcript-pauses', '${l10n.transcriptPauses} $_pauseCount',
                  () => _openFind(TranscriptFindMode.pauses),
                  on: _find.mode == TranscriptFindMode.pauses,
                  action: 'transcript.pauses',
                  tip: l10n.transcriptPauses),
              const SizedBox(width: 6),
              mark('transcript-follow', LumitIcons.fitToView,
                  l10n.transcriptFollow, _toggleFollow,
                  on: _s.transcriptFollow),
            ],
            const SizedBox(width: 4),
            timelineChromeControl(
              t,
              LumitTooltip(
                message: l10n.transcriptMore,
                child: SizedBox(
                  width: 26,
                  height: cell,
                  child: Builder(
                    builder: (context) => HouseButton(
                      key: const ValueKey('transcript-more'),
                      frameless: true,
                      padding: EdgeInsets.zero,
                      onPressed: () {
                        final box = context.findRenderObject() as RenderBox?;
                        final at =
                            box?.localToGlobal(Offset(0, box.size.height)) ??
                                Offset.zero;
                        _moreMenu(at, folded: !wide);
                      },
                      child: Center(
                          child: glyph.LumitIcon(LumitIcons.more,
                              colour: t.textSecondary)),
                    ),
                  ),
                ),
              ),
            ),
          ]),
        );
      });
}
