// The Ctrl+Space console (K-324, reshaped by K-325): a radial menu around the
// pointer, with a search bar floating beside it.
//
// **In plain terms.** Two ways of reaching the same kinds of thing, in one
// gesture, because they suit different moments.
//
// The console opens **where the mouse is**: the ring of choices is centred on
// the pointer, so the flick that picks a slice can start the instant the key
// goes down — no travel to a window first. The **search bar** floats just
// above the ring (or below it, when the pointer is near the top of the
// window). It starts empty and shows nothing: the ring is the offer. Start
// typing and the ring steps aside for a dropdown of matches under the bar —
// type "gau", press Enter, and Gaussian blur is on every selected layer.
//
// Effects come first in that dropdown and compositions after a divider,
// because the overwhelmingly common thing to want is an effect; comps are
// there so the same bar can also be "take me to that comp".
//
// The search half is modelled on Video Copilot's FX Console — including its
// **snapshot** button, which writes the frame on screen to a PNG so two
// versions of a look can be compared without setting up an export.
//
// The **radial menu** is for when you do not want to type at all. Its entries
// are chosen by what is selected, and every entry sits at a fixed angle, so
// the hand learns the direction and stops reading. A slice can carry a ring
// of its own: choosing it expands the menu in place (Blender's nested pies),
// and the centre — or Escape — steps back out. See `widgets/radial_maths.dart`
// for why direction rather than hit-testing decides the choice.
//
// Nothing here is a boxed window: the console floats translucent over the
// work, because what it acts on is what you should keep seeing.
//
// **The console applies things; it does not know how.** Every entry carries a
// callback the caller supplied, exactly as the command palette does (docs/07
// §12), so this file holds no idea about the document and cannot drift out of
// step with what the menus do.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/radial_maths.dart';

/// Where the pointer last was, in global coordinates — recorded by the shell
/// on every hover, move and press, because the keyboard event that opens the
/// console carries no position and the ring is centred on the mouse (K-325).
/// Null until the pointer has ever been seen, which falls back to centre.
Offset? lastKnownPointerPosition;

/// What kind of thing a search row is — what the divider separates, and what
/// the row's badge says.
enum FxConsoleKind {
  /// A built-in effect, applied to the selection.
  effect,

  /// A composition, fronted in the Timeline.
  composition,
}

/// One row the console's search can find.
class FxConsoleEntry {
  final String label;
  final FxConsoleKind kind;

  /// The group shown beside the label — an effect's category, or nothing.
  final String? group;
  final VoidCallback run;

  const FxConsoleEntry({
    required this.label,
    required this.kind,
    required this.run,
    this.group,
  });
}

/// One slice of the radial menu.
class RadialEntry {
  final String label;

  /// What choosing the slice does — or null for a slice that only opens
  /// [children].
  final VoidCallback? run;

  /// Drawn dimmed and unpickable — an action that belongs in this context but
  /// cannot run right now, so the ring keeps its shape and the direction a
  /// hand has learned still means the same thing.
  final bool enabled;

  /// A ring of its own (K-325): choosing this slice expands the menu in place
  /// rather than running anything, the way Blender nests its pies. The centre
  /// of the ring, or Escape, steps back out.
  final List<RadialEntry> children;

  const RadialEntry({
    required this.label,
    this.run,
    this.enabled = true,
    this.children = const [],
  }) : assert(run != null || children.length > 0,
            'a slice either runs or expands');
}

/// Everything the console shows, gathered by the caller from the live
/// selection — see `menu_bar_frb.dart`, which builds it beside the menus.
class FxConsoleModel {
  final List<FxConsoleEntry> entries;

  /// The radial slices for the current selection, in ring order from straight
  /// up, clockwise. Empty hides the ring entirely rather than drawing a
  /// circle with nothing in it.
  final List<RadialEntry> radial;

  /// What the radial menu is about right now ("Timeline", "Gaussian blur") —
  /// drawn in the middle of the ring so the context is never a guess.
  final String radialTitle;

