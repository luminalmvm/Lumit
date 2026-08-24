// The Timeline's smaller surfaces: comp tabs, the cache bar, the search field,
// the parent picker, and the marker / work-area editors. (The cache *meter*
// moved to the shell's status line, where whole-store readouts belong.)
//
// A file of their own rather than more of timeline_panel_frb.dart, which is
// already the length it wants to be. Each is small, self-contained and used
// once — kept together because they are all "the chrome around the tracks"
// rather than because they share anything.

import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/icons.dart';
// The K-440 set's drawing widget, under a prefix: `LumitIcon` is also the name
// of the older Iconoir enum this file uses for layer kinds.
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/comp_settings_frb.dart';
import '../state/comp_time.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

/// The Timeline's **panel header strip** (§12A.1, §12A.6: 22 tall): the panel's
/// own kicker, then the open compositions as tabs running the full width, then
/// the single filled Export action at the far right.
///
/// Clicking a tab fronts it; its × closes the tab (docs/07 §4: one tab per
/// *open* comp — the comp itself stays in the project, and fronting it from the
/// Project panel opens it again).
class CompTabsFrb extends StatelessWidget {
  final LumitState state;
  final LumitUiState uiState;

  /// The Export command, run by the header's filled button. Handed in rather
  /// than reached for here: the panel already knows how the menu and the
  /// command palette start an export, and this button must start *that* one.
  final VoidCallback onExport;

  const CompTabsFrb({
    super.key,
    required this.state,
    required this.uiState,
    required this.onExport,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Served from LumitState's cached walk (K-184): the item tree is only
    // re-read when the engine says it changed shape. Filtered to the tabs the
    // user has opened, so a deleted comp's tab also simply stops matching.
    final selected = uiState.selectedComp?.internalid;
    // In the tab strip's own order, not the project's: the strip is dragged
    // into whatever order suits the work, and `openComps` is where that order
    // lives (and what the session writes down).
    final byId = {
      for (final entry in state.comps()) entry.$1.internalid: entry
    };
    final comps = [
      for (final id in uiState.openComps)
        if (byId[id] != null) byId[id]!,
    ];
    // A fronted comp always joins `openComps`, so this only catches a comp
    // fronted from somewhere that has not been through `setSelectedComp` yet.
    if (selected != null &&
        !uiState.openComps.contains(selected) &&
        byId[selected] != null) {
      comps.add(byId[selected]!);
    }
    if (comps.isEmpty) return const SizedBox.shrink();

    return Container(
      height: 22,
      color: t.surface2,
      child: Row(
        children: [
          // The panel's own name, ahead of the tabs (§12A.1). A kicker like
          // every other panel title (§7.1), and lit because the Timeline is
          // the container these tabs belong to rather than one of them.
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Text(l10n.panelTimeline.toUpperCase(), style: t.kickerOn),
          ),
          Expanded(child: _strip(context, t, comps, selected)),
          // The single filled action this surface is allowed (§3.1, §12A.1):
          // the same Export the File menu and the command palette run, one
          // click from the composition it would write.
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: HouseButton(
              key: const ValueKey('tl-export'),
              small: true,
              primary: true,
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 3),
              onPressed: onExport,
              // `primary` sets the label's own style — kicker in `surface_0`
              // on the accent fill (§7.1) — so only the capitals are ours.
              child: Text(l10n.exportAction.toUpperCase()),
            ),
          ),
        ],
      ),
    );
  }

  /// The tabs themselves, scrolling sideways when there are more than fit.
  Widget _strip(
    BuildContext context,
    LumitTheme t,
    List<(CompositionReference, String)> comps,
    UuidValue? selected,
  ) {
    return ListView(
      scrollDirection: Axis.horizontal,
      children: [
        for (var i = 0; i < comps.length; i++)
          DragTarget<UuidValue>(
            onWillAcceptWithDetails: (d) => d.data != comps[i].$1.internalid,
            onAcceptWithDetails: (d) =>
                uiState.moveComp(d.data, comps[i].$1.internalid),
            builder: (context, candidate, _) => Draggable<UuidValue>(
              data: comps[i].$1.internalid,
              feedback: Container(
                height: 22,
                padding: const EdgeInsets.symmetric(horizontal: 10),
                color: t.surface2,
                child: Center(child: Text(comps[i].$2, style: t.small)),
              ),
              childWhenDragging: const SizedBox.shrink(),
              child: _CompTab(
                key: ValueKey<String>('tl-tab-${comps[i].$1.internalid}'),
                name: comps[i].$2,
                active: selected == comps[i].$1.internalid,
                dropping: candidate.isNotEmpty,
                onTap: () => uiState.setSelectedComp(comps[i].$1),
                onMenu: (position) => showCompTabMenuFrb(
                  context: context,
                  comp: comps[i].$1,
                  position: position,
                  onChanged: uiState.model.refresh,
                ),
                closeKey:
                    ValueKey<String>('tl-tab-close-${comps[i].$1.internalid}'),
                onClose: () => uiState.closeComp(
                  comps[i].$1.internalid,
                  // The nearest remaining neighbour fronts: the one to the
                  // left, or the next one when the first tab closes.
                  fallback:
                      comps.length == 1 ? null : comps[i == 0 ? 1 : i - 1].$1,
                ),
              ),
            ),
          ),
      ],
    );
  }
}

/// Spot a double-click from two timestamps, without a recogniser.
///
/// A double-tap recogniser holds every single tap back for the whole
/// double-tap window while the arena waits to see whether a second one is
/// coming — which delays the selection a single click makes, and beside the
/// razor's `onTapUp` stops it cutting at all. Two timestamps owe the arena
/// nothing. One instance per surface that wants the gesture.
class DoubleTap {
  DateTime? _last;
  Offset? _lastAt;

  /// Record a tap; true when it is the second inside [kDoubleTapTimeout] —
  /// and, when [at] is given, within [slop] of the first.
  bool tap({Offset? at, double slop = 0}) {
    final now = DateTime.now();
    final last = _last;
    final lastAt = _lastAt;
    _last = now;
    _lastAt = at;
    if (last != null &&
        now.difference(last) < kDoubleTapTimeout &&
        (at == null || lastAt == null || (lastAt - at).distance < slop)) {
      _last = null;
      return true;
    }
    return false;
  }
}

/// One floating menu at [position] — the `showLumitPopup(FloatSurface(
/// Column(MenuRow…)))` sandwich every context menu here was hand-rolling.
///
/// [rows] builds the menu rows around `close`, which resolves the popup with
/// what was picked (or null when it is dismissed). With no [width] the menu
/// sizes itself to its widest row, which is what the marker menus always did.
Future<T?> showMenuAt<T>({
  required BuildContext context,
  required Offset position,
  double? width,
  required List<Widget> Function(void Function(T?) close) rows,
}) =>
    showLumitPopup<T>(
      context: context,
      position: position,
      builder: (close) {
        final column = Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: rows(close),
        );
        return FloatSurface(
          width: width,
          child: width == null ? IntrinsicWidth(child: column) : column,
        );
      },
    );

/// The right-click menu on a marker flag: edit what it says, take it away, or
/// — for a layer's own markers — clear the lot. Shared by the ruler's comp
/// markers and the bars' layer markers (K-254), which had grown one copy each.
///
/// [markers] is read when a command is picked, not when the menu opens, so an
/// edit made while the label dialog was up is not silently overwritten.
/// [write] commits a replacement list wherever the list lives; [keyPrefix]
/// keeps each surface's long-standing widget keys.
Future<void> showMarkerMenuFrb({
  required BuildContext context,
  required Offset position,
  required BridgeMarker marker,
  required List<BridgeMarker> Function() markers,
  required void Function(List<BridgeMarker>) write,
  bool deleteAll = false,
  String keyPrefix = 'marker-menu',
}) async {
  final picked = await showMenuAt<String>(
    context: context,
    position: position,
    rows: (close) => [
      MenuRow(
        key: ValueKey<String>('$keyPrefix-edit'),
        onPressed: () => close('edit'),
        child: Text(l10n.editMarkerEllipsis),
      ),
      MenuRow(
        key: ValueKey<String>('$keyPrefix-delete'),
        onPressed: () => close('delete'),
        child: Text(l10n.deleteMarker),
      ),
      if (deleteAll)
        MenuRow(
          key: ValueKey<String>('$keyPrefix-delete-all'),
          onPressed: () => close('delete-all'),
          child: Text(l10n.deleteAllMarkers),
        ),
    ],
  );
  if (picked == null || !context.mounted) return;
  switch (picked) {
    case 'edit':
      final label = await showMarkerLabelDialogFrb(
          context: context, initial: marker.label);
      if (label == null) return;
      write([
        for (final m in markers())
          if (m.id == marker.id)
            BridgeMarker(id: m.id, time: m.time, label: label)
          else
            m,
      ]);
    case 'delete':
      write([
        for (final m in markers())
          if (m.id != marker.id) m,
      ]);
    case 'delete-all':
      write(const []);
  }
}

