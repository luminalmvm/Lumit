// The command palette, on the flutter_rust_bridge API.
//
// Type to filter, arrow keys to move, Enter to run. The commands are declared
// where they act — the caller passes them in — so the palette itself knows
// nothing about the document and cannot drift out of step with what the menus
// actually do.
//
// Matching is subsequence, not substring: "nc" finds "New composition", which is
// what makes a palette faster than a menu. Ranking prefers matches that start
// earlier and are more tightly packed, so the thing you half-remembered is at
// the top rather than buried under a coincidence.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../icons/icons.dart' show iconSize;
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/escape_ladder.dart';

/// One thing the palette can run.
class PaletteCommand {
  final String label;

  /// The group it belongs to, shown as the row's category badge (docs/07
  /// §12: an effect must never be mistaken for a command).
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
  /// comp, a layer of its kind, an effect. Every row keeps the slot, so the
  /// names stand on one edge whether or not a row has a mark.
  final Widget? icon;
  final VoidCallback run;

  const PaletteCommand({
    required this.label,
    required this.category,
    this.shortcut,
    this.ticked = false,
    this.disabled,
    this.icon,
    required this.run,
  });
}

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

/// How well `needle` matches `haystack` as a subsequence, or null for no match.
///
/// Lower is better. The score is the span the match occupies plus where it
/// starts, so "comp" scores better against "Composition settings" than against
/// "New composition" — earlier and tighter wins.
int? paletteScore(String needle, String haystack) {
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
    final scored = <(int, int, int)>[];
    for (var i = 0; i < widget.commands.length; i++) {
      final label = widget.commands[i].label;
      final whole = paletteScore(needle, label);
      if (whole == null) continue;
      // A menu command is known by its last step, so "solid" ranks
      // "Layer › New › Solid" as it would rank "Solid", not as a match that
      // starts fourteen letters in.
      final leaf =
          paletteScore(needle, label.split(palettePathSeparator).last) ?? whole;
      final recency = widget.recent.indexOf(label);
      scored.add((
        leaf < whole ? leaf : whole,
        recency < 0 ? widget.recent.length : recency,
        i,
      ));
    }
    // Relevance first, recency breaking ties — which, for the empty query
    // where every score is zero, is exactly "recently used rank first". Then
    // the order the caller listed them in, because the sort keeps no order of
    // its own.
    scored.sort((a, b) {
      final byScore = a.$1.compareTo(b.$1);
      if (byScore != 0) return byScore;
      final byRecency = a.$2.compareTo(b.$2);
      return byRecency != 0 ? byRecency : a.$3.compareTo(b.$3);
    });
    return [for (final entry in scored) widget.commands[entry.$3]];
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

  /// The list as drawn: the matches, and before anything is typed the
  /// headings that group them, the recently run first and then each kind
  /// under its own name. A typed query is one ranked list, where the badge
  /// at the row's end says the kind.
  List<Object> _entries(List<PaletteCommand> matches) {
    if (_query.text.trim().isNotEmpty) return matches;
    final out = <Object>[];
    final recent = [
      for (final m in matches)
        if (widget.recent.contains(m.label)) m,
    ];
    if (recent.isNotEmpty) {
      out
        ..add(l10n.paletteRecent)
        ..addAll(recent);
    }
    String? last;
    for (final m in matches) {
      if (recent.contains(m)) continue;
      if (m.category != last) {
        last = m.category;
        out.add(last);
      }
      out.add(m);
    }
    return out;
  }

  /// Move the highlight to match [to] and scroll its row into view. Every
  /// row, heading or match, is the height of the list's prototype, so the
  /// whole list divided by the entry count is one row.
  void _highlight(int to, List<PaletteCommand> matches) {
    setState(() => _highlighted = to);
    if (!_scroll.hasClients || matches.isEmpty) return;
    final entries = _entries(matches);
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

  /// A group's name over its rows, one row tall so the list stays regular.
  Widget _heading(LumitTheme t, String name) => Container(
        height: _rowHeight,
        alignment: AlignmentDirectional.centerStart,
        padding: const EdgeInsets.symmetric(horizontal: 10),
        child: Text(t.kickerCase(name), style: t.kicker),
      );

  /// One line of the list, heading or match, for the prototype and the
  /// scrolling arithmetic.
  static const double _rowHeight = 26;

  /// One result row: the mark, the name, the tick, the chord and the badge.
  /// Under a heading the badge would say the heading again, so it goes.
  Widget _row(LumitTheme t, PaletteCommand command, {bool selected = false}) {
    final off = command.disabled != null;
    final badged = _query.text.trim().isNotEmpty;
    return SizedBox(
      height: _rowHeight,
      child: MenuRow(
        selected: selected,
        onPressed: () => _run(command),
        child: Row(
          children: [
            SizedBox(width: iconSize, child: command.icon),
            const SizedBox(width: 8),
            Expanded(
              child: Text(
                command.label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: off ? t.body.copyWith(color: t.textDisabled) : null,
              ),
            ),
            // After the name, so ticked and unticked names start in line.
            if (command.ticked) ...[
              menuTick(true, colour: t.textMuted),
              const SizedBox(width: 8),
            ],
            if (command.disabled ?? command.shortcut case final note?) ...[
              Text(note,
                  style:
                      off ? t.small.copyWith(color: t.textDisabled) : t.mono),
              const SizedBox(width: 8),
            ],
            if (badged)
              Text(command.category,
                  style: t.small.copyWith(color: t.textMuted)),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final matches = _matches;
    final entries = _entries(matches);

    return Focus(
      onKeyEvent: (node, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        switch (event.logicalKey) {
          case LogicalKeyboardKey.arrowDown:
            _highlight(
                matches.isEmpty ? 0 : (_highlighted + 1) % matches.length,
                matches);
            return KeyEventResult.handled;
          case LogicalKeyboardKey.arrowUp:
            _highlight(
                matches.isEmpty
                    ? 0
                    : (_highlighted - 1 + matches.length) % matches.length,
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
                child: ListView.builder(
                  shrinkWrap: true,
                  controller: _scroll,
                  padding: EdgeInsets.zero,
                  prototypeItem: _row(t, matches.first),
                  itemCount: entries.length,
                  itemBuilder: (context, i) => switch (entries[i]) {
                    final String name => _heading(t, name),
                    final PaletteCommand command => KeyedSubtree(
                        key: ValueKey<String>('palette-item-${command.label}'),
                        child: _row(t, command,
                            selected: matches.indexOf(command) == _highlighted),
                      ),
                    _ => const SizedBox.shrink(),
                  },
                ),
              ),
          ],
        ),
      ),
    );
  }
}