  /// Save the frame on screen as a PNG. Null where there is nothing to save
  /// (no composition open), which greys the button.
  final VoidCallback? onSnapshot;

  const FxConsoleModel({
    required this.entries,
    required this.radial,
    required this.radialTitle,
    this.onSnapshot,
  });
}

/// Show the console over the work, centred on [anchor] (global coordinates —
/// normally [lastKnownPointerPosition]). Null anchors fall back to the middle
/// of the window, which is where a pointer nobody has moved yet would be
/// guessed to be.
///
/// Its own overlay entry rather than `showLumitModal`: the console is not a
/// boxed window but a ring floating where the pointer is, with no dimmed
/// backdrop — the work stays visible because the work is what the console
/// acts on (K-325).
Future<void> showFxConsoleFrb({
  required BuildContext context,
  required FxConsoleModel model,
  Offset? anchor,
}) {
  final overlay = Overlay.of(context);
  final completer = Completer<void>();
  late OverlayEntry entry;
  void close() {
    if (completer.isCompleted) return;
    completer.complete();
    entry.remove();
  }

  entry = OverlayEntry(
    builder: (_) => _FxConsole(model: model, anchor: anchor, onClose: close),
  );
  overlay.insert(entry);
  return completer.future;
}

/// How well `needle` matches `haystack` as a subsequence, or null for no
/// match. Lower is better. Shared shape with the command palette's ranking
/// (docs/07 §12): earlier and tighter wins, so the thing half-remembered comes
/// to the top rather than a coincidence further down.
int? fxConsoleScore(String needle, String haystack) {
  if (needle.isEmpty) return 0;
  final lower = haystack.toLowerCase();
  var at = 0;
  var first = -1;
  var last = 0;
  for (final rune in needle.toLowerCase().runes) {
    final found = lower.indexOf(String.fromCharCode(rune), at);
    if (found < 0) return null;
    if (first < 0) first = found;
    last = found;
    at = found + 1;
  }
  return (last - first) + first;
}

/// The matching entries, effects first and compositions after — the order the
/// divider in the list stands for. Ranked within each kind, never across it:
/// a comp is never allowed to outrank an effect, because the reason to open
/// this window is nearly always an effect.
List<FxConsoleEntry> fxConsoleMatches(
    List<FxConsoleEntry> entries, String query) {
  final needle = query.trim();
  final scored = <(int, int, int, FxConsoleEntry)>[];
  for (var i = 0; i < entries.length; i++) {
    final entry = entries[i];
    final score = fxConsoleScore(needle, entry.label);
    if (score == null) continue;
    // Kind first, then relevance, then the declared order so the ranking is
    // stable for equal scores rather than depending on the sort.
    scored.add((entry.kind.index, score, i, entry));
  }
  scored.sort((a, b) {
    final byKind = a.$1.compareTo(b.$1);
    if (byKind != 0) return byKind;
    final byScore = a.$2.compareTo(b.$2);
    return byScore != 0 ? byScore : a.$3.compareTo(b.$3);
  });
  return [for (final entry in scored) entry.$4];
}

class _FxConsole extends StatefulWidget {
  final FxConsoleModel model;
  final Offset? anchor;
  final VoidCallback onClose;
  const _FxConsole({
    required this.model,
    required this.anchor,
    required this.onClose,
  });

  @override
  State<_FxConsole> createState() => _FxConsoleState();
}

/// The bar's fixed footprint, shared by the layout maths and the widgets.
const double _barWidth = 356;
const double _barHeight = 44;

class _FxConsoleState extends State<_FxConsole> {
  final TextEditingController _query = TextEditingController();

  /// The search field's focus, held here because the console steers it: the
  /// field is focused on open and *kept* focused for the console's whole life
  /// (K-328), so anything typed while the console is up lands in the box.
  final FocusNode _queryFocus = FocusNode(debugLabel: 'fx-console-query');
  int _highlighted = 0;

