// The Settings window, ported from shell/settings.rs (docs/07-UI-SPEC §15):
// a modal with a sidebar of pages, each page a column of grouped cards of
// rows — label left, control right. Fixed 680×420 body; always opens on
// Appearance; long pages scroll inside.

import 'package:flutter/widgets.dart';

import '../state/app_state.dart';
import '../state/settings.dart';
import '../state/workspace.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';

enum SettingsPage { general, appearance, interface, performance, export }

extension on SettingsPage {
  String get title => switch (this) {
        SettingsPage.general => 'General',
        SettingsPage.appearance => 'Appearance',
        SettingsPage.interface => 'Interface',
        SettingsPage.performance => 'Performance',
        SettingsPage.export => 'Export',
      };
}

const _settingsWidth = 680.0;
const _settingsBodyHeight = 420.0;
const _sidebarWidth = 150.0;

class SettingsWindow extends StatefulWidget {
  final Workspace workspace;
  final AppStateStub app;
  final VoidCallback onClose;

  const SettingsWindow({
    super.key,
    required this.workspace,
    required this.app,
    required this.onClose,
  });

  @override
  State<SettingsWindow> createState() => _SettingsWindowState();
}

class _SettingsWindowState extends State<SettingsWindow> {
  // Opens on General by owner request (2026-07-21) — a recorded deviation
  // from the egui window, which always opens on Appearance.
  SettingsPage _page = SettingsPage.general;

  Workspace get ws => widget.workspace;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // A true modal: dimmed backdrop eats clicks; clicking it closes.
    return Stack(
      children: [
        Positioned.fill(
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: widget.onClose,
            child: Container(color: t.modalBackdrop),
          ),
        ),
        Center(
          child: GestureDetector(
            onTap: () {}, // swallow clicks inside the dialog
            child: Container(
              width: _settingsWidth,
              decoration: BoxDecoration(
                color: t.surface3,
                borderRadius: BorderRadius.circular(t.tokens.floatRadius),
                border: Border.all(color: t.hairline),
                boxShadow: t.floatShadow,
              ),
              padding: const EdgeInsets.all(12),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    children: [
                      Text('Settings', style: t.heading),
                      const Spacer(),
                      HouseButton(
                        onPressed: widget.onClose,
                        child: const Text('Done'),
                      ),
                    ],
                  ),
                  const SizedBox(height: 8),
                  Container(height: 1, color: t.hairline),
                  const SizedBox(height: 8),
                  SizedBox(
                    height: _settingsBodyHeight,
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        SizedBox(width: _sidebarWidth, child: _sidebar(t)),
                        Container(width: 1, color: t.hairline),
                        const SizedBox(width: 12),
                        Expanded(
                          child: SingleChildScrollView(child: _body(t)),
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ],
    );
  }

  Widget _sidebar(LumitTheme t) => Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const SizedBox(height: 4),
          for (final page in SettingsPage.values)
            MenuRow(
              selected: _page == page,
              onPressed: () => setState(() => _page = page),
              child: Text(
                page.title,
                style: _page == page
                    ? t.bodyPrimary
                    : t.body.copyWith(color: t.textSecondary),
              ),
            ),
        ],
      );

  Widget _body(LumitTheme t) => switch (_page) {
        SettingsPage.general => _general(t),
        SettingsPage.appearance => _appearance(t),
        SettingsPage.interface => _interface(t),
        SettingsPage.performance => _performance(t),
        SettingsPage.export => _export(t),
      };

  // --- General -------------------------------------------------------------

