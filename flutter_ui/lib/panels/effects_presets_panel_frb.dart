// The Effects & presets panel, on the flutter_rust_bridge API.
//
// Every built-in effect under its category heading, filtered by a search field,
// with the selected layer's `.lumfx` save and load beneath. An effect applies by
// double-click or by dragging it onto the Effect controls panel — the drag
// carries an `EffectDragData`, which is the only thing that produces one.
//
// The list comes from `listEffects`, which is the engine's own schema order, so
// the panel never holds a copy of what effects exist. Adding a built-in to the
// engine puts it here with no Dart change at all.
//
// **Every heading twirls.** A category — and the saved-preset group above them
// — folds its rows away behind the set's triangle, the same one the Timeline
// and the Project panel fold with: right is shut, down is open. Which headings
// are shut is *view* state, so it lives in this widget and dies with the
// session rather than crossing the bridge. A search overrides it: while the
// field has something in it every match shows, whatever was folded, because a
// search that hides what it found is a trap. Clearing the field puts the folds
// back exactly as they were.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../state/dock.dart' show Panel;
import '../theme/theme.dart';
import '../state/drag_payloads.dart';
import '../state/file_dialogs.dart';
import '../widgets/controls.dart';

class EffectsPresetsPanelFrb extends StatefulWidget {
  /// The preset file seams, injected by tests so no plugin channel opens.
  final Future<String?> Function()? savePicker;
  final Future<String?> Function()? loadPicker;

  /// The library listing seam, injected by tests so no real library is read.
  final List<BridgePresetInfo> Function()? presetsLister;

  const EffectsPresetsPanelFrb(
      {super.key, this.savePicker, this.loadPicker, this.presetsLister});

  @override
  State<EffectsPresetsPanelFrb> createState() => _EffectsPresetsPanelFrbState();
}

class _EffectsPresetsPanelFrbState extends State<EffectsPresetsPanelFrb> {
  final TextEditingController _search = TextEditingController();

  /// The search field's focus, owned here so `Ctrl+F` can put the cursor in it
  /// (docs/07 §15, "Panels").
  final FocusNode _searchFocus = FocusNode();

  /// The shell state this panel is listening to, so the listener comes off the
  /// same object it went on.
  LumitUiState? _boundUi;

  /// The saved-preset library, read once and after each save — not per
  /// rebuild, which would scan a folder on every search keystroke.
  List<BridgePresetInfo> _presets = const [];

  /// The headings the user has folded shut, by engine category name. Nothing
  /// here goes to the engine: which groups are open is how this panel is being
  /// looked at, not part of the document (the Project panel's `_closedFolders`
  /// keeps its folders the same way).
  final Set<String> _shut = <String>{};

  /// The saved-preset group's name in [_shut]. No engine category is called
  /// this, so one set holds the presets and the categories without collision.
  static const String _presetsKey = '*saved-presets';

  /// Whether a heading's rows are showing. A live search opens every group
  /// that still has a match in it, without disturbing what was folded — so
  /// clearing the field puts the folds back as they were.
  bool _isOpen(String key, String needle) =>
      needle.isNotEmpty || !_shut.contains(key);

  void _toggle(String key) =>
      setState(() => _shut.contains(key) ? _shut.remove(key) : _shut.add(key));

  @override
  void initState() {
    super.initState();
    _search.addListener(() => setState(() {}));
    _presets = (widget.presetsLister ?? listPresets)();
    // `Ctrl+F` asks the focused panel for its search box; this answers only
    // while Effects & presets is the focused one.
    _boundUi = Provider.of<LumitUiState>(context, listen: false);
    _boundUi!.panelSearchRequest.addListener(_onSearchRequested);
  }

  void _onSearchRequested() {
    if (!mounted) return;
    if (_boundUi?.searchRequestIsFor(Panel.effectsAndPresets) ?? false) {
      _searchFocus.requestFocus();
    }
  }

  void _refreshPresets() {
    setState(() => _presets = (widget.presetsLister ?? listPresets)());
  }

