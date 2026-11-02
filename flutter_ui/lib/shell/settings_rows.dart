// The two shapes every settings form is built from: a named section, and a row
// inside it.
//
// They live here rather than in the Settings window because the Settings window
// is no longer the only form that uses them — Project settings asks its
// questions in the same voice, and two windows that look alike should be alike
// because they share the drawing, not because someone kept them in step.
//
// **The drawing decides the shape**. A row is a label in a fixed 190px
// column, a 12px gap, and its control at the *start* of what is left — not
// pushed to the right edge, which is what these rows used to do. A section is a
// kicker over its rows with a rule and a little air above it, and no card
// around them: the drawing separates sections with a line, not with a box.
//
// The one exception is a button. A row with something to report and a button
// to change it (Settings ▸ Account) gives the button as `actions`, which sit
// at the row's end, so a page's buttons line up in one column whatever the
// values beside them say.

import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../state/files.dart' if (dart.library.js_interop) '../state/files_web.dart'
    show forgetUnused, storageUsed;
import 'settings_account.dart' show askBefore;

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
              alignment: AlignmentDirectional.centerStart,
              child: Text(t.kickerCase(title), style: t.kicker),
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
///
/// [actions] are buttons, at the row's end. [control] is then what they act
/// on, a value read off the account or a field, and stays at the start.
Widget settingsRow(
  LumitTheme t,
  String title,
  String description,
  Widget control, {
  List<Widget> actions = const [],
}) =>
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
                  child: Align(alignment: AlignmentDirectional.centerStart, child: control),
                ),
                for (final (i, action) in actions.indexed) ...[
                  SizedBox(width: i == 0 ? settingsRowGap : 8),
                  action,
                ],
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

/// A size in bytes, in megabytes or, from one up, gigabytes.
String byteSize(int bytes) {
  final b = bytes.toDouble();
  if (b >= 1 << 30) {
    return '${(b / (1 << 30)).toStringAsFixed(1)} ${l10n.unitGb}';
  }
  return '${(b / (1 << 20)).toStringAsFixed(0)} ${l10n.unitMb}';
}

/// How much the browser keeps for Lumit, once it has said, and a Clear that
/// forgets the files no recent project uses.
Widget storageKept(LumitTheme t) => StatefulBuilder(
      builder: (context, setState) => Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          FutureBuilder<int?>(
            future: storageUsed(),
            builder: (context, used) => Text(
                switch (used.data) {
                  null => '',
                  // To a tenth, since settings and a few projects come to
                  // less than one.
                  final bytes when bytes < 1 << 30 =>
                    '${(bytes / (1 << 20)).toStringAsFixed(1)} ${l10n.unitMb}',
                  final bytes => byteSize(bytes),
                },
                style: t.body),
          ),
          const SizedBox(width: 8),
          HouseButton(
            key: const ValueKey('settings-browser-forget'),
            small: true,
            onPressed: () async {
              final recent = Provider.of<LumitUiState>(context, listen: false)
                  .workspace
                  .recentProjects;
              if (!await askBefore(context, l10n.settingsBrowserForget,
                  l10n.settingsBrowserForgetYes)) {
                return;
              }
              await forgetUnused(recent);
              setState(() {});
            },
            child: Text(l10n.clear, style: t.small),
          ),
        ],
      ),
    );

/// A size in bytes as a whole number of megabytes, without the unit.
String mibText(int bytes) => (bytes / (1 << 20)).toStringAsFixed(0);

/// The settings the browser build leaves out, by name. Each needs an account,
/// a server, another program or the machine's own folders, and a page has
/// none of them.
Set<String> get settingsNotInPage => {
      l10n.settingsShareUpLimit,
      l10n.settingsShareDownLimit,
      l10n.settingsKeepRunning,
      l10n.settingsOpenAtSignIn,
      l10n.settingsDiscordStatus,
      l10n.settingsLiveEndpoint,
      l10n.settingsScriptsFolder,
      l10n.settingsScriptAfterExport,
      l10n.settingsScriptAfterOpen,
      l10n.settingsScriptAfterSave,
      l10n.settingsAutomaticUpdates,
      l10n.settingsNightlyBuilds,
      l10n.settingsNightlyIcon,
      l10n.settingsThisVersion,
      l10n.settingsOwnTitleBar,
      l10n.settingsPluginsOnCard,
      l10n.settingsWhere,
      l10n.settingsAppliesTo,
      l10n.settingsStylesFolder,
      l10n.settingsIconsFolder,
    };
