// The After Effects import report (docs/11-AE-IMPORT.md §9).
//
// An import never fails and never stops to ask a question, so this window is
// the only place a person finds out that a blend mode had no equivalent or that
// an effect came across as an inert placeholder. It is informative, never
// blocking: the project is already open behind it, and closing it changes
// nothing.
//
// **The whole report crosses once and is held here.** It arrives as a plain
// Dart object on the one call that did the import, and every filter and every
// row is read from that object — no bridge call rides a rebuild
// (test/frb/bridge_call_budget_test.dart is the guard).
//
// The reasons are written in `l10n/engine_labels.dart` from the stable id and
// the facts the engine sends, rather than shown as the engine's English: a
// sentence built with `format!` on the other side could not be translated
// (docs/17).

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/import.dart';

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

/// The grade shown on a row and named on its filter button.
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

class _ReportBody extends StatefulWidget {
  final BridgeImportReport report;
  final VoidCallback onClose;

  const _ReportBody({required this.report, required this.onClose});

  @override
  State<_ReportBody> createState() => _ReportBodyState();
}

class _ReportBodyState extends State<_ReportBody> {
  /// Which grade is being shown, or null for all of them.
  BridgeImportOutcome? _filter;

  /// The grades that actually occur, in the spec's order, so the filter offers
  /// nothing that would answer with an empty list. `Imported` is deliberately
  /// absent from the rows themselves — what came across whole is counted, not
  /// listed — so it appears here only if the engine ever starts listing it.
  List<BridgeImportOutcome> get _grades => [
        for (final grade in BridgeImportOutcome.values)
          if (widget.report.rows.any((r) => r.outcome == grade)) grade,
      ];

  List<BridgeImportRow> get _rows => _filter == null
      ? widget.report.rows
      : [
          for (final row in widget.report.rows)
            if (row.outcome == _filter) row,
        ];

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final rows = _rows;
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
            l10n.aeSummary(
              widget.report.imported,
              widget.report.adjusted,
              widget.report.placeholders,
              widget.report.skipped,
            ),
            style: t.small.copyWith(color: t.textSecondary),
          ),
          const SizedBox(height: 10),
          if (_grades.isNotEmpty) _filters(t),
          const SizedBox(height: 8),
          Flexible(
            child: widget.report.rows.isEmpty
                ? _empty(t, l10n.aeNothingToReport)
                : rows.isEmpty
                    ? _empty(t, l10n.aeNoneOfThatKind)
                    : ListView.builder(
                        key: const ValueKey<String>('ae-report-rows'),
                        itemCount: rows.length,
                        itemBuilder: (_, i) => _row(t, rows[i]),
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

  Widget _filters(LumitTheme t) => Wrap(
        spacing: 4,
        runSpacing: 4,
        children: [
          for (final grade in <BridgeImportOutcome?>[null, ..._grades])
            HouseButton(
              key: ValueKey<String>('ae-filter-${grade?.name ?? 'all'}'),
              small: true,
              active: _filter == grade,
              onPressed: () => setState(() => _filter = grade),
              child: Text(
                grade == null ? l10n.aeFilterAll : outcomeLabel(grade),
              ),
            ),
        ],
      );

  Widget _empty(LumitTheme t, String said) => Padding(
        padding: const EdgeInsets.symmetric(vertical: 24),
        child: Text(
          said,
          textAlign: TextAlign.center,
          style: t.body.copyWith(color: t.textMuted),
        ),
      );

  Widget _row(LumitTheme t, BridgeImportRow row) => Padding(
        padding: const EdgeInsets.symmetric(vertical: 4),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.baseline,
              textBaseline: TextBaseline.alphabetic,
              children: [
                Flexible(child: Text(row.path, style: t.bodyPrimary)),
                const SizedBox(width: 6),
                Text(
                  outcomeLabel(row.outcome),
                  style: t.caption.copyWith(color: t.textMuted),
                ),
              ],
            ),
            Text(
              reasonLine(row),
              style: t.small.copyWith(color: t.textSecondary),
            ),
          ],
        ),
      );
}
