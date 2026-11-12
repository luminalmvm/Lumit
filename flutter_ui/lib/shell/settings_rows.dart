// The two shapes every settings form is built from: a named section, and a row
// inside it.
//
// They live here rather than in the Settings window because the Settings window
// is no longer the only form that uses them — Project settings asks its
// questions in the same voice, and two windows that look alike should be alike
// because they share the drawing, not because someone kept them in step.
//
// A row is its name at one edge and its control at the other, so a page's
// controls stand in one column down the far edge and the row uses the width it
// has. A section is a kicker over its rows with a rule and a little air above
// it, and no card around them.

import 'dart:math' as math;

import 'package:flutter/services.dart' show Clipboard, ClipboardData;
import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../state/ui_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../state/files.dart' if (dart.library.js_interop) '../state/files_web.dart'
    show forgetUnused, storageUsed;
import 'dialog_frame.dart' show FittedText;
import 'settings_account.dart' show askBefore;

/// A row's height, and the numbers that make it up (docs/15 §12A.4/§12A.6: a
/// dialog row is 30 under both densities). A row carrying a live readout under
/// it is taller by that line; nothing else grows.
const double settingsRowHeight = 30;

/// The inset every row and section header takes from the page's edges.
const double settingsRowPadding = 16;

/// The least a row's name keeps, the gap beside it, and what a row sets aside
/// for each button after its control.
const double settingsLabelMin = 130;
const double settingsRowGap = 12;
const double settingsActionAllowance = 80;

/// A section's kicker band: 12 above the label, its 14px line, 4 below.
const double settingsSectionHeaderHeight = 30;

/// The air above a section's rule — every section but the page's first.
const double settingsSectionGap = 6;

/// A dropdown's closed face and a value well **in a dialog**: 22, the one
/// height §12A.6 gives them under both densities. Panel wells are 20; a dialog
/// is roomier because it is read once rather than lived in.
const double settingsControlHeight = 22;

/// The dot beside a row's name when its setting differs from a new install's.
const double settingsChangedMark = 4;

/// Where a row's setting stands against a new install's: whether it differs,
/// how to put it back, and what Copy value copies. A row with nothing to go
/// back to has no [reset]; one with nothing worth copying has no [copy].
class SettingState {
  const SettingState({this.changed = false, this.reset, this.copy});
  final bool changed;
  final VoidCallback? reset;
  final String? copy;
}

/// A row's control, carrying where its setting stands, so [settingsRow] can
/// offer Reset and mark a change without each row wiring both.
class SettingControl extends StatelessWidget {
  const SettingControl({super.key, required this.setting, required this.child});
  final SettingState setting;
  final Widget child;

  @override
  Widget build(BuildContext context) => child;
}

/// A named group of rows: a quiet kicker, then the rows themselves.
///
/// [first] leaves off the rule and the air above it, which the page's opening
/// section has nothing to be separated from. [onResetPage] puts the page it
/// is on back, from the kicker's right-click, greyed unless [pageChanged].
Widget settingsSection(
  LumitTheme t,
  String title,
  List<Widget> rows, {
  bool first = false,
  VoidCallback? onResetPage,
  bool pageChanged = true,
}) =>
    Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (!first) ...[
          const SizedBox(height: settingsSectionGap),
          Container(height: 1, color: t.hairline),
        ],
        pageMenu(onResetPage, SizedBox(
          height: settingsSectionHeaderHeight,
          child: Padding(
            padding: const EdgeInsets.fromLTRB(
                settingsRowPadding, 12, settingsRowPadding, 4),
            child: Align(
              alignment: AlignmentDirectional.centerStart,
              child: Text(t.kickerCase(title), style: t.kicker),
            ),
          ),
        ), changed: () => pageChanged),
        ...rows,
      ],
    );

/// [heading] with a right-click holding Reset page, when [reset] is given.
/// [changed] is asked as the menu opens, and greys the row when it says no.
Widget pageMenu(VoidCallback? reset, Widget heading,
        {bool Function()? changed}) =>
    reset == null
        ? heading
        : _rightClick(
            (close) =>
                [resetPageRow(reset, close, enabled: changed?.call() ?? true)],
            heading);

/// [child] with a menu of [rows] on a right-click. Opened as the button
/// comes up, so a control inside with its own menu, which opens as the button
/// goes down, keeps the click to itself.
Widget _rightClick(
        List<Widget> Function(VoidCallback close) rows, Widget child) =>
    Builder(
      builder: (context) => GestureDetector(
        behavior: HitTestBehavior.translucent,
        onSecondaryTapUp: (up) => showMenuAt<void>(
          context: context,
          position: up.globalPosition,
          rows: (close) => rows(() => close(null)),
        ),
        child: child,
      ),
    );

