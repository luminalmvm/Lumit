// The command palette, on the flutter_rust_bridge API.
//
// Type to filter, arrow keys to move, Enter to run. The commands are declared
// where they act — the caller passes them in — so the palette itself knows
// nothing about the document and cannot drift out of step with what the menus
// actually do.
//
// A letter or two match as a subsequence: "nc" finds "New composition", which
// is what makes a palette faster than a menu. From three letters the needle
// has to sit in the name whole or start its words. Ranking prefers matches
// that start earlier and are more tightly packed, so the thing you
// half-remembered is at the top rather than buried under a coincidence.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../icons/icons.dart' show iconSize;
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/edge_fade.dart';
import '../widgets/escape_ladder.dart';
import 'profile_chip.dart' show ProBadge;

/// One thing the palette can run.
class PaletteCommand {
  final String label;

  /// The group it belongs to, the heading its row sits under (docs/07 §12:
  /// an effect must never be mistaken for a command).
  final String category;

  /// The keyboard shortcut, taught in the result row where one exists. The
  /// caller reads it from the live keymap, so a rebound chord is the chord the
  /// palette teaches.
  final String? shortcut;

  /// Whether the row wears a tick: a menu toggle or option that is on now.
  final bool ticked;

  /// Why the row cannot be run just now, shown greyed in the shortcut's place.
  /// Null for a row that can.
  final String? disabled;

  /// The mark before the name, drawn by the caller, for what has one: a
  /// comp, a layer of its kind, an effect. A command has none, and its group
  /// has no slot for one.
  final Widget? icon;

  /// A row for Lumit Pro, to an account without it, wears the menu's badge.
  final bool needsPro;
  final VoidCallback run;

  const PaletteCommand({
    required this.label,
    required this.category,
    this.shortcut,
    this.ticked = false,
    this.disabled,
    this.icon,
    this.needsPro = false,
    required this.run,
  });
}

/// How many of the recently run lead the list before anything is typed.
const int paletteRecentShown = 8;

/// What stands between the steps of a path in a label, as in
/// "Layer › New › Solid".
const String palettePathSeparator = ' › ';

/// Open the palette over [context].
///
/// [recent] is the labels of recently run entries, newest first — what
/// "recently used entries rank first" (docs/07 §12) means in practice: for an
/// empty query they lead outright, and for a typed one they break score ties.
/// [onRun] is told what was run. The list and the remembering are the
/// workspace's, held with the rest of the per-user settings, so the palette
/// keeps no state of its own and remembers across restarts.
Future<void> showCommandPaletteFrb({
  required BuildContext context,
  required List<PaletteCommand> commands,
  required List<String> recent,
  required void Function(String label) onRun,
}) =>
    showLumitModal<void>(
      context: context,
      still: true,
      builder: (close) => _Palette(
        commands: commands,
        recent: recent,
        onRun: onRun,
        onClose: () => close(null),
      ),
    );

/// How well `needle` matches `haystack`, or null for no match.
///
/// Lower is better. The score is the span the match occupies plus where it
/// starts, so "comp" scores better against "Composition settings" than against
/// "New composition": earlier and tighter wins. One or two letters match as
/// a subsequence; three or more have to sit in the name whole from the start
/// of a word, or each start a word, since a loose subsequence had "fit"
/// finding Photo filter and a match inside a word had "reset" finding every
/// preset. Hyphens don't count, so "precompose" finds Pre-compose.
int? paletteScore(String needle, String haystack) {
  final want = needle.toLowerCase().replaceAll('-', '');
  if (want.isEmpty) return 0;
  final strict = want.length >= 3;
  if (strict) {
    if (_wholeAtWord(want, haystack) case final at?) {
      return (want.length - 1) + at;
    }
  }
  final (lower, starts) = _words(haystack);
  var at = 0;
  var first = -1;
  var last = 0;
  for (final rune in want.runes) {
    final char = String.fromCharCode(rune);
    var found = lower.indexOf(char, at);
    if (strict) {
      while (found >= 0 && !starts.contains(found)) {
        found = lower.indexOf(char, found + 1);
      }
    }
    if (found < 0) return null;
    if (first < 0) first = found;
    last = found;
    at = found + 1;
  }
  return (last - first) + first;
}

final RegExp _wordChar = RegExp(r'[\p{L}\p{N}]', unicode: true);

/// [text] in lower case with its hyphens taken out, and where each of its
/// words starts in that, a hyphen still counting as a break.
(String, Set<int>) _words(String text) {
  final source = text.toLowerCase();
  final out = StringBuffer();
  final starts = <int>{};
  for (var i = 0; i < source.length; i++) {
    if (source[i] == '-') continue;
    if (i == 0 || !_wordChar.hasMatch(source[i - 1])) starts.add(out.length);
    out.write(source[i]);
  }
  return (out.toString(), starts);
}

