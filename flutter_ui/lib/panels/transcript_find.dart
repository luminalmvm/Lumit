// The Transcript panel's results bar: a search, the filler words, the long
// pauses or one speaker's words, walked one at a time, with the actions that
// take them out.
//
// The bar holds what is found and which result is in hand; the panel does
// the edits, so every action here is a callback into it.

import 'package:flutter/widgets.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/settings.dart';
import '../widgets/controls.dart';
import 'transcript_text.dart';

enum TranscriptFindMode { search, fillers, pauses, speakers }

/// What is found and which one is in hand. Results are runs of tokens.
class TranscriptFind extends ChangeNotifier {
  TranscriptFindMode? mode;
  final TextEditingController query = TextEditingController();
  final TextEditingController replace = TextEditingController();
  final FocusNode queryFocus = FocusNode();

  List<(int, int)> results = const [];
  int current = -1;

  /// Every token in a result, worked out with them.
  Set<int> foundTokens = const {};

  /// The fillers Keep passed over, by [TranscriptToken.identity], which
  /// outlives the token indices an edit shifts.
  final Set<Object> kept = {};

  /// The speaker the speakers review lists.
  int speaker = 0;

  /// Whether the search's Replace row is out.
  bool replaceOpen = false;

  void toggleReplace() {
    replaceOpen = !replaceOpen;
    notifyListeners();
  }

  /// The result in hand's first word and where it started, so it is found
  /// again after an edit moves everything, or the next one is when it has
  /// gone.
  Object? _currentKey;
  int? _currentStart;

  void _hold(TranscriptModel model) {
    if (current < 0) return;
    final t = model.tokens[results[current].$1];
    _currentKey = t.identity;
    _currentStart = t.startFrame;
  }

  bool get isOpen => mode != null;

  /// Open the bar in [to], or bring it to that mode.
  void show(TranscriptFindMode to) {
    if (mode != to) {
      mode = to;
      kept.clear();
      current = -1;
      _currentKey = null;
      _currentStart = null;
    }
    notifyListeners();
  }

  void close() {
    mode = null;
    results = const [];
    foundTokens = const {};
    current = -1;
    _currentKey = null;
    _currentStart = null;
    // The well empties with the bar, so no search is left standing in it.
    query.clear();
    notifyListeners();
  }

  /// Work the results out again over [model] with the panel's settings.
  void refresh(TranscriptModel model, InterfaceSettings s) {
    final m = mode;
    if (m == null) return;
    results = switch (m) {
      TranscriptFindMode.search => model.find(query.text),
      TranscriptFindMode.fillers => [
          for (final i in model.fillers(
            hesitations: s.transcriptHesitations,
            phrases: s.transcriptPhrases,
            repeats: s.transcriptRepeats,
            retakes: s.transcriptRetakes,
            skipTight: s.transcriptSkipTight,
          ))
            if (!kept.contains(model.tokens[i].identity)) (i, i + 1),
        ],
      TranscriptFindMode.pauses => [
          for (final i in model.pauses(s.transcriptLongerThanMs)) (i, i + 1),
        ],
      TranscriptFindMode.speakers => _runs(model.saidBy(speaker)),
    };
    if (results.isEmpty) {
      current = -1;
    } else if (_currentStart case final start?) {
      // The one in hand, or the first one after where it was.
      current = results
          .indexWhere((r) => model.tokens[r.$1].identity == _currentKey);
      if (current < 0) {
        current = results
            .indexWhere((r) => model.tokens[r.$1].startFrame >= start);
        if (current < 0) current = results.length - 1;
      }
    } else {
      current = current.clamp(-1, results.length - 1);
    }
    _hold(model);
    foundTokens = {
      for (final (a, b) in results)
        for (var i = a; i < b; i++) i,
    };
    notifyListeners();
  }

  /// Move to the next result, or back, wrapping at the ends.
  void step(TranscriptModel model, int by) {
    if (results.isEmpty) return;
    current = current < 0
        ? (by > 0 ? 0 : results.length - 1)
        : (current + by) % results.length;
    _hold(model);
    notifyListeners();
  }