  /// Which radial slice the pointer is choosing, or null in the dead zone.
  int? _radialHover;

  /// The rings entered so far: the model's own, then one more per expanded
  /// slice. The last is what is drawn; leaving a sub-ring pops it.
  late final List<({String title, List<RadialEntry> entries})> _rings = [
    (title: widget.model.radialTitle, entries: widget.model.radial),
  ];

  @override
  void initState() {
    super.initState();
    _query.addListener(() => setState(() => _highlighted = 0));
    // Escape has to work from anywhere — over the ring, mid-flick, wherever
    // focus happens to sit. A handler on the search field's node covers only
    // the field, so this listens at the keyboard itself for the console's
    // lifetime, the same reason the shell's own shortcuts are global. It is
    // the only place Escape is handled, so one press is one step back.
    HardwareKeyboard.instance.addHandler(_escapeAnywhere);
    // While the console is up, the keyboard is the console's (K-328): the
    // panels' hardware-keyboard commands stand down exactly as they do for a
    // dialogue, so a keystroke meant for the search box can never rename a
    // layer underneath.
    markModalMounted();
    // And the search box owns typing outright: focused now — deterministic,
    // where `autofocus` lost a race against the shell's own scope — and
    // re-taken the moment anything else grabs focus, for as long as the
    // console is open. The only ways out are Escape and a click outside,
    // both of which close the whole console.
    _queryFocus.addListener(_keepFocus);
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _queryFocus.requestFocus();
    });
  }

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_escapeAnywhere);
    markModalUnmounted();
    _queryFocus
      ..removeListener(_keepFocus)
      ..dispose();
    _query.dispose();
    super.dispose();
  }

  void _keepFocus() {
    if (mounted && !_queryFocus.hasFocus) _queryFocus.requestFocus();
  }

  bool _escapeAnywhere(KeyEvent event) {
    if (event is! KeyDownEvent ||
        event.logicalKey != LogicalKeyboardKey.escape) {
      return false;
    }
    _back();
    return true;
  }

  bool get _typing => _query.text.trim().isNotEmpty;

  List<FxConsoleEntry> get _matches =>
      fxConsoleMatches(widget.model.entries, _query.text);

  void _runHighlighted(List<FxConsoleEntry> matches) {
    if (!_typing) {
      // Enter on an empty bar: nothing chosen, nothing to run — the key that
      // usually means "done" closes the console rather than sitting inert.
      widget.onClose();
      return;
    }
    if (matches.isEmpty) return;
    final entry = matches[_highlighted.clamp(0, matches.length - 1)];
    widget.onClose();
    entry.run();
  }

  void _runSlice(int index) {
    final entries = _rings.last.entries;
    if (index < 0 || index >= entries.length) return;
    final entry = entries[index];
    if (!entry.enabled) return;
    if (entry.children.isNotEmpty) {
      // The slice is a ring of its own: expand in place rather than run.
      setState(() {
        _rings.add((title: entry.label, entries: entry.children));
        _radialHover = null;
      });
      return;
    }
    widget.onClose();
    entry.run!();
  }

  /// One step out: typed text clears first, then a sub-ring pops, then the
  /// console closes — so Escape always retreats by exactly one decision.
  void _back() {
    if (_typing) {
      _query.clear();
    } else if (_rings.length > 1) {
      setState(() {
        _rings.removeLast();
        _radialHover = null;
      });
    } else {
      widget.onClose();
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final matches = _matches;

    return Focus(
      onKeyEvent: (node, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        switch (event.logicalKey) {
          case LogicalKeyboardKey.arrowDown:
            setState(() => _highlighted =
                matches.isEmpty ? 0 : (_highlighted + 1) % matches.length);
            return KeyEventResult.handled;
          case LogicalKeyboardKey.arrowUp:
            setState(() => _highlighted = matches.isEmpty
                ? 0
                : (_highlighted - 1 + matches.length) % matches.length);
            return KeyEventResult.handled;
          case LogicalKeyboardKey.enter:
          case LogicalKeyboardKey.numpadEnter:
            _runHighlighted(matches);
            return KeyEventResult.handled;
          // Escape is deliberately absent: [_escapeAnywhere] owns it, so it
          // works with focus anywhere and never fires twice.
          default:
            return KeyEventResult.ignored;
        }
      },
      // LayoutBuilder, not MediaQuery: what matters is the room the overlay
      // actually has (the showLumitPopup lesson).
      child: LayoutBuilder(
        builder: (context, box) {
          final anchor = widget.anchor ??
              Offset(box.maxWidth / 2, box.maxHeight / 2);
          final at = fxConsoleLayout(
            screenWidth: box.maxWidth,
            screenHeight: box.maxHeight,
            anchorX: anchor.dx,
            anchorY: anchor.dy,
            barWidth: _barWidth,
            barHeight: _barHeight,
          );
          // **Every child is keyed, and that is load-bearing** (K-328). The
          // ring comes and goes with the query, so without keys the bar
          // shifts index the moment a letter is typed — and Flutter matches
          // unkeyed children by index and type, both of these being
          // `Positioned`. The bar's element was recycled onto the ring's old
          // slot, the field beneath it rebuilt from nothing, and a fresh
          // `EditableText` whose focus node is *already* focused never opens
          // a text-input connection: typing stopped dead after one letter.
          // Keys make the match by identity, so the field survives untouched.
          return Stack(
            children: [
              // A hush, not a blackout: the modal scrim at half its strength,
              // so the slices stay legible over any frame while the work stays
              // part of the picture. It also catches the click that means
              // "never mind".
              Positioned.fill(
                key: const ValueKey('fx-console-scrim'),
                child: GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onTap: widget.onClose,
                  onSecondaryTap: widget.onClose,
                  child: ColoredBox(
                    color: t.scrim.withValues(alpha: t.scrim.a * 0.5),
                  ),
                ),
              ),
              // The ring steps aside while the user is typing: the dropdown
              // needs the space, and starting to type is choosing the other
              // way in.
              if (!_typing && _rings.last.entries.isNotEmpty)
                Positioned(
                  key: const ValueKey('fx-console-ring'),
                  left: at.centreX - radialExtent,
                  top: at.centreY - radialExtent,
                  width: radialExtent * 2,
                  height: radialExtent * 2,
                  child: _ring(t),
                ),
              Positioned(
                key: const ValueKey('fx-console-bar'),
                left: at.barLeft,
                top: at.barTop,
                width: _barWidth,
                height: _barHeight,
                child: _searchBar(t, matches),
              ),
              if (_typing)
                Positioned(
                  key: const ValueKey('fx-console-dropdown'),
                  left: at.barLeft,
                  top: at.barTop + _barHeight + 4,
                  width: _barWidth,
                  child: _dropdown(t, matches, box.maxHeight, at.barTop),
                ),
            ],
          );
        },
      ),
    );
  }

  /// The translucent float every part of the console sits on: the standard
  /// menu surface let through a little (K-325), so the work underneath stays
  /// part of the picture. Derived from the theme — never its own colour.
  BoxDecoration _float(LumitTheme t, {double radius = 0}) => BoxDecoration(
        color: t.surface3.withValues(alpha: 0.88),
        borderRadius: BorderRadius.circular(
            radius == 0 ? t.tokens.floatRadius : radius),
        border: Border.all(color: t.hairline, width: 1),
        boxShadow: t.floatShadow,
      );

  Widget _searchBar(LumitTheme t, List<FxConsoleEntry> matches) => Container(
        decoration: _float(t),
        padding: const EdgeInsets.symmetric(horizontal: 8),
        child: Row(
          children: [
            Expanded(
              child: HouseTextField(
                key: const ValueKey('fx-console-query'),
                controller: _query,
                width: 280,
                focusNode: _queryFocus,
                hint: l10n.fxConsoleHint,
                onSubmitted: (_) => _runHighlighted(matches),
              ),
            ),
            const SizedBox(width: 6),
            // The snapshot button, in the corner FX Console puts it: one press
            // writes the frame on screen to a PNG, so two versions of a look
            // can be compared without setting an export up.
            LumitTooltip(
              message: l10n.fxConsoleSnapshotTip,
              child: HouseButton(
                key: const ValueKey('fx-console-snapshot'),
                small: true,
                onPressed: widget.model.onSnapshot == null
                    ? null
                    : () {
                        widget.onClose();
                        widget.model.onSnapshot!();
                      },
                child: lumitIcon(LumitIcon.snapshot,
                    size: iconSize, color: t.textSecondary),
              ),
            ),
          ],
        ),
      );

  /// The matches, under the bar, only while there is a query to match: an
  /// empty bar offers the ring, not a directory (K-325).
  Widget _dropdown(
    LumitTheme t,
    List<FxConsoleEntry> matches,
    double screenHeight,
    double barTop,
  ) {
    if (matches.isEmpty) return const SizedBox.shrink();
    // Capped at the room below the bar, scrolling inside that.
    final room = screenHeight - barTop - _barHeight - 12;
    return Container(
      decoration: _float(t),
      padding: const EdgeInsets.all(4),
      child: ConstrainedBox(
        constraints:
            BoxConstraints(maxHeight: room.clamp(48.0, 260.0)),
        child: ListView.builder(
          shrinkWrap: true,
          itemCount: matches.length,
          itemBuilder: (context, i) {
            final entry = matches[i];
            // The divider between the effects and everything below them: drawn
            // where the kind changes, so it is right however the list is
            // filtered rather than at a fixed row.
            final startsSection = i > 0 && matches[i - 1].kind != entry.kind;
            final row = MenuRow(
              key: ValueKey<String>('fx-console-item-${entry.label}'),
              selected: i == _highlighted,
              onPressed: () {
                widget.onClose();
                entry.run();
              },
              child: Row(
                children: [
                  Expanded(child: Text(entry.label)),
                  if (entry.group != null)
                    Text(entry.group!,
                        style: t.small.copyWith(color: t.textMuted)),
                ],
              ),
            );
            if (!startsSection) return row;
            return Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Padding(
                  padding: const EdgeInsets.fromLTRB(10, 6, 10, 4),
                  child: Row(
                    children: [
                      Expanded(child: Container(height: 1, color: t.hairline)),
                      const SizedBox(width: 6),
                      Text(_sectionLabel(entry.kind),
                          style: t.small.copyWith(color: t.textMuted)),
                    ],
                  ),
                ),
                row,
              ],
            );
          },
        ),
      ),
    );
  }

  String _sectionLabel(FxConsoleKind kind) => switch (kind) {
        FxConsoleKind.effect => l10n.fxConsoleEffects,
        FxConsoleKind.composition => l10n.fxConsoleCompositions,
      };

  /// The ring. A press anywhere in it chooses by direction — see
  /// `radial_maths.dart` — and releasing runs what is chosen, so the whole
  /// menu is one flick. Clicking a label works too, for a hand that would
  /// rather aim than flick. A slice with children expands in place; the
  /// centre steps back out.
  Widget _ring(LumitTheme t) {
    final ring = _rings.last;
    const centre = Offset(radialExtent, radialExtent);
    void track(Offset local) {
      final at = local - centre;
      final slice = radialSliceAt(at.dx, at.dy, ring.entries.length);
      if (slice != _radialHover) setState(() => _radialHover = slice);
    }

    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onPanStart: (d) => track(d.localPosition),
      onPanUpdate: (d) => track(d.localPosition),
      onPanEnd: (_) {
        final slice = _radialHover;
        setState(() => _radialHover = null);
        if (slice != null) _runSlice(slice);
      },
      child: MouseRegion(
        onHover: (e) => track(e.localPosition),
        onExit: (_) => setState(() => _radialHover = null),
        child: Stack(
          children: [
            Positioned.fill(child: Center(child: _centre(t))),
            for (var i = 0; i < ring.entries.length; i++)
              _slice(t, i, ring.entries[i], centre),
          ],
        ),
      ),
    );
  }

  /// The middle of the ring: the context's name on a translucent disc — and,
  /// inside a sub-ring, the way back out (K-325), because the hand that
  /// expanded a slice is already there.
  Widget _centre(LumitTheme t) {
    final inSubRing = _rings.length > 1;
    final disc = Container(
      key: const ValueKey('fx-radial-centre'),
      width: radialDeadZone * 2 + 8,
      height: radialDeadZone * 2 + 8,
      alignment: Alignment.center,
      padding: const EdgeInsets.all(4),
      decoration: BoxDecoration(
        color: t.surface2.withValues(alpha: 0.8),
        shape: BoxShape.circle,
        border: Border.all(color: t.hairline, width: 1),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (inSubRing)
            CustomPaint(
              size: const Size(7, 9),
              painter: _RadialCaret(t.textMuted, pointRight: false),
            ),
          if (inSubRing) const SizedBox(width: 3),
          Flexible(
            child: Text(
              _rings.last.title,
              textAlign: TextAlign.center,
              style: t.small.copyWith(color: t.textMuted),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
            ),
          ),
        ],
      ),
    );
    if (!inSubRing) return disc;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: _back,
      child: disc,
    );
  }

  Widget _slice(LumitTheme t, int index, RadialEntry entry, Offset centre) {
    final at = radialSliceOffset(index, _rings.last.entries.length);
    final chosen = _radialHover == index && entry.enabled;
    const width = 108.0;
    const height = 26.0;
    return Positioned(
      left: centre.dx + at.dx - width / 2,
      top: centre.dy + at.dy - height / 2,
      width: width,
      height: height,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: entry.enabled ? () => _runSlice(index) : null,
        child: Container(
          key: ValueKey<String>('fx-radial-${entry.label}'),
          alignment: Alignment.center,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          decoration: BoxDecoration(
            // Translucent like the bar, so the ring sits over the work
            // rather than blotting it out; the chosen slice goes solid
            // accent, which is what "about to happen" looks like.
            color: chosen ? t.accent : t.surface3.withValues(alpha: 0.88),
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(
                color: chosen ? t.accent : t.hairline, width: 1),
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Flexible(
                child: Text(
                  entry.label,
                  textAlign: TextAlign.center,
                  overflow: TextOverflow.ellipsis,
                  maxLines: 1,
                  style: t.small.copyWith(
                    color: !entry.enabled
                        ? t.textDisabled
                        : chosen
                            ? t.surface0
                            : t.textPrimary,
                  ),
                ),
              ),
              // A slice that expands says so, the way a menu row with a
              // flyout does.
              if (entry.children.isNotEmpty) ...[
                const SizedBox(width: 4),
                CustomPaint(
                  size: const Size(5, 7),
                  painter: _RadialCaret(
                    !entry.enabled
                        ? t.textDisabled
                        : chosen
                            ? t.surface0
                            : t.textMuted,
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

/// The little triangle a slice with children carries — pointing right on the
/// slice ("more this way"), left in the centre ("back out").
class _RadialCaret extends CustomPainter {
  final Color colour;
  final bool pointRight;
  const _RadialCaret(this.colour, {this.pointRight = true});

  @override
  void paint(Canvas canvas, Size size) {
    final path = pointRight
        ? (Path()
          ..moveTo(0, 0)
          ..lineTo(size.width, size.height / 2)
          ..lineTo(0, size.height)
          ..close())
        : (Path()
          ..moveTo(size.width, 0)
          ..lineTo(0, size.height / 2)
          ..lineTo(size.width, size.height)
          ..close());
    canvas.drawPath(path, Paint()..color = colour);
  }

  @override
  bool shouldRepaint(_RadialCaret old) =>
      old.colour != colour || old.pointRight != pointRight;
}
