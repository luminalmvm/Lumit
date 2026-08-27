// The value well: the modifier ladder a scrub runs on, the chip that shows it,
// and the field itself.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../l10n/strings.dart';
import '../../theme/theme.dart';
import '../time_readout.dart' show monoSlotWidth;
import 'base.dart';
import 'menus.dart';
import 'popups.dart';
import 'value_arithmetic.dart';

/// The **modifier ladder** a value scrub runs on, coarsest first — the study's
/// four rungs (`Caddis study/notes-editor-ux.md` §3, docs/impl
/// /timeline-interaction.md polish 27), which are After Effects' own two with
/// a finer one under them: `Shift` ×10, nothing held ×1, `Ctrl` ×0.1, `Alt`
/// ×0.01.
///
/// Ordered, because the list is both the arithmetic and the chip's drawing:
/// [ScrubLadder] boxes whichever rung [scrubFactor] is answering.
const List<double> scrubLadder = [10, 1, 0.1, 0.01];

/// How much a scrub tick is worth right now, from the modifier keys — the
/// [scrubLadder]'s four rungs. Sampled inside the drag handler on every
/// update, so pressing or releasing a modifier mid-drag takes effect at once.
///
/// Coarse beats fine where two are held at once: `Shift` first, then `Alt`,
/// then `Ctrl`. A ladder needs one answer, and the one the hand meant is the
/// one it pressed on purpose — which, with two held, cannot be told apart, so
/// the order is fixed here rather than guessed.
double scrubFactor() => HardwareKeyboard.instance.isShiftPressed
    ? 10
    : HardwareKeyboard.instance.isAltPressed
        ? 0.01
        : HardwareKeyboard.instance.isControlPressed
            ? 0.1
            : 1;

/// The floating **sensitivity ladder** shown while a value scrub runs (polish
/// 27, study §3): all four rungs at once, the one in force boxed, so the
/// modifier that makes a drag fine is learned by using the field rather than
/// by reading the manual.
///
/// Transient and local (P1): [DragValueField] puts it up on the pointer's way
/// down and takes it down on release, and the resting panel keeps every pixel
/// it had.
class ScrubLadder extends StatelessWidget {
  /// What [scrubFactor] answers right now — which rung is boxed.
  final double factor;

  const ScrubLadder({super.key, required this.factor});

