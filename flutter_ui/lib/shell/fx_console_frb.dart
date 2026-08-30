// The Ctrl+Space console (K-324; popover face K-658):
// one search popover floating where the pointer is.
//
// **In plain terms.** One box, four bands, top to bottom: a search row (the
// magnifier, the query, and a kicker naming the key that opened it), a strip
// of category kickers to browse by, the matching rows — each with its name,
// its category and a small preview swatch — and one quiet sentence saying
// what choosing a row will do. The radial ring the console used to raise, and
// the hush it drew over the work, are gone by the owner's ruling: the list is
// the offer, open from the first frame, and typing narrows it.
//
// The console opens **where the mouse is**, so the eye never travels: type
// "gau", press Enter, and Gaussian blur is on every selected layer (K-523).
// The search half is modelled on Video Copilot's FX Console — including its
// **snapshot** button, which writes the frame on screen to a PNG so two
// versions of a look can be compared without setting up an export.
//
// The graph canvas's Tab opens this same surface (K-645): what a caller
// contributes is the list, the kicker naming its key, and the foot sentence.
//
// **The console applies things; it does not know how.** Every entry carries a
// callback the caller supplied, exactly as the command palette does (docs/07
// §12), so this file holds no idea about the document and cannot drift out of
// step with what the menus do.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';

/// Where the pointer last was, in global coordinates — recorded by the shell
/// on every hover, move and press, because the keyboard event that opens the
/// console carries no position and the popover opens on the mouse (K-325).
/// Null until the pointer has ever been seen, which falls back to centre.
Offset? lastKnownPointerPosition;

/// What kind of thing a search row is — what the ranking keeps apart.
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
  /// It is also what the category strip filters by.
  final String? group;
  final VoidCallback run;

  const FxConsoleEntry({
    required this.label,
    required this.kind,
    required this.run,
    this.group,
  });
}

/// Everything the console shows, gathered by the caller from the live
/// selection — see `menu_bar_frb.dart`, which builds it beside the menus.
class FxConsoleModel {
  final List<FxConsoleEntry> entries;

  /// Save the frame on screen as a PNG. Null where there is nothing to save
  /// (no composition open), which greys the button.
  final VoidCallback? onSnapshot;

  /// The key that opened this — drawn as a kicker at the search row's right
  /// end, the way the drawing has it ("Ctrl+Space" from the shell, "Tab" from
  /// the graph canvas). Null draws nothing.
  final String? keyHint;

  /// One quiet sentence under the list saying what choosing a row will do
  /// ("Enter applies to the selected layers", "Adds a driver node"). Null
  /// draws no foot at all.
  final String? footer;

  const FxConsoleModel({
    required this.entries,
    this.onSnapshot,
    this.keyHint,
    this.footer,
  });
}

