// The Project settings window (K-286).
//
// **Why it is not a page in Settings.** Everything in the Settings window is
// this machine's: the theme, the interface, the cache budgets, the keymap. None
// of it travels, and none of it is undoable. A project setting is the opposite
// on both counts — it is saved inside the `.lum`, it opens the same way on
// somebody else's machine, and changing it is an edit like any other. Those two
// kinds of value were sharing one window, distinguished only by a section
// heading reading "This project", which is a caption doing a window's job. So
// the project's own settings have their own window, reached from
// **File ▸ Project settings…**, and Settings is machine-local throughout again.
//
// It asks one question so far. That is fine: the window exists to say where the
// project's answers live, and colour management and export defaults land here
// when they are built (docs/TODO.md) rather than back in Settings.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'settings_rows.dart';

/// The coverage-sample counts on offer — the ones graphics hardware actually
/// implements, which is why this is a pick rather than a free number.
const List<int> _aaCounts = [1, 2, 4, 8];

String _aaLabel(int samples) =>
    samples <= 1 ? l10n.off : l10n.samples('$samples');

const Size _windowSize = Size(560, 300);

Future<void> showProjectSettingsFrb(
  BuildContext context,
  ProjectReference project,
) =>
    showLumitModal<void>(
      context: context,
      id: 'project-settings',
      initialSize: _windowSize,
      builder: (close) => _ProjectSettingsWindow(
        project: project,
        onClose: () => close(null),
      ),
    );

class _ProjectSettingsWindow extends StatefulWidget {
  final ProjectReference project;
  final VoidCallback onClose;
  const _ProjectSettingsWindow({required this.project, required this.onClose});

  @override
  State<_ProjectSettingsWindow> createState() => _ProjectSettingsWindowState();
}

class _ProjectSettingsWindowState extends State<_ProjectSettingsWindow> {
  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      child: SizedBox.expand(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(14, 10, 10, 10),
              child: Row(
                children: [
                  Expanded(
                      child: Text(l10n.projectSettings, style: t.bodyPrimary)),
                  HouseButton(
                    key: const ValueKey('project-settings-close'),
                    small: true,
                    // The window's only action (K-319): Enter closes it.
                    primary: true,
                    autofocus: true,
                    onPressed: widget.onClose,
                    child: Text(l10n.done),
                  ),
                ],
              ),
            ),
            Expanded(
              child: ListView(
                padding: const EdgeInsets.fromLTRB(14, 0, 14, 14),
                children: _rendering(t),
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// Rendering: the settings that change what the composition looks like, and
  /// so are the same for the preview and the export.
  List<Widget> _rendering(LumitTheme t) {
    final project = widget.project;
    final set = project.antiAliasing();
    final inUse = project.antiAliasingInUse();
    return [
      settingsSection(t, l10n.settingsGroupRendering, [
        settingsRow(
          t,
          l10n.settingsAntiAliasing,
          l10n.settingsHelpAntiAliasing,
          SizedBox(
            width: 130,
            child: BareDropdown<int>(
              key: const ValueKey('project-anti-aliasing'),
              value: set,
              options: _aaCounts,
              label: _aaLabel,
              onChanged: (n) =>
                  setState(() => project.setAntiAliasing(samples: n)),
            ),
          ),
        ),
        // Only when the card cannot manage what was asked for. A statement,
        // never a warning (docs/15-DESIGN.md): the project keeps the value its
        // author chose, and this says what is actually being drawn.
        if (inUse != set)
          settingsRow(
            t,
            l10n.settingsAntiAliasingInUse,
            l10n.settingsHelpAntiAliasingInUse,
            Text(
              _aaLabel(inUse),
              key: const ValueKey('project-anti-aliasing-in-use'),
              style: t.small,
            ),
          ),
      ]),
    ];
  }
}
