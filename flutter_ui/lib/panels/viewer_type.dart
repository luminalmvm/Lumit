// The Type tool: making and editing text layers on the picture (docs/07 §1.7,
// §2.3.2).
//
// **In plain terms.** With the Type tool in hand, clicking empty picture makes a
// **new text layer** where you clicked and puts a caret there; clicking an
// existing text layer edits *that* one, with the caret where you clicked. Drag
// across the words to select some of them, double-click for a word, triple-click
// for the line, Shift-click to stretch a selection. What you type appears in the
// picture as you type it, and the edit ends when you press `Escape`, press
// `Enter`, click somewhere else, or put the tool down. `Shift+Enter` starts a
// new line. A new layer you never
// typed anything into is removed again — After Effects does the same, and a
// project full of empty text layers left by stray clicks is nobody's idea of a
// feature.
//
// **Why the document is written only once.** Every edit to the document is an
// undo step, so writing the layer on each keystroke would make `Ctrl+Z` walk
// back through a sentence one letter at a time. Instead the picture is kept in
// step with `render_frame_with_text_preview` — the same live-preview path a
// dragged transform uses, which shows a provisional value without the
// document ever holding it — and the layer is written once, when the edit ends.
// One typing session, one undo step.
//
// **Where the caret comes from.** The typing itself is a real Flutter text
// field, so arrows, selection, backspace, paste and IME all behave as they do
// everywhere else — but its *drawing* is turned off, because the text the user
// should see is the engine's own rendering of the layer. What is drawn here is
// the caret and the selection, placed by the engine's own layout of the block
// (`measuredText`). It's the same layout the rasteriser sets the letters with,
// so the caret stands in the gap between two letters and the box hugs the
// words. Up, Down, Home and End are placed by that layout too, since the
// hidden field has no idea where the lines are.
//
// **Why the field is always there.** It stays mounted for as long as the tool
// is in hand, and only takes focus while an edit is open. A field built at the
// moment of the click takes the keyboard a frame late, so the first letters
// typed would run as shortcuts instead (`S` for Scale). Clicks on the picture
// are this tool's to read, and a click anywhere else in the application ends
// the edit.

import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';

import '../l10n/strings.dart';
import '../state/layer_bounds.dart' show measuredText;
import '../state/text_documents.dart';
import '../state/preview_throttle.dart';
import '../widgets/controls.dart';
import 'viewer_gizmo.dart';
import 'viewer_shape_layer.dart' show ShapeSpace;
import 'viewer_tool_cursor.dart';
import '../widgets/escape_ladder.dart';
import 'viewer_layer_map.dart';

/// The anchor a text layer saying [document] wants: the middle of the box the
/// engine draws it into, so it scales and turns about itself rather than about
/// its first letter. Empty text has no middle, and keeps its anchor where the
/// line starts.
Offset textAnchor(BridgeTextDocument document) {
  final block = measuredText(document);
  if (document.text.isEmpty) {
    final line = block.lines.first;
    return Offset(line.carets.first, line.baseline);
  }
  return Offset(block.width * 0.5, block.height * 0.5);
}

/// The gap on [line] nearest [x] (layer pixels), counted from the line's own
/// first character.
int nearestGapOn(BridgeTextBlockLine line, double x) {
  final carets = line.carets;
  var best = 0;
  var bestDistance = double.infinity;
  for (var i = 0; i < carets.length; i++) {
    final d = (carets[i] - x).abs();
    if (d < bestDistance) {
      best = i;
      bestDistance = d;
    }
  }
  return best;
}

/// The character gap nearest [at] (layer pixels) in [block], which is the
/// caret a click there puts down: the line whose baseline is nearest, then the
/// nearest gap on it. Counted in characters over the whole text, as the engine
/// counts them.
int caretNearest(BridgeTextBlock block, Offset at) {
  var line = block.lines.first;
  var bestDistance = double.infinity;
  // A line's letters sit above its baseline, so the middle of that band is
  // what a click is nearest to.
  final middle = (block.ascent - block.descent) * 0.5;
  for (final candidate in block.lines) {
    final d = (candidate.baseline - middle - at.dy).abs();
    if (d < bestDistance) {
      line = candidate;
      bestDistance = d;
    }
  }
  return line.start + nearestGapOn(line, at.dx);
}

/// Where the caret in front of character [index] stands in [block]: which
/// line, and how far along it. The break that ends a line belongs to the line
/// it ends, so a caret there stands after its last letter.
({int line, double x}) caretPlace(BridgeTextBlock block, int index) {
  var at = 0;
  for (var i = 1; i < block.lines.length; i++) {
    if (block.lines[i].start <= index) at = i;
  }
  final line = block.lines[at];
  final gap = (index - line.start).clamp(0, line.carets.length - 1);
  return (line: at, x: line.carets[gap]);
}

