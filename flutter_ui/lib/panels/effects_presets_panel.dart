// The Effects & presets panel (phase F-D): a search field over the built-in
// effect registry (`app.listEffects()`), the matching effects grouped under
// collapsing category headers (the registry's `category`/`categoryLabel`, from
// bridge v0.5), and Apply — double-click a row, or the Add button on the hovered
// row, applies the effect to the selected layer of the front composition. Each
// row is also Draggable: dropping it on the Effect controls panel applies it to
// the shown layer (a DragTarget there). No selected layer shows a quiet hint.
//
// Grounding: the egui `effects_panel` (crates/lumit-ui/src/shell/panels.rs)
// groups the built-ins by `FxCategory` and lists user `.lumfx` presets above
// them. The category grouping is mirrored, and Save/Load preset (below) drive
// the bridge v0.9 `save_effect_preset`/`load_effect_preset` ops — byte-compatible
// with `lumit-ui`'s `preset.rs`, so a file round-trips into the egui app. The one
// gap: egui also LISTS the saved preset files above the categories (scanning
// `lumit_project::presets_dir()`); the bridge exposes save/load but no listing,
// so that browser row awaits a `list_presets`/`presets_dir` bridge op (ledger).

import 'package:flutter/widgets.dart';

import '../bridge/bridge.dart';
import '../state/app_state.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

class EffectsPresetsPanel extends StatefulWidget {
  final AppStateStub app;
  const EffectsPresetsPanel({super.key, required this.app});

  @override
  State<EffectsPresetsPanel> createState() => _EffectsPresetsPanelState();
}

class _EffectsPresetsPanelState extends State<EffectsPresetsPanel> {
  final TextEditingController _search = TextEditingController();
  final FocusNode _searchFocus = FocusNode();
  String _needle = '';

  /// Collapsed category keys (empty = all open). Keyed by the stable category
  /// machine key so the fold survives a rebuild.
  final Set<String> _collapsed = {};

  @override
  void dispose() {
    _search.dispose();
    _searchFocus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ListenableBuilder(
      listenable: widget.app,
      builder: (context, _) {
        final app = widget.app;
        final layerId = app.selectedLayer;
        final compId = app.frontCompIdResolved;
        final canApply = layerId != null && compId != null;

        final all = app.listEffects();
        final needle = _needle.trim().toLowerCase();
        final shown = needle.isEmpty
            ? all
            : all
                .where((e) => e.label.toLowerCase().contains(needle))
                .toList();

        // Group by category, preserving the registry's order (first-seen wins).
        final groups = <String, _Category>{};
        for (final e in shown) {
          final key = e.category.isEmpty ? '' : e.category;
          final label = e.categoryLabel.isEmpty ? 'Effects' : e.categoryLabel;
          (groups[key] ??= _Category(key, label)).effects.add(e);
        }

        return Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(10, 8, 10, 6),
              child: _SearchField(
                controller: _search,
                focus: _searchFocus,
                onChanged: (v) => setState(() => _needle = v),
              ),
            ),
            if (!canApply)
              Padding(
                padding: const EdgeInsets.fromLTRB(10, 0, 10, 6),
                child: Text(
                  'Select a layer to apply an effect.',
                  style: t.small.copyWith(color: t.textMuted),
                ),
              ),
            Expanded(
              child: shown.isEmpty
                  ? _emptyHint(t, all.isEmpty, needle)
                  : ListView(
                      padding: const EdgeInsets.symmetric(horizontal: 6),
                      children: [
                        for (final group in groups.values)
                          ..._categorySection(
                              group, canApply, compId, layerId, needle),
                      ],
                    ),
            ),
            _PresetActions(app: app, canApply: canApply),
          ],
        );
      },
    );
  }

  /// A category header (collapsing) plus its effect rows when open. A single
  /// uncategorised group (an older registry) shows no header — the list stays
  /// flat, the honest fallback.
  List<Widget> _categorySection(_Category group, bool canApply, String? compId,
      String? layerId, String needle) {
    final app = widget.app;
    final soleUncategorised =
        group.key.isEmpty; // no category field: skip the header
    // While searching, keep every matching category expanded so a hit is never
    // hidden behind a collapsed header.
    final collapsed = needle.isEmpty && _collapsed.contains(group.key);
    return [
      if (!soleUncategorised)
        _CategoryHeader(
          label: group.label,
          collapsed: collapsed,
          onTap: () => setState(() {
            if (!_collapsed.remove(group.key)) _collapsed.add(group.key);
          }),
        ),
      if (soleUncategorised || !collapsed)
        for (final e in group.effects)
          _EffectRow(
            info: e,
            canApply: canApply,
            onApply: canApply
                ? () => app.addEffect(compId!, layerId!, e.name)
                : null,
          ),
    ];
  }

  Widget _emptyHint(LumitTheme t, bool registryEmpty, String needle) {
    final text = registryEmpty
        ? 'No effects available.'
        : 'No effects match your search.';
    return Padding(
      padding: const EdgeInsets.all(10),
      child: Text(text, style: t.small.copyWith(color: t.textMuted)),
    );
  }
}

/// One category and the effects that fall under it, in registry order.
class _Category {
  final String key;
  final String label;
  final List<BridgeEffectInfo> effects = [];
  _Category(this.key, this.label);
}

