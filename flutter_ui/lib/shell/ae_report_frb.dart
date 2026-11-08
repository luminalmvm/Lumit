// The After Effects import report (docs/11-AE-IMPORT.md §9).
//
// An import never fails and never stops to ask a question, so this window is
// the only place a person finds out that a blend mode had no equivalent or that
// an effect came across as an inert placeholder. It is informative, never
// blocking: the project is already open behind it, and closing it changes
// nothing.
//
// **What was left out comes first.** The rows are sorted into what did not
// come, what came as a stand-in and what came changed, and inside each the
// rows that say the same thing are one line with a count, opened to see where.
// A project with a hundred expressions is one line about expressions, not a
// hundred.
//
// **The whole report crosses once and is held here.** It arrives as a plain
// Dart object on the one call that did the import, and every row is read from
// that object — no bridge call rides a rebuild
// (test/frb/bridge_call_budget_test.dart is the guard).
//
// The reasons are written in `l10n/engine_labels.dart` from the stable id and
// the facts the engine sends, rather than shown as the engine's English: a
// sentence built with `format!` on the other side could not be translated
// (docs/17).

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/import.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'dialog_frame.dart';
import 'question_window.dart';
import 'review_link_parts.dart' show reviewRowMenu;

/// Show the report for a finished import. Returns when the window closes.
Future<void> showAeImportReport({
  required BuildContext context,
  required BridgeImportReport report,
}) =>
    showLumitModal<void>(
      context: context,
      id: 'ae-import-report',
      initialSize: Size(560, _heightFor(report)),
      builder: (close) => _ReportBody(report: report, onClose: () => close(null)),
    );

/// As tall as what there is to say, up to where it scrolls.
double _heightFor(BridgeImportReport report) {
  final sections = reportSections(report);
  final lines = sections.fold<int>(0, (n, s) => n + s.$3.length);
  return (dialogTitleStrip +
          dialogFooterHeight +
          30 +
          sections.length * 30 +
          lines * 40)
      .clamp(180.0, 560.0);
}

/// The grade a section is headed with.
String outcomeLabel(BridgeImportOutcome outcome) => switch (outcome) {
      BridgeImportOutcome.imported => l10n.aeOutcomeImported,
      BridgeImportOutcome.adjusted => l10n.aeOutcomeAdjusted,
      BridgeImportOutcome.placeholder => l10n.aeOutcomePlaceholder,
      BridgeImportOutcome.skipped => l10n.aeOutcomeSkipped,
    };

/// The one-line reason for a row, in the reader's language where this build
/// has a sentence for it and in the engine's English where it does not.
String reasonLine(BridgeImportRow row) =>
    importReason(row.reason, {for (final a in row.args) a.name: a.value}) ??
    row.english;

/// Rows that say the same thing, and every place they say it about.
class ReportGroup {
  final String said;
  final List<String> places = [];

  ReportGroup(this.said);
}

/// The report's rows in sections, what was left out first, each section's
/// rows gathered by what they say, the most said first.
List<(BridgeImportOutcome, int, List<ReportGroup>)> reportSections(
    BridgeImportReport report) {
  const order = [
    BridgeImportOutcome.skipped,
    BridgeImportOutcome.placeholder,
    BridgeImportOutcome.adjusted,
    BridgeImportOutcome.imported,
  ];
  return [
    for (final outcome in order)
      if (report.rows.any((row) => row.outcome == outcome))
        () {
          final groups = <String, ReportGroup>{};
          var count = 0;
          for (final row in report.rows) {
            if (row.outcome != outcome) continue;
            count++;
            final said = reasonLine(row);
            groups.putIfAbsent(said, () => ReportGroup(said)).places.add(row.path);
          }
          final sorted = groups.values.toList()
            ..sort((a, b) => b.places.length.compareTo(a.places.length));
          return (outcome, count, sorted);
        }(),
  ];
}

class _ReportBody extends StatefulWidget {
  final BridgeImportReport report;
  final VoidCallback onClose;

  const _ReportBody({required this.report, required this.onClose});

  @override
  State<_ReportBody> createState() => _ReportBodyState();
}

class _ReportBodyState extends State<_ReportBody> {
  late final _sections = reportSections(widget.report);
  final _scroll = ScrollController();

  /// The lines opened to show where they apply.
  final Set<ReportGroup> _open = {};

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  void _copy(String text) =>
      unawaited(Clipboard.setData(ClipboardData(text: text)));

