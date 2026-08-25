// The Type tool: making and editing text layers on the picture (K-225,
// docs/07 §1.7, §2.3.2).
//
// **In plain terms.** With the Type tool in hand, clicking empty picture makes a
// **new text layer** where you clicked and puts a caret there; clicking an
// existing text layer edits *that* one. What you type appears in the picture as
// you type it, and the edit ends when you press `Escape`, press `Enter`, click
// somewhere else, or put the tool down. A new layer you never typed anything
// into is removed again — After Effects does the same, and a project full of
// empty text layers left by stray clicks is nobody's idea of a feature.
//
// **Why the document is written only once.** Every edit to the document is an
// undo step, so writing the layer on each keystroke would make `Ctrl+Z` walk
// back through a sentence one letter at a time. Instead the picture is kept in
// step with `render_frame_with_text_preview` — the same live-preview path a
// dragged transform uses (K-183), which shows a provisional value without the
// document ever holding it — and the layer is written once, when the edit ends.
// One typing session, one undo step.
//
// **Where the caret comes from.** The typing itself is a real Flutter text
// field, so arrows, selection, backspace, paste and IME all behave as they do
// everywhere else — but its *drawing* is turned off, because the text the user
// should see is the engine's own rendering of the layer. What is drawn here is
// the caret, placed by the same rough estimate of a line's width the engine
// uses to anchor a text layer (half the point size per character). It is an
// estimate, and it is the same estimate on both sides, which is what keeps the
// caret and the picture from disagreeing about where the line ends.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';

import '../l10n/strings.dart';
import '../state/layer_bounds.dart' show estimatedTextWidth;
import '../state/preview_throttle.dart';
import '../widgets/controls.dart';
import 'viewer_gizmo.dart';
import 'viewer_shape_layer.dart' show ShapeSpace;
import 'viewer_tool_cursor.dart';
import 'viewer_layer_map.dart';

/// The anchor a text layer of this text wants: the middle of its estimated
/// bounds, so it scales and turns about itself rather than about its first
/// letter. The engine picks the same point when it makes a text layer.
Offset textAnchor(String text, double size) =>
    Offset(estimatedTextWidth(text, size) * 0.5, size * 0.5);

/// The Type tool over the picture.
class ViewerTypeLayer extends StatefulWidget {
  /// Whether a type tool is armed. Inert otherwise.
  final bool active;

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

  /// Where the line starts on screen — the point clicked for a new layer, or the
  /// layer's own origin for an existing one. The caret is measured from it.
  Offset _origin = Offset.zero;

  /// The point size and fill the edit is using, from the toolbar's options.
  double _size = 72;
  BridgeColourRgba _fill = const BridgeColourRgba(r: 1, g: 1, b: 1, a: 1);

  /// Where the pointer is, for the drawn beam vertical type wears (K-226).
  Offset? _pointer;

  final TextEditingController _controller = TextEditingController();
  final FocusNode _focus = FocusNode(debugLabel: 'Type tool');
  final PreviewThrottle _throttle = PreviewThrottle();

  @override
  void initState() {
    super.initState();
    _controller.addListener(_onTyped);
    HardwareKeyboard.instance.addHandler(_onKey);
  }

