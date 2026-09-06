// The Project settings window.
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
// It asks two questions so far. That is fine: the window exists to say where
// the project's answers live, and the export defaults land here when they are
// built (docs/TODO.md) rather than back in Settings.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/colour.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';

import '../l10n/engine_labels.dart' show colourProblem;
import '../l10n/strings.dart';
import '../state/file_dialogs.dart' show pickOcioConfig;
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'settings_rows.dart';

/// The coverage-sample counts on offer — the ones graphics hardware actually
/// implements, which is why this is a pick rather than a free number.
const List<int> _aaCounts = [1, 2, 4, 8];

String _aaLabel(int samples) =>
    samples <= 1 ? l10n.off : l10n.samples('$samples');

const Size _windowSize = Size(560, 420);

Future<void> showProjectSettingsFrb(
  BuildContext context,
  ProjectReference project, {
  /// Where the colour config is chosen. The real file dialogue by default; a
  /// widget test hands its own, because a plugin channel cannot open one.
  Future<String?> Function()? configPicker,
}) =>
    showLumitModal<void>(
      context: context,
      id: 'project-settings',
      initialSize: _windowSize,
      builder: (close) => _ProjectSettingsWindow(
        project: project,
        configPicker: configPicker ?? pickOcioConfig,
        onClose: () => close(null),
      ),
    );

class _ProjectSettingsWindow extends StatefulWidget {
  final ProjectReference project;
  final Future<String?> Function() configPicker;
  final VoidCallback onClose;
  const _ProjectSettingsWindow({
    required this.project,
    required this.configPicker,
    required this.onClose,
  });

  @override
  State<_ProjectSettingsWindow> createState() => _ProjectSettingsWindowState();
}

class _ProjectSettingsWindowState extends State<_ProjectSettingsWindow> {
  /// The colour config as the engine reads it. Held, and re-read only when
  /// this window has just changed it: `colourSummary` opens the config file to
  /// see whether it has changed on disk, which is not work for a `build`
  /// (docs/impl/ocio.md §6.1).
  late BridgeColourSummary _colour = _readColour();

  BridgeColourSummary _readColour() {
    try {
      return widget.project.colourSummary();
    } catch (_) {
      return const BridgeColourSummary(
        path: '',
        loaded: false,
        problem: '',
        problemArgs: [],
        problemEnglish: '',
        spaces: [],
        displays: [],
        looks: [],
        name: '',
        workingFromConfig: false,
        workingSpace: '',
      );
    }
  }

  /// Point the project at a config, or at none. An ordinary op, so it is one
  /// undo step and it travels in the `.lum`.
  void _setConfig(String? path) {
    widget.project.setColourConfig(path: path);
    setState(() => _colour = _readColour());
  }

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
                    // The window's only action: Enter closes it.
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
                children: [..._rendering(t), ..._colourGroup(t)],
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

  /// **Colour management** (docs/impl/ocio.md §6.4). The project points at
  /// one OCIO config file; its names then fill the Viewer's picker, the
  /// export's colour space and each footage item's interpretation. It is the
  /// project's property rather than the machine's for the same reason
  /// anti-aliasing is: it changes what the comp looks like, so it travels.
  ///
  /// The state line is where a config that is not in force says why, in one
  /// calm sentence and in the words the engine sent an id for. Nothing else
  /// happens: the picture keeps rendering through the built-in family and
  /// every name the project has stored is kept (§3.3). Choosing again is how a
  /// config that moved is relinked — the same gesture as the first choice.
  List<Widget> _colourGroup(LumitTheme t) => [
        settingsSection(t, l10n.settingsGroupColour, [
          settingsRow(
            t,
            l10n.projectColourConfig,
            _colourState,
            Row(children: [
              // The path takes whatever the row has left: a config lives at a
              // long path, and a fixed well would push its two buttons off the
              // window rather than shorten the text.
              Expanded(
                child:
                    _well(t, _colour.path.isEmpty ? l10n.none : _colour.path),
              ),
              const SizedBox(width: 6),
              HouseButton(
                key: const ValueKey('project-colour-config-choose'),
                small: true,
                onPressed: () async {
                  final path = await widget.configPicker();
                  if (path == null || !mounted) return;
                  _setConfig(path);
                },
                child: Text(l10n.chooseEllipsis),
              ),
              const SizedBox(width: 6),
              HouseButton(
                key: const ValueKey('project-colour-config-clear'),
                small: true,
                onPressed: _colour.path.isEmpty ? null : () => _setConfig(null),
                child: Text(l10n.clear),
              ),
            ]),
          ),
          // Which primaries the compositing maths is done in: Lumit's own
          // linear Rec. 709, or the config's scene-linear space. The second
          // needs a loaded config to mean anything, so it is offered only
          // then; the setting itself is the project's and survives either way.
          settingsRow(
            t,
            l10n.projectColourWorkingSpace,
            '',
            BareDropdown<int>(
              key: const ValueKey('project-colour-working-space'),
              value: _colour.workingFromConfig ? 1 : 0,
              options: const [0, 1],
              label: (i) => i == 0
                  ? l10n.projectColourWorkingSpaceValue
                  : l10n.projectColourWorkingSpaceConfig(
                      _colour.workingSpace.isEmpty
                          ? l10n.none
                          : _colour.workingSpace),
              onChanged: _colour.loaded || _colour.workingFromConfig
                  ? (i) {
                      widget.project.setColourWorkingSpace(fromConfig: i == 1);
                      setState(() => _colour = _readColour());
                    }
                  : null,
            ),
          ),
        ]),
      ];

  /// The line under the config row: what was loaded, or why nothing was.
  String get _colourState {
    if (_colour.path.isEmpty) return l10n.projectColourNoConfig;
    if (_colour.loaded) {
      return l10n.projectColourLoaded(
        '${_colour.spaces.length}',
        '${_colour.displays.length}',
      );
    }
    final why = colourProblem(_colour.problem, {
          for (final arg in _colour.problemArgs) arg.name: arg.value,
        }) ??
        _colour.problemEnglish;
    return l10n.projectColourNotInForce(why);
  }

  /// A recessed box holding something the user did not type — here, the path
  /// the project names.
  Widget _well(LumitTheme t, String text) => Container(
        key: const ValueKey('project-colour-config-path'),
        height: settingsControlHeight,
        alignment: Alignment.centerLeft,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        decoration: BoxDecoration(
          color: t.surface0,
          border: Border.all(color: t.hairline),
          borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        ),
        child: Text(
          text,
          style: t.mono.copyWith(fontSize: 11),
          overflow: TextOverflow.ellipsis,
        ),
      );
}