  /// The whole report as text, for pasting into a message.
  String get _asText => [
        for (final (outcome, _, groups) in _sections) ...[
          outcomeLabel(outcome),
          for (final group in groups) ...[
            '  ${group.said}',
            for (final place in group.places) '    $place',
          ],
        ],
      ].join('\n');

  void _menu(Offset at, ReportGroup group) {
    final several = group.places.length > 1;
    reviewRowMenu(context, at, [
      if (several)
        (
          'ae-fold',
          _open.contains(group) ? l10n.aeHidePlaces : l10n.aeShowPlaces,
          () => setState(() =>
              _open.contains(group) ? _open.remove(group) : _open.add(group)),
        ),
      if (several)
        ('ae-open-all', l10n.aeShowAllPlaces, () {
          setState(() {
            for (final (_, _, groups) in _sections) {
              _open.addAll(groups.where((g) => g.places.length > 1));
            }
          });
        }),
      if (several) null,
      ('ae-copy-line', l10n.copyLine, () => _copy(group.said)),
      (
        'ae-copy-place',
        l10n.aeCopyPlaces,
        () => _copy(group.places.join('\n')),
      ),
      ('ae-copy-all', l10n.aeCopyReport, () => _copy(_asText)),
    ]);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return DialogFrame(
      children: [
        dialogTitleBar(t,
            title: l10n.aeReportTitle,
            onClose: widget.onClose,
            keyPrefix: 'ae-report'),
        Expanded(
          child: _sections.isEmpty
              ? Center(
                  child: Text(
                    l10n.aeNothingToReport,
                    style: t.body.copyWith(color: t.textSecondary),
                  ),
                )
              : dialogScroller(
                  context,
                  t,
                  _scroll,
                  child: ListView(
                    key: const ValueKey<String>('ae-report-rows'),
                    controller: _scroll,
                    padding: const EdgeInsets.fromLTRB(
                        dialogPadding, 6, dialogPadding, dialogPadding),
                    children: [
                      for (final (outcome, count, groups) in _sections) ...[
                        _heading(t, outcome, count),
                        for (final group in groups) _group(t, group),
                      ],
                    ],
                  ),
                ),
        ),
        dialogFooter(
          t,
          keyPrefix: 'ae-report',
          summary: l10n.aeItemsImported(widget.report.imported),
          actions: [
            if (_sections.isNotEmpty)
              questionPlain(
                  'ae-report-copy', l10n.aeCopyReport, () => _copy(_asText)),
            questionFilled('ae-report-close', l10n.close, widget.onClose),
          ],
        ),
      ],
    );
  }

  Widget _heading(LumitTheme t, BridgeImportOutcome outcome, int count) =>
      Padding(
        padding: const EdgeInsets.only(top: 12, bottom: 4),
        child: Row(
          children: [
            Expanded(
              child: Text(t.kickerCase(outcomeLabel(outcome)), style: t.kicker),
            ),
            Text('$count', style: dialogMono(t)),
          ],
        ),
      );

  /// One thing the report says. Said about one place, the place sits under
  /// it; said about several, it is a line with a count that opens to list
  /// them. Every line starts at one edge, with the twirl hanging in the margin
  /// before it.
  Widget _group(LumitTheme t, ReportGroup group) {
    final quiet = t.small.copyWith(color: t.textMuted);
    final several = group.places.length > 1;
    final open = _open.contains(group);
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapDown: (press) => _menu(press.globalPosition, group),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          HoverMark(
            builder: (context, lit) => GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: !several
                  ? null
                  : () => setState(
                      () => open ? _open.remove(group) : _open.add(group)),
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: 4),
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
                          Text(group.said, style: t.body),
                          if (!several) Text(group.places.first, style: quiet),
                        ],
                      ),
                    ),
                    if (several) ...[
                      const SizedBox(width: 8),
                      Text('${group.places.length}', style: dialogMono(t)),
                    ],
                  ],
                ),
              ),
            ),
          ),
          if (open)
            Padding(
              padding:
                  const EdgeInsetsDirectional.fromSTEB(_placeInset, 0, 0, 6),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  for (final place in group.places) Text(place, style: quiet),
                ],
              ),
            ),
        ],
      ),
    );
  }
}

/// How far a line's words stand in, which is the room its twirl takes.
const double _placeInset = 18;