  Widget _general(LumitTheme t) => _Page(
        title: 'General',
        groups: [
          _Group(title: 'Workspace', rows: [
            _Row(
              label: 'Panel layout',
              hint: 'Return every panel to its default place and size.',
              control: HouseButton(
                onPressed: () {
                  ws.resetWorkspaceLayout();
                  widget.app.setNotice('workspace reset');
                },
                child: const Text('Reset workspace'),
              ),
            ),
          ]),
          _Group(title: 'Autosave', rows: [
            _Row(
              label: 'Every',
              hint: 'Minutes between automatic saves of a saved project.',
              control: Row(mainAxisSize: MainAxisSize.min, children: [
                Text('min', style: t.small),
                const SizedBox(width: 6),
                DragValueField(
                  value: ws.autosave.intervalMins,
                  min: 1,
                  max: 60,
                  onChanged: (v) => setState(() {
                    ws.autosave.intervalMins = v.round();
                    ws.touch();
                  }),
                ),
              ]),
            ),
            _Row(
              label: 'Copies kept',
              hint: 'How many timestamped backups to keep.',
              control: DragValueField(
                value: ws.autosave.keep,
                min: 1,
                max: 50,
                onChanged: (v) => setState(() {
                  ws.autosave.keep = v.round();
                  ws.touch();
                }),
              ),
            ),
          ]),
          _Group(title: 'About', rows: [
            _Row(
              label: 'Version',
              control:
                  Text('0.1.0 (Flutter frontend)', style: t.body),
            ),
          ]),
        ],
      );

  // --- Appearance ----------------------------------------------------------

  Widget _appearance(LumitTheme t) => _Page(
        title: 'Appearance',
        groups: [
          _Group(title: 'Theme', rows: [
            _Row(
              label: 'Colour scheme',
              hint: 'The whole palette — light, dark, and community themes.',
              control: BareDropdown<LumitColorScheme>(
                value: ws.colorScheme,
                options: LumitColorScheme.values,
                label: (s) => s.label,
                onChanged: (s) => setState(() => ws.setScheme(s)),
              ),
            ),
            _Row(
              label: 'Accent',
              hint: 'The single highlight colour.',
              control: Row(mainAxisSize: MainAxisSize.min, children: [
                if (ws.accentOverride != null)
                  HouseButton(
                    small: true,
                    onPressed: () => setState(() => ws.setAccent(null)),
                    child: const Text('Reset'),
                  ),
                const SizedBox(width: 6),
                _AccentButton(workspace: ws, onPicked: () => setState(() {})),
              ]),
            ),
          ]),
          _Group(title: 'Shape and motion', rows: [
            _Row(
              label: 'Panel shape',
              hint: 'Sharp edge-to-edge, or rounded floating cards.',
              control: BareDropdown<ThemeShape>(
                value: ws.themeShape,
                options: ThemeShape.values,
                label: (s) => s == ThemeShape.sharp ? 'Sharp' : 'Round',
                onChanged: (s) => setState(() => ws.setShape(s)),
              ),
            ),
            _Row(
              label: 'Interface motion',
              hint: 'How much the chrome animates.',
              control: BareDropdown<AnimationLevel>(
                value: ws.animationLevel,
                options: AnimationLevel.values,
                label: (a) => switch (a) {
                  AnimationLevel.all => 'All',
                  AnimationLevel.minimal => 'Minimal',
                  AnimationLevel.none => 'None',
                },
                onChanged: (a) => setState(() => ws.setAnimationLevel(a)),
              ),
            ),
          ]),
        ],
      );

  // --- Interface -----------------------------------------------------------

  Widget _interface(LumitTheme t) => _Page(
        title: 'Interface',
        groups: [
          _Group(title: 'Display', rows: [
            _Row(
              label: 'UI scale',
              hint:
                  "How large Lumit's interface draws relative to your display's native scale.",
              control: HouseSlider(
                value: ws.interface.uiScale,
                min: 0.75,
                max: 2.0,
                step: 0.05,
                suffix: '×',
                commitOnRelease: true,
                onChanged: (v) => setState(() {
                  ws.interface.uiScale = v;
                  ws.touch();
                }),
              ),
            ),
            _Row(
              label: 'Show tooltips',
              hint: 'Show hover tooltips throughout the app.',
              control: HouseCheckbox(
                value: ws.interface.showTooltips,
                onChanged: (v) => setState(() {
                  ws.interface.showTooltips = v;
                  ws.touch();
                }),
              ),
            ),
          ]),
        ],
      );

  // --- Performance ---------------------------------------------------------

