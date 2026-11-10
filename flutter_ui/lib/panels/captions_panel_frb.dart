// The Captions panel: a caption layer's captions, a row each, in the order
// they show, to read, retype, retime, split, join, find and restyle.
//
// It shows the selected layer when that is a caption layer, and the top
// caption layer of the composition otherwise; a dropdown picks among several.
// The captions are read when the document moves, never while drawing, and the
// list is lazy, so a two-hour layer scrolls as freely as a short one.
// Which captions are picked is shared with the Timeline's captions row.

import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:uuid/uuid.dart' show UuidValue;
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/captions.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/src/rust/api/transcript.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../state/dock.dart' show Panel;
import '../state/timecode.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'caption_layers.dart';
import 'captions_rows_frb.dart';
import 'captions_states_frb.dart';
import 'captions_tools_frb.dart';
import 'expressions_panel_frb.dart' show panelBar;
import 'placeholder.dart';
import 'timeline_layer_menu_frb.dart' show showMenuEntriesAt;
import '../shell/menu_bar_frb.dart' show MenuEntry;

class CaptionsPanelFrb extends StatefulWidget {
  const CaptionsPanelFrb({super.key});

  @override
  State<CaptionsPanelFrb> createState() => _CaptionsPanelFrbState();
}

class _CaptionsPanelFrbState extends State<CaptionsPanelFrb> {
  late final LumitState _app = Provider.of<LumitState>(context, listen: false);
  late final LumitUiState _ui =
      Provider.of<LumitUiState>(context, listen: false);

  /// The composition's caption layers, and the one shown.
  List<BridgeLayerEntry> _layers = const [];
  BridgeLayerEntry? _entry;

  /// The one picked in the dropdown, which holds while it is there.
  UuidValue? _picked;

  /// What was read, and at which revision of the document.
  BridgeCaptions? _captions;
  List<CaptionRow> _rows = const [];
  UuidValue? _readLayer;
  int? _readRevision;

  /// The selected layer has sound to caption, for the empty state's button.
  bool _canCreate = false;

  /// The row under the playhead, or -1.
  int _current = -1;

  /// The row the last plain or Ctrl click landed on, for Shift to run from.
  int? _anchor;

  final ScrollController _scroll = ScrollController();
  DateTime _scrolledByHand = DateTime.fromMillisecondsSinceEpoch(0);
  bool _scrollingForPlayhead = false;

  final TextEditingController _find = TextEditingController();
  final FocusNode _findFocus = FocusNode();
  final TextEditingController _with = TextEditingController();
  bool _replacing = false;
  bool _matchCase = false;
  bool _wholeWord = false;

  /// The rows a find has found, and which of them was stepped to last.
  List<int> _found = const [];
  int _foundAt = -1;

  /// The caption being typed into, by its index, and what it said.
  int? _typing;
  String _typed = '';
  final TextEditingController _typeText = TextEditingController();
  final FocusNode _typeFocus = FocusNode();

  /// The time being changed: the caption's index, and whether its end.
  (int, bool)? _timing;
  final TextEditingController _timeText = TextEditingController();

  bool Function()? _priorDelete;

  /// A read put off while the document moves, and whether the next one is
  /// owed at once.
  Timer? _later;
  bool _urgent = false;

  @override
  void initState() {
    super.initState();
    _ui.model.addListener(_sync);
    _ui.selectedLayer.addListener(_sync);
    _ui.playheadFrame.addListener(_playheadMoved);
    _ui.captionSelection.addListener(_redraw);
    _ui.captionSelection.typeRequest.addListener(_typeAsked);
    _ui.panelSearchRequest.addListener(_searchAsked);
    _ui.selectAllRequest.addListener(_selectAllAsked);
    _find.addListener(_findChanged);
    _priorDelete = _ui.deleteClaim;
    _ui.deleteClaim = _deleteClaim;
    HardwareKeyboard.instance.addHandler(_onKey);
    _sync();
    // A caption asked for before the panel was up is opened as it comes up.
    if (_ui.captionSelection.typing != null) {
      WidgetsBinding.instance.addPostFrameCallback((_) => _typeAsked());
    }
  }

