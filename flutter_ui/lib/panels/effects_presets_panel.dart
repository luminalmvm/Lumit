// The Effects & presets panel (phase F4): a search field over the built-in
// effect registry (`app.listEffects()`), the matching effects listed, and
// Apply — double-click a row, or the Add button on the hovered/selected row,
// applies the effect to the selected layer of the front composition
// (`app.addEffect`). No selected layer shows a quiet hint.
//
// The egui `effects_panel` (shell/panels.rs) groups the built-ins by
// `FxCategory` and lists user `.lumfx` presets above them. The Dart registry
// (`BridgeEffectInfo {name, label}`) carries no category, so the list here is
// flat — the honest mirror of what the bridge exposes. The .lumfx preset
// save/load stays out until the file + preset bridge ops exist; a placeholder
// row at the bottom says exactly that.

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
                  : ListView.builder(
                      padding: const EdgeInsets.symmetric(horizontal: 6),
                      itemCount: shown.length,
                      itemBuilder: (context, i) {
                        final e = shown[i];
                        return _EffectRow(
                          info: e,
                          canApply: canApply,
                          onApply: canApply
                              ? () => app.addEffect(compId, layerId, e.name)
                              : null,
                        );
                      },
                    ),
            ),
            _PresetPlaceholder(),
          ],
        );
      },
    );
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
/// that appears on hover. Double-clicking the row applies the effect too.
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
    return MouseRegion(
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
  }
}

/// The .lumfx preset row: honestly disabled until the file + preset bridge ops
/// exist.
class _PresetPlaceholder extends StatelessWidget {
  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
      decoration: BoxDecoration(
        border: Border(top: BorderSide(color: t.hairline)),
      ),
      child: Text(
        'Saving and loading .lumfx presets arrives with the file and preset '
        'bridge ops.',
        style: t.small.copyWith(color: t.textMuted),
      ),
    );
  }
}