/// The Reset page row every page heading's menu carries.
Widget resetPageRow(VoidCallback reset, VoidCallback close,
        {bool enabled = true}) =>
    MenuRow(
      key: const ValueKey('settings-menu-reset-page'),
      enabled: enabled,
      onPressed: () {
        close();
        reset();
      },
      child: MenuLabel(l10n.settingsResetPage, enabled: enabled),
    );

/// A row's right-click: Reset, greyed while it stands at the default, then
/// Copy value where there is one worth copying.
List<Widget> _settingMenu(SettingState setting, VoidCallback close) => [
      if (setting.reset case final reset?)
        MenuRow(
          key: const ValueKey('settings-menu-reset'),
          enabled: setting.changed,
          onPressed: () {
            close();
            reset();
          },
          child: MenuLabel(l10n.reset, enabled: setting.changed),
        ),
      if (setting.reset != null && setting.copy != null) const MenuDivider(),
      if (setting.copy case final copy?)
        MenuRow(
          key: const ValueKey('settings-menu-copy'),
          onPressed: () {
            close();
            Clipboard.setData(ClipboardData(text: copy));
          },
          child: MenuLabel(l10n.paramMenuCopyValue),
        ),
    ];

/// One row: what it is on the left, its control beside it.
///
/// [description] is for a line the row has to *report*, such as where the cache
/// folder is or what the last update check found. It runs the full width under
/// the row, because a path squeezed beside the name is a path nobody can read.
/// Rows with nothing to report pass an empty string and measure exactly
/// [settingsRowHeight]. It never explains the setting.
///
/// [actions] are buttons, after the control, which is then what they act on.
///
/// A [SettingControl] gives the row a right-click and, while its setting
/// differs from a new install's, the dot beside its name.
Widget settingsRow(
  LumitTheme t,
  String title,
  String description,
  Widget control, {
  List<Widget> actions = const [],
}) =>
    _withSetting(t, control, HoverFill(
      faint: true,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          ConstrainedBox(
            constraints: const BoxConstraints(minHeight: settingsRowHeight),
            child: Padding(
              padding:
                  const EdgeInsets.symmetric(horizontal: settingsRowPadding),
              child: LayoutBuilder(
                builder: (context, box) => Row(
                  children: [
                    Expanded(child: FittedText(title, t.body, lines: 2)),
                    const SizedBox(width: settingsRowGap),
                    // As wide as it wants, short of crowding the name out. A
                    // control that fills what it is given, a path well, takes
                    // all of this.
                    ConstrainedBox(
                      constraints: BoxConstraints(
                          maxWidth: math.max(
                              0,
                              box.maxWidth -
                                  settingsLabelMin -
                                  settingsRowGap -
                                  actions.length * settingsActionAllowance)),
                      child: control,
                    ),
                    for (final (i, action) in actions.indexed) ...[
                      SizedBox(width: i == 0 ? settingsRowGap : 8),
                      action,
                    ],
                  ],
                ),
              ),
            ),
          ),
          if (description.isNotEmpty)
            Padding(
              padding: const EdgeInsets.fromLTRB(
                  settingsRowPadding, 0, settingsRowPadding, 6),
              child: Text(description,
                  style: t.small.copyWith(color: t.textMuted)),
            ),
        ],
      ),
    ));

/// [row] with its setting's menu and mark, when [control] carries one.
Widget _withSetting(LumitTheme t, Widget control, Widget row) {
  if (control is! SettingControl) return row;
  final setting = control.setting;
  return _rightClick(
    (close) => _settingMenu(setting, close),
    Stack(
      children: [
        row,
        // In the gutter, level with the name's first line, so the names
        // keep one edge down the page.
        if (setting.changed)
          PositionedDirectional(
            start: (settingsRowPadding - settingsChangedMark) / 2,
            top: (settingsRowHeight - settingsChangedMark) / 2,
            child: Container(
              width: settingsChangedMark,
              height: settingsChangedMark,
              decoration:
                  BoxDecoration(color: t.accent, shape: BoxShape.circle),
            ),
          ),
      ],
    ),
  );
}

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
      l10n.settingsTangentPanels,
      l10n.settingsMidiDevices,
      l10n.settingsMidiLearn,
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