/// [text]'s UTF-16 offset (what a [TextSelection] counts) as a character index
/// (what the engine's carets count). The two differ past any character outside
/// the Basic Multilingual Plane, since an emoji is two of one and one of the
/// other.
int characterIndexOf(String text, int utf16) {
  final end = utf16.clamp(0, text.length);
  var characters = 0;
  for (var i = 0; i < end; i++) {
    final unit = text.codeUnitAt(i);
    // The second half of a pair counts nothing; the first counts the whole.
    if (unit < 0xDC00 || unit > 0xDFFF) characters++;
  }
  return characters;
}

/// The inverse of [characterIndexOf].
int utf16OffsetOf(String text, int characters) {
  var offset = 0;
  var seen = 0;
  for (final rune in text.runes) {
    if (seen >= characters) break;
    offset += rune > 0xFFFF ? 2 : 1;
    seen++;
  }
  return offset;
}

/// The word round UTF-16 offset [at] in [text], which a double-click selects.
/// A run of letters and digits, a run of spaces, or one other character.
TextSelection wordAround(String text, int at) {
  if (text.isEmpty) return const TextSelection.collapsed(offset: 0);
  final word = RegExp(r'[\p{L}\p{N}_]', unicode: true);
  final space = RegExp(r'\s');
  int classOf(int i) {
    final c = text[i];
    if (word.hasMatch(c)) return 0;
    if (space.hasMatch(c)) return 1;
    return 2;
  }

  // The character the click is on: the one after the caret, or the last one
  // when the click was past the end of the line.
  final i = at.clamp(0, text.length - 1);
  final kind = classOf(i);
  if (kind == 2) return TextSelection(baseOffset: i, extentOffset: i + 1);
  var start = i;
  var end = i + 1;
  while (start > 0 && classOf(start - 1) == kind) {
    start--;
  }
  while (end < text.length && classOf(end) == kind) {
    end++;
  }
  return TextSelection(baseOffset: start, extentOffset: end);
}

/// A text layer the Selection tool's double-click asked to have opened for
/// typing: the view it was clicked in, the layer, and where on it. The gizmo
/// sets this and arms the Type tool; that view's Type layer reads it as the
/// tool arrives, and the gizmo clears it when the frame is done.
({int? view, LayerReference layer, Offset at})? viewerTypeEditRequest;

/// The Type tool over the picture.
class ViewerTypeLayer extends StatefulWidget {
  /// Whether a type tool is armed. Inert otherwise.
  final bool active;

  /// The engine's id for the view this is drawn over, so a request made in one
  /// view is not answered in another.
  final int? viewId;

  final ToolMode tool;
  final CompositionReference comp;
  final LumitState state;
  final LumitUiState uiState;

  /// Every layer with its box, top first — for finding the text layer under a
  /// click and for placing the caret over it.
  final List<LayerBox> boxes;

  /// Where the picture sits on screen.
  final Rect fitted;

  /// The composition's size in its own pixels.
  final Size compSize;

  final Color accent;

  final VoidCallback onChanged;

  const ViewerTypeLayer({
    super.key,
    required this.active,
    required this.tool,
    required this.comp,
    required this.state,
    required this.uiState,
    required this.boxes,
    required this.fitted,
    required this.compSize,
    required this.accent,
    required this.onChanged,
    this.viewId,
  });

  @override
  State<ViewerTypeLayer> createState() => _ViewerTypeLayerState();
}

class _ViewerTypeLayerState extends State<ViewerTypeLayer> {
  /// The layer being typed into, if any.
  LayerReference? _editing;

  /// Whether this tool made that layer, so an edit that ends with nothing typed
  /// can take it away again.
  bool _created = false;

  /// The document the edit opened on, for the moment the layer can't be read.
  BridgeTextDocument? _held;

  /// The words as the picture was last asked to show them.
  String _typed = '';

  /// Where the pointer is, for the drawn beam vertical type wears.
  Offset? _pointer;

  /// Whether the press under way belongs to the words: it landed on a text
  /// layer, so a drag from it selects rather than panning the picture.
  bool _pressOnText = false;

  /// Where a drag-select is anchored, as a UTF-16 offset: the end of the
  /// selection that stays put while the other follows the pointer.
  int _dragBase = 0;

  /// What a double- or triple-click selected, so dragging on from it grows by
  /// whole words, or keeps the whole line.
  TextSelection? _dragUnit;

  /// Consecutive clicks in one place: two selects a word, three the line.
  int _clicks = 0;
  Offset? _lastClickPos;
  Timer? _clickRun;

  final TextEditingController _controller = TextEditingController();

  /// Never tabbed to, and unable to take focus unless an edit is open, since a
  /// field holding the keyboard with no edit under way would swallow every
  /// shortcut in the application.
  final FocusNode _focus = FocusNode(
    debugLabel: 'Type tool',
    skipTraversal: true,
    canRequestFocus: false,
  );
  final PreviewThrottle _throttle = PreviewThrottle();