  @override
  void dispose() {
    _commitTyping();
    _later?.cancel();
    HardwareKeyboard.instance.removeHandler(_onKey);
    if (_ui.deleteClaim == _deleteClaim) _ui.deleteClaim = _priorDelete;
    _ui.model.removeListener(_sync);
    _ui.selectedLayer.removeListener(_sync);
    _ui.playheadFrame.removeListener(_playheadMoved);
    _ui.captionSelection.removeListener(_redraw);
    _ui.captionSelection.typeRequest.removeListener(_typeAsked);
    _ui.panelSearchRequest.removeListener(_searchAsked);
    _ui.selectAllRequest.removeListener(_selectAllAsked);
    _scroll.dispose();
    _find.dispose();
    _findFocus.dispose();
    _with.dispose();
    _typeText.dispose();
    _typeFocus.dispose();
    _timeText.dispose();
    super.dispose();
  }

  void _redraw() {
    if (mounted) setState(() {});
  }

  (int, int) get _fps => _ui.model.fpsExact;

  // --- Reading ----------------------------------------------------------------

  /// Work out which layer is shown, and read its captions again when it or the
  /// document has moved.
  void _sync() {
    if (!mounted) return;
    final comp = _ui.selectedComp;
    final layers =
        comp == null ? const <BridgeLayerEntry>[] : captionLayersIn(_ui);
    final entry = shownCaptionLayer(_ui, layers, picked: _picked);
    final id = entry?.layer.internallayerId;
    final revision = _ui.model.revision;
    var canCreate = false;
    if (entry == null) {
      try {
        canCreate = _ui.selectedLayer.value?.heardItems().isNotEmpty ?? false;
      } catch (_) {}
    }
    final other = id != _readLayer;
    var moved = other || revision != _readRevision;
    // A long layer is read again a moment after the document stops moving
    // under it, so dragging something else is not slowed by it. Its own
    // edits and a change of layer are read at once.
    if (moved && !other && !_urgent && _rows.length > 400) {
      _later?.cancel();
      _later = Timer(const Duration(milliseconds: 150), () {
        _urgent = true;
        _sync();
      });
      moved = false;
    }
    if (moved) {
      if (other) {
        _commitTyping();
        _typing = null;
        _timing = null;
        _anchor = null;
      }
      _later?.cancel();
      _urgent = false;
      _readLayer = id;
      _readRevision = revision;
      _read(entry);
    }
    if (!moved &&
        canCreate == _canCreate &&
        listEquals(_ids(layers), _ids(_layers))) {
      return;
    }
    setState(() {
      _layers = layers;
      _entry = entry;
      _canCreate = canCreate;
    });
  }

  List<UuidValue> _ids(List<BridgeLayerEntry> layers) =>
      [for (final e in layers) e.layer.internallayerId];

  void _read(BridgeLayerEntry? entry) {
    BridgeCaptions? captions;
    try {
      captions = entry?.layer.captions();
    } catch (_) {}
    final fps = _fps;
    _captions = captions;
    _rows = [
      for (final cue in captions?.cues ?? const <BridgeCue>[])
        if (cue.start != null && cue.end != null) CaptionRow.of(cue, fps),
    ];
    _current = _rowAt(_ui.playheadFrame.value);
    _refind();
  }

  /// The row showing at [frame], or -1.
  int _rowAt(int frame) {
    var low = 0, high = _rows.length - 1, at = -1;
    while (low <= high) {
      final mid = (low + high) >> 1;
      if (_rows[mid].start <= frame) {
        at = mid;
        low = mid + 1;
      } else {
        high = mid - 1;
      }
    }
    return at >= 0 && frame < _rows[at].end ? at : -1;
  }

  int _rowOf(int cue) => _rows.indexWhere((r) => r.index == cue);

  // --- The playhead ------------------------------------------------------------