  @override
  void dispose() {
    _boundUi?.panelSearchRequest.removeListener(_onSearchRequested);
    _search.dispose();
    _searchFocus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = Provider.of<LumitUiState>(context);
    final needle = _search.text.trim().toLowerCase();

    // Grouped in schema order, so the headings come out in the order the engine
    // declares rather than alphabetically by accident.
    final grouped = <String, List<BridgeEffectInfo>>{};
    final headings = <String, String>{};
    for (final effect in listEffects()) {
      if (needle.isNotEmpty &&
          !effect.label.toLowerCase().contains(needle) &&
          !engineLabel(effect.categoryLabel).toLowerCase().contains(needle)) {
        continue;
      }
      grouped.putIfAbsent(effect.category, () => []).add(effect);
      headings[effect.category] = engineLabel(effect.categoryLabel);
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Container(
          height: 26,
          color: t.surface1,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          child: Row(
            children: [
              lumitIcon(LumitIcon.star, size: iconSize, color: t.textMuted),
              const SizedBox(width: 6),
              Expanded(
                child: HouseTextField(
                  key: const ValueKey('fx-search'),
                  controller: _search,
                  focusNode: _searchFocus,
                  width: 160,
                ),
              ),
            ],
          ),
        ),
        Expanded(
          // The saved-preset library first — a handful of the user's own
          // things reads better above the forty built-ins than buried
          // beneath them. The same search field filters it, and a
          // double-click applies the whole stack. A search that matches a
          // preset but no effect still shows the preset.
          child: Builder(builder: (context) {
            final presetRows = _presetRows(t, ui, needle);
            if (grouped.isEmpty && presetRows.isEmpty) {
              return Center(child: Text(l10n.noEffectsMatch, style: t.small));
            }
            return ListView(
              padding: const EdgeInsets.symmetric(vertical: 4),
              children: [
                ...presetRows,
                for (final entry in grouped.entries) ...[
                  _heading(
                    t,
                    group: entry.key,
                    label: headings[entry.key] ?? entry.key,
                    open: _isOpen(entry.key, needle),
                  ),
                  if (_isOpen(entry.key, needle))
                    for (final effect in entry.value)
                      _EffectRow(
                        key: ValueKey<String>('fx-item-${effect.name}'),
                        effect: effect,
                        onApply: () => _apply(ui, effect.name),
                      ),
                ],
              ],
            );
          }),
        ),
        _PresetBar(
          layer: ui.selectedLayer.value,
          savePicker: widget.savePicker,
          loadPicker: widget.loadPicker,
          onChanged: _refreshPresets,
        ),
      ],
    );
  }

  /// Apply to the selected layer. With none selected there is nothing to apply
  /// to, and silently doing nothing is better than guessing at a layer.
  void _apply(LumitUiState ui, String name) {
    final layer = ui.selectedLayer.value;
    if (layer == null) return;
    layer.addEffect(name: name);
    setState(() {});
  }

  /// A heading and its twirl: the whole strip is the target, the way the
  /// Timeline's section headers are, and the triangle points right while the
  /// rows under it are away.
  Widget _heading(
    LumitTheme t, {
    required String group,
    required String label,
    required bool open,
  }) =>
      GestureDetector(
        key: ValueKey<String>('fx-group-$group'),
        behavior: HitTestBehavior.opaque,
        onTap: () => _toggle(group),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(4, 6, 10, 2),
          child: Row(
            children: [
              lumitIcon(
                open ? LumitIcon.twirlOpen : LumitIcon.twirlClosed,
                size: iconSize,
                color: t.textMuted,
              ),
              const SizedBox(width: 2),
              Expanded(
                child: Text(label,
                    style: t.small.copyWith(color: t.textMuted),
                    overflow: TextOverflow.ellipsis),
              ),
            ],
          ),
        ),
      );

  List<Widget> _presetRows(LumitTheme t, LumitUiState ui, String needle) {
    final shown = _presets
        .where((p) => needle.isEmpty || p.name.toLowerCase().contains(needle))
        .toList();
    if (shown.isEmpty) return const [];
    final open = _isOpen(_presetsKey, needle);
    return [
      _heading(t, group: _presetsKey, label: l10n.savedPresets, open: open),
      if (open)
        for (final preset in shown)
          GestureDetector(
            key: ValueKey<String>('preset-item-${preset.name}'),
            behavior: HitTestBehavior.opaque,
            onDoubleTap: () => _applyPreset(ui, preset),
            child: Container(
              height: 20,
              padding: const EdgeInsets.symmetric(horizontal: 22),
              alignment: Alignment.centerLeft,
              child: Text(preset.name, style: t.body),
            ),
          ),
    ];
  }

  /// Apply a library preset's whole stack to the selected layer. A file that
  /// has gone away since the listing just refreshes the listing — the library
  /// is a folder, and folders change behind running programs.
  void _applyPreset(LumitUiState ui, BridgePresetInfo preset) {
    final layer = ui.selectedLayer.value;
    if (layer == null) return;
    final file = File(preset.path);
    if (!file.existsSync()) {
      _refreshPresets();
      return;
    }
    try {
      layer.loadPreset(text: file.readAsStringSync());
    } catch (_) {
      return;
    }
    setState(() {});
  }
}