  Widget _performance(LumitTheme t) => _Page(
        title: 'Performance',
        groups: [
          _Group(title: 'Frame cache', rows: [
            _Row(
              label: 'Memory budget',
              hint:
                  "The one cap on everything Lumit caches in RAM: rendered frames, decoded video and decoded audio together. Defaults to half the machine's memory.",
              control: _mb(ws.performance.ramBudgetMb, 2048, 1048576,
                  (v) => ws.performance.ramBudgetMb = v),
            ),
            _Row(
              label: 'Disk budget',
              hint: 'Cap on the on-disk frame cache (.lum-cache).',
              control: _mb(ws.performance.diskCacheMb, 0, 1048576,
                  (v) => ws.performance.diskCacheMb = v),
            ),
            _Row(
              label: 'Video memory budget',
              hint: 'How much VRAM the displayed-frame cache may hold.',
              control: _mb(ws.performance.vramCacheMb, 128, 16384,
                  (v) => ws.performance.vramCacheMb = v),
            ),
          ]),
          _Group(title: 'Cache', rows: [
            _Row(
              label: 'Clear cache',
              hint: 'Empty the RAM and video-memory frame caches now.',
              control: HouseButton(
                onPressed: () => widget.app.engine('Clear cache'),
                child: const Text('Clear cache'),
              ),
            ),
            _Row(
              label: 'Background fill',
              hint:
                  'Decode ahead around the playhead while idle, so scrubbing hits a warm cache.',
              control: HouseCheckbox(
                value: ws.performance.backgroundFill,
                onChanged: (v) => setState(() {
                  ws.performance.backgroundFill = v;
                  ws.touch();
                }),
              ),
            ),
            _Row(
              label: 'Cache root folder',
              hint:
                  'Where the on-disk frame cache is stored. Choosing a folder moves new project caches there instead of next to the project file.',
              control: Row(mainAxisSize: MainAxisSize.min, children: [
                if (ws.performance.cacheRoot != null)
                  HouseButton(
                    small: true,
                    onPressed: () => setState(() {
                      ws.performance.cacheRoot = null;
                      ws.touch();
                    }),
                    child: const Text('Use default'),
                  ),
                const SizedBox(width: 6),
                HouseButton(
                  onPressed: () =>
                      widget.app.engine('Choose cache root folder'),
                  child: const Text('Choose…'),
                ),
                const SizedBox(width: 6),
                ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 150),
                  child: Text(
                    ws.performance.cacheRoot ??
                        'Default (next to the project file)',
                    style: t.small,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ]),
            ),
          ]),
        ],
      );

  Widget _mb(int value, int min, int max, void Function(int) set) {
    final t = ws.theme;
    return Row(mainAxisSize: MainAxisSize.min, children: [
      Text('MB', style: t.small),
      const SizedBox(width: 6),
      DragValueField(
        value: value,
        min: min,
        max: max,
        speed: 64,
        onChanged: (v) => setState(() {
          set(v.round());
          ws.touch();
        }),
      ),
    ]);
  }

  // --- Export --------------------------------------------------------------

  Widget _export(LumitTheme t) => _Page(
        title: 'Export',
        groups: [
          _Group(title: 'Defaults', rows: [
            _Row(
              label: 'Default preset',
              hint:
                  'The preset a plain "Export…" action stamps. Picking a specific preset from the Export preset menu always uses that preset instead.',
              control: BareDropdown<ExportPreset>(
                value: ws.export.defaultPreset,
                options: ExportPreset.values,
                label: (p) => p.label,
                onChanged: (p) => setState(() {
                  ws.export.defaultPreset = p;
                  ws.touch();
                }),
              ),
            ),
            _Row(
              label: 'Filename template',
              hint:
                  "{comp}, {preset}, and {date} stand for the composition name, the preset's file name, and today's date (YYYY-MM-DD). Leave blank to use each preset's own default file name.",
              control: _TemplateField(workspace: ws),
            ),
          ]),
        ],
      );
}