  (int, int)? get currentRange =>
      current >= 0 && current < results.length ? results[current] : null;

  int? get foundCurrentToken => currentRange?.$1;

  static List<(int, int)> _runs(List<int> tokens) {
    final runs = <(int, int)>[];
    for (final i in tokens) {
      if (runs.isNotEmpty && runs.last.$2 == i) {
        runs.last = (runs.last.$1, i + 1);
      } else {
        runs.add((i, i + 1));
      }
    }
    return runs;
  }

  @override
  void dispose() {
    query.dispose();
    replace.dispose();
    queryFocus.dispose();
    super.dispose();
  }
}

/// What the bar's buttons do, all the panel's.
class TranscriptFindActions {
  final VoidCallback previous;
  final VoidCallback next;
  final VoidCallback close;
  final VoidCallback delete;
  final VoidCallback deleteAll;
  final VoidCallback selectAll;
  final VoidCallback keep;
  final VoidCallback shorten;
  final VoidCallback shortenAll;
  final VoidCallback replace;
  final VoidCallback replaceAll;
  final ValueChanged<bool> setLift;
  final void Function(TranscriptFiller kind, bool on) setFiller;
  final ValueChanged<bool> setSkipTight;
  final ValueChanged<int> setLongerThanMs;
  final ValueChanged<int> setShortenToMs;
  final ValueChanged<int> setSpeaker;

  const TranscriptFindActions({
    required this.previous,
    required this.next,
    required this.close,
    required this.delete,
    required this.deleteAll,
    required this.selectAll,
    required this.keep,
    required this.shorten,
    required this.shortenAll,
    required this.replace,
    required this.replaceAll,
    required this.setLift,
    required this.setFiller,
    required this.setSkipTight,
    required this.setLongerThanMs,
    required this.setShortenToMs,
    required this.setSpeaker,
  });
}

/// The bar under the strip. Rebuilt by the panel when [find] changes.
class TranscriptFindBar extends StatelessWidget {
  final TranscriptFind find;
  final TranscriptModel model;
  final InterfaceSettings settings;
  final TranscriptFindActions actions;