/// Where [want], lower case and without hyphens, sits whole in [text] from
/// the start of a word, or null.
int? _wholeAtWord(String want, String text) {
  final (lower, starts) = _words(text);
  for (var at = lower.indexOf(want);
      at >= 0;
      at = lower.indexOf(want, at + 1)) {
    if (starts.contains(at)) return at;
  }
  return null;
}

class _Palette extends StatefulWidget {
  final List<PaletteCommand> commands;
  final List<String> recent;
  final void Function(String label) onRun;
  final VoidCallback onClose;
  const _Palette({
    required this.commands,
    required this.recent,
    required this.onRun,
    required this.onClose,
  });

  @override
  State<_Palette> createState() => _PaletteState();
}

class _PaletteState extends State<_Palette> {
  final TextEditingController _query = TextEditingController();
  final FocusNode _focus = FocusNode();
  final ScrollController _scroll = ScrollController();
  int _highlighted = 0;

  @override
  void initState() {
    super.initState();
    _query.addListener(() {
      // A new list starts from its top, highlight and all.
      if (_scroll.hasClients) _scroll.jumpTo(0);
      setState(() => _highlighted = 0);
    });
    _focus.requestFocus();
    // Escape closes the palette from the ladder's dialogue rung
    // (widgets/escape_ladder.dart) rather than from the field's focus node:
    // the focus path runs last of all, so a press meant for a menu raised over
    // the palette used to shut both.
    _escapeRelease = EscapeLadder.register(EscapeRung.dialog, () {
      widget.onClose();
      return true;
    });
  }

  /// How to stand down from the ladder.
  VoidCallback? _escapeRelease;

  @override
  void dispose() {
    _escapeRelease?.call();
    _escapeRelease = null;
    _query.dispose();
    _focus.dispose();
    _scroll.dispose();
    super.dispose();
  }

  List<PaletteCommand> get _matches {
    final needle = _query.text.trim();
    final scored = <(int, int, int, int)>[];
    for (var i = 0; i < widget.commands.length; i++) {
      final label = widget.commands[i].label;
      final crumbs = label.split(palettePathSeparator);
      // A menu command is known by its last step, so "solid" ranks
      // "Layer › New › Solid" as it would rank "Solid". The steps before it
      // count only when one of them holds the needle whole, and after every
      // match on a last step: a subsequence over the whole path had "blur"
      // finding Blending mode › Subtract.
      final leaf = paletteScore(needle, crumbs.last);
      final score = leaf ?? _crumbScore(needle, crumbs);
      if (score == null) continue;
      final recency = widget.recent.indexOf(label);
      scored.add((
        // What can't run now comes after everything that can.
        (widget.commands[i].disabled == null ? 0 : 2) + (leaf == null ? 1 : 0),
        score,
        recency < 0 ? widget.recent.length : recency,
        i,
      ));
    }
    // Relevance first, recency breaking ties — which, for the empty query
    // where every score is zero, is exactly "recently used rank first". Then
    // the order the caller listed them in, because the sort keeps no order of
    // its own.
    scored.sort((a, b) {
      final byKind = a.$1.compareTo(b.$1);
      if (byKind != 0) return byKind;
      final byScore = a.$2.compareTo(b.$2);
      if (byScore != 0) return byScore;
      final byRecency = a.$3.compareTo(b.$3);
      return byRecency != 0 ? byRecency : a.$4.compareTo(b.$4);
    });
    return [for (final entry in scored) widget.commands[entry.$4]];
  }

  /// Where [needle] sits whole in one of [crumbs], or null when it is in
  /// none of them. Never across two: a path is read a step at a time.
  static int? _crumbScore(String needle, List<String> crumbs) {
    final want = needle.toLowerCase().replaceAll('-', '');
    if (want.isEmpty) return null;
    for (final crumb in crumbs) {
      if (_wholeAtWord(want, crumb) case final at?) return at;
    }
    return null;
  }

  void _run(PaletteCommand command) {
    if (command.disabled != null) return;
    widget.onRun(command.label);
    widget.onClose();
    command.run();
  }

  void _runHighlighted(List<PaletteCommand> matches) {
    if (matches.isEmpty) return;
    _run(matches[_highlighted.clamp(0, matches.length - 1)]);
  }

  /// The list as drawn: the matches under one heading per kind, in the order
  /// the kinds first appear, so the best-ranked kind leads a typed list.
  /// Before anything is typed the last eight run come first, under their own.
  List<Object> _entries(List<PaletteCommand> matches) {
    final out = <Object>[];
    final recent = [
      if (_query.text.trim().isEmpty)
        for (final m in matches)
          if (widget.recent.contains(m.label)) m,
    ].take(paletteRecentShown).toList();
    if (recent.isNotEmpty) {
      out
        ..add(l10n.paletteRecent)
        ..addAll(recent);
    }
    final byKind = <String, List<PaletteCommand>>{};
    for (final m in matches) {
      if (recent.contains(m)) continue;
      (byKind[m.category] ??= []).add(m);
    }
    for (final MapEntry(key: kind, value: rows) in byKind.entries) {
      out
        ..add(kind)
        ..addAll(rows);
    }
    return out;
  }