  /// Whether the drag recogniser takes part in a press. It asks as the press
  /// arrives, which is after [_onPointerDown] has looked at where it landed.
  /// A drag that starts on the words selects, and one on empty picture draws
  /// the box new text will wrap to.
  bool _dragAllowed(int buttons) =>
      buttons == kPrimaryButton && widget.tool != ToolMode.typeVertical;

  /// The two corners of a text box being dragged out on empty picture.
  Offset? _boxFrom;
  Offset? _boxTo;

  /// A box narrower than this on screen was a click with a shaky hand.
  static const double _smallestBox = 8;

  void _dragStart(Offset at) {
    if (_pressOnText) return _dragTo(at);
    setState(() {
      _boxFrom = _lastClickPos ?? at;
      _boxTo = at;
    });
  }

  void _dragUpdate(Offset at) {
    if (_boxFrom == null) return _dragTo(at);
    setState(() => _boxTo = at);
  }

  void _dragEnd() {
    _dragUnit = null;
    final from = _boxFrom;
    final to = _boxTo;
    if (from == null || to == null) return;
    setState(() => _boxFrom = _boxTo = null);
    final box = Rect.fromPoints(from, to);
    if (box.width < _smallestBox) return _create(from);
    _create(box.topLeft, boxScreenWidth: box.width);
  }

  @override
  void initState() {
    super.initState();
    _controller.addListener(_onTyped);
    HardwareKeyboard.instance.addHandler(_onKey);
    _escapeRelease = EscapeLadder.register(EscapeRung.gesture, _escape);
  }

  /// How to stand down from the ladder.
  VoidCallback? _escapeRelease;

  /// Escape ends the edit, which the tool always promised and never did — the
  /// ladder's gesture rung (widgets/escape_ladder.dart), because a sentence
  /// being typed is the innermost thing on screen.
  bool _escape() {
    if (!_editingNow) return false;
    _finish();
    return true;
  }

  /// The other key a typing session has to answer while the text field has the
  /// keyboard; Escape is [_escape] on the ladder.
  ///
  /// **Ctrl+Z** ends the edit as well and then lets go: an undo pressed mid-sentence
  /// used to be swallowed by the text field, so the document did not move and
  /// the application looked as though undo had stopped working. Ending the edit
  /// first is what makes the next `Ctrl+Z` undo the thing the user means — the
  /// line they just typed, and after that the layer itself.
  ///
  /// **Shift+Enter** starts a new line, and the keys that move by line are
  /// answered here from the engine's layout.
  bool _onKey(KeyEvent event) {
    if (!_editingNow || event is KeyUpEvent) return false;
    if (_moveByLine(event)) return true;
    if (event is! KeyDownEvent) return false;
    final keys = HardwareKeyboard.instance;
    final enter = event.logicalKey == LogicalKeyboardKey.enter ||
        event.logicalKey == LogicalKeyboardKey.numpadEnter;
    if (enter && keys.isShiftPressed) {
      final value = _controller.value;
      final selection = value.selection.isValid
          ? value.selection
          : TextSelection.collapsed(offset: value.text.length);
      _controller.value = TextEditingValue(
        text: value.text.replaceRange(selection.start, selection.end, '\n'),
        selection: TextSelection.collapsed(offset: selection.start + 1),
      );
      return true;
    }
    final undo = event.logicalKey == LogicalKeyboardKey.keyZ &&
        (keys.isControlPressed || keys.isMetaPressed);
    if (!undo) return false;
    // Written, then handed on: the shell's own undo takes it from here, so
    // there is one undo path in the application rather than two.
    _finish();
    return false;
  }