  const TranscriptFindBar({
    super.key,
    required this.find,
    required this.model,
    required this.settings,
    required this.actions,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final mode = find.mode;
    if (mode == null) return const SizedBox.shrink();
    final count = find.results.length;
    final n = find.current < 0 ? 0 : find.current + 1;
    final some = count > 0;
    final one = find.currentRange != null;
    Widget button(String key, String label, VoidCallback? press,
            {bool on = false}) =>
        HouseButton(
          key: ValueKey<String>(key),
          small: true,
          frameless: !on,
          active: on,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          onPressed: press,
          child: Text(label),
        );
    Widget arrow(String key, String tip, AxisDirection dir, VoidCallback go) =>
        LumitTooltip(
          message: tip,
          action: dir == AxisDirection.up
              ? 'transcript.find.prev'
              : 'transcript.find.next',
          child: HouseButton(
            key: ValueKey<String>(key),
            small: true,
            frameless: true,
            padding: const EdgeInsets.symmetric(horizontal: 4),
            onPressed: count == 0 ? null : go,
            child: Caret(colour: t.textSecondary, direction: dir),
          ),
        );
    Widget labelled(String label, Widget field) =>
        Row(mainAxisSize: MainAxisSize.min, children: [
          Flexible(
              child: Text(label,
                  style: t.body, overflow: TextOverflow.ellipsis, maxLines: 1)),
          const SizedBox(width: 6),
          field,
        ]);
    // The search well itself is on the strip above.
    final controls = <Widget>[
      ...switch (mode) {
        TranscriptFindMode.search => const <Widget>[],
        TranscriptFindMode.fillers => [
            for (final (kind, label, key) in [
              (TranscriptFiller.hesitation, l10n.transcriptHesitations,
                  'transcript-chip-hesitations'),
              (TranscriptFiller.phrase, l10n.transcriptPhrases,
                  'transcript-chip-phrases'),
              (TranscriptFiller.repeat, l10n.transcriptRepeats,
                  'transcript-chip-repeats'),
              (TranscriptFiller.retake, l10n.transcriptRetakes,
                  'transcript-chip-retakes'),
            ])
              button(
                key,
                '$label ${_countOf(kind)}',
                () => actions.setFiller(kind, !_fillerOn(kind)),
                on: _fillerOn(kind),
              ),
            HouseCheckbox(
              key: const ValueKey('transcript-skip-tight'),
              value: settings.transcriptSkipTight,
              label: l10n.transcriptSkipTight,
              onChanged: actions.setSkipTight,
            ),
          ],
        TranscriptFindMode.pauses => [
            labelled(
                l10n.transcriptLongerThan,
                _seconds('transcript-longer-than',
                    settings.transcriptLongerThanMs, actions.setLongerThanMs)),
            labelled(
                l10n.transcriptShortenTo,
                _seconds('transcript-shorten-to',
                    settings.transcriptShortenToMs, actions.setShortenToMs)),
          ],
        TranscriptFindMode.speakers => [
            SizedBox(
              width: 140,
              child: BareDropdown<int>(
                key: const ValueKey('transcript-speaker-pick'),
                value: model.speakers.isEmpty
                    ? 0
                    : find.speaker.clamp(0, model.speakers.length - 1),
                options: model.speakers.isEmpty
                    ? const [0]
                    : [for (var i = 0; i < model.speakers.length; i++) i],
                label: (i) => i < model.speakers.length
                    ? model.speakers[i].name
                    : '',
                dense: true,
                onChanged: model.speakers.isEmpty ? null : actions.setSpeaker,
              ),
            ),
          ],
      },
      Row(mainAxisSize: MainAxisSize.min, children: [
        Text(l10n.transcriptCountOf(n, count),
            key: const ValueKey('transcript-count'),
            style: t.mono.copyWith(fontSize: 11)),
        const SizedBox(width: 2),
        arrow('transcript-prev', l10n.transcriptPrevious, AxisDirection.up,
            actions.previous),
        arrow('transcript-next', l10n.transcriptNext, AxisDirection.down,
            actions.next),
      ]),
    ];
    final acts = <Widget>[
      ...switch (mode) {
        TranscriptFindMode.search => [
            button('transcript-delete', l10n.delete, one ? actions.delete : null),
            button('transcript-delete-all', l10n.transcriptDeleteAll,
                some ? actions.deleteAll : null),
            button('transcript-select-all', l10n.menuSelectAll,
                some ? actions.selectAll : null),
          ],
        TranscriptFindMode.fillers => [
            button('transcript-keep', l10n.transcriptKeep,
                one ? actions.keep : null),
            button('transcript-delete', l10n.delete, one ? actions.delete : null),
            button('transcript-delete-all', l10n.transcriptDeleteAll,
                some ? actions.deleteAll : null),
          ],
        TranscriptFindMode.pauses => [
            button('transcript-shorten', l10n.transcriptShorten,
                one ? actions.shorten : null),
            button('transcript-shorten-all', l10n.transcriptShortenAll,
                some ? actions.shortenAll : null),
            button('transcript-delete-all', l10n.transcriptDeleteAll,
                some ? actions.deleteAll : null),
          ],
        TranscriptFindMode.speakers => [
            button('transcript-select-all', l10n.menuSelectAll,
                some ? actions.selectAll : null),
            button('transcript-delete-all', l10n.transcriptDeleteAll,
                some ? actions.deleteAll : null),
          ],
      },
      // How a delete takes its words: closing the gap, or leaving it.
      Row(mainAxisSize: MainAxisSize.min, children: [
        Container(width: 1, height: 14, color: t.hairline),
        const SizedBox(width: 6),
        button('transcript-extract', l10n.transcriptExtract,
            () => actions.setLift(false),
            on: !settings.transcriptLift),
        const SizedBox(width: 2),
        button('transcript-lift', l10n.transcriptLift,
            () => actions.setLift(true),
            on: settings.transcriptLift),
      ]),
      // Replace waits behind a disclosure, as an editor's find does.
      if (mode == TranscriptFindMode.search)
        HouseButton(
          key: const ValueKey('transcript-replace-toggle'),
          small: true,
          frameless: !find.replaceOpen,
          active: find.replaceOpen,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          onPressed: find.toggleReplace,
          child: Row(mainAxisSize: MainAxisSize.min, children: [
            Caret(
                colour: t.textSecondary,
                direction: find.replaceOpen
                    ? AxisDirection.down
                    : AxisDirection.right),
            const SizedBox(width: 4),
            Text(l10n.transcriptReplace),
          ]),
        ),
    ];
    final close = LumitTooltip(
      message: l10n.close,
      child: HouseButton(
        key: const ValueKey('transcript-find-close'),
        small: true,
        frameless: true,
        padding: const EdgeInsets.symmetric(horizontal: 4),
        onPressed: actions.close,
        child:
            glyph.LumitIcon(LumitIcons.close, size: 12, colour: t.textSecondary),
      ),
    );
    final replaceRow = mode == TranscriptFindMode.search && find.replaceOpen
        ? Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Row(children: [
              Expanded(
                child: HouseTextField(
                  key: const ValueKey('transcript-replace'),
                  controller: find.replace,
                  width: double.infinity,
                  hint: l10n.transcriptReplaceHint,
                  onSubmitted: (_) => actions.replace(),
                ),
              ),
              const SizedBox(width: 4),
              button('transcript-replace-one', l10n.transcriptReplace,
                  one ? actions.replace : null),
              const SizedBox(width: 2),
              button('transcript-replace-all', l10n.transcriptReplaceAll,
                  some ? actions.replaceAll : null),
            ]),
          )
        : null;
    return Container(
      key: ValueKey<String>('transcript-find-${mode.name}'),
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      padding: const EdgeInsets.fromLTRB(8, 3, 8, 3),
      child: LayoutBuilder(builder: (context, box) {
        // One row where it fits; a tight panel puts the actions on a second
        // row, and either wraps rather than clips. The close mark keeps the
        // right edge.
        final oneRow = box.maxWidth >= 860;
        Widget wrap(List<Widget> children) => Wrap(
              spacing: 4,
              runSpacing: 4,
              crossAxisAlignment: WrapCrossAlignment.center,
              children: children,
            );
        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          mainAxisSize: MainAxisSize.min,
          children: [
            Row(children: [
              Expanded(child: wrap(oneRow ? [...controls, ...acts] : controls)),
              const SizedBox(width: 4),
              close,
            ]),
            if (!oneRow) ...[const SizedBox(height: 4), wrap(acts)],
            if (replaceRow != null) replaceRow,
          ],
        );
      }),
    );
  }


  bool _fillerOn(TranscriptFiller kind) => switch (kind) {
        TranscriptFiller.hesitation => settings.transcriptHesitations,
        TranscriptFiller.phrase => settings.transcriptPhrases,
        TranscriptFiller.repeat => settings.transcriptRepeats,
        TranscriptFiller.retake => settings.transcriptRetakes,
        TranscriptFiller.none => false,
      };

  int _countOf(TranscriptFiller kind) => model
      .fillers(
        hesitations: kind == TranscriptFiller.hesitation,
        phrases: kind == TranscriptFiller.phrase,
        repeats: kind == TranscriptFiller.repeat,
        retakes: kind == TranscriptFiller.retake,
        skipTight: settings.transcriptSkipTight,
      )
      .length;

  Widget _seconds(String key, int ms, ValueChanged<int> set) => SizedBox(
        width: 64,
        child: DragValueField(
          key: ValueKey<String>(key),
          value: ms / 1000,
          min: 0.1,
          max: 10,
          speed: 0.01,
          decimals: 1,
          suffix: ' s',
          onChanged: (v) => set((v * 1000).round()),
        ),
      );
}
