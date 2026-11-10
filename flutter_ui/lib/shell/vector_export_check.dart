// The Lottie and SVG window's check: what won't come across, grouped by
// what happens to it, lines that say the same thing gathered with a count,
// each place clickable to select its layer. The import report's grammar
// (ae_report_frb.dart), with the places live.

part of 'vector_export_frb.dart';

/// Lines that say the same thing, and every issue they say it about.
class _Line {
  final String said;

  /// The players that miss it, for a Some players line. Empty otherwise.
  final String players;
  final List<BridgeVectorIssue> issues = [];

  _Line(this.said, this.players);

  String get key => '$said\u0000$players';
}

/// The sentence for an issue: in the reader's language where this build has
/// one for the id, and the engine's English where it does not.
String issueLine(BridgeVectorIssue issue) =>
    vectorReason(issue.reason, {for (final a in issue.args) a.name: a.value},
        somePlayers: issue.outcome == 'some_players') ??
    issue.english;

/// Where an issue is, for the reader: comp, layer, then the part.
String issuePlace(BridgeVectorIssue issue) => issue.place.join(' › ');

/// The issues in the three sections, in the window's order, each section's
/// lines gathered by what they say, the most said first.
List<(String, String, List<_Line>)> checkSections(
    Iterable<BridgeVectorIssue> issues) {
  const order = [
    ('left_out', 'vector-left-out'),
    ('some_players', 'vector-some-players'),
    ('changed', 'vector-changed'),
  ];
  return [
    for (final (outcome, key) in order)
      if (issues.any((i) => i.outcome == outcome))
        () {
          final lines = <String, _Line>{};
          for (final issue in issues) {
            if (issue.outcome != outcome) continue;
            final said = issueLine(issue);
            final players = issue.players.map(playerName).join(', ');
            final line = _Line(said, players);
            lines.putIfAbsent(line.key, () => line).issues.add(issue);
          }
          final sorted = lines.values.toList()
            ..sort((a, b) => b.issues.length.compareTo(a.issues.length));
          return (outcome, key, sorted);
        }(),
  ];
}

String _outcomeLabel(String outcome) => switch (outcome) {
      'left_out' => l10n.vectorOutcomeLeftOut,
      'some_players' => l10n.vectorOutcomeSomePlayers,
      _ => l10n.vectorOutcomeChanged,
    };

/// How far a line's words stand in, which is the room its twirl takes.
const double _placeInset = 18;

extension _Check on _VectorWindowState {
  /// Every issue the window shows: the front comp's, then the ticked ones'.
  Iterable<BridgeVectorIssue> get _allIssues => [
        ...?_preview?.issues,
        for (final row in _comps)
          if (_ticked.contains(row.comp.internalid))
            ...?_others[row.comp.internalid],
      ];

