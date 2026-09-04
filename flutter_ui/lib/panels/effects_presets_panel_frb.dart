// The Effects & presets panel, on the flutter_rust_bridge API.
//
// Every built-in effect under its category heading, filtered by a search field,
// with the selected layer's `.lumfx` save and load beneath. An effect applies by
// double-click — to every selected layer, as the Effect menu does — or by
// dragging it onto the Effect controls panel, which carries an `EffectDragData`
// and lands on that panel's one layer.
//
// The list comes from `listEffects`, which is the engine's own schema order, so
// the panel never holds a copy of what effects exist. Adding a built-in to the
// engine puts it here with no Dart change at all.
//
// **Plugins are in that same list** (docs/12 §2.6). An OFX plugin the
// engine found on this machine arrives as one more entry, under a heading that
// is its own declared grouping rather than one of Lumit's ten categories, and
// it groups, folds, searches, stars and drags exactly as a built-in does. The
// one difference the spec asks for is a small provenance tag in the row's
// context menu, which is also where the plugin can be switched off.
//
// **Favourites** (owner, desk test). A star on every row, and the ones starred
// gather under a Favourites heading above everything else — the effects you
// reach for are four or five of the forty, and hunting them down their
// categories every time is the panel's oldest annoyance. A star is a
// *preference*, so it lives in the workspace and survives a restart, not in
// this widget beside the folds.
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

  /// The catalogue seam, on the same footing as [presetsLister]. It exists so a
  /// test can put a *discovered plugin* in the list without a real bundle
  /// installed on the machine running it — the engine's own scan is tested
  /// where it lives (`crates/lumit-ofx/tests/discover.rs`), and what this panel
  /// owes is the grouping, the fold and the provenance tag.
  final List<BridgeEffectInfo> Function()? effectsLister;

  const EffectsPresetsPanelFrb({
    super.key,
    this.savePicker,
    this.loadPicker,
    this.presetsLister,
    this.effectsLister,
  });

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

  /// The Favourites group's name in [_shut], on the same footing.
  static const String _favouritesKey = '*favourites';

  /// A saved preset's key in the workspace's favourites. An effect is starred
  /// under its own match name, which no preset can collide with because a
  /// preset's carries this prefix.
  static String _presetFavouriteKey(String name) => 'preset:$name';

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
    for (final effect in (widget.effectsLister ?? listEffects)()) {
      // A discovered plugin arrives with its own declared grouping as its
      // category (docs/12 §2.6), so it groups and folds through exactly this
      // map — no second list, no plugin branch. A plugin that declared no
      // grouping at all has nothing to head it with, and this is the only place
      // that knows the word for that — audio plugins deliberately declare none
      // (neither standard has OFX's menu path), so the one Audio plugins group
      // sits beside the OFX ones (AP5).
      final heading = effect.categoryLabel.isEmpty
          ? (effect.namespace == _audioNamespace
              ? l10n.effectsAudioPlugins
              : l10n.effectsPlugins)
          : engineLabel(effect.categoryLabel);
      if (needle.isNotEmpty &&
          !effect.label.toLowerCase().contains(needle) &&
          !heading.toLowerCase().contains(needle)) {
        continue;
      }
      grouped.putIfAbsent(effect.category, () => []).add(effect);
      headings[effect.category] = heading;
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
                  hint: l10n.searchEffectsAndPresets,
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
            final favouriteRows = _favouriteRows(t, ui, grouped, needle);
            final presetRows = _presetRows(t, ui, needle);
            if (grouped.isEmpty && presetRows.isEmpty) {
              return Center(child: Text(l10n.noEffectsMatch, style: t.small));
            }
            return ListView(
              padding: const EdgeInsets.symmetric(vertical: 4),
              children: [
                ...favouriteRows,
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
                        favourite: ui.workspace.isFavouriteEffect(effect.name),
                        onToggleFavourite: () => _star(ui, effect.name),
                        onCatalogueChanged: () => setState(() {}),
                      ),
                ],
              ],
            );
          }),
        ),
        _PresetBar(
          layers: ui.selectedLayers.value,
          savePicker: widget.savePicker,
          loadPicker: widget.loadPicker,
          onChanged: _refreshPresets,
        ),
      ],
    );
  }

  /// Apply to **every** selected layer, as the Effect menu and the effects
  /// console do. This panel used to reach for the primary layer alone,
  /// so the same effect on the same selection landed on three layers from the
  /// menu and on one from here — the sort of difference that is read as the
  /// selection having been lost rather than as two paths disagreeing.
  ///
  /// With nothing selected there is nowhere for the effect to go, and silently
  /// doing nothing is better than guessing at a layer.
  void _apply(LumitUiState ui, String name) {
    for (final layer in ui.selectedLayers.value) {
      layer.addEffect(name: name);
    }
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

  /// Star something, or take the star off. The workspace saves itself; this
  /// only has to redraw, because the Favourites group above is built from the
  /// same set the star reads.
  void _star(LumitUiState ui, String key) {
    ui.workspace.toggleFavouriteEffect(key);
    setState(() {});
  }

  /// The Favourites heading and its rows — the starred effects in the engine's
  /// own schema order, then the starred presets.
  ///
  /// It draws nothing at all until something is starred: a permanently empty
  /// heading at the top of the panel would be a standing instruction to use a
  /// feature, which is not what a heading is for. [grouped] is the already
  /// search-filtered effect list, so a search narrows Favourites exactly as it
  /// narrows everything under it.
  List<Widget> _favouriteRows(
    LumitTheme t,
    LumitUiState ui,
    Map<String, List<BridgeEffectInfo>> grouped,
    String needle,
  ) {
    final effects = [
      for (final entry in grouped.entries)
        for (final effect in entry.value)
          if (ui.workspace.isFavouriteEffect(effect.name)) effect,
    ];
    final presets = [
      for (final preset in _presets)
        if (ui.workspace.isFavouriteEffect(_presetFavouriteKey(preset.name)) &&
            (needle.isEmpty || preset.name.toLowerCase().contains(needle)))
          preset,
    ];
    if (effects.isEmpty && presets.isEmpty) return const [];
    final open = _isOpen(_favouritesKey, needle);
    return [
      _heading(t, group: _favouritesKey, label: l10n.favourites, open: open),
      if (open) ...[
        for (final effect in effects)
          _EffectRow(
            key: ValueKey<String>('fav-item-${effect.name}'),
            effect: effect,
            onApply: () => _apply(ui, effect.name),
            favourite: true,
            onToggleFavourite: () => _star(ui, effect.name),
            onCatalogueChanged: () => setState(() {}),
          ),
        for (final preset in presets)
          _PresetRow(
            key: ValueKey<String>('fav-preset-${preset.name}'),
            preset: preset,
            onApply: () => _applyPreset(ui, preset),
            favourite: true,
            onToggleFavourite: () =>
                _star(ui, _presetFavouriteKey(preset.name)),
          ),
      ],
    ];
  }

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
          _PresetRow(
            key: ValueKey<String>('preset-item-${preset.name}'),
            preset: preset,
            onApply: () => _applyPreset(ui, preset),
            favourite: ui.workspace
                .isFavouriteEffect(_presetFavouriteKey(preset.name)),
            onToggleFavourite: () =>
                _star(ui, _presetFavouriteKey(preset.name)),
          ),
    ];
  }

  /// Apply a library preset's whole stack to **every** selected layer, exactly
  /// as [_apply] does with a single effect: the two rows sit in the
  /// same list and are double-clicked with the same gesture, so one of them
  /// quietly meaning "the first layer only" would read as the selection having
  /// been lost.
  ///
  /// A file that has gone away since the listing just refreshes the listing —
  /// the library is a folder, and folders change behind running programs. Read
  /// once and applied many times: the text is the same for every layer, and
  /// re-reading it per layer would let a file change halfway through a batch.
  void _applyPreset(LumitUiState ui, BridgePresetInfo preset) {
    final layers = ui.selectedLayers.value;
    if (layers.isEmpty) return;
    final file = File(preset.path);
    if (!file.existsSync()) {
      _refreshPresets();
      return;
    }
    final String text;
    try {
      text = file.readAsStringSync();
    } catch (_) {
      return;
    }
    for (final layer in layers) {
      // Each layer keeps its own `try`: a stack one layer will not take
      // leaves the rest of the batch standing.
      try {
        layer.loadPreset(text: text);
      } catch (_) {}
    }
    setState(() {});
  }
}