/// A collapsing category header (the egui `FxCategory` heading).
class _CategoryHeader extends StatelessWidget {
  final String label;
  final bool collapsed;
  final VoidCallback onTap;
  const _CategoryHeader({
    required this.label,
    required this.collapsed,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 6),
        child: Row(
          children: [
            Text(collapsed ? '▸' : '▾',
                style: t.small.copyWith(color: t.textMuted)),
            const SizedBox(width: 6),
            Text(label,
                style: t.small.copyWith(
                    color: t.textSecondary, fontWeight: FontWeight.w600)),
          ],
        ),
      ),
    );
  }
}

/// The search box, styled like the colour picker's hex field (borderless, an
/// accent edge on focus).
class _SearchField extends StatelessWidget {
  final TextEditingController controller;
  final FocusNode focus;
  final ValueChanged<String> onChanged;
  const _SearchField({
    required this.controller,
    required this.focus,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 5),
      decoration: BoxDecoration(
        color: t.surface0,
        borderRadius: BorderRadius.circular(t.tokens.controlRadius),
        border: Border.all(color: focus.hasFocus ? t.accent : t.hairline),
      ),
      child: EditableText(
        controller: controller,
        focusNode: focus,
        style: t.bodyPrimary,
        cursorColor: t.accent,
        backgroundCursorColor: t.surface2,
        selectionColor: t.accent.withValues(alpha: 0.5),
        onChanged: onChanged,
      ),
    );
  }
}

/// One effect row: the label, and — when a layer is selected — an Add button
/// that appears on hover. Double-clicking the row applies the effect; the row is
/// Draggable (drop it on the Effect controls panel to apply it to the shown
/// layer). The Timeline-row drop target awaits the timeline agent's DragTarget
/// seam (annotated on the ledger).
class _EffectRow extends StatefulWidget {
  final BridgeEffectInfo info;
  final bool canApply;
  final VoidCallback? onApply;
  const _EffectRow({
    required this.info,
    required this.canApply,
    required this.onApply,
  });

  @override
  State<_EffectRow> createState() => _EffectRowState();
}

class _EffectRowState extends State<_EffectRow> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final row = MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onDoubleTap: widget.onApply,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 4),
          decoration: BoxDecoration(
            color: _hover ? t.surface3 : null,
            borderRadius: BorderRadius.circular(t.tokens.controlRadius),
          ),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  widget.info.label,
                  style: t.bodyPrimary,
                  overflow: TextOverflow.ellipsis,
                ),
              ),
              if (_hover && widget.canApply) ...[
                const SizedBox(width: 8),
                LumitTooltip(
                  message: 'Apply to the selected layer',
                  child: HouseButton(
                    small: true,
                    onPressed: widget.onApply,
                    child: Text('Add', style: t.small),
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
    // Draggable onto a layer (the Effect controls DragTarget). The payload is
    // the effect's match name; the drop applies it through `addEffect`.
    return Draggable<EffectDragData>(
      data: EffectDragData(widget.info.name, widget.info.label),
      dragAnchorStrategy: pointerDragAnchorStrategy,
      feedback: _EffectDragFeedback(label: widget.info.label),
      child: row,
    );
  }
}

/// The floating label shown under the pointer while an effect row is dragged.
class _EffectDragFeedback extends StatelessWidget {
  final String label;
  const _EffectDragFeedback({required this.label});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        child: Text(label, style: t.small),
      ),
    );
  }
}

/// The `.lumfx` preset actions (bridge v0.9): Save preset serialises the
/// selected layer's effect stack through `save_effect_preset` (byte-compatible
/// with `lumit-ui`'s `preset.rs`) into a file the user picks; Load preset reads
/// a chosen `.lumfx` and appends its effects with fresh ids. Save is offered
/// only with a selected layer; Load only needs a selected target.
///
/// Honest gap: egui's browser also LISTS the user's saved presets above the
/// built-in categories, scanning `lumit_project::presets_dir()` each paint. The
/// bridge exposes only save/load — not the presets folder or a listing — so the
/// browser listing awaits a `list_presets`/`presets_dir` bridge op (ledger).
class _PresetActions extends StatelessWidget {
  final AppStateStub app;
  final bool canApply;
  const _PresetActions({required this.app, required this.canApply});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      decoration: BoxDecoration(
        border: Border(top: BorderSide(color: t.hairline)),
      ),
      // A Wrap so the two actions flow to a second line rather than overflowing
      // when the panel is narrow (the leading label rides the first line).
      child: Wrap(
        spacing: 6,
        runSpacing: 6,
        crossAxisAlignment: WrapCrossAlignment.center,
        children: [
          Text('Presets', style: t.small.copyWith(color: t.textMuted)),
          LumitTooltip(
            message: canApply
                ? "Save the selected layer's effects as a .lumfx preset"
                : 'Select a layer to save its effects',
            child: HouseButton(
              key: const ValueKey('preset-save'),
              small: true,
              onPressed: canApply ? () => app.saveSelectedEffectPreset() : null,
              child: Text('Save preset', style: t.small),
            ),
          ),
          LumitTooltip(
            message: canApply
                ? 'Load a .lumfx preset onto the selected layer'
                : 'Select a layer to load a preset onto',
            child: HouseButton(
              key: const ValueKey('preset-load'),
              small: true,
              onPressed: canApply ? () => app.loadPresetOntoSelected() : null,
              child: Text('Load preset', style: t.small),
            ),
          ),
        ],
      ),
    );
  }
}