class _EffectRow extends StatelessWidget {
  final BridgeEffectInfo effect;
  final VoidCallback onApply;
  const _EffectRow({super.key, required this.effect, required this.onApply});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final row = GestureDetector(
      behavior: HitTestBehavior.opaque,
      onDoubleTap: onApply,
      child: Container(
        height: 20,
        // Indented under the heading's own label, past the twirl's column.
        padding: const EdgeInsets.symmetric(horizontal: 22),
        alignment: Alignment.centerLeft,
        child: Text(effect.label, style: t.body),
      ),
    );

    return Draggable<EffectDragData>(
      data: EffectDragData(effect.name, engineLabel(effect.label)),
      dragAnchorStrategy: pointerDragAnchorStrategy,
      feedback: FloatSurface(
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
          child: Text(effect.label, style: t.small),
        ),
      ),
      child: row,
    );
  }
}

/// Save the selected layer's stack as a `.lumfx`, or load one onto it.
class _PresetBar extends StatelessWidget {
  final LayerReference? layer;
  final Future<String?> Function()? savePicker;
  final Future<String?> Function()? loadPicker;
  final VoidCallback onChanged;

  const _PresetBar({
    required this.layer,
    required this.savePicker,
    required this.loadPicker,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final target = layer;

    return Container(
      height: 26,
      color: t.surface1,
      padding: const EdgeInsets.symmetric(horizontal: 6),
      // Scrolls sideways when docked narrow rather than striping (the
      // Timeline toolbar's answer).
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: Row(
          children: [
            HouseButton(
              key: const ValueKey('preset-save'),
              small: true,
              frameless: true,
              onPressed: target == null ? null : () => _save(target),
              child: Text(l10n.savePresetEllipsis, style: t.small),
            ),
            const SizedBox(width: 6),
            HouseButton(
              key: const ValueKey('preset-load'),
              small: true,
              frameless: true,
              onPressed: target == null ? null : () => _load(target),
              child: Text(l10n.loadPresetEllipsis, style: t.small),
            ),
            if (target == null) ...[
              const SizedBox(width: 16),
              Text(l10n.selectALayer,
                  style: t.small.copyWith(color: t.textMuted)),
            ],
          ],
        ),
      ),
    );
  }

  /// The engine hands back the text; choosing where it goes is the picker's job
  /// and writing it is Dart's, so the engine never opens a file dialogue. The
  /// dialogue starts in the preset library folder, so a plain save lands
  /// where the listing above will find it.
  Future<void> _save(LayerReference target) async {
    final picker = savePicker;
    final path = picker != null
        ? await picker()
        : await pickPresetSaveLocation('preset.lumfx',
            initialDirectory: presetsDirPath());
    if (path == null) return;
    // The preset's display name is its file's stem, matching the egui frontend.
    final name = path.split(RegExp(r'[/\\]')).last.replaceAll('.lumfx', '');
    File(path).writeAsStringSync(target.savePreset(name: name));
    onChanged();
  }

  Future<void> _load(LayerReference target) async {
    final path = await (loadPicker ?? pickPresetToOpen)();
    if (path == null) return;
    final file = File(path);
    if (!file.existsSync()) return;
    try {
      target.loadPreset(text: file.readAsStringSync());
    } catch (_) {
      // Not a preset: the picker will take any file, so this is a normal thing
      // for a user to do and not something to shout about.
      return;
    }
    onChanged();
  }
}
