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

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/import.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

/// Show the report for a finished import. Returns when the window closes.
Future<void> showAeImportReport({
  required BuildContext context,
  required BridgeImportReport report,
}) =>
    showLumitModal<void>(
      context: context,
      id: 'ae-import-report',
      initialSize: const Size(560, 460),
      builder: (close) => _ReportBody(report: report, onClose: () => close(null)),
    );

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

  /// The lines opened to show where they apply.
  final Set<ReportGroup> _open = {};

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.only(bottom: 6),
            child: Text(l10n.aeReportTitle, style: t.heading),
          ),
          Text(
            l10n.aeCameAcross(widget.report.imported),
            style: t.small.copyWith(color: t.textSecondary),
          ),
          const SizedBox(height: 10),
          Flexible(
            child: _sections.isEmpty
                ? Padding(
                    padding: const EdgeInsets.symmetric(vertical: 24),
                    child: Text(
                      l10n.aeNothingToReport,
                      textAlign: TextAlign.center,
                      style: t.body.copyWith(color: t.textMuted),
                    ),
                  )
                : ListView(
                    key: const ValueKey<String>('ae-report-rows'),
                    children: [
                      for (final (outcome, count, groups) in _sections) ...[
                        _heading(t, outcome, count),
                        for (final group in groups) _group(t, group),
                      ],
                    ],
                  ),
          ),
          const SizedBox(height: 10),
          Align(
            alignment: Alignment.centerRight,
            child: HouseButton(
              primary: true,
              autofocus: true,
              onPressed: widget.onClose,
              child: Text(l10n.close),
            ),
          ),
        ],
      ),
    );
  }

  Widget _heading(LumitTheme t, BridgeImportOutcome outcome, int count) =>
      Padding(
        padding: const EdgeInsets.only(top: 10, bottom: 4),
        child: Row(
          children: [
            Expanded(
              child: Text(
                outcomeLabel(outcome),
                style: t.small.copyWith(color: t.textSecondary),
              ),
            ),
            Text('$count', style: t.small.copyWith(color: t.textMuted)),
          ],
        ),
      );

  /// One thing the report says. Said about one place, the place sits under
  /// it; said about several, it is a line with a count that opens to list
  /// them.
  Widget _group(LumitTheme t, ReportGroup group) {
    final place = Text(
      group.places.first,
      style: t.small.copyWith(color: t.textMuted),
    );
    if (group.places.length == 1) {
      return Padding(
        padding: const EdgeInsets.fromLTRB(20, 4, 0, 4),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [Text(group.said, style: t.body), place],
        ),
      );
    }
    final open = _open.contains(group);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        HoverMark(
          builder: (context, lit) => GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () => setState(
                () => open ? _open.remove(group) : _open.add(group)),
            child: Padding(
              padding: const EdgeInsets.symmetric(vertical: 4),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  TwirlTurn(
                    open: open,
                    child: lumitIcon(
                      open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                      size: iconSize,
                      color: Color.lerp(t.textMuted, t.textPrimary, lit)!,
                    ),
                  ),
                  const SizedBox(width: 4),
                  Expanded(child: Text(group.said, style: t.body)),
                  const SizedBox(width: 8),
                  Text(
                    '${group.places.length}',
                    style: t.small.copyWith(color: t.textMuted),
                  ),
                ],
              ),
            ),
          ),
        ),
        if (open)
          Padding(
            padding: const EdgeInsets.fromLTRB(20, 0, 0, 6),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                for (final place in group.places)
                  Text(place, style: t.small.copyWith(color: t.textMuted)),
              ],
            ),
          ),
      ],
    );
  }
}