  /// Up, Down, Home and End: the caret goes to the gap the engine's layout
  /// says is there, and Shift stretches the selection to it. Up from the first
  /// line goes to the start of the text and Down from the last to its end.
  bool _moveByLine(KeyEvent event) {
    final key = event.logicalKey;
    final up = key == LogicalKeyboardKey.arrowUp;
    final down = key == LogicalKeyboardKey.arrowDown;
    final home = key == LogicalKeyboardKey.home;
    final end = key == LogicalKeyboardKey.end;
    if (!(up || down || home || end)) return false;
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed || keys.isMetaPressed || keys.isAltPressed) {
      return false;
    }
    final block = _block;
    final selection = _controller.selection;
    if (block == null || !selection.isValid) return false;
    final text = _controller.text;
    final here =
        caretPlace(block, characterIndexOf(text, selection.extentOffset));
    final line = block.lines[here.line];
    final int target;
    if (home) {
      target = line.start;
    } else if (end) {
      target = line.start + line.carets.length - 1;
    } else if (up && here.line == 0) {
      target = 0;
    } else if (down && here.line == block.lines.length - 1) {
      target = characterIndexOf(text, text.length);
    } else {
      final next = block.lines[here.line + (down ? 1 : -1)];
      target = next.start + nearestGapOn(next, here.x);
    }
    final offset = utf16OffsetOf(text, target);
    _controller.selection = keys.isShiftPressed
        ? selection.extendTo(TextPosition(offset: offset))
        : TextSelection.collapsed(offset: offset);
    return true;
  }

  @override
  void didUpdateWidget(ViewerTypeLayer old) {
    super.didUpdateWidget(old);
    // Putting the tool down finishes the edit, as does swapping horizontal for
    // vertical: an edit belongs to the tool that started it.
    //
    // After the frame, not inside it. This runs while the tree above is
    // building, and [_finish] writes the document, clears the live-text
    // notifier and calls `onChanged` — a notifier that fires mid-build marks
    // an ancestor dirty in the middle of its own build, which is an assertion
    // in a debug build and a dropped rebuild in a release one. The edit ends
    // either way; it now ends as the frame commits rather than during it.
    // Repeat calls are harmless: the first clears `_editing` and the rest
    // return at the top.
    if (!widget.active || widget.tool != old.tool) {
      // A double-click with the Selection tool arms this tool and asks for a
      // layer to be opened with it. Read now and opened after the frame, once
      // whatever was being typed has been written.
      final asked = viewerTypeEditRequest;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        _finish();
        if (asked == null || !widget.active || asked.view != widget.viewId) {
          return;
        }
        final id = asked.layer.internallayerId;
        for (final box in widget.boxes) {
          if (box.id != id) continue;
          _begin(asked.layer, created: false, box: box, at: asked.at);
          break;
        }
      });
    }
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    _escapeRelease?.call();
    _escapeRelease = null;
    _clearLive();
    _throttle.cancel();
    _clickRun?.cancel();
    _controller.dispose();
    _focus.dispose();
    super.dispose();
  }

  bool get _editingNow => _editing != null;

  /// The box of the layer being typed into, as the Viewer has it this frame.
  /// Null for the frame between a new layer being made and the boxes being
  /// walked again.
  LayerBox? get _editingBox {
    final id = _editing?.internallayerId;
    if (id == null) return null;
    for (final box in widget.boxes) {
      if (box.id == id) return box;
    }
    return null;
  }

  /// The document being typed into, as the layer has it now. The Text panel
  /// can restyle the layer while an edit is open, so it is read each time.
  BridgeTextDocument? get _document {
    final layer = _editing;
    if (layer == null) return null;
    try {
      return layer.getText() ?? _held;
    } catch (_) {
      return _held;
    }
  }

  /// The words being typed, laid out as the engine lays them out.
  BridgeTextBlock? get _block {
    final document = _document;
    if (document == null) return null;
    return measuredText(document, text: _controller.text);
  }

  /// The UTF-16 offset of the gap nearest [at] on screen, in the words being
  /// typed.
  int _offsetAt(LayerBox box, Offset at) {
    final block = _block;
    if (block == null) return _controller.text.length;
    return utf16OffsetOf(
        _controller.text, caretNearest(block, box.map.layerOf(at)));
  }

  /// Whether a press at [at] is on the words being typed: inside the layer's
  /// box, or the band the carets occupy, with a little room round either so a
  /// click just past the last letter still lands on the line.
  bool _onEditingText(Offset at) {
    final box = _editingBox;
    final block = _block;
    if (box == null || block == null) return false;
    final p = box.map.layerOf(at);
    final pad = (_document?.size ?? 72) * 0.25;
    final top = block.lines.first.baseline - block.ascent;
    final bottom = block.lines.last.baseline + block.descent;
    final inBand = p.dx >= block.left - pad &&
        p.dx <= block.right + pad &&
        p.dy >= top - pad &&
        p.dy <= bottom + pad;
    return inBand || box.contains(at);
  }

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return const SizedBox.shrink();
    final viewScale = widget.fitted.width / widget.compSize.width;
    // Horizontal type wears the system's own I-beam; vertical type has one
    // drawn for it, because no platform ships a sideways beam.
    final vertical = widget.tool == ToolMode.typeVertical;
    final t = ThemeScope.of(context).theme;
    final box = _editingNow ? _editingBox : null;
    final block = _block;
    final text = _controller.text;
    final selection = _controller.selection;
    final valid = selection.isValid;
    final start = characterIndexOf(text, valid ? selection.start : text.length);
    final end = characterIndexOf(text, valid ? selection.end : text.length);
    final place = block == null ? null : caretPlace(block, end);
    final caretAt = place == null
        ? null
        : box?.map.toScreen(place.x,
            block!.lines[place.line].baseline - block.ascent);
    // Clicks on the picture are this tool's (they place the caret, or make a
    // layer), so they are inside the field's tap region and don't take its
    // focus away. A click anywhere else in the application is outside it, and
    // ends the edit.
    return Positioned.fill(
      child: TextFieldTapRegion(
        child: DrawnPointerRegion(
          cursor: vertical ? SystemMouseCursors.none : SystemMouseCursors.text,
          onPointer: (at) => setState(() => _pointer = at),
          child: RawGestureDetector(
            behavior: HitTestBehavior.opaque,
            gestures: {
              PanGestureRecognizer:
                  GestureRecognizerFactoryWithHandlers<PanGestureRecognizer>(
                () => PanGestureRecognizer(
                  debugOwner: this,
                  allowedButtonsFilter: _dragAllowed,
                ),
                (r) => r
                  ..onStart = ((d) => _dragStart(d.localPosition))
                  ..onUpdate = ((d) => _dragUpdate(d.localPosition))
                  ..onEnd = ((_) => _dragEnd())
                  ..onCancel = (() => setState(() => _boxFrom = _boxTo = null)),
              ),
              TapGestureRecognizer:
                  GestureRecognizerFactoryWithHandlers<TapGestureRecognizer>(
                () => TapGestureRecognizer(debugOwner: this),
                (r) => r.onTapUp = _onTapUp,
              ),
            },
            // Under the recognisers, so the press is read and the caret put down
            // before either of them is asked whether it wants it.
            child: Listener(
              behavior: HitTestBehavior.opaque,
              onPointerDown: _onPointerDown,
              child: Stack(
                children: [
                  if (_boxFrom != null && _boxTo != null)
                    Positioned.fromRect(
                      rect: Rect.fromPoints(_boxFrom!, _boxTo!),
                      child: IgnorePointer(
                        child: DecoratedBox(
                          decoration: BoxDecoration(
                            border: Border.all(color: widget.accent),
                          ),
                        ),
                      ),
                    ),
                  if (vertical)
                    TextPointer(
                      at: _pointer,
                      mark: t.textPrimary,
                      outline: t.surface0,
                    ),
                  Positioned(
                    left: caretAt?.dx ?? 0,
                    top: caretAt?.dy ?? 0,
                    width: 1,
                    height: 1,
                    // The field itself never shows: the text a user should see
                    // is the engine's rendering of the layer, and a second copy
                    // of it in a different font on top of that would only
                    // disagree. What it is here for is the keyboard — arrows,
                    // backspace, selection, paste and IME, all of it for free.
                    // Invisible rather than *offstage*, because an offstage
                    // field is not built, and one that is not built takes no
                    // keystrokes. Placed on the caret so an IME's candidate
                    // window opens beside the words.
                    child: Opacity(
                      opacity: 0,
                      child: EditableText(
                        controller: _controller,
                        focusNode: _focus,
                        style: TextStyle(
                            fontSize: (_document?.size ?? 72) * viewScale),
                        cursorColor: widget.accent,
                        backgroundCursorColor: widget.accent,
                        // Several lines, so a pasted break is kept. Enter
                        // still ends the edit, and Shift+Enter is what breaks
                        // a line ([_onKey]).
                        maxLines: null,
                        textInputAction: TextInputAction.done,
                        // A desktop field selects all its text when it takes
                        // focus, which would throw away the caret a click
                        // just placed.
                        selectAllOnFocus: false,
                        onSubmitted: (_) => _finish(),
                        // A press outside the picture ends the edit; a press
                        // on it never reaches here (the tap region above).
                        onTapOutside: (_) => _finish(),
                      ),
                    ),
                  ),
                  Positioned.fill(
                    child: IgnorePointer(
                      child: CustomPaint(
                        painter: _SelectionPainter(
                          map: box?.map,
                          block: block,
                          start: start,
                          end: end,
                          accent: widget.accent,
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  // --- The gesture ----------------------------------------------------------

  /// A press on the picture, read before any recogniser decides what it is.
  ///
  /// On the words being typed it puts the caret down where it landed. A second
  /// or third click selects the word or the line, and Shift stretches the
  /// selection there. On another text layer it ends this edit and opens that
  /// one, caret where it landed. Either way the press now belongs to the
  /// words, so dragging on selects. On empty picture it ends the edit and
  /// leaves the rest to the tap (which makes a layer) or to the pan.
  void _onPointerDown(PointerDownEvent event) {
    _pressOnText = false;
    if (event.buttons != kPrimaryButton) return;
    if (widget.tool == ToolMode.typeVertical) return;
    final at = event.localPosition;
    final clicks = _countClick(event);

    if (_editingNow && _onEditingText(at)) {
      _pressOnText = true;
      final box = _editingBox!;
      final hit = _offsetAt(box, at);
      final text = _controller.text;
      final TextSelection selection;
      if (clicks >= 3) {
        selection = TextSelection(baseOffset: 0, extentOffset: text.length);
      } else if (clicks == 2) {
        selection = wordAround(text, hit);
      } else if (HardwareKeyboard.instance.isShiftPressed &&
          _controller.selection.isValid) {
        selection = _controller.selection.extendTo(TextPosition(offset: hit));
      } else {
        selection = TextSelection.collapsed(offset: hit);
      }
      _dragUnit = clicks >= 2 ? selection : null;
      _dragBase = selection.baseOffset;
      _controller.selection = selection;
      _takeKeyboard();
      return;
    }

    // Whatever was being typed is finished first: a click elsewhere is what
    // ends an edit, exactly as it does in After Effects.
    _finish();
    final existing = _textLayerAt(at);
    if (existing == null) return;
    _pressOnText = true;
    _begin(existing.layer, created: false, box: existing.box, at: at);
    _dragUnit = null;
    _dragBase = _controller.selection.baseOffset;
  }

  /// Counts this press into a run of clicks in one place, by the platform's
  /// double-click time and distance, and returns where in the run it is. The
  /// run is ended by a timer, the same way the framework's own double-tap
  /// recogniser does it.
  int _countClick(PointerDownEvent event) {
    final lastPos = _lastClickPos;
    final near = lastPos != null &&
        (event.localPosition - lastPos).distance <= kDoubleTapSlop;
    _clicks = _clicks > 0 && near ? _clicks + 1 : 1;
    _lastClickPos = event.localPosition;
    _clickRun?.cancel();
    _clickRun = Timer(kDoubleTapTimeout, () => _clicks = 0);
    return _clicks;
  }

  /// A drag from the words: the far end of the selection follows the pointer,
  /// letter by letter, or word by word after a double-click.
  void _dragTo(Offset at) {
    final box = _editingBox;
    if (!_editingNow || box == null) return;
    final text = _controller.text;
    final hit = _offsetAt(box, at);
    final unit = _dragUnit;
    if (unit != null && !unit.isCollapsed) {
      if (unit.end - unit.start == text.length) return; // the whole line
      final word = wordAround(text, hit);
      _controller.selection = hit < unit.start
          ? TextSelection(baseOffset: unit.end, extentOffset: word.start)
          : TextSelection(
              baseOffset: unit.start,
              extentOffset: word.end > unit.end ? word.end : unit.end,
            );
      return;
    }
    _controller.selection =
        TextSelection(baseOffset: _dragBase, extentOffset: hit);
  }

  /// A click that came to nothing else: on empty picture it makes a layer.
  void _onTapUp(TapUpDetails details) {
    if (widget.tool == ToolMode.typeVertical) {
      widget.state.postNotice(
        l10n.typeVerticalNotBuilt,
      );
      return;
    }
    // A click on the words was handled as it went down.
    if (_pressOnText) return;
    _create(details.localPosition);
  }

  /// Give the hidden field the keyboard now. It is always built while the tool
  /// is in hand, so the request lands this frame.
  ///
  /// And once more after the frame, in case something else answering the same
  /// press (a panel claiming the click, say) moved focus after this did.
  void _takeKeyboard() {
    _focus.canRequestFocus = true;
    if (!_focus.hasFocus) _focus.requestFocus();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted && _editingNow && !_focus.hasFocus) _focus.requestFocus();
    });
  }

  /// The topmost text layer whose box contains [at], or null.
  ///
  /// Which layers are text comes off the read model, so a click costs
  /// no bridge calls however deep the stack — it used to ask `getText()` of
  /// every layer under the pointer. The one read the edit needs is asked of
  /// the layer chosen, in [_begin].
  ({LayerBox box, LayerReference layer})? _textLayerAt(Offset at) {
    final textIds = {
      for (final entry in widget.uiState.model.heldLayers)
        if (entry.info.kind == BridgeLayerKind.text)
          entry.layer.internallayerId,
    };
    for (final box in widget.boxes) {
      if (box.contains(at) && textIds.contains(box.id)) {
        return (box: box, layer: box.layer);
      }
    }
    return null;
  }

  /// Make a text layer where the pointer is, and start typing into it.
  ///
  /// [boxScreenWidth] is the width of a box dragged out with its top left at
  /// [at]. The text then wraps to it, and starts inside its top left corner
  /// rather than standing on the point.
  void _create(Offset at, {double? boxScreenWidth}) {
    final options = widget.uiState.tools;
    // The composition's own placement — the same conversion the shape tools
    // build a new layer's art with.
    final space =
        ShapeSpace.ofComp(fitted: widget.fitted, compSize: widget.compSize);
    var (cx, cy) = space.ofScreen(at);
    var paragraph = options.paragraphStyle;
    if (boxScreenWidth != null && space.screenScale > 0) {
      paragraph =
          paragraph.copyWith(boxWidth: boxScreenWidth / space.screenScale);
      // The engine stands the new layer's first baseline on the point it is
      // given, so the point is moved from the box's corner to that baseline.
      final block = measuredText(
          BridgeTextDocument(
            text: '',
            size: options.textSize,
            fill: options.fillRgba,
            pathOffset: const BridgeScalar.static_(0),
            animators: const [],
            style: options.textStyle,
            paragraph: paragraph,
          ),
          text: '');
      cx += block.lines.first.carets.first - block.left;
      cy += block.ascent;
    }
    try {
      // One op, so one undo step, and undoing it takes the layer away.
      // This used to be three — a layer saying "Text" in the middle of the
      // composition, then an empty line written into it, then a move to the
      // click — so `Ctrl+Z` walked back through two states nobody had ever
      // seen before the layer finally went.
      //
      // The anchor the engine gives it sits on the left end of the baseline, so
      // what is typed runs to the right of the pointer and sits on it rather
      // than straddling it; it is recentred on the finished line when the edit
      // ends.
      final layer = widget.comp.addTextLayerAt(
        document: BridgeTextDocument(
          text: '',
          size: options.textSize,
          fill: options.fillRgba,
          // A layer being made has no mask to run along yet, so it lays
          // straight, and has no letters to animate separately.
          pathOffset: const BridgeScalar.static_(0),
          animators: const [],
          // What the Text and Paragraph panels were last set to with no text
          // layer selected.
          style: options.textStyle,
          paragraph: paragraph,
        ),
        x: cx,
        y: cy,
      );
      widget.uiState.setSelection([layer]);
      _begin(layer, created: true);
      widget.onChanged();
    } catch (_) {
      widget.state.postNotice(l10n.couldNotAddTextLayer, error: true);
    }
  }

  /// Open an edit on [layer]. A click on its words ([at], over [box]) puts the
  /// caret in the gap nearest the click; otherwise it goes after the last
  /// letter.
  void _begin(
    LayerReference layer, {
    required bool created,
    LayerBox? box,
    Offset? at,
  }) {
    final document = () {
      try {
        return layer.getText();
      } catch (_) {
        return null;
      }
    }();
    if (document == null) return;
    final text = document.text;
    final caret = box != null && at != null
        ? utf16OffsetOf(
            text,
            caretNearest(measuredText(document), box.map.layerOf(at)),
          )
        : text.length;
    setState(() {
      _editing = layer;
      _created = created;
      _held = document;
      _typed = text;
      _controller.value = TextEditingValue(
        text: text,
        selection: TextSelection.collapsed(offset: caret),
      );
    });
    _takeKeyboard();
    _publishLive(layer);
  }

  /// Tell the Viewer's boxes what is being typed, and stop telling it when the
  /// edit ends — the document is the only truth from then on.
  void _publishLive(LayerReference layer) {
    final document = _document;
    if (document == null) return;
    widget.uiState.liveText.value = {
      layer.internallayerId: document.copyWith(text: _controller.text),
    };
  }

  void _clearLive() {
    if (widget.uiState.liveText.value.isNotEmpty) {
      widget.uiState.liveText.value = const {};
    }
  }

  /// Every keystroke: the picture keeps up through the preview path, and the
  /// caret moves. The document is not touched.
  void _onTyped() {
    if (!_editingNow) return;
    setState(() {});
    // The controller speaks for a moved caret or a dragged selection as well
    // as for a letter typed; only a change to the words re-renders them.
    if (_controller.text == _typed) return;
    _typed = _controller.text;
    final layer = _editing!;
    // What the box round the words should be measured from while they are
    // being typed. The document still holds the old line — it is
    // written once, when the edit ends — so a box measured from the document
    // does not grow as the words do.
    _publishLive(layer);
    _throttle.request(() {
      try {
        final document = _typedDocument(_controller.text);
        if (document == null) return;
        widget.comp.renderFrameWithTextPreview(
          frame: BigInt.from(widget.uiState.playheadFrame.value),
          scale: widget.uiState.viewerScale,
          layer: layer,
          document: document,
        );
      } catch (_) {
        // A preview is a courtesy; the typing carries on without it.
      }
    });
  }

  /// End the edit: write the document once, or take the layer away if nothing
  /// was ever typed into a layer this tool made.
  void _finish() {
    final layer = _editing;
    if (layer == null) return;
    final text = _controller.text;
    // Read before the edit is closed, since it is the open edit's document.
    final document = _typedDocument(text);
    _throttle.cancel();
    _clearLive();
    setState(() {
      _editing = null;
      _held = null;
      _typed = '';
      _controller.clear();
    });
    _focus.unfocus();
    // Unable to take the keyboard again until the next edit opens, so nothing
    // can hand it back to a field with no edit under way.
    _focus.canRequestFocus = false;

    try {
      if (text.isEmpty) {
        // An empty line renders nothing, so a layer left empty by a stray click
        // would be an invisible row in the Timeline. One this tool made goes
        // away again; one the user already had keeps whatever it had.
        if (_created) {
          layer.delete();
          widget.onChanged();
        }
        return;
      }
      if (document != null) _write(layer, document);
      widget.onChanged();
    } catch (_) {
      // The layer was deleted while it was being typed into.
    }
  }

  /// Write what was typed, as **one** undo step.
  ///
  /// For a layer this tool made that means the document and the recentred
  /// anchor together: they are one action to the user — "I typed a line" — and
  /// committing them separately made the first `Ctrl+Z` undo a pivot the user
  /// had never moved, leaving the words exactly where they were and the undo
  /// looking broken.
  void _write(LayerReference layer, BridgeTextDocument document) {
    if (!_created) {
      layer.setText(document: document);
      return;
    }
    final placed = _recentredAnchor(layer, document);
    layer.setTextPlaced(
      document: document,
      anchorX: placed.anchor.dx,
      anchorY: placed.anchor.dy,
      positionX: placed.position.dx,
      positionY: placed.position.dy,
    );
  }

  /// The open edit's document saying [text]. Everything else is carried
  /// along, since the document is written whole: typing into a line that runs
  /// round a curve must not straighten it, or throw away its animators or its
  /// style. An expression is the one thing typing replaces, as it always has.
  BridgeTextDocument? _typedDocument(String text) =>
      _document?.copyWith(text: text, expression: '');

  /// Where a new layer's anchor and position want to be once the line is known:
  /// the pivot in the middle of the text, **without the line moving** — the
  /// pivot slides and Position compensates, the same pan-behind sum the Anchor
  /// point tool commits.
  ({Offset anchor, Offset position}) _recentredAnchor(
      LayerReference layer, BridgeTextDocument document) {
    final transform = layer.getTransform();
    final old = Offset(
      staticValueOf(transform.anchorX) ?? 0,
      staticValueOf(transform.anchorY) ?? 0,
    );
    final here = Offset(
      staticValueOf(transform.positionX) ?? 0,
      staticValueOf(transform.positionY) ?? 0,
    );
    final wanted = textAnchor(document);
    return (
      anchor: wanted,
      position: panBehindPosition(
        oldAnchor: old,
        newAnchor: wanted,
        position: here,
        scaleXPercent: staticValueOf(transform.scaleX) ?? 100,
        scaleYPercent: staticValueOf(transform.scaleY) ?? 100,
        rotationDegrees: staticValueOf(transform.rotation) ?? 0,
      ),
    );
  }
}

/// A transform channel's plain value, or null when it is keyframed and so has
/// no one value to read.
double? staticValueOf(BridgeScalar scalar) =>
    scalar is BridgeScalar_Static ? scalar.field0 : null;

/// The caret, or the selection, and nothing else: the text belongs to the
/// picture.
///
/// Both are drawn in the layer's own pixels and carried to the screen by its
/// map, so they turn and scale with the layer exactly as its words do.
class _SelectionPainter extends CustomPainter {
  /// The layer's map, or null when there is nothing to draw: no edit open, or
  /// a layer the Viewer has not boxed yet.
  final ViewerLayerMap? map;
  final BridgeTextBlock? block;

  /// The selection, in characters: equal for a caret.
  final int start;
  final int end;
  final Color accent;

  const _SelectionPainter({
    required this.map,
    required this.block,
    required this.start,
    required this.end,
    required this.accent,
  });

  @override
  void paint(Canvas canvas, Size canvasSize) {
    final map = this.map;
    final block = this.block;
    if (map == null || block == null) return;
    if (start == end) {
      final place = caretPlace(block, end);
      final baseline = block.lines[place.line].baseline;
      canvas.drawLine(
        map.toScreen(place.x, baseline - block.ascent),
        map.toScreen(place.x, baseline + block.descent),
        Paint()
          ..color = accent
          ..strokeWidth = 1.5,
      );
      return;
    }
    // One band per line the selection reaches.
    final paint = Paint()..color = accent.withValues(alpha: 0.35);
    for (final line in block.lines) {
      final last = line.start + line.carets.length - 1;
      if (end < line.start || start > last) continue;
      final x0 = line.carets[(start - line.start).clamp(0, last - line.start)];
      final x1 = line.carets[(end - line.start).clamp(0, last - line.start)];
      if (x0 == x1) continue;
      final top = line.baseline - block.ascent;
      final bottom = line.baseline + block.descent;
      canvas.drawPath(
        Path()
          ..addPolygon([
            map.toScreen(x0, top),
            map.toScreen(x1, top),
            map.toScreen(x1, bottom),
            map.toScreen(x0, bottom),
          ], true),
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(_SelectionPainter old) =>
      old.map != map ||
      old.block != block ||
      old.start != start ||
      old.end != end ||
      old.accent != accent;
}
