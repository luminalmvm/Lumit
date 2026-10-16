// The expression editor: the well an expression is written in.
//
// A property row has room for a few characters of code, which is not enough
// to write in. So the row only shows the expression, and pressing it opens
// this over the row: several lines, wide enough to read, with the number the
// expression works out underneath. The Timeline shows the same text in the
// row's lane and opens the same editor across it.
//
// Clicking away keeps what was typed, as it does for every other value.
// Escape puts back what was there.

import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/widgets/autofill.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../state/expression_language.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart' show ExpressionTextEditingController;
import 'graph_channels.dart';
import 'graph_edits.dart';

/// The narrowest the editor opens, however small the thing it opens over.
const double _minWidth = 420;

/// Open the editor over [over], a box in window coordinates.
///
/// [onLive] is handed each change as it is typed, for a row that can preview
/// one. [onCommit] is handed what the property ends up holding: the new
/// expression, or a plain number when the expression was taken off.
/// [fresh] is an expression that was only just put there, holding the number
/// the property had, so it opens selected and the first key typed replaces it.
/// [held] is what the property holds, when that is not [value].
Future<void> showExpressionEditor({
  required BuildContext context,
  required Rect over,
  required BridgeScalar_Expression value,
  required CompositionReference comp,
  required LayerReference layer,
  void Function(BridgeScalar_Expression draft)? onLive,
  required void Function(BridgeScalar next) onCommit,
  bool fresh = false,
  BridgeScalar? held,
}) async {
  final frame =
      Provider.of<LumitUiState>(context, listen: false).playheadFrame.value;
  final time = timeOfFrame(comp, frame);
  var draft = value;
  var cancelled = false;
  var removed = false;
  await showLumitPopup<void>(
    context: context,
    position: over.topLeft,
    builder: (close) => _ExpressionEditor(
      width: math.max(over.width, _minWidth),
      value: value,
      fresh: fresh,
      sample: (scalar) => sampleScalarWithContext(
          scalar: scalar, time: time, layer: layer, held: held ?? value),
      fault: (scalar) => expressionFault(
          expression: scalar.field0,
          language: scalar.field1,
          time: time,
          layer: layer,
          held: held ?? value),
      onDraft: (next) {
        draft = next;
        onLive?.call(next);
      },
      onDone: () => close(null),
      onCancel: () {
        cancelled = true;
        close(null);
      },
      onRemove: () {
        removed = true;
        close(null);
      },
    ),
  );
  if (removed) {
    // The number the expression was showing, so the picture does not move.
    onCommit(BridgeScalar.static_(sampleScalarWithContext(
        scalar: draft, time: time, layer: layer, held: held ?? value)));
  } else if (draft != value) {
    // A cancelled edit that was previewed has to be written back over it.
    if (!cancelled) {
      onCommit(draft);
    } else if (onLive != null) {
      onCommit(value);
    }
  }
}

class _ExpressionEditor extends StatefulWidget {
  final double width;
  final BridgeScalar_Expression value;
  final bool fresh;
  final double Function(BridgeScalar_Expression scalar) sample;

  /// Why the expression gives no number, or null when it does.
  final String? Function(BridgeScalar_Expression scalar) fault;
  final ValueChanged<BridgeScalar_Expression> onDraft;
  final VoidCallback onDone;
  final VoidCallback onCancel;
  final VoidCallback onRemove;

  const _ExpressionEditor({
    required this.width,
    required this.value,
    required this.fresh,
    required this.sample,
    required this.fault,
    required this.onDraft,
    required this.onDone,
    required this.onCancel,
    required this.onRemove,
  });

  @override
  State<_ExpressionEditor> createState() => _ExpressionEditorState();
}

class _ExpressionEditorState extends State<_ExpressionEditor> {
  late final ExpressionTextEditingController _text =
      ExpressionTextEditingController(
          text: widget.value.field0,
          language: expressionGrammar(widget.value.field1));
  final FocusNode _focus = FocusNode();
  late BridgeScalar_Expression _draft = widget.value;

  @override
  void initState() {
    super.initState();
    if (widget.fresh) {
      _text.selection =
          TextSelection(baseOffset: 0, extentOffset: _text.text.length);
    }
    _text.addListener(_changed);
  }