  /// The two keys a typing session has to answer while the text field has the
  /// keyboard (K-230).
  ///
  /// **Escape** ends the edit, which the tool always promised and never did.
  /// **Ctrl+Z** ends it as well and then lets go: an undo pressed mid-sentence
  /// used to be swallowed by the text field, so the document did not move and
  /// the application looked as though undo had stopped working. Ending the edit
  /// first is what makes the next `Ctrl+Z` undo the thing the user means — the
  /// line they just typed, and after that the layer itself.
  bool _onKey(KeyEvent event) {
    if (!_editingNow || event is! KeyDownEvent) return false;
    if (event.logicalKey == LogicalKeyboardKey.escape) {
      _finish();
      return true;
    }
    final undo = event.logicalKey == LogicalKeyboardKey.keyZ &&
        (HardwareKeyboard.instance.isControlPressed ||
            HardwareKeyboard.instance.isMetaPressed);
    if (!undo) return false;
    // Written, then handed on: the shell's own undo takes it from here, so
    // there is one undo path in the application rather than two.
    _finish();
    return false;
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
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) _finish();
      });
    }
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_onKey);
    _clearLive();
    _throttle.cancel();
    _controller.dispose();
    _focus.dispose();
    super.dispose();
  }

  bool get _editingNow => _editing != null;

  @override
  Widget build(BuildContext context) {
    if (!widget.active) return const SizedBox.shrink();
    final viewScale = widget.fitted.width / widget.compSize.width;
    // Horizontal type wears the system's own I-beam; vertical type has one
    // drawn for it, because no platform ships a sideways beam (K-226).
    final vertical = widget.tool == ToolMode.typeVertical;
    final t = ThemeScope.of(context).theme;
    return Positioned.fill(
      child: DrawnPointerRegion(
        cursor: vertical ? SystemMouseCursors.none : SystemMouseCursors.text,
        onPointer: (at) => setState(() => _pointer = at),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapUp: _onTapUp,
          child: Stack(
            children: [
              if (vertical)
                TextPointer(
                  at: _pointer,
                  mark: t.textPrimary,
                  outline: t.surface0,
                ),
              if (_editingNow)
                Positioned(
                  left: _origin.dx,
                  top: _origin.dy - _size * viewScale,
                  width: 1,
                  height: 1,
                  // The field itself never shows: the text a user should see is
                  // the engine's rendering of the layer, and a second copy of
                  // it in a different font on top of that would only disagree.
                  // What it is here for is the keyboard — arrows, backspace,
                  // selection, paste and IME, all of it for free. Invisible
                  // rather than *offstage*, because an offstage field is not
                  // built, and one that is not built takes no keystrokes.
                  child: Opacity(
                    opacity: 0,
                    child: EditableText(
                      controller: _controller,
                      focusNode: _focus,
                      style: TextStyle(fontSize: _size * viewScale),
                      cursorColor: widget.accent,
                      backgroundCursorColor: widget.accent,
                      onSubmitted: (_) => _finish(),
                    ),
                  ),
                ),
              Positioned.fill(
                child: IgnorePointer(
                  child: CustomPaint(
                    painter: _CaretPainter(
                      show: _editingNow,
                      origin: _origin,
                      before: _controller.selection.isValid
                          ? _controller.text.substring(
                              0,
                              _controller.selection.baseOffset
                                  .clamp(0, _controller.text.length),
                            )
                          : _controller.text,
                      size: _size,
                      viewScale: viewScale,
                      accent: widget.accent,
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  // --- The gesture ----------------------------------------------------------

  void _onTapUp(TapUpDetails details) {
    if (widget.tool == ToolMode.typeVertical) {
      widget.state.postNotice(
        l10n.typeVerticalNotBuilt,
      );
      return;
    }
    // Whatever was being typed is finished first: a click elsewhere is what
    // ends an edit, exactly as it does in After Effects.
    _finish();

    final existing = _textLayerAt(details.localPosition);
    if (existing != null) {
      _begin(existing.layer, existing.box.map.toScreen(0, existing.box.map.ay),
          created: false);
      return;
    }
    _create(details.localPosition);
  }

  /// The topmost text layer whose box contains [at], or null.
  ///
  /// Which layers are text comes off the read model (K-184), so a click costs
  /// no bridge calls however deep the stack — it used to ask `getText()` of
  /// every layer under the pointer. The one read the edit needs is asked of
  /// the layer chosen, in [_begin].
  ({LayerBox box, LayerReference layer})? _textLayerAt(Offset at) {
    final textIds = {
      for (final entry in widget.uiState.model.heldLayers)
        if (entry.info.kind == BridgeLayerKind.text) entry.layer.internallayerId,
    };
    for (final box in widget.boxes) {
      if (box.contains(at) && textIds.contains(box.id)) {
        return (box: box, layer: box.layer);
      }
    }
    return null;
  }

  /// Make a text layer where the pointer is, and start typing into it.
  void _create(Offset at) {
    final options = widget.uiState.tools;
    // The composition's own placement — the same conversion the shape tools
    // build a new layer's art with (K-237).
    final (cx, cy) =
        ShapeSpace.ofComp(fitted: widget.fitted, compSize: widget.compSize)
            .ofScreen(at);
    try {
      // One op, so one undo step, and undoing it takes the layer away (K-230).
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
        ),
        x: cx,
        y: cy,
      );
      widget.uiState.setSelection([layer]);
      _begin(layer, at, created: true);
      widget.onChanged();
    } catch (_) {
      widget.state.postNotice(l10n.couldNotAddTextLayer, error: true);
    }
  }

  void _begin(LayerReference layer, Offset origin, {required bool created}) {
    final document = () {
      try {
        return layer.getText();
      } catch (_) {
        return null;
      }
    }();
    if (document == null) return;
    setState(() {
      _editing = layer;
      _created = created;
      _origin = origin;
      _size = document.size;
      _fill = document.fill;
      _controller.text = document.text;
      _controller.selection =
          TextSelection.collapsed(offset: document.text.length);
    });
    _focus.requestFocus();
    _publishLive(layer);
  }

  /// Tell the Viewer's boxes what is being typed, and stop telling it when the
  /// edit ends — the document is the only truth from then on.
  void _publishLive(LayerReference layer) {
    widget.uiState.liveText.value = {
      layer.internallayerId: (text: _controller.text, size: _size),
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
    final layer = _editing!;
    // What the box round the words should be measured from while they are
    // being typed (K-232). The document still holds the old line — it is
    // written once, when the edit ends — so a box measured from the document
    // does not grow as the words do.
    _publishLive(layer);
    _throttle.request(() {
      try {
        widget.comp.renderFrameWithTextPreview(
          frame: BigInt.from(widget.uiState.playheadFrame.value),
          scale: widget.uiState.viewerScale,
          layer: layer,
          document: BridgeTextDocument(
            text: _controller.text,
            size: _size,
            fill: _fill,
          ),
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
    _throttle.cancel();
    _clearLive();
    setState(() {
      _editing = null;
      _controller.clear();
    });
    _focus.unfocus();

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
      _write(layer, text);
      widget.onChanged();
    } catch (_) {
      // The layer was deleted while it was being typed into.
    }
  }

  /// Write what was typed, as **one** undo step (K-230).
  ///
  /// For a layer this tool made that means the document and the recentred
  /// anchor together: they are one action to the user — "I typed a line" — and
  /// committing them separately made the first `Ctrl+Z` undo a pivot the user
  /// had never moved, leaving the words exactly where they were and the undo
  /// looking broken.
  void _write(LayerReference layer, String text) {
    final document = BridgeTextDocument(text: text, size: _size, fill: _fill);
    if (!_created) {
      layer.setText(document: document);
      return;
    }
    final placed = _recentredAnchor(layer, text);
    layer.setTextPlaced(
      document: document,
      anchorX: placed.anchor.dx,
      anchorY: placed.anchor.dy,
      positionX: placed.position.dx,
      positionY: placed.position.dy,
    );
  }

  /// Where a new layer's anchor and position want to be once the line is known:
  /// the pivot in the middle of the text, **without the line moving** — the
  /// pivot slides and Position compensates, the same pan-behind sum the Anchor
  /// point tool commits (K-220).
  ({Offset anchor, Offset position}) _recentredAnchor(
      LayerReference layer, String text) {
    final transform = layer.getTransform();
    final old = Offset(
      staticValueOf(transform.anchorX) ?? 0,
      staticValueOf(transform.anchorY) ?? 0,
    );
    final here = Offset(
      staticValueOf(transform.positionX) ?? 0,
      staticValueOf(transform.positionY) ?? 0,
    );
    final wanted = textAnchor(text, _size);
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

/// The caret, and nothing else: the text belongs to the picture.
class _CaretPainter extends CustomPainter {
  final bool show;
  final Offset origin;
  final String before;
  final double size;
  final double viewScale;
  final Color accent;

  const _CaretPainter({
    required this.show,
    required this.origin,
    required this.before,
    required this.size,
    required this.viewScale,
    required this.accent,
  });

  @override
  void paint(Canvas canvas, Size canvasSize) {
    if (!show) return;
    final x = origin.dx + estimatedTextWidth(before, size) * viewScale;
    final height = size * viewScale;
    canvas.drawRect(
      Rect.fromLTWH(x, origin.dy - height, 1.5, height),
      Paint()..color = accent,
    );
  }

  @override
  bool shouldRepaint(_CaretPainter old) =>
      old.show != show ||
      old.origin != origin ||
      old.before != before ||
      old.size != size ||
      old.viewScale != viewScale ||
      old.accent != accent;
}