  /// A rung's label. Ordered as [scrubLadder] is.
  static List<String> get labels => [
        l10n.scrubLadderShift,
        l10n.scrubLadderBase,
        l10n.scrubLadderCtrl,
        l10n.scrubLadderAlt,
      ];

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // The hint pill's own ground and face (§4.2): every readout a gesture
    // summons in this application is 8px mono on `surface_4`.
    return IgnorePointer(
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 2, vertical: 2),
        decoration: BoxDecoration(
          color: t.surface4,
          borderRadius: BorderRadius.circular(2),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            // Coarsest on the right, finest on the left: the chip reads the
            // way the drag does, and the rungs sit in the order the study
            // draws them (ALT · CTRL · BASE · SHIFT).
            for (var i = scrubLadder.length - 1; i >= 0; i--)
              Container(
                margin: const EdgeInsets.symmetric(horizontal: 1),
                padding: const EdgeInsets.symmetric(horizontal: 3),
                decoration: BoxDecoration(
                  // The box is the whole mark: no fill, no colour beyond the
                  // one selection speaks in (P4).
                  border: Border.all(
                    color: factor == scrubLadder[i]
                        ? t.textPrimary
                        : const Color(0x00000000),
                  ),
                  borderRadius: BorderRadius.circular(2),
                ),
                child: Text(
                  labels[i],
                  style: t.mono.copyWith(
                    fontSize: 8,
                    color:
                        factor == scrubLadder[i] ? t.textPrimary : t.textMuted,
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// A value well's height in a panel: **20** (K-451, docs/15 §12A.6). Fixed
/// rather than grown from the number inside it, because the mockups' heights
/// are canonical and a well that measured its own font drifted with the face.
/// Dialog wells are 22 and set their own; they do not come through here yet.
const double wellHeight = 20;

/// The number inside a well: **11px mono**, the size the approved mockups
/// compute for every `.well` they draw (§7.1's mono row, K-454). It had been
/// 13, which is a size the mockups use nowhere and which crowded the well's
/// 20 from the inside.
const double wellTextSize = 11;

/// The number on a **bar**, where the drawing gives it no well: 10px mono, the
/// Viewer bottom bar's own `+0.0` (K-466). A bar reading is an aside beside the
/// picture, and it is set a size down from a panel's editable value.
const double barValueTextSize = 10;

/// The **value well** (docs/15-DESIGN.md §2.1/§3.1, K-439): drag horizontally
/// to adjust, click to type, right-click for Reset / Copy / Paste.
///
/// In plain terms, a number you can edit is drawn as a *recess* rather than as
/// a raised box — a `surface0` fill, darker than the panel around it, inside a
/// hairline. The well is what says "editable", so a resting panel keeps to its
/// three greys however many numbers it carries, and no colour has to be spent
/// saying it. The number itself is mono at [wellTextSize] (§7.1's absolute
/// rule, and its property-value row) and turns `accent` while it is actually
/// being dragged, `animated` when the property is keyed ([keyed]).
///
/// [resetTo] is the field's known default — Reset appears only when a call site
/// supplies one.
class DragValueField extends StatefulWidget {
  final num value;
  final num min;
  final num max;
  final double speed;
  final int decimals;
  final String? suffix;
  final num? resetTo;

  /// Whether a positive value is shown with its `+`.
  ///
  /// For a field whose zero is a *middle* rather than a floor — the Viewer's
  /// exposure in stops (K-314), which reads `+1.4` and `-2.3` — so the sign is
  /// part of the reading and the number does not appear to jump width when it
  /// crosses zero. Display only: what is typed, copied and pasted is the plain
  /// number, and `+1.4` parses as readily as `1.4`.
  final bool signed;

  /// The property this well edits has keyframes on it, so the number rests in
  /// `animated` rather than `text_primary` (§3.1). A live drag still wins: a
  /// value in hand is `accent` whether or not it is keyed.
  final bool keyed;

  /// The well's own fill, for the rare ground `surface0` cannot sit on. It is
  /// the inset every well now takes by default (§2.1), so a call site has no
  /// reason to pass anything.
  final Color? fill;

  /// Drawn **bare**: no inset, no hairline, the number alone at a bar's own
  /// 10px in `text_secondary` (K-466).
  ///
  /// One caller, and it is a measurement rather than a taste: the approved
  /// Viewer drawing sets the exposure as a plain `.mono` span on the bottom
  /// bar, with no background and no border, where every other editable number
  /// in the application rests in a well. A 20px well in a 22px bar would leave
  /// a pixel of ground above and below it and read as the bar's own edge.
  /// Everything else about the field is unchanged — the scrub, the modifier
  /// ladder, click-to-type, the context menu — and the drag and focus colours
  /// still speak, through the number rather than through an edge it has not
  /// got.
  final bool bare;
  final ValueChanged<num> onChanged;

  /// Fired once when a drag begins. Optional — a caller with nothing to do at
  /// drag-start (the common case) simply omits it.
  final VoidCallback? onChangeStart;

  /// Fired with the live value on every accumulated drag tick, in place of
  /// [onChanged], when supplied (a live-preview fast path — see
  /// [onChangeEnd]). Falls back to [onChanged] when null, so every existing
  /// call site behaves exactly as before.
  final ValueChanged<num>? onChangeLive;

  /// Fired once, with the final value, when a drag ends (mouse-up). Falls
  /// back to [onChanged] when null. Reset/Copy/Paste and the text-edit commit
  /// always call [onChanged] directly and never this — they are already
  /// one-shot edits, not a drag.
  final ValueChanged<num>? onChangeEnd;

  /// Fired when a drag is cancelled (a gesture cancel, or a released drag
  /// that never crossed one [speed] increment — so nothing was ever ticked).
  final VoidCallback? onDragCancel;

  const DragValueField({
    super.key,
    required this.value,
    required this.min,
    required this.max,
    required this.onChanged,
    this.speed = 1,
    this.decimals = 0,
    this.suffix,
    this.resetTo,
    this.signed = false,
    this.keyed = false,
    this.fill,
    this.bare = false,
    this.onChangeStart,
    this.onChangeLive,
    this.onChangeEnd,
    this.onDragCancel,
    this.setExpression,
  });

  /// Offered in the value's context menu when the property can take one.
  /// Absent (and the menu entry with it) for a field that cannot.
  final VoidCallback? setExpression;

  @override
  State<DragValueField> createState() => _DragValueFieldState();
}

class _DragValueFieldState extends State<DragValueField>
    implements TextSelectionGestureDetectorBuilderDelegate {
  bool _editing = false;
  bool _hover = false;
  bool _focused = false;

  /// A scrub is under the pointer right now — the one thing that turns the
  /// number `accent` (§3.1). Transient and local, like all feedback (§12A.5):
  /// it goes the moment the pointer lifts and leaves no trace behind.
  bool _dragging = false;
  double _dragAccum = 0;

  /// The last value ticked this drag (via [onChangeLive]/[onChanged]), or
  /// null before the first tick / after a commit or cancel. Distinguishes "a
  /// released drag that ticked at least once" (commit the last value) from "a
  /// released drag that never crossed one [DragValueField.speed] increment"
  /// (nothing to commit — a no-op cancel, which still opens the editor: the
  /// press was a click that wobbled, not a scrub).
  num? _lastDragValue;
  late TextEditingController _controller;
  late final FocusNode _focus = FocusNode(onKeyEvent: _onEditorKey);

  /// The floating [ScrubLadder], up only while a drag runs (polish 27).
  ///
  /// An overlay entry rather than a child of this field: the chip is bigger
  /// than the well it belongs to and every well in the application sits in a
  /// row that would either clip it or make room for it, and making room is a
  /// resting-state change (P1). Placed from the field's rect taken once at the
  /// down — a field does not move while it is being scrubbed — so a pointer
  /// move costs the overlay nothing.
  OverlayEntry? _ladder;

  /// Which rung is boxed, read afresh whenever a modifier goes down or up.
  /// A notifier rather than `setState`, so a modifier pressed mid-drag
  /// repaints the chip alone and not the panel behind it.
  final ValueNotifier<double> _factor = ValueNotifier<double>(1);

  /// Modifiers pressed and released **without the pointer moving** still change
  /// what the next pixel is worth, so the chip cannot wait for a drag update to
  /// find out. Never handles the key: it only looks.
  bool _ladderKey(KeyEvent event) {
    _factor.value = scrubFactor();
    return false;
  }

  void _showLadder() {
    final overlay = Overlay.maybeOf(context, rootOverlay: true);
    final box = context.findRenderObject();
    final overlayBox = overlay?.context.findRenderObject();
    // No overlay above this field (a bare widget-test host) simply means no
    // chip: the scrub itself is unaffected.
    if (overlay == null ||
        box is! RenderBox ||
        !box.hasSize ||
        overlayBox is! RenderBox) {
      return;
    }
    final top =
        box.localToGlobal(Offset(box.size.width / 2, 0), ancestor: overlayBox);
    final scope = ThemeScope.of(context);
    _factor.value = scrubFactor();
    _ladder = OverlayEntry(
      builder: (_) => Positioned(
        left: top.dx,
        top: top.dy - 4,
        // Centred over the field and sitting just above it, wherever that
        // leaves it: the chip is placed by its own bottom-centre, so a field
        // at the left edge of the window does not push it off screen.
        child: FractionalTranslation(
          translation: const Offset(-0.5, -1),
          // The overlay is above this field's own ThemeScope, so the chip is
          // handed the scope again on its way in.
          child: ThemeScope(
            theme: scope.theme,
            animationLevel: scope.animationLevel,
            showTooltips: scope.showTooltips,
            child: ValueListenableBuilder<double>(
              valueListenable: _factor,
              builder: (_, factor, __) => ScrubLadder(factor: factor),
            ),
          ),
        ),
      ),
    );
    overlay.insert(_ladder!);
    HardwareKeyboard.instance.addHandler(_ladderKey);
  }

  void _hideLadder() {
    HardwareKeyboard.instance.removeHandler(_ladderKey);
    _ladder?.remove();
    _ladder = null;
  }

  /// `Escape` in the open editor: shut it and keep the value the field had
  /// (K-323). Every other way out commits — Enter, Tab, clicking away — so
  /// without this a half-typed number had no way back.
  ///
  /// Clearing `_editing` first matters: the focus listener below commits on
  /// focus loss, and closing the editor is what loses it.
  KeyEventResult _onEditorKey(FocusNode node, KeyEvent event) {
    if (event is KeyDownEvent &&
        event.logicalKey == LogicalKeyboardKey.escape &&
        _editing) {
      setState(() => _editing = false);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  /// The idle box's focus — how Tab reaches the field, and what `Enter`
  /// opens the editor from (K-319).
  final ControlFocusNode _idleFocus = ControlFocusNode(debugLabel: 'value');

  /// The open editor, for the selection gestures: pressing in it puts the
  /// caret down and dragging highlights, the way any text box works.
  final GlobalKey<EditableTextState> textFieldKey = GlobalKey();

  @override
  GlobalKey<EditableTextState> get editableTextKey => textFieldKey;

  @override
  bool get forcePressEnabled => false;

  @override
  bool get selectionEnabled => true;

  @override
  void initState() {
    super.initState();
    _controller = TextEditingController();
    _focus.addListener(() {
      if (!_focus.hasFocus && _editing) _commitText();
    });
  }

  @override
  void dispose() {
    // The chip lives in the Overlay rather than under this field, so a field
    // disposed mid-drag would leave it on screen over whatever came next.
    _hideLadder();
    _factor.dispose();
    _controller.dispose();
    _focus.dispose();
    _idleFocus.dispose();
    super.dispose();
  }

  /// Open the text editor with the whole value selected — a value box is
  /// retyped far more often than it is amended, and a selected value means
  /// the first keystroke replaces it (K-319).
  void _beginEdit() {
    setState(() {
      _editing = true;
      final text = widget.decimals == 0
          ? widget.value.round().toString()
          : widget.value.toDouble().toStringAsFixed(widget.decimals);
      _controller.text = text;
      _controller.selection =
          TextSelection(baseOffset: 0, extentOffset: text.length);
    });
    _focus.requestFocus();
  }

  /// The face both states are set in: the resting reading and the open
  /// editor, so neither can drift from the other.
  TextStyle _valueStyle(LumitTheme t) =>
      t.mono.copyWith(fontSize: widget.bare ? barValueTextSize : wellTextSize);

  /// How wide the resting face draws — the reading's own width plus the well's
  /// padding and its edge.
  ///
  /// The reading is monospaced, so a character count is a width; [monoSlotWidth]
  /// is the same measurement the readouts use, and caches by face and length.
  double _restingWidth(LumitTheme t) =>
      monoSlotWidth(_valueStyle(t), _format(widget.value).length) +
      (widget.bare ? 0 : 12) +
      2;

  String _format(num v) {
    var s = _plain(v);
    // `toStringAsFixed` already carries a minus; only the plus has to be put
    // back, and only where the reading is signed.
    if (widget.signed && !s.startsWith('-')) s = '+$s';
    return widget.suffix == null ? s : '$s${widget.suffix}';
  }

  void _commitText() {
    final raw = _controller.text.replaceAll(widget.suffix ?? '', '').trim();
    final parsed = parseNumberField(raw);
    if (parsed != null) {
      widget.onChanged(parsed.clamp(widget.min, widget.max));
    }
    setState(() => _editing = false);
  }

  /// The plain numeric string (no suffix) — what Copy puts on the clipboard and
  /// what Paste parses back, so a value round-trips between fields.
  String _plain(num v) => widget.decimals == 0
      ? v.round().toString()
      : v.toDouble().toStringAsFixed(widget.decimals);

  /// The egui drag-value right-click menu: Reset (when a default is known),
  /// Copy and Paste, over the system clipboard with the field's own clamp.
  void _contextMenu(BuildContext context, Offset globalPos) {
    showLumitPopup<void>(
      context: context,
      position: globalPos,
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (widget.resetTo != null)
                MenuRow(
                  onPressed: () {
                    close(null);
                    widget.onChanged(
                        widget.resetTo!.clamp(widget.min, widget.max));
                  },
                  child: Text(l10n.reset),
                ),
              MenuRow(
                onPressed: () {
                  close(null);
                  Clipboard.setData(ClipboardData(text: _plain(widget.value)));
                },
                child: Text(l10n.menuCopy),
              ),
              MenuRow(
                onPressed: () async {
                  close(null);
                  final data = await Clipboard.getData(Clipboard.kTextPlain);
                  final raw =
                      data?.text?.replaceAll(widget.suffix ?? '', '').trim();
                  final parsed = raw == null ? null : parseNumberField(raw);
                  if (parsed != null) {
                    widget.onChanged(parsed.clamp(widget.min, widget.max));
                  }
                },
                child: Text(l10n.menuPaste),
              ),
              // Only where the property can actually hold one, so the menu on
              // a field that cannot never offers it.
              if (widget.setExpression != null)
                MenuRow(
                  onPressed: () {
                    close(null);
                    widget.setExpression?.call();
                  },
                  child: Text(l10n.setExpression),
                ),
            ],
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    if (_editing) {
      // **The editor is the resting face, with a caret in it.** Same box, same
      // padding, same border, same type, same right-hand anchor — because
      // anything else moves the number under the pointer that just clicked it,
      // which the owner read as jarring and was right to. It used to be a
      // fixed 72-wide box with the text against its *left* edge, so clicking
      // a well both resized the box and threw the digits across it.
      return SizedBox(
        width: _restingWidth(t),
        height: widget.bare ? null : wellHeight,
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: widget.bare ? null : widget.fill ?? t.surface0,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            // `animated`, not `accent`: the focused value field is the one
            // focus that means "you are about to change a value" (§3.1). Drawn
            // at the resting face's own width so the edge does not move either.
            border: Border.all(color: t.animated, width: 1),
          ),
          // The selection gestures, so a press puts the caret down and a drag
          // highlights — without this the editor took keys but a drag over the
          // text selected nothing (K-319).
          child: TextSelectionGestureDetectorBuilder(delegate: this)
              .buildGestureDetector(
            child: Padding(
              // The resting face's 6 of padding **plus its 1px edge**: a
              // `Container`'s decoration insets its child by the border it
              // draws and a `DecoratedBox` does not, so the 7 here is what
              // puts the two readings on exactly the same pixel.
              padding: const EdgeInsets.symmetric(horizontal: 7),
              // **Centred down the well, exactly as the resting reading is.**
              // The well hands its child a tight height, and an `EditableText`
              // given one lays its line out at the *top* of it — so the digits
              // rose four and a half pixels the moment the editor opened, on
              // top of not moving sideways. The `SizedBox` keeps the full
              // width, which is what the right-hand anchor needs; the `Center`
              // gives the line its natural height and puts it down the middle.
              child: Center(
                child: SizedBox(
                  width: double.infinity,
                  child: EditableText(
                    key: textFieldKey,
                    controller: _controller,
                    focusNode: _focus,
                    // Mono while focused too — the number must not change
                    // width between reading it and typing over it (§7.1) —
                    // the same size as the resting number, so nothing reflows
                    // on the click.
                    style: _valueStyle(t).copyWith(color: t.textPrimary),
                    // The resting face is right-anchored, so the editor is
                    // too: the digits stay where they were even though the
                    // reading loses its sign or its unit on the way into the
                    // field.
                    textAlign: TextAlign.right,
                    cursorColor: t.accent,
                    backgroundCursorColor: t.surface2,
                    selectionColor: t.accent.withValues(alpha: 0.5),
                    selectionControls: desktopTextSelectionHandleControls,
                    onSubmitted: (_) => _commitText(),
                  ),
                ),
              ),
            ),
          ),
        ),
      );
    }
    return FocusableActionDetector(
      focusNode: _idleFocus,
      // Enter only, no Space: this is a number box, and `Enter` opening the
      // editor is what Tab-and-type needs (K-319).
      shortcuts: const {
        SingleActivator(LogicalKeyboardKey.enter, includeRepeats: false):
            ActivateIntent(),
        SingleActivator(LogicalKeyboardKey.numpadEnter, includeRepeats: false):
            ActivateIntent(),
      },
      actions: <Type, Action<Intent>>{
        ActivateIntent: CallbackAction<ActivateIntent>(onInvoke: (_) {
          _beginEdit();
          return null;
        }),
      },
      onFocusChange: (has) {
        setState(() => _focused = has);
        // **Tab arrives ready to type** (§12A.3, K-529). The only way this box
        // takes focus is keyboard traversal — a click opens the editor
        // directly — and a value well reached by Tab is one about to be
        // retyped, so it opens its editor at once. `_beginEdit` is the call
        // that already selects the whole value, which is the half the owner
        // read as missing: the hop worked, the first keystroke appended.
        if (has && !_editing) _beginEdit();
      },
      mouseCursor: SystemMouseCursors.resizeLeftRight,
      onShowHoverHighlight: (over) => setState(() => _hover = over),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: _beginEdit,
        onSecondaryTapDown: (d) => _contextMenu(context, d.globalPosition),
        onHorizontalDragStart: (_) {
          _dragAccum = 0;
          _lastDragValue = null;
          setState(() => _dragging = true);
          _showLadder();
          widget.onChangeStart?.call();
        },
        onHorizontalDragUpdate: (d) {
          final factor = scrubFactor();
          _factor.value = factor;
          _dragAccum += d.delta.dx * widget.speed * factor;
          if (_dragAccum.abs() >= widget.speed * factor) {
            // The drag runs from its own last tick, not from `widget.value`:
            // pointer events arrive faster than rebuilds, and a base read
            // from the stale prop dropped every chunk but the frame's last —
            // a fast drag lost most of its travel.
            final next = ((_lastDragValue ?? widget.value) + _dragAccum)
                .clamp(widget.min, widget.max);
            _dragAccum = 0;
            _lastDragValue = next;
            (widget.onChangeLive ?? widget.onChanged)(next);
          }
        },
        onHorizontalDragEnd: (_) {
          final v = _lastDragValue;
          _lastDragValue = null;
          _hideLadder();
          setState(() => _dragging = false);
          if (v != null) {
            (widget.onChangeEnd ?? widget.onChanged)(v);
          } else {
            // Never crossed one speed-increment: nothing was ticked, so the
            // press was a click that wobbled a few pixels, not a scrub. It
            // cancels as a drag — and then does what the click meant, which
            // is open the editor (K-319). Before this, a click that moved
            // at all did nothing, and value boxes felt like they swallowed
            // clicks.
            widget.onDragCancel?.call();
            _beginEdit();
          }
        },
        onHorizontalDragCancel: () {
          _lastDragValue = null;
          _hideLadder();
          setState(() => _dragging = false);
          widget.onDragCancel?.call();
        },
        child: Container(
          height: widget.bare ? null : wellHeight,
          padding: widget.bare
              ? EdgeInsets.zero
              : const EdgeInsets.symmetric(horizontal: 6),
          decoration: BoxDecoration(
            // The inset stays the inset in every state: a well does not lift
            // under the pointer, because then it would stop being a recess
            // (§2.1). Hover and scrub speak through the edge instead.
            color: widget.bare ? null : widget.fill ?? t.surface0,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(
                color: widget.bare
                    ? const Color(0x00000000)
                    : _dragging
                        ? t.accent
                        // The one focus ring that is `animated` rather than
                        // `accent`: it means "you are about to change a value"
                        // (§3.1, §6.5).
                        : _focused
                            ? t.animated
                            : _hover
                                ? t.hairlineStrong
                                : t.hairline,
                width: 1),
          ),
          child: Align(
            alignment: Alignment.centerRight,
            widthFactor: 1,
            child: Text(
              _format(widget.value),
              textAlign: TextAlign.right,
              style: _valueStyle(t).copyWith(
                color: _dragging
                    ? t.accent
                    : widget.keyed
                        ? t.animated
                        // A bare number has no well to say "editable", so it
                        // rests where the drawing puts it — a bar's own
                        // secondary reading rather than the well's primary.
                        : widget.bare
                            ? (_hover || _focused
                                ? t.textPrimary
                                : t.textSecondary)
                            : t.textPrimary,
              ),
            ),
          ),
        ),
      ),
    );
  }
}