  @override
  void dispose() {
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  /// The controller also speaks when only the caret moved, which is no change.
  void _changed() => _write(BridgeScalar_Expression(_text.text, _draft.field1));

  void _write(BridgeScalar_Expression next) {
    if (next == _draft) return;
    setState(() => _draft = next);
    widget.onDraft(next);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    _text.language = expressionGrammar(_draft.field1);
    // The well grows with what is written, between a few lines and a page.
    final lines = '\n'.allMatches(_text.text).length + 1;
    final lineHeight = (t.mono.fontSize ?? 11) * (t.mono.height ?? 1.4);
    final fault = widget.fault(_draft);
    return FloatSurface(
      width: widget.width,
      // Ctrl+Enter is done, since Enter alone is a new line.
      child: Focus(
        canRequestFocus: false,
        skipTraversal: true,
        onKeyEvent: (_, event) {
          final keys = HardwareKeyboard.instance;
          if (event is KeyDownEvent &&
              event.logicalKey == LogicalKeyboardKey.enter &&
              (keys.isControlPressed || keys.isMetaPressed)) {
            widget.onDone();
            return KeyEventResult.handled;
          }
          return KeyEventResult.ignored;
        },
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SizedBox(
              height: lines.clamp(4, 18) * lineHeight + 8,
              child: HouseTextField(
                key: const ValueKey('expression-editor-text'),
                controller: _text,
                focusNode: _focus,
                width: double.infinity,
                autofocus: true,
                multiline: true,
                indentOnTab: true,
                topAlign: true,
                style: t.mono,
                hint: l10n.expressionHint,
                autofill: ExpressionAutofillGenerator(),
                onCancelled: widget.onCancel,
              ),
            ),
            Padding(
              padding: const EdgeInsets.only(top: 6),
              child: Row(
                children: [
                  // What it comes to, or why it comes to nothing. The engine
                  // words the reason, and it names the line.
                  Expanded(
                    child: fault != null
                        ? Text(
                            fault,
                            key: const ValueKey('expression-editor-fault'),
                            style: t.small.copyWith(color: t.error),
                            maxLines: 2,
                            overflow: TextOverflow.ellipsis,
                          )
                        : Text(
                            '= ${widget.sample(_draft).toStringAsPrecision(6)}',
                            key: const ValueKey('expression-editor-value'),
                            style: t.mono.copyWith(color: t.textMuted),
                            softWrap: false,
                            overflow: TextOverflow.ellipsis,
                          ),
                  ),
                  HouseButton(
                    key: const ValueKey('expression-editor-remove'),
                    small: true,
                    frameless: true,
                    onPressed: widget.onRemove,
                    child: Text(l10n.removeExpression, style: t.small),
                  ),
                  const SizedBox(width: 6),
                  SizedBox(
                    width: 110,
                    height: 22,
                    child: ExpressionLanguagePicker(
                      key: const ValueKey('expression-editor-language'),
                      value: _draft.field1,
                      onChanged: (language) {
                        _write(BridgeScalar_Expression(_text.text, language));
                        // Back to the text, so Escape still reaches it.
                        _focus.requestFocus();
                      },
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// An expression drawn on one line, in its language's colours.
InlineSpan expressionLine(
        BuildContext context, BridgeScalar_Expression value) =>
    ExpressionTextEditingController.spanOf(
      context,
      value.field0.replaceAll(RegExp(r'\s*\n\s*'), ' '),
      expressionGrammar(value.field1),
    );

/// The lane of a property row an expression drives.
///
/// Such a row has no keys to draw, so its lane shows the expression instead,
/// and pressing it opens the editor across the lane. A row of two axes shows
/// each axis's own.
class ExpressionLane extends StatelessWidget {
  /// The row's channels that hold an expression.
  final List<GraphChannel> channels;

  /// Whether each is named, which a row of more than one axis needs.
  final bool named;
  final CompositionReference comp;

  /// The lanes' sideways scroll, so the text stays in view as they move.
  final ScrollController? hScroll;
  final bool locked;
  final VoidCallback onChanged;

  const ExpressionLane({
    super.key,
    required this.channels,
    required this.named,
    required this.comp,
    required this.hScroll,
    required this.locked,
    required this.onChanged,
  });

  Widget _cell(BuildContext context, GraphChannel channel) {
    final t = ThemeScope.of(context).theme;
    final value = channel.scalar as BridgeScalar_Expression;
    return Expanded(
      child: HoverMark(
        cursor: SystemMouseCursors.text,
        builder: (context, lit) => GestureDetector(
          key: ValueKey<String>('tl-expression-${channel.id}'),
          behavior: HitTestBehavior.opaque,
          onTap: locked
              ? null
              : () {
                  final box = context.findRenderObject() as RenderBox;
                  showExpressionEditor(
                    context: context,
                    over: box.localToGlobal(Offset.zero) & box.size,
                    value: value,
                    comp: comp,
                    layer: channel.entry.layer,
                    onCommit: (next) {
                      commitChannelEdits({channel: next});
                      onChanged();
                    },
                  );
                },
          child: Container(
            alignment: Alignment.centerLeft,
            padding: const EdgeInsets.symmetric(horizontal: 8),
            child: Opacity(
              opacity: 0.7 + 0.3 * lit,
              child: Text.rich(
                TextSpan(children: [
                  // The axis letter, which a channel's label ends in.
                  if (named)
                    TextSpan(
                      text: '${channel.label.split(' ').last}  ',
                      style: t.small.copyWith(color: t.textMuted),
                    ),
                  expressionLine(context, value),
                ]),
                maxLines: 1,
                softWrap: false,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final row = Row(
      children: [for (final channel in channels) _cell(context, channel)],
    );
    final scroll = hScroll;
    if (scroll == null) return row;
    // The lane is as long as the composition and slides under the panel, so
    // the text is held to the part of it on screen.
    return AnimatedBuilder(
      animation: scroll,
      child: row,
      builder: (context, child) {
        final position = scroll.hasClients ? scroll.positions.first : null;
        final laidOut = position != null &&
            position.hasPixels &&
            position.hasViewportDimension;
        return Align(
          alignment: Alignment.centerLeft,
          child: Padding(
            padding: EdgeInsets.only(left: laidOut ? position.pixels : 0),
            child: SizedBox(
              width: laidOut ? position.viewportDimension : null,
              child: child,
            ),
          ),
        );
      },
    );
  }
}