  void _playheadMoved() {
    if (!mounted) return;
    final at = _rowAt(_ui.playheadFrame.value);
    if (at == _current) return;
    setState(() => _current = at);
    // Played through, the list keeps the caption being said in sight, unless
    // it was scrolled by hand a moment ago.
    final byHand =
        DateTime.now().difference(_scrolledByHand) < const Duration(seconds: 4);
    if (at >= 0 && _ui.playing.value && !byHand) _reveal(at, ease: true);
  }

  double _extent(int row) => _heights.of(_rows[row],
      wide: _wide,
      typing: _rows[row].index == _typing,
      timing: _rows[row].index == _timing?.$1);

  /// How the list was last laid out: the times side by side, and every
  /// row's height at the width its words have.
  bool _wide = false;
  final CaptionRowHeights _heights = CaptionRowHeights();

  /// Scroll row [row] into view, as little as it takes.
  void _reveal(int row, {bool ease = false}) {
    if (!_scroll.hasClients || row < 0 || row >= _rows.length) return;
    var top = 0.0;
    for (var i = 0; i < row; i++) {
      top += _extent(i);
    }
    final bottom = top + _extent(row);
    final position = _scroll.position;
    final view = position.viewportDimension;
    double? to;
    if (top < position.pixels) {
      to = top;
    } else if (bottom > position.pixels + view) {
      to = ease ? top - view * 0.25 : bottom - view;
    }
    if (to == null) return;
    to = to.clamp(0.0, position.maxScrollExtent);
    _scrollingForPlayhead = true;
    final reveal = ThemeScope.of(context).motion.reveal;
    if (ease && !reveal.isStill) {
      _scroll
          .animateTo(to, duration: reveal.duration, curve: reveal.curve)
          .whenComplete(() => _scrollingForPlayhead = false);
    } else {
      _scroll.jumpTo(to);
      _scrollingForPlayhead = false;
    }
  }

  // --- Picking ---------------------------------------------------------------

  Set<int> get _picks {
    final selection = _ui.captionSelection;
    return selection.layer == _readLayer ? selection.cues : const {};
  }

  void _choose(Set<int> cues) => _ui.captionSelection.choose(_readLayer, cues);

  /// A press on row [row]: pick it, or add it or run to it with Ctrl or
  /// Shift, and take the playhead to where it starts.
  void _press(int row) {
    final keys = HardwareKeyboard.instance;
    final cue = _rows[row].index;
    if (keys.isShiftPressed && _anchor != null) {
      final from = _anchor!.clamp(0, _rows.length - 1);
      final (a, b) = from <= row ? (from, row) : (row, from);
      _choose({for (var i = a; i <= b; i++) _rows[i].index});
    } else if (keys.isControlPressed || keys.isMetaPressed) {
      final picks = {..._picks};
      if (!picks.remove(cue)) picks.add(cue);
      _choose(picks);
      _anchor = row;
    } else {
      _choose({cue});
      _anchor = row;
    }
    _ui.scrubTo(_rows[row].start);
  }

  /// The arrows: the pick moves a row, or grows with Shift.
  bool _step(int by) {
    if (_rows.isEmpty) return false;
    final picked = _picks;
    var at = -1;
    for (var i = 0; i < _rows.length; i++) {
      if (picked.contains(_rows[i].index)) {
        at = i;
        if (by < 0) break;
      }
    }
    final next = at < 0
        ? (_current >= 0 ? _current : 0)
        : (at + by).clamp(0, _rows.length - 1);
    if (HardwareKeyboard.instance.isShiftPressed && _anchor != null) {
      final (a, b) = _anchor! <= next ? (_anchor!, next) : (next, _anchor!);
      _choose({for (var i = a; i <= b; i++) _rows[i].index});
    } else {
      _choose({_rows[next].index});
      _anchor = next;
    }
    _ui.scrubTo(_rows[next].start);
    _reveal(next);
    return true;
  }

  void _selectAllAsked() {
    if (!mounted || !_ui.selectAllRequestIsFor(Panel.captions)) return;
    _choose({for (final row in _rows) row.index});
  }