/// A single current-accent swatch that opens the HSV colour picker seeded
/// with the live accent. The old eight quick swatches now live inside the
/// picker as a preset row, so the settings row stays to one control.
class _AccentButton extends StatelessWidget {
  final Workspace workspace;
  final VoidCallback onPicked;
  const _AccentButton({required this.workspace, required this.onPicked});

  /// The theme roles that seeded the old quick swatches, offered as presets.
  List<Color> _presets(LumitTheme t) => [
        LumitTheme.defaultAccent,
        t.success,
        t.warning,
        t.error,
        t.cacheDisk,
        t.curve[0],
        t.curve[2],
        t.curve[3],
      ];

  @override
  Widget build(BuildContext context) {
    final t = workspace.theme;
    return GestureDetector(
      key: const Key('accent-swatch'),
      onTap: () async {
        final box = context.findRenderObject()! as RenderBox;
        final origin = box.localToGlobal(Offset.zero);
        final picked = await showColourPicker(
          context: context,
          position: origin + Offset(0, box.size.height + 4),
          initial: t.accent,
          presets: _presets(t),
        );
        if (picked != null) {
          workspace.setAccent(picked);
          onPicked();
        }
      },
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        child: Container(
          width: 28,
          height: 18,
          decoration: BoxDecoration(
            color: t.accent,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            border: Border.all(color: t.hairlineStrong),
          ),
        ),
      ),
    );
  }
}

class _TemplateField extends StatefulWidget {
  final Workspace workspace;
  const _TemplateField({required this.workspace});

  @override
  State<_TemplateField> createState() => _TemplateFieldState();
}

class _TemplateFieldState extends State<_TemplateField> {
  late final TextEditingController _controller = TextEditingController(
    text: widget.workspace.export.filenameTemplate ?? '',
  );
  final FocusNode _focus = FocusNode();

  @override
  void initState() {
    super.initState();
    _focus.addListener(() {
      if (!_focus.hasFocus) _commit();
    });
  }

  void _commit() {
    final v = _controller.text.trim();
    widget.workspace.export.filenameTemplate = v.isEmpty ? null : v;
    widget.workspace.touch();
  }

  @override
  void dispose() {
    _controller.dispose();
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      width: 180,
      padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 3),
      decoration: BoxDecoration(
        color: t.surface0,
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        border: Border.all(color: t.hairline),
      ),
      child: EditableText(
        controller: _controller,
        focusNode: _focus,
        style: t.bodyPrimary,
        cursorColor: t.accent,
        backgroundCursorColor: t.surface2,
        selectionColor: t.accent.withValues(alpha: 0.5),
        onSubmitted: (_) => _commit(),
      ),
    );
  }
}

// --- Page scaffolding (page_heading / settings_group / settings_row) -------

class _Page extends StatelessWidget {
  final String title;
  final List<_Group> groups;
  const _Page({required this.title, required this.groups});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SizedBox(height: 2),
        Text(title, style: t.heading),
        for (final g in groups) g,
        const SizedBox(height: 8),
      ],
    );
  }
}

class _Group extends StatelessWidget {
  final String title;
  final List<_Row> rows;
  const _Group({required this.title, required this.rows});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final round = t.shape == ThemeShape.round;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SizedBox(height: 12),
        Text(title, style: t.small),
        const SizedBox(height: 4),
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
          decoration: BoxDecoration(
            color: t.surface2,
            borderRadius:
                round ? BorderRadius.circular(t.tokens.cardRadius) : null,
            border: round ? null : Border.all(color: t.hairline),
          ),
          child: Column(
            children: [
              for (var i = 0; i < rows.length; i++) ...[
                if (i > 0) Container(height: 1, color: t.hairline),
                rows[i],
              ],
            ],
          ),
        ),
      ],
    );
  }
}

class _Row extends StatelessWidget {
  final String label;
  final String? hint;
  final Widget control;
  const _Row({required this.label, this.hint, required this.control});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(label, style: t.bodyPrimary),
                if (hint != null) Text(hint!, style: t.small),
              ],
            ),
          ),
          const SizedBox(width: 12),
          control,
        ],
      ),
    );
  }
}
