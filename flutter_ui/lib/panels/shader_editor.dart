// The shader editor: the window `Edit shader…` opens on a Custom shader
// (docs/impl/custom-shader.md §3.2, CS3).
//
// **In plain terms.** A Custom shader holds a small program the user wrote, and
// this is where they write it. A monospaced box with numbered lines, a line
// under it saying whether the program compiles — and if it does not, the
// compiler's own sentence with the line number moved onto the text they are
// looking at — and two buttons: put it on the effect, or leave it alone.
//
// **Nothing here decides anything.** The text is compiled by the engine
// (`shader_status`), the rows it declares are grown by the engine, and Apply is
// one `setShaderSource` staged on the handle and committed with the stack —
// which makes an edit one `SetLayerEffects` and one undo step, the shape every
// other stack edit has. The window is a place to type.
//
// **Typing does not compile.** Apply compiles, and a short pause in the typing
// asks the engine what it thinks of the text so far — the answer is cached by
// source hash on the engine side, so asking twice about the same text costs
// nothing, and the question is never asked from a rebuild.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter/services.dart';
import 'package:uuid/uuid.dart';

import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../l10n/strings.dart';
import '../shell/dialog_frame.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

/// The editor's width and the height of the code well inside it — the dialog
/// pattern's frame (K-444) at the one size a page of code wants.
const double shaderEditorWidth = 640;
const double shaderEditorWellHeight = 320;

/// Put [source] on one effect of [layer]'s stack and commit the stack.
///
/// The one write both ways of getting text onto a Custom shader go through —
/// the editor's Apply and `Load from file…` — so an edit is one
/// `SetLayerEffects` and one undo step whichever gesture made it. [origin] is
/// the file the text was read from, or null for text somebody typed.
///
/// Answers whether it landed: false when the effect has gone from the stack
/// under the window, where re-reading is the recovery and half a write would be
/// worse than none.
bool applyShaderSource({
  required LayerReference layer,
  required UuidValue effect,
  required String source,
  String? origin,
}) {
  final stack = layer.getEffects();
  for (final instance in stack) {
    if (instance.id() != effect) continue;
    instance.setShaderSource(source: source, origin: origin);
    try {
      layer.setEffects(effects: stack);
    } catch (_) {
      // The stack changed under us; re-reading is the recovery.
      return false;
    }
    return true;
  }
  return false;
}

/// What the engine makes of [source] on this effect, without committing it.
///
/// The source is staged on a handle and never handed to `setEffects`, so the
/// document does not move: this is a question, not an edit. Null when the
/// effect has gone.
BridgeShaderStatus? shaderStatusFor({
  required LayerReference layer,
  required UuidValue effect,
  required String source,
}) {
  for (final instance in layer.getEffects()) {
    if (instance.id() != effect) continue;
    instance.setShaderSource(source: source, origin: null);
    return instance.shaderStatus();
  }
  return null;
}

/// Open the editor on one Custom shader instance, and commit what it returns.
///
/// Answers whether anything was applied, so the caller can refresh the read
/// model on the edit and not on a cancel.
Future<bool> showShaderEditor({
  required BuildContext context,
  required LayerReference layer,
  required UuidValue effect,
}) async {
  String? held;
  for (final instance in layer.getEffects()) {
    if (instance.id() != effect) continue;
    held = instance.shaderSource() ?? '';
    break;
  }
  if (held == null) return false;
  final source = held;

  final applied = await showLumitModal<String>(
    context: context,
    id: 'shader-editor',
    builder: (close) => _ShaderEditor(
      source: source,
      status: (text) =>
          shaderStatusFor(layer: layer, effect: effect, source: text),
      onApply: close,
      onCancel: () => close(null),
    ),
  );
  if (applied == null) return false;
  return applyShaderSource(layer: layer, effect: effect, source: applied);
}

class _ShaderEditor extends StatefulWidget {
  final String source;
  final BridgeShaderStatus? Function(String) status;
  final ValueChanged<String> onApply;
  final VoidCallback onCancel;

  const _ShaderEditor({
    required this.source,
    required this.status,
    required this.onApply,
    required this.onCancel,
  });

  @override
  State<_ShaderEditor> createState() => _ShaderEditorState();
}

class _ShaderEditorState extends State<_ShaderEditor> {
  late final TextEditingController _text =
      TextEditingController(text: widget.source);
  final ScrollController _scroll = ScrollController();

  /// The engine's last answer about what is in the box.
  BridgeShaderStatus? _status;

  /// The pause in the typing that asks for the next one. A keystroke is not a
  /// question: ten of them in a second ask once, when the tenth has settled.
  Timer? _settle;

  @override
  void initState() {
    super.initState();
    _status = widget.status(_text.text);
    _text.addListener(_typed);
  }

  @override
  void dispose() {
    _settle?.cancel();
    _text.removeListener(_typed);
    _text.dispose();
    _scroll.dispose();
    super.dispose();
  }

