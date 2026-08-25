// The two shapes every settings form is built from: a named section, and a row
// inside it.
//
// They live here rather than in the Settings window because the Settings window
// is no longer the only form that uses them — Project settings (K-286) asks its
// questions in the same voice, and two windows that look alike should be alike
// because they share the drawing, not because someone kept them in step.
//
// **The drawing decides the shape** (K-465). A row is a label in a fixed 190px
// column, a 12px gap, and its control at the *start* of what is left — not
// pushed to the right edge, which is what these rows used to do. A section is a
// kicker over its rows with a rule and a little air above it, and no card
// around them: the drawing separates sections with a line, not with a box.

import 'package:flutter/widgets.dart';

import '../theme/theme.dart';

/// A row's height, and the numbers that make it up (docs/15 §12A.4/§12A.6: a
/// dialog row is 30 under both densities). A row carrying a live readout under
/// it is taller by that line; nothing else grows.
const double settingsRowHeight = 30;

/// The inset every row and section header takes from the page's edges.
const double settingsRowPadding = 16;

/// The label column, and the gap between it and the control beside it.
const double settingsLabelColumn = 190;
const double settingsRowGap = 12;

/// A section's kicker band: 12 above the label, its 14px line, 4 below.
const double settingsSectionHeaderHeight = 30;

/// The air above a section's rule — every section but the page's first.
const double settingsSectionGap = 6;

/// A dropdown's closed face and a value well **in a dialog**: 22, the one
/// height §12A.6 gives them under both densities. Panel wells are 20; a dialog
/// is roomier because it is read once rather than lived in.
const double settingsControlHeight = 22;

/// A named group of rows: a quiet kicker, then the rows themselves.
///
/// [first] leaves off the rule and the air above it, which the page's opening
/// section has nothing to be separated from.
Widget settingsSection(
  LumitTheme t,
  String title,
  List<Widget> rows, {
  bool first = false,
}) =>
    Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (!first) ...[
          const SizedBox(height: settingsSectionGap),
          Container(height: 1, color: t.hairline),
        ],
        SizedBox(
          height: settingsSectionHeaderHeight,
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
                settingsRowPadding, 12, settingsRowPadding, 4),
            child: Align(
              alignment: Alignment.centerLeft,
              child: Text(title.toUpperCase(), style: t.kicker),
            ),
          ),
        ),
        ...rows,
      ],
    );

/// One row: what it is on the left, its control beside it.
///
/// [description] is for a line the row has to *report* — where the cache
/// folder is, what the last update check found — and it runs the full width
/// under the row rather than wrapping inside the label column, because a path
/// squeezed into 190px is a path nobody can read. Rows with nothing to report
/// pass an empty string and measure exactly [settingsRowHeight]; the help
/// sentences these rows used to carry are gone with the drawing that had no
/// room for them.
Widget settingsRow(
  LumitTheme t,
  String title,
  String description,
  Widget control,
) =>
    Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        ConstrainedBox(
          constraints: const BoxConstraints(minHeight: settingsRowHeight),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: settingsRowPadding),
            child: Row(
              children: [
                SizedBox(
                  width: settingsLabelColumn,
                  child: Text(title, style: t.body),
                ),
                const SizedBox(width: settingsRowGap),
                Expanded(
                  child: Align(alignment: Alignment.centerLeft, child: control),
                ),
              ],
            ),
          ),
        ),
        if (description.isNotEmpty)
          Padding(
            padding: const EdgeInsets.fromLTRB(
                settingsRowPadding, 0, settingsRowPadding, 6),
            child:
                Text(description, style: t.small.copyWith(color: t.textMuted)),
          ),
      ],
    );