  // --- Editing ----------------------------------------------------------------

  /// An edit went in: say so, and read it back.
  void _edited() {
    _urgent = true;
    _app.notifyDocumentChanged();
    _ui.model.refresh();
  }

  /// Run [edit] on the shown layer, and say in the status line when the
  /// engine turns it down.
  bool _try(void Function() edit, {String? refused}) {
    try {
      edit();
      _edited();
      return true;
    } catch (_) {
      _app.postNotice(refused ?? l10n.captionsEditFailed, error: true);
      return false;
    }
  }

  void _type(int cue) {
    _commitTyping();
    final row = _rowOf(cue);
    if (row < 0) return;
    setState(() {
      _timing = null;
      _typing = cue;
      _typed = _rows[row].text;
      _typeText.value = TextEditingValue(
          text: _typed,
          selection: TextSelection(baseOffset: 0, extentOffset: _typed.length));
    });
    _choose({cue});
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _reveal(_rowOf(cue));
      _typeFocus.requestFocus();
    });
  }

  void _commitTyping() {
    final cue = _typing;
    final layer = _entry?.layer;
    if (cue == null) return;
    _typing = null;
    final text = _typeText.text.trim();
    if (layer == null || text == _typed.trim()) {
      if (mounted) setState(() {});
      return;
    }
    _try(() => layer.setCueText(cue: cue, text: text));
  }

  void _cancelTyping() => setState(() => _typing = null);

  void _typeAsked() {
    final selection = _ui.captionSelection;
    final cue = selection.typing;
    if (!mounted || cue == null) return;
    // The Timeline or the Viewer asked about a layer; show that one.
    if (selection.layer != null && selection.layer != _readLayer) {
      _picked = selection.layer;
      _sync();
    }
    if (selection.layer != _readLayer) return;
    selection.typing = null;
    _type(cue);
  }

  void _time(int cue, bool end) {
    _commitTyping();
    final row = _rowOf(cue);
    if (row < 0) return;
    final frame = end ? _rows[row].end : _rows[row].start;
    final (n, d) = _fps;
    final text = timecodeOfRate(frame + _ui.model.startFrame, n, d);
    setState(() {
      _timing = (cue, end);
      _timeText.value = TextEditingValue(
          text: text,
          selection: TextSelection(baseOffset: 0, extentOffset: text.length));
    });
    _choose({cue});
  }

  void _commitTime() {
    final timing = _timing;
    final comp = _ui.selectedComp;
    final layer = _entry?.layer;
    if (timing == null || comp == null || layer == null) return;
    setState(() => _timing = null);
    final (cue, end) = timing;
    final row = _rowOf(cue);
    if (row < 0) return;
    final (n, d) = _fps;
    final typed = framesOfTimecode(_timeText.text, n, d);
    if (typed == null) return;
    final frame = typed - _ui.model.startFrame;
    final was = _rows[row];
    if (frame == (end ? was.end : was.start)) return;
    final at = timeOfFrame(comp, frame);
    _try(() => layer.setCueSpan(
        cue: cue,
        start: end ? was.cue.start! : at,
        end: end ? at : was.cue.end!));
  }

  void _split() {
    final comp = _ui.selectedComp;
    final layer = _entry?.layer;
    if (comp == null || layer == null) return;
    final frame = _ui.playheadFrame.value;
    if (_try(() => layer.splitCueAt(at: timeOfFrame(comp, frame)),
        refused: l10n.captionsNothingToSplit)) {
      final at = _rowAt(frame);
      if (at >= 0) _choose({_rows[at].index});
    }
  }

  void _join() {
    final layer = _entry?.layer;
    if (layer == null) return;
    final picks = _picks.toList()..sort();
    if (picks.isEmpty && _current >= 0) picks.add(_rows[_current].index);
    if (picks.isEmpty) return;
    final first = picks.first;
    final count = picks.length == 1 ? 2 : picks.last - first + 1;
    if (_try(() => layer.mergeCues(first: first, count: count),
        refused: l10n.captionsNothingToJoin)) {
      _choose({first});
    }
  }

  void _add() {
    final comp = _ui.selectedComp;
    final layer = _entry?.layer;
    if (comp == null || layer == null) return;
    int? made;
    if (_try(
        () => made = layer.addCue(
            at: timeOfFrame(comp, _ui.playheadFrame.value), text: ''),
        refused: l10n.captionsCannotAddHere)) {
      final cue = made;
      if (cue != null) {
        WidgetsBinding.instance.addPostFrameCallback((_) => _type(cue));
      }
    }
  }

  void _remove(Iterable<int> cues) {
    final layer = _entry?.layer;
    if (layer == null || cues.isEmpty) return;
    if (_try(() => layer.removeCues(cues: cues.toList()))) _choose({});
  }

  bool _deleteClaim() {
    if (!mounted || _ui.activePanel != Panel.captions) {
      return _priorDelete?.call() ?? false;
    }
    if (_picks.isEmpty) return false;
    _remove(_picks);
    return true;
  }

  // --- Finding ----------------------------------------------------------------

  void _searchAsked() {
    if (mounted && _ui.searchRequestIsFor(Panel.captions)) {
      _findFocus.requestFocus();
    }
  }

  void _findChanged() {
    setState(_refind);
  }

  void _refind() {
    final find = _find.text;
    _found = find.isEmpty
        ? const []
        : [
            for (var i = 0; i < _rows.length; i++)
              if (captionMatches(_rows[i].text, find,
                      matchCase: _matchCase, wholeWord: _wholeWord)
                  .isNotEmpty)
                i,
          ];
    if (_foundAt >= _found.length) _foundAt = _found.length - 1;
  }

  /// Step to the next or previous caption found, from the playhead.
  void _stepFound(int by) {
    if (_found.isEmpty) return;
    if (_foundAt < 0) {
      final now = _ui.playheadFrame.value;
      _foundAt = by > 0
          ? _found.indexWhere((r) => _rows[r].start >= now)
          : _found.lastIndexWhere((r) => _rows[r].start < now);
      if (_foundAt < 0) _foundAt = by > 0 ? 0 : _found.length - 1;
    } else {
      _foundAt = (_foundAt + by) % _found.length;
    }
    final row = _found[_foundAt];
    setState(() {});
    _choose({_rows[row].index});
    _anchor = row;
    _ui.scrubTo(_rows[row].start);
    _reveal(row);
  }

  /// Change what was found in the caption stepped to, and step on.
  void _replaceOne() {
    final layer = _entry?.layer;
    if (layer == null || _found.isEmpty) return;
    if (_foundAt < 0) _stepFound(1);
    final row = _rows[_found[_foundAt]];
    final at = captionMatches(row.text, _find.text,
        matchCase: _matchCase, wholeWord: _wholeWord);
    if (at.isEmpty) return;
    final text = row.text
        .replaceRange(at.first, at.first + _find.text.length, _with.text);
    if (_try(() => layer.setCueText(cue: row.index, text: text))) {
      _stepFound(0);
    }
  }

  void _replaceAll() {
    final layer = _entry?.layer;
    if (layer == null || _find.text.isEmpty) return;
    var count = 0;
    if (_try(() => count = layer.replaceInCaptions(
        find: _find.text,
        with_: _with.text,
        matchCase: _matchCase,
        wholeWord: _wholeWord))) {
      _app.postNotice(l10n.captionsReplaced(count));
    }
  }

  // --- Keys -------------------------------------------------------------------

  bool _onKey(KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) return false;
    if (!mounted || lumitModalOpen || _ui.activePanel != Panel.captions) {
      return false;
    }
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return false;
    }
    if (_entry == null) return false;
    switch (_ui.keymap.actionFor(BridgeKeyContext.captions, event)) {
      case 'captions.split':
        _split();
        return true;
      case 'captions.merge':
        _join();
        return true;
      case 'captions.add':
        _add();
        return true;
    }
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed || keys.isMetaPressed || keys.isAltPressed) {
      return false;
    }
    switch (event.logicalKey) {
      case LogicalKeyboardKey.arrowUp:
        return _step(-1);
      case LogicalKeyboardKey.arrowDown:
        return _step(1);
      case LogicalKeyboardKey.enter || LogicalKeyboardKey.numpadEnter:
        if (event is KeyRepeatEvent) return true;
        final picks = _picks.toList()..sort();
        final cue =
            picks.firstOrNull ?? (_current >= 0 ? _rows[_current].index : null);
        if (cue == null) return false;
        _type(cue);
        return true;
    }
    return false;
  }

  // --- Drawing ----------------------------------------------------------------

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final comp = _ui.selectedComp;
    if (comp == null) {
      return PlaceholderPanel(
        icon: LumitIcon.text,
        title: l10n.panelCaptions,
        hint: l10n.selectACompositionFirst,
      );
    }
    final progress = _app.transcription.progress;
    final entry = _entry;
    final captions = _captions;
    if (entry == null || captions == null) {
      return ValueListenableBuilder<BridgeTranscribeEvent?>(
        valueListenable: progress,
        builder: (context, event, _) => event == null
            ? captionsEmpty(context, _ui, canCreate: _canCreate)
            : captionsRunning(_app, event),
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _bar(t, comp, entry, captions),
        if (_replacing)
          captionsReplaceBar(
            t,
            replaceWith: _with,
            enabled: _find.text.isNotEmpty && _found.isNotEmpty,
            onReplace: _replaceOne,
            onReplaceAll: _replaceAll,
            matchCase: _matchCase,
            wholeWord: _wholeWord,
            onMatchCase: () => setState(() {
              _matchCase = !_matchCase;
              _refind();
            }),
            onWholeWord: () => setState(() {
              _wholeWord = !_wholeWord;
              _refind();
            }),
          ),
        ValueListenableBuilder<BridgeTranscribeEvent?>(
          valueListenable: progress,
          builder: (context, event, _) => event == null
              ? const SizedBox.shrink()
              : captionsStrip(t, _app, event),
        ),
        Expanded(child: _list(t, captions)),
      ],
    );
  }

  Widget _bar(LumitTheme t, CompositionReference comp, BridgeLayerEntry entry,
      BridgeCaptions captions) {
    final layer = entry.layer;
    final hasRows = _rows.isNotEmpty;
    const gap = SizedBox(width: 6);
    final count = _find.text.isEmpty
        ? null
        : _found.isEmpty
            ? l10n.captionsFoundNone
            : _foundAt < 0
                ? '${_found.length}'
                : l10n.captionsFoundOf(_foundAt + 1, _found.length);
    return Container(
      height: 26,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: panelBar(
        least: _layers.length > 1 ? 400 : 300,
        child: Row(
          children: [
            if (_layers.length > 1) ...[
              SizedBox(
                width: 108,
                height: 20,
                child: BareDropdown<UuidValue>(
                  key: const ValueKey('captions-layer'),
                  dense: true,
                  value: layer.internallayerId,
                  options: _ids(_layers),
                  label: (id) => _layers
                      .firstWhere((e) => e.layer.internallayerId == id)
                      .info
                      .name,
                  onChanged: (id) {
                    _picked = id;
                    final picked = _layers
                        .firstWhere((e) => e.layer.internallayerId == id);
                    _ui.setSelection([picked.layer]);
                    _sync();
                  },
                ),
              ),
              gap,
            ],
            Expanded(
              child: Focus(
                canRequestFocus: false,
                skipTraversal: true,
                onKeyEvent: (_, event) {
                  if (event is! KeyDownEvent ||
                      event.logicalKey != LogicalKeyboardKey.enter) {
                    return KeyEventResult.ignored;
                  }
                  _stepFound(HardwareKeyboard.instance.isShiftPressed ? -1 : 1);
                  return KeyEventResult.handled;
                },
                child: HouseTextField(
                  key: const ValueKey('captions-find'),
                  controller: _find,
                  focusNode: _findFocus,
                  hint: l10n.captionsFind,
                  width: double.infinity,
                  onCancelled: () => _find.clear(),
                ),
              ),
            ),
            if (count != null) ...[
              gap,
              Text(count,
                  key: const ValueKey('captions-found'),
                  style: captionTimeStyle(t)),
              captionsMark(
                  t,
                  'captions-found-prev',
                  l10n.captionsFoundPrevious,
                  LumitIcons.previousKey,
                  _found.isEmpty ? null : (_) => _stepFound(-1)),
              captionsMark(
                  t,
                  'captions-found-next',
                  l10n.captionsFoundNext,
                  LumitIcons.nextKey,
                  _found.isEmpty ? null : (_) => _stepFound(1)),
            ],
            captionsMark(
                t,
                'captions-replace-toggle',
                l10n.captionsReplace,
                LumitIcons.replace,
                (_) => setState(() => _replacing = !_replacing),
                active: _replacing),
            gap,
            captionsMark(t, 'captions-add', l10n.captionsAdd,
                LumitIcons.addToQueue, (_) => _add(),
                action: 'captions.add'),
            captionsMark(t, 'captions-split', l10n.captionsSplit,
                LumitIcons.razor, _current >= 0 ? (_) => _split() : null,
                action: 'captions.split'),
            captionsMark(t, 'captions-join', l10n.captionsJoin, LumitIcons.heal,
                hasRows ? (_) => _join() : null,
                action: 'captions.merge'),
            gap,
            captionsMark(
                t,
                'captions-style',
                l10n.captionsStyle,
                LumitIcons.paint,
                (context) => unawaited(showCaptionStylePopover(context,
                    app: _app,
                    ui: _ui,
                    comp: comp,
                    layer: layer,
                    onEdited: _edited))),
            captionsMark(
                t,
                'captions-limits',
                l10n.captionsLimits,
                LumitIcons.switches,
                (context) => unawaited(showCaptionLimitsPopover(context,
                    app: _app,
                    layer: layer,
                    limits: captions.limits,
                    onEdited: _edited))),
            captionsMark(
                t,
                'captions-export',
                l10n.menuExport,
                LumitIcons.export,
                hasRows
                    ? (context) => unawaited(showMenuEntriesAt(
                        context,
                        popupBelow(context),
                        captionExportRows(_app, layer, entry.info.name),
                        width: 180))
                    : null),
            captionsMark(
                t,
                'captions-more',
                l10n.captionsMore,
                LumitIcons.more,
                (context) => unawaited(showMenuEntriesAt(
                    context,
                    popupBelow(context),
                    captionMoreRows(context, _app, layer, captions, _edited),
                    width: 220))),
          ],
        ),
      ),
    );
  }

  Widget _list(LumitTheme t, BridgeCaptions captions) {
    final (n, d) = _fps;
    final startFrame = _ui.model.startFrame;
    final style = captionTimeStyle(t);
    final painter = TextPainter(
      text: TextSpan(text: timecodeOfRate(0, n, d), style: style),
      textDirection: TextDirection.ltr,
      maxLines: 1,
    )..layout();
    final timeWidth = painter.width.ceilToDouble() + 1;
    painter.dispose();
    final picks = _picks;
    final find = _find.text;
    return LayoutBuilder(builder: (context, box) {
      // The two times side by side once the words still have room beside
      // them, and one over the other before that.
      _wide = box.maxWidth >= timeWidth * 2 + 320;
      final wide = _wide;
      final words = box.maxWidth -
          (wide ? (timeWidth + 8) * 2 + 6 : timeWidth + 8) -
          (2 + 6 + 2 + 8 + 22 + 8);
      _heights.lay(
          words, captionWordStyle(t), MediaQuery.textScalerOf(context));
      return NotificationListener<ScrollNotification>(
        onNotification: (note) {
          if (note is UserScrollNotification && !_scrollingForPlayhead) {
            _scrolledByHand = DateTime.now();
          }
          return false;
        },
        child: ListView.builder(
          key: const ValueKey('captions-list'),
          controller: _scroll,
          itemCount: _rows.length,
          itemExtentBuilder: (i, _) => i < _rows.length ? _extent(i) : null,
          itemBuilder: (context, i) {
            final row = _rows[i];
            final typing = row.index == _typing;
            final timing = _timing?.$1 == row.index ? _timing : null;
            return CaptionRowView(
              key: ValueKey<int>(row.index),
              row: row,
              fps: (n, d),
              startFrame: startFrame,
              wide: wide,
              timeWidth: timeWidth,
              selected: picks.contains(row.index),
              current: i == _current,
              readingSpeed: captions.limits.readingSpeed,
              find: find,
              matchCase: _matchCase,
              wholeWord: _wholeWord,
              typing: typing
                  ? CaptionTypingField(
                      controller: _typeText,
                      focusNode: _typeFocus,
                      onCommit: _commitTyping,
                      onCancel: _cancelTyping,
                    )
                  : null,
              timing: timing == null
                  ? null
                  : HouseTextField(
                      key: const ValueKey('caption-timing'),
                      controller: _timeText,
                      autofocus: true,
                      style: captionTimeStyle(t),
                      padding: const EdgeInsets.symmetric(horizontal: 3),
                      width: double.infinity,
                      onSubmitted: (_) => _commitTime(),
                      onTapOutside: _commitTime,
                      onCancelled: () => setState(() => _timing = null),
                    ),
              timingEnd: timing?.$2 ?? false,
              onTapDown: (_) => _press(i),
              onDoubleTap: () => _type(row.index),
              onTimeDoubleTap: (end) => _time(row.index, end),
              onMenu: (at, word) => _rowMenu(row, at, word),
            );
          },
        ),
      );
    });
  }

  /// A caption's right-click menu, about it and the rest of the pick, and
  /// about [word] when the press was on one of its words.
  void _rowMenu(CaptionRow row, Offset at, int? word) {
    final layer = _entry?.layer;
    if (layer == null) return;
    if (!_picks.contains(row.index)) {
      _choose({row.index});
      _anchor = _rowOf(row.index);
    }
    final frame = _ui.playheadFrame.value;
    final inside = frame > row.start && frame < row.end;
    final raised = row.cue.offsetX != 0 || row.cue.offsetY != 0;
    final last = _rows.isNotEmpty && _rows.last.index == row.index;
    final emphasised = word != null && row.cue.words[word].emphasis;
    // The picked captions together when the row is one of them.
    final cues = [
      row.cue,
      for (final r in _rows)
        if (r.index != row.index && _picks.contains(r.index)) r.cue,
    ];
    unawaited(showMenuEntriesAt(context, at, [
      MenuEntry(l10n.captionsRetype, () => _type(row.index)),
      if (word != null)
        MenuEntry(
            emphasised ? l10n.captionsStopEmphasising : l10n.captionsEmphasise,
            () => _try(() => layer.setWordEmphasis(
                cue: row.index, word: word, on_: !emphasised))),
      MenuEntry(l10n.captionsSplit, inside ? _split : null,
          action: 'captions.split'),
      MenuEntry(
          l10n.captionsJoinNext,
          last
              ? null
              : () => _try(() => layer.mergeCues(first: row.index, count: 2),
                  refused: l10n.captionsNothingToJoin)),
      MenuEntry(
          raised ? l10n.captionsPutDown : l10n.captionsRaise,
          () => _try(() => layer.raiseCues(
              cues: [for (final cue in cues) cue.index], raised: !raised))),
      MenuEntry(
          l10n.captionsPosition,
          () => unawaited(showCaptionPositionPopover(context,
              position: at,
              app: _app,
              layer: layer,
              cues: cues,
              onEdited: _edited))),
      MenuEntry.divider(),
      MenuEntry(l10n.delete, () => _remove(_picks)),
    ]));
  }
}