  /// Move the highlight to match [to] and scroll its row into view. Every
  /// row, heading or match, is the height of the list's prototype, so the
  /// whole list divided by the entry count is one row.
  void _highlight(
      int to, List<Object> entries, List<PaletteCommand> matches) {
    setState(() => _highlighted = to);
    if (!_scroll.hasClients || matches.isEmpty) return;
    final at = entries.indexOf(matches[to]);
    final position = _scroll.position;
    final row = (position.maxScrollExtent + position.viewportDimension) /
        entries.length;
    // With its heading, when it is the first under one.
    final top = (at > 0 && entries[at - 1] is String ? at - 1 : at) * row;
    final bottom = at * row + row - position.viewportDimension;
    if (position.pixels > top) _scroll.jumpTo(top);
    if (position.pixels < bottom) _scroll.jumpTo(bottom);
  }

  /// A group's name over its rows. The list's prototype row gives it the
  /// row's own height, so the list stays regular.
  Widget _heading(LumitTheme t, String name) => Container(
        alignment: AlignmentDirectional.centerStart,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        child: Text(t.kickerCase(name), style: t.kicker),
      );

  /// One result row: the mark, the name, the tick and the chord, at the
  /// height every menu row has. The heading says the kind. A kind's rows all
  /// have a mark or none do, so the names of a group stand on one edge.
  Widget _row(LumitTheme t, PaletteCommand command, {bool selected = false}) {
    final off = command.disabled != null;
    return MenuRow(
      selected: selected,
      onPressed: () => _run(command),
      child: Row(
        children: [
          if (command.icon case final icon?) ...[
            SizedBox(width: iconSize, child: icon),
            const SizedBox(width: 8),
          ],
          Expanded(
            child: Text(
              command.label,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: off ? t.body.copyWith(color: t.textDisabled) : null,
            ),
          ),
          // On the right, before the tick and the chord, as in the menus.
          if (command.needsPro)
            const Padding(
              padding: EdgeInsetsDirectional.only(start: 16, end: 8),
              child: Opacity(opacity: 0.6, child: ProBadge()),
            ),
          // After the name, so ticked and unticked names start in line.
          if (command.ticked) ...[
            menuTick(true, colour: t.textMuted),
            const SizedBox(width: 8),
          ],
          // Drawn as the menus draw a chord, greyed row or not.
          if (command.disabled ?? command.shortcut case final note?)
            Text(note, style: t.small.copyWith(color: t.textMuted)),
        ],
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final entries = _entries(_matches);
    // In the order drawn, which is what the arrow keys walk.
    final matches = [for (final e in entries) if (e is PaletteCommand) e];

    return Focus(
      onKeyEvent: (node, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        switch (event.logicalKey) {
          case LogicalKeyboardKey.arrowDown:
            _highlight(
                matches.isEmpty ? 0 : (_highlighted + 1) % matches.length,
                entries,
                matches);
            return KeyEventResult.handled;
          case LogicalKeyboardKey.arrowUp:
            _highlight(
                matches.isEmpty
                    ? 0
                    : (_highlighted - 1 + matches.length) % matches.length,
                entries,
                matches);
            return KeyEventResult.handled;
          case LogicalKeyboardKey.enter:
            _runHighlighted(matches);
            return KeyEventResult.handled;
          default:
            return KeyEventResult.ignored;
        }
      },
      child: FloatSurface(
        width: 420,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.all(8),
              child: HouseTextField(
                key: const ValueKey('palette-query'),
                controller: _query,
                focusNode: _focus,
                width: 400,
                onSubmitted: (_) => _runHighlighted(matches),
              ),
            ),
            if (matches.isEmpty)
              Padding(
                padding: const EdgeInsets.all(10),
                child: Text(l10n.noCommandsMatch, style: t.small),
              )
            else
              ConstrainedBox(
                constraints: const BoxConstraints(maxHeight: 300),
                child: EdgeFade(
                  axis: Axis.vertical,
                  child: ListView.builder(
                    shrinkWrap: true,
                    controller: _scroll,
                    padding: EdgeInsets.zero,
                    prototypeItem: _row(t, matches.first),
                    itemCount: entries.length,
                    itemBuilder: (context, i) => switch (entries[i]) {
                      final String name => _heading(t, name),
                      final PaletteCommand command => KeyedSubtree(
                          key: ValueKey<String>(
                              'palette-item-${command.label}'),
                          child: _row(t, command,
                              selected:
                                  matches.indexOf(command) == _highlighted),
                        ),
                      _ => const SizedBox.shrink(),
                    },
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}