/// How wide the column the star sits in is — the same indent the rows already
/// had before their labels, so the star costs no width and the labels do not
/// move.
const double _starColumn = 22;

/// One row's star, and the tap that turns it on or off.
///
/// Filled with the accent when it is on, drawn in the hairline when it is not:
/// an unstarred row still shows where its star would be, because a control
/// that only appears on hover is a control most people never find.
Widget _star(BuildContext context,
    {required bool on, required VoidCallback onToggle, required String name}) {
  final t = ThemeScope.of(context).theme;
  return LumitTooltip(
    message: on ? l10n.tipUnfavourite : l10n.tipFavourite,
    child: GestureDetector(
      key: ValueKey<String>(name),
      behavior: HitTestBehavior.opaque,
      onTap: onToggle,
      child: SizedBox(
        width: _starColumn,
        height: 20,
        child: Center(
          child: lumitIcon(
            LumitIcon.star,
            size: iconSize,
            color: on ? t.accent : t.hairlineStrong,
          ),
        ),
      ),
    ),
  );
}

/// One row in the list: the star, then the name, laid out so that starring
/// something and dragging it onto a layer are the same row's two gestures.
Widget _libraryRow(
  BuildContext context, {
  required String label,
  required bool favourite,
  required VoidCallback onToggleFavourite,
  required VoidCallback onApply,
  required String starKey,
}) {
  final t = ThemeScope.of(context).theme;
  return SizedBox(
    height: 20,
    child: Row(
      children: [
        _star(context,
            on: favourite, onToggle: onToggleFavourite, name: starKey),
        Expanded(
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onDoubleTap: onApply,
            child: Align(
              alignment: Alignment.centerLeft,
              child: Padding(
                padding: const EdgeInsets.only(right: 6),
                child:
                    Text(label, style: t.body, overflow: TextOverflow.ellipsis),
              ),
            ),
          ),
        ),
      ],
    ),
  );
}