/// Show the console over the work, its search row on [anchor] (global
/// coordinates — normally [lastKnownPointerPosition]). Null anchors fall back
/// to the middle of the window, which is where a pointer nobody has moved yet
/// would be guessed to be.
///
/// Its own overlay entry rather than `showLumitModal`: no box chrome, no
/// dimmed backdrop — the work stays fully visible because the work is what
/// the console acts on (the scrim went with the ring, owner's ruling).
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

  // The anchor arrives in window coordinates and the console is laid out in
  // the overlay's own space, which is not the same space at any UI scale but
  // 100% (K-560) — so it is converted here, exactly as a popup's is.
  final at = anchor == null ? null : overlayLocal(context, anchor);
  entry = OverlayEntry(
    builder: (_) => _FxConsole(model: model, anchor: at, onClose: close),
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

/// The matching entries, effects first and compositions after. Ranked within
/// each kind, never across it: a comp is never allowed to outrank an effect,
/// because the reason to open this window is nearly always an effect.
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

// The popover's manifest, from the approved board (Console.dc.html): each
// band's height and inset. The width is the board's 320 as a floor, grown to
// fit the category strip — see [_FxConsoleState._width].
const double _popWidth = 320;
const double _popWidthMax = 720;
const double _searchRowHeight = 28;
const double _stripHeight = 22;
const double _rowHeight = 26;
const double _footHeight = 20;
const double _margin = 8;

class _FxConsoleState extends State<_FxConsole> {
  final TextEditingController _query = TextEditingController();

  /// The search field's focus, held here because the console steers it: the
  /// field is focused on open and *kept* focused for the console's whole life
  /// (K-328), so anything typed while the console is up lands in the box.
  final FocusNode _queryFocus = FocusNode(debugLabel: 'fx-console-query');
  int _highlighted = 0;

  /// The category the strip has narrowed to, or null for All.
  String? _category;

  @override
  void initState() {
    super.initState();
    _query.addListener(() => setState(() => _highlighted = 0));
    // Escape has to work with focus anywhere. A handler on the search field's
    // node covers only the field, so this claims the ladder's dialogue rung
    // for the console's lifetime (widgets/escape_ladder.dart): one press is
    // one step back, and a menu raised over the console is what that press
    // takes first.
    _escapeRelease = EscapeLadder.register(EscapeRung.dialog, _escapeAnywhere);
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
    _escapeRelease?.call();
    _escapeRelease = null;
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

  /// How to stand down from the ladder.
  VoidCallback? _escapeRelease;

  bool _escapeAnywhere() {
    _back();
    return true;
  }

  /// The matches, ranked by the query and then narrowed to the strip's
  /// category. The category compares against the entry's own group, so the
  /// strip needs no idea of what the groups mean.
  List<FxConsoleEntry> get _matches => [
        for (final entry in fxConsoleMatches(widget.model.entries, _query.text))
          if (_category == null || entry.group == _category) entry,
      ];

  /// Every group the entries carry, in first-appearance order — the browse
  /// groupings the strip offers after All (K-645 files the drivers under
  /// Controls before they ever get here).
  List<String> get _groups {
    final seen = <String>[];
    for (final entry in widget.model.entries) {
      final group = entry.group;
      if (group != null && !seen.contains(group)) seen.add(group);
    }
    return seen;
  }

  void _runHighlighted(List<FxConsoleEntry> matches) {
    if (matches.isEmpty) return;
    final entry = matches[_highlighted.clamp(0, matches.length - 1)];
    widget.onClose();
    entry.run();
  }

  /// One step out: typed text clears first, then the console closes — so
  /// Escape always retreats by exactly one decision.
  void _back() {
    if (_query.text.trim().isNotEmpty) {
      _query.clear();
    } else {
      widget.onClose();
    }
  }

  /// The strip's own kicker face — one definition, so the measurement below
  /// and the drawing cannot disagree about a pixel.
  TextStyle _kickerStyle(LumitTheme t) => t.kicker.copyWith(letterSpacing: 0.54);

  /// The popover's width: the board's 320 as the floor, grown so every
  /// category kicker in the strip fits without truncation (the longest set is
  /// the whole effect catalogue's groupings plus Presets), and capped at
  /// [_popWidthMax] — past the cap the strip scrolls sideways rather than
  /// clipping, since a plugin may declare any number of groupings.
  double _width(LumitTheme t) {
    double text(String label) {
      final painter = TextPainter(
        text: TextSpan(text: label, style: _kickerStyle(t)),
        textDirection: TextDirection.ltr,
      )..layout();
      final width = painter.width;
      painter.dispose();
      return width;
    }

    // The strip's row: 10 padding each side, All, then 10 of air before each
    // group — the same numbers [_categoryStrip] lays out with.
    var strip = 20 + text(l10n.fxConsoleAll);
    for (final group in _groups) {
      strip += 10 + text(group);
    }
    return strip.clamp(_popWidth, _popWidthMax);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final matches = _matches;
    final width = _width(t);

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
          final anchor =
              widget.anchor ?? Offset(box.maxWidth / 2, box.maxHeight / 2);
          // The search row lands on the pointer, pulled in so the whole
          // popover fits; the list is capped to the room below it. 56 is the
          // least list the clamp guarantees room for — two rows and the
          // vertical padding — so a pointer at the very bottom still shows a
          // usable console rather than a bar with nothing under it.
          final left = _fit(
              anchor.dx - width / 2, _margin, box.maxWidth - width - _margin);
          final top = _fit(anchor.dy - _searchRowHeight / 2, _margin,
              box.maxHeight - _searchRowHeight - _stripHeight - _footHeight - 56 - _margin);
          final room = box.maxHeight -
              top -
              _margin -
              _searchRowHeight -
              _stripHeight -
              _footHeight;
          return Stack(
            children: [
              // Invisible, not a scrim (owner's ruling: the hush went with the
              // ring): the work stays untouched underneath, and this only
              // catches the click that means "never mind".
              Positioned.fill(
                key: const ValueKey('fx-console-away'),
                child: GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onTap: widget.onClose,
                  onSecondaryTap: widget.onClose,
                ),
              ),
              Positioned(
                key: const ValueKey('fx-console-bar'),
                left: left,
                top: top,
                width: width,
                child: _popover(t, matches, room),
              ),
            ],
          );
        },
      ),
    );
  }

  static double _fit(double v, double lo, double hi) =>
      hi < lo ? lo : (v < lo ? lo : (v > hi ? hi : v));

  /// The popover: the standard float surface with the board's four bands.
  Widget _popover(LumitTheme t, List<FxConsoleEntry> matches, double room) =>
      Container(
        decoration: BoxDecoration(
          color: t.surface1,
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          border: Border.all(color: t.hairline, width: 1),
          boxShadow: t.floatShadow,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            _searchRow(t, matches),
            if (_groups.isNotEmpty) ...[
              Container(height: 1, color: t.hairline),
              _categoryStrip(t),
            ],
            Container(height: 1, color: t.hairline),
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 4),
              child: _list(t, matches, room),
            ),
            if (widget.model.footer case final footer?) ...[
              Container(height: 1, color: t.hairline),
              Container(
                height: _footHeight,
                padding: const EdgeInsets.symmetric(horizontal: 10),
                alignment: Alignment.centerLeft,
                child: Text(footer,
                    key: const ValueKey('fx-console-foot'),
                    style: t.kicker.copyWith(letterSpacing: 0.54)),
              ),
            ],
          ],
        ),
      );

  Widget _searchRow(LumitTheme t, List<FxConsoleEntry> matches) => Container(
        height: _searchRowHeight,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        child: Row(
          children: [
            glyph.LumitIcon(LumitIcons.search,
                size: iconSize, colour: t.textMuted),
            const SizedBox(width: 8),
            Expanded(
              child: HouseTextField(
                key: const ValueKey('fx-console-query'),
                controller: _query,
                width: 200,
                frameless: true,
                padding: EdgeInsets.zero,
                focusNode: _queryFocus,
                style: t.bodyPrimary,
                hint: l10n.fxConsoleHint,
                onSubmitted: (_) => _runHighlighted(matches),
              ),
            ),
            if (widget.model.keyHint case final hint?) ...[
              const SizedBox(width: 8),
              Text(hint,
                  key: const ValueKey('fx-console-key'),
                  style: t.kicker.copyWith(letterSpacing: 0.54)),
            ],
            // The snapshot button, folded in from the old bar (K-324): one
            // press writes the frame on screen to a PNG, so two versions of a
            // look can be compared without setting an export up. The board
            // draws no home for it; the row's end is where FX Console keeps
            // its own.
            const SizedBox(width: 8),
            LumitTooltip(
              message: l10n.fxConsoleSnapshotTip,
              child: HouseButton(
                key: const ValueKey('fx-console-snapshot'),
                small: true,
                frameless: true,
                padding: const EdgeInsets.symmetric(horizontal: 2),
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

  /// The strip of category kickers: All, then every group the entries carry.
  /// The chosen one reads at full strength; choosing narrows the list to that
  /// group, and All lets everything back in. The popover is sized so the whole
  /// strip fits ([_width]); the scroll view is the ceiling for a plugin
  /// catalogue past the cap, never a truncation.
  Widget _categoryStrip(LumitTheme t) => Container(
        height: _stripHeight,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        child: SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: Row(
            children: [
              _categoryKicker(t, label: l10n.fxConsoleAll, category: null),
              for (final group in _groups) ...[
                const SizedBox(width: 10),
                _categoryKicker(t, label: group, category: group),
              ],
            ],
          ),
        ),
      );

  Widget _categoryKicker(LumitTheme t,
      {required String label, required String? category}) {
    final chosen = _category == category;
    return GestureDetector(
      key: ValueKey<String>('fx-console-cat-${category ?? '*all'}'),
      behavior: HitTestBehavior.opaque,
      onTap: () => setState(() {
        _category = category;
        _highlighted = 0;
      }),
      child: Text(label,
          style: _kickerStyle(t)
              .copyWith(color: chosen ? t.textPrimary : t.textMuted)),
    );
  }

  Widget _list(LumitTheme t, List<FxConsoleEntry> matches, double room) {
    if (matches.isEmpty) {
      return Container(
        height: _rowHeight,
        alignment: Alignment.centerLeft,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        child: Text(l10n.noEffectsMatch,
            style: t.small.copyWith(color: t.textMuted)),
      );
    }
    return ConstrainedBox(
        constraints: BoxConstraints(maxHeight: room.clamp(48.0, 260.0)),
        child: ListView.builder(
          shrinkWrap: true,
          itemCount: matches.length,
          itemBuilder: (context, i) {
            final entry = matches[i];
            final hot = i == _highlighted;
            return GestureDetector(
              key: ValueKey<String>('fx-console-item-${entry.label}'),
              behavior: HitTestBehavior.opaque,
              onTap: () {
                widget.onClose();
                entry.run();
              },
              child: Container(
                height: _rowHeight,
                padding: const EdgeInsets.symmetric(horizontal: 10),
                color: hot ? t.surface2 : null,
                child: Row(
                  children: [
                    Expanded(
                      child: Text(entry.label,
                          overflow: TextOverflow.ellipsis,
                          style: hot ? t.bodyPrimary : t.body),
                    ),
                    if (entry.group case final group?) ...[
                      const SizedBox(width: 8),
                      Text(group, style: t.kicker.copyWith(letterSpacing: 0.54)),
                    ],
                    const SizedBox(width: 8),
                    // The preview swatch the board draws on every row. There
                    // is no per-effect render behind it yet, so it is the
                    // surface's own gradient — the slot the picture will take.
                    Container(
                      width: 34,
                      height: 19,
                      decoration: BoxDecoration(
                        borderRadius:
                            BorderRadius.circular(t.tokens.controlRadius),
                        border: Border.all(color: t.hairline, width: 1),
                        gradient: LinearGradient(
                          begin: Alignment.topLeft,
                          end: Alignment.bottomRight,
                          colors: [t.surface2, t.surface4],
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            );
          },
        ));
  }
}