/// A comp tab's context menu. Only one entry so far — the same Composition
/// settings dialog the Project panel's menu opens, reached from the comp the
/// user is actually working in rather than by hunting for its project row.
Future<void> showCompTabMenuFrb({
  required BuildContext context,
  required CompositionReference comp,
  required Offset position,
  required VoidCallback onChanged,
}) async {
  final open = await showMenuAt<bool>(
    context: context,
    position: position,
    width: 210,
    rows: (close) => [
      MenuRow(
        key: const ValueKey('tl-tab-menu-settings'),
        onPressed: () => close(true),
        child: Text(l10n.compositionSettingsEllipsis),
      ),
    ],
  );
  if (open != true || !context.mounted) return;
  if (await showCompSettingsFrb(context: context, comp: comp)) onChanged();
}

class _CompTab extends StatelessWidget {
  final String name;
  final bool active;

  /// A tab being dragged is hovering over this one, which is where it would
  /// land — lit so the drop is visible before it is taken.
  final bool dropping;
  final VoidCallback onTap;
  final ValueChanged<Offset> onMenu;
  final Key closeKey;
  final VoidCallback onClose;
  const _CompTab({
    super.key,
    required this.name,
    required this.active,
    required this.dropping,
    required this.onTap,
    required this.onMenu,
    required this.closeKey,
    required this.onClose,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final round = t.shape == ThemeShape.round;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      onSecondaryTapUp: (d) => onMenu(d.globalPosition),
      child: Container(
        padding: const EdgeInsets.only(left: 10, right: 10),
        decoration: BoxDecoration(
          // Round fills the fronted tab with the accent (K-394, §12.1); Sharp
          // seats the fronted tab in the panel's own surface, so the tab and
          // the comp under it read as one thing.
          color: dropping
              ? t.accent.withValues(alpha: 0.18)
              : (active ? (round ? t.accent : t.surface1) : null),
          // **No accent tick, and no seams** (§12A.1): the seated surface
          // colour alone marks the open composition, exactly as the mockup
          // draws it — it computes no border on any tab. The accent's "active
          // tab tick" (§3.1) is the workspace tabs', not these, and the
          // hairlines that used to rule each seam only turned the strip into a
          // grid over a header that already reads as one row.
          //
          // The sides are still *reserved*, transparent: a tab that lost its
          // border would be two pixels narrower than the same tab in Round,
          // and every tab would shift the moment the shape changed.
          border: round
              ? Border.all(color: t.accent.withValues(alpha: 0), width: 2)
              : Border.symmetric(
                  vertical: BorderSide(color: t.hairline.withValues(alpha: 0)),
                ),
          borderRadius:
              round ? BorderRadius.circular(t.tokens.controlRadius) : null,
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Center(
              // Body, not `small`: a composition's name is the *user's* text
              // (§7.1), and the mockup sets both states at 11. Only the colour
              // tells the fronted tab from the rest.
              child: Text(
                name,
                style: !active
                    ? t.body.copyWith(color: t.textMuted)
                    : round
                        ? t.bodyPrimary.copyWith(color: t.surface0)
                        : t.bodyPrimary,
              ),
            ),
            const SizedBox(width: 8),
            GestureDetector(
              key: closeKey,
              behavior: HitTestBehavior.opaque,
              onTap: onClose,
              child: SizedBox(
                width: 12,
                height: 22,
                child: Center(
                  // Muted, unless it is sitting on Round's filled accent —
                  // where muted grey is barely there. Same flip as the label.
                  child: Text('×',
                      style: t.body.copyWith(
                          color: round && active ? t.surface0 : t.textMuted)),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// The outline's search field: narrows the rows to those whose name matches.
class LayerSearchFrb extends StatefulWidget {
  final ValueChanged<String> onChanged;
  final double width;
  const LayerSearchFrb({super.key, required this.onChanged, this.width = 120});

  @override
  State<LayerSearchFrb> createState() => _LayerSearchFrbState();
}

class _LayerSearchFrbState extends State<LayerSearchFrb> {
  final TextEditingController _controller = TextEditingController();

  @override
  void initState() {
    super.initState();
    _controller.addListener(() => widget.onChanged(_controller.text));
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return HouseTextField(
      key: const ValueKey('tl-search'),
      controller: _controller,
      width: widget.width,
      // The well fills the secondary row it sits in rather than floating
      // inside it: that row is 18 (K-451) and the 16px search glyph plus the
      // well's own hairline is already all of it, so there is no room above
      // and below to spend.
      padding: const EdgeInsets.symmetric(horizontal: 6),
      hint: l10n.searchLayers,
      // The well says "editable"; the glyph says *what* it edits — the field
      // stretches the width of the outline (§12A.1), and a bare well that wide
      // reads as a name box rather than as a search.
      leading: glyph.LumitIcon(LumitIcons.search,
          size: iconSize, colour: t.textMuted),
    );
  }
}

/// The parent picker: every *other* layer in the comp, plus None.
///
/// A layer cannot parent to itself, so it is not in its own list — the engine
/// refuses it anyway, but offering a choice that always fails is a worse way to
/// say so than not offering it.
///
/// Costs no bridge calls at all (K-184): the current parent's name and every
/// other layer's name come from the read model. This used to be one name call
/// per other layer per row per rebuild — O(layers²) across the outline.
class ParentPickerFrb extends StatelessWidget {
  final LayerReference layer;
  final BridgeLayerInfo info;

  /// Every layer in the comp, from the read model.
  final List<BridgeLayerEntry> all;

  /// The cell's width — its share of the compose group, which the header's
  /// seam can be dragged to widen.
  final double width;
  final VoidCallback onChanged;

  const ParentPickerFrb({
    super.key,
    required this.layer,
    required this.info,
    required this.all,
    required this.onChanged,
    this.width = parentCellWidth,
  });

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: width,
      child: BareLazyDropdown(
        key: ValueKey<String>('tl-parent-${layer.internallayerId}'),
        // In an outline row, so the mockup's 16/10 face (§12A.6, K-451).
        dense: true,
        label: info.parent == null ? l10n.none : (info.parentName ?? l10n.none),
        options: () => [
          (null, l10n.none),
          for (final e in all)
            if (e.layer.internallayerId != layer.internallayerId)
              (e.layer.internallayerId, e.info.name),
        ],
        onChanged: (id) {
          // A cycle is refused engine-side; the picker reports nothing and the
          // row keeps the parent it had.
          try {
            layer.setParent(parent: id);
          } catch (_) {
            return;
          }
          onChanged();
        },
      ),
    );
  }
}

/// The layer's matte cell (docs/06 §1.6): which layer gates this one, drawn
/// straight from the row's info (K-184). The dropdown picks the source; with
/// one set, the two small toggles choose luma-over-alpha and invert.
class MattePickerFrb extends StatelessWidget {
  final LayerReference layer;
  final BridgeLayerInfo info;

  /// Every layer in the comp, from the read model.
  final List<BridgeLayerEntry> all;

  /// The cell's width — its share of the compose group, which the header's
  /// seam can be dragged to widen.
  final double width;
  final VoidCallback onChanged;

  const MattePickerFrb({
    super.key,
    required this.layer,
    required this.info,
    required this.all,
    required this.onChanged,
    this.width = matteCellWidth,
  });

  void _set(BridgeMatte? matte) {
    try {
      layer.setMatte(matte: matte);
    } catch (_) {
      return;
    }
    onChanged();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final matte = info.matte;
    final sourceName = matte == null
        ? l10n.noMatte
        : all
                .where((e) => e.layer.internallayerId == matte.layer)
                .map((e) => e.info.name)
                .firstOrNull ??
            engineLabel('Matte');

    // A fixed overall width whether or not the mode toggles are showing, so
    // the columns after the matte cell never shift as mattes come and go —
    // with no matte set, the dropdown takes the toggles' room rather than
    // leaving a dead gap before the blend cell.
    return SizedBox(
      width: width,
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          SizedBox(
            // The two mode toggles are 28 px between them; with no matte set
            // the dropdown takes that room rather than leaving a dead gap.
            width: matte == null ? width : (width - 28).clamp(40.0, width),
            child: BareLazyDropdown<UuidValue?>(
              key: ValueKey<String>('tl-matte-${layer.internallayerId}'),
              // In an outline row, so the mockup's 16/10 face (§12A.6, K-451).
              dense: true,
              label: sourceName,
              // Built when the menu opens, never per rebuild — which is what
              // lets it probe (K-194). A matte gates this layer with another
              // layer's *picture*, so a layer with none (a camera, a Null, an
              // audio-only clip) is not offered, and neither is this one:
              // matting a layer with itself has no meaning.
              options: () => [
                (null, l10n.noMatte),
                for (final e in all)
                  if (e.layer.internallayerId != layer.internallayerId &&
                      e.layer.hasPicture())
                    (e.layer.internallayerId, e.info.name),
              ],
              onChanged: (id) => _set(id == null
                  ? null
                  : BridgeMatte(
                      layer: id,
                      luma: matte?.luma ?? false,
                      inverted: matte?.inverted ?? false,
                    )),
            ),
          ),
          // The mode toggles only mean something once a source is set.
          if (matte != null) ...[
            _toggle(
              t,
              key: 'tl-matte-luma-${layer.internallayerId}',
              glyph: matte.luma ? 'L' : 'α',
              on: true,
              tip: matte.luma ? l10n.tipLumaMatte : l10n.tipAlphaMatte,
              onTap: () => _set(BridgeMatte(
                  layer: matte.layer,
                  luma: !matte.luma,
                  inverted: matte.inverted)),
            ),
            _toggle(
              t,
              key: 'tl-matte-invert-${layer.internallayerId}',
              glyph: '−',
              on: matte.inverted,
              tip: matte.inverted ? l10n.tipInverted : l10n.tipNotInverted,
              onTap: () => _set(BridgeMatte(
                  layer: matte.layer,
                  luma: matte.luma,
                  inverted: !matte.inverted)),
            ),
          ],
        ],
      ),
    );
  }

  Widget _toggle(
    LumitTheme t, {
    required String key,
    required String glyph,
    required bool on,
    required String tip,
    required VoidCallback onTap,
  }) {
    return LumitTooltip(
      message: tip,
      child: GestureDetector(
        key: ValueKey<String>(key),
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: SizedBox(
          width: 14,
          height: 22,
          child: Center(
            child: Text(glyph,
                style: t.small
                    .copyWith(color: on ? t.textPrimary : t.textDisabled)),
          ),
        ),
      ),
    );
  }
}

/// Add, rename and remove markers, and set the work area, from one dialogue.
///
/// A dialogue rather than direct manipulation on the ruler: dragging markers is
/// its own gesture layer, and having the commands somewhere reachable first
/// means the capability exists before the polish does.
Future<void> showMarkerEditorFrb({
  required BuildContext context,
  required CompositionReference comp,
  required int playheadFrame,
}) async {
  await showLumitModal<void>(
    context: context,
    builder: (close) => _MarkerEditor(
      comp: comp,
      playheadFrame: playheadFrame,
      onClose: () => close(null),
    ),
  );
}

class _MarkerEditor extends StatefulWidget {
  final CompositionReference comp;
  final int playheadFrame;
  final VoidCallback onClose;
  const _MarkerEditor({
    required this.comp,
    required this.playheadFrame,
    required this.onClose,
  });

  @override
  State<_MarkerEditor> createState() => _MarkerEditorState();
}

class _MarkerEditorState extends State<_MarkerEditor> {
  final TextEditingController _label = TextEditingController();

  @override
  void dispose() {
    _label.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final markers = markersOf(widget.comp);

    return FloatSurface(
      width: 340,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.all(8),
            child: Text(l10n.menuMarkers, style: t.bodyPrimary),
          ),
          for (final marker in markers)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
              child: Row(
                children: [
                  SizedBox(
                    width: 44,
                    child: Text(
                      '${frameAtTime(widget.comp, marker.time)}',
                      style: t.mono,
                    ),
                  ),
                  Expanded(
                    child: Text(
                      marker.label.isEmpty ? l10n.markerNoLabel : marker.label,
                      style: t.body,
                      overflow: TextOverflow.ellipsis,
                    ),
                  ),
                  HouseButton(
                    key: ValueKey<String>('marker-remove-${marker.id}'),
                    small: true,
                    frameless: true,
                    onPressed: () {
                      writeMarkers(widget.comp, [
                        for (final m in markers)
                          if (m.id != marker.id) m,
                      ]);
                      setState(() {});
                    },
                    child:
                        Text('×', style: t.small.copyWith(color: t.textMuted)),
                  ),
                ],
              ),
            ),
          if (markers.isEmpty)
            Padding(
              padding: const EdgeInsets.all(8),
              child: Text(l10n.noMarkersYet, style: t.small),
            ),
          const SizedBox(height: 6),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 8),
            child: Row(
              children: [
                Expanded(
                  child: HouseTextField(
                    key: const ValueKey('marker-label'),
                    controller: _label,
                    width: 170,
                  ),
                ),
                const SizedBox(width: 6),
                HouseButton(
                  key: const ValueKey('marker-add'),
                  small: true,
                  onPressed: () {
                    addMarkerFrb(widget.comp,
                        frame: widget.playheadFrame, label: _label.text);
                    _label.clear();
                    setState(() {});
                  },
                  child: Text(l10n.addAtPlayhead, style: t.small),
                ),
              ],
            ),
          ),
          const SizedBox(height: 8),
          Padding(
            padding: const EdgeInsets.all(8),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                HouseButton(
                  key: const ValueKey('marker-close'),
                  small: true,
                  onPressed: widget.onClose,
                  child: Text(l10n.close),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// The work area as the Timeline draws it, in frames (K-203).
///
/// The engine stores "no work area" as null, which is right — it means the
/// comp has not been narrowed. The *interface* has no such state: a comp that
/// has not been narrowed is one whose work area is the whole thing, which is
/// what every editor shows and what makes the ends grabbable from the first
/// frame. Without this the handles had nothing to hang on and B and N had
/// nothing to move, so the work area read as unimplemented.
///
/// Frames, not a [BridgeSpan], because frames are what everything drawing it
/// actually wants — an x on the axis. Handing back a span meant every caller
/// converted it straight back, four bridge calls at a time, on a widget that
/// rebuilds with the panel (docs/13). `whole` says the span covers the comp,
/// which is when there is no out-of-range ground to wash.
///
/// Two bridge calls when nothing is set, four when it is. Work it out once per
/// build and pass it down rather than asking again in each widget.
({int start, int end, bool whole}) workAreaFrames(CompositionReference comp) {
  final duration = comp.durationFrames();
  final set = comp.getWorkArea();
  if (set == null) {
    return (start: 0, end: duration < 1 ? 1 : duration, whole: true);
  }
  final start = comp.frameAtTime(time: set.inPoint);
  final end = comp.frameAtTime(time: set.outPoint);
  return (start: start, end: end, whole: start <= 0 && end >= duration);
}

/// Set the work area from the playhead: one click for its start, one for its
/// end. The two buttons match the egui frontend's B and N keys.
BridgeSpan workAreaWith({
  required CompositionReference comp,
  required BridgeSpan? current,
  required int wanted,
  required bool isStart,
}) {
  final duration = comp.durationFrames();
  final zero = comp.timeOfFrame(frame: 0);
  // Clamped to the composition. There are no frames outside it to work on, and
  // a span that reaches past either end is refused by the engine — which used
  // to mean a drag that went past frame zero threw, after taking the render
  // worker down with it (a negative frame number cast unsigned). The engine
  // clamps too (lumit-core's `SetWorkArea`); this is what makes the handle stop
  // at the edge under the pointer rather than snap back after the fact.
  final frame = wanted.clamp(0, duration);
  final existingIn =
      current == null ? 0 : comp.frameAtTime(time: current.inPoint);
  final existingOut =
      current == null ? duration : comp.frameAtTime(time: current.outPoint);

  // A work area has to have length, so the opposite edge gives way rather than
  // the click being ignored — the same thing the egui frontend does.
  var start = isStart ? frame : existingIn;
  var end = isStart ? existingOut : frame;
  if (end <= start) {
    if (isStart) {
      end = (start + 1).clamp(0, duration);
      if (end <= start) start = end - 1;
    } else {
      start = (end - 1).clamp(0, duration);
    }
  }

  return BridgeSpan(
    inPoint: comp.timeOfFrame(frame: start),
    outPoint: comp.timeOfFrame(frame: end),
    startOffset: zero,
  );
}

/// The glyph for a layer kind, matching the Project panel's row glyphs so a
/// footage layer reads as footage in both.
LumitIcon iconForKind(BridgeLayerKind kind) => switch (kind) {
      BridgeLayerKind.footage => LumitIcon.footage,
      BridgeLayerKind.sequence => LumitIcon.sequence,
      BridgeLayerKind.precomp => LumitIcon.comp,
      BridgeLayerKind.text => LumitIcon.text,
      // Vector art, drawn as the shape tool that usually makes it (K-237).
      BridgeLayerKind.shape => LumitIcon.rectangle,
      BridgeLayerKind.camera => LumitIcon.camera,
      // A light borrows the aperture glyph (K-360): the icon set has no lamp,
      // and an iris is at least the right family — something about how light
      // reaches the sensor rather than about the picture.
      BridgeLayerKind.light => LumitIcon.aperture,
      // An Audio layer (K-435) wears the speaker the audible switch wears, so
      // a row that only makes sound says so in the same glyph twice.
      BridgeLayerKind.audio => LumitIcon.audio,
      // An adjustment layer is a comp-sized effect container, drawn as a solid —
      // the same choice layer_style.dart and the egui frontend make.
      BridgeLayerKind.solid || BridgeLayerKind.adjustment => LumitIcon.solid,
      BridgeLayerKind.nullLayer => LumitIcon.nullLayer,
    };

/// The cache bar: a thin stripe under the time ruler showing which frames are
/// already rendered and held (docs/07-UI-SPEC.md §3.2, docs/15-DESIGN.md §6.3).
///
/// **What the colours mean.** Mint means the frame is held in memory or on the
/// graphics card at the resolution the Viewer is showing — it plays now, which is
/// the promise the bar exists to make (docs/13 §B5). Steel blue means it is
/// parked on disk only: one promotion from playing, not playable this instant.
/// Either colour dimmed means it is held only at a coarser resolution than is
/// being displayed — there is something, but it would be rendered again to show
/// it at this size. Nothing drawn means nothing held. No amber, no red, no
/// pulsing — an empty cache is not a fault.
///
/// **The redesign's resolution-tier hues are not drawn yet** (docs/15 §6.3,
/// §12A.1): a bar whose hue says *full, half or quarter* needs the engine to
/// report which resolution a frame is held at, and `cached_frames` answers a
/// different question — held or parked, at the shown resolution or coarser,
/// relative to the scale it is asked about. Until that reaches the bridge, the
/// four storage states above are what there is to colour by, and they are
/// coloured by §6.3's table.
///
/// **It never polls, and it is not asked again just because the panel
/// rebuilt.** The cache's lock is the one a render holds, so reading it per
/// paint would put the interface behind the renderer. `revision` is bumped when
/// a frame arrives, and only then — or when the comp, its length or the
/// resolution changes — is the cache asked. Held in state rather than read in
/// `build` for exactly that reason: a zoom flight rebuilds this widget on every
/// animation frame, and a stateless read made each of those frames take the
/// render lock and allocate a byte per frame of the composition (K-293).
class TimelineCacheBar extends StatefulWidget {
  final CompositionReference comp;
  final CacheBarAxis axis;
  final Listenable revision;

  /// Three logical pixels — the approved mockup's own stripe (K-451), and
  /// docs/15 §6.3. It is drawn **on the ruler's floor**, inside its 36
  /// (§12A.6), over the work-area band's own row rather than as a strip of its
  /// own below it: the mockup draws the cached segments on the band, and a
  /// separate strip cut the band three pixels short of the lanes.
  static const double height = 3;

  const TimelineCacheBar({
    super.key,
    required this.comp,
    required this.axis,
    required this.revision,
  });

  @override
  State<TimelineCacheBar> createState() => _TimelineCacheBarState();
}

class _TimelineCacheBarState extends State<TimelineCacheBar> {
  Uint8List _tiers = Uint8List(0);

  /// What the held [_tiers] were read for. A read is repeated when one of these
  /// moves, and skipped when the rebuild is only the zoom widening the bar.
  int? _readFrames;
  double? _readScale;

  @override
  void initState() {
    super.initState();
    widget.revision.addListener(_invalidate);
  }

  @override
  void didUpdateWidget(TimelineCacheBar old) {
    super.didUpdateWidget(old);
    if (old.revision != widget.revision) {
      old.revision.removeListener(_invalidate);
      widget.revision.addListener(_invalidate);
    }
    // A different composition is a different cache. Cleared directly rather
    // than through [_invalidate]: a build follows this call anyway, and a
    // `setState` here would only ask for a second one.
    if (old.comp != widget.comp) _readFrames = null;
  }

  @override
  void dispose() {
    widget.revision.removeListener(_invalidate);
    super.dispose();
  }

  /// A frame arrived (or the comp changed): read again on the next build.
  void _invalidate() {
    if (!mounted) return;
    setState(() => _readFrames = null);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final frames = widget.axis.frames;
    final scale = Provider.of<LumitUiState>(context, listen: false).viewerScale;
    if (_readFrames != frames || _readScale != scale) {
      _tiers = frames <= 0
          ? Uint8List(0)
          : widget.comp.cachedFrames(
              frames: BigInt.from(frames),
              scale: scale,
            );
      _readFrames = frames;
      _readScale = scale;
    }
    return SizedBox(
      height: TimelineCacheBar.height,
      child: CustomPaint(
        key: const ValueKey('tl-cache-bar'),
        painter: _CacheBarPainter(
          tiers: _tiers,
          axis: widget.axis,
          ready: t.success,
          coarse: t.success.withValues(alpha: 0.4),
          onDisk: t.cacheDisk,
          onDiskCoarse: t.cacheDisk.withValues(alpha: 0.4),
        ),
      ),
    );
  }
}

/// What the cache bar needs from the Timeline's frames-to-pixels mapping. Named
/// separately so the painter can be tested without building a Timeline.
abstract class CacheBarAxis {
  int get frames;
  double xOf(int frame);
}

/// The Timeline's one frames-to-pixels mapping, shared by the lane view and
/// the graph editor so a frame sits at the same x in both — zoom and scroll
/// included.
class TimelineAxis implements CacheBarAxis {
  @override
  final int frames;

  /// The whole width the axis is laid out in — [pad] either side plus the
  /// [span] the frames occupy.
  final double width;
  const TimelineAxis({required this.frames, required this.width});

  /// The few pixels either side of the frames (docs/15 §12A.1). Without them a
  /// handle on the first or last frame is half outside the area that draws it,
  /// and the half that is left cannot be grabbed: the work-area edges at the
  /// ends of a comp, and a keyframe on frame zero, were both unreachable. Every
  /// timeline mode shares the one number, because they share this axis — which
  /// is what keeps the ruler, the lanes and the curves lined up.
  ///
  /// Six: half the widest thing hung on a single frame — the work-area
  /// handle's ten-pixel grab, the lane's twelve-pixel keyframe slot. Symmetric,
  /// so the middle of the axis is still the middle frame.
  static const double pad = 6;

  /// The pixels the frames themselves occupy.
  double get span => max(0.0, width - pad * 2);

  double get perFrame => frames <= 0 ? 0 : span / frames;
  @override
  double xOf(num frame) => pad + frame * perFrame;
  int frameAt(double x) => frameAtExact(x).round();

  /// Where [x] falls **between** frames — what a drag with the magnet off, or
  /// a curve being sampled across the pane, asks for.
  double frameAtExact(double x) => perFrame <= 0 ? 0 : (x - pad) / perFrame;

  /// How many frames a *travel* of [dx] pixels is worth. Not [frameAtExact]:
  /// a distance has no origin, so the padding must not be taken off it.
  double framesOfPx(double dx) => perFrame <= 0 ? 0 : dx / perFrame;
}

/// The time ruler: the time labels and ticks, the work area, the markers, and
/// the scrub surface — drawn over the lanes in lane view and over the curves
/// in graph view, so neither loses the clock (docs/07 §4.1, §5).
class TimelineRuler extends StatefulWidget {
  final CompositionReference comp;
  final TimelineAxis axis;

  /// The comp's rate, turning frames into the seconds the labels speak.
  final double fps;
  final double height;
  final ValueChanged<int> onSeek;

  /// Dragging a work-area edge (K-202). Given the new span; null leaves the
  /// edges as plain marks, which is what a caller with nothing to commit to
  /// wants.
  final void Function(BridgeSpan span)? onWorkArea;

  /// Where the work area falls, in frames — worked out once by the panel and
  /// handed down, because asking the engine again in each widget that draws it
  /// is a per-rebuild cost on a panel that rebuilds a lot (docs/13).
  final ({int start, int end, bool whole}) work;

  /// A marker was moved, renamed or removed on the ruler (K-254) — the ruler
  /// has already written it to the document, and this is the panel being told
  /// so the rest of it redraws. Null in a ruler with no markers to edit.
  final VoidCallback? onMarkersChanged;

  /// The cache bar, laid on the ruler's floor over the work-area band
  /// (§12A.1): the band paints behind it because the band is part of the same
  /// row. Null for a ruler with no cache to show.
  final Widget? cache;

  const TimelineRuler({
    super.key,
    required this.comp,
    required this.axis,
    required this.fps,
    required this.height,
    required this.onSeek,
    required this.work,
    this.onWorkArea,
    this.onMarkersChanged,
    this.cache,
  });

  @override
  State<TimelineRuler> createState() => _TimelineRulerState();
}

class _TimelineRulerState extends State<TimelineRuler> {
  /// The frame a work-area edge has been dragged to, and which edge it is.
  ///
  /// Held here so the handle follows the pointer at once: committing a drag
  /// goes through the engine and comes back out as a fresh `work`, and drawing
  /// the edge from *that* left it visibly trailing the mouse. The commit still
  /// happens on every frame the drag crosses — this only decides where the
  /// edge is drawn while the button is down.
  int? _dragFrame;
  bool _dragIsStart = false;

  /// The marker being dragged, and the frame it has reached — the same
  /// arrangement as the work area's above, for the same reason: a flag that
  /// waits for the document to come back round visibly trails the pointer.
  UuidValue? _dragMarker;
  int? _dragMarkerFrame;

  /// How far into the flag the drag took hold. Without it the flag's left edge
  /// jumped to the pointer the moment the drag started, which reads as the
  /// marker flinching away from the grab.
  double _dragMarkerGrab = 0;

  /// Where a marker draws right now: the document's frame, or the dragged one.
  int _markerFrame(BridgeMarker marker) =>
      marker.id == _dragMarker && _dragMarkerFrame != null
          ? _dragMarkerFrame!
          : frameAtTime(widget.comp, marker.time);

  /// The last frame of the comp, read once when a drag starts. Asking per
  /// pointer move was a bridge call per pixel of travel.
  int _dragMarkerLast = 0;

  /// Write a whole marker list and tell the panel. Every marker edit on the
  /// ruler goes through here, so there is one place that knows a marker change
  /// is a document change.
  void _writeMarkers(List<BridgeMarker> markers) {
    writeMarkers(widget.comp, markers);
    widget.onMarkersChanged?.call();
    if (mounted) setState(() {});
  }

  /// Follow the pointer. **Nothing is written while the button is down**: the
  /// flag draws from [_dragMarkerFrame] and the document hears about the move
  /// once, on release. Committing per frame crossed — the way the work-area
  /// edges do — cost a document write, a cache flush and a panel rebuild for
  /// every frame of travel, which is what made the drag feel heavy. A
  /// work-area edge can afford it because the Viewer preview range changes as
  /// it moves; a marker has nothing to show until it lands.
  void _dragMarkerTo(int frame) {
    final to = frame.clamp(0, _dragMarkerLast < 0 ? 0 : _dragMarkerLast);
    if (to == _dragMarkerFrame) return;
    setState(() => _dragMarkerFrame = to);
  }

  /// The drag ended: write where the flag has been sitting, once.
  void _dropMarker(BridgeMarker marker) {
    final to = _dragMarkerFrame;
    setState(() {
      _dragMarker = null;
      _dragMarkerFrame = null;
    });
    if (to == null) return;
    // The same placement rule adding a marker follows, so a flag dropped onto
    // another behaves exactly as `Ctrl`+digit aimed at an occupied frame does.
    _writeMarkers(markersWithFrb(widget.comp,
        frame: to, label: marker.label, id: marker.id));
  }

  /// The right-click menu on a flag: change what it says, or take it away.
  void _markerMenu(BuildContext context, BridgeMarker marker, Offset at) {
    showMarkerMenuFrb(
      context: context,
      position: at,
      marker: marker,
      markers: () => markersOf(widget.comp),
      write: _writeMarkers,
    );
  }

  /// The work area as it should draw right now: the panel's, with the edge
  /// being dragged moved to where the pointer is. Each edge stops one frame
  /// short of the other, the rule [workAreaWith] commits.
  ({int start, int end, bool whole}) get _work {
    final work = widget.work;
    final frame = _dragFrame;
    if (frame == null) return work;
    return _dragIsStart
        ? (start: frame.clamp(0, work.end - 1), end: work.end, whole: false)
        : (
            start: work.start,
            end: frame.clamp(work.start + 1, widget.axis.frames),
            whole: false
          );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final comp = widget.comp;
    final axis = widget.axis;
    final work = _work;
    final markers = markersOf(comp);

    return GestureDetector(
      key: const ValueKey('tl-ruler'),
      behavior: HitTestBehavior.opaque,
      onTapDown: (d) => widget.onSeek(axis.frameAt(d.localPosition.dx)),
      onHorizontalDragUpdate: (d) =>
          widget.onSeek(axis.frameAt(d.localPosition.dx)),
      child: Container(
        height: widget.height,
        // **The lane ground, not a strip of its own** (K-451): the mockup
        // draws the whole lane pane — ruler, cache bar and lanes — on one
        // colour, so the clock reads as the top of the time area rather than
        // as a bar bolted above it.
        color: t.timelineOutOfRange,
        child: Stack(
          children: [
            Positioned.fill(
              child: IgnorePointer(
                child: CustomPaint(
                  painter: _RulerTicksPainter(
                    axis: axis,
                    fps: widget.fps,
                    tick: t.hairlineStrong,
                    minorTick: t.hairline,
                    // Mono at 9, per §7.1: a clock is a number, and the
                    // ruler's numbers set in the face every other number in
                    // the interface does.
                    label: t.mono.copyWith(fontSize: 9, color: t.textMuted),
                  ),
                ),
              ),
            ),
            // The work area: the span the Viewer previews and the export
            // writes. The ruler's lower half only, so the ticks and labels
            // above it stay legible and the band reads as a bar hung under the
            // clock rather than a tint over it — and this is the top of *one*
            // band that carries on behind the cache bar and down through the
            // lanes (docs/15 §12A.1).
            Positioned(
              left: axis.xOf(work.start),
              width:
                  (axis.xOf(work.end) - axis.xOf(work.start)).clamp(1.0, 1e6),
              top: widget.height / 2,
              bottom: 0,
              child: IgnorePointer(
                child: Container(
                  key: const ValueKey('tl-work-area'),
                  decoration:
                      workAreaBand(t, fillAlpha: workAreaRulerFillAlpha),
                ),
              ),
            ),
            // The work area's two edges, draggable (K-202). Grabbable rather
            // than drawn-only: the menu's "set from playhead" is precise but
            // roundabout, and a span you can see is one you expect to be able
            // to take hold of. Each edge stops one frame short of the other,
            // so a drag can never invert the span.
            //
            // Only in the lower half, where the band is drawn. A handle over
            // the full height sat on top of the ticks and stole the drag from
            // the playhead whenever the two were near each other, which made
            // the playhead unscrubbable next to a work-area edge. The rule is
            // the one the band already reads as: clock above, bar below.
            if (widget.onWorkArea != null)
              for (final isStart in const [true, false])
                Positioned(
                  left: axis.xOf(isStart ? work.start : work.end) -
                      _workHandleWidth / 2,
                  width: _workHandleWidth,
                  top: widget.height / 2,
                  bottom: 0,
                  child: MouseRegion(
                    cursor: SystemMouseCursors.resizeLeftRight,
                    child: GestureDetector(
                      key: ValueKey('tl-work-${isStart ? 'start' : 'end'}'),
                      behavior: HitTestBehavior.opaque,
                      supportedDevices: dragDevices,
                      onHorizontalDragStart: (_) =>
                          setState(() => _dragIsStart = isStart),
                      // The drag is staged: the band and handle draw from
                      // `_dragFrame` alone, and the document hears nothing
                      // until the pointer lifts — one write, one undo step,
                      // and no bridge chatter while the hand is moving
                      // (owner, 2026-08-21: a mid-drag commit per frame made
                      // the drag lag and undo walk back through every frame
                      // it crossed).
                      onHorizontalDragUpdate: (d) {
                        final frame = axis
                            .frameAt(d.globalPosition.dx - _originX(context));
                        if (frame == _dragFrame) return;
                        setState(() => _dragFrame = frame);
                      },
                      onHorizontalDragEnd: (_) {
                        final frame = _dragFrame;
                        setState(() => _dragFrame = null);
                        if (frame == null) return;
                        // A refusal is not an exception for a drag to carry:
                        // a degenerate comp (no frames to work on) has no
                        // valid span, and the document keeps the one it has.
                        try {
                          widget.onWorkArea!(workAreaWith(
                            comp: comp,
                            current: comp.getWorkArea(),
                            wanted: frame,
                            isStart: isStart,
                          ));
                        } catch (_) {
                          // The span on screen snaps back to the document's.
                        }
                      },
                      // A cancelled drag commits nothing: the band snaps back
                      // to the document's span, which is what cancel means.
                      onHorizontalDragCancel: () =>
                          setState(() => _dragFrame = null),
                      // Nothing of its own to draw: the band's own edges *are*
                      // the two handles (docs/15 §12A.1), and a second mark
                      // over them only thickened the line. The grab stays the
                      // ten pixels this box is wide — a handle you have to aim
                      // at is not a handle.
                      child: const SizedBox.expand(),
                    ),
                  ),
                ),
            // Comp markers (docs/07 §4.1): After Effects' bookmark flags, in
            // the ruler's lower row where the work-area band lives. The clock
            // above stays legible, and a flag never sits on a tick.
            //
            // Last in the stack so they take the pointer ahead of the
            // work-area handles: a flag is the smaller target of the two and
            // has to win where they overlap, or a marker parked on a work-area
            // edge could not be picked up at all.
            for (final marker in markers)
              Positioned(
                // Centred on the frame, so the flag's point sits *on* the
                // playhead rather than beside it — the point is what says
                // where, and a shape hung off to one side reads as marking the
                // frame next door.
                left: axis.xOf(_markerFrame(marker)) - MarkerFlag.width / 2,
                // Standing **on the cache bar** at the ruler's floor (§12A.1):
                // markers and the band share the lower row, and a flag lifted
                // off the edge read as floating over the lanes below.
                bottom: TimelineCacheBar.height,
                child: MouseRegion(
                  cursor: SystemMouseCursors.click,
                  child: GestureDetector(
                    key: ValueKey<String>('tl-marker-${marker.id}'),
                    behavior: HitTestBehavior.opaque,
                    onSecondaryTapUp: (d) =>
                        _markerMenu(context, marker, d.globalPosition),
                    supportedDevices: dragDevices,
                    onHorizontalDragStart: (d) => setState(() {
                      _dragMarker = marker.id;
                      _dragMarkerFrame = null;
                      // Measured from the point, not the flag's left edge,
                      // because the point is what the frame means.
                      _dragMarkerGrab =
                          d.localPosition.dx - MarkerFlag.width / 2;
                      _dragMarkerLast = comp.durationFrames() - 1;
                    }),
                    onHorizontalDragUpdate: (d) => _dragMarkerTo(axis.frameAt(
                        d.globalPosition.dx -
                            _originX(context) -
                            _dragMarkerGrab)),
                    onHorizontalDragEnd: (_) => _dropMarker(marker),
                    onHorizontalDragCancel: () => setState(() {
                      _dragMarker = null;
                      _dragMarkerFrame = null;
                    }),
                    child: MarkerFlag(
                      label: marker.label,
                      fill: t.marker,
                      pill: t.surface4,
                      text: markerLabelStyle(t),
                    ),
                  ),
                ),
              ),
            // The cached segments, on the band's own row at the ruler's floor
            // (§12A.1). Last, so they lie over the band — the band is what
            // they are drawn *on*, not a strip beside them.
            if (widget.cache != null)
              Positioned(
                left: 0,
                right: 0,
                bottom: 0,
                height: TimelineCacheBar.height,
                child: IgnorePointer(child: widget.cache!),
              ),
          ],
        ),
      ),
    );
  }
}

/// How wide a work-area edge is to grab. Wider than the 2 px it draws, so the
/// handle is catchable without the mark being heavy.
const double _workHandleWidth = 10;

/// A comp marker on the time ruler: an **upward triangle sitting on the cache
/// bar**, half inside the backdrop pill that carries what it says
/// (docs/15 §12A.1, docs/07 §4.1, K-254).
///
/// The point is the whole of the design. It is what carries the meaning — this
/// frame, not the one next door — so it points *up*, at the clock in the
/// ruler's upper half, and stands on the cache bar at the ruler's floor where
/// nothing else is drawn. The pill starts at the point and runs right, so the
/// triangle's left half stands clear of it and its right half is inside: a long
/// comment then reads as belonging to *this* moment rather than as a bar
/// starting somewhere to its right.
class MarkerFlag extends StatelessWidget {
  final String label;

  /// The triangle — the part that says *which frame*.
  final Color fill;

  /// The pill behind the label. A surface, not the marker's own colour: the
  /// writing is read, the triangle is aimed at, and giving them one value made
  /// the label the louder of the two.
  final Color pill;
  final TextStyle text;

  /// The triangle's footprint. The whole flag is placed at
  /// `xOf(frame) - width / 2`, so the point lands on the frame.
  static const double width = 8;

  /// How tall the triangle stands off the ruler's floor.
  static const double pointHeight = 6;

  /// The pill's height — and the flag's, the triangle standing inside it.
  static const double height = 12;

  const MarkerFlag({
    super.key,
    required this.label,
    required this.fill,
    required this.pill,
    required this.text,
  });

  @override
  Widget build(BuildContext context) {
    final flag = SizedBox(
      width: width,
      height: height,
      child: CustomPaint(painter: _MarkerFlagPainter(fill: fill)),
    );
    if (label.isEmpty) return flag;
    return LumitTooltip(
      message: label,
      child: Stack(
        alignment: Alignment.bottomLeft,
        children: [
          Padding(
            padding: const EdgeInsets.only(left: width / 2),
            child: Container(
              height: height,
              // Clear of the triangle's right half, which lies over the pill.
              padding: const EdgeInsets.only(left: width / 2 + 3, right: 4),
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: pill,
                // Square where the triangle meets it, rounded away from it.
                borderRadius: const BorderRadius.only(
                  topRight: Radius.circular(2),
                  bottomRight: Radius.circular(2),
                  topLeft: Radius.circular(2),
                ),
              ),
              child: Text(
                label,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: text,
              ),
            ),
          ),
          // Over the pill, so the point stays the shape you aim at.
          flag,
        ],
      ),
    );
  }
}

class _MarkerFlagPainter extends CustomPainter {
  final Color fill;

  const _MarkerFlagPainter({required this.fill});

  @override
  void paint(Canvas canvas, Size size) {
    // Base on the floor, point up: the shape *stands on* the cache bar and
    // aims at the time it marks.
    final base = size.height;
    canvas.drawPath(
      Path()
        ..moveTo(0, base)
        ..lineTo(size.width, base)
        ..lineTo(size.width / 2, base - MarkerFlag.pointHeight)
        ..close(),
      Paint()..color = fill,
    );
  }

  @override
  bool shouldRepaint(_MarkerFlagPainter old) => old.fill != fill;
}

/// Ask for what a marker says. Returns the new label, or null when the user
/// cancelled — an empty string is a real answer, being a marker with nothing
/// written on it.
Future<String?> showMarkerLabelDialogFrb({
  required BuildContext context,
  required String initial,
}) =>
    showLumitModal<String>(
      context: context,
      initialSize: const Size(320, 150),
      minSize: const Size(260, 140),
      builder: (close) => _MarkerLabelDialog(initial: initial, onDone: close),
    );

class _MarkerLabelDialog extends StatefulWidget {
  final String initial;
  final ValueChanged<String?> onDone;
  const _MarkerLabelDialog({required this.initial, required this.onDone});

  @override
  State<_MarkerLabelDialog> createState() => _MarkerLabelDialogState();
}

class _MarkerLabelDialogState extends State<_MarkerLabelDialog> {
  late final TextEditingController _label =
      TextEditingController(text: widget.initial);

  @override
  void dispose() {
    _label.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(10, 10, 10, 6),
            child: Text(l10n.marker, style: t.bodyPrimary),
          ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: HouseTextField(
              key: const ValueKey('marker-edit-label'),
              controller: _label,
              autofocus: true,
              hint: l10n.markerHint,
              onSubmitted: widget.onDone,
            ),
          ),
          Padding(
            padding: const EdgeInsets.all(10),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                HouseButton(
                  key: const ValueKey('marker-edit-cancel'),
                  small: true,
                  frameless: true,
                  onPressed: () => widget.onDone(null),
                  child: Text(l10n.cancel, style: t.small),
                ),
                const SizedBox(width: 6),
                HouseButton(
                  key: const ValueKey('marker-edit-ok'),
                  small: true,
                  // The default action (K-319). The label field holds focus,
                  // so Enter lands there and submits the same commit.
                  primary: true,
                  onPressed: () => widget.onDone(_label.text),
                  child: Text(l10n.done, style: t.small),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// Put a marker labelled [label] at [frame], replacing anything already on that
/// frame and any marker already carrying that label (K-254).
///
/// Two replacement rules, each for its own reason. **One per frame**, because
/// markers do not stack: two flags on the same moment are two things to click
/// and one place, and the second would hide the first exactly. **One per
/// number**, because `1` has to name one place — `Ctrl+1` pressed again *moves*
/// marker 1 rather than leaving two for the bare `1` to choose between. An
/// empty label never replaces by label; unlabelled cues are told apart by where
/// they are, which the frame rule already keeps distinct.
void addMarkerFrb(
  CompositionReference comp, {
  required int frame,
  String label = '',
}) =>
    writeMarkers(comp, markersWithFrb(comp, frame: frame, label: label));

/// [comp]'s marker list with one marker placed at [frame] — the shared
/// placement rule, used both when a marker is added and when one is dragged
/// onto a new moment (K-254).
///
/// Two things give way to the newcomer. **Whatever is already on that frame**,
/// because markers do not stack: two flags on one moment are two things to
/// click and one place, and the second hides the first exactly. **Whatever
/// else carries the same label**, when the label is not empty, because `1` has
/// to name one place — `Ctrl+1` pressed again *moves* marker 1 rather than
/// leaving two for the bare `1` to choose between. Unlabelled cues are told
/// apart by where they are, which the frame rule already keeps distinct.
///
/// [id] is the marker being *moved*, if this is a move; it keeps its identity
/// rather than being deleted and made again, so undo and selection see one
/// marker that travelled.
List<BridgeMarker> markersWithFrb(
  CompositionReference comp, {
  required int frame,
  required String label,
  UuidValue? id,
}) =>
    [
      for (final m in markersOf(comp))
        if (m.id != id &&
            frameAtTime(comp, m.time) != frame &&
            (label.isEmpty || m.label != label))
          m,
      BridgeMarker(
        id: id ?? UuidValue.fromString(const Uuid().v4()),
        time: timeOfFrame(comp, frame),
        label: label,
      ),
    ];

/// The frame of the marker labelled [label], or null when there is none — what
/// the bare digit keys jump to, and what makes them a quiet no-op until the
/// matching `Ctrl`+digit has been pressed.
int? markerFrameFrb(CompositionReference comp, String label) {
  for (final m in markersOf(comp)) {
    if (m.label == label) return frameAtTime(comp, m.time);
  }
  return null;
}

/// The playhead: a hairline down the whole area with a head at the top.
///
/// The head is the familiar editor marker — a bare hairline is findable only by
/// hunting along the ruler, and at a glance it reads as a row seam rather than
/// as where you are. The notch through it is drawn in the darkest surface (so
/// black on a dark scheme, white on a light one), which is what makes the head
/// read as the line running *into* it rather than a shape parked near it.
///
/// Centred on the frame, so a caller positions it at `xOf(frame) - halfWidth`.
class PlayheadMarker extends StatelessWidget {
  const PlayheadMarker({super.key});

  /// Half the head's width — how far left of the frame the marker starts.
  static const double halfWidth = 5;

  /// How tall the head is — the mockup's own 6 (K-451). It sits at the very
  /// top of the ruler, with the labels: in the lower half the work-area band
  /// would sit over it.
  static const double headHeight = 6;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return IgnorePointer(
      child: SizedBox(
        // Ten across, so the 1px stem lands on the frame rather than half a
        // pixel past it.
        width: halfWidth * 2,
        child: Column(
          children: [
            CustomPaint(
              size: const Size(halfWidth * 2, headHeight),
              painter: _PlayheadHeadPainter(head: t.accent, notch: t.surface0),
            ),
            Expanded(
              child: SizedBox(width: 1, child: ColoredBox(color: t.accent)),
            ),
          ],
        ),
      ),
    );
  }
}

/// The playhead's head: a downward triangle with the hairline carried up into
/// it as a notch.
class _PlayheadHeadPainter extends CustomPainter {
  final Color head;
  final Color notch;

  const _PlayheadHeadPainter({required this.head, required this.notch});

  @override
  void paint(Canvas canvas, Size size) {
    final mid = size.width / 2;
    canvas.drawPath(
      Path()
        ..moveTo(0, 0)
        ..lineTo(size.width, 0)
        ..lineTo(mid, size.height)
        ..close(),
      Paint()..color = head,
    );
    // Up from the tip to about where the triangle is still wide enough to hold
    // it: the short stub that joins the head to the line.
    canvas.drawLine(
      Offset(mid, size.height * 0.45),
      Offset(mid, size.height),
      Paint()
        ..color = notch
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(_PlayheadHeadPainter old) =>
      old.head != head || old.notch != notch;
}

/// The ruler's left edge in global coordinates — a drag reports globally, and
/// the axis speaks in the ruler's own pixels.
double _originX(BuildContext context) {
  final box = context.findRenderObject();
  return box is RenderBox ? box.localToGlobal(Offset.zero).dx : 0;
}

/// The label step for a ruler: the smallest nice second count whose labels
/// sit at least ~80 px apart, so zooming out thins the labels rather than
/// piling them up. Exposed for its test.
double rulerLabelStepSeconds({required double pixelsPerSecond}) {
  const nice = [
    0.5,
    1.0,
    2.0,
    5.0,
    10.0,
    15.0,
    30.0,
    60.0,
    120.0,
    300.0,
    600.0
  ];
  for (final step in nice) {
    if (step * pixelsPerSecond >= 80) return step;
  }
  return nice.last;
}

/// The minor-tick step for a ruler, in seconds: the finest division that still
/// gives each tick room to be read, and **never finer than one frame** — so
/// zooming in subdivides the ruler step by step until one tick is one frame,
/// and no further (docs/15 §12A.1). Exposed for its test.
///
/// The ladder is anchored on the frame rather than on the label step, which is
/// what makes the finest rung land exactly on frames: a step of "the label step
/// over ten" would sit between them at most rates. Returns [labelStep] itself
/// when there is no room for anything finer — meaning no minor ticks, because
/// they would only double the ones already labelled.
double rulerMinorStepSeconds({
  required double pixelsPerSecond,
  required double labelStep,
  required double fps,
}) {
  // **Thirty pixels, not six** (K-451, the approved mockup). The mockup's ruler
  // at the resting zoom labels every two seconds 140px apart and puts three
  // minor ticks between them — a half-second apart, at 35px. Six pixels was a
  // floor on *legibility*, and at 70 pixels a second it let the ladder fall two
  // rungs further, to a tick every fifth of a second: a comb rather than a
  // ruler. Thirty is the density the mockup draws, and it is a floor rather
  // than a fixed subdivision, so the ladder still refines as the zoom deepens —
  // full zoom shows twenty frames across the lanes (35px a frame), which clears
  // this and lands the finest rung, one tick per frame, exactly where it was
  // always meant to arrive.
  const minPixels = 30.0;
  final frame = fps > 0 ? 1 / fps : 0.0;
  final ladder = <double>[
    if (frame > 0) ...[frame, frame * 2, frame * 5, frame * 10],
    0.5,
    1,
    2,
    5,
    10,
    15,
    30,
    60,
    120,
    300,
    600,
  ]..sort();
  for (final step in ladder) {
    if (step >= labelStep) break;
    if (step * pixelsPerSecond >= minPixels) return step;
  }
  return labelStep;
}

/// The work-area band (docs/15 §12A.1, K-441): **one** band in `animated`
/// running from the ruler's lower half, behind the cache bar, down through the
/// lanes — the span the Viewer previews and the export writes.
///
/// [fillAlpha] varies with what the piece is lying on — heavier over the
/// ruler's own surface than over the lane ground — and the edges never do,
/// because it is the two edges lining up that make three drawn pieces read as
/// one band. They are also the two handles: what you grab is the line you see.
BoxDecoration workAreaBand(LumitTheme t, {required double fillAlpha}) =>
    BoxDecoration(
      color: t.animated.withValues(alpha: fillAlpha),
      border: Border.symmetric(
        vertical: BorderSide(color: workAreaEdgeColour(t)),
      ),
    );

/// The band's two edges, at half strength (§12A.1).
Color workAreaEdgeColour(LumitTheme t) => t.animated.withValues(alpha: 0.5);

/// What a marker's label is set in: mono at 8, the mockup's own size (K-451) —
/// a marker's label is a cue read at a glance beside a clock, and it sets in
/// the same face the clock does, one step quieter so the pill stays a pill
/// inside the ruler's 12px lower row.
TextStyle markerLabelStyle(LumitTheme t) =>
    t.mono.copyWith(fontSize: 8, color: t.textPrimary, height: 1);

/// The band's fill over the ruler's surface, and over the lane ground. Two
/// values because the two grounds are not the same value; one band either way.
/// Both are the mockup's own alphas (K-451).
const double workAreaRulerFillAlpha = 0.10;
const double workAreaLaneFillAlpha = 0.04;

/// How a layer bar — and every clip inside a Sequence layer — fills, as a
/// share of its label colour (§12A.1, K-441).
///
/// The bar is that colour *thinned* over the lane's ground rather than the
/// colour itself: at full strength a stack of layers is a row of bright slabs
/// and the eye has nothing left over for the selection or the playhead. Every
/// value here is applied to the token, never written as a second hex, so a
/// recoloured layer recolours its bar and its clips in any theme.
const double clipFillAlpha = 0.38;

/// A bar's corner radius under Sharp: **none**, as the mockup draws it
/// (K-451). Round keeps its stadium ends (K-394, §12.1) — that is the shape's
/// whole difference — and this is the other end of the same choice. It was 2,
/// which rounded nothing visibly and softened every bar end by a pixel.
const double sharpClipRadius = 0;

/// A bar's own height inside a lane row (§12A.6's table, K-451). A plain
/// constant, because it is one of the rows the table gives the same height
/// under both densities — only the ground around it changes.
const double clipBarHeight = 16;

/// The ground left above (and below) the bar: whatever the row has over.
///
/// **A fraction under Regular**, where a 16 bar sits in a 23 row and the
/// inset is 3.5. That is not a defect: Flutter's lengths are logical pixels,
/// and half of one is a real distance the compositor resolves — the bar is
/// centred either way, which is the claim §12A.6 makes about it.
double clipBarInsetFor(DensityTokens d) => (d.laneRow - clipBarHeight) / 2;

/// The same fill on a selected bar. Brighter as well as lighter, so selection
/// beats every label colour in the palette (§6.1).
const double clipFillSelectedAlpha = 0.62;

/// The solid mark at a bar's or a clip's start, in the label's full colour:
/// what makes a desaturated fill still land with a snap.
const double clipEdgeWidth = 2;

/// A ruler label. Seconds and minutes are zero-padded (`00s 02s`, `01:05s`) so
/// a row of labels keeps one rhythm; hours are not (`1:00:00s`) — the owner's
/// ruling. Deep zoom labels fractions as decimals, at most two places and
/// with trailing zeros dropped: `0.5s`, `0.25s`, `02.5s`.
String rulerLabelOf(double seconds) {
  final whole = seconds.round();
  final isWhole = (seconds - whole).abs() < 1e-9;
  if (seconds < 60) {
    if (!isWhole) {
      var text = seconds.toStringAsFixed(2);
      if (text.endsWith('0')) text = text.substring(0, text.length - 1);
      final dot = text.indexOf('.');
      // Sub-second labels keep their bare `0.` — `00.5s` reads as a timecode.
      final head = seconds >= 1
          ? text.substring(0, dot).padLeft(2, '0')
          : text.substring(0, dot);
      return '$head${text.substring(dot)}s';
    }
    return '${whole.toString().padLeft(2, '0')}s';
  }
  final h = whole ~/ 3600;
  final m = (whole % 3600) ~/ 60;
  final ss = (whole % 60).toString().padLeft(2, '0');
  final mm = m.toString().padLeft(2, '0');
  return h > 0 ? '$h:$mm:${ss}s' : '$mm:${ss}s';
}

/// The ruler's ticks and time labels — the **upper half** of the double-height
/// ruler (docs/15 §12A.1). Labelled ticks at a nice step, minor ticks
/// subdividing between them as the zoom allows, and the seam the markers and
/// the work area hang below.
class _RulerTicksPainter extends CustomPainter {
  final TimelineAxis axis;
  final double fps;
  final Color tick;

  /// The minor ticks, and the seam across the ruler's waist: quieter than the
  /// labelled ticks, so a subdivided ruler still reads as a row of labels
  /// rather than as a comb.
  final Color minorTick;
  final TextStyle label;

  const _RulerTicksPainter({
    required this.axis,
    required this.fps,
    required this.tick,
    required this.minorTick,
    required this.label,
  });

  @override
  void paint(Canvas canvas, Size size) {
    // The waist: everything the clock owns sits above it, the markers and the
    // work-area band below.
    final mid = size.height / 2;
    final quiet = Paint()
      ..color = minorTick
      ..strokeWidth = 1;
    canvas.drawLine(Offset(0, mid), Offset(size.width, mid), quiet);

    if (axis.frames <= 0 || fps <= 0 || size.width <= 0) return;
    final seconds = axis.frames / fps;
    final pxPerSec = axis.perFrame * fps;
    if (pxPerSec <= 0) return;
    final step = rulerLabelStepSeconds(pixelsPerSecond: pxPerSec);
    final paint = Paint()
      ..color = tick
      ..strokeWidth = 1;

    // Minor ticks between the labels, subdividing further the more room the
    // zoom gives them — down to one tick per frame (§12A.1).
    final minor = rulerMinorStepSeconds(
        pixelsPerSecond: pxPerSec, labelStep: step, fps: fps);
    if (minor < step) {
      for (var s = 0.0; s <= seconds; s += minor) {
        final x = axis.xOf(s * fps);
        canvas.drawLine(Offset(x, mid - 4), Offset(x, mid), quiet);
      }
    }

    for (var s = 0.0; s <= seconds; s += step) {
      final x = axis.xOf(s * fps);
      // Seven pixels of labelled tick against the minor ticks' four — the
      // mockup's own pair (K-451).
      canvas.drawLine(Offset(x, mid - 7), Offset(x, mid), paint);
      final text = TextPainter(
        text: TextSpan(text: rulerLabelOf(s), style: label),
        textDirection: TextDirection.ltr,
      )..layout();
      // Labels sit just right of their tick, at the top of the upper half; the
      // last one may clip out at the comp's end rather than jumping inside,
      // which would misplace it.
      text.paint(canvas, Offset(x + 4, 4));
    }
  }

  @override
  bool shouldRepaint(_RulerTicksPainter old) =>
      old.fps != fps ||
      old.tick != tick ||
      old.minorTick != minorTick ||
      old.axis.frames != axis.frames ||
      old.axis.width != axis.width;
}

/// Collapse per-frame tiers into the fewest contiguous runs, so a 3000-frame
/// composition draws a handful of rectangles rather than three thousand.
///
/// Returns `(startFrame, endFrameExclusive, tier)`, skipping tier 0.
List<(int, int, int)> cacheBarRuns(List<int> tiers) {
  final runs = <(int, int, int)>[];
  var start = 0;
  while (start < tiers.length) {
    final tier = tiers[start];
    var end = start + 1;
    while (end < tiers.length && tiers[end] == tier) {
      end++;
    }
    if (tier != 0) runs.add((start, end, tier));
    start = end;
  }
  return runs;
}

class _CacheBarPainter extends CustomPainter {
  final Uint8List tiers;
  final CacheBarAxis axis;
  final Color ready;
  final Color coarse;
  final Color onDisk;
  final Color onDiskCoarse;

  const _CacheBarPainter({
    required this.tiers,
    required this.axis,
    required this.ready,
    required this.coarse,
    required this.onDisk,
    required this.onDiskCoarse,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint();
    for (final (start, end, tier) in cacheBarRuns(tiers)) {
      // The engine's five states (`cached_frames`): 1 held coarser, 2 held at
      // this resolution, 3 on disk coarser, 4 on disk at this resolution. An
      // unknown value from a newer engine draws as the plainest "something is
      // held" rather than as nothing.
      paint.color = switch (tier) {
        1 => coarse,
        3 => onDiskCoarse,
        4 => onDisk,
        _ => ready,
      };
      final left = axis.xOf(start).clamp(0.0, size.width);
      // The run's right edge is the left edge of the frame after it, so a run
      // covers its last frame rather than stopping at that frame's start. At
      // least a hairline wide so a single held frame still shows, but never
      // wider than the bar — and never expressed as a clamp whose lower bound
      // could exceed its upper, because `num.clamp` throws outright when it
      // does. A composition longer than the panel is wide in pixels reaches
      // exactly that case at its last frame.
      final right = axis.xOf(end).clamp(left, size.width);
      canvas.drawRect(
          Rect.fromLTRB(
              left, 0, max(right, min(left + 1, size.width)), size.height),
          paint);
    }
  }

  /// The bytes are compared by identity on purpose: the bar holds one read
  /// until a frame arrives (see [TimelineCacheBar]), so a new list *is* new
  /// news, and comparing a byte per frame of the composition every rebuild
  /// would cost more than the paint it saves. The mapping is compared by value,
  /// because a zoom hands the same bytes a different width.
  @override
  bool shouldRepaint(_CacheBarPainter old) =>
      !identical(old.tiers, tiers) ||
      old.axis.frames != axis.frames ||
      old.axis.xOf(axis.frames) != axis.xOf(axis.frames) ||
      old.ready != ready ||
      old.coarse != coarse ||
      old.onDisk != onDisk;
}

/// The Timeline's two-tone ground (K-202): the work area at one value, and a
/// darker wash either side of it.
///
/// Painted rather than laid out as two boxes because it sits *under* the bars
/// and the marquee, and a decorated box there would absorb the pointer — the
/// same reason the row seams are a painter. With no work area both shades
/// collapse to the inside one, so an unmarked comp looks exactly as it did.
class WorkAreaGroundPainter extends CustomPainter {
  /// The work area's edges in this area's own pixels, or null for none.
  final double? startX;
  final double? endX;
  final Color inside;
  final Color outside;

  /// The band's two edges (docs/15 §12A.1), or null for a wash with no edges —
  /// which is what the overlay drawn *over* the bars wants, since the band
  /// beneath them has already drawn them.
  final Color? edge;

  const WorkAreaGroundPainter({
    required this.startX,
    required this.endX,
    required this.inside,
    required this.outside,
    this.edge,
  });

  /// The two edges, drawn last so nothing washes over them.
  void _paintEdges(Canvas canvas, Size size) {
    final colour = edge;
    if (colour == null || startX == null || endX == null) return;
    final paint = Paint()
      ..color = colour
      ..strokeWidth = 1;
    for (final x in [startX!, endX!]) {
      if (x < 0 || x > size.width) continue;
      canvas.drawLine(Offset(x, 0), Offset(x, size.height), paint);
    }
  }

  @override
  void paint(Canvas canvas, Size size) {
    final full = Offset.zero & size;
    if (startX == null || endX == null) {
      if (inside.a > 0) canvas.drawRect(full, Paint()..color = inside);
      return;
    }
    // A transparent inside makes this an overlay: the same wash painted over
    // the bars instead of under them, so the span being delivered is legible
    // across a row that has a layer in it — which is every row worth looking
    // at. Only the two outside strips are painted; there is nothing to lay
    // over the work area itself.
    if (inside.a == 0) {
      final paint = Paint()..color = outside;
      final from = startX!.clamp(0.0, size.width);
      final to = endX!.clamp(from, size.width);
      canvas.drawRect(Rect.fromLTRB(0, 0, from, size.height), paint);
      canvas.drawRect(Rect.fromLTRB(to, 0, size.width, size.height), paint);
      _paintEdges(canvas, size);
      return;
    }
    // The wash goes down first and the work area is painted back over it, so
    // the two always meet exactly — two abutting rectangles would show a seam
    // at fractional pixel positions.
    canvas.drawRect(full, Paint()..color = outside);
    final left = startX!.clamp(0.0, size.width);
    final right = endX!.clamp(0.0, size.width);
    if (right > left) {
      canvas.drawRect(
        Rect.fromLTRB(left, 0, right, size.height),
        Paint()..color = inside,
      );
    }
    _paintEdges(canvas, size);
  }

  @override
  bool shouldRepaint(WorkAreaGroundPainter old) =>
      old.startX != startX ||
      old.endX != endX ||
      old.inside != inside ||
      old.outside != outside ||
      old.edge != edge;

  /// Never absorbs a pointer — it is the ground, not a control.
  @override
  bool? hitTest(Offset position) => false;
}

/// The stretches of a collapsed Sequence layer's bar that no clip covers
/// (K-248).
///
/// A Sequence layer's bar runs from its first clip to its last, and the gaps
/// in between render transparent — they are legal, and never closed for you.
/// Shut, that used to be invisible: the bar read as solid footage all the way
/// across. This washes the gaps out, the same idea as the faint outline a
/// trimmed footage layer draws over the source it is not using (K-212): the
/// bar says what is there, and what is only reserved.
class SequenceGapsPainter extends CustomPainter {
  final List<BridgeClip> clips;
  final TimelineAxis axis;

  /// The bar's own left edge in the same pixels [axis] speaks, so a gap can be
  /// placed inside a box that does not start at time zero.
  final double left;
  final Color ink;

  const SequenceGapsPainter({
    required this.clips,
    required this.axis,
    required this.left,
    required this.ink,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (clips.isEmpty) return;
    final spans = [
      for (final c in clips)
        (
          axis.xOf(c.startFrame.toInt()) - left,
          axis.xOf(c.endFrame.toInt()) - left
        ),
    ]..sort((a, b) => a.$1.compareTo(b.$1));

    final paint = Paint()..color = ink.withValues(alpha: 0.55);
    var x = 0.0;
    for (final (start, end) in spans) {
      if (start > x) {
        canvas.drawRect(
            Rect.fromLTRB(x, 0, start.clamp(0.0, size.width), size.height),
            paint);
      }
      if (end > x) x = end;
    }
    if (x < size.width) {
      canvas.drawRect(
          Rect.fromLTRB(x.clamp(0.0, size.width), 0, size.width, size.height),
          paint);
    }
  }

  @override
  bool shouldRepaint(SequenceGapsPainter old) =>
      old.clips != clips || old.left != left || old.ink != ink;

  @override
  bool? hitTest(Offset position) => false;
}
