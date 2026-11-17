part of 'transcript_panel_frb.dart';

// The Transcript panel's keys: its actions from the keymap, and the caret's
// and the text size's, which it keeps for itself the way a text field keeps
// its own. The shell passes those over while the panel is active, by the
// same test the panel takes them by, so the two cannot disagree.

final _caretKeys = {
  LogicalKeyboardKey.arrowLeft,
  LogicalKeyboardKey.arrowRight,
  LogicalKeyboardKey.arrowUp,
  LogicalKeyboardKey.arrowDown,
  LogicalKeyboardKey.home,
  LogicalKeyboardKey.end,
};

final _largerKeys = {
  LogicalKeyboardKey.equal,
  LogicalKeyboardKey.add,
  LogicalKeyboardKey.numpadAdd,
};

final _smallerKeys = {
  LogicalKeyboardKey.minus,
  LogicalKeyboardKey.numpadSubtract,
};

/// What [event] does in the Transcript panel: moves the caret, makes the
/// text larger or smaller, or nothing the panel keeps.
enum TranscriptKey { caret, larger, smaller }

TranscriptKey? transcriptKeyOf(KeyEvent event) {
  final keys = HardwareKeyboard.instance;
  if (keys.isAltPressed) return null;
  final key = event.logicalKey;
  if (_caretKeys.contains(key)) return TranscriptKey.caret;
  if (!keys.isControlPressed && !keys.isMetaPressed) return null;
  if (_largerKeys.contains(key)) return TranscriptKey.larger;
  if (_smallerKeys.contains(key)) return TranscriptKey.smaller;
  return null;
}

extension _TranscriptKeys on _TranscriptPanelFrbState {
  bool _key(KeyEvent event) {
    if (!mounted || event is KeyUpEvent) return false;
    if (lumitModalOpen || _ui.activePanel != Panel.transcript) return false;
    // The busy card holds every key while it is up.
    if (_app.busy.value != null) return false;
    // A focused control or field keeps its keys, as in the shell.
    final focus = FocusManager.instance.primaryFocus;
    if (focus is ControlFocusNode) return false;
    final focused = focus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return false;
    }
    final action = _ui.keymap.actionFor(BridgeKeyContext.transcript, event);
    if (action != null && action.startsWith('transcript.')) {
      if (event is KeyRepeatEvent) return true;
      _runAction(action, event);
      return true;
    }
    final kind = transcriptKeyOf(event);
    if (kind == null) return false;
    if (kind != TranscriptKey.caret) {
      _setTextSize(
          _s.transcriptTextSize + (kind == TranscriptKey.larger ? 1 : -1));
      return true;
    }
    if (_model.tokens.isEmpty) return false;
    final keys = HardwareKeyboard.instance;
    final shift = keys.isShiftPressed;
    final ctrl = keys.isControlPressed || keys.isMetaPressed;
    final key = event.logicalKey;
    final caret = _selection.caret ?? _selection.bounds?.$1 ?? 0;
    final last = _model.tokens.length;
    int? to;
    double? keepX;
    if (key == LogicalKeyboardKey.arrowLeft) {
      to = ctrl ? _wordStep(caret, -1) : math.max(0, caret - 1);
    } else if (key == LogicalKeyboardKey.arrowRight) {
      to = ctrl ? _wordStep(caret, 1) : math.min(last, caret + 1);
    } else if (key == LogicalKeyboardKey.arrowUp ||
        key == LogicalKeyboardKey.arrowDown) {
      final layout = _layout;
      if (layout == null) return true;
      final x = _goalX ?? _caretX(layout, caret) ?? 0;
      to = layout.lineStep(caret, x, down: key == LogicalKeyboardKey.arrowDown) ??
          (key == LogicalKeyboardKey.arrowDown ? last : 0);
      keepX = x;
    } else if (key == LogicalKeyboardKey.home) {
      to = ctrl ? 0 : _lineEnd(caret, start: true);
    } else if (key == LogicalKeyboardKey.end) {
      to = ctrl ? last : _lineEnd(caret, start: false);
    }
    if (to == null) return false;
    _moveCaret(to, extend: shift);
    _goalX = keepX;
    return true;
  }

  int _wordStep(int caret, int by) {
    var i = caret + by;
    while (i > 0 && i < _model.tokens.length && !_model.tokens[i].isWord) {
      i += by;
    }
    return i.clamp(0, _model.tokens.length);
  }

  double? _caretX(TranscriptLayout layout, int caret) {
    final p = layout.paragraphOf(caret.clamp(0, _model.tokens.length - 1));
    return p?.caretAt(caret)?.x;
  }

  int _lineEnd(int caret, {required bool start}) {
    final layout = _layout;
    final tokens = _model.tokens;
    if (layout == null) return caret;
    final at = caret.clamp(0, tokens.length - 1);
    final p = layout.paragraphOf(at);
    if (p == null) return caret;
    final (first, lastOnLine) =
        p.lineTokens(p.lineOf(caret >= tokens.length ? p.to : caret));
    return start ? first : math.min(lastOnLine + 1, tokens.length);
  }

  /// The caret moves, extending the selection with Shift, and with nothing
  /// playing the playhead goes with it.
  void _moveCaret(int to, {required bool extend}) {
    final from = _selection.caret ?? to;
    TranscriptSelection next;
    if (extend) {
      _anchor ??= from;
      next = TranscriptSelection.of(_anchor!, to, caret: to);
    } else {
      _anchor = null;
      next = TranscriptSelection(caret: to);
    }
    final seek = !_ui.playing.value;
    _select(next, seek: seek);
    _revealToken(to.clamp(0, math.max(0, _model.tokens.length - 1)),
        sought: seek);
  }

  void _runAction(String action, KeyEvent event) {
    switch (action) {
      case 'transcript.extract':
        _remove(
            ripple: true,
            before: event.logicalKey == LogicalKeyboardKey.backspace);
      case 'transcript.lift':
        _remove(ripple: false, name: 'Lift words');
      case 'transcript.restore':
        _restore();
      case 'transcript.correct':
        _startCorrect();
      case 'transcript.insert':
        _placeWords(insert: true);
      case 'transcript.overwrite':
        _placeWords(insert: false);
      case 'transcript.find.next':
        _stepFind(1);
      case 'transcript.find.prev':
        _stepFind(-1);
      case 'transcript.mark':
        _mark();
      case 'transcript.fillers':
        _openFind(TranscriptFindMode.fillers);
      case 'transcript.pauses':
        _openFind(TranscriptFindMode.pauses);
    }
  }
}