/// A saved preset's row: the same shape an effect's has, so the two halves of
/// the panel read as one list.
class _PresetRow extends StatelessWidget {
  final BridgePresetInfo preset;
  final VoidCallback onApply;
  final bool favourite;
  final VoidCallback onToggleFavourite;
  const _PresetRow({
    super.key,
    required this.preset,
    required this.onApply,
    required this.favourite,
    required this.onToggleFavourite,
  });

  @override
  Widget build(BuildContext context) => _libraryRow(
        context,
        label: preset.name,
        favourite: favourite,
        onToggleFavourite: onToggleFavourite,
        onApply: onApply,
        starKey: 'preset-star-${preset.name}',
      );
}

/// The `namespace` an entry carries when it came out of an OFX plugin — the
/// engine's own spelling (`NAMESPACE_OFX`).
const String _ofxNamespace = 'ofx';

/// And the one an audio plugin carries (`NAMESPACE_AUDIO`, AP5) — CLAP or
/// VST3, which the browser has no reason to tell apart.
const String _audioNamespace = 'audio';

class _EffectRow extends StatelessWidget {
  final BridgeEffectInfo effect;
  final VoidCallback onApply;
  final bool favourite;
  final VoidCallback onToggleFavourite;

  /// Something in the catalogue changed — a plugin was switched off, so the
  /// listing has to be read again.
  final VoidCallback onCatalogueChanged;
  const _EffectRow({
    super.key,
    required this.effect,
    required this.onApply,
    required this.favourite,
    required this.onToggleFavourite,
    required this.onCatalogueChanged,
  });

  /// Where an effect says where it came from (docs/12 §2.6): a small provenance
  /// tag, in the context menu and nowhere else, so a plugin's row in the list is
  /// otherwise identical to a built-in's.
  ///
  /// A plugin's menu carries one command with it, because the tag would
  /// otherwise be a fact with nothing to do: switching the plugin off is the
  /// preference docs/12 §2.6 asks for, and this is where a person is already
  /// looking when they wonder what a plugin is doing in their list.
  void _menu(BuildContext context, Offset at) {
    final audio = effect.namespace == _audioNamespace;
    final plugin = effect.namespace == _ofxNamespace || audio;
    showLumitPopup<void>(
      context: context,
      position: at,
      builder: (close) => FloatSurface(
        child: IntrinsicWidth(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(10, 4, 10, 4),
                child: Text(
                  audio
                      ? l10n.effectFromAudioPlugin
                      : plugin
                          ? l10n.effectFromPlugin
                          : l10n.effectBuiltIn,
                  key: ValueKey<String>('fx-provenance-${effect.name}'),
                  style: ThemeScope.of(context)
                      .theme
                      .small
                      .copyWith(color: ThemeScope.of(context).theme.textMuted),
                ),
              ),
              if (plugin)
                MenuRow(
                  key: ValueKey<String>('fx-disable-${effect.name}'),
                  onPressed: () {
                    close(null);
                    try {
                      setPluginEnabled(effect: effect.name, enabled: false);
                    } catch (_) {
                      // The preference file would not take it. The plugin is
                      // still off for this session, which is what was asked.
                    }
                    // An audio plugin plays from a baked mix, so the switch
                    // asks for a re-prepare — the switched-off list is part
                    // of the mix signature, and the rebake is what actually
                    // silences it (AP5).
                    if (audio) {
                      Provider.of<LumitUiState>(context, listen: false)
                          .selectedComp
                          ?.audioPrepare();
                    }
                    onCatalogueChanged();
                  },
                  child: Text(l10n.switchPluginOff),
                ),
            ],
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final row = GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapDown: (d) => _menu(context, d.globalPosition),
      child: _libraryRow(
        context,
        label: effect.label,
        favourite: favourite,
        onToggleFavourite: onToggleFavourite,
        onApply: onApply,
        starKey: 'fx-star-${effect.name}',
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
  /// Every picked layer. **Save** takes the first — a preset file is one
  /// stack, and saving four would mean choosing which one survives — while
  /// **Load** lands on all of them, the way every other add here does.
  final List<LayerReference> layers;
  final Future<String?> Function()? savePicker;
  final Future<String?> Function()? loadPicker;
  final VoidCallback onChanged;

  const _PresetBar({
    required this.layers,
    required this.savePicker,
    required this.loadPicker,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final target = layers.isEmpty ? null : layers.first;

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
              onPressed: target == null ? null : _load,
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

  Future<void> _load() async {
    final path = await (loadPicker ?? pickPresetToOpen)();
    if (path == null) return;
    final file = File(path);
    if (!file.existsSync()) return;
    final String text;
    try {
      text = file.readAsStringSync();
    } catch (_) {
      return;
    }
    for (final layer in layers) {
      try {
        layer.loadPreset(text: text);
      } catch (_) {
        // Not a preset: the picker will take any file, so this is a normal
        // thing for a user to do and not something to shout about.
      }
    }
    onChanged();
  }
}