  /// ponytail: the preview compile runs on the UI thread, on the debounce —
  /// naga's answer is cached by source hash, so it is one parse per distinct
  /// text and milliseconds at that. The trigger for moving it to the worker job
  /// §3.2 specifies is a felt stutter *while typing* in this window; nothing
  /// else in the app waits on it.
  void _typed() {
    // The line numbers follow the text as it is typed; the compiler's opinion
    // waits for a pause.
    setState(() {});
    _settle?.cancel();
    _settle = Timer(const Duration(milliseconds: 400), () {
      if (!mounted) return;
      setState(() => _status = widget.status(_text.text));
    });
  }

  void _apply() => widget.onApply(_text.text);

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Focus(
      // Ctrl+Enter applies from wherever the caret is — the chord a code box
      // needs, because Enter itself belongs to the text (K-319: the window's
      // default action, reachable without leaving the keyboard).
      onKeyEvent: (_, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        final enter = event.logicalKey == LogicalKeyboardKey.enter ||
            event.logicalKey == LogicalKeyboardKey.numpadEnter;
        if (!enter || !HardwareKeyboard.instance.isControlPressed) {
          return KeyEventResult.ignored;
        }
        _apply();
        return KeyEventResult.handled;
      },
      child: DialogFrame(
        width: shaderEditorWidth,
        children: [
          dialogTitleBar(
            t,
            title: l10n.shaderEditorTitle,
            onClose: widget.onCancel,
            keyPrefix: 'shader-editor',
          ),
          Padding(
            padding: const EdgeInsets.all(dialogPadding),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              mainAxisSize: MainAxisSize.min,
              children: [
                SizedBox(
                  height: shaderEditorWellHeight,
                  child: _CodeWell(text: _text, scroll: _scroll),
                ),
                const SizedBox(height: 8),
                _message(t),
              ],
            ),
          ),
          dialogFooter(
            t,
            keyPrefix: 'shader-editor',
            actions: [
              HouseButton(
                key: const ValueKey<String>('shader-editor-cancel'),
                onPressed: widget.onCancel,
                child: Text(l10n.cancel),
              ),
              HouseButton(
                key: const ValueKey<String>('shader-editor-apply'),
                primary: true,
                padding: const EdgeInsets.symmetric(horizontal: 16),
                onPressed: _apply,
                child: Text(l10n.apply),
              ),
            ],
          ),
        ],
      ),
    );
  }

  /// What the compiler makes of the text: its own sentence when it refuses,
  /// one line per annotation it could not read, and a quiet confirmation when
  /// it is happy. An empty box says nothing — a shader nobody has written yet
  /// is not a broken one (K-111).
  ///
  /// Never red and never an alarm: the composition is still compositing, the
  /// effect is drawing the last program that worked, and this is the sentence
  /// that says so (§3.2).
  Widget _message(LumitTheme t) {
    final status = _status;
    final error = status?.error;
    final notes = status?.notes ?? const <String>[];
    final blank = _text.text.trim().isEmpty;
    return Column(
      key: const ValueKey<String>('shader-editor-message'),
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        if (error != null)
          Text(error, style: t.small.copyWith(color: t.accent))
        else if (!blank)
          Text(l10n.shaderCompiles, style: t.small.copyWith(color: t.textMuted)),
        for (final note in notes)
          Text(note, style: t.small.copyWith(color: t.textMuted)),
      ],
    );
  }
}

/// The code well: a monospaced page with its lines numbered down the left.
///
/// The gutter is not a second scrolling box — two scroll views sharing one
/// position is a Flutter error waiting to happen — it is the numbers drawn
/// once and slid by however far the text has scrolled, which is the whole of
/// what following it means. Both take the same text style, so line 40 in the
/// gutter is beside line 40 in the text.
class _CodeWell extends StatelessWidget {
  final TextEditingController text;
  final ScrollController scroll;

  const _CodeWell({required this.text, required this.scroll});

  /// The gutter's width: room for four digits at the mono size, which is more
  /// lines than a shader the grammar can carry will ever have.
  static const double gutterWidth = 30;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // An explicit line height on both, because "the numbers line up with the
    // lines" is the only thing the gutter has to be right about.
    final code = t.mono.copyWith(color: t.textPrimary, height: 1.4);
    final numbers = code.copyWith(color: t.textMuted);
    final lines = '\n'.allMatches(text.text).length + 1;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SizedBox(
          width: gutterWidth,
          child: ClipRect(
            child: AnimatedBuilder(
              animation: scroll,
              builder: (context, _) => OverflowBox(
                alignment: Alignment.topRight,
                maxHeight: double.infinity,
                child: Transform.translate(
                  offset: Offset(
                      0, -(scroll.hasClients ? scroll.offset : 0.0)),
                  child: Padding(
                    // The 3 above matches the well's own inset, so line one
                    // starts level with the caret; the 6 to the right is the
                    // air between a number and its line.
                    padding: const EdgeInsets.only(top: 3, right: 6),
                    child: Text(
                      [for (var i = 1; i <= lines; i++) '$i'].join('\n'),
                      textAlign: TextAlign.right,
                      style: numbers,
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
        Expanded(
          child: HouseTextField(
            key: const ValueKey<String>('shader-editor-code'),
            controller: text,
            multiline: true,
            scrollController: scroll,
            autofocus: true,
            style: code,
            // The well takes the width the row gives it, not a number of its
            // own: a page of code is as wide as the window is.
            width: double.infinity,
          ),
        ),
      ],
    );
  }
}