  List<Widget> _checkLines(BuildContext context, LumitTheme t) {
    if (_preview == null) return const [];
    final sections = checkSections(_allIssues);
    if (sections.isEmpty) {
      return [
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 6),
          child: Text(
            l10n.vectorNothingLeftOut,
            key: const ValueKey<String>('vector-check-clear'),
            style: t.body.copyWith(color: t.textSecondary),
          ),
        ),
      ];
    }
    return [
      for (final (outcome, key, lines) in sections) ...[
        Padding(
          key: ValueKey<String>(key),
          padding: const EdgeInsets.only(top: 8, bottom: 2),
          child: Row(children: [
            Expanded(
                child: Text(t.kickerCase(_outcomeLabel(outcome)),
                    style: t.kicker)),
            Text('${lines.fold<int>(0, (n, l) => n + l.issues.length)}',
                style: dialogMono(t)),
          ]),
        ),
        for (final line in lines) _checkLine(context, t, line),
      ],
    ];
  }

  /// What the check says about every line, as text for pasting.
  String get _checkText => [
        for (final (outcome, _, lines) in checkSections(_allIssues)) ...[
          _outcomeLabel(outcome),
          for (final line in lines) ...[
            '  ${line.said}${line.players.isEmpty ? '' : ' · ${line.players}'}',
            for (final issue in line.issues) '    ${issuePlace(issue)}',
          ],
        ],
      ].join('\n');

  void _copyText(String text) =>
      unawaited(Clipboard.setData(ClipboardData(text: text)));

  void _lineMenu(BuildContext context, Offset at, _Line line) {
    final several = line.issues.length > 1;
    final open = _openLines.contains(line.key);
    reviewRowMenu(context, at, [
      if (several)
        (
          'vector-fold',
          open ? l10n.aeHidePlaces : l10n.aeShowPlaces,
          () => _redraw(() =>
              open ? _openLines.remove(line.key) : _openLines.add(line.key)),
        ),
      if (several) null,
      ('vector-copy-line', l10n.copyLine, () => _copyText(line.said)),
      (
        'vector-copy-places',
        l10n.aeCopyPlaces,
        () => _copyText(line.issues.map(issuePlace).join('\n')),
      ),
      ('vector-copy-check', l10n.vectorCopyCheck, () => _copyText(_checkText)),
    ]);
  }

  /// Select what an issue is about: its comp, its layer, and for an effect
  /// the effect's own card. The window stays where it is.
  void _goTo(BridgeVectorIssue issue) {
    final ui = widget.ui;
    ui.setSelectedComp(issue.comp);
    final layer = issue.layer;
    if (layer == null) return;
    ui.setSelection([layer]);
    if (issue.reason != 'effect' && issue.reason != 'style') return;
    final label = issue.args
        .where((a) => a.name == 'label')
        .map((a) => a.value)
        .firstOrNull;
    if (label == null) return;
    for (final effect in layer.getEffects()) {
      final info = effect.getInfo();
      if ((info.customName ?? effectLabelOf(info.name)) == label) {
        ui.revealEffectCard(layer, info.id);
        return;
      }
    }
  }

  /// One thing the check says. Said about one place, the place sits under it
  /// as a link; said about several, it is a line with a count that opens to
  /// list them.
  Widget _checkLine(BuildContext context, LumitTheme t, _Line line) {
    final several = line.issues.length > 1;
    final open = _openLines.contains(line.key);
    return GestureDetector(
      key: ValueKey<String>('vector-line-${line.key.hashCode}'),
      behavior: HitTestBehavior.opaque,
      onSecondaryTapDown: (press) =>
          _lineMenu(context, press.globalPosition, line),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          HoverMark(
            builder: (context, lit) => GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: !several
                  ? null
                  : () => _redraw(() => open
                      ? _openLines.remove(line.key)
                      : _openLines.add(line.key)),
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: 3),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    SizedBox(
                      width: _placeInset,
                      child: !several
                          ? null
                          : Align(
                              alignment: AlignmentDirectional.centerStart,
                              child: TwirlTurn(
                                open: open,
                                child: lumitIcon(
                                  open
                                      ? LumitIcon.twirlOpen
                                      : LumitIcon.twirlClosed,
                                  size: iconSize,
                                  color: Color.lerp(
                                      t.textMuted, t.textPrimary, lit)!,
                                ),
                              ),
                            ),
                    ),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text.rich(
                            TextSpan(children: [
                              TextSpan(text: line.said),
                              if (line.players.isNotEmpty)
                                TextSpan(
                                  text: ' · ${line.players}',
                                  style: TextStyle(color: t.textMuted),
                                ),
                            ]),
                            style: t.body,
                          ),
                          if (!several) _place(t, line.issues.first),
                        ],
                      ),
                    ),
                    if (several) ...[
                      const SizedBox(width: 8),
                      Text('${line.issues.length}', style: dialogMono(t)),
                    ],
                  ],
                ),
              ),
            ),
          ),
          if (open)
            Padding(
              padding:
                  const EdgeInsetsDirectional.fromSTEB(_placeInset, 0, 0, 4),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [for (final issue in line.issues) _place(t, issue)],
              ),
            ),
        ],
      ),
    );
  }

  /// A place, which selects what it names when clicked.
  Widget _place(LumitTheme t, BridgeVectorIssue issue) => HoverMark(
        builder: (context, lit) => GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: () => _goTo(issue),
          child: Text(
            issuePlace(issue),
            style: t.small.copyWith(
                color: Color.lerp(t.textMuted, t.textPrimary, lit)),
          ),
        ),
      );
}
