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

import '../l10n/strings.dart';
import '../widgets/controls.dart';

/// One thing the palette can run.
class PaletteCommand {
  final String label;

  /// The group it belongs to, shown as the row's category badge (docs/07
  /// §12: an effect must never be mistaken for a command).
  final String category;

  /// The keyboard shortcut, taught in the result row where one exists.
  final String? shortcut;
  final VoidCallback run;

  const PaletteCommand({
    required this.label,
    required this.category,
    this.shortcut,
    required this.run,
  });
}

/// The labels of recently run entries, most recent first — what "recently
/// used entries rank first" (docs/07 §12) means in practice: for an empty
/// query they lead outright, and for a typed one they break score ties.
/// Session-lived on purpose; a palette that remembers across restarts is a
/// settings file for another day.
final List<String> _recent = [];

void _noteRun(String label) {
  _recent.remove(label);
  _recent.insert(0, label);
  if (_recent.length > 20) _recent.removeLast();
}

Future<void> showCommandPaletteFrb({
  required BuildContext context,
  required List<PaletteCommand> commands,
}) =>
    showLumitModal<void>(
      context: context,
      builder: (close) => _Palette(
        commands: commands,
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
  final VoidCallback onClose;
  const _Palette({required this.commands, required this.onClose});

  @override
  State<_Palette> createState() => _PaletteState();
}

class _PaletteState extends State<_Palette> {
  final TextEditingController _query = TextEditingController();
  final FocusNode _focus = FocusNode();
  int _highlighted = 0;

  @override
  void initState() {
    super.initState();
    _query.addListener(() => setState(() => _highlighted = 0));
    _focus.requestFocus();
  }

  @override
  void dispose() {
    _query.dispose();
    _focus.dispose();
    super.dispose();
  }

  List<PaletteCommand> get _matches {
    final needle = _query.text.trim();
    final scored = <(int, int, PaletteCommand)>[];
    for (final command in widget.commands) {
      final score = paletteScore(needle, command.label);
      if (score == null) continue;
      final recency = _recent.indexOf(command.label);
      scored.add((score, recency < 0 ? _recent.length : recency, command));
    }
    // Relevance first, recency breaking ties — which, for the empty query
    // where every score is zero, is exactly "recently used rank first".
    scored.sort((a, b) {
      final byScore = a.$1.compareTo(b.$1);
      return byScore != 0 ? byScore : a.$2.compareTo(b.$2);
    });
    return [for (final entry in scored) entry.$3];
  }

  void _runHighlighted(List<PaletteCommand> matches) {
    if (matches.isEmpty) return;
    final command = matches[_highlighted.clamp(0, matches.length - 1)];
    _noteRun(command.label);
    widget.onClose();
    command.run();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final matches = _matches;

    return Focus(
      autofocus: true,
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
            _runHighlighted(matches);
            return KeyEventResult.handled;
          case LogicalKeyboardKey.escape:
            widget.onClose();
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
                child: ListView(
                  shrinkWrap: true,
                  children: [
                    for (var i = 0; i < matches.length; i++)
                      MenuRow(
                        key: ValueKey<String>(
                            'palette-item-${matches[i].label}'),
                        selected: i == _highlighted,
                        onPressed: () {
                          _noteRun(matches[i].label);
                          widget.onClose();
                          matches[i].run();
                        },
                        child: Row(
                          children: [
                            Expanded(child: Text(matches[i].label)),
                            if (matches[i].shortcut != null) ...[
                              Text(matches[i].shortcut!, style: t.mono),
                              const SizedBox(width: 8),
                            ],
                            Text(matches[i].category,
                                style: t.small.copyWith(color: t.textMuted)),
                          ],
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
